//! TypeScript/JavaScript field-sensitive points-to projection for the unified solver.
//!
//! This module owns the frontend-specific projection from semantic-graph constraints
//! to Andersen constraints and the derived indirect-call edges.

use std::collections::BTreeMap;

use crate::analysis_neutral::AnalysisHost;

use crate::analysis_api::{FactFamily, stable_key_from_parts};
use crate::analysis_neutral::ids::{
    DerivedEdgeId, ObjectTokenId, PointsToConstraintId, PtVarId, SemanticNodeId,
};
use crate::analysis_neutral::points_to::facts::{
    PointsToConstraintFact, PointsToConstraintKind, PointsToPrecision, PointsToStatus,
};
use crate::analysis_neutral::points_to::solver::{PointsToSolveResult, solve_points_to};
use crate::analysis_neutral::semantic_graph::constraints::{ConstraintFact, ConstraintKind};
use crate::analysis_neutral::semantic_graph::facts::NodeKind;
use crate::analysis_neutral::solver::budget::{BudgetStatus, SolverBudget};
use crate::analysis_neutral::solver::facts::DerivedEdgeFact;
use crate::analysis_neutral::solver::provenance::{ContributingFact, DerivedEdgeProvenance};
use crate::internal_core::StableKeyId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsPointsToCallsite {
    caller_node: SemanticNodeId,
    callsite_node: SemanticNodeId,
    callsite_stable_key: StableKeyId,
    constraint_stable_key: StableKeyId,
    precision: PointsToPrecision,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TsPointsToInputs {
    constraints: Vec<ConstraintFact>,
    callable_objects: BTreeMap<ObjectTokenId, (SemanticNodeId, StableKeyId)>,
    callsites: Vec<TsPointsToCallsite>,
}

impl TsPointsToInputs {
    pub fn from_db(db: &impl AnalysisHost) -> Self {
        let function_node_by_id = db
            .semantic_nodes()
            .iter()
            .filter_map(|node| match node.kind {
                NodeKind::Function(function) => Some((function, node.id)),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        let callsite_node_by_id = db
            .semantic_nodes()
            .iter()
            .filter_map(|node| match node.kind {
                NodeKind::Callsite(callsite) => Some((callsite, node.id)),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        let node_key_by_id = db
            .semantic_nodes()
            .iter()
            .map(|node| (node.id, node.stable_key))
            .collect::<BTreeMap<_, _>>();
        let constraint_key_by_callsite = db
            .semantic_constraints()
            .iter()
            .filter_map(|constraint| match constraint.kind {
                ConstraintKind::CallConstraint { callsite } => {
                    Some((callsite, constraint.stable_key))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();

        let mut precision_by_callsite = BTreeMap::<SemanticNodeId, PointsToPrecision>::new();
        for constraint in db.semantic_constraints() {
            if let ConstraintKind::CallConstraint { callsite } = constraint.kind {
                precision_by_callsite
                    .entry(callsite)
                    .and_modify(|precision| *precision = (*precision).max(constraint.precision))
                    .or_insert(constraint.precision);
            }
        }

        let callable_objects = db
            .semantic_nodes()
            .iter()
            .filter_map(|node| match node.kind {
                NodeKind::Function(_) => {
                    Some((object_for_node(node.id), (node.id, node.stable_key)))
                }
                _ => None,
            })
            .collect();
        let callsites = db
            .call_sites()
            .iter()
            .filter(|site| site.language.is_ts_family())
            .filter_map(|site| {
                let callsite_node = *callsite_node_by_id.get(&site.id)?;
                let caller_node = *function_node_by_id.get(&site.caller)?;
                Some(TsPointsToCallsite {
                    caller_node,
                    callsite_node,
                    precision: precision_by_callsite
                        .get(&callsite_node)
                        .copied()
                        .unwrap_or(PointsToPrecision::FlowInsensitive),
                    callsite_stable_key: *node_key_by_id.get(&callsite_node)?,
                    constraint_stable_key: constraint_key_by_callsite
                        .get(&callsite_node)
                        .copied()
                        .unwrap_or(site.stable_key),
                })
            })
            .collect();

        Self {
            constraints: db.semantic_constraints().to_vec(),
            callable_objects,
            callsites,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsPointsToSolveResult {
    pub points_to: PointsToSolveResult,
    pub derived_edges: Vec<DerivedEdgeFact>,
}

pub fn solve_ts_points_to(
    interner: &crate::internal_core::StableKeyInterner,
    inputs: &TsPointsToInputs,
    budget: &SolverBudget,
) -> TsPointsToSolveResult {
    let constraints = project_constraints(interner, inputs);
    let points_to = solve_points_to(interner, &constraints, budget.points_to_budget());
    let sets = points_to
        .sets
        .iter()
        .map(|set| (set.variable, set))
        .collect::<BTreeMap<_, _>>();
    // Membership-only dedup by StableKeyId; emission order is assigned below via
    // `SolverOutput::normalized` (resolved-text sort), never allocation order.
    let mut edges = std::collections::HashMap::new();

    for callsite in &inputs.callsites {
        let Some(set) = sets.get(&var_for_node(callsite.callsite_node)) else {
            continue;
        };
        if set.status != PointsToStatus::Present {
            continue;
        }
        for object in &set.objects {
            let Some((target, target_stable_key)) = inputs.callable_objects.get(object) else {
                continue;
            };
            let Some((_, caller_stable_key)) = inputs
                .callable_objects
                .get(&object_for_node(callsite.caller_node))
            else {
                continue;
            };
            let provenance = DerivedEdgeProvenance::new(
                interner,
                vec![
                    ContributingFact {
                        stable_key: callsite.constraint_stable_key,
                    },
                    ContributingFact {
                        stable_key: callsite.callsite_stable_key,
                    },
                    ContributingFact {
                        stable_key: *target_stable_key,
                    },
                    ContributingFact {
                        stable_key: set.stable_key,
                    },
                ],
                &ConstraintKind::CallConstraint {
                    callsite: callsite.callsite_node,
                },
                0,
            );
            let stable_key = stable_key_from_parts(
                interner,
                FactFamily::SolverDerivedEdge,
                &[
                    ("source", interner.resolve(*caller_stable_key).to_string()),
                    ("target", interner.resolve(*target_stable_key).to_string()),
                    ("provenance", provenance.stable_key_fragment(interner)),
                ],
            );
            edges.entry(stable_key).or_insert(DerivedEdgeFact {
                id: DerivedEdgeId(0),
                source: callsite.caller_node,
                target: *target,
                status: set.status,
                precision: set.precision.max(callsite.precision),
                stable_key,
                provenance,
            });
        }
    }

    // Direct/policy consumers must never observe allocation-order emission.
    let derived_edges = crate::analysis_neutral::solver::store::SolverOutput {
        derived_edges: edges.into_values().collect(),
        ..crate::analysis_neutral::solver::store::SolverOutput::default()
    }
    .normalized(interner)
    .derived_edges;

    TsPointsToSolveResult {
        points_to,
        derived_edges,
    }
}

fn project_constraints(
    interner: &crate::internal_core::StableKeyInterner,
    inputs: &TsPointsToInputs,
) -> Vec<PointsToConstraintFact> {
    let mut projected = BTreeMap::new();
    for (&object, &(node, node_stable_key)) in &inputs.callable_objects {
        insert_projected_constraint(
            interner,
            &mut projected,
            node_stable_key,
            "callable_object",
            PointsToConstraintKind::AddressOf {
                dst: var_for_node(node),
                object,
            },
        );
    }
    for constraint in &inputs.constraints {
        if constraint.status != PointsToStatus::Present {
            continue;
        }
        match &constraint.kind {
            ConstraintKind::CopyEdge { dst, src } => {
                insert_projected_constraint(
                    interner,
                    &mut projected,
                    constraint.stable_key,
                    "copy",
                    PointsToConstraintKind::Copy {
                        dst: var_for_node(*dst),
                        src: var_for_node(*src),
                    },
                );
            }
            ConstraintKind::Alloc { dst, object } => {
                let object_token = object_for_node(*object);
                insert_projected_constraint(
                    interner,
                    &mut projected,
                    constraint.stable_key,
                    "allocation_object",
                    PointsToConstraintKind::AddressOf {
                        dst: var_for_node(*object),
                        object: object_token,
                    },
                );
                insert_projected_constraint(
                    interner,
                    &mut projected,
                    constraint.stable_key,
                    "allocation_destination",
                    PointsToConstraintKind::AddressOf {
                        dst: var_for_node(*dst),
                        object: object_token,
                    },
                );
            }
            ConstraintKind::FieldLoad { dst, base, field } => {
                insert_projected_constraint(
                    interner,
                    &mut projected,
                    constraint.stable_key,
                    "field_load",
                    PointsToConstraintKind::FieldLoad {
                        dst: var_for_node(*dst),
                        base: var_for_node(*base),
                        field: field.clone(),
                    },
                );
            }
            ConstraintKind::FieldStore { base, field, src } => {
                insert_projected_constraint(
                    interner,
                    &mut projected,
                    constraint.stable_key,
                    "field_store",
                    PointsToConstraintKind::FieldStore {
                        base: var_for_node(*base),
                        field: field.clone(),
                        src: var_for_node(*src),
                    },
                );
            }
            ConstraintKind::CallConstraint { .. }
            | ConstraintKind::ModelEdge { .. }
            | ConstraintKind::TypeConstraint { .. } => {}
        }
    }

    projected
        .into_iter()
        .enumerate()
        .map(|(index, (_, (stable_key, kind)))| PointsToConstraintFact {
            id: PointsToConstraintId(index as u64),
            stable_key,
            kind,
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
        })
        .collect()
}

fn insert_projected_constraint(
    interner: &crate::internal_core::StableKeyInterner,
    projected: &mut BTreeMap<String, (StableKeyId, PointsToConstraintKind)>,
    source_stable_key: StableKeyId,
    relation: &str,
    kind: PointsToConstraintKind,
) {
    let stable_key = stable_key_from_parts(
        interner,
        FactFamily::PointsToConstraint,
        &[
            ("source", interner.resolve(source_stable_key).to_string()),
            ("relation", relation.to_string()),
        ],
    );
    projected
        .entry(interner.resolve(stable_key).to_string())
        .or_insert((stable_key, kind));
}

fn var_for_node(node: SemanticNodeId) -> PtVarId {
    crate::analysis_neutral::points_to::vars::semantic_node_var(node)
}

fn object_for_node(node: SemanticNodeId) -> ObjectTokenId {
    ObjectTokenId(node.0)
}

pub fn budget_status(result: &PointsToSolveResult) -> BudgetStatus {
    BudgetStatus::from_points_to(result.budget_status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_slots_cannot_alias_unrelated_semantic_nodes() {
        let interner = crate::internal_core::test_stable_key_interner();
        let caller = SemanticNodeId(1);
        let target = SemanticNodeId(2);
        let loaded = SemanticNodeId(3);
        let object = SemanticNodeId(4);
        // The first dynamic heap slot has raw ID 5. A semantic node numbered
        // 5 is an unrelated variable and must never inherit that slot's value.
        let unrelated = SemanticNodeId(5);
        let invoked = SemanticNodeId(6);
        let constraints = [
            (
                "allocate",
                ConstraintKind::Alloc {
                    dst: object,
                    object,
                },
            ),
            (
                "store",
                ConstraintKind::FieldStore {
                    base: object,
                    field: "method".into(),
                    src: target,
                },
            ),
            (
                "load",
                ConstraintKind::FieldLoad {
                    dst: loaded,
                    base: object,
                    field: "method".into(),
                },
            ),
            (
                "callee",
                ConstraintKind::CopyEdge {
                    dst: invoked,
                    src: loaded,
                },
            ),
            (
                "call:unrelated",
                ConstraintKind::CallConstraint {
                    callsite: unrelated,
                },
            ),
            (
                "call:invoked",
                ConstraintKind::CallConstraint { callsite: invoked },
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (key, kind))| ConstraintFact {
            id: crate::analysis_neutral::ids::SemanticConstraintId(index as u64),
            kind,
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            stable_key: interner.intern(key),
        })
        .collect();
        let inputs = TsPointsToInputs {
            constraints,
            callable_objects: BTreeMap::from([
                (
                    object_for_node(caller),
                    (caller, interner.intern("function:caller")),
                ),
                (
                    object_for_node(target),
                    (target, interner.intern("function:target")),
                ),
            ]),
            callsites: [unrelated, invoked]
                .into_iter()
                .map(|node| TsPointsToCallsite {
                    precision: PointsToPrecision::FlowInsensitive,
                    caller_node: caller,
                    callsite_node: node,
                    callsite_stable_key: interner.intern(format!("site:{}", node.0)),
                    constraint_stable_key: interner.intern(format!("constraint:{}", node.0)),
                })
                .collect(),
        };
        let result = solve_ts_points_to(&interner, &inputs, &SolverBudget::default());
        assert_eq!(budget_status(&result.points_to), BudgetStatus::WithinBudget);
        assert!(
            result
                .points_to
                .sets
                .iter()
                .all(|set| { set.variable != var_for_node(unrelated) || set.objects.is_empty() })
        );
        assert!(result.points_to.sets.iter().any(|set| {
            set.variable == var_for_node(invoked) && set.objects == [object_for_node(target)]
        }));
        assert_eq!(result.derived_edges.len(), 1);
        assert_eq!(result.derived_edges[0].target, target);
    }
}
