#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::analysis_api::{FactDatabase, SourceFile};
use crate::internal_core::Language;
use toml::Value;

use crate::go::repo_fs::{
    TOPOLOGY_MANIFEST_MAX_BYTES, normalize_repo_relative_path, read_repo_file_to_string_with_limit,
    repo_file_exists, repo_relative_existing_path,
};

const MIN_SYNTHETIC_GO_WORK_VERSION: &str = "1.24";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoAnalysisConfig {
    pub module_roots: Vec<String>,
    pub package_patterns: Vec<String>,
    pub build_tags: Vec<String>,
    pub include_tests: bool,
    pub offline: bool,
    /// `[languages.go] semantic_timeout_ms`, when configured.
    pub semantic_timeout_ms: Option<u64>,
    /// `[languages.go] rta_edges`. The analysis kernel never reads `rta_edge`
    /// rows, and `rta.Analyze` runs once per main package, so this is off by
    /// default. The polint-eval external-callgraph comparison turns it on.
    pub emit_rta_edges: bool,
    /// Package patterns for the symbol sidecar, already rooted at the
    /// repository root. Every fact family the symbol graph emits is anchored to
    /// a file, and `validate_paths` deletes each row whose file is out of scope,
    /// so loading packages that hold no in-scope file cannot change the kept
    /// output. Derived from the in-scope files unless `package_patterns` is set.
    pub symbol_rooted_patterns: Vec<String>,
    /// The repository-relative Go files this scan discovered, sorted.
    ///
    /// The semantic sidecar uses it to skip emitting rows the kernel would drop on
    /// receipt. It is not a scope for the LOAD: the package patterns still cover the
    /// whole program, so the rapid-type set is unchanged.
    pub scope_files: Vec<String>,

    pub files_without_module_root: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GoLifecycleError {
    reason: String,
}

impl GoLifecycleError {
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

#[derive(Debug)]
pub struct GoWorkspaceEnv {
    value: String,
    _temp_file: Option<tempfile::NamedTempFile>,
}

impl GoWorkspaceEnv {
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl GoAnalysisConfig {
    pub fn from_settings(
        root: &Path,
        settings: &BTreeMap<String, Value>,
        db: &dyn FactDatabase,
    ) -> Result<Self, GoLifecycleError> {
        let files = go_files(db);
        Self::from_settings_files(root, settings, &files)
    }

    pub fn from_settings_files(
        root: &Path,
        settings: &BTreeMap<String, Value>,
        files: &[&SourceFile],
    ) -> Result<Self, GoLifecycleError> {
        let configured_roots = configured_module_roots(settings)?;
        let (module_roots, files_without_module_root) = if let Some(configured_roots) =
            configured_roots
        {
            let files_without_module_root = files_not_under_module_roots(files, &configured_roots);
            (configured_roots, files_without_module_root)
        } else {
            infer_go_module_roots(root, files)
        };
        let package_patterns = validate_package_patterns(string_or_array_setting(
            settings,
            "package_patterns",
            &["./..."],
        ))?;
        let symbol_rooted_patterns = if settings.contains_key("package_patterns") {
            rooted_package_patterns(&module_roots, &package_patterns)
        } else {
            scoped_rooted_patterns(&module_roots, files)
        };
        let symbol_include_tests = match settings.get("include_tests").and_then(Value::as_bool) {
            Some(configured) => configured,
            None => files
                .iter()
                .any(|file| file.relative_path.ends_with("_test.go")),
        };
        let scope_files = files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect::<Vec<_>>();
        Ok(Self {
            module_roots,
            package_patterns,
            symbol_rooted_patterns,
            scope_files,
            build_tags: string_or_array_setting(settings, "build_tags", &[]),
            // A scan that discovered no `_test.go` file cannot report a finding
            // in one, and every test-only type it would load is absent from the
            // production program the scan was asked about. Loading test variants
            // anyway triples the package count on this shape of repository.
            include_tests: symbol_include_tests,
            offline: settings
                .get("offline")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            semantic_timeout_ms: positive_integer_setting(settings, "semantic_timeout_ms"),
            emit_rta_edges: settings
                .get("rta_edges")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            files_without_module_root,
        })
    }

    pub fn missing_module_roots(&self, root: &Path) -> Vec<String> {
        self.module_roots
            .iter()
            .filter(|module_root| !module_root_has_go_mod(root, module_root))
            .cloned()
            .collect()
    }

    pub fn rooted_package_patterns(&self) -> Vec<String> {
        rooted_package_patterns(&self.module_roots, &self.package_patterns)
    }
}

/// Reads a positive integer lifecycle setting, warning on a value that is
/// present but unusable rather than silently treating it as unset.
fn positive_integer_setting(settings: &BTreeMap<String, Value>, key: &str) -> Option<u64> {
    let value = settings.get(key)?;
    match value.as_integer() {
        Some(raw) if raw > 0 => u64::try_from(raw).ok(),
        _ => {
            tracing::warn!(
                target: "polint::kernel",
                setting = key,
                value = %value,
                "ignoring a Go lifecycle setting that is not a positive integer"
            );
            None
        }
    }
}

pub fn workspace_env(
    root: &Path,
    module_roots: &[String],
) -> Result<GoWorkspaceEnv, GoLifecycleError> {
    let checked_in = root.join("go.work");
    if repo_file_exists(root, "go.work")
        && go_work_covers_module_roots(root, &checked_in, module_roots)
    {
        return Ok(GoWorkspaceEnv {
            value: checked_in.to_string_lossy().to_string(),
            _temp_file: None,
        });
    }
    if needs_synthetic_workspace(root, module_roots) {
        let file = write_synthetic_go_work(root, module_roots)?;
        let value = file.path().to_string_lossy().to_string();
        return Ok(GoWorkspaceEnv {
            value,
            _temp_file: Some(file),
        });
    }
    Ok(GoWorkspaceEnv {
        value: "off".to_string(),
        _temp_file: None,
    })
}

pub fn go_files(db: &dyn FactDatabase) -> Vec<&SourceFile> {
    let mut files = db
        .files()
        .iter()
        .filter(|file| file.language == Language::Go)
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    files
}

fn configured_module_roots(
    settings: &BTreeMap<String, Value>,
) -> Result<Option<Vec<String>>, GoLifecycleError> {
    let values = string_or_array_value(
        settings
            .get("module_roots")
            .or_else(|| settings.get("module_root")),
        &[],
    );
    if values.is_empty() {
        return Ok(None);
    }
    let mut roots = BTreeSet::new();
    for value in values {
        roots.insert(normalize_module_root(&value)?);
    }
    Ok(Some(roots.into_iter().collect()))
}

fn normalize_module_root(raw: &str) -> Result<String, GoLifecycleError> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "." {
        return Ok(".".to_string());
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(error(format!(
            "Go module root `{raw}` must be relative to the repository root."
        )));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(error(format!(
                        "Go module root `{raw}` escapes the repository root."
                    )));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(error(format!(
                    "Go module root `{raw}` escapes the repository root."
                )));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        Ok(".".to_string())
    } else {
        Ok(normalized.to_string_lossy().replace('\\', "/"))
    }
}

fn infer_go_module_roots(root: &Path, files: &[&SourceFile]) -> (Vec<String>, Vec<String>) {
    let mut module_roots = BTreeSet::new();
    let mut files_without_module_root = Vec::new();
    for file in files {
        if let Some(module_root) = nearest_go_module_root(root, &file.relative_path) {
            module_roots.insert(module_root);
        } else {
            files_without_module_root.push(file.relative_path.clone());
        }
    }
    (
        module_roots.into_iter().collect(),
        files_without_module_root,
    )
}

fn nearest_go_module_root(root: &Path, relative_path: &str) -> Option<String> {
    let file_path = root.join(relative_path);
    let mut current = file_path.parent()?.to_path_buf();
    loop {
        if let Some(module_root) = module_root_relative_path(root, &current)
            && repo_file_exists(root, module_root_manifest_path(&module_root, "go.mod"))
        {
            return Some(module_root);
        }
        if current == root || !current.pop() {
            return None;
        }
    }
}

fn module_root_relative_path(root: &Path, module_root: &Path) -> Option<String> {
    let relative = module_root.strip_prefix(root).ok()?;
    if relative.as_os_str().is_empty() {
        Some(".".to_string())
    } else {
        Some(relative.to_string_lossy().replace('\\', "/"))
    }
}

fn module_root_has_go_mod(root: &Path, module_root: &str) -> bool {
    repo_file_exists(root, module_root_manifest_path(module_root, "go.mod"))
}

fn module_root_manifest_path(module_root: &str, file_name: &str) -> String {
    if module_root == "." {
        file_name.to_string()
    } else {
        format!("{module_root}/{file_name}")
    }
}

fn files_not_under_module_roots(files: &[&SourceFile], module_roots: &[String]) -> Vec<String> {
    files
        .iter()
        .filter(|file| {
            !module_roots
                .iter()
                .any(|module_root| file_is_under_module_root(&file.relative_path, module_root))
        })
        .map(|file| file.relative_path.clone())
        .collect()
}

fn file_is_under_module_root(relative_path: &str, module_root: &str) -> bool {
    module_root == "."
        || relative_path == module_root
        || relative_path
            .strip_prefix(module_root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn string_or_array_setting(
    settings: &BTreeMap<String, Value>,
    key: &str,
    default: &[&str],
) -> Vec<String> {
    string_or_array_value(settings.get(key), default)
}

fn string_or_array_value(value: Option<&Value>, default: &[&str]) -> Vec<String> {
    let Some(value) = value else {
        return default.iter().map(|value| (*value).to_string()).collect();
    };
    match value {
        Value::String(value) => split_comma(value),
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .flat_map(split_comma)
            .collect::<Vec<_>>(),
        _ => default.iter().map(|value| (*value).to_string()).collect(),
    }
}

fn validate_package_patterns(patterns: Vec<String>) -> Result<Vec<String>, GoLifecycleError> {
    for pattern in &patterns {
        if pattern.starts_with('-') {
            return Err(error(format!(
                "Go package pattern `{pattern}` must not start with `-` because it would be interpreted as a go list flag."
            )));
        }
    }
    Ok(patterns)
}

fn split_comma(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn rooted_package_patterns(module_roots: &[String], patterns: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut rooted = Vec::new();
    for module_root in module_roots {
        for pattern in patterns {
            let rooted_pattern = rooted_package_pattern(module_root, pattern);
            if seen.insert(rooted_pattern.clone()) {
                rooted.push(rooted_pattern);
            }
        }
    }
    rooted
}

/// The directories of the in-scope Go files, rooted at the repository root.
///
/// `go list` and `packages.Load` take directories, so one entry per directory
/// that actually holds a scanned file is the smallest sound request. Returns the
/// rooted `./...` patterns when no in-scope file sits under a module root, so an
/// empty or non-Go scan behaves exactly as before.
fn scoped_rooted_patterns(module_roots: &[String], files: &[&SourceFile]) -> Vec<String> {
    let mut directories = BTreeSet::new();
    for file in files {
        let path = file.relative_path.as_str();
        let directory = match path.rfind('/') {
            Some(index) => &path[..index],
            None => ".",
        };
        let under_module_root = module_roots.iter().any(|module_root| {
            module_root == "."
                || directory == module_root
                || directory.starts_with(&format!("{module_root}/"))
        });
        if !under_module_root {
            continue;
        }
        directories.insert(if directory == "." {
            ".".to_string()
        } else {
            format!("./{directory}")
        });
    }
    if directories.is_empty() {
        return rooted_package_patterns(module_roots, &["./...".to_string()]);
    }
    directories.into_iter().collect()
}

fn rooted_package_pattern(module_root: &str, pattern: &str) -> String {
    if module_root == "." {
        return pattern.to_string();
    }
    let base = format!("./{}", module_root.trim_start_matches("./"));
    if pattern == "." {
        return base;
    }
    if let Some(suffix) = pattern.strip_prefix("./") {
        if suffix.is_empty() {
            return base;
        }
        return format!("{base}/{suffix}");
    }
    pattern.to_string()
}

fn needs_synthetic_workspace(root: &Path, module_roots: &[String]) -> bool {
    if module_roots.len() != 1 || module_roots[0] != "." {
        return true;
    }
    !repo_file_exists(root, "go.mod")
}

pub fn go_work_covers_module_roots(root: &Path, go_work: &Path, module_roots: &[String]) -> bool {
    let Some(relative_path) = repo_relative_existing_path(root, go_work) else {
        return false;
    };
    let Ok(contents) =
        read_repo_file_to_string_with_limit(root, &relative_path, TOPOLOGY_MANIFEST_MAX_BYTES)
    else {
        return false;
    };
    let Some(roots) = parse_go_work_use_roots(root, &contents) else {
        return false;
    };
    module_roots
        .iter()
        .all(|module_root| roots.contains(module_root))
}

fn parse_go_work_use_roots(root: &Path, contents: &str) -> Option<BTreeSet<String>> {
    let mut roots = BTreeSet::new();
    let mut in_use_block = false;
    for raw_line in contents.lines() {
        let line = strip_go_line_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if in_use_block {
            let (entry, block_done) = split_go_work_block_line(line);
            if let Some(module_root) = go_work_module_root(root, entry)
                && !record_go_work_module_root(root, &mut roots, module_root)
            {
                return None;
            }
            if block_done {
                in_use_block = false;
            }
            continue;
        }
        let Some(rest) = line.strip_prefix("use") else {
            continue;
        };
        if !rest
            .chars()
            .next()
            .is_none_or(|ch| ch.is_ascii_whitespace() || ch == '(')
        {
            continue;
        }
        let rest = rest.trim_start();
        if let Some(block_rest) = rest.strip_prefix('(') {
            let (entry, block_done) = split_go_work_block_line(block_rest);
            if let Some(module_root) = go_work_module_root(root, entry)
                && !record_go_work_module_root(root, &mut roots, module_root)
            {
                return None;
            }
            in_use_block = !block_done;
        } else if let Some(module_root) = go_work_module_root(root, rest)
            && !record_go_work_module_root(root, &mut roots, module_root)
        {
            return None;
        }
    }
    Some(roots)
}

fn record_go_work_module_root(
    root: &Path,
    roots: &mut BTreeSet<String>,
    module_root: GoWorkModuleRoot,
) -> bool {
    match module_root {
        GoWorkModuleRoot::InRepo(module_root) => {
            if !module_root_has_go_mod(root, &module_root) {
                return false;
            }
            roots.insert(module_root);
            true
        }
        GoWorkModuleRoot::OutsideRepo => false,
    }
}

fn strip_go_line_comment(line: &str) -> &str {
    line.split_once("//")
        .map(|(before, _)| before)
        .unwrap_or(line)
}

fn split_go_work_block_line(line: &str) -> (&str, bool) {
    if let Some((before, _)) = line.split_once(')') {
        (before.trim(), true)
    } else {
        (line, false)
    }
}

enum GoWorkModuleRoot {
    InRepo(String),
    OutsideRepo,
}

fn go_work_module_root(root: &Path, value: &str) -> Option<GoWorkModuleRoot> {
    let token = first_go_work_path_token(value)?;
    let module_path = parse_go_work_path_token(token)?;
    let path = Path::new(&module_path);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    match normalize_repo_relative_path(root, &absolute) {
        Some(relative_path) => Some(GoWorkModuleRoot::InRepo(relative_path)),
        None => Some(GoWorkModuleRoot::OutsideRepo),
    }
}

fn first_go_work_path_token(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.starts_with('"') {
        let mut escaped = false;
        for (index, ch) in value.char_indices().skip(1) {
            if ch == '"' && !escaped {
                return Some(&value[..=index]);
            }
            escaped = ch == '\\' && !escaped;
            if ch != '\\' {
                escaped = false;
            }
        }
        return None;
    }
    if let Some(rest) = value.strip_prefix('`') {
        return rest.find('`').map(|index| &value[..=index + 1]);
    }
    value.split_whitespace().next()
}

fn parse_go_work_path_token(token: &str) -> Option<String> {
    if token.starts_with('"') {
        serde_json::from_str::<String>(token).ok()
    } else if token.starts_with('`') && token.ends_with('`') {
        Some(token[1..token.len() - 1].to_string())
    } else {
        Some(token.to_string())
    }
}

fn write_synthetic_go_work(
    root: &Path,
    module_roots: &[String],
) -> Result<tempfile::NamedTempFile, GoLifecycleError> {
    let directory = root.parent().unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::Builder::new()
        .prefix("polint-go-work-")
        .suffix(".work")
        .tempfile_in(directory)
        .map_err(|source| {
            error(format!(
                "failed to create temporary Go workspace in `{}`: {source}",
                directory.display()
            ))
        })?;
    let path = file.path().to_path_buf();
    let work_dir = path.parent().unwrap_or(directory);
    let mut contents = format!(
        "go {}\n\nuse (\n",
        synthetic_go_work_version(root, module_roots)
    );
    for module_root in module_roots {
        let path = if module_root == "." {
            root.to_path_buf()
        } else {
            root.join(module_root)
        };
        contents.push('\t');
        contents.push_str(&quote_go_work_path(&go_work_use_path(work_dir, &path)));
        contents.push('\n');
    }
    contents.push_str(")\n");
    file.write_all(contents.as_bytes()).map_err(|source| {
        error(format!(
            "failed to write temporary Go workspace `{}`: {source}",
            path.display()
        ))
    })?;
    file.flush().map_err(|source| {
        error(format!(
            "failed to flush temporary Go workspace `{}`: {source}",
            path.display()
        ))
    })?;
    Ok(file)
}

fn go_work_use_path(work_dir: &Path, module_path: &Path) -> String {
    module_path
        .strip_prefix(work_dir)
        .unwrap_or(module_path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn synthetic_go_work_version(root: &Path, module_roots: &[String]) -> String {
    let mut version = MIN_SYNTHETIC_GO_WORK_VERSION.to_string();
    for module_root in module_roots {
        let Some(module_version) = go_mod_version(root, module_root) else {
            continue;
        };
        if go_version_is_greater(&module_version, &version) {
            version = module_version;
        }
    }
    version
}

fn go_mod_version(root: &Path, module_root: &str) -> Option<String> {
    let contents = read_repo_file_to_string_with_limit(
        root,
        module_root_manifest_path(module_root, "go.mod"),
        TOPOLOGY_MANIFEST_MAX_BYTES,
    )
    .ok()?;
    contents.lines().find_map(|line| {
        let line = line.trim();
        let version = line.strip_prefix("go ")?;
        version.split_whitespace().next().map(str::to_string)
    })
}

fn go_version_is_greater(left: &str, right: &str) -> bool {
    let mut left = left.split('.').map(|part| part.parse::<u64>().unwrap_or(0));
    let mut right = right
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0));
    loop {
        match (left.next(), right.next()) {
            (Some(left), Some(right)) if left != right => return left > right,
            (Some(left), None) if left != 0 => return true,
            (None, Some(right)) if right != 0 => return false,
            (Some(_), Some(_)) | (Some(_), None) | (None, Some(_)) => {}
            (None, None) => return false,
        }
    }
}

fn quote_go_work_path(path: &str) -> String {
    format!("{path:?}")
}

fn error(reason: String) -> GoLifecycleError {
    GoLifecycleError { reason }
}

pub fn command_with_go_env(
    root: &Path,
    module_roots: &[String],
) -> Result<(Command, GoWorkspaceEnv), GoLifecycleError> {
    let workspace = workspace_env(root, module_roots)?;
    let mut command = Command::new("go");
    command
        .current_dir(root)
        .env("GOWORK", workspace.value())
        .env_remove("GOFLAGS");
    Ok((command, workspace))
}

pub fn apply_go_offline_env(command: &mut Command, offline: bool) {
    if offline {
        command
            .env("GOENV", "off")
            .env("GOPROXY", "off")
            .env("GOSUMDB", "off")
            .env("GOPRIVATE", "none")
            .env("GONOPROXY", "none")
            .env("GONOSUMDB", "none")
            .env("GOINSECURE", "none")
            .env("GOVCS", "*:off")
            .env("GOAUTH", "off")
            .env("GOTOOLCHAIN", "local")
            .env_remove("GOCACHEPROG");
    }
}
