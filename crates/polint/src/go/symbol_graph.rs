use crate::analysis_api::{
    DefinitionKind, FactDatabase, ReferenceKind, SourceFile, SymbolKind, SymbolNamespace,
    SymbolPrecision,
};
use crate::analysis_neutral::symbol_graph::{
    LanguageCapabilitySupport, LanguageSymbolOutput, SymbolCapabilityStatus, SymbolGraphRequest,
    model::{DefinitionDraft, ReferenceDraft, SymbolDraft, SymbolGraphBuilder},
    semantic::{
        AliasFact, AliasId, AliasKind, ExportFact, ExportId, ExportKind, ResolutionFact,
        ResolutionId, ResolutionStepKind, ScopeFact, ScopeId, ScopeKind, SemanticImportFact,
        SemanticImportId, SemanticImportKind, SemanticIndexBuilder, SemanticIndexOutput,
        SemanticStatus, StableExportId, StableExportIdentity,
    },
    stable_id::{StableReferenceKey, StableSymbolKey},
};
use crate::go::embedded_cache::materialize_embedded_sources;
use crate::go::lifecycle::{self, GoAnalysisConfig};
use crate::go::process_runner::{GO_SUBPROCESS_TIMEOUT, GoProcessError, run_bounded};
use crate::internal_core::{
    Diagnostic, DiagnosticRange as TextRange, FileId, Language, Span, SymbolId,
};
use serde::{Deserialize, Deserializer};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Inputs owned by the Go composition boundary for symbol/reference extraction.
#[derive(Debug, Clone)]
pub struct GoSymbolOptions {
    pub root: PathBuf,
    pub settings: BTreeMap<String, toml::Value>,
    pub request: SymbolGraphRequest,
    /// When present, restrict setup-missing reference placeholders to these files.
    pub reference_files: Option<BTreeSet<String>>,
}

const SYMBOL_FACTS_DOCS_PATH: &str = "docs/facts/symbols-and-references.md";

const GO_SYMBOL_GRAPH_CAPABILITIES: &[&str] = &["symbols", "references"];
const GO_SYMBOL_SIDECAR_SCHEMA: &str = "polint-go-symbols-semantic-1";
const GO_SYMBOL_SIDECAR_ENV: &str = "POLINT_GO_SYMBOLS";
const EMBEDDED_GO_SIDECAR_FILES: &[(&str, &str)] = &[
    (
        "go.mod",
        include_str!("../go-sidecar/polint-go-symbols/go.mod"),
    ),
    (
        "go.sum",
        include_str!("../go-sidecar/polint-go-symbols/go.sum"),
    ),
    (
        "main.go",
        include_str!("../go-sidecar/polint-go-symbols/main.go"),
    ),
    (
        "internal/symbols/emit.go",
        include_str!("../go-sidecar/polint-go-symbols/internal/symbols/emit.go"),
    ),
];

#[derive(Debug, Clone)]
enum GoSidecarFailure {
    CommandUnavailable(String),
    CommandFailed(String),
    Timeout(String),
    InvalidJson(String),
    InvalidPath(String),
}

#[derive(Debug, Clone)]
enum GoSidecarCommand {
    Binary(PathBuf),
    SourceDir(PathBuf),
}

#[derive(Debug, Clone, Deserialize)]
struct GoSidecarOutput {
    schema: String,
    #[serde(default, deserialize_with = "null_as_default_vec")]
    packages: Vec<GoSidecarPackage>,
    #[serde(default, deserialize_with = "null_as_default_vec")]
    symbols: Vec<GoSidecarSymbol>,
    #[serde(default, deserialize_with = "null_as_default_vec")]
    definitions: Vec<GoSidecarDefinition>,
    #[serde(default, deserialize_with = "null_as_default_vec")]
    references: Vec<GoSidecarReference>,
    #[serde(default, deserialize_with = "null_as_default_vec")]
    scopes: Vec<GoSidecarScope>,
    #[serde(default, deserialize_with = "null_as_default_vec")]
    imports: Vec<GoSidecarImport>,
    #[serde(default, deserialize_with = "null_as_default_vec")]
    exports: Vec<GoSidecarExport>,
    #[serde(default, deserialize_with = "null_as_default_vec")]
    resolution_steps: Vec<GoSidecarResolutionStep>,
    #[serde(default, deserialize_with = "null_as_default_vec")]
    errors: Vec<GoSidecarPackageError>,
}

