use std::collections::{BTreeMap, BTreeSet};

use tree_sitter::{Node, Parser};

use crate::analysis_api::{FactFamily, FunctionFact, SourceFile};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::ids::{
    CallSiteId, MirBodyId, MirOpId, MirPredicateId, MirStatementId, MirTerminatorId, PlaceId,
    UnsupportedId,
};
use crate::analysis_neutral::mir_body::{
    MirBlock, MirBlockId, MirBody, MirOutput, MirStatement, MirStatus, MirTerminator,
    MirTerminatorKind, SuspendKind,
};
use crate::analysis_neutral::mir_op::{
    AssignMode, BranchNilTest, ConservativeAction, MirAggregateField, MirAggregateKind,
    MirOperation, MirOperationKind, MirValue, UnsupportedDomain, UnsupportedPrecision,
    UnsupportedSemanticFact,
};
use crate::analysis_neutral::places::{
    PlaceInsert, PlaceProjection, PlaceRoot, PlaceStableContext, PlaceStatus, PlaceTableBuilder,
};
use crate::analysis_neutral::stable_key::semantic_stable_key;
use crate::analysis_neutral::types::facts::TypeShape;
use crate::internal_core::{FileId, FunctionId, Language, Span, StableKeyId};

#[doc(hidden)]
pub fn lower_go_mir(db: &impl AnalysisHost) -> MirOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut lowering = GoMirLowering::default();
    let mut files = db
        .files()
        .iter()
        .filter(|file| file.language == Language::Go)
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    for file in files {
        lowering.lower_file(interner, db, file);
    }

    let (places, place_types) = lowering.places.clone().finish_with_types(interner);
    let place_ids = places
        .iter()
        .map(|place| (interner.resolve(place.stable_key).to_string(), place.id))
        .collect::<BTreeMap<_, _>>();
    let operations = lowering.finish_operations(&place_ids);
    let unsupported = lowering.finish_unsupported(interner, &place_ids);
    let control_effects = lowering
        .operations
        .iter()
        .filter_map(|operation| operation.to_control_effect(&place_ids))
        .collect::<Vec<_>>();
    let call_unwinds = lowering
        .operations
        .iter()
        .filter_map(|operation| match operation.kind {
            OperationKindDraft::Call { unwind: true, .. } => Some(operation.id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let control_shapes = lowering
        .operations
        .iter()
        .filter_map(|operation| match operation.kind {
            OperationKindDraft::Branch { shape, .. } => Some((operation.id, shape)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let branch_regions = lowering
        .operations
        .iter()
        .filter_map(|operation| match operation.kind {
            OperationKindDraft::Branch { region, .. } => Some((operation.id, region)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let (blocks, statements, terminators) = lower_control_flow(
        interner,
        &lowering.bodies,
        &operations,
        &control_shapes,
        &branch_regions,
        &control_effects,
        &call_unwinds,
    );

    MirOutput {
        bodies: lowering.bodies,
        blocks,
        statements,
        terminators,
        places,
        place_types,
        operations,
        unsupported,
    }
    .normalized(interner)
}

fn lower_control_flow(
    interner: &crate::internal_core::StableKeyInterner,
    bodies: &[MirBody],
    operations: &[MirOperation],
    control_shapes: &BTreeMap<MirOpId, ControlShape>,
    branch_regions: &BTreeMap<MirOpId, BranchRegion>,
    control_effects: &[ControlEffect],
    call_unwinds: &BTreeSet<MirOpId>,
) -> (Vec<MirBlock>, Vec<MirStatement>, Vec<MirTerminator>) {
    let mut blocks = Vec::new();
    let mut statements = Vec::new();
    let mut terminators = Vec::new();

    for body in bodies {
        let body_operations = operations
            .iter()
            .filter(|operation| operation.body == body.id)
            .map(|operation| (operation.id, operation))
            .collect::<BTreeMap<_, _>>();
        let body_effects = control_effects
            .iter()
            .filter(|effect| effect.body == body.id)
            .map(|effect| (effect.id, effect))
            .collect::<BTreeMap<_, _>>();
        let step_ids = body_operations
            .keys()
            .chain(body_effects.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        let mut drafts = vec![BlockDraft::new(
            MirBlockId(blocks.len() as u64),
            (body.id, 0),
        )];
        let mut current = push_block_draft(&mut drafts, blocks.len(), body.id);
        drafts[0].terminator = Some(MirTerminatorKind::Goto {
            target: drafts[current].id,
        });
        let step_count = step_ids.len();
        let mut transitions = BTreeMap::<MirOpId, Vec<RegionTransition>>::new();

        for (step_index, step_id) in step_ids.into_iter().enumerate() {
            if let Some(operation) = body_operations.get(&step_id).copied() {
                let operation_stable_key = interner.resolve(operation.stable_key);
                let statement = MirStatement {
                    id: MirStatementId(statements.len() as u64),
                    body: body.id,
                    ordinal: operation.ordinal,
                    operation: operation.id,
                    stable_key: interner.intern(format!("{operation_stable_key}:statement")),
                    status: operation.status,
                };
                drafts[current].statements.push(statement.id);
                statements.push(statement);

                match &operation.kind {
                    MirOperationKind::Branch {
                        predicate,
                        predicate_place,
                        ..
                    } => {
                        let shape = control_shapes
                            .get(&operation.id)
                            .copied()
                            .unwrap_or(ControlShape::Conditional);
                        let first = push_block_draft(&mut drafts, blocks.len(), body.id);
                        let second = push_block_draft(&mut drafts, blocks.len(), body.id);
                        let join = push_block_draft(&mut drafts, blocks.len(), body.id);
                        drafts[current].terminator = Some(match shape {
                            ControlShape::Conditional => MirTerminatorKind::Branch {
                                predicate: *predicate,
                                predicate_place: *predicate_place,
                                then_target: drafts[first].id,
                                else_target: drafts[second].id,
                            },
                            ControlShape::Loop => MirTerminatorKind::Branch {
                                predicate: *predicate,
                                predicate_place: *predicate_place,
                                then_target: drafts[first].id,
                                else_target: drafts[join].id,
                            },
                            ControlShape::Switch => MirTerminatorKind::Switch {
                                discriminant: MirValue::Unknown {
                                    evidence: operation_stable_key.to_string(),
                                },
                                cases: vec![(
                                    MirValue::Unknown {
                                        evidence: "case".to_string(),
                                    },
                                    drafts[first].id,
                                )],
                                otherwise: drafts[second].id,
                            },
                        });
                        drafts[first].terminator = Some(MirTerminatorKind::Goto {
                            target: if shape == ControlShape::Loop {
                                drafts[current].id
                            } else {
                                drafts[join].id
                            },
                        });
                        drafts[second].terminator = Some(MirTerminatorKind::Goto {
                            target: drafts[join].id,
                        });
                        if shape == ControlShape::Conditional {
                            let region = branch_regions
                                .get(&operation.id)
                                .copied()
                                .unwrap_or_default();
                            match region.then_end {
                                Some(then_end) => {
                                    drafts[first].terminator = None;
                                    current = first;
                                    transitions.entry(then_end).or_default().push(
                                        RegionTransition::FinishThen {
                                            second,
                                            join,
                                            has_else: region.else_end.is_some(),
                                        },
                                    );
                                    if let Some(else_end) = region.else_end {
                                        drafts[second].terminator = None;
                                        transitions
                                            .entry(else_end)
                                            .or_default()
                                            .push(RegionTransition::FinishElse { join });
                                    }
                                }
                                None if region.else_end.is_some() => {
                                    drafts[second].terminator = None;
                                    current = second;
                                    transitions
                                        .entry(region.else_end.expect("else end"))
                                        .or_default()
                                        .push(RegionTransition::FinishElse { join });
                                }
                                None => current = join,
                            }
                        } else {
                            current = join;
                        }
                    }
                    MirOperationKind::Return { value } => {
                        drafts[current].terminator = Some(MirTerminatorKind::Return {
                            value: value.clone(),
                        });
                        if step_index + 1 < step_count {
                            current = push_block_draft(&mut drafts, blocks.len(), body.id);
                            drafts[current].terminator = Some(MirTerminatorKind::Unreachable);
                        }
                    }
                    MirOperationKind::Call {
                        site,
                        callee,
                        arguments,
                        return_place,
                    } => {
                        let normal = push_block_draft(&mut drafts, blocks.len(), body.id);
                        let unwind = call_unwinds.contains(&operation.id).then(|| {
                            let unwind = push_block_draft(&mut drafts, blocks.len(), body.id);
                            drafts[unwind].terminator = Some(MirTerminatorKind::Unreachable);
                            drafts[unwind].id
                        });
                        drafts[current].terminator = Some(MirTerminatorKind::Call {
                            site: *site,
                            callee: callee.clone(),
                            arguments: arguments.clone(),
                            return_place: *return_place,
                            normal: drafts[normal].id,
                            unwind,
                        });
                        current = normal;
                    }
                    MirOperationKind::Unsupported { unsupported }
                        if step_index + 1 == step_count =>
                    {
                        drafts[current].terminator = Some(MirTerminatorKind::Unsupported {
                            unsupported: *unsupported,
                        });
                    }
                    _ => {}
                }
            } else if let Some(effect) = body_effects.get(&step_id) {
                match &effect.kind {
                    ControlEffectKind::Suspend { kind, value } => {
                        let resume = push_block_draft(&mut drafts, blocks.len(), body.id);
                        drafts[current].terminator = Some(MirTerminatorKind::Suspend {
                            kind: *kind,
                            value: value.clone(),
                            resume: drafts[resume].id,
                        });
                        current = resume;
                    }
                }
            }
            if let Some(region_transitions) = transitions.remove(&step_id) {
                for transition in region_transitions.into_iter().rev() {
                    match transition {
                        RegionTransition::FinishThen {
                            second,
                            join,
                            has_else,
                        } => {
                            set_goto_if_open(&mut drafts, current, join);
                            if has_else {
                                current = second;
                            } else {
                                current = join;
                            }
                        }
                        RegionTransition::FinishElse { join } => {
                            set_goto_if_open(&mut drafts, current, join);
                            current = join;
                        }
                    }
                }
            }
        }

        // A region closes on the last operation its arm lowered. That operation
        // is dropped when its place never made the place table, which would
        // otherwise leave `current` inside the arm and parent the rest of the
        // body to it. Close whatever is still open, innermost region first.
        for (_, region_transitions) in std::mem::take(&mut transitions) {
            for transition in region_transitions.into_iter().rev() {
                match transition {
                    RegionTransition::FinishThen {
                        second,
                        join,
                        has_else,
                    } => {
                        set_goto_if_open(&mut drafts, current, join);
                        current = if has_else { second } else { join };
                    }
                    RegionTransition::FinishElse { join } => {
                        set_goto_if_open(&mut drafts, current, join);
                        current = join;
                    }
                }
            }
        }

        if drafts[current].terminator.is_none() {
            drafts[current].terminator = Some(MirTerminatorKind::Return { value: None });
        }
        let body_stable_key = interner.resolve(body.stable_key);
        for draft in drafts {
            let kind = draft
                .terminator
                .expect("every lowered MIR block must have a terminator");
            let terminator = MirTerminator {
                id: MirTerminatorId(terminators.len() as u64),
                body: draft.body,
                ordinal: draft.ordinal,
                stable_key: interner
                    .intern(format!("{body_stable_key}:terminator:{}", draft.ordinal)),
                status: body.status,
                kind,
            };
            blocks.push(MirBlock {
                id: draft.id,
                body: draft.body,
                ordinal: draft.ordinal,
                statements: draft.statements,
                terminator: terminator.id,
                stable_key: interner.intern(format!("{body_stable_key}:block:{}", draft.ordinal)),
            });
            terminators.push(terminator);
        }
    }

    (blocks, statements, terminators)
}

struct ControlEffect {
    id: MirOpId,
    body: MirBodyId,
    kind: ControlEffectKind,
}

enum ControlEffectKind {
    Suspend {
        kind: SuspendKind,
        value: Option<MirValue>,
    },
}

enum RegionTransition {
    FinishThen {
        second: usize,
        join: usize,
        has_else: bool,
    },
    FinishElse {
        join: usize,
    },
}

fn set_goto_if_open(drafts: &mut [BlockDraft], from: usize, to: usize) {
    if drafts[from].terminator.is_none() {
        drafts[from].terminator = Some(MirTerminatorKind::Goto {
            target: drafts[to].id,
        });
    }
}

struct BlockDraft {
    id: MirBlockId,
    body: MirBodyId,
    ordinal: u32,
    statements: Vec<MirStatementId>,
    terminator: Option<MirTerminatorKind>,
}

impl BlockDraft {
    fn new(id: MirBlockId, pair: (MirBodyId, u32)) -> Self {
        let (body, ordinal) = pair;
        Self {
            id,
            body,
            ordinal,
            statements: Vec::new(),
            terminator: None,
        }
    }
}

fn push_block_draft(drafts: &mut Vec<BlockDraft>, base: usize, body: MirBodyId) -> usize {
    let index = drafts.len();
    drafts.push(BlockDraft::new(
        MirBlockId((base + index) as u64),
        (body, index as u32),
    ));
    index
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlShape {
    Conditional,
    Loop,
    Switch,
}

#[derive(Debug, Clone, Copy, Default)]
struct BranchRegion {
    then_end: Option<MirOpId>,
    else_end: Option<MirOpId>,
}

fn set_branch_region(operations: &mut [OperationDraft], branch: MirOpId, region: BranchRegion) {
    let operation = operations
        .iter_mut()
        .find(|operation| operation.id == branch)
        .expect("branch operation should exist");
    let OperationKindDraft::Branch { region: stored, .. } = &mut operation.kind else {
        panic!("branch operation id should identify a branch");
    };
    *stored = region;
}

/// Records the value a switch dispatches on.
///
/// Switch branches lower to a `Switch` terminator whose case and default edges
/// carry no truthiness or nilness assumption, so the recorded place is
/// descriptive evidence only and never refines an abstract state.
fn set_switch_discriminant(
    operations: &mut [OperationDraft],
    branch: MirOpId,
    predicate_place_key: Option<String>,
) {
    let operation = operations
        .iter_mut()
        .find(|operation| operation.id == branch)
        .expect("branch operation should exist");
    let OperationKindDraft::Branch {
        predicate_place_key: stored_place,
        ..
    } = &mut operation.kind
    else {
        panic!("branch operation id should identify a branch");
    };
    *stored_place = predicate_place_key;
}

#[derive(Debug, Default)]
struct GoMirLowering {
    bodies: Vec<MirBody>,
    places: PlaceTableBuilder,
    operations: Vec<OperationDraft>,
    unsupported: Vec<UnsupportedDraft>,
}

impl GoMirLowering {
    fn lower_file(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        db: &impl AnalysisHost,
        file: &SourceFile,
    ) {
        let mut parser = Parser::new();
        if parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .is_err()
        {
            return;
        }
        let Some(tree) = parser.parse(file.source.as_ref(), None) else {
            return;
        };
        let root = tree.root_node();
        let mut functions = Vec::new();
        visit_named_descendants(root, &mut |node| {
            if matches!(node.kind(), "function_declaration" | "method_declaration") {
                functions.push(node);
            }
        });
        functions.sort_by(|left, right| {
            (
                file.relative_path.as_str(),
                left.start_byte(),
                left.end_byte(),
                function_name(file.source.as_ref(), *left).unwrap_or_default(),
            )
                .cmp(&(
                    file.relative_path.as_str(),
                    right.start_byte(),
                    right.end_byte(),
                    function_name(file.source.as_ref(), *right).unwrap_or_default(),
                ))
        });

        for node in functions {
            let Some(body_node) = node.child_by_field_name("body") else {
                continue;
            };
            let Some(name) = function_name(file.source.as_ref(), node) else {
                continue;
            };
            let span = node_span(file, node);
            let Some(function) = matching_function(db, file.id, &name, &span) else {
                continue;
            };
            let body = self.push_body(interner, db, file, function, span);
            let mut literals = Vec::new();
            visit_named_descendants(body_node, &mut |node| {
                if node.kind() == "func_literal" {
                    literals.push(node);
                }
            });
            literals.sort_by_key(|node| (node.start_byte(), node.end_byte()));
            let closure_bodies = literals
                .iter()
                .map(|node| {
                    let closure_body =
                        self.push_body(interner, db, file, function, node_span(file, *node));
                    (
                        (node.start_byte() as u32, node.end_byte() as u32),
                        closure_body,
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let closure_capture_names = go_closure_capture_names(
                db,
                file.id,
                file.source.as_ref(),
                closure_bodies.keys().copied(),
            );
            let closure_body_ids = closure_bodies
                .iter()
                .map(|(span, body)| (*span, body.id))
                .collect::<BTreeMap<_, _>>();
            let mut function_lowering = FunctionLowering::new(
                interner,
                file,
                file.source.as_ref(),
                function.id,
                &body,
                closure_body_ids.clone(),
                closure_capture_names.clone(),
            );
            function_lowering.lower_parameters(node, &mut self.places);
            function_lowering.lower_body(
                interner,
                body_node,
                &mut self.places,
                &mut self.operations,
                &mut self.unsupported,
            );
            for literal in literals {
                let Some(closure_body) =
                    closure_bodies.get(&(literal.start_byte() as u32, literal.end_byte() as u32))
                else {
                    continue;
                };
                let Some(closure_block) = literal.child_by_field_name("body") else {
                    continue;
                };
                let mut closure_lowering = FunctionLowering::new(
                    interner,
                    file,
                    file.source.as_ref(),
                    function.id,
                    closure_body,
                    closure_body_ids.clone(),
                    closure_capture_names.clone(),
                );
                closure_lowering.lower_parameters(literal, &mut self.places);
                closure_lowering.lower_body(
                    interner,
                    closure_block,
                    &mut self.places,
                    &mut self.operations,
                    &mut self.unsupported,
                );
            }
            let body_stable_key = interner.resolve(body.stable_key);
            self.lower_parser_errors(interner, file, body.id, &body_stable_key, body_node);
        }
    }

    fn push_body(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        db: &impl AnalysisHost,
        file: &SourceFile,
        function: &FunctionFact,
        span: Span,
    ) -> MirBody {
        let id = MirBodyId(self.bodies.len() as u64);
        let owner_stable_key_text = owner_stable_key(file, function);
        let stable_key_text = semantic_stable_key(
            FactFamily::MirBody,
            &[
                ("language", "go".to_string()),
                ("path", file.relative_path.clone()),
                ("owner", owner_stable_key_text.clone()),
                ("start_byte", span.start_byte.to_string()),
                ("end_byte", span.end_byte.to_string()),
            ],
        )
        .into_string();
        let owner_stable_key = interner.intern(owner_stable_key_text);
        let stable_key = interner.intern(stable_key_text);
        let body = MirBody {
            id,
            language: Language::Go,
            file: file.id,
            function: function.id,
            package: db
                .packages()
                .iter()
                .find(|package| package.file == file.id && package.language == Language::Go)
                .map(|package| package.id),
            module: db
                .module_nodes()
                .iter()
                .find(|module| {
                    module.file == Some(file.id) && module.language == Some(Language::Go)
                })
                .map(|module| module.id),
            owner_stable_key,
            span,
            stable_key,
            status: MirStatus::Partial,
        };
        self.bodies.push(body.clone());
        body
    }

    fn lower_parser_errors(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        file: &SourceFile,
        body: MirBodyId,
        body_stable_key: &str,
        node: Node<'_>,
    ) {
        visit_named_descendants(node, &mut |descendant| {
            if descendant.is_error() || descendant.kind() == "ERROR" {
                let unsupported_id = UnsupportedId(self.unsupported.len() as u64);
                let operation_id = MirOpId(self.operations.len() as u64);
                let span = node_span(file, descendant);
                self.unsupported
                    .push(UnsupportedDraft::new(UnsupportedDraftInput {
                        id: unsupported_id,
                        body: Some(body),
                        operation: Some(operation_id),
                        file_key: file.relative_path.clone(),
                        file: file.id,
                        span: span.clone(),
                        construct: "ERROR".to_string(),
                        source_evidence: node_text(file.source.as_ref(), descendant)
                            .unwrap_or("ERROR"),
                        affected_domains: vec![UnsupportedDomain::Mir, UnsupportedDomain::Cfg],
                        conservative_action: ConservativeAction::StopLowering,
                    }));
                self.operations.push(OperationDraft::new(
                    interner,
                    operation_id,
                    body,
                    body_stable_key,
                    operation_id.0 as u32,
                    span,
                    (
                        OperationKindDraft::Unsupported { unsupported_id },
                        MirStatus::Unsupported,
                    ),
                ));
            }
        });
    }

    fn finish_operations(&self, place_ids: &BTreeMap<String, PlaceId>) -> Vec<MirOperation> {
        self.operations
            .iter()
            .filter_map(|draft| draft.to_operation(place_ids))
            .collect()
    }

    fn finish_unsupported(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        place_ids: &BTreeMap<String, PlaceId>,
    ) -> Vec<UnsupportedSemanticFact> {
        self.unsupported
            .iter()
            .map(|draft| draft.to_fact(interner, place_ids))
            .collect()
    }
}

fn go_closure_capture_names(
    db: &impl AnalysisHost,
    file: FileId,
    source: &str,
    spans: impl IntoIterator<Item = (u32, u32)>,
) -> BTreeMap<(u32, u32), Vec<String>> {
    spans
        .into_iter()
        .map(|span| {
            let mut names = db
                .references_for_file(file)
                .into_iter()
                .filter(|reference| {
                    reference
                        .primary_span
                        .as_ref()
                        .is_some_and(|reference_span| {
                            reference_span.start_byte >= span.0 && reference_span.end_byte <= span.1
                        })
                })
                .filter_map(|reference| {
                    let target = reference.target?;
                    let definition = db.definition_for_symbol(target)?;
                    let definition_span = definition.primary_span.as_ref()?;
                    (definition_span.start_byte < span.0 || definition_span.end_byte > span.1)
                        .then(|| reference.name.clone())
                })
                .collect::<BTreeSet<_>>();
            if let Some(body_source) = source.get(span.0 as usize..span.1 as usize) {
                names.extend(identifier_tokens(body_source));
            }
            (span, names.into_iter().collect())
        })
        .collect()
}

fn identifier_tokens(source: &str) -> impl Iterator<Item = String> + '_ {
    source
        .split(|character: char| !(character == '_' || character.is_alphanumeric()))
        .filter(|token| {
            token
                .chars()
                .next()
                .is_some_and(|character| character == '_' || character.is_alphabetic())
        })
        .map(str::to_string)
}

struct FunctionLowering<'source> {
    file: FileId,
    source_file: &'source SourceFile,
    source: &'source str,
    function: FunctionId,
    body: MirBodyId,
    stable_context: PlaceStableContext,
    closure_bodies: BTreeMap<(u32, u32), MirBodyId>,
    closure_capture_names: BTreeMap<(u32, u32), Vec<String>>,
    parameters: BTreeMap<String, PlaceRoot>,
    locals: BTreeMap<String, PlaceRoot>,
}

impl<'source> FunctionLowering<'source> {
    fn new(
        interner: &crate::internal_core::StableKeyInterner,
        file: &'source SourceFile,
        source: &'source str,
        function: FunctionId,
        body: &MirBody,
        closure_bodies: BTreeMap<(u32, u32), MirBodyId>,
        closure_capture_names: BTreeMap<(u32, u32), Vec<String>>,
    ) -> Self {
        Self {
            file: file.id,
            source_file: file,
            source,
            function,
            body: body.id,
            stable_context: PlaceStableContext::new(
                file.relative_path.clone(),
                interner.resolve(body.owner_stable_key).to_string(),
                interner.resolve(body.stable_key).to_string(),
            ),
            closure_bodies,
            closure_capture_names,
            parameters: BTreeMap::new(),
            locals: BTreeMap::new(),
        }
    }

    fn lower_parameters(&mut self, node: Node<'_>, places: &mut PlaceTableBuilder) {
        let mut index = 0_u32;
        if let Some(receiver) = node.child_by_field_name("receiver") {
            let name = parameter_names(self.source, receiver).into_iter().next();
            let root = PlaceRoot::Parameter {
                function: self.function,
                index,
                name: name.clone(),
            };
            if let Some(name) = name {
                self.parameters.insert(name, root.clone());
            }
            self.insert_place(places, root, Vec::new(), PlaceStatus::Resolved);
            index += 1;
        }

        if let Some(parameters) = node.child_by_field_name("parameters") {
            for name in parameter_names(self.source, parameters) {
                let root = PlaceRoot::Parameter {
                    function: self.function,
                    index,
                    name: Some(name.clone()),
                };
                self.parameters.insert(name, root.clone());
                self.insert_place(places, root, Vec::new(), PlaceStatus::Resolved);
                index += 1;
            }
        }
    }

    fn lower_body(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        body: Node<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) {
        for index in 0..body.named_child_count() as u32 {
            let Some(statement) = body.named_child(index) else {
                continue;
            };
            self.lower_statement(interner, statement, places, operations, unsupported);
        }
    }

    fn lower_statement(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        statement: Node<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) {
        self.lower_unsupported(interner, statement, operations, unsupported);
        match statement.kind() {
            "short_var_declaration" => {
                let right = statement.child_by_field_name("right");
                let value = right
                    .and_then(|right| {
                        self.lower_value(interner, right, places, operations, unsupported)
                    })
                    .unwrap_or_else(|| ValueDraft::Unknown {
                        evidence: "short declaration initializer".to_string(),
                    });
                for name in assignment_left_names(self.source, statement) {
                    let key = self.insert_local_typed(
                        places,
                        &name,
                        right.map(type_shape_for_go_expression),
                    );
                    self.push_assign(
                        interner,
                        operations,
                        statement,
                        key,
                        value.clone(),
                        AssignMode::DeclarationBinding,
                    );
                }
            }
            "var_declaration" => {
                let mut names = BTreeSet::new();
                visit_named_descendants(statement, &mut |node| {
                    if node.kind() == "var_spec" {
                        names.extend(var_spec_names(self.source, node));
                    }
                });
                for name in names {
                    let key = self.insert_local(places, &name);
                    self.push_assign(
                        interner,
                        operations,
                        statement,
                        key,
                        ValueDraft::Unknown {
                            evidence: "zero value".to_string(),
                        },
                        AssignMode::DeclarationBinding,
                    );
                }
            }
            "assignment_statement" => {
                let left_places = self.assignment_left_places(interner, statement, places);
                let value = statement
                    .child_by_field_name("right")
                    .and_then(|right| {
                        self.lower_value(interner, right, places, operations, unsupported)
                    })
                    .unwrap_or_else(|| ValueDraft::Unknown {
                        evidence: "assignment value".to_string(),
                    });
                let simultaneous = left_places.len() > 1;
                let compound = assignment_operator(self.source, statement)
                    .is_some_and(|operator| operator != "=");
                for place in left_places {
                    let mode = if compound {
                        AssignMode::PartialWrite
                    } else if simultaneous {
                        AssignMode::Simultaneous
                    } else if !place.projections.is_empty() {
                        AssignMode::ProjectionMutation
                    } else {
                        AssignMode::Overwrite
                    };
                    self.push_assign(
                        interner,
                        operations,
                        statement,
                        place.key,
                        value.clone(),
                        mode,
                    );
                }
            }
            "if_statement" => {
                if let Some(initializer) = statement.child_by_field_name("initializer") {
                    self.lower_statement(interner, initializer, places, operations, unsupported);
                }
                let predicate_place_key =
                    if let Some(condition) = statement.child_by_field_name("condition") {
                        let predicate = go_nil_operand(self.source, condition).unwrap_or(condition);
                        self.lower_expression(
                            interner,
                            predicate,
                            places,
                            operations,
                            unsupported,
                            false,
                        )
                        .map(|source| source.key)
                    } else {
                        None
                    };
                let branch = self.push_branch(
                    interner,
                    operations,
                    statement
                        .child_by_field_name("condition")
                        .unwrap_or(statement),
                    predicate_place_key,
                    statement
                        .child_by_field_name("condition")
                        .and_then(|condition| go_nil_test(self.source, condition)),
                );
                let then_start = operations.len();
                if let Some(consequence) = statement.child_by_field_name("consequence") {
                    self.lower_statement(interner, consequence, places, operations, unsupported);
                }
                let then_end = (operations.len() > then_start)
                    .then(|| operations.last().expect("then operation").id);
                let else_start = operations.len();
                if let Some(alternative) = statement.child_by_field_name("alternative") {
                    self.lower_statement(interner, alternative, places, operations, unsupported);
                }
                let else_end = (operations.len() > else_start)
                    .then(|| operations.last().expect("else operation").id);
                set_branch_region(operations, branch, BranchRegion { then_end, else_end });
            }
            "for_statement"
            | "expression_switch_statement"
            | "type_switch_statement"
            | "switch_statement" => {
                let branch = self.push_branch(interner, operations, statement, None, None);
                for index in 0..statement.named_child_count() as u32 {
                    let Some(child) = statement.named_child(index) else {
                        continue;
                    };
                    self.lower_statement(interner, child, places, operations, unsupported);
                }
                // Loop conditions are deliberately left unbound. A loop body is
                // lowered after its branch operation, so it lands in the block the
                // branch reaches on its `false` edge; binding the condition would
                // apply the loop-exit assumption to the body instead.
                if let Some(discriminant) = go_switch_discriminant(statement) {
                    let predicate_place_key = self
                        .lower_expression(
                            interner,
                            discriminant,
                            places,
                            &mut Vec::new(),
                            &mut Vec::new(),
                            false,
                        )
                        .map(|shape| shape.key);
                    set_switch_discriminant(operations, branch, predicate_place_key);
                }
            }
            "return_statement" => {
                let value = statement.named_child(0).and_then(|child| {
                    self.lower_value(interner, child, places, operations, unsupported)
                });
                self.push_operation(
                    interner,
                    operations,
                    statement,
                    OperationKindDraft::Return { value },
                    MirStatus::Partial,
                );
            }
            "call_expression" => {
                self.lower_call(interner, statement, places, operations, unsupported);
            }
            "unary_expression"
                if node_text(self.source, statement)
                    .is_some_and(|text| text.trim_start().starts_with("<-")) =>
            {
                self.lower_expression(interner, statement, places, operations, unsupported, false);
            }
            "send_statement" => {
                let value = self.lower_expression_children(
                    interner,
                    statement,
                    places,
                    operations,
                    unsupported,
                    false,
                );
                self.push_operation(
                    interner,
                    operations,
                    statement,
                    OperationKindDraft::Suspend {
                        kind: SuspendKind::ChannelSend,
                        value: value.map(|shape| ValueDraft::PlaceKey(shape.key)),
                    },
                    MirStatus::Partial,
                );
            }
            "identifier" | "selector_expression" | "index_expression" | "binary_expression" => {
                if let Some(shape) = self.lower_expression(
                    interner,
                    statement,
                    places,
                    operations,
                    unsupported,
                    false,
                ) {
                    self.push_operation(
                        interner,
                        operations,
                        statement,
                        OperationKindDraft::Read {
                            place_key: shape.key,
                        },
                        MirStatus::Partial,
                    );
                }
            }
            _ => {
                for index in 0..statement.named_child_count() as u32 {
                    let Some(child) = statement.named_child(index) else {
                        continue;
                    };
                    self.lower_statement(interner, child, places, operations, unsupported);
                }
            }
        }
    }

    fn assignment_left_places(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        statement: Node<'_>,
        places: &mut PlaceTableBuilder,
    ) -> Vec<PlaceShape> {
        if let Some(left) = statement.child_by_field_name("left") {
            let mut shapes = Vec::new();
            for index in 0..left.named_child_count() as u32 {
                if let Some(child) = left.named_child(index)
                    && let Some(shape) = self.lower_expression(
                        interner,
                        child,
                        places,
                        &mut Vec::new(),
                        &mut Vec::new(),
                        true,
                    )
                {
                    shapes.push(shape);
                }
            }
            if !shapes.is_empty() {
                return shapes;
            }
        }

        assignment_left_names(self.source, statement)
            .into_iter()
            .map(|name| {
                if let Some(root) = self
                    .locals
                    .get(&name)
                    .or_else(|| self.parameters.get(&name))
                {
                    PlaceShape {
                        root: root.clone(),
                        projections: Vec::new(),
                        status: PlaceStatus::Resolved,
                        key: self.insert_place(
                            places,
                            root.clone(),
                            Vec::new(),
                            PlaceStatus::Resolved,
                        ),
                    }
                } else {
                    let root = PlaceRoot::Global { symbol: None, name };
                    let key =
                        self.insert_place(places, root.clone(), Vec::new(), PlaceStatus::Partial);
                    PlaceShape {
                        root,
                        projections: Vec::new(),
                        status: PlaceStatus::Partial,
                        key,
                    }
                }
            })
            .collect()
    }

    fn insert_local(&mut self, places: &mut PlaceTableBuilder, name: &str) -> String {
        self.insert_local_typed(places, name, None)
    }

    fn insert_local_typed(
        &mut self,
        places: &mut PlaceTableBuilder,
        name: &str,
        ty: Option<TypeShape>,
    ) -> String {
        let root = PlaceRoot::Local {
            function: self.function,
            name: name.to_string(),
        };
        self.locals.insert(name.to_string(), root.clone());
        self.insert_typed_place(places, root, Vec::new(), ty, PlaceStatus::Resolved)
    }

    fn lower_value(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        node: Node<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) -> Option<ValueDraft> {
        match node.kind() {
            "expression_list" if node.named_child_count() == 1 => self.lower_value(
                interner,
                node.named_child(0)?,
                places,
                operations,
                unsupported,
            ),
            "interpreted_string_literal"
            | "raw_string_literal"
            | "int_literal"
            | "float_literal"
            | "rune_literal"
            | "true"
            | "false"
            | "nil" => Some(ValueDraft::Literal {
                value: node_text(self.source, node).unwrap_or_default().to_string(),
            }),
            "binary_expression" => {
                let left = node
                    .child_by_field_name("left")
                    .or_else(|| node.named_child(0))?;
                let right = node
                    .child_by_field_name("right")
                    .or_else(|| node.named_child(1))?;
                Some(ValueDraft::BinOp {
                    op: go_binary_operator_text(self.source, left, right),
                    lhs: Box::new(
                        self.lower_value(interner, left, places, operations, unsupported)
                            .unwrap_or_else(|| ValueDraft::Unknown {
                                evidence: "binary left operand".to_string(),
                            }),
                    ),
                    rhs: Box::new(
                        self.lower_value(interner, right, places, operations, unsupported)
                            .unwrap_or_else(|| ValueDraft::Unknown {
                                evidence: "binary right operand".to_string(),
                            }),
                    ),
                })
            }
            "composite_literal" => {
                let literal = node.child_by_field_name("body").or_else(|| {
                    (0..node.named_child_count() as u32)
                        .filter_map(|index| node.named_child(index))
                        .find(|child| child.kind() == "literal_value")
                })?;
                let mut fields = Vec::new();
                for index in 0..literal.named_child_count() as u32 {
                    let Some(element) = literal.named_child(index) else {
                        continue;
                    };
                    let (name, value_node) = if element.kind() == "keyed_element" {
                        let key = element
                            .child_by_field_name("key")
                            .or_else(|| element.named_child(0));
                        let value = element
                            .child_by_field_name("value")
                            .or_else(|| element.named_child(1));
                        (
                            key.and_then(|key| node_text(self.source, key))
                                .map(str::to_string),
                            value,
                        )
                    } else {
                        (None, Some(element))
                    };
                    let value = value_node
                        .and_then(|value| {
                            self.lower_value(interner, value, places, operations, unsupported)
                        })
                        .unwrap_or_else(|| ValueDraft::Unknown {
                            evidence: "composite element value".to_string(),
                        });
                    fields.push((name, value));
                }
                Some(ValueDraft::Aggregate {
                    kind: MirAggregateKind::Composite,
                    fields,
                })
            }
            "func_literal" => self.closure_value(node, places),
            "call_expression" => self
                .lower_call(interner, node, places, operations, unsupported)
                .map(ValueDraft::PlaceKey),
            _ => self
                .lower_expression(interner, node, places, operations, unsupported, false)
                .map(|shape| ValueDraft::PlaceKey(shape.key)),
        }
    }

    fn closure_value(&self, node: Node<'_>, places: &mut PlaceTableBuilder) -> Option<ValueDraft> {
        let span = (node.start_byte() as u32, node.end_byte() as u32);
        let body = *self.closure_bodies.get(&span)?;
        let capture_keys = self
            .closure_capture_names
            .get(&span)
            .into_iter()
            .flatten()
            .filter_map(|name| {
                self.locals
                    .get(name)
                    .or_else(|| self.parameters.get(name))
                    .map(|root| {
                        self.insert_place(places, root.clone(), Vec::new(), PlaceStatus::Resolved)
                    })
            })
            .collect();
        Some(ValueDraft::Closure { body, capture_keys })
    }

    fn lower_expression(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        node: Node<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
        assignment_destination: bool,
    ) -> Option<PlaceShape> {
        if node_text(self.source, node).is_some_and(|text| text.trim_start().starts_with("<-"))
            || node
                .child_by_field_name("operator")
                .and_then(|operator| node_text(self.source, operator))
                == Some("<-")
        {
            let value = self.lower_expression_children(
                interner,
                node,
                places,
                operations,
                unsupported,
                false,
            );
            self.push_operation(
                interner,
                operations,
                node,
                OperationKindDraft::Suspend {
                    kind: SuspendKind::ChannelRecv,
                    value: value
                        .as_ref()
                        .map(|shape| ValueDraft::PlaceKey(shape.key.clone())),
                },
                MirStatus::Partial,
            );
            return value;
        }
        self.lower_unsupported(interner, node, operations, unsupported);
        match node.kind() {
            "identifier" => self.lower_identifier(node, places, assignment_destination),
            "selector_expression" => self.lower_selector(
                interner,
                node,
                places,
                operations,
                unsupported,
                assignment_destination,
            ),
            "index_expression" => self.lower_index(
                interner,
                node,
                places,
                operations,
                unsupported,
                assignment_destination,
            ),
            "call_expression" => self
                .lower_call(interner, node, places, operations, unsupported)
                .map(|key| PlaceShape {
                    root: PlaceRoot::CallReturn {
                        call: self.call_site_for(node),
                    },
                    projections: Vec::new(),
                    status: PlaceStatus::Partial,
                    key,
                }),
            "func_literal" => {
                let value = self.closure_value(node, places)?;
                let key = self.insert_temporary_typed(
                    places,
                    node,
                    PlaceStatus::Partial,
                    TypeShape::Callable {
                        signature: "closure".to_string(),
                    },
                );
                self.push_assign(
                    interner,
                    operations,
                    node,
                    key.clone(),
                    value,
                    AssignMode::Overwrite,
                );
                Some(PlaceShape {
                    root: PlaceRoot::Temporary {
                        body: self.body,
                        ordinal: node.start_byte() as u32,
                    },
                    projections: Vec::new(),
                    status: PlaceStatus::Partial,
                    key,
                })
            }
            "interpreted_string_literal"
            | "raw_string_literal"
            | "int_literal"
            | "float_literal"
            | "rune_literal" => None,
            _ => self.lower_expression_children(
                interner,
                node,
                places,
                operations,
                unsupported,
                assignment_destination,
            ),
        }
    }

    fn lower_expression_children(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        node: Node<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
        assignment_destination: bool,
    ) -> Option<PlaceShape> {
        let mut last = None;
        for index in 0..node.named_child_count() as u32 {
            let Some(child) = node.named_child(index) else {
                continue;
            };
            last = self
                .lower_expression(
                    interner,
                    child,
                    places,
                    operations,
                    unsupported,
                    assignment_destination,
                )
                .or(last);
        }
        last
    }

    fn lower_identifier(
        &mut self,
        node: Node<'_>,
        places: &mut PlaceTableBuilder,
        assignment_destination: bool,
    ) -> Option<PlaceShape> {
        if is_selector_field(node) {
            return None;
        }
        let name = node_text(self.source, node)?.to_string();
        let (root, status) = if let Some(root) = self.locals.get(&name) {
            (root.clone(), PlaceStatus::Resolved)
        } else if let Some(root) = self.parameters.get(&name) {
            (root.clone(), PlaceStatus::Resolved)
        } else if assignment_destination {
            (
                PlaceRoot::Global { symbol: None, name },
                PlaceStatus::Partial,
            )
        } else {
            (PlaceRoot::Unknown { evidence: name }, PlaceStatus::Unknown)
        };
        let shape = PlaceShape {
            root,
            projections: Vec::new(),
            status,
            key: String::new(),
        };
        let key = self.insert_shape(places, &shape);
        let shape = PlaceShape { key, ..shape };
        Some(shape)
    }

    fn lower_selector(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        node: Node<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
        assignment_destination: bool,
    ) -> Option<PlaceShape> {
        let operand = node
            .child_by_field_name("operand")
            .or_else(|| node.named_child(0))?;
        let field = node
            .child_by_field_name("field")
            .or_else(|| node.named_child(1))
            .and_then(|field| node_text(self.source, field))?;
        let mut shape = self.lower_expression(
            interner,
            operand,
            places,
            operations,
            unsupported,
            assignment_destination,
        )?;
        shape
            .projections
            .push(PlaceProjection::Field(field.to_string()));
        shape.key = self.insert_shape(places, &shape);
        self.insert_temporary(places, node, PlaceStatus::Partial);
        Some(shape)
    }

    fn lower_index(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        node: Node<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
        assignment_destination: bool,
    ) -> Option<PlaceShape> {
        let operand = node
            .child_by_field_name("operand")
            .or_else(|| node.named_child(0))?;
        let index = node
            .child_by_field_name("index")
            .or_else(|| node.named_child(1))?;
        let mut shape = self.lower_expression(
            interner,
            operand,
            places,
            operations,
            unsupported,
            assignment_destination,
        )?;
        shape.projections.push(index_projection(self.source, index));
        shape.key = self.insert_shape(places, &shape);
        self.insert_temporary(places, node, PlaceStatus::Partial);
        Some(shape)
    }

    fn lower_call(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        node: Node<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) -> Option<String> {
        self.lower_unsupported_call(node, unsupported);
        let site = self.call_site_for(node);
        let return_key = self.insert_place(
            places,
            PlaceRoot::CallReturn { call: site },
            Vec::new(),
            PlaceStatus::Partial,
        );
        let callee_node = node
            .child_by_field_name("function")
            .or_else(|| node.named_child(0));
        let unwind = callee_node
            .and_then(|callee| node_text(self.source, callee))
            .is_some_and(|callee| callee == "panic");
        let callee = callee_node
            .and_then(|callee| node_text(self.source, callee))
            .map(|evidence| ValueDraft::Unknown {
                evidence: evidence.to_string(),
            })
            .unwrap_or_else(|| ValueDraft::Unknown {
                evidence: "call".to_string(),
            });
        let mut arguments = Vec::new();
        if let Some(argument_list) = node.child_by_field_name("arguments") {
            for index in 0..argument_list.named_child_count() as u32 {
                let Some(argument) = argument_list.named_child(index) else {
                    continue;
                };
                if let Some(shape) = self.lower_expression(
                    interner,
                    argument,
                    places,
                    operations,
                    unsupported,
                    false,
                ) {
                    arguments.push(shape.key);
                }
            }
        }
        self.push_operation(
            interner,
            operations,
            node,
            OperationKindDraft::Call {
                site,
                callee,
                arguments,
                return_place_key: return_key.clone(),
                unwind,
            },
            MirStatus::Partial,
        );
        Some(return_key)
    }

    fn lower_unsupported(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        node: Node<'_>,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) {
        let construct = match node.kind() {
            "go_statement" => Some("go_statement"),
            "defer_statement" => Some("defer_statement"),
            "select_statement" => Some("select_statement"),
            _ if node.is_error() || node.kind() == "ERROR" => Some("ERROR"),
            _ => None,
        };
        let Some(construct) = construct else {
            return;
        };
        self.push_unsupported(
            interner,
            operations,
            unsupported,
            node,
            construct,
            (
                unsupported_domains_for(construct),
                if construct == "ERROR" {
                    ConservativeAction::StopLowering
                } else {
                    ConservativeAction::HavocAffectedPlaces
                },
            ),
        );
    }

    fn lower_unsupported_call(&self, node: Node<'_>, unsupported: &mut Vec<UnsupportedDraft>) {
        let text = node_text(self.source, node).unwrap_or_default();
        let construct = if text.contains("panic(") {
            Some("panic")
        } else if text.contains("recover(") {
            Some("recover")
        } else if text.contains("unsafe.") {
            Some("unsafe")
        } else if text.contains("reflect.") {
            Some("reflect")
        } else {
            None
        };
        if let Some(construct) = construct {
            unsupported.push(UnsupportedDraft::new(UnsupportedDraftInput {
                id: UnsupportedId(unsupported.len() as u64),
                body: Some(self.body),
                operation: None,
                file_key: self.stable_context.file_key().to_string(),
                file: self.file,
                span: node_span(self.source_file, node),
                construct: construct.to_string(),
                source_evidence: text,
                affected_domains: unsupported_domains_for(construct),
                conservative_action: ConservativeAction::HavocAffectedPlaces,
            }));
        }
    }

    fn push_unsupported(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
        node: Node<'_>,
        construct: &str,
        pair: (Vec<UnsupportedDomain>, ConservativeAction),
    ) {
        let (domains, action) = pair;
        let unsupported_id = UnsupportedId(unsupported.len() as u64);
        let operation_id = MirOpId(operations.len() as u64);
        let span = node_span(self.source_file, node);
        unsupported.push(UnsupportedDraft::new(UnsupportedDraftInput {
            id: unsupported_id,
            body: Some(self.body),
            operation: Some(operation_id),
            file_key: self.stable_context.file_key().to_string(),
            file: self.file,
            span: span.clone(),
            construct: construct.to_string(),
            source_evidence: node_text(self.source, node).unwrap_or(construct),
            affected_domains: domains,
            conservative_action: action,
        }));
        operations.push(OperationDraft::new(
            interner,
            operation_id,
            self.body,
            self.stable_context.body_key(),
            self.ordinal_for(node),
            span,
            (
                OperationKindDraft::Unsupported { unsupported_id },
                MirStatus::Unsupported,
            ),
        ));
    }

    fn push_assign(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        operations: &mut Vec<OperationDraft>,
        node: Node<'_>,
        place_key: String,
        value: ValueDraft,
        mode: AssignMode,
    ) {
        self.push_operation(
            interner,
            operations,
            node,
            OperationKindDraft::Assign {
                place_key,
                value,
                mode,
            },
            MirStatus::Partial,
        );
    }

    fn push_branch(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        operations: &mut Vec<OperationDraft>,
        node: Node<'_>,
        predicate_place_key: Option<String>,
        nil_test: Option<BranchNilTest>,
    ) -> MirOpId {
        let id = MirOpId(operations.len() as u64);
        let shape = match node.kind() {
            "for_statement" => ControlShape::Loop,
            "expression_switch_statement" | "type_switch_statement" | "switch_statement" => {
                ControlShape::Switch
            }
            _ => ControlShape::Conditional,
        };
        self.push_operation(
            interner,
            operations,
            node,
            OperationKindDraft::Branch {
                predicate: MirPredicateId(self.ordinal_for(node) as u64),
                predicate_place_key,
                nil_test,
                shape,
                region: BranchRegion::default(),
            },
            MirStatus::Partial,
        );
        id
    }

    fn push_operation(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        operations: &mut Vec<OperationDraft>,
        node: Node<'_>,
        kind: OperationKindDraft,
        status: MirStatus,
    ) {
        let id = MirOpId(operations.len() as u64);
        operations.push(OperationDraft::new(
            interner,
            id,
            self.body,
            self.stable_context.body_key(),
            self.ordinal_for(node),
            node_span(self.source_file, node),
            (kind, status),
        ));
    }

    fn ordinal_for(&self, node: Node<'_>) -> u32 {
        node.start_byte() as u32
    }

    fn call_site_for(&self, node: Node<'_>) -> CallSiteId {
        CallSiteId(node.start_byte() as u64)
    }

    fn insert_temporary(
        &self,
        places: &mut PlaceTableBuilder,
        node: Node<'_>,
        status: PlaceStatus,
    ) -> String {
        self.insert_temporary_typed(
            places,
            node,
            status,
            TypeShape::Unknown {
                reason: "go temporary".to_string(),
            },
        )
    }

    fn insert_temporary_typed(
        &self,
        places: &mut PlaceTableBuilder,
        node: Node<'_>,
        status: PlaceStatus,
        ty: TypeShape,
    ) -> String {
        self.insert_typed_place(
            places,
            PlaceRoot::Temporary {
                body: self.body,
                ordinal: node.start_byte() as u32,
            },
            Vec::new(),
            Some(ty),
            status,
        )
    }

    fn insert_shape(&self, places: &mut PlaceTableBuilder, shape: &PlaceShape) -> String {
        self.insert_place(
            places,
            shape.root.clone(),
            shape.projections.clone(),
            shape.status,
        )
    }

    fn insert_place(
        &self,
        places: &mut PlaceTableBuilder,
        root: PlaceRoot,
        projections: Vec<PlaceProjection>,
        status: PlaceStatus,
    ) -> String {
        self.insert_typed_place(places, root, projections, None, status)
    }

    fn insert_typed_place(
        &self,
        places: &mut PlaceTableBuilder,
        root: PlaceRoot,
        projections: Vec<PlaceProjection>,
        ty: Option<TypeShape>,
        status: PlaceStatus,
    ) -> String {
        places.insert_typed_with_context(
            &self.stable_context,
            PlaceInsert {
                language: Language::Go,
                file: Some(self.file),
                function: Some(self.function),
                root,
                projections,
                status,
            },
            ty,
        )
    }
}

#[derive(Debug, Clone)]
struct PlaceShape {
    root: PlaceRoot,
    projections: Vec<PlaceProjection>,
    status: PlaceStatus,
    key: String,
}

#[derive(Debug, Clone)]
struct OperationDraft {
    id: MirOpId,
    body: MirBodyId,
    ordinal: u32,
    span: Span,
    kind: OperationKindDraft,
    stable_key: StableKeyId,
    status: MirStatus,
}

impl OperationDraft {
    fn new(
        interner: &crate::internal_core::StableKeyInterner,
        id: MirOpId,
        body: MirBodyId,
        body_stable_key: &str,
        ordinal: u32,
        span: Span,
        pair: (OperationKindDraft, MirStatus),
    ) -> Self {
        let (kind, status) = pair;
        let stable_key =
            interner.intern(operation_stable_key(body_stable_key, ordinal, &span, &kind));
        Self {
            id,
            body,
            ordinal,
            span,
            kind,
            stable_key,
            status,
        }
    }

    fn to_operation(&self, place_ids: &BTreeMap<String, PlaceId>) -> Option<MirOperation> {
        let kind = self.kind.to_kind(place_ids)?;
        Some(MirOperation {
            id: self.id,
            body: self.body,
            ordinal: self.ordinal,
            span: self.span.clone(),
            kind,
            stable_key: self.stable_key,
            status: self.status,
        })
    }

    fn to_control_effect(&self, place_ids: &BTreeMap<String, PlaceId>) -> Option<ControlEffect> {
        let kind = match &self.kind {
            OperationKindDraft::Suspend { kind, value } => ControlEffectKind::Suspend {
                kind: *kind,
                value: value.as_ref().map(|value| value.to_value(place_ids)),
            },
            _ => return None,
        };
        Some(ControlEffect {
            id: self.id,
            body: self.body,
            kind,
        })
    }
}

#[derive(Debug, Clone)]
enum OperationKindDraft {
    Assign {
        place_key: String,
        value: ValueDraft,
        mode: AssignMode,
    },
    Read {
        place_key: String,
    },
    Branch {
        predicate: MirPredicateId,
        predicate_place_key: Option<String>,
        nil_test: Option<BranchNilTest>,
        shape: ControlShape,
        region: BranchRegion,
    },
    Call {
        site: CallSiteId,
        callee: ValueDraft,
        arguments: Vec<String>,
        return_place_key: String,
        unwind: bool,
    },
    Return {
        value: Option<ValueDraft>,
    },
    Suspend {
        kind: SuspendKind,
        value: Option<ValueDraft>,
    },
    Unsupported {
        unsupported_id: UnsupportedId,
    },
}

impl OperationKindDraft {
    fn to_kind(&self, place_ids: &BTreeMap<String, PlaceId>) -> Option<MirOperationKind> {
        match self {
            Self::Assign {
                place_key,
                value,
                mode,
            } => Some(MirOperationKind::Assign {
                place: *place_ids.get(place_key)?,
                value: value.to_value(place_ids),
                mode: *mode,
            }),
            Self::Read { place_key } => Some(MirOperationKind::Read {
                place: *place_ids.get(place_key)?,
            }),
            Self::Branch {
                predicate,
                predicate_place_key,
                nil_test,
                ..
            } => Some(MirOperationKind::Branch {
                predicate: *predicate,
                predicate_place: predicate_place_key
                    .as_ref()
                    .and_then(|key| place_ids.get(key))
                    .copied(),
                nil_test: *nil_test,
            }),
            Self::Call {
                site,
                callee,
                arguments,
                return_place_key,
                ..
            } => {
                debug_assert!(
                    arguments.iter().all(|key| place_ids.contains_key(key)),
                    "MIR call argument place key missing from place table"
                );
                Some(MirOperationKind::Call {
                    site: *site,
                    callee: callee.to_value(place_ids),
                    arguments: arguments
                        .iter()
                        .map(|key| place_ids.get(key).copied())
                        .collect::<Option<Vec<_>>>()?,
                    return_place: *place_ids.get(return_place_key)?,
                })
            }
            Self::Return { value } => Some(MirOperationKind::Return {
                value: value.as_ref().map(|value| value.to_value(place_ids)),
            }),
            Self::Suspend { .. } => None,
            Self::Unsupported { unsupported_id } => Some(MirOperationKind::Unsupported {
                unsupported: *unsupported_id,
            }),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Assign { .. } => "assign",
            Self::Read { .. } => "read",
            Self::Branch { .. } => "branch",
            Self::Call { .. } => "call",
            Self::Return { .. } => "return",
            Self::Suspend { kind, .. } => match kind {
                SuspendKind::Await => "await",
                SuspendKind::Yield => "yield",
                SuspendKind::ChannelRecv => "channel-recv",
                SuspendKind::ChannelSend => "channel-send",
            },
            Self::Unsupported { .. } => "unsupported",
        }
    }

    fn place_keys(&self) -> Vec<String> {
        match self {
            Self::Assign {
                place_key, value, ..
            } => {
                let mut keys = vec![place_key.clone()];
                keys.extend(value.place_keys());
                keys
            }
            Self::Read { place_key } => vec![place_key.clone()],
            Self::Call {
                arguments,
                return_place_key,
                callee,
                ..
            } => {
                let mut keys = arguments.clone();
                keys.push(return_place_key.clone());
                keys.extend(callee.place_keys());
                keys
            }
            Self::Return { value } => value.as_ref().map_or_else(Vec::new, ValueDraft::place_keys),
            Self::Suspend { value, .. } => {
                value.as_ref().map_or_else(Vec::new, ValueDraft::place_keys)
            }
            Self::Branch { .. } | Self::Unsupported { .. } => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ValueDraft {
    Literal {
        value: String,
    },
    PlaceKey(String),
    BinOp {
        op: String,
        lhs: Box<ValueDraft>,
        rhs: Box<ValueDraft>,
    },
    Aggregate {
        kind: MirAggregateKind,
        fields: Vec<(Option<String>, ValueDraft)>,
    },
    Closure {
        body: MirBodyId,
        capture_keys: Vec<String>,
    },
    Unknown {
        evidence: String,
    },
}

impl ValueDraft {
    fn to_value(&self, place_ids: &BTreeMap<String, PlaceId>) -> MirValue {
        match self {
            Self::Literal { value } if value.trim().is_empty() => MirValue::Unknown {
                evidence: "empty literal lowering".to_string(),
            },
            Self::Literal { value } => MirValue::Literal {
                value: value.trim().to_string(),
            },
            Self::PlaceKey(key) => place_ids
                .get(key)
                .map(|id| MirValue::Place(*id))
                .unwrap_or_else(|| MirValue::Unknown {
                    evidence: key.clone(),
                }),
            Self::BinOp { op, lhs, rhs } => MirValue::BinOp {
                op: op.clone(),
                lhs: Box::new(lhs.to_value(place_ids)),
                rhs: Box::new(rhs.to_value(place_ids)),
            },
            Self::Aggregate { kind, fields } => MirValue::Aggregate {
                kind: *kind,
                fields: fields
                    .iter()
                    .map(|(name, value)| MirAggregateField {
                        name: name.clone(),
                        value: value.to_value(place_ids),
                    })
                    .collect(),
            },
            Self::Closure { body, capture_keys } => MirValue::Closure {
                body: *body,
                captures: capture_keys
                    .iter()
                    .filter_map(|key| place_ids.get(key).copied())
                    .collect(),
            },
            Self::Unknown { evidence } => MirValue::Unknown {
                evidence: evidence.clone(),
            },
        }
    }

    fn place_keys(&self) -> Vec<String> {
        match self {
            Self::PlaceKey(key) => vec![key.clone()],
            Self::BinOp { lhs, rhs, .. } => {
                let mut keys = lhs.place_keys();
                keys.extend(rhs.place_keys());
                keys
            }
            Self::Aggregate { fields, .. } => fields
                .iter()
                .flat_map(|(_, value)| value.place_keys())
                .collect(),
            Self::Closure { capture_keys, .. } => capture_keys.clone(),
            Self::Literal { .. } | Self::Unknown { .. } => Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct UnsupportedDraft {
    id: UnsupportedId,
    body: Option<MirBodyId>,
    operation: Option<MirOpId>,
    file_key: String,
    file: FileId,
    span: Span,
    construct: String,
    source_evidence: String,
    affected_place_keys: Vec<String>,
    affected_domains: Vec<UnsupportedDomain>,
    conservative_action: ConservativeAction,
}

struct UnsupportedDraftInput<S> {
    id: UnsupportedId,
    body: Option<MirBodyId>,
    operation: Option<MirOpId>,
    file_key: String,
    file: FileId,
    span: Span,
    construct: String,
    source_evidence: S,
    affected_domains: Vec<UnsupportedDomain>,
    conservative_action: ConservativeAction,
}

impl UnsupportedDraft {
    fn new<S>(input: UnsupportedDraftInput<S>) -> Self
    where
        S: AsRef<str>,
    {
        Self {
            id: input.id,
            body: input.body,
            operation: input.operation,
            file_key: input.file_key,
            file: input.file,
            span: input.span,
            construct: input.construct,
            source_evidence: input.source_evidence.as_ref().trim().to_string(),
            affected_place_keys: Vec::new(),
            affected_domains: input.affected_domains,
            conservative_action: input.conservative_action,
        }
    }

    fn to_fact(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        place_ids: &BTreeMap<String, PlaceId>,
    ) -> UnsupportedSemanticFact {
        UnsupportedSemanticFact {
            id: self.id,
            body: self.body,
            operation: self.operation,
            language: Language::Go,
            file: self.file,
            span: self.span.clone(),
            construct: self.construct.clone(),
            source_evidence: self.source_evidence.clone(),
            affected_places: {
                let affected_places = self
                    .affected_place_keys
                    .iter()
                    .map(|key| place_ids.get(key).copied())
                    .collect::<Option<Vec<_>>>();
                debug_assert!(
                    affected_places.is_some(),
                    "unsupported semantic affected place key missing from place table"
                );
                affected_places.unwrap_or_default()
            },
            affected_domains: self.affected_domains.clone(),
            conservative_action: self.conservative_action,
            precision: UnsupportedPrecision::Unsupported,
            status: MirStatus::Unsupported,
            stable_key: interner.intern(unsupported_stable_key(self)),
        }
    }
}

fn operation_stable_key(
    body_stable_key: &str,
    ordinal: u32,
    span: &Span,
    kind: &OperationKindDraft,
) -> String {
    let mut parts = vec![
        ("language", "go".to_string()),
        ("body", body_stable_key.to_string()),
        ("ordinal", ordinal.to_string()),
        ("kind", kind.label().to_string()),
        ("start_byte", span.start_byte.to_string()),
        ("end_byte", span.end_byte.to_string()),
    ];
    for (index, key) in kind.place_keys().into_iter().enumerate() {
        parts.push((operation_place_label(index), key));
    }
    let borrowed = parts
        .iter()
        .map(|(label, value)| (*label, value.clone()))
        .collect::<Vec<_>>();
    semantic_stable_key(FactFamily::MirOperation, &borrowed).into_string()
}

fn operation_place_label(index: usize) -> &'static str {
    match index {
        0 => "place_000000",
        1 => "place_000001",
        2 => "place_000002",
        3 => "place_000003",
        _ => "place_extra",
    }
}

fn unsupported_stable_key(draft: &UnsupportedDraft) -> String {
    semantic_stable_key(
        FactFamily::UnsupportedSemantic,
        &[
            ("language", "go".to_string()),
            ("file", draft.file_key.clone()),
            ("construct", draft.construct.clone()),
            ("start_byte", draft.span.start_byte.to_string()),
            ("end_byte", draft.span.end_byte.to_string()),
            ("evidence", draft.source_evidence.clone()),
        ],
    )
    .into_string()
}

fn unsupported_domains_for(construct: &str) -> Vec<UnsupportedDomain> {
    match construct {
        "go_statement" | "defer_statement" => vec![
            UnsupportedDomain::Mir,
            UnsupportedDomain::Cfg,
            UnsupportedDomain::Calls,
            UnsupportedDomain::DataFlow,
        ],
        "select_statement" => vec![
            UnsupportedDomain::Mir,
            UnsupportedDomain::Cfg,
            UnsupportedDomain::Domains,
            UnsupportedDomain::DataFlow,
        ],
        "panic" | "recover" => vec![
            UnsupportedDomain::Mir,
            UnsupportedDomain::Domains,
            UnsupportedDomain::DataFlow,
        ],
        "reflect" | "unsafe" => vec![
            UnsupportedDomain::Mir,
            UnsupportedDomain::Calls,
            UnsupportedDomain::Domains,
            UnsupportedDomain::Aliases,
            UnsupportedDomain::DataFlow,
        ],
        _ => vec![UnsupportedDomain::Mir, UnsupportedDomain::Cfg],
    }
}

fn matching_function<'db>(
    db: &'db impl AnalysisHost,
    file: FileId,
    name: &str,
    span: &Span,
) -> Option<&'db FunctionFact> {
    db.functions().iter().find(|function| {
        function.file == file
            && function.language == Language::Go
            && function.name == name
            && span_contains(span, &function.span)
    })
}

fn span_contains(outer: &Span, inner: &Span) -> bool {
    outer.start_byte <= inner.start_byte && outer.end_byte >= inner.end_byte
}

fn owner_stable_key(file: &SourceFile, function: &FunctionFact) -> String {
    semantic_stable_key(
        FactFamily::Function,
        &[
            ("language", "go".to_string()),
            ("path", file.relative_path.clone()),
            ("function", function.name.clone()),
            ("start_byte", function.span.start_byte.to_string()),
            ("end_byte", function.span.end_byte.to_string()),
        ],
    )
    .into_string()
}

fn node_span(file: &SourceFile, node: Node<'_>) -> Span {
    file.span_from_byte_range(node.start_byte(), node.end_byte())
}

fn node_text<'source>(source: &'source str, node: Node<'_>) -> Option<&'source str> {
    source.get(node.start_byte()..node.end_byte())
}

fn go_switch_discriminant(statement: Node<'_>) -> Option<Node<'_>> {
    match statement.kind() {
        "expression_switch_statement" | "type_switch_statement" | "switch_statement" => {
            statement.child_by_field_name("value")
        }
        _ => None,
    }
}

fn go_nil_test(source: &str, condition: Node<'_>) -> Option<BranchNilTest> {
    if condition.kind() != "binary_expression" {
        return None;
    }
    let left = condition
        .child_by_field_name("left")
        .or_else(|| condition.named_child(0))?;
    let right = condition
        .child_by_field_name("right")
        .or_else(|| condition.named_child(1))?;
    if node_text(source, left).map(str::trim) != Some("nil")
        && node_text(source, right).map(str::trim) != Some("nil")
    {
        return None;
    }
    // Go has a single nil value, so both edges of `== nil` / `!= nil` carry a
    // conclusion.
    let operator = go_binary_operator_text(source, left, right);
    if operator == "!=" {
        Some(BranchNilTest::Exhaustive { nil_on_true: false })
    } else if operator == "==" {
        Some(BranchNilTest::Exhaustive { nil_on_true: true })
    } else {
        None
    }
}

fn go_nil_operand<'tree>(source: &str, condition: Node<'tree>) -> Option<Node<'tree>> {
    if condition.kind() != "binary_expression" {
        return None;
    }
    let left = condition
        .child_by_field_name("left")
        .or_else(|| condition.named_child(0))?;
    let right = condition
        .child_by_field_name("right")
        .or_else(|| condition.named_child(1))?;
    if node_text(source, left).map(str::trim) == Some("nil") {
        Some(right)
    } else if node_text(source, right).map(str::trim) == Some("nil") {
        Some(left)
    } else {
        None
    }
}

fn go_binary_operator_text(source: &str, left: Node<'_>, right: Node<'_>) -> String {
    source
        .get(left.end_byte()..right.start_byte())
        .map(str::trim)
        .filter(|operator| !operator.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn type_shape_for_go_expression(node: Node<'_>) -> TypeShape {
    if node.kind() == "expression_list"
        && node.named_child_count() == 1
        && let Some(expression) = node.named_child(0)
    {
        return type_shape_for_go_expression(expression);
    }
    match node.kind() {
        "interpreted_string_literal" | "raw_string_literal" => {
            TypeShape::Primitive("string".to_string())
        }
        "int_literal" => TypeShape::Primitive("int".to_string()),
        "float_literal" => TypeShape::Primitive("float64".to_string()),
        "rune_literal" => TypeShape::Primitive("rune".to_string()),
        "true" | "false" => TypeShape::Primitive("bool".to_string()),
        "nil" => TypeShape::Nullish("nil".to_string()),
        "composite_literal" => TypeShape::Object { shape_id: None },
        "func_literal" => TypeShape::Callable {
            signature: "closure".to_string(),
        },
        _ => TypeShape::Unknown {
            reason: "go expression".to_string(),
        },
    }
}

fn function_name(source: &str, node: Node<'_>) -> Option<String> {
    let simple_name = declaration_name(source, node)?;
    if node.kind() == "method_declaration" {
        receiver_type_name(source, node)
            .map(|receiver| format!("{receiver}.{simple_name}"))
            .or(Some(simple_name))
    } else {
        Some(simple_name)
    }
}

fn declaration_name(source: &str, node: Node<'_>) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|name| node_text(source, name))
        .map(str::to_string)
}

fn receiver_type_name(source: &str, node: Node<'_>) -> Option<String> {
    let receiver = node.child_by_field_name("receiver")?;
    let receiver_text = node_text(source, receiver)?;
    let inner = receiver_text
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();
    let raw_type = inner.split_whitespace().last()?.trim_start_matches('*');
    if raw_type.is_empty() {
        None
    } else {
        Some(raw_type.to_string())
    }
}

fn parameter_names(source: &str, parameter_list: Node<'_>) -> Vec<String> {
    let mut names = Vec::new();
    for index in 0..parameter_list.named_child_count() as u32 {
        let Some(parameter) = parameter_list.named_child(index) else {
            continue;
        };
        if parameter.kind() != "parameter_declaration" {
            continue;
        }
        for child_index in 0..parameter.named_child_count() as u32 {
            let Some(child) = parameter.named_child(child_index) else {
                continue;
            };
            if matches!(child.kind(), "identifier" | "field_identifier")
                && let Some(name) = node_text(source, child)
            {
                names.push(name.to_string());
            }
        }
    }
    names
}

fn assignment_operator<'source>(source: &'source str, statement: Node<'_>) -> Option<&'source str> {
    statement
        .child_by_field_name("operator")
        .and_then(|operator| node_text(source, operator))
}

fn assignment_left_names(source: &str, statement: Node<'_>) -> Vec<String> {
    if let Some(left) = statement.child_by_field_name("left") {
        let names = direct_identifier_names(source, left);
        if !names.is_empty() {
            return names;
        }
    }

    let Some(text) = node_text(source, statement) else {
        return Vec::new();
    };
    let Some((left, _)) = assignment_delimiters()
        .iter()
        .find_map(|delimiter| text.split_once(delimiter))
    else {
        return Vec::new();
    };
    left.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .filter(|part| {
            part.chars()
                .all(|character| character == '_' || character.is_ascii_alphanumeric())
        })
        .filter(|part| *part != "_")
        .map(str::to_string)
        .collect()
}

fn var_spec_names(source: &str, node: Node<'_>) -> Vec<String> {
    let mut cursor = node.walk();
    node.children_by_field_name("name", &mut cursor)
        .filter_map(|name| node_text(source, name))
        .filter(|name| !name.trim().is_empty() && *name != "_")
        .map(str::to_string)
        .collect()
}

fn direct_identifier_names(source: &str, node: Node<'_>) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "identifier"
            && let Some(name) = node_text(source, child)
            && name != "_"
        {
            names.push(name.to_string());
        }
    }
    names
}

fn assignment_delimiters() -> &'static [&'static str] {
    &[
        "&^=", "<<=", ">>=", ":=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "=",
    ]
}

fn index_projection(source: &str, node: Node<'_>) -> PlaceProjection {
    let evidence = node_text(source, node)
        .unwrap_or("unknown")
        .trim()
        .to_string();
    if matches!(
        node.kind(),
        "interpreted_string_literal" | "raw_string_literal" | "int_literal"
    ) {
        PlaceProjection::IndexKnown(evidence.trim_matches(['"', '`']).to_string())
    } else {
        PlaceProjection::IndexUnknown { evidence }
    }
}

fn is_selector_field(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        parent.kind() == "selector_expression"
            && parent
                .child_by_field_name("field")
                .is_some_and(|field| field == node)
    })
}

fn visit_named_descendants<'tree, F>(node: Node<'tree>, visit: &mut F)
where
    F: FnMut(Node<'tree>),
{
    visit(node);
    for index in 0..node.named_child_count() as u32 {
        let Some(child) = node.named_child(index) else {
            continue;
        };
        visit_named_descendants(child, visit);
    }
}

#[cfg(test)]
mod places {
    use super::*;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::places::{PlaceProjection, PlaceRoot};
    use crate::internal_core::Language;
    use std::path::PathBuf;

    fn lower(source: &str) -> (MirOutput, crate::internal_core::StableKeyInterner) {
        let mut db = LocalAnalysisDb::new();
        db.add_file(
            PathBuf::from("auth.go"),
            "auth.go".to_string(),
            source.to_string(),
        );
        let cache = crate::analysis_api::DisabledAnalysisCache;
        let diagnostics = crate::go::analyze_with_options(&mut db, &cache, "", "", false);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let output = lower_go_mir(&db);
        (output, db.stable_key_interner())
    }

    #[test]
    fn go_function_places_include_parameters_locals_globals_and_projections() {
        let (first, first_interner) = lower(
            r#"
package auth

type User struct { Tokens []string }

func authorize(user User, index int) bool {
    token := user.Tokens[index]
    global = token
    return token != ""
}
"#,
        );
        let (second, second_interner) = lower(
            r#"
package auth

type User struct { Tokens []string }

func authorize(user User, index int) bool {
    token := user.Tokens[index]
    global = token
    return token != ""
}
"#,
        );

        assert_eq!(first.bodies.len(), 1);
        assert!(
            first_interner
                .resolve(first.bodies[0].stable_key)
                .contains("authorize")
        );
        assert_eq!(
            first
                .places
                .iter()
                .map(|place| first_interner.resolve(place.stable_key).to_string())
                .collect::<Vec<_>>(),
            second
                .places
                .iter()
                .map(|place| second_interner.resolve(place.stable_key).to_string())
                .collect::<Vec<_>>()
        );

        assert!(first.places.iter().any(|place| matches!(
            &place.root,
            PlaceRoot::Parameter {
                index: 0,
                name: Some(name),
                ..
            } if name == "user"
        )));
        assert!(first.places.iter().any(|place| matches!(
            &place.root,
            PlaceRoot::Parameter {
                index: 1,
                name: Some(name),
                ..
            } if name == "index"
        )));
        assert!(first.places.iter().any(|place| matches!(
            &place.root,
            PlaceRoot::Local { name, .. } if name == "token"
        )));
        assert!(first.places.iter().any(|place| matches!(
            &place.root,
            PlaceRoot::Global { name, .. } if name == "global"
        )));
        assert!(first.places.iter().any(|place| {
            matches!(&place.root, PlaceRoot::Parameter { name: Some(name), .. } if name == "user")
                && place
                    .projections
                    .contains(&PlaceProjection::Field("Tokens".to_string()))
                && place.projections.iter().any(|projection| {
                    matches!(projection, PlaceProjection::IndexUnknown { evidence } if evidence == "index")
                })
            }));
    }

    #[test]
    fn go_literal_values_still_traverse_nested_expression_places() {
        let (output, _) = lower(
            r#"
package auth

type User struct { Tokens []string }

func authorize(user User, index int) func() string {
    wrapped := User{Token: user.Tokens[index]}
    callback := func() string { return user.Tokens[index] }
    _ = wrapped
    return callback
}
"#,
        );

        assert!(output.places.iter().any(|place| {
            matches!(&place.root, PlaceRoot::Parameter { name: Some(name), .. } if name == "user")
                && place
                    .projections
                    .contains(&PlaceProjection::Field("Tokens".to_string()))
                && place.projections.iter().any(|projection| {
                    matches!(projection, PlaceProjection::IndexUnknown { evidence } if evidence == "index")
                })
        }));
    }

    #[test]
    fn go_lowerer_constructs_structured_values_closure_captures_and_place_types() {
        let (output, _) = lower(
            r#"
package auth

type Pair struct { Value int }

func make(seed int) func() int {
    count := 1
    callback := func() int { return seed + count }
    record := Pair{Value: count}
    _ = record
    return callback
}
"#,
        );

        let closure = output
            .operations
            .iter()
            .find_map(|operation| match &operation.kind {
                MirOperationKind::Assign {
                    value: MirValue::Closure { body, captures },
                    ..
                } => Some((*body, captures)),
                _ => None,
            });
        let (closure_body, captures) =
            closure.unwrap_or_else(|| panic!("closure assignment missing: {output:#?}"));
        assert!(output.bodies.iter().any(|body| body.id == closure_body));
        assert_eq!(captures.len(), 2);
        assert!(output.operations.iter().any(|operation| matches!(
            operation.kind,
            MirOperationKind::Return {
                value: Some(MirValue::BinOp { .. })
            }
        )));
        assert!(output.operations.iter().any(|operation| matches!(
            operation.kind,
            MirOperationKind::Assign {
                value: MirValue::Aggregate {
                    kind: MirAggregateKind::Composite,
                    ..
                },
                ..
            }
        )));
        assert!(
            output
                .place_types
                .iter()
                .any(|fact| matches!(fact.ty, TypeShape::Callable { .. }))
        );
    }

    #[test]
    fn go_method_receiver_is_parameter_zero_and_function_name_contract_is_preserved() {
        let mut db = LocalAnalysisDb::new();
        db.add_file(
            PathBuf::from("service.go"),
            "service.go".to_string(),
            r#"
package auth

type Service struct { cache map[string]string }

func (svc *Service) authorize(user User) bool {
    token := svc.cache[user.Name]
    return token != ""
}
"#
            .to_string(),
        );
        let cache = crate::analysis_api::DisabledAnalysisCache;
        let diagnostics = crate::go::analyze_with_options(&mut db, &cache, "", "", false);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        let function = db
            .functions()
            .iter()
            .find(|function| function.name == "Service.authorize")
            .expect("method fact should retain existing receiver-qualified name");
        assert_eq!(function.id, FunctionId::from_raw(0));
        assert_eq!(function.language, Language::Go);

        let output = lower_go_mir(&db);
        assert!(output.places.iter().any(|place| matches!(
            &place.root,
            PlaceRoot::Parameter {
                function,
                index: 0,
                name: Some(name),
            } if *function == FunctionId::from_raw(0) && name == "svc"
        )));
        assert!(
            db.resolve_stable_key(output.bodies[0].owner_stable_key)
                .contains("Service.authorize")
        );
    }

    #[test]
    fn go_mir_place_rows_do_not_carry_parser_node_debug_evidence() {
        let (output, _) = lower(
            r#"
package auth

func authorize(user User) bool {
    token := user.Token
    return token != ""
}
"#,
        );
        let debug = format!("{output:#?}");

        assert!(!debug.contains("tree_sitter::Node"));
        assert!(!debug.contains("Node<'_"));
        assert!(!debug.contains("function_declaration"));
        assert!(!debug.contains("method_declaration"));
    }
}

#[cfg(test)]
mod operations {
    use super::*;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::mir_op::{
        AssignMode, ConservativeAction, MirOperationKind, UnsupportedDomain,
    };
    use std::path::PathBuf;

    fn lower(source: &str) -> (MirOutput, crate::internal_core::StableKeyInterner) {
        let mut db = LocalAnalysisDb::new();
        db.add_file(
            PathBuf::from("flow.go"),
            "flow.go".to_string(),
            source.to_string(),
        );
        let cache = crate::analysis_api::DisabledAnalysisCache;
        let diagnostics = crate::go::analyze_with_options(&mut db, &cache, "", "", false);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let output = lower_go_mir(&db);
        (output, db.stable_key_interner())
    }

    #[test]
    fn go_statement_lowering_emits_assignment_modes_and_control_shapes() {
        let (output, _) = lower(
            r#"
package auth

type User struct { Tokens []string }

func flow(user User, index int) bool {
    var count int
    token := user.Tokens[index]
    a, b = b, a
    user.Tokens[index] = token
    count = index
    if token != "" { count = count + 1 }
    for count < 10 { count = count + 1 }
    switch token { case "": return false; default: count = count + 1 }
    return token != ""
}
"#,
        );

        let modes = output
            .operations
            .iter()
            .filter_map(|operation| match operation.kind {
                MirOperationKind::Assign { mode, .. } => Some(mode),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(modes.contains(&AssignMode::DeclarationBinding));
        assert!(modes.contains(&AssignMode::Overwrite));
        assert!(modes.contains(&AssignMode::ProjectionMutation));
        assert!(modes.contains(&AssignMode::Simultaneous));
        assert!(
            output
                .operations
                .iter()
                .any(|operation| matches!(operation.kind, MirOperationKind::Branch { .. }))
        );
        assert!(
            output
                .operations
                .iter()
                .any(|operation| matches!(operation.kind, MirOperationKind::Return { .. }))
        );
        assert!(!output.blocks.is_empty());
        assert_eq!(output.blocks.len(), output.terminators.len());
        assert!(
            output
                .terminators
                .iter()
                .any(|terminator| matches!(terminator.kind, MirTerminatorKind::Branch { .. }))
        );
        assert!(
            output
                .terminators
                .iter()
                .any(|terminator| matches!(terminator.kind, MirTerminatorKind::Goto { .. }))
        );
        assert!(
            output
                .terminators
                .iter()
                .any(|terminator| matches!(terminator.kind, MirTerminatorKind::Switch { .. }))
        );
        assert!(
            output
                .terminators
                .iter()
                .any(|terminator| matches!(terminator.kind, MirTerminatorKind::Return { .. }))
        );
    }

    #[test]
    fn go_branch_operations_bind_if_and_switch_condition_places() {
        let (output, interner) = lower(
            r#"
package auth

func flow(left bool, right bool, kind int) {
    if left && right {}
    switch kind { case 1: return; default: return }
    for left || right { return }
}
"#,
        );

        let predicates = output
            .operations
            .iter()
            .filter_map(|operation| match operation.kind {
                MirOperationKind::Branch {
                    predicate_place,
                    nil_test,
                    ..
                } => Some((predicate_place, nil_test)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(predicates.len(), 3, "{output:#?}");

        let key_for = |predicate| {
            let place = output
                .places
                .iter()
                .find(|place| place.id == predicate)
                .expect("predicate place should survive remapping");
            interner.resolve(place.stable_key).to_string()
        };

        // The `if` binds the last place its condition reads, the `switch` binds
        // its discriminant, and the loop stays unbound: a loop body is lowered
        // into the block the branch reaches on its `false` edge, so a bound
        // condition would refine the body with the loop-exit assumption.
        let bound = predicates
            .iter()
            .filter_map(|(predicate, _)| predicate.map(key_for))
            .collect::<Vec<_>>();
        assert_eq!(bound.len(), 2, "{predicates:?}");
        assert!(
            bound.iter().any(|key| key.ends_with("root_name=5:right")),
            "{bound:?}"
        );
        assert!(
            bound.iter().any(|key| key.ends_with("root_name=4:kind")),
            "{bound:?}"
        );
    }

    #[test]
    fn go_loop_branches_carry_no_condition_assumption() {
        let (output, _) = lower(
            r#"
package auth

type User struct{}

func flow(user *User, items []string, ready bool) {
    for user != nil { return }
    for index := 0; index < len(items); index++ { return }
    for ready { return }
    for range items { return }
}
"#,
        );

        // Every branch in this body is a loop branch.
        let branches = output
            .operations
            .iter()
            .filter_map(|operation| match operation.kind {
                MirOperationKind::Branch {
                    predicate_place,
                    nil_test,
                    ..
                } => Some((predicate_place, nil_test)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(branches.len(), 4, "{output:#?}");
        assert!(
            branches
                .iter()
                .all(|(place, nil_test)| place.is_none() && nil_test.is_none()),
            "{branches:?}"
        );
    }

    #[test]
    fn go_declarations_and_compound_assignments_keep_all_mutated_places() {
        let (output, _) = lower(
            r#"
package auth

func flow(delta int) int {
    var count, limit int
    count += delta
    limit = count
    return limit
}
"#,
        );

        assert!(output.places.iter().any(|place| matches!(
            &place.root,
            PlaceRoot::Local { name, .. } if name == "count"
        )));
        assert!(output.places.iter().any(|place| matches!(
            &place.root,
            PlaceRoot::Local { name, .. } if name == "limit"
        )));
        assert!(output.operations.iter().any(|operation| matches!(
            operation.kind,
            MirOperationKind::Assign {
                mode: AssignMode::PartialWrite,
                ..
            }
        )));
    }

    #[test]
    fn go_call_operations_are_shape_evidence_with_deterministic_call_sites() {
        let source = r#"
package auth

func flow(token string, count int) bool {
    result := helper(token, count)
    return result
}
"#;
        let (first, first_interner) = lower(source);
        let (second, second_interner) = lower(source);

        let first_calls = first
            .operations
            .iter()
            .filter_map(|operation| match &operation.kind {
                MirOperationKind::Call {
                    site,
                    callee,
                    arguments,
                    return_place,
                } => Some((
                    first_interner.resolve(operation.stable_key).to_string(),
                    *site,
                    callee,
                    arguments,
                    return_place,
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        let second_calls = second
            .operations
            .iter()
            .filter_map(|operation| match &operation.kind {
                MirOperationKind::Call {
                    site,
                    arguments,
                    return_place,
                    ..
                } => Some((
                    second_interner.resolve(operation.stable_key).to_string(),
                    *site,
                    arguments.len(),
                    *return_place,
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(first_calls.len(), 1);
        assert_eq!(first_calls[0].3.len(), 2);
        assert_eq!(
            first_calls
                .iter()
                .map(|(key, site, _, arguments, return_place)| {
                    (key.clone(), *site, arguments.len(), **return_place)
                })
                .collect::<Vec<_>>(),
            second_calls
        );
        assert!(
            first_calls[0].2
                != &crate::analysis_neutral::mir_op::MirValue::Unknown {
                    evidence: "direct target".to_string()
                }
        );
    }

    #[test]
    fn go_unsupported_semantics_are_structured_and_conservative() {
        let (output, _) = lower(
            r#"
package auth

func flow(ch chan int, token string, count int) bool {
    go helper(token)
    defer helper(token)
    select {
    case ch <- count:
    case value := <-ch:
        count = value
    }
    reflect.ValueOf(token)
    unsafe.Sizeof(count)
    panic(token)
    recover()
    return token != ""
}
"#,
        );

        for construct in [
            "go_statement",
            "defer_statement",
            "select_statement",
            "reflect",
            "unsafe",
            "panic",
            "recover",
        ] {
            let row = output
                .unsupported
                .iter()
                .find(|row| row.construct == construct)
                .unwrap_or_else(|| panic!("missing unsupported row: {construct}"));
            assert!(row.is_complete());
            assert!(row.affected_domains.contains(&UnsupportedDomain::Mir));
            assert!(matches!(
                row.conservative_action,
                ConservativeAction::HavocAffectedPlaces | ConservativeAction::StopLowering
            ));
        }
        assert!(output.terminators.iter().any(|terminator| matches!(
            terminator.kind,
            MirTerminatorKind::Suspend {
                kind: SuspendKind::ChannelSend,
                ..
            }
        )));
        assert!(output.terminators.iter().any(|terminator| matches!(
            terminator.kind,
            MirTerminatorKind::Suspend {
                kind: SuspendKind::ChannelRecv,
                ..
            }
        )));
        assert!(output.terminators.iter().any(|terminator| matches!(
            terminator.kind,
            MirTerminatorKind::Call {
                unwind: Some(_),
                ..
            }
        )));
    }
}
