use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::subprocess::{SubprocessError, run_bounded};
use crate::ts::types::cache_key::ts_types_sidecar_cache_key;
use crate::ts::types::diagnostics::TsTypesDiagnosticCategory;
use crate::ts::types::lifecycle::TsTypesConfig;
use crate::ts::types::process::{
    ResolvedTypeScript, TsTypesProcessError, node_command, resolve_sidecar_script,
    resolve_typescript, sidecar_digest, ts_types_timeout,
};
use crate::ts::types::protocol::{TsTypesOutput, TsTypesProtocolError, decode_ndjson};

#[derive(Debug)]
pub(crate) enum TsTypesClientError {
    Process(TsTypesProcessError),
    Protocol(TsTypesProtocolError),
}

impl std::fmt::Display for TsTypesClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Process(error) => write!(f, "{error}"),
            Self::Protocol(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for TsTypesClientError {}

impl From<TsTypesProcessError> for TsTypesClientError {
    fn from(error: TsTypesProcessError) -> Self {
        Self::Process(error)
    }
}

impl From<TsTypesProtocolError> for TsTypesClientError {
    fn from(error: TsTypesProtocolError) -> Self {
        Self::Protocol(error)
    }
}

#[derive(Debug)]
pub(crate) struct TsTypesClientRun {
    pub(crate) output: TsTypesOutput,
    pub(crate) sidecar_digest: String,
    pub(crate) typescript: ResolvedTypeScript,
}

#[derive(Debug, Clone)]
pub(crate) struct TsTypesClient {
    root: PathBuf,
    timeout: Duration,
}

impl TsTypesClient {
    pub(crate) fn new(root: PathBuf, config: &TsTypesConfig) -> Self {
        Self {
            root,
            timeout: ts_types_timeout(config.timeout_ms),
        }
    }

    pub(crate) fn run(
        &self,
        config: &TsTypesConfig,
    ) -> Result<TsTypesClientRun, TsTypesClientError> {
        let prepared = self.prepare(config)?;
        let stdout = self.invoke(config, &prepared)?;
        Ok(TsTypesClientRun {
            output: decode_ndjson(&stdout).map_err(TsTypesClientError::from)?,
            sidecar_digest: prepared.digest,
            typescript: prepared.typescript,
        })
    }

    /// Runs the sidecar, reusing stored output whose inputs are identical.
    ///
    /// The cached artifact is the raw NDJSON, not the lowered facts: replaying
    /// the wire keeps the decode, validation and lowering on the warm path, so
    /// a cache hit cannot skip a check a cold run would make.
    pub(crate) fn run_cached(
        &self,
        config: &TsTypesConfig,
        cache_dir: &Path,
        upstream_digest: &str,
    ) -> Result<TsTypesClientRun, TsTypesClientError> {
        let prepared = self.prepare(config)?;
        let cache_key = ts_types_sidecar_cache_key(
            &prepared.digest,
            &prepared.typescript.version,
            upstream_digest,
            config,
        );
        let cache_path = cache_dir.join(format!("{cache_key}.ndjson"));

        if let Ok(cached_bytes) = std::fs::read(&cache_path) {
            match decode_ndjson(&cached_bytes) {
                Ok(output) => {
                    tracing::info!(
                        target: "polint::kernel::stage",
                        provider = "polint.ts.types",
                        "sidecar cache hit"
                    );
                    return Ok(TsTypesClientRun {
                        output,
                        sidecar_digest: prepared.digest,
                        typescript: prepared.typescript,
                    });
                }
                Err(_) => {
                    let _ = std::fs::remove_file(&cache_path);
                }
            }
        }

        let stdout = self.invoke(config, &prepared)?;
        let _ = std::fs::create_dir_all(cache_dir);
        let _ = std::fs::write(&cache_path, &stdout);

        Ok(TsTypesClientRun {
            output: decode_ndjson(&stdout).map_err(TsTypesClientError::from)?,
            sidecar_digest: prepared.digest,
            typescript: prepared.typescript,
        })
    }

    fn prepare(&self, config: &TsTypesConfig) -> Result<PreparedRun, TsTypesProcessError> {
        let project_directories = config
            .projects
            .iter()
            .filter_map(|project| {
                self.root
                    .join(project)
                    .parent()
                    .map(std::path::Path::to_path_buf)
            })
            .collect::<Vec<_>>();
        let typescript = resolve_typescript(
            &self.root,
            config.typescript_path.as_deref(),
            &project_directories,
        )?;
        let script = resolve_sidecar_script()?;
        let digest = sidecar_digest(&script, &typescript)?;
        Ok(PreparedRun {
            script,
            typescript,
            digest,
        })
    }

    fn invoke(
        &self,
        config: &TsTypesConfig,
        prepared: &PreparedRun,
    ) -> Result<Vec<u8>, TsTypesProcessError> {
        let mut command = node_command(&self.root, &prepared.script);
        let scope_file = write_scope_file(config);
        command
            .arg("--root")
            .arg(self.root.as_os_str())
            .arg("--projects")
            .arg(config.projects.join(","))
            .arg("--typescript")
            .arg(prepared.typescript.directory.as_os_str());
        if let Some(scope_file) = scope_file.as_ref() {
            command.arg("--scope-files").arg(scope_file.path());
        }
        command.arg("--ndjson");

        let output = run_bounded(
            command,
            self.timeout,
            "TypeScript type sidecar",
            TsTypesDiagnosticCategory::Timeout.as_str(),
        )
        .map_err(|error| match error {
            SubprocessError::Unavailable(reason) => TsTypesProcessError::SetupMissing(format!(
                "{reason} Node is required for type-directed TypeScript analysis."
            )),
            SubprocessError::Failed(reason) => TsTypesProcessError::CommandFailed(reason),
            SubprocessError::Timeout(reason) => TsTypesProcessError::Timeout(reason),
        })?;
        tracing::debug!(
            target: "polint::kernel::stage",
            provider = "polint.ts.types",
            timeout_ms = self.timeout.as_millis() as u64,
            typescript_version = prepared.typescript.version.as_str(),
            typescript_source = prepared.typescript.source.as_str(),
            "ts type sidecar returned"
        );
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let reason = if stderr.is_empty() {
                format!(
                    "TypeScript type sidecar exited with status {}.",
                    output.status
                )
            } else {
                format!(
                    "TypeScript type sidecar exited with status {}: {stderr}",
                    output.status
                )
            };
            return Err(TsTypesProcessError::CommandFailed(reason));
        }
        Ok(output.stdout)
    }
}