#[derive(Debug, Clone, Deserialize)]
struct GoSidecarPackage {
    #[serde(default, deserialize_with = "null_as_default_vec")]
    files: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GoSidecarSymbol {
    key: String,
    package_path: String,
    #[serde(default)]
    file: String,
    #[serde(default, deserialize_with = "null_as_default_vec")]
    owner_chain: Vec<String>,
    name: String,
    qualified_name: String,
    namespace: String,
    kind: String,
    span: GoSidecarSpan,
    #[serde(default)]
    exported: bool,
}

fn null_as_default_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Deserialize)]
struct GoSidecarDefinition {
    symbol_key: String,
    #[serde(default)]
    file: String,
    name: String,
    kind: String,
    span: GoSidecarSpan,
    #[serde(default)]
    primary: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct GoSidecarReference {
    package_id: String,
    #[serde(default)]
    file: String,
    name: String,
    #[serde(default)]
    target_key: String,
    kind: String,
    span: GoSidecarSpan,
    precision: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GoSidecarScope {
    key: String,
    #[serde(default)]
    parent_key: String,
    kind: String,
    package_path: String,
    #[serde(default)]
    file: String,
    span: GoSidecarSpan,
}

#[derive(Debug, Clone, Deserialize)]
struct GoSidecarImport {
    path: String,
    #[serde(default)]
    local_name: String,
    alias_kind: String,
    #[serde(default)]
    file: String,
    span: GoSidecarSpan,
}

#[derive(Debug, Clone, Deserialize)]
struct GoSidecarExport {
    symbol_key: String,
    export_name: String,
    namespace: String,
    object_path: String,
    package_path: String,
    #[serde(default)]
    generated: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct GoSidecarResolutionStep {
    reference_key: String,
    step: String,
    status: String,
    #[serde(default)]
    target_key: String,
    #[serde(default, deserialize_with = "null_as_default_vec")]
    candidate_keys: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GoSidecarPackageError {
    package_id: String,
    package_path: String,
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GoSidecarSpan {
    start_byte: usize,
    end_byte: usize,
}

pub fn derive_go_symbols(
    builder: &mut SymbolGraphBuilder,
    db: &dyn FactDatabase,
    options: &GoSymbolOptions,
) -> LanguageSymbolOutput {
    derive_go_symbols_with_runner(builder, db, options, |config| {
        run_go_sidecar(options.root.as_path(), config)
    })
}

fn derive_go_symbols_with_runner(
    builder: &mut SymbolGraphBuilder,
    db: &dyn FactDatabase,
    options: &GoSymbolOptions,
    runner: impl FnOnce(&GoAnalysisConfig) -> Result<Vec<u8>, GoSidecarFailure>,
) -> LanguageSymbolOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let files = lifecycle::go_files(db);
    if files.is_empty() || !options.request.any() {
        return LanguageSymbolOutput::default();
    }

    let config =
        match GoAnalysisConfig::from_settings_files(&options.root, &options.settings, &files) {
            Ok(config) => config,
            Err(error) => {
                return setup_missing_output(
                    interner,
                    builder,
                    &files,
                    options.request,
                    options.reference_files.as_ref(),
                    error.reason(),
                    "Configure languages.go.module_roots with repository-relative Go module roots.",
                );
            }
        };
    let uncovered_files = files_matching_paths(&files, &config.files_without_module_root);
    if !uncovered_files.is_empty() {
        let missing = setup_missing_output(
            interner,
            builder,
            &uncovered_files,
            options.request,
            options.reference_files.as_ref(),
            "some Go files are not under a go.mod module root.",
            "Move those files under a Go module or set languages.go.module_roots in .polint.toml.",
        );
        if !missing.capability_support.is_empty() {
            return missing;
        }
    }
    let missing_roots = config.missing_module_roots(&options.root);
    if !missing_roots.is_empty() {
        return setup_missing_output(
            interner,
            builder,
            &files,
            options.request,
            options.reference_files.as_ref(),
            &format!(
                "configured Go module roots are missing go.mod: {}.",
                missing_roots.join(", ")
            ),
            "Check languages.go.module_roots in .polint.toml.",
        );
    }

    let stdout = match runner(&config) {
        Ok(stdout) => stdout,
        Err(error) => {
            return setup_missing_output(
                interner,
                builder,
                &files,
                options.request,
                options.reference_files.as_ref(),
                &error.reason(),
                "Ensure Go is installed and the repository can be loaded with the configured package patterns.",
            );
        }
    };
    let sidecar = match parse_sidecar_output(&stdout).map(|output| validate_paths(output, db)) {
        Ok(output) => output,
        Err(error) => {
            return setup_missing_output(
                interner,
                builder,
                &files,
                options.request,
                options.reference_files.as_ref(),
                &error.reason(),
                "Run go test ./... or adjust languages.go package_patterns/build_tags so Go packages load cleanly.",
            );
        }
    };

    let mut output = LanguageSymbolOutput::default();
    output
        .capability_support
        .extend(supported_language_support(options.request, &files));
    output
        .semantic
        .extend(convert_sidecar_output(builder, db, &sidecar));
    output
        .diagnostics
        .extend(package_error_diagnostics(&sidecar.errors));

    output
}

impl GoSidecarFailure {
    fn reason(&self) -> String {
        match self {
            Self::CommandUnavailable(message) => message.clone(),
            Self::CommandFailed(message) => message.clone(),
            Self::Timeout(message) => message.clone(),
            Self::InvalidJson(message) => message.clone(),
            Self::InvalidPath(message) => message.clone(),
        }
    }
}

fn files_matching_paths<'a>(
    files: &'a [&'a SourceFile],
    relative_paths: &[String],
) -> Vec<&'a SourceFile> {
    let relative_paths = relative_paths.iter().collect::<BTreeSet<_>>();
    files
        .iter()
        .copied()
        .filter(|file| relative_paths.contains(&file.relative_path))
        .collect()
}

fn run_go_sidecar(root: &Path, config: &GoAnalysisConfig) -> Result<Vec<u8>, GoSidecarFailure> {
    let sidecar = resolve_go_sidecar()?;
    let mut command = match sidecar {
        GoSidecarCommand::Binary(path) => {
            let mut command = Command::new(path);
            command.current_dir(root);
            command
        }
        GoSidecarCommand::SourceDir(path) => {
            let mut command = Command::new("go");
            command
                .arg("run")
                .arg(".")
                .current_dir(path)
                .env("GOWORK", "off")
                .env("GOTOOLCHAIN", "local");
            command
        }
    };

    lifecycle::apply_go_offline_env(&mut command, config.offline);
    command
        .arg("symbols")
        .arg("--root")
        .arg(root.as_os_str())
        .arg("--module-roots")
        .arg(config.module_roots.join(","))
        .arg("--patterns")
        .arg(config.symbol_rooted_patterns.join(","))
        .arg("--rooted-patterns")
        .arg("--tests")
        .arg(config.include_tests.to_string())
        .arg("--build-tags")
        .arg(config.build_tags.join(","))
        .arg("--json")
        .env_remove("GOFLAGS");
    run_go_sidecar_command(command, GO_SUBPROCESS_TIMEOUT)
}

fn run_go_sidecar_command(
    command: Command,
    timeout: Duration,
) -> Result<Vec<u8>, GoSidecarFailure> {
    let output =
        run_bounded(command, timeout, "Go symbol sidecar").map_err(|error| match error {
            GoProcessError::Unavailable(reason) => GoSidecarFailure::CommandUnavailable(reason),
            GoProcessError::Failed(reason) => GoSidecarFailure::CommandFailed(reason),
            GoProcessError::Timeout(reason) => GoSidecarFailure::Timeout(reason),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let reason = if stderr.is_empty() {
            format!("go sidecar exited with status {}.", output.status)
        } else {
            format!("go sidecar exited with status {}: {stderr}", output.status)
        };
        return Err(GoSidecarFailure::CommandFailed(reason));
    }

    Ok(output.stdout)
}

fn resolve_go_sidecar() -> Result<GoSidecarCommand, GoSidecarFailure> {
    if let Ok(path) = std::env::var(GO_SYMBOL_SIDECAR_ENV)
        && !path.trim().is_empty()
    {
        return sidecar_command_for_path(PathBuf::from(path));
    }

    if let Some(path) = installed_sidecar_binary()? {
        return Ok(GoSidecarCommand::Binary(path));
    }

    materialize_embedded_go_sidecar().map(GoSidecarCommand::SourceDir)
}

fn sidecar_command_for_path(path: PathBuf) -> Result<GoSidecarCommand, GoSidecarFailure> {
    if path.is_file() {
        return Ok(GoSidecarCommand::Binary(path));
    }
    if path.join("go.mod").is_file() {
        return Ok(GoSidecarCommand::SourceDir(path));
    }
    Err(GoSidecarFailure::CommandFailed(format!(
        "{GO_SYMBOL_SIDECAR_ENV} must point to a polint-go-symbols binary or source directory."
    )))
}

fn installed_sidecar_binary() -> Result<Option<PathBuf>, GoSidecarFailure> {
    let executable = std::env::current_exe().map_err(|error| {
        GoSidecarFailure::CommandFailed(format!("failed to resolve current executable: {error}"))
    })?;
    let Some(directory) = executable.parent() else {
        return Ok(None);
    };
    let candidate = directory.join(sidecar_binary_name());
    Ok(candidate.is_file().then_some(candidate))
}

fn sidecar_binary_name() -> &'static str {
    if cfg!(windows) {
        "polint-go-symbols.exe"
    } else {
        "polint-go-symbols"
    }
}

fn materialize_embedded_go_sidecar() -> Result<PathBuf, GoSidecarFailure> {
    materialize_embedded_sources(
        "symbols",
        env!("CARGO_PKG_VERSION"),
        &embedded_go_sidecar_hash(),
        EMBEDDED_GO_SIDECAR_FILES,
    )
    .map_err(|reason| {
        GoSidecarFailure::CommandFailed(format!(
            "failed to materialize embedded Go symbol sidecar: {reason}"
        ))
    })
}

fn embedded_go_sidecar_hash() -> String {
    let mut parts = Vec::new();
    parts.push(GO_SYMBOL_SIDECAR_SCHEMA.to_string());
    for (relative_path, contents) in EMBEDDED_GO_SIDECAR_FILES {
        parts.push(format!(
            "{relative_path}:{}",
            crate::go::hash::stable_hash(&[*contents])
        ));
    }
    let part_refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    crate::go::hash::stable_hash(&part_refs)
}

fn parse_sidecar_output(stdout: &[u8]) -> Result<GoSidecarOutput, GoSidecarFailure> {
    let output: GoSidecarOutput = serde_json::from_slice(stdout).map_err(|error| {
        GoSidecarFailure::InvalidJson(format!("invalid Go symbol sidecar JSON: {error}"))
    })?;
    if output.schema != GO_SYMBOL_SIDECAR_SCHEMA {
        return Err(GoSidecarFailure::InvalidJson(format!(
            "unsupported Go symbol sidecar schema `{}`; expected `{GO_SYMBOL_SIDECAR_SCHEMA}`",
            output.schema
        )));
    }
    Ok(output)
}

/// Keep only the sidecar rows this scan discovered, dropping the rest.
///
/// The sidecar loads WHOLE packages (`package_patterns`), so it legitimately emits rows for
/// files a narrower scan never discovered — a PATH-argument scan of two files still gets rows
/// for every file of every loaded package. Those rows are ordinary input, not corruption: the
/// discovered-file map is the only trust anchor, and any path missing from it is out of scope
/// and is skipped, never fatal. Paths that are absolute or climb out of the repository take
/// the same route: they can never be a discovered file, so they are undiscoverable by
/// definition rather than an error. A row carrying no `file` at all is unaffected (it stays a
/// location-less row), and a package row that named files but kept none has no in-scope
/// content left to contribute and is dropped entirely. The scope reduction is already visible
/// through the `polint/scope` diagnostic and the run summary, so it is not raised here.
fn validate_paths(mut output: GoSidecarOutput, db: &dyn FactDatabase) -> GoSidecarOutput {
    let file_ids = go_file_ids(db);
    let dropped = DroppedRows::of(&output, &file_ids);
    let before = located_row_count(&output);
    output.packages.retain_mut(|package| {
        let named_files = !package.files.is_empty();
        package.files = package
            .files
            .iter()
            .filter_map(|file| in_scope_sidecar_path(file, &file_ids))
            .collect();
        !named_files || !package.files.is_empty()
    });
    retain_in_scope_rows(&mut output.symbols, |row| &mut row.file, &file_ids);
    retain_in_scope_rows(&mut output.definitions, |row| &mut row.file, &file_ids);
    retain_in_scope_rows(&mut output.references, |row| &mut row.file, &file_ids);
    retain_in_scope_rows(&mut output.scopes, |row| &mut row.file, &file_ids);
    retain_in_scope_rows(&mut output.imports, |row| &mut row.file, &file_ids);
    dropped.prune_dependents(&mut output);
    // The semantic lowering counts its skipped rows the same way. Say how much
    // the scope reduction removed: an empty symbol graph that came entirely
    // from out-of-scope paths is otherwise indistinguishable from a repository
    // with nothing in it.
    let after = located_row_count(&output);
    if after < before {
        tracing::debug!(
            rows = before - after,
            "skipped Go symbol sidecar rows naming files outside the scan scope"
        );
    }
    output
}

/// Rows that carry a file, and so can be dropped for naming an undiscovered one.
fn located_row_count(output: &GoSidecarOutput) -> usize {
    output.packages.len()
        + output.symbols.len()
        + output.definitions.len()
        + output.references.len()
        + output.scopes.len()
        + output.imports.len()
}

/// The keys of the rows this scan dropped, so the rows that only point at them can go too.
///
/// `exports` and `resolution_steps` carry no file of their own — they name a symbol or a
/// reference by key — so they are only ever out of scope by association. Keeping one whose
/// target is gone would emit a fact whose `symbol_stable_key` / `source_stable_key` /
/// `target_stable_keys` names nothing in the index, and the semantic index rejects that: a
/// scope reduction has to shrink the fact set, never invalidate it. Keys the sidecar never
/// emitted a row for (builtins, symbols outside the loaded packages) are not listed here and
/// keep their existing treatment, so a scan that drops nothing changes nothing.
struct DroppedRows {
    symbols: BTreeSet<String>,
    references: BTreeSet<String>,
}

impl DroppedRows {
    fn of(output: &GoSidecarOutput, file_ids: &BTreeMap<String, FileId>) -> Self {
        Self {
            symbols: output
                .symbols
                .iter()
                .filter(|symbol| !row_is_in_scope(&symbol.file, file_ids))
                .map(|symbol| symbol.key.clone())
                .collect(),
            references: output
                .references
                .iter()
                .filter(|reference| !row_is_in_scope(&reference.file, file_ids))
                .map(go_reference_key)
                .collect(),
        }
    }

