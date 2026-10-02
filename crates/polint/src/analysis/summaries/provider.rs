use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::builder::DirectSummaryBuilder;
use super::cache_key::direct_summaries_provider_parameter_digest;
#[cfg(test)]
use super::closure::SccClosureResult;
use super::closure::{SccClosureConfig, close_summaries_by_scc};
#[cfg(test)]
use super::scc::SccSchedule;
use super::scc::compute_scc_schedule;
use super::store::SummaryOutput;
use crate::analysis::ids::MirBodyId;
use crate::analysis_api::{ProviderExecution, ProviderFailureReason, ProviderFailureStage};
use crate::analysis_kernel::ProviderManifest;
use crate::analysis_kernel::incremental::{
    CacheStats, DemandQueryEngine, DemandQueryTrace, Digest, DigestKind, InputSnapshot,
};
pub(crate) use crate::analysis_neutral::summaries::provider::{
    callable_stable_key_map, direct_summaries_output_digest,
};
use crate::cache::{Cache, CacheKey};
use crate::core::AnalysisDb;
use crate::diagnostics::Diagnostic;

#[derive(Debug, Clone, Default)]
pub(crate) struct DirectSummariesProviderOutput {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) cache_stats: CacheStats,
    pub(crate) output_digest: Option<Digest>,
    pub(crate) execution: ProviderExecution,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn derive_direct_summaries_with_cache_stats(
    db: &mut AnalysisDb,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    semantic_mir_output_digest: Digest,
    cfg_output_digest: Digest,
    calls_output_digest: Digest,
    abstract_domains_output_digest: Digest,
    symbol_graph_output_digest: Digest,
    module_topology_output_digest: Digest,
    upstream_syntax_output_digests: Vec<Digest>,
) -> DirectSummariesProviderOutput {
    let mut started = std::time::Instant::now();
    let mut checkpoint = |step: &'static str| {
        tracing::debug!(target: "polint::kernel::stage", provider = "polint.direct_summaries", step, elapsed_ms = started.elapsed().as_millis() as u64, "provider step");
        started = std::time::Instant::now();
    };
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let output = DirectSummaryBuilder::build(interner, db);
    checkpoint("build");
    let callable_keys = callable_stable_key_map(db);
    let output_digest = direct_summaries_output_digest(
        manifest,
        input_snapshot,
        &semantic_mir_output_digest,
        &cfg_output_digest,
        &calls_output_digest,
        &abstract_domains_output_digest,
        &symbol_graph_output_digest,
        &module_topology_output_digest,
        &upstream_syntax_output_digests,
        interner,
        &callable_keys,
        &output,
    );
    checkpoint("digest");
    let mut cache_stats = CacheStats::default();
    cache_stats.record_recompute();
    db.replace_summary_facts(output);
    checkpoint("store_metadata");

    DirectSummariesProviderOutput {
        diagnostics: Vec::new(),
        cache_stats,
        output_digest: Some(output_digest),
        execution: Default::default(),
    }
}

// ---------------------------------------------------------------------------
// SCC closure orchestration
// ---------------------------------------------------------------------------

