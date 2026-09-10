use crate::analysis_api::FactFamily;
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::cfg::builder::CfgBuilder;
use crate::analysis_neutral::cfg::facts::{
    BasicBlockKind, CfgEdgeKind, CfgNodeKind, CfgPrecision, CfgStatus, ControlFlowAction,
    UnsupportedControlFlowFact,
};
use crate::analysis_neutral::cfg::ids::{CfgFunctionId, UnsupportedControlFlowId};
use crate::analysis_neutral::cfg::store::CfgOutput;
use crate::analysis_neutral::ids::{
    MirBodyId, MirOpId, MirStatementId, MirTerminatorId, UnsupportedId,
};
use crate::analysis_neutral::mir_body::{
    MirBlock, MirBlockId, MirStatement, MirStatus, MirTerminator, MirTerminatorKind,
};
use crate::analysis_neutral::mir_op::{
    ConservativeAction, MirOperation, MirOperationKind, UnsupportedDomain, UnsupportedPrecision,
    UnsupportedSemanticFact,
};
use crate::analysis_neutral::stable_key::semantic_stable_key;
use crate::internal_core::Language;
use std::collections::{BTreeMap, HashMap};

pub fn lower_cfg(db: &impl AnalysisHost) -> CfgOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut lowering = CfgLowering::new(db);
    lowering.lower(interner);
    lowering.finish(interner)
}

struct CfgLowering<'db, H: AnalysisHost + ?Sized> {
    db: &'db H,
    inputs: CfgInputs<'db>,
    builder: CfgBuilder,
    body_to_function: BTreeMap<MirBodyId, CfgFunctionId>,
}

/// Lookup-only ID indexes preserve the first matching row. Grouped inputs retain
/// source order, including repeated IDs, until the existing stable sort runs.
struct CfgInputs<'db> {
    blocks_by_body: BTreeMap<MirBodyId, Vec<&'db MirBlock>>,
    terminators_by_body: BTreeMap<MirBodyId, Vec<&'db MirTerminator>>,
    terminators: HashMap<MirTerminatorId, &'db MirTerminator>,
    statements: HashMap<MirStatementId, &'db MirStatement>,
    operations: HashMap<MirOpId, &'db MirOperation>,
    unsupported: HashMap<UnsupportedId, &'db UnsupportedSemanticFact>,
    unsupported_by_body: BTreeMap<MirBodyId, Vec<&'db UnsupportedSemanticFact>>,
}

impl<'db> CfgInputs<'db> {
    fn new(db: &'db (impl AnalysisHost + ?Sized)) -> Self {
        let mut inputs = Self {
            blocks_by_body: BTreeMap::new(),
            terminators_by_body: BTreeMap::new(),
            terminators: HashMap::with_capacity(db.mir_terminators().len()),
            statements: HashMap::with_capacity(db.mir_statements().len()),
            operations: HashMap::with_capacity(db.mir_operations().len()),
            unsupported: HashMap::with_capacity(db.unsupported_semantics().len()),
            unsupported_by_body: BTreeMap::new(),
        };
        for block in db.mir_blocks() {
            inputs
                .blocks_by_body
                .entry(block.body)
                .or_default()
                .push(block);
        }
        for terminator in db.mir_terminators() {
            inputs
                .terminators
                .entry(terminator.id)
                .or_insert(terminator);
            inputs
                .terminators_by_body
                .entry(terminator.body)
                .or_default()
                .push(terminator);
        }
        for statement in db.mir_statements() {
            inputs.statements.entry(statement.id).or_insert(statement);
        }
        for operation in db.mir_operations() {
            inputs.operations.entry(operation.id).or_insert(operation);
        }
        for row in db.unsupported_semantics() {
            inputs.unsupported.entry(row.id).or_insert(row);
            if let Some(body) = row.body
                && row.affected_domains.contains(&UnsupportedDomain::Cfg)
            {
                inputs
                    .unsupported_by_body
                    .entry(body)
                    .or_default()
                    .push(row);
            }
        }
        inputs
    }