    fn prune_dependents(&self, output: &mut GoSidecarOutput) {
        output
            .exports
            .retain(|export| !self.symbols.contains(&export.symbol_key));
        output
            .resolution_steps
            .retain(|step| !self.references.contains(&step.reference_key));
        for step in &mut output.resolution_steps {
            if self.symbols.contains(&step.target_key) {
                step.target_key.clear();
            }
            step.candidate_keys
                .retain(|candidate| !self.symbols.contains(candidate));
        }
    }
}

/// Drop every row whose `file` this scan did not discover, normalizing the ones it keeps.
///
/// Rows with an empty `file` claim no location and are kept untouched.
fn retain_in_scope_rows<T>(
    rows: &mut Vec<T>,
    file: fn(&mut T) -> &mut String,
    file_ids: &BTreeMap<String, FileId>,
) {
    rows.retain_mut(|row| {
        let path = file(row);
        if path.is_empty() {
            return true;
        }
        match in_scope_sidecar_path(path, file_ids) {
            Some(in_scope) => {
                *path = in_scope;
                true
            }
            None => false,
        }
    });
}

fn row_is_in_scope(raw_path: &str, file_ids: &BTreeMap<String, FileId>) -> bool {
    raw_path.is_empty() || in_scope_sidecar_path(raw_path, file_ids).is_some()
}

fn go_file_ids(db: &dyn FactDatabase) -> BTreeMap<String, FileId> {
    db.files()
        .iter()
        .filter(|file| file.language == Language::Go)
        .map(|file| (file.relative_path.clone(), file.id))
        .collect()
}

/// Normalize a sidecar path, keeping it only when this scan discovered that file.
///
/// `None` means the row is out of scope — the path does not resolve inside the repository, or
/// names a Go file this scan never discovered — and the caller drops it.
fn in_scope_sidecar_path(raw_path: &str, file_ids: &BTreeMap<String, FileId>) -> Option<String> {
    lexical_repo_relative(raw_path)
        .ok()
        .filter(|path| file_ids.contains_key(path))
}

fn lexical_repo_relative(raw_path: &str) -> Result<String, GoSidecarFailure> {
    let path = Path::new(raw_path);
    if path.is_absolute() {
        return Err(GoSidecarFailure::InvalidPath(format!(
            "Go symbol sidecar file path `{raw_path}` is absolute and escapes repository."
        )));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(GoSidecarFailure::InvalidPath(format!(
                        "Go symbol sidecar file path `{raw_path}` escapes repository."
                    )));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(GoSidecarFailure::InvalidPath(format!(
                    "Go symbol sidecar file path `{raw_path}` escapes repository."
                )));
            }
        }
    }
    Ok(normalized.to_string_lossy().replace('\\', "/"))
}

