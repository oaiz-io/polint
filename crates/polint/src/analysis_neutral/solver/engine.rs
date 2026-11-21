//! Unified solver engine: deterministic worklist core (D-02, D-11).
//!
//! The engine owns the single deterministic `VecDeque` worklist that drives the
//! registered [`super::policy::SolverPolicy`] impls to a **single fixpoint per
//! run** (D-11), enforces the [`SolverBudget`] ceilings, projects the outcome to
//! [`BudgetStatus`], and maintains a monotonic `u64` step counter used as the provenance solver-step
//! field (D-08). The worklist drain mirrors
//! the proven `points_to::solver` shape (`while let Some(..) = queue.pop_front()`
//! with `if !budget_ok { break; }`) and `BTreeMap`/`BTreeSet`-ordered
//! accumulation, which is what makes the output byte-stable (D-02).
//!
//! The points-to fixpoint is folded in by composition (D-03): the engine drives
//! the [`super::policy::PointsToPolicy`], which invokes the existing
//! `solve_points_to` engine in place. Driving the points-to policy through this
//! engine produces a result byte-identical to calling `solve_points_to`
//! directly.
//!
//! Production uses [`SolverEngine::run_to_solver_output`] to merge the model edges
//! ([`derive_model_edges`]) with edge-contributing language policies such as Go RTA
//! and JS/TS function tokens. This keeps multiple sub-domains under one budget and
//! one normalization path.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::analysis_api::{FactFamily, stable_key_from_parts};
use crate::analysis_neutral::ids::DerivedEdgeId;
use crate::analysis_neutral::points_to::facts::{PointsToPrecision, PointsToStatus};
use crate::analysis_neutral::semantic_graph::constraints::{ConstraintFact, ConstraintKind};

use super::budget::{BudgetReason, BudgetStatus, SolverBudget};
use super::facts::DerivedEdgeFact;
use super::policy::{SolverPolicy, SolverPolicyOutcome};
use super::provenance::{ContributingFact, DerivedEdgeProvenance};
use super::store::SolverOutput;

/// Result of a single unified solver run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolverRunResult {
    /// Per-policy outcomes, in the deterministic order the engine drained them.
    pub policy_outcomes: Vec<PolicyRunRecord>,
    /// The combined budget outcome across all driven policies (worst-case wins:
    /// any policy exceeding its budget surfaces [`BudgetStatus::BudgetExceeded`]).
    pub budget_status: BudgetStatus,
    /// Monotonic worklist step counter for this run (provenance solver-step).
    pub steps: u64,
    /// Stable reason labels for every policy or engine-level budget ceiling exhausted.
    pub budget_reasons: BTreeSet<String>,
}

/// One policy's contribution recorded by the engine, tagged by policy id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRunRecord {
    pub policy_id: &'static str,
    pub outcome: SolverPolicyOutcome,
}

/// The unified solver engine. Holds a closed, ordered set of registered policies
/// (the closed input set, D-11) and drives them to convergence with a single
/// deterministic worklist drain.
///
/// Production drives the Go and JS/TS policies through
/// [`Self::run_to_solver_output`] (see module docs).
pub struct SolverEngine {
    policies: Vec<Box<dyn SolverPolicy>>,
    budget: SolverBudget,
}

impl SolverEngine {
    /// Build an engine over a closed, ordered policy set. Order is the
    /// registration order; the worklist drains it deterministically.
    pub fn new(policies: Vec<Box<dyn SolverPolicy>>, budget: SolverBudget) -> Self {
        Self { policies, budget }
    }

