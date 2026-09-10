use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_neutral::error::AnalysisError;
use crate::analysis_neutral::ids::{SemanticConstraintId, SemanticEdgeId, SemanticNodeId};
use crate::analysis_neutral::semantic_graph::constraints::{ConstraintFact, ConstraintKind};
use crate::analysis_neutral::semantic_graph::facts::{
    EdgeKind, NodeKind, SemanticEdgeFact, SemanticNodeFact,
};
use crate::internal_core::StableKeyInterner;

pub const SEMANTIC_GRAPH_PROVIDER_ID: &str = "polint.semantic_graph";

/// Provider output for `polint.semantic_graph` — the normalized node, edge, and
/// constraint sets.
///
/// The constraint vocabulary (GRAPH-02) arrives in Plan 02 via the `constraints`
/// field: the closed `ConstraintKind`/`ConstraintFact` rows that the
/// unified solver folds over.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticGraphOutput {
    pub nodes: Vec<SemanticNodeFact>,
    pub edges: Vec<SemanticEdgeFact>,
    pub constraints: Vec<ConstraintFact>,
}

impl SemanticGraphOutput {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Sorts nodes and edges by `(stable_key, id)` THEN reassigns dense
    /// `SemanticNodeId`/`SemanticEdgeId` sequentially by index (D-05: dense IDs only
    /// after the stable-key sort). Edge `source`/`target` handles are remapped from
    /// the pre-sort node IDs to the post-sort dense node IDs so the adjacency stays
    /// consistent after re-densification.
    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.nodes.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        // Map each node's pre-sort dense id to its post-sort dense id so edges can
        // be rewritten to the new node numbering.
        let mut remap: BTreeMap<SemanticNodeId, SemanticNodeId> = BTreeMap::new();
        for (index, node) in self.nodes.iter_mut().enumerate() {
            let new_id = SemanticNodeId(index as u64);
            remap.insert(node.id, new_id);
            node.id = new_id;
        }
        // Rewrite edge endpoints to the post-sort node IDs before sorting edges, so
        // an edge's `source`/`target` always reference the densified node numbering.
        for edge in &mut self.edges {
            if let Some(&new_source) = remap.get(&edge.source) {
                edge.source = new_source;
            }
            if let Some(&new_target) = remap.get(&edge.target) {
                edge.target = new_target;
            }
        }
        self.edges.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, edge) in self.edges.iter_mut().enumerate() {
            edge.id = SemanticEdgeId(index as u64);
        }
        // Rewrite every `SemanticNodeId` a constraint payload references to the
        // post-sort node numbering (the same discipline as edge endpoints), then
        // sort+densify constraints exactly as nodes/edges (D-05). Without this, a
        // constraint's node references would point at stale pre-sort node IDs after
        // re-densification.
        for constraint in &mut self.constraints {
            constraint
                .kind
                .remap_nodes(|node| remap.get(&node).copied().unwrap_or(node));
        }
        self.constraints.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, constraint) in self.constraints.iter_mut().enumerate() {
            constraint.id = SemanticConstraintId(index as u64);
        }
        self
    }

    pub fn validate_references(&self, interner: &StableKeyInterner) -> Result<(), AnalysisError> {
        let node_ids: BTreeSet<SemanticNodeId> = self.nodes.iter().map(|node| node.id).collect();

        for edge in &self.edges {
            if !node_ids.contains(&edge.source) {
                return Err(AnalysisError::InvalidFact {
                    provider: SEMANTIC_GRAPH_PROVIDER_ID,
                    reason: format!(
                        "dangling edge source {:?} for semantic edge `{}`",
                        edge.source,
                        interner.resolve(edge.stable_key)
                    ),
                });
            }
            if !node_ids.contains(&edge.target) {
                return Err(AnalysisError::InvalidFact {
                    provider: SEMANTIC_GRAPH_PROVIDER_ID,
                    reason: format!(
                        "dangling edge target {:?} for semantic edge `{}`",
                        edge.target,
                        interner.resolve(edge.stable_key)
                    ),
                });
            }
        }

        // `TypeConstraint.type_fact` references the type substrate (a different
        // family) and is intentionally NOT checked here.
        for constraint in &self.constraints {
            for node in constraint.kind.referenced_nodes() {
                if !node_ids.contains(&node) {
                    return Err(AnalysisError::InvalidFact {
                        provider: SEMANTIC_GRAPH_PROVIDER_ID,
                        reason: format!(
                            "dangling constraint node {:?} for semantic constraint `{}`",
                            node,
                            interner.resolve(constraint.stable_key)
                        ),
                    });
                }
            }
        }

        Ok(())
    }
}

