use std::collections::BTreeMap;
use std::path::Path;

use crate::analysis_api::{
    CacheStats, Digest, DigestKind, FactDatabase, ProviderExecution, ProviderFailureReason,
    ProviderFailureStage, ProviderManifest,
};
use crate::internal_core::{Diagnostic, DiagnosticRange as TextRange, Span};
use toml::Value;

use crate::go::lifecycle::{GoAnalysisConfig, go_files};
use crate::go::semantic::cache_key::{
    GoSemanticCacheInputs, go_semantic_input_digest, go_semantic_provider_parameter_digest,
};
use crate::go::semantic::client::{GoSemanticClient, GoSemanticClientError, GoSemanticClientRun};
use crate::go::semantic::diagnostics::{
    GoSemanticDiagnosticCategory, category_for_package_error, category_for_timeout,
    category_for_unsupported_go_version,
};
use crate::go::semantic::lower::lower_go_semantic;
use crate::go::semantic::prefetch::GoSemanticPrefetch;
use crate::go::semantic::process::GoSemanticProcessError;
use crate::go::semantic::store::{
    GO_SEMANTIC_STORE_FAMILY, GoSemanticFactsOutput, GoSemanticStore, StructuralDuplicateReport,
};

#[derive(Debug, Clone, Default)]
pub struct GoSemanticProviderRunOutput {
    pub diagnostics: Vec<Diagnostic>,
    pub cache_stats: CacheStats,
    pub output_digest: Option<Digest>,
    pub execution: ProviderExecution,
    /// Sidecar stage timings and workload sizes, keyed for the run report.
    /// Empty when the sidecar did not run or reported no phases.
    pub counts: BTreeMap<String, u64>,
}

/// Folds the sidecar's phase frames into report counters and logs each stage.
///
/// Timings are the answer to "why was this run slow", so they are logged on the
/// same target as every other kernel stage and carried into the run report
/// rather than staying stderr-only.
fn phase_counts(output: &crate::go::semantic::protocol::GoSemanticOutput) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for phase in &output.phases {
        tracing::debug!(
            target: "polint::kernel::stage",
            provider = "polint.go.semantic",
            phase = phase.phase.as_str(),
            elapsed_ms = phase.elapsed_ms,
            packages = phase.packages,
            compiled_go_files = phase.compiled_go_files,
            deps_with_types = phase.deps_with_types,
            rows_emitted = phase.rows_emitted,
            peak_heap_bytes = phase.peak_heap_bytes,
            "go semantic phase"
        );
        counts.insert(
            format!("go_semantic.phase.{}.elapsed_ms", phase.phase),
            phase.elapsed_ms,
        );
    }
    let totals = &output.totals;
    if totals.elapsed_ms > 0 || totals.packages > 0 {
        tracing::info!(
            target: "polint::kernel::stage",
            provider = "polint.go.semantic",
            elapsed_ms = totals.elapsed_ms,
            packages = totals.packages,
            compiled_go_files = totals.compiled_go_files,
            deps_with_types = totals.deps_with_types,
            peak_heap_bytes = totals.peak_heap_bytes,
            "go semantic sidecar totals"
        );
        counts.insert("go_semantic.elapsed_ms".to_string(), totals.elapsed_ms);
        counts.insert("go_semantic.packages".to_string(), totals.packages);
        counts.insert(
            "go_semantic.compiled_go_files".to_string(),
            totals.compiled_go_files,
        );
        counts.insert(
            "go_semantic.deps_with_types".to_string(),
            totals.deps_with_types,
        );
        counts.insert(
            "go_semantic.peak_heap_bytes".to_string(),
            totals.peak_heap_bytes,
        );
    }
    counts
}

/// The two ways this run can reach a sidecar result without starting one here:
/// the file a previous identical run left behind, and a run this run already
/// started. Both are optional and neither can change what the sidecar answers.
#[derive(Default)]
pub struct GoSemanticSidecarAccess<'a> {
    /// Where [`GoSemanticClient::run_cached`] reads and writes stored output.
    pub cache_dir: Option<&'a Path>,
    /// A run started before this provider was reached.
    pub prefetch: Option<GoSemanticPrefetch>,
}

