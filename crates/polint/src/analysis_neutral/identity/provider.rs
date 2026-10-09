use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::analysis_api::FunctionFact;
use crate::analysis_api::ProviderManifest;
use crate::analysis_api::{
    CacheStats, Digest, DigestKind, InputSnapshot, ProviderExecution, ProviderFailureReason,
    ProviderFailureStage,
};
use crate::analysis_neutral::calls::facts::CallSiteFact;
use crate::analysis_neutral::identity::cache_key::identity_provider_parameter_digest;
use crate::analysis_neutral::identity::dedup::dedup_identity_records;
use crate::analysis_neutral::identity::facts::{
    IdentityKind, IdentityRecord, IdentityRecordId, LanguageTag, compute_identity_stable_key,
    compute_signature_digest,
};
use crate::analysis_neutral::identity::store::IdentityProviderOutput;
use crate::analysis_neutral::ids::CallSiteId;
use crate::internal_core::{
    Diagnostic, DiagnosticRange, FileId, Language, Span, StableKeyInterner,
};

use crate::analysis_neutral::AnalysisHost;

#[derive(Debug, Clone, Default)]
pub struct IdentityProviderRunOutput {
    pub diagnostics: Vec<Diagnostic>,
    pub cache_stats: CacheStats,
    pub output_digest: Option<Digest>,
    pub execution: ProviderExecution,
}

/// Identity provider entry point (Pattern E).
///
/// Five-stage pipeline: extract identity records by projecting existing
/// `analysis::calls` and function facts (no mutation, D-04) -> dedup (D-09) ->
/// assign dense IDs after sort+dedup -> normalize -> compute output digest over
/// stable payloads (Pattern F) and replace identity facts.
#[allow(clippy::too_many_arguments)]
pub fn derive_identity_with_cache_stats(
    db: &mut impl AnalysisHost,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    calls_provider_output_digest: Digest,
    go_semantic_output_digest: Digest,
    go_semantic_package_paths: &BTreeMap<FileId, String>,
) -> IdentityProviderRunOutput {
    // Step: extract.
    let mut records = extract_identity_records(db, go_semantic_package_paths);
    // Step: dedup.
    records = dedup_identity_records(records);
    // Step: assign dense IDs after sort+dedup.
    for (index, record) in records.iter_mut().enumerate() {
        record.id = IdentityRecordId(index as u64);
    }
    // Step: normalize (dedup already sorts, but normalize keeps the contract
    // single-sourced through IdentityProviderOutput).
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let output = IdentityProviderOutput { records }.normalized(interner);
    // Step: digest.
    let output_digest = identity_output_digest(
        interner,
        manifest,
        input_snapshot,
        &calls_provider_output_digest,
        &go_semantic_output_digest,
        &output,
    );

    let mut cache_stats = CacheStats::default();
    cache_stats.record_recompute();

    match db.replace_identity_facts(output) {
        Ok(()) => IdentityProviderRunOutput {
            diagnostics: Vec::new(),
            cache_stats,
            output_digest: Some(output_digest),
            execution: Default::default(),
        },
        Err(error) => IdentityProviderRunOutput {
            diagnostics: vec![provider_error_diagnostic(error.to_string())],
            cache_stats,
            output_digest: None,
            execution: ProviderExecution::Failed {
                stage: ProviderFailureStage::Validation,
                reason: ProviderFailureReason::ValidationRejected,
            },
        },
    }
}

/// Projects existing function and call-site facts into identity records.
fn extract_identity_records(
    db: &impl AnalysisHost,
    go_semantic_package_paths: &BTreeMap<FileId, String>,
) -> Vec<IdentityRecord> {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut records = Vec::new();
    // Every record asks for its file's package and every call-site record for its
    // caller's name; resolve both once instead of scanning per record. A Go file
    // the semantic sidecar gave no package path falls back to its first package
    // clause, as `package_or_module_for_record` would.
    let mut package_paths = go_semantic_package_paths.clone();
    let mut clause_files = std::collections::BTreeSet::new();
    for package in db
        .packages()
        .iter()
        .filter(|package| package.language == Language::Go)
    {
        if !clause_files.insert(package.file) {
            continue;
        }
        if package_paths
            .get(&package.file)
            .is_none_or(|path| path.is_empty())
        {
            package_paths.insert(package.file, package.name.clone());
        }
    }
    let mut function_names = std::collections::HashMap::new();
    for function in db.functions() {
        function_names
            .entry(function.id)
            .or_insert(function.name.as_str());
    }

    for function in db.functions() {
        if let Some(record) = function_identity_record(db, interner, function, &package_paths) {
            records.push(record);
        }
    }

    for site in db.call_sites() {
        if let Some(record) =
            callsite_identity_record(db, interner, site, &package_paths, &function_names)
        {
            records.push(record);
        }
    }

    records
}