/// Typed semantic-graph store with the deterministic read indexes consumers use
/// (D-14): nodes-by-kind, edges-by-kind, outgoing (forward) adjacency, and incoming
/// (backward) adjacency. The incoming index is the one the unified solver's
/// reachability/RTA fixpoint traverses; it is built alongside the outgoing index in
/// the same post-normalization pass, never on demand.
#[derive(Debug, Clone, Default)]
pub struct SemanticGraphStore {
    nodes: Vec<SemanticNodeFact>,
    edges: Vec<SemanticEdgeFact>,
    constraints: Vec<ConstraintFact>,
    nodes_by_kind: BTreeMap<&'static str, Vec<usize>>,
    edges_by_kind: BTreeMap<EdgeKind, Vec<usize>>,
    /// Constraints indexed by their `ConstraintKind` snake_case tag (D-14).
    constraints_by_kind: BTreeMap<&'static str, Vec<usize>>,
    /// Forward/outgoing adjacency: edge source node -> edge IDs leaving it.
    outgoing: BTreeMap<SemanticNodeId, Vec<SemanticEdgeId>>,
    /// Backward/incoming adjacency: edge target node -> edge IDs entering it.
    incoming: BTreeMap<SemanticNodeId, Vec<SemanticEdgeId>>,
}

impl SemanticGraphStore {
    /// Builds the store after `normalized()`, validating that every edge
    /// `source`/`target` resolves to a stored node (dangling endpoint ->
    /// [`AnalysisError::InvalidFact`], mirroring the reachability store) and building
    /// the four deterministic index sidecars.
    pub fn from_output(
        output: SemanticGraphOutput,
        interner: &StableKeyInterner,
    ) -> Result<Self, AnalysisError> {
        Self::from_normalized_output(output.normalized(interner), interner)
    }

    pub fn from_normalized_output(
        output: SemanticGraphOutput,
        interner: &StableKeyInterner,
    ) -> Result<Self, AnalysisError> {
        output.validate_references(interner)?;
        let mut store = Self {
            nodes: output.nodes,
            edges: output.edges,
            constraints: output.constraints,
            ..Self::default()
        };

        for (index, node) in store.nodes.iter().enumerate() {
            store
                .nodes_by_kind
                .entry(node.kind.as_str())
                .or_default()
                .push(index);
        }

        // Single post-normalization edge pass builds the by-kind index and BOTH
        // adjacency directions so they stay deterministic and order-independent.
        // Edges are already sorted by (stable_key, id), so each per-node edge-id
        // vector is appended in stable order.
        for (index, edge) in store.edges.iter().enumerate() {
            store
                .edges_by_kind
                .entry(edge.kind)
                .or_default()
                .push(index);
            store.outgoing.entry(edge.source).or_default().push(edge.id);
            store.incoming.entry(edge.target).or_default().push(edge.id);
        }

        // Constraints are already sorted by (stable_key, id), so each per-kind index
        // vector is appended in stable order.
        for (index, constraint) in store.constraints.iter().enumerate() {
            store
                .constraints_by_kind
                .entry(constraint.kind.as_str())
                .or_default()
                .push(index);
        }

        Ok(store)
    }

    pub fn nodes(&self) -> &[SemanticNodeFact] {
        &self.nodes
    }

    pub fn edges(&self) -> &[SemanticEdgeFact] {
        &self.edges
    }

    pub fn nodes_for_kind(&self, kind: &NodeKind) -> &[usize] {
        self.nodes_by_kind
            .get(kind.as_str())
            .map_or(&[], Vec::as_slice)
    }

    pub fn edges_for_kind(&self, kind: EdgeKind) -> &[usize] {
        self.edges_by_kind.get(&kind).map_or(&[], Vec::as_slice)
    }

