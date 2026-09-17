use std::collections::BTreeMap;
use std::path::Path;

use toml::Value;

use crate::analysis_api::{
    CacheStats, Digest, DigestKind, FactDatabase, ProviderExecution, ProviderFailureReason,
    ProviderFailureStage, ProviderManifest,
};
use crate::internal_core::{Diagnostic, DiagnosticRange as TextRange, Span};

use crate::ts::types::cache_key::{
    TsTypesCacheInputs, ts_types_input_digest, ts_types_provider_parameter_digest,
};
use crate::ts::types::client::{TsTypesClient, TsTypesClientError, TsTypesClientRun};
use crate::ts::types::diagnostics::TsTypesDiagnosticCategory;
use crate::ts::types::lifecycle::{TsTypesConfig, ts_files};
use crate::ts::types::lower::lower_ts_types;
use crate::ts::types::process::TsTypesProcessError;
use crate::ts::types::store::{
    TS_TYPES_STORE_FAMILY, TsTypesFactsOutput, TsTypesStore, TsTypesStoreReport,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct TsTypesProviderRunOutput {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) cache_stats: CacheStats,
    pub(crate) output_digest: Option<Digest>,
    pub(crate) execution: ProviderExecution,
    /// Sidecar stage timings and workload sizes, keyed for the run report.
    /// Empty when the sidecar did not run or reported no phases.
    pub(crate) counts: BTreeMap<String, u64>,
}

/// Where [`TsTypesClient::run_cached`] reads and writes stored sidecar output.
#[derive(Default)]
pub(crate) struct TsTypesSidecarAccess<'a> {
    pub(crate) cache_dir: Option<&'a Path>,
}

pub(crate) fn derive_ts_types_with_cache_stats(
    db: &mut dyn FactDatabase,
    root: &Path,
    ts_settings: &BTreeMap<String, Value>,
    config_digest: &str,
    manifest: &ProviderManifest,
    ts_syntax_output_digest: Digest,
    sidecar: TsTypesSidecarAccess<'_>,
) -> TsTypesProviderRunOutput {
    let upstream = ts_syntax_output_digest.to_string();
    let cache_dir = sidecar.cache_dir.map(Path::to_path_buf);
    let root_owned = root.to_path_buf();
    derive_ts_types_with_runner(
        db,
        root,
        ts_settings,
        config_digest,
        manifest,
        ts_syntax_output_digest,
        move |config| match cache_dir.as_deref() {
            Some(dir) => TsTypesClient::new(root_owned, config).run_cached(config, dir, &upstream),
            None => TsTypesClient::new(root_owned, config).run(config),
        },
    )
}

/// Runs the provider against a scripted sidecar result.
///
/// Every failure mode this provider has to survive is a sidecar outcome, and
/// none of them should need a Node process to test.
#[cfg(test)]
pub(crate) fn derive_ts_types_with_runner_for_test(
    db: &mut dyn FactDatabase,
    root: &Path,
    ts_settings: &BTreeMap<String, Value>,
    config_digest: &str,
    manifest: &ProviderManifest,
    ts_syntax_output_digest: Digest,
    runner: impl FnOnce(&TsTypesConfig) -> Result<TsTypesClientRun, TsTypesClientError>,
) -> TsTypesProviderRunOutput {
    derive_ts_types_with_runner(
        db,
        root,
        ts_settings,
        config_digest,
        manifest,
        ts_syntax_output_digest,
        runner,
    )
}

