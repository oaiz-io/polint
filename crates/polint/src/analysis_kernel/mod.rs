use crate::analysis_neutral::metrics::METRIC_CAPABILITIES;
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::LoadedConfig;
use crate::core::{AnalysisDb, CapabilitySupportView};
use crate::diagnostics::{Diagnostic, TextRange};
use std::collections::{BTreeMap, BTreeSet};

#[rustfmt::skip]
#[cfg(test)] mod debug;
pub(crate) mod call_cache;
mod completeness;
pub(crate) use completeness::pipeline_failure_unknowns;
#[cfg(test)]
mod dispatch_tests;
pub(crate) mod go_syntax_projection;
pub(crate) mod host;
pub(crate) mod incremental;
mod metadata;
pub(crate) mod metrics_projection;
mod outcome;
mod provider;
pub(crate) mod resource;
mod store;
#[cfg(all(test, feature = "lang-typescript"))]
mod ts_types_tests;
pub(crate) mod validation;

pub(crate) use metadata::{
    FactConfidence, FactFamily, FactMeta, FactMetaStore, FactPrecision, FactRef, MissingFactMeta,
    ValidationStatus, resolution_metadata, resolution_status_metadata, stable_key_text_from_parts,
    symbol_metadata, write_stable_key_text,
};
#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
pub(crate) use outcome::hard_dependencies;
pub(crate) use outcome::provider_outcome_rows;
pub(crate) use outcome::{
    ProviderFailureReason, ProviderFailureStage, ProviderOutcome, ProviderOutcomeStatus,
    ProviderOutcomeTracker, ProviderOutputIdentity, ValidationDowngrades,
};
#[cfg(test)]
pub(crate) use provider::ProviderKind;
#[cfg(test)]
pub(crate) use provider::{CachePolicy, SchemaVersion};
pub(crate) use provider::{
    PrecisionCeiling, ProviderCtx, ProviderManifest, ProviderRunResult,
    providers_enabled_by_boolean_gates, providers_enabled_by_capability_closure,
    run_named_provider, scheduled_order, scheduled_order_for,
};
pub(crate) use store::StoreStatus;

fn requested_trigger_capabilities(plan: &AnalysisPlan) -> std::collections::BTreeSet<&str> {
    const CANDIDATES: &[&str] = &[
        "resolved_imports",
        "module_graph",
        "symbols",
        "references",
        "calls",
        "control_flow",
        "dataflow",
        "call_graph",
        "go_types",
        "routes",
        "file_metrics",
        "function_metrics",
        "complexity_metrics",
    ];
    CANDIDATES
        .iter()
        .copied()
        .filter(|capability| plan.requests_capability(capability))
        .collect()
}

/// Rule id for scope-versus-analysis notes.
pub(crate) const SCOPE_RULE_ID: &str = "polint/scope";

/// Capabilities whose analysis crosses file boundaries, so requesting one loads
/// every discovered file regardless of any rule's `files` list.
const CROSS_FILE_CAPABILITIES: [&str; 10] = [
    "calls",
    "call_graph",
    "control_flow",
    "dataflow",
    "go_types",
    "module_graph",
    "references",
    "resolved_imports",
    "routes",
    "symbols",
];

/// Notes rules whose `files` list is strictly narrower than the analysed set.
///
/// A cross-file capability makes the kernel load every discovered file, so
/// `files` narrows *reporting*, not analysis. That is invisible in the output
/// and is regularly mistaken for a way to bound a slow scan, so say it once per
/// affected rule instead of leaving it to folklore.
fn rule_scope_narrower_than_analysis(plan: &AnalysisPlan, db: &AnalysisDb) -> Vec<Diagnostic> {
    let analyzed = db.files().len();
    if analyzed == 0 {
        return Vec::new();
    }
    let paths = db
        .files()
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect::<Vec<_>>();
    plan.rules()
        .iter()
        .filter_map(|rule| {
            if rule.files.is_empty() {
                return None;
            }
            let cross_file = rule
                .requested_capabilities
                .iter()
                .filter(|capability| CROSS_FILE_CAPABILITIES.contains(&capability.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            if cross_file.is_empty() {
                return None;
            }
            let globs = crate::config::build_glob_set(&rule.files).ok()?;
            let in_scope = paths.iter().filter(|path| globs.is_match(path)).count();
            if in_scope >= analyzed {
                return None;
            }
            Some(
                Diagnostic::info(
                    SCOPE_RULE_ID.to_string(),
                    "<workspace>",
                    crate::diagnostics::TextRange::point(1, 1),
                    format!(
                        "Rule `{}` reports on {in_scope} of {analyzed} analyzed files, but `{}` \
                         analyzes every discovered file.",
                        rule.id,
                        cross_file.join("`, `")
                    ),
                )
                .with_evidence("rule", rule.id.clone())
                .with_evidence("files_in_scope", in_scope.to_string())
                .with_evidence("analyzed_files", analyzed.to_string())
                .with_evidence("cross_file_capabilities", cross_file.join(","))
                .with_help(
                    "`files` narrows which findings are reported, not which files are analyzed. \
                     Bound the analysis with `[languages.go] package_patterns` and \
                     `include_tests`, or with `[workspace] include` / `exclude`.",
                ),
            )
        })
        .collect()
}

fn log_loaded_source_files(db: &AnalysisDb) {
    if !tracing::enabled!(target: "polint::kernel", tracing::Level::INFO) {
        return;
    }
    let files = db.files();
    let total_bytes: usize = files.iter().map(|f| f.source.len()).sum();
    let (mut go, mut ts, mut go_bytes, mut ts_bytes) = (0usize, 0usize, 0usize, 0usize);
    for f in files.iter() {
        match f.language {
            crate::core::Language::Go => {
                go += 1;
                go_bytes += f.source.len();
            }
            crate::core::Language::TypeScript
            | crate::core::Language::Tsx
            | crate::core::Language::JavaScript
            | crate::core::Language::Jsx => {
                ts += 1;
                ts_bytes += f.source.len();
            }
            _ => {}
        }
    }
    tracing::info!(
        target: "polint::kernel",
        files = files.len(),
        total_mb = total_bytes / 1_048_576,
        go,
        go_mb = go_bytes / 1_048_576,
        ts,
        ts_mb = ts_bytes / 1_048_576,
        "phase: source files loaded"
    );
}

fn skipped_direct_summaries_result(
    db: &AnalysisDb,
    input_snapshot: &incremental::InputSnapshot,
    upstream_digests: &std::collections::BTreeMap<&'static str, incremental::Digest>,
) -> ProviderRunResult {
    // Historical run() still recorded an empty post-SCC digest when the deep
    // refinement stack was gated off.
    let absent = |id: &'static str| {
        upstream_digests.get(id).cloned().unwrap_or_else(|| {
            incremental::Digest::absent(incremental::DigestKind::ProviderOutput, id)
        })
    };
    let go_ts = [absent("polint.go.syntax"), absent("polint.ts.syntax")];
    let final_output = crate::analysis::summaries::store::SummaryOutput {
        summaries: db.summary_facts().to_vec(),
        events: db.summary_events().to_vec(),
    };
    let interner = db.stable_key_interner();
    let output_digest = crate::analysis::summaries::provider::direct_summaries_output_digest(
        AnalysisKernel::provider_manifest("polint.direct_summaries"),
        input_snapshot,
        &absent("polint.semantic_mir"),
        &absent("polint.cfg"),
        &absent("polint.calls"),
        &absent("polint.abstract_domains"),
        &absent("polint.symbol_graph"),
        &absent("polint.module_topology"),
        &go_ts,
        &interner,
        &crate::analysis::summaries::provider::callable_stable_key_map(db),
        &final_output,
    );
    ProviderRunResult {
        counts: Default::default(),
        diagnostics: Vec::new(),
        cache_stats: incremental::CacheStats::default(),
        output_digest: Some(output_digest),
        execution: Default::default(),
    }
}

/// `syntax_level_rule_check` marks a rule check that requests nothing past
/// syntax and the metrics derived from it: its only readers are the rules and
/// the report.
fn run_scheduled_providers<'a>(
    db: &'a mut AnalysisDb,
    input: &KernelInput<'a>,
    input_snapshot: &'a incremental::InputSnapshot,
    enabled_providers: &std::collections::BTreeSet<&'static str>,
    syntax_level_rule_check: bool,
    diagnostics: &mut Vec<Diagnostic>,
    provider_outputs: &mut Vec<incremental::ProviderOutputMeta>,
) -> anyhow::Result<(
    CapabilitySupportView,
    Option<crate::analysis::summaries::provider::SccClosureProviderOutput>,
    ProviderOutcomeTracker,
    Vec<incremental::ProviderTelemetry>,
)> {
    let mut upstream_digests = std::collections::BTreeMap::new();
    let capability_support = input.plan.support_view().clone();
    let host_scope = crate::analysis_kernel::host::ProviderHostSessionScope::install(
        crate::analysis_kernel::host::ProviderHostSession {
            cache: input.cache.clone(),
            loaded: input.loaded.clone(),
            input_snapshot: input_snapshot.clone(),
            plan: input.plan.clone(),
            capability_support,
            scc_closure: None,
            go_semantic_prefetch: None,
        },
    );
    let mut host_services = crate::analysis_kernel::host::FacadeHostServices {
        plan_digest: input.plan.digest().to_string(),
        analysis_cache: Some(crate::cache::CacheAnalysisCache::new(input.cache.clone())),
    };
    let mut host_attachment = crate::analysis_kernel::host::FacadeHostAttachment::default();
    let mut tracker = ProviderOutcomeTracker::from_manifests(
        AnalysisKernel::provider_manifests(),
        enabled_providers,
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let mut provider_telemetry = Vec::with_capacity(AnalysisKernel::provider_manifests().len());
    let mut envelope = resource::ResourceEnvelope::from_env();
    let mut call_cache = CallCacheState::Inactive;
    // Emit a provider_outputs row for every manifest entry (historical identity),
    // but only execute providers selected by capability closure.
    for provider_id in scheduled_order() {
        if provider_id == call_cache::CACHED_PROVIDERS[0] {
            call_cache = call_cache.restore_or_compute(db);
        }
        let selected = enabled_providers.contains(provider_id);
        let blockers = if selected {
            tracker
                .can_run(provider_id)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?
        } else {
            Vec::new()
        };
        let ready = selected && blockers.is_empty() && !envelope.exhausted();
        if selected && !ready && envelope.exhausted() {
            // The envelope is a *safety net*: the run stops adding facts rather
            // than being killed by the OOM reaper with nothing to report. The
            // capabilities this provider backs degrade through the normal
            // outcome path, and the run emits one `polint/resource-budget`
            // diagnostic (below) that `polint unknowns` surfaces.
            tracker
                .record_failure(
                    provider_id,
                    ProviderOutcomeStatus::BudgetExceeded,
                    ProviderFailureStage::Execution,
                    ProviderFailureReason::MemoryCeiling,
                )
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        } else if selected && !ready {
            tracker
                .record_dependency_blocked(provider_id, blockers)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        let stage_started = std::time::Instant::now();
        let deferred_before = db.deferred_syntax_metadata_len();
        let stage_rss_before = crate::measure::current_rss_bytes();
        let restored = match &call_cache {
            CallCacheState::Restored(providers)
                if ready && call_cache::restores(provider_id, input.plan) =>
            {
                providers.get(provider_id).cloned()
            }
            _ => None,
        };
        let result = if let Some(restored) = restored {
            // Restored from the call-resolution cache: the provider's facts are
            // in the database already and it reports the digest and diagnostics
            // the computing run recorded. It did not run, so it reports no
            // counters.
            let mut cache_stats = incremental::CacheStats::default();
            cache_stats.record_hit();
            ProviderRunResult {
                diagnostics: restored.diagnostics,
                cache_stats,
                output_digest: Some(restored.output_digest),
                execution: Default::default(),
                counts: std::collections::BTreeMap::new(),
            }
        } else if ready {
            let mut ctx = ProviderCtx {
                facts: &mut *db,
                host: &mut host_services,
                config_digest: input.config_digest,
                rule_digest: input.rule_digest,
                parallel: input.parallel,
                upstream_digests: &upstream_digests,
                host_attachment: &mut host_attachment,
            };
            run_named_provider(provider_id, &mut ctx)
        } else if provider_id == "polint.direct_summaries" {
            skipped_direct_summaries_result(db, input_snapshot, &upstream_digests)
        } else {
            ProviderRunResult {
                diagnostics: Vec::new(),
                cache_stats: incremental::CacheStats::default(),
                output_digest: None,
                execution: Default::default(),
                counts: std::collections::BTreeMap::new(),
            }
        };
        call_cache.observe(db, provider_id, ready, &result);
        let stage_elapsed_ms = ready.then(|| stage_started.elapsed().as_millis() as u64);
        // A provider owns the truth of whether its output is usable. Never let
        // a failed result's digest enter the dependency map or the identity
        // report; downstream blockers must observe the failure immediately.
        let execution = result.execution;
        let output_digest =
            if matches!(execution, crate::analysis_api::ProviderExecution::Succeeded) {
                result.output_digest
            } else {
                None
            };
        // A syntax provider without a native identity falls back to a digest
        // over the metadata of the facts it produced, so any of those it
        // deferred must be recorded before the fallback reads them. A provider
        // that restored nothing — no sources in its language — deferred nothing,
        // and the other provider's deferred rows are not its to read.
        let deferred_by_provider = db.deferred_syntax_metadata_len() > deferred_before;
        if output_digest.is_none() && is_syntax_provider(provider_id) && deferred_by_provider {
            db.record_deferred_syntax_metadata();
        }
        if ready {
            envelope.observe(provider_id);
            let stage_rss_after = crate::measure::current_rss_bytes();
            let interner = db.stable_key_interner();
            tracing::info!(
                target: "polint::kernel::stage",
                provider = provider_id,
                elapsed_ms = stage_started.elapsed().as_millis() as u64,
                rss_mb = stage_rss_after / (1024 * 1024),
                rss_delta_mb = stage_rss_after.saturating_sub(stage_rss_before) / (1024 * 1024),
                peak_rss_mb = crate::measure::peak_rss_bytes() / (1024 * 1024),
                facts = db.fact_meta().row_count() + db.deferred_syntax_metadata_len(),
                keys = interner.len(),
                key_mb = interner.text_bytes() / (1024 * 1024),
                digest = output_digest.as_ref().map_or("-", |digest| digest.value.as_str()),
                "stage done"
            );
        }
        if provider_id == "polint.go.syntax" {
            tracing::info!(target: "polint::kernel", "phase: go.syntax done");
        } else if provider_id == "polint.ts.syntax" {
            tracing::info!(target: "polint::kernel", "phase: ts.syntax done");
        }
        diagnostics.extend(result.diagnostics);
        let cache_stats = result.cache_stats;
        if let Some(digest) = output_digest.clone() {
            upstream_digests.insert(provider_id, digest);
        }
        provider_telemetry.push(
            incremental::ProviderTelemetry::new(provider_id, cache_stats.clone())
                .with_stage(stage_elapsed_ms, result.counts),
        );
        provider_outputs.push(AnalysisKernel::provider_output_for_with_optional_digest(
            provider_id,
            db,
            cache_stats,
            output_digest.clone(),
            ready && matches!(execution, crate::analysis_api::ProviderExecution::Succeeded),
        ));
        if ready {
            let manifest = AnalysisKernel::provider_manifest(provider_id);
            if let crate::analysis_api::ProviderExecution::Failed { stage, reason } = execution {
                let (status, stage, reason) = provider_failure_outcome(stage, reason);
                tracker
                    .record_failure(provider_id, status, stage, reason)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            } else {
                let digest = match provider_id {
                    // The canonical projection makes the go.syntax identity
                    // independent of the adapter's encoding, for the providers
                    // that key their caches on it. A syntax-level rule check
                    // runs none of them — the metrics provider keys its layer on
                    // its own canonical inputs — and past them the identity
                    // reaches only the run report, which a rule check neither
                    // prints nor persists, so it keeps the adapter's native
                    // digest.
                    "polint.go.syntax" if !syntax_level_rule_check => {
                        let parser_diagnostics = diagnostics
                            .iter()
                            .filter(|diagnostic| diagnostic.rule_id == "parser/go")
                            .cloned()
                            .collect::<Vec<_>>();
                        crate::analysis_kernel::go_syntax_projection::canonical_go_syntax_digest(
                            db,
                            input.cache,
                            output_digest.as_ref(),
                            &parser_diagnostics,
                        )
                    }
                    _ => output_digest,
                };
                let digest = match digest {
                    Some(digest) => digest,
                    None => {
                        if is_syntax_provider(provider_id) && deferred_by_provider {
                            db.record_deferred_syntax_metadata();
                        }
                        incremental::provider_output_digest_from_manifest(
                            manifest,
                            &provider_output_summary_parts(db, manifest),
                        )
                    }
                };
                let identity =
                    incremental::provider_output_identity_from_manifest(manifest, digest.clone());
                tracker
                    .record_success(provider_id, identity)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                upstream_digests.insert(provider_id, digest);
            }
        }
        if provider_id == "polint.go.syntax"
            && ready
            && matches!(execution, crate::analysis_api::ProviderExecution::Succeeded)
        {
            call_cache = CallCacheState::decide(
                db,
                input,
                input_snapshot,
                enabled_providers,
                &upstream_digests,
            );
            if enabled_providers.contains("polint.go.semantic")
                && (!call_cache.will_restore()
                    || !call_cache::restores("polint.go.semantic", input.plan))
            {
                start_go_semantic_prefetch(db, input, &upstream_digests);
            }
        }
    }

    // A prefetch nobody read still owns a subprocess. Wait for it rather than
    // leaving a Go sidecar behind when the run ends.
    if let Some(prefetch) =
        crate::analysis_kernel::host::with_provider_host_session_mut(|session| {
            session.go_semantic_prefetch.take()
        })
    {
        prefetch.abandon();
    }

    if let Some(trip) = envelope.trip() {
        diagnostics.push(resource::budget_diagnostic(&trip));
    }

    let session = host_scope.take();
    Ok((
        session.capability_support,
        session.scc_closure,
        tracker,
        provider_telemetry,
    ))
}

/// Where a run stands with the call-resolution cache (see [`call_cache`]).
enum CallCacheState {
    /// The run cannot use the cache.
    Inactive,
    /// An entry is stored under the run's key; it is restored when the cached
    /// providers' turn comes.
    Hit {
        directory: std::path::PathBuf,
        key: String,
    },
    /// The cached providers' facts are in the database; their recorded outcomes.
    Restored(BTreeMap<String, call_cache::CachedProvider>),
    /// The cached providers compute; their outcomes so far.
    Computing {
        directory: std::path::PathBuf,
        key: String,
        providers: Vec<call_cache::CachedProvider>,
        clean: bool,
    },
}

impl CallCacheState {
    /// Decides, once the Go syntax is known, whether the run will restore or
    /// compute; see [`call_cache::KEYED_UPSTREAM`] for why that is enough.
    fn decide(
        db: &AnalysisDb,
        input: &KernelInput<'_>,
        input_snapshot: &incremental::InputSnapshot,
        enabled_providers: &std::collections::BTreeSet<&'static str>,
        upstream_digests: &BTreeMap<&'static str, crate::analysis_api::Digest>,
    ) -> Self {
        let Some(directory) = call_cache::cache_dir(input.cache) else {
            return Self::Inactive;
        };
        if !call_cache::eligible(input.plan, db, enabled_providers) {
            return Self::Inactive;
        }
        let identity = go_semantic_run_identity(db, input, upstream_digests);
        let Some(key) = call_cache::entry_key(
            input_snapshot,
            upstream_digests,
            identity.as_deref(),
            &input.loaded.root,
        ) else {
            return Self::Inactive;
        };
        if call_cache::has_entry(&directory, &key) {
            Self::Hit { directory, key }
        } else {
            Self::Computing {
                directory,
                key,
                providers: Vec::new(),
                clean: true,
            }
        }
    }

    fn will_restore(&self) -> bool {
        matches!(self, Self::Hit { .. })
    }

    /// At the first cached provider's turn: restores a stored entry, or falls
    /// back to computing when it cannot be used.
    fn restore_or_compute(self, db: &mut AnalysisDb) -> Self {
        let Self::Hit { directory, key } = self else {
            return self;
        };
        match call_cache::restore(db, &directory, &key) {
            Some(providers) => Self::Restored(
                providers
                    .into_iter()
                    .map(|provider| (provider.id.clone(), provider))
                    .collect(),
            ),
            None => Self::Computing {
                directory,
                key,
                providers: Vec::new(),
                clean: true,
            },
        }
    }

    /// Records a cached provider's outcome while computing, and stores the
    /// entry after the last one when every cached provider succeeded.
    fn observe(
        &mut self,
        db: &AnalysisDb,
        provider_id: &str,
        ready: bool,
        result: &ProviderRunResult,
    ) {
        let Self::Computing {
            directory,
            key,
            providers,
            clean,
        } = self
        else {
            return;
        };
        if !call_cache::CACHED_PROVIDERS.contains(&provider_id) {
            return;
        }
        match (&result.output_digest, &result.execution) {
            (Some(digest), crate::analysis_api::ProviderExecution::Succeeded) if ready => {
                providers.push(call_cache::CachedProvider {
                    id: provider_id.to_string(),
                    output_digest: digest.clone(),
                    diagnostics: result.diagnostics.clone(),
                });
            }
            _ => {
                if *clean {
                    tracing::debug!(
                        target: "polint::kernel::stage",
                        provider = provider_id,
                        "call-resolution cache entry not written: a cached provider did not succeed"
                    );
                }
                *clean = false;
            }
        }
        if Some(&provider_id) == call_cache::CACHED_PROVIDERS.last() && *clean {
            call_cache::persist(db, directory, key, std::mem::take(providers));
        }
    }
}

/// The identity of the semantic sidecar run this scan would make; see
/// [`crate::go::semantic::client::sidecar_run_identity`].
fn go_semantic_run_identity(
    db: &AnalysisDb,
    input: &KernelInput<'_>,
    upstream_digests: &BTreeMap<&'static str, crate::analysis_api::Digest>,
) -> Option<String> {
    let files = crate::go::lifecycle::go_files(db);
    let config = provider::go_semantic_request(input.plan, &input.loaded.root).apply(
        crate::go::lifecycle::GoAnalysisConfig::from_settings_files(
            &input.loaded.root,
            &input.loaded.config.languages.go,
            &files,
        )
        .ok()?,
    );
    let upstream = upstream_digests.get("polint.go.syntax")?.to_string();
    crate::go::semantic::client::sidecar_run_identity(&config, &upstream)
}

/// Starts the Go semantic sidecar the moment `polint.go.syntax` has fixed its
/// inputs.
///
/// The sidecar reads the working tree, not anything the kernel builds, so every
/// stage the schedule runs between `polint.go.syntax` and `polint.go.semantic`
/// is wall time the subprocess could have been using. The digest passed here is
/// the same one `polint.go.semantic` will ask `ProviderCtx::dependency_digest`
/// for, so a prefetch keyed on it answers the provider's question or is
/// discarded; see [`crate::go::semantic::prefetch`].
fn start_go_semantic_prefetch(
    db: &AnalysisDb,
    input: &KernelInput<'_>,
    upstream_digests: &BTreeMap<&'static str, crate::analysis_api::Digest>,
) {
    let upstream = upstream_digests
        .get("polint.go.syntax")
        .cloned()
        .unwrap_or_else(|| {
            crate::analysis_api::Digest::absent(
                crate::analysis_api::DigestKind::ProviderOutput,
                "polint.go.syntax",
            )
        });
    let prefetch = crate::go::semantic::prefetch::GoSemanticPrefetch::start(
        &input.loaded.root,
        &input.loaded.config.languages.go,
        db,
        input.cache.sidecar_cache_dir(),
        upstream.to_string(),
        &provider::go_semantic_request(input.plan, &input.loaded.root),
    );
    if prefetch.is_some() {
        crate::analysis_kernel::host::with_provider_host_session_mut(|session| {
            session.go_semantic_prefetch = prefetch;
        });
    }
}

fn provider_failure_outcome(
    stage: crate::analysis_api::ProviderFailureStage,
    reason: crate::analysis_api::ProviderFailureReason,
) -> (
    ProviderOutcomeStatus,
    ProviderFailureStage,
    ProviderFailureReason,
) {
    match (stage, reason) {
        (
            crate::analysis_api::ProviderFailureStage::Setup,
            crate::analysis_api::ProviderFailureReason::Unsupported,
        ) => (
            ProviderOutcomeStatus::Unsupported,
            ProviderFailureStage::Setup,
            ProviderFailureReason::Unsupported,
        ),
        (
            crate::analysis_api::ProviderFailureStage::Setup,
            crate::analysis_api::ProviderFailureReason::SetupMissing,
        ) => (
            ProviderOutcomeStatus::SetupMissing,
            ProviderFailureStage::Setup,
            ProviderFailureReason::SetupMissing,
        ),
        (
            crate::analysis_api::ProviderFailureStage::Execution,
            crate::analysis_api::ProviderFailureReason::ExecutionFailed,
        ) => (
            ProviderOutcomeStatus::Failed,
            ProviderFailureStage::Execution,
            ProviderFailureReason::ExecutionFailed,
        ),
        (
            crate::analysis_api::ProviderFailureStage::Validation,
            crate::analysis_api::ProviderFailureReason::ValidationRejected,
        ) => (
            ProviderOutcomeStatus::Failed,
            ProviderFailureStage::Validation,
            ProviderFailureReason::ValidationRejected,
        ),
        // ProviderExecution is a public data carrier, so keep malformed pairs
        // from entering the sealed tracker state. Treat them as execution
        // failures without certifying an output.
        _ => (
            ProviderOutcomeStatus::Failed,
            ProviderFailureStage::Execution,
            ProviderFailureReason::ExecutionFailed,
        ),
    }
}

pub(crate) struct AnalysisKernel;

pub(crate) struct KernelInput<'a> {
    pub(crate) loaded: &'a LoadedConfig,
    pub(crate) cache: &'a Cache,
    pub(crate) config_digest: &'a str,
    pub(crate) rule_digest: &'a str,
    pub(crate) plan: &'a AnalysisPlan,
    pub(crate) parallel: bool,
}

pub(crate) struct KernelOutput {
    pub(crate) db: AnalysisDb,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) capability_support: CapabilitySupportView,
    pub(crate) completeness: crate::core::CompletenessView,
    pub(crate) run_report: incremental::KernelRunReport,
    pub(crate) runtime_blocked_rules: BTreeSet<String>,
}