    /// Drive every registered policy to its single fixpoint, bounded by the
    /// budget, accumulating outcomes in deterministic order (D-02). The monotonic
    /// step counter increments once per worklist step and latches budget
    /// exhaustion honestly (D-06/D-11) rather than looping unbounded.
    pub fn run(&self, interner: &crate::internal_core::StableKeyInterner) -> SolverRunResult {
        // Deterministic worklist of policy indices in stable registration order.
        // The bounded outer-iteration cap (D-11) guards against draining more
        // policies than the budget allows; policy-internal work contributes to
        // `steps` for provenance but must not consume this outer cap.
        let mut queue: VecDeque<usize> = (0..self.policies.len()).collect();
        let mut outer_iterations: u64 = 0;
        let mut steps: u64 = 0;
        let mut budget_exceeded = false;
        let mut budget_reasons = BTreeSet::new();
        let mut records: Vec<PolicyRunRecord> = Vec::with_capacity(self.policies.len());

        while let Some(index) = queue.pop_front() {
            // Bounded outer-iteration cap: each policy drained is one outer
            // iteration; exceeding the cap latches exhaustion instead of looping.
            outer_iterations += 1;
            if outer_iterations > self.budget.max_outer_iterations as u64 {
                budget_exceeded = true;
                budget_reasons.insert(BudgetReason::SolverMaxOuterIterations.as_str().to_string());
                break;
            }

            let policy = &self.policies[index];
            let outcome = policy.solve(interner, &self.budget);
            if outcome.budget_status == BudgetStatus::BudgetExceeded {
                budget_exceeded = true;
                budget_reasons.extend(outcome.budget_reasons.iter().cloned());
            }
            // Fold the policy's own internal step count into the run counter so
            // provenance records the full worklist progress.
            steps = steps.saturating_add(1 + outcome.steps);
            records.push(PolicyRunRecord {
                policy_id: policy.id(),
                outcome,
            });
        }

        let budget_status = if budget_exceeded {
            BudgetStatus::BudgetExceeded
        } else if records.is_empty() {
            BudgetStatus::NotRun
        } else {
            BudgetStatus::WithinBudget
        };

        SolverRunResult {
            policy_outcomes: records,
            budget_status,
            steps,
            budget_reasons,
        }
    }

    /// Drive every registered policy and the model edges into ONE merged
    /// [`SolverOutput`] under one [`SolverBudget`].
    ///
    /// 1. the `ModelEdge` constraints become derived edges via
    ///    [`derive_model_edges`], bounded by the adaptation caps;
    /// 2. the registered policies are driven via [`Self::run`]; each policy's
    ///    `derived_edges` are collected (the JS/TS points-to policy contributes the
    ///    call edges it resolves; the Go RTA policy contributes its resolved call
    ///    edges);
    /// 3. the edge sets are concatenated, the run-level `budget_status` is the
    ///    worst-case across the model edges and the EDGE-CONTRIBUTING policies, and the
    ///    result is `normalized()` (dense IDs only after the stable-key sort), which
    ///    keeps the merged output byte-stable under input shuffle. A points-to POLICY
    ///    fold (`outcome.points_to` set) contributes no edge here, so its own Andersen
    ///    object/var-budget exhaustion is EXCLUDED from the run-level status: a
    ///    discarded fold must not pollute the diagnostic or bust the output digest.
    ///
    /// The engine owns the single-fixpoint-per-run / bounded-outer-iteration contract
    /// (each policy runs one fixpoint; `run`'s `max_outer_iterations` bounds the
    /// policy drain).
    pub fn run_to_solver_output(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        constraints: &[ConstraintFact],
    ) -> SolverOutput {
        // Step 1: the model edges.
        let model_output = derive_model_edges(interner, constraints, &self.budget);

        // Step 2: drive the registered policies (points-to fold + Go RTA) and collect
        // their derived edges.
        let run = self.run(interner);
        let mut derived_edges = model_output.derived_edges;
        for record in &run.policy_outcomes {
            derived_edges.extend(record.outcome.derived_edges.iter().cloned());
        }

        // Step 3: worst-case budget combine + normalize (stable-key sort, dense IDs). The
        // run-level status must reflect ONLY sub-domains whose edges actually enter THIS
        // output: the step-1 model edges (`model_output`) and the policies whose
        // derived edges were collected above. The points-to POLICY fold (identified by
        // `outcome.points_to.is_some()`) emits NO edge here, so its own Andersen
        // object/var-budget exhaustion must NOT pollute the run-level status: a
        // discarded fold reporting
        // BudgetExceeded would be a misleading diagnostic AND spuriously bust the
        // solver_output_digest (WR-06 folds budget_status). Exclude it from the combine;
        // every edge-contributing policy (e.g. Go RTA) still surfaces its exhaustion.
        let policy_budget_status = run
            .policy_outcomes
            .iter()
            .filter(|record| record.outcome.points_to.is_none())
            .map(|record| record.outcome.budget_status)
            .fold(BudgetStatus::NotRun, combine_budget_status);
        let mut budget_reasons = model_output.budget_reasons;
        for record in &run.policy_outcomes {
            if record.outcome.points_to.is_none() {
                budget_reasons.extend(record.outcome.budget_reasons.iter().cloned());
            }
        }
        if run
            .budget_reasons
            .contains(BudgetReason::SolverMaxOuterIterations.as_str())
        {
            budget_reasons.insert(BudgetReason::SolverMaxOuterIterations.as_str().to_string());
        }
        let engine_budget_status = if run
            .budget_reasons
            .contains(BudgetReason::SolverMaxOuterIterations.as_str())
        {
            BudgetStatus::BudgetExceeded
        } else {
            BudgetStatus::NotRun
        };
        let budget_status = combine_budget_status(
            combine_budget_status(model_output.budget_status, policy_budget_status),
            engine_budget_status,
        );
        let incomplete_sites = run
            .policy_outcomes
            .iter()
            .filter(|record| record.outcome.points_to.is_none())
            .fold(model_output.incomplete_sites, |sum, record| {
                sum.saturating_add(record.outcome.incomplete_sites)
            });
        SolverOutput {
            derived_edges,
            budget_status,
            budget_reasons,
            incomplete_sites,
        }
        .normalized(interner)
    }
}