fn derive_ts_types_with_runner(
    db: &mut dyn FactDatabase,
    root: &Path,
    ts_settings: &BTreeMap<String, Value>,
    config_digest: &str,
    manifest: &ProviderManifest,
    ts_syntax_output_digest: Digest,
    runner: impl FnOnce(&TsTypesConfig) -> Result<TsTypesClientRun, TsTypesClientError>,
) -> TsTypesProviderRunOutput {
    debug_assert_eq!(manifest.id, "polint.ts.types");
    let mut cache_stats = CacheStats::default();
    cache_stats.record_recompute();

    let files = ts_files(db);
    if files.is_empty() {
        return store_output(
            db,
            config_digest,
            manifest,
            StoreOutputParts {
                ts_syntax_output_digest,
                output: TsTypesFactsOutput::default(),
                lifecycle: TsTypesConfig::default(),
                digest_inputs: NotRunInputs::from_reason("no TypeScript sources").into(),
                cache_stats,
                diagnostics: Vec::new(),
                execution: ProviderExecution::Succeeded,
            },
        );
    }

    let config = match TsTypesConfig::from_settings_files(root, ts_settings, &files) {
        Ok(config) => config,
        Err(error) => {
            return store_output(
                db,
                config_digest,
                manifest,
                StoreOutputParts {
                    ts_syntax_output_digest,
                    output: TsTypesFactsOutput::default(),
                    lifecycle: TsTypesConfig::default(),
                    digest_inputs: NotRunInputs::from_reason(error.reason()).into(),
                    cache_stats,
                    diagnostics: vec![category_diagnostic(
                        TsTypesDiagnosticCategory::SetupMissing,
                        error.reason().to_string(),
                    )],
                    execution: ProviderExecution::Failed {
                        stage: ProviderFailureStage::Setup,
                        reason: ProviderFailureReason::SetupMissing,
                    },
                },
            );
        }
    };

    // A turned-off tier is a successful no-op: the heap tier answers the
    // repository and nothing is blocked.
    if !config.enabled {
        return store_output(
            db,
            config_digest,
            manifest,
            StoreOutputParts {
                ts_syntax_output_digest,
                output: TsTypesFactsOutput::default(),
                lifecycle: config,
                digest_inputs: NotRunInputs::from_reason("disabled by configuration").into(),
                cache_stats,
                diagnostics: Vec::new(),
                execution: ProviderExecution::Succeeded,
            },
        );
    }

    if config.projects.is_empty() {
        return setup_gap(
            db,
            config_digest,
            manifest,
            ts_syntax_output_digest,
            config,
            cache_stats,
            "no tsconfig.json",
            "no tsconfig.json was found above the discovered TypeScript files; \
             type-directed TS analysis needs one. Call-graph answers fall back to \
             the points-to tier.",
        );
    }

    let missing = config.missing_projects(root);
    if !missing.is_empty() {
        let message = format!(
            "configured TypeScript projects are missing: {}.",
            missing.join(", ")
        );
        return setup_gap(
            db,
            config_digest,
            manifest,
            ts_syntax_output_digest,
            config,
            cache_stats,
            "missing configured projects",
            &message,
        );
    }

    let run = match runner(&config) {
        Ok(run) => run,
        Err(error) => {
            if is_setup_gap(&error) {
                let message = error.to_string();
                return setup_gap(
                    db,
                    config_digest,
                    manifest,
                    ts_syntax_output_digest,
                    config,
                    cache_stats,
                    "toolchain unavailable",
                    &message,
                );
            }
            // Everything past setup is a real failure: the sidecar was found
            // and started, so a timeout, a crash or an unreadable wire is a
            // problem to report rather than a repository the tier skips.
            let (diagnostic, stage, reason) = client_failure(&error);
            return store_output(
                db,
                config_digest,
                manifest,
                StoreOutputParts {
                    ts_syntax_output_digest,
                    output: TsTypesFactsOutput::default(),
                    lifecycle: config,
                    digest_inputs: NotRunInputs::from_reason(&format!("client:{error}")).into(),
                    cache_stats,
                    diagnostics: vec![diagnostic],
                    execution: ProviderExecution::Failed { stage, reason },
                },
            );
        }
    };

    let mut counts = phase_counts(&run.output);
    let (lowered, lower_report) = lower_ts_types(db, &run.output);
    counts.insert(
        "ts_types.out_of_scope_rows".to_string(),
        lower_report.out_of_scope_rows as u64,
    );
    let mut diagnostics = project_error_diagnostics(&lowered);
    diagnostics.extend(uncovered_files_diagnostic(&config));

    let digest_inputs = DigestInputs {
        sidecar_digest: run.sidecar_digest,
        typescript_version: run.output.typescript_version.clone(),
        node_version: run.output.node_version,
    };
    let mut stored = store_output(
        db,
        config_digest,
        manifest,
        StoreOutputParts {
            ts_syntax_output_digest,
            output: lowered,
            lifecycle: config,
            digest_inputs,
            cache_stats,
            diagnostics,
            execution: ProviderExecution::Succeeded,
        },
    );
    for (key, value) in counts {
        stored.counts.insert(key, value);
    }
    stored
}

