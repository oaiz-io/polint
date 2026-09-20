use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::subprocess::SidecarCacheFamily;
use crate::ts::types::protocol::TS_TYPES_SCHEMA;

/// Explicit path to a `typescript` package directory or its entry script.
pub(crate) const TS_TYPESCRIPT_ENV: &str = "POLINT_TS_TYPESCRIPT";
/// Explicit path to a sidecar script, used by tests and by local debugging.
pub(crate) const TS_TYPES_SIDECAR_ENV: &str = "POLINT_TS_TYPES_SIDECAR";
/// Explicit Node executable. Defaults to `node` on `PATH`.
pub(crate) const TS_TYPES_NODE_ENV: &str = "POLINT_TS_TYPES_NODE";
/// `[languages.ts] type_timeout_ms` override.
pub(crate) const TS_TYPES_TIMEOUT_ENV: &str = "POLINT_TS_TYPES_TIMEOUT_MS";

/// Default wall-clock budget for one sidecar run.
///
/// The sidecar builds a TypeScript program per project, which is the same shape
/// of work `tsc --noEmit` does, so the budget is sized for a whole-repository
/// typecheck rather than for a single file.
pub(crate) const TS_TYPES_TIMEOUT: Duration = Duration::from_secs(300);

/// Oldest TypeScript major the compiler API this sidecar uses is available on.
const MINIMUM_TYPESCRIPT_MAJOR: u64 = 4;
/// First TypeScript major with no supported programmatic API.
///
/// TypeScript 7 is the native rewrite and exposes no stable programmatic API
/// before 7.1. Guessing at it would produce wrong types rather than no types,
/// so the version is refused and the heap tier answers alone.
const UNSUPPORTED_TYPESCRIPT_MAJOR: u64 = 7;

const TS_SIDECAR_CACHE: SidecarCacheFamily = SidecarCacheFamily {
    directory: "ts-sidecars",
    runtime_artifacts: is_ts_sidecar_runtime_artifact,
};

const EMBEDDED_TS_SIDECAR_FILES: &[(&str, &str)] = &[(
    "index.js",
    include_str!("../../ts-sidecar/polint-ts-types/index.js"),
)];

/// The sidecar runs under Node with no build step, so the only file it can
/// leave behind in its own source directory is the completion marker.
fn is_ts_sidecar_runtime_artifact(relative: &str) -> bool {
    relative == ".complete"
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TsTypesProcessError {
    /// Node, the TypeScript compiler, or a tsconfig is missing.
    SetupMissing(String),
    /// The resolved compiler has no supported programmatic API.
    VersionUnsupported(String),
    /// The sidecar could not be started or materialized.
    CommandUnavailable(String),
    /// The sidecar ran and failed.
    CommandFailed(String),
    /// The sidecar exceeded its budget.
    Timeout(String),
}

impl std::fmt::Display for TsTypesProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SetupMissing(reason)
            | Self::VersionUnsupported(reason)
            | Self::CommandUnavailable(reason)
            | Self::CommandFailed(reason)
            | Self::Timeout(reason) => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for TsTypesProcessError {}

/// The TypeScript compiler this scan will use, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedTypeScript {
    pub(crate) directory: PathBuf,
    pub(crate) version: String,
    pub(crate) source: TypeScriptSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypeScriptSource {
    /// From [`TS_TYPESCRIPT_ENV`].
    Environment,
    /// From the analyzed repository's own `node_modules`.
    Repository,
    /// From a global npm install.
    Global,
}

impl TypeScriptSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::Repository => "repository",
            Self::Global => "global",
        }
    }
}

/// Resolves the TypeScript compiler for `root`, preferring the repository's own.
///
/// The repository's compiler is preferred over anything polint could ship
/// because it is the one the repository type-checks with: matching its version
/// means matching its type semantics, and it needs no install step and no
/// network at scan time.
pub(crate) fn resolve_typescript(
    root: &Path,
    configured: Option<&str>,
    project_directories: &[PathBuf],
) -> Result<ResolvedTypeScript, TsTypesProcessError> {
    resolve_typescript_with(
        std::env::var(TS_TYPESCRIPT_ENV).ok().as_deref(),
        root,
        configured,
        project_directories,
        global_typescript,
    )
}