fn setup_missing_output(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SymbolGraphBuilder,
    files: &[&SourceFile],
    request: SymbolGraphRequest,
    reference_files: Option<&BTreeSet<String>>,
    reason: &str,
    hint: &str,
) -> LanguageSymbolOutput {
    let mut output = LanguageSymbolOutput::default();
    let references_requested = request.references
        && reference_files
            .is_none_or(|paths| files.iter().any(|file| paths.contains(&file.relative_path)));
    if references_requested {
        for file in files {
            builder.add_setup_missing_reference(
                file.language,
                file.id,
                file.relative_path.clone(),
                "<setup-missing>",
            );
        }
        output
            .semantic
            .extend(setup_missing_semantic_index_for_files(interner, files));
    }

    output
        .capability_support
        .extend(setup_support(request, files, reason, hint));
    output.capability_support.sort_by(|left, right| {
        (
            left.capability.as_str(),
            left.language,
            left.reason.as_deref(),
        )
            .cmp(&(
                right.capability.as_str(),
                right.language,
                right.reason.as_deref(),
            ))
    });
    output
}

fn setup_support(
    request: SymbolGraphRequest,
    _files: &[&SourceFile],
    reason: &str,
    hint: &str,
) -> Vec<LanguageCapabilitySupport> {
    GO_SYMBOL_GRAPH_CAPABILITIES
        .iter()
        .filter(|capability| {
            (**capability == "symbols" && request.symbols)
                || (**capability == "references" && request.references)
        })
        .map(|capability| LanguageCapabilitySupport {
            capability: (*capability).to_string(),
            language: Language::Go,
            status: SymbolCapabilityStatus::SetupMissing,
            reason: Some(reason.to_string()),
            hint: Some(hint.to_string()),
        })
        .collect()
}

fn supported_language_support(
    request: SymbolGraphRequest,
    _files: &[&SourceFile],
) -> Vec<LanguageCapabilitySupport> {
    GO_SYMBOL_GRAPH_CAPABILITIES
        .iter()
        .filter(|capability| {
            (**capability == "symbols" && request.symbols)
                || (**capability == "references" && request.references)
        })
        .map(|capability| LanguageCapabilitySupport {
            capability: (*capability).to_string(),
            language: Language::Go,
            status: SymbolCapabilityStatus::Supported,
            reason: None,
            hint: None,
        })
        .collect()
}