/// Installs an empty store for a tier that could not run, and decides whether
/// the gap is worth telling the user about.
///
/// A repository with no TypeScript compiler and no mention of this tier in its
/// configuration is not misconfigured: the tier does not apply, the points-to
/// tier answers its calls, and a warning on every scan would be noise. A
/// repository that named the tier asked for it, so the same gap becomes a
/// reported setup failure. Either way the provider row is in the run report
/// with a zero row count, so the skip is never invisible.
#[allow(clippy::too_many_arguments)]
fn setup_gap(
    db: &mut dyn FactDatabase,
    config_digest: &str,
    manifest: &ProviderManifest,
    ts_syntax_output_digest: Digest,
    config: TsTypesConfig,
    cache_stats: CacheStats,
    digest_reason: &str,
    message: &str,
) -> TsTypesProviderRunOutput {
    let requested = config.explicitly_requested;
    tracing::info!(
        target: "polint::kernel::stage",
        provider = "polint.ts.types",
        requested,
        reason = message,
        "type-directed TS analysis did not run"
    );
    let mut output = store_output(
        db,
        config_digest,
        manifest,
        StoreOutputParts {
            ts_syntax_output_digest,
            output: TsTypesFactsOutput::default(),
            lifecycle: config,
            digest_inputs: NotRunInputs::from_reason(digest_reason).into(),
            cache_stats,
            diagnostics: if requested {
                vec![category_diagnostic(
                    TsTypesDiagnosticCategory::SetupMissing,
                    message.to_string(),
                )]
            } else {
                Vec::new()
            },
            execution: if requested {
                ProviderExecution::Failed {
                    stage: ProviderFailureStage::Setup,
                    reason: ProviderFailureReason::SetupMissing,
                }
            } else {
                ProviderExecution::Succeeded
            },
        },
    );
    output
        .counts
        .insert("ts_types.setup_missing".to_string(), 1);
    output
}

/// Whether a client failure means the toolchain is absent rather than broken.
fn is_setup_gap(error: &TsTypesClientError) -> bool {
    matches!(
        error,
        TsTypesClientError::Process(
            TsTypesProcessError::SetupMissing(_)
                | TsTypesProcessError::VersionUnsupported(_)
                | TsTypesProcessError::CommandUnavailable(_)
        )
    )
}

/// Folds the sidecar's phase frames into report counters and logs each stage.
///
/// Timings are the answer to "why was this run slow", so they are logged on the
/// same target as every other kernel stage and carried into the run report
/// rather than staying stderr-only.
fn phase_counts(output: &crate::ts::types::protocol::TsTypesOutput) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for phase in &output.phases {
        tracing::debug!(
            target: "polint::kernel::stage",
            provider = "polint.ts.types",
            phase = phase.phase.as_str(),
            elapsed_ms = phase.elapsed_ms,
            projects = phase.projects,
            files = phase.files,
            rows_emitted = phase.rows_emitted,
            peak_heap_bytes = phase.peak_heap_bytes,
            "ts types phase"
        );
        // A repository with several projects closes the same stage once per
        // project, so the counter accumulates rather than keeping the last.
        *counts
            .entry(format!("ts_types.phase.{}.elapsed_ms", phase.phase))
            .or_insert(0) += phase.elapsed_ms;
    }
    let totals = &output.totals;
    if totals.elapsed_ms > 0 || totals.projects > 0 {
        tracing::info!(
            target: "polint::kernel::stage",
            provider = "polint.ts.types",
            elapsed_ms = totals.elapsed_ms,
            projects = totals.projects,
            files = totals.files,
            rows_emitted = totals.rows_emitted,
            peak_heap_bytes = totals.peak_heap_bytes,
            "ts types sidecar totals"
        );
        counts.insert("ts_types.elapsed_ms".to_string(), totals.elapsed_ms);
        counts.insert("ts_types.projects".to_string(), totals.projects);
        counts.insert("ts_types.files".to_string(), totals.files);
        counts.insert("ts_types.rows_emitted".to_string(), totals.rows_emitted);
        counts.insert(
            "ts_types.peak_heap_bytes".to_string(),
            totals.peak_heap_bytes,
        );
    }
    counts
}

