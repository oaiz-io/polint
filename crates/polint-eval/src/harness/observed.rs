#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::Path;
#[cfg(test)]
use std::time::{Duration, Instant};

#[cfg(test)]
use anyhow::{Context, bail};
#[cfg(test)]
use serde_json::Value;

#[cfg(test)]
use crate::analysis_kernel::incremental::{
    CacheStats, InputComponent, InputComponentStatus, KernelRunReport,
};
#[cfg(test)]
use crate::analysis_kernel::{AnalysisKernel, KernelInput};
#[cfg(test)]
use crate::analysis_plan::AnalysisPlan;
#[cfg(test)]
use crate::cache::Cache;
#[cfg(test)]
use crate::cache::keys::{config_hash, rule_hash};
#[cfg(test)]
use crate::config::load_config;
#[cfg(test)]
use crate::core::AnalysisDb;
#[cfg(test)]
use crate::eval::fixtures::NativeFixture;
#[cfg(test)]
use crate::eval::model::{
    AssertionMode, ExpectedItem, ObservedDiagnostic, ObservedFact, ObservedInvariant, ObservedItem,
    ObservedRuntimeBudget, ObservedStatus,
};
#[cfg(test)]
use crate::module_graph::topology::{ImportToPackageStatus, TopologyPrecision, TopologyStatus};

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ObservedCallGraphEdge {
    pub(crate) site_span: crate::core::Span,
    pub(crate) caller_name: String,
    pub(crate) caller_span: crate::core::Span,
    pub(crate) target_name: String,
    pub(crate) target_span: Option<crate::core::Span>,
    pub(crate) edge_kind: String,
    pub(crate) algorithm: String,
    pub(crate) producer_id: String,
    pub(crate) provenance: String,
    pub(crate) precision: String,
    pub(crate) status: ObservedStatus,
}

#[cfg(test)]
pub(crate) fn graph_edges_from_kernel_output(
    output: &crate::analysis_kernel::KernelOutput,
) -> Vec<ObservedCallGraphEdge> {
    let db = &output.db;
    let functions = db
        .functions()
        .iter()
        .map(|function| (function.id, function))
        .collect::<BTreeMap<_, _>>();
    let sites = db
        .call_sites()
        .iter()
        .map(|site| (site.id, site))
        .collect::<BTreeMap<_, _>>();
    let mut observed = Vec::new();
    let mut refined_keys = std::collections::BTreeSet::new();

    for edge in db.refined_call_edges() {
        refined_keys.insert((
            edge.site,
            edge.target_function,
            edge.synthetic_target.clone(),
        ));
        if let Some(row) = call_graph_edge_from_parts(
            edge.site,
            edge.caller,
            edge.target_function,
            edge.synthetic_target.as_deref(),
            &sites,
            &functions,
            "polint.refined_calls",
            edge.edge_kind,
            edge.algorithm,
            edge.provenance,
            edge.precision,
            edge.status,
        ) {
            observed.push(row);
        }
    }

    for target in db.call_targets() {
        if refined_keys.contains(&(target.site, target.target_function, None)) {
            continue;
        }
        if let Some(row) = call_graph_edge_from_parts(
            target.site,
            target.caller,
            target.target_function,
            None,
            &sites,
            &functions,
            "polint.calls",
            target.edge_kind,
            target.algorithm,
            target.provenance,
            target.precision,
            target.status,
        ) {
            observed.push(row);
        }
    }

    observed.sort_by(|left, right| {
        (
            left.producer_id.as_str(),
            left.site_span.file.0,
            left.site_span.start_line,
            left.site_span.start_col,
            left.caller_name.as_str(),
            left.target_name.as_str(),
        )
            .cmp(&(
                right.producer_id.as_str(),
                right.site_span.file.0,
                right.site_span.start_line,
                right.site_span.start_col,
                right.caller_name.as_str(),
                right.target_name.as_str(),
            ))
    });
    observed.dedup_by(|left, right| {
        left.producer_id == right.producer_id
            && left.site_span == right.site_span
            && left.caller_name == right.caller_name
            && left.target_name == right.target_name
    });
    observed
}

#[cfg(test)]
pub(crate) fn call_graph_unknown_facts_from_kernel_output(
    output: &crate::analysis_kernel::KernelOutput,
) -> Vec<ObservedItem> {
    output
        .db
        .unresolved_calls()
        .iter()
        .map(|unresolved| {
            ObservedItem::Fact(ObservedFact {
                family: "UnresolvedCall".to_string(),
                stable_key: output
                    .db
                    .resolve_stable_key(unresolved.stable_key)
                    .to_string(),
                mode: AssertionMode::Partial,
                producer_id: Some("polint.calls".to_string()),
                provenance: Some(format!("{:?}", unresolved.provenance)),
                precision: Some(format!("{:?}", unresolved.precision)),
                status: Some(call_status_to_observed(unresolved.status)),
                payload: Some(format!("{:?}", unresolved.reason)),
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn adaptation_model_facts_from_kernel_output(
    output: &crate::analysis_kernel::KernelOutput,
) -> Vec<ObservedItem> {
    let interner = output.db.stable_key_interner();
    let accepted = output.db.adaptation_model_facts().iter().map(|fact| {
        ObservedItem::Fact(ObservedFact {
            family: "AdaptationModel".to_string(),
            stable_key: interner.resolve(fact.fact.stable_key).to_string(),
            mode: AssertionMode::Exact,
            producer_id: Some("polint.adaptation.model".to_string()),
            provenance: Some(format!("model_path={}", fact.fact.model_path)),
            precision: Some(fact.fact.confidence.as_str().to_string()),
            status: Some(ObservedStatus::Accepted),
            payload: Some(format!(
                "{}:{}:{}",
                fact.fact.language.as_str(),
                fact.fact.source_pattern,
                fact.fact.target_pattern
            )),
        })
    });
    let rejected = output
        .db
        .rejected_adaptation_model_facts()
        .iter()
        .map(|fact| {
            ObservedItem::Fact(ObservedFact {
                family: "AdaptationModel".to_string(),
                stable_key: interner.resolve(fact.fact.stable_key).to_string(),
                mode: AssertionMode::Exact,
                producer_id: Some("polint.adaptation.model".to_string()),
                provenance: Some(format!("model_path={}", fact.fact.model_path)),
                precision: Some(fact.fact.confidence.as_str().to_string()),
                status: Some(ObservedStatus::Rejected),
                payload: Some(fact.reason.as_str().to_string()),
            })
        });
    accepted.chain(rejected).collect()
}

#[cfg(test)]
#[expect(
    clippy::too_many_arguments,
    reason = "The row is a mechanical projection from call/refined-call facts."
)]
fn call_graph_edge_from_parts(
    site_id: crate::analysis::ids::CallSiteId,
    caller_id: crate::core::FunctionId,
    target_id: Option<crate::core::FunctionId>,
    synthetic_target: Option<&str>,
    sites: &BTreeMap<
        crate::analysis::ids::CallSiteId,
        &crate::analysis::calls::facts::CallSiteFact,
    >,
    functions: &BTreeMap<crate::core::FunctionId, &crate::core::FunctionFact>,
    producer_id: &str,
    edge_kind: crate::analysis::calls::facts::CallEdgeKind,
    algorithm: crate::analysis::calls::facts::CallAlgorithm,
    provenance: crate::analysis::calls::facts::CallProvenance,
    precision: crate::analysis::calls::facts::CallPrecision,
    status: crate::analysis::calls::facts::CallTargetStatus,
) -> Option<ObservedCallGraphEdge> {
    let site = sites.get(&site_id)?;
    let caller = functions.get(&caller_id)?;
    let target = target_id.and_then(|id| functions.get(&id).copied());
    let target_name = target
        .map(|function| function.name.clone())
        .or_else(|| synthetic_target.map(str::to_string))?;

    Some(ObservedCallGraphEdge {
        site_span: site.span.clone(),
        caller_name: caller.name.clone(),
        caller_span: caller.span.clone(),
        target_name,
        target_span: target.map(|function| function.span.clone()),
        edge_kind: format!("{edge_kind:?}"),
        algorithm: format!("{algorithm:?}"),
        producer_id: producer_id.to_string(),
        provenance: format!("{provenance:?}"),
        precision: format!("{precision:?}"),
        status: call_status_to_observed(status),
    })
}

#[cfg(test)]
fn call_status_to_observed(
    status: crate::analysis::calls::facts::CallTargetStatus,
) -> ObservedStatus {
    match status {
        crate::analysis::calls::facts::CallTargetStatus::Resolved => ObservedStatus::Resolved,
        crate::analysis::calls::facts::CallTargetStatus::Ambiguous => ObservedStatus::Ambiguous,
        crate::analysis::calls::facts::CallTargetStatus::Unresolved => ObservedStatus::Unresolved,
        crate::analysis::calls::facts::CallTargetStatus::Unsupported => ObservedStatus::Unsupported,
        crate::analysis::calls::facts::CallTargetStatus::SetupMissing => {
            ObservedStatus::SetupMissing
        }
        crate::analysis::calls::facts::CallTargetStatus::BudgetExceeded => {
            ObservedStatus::BudgetExceeded
        }
        crate::analysis::calls::facts::CallTargetStatus::Rejected => ObservedStatus::Rejected,
    }
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "current evidence delta rows are consumed by eval fixtures that assert extension merge behavior."
)]
pub(crate) fn observed_extension_evidence_delta_rows(
    delta: &crate::analysis::evidence::validate::ExtensionEvidenceDelta,
) -> Vec<ObservedItem> {
    vec![
        evidence_delta_row("accepted", delta.accepted),
        evidence_delta_row("downgraded", delta.downgraded),
        evidence_delta_row("candidate_only", delta.candidate_only),
        evidence_delta_row("rejected", delta.rejected),
        ObservedItem::Fact(ObservedFact {
            family: "evidence_extension_delta".to_string(),
            stable_key: "evidence_extension_delta:reasons".to_string(),
            mode: AssertionMode::Partial,
            producer_id: Some("polint.evidence".to_string()),
            provenance: Some("extension_delta".to_string()),
            precision: Some("debug".to_string()),
            status: Some(ObservedStatus::Present),
            payload: Some(delta.representative_reasons.join(",")),
        }),
        ObservedItem::Fact(ObservedFact {
            family: "evidence_extension_delta".to_string(),
            stable_key: "evidence_extension_delta:native_edge_count".to_string(),
            mode: AssertionMode::Exact,
            producer_id: Some("polint.evidence".to_string()),
            provenance: Some("native".to_string()),
            precision: Some("debug".to_string()),
            status: Some(ObservedStatus::Present),
            payload: Some(delta.native_edge_count.to_string()),
        }),
    ]
}

#[cfg(test)]
fn evidence_delta_row(verdict: &str, count: usize) -> ObservedItem {
    ObservedItem::Fact(ObservedFact {
        family: "evidence_extension_delta".to_string(),
        stable_key: format!("evidence_extension_delta:{verdict}"),
        mode: AssertionMode::Exact,
        producer_id: Some("polint.evidence".to_string()),
        provenance: Some("extension".to_string()),
        precision: Some("debug".to_string()),
        status: Some(ObservedStatus::Present),
        payload: Some(count.to_string()),
    })
}

#[cfg(test)]
mod extension_evidence_delta_tests {
    use super::*;

    #[test]
    fn extension_evidence_deltas_render_deterministic_eval_rows() {
        let rows = observed_extension_evidence_delta_rows(
            &crate::analysis::evidence::validate::ExtensionEvidenceDelta {
                native_edge_count: 3,
                accepted: 1,
                downgraded: 2,
                candidate_only: 1,
                rejected: 4,
                representative_reasons: vec![
                    "InvalidEndpoint".to_string(),
                    "ExactClaimRequiresNativeAnchor".to_string(),
                ],
            },
        );

        let rendered = serde_json::to_string(&rows).expect("rows serialize");

        assert!(rendered.contains("evidence_extension_delta:accepted"));
        assert!(rendered.contains("InvalidEndpoint"));
        assert!(!rendered.contains("/Users/"));
        assert!(!rendered.contains("raw source"));
    }
}

#[cfg(test)]
const SEMANTIC_DEBUG_SECTIONS: &[&str] = &[
    "scopes",
    "imports",
    "exports",
    "aliases",
    "resolutions",
    "generated_symbols",
    "stable_exports",
];

#[cfg(test)]
const SEMANTIC_MIR_DEBUG_SECTIONS: &[&str] = &["bodies", "operations", "places", "unsupported"];

#[cfg(test)]
pub(crate) fn observe_kernel_fixture(fixture: &NativeFixture) -> anyhow::Result<Vec<ObservedItem>> {
    let temp = copy_fixture_repo_for_test(fixture)?;
    observe_kernel_fixture_repo_for_test(fixture, temp.path(), true)
}

#[cfg(test)]
pub(crate) fn copy_fixture_repo_for_test(
    fixture: &NativeFixture,
) -> anyhow::Result<tempfile::TempDir> {
    let temp = tempfile::tempdir().context("create temp fixture repo")?;
    copy_dir_contents(&fixture.repo_dir, temp.path())?;
    Ok(temp)
}

#[cfg(test)]
pub(crate) fn observe_kernel_fixture_repo_for_test(
    fixture: &NativeFixture,
    repo_root: &Path,
    cache_enabled: bool,
) -> anyhow::Result<Vec<ObservedItem>> {
    let plan = AnalysisPlan::full_pipeline_for_test();
    observe_kernel_fixture_repo_with_plan_for_test(fixture, repo_root, cache_enabled, &plan)
}

#[cfg(test)]
pub(crate) fn observe_kernel_fixture_repo_with_plan_for_test(
    fixture: &NativeFixture,
    repo_root: &Path,
    cache_enabled: bool,
    plan: &AnalysisPlan,
) -> anyhow::Result<Vec<ObservedItem>> {
    let started = Instant::now();
    let loaded = load_config(repo_root)?;
    let config_digest = config_hash(&loaded);
    let rule_digest = rule_hash(&[], None, &BTreeMap::new());
    let cache = Cache::default_for_repo(repo_root, cache_enabled);
    let output = AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &cache,
        config_digest: &config_digest,
        rule_digest: &rule_digest,
        plan,
        parallel: true,
    })?;
    let elapsed = started.elapsed();

    eprintln!("RAW_DIAGNOSTICS: {:#?}", output.diagnostics);
    let mut observed = Vec::new();
    observed.extend(observed_diagnostics(&output.diagnostics, repo_root));
    observed.extend(provider_order_invariants());
    observed.extend(metadata_debug_facts(
        &AnalysisKernel::metadata_debug_json_for_output_for_test(&output),
    ));
    observed.extend(topology_facts(&output.db));
    observed.extend(snapshot_invariants(&output.run_report));
    observed.extend(layer_key_invariants(&output.run_report));
    observed.extend(provider_output_invariants(&output.run_report));
    observed.extend(layer_cache_invariants(&output.run_report));
    observed.extend(type_value_alias_facts(&output.db));
    observed.extend(identity_invariants(&output.db));
    observed.extend(extension_facts(&output.db));
    observed.extend(extension_invariants(&output.db));
    if let Some(budget) = &fixture.manifest.budget {
        let peak_rss_bytes = crate::measure::peak_rss_bytes();
        let runtime_ms = saturating_millis(elapsed);
        let runtime_ok = runtime_ms <= budget.max_runtime_ms;
        let rss_ok = budget
            .max_peak_rss_bytes
            .is_none_or(|max| peak_rss_bytes <= max);
        observed.push(ObservedItem::RuntimeBudget(ObservedRuntimeBudget {
            name: runtime_budget_name(&fixture.manifest.expected, &fixture.manifest.case_id),
            budget_passed: runtime_ok && rss_ok,
            observed_runtime_ms: Some(runtime_ms),
            peak_rss_bytes: Some(peak_rss_bytes),
        }));
    }
    observed.sort_by_key(observed_sort_key);

    Ok(observed)
}

/// Runs the analysis kernel on a repo directory and returns the raw
/// `KernelOutput`, so renderer-level tests (e.g. the CRLF byte-identical
/// fixture) can render identity records directly against the live `AnalysisDb`.
#[cfg(test)]
pub(crate) fn run_kernel_for_repo_for_test(
    repo_root: &Path,
) -> anyhow::Result<crate::analysis_kernel::KernelOutput> {
    let plan = AnalysisPlan::full_pipeline_for_test();
    run_kernel_for_repo_with_plan_for_test(repo_root, &plan)
}

