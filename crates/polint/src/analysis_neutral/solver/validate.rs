//! Solver validation pass + D-12 cycle detection (D-06, D-11, D-12).
//!
//! Mirrors `analysis::semantic_graph::validate`: emits an evidence-bearing
//! [`Diagnostic`] per problem rather than silently dropping a malformed row —
//!
//! - duplicate stable keys across derived edges,
//! - dangling derived-edge endpoints (a `source`/`target` not present in the
//!   `SemanticNodeId` set the solver derived over),
//! - dense IDs that are not contiguous (`0..n`) and stable-key-sorted, and
//! - the precision ceiling (D-06): a derived edge must never claim
//!   `FactPrecision::Exact` — derived edges are an over-approximation.
//!
//! ## D-12 cycle detection (closed-input-set / single-fixpoint contract)
//!
//! [`detect_solver_summary_cycle`] proves no solver↔summary loop is admitted.
//! Function/procedure summaries are an *input* snapshot to the solver, never re-fed
//! into the same fixpoint as they are produced. A constraint set that would create a
//! `solver → summary → solver` cycle — modeled as a directed cycle in the
//! value-flow (`CopyEdge`) graph that passes through a `CallConstraint` (summary)
//! node — is DETECTED and reported as a bounded diagnostic rather than allowed to
//! diverge. This is the concrete mechanism behind D-11's "closed input set / single
//! fixpoint per run / bounded outer iterations": the solver's value flow is
//! deterministic and terminating because the cycle is detected and the back-edge is
//! not re-expanded.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_api::FactPrecision;
use crate::analysis_neutral::ids::SemanticNodeId;
use crate::analysis_neutral::semantic_graph::constraints::{ConstraintFact, ConstraintKind};
use crate::analysis_neutral::solver::facts::{DerivedEdgeFact, derived_edge_precision_ceiling};
use crate::analysis_neutral::solver::store::SOLVER_PROVIDER_ID;
use crate::internal_core::{Diagnostic, DiagnosticRange as TextRange};

/// Validates the solver's derived-edge rows, emitting an evidence-bearing
/// [`Diagnostic`] per problem (D-06). Operates over the normalized rows the store
/// holds; `node_ids` is the set of `SemanticNodeId`s the solver derived over (the
/// edge endpoints must reference nodes that exist in the input graph).
pub fn validate_derived_edges(
    interner: &crate::internal_core::StableKeyInterner,
    derived_edges: &[DerivedEdgeFact],
    node_ids: &BTreeSet<SemanticNodeId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let resolved_keys: Vec<std::sync::Arc<str>> = derived_edges
        .iter()
        .map(|row| interner.resolve(row.stable_key))
        .collect();

    // Duplicate stable keys (compare by resolved text).
    check_duplicate_stable_keys(diagnostics, resolved_keys.iter().map(AsRef::as_ref));

    // Dense IDs contiguous (0..n) and stable-key-sorted by resolved text.
    check_dense_ids_sorted(
        diagnostics,
        derived_edges
            .iter()
            .zip(resolved_keys.iter())
            .map(|(row, key)| (row.id.0, key.as_ref())),
    );

    for (edge, edge_key) in derived_edges.iter().zip(&resolved_keys) {
        // Dangling endpoint references.
        if !node_ids.is_empty() {
            if !node_ids.contains(&edge.source) {
                push_diagnostic(
                    diagnostics,
                    edge_key.as_ref(),
                    "source",
                    "dangling derived edge source node reference",
                );
            }
            if !node_ids.contains(&edge.target) {
                push_diagnostic(
                    diagnostics,
                    edge_key.as_ref(),
                    "target",
                    "dangling derived edge target node reference",
                );
            }
        }

        // Precision ceiling (D-06): a derived edge must never claim Exact.
        if let Some(diagnostic) = reject_exact_precision(
            derived_edge_precision_ceiling(edge.precision),
            edge_key.as_ref(),
        ) {
            diagnostics.push(diagnostic);
        }
    }
}