impl AnalysisKernel {
    pub(crate) fn provider_manifests() -> &'static [ProviderManifest] {
        provider::provider_manifests()
    }

    pub(crate) fn run(input: KernelInput<'_>) -> anyhow::Result<KernelOutput> {
        Self::run_with(input, false)
    }

    /// Runs the kernel for a rule check or review, whose readers after the run
    /// are the rules and the report. When the plan requests nothing past
    /// syntax and the metrics derived from it, the metadata of restored syntax
    /// facts and of metric facts is recorded only if validation or a syntax
    /// provider's fallback identity needs it (see
    /// [`AnalysisDb::defer_syntax_fact_metadata`]), and the go.syntax identity
    /// is the adapter's native digest rather than the canonical projection.
    pub(crate) fn run_for_rule_check(input: KernelInput<'_>) -> anyhow::Result<KernelOutput> {
        Self::run_with(input, true)
    }

    fn run_with(input: KernelInput<'_>, rule_check: bool) -> anyhow::Result<KernelOutput> {
        let requested_capabilities = requested_trigger_capabilities(input.plan);
        let run_cross_file_analysis = requested_capabilities.iter().any(|capability| {
            matches!(
                *capability,
                "resolved_imports"
                    | "module_graph"
                    | "symbols"
                    | "references"
                    | "calls"
                    | "control_flow"
                    | "dataflow"
                    | "call_graph"
                    | "go_types"
                    | "routes"
            )
        });
        let rule_scope = if run_cross_file_analysis {
            None
        } else {
            Self::rule_scope_globset(input.plan)
        };
        tracing::info!(
            target: "polint::kernel",
            ?requested_capabilities,
            rule_scoped = rule_scope.is_some(),
            "analysis kernel pipeline gate"
        );

        let (mut db, load_diagnostics) =
            crate::fs::load_analysis_files_scoped(input.loaded, rule_scope.as_ref())?;
        // Every capability a provider past syntax does work for is a trigger
        // capability. The metric ones give work only to the metrics provider,
        // which derives its facts from syntax facts without reading their
        // metadata or the go.syntax identity, and defers its own metadata with
        // theirs. So with no other one requested, nothing in the run reads
        // syntax metadata before validation, or the go.syntax identity at all.
        let syntax_level_rule_check = rule_check
            && requested_capabilities
                .iter()
                .all(|capability| METRIC_CAPABILITIES.contains(capability));
        if syntax_level_rule_check {
            db.defer_syntax_fact_metadata();
        }
        log_loaded_source_files(&db);
        let scope_diagnostics = if run_cross_file_analysis {
            rule_scope_narrower_than_analysis(input.plan, &db)
        } else {
            Vec::new()
        };

        let input_snapshot = incremental::input_snapshot_from_run_inputs(
            input.loaded,
            &db,
            input.config_digest,
            input.rule_digest,
            input.plan.digest(),
            Self::provider_manifests(),
        );
        let mut diagnostics = load_diagnostics;
        diagnostics.extend(scope_diagnostics);
        let mut provider_outputs = Vec::new();
        let enabled_providers = providers_enabled_by_capability_closure(&requested_capabilities);
        debug_assert_eq!(
            scheduled_order(),
            Self::provider_manifests()
                .iter()
                .map(|manifest| manifest.id)
                .collect::<Vec<_>>(),
            "scheduled provider order must match declared manifest order"
        );
        debug_assert_eq!(
            enabled_providers,
            providers_enabled_by_boolean_gates(&requested_capabilities),
            "capability closure must match boolean gates for {requested_capabilities:?}"
        );
        debug_assert_eq!(
            scheduled_order_for(&requested_capabilities),
            scheduled_order()
                .into_iter()
                .filter(|id| enabled_providers.contains(id))
                .collect::<Vec<_>>(),
        );

        let (capability_support, scc_closure, provider_tracker, provider_telemetry) =
            run_scheduled_providers(
                &mut db,
                &input,
                &input_snapshot,
                &enabled_providers,
                syntax_level_rule_check,
                &mut diagnostics,
                &mut provider_outputs,
            )?;
        tracing::info!(target: "polint::kernel", "phase: metrics + derived done");

        let validation_downgrades = if validation::fact_metadata_validation_enabled() {
            // Validation reads every metadata row. A run that deferred syntax
            // metadata validates a copy with the rows recorded and returns the
            // database a release build returns, so a reader of deferred
            // metadata after the run trips `AnalysisDb::metadata_for` in tests
            // instead of passing them.
            let recorded;
            let validated = if db.deferred_syntax_metadata_len() > 0 {
                let mut copy = db.clone();
                copy.record_deferred_syntax_metadata();
                recorded = copy;
                &recorded
            } else {
                &db
            };
            let validation_report =
                validation::validate_fact_metadata(validated, Self::provider_manifests());
            diagnostics.extend(validation_report.iter().cloned());
            validation_report.downgrades()
        } else {
            ValidationDowngrades::default()
        };
        let provider_outcomes = provider_tracker
            .seal(&validation_downgrades)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let (runtime_blocked_rules, capability_diagnostics, unavailable_capabilities) =
            Self::runtime_capability_blockers(input.plan, &db, &provider_outcomes);
        diagnostics.extend(capability_diagnostics);
        db.set_unavailable_capabilities(unavailable_capabilities);
        db.finish_all_fact_meta_insertions();
        let store_config = store::StoreConfig::new(
            input.cache.semantic_store_path(),
            input.cache.semantic_store_enabled(),
        );
        let store_status = store::SemanticStore::maintain(&store_config);
        #[cfg(test)]
        let scc_closure_debug = scc_closure
            .as_ref()
            .and_then(|output| output.debug_snapshot.clone());
        let demand_query_trace = scc_closure
            .map(|output| output.demand_query_trace)
            .unwrap_or_default();
        let mut run_report = incremental::KernelRunReport::new(
            input_snapshot,
            provider_outcomes,
            provider_telemetry,
            demand_query_trace,
            store_status,
        );
        run_report.provider_outputs = provider_outputs;
        let completeness = completeness::view_from_run(
            input.plan,
            &db,
            &run_report.provider_outcomes,
            &diagnostics,
        );
        #[cfg(test)]
        let run_report = run_report.with_scc_closure_debug(scc_closure_debug);

        Ok(KernelOutput {
            db,
            diagnostics,
            capability_support,
            completeness,
            run_report,
            runtime_blocked_rules,
        })
    }

    #[cfg(test)]
    pub(crate) fn missing_fact_metadata_for_test(db: &AnalysisDb) -> Vec<MissingFactMeta> {
        db.missing_fact_metadata()
    }

    #[cfg(test)]
    pub(crate) fn metadata_debug_json_for_test(db: &AnalysisDb) -> serde_json::Value {
        debug::metadata_debug_json_for_test(db)
    }

    #[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
    pub(crate) fn metadata_debug_json_for_output_for_test(
        output: &KernelOutput,
    ) -> serde_json::Value {
        debug::metadata_debug_json_with_demand_trace_for_test(
            &output.db,
            output.run_report.demand_query_trace(),
            output.run_report.scc_closure_debug(),
        )
    }

    #[cfg(test)]
    pub(crate) fn input_snapshot_json_for_test(output: &KernelOutput) -> serde_json::Value {
        serde_json::to_value(&output.run_report.input_snapshot)
            .expect("input snapshot should serialize")
    }

    #[cfg(test)]
    pub(crate) fn provider_output_report_for_test(
        output: &KernelOutput,
    ) -> Vec<incremental::ProviderOutputMeta> {
        output.run_report.provider_outputs.clone()
    }

    #[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
    pub(crate) fn semantic_store_schema_is_current_for_test(path: &std::path::Path) -> bool {
        store::current_schema_is_valid_for_test(path)
    }

    fn provider_output_for_with_optional_digest(
        provider_id: &'static str,
        db: &AnalysisDb,
        cache_stats: incremental::CacheStats,
        output_digest: Option<incremental::Digest>,
        publish_identity: bool,
    ) -> incremental::ProviderOutputMeta {
        let manifest = Self::provider_manifest(provider_id);
        let (output_digest, validation) = if !publish_identity {
            (
                incremental::Digest::absent(incremental::DigestKind::ProviderOutput, provider_id),
                "provider_failed",
            )
        } else {
            match output_digest {
                Some(output_digest) => (output_digest, "native_trusted"),
                None => (
                    incremental::provider_output_digest_from_manifest(
                        manifest,
                        &provider_output_summary_parts(db, manifest),
                    ),
                    "native_trusted",
                ),
            }
        };
        let mut meta =
            incremental::provider_output_from_manifest(manifest, output_digest, cache_stats);
        meta.validation = validation.to_string();
        meta
    }

    /// Union of every enabled rule's `files` scope, as a glob set, or `None` when the
    /// set cannot safely narrow discovery — i.e. there are no rules, or some rule has
    /// an empty `files` scope (which matches every file). Returning `None` falls back
    /// to full workspace discovery.
    fn rule_scope_globset(plan: &AnalysisPlan) -> Option<globset::GlobSet> {
        let rules = plan.rules();
        if rules.is_empty() {
            return None;
        }
        let mut patterns = Vec::new();
        for rule in rules {
            if rule.files.is_empty() {
                return None;
            }
            patterns.extend(rule.files.iter().cloned());
        }
        crate::config::build_glob_set(&patterns).ok()
    }

    fn provider_manifest(provider_id: &str) -> &'static ProviderManifest {
        Self::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == provider_id)
            .unwrap_or_else(|| panic!("missing provider manifest {provider_id}"))
    }

    /// The rules that cannot run because a capability they require did not
    /// succeed, their diagnostics, and every requested capability that is not
    /// available; a rule that requested one optionally runs without it.
    pub(crate) fn runtime_capability_blockers(
        plan: &AnalysisPlan,
        db: &AnalysisDb,
        outcomes: &[ProviderOutcome],
    ) -> (BTreeSet<String>, Vec<Diagnostic>, BTreeSet<String>) {
        let by_id = outcomes
            .iter()
            .map(|outcome| (outcome.provider_id.as_str(), outcome))
            .collect::<BTreeMap<_, _>>();
        let mut blocked_rules = BTreeSet::new();
        let mut diagnostics = Vec::new();
        let mut unavailable = BTreeSet::new();
        for rule in plan.rules() {
            for capability in &rule.requested_capabilities {
                if plan.support_view().status_for(capability)
                    != Some(crate::core::CapabilitySupportStatus::Supported)
                {
                    unavailable.insert(capability.clone());
                    continue;
                }
                let mut providers = Self::capability_providers(capability, db);
                if capability == "events" {
                    for (provider_id, has_rows) in [
                        ("polint.calls", !db.call_sites().is_empty()),
                        ("polint.refined_calls", !db.refined_call_edges().is_empty()),
                    ] {
                        if has_rows
                            && by_id.get(provider_id).is_some_and(|outcome| {
                                outcome.status != ProviderOutcomeStatus::PlannedAbsent
                            })
                        {
                            providers.push(provider_id);
                        }
                    }
                }
                let failed = providers
                    .into_iter()
                    .filter_map(|provider_id| by_id.get(provider_id).copied())
                    .filter(|outcome| outcome.status != ProviderOutcomeStatus::Succeeded)
                    .collect::<Vec<_>>();
                if failed.is_empty() {
                    if provider::reads_typed_go_frontend(capability)
                        && provider::go_types_unloaded(db)
                    {
                        unavailable.insert(capability.clone());
                        if !rule.optional_capabilities.contains(capability) {
                            blocked_rules.insert(rule.id.clone());
                            diagnostics
                                .push(typed_go_frontend_unloaded_diagnostic(&rule.id, capability));
                        }
                    }
                    continue;
                }
                unavailable.insert(capability.clone());
                if rule.optional_capabilities.contains(capability) {
                    continue;
                }
                blocked_rules.insert(rule.id.clone());
                let blockers = failed
                    .iter()
                    .flat_map(|outcome| match outcome.blockers.as_slice() {
                        [] => std::slice::from_ref(&outcome.provider_id),
                        blockers => blockers,
                    })
                    .cloned()
                    .collect::<BTreeSet<_>>();
                diagnostics.push(
                    Diagnostic::error(
                        "polint/capability",
                        "<workspace>",
                        TextRange::point(1, 1),
                        format!(
                            "Rule `{}` requested capability `{capability}`, but provider closure did not succeed.",
                            rule.id
                        ),
                    )
                    .with_evidence("rule", rule.id.clone())
                    .with_evidence("capability", capability.clone())
                    .with_evidence("status", failed[0].status.label())
                    .with_evidence("blockers", blockers.into_iter().collect::<Vec<_>>().join(",")),
                );
            }
        }
        (blocked_rules, diagnostics, unavailable)
    }

    fn capability_providers(capability: &str, db: &AnalysisDb) -> Vec<&'static str> {
        let providers: &[&str] = match capability {
            "source_files" => &["polint.source"],
            "syntax" | "packages" | "functions" | "imports" => {
                &["polint.go.syntax", "polint.ts.syntax"]
            }
            "go_tests" | "branch_obligations" | "coverage_facts" => &["polint.go.syntax"],
            "string_literals" => &["polint.go.syntax", "polint.ts.syntax"],
            "ts_components" | "ts_classes" | "jsx_attributes" => &["polint.ts.syntax"],
            "events" => &["polint.go.syntax", "polint.ts.syntax"],
            "resolved_imports" | "module_graph" => &["polint.module_graph"],
            "symbols" | "references" => &["polint.symbol_graph"],
            "calls" | "control_flow" | "cfg" => &["polint.refined_calls"],
            "call_graph" => &["polint.calls"],
            "go_types" | "routes" => &["polint.go.semantic"],
            "dataflow" => &["polint.evidence"],
            "file_metrics" | "function_metrics" | "complexity_metrics" => &["polint.metrics"],
            _ => &[],
        };
        providers
            .iter()
            .copied()
            .filter(|provider| {
                !matches!(capability, "events" | "string_literals")
                    || db.files().iter().any(|file| match *provider {
                        "polint.go.syntax" => file.language == crate::core::Language::Go,
                        "polint.ts.syntax" => file.language.is_ts_family(),
                        _ => false,
                    })
            })
            .collect()
    }
}