fn convert_sidecar_output(
    builder: &mut SymbolGraphBuilder,
    db: &dyn FactDatabase,
    sidecar: &GoSidecarOutput,
) -> SemanticIndexOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let files = go_files_by_path(db);
    let symbol_rows = sidecar
        .symbols
        .iter()
        .map(|symbol| (symbol.key.as_str(), symbol))
        .collect::<BTreeMap<_, _>>();
    let mut symbols = BTreeMap::new();
    let mut symbol_stable_keys = BTreeMap::new();
    let mut symbol_key_inputs = BTreeMap::new();
    let mut reference_stable_keys = BTreeMap::new();
    let resolution_steps = sidecar
        .resolution_steps
        .iter()
        .map(|step| (step.reference_key.as_str(), step))
        .collect::<BTreeMap<_, _>>();

    for symbol in &sidecar.symbols {
        let stable_key_input = go_stable_symbol_key(symbol, &files);
        let stable_key = stable_key_input.stable_key();
        let id = builder.add_symbol(symbol_draft(symbol, &files));
        symbols.insert(symbol.key.clone(), id);
        symbol_stable_keys.insert(symbol.key.clone(), stable_key);
        symbol_key_inputs.insert(symbol.key.clone(), stable_key_input);
    }

    for definition in &sidecar.definitions {
        let Some(symbol) = symbols.get(&definition.symbol_key).copied() else {
            continue;
        };
        let source_symbol = symbol_rows.get(definition.symbol_key.as_str()).copied();
        builder.add_definition(symbol, definition_draft(definition, source_symbol, &files));
    }

    for reference in &sidecar.references {
        let mut draft = reference_draft(reference, &files);
        reference_stable_keys.insert(
            go_reference_key(reference),
            go_stable_reference_key(reference, &files, &symbol_key_inputs),
        );
        if let Some(target) = symbols.get(&reference.target_key).copied() {
            builder.add_reference(target, draft);
        } else {
            let candidates = resolution_steps
                .get(go_reference_key(reference).as_str())
                .map(|step| semantic_candidate_symbol_ids(step, &symbols))
                .unwrap_or_default();
            if candidates.len() > 1 {
                draft.precision = SymbolPrecision::Ambiguous;
                builder.add_ambiguous_reference(candidates, draft);
            } else if let Some(candidate) = candidates.first().copied() {
                builder.add_reference(candidate, draft);
            } else {
                draft.precision = SymbolPrecision::Unresolved;
                builder.add_unresolved_reference(draft);
            }
        }
    }

    derive_go_semantic_index(
        interner,
        sidecar,
        &files,
        &symbol_stable_keys,
        &reference_stable_keys,
    )
}

fn setup_missing_semantic_index_for_files(
    interner: &crate::internal_core::StableKeyInterner,
    files: &[&SourceFile],
) -> SemanticIndexOutput {
    let mut builder = SemanticIndexBuilder::new();
    for file in files {
        let source_key = format!("go:setup-missing|file:{}", file.relative_path);
        builder.add_resolution(
            interner,
            ResolutionFact {
                id: ResolutionId(0),
                language: Language::Go,
                file: Some(file.id),
                package: None,
                module: None,
                source_stable_key: interner.intern(source_key.clone()),
                target_stable_keys: Vec::new(),
                step: ResolutionStepKind::UnknownFallback,
                stable_key: interner.intern(format!(
                    "{source_key}|step:UnknownFallback|status:setup_missing"
                )),
                status: SemanticStatus::SetupMissing,
            },
        );
    }
    builder.finish(interner)
}

fn derive_go_semantic_index(
    interner: &crate::internal_core::StableKeyInterner,
    sidecar: &GoSidecarOutput,
    files: &BTreeMap<&str, &SourceFile>,
    symbol_stable_keys: &BTreeMap<String, String>,
    reference_stable_keys: &BTreeMap<String, String>,
) -> SemanticIndexOutput {
    let mut builder = SemanticIndexBuilder::new();
    let mut scope_ids = BTreeMap::new();

    for scope in &sidecar.scopes {
        let id = builder.add_scope(
            interner,
            ScopeFact {
                id: ScopeId(0),
                language: Language::Go,
                file: file_for_path(files, &scope.file).map(|file| file.id),
                package: None,
                module: None,
                parent: scope_ids.get(scope.parent_key.as_str()).copied(),
                scope_path: go_scope_path(scope),
                kind: go_scope_kind(&scope.kind),
                stable_key: interner.intern(scope.key.clone()),
                status: SemanticStatus::Resolved,
            },
        );
        scope_ids.insert(scope.key.as_str(), id);
    }

    let resolution_steps = sidecar
        .resolution_steps
        .iter()
        .map(|step| (step.reference_key.as_str(), step))
        .collect::<BTreeMap<_, _>>();

    for import in &sidecar.imports {
        let source_key = go_import_stable_key(import);
        let resolution = resolution_steps.get(source_key.as_str()).copied();
        let status = go_import_status(import, resolution);
        builder.add_semantic_import(
            interner,
            SemanticImportFact {
                id: SemanticImportId(0),
                language: Language::Go,
                file: file_for_path(files, &import.file).map(|file| file.id),
                package: None,
                module: None,
                scope: None,
                import_path: import.path.clone(),
                local_name: go_import_local_name(import),
                imported_name: None,
                namespace: SymbolNamespace::Package,
                kind: go_import_kind(&import.alias_kind),
                stable_key: interner.intern(source_key.clone()),
                status,
            },
        );
        if let Some(alias) =
            go_import_alias_fact(interner, import, resolution, status, symbol_stable_keys)
        {
            builder.add_alias(interner, alias);
        }
    }

    for export in &sidecar.exports {
        let export_id = builder.add_export_identity(
            interner,
            ExportFact {
                id: ExportId(0),
                language: Language::Go,
                file: None,
                package: None,
                module: None,
                scope: None,
                symbol: None,
                export_name: export.export_name.clone(),
                namespace: symbol_namespace(&export.namespace),
                kind: ExportKind::Named,
                stable_key: interner.intern(go_export_stable_key(export)),
                status: if export.generated {
                    SemanticStatus::Generated
                } else {
                    SemanticStatus::Resolved
                },
            },
        );
        builder.add_stable_export(
            interner,
            StableExportIdentity {
                id: StableExportId(0),
                export: export_id,
                language: Language::Go,
                package_key: Some(format!("go:package:{}", export.package_path)),
                module_key: None,
                export_name: export.export_name.clone(),
                namespace: symbol_namespace(&export.namespace),
                symbol_stable_key: interner
                    .intern(mapped_symbol_key(symbol_stable_keys, &export.symbol_key)),
                generated_discriminator: Some("native".to_string()),
                stable_key: interner.intern(go_stable_export_key(export)),
                status: SemanticStatus::Resolved,
            },
        );
    }

    for step in &sidecar.resolution_steps {
        let target_stable_keys = mapped_candidate_keys(step, symbol_stable_keys);
        builder.add_resolution(
            interner,
            ResolutionFact {
                id: ResolutionId(0),
                language: Language::Go,
                file: None,
                package: None,
                module: None,
                source_stable_key: interner.intern(mapped_reference_key(
                    reference_stable_keys,
                    &step.reference_key,
                )),
                target_stable_keys: target_stable_keys
                    .into_iter()
                    .map(|key| interner.intern(key))
                    .collect(),
                step: go_resolution_step_kind(&step.step),
                stable_key: interner.intern(go_resolution_stable_key(step)),
                status: semantic_status(&step.status),
            },
        );
    }
    for reference in &sidecar.references {
        let reference_key = go_reference_key(reference);
        if resolution_steps.contains_key(reference_key.as_str()) {
            continue;
        }
        if reference.target_key.is_empty() {
            builder.add_resolution(
                interner,
                ResolutionFact {
                    id: ResolutionId(0),
                    language: Language::Go,
                    file: file_for_path(files, &reference.file).map(|file| file.id),
                    package: None,
                    module: None,
                    source_stable_key: interner
                        .intern(mapped_reference_key(reference_stable_keys, &reference_key)),
                    target_stable_keys: Vec::new(),
                    step: ResolutionStepKind::UnknownFallback,
                    stable_key: interner.intern(go_reference_unknown_fallback_key(reference)),
                    status: SemanticStatus::Unresolved,
                },
            );
        }
    }

    builder.finish(interner)
}

