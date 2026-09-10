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
//! Production uses [`SolverEngine::run_to_solver_output`] to merge the free
//! [`derive_edges`] CopyEdge closure with edge-contributing language policies such as
//! Go RTA and JS/TS function tokens. This keeps multiple sub-domains under one budget
//! and one normalization path.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use crate::analysis_api::{FactFamily, stable_key_from_parts};
use crate::analysis_neutral::ids::DerivedEdgeId;
use crate::analysis_neutral::points_to::facts::{PointsToPrecision, PointsToStatus};
use crate::analysis_neutral::semantic_graph::constraints::{ConstraintFact, ConstraintKind};
use crate::internal_core::StableKeyId;

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
/// Reserved orchestration: production calls [`derive_edges`] directly; this engine is
/// the seam concrete policies route through once Go/TS policies exist (see module docs).
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

    /// Drive every registered policy AND the points-to `CopyEdge` closure into ONE
    /// merged [`SolverOutput`] under one [`SolverBudget`] (D-02, the reserved-seam
    /// composition concrete policies realize).
    ///
    /// Composition, NOT rewrite (the acceptance bar is points-to byte-identity):
    /// 1. the points-to transitive `CopyEdge` closure is computed via the existing
    ///    [`derive_edges`] over `copy_edge_constraints`, UNCHANGED — so its derived
    ///    edges and fixtures stay byte-identical (`points_to_via_engine_equals_solve_
    ///    points_to` / `derive_edges_is_shuffle_stable` prove this);
    /// 2. the registered policies are driven via [`Self::run`]; each policy's
    ///    `derived_edges` are collected (the points-to policy contributes none — its
    ///    edges are the closure in step 1; the Go RTA policy contributes its resolved
    ///    call edges);
    /// 3. the two edge sets are concatenated, the run-level `budget_status` is the
    ///    worst-case across the closure and the EDGE-CONTRIBUTING policies, and the result
    ///    is `normalized()` (dense IDs only after the stable-key sort, D-02), which is
    ///    what keeps the merged output byte-stable under input shuffle. The points-to
    ///    POLICY fold contributes no edge here (its closure is the step-1 `derive_edges`
    ///    output), so its own Andersen object/var-budget exhaustion is EXCLUDED from the
    ///    run-level status — a discarded fold must not pollute the diagnostic or bust the
    ///    output digest (FINDING 3 / WR-06).
    ///
    /// The engine still owns the single-fixpoint-per-run / bounded-outer-iteration
    /// contract (each policy runs one fixpoint; `run`'s `max_outer_iterations` bounds
    /// the policy drain; `derive_edges` is per-source bounded by `max_steps`).
    pub fn run_to_solver_output(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        copy_edge_constraints: &[ConstraintFact],
    ) -> SolverOutput {
        // Step 1: the points-to CopyEdge closure, byte-identical to today's production.
        let points_to_output = derive_edges(interner, copy_edge_constraints, &self.budget);

        // Step 2: drive the registered policies (points-to fold + Go RTA) and collect
        // their derived edges.
        let run = self.run(interner);
        let mut derived_edges = points_to_output.derived_edges;
        for record in &run.policy_outcomes {
            derived_edges.extend(record.outcome.derived_edges.iter().cloned());
        }

        // Step 3: worst-case budget combine + normalize (stable-key sort, dense IDs). The
        // run-level status must reflect ONLY sub-domains whose edges actually enter THIS
        // output: the step-1 CopyEdge closure (`points_to_output`) and the policies whose
        // derived edges were collected above. The points-to POLICY fold (identified by
        // `outcome.points_to.is_some()`) emits NO edge here — its points-to closure is the
        // step-1 `derive_edges` output — so its own Andersen object/var-budget exhaustion
        // must NOT pollute the run-level status (FINDING 3): a discarded fold reporting
        // BudgetExceeded would be a misleading diagnostic AND spuriously bust the
        // solver_output_digest (WR-06 folds budget_status). Exclude it from the combine;
        // every edge-contributing policy (e.g. Go RTA) still surfaces its exhaustion.
        let policy_budget_status = run
            .policy_outcomes
            .iter()
            .filter(|record| record.outcome.points_to.is_none())
            .map(|record| record.outcome.budget_status)
            .fold(BudgetStatus::NotRun, combine_budget_status);
        let mut budget_reasons = points_to_output.budget_reasons;
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
            combine_budget_status(points_to_output.budget_status, policy_budget_status),
            engine_budget_status,
        );
        SolverOutput {
            derived_edges,
            budget_status,
            budget_reasons,
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

/// Derive transitive copy edges over the unified `ConstraintKind::CopyEdge`
/// vocabulary, attaching full [`DerivedEdgeProvenance`] (GRAPH-04, D-08) to each
/// derived edge.
///
/// A `CopyEdge { dst, src }` constraint is a primitive value-flow obligation
/// `src -> dst`. The unified solver derives the TRANSITIVE closure: if `a -> b` and
/// `b -> c` are primitive copy edges, the solver derives `a -> c`. Each derived edge
/// records, as its provenance, the totally-ordered set of contributing primitive
/// `CopyEdge` constraints (referenced by their EXISTING stable keys) whose
/// composition produced it, the producing `ConstraintKind` (`copy_edge`), and the
/// `solver_step` at which it was emitted.
///
/// Provenance records the edge's WORST-TRUST derivation (D-09). The per-source BFS is
/// a worst-path fixpoint: each `source -> target` edge carries the contributing
/// primitive `CopyEdge` constraints on the least-trusted path discovered to the
/// target (ties broken deterministically by BFS-shortest + `BTreeMap`/`BTreeSet`
/// stable-key order), with every constraint justifying a hop on that path recorded
/// (duplicate justifications for one hop are all kept — review finding #10). Because
/// `keys`, `status`, and `precision` all describe the same adopted derivation, the
/// edge's provenance always JUSTIFIES its status. The edge fact's `stable_key` embeds
/// that derivation, so deleting any contributing fact means THIS edge fact (by stable
/// key) is not reproduced. On a multi-path graph the underlying value-flow may persist
/// via a different path under a different edge identity. Dense IDs are assigned only
/// after the stable-key sort in [`SolverOutput::normalized`], so the output is
/// byte-stable (D-02).
///
/// Honesty (D-06): status/precision are the WEAKEST across every discovered path to
/// the target, and adopting a weaker path re-enqueues the node so the downgrade
/// PROPAGATES transitively to descendants — an untrusted upstream hop on any
/// derivation can never be laundered into a confident edge, even multiple hops
/// downstream (review findings #4, #R2). The worklist cap (`budget.max_steps`) is
/// applied PER SOURCE; exhausting it on one source sets the run-level
/// [`crate::analysis_neutral::solver::budget::BudgetStatus::BudgetExceeded`] (surfaced as a
/// provider diagnostic) WITHOUT silently dropping the other, independent sources
/// (review finding #3). Edges fully derived before a source's cap was hit keep their
/// honest status — exhaustion costs the edges never reached, not the ones already
/// derived (review finding #R1). `solver_step` is a GLOBAL monotonic (non-decreasing)
/// counter, independent of the per-source budget counter (review finding #R3). Derived
/// edges never claim exact precision (D-06).
pub fn derive_edges(
    interner: &crate::internal_core::StableKeyInterner,
    constraints: &[ConstraintFact],
    budget: &SolverBudget,
) -> SolverOutput {
    // Primitive copy adjacency `src -> {dst}`. For each hop `src -> dst` accumulate
    // (a) the UNION of contributing constraint stable keys (#10 — never drop a
    // justifying identity) and (b) the WEAKEST status/precision across the
    // constraints asserting it (#4 — never launder an untrusted hop).
    // Adjacency/endpoints stay BTree-ordered by SemanticNodeRef. Hop key sets are
    // membership-only (`HashSet<StableKeyId>`); ordered emission sorts by resolved
    // text inside `DerivedEdgeProvenance::new` (never StableKeyId allocation order).
    let mut adjacency: BTreeMap<SemanticNodeRef, BTreeSet<SemanticNodeRef>> = BTreeMap::new();
    let mut hop_keys: BTreeMap<(SemanticNodeRef, SemanticNodeRef), HashSet<StableKeyId>> =
        BTreeMap::new();
    let mut hop_meta: BTreeMap<
        (SemanticNodeRef, SemanticNodeRef),
        (PointsToStatus, PointsToPrecision),
    > = BTreeMap::new();
    for constraint in constraints {
        if let ConstraintKind::CopyEdge { dst, src } = &constraint.kind {
            let (src, dst) = (src.0, dst.0);
            adjacency.entry(src).or_default().insert(dst);
            hop_keys
                .entry((src, dst))
                .or_default()
                .insert(constraint.stable_key);
            let meta = hop_meta
                .entry((src, dst))
                .or_insert((PointsToStatus::Present, PointsToPrecision::FlowInsensitive));
            meta.0 = weakest_status(meta.0, constraint.status);
            meta.1 = weakest_precision(meta.1, constraint.precision);
        }
    }

    let mut edges: Vec<DerivedEdgeFact> = Vec::new();
    let (model_edges, model_budget_exceeded, model_budget_reasons) =
        model_edges(interner, constraints, budget);
    edges.extend(model_edges);
    let mut run_budget_exceeded = model_budget_exceeded;
    let mut budget_reasons = model_budget_reasons;
    // GLOBAL monotonic derivation step (R3): increments once per worklist pop across
    // the WHOLE run and is never reset, so every derived edge's `solver_step` is
    // globally monotonic and the run's progress is totally ordered. The PER-SOURCE
    // budget counter below is separate, so the per-source budget (#3) does not
    // perturb this contract.
    let mut solver_step: u64 = 0;

    let sources: Vec<SemanticNodeRef> = adjacency.keys().copied().collect();
    for start in sources {
        // PER-SOURCE budget (#3): reset the budget counter for each source so one
        // large source cannot starve the others. Exhaustion is recorded run-level,
        // never as a silent cross-source drop.
        let mut budget_steps: u64 = 0;
        let mut source_budget_exceeded = false;
        // reached: node -> path metadata for its WORST-trust derivation. A derivation
        // is adopted iff it is strictly more conservative (weaker) than the one already
        // recorded, and adoption re-enqueues the node so the downgrade PROPAGATES to
        // descendants (a worst-path fixpoint, R2/round-3). Equal-weakness ties keep the
        // first (BFS-shortest) path, so all-trusted graphs behave exactly like a plain
        // shortest-path BFS (byte-identical) while mixed-trust graphs converge to the
        // least-trusted derivation — `keys`/`step` then describe the SAME derivation as
        // `status`/`precision`, so an edge's provenance always justifies its status.
        let mut reached: BTreeMap<SemanticNodeRef, PathMeta> = BTreeMap::new();
        let mut queue: VecDeque<SemanticNodeRef> = VecDeque::new();
        reached.insert(start, PathMeta::seed());
        queue.push_back(start);

        while let Some(node) = queue.pop_front() {
            budget_steps += 1;
            solver_step += 1;
            if budget_steps > budget.max_steps as u64 {
                source_budget_exceeded = true;
                budget_reasons.insert(BudgetReason::SolverMaxSteps.as_str().to_string());
                break;
            }
            let path = reached.get(&node).cloned().unwrap_or_else(PathMeta::seed);
            let Some(targets) = adjacency.get(&node) else {
                continue;
            };
            for &next in targets {
                if next == start {
                    // Skip self-loops back to the start (cycle guard).
                    continue;
                }
                let mut next_meta = path.clone();
                if let Some(keys) = hop_keys.get(&(node, next)) {
                    next_meta.keys.extend(keys.iter().cloned());
                }
                if let Some((status, precision)) = hop_meta.get(&(node, next)) {
                    next_meta.status = weakest_status(next_meta.status, *status);
                    next_meta.precision = weakest_precision(next_meta.precision, *precision);
                }
                next_meta.step = solver_step;
                // Worst-path fixpoint (R2/round-3): adopt this derivation for `next` iff
                // the node is unreached OR this path is STRICTLY weaker (more
                // conservative) than the one already recorded. Adoption re-enqueues the
                // node so the downgrade propagates transitively to descendants already
                // derived via an earlier, more-trusted path. Ties (equal weakness) keep
                // the first path — deterministic, and weakness increases monotonically
                // and is bounded, so re-enqueues terminate.
                let candidate_weakness = (
                    status_rank(next_meta.status),
                    precision_rank(next_meta.precision),
                );
                let adopt = match reached.get(&next) {
                    None => true,
                    Some(current) => {
                        candidate_weakness
                            > (
                                status_rank(current.status),
                                precision_rank(current.precision),
                            )
                    }
                };
                if adopt {
                    reached.insert(next, next_meta);
                    queue.push_back(next);
                }
            }
        }

        if source_budget_exceeded {
            run_budget_exceeded = true;
        }

        // Emit a derived edge for every node reachable from `start` in >= 1 hop. Every
        // node in `reached` was traversed via a COMPLETE witness path, so its edge is
        // sound and keeps its honest (conservatively weakened) status/precision even
        // when this source later exhausted the budget — budget exhaustion costs the
        // edges we never reached, which is conveyed by the run-level signal, NOT a
        // downgrade of the edges we did derive (R1). Derived edges are at most
        // flow-insensitive, never exact (D-06).
        for (&node, meta) in &reached {
            if node == start || meta.keys.is_empty() {
                continue;
            }
            let provenance = DerivedEdgeProvenance::new(
                interner,
                meta.keys
                    .iter()
                    .map(|&key| ContributingFact { stable_key: key }),
                &ConstraintKind::CopyEdge {
                    dst: crate::analysis_neutral::ids::SemanticNodeId(node),
                    src: crate::analysis_neutral::ids::SemanticNodeId(start),
                },
                meta.step,
            );
            let stable_key = stable_key_from_parts(
                interner,
                FactFamily::SolverDerivedEdge,
                &[
                    ("source", start.to_string()),
                    ("target", node.to_string()),
                    ("provenance", provenance.stable_key_fragment(interner)),
                ],
            );
            edges.push(DerivedEdgeFact {
                id: DerivedEdgeId(0),
                source: crate::analysis_neutral::ids::SemanticNodeId(start),
                target: crate::analysis_neutral::ids::SemanticNodeId(node),
                status: meta.status,
                precision: meta.precision,
                stable_key,
                provenance,
            });
        }
        // No `break`: every source is processed so budget exhaustion on one source
        // never silently drops the independent edges of another (#3).
    }

    let budget_status = if run_budget_exceeded {
        BudgetStatus::BudgetExceeded
    } else {
        BudgetStatus::WithinBudget
    };

    SolverOutput {
        derived_edges: edges,
        budget_status,
        budget_reasons,
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
    let mut targets_by_source: BTreeMap<SemanticNodeRef, BTreeSet<SemanticNodeRef>> =
        BTreeMap::new();
    let mut expansions_by_model: BTreeMap<String, usize> = BTreeMap::new();
    let mut model_constraints = constraints
        .iter()
        .filter_map(|constraint| match &constraint.kind {
            ConstraintKind::ModelEdge { source, target, .. } => Some((constraint, source, target)),
            _ => None,
        })
        .collect::<Vec<_>>();
    model_constraints.sort_by(|left, right| {
        (interner.resolve(left.0.stable_key), left.1.0, left.2.0).cmp(&(
            interner.resolve(right.0.stable_key),
            right.1.0,
            right.2.0,
        ))
    });

    for (constraint, source, target) in model_constraints {
        if edges.len() >= budget.adaptation.max_model_derived_edges {
            budget_exceeded = true;
            budget_reasons.insert(
                BudgetReason::AdaptationMaxModelDerivedEdges
                    .as_str()
                    .to_string(),
            );
            continue;
        }

        let expansions = expansions_by_model
            .entry(interner.resolve(constraint.stable_key).to_string())
            .or_default();
        if *expansions >= budget.adaptation.max_expansions_per_model {
            budget_exceeded = true;
            budget_reasons.insert(
                BudgetReason::AdaptationMaxExpansionsPerModel
                    .as_str()
                    .to_string(),
            );
            continue;
        }

        let source_targets = targets_by_source.entry(source.0).or_default();
        if !source_targets.contains(&target.0)
            && source_targets.len() >= budget.adaptation.max_targets_per_source
        {
            budget_exceeded = true;
            budget_reasons.insert(
                BudgetReason::AdaptationMaxTargetsPerSource
                    .as_str()
                    .to_string(),
            );
            continue;
        }

        *expansions += 1;
        source_targets.insert(target.0);
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
        });
    }
    (edges, budget_exceeded, budget_reasons)
}

