use crate::analysis_neutral::AnalysisHost;
use std::collections::BTreeSet;

use crate::analysis_api::FactPrecision;
use crate::analysis_neutral::ids::SemanticNodeId;
use crate::analysis_neutral::semantic_graph::constraints::{ConstraintFact, ConstraintKind};
use crate::analysis_neutral::semantic_graph::facts::{
    SemanticEdgeFact, SemanticNodeFact, SemanticPrecision,
};
use crate::analysis_neutral::semantic_graph::store::SEMANTIC_GRAPH_PROVIDER_ID;
use crate::internal_core::{Diagnostic, DiagnosticRange as TextRange};

/// Validates the stored semantic-graph facts, emitting an evidence-bearing
/// diagnostic per problem rather than silently dropping a malformed row (D-15):
///
/// - duplicate stable keys across nodes / edges / constraints,
/// - dangling edge endpoints and dangling constraint node references,
/// - dense IDs that are not contiguous (`0..n`) and stable-key-sorted, and
/// - the precision ceiling: a derived node/edge must NOT claim the exact-equivalent
///   precision tier (`SemanticPrecision::ResolvedStatic`) — graph rows are
///   derived/aggregated, never exact (D-07).
///
/// Failures surface as structured [`Diagnostic`]s carrying `family`/`stable_key`/
/// `field`/`reason` evidence, mirroring `reachability::validate`.
/// The most arguments a stored call constraint may carry: more than any call
/// a frontend lowers, so a longer list is a corrupt row.
const MAX_CALL_ARGUMENTS: usize = 255;

pub fn validate_semantic_graph(db: &impl AnalysisHost, diagnostics: &mut Vec<Diagnostic>) {
    validate_semantic_graph_rows(
        &db.stable_key_interner(),
        db.semantic_nodes(),
        db.semantic_edges(),
        db.semantic_constraints(),
        diagnostics,
    );
}

fn validate_semantic_graph_rows(
    interner: &crate::internal_core::StableKeyInterner,
    nodes: &[SemanticNodeFact],
    edges: &[SemanticEdgeFact],
    constraints: &[ConstraintFact],
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Duplicate stable keys per family.
    check_duplicate_stable_keys(
        diagnostics,
        "SemanticNode",
        nodes.iter().map(|row| interner.resolve(row.stable_key)),
    );
    check_duplicate_stable_keys(
        diagnostics,
        "SemanticEdge",
        edges.iter().map(|row| interner.resolve(row.stable_key)),
    );
    check_duplicate_stable_keys(
        diagnostics,
        "SemanticConstraint",
        constraints
            .iter()
            .map(|row| interner.resolve(row.stable_key)),
    );

    // Dense IDs contiguous (0..n) and stable-key-sorted, per family.
    check_dense_ids_sorted(
        diagnostics,
        "SemanticNode",
        nodes
            .iter()
            .map(|row| (row.id.0, interner.resolve(row.stable_key))),
    );
    check_dense_ids_sorted(
        diagnostics,
        "SemanticEdge",
        edges
            .iter()
            .map(|row| (row.id.0, interner.resolve(row.stable_key))),
    );
    check_dense_ids_sorted(
        diagnostics,
        "SemanticConstraint",
        constraints
            .iter()
            .map(|row| (row.id.0, interner.resolve(row.stable_key))),
    );

    let node_ids: BTreeSet<SemanticNodeId> = nodes.iter().map(|node| node.id).collect();

    // Referential checks + precision ceiling on nodes.
    for node in nodes {
        let stable_key = interner.resolve(node.stable_key);
        if let Some(diagnostic) =
            reject_exact_node_edge_precision("SemanticNode", node.precision, &stable_key)
        {
            diagnostics.push(diagnostic);
        }
    }

    // Edge endpoint referential checks + precision ceiling.
    for edge in edges {
        let stable_key = interner.resolve(edge.stable_key);
        if !node_ids.contains(&edge.source) {
            push_diagnostic(
                diagnostics,
                "SemanticEdge",
                &stable_key,
                "source",
                "dangling semantic edge source node reference",
            );
        }
        if !node_ids.contains(&edge.target) {
            push_diagnostic(
                diagnostics,
                "SemanticEdge",
                &stable_key,
                "target",
                "dangling semantic edge target node reference",
            );
        }
        if let Some(diagnostic) =
            reject_exact_node_edge_precision("SemanticEdge", edge.precision, &stable_key)
        {
            diagnostics.push(diagnostic);
        }
    }

    // Constraint node-reference referential checks. `TypeConstraint.type_fact`
    // references the type substrate (a different family) and is intentionally not
    // checked against the node set here.
    for constraint in constraints {
        let stable_key = interner.resolve(constraint.stable_key);
        if let ConstraintKind::CallConstraint { arguments, .. } = &constraint.kind
            && arguments.len() > MAX_CALL_ARGUMENTS
        {
            push_diagnostic(
                diagnostics,
                "SemanticConstraint",
                &stable_key,
                "arguments",
                "call constraint has more arguments than a call can pass",
            );
        }
        for node in constraint.kind.referenced_nodes() {
            if !node_ids.contains(&node) {
                push_diagnostic(
                    diagnostics,
                    "SemanticConstraint",
                    &stable_key,
                    "node_ref",
                    "dangling semantic constraint node reference",
                );
            }
        }
    }
}