/// Precision-ceiling check for derived edges (D-06).
///
/// A derived edge is an over-approximation and must never claim the exact tier.
/// Returns a diagnostic when the (already-mapped) kernel precision is
/// [`FactPrecision::Exact`].
pub fn reject_exact_precision(precision: FactPrecision, stable_key: &str) -> Option<Diagnostic> {
    if precision == FactPrecision::Exact {
        Some(precision_ceiling_diagnostic(stable_key))
    } else {
        None
    }
}

/// D-12 cycle detection: prove no `solver → summary → solver` loop is admitted.
///
/// Builds the directed value-flow graph from the input `CopyEdge` constraints, marks
/// the nodes that are `CallConstraint` (summary) anchors, and detects a directed
/// cycle that passes through a summary node. Such a cycle is the structural shape of
/// a `solver → summary → solver` loop: a derived edge feeding back into a summary
/// that re-enters the solver. Because summaries are an INPUT snapshot (never re-fed
/// into the same fixpoint), the cycle is DETECTED and reported as a bounded
/// diagnostic — the solver does not diverge. Returns the number of distinct summary
/// nodes implicated in a cycle (0 when the constraint set is acyclic through
/// summaries), and pushes one diagnostic per implicated summary node.
///
/// This is deterministic: the traversal is over `BTreeMap`/`BTreeSet`-ordered
/// adjacency, so the diagnostics are emitted in stable node order.
pub fn detect_solver_summary_cycle(
    constraints: &[ConstraintFact],
    diagnostics: &mut Vec<Diagnostic>,
) -> usize {
    // Directed value-flow adjacency `src -> {dst}` over CopyEdge constraints.
    let mut adjacency: BTreeMap<SemanticNodeId, BTreeSet<SemanticNodeId>> = BTreeMap::new();
    // Summary (CallConstraint) anchor nodes.
    let mut summary_nodes: BTreeSet<SemanticNodeId> = BTreeSet::new();

    for constraint in constraints {
        match &constraint.kind {
            ConstraintKind::CopyEdge { dst, src } => {
                adjacency.entry(*src).or_default().insert(*dst);
            }
            ConstraintKind::CallConstraint { callsite, .. } => {
                summary_nodes.insert(*callsite);
            }
            _ => {}
        }
    }

    // A summary node participates in a solver↔summary cycle iff it lies on a directed
    // cycle in the value-flow graph. Compute ALL on-cycle nodes ONCE via a single
    // Tarjan SCC pass (O(N+E)) rather than a per-summary full traversal
    // (O(summaries · (N+E))), then intersect with the summary anchors.
    let on_cycle = nodes_on_cycle(&adjacency);
    let implicated: BTreeSet<SemanticNodeId> =
        summary_nodes.intersection(&on_cycle).copied().collect();

    for node in &implicated {
        push_diagnostic(
            diagnostics,
            &format!("summary|node|{}", node.0),
            "cycle",
            "solver-summary cycle detected and bounded: summaries are an input \
             snapshot, never re-fed into the same fixpoint (D-12)",
        );
    }

    implicated.len()
}

