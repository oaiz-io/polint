//! `polint.solver` provider entry point (D-13, D-15).
//!
//! Mirrors `analysis::semantic_graph::provider`: a `derive_*_with_cache_stats` entry
//! that drives the unified solver engine over the closed input snapshot (the stored
//! `polint.semantic_graph` constraints), emits derived edges + provenance, validates
//! via [`crate::analysis::solver::validate`], persists through
//! [`crate::core::AnalysisDb::replace_solver_facts`], and returns a
//! [`SolverProviderRunOutput`] carrying the output digest (`None` on store error so a
//! cache layer never records a hit for un-persisted state).
//!
//! The output digest (D-15) folds (a) provider/version/schema/parameter digests
//! (the parameter digest already embeds active [`SolverBudget`] knobs), (b) every consumed
//! upstream provider output digest — `polint.semantic_graph`, the points-to source
//! families produced by `polint.type_value_alias` (`points_to_constraints` /
//! `points_to_sets`), and `polint.go.semantic` (whose RTA-signal harvest families the
//! Go RTA policy reads to resolve dynamic dispatch — FIX 4), (c) the [`SolverBudget`]
//! explicitly, and (d) per-row stable keys, then `parts.sort()` + `Digest::from_parts`.
//! Any upstream change, algorithm bump, or active budget change deterministically
//! invalidates the solver cache.

use std::collections::BTreeSet;

use crate::analysis::ids::SemanticNodeId;
use crate::analysis::solver::budget::{BudgetStatus, SolverBudget};
use crate::analysis::solver::engine::SolverEngine;
use crate::analysis::solver::policy::{GoRtaPolicy, SolverPolicy, TsPointsToPolicy};
use crate::analysis::solver::store::{SOLVER_PROVIDER_ID, SolverOutput};
use crate::analysis::solver::validate::{detect_solver_summary_cycle, validate_derived_edges};
use crate::analysis_api::{ProviderExecution, ProviderFailureReason, ProviderFailureStage};
use crate::analysis_kernel::ProviderManifest;
use crate::analysis_kernel::incremental::{CacheStats, Digest, InputSnapshot};
use crate::core::AnalysisDb;
use crate::diagnostics::{Diagnostic, TextRange};
use crate::go::rta::GoRtaInputs;

#[derive(Debug, Clone, Default)]
pub(crate) struct SolverProviderRunOutput {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) cache_stats: CacheStats,
    pub(crate) output_digest: Option<Digest>,
    pub(crate) execution: ProviderExecution,
}