    fn terminators_for_body(
        &self,
        body: MirBodyId,
    ) -> impl Iterator<Item = &'db MirTerminator> + '_ {
        self.terminators_by_body
            .get(&body)
            .into_iter()
            .flatten()
            .copied()
    }
}

impl<'db, H: AnalysisHost + ?Sized> CfgLowering<'db, H> {
    fn new(db: &'db H) -> Self {
        Self {
            db,
            inputs: CfgInputs::new(db),
            builder: CfgBuilder::new(),
            body_to_function: BTreeMap::new(),
        }
    }

    fn lower(&mut self, interner: &crate::internal_core::StableKeyInterner) {
        let mut bodies = self.db.mir_bodies().iter().collect::<Vec<_>>();
        bodies.sort_by(|body, other| interner.compare_canonical(body.stable_key, other.stable_key));

        for body in bodies {
            let has_exceptional_control =
                self.inputs.terminators_for_body(body.id).any(|terminator| {
                    matches!(
                        terminator.kind,
                        MirTerminatorKind::Throw { .. }
                            | MirTerminatorKind::Call {
                                unwind: Some(_),
                                ..
                            }
                    )
                }) || self.unsupported_for_body(body.id).any(|row| {
                    matches!(row.construct.as_str(), "try" | "parser recovery" | "ERROR")
                });
            let function = self
                .builder
                .start_function(interner, body, has_exceptional_control);
            self.body_to_function.insert(body.id, function);
            self.lower_body(interner, body.id);
            self.builder.finish_function();
        }
    }