pub fn function_identity_record(
    db: &impl AnalysisHost,
    interner: &StableKeyInterner,
    function: &FunctionFact,
    go_semantic_package_paths: &BTreeMap<FileId, String>,
) -> Option<IdentityRecord> {
    let language = language_tag(function.language)?;
    let package_or_module: Arc<str> = Arc::from(package_or_module_for_record(
        db,
        function.language,
        function.file,
        go_semantic_package_paths,
    ));
    let container_path: Arc<str> = Arc::from(function.name.as_str());
    let display_name: Arc<str> = Arc::from(function.name.as_str());
    Some(build_record(
        interner,
        &db.path_for(function.file),
        IdentityKind::Function,
        function.file,
        &function.span,
        language,
        package_or_module,
        container_path,
        display_name,
        None,
    ))
}

fn callsite_identity_record(
    db: &impl AnalysisHost,
    interner: &StableKeyInterner,
    site: &CallSiteFact,
    go_semantic_package_paths: &BTreeMap<FileId, String>,
    function_names: &std::collections::HashMap<crate::internal_core::FunctionId, &str>,
) -> Option<IdentityRecord> {
    let language = language_tag(site.language)?;
    let package_or_module: Arc<str> = Arc::from(package_or_module_for_record(
        db,
        site.language,
        site.file,
        go_semantic_package_paths,
    ));
    let container_path: Arc<str> = Arc::from(callsite_container_path(
        db,
        site,
        go_semantic_package_paths,
        function_names,
    ));
    let display_name: Arc<str> = Arc::from(callsite_display_name(site));
    Some(build_record(
        interner,
        &db.path_for(site.file),
        IdentityKind::Callsite,
        site.file,
        &site.span,
        language,
        package_or_module,
        container_path,
        display_name,
        Some(site.id),
    ))
}

#[allow(clippy::too_many_arguments)]
fn build_record(
    interner: &StableKeyInterner,
    repository_relative_path: &str,
    kind: IdentityKind,
    file: FileId,
    span: &Span,
    language: LanguageTag,
    package_or_module: Arc<str>,
    container_path: Arc<str>,
    display_name: Arc<str>,
    originating_call_site_id: Option<CallSiteId>,
) -> IdentityRecord {
    let signature_digest = compute_signature_digest(
        language,
        &package_or_module,
        &container_path,
        &display_name,
        None,
        None,
    );
    let stable_key = interner.intern(compute_identity_stable_key(
        kind,
        language,
        &package_or_module,
        &container_path,
        repository_relative_path,
        span,
    ));
    IdentityRecord {
        id: IdentityRecordId(0),
        kind,
        file_id: file,
        span: span.clone(),
        language,
        package_or_module,
        container_path,
        display_name,
        signature_digest,
        multiplicity: 1,
        stable_key,
        originating_call_site_id,
        originating_call_target_id: None,
    }
}

fn callsite_container_path(
    db: &impl AnalysisHost,
    site: &CallSiteFact,
    go_semantic_package_paths: &BTreeMap<FileId, String>,
    function_names: &std::collections::HashMap<crate::internal_core::FunctionId, &str>,
) -> String {
    // The container is the enclosing caller function. Fall back to a file-scoped
    // container when the caller cannot be resolved by name.
    function_names
        .get(&site.caller)
        .map(|name| (*name).to_string())
        .unwrap_or_else(|| {
            format!(
                "{}#<anon>",
                package_or_module_for_record(
                    db,
                    site.language,
                    site.file,
                    go_semantic_package_paths,
                )
            )
        })
}

