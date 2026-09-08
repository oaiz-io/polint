use std::collections::{BTreeMap, BTreeSet};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, BinaryOperator, BindingPattern, Class, ClassElement, Declaration,
    ExportDefaultDeclarationKind, Expression, FormalParameters, Function, FunctionBody,
    LogicalOperator, MethodDefinition, ObjectPropertyKind, Program, PropertyKey, Statement,
    VariableDeclarator,
};
use oxc_span::GetSpan;

use crate::analysis_api::{
    FactFamily, FunctionFact, SourceFile, is_synthetic_ts_js_module_function,
};
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
use crate::ts::{
    PARSER_RECOVERY_CONSTRUCT, anonymous_callable_name, class_callable_name, parse_ts_file,
    spans::{
        normalized_call_expression_span, normalized_new_expression_span,
        normalized_tagged_template_span,
    },
};

#[doc(hidden)]
pub fn lower_ts_mir(db: &impl AnalysisHost) -> MirOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut lowering = TsMirLowering::default();
    let mut files = db
        .files()
        .iter()
        .filter(|file| file.language.is_ts_family())
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
                    ControlEffectKind::Throw { value } => {
                        let unwind = push_block_draft(&mut drafts, blocks.len(), body.id);
                        drafts[unwind].terminator = Some(MirTerminatorKind::Unreachable);
                        drafts[current].terminator = Some(MirTerminatorKind::Throw {
                            value: value.clone(),
                            unwind: drafts[unwind].id,
                        });
                        if step_index + 1 < step_count {
                            current = push_block_draft(&mut drafts, blocks.len(), body.id);
                            drafts[current].terminator = Some(MirTerminatorKind::Unreachable);
                        }
                    }
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
    Throw {
        value: Option<MirValue>,
    },
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

fn set_branch_predicate(
    operations: &mut [OperationDraft],
    branch: MirOpId,
    predicate_place_key: Option<String>,
    nil_test: Option<BranchNilTest>,
) {
    let operation = operations
        .iter_mut()
        .find(|operation| operation.id == branch)
        .expect("branch operation should exist");
    let OperationKindDraft::Branch {
        predicate_place_key: stored_place,
        nil_test: stored_nil_test,
        ..
    } = &mut operation.kind
    else {
        panic!("branch operation id should identify a branch");
    };
    *stored_place = predicate_place_key;
    *stored_nil_test = nil_test;
}

#[derive(Debug, Default)]
struct TsMirLowering {
    bodies: Vec<MirBody>,
    places: PlaceTableBuilder,
    operations: Vec<OperationDraft>,
    unsupported: Vec<UnsupportedDraft>,
}

impl TsMirLowering {
    fn lower_file(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        db: &impl AnalysisHost,
        file: &SourceFile,
    ) {
        let allocator = Allocator::default();
        let parsed = parse_ts_file(&allocator, file);
        for error in &parsed.errors {
            let span = match (error.start_byte, error.end_byte) {
                (Some(start), Some(end)) => file.span_from_byte_range(start, end),
                _ => Span::point(file.id, 1, 1),
            };
            self.unsupported
                .push(UnsupportedDraft::new(UnsupportedDraftInput {
                    id: UnsupportedId(self.unsupported.len() as u64),
                    body: None,
                    operation: None,
                    language: file.language,
                    file_key: file.relative_path.clone(),
                    file: file.id,
                    span,
                    construct: PARSER_RECOVERY_CONSTRUCT.to_string(),
                    source_evidence: error.message.clone(),
                    affected_place_keys: Vec::new(),
                    affected_domains: vec![
                        UnsupportedDomain::Mir,
                        UnsupportedDomain::Cfg,
                        UnsupportedDomain::Calls,
                        UnsupportedDomain::Domains,
                        UnsupportedDomain::DataFlow,
                        UnsupportedDomain::Aliases,
                        UnsupportedDomain::Summaries,
                    ],
                    conservative_action: ConservativeAction::StopLowering,
                }));
        }
        let mut functions = Vec::new();
        collect_functions(file.source.as_ref(), parsed.program(), &mut functions);
        functions.sort_by(|left, right| {
            (
                file.relative_path.as_str(),
                left.span.start,
                left.span.end,
                left.name.as_str(),
            )
                .cmp(&(
                    file.relative_path.as_str(),
                    right.span.start,
                    right.span.end,
                    right.name.as_str(),
                ))
        });
        // A class expression's methods can be collected via both the declaration
        // path and the anonymous-callable walk; drop adjacent duplicates so each
        // method is lowered once (a duplicate MIR body would duplicate call sites).
        functions.dedup_by(|left, right| left.span == right.span && left.name == right.name);

        let mut prepared = Vec::new();
        for function in functions {
            let span = span_from_oxc(file, function.span);
            let Some(function_fact) =
                matching_function(db, file.id, file.language, &function.name, &span)
                    .or_else(|| enclosing_function(db, file.id, file.language, &span))
            else {
                continue;
            };
            let body = self.push_body(interner, db, file, function_fact, span);
            prepared.push((function, function_fact.id, body));
        }
        let closure_bodies = prepared
            .iter()
            .map(|(function, _, body)| ((function.span.start, function.span.end), body.id))
            .collect::<BTreeMap<_, _>>();
        let closure_capture_names =
            closure_capture_names(db, file.id, file.source.as_ref(), &prepared);

        if let Some(module_function) = matching_module_function(db, file.id, file.language) {
            let span = file.span_from_byte_range(0, file.source.len());
            let body = self.push_body(interner, db, file, module_function, span);
            let mut module_lowering = FunctionLowering::new(
                interner,
                file,
                file.source.as_ref(),
                module_function.id,
                &body,
                closure_bodies.clone(),
                closure_capture_names.clone(),
            );
            module_lowering.lower_statements(
                interner,
                &parsed.program().body,
                &mut self.places,
                &mut self.operations,
                &mut self.unsupported,
            );
        }

        for (function, function_id, body) in prepared {
            let mut function_lowering = FunctionLowering::new(
                interner,
                file,
                file.source.as_ref(),
                function_id,
                &body,
                closure_bodies.clone(),
                closure_capture_names.clone(),
            );
            function_lowering.lower_parameters(&function.parameters, &mut self.places);
            match function.body {
                CandidateBody::Statements(statements) => function_lowering.lower_statements(
                    interner,
                    statements,
                    &mut self.places,
                    &mut self.operations,
                    &mut self.unsupported,
                ),
                CandidateBody::Expression(expression) => {
                    function_lowering.lower_expression(
                        interner,
                        expression,
                        &mut self.places,
                        &mut self.operations,
                        &mut self.unsupported,
                        false,
                    );
                }
                CandidateBody::ReturnExpression(expression) => {
                    let value = function_lowering.lower_value(
                        interner,
                        expression,
                        &mut self.places,
                        &mut self.operations,
                        &mut self.unsupported,
                    );
                    function_lowering.push_operation(
                        interner,
                        &mut self.operations,
                        expression.span(),
                        OperationKindDraft::Return { value },
                        MirStatus::Partial,
                    );
                }
            }
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
                ("language", language_label(file.language).to_string()),
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
            language: file.language,
            file: file.id,
            function: function.id,
            package: db
                .packages()
                .iter()
                .find(|package| package.file == file.id && package.language == file.language)
                .map(|package| package.id),
            module: db
                .module_nodes()
                .iter()
                .find(|module| {
                    module.file == Some(file.id) && module.language == Some(file.language)
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

#[derive(Debug)]
struct TsFunctionCandidate<'ast> {
    name: String,
    span: oxc_span::Span,
    parameters: Vec<String>,
    body: CandidateBody<'ast>,
}

fn closure_capture_names(
    db: &impl AnalysisHost,
    file: FileId,
    source: &str,
    functions: &[(TsFunctionCandidate<'_>, FunctionId, MirBody)],
) -> BTreeMap<(u32, u32), Vec<String>> {
    functions
        .iter()
        .map(|(function, _, _)| {
            let mut names = db
                .references_for_file(file)
                .into_iter()
                .filter(|reference| {
                    reference.primary_span.as_ref().is_some_and(|span| {
                        span.start_byte >= function.span.start && span.end_byte <= function.span.end
                    })
                })
                .filter_map(|reference| {
                    let target = reference.target?;
                    let definition = db.definition_for_symbol(target)?;
                    let definition_span = definition.primary_span.as_ref()?;
                    (definition_span.start_byte < function.span.start
                        || definition_span.end_byte > function.span.end)
                        .then(|| reference.name.clone())
                })
                .collect::<BTreeSet<_>>();
            if let Some(body_source) =
                source.get(function.span.start as usize..function.span.end as usize)
            {
                names.extend(identifier_tokens(body_source));
            }
            for parameter in &function.parameters {
                names.remove(parameter);
            }
            (
                (function.span.start, function.span.end),
                names.into_iter().collect(),
            )
        })
        .collect()
}

fn identifier_tokens(source: &str) -> impl Iterator<Item = String> + '_ {
    source
        .split(|character: char| {
            !(character == '_' || character == '$' || character.is_alphanumeric())
        })
        .filter(|token| {
            token.chars().next().is_some_and(|character| {
                character == '_' || character == '$' || character.is_alphabetic()
            })
        })
        .map(str::to_string)
}

/// What a [`TsFunctionCandidate`] lowers. Function/arrow bodies and class static
/// blocks are statement lists; class field initializers (`x = <expr>`) are a
/// single expression, lowered the same way an expression statement would be so
/// calls buried in the initializer (`f1()` in `x = (f1(), …)`, `super.m()` in
/// `x = super.m()`) get call sites attributed to the field-initializer function.
#[derive(Debug)]
enum CandidateBody<'ast> {
    Statements(&'ast [Statement<'ast>]),
    Expression(&'ast Expression<'ast>),
    ReturnExpression(&'ast Expression<'ast>),
}

fn arrow_candidate_body<'ast>(
    function: &'ast oxc_ast::ast::ArrowFunctionExpression<'ast>,
) -> CandidateBody<'ast> {
    function.get_expression().map_or_else(
        || CandidateBody::Statements(&function.body.statements),
        CandidateBody::ReturnExpression,
    )
}

fn collect_functions<'ast>(
    source: &str,
    program: &'ast Program<'ast>,
    functions: &mut Vec<TsFunctionCandidate<'ast>>,
) {
    for statement in &program.body {
        collect_statement_functions(source, statement, functions);
        collect_anonymous_functions_from_statement(statement, functions);
    }
}

fn collect_statement_functions<'ast>(
    source: &str,
    statement: &'ast Statement<'ast>,
    functions: &mut Vec<TsFunctionCandidate<'ast>>,
) {
    match statement {
        Statement::FunctionDeclaration(function) => {
            if let Some(name) = function.id.as_ref().map(|id| id.name.to_string()) {
                collect_function(name, function.span, function, functions);
            }
        }
        Statement::VariableDeclaration(variable) => {
            for declarator in &variable.declarations {
                collect_variable_function(declarator, functions);
            }
        }
        Statement::ClassDeclaration(class) => {
            if let Some(name) = class.id.as_ref().map(|id| id.name.to_string()) {
                collect_class_functions(&name, class, functions);
            }
        }
        Statement::ExportNamedDeclaration(export) => {
            if let Some(declaration) = &export.declaration {
                collect_declaration_functions(source, declaration, functions);
            }
        }
        Statement::ExportDefaultDeclaration(export) => match &export.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                if let Some(name) = function.id.as_ref().map(|id| id.name.to_string()) {
                    collect_function(name, function.span, function, functions);
                }
            }
            ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                if let Some(name) = class.id.as_ref().map(|id| id.name.to_string()) {
                    collect_class_functions(&name, class, functions);
                }
            }
            _ => {}
        },
        _ => {}
    }
}

fn collect_declaration_functions<'ast>(
    _source: &str,
    declaration: &'ast Declaration<'ast>,
    functions: &mut Vec<TsFunctionCandidate<'ast>>,
) {
    match declaration {
        Declaration::FunctionDeclaration(function) => {
            if let Some(name) = function.id.as_ref().map(|id| id.name.to_string()) {
                collect_function(name, function.span, function, functions);
            }
            if let Some(body) = function.body.as_deref() {
                collect_anonymous_functions_from_body(body, functions);
            }
        }
        Declaration::VariableDeclaration(variable) => {
            for declarator in &variable.declarations {
                collect_variable_function(declarator, functions);
            }
        }
        Declaration::ClassDeclaration(class) => {
            if let Some(name) = class.id.as_ref().map(|id| id.name.to_string()) {
                collect_class_functions(&name, class, functions);
            }
        }
        _ => {}
    }
}

fn collect_variable_function<'ast>(
    declarator: &'ast VariableDeclarator<'ast>,
    functions: &mut Vec<TsFunctionCandidate<'ast>>,
) {
    let BindingPattern::BindingIdentifier(name) = &declarator.id else {
        return;
    };
    let Some(init) = &declarator.init else {
        return;
    };
    match init {
        Expression::ArrowFunctionExpression(function) => {
            functions.push(TsFunctionCandidate {
                name: name.name.to_string(),
                span: function.span,
                parameters: parameter_names(&function.params),
                body: arrow_candidate_body(function),
            });
        }
        Expression::FunctionExpression(function) => {
            if let Some(body) = function.body.as_deref() {
                functions.push(TsFunctionCandidate {
                    name: name.name.to_string(),
                    span: function.span,
                    parameters: parameter_names(&function.params),
                    body: CandidateBody::Statements(&body.statements),
                });
            }
        }
        _ => {}
    }
}

fn collect_function<'ast>(
    name: String,
    span: oxc_span::Span,
    function: &'ast Function<'ast>,
    functions: &mut Vec<TsFunctionCandidate<'ast>>,
) {
    if let Some(body) = function.body.as_deref() {
        functions.push(TsFunctionCandidate {
            name,
            span,
            parameters: parameter_names(&function.params),
            body: CandidateBody::Statements(&body.statements),
        });
    }
}