fn go_files_by_path(db: &dyn FactDatabase) -> BTreeMap<&str, &SourceFile> {
    db.files()
        .iter()
        .filter(|file| file.language == Language::Go)
        .map(|file| (file.relative_path.as_str(), file))
        .collect()
}

fn go_scope_path(scope: &GoSidecarScope) -> Vec<String> {
    [
        format!("package:{}", scope.package_path),
        format!("file:{}", scope.file),
        format!("kind:{}", scope.kind),
        format!("span:{}-{}", scope.span.start_byte, scope.span.end_byte),
        scope.key.clone(),
    ]
    .into_iter()
    .filter(|part| !part.ends_with(':'))
    .collect()
}

fn go_scope_kind(kind: &str) -> ScopeKind {
    match kind {
        "package" => ScopeKind::Package,
        "file" => ScopeKind::File,
        "function" => ScopeKind::Function,
        "method" => ScopeKind::Method,
        "block" => ScopeKind::Block,
        "type" => ScopeKind::Type,
        _ => ScopeKind::Unknown,
    }
}

fn go_import_kind(alias_kind: &str) -> SemanticImportKind {
    match alias_kind {
        "named" => SemanticImportKind::GoNamed,
        "dot" => SemanticImportKind::GoDot,
        "blank" => SemanticImportKind::GoBlank,
        "implicit" => SemanticImportKind::GoImplicit,
        _ => SemanticImportKind::Unknown,
    }
}

fn go_import_status(
    import: &GoSidecarImport,
    resolution: Option<&GoSidecarResolutionStep>,
) -> SemanticStatus {
    if import.alias_kind == "dot" {
        let candidates = resolution
            .map(normalized_candidate_keys)
            .unwrap_or_default();
        if candidates.len() == 1 {
            SemanticStatus::Resolved
        } else {
            SemanticStatus::Ambiguous
        }
    } else {
        resolution
            .map(|step| semantic_status(&step.status))
            .unwrap_or(SemanticStatus::Resolved)
    }
}

fn go_import_local_name(import: &GoSidecarImport) -> Option<String> {
    if import.local_name.is_empty() {
        None
    } else {
        Some(import.local_name.clone())
    }
}

fn go_import_alias_fact(
    interner: &crate::internal_core::StableKeyInterner,
    import: &GoSidecarImport,
    resolution: Option<&GoSidecarResolutionStep>,
    status: SemanticStatus,
    symbol_stable_keys: &BTreeMap<String, String>,
) -> Option<AliasFact> {
    if import.alias_kind == "blank" || import.alias_kind == "implicit" {
        return None;
    }
    let source_key = go_import_stable_key(import);
    Some(AliasFact {
        id: AliasId(0),
        language: Language::Go,
        file: None,
        package: None,
        module: None,
        source_symbol_stable_key: interner.intern(source_key.clone()),
        target_symbol_stable_keys: resolution
            .map(|step| mapped_candidate_keys(step, symbol_stable_keys))
            .unwrap_or_default()
            .into_iter()
            .map(|key| interner.intern(key))
            .collect(),
        kind: AliasKind::ImportAlias,
        stable_key: interner.intern(format!("{source_key}|alias")),
        status,
    })
}

fn go_import_stable_key(import: &GoSidecarImport) -> String {
    format!(
        "go:import|file:{}|path:{}|local:{}|span:{}-{}",
        import.file, import.path, import.local_name, import.span.start_byte, import.span.end_byte
    )
}

fn go_export_stable_key(export: &GoSidecarExport) -> String {
    format!(
        "go:export|package:{}|namespace:{}|name:{}|object_path:{}",
        export.package_path, export.namespace, export.export_name, export.object_path
    )
}

fn go_stable_export_key(export: &GoSidecarExport) -> String {
    format!(
        "{}|symbol:{}|discriminator:native",
        go_export_stable_key(export),
        export.symbol_key
    )
}

fn go_resolution_stable_key(step: &GoSidecarResolutionStep) -> String {
    format!(
        "go:resolution|reference:{}|step:{}|status:{}",
        step.reference_key, step.step, step.status
    )
}

fn go_reference_key(reference: &GoSidecarReference) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        reference.package_id,
        reference.file,
        reference.name,
        reference.target_key,
        reference.kind,
        reference.span.start_byte,
        reference.span.end_byte
    )
}