struct PreparedRun {
    script: PathBuf,
    typescript: ResolvedTypeScript,
    digest: String,
}

/// Writes the discovered-file list for `--scope-files`.
///
/// The returned handle must outlive the sidecar process: dropping it deletes
/// the file the child is about to read.
///
/// A failure here is not a correctness problem — without the list the sidecar
/// emits every row it can and the kernel drops the out-of-scope ones on
/// receipt — but it silently restores the old cost, so say so rather than
/// degrade quietly.
fn write_scope_file(config: &TsTypesConfig) -> Option<tempfile::NamedTempFile> {
    if config.scope_files.is_empty() {
        return None;
    }
    match write_scope_file_inner(config) {
        Ok(file) => Some(file),
        Err(error) => {
            tracing::warn!(
                target: "polint::kernel::stage",
                provider = "polint.ts.types",
                %error,
                "could not write the sidecar scope list; the sidecar will emit every row"
            );
            None
        }
    }
}

fn write_scope_file_inner(config: &TsTypesConfig) -> std::io::Result<tempfile::NamedTempFile> {
    use std::io::Write;
    let mut file = tempfile::Builder::new()
        .prefix("polint-ts-types-scope-")
        .suffix(".txt")
        .tempfile()?;
    for path in &config.scope_files {
        writeln!(file, "{path}")?;
    }
    file.flush()?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> TsTypesConfig {
        TsTypesConfig {
            enabled: true,
            projects: vec!["tsconfig.json".to_string()],
            typescript_path: None,
            timeout_ms: None,
            scope_files: vec!["src/app.ts".to_string(), "src/other.ts".to_string()],
            files_without_project: Vec::new(),
            project_digests: vec!["tsconfig.json=digest".to_string()],
            explicitly_requested: false,
        }
    }

    #[test]
    fn the_scope_file_lists_every_discovered_path_on_its_own_line() {
        let file = write_scope_file(&config()).expect("scope file");
        let contents = std::fs::read_to_string(file.path()).expect("read scope file");

        assert_eq!(contents, "src/app.ts\nsrc/other.ts\n");
    }

    #[test]
    fn an_empty_scan_writes_no_scope_file_so_the_filter_stays_off() {
        let mut config = config();
        config.scope_files.clear();

        assert!(write_scope_file(&config).is_none());
    }
}