fn collect_class_functions<'ast>(
    class_name: &str,
    class: &'ast Class<'ast>,
    functions: &mut Vec<TsFunctionCandidate<'ast>>,
) {
    for element in &class.body.body {
        match element {
            ClassElement::MethodDefinition(method) => {
                if let Some(method_name) = method_name(method) {
                    collect_function(
                        format!("{class_name}.{method_name}"),
                        method.span,
                        &method.value,
                        functions,
                    );
                }
            }
            // A class static block (`static { … }`) executes at class-definition
            // time and is its own function in Jelly's model. Lower its statement
            // list as a body (named by span, matching the frontend FunctionFact)
            // so direct calls inside it — `super.f()`, top-level `f1()` — get call
            // sites attributed to the static-block function.
            ClassElement::StaticBlock(block) => {
                functions.push(TsFunctionCandidate {
                    name: anonymous_callable_name(block.span.start, block.span.end),
                    span: block.span,
                    parameters: Vec::new(),
                    body: CandidateBody::Statements(&block.body),
                });
            }
            // A class field initializer (`x = <expr>`) is its own function in
            // Jelly's model: calls in the initializer (`f1()` in `x = (f1(), …)`,
            // `super.m()` in `x = super.m()`) are attributed to it. Lower the
            // initializer expression so those calls get call sites; the value-flow
            // walks the same initializer with `this`/`super` bound.
            ClassElement::PropertyDefinition(property) => {
                if let Some(value) = &property.value {
                    let span = property_init_function_span(property);
                    functions.push(TsFunctionCandidate {
                        name: anonymous_callable_name(span.start, span.end),
                        span,
                        parameters: Vec::new(),
                        body: CandidateBody::Expression(value),
                    });
                }
            }
            _ => {}
        }
    }
}

/// The span Jelly assigns to a class field-initializer function: from the
/// property key (after any `static` modifier) through the initializer value.
/// Shared by the frontend FunctionFact emission and MIR lowering so the
/// span-keyed fact and the MIR body always agree (and align with Jelly's oracle).
fn property_init_function_span(property: &oxc_ast::ast::PropertyDefinition<'_>) -> oxc_span::Span {
    // Start after any `static` modifier (at the key), end at the property
    // definition's end — which includes the trailing `;`, matching Jelly's
    // field-initializer function span (so the fun2fun mirror edge matches).
    oxc_span::Span::new(property.key.span().start, property.span.end)
}

fn collect_anonymous_functions_from_statement<'ast>(
    statement: &'ast Statement<'ast>,
    functions: &mut Vec<TsFunctionCandidate<'ast>>,
) {
    match statement {
        Statement::BlockStatement(block) => {
            for statement in &block.body {
                collect_anonymous_functions_from_statement(statement, functions);
            }
        }
        Statement::ExpressionStatement(statement) => {
            collect_anonymous_functions_from_expression(&statement.expression, true, functions);
        }
        Statement::VariableDeclaration(variable) => {
            for declarator in &variable.declarations {
                if let Some(init) = &declarator.init {
                    // `include_self = true` mirrors the frontend: a nested
                    // `const x = function(){}` init function gets its own MIR body
                    // + call sites. Deduped by (span, name) with the top-level
                    // `collect_variable_function` emission.
                    collect_anonymous_functions_from_expression(init, true, functions);
                }
            }
        }
        Statement::FunctionDeclaration(function) => {
            // Nested function declaration: register it as a candidate so its body
            // gets its own MirBody + Call operations (call sites are derived only
            // from MIR Call ops). This matches the frontend's nested-function
            // FunctionFact emission; without it the fact would exist but its body
            // would carry no call sites. Deduped by (span, name) by the caller.
            if let Some(name) = function.id.as_ref().map(|id| id.name.to_string()) {
                collect_function(name, function.span, function, functions);
            }
            if let Some(body) = function.body.as_deref() {
                collect_anonymous_functions_from_body(body, functions);
            }
        }
        Statement::ReturnStatement(statement) => {
            if let Some(argument) = &statement.argument {
                collect_anonymous_functions_from_expression(argument, true, functions);
            }
        }
        Statement::ThrowStatement(statement) => {
            collect_anonymous_functions_from_expression(&statement.argument, true, functions);
        }
        Statement::IfStatement(statement) => {
            collect_anonymous_functions_from_expression(&statement.test, true, functions);
            collect_anonymous_functions_from_statement(&statement.consequent, functions);
            if let Some(alternate) = &statement.alternate {
                collect_anonymous_functions_from_statement(alternate, functions);
            }
        }
        Statement::WhileStatement(statement) => {
            collect_anonymous_functions_from_expression(&statement.test, true, functions);
            collect_anonymous_functions_from_statement(&statement.body, functions);
        }
        Statement::DoWhileStatement(statement) => {
            collect_anonymous_functions_from_statement(&statement.body, functions);
            collect_anonymous_functions_from_expression(&statement.test, true, functions);
        }
        Statement::ForStatement(statement) => {
            if let Some(init) = &statement.init {
                match init {
                    oxc_ast::ast::ForStatementInit::VariableDeclaration(variable) => {
                        for declarator in &variable.declarations {
                            if let Some(init) = &declarator.init {
                                collect_anonymous_functions_from_expression(init, false, functions);
                            }
                        }
                    }
                    _ => collect_anonymous_functions_from_expression(
                        init.to_expression(),
                        true,
                        functions,
                    ),
                }
            }
            if let Some(test) = &statement.test {
                collect_anonymous_functions_from_expression(test, true, functions);
            }
            if let Some(update) = &statement.update {
                collect_anonymous_functions_from_expression(update, true, functions);
            }
            collect_anonymous_functions_from_statement(&statement.body, functions);
        }
        Statement::ForInStatement(statement) => {
            collect_anonymous_functions_from_expression(&statement.right, true, functions);
            collect_anonymous_functions_from_statement(&statement.body, functions);
        }
        Statement::ForOfStatement(statement) => {
            collect_anonymous_functions_from_expression(&statement.right, true, functions);
            collect_anonymous_functions_from_statement(&statement.body, functions);
        }
        Statement::SwitchStatement(statement) => {
            collect_anonymous_functions_from_expression(&statement.discriminant, true, functions);
            for case in &statement.cases {
                if let Some(test) = &case.test {
                    collect_anonymous_functions_from_expression(test, true, functions);
                }
                for statement in &case.consequent {
                    collect_anonymous_functions_from_statement(statement, functions);
                }
            }
        }
        Statement::ClassDeclaration(class) => {
            collect_anonymous_functions_from_class(class, functions)
        }
        _ => {}
    }
}

fn collect_anonymous_functions_from_class<'ast>(
    class: &'ast Class<'ast>,
    functions: &mut Vec<TsFunctionCandidate<'ast>>,
) {
    // Lower the class's own methods/constructor (matched to the class-expression
    // FunctionFacts emitted by the frontend) so their bodies get MIR call sites.
    // For top-level class declarations these are also emitted via the declaration
    // path; `collect_functions` dedups by (span, name). `class_callable_name` is
    // shared with the frontend so the names always agree.
    let class_name = class_callable_name(class);
    collect_class_functions(&class_name, class, functions);
    for element in &class.body.body {
        match element {
            ClassElement::MethodDefinition(method) => {
                if let Some(body) = method.value.body.as_deref() {
                    collect_anonymous_functions_from_body(body, functions);
                }
            }
            ClassElement::PropertyDefinition(property) => {
                if let Some(value) = &property.value {
                    collect_anonymous_functions_from_expression(value, true, functions);
                }
            }
            ClassElement::StaticBlock(block) => {
                for statement in &block.body {
                    collect_anonymous_functions_from_statement(statement, functions);
                }
            }
            _ => {}
        }
    }
}

fn collect_anonymous_functions_from_body<'ast>(
    body: &'ast FunctionBody<'ast>,
    functions: &mut Vec<TsFunctionCandidate<'ast>>,
) {
    for statement in &body.statements {
        collect_anonymous_functions_from_statement(statement, functions);
    }
}

fn collect_anonymous_functions_from_expression<'ast>(
    expression: &'ast Expression<'ast>,
    include_self: bool,
    functions: &mut Vec<TsFunctionCandidate<'ast>>,
) {
    match expression {
        Expression::ArrowFunctionExpression(function) => {
            if include_self {
                functions.push(TsFunctionCandidate {
                    name: anonymous_callable_name(function.span.start, function.span.end),
                    span: function.span,
                    parameters: parameter_names(&function.params),
                    body: arrow_candidate_body(function),
                });
            }
            collect_anonymous_functions_from_body(&function.body, functions);
        }
        Expression::FunctionExpression(function) => {
            if include_self && let Some(body) = function.body.as_deref() {
                functions.push(TsFunctionCandidate {
                    name: anonymous_callable_name(function.span.start, function.span.end),
                    span: function.span,
                    parameters: parameter_names(&function.params),
                    body: CandidateBody::Statements(&body.statements),
                });
            }
            if let Some(body) = function.body.as_deref() {
                collect_anonymous_functions_from_body(body, functions);
            }
        }
        Expression::CallExpression(call) => {
            collect_anonymous_functions_from_expression(&call.callee, true, functions);
            for argument in &call.arguments {
                collect_anonymous_functions_from_argument(argument, functions);
            }
        }
        Expression::NewExpression(expression) => {
            collect_anonymous_functions_from_expression(&expression.callee, true, functions);
            for argument in &expression.arguments {
                collect_anonymous_functions_from_argument(argument, functions);
            }
        }
        Expression::StaticMemberExpression(member) => {
            collect_anonymous_functions_from_expression(&member.object, true, functions);
        }
        Expression::ComputedMemberExpression(member) => {
            collect_anonymous_functions_from_expression(&member.object, true, functions);
            collect_anonymous_functions_from_expression(&member.expression, true, functions);
        }
        Expression::PrivateFieldExpression(member) => {
            collect_anonymous_functions_from_expression(&member.object, true, functions);
        }
        Expression::ChainExpression(chain) => {
            collect_anonymous_functions_from_chain_element(&chain.expression, functions);
        }
        Expression::TaggedTemplateExpression(tagged) => {
            collect_anonymous_functions_from_expression(&tagged.tag, true, functions);
            for expression in &tagged.quasi.expressions {
                collect_anonymous_functions_from_expression(expression, true, functions);
            }
        }
        Expression::ParenthesizedExpression(expression) => {
            collect_anonymous_functions_from_expression(
                &expression.expression,
                include_self,
                functions,
            );
        }
        Expression::AssignmentExpression(expression) => {
            collect_anonymous_functions_from_expression(&expression.right, true, functions);
        }
        Expression::AwaitExpression(expression) => {
            collect_anonymous_functions_from_expression(&expression.argument, true, functions);
        }
        Expression::YieldExpression(expression) => {
            if let Some(argument) = &expression.argument {
                collect_anonymous_functions_from_expression(argument, true, functions);
            }
        }
        Expression::SequenceExpression(expression) => {
            for expression in &expression.expressions {
                collect_anonymous_functions_from_expression(expression, true, functions);
            }
        }
        Expression::ConditionalExpression(expression) => {
            collect_anonymous_functions_from_expression(&expression.test, true, functions);
            collect_anonymous_functions_from_expression(&expression.consequent, true, functions);
            collect_anonymous_functions_from_expression(&expression.alternate, true, functions);
        }
        Expression::LogicalExpression(expression) => {
            collect_anonymous_functions_from_expression(&expression.left, true, functions);
            collect_anonymous_functions_from_expression(&expression.right, true, functions);
        }
        Expression::ObjectExpression(object) => {
            for property in &object.properties {
                match property {
                    // A shorthand method (`{ m() { … } }`) is emitted by the frontend
                    // at the PROPERTY span (anonymous_callable_name over property.span),
                    // but its `value` FunctionExpression span starts at `(` — so the
                    // generic FunctionExpression arm would mint a candidate at a span
                    // that no FunctionFact matches, and the method body would never be
                    // lowered (no call sites for `super.m()`, `this.x()`, direct calls
                    // inside it). Lower it at the property span to match the fact.
                    ObjectPropertyKind::ObjectProperty(property)
                        if property.method
                            && let Expression::FunctionExpression(function) = &property.value
                            && let Some(body) = function.body.as_deref() =>
                    {
                        functions.push(TsFunctionCandidate {
                            name: anonymous_callable_name(property.span.start, property.span.end),
                            span: property.span,
                            parameters: parameter_names(&function.params),
                            body: CandidateBody::Statements(&body.statements),
                        });
                        collect_anonymous_functions_from_body(body, functions);
                    }
                    ObjectPropertyKind::ObjectProperty(property) => {
                        collect_anonymous_functions_from_expression(
                            &property.value,
                            true,
                            functions,
                        );
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        collect_anonymous_functions_from_expression(
                            &spread.argument,
                            true,
                            functions,
                        );
                    }
                }
            }
        }
        Expression::ArrayExpression(array) => {
            for element in &array.elements {
                match element {
                    oxc_ast::ast::ArrayExpressionElement::SpreadElement(spread) => {
                        collect_anonymous_functions_from_expression(
                            &spread.argument,
                            true,
                            functions,
                        );
                    }
                    oxc_ast::ast::ArrayExpressionElement::Elision(_) => {}
                    _ => collect_anonymous_functions_from_expression(
                        element.to_expression(),
                        true,
                        functions,
                    ),
                }
            }
        }
        Expression::ClassExpression(class) => {
            collect_anonymous_functions_from_class(class, functions);
        }
        _ => {}
    }
}

fn collect_anonymous_functions_from_chain_element<'ast>(
    element: &'ast oxc_ast::ast::ChainElement<'ast>,
    functions: &mut Vec<TsFunctionCandidate<'ast>>,
) {
    match element {
        oxc_ast::ast::ChainElement::CallExpression(call) => {
            collect_anonymous_functions_from_expression(&call.callee, true, functions);
            for argument in &call.arguments {
                collect_anonymous_functions_from_argument(argument, functions);
            }
        }
        oxc_ast::ast::ChainElement::StaticMemberExpression(member) => {
            collect_anonymous_functions_from_expression(&member.object, true, functions);
        }
        oxc_ast::ast::ChainElement::ComputedMemberExpression(member) => {
            collect_anonymous_functions_from_expression(&member.object, true, functions);
            collect_anonymous_functions_from_expression(&member.expression, true, functions);
        }
        oxc_ast::ast::ChainElement::PrivateFieldExpression(member) => {
            collect_anonymous_functions_from_expression(&member.object, true, functions);
        }
        oxc_ast::ast::ChainElement::TSNonNullExpression(expression) => {
            collect_anonymous_functions_from_expression(&expression.expression, true, functions);
        }
    }
}

fn collect_anonymous_functions_from_argument<'ast>(
    argument: &'ast Argument<'ast>,
    functions: &mut Vec<TsFunctionCandidate<'ast>>,
) {
    match argument {
        Argument::SpreadElement(spread) => {
            collect_anonymous_functions_from_expression(&spread.argument, true, functions);
        }
        Argument::ArrowFunctionExpression(function) => {
            functions.push(TsFunctionCandidate {
                name: anonymous_callable_name(function.span.start, function.span.end),
                span: function.span,
                parameters: parameter_names(&function.params),
                body: arrow_candidate_body(function),
            });
            collect_anonymous_functions_from_body(&function.body, functions);
        }
        Argument::FunctionExpression(function) => {
            if let Some(body) = function.body.as_deref() {
                functions.push(TsFunctionCandidate {
                    name: anonymous_callable_name(function.span.start, function.span.end),
                    span: function.span,
                    parameters: parameter_names(&function.params),
                    body: CandidateBody::Statements(&body.statements),
                });
                collect_anonymous_functions_from_body(body, functions);
            }
        }
        _ => collect_anonymous_functions_from_expression(argument.to_expression(), true, functions),
    }
}