    fn lower_body(&mut self, interner: &crate::internal_core::StableKeyInterner, body: MirBodyId) {
        let mut blocks = self
            .inputs
            .blocks_by_body
            .get(&body)
            .into_iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        blocks.sort_by(|block, other| {
            block
                .ordinal
                .cmp(&other.ordinal)
                .then_with(|| interner.compare_canonical(block.stable_key, other.stable_key))
        });
        let unwind_targets = self
            .inputs
            .terminators_for_body(body)
            .filter_map(|terminator| match terminator.kind {
                MirTerminatorKind::Throw { unwind, .. } => Some((unwind, CfgEdgeKind::Throw)),
                MirTerminatorKind::Call {
                    unwind: Some(unwind),
                    ..
                } => Some((unwind, CfgEdgeKind::ImplicitThrow)),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        if blocks.is_empty() {
            let current = self.builder.current_block();
            let exit = self.builder.normal_exit_block();
            self.builder
                .add_edge(interner, current, exit, CfgEdgeKind::Normal);
            return;
        }

        let entry = self.builder.current_block();
        let mut cfg_blocks = BTreeMap::new();
        for block in &blocks {
            let terminator = self
                .inputs
                .terminators
                .get(&block.terminator)
                .copied()
                .expect("MIR block terminator must exist");
            let kind = match terminator.kind {
                MirTerminatorKind::Branch { .. } | MirTerminatorKind::Switch { .. } => {
                    BasicBlockKind::Branch
                }
                MirTerminatorKind::Unreachable if !unwind_targets.contains_key(&block.id) => {
                    BasicBlockKind::Unreachable
                }
                _ => BasicBlockKind::StraightLine,
            };
            let cfg_block = self.builder.start_block(interner, kind);
            if matches!(terminator.kind, MirTerminatorKind::Unreachable)
                && !unwind_targets.contains_key(&block.id)
            {
                self.builder.mark_unreachable(cfg_block);
            }
            for (index, statement_id) in block.statements.iter().enumerate() {
                let statement = self
                    .inputs
                    .statements
                    .get(statement_id)
                    .copied()
                    .expect("MIR block statement must exist");
                let operation = self
                    .inputs
                    .operations
                    .get(&statement.operation)
                    .copied()
                    .expect("MIR statement operation must exist");
                let node_kind = if index + 1 == block.statements.len() {
                    match terminator.kind {
                        MirTerminatorKind::Branch { .. } | MirTerminatorKind::Switch { .. } => {
                            CfgNodeKind::Condition
                        }
                        MirTerminatorKind::Throw { .. } => CfgNodeKind::Throw,
                        MirTerminatorKind::Suspend {
                            kind: crate::analysis_neutral::mir_body::SuspendKind::Await,
                            ..
                        } => CfgNodeKind::Await,
                        MirTerminatorKind::Suspend {
                            kind: crate::analysis_neutral::mir_body::SuspendKind::Yield,
                            ..
                        } => CfgNodeKind::Yield,
                        _ => self.operation_node_kind(operation),
                    }
                } else {
                    self.operation_node_kind(operation)
                };
                self.builder.append_operation_node(
                    interner,
                    Some(operation),
                    node_kind,
                    Some(operation.span.clone()),
                );
            }
            if block.statements.is_empty() {
                let node_kind = match terminator.kind {
                    MirTerminatorKind::Throw { .. } => CfgNodeKind::Throw,
                    MirTerminatorKind::Suspend {
                        kind: crate::analysis_neutral::mir_body::SuspendKind::Await,
                        ..
                    } => CfgNodeKind::Await,
                    MirTerminatorKind::Suspend {
                        kind: crate::analysis_neutral::mir_body::SuspendKind::Yield,
                        ..
                    } => CfgNodeKind::Yield,
                    _ => CfgNodeKind::Synthetic,
                };
                self.builder
                    .append_operation_node(interner, None, node_kind, None);
            }
            cfg_blocks.insert(block.id, cfg_block);
        }
        self.builder.add_edge(
            interner,
            entry,
            *cfg_blocks
                .get(&blocks[0].id)
                .expect("first MIR block must have a CFG block"),
            CfgEdgeKind::Normal,
        );

        for block in blocks {
            let from = *cfg_blocks
                .get(&block.id)
                .expect("MIR block must have a CFG block");
            let mut stop_lowering = false;
            for statement_id in &block.statements {
                let statement = self
                    .inputs
                    .statements
                    .get(statement_id)
                    .copied()
                    .expect("MIR block statement must exist");
                let operation = self
                    .inputs
                    .operations
                    .get(&statement.operation)
                    .copied()
                    .expect("MIR statement operation must exist");
                let MirOperationKind::Unsupported { unsupported } = &operation.kind else {
                    continue;
                };
                let shape = self.unsupported_shape(*unsupported);
                if !shape.boundary_edges.is_empty() {
                    let target = if shape.to_exceptional_exit {
                        self.builder
                            .exceptional_exit_block()
                            .unwrap_or_else(|| self.builder.normal_exit_block())
                    } else {
                        let target = self
                            .builder
                            .start_block(interner, BasicBlockKind::Synthetic);
                        self.builder.append_operation_node(
                            interner,
                            None,
                            CfgNodeKind::Synthetic,
                            Some(operation.span.clone()),
                        );
                        target
                    };
                    for edge_kind in &shape.boundary_edges {
                        self.builder.add_edge(interner, from, target, *edge_kind);
                    }
                }
                stop_lowering |= shape.stop_lowering;
            }
            if stop_lowering {
                continue;
            }
            let terminator = self
                .inputs
                .terminators
                .get(&block.terminator)
                .copied()
                .expect("MIR block terminator must exist");
            match &terminator.kind {
                MirTerminatorKind::Goto { target } => {
                    self.add_mir_edge(interner, from, *target, &cfg_blocks, CfgEdgeKind::Normal);
                }
                MirTerminatorKind::Branch {
                    then_target,
                    else_target,
                    ..
                } => {
                    self.add_mir_edge(interner, from, *then_target, &cfg_blocks, CfgEdgeKind::True);
                    self.add_mir_edge(
                        interner,
                        from,
                        *else_target,
                        &cfg_blocks,
                        CfgEdgeKind::False,
                    );
                }
                MirTerminatorKind::Switch {
                    cases, otherwise, ..
                } => {
                    for (_, target) in cases {
                        self.add_mir_edge(
                            interner,
                            from,
                            *target,
                            &cfg_blocks,
                            CfgEdgeKind::SwitchCase,
                        );
                    }
                    self.add_mir_edge(
                        interner,
                        from,
                        *otherwise,
                        &cfg_blocks,
                        CfgEdgeKind::DefaultCase,
                    );
                }
                MirTerminatorKind::Return { .. } => {
                    self.builder.add_edge(
                        interner,
                        from,
                        self.builder.normal_exit_block(),
                        CfgEdgeKind::Return,
                    );
                }
                MirTerminatorKind::Throw { unwind, .. } => {
                    self.add_mir_edge(interner, from, *unwind, &cfg_blocks, CfgEdgeKind::Throw);
                }
                MirTerminatorKind::Call { normal, unwind, .. } => {
                    self.add_mir_edge(interner, from, *normal, &cfg_blocks, CfgEdgeKind::Normal);
                    if let Some(unwind) = unwind {
                        self.add_mir_edge(
                            interner,
                            from,
                            *unwind,
                            &cfg_blocks,
                            CfgEdgeKind::ImplicitThrow,
                        );
                    }
                }
                MirTerminatorKind::Suspend { kind, resume, .. } => {
                    let (suspend, resumed) = match kind {
                        crate::analysis_neutral::mir_body::SuspendKind::Await => {
                            (CfgEdgeKind::AwaitSuspend, CfgEdgeKind::AwaitResume)
                        }
                        crate::analysis_neutral::mir_body::SuspendKind::Yield => {
                            (CfgEdgeKind::YieldSuspend, CfgEdgeKind::YieldResume)
                        }
                        crate::analysis_neutral::mir_body::SuspendKind::ChannelRecv
                        | crate::analysis_neutral::mir_body::SuspendKind::ChannelSend => {
                            (CfgEdgeKind::Unknown, CfgEdgeKind::Normal)
                        }
                    };
                    self.add_mir_edge(interner, from, *resume, &cfg_blocks, suspend);
                    self.add_mir_edge(interner, from, *resume, &cfg_blocks, resumed);
                }
                MirTerminatorKind::Unreachable => {
                    if unwind_targets.contains_key(&block.id) {
                        let exit = self
                            .builder
                            .exceptional_exit_block()
                            .unwrap_or_else(|| self.builder.normal_exit_block());
                        self.builder
                            .add_edge(interner, from, exit, CfgEdgeKind::Normal);
                    }
                }
                MirTerminatorKind::Unsupported { .. } => {}
            }
        }
    }

    fn add_mir_edge(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        from: crate::analysis_neutral::cfg::ids::BasicBlockId,
        target: MirBlockId,
        blocks: &BTreeMap<MirBlockId, crate::analysis_neutral::cfg::ids::BasicBlockId>,
        kind: CfgEdgeKind,
    ) {
        let to = *blocks
            .get(&target)
            .expect("MIR terminator target must exist in its body");
        self.builder.add_edge(interner, from, to, kind);
    }

    fn operation_node_kind(
        &self,
        operation: &crate::analysis_neutral::mir_op::MirOperation,
    ) -> CfgNodeKind {
        match &operation.kind {
            MirOperationKind::Return { .. } => CfgNodeKind::Return,
            MirOperationKind::Call { .. } => CfgNodeKind::CallSite,
            MirOperationKind::Unsupported { unsupported } => {
                self.unsupported_shape(*unsupported).node_kind
            }
            _ => CfgNodeKind::Operation,
        }
    }

    fn unsupported_shape(&self, unsupported: UnsupportedId) -> UnsupportedShape {
        match self
            .unsupported_by_id(unsupported)
            .map(|row| row.construct.as_str())
        {
            Some("go_statement") => UnsupportedShape {
                node_kind: CfgNodeKind::CallSite,
                boundary_edges: vec![CfgEdgeKind::Spawn],
                to_exceptional_exit: false,
                stop_lowering: false,
            },
            Some("defer_statement") => UnsupportedShape {
                node_kind: CfgNodeKind::Defer,
                boundary_edges: vec![CfgEdgeKind::Defer],
                to_exceptional_exit: false,
                stop_lowering: false,
            },
            Some("select_statement") => UnsupportedShape {
                node_kind: CfgNodeKind::Unsupported,
                boundary_edges: vec![CfgEdgeKind::Unknown],
                to_exceptional_exit: false,
                stop_lowering: false,
            },
            Some("ERROR") => UnsupportedShape {
                node_kind: CfgNodeKind::Unsupported,
                boundary_edges: vec![CfgEdgeKind::Unknown],
                to_exceptional_exit: false,
                stop_lowering: true,
            },
            Some("fallthrough") => UnsupportedShape {
                node_kind: CfgNodeKind::Goto,
                boundary_edges: vec![CfgEdgeKind::Unknown],
                to_exceptional_exit: false,
                stop_lowering: false,
            },
            Some("goto") => UnsupportedShape {
                node_kind: CfgNodeKind::Goto,
                boundary_edges: vec![CfgEdgeKind::Goto],
                to_exceptional_exit: false,
                stop_lowering: false,
            },
            Some("break") => UnsupportedShape {
                node_kind: CfgNodeKind::Break,
                boundary_edges: vec![CfgEdgeKind::Break],
                to_exceptional_exit: false,
                stop_lowering: false,
            },
            Some("continue") => UnsupportedShape {
                node_kind: CfgNodeKind::Continue,
                boundary_edges: vec![CfgEdgeKind::Continue],
                to_exceptional_exit: false,
                stop_lowering: false,
            },
            Some("optional chaining") => UnsupportedShape {
                node_kind: CfgNodeKind::Condition,
                boundary_edges: vec![CfgEdgeKind::OptionalChain],
                to_exceptional_exit: false,
                stop_lowering: false,
            },
            Some("try") => UnsupportedShape {
                node_kind: CfgNodeKind::FinallyEnter,
                boundary_edges: vec![CfgEdgeKind::Finally, CfgEdgeKind::Cleanup],
                to_exceptional_exit: false,
                stop_lowering: false,
            },
            Some("switch") => UnsupportedShape {
                node_kind: CfgNodeKind::Condition,
                boundary_edges: vec![CfgEdgeKind::SwitchCase, CfgEdgeKind::DefaultCase],
                to_exceptional_exit: false,
                stop_lowering: false,
            },
            Some("parser recovery") => UnsupportedShape {
                node_kind: CfgNodeKind::Unsupported,
                boundary_edges: vec![CfgEdgeKind::Unknown],
                to_exceptional_exit: false,
                stop_lowering: true,
            },
            Some("dynamic import")
            | Some("eval")
            | Some("Proxy")
            | Some("getter")
            | Some("setter") => UnsupportedShape {
                node_kind: CfgNodeKind::Unsupported,
                boundary_edges: vec![CfgEdgeKind::Unknown],
                to_exceptional_exit: false,
                stop_lowering: false,
            },
            _ => UnsupportedShape {
                node_kind: CfgNodeKind::Unsupported,
                boundary_edges: vec![CfgEdgeKind::Unknown],
                to_exceptional_exit: false,
                stop_lowering: false,
            },
        }
    }

    fn unsupported_by_id(&self, id: UnsupportedId) -> Option<&UnsupportedSemanticFact> {
        self.inputs.unsupported.get(&id).copied()
    }

    fn unsupported_for_body(
        &self,
        body: MirBodyId,
    ) -> impl Iterator<Item = &'db UnsupportedSemanticFact> + '_ {
        self.inputs
            .unsupported_by_body
            .get(&body)
            .into_iter()
            .flatten()
            .copied()
    }

    fn finish(self, interner: &crate::internal_core::StableKeyInterner) -> CfgOutput {
        let body_to_function = self.body_to_function;
        let db = self.db;
        let mut output = self.builder.finish(interner);
        let mut unsupported = db
            .unsupported_semantics()
            .iter()
            .filter(|row| row.affected_domains.contains(&UnsupportedDomain::Cfg))
            .enumerate()
            .map(|(index, row)| {
                unsupported_control_flow_fact(interner, index, row, &body_to_function)
            })
            .collect::<Vec<_>>();
        output.unsupported.append(&mut unsupported);
        output.normalized(interner)
    }
}

struct UnsupportedShape {
    node_kind: CfgNodeKind,
    boundary_edges: Vec<CfgEdgeKind>,
    to_exceptional_exit: bool,
    stop_lowering: bool,
}

fn unsupported_control_flow_fact(
    interner: &crate::internal_core::StableKeyInterner,
    index: usize,
    row: &UnsupportedSemanticFact,
    body_to_function: &BTreeMap<MirBodyId, CfgFunctionId>,
) -> UnsupportedControlFlowFact {
    let cfg_function = row
        .body
        .and_then(|body| body_to_function.get(&body).copied());
    UnsupportedControlFlowFact {
        id: UnsupportedControlFlowId(index as u64 + 1),
        cfg_function,
        body: row.body,
        operation: row.operation,
        language: row.language,
        file: row.file,
        span: row.span.clone(),
        construct: row.construct.clone(),
        source_evidence: row.source_evidence.clone(),
        conservative_action: control_flow_action(row.conservative_action),
        stable_key: interner.intern(
            semantic_stable_key(
                FactFamily::UnsupportedControlFlow,
                &[
                    ("language", language_label(row.language).to_string()),
                    ("construct", row.construct.clone()),
                    (
                        "span",
                        format!("{}..{}", row.span.start_byte, row.span.end_byte),
                    ),
                    ("source", interner.resolve(row.stable_key).to_string()),
                ],
            )
            .into_string(),
        ),
        status: cfg_status(row.status),
        precision: cfg_precision(row.precision),
    }
}

fn control_flow_action(action: ConservativeAction) -> ControlFlowAction {
    match action {
        ConservativeAction::SkipOperation => ControlFlowAction::SkipUnreachableTail,
        ConservativeAction::HavocAffectedPlaces | ConservativeAction::PreserveWithUnknownValue => {
            ControlFlowAction::PreserveUnknownEdge
        }
        ConservativeAction::StopLowering => ControlFlowAction::StopAtBoundary,
    }
}

fn cfg_status(status: MirStatus) -> CfgStatus {
    match status {
        MirStatus::Resolved => CfgStatus::Resolved,
        MirStatus::Partial => CfgStatus::Partial,
        MirStatus::Unknown => CfgStatus::Unknown,
        MirStatus::Unsupported => CfgStatus::Unsupported,
    }
}

fn cfg_precision(precision: UnsupportedPrecision) -> CfgPrecision {
    match precision {
        UnsupportedPrecision::Partial => CfgPrecision::Conservative,
        UnsupportedPrecision::Unknown => CfgPrecision::Unknown,
        UnsupportedPrecision::Unsupported => CfgPrecision::Unsupported,
    }
}

fn language_label(language: Language) -> &'static str {
    match language {
        Language::TypeScript => "ts",
        Language::Tsx => "tsx",
        Language::JavaScript => "js",
        Language::Jsx => "jsx",
        Language::Go => "go",
        Language::Unknown => "unknown",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::ids::{MirStatementId, MirTerminatorId, PlaceId};
    use crate::analysis_neutral::mir_body::{
        MirBlock, MirBlockId, MirBody, MirOutput, MirStatement, MirTerminator, MirTerminatorKind,
    };
    use crate::analysis_neutral::mir_op::{
        AssignMode, MirOperation, MirOperationKind, MirValue, UnsupportedDomain,
        UnsupportedSemanticFact,
    };
    use crate::analysis_neutral::places::{PlaceFact, PlaceRoot, PlaceStatus};
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    fn span(start: usize, end: usize) -> Span {
        Span::new(
            FileId::from_raw(1),
            start as u32,
            end as u32,
            1,
            start as u32 + 1,
            1,
            end as u32 + 1,
        )
    }

    fn body(interner: &crate::internal_core::StableKeyInterner, language: Language) -> MirBody {
        MirBody {
            id: MirBodyId(0),
            language,
            file: FileId::from_raw(1),
            function: FunctionId::from_raw(1),
            package: None,
            module: None,
            owner_stable_key: interner.intern("ts:function:f".to_string()),
            span: span(0, 10),
            stable_key: interner.intern(format!("{}:body:f", language_label(language))),
            status: MirStatus::Partial,
        }
    }

    fn assign(
        interner: &crate::internal_core::StableKeyInterner,
        id: u64,
        ordinal: u32,
    ) -> MirOperation {
        MirOperation {
            id: MirOpId(id),
            body: MirBodyId(0),
            ordinal,
            span: span(ordinal as usize, ordinal as usize + 1),
            kind: MirOperationKind::Assign {
                place: PlaceId(1),
                value: MirValue::Place(PlaceId(1)),
                mode: AssignMode::Overwrite,
            },
            stable_key: interner.intern(format!("ts:assign:{ordinal}")),
            status: MirStatus::Partial,
        }
    }

    fn return_op(
        interner: &crate::internal_core::StableKeyInterner,
        id: u64,
        ordinal: u32,
    ) -> MirOperation {
        MirOperation {
            id: MirOpId(id),
            body: MirBodyId(0),
            ordinal,
            span: span(ordinal as usize, ordinal as usize + 1),
            kind: MirOperationKind::Return { value: None },
            stable_key: interner.intern(format!("ts:return:{ordinal}")),
            status: MirStatus::Partial,
        }
    }

    fn unsupported(
        interner: &crate::internal_core::StableKeyInterner,
        id: u64,
        operation: Option<MirOpId>,
        construct: &str,
    ) -> UnsupportedSemanticFact {
        let ordinal = operation.map_or(9, |operation| operation.0 as u32);
        UnsupportedSemanticFact {
            id: UnsupportedId(id),
            body: Some(MirBodyId(0)),
            operation,
            language: Language::TypeScript,
            file: FileId::from_raw(1),
            span: span(ordinal as usize, ordinal as usize + 1),
            construct: construct.to_string(),
            source_evidence: construct.to_string(),
            affected_places: Vec::new(),
            affected_domains: vec![UnsupportedDomain::Mir, UnsupportedDomain::Cfg],
            conservative_action: ConservativeAction::HavocAffectedPlaces,
            precision: UnsupportedPrecision::Unsupported,
            status: MirStatus::Unsupported,
            stable_key: interner.intern(format!("ts:unsupported:{construct}:{id}")),
        }
    }

    fn place(
        interner: &crate::internal_core::StableKeyInterner,
        id: u64,
        name: &str,
        language: Language,
    ) -> PlaceFact {
        PlaceFact {
            id: PlaceId(id),
            language,
            file: Some(FileId::from_raw(1)),
            function: Some(FunctionId::from_raw(1)),
            root: PlaceRoot::Local {
                function: FunctionId::from_raw(1),
                name: name.to_string(),
            },
            projections: Vec::new(),
            stable_key: interner.intern(format!("ts:place:{name}")),
            status: PlaceStatus::Resolved,
        }
    }

    fn db_with(
        language: Language,
        operations: Vec<MirOperation>,
        unsupported: Vec<UnsupportedSemanticFact>,
    ) -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        let statements = operations
            .iter()
            .enumerate()
            .map(|(index, operation)| MirStatement {
                id: MirStatementId(index as u64),
                body: MirBodyId(0),
                ordinal: operation.ordinal,
                operation: operation.id,
                stable_key: interner.intern(format!("statement:{index}")),
                status: operation.status,
            })
            .collect::<Vec<_>>();
        let value = operations.iter().rev().find_map(|operation| {
            if let MirOperationKind::Return { value } = &operation.kind {
                Some(value.clone())
            } else {
                None
            }
        });
        db.replace_semantic_mir(MirOutput {
            bodies: vec![body(&interner, language)],
            blocks: vec![MirBlock {
                id: MirBlockId(0),
                body: MirBodyId(0),
                ordinal: 0,
                statements: statements.iter().map(|statement| statement.id).collect(),
                terminator: MirTerminatorId(0),
                stable_key: interner.intern("block:0".to_string()),
            }],
            statements,
            terminators: vec![MirTerminator {
                id: MirTerminatorId(0),
                body: MirBodyId(0),
                ordinal: 0,
                kind: MirTerminatorKind::Return {
                    value: value.flatten(),
                },
                stable_key: interner.intern("terminator:0".to_string()),
                status: MirStatus::Partial,
            }],
            places: vec![
                place(&interner, 1, "x", language),
                place(&interner, 2, "ret", language),
            ],
            place_types: Vec::new(),
            operations,
            unsupported,
        })
        .expect("MIR output should store");
        db
    }

    #[test]
    fn cfg_lowers_straight_line_return_without_derived_rows() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let db = db_with(
            Language::TypeScript,
            vec![assign(&interner, 1, 1), return_op(&interner, 2, 2)],
            Vec::new(),
        );
        let output = lower_cfg(&db);

        assert!(
            output
                .functions
                .iter()
                .any(|function| function.language == Language::TypeScript)
        );
        assert!(
            output
                .edges
                .iter()
                .any(|edge| edge.kind == CfgEdgeKind::Return)
        );
        assert!(output.reachability.is_empty());
    }