/// Runs the analysis kernel on a repo directory with a caller-supplied plan and
/// returns the raw [`crate::analysis_kernel::KernelOutput`] for focused fixture
/// assertions that need private fact access.
#[cfg(test)]
pub(crate) fn run_kernel_for_repo_with_plan_for_test(
    repo_root: &Path,
    plan: &AnalysisPlan,
) -> anyhow::Result<crate::analysis_kernel::KernelOutput> {
    let loaded = load_config(repo_root)?;
    let config_digest = config_hash(&loaded);
    let rule_digest = rule_hash(&[], None, &BTreeMap::new());
    let cache = Cache::default_for_repo(repo_root, true);
    AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &cache,
        config_digest: &config_digest,
        rule_digest: &rule_digest,
        plan,
        parallel: true,
    })
}

#[cfg(test)]
fn copy_dir_contents(source: &Path, destination: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(destination)
        .with_context(|| format!("create fixture copy destination {}", destination.display()))?;
    let mut entries = fs::read_dir(source)
        .with_context(|| format!("read fixture repo {}", source.display()))?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!(
                "fixture repos must not contain symlinks: {}",
                source_path.display()
            );
        }
        if file_type.is_dir() {
            copy_dir_contents(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "copy fixture file {} to {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn observed_diagnostics(
    diagnostics: &[crate::diagnostics::Diagnostic],
    repo_root: &Path,
) -> Vec<ObservedItem> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            ObservedItem::Diagnostic(ObservedDiagnostic {
                rule_id: diagnostic.rule_id.clone(),
                relative_path: normalize_observed_path(&diagnostic.file, repo_root),
                line: Some(diagnostic.range.start_line),
                fingerprint: Some(diagnostic.stable_fingerprint.clone()),
                mode: AssertionMode::Exact,
                producer_id: Some("polint.eval".to_string()),
                provenance: Some("kernel.diagnostics".to_string()),
                precision: Some("exact".to_string()),
                status: Some(ObservedStatus::Present),
            })
        })
        .collect()
}

#[cfg(test)]
fn provider_order_invariants() -> Vec<ObservedItem> {
    AnalysisKernel::provider_manifests()
        .iter()
        .enumerate()
        .map(|(index, manifest)| {
            ObservedItem::Invariant(ObservedInvariant {
                name: format!("provider_order.{index}"),
                value: manifest.id.to_string(),
                mode: AssertionMode::Exact,
                producer_id: Some("polint.eval".to_string()),
                provenance: Some("kernel.provider_manifests".to_string()),
                precision: Some("exact".to_string()),
                status: Some(ObservedStatus::Present),
            })
        })
        .collect()
}

#[cfg(test)]
fn snapshot_invariants(run_report: &KernelRunReport) -> Vec<ObservedItem> {
    let snapshot = &run_report.input_snapshot;

    vec![
        observed_invariant(
            "snapshot.schema_version",
            snapshot.schema_version.as_str(),
            "kernel.run_report.input_snapshot",
        ),
        observed_invariant(
            "snapshot.source_text_digest.present",
            bool_string(
                !snapshot.files.is_empty()
                    && snapshot
                        .files
                        .iter()
                        .all(|file| !file.source_text_digest.value.is_empty()),
            ),
            "kernel.run_report.input_snapshot",
        ),
        observed_invariant(
            "snapshot.config_digest.present",
            bool_string(component_present(&snapshot.config)),
            "kernel.run_report.input_snapshot",
        ),
        observed_invariant(
            "snapshot.go_lifecycle.present",
            bool_string(any_component_present(&snapshot.go_lifecycle.components)),
            "kernel.run_report.input_snapshot",
        ),
        observed_invariant(
            "snapshot.ts_js_lifecycle.present",
            bool_string(any_component_present(&snapshot.ts_js_lifecycle.components)),
            "kernel.run_report.input_snapshot",
        ),
        observed_invariant(
            "snapshot.rule_digest.present",
            bool_string(snapshot.rules.iter().any(component_present)),
            "kernel.run_report.input_snapshot",
        ),
        observed_invariant(
            "snapshot.model_digest.absent",
            bool_string(
                snapshot
                    .models
                    .iter()
                    .any(|component| component.status == InputComponentStatus::Absent),
            ),
            "kernel.run_report.input_snapshot",
        ),
        observed_invariant(
            "snapshot.extension.providers.present",
            bool_string(snapshot.extensions.iter().any(component_present)),
            "kernel.run_report.input_snapshot",
        ),
        observed_invariant(
            "snapshot.tool_invocation.unsupported",
            bool_string(
                snapshot
                    .tool_invocations
                    .iter()
                    .any(|component| component.status == InputComponentStatus::Unsupported),
            ),
            "kernel.run_report.input_snapshot",
        ),
    ]
}

#[cfg(test)]
fn layer_key_invariants(run_report: &KernelRunReport) -> Vec<ObservedItem> {
    let has_ts_output = run_report.provider_outcomes.iter().any(|outcome| {
        outcome.provider_id == "polint.ts.syntax"
            && outcome.status == crate::analysis_kernel::ProviderOutcomeStatus::Succeeded
    });
    vec![observed_invariant(
        "layer_key.polint.ts.syntax.has_config_digest",
        bool_string(has_ts_output && component_present(&run_report.input_snapshot.config)),
        "kernel.run_report.layer_key_inputs",
    )]
}

#[cfg(test)]
fn identity_invariants(db: &crate::core::AnalysisDb) -> Vec<ObservedItem> {
    // Surfaces the deduplicated identity record set so the dedup snapshot fixture
    // can assert the maximum collapse multiplicity deterministically (D-10,
    // D-11). The dedup determinism contract itself is proven by the co-located
    // tests in analysis::identity::dedup.
    let records = db.identity_records();
    let max_multiplicity = records
        .iter()
        .map(|record| record.multiplicity)
        .max()
        .unwrap_or(0);
    let mut invariants = vec![observed_invariant(
        "identity.dedup.multiplicity",
        max_multiplicity.to_string(),
        "kernel.run_report.identity_records",
    )];
    invariants.extend(identity_render_invariants(db));
    invariants.extend(identity_categorized_failure_invariants(db));
    invariants
}

/// Surfaces the closed `IdentityCategory` counter map computed from the live
/// analysis facts (Plan 42-03, D-15) as deterministic observed invariants so the
/// categorized-failures fixture can assert each counter directly.
///
/// Native fixtures carry no benchmark oracle, so `wrong_identity` (which requires
/// oracle span overlap, D-16) never fires here — the empty oracle-span set is
/// passed through `categorized_failures_from_db` and the wrong-identity pass is
/// skipped. The fixture exercises the four naturally-emitted categories; the
/// fifth-category wiring is proven by the `drive_record_category_model_missing`
/// unit test (BLOCKER #4).
#[cfg(test)]
fn identity_categorized_failure_invariants(db: &crate::core::AnalysisDb) -> Vec<ObservedItem> {
    let section =
        crate::eval::metrics::categorized_failures_from_db(db, &std::collections::BTreeSet::new());
    // Exact per-category counts (rehydrated into the report's
    // `categorized_failures` section by `categorized_failures_from_observed`) plus
    // byte-stable `.nonzero` booleans the fixture asserts (exact counts are
    // brittle for the determinism gate; the boolean is order-stable).
    let counters = [
        ("wrong_identity", section.wrong_identity),
        ("unsupported_edge", section.unsupported_edge),
        ("unresolved_edge", section.unresolved_edge),
        ("package_load_limitation", section.package_load_limitation),
        ("model_missing", section.model_missing),
    ];
    let mut invariants = Vec::with_capacity(counters.len() * 2);
    for (name, value) in counters {
        invariants.push(observed_invariant(
            format!("identity.categorized_failures.{name}"),
            value.to_string(),
            "kernel.run_report.identity_records",
        ));
        invariants.push(observed_invariant(
            format!("identity.categorized_failures.{name}.nonzero"),
            bool_string(value > 0),
            "kernel.run_report.identity_records",
        ));
    }
    invariants
}

/// Renders every identity record through the single-source-of-truth renderers
/// (D-05) and surfaces deterministic render invariants:
///
/// - `identity.render.jelly.no_absolute_path` proves the Jelly renderer emits
///   only workspace-relative forward-slash paths (T-42-02-02);
/// - `identity.render.jelly.rendered_count` proves the renderer ran over the
///   live identity record set so the CRLF / oracle-coverage fixtures assert an
///   actually-observed value rather than a phantom.
#[cfg(test)]
fn identity_render_invariants(db: &crate::core::AnalysisDb) -> Vec<ObservedItem> {
    use crate::analysis::identity::render::{go_relstring, jelly_span};

    let files = db
        .files()
        .iter()
        .map(|file| (file.id, file))
        .collect::<BTreeMap<_, _>>();

    let mut rendered_count = 0u32;
    let mut all_workspace_relative = true;
    for record in db.identity_records() {
        // Both renderers participate so the leak gate (Plan 04) and downstream
        // consumers see them exercised end-to-end on real records. The Go
        // RelString output is asserted (not discarded) so a renderer regression
        // that produced an empty name would fail here rather than pass silently.
        let go_rel = go_relstring::render(record);
        assert!(
            !go_rel.is_empty(),
            "go_relstring render must be non-empty for {}",
            db.resolve_stable_key(record.stable_key)
        );
        let Some(source) = files.get(&record.file_id) else {
            continue;
        };
        let rendered = jelly_span::render(record, source);
        rendered_count += 1;
        if rendered.starts_with("/Users/")
            || rendered.starts_with("/home/")
            || rendered.contains(":\\")
        {
            all_workspace_relative = false;
        }
    }

    vec![
        observed_invariant(
            "identity.render.jelly.no_absolute_path",
            bool_string(all_workspace_relative),
            "kernel.run_report.identity_records",
        ),
        observed_invariant(
            "identity.render.jelly.rendered_count.nonzero",
            bool_string(rendered_count > 0),
            "kernel.run_report.identity_records",
        ),
    ]
}

#[cfg(test)]
fn provider_output_invariants(run_report: &KernelRunReport) -> Vec<ObservedItem> {
    let mut invariants = Vec::new();
    let source_present = run_report.provider_outcomes.iter().any(|outcome| {
        outcome.provider_id == "polint.source"
            && outcome.status == crate::analysis_kernel::ProviderOutcomeStatus::Succeeded
    });
    invariants.push(observed_invariant(
        "provider_output.polint.source.present",
        bool_string(source_present),
        "kernel.run_report.provider_outcomes",
    ));

    for outcome in &run_report.provider_outcomes {
        if outcome.provider_id == "polint.abstract_domains" {
            let prefix = "provider_output.polint.abstract_domains";
            invariants.push(observed_invariant(
                format!("{prefix}.present"),
                "true",
                "kernel.run_report.provider_outcomes",
            ));
            invariants.push(observed_invariant(
                format!("{prefix}.schema_version"),
                outcome
                    .output_identity
                    .as_ref()
                    .map(|identity| identity.schema_version.as_str())
                    .unwrap_or(""),
                "kernel.run_report.provider_outcomes",
            ));
            invariants.push(observed_invariant(
                format!("{prefix}.status"),
                outcome.status.label(),
                "kernel.run_report.provider_outcomes",
            ));
            invariants.push(observed_invariant(
                format!("{prefix}.blockers.count"),
                outcome.blockers.len().to_string(),
                "kernel.run_report.provider_outcomes",
            ));
        }
        if outcome.provider_id == "polint.extensions" {
            invariants.push(observed_invariant(
                "provider_output.polint.extensions.present",
                "true",
                "kernel.run_report.provider_outcomes",
            ));
        }
        if outcome.provider_id == "polint.refined_calls" {
            invariants.push(observed_invariant(
                "provider_output.polint.refined_calls.present",
                "true",
                "kernel.run_report.provider_outcomes",
            ));
        }
    }
    for telemetry in &run_report.provider_telemetry {
        if !matches!(
            telemetry.provider_id.as_str(),
            "polint.go.syntax" | "polint.ts.syntax"
        ) {
            continue;
        }
        let prefix = format!("provider_output.{}", telemetry.provider_id);
        invariants.push(observed_invariant(
            format!("{prefix}.cache_stats.recorded"),
            "true",
            "kernel.run_report.provider_telemetry",
        ));
        invariants.extend(cache_stat_invariants(&prefix, &telemetry.cache_stats));
    }

    invariants
}

#[cfg(test)]
fn extension_facts(db: &AnalysisDb) -> Vec<ObservedItem> {
    let interner = db.stable_key_interner();
    let mut facts = db
        .extension_facts()
        .iter()
        .map(|fact| {
            ObservedItem::Fact(ObservedFact {
                family: fact.fact_family.clone(),
                stable_key: interner.resolve(fact.stable_key).to_string(),
                mode: AssertionMode::Exact,
                producer_id: Some(extension_producer_id(&fact.extension_id, &fact.provider_id)),
                provenance: Some("kernel.extension_sink.accepted".to_string()),
                precision: Some(extension_precision_label(fact.precision).to_string()),
                status: Some(ObservedStatus::Accepted),
                payload: Some(format!(
                    "bindings={};confidence={};payload={}",
                    fact.binding_refs.join(","),
                    extension_confidence_label(fact.confidence),
                    fact.payload_labels.join(",")
                )),
            })
        })
        .collect::<Vec<_>>();

    facts.extend(db.rejected_extension_facts().iter().map(|fact| {
        ObservedItem::Fact(ObservedFact {
            family: fact.fact_family.clone(),
            stable_key: interner.resolve(fact.stable_key).to_string(),
            mode: AssertionMode::Exact,
            producer_id: Some(extension_producer_id(&fact.extension_id, &fact.provider_id)),
            provenance: Some("kernel.extension_sink.rejected".to_string()),
            precision: None,
            status: Some(ObservedStatus::Rejected),
            payload: Some(format!(
                "reason={};evidence={}",
                extension_rejection_reason_label(fact.reason),
                fact.evidence.join(",")
            )),
        })
    }));

    facts
}

