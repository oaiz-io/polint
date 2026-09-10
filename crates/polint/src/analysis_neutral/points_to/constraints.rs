use super::facts::{
    PointsToConstraintFact, PointsToConstraintKind, PointsToPrecision, PointsToStatus,
};
use crate::analysis_api::{FactFamily, stable_key_from_parts, stable_key_text_from_parts};
use crate::analysis_neutral::access_paths::facts::{AccessPathFact, AccessPathProjection};
use crate::analysis_neutral::ids::{PointsToConstraintId, PtVarId};
use crate::analysis_neutral::points_to::vars;
use crate::analysis_neutral::types::store::TypeValueAliasOutput;
use crate::analysis_neutral::values::facts::{
    AllocationTokenFact, ValueFact, ValueKind, ValueStatus, ValueSubject,
};

pub fn derive_points_to_constraints(
    interner: &crate::internal_core::StableKeyInterner,
    output: &TypeValueAliasOutput,
) -> Vec<PointsToConstraintFact> {
    let mut builder = ConstraintBuilder::default();
    builder.collect_values(interner, &output.values.values);
    builder.collect_allocations(interner, &output.values.allocations);
    builder.collect_access_paths(interner, &output.access_paths.access_paths);
    builder.finish(interner)
}

#[derive(Default)]
struct ConstraintBuilder {
    constraints: Vec<PointsToConstraintFact>,
}

impl ConstraintBuilder {
    fn collect_values(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        values: &[ValueFact],
    ) {
        for value in values {
            let Some(dst) = subject_var(&value.subject) else {
                continue;
            };
            let source_identity = interner.resolve(value.stable_key);
            match &value.kind {
                ValueKind::Object(object)
                | ValueKind::Array(object)
                | ValueKind::CompositeLiteral(object) => {
                    self.push(
                        interner,
                        PointsToConstraintKind::AddressOf {
                            dst,
                            object: vars::allocation_object(*object),
                        },
                        &source_identity,
                        "value_object",
                    );
                }
                ValueKind::PlaceRef(src) => {
                    self.push(
                        interner,
                        PointsToConstraintKind::Copy {
                            dst,
                            src: vars::place_var(*src),
                        },
                        &source_identity,
                        "place_copy",
                    );
                }
                ValueKind::CallReturn(place) if value.status == ValueStatus::Present => {
                    self.push(
                        interner,
                        PointsToConstraintKind::CallReturn {
                            dst,
                            value: value.id,
                        },
                        &source_identity,
                        "call_return_value",
                    );
                    self.push(
                        interner,
                        PointsToConstraintKind::Copy {
                            dst,
                            src: vars::place_var(*place),
                        },
                        &source_identity,
                        "call_return_place",
                    );
                }
                ValueKind::CallReturn(_) => {}
                ValueKind::FunctionObject | ValueKind::ClassObject | ValueKind::ModuleObject => {
                    self.push(
                        interner,
                        PointsToConstraintKind::AddressOf {
                            dst,
                            object: vars::abstract_value_object(value.value),
                        },
                        &source_identity,
                        "abstract_value_object",
                    );
                }
                ValueKind::Unknown { .. }
                | ValueKind::Null
                | ValueKind::Undefined
                | ValueKind::Nil
                | ValueKind::Bool(_)
                | ValueKind::Number(_)
                | ValueKind::String(_)
                | ValueKind::Literal(_) => {}
            }
        }
    }

    fn collect_allocations(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        allocations: &[AllocationTokenFact],
    ) {
        for allocation in allocations {
            if let Some(place) = allocation.source_place {
                let source_identity = interner.resolve(allocation.stable_key);
                self.push(
                    interner,
                    PointsToConstraintKind::AddressOf {
                        dst: vars::place_var(place),
                        object: vars::allocation_object(allocation.id),
                    },
                    &source_identity,
                    "allocation_source_place",
                );
            }
        }
    }