/// All nodes lying on a directed cycle of `adjacency`, computed in one deterministic
/// Tarjan SCC pass (iterative — no recursion, so deep graphs cannot blow the stack).
///
/// A node is on a cycle iff its strongly-connected component has more than one member
/// OR it has a self-loop edge. Roots and successors are visited in
/// `BTreeMap`/`BTreeSet` order, so the result is byte-stable. This replaces the prior
/// per-summary reachability BFS (one full traversal per summary node) with a single
/// whole-graph pass.
fn nodes_on_cycle(
    adjacency: &BTreeMap<SemanticNodeId, BTreeSet<SemanticNodeId>>,
) -> BTreeSet<SemanticNodeId> {
    /// One node's frame on the explicit DFS work stack.
    struct Frame {
        node: SemanticNodeId,
        successors: Vec<SemanticNodeId>,
        next: usize,
    }

    let mut index_of: BTreeMap<SemanticNodeId, usize> = BTreeMap::new();
    let mut lowlink: BTreeMap<SemanticNodeId, usize> = BTreeMap::new();
    let mut on_stack: BTreeSet<SemanticNodeId> = BTreeSet::new();
    let mut scc_stack: Vec<SemanticNodeId> = Vec::new();
    let mut next_index: usize = 0;
    let mut on_cycle: BTreeSet<SemanticNodeId> = BTreeSet::new();

    let successors_of = |node: SemanticNodeId| -> Vec<SemanticNodeId> {
        adjacency
            .get(&node)
            .map(|targets| targets.iter().copied().collect())
            .unwrap_or_default()
    };

    for &root in adjacency.keys() {
        if index_of.contains_key(&root) {
            continue;
        }
        index_of.insert(root, next_index);
        lowlink.insert(root, next_index);
        next_index += 1;
        scc_stack.push(root);
        on_stack.insert(root);
        let mut work: Vec<Frame> = vec![Frame {
            node: root,
            successors: successors_of(root),
            next: 0,
        }];

        while let Some(frame) = work.last_mut() {
            let node = frame.node;
            if frame.next < frame.successors.len() {
                let w = frame.successors[frame.next];
                frame.next += 1;
                if w == node {
                    // Self-loop: the node is trivially on a cycle.
                    on_cycle.insert(node);
                }
                match index_of.get(&w).copied() {
                    // Tree edge: `w` is unvisited — assign its index and descend.
                    None => {
                        index_of.insert(w, next_index);
                        lowlink.insert(w, next_index);
                        next_index += 1;
                        scc_stack.push(w);
                        on_stack.insert(w);
                        work.push(Frame {
                            node: w,
                            successors: successors_of(w),
                            next: 0,
                        });
                    }
                    // Back/cross edge: only an on-stack target tightens the lowlink.
                    Some(iw) => {
                        if on_stack.contains(&w) {
                            let ln = lowlink[&node];
                            lowlink.insert(node, ln.min(iw));
                        }
                    }
                }
            } else {
                // Finished `node`: if it roots an SCC, pop the component.
                if lowlink[&node] == index_of[&node] {
                    let mut component: Vec<SemanticNodeId> = Vec::new();
                    loop {
                        let w = scc_stack.pop().expect("scc stack non-empty");
                        on_stack.remove(&w);
                        component.push(w);
                        if w == node {
                            break;
                        }
                    }
                    if component.len() > 1 {
                        on_cycle.extend(component);
                    }
                }
                work.pop();
                if let Some(parent) = work.last() {
                    let lp = lowlink[&parent.node];
                    let ln = lowlink[&node];
                    lowlink.insert(parent.node, lp.min(ln));
                }
            }
        }
    }

    on_cycle
}

fn check_duplicate_stable_keys<'a>(
    diagnostics: &mut Vec<Diagnostic>,
    keys: impl Iterator<Item = &'a str>,
) {
    let mut seen = BTreeSet::new();
    for key in keys {
        if !seen.insert(key) {
            push_diagnostic(diagnostics, key, "stable_key", "duplicate stable key");
        }
    }
}

/// Asserts the dense IDs are exactly `0..n` in stable-key order.
fn check_dense_ids_sorted<'a>(
    diagnostics: &mut Vec<Diagnostic>,
    rows: impl Iterator<Item = (u64, &'a str)>,
) {
    let mut previous_key: Option<&str> = None;
    for (index, (id, stable_key)) in rows.enumerate() {
        if id != index as u64 {
            push_diagnostic(
                diagnostics,
                stable_key,
                "id",
                "dense id is not contiguous with the stable-key sort order",
            );
        }
        if let Some(previous) = previous_key
            && previous > stable_key
        {
            push_diagnostic(
                diagnostics,
                stable_key,
                "stable_key",
                "rows are not sorted by stable key",
            );
        }
        previous_key = Some(stable_key);
    }
}