    #[test]
    fn cfg_connects_explicit_mir_return_to_normal_exit() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let db = db_with(
            Language::TypeScript,
            vec![assign(&interner, 1, 1)],
            Vec::new(),
        );
        let output = lower_cfg(&db);
        let exit = output
            .blocks
            .iter()
            .find(|block| block.kind == BasicBlockKind::ExitNormal)
            .expect("normal exit block")
            .id;

        assert!(
            output
                .edges
                .iter()
                .any(|edge| { edge.to_block == exit && edge.kind == CfgEdgeKind::Return })
        );
    }

    #[test]
    fn cfg_lowers_go_body_without_language_dispatch() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let db = db_with(Language::Go, vec![assign(&interner, 1, 1)], Vec::new());
        let output = lower_cfg(&db);

        assert_eq!(output.functions.len(), 1);
        assert_eq!(output.functions[0].language, Language::Go);
        assert!(
            output
                .edges
                .iter()
                .any(|edge| edge.kind == CfgEdgeKind::Return)
        );
    }

    #[test]
    fn unsupported_control_flow_key_uses_source_stable_identity() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let mut first = unsupported(&interner, 1, Some(MirOpId(1)), "throw");
        let mut second = unsupported(&interner, 2, Some(MirOpId(99)), "throw");
        first.body = Some(MirBodyId(7));
        second.body = Some(MirBodyId(42));
        second.span = first.span.clone();
        first.stable_key = interner.intern("ts:unsupported:stable-source".to_string());
        second.stable_key = first.stable_key;

        let first_fact = unsupported_control_flow_fact(&interner, 0, &first, &BTreeMap::new());
        let second_fact = unsupported_control_flow_fact(&interner, 0, &second, &BTreeMap::new());

        assert_eq!(first_fact.stable_key, second_fact.stable_key);
    }
}