#[cfg(test)]
fn type_value_alias_facts(db: &AnalysisDb) -> Vec<ObservedItem> {
    let interner = db.stable_key_interner();
    let mut facts = Vec::new();
    facts.extend(db.type_facts().iter().map(|fact| {
        ObservedItem::Fact(ObservedFact {
            family: "Type".to_string(),
            stable_key: interner.resolve(fact.stable_key).to_string(),
            mode: AssertionMode::Exact,
            producer_id: Some("polint.type_value_alias".to_string()),
            provenance: Some(format!("type_value_alias.type.{:?}", fact.provenance)),
            precision: Some(type_precision_label(fact.precision).to_string()),
            status: Some(type_status(fact.status)),
            payload: Some(format!(
                "phase={:?};shape={:?};subject={:?}",
                fact.phase, fact.shape, fact.subject
            )),
        })
    }));
    facts.extend(db.narrowed_type_facts().iter().map(|fact| {
        ObservedItem::Fact(ObservedFact {
            family: "NarrowedType".to_string(),
            stable_key: interner.resolve(fact.stable_key).to_string(),
            mode: AssertionMode::Exact,
            producer_id: Some("polint.type_value_alias".to_string()),
            provenance: Some("type_value_alias.narrowed_type".to_string()),
            precision: Some(type_precision_label(fact.precision).to_string()),
            status: Some(type_status(fact.status)),
            payload: Some(format!(
                "place={:?};type_set={:?};evidence={}",
                fact.place, fact.type_set, fact.evidence
            )),
        })
    }));
    facts.extend(db.value_facts().iter().map(|fact| {
        ObservedItem::Fact(ObservedFact {
            family: "Value".to_string(),
            stable_key: interner.resolve(fact.stable_key).to_string(),
            mode: AssertionMode::Exact,
            producer_id: Some("polint.type_value_alias".to_string()),
            provenance: Some(format!("type_value_alias.value.{:?}", fact.provenance)),
            precision: Some(value_precision_label(fact.precision).to_string()),
            status: Some(value_status(fact.status)),
            payload: Some(format!("subject={:?};kind={:?}", fact.subject, fact.kind)),
        })
    }));
    facts.extend(db.allocation_tokens().iter().map(|fact| {
        ObservedItem::Fact(ObservedFact {
            family: "AllocationToken".to_string(),
            stable_key: interner.resolve(fact.stable_key).to_string(),
            mode: AssertionMode::Exact,
            producer_id: Some("polint.type_value_alias".to_string()),
            provenance: Some(format!("type_value_alias.allocation.{:?}", fact.provenance)),
            precision: Some("setup_aware".to_string()),
            status: Some(ObservedStatus::Present),
            payload: Some(format!(
                "kind={:?};source_place={:?}",
                fact.kind, fact.source_place
            )),
        })
    }));
    facts.extend(db.access_path_facts().iter().map(|fact| {
        ObservedItem::Fact(ObservedFact {
            family: "AccessPath".to_string(),
            stable_key: interner.resolve(fact.stable_key).to_string(),
            mode: AssertionMode::Exact,
            producer_id: Some("polint.type_value_alias".to_string()),
            provenance: Some("type_value_alias.access_path".to_string()),
            precision: Some("setup_aware".to_string()),
            status: Some(access_path_status(fact.status)),
            payload: Some(format!(
                "base={:?};depth={};projections={:?}",
                fact.base, fact.depth, fact.projections
            )),
        })
    }));
    facts.extend(db.points_to_constraints().iter().map(|fact| {
        ObservedItem::Fact(ObservedFact {
            family: "PointsToConstraint".to_string(),
            stable_key: interner.resolve(fact.stable_key).to_string(),
            mode: AssertionMode::Exact,
            producer_id: Some("polint.type_value_alias".to_string()),
            provenance: Some("type_value_alias.points_to_constraint".to_string()),
            precision: Some(points_to_precision_label(fact.precision).to_string()),
            status: Some(points_to_status(fact.status)),
            payload: Some(format!("kind={:?}", fact.kind)),
        })
    }));
    facts.extend(db.points_to_sets().iter().map(|fact| {
        ObservedItem::Fact(ObservedFact {
            family: "PointsToSet".to_string(),
            stable_key: interner.resolve(fact.stable_key).to_string(),
            mode: AssertionMode::Exact,
            producer_id: Some("polint.type_value_alias".to_string()),
            provenance: Some("type_value_alias.points_to_set".to_string()),
            precision: Some(points_to_precision_label(fact.precision).to_string()),
            status: Some(points_to_status(fact.status)),
            payload: Some(format!(
                "variable={:?};objects={:?};budget={:?}",
                fact.variable, fact.objects, fact.budget
            )),
        })
    }));
    facts.extend(db.alias_answers().iter().map(|fact| {
        ObservedItem::Fact(ObservedFact {
            family: "AliasAnswer".to_string(),
            stable_key: interner.resolve(fact.stable_key).to_string(),
            mode: AssertionMode::Exact,
            producer_id: Some("polint.type_value_alias".to_string()),
            provenance: Some("type_value_alias.alias_answer".to_string()),
            precision: Some(alias_precision_label(fact.precision).to_string()),
            status: Some(alias_status(fact.status)),
            payload: Some(format!(
                "status={:?};reason={:?};evidence={}",
                fact.status,
                fact.reason,
                fact.evidence.join(",")
            )),
        })
    }));
    facts.extend(db.alias_answers().iter().map(|fact| {
        ObservedItem::Fact(ObservedFact {
            family: format!("AliasAnswer.{:?}", fact.status),
            stable_key: interner.resolve(fact.stable_key).to_string(),
            mode: AssertionMode::Exact,
            producer_id: Some("polint.type_value_alias".to_string()),
            provenance: Some("type_value_alias.alias_answer.status".to_string()),
            precision: Some(alias_precision_label(fact.precision).to_string()),
            status: Some(alias_status(fact.status)),
            payload: Some(format!(
                "reason={:?};evidence={}",
                fact.reason,
                fact.evidence.join(",")
            )),
        })
    }));
    facts
}

#[cfg(test)]
fn alias_precision_label(
    precision: crate::analysis::aliases::facts::AliasPrecision,
) -> &'static str {
    match precision {
        crate::analysis::aliases::facts::AliasPrecision::ExactLocal => "exact_local",
        crate::analysis::aliases::facts::AliasPrecision::FlowInsensitive => "flow_insensitive",
        crate::analysis::aliases::facts::AliasPrecision::SetupAware => "setup_aware",
        crate::analysis::aliases::facts::AliasPrecision::Conservative => "conservative",
        crate::analysis::aliases::facts::AliasPrecision::Heuristic => "heuristic",
        crate::analysis::aliases::facts::AliasPrecision::Unknown => "unknown",
        crate::analysis::aliases::facts::AliasPrecision::Unsupported => "unsupported",
    }
}

#[cfg(test)]
fn type_precision_label(precision: crate::analysis::types::facts::TypePrecision) -> &'static str {
    match precision {
        crate::analysis::types::facts::TypePrecision::ExactLocal => "exact_local",
        crate::analysis::types::facts::TypePrecision::SetupAware => "setup_aware",
        crate::analysis::types::facts::TypePrecision::Conservative => "conservative",
        crate::analysis::types::facts::TypePrecision::Heuristic => "heuristic",
        crate::analysis::types::facts::TypePrecision::Unknown => "unknown",
        crate::analysis::types::facts::TypePrecision::Unsupported => "unsupported",
    }
}

#[cfg(test)]
fn value_precision_label(
    precision: crate::analysis::values::facts::ValuePrecision,
) -> &'static str {
    match precision {
        crate::analysis::values::facts::ValuePrecision::ExactLocal => "exact_local",
        crate::analysis::values::facts::ValuePrecision::SetupAware => "setup_aware",
        crate::analysis::values::facts::ValuePrecision::Conservative => "conservative",
        crate::analysis::values::facts::ValuePrecision::Heuristic => "heuristic",
        crate::analysis::values::facts::ValuePrecision::Unknown => "unknown",
        crate::analysis::values::facts::ValuePrecision::Unsupported => "unsupported",
    }
}

#[cfg(test)]
fn points_to_precision_label(
    precision: crate::analysis::points_to::facts::PointsToPrecision,
) -> &'static str {
    match precision {
        crate::analysis::points_to::facts::PointsToPrecision::FlowInsensitive => "flow_insensitive",
        crate::analysis::points_to::facts::PointsToPrecision::LocalFlowSensitive => {
            "local_flow_sensitive"
        }
        crate::analysis::points_to::facts::PointsToPrecision::SummaryProjected => {
            "summary_projected"
        }
        crate::analysis::points_to::facts::PointsToPrecision::Heuristic => "heuristic",
        crate::analysis::points_to::facts::PointsToPrecision::Unknown => "unknown",
        crate::analysis::points_to::facts::PointsToPrecision::Unsupported => "unsupported",
    }
}

#[cfg(test)]
fn type_status(status: crate::analysis::types::facts::TypeStatus) -> ObservedStatus {
    match status {
        crate::analysis::types::facts::TypeStatus::Present => ObservedStatus::Present,
        crate::analysis::types::facts::TypeStatus::Unknown => ObservedStatus::Unknown,
        crate::analysis::types::facts::TypeStatus::Unsupported => ObservedStatus::Unsupported,
        crate::analysis::types::facts::TypeStatus::SetupMissing => ObservedStatus::SetupMissing,
        crate::analysis::types::facts::TypeStatus::BudgetExceeded => ObservedStatus::BudgetExceeded,
    }
}

#[cfg(test)]
fn value_status(status: crate::analysis::values::facts::ValueStatus) -> ObservedStatus {
    match status {
        crate::analysis::values::facts::ValueStatus::Present => ObservedStatus::Present,
        crate::analysis::values::facts::ValueStatus::Unknown => ObservedStatus::Unknown,
        crate::analysis::values::facts::ValueStatus::Unsupported => ObservedStatus::Unsupported,
        crate::analysis::values::facts::ValueStatus::SetupMissing => ObservedStatus::SetupMissing,
        crate::analysis::values::facts::ValueStatus::BudgetExceeded => {
            ObservedStatus::BudgetExceeded
        }
    }
}

#[cfg(test)]
fn access_path_status(
    status: crate::analysis::access_paths::facts::AccessPathStatus,
) -> ObservedStatus {
    match status {
        crate::analysis::access_paths::facts::AccessPathStatus::Resolved => {
            ObservedStatus::Resolved
        }
        crate::analysis::access_paths::facts::AccessPathStatus::Partial => ObservedStatus::Partial,
        crate::analysis::access_paths::facts::AccessPathStatus::Unknown => ObservedStatus::Unknown,
        crate::analysis::access_paths::facts::AccessPathStatus::Unsupported => {
            ObservedStatus::Unsupported
        }
        crate::analysis::access_paths::facts::AccessPathStatus::BudgetExceeded => {
            ObservedStatus::BudgetExceeded
        }
    }
}

#[cfg(test)]
fn points_to_status(status: crate::analysis::points_to::facts::PointsToStatus) -> ObservedStatus {
    match status {
        crate::analysis::points_to::facts::PointsToStatus::Present => ObservedStatus::Present,
        crate::analysis::points_to::facts::PointsToStatus::Unknown => ObservedStatus::Unknown,
        crate::analysis::points_to::facts::PointsToStatus::Unsupported => {
            ObservedStatus::Unsupported
        }
        crate::analysis::points_to::facts::PointsToStatus::SetupMissing => {
            ObservedStatus::SetupMissing
        }
        crate::analysis::points_to::facts::PointsToStatus::BudgetExceeded => {
            ObservedStatus::BudgetExceeded
        }
    }
}

#[cfg(test)]
fn alias_status(status: crate::analysis::aliases::facts::AliasStatus) -> ObservedStatus {
    match status {
        crate::analysis::aliases::facts::AliasStatus::NoAlias
        | crate::analysis::aliases::facts::AliasStatus::MustAlias => ObservedStatus::Present,
        crate::analysis::aliases::facts::AliasStatus::MayAlias
        | crate::analysis::aliases::facts::AliasStatus::PartialAlias => ObservedStatus::Ambiguous,
        crate::analysis::aliases::facts::AliasStatus::Unknown => ObservedStatus::Unknown,
    }
}

#[cfg(test)]
fn extension_invariants(db: &AnalysisDb) -> Vec<ObservedItem> {
    let changed_facts = db.extension_facts().len() + db.rejected_extension_facts().len();
    let rejected = db.rejected_extension_facts().len();
    let real_sink_active = !db.extension_facts().is_empty()
        && db
            .extension_activations()
            .iter()
            .any(|row| row.provider_id.is_some());

    vec![
        observed_invariant(
            "extension.default_vs_extension_delta",
            format!("changed_facts={changed_facts},rejected={rejected}"),
            "kernel.extension_sink",
        ),
        observed_invariant(
            "extension.real_sink_active",
            bool_string(real_sink_active),
            "kernel.extension_sink",
        ),
    ]
}

#[cfg(test)]
fn extension_producer_id(extension_id: &str, provider_id: &str) -> String {
    format!("polint.extension.{extension_id}.{provider_id}")
}

#[cfg(test)]
fn extension_precision_label(
    precision: crate::analysis::extensions::sinks::ExtensionFactPrecision,
) -> &'static str {
    match precision {
        crate::analysis::extensions::sinks::ExtensionFactPrecision::Exact => "exact",
        crate::analysis::extensions::sinks::ExtensionFactPrecision::SetupAware => "setup_aware",
        crate::analysis::extensions::sinks::ExtensionFactPrecision::Heuristic => "heuristic",
        crate::analysis::extensions::sinks::ExtensionFactPrecision::GeneratedUnvalidated => {
            "generated_unvalidated"
        }
    }
}

#[cfg(test)]
fn extension_confidence_label(
    confidence: crate::analysis::extensions::sinks::ExtensionFactConfidence,
) -> &'static str {
    match confidence {
        crate::analysis::extensions::sinks::ExtensionFactConfidence::High => "high",
        crate::analysis::extensions::sinks::ExtensionFactConfidence::Medium => "medium",
        crate::analysis::extensions::sinks::ExtensionFactConfidence::Low => "low",
    }
}

#[cfg(test)]
fn extension_rejection_reason_label(
    reason: crate::analysis::extensions::validate::ExtensionRejectionReason,
) -> &'static str {
    match reason {
        crate::analysis::extensions::validate::ExtensionRejectionReason::UndeclaredOutput => {
            "undeclared_output"
        }
        crate::analysis::extensions::validate::ExtensionRejectionReason::MissingBinding => {
            "missing_binding"
        }
        crate::analysis::extensions::validate::ExtensionRejectionReason::InvalidSpan => {
            "invalid_span"
        }
        crate::analysis::extensions::validate::ExtensionRejectionReason::MissingPrecision => {
            "missing_precision"
        }
        crate::analysis::extensions::validate::ExtensionRejectionReason::MissingProvenance => {
            "missing_provenance"
        }
        crate::analysis::extensions::validate::ExtensionRejectionReason::SyntheticIdMissingEvidence => {
            "synthetic_id_missing_evidence"
        }
        crate::analysis::extensions::validate::ExtensionRejectionReason::DuplicateStableKey => {
            "duplicate_stable_key"
        }
        crate::analysis::extensions::validate::ExtensionRejectionReason::NativeConflict => {
            "native_conflict"
        }
        crate::analysis::extensions::validate::ExtensionRejectionReason::FrameworkPrecisionCeiling => {
            "framework_precision_ceiling"
        }
        crate::analysis::extensions::validate::ExtensionRejectionReason::TypeValueAliasPrecisionCeiling => {
            "type_value_alias_precision_ceiling"
        }
        crate::analysis::extensions::validate::ExtensionRejectionReason::MalformedPayload => {
            "malformed_payload"
        }
    }
}

#[cfg(test)]
fn layer_cache_invariants(run_report: &KernelRunReport) -> Vec<ObservedItem> {
    let mut invariants = Vec::new();
    for telemetry in &run_report.provider_telemetry {
        if !is_layer_cache_provider(&telemetry.provider_id) {
            continue;
        }
        let prefix = format!("layer_cache.provider.{}", telemetry.provider_id);
        invariants.extend(layer_cache_stat_invariants(
            &prefix,
            &telemetry.cache_stats,
            "kernel.run_report.layer_cache.provider_telemetry",
        ));
    }
    invariants.extend(layer_cache_stat_invariants(
        "layer_cache.aggregate",
        &run_report.cache_stats,
        "kernel.run_report.layer_cache.aggregate",
    ));
    invariants
}

#[cfg(test)]
fn is_layer_cache_provider(provider_id: &str) -> bool {
    matches!(
        provider_id,
        "polint.go.syntax"
            | "polint.ts.syntax"
            | "polint.module_graph"
            | "polint.symbol_graph"
            | "polint.module_topology"
            | "polint.cfg"
            | "polint.metrics"
    )
}

#[cfg(test)]
fn layer_cache_stat_invariants(
    prefix: &str,
    stats: &CacheStats,
    provenance: &'static str,
) -> Vec<ObservedItem> {
    [
        ("hits", stats.hits),
        ("misses", stats.misses),
        ("recomputes", stats.recomputes),
        ("writes", stats.writes),
        ("bypasses_disabled", stats.bypasses_disabled),
        ("invalid_evicted_reads", stats.invalid_evicted_reads),
        ("verified_reuse", stats.verified_reuse),
    ]
    .into_iter()
    .map(|(counter, value)| {
        observed_invariant(format!("{prefix}.{counter}"), value.to_string(), provenance)
    })
    .collect()
}

#[cfg(test)]
fn cache_stat_invariants(prefix: &str, stats: &CacheStats) -> Vec<ObservedItem> {
    [
        ("cache_stats.hits", stats.hits),
        ("cache_stats.misses", stats.misses),
        ("cache_stats.recomputes", stats.recomputes),
        ("cache_stats.writes", stats.writes),
        ("cache_stats.bypasses_disabled", stats.bypasses_disabled),
        (
            "cache_stats.invalid_evicted_reads",
            stats.invalid_evicted_reads,
        ),
        ("cache_stats.verified_reuse", stats.verified_reuse),
        ("cache_stats.quarantines", stats.quarantines),
    ]
    .into_iter()
    .map(|(counter, value)| {
        observed_invariant(
            format!("{prefix}.{counter}"),
            value.to_string(),
            "kernel.run_report.provider_telemetry",
        )
    })
    .collect()
}

#[cfg(test)]
fn any_component_present(components: &[InputComponent]) -> bool {
    components.iter().any(component_present)
}

#[cfg(test)]
fn component_present(component: &InputComponent) -> bool {
    component.status == InputComponentStatus::Present && !component.digest.value.is_empty()
}

#[cfg(test)]
fn observed_invariant(
    name: impl Into<String>,
    value: impl Into<String>,
    provenance: &'static str,
) -> ObservedItem {
    ObservedItem::Invariant(ObservedInvariant {
        name: name.into(),
        value: value.into(),
        mode: AssertionMode::Exact,
        producer_id: Some("polint.eval".to_string()),
        provenance: Some(provenance.to_string()),
        precision: Some("exact".to_string()),
        status: Some(ObservedStatus::Present),
    })
}