/// Output of the SCC closure step.
#[derive(Debug, Clone, Default)]
pub(crate) struct SccClosureProviderOutput {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) demand_query_trace: DemandQueryTrace,
    pub(crate) scc_output_digests: BTreeMap<Vec<String>, String>,
    #[cfg(test)]
    pub(crate) debug_snapshot: Option<SccClosureDebugSnapshot>,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SccClosureDebugSnapshot {
    pub(crate) schedule: SccSchedule,
    pub(crate) result: SccClosureResult,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct SccClosureDigestCache {
    schema: String,
    scc_digests: Vec<SccClosureDigestCacheEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SccClosureDigestCacheEntry {
    members: Vec<String>,
    digest: String,
}

/// Runs interprocedural summary closure over SCCs discovered from the current
/// call graph.
///
/// Steps:
/// 1. Compute SCC schedule from current call targets.
/// 2. If no SCCs exist, return empty output with default trace.
/// 3. Create SccClosureConfig, empty previous_scc_digests (cross-run caching
///    is future work), and a DemandQueryEngine.
/// 4. Call close_summaries_by_scc.
/// 5. Return closure result, demand query trace, and any diagnostics.
#[cfg(test)]
pub(crate) fn run_scc_closure(db: &mut AnalysisDb) -> SccClosureProviderOutput {
    run_scc_closure_with_previous_digests(db, BTreeMap::new())
}

pub(crate) fn run_scc_closure_with_cache(
    db: &mut AnalysisDb,
    cache: &Cache,
    config_digest: &str,
    rule_digest: &str,
    plan_digest: &str,
) -> SccClosureProviderOutput {
    let cache_key = scc_closure_cache_key(config_digest, rule_digest, plan_digest);
    let previous_scc_digests = cache
        .read_json_with_status::<SccClosureDigestCache>(&cache_key)
        .value
        .filter(|entry| entry.schema == "summary-scc-closure-digests-v2")
        .map(|entry| {
            entry
                .scc_digests
                .into_iter()
                .map(|entry| (entry.members, entry.digest))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();

    let output = run_scc_closure_with_previous_digests(db, previous_scc_digests);

    let scc_digests = output.scc_output_digests.clone();

    if !scc_digests.is_empty() {
        let _ = cache.write_json_with_status(
            &cache_key,
            &SccClosureDigestCache {
                schema: "summary-scc-closure-digests-v2".to_string(),
                scc_digests: scc_digests
                    .into_iter()
                    .map(|(members, digest)| SccClosureDigestCacheEntry { members, digest })
                    .collect(),
            },
        );
    }

    output
}

fn run_scc_closure_with_previous_digests(
    db: &mut AnalysisDb,
    previous_scc_digests: BTreeMap<Vec<String>, String>,
) -> SccClosureProviderOutput {
    let schedule = compute_scc_schedule(db);

    if schedule.sccs.is_empty() {
        return SccClosureProviderOutput::default();
    }

    let config = SccClosureConfig::default();
    let mut demand_engine = DemandQueryEngine::default();

    let closure_result = close_summaries_by_scc(
        db,
        &schedule,
        &config,
        &mut demand_engine,
        &previous_scc_digests,
    );
    #[cfg(test)]
    let debug_snapshot = Some(SccClosureDebugSnapshot {
        schedule: schedule.clone(),
        result: closure_result.clone(),
    });

    // Generate diagnostics for budget-exceeded SCCs
    let mut diagnostics = Vec::new();
    if closure_result.budget_exceeded_sccs > 0 {
        for (members, iterations) in &closure_result.scc_iteration_counts {
            if *iterations >= config.max_iterations {
                diagnostics.push(Diagnostic::warning(
                    "internal/scc-closure",
                    members.join(", "),
                    crate::diagnostics::TextRange::point(0, 0),
                    format!(
                        "SCC fixpoint did not converge within {} iterations for SCC with members: {}",
                        config.max_iterations,
                        members.join(", ")
                    ),
                ));
            }
        }
    }

    let trace = demand_engine.into_trace();
    let scc_output_digests = closure_result.scc_output_digests;

    SccClosureProviderOutput {
        diagnostics,
        demand_query_trace: trace,
        scc_output_digests,
        #[cfg(test)]
        debug_snapshot,
    }
}

fn scc_closure_cache_key(config_digest: &str, rule_digest: &str, plan_digest: &str) -> CacheKey {
    CacheKey::for_file(
        "summary-scc-closure",
        "scc-output-digests",
        config_digest,
        rule_digest,
        plan_digest,
        "summary-scc-closure-digests-v2",
        &crate::analysis_api::engine_parser_identity(),
    )
}

#[cfg(test)]
mod direct_summaries_provider {
    use super::*;
    use crate::analysis_kernel::AnalysisKernel;
    use crate::analysis_kernel::incremental::{Digest, DigestKind};
    use crate::analysis_plan::AnalysisPlan;
    use crate::config::load_config;
    use crate::core::AnalysisDb;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn provider_accepts_empty_output_with_deterministic_digest() {
        let mut db = AnalysisDb::new();
        let temp = tempdir().expect("tempdir");
        fs::write(temp.path().join(".polint.toml"), "").expect("config");
        let loaded = load_config(temp.path()).expect("config loads");
        let input_snapshot = crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            &loaded,
            &db,
            "config",
            "rules",
            AnalysisPlan::empty().digest(),
            AnalysisKernel::provider_manifests(),
        );
        let output = derive_direct_summaries_with_cache_stats(
            &mut db,
            &input_snapshot,
            AnalysisKernel::provider_manifests()
                .iter()
                .find(|manifest| manifest.id == "polint.direct_summaries")
                .expect("direct summaries manifest should exist"),
            Digest::absent(DigestKind::ProviderOutput, "semantic_mir"),
            Digest::absent(DigestKind::ProviderOutput, "cfg"),
            Digest::absent(DigestKind::ProviderOutput, "calls"),
            Digest::absent(DigestKind::ProviderOutput, "abstract_domains"),
            Digest::absent(DigestKind::ProviderOutput, "symbol_graph"),
            Digest::absent(DigestKind::ProviderOutput, "module_topology"),
            Vec::new(),
        );

        assert!(output.diagnostics.is_empty());
        assert!(output.output_digest.is_some());
        assert_eq!(output.cache_stats.recomputes, 1);

        // Verify determinism
        let mut db2 = AnalysisDb::new();
        let output2 = derive_direct_summaries_with_cache_stats(
            &mut db2,
            &input_snapshot,
            AnalysisKernel::provider_manifests()
                .iter()
                .find(|manifest| manifest.id == "polint.direct_summaries")
                .expect("direct summaries manifest should exist"),
            Digest::absent(DigestKind::ProviderOutput, "semantic_mir"),
            Digest::absent(DigestKind::ProviderOutput, "cfg"),
            Digest::absent(DigestKind::ProviderOutput, "calls"),
            Digest::absent(DigestKind::ProviderOutput, "abstract_domains"),
            Digest::absent(DigestKind::ProviderOutput, "symbol_graph"),
            Digest::absent(DigestKind::ProviderOutput, "module_topology"),
            Vec::new(),
        );
        assert_eq!(output.output_digest, output2.output_digest);
    }
}

#[cfg(test)]
mod direct_summaries_provider_order {
    use crate::analysis_kernel::AnalysisKernel;

    #[test]
    fn direct_summaries_runs_after_abstract_domains_and_before_metrics() {
        let order = AnalysisKernel::provider_manifests()
            .iter()
            .map(|manifest| manifest.id)
            .collect::<Vec<_>>();
        let calls = order
            .iter()
            .position(|provider| *provider == "polint.calls")
            .expect("calls provider");
        let abstract_domains = order
            .iter()
            .position(|provider| *provider == "polint.abstract_domains")
            .expect("abstract domains provider");
        let direct_summaries = order
            .iter()
            .position(|provider| *provider == "polint.direct_summaries")
            .expect("direct summaries provider");
        let metrics = order
            .iter()
            .position(|provider| *provider == "polint.metrics")
            .expect("metrics provider");

        assert!(calls < abstract_domains);
        assert!(abstract_domains < direct_summaries);
        assert!(direct_summaries < metrics);
    }
}

#[cfg(test)]
mod scc_closure_provider {
    use super::*;
    use crate::analysis::calls::facts::{
        CallAlgorithm, CallCallee, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact,
        CallSyntaxKind, CallTargetFact, CallTargetStatus,
    };
    use crate::analysis::calls::store::CallOutput;
    use crate::analysis::ids::{CallSiteId, CallTargetId, MirBodyId, MirOpId, SummaryId};
    use crate::analysis::summaries::facts::{
        SummaryDomainKind, SummaryFact, SummaryPrecision, SummaryProvenance, SummaryStatus,
    };
    use crate::analysis::summaries::store::SummaryOutput;
    use crate::cache::Cache;
    use crate::core::{FileId, FunctionId, Language, Span, stable_key_for_test};

    fn span() -> Span {
        Span::point(FileId::from_raw(1), 1, 1)
    }

    fn summary_fact(function_id: u64, callable_key: &str) -> SummaryFact {
        SummaryFact {
            id: SummaryId(0),
            callable_stable_key: stable_key_for_test(callable_key),
            function: FunctionId::from_raw(function_id),
            domain: SummaryDomainKind::ControlEffects,
            status: SummaryStatus::Present,
            precision: SummaryPrecision::Local,
            provenance: SummaryProvenance::NativeLocal,
            payload_digest: format!("digest:{callable_key}"),
            tito_flows: Vec::new(),
            stable_key: stable_key_for_test(&format!("summary:control_effects:{callable_key}")),
        }
    }

    fn call_site(id: u64, caller: u64) -> CallSiteFact {
        CallSiteFact {
            in_throw: false,
            id: CallSiteId(id),
            language: Language::TypeScript,
            file: FileId::from_raw(1),
            caller: FunctionId::from_raw(caller),
            owner_symbol: None,
            body: MirBodyId(caller),
            operation: MirOpId(id),
            span: span(),
            kind: CallSyntaxKind::Function,
            callee: CallCallee::Identifier {
                reference: None,
                name: format!("call_{id}"),
            },
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Resolved,
            precision: CallPrecision::Exact,
            stable_key: crate::core::StableKeyId(id as u32),
        }
    }

    fn call_target(id: u64, site_id: u64, caller: u64, target_func: u64) -> CallTargetFact {
        CallTargetFact {
            id: CallTargetId(id),
            site: CallSiteId(site_id),
            caller: FunctionId::from_raw(caller),
            target_function: Some(FunctionId::from_raw(target_func)),
            target_symbol: None,
            synthetic_target: None,
            edge_kind: CallEdgeKind::Direct,
            algorithm: CallAlgorithm::DirectReference,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::Native,
            precision: CallPrecision::Exact,
            stable_key: crate::core::StableKeyId(id as u32),
        }
    }

    #[test]
    fn scc_closure_processes_chain_in_correct_order_and_records_demand_trace() {
        // A calls B. Both have direct summaries.
        // SCC closure should process B first (leaf callee), then A.
        let summaries = vec![summary_fact(1, "func::a"), summary_fact(2, "func::b")];
        let sites = vec![call_site(1, 1)]; // A calls something
        let targets = vec![call_target(1, 1, 1, 2)]; // A -> B

        let mut db = AnalysisDb::new();
        db.replace_summary_facts(SummaryOutput {
            summaries,
            events: Vec::new(),
        });
        db.replace_call_facts(CallOutput {
            sites,
            targets,
            unresolved: Vec::new(),
        })
        .expect("call output should be valid");

        let output = run_scc_closure(&mut db);

        // Should have processed 2 SCCs (B and A, both non-recursive)
        let result = output
            .debug_snapshot
            .expect("closure debug snapshot should be present")
            .result;
        assert_eq!(result.total_sccs_processed, 2);
        assert_eq!(result.non_recursive_sccs, 2);
        assert_eq!(result.recursive_sccs, 0);

        // Demand query trace should have entries for each SCC
        assert_eq!(
            output.demand_query_trace.len(),
            2,
            "demand trace should have one entry per SCC"
        );

        // All entries should be scc_closure queries
        for entry in output.demand_query_trace.entries() {
            assert_eq!(entry.query_kind, "scc_closure");
        }

        // No diagnostics for non-recursive SCCs
        assert!(output.diagnostics.is_empty());
    }

    #[test]
    fn scc_closure_skips_empty_schedule() {
        let mut db = AnalysisDb::new();

        let output = run_scc_closure(&mut db);

        assert!(output.debug_snapshot.is_none());
        assert!(output.demand_query_trace.is_empty());
        assert!(output.diagnostics.is_empty());
    }

    #[test]
    fn scc_closure_with_cache_backdates_warm_run_and_records_hit_trace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::default_for_repo(temp.path(), true);
        let summaries = vec![summary_fact(1, "func::a"), summary_fact(2, "func::b")];
        let sites = vec![call_site(1, 1)];
        let targets = vec![call_target(1, 1, 1, 2)];

        let mut cold_db = AnalysisDb::new();
        cold_db.replace_summary_facts(SummaryOutput {
            summaries: summaries.clone(),
            events: Vec::new(),
        });
        cold_db
            .replace_call_facts(CallOutput {
                sites: sites.clone(),
                targets: targets.clone(),
                unresolved: Vec::new(),
            })
            .expect("call output should be valid");
        let cold = run_scc_closure_with_cache(
            &mut cold_db,
            &cache,
            "config-digest",
            "rule-digest",
            "plan-digest",
        );
        assert_eq!(
            cold.debug_snapshot
                .as_ref()
                .expect("cold snapshot")
                .result
                .backdated_sccs,
            0
        );

        let mut warm_db = AnalysisDb::new();
        warm_db.replace_summary_facts(SummaryOutput {
            summaries,
            events: Vec::new(),
        });
        warm_db
            .replace_call_facts(CallOutput {
                sites,
                targets,
                unresolved: Vec::new(),
            })
            .expect("call output should be valid");
        let warm = run_scc_closure_with_cache(
            &mut warm_db,
            &cache,
            "config-digest",
            "rule-digest",
            "plan-digest",
        );

        assert!(
            warm.debug_snapshot
                .as_ref()
                .expect("warm snapshot")
                .result
                .backdated_sccs
                > 0,
            "warm SCC closure should backdate from persisted SCC digests"
        );
        assert!(
            warm.demand_query_trace
                .entries()
                .iter()
                .any(|entry| entry.cache_status == "hit"),
            "warm SCC closure should record hit trace rows: {:#?}",
            warm.demand_query_trace
        );
    }
}
