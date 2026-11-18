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
    PointsToBudgetStatus, PointsToConstraintFact, PointsToConstraintKind, PointsToPrecision,
    PointsToSetFact, PointsToStatus,
};
use crate::analysis_neutral::points_to::solver::{
    PointsToCall, PointsToCallInputs, PointsToSignature, PointsToSolveResult, SignaturePlace,
    solve_points_to_logged,
};
use crate::analysis_neutral::points_to::tiers::{Assumption, EdgeClaim};
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
    /// The parameter, rest, `this` and return places of each function, keyed
    /// by the function's object.
    signatures: BTreeMap<ObjectTokenId, PointsToSignature>,
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
                ConstraintKind::CallConstraint { callsite, .. } => {
                    Some((callsite, constraint.stable_key))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();

        let mut precision_by_callsite = BTreeMap::<SemanticNodeId, PointsToPrecision>::new();
        for constraint in db.semantic_constraints() {
            if let ConstraintKind::CallConstraint { callsite, .. } = constraint.kind {
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

        let mut signatures = BTreeMap::<ObjectTokenId, PointsToSignature>::new();
        for node in db.semantic_nodes() {
            let function = match node.kind {
                NodeKind::FunctionParam(function, _)
                | NodeKind::FunctionRest(function)
                | NodeKind::FunctionThis(function)
                | NodeKind::FunctionReturn(function) => function,
                _ => continue,
            };
            let Some(function_node) = function_node_by_id.get(&function) else {
                continue;
            };
            let signature = signatures
                .entry(object_for_node(*function_node))
                .or_default();
            let place = SignaturePlace {
                var: var_for_node(node.id),
                stable_key: node.stable_key,
            };
            match node.kind {
                NodeKind::FunctionParam(_, position) => {
                    signature.params.insert(usize::from(position), place);
                }
                NodeKind::FunctionRest(_) => signature.rest = Some(place),
                NodeKind::FunctionThis(_) => signature.this = Some(place),
                NodeKind::FunctionReturn(_) => signature.ret = Some(place),
                _ => {}
            }
        }

        Self {
            constraints: db.semantic_constraints().to_vec(),
            callable_objects,
            callsites,
            signatures,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsPointsToSolveResult {
    pub points_to: PointsToSolveResult,
    pub derived_edges: Vec<DerivedEdgeFact>,
    /// Call sites whose callee set the solve left incomplete under its budget
    /// or indirection bound.
    pub incomplete_sites: u64,
}

pub fn solve_ts_points_to(
    interner: &crate::internal_core::StableKeyInterner,
    inputs: &TsPointsToInputs,
    budget: &SolverBudget,
) -> TsPointsToSolveResult {
    let constraints = project_constraints(interner, inputs);
    let calls = project_calls(inputs);
    let points_to = solve_points_to_logged(
        "call_graph",
        interner,
        &constraints,
        &calls,
        budget.points_to_budget(),
    );
    let sets = points_to
        .sets
        .iter()
        .map(|set| (set.variable, set))
        .collect::<BTreeMap<_, _>>();
    // Membership-only dedup by StableKeyId; emission order is assigned below via
    // `SolverOutput::normalized` (resolved-text sort), never allocation order.
    let mut edges = std::collections::HashMap::new();
    let mut incomplete_sites = 0_u64;

    for callsite in &inputs.callsites {
        let Some(set) = sets.get(&var_for_node(callsite.callsite_node)) else {
            continue;
        };
        // An incomplete set still derives its edges, with the set's status: every
        // object in it was derived, and the site is reported as incomplete.
        match set.status {
            PointsToStatus::Present => {}
            PointsToStatus::BudgetExceeded | PointsToStatus::IndirectionBound => {
                incomplete_sites += 1
            }
            PointsToStatus::Unknown
            | PointsToStatus::Unsupported
            | PointsToStatus::SetupMissing => {
                continue;
            }
        }
        let claim = site_claim(inputs, &points_to, callsite, set);
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
            // The calls whose bindings carried values to the callee name the
            // call and the signature place they bound.
            let bindings = points_to
                .call_bindings
                .get(&var_for_node(callsite.callsite_node))
                .into_iter()
                .flatten()
                .flat_map(|binding| {
                    [
                        ContributingFact {
                            stable_key: binding.call,
                        },
                        ContributingFact {
                            stable_key: binding.place,
                        },
                    ]
                });
            let provenance = DerivedEdgeProvenance::new(
                interner,
                [
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
                ]
                .into_iter()
                .chain(bindings),
                &ConstraintKind::call_at(callsite.callsite_node),
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
                claim: claim.clone(),
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
        incomplete_sites,
    }
}

/// What the edges derived at a call site claim.
///
/// A site is exact when the solve finished within its budget, the site's
/// callee set is complete, no value from outside the package can reach it,
/// its inputs were at most local flow-sensitive and it holds one function.
/// The claim rests on the solve seeing every write of every binding, on no
/// inward flow and on the budget, and, when the function was read from a
/// heap slot, on no other package writing that field or a prototype the read
/// went through. That the import links and the exported paths the site
/// reads stay as they are is assumed too; the solve does not record it.
///
/// The same site holding several functions is complete: it reaches some of
/// them and nothing else. A site values from outside can reach is open, with
/// the most specific way they reach it. Any other site claims nothing beyond
/// its precision.
fn site_claim(
    inputs: &TsPointsToInputs,
    points_to: &PointsToSolveResult,
    callsite: &TsPointsToCallsite,
    set: &PointsToSetFact,
) -> EdgeClaim {
    if points_to.budget_status != PointsToBudgetStatus::WithinBudget
        || set.status != PointsToStatus::Present
        || callsite.precision > PointsToPrecision::LocalFlowSensitive
    {
        return EdgeClaim::Unassigned;
    }
    let grounds = points_to
        .grounds
        .get(&set.variable)
        .copied()
        .unwrap_or_default();
    if let Some(inflow) = grounds.inflow {
        return EdgeClaim::Open(inflow);
    }
    let callable = set
        .objects
        .iter()
        .all(|object| inputs.callable_objects.contains_key(object));
    if set.precision > PointsToPrecision::LocalFlowSensitive || !callable {
        return EdgeClaim::Unassigned;
    }
    match set.objects.len() {
        0 => EdgeClaim::Unassigned,
        1 => {
            let mut assumptions = vec![
                Assumption::SealedBindings,
                Assumption::NoInwardFlow,
                Assumption::WithinBudget,
            ];
            if grounds.through_slot {
                assumptions.extend([Assumption::PrototypeClosed, Assumption::NoOutsideFieldWrite]);
            }
            assumptions.sort();
            EdgeClaim::Exact(assumptions)
        }
        _ => EdgeClaim::Complete,
    }
}

/// Writes the solver input `solve_ts_points_to` would solve, for the
/// stored-input solver benchmark.
#[cfg(test)]
pub(crate) fn write_solver_input(
    path: &std::path::Path,
    interner: &crate::internal_core::StableKeyInterner,
    inputs: &TsPointsToInputs,
) -> anyhow::Result<()> {
    crate::analysis_neutral::points_to::stored::write(
        path,
        interner,
        &project_constraints(interner, inputs),
        &project_calls(inputs),
    )
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
            PointsToPrecision::FlowInsensitive,
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
                    constraint.precision,
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
                    constraint.precision,
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
                    constraint.precision,
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
                    constraint.precision,
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
                    constraint.precision,
                    PointsToConstraintKind::FieldStore {
                        base: var_for_node(*base),
                        field: field.clone(),
                        src: var_for_node(*src),
                    },
                );
            }
            ConstraintKind::CallConstraint {
                callsite,
                callee: Some(callee),
                ..
            } if callsite != callee => {
                // The callsite's set is the callees: what the callee value holds.
                insert_projected_constraint(
                    interner,
                    &mut projected,
                    constraint.stable_key,
                    "call_callee",
                    constraint.precision,
                    PointsToConstraintKind::Copy {
                        dst: var_for_node(*callsite),
                        src: var_for_node(*callee),
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
        .map(
            |(index, (_, (stable_key, kind, precision)))| PointsToConstraintFact {
                id: PointsToConstraintId(index as u64),
                stable_key,
                kind,
                status: PointsToStatus::Present,
                precision,
            },
        )
        .collect()
}

/// The calls that pass values in or read a result, for the solver's call rule.
/// A call that only names its callee adds nothing the constraints do not.
fn project_calls(inputs: &TsPointsToInputs) -> PointsToCallInputs {
    let calls = inputs
        .constraints
        .iter()
        .filter(|constraint| constraint.status == PointsToStatus::Present)
        .filter_map(|constraint| match &constraint.kind {
            ConstraintKind::CallConstraint {
                callsite,
                receiver,
                arguments,
                spread_from,
                result,
                ..
            } if receiver.is_some() || !arguments.is_empty() || result.is_some() => {
                // The callsite's set is the site's callees: the callee place
                // copies into it, and so do the walker and the object model.
                Some(PointsToCall {
                    stable_key: constraint.stable_key,
                    callee: var_for_node(*callsite),
                    receiver: receiver.map(var_for_node),
                    arguments: arguments.iter().copied().map(var_for_node).collect(),
                    spread_from: *spread_from,
                    result: result.map(var_for_node),
                    precision: constraint.precision,
                })
            }
            _ => None,
        })
        .collect();
    PointsToCallInputs {
        calls,
        signatures: inputs.signatures.clone(),
    }
}

fn insert_projected_constraint(
    interner: &crate::internal_core::StableKeyInterner,
    projected: &mut BTreeMap<String, (StableKeyId, PointsToConstraintKind, PointsToPrecision)>,
    source_stable_key: StableKeyId,
    relation: &str,
    precision: PointsToPrecision,
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
    // A key seen twice keeps the weaker precision, so a heuristic source is
    // never projected as exact.
    projected
        .entry(interner.resolve(stable_key).to_string())
        .and_modify(|(_, _, held)| *held = (*held).max(precision))
        .or_insert((stable_key, kind, precision));
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
mod solver_composition;

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
            ("call:unrelated", ConstraintKind::call_at(unrelated)),
            ("call:invoked", ConstraintKind::call_at(invoked)),
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
            signatures: BTreeMap::new(),
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