/// The resolution itself, with the process environment and the global lookup
/// passed in.
///
/// Both are process-wide state, and a test that reads either answers
/// differently depending on the machine it runs on.
fn resolve_typescript_with(
    environment: Option<&str>,
    root: &Path,
    configured: Option<&str>,
    project_directories: &[PathBuf],
    global: impl FnOnce() -> Option<PathBuf>,
) -> Result<ResolvedTypeScript, TsTypesProcessError> {
    if let Some(directory) = pinned_typescript(environment, root, configured)? {
        return typescript_at(directory, TypeScriptSource::Environment);
    }
    if let Some(directory) = repository_typescript(root, project_directories) {
        return typescript_at(directory, TypeScriptSource::Repository);
    }
    if let Some(directory) = global() {
        return typescript_at(directory, TypeScriptSource::Global);
    }
    Err(TsTypesProcessError::SetupMissing(format!(
        "no TypeScript compiler was found for type-directed TS analysis; install `typescript` \
         in the analyzed repository, install it globally, or set {TS_TYPESCRIPT_ENV}."
    )))
}

fn pinned_typescript(
    environment: Option<&str>,
    root: &Path,
    configured: Option<&str>,
) -> Result<Option<PathBuf>, TsTypesProcessError> {
    let candidate = environment
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| configured.map(str::to_string));
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    // A configured path is written relative to the repository, not to whatever
    // directory the command happened to start in.
    let path = root.join(candidate.trim());
    let directory = if path.is_dir() {
        path
    } else if path.is_file() {
        // `lib/typescript.js` was named directly; the package directory is two
        // levels up when the file sits in `lib`, one otherwise.
        typescript_directory_for_entry(&path)
    } else {
        return Err(TsTypesProcessError::SetupMissing(format!(
            "{TS_TYPESCRIPT_ENV} points at `{}`, which does not exist.",
            path.display()
        )));
    };
    Ok(Some(directory))
}

fn typescript_directory_for_entry(entry: &Path) -> PathBuf {
    let parent = entry.parent().map(Path::to_path_buf).unwrap_or_default();
    if parent.file_name().and_then(|name| name.to_str()) == Some("lib") {
        parent.parent().map(Path::to_path_buf).unwrap_or(parent)
    } else {
        parent
    }
}

/// Walks up from each project directory, then from the root, looking for the
/// repository's own compiler.
fn repository_typescript(root: &Path, project_directories: &[PathBuf]) -> Option<PathBuf> {
    let mut starts = project_directories.to_vec();
    starts.push(root.to_path_buf());
    for start in starts {
        let mut current = start;
        loop {
            let candidate = current.join("node_modules").join("typescript");
            if candidate.join("package.json").is_file() {
                return Some(candidate);
            }
            if current == root || !current.starts_with(root) || !current.pop() {
                break;
            }
        }
    }
    None
}