    fn collect_access_paths(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        paths: &[AccessPathFact],
    ) {
        for path in paths {
            let path_identity = interner.resolve(path.stable_key);
            let mut projection_prefix = Vec::new();
            let mut base = vars::place_var(path.base);
            for (index, projection) in path.projections.iter().enumerate() {
                projection_prefix.push(projection_identity(projection));
                let source_identity =
                    projection_source_identity(&path_identity, &projection_prefix);
                let dst = if index + 1 == path.projections.len() {
                    access_path_var(path)
                } else {
                    vars::access_path_prefix_var(path.id, index)
                };
                match projection {
                    AccessPathProjection::Field(field) | AccessPathProjection::Property(field) => {
                        self.push(
                            interner,
                            PointsToConstraintKind::FieldLoad {
                                dst,
                                base,
                                field: field.clone(),
                            },
                            &source_identity,
                            "field_load",
                        );
                        self.push(
                            interner,
                            PointsToConstraintKind::FieldStore {
                                base,
                                field: field.clone(),
                                src: dst,
                            },
                            &source_identity,
                            "field_store",
                        );
                    }
                    AccessPathProjection::IndexKnown(index) => {
                        self.push(
                            interner,
                            PointsToConstraintKind::ElementLoad {
                                dst,
                                base,
                                index: index.clone(),
                            },
                            &source_identity,
                            "element_load",
                        );
                        self.push(
                            interner,
                            PointsToConstraintKind::ElementStore {
                                base,
                                index: index.clone(),
                                src: dst,
                            },
                            &source_identity,
                            "element_store",
                        );
                    }
                    AccessPathProjection::IndexUnknown { evidence } => {
                        self.push(
                            interner,
                            PointsToConstraintKind::ElementLoad {
                                dst,
                                base,
                                index: format!("unknown:{evidence}"),
                            },
                            &source_identity,
                            "unknown_element_load",
                        );
                        self.push(
                            interner,
                            PointsToConstraintKind::ElementStore {
                                base,
                                index: format!("unknown:{evidence}"),
                                src: dst,
                            },
                            &source_identity,
                            "unknown_element_store",
                        );
                    }
                    AccessPathProjection::Deref => {
                        self.push(
                            interner,
                            PointsToConstraintKind::Load { dst, pointer: base },
                            &source_identity,
                            "load",
                        );
                        self.push(
                            interner,
                            PointsToConstraintKind::Store {
                                pointer: base,
                                src: dst,
                            },
                            &source_identity,
                            "store",
                        );
                    }
                    AccessPathProjection::CallReturn(_) => {
                        self.push(
                            interner,
                            PointsToConstraintKind::SummaryFlow {
                                dst,
                                src: base,
                                summary_key: source_identity.clone(),
                            },
                            &source_identity,
                            "call_return",
                        );
                    }
                    AccessPathProjection::AwaitResult | AccessPathProjection::Unknown { .. } => {}
                }
                base = dst;
            }
        }
    }

    fn push(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        kind: PointsToConstraintKind,
        source_identity: &str,
        relation: &str,
    ) {
        let stable_key = constraint_stable_key(interner, source_identity, relation);
        self.constraints.push(PointsToConstraintFact {
            id: PointsToConstraintId(self.constraints.len() as u64),
            kind,
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            stable_key,
        });
    }

