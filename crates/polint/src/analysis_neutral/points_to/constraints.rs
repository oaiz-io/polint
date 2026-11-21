use super::facts::{
    PointsToConstraintFact, PointsToConstraintKind, PointsToPrecision, PointsToStatus,
};
use crate::analysis_api::{FactFamily, stable_key_from_parts, stable_key_text_from_parts};
use crate::analysis_neutral::access_paths::facts::{AccessPathFact, AccessPathProjection};
use crate::analysis_neutral::ids::{PlaceId, PointsToConstraintId, PtVarId};
use crate::analysis_neutral::points_to::vars;
use crate::analysis_neutral::types::store::TypeValueAliasOutput;
use crate::analysis_neutral::values::facts::{
    AllocationTokenFact, ValueFact, ValueKind, ValueStatus, ValueSubject,
};
use std::collections::HashMap;

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

    /// Every access path that starts at one place and takes the same steps names
    /// one link, so a member chain whose every prefix is a path derives one
    /// constraint pair per link rather than one per prefix of every path.
    fn collect_access_paths(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        paths: &[AccessPathFact],
    ) {
        let links = AccessPathLinks::new(paths);
        for link in &links.links {
            let creator = &paths[link.creator];
            let (dst, source_identity) = match link.owner {
                Some(owner) => (
                    access_path_var(&paths[owner]),
                    interner.resolve(paths[owner].stable_key).to_string(),
                ),
                None => (
                    vars::access_path_prefix_var(creator.id, link.step),
                    link_source_identity(&interner.resolve(creator.stable_key), link.step),
                ),
            };
            let base = match link.parent {
                LinkParent::Base(place) => vars::place_var(place),
                LinkParent::Link(parent) => links.var(paths, parent),
            };
            self.push_projection(
                interner,
                dst,
                base,
                &creator.projections[link.step],
                &source_identity,
            );
        }
        for (index, path) in paths.iter().enumerate() {
            let Some(end) = links.path_end[index] else {
                continue;
            };
            let Some(owner) = links.links[end].owner.filter(|owner| *owner != index) else {
                continue;
            };
            let path_identity = interner.resolve(path.stable_key).to_string();
            let path_var = access_path_var(path);
            let owner_var = access_path_var(&paths[owner]);
            self.push(
                interner,
                PointsToConstraintKind::Copy {
                    dst: path_var,
                    src: owner_var,
                },
                &path_identity,
                "same_place_copy",
            );
            self.push(
                interner,
                PointsToConstraintKind::Copy {
                    dst: owner_var,
                    src: path_var,
                },
                &path_identity,
                "same_place_copy_back",
            );
        }
    }

    fn push_projection(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        dst: PtVarId,
        base: PtVarId,
        projection: &AccessPathProjection,
        source_identity: &str,
    ) {
        match projection {
            AccessPathProjection::Field(field) | AccessPathProjection::Property(field) => {
                self.push(
                    interner,
                    PointsToConstraintKind::FieldLoad {
                        dst,
                        base,
                        field: field.clone(),
                    },
                    source_identity,
                    "field_load",
                );
                self.push(
                    interner,
                    PointsToConstraintKind::FieldStore {
                        base,
                        field: field.clone(),
                        src: dst,
                    },
                    source_identity,
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
                    source_identity,
                    "element_load",
                );
                self.push(
                    interner,
                    PointsToConstraintKind::ElementStore {
                        base,
                        index: index.clone(),
                        src: dst,
                    },
                    source_identity,
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
                    source_identity,
                    "unknown_element_load",
                );
                self.push(
                    interner,
                    PointsToConstraintKind::ElementStore {
                        base,
                        index: format!("unknown:{evidence}"),
                        src: dst,
                    },
                    source_identity,
                    "unknown_element_store",
                );
            }
            AccessPathProjection::Deref => {
                self.push(
                    interner,
                    PointsToConstraintKind::Load { dst, pointer: base },
                    source_identity,
                    "load",
                );
                self.push(
                    interner,
                    PointsToConstraintKind::Store {
                        pointer: base,
                        src: dst,
                    },
                    source_identity,
                    "store",
                );
            }
            AccessPathProjection::CallReturn(_) => {
                self.push(
                    interner,
                    PointsToConstraintKind::SummaryFlow {
                        dst,
                        src: base,
                        summary_key: source_identity.to_string(),
                    },
                    source_identity,
                    "call_return",
                );
            }
            AccessPathProjection::AwaitResult | AccessPathProjection::Unknown { .. } => {}
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
        interner.sort_by_key_text(&mut self.constraints, |row| row.stable_key, |_| ());
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

fn link_source_identity(creator_identity: &str, step: usize) -> String {
    stable_key_text_from_parts(
        FactFamily::AccessPath,
        &[
            ("path", creator_identity.to_string()),
            ("link", step.to_string()),
        ],
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LinkParent {
    Base(PlaceId),
    Link(usize),
}

/// One step from a place or from a shorter link, shared by every access path
/// that takes it.
struct AccessPathLink {
    parent: LinkParent,
    /// The first path, in input order, that takes this step.
    creator: usize,
    /// Which of the creator's projections this step is.
    step: usize,
    /// The first path, in input order, that ends at this step.
    owner: Option<usize>,
}

struct AccessPathLinks {
    links: Vec<AccessPathLink>,
    /// The link each path ends at; `None` for a path with no projections.
    path_end: Vec<Option<usize>>,
}

impl AccessPathLinks {
    fn new(paths: &[AccessPathFact]) -> Self {
        let mut links = Vec::<AccessPathLink>::new();
        let mut path_end = Vec::with_capacity(paths.len());
        let mut index_by_step = HashMap::<(LinkParent, String), usize>::new();
        for (path_index, path) in paths.iter().enumerate() {
            let mut parent = LinkParent::Base(path.base);
            for (step, projection) in path.projections.iter().enumerate() {
                let link = *index_by_step
                    .entry((parent, projection_identity(projection)))
                    .or_insert_with(|| {
                        links.push(AccessPathLink {
                            parent,
                            creator: path_index,
                            step,
                            owner: None,
                        });
                        links.len() - 1
                    });
                parent = LinkParent::Link(link);
            }
            let end = match parent {
                LinkParent::Base(_) => None,
                LinkParent::Link(link) => Some(link),
            };
            if let Some(end) = end {
                links[end].owner.get_or_insert(path_index);
            }
            path_end.push(end);
        }
        Self { links, path_end }
    }

    fn var(&self, paths: &[AccessPathFact], link: usize) -> PtVarId {
        let link = &self.links[link];
        match link.owner {
            Some(owner) => access_path_var(&paths[owner]),
            None => vars::access_path_prefix_var(paths[link.creator].id, link.step),
        }
    }
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

    #[test]
    fn a_member_chain_with_a_path_for_every_prefix_derives_constraints_linear_in_its_depth() {
        // `a.f0.f1...`: the extractor names every prefix of the chain as its
        // own access path, so the paths together carry depth²/2 projections.
        let chain = |depth: usize| -> Vec<AccessPathFact> {
            (1..=depth)
                .map(|length| {
                    let mut path = path(
                        AccessPathId(length as u64),
                        AccessPathProjection::Unknown {
                            evidence: String::new(),
                        },
                    );
                    path.projections = (0..length)
                        .map(|link| AccessPathProjection::Property(format!("f{link}")))
                        .collect();
                    path.depth = length as u32;
                    path
                })
                .collect()
        };
        let constraints_for = |depth: usize| {
            let output = TypeValueAliasOutput {
                access_paths: AccessPathOutput {
                    access_paths: chain(depth),
                },
                ..TypeValueAliasOutput::default()
            };
            derive_points_to_constraints(&crate::internal_core::test_stable_key_interner(), &output)
        };

        for depth in [1, 10, 200] {
            let constraints = constraints_for(depth);
            assert_eq!(
                constraints.len(),
                2 * depth,
                "one load and one store per link of a {depth}-link chain"
            );
            for link in 1..=depth {
                let dst = vars::access_path_var(AccessPathId(link as u64));
                let base = if link == 1 {
                    vars::place_var(crate::analysis_neutral::ids::PlaceId(1))
                } else {
                    vars::access_path_var(AccessPathId(link as u64 - 1))
                };
                let field = format!("f{}", link - 1);
                assert!(constraints.iter().any(|row| row.kind
                    == PointsToConstraintKind::FieldLoad {
                        dst,
                        base,
                        field: field.clone(),
                    }));
                assert!(constraints.iter().any(|row| row.kind
                    == PointsToConstraintKind::FieldStore {
                        base,
                        field: field.clone(),
                        src: dst,
                    }));
            }
        }
    }

    #[test]
    fn two_paths_that_name_the_same_place_share_its_contents() {
        let mut first = path(
            AccessPathId(4),
            AccessPathProjection::Property("x".to_string()),
        );
        first.stable_key = crate::internal_core::stable_key_for_test("path:first");
        let mut second = first.clone();
        second.id = AccessPathId(9);
        second.stable_key = crate::internal_core::stable_key_for_test("path:second");
        let output = TypeValueAliasOutput {
            access_paths: AccessPathOutput {
                access_paths: vec![first, second],
            },
            ..TypeValueAliasOutput::default()
        };

        let constraints = derive_points_to_constraints(
            &crate::internal_core::test_stable_key_interner(),
            &output,
        );

        let first_var = vars::access_path_var(AccessPathId(4));
        let second_var = vars::access_path_var(AccessPathId(9));
        for (dst, src) in [(first_var, second_var), (second_var, first_var)] {
            assert!(
                constraints
                    .iter()
                    .any(|row| row.kind == PointsToConstraintKind::Copy { dst, src }),
                "{dst:?} holds what {src:?} holds"
            );
        }
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