#[derive(Debug, Clone)]
struct DigestInputs {
    sidecar_digest: String,
    typescript_version: String,
    node_version: String,
}

#[derive(Debug, Clone, Default)]
struct NotRunInputs {
    reason: String,
}

impl NotRunInputs {
    fn from_reason(reason: &str) -> Self {
        Self {
            reason: reason.to_string(),
        }
    }
}

impl From<NotRunInputs> for DigestInputs {
    fn from(inputs: NotRunInputs) -> Self {
        Self {
            sidecar_digest: format!("not-run:{}", inputs.reason),
            typescript_version: "not-run".to_string(),
            node_version: "not-run".to_string(),
        }
    }
}

struct StoreOutputParts {
    ts_syntax_output_digest: Digest,
    output: TsTypesFactsOutput,
    lifecycle: TsTypesConfig,
    cache_stats: CacheStats,
    diagnostics: Vec<Diagnostic>,
    digest_inputs: DigestInputs,
    execution: ProviderExecution,
}

fn store_output(
    db: &mut dyn FactDatabase,
    config_digest: &str,
    manifest: &ProviderManifest,
    parts: StoreOutputParts,
) -> TsTypesProviderRunOutput {
    let interner = db.stable_key_interner();
    let execution = parts.execution;
    match replace_ts_types_facts(db, parts.output) {
        // The digest is issued only for an explicitly successful execution.
        // Setup and execution failures install an empty store but do not
        // certify it as a usable provider output.
        Ok(report) if execution == ProviderExecution::Succeeded => {
            let stored_output = ts_types_facts_output(db);
            let output_digest = ts_types_output_digest(
                manifest,
                config_digest,
                &parts.ts_syntax_output_digest,
                &parts.digest_inputs,
                &parts.lifecycle,
                &stored_output,
                &interner,
            );
            let mut diagnostics = parts.diagnostics;
            diagnostics.extend(dropped_rows_diagnostics(&report));
            TsTypesProviderRunOutput {
                counts: BTreeMap::new(),
                diagnostics,
                cache_stats: parts.cache_stats,
                output_digest: Some(output_digest),
                execution,
            }
        }
        Ok(_report) => TsTypesProviderRunOutput {
            counts: BTreeMap::new(),
            diagnostics: parts.diagnostics,
            cache_stats: parts.cache_stats,
            output_digest: None,
            execution,
        },
        Err(error) => TsTypesProviderRunOutput {
            counts: BTreeMap::new(),
            diagnostics: vec![provider_error_diagnostic(error.to_string())],
            cache_stats: parts.cache_stats,
            output_digest: None,
            execution: ProviderExecution::Failed {
                stage: ProviderFailureStage::Validation,
                reason: ProviderFailureReason::ValidationRejected,
            },
        },
    }
}