    /// Outgoing (forward) adjacency: the edge IDs whose `source` is `node`.
    pub fn outgoing_edges(&self, node: SemanticNodeId) -> &[SemanticEdgeId] {
        self.outgoing.get(&node).map_or(&[], Vec::as_slice)
    }

    /// Incoming (backward) adjacency: the edge IDs whose `target` is `node`. This
    /// is the index the unified solver's reachability/RTA fixpoint traverses.
    pub fn incoming_edges(&self, node: SemanticNodeId) -> &[SemanticEdgeId] {
        self.incoming.get(&node).map_or(&[], Vec::as_slice)
    }

    pub fn constraints(&self) -> &[ConstraintFact] {
        &self.constraints
    }

    /// Indices into [`Self::constraints`] for every constraint of the given kind
    /// tag (the `ConstraintKind::as_str()` snake_case label).
    pub fn constraints_for_kind(&self, kind: &ConstraintKind) -> &[usize] {
        self.constraints_by_kind
            .get(kind.as_str())
            .map_or(&[], Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::ids::{CallSiteId, ObjectTokenId, SemanticConstraintId};
    use crate::analysis_neutral::points_to::facts::{PointsToPrecision, PointsToStatus};
    use crate::analysis_neutral::semantic_graph::facts::SemanticPrecision;
    use crate::internal_core::{FunctionId, stable_key_for_test, test_stable_key_interner};

    fn node(id: u64, kind: NodeKind, stable_key: &str) -> SemanticNodeFact {
        SemanticNodeFact {
            id: SemanticNodeId(id),
            kind,
            precision: SemanticPrecision::SetupAware,
            stable_key: stable_key_for_test(stable_key),
        }
    }

    fn edge(
        id: u64,
        source: u64,
        target: u64,
        kind: EdgeKind,
        stable_key: &str,
    ) -> SemanticEdgeFact {
        SemanticEdgeFact {
            id: SemanticEdgeId(id),
            source: SemanticNodeId(source),
            target: SemanticNodeId(target),
            kind,
            precision: SemanticPrecision::Conservative,
            stable_key: stable_key_for_test(stable_key),
        }
    }

    fn constraint(id: u64, kind: ConstraintKind, stable_key: &str) -> ConstraintFact {
        ConstraintFact {
            id: SemanticConstraintId(id),
            kind,
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            stable_key: stable_key_for_test(stable_key),
        }
    }

    fn sample_output() -> SemanticGraphOutput {
        // Two nodes and one edge from node-A (source) to node-B (target), plus one
        // CallConstraint anchored at node-B.
        SemanticGraphOutput {
            nodes: vec![
                node(
                    0,
                    NodeKind::Function(FunctionId::from_raw(1)),
                    "node|function|a",
                ),
                node(1, NodeKind::Callsite(CallSiteId(2)), "node|callsite|b"),
            ],
            edges: vec![edge(0, 0, 1, EdgeKind::Call, "edge|call|a|b")],
            constraints: vec![constraint(
                0,
                ConstraintKind::CallConstraint {
                    callsite: SemanticNodeId(1),
                },
                "constraint|call_constraint|b",
            )],
        }
    }

    #[test]
    fn normalized_assigns_dense_ids_after_stable_key_sort() {
        let normalized = SemanticGraphOutput {
            nodes: vec![
                node(99, NodeKind::Callsite(CallSiteId(2)), "node|callsite|z"),
                node(
                    7,
                    NodeKind::Function(FunctionId::from_raw(1)),
                    "node|function|a",
                ),
            ],
            edges: Vec::new(),
            constraints: Vec::new(),
        }
        .normalized(&test_stable_key_interner());
        // Sorted by stable_key: "node|callsite|z" < "node|function|a" ('c' < 'f'),
        // so the callsite node sorts first and is assigned dense id 0, regardless of
        // its larger pre-sort id (99). Dense IDs are assigned only after the sort.
        assert_eq!(
            test_stable_key_interner()
                .resolve(normalized.nodes[0].stable_key)
                .as_ref(),
            "node|callsite|z"
        );
        assert_eq!(normalized.nodes[0].id, SemanticNodeId(0));
        assert_eq!(
            test_stable_key_interner()
                .resolve(normalized.nodes[1].stable_key)
                .as_ref(),
            "node|function|a"
        );
        assert_eq!(normalized.nodes[1].id, SemanticNodeId(1));
    }

    #[test]
    fn normalized_is_shuffle_stable() {
        let base = sample_output();
        // Clone and shuffle node + edge row order.
        let mut shuffled = base.clone();
        shuffled.nodes.reverse();
        shuffled.edges.reverse();

        let interner = test_stable_key_interner();
        let a = base.normalized(&interner);
        let b = shuffled.normalized(&interner);

        // Byte-identical serialized output under shuffle: serialize node and edge
        // vectors (dense `id` is `#[serde(skip)]`, so this captures kind/precision/
        // stable_key/endpoints).
        assert_eq!(a.nodes, b.nodes);
        assert_eq!(a.edges, b.edges);
        // The dense IDs assigned are identical too.
        assert_eq!(
            a.nodes.iter().map(|n| n.id).collect::<Vec<_>>(),
            b.nodes.iter().map(|n| n.id).collect::<Vec<_>>()
        );
        assert_eq!(
            a.edges.iter().map(|e| e.id).collect::<Vec<_>>(),
            b.edges.iter().map(|e| e.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn from_output_builds_deterministic_kind_indexes() {
        let store = SemanticGraphStore::from_output(sample_output(), &test_stable_key_interner())
            .expect("store");
        assert_eq!(store.nodes().len(), 2);
        assert_eq!(store.edges().len(), 1);
        assert_eq!(
            store
                .nodes_for_kind(&NodeKind::Function(FunctionId::from_raw(0)))
                .len(),
            1
        );
        assert_eq!(
            store
                .nodes_for_kind(&NodeKind::Callsite(CallSiteId(0)))
                .len(),
            1
        );
        assert!(
            store
                .nodes_for_kind(&NodeKind::AbstractObject(ObjectTokenId(0)))
                .is_empty()
        );
        assert_eq!(store.edges_for_kind(EdgeKind::Call).len(), 1);
        assert!(store.edges_for_kind(EdgeKind::Flow).is_empty());
    }

    #[test]
    fn incoming_adjacency_is_built_and_consistent_with_outgoing() {
        let interner = test_stable_key_interner();
        let store = SemanticGraphStore::from_output(sample_output(), &interner).expect("store");

        // Resolve the dense node ids after normalization by stable key.
        let source = store
            .nodes()
            .iter()
            .find(|n| interner.resolve(n.stable_key).as_ref() == "node|function|a")
            .expect("source node")
            .id;
        let target = store
            .nodes()
            .iter()
            .find(|n| interner.resolve(n.stable_key).as_ref() == "node|callsite|b")
            .expect("target node")
            .id;
        let edge_id = store.edges()[0].id;

        // The source node's outgoing entry contains the edge id, and the target
        // node's incoming entry contains the SAME edge id — the two indexes mirror
        // each other for this fixture edge.
        assert_eq!(store.outgoing_edges(source), &[edge_id]);
        assert_eq!(store.incoming_edges(target), &[edge_id]);
        // The target has no outgoing edges and the source has no incoming edges.
        assert!(store.outgoing_edges(target).is_empty());
        assert!(store.incoming_edges(source).is_empty());
    }

    #[test]
    fn from_output_rejects_dangling_edge_endpoint() {
        let output = SemanticGraphOutput {
            nodes: vec![node(
                0,
                NodeKind::Function(FunctionId::from_raw(1)),
                "node|function|a",
            )],
            // Edge targets node id 5 which does not resolve to a stored node.
            edges: vec![edge(0, 0, 5, EdgeKind::Call, "edge|call|a|missing")],
            constraints: Vec::new(),
        };
        let error = SemanticGraphStore::from_output(output, &test_stable_key_interner())
            .expect_err("dangling endpoint rejected");
        assert!(error.to_string().contains("dangling edge target"));
        assert!(error.to_string().contains("polint.semantic_graph"));
    }

    #[test]
    fn from_output_builds_constraints_by_kind_index() {
        let interner = test_stable_key_interner();
        let store = SemanticGraphStore::from_output(sample_output(), &interner).expect("store");
        assert_eq!(store.constraints().len(), 1);
        // The CallConstraint is indexed by its snake_case tag.
        assert_eq!(
            store
                .constraints_for_kind(&ConstraintKind::CallConstraint {
                    callsite: SemanticNodeId(0),
                })
                .len(),
            1
        );
        // A kind with no rows resolves to an empty slice.
        assert!(
            store
                .constraints_for_kind(&ConstraintKind::ModelEdge {
                    source: SemanticNodeId(0),
                    target: SemanticNodeId(1),
                    language: "typescript".to_string(),
                    scope: "src/app.ts".to_string(),
                    confidence: "heuristic".to_string(),
                    evidence: Vec::new(),
                })
                .is_empty()
        );
        // The stored constraint's callsite was remapped to the post-sort node id of
        // the callsite node (which sorts to index 0 since "node|callsite|b" <
        // "node|function|a").
        let callsite_node = store
            .nodes()
            .iter()
            .find(|n| interner.resolve(n.stable_key).as_ref() == "node|callsite|b")
            .expect("callsite node")
            .id;
        match &store.constraints()[0].kind {
            ConstraintKind::CallConstraint { callsite } => {
                assert_eq!(*callsite, callsite_node);
            }
            other => panic!("expected call_constraint, got {other:?}"),
        }
    }

    #[test]
    fn normalized_constraints_are_shuffle_stable() {
        // Two constraints with distinct stable keys; shuffling input order must yield
        // byte-identical normalized output and identical dense IDs.
        let base = SemanticGraphOutput {
            nodes: vec![
                node(
                    0,
                    NodeKind::Function(FunctionId::from_raw(1)),
                    "node|function|a",
                ),
                node(1, NodeKind::Callsite(CallSiteId(2)), "node|callsite|b"),
            ],
            edges: Vec::new(),
            constraints: vec![
                constraint(
                    5,
                    ConstraintKind::CallConstraint {
                        callsite: SemanticNodeId(1),
                    },
                    "constraint|call_constraint|b",
                ),
                constraint(
                    9,
                    ConstraintKind::CopyEdge {
                        dst: SemanticNodeId(0),
                        src: SemanticNodeId(1),
                    },
                    "constraint|copy_edge|a|b",
                ),
            ],
        };
        let mut shuffled = base.clone();
        shuffled.nodes.reverse();
        shuffled.constraints.reverse();

        let interner = test_stable_key_interner();
        let a = base.normalized(&interner);
        let b = shuffled.normalized(&interner);

        assert_eq!(a.constraints, b.constraints);
        assert_eq!(
            a.constraints.iter().map(|c| c.id).collect::<Vec<_>>(),
            b.constraints.iter().map(|c| c.id).collect::<Vec<_>>()
        );
        // Dense IDs were reassigned by index after the (stable_key, id) sort.
        assert_eq!(a.constraints[0].id, SemanticConstraintId(0));
        assert_eq!(a.constraints[1].id, SemanticConstraintId(1));
    }

    #[test]
    fn from_output_rejects_dangling_constraint_node_ref() {
        let output = SemanticGraphOutput {
            nodes: vec![node(
                0,
                NodeKind::Function(FunctionId::from_raw(1)),
                "node|function|a",
            )],
            edges: Vec::new(),
            // The CopyEdge references node id 7 which does not resolve to a stored
            // node.
            constraints: vec![constraint(
                0,
                ConstraintKind::CopyEdge {
                    dst: SemanticNodeId(0),
                    src: SemanticNodeId(7),
                },
                "constraint|copy_edge|a|missing",
            )],
        };
        let error = SemanticGraphStore::from_output(output, &test_stable_key_interner())
            .expect_err("dangling constraint ref rejected");
        assert!(error.to_string().contains("dangling constraint node"));
        assert!(error.to_string().contains("polint.semantic_graph"));
    }

    #[test]
    fn empty_output_builds_empty_store() {
        let store = SemanticGraphStore::from_output(
            SemanticGraphOutput::empty(),
            &test_stable_key_interner(),
        )
        .expect("empty store");
        assert!(store.nodes().is_empty());
        assert!(store.edges().is_empty());
        assert!(store.constraints().is_empty());
    }
}