fn method_name(method: &MethodDefinition<'_>) -> Option<String> {
    match &method.key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
        PropertyKey::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
            Some(format!(
                "{}{}",
                constant_property_key_expression(&binary.left)?,
                constant_property_key_expression(&binary.right)?
            ))
        }
        _ => None,
    }
}

fn constant_property_key(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
        _ => constant_property_key_expression(key.to_expression()),
    }
}

fn constant_property_key_expression(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::StringLiteral(literal) => Some(literal.value.to_string()),
        Expression::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
            Some(format!(
                "{}{}",
                constant_property_key_expression(&binary.left)?,
                constant_property_key_expression(&binary.right)?
            ))
        }
        Expression::ParenthesizedExpression(expression) => {
            constant_property_key_expression(&expression.expression)
        }
        _ => None,
    }
}

fn binary_operator_text(source: &str, binary: &oxc_ast::ast::BinaryExpression<'_>) -> String {
    source
        .get(binary.left.span().end as usize..binary.right.span().start as usize)
        .map(str::trim)
        .filter(|operator| !operator.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn ts_nil_test(expression: &Expression<'_>) -> Option<BranchNilTest> {
    let Expression::BinaryExpression(binary) = expression else {
        return None;
    };
    let compares_null = matches!(&binary.left, Expression::NullLiteral(_))
        || matches!(&binary.right, Expression::NullLiteral(_));
    if !compares_null {
        return None;
    }
    // `== null` also matches `undefined`, so its non-null edge proves non-nil.
    // `=== null` does not: a value that is `undefined` takes the same edge, so
    // only the null edge carries a conclusion.
    match binary.operator {
        BinaryOperator::Equality => Some(BranchNilTest::Exhaustive { nil_on_true: true }),
        BinaryOperator::Inequality => Some(BranchNilTest::Exhaustive { nil_on_true: false }),
        BinaryOperator::StrictEquality => Some(BranchNilTest::NilEdgeOnly { nil_on_true: true }),
        BinaryOperator::StrictInequality => Some(BranchNilTest::NilEdgeOnly { nil_on_true: false }),
        _ => None,
    }
}

fn ts_null_operand<'a, 'ast>(expression: &'a Expression<'ast>) -> Option<&'a Expression<'ast>> {
    let Expression::BinaryExpression(binary) = expression else {
        return None;
    };
    if matches!(&binary.left, Expression::NullLiteral(_)) {
        Some(&binary.right)
    } else if matches!(&binary.right, Expression::NullLiteral(_)) {
        Some(&binary.left)
    } else {
        None
    }
}

fn type_shape_for_ts_expression(expression: &Expression<'_>) -> TypeShape {
    match expression {
        Expression::StringLiteral(_) => TypeShape::Primitive("string".to_string()),
        Expression::NumericLiteral(_) => TypeShape::Primitive("number".to_string()),
        Expression::BooleanLiteral(_) => TypeShape::Primitive("boolean".to_string()),
        Expression::NullLiteral(_) => TypeShape::Nullish("null".to_string()),
        Expression::ObjectExpression(_) => TypeShape::Object { shape_id: None },
        Expression::ArrayExpression(_) => TypeShape::Structural {
            shape_id: "array".to_string(),
        },
        Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {
            TypeShape::Callable {
                signature: "closure".to_string(),
            }
        }
        Expression::ParenthesizedExpression(expression) => {
            type_shape_for_ts_expression(&expression.expression)
        }
        _ => TypeShape::Unknown {
            reason: "ts/js expression".to_string(),
        },
    }
}

fn parameter_names(parameters: &FormalParameters<'_>) -> Vec<String> {
    let mut names = Vec::new();
    for parameter in &parameters.items {
        if let Some(name) = binding_identifier_name(&parameter.pattern) {
            names.push(name);
        }
    }
    if let Some(rest) = &parameters.rest
        && let Some(name) = binding_identifier_name(&rest.rest.argument)
    {
        names.push(name);
    }
    names
}

fn binding_identifier_name(pattern: &BindingPattern<'_>) -> Option<String> {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => Some(identifier.name.to_string()),
        BindingPattern::AssignmentPattern(pattern) => binding_identifier_name(&pattern.left),
        _ => None,
    }
}

struct FunctionLowering<'source> {
    language: Language,
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
            language: file.language,
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

    fn lower_parameters(&mut self, names: &[String], places: &mut PlaceTableBuilder) {
        for (index, name) in names.iter().enumerate() {
            let root = PlaceRoot::Parameter {
                function: self.function,
                index: index as u32,
                name: Some(name.clone()),
            };
            self.parameters.insert(name.clone(), root.clone());
            self.insert_place(places, root, Vec::new(), PlaceStatus::Resolved);
        }
    }

    fn lower_statements(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        statements: &[Statement<'_>],
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) {
        for statement in statements {
            self.lower_statement(interner, statement, places, operations, unsupported);
        }
    }

    fn lower_statement(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        statement: &Statement<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) {
        match statement {
            Statement::BlockStatement(block) => {
                for statement in &block.body {
                    self.lower_statement(interner, statement, places, operations, unsupported);
                }
            }
            Statement::ExpressionStatement(statement) => {
                if let Some(shape) = self.lower_expression(
                    interner,
                    &statement.expression,
                    places,
                    operations,
                    unsupported,
                    false,
                ) {
                    self.push_operation(
                        interner,
                        operations,
                        statement.span,
                        OperationKindDraft::Read {
                            place_key: shape.key,
                        },
                        MirStatus::Partial,
                    );
                }
            }
            Statement::VariableDeclaration(variable) => {
                for declarator in &variable.declarations {
                    self.lower_variable_declarator(
                        interner,
                        declarator,
                        places,
                        operations,
                        unsupported,
                    );
                }
            }
            Statement::ReturnStatement(statement) => {
                let value = statement.argument.as_ref().and_then(|argument| {
                    self.lower_value(interner, argument, places, operations, unsupported)
                });
                self.push_operation(
                    interner,
                    operations,
                    statement.span,
                    OperationKindDraft::Return { value },
                    MirStatus::Partial,
                );
            }
            Statement::IfStatement(statement) => {
                let nil_test = ts_nil_test(&statement.test);
                let predicate_expression =
                    ts_null_operand(&statement.test).unwrap_or(&statement.test);
                let predicate_place_key = self
                    .lower_expression(
                        interner,
                        predicate_expression,
                        places,
                        operations,
                        unsupported,
                        false,
                    )
                    .map(|source| source.key);
                let branch = self.push_branch_with_predicate(
                    interner,
                    operations,
                    statement.test.span(),
                    ControlShape::Conditional,
                    predicate_place_key,
                    nil_test,
                );
                let then_start = operations.len();
                self.lower_statement(
                    interner,
                    &statement.consequent,
                    places,
                    operations,
                    unsupported,
                );
                let then_end = (operations.len() > then_start)
                    .then(|| operations.last().expect("then operation").id);
                let else_start = operations.len();
                if let Some(alternate) = &statement.alternate {
                    self.lower_statement(interner, alternate, places, operations, unsupported);
                }
                let else_end = (operations.len() > else_start)
                    .then(|| operations.last().expect("else operation").id);
                set_branch_region(operations, branch, BranchRegion { then_end, else_end });
            }
            Statement::WhileStatement(statement) => {
                self.push_branch(interner, operations, statement.span, ControlShape::Loop);
                self.lower_expression(
                    interner,
                    &statement.test,
                    places,
                    operations,
                    unsupported,
                    false,
                );
                self.lower_statement(interner, &statement.body, places, operations, unsupported);
            }
            Statement::DoWhileStatement(statement) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    statement.span,
                    "do while",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                self.push_branch(interner, operations, statement.span, ControlShape::Loop);
                self.lower_statement(interner, &statement.body, places, operations, unsupported);
                self.lower_expression(
                    interner,
                    &statement.test,
                    places,
                    operations,
                    unsupported,
                    false,
                );
            }
            Statement::ForStatement(statement) => {
                if let Some(init) = &statement.init {
                    self.lower_for_statement_init(interner, init, places, operations, unsupported);
                }
                if let Some(test) = &statement.test {
                    self.push_branch(interner, operations, test.span(), ControlShape::Loop);
                    self.lower_expression(interner, test, places, operations, unsupported, false);
                }
                if let Some(update) = &statement.update {
                    self.lower_expression(interner, update, places, operations, unsupported, false);
                }
                self.lower_statement(interner, &statement.body, places, operations, unsupported);
            }
            Statement::ForInStatement(statement) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    statement.span,
                    "for-in",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                self.lower_for_statement_left(
                    interner,
                    &statement.left,
                    places,
                    operations,
                    unsupported,
                );
                self.lower_expression(
                    interner,
                    &statement.right,
                    places,
                    operations,
                    unsupported,
                    false,
                );
                self.lower_statement(interner, &statement.body, places, operations, unsupported);
            }
            Statement::ForOfStatement(statement) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    statement.span,
                    "for-of",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                self.lower_for_statement_left(
                    interner,
                    &statement.left,
                    places,
                    operations,
                    unsupported,
                );
                let value =
                    self.lower_value(interner, &statement.right, places, operations, unsupported);
                if statement.r#await {
                    self.push_operation(
                        interner,
                        operations,
                        statement.span,
                        OperationKindDraft::Suspend {
                            kind: SuspendKind::Await,
                            value,
                        },
                        MirStatus::Partial,
                    );
                }
                self.lower_statement(interner, &statement.body, places, operations, unsupported);
            }
            Statement::SwitchStatement(statement) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    statement.span,
                    "switch",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                let branch =
                    self.push_branch(interner, operations, statement.span, ControlShape::Switch);
                let discriminant = self.lower_expression(
                    interner,
                    &statement.discriminant,
                    places,
                    operations,
                    unsupported,
                    false,
                );
                // A switch lowers to a `Switch` terminator whose case and default
                // edges carry no truthiness or nilness assumption, so the
                // discriminant is recorded as evidence and never refines a state.
                set_branch_predicate(
                    operations,
                    branch,
                    discriminant.map(|shape| shape.key),
                    None,
                );
                for case in &statement.cases {
                    if let Some(test) = &case.test {
                        // A case tests `discriminant === test`, not the truthiness
                        // of `test`, so the case branch carries no bound predicate.
                        self.push_branch(
                            interner,
                            operations,
                            test.span(),
                            ControlShape::Conditional,
                        );
                        self.lower_expression(
                            interner,
                            test,
                            places,
                            operations,
                            unsupported,
                            false,
                        );
                    }
                    for statement in &case.consequent {
                        self.lower_statement(interner, statement, places, operations, unsupported);
                    }
                }
            }
            Statement::TryStatement(statement) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    statement.span,
                    "try",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                for statement in &statement.block.body {
                    self.lower_statement(interner, statement, places, operations, unsupported);
                }
                if let Some(handler) = &statement.handler {
                    if let Some(param) = &handler.param {
                        if let Some(name) = binding_identifier_name(&param.pattern) {
                            self.insert_local(places, &name);
                        } else {
                            self.push_unsupported(
                                interner,
                                operations,
                                unsupported,
                                param.span,
                                "catch destructuring",
                                (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                            );
                        }
                    }
                    for statement in &handler.body.body {
                        self.lower_statement(interner, statement, places, operations, unsupported);
                    }
                }
                if let Some(finalizer) = &statement.finalizer {
                    for statement in &finalizer.body {
                        self.lower_statement(interner, statement, places, operations, unsupported);
                    }
                }
            }
            Statement::ThrowStatement(statement) => {
                let value = self.lower_value(
                    interner,
                    &statement.argument,
                    places,
                    operations,
                    unsupported,
                );
                self.push_operation(
                    interner,
                    operations,
                    statement.span,
                    OperationKindDraft::Throw { value },
                    MirStatus::Partial,
                );
            }
            Statement::BreakStatement(statement) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    statement.span,
                    "break",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
            }
            Statement::ContinueStatement(statement) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    statement.span,
                    "continue",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
            }
            Statement::LabeledStatement(statement) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    statement.span,
                    "labeled statement",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                self.lower_statement(interner, &statement.body, places, operations, unsupported);
            }
            Statement::WithStatement(statement) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    statement.span,
                    "with",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                self.lower_expression(
                    interner,
                    &statement.object,
                    places,
                    operations,
                    unsupported,
                    false,
                );
                self.lower_statement(interner, &statement.body, places, operations, unsupported);
            }
            Statement::EmptyStatement(_) => {}
            Statement::DebuggerStatement(statement) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    statement.span,
                    "debugger",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
            }
            _ => {}
        }
    }

    fn lower_variable_declarator(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        declarator: &VariableDeclarator<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) {
        let Some(name) = binding_identifier_name(&declarator.id) else {
            if let Some(init) = &declarator.init {
                self.lower_expression(interner, init, places, operations, unsupported, false);
            }
            self.push_unsupported(
                interner,
                operations,
                unsupported,
                declarator.span,
                "complex destructuring",
                (Vec::new(), ConservativeAction::HavocAffectedPlaces),
            );
            return;
        };
        let key = self.insert_local_typed(
            places,
            &name,
            declarator
                .init
                .as_ref()
                .map(|init| type_shape_for_ts_expression(init)),
        );
        if let Some(init) = &declarator.init {
            let value = self
                .lower_value(interner, init, places, operations, unsupported)
                .unwrap_or_else(|| ValueDraft::Unknown {
                    evidence: "declaration initializer".to_string(),
                });
            self.push_assign(
                interner,
                operations,
                declarator.span,
                key,
                value,
                AssignMode::DeclarationBinding,
            );
        } else {
            self.push_operation(
                interner,
                operations,
                declarator.span,
                OperationKindDraft::StorageLive { place_key: key },
                MirStatus::Partial,
            );
        }
    }

    fn lower_for_statement_init(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        init: &oxc_ast::ast::ForStatementInit<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) {
        match init {
            oxc_ast::ast::ForStatementInit::VariableDeclaration(variable) => {
                for declarator in &variable.declarations {
                    self.lower_variable_declarator(
                        interner,
                        declarator,
                        places,
                        operations,
                        unsupported,
                    );
                }
            }
            _ => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    init.span(),
                    "for initializer",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                self.lower_expression(
                    interner,
                    init.to_expression(),
                    places,
                    operations,
                    unsupported,
                    false,
                );
            }
        }
    }

    fn lower_for_statement_left(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        left: &oxc_ast::ast::ForStatementLeft<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) {
        match left {
            oxc_ast::ast::ForStatementLeft::VariableDeclaration(variable) => {
                for declarator in &variable.declarations {
                    self.lower_variable_declarator(
                        interner,
                        declarator,
                        places,
                        operations,
                        unsupported,
                    );
                }
            }
            _ => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    left.span(),
                    "for left binding",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                if let Some(target) = self.assignment_target_shape(
                    interner,
                    left.to_assignment_target(),
                    places,
                    operations,
                    unsupported,
                ) {
                    self.push_assign(
                        interner,
                        operations,
                        left.span(),
                        target.key,
                        ValueDraft::Unknown {
                            evidence: "for left binding".to_string(),
                        },
                        if target.projections.is_empty() {
                            AssignMode::Overwrite
                        } else {
                            AssignMode::ProjectionMutation
                        },
                    );
                }
            }
        }
    }

    fn lower_value(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        expression: &Expression<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) -> Option<ValueDraft> {
        match expression {
            Expression::StringLiteral(literal) => Some(ValueDraft::Literal {
                value: literal.value.to_string(),
            }),
            Expression::NumericLiteral(literal) => Some(ValueDraft::Literal {
                value: literal
                    .raw
                    .as_ref()
                    .map_or_else(|| literal.value.to_string(), ToString::to_string),
            }),
            Expression::BooleanLiteral(literal) => Some(ValueDraft::Literal {
                value: literal.value.to_string(),
            }),
            Expression::NullLiteral(_) => Some(ValueDraft::Literal {
                value: "null".to_string(),
            }),
            Expression::BinaryExpression(binary) => Some(ValueDraft::BinOp {
                op: binary_operator_text(self.source, binary),
                lhs: Box::new(
                    self.lower_value(interner, &binary.left, places, operations, unsupported)
                        .unwrap_or_else(|| ValueDraft::Unknown {
                            evidence: "binary left operand".to_string(),
                        }),
                ),
                rhs: Box::new(
                    self.lower_value(interner, &binary.right, places, operations, unsupported)
                        .unwrap_or_else(|| ValueDraft::Unknown {
                            evidence: "binary right operand".to_string(),
                        }),
                ),
            }),
            Expression::ObjectExpression(object) => Some(ValueDraft::Aggregate {
                kind: MirAggregateKind::Object,
                fields: object
                    .properties
                    .iter()
                    .map(|property| match property {
                        ObjectPropertyKind::ObjectProperty(property) => {
                            if property.computed {
                                self.lower_expression(
                                    interner,
                                    property.key.to_expression(),
                                    places,
                                    operations,
                                    unsupported,
                                    false,
                                );
                            }
                            (
                                constant_property_key(&property.key),
                                self.lower_value(
                                    interner,
                                    &property.value,
                                    places,
                                    operations,
                                    unsupported,
                                )
                                .unwrap_or_else(|| {
                                    ValueDraft::Unknown {
                                        evidence: "object property value".to_string(),
                                    }
                                }),
                            )
                        }
                        ObjectPropertyKind::SpreadProperty(spread) => {
                            self.push_unsupported(
                                interner,
                                operations,
                                unsupported,
                                spread.span,
                                "spread",
                                (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                            );
                            (
                                None,
                                self.lower_value(
                                    interner,
                                    &spread.argument,
                                    places,
                                    operations,
                                    unsupported,
                                )
                                .unwrap_or_else(|| {
                                    ValueDraft::Unknown {
                                        evidence: "object spread value".to_string(),
                                    }
                                }),
                            )
                        }
                    })
                    .collect(),
            }),
            Expression::ArrayExpression(array) => Some(ValueDraft::Aggregate {
                kind: MirAggregateKind::Array,
                fields: array
                    .elements
                    .iter()
                    .enumerate()
                    .filter_map(|(index, element)| match element {
                        oxc_ast::ast::ArrayExpressionElement::Elision(_) => None,
                        oxc_ast::ast::ArrayExpressionElement::SpreadElement(spread) => {
                            self.push_unsupported(
                                interner,
                                operations,
                                unsupported,
                                spread.span,
                                "spread",
                                (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                            );
                            Some((
                                Some(index.to_string()),
                                self.lower_value(
                                    interner,
                                    &spread.argument,
                                    places,
                                    operations,
                                    unsupported,
                                )
                                .unwrap_or_else(|| {
                                    ValueDraft::Unknown {
                                        evidence: "array spread value".to_string(),
                                    }
                                }),
                            ))
                        }
                        _ => Some((
                            Some(index.to_string()),
                            self.lower_value(
                                interner,
                                element.to_expression(),
                                places,
                                operations,
                                unsupported,
                            )
                            .unwrap_or_else(|| ValueDraft::Unknown {
                                evidence: "array element value".to_string(),
                            }),
                        )),
                    })
                    .collect(),
            }),
            Expression::ArrowFunctionExpression(function) => {
                self.closure_value(function.span, places)
            }
            Expression::FunctionExpression(function) => self.closure_value(function.span, places),
            Expression::CallExpression(call) => self
                .lower_call(interner, call, places, operations, unsupported)
                .map(ValueDraft::PlaceKey),
            _ => self
                .lower_expression(interner, expression, places, operations, unsupported, false)
                .map(|shape| ValueDraft::PlaceKey(shape.key)),
        }
    }

    fn closure_value(
        &self,
        span: oxc_span::Span,
        places: &mut PlaceTableBuilder,
    ) -> Option<ValueDraft> {
        let key = (span.start, span.end);
        let body = *self.closure_bodies.get(&key)?;
        let capture_keys = self
            .closure_capture_names
            .get(&key)
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
        expression: &Expression<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
        assignment_destination: bool,
    ) -> Option<PlaceShape> {
        match expression {
            Expression::Identifier(identifier) => {
                self.lower_identifier(identifier.name.as_str(), places, assignment_destination)
            }
            Expression::StaticMemberExpression(member) => self.lower_static_member(
                interner,
                member,
                places,
                operations,
                unsupported,
                assignment_destination,
            ),
            Expression::ComputedMemberExpression(member) => self.lower_computed_member(
                interner,
                member,
                places,
                operations,
                unsupported,
                assignment_destination,
            ),
            Expression::AssignmentExpression(assignment) => {
                if let Some(target) = self.assignment_target_shape(
                    interner,
                    &assignment.left,
                    places,
                    operations,
                    unsupported,
                ) {
                    let value = self
                        .lower_value(interner, &assignment.right, places, operations, unsupported)
                        .unwrap_or_else(|| ValueDraft::Unknown {
                            evidence: "assignment value".to_string(),
                        });
                    self.push_assign(
                        interner,
                        operations,
                        assignment.span,
                        target.key.clone(),
                        value,
                        if target.projections.is_empty() {
                            AssignMode::Overwrite
                        } else {
                            AssignMode::ProjectionMutation
                        },
                    );
                    Some(target)
                } else {
                    self.lower_expression(
                        interner,
                        &assignment.right,
                        places,
                        operations,
                        unsupported,
                        false,
                    )
                }
            }
            Expression::UpdateExpression(update) => {
                let target = self.simple_assignment_target_shape(
                    interner,
                    &update.argument,
                    places,
                    operations,
                    unsupported,
                )?;
                self.push_assign(
                    interner,
                    operations,
                    update.span,
                    target.key.clone(),
                    ValueDraft::Unknown {
                        evidence: "update expression".to_string(),
                    },
                    AssignMode::Overwrite,
                );
                Some(target)
            }
            Expression::ObjectExpression(object) => {
                for property in &object.properties {
                    match property {
                        ObjectPropertyKind::ObjectProperty(property) => {
                            if property.computed {
                                self.lower_expression(
                                    interner,
                                    property.key.to_expression(),
                                    places,
                                    operations,
                                    unsupported,
                                    false,
                                );
                            }
                            self.lower_expression(
                                interner,
                                &property.value,
                                places,
                                operations,
                                unsupported,
                                false,
                            );
                        }
                        ObjectPropertyKind::SpreadProperty(spread) => {
                            self.push_unsupported(
                                interner,
                                operations,
                                unsupported,
                                spread.span,
                                "spread",
                                (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                            );
                            self.lower_expression(
                                interner,
                                &spread.argument,
                                places,
                                operations,
                                unsupported,
                                false,
                            );
                        }
                    }
                }
                Some(self.temporary_shape_typed(
                    places,
                    expression.span(),
                    TypeShape::Object { shape_id: None },
                ))
            }
            Expression::ArrayExpression(array) => {
                for element in &array.elements {
                    match element {
                        oxc_ast::ast::ArrayExpressionElement::SpreadElement(spread) => {
                            self.push_unsupported(
                                interner,
                                operations,
                                unsupported,
                                spread.span,
                                "spread",
                                (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                            );
                            self.lower_expression(
                                interner,
                                &spread.argument,
                                places,
                                operations,
                                unsupported,
                                false,
                            );
                        }
                        oxc_ast::ast::ArrayExpressionElement::Elision(_) => {}
                        _ => {
                            self.lower_expression(
                                interner,
                                element.to_expression(),
                                places,
                                operations,
                                unsupported,
                                false,
                            );
                        }
                    }
                }
                Some(self.temporary_shape_typed(
                    places,
                    expression.span(),
                    TypeShape::Structural {
                        shape_id: "array".to_string(),
                    },
                ))
            }
            Expression::ParenthesizedExpression(parenthesized) => self.lower_expression(
                interner,
                &parenthesized.expression,
                places,
                operations,
                unsupported,
                assignment_destination,
            ),
            Expression::TSAsExpression(expression) => self.lower_expression(
                interner,
                &expression.expression,
                places,
                operations,
                unsupported,
                assignment_destination,
            ),
            Expression::TSSatisfiesExpression(expression) => self.lower_expression(
                interner,
                &expression.expression,
                places,
                operations,
                unsupported,
                assignment_destination,
            ),
            Expression::TSNonNullExpression(expression) => self.lower_expression(
                interner,
                &expression.expression,
                places,
                operations,
                unsupported,
                assignment_destination,
            ),
            Expression::TSTypeAssertion(expression) => self.lower_expression(
                interner,
                &expression.expression,
                places,
                operations,
                unsupported,
                assignment_destination,
            ),
            Expression::CallExpression(call) => self
                .lower_call(interner, call, places, operations, unsupported)
                .map(|key| PlaceShape {
                    root: PlaceRoot::CallReturn {
                        call: call_site_for_span(normalized_call_expression_span(
                            self.source,
                            call,
                        )),
                    },
                    projections: Vec::new(),
                    status: PlaceStatus::Partial,
                    key,
                }),
            Expression::BinaryExpression(binary) => {
                self.lower_expression(
                    interner,
                    &binary.left,
                    places,
                    operations,
                    unsupported,
                    false,
                );
                self.lower_expression(
                    interner,
                    &binary.right,
                    places,
                    operations,
                    unsupported,
                    false,
                );
                Some(self.temporary_shape_typed(
                    places,
                    binary.span,
                    TypeShape::Unknown {
                        reason: format!("binary {}", binary_operator_text(self.source, binary)),
                    },
                ))
            }
            Expression::UnaryExpression(unary) => {
                self.lower_expression(
                    interner,
                    &unary.argument,
                    places,
                    operations,
                    unsupported,
                    false,
                );
                Some(self.temporary_shape(places, unary.span))
            }
            Expression::ConditionalExpression(conditional) => {
                let branch = self.push_branch(
                    interner,
                    operations,
                    conditional.span,
                    ControlShape::Conditional,
                );
                let lowered = self.lower_expression(
                    interner,
                    &conditional.test,
                    places,
                    operations,
                    unsupported,
                    false,
                );
                self.bind_branch_predicate(
                    interner,
                    places,
                    operations,
                    branch,
                    &conditional.test,
                    lowered,
                );
                self.lower_expression(
                    interner,
                    &conditional.consequent,
                    places,
                    operations,
                    unsupported,
                    false,
                );
                self.lower_expression(
                    interner,
                    &conditional.alternate,
                    places,
                    operations,
                    unsupported,
                    false,
                )
            }
            Expression::LogicalExpression(logical) => {
                let branch = matches!(
                    logical.operator,
                    LogicalOperator::And | LogicalOperator::Or | LogicalOperator::Coalesce
                )
                .then(|| {
                    self.push_branch(
                        interner,
                        operations,
                        logical.span,
                        ControlShape::Conditional,
                    )
                });
                // `??` short-circuits on nullishness, not truthiness, so its left
                // operand is not bound: a bound predicate place without a nil test
                // is consumed as a truthiness assumption.
                let truthiness_branch = branch.filter(|_| {
                    matches!(logical.operator, LogicalOperator::And | LogicalOperator::Or)
                });
                let lowered = self.lower_expression(
                    interner,
                    &logical.left,
                    places,
                    operations,
                    unsupported,
                    false,
                );
                if let Some(branch) = truthiness_branch {
                    self.bind_branch_predicate(
                        interner,
                        places,
                        operations,
                        branch,
                        &logical.left,
                        lowered,
                    );
                }
                self.lower_expression(
                    interner,
                    &logical.right,
                    places,
                    operations,
                    unsupported,
                    false,
                )
            }
            Expression::TemplateLiteral(template) => {
                for expression in &template.expressions {
                    self.lower_expression(
                        interner,
                        expression,
                        places,
                        operations,
                        unsupported,
                        false,
                    );
                }
                Some(self.temporary_shape(places, template.span))
            }
            Expression::TaggedTemplateExpression(tagged) => self.lower_tagged_template_expression(
                interner,
                tagged,
                places,
                operations,
                unsupported,
            ),
            Expression::SequenceExpression(sequence) => {
                let mut last = None;
                for expression in &sequence.expressions {
                    last = self.lower_expression(
                        interner,
                        expression,
                        places,
                        operations,
                        unsupported,
                        false,
                    );
                }
                last.or_else(|| Some(self.temporary_shape(places, sequence.span)))
            }
            Expression::ChainExpression(chain) => self.lower_chain_element(
                interner,
                &chain.expression,
                places,
                operations,
                unsupported,
            ),
            Expression::AwaitExpression(await_expression) => {
                let shape = self.lower_expression(
                    interner,
                    &await_expression.argument,
                    places,
                    operations,
                    unsupported,
                    false,
                );
                self.push_operation(
                    interner,
                    operations,
                    await_expression.span,
                    OperationKindDraft::Suspend {
                        kind: SuspendKind::Await,
                        value: shape
                            .as_ref()
                            .map(|shape| ValueDraft::PlaceKey(shape.key.clone())),
                    },
                    MirStatus::Partial,
                );
                shape
            }
            Expression::ImportExpression(import_expression) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    import_expression.span,
                    "dynamic import",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                self.lower_expression(
                    interner,
                    &import_expression.source,
                    places,
                    operations,
                    unsupported,
                    false,
                );
                if let Some(options) = &import_expression.options {
                    self.lower_expression(
                        interner,
                        options,
                        places,
                        operations,
                        unsupported,
                        false,
                    );
                }
                Some(self.temporary_shape(places, import_expression.span))
            }
            Expression::YieldExpression(yield_expression) => {
                let shape = yield_expression.argument.as_ref().and_then(|argument| {
                    self.lower_expression(
                        interner,
                        argument,
                        places,
                        operations,
                        unsupported,
                        false,
                    )
                });
                self.push_operation(
                    interner,
                    operations,
                    yield_expression.span,
                    OperationKindDraft::Suspend {
                        kind: SuspendKind::Yield,
                        value: shape
                            .as_ref()
                            .map(|shape| ValueDraft::PlaceKey(shape.key.clone())),
                    },
                    MirStatus::Partial,
                );
                shape
            }
            Expression::NewExpression(new_expression) => self
                .lower_new_expression(interner, new_expression, places, operations, unsupported)
                .map(|key| PlaceShape {
                    root: PlaceRoot::CallReturn {
                        call: call_site_for_span(normalized_new_expression_span(
                            self.source,
                            new_expression,
                        )),
                    },
                    projections: Vec::new(),
                    status: PlaceStatus::Partial,
                    key,
                }),
            Expression::PrivateFieldExpression(private) => self.lower_private_field(
                interner,
                private,
                places,
                operations,
                unsupported,
                assignment_destination,
            ),
            Expression::PrivateInExpression(private_in) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    private_in.span,
                    "private in",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                self.lower_expression(
                    interner,
                    &private_in.right,
                    places,
                    operations,
                    unsupported,
                    false,
                );
                Some(self.temporary_shape(places, private_in.span))
            }
            Expression::ArrowFunctionExpression(function) => {
                let value = self.closure_value(function.span, places)?;
                let shape = self.temporary_shape_typed(
                    places,
                    function.span,
                    TypeShape::Callable {
                        signature: "closure".to_string(),
                    },
                );
                self.push_assign(
                    interner,
                    operations,
                    function.span,
                    shape.key.clone(),
                    value,
                    AssignMode::Overwrite,
                );
                Some(shape)
            }
            Expression::FunctionExpression(function) => {
                let value = self.closure_value(function.span, places)?;
                let shape = self.temporary_shape_typed(
                    places,
                    function.span,
                    TypeShape::Callable {
                        signature: "closure".to_string(),
                    },
                );
                self.push_assign(
                    interner,
                    operations,
                    function.span,
                    shape.key.clone(),
                    value,
                    AssignMode::Overwrite,
                );
                Some(shape)
            }
            Expression::ClassExpression(class) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    class.span,
                    "class expression",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                Some(self.temporary_shape(places, class.span))
            }
            Expression::ThisExpression(this_expression) => Some(self.keyword_shape(
                places,
                "this",
                this_expression.span,
                assignment_destination,
            )),
            Expression::Super(super_expression) => Some(self.keyword_shape(
                places,
                "super",
                super_expression.span,
                assignment_destination,
            )),
            Expression::MetaProperty(meta) => Some(self.keyword_shape(
                places,
                &format!("{}.{}", meta.meta.name, meta.property.name),
                meta.span,
                assignment_destination,
            )),
            Expression::TSInstantiationExpression(expression) => self.lower_expression(
                interner,
                &expression.expression,
                places,
                operations,
                unsupported,
                assignment_destination,
            ),
            Expression::JSXElement(element) => {
                if source_text(self.source, element.span).is_some_and(|text| text.contains("=>")) {
                    self.push_unsupported(
                        interner,
                        operations,
                        unsupported,
                        element.span,
                        "JSX callback scheduling",
                        (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                    );
                }
                self.lower_jsx_element(interner, element, places, operations, unsupported);
                Some(self.temporary_shape(places, element.span))
            }
            Expression::JSXFragment(fragment) => {
                self.lower_jsx_children(
                    interner,
                    &fragment.children,
                    places,
                    operations,
                    unsupported,
                );
                Some(self.temporary_shape(places, fragment.span))
            }
            Expression::V8IntrinsicExpression(intrinsic) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    intrinsic.span,
                    "v8 intrinsic",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                Some(self.temporary_shape(places, intrinsic.span))
            }
            Expression::BigIntLiteral(literal) => Some(self.temporary_shape(places, literal.span)),
            Expression::RegExpLiteral(literal) => Some(self.temporary_shape(places, literal.span)),
            Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::StringLiteral(_) => Some(self.temporary_shape(places, expression.span())),
        }
    }

    fn lower_chain_element(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        element: &oxc_ast::ast::ChainElement<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) -> Option<PlaceShape> {
        match element {
            oxc_ast::ast::ChainElement::CallExpression(call) => self
                .lower_call(interner, call, places, operations, unsupported)
                .map(|key| PlaceShape {
                    root: PlaceRoot::CallReturn {
                        call: call_site_for_span(normalized_call_expression_span(
                            self.source,
                            call,
                        )),
                    },
                    projections: Vec::new(),
                    status: PlaceStatus::Partial,
                    key,
                }),
            oxc_ast::ast::ChainElement::StaticMemberExpression(member) => {
                self.lower_static_member(interner, member, places, operations, unsupported, false)
            }
            oxc_ast::ast::ChainElement::ComputedMemberExpression(member) => {
                self.lower_computed_member(interner, member, places, operations, unsupported, false)
            }
            oxc_ast::ast::ChainElement::PrivateFieldExpression(private) => {
                self.lower_private_field(interner, private, places, operations, unsupported, false)
            }
            oxc_ast::ast::ChainElement::TSNonNullExpression(expression) => self.lower_expression(
                interner,
                &expression.expression,
                places,
                operations,
                unsupported,
                false,
            ),
        }
    }

    fn lower_private_field(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        private: &oxc_ast::ast::PrivateFieldExpression<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
        _assignment_destination: bool,
    ) -> Option<PlaceShape> {
        if private.optional {
            self.push_unsupported(
                interner,
                operations,
                unsupported,
                private.span,
                "optional chaining",
                (Vec::new(), ConservativeAction::HavocAffectedPlaces),
            );
        }
        self.push_unsupported(
            interner,
            operations,
            unsupported,
            private.field.span,
            "private field",
            (Vec::new(), ConservativeAction::HavocAffectedPlaces),
        );
        let mut shape = self.lower_expression(
            interner,
            &private.object,
            places,
            operations,
            unsupported,
            false,
        )?;
        shape.projections.push(PlaceProjection::Unknown {
            evidence: format!("#{}", private.field.name),
        });
        shape.key = self.insert_shape(places, &shape);
        self.insert_temporary(places, private.span, PlaceStatus::Partial);
        Some(shape)
    }

    fn keyword_shape(
        &self,
        places: &mut PlaceTableBuilder,
        evidence: &str,
        span: oxc_span::Span,
        _assignment_destination: bool,
    ) -> PlaceShape {
        let root = PlaceRoot::Unknown {
            evidence: evidence.to_string(),
        };
        let key = self.insert_place(places, root.clone(), Vec::new(), PlaceStatus::Unknown);
        self.insert_temporary(places, span, PlaceStatus::Partial);
        PlaceShape {
            root,
            projections: Vec::new(),
            status: PlaceStatus::Unknown,
            key,
        }
    }

    fn lower_jsx_element(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        element: &oxc_ast::ast::JSXElement<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) {
        for attribute in &element.opening_element.attributes {
            match attribute {
                oxc_ast::ast::JSXAttributeItem::Attribute(attribute) => {
                    if let Some(value) = &attribute.value {
                        self.lower_jsx_attribute_value(
                            interner,
                            value,
                            places,
                            operations,
                            unsupported,
                        );
                    }
                }
                oxc_ast::ast::JSXAttributeItem::SpreadAttribute(spread) => {
                    self.push_unsupported(
                        interner,
                        operations,
                        unsupported,
                        spread.span,
                        "spread",
                        (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                    );
                    self.lower_expression(
                        interner,
                        &spread.argument,
                        places,
                        operations,
                        unsupported,
                        false,
                    );
                }
            }
        }
        self.lower_jsx_children(interner, &element.children, places, operations, unsupported);
    }

    fn lower_jsx_attribute_value(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        value: &oxc_ast::ast::JSXAttributeValue<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) {
        match value {
            oxc_ast::ast::JSXAttributeValue::ExpressionContainer(container) => {
                self.lower_jsx_expression(
                    interner,
                    &container.expression,
                    places,
                    operations,
                    unsupported,
                );
            }
            oxc_ast::ast::JSXAttributeValue::Element(element) => {
                self.lower_jsx_element(interner, element, places, operations, unsupported);
            }
            oxc_ast::ast::JSXAttributeValue::Fragment(fragment) => {
                self.lower_jsx_children(
                    interner,
                    &fragment.children,
                    places,
                    operations,
                    unsupported,
                );
            }
            oxc_ast::ast::JSXAttributeValue::StringLiteral(_) => {}
        }
    }

    fn lower_jsx_children(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        children: &[oxc_ast::ast::JSXChild<'_>],
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) {
        for child in children {
            match child {
                oxc_ast::ast::JSXChild::Element(element) => {
                    self.lower_jsx_element(interner, element, places, operations, unsupported);
                }
                oxc_ast::ast::JSXChild::Fragment(fragment) => {
                    self.lower_jsx_children(
                        interner,
                        &fragment.children,
                        places,
                        operations,
                        unsupported,
                    );
                }
                oxc_ast::ast::JSXChild::ExpressionContainer(container) => {
                    self.lower_jsx_expression(
                        interner,
                        &container.expression,
                        places,
                        operations,
                        unsupported,
                    );
                }
                oxc_ast::ast::JSXChild::Spread(spread) => {
                    self.push_unsupported(
                        interner,
                        operations,
                        unsupported,
                        spread.span,
                        "spread",
                        (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                    );
                    self.lower_expression(
                        interner,
                        &spread.expression,
                        places,
                        operations,
                        unsupported,
                        false,
                    );
                }
                oxc_ast::ast::JSXChild::Text(_) => {}
            }
        }
    }

    fn lower_jsx_expression(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        expression: &oxc_ast::ast::JSXExpression<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) {
        match expression {
            oxc_ast::ast::JSXExpression::EmptyExpression(_) => {}
            _ => {
                self.lower_expression(
                    interner,
                    expression.to_expression(),
                    places,
                    operations,
                    unsupported,
                    false,
                );
            }
        }
    }

    fn lower_static_member(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        member: &oxc_ast::ast::StaticMemberExpression<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
        _assignment_destination: bool,
    ) -> Option<PlaceShape> {
        if member.optional {
            self.push_unsupported(
                interner,
                operations,
                unsupported,
                member.span,
                "optional chaining",
                (Vec::new(), ConservativeAction::HavocAffectedPlaces),
            );
        }
        let mut shape = self.lower_expression(
            interner,
            &member.object,
            places,
            operations,
            unsupported,
            false,
        )?;
        shape
            .projections
            .push(PlaceProjection::Property(member.property.name.to_string()));
        shape.key = self.insert_shape(places, &shape);
        self.insert_temporary(places, member.span, PlaceStatus::Partial);
        Some(shape)
    }

    fn lower_computed_member(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        member: &oxc_ast::ast::ComputedMemberExpression<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
        _assignment_destination: bool,
    ) -> Option<PlaceShape> {
        if member.optional {
            self.push_unsupported(
                interner,
                operations,
                unsupported,
                member.span,
                "optional chaining",
                (Vec::new(), ConservativeAction::HavocAffectedPlaces),
            );
        }
        let mut shape = self.lower_expression(
            interner,
            &member.object,
            places,
            operations,
            unsupported,
            false,
        )?;
        shape.projections.push(index_projection(&member.expression));
        shape.key = self.insert_shape(places, &shape);
        if matches!(
            shape.projections.last(),
            Some(PlaceProjection::IndexUnknown { .. })
        ) {
            self.push_unsupported(
                interner,
                operations,
                unsupported,
                member.expression.span(),
                "dynamic property key",
                (
                    vec![shape.key.clone()],
                    ConservativeAction::HavocAffectedPlaces,
                ),
            );
        }
        self.lower_expression(
            interner,
            &member.expression,
            places,
            operations,
            unsupported,
            false,
        );
        self.insert_temporary(places, member.span, PlaceStatus::Partial);
        Some(shape)
    }

    fn lower_identifier(
        &self,
        name: &str,
        places: &mut PlaceTableBuilder,
        assignment_destination: bool,
    ) -> Option<PlaceShape> {
        let (root, status) = if let Some(root) = self.locals.get(name) {
            (root.clone(), PlaceStatus::Resolved)
        } else if let Some(root) = self.parameters.get(name) {
            (root.clone(), PlaceStatus::Resolved)
        } else if assignment_destination {
            (
                PlaceRoot::Global {
                    symbol: None,
                    name: name.to_string(),
                },
                PlaceStatus::Partial,
            )
        } else {
            (
                PlaceRoot::Unknown {
                    evidence: name.to_string(),
                },
                PlaceStatus::Unknown,
            )
        };
        let mut shape = PlaceShape {
            root,
            projections: Vec::new(),
            status,
            key: String::new(),
        };
        shape.key = self.insert_shape(places, &shape);
        Some(shape)
    }

    fn assignment_target_shape(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        target: &oxc_ast::ast::AssignmentTarget<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) -> Option<PlaceShape> {
        match target {
            oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(identifier) => {
                self.lower_identifier(identifier.name.as_str(), places, true)
            }
            oxc_ast::ast::AssignmentTarget::StaticMemberExpression(member) => {
                self.lower_static_member(interner, member, places, operations, unsupported, true)
            }
            oxc_ast::ast::AssignmentTarget::ComputedMemberExpression(member) => {
                self.lower_computed_member(interner, member, places, operations, unsupported, true)
            }
            oxc_ast::ast::AssignmentTarget::PrivateFieldExpression(private) => {
                self.lower_private_field(interner, private, places, operations, unsupported, true)
            }
            oxc_ast::ast::AssignmentTarget::TSAsExpression(expression) => self.lower_expression(
                interner,
                &expression.expression,
                places,
                operations,
                unsupported,
                true,
            ),
            oxc_ast::ast::AssignmentTarget::TSSatisfiesExpression(expression) => self
                .lower_expression(
                    interner,
                    &expression.expression,
                    places,
                    operations,
                    unsupported,
                    true,
                ),
            oxc_ast::ast::AssignmentTarget::TSNonNullExpression(expression) => self
                .lower_expression(
                    interner,
                    &expression.expression,
                    places,
                    operations,
                    unsupported,
                    true,
                ),
            oxc_ast::ast::AssignmentTarget::TSTypeAssertion(expression) => self.lower_expression(
                interner,
                &expression.expression,
                places,
                operations,
                unsupported,
                true,
            ),
            oxc_ast::ast::AssignmentTarget::ObjectAssignmentTarget(_)
            | oxc_ast::ast::AssignmentTarget::ArrayAssignmentTarget(_) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    target.span(),
                    "complex destructuring",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                None
            }
        }
    }

    fn simple_assignment_target_shape(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        target: &oxc_ast::ast::SimpleAssignmentTarget<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) -> Option<PlaceShape> {
        match target {
            oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier) => {
                self.lower_identifier(identifier.name.as_str(), places, true)
            }
            oxc_ast::ast::SimpleAssignmentTarget::StaticMemberExpression(member) => {
                self.lower_static_member(interner, member, places, operations, unsupported, true)
            }
            oxc_ast::ast::SimpleAssignmentTarget::ComputedMemberExpression(member) => {
                self.lower_computed_member(interner, member, places, operations, unsupported, true)
            }
            oxc_ast::ast::SimpleAssignmentTarget::PrivateFieldExpression(private) => {
                self.lower_private_field(interner, private, places, operations, unsupported, true)
            }
            oxc_ast::ast::SimpleAssignmentTarget::TSAsExpression(expression) => self
                .lower_expression(
                    interner,
                    &expression.expression,
                    places,
                    operations,
                    unsupported,
                    true,
                ),
            oxc_ast::ast::SimpleAssignmentTarget::TSSatisfiesExpression(expression) => self
                .lower_expression(
                    interner,
                    &expression.expression,
                    places,
                    operations,
                    unsupported,
                    true,
                ),
            oxc_ast::ast::SimpleAssignmentTarget::TSNonNullExpression(expression) => self
                .lower_expression(
                    interner,
                    &expression.expression,
                    places,
                    operations,
                    unsupported,
                    true,
                ),
            oxc_ast::ast::SimpleAssignmentTarget::TSTypeAssertion(expression) => self
                .lower_expression(
                    interner,
                    &expression.expression,
                    places,
                    operations,
                    unsupported,
                    true,
                ),
        }
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

    fn temporary_shape(&self, places: &mut PlaceTableBuilder, span: oxc_span::Span) -> PlaceShape {
        self.temporary_shape_typed(
            places,
            span,
            TypeShape::Unknown {
                reason: "ts/js temporary".to_string(),
            },
        )
    }

    fn temporary_shape_typed(
        &self,
        places: &mut PlaceTableBuilder,
        span: oxc_span::Span,
        ty: TypeShape,
    ) -> PlaceShape {
        let root = PlaceRoot::Temporary {
            body: self.body,
            ordinal: span.start,
        };
        let key = self.insert_typed_place(
            places,
            root.clone(),
            Vec::new(),
            Some(ty),
            PlaceStatus::Partial,
        );
        PlaceShape {
            root,
            projections: Vec::new(),
            status: PlaceStatus::Partial,
            key,
        }
    }

    fn insert_temporary(
        &self,
        places: &mut PlaceTableBuilder,
        span: oxc_span::Span,
        status: PlaceStatus,
    ) -> String {
        self.insert_place(
            places,
            PlaceRoot::Temporary {
                body: self.body,
                ordinal: span.start,
            },
            Vec::new(),
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
                language: self.language,
                file: Some(self.file),
                function: Some(self.function),
                root,
                projections,
                status,
            },
            ty,
        )
    }

    fn push_assign(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        operations: &mut Vec<OperationDraft>,
        span: oxc_span::Span,
        place_key: String,
        value: ValueDraft,
        mode: AssignMode,
    ) {
        self.push_operation(
            interner,
            operations,
            span,
            OperationKindDraft::Assign {
                place_key,
                value,
                mode,
            },
            MirStatus::Partial,
        );
    }

    fn lower_call(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        call: &oxc_ast::ast::CallExpression<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) -> Option<String> {
        if call.optional {
            self.push_unsupported(
                interner,
                operations,
                unsupported,
                call.span,
                "optional chaining",
                (Vec::new(), ConservativeAction::HavocAffectedPlaces),
            );
        }
        if callee_text(&call.callee).as_deref() == Some("eval") {
            self.push_unsupported(
                interner,
                operations,
                unsupported,
                call.span,
                "eval",
                (Vec::new(), ConservativeAction::HavocAffectedPlaces),
            );
        }
        if callee_text(&call.callee).as_deref() == Some("require")
            && !matches!(call.arguments.first(), Some(Argument::StringLiteral(_)))
        {
            self.push_unsupported(
                interner,
                operations,
                unsupported,
                call.span,
                "dynamic CommonJS require",
                (Vec::new(), ConservativeAction::HavocAffectedPlaces),
            );
        }
        let span = normalized_call_expression_span(self.source, call);
        let site = call_site_for_span(span);
        let return_key = self.insert_place(
            places,
            PlaceRoot::CallReturn { call: site },
            Vec::new(),
            PlaceStatus::Partial,
        );
        self.lower_expression(
            interner,
            &call.callee,
            places,
            operations,
            unsupported,
            false,
        );
        let callee = callee_text(&call.callee).map_or_else(
            || ValueDraft::Unknown {
                evidence: "call".to_string(),
            },
            |evidence| ValueDraft::Unknown { evidence },
        );
        let mut arguments = Vec::new();
        for argument in &call.arguments {
            if let Some(shape) =
                self.argument_shape(interner, argument, places, operations, unsupported)
            {
                arguments.push(shape.key);
            }
        }
        self.push_operation(
            interner,
            operations,
            span,
            OperationKindDraft::Call {
                site,
                callee,
                arguments,
                return_place_key: return_key.clone(),
                unwind: false,
            },
            MirStatus::Partial,
        );
        Some(return_key)
    }

    /// A tagged template `tag`…${e1}${e2}`…`` desugars to a call
    /// `tag(strings, e1, e2)`. Lowering it as a real call (rather than the prior
    /// `unsupported` stub) gives it a call site so the call-graph resolves the
    /// tag and the interpolation expressions flow to the tag's parameters
    /// (offset by the implicit `strings` array at index 0).
    fn lower_tagged_template_expression(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        tagged: &oxc_ast::ast::TaggedTemplateExpression<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) -> Option<PlaceShape> {
        let span = normalized_tagged_template_span(tagged);
        let site = call_site_for_span(span);
        let return_key = self.insert_place(
            places,
            PlaceRoot::CallReturn { call: site },
            Vec::new(),
            PlaceStatus::Partial,
        );
        self.lower_expression(
            interner,
            &tagged.tag,
            places,
            operations,
            unsupported,
            false,
        );
        let callee = callee_text(&tagged.tag).map_or_else(
            || ValueDraft::Unknown {
                evidence: "tagged template".to_string(),
            },
            |evidence| ValueDraft::Unknown { evidence },
        );
        // The strings array occupies argument slot 0; interpolations follow.
        let mut arguments = vec![self.temporary_shape(places, span).key];
        for expression in &tagged.quasi.expressions {
            if let Some(shape) =
                self.lower_expression(interner, expression, places, operations, unsupported, false)
            {
                arguments.push(shape.key);
            }
        }
        self.push_operation(
            interner,
            operations,
            span,
            OperationKindDraft::Call {
                site,
                callee,
                arguments,
                return_place_key: return_key.clone(),
                unwind: false,
            },
            MirStatus::Partial,
        );
        Some(PlaceShape {
            root: PlaceRoot::CallReturn { call: site },
            projections: Vec::new(),
            status: PlaceStatus::Partial,
            key: return_key,
        })
    }

    fn lower_new_expression(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        expression: &oxc_ast::ast::NewExpression<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) -> Option<String> {
        if callee_text(&expression.callee).as_deref() == Some("Proxy") {
            self.push_unsupported(
                interner,
                operations,
                unsupported,
                expression.span,
                "Proxy",
                (Vec::new(), ConservativeAction::HavocAffectedPlaces),
            );
        }
        let span = normalized_new_expression_span(self.source, expression);
        let site = call_site_for_span(span);
        let return_key = self.insert_place(
            places,
            PlaceRoot::CallReturn { call: site },
            Vec::new(),
            PlaceStatus::Partial,
        );
        self.lower_expression(
            interner,
            &expression.callee,
            places,
            operations,
            unsupported,
            false,
        );
        let callee = callee_text(&expression.callee).map_or_else(
            || ValueDraft::Unknown {
                evidence: "new".to_string(),
            },
            |evidence| ValueDraft::Unknown {
                evidence: format!("new {evidence}"),
            },
        );
        let mut arguments = Vec::new();
        for argument in &expression.arguments {
            if let Some(shape) =
                self.argument_shape(interner, argument, places, operations, unsupported)
            {
                arguments.push(shape.key);
            }
        }
        self.push_operation(
            interner,
            operations,
            span,
            OperationKindDraft::Call {
                site,
                callee,
                arguments,
                return_place_key: return_key.clone(),
                unwind: false,
            },
            MirStatus::Partial,
        );
        Some(return_key)
    }

    fn argument_shape(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        argument: &Argument<'_>,
        places: &mut PlaceTableBuilder,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
    ) -> Option<PlaceShape> {
        match argument {
            Argument::SpreadElement(spread) => {
                self.push_unsupported(
                    interner,
                    operations,
                    unsupported,
                    spread.span,
                    "spread",
                    (Vec::new(), ConservativeAction::HavocAffectedPlaces),
                );
                self.lower_expression(
                    interner,
                    &spread.argument,
                    places,
                    operations,
                    unsupported,
                    false,
                )
            }
            _ => self.lower_expression(
                interner,
                argument.to_expression(),
                places,
                operations,
                unsupported,
                false,
            ),
        }
    }

    /// Binds the place a conditional branch tests, plus any nil comparison.
    ///
    /// `predicate_place` is consumed as a truthiness (or, with a nil test,
    /// nilness) assumption on the branch's `true` / `false` edges, so it may
    /// only be bound where the `true` edge really does prove the place truthy.
    fn bind_branch_predicate(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        places: &mut PlaceTableBuilder,
        operations: &mut [OperationDraft],
        branch: MirOpId,
        expression: &Expression<'_>,
        lowered: Option<PlaceShape>,
    ) {
        let nil_test = ts_nil_test(expression);
        let predicate_place_key = if let Some(operand) = ts_null_operand(expression) {
            self.lower_expression(
                interner,
                operand,
                places,
                &mut Vec::new(),
                &mut Vec::new(),
                false,
            )
            .map(|shape| shape.key)
        } else {
            lowered.map(|shape| shape.key)
        };
        set_branch_predicate(operations, branch, predicate_place_key, nil_test);
    }

    fn push_branch(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        operations: &mut Vec<OperationDraft>,
        span: oxc_span::Span,
        shape: ControlShape,
    ) -> MirOpId {
        self.push_branch_with_predicate(interner, operations, span, shape, None, None)
    }

    fn push_branch_with_predicate(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        operations: &mut Vec<OperationDraft>,
        span: oxc_span::Span,
        shape: ControlShape,
        predicate_place_key: Option<String>,
        nil_test: Option<BranchNilTest>,
    ) -> MirOpId {
        let id = MirOpId(operations.len() as u64);
        self.push_operation(
            interner,
            operations,
            span,
            OperationKindDraft::Branch {
                predicate: MirPredicateId(span.start as u64),
                predicate_place_key,
                nil_test,
                shape,
                region: BranchRegion::default(),
            },
            MirStatus::Partial,
        );
        id
    }

    fn push_unsupported(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        operations: &mut Vec<OperationDraft>,
        unsupported: &mut Vec<UnsupportedDraft>,
        span: oxc_span::Span,
        construct: &str,
        pair: (Vec<String>, ConservativeAction),
    ) {
        let (affected_place_keys, action) = pair;
        let unsupported_id = UnsupportedId(unsupported.len() as u64);
        let operation_id = MirOpId(operations.len() as u64);
        let source_evidence = source_text(self.source, span)
            .unwrap_or(construct)
            .to_string();
        let span = span_from_oxc(self.source_file, span);
        unsupported.push(UnsupportedDraft::new(UnsupportedDraftInput {
            id: unsupported_id,
            body: Some(self.body),
            operation: Some(operation_id),
            language: self.language,
            file_key: self.stable_context.file_key().to_string(),
            file: self.file,
            span: span.clone(),
            construct: construct.to_string(),
            source_evidence,
            affected_place_keys,
            affected_domains: unsupported_domains_for(construct),
            conservative_action: action,
        }));
        operations.push(OperationDraft::new(
            interner,
            operation_id,
            self.body,
            self.stable_context.body_key(),
            span.start_byte,
            span,
            (
                OperationKindDraft::Unsupported { unsupported_id },
                MirStatus::Unsupported,
            ),
        ));
    }

    fn push_operation(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        operations: &mut Vec<OperationDraft>,
        span: oxc_span::Span,
        kind: OperationKindDraft,
        status: MirStatus,
    ) {
        let id = MirOpId(operations.len() as u64);
        operations.push(OperationDraft::new(
            interner,
            id,
            self.body,
            self.stable_context.body_key(),
            span.start,
            span_from_oxc(self.source_file, span),
            (kind, status),
        ));
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
            OperationKindDraft::Throw { value } => ControlEffectKind::Throw {
                value: value.as_ref().map(|value| value.to_value(place_ids)),
            },
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
    StorageLive {
        place_key: String,
    },
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
    Throw {
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
            Self::StorageLive { place_key } => Some(MirOperationKind::StorageLive {
                place: *place_ids.get(place_key)?,
            }),
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
            Self::Throw { .. } | Self::Suspend { .. } => None,
            Self::Unsupported { unsupported_id } => Some(MirOperationKind::Unsupported {
                unsupported: *unsupported_id,
            }),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::StorageLive { .. } => "storage-live",
            Self::Assign { .. } => "assign",
            Self::Read { .. } => "read",
            Self::Branch { .. } => "branch",
            Self::Call { .. } => "call",
            Self::Return { .. } => "return",
            Self::Throw { .. } => "throw",
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
            Self::StorageLive { place_key } => vec![place_key.clone()],
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
            Self::Throw { value } | Self::Suspend { value, .. } => {
                value.as_ref().map_or_else(Vec::new, ValueDraft::place_keys)
            }
            Self::Branch { .. } | Self::Unsupported { .. } => Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
enum ValueDraft {
    PlaceKey(String),
    Literal {
        value: String,
    },
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
            Self::PlaceKey(key) => place_ids.get(key).copied().map_or_else(
                || MirValue::Unknown {
                    evidence: key.clone(),
                },
                MirValue::Place,
            ),
            Self::Literal { value } if value.trim().is_empty() => MirValue::Unknown {
                evidence: "empty literal lowering".to_string(),
            },
            Self::Literal { value } => MirValue::Literal {
                value: value.trim().to_string(),
            },
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
    language: Language,
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
    language: Language,
    file_key: String,
    file: FileId,
    span: Span,
    construct: String,
    source_evidence: S,
    affected_place_keys: Vec<String>,
    affected_domains: Vec<UnsupportedDomain>,
    conservative_action: ConservativeAction,
}

impl UnsupportedDraft {
    fn new<S>(input: UnsupportedDraftInput<S>) -> Self
    where
        S: Into<String>,
    {
        Self {
            id: input.id,
            body: input.body,
            operation: input.operation,
            language: input.language,
            file_key: input.file_key,
            file: input.file,
            span: input.span,
            construct: input.construct,
            source_evidence: input.source_evidence.into().trim().to_string(),
            affected_place_keys: input.affected_place_keys,
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
            language: self.language,
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

fn unsupported_domains_for(construct: &str) -> Vec<UnsupportedDomain> {
    match construct {
        "eval" | "with" | "Proxy" | "dynamic CommonJS require" => vec![
            UnsupportedDomain::Mir,
            UnsupportedDomain::Calls,
            UnsupportedDomain::Domains,
            UnsupportedDomain::Aliases,
            UnsupportedDomain::DataFlow,
        ],
        "dynamic property key"
        | "optional chaining"
        | "switch"
        | "try"
        | "for initializer"
        | "for left binding"
        | "for-in"
        | "for-of"
        | "do while"
        | "break"
        | "continue"
        | "labeled statement"
        | "debugger"
        | "catch destructuring"
        | "getter"
        | "setter"
        | "complex destructuring"
        | "spread"
        | "rest"
        | "JSX callback scheduling"
        | "tagged template"
        | "dynamic import"
        | "private field"
        | "private in"
        | "class expression"
        | "v8 intrinsic"
        | "unhandled expression" => vec![
            UnsupportedDomain::Mir,
            UnsupportedDomain::Cfg,
            UnsupportedDomain::Domains,
            UnsupportedDomain::DataFlow,
        ],
        "parser recovery" => vec![
            UnsupportedDomain::Mir,
            UnsupportedDomain::Cfg,
            UnsupportedDomain::Calls,
            UnsupportedDomain::Domains,
            UnsupportedDomain::DataFlow,
            UnsupportedDomain::Aliases,
            UnsupportedDomain::Summaries,
        ],
        _ => vec![UnsupportedDomain::Mir],
    }
}

fn operation_stable_key(
    body_stable_key: &str,
    ordinal: u32,
    span: &Span,
    kind: &OperationKindDraft,
) -> String {
    let mut parts = vec![
        ("language", "ts-js".to_string()),
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
            ("language", language_label(draft.language).to_string()),
            ("file", draft.file_key.clone()),
            ("construct", draft.construct.clone()),
            ("start_byte", draft.span.start_byte.to_string()),
            ("end_byte", draft.span.end_byte.to_string()),
            ("evidence", draft.source_evidence.clone()),
        ],
    )
    .into_string()
}

fn index_projection(expression: &Expression<'_>) -> PlaceProjection {
    match expression {
        Expression::StringLiteral(literal) => {
            PlaceProjection::IndexKnown(literal.value.to_string())
        }
        Expression::NumericLiteral(literal) => PlaceProjection::IndexKnown(
            literal
                .raw
                .as_ref()
                .map_or_else(|| literal.value.to_string(), ToString::to_string),
        ),
        _ => PlaceProjection::IndexUnknown {
            evidence: expression_text(expression).unwrap_or_else(|| "dynamic".to_string()),
        },
    }
}

fn expression_text(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier.name.to_string()),
        _ => None,
    }
}

fn callee_text(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier.name.to_string()),
        // `this` as a callee base: `this.m()` yields evidence `this.m`. Its base
        // segment `this` is lower-case, so `is_static_member_evidence` classifies it
        // as a `Member` (not `StaticMember`) callee — the direct resolver does NOT
        // name-match `Member`-kind callees (`lexical_callee_name` requires
        // `StaticMember`), so this adds no spurious by-name edges. The value-flow
        // this-method resolvers (and the points-to heap) DO produce precise `this.m`
        // edges, which now line up with a `Member` call site instead of falling back
        // to the bare `"call"` evidence that no resolved edge could match.
        Expression::ThisExpression(_) => Some("this".to_string()),
        Expression::StaticMemberExpression(member) => {
            let object = callee_text(&member.object)?;
            Some(format!("{}.{}", object, member.property.name))
        }
        Expression::ArrowFunctionExpression(function) => Some(anonymous_callable_name(
            function.span.start,
            function.span.end,
        )),
        Expression::FunctionExpression(function) => Some(anonymous_callable_name(
            function.span.start,
            function.span.end,
        )),
        Expression::ParenthesizedExpression(parenthesized) => {
            callee_text(&parenthesized.expression)
        }
        _ => None,
    }
}

fn source_text(source: &str, span: oxc_span::Span) -> Option<&str> {
    source.get(span.start as usize..span.end as usize)
}

fn matching_function<'db>(
    db: &'db impl AnalysisHost,
    file: FileId,
    language: Language,
    name: &str,
    span: &Span,
) -> Option<&'db FunctionFact> {
    db.functions().iter().find(|function| {
        function.file == file
            && function.language == language
            && function.name == name
            && span_contains(span, &function.span)
    })
}