fn ts_types_output_digest(
    manifest: &ProviderManifest,
    config_digest: &str,
    ts_syntax_output_digest: &Digest,
    digest_inputs: &DigestInputs,
    lifecycle: &TsTypesConfig,
    output: &TsTypesFactsOutput,
    interner: &crate::internal_core::StableKeyInterner,
) -> Digest {
    let input_digest = ts_types_input_digest(&TsTypesCacheInputs {
        sidecar_digest: digest_inputs.sidecar_digest.clone(),
        typescript_version: digest_inputs.typescript_version.clone(),
        node_version: digest_inputs.node_version.clone(),
        upstream_digest: ts_syntax_output_digest.to_string(),
        lifecycle: lifecycle.clone(),
    });
    let mut parts = vec![
        format!("provider_id={}", manifest.id),
        format!("provider_version={}", manifest.provider_version()),
        format!("schema={}", manifest.primary_schema_label()),
        format!("parameters={}", ts_types_provider_parameter_digest()),
        format!("config={config_digest}"),
        format!("ts_syntax={ts_syntax_output_digest}"),
        format!("input_digest={input_digest}"),
    ];
    parts.extend(output.projects.iter().map(|project| {
        format!(
            "project={} path={} options={} typescript={} files={}",
            interner.resolve(project.stable_key),
            project.project,
            project.options_digest,
            project.typescript_version,
            project.file_count
        )
    }));
    parts.extend(output.callables.iter().map(|callable| {
        format!(
            "callable={} name={} kind={:?} file={} span={}",
            interner.resolve(callable.stable_key),
            callable.name,
            callable.kind,
            callable.relative_file.as_deref().unwrap_or(""),
            option_span_part(callable.span.as_ref())
        )
    }));
    parts.extend(output.callsites.iter().map(|callsite| {
        format!(
            "callsite={} status={:?} kind={} enclosing={} file={} span={}",
            interner.resolve(callsite.stable_key),
            callsite.status,
            callsite.call_kind,
            callsite.enclosing.as_deref().unwrap_or(""),
            callsite.relative_file.as_deref().unwrap_or(""),
            option_span_part(callsite.span.as_ref())
        )
    }));
    parts.extend(output.callees.iter().map(|callee| {
        format!(
            "callee={} site={} callable={} external={} dispatch={:?} span={}",
            interner.resolve(callee.stable_key),
            interner.resolve(callee.callsite_stable_key),
            callee.callable.as_deref().unwrap_or(""),
            callee.external.as_deref().unwrap_or(""),
            callee.dispatch,
            option_span_part(callee.span.as_ref())
        )
    }));
    parts.extend(output.receivers.iter().map(|receiver| {
        format!(
            "receiver={} site={} printed={} any={} unknown={} union={}",
            interner.resolve(receiver.stable_key),
            interner.resolve(receiver.callsite_stable_key),
            receiver.printed,
            receiver.is_any,
            receiver.is_unknown,
            receiver.union_size
        )
    }));
    // The density counts gate which typed edges survive, so a file whose `any`
    // share moved must invalidate the tier's output even when no row moved.
    parts.extend(output.file_densities.iter().map(|density| {
        format!(
            "any_density={} file={} callsites={} any={}",
            interner.resolve(density.stable_key),
            density.relative_file,
            density.callsites,
            density.any_receivers
        )
    }));
    parts.extend(output.project_errors.iter().map(|error| {
        format!(
            "project_error={} category={} message={}",
            interner.resolve(error.stable_key),
            error.category,
            error.message
        )
    }));
    if output.is_empty() {
        parts.push("ts_types_output=empty".to_string());
    }
    parts.sort();
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Digest::from_parts(DigestKind::ProviderOutput, "ts_types_output", &refs)
}

fn option_span_part(span: Option<&Span>) -> String {
    span.map(span_part).unwrap_or_else(|| "none".to_string())
}

fn span_part(span: &Span) -> String {
    format!(
        "{}:{}..{}:{}@{}..{}",
        span.start_line,
        span.start_col,
        span.end_line,
        span.end_col,
        span.start_byte,
        span.end_byte
    )
}

fn project_error_diagnostics(output: &TsTypesFactsOutput) -> Vec<Diagnostic> {
    output
        .project_errors
        .iter()
        .map(|error| {
            let category = match error.category.as_str() {
                "setup_missing" => TsTypesDiagnosticCategory::SetupMissing,
                _ => TsTypesDiagnosticCategory::ProjectError,
            };
            let location = error
                .relative_file
                .as_deref()
                .map(|path| format!("{path}: "))
                .unwrap_or_default();
            category_diagnostic(category, format!("{location}{}", error.message))
        })
        .collect()
}

/// Says so when the tier covers only part of the repository.
///
/// A TS file with no `tsconfig.json` above it gets no typed answers, which is a
/// thinner call graph for that file and not an error. Reporting it is the
/// difference between a known gap and an invisible one.
fn uncovered_files_diagnostic(config: &TsTypesConfig) -> Option<Diagnostic> {
    if config.files_without_project.is_empty() || !config.explicitly_requested {
        return None;
    }
    Some(category_diagnostic(
        TsTypesDiagnosticCategory::SetupMissing,
        format!(
            "{} TypeScript/JavaScript file(s) have no tsconfig.json above them and get no \
             type-directed call edges; the points-to tier answers them instead.",
            config.files_without_project.len()
        ),
    ))
}