fn callsite_display_name(site: &CallSiteFact) -> String {
    use crate::analysis_neutral::calls::facts::CallCallee;
    match &site.callee {
        CallCallee::Identifier { name, .. } => name.clone(),
        CallCallee::Member { property, .. } => property.clone(),
        CallCallee::Constructor { name, .. } => {
            name.clone().unwrap_or_else(|| "<ctor>".to_string())
        }
        CallCallee::Index { .. } => "<index>".to_string(),
        CallCallee::Super => "super".to_string(),
        CallCallee::Import => "import".to_string(),
        CallCallee::FunctionValue { .. } => "<function-value>".to_string(),
        CallCallee::Unknown { .. } => "<unknown>".to_string(),
    }
}

/// Resolves the `package_or_module` string for a record's language and file.
///
/// For `Language::Go` records this prefers the semantic frontend's full
/// Go package import path when a validated package row covers the file. It falls
/// back to the Go package-clause name, then the workspace-relative file path, so
/// missing semantic setup preserves the earlier panic-free behavior.
fn package_or_module_for_record(
    db: &impl AnalysisHost,
    language: Language,
    file: FileId,
    go_semantic_package_paths: &BTreeMap<FileId, String>,
) -> String {
    match language {
        Language::Go => go_semantic_package_paths
            .get(&file)
            .filter(|package_path| !package_path.is_empty())
            .cloned()
            .or_else(|| package_name_for_go_file(db, file))
            .unwrap_or_else(|| package_or_module_for_file(db, file)),
        _ => package_or_module_for_file(db, file),
    }
}

/// Returns the Go package-clause name (e.g. `main` / `foo`) for a file by
/// scanning `db.packages()` for the first Go [`PackageFact`] on that file.
fn package_name_for_go_file(db: &impl AnalysisHost, file: FileId) -> Option<String> {
    db.packages()
        .iter()
        .find(|package| package.file == file && package.language == Language::Go)
        .map(|package| package.name.clone())
}

fn package_or_module_for_file(db: &impl AnalysisHost, file: FileId) -> String {
    db.path_for(file)
}

fn language_tag(language: Language) -> Option<LanguageTag> {
    match language {
        Language::Go => Some(LanguageTag::Go),
        Language::TypeScript | Language::Tsx => Some(LanguageTag::TypeScript),
        Language::JavaScript | Language::Jsx => Some(LanguageTag::JavaScript),
        Language::Unknown => None,
        _ => None,
    }
}

/// Output digest over stable payloads, never dense IDs (Pattern F, T-42-02).
pub fn identity_output_digest(
    interner: &StableKeyInterner,
    manifest: &ProviderManifest,
    input_snapshot: &InputSnapshot,
    calls_provider_output_digest: &Digest,
    go_semantic_output_digest: &Digest,
    output: &IdentityProviderOutput,
) -> Digest {
    let mut parts = vec![
        format!("provider_id={}", manifest.id),
        format!("provider_version={}", manifest.provider_version()),
        format!("schema={}", manifest.primary_schema_label()),
        format!("parameters={}", identity_provider_parameter_digest()),
        format!("config={}", input_snapshot.config.digest),
        format!("calls_output={calls_provider_output_digest}"),
        format!("go_semantic_output={go_semantic_output_digest}"),
    ];
    parts.extend(output.records.iter().map(|record| {
        format!(
            "identity_record={} language={} package_or_module={} container={} digest={} kind={:?} multiplicity={}",
            interner.resolve(record.stable_key),
            record.language.as_str(),
            record.package_or_module,
            record.container_path,
            encode_digest_hex(record.signature_digest.0),
            record.kind,
            record.multiplicity,
        )
    }));
    if output.records.is_empty() {
        parts.push("identity_output=empty".to_string());
    }
    parts.sort();
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Digest::from_parts(DigestKind::ProviderOutput, "identity_output", &refs)
}

fn encode_digest_hex(bytes: [u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(32);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn provider_error_diagnostic(message: String) -> Diagnostic {
    Diagnostic::error(
        "polint/internal",
        "<workspace>",
        DiagnosticRange::point(1, 1),
        format!("Identity provider failed: {message}"),
    )
}

/// Valid call-site ID set for store validation by the kernel caller.
pub fn valid_call_site_ids(db: &impl AnalysisHost) -> BTreeSet<CallSiteId> {
    db.call_sites().iter().map(|site| site.id).collect()
}