/// Precision-ceiling check for derived semantic-graph nodes/edges (D-07).
///
/// Graph rows are derived/aggregated — they must never claim the exact-equivalent
/// precision tier. `SemanticPrecision` carries no literal `Exact` variant by design
/// (see `facts.rs`); `ResolvedStatic` is its exact-equivalent ceiling, so this
/// rejects exactly that tier. Returns a diagnostic when the ceiling is exceeded.
fn reject_exact_node_edge_precision(
    family: &'static str,
    precision: SemanticPrecision,
    stable_key: &str,
) -> Option<Diagnostic> {
    if precision == SemanticPrecision::ResolvedStatic {
        Some(precision_ceiling_diagnostic(family, stable_key))
    } else {
        None
    }
}

/// Literal `FactPrecision::Exact` ceiling check, mirroring
/// `reachability::validate::reject_exact_precision`. The semantic-graph fact types
/// carry no `FactPrecision`-typed precision field of their own, but this helper
/// preserves the established producer-level contract: a derived semantic-graph
/// producer must never be reported as [`FactPrecision::Exact`].
pub fn reject_exact_precision(precision: FactPrecision, stable_key: &str) -> Option<Diagnostic> {
    if precision == FactPrecision::Exact {
        Some(precision_ceiling_diagnostic("SemanticNode", stable_key))
    } else {
        None
    }
}

fn check_duplicate_stable_keys(
    diagnostics: &mut Vec<Diagnostic>,
    family: &'static str,
    keys: impl Iterator<Item = std::sync::Arc<str>>,
) {
    let mut seen = BTreeSet::new();
    for key in keys {
        if !seen.insert(key.clone()) {
            push_diagnostic(
                diagnostics,
                family,
                &key,
                "stable_key",
                "duplicate stable key",
            );
        }
    }
}

/// Asserts the dense IDs are exactly `0..n` in stable-key order: each row's dense id
/// must equal its index, and the stable keys must be non-decreasing. A mismatch means
/// `normalized()` was skipped or the store was mutated out of band.
fn check_dense_ids_sorted(
    diagnostics: &mut Vec<Diagnostic>,
    family: &'static str,
    rows: impl Iterator<Item = (u64, std::sync::Arc<str>)>,
) {
    let mut previous_key: Option<std::sync::Arc<str>> = None;
    for (index, (id, stable_key)) in rows.enumerate() {
        if id != index as u64 {
            push_diagnostic(
                diagnostics,
                family,
                &stable_key,
                "id",
                "dense id is not contiguous with the stable-key sort order",
            );
        }
        if let Some(previous) = &previous_key
            && previous > &stable_key
        {
            push_diagnostic(
                diagnostics,
                family,
                &stable_key,
                "stable_key",
                "rows are not sorted by stable key",
            );
        }
        previous_key = Some(stable_key);
    }
}

fn push_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    family: &'static str,
    stable_key: &str,
    field: &'static str,
    reason: &'static str,
) {
    diagnostics.push(
        Diagnostic::error(
            "polint/internal",
            "<workspace>",
            TextRange::point(1, 1),
            format!("Semantic-graph validation failed for {family} stable key."),
        )
        .with_evidence("family", family)
        .with_evidence("stable_key", stable_key.to_string())
        .with_evidence("field", field)
        .with_evidence("reason", reason),
    );
}