/// Observable signal that rows were dropped at the store boundary.
///
/// Not fatal: one malformed row must not zero the typed tier for the whole
/// repository. But it must be visible, because a systematic emitter regression
/// otherwise looks like a repository that simply has fewer typed calls.
fn dropped_rows_diagnostics(report: &TsTypesStoreReport) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if report.dropped_rows > 0 {
        diagnostics.push(category_diagnostic(
            TsTypesDiagnosticCategory::ProjectError,
            format!(
                "{} TS type row(s) dropped (missing or duplicate identity); \
                 type-directed call edges may be thinner than the sources allow.",
                report.dropped_rows
            ),
        ));
    }
    if report.dangling_callees > 0 {
        diagnostics.push(category_diagnostic(
            TsTypesDiagnosticCategory::ProjectError,
            format!(
                "{} TS type callee row(s) named a call site this run does not have; \
                 those targets were dropped.",
                report.dangling_callees
            ),
        ));
    }
    diagnostics
}

fn client_failure(
    error: &TsTypesClientError,
) -> (Diagnostic, ProviderFailureStage, ProviderFailureReason) {
    match error {
        TsTypesClientError::Process(TsTypesProcessError::SetupMissing(reason)) => (
            category_diagnostic(TsTypesDiagnosticCategory::SetupMissing, reason.clone()),
            ProviderFailureStage::Setup,
            ProviderFailureReason::SetupMissing,
        ),
        TsTypesClientError::Process(TsTypesProcessError::VersionUnsupported(reason)) => (
            category_diagnostic(
                TsTypesDiagnosticCategory::UnsupportedVersion,
                reason.clone(),
            ),
            ProviderFailureStage::Setup,
            ProviderFailureReason::SetupMissing,
        ),
        TsTypesClientError::Process(TsTypesProcessError::Timeout(reason)) => (
            category_diagnostic(TsTypesDiagnosticCategory::Timeout, reason.clone()),
            ProviderFailureStage::Execution,
            ProviderFailureReason::ExecutionFailed,
        ),
        TsTypesClientError::Process(TsTypesProcessError::CommandUnavailable(reason)) => (
            category_diagnostic(TsTypesDiagnosticCategory::SetupMissing, reason.clone()),
            ProviderFailureStage::Setup,
            ProviderFailureReason::SetupMissing,
        ),
        TsTypesClientError::Process(TsTypesProcessError::CommandFailed(reason)) => (
            category_diagnostic(TsTypesDiagnosticCategory::ProjectError, reason.clone()),
            ProviderFailureStage::Execution,
            ProviderFailureReason::ExecutionFailed,
        ),
        TsTypesClientError::Protocol(error) => (
            category_diagnostic(TsTypesDiagnosticCategory::ProjectError, error.to_string()),
            ProviderFailureStage::Execution,
            ProviderFailureReason::ExecutionFailed,
        ),
    }
}

fn category_diagnostic(category: TsTypesDiagnosticCategory, message: String) -> Diagnostic {
    Diagnostic::warning(
        "polint/ts-types",
        "<workspace>",
        TextRange::point(1, 1),
        format!("{}: {message}", category.as_str()),
    )
}

fn provider_error_diagnostic(message: String) -> Diagnostic {
    Diagnostic::error(
        "polint/internal",
        "<workspace>",
        TextRange::point(1, 1),
        format!("TS types provider failed: {message}"),
    )
}

fn replace_ts_types_facts(
    db: &mut dyn FactDatabase,
    output: TsTypesFactsOutput,
) -> Result<TsTypesStoreReport, crate::ts::error::AnalysisError> {
    let interner = db.stable_key_interner();
    let store = TsTypesStore::from_output(output, &interner)?;
    let report = store.report();
    let slot = db
        .store_mut(TS_TYPES_STORE_FAMILY)
        .and_then(|entry| entry.as_any_mut().downcast_mut::<TsTypesStore>())
        .expect("TsTypesStore installed on host FactDatabase");
    *slot = store;
    Ok(report)
}

fn ts_types_facts_output(db: &dyn FactDatabase) -> TsTypesFactsOutput {
    db.store(TS_TYPES_STORE_FAMILY)
        .and_then(|entry| entry.as_any().downcast_ref::<TsTypesStore>())
        .expect("TsTypesStore installed on host FactDatabase")
        .output()
        .clone()
}
