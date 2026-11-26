//! D-09 deletion property for the neutral solver engine.

use crate::analysis_neutral::ids::{SemanticConstraintId, SemanticNodeId};
use crate::analysis_neutral::points_to::facts::{PointsToPrecision, PointsToStatus};
use crate::analysis_neutral::semantic_graph::constraints::{ConstraintFact, ConstraintKind};
use crate::analysis_neutral::solver::budget::SolverBudget;
use crate::analysis_neutral::solver::engine::derive_model_edges;
use crate::internal_core::{StableKeyId, StableKeyInterner};

fn model_constraint(
    interner: &StableKeyInterner,
    stable_key: &str,
    source: u64,
    target: u64,
) -> ConstraintFact {
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
        stable_key: interner.intern(stable_key),
    }
}

/// D-09: deleting ANY single contributing fact INVALIDATES the derived edge.
#[test]
fn deleting_any_contributing_fact_invalidates_the_derived_edge() {
    let interner = StableKeyInterner::default();
    let constraints = vec![
        model_constraint(&interner, "model|a-c", 1, 3),
        model_constraint(&interner, "model|b-d", 2, 4),
    ];
    let budget = SolverBudget::default();

    let baseline = derive_model_edges(&interner, &constraints, &budget);
    let derived = baseline
        .derived_edges
        .iter()
        .find(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(3))
        .expect("baseline derives the modelled a -> c edge");
    let contributing_keys: Vec<StableKeyId> = derived
        .provenance
        .contributing_facts
        .iter()
        .map(|fact| fact.stable_key)
        .collect();
    assert_eq!(contributing_keys.len(), 1);

    for deleted in &contributing_keys {
        let remaining: Vec<ConstraintFact> = constraints
            .iter()
            .filter(|c| c.stable_key != *deleted)
            .cloned()
            .collect();
        assert_eq!(
            remaining.len(),
            1,
            "exactly one contributing constraint removed"
        );

        let rerun = derive_model_edges(&interner, &remaining, &budget);
        let reproduced = rerun
            .derived_edges
            .iter()
            .any(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(3));
        assert!(
            !reproduced,
            "deleting contributing fact `{}` must invalidate the derived a -> c edge",
            interner.resolve(*deleted),
        );
        assert!(
            rerun
                .derived_edges
                .iter()
                .any(|e| e.source == SemanticNodeId(2) && e.target == SemanticNodeId(4)),
            "an edge the deleted fact did not contribute to survives"
        );
    }
}