fn global_typescript() -> Option<PathBuf> {
    let mut command = Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" });
    command.arg("root").arg("-g");
    let output = crate::subprocess::run_bounded(
        command,
        Duration::from_secs(20),
        "npm root -g",
        "TsTypesSidecarTimeout",
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let directory = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if directory.is_empty() {
        return None;
    }
    let candidate = PathBuf::from(directory).join("typescript");
    candidate
        .join("package.json")
        .is_file()
        .then_some(candidate)
}

fn typescript_at(
    directory: PathBuf,
    source: TypeScriptSource,
) -> Result<ResolvedTypeScript, TsTypesProcessError> {
    let manifest = directory.join("package.json");
    let contents = std::fs::read_to_string(&manifest).map_err(|error| {
        TsTypesProcessError::SetupMissing(format!(
            "failed to read `{}` for the TypeScript version: {error}",
            manifest.display()
        ))
    })?;
    let version = typescript_version_from_manifest(&contents).ok_or_else(|| {
        TsTypesProcessError::SetupMissing(format!(
            "`{}` declares no version; cannot key the type cache on it.",
            manifest.display()
        ))
    })?;
    let major = version_major(&version).ok_or_else(|| {
        TsTypesProcessError::SetupMissing(format!(
            "could not read a major version from TypeScript version `{version}`."
        ))
    })?;
    if major >= UNSUPPORTED_TYPESCRIPT_MAJOR {
        return Err(TsTypesProcessError::VersionUnsupported(format!(
            "TypeScript {version} exposes no supported programmatic API; \
             type-directed TS analysis needs a {MINIMUM_TYPESCRIPT_MAJOR}.x to \
             {}.x compiler.",
            UNSUPPORTED_TYPESCRIPT_MAJOR - 1
        )));
    }
    if major < MINIMUM_TYPESCRIPT_MAJOR {
        return Err(TsTypesProcessError::VersionUnsupported(format!(
            "TypeScript {version} is older than the {MINIMUM_TYPESCRIPT_MAJOR}.x compiler API \
             this analysis uses."
        )));
    }
    Ok(ResolvedTypeScript {
        directory,
        version,
        source,
    })
}

/// Reads `"version"` out of a package manifest without a JSON dependency on
/// the shape of the rest of the file.
fn typescript_version_from_manifest(contents: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(contents).ok()?;
    let version = parsed.get("version")?.as_str()?.trim();
    (!version.is_empty()).then(|| version.to_string())
}

fn version_major(version: &str) -> Option<u64> {
    version
        .trim_start_matches(['v', '='])
        .split(['.', '-', '+'])
        .next()?
        .parse()
        .ok()
}

/// The sidecar script, materialized from the embedded copy unless overridden.
pub(crate) fn resolve_sidecar_script() -> Result<PathBuf, TsTypesProcessError> {
    if let Ok(path) = std::env::var(TS_TYPES_SIDECAR_ENV)
        && !path.trim().is_empty()
    {
        let candidate = PathBuf::from(path.trim());
        if candidate.is_file() {
            return Ok(candidate);
        }
        return Err(TsTypesProcessError::SetupMissing(format!(
            "{TS_TYPES_SIDECAR_ENV} must point at a sidecar script file; `{}` is not one.",
            candidate.display()
        )));
    }
    let directory = crate::subprocess::materialize_embedded_sources(
        &TS_SIDECAR_CACHE,
        "types",
        env!("CARGO_PKG_VERSION"),
        &embedded_sidecar_hash(),
        EMBEDDED_TS_SIDECAR_FILES,
    )
    .map_err(|reason| {
        TsTypesProcessError::CommandUnavailable(format!(
            "failed to materialize the embedded TS type sidecar: {reason}"
        ))
    })?;
    Ok(directory.join("index.js"))
}

/// Content hash of the embedded sidecar, including the wire schema.
///
/// The schema is folded in so a protocol change invalidates stored output even
/// if the script happens to be byte-identical.
pub(crate) fn embedded_sidecar_hash() -> String {
    let mut parts = vec![TS_TYPES_SCHEMA.to_string()];
    for (relative_path, contents) in EMBEDDED_TS_SIDECAR_FILES {
        parts.push(format!(
            "{relative_path}:{}",
            crate::ts::hash::stable_hash(&[*contents])
        ));
    }
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    crate::ts::hash::stable_hash(&refs)
}

/// Digest of the exact script that will run, plus the compiler behind it.
pub(crate) fn sidecar_digest(
    script: &Path,
    typescript: &ResolvedTypeScript,
) -> Result<String, TsTypesProcessError> {
    let contents = std::fs::read_to_string(script).map_err(|error| {
        TsTypesProcessError::CommandUnavailable(format!(
            "failed to read the TS type sidecar `{}` for digest: {error}",
            script.display()
        ))
    })?;
    Ok(crate::ts::hash::stable_hash(&[
        TS_TYPES_SCHEMA,
        contents.as_str(),
        typescript.version.as_str(),
    ]))
}

pub(crate) fn node_command(root: &Path, script: &Path) -> Command {
    let program = std::env::var(TS_TYPES_NODE_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "node".to_string());
    let mut command = Command::new(program);
    command.current_dir(root).arg(script);
    command
}

/// Resolves the sidecar budget from the setting, then the environment, then the
/// default.
///
/// The environment wins so a one-off diagnostic run can widen the budget
/// without editing committed configuration.
pub(crate) fn ts_types_timeout(setting_ms: Option<u64>) -> Duration {
    resolve_timeout(
        std::env::var(TS_TYPES_TIMEOUT_ENV).ok().as_deref(),
        setting_ms,
    )
}

fn resolve_timeout(env_ms: Option<&str>, setting_ms: Option<u64>) -> Duration {
    if let Some(raw) = env_ms {
        match raw.trim().parse::<u64>() {
            Ok(millis) if millis > 0 => return Duration::from_millis(millis),
            _ => tracing::warn!(
                target: "polint::kernel",
                value = raw,
                "ignoring unusable {TS_TYPES_TIMEOUT_ENV}; using the configured budget"
            ),
        }
    }
    match setting_ms {
        Some(millis) if millis > 0 => Duration::from_millis(millis),
        _ => TS_TYPES_TIMEOUT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hermetic resolution: no process environment, no global lookup.
    fn resolve(
        environment: Option<&str>,
        root: &Path,
        configured: Option<&str>,
        project_directories: &[PathBuf],
    ) -> Result<ResolvedTypeScript, TsTypesProcessError> {
        resolve_typescript_with(environment, root, configured, project_directories, || None)
    }

    fn write_typescript(directory: &Path, version: &str) {
        std::fs::create_dir_all(directory.join("lib")).expect("create lib");
        std::fs::write(
            directory.join("package.json"),
            format!("{{\"name\":\"typescript\",\"version\":\"{version}\"}}"),
        )
        .expect("write manifest");
        std::fs::write(directory.join("lib").join("typescript.js"), "").expect("write entry");
    }

    #[test]
    fn the_repository_compiler_is_preferred_over_a_global_one() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_typescript(&root.join("node_modules").join("typescript"), "5.9.3");

        let resolved = resolve(None, root, None, &[]).expect("repository compiler");

        assert_eq!(resolved.source, TypeScriptSource::Repository);
        assert_eq!(resolved.version, "5.9.3");
    }

    #[test]
    fn a_nested_project_finds_the_workspace_root_compiler() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_typescript(&root.join("node_modules").join("typescript"), "5.4.0");
        let project = root.join("packages").join("web");
        std::fs::create_dir_all(&project).expect("create project");

        let resolved = resolve(None, root, None, &[project]).expect("workspace compiler");

        assert_eq!(resolved.source, TypeScriptSource::Repository);
    }

    #[test]
    fn a_nested_compiler_wins_over_the_workspace_root_one() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_typescript(&root.join("node_modules").join("typescript"), "5.0.0");
        let project = root.join("packages").join("web");
        write_typescript(&project.join("node_modules").join("typescript"), "5.9.3");

        let resolved = resolve(None, root, None, &[project]).expect("nested compiler");

        assert_eq!(resolved.version, "5.9.3");
    }

    #[test]
    fn a_configured_path_wins_over_the_repository_compiler() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_typescript(&root.join("node_modules").join("typescript"), "5.0.0");
        let pinned = root.join("vendor").join("typescript");
        write_typescript(&pinned, "5.9.3");

        let resolved = resolve(None, root, Some(pinned.to_str().expect("utf-8")), &[])
            .expect("configured compiler");

        assert_eq!(resolved.source, TypeScriptSource::Environment);
        assert_eq!(resolved.version, "5.9.3");
    }

    #[test]
    fn naming_the_entry_script_resolves_its_package_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pinned = temp.path().join("typescript");
        write_typescript(&pinned, "5.9.3");
        let entry = pinned.join("lib").join("typescript.js");

        let resolved = resolve(None, temp.path(), Some(entry.to_str().expect("utf-8")), &[])
            .expect("entry script resolves");

        assert_eq!(resolved.directory, pinned);
    }

    #[test]
    fn typescript_seven_is_refused_rather_than_guessed_at() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pinned = temp.path().join("typescript");
        write_typescript(&pinned, "7.0.0");

        let error = resolve(
            None,
            temp.path(),
            Some(pinned.to_str().expect("utf-8")),
            &[],
        )
        .expect_err("major 7 is refused");

        assert!(matches!(error, TsTypesProcessError::VersionUnsupported(_)));
        assert!(error.to_string().contains("no supported programmatic API"));
    }

    #[test]
    fn a_compiler_older_than_the_api_used_is_refused() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pinned = temp.path().join("typescript");
        write_typescript(&pinned, "3.9.10");

        let error = resolve(
            None,
            temp.path(),
            Some(pinned.to_str().expect("utf-8")),
            &[],
        )
        .expect_err("major 3 is refused");

        assert!(matches!(error, TsTypesProcessError::VersionUnsupported(_)));
    }

    #[test]
    fn the_environment_overrides_a_configured_compiler_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let configured = root.join("vendor").join("typescript");
        write_typescript(&configured, "5.0.0");
        let pinned = root.join("pinned").join("typescript");
        write_typescript(&pinned, "5.9.3");

        let resolved = resolve(
            Some(pinned.to_str().expect("utf-8")),
            root,
            Some("vendor/typescript"),
            &[],
        )
        .expect("environment wins");

        assert_eq!(resolved.version, "5.9.3");
    }

    #[test]
    fn a_global_install_is_the_last_resort() {
        let temp = tempfile::tempdir().expect("tempdir");
        let global = temp.path().join("global").join("typescript");
        write_typescript(&global, "5.9.3");

        let resolved =
            resolve_typescript_with(None, temp.path(), None, &[], || Some(global.clone()))
                .expect("global compiler");

        assert_eq!(resolved.source, TypeScriptSource::Global);
    }

    #[test]
    fn no_compiler_anywhere_is_a_setup_error_naming_the_override() {
        let temp = tempfile::tempdir().expect("tempdir");

        let error = resolve_typescript_with(None, temp.path(), None, &[], || None)
            .expect_err("no compiler is refused");

        assert!(matches!(error, TsTypesProcessError::SetupMissing(_)));
        assert!(error.to_string().contains(TS_TYPESCRIPT_ENV));
    }

    #[test]
    fn a_configured_path_is_resolved_against_the_repository_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_typescript(&root.join("vendor").join("typescript"), "5.9.3");

        let resolved = resolve(None, root, Some("vendor/typescript"), &[]).expect("relative path");

        assert_eq!(resolved.directory, root.join("vendor").join("typescript"));
    }

    #[test]
    fn a_configured_path_that_does_not_exist_is_a_setup_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let error = resolve(None, temp.path(), Some("/nowhere/typescript"), &[])
            .expect_err("missing path is refused");

        assert!(matches!(error, TsTypesProcessError::SetupMissing(_)));
    }

    #[test]
    fn version_major_reads_prerelease_and_prefixed_versions() {
        assert_eq!(version_major("5.9.3"), Some(5));
        assert_eq!(version_major("v6.0.0"), Some(6));
        assert_eq!(version_major("7.0.0-dev.20260101"), Some(7));
        assert_eq!(version_major("next"), None);
    }

    #[test]
    fn the_embedded_sidecar_hash_covers_the_script_and_the_schema() {
        let hash = embedded_sidecar_hash();
        assert_eq!(hash.len(), 16);
        assert_eq!(hash, embedded_sidecar_hash());
    }

    #[test]
    fn an_unset_budget_uses_the_whole_repository_typecheck_default() {
        assert_eq!(resolve_timeout(None, None), TS_TYPES_TIMEOUT);
    }

    #[test]
    fn the_environment_budget_overrides_the_setting() {
        assert_eq!(
            resolve_timeout(Some("30000"), Some(4_500)),
            Duration::from_millis(30_000)
        );
    }

    #[test]
    fn an_unusable_budget_falls_through_instead_of_removing_the_bound() {
        assert_eq!(resolve_timeout(Some("soon"), None), TS_TYPES_TIMEOUT);
        assert_eq!(resolve_timeout(Some("0"), None), TS_TYPES_TIMEOUT);
        assert_eq!(resolve_timeout(None, Some(0)), TS_TYPES_TIMEOUT);
    }

    #[test]
    fn only_the_completion_marker_is_a_sidecar_runtime_artifact() {
        assert!(is_ts_sidecar_runtime_artifact(".complete"));
        assert!(!is_ts_sidecar_runtime_artifact("index.js"));
        assert!(!is_ts_sidecar_runtime_artifact("attacker.js"));
    }
}
