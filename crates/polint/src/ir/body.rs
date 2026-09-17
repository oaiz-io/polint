use serde::{Deserialize, Serialize};

use crate::internal_core::{
    FileId, FunctionId, Language, ModuleNodeId, PackageId, Span, StableKeyId, StableKeyInterner,
};
use crate::ir::ids::{
    CallSiteId, MirBodyId, MirOpId, MirPredicateId, MirStatementId, MirTerminatorId, PlaceId,
    UnsupportedId,
};
use crate::ir::op::{MirOperation, MirValue, UnsupportedSemanticFact};
use crate::ir::places::{PlaceFact, PlaceTypeFact};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MirBlockId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirBody {
    pub id: MirBodyId,
    pub language: Language,
    pub file: FileId,
    pub function: FunctionId,
    pub package: Option<PackageId>,
    pub module: Option<ModuleNodeId>,
    pub owner_stable_key: StableKeyId,
    pub span: Span,
    pub stable_key: StableKeyId,
    pub status: MirStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirStatement {
    pub id: MirStatementId,
    pub body: MirBodyId,
    pub ordinal: u32,
    pub operation: MirOpId,
    pub stable_key: StableKeyId,
    pub status: MirStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirTerminator {
    pub id: MirTerminatorId,
    pub body: MirBodyId,
    pub ordinal: u32,
    pub kind: MirTerminatorKind,
    pub stable_key: StableKeyId,
    pub status: MirStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MirTerminatorKind {
    Goto {
        target: MirBlockId,
    },
    Branch {
        predicate: MirPredicateId,
        predicate_place: Option<crate::ir::ids::PlaceId>,
        then_target: MirBlockId,
        else_target: MirBlockId,
    },
    Switch {
        discriminant: MirValue,
        cases: Vec<(MirValue, MirBlockId)>,
        otherwise: MirBlockId,
    },
    Return {
        value: Option<MirValue>,
    },
    Throw {
        value: Option<MirValue>,
        unwind: MirBlockId,
    },
    Call {
        site: CallSiteId,
        callee: MirValue,
        arguments: Vec<PlaceId>,
        return_place: PlaceId,
        normal: MirBlockId,
        unwind: Option<MirBlockId>,
    },
    Suspend {
        kind: SuspendKind,
        value: Option<MirValue>,
        resume: MirBlockId,
    },
    Unreachable,
    Unsupported {
        unsupported: UnsupportedId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuspendKind {
    Await,
    Yield,
    ChannelRecv,
    ChannelSend,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirBlock {
    pub id: MirBlockId,
    pub body: MirBodyId,
    pub ordinal: u32,
    pub statements: Vec<MirStatementId>,
    pub terminator: MirTerminatorId,
    pub stable_key: StableKeyId,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirOutput {
    pub bodies: Vec<MirBody>,
    pub blocks: Vec<MirBlock>,
    pub statements: Vec<MirStatement>,
    pub terminators: Vec<MirTerminator>,
    pub places: Vec<PlaceFact>,
    pub place_types: Vec<PlaceTypeFact>,
    pub operations: Vec<MirOperation>,
    pub unsupported: Vec<UnsupportedSemanticFact>,
}

impl MirOutput {
    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.bodies
            .sort_by(|body, other| interner.compare_canonical(body.stable_key, other.stable_key));
        self.blocks.sort_by(|block, other| {
            block
                .body
                .cmp(&other.body)
                .then_with(|| block.ordinal.cmp(&other.ordinal))
                .then_with(|| interner.compare_canonical(block.stable_key, other.stable_key))
        });
        self.statements.sort_by(|statement, other| {
            statement
                .body
                .cmp(&other.body)
                .then_with(|| statement.ordinal.cmp(&other.ordinal))
                .then_with(|| interner.compare_canonical(statement.stable_key, other.stable_key))
        });
        self.terminators.sort_by(|terminator, other| {
            terminator
                .body
                .cmp(&other.body)
                .then_with(|| terminator.ordinal.cmp(&other.ordinal))
                .then_with(|| interner.compare_canonical(terminator.stable_key, other.stable_key))
        });
        self.places
            .sort_by(|place, other| interner.compare_canonical(place.stable_key, other.stable_key));
        self.place_types.sort_by_key(|fact| fact.place);
        self.operations.sort_by(|operation, other| {
            operation
                .body
                .cmp(&other.body)
                .then_with(|| operation.ordinal.cmp(&other.ordinal))
                .then_with(|| interner.compare_canonical(operation.stable_key, other.stable_key))
        });
        self.unsupported
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MirStatus {
    Resolved,
    Partial,
    Unknown,
    Unsupported,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal_core::{FileId, FunctionId, Language, ModuleNodeId, PackageId, Span};
    use crate::ir::ids::{CallSiteId, MirBodyId, MirOpId, PlaceId, UnsupportedId};
    use crate::ir::op::{
        AssignMode, ConservativeAction, MirOperation, MirOperationKind, MirValue,
        UnsupportedDomain, UnsupportedPrecision, UnsupportedSemanticFact,
    };
    use crate::ir::places::{PlaceFact, PlaceRoot, PlaceStatus};

    fn span() -> Span {
        Span::new(FileId::from_raw(1), 1, 2, 1, 1, 1, 2)
    }

    fn body(interner: &StableKeyInterner, id: u64, stable_key: &str) -> MirBody {
        MirBody {
            id: MirBodyId(id),
            language: Language::Go,
            file: FileId::from_raw(1),
            function: FunctionId::from_raw(id),
            package: Some(PackageId::from_raw(1)),
            module: Some(ModuleNodeId::from_raw(1)),
            owner_stable_key: interner.intern(format!("owner:{id}")),
            span: span(),
            stable_key: interner.intern(stable_key.to_string()),
            status: MirStatus::Resolved,
        }
    }

    fn place(interner: &StableKeyInterner, id: u64, stable_key: &str) -> PlaceFact {
        PlaceFact {
            id: PlaceId(id),
            language: Language::Go,
            file: Some(FileId::from_raw(1)),
            function: Some(FunctionId::from_raw(1)),
            root: PlaceRoot::Local {
                function: FunctionId::from_raw(1),
                name: stable_key.to_string(),
            },
            projections: Vec::new(),
            stable_key: interner.intern(stable_key.to_string()),
            status: PlaceStatus::Resolved,
        }
    }

    fn operation(
        interner: &StableKeyInterner,
        body: u64,
        ordinal: u32,
        stable_key: &str,
    ) -> MirOperation {
        MirOperation {
            id: MirOpId(u64::from(ordinal)),
            body: MirBodyId(body),
            ordinal,
            span: span(),
            kind: MirOperationKind::Assign {
                place: PlaceId(1),
                value: MirValue::Place(PlaceId(2)),
                mode: AssignMode::Overwrite,
            },
            stable_key: interner.intern(stable_key.to_string()),
            status: MirStatus::Resolved,
        }
    }

    fn unsupported(interner: &StableKeyInterner, stable_key: &str) -> UnsupportedSemanticFact {
        UnsupportedSemanticFact {
            id: UnsupportedId(1),
            body: Some(MirBodyId(1)),
            operation: Some(MirOpId(1)),
            language: Language::Go,
            file: FileId::from_raw(1),
            span: span(),
            construct: "go-reflect".to_string(),
            source_evidence: "reflect.ValueOf(x)".to_string(),
            affected_places: vec![PlaceId(1)],
            affected_domains: vec![UnsupportedDomain::Mir, UnsupportedDomain::DataFlow],
            conservative_action: ConservativeAction::HavocAffectedPlaces,
            precision: UnsupportedPrecision::Unsupported,
            status: MirStatus::Unsupported,
            stable_key: interner.intern(stable_key.to_string()),
        }
    }

    #[test]
    fn mir_output_normalized_sorts_all_fact_rows_deterministically() {
        let interner = StableKeyInterner::default();
        let output = MirOutput {
            bodies: vec![body(&interner, 2, "body:z"), body(&interner, 1, "body:a")],
            blocks: vec![
                MirBlock {
                    id: MirBlockId(2),
                    body: MirBodyId(2),
                    ordinal: 2,
                    statements: vec![MirStatementId(2)],
                    terminator: MirTerminatorId(2),
                    stable_key: interner.intern("block:z".to_string()),
                },
                MirBlock {
                    id: MirBlockId(1),
                    body: MirBodyId(1),
                    ordinal: 1,
                    statements: vec![MirStatementId(1)],
                    terminator: MirTerminatorId(1),
                    stable_key: interner.intern("block:a".to_string()),
                },
            ],
            statements: vec![
                MirStatement {
                    id: MirStatementId(2),
                    body: MirBodyId(2),
                    ordinal: 2,
                    operation: MirOpId(2),
                    stable_key: interner.intern("statement:z".to_string()),
                    status: MirStatus::Resolved,
                },
                MirStatement {
                    id: MirStatementId(1),
                    body: MirBodyId(1),
                    ordinal: 1,
                    operation: MirOpId(1),
                    stable_key: interner.intern("statement:a".to_string()),
                    status: MirStatus::Resolved,
                },
            ],
            terminators: vec![
                MirTerminator {
                    id: MirTerminatorId(2),
                    body: MirBodyId(2),
                    ordinal: 2,
                    kind: MirTerminatorKind::Return { value: None },
                    stable_key: interner.intern("terminator:z".to_string()),
                    status: MirStatus::Resolved,
                },
                MirTerminator {
                    id: MirTerminatorId(1),
                    body: MirBodyId(1),
                    ordinal: 1,
                    kind: MirTerminatorKind::Return { value: None },
                    stable_key: interner.intern("terminator:a".to_string()),
                    status: MirStatus::Resolved,
                },
            ],
            places: vec![
                place(&interner, 2, "place:z"),
                place(&interner, 1, "place:a"),
            ],
            place_types: Vec::new(),
            operations: vec![
                operation(&interner, 2, 2, "op:z"),
                operation(&interner, 1, 2, "op:b"),
                operation(&interner, 1, 1, "op:z"),
                operation(&interner, 1, 1, "op:a"),
            ],
            unsupported: vec![
                unsupported(&interner, "unsupported:z"),
                unsupported(&interner, "unsupported:a"),
            ],
        }
        .normalized(&interner);

        assert_eq!(
            output
                .bodies
                .iter()
                .map(|body| interner.resolve(body.stable_key).to_string())
                .collect::<Vec<_>>(),
            vec!["body:a", "body:z"]
        );
        assert_eq!(
            output
                .places
                .iter()
                .map(|place| interner.resolve(place.stable_key).to_string())
                .collect::<Vec<_>>(),
            vec!["place:a", "place:z"]
        );
        assert_eq!(
            interner.resolve(output.blocks[0].stable_key).as_ref(),
            "block:a"
        );
        assert_eq!(
            interner.resolve(output.statements[0].stable_key).as_ref(),
            "statement:a"
        );
        assert_eq!(
            interner.resolve(output.terminators[0].stable_key).as_ref(),
            "terminator:a"
        );
        assert_eq!(
            output
                .operations
                .iter()
                .map(|op| {
                    (
                        op.body,
                        op.ordinal,
                        interner.resolve(op.stable_key).to_string(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (MirBodyId(1), 1, "op:a".to_string()),
                (MirBodyId(1), 1, "op:z".to_string()),
                (MirBodyId(1), 2, "op:b".to_string()),
                (MirBodyId(2), 2, "op:z".to_string()),
            ]
        );
        assert_eq!(
            output
                .unsupported
                .iter()
                .map(|row| interner.resolve(row.stable_key).to_string())
                .collect::<Vec<_>>(),
            vec!["unsupported:a", "unsupported:z"]
        );
    }

    #[test]
    fn mir_contract_source_does_not_store_parser_ast_objects() {
        let source = [
            include_str!("body.rs"),
            include_str!("op.rs"),
            include_str!("places.rs"),
        ]
        .join("\n");

        let forbidden = [
            concat!("tree_sitter", "::Node"),
            concat!("oxc", "_ast"),
            concat!("oxc", "_span::Span"),
            concat!("Node", "<'_"),
            concat!("Program", "<'_"),
        ];

        for forbidden in forbidden {
            assert!(
                !source.contains(forbidden),
                "forbidden parser type leaked: {forbidden}"
            );
        }
    }

    #[test]
    fn unsupported_semantic_rows_require_evidence_and_conservative_labels() {
        let interner = StableKeyInterner::default();
        assert!(unsupported(&interner, "unsupported:complete").is_complete());

        let incomplete = UnsupportedSemanticFact {
            construct: String::new(),
            source_evidence: String::new(),
            affected_domains: Vec::new(),
            ..unsupported(&interner, "unsupported:missing")
        };

        assert!(!incomplete.is_complete());
    }

    #[test]
    fn mir_operation_kind_covers_call_shaped_operations_without_targets() {
        let operation = MirOperationKind::Call {
            site: CallSiteId(1),
            callee: MirValue::Unknown {
                evidence: "dynamic callee".to_string(),
            },
            arguments: vec![PlaceId(2)],
            return_place: PlaceId(3),
        };

        assert!(matches!(operation, MirOperationKind::Call { .. }));
    }
}