#[cfg(test)]
fn bool_string(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

#[cfg(test)]
pub(crate) fn metadata_debug_facts_for_test(debug_json: &Value) -> Vec<ObservedItem> {
    metadata_debug_facts(debug_json)
}

#[cfg(test)]
pub(crate) fn semantic_mir_facts_for_test(debug_json: &Value) -> Vec<ObservedItem> {
    semantic_mir_facts(debug_json)
}

#[cfg(test)]
pub(crate) fn cfg_facts_for_test(debug_json: &Value) -> Vec<ObservedItem> {
    cfg_facts(debug_json)
}

#[cfg(test)]
pub(crate) fn call_facts_for_test(debug_json: &Value) -> Vec<ObservedItem> {
    call_facts(debug_json)
}

#[cfg(test)]
pub(crate) fn refined_call_facts_for_test(debug_json: &Value) -> Vec<ObservedItem> {
    refined_call_facts(debug_json)
}

#[cfg(test)]
pub(crate) fn abstract_domain_facts_for_test(debug_json: &Value) -> Vec<ObservedItem> {
    abstract_domain_facts(debug_json)
}

#[cfg(test)]
pub(crate) fn direct_summary_facts_for_test(debug_json: &Value) -> Vec<ObservedItem> {
    direct_summary_facts(debug_json)
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "Data-flow observed-row helper is exercised by focused eval row tests and fixtures."
)]
pub(crate) fn data_flow_facts_for_test(debug_json: &Value) -> Vec<ObservedItem> {
    data_flow_facts(debug_json)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn entrypoint_facts_for_test(debug_json: &Value) -> Vec<ObservedItem> {
    entrypoint_facts(debug_json)
}

#[cfg(test)]
pub(crate) fn topology_facts_for_test(db: &AnalysisDb) -> Vec<ObservedItem> {
    topology_facts(db)
}

#[cfg(test)]
fn topology_facts(db: &AnalysisDb) -> Vec<ObservedItem> {
    let mut facts = Vec::new();

    facts.extend(db.workspace_roots().iter().map(|row| {
        topology_fact(
            "WorkspaceRoot",
            db.resolve_stable_key(row.stable_key).as_ref(),
            row.producer_id,
            row.precision,
            topology_status(row.status),
            format!(
                "kind:{:?};root:{};manifest:{}",
                row.kind,
                row.root_path,
                row.manifest_path.as_deref().unwrap_or("")
            ),
        )
    }));
    facts.extend(db.topology_packages().iter().map(|row| {
        topology_fact(
            "TopologyPackage",
            db.resolve_stable_key(row.stable_key).as_ref(),
            row.producer_id,
            row.precision,
            topology_status(row.status),
            format!(
                "kind:{:?};name:{};path:{};root:{:?}",
                row.kind, row.name, row.path, row.workspace_root
            ),
        )
    }));
    facts.extend(db.source_sets().iter().map(|row| {
        topology_fact(
            "SourceSet",
            db.resolve_stable_key(row.stable_key).as_ref(),
            row.producer_id,
            row.precision,
            topology_status(row.status),
            format!(
                "kind:{:?};path:{};package:{:?};root:{:?}",
                row.kind, row.path, row.package, row.root
            ),
        )
    }));
    facts.extend(db.dependency_requirements().iter().map(|row| {
        topology_fact(
            "DependencyRequirement",
            db.resolve_stable_key(row.stable_key).as_ref(),
            row.producer_id,
            row.precision,
            topology_status(row.status),
            format!(
                "target:{};kind:{:?};from:{:?};to:{:?};manifest:{}",
                row.target_name,
                row.kind,
                row.from_package,
                row.target_package,
                row.manifest_path.as_deref().unwrap_or("")
            ),
        )
    }));
    facts.extend(db.resolved_dependency_edges().iter().map(|row| {
        topology_fact(
            "ResolvedDependencyEdge",
            db.resolve_stable_key(row.stable_key).as_ref(),
            row.producer_id,
            row.precision,
            topology_status(row.status),
            format!(
                "package:{};kind:{:?};requirement:{:?};from:{:?};to:{:?}",
                row.package_name, row.kind, row.requirement, row.from_package, row.to_package
            ),
        )
    }));
    facts.extend(db.import_to_package_edges().iter().map(|row| {
        topology_fact(
            "ImportToPackage",
            db.resolve_stable_key(row.stable_key).as_ref(),
            row.producer_id,
            row.precision,
            import_to_package_status(row.status),
            format!(
                "import:{};context:{:?};from:{};to:{};source_set:{};semantic:{}",
                row.import_path,
                row.context,
                row.from_package_stable_key
                    .map(|key| db.resolve_stable_key(key))
                    .unwrap_or_default(),
                row.to_package_stable_key
                    .map(|key| db.resolve_stable_key(key))
                    .unwrap_or_default(),
                row.source_set_stable_key
                    .map(|key| db.resolve_stable_key(key))
                    .unwrap_or_default(),
                row.semantic_import_stable_key
                    .map(|key| db.resolve_stable_key(key))
                    .unwrap_or_default()
            ),
        )
    }));
    facts.extend(db.repo_topology_overlays().iter().map(|row| {
        topology_fact(
            "RepoTopologyOverlay",
            db.resolve_stable_key(row.stable_key).as_ref(),
            row.producer_id,
            row.precision,
            topology_status(row.status),
            format!(
                "kind:{:?};label:{};path:{};root:{:?};package:{:?};source_set:{:?}",
                row.kind,
                row.label,
                row.path.as_deref().unwrap_or(""),
                row.root,
                row.package,
                row.source_set
            ),
        )
    }));

    facts
}

#[cfg(test)]
fn topology_fact(
    family: &str,
    stable_key: &str,
    producer_id: &'static str,
    precision: TopologyPrecision,
    status: ObservedStatus,
    payload: String,
) -> ObservedItem {
    ObservedItem::Fact(ObservedFact {
        family: family.to_string(),
        stable_key: stable_key.to_string(),
        mode: AssertionMode::Exact,
        producer_id: Some(producer_id.to_string()),
        provenance: Some(producer_id.to_string()),
        precision: Some(topology_precision_label(precision).to_string()),
        status: Some(status),
        payload: Some(payload),
    })
}

#[cfg(test)]
fn topology_precision_label(precision: TopologyPrecision) -> &'static str {
    match precision {
        TopologyPrecision::ExactStatic => "exact_static",
        TopologyPrecision::ExactLockfile => "exact_lockfile",
        TopologyPrecision::Heuristic => "heuristic",
        TopologyPrecision::Unknown => "unknown",
        TopologyPrecision::Unsupported => "unsupported",
    }
}

#[cfg(test)]
mod public_boundary_no_leak {
    use std::fs;
    use std::path::{Path, PathBuf};

    const INTERNAL_MARKERS: &[&str] = &[
        "demand_query",
        "DemandQuery",
        "scc_schedule",
        "SccSchedule",
        "SccClosure",
        "scc_closure",
        "quarantine",
        "QuarantineStore",
        "QuarantineEntry",
        "quarantine_policy",
        "backdating",
        "backdated",
        "interprocedural_closure",
        "SccClosureResult",
        "SccClosureConfig",
        "DemandQueryEngine",
        "DemandQueryTrace",
        "DemandQueryTraceEntry",
        "DemandQueryResult",
        "is_quarantined",
        "reinstate",
    ];

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root")
            .to_path_buf()
    }

    fn assert_no_markers(label: &str, source: &str) {
        for marker in INTERNAL_MARKERS {
            assert!(
                !source.contains(marker),
                "{label} leaked internal implementation marker `{marker}`"
            );
        }
    }

    fn collect_files(root: &Path, files: &mut Vec<PathBuf>) {
        if !root.exists() {
            return;
        }
        for entry in fs::read_dir(root).expect("read public surface directory") {
            let entry = entry.expect("public surface entry");
            let path = entry.path();
            if entry
                .file_type()
                .expect("public surface file type")
                .is_dir()
            {
                collect_files(&path, files);
            } else {
                files.push(path);
            }
        }
    }

    #[test]
    fn public_check_json_does_not_expose_phase33_internal_markers() {
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
                rule_execution: &[],
                run_summary: crate::diagnostics::EMPTY_RUN_SUMMARY,
            },
        );

        assert_no_markers("polint check --format json", &rendered);
    }

    #[test]
    fn public_sdk_runner_cli_docs_and_readme_do_not_expose_phase33_internal_markers() {
        let root = repo_root();
        let crate_root = root.join("crates/polint");
        let mut sources = Vec::new();

        for dir in [crate_root.join("src/sdk"), root.join("docs")] {
            collect_files(&dir, &mut sources);
        }
        sources.extend([
            crate_root.join("src/runner/mod.rs"),
            crate_root.join("src/cli/mod.rs"),
            crate_root.join("src/lib.rs"),
            root.join("README.md"),
        ]);

        for source in sources {
            let text = fs::read_to_string(&source)
                .unwrap_or_else(|err| panic!("read {}: {err}", source.display()));
            assert_no_markers(&source.display().to_string(), &text);
        }
    }
}

#[cfg(test)]
fn topology_status(status: TopologyStatus) -> ObservedStatus {
    match status {
        TopologyStatus::Present => ObservedStatus::Present,
        TopologyStatus::Resolved => ObservedStatus::Resolved,
        TopologyStatus::Ambiguous => ObservedStatus::Ambiguous,
        TopologyStatus::Unresolved => ObservedStatus::Unresolved,
        TopologyStatus::Unknown => ObservedStatus::Unknown,
        TopologyStatus::SetupMissing => ObservedStatus::SetupMissing,
        TopologyStatus::MissingLockfile => ObservedStatus::MissingLockfile,
        TopologyStatus::Generated => ObservedStatus::Generated,
        TopologyStatus::External => ObservedStatus::External,
        TopologyStatus::Unsupported => ObservedStatus::Unsupported,
    }
}

#[cfg(test)]
fn import_to_package_status(status: ImportToPackageStatus) -> ObservedStatus {
    match status {
        ImportToPackageStatus::Resolved => ObservedStatus::Resolved,
        ImportToPackageStatus::External => ObservedStatus::External,
        ImportToPackageStatus::Unresolved => ObservedStatus::Unresolved,
        ImportToPackageStatus::SetupMissing => ObservedStatus::SetupMissing,
        ImportToPackageStatus::Unsupported => ObservedStatus::Unsupported,
        ImportToPackageStatus::Dynamic => ObservedStatus::Dynamic,
        ImportToPackageStatus::Ambiguous => ObservedStatus::Ambiguous,
        ImportToPackageStatus::Undeclared => ObservedStatus::Undeclared,
        ImportToPackageStatus::OutsideWorkspace => ObservedStatus::OutsideWorkspace,
    }
}

#[cfg(test)]
fn metadata_debug_facts(debug_json: &Value) -> Vec<ObservedItem> {
    let mut facts = Vec::new();
    for section in ["files", "imports", "symbols", "references"] {
        let Some(rows) = debug_json.get(section).and_then(Value::as_array) else {
            continue;
        };
        for row in rows {
            if let Some(fact) = metadata_debug_fact(row) {
                facts.push(ObservedItem::Fact(fact));
            }
        }
    }
    if let Some(semantic) = debug_json.get("semantic").and_then(Value::as_object) {
        for &section in SEMANTIC_DEBUG_SECTIONS {
            let Some(rows) = semantic.get(section).and_then(Value::as_array) else {
                continue;
            };
            for row in rows {
                if let Some(fact) = metadata_debug_fact(row) {
                    facts.push(ObservedItem::Fact(fact));
                }
            }
        }
    }
    facts.extend(semantic_mir_facts(debug_json));
    facts.extend(cfg_facts(debug_json));
    facts.extend(call_facts(debug_json));
    facts.extend(refined_call_facts(debug_json));
    facts.extend(abstract_domain_facts(debug_json));
    facts.extend(direct_summary_facts(debug_json));
    facts.extend(data_flow_facts(debug_json));
    facts.extend(evidence_facts(debug_json));
    facts.extend(entrypoint_facts(debug_json));
    facts.extend(scc_and_demand_query_invariants(debug_json));
    facts
}

#[cfg(test)]
fn data_flow_facts(debug_json: &Value) -> Vec<ObservedItem> {
    let mut facts = Vec::new();
    let Some(data_flow) = debug_json.get("data_flow").and_then(Value::as_object) else {
        return facts;
    };
    for section in ["nodes", "edges", "models", "budgets"] {
        let Some(rows) = data_flow.get(section).and_then(Value::as_array) else {
            continue;
        };
        for row in rows {
            if let Some(fact) = data_flow_fact(row, section) {
                facts.push(ObservedItem::Fact(fact));
            }
        }
    }
    facts.extend(data_flow_count_invariants(data_flow));
    facts.extend(data_flow_taxonomy_facts(data_flow));
    facts
}

#[cfg(test)]
fn data_flow_taxonomy_facts(data_flow: &serde_json::Map<String, Value>) -> Vec<ObservedItem> {
    let mut facts = Vec::new();

    if has_data_flow_edge(
        data_flow,
        Some("LocalMir"),
        Some("LocalBinding"),
        Some("Present"),
    ) && has_data_flow_edge(
        data_flow,
        Some("LocalMir"),
        Some("ReturnValue"),
        Some("Present"),
    ) {
        facts.push(data_flow_taxonomy_fact(
            "DataFlowEdge",
            "data-flow:local:param-local-return",
            "syntax",
            ObservedStatus::Present,
        ));
    }
    if has_data_flow_edge(
        data_flow,
        Some("DirectCall"),
        Some("UnknownFlow"),
        Some("Unknown"),
    ) {
        facts.push(data_flow_taxonomy_fact(
            "DataFlowEdge",
            "data-flow:direct-call:unknown-unresolved",
            "unknown",
            ObservedStatus::Unknown,
        ));
    }
    if has_data_flow_edge(
        data_flow,
        Some("DirectCall"),
        Some("CallArgumentToParameter"),
        Some("Present"),
    ) && has_data_flow_edge(
        data_flow,
        Some("DirectCall"),
        Some("CallReturnToUse"),
        Some("Present"),
    ) {
        facts.push(data_flow_taxonomy_fact(
            "DataFlowEdge",
            "data-flow:direct-call:resolved-projection",
            "heuristic",
            ObservedStatus::Present,
        ));
    }
    if has_data_flow_edge(
        data_flow,
        Some("SummaryProjection"),
        Some("SummaryTito"),
        None,
    ) {
        facts.push(data_flow_taxonomy_fact(
            "DataFlowEdge",
            "data-flow:summary-projected:tito",
            "syntax",
            ObservedStatus::Present,
        ));
    }
    if has_data_flow_edge(
        data_flow,
        Some("SummaryProjection"),
        Some("UnknownFlow"),
        Some("Unknown"),
    ) {
        facts.push(data_flow_taxonomy_fact(
            "DataFlowEdge",
            "data-flow:unknown:missing-summary",
            "unknown",
            ObservedStatus::Unknown,
        ));
    }
    if has_data_flow_edge(data_flow, None, Some("HavocFlow"), Some("Unsupported")) {
        facts.push(data_flow_taxonomy_fact(
            "DataFlowEdge",
            "data-flow:havoc:unsupported-write",
            "unknown",
            ObservedStatus::Unsupported,
        ));
    }
    if has_data_flow_budget(data_flow, "PathDepth", "BudgetExceeded") {
        facts.push(data_flow_taxonomy_fact(
            "DataFlowBudget",
            "data-flow:budget:path-depth",
            "unknown",
            ObservedStatus::BudgetExceeded,
        ));
    }

    if has_data_flow_model(data_flow, "Source") {
        facts.push(data_flow_taxonomy_fact(
            "DataFlowModel",
            "data-flow:model:source:trust-boundary",
            "setup_aware",
            ObservedStatus::Present,
        ));
    }
    if has_data_flow_model(data_flow, "Sink") {
        facts.push(data_flow_taxonomy_fact(
            "DataFlowModel",
            "data-flow:model:sink:repo",
            "heuristic",
            ObservedStatus::Present,
        ));
    }
    if has_data_flow_model(data_flow, "Sanitizer") {
        facts.push(data_flow_taxonomy_fact(
            "DataFlowModel",
            "data-flow:model:sanitizer:repo",
            "heuristic",
            ObservedStatus::Present,
        ));
    }
    if has_data_flow_model(data_flow, "Barrier") {
        facts.push(data_flow_taxonomy_fact(
            "DataFlowModel",
            "data-flow:model:barrier:repo",
            "heuristic",
            ObservedStatus::Present,
        ));
    }
    if has_data_flow_model(data_flow, "Tito") {
        facts.push(data_flow_taxonomy_fact(
            "DataFlowModel",
            "data-flow:model:extension:additional-step",
            "heuristic",
            ObservedStatus::Present,
        ));
    }

    if has_data_flow_model(data_flow, "Sanitizer")
        || has_data_flow_model(data_flow, "Barrier")
        || has_data_flow_edge(data_flow, None, Some("Sanitizer"), None)
        || has_data_flow_edge(data_flow, None, Some("Barrier"), None)
    {
        facts.push(observed_invariant(
            "data_flow.false_positive_trap.sanitized_or_barriered",
            "true",
            "kernel.metadata_debug_json.data_flow.taxonomy",
        ));
    }

    facts
}