fn matching_module_function(
    db: &impl AnalysisHost,
    file: FileId,
    language: Language,
) -> Option<&FunctionFact> {
    db.functions().iter().find(|function| {
        function.file == file
            && function.language == language
            && is_synthetic_ts_js_module_function(function)
    })
}

fn enclosing_function<'db>(
    db: &'db impl AnalysisHost,
    file: FileId,
    language: Language,
    span: &Span,
) -> Option<&'db FunctionFact> {
    db.functions()
        .iter()
        .filter(|function| {
            function.file == file
                && function.language == language
                && span_contains(&function.span, span)
                && !is_synthetic_ts_js_module_function(function)
        })
        .min_by_key(|function| function.span.end_byte - function.span.start_byte)
}

fn span_contains(outer: &Span, inner: &Span) -> bool {
    outer.start_byte <= inner.start_byte && outer.end_byte >= inner.end_byte
}

fn owner_stable_key(file: &SourceFile, function: &FunctionFact) -> String {
    semantic_stable_key(
        FactFamily::Function,
        &[
            ("language", language_label(file.language).to_string()),
            ("path", file.relative_path.clone()),
            ("function", function.name.clone()),
            ("start_byte", function.span.start_byte.to_string()),
            ("end_byte", function.span.end_byte.to_string()),
        ],
    )
    .into_string()
}