fn go_reference_unknown_fallback_key(reference: &GoSidecarReference) -> String {
    format!(
        "go:resolution|reference:{}|step:UnknownFallback|status:unresolved",
        go_reference_key(reference)
    )
}

fn semantic_candidate_symbol_ids(
    step: &GoSidecarResolutionStep,
    symbols: &BTreeMap<String, SymbolId>,
) -> Vec<SymbolId> {
    normalized_candidate_keys(step)
        .into_iter()
        .filter_map(|key| symbols.get(&key).copied())
        .collect()
}

fn go_resolution_step_kind(step: &str) -> ResolutionStepKind {
    match step {
        "LexicalLookup" | "lexical" => ResolutionStepKind::LexicalLookup,
        "ImportAliasLookup" | "import_alias" => ResolutionStepKind::ImportAliasLookup,
        "Package" | "package" => ResolutionStepKind::Package,
        "ModuleLookup" | "module" => ResolutionStepKind::ModuleLookup,
        "MemberLookup" | "member" => ResolutionStepKind::MemberLookup,
        "GeneratedHintLookup" | "generated" => ResolutionStepKind::GeneratedHintLookup,
        "UnknownFallback" | "unknown_fallback" => ResolutionStepKind::UnknownFallback,
        "external" => ResolutionStepKind::External,
        "unsupported" => ResolutionStepKind::Unsupported,
        _ => ResolutionStepKind::UnknownFallback,
    }
}

fn semantic_status(status: &str) -> SemanticStatus {
    match status {
        "resolved" => SemanticStatus::Resolved,
        "ambiguous" => SemanticStatus::Ambiguous,
        "unresolved" => SemanticStatus::Unresolved,
        "cycle" => SemanticStatus::Cycle,
        "generated" => SemanticStatus::Generated,
        "dynamic" => SemanticStatus::Dynamic,
        "external" => SemanticStatus::External,
        "setup_missing" => SemanticStatus::SetupMissing,
        "unsupported" => SemanticStatus::Unsupported,
        _ => SemanticStatus::Unresolved,
    }
}

fn normalized_candidate_keys(step: &GoSidecarResolutionStep) -> Vec<String> {
    let mut keys = step.candidate_keys.clone();
    if !step.target_key.is_empty() {
        keys.push(step.target_key.clone());
    }
    keys.sort();
    keys.dedup();
    keys
}

fn mapped_candidate_keys(
    step: &GoSidecarResolutionStep,
    symbol_stable_keys: &BTreeMap<String, String>,
) -> Vec<String> {
    if symbol_stable_keys.is_empty() {
        return normalized_candidate_keys(step);
    }
    let mut keys = normalized_candidate_keys(step)
        .into_iter()
        .filter_map(|key| mapped_candidate_key(symbol_stable_keys, key))
        .collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    keys
}

fn mapped_candidate_key(
    symbol_stable_keys: &BTreeMap<String, String>,
    key: String,
) -> Option<String> {
    if let Some(mapped) = symbol_stable_keys.get(&key) {
        Some(mapped.clone())
    } else if key.starts_with("go:builtin|") {
        None
    } else {
        Some(key)
    }
}

fn mapped_symbol_key(symbol_stable_keys: &BTreeMap<String, String>, key: &str) -> String {
    symbol_stable_keys
        .get(key)
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

fn mapped_reference_key(reference_stable_keys: &BTreeMap<String, String>, key: &str) -> String {
    reference_stable_keys
        .get(key)
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

fn go_stable_symbol_key(
    symbol: &GoSidecarSymbol,
    files: &BTreeMap<&str, &SourceFile>,
) -> StableSymbolKey {
    StableSymbolKey::new(
        Language::Go,
        (!symbol.package_path.is_empty()).then(|| format!("go:module:{}", symbol.package_path)),
        Some(format!("go:sidecar:{}", symbol.key)),
        None,
        symbol.owner_chain.clone(),
        symbol_namespace(&symbol.namespace),
        symbol_kind(&symbol.kind),
        symbol.name.clone(),
        if symbol.key.starts_with("go:local") {
            span_for_file(files, &symbol.file, &symbol.span)
        } else {
            None
        },
    )
}

fn go_stable_reference_key(
    reference: &GoSidecarReference,
    files: &BTreeMap<&str, &SourceFile>,
    symbol_key_inputs: &BTreeMap<String, StableSymbolKey>,
) -> String {
    let span = span_for_file(files, &reference.file, &reference.span)
        .unwrap_or_else(|| Span::point(FileId::from_raw(u32::MAX), 1, 1));
    symbol_key_inputs
        .get(&reference.target_key)
        .cloned()
        .map_or_else(
            || {
                StableReferenceKey::unresolved(
                    Language::Go,
                    reference.file.clone(),
                    reference.name.clone(),
                    span.clone(),
                )
            },
            |target| StableReferenceKey::resolved(target, reference.file.clone(), span.clone()),
        )
        .stable_key()
}

fn symbol_draft(symbol: &GoSidecarSymbol, files: &BTreeMap<&str, &SourceFile>) -> SymbolDraft {
    let file = file_for_path(files, &symbol.file).map(|file| file.id);
    let primary_span = span_for_file(files, &symbol.file, &symbol.span);
    SymbolDraft {
        language: Language::Go,
        name: symbol.name.clone(),
        qualified_name: symbol.qualified_name.clone(),
        kind: symbol_kind(&symbol.kind),
        namespace: symbol_namespace(&symbol.namespace),
        file,
        package: None,
        module: None,
        owner: None,
        module_key: if symbol.package_path.is_empty() {
            None
        } else {
            Some(format!("go:module:{}", symbol.package_path))
        },
        package_key: Some(format!("go:sidecar:{}", symbol.key)),
        file_key: None,
        owner_chain: symbol.owner_chain.clone(),
        primary_span: if symbol.key.starts_with("go:local") {
            primary_span
        } else {
            None
        },
        is_exported: symbol.exported,
        precision: SymbolPrecision::ExactSemantic,
    }
}

fn definition_draft(
    definition: &GoSidecarDefinition,
    symbol: Option<&GoSidecarSymbol>,
    files: &BTreeMap<&str, &SourceFile>,
) -> DefinitionDraft {
    let file = file_for_path(files, &definition.file).map(|file| file.id);
    DefinitionDraft {
        language: Language::Go,
        name: definition.name.clone(),
        qualified_name: symbol
            .map(|symbol| symbol.qualified_name.clone())
            .unwrap_or_else(|| definition.name.clone()),
        kind: definition_kind(&definition.kind),
        namespace: symbol
            .map(|symbol| symbol_namespace(&symbol.namespace))
            .unwrap_or(SymbolNamespace::Unknown),
        file,
        package: None,
        module: None,
        owner: None,
        file_key: definition.file.clone(),
        primary_span: span_for_file(files, &definition.file, &definition.span),
        is_primary: definition.primary,
        is_exported: symbol.is_some_and(|symbol| symbol.exported),
        precision: SymbolPrecision::ExactSemantic,
    }
}

fn reference_draft(
    reference: &GoSidecarReference,
    files: &BTreeMap<&str, &SourceFile>,
) -> ReferenceDraft {
    let file = file_for_path(files, &reference.file).map(|file| file.id);
    ReferenceDraft {
        language: Language::Go,
        name: reference.name.clone(),
        qualified_name: reference.name.clone(),
        kind: reference_kind(&reference.kind),
        namespace: reference_namespace(&reference.kind),
        file,
        package: None,
        module: None,
        owner: None,
        file_key: reference.file.clone(),
        primary_span: span_for_file(files, &reference.file, &reference.span),
        precision: reference_precision(&reference.precision),
    }
}

fn file_for_path<'a>(files: &'a BTreeMap<&str, &SourceFile>, path: &str) -> Option<&'a SourceFile> {
    if path.is_empty() {
        None
    } else {
        files.get(path).copied()
    }
}