fn is_syntax_provider(provider_id: &str) -> bool {
    matches!(provider_id, "polint.go.syntax" | "polint.ts.syntax")
}

fn typed_go_frontend_unloaded_diagnostic(rule_id: &str, capability: &str) -> Diagnostic {
    Diagnostic::error(
        "polint/capability",
        "<workspace>",
        TextRange::point(1, 1),
        format!(
            "Rule `{rule_id}` requested capability `{capability}`, but the typed Go frontend loaded no package: Go files outside every go.mod module root are not type-checked."
        ),
    )
    .with_evidence("rule", rule_id.to_string())
    .with_evidence("capability", capability.to_string())
    .with_evidence("status", ProviderOutcomeStatus::SetupMissing.label())
    .with_evidence("blockers", "polint.go.semantic")
}

fn provider_output_summary_parts(db: &AnalysisDb, manifest: &ProviderManifest) -> Vec<String> {
    let mut parts = db
        .fact_meta()
        .rows()
        .filter(|(_reference, metadata)| {
            metadata.producer_id == manifest.id || metadata.layer_id == manifest.id
        })
        .flat_map(|(reference, metadata)| {
            [
                format!("fact_family={}", reference.family.label()),
                format!("run_id={}", reference.run_id),
                format!("stable_key={}", db.resolve_stable_key(metadata.stable_key)),
                format!("payload_digest={}", metadata.payload_digest),
                format!("precision={:?}", metadata.precision),
                format!("validation={:?}", metadata.validation),
            ]
        })
        .collect::<Vec<_>>();

    if parts.is_empty() {
        parts.push("fact_summary=empty".to_string());
    }
    parts.sort();
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_kernel::incremental::{CacheStats, INPUT_SNAPSHOT_SCHEMA_VERSION};
    use crate::config::load_config;
    use crate::core::{
        BranchObligation, ComplexityMetricFact, CoverageFact, DefinitionFact, DefinitionId,
        DefinitionKind, FileMetricFact, FunctionFact, FunctionId, FunctionMetricFact, ImportFact,
        ImportId, JsxAttributeFact, Language, ModuleEdge, ModuleEdgeId, ModuleEdgeKind, ModuleNode,
        ModuleNodeId, ModuleNodeKind, PackageFact, PackageId, ReferenceFact, ReferenceId,
        ReferenceKind, ResolutionPrecision, ResolutionStatus, ResolvedImportFact, ResolvedImportId,
        Span, StringLiteralFact, SymbolFact, SymbolId, SymbolKind, SymbolNamespace,
        SymbolPrecision, SymbolResolutionStatus, TestFact, TsClassFact, TsComponentFact,
    };
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    fn span(file: crate::core::FileId, start_byte: u32) -> Span {
        Span::new(
            file,
            start_byte,
            start_byte + 10,
            1,
            start_byte + 1,
            1,
            start_byte + 11,
        )
    }

    fn db_with_one_fact_from_every_current_family() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.tsx"),
            "src/app.tsx".to_string(),
            "import React from 'react';\nexport function Button() { return <button aria-label=\"Save\">Save</button>; }\n".to_string(),
        );
        let package = db.push_package(PackageFact::new(
            PackageId::from_raw(99),
            file,
            "app".to_string(),
            span(file, 0),
            Language::Tsx,
        ));
        let function = db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            file,
            "Button".to_string(),
            span(file, 27),
            Language::Tsx,
            false,
            true,
            1,
            vec!["React.createElement".to_string()],
        ));
        let import = db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            file,
            None,
            "react".to_string(),
            span(file, 0),
            Language::Tsx,
        ));
        let branch = db.push_branch(BranchObligation::new(
            crate::core::BranchId::from_raw(99),
            Some(function),
            file,
            span(file, 40),
            "enabled".to_string(),
            "true".to_string(),
            false,
            "branch:key".to_string(),
        ));
        db.push_test(TestFact::new(
            file,
            Some(function),
            "TestButton".to_string(),
            span(file, 50),
            vec!["render".to_string()],
            1,
            0,
            Vec::new(),
            0,
        ));
        db.push_coverage(CoverageFact::new(branch, Some(true), "fixture".to_string()));
        db.push_ts_component(TsComponentFact::new(
            file,
            Some(function),
            "Button".to_string(),
            span(file, 27),
        ));
        db.push_ts_class(TsClassFact::new(
            file,
            "Dialog".to_string(),
            span(file, 61),
            true,
            false,
        ));
        db.push_string_literal(StringLiteralFact::new(
            file,
            "Save".to_string(),
            span(file, 88),
            Language::Tsx,
        ));
        db.push_jsx_attribute(JsxAttributeFact::new(
            file,
            "aria-label".to_string(),
            Some("Save".to_string()),
            span(file, 72),
        ));
        db.replace_module_graph_facts(
            vec![ResolvedImportFact::new(
                ResolvedImportId::from_raw(99),
                import,
                file,
                Some(ModuleNodeId::from_raw(1)),
                ResolutionStatus::Resolved,
                ResolutionPrecision::ExactFile,
                None,
            )],
            vec![
                ModuleNode::new(
                    ModuleNodeId::from_raw(99),
                    ModuleNodeKind::File,
                    "src/app.tsx".to_string(),
                    Some(file),
                    Some(package),
                    Some(Language::Tsx),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(100),
                    ModuleNodeKind::External,
                    "react".to_string(),
                    None,
                    None,
                    Some(Language::Tsx),
                ),
            ],
            vec![ModuleEdge::new(
                ModuleEdgeId::from_raw(99),
                ModuleNodeId::from_raw(0),
                ModuleNodeId::from_raw(1),
                Some(import),
                Some(ResolvedImportId::from_raw(0)),
                ModuleEdgeKind::Imports,
                ResolutionStatus::Resolved,
            )],
        );
        let interner = db.stable_key_interner();
        db.replace_symbol_graph_facts(
            vec![SymbolFact::new(
                SymbolId::from_raw(0),
                Language::Tsx,
                "Button".to_string(),
                "Button".to_string(),
                SymbolKind::Function,
                SymbolNamespace::Value,
                Some(file),
                Some(package),
                Some(ModuleNodeId::from_raw(0)),
                None,
                Some(span(file, 27)),
                true,
                interner.intern("symbol:Button".to_string()),
                SymbolPrecision::ExactLocal,
            )],
            vec![DefinitionFact::new(
                DefinitionId::from_raw(0),
                SymbolId::from_raw(0),
                Language::Tsx,
                "Button".to_string(),
                "Button".to_string(),
                DefinitionKind::Declaration,
                SymbolNamespace::Value,
                Some(file),
                Some(package),
                Some(ModuleNodeId::from_raw(0)),
                None,
                Some(span(file, 27)),
                true,
                true,
                interner.intern("definition:Button".to_string()),
                SymbolPrecision::ExactLocal,
            )],
            vec![ReferenceFact::new(
                ReferenceId::from_raw(0),
                Language::Tsx,
                "Button".to_string(),
                "Button".to_string(),
                ReferenceKind::Read,
                SymbolNamespace::Value,
                Some(file),
                Some(package),
                Some(ModuleNodeId::from_raw(0)),
                None,
                Some(span(file, 27)),
                Some(SymbolId::from_raw(0)),
                Vec::new(),
                interner.intern("reference:Button".to_string()),
                SymbolResolutionStatus::Resolved,
                SymbolPrecision::ExactLocal,
            )],
        );
        db.replace_metric_facts(
            vec![FileMetricFact::new(file, Language::Tsx, 2, 2, 100, 1)],
            vec![FunctionMetricFact::new(
                function,
                file,
                "Button".to_string(),
                span(file, 27),
                Language::Tsx,
                1,
                10,
            )],
            vec![ComplexityMetricFact::new(
                function,
                file,
                "Button".to_string(),
                span(file, 27),
                Language::Tsx,
                1,
            )],
        );
        db
    }

    #[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
    #[test]
    fn rule_check_runs_defer_syntax_metadata_without_changing_results() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("main.go"),
            "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tif len(\"x\") > 0 {\n\t\tfmt.Println(\"hi\")\n\t}\n}\n",
        )
        .expect("write go");
        std::fs::write(
            temp.path().join("app.ts"),
            "import { x } from './x';\nexport function render(value: number) { return `${x}${value}`; }\n",
        )
        .expect("write ts");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan =
            AnalysisPlan::from_capability_names_for_test(&["syntax", "imports", "string_literals"]);
        let input = || KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        };
        let eager = AnalysisKernel::run(input()).expect("kernel runs");
        let mut deferred = AnalysisKernel::run_for_rule_check(input()).expect("kernel runs");

        assert_eq!(deferred.diagnostics, eager.diagnostics);
        assert_eq!(
            serde_json::to_string(deferred.db.functions()).expect("functions serialize"),
            serde_json::to_string(eager.db.functions()).expect("functions serialize")
        );
        assert_eq!(
            serde_json::to_string(deferred.db.imports()).expect("imports serialize"),
            serde_json::to_string(eager.db.imports()).expect("imports serialize")
        );
        assert_eq!(
            serde_json::to_string(deferred.db.string_literals()).expect("literals serialize"),
            serde_json::to_string(eager.db.string_literals()).expect("literals serialize")
        );
        // Validation ran on a recorded copy: the run returns what a release
        // build returns, restored syntax facts without their metadata rows.
        assert!(deferred.db.deferred_syntax_metadata_len() > 0);
        assert!(deferred.db.fact_meta().row_count() < eager.db.fact_meta().row_count());
        deferred.db.record_deferred_syntax_metadata();
        assert_eq!(
            deferred.db.fact_meta().row_count(),
            eager.db.fact_meta().row_count()
        );
    }

    /// A rule check whose only trigger capabilities are the metric ones keeps
    /// syntax metadata deferred, and the metric facts' metadata with it, on a
    /// cold cache and on a warm one, and nothing the rules or the report read
    /// changes. Once recorded, the metadata rows are the eager run's.
    #[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
    #[test]
    fn metrics_rule_checks_defer_metadata_without_changing_results() {
        fn json<T: serde::Serialize>(facts: &[T]) -> String {
            serde_json::to_string(facts).expect("facts serialize")
        }
        fn metadata_rows(db: &AnalysisDb) -> Vec<(FactRef, String, FactMeta)> {
            db.fact_meta()
                .rows()
                .map(|(reference, metadata)| {
                    let key = db.resolve_stable_key(metadata.stable_key).to_string();
                    (reference, key, metadata.clone())
                })
                .collect()
        }
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("main.go"),
            "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tif len(\"x\") > 0 {\n\t\tfmt.Println(\"hi\")\n\t}\n}\n",
        )
        .expect("write go");
        std::fs::write(
            temp.path().join("app.ts"),
            "import { x } from './x';\nexport function render(value: number) {\n  if (value > 1) { return `${x}${value}`; }\n  return x;\n}\n",
        )
        .expect("write ts");
        let loaded = load_config(temp.path()).expect("default config loads");
        let disabled = Cache::new("", false);
        let warm = Cache::new(temp.path().join(".polint/cache/analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&[
            "syntax",
            "imports",
            "string_literals",
            "file_metrics",
            "function_metrics",
            "complexity_metrics",
        ]);
        let run = |cache: &Cache, rule_check: bool| {
            let input = KernelInput {
                loaded: &loaded,
                cache,
                config_digest: "config",
                rule_digest: "rules",
                plan: &plan,
                parallel: false,
            };
            if rule_check {
                AnalysisKernel::run_for_rule_check(input)
            } else {
                AnalysisKernel::run(input)
            }
            .expect("kernel runs")
        };
        let _prime = run(&warm, false);

        for (label, cache) in [("cold", &disabled), ("warm", &warm)] {
            let eager = run(cache, false);
            let mut deferred = run(cache, true);

            assert_eq!(deferred.diagnostics, eager.diagnostics, "{label}");
            for (family, deferred_facts, eager_facts) in [
                (
                    "functions",
                    json(deferred.db.functions()),
                    json(eager.db.functions()),
                ),
                (
                    "imports",
                    json(deferred.db.imports()),
                    json(eager.db.imports()),
                ),
                (
                    "string literals",
                    json(deferred.db.string_literals()),
                    json(eager.db.string_literals()),
                ),
                (
                    "file metrics",
                    json(deferred.db.file_metrics()),
                    json(eager.db.file_metrics()),
                ),
                (
                    "function metrics",
                    json(deferred.db.function_metrics()),
                    json(eager.db.function_metrics()),
                ),
                (
                    "complexity metrics",
                    json(deferred.db.complexity_metrics()),
                    json(eager.db.complexity_metrics()),
                ),
            ] {
                assert_eq!(deferred_facts, eager_facts, "{label}: {family}");
            }
            assert!(!eager.db.function_metrics().is_empty(), "{label}");
            // Every provider outcome but the go.syntax identity, which a rule
            // check keeps native: nothing it runs reads the canonical one.
            let outcomes = |output: &KernelOutput| {
                output
                    .run_report
                    .provider_outcomes
                    .iter()
                    .filter(|outcome| outcome.provider_id != "polint.go.syntax")
                    .cloned()
                    .collect::<Vec<_>>()
            };
            assert_eq!(outcomes(&deferred), outcomes(&eager), "{label}");
            // The rows the report prints, less the wall time two runs never share.
            let report_rows = |output: &KernelOutput| {
                provider_outcome_rows(
                    &output.run_report.provider_outcomes,
                    &output.run_report.provider_telemetry,
                )
                .into_iter()
                .map(|mut row| {
                    row.elapsed_ms = None;
                    row
                })
                .collect::<Vec<_>>()
            };
            assert_eq!(report_rows(&deferred), report_rows(&eager), "{label}");
            assert_eq!(deferred.completeness, eager.completeness, "{label}");

            // Validation ran on a recorded copy: the run returns what a release
            // build returns, restored facts without their metadata rows.
            assert!(deferred.db.deferred_syntax_metadata_len() > 0, "{label}");
            assert_eq!(
                deferred
                    .db
                    .fact_meta()
                    .family_rows(FactFamily::ComplexityMetric)
                    .count(),
                0,
                "{label}"
            );
            deferred.db.record_deferred_syntax_metadata();
            assert_eq!(
                metadata_rows(&deferred.db),
                metadata_rows(&eager.db),
                "{label}"
            );
        }
    }

    /// Runs a syntax-only rule check over a repository holding one source file.
    ///
    /// The syntax provider of the absent language restores nothing and has no
    /// native identity, so it takes the fallback digest. That fallback must not
    /// record the rows the other provider deferred before it, nor switch deferral
    /// off for the provider that runs after it: either way the returned database
    /// would carry every metadata row, and the rule check would pay for all of it.
    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    fn assert_single_language_rule_check_keeps_metadata_deferred(path: &str, source: &str) {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join(path), source).expect("write source");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan =
            AnalysisPlan::from_capability_names_for_test(&["syntax", "imports", "string_literals"]);
        let output = AnalysisKernel::run_for_rule_check(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel runs");

        assert!(
            output.db.deferred_syntax_metadata_len() > 0,
            "a rule check over only {path} recorded its deferred syntax metadata"
        );
    }

    #[cfg(feature = "lang-go")]
    #[test]
    fn go_only_rule_checks_keep_syntax_metadata_deferred() {
        assert_single_language_rule_check_keeps_metadata_deferred(
            "main.go",
            "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n",
        );
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn typescript_only_rule_checks_keep_syntax_metadata_deferred() {
        assert_single_language_rule_check_keeps_metadata_deferred(
            "app.ts",
            "import { x } from './x';\nexport function render(value: number) { return `${x}${value}`; }\n",
        );
    }

    #[cfg(feature = "lang-go")]
    #[test]
    fn syntax_level_rule_checks_keep_the_native_go_syntax_identity() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("main.go"),
            "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n",
        )
        .expect("write go");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let run = |plan: &AnalysisPlan, rule_check: bool| {
            let input = KernelInput {
                loaded: &loaded,
                cache: &cache,
                config_digest: "config",
                rule_digest: "rules",
                plan,
                parallel: false,
            };
            let output = if rule_check {
                AnalysisKernel::run_for_rule_check(input)
            } else {
                AnalysisKernel::run(input)
            }
            .expect("kernel runs");
            let identity = output
                .run_report
                .provider_outcomes
                .iter()
                .find(|outcome| outcome.provider_id == "polint.go.syntax")
                .and_then(|outcome| outcome.output_identity.clone())
                .expect("go.syntax succeeds")
                .output_digest;
            let native = output
                .run_report
                .provider_outputs
                .iter()
                .find(|row| row.provider_id == "polint.go.syntax")
                .expect("go.syntax output row")
                .output_digest
                .clone();
            let canonical = go_syntax_projection::CanonicalGoSyntaxOutput::from_db(&output.db, &[])
                .expect("projection")
                .digest();
            (identity, native, canonical)
        };
        let syntax = AnalysisPlan::from_capability_names_for_test(&["syntax"]);
        let metrics = AnalysisPlan::from_capability_names_for_test(&["syntax", "function_metrics"]);
        let module_graph =
            AnalysisPlan::from_capability_names_for_test(&["syntax", "module_graph"]);

        for plan in [&syntax, &metrics] {
            let (identity, native, canonical) = run(plan, true);
            assert_eq!(identity, native);
            assert_ne!(identity, canonical);
        }
        for (plan, rule_check) in [
            (&syntax, false),
            (&metrics, false),
            (&module_graph, true),
            (&module_graph, false),
        ] {
            let (identity, _, canonical) = run(plan, rule_check);
            assert_eq!(identity, canonical);
        }
    }

    #[test]
    fn run_with_empty_plan_returns_empty_db_and_plan_support() {
        let temp = tempfile::tempdir().expect("tempdir");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::empty();

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run on an empty repo");

        assert!(output.db.files().is_empty());
        assert!(output.diagnostics.is_empty());
        assert_eq!(&output.capability_support, plan.support_view());
    }

    mod semantic_store {
        use super::*;

        #[test]
        fn disabled_full_kernel_run_is_filesystem_free_and_records_status() {
            let temp = tempfile::tempdir().expect("temp directory");
            let loaded = load_config(temp.path()).expect("default config loads");
            let cache = Cache::default_for_repo(temp.path(), true);
            let store_path = cache.semantic_store_path();
            let plan = AnalysisPlan::empty();

            let output = AnalysisKernel::run(KernelInput {
                loaded: &loaded,
                cache: &cache,
                config_digest: "config",
                rule_digest: "rules",
                plan: &plan,
                parallel: false,
            })
            .expect("kernel should run");

            assert_eq!(output.run_report.store_status(), &StoreStatus::Disabled);
            assert!(!store_path.exists());
            assert!(!store_path.parent().expect("store directory").exists());
        }

        #[test]
        fn enabled_maintenance_runs_after_validated_fact_finalization() {
            let temp = tempfile::tempdir().expect("temp directory");
            std::fs::write(temp.path().join("main.go"), "package main\n").expect("write source");
            let loaded = load_config(temp.path()).expect("default config loads");
            let cache =
                Cache::default_for_repo(temp.path(), true).with_semantic_store_enabled_for_test();
            let store_path = cache.semantic_store_path();
            let plan = AnalysisPlan::empty();

            let output = AnalysisKernel::run(KernelInput {
                loaded: &loaded,
                cache: &cache,
                config_digest: "config",
                rule_digest: "rules",
                plan: &plan,
                parallel: false,
            })
            .expect("kernel should run");

            assert!(AnalysisKernel::missing_fact_metadata_for_test(&output.db).is_empty());
            assert_eq!(output.run_report.store_status(), &StoreStatus::Ready);
            assert!(store_path.is_file());
        }
    }

    /// Runs the kernel over a temporary tree holding `files`.
    #[cfg(feature = "lang-go")]
    fn run_tree_for_domain_test(files: &[(&str, &str)], plan: &AnalysisPlan) -> KernelOutput {
        let temp = tempfile::tempdir().expect("temp directory");
        for (path, text) in files {
            std::fs::write(temp.path().join(path), text).expect("write source");
        }
        let loaded = load_config(temp.path()).expect("default config loads");
        AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &Cache::new("", false),
            config_digest: "config",
            rule_digest: "rules",
            plan,
            parallel: false,
        })
        .expect("kernel should run")
    }

    #[cfg(feature = "lang-go")]
    const DOMAIN_TEST_GO: &[(&str, &str)] = &[
        ("go.mod", "module example.com/domains\n\ngo 1.22\n"),
        (
            "main.go",
            "package main\n\nfunc sink(value int) int { return value + 1 }\n\nfunc branch(ready bool) int {\n\tif ready {\n\t\treturn sink(1)\n\t}\n\tpanic(\"never\")\n}\n\nfunc main() { branch(true) }\n",
        ),
    ];

    #[cfg(feature = "lang-go")]
    fn per_operation_domain_facts(output: &KernelOutput) -> usize {
        use crate::analysis_neutral::domains::facts::DomainLocation;
        output
            .db
            .abstract_domain_observations()
            .iter()
            .filter(|row| {
                matches!(
                    row.location,
                    DomainLocation::BeforeOperation | DomainLocation::AfterOperation
                )
            })
            .count()
    }

    /// A summary as the domain test compares it: keys as text, then its fields.
    #[cfg(feature = "lang-go")]
    type SummaryRow = (
        String,
        String,
        crate::analysis_neutral::summaries::facts::SummaryDomainKind,
        crate::analysis_neutral::summaries::facts::SummaryStatus,
        crate::analysis_neutral::summaries::facts::SummaryPrecision,
        crate::analysis_neutral::summaries::facts::SummaryProvenance,
        String,
        Vec<crate::analysis_neutral::summaries::facts::SummaryFlowEdge>,
    );

    #[cfg(feature = "lang-go")]
    fn summary_rows(output: &KernelOutput) -> Vec<SummaryRow> {
        let interner = output.db.stable_key_interner();
        output
            .db
            .summary_facts()
            .iter()
            .map(|fact| {
                (
                    interner.resolve(fact.callable_stable_key).to_string(),
                    interner.resolve(fact.stable_key).to_string(),
                    fact.domain,
                    fact.status,
                    fact.precision,
                    fact.provenance,
                    fact.payload_digest.clone(),
                    fact.tito_flows.clone(),
                )
            })
            .collect()
    }

    /// Asserts the compact domain materialization for `plan` over `files`: no
    /// per-operation states unless asked for, and the same summaries either way.
    #[cfg(feature = "lang-go")]
    fn assert_compact_domains(files: &[(&str, &str)], plan: AnalysisPlan, capability: &str) {
        assert!(!plan.requests_per_point_domain_facts());
        let compact = run_tree_for_domain_test(files, &plan);
        let full = run_tree_for_domain_test(files, &plan.with_per_point_domain_facts());
        assert_eq!(
            per_operation_domain_facts(&compact),
            0,
            "`{capability}` must not materialize per-operation domain states"
        );
        assert!(
            per_operation_domain_facts(&full) > 0,
            "an explicit per-point request must still get per-operation states"
        );
        assert!(!compact.db.summary_facts().is_empty());
        assert_eq!(
            summary_rows(&compact),
            summary_rows(&full),
            "`{capability}` summaries must not depend on the domain materialization"
        );
    }

    /// Control flow gets the compact domain materialization: the summaries read
    /// only entry reachability, and the per-point states were most of a deep
    /// run's time and memory. The summaries a rule sees must not depend on the
    /// choice. Call resolution reads no summary and gets no domain facts at all,
    /// and neither does data flow over Go alone, which the taint solver answers
    /// over the typed frontend's flow programs.
    #[cfg(feature = "lang-go")]
    #[test]
    fn deep_capabilities_get_compact_domain_facts_unless_per_point_states_are_requested() {
        let calls = AnalysisPlan::from_capability_names_for_test(&["calls"]);
        let calls_run = run_tree_for_domain_test(DOMAIN_TEST_GO, &calls);
        assert!(calls_run.db.abstract_domain_observations().is_empty());
        assert!(calls_run.db.summary_facts().is_empty());
        assert!(
            per_operation_domain_facts(&run_tree_for_domain_test(
                DOMAIN_TEST_GO,
                &calls.with_per_point_domain_facts()
            )) > 0
        );

        let control = AnalysisPlan::from_capability_names_for_test(&["control_flow"]);
        assert_compact_domains(DOMAIN_TEST_GO, control, "control_flow");

        let dataflow = AnalysisPlan::from_capability_names_for_test(&["dataflow"]);
        let go_only = run_tree_for_domain_test(DOMAIN_TEST_GO, &dataflow);
        assert!(go_only.db.abstract_domain_observations().is_empty());
        assert!(go_only.db.summary_facts().is_empty());
        assert!(
            go_only.db.go_flow_program().is_some(),
            "Go data flow reads the typed frontend's flow programs"
        );
    }

    /// Data flow over a language the taint solver does not answer still builds
    /// the value-flow graph from compact domains and summaries.
    #[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
    #[test]
    fn graph_data_flow_gets_compact_domain_facts_unless_per_point_states_are_requested() {
        let mut files = DOMAIN_TEST_GO.to_vec();
        files.push((
            "app.ts",
            "export function handler(token: string): string {\n  return token;\n}\n",
        ));
        let dataflow = AnalysisPlan::from_capability_names_for_test(&["dataflow"]);
        assert_compact_domains(&files, dataflow, "dataflow");
    }

    #[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
    #[test]
    fn loop_bodies_stay_reachable_under_a_constant_true_condition() {
        let temp = tempfile::tempdir().expect("temp directory");
        std::fs::write(
            temp.path().join("go.mod"),
            "module example.com/loops\n\ngo 1.22\n",
        )
        .expect("write go.mod");
        std::fs::write(
            temp.path().join("main.go"),
            "package main\n\nfunc sink(value int) {}\n\nfunc loopFlow() {\n\tready := true\n\tfor ready {\n\t\tsink(1)\n\t}\n}\n\nfunc main() { loopFlow() }\n",
        )
        .expect("write Go source");
        std::fs::write(
            temp.path().join("app.ts"),
            "function sink(value: number) {}\n\nexport function loopFlow() {\n  const ready = true;\n  while (ready) {\n    sink(1);\n  }\n}\n\nexport function doFlow() {\n  const ready = true;\n  do {\n    sink(2);\n  } while (ready);\n}\n",
        )
        .expect("write TypeScript source");
        let loaded = load_config(temp.path()).expect("default config loads");
        // The assertion reads the state after each call, which only the
        // per-point materialization records.
        let plan = AnalysisPlan::from_capability_names_for_test(&["dataflow"])
            .with_per_point_domain_facts();

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &Cache::new("", false),
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");
        assert!(
            output.db.abstract_domain_observations().iter().any(|row| {
                row.location
                    == crate::analysis_neutral::domains::facts::DomainLocation::AfterOperation
            }),
            "the fixture must materialize per-operation states"
        );

        // A loop body is lowered into the block its branch reaches on the
        // `false` edge, so binding the loop condition would refine the body with
        // the loop-exit assumption and retract every block behind it.
        let call_operations = output
            .db
            .mir_operations()
            .iter()
            .filter(|operation| {
                matches!(
                    operation.kind,
                    crate::analysis_neutral::mir_op::MirOperationKind::Call { .. }
                )
            })
            .map(|operation| operation.id)
            .collect::<BTreeSet<_>>();
        assert!(!call_operations.is_empty(), "fixture should lower calls");

        let unreachable_calls = output
            .db
            .abstract_domain_observations()
            .iter()
            .filter(|row| {
                row.slot == crate::analysis_neutral::domains::facts::DomainSlot::Reachability
                    && row
                        .operation
                        .is_some_and(|operation| call_operations.contains(&operation))
                    && matches!(
                        &row.value,
                        crate::analysis_neutral::domains::facts::DomainValue::Label(label)
                            if label == "unreachable"
                    )
            })
            .count();
        assert_eq!(
            unreachable_calls,
            0,
            "{:#?}",
            output.db.abstract_domain_observations()
        );
    }

    #[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
    #[test]
    fn scheduled_deep_provider_stack_has_complete_valid_metadata() {
        let temp = tempfile::tempdir().expect("temp directory");
        std::fs::write(
            temp.path().join("main.go"),
            "package main\n\nfunc main() { println(\"hello\") }\n",
        )
        .expect("write Go source");
        std::fs::write(
            temp.path().join("app.ts"),
            "export function app(value: string) { return value; }\napp(\"hello\");\n",
        )
        .expect("write TypeScript source");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::from_capability_names_for_test(&["dataflow"]);

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("scheduled deep provider stack should run");

        for provider_id in [
            "polint.semantic_mir",
            "polint.cfg",
            "polint.calls",
            "polint.identity",
            "polint.abstract_domains",
            "polint.entrypoints",
            "polint.reachability",
            "polint.semantic_graph",
            "polint.solver",
            "polint.refined_calls",
            "polint.data_flow",
            "polint.evidence",
        ] {
            let outcome = output
                .run_report
                .provider_outcomes
                .iter()
                .find(|row| row.provider_id == provider_id)
                .unwrap_or_else(|| panic!("missing scheduled provider outcome {provider_id}"));
            assert_eq!(
                outcome.status,
                ProviderOutcomeStatus::Succeeded,
                "scheduled provider {provider_id} did not succeed: {outcome:?}"
            );
        }

        let missing = AnalysisKernel::missing_fact_metadata_for_test(&output.db);
        assert!(
            missing.is_empty(),
            "unexpected missing metadata: {missing:?}"
        );

        let report = validation::validate_fact_metadata(&output.db, provider::provider_manifests());
        assert!(
            report.is_empty(),
            "unexpected metadata diagnostics: {report:#?}"
        );
    }

    #[test]
    fn kernel_run_respects_fact_metadata_validation_gate() {
        let temp = tempfile::tempdir().expect("temp directory");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::empty();
        let before = validation::fact_metadata_validation_call_count_for_test();
        let enabled = validation::fact_metadata_validation_enabled();

        AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");

        let after = validation::fact_metadata_validation_call_count_for_test();
        if enabled {
            assert!(
                after > before,
                "gated kernel path must call validate_fact_metadata when enabled"
            );
        } else {
            assert_eq!(
                after, before,
                "gated kernel path must skip validate_fact_metadata when disabled"
            );
        }
    }

    mod semantic_store_check_parity {
        use super::*;
        use crate::diagnostics::{
            ColorChoice, JsonReportMeta, OutputFormat, RenderOpts, Severity, render,
            sort_diagnostics,
        };

        #[derive(Clone, Copy)]
        enum StoreMode {
            Disabled,
            Enabled,
            Corrupt,
            Future,
            Invalid,
            Busy,
        }

        fn run_mode(mode: StoreMode) -> (String, u8, StoreStatus) {
            let temp = tempfile::tempdir().expect("temp directory");
            std::fs::write(
                temp.path().join("main.go"),
                "package main\n\nfunc main() { println(\"hello\") }\n",
            )
            .expect("write source");
            let loaded = load_config(temp.path()).expect("default config loads");
            let cache = Cache::default_for_repo(temp.path(), true);
            let cache = if matches!(mode, StoreMode::Disabled) {
                cache
            } else {
                cache.with_semantic_store_enabled_for_test()
            };
            let path = cache.semantic_store_path();
            let config = store::StoreConfig::new(&path, true);

            let mut corrupt_before = None;
            let mut fixture_before = None;
            let mut held_writer = None;
            match mode {
                StoreMode::Disabled | StoreMode::Enabled => {}
                StoreMode::Corrupt => {
                    std::fs::create_dir_all(path.parent().expect("store directory"))
                        .expect("create store directory");
                    let bytes = b"parity corrupt sqlite bytes".to_vec();
                    std::fs::write(&path, &bytes).expect("write corrupt fixture");
                    corrupt_before = Some(bytes);
                }
                StoreMode::Future => {
                    std::fs::create_dir_all(path.parent().expect("store directory"))
                        .expect("create store directory");
                    store::install_future_fixture_for_test(&path).expect("install future fixture");
                    fixture_before = Some(
                        store::fixture_snapshot_for_test(&path).expect("snapshot future fixture"),
                    );
                }
                StoreMode::Invalid => {
                    std::fs::create_dir_all(path.parent().expect("store directory"))
                        .expect("create store directory");
                    store::install_invalid_fixture_for_test(&path)
                        .expect("install invalid fixture");
                    fixture_before = Some(
                        store::fixture_snapshot_for_test(&path).expect("snapshot invalid fixture"),
                    );
                }
                StoreMode::Busy => {
                    assert_eq!(store::SemanticStore::maintain(&config), StoreStatus::Ready);
                    fixture_before = Some(
                        store::fixture_snapshot_for_test(&path).expect("snapshot current fixture"),
                    );
                    held_writer = Some(
                        store::hold_writer_connection_for_test(&path).expect("hold writer lease"),
                    );
                }
            }

            let plan = AnalysisPlan::empty();
            let mut output = AnalysisKernel::run(KernelInput {
                loaded: &loaded,
                cache: &cache,
                config_digest: "config",
                rule_digest: "rules",
                plan: &plan,
                parallel: false,
            })
            .expect("kernel run");
            let status = output.run_report.store_status().clone();
            sort_diagnostics(&mut output.diagnostics);
            let exit_code = u8::from(
                output
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.severity.is_at_least(Severity::Warn)),
            );
            let json = render(
                OutputFormat::Json,
                &output.diagnostics,
                RenderOpts {
                    json: JsonReportMeta {
                        tool_name: "polint",
                        tool_version: env!("CARGO_PKG_VERSION"),
                    },
                    color: ColorChoice::Never,
                    sources: None,
                    rule_execution: &[],
                    run_summary: crate::diagnostics::EMPTY_RUN_SUMMARY,
                },
            );

            if let Some(bytes) = corrupt_before {
                assert_eq!(std::fs::read(&path).expect("read corrupt fixture"), bytes);
            }
            if let Some(before) = fixture_before {
                assert_eq!(
                    store::fixture_snapshot_for_test(&path).expect("snapshot fixture after run"),
                    before
                );
            }
            drop(held_writer);
            (json, exit_code, status)
        }

        #[test]
        fn all_store_modes_preserve_byte_identical_json_and_exit_semantics() {
            let (disabled_json, disabled_exit, disabled_status) = run_mode(StoreMode::Disabled);
            assert_eq!(disabled_status, StoreStatus::Disabled);

            let cases = [
                (StoreMode::Enabled, StoreStatus::Ready),
                (
                    StoreMode::Corrupt,
                    StoreStatus::RebuildNeeded(store::StoreRebuildReason::Corrupt),
                ),
                (
                    StoreMode::Future,
                    StoreStatus::Skipped(store::StoreSkipReason::FutureSchema {
                        found: store::CURRENT_SCHEMA_VERSION_FOR_TEST + 1,
                        supported: store::CURRENT_SCHEMA_VERSION_FOR_TEST,
                    }),
                ),
                (
                    StoreMode::Invalid,
                    StoreStatus::RebuildNeeded(store::StoreRebuildReason::InvalidSchema),
                ),
                (StoreMode::Busy, StoreStatus::BusySkipped),
            ];

            for (mode, expected_status) in cases {
                let (json, exit_code, status) = run_mode(mode);
                assert_eq!(status, expected_status);
                assert_eq!(json.as_bytes(), disabled_json.as_bytes());
                assert_eq!(exit_code, disabled_exit);
            }
        }
    }

    #[test]
    fn kernel_run_report_records_input_snapshot_and_provider_outputs() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("main.go"), "package main\n").expect("write go");
        std::fs::create_dir_all(temp.path().join("src")).expect("create src");
        std::fs::write(
            temp.path().join("src/app.ts"),
            "export const answer = 42;\n",
        )
        .expect("write ts");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::empty();

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");

        let snapshot = AnalysisKernel::input_snapshot_json_for_test(&output);
        let provider_outputs = AnalysisKernel::provider_output_report_for_test(&output);

        assert_eq!(snapshot["schema_version"], INPUT_SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(
            provider_outputs
                .iter()
                .map(|row| row.provider_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "polint.source",
                "polint.go.syntax",
                "polint.ts.syntax",
                "polint.module_graph",
                "polint.symbol_graph",
                "polint.module_topology",
                "polint.semantic_mir",
                "polint.go.semantic",
                "polint.cfg",
                "polint.calls",
                "polint.ts.types",
                "polint.identity",
                "polint.abstract_domains",
                "polint.direct_summaries",
                "polint.entrypoints",
                "polint.reachability",
                "polint.extensions",
                "polint.type_value_alias",
                "polint.semantic_graph",
                "polint.solver",
                "polint.refined_calls",
                "polint.data_flow",
                "polint.evidence",
                "polint.metrics",
            ]
        );
    }

    #[test]
    fn direct_summaries_provider_output_reflects_final_summary_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::empty();

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel run should succeed");
        let manifest = AnalysisKernel::provider_manifest("polint.direct_summaries");
        let generic = incremental::provider_output_digest_from_manifest(
            manifest,
            &provider_output_summary_parts(&output.db, manifest),
        );
        let direct_summaries = provider_output(&output, "polint.direct_summaries");

        assert_ne!(
            direct_summaries.output_digest, generic,
            "post-SCC direct summaries must use the direct-summaries digest, including provider parameters and dependency digests"
        );
        assert_eq!(
            direct_summaries.output_digest.kind,
            incremental::DigestKind::ProviderOutput
        );
    }

    #[test]
    fn failed_derived_provider_output_is_not_reported_as_trusted_fallback_digest() {
        let row = AnalysisKernel::provider_output_for_with_optional_digest(
            "polint.data_flow",
            &AnalysisDb::default(),
            incremental::CacheStats::default(),
            None,
            false,
        );

        assert_eq!(row.validation, "provider_failed");
        assert_eq!(
            row.output_digest.kind,
            incremental::DigestKind::ProviderOutput
        );
    }

    #[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
    #[test]
    fn kernel_run_report_syntax_provider_rows_carry_adapter_cache_stats() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("main.go"), "package main\n").expect("write go");
        std::fs::write(temp.path().join("app.ts"), "export const app = 1;\n").expect("write ts");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::empty();

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");
        let go = provider_output(&output, "polint.go.syntax");
        let ts = provider_output(&output, "polint.ts.syntax");

        assert!(go.cache_stats.bypasses_disabled > 0);
        assert!(go.cache_stats.recomputes > 0);
        assert!(ts.cache_stats.bypasses_disabled > 0);
        assert!(ts.cache_stats.recomputes > 0);
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn kernel_run_report_module_graph_row_carries_layer_cache_stats() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join("src")).expect("create src");
        std::fs::write(
            temp.path().join("src/app.ts"),
            "import tokens from './tokens';\n",
        )
        .expect("write app");
        std::fs::write(
            temp.path().join("src/tokens.ts"),
            "export const tokens = {};\n",
        )
        .expect("write tokens");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["resolved_imports"]);

        let first = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("first kernel run should succeed");
        let second = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("second kernel run should succeed");

        let first_module_graph = provider_output(&first, "polint.module_graph");
        let second_module_graph = provider_output(&second, "polint.module_graph");

        assert_eq!(first_module_graph.cache_stats.misses, 1);
        assert_eq!(first_module_graph.cache_stats.recomputes, 1);
        assert_eq!(first_module_graph.cache_stats.writes, 1);
        assert_eq!(second_module_graph.cache_stats.hits, 1);
        assert_eq!(second_module_graph.cache_stats.verified_reuse, 1);
        assert_eq!(second_module_graph.cache_stats.recomputes, 0);
        assert_eq!(
            first_module_graph.output_digest,
            second_module_graph.output_digest
        );
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn kernel_run_report_symbol_graph_row_carries_layer_cache_stats() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join("src")).expect("create src");
        std::fs::write(
            temp.path().join("src/app.ts"),
            "export function answer() { return 42; }\nexport const value = answer();\n",
        )
        .expect("write app");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["symbols", "references"]);

        let first = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("first kernel run should succeed");
        let second = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("second kernel run should succeed");

        let first_symbol_graph = provider_output(&first, "polint.symbol_graph");
        let second_symbol_graph = provider_output(&second, "polint.symbol_graph");

        assert_eq!(first_symbol_graph.cache_stats.misses, 1);
        assert_eq!(first_symbol_graph.cache_stats.recomputes, 1);
        assert_eq!(first_symbol_graph.cache_stats.writes, 1);
        assert_eq!(second_symbol_graph.cache_stats.hits, 1);
        assert_eq!(second_symbol_graph.cache_stats.verified_reuse, 1);
        assert_eq!(second_symbol_graph.cache_stats.recomputes, 0);
        assert_eq!(
            first_symbol_graph.output_digest,
            second_symbol_graph.output_digest
        );
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn kernel_run_report_module_topology_row_carries_layer_cache_stats() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("package.json"),
            r#"{"name":"root","dependencies":{"react":"^18.0.0"}}"#,
        )
        .expect("write package");
        std::fs::create_dir_all(temp.path().join("src")).expect("create src");
        std::fs::write(
            temp.path().join("src/app.ts"),
            "import React from 'react';\nexport function App() { return React.createElement('main'); }\n",
        )
        .expect("write app");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["calls"]);

        let first = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("first kernel run should succeed");
        let second = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("second kernel run should succeed");
        let disabled_cache = Cache::new("", false);
        let disabled = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &disabled_cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("disabled-cache kernel run should succeed");

        let first_module_topology = provider_output(&first, "polint.module_topology");
        let second_module_topology = provider_output(&second, "polint.module_topology");
        let disabled_module_topology = provider_output(&disabled, "polint.module_topology");

        assert_eq!(first_module_topology.cache_stats.misses, 1);
        assert_eq!(first_module_topology.cache_stats.recomputes, 1);
        assert_eq!(first_module_topology.cache_stats.writes, 1);
        assert!(!first_module_topology.output_digest.value.is_empty());
        assert_eq!(second_module_topology.cache_stats.hits, 1);
        assert_eq!(second_module_topology.cache_stats.verified_reuse, 1);
        assert_eq!(second_module_topology.cache_stats.recomputes, 0);
        assert_eq!(
            first_module_topology.output_digest,
            second_module_topology.output_digest
        );
        assert_eq!(disabled_module_topology.cache_stats.bypasses_disabled, 1);
        assert_eq!(disabled_module_topology.cache_stats.recomputes, 1);
        assert!(!disabled_module_topology.output_digest.value.is_empty());
    }

    #[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
    #[test]
    fn kernel_run_report_semantic_mir_row_carries_output_digest() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("main.go"),
            "package main\nfunc answer() int { return 42 }\n",
        )
        .expect("write go");
        std::fs::write(
            temp.path().join("app.ts"),
            "export function app() { return 42; }\n",
        )
        .expect("write ts");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::from_capability_names_for_test(&["calls"]);

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");
        let semantic_mir = provider_output(&output, "polint.semantic_mir");

        assert_eq!(semantic_mir.schema_version, "semantic-mir-facts-1:1");
        assert!(!semantic_mir.output_digest.value.is_empty());
        assert_eq!(semantic_mir.cache_stats.recomputes, 1);
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn kernel_run_report_cfg_row_carries_output_digest() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("app.ts"),
            "export function app() { return 42; }\n",
        )
        .expect("write ts");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::from_capability_names_for_test(&["calls"]);

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");
        let cfg = provider_output(&output, "polint.cfg");

        assert_eq!(cfg.schema_version, "cfg-facts-1:1");
        assert!(!cfg.output_digest.value.is_empty());
        assert_eq!(cfg.cache_stats.recomputes, 1);
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn kernel_run_report_calls_row_carries_output_digest() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("app.ts"),
            "export function app() { return 42; }\n",
        )
        .expect("write ts");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::from_capability_names_for_test(&["control_flow"]);

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");
        let calls = provider_output(&output, "polint.calls");

        assert_eq!(calls.schema_version, "calls-facts-1:1");
        assert!(!calls.output_digest.value.is_empty());
        assert_eq!(calls.cache_stats.recomputes, 1);
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn kernel_run_report_metrics_row_carries_layer_cache_stats() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join("src")).expect("create src");
        std::fs::write(
            temp.path().join("src/app.ts"),
            "export function answer() { return 42; }\n",
        )
        .expect("write app");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&[
            "file_metrics",
            "function_metrics",
            "complexity_metrics",
        ]);

        let first = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("first kernel run should succeed");
        let second = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("second kernel run should succeed");

        let first_metrics = provider_output(&first, "polint.metrics");
        let second_metrics = provider_output(&second, "polint.metrics");

        assert_eq!(first_metrics.cache_stats.misses, 1);
        assert_eq!(first_metrics.cache_stats.recomputes, 1);
        assert_eq!(first_metrics.cache_stats.writes, 1);
        assert_eq!(second_metrics.cache_stats.hits, 1);
        assert_eq!(second_metrics.cache_stats.verified_reuse, 1);
        assert_eq!(second_metrics.cache_stats.recomputes, 0);
        assert_eq!(first_metrics.output_digest, second_metrics.output_digest);
    }

    #[test]
    fn kernel_surfaces_metrics_layer_cache_write_diagnostics() {
        let temp = tempfile::tempdir().expect("tempdir");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache_root = temp.path().join("cache");
        std::fs::create_dir_all(&cache_root).expect("cache root");
        std::fs::write(cache_root.join("layers"), "not a directory").expect("layer root file");
        let cache = Cache::new(cache_root.join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["file_metrics"]);

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");
        let metrics = provider_output(&output, "polint.metrics");

        assert_eq!(metrics.cache_stats.misses, 0);
        assert_eq!(metrics.cache_stats.invalid_evicted_reads, 1);
        assert_eq!(metrics.cache_stats.recomputes, 1);
        assert_eq!(metrics.cache_stats.writes, 0);
        assert!(output.diagnostics.iter().any(|diagnostic| {
            diagnostic.rule_id == "internal/cache"
                && diagnostic.file == "metrics layer"
                && diagnostic.message.contains("cache write failed")
        }));
    }

    #[test]
    fn kernel_run_report_source_and_derived_provider_rows_have_expected_stats_and_output_digests() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("app.ts"), "export const app = 1;\n").expect("write ts");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::empty();

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");

        for provider_id in [
            "polint.source",
            "polint.module_graph",
            "polint.symbol_graph",
            "polint.module_topology",
            "polint.metrics",
        ] {
            let row = provider_output(&output, provider_id);
            assert_eq!(row.cache_stats, CacheStats::default());
            assert!(!row.output_digest.value.is_empty());
        }

        // With an empty plan no rule requests a graph capability, so the
        // interprocedural/semantic pipeline (semantic_mir and everything
        // downstream) is gated off and never recomputes. Its provider row is still
        // emitted with a deterministic synthesized output digest.
        let semantic_mir = provider_output(&output, "polint.semantic_mir");
        assert_eq!(semantic_mir.cache_stats.recomputes, 0);
        assert!(!semantic_mir.output_digest.value.is_empty());
    }

    #[test]
    fn events_only_plan_stays_off_semantic_pipeline() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("app.ts"),
            "export function app() { fetch('/health'); }\n",
        )
        .expect("write ts");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::from_capability_names_for_test(&["events"]);

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");

        assert_eq!(
            provider_output(&output, "polint.semantic_mir")
                .cache_stats
                .recomputes,
            0
        );
        assert_eq!(
            provider_output(&output, "polint.calls")
                .cache_stats
                .recomputes,
            0
        );
        assert_eq!(
            provider_output(&output, "polint.refined_calls")
                .cache_stats
                .recomputes,
            0
        );
        assert_eq!(
            provider_output(&output, "polint.data_flow")
                .cache_stats
                .recomputes,
            0
        );
        assert_eq!(
            provider_output(&output, "polint.evidence")
                .cache_stats
                .recomputes,
            0
        );
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn symbols_and_references_stay_off_semantic_pipeline() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("app.ts"),
            "export function app() { return 42; }\nexport const value = app();\n",
        )
        .expect("write ts");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::from_capability_names_for_test(&["symbols", "references"]);

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");

        assert_eq!(
            provider_output(&output, "polint.symbol_graph")
                .cache_stats
                .recomputes,
            1
        );
        for provider_id in [
            "polint.module_topology",
            "polint.semantic_mir",
            "polint.cfg",
            "polint.calls",
            "polint.refined_calls",
            "polint.data_flow",
            "polint.evidence",
        ] {
            assert_eq!(
                provider_output(&output, provider_id).cache_stats.recomputes,
                0,
                "{provider_id} should stay off for symbols/references-only plans"
            );
        }
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn control_flow_plan_keeps_cfg_and_refined_call_facts() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("app.ts"),
            "function authorize() {}\nfunction dangerous() {}\nexport function app(input: boolean) { if (input) { authorize(); } dangerous(); }\n",
        )
        .expect("write ts");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::from_capability_names_for_test(&["control_flow"]);

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");

        for provider_id in [
            "polint.semantic_mir",
            "polint.go.semantic",
            "polint.cfg",
            "polint.calls",
            "polint.ts.types",
            "polint.identity",
            "polint.abstract_domains",
            "polint.direct_summaries",
            "polint.entrypoints",
            "polint.reachability",
            "polint.extensions",
            "polint.type_value_alias",
            "polint.semantic_graph",
            "polint.solver",
            "polint.refined_calls",
        ] {
            assert_eq!(
                provider_output(&output, provider_id).cache_stats.recomputes,
                1,
                "{provider_id} should run for control-flow plans"
            );
        }
        assert!(
            !output.db.call_sites().is_empty(),
            "control-flow plans should derive call-site facts"
        );
        assert!(
            !output.db.refined_call_edges().is_empty(),
            "control-flow plans should keep refined call edges for semantic target matching"
        );
        assert!(
            !output.db.cfg_nodes().is_empty(),
            "control-flow plans should derive CFG nodes for control ordering"
        );
        assert!(
            !output.db.cfg_reachability().is_empty(),
            "control-flow plans should derive CFG reachability rows"
        );
        assert!(
            !output.db.cfg_dominators().is_empty(),
            "control-flow plans should derive CFG dominator rows"
        );
        assert!(
            !output.db.cfg_postdominators().is_empty(),
            "control-flow plans should derive CFG postdominator rows"
        );
        for provider_id in ["polint.data_flow", "polint.evidence"] {
            assert_eq!(
                provider_output(&output, provider_id).cache_stats.recomputes,
                0,
                "{provider_id} should stay off unless a dataflow rule asks for it"
            );
        }
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn cfg_relations_are_derived_only_for_plans_that_read_them() {
        // Reachability, dominance and control dependence rows are read by the
        // control-flow queries and the data-flow evidence, never by call
        // resolution: a calls plan lowers the CFG but derives none of them.
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("app.ts"),
            "export function app(input: string) { if (input) { dangerous(); } cleanup(); }\n",
        )
        .expect("write ts");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let run = |capability: &str| {
            let plan = AnalysisPlan::from_capability_names_for_test(&[capability]);
            AnalysisKernel::run(KernelInput {
                loaded: &loaded,
                cache: &cache,
                config_digest: "config",
                rule_digest: "rules",
                plan: &plan,
                parallel: false,
            })
            .expect("kernel should run")
        };

        let calls = run("calls");
        assert_eq!(
            provider_output(&calls, "polint.cfg").cache_stats.recomputes,
            1
        );
        assert!(
            !calls.db.cfg_functions().is_empty(),
            "a calls plan still lowers the CFG"
        );
        assert!(calls.db.cfg_reachability().is_empty());
        assert!(calls.db.cfg_dominators().is_empty());
        assert!(calls.db.cfg_postdominators().is_empty());
        assert!(calls.db.cfg_control_dependence().is_empty());

        let control_flow = run("control_flow");
        assert!(!control_flow.db.cfg_reachability().is_empty());
        assert!(!control_flow.db.cfg_postdominators().is_empty());
    }

    #[test]
    #[ignore = "synthetic performance smoke; run manually with POLINT_SYNTHETIC_FILES=N"]
    fn control_flow_synthetic_refined_pipeline_benchmark() {
        let file_count = std::env::var("POLINT_SYNTHETIC_FILES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(100);
        if file_count > 500 && std::env::var_os("POLINT_SYNTHETIC_ALLOW_LARGE").is_none() {
            panic!(
                "refusing to generate {file_count} files without POLINT_SYNTHETIC_ALLOW_LARGE=1"
            );
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let src = temp.path().join("src");
        std::fs::create_dir_all(&src).expect("create src");
        for index in 0..file_count {
            std::fs::write(
                src.join(format!("route_{index:05}.ts")),
                format!(
                    "\
export function route_{index}(input: string) {{
  authorize(input);
  fetch(`/api/${{input}}`);
  cleanup(input);
}}

function authorize(value: string) {{ return value.length > 0; }}
function cleanup(value: string) {{ return value.trim(); }}
"
                ),
            )
            .expect("write synthetic file");
        }

        let loaded = load_config(temp.path()).expect("default config loads");
        let cache_enabled = std::env::var_os("POLINT_SYNTHETIC_CACHE").is_some();
        let cache_root = std::env::var_os("POLINT_SYNTHETIC_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| temp.path().join(".polint-cache"));
        let cache = Cache::new(cache_root, cache_enabled);
        let plan = AnalysisPlan::from_capability_names_for_test(&["control_flow"]);

        let started = Instant::now();
        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: true,
        })
        .expect("kernel should run");
        let elapsed = started.elapsed();

        eprintln!(
            "control_flow refined synthetic: files={} functions={} mir_ops={} cfg_nodes={} call_sites={} refined_edges={} domain_obs={} summary_facts={} semantic_nodes={} semantic_edges={} semantic_constraints={} type_facts={} points_to_constraints={} points_to_sets={} solver_edges={} metadata_rows={} elapsed_ms={:.3}",
            output.db.files().len(),
            output.db.functions().len(),
            output.db.mir_operations().len(),
            output.db.cfg_nodes().len(),
            output.db.call_sites().len(),
            output.db.refined_call_edges().len(),
            output.db.abstract_domain_observations().len(),
            output.db.summary_facts().len(),
            output.db.semantic_nodes().len(),
            output.db.semantic_edges().len(),
            output.db.semantic_constraints().len(),
            output.db.type_facts().len(),
            output.db.points_to_constraints().len(),
            output.db.points_to_sets().len(),
            output.db.solver_derived_edges().len(),
            output.db.fact_meta().rows().count(),
            elapsed.as_secs_f64() * 1000.0
        );
        for provider_id in [
            "polint.module_graph",
            "polint.symbol_graph",
            "polint.semantic_mir",
            "polint.go.semantic",
            "polint.cfg",
            "polint.calls",
            "polint.ts.types",
            "polint.identity",
            "polint.abstract_domains",
            "polint.direct_summaries",
            "polint.entrypoints",
            "polint.reachability",
            "polint.extensions",
            "polint.type_value_alias",
            "polint.semantic_graph",
            "polint.solver",
            "polint.refined_calls",
        ] {
            let stats = &provider_output(&output, provider_id).cache_stats;
            if cache_enabled {
                assert!(
                    stats.recomputes == 1 || stats.hits + stats.verified_reuse > 0,
                    "{provider_id} should recompute or reuse cache for synthetic control-flow benchmark"
                );
            } else {
                assert_eq!(
                    stats.recomputes, 1,
                    "{provider_id} should run for synthetic control-flow benchmark"
                );
            }
        }
        for provider_id in ["polint.data_flow", "polint.evidence"] {
            assert_eq!(
                provider_output(&output, provider_id).cache_stats.recomputes,
                0,
                "{provider_id} should stay off for synthetic control-flow benchmark"
            );
        }
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn calls_plan_skips_data_flow_and_evidence() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("app.ts"),
            "export function root() { target(); }\nexport function target() {}\n",
        )
        .expect("write ts");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::from_capability_names_for_test(&["calls"]);

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");

        assert_eq!(
            provider_output(&output, "polint.refined_calls")
                .cache_stats
                .recomputes,
            1
        );
        assert_eq!(
            provider_output(&output, "polint.data_flow")
                .cache_stats
                .recomputes,
            0
        );
        assert_eq!(
            provider_output(&output, "polint.evidence")
                .cache_stats
                .recomputes,
            0
        );
    }

    #[test]
    fn kernel_run_report_synthetic_manifest_consumption_helpers_are_removed_from_kernel() {
        let source = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/analysis_kernel/mod.rs"),
        )
        .expect("read analysis kernel source");

        let forbidden_terms = [
            ["provider", "manifest", "metadata", "token"].join("_"),
            ["provider", "manifest", "metadata", "weight"].join("_"),
            ["provider", "kind", "weight"].join("_"),
            ["language", "scope", "weight"].join("_"),
            ["cache", "policy", "weight"].join("_"),
            ["precision", "ceiling", "weight"].join("_"),
            ["schema", "version", "weight"].join("_"),
            ["", "manifest", "metadata", "token"].join("_"),
        ];

        for forbidden in forbidden_terms {
            assert!(
                !source.contains(&forbidden),
                "synthetic manifest helper remains in kernel: {forbidden}"
            );
        }
    }

    #[test]
    fn framework_entrypoint_internals_do_not_leak_into_public_surfaces_no_leak() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let markers = framework_internal_markers();

        let summary = run_summary_naming_every_reportable_provider();
        let rendered = crate::diagnostics::render(
            crate::diagnostics::OutputFormat::Json,
            &[],
            crate::diagnostics::RenderOpts {
                json: crate::diagnostics::JsonReportMeta {
                    tool_name: "polint",
                    tool_version: "test",
                },
                color: crate::diagnostics::ColorChoice::Never,
                sources: None,
                rule_execution: &summary.rules,
                run_summary: &summary,
            },
        );
        assert_no_framework_markers("polint check --format json", &rendered, &markers);

        let mut public_surfaces = Vec::new();
        collect_files_with_extensions(&crate_root.join("src/sdk"), &["rs"], &mut public_surfaces);
        public_surfaces.extend([
            crate_root.join("src/runner/mod.rs"),
            crate_root.join("src/cli/mod.rs"),
            crate_root.join("src/lib.rs"),
            repo_root.join("README.md"),
            repo_root.join("docs/API-VISIBILITY-PLAN.md"),
        ]);
        collect_files_with_extensions(&repo_root.join("docs/facts"), &["md"], &mut public_surfaces);
        public_surfaces.sort();
        public_surfaces.dedup();

        for source_path in public_surfaces {
            if !source_path.exists() {
                continue;
            }
            let source = std::fs::read_to_string(&source_path)
                .unwrap_or_else(|error| panic!("read {}: {error}", source_path.display()));
            assert_no_framework_markers(&source_path.display().to_string(), &source, &markers);
        }
    }

    #[test]
    fn refined_call_internals_do_not_leak_into_public_surfaces_no_leak() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let markers = refined_call_internal_markers();

        let summary = run_summary_naming_every_reportable_provider();
        let rendered = crate::diagnostics::render(
            crate::diagnostics::OutputFormat::Json,
            &[],
            crate::diagnostics::RenderOpts {
                json: crate::diagnostics::JsonReportMeta {
                    tool_name: "polint",
                    tool_version: "test",
                },
                color: crate::diagnostics::ColorChoice::Never,
                sources: None,
                rule_execution: &summary.rules,
                run_summary: &summary,
            },
        );
        assert_no_refined_call_markers("polint check --format json", &rendered, &markers);

        let mut public_surfaces = Vec::new();
        collect_files_with_extensions(&crate_root.join("src/sdk"), &["rs"], &mut public_surfaces);
        public_surfaces.extend([
            crate_root.join("src/runner/mod.rs"),
            crate_root.join("src/cli/mod.rs"),
            crate_root.join("src/lib.rs"),
            repo_root.join("README.md"),
        ]);
        collect_files_with_extensions(&repo_root.join("docs/facts"), &["md"], &mut public_surfaces);
        public_surfaces.sort();
        public_surfaces.dedup();

        for source_path in public_surfaces {
            if !source_path.exists() {
                continue;
            }
            let source = std::fs::read_to_string(&source_path)
                .unwrap_or_else(|error| panic!("read {}: {error}", source_path.display()));
            assert_no_refined_call_markers(&source_path.display().to_string(), &source, &markers);
        }
    }

    #[test]
    fn data_flow_public_no_leak() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let markers = data_flow_internal_markers();

        let summary = run_summary_naming_every_reportable_provider();
        let rendered = crate::diagnostics::render(
            crate::diagnostics::OutputFormat::Json,
            &[],
            crate::diagnostics::RenderOpts {
                json: crate::diagnostics::JsonReportMeta {
                    tool_name: "polint",
                    tool_version: "test",
                },
                color: crate::diagnostics::ColorChoice::Never,
                sources: None,
                rule_execution: &summary.rules,
                run_summary: &summary,
            },
        );
        assert_no_data_flow_markers("polint check --format json", &rendered, &markers);

        let mut public_surfaces = Vec::new();
        collect_files_with_extensions(&crate_root.join("src/sdk"), &["rs"], &mut public_surfaces);
        public_surfaces.extend([
            crate_root.join("src/runner/mod.rs"),
            crate_root.join("src/cli/mod.rs"),
            crate_root.join("src/lib.rs"),
            repo_root.join("README.md"),
        ]);
        collect_files_with_extensions(&repo_root.join("docs/facts"), &["md"], &mut public_surfaces);
        public_surfaces.sort();
        public_surfaces.dedup();

        for source_path in public_surfaces {
            if !source_path.exists() {
                continue;
            }
            let source = std::fs::read_to_string(&source_path)
                .unwrap_or_else(|error| panic!("read {}: {error}", source_path.display()));
            assert_no_data_flow_markers(&source_path.display().to_string(), &source, &markers);
        }
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn typescript_framework_entrypoints_from_real_source_include_handler_and_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join(".polint.toml"),
            r#"