#[cfg(test)]
fn data_flow_taxonomy_fact(
    family: &str,
    stable_key: &str,
    precision: &str,
    status: ObservedStatus,
) -> ObservedItem {
    ObservedItem::Fact(ObservedFact {
        family: family.to_string(),
        stable_key: stable_key.to_string(),
        mode: AssertionMode::Exact,
        producer_id: Some("polint.data_flow".to_string()),
        provenance: Some("kernel.metadata_debug_json.data_flow.taxonomy".to_string()),
        precision: Some(precision.to_string()),
        status: Some(status),
        payload: Some("source=kernel_debug_taxonomy".to_string()),
    })
}

#[cfg(test)]
fn has_data_flow_edge(
    data_flow: &serde_json::Map<String, Value>,
    algorithm: Option<&str>,
    kind: Option<&str>,
    status: Option<&str>,
) -> bool {
    data_flow
        .get("edges")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                optional_json_str_matches(row.get("algorithm"), algorithm)
                    && optional_json_str_matches(row.get("kind"), kind)
                    && optional_json_str_matches(row.get("status"), status)
            })
        })
}

#[cfg(test)]
fn has_data_flow_model(data_flow: &serde_json::Map<String, Value>, kind: &str) -> bool {
    data_flow
        .get("models")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get("kind")
                    .and_then(Value::as_str)
                    .is_some_and(|label| label == kind)
            })
        })
}

#[cfg(test)]
fn has_data_flow_budget(
    data_flow: &serde_json::Map<String, Value>,
    reason: &str,
    status: &str,
) -> bool {
    data_flow
        .get("budgets")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get("reason")
                    .and_then(Value::as_str)
                    .is_some_and(|label| label == reason)
                    && row
                        .get("status")
                        .and_then(Value::as_str)
                        .is_some_and(|label| label == status)
            })
        })
}

#[cfg(test)]
fn optional_json_str_matches(value: Option<&Value>, expected: Option<&str>) -> bool {
    expected.is_none_or(|expected| value.and_then(Value::as_str) == Some(expected))
}

#[cfg(test)]
fn evidence_facts(debug_json: &Value) -> Vec<ObservedItem> {
    let mut facts = Vec::new();
    let Some(evidence) = debug_json.get("evidence").and_then(Value::as_object) else {
        return facts;
    };
    let Some(counts) = evidence.get("counts").and_then(Value::as_object) else {
        return facts;
    };
    for family in [
        ("nodes", "EvidenceNode"),
        ("edges", "EvidenceEdge"),
        ("bundles", "EvidenceBundle"),
        ("paths", "EvidencePath"),
        ("slices", "EvidenceSlice"),
        ("unknowns", "EvidenceUnknown"),
        ("omitted_regions", "EvidenceOmittedRegion"),
    ] {
        let Some(count) = counts.get(family.0).and_then(Value::as_u64) else {
            continue;
        };
        facts.push(ObservedItem::Fact(ObservedFact {
            family: family.1.to_string(),
            stable_key: format!("evidence.debug.counts.{}", family.0),
            mode: AssertionMode::Exact,
            producer_id: Some("polint.evidence".to_string()),
            provenance: Some("kernel.metadata_debug_json.evidence.counts".to_string()),
            precision: Some("exact".to_string()),
            status: Some(ObservedStatus::Present),
            payload: Some(format!("count={count}")),
        }));
        if count > 0 {
            facts.push(observed_invariant(
                format!("evidence.debug.counts.{}.nonzero", family.0),
                "true",
                "kernel.metadata_debug_json.evidence.counts",
            ));
        }
    }
    facts
}

#[cfg(test)]
fn data_flow_fact(row: &Value, section: &str) -> Option<ObservedFact> {
    let status = row
        .get("status")
        .and_then(Value::as_str)
        .and_then(data_flow_status_from_label)?;
    let precision = row
        .get("precision")
        .and_then(Value::as_str)
        .map(data_flow_precision_label)
        .or_else(|| Some("unknown".to_string()));
    Some(ObservedFact {
        family: row.get("family")?.as_str()?.to_string(),
        stable_key: row.get("stable_key")?.as_str()?.to_string(),
        mode: AssertionMode::Exact,
        producer_id: row
            .get("producer_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        provenance: Some("kernel.metadata_debug_json.data_flow".to_string()),
        precision,
        status: Some(status),
        payload: Some(data_flow_payload(row, section)),
    })
}

#[cfg(test)]
fn data_flow_payload(row: &Value, section: &str) -> String {
    let mut parts = vec![format!("section={section}")];
    push_str_fragment(&mut parts, "kind", row.get("kind"));
    push_str_fragment(&mut parts, "algorithm", row.get("algorithm"));
    push_str_fragment(&mut parts, "status", row.get("status"));
    push_str_fragment(&mut parts, "precision", row.get("precision"));
    push_str_fragment(&mut parts, "provenance", row.get("provenance"));
    push_str_fragment(&mut parts, "reason", row.get("reason"));
    parts.join(";")
}

#[cfg(test)]
fn data_flow_count_invariants(data_flow: &serde_json::Map<String, Value>) -> Vec<ObservedItem> {
    let Some(counts) = data_flow.get("counts").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut invariants = Vec::new();
    for field in [
        "nodes",
        "edges",
        "models",
        "budgets",
        "local_edges",
        "direct_call_edges",
        "summary_projected_edges",
        "unknown_or_havoc_edges",
        "budget_rows",
    ] {
        if let Some(value) = counts.get(field).and_then(Value::as_u64) {
            invariants.push(observed_invariant(
                format!("data_flow.counts.{field}"),
                value.to_string(),
                "kernel.metadata_debug_json.data_flow.counts",
            ));
            if value > 0 {
                invariants.push(observed_invariant(
                    format!("data_flow.counts.{field}.nonzero"),
                    "true",
                    "kernel.metadata_debug_json.data_flow.counts",
                ));
            }
        }
    }
    for group in ["by_kind", "by_status"] {
        let Some(values) = counts.get(group).and_then(Value::as_object) else {
            continue;
        };
        for (label, count) in values {
            if count.as_u64().is_some_and(|count| count > 0) {
                invariants.push(observed_invariant(
                    format!("data_flow.counts.{group}.{label}.nonzero"),
                    "true",
                    "kernel.metadata_debug_json.data_flow.counts",
                ));
            }
        }
    }
    invariants
}

#[cfg(test)]
fn data_flow_status_from_label(label: &str) -> Option<ObservedStatus> {
    match label {
        "Present" | "present" => Some(ObservedStatus::Present),
        "Unknown" | "unknown" => Some(ObservedStatus::Unknown),
        "Unsupported" | "unsupported" => Some(ObservedStatus::Unsupported),
        "SetupMissing" | "setup_missing" => Some(ObservedStatus::SetupMissing),
        "BudgetExceeded" | "budget_exceeded" => Some(ObservedStatus::BudgetExceeded),
        "Rejected" | "rejected" => Some(ObservedStatus::Rejected),
        _ => None,
    }
}

#[cfg(test)]
fn data_flow_precision_label(label: &str) -> String {
    match label {
        "Exact" => "exact",
        "SetupAware" => "setup_aware",
        "Syntax" => "syntax",
        "Conservative" => "conservative",
        "Heuristic" => "heuristic",
        "Unknown" => "unknown",
        other => other,
    }
    .to_string()
}

#[cfg(test)]
fn scc_and_demand_query_invariants(debug_json: &Value) -> Vec<ObservedItem> {
    let mut invariants = Vec::new();

    if let Some(scc_schedule) = debug_json.get("scc_schedule").and_then(Value::as_object) {
        if scc_schedule
            .get("total_sccs")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0)
        {
            invariants.push(observed_invariant(
                "scc_schedule.total_sccs.nonzero",
                "true",
                "kernel.metadata_debug_json.scc_schedule",
            ));
        }
        if scc_schedule
            .get("recursive_scc_count")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0)
        {
            invariants.push(observed_invariant(
                "scc_schedule.recursive_scc_count.nonzero",
                "true",
                "kernel.metadata_debug_json.scc_schedule",
            ));
        }
        if scc_schedule
            .get("max_scc_size")
            .and_then(Value::as_u64)
            .is_some_and(|size| size >= 2)
        {
            invariants.push(observed_invariant(
                "scc_schedule.max_scc_size.at_least_2",
                "true",
                "kernel.metadata_debug_json.scc_schedule",
            ));
        }
        if scc_schedule
            .get("iteration_counts")
            .and_then(Value::as_array)
            .is_some_and(|rows| !rows.is_empty())
        {
            invariants.push(observed_invariant(
                "scc_schedule.iteration_counts.nonzero",
                "true",
                "kernel.metadata_debug_json.scc_schedule",
            ));
        }
    }

    if let Some(demand_queries) = debug_json.get("demand_queries").and_then(Value::as_object) {
        if demand_queries
            .get("total_queries")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0)
        {
            invariants.push(observed_invariant(
                "demand_queries.total_queries.nonzero",
                "true",
                "kernel.metadata_debug_json.demand_queries",
            ));
        }
        if demand_queries
            .get("cache_hits")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0)
        {
            invariants.push(observed_invariant(
                "demand_queries.cache_hits.nonzero",
                "true",
                "kernel.metadata_debug_json.demand_queries",
            ));
        }
        if demand_queries
            .get("cache_misses")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0)
        {
            invariants.push(observed_invariant(
                "demand_queries.cache_misses.nonzero",
                "true",
                "kernel.metadata_debug_json.demand_queries",
            ));
        }
    }

    invariants
}

#[cfg(test)]
fn semantic_mir_facts(debug_json: &Value) -> Vec<ObservedItem> {
    let mut facts = Vec::new();
    let Some(mir) = debug_json.get("mir").and_then(Value::as_object) else {
        return facts;
    };
    for &section in SEMANTIC_MIR_DEBUG_SECTIONS {
        let Some(rows) = mir.get(section).and_then(Value::as_array) else {
            continue;
        };
        for row in rows {
            if let Some(fact) = semantic_mir_fact(row) {
                facts.push(ObservedItem::Fact(fact));
            }
        }
    }
    facts
}

#[cfg(test)]
fn semantic_mir_fact(row: &Value) -> Option<ObservedFact> {
    Some(ObservedFact {
        family: row.get("family")?.as_str()?.to_string(),
        stable_key: row.get("stable_key")?.as_str()?.to_string(),
        mode: AssertionMode::Exact,
        producer_id: row
            .get("producer_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        provenance: Some("kernel.metadata_debug_json.mir".to_string()),
        precision: row
            .get("precision")
            .and_then(Value::as_str)
            .map(str::to_string),
        status: Some(observed_status_from_metadata(row)),
        payload: semantic_mir_payload(row),
    })
}

#[cfg(test)]
fn semantic_mir_payload(row: &Value) -> Option<String> {
    let mut parts = Vec::new();
    push_str_fragment(&mut parts, "path", row.get("path"));
    push_span_fragment(&mut parts, row.get("span"));
    push_u64_fragment(&mut parts, "owner", row.get("owner_function"));
    push_str_fragment(&mut parts, "kind", row.get("operation_kind"));
    push_str_fragment(&mut parts, "root", row.get("place_root"));
    push_list_fragment(&mut parts, "projections", row.get("place_projections"));
    push_str_fragment(&mut parts, "construct", row.get("unsupported_construct"));
    push_str_fragment(
        &mut parts,
        "conservative_action",
        row.get("conservative_action"),
    );

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(";"))
    }
}

#[cfg(test)]
fn cfg_facts(debug_json: &Value) -> Vec<ObservedItem> {
    let mut facts = Vec::new();
    let Some(cfg) = debug_json.get("cfg").and_then(Value::as_object) else {
        return facts;
    };
    for section in [
        "functions",
        "nodes",
        "blocks",
        "edges",
        "reachability",
        "dominators",
        "postdominators",
        "control_dependence",
        "unsupported",
    ] {
        let Some(rows) = cfg.get(section).and_then(Value::as_array) else {
            continue;
        };
        for row in rows {
            if let Some(fact) = cfg_fact(row) {
                facts.push(ObservedItem::Fact(fact));
            }
        }
    }
    facts
}

#[cfg(test)]
fn cfg_fact(row: &Value) -> Option<ObservedFact> {
    Some(ObservedFact {
        family: row.get("family")?.as_str()?.to_string(),
        stable_key: row.get("stable_key")?.as_str()?.to_string(),
        mode: AssertionMode::Exact,
        producer_id: row
            .get("producer_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        provenance: Some("kernel.metadata_debug_json.cfg".to_string()),
        precision: row
            .get("precision")
            .and_then(Value::as_str)
            .map(str::to_string),
        status: Some(observed_status_from_metadata(row)),
        payload: cfg_payload(row),
    })
}

#[cfg(test)]
fn cfg_payload(row: &Value) -> Option<String> {
    let mut parts = Vec::new();
    push_str_fragment(&mut parts, "path", row.get("path"));
    push_span_fragment(&mut parts, row.get("span"));
    push_u64_fragment(&mut parts, "function", row.get("function"));
    push_str_fragment(&mut parts, "view", row.get("view"));
    push_str_fragment(&mut parts, "kind", row.get("node_kind"));
    push_str_fragment(&mut parts, "kind", row.get("block_kind"));
    push_str_fragment(&mut parts, "construct", row.get("unsupported_construct"));
    push_str_fragment(&mut parts, "payload", row.get("payload"));
    push_str_fragment(&mut parts, "status", row.get("status"));
    push_str_fragment(&mut parts, "precision", row.get("precision"));

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(";"))
    }
}

#[cfg(test)]
fn call_facts(debug_json: &Value) -> Vec<ObservedItem> {
    let mut facts = Vec::new();
    let Some(calls) = debug_json.get("calls").and_then(Value::as_object) else {
        return facts;
    };
    for section in ["sites", "targets", "unresolved"] {
        let Some(rows) = calls.get(section).and_then(Value::as_array) else {
            continue;
        };
        for row in rows {
            if let Some(fact) = call_fact(row) {
                facts.push(ObservedItem::Fact(fact));
            }
        }
    }
    facts.extend(call_count_invariants(calls));
    facts.extend(call_index_count_invariants(calls));
    facts
}

#[cfg(test)]
fn call_count_invariants(calls: &serde_json::Map<String, Value>) -> Vec<ObservedItem> {
    let Some(counts) = calls.get("counts").and_then(Value::as_object) else {
        return Vec::new();
    };

    let mut invariants = Vec::new();
    for group in [
        "by_language",
        "by_call_kind",
        "by_algorithm",
        "by_status",
        "by_unresolved_reason",
        "by_provider",
    ] {
        let Some(values) = counts.get(group).and_then(Value::as_object) else {
            continue;
        };
        for (label, count) in values {
            if count.as_u64().is_some_and(|count| count > 0) {
                invariants.push(observed_invariant(
                    format!("direct_calls.counts.{group}.{label}.nonzero"),
                    "true",
                    "kernel.metadata_debug_json.calls.counts",
                ));
            }
        }
    }
    invariants
}

#[cfg(test)]
fn call_index_count_invariants(calls: &serde_json::Map<String, Value>) -> Vec<ObservedItem> {
    let Some(index_counts) = calls.get("index_counts").and_then(Value::as_object) else {
        return Vec::new();
    };

    index_counts
        .iter()
        .filter(|(_, count)| count.as_u64().is_some_and(|count| count > 0))
        .map(|(index, _)| {
            observed_invariant(
                format!("direct_calls.index_counts.{index}.nonzero"),
                "true",
                "kernel.metadata_debug_json.calls.index_counts",
            )
        })
        .collect()
}

#[cfg(test)]
fn call_fact(row: &Value) -> Option<ObservedFact> {
    Some(ObservedFact {
        family: row.get("family")?.as_str()?.to_string(),
        stable_key: row.get("stable_key")?.as_str()?.to_string(),
        mode: AssertionMode::Exact,
        producer_id: row
            .get("producer_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        provenance: Some("kernel.metadata_debug_json.calls".to_string()),
        precision: row
            .get("precision")
            .and_then(Value::as_str)
            .map(call_precision_label),
        status: Some(call_status_from_row(row)),
        payload: call_payload(row),
    })
}

#[cfg(test)]
fn call_payload(row: &Value) -> Option<String> {
    let mut parts = Vec::new();
    push_str_fragment(&mut parts, "path", row.get("path"));
    push_span_fragment(&mut parts, row.get("span"));
    push_str_fragment(&mut parts, "kind", row.get("kind"));
    push_str_fragment(&mut parts, "callee", row.get("callee"));
    push_str_fragment(&mut parts, "edge_kind", row.get("edge_kind"));
    push_str_fragment(&mut parts, "algorithm", row.get("algorithm"));
    push_str_fragment(&mut parts, "status", row.get("status"));
    push_str_fragment(&mut parts, "reason", row.get("reason"));
    push_str_fragment(&mut parts, "provider", row.get("producer_id"));
    push_str_fragment(&mut parts, "provenance", row.get("provenance"));
    push_str_fragment(&mut parts, "site", row.get("site_stable_key"));
    push_str_fragment(&mut parts, "caller", row.get("caller_stable_key"));
    push_str_fragment(
        &mut parts,
        "target_function",
        row.get("target_function_stable_key"),
    );
    push_str_fragment(
        &mut parts,
        "target_symbol",
        row.get("target_symbol_stable_key"),
    );

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(";"))
    }
}