/// `polint.solver` provider entry point (D-13).
///
/// Pipeline mirroring `polint.semantic_graph`:
/// 1. read the closed input snapshot — the stored semantic-graph constraints — and
///    drive [`derive_edges`] over the unified `CopyEdge` vocabulary, bounded by the
///    [`SolverBudget`] (D-11),
/// 2. `normalized()` is applied inside `derive_edges` (stable-key sort),
/// 3. compute the output digest over the stored stable KEYS + upstream digests +
///    the budget (D-15), never run-local dense IDs,
/// 4. validate the derived edges (precision ceiling, dup keys, dangling endpoints)
///    AND run the D-12 cycle-detection check over the input constraints — both
///    surface as evidence-bearing diagnostics, never silent drops,
/// 5. `db.replace_solver_facts(...)` stores + referentially validates,
/// 6. on store error return `output_digest: None`.
pub(crate) fn derive_solver_with_cache_stats(
    db: &mut AnalysisDb,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    budget: SolverBudget,
    semantic_graph_output_digest: Digest,
    type_value_alias_output_digest: Digest,
    go_semantic_output_digest: Digest,
) -> SolverProviderRunOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    debug_assert_eq!(manifest.id, SOLVER_PROVIDER_ID);

    // Step: drive the unified solver ENGINE over the closed input snapshot (D-02,
    // the reserved seam). The points-to CopyEdge closure (`derive_edges`, byte-
    // identical) AND the Go RTA policy's resolved call edges converge into one
    // SolverOutput under one SolverBudget. The build is read-only; the engine
    // normalizes the merged output.
    //
    // The TS policy projects the semantic graph into the shared Andersen domain and
    // is the only indirect-call resolver for JavaScript and TypeScript.
    let constraints = db.semantic_constraints().to_vec();
    let engine = SolverEngine::new(solver_policies_for_db(db, &budget), budget);
    let output = engine.run_to_solver_output(interner, &constraints);

    // Step: digest over the stored stable KEYS + upstream digests + the budget.
    let output_digest = solver_output_digest(
        interner,
        manifest,
        input_snapshot,
        &budget,
        &semantic_graph_output_digest,
        &type_value_alias_output_digest,
        &go_semantic_output_digest,
        &output,
    );

    // Step: validate the derived edges + the D-12 cycle-detection check. These are
    // surfaced as diagnostics; the store performs the hard referential rejection.
    let mut diagnostics = Vec::new();
    let node_ids: BTreeSet<SemanticNodeId> =
        db.semantic_nodes().iter().map(|node| node.id).collect();
    validate_derived_edges(interner, &output.derived_edges, &node_ids, &mut diagnostics);
    detect_solver_summary_cycle(&constraints, &mut diagnostics);
    // Surface budget exhaustion as an honest diagnostic (D-06): when the solver
    // truncated any source's closure under the per-source step budget, the run is
    // flagged rather than presenting a silently-truncated edge set as complete.
    if output.budget_status == BudgetStatus::BudgetExceeded {
        diagnostics.push(budget_exceeded_diagnostic(&output.budget_reasons));
    }

    let mut cache_stats = CacheStats::default();
    cache_stats.record_recompute();

    // Step: store (assigns dense IDs + referentially validates inside
    // from_output). On store error the db keeps its prior state and the facts the
    // digest certifies were not persisted, so return output_digest: None.
    match db.replace_solver_facts(output) {
        Ok(()) => SolverProviderRunOutput {
            diagnostics,
            cache_stats,
            output_digest: Some(output_digest),
            execution: Default::default(),
        },
        Err(error) => {
            diagnostics.push(provider_error_diagnostic(error.to_string()));
            SolverProviderRunOutput {
                diagnostics,
                cache_stats,
                output_digest: None,
                execution: ProviderExecution::Failed {
                    stage: ProviderFailureStage::Validation,
                    reason: ProviderFailureReason::ValidationRejected,
                },
            }
        }
    }
}

fn solver_policies_for_db(db: &AnalysisDb, _budget: &SolverBudget) -> Vec<Box<dyn SolverPolicy>> {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let policies: Vec<Box<dyn SolverPolicy>> = vec![
        // The real Go RTA policy (GO-05): contributes resolved call edges.
        Box::new(GoRtaPolicy::new(GoRtaInputs::from_db(interner, db))),
        Box::new(TsPointsToPolicy::new(
            super::policy::ts_points_to_inputs_from_db(db),
        )),
    ];
    policies
}

/// Output digest over stable KEYS + upstream digests + budget (D-15), never dense
/// IDs.
///
/// Folds (a) provider/version/schema/parameter digests (the parameter digest already
/// embeds the budget via `solver_provider_parameter_digest`), (b) the consumed
/// upstream provider output digests — `polint.semantic_graph`, the points-to source
/// families carried by `polint.type_value_alias`'s output digest, and
/// `polint.go.semantic`'s output digest (the RTA-signal families the Go RTA policy
/// reads — FIX 4), (c) the `SolverBudget` explicitly (belt-and-suspenders with the
/// parameter digest so a budget change is unmissable, D-15), and (d) per-row stable
/// keys + status/precision + provenance fragment, then `parts.sort()` +
/// `Digest::from_parts`.
#[allow(clippy::too_many_arguments)]
fn solver_output_digest(
    interner: &crate::core::StableKeyInterner,
    manifest: &ProviderManifest,
    _input_snapshot: &InputSnapshot,
    budget: &SolverBudget,
    semantic_graph_output_digest: &Digest,
    type_value_alias_output_digest: &Digest,
    go_semantic_output_digest: &Digest,
    output: &SolverOutput,
) -> Digest {
    crate::analysis_neutral::solver::digest::solver_output_digest(
        interner,
        manifest,
        budget,
        semantic_graph_output_digest,
        type_value_alias_output_digest,
        go_semantic_output_digest,
        output,
    )
}