/// Run-local node handle (the `SemanticNodeId.0` value) used as a deterministic
/// `BTreeMap`/`BTreeSet` key during closure derivation.
type SemanticNodeRef = u64;

/// Path metadata for a node's adopted (WORST-trust) derivation. All four fields
/// describe the SAME derivation: `keys` (the union of contributing constraint stable
/// keys), `status`/`precision` (its trust), and `step` (the global monotonic step at
/// which that derivation reached the node). A strictly-weaker derivation discovered
/// later replaces the whole record (and re-enqueues the node); equal-weakness ties
/// keep the first.
///
/// `keys` is membership-only (`HashSet<StableKeyId>`). When an edge is emitted,
/// [`DerivedEdgeProvenance::new`] sorts contributing identities by resolved stable
/// text — never by StableKeyId allocation order.
#[derive(Debug, Clone)]
struct PathMeta {
    keys: HashSet<StableKeyId>,
    status: PointsToStatus,
    precision: PointsToPrecision,
    /// The global monotonic `solver_step` at which this node's adopted derivation
    /// reached it.
    step: u64,
}

impl PathMeta {
    /// The start node's seed: zero contributing facts, fully trusted, step 0.
    fn seed() -> Self {
        Self {
            keys: HashSet::new(),
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            step: 0,
        }
    }
}

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
    fn derive_edges_emits_transitive_copy_edge_with_provenance() {
        // Chain a -> b -> c: the solver derives the transitive edge a -> c whose
        // provenance carries BOTH contributing copy constraints.
        let constraints = vec![
            copy_constraint("copy|a-b", 1, 2),
            copy_constraint("copy|b-c", 2, 3),
        ];
        let output = derive_edges(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            &SolverBudget::default(),
        );

        use crate::analysis_neutral::ids::SemanticNodeId;
        let transitive = output
            .derived_edges
            .iter()
            .find(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(3))
            .expect("transitive a -> c derived");

        assert_eq!(transitive.provenance.constraint_kind, "copy_edge");
        assert_eq!(transitive.provenance.contributing_len(), 2);
        assert!(transitive.provenance.solver_step > 0);
    }

    #[test]
    fn solver_model_edge_provenance_is_private_and_deterministic() {
        let output = derive_edges(
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
        let first = derive_edges(
            &crate::internal_core::test_stable_key_interner(),
            &[model_constraint("model|register-a", 10, 20)],
            &SolverBudget::default(),
        );
        let second = derive_edges(
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
        let output = derive_edges(
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
        let output = derive_edges(
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

    #[test]
    fn derive_edges_propagates_weakest_contributing_status_and_precision() {
        // a -> b is a clean Present/FlowInsensitive hop; b -> c is an UNTRUSTED hop
        // (BudgetExceeded status, Unknown precision). The derived transitive a -> c must
        // inherit the WEAKEST status/precision, never launder an untrusted input hop into
        // a confidently-precise derived edge (review finding #4 / D-06).
        let clean = copy_constraint("copy|a-b", 1, 2);
        let mut untrusted = copy_constraint("copy|b-c", 2, 3);
        untrusted.status = PointsToStatus::BudgetExceeded;
        untrusted.precision = PointsToPrecision::Unknown;
        let constraints = vec![clean, untrusted];

        let output = derive_edges(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            &SolverBudget::default(),
        );

        use crate::analysis_neutral::ids::SemanticNodeId;
        let edge = output
            .derived_edges
            .iter()
            .find(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(3))
            .expect("transitive a -> c derived");
        assert_eq!(
            edge.status,
            PointsToStatus::BudgetExceeded,
            "weakest contributing status must win"
        );
        assert_eq!(
            edge.precision,
            PointsToPrecision::Unknown,
            "weakest contributing precision must win"
        );
    }

    #[test]
    fn derive_edges_records_all_contributing_keys_for_a_duplicated_hop() {
        // Two distinct constraints both assert a -> b; the derived a -> c provenance must
        // record BOTH contributing identities, order-independently (review finding #10).
        let constraints = vec![
            copy_constraint("copy|a-b#1", 1, 2),
            copy_constraint("copy|a-b#2", 1, 2),
            copy_constraint("copy|b-c", 2, 3),
        ];

        let output = derive_edges(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            &SolverBudget::default(),
        );

        use crate::analysis_neutral::ids::SemanticNodeId;
        let edge = output
            .derived_edges
            .iter()
            .find(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(3))
            .expect("transitive a -> c derived");
        let interner = crate::internal_core::test_stable_key_interner();
        let keys: Vec<String> = edge
            .provenance
            .contributing_facts
            .iter()
            .map(|f| interner.resolve(f.stable_key).to_string())
            .collect();
        assert!(keys.iter().any(|k| k == "copy|a-b#1"), "{keys:?}");
        assert!(keys.iter().any(|k| k == "copy|a-b#2"), "{keys:?}");
        assert!(keys.iter().any(|k| k == "copy|b-c"), "{keys:?}");
    }

    #[test]
    fn budget_exhaustion_does_not_drop_independent_sources_and_is_signalled() {
        // Source 1 has a chain long enough to exhaust a tiny per-source step budget;
        // source 100 has a single short hop. The independent source 100 MUST still derive
        // its edge (no silent cross-source drop), and the run-level budget_status MUST
        // surface BudgetExceeded honestly (review finding #3 / D-06).
        let constraints = vec![
            copy_constraint("copy|1-2", 1, 2),
            copy_constraint("copy|2-3", 2, 3),
            copy_constraint("copy|3-4", 3, 4),
            copy_constraint("copy|100-101", 100, 101),
        ];
        let budget = SolverBudget {
            max_steps: 2,
            ..SolverBudget::default()
        };

        let output = derive_edges(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            &budget,
        );

        use crate::analysis_neutral::ids::SemanticNodeId;
        assert!(
            output
                .derived_edges
                .iter()
                .any(|e| e.source == SemanticNodeId(100) && e.target == SemanticNodeId(101)),
            "the independent source 100 must not be silently dropped when source 1 exhausts the budget"
        );
        assert_eq!(
            output.budget_status,
            BudgetStatus::BudgetExceeded,
            "budget exhaustion must surface as a run-level signal, never a silent drop"
        );
    }

    #[test]
    fn diamond_provenance_is_a_deterministic_witness_not_a_global_dependency() {
        // Diamond: a(1) -> b(2) -> d(4) AND a(1) -> c(3) -> d(4). There is ONE a -> d edge
        // whose provenance is a DETERMINISTIC witnessing path ({a-b, b-d} by BFS order).
        // Deleting a fact NOT on the witness path does NOT remove the a -> d flow (it
        // persists via the witness); deleting a witness fact does NOT reproduce the SAME
        // edge fact (by stable key) — the flow re-derives via the other path under a new
        // identity. This is the honest multi-path semantic (review finding #2 / D-09).
        let constraints = vec![
            copy_constraint("copy|a-b", 1, 2),
            copy_constraint("copy|b-d", 2, 4),
            copy_constraint("copy|a-c", 1, 3),
            copy_constraint("copy|c-d", 3, 4),
        ];
        let budget = SolverBudget::default();
        use crate::analysis_neutral::ids::SemanticNodeId;

        let base = derive_edges(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            &budget,
        );
        let ad: Vec<&DerivedEdgeFact> = base
            .derived_edges
            .iter()
            .filter(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(4))
            .collect();
        assert_eq!(ad.len(), 1, "exactly one a -> d edge");
        let interner = crate::internal_core::test_stable_key_interner();
        let witness_key = ad[0].stable_key;
        let witness: Vec<String> = ad[0]
            .provenance
            .contributing_facts
            .iter()
            .map(|f| interner.resolve(f.stable_key).to_string())
            .collect();
        assert!(
            witness.iter().any(|k| k == "copy|a-b") && witness.iter().any(|k| k == "copy|b-d"),
            "deterministic witness path is a-b -> b-d: {witness:?}"
        );

        // Delete a fact NOT on the witness path: the a -> d flow persists via the witness.
        let without_off_path: Vec<ConstraintFact> = constraints
            .iter()
            .filter(|c| c.stable_key != crate::internal_core::stable_key_for_test("copy|a-c"))
            .cloned()
            .collect();
        let rerun_off = derive_edges(
            &crate::internal_core::test_stable_key_interner(),
            &without_off_path,
            &budget,
        );
        assert!(
            rerun_off
                .derived_edges
                .iter()
                .any(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(4)),
            "deleting an off-witness fact must NOT remove the a -> d flow"
        );

        // Delete a witness fact: the SAME edge fact (by stable key) is NOT reproduced;
        // the flow re-derives via the other path under a different identity.
        let without_witness: Vec<ConstraintFact> = constraints
            .iter()
            .filter(|c| c.stable_key != crate::internal_core::stable_key_for_test("copy|a-b"))
            .cloned()
            .collect();
        let rerun_witness = derive_edges(
            &crate::internal_core::test_stable_key_interner(),
            &without_witness,
            &budget,
        );
        assert!(
            !rerun_witness
                .derived_edges
                .iter()
                .any(|e| e.stable_key == witness_key),
            "deleting a witness fact must invalidate THAT derived edge fact (by stable key)"
        );
    }

    #[test]
    fn budget_exhaustion_keeps_complete_pretruncation_edges_present() {
        // R1: a source's edges that were FULLY derived before the per-source step cap
        // was hit are sound and must keep their honest status — budget exhaustion costs
        // the edges never reached (signalled run-level), not the ones already derived.
        let constraints = vec![
            copy_constraint("copy|1-2", 1, 2),
            copy_constraint("copy|2-3", 2, 3),
            copy_constraint("copy|3-4", 3, 4),
        ];
        let budget = SolverBudget {
            max_steps: 2,
            ..SolverBudget::default()
        };

        let output = derive_edges(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            &budget,
        );

        use crate::analysis_neutral::ids::SemanticNodeId;
        let edge_1_2 = output
            .derived_edges
            .iter()
            .find(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(2))
            .expect("complete edge 1 -> 2 derived before truncation");
        assert_eq!(
            edge_1_2.status,
            PointsToStatus::Present,
            "a complete pre-truncation edge must NOT be downgraded"
        );
        assert_eq!(edge_1_2.precision, PointsToPrecision::FlowInsensitive);
        // The unreached edge is simply absent; the run-level signal conveys the loss.
        assert!(
            !output
                .derived_edges
                .iter()
                .any(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(4)),
            "the edge never reached under budget is absent"
        );
        assert_eq!(output.budget_status, BudgetStatus::BudgetExceeded);
    }

    #[test]
    fn untrusted_hop_on_alternate_path_conservatively_downgrades_derived_edge() {
        // R2: diamond a(1) -> b(2) -> d(4) (clean) AND a(1) -> c(3) -> d(4) where a -> c
        // is UNTRUSTED. The BFS witness reaches d via b (the shorter, clean path), but
        // because an untrusted alternate path also derives a -> d, the edge must be
        // conservatively downgraded — never laundered to a confident edge (#4, fully).
        let mut untrusted = copy_constraint("copy|a-c", 1, 3);
        untrusted.status = PointsToStatus::BudgetExceeded;
        untrusted.precision = PointsToPrecision::Unknown;
        let constraints = vec![
            copy_constraint("copy|a-b", 1, 2),
            copy_constraint("copy|b-d", 2, 4),
            untrusted,
            copy_constraint("copy|c-d", 3, 4),
        ];

        let output = derive_edges(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            &SolverBudget::default(),
        );

        use crate::analysis_neutral::ids::SemanticNodeId;
        let edge = output
            .derived_edges
            .iter()
            .find(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(4))
            .expect("transitive a -> d derived");
        assert_eq!(
            edge.status,
            PointsToStatus::BudgetExceeded,
            "an untrusted alternate path must conservatively downgrade the edge"
        );
        assert_eq!(edge.precision, PointsToPrecision::Unknown);
        // Provenance must JUSTIFY the downgraded status: the adopted derivation is the
        // untrusted path, so its contributing facts are recorded (not the clean witness).
        let interner = crate::internal_core::test_stable_key_interner();
        let keys: Vec<String> = edge
            .provenance
            .contributing_facts
            .iter()
            .map(|f| interner.resolve(f.stable_key).to_string())
            .collect();
        assert!(
            keys.iter().any(|k| k == "copy|a-c") && keys.iter().any(|k| k == "copy|c-d"),
            "provenance must list the untrusted derivation that set the status: {keys:?}"
        );
    }

    #[test]
    fn untrusted_path_downgrades_edges_multiple_hops_downstream() {
        // R2 (round-3): the conservative downgrade must PROPAGATE transitively, not just
        // one hop. Graph: a(1) -> d(4) clean [witness, shortest]; d(4) -> e(5) clean;
        // a(1) -> x(2) UNTRUSTED; x(2) -> y(3) clean; y(3) -> d(4) clean.
        // The clean a -> d is reached first and expands d -> e (e = Present) BEFORE the
        // untrusted a -> x -> y -> d path reaches d. The worst-path fixpoint must
        // re-derive d AND e so a -> e is downgraded — never laundered two hops past the
        // untrusted hop.
        let mut untrusted = copy_constraint("copy|a-x", 1, 2);
        untrusted.status = PointsToStatus::BudgetExceeded;
        untrusted.precision = PointsToPrecision::Unknown;
        let constraints = vec![
            copy_constraint("copy|a-d", 1, 4),
            copy_constraint("copy|d-e", 4, 5),
            untrusted,
            copy_constraint("copy|x-y", 2, 3),
            copy_constraint("copy|y-d", 3, 4),
        ];

        let output = derive_edges(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            &SolverBudget::default(),
        );

        use crate::analysis_neutral::ids::SemanticNodeId;
        let edge_ae = output
            .derived_edges
            .iter()
            .find(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(5))
            .expect("transitive a -> e derived");
        assert_eq!(
            edge_ae.status,
            PointsToStatus::BudgetExceeded,
            "an untrusted hop must downgrade edges multiple hops downstream (transitive)"
        );
        let edge_ad = output
            .derived_edges
            .iter()
            .find(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(4))
            .expect("a -> d derived");
        assert_eq!(edge_ad.status, PointsToStatus::BudgetExceeded);
    }

    #[test]
    fn solver_step_is_globally_monotonic_across_sources() {
        // R3: the solver step is a GLOBAL monotonic counter, not reset per source, so an
        // edge from a later source carries a strictly larger solver_step than an edge
        // from an earlier source (the documented monotonic contract).
        let constraints = vec![
            copy_constraint("copy|1-2", 1, 2),
            copy_constraint("copy|10-11", 10, 11),
        ];

        let output = derive_edges(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            &SolverBudget::default(),
        );

        use crate::analysis_neutral::ids::SemanticNodeId;
        let first = output
            .derived_edges
            .iter()
            .find(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(2))
            .expect("edge 1 -> 2");
        let later = output
            .derived_edges
            .iter()
            .find(|e| e.source == SemanticNodeId(10) && e.target == SemanticNodeId(11))
            .expect("edge 10 -> 11");
        assert!(
            later.provenance.solver_step > first.provenance.solver_step,
            "later source's solver_step ({}) must exceed the earlier source's ({})",
            later.provenance.solver_step,
            first.provenance.solver_step
        );
    }

    #[test]
    fn discarded_points_to_fold_does_not_pollute_run_level_budget_status() {
        // FINDING 3: `run_to_solver_output` emits the points-to edges from the step-1
        // `derive_edges` CopyEdge closure; the registered `PointsToPolicy`'s OWN Andersen
        // solve contributes ZERO edges to the output (its result rides `outcome.points_to`,
        // not `outcome.derived_edges`). So if that discarded Andersen solve exhausts its
        // object budget while the CopyEdge closure + Go RTA complete, the merged run-level
        // budget_status must still be WithinBudget — the discarded fold's exhaustion is a
        // misleading signal (no edge it produced is in the output) and would also spuriously
        // bust the solver_output_digest (WR-06 folds budget_status).
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
        // A CopyEdge closure that COMPLETES (a -> b -> c), so the points-to edges that
        // actually enter the output are within budget.
        let copy_constraints = vec![
            copy_constraint("copy|a-b", 1, 2),
            copy_constraint("copy|b-c", 2, 3),
        ];

        // A registered points-to policy may exhaust its own fold while the CopyEdge
        // closure that contributes to the output still completes within budget.
        let mut budget = SolverBudget::default();
        budget.points_to.max_objects_per_var = 1;
        let engine = SolverEngine::new(
            vec![Box::new(PointsToPolicy::new(exhausting_points_to))],
            budget,
        );

        let output = engine.run_to_solver_output(
            &crate::internal_core::test_stable_key_interner(),
            &copy_constraints,
        );

        // The transitive CopyEdge closure is present (the points-to edges that DO enter
        // the output completed within budget).
        assert!(
            output.derived_edges.iter().any(|e| e.source
                == crate::analysis_neutral::ids::SemanticNodeId(1)
                && e.target == crate::analysis_neutral::ids::SemanticNodeId(3)),
            "the CopyEdge closure a -> c must be present"
        );
        // The discarded Andersen fold's exhaustion must NOT surface at run level.
        assert_eq!(
            output.budget_status,
            BudgetStatus::WithinBudget,
            "a discarded points-to fold's budget exhaustion must not pollute run-level status"
        );
    }
    #[test]
    fn derive_edges_is_shuffle_stable() {
        let constraints = vec![
            copy_constraint("copy|a-b", 1, 2),
            copy_constraint("copy|b-c", 2, 3),
            copy_constraint("copy|c-d", 3, 4),
        ];
        let mut shuffled = constraints.clone();
        shuffled.reverse();

        let a = derive_edges(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            &SolverBudget::default(),
        );
        let b = derive_edges(
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
    fn derive_edges_stable_payload_ignores_reverse_intern_allocation_order() {
        let forward_interner = crate::internal_core::StableKeyInterner::default();
        let reverse_interner = crate::internal_core::StableKeyInterner::default();
        let forward = vec![
            copy_constraint_on(&forward_interner, "copy|a-b", 1, 2),
            copy_constraint_on(&forward_interner, "copy|b-c", 2, 3),
        ];
        let reverse = vec![
            copy_constraint_on(&reverse_interner, "copy|b-c", 2, 3),
            copy_constraint_on(&reverse_interner, "copy|a-b", 1, 2),
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

        let a = derive_edges(&forward_interner, &forward, &SolverBudget::default());
        let b = derive_edges(&reverse_interner, &reverse, &SolverBudget::default());
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

    fn copy_constraint_on(
        interner: &crate::internal_core::StableKeyInterner,
        stable_key: &str,
        src: u64,
        dst: u64,
    ) -> ConstraintFact {
        use crate::analysis_neutral::ids::SemanticConstraintId;
        ConstraintFact {
            id: SemanticConstraintId(0),
            kind: ConstraintKind::CopyEdge {
                dst: crate::analysis_neutral::ids::SemanticNodeId(dst),
                src: crate::analysis_neutral::ids::SemanticNodeId(src),
            },
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            stable_key: interner.intern(stable_key),
        }
    }
}