/// Worst-case combine of two run-level budget statuses for the merged solver output:
/// any `BudgetExceeded` wins (an exhausted sub-domain is never masked); otherwise
/// `WithinBudget` if either ran; `NotRun` only if neither did. `NotRun` from the
/// policy run (empty policy set) does not mask a real points-to signal.
fn combine_budget_status(a: BudgetStatus, b: BudgetStatus) -> BudgetStatus {
    if a == BudgetStatus::BudgetExceeded || b == BudgetStatus::BudgetExceeded {
        BudgetStatus::BudgetExceeded
    } else if a == BudgetStatus::WithinBudget || b == BudgetStatus::WithinBudget {
        BudgetStatus::WithinBudget
    } else {
        BudgetStatus::NotRun
    }
}

/// The derived edges of the `ConstraintKind::ModelEdge` constraints, bounded by the
/// adaptation caps and normalized.
///
/// The solver stores no transitive closure of `CopyEdge`s. Value flow is solved by
/// the points-to policy, and the call edges it resolves are the solver's answer. A
/// closure over every pair of nodes a copy chain connects grows with the square of
/// the chain and is read by no consumer, so it is not built.
pub fn derive_model_edges(
    interner: &crate::internal_core::StableKeyInterner,
    constraints: &[ConstraintFact],
    budget: &SolverBudget,
) -> SolverOutput {
    let (derived_edges, budget_exceeded, budget_reasons) =
        model_edges(interner, constraints, budget);
    let budget_status = if budget_exceeded {
        BudgetStatus::BudgetExceeded
    } else {
        BudgetStatus::WithinBudget
    };
    SolverOutput {
        derived_edges,
        budget_status,
        budget_reasons,
        incomplete_sites: 0,
    }
    .normalized(interner)
}