fn push_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    stable_key: &str,
    field: &'static str,
    reason: &'static str,
) {
    diagnostics.push(
        Diagnostic::error(
            "polint/internal",
            "<workspace>",
            TextRange::point(1, 1),
            "Solver validation failed for a derived-edge stable key.".to_string(),
        )
        .with_evidence("family", "SolverDerivedEdge")
        .with_evidence("stable_key", stable_key.to_string())
        .with_evidence("field", field)
        .with_evidence("reason", reason),
    );
}

fn precision_ceiling_diagnostic(stable_key: &str) -> Diagnostic {
    Diagnostic::error(
        "polint/internal",
        "<workspace>",
        TextRange::point(1, 1),
        format!("Solver precision ceiling exceeded for {SOLVER_PROVIDER_ID}."),
    )
    .with_evidence("family", "SolverDerivedEdge")
    .with_evidence("stable_key", stable_key.to_string())
    .with_evidence("field", "precision")
    .with_evidence(
        "reason",
        "precision ceiling exceeded: derived edges must not claim Exact",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::FactFamily;
    use crate::analysis_neutral::ids::{DerivedEdgeId, SemanticConstraintId, SemanticNodeId};
    use crate::analysis_neutral::points_to::facts::{PointsToPrecision, PointsToStatus};
    use crate::analysis_neutral::semantic_graph::constraints::{ConstraintFact, ConstraintKind};
    use crate::analysis_neutral::solver::provenance::{ContributingFact, DerivedEdgeProvenance};

    fn provenance() -> DerivedEdgeProvenance {
        let interner = crate::internal_core::test_stable_key_interner();
        DerivedEdgeProvenance::new(
            &interner,
            vec![ContributingFact::from_parts(
                &interner,
                FactFamily::PointsToConstraint,
                &[("constraint", "copy".to_string())],
            )],
            &ConstraintKind::CopyEdge {
                dst: SemanticNodeId(1),
                src: SemanticNodeId(2),
            },
            1,
        )
    }

    fn edge(id: u64, source: u64, target: u64, stable_key: &str) -> DerivedEdgeFact {
        DerivedEdgeFact {
            id: DerivedEdgeId(id),
            source: SemanticNodeId(source),
            target: SemanticNodeId(target),
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
            provenance: provenance(),
            claim: crate::analysis_neutral::points_to::tiers::EdgeClaim::Unassigned,
        }
    }

    fn copy(src: u64, dst: u64, stable_key: &str) -> ConstraintFact {
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

    fn call_constraint(callsite: u64, stable_key: &str) -> ConstraintFact {
        ConstraintFact {
            id: SemanticConstraintId(0),
            kind: ConstraintKind::call_at(SemanticNodeId(callsite)),
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    #[test]
    fn valid_edges_produce_no_diagnostics() {
        let node_ids: BTreeSet<SemanticNodeId> =
            [SemanticNodeId(0), SemanticNodeId(1)].into_iter().collect();
        let edges = vec![edge(0, 0, 1, "edge|copy_edge|a")];
        let mut diagnostics = Vec::new();
        validate_derived_edges(
            &crate::internal_core::test_stable_key_interner(),
            &edges,
            &node_ids,
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn precision_ceiling_helper_rejects_fact_precision_exact() {
        assert!(reject_exact_precision(FactPrecision::Exact, "k").is_some());
        assert!(reject_exact_precision(FactPrecision::SetupAware, "k").is_none());
        assert!(reject_exact_precision(FactPrecision::Heuristic, "k").is_none());
    }

    #[test]
    fn duplicate_stable_keys_are_rejected() {
        let node_ids: BTreeSet<SemanticNodeId> =
            [SemanticNodeId(0), SemanticNodeId(1)].into_iter().collect();
        let edges = vec![
            edge(0, 0, 1, "edge|copy_edge|dup"),
            edge(1, 0, 1, "edge|copy_edge|dup"),
        ];
        let mut diagnostics = Vec::new();
        validate_derived_edges(
            &crate::internal_core::test_stable_key_interner(),
            &edges,
            &node_ids,
            &mut diagnostics,
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| format!("{d:?}").contains("duplicate stable key")),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn dangling_endpoint_is_rejected() {
        let node_ids: BTreeSet<SemanticNodeId> = [SemanticNodeId(0)].into_iter().collect();
        // target node 9 is not in the node set.
        let edges = vec![edge(0, 0, 9, "edge|copy_edge|a")];
        let mut diagnostics = Vec::new();
        validate_derived_edges(
            &crate::internal_core::test_stable_key_interner(),
            &edges,
            &node_ids,
            &mut diagnostics,
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| format!("{d:?}").contains("dangling derived edge target")),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn non_contiguous_dense_id_is_rejected() {
        let node_ids: BTreeSet<SemanticNodeId> =
            [SemanticNodeId(0), SemanticNodeId(1)].into_iter().collect();
        // id 5 at index 0 — not contiguous.
        let edges = vec![edge(5, 0, 1, "edge|copy_edge|a")];
        let mut diagnostics = Vec::new();
        validate_derived_edges(
            &crate::internal_core::test_stable_key_interner(),
            &edges,
            &node_ids,
            &mut diagnostics,
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| format!("{d:?}").contains("dense id is not contiguous")),
            "{diagnostics:?}"
        );
    }

    // -----------------------------------------------------------------------
    // D-12 cycle detection.
    // -----------------------------------------------------------------------

    #[test]
    fn acyclic_constraint_set_admits_no_solver_summary_cycle() {
        // A linear chain a -> b -> c through a summary node `b` is acyclic: the
        // summary does not reach itself, so no cycle is reported.
        let constraints = vec![
            copy(1, 2, "copy|a-b"),
            copy(2, 3, "copy|b-c"),
            call_constraint(2, "call|b"),
        ];
        let mut diagnostics = Vec::new();
        let implicated = detect_solver_summary_cycle(&constraints, &mut diagnostics);
        assert_eq!(implicated, 0, "{diagnostics:?}");
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn solver_summary_solver_cycle_is_detected_and_bounded() {
        // A value-flow cycle a -> b -> a passing through a summary node `b`
        // (CallConstraint at node 2) is the structural shape of a
        // solver → summary → solver loop. It MUST be detected (and bounded, not
        // divergent): the check terminates and reports the implicated summary node.
        let constraints = vec![
            copy(1, 2, "copy|a-b"),
            copy(2, 1, "copy|b-a"),
            call_constraint(2, "call|b"),
        ];
        let mut diagnostics = Vec::new();
        let implicated = detect_solver_summary_cycle(&constraints, &mut diagnostics);
        assert_eq!(implicated, 1, "the summary node should be implicated");
        assert!(
            diagnostics
                .iter()
                .any(|d| format!("{d:?}").contains("solver-summary cycle detected and bounded")),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn cycle_detection_terminates_on_a_self_loop_summary() {
        // A degenerate self-loop b -> b on a summary node terminates (bounded by the
        // visited set) rather than diverging, and is reported.
        let constraints = vec![copy(2, 2, "copy|b-b"), call_constraint(2, "call|b")];
        let mut diagnostics = Vec::new();
        let implicated = detect_solver_summary_cycle(&constraints, &mut diagnostics);
        assert_eq!(implicated, 1);
    }

    #[test]
    fn non_summary_cycle_is_not_flagged_as_solver_summary_loop() {
        // A pure value-flow cycle a -> b -> a with NO summary (CallConstraint) node is
        // not a solver↔summary loop — the closed-input contract is about summaries
        // specifically — so it is not flagged here.
        let constraints = vec![copy(1, 2, "copy|a-b"), copy(2, 1, "copy|b-a")];
        let mut diagnostics = Vec::new();
        let implicated = detect_solver_summary_cycle(&constraints, &mut diagnostics);
        assert_eq!(implicated, 0);
        assert!(diagnostics.is_empty());
    }

    // -----------------------------------------------------------------------
    // Tarjan SCC coverage: lock the cases the prior 4 tests didn't exercise, so a
    // future regression in lowlink propagation / SCC-vs-self-loop union is caught.
    // -----------------------------------------------------------------------

    #[test]
    fn three_node_cycle_implicates_its_summary_member() {
        // A >2-node cycle 1 -> 2 -> 3 -> 1 (SCC size 3). The summary node 3 is a member,
        // so it is implicated; a non-member summary would not be.
        let constraints = vec![
            copy(1, 2, "copy|1-2"),
            copy(2, 3, "copy|2-3"),
            copy(3, 1, "copy|3-1"),
            call_constraint(3, "call|3"),
        ];
        let mut diagnostics = Vec::new();
        let implicated = detect_solver_summary_cycle(&constraints, &mut diagnostics);
        assert_eq!(implicated, 1, "summary node in the 3-cycle is implicated");
    }

    #[test]
    fn self_loop_inside_a_larger_scc_is_implicated() {
        // Node 2 has BOTH a self-loop and membership in the 1<->2 SCC; it must be
        // implicated exactly once (the SCC-size and self-loop signals do not double-count).
        let constraints = vec![
            copy(1, 2, "copy|1-2"),
            copy(2, 1, "copy|2-1"),
            copy(2, 2, "copy|2-2"),
            call_constraint(2, "call|2"),
        ];
        let mut diagnostics = Vec::new();
        let implicated = detect_solver_summary_cycle(&constraints, &mut diagnostics);
        assert_eq!(implicated, 1);
        assert_eq!(
            diagnostics.len(),
            1,
            "implicated summary reported exactly once"
        );
    }

    #[test]
    fn deep_back_edge_to_an_ancestor_is_detected() {
        // A long chain 1->2->3->4 with a back edge 4->2 forms an SCC {2,3,4} reached
        // only after a depth-3 descent — this exercises iterative-Tarjan lowlink
        // propagation up the explicit work stack. Summary node 3 (mid-SCC) is implicated;
        // node 1 (a tree-root feeding the SCC but not in it) is NOT.
        let constraints = vec![
            copy(1, 2, "copy|1-2"),
            copy(2, 3, "copy|2-3"),
            copy(3, 4, "copy|3-4"),
            copy(4, 2, "copy|4-2"),
            call_constraint(3, "call|3"),
            call_constraint(1, "call|1"),
        ];
        let mut diagnostics = Vec::new();
        let implicated = detect_solver_summary_cycle(&constraints, &mut diagnostics);
        assert_eq!(
            implicated, 1,
            "only the in-SCC summary node 3 is implicated, not the feeder node 1"
        );
    }

    #[test]
    fn target_only_summary_node_is_not_implicated() {
        // Summary node 9 is only a CopyEdge TARGET (no outgoing edge), so it cannot be
        // on a cycle and must never be implicated — even alongside an unrelated cycle.
        let constraints = vec![
            copy(1, 9, "copy|1-9"),
            copy(2, 3, "copy|2-3"),
            copy(3, 2, "copy|3-2"),
            call_constraint(9, "call|9"),
        ];
        let mut diagnostics = Vec::new();
        let implicated = detect_solver_summary_cycle(&constraints, &mut diagnostics);
        assert_eq!(
            implicated, 0,
            "a target-only summary node is never on a cycle"
        );
        assert!(diagnostics.is_empty());
    }
}