#[cfg(test)]
fn call_status_from_row(row: &Value) -> ObservedStatus {
    row.get("status")
        .and_then(Value::as_str)
        .and_then(call_status_label)
        .or_else(|| {
            row.get("precision")
                .and_then(Value::as_str)
                .and_then(call_status_label)
        })
        .unwrap_or(ObservedStatus::Present)
}

#[cfg(test)]
fn call_status_label(label: &str) -> Option<ObservedStatus> {
    match label {
        "Resolved" | "resolved" => Some(ObservedStatus::Resolved),
        "Ambiguous" | "ambiguous" => Some(ObservedStatus::Ambiguous),
        "Unresolved" | "unresolved" => Some(ObservedStatus::Unresolved),
        "Unsupported" | "unsupported" => Some(ObservedStatus::Unsupported),
        "SetupMissing" | "setup_missing" => Some(ObservedStatus::SetupMissing),
        "Rejected" | "rejected" => Some(ObservedStatus::Rejected),
        "BudgetExceeded" | "budget_exceeded" => Some(ObservedStatus::BudgetExceeded),
        "Unknown" | "unknown" => Some(ObservedStatus::Unknown),
        _ => None,
    }
}

#[cfg(test)]
fn refined_call_facts(debug_json: &Value) -> Vec<ObservedItem> {
    let mut facts = Vec::new();
    let Some(refined_calls) = debug_json.get("refined_calls").and_then(Value::as_object) else {
        return facts;
    };
    if let Some(rows) = refined_calls.get("edges").and_then(Value::as_array) {
        for row in rows {
            if let Some(fact) = refined_call_fact(row) {
                facts.push(ObservedItem::Fact(fact));
            }
        }
    }
    facts.extend(refined_call_count_invariants(refined_calls));
    facts.extend(refined_call_delta_invariants(refined_calls));
    facts
}

#[cfg(test)]
fn refined_call_fact(row: &Value) -> Option<ObservedFact> {
    Some(ObservedFact {
        family: row.get("family")?.as_str()?.to_string(),
        stable_key: row.get("stable_key")?.as_str()?.to_string(),
        mode: AssertionMode::Exact,
        producer_id: row
            .get("producer_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        provenance: row
            .get("provenance")
            .and_then(Value::as_str)
            .map(str::to_string),
        precision: row
            .get("precision")
            .and_then(Value::as_str)
            .map(call_precision_label),
        status: Some(call_status_from_row(row)),
        payload: refined_call_payload(row),
    })
}

#[cfg(test)]
fn refined_call_payload(row: &Value) -> Option<String> {
    let mut parts = Vec::new();
    push_str_fragment(&mut parts, "language", row.get("language"));
    push_str_fragment(&mut parts, "tier", row.get("tier"));
    push_str_fragment(&mut parts, "edge_kind", row.get("edge_kind"));
    push_str_fragment(&mut parts, "algorithm", row.get("algorithm"));
    push_str_fragment(&mut parts, "status", row.get("status"));
    push_str_fragment(&mut parts, "precision", row.get("precision"));
    push_str_fragment(&mut parts, "provenance", row.get("provenance"));
    push_str_fragment(&mut parts, "reason", row.get("reason"));
    push_str_fragment(&mut parts, "site", row.get("site_stable_key"));
    push_str_fragment(&mut parts, "base", row.get("base_target_stable_key"));
    push_str_fragment(&mut parts, "caller", row.get("caller_stable_key"));
    push_str_fragment(
        &mut parts,
        "target_function",
        row.get("target_function_stable_key"),
    );
    push_str_fragment(
        &mut parts,
        "target_symbol",
        row.get("target_symbol_stable_key"),
    );
    push_str_fragment(&mut parts, "synthetic_target", row.get("synthetic_target"));

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(";"))
    }
}

#[cfg(test)]
fn refined_call_count_invariants(
    refined_calls: &serde_json::Map<String, Value>,
) -> Vec<ObservedItem> {
    let Some(counts) = refined_calls.get("counts").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut invariants = Vec::new();
    for group in [
        "by_language",
        "by_algorithm",
        "by_tier",
        "by_status",
        "by_precision",
        "by_provenance",
        "by_reason",
    ] {
        let Some(values) = counts.get(group).and_then(Value::as_object) else {
            continue;
        };
        for (label, count) in values {
            if count.as_u64().is_some_and(|count| count > 0) {
                invariants.push(observed_invariant(
                    format!("refined_calls.counts.{group}.{label}.nonzero"),
                    "true",
                    "kernel.metadata_debug_json.refined_calls.counts",
                ));
            }
        }
    }
    invariants
}

#[cfg(test)]
fn refined_call_delta_invariants(
    refined_calls: &serde_json::Map<String, Value>,
) -> Vec<ObservedItem> {
    let Some(deltas) = refined_calls.get("deltas").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut invariants = Vec::new();
    for field in [
        "direct_edges",
        "refined_edges",
        "changed_edges",
        "extension_model_edges",
        "unresolved_refined_edges",
        "budget_exceeded_refined_edges",
    ] {
        let Some(value) = deltas.get(field).and_then(Value::as_u64) else {
            continue;
        };
        invariants.push(observed_invariant(
            format!("refined_calls.deltas.{field}"),
            value.to_string(),
            "kernel.metadata_debug_json.refined_calls.deltas",
        ));
        if value > 0 {
            invariants.push(observed_invariant(
                format!("refined_calls.deltas.{field}.nonzero"),
                "true",
                "kernel.metadata_debug_json.refined_calls.deltas",
            ));
        }
    }
    invariants
}

#[cfg(test)]
fn call_precision_label(label: &str) -> String {
    match label {
        "Exact" | "exact" => "exact",
        "SetupAware" | "setup_aware" => "setup_aware",
        "Conservative" | "conservative" => "conservative",
        "Heuristic" | "heuristic" => "heuristic",
        "Ambiguous" | "ambiguous" => "ambiguous",
        "Unknown" | "unknown" => "unknown",
        "Unsupported" | "unsupported" => "unsupported",
        _ => label,
    }
    .to_string()
}

#[cfg(test)]
fn abstract_domain_facts(debug_json: &Value) -> Vec<ObservedItem> {
    let mut facts = Vec::new();
    let Some(domains) = debug_json
        .get("abstract_domains")
        .and_then(Value::as_object)
    else {
        return facts;
    };
    for section in ["observations", "events"] {
        let Some(rows) = domains.get(section).and_then(Value::as_array) else {
            continue;
        };
        for row in rows {
            if let Some(fact) = abstract_domain_fact(row) {
                facts.push(ObservedItem::Fact(fact));
            }
        }
    }
    facts.extend(abstract_domain_count_invariants(domains));
    facts.extend(abstract_domain_index_count_invariants(domains));
    facts
}

#[cfg(test)]
fn abstract_domain_fact(row: &Value) -> Option<ObservedFact> {
    Some(ObservedFact {
        family: row.get("family")?.as_str()?.to_string(),
        stable_key: row.get("stable_key")?.as_str()?.to_string(),
        mode: AssertionMode::Exact,
        producer_id: row
            .get("producer_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        provenance: Some("kernel.metadata_debug_json.abstract_domains".to_string()),
        precision: row
            .get("precision")
            .and_then(Value::as_str)
            .map(abstract_domain_precision_label),
        status: Some(abstract_domain_status_from_row(row)),
        payload: abstract_domain_payload(row),
    })
}

#[cfg(test)]
fn abstract_domain_payload(row: &Value) -> Option<String> {
    let mut parts = Vec::new();
    push_str_fragment(&mut parts, "path", row.get("path"));
    push_span_fragment(&mut parts, row.get("span"));
    push_str_fragment(&mut parts, "body", row.get("body_stable_key"));
    push_str_fragment(&mut parts, "block", row.get("block_stable_key"));
    push_str_fragment(&mut parts, "operation", row.get("operation_stable_key"));
    push_str_fragment(&mut parts, "place", row.get("place_stable_key"));
    push_str_fragment(&mut parts, "slot", row.get("slot"));
    push_str_fragment(&mut parts, "location", row.get("location"));
    push_str_fragment(&mut parts, "value", row.get("value"));
    push_str_fragment(&mut parts, "status", row.get("status"));
    push_str_fragment(&mut parts, "precision", row.get("precision"));
    push_str_fragment(&mut parts, "reason", row.get("reason"));

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(";"))
    }
}

#[cfg(test)]
fn abstract_domain_count_invariants(domains: &serde_json::Map<String, Value>) -> Vec<ObservedItem> {
    let Some(counts) = domains.get("counts").and_then(Value::as_object) else {
        return Vec::new();
    };

    let mut invariants = Vec::new();
    for group in [
        "by_slot",
        "by_status",
        "by_precision",
        "by_reason",
        "by_provider",
    ] {
        let Some(values) = counts.get(group).and_then(Value::as_object) else {
            continue;
        };
        for (label, count) in values {
            if count.as_u64().is_some_and(|count| count > 0) {
                invariants.push(observed_invariant(
                    format!("abstract_domains.counts.{group}.{label}.nonzero"),
                    "true",
                    "kernel.metadata_debug_json.abstract_domains.counts",
                ));
            }
        }
    }
    invariants
}

#[cfg(test)]
fn abstract_domain_index_count_invariants(
    domains: &serde_json::Map<String, Value>,
) -> Vec<ObservedItem> {
    let Some(index_counts) = domains.get("index_counts").and_then(Value::as_object) else {
        return Vec::new();
    };

    index_counts
        .iter()
        .filter(|(_, count)| count.as_u64().is_some_and(|count| count > 0))
        .map(|(index, _)| {
            observed_invariant(
                format!("abstract_domains.index_counts.{index}.nonzero"),
                "true",
                "kernel.metadata_debug_json.abstract_domains.index_counts",
            )
        })
        .collect()
}

#[cfg(test)]
fn direct_summary_facts(debug_json: &Value) -> Vec<ObservedItem> {
    let mut facts = Vec::new();
    let Some(summaries) = debug_json.get("summaries").and_then(Value::as_object) else {
        return facts;
    };

    if let Some(rows) = summaries.get("summaries").and_then(Value::as_array) {
        for row in rows {
            if let Some(fact) = direct_summary_fact(row) {
                facts.push(ObservedItem::Fact(fact));
            }
        }
    }
    if let Some(rows) = summaries.get("events").and_then(Value::as_array) {
        for row in rows {
            if let Some(fact) = direct_summary_event_fact(row) {
                facts.push(ObservedItem::Fact(fact));
            }
        }
    }
    facts.extend(direct_summary_count_invariants(summaries));
    facts.extend(direct_summary_domain_count_invariants(summaries));
    facts
}

#[cfg(test)]
fn direct_summary_fact(row: &Value) -> Option<ObservedFact> {
    let domain = row.get("domain")?.as_str()?;
    let family = direct_summary_domain_to_family(domain)?;
    let status_str = row.get("status")?.as_str()?;
    let precision_str = row.get("precision")?.as_str()?;
    let provenance_str = row.get("provenance").and_then(Value::as_str).unwrap_or("");
    let payload_digest = row
        .get("payload_digest")
        .and_then(Value::as_str)
        .unwrap_or("");
    let payload_prefix = if payload_digest.len() > 8 {
        &payload_digest[..8]
    } else {
        payload_digest
    };

    Some(ObservedFact {
        family: family.to_string(),
        stable_key: row.get("stable_key")?.as_str()?.to_string(),
        mode: AssertionMode::Exact,
        producer_id: Some("polint.direct_summaries".to_string()),
        provenance: Some("kernel.metadata_debug_json.summaries".to_string()),
        precision: Some(direct_summary_precision_label(precision_str).to_string()),
        status: Some(direct_summary_status_from_label(status_str)),
        payload: Some(format!(
            "domain={domain};status={status_str};precision={precision_str};provenance={provenance_str};payload_digest_prefix={payload_prefix}"
        )),
    })
}

#[cfg(test)]
fn direct_summary_event_fact(row: &Value) -> Option<ObservedFact> {
    let domain = row.get("domain")?.as_str()?;
    let status_str = row.get("status")?.as_str()?;
    let precision_str = row.get("precision")?.as_str()?;
    let event_kind = row.get("event_kind").and_then(Value::as_str).unwrap_or("");
    let reason = row.get("reason").and_then(Value::as_str).unwrap_or("");

    Some(ObservedFact {
        family: "summary_event".to_string(),
        stable_key: row.get("stable_key")?.as_str()?.to_string(),
        mode: AssertionMode::Exact,
        producer_id: Some("polint.direct_summaries".to_string()),
        provenance: Some("kernel.metadata_debug_json.summaries".to_string()),
        precision: Some(direct_summary_precision_label(precision_str).to_string()),
        status: Some(direct_summary_status_from_label(status_str)),
        payload: Some(format!(
            "domain={domain};event_kind={event_kind};reason={reason};status={status_str};precision={precision_str}"
        )),
    })
}

#[cfg(test)]
fn direct_summary_domain_to_family(domain: &str) -> Option<&'static str> {
    match domain {
        "control_effects" => Some("summary_control"),
        "call_effects" => Some("summary_call"),
        "memory_effects" => Some("summary_memory"),
        "data_flow_tito" => Some("summary_tito"),
        _ => None,
    }
}

#[cfg(test)]
fn direct_summary_status_from_label(label: &str) -> ObservedStatus {
    match label {
        "present" => ObservedStatus::Present,
        "unknown" => ObservedStatus::Unknown,
        "unsupported" => ObservedStatus::Unsupported,
        "setup_missing" => ObservedStatus::SetupMissing,
        "budget_exceeded" => ObservedStatus::BudgetExceeded,
        _ => ObservedStatus::Present,
    }
}

#[cfg(test)]
fn direct_summary_precision_label(label: &str) -> &str {
    match label {
        "local" => "local",
        "setup_aware" => "setup_aware",
        "heuristic" => "heuristic",
        "unknown_top" => "unknown_top",
        _ => label,
    }
}

#[cfg(test)]
fn direct_summary_count_invariants(
    summaries: &serde_json::Map<String, Value>,
) -> Vec<ObservedItem> {
    let Some(counts) = summaries.get("counts").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut invariants = Vec::new();
    for field in [
        "total",
        "present",
        "unknown",
        "unsupported",
        "setup_missing",
        "budget_exceeded",
        "events",
    ] {
        if let Some(value) = counts.get(field).and_then(Value::as_u64) {
            invariants.push(observed_invariant(
                format!("direct_summaries.counts.{field}"),
                value.to_string(),
                "kernel.metadata_debug_json.summaries.counts",
            ));
        }
    }
    invariants
}

#[cfg(test)]
fn direct_summary_domain_count_invariants(
    summaries: &serde_json::Map<String, Value>,
) -> Vec<ObservedItem> {
    let Some(domain_counts) = summaries.get("domain_counts").and_then(Value::as_object) else {
        return Vec::new();
    };
    domain_counts
        .iter()
        .filter(|(_, v)| v.as_u64().is_some_and(|n| n > 0))
        .map(|(domain, _)| {
            observed_invariant(
                format!("direct_summaries.domain_counts.{domain}.nonzero"),
                "true",
                "kernel.metadata_debug_json.summaries.domain_counts",
            )
        })
        .collect()
}