/// Honest budget-exhaustion signal (D-06): emitted when the solver truncated a
/// source's transitive closure under the per-source step budget. The downstream
/// unknown taxonomy categorizes it; surviving derived edges keep their own honest
/// status and precision.
fn budget_exceeded_diagnostic(budget_reasons: &BTreeSet<String>) -> Diagnostic {
    let mut diagnostic = Diagnostic::warning(
        "polint/internal",
        "<workspace>",
        TextRange::point(1, 1),
        "Solver budget exceeded; some potential derived edges may be missing, while \
         surviving derived edges keep their own status and precision.",
    )
    .with_evidence("provider", SOLVER_PROVIDER_ID)
    .with_evidence(crate::diagnostics::BUDGET_EVIDENCE_LABEL, "solver_steps")
    .with_evidence(
        crate::diagnostics::BUDGET_STATUS_EVIDENCE_LABEL,
        BudgetStatus::BudgetExceeded.as_str(),
    );
    for reason in budget_reasons {
        diagnostic = diagnostic.with_evidence("budget_reason", reason.clone());
    }
    diagnostic
}

fn provider_error_diagnostic(message: String) -> Diagnostic {
    Diagnostic::error(
        "polint/internal",
        "<workspace>",
        TextRange::point(1, 1),
        "Solver analysis failed; solver-derived edges were not stored.",
    )
    .with_evidence("provider", SOLVER_PROVIDER_ID)
    .with_evidence("reason", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::adaptation::budget::AdaptationModelBudget;
    use crate::analysis::ids::SemanticConstraintId;
    use crate::analysis::points_to::facts::{PointsToPrecision, PointsToStatus};
    use crate::analysis::semantic_graph::constraints::{ConstraintFact, ConstraintKind};
    use crate::analysis::semantic_graph::facts::{NodeKind, SemanticNodeFact, SemanticPrecision};
    use crate::analysis::semantic_graph::provider::derive_semantic_graph_with_cache_stats;
    use crate::analysis::semantic_graph::store::SEMANTIC_GRAPH_PROVIDER_ID;
    use crate::analysis::semantic_graph::store::SemanticGraphOutput;
    use crate::analysis::solver::budget::{BudgetReason, SolverBudget};
    use crate::analysis_kernel::AnalysisKernel;
    use crate::analysis_kernel::incremental::{Digest, DigestKind};
    use crate::analysis_plan::AnalysisPlan;
    use crate::config::{LoadedConfig, load_config};
    use crate::core::{AnalysisDb, FunctionId};
    use std::fs;
    use tempfile::tempdir;

    fn manifest() -> &'static ProviderManifest {
        AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == SOLVER_PROVIDER_ID)
            .expect("solver manifest")
    }

    fn semantic_graph_manifest() -> &'static ProviderManifest {
        AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == SEMANTIC_GRAPH_PROVIDER_ID)
            .expect("semantic graph manifest")
    }

    fn snapshot(db: &AnalysisDb) -> InputSnapshot {
        snapshot_with_config(db, "")
    }

    fn snapshot_with_config(db: &AnalysisDb, config: &str) -> InputSnapshot {
        let temp = tempdir().expect("tempdir");
        fs::write(temp.path().join(".polint.toml"), config).expect("config");
        let loaded = load_config(temp.path()).expect("config loads");
        snapshot_from_loaded(db, &loaded)
    }

    fn snapshot_from_loaded(db: &AnalysisDb, loaded: &LoadedConfig) -> InputSnapshot {
        crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            loaded,
            db,
            "config-a",
            "rules-a",
            AnalysisPlan::empty().digest(),
            AnalysisKernel::provider_manifests(),
        )
    }

    fn solver_budget_from_loaded(loaded: &LoadedConfig) -> SolverBudget {
        SolverBudget {
            go: loaded.config.solver.to_go_sub_budget(),
            ..SolverBudget::default()
        }
    }

    fn semantic_graph_digest_for_loaded(
        db: &mut AnalysisDb,
        loaded: &LoadedConfig,
    ) -> Option<Digest> {
        let snapshot = snapshot_from_loaded(db, loaded);
        derive_semantic_graph_with_cache_stats(
            db,
            loaded,
            AdaptationModelBudget::default(),
            &snapshot,
            semantic_graph_manifest(),
            absent("polint.calls"),
            absent("polint.identity"),
            absent("polint.abstract_domains"),
            absent("polint.entrypoints"),
            absent("polint.reachability"),
            absent("polint.type_value_alias"),
            absent("polint.symbol_graph"),
            absent("polint.module_topology"),
            absent("polint.go.syntax"),
            absent("polint.ts.syntax"),
            absent("polint.semantic_mir"),
            absent("polint.go.semantic"),
        )
        .output_digest
    }

    fn absent(kind: &str) -> Digest {
        Digest::absent(DigestKind::ProviderOutput, kind)
    }

    fn node(
        interner: &crate::core::StableKeyInterner,
        id: u64,
        stable_key: &str,
    ) -> SemanticNodeFact {
        SemanticNodeFact {
            id: SemanticNodeId(id),
            kind: NodeKind::Function(FunctionId::from_raw(id)),
            precision: SemanticPrecision::Conservative,
            stable_key: interner.intern(stable_key),
        }
    }

    fn copy(
        interner: &crate::core::StableKeyInterner,
        src: u64,
        dst: u64,
        stable_key: &str,
    ) -> ConstraintFact {
        ConstraintFact {
            id: SemanticConstraintId(0),
            kind: ConstraintKind::CopyEdge {
                dst: SemanticNodeId(dst),
                src: SemanticNodeId(src),
            },
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            stable_key: interner.intern(stable_key),
        }
    }

    /// Builds a db with a small semantic graph: a chain a -> b -> c so the solver
    /// derives the transitive edge a -> c.
    fn db_with_copy_chain() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let interner = db.stable_key_interner();
        let graph = SemanticGraphOutput {
            nodes: vec![
                node(&interner, 0, "node|function|a"),
                node(&interner, 1, "node|function|b"),
                node(&interner, 2, "node|function|c"),
            ],
            edges: Vec::new(),
            constraints: vec![
                copy(&interner, 0, 1, "constraint|copy|a-b"),
                copy(&interner, 1, 2, "constraint|copy|b-c"),
            ],
        };
        db.replace_semantic_graph_facts(graph)
            .expect("semantic graph stores");
        db
    }

    fn run(db: &mut AnalysisDb) -> SolverProviderRunOutput {
        let snapshot = snapshot(db);
        derive_solver_with_cache_stats(
            db,
            &snapshot,
            manifest(),
            SolverBudget::default(),
            absent("polint.semantic_graph"),
            absent("polint.type_value_alias"),
            absent("polint.go.semantic"),
        )
    }

    #[test]
    fn empty_db_produces_empty_output_sentinel_digest() {
        let mut db = AnalysisDb::new();
        let output = run(&mut db);
        assert!(output.output_digest.is_some());
        assert!(output.diagnostics.is_empty());
        assert!(db.solver_derived_edges().is_empty());
    }

    #[test]
    fn populated_db_derives_transitive_edge_and_stores() {
        let mut db = db_with_copy_chain();
        let output = run(&mut db);
        assert!(output.output_digest.is_some());
        assert!(
            db.solver_derived_edges()
                .iter()
                .any(|e| e.source == SemanticNodeId(0) && e.target == SemanticNodeId(2)),
            "expected the transitive a -> c edge stored"
        );
    }

    #[test]
    fn semantic_graph_upstream_digest_change_invalidates_output_digest() {
        let mut db_a = db_with_copy_chain();
        let snapshot = snapshot(&db_a);
        let with_absent = derive_solver_with_cache_stats(
            &mut db_a,
            &snapshot,
            manifest(),
            SolverBudget::default(),
            absent("polint.semantic_graph"),
            absent("polint.type_value_alias"),
            absent("polint.go.semantic"),
        )
        .output_digest;

        let mut db_b = db_with_copy_chain();
        let with_present = derive_solver_with_cache_stats(
            &mut db_b,
            &snapshot,
            manifest(),
            SolverBudget::default(),
            Digest::from_parts(
                DigestKind::ProviderOutput,
                "polint.semantic_graph",
                &["changed"],
            ),
            absent("polint.type_value_alias"),
            absent("polint.go.semantic"),
        )
        .output_digest;

        assert_ne!(with_absent, with_present);
    }

    #[test]
    fn points_to_source_family_digest_change_invalidates_output_digest() {
        // The points-to source families are carried by polint.type_value_alias's
        // output digest; changing it invalidates the solver output (D-15).
        let mut db_a = db_with_copy_chain();
        let snapshot = snapshot(&db_a);
        let base = derive_solver_with_cache_stats(
            &mut db_a,
            &snapshot,
            manifest(),
            SolverBudget::default(),
            absent("polint.semantic_graph"),
            absent("polint.type_value_alias"),
            absent("polint.go.semantic"),
        )
        .output_digest;

        let mut db_b = db_with_copy_chain();
        let changed = derive_solver_with_cache_stats(
            &mut db_b,
            &snapshot,
            manifest(),
            SolverBudget::default(),
            absent("polint.semantic_graph"),
            Digest::from_parts(
                DigestKind::ProviderOutput,
                "polint.type_value_alias",
                &["changed"],
            ),
            absent("polint.go.semantic"),
        )
        .output_digest;

        assert_ne!(base, changed);

        let mut bumped_points_to = SolverBudget::default();
        bumped_points_to.points_to.max_objects_per_var += 1;
        let mut db_c = db_with_copy_chain();
        let points_to_changed = derive_solver_with_cache_stats(
            &mut db_c,
            &snapshot,
            manifest(),
            bumped_points_to,
            absent("polint.semantic_graph"),
            absent("polint.type_value_alias"),
            absent("polint.go.semantic"),
        )
        .output_digest;

        assert_ne!(
            base, points_to_changed,
            "changing the TS points-to budget must invalidate the solver output digest"
        );
    }

    #[test]
    fn go_semantic_upstream_digest_change_invalidates_output_digest() {
        // FIX 4: the Go RTA policy reads the stored polint.go.semantic RTA-signal families
        // (instantiated_types / address_taken / dynamic_dispatch / method_sets) to resolve
        // dynamic dispatch, so a Go edit touching ONLY those families changes the resolved
        // edges and MUST invalidate the solver cache. A change to the consumed
        // polint.go.semantic output digest therefore changes the solver output digest.
        let mut db_a = db_with_copy_chain();
        let snapshot = snapshot(&db_a);
        let base = derive_solver_with_cache_stats(
            &mut db_a,
            &snapshot,
            manifest(),
            SolverBudget::default(),
            absent("polint.semantic_graph"),
            absent("polint.type_value_alias"),
            absent("polint.go.semantic"),
        )
        .output_digest;

        let mut db_b = db_with_copy_chain();
        let changed = derive_solver_with_cache_stats(
            &mut db_b,
            &snapshot,
            manifest(),
            SolverBudget::default(),
            absent("polint.semantic_graph"),
            absent("polint.type_value_alias"),
            Digest::from_parts(
                DigestKind::ProviderOutput,
                "polint.go.semantic",
                &["changed"],
            ),
        )
        .output_digest;

        assert_ne!(
            base, changed,
            "a polint.go.semantic output-digest change must invalidate the solver output digest"
        );
    }

    #[test]
    fn budget_change_invalidates_output_digest() {
        // D-15: active SolverBudget knobs participate in the output digest.
        let mut db_a = db_with_copy_chain();
        let snapshot = snapshot(&db_a);
        let base = derive_solver_with_cache_stats(
            &mut db_a,
            &snapshot,
            manifest(),
            SolverBudget::default(),
            absent("polint.semantic_graph"),
            absent("polint.type_value_alias"),
            absent("polint.go.semantic"),
        )
        .output_digest;

        let mut bumped = SolverBudget::default();
        bumped.max_steps += 1;
        let mut db_b = db_with_copy_chain();
        let changed = derive_solver_with_cache_stats(
            &mut db_b,
            &snapshot,
            manifest(),
            bumped,
            absent("polint.semantic_graph"),
            absent("polint.type_value_alias"),
            absent("polint.go.semantic"),
        )
        .output_digest;

        assert_ne!(base, changed);
    }

    #[test]
    fn inactive_solver_config_changes_do_not_invalidate_output_digest() {
        let mut db_a = db_with_copy_chain();
        let base_snapshot = snapshot_with_config(&db_a, "");
        let base = derive_solver_with_cache_stats(
            &mut db_a,
            &base_snapshot,
            manifest(),
            SolverBudget::default(),
            absent("polint.semantic_graph"),
            absent("polint.type_value_alias"),
            absent("polint.go.semantic"),
        )
        .output_digest;

        let mut db_b = db_with_copy_chain();
        let inactive_object_snapshot = snapshot_with_config(
            &db_b,
            r#"
[solver.js]
max_object_receiver_candidates_per_callsite = 127
"#,
        );
        let changed = derive_solver_with_cache_stats(
            &mut db_b,
            &inactive_object_snapshot,
            manifest(),
            SolverBudget::default(),
            absent("polint.semantic_graph"),
            absent("polint.type_value_alias"),
            absent("polint.go.semantic"),
        )
        .output_digest;

        assert_eq!(
            base, changed,
            "inactive solver config changes must not invalidate solver output"
        );
    }

    #[test]
    fn inactive_solver_config_changes_do_not_flow_through_semantic_graph_digest() {
        let base_temp = tempdir().expect("tempdir");
        fs::write(base_temp.path().join(".polint.toml"), "").expect("config");
        let base_loaded = load_config(base_temp.path()).expect("config loads");
        let mut base_semantic_db = AnalysisDb::new();
        let base_semantic_digest =
            semantic_graph_digest_for_loaded(&mut base_semantic_db, &base_loaded);

        let changed_temp = tempdir().expect("tempdir");
        fs::write(
            changed_temp.path().join(".polint.toml"),
            r#"
[solver.js]
max_object_receiver_candidates_per_callsite = 127
"#,
        )
        .expect("config");
        let changed_loaded = load_config(changed_temp.path()).expect("config loads");
        let mut changed_semantic_db = AnalysisDb::new();
        let changed_semantic_digest =
            semantic_graph_digest_for_loaded(&mut changed_semantic_db, &changed_loaded);

        assert_eq!(
            base_semantic_digest, changed_semantic_digest,
            "disabled object-model caps must not invalidate semantic graph output"
        );

        let mut base_solver_db = AnalysisDb::new();
        let base_snapshot = snapshot_from_loaded(&base_solver_db, &base_loaded);
        let base = derive_solver_with_cache_stats(
            &mut base_solver_db,
            &base_snapshot,
            manifest(),
            solver_budget_from_loaded(&base_loaded),
            base_semantic_digest.expect("semantic digest"),
            absent("polint.type_value_alias"),
            absent("polint.go.semantic"),
        )
        .output_digest;

        let mut changed_solver_db = AnalysisDb::new();
        let changed_snapshot = snapshot_from_loaded(&changed_solver_db, &changed_loaded);
        let changed = derive_solver_with_cache_stats(
            &mut changed_solver_db,
            &changed_snapshot,
            manifest(),
            solver_budget_from_loaded(&changed_loaded),
            changed_semantic_digest.expect("semantic digest"),
            absent("polint.type_value_alias"),
            absent("polint.go.semantic"),
        )
        .output_digest;

        assert_eq!(
            base, changed,
            "inactive solver config changes must not invalidate solver through semantic graph"
        );
    }

    #[test]
    fn production_registers_one_ts_points_to_policy() {
        let db = AnalysisDb::new();
        let policies = solver_policies_for_db(&db, &SolverBudget::default());

        assert_eq!(policies.len(), 2);
        assert_eq!(
            policies
                .iter()
                .map(|policy| policy.id())
                .collect::<Vec<_>>(),
            vec!["go_rta", "ts_points_to"]
        );
    }

    #[test]
    fn adaptation_model_budget_change_invalidates_output_digest() {
        let mut db_a = db_with_copy_chain();
        let snapshot = snapshot(&db_a);
        let base = derive_solver_with_cache_stats(
            &mut db_a,
            &snapshot,
            manifest(),
            SolverBudget::default(),
            absent("polint.semantic_graph"),
            absent("polint.type_value_alias"),
            absent("polint.go.semantic"),
        )
        .output_digest;

        let bumped = SolverBudget {
            adaptation: AdaptationModelBudget {
                max_model_derived_edges: SolverBudget::default().adaptation.max_model_derived_edges
                    + 1,
                ..AdaptationModelBudget::default()
            },
            ..SolverBudget::default()
        };
        let mut db_b = db_with_copy_chain();
        let changed = derive_solver_with_cache_stats(
            &mut db_b,
            &snapshot,
            manifest(),
            bumped,
            absent("polint.semantic_graph"),
            absent("polint.type_value_alias"),
            absent("polint.go.semantic"),
        )
        .output_digest;

        assert_ne!(
            base, changed,
            "changing an adaptation-model budget knob must invalidate solver output"
        );
    }

    #[test]
    fn run_level_budget_status_invalidates_output_digest() {
        // WR-06: two outputs with the SAME surviving edge set but different run-level
        // budget_status (WithinBudget vs BudgetExceeded) must produce DIFFERENT output
        // digests, so a cache layer keyed on the digest cannot serve a truncated
        // (BudgetExceeded) result under a complete (WithinBudget) run's digest.
        let db = db_with_copy_chain();
        let snapshot = snapshot(&db);
        let budget = SolverBudget::default();

        // Identical (empty) edge sets; only the run-level status differs.
        let within = SolverOutput {
            derived_edges: Vec::new(),
            budget_status: BudgetStatus::WithinBudget,
            budget_reasons: BTreeSet::new(),
        };
        let exceeded = SolverOutput {
            derived_edges: Vec::new(),
            budget_status: BudgetStatus::BudgetExceeded,
            budget_reasons: BTreeSet::from([BudgetReason::SolverMaxSteps.as_str().to_string()]),
        };

        let interner = crate::core::test_stable_key_interner();
        let within_digest = solver_output_digest(
            &interner,
            manifest(),
            &snapshot,
            &budget,
            &absent("polint.semantic_graph"),
            &absent("polint.type_value_alias"),
            &absent("polint.go.semantic"),
            &within,
        );
        let exceeded_digest = solver_output_digest(
            &interner,
            manifest(),
            &snapshot,
            &budget,
            &absent("polint.semantic_graph"),
            &absent("polint.type_value_alias"),
            &absent("polint.go.semantic"),
            &exceeded,
        );

        assert_ne!(
            within_digest, exceeded_digest,
            "a budget-truncated run must not share an output digest with a complete run"
        );
    }

    #[test]
    fn provider_manifests_list_solver_between_semantic_graph_and_refined_calls() {
        let manifests = AnalysisKernel::provider_manifests();
        let semantic_graph = manifests
            .iter()
            .position(|manifest| manifest.id == "polint.semantic_graph")
            .expect("semantic_graph present");
        assert_eq!(manifests[semantic_graph + 1].id, "polint.solver");
        assert_eq!(manifests[semantic_graph + 2].id, "polint.refined_calls");
    }
}