fn precision_ceiling_diagnostic(family: &'static str, stable_key: &str) -> Diagnostic {
    Diagnostic::error(
        "polint/internal",
        "<workspace>",
        TextRange::point(1, 1),
        format!("Semantic-graph precision ceiling exceeded for {SEMANTIC_GRAPH_PROVIDER_ID}."),
    )
    .with_evidence("family", family)
    .with_evidence("stable_key", stable_key.to_string())
    .with_evidence("field", "precision")
    .with_evidence(
        "reason",
        "precision ceiling exceeded: semantic-graph facts must not claim Exact",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::ids::{CallSiteId, SemanticNodeId};
    use crate::analysis_neutral::points_to::facts::{PointsToPrecision, PointsToStatus};

    use crate::analysis_neutral::semantic_graph::facts::{
        EdgeKind, NodeKind, SemanticEdgeFact, SemanticNodeFact,
    };
    use crate::analysis_neutral::semantic_graph::store::SemanticGraphOutput;
    use crate::internal_core::{FunctionId, stable_key_for_test, test_stable_key_interner};

    fn node(kind: NodeKind, precision: SemanticPrecision, stable_key: &str) -> SemanticNodeFact {
        SemanticNodeFact {
            id: Default::default(),
            kind,
            precision,
            stable_key: stable_key_for_test(stable_key),
        }
    }

    fn store_db(mut output: SemanticGraphOutput) -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        let source = test_stable_key_interner();
        let target = db.stable_key_interner();
        for row in &mut output.nodes {
            row.stable_key = target.intern(source.resolve(row.stable_key).to_string());
        }
        for row in &mut output.edges {
            row.stable_key = target.intern(source.resolve(row.stable_key).to_string());
        }
        for row in &mut output.constraints {
            row.stable_key = target.intern(source.resolve(row.stable_key).to_string());
        }
        db.replace_semantic_graph_facts(output)
            .expect("semantic graph facts store");
        db
    }

    #[test]
    fn valid_graph_produces_no_diagnostics() {
        let output = SemanticGraphOutput {
            nodes: vec![
                node(
                    NodeKind::Function(FunctionId::from_raw(1)),
                    SemanticPrecision::Conservative,
                    "node|function|a",
                ),
                node(
                    NodeKind::Callsite(CallSiteId(2)),
                    SemanticPrecision::Conservative,
                    "node|callsite|b",
                ),
            ],
            edges: vec![SemanticEdgeFact {
                id: Default::default(),
                source: SemanticNodeId(0),
                target: SemanticNodeId(1),
                kind: EdgeKind::Call,
                precision: SemanticPrecision::Conservative,
                stable_key: stable_key_for_test("edge|call|a|b"),
            }],
            constraints: vec![ConstraintFact {
                id: Default::default(),
                kind: ConstraintKind::call_at(SemanticNodeId(1)),
                status: PointsToStatus::Present,
                precision: PointsToPrecision::FlowInsensitive,
                stable_key: stable_key_for_test("constraint|call_constraint|b"),
            }],
        };
        let db = store_db(output);
        let mut diagnostics = Vec::new();
        validate_semantic_graph(&db, &mut diagnostics);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn exact_equivalent_precision_node_is_rejected() {
        // A derived node claiming the exact-equivalent precision tier
        // (ResolvedStatic) must produce a precision-ceiling diagnostic.
        let output = SemanticGraphOutput {
            nodes: vec![node(
                NodeKind::Function(FunctionId::from_raw(1)),
                SemanticPrecision::ResolvedStatic,
                "node|function|exact",
            )],
            edges: Vec::new(),
            constraints: Vec::new(),
        };
        let db = store_db(output);
        let mut diagnostics = Vec::new();
        validate_semantic_graph(&db, &mut diagnostics);
        assert!(
            diagnostics
                .iter()
                .any(|d| format!("{d:?}").contains("must not claim Exact")),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn precision_ceiling_helper_rejects_fact_precision_exact() {
        assert!(reject_exact_precision(FactPrecision::Exact, "k").is_some());
        assert!(reject_exact_precision(FactPrecision::SetupAware, "k").is_none());
        assert!(reject_exact_precision(FactPrecision::Heuristic, "k").is_none());
    }

    #[test]
    fn validation_rejects_dangling_ts_direct_binding_constraint_endpoint() {
        let nodes = vec![node(
            NodeKind::Function(FunctionId::from_raw(1)),
            SemanticPrecision::Conservative,
            "node|function|target",
        )];
        let constraints = vec![ConstraintFact {
            id: Default::default(),
            kind: ConstraintKind::CopyEdge {
                dst: SemanticNodeId(0),
                src: SemanticNodeId(99),
            },
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            stable_key: stable_key_for_test("constraint|ts_direct_binding|dangling"),
        }];
        let mut diagnostics = Vec::new();

        validate_semantic_graph_rows(
            &test_stable_key_interner(),
            &nodes,
            &[],
            &constraints,
            &mut diagnostics,
        );

        assert!(
            diagnostics.iter().any(|diagnostic| {
                format!("{diagnostic:?}").contains("dangling semantic constraint node reference")
            }),
            "{diagnostics:?}"
        );
    }

    fn call_constraint(kind: ConstraintKind, stable_key: &str) -> ConstraintFact {
        ConstraintFact {
            id: Default::default(),
            kind,
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            stable_key: stable_key_for_test(stable_key),
        }
    }

    fn rejections(
        constraints: &[ConstraintFact],
        nodes: &[SemanticNodeFact],
        reason: &str,
    ) -> usize {
        let mut diagnostics = Vec::new();
        validate_semantic_graph_rows(
            &test_stable_key_interner(),
            nodes,
            &[],
            constraints,
            &mut diagnostics,
        );
        diagnostics
            .iter()
            .filter(|diagnostic| format!("{diagnostic:?}").contains(reason))
            .count()
    }

    #[test]
    fn validation_rejects_a_call_constraint_with_more_than_255_arguments() {
        let nodes = vec![node(
            NodeKind::Callsite(CallSiteId(1)),
            SemanticPrecision::Conservative,
            "node|callsite|call",
        )];
        let call = |count: usize, key: &str| {
            call_constraint(
                ConstraintKind::CallConstraint {
                    callsite: SemanticNodeId(0),
                    callee: None,
                    receiver: None,
                    arguments: vec![SemanticNodeId(0); count],
                    spread_from: None,
                    result: None,
                },
                key,
            )
        };
        let reason = "call constraint has more arguments than a call can pass";

        assert_eq!(
            rejections(&[call(255, "constraint|call|255")], &nodes, reason),
            0
        );
        assert_eq!(
            rejections(&[call(256, "constraint|call|256")], &nodes, reason),
            1
        );
    }

    #[test]
    fn validation_rejects_dangling_call_constraint_places() {
        let nodes = vec![node(
            NodeKind::Callsite(CallSiteId(1)),
            SemanticPrecision::Conservative,
            "node|callsite|call",
        )];
        let constraints = vec![call_constraint(
            ConstraintKind::CallConstraint {
                callsite: SemanticNodeId(0),
                callee: Some(SemanticNodeId(91)),
                receiver: Some(SemanticNodeId(92)),
                arguments: vec![SemanticNodeId(0), SemanticNodeId(93)],
                spread_from: None,
                result: Some(SemanticNodeId(94)),
            },
            "constraint|call|dangling",
        )];

        assert_eq!(
            rejections(
                &constraints,
                &nodes,
                "dangling semantic constraint node reference"
            ),
            4
        );
    }

    #[test]
    fn validation_rejects_dangling_object_model_constraint_endpoint() {
        let nodes = vec![node(
            NodeKind::Callsite(CallSiteId(1)),
            SemanticPrecision::Conservative,
            "node|callsite|target",
        )];
        let constraints = vec![
            ConstraintFact {
                id: Default::default(),
                kind: ConstraintKind::Alloc {
                    dst: SemanticNodeId(0),
                    object: SemanticNodeId(99),
                },
                status: PointsToStatus::Present,
                precision: PointsToPrecision::FlowInsensitive,
                stable_key: stable_key_for_test("constraint|object_model|dangling_alloc"),
            },
            ConstraintFact {
                id: Default::default(),
                kind: ConstraintKind::FieldLoad {
                    dst: SemanticNodeId(98),
                    base: SemanticNodeId(0),
                    field: "computed_bucket".to_string(),
                },
                status: PointsToStatus::Present,
                precision: PointsToPrecision::FlowInsensitive,
                stable_key: stable_key_for_test("constraint|object_model|dangling_load"),
            },
            ConstraintFact {
                id: Default::default(),
                kind: ConstraintKind::FieldStore {
                    base: SemanticNodeId(0),
                    field: "static:target".to_string(),
                    src: SemanticNodeId(97),
                },
                status: PointsToStatus::Present,
                precision: PointsToPrecision::FlowInsensitive,
                stable_key: stable_key_for_test("constraint|object_model|dangling_store"),
            },
        ];
        let mut diagnostics = Vec::new();

        validate_semantic_graph_rows(
            &test_stable_key_interner(),
            &nodes,
            &[],
            &constraints,
            &mut diagnostics,
        );

        let dangling_count = diagnostics
            .iter()
            .filter(|diagnostic| {
                format!("{diagnostic:?}").contains("dangling semantic constraint node reference")
            })
            .count();
        assert_eq!(dangling_count, 3, "{diagnostics:?}");
    }

    #[test]
    fn conservative_precision_node_is_within_ceiling() {
        assert!(
            reject_exact_node_edge_precision("SemanticNode", SemanticPrecision::Conservative, "k")
                .is_none()
        );
        assert!(
            reject_exact_node_edge_precision("SemanticEdge", SemanticPrecision::SetupAware, "k")
                .is_none()
        );
    }
}