[workspace]
include = ["*.ts"]
"#,
        )
        .expect("write config");
        std::fs::write(
            temp.path().join("app.ts"),
            r#"
import express from "express";

const app = express();

function getUsers(req, res) {
  res.json([]);
}

function setup() {
  app.get("/api/users/:id", getUsers);
}
"#,
        )
        .expect("write ts");
        let loaded = load_config(temp.path()).expect("config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::from_capability_names_for_test(&["calls"]);

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");

        let entrypoint = output
            .db
            .entrypoint_facts()
            .iter()
            .find(|entrypoint| entrypoint.framework_id == "ts.express")
            .expect("express entrypoint");
        let function_name = output
            .db
            .functions()
            .iter()
            .find(|function| function.id == entrypoint.target_function)
            .map(|function| function.name.as_str());

        assert_eq!(function_name, Some("getUsers"));
        assert_eq!(
            entrypoint.trigger_metadata.path.as_deref(),
            Some("/api/users/:id")
        );
    }

    #[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
    #[test]
    fn framework_entrypoint_eval_fixture_sources_include_go_and_ts_entrypoints() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let fixture_repo =
            repo_root.join("tests/eval-fixtures/framework-entrypoints/mixed-go-ts/repo");
        let loaded = load_config(&fixture_repo).expect("fixture config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::from_capability_names_for_test(&["calls"]);

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");

        let frameworks = output
            .db
            .entrypoint_facts()
            .iter()
            .map(|entrypoint| entrypoint.framework_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let files = output
            .db
            .files()
            .iter()
            .map(|file| (file.relative_path.as_str(), file.language))
            .collect::<Vec<_>>();
        let imports = output
            .db
            .imports()
            .iter()
            .map(|import| (import.path.as_str(), import.language))
            .collect::<Vec<_>>();
        let call_sites = output
            .db
            .call_sites()
            .iter()
            .map(|site| (&site.callee, site.language))
            .collect::<Vec<_>>();

        assert!(
            frameworks.contains("go.net_http"),
            "expected Go net/http entrypoints, got {frameworks:#?}"
        );
        assert!(
            frameworks.contains("ts.express"),
            "expected TS Express entrypoints, got frameworks={frameworks:#?} files={files:#?} imports={imports:#?} calls={call_sites:#?}"
        );
        assert!(
            frameworks.contains("ts.mcp_sdk"),
            "expected TS MCP SDK entrypoints, got {frameworks:#?}"
        );
    }

    fn framework_internal_markers() -> [&'static str; 26] {
        [
            "polint.entrypoints",
            "EntrypointFact",
            "TrustBoundaryFact",
            "FrameworkDispatchEdgeFact",
            "UnresolvedFrameworkFact",
            "EntrypointKind",
            "TrustBoundarySourceKind",
            "DispatchEdgeKind",
            "UnresolvedFrameworkReason",
            "EntrypointPrecision",
            "EntrypointProvenance",
            "EntrypointConfidence",
            "EntrypointStatus",
            "recognizers_go",
            "recognizers_ts",
            "trust_boundaries",
            "dispatch",
            "derive_entrypoints_with_cache_stats",
            "extract_entrypoints",
            "recognize_go_entrypoints",
            "recognize_ts_entrypoints",
            "entrypoints_debug",
            "metadata_debug_json_for_test",
            "EntrypointStore",
            "EntrypointOutput",
            "Entrypoints<'_>",
        ]
    }

    fn refined_call_internal_markers() -> [&'static str; 15] {
        [
            "polint.refined_calls",
            "RefinedCallEdgeFact",
            "RefinedCallTier",
            "RefinedCallReason",
            "RefinedCallGraph",
            "refined_call_edges",
            "direct_plus_framework",
            "points_to_assisted",
            "extension_model",
            "derive_refined_calls_with_cache_stats",
            "RefinedCallStore",
            "RefinedCallOutput",
            "refined_calls.edge",
            "TypeValueFunctionToken",
            "DirectPlusFramework",
        ]
    }

    fn data_flow_internal_markers() -> [&'static str; 7] {
        [
            "polint.data_flow",
            "DataFlowNodeFact",
            "DataFlowEdgeFact",
            "DataFlowModelFact",
            "DataFlowBudgetFact",
            "summary_projected",
            "query path search",
        ]
    }

    /// A run summary naming every provider a successful run may report.
    ///
    /// The leak gates render a report rather than reading source, so they only
    /// cover `summary.providers[]` if the summary actually carries rows. An
    /// empty one silently exempts the whole vocabulary from the check.
    fn run_summary_naming_every_reportable_provider() -> crate::diagnostics::RunSummary {
        crate::diagnostics::RunSummary {
            rules: vec![crate::diagnostics::RuleExecutionRow {
                rule_id: "local/example".to_string(),
                planned: true,
                capabilities_ok: true,
                files_in_scope: 0,
                diagnostics_emitted: 0,
                skipped_reason: None,
                outcome: crate::diagnostics::RULE_OUTCOME_ANALYZED.to_string(),
                blocking_providers: Vec::new(),
                observed_events: 0,
            }],
            providers: outcome::PUBLICLY_NAMED_PROVIDERS
                .iter()
                .map(|provider_id| crate::diagnostics::ProviderOutcomeRow {
                    provider_id: (*provider_id).to_string(),
                    status: "succeeded".to_string(),
                    stage: None,
                    reason: None,
                    elapsed_ms: None,
                    blockers: Vec::new(),
                    cache: None,
                    counts: std::collections::BTreeMap::new(),
                })
                .collect(),
            budgets: Vec::new(),
        }
    }

    fn assert_no_refined_call_markers(label: &str, source: &str, markers: &[&str]) {
        for marker in markers {
            assert!(
                !source.contains(marker),
                "{label} leaked refined-call internal marker `{marker}`"
            );
        }
    }

    fn assert_no_framework_markers(label: &str, source: &str, markers: &[&str]) {
        for marker in markers {
            assert!(
                !source.contains(marker),
                "{label} leaked framework internal marker `{marker}`"
            );
        }
    }

    fn assert_no_data_flow_markers(label: &str, source: &str, markers: &[&str]) {
        for marker in markers {
            assert!(
                !source.contains(marker),
                "{label} leaked data-flow internal marker `{marker}`"
            );
        }
    }

    fn collect_files_with_extensions(root: &Path, extensions: &[&str], files: &mut Vec<PathBuf>) {
        if !root.exists() {
            return;
        }
        for entry in std::fs::read_dir(root)
            .unwrap_or_else(|error| panic!("read {}: {error}", root.display()))
        {
            let entry = entry.expect("read public surface entry");
            let path = entry.path();
            if entry
                .file_type()
                .expect("public surface file type")
                .is_dir()
            {
                collect_files_with_extensions(&path, extensions, files);
            } else if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extensions.contains(&extension))
            {
                files.push(path);
            }
        }
    }

    #[test]
    fn run_propagates_file_loading_errors() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut loaded = load_config(temp.path()).expect("default config loads");
        loaded.config.workspace.include = vec!["[".to_string()];
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::empty();

        let result = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        });
        let Err(error) = result else {
            panic!("kernel should propagate load_analysis_files errors");
        };

        assert!(
            error.to_string().contains("invalid glob"),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn run_surfaces_oversized_source_as_capability_diagnostic() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("ok.go"), "package main\n").expect("write ok.go");
        let oversized = temp.path().join("huge.go");
        {
            let file = std::fs::File::create(&oversized).expect("create huge.go");
            file.set_len(crate::fs::SOURCE_FILE_MAX_BYTES + 1)
                .expect("set oversized length");
        }
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::empty();

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should continue after skipping oversized sources");

        assert_eq!(
            output
                .db
                .files()
                .iter()
                .map(|file| file.relative_path.as_str())
                .collect::<Vec<_>>(),
            ["ok.go"]
        );
        let diagnostic = output
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.rule_id == "polint/capability" && diagnostic.file == "huge.go"
            })
            .expect("kernel must surface the load-time capability diagnostic");
        assert!(
            diagnostic.evidence.iter().any(|evidence| {
                evidence.label == "reason"
                    && evidence.value == "file-exceeds-source-read-size-limit"
            }),
            "missing oversized-source evidence: {diagnostic:?}"
        );
    }

    #[test]
    fn provider_manifests_cover_existing_kernel_providers() {
        let ids = AnalysisKernel::provider_manifests()
            .iter()
            .map(|manifest| manifest.id)
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            [
                "polint.source",
                "polint.go.syntax",
                "polint.ts.syntax",
                "polint.module_graph",
                "polint.symbol_graph",
                "polint.module_topology",
                "polint.semantic_mir",
                "polint.go.semantic",
                "polint.cfg",
                "polint.calls",
                "polint.ts.types",
                "polint.identity",
                "polint.abstract_domains",
                "polint.direct_summaries",
                "polint.entrypoints",
                "polint.reachability",
                "polint.extensions",
                "polint.type_value_alias",
                "polint.semantic_graph",
                "polint.solver",
                "polint.refined_calls",
                "polint.data_flow",
                "polint.evidence",
                "polint.metrics",
            ]
        );
    }

    #[test]
    fn missing_fact_metadata_reports_no_gaps_when_all_current_families_have_metadata() {
        let db = db_with_one_fact_from_every_current_family();

        let report = AnalysisKernel::missing_fact_metadata_for_test(&db);

        assert!(report.is_empty(), "unexpected missing metadata: {report:?}");
    }

    #[test]
    fn missing_fact_metadata_reports_removed_rows_sorted_by_family_and_run_id() {
        let mut db = db_with_one_fact_from_every_current_family();
        db.remove_fact_metadata_for_test(FactRef::new(FactFamily::Reference, 0));
        db.remove_fact_metadata_for_test(FactRef::new(FactFamily::FileMetric, 0));

        let report = AnalysisKernel::missing_fact_metadata_for_test(&db);

        assert_eq!(
            report,
            vec![
                MissingFactMeta {
                    family: FactFamily::FileMetric,
                    run_id: 0,
                },
                MissingFactMeta {
                    family: FactFamily::Reference,
                    run_id: 0,
                },
            ]
        );
    }

    /// A calls run over unchanged Go sources restores the call facts the first
    /// run computed instead of running the deep providers again, and its queries
    /// read the same facts; an edit makes the next run compute again.
    #[cfg(feature = "lang-go")]
    #[test]
    fn a_calls_run_over_unchanged_go_sources_restores_its_call_facts() {
        let temp = tempfile::tempdir().expect("temp directory");
        std::fs::write(
            temp.path().join("go.mod"),
            "module example.com/cached\n\ngo 1.22\n",
        )
        .expect("write go.mod");
        let source = "package main\n\ntype speaker interface{ speak() string }\n\ntype dog struct{}\n\nfunc (dog) speak() string { return \"woof\" }\n\nfunc helper(value int) int { return value + 1 }\n\nfunc main() {\n\tvar s speaker = dog{}\n\t_ = s.speak()\n\tf := helper\n\t_ = f(helper(1))\n}\n";
        std::fs::write(temp.path().join("main.go"), source).expect("write Go source");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::default_for_repo(temp.path(), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["calls"]);
        let run = || {
            AnalysisKernel::run(KernelInput {
                loaded: &loaded,
                cache: &cache,
                config_digest: "config",
                rule_digest: "rules",
                plan: &plan,
                parallel: false,
            })
            .expect("kernel should run")
        };
        // What a query can read, by stable-key text rather than run-local ids.
        let call_facts = |output: &KernelOutput| {
            let interner = output.db.stable_key_interner();
            let key = |id| interner.resolve(id).to_string();
            let mut rows = Vec::new();
            for site in output.db.call_sites() {
                rows.push(format!(
                    "site {} {:?} {:?}",
                    key(site.stable_key),
                    site.callee,
                    site.status
                ));
            }
            for target in output.db.call_targets() {
                rows.push(format!(
                    "target {} {:?} {:?} {:?}",
                    key(target.stable_key),
                    target.target_function,
                    target.synthetic_target,
                    target.algorithm
                ));
            }
            for edge in output.db.refined_call_edges() {
                rows.push(format!(
                    "edge {} {:?} {:?} {:?}",
                    key(edge.stable_key),
                    edge.tier,
                    edge.status,
                    edge.target_function
                ));
            }
            for root in output.db.reachability_roots() {
                rows.push(format!("root {} {:?}", key(root.stable_key), root.id));
            }
            rows
        };

        let computed = run();
        assert!(!computed.db.refined_call_edges().is_empty());
        assert!(
            computed.db.call_targets().iter().any(|target| {
                target.algorithm == crate::analysis_neutral::calls::facts::CallAlgorithm::GoVta
            }),
            "the typed call layer answered the interface call"
        );
        for provider in call_cache::CACHED_PROVIDERS {
            assert_eq!(
                provider_output(&computed, provider).cache_stats.hits,
                0,
                "{provider} computes on the first run"
            );
        }

        let restored = run();
        for provider in call_cache::CACHED_PROVIDERS {
            let row = provider_output(&restored, provider);
            assert_eq!(row.cache_stats.hits, 1, "{provider} is restored");
            assert_eq!(
                row.output_digest,
                provider_output(&computed, provider).output_digest,
                "{provider} reports the digest the computing run recorded"
            );
        }
        assert!(
            restored.db.mir_bodies().is_empty(),
            "nothing below the call facts ran"
        );
        assert_eq!(call_facts(&restored), call_facts(&computed));
        assert_eq!(
            restored.diagnostics, computed.diagnostics,
            "a restored run reports what the computing run reported"
        );

        std::fs::write(
            temp.path().join("main.go"),
            source.replace("value + 1", "value + 2"),
        )
        .expect("edit Go source");
        let edited = run();
        assert_eq!(
            provider_output(&edited, "polint.semantic_mir")
                .cache_stats
                .hits,
            0,
            "an edit computes again"
        );
        assert!(!edited.db.mir_bodies().is_empty());
    }

    /// A data-flow run over unchanged Go sources restores the call facts too,
    /// but still runs the semantic sidecar's provider for the flow programs an
    /// entry does not hold, and its questions get the computing run's answers.
    #[cfg(feature = "lang-go")]
    #[test]
    fn a_go_data_flow_run_over_unchanged_sources_restores_call_facts_and_reads_flow_programs() {
        use crate::sdk::dataflow::{FlowSink, FlowSource, FlowSpec};

        let temp = tempfile::tempdir().expect("temp directory");
        std::fs::write(
            temp.path().join("go.mod"),
            "module example.com/cachedflow\n\ngo 1.22\n",
        )
        .expect("write go.mod");
        std::fs::write(
            temp.path().join("main.go"),
            "package main\n\nfunc source() string { return \"secret\" }\n\nfunc pass(value string) string { return value }\n\nfunc sink(value string) {}\n\nfunc main() {\n\tsink(pass(source()))\n}\n",
        )
        .expect("write Go source");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::default_for_repo(temp.path(), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["dataflow"]);
        let run = || {
            AnalysisKernel::run(KernelInput {
                loaded: &loaded,
                cache: &cache,
                config_digest: "config",
                rule_digest: "rules",
                plan: &plan,
                parallel: false,
            })
            .expect("kernel should run")
        };
        let spec = FlowSpec::new()
            .source(FlowSource::call_result("source"))
            .sink(FlowSink::call_argument("sink", 0));

        let computed = run();
        for provider in call_cache::CACHED_PROVIDERS {
            assert_eq!(
                provider_output(&computed, provider).cache_stats.hits,
                0,
                "{provider} computes on the first run"
            );
        }
        let computed_answer = crate::flow_queries::flows(&computed.db, &spec);
        assert_eq!(
            computed_answer.flows.len(),
            1,
            "the source reaches the sink through pass"
        );

        let restored = run();
        for provider in call_cache::CACHED_PROVIDERS {
            let row = provider_output(&restored, provider);
            if call_cache::restores(provider, &plan) {
                assert_eq!(row.cache_stats.hits, 1, "{provider} is restored");
            }
            assert_eq!(
                row.output_digest,
                provider_output(&computed, provider).output_digest,
                "{provider} reports the computing run's digest"
            );
        }
        assert!(
            restored.db.mir_bodies().is_empty(),
            "nothing below the call facts ran"
        );
        assert_eq!(
            restored
                .db
                .go_flow_program()
                .map(|program| program.digest()),
            computed
                .db
                .go_flow_program()
                .map(|program| program.digest()),
            "the restoring run loads the same flow programs"
        );
        assert_eq!(
            crate::flow_queries::flows(&restored.db, &spec),
            computed_answer
        );
        assert_eq!(restored.diagnostics, computed.diagnostics);
    }

    fn provider_output<'a>(
        output: &'a KernelOutput,
        provider_id: &str,
    ) -> &'a crate::analysis_kernel::incremental::ProviderOutputMeta {
        output
            .run_report
            .provider_outputs
            .iter()
            .find(|row| row.provider_id == provider_id)
            .unwrap_or_else(|| panic!("missing provider output row {provider_id}"))
    }
}