    fn finish(
        mut self,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> Vec<PointsToConstraintFact> {
        self.constraints
            .sort_by(|left, right| interner.compare_canonical(left.stable_key, right.stable_key));
        self.constraints
            .dedup_by(|left, right| left.stable_key == right.stable_key);
        for (index, constraint) in self.constraints.iter_mut().enumerate() {
            constraint.id = PointsToConstraintId(index as u64);
        }
        self.constraints
    }
}

fn subject_var(subject: &ValueSubject) -> Option<PtVarId> {
    match subject {
        ValueSubject::Place(place) => Some(vars::place_var(*place)),
        ValueSubject::Operation(operation) => Some(vars::operation_var(*operation)),
        ValueSubject::Allocation(allocation) => Some(vars::allocation_var(*allocation)),
        ValueSubject::Synthetic(_) | ValueSubject::Unknown(_) => None,
    }
}

fn access_path_var(path: &AccessPathFact) -> PtVarId {
    vars::access_path_var(path.id)
}

fn projection_source_identity(path_identity: &str, projection_prefix: &[String]) -> String {
    let encoded_prefix = projection_prefix
        .iter()
        .map(|projection| format!("{}:{projection}", projection.len()))
        .collect::<String>();
    stable_key_text_from_parts(
        FactFamily::AccessPath,
        &[
            ("path", path_identity.to_string()),
            ("projection_prefix", encoded_prefix),
        ],
    )
}

fn projection_identity(projection: &AccessPathProjection) -> String {
    match projection {
        AccessPathProjection::Field(field) => format!("field:{field}"),
        AccessPathProjection::Property(property) => format!("property:{property}"),
        AccessPathProjection::IndexKnown(index) => format!("index_known:{index}"),
        AccessPathProjection::IndexUnknown { evidence } => format!("index_unknown:{evidence}"),
        AccessPathProjection::Deref => "deref".to_string(),
        AccessPathProjection::AwaitResult => "await_result".to_string(),
        AccessPathProjection::CallReturn(_) => "call_return".to_string(),
        AccessPathProjection::Unknown { evidence } => format!("unknown:{evidence}"),
    }
}

fn constraint_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    source_identity: &str,
    relation: &str,
) -> crate::internal_core::StableKeyId {
    stable_key_from_parts(
        interner,
        FactFamily::PointsToConstraint,
        &[
            ("source", source_identity.to_string()),
            ("relation", relation.to_string()),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::access_paths::facts::AccessPathStatus;
    use crate::analysis_neutral::access_paths::store::AccessPathOutput;
    use crate::analysis_neutral::ids::{AccessPathId, AllocationTokenId, MirBodyId, ValueFactId};
    use crate::analysis_neutral::types::store::TypeValueAliasOutput;
    use crate::analysis_neutral::values::facts::{ValuePrecision, ValueProvenance, ValueStatus};
    use crate::analysis_neutral::values::store::ValueOutput;
    use crate::internal_core::Language;

    #[test]
    fn derives_core_constraints_from_values_and_access_paths() {
        let output = TypeValueAliasOutput {
            values: ValueOutput {
                values: vec![
                    value(
                        ValueFactId(0),
                        ValueSubject::Place(crate::analysis_neutral::ids::PlaceId(1)),
                        ValueKind::Object(AllocationTokenId(7)),
                    ),
                    value(
                        ValueFactId(1),
                        ValueSubject::Place(crate::analysis_neutral::ids::PlaceId(2)),
                        ValueKind::PlaceRef(crate::analysis_neutral::ids::PlaceId(1)),
                    ),
                    value(
                        ValueFactId(2),
                        ValueSubject::Place(crate::analysis_neutral::ids::PlaceId(3)),
                        ValueKind::CallReturn(crate::analysis_neutral::ids::PlaceId(2)),
                    ),
                ],
                allocations: Vec::new(),
            },
            access_paths: AccessPathOutput {
                access_paths: vec![
                    path(
                        AccessPathId(0),
                        AccessPathProjection::Property("name".to_string()),
                    ),
                    path(
                        AccessPathId(1),
                        AccessPathProjection::IndexUnknown {
                            evidence: "key".to_string(),
                        },
                    ),
                    path(AccessPathId(2), AccessPathProjection::Deref),
                    path(
                        AccessPathId(3),
                        AccessPathProjection::CallReturn(crate::analysis_neutral::ids::CallSiteId(
                            9,
                        )),
                    ),
                ],
            },
            ..TypeValueAliasOutput::default()
        };

        let constraints = derive_points_to_constraints(
            &crate::internal_core::test_stable_key_interner(),
            &output,
        );
        assert!(
            constraints
                .iter()
                .any(|row| matches!(row.kind, PointsToConstraintKind::AddressOf { .. }))
        );
        assert!(
            constraints
                .iter()
                .any(|row| matches!(row.kind, PointsToConstraintKind::Copy { .. }))
        );
        assert!(
            constraints
                .iter()
                .any(|row| matches!(row.kind, PointsToConstraintKind::FieldLoad { .. }))
        );
        assert!(
            constraints
                .iter()
                .any(|row| matches!(row.kind, PointsToConstraintKind::FieldStore { .. }))
        );
        assert!(
            constraints
                .iter()
                .any(|row| matches!(row.kind, PointsToConstraintKind::ElementLoad { .. }))
        );
        assert!(
            constraints
                .iter()
                .any(|row| matches!(row.kind, PointsToConstraintKind::ElementStore { .. }))
        );
        assert!(
            constraints
                .iter()
                .any(|row| matches!(row.kind, PointsToConstraintKind::Load { .. }))
        );
        assert!(
            constraints
                .iter()
                .any(|row| matches!(row.kind, PointsToConstraintKind::Store { .. }))
        );
        assert!(
            constraints
                .iter()
                .any(|row| matches!(row.kind, PointsToConstraintKind::CallReturn { .. }))
        );
        assert!(
            constraints
                .iter()
                .any(|row| matches!(row.kind, PointsToConstraintKind::SummaryFlow { .. }))
        );
    }

    #[test]
    fn unknown_call_returns_do_not_create_fresh_points_to_objects() {
        let output = TypeValueAliasOutput {
            values: ValueOutput {
                values: vec![value_with_status(
                    ValueFactId(0),
                    ValueSubject::Place(crate::analysis_neutral::ids::PlaceId(3)),
                    ValueKind::CallReturn(crate::analysis_neutral::ids::PlaceId(3)),
                    ValueStatus::Unknown,
                )],
                allocations: Vec::new(),
            },
            ..TypeValueAliasOutput::default()
        };

        let constraints = derive_points_to_constraints(
            &crate::internal_core::test_stable_key_interner(),
            &output,
        );

        assert!(
            !constraints
                .iter()
                .any(|row| matches!(row.kind, PointsToConstraintKind::CallReturn { .. }))
        );
    }

    fn value(id: ValueFactId, subject: ValueSubject, kind: ValueKind) -> ValueFact {
        value_with_status(id, subject, kind, ValueStatus::Present)
    }

    fn value_with_status(
        id: ValueFactId,
        subject: ValueSubject,
        kind: ValueKind,
        status: ValueStatus,
    ) -> ValueFact {
        ValueFact {
            id,
            subject,
            value: crate::analysis_neutral::ids::AbstractValueId(id.0),
            kind,
            language: Language::TypeScript,
            file: None,
            function: None,
            body: Some(MirBodyId(0)),
            precision: ValuePrecision::Heuristic,
            status,
            provenance: ValueProvenance::Native,
            stable_key: crate::internal_core::stable_key_for_test(&format!("value:{}", id.0)),
        }
    }

    fn path(id: AccessPathId, projection: AccessPathProjection) -> AccessPathFact {
        AccessPathFact {
            id,
            base: crate::analysis_neutral::ids::PlaceId(1),
            projections: vec![projection],
            depth: 1,
            language: Language::TypeScript,
            file: None,
            function: None,
            body: Some(MirBodyId(0)),
            status: AccessPathStatus::Partial,
            stable_key: crate::internal_core::stable_key_for_test(&format!("path:{}", id.0)),
        }
    }
}