pub fn derive_go_semantic_with_cache_stats(
    db: &mut dyn FactDatabase,
    root: &Path,
    go_settings: &BTreeMap<String, Value>,
    config_digest: &str,
    manifest: &ProviderManifest,
    go_syntax_output_digest: Digest,
    sidecar: GoSemanticSidecarAccess<'_>,
) -> GoSemanticProviderRunOutput {
    let GoSemanticSidecarAccess {
        cache_dir,
        prefetch,
    } = sidecar;
    let upstream_str = go_syntax_output_digest.to_string();
    let cache_dir = cache_dir.map(Path::to_path_buf);
    let root_owned = root.to_path_buf();
    derive_go_semantic_with_runner(
        db,
        root,
        go_settings,
        config_digest,
        manifest,
        go_syntax_output_digest,
        move |config| {
            // A prefetch is only ever an already-started copy of the run below,
            // and only for the config it was started with; anything else falls
            // through to the run this provider would always have made.
            if let Some(prefetch) = prefetch
                && let Some(prefetched) = prefetch.take_for(config)
            {
                return prefetched;
            }
            match cache_dir.as_deref() {
                Some(dir) => {
                    GoSemanticClient::new(root_owned, config).run_cached(config, dir, &upstream_str)
                }
                None => GoSemanticClient::new(root_owned, config).run(config),
            }
        },
    )
}