fn span_for_file(
    files: &BTreeMap<&str, &SourceFile>,
    path: &str,
    span: &GoSidecarSpan,
) -> Option<Span> {
    let file = file_for_path(files, path)?;
    Some(file.span_from_byte_range(span.start_byte, span.end_byte))
}

fn symbol_kind(kind: &str) -> SymbolKind {
    match kind {
        "package" => SymbolKind::Package,
        "function" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        "variable" => SymbolKind::Variable,
        "constant" => SymbolKind::Constant,
        "type" => SymbolKind::TypeAlias,
        "field" => SymbolKind::Field,
        "parameter" => SymbolKind::Parameter,
        _ => SymbolKind::Unknown,
    }
}

fn symbol_namespace(namespace: &str) -> SymbolNamespace {
    match namespace {
        "value" => SymbolNamespace::Value,
        "type" => SymbolNamespace::Type,
        "package" => SymbolNamespace::Package,
        "module" => SymbolNamespace::Module,
        _ => SymbolNamespace::Unknown,
    }
}

fn definition_kind(kind: &str) -> DefinitionKind {
    match kind {
        "declaration" => DefinitionKind::Declaration,
        "definition" => DefinitionKind::Definition,
        "import" => DefinitionKind::Import,
        "export" => DefinitionKind::Export,
        "implicit" => DefinitionKind::Implicit,
        _ => DefinitionKind::Unknown,
    }
}

fn reference_kind(kind: &str) -> ReferenceKind {
    match kind {
        "read" => ReferenceKind::Read,
        "write" => ReferenceKind::Write,
        "read_write" => ReferenceKind::ReadWrite,
        "call" => ReferenceKind::Call,
        "type" => ReferenceKind::TypeUse,
        "package" => ReferenceKind::Import,
        "field" | "method" | "member" => ReferenceKind::MemberAccess,
        _ => ReferenceKind::Unknown,
    }
}

fn reference_namespace(kind: &str) -> SymbolNamespace {
    match kind {
        "type" => SymbolNamespace::Type,
        "package" => SymbolNamespace::Package,
        _ => SymbolNamespace::Value,
    }
}

fn reference_precision(precision: &str) -> SymbolPrecision {
    match precision {
        "exact_semantic" => SymbolPrecision::ExactSemantic,
        "exact_local" => SymbolPrecision::ExactLocal,
        "module_linked" => SymbolPrecision::ModuleLinked,
        "heuristic" => SymbolPrecision::Heuristic,
        "setup_missing" => SymbolPrecision::SetupMissing,
        "unsupported" => SymbolPrecision::Unsupported,
        _ => SymbolPrecision::Unsupported,
    }
}

fn package_error_diagnostics(errors: &[GoSidecarPackageError]) -> Vec<Diagnostic> {
    let mut diagnostics = errors
        .iter()
        .map(|error| {
            Diagnostic::error(
                "polint/capability",
                "<workspace>",
                TextRange::point(1, 1),
                format!(
                    "Go package `{}` loaded with errors while deriving symbols and references.",
                    error.package_path
                ),
            )
            .with_evidence("capability", "symbols".to_string())
            .with_evidence("language", "Go".to_string())
            .with_evidence("package_id", error.package_id.clone())
            .with_evidence("package_path", error.package_path.clone())
            .with_evidence("reason", error.message.clone())
            .with_evidence("docs_path", SYMBOL_FACTS_DOCS_PATH.to_string())
            .with_help(format!(
                "Fix Go package setup for `{}` to make symbol/reference coverage complete.",
                error.package_path
            ))
        })
        .collect::<Vec<_>>();
    diagnostics.sort_by(|left, right| {
        (
            left.file.as_str(),
            left.message.as_str(),
            left.stable_fingerprint.as_str(),
        )
            .cmp(&(
                right.file.as_str(),
                right.message.as_str(),
                right.stable_fingerprint.as_str(),
            ))
    });
    diagnostics
}

#[cfg(test)]
mod subprocess_timeout_tests {
    use super::*;

    #[test]
    fn symbol_sidecar_command_times_out_instead_of_hanging() {
        let command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/C", "ping -n 3 127.0.0.1 > nul"]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 2"]);
            command
        };
        let error = run_go_sidecar_command(command, Duration::from_millis(1))
            .expect_err("sidecar should time out");
        assert!(matches!(error, GoSidecarFailure::Timeout(_)));
    }
}

#[cfg(test)]
include!("symbol_graph_tests.rs");
#[cfg(test)]
include!("symbol_graph_setup_tests.rs");
#[cfg(test)]
include!("symbol_graph_value_tests.rs");