#[cfg(test)]
fn entrypoint_facts(debug_json: &Value) -> Vec<ObservedItem> {
    let mut facts = Vec::new();
    let Some(entrypoints) = debug_json.get("entrypoints").and_then(Value::as_object) else {
        return facts;
    };

    // Entrypoint detail rows -> ObservedFact
    if let Some(rows) = entrypoints.get("entrypoints").and_then(Value::as_array) {
        for row in rows {
            if let Some(fact) = entrypoint_fact(row, "Entrypoint") {
                facts.push(ObservedItem::Fact(fact));
            }
            if let Some(invariant) = entrypoint_detail_invariant(row) {
                facts.push(invariant);
            }
        }
    }

    // Trust boundary detail rows -> ObservedFact
    if let Some(rows) = entrypoints
        .get("trust_boundaries")
        .and_then(Value::as_array)
    {
        for row in rows {
            if let Some(fact) = entrypoint_fact(row, "TrustBoundary") {
                facts.push(ObservedItem::Fact(fact));
            }
        }
    }

    // Dispatch edge detail rows -> ObservedFact
    if let Some(rows) = entrypoints.get("dispatch_edges").and_then(Value::as_array) {
        for row in rows {
            if let Some(fact) = entrypoint_fact(row, "DispatchEdge") {
                facts.push(ObservedItem::Fact(fact));
            }
        }
    }

    // Unresolved detail rows -> ObservedFact
    if let Some(rows) = entrypoints.get("unresolved").and_then(Value::as_array) {
        for row in rows {
            if let Some(fact) = entrypoint_fact(row, "UnresolvedFramework") {
                facts.push(ObservedItem::Fact(fact));
            }
        }
    }

    // Count invariants from the counts section
    facts.extend(entrypoint_count_invariants(entrypoints));
    facts
}

#[cfg(test)]
fn entrypoint_fact(row: &Value, family: &str) -> Option<ObservedFact> {
    let stable_key = row.get("stable_key")?.as_str()?.to_string();
    let precision = row.get("precision").and_then(Value::as_str).map(|p| {
        match p {
            "ResolvedStatic" => "resolved_static",
            "SetupAware" => "setup_aware",
            "Heuristic" => "heuristic",
            "Conservative" => "conservative",
            "Unknown" => "unknown",
            other => other,
        }
        .to_string()
    });
    let status = row
        .get("status")
        .and_then(Value::as_str)
        .and_then(|s| match s {
            "Resolved" => Some(ObservedStatus::Resolved),
            "Partial" => Some(ObservedStatus::Partial),
            "Unresolved" => Some(ObservedStatus::Unresolved),
            "SetupMissing" => Some(ObservedStatus::SetupMissing),
            "Unsupported" => Some(ObservedStatus::Unsupported),
            _ => None,
        });

    // Build compact payload fragment
    let mut payload_parts = Vec::new();
    if let Some(framework_id) = row.get("framework_id").and_then(Value::as_str) {
        payload_parts.push(format!("framework={framework_id}"));
    }
    if let Some(function_name) = row.get("function_name").and_then(Value::as_str) {
        payload_parts.push(format!("function={function_name}"));
    }
    if let Some(kind) = row.get("kind").and_then(Value::as_str) {
        payload_parts.push(format!("kind={kind}"));
    }
    if let Some(source_kind) = row.get("source_kind").and_then(Value::as_str) {
        payload_parts.push(format!("source_kind={source_kind}"));
    }
    if let Some(edge_kind) = row.get("edge_kind").and_then(Value::as_str) {
        payload_parts.push(format!("edge_kind={edge_kind}"));
    }
    if let Some(reason) = row.get("reason").and_then(Value::as_str) {
        payload_parts.push(format!("reason={reason}"));
    }
    if let Some(trigger) = row.get("trigger_summary").and_then(Value::as_str)
        && trigger != "none"
    {
        payload_parts.push(format!("trigger={trigger}"));
    }
    let payload = if payload_parts.is_empty() {
        None
    } else {
        Some(payload_parts.join(";"))
    };

    Some(ObservedFact {
        family: family.to_string(),
        stable_key,
        mode: AssertionMode::Partial,
        producer_id: Some("polint.entrypoints".to_string()),
        provenance: Some("kernel.metadata_debug_json.entrypoints".to_string()),
        precision,
        status: Some(status.unwrap_or(ObservedStatus::Present)),
        payload,
    })
}

#[cfg(test)]
fn entrypoint_detail_invariant(row: &Value) -> Option<ObservedItem> {
    let framework_id = row.get("framework_id")?.as_str()?;
    let kind = row.get("kind")?.as_str()?;
    let function_name = row.get("function_name")?.as_str()?;
    let trigger = row
        .get("trigger_summary")
        .and_then(Value::as_str)
        .unwrap_or("none");

    Some(observed_invariant(
        format!(
            "framework_entrypoints.entrypoint.{}.{}.{}",
            invariant_label_part(framework_id),
            invariant_label_part(kind),
            invariant_label_part(function_name)
        ),
        trigger,
        "kernel.metadata_debug_json.entrypoints.entrypoints",
    ))
}

#[cfg(test)]
fn invariant_label_part(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch == '.' || ch == '_' || ch == '-' || ch.is_ascii_alphanumeric() {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
fn entrypoint_count_invariants(entrypoints: &serde_json::Map<String, Value>) -> Vec<ObservedItem> {
    let Some(counts) = entrypoints.get("counts").and_then(Value::as_object) else {
        return Vec::new();
    };

    let mut invariants = Vec::new();

    // Total counts
    for field in [
        "total_entrypoints",
        "total_trust_boundaries",
        "total_dispatch_edges",
        "total_unresolved",
    ] {
        if let Some(value) = counts.get(field).and_then(Value::as_u64)
            && value > 0
        {
            invariants.push(observed_invariant(
                format!("framework_entrypoints.counts.{field}.nonzero"),
                "true",
                "kernel.metadata_debug_json.entrypoints.counts",
            ));
        }
    }

    // Breakdown counts
    for group in [
        "by_language",
        "by_framework_id",
        "by_kind",
        "by_status",
        "by_precision",
        "trust_boundary_by_source_kind",
        "dispatch_edge_by_edge_kind",
        "unresolved_by_reason",
    ] {
        let Some(values) = counts.get(group).and_then(Value::as_object) else {
            continue;
        };
        for (label, count) in values {
            if count.as_u64().is_some_and(|count| count > 0) {
                invariants.push(observed_invariant(
                    format!("framework_entrypoints.counts.{group}.{label}.nonzero"),
                    "true",
                    "kernel.metadata_debug_json.entrypoints.counts",
                ));
            }
        }
    }
    invariants
}

#[cfg(test)]
fn abstract_domain_status_from_row(row: &Value) -> ObservedStatus {
    row.get("status")
        .and_then(Value::as_str)
        .and_then(abstract_domain_status_label)
        .or_else(|| {
            row.get("precision")
                .and_then(Value::as_str)
                .and_then(abstract_domain_status_label)
        })
        .unwrap_or(ObservedStatus::Present)
}

#[cfg(test)]
fn abstract_domain_status_label(label: &str) -> Option<ObservedStatus> {
    match label {
        "Present" | "present" => Some(ObservedStatus::Present),
        "Top" | "top" => Some(ObservedStatus::Top),
        "Unknown" | "unknown" => Some(ObservedStatus::Unknown),
        "Unsupported" | "unsupported" => Some(ObservedStatus::Unsupported),
        "SetupMissing" | "setup_missing" => Some(ObservedStatus::SetupMissing),
        "BudgetExceeded" | "budget_exceeded" => Some(ObservedStatus::BudgetExceeded),
        _ => None,
    }
}

#[cfg(test)]
fn abstract_domain_precision_label(label: &str) -> String {
    match label {
        "ExactLocal" | "exact_local" => "exact_local",
        "SetupAware" | "setup_aware" => "setup_aware",
        "Conservative" | "conservative" => "conservative",
        "Heuristic" | "heuristic" => "heuristic",
        "Unknown" | "unknown" => "unknown",
        "Unsupported" | "unsupported" => "unsupported",
        _ => label,
    }
    .to_string()
}

#[cfg(test)]
fn push_str_fragment(parts: &mut Vec<String>, key: &str, value: Option<&Value>) {
    if let Some(value) = value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("{key}={value}"));
    }
}

#[cfg(test)]
fn push_u64_fragment(parts: &mut Vec<String>, key: &str, value: Option<&Value>) {
    if let Some(value) = value.and_then(Value::as_u64) {
        parts.push(format!("{key}={value}"));
    }
}

#[cfg(test)]
fn push_list_fragment(parts: &mut Vec<String>, key: &str, value: Option<&Value>) {
    let Some(values) = value.and_then(Value::as_array) else {
        return;
    };
    let labels = values
        .iter()
        .filter_map(Value::as_str)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if !labels.is_empty() {
        parts.push(format!("{key}={}", labels.join(">")));
    }
}

#[cfg(test)]
fn push_span_fragment(parts: &mut Vec<String>, value: Option<&Value>) {
    let Some(span) = value else {
        return;
    };
    let (Some(start_line), Some(start_col), Some(end_line), Some(end_col)) = (
        span.get("start_line").and_then(Value::as_u64),
        span.get("start_col").and_then(Value::as_u64),
        span.get("end_line").and_then(Value::as_u64),
        span.get("end_col").and_then(Value::as_u64),
    ) else {
        return;
    };
    parts.push(format!(
        "span={start_line}:{start_col}-{end_line}:{end_col}"
    ));
}