fn derive_go_semantic_with_runner(
    db: &mut dyn FactDatabase,
    root: &Path,
    go_settings: &BTreeMap<String, Value>,
    config_digest: &str,
    manifest: &ProviderManifest,
    go_syntax_output_digest: Digest,
    runner: impl FnOnce(&GoAnalysisConfig) -> Result<GoSemanticClientRun, GoSemanticClientError>,
) -> GoSemanticProviderRunOutput {
    debug_assert_eq!(manifest.id, "polint.go.semantic");
    let mut cache_stats = CacheStats::default();
    cache_stats.record_recompute();

    let files = go_files(db);
    if files.is_empty() {
        return store_output(
            db,
            config_digest,
            manifest,
            StoreOutputParts {
                go_syntax_output_digest,
                output: GoSemanticFactsOutput::default(),
                lifecycle: default_lifecycle(),
                digest_inputs: EmptyDigestInputs::default().into(),
                cache_stats,
                diagnostics: Vec::new(),
                execution: ProviderExecution::Succeeded,
            },
        );
    }

    let config = match GoAnalysisConfig::from_settings_files(root, go_settings, &files) {
        Ok(config) => config,
        Err(error) => {
            return store_output(
                db,
                config_digest,
                manifest,
                StoreOutputParts {
                    go_syntax_output_digest,
                    output: GoSemanticFactsOutput::default(),
                    lifecycle: default_lifecycle(),
                    digest_inputs: EmptyDigestInputs::from_lifecycle_error(error.reason()).into(),
                    cache_stats,
                    diagnostics: vec![setup_missing_diagnostic(error.reason())],
                    execution: ProviderExecution::Failed {
                        stage: ProviderFailureStage::Setup,
                        reason: ProviderFailureReason::SetupMissing,
                    },
                },
            );
        }
    };

    if !config.files_without_module_root.is_empty() {
        let diagnostics = if go_module_roots_configured(go_settings) {
            vec![setup_missing_diagnostic(
                "some Go files are not under a configured go.mod module root.",
            )]
        } else {
            Vec::new()
        };
        return store_output(
            db,
            config_digest,
            manifest,
            StoreOutputParts {
                go_syntax_output_digest,
                output: GoSemanticFactsOutput::default(),
                lifecycle: config,
                digest_inputs: EmptyDigestInputs::from_lifecycle_error(
                    "files outside module roots",
                )
                .into(),
                cache_stats,
                diagnostics,
                execution: if go_module_roots_configured(go_settings) {
                    ProviderExecution::Failed {
                        stage: ProviderFailureStage::Setup,
                        reason: ProviderFailureReason::SetupMissing,
                    }
                } else {
                    ProviderExecution::Succeeded
                },
            },
        );
    }

    let missing_roots = config.missing_module_roots(root);
    if !missing_roots.is_empty() {
        return store_output(
            db,
            config_digest,
            manifest,
            StoreOutputParts {
                go_syntax_output_digest,
                output: GoSemanticFactsOutput::default(),
                lifecycle: config,
                digest_inputs: EmptyDigestInputs::from_lifecycle_error("missing module roots")
                    .into(),
                cache_stats,
                diagnostics: vec![setup_missing_diagnostic(&format!(
                    "configured Go module roots are missing go.mod: {}.",
                    missing_roots.join(", ")
                ))],
                execution: ProviderExecution::Failed {
                    stage: ProviderFailureStage::Setup,
                    reason: ProviderFailureReason::SetupMissing,
                },
            },
        );
    }

    let run = match runner(&config) {
        Ok(run) => run,
        Err(error) => {
            return store_output(
                db,
                config_digest,
                manifest,
                StoreOutputParts {
                    go_syntax_output_digest,
                    output: GoSemanticFactsOutput::default(),
                    lifecycle: config,
                    digest_inputs: EmptyDigestInputs::from_client_error(&error).into(),
                    cache_stats,
                    diagnostics: vec![client_error_diagnostic(error)],
                    execution: ProviderExecution::Failed {
                        stage: ProviderFailureStage::Execution,
                        reason: ProviderFailureReason::ExecutionFailed,
                    },
                },
            );
        }
    };

    let counts = phase_counts(&run.output);
    let lowered = match lower_go_semantic(db, &run.output) {
        Ok(output) => output,
        Err(error) => {
            return GoSemanticProviderRunOutput {
                diagnostics: vec![provider_error_diagnostic(error.to_string())],
                cache_stats,
                output_digest: None,
                execution: ProviderExecution::Failed {
                    stage: ProviderFailureStage::Execution,
                    reason: ProviderFailureReason::ExecutionFailed,
                },
                counts,
            };
        }
    };
    let diagnostics = package_error_diagnostics(&lowered);
    let digest_inputs = DigestInputs {
        sidecar_digest: run.frontend_digest,
        go_version: run.output.go_version,
        x_tools_version: run.output.x_tools_version,
    };
    let mut stored = store_output(
        db,
        config_digest,
        manifest,
        StoreOutputParts {
            go_syntax_output_digest,
            output: lowered,
            lifecycle: config,
            digest_inputs,
            cache_stats,
            diagnostics,
            execution: ProviderExecution::Succeeded,
        },
    );
    stored.counts = counts;
    stored
}

#[derive(Debug, Clone)]
struct DigestInputs {
    sidecar_digest: String,
    go_version: String,
    x_tools_version: String,
}

#[derive(Debug, Clone, Default)]
struct EmptyDigestInputs {
    reason: String,
}

impl EmptyDigestInputs {
    fn from_lifecycle_error(reason: &str) -> Self {
        Self {
            reason: format!("lifecycle:{reason}"),
        }
    }

    fn from_client_error(error: &GoSemanticClientError) -> Self {
        Self {
            reason: format!("client:{error}"),
        }
    }
}

impl From<EmptyDigestInputs> for DigestInputs {
    fn from(inputs: EmptyDigestInputs) -> Self {
        Self {
            sidecar_digest: format!("not-run:{}", inputs.reason),
            go_version: "not-run".to_string(),
            x_tools_version: "not-run".to_string(),
        }
    }
}