fn model_edges(
    interner: &crate::internal_core::StableKeyInterner,
    constraints: &[ConstraintFact],
    budget: &SolverBudget,
) -> (Vec<DerivedEdgeFact>, bool, BTreeSet<String>) {
    let mut edges = Vec::new();
    let mut budget_exceeded = false;
    let mut budget_reasons = BTreeSet::new();
    let mut caps = AdaptationCaps::new(budget);
    let mut model_constraints = constraints
        .iter()
        .filter_map(|constraint| match &constraint.kind {
            ConstraintKind::ModelEdge {
                source,
                target,
                evidence,
                ..
            } => Some((
                constraint,
                source,
                target,
                crate::analysis_neutral::adaptation::platform::is_platform_model_edge(evidence),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    {
        let keys = interner.read_view();
        model_constraints.sort_by(|left, right| {
            (keys.text(left.0.stable_key), left.1.0, left.2.0).cmp(&(
                keys.text(right.0.stable_key),
                right.1.0,
                right.2.0,
            ))
        });
    }

    for (constraint, source, target, platform) in model_constraints {
        // The adaptation caps bound repo-local model facts. Platform model
        // edges follow points-to sets the solver budget already bounds.
        if !platform
            && let Err(reason) =
                caps.admit(&interner.resolve(constraint.stable_key), source.0, target.0)
        {
            budget_exceeded = true;
            budget_reasons.insert(reason.as_str().to_string());
            continue;
        }
        let provenance = DerivedEdgeProvenance::new(
            interner,
            [ContributingFact {
                stable_key: constraint.stable_key,
            }],
            &constraint.kind,
            0,
        );
        let stable_key = stable_key_from_parts(
            interner,
            FactFamily::SolverDerivedEdge,
            &[
                ("source", source.0.to_string()),
                ("target", target.0.to_string()),
                ("provenance", provenance.stable_key_fragment(interner)),
            ],
        );
        edges.push(DerivedEdgeFact {
            id: DerivedEdgeId(0),
            source: *source,
            target: *target,
            status: constraint.status,
            precision: constraint.precision,
            stable_key,
            provenance,
            claim: crate::analysis_neutral::points_to::tiers::EdgeClaim::Unassigned,
        });
    }
    (edges, budget_exceeded, budget_reasons)
}

/// Counters for the adaptation caps, which bound the edges repo-local model
/// facts may add.
struct AdaptationCaps<'a> {
    budget: &'a SolverBudget,
    edges: usize,
    targets_by_source: BTreeMap<SemanticNodeRef, BTreeSet<SemanticNodeRef>>,
    expansions_by_model: BTreeMap<String, usize>,
}

impl<'a> AdaptationCaps<'a> {
    fn new(budget: &'a SolverBudget) -> Self {
        Self {
            budget,
            edges: 0,
            targets_by_source: BTreeMap::new(),
            expansions_by_model: BTreeMap::new(),
        }
    }

    /// Counts the edge against the caps, or names the cap it would exceed.
    fn admit(
        &mut self,
        model: &str,
        source: SemanticNodeRef,
        target: SemanticNodeRef,
    ) -> Result<(), BudgetReason> {
        let caps = &self.budget.adaptation;
        if self.edges >= caps.max_model_derived_edges {
            return Err(BudgetReason::AdaptationMaxModelDerivedEdges);
        }
        let expansions = self
            .expansions_by_model
            .entry(model.to_string())
            .or_default();
        if *expansions >= caps.max_expansions_per_model {
            return Err(BudgetReason::AdaptationMaxExpansionsPerModel);
        }
        let targets = self.targets_by_source.entry(source).or_default();
        if !targets.contains(&target) && targets.len() >= caps.max_targets_per_source {
            return Err(BudgetReason::AdaptationMaxTargetsPerSource);
        }
        *expansions += 1;
        targets.insert(target);
        self.edges += 1;
        Ok(())
    }
}

/// Run-local node handle (the `SemanticNodeId.0` value) used as a deterministic
/// `BTreeMap`/`BTreeSet` key by the adaptation caps.
type SemanticNodeRef = u64;

/// Severity rank of a derivation status (#4): `Present` is the best/most-trusted (0);
/// higher is less trusted. The total order drives both `weakest_status` and the
/// worst-path fixpoint comparison, so it is the single source of severity ordering.
///
/// `pub(crate)` so the Go RTA dispatch resolver (`go_rta::dispatch`) reuses the SAME
/// severity ordering for its worst-trust edge status (D-09), rather than minting a
/// parallel ranking.
pub fn status_rank(status: PointsToStatus) -> u8 {
    match status {
        PointsToStatus::Present => 0,
        PointsToStatus::Unknown => 1,
        PointsToStatus::Unsupported => 2,
        PointsToStatus::SetupMissing => 3,
        PointsToStatus::BudgetExceeded => 4,
        PointsToStatus::IndirectionBound => 5,
    }
}

/// Severity rank of a precision tier (#4): `FlowInsensitive` is the most precise a
/// derived edge may claim (0); higher is weaker. `pub(crate)` for the same Go RTA
/// reuse as [`status_rank`].
pub fn precision_rank(precision: PointsToPrecision) -> u8 {
    match precision {
        PointsToPrecision::FlowInsensitive => 0,
        PointsToPrecision::LocalFlowSensitive => 1,
        PointsToPrecision::SummaryProjected => 2,
        PointsToPrecision::Heuristic => 3,
        PointsToPrecision::Unknown => 4,
        PointsToPrecision::Unsupported => 5,
    }
}

/// Worst-of-two derivation status (#4): the more severe (higher-ranked) one wins.
/// Deterministic and independent of input order. `pub(crate)` so the Go RTA dispatch
/// resolver inherits the WEAKEST status across its adopted derivation (D-09).
pub fn weakest_status(a: PointsToStatus, b: PointsToStatus) -> PointsToStatus {
    if status_rank(a) >= status_rank(b) {
        a
    } else {
        b
    }
}

/// Worst-of-two precision (#4): the weaker (higher-ranked) tier wins. Deterministic
/// and independent of input order. `pub(crate)` for the same Go RTA reuse as
/// [`weakest_status`].
pub fn weakest_precision(a: PointsToPrecision, b: PointsToPrecision) -> PointsToPrecision {
    if precision_rank(a) >= precision_rank(b) {
        a
    } else {
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::ids::{
        ObjectTokenId, PointsToConstraintId, PtVarId, SemanticNodeId,
    };
    use crate::analysis_neutral::points_to::facts::{
        PointsToConstraintFact, PointsToConstraintKind, PointsToPrecision, PointsToStatus,
    };
    use crate::analysis_neutral::points_to::solver::{PointsToBudget, solve_points_to};
    use crate::analysis_neutral::solver::policy::PointsToPolicy;

    fn constraint(stable_key: &str, kind: PointsToConstraintKind) -> PointsToConstraintFact {
        PointsToConstraintFact {
            id: PointsToConstraintId(0),
            kind,
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    fn sample_constraints() -> Vec<PointsToConstraintFact> {
        vec![
            constraint(
                "addr-a",
                PointsToConstraintKind::AddressOf {
                    dst: PtVarId(1),
                    object: ObjectTokenId(10),
                },
            ),
            constraint(
                "copy",
                PointsToConstraintKind::Copy {
                    dst: PtVarId(2),
                    src: PtVarId(1),
                },
            ),
            constraint(
                "field-store",
                PointsToConstraintKind::FieldStore {
                    base: PtVarId(1),
                    field: "name".to_string(),
                    src: PtVarId(2),
                },
            ),
            constraint(
                "field-load",
                PointsToConstraintKind::FieldLoad {
                    dst: PtVarId(3),
                    base: PtVarId(1),
                    field: "name".to_string(),
                },
            ),
        ]
    }

    #[test]
    fn points_to_via_engine_equals_solve_points_to() {
        // The fold (D-03) preserves behavior: driving the points-to policy through
        // the unified engine yields exactly the standalone `solve_points_to`
        // result over the same constraints.
        let constraints = sample_constraints();
        let budget = SolverBudget::default();

        let direct = solve_points_to(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            PointsToBudget::default(),
        );

        let policy = PointsToPolicy::new(constraints);
        let engine = SolverEngine::new(vec![Box::new(policy)], budget);
        let run = engine.run(&crate::internal_core::test_stable_key_interner());

        let via_engine = run.policy_outcomes[0]
            .outcome
            .points_to
            .clone()
            .expect("points-to policy yields a folded result");

        assert_eq!(via_engine, direct);
    }

    #[test]
    fn engine_is_deterministic_across_two_runs() {
        let constraints = sample_constraints();
        let budget = SolverBudget::default();

        let first = SolverEngine::new(
            vec![Box::new(PointsToPolicy::new(constraints.clone()))],
            budget,
        )
        .run(&crate::internal_core::test_stable_key_interner());
        let second = SolverEngine::new(vec![Box::new(PointsToPolicy::new(constraints))], budget)
            .run(&crate::internal_core::test_stable_key_interner());

        assert_eq!(first, second);
        assert_eq!(first.budget_status, BudgetStatus::WithinBudget);
    }

    #[test]
    fn engine_surfaces_budget_exhaustion_honestly() {
        // A tight per-sub-domain object cap forces the points-to fixpoint to latch
        // exhaustion; the engine projects it to BudgetStatus::BudgetExceeded
        // rather than dropping silently (D-06).
        let constraints = vec![
            constraint(
                "addr-a",
                PointsToConstraintKind::AddressOf {
                    dst: PtVarId(1),
                    object: ObjectTokenId(10),
                },
            ),
            constraint(
                "addr-b",
                PointsToConstraintKind::AddressOf {
                    dst: PtVarId(1),
                    object: ObjectTokenId(11),
                },
            ),
        ];
        let mut budget = SolverBudget::default();
        budget.points_to.max_objects_per_var = 1;

        let engine = SolverEngine::new(vec![Box::new(PointsToPolicy::new(constraints))], budget);
        let run = engine.run(&crate::internal_core::test_stable_key_interner());

        assert_eq!(run.budget_status, BudgetStatus::BudgetExceeded);
    }
    #[derive(Clone)]
    struct FixedPolicy {
        id: &'static str,
        outcome: SolverPolicyOutcome,
    }

    impl SolverPolicy for FixedPolicy {
        fn id(&self) -> &'static str {
            self.id
        }

        fn solve(
            &self,
            _interner: &crate::internal_core::StableKeyInterner,
            _budget: &SolverBudget,
        ) -> SolverPolicyOutcome {
            self.outcome.clone()
        }
    }

    fn policy_edge(stable_key: &str, source: u64, target: u64) -> DerivedEdgeFact {
        let interner = crate::internal_core::test_stable_key_interner();
        let source = SemanticNodeId(source);
        let target = SemanticNodeId(target);
        let provenance = DerivedEdgeProvenance::new(
            &interner,
            [ContributingFact::from_parts(
                &interner,
                FactFamily::ExtensionFact,
                &[("constraint", stable_key.to_string())],
            )],
            &ConstraintKind::ModelEdge {
                source,
                target,
                language: "typescript".to_string(),
                scope: "src/app.ts".to_string(),
                confidence: "heuristic".to_string(),
                evidence: vec!["src/app.ts:1".to_string()],
            },
            1,
        );
        DerivedEdgeFact {
            id: DerivedEdgeId(0),
            source,
            target,
            status: PointsToStatus::Present,
            precision: PointsToPrecision::Heuristic,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
            provenance,
            claim: crate::analysis_neutral::points_to::tiers::EdgeClaim::Unassigned,
        }
    }

    #[test]
    fn policy_internal_steps_do_not_consume_outer_iteration_budget() {
        let budget = SolverBudget {
            max_outer_iterations: 2,
            ..SolverBudget::default()
        };
        let first = FixedPolicy {
            id: "high_internal_work",
            outcome: SolverPolicyOutcome {
                steps: budget.max_outer_iterations as u64 + 1,
                ..SolverPolicyOutcome::empty()
            },
        };
        let second = FixedPolicy {
            id: "edge_producer",
            outcome: SolverPolicyOutcome {
                derived_edges: vec![policy_edge("edge-after-high-internal-work", 1, 2)],
                steps: 0,
                ..SolverPolicyOutcome::empty()
            },
        };
        let engine = SolverEngine::new(vec![Box::new(first), Box::new(second)], budget);

        let run = engine.run(&crate::internal_core::test_stable_key_interner());

        assert_eq!(run.policy_outcomes.len(), 2);
        assert_eq!(run.policy_outcomes[1].policy_id, "edge_producer");
        assert_eq!(run.policy_outcomes[1].outcome.derived_edges.len(), 1);
        assert_eq!(run.budget_status, BudgetStatus::WithinBudget);
        assert!(
            !run.budget_reasons
                .contains(BudgetReason::SolverMaxOuterIterations.as_str())
        );
    }

    #[test]
    fn empty_engine_reports_not_run() {
        let engine = SolverEngine::new(Vec::new(), SolverBudget::default());
        let run = engine.run(&crate::internal_core::test_stable_key_interner());
        assert_eq!(run.budget_status, BudgetStatus::NotRun);
        assert!(run.policy_outcomes.is_empty());
    }

    fn copy_constraint(stable_key: &str, src: u64, dst: u64) -> ConstraintFact {
        use crate::analysis_neutral::ids::{SemanticConstraintId, SemanticNodeId};
        ConstraintFact {
            id: SemanticConstraintId(0),
            kind: ConstraintKind::CopyEdge {
                dst: SemanticNodeId(dst),
                src: SemanticNodeId(src),
            },
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    fn model_constraint(stable_key: &str, source: u64, target: u64) -> ConstraintFact {
        use crate::analysis_neutral::ids::{SemanticConstraintId, SemanticNodeId};
        ConstraintFact {
            id: SemanticConstraintId(0),
            kind: ConstraintKind::ModelEdge {
                source: SemanticNodeId(source),
                target: SemanticNodeId(target),
                language: "typescript".to_string(),
                scope: "src/app.ts".to_string(),
                confidence: "heuristic".to_string(),
                evidence: vec!["src/app.ts:10".to_string()],
            },
            status: PointsToStatus::Present,
            precision: PointsToPrecision::Heuristic,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    #[test]
    fn solver_model_edge_provenance_is_private_and_deterministic() {
        let output = derive_model_edges(
            &crate::internal_core::test_stable_key_interner(),
            &[model_constraint("model|register", 10, 20)],
            &SolverBudget::default(),
        );

        let edge = output
            .derived_edges
            .iter()
            .find(|edge| edge.source == SemanticNodeId(10) && edge.target == SemanticNodeId(20))
            .expect("model edge derived");
        assert_eq!(edge.provenance.constraint_kind, "model_edge");
        assert_eq!(edge.provenance.contributing_len(), 1);
        let interner = crate::internal_core::test_stable_key_interner();
        assert_eq!(
            interner
                .resolve(edge.provenance.contributing_facts[0].stable_key)
                .as_ref(),
            "model|register"
        );
        assert!(interner.resolve(edge.stable_key).contains("model_edge"));
    }

    #[test]
    fn solver_model_edge_cache_identity_changes_with_model_input() {
        let first = derive_model_edges(
            &crate::internal_core::test_stable_key_interner(),
            &[model_constraint("model|register-a", 10, 20)],
            &SolverBudget::default(),
        );
        let second = derive_model_edges(
            &crate::internal_core::test_stable_key_interner(),
            &[model_constraint("model|register-b", 10, 20)],
            &SolverBudget::default(),
        );

        let first_key = &first.derived_edges[0].stable_key;
        let second_key = &second.derived_edges[0].stable_key;
        assert_ne!(
            first_key, second_key,
            "model constraint stable keys must affect derived edge identity"
        );
    }

    #[test]
    fn solver_model_edges_respect_derived_edge_budget() {
        let mut budget = SolverBudget::default();
        budget.adaptation.max_model_derived_edges = 1;
        let output = derive_model_edges(
            &crate::internal_core::test_stable_key_interner(),
            &[
                model_constraint("model|register-a", 10, 20),
                model_constraint("model|register-b", 11, 21),
            ],
            &budget,
        );

        assert_eq!(output.derived_edges.len(), 1);
        assert_eq!(output.budget_status, BudgetStatus::BudgetExceeded);
    }

    #[test]
    fn solver_model_edges_respect_per_source_target_fanout_budget() {
        let mut budget = SolverBudget::default();
        budget.adaptation.max_targets_per_source = 1;
        let output = derive_model_edges(
            &crate::internal_core::test_stable_key_interner(),
            &[
                model_constraint("model|register-a", 10, 20),
                model_constraint("model|register-b", 10, 21),
            ],
            &budget,
        );

        assert_eq!(output.derived_edges.len(), 1);
        assert_eq!(output.derived_edges[0].source, SemanticNodeId(10));
        assert_eq!(output.budget_status, BudgetStatus::BudgetExceeded);
    }

    fn platform_model_constraint(stable_key: &str, source: u64, target: u64) -> ConstraintFact {
        let mut constraint = model_constraint(stable_key, source, target);
        if let ConstraintKind::ModelEdge { evidence, .. } = &mut constraint.kind {
            *evidence = vec![
                format!(
                    "{}Function.prototype.apply",
                    crate::analysis_neutral::adaptation::platform::EVIDENCE_PREFIX
                ),
                crate::analysis_neutral::adaptation::platform::digest_evidence(),
            ];
        }
        constraint
    }

    #[test]
    fn solver_platform_model_edges_are_not_held_to_adaptation_caps() {
        let budget = SolverBudget::default();
        let fanout = budget.adaptation.max_targets_per_source as u64 + 4;
        let mut constraints = (0..fanout)
            .map(|index| platform_model_constraint(&format!("platform|{index}"), 10, 100 + index))
            .collect::<Vec<_>>();
        constraints.extend(
            (0..fanout).map(|index| model_constraint(&format!("model|{index}"), 11, 200 + index)),
        );
        let output = derive_model_edges(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            &budget,
        );

        let from = |source| {
            output
                .derived_edges
                .iter()
                .filter(|edge| edge.source == SemanticNodeId(source))
                .count()
        };
        assert_eq!(from(10), fanout as usize);
        assert_eq!(from(11), budget.adaptation.max_targets_per_source);
        assert_eq!(output.budget_status, BudgetStatus::BudgetExceeded);
        assert_eq!(
            output.budget_reasons.iter().collect::<Vec<_>>(),
            [BudgetReason::AdaptationMaxTargetsPerSource.as_str()]
        );
    }

    #[test]
    fn solver_model_edge_with_a_foreign_platform_digest_keeps_adaptation_caps() {
        let mut budget = SolverBudget::default();
        budget.adaptation.max_targets_per_source = 1;
        let forged = |key: &str, target| {
            let mut constraint = platform_model_constraint(key, 10, target);
            if let ConstraintKind::ModelEdge { evidence, .. } = &mut constraint.kind {
                evidence[1] = "platform_digest:other".to_string();
            }
            constraint
        };
        let output = derive_model_edges(
            &crate::internal_core::test_stable_key_interner(),
            &[forged("model|a", 20), forged("model|b", 21)],
            &budget,
        );

        assert_eq!(output.derived_edges.len(), 1);
        assert_eq!(output.budget_status, BudgetStatus::BudgetExceeded);
    }

    #[test]
    fn discarded_points_to_fold_does_not_pollute_run_level_budget_status() {
        // The registered `PointsToPolicy`'s OWN Andersen solve contributes ZERO edges
        // to the output (its result rides `outcome.points_to`, not
        // `outcome.derived_edges`). So if that discarded Andersen solve exhausts its
        // object budget while the model edges complete, the merged run-level
        // budget_status must still be WithinBudget: the discarded fold's exhaustion is
        // a misleading signal (no edge it produced is in the output) and would also
        // spuriously bust the solver output digest.
        let exhausting_points_to = vec![
            constraint(
                "addr-a",
                PointsToConstraintKind::AddressOf {
                    dst: PtVarId(1),
                    object: ObjectTokenId(10),
                },
            ),
            constraint(
                "addr-b",
                PointsToConstraintKind::AddressOf {
                    dst: PtVarId(1),
                    object: ObjectTokenId(11),
                },
            ),
        ];
        // A model edge that completes, beside a copy chain the solver derives no
        // edge from.
        let constraints = vec![
            model_constraint("model|register", 10, 20),
            copy_constraint("copy|a-b", 1, 2),
            copy_constraint("copy|b-c", 2, 3),
        ];

        // A registered points-to policy may exhaust its own fold while the model
        // edges that contribute to the output still complete within budget.
        let mut budget = SolverBudget::default();
        budget.points_to.max_objects_per_var = 1;
        let engine = SolverEngine::new(
            vec![Box::new(PointsToPolicy::new(exhausting_points_to))],
            budget,
        );

        let output = engine.run_to_solver_output(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
        );

        let pairs = output
            .derived_edges
            .iter()
            .map(|edge| (edge.source, edge.target))
            .collect::<Vec<_>>();
        assert_eq!(
            pairs,
            [(SemanticNodeId(10), SemanticNodeId(20))],
            "the model edge is derived and the copy chain derives nothing"
        );
        // The discarded Andersen fold's exhaustion must NOT surface at run level.
        assert_eq!(
            output.budget_status,
            BudgetStatus::WithinBudget,
            "a discarded points-to fold's budget exhaustion must not pollute run-level status"
        );
    }
    #[test]
    fn derive_model_edges_is_shuffle_stable() {
        let constraints = vec![
            model_constraint("model|a-b", 1, 2),
            model_constraint("model|b-c", 2, 3),
            model_constraint("model|c-d", 3, 4),
        ];
        let mut shuffled = constraints.clone();
        shuffled.reverse();

        let a = derive_model_edges(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            &SolverBudget::default(),
        );
        let b = derive_model_edges(
            &crate::internal_core::test_stable_key_interner(),
            &shuffled,
            &SolverBudget::default(),
        );

        let interner = crate::internal_core::test_stable_key_interner();
        let a_json = crate::analysis_neutral::solver::facts::serialize_derived_edges_stable(
            &interner,
            &a.derived_edges,
        )
        .expect("serialize a");
        let b_json = crate::analysis_neutral::solver::facts::serialize_derived_edges_stable(
            &interner,
            &b.derived_edges,
        )
        .expect("serialize b");
        assert_eq!(a_json, b_json);
    }

    #[test]
    fn derive_model_edges_stable_payload_ignores_reverse_intern_allocation_order() {
        let forward_interner = crate::internal_core::StableKeyInterner::default();
        let reverse_interner = crate::internal_core::StableKeyInterner::default();
        let forward = vec![
            model_constraint_on(&forward_interner, "model|a-b", 1, 2),
            model_constraint_on(&forward_interner, "model|b-c", 2, 3),
        ];
        let reverse = vec![
            model_constraint_on(&reverse_interner, "model|b-c", 2, 3),
            model_constraint_on(&reverse_interner, "model|a-b", 1, 2),
        ];
        let forward_ab = forward[0].stable_key;
        let reverse_ab = reverse[1].stable_key;
        assert_eq!(
            forward_interner.resolve(forward_ab).as_ref(),
            reverse_interner.resolve(reverse_ab).as_ref()
        );
        assert_ne!(
            forward_ab.0, reverse_ab.0,
            "same text must receive different allocation ids across reverse-interned tables"
        );

        let a = derive_model_edges(&forward_interner, &forward, &SolverBudget::default());
        let b = derive_model_edges(&reverse_interner, &reverse, &SolverBudget::default());
        let a_json = crate::analysis_neutral::solver::facts::serialize_derived_edges_stable(
            &forward_interner,
            &a.derived_edges,
        )
        .expect("serialize a");
        let b_json = crate::analysis_neutral::solver::facts::serialize_derived_edges_stable(
            &reverse_interner,
            &b.derived_edges,
        )
        .expect("serialize b");
        assert_eq!(a_json, b_json);
    }

    fn model_constraint_on(
        interner: &crate::internal_core::StableKeyInterner,
        stable_key: &str,
        source: u64,
        target: u64,
    ) -> ConstraintFact {
        let mut constraint = model_constraint(stable_key, source, target);
        constraint.stable_key = interner.intern(stable_key);
        constraint
    }
}