#[cfg(test)]
fn metadata_debug_fact(row: &Value) -> Option<ObservedFact> {
    Some(ObservedFact {
        family: row.get("family")?.as_str()?.to_string(),
        stable_key: row.get("stable_key")?.as_str()?.to_string(),
        mode: AssertionMode::Exact,
        producer_id: row
            .get("producer_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        provenance: row
            .get("layer_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        precision: row
            .get("precision")
            .or_else(|| row.pointer("/metadata/precision"))
            .and_then(Value::as_str)
            .map(str::to_string),
        status: Some(observed_status_from_metadata(row)),
        payload: observed_fact_payload(row),
    })
}

#[cfg(test)]
fn observed_fact_payload(row: &Value) -> Option<String> {
    let payload = serde_json::json!({
        "path": row.get("path").cloned(),
        "span": row.get("span").cloned(),
        "name": row.get("name").cloned(),
        "export_name": row.get("export_name").cloned(),
        "source_stable_key": row.get("source_stable_key").cloned(),
        "target_stable_keys": row.get("target_stable_keys").cloned(),
        "generated_discriminator": row.get("generated_discriminator").cloned(),
    });
    let object = payload.as_object()?;
    if object.values().all(Value::is_null) {
        return None;
    }
    Some(payload.to_string())
}

#[cfg(test)]
fn observed_status_from_metadata(row: &Value) -> ObservedStatus {
    for field in ["status", "precision"] {
        if let Some(status) = row
            .get(field)
            .and_then(Value::as_str)
            .and_then(observed_status_from_label)
        {
            return status;
        }
    }
    ObservedStatus::Present
}

#[cfg(test)]
fn observed_status_from_label(label: &str) -> Option<ObservedStatus> {
    match label {
        "resolved" => Some(ObservedStatus::Resolved),
        "partial" => Some(ObservedStatus::Partial),
        "top" => Some(ObservedStatus::Top),
        "unknown" => Some(ObservedStatus::Unknown),
        "unresolved" => Some(ObservedStatus::Unresolved),
        "ambiguous" => Some(ObservedStatus::Ambiguous),
        "dynamic" => Some(ObservedStatus::Dynamic),
        "setup_missing" => Some(ObservedStatus::SetupMissing),
        "missing_lockfile" => Some(ObservedStatus::MissingLockfile),
        "unsupported" => Some(ObservedStatus::Unsupported),
        "external" => Some(ObservedStatus::External),
        "cycle" => Some(ObservedStatus::Cycle),
        "generated" => Some(ObservedStatus::Generated),
        "undeclared" => Some(ObservedStatus::Undeclared),
        "outside_workspace" => Some(ObservedStatus::OutsideWorkspace),
        "budget_exceeded" => Some(ObservedStatus::BudgetExceeded),
        _ => None,
    }
}

#[cfg(test)]
fn runtime_budget_name(expected: &[ExpectedItem], case_id: &str) -> String {
    expected
        .iter()
        .find_map(|item| match item {
            ExpectedItem::RuntimeBudget(budget) => Some(budget.name.clone()),
            _ => None,
        })
        .unwrap_or_else(|| case_id.to_string())
}

#[cfg(test)]
fn normalize_observed_path(path: &str, repo_root: &Path) -> String {
    let path = Path::new(path);
    let relative = if path.is_absolute() {
        path.strip_prefix(repo_root).unwrap_or(path)
    } else {
        path
    };
    slash_path(relative)
}

#[cfg(test)]
fn slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
fn saturating_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
fn observed_sort_key(item: &ObservedItem) -> String {
    serde_json::to_string(item).unwrap_or_default()
}

#[cfg(test)]
mod cfg {
    use crate::eval::model::ObservedItem;

    use super::*;

    #[test]
    fn cfg_facts_include_all_debug_families_with_compact_payloads() {
        let debug_json = serde_json::json!({
            "cfg": {
                "functions": [cfg_row("CfgFunction", "cfg:function", "resolved", "exact_lowered")],
                "nodes": [cfg_row("CfgNode", "cfg:node", "resolved", "exact_lowered")],
                "blocks": [cfg_row("BasicBlock", "cfg:block", "resolved", "exact_lowered")],
                "edges": [cfg_row("CfgEdge", "cfg:edge", "resolved", "exact_lowered")],
                "reachability": [cfg_row("CfgReachability", "cfg:reachability", "resolved", "exact_lowered")],
                "dominators": [cfg_row("CfgDominator", "cfg:dominator", "resolved", "exact_lowered")],
                "postdominators": [cfg_row("CfgPostDominator", "cfg:postdominator", "resolved", "exact_lowered")],
                "control_dependence": [cfg_row("CfgControlDependence", "cfg:dependence", "resolved", "exact_lowered")],
                "unsupported": [cfg_row("UnsupportedControlFlow", "cfg:unsupported", "unsupported", "unsupported")]
            }
        });
        let facts = cfg_facts_for_test(&debug_json);
        let families = facts
            .iter()
            .filter_map(|item| match item {
                ObservedItem::Fact(fact) => Some(fact.family.as_str()),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();

        for family in crate::eval::model::CFG_FACT_FAMILIES {
            assert!(families.contains(family), "missing CFG family {family}");
        }
        assert!(facts.iter().any(|item| {
            match item {
                ObservedItem::Fact(fact) => fact
                    .payload
                    .as_deref()
                    .is_some_and(|payload| payload.contains("view=normal_control")),
                _ => false,
            }
        }));
    }

    fn cfg_row(family: &str, stable_key: &str, status: &str, precision: &str) -> serde_json::Value {
        serde_json::json!({
            "family": family,
            "stable_key": stable_key,
            "producer_id": "polint.cfg",
            "status": status,
            "precision": precision,
            "path": "src/app.ts",
            "span": {"start_line": 1, "start_col": 1, "end_line": 1, "end_col": 2},
            "function": 1,
            "view": "normal_control",
            "node_kind": "operation",
            "block_kind": "straight_line",
            "payload": "kind=return;from=1;to=2",
            "unsupported_construct": "throw"
        })
    }
}

#[cfg(test)]
mod eval_observed_kernel_tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::eval::fixtures::{NativeFixture, load_native_fixture, run_native_fixture_for_test};
    use crate::eval::model::{ObservedItem, ObservedStatus};
    use crate::eval::report::{
        CaseResult, EvaluationRun, MetricSummary, RuntimeObservation, deterministic_output_hash,
    };

    use super::*;

    #[test]
    fn eval_observed_native_promotion_fixture_runs_through_fixture_runner() {
        let fixture_dir = repo_root().join("tests/eval-fixtures/promotion/cfg-call-flow-evidence");
        let run = run_native_fixture_for_test(&fixture_dir).unwrap();
        let case = &run.cases[0];

        assert_eq!(case.case_id, "cfg-call-flow-evidence");
        assert!(case.matches.iter().any(|summary| {
            summary.item_kind == crate::eval::matcher::MatchItemKind::GraphEdge
                && summary.outcome == crate::eval::matcher::MatchOutcome::Unconfirmed
        }));
        assert_eq!(run.metrics.runtime_budget_passed, 1);
        assert!(run.metrics.unknown_count >= 1);
    }

    fn write_fixture(root: &Path, source: &str, budget: Option<u64>) {
        fs::create_dir_all(root.join("repo/src")).unwrap();
        fs::write(
            root.join("repo/.polint.toml"),
            r#"
[workspace]
include = ["src/**"]
exclude = []
"#,
        )
        .unwrap();
        fs::write(root.join("repo/src/app.ts"), source).unwrap();

        let budget = budget.map_or_else(String::new, |max_runtime_ms| {
            format!(
                r#"
[budget]
max_runtime_ms = {max_runtime_ms}
"#
            )
        });
        fs::write(
            root.join("expected.polint-eval.toml"),
            format!(
                r#"
schema_version = "polint-eval-fixture-1"
case_id = "provider-order"
area = "kernel"

[repo]
path = "repo"
{budget}
"#
            ),
        )
        .unwrap();
    }

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root")
            .to_path_buf()
    }

    #[test]
    fn eval_observed_type_value_alias_emits_status_specific_alias_rows() {
        use crate::analysis::aliases::facts::{
            AliasAnswerFact, AliasOperand, AliasPrecision, AliasReason, AliasStatus,
        };
        use crate::analysis::aliases::store::AliasOutput;
        use crate::analysis::ids::{AliasAnswerId, PlaceId};
        use crate::analysis::types::store::TypeValueAliasOutput;

        let mut db = AnalysisDb::new();
        db.replace_type_value_alias_facts(TypeValueAliasOutput {
            aliases: AliasOutput {
                answers: [
                    AliasStatus::NoAlias,
                    AliasStatus::MayAlias,
                    AliasStatus::MustAlias,
                    AliasStatus::PartialAlias,
                    AliasStatus::Unknown,
                ]
                .into_iter()
                .enumerate()
                .map(|(index, status)| AliasAnswerFact {
                    id: AliasAnswerId(index as u64),
                    left: AliasOperand::Place(PlaceId(index as u64)),
                    right: AliasOperand::Place(PlaceId(index as u64 + 1)),
                    status,
                    reason: AliasReason::ExtensionProvided,
                    evidence: vec!["eval".to_string()],
                    precision: AliasPrecision::Heuristic,
                    stable_key: crate::core::stable_key_for_test(&format!("alias:{status:?}")),
                })
                .collect(),
            },
            ..TypeValueAliasOutput::default()
        });

        let observed = type_value_alias_facts(&db);
        for status in [
            "NoAlias",
            "MayAlias",
            "MustAlias",
            "PartialAlias",
            "Unknown",
        ] {
            assert!(observed.iter().any(|item| match item {
                ObservedItem::Fact(fact) =>
                    fact.family == format!("AliasAnswer.{status}")
                        && fact.producer_id.as_deref() == Some("polint.type_value_alias"),
                _ => false,
            }));
        }
    }

    fn native_fixture(source: &str, budget: Option<u64>) -> (tempfile::TempDir, NativeFixture) {
        let temp = tempfile::tempdir().unwrap();
        let fixture_dir = temp
            .path()
            .join("tests/eval-fixtures/kernel/provider-order");
        write_fixture(&fixture_dir, source, budget);
        let fixture = load_native_fixture(&fixture_dir).unwrap();
        (temp, fixture)
    }

    fn observed_for(source: &str, budget: Option<u64>) -> (tempfile::TempDir, Vec<ObservedItem>) {
        let (temp, fixture) = native_fixture(source, budget);
        let observed = observe_kernel_fixture(&fixture).unwrap();
        (temp, observed)
    }

    mod observed_phase23_cache_counter_invariants {
        use std::collections::BTreeMap;

        use super::*;

        fn write_mixed_fixture(root: &Path) {
            fs::create_dir_all(root.join("repo/goapp")).unwrap();
            fs::create_dir_all(root.join("repo/web/src")).unwrap();
            fs::write(
                root.join("repo/.polint.toml"),
                r#"
[workspace]
include = ["goapp/**", "web/**"]
exclude = []

[languages.go]
module_roots = ["goapp"]
package_patterns = ["./..."]
build_tags = ["integration"]
include_tests = false

[languages.ts]
module_kind = "esnext"
target = "es2022"
"#,
            )
            .unwrap();
            fs::write(
                root.join("repo/goapp/go.mod"),
                "module example.com/payment\n\ngo 1.24\n",
            )
            .unwrap();
            fs::write(root.join("repo/goapp/payment.go"), "package payment\n").unwrap();
            fs::write(
                root.join("repo/web/package.json"),
                r#"{"name":"web","version":"1.0.0","type":"module"}"#,
            )
            .unwrap();
            fs::write(
                root.join("repo/web/package-lock.json"),
                r#"{"name":"web","lockfileVersion":3,"packages":{}}"#,
            )
            .unwrap();
            fs::write(
                root.join("repo/web/tsconfig.json"),
                r#"{"compilerOptions":{"module":"esnext","target":"es2022"}}"#,
            )
            .unwrap();
            fs::write(
                root.join("repo/web/src/app.ts"),
                "export const amount = 42;\n",
            )
            .unwrap();
            fs::write(
                root.join("expected.polint-eval.toml"),
                r#"
schema_version = "polint-eval-fixture-1"
case_id = "input-snapshots"
area = "cache"

[repo]
path = "repo"
"#,
            )
            .unwrap();
        }

        fn observed_for_mixed_fixture() -> (tempfile::TempDir, Vec<ObservedItem>) {
            let temp = tempfile::tempdir().unwrap();
            let fixture_dir = temp
                .path()
                .join("tests/eval-fixtures/cache/input-snapshots");
            write_mixed_fixture(&fixture_dir);
            let fixture = load_native_fixture(&fixture_dir).unwrap();
            let observed = observe_kernel_fixture(&fixture).unwrap();
            (temp, observed)
        }

        fn invariant_values(observed: &[ObservedItem]) -> BTreeMap<&str, &str> {
            observed
                .iter()
                .filter_map(|item| match item {
                    ObservedItem::Invariant(invariant) => {
                        Some((invariant.name.as_str(), invariant.value.as_str()))
                    }
                    _ => None,
                })
                .collect()
        }

        #[test]
        fn snapshot_invariants_emit_required_phase23_inputs() {
            let (_temp, observed) = observed_for_mixed_fixture();
            let invariants = invariant_values(&observed);

            assert_eq!(
                invariants.get("snapshot.schema_version").copied(),
                Some("polint-input-snapshot-1")
            );
            assert_eq!(
                invariants
                    .get("snapshot.source_text_digest.present")
                    .copied(),
                Some("true")
            );
            assert_eq!(
                invariants.get("snapshot.config_digest.present").copied(),
                Some("true")
            );
            assert_eq!(
                invariants.get("snapshot.go_lifecycle.present").copied(),
                Some("true")
            );
            assert_eq!(
                invariants.get("snapshot.ts_js_lifecycle.present").copied(),
                Some("true")
            );
            assert_eq!(
                invariants.get("snapshot.rule_digest.present").copied(),
                Some("true")
            );
            assert_eq!(
                invariants.get("snapshot.model_digest.absent").copied(),
                Some("true")
            );
            assert_eq!(
                invariants
                    .get("snapshot.tool_invocation.unsupported")
                    .copied(),
                Some("true")
            );
        }

        #[test]
        fn provider_and_layer_key_invariants_emit_required_phase23_rows() {
            let (_temp, observed) = observed_for_mixed_fixture();
            let invariants = invariant_values(&observed);

            assert_eq!(
                invariants
                    .get("provider_output.polint.source.present")
                    .copied(),
                Some("true")
            );
            assert_eq!(
                invariants
                    .get("provider_output.polint.go.syntax.cache_stats.recorded")
                    .copied(),
                Some("true")
            );
            assert_eq!(
                invariants
                    .get("provider_output.polint.ts.syntax.cache_stats.recorded")
                    .copied(),
                Some("true")
            );
            assert_eq!(
                invariants
                    .get("layer_key.polint.ts.syntax.has_config_digest")
                    .copied(),
                Some("true")
            );
        }

        #[test]
        fn provider_cache_counter_invariants_emit_first_run_current_cache_values() {
            let (_temp, observed) = observed_for_mixed_fixture();
            let invariants = invariant_values(&observed);

            for provider_id in ["go", "ts"] {
                let prefix = format!("provider_output.polint.{provider_id}.syntax.cache_stats");
                for (counter, expected) in [
                    ("hits", "0"),
                    ("misses", "1"),
                    ("recomputes", "1"),
                    ("writes", "1"),
                    ("bypasses_disabled", "0"),
                    ("invalid_evicted_reads", "0"),
                    ("verified_reuse", "0"),
                    ("quarantines", "0"),
                ] {
                    let name = format!("{prefix}.{counter}");
                    assert_eq!(
                        invariants.get(name.as_str()).copied(),
                        Some(expected),
                        "missing or wrong invariant {name}"
                    );
                }
            }
        }

        #[test]
        fn observed_invariants_do_not_claim_future_cache_layer_semantics() {
            let (_temp, observed) = observed_for_mixed_fixture();
            let rendered = serde_json::to_string(&observed).unwrap();

            for forbidden in [
                ["Layer", "Cache"].concat(),
                ["Dependency", "Index"].concat(),
                ["Invalidation", "Plan"].concat(),
                ["stale", " reuse"].concat(),
                ["verified", " reuse success"].concat(),
                ["quarantine", " event"].concat(),
            ] {
                assert!(
                    !rendered.contains(forbidden.as_str()),
                    "unexpected {forbidden}"
                );
            }
        }
    }

    #[test]
    fn eval_observed_kernel_collects_provider_order_invariants_from_real_kernel() {
        let (_temp, observed) = observed_for("export function answer() { return 42; }\n", None);

        let provider_order = observed
            .iter()
            .filter_map(|item| match item {
                ObservedItem::Invariant(invariant)
                    if invariant.name.starts_with("provider_order.") =>
                {
                    Some((invariant.name.as_str(), invariant.value.as_str()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut provider_order = provider_order;
        provider_order.sort_by_key(|(name, _)| {
            name.strip_prefix("provider_order.")
                .and_then(|index| index.parse::<usize>().ok())
                .expect("provider order invariant names use numeric suffixes")
        });

        assert_eq!(
            provider_order,
            vec![
                ("provider_order.0", "polint.source"),
                ("provider_order.1", "polint.go.syntax"),
                ("provider_order.2", "polint.ts.syntax"),
                ("provider_order.3", "polint.module_graph"),
                ("provider_order.4", "polint.symbol_graph"),
                ("provider_order.5", "polint.module_topology"),
                ("provider_order.6", "polint.semantic_mir"),
                ("provider_order.7", "polint.cfg"),
                ("provider_order.8", "polint.calls"),
                ("provider_order.9", "polint.go.semantic"),
                ("provider_order.10", "polint.identity"),
                ("provider_order.11", "polint.abstract_domains"),
                ("provider_order.12", "polint.direct_summaries"),
                ("provider_order.13", "polint.entrypoints"),
                ("provider_order.14", "polint.reachability"),
                ("provider_order.15", "polint.extensions"),
                ("provider_order.16", "polint.type_value_alias"),
                ("provider_order.17", "polint.semantic_graph"),
                ("provider_order.18", "polint.solver"),
                ("provider_order.19", "polint.refined_calls"),
                ("provider_order.20", "polint.data_flow"),
                ("provider_order.21", "polint.evidence"),
                ("provider_order.22", "polint.metrics"),
            ]
        );
    }

    #[test]
    fn eval_observed_kernel_records_abstract_domain_provider_output_schema() {
        let (_temp, observed) = observed_for("export function answer() { return 42; }\n", None);
        let invariants = observed
            .iter()
            .filter_map(|item| match item {
                ObservedItem::Invariant(invariant) => {
                    Some((invariant.name.as_str(), invariant.value.as_str()))
                }
                _ => None,
            })
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(
            invariants
                .get("provider_output.polint.abstract_domains.schema_version")
                .copied(),
            Some("abstract-domain-facts-1:1")
        );
    }

    #[test]
    fn eval_observed_kernel_extracts_scc_and_demand_query_debug_invariants() {
        let debug = serde_json::json!({
            "scc_schedule": {
                "total_sccs": 3,
                "recursive_scc_count": 1,
                "max_scc_size": 2,
                "iteration_counts": [
                    {"member_stable_keys": ["func::pingGo", "func::pongGo"], "iterations": 2}
                ]
            },
            "demand_queries": {
                "total_queries": 3,
                "cache_hits": 1,
                "cache_misses": 2,
                "entries": []
            }
        });
        let observed = metadata_debug_facts_for_test(&debug);
        let invariants = observed
            .iter()
            .filter_map(|item| match item {
                ObservedItem::Invariant(invariant) => {
                    Some((invariant.name.as_str(), invariant.value.as_str()))
                }
                _ => None,
            })
            .collect::<std::collections::BTreeMap<_, _>>();

        for required in [
            "scc_schedule.total_sccs.nonzero",
            "scc_schedule.recursive_scc_count.nonzero",
            "scc_schedule.max_scc_size.at_least_2",
            "scc_schedule.iteration_counts.nonzero",
            "demand_queries.total_queries.nonzero",
            "demand_queries.cache_hits.nonzero",
            "demand_queries.cache_misses.nonzero",
        ] {
            assert_eq!(invariants.get(required).copied(), Some("true"));
        }
    }

    #[test]
    fn eval_observed_kernel_normalizes_diagnostics_to_relative_paths_and_fingerprints() {
        let (temp, observed) = observed_for("export function broken(\n", None);
        let rendered = serde_json::to_string_pretty(&observed).unwrap();
        let temp_root = temp.path().to_string_lossy();

        let diagnostics = observed
            .iter()
            .filter_map(|item| match item {
                ObservedItem::Diagnostic(diagnostic) => Some(diagnostic),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.rule_id == "parser/ts"
                && diagnostic.relative_path == "src/app.ts"
                && diagnostic
                    .fingerprint
                    .as_deref()
                    .is_some_and(|fingerprint| !fingerprint.is_empty())
                && diagnostic.status == Some(ObservedStatus::Present)
        }));
        assert!(!rendered.contains(temp_root.as_ref()));
    }

    #[test]
    fn eval_observed_kernel_collects_metadata_debug_facts_with_stable_identity() {
        let (_temp, observed) = observed_for("export function answer() { return 42; }\n", None);

        assert!(observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.family == "SourceFile"
                    && fact.stable_key.contains("src/app.ts")
                    && fact.producer_id.as_deref() == Some("polint.source")
                    && fact.precision.as_deref() == Some("exact")
                    && fact.status == Some(ObservedStatus::Present)
            }
            _ => false,
        }));
    }

    #[test]
    fn eval_observed_kernel_json_excludes_absolute_temp_paths() {
        let (temp, observed) =
            observed_for("export function answer() { return 42; }\n", Some(60_000));
        let rendered = serde_json::to_string_pretty(&observed).unwrap();
        let temp_root = temp.path().to_string_lossy();

        assert!(!rendered.contains(temp_root.as_ref()));
        assert!(!rendered.contains("0x"));
    }

    #[test]
    fn eval_observed_kernel_runtime_budget_duration_does_not_change_output_hash() {
        let (_temp, observed) =
            observed_for("export function answer() { return 42; }\n", Some(60_000));
        let first = run_with_observed_runtime(observed.clone(), 1);
        let second = run_with_observed_runtime(observed, 999);

        assert_eq!(
            deterministic_output_hash(&first),
            deterministic_output_hash(&second)
        );
    }

    fn run_with_observed_runtime(
        mut observed: Vec<ObservedItem>,
        runtime_ms: u64,
    ) -> EvaluationRun {
        for item in &mut observed {
            if let ObservedItem::RuntimeBudget(budget) = item {
                budget.observed_runtime_ms = Some(runtime_ms);
            }
        }

        EvaluationRun {
            schema_version: "polint-eval-internal-1".to_string(),
            suite_id: "native-fixture".to_string(),
            mode: crate::eval::model::EvaluationMode::PolintBaseline,
            suite_manifest: None,
            cases: vec![CaseResult {
                case_id: "provider-order".to_string(),
                area: crate::eval::model::FixtureArea::Kernel,
                expected: Vec::new(),
                observed,
                matches: Vec::new(),
                runtime: RuntimeObservation {
                    budget_name: "provider-order".to_string(),
                    budget_passed: true,
                    observed_runtime_ms: Some(runtime_ms),
                    peak_rss_bytes: None,
                },
            }],
            metrics: MetricSummary {
                true_positives: 0,
                false_positives: 0,
                false_negatives: 0,
                true_negatives: 0,
                unconfirmed: 0,
                false_positive_trap_hits: 0,
                forbidden_hits: 0,
                unknown_count: 0,
                facts_present: 0,
                facts_accepted: 0,
                facts_rejected: 0,
                graph_edges_expected: 0,
                graph_edges_observed: 0,
                graph_edges_unconfirmed: 0,
                paths_expected: 0,
                paths_observed: 0,
                paths_unconfirmed: 0,
                runtime_budget_passed: 1,
                runtime_budget_failed: 0,
                precision: None,
                recall: None,
                f1: None,
                f2: None,
                f3: None,
                false_positive_rate: None,
                sections: crate::eval::report::MetricSections::default(),
            },
            performance: None,
            comparison_rows: Vec::new(),
            adaptation: None,
            adaptation_delta: None,
            limitations: Vec::new(),
            output_hash: String::new(),
        }
    }
}