fn span_from_oxc(file: &SourceFile, span: oxc_span::Span) -> Span {
    file.span_from_byte_range(span.start as usize, span.end as usize)
}

fn call_site_for_span(span: oxc_span::Span) -> CallSiteId {
    CallSiteId(((span.start as u64) << 32) | span.end as u64)
}

fn language_label(language: Language) -> &'static str {
    match language {
        Language::TypeScript => "typescript",
        Language::Tsx => "tsx",
        Language::JavaScript => "javascript",
        Language::Jsx => "jsx",
        Language::Go | Language::Unknown => "unknown",
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod places {
    use super::*;
    use crate::analysis_api::TS_JS_MODULE_FUNCTION_NAME;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::places::{PlaceProjection, PlaceRoot};
    use crate::internal_core::Language;
    use std::path::PathBuf;

    fn lower(path: &str, source: &str) -> (MirOutput, crate::internal_core::StableKeyInterner) {
        let mut db = LocalAnalysisDb::new();
        db.add_file(PathBuf::from(path), path.to_string(), source.to_string());
        let diagnostics = crate::ts::analyze_with_options(
            &mut db,
            &crate::analysis_api::DisabledAnalysisCache,
            "",
            "",
            false,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let output = lower_ts_mir(&db);
        (output, db.stable_key_interner())
    }

    fn lower_allowing_parser_diagnostics(path: &str, source: &str) -> (LocalAnalysisDb, MirOutput) {
        let mut db = LocalAnalysisDb::new();
        db.add_file(PathBuf::from(path), path.to_string(), source.to_string());
        let _diagnostics = crate::ts::analyze_with_options(
            &mut db,
            &crate::analysis_api::DisabledAnalysisCache,
            "",
            "",
            false,
        );
        let output = lower_ts_mir(&db);
        (db, output)
    }

    #[test]
    fn catastrophic_and_recoverable_fixtures_both_record_parser_recovery() {
        let (_db, recoverable) =
            lower_allowing_parser_diagnostics("recoverable.tsx", "const x = <div></span>;");
        assert!(
            recoverable
                .unsupported
                .iter()
                .any(|row| row.construct == crate::ts::PARSER_RECOVERY_CONSTRUCT),
            "recoverable syntax error must record parser recovery: {:?}",
            recoverable.unsupported
        );

        let (_db, catastrophic) = lower_allowing_parser_diagnostics(
            "catastrophic.ts",
            "import x from \"./x\";\nconst value = ;",
        );
        assert!(
            catastrophic
                .unsupported
                .iter()
                .any(|row| row.construct == crate::ts::PARSER_RECOVERY_CONSTRUCT),
            "catastrophic syntax error must record parser recovery: {:?}",
            catastrophic.unsupported
        );
    }

    #[test]
    fn ts_function_places_include_parameters_locals_globals_properties_and_indexes() {
        let source = r#"
export function render(user, index) {
  const token = user.tokens[index];
  window.value = token;
  return token;
}
"#;
        let (first, first_interner) = lower("src/render.ts", source);
        let (second, second_interner) = lower("src/render.ts", source);

        assert_eq!(first.bodies.len(), 2);
        assert!(first.bodies.iter().any(|body| {
            first_interner
                .resolve(body.owner_stable_key)
                .contains(TS_JS_MODULE_FUNCTION_NAME)
        }));
        assert!(first.bodies.iter().any(|body| {
            first_interner
                .resolve(body.owner_stable_key)
                .contains("render")
        }));
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
            PlaceRoot::Unknown { evidence } if evidence == "window"
        )));
        assert!(first.places.iter().any(|place| {
            matches!(&place.root, PlaceRoot::Parameter { name: Some(name), .. } if name == "user")
                && place
                    .projections
                    .contains(&PlaceProjection::Property("tokens".to_string()))
                && place.projections.iter().any(|projection| {
                    matches!(projection, PlaceProjection::IndexUnknown { evidence } if evidence == "index")
                })
        }));
    }

    #[test]
    fn ts_lowerer_constructs_structured_values_closure_captures_and_place_types() {
        let (output, _interner) = lower(
            "src/values.ts",
            r#"
export function make(seed: number) {
  const count = 1;
  const callback = () => seed + count;
  const record = { value: count };
  return callback;
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
                    kind: MirAggregateKind::Object,
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
    fn ts_arrow_functions_and_class_methods_join_existing_function_facts() {
        let source = r#"
const render = (user) => user.name;

class View {
  render(user) {
    return user.name;
  }
}
"#;
        let mut db = LocalAnalysisDb::new();
        db.add_file(
            PathBuf::from("src/view.tsx"),
            "src/view.tsx".to_string(),
            source.to_string(),
        );
        let diagnostics = crate::ts::analyze_with_options(
            &mut db,
            &crate::analysis_api::DisabledAnalysisCache,
            "",
            "",
            false,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        let arrow = db
            .functions()
            .iter()
            .find(|function| function.name == "render")
            .expect("arrow function fact should use variable name");
        let method = db
            .functions()
            .iter()
            .find(|function| function.name == "View.render")
            .expect("method function fact should use class-qualified name");

        let output = lower_ts_mir(&db);
        assert!(output.bodies.iter().any(|body| {
            body.function == arrow.id
                && body.language == Language::Tsx
                && db
                    .resolve_stable_key(body.owner_stable_key)
                    .contains("render")
        }));
        assert!(output.bodies.iter().any(|body| {
            body.function == method.id
                && body.language == Language::Tsx
                && db
                    .resolve_stable_key(body.owner_stable_key)
                    .contains("View.render")
        }));
        assert!(output.places.iter().any(|place| matches!(
            &place.root,
            PlaceRoot::Parameter {
                function,
                index: 0,
                name: Some(name),
            } if *function == arrow.id && name == "user"
        )));
        assert!(output.places.iter().any(|place| matches!(
            &place.root,
            PlaceRoot::Parameter {
                function,
                index: 0,
                name: Some(name),
            } if *function == method.id && name == "user"
        )));
    }

    #[test]
    fn ts_mir_place_rows_do_not_carry_oxc_ast_debug_evidence() {
        let (output, _) = lower(
            "src/render.ts",
            r#"
export function render(user) {
  const token = user.token;
  return token;
}
"#,
        );
        let debug = format!("{output:#?}");

        assert!(!debug.contains("oxc_ast"));
        assert!(!debug.contains("oxc_span::Span"));
        assert!(!debug.contains("Program<'_"));
        assert!(!debug.contains("Expression<'_"));
        assert!(!debug.contains("Statement<'_"));
        assert!(!debug.contains("FunctionDeclaration"));
        assert!(!debug.contains("ArrowFunctionExpression"));
        assert!(!debug.contains("ClassElement"));
    }
}

#[cfg(test)]
mod operations {
    use super::*;
    use crate::analysis_api::TS_JS_MODULE_FUNCTION_NAME;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::mir_op::{
        AssignMode, ConservativeAction, MirOperationKind, UnsupportedDomain,
    };
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn lower(path: &str, source: &str) -> (MirOutput, crate::internal_core::StableKeyInterner) {
        let mut db = LocalAnalysisDb::new();
        db.add_file(PathBuf::from(path), path.to_string(), source.to_string());
        let diagnostics = crate::ts::analyze_with_options(
            &mut db,
            &crate::analysis_api::DisabledAnalysisCache,
            "",
            "",
            false,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let output = lower_ts_mir(&db);
        (output, db.stable_key_interner())
    }

    #[test]
    fn ts_statement_lowering_emits_assignment_modes_and_control_shapes() {
        let (output, _) = lower(
            "src/flow.ts",
            r#"
export function flow(user, index, enabled) {
  const token = user.tokens[index];
  let count = 0;
  count = index;
  user.tokens[index] = token;
  ({ token } = user);
  if (enabled && (token ?? false)) { count = count + 1; }
  const label = enabled ? token : "none";
  for (; count < 10; count++) { label; }
  return token;
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
        assert!(
            output
                .unsupported
                .iter()
                .any(|row| row.construct.contains("destructur") && row.is_complete())
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
                .any(|terminator| matches!(terminator.kind, MirTerminatorKind::Return { .. }))
        );
    }

    #[test]
    fn ts_branch_operations_bind_if_switch_ternary_and_logical_condition_places() {
        let (output, interner) = lower(
            "src/branches.ts",
            r#"
export function flow(flag, other, kind) {
  if (flag) {}
  switch (kind) { case other: break; default: break; }
  const selected = flag ? kind : other;
  const both = flag && other;
  const either = flag || other;
}
"#,
        );

        let predicates = output
            .operations
            .iter()
            .filter_map(|operation| match operation.kind {
                MirOperationKind::Branch {
                    predicate_place, ..
                } => Some(predicate_place),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(predicates.len(), 6, "{output:#?}");

        // The `case` branch stays unbound: a case tests `discriminant === test`,
        // not the truthiness of the case expression.
        let keys = predicates
            .iter()
            .copied()
            .flatten()
            .map(|predicate| {
                let place = output
                    .places
                    .iter()
                    .find(|place| place.id == predicate)
                    .expect("predicate place should survive remapping");
                interner.resolve(place.stable_key).to_string()
            })
            .collect::<Vec<_>>();
        // `if`, the ternary test, and both logical left operands bind `flag`;
        // the switch binds its `kind` discriminant. `other` is only ever a case
        // test or a right operand, so no branch claims it as a predicate.
        let mut names = keys
            .iter()
            .map(|key| {
                key.rsplit_once("root_name=")
                    .expect("place key carries a root name")
                    .1
                    .to_string()
            })
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            vec!["4:flag", "4:flag", "4:flag", "4:flag", "4:kind"],
            "{keys:?}"
        );
    }

    #[test]
    fn ts_loop_and_coalesce_branches_carry_no_condition_assumption() {
        let (output, _) = lower(
            "src/loops.ts",
            r#"
export function flow(user, items, ready) {
  while (user !== null) { break; }
  do { break; } while (ready);
  for (let index = 0; index < items.length; index++) { break; }
  const fallback = user ?? items;
}
"#,
        );

        // Every branch in this body is a loop branch or the `??` branch.
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
    fn ts_call_operations_are_shape_evidence_with_deterministic_call_sites() {
        let source = r#"
export function flow(token, count) {
  const result = helper(token, count);
  return result;
}
"#;
        let (first, first_interner) = lower("src/calls.ts", source);
        let (second, second_interner) = lower("src/calls.ts", source);

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
                    arguments.len(),
                    *return_place,
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
        assert_eq!(
            first_calls
                .iter()
                .map(|(key, site, _, arguments, return_place)| {
                    (key.clone(), *site, *arguments, *return_place)
                })
                .collect::<Vec<_>>(),
            second_calls
        );
        assert!(
            first_calls[0].2
                != &MirValue::Unknown {
                    evidence: "direct target".to_string()
                }
        );
    }

    #[test]
    fn ts_nested_same_start_calls_get_distinct_call_site_ids() {
        let source = r#"
const k1 = {
  a2() {},
  a4() { return this; },
};
k1.a4().a2();
"#;
        let (output, _) = lower("src/chained.js", source);
        let chain_start = source.find("k1.a4()").expect("chained call source exists") as u32;
        let same_start_calls = output
            .operations
            .iter()
            .filter_map(|operation| match &operation.kind {
                MirOperationKind::Call { site, .. } if operation.span.start_byte == chain_start => {
                    Some((*site, operation.span.end_byte))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let unique_sites = same_start_calls
            .iter()
            .map(|(site, _)| *site)
            .collect::<BTreeSet<_>>();

        assert_eq!(same_start_calls.len(), 2);
        assert_eq!(unique_sites.len(), 2);
    }

    #[test]
    fn ts_call_operation_spans_match_jelly_parenthesized_call_shapes() {
        let source = r#"
(function(){})();
((f))();
(f());
((new f()));
function f() {}
"#;
        let (output, _) = lower("src/call_shapes.js", source);
        let span_texts = output
            .operations
            .iter()
            .filter_map(|operation| match operation.kind {
                MirOperationKind::Call { .. } => Some(
                    &source[operation.span.start_byte as usize..operation.span.end_byte as usize],
                ),
                _ => None,
            })
            .collect::<BTreeSet<_>>();

        for expected in ["function(){})()", "(f))()", "(f())", "(new f())"] {
            assert!(span_texts.contains(expected), "missing span {expected:?}");
        }
    }

    #[test]
    fn ts_module_body_lowering_emits_top_level_calls() {
        let (output, interner) = lower(
            "src/main.ts",
            r#"
function boot() {
  return 1;
}

boot();
"#,
        );
        let module = output
            .bodies
            .iter()
            .find(|body| {
                interner
                    .resolve(body.owner_stable_key)
                    .contains(TS_JS_MODULE_FUNCTION_NAME)
            })
            .expect("module body should be lowered");
        let module_boot_calls = output
            .operations
            .iter()
            .filter(|operation| operation.body == module.id)
            .filter(|operation| {
                matches!(
                    &operation.kind,
                    MirOperationKind::Call {
                        callee: MirValue::Unknown { evidence },
                        ..
                    } if evidence == "boot"
                )
            })
            .count();

        assert_eq!(module_boot_calls, 1);
    }

    #[test]
    fn ts_new_expression_lowers_constructor_call() {
        let (output, _) = lower(
            "src/new.ts",
            r#"
function Widget(value) {
  this.value = value;
}

new Widget(1);
"#,
        );

        let constructor_calls = output
            .operations
            .iter()
            .filter(|operation| {
                matches!(
                    &operation.kind,
                    MirOperationKind::Call {
                        callee: MirValue::Unknown { evidence },
                        arguments,
                        ..
                    } if evidence == "new Widget" && arguments.len() == 1
                )
            })
            .count();

        assert_eq!(constructor_calls, 1);
    }

    #[test]
    fn ts_iife_callees_use_anonymous_callable_identity() {
        let (output, interner) = lower(
            "src/iife.ts",
            r#"
(function(value) {
  return value;
})(1);

(() => helper())();
"#,
        );

        let anonymous_bodies = output
            .bodies
            .iter()
            .filter(|body| {
                interner
                    .resolve(body.owner_stable_key)
                    .contains("<polint:anonymous:")
            })
            .count();
        let anonymous_calls = output
            .operations
            .iter()
            .filter(|operation| {
                matches!(
                    &operation.kind,
                    MirOperationKind::Call {
                        callee: MirValue::Unknown { evidence },
                        ..
                    } if evidence.starts_with("<polint:anonymous:")
                )
            })
            .count();

        assert_eq!(anonymous_bodies, 2);
        assert_eq!(anonymous_calls, 2);
    }

    #[test]
    fn ts_unsupported_semantics_are_structured_and_conservative() {
        let (output, _) = lower(
            "src/dynamic.tsx",
            r#"
export async function flow(obj, key, promise, value) {
  eval("value");
  with (obj) { value = key; }
  const proxy = new Proxy(obj, {});
  const dynamic = obj[key];
  const maybe = obj?.value;
  await promise;
  const { nested: { name } } = obj;
  const clone = { ...obj };
  const mod = require(key);
  return <Button onClick={() => helper(value)} />;
}
"#,
        );

        for construct in [
            "eval",
            "with",
            "Proxy",
            "dynamic property key",
            "optional chaining",
            "complex destructuring",
            "spread",
            "dynamic CommonJS require",
            "JSX callback scheduling",
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
                kind: SuspendKind::Await,
                ..
            }
        )));
    }

    #[test]
    fn ts_nested_expression_lowering_preserves_calls_and_argument_places() {
        let (output, _) = lower(
            "src/nested.ts",
            r#"
export function nested(a, b, tag) {
  const computed = a + helper(b);
  const templated = `${computed}:${helper(a)}`;
  const tagged = tag`${helper(b)}`;
  const inverted = !helper(computed);
  const sequenced = (helper(a), helper(b));
  const invoked = api[helper(tag)](helper(a));
  return templated;
}
"#,
        );

        let helper_calls = output
            .operations
            .iter()
            .filter_map(|operation| match &operation.kind {
                MirOperationKind::Call {
                    callee: MirValue::Unknown { evidence },
                    arguments,
                    ..
                } if evidence == "helper" => Some(arguments.len()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(helper_calls.len(), 8);
        assert!(
            helper_calls
                .iter()
                .all(|argument_count| *argument_count == 1)
        );
        // A tagged template `tag`${helper(b)}`` now lowers to a real call
        // `tag(strings, helper(b))` (two arguments: the implicit strings array
        // plus the single interpolation) rather than an unsupported construct.
        let tag_call_arguments =
            output
                .operations
                .iter()
                .find_map(|operation| match &operation.kind {
                    MirOperationKind::Call {
                        callee: MirValue::Unknown { evidence },
                        arguments,
                        ..
                    } if evidence == "tag" => Some(arguments.len()),
                    _ => None,
                });
        assert_eq!(tag_call_arguments, Some(2));
        assert!(
            !output
                .unsupported
                .iter()
                .any(|row| row.construct == "tagged template")
        );
    }

    #[test]
    fn ts_control_statement_edges_are_explicit_and_still_lower_nested_calls() {
        let (output, _) = lower(
            "src/control.ts",
            r#"
export function control(kind, items, errors) {
  try {
    helper(kind);
  } catch (err) {
    recover(err);
  } finally {
    cleanup();
  }

  switch (kind) {
    case "a":
      helper(items[0]);
      break;
    default:
      fallback();
  }

  for (const item of items) {
    helper(item);
  }

  for (const key in errors) {
    helper(errors[key]);
    continue;
  }

  do {
    helper(kind);
  } while (kind);

  throw new Error(kind);
}
"#,
        );

        for construct in [
            "try", "switch", "break", "for-of", "for-in", "continue", "do while",
        ] {
            let row = output
                .unsupported
                .iter()
                .find(|row| row.construct == construct)
                .unwrap_or_else(|| panic!("missing unsupported row: {construct}"));
            assert!(row.is_complete());
            assert!(row.affected_domains.contains(&UnsupportedDomain::Mir));
        }

        let calls = output
            .operations
            .iter()
            .filter(|operation| matches!(operation.kind, MirOperationKind::Call { .. }))
            .count();

        assert!(
            calls >= 7,
            "expected nested calls to remain visible: {output:#?}"
        );
        assert!(
            output
                .terminators
                .iter()
                .any(|terminator| matches!(terminator.kind, MirTerminatorKind::Throw { .. }))
        );
    }

    #[test]
    fn ts_for_initializer_expression_is_explicitly_unsupported_not_silent() {
        let (output, _) = lower(
            "src/for-init.ts",
            r#"
export function loop(start, test, update) {
  for (helper(start); helper(test); helper(update)) {
    start;
  }
}
"#,
        );

        let row = output
            .unsupported
            .iter()
            .find(|row| row.construct == "for initializer")
            .expect("for initializer expression should be explicit unsupported evidence");

        assert!(row.is_complete());
        let helper_calls = output
            .operations
            .iter()
            .filter(|operation| {
                matches!(
                    &operation.kind,
                    MirOperationKind::Call {
                        callee: MirValue::Unknown { evidence },
                        ..
                    } if evidence == "helper"
                )
            })
            .count();
        assert_eq!(helper_calls, 3);
    }

    #[test]
    fn ts_for_in_and_for_of_existing_targets_record_left_writes() {
        let (output, _) = lower(
            "src/for-left.ts",
            r#"
export function loop(target, index, items, map) {
  for (target[index] of items) {
    helper(target[index]);
  }
  for (target.value in map) {
    helper(target.value);
  }
}
"#,
        );

        let projection_mutations = output
            .operations
            .iter()
            .filter(|operation| {
                matches!(
                    operation.kind,
                    MirOperationKind::Assign {
                        mode: AssignMode::ProjectionMutation,
                        ..
                    }
                )
            })
            .count();

        assert!(projection_mutations >= 2, "{output:#?}");
        for construct in ["for-of", "for-in", "for left binding"] {
            let row = output
                .unsupported
                .iter()
                .find(|row| row.construct == construct)
                .unwrap_or_else(|| panic!("missing unsupported row: {construct}"));
            assert!(row.is_complete());
            assert!(row.affected_domains.contains(&UnsupportedDomain::Mir));
        }
    }

    #[test]
    fn ts_array_object_and_jsx_expression_children_preserve_nested_calls() {
        let (output, _) = lower(
            "src/nested-values.tsx",
            r#"
export function view(a, b, items) {
  const data = [helper(a), ...items, { [helper(items)]: helper(b) }];
  return <Panel data={helper(data)}>{helper(a)}</Panel>;
}
"#,
        );

        let helper_calls = output
            .operations
            .iter()
            .filter(|operation| {
                matches!(
                    &operation.kind,
                    MirOperationKind::Call {
                        callee: MirValue::Unknown { evidence },
                        ..
                    } if evidence == "helper"
                )
            })
            .count();

        assert_eq!(helper_calls, 5);
        assert!(
            output
                .unsupported
                .iter()
                .any(|row| row.construct == "spread" && row.is_complete())
        );
    }

    #[test]
    fn ts_optional_private_and_dynamic_import_edges_are_explicit() {
        let (output, _) = lower(
            "src/private.tsx",
            r#"
class Box {
  #value;

  read(obj, modName) {
    const optional = obj?.value?.();
    const loaded = import(modName);
    obj.value! = modName;
    this.#value = obj;
    this.#value++;
    const node = <Panel data={helper(obj)}>{helper(modName)}</Panel>;
    return this.#value;
  }
}
"#,
        );

        for construct in ["optional chaining", "dynamic import", "private field"] {
            let row = output
                .unsupported
                .iter()
                .find(|row| row.construct == construct)
                .unwrap_or_else(|| panic!("missing unsupported row: {construct}"));
            assert!(row.is_complete());
        }

        let helper_calls = output
            .operations
            .iter()
            .filter(|operation| {
                matches!(
                    &operation.kind,
                    MirOperationKind::Call {
                        callee: MirValue::Unknown { evidence },
                        ..
                    } if evidence == "helper"
                )
            })
            .count();

        assert_eq!(helper_calls, 2);
        assert!(output.operations.iter().any(|operation| {
            matches!(
                operation.kind,
                MirOperationKind::Assign {
                    mode: AssignMode::ProjectionMutation,
                    ..
                }
            )
        }));
    }

    #[test]
    fn ts_optional_chaining_emits_unique_mir_and_unsupported_keys() {
        let (output, interner) = lower(
            "src/optional.ts",
            r#"
export function flow(options, data) {
  if (options?.enabled) {
    return data.model?.display_name;
  }
}
"#,
        );

        let operation_keys = output
            .operations
            .iter()
            .map(|operation| interner.resolve(operation.stable_key))
            .collect::<BTreeSet<_>>();
        let unsupported_keys = output
            .unsupported
            .iter()
            .map(|row| interner.resolve(row.stable_key))
            .collect::<BTreeSet<_>>();

        assert_eq!(operation_keys.len(), output.operations.len());
        assert_eq!(unsupported_keys.len(), output.unsupported.len());
    }
}
