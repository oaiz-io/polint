use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::semantic_graph::constraints::ConstraintFact;
use crate::analysis_neutral::semantic_graph::facts::{SemanticEdgeFact, SemanticNodeFact};

/// Produces a byte-stable debug JSON snapshot of all stored semantic-graph facts.
///
/// Rows use the composed stable keys and label strings — never run-local dense IDs
/// as identity, never raw source, never absolute paths. Total-ordered by stable key
/// per family so the snapshot is byte-identical across provider-order / row-order
/// shuffles. Used by the Plan 03 snapshot fixtures.
pub fn metadata_debug_json_for_test(db: &impl AnalysisHost) -> Value {
    let report = SemanticGraphDebugReport {
        counts: counts(db),
        nodes: node_rows(db),
        edges: edge_rows(db),
        constraints: constraint_rows(db),
    };
    serde_json::to_value(report).expect("semantic graph debug report should serialize")
}

#[derive(Serialize)]
struct SemanticGraphDebugReport {
    counts: SemanticGraphCounts,
    nodes: Vec<NodeRow>,
    edges: Vec<EdgeRow>,
    constraints: Vec<ConstraintRow>,
}

#[derive(Default, Serialize)]
struct SemanticGraphCounts {
    total_nodes: usize,
    total_edges: usize,
    total_constraints: usize,
    nodes_by_kind: BTreeMap<String, usize>,
    edges_by_kind: BTreeMap<String, usize>,
    constraints_by_kind: BTreeMap<String, usize>,
}

#[derive(Serialize)]
struct NodeRow {
    kind: String,
    precision: String,
    #[serde(rename = "stable_key")]
    stable_key_text: String,
}

#[derive(Serialize)]
struct EdgeRow {
    kind: String,
    precision: String,
    #[serde(rename = "stable_key")]
    stable_key_text: String,
}

#[derive(Serialize)]
struct ConstraintRow {
    kind: String,
    status: String,
    source: String,
    referenced_nodes: Vec<u64>,
    #[serde(rename = "stable_key")]
    stable_key_text: String,
}

fn counts(db: &impl AnalysisHost) -> SemanticGraphCounts {
    let mut counts = SemanticGraphCounts {
        total_nodes: db.semantic_nodes().len(),
        total_edges: db.semantic_edges().len(),
        total_constraints: db.semantic_constraints().len(),
        ..Default::default()
    };
    for node in db.semantic_nodes() {
        increment(&mut counts.nodes_by_kind, node.kind.as_str());
    }
    for edge in db.semantic_edges() {
        increment(&mut counts.edges_by_kind, edge.kind.as_str());
    }
    for constraint in db.semantic_constraints() {
        increment(&mut counts.constraints_by_kind, constraint.kind.as_str());
    }
    counts
}

fn node_rows(db: &impl AnalysisHost) -> Vec<NodeRow> {
    let interner = db.stable_key_interner();
    let mut rows: Vec<NodeRow> = db
        .semantic_nodes()
        .iter()
        .map(|node: &SemanticNodeFact| NodeRow {
            kind: node.kind.as_str().to_string(),
            precision: node.precision.as_str().to_string(),
            stable_key_text: interner.resolve(node.stable_key).to_string(),
        })
        .collect();
    rows.sort_by(|a, b| a.stable_key_text.cmp(&b.stable_key_text));
    rows
}

fn edge_rows(db: &impl AnalysisHost) -> Vec<EdgeRow> {
    let interner = db.stable_key_interner();
    let mut rows: Vec<EdgeRow> = db
        .semantic_edges()
        .iter()
        .map(|edge: &SemanticEdgeFact| EdgeRow {
            kind: edge.kind.as_str().to_string(),
            precision: edge.precision.as_str().to_string(),
            stable_key_text: interner.resolve(edge.stable_key).to_string(),
        })
        .collect();
    rows.sort_by(|a, b| a.stable_key_text.cmp(&b.stable_key_text));
    rows
}

fn constraint_rows(db: &impl AnalysisHost) -> Vec<ConstraintRow> {
    let interner = db.stable_key_interner();
    let mut rows: Vec<ConstraintRow> = db
        .semantic_constraints()
        .iter()
        .map(|constraint: &ConstraintFact| ConstraintRow {
            kind: constraint.kind.as_str().to_string(),
            status: format!("{:?}", constraint.status),
            source: if interner.resolve(constraint.stable_key).contains(&format!(
                "|6:origin={}:{}",
                super::build::TS_DIRECT_BINDING_ORIGIN.len(),
                super::build::TS_DIRECT_BINDING_ORIGIN
            )) {
                super::build::TS_DIRECT_BINDING_ORIGIN.to_string()
            } else {
                "semantic_graph".to_string()
            },
            referenced_nodes: constraint
                .kind
                .referenced_nodes()
                .into_iter()
                .map(|node| node.0)
                .collect(),
            stable_key_text: interner.resolve(constraint.stable_key).to_string(),
        })
        .collect();
    rows.sort_by(|a, b| a.stable_key_text.cmp(&b.stable_key_text));
    rows
}

fn increment(counts: &mut BTreeMap<String, usize>, key: &str) {
    *counts.entry(key.to_string()).or_default() += 1;
}

#[cfg(test)]
mod tests {
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::ids::SemanticNodeId;
    use crate::analysis_neutral::points_to::facts::{PointsToPrecision, PointsToStatus};
    use crate::analysis_neutral::semantic_graph::constraints::{ConstraintFact, ConstraintKind};
    use crate::analysis_neutral::semantic_graph::facts::{
        NodeKind, SemanticNodeFact, SemanticPrecision,
    };
    use crate::analysis_neutral::semantic_graph::store::SemanticGraphOutput;
    use crate::internal_core::FunctionId;

    use super::*;

    #[test]
    fn debug_json_marks_ts_direct_binding_constraints_with_node_refs() {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        let output = SemanticGraphOutput {
            nodes: vec![
                node(
                    &interner,
                    0,
                    FunctionId::from_raw(1),
                    "node:function:target",
                ),
                node(
                    &interner,
                    1,
                    FunctionId::from_raw(2),
                    "node:function:callsite",
                ),
            ],
            edges: Vec::new(),
            constraints: vec![ConstraintFact {
                id: Default::default(),
                kind: ConstraintKind::CopyEdge {
                    dst: SemanticNodeId(1),
                    src: SemanticNodeId(0),
                },
                status: PointsToStatus::Present,
                precision: PointsToPrecision::FlowInsensitive,
                stable_key: interner.intern("16:PointsToConstraint|6:origin=17:ts_direct_binding"),
            }],
        };
        crate::analysis_neutral::AnalysisHost::replace_semantic_graph_facts(&mut db, output)
            .expect("store graph");

        let json = metadata_debug_json_for_test(&db);
        let constraints = json["constraints"].as_array().expect("constraints array");
        assert_eq!(constraints[0]["source"], "ts_direct_binding");
        assert_eq!(
            constraints[0]["referenced_nodes"].as_array().unwrap().len(),
            2
        );
    }

    fn node(
        interner: &crate::internal_core::StableKeyInterner,
        id: u64,
        function: FunctionId,
        stable_key: &str,
    ) -> SemanticNodeFact {
        SemanticNodeFact {
            id: SemanticNodeId(id),
            kind: NodeKind::Function(function),
            precision: SemanticPrecision::Conservative,
            stable_key: interner.intern(stable_key),
        }
    }
}