struct StoreOutputParts {
    go_syntax_output_digest: Digest,
    output: GoSemanticFactsOutput,
    lifecycle: GoAnalysisConfig,
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
) -> GoSemanticProviderRunOutput {
    let interner = db.stable_key_interner();
    let output = parts.output.normalized(&interner);
    let execution = parts.execution;
    match replace_go_semantic_facts(db, output) {
        // The digest is issued only for an explicitly successful execution. Setup
        // and execution failures install an empty, setup-aware store but do not
        // certify it as a usable provider output.
        Ok(report) if execution == ProviderExecution::Succeeded => {
            let stored_output = go_semantic_facts_output(db);
            let output_digest = go_semantic_output_digest(
                manifest,
                config_digest,
                &parts.go_syntax_output_digest,
                &parts.digest_inputs,
                &parts.lifecycle,
                &stored_output,
                &interner,
            );
            let mut diagnostics = parts.diagnostics;
            if report.dropped_harvest_rows > 0 {
                diagnostics.push(dropped_harvest_rows_diagnostic(report.dropped_harvest_rows));
            }
            diagnostics.extend(structural_duplicate_diagnostics(
                &report.structural_duplicates,
            ));
            GoSemanticProviderRunOutput {
                counts: BTreeMap::new(),
                diagnostics,
                cache_stats: parts.cache_stats,
                output_digest: Some(output_digest),
                execution,
            }
        }
        Ok(_report) => GoSemanticProviderRunOutput {
            counts: BTreeMap::new(),
            diagnostics: parts.diagnostics,
            cache_stats: parts.cache_stats,
            output_digest: None,
            execution,
        },
        Err(error) => GoSemanticProviderRunOutput {
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

fn go_semantic_output_digest(
    manifest: &ProviderManifest,
    config_digest: &str,
    go_syntax_output_digest: &Digest,
    digest_inputs: &DigestInputs,
    lifecycle: &GoAnalysisConfig,
    output: &GoSemanticFactsOutput,
    interner: &crate::internal_core::StableKeyInterner,
) -> Digest {
    let cache_inputs = GoSemanticCacheInputs {
        sidecar_digest: digest_inputs.sidecar_digest.clone(),
        go_version: digest_inputs.go_version.clone(),
        x_tools_version: digest_inputs.x_tools_version.clone(),
        upstream_digest: go_syntax_output_digest.to_string(),
        lifecycle: lifecycle.clone(),
    };
    let input_digest = go_semantic_input_digest(&cache_inputs);
    let mut parts = vec![
        format!("provider_id={}", manifest.id),
        format!("provider_version={}", manifest.provider_version()),
        format!("schema={}", manifest.primary_schema_label()),
        format!("parameters={}", go_semantic_provider_parameter_digest()),
        format!("config={config_digest}"),
        format!("go_syntax={go_syntax_output_digest}"),
        format!("input_digest={input_digest}"),
    ];
    parts.extend(output.packages.iter().map(|package| {
        format!(
            "package={} id={} path={} name={} module={} files={}",
            interner.resolve(package.stable_key),
            package.package_id,
            package.package_path,
            package.package_name,
            package.module_path,
            package.files.join(",")
        )
    }));
    parts.extend(output.functions.iter().map(|function| {
        format!(
            "function={} package={} qualified={} kind={:?} file={} span={}",
            interner.resolve(function.stable_key),
            function.package_path,
            function.qualified,
            function.kind,
            function.relative_file.as_deref().unwrap_or(""),
            option_span_part(function.span.as_ref())
        )
    }));
    parts.extend(output.callsites.iter().map(|callsite| {
        format!(
            "callsite={} package={} caller={} static={} status={:?} file={} span={}",
            interner.resolve(callsite.stable_key),
            callsite.package_path,
            callsite.caller,
            callsite.static_callee.as_deref().unwrap_or(""),
            callsite.status,
            callsite.relative_file.as_deref().unwrap_or(""),
            option_span_part(callsite.span.as_ref())
        )
    }));
    parts.extend(output.method_sets.iter().map(|method_set| {
        format!(
            "method_set={} package={} type={} methods={}",
            interner.resolve(method_set.stable_key),
            method_set.package_path,
            method_set.type_name,
            method_set.methods.join(",")
        )
    }));
    // FIX 4: the three RTA-signal harvest families (`instantiated_types`, `address_taken`,
    // `dynamic_dispatch`) change the RTA-resolved derived edges but were NOT folded here, so
    // a Go edit touching ONLY them changed neither this digest nor the downstream solver
    // digest — a stale-edge false-cache-hit risk once a persistent cache lands. Fold each
    // row's content (mirroring `method_sets`); the output is `parts.sort()`-ed below, so
    // insertion order does not matter. The instantiated_type FILTER is the RTA discriminant,
    // so its membership must invalidate; the address-taken set drives func-value resolution;
    // the dynamic-dispatch discriminant decides which callees a callsite resolves to.
    parts.extend(output.instantiated_types.iter().map(|instantiated_type| {
        format!(
            "instantiated_type={} package={} type={}",
            interner.resolve(instantiated_type.stable_key),
            instantiated_type.package_path,
            instantiated_type.type_name
        )
    }));
    parts.extend(output.address_taken.iter().map(|address_taken| {
        format!(
            "address_taken={} package={} function={}",
            interner.resolve(address_taken.stable_key),
            address_taken.package_path,
            address_taken.function
        )
    }));
    parts.extend(output.dynamic_dispatch.iter().map(|dynamic_dispatch| {
        format!(
            "dynamic_dispatch={} package={} caller={} callsite={} interface={} method={} signature={}",
            interner.resolve(dynamic_dispatch.stable_key),
            dynamic_dispatch.package_path,
            dynamic_dispatch.caller,
            interner.resolve(dynamic_dispatch.callsite_stable_key),
            dynamic_dispatch.interface_type.as_deref().unwrap_or(""),
            dynamic_dispatch.method.as_deref().unwrap_or(""),
            dynamic_dispatch.signature.as_deref().unwrap_or("")
        )
    }));
    parts.extend(output.rta_edges.iter().map(|edge| {
        format!(
            "rta_edge={} package={} caller={} callee={} kind={}",
            interner.resolve(edge.stable_key),
            edge.package_path,
            edge.caller,
            edge.callee,
            edge.edge_kind
        )
    }));
    parts.extend(output.package_errors.iter().map(|package_error| {
        format!(
            "package_error={} package={} message={}",
            interner.resolve(package_error.stable_key),
            package_error.package_path,
            package_error.message
        )
    }));
    if output.packages.is_empty()
        && output.functions.is_empty()
        && output.callsites.is_empty()
        && output.method_sets.is_empty()
        && output.instantiated_types.is_empty()
        && output.address_taken.is_empty()
        && output.dynamic_dispatch.is_empty()
        && output.rta_edges.is_empty()
        && output.package_errors.is_empty()
    {
        parts.push("go_semantic_output=empty".to_string());
    }
    parts.sort();
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Digest::from_parts(DigestKind::ProviderOutput, "go_semantic_output", &refs)
}

fn option_span_part(span: Option<&Span>) -> String {
    span.map(span_part).unwrap_or_else(|| "none".to_string())
}

fn go_module_roots_configured(settings: &BTreeMap<String, Value>) -> bool {
    settings.contains_key("module_roots") || settings.contains_key("module_root")
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

fn package_error_diagnostics(output: &GoSemanticFactsOutput) -> Vec<Diagnostic> {
    output
        .package_errors
        .iter()
        .map(|error| {
            category_diagnostic(
                category_for_package_error(),
                format!(
                    "package {} failed to load: {}",
                    error.package_path, error.message
                ),
            )
        })
        .collect()
}

fn setup_missing_diagnostic(reason: &str) -> Diagnostic {
    category_diagnostic(category_for_package_error(), reason.to_string())
}

/// Observable signal that malformed RTA-signal harvest rows were dropped (FIX 3). Routed
/// through the same diagnostic channel the provider uses for setup/diagnostic messages so
/// a systematic frontend regression surfaces loudly instead of being swallowed into
/// repo-wide under-resolution. NOT fatal (the row-resilience contract, FINDING B) — just
/// visible.
fn dropped_harvest_rows_diagnostic(dropped: usize) -> Diagnostic {
    category_diagnostic(
        category_for_package_error(),
        format!(
            "{dropped} Go RTA-signal rows dropped (invalid identity/stable_key); \
             interface/func-value dispatch may under-resolve."
        ),
    )
}

/// Observable signal that duplicate STRUCTURAL rows (packages/functions/method_sets) were
/// collapsed keep-first (FIX-08). A SINGLE duplicate structural stable key used to make
/// `validate_unique` reject the whole output, leaving the DB with ZERO Go facts and driving
/// RTA to zero edges repo-wide (recurring 3×); the store now collapses the duplicate and
/// surfaces this counted diagnostic so the emitter regression is loud instead of catastrophic.
/// A "conflicting" duplicate (same stable_key, DIFFERING rows) can only come from a
/// stable-key-recipe bug, so its message is ESCALATED. Empty (no diagnostics) on a clean run.
fn structural_duplicate_diagnostics(report: &StructuralDuplicateReport) -> Vec<Diagnostic> {
    if report.total() == 0 {
        return Vec::new();
    }
    let families = [
        ("package", report.packages),
        ("function", report.functions),
        ("method_set", report.method_sets),
    ];
    families
        .into_iter()
        .filter(|(_, dropped)| *dropped > 0)
        .map(|(family, dropped)| {
            let message = if report.conflicting {
                format!(
                    "conflicting duplicate Go {family} stable key — {dropped} row(s) collapsed \
                     keep-first; possible identity-recipe bug (rows differ for one stable key)."
                )
            } else {
                format!(
                    "{dropped} duplicate Go {family} stable key row(s) collapsed keep-first \
                     (byte-identical double-emit); facts preserved."
                )
            };
            category_diagnostic(category_for_package_error(), message)
        })
        .collect()
}

fn client_error_diagnostic(error: GoSemanticClientError) -> Diagnostic {
    let category = match &error {
        GoSemanticClientError::Process(GoSemanticProcessError::Timeout(_)) => {
            category_for_timeout()
        }
        GoSemanticClientError::Process(GoSemanticProcessError::VersionUnsupported(_)) => {
            category_for_unsupported_go_version()
        }
        GoSemanticClientError::Process(_) | GoSemanticClientError::Protocol(_) => {
            category_for_package_error()
        }
    };
    category_diagnostic(category, error.to_string())
}

fn category_diagnostic(category: GoSemanticDiagnosticCategory, message: String) -> Diagnostic {
    Diagnostic::warning(
        "polint/go-semantic",
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
        format!("Go semantic provider failed: {message}"),
    )
}

fn default_lifecycle() -> GoAnalysisConfig {
    GoAnalysisConfig {
        module_roots: vec![".".to_string()],
        package_patterns: vec!["./...".to_string()],
        build_tags: Vec::new(),
        include_tests: true,
        offline: false,
        semantic_timeout_ms: None,
        files_without_module_root: Vec::new(),
    }
}

fn replace_go_semantic_facts(
    db: &mut dyn FactDatabase,
    output: GoSemanticFactsOutput,
) -> Result<crate::go::semantic::store::GoSemanticStoreReport, crate::go::error::AnalysisError> {
    let interner = db.stable_key_interner();
    let store = GoSemanticStore::from_output(output, &interner)?;
    let report = store.report();
    let slot = db
        .store_mut(GO_SEMANTIC_STORE_FAMILY)
        .and_then(|entry| entry.as_any_mut().downcast_mut::<GoSemanticStore>())
        .expect("GoSemanticStore installed on host FactDatabase");
    *slot = store;
    Ok(report)
}

fn go_semantic_facts_output(db: &dyn FactDatabase) -> GoSemanticFactsOutput {
    db.store(GO_SEMANTIC_STORE_FAMILY)
        .and_then(|entry| entry.as_any().downcast_ref::<GoSemanticStore>())
        .expect("GoSemanticStore installed on host FactDatabase")
        .output()
        .clone()
}
