use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_api::{FactFamily, stable_key_from_key_parts};
use crate::analysis_neutral::cfg::facts::{
    BasicBlockFact, BasicBlockKind, CfgEdgeFact, CfgEdgeKind, CfgFunctionFact, CfgNodeFact,
    CfgNodeKind, CfgPrecision, CfgStatus, CfgView,
};
use crate::analysis_neutral::cfg::ids::{BasicBlockId, CfgEdgeId, CfgFunctionId, CfgNodeId};
use crate::analysis_neutral::cfg::store::CfgOutput;
use crate::analysis_neutral::ids::{MirBodyId, MirOpId};
use crate::analysis_neutral::mir_body::MirBody;
use crate::analysis_neutral::mir_op::MirOperation;
use crate::analysis_neutral::stable_key::semantic_stable_key;
use crate::internal_core::{KeyPart, Span, StableKeyId};

#[derive(Debug, Clone)]
pub struct CfgBuilder {
    output: CfgOutput,
    next_function: u64,
    next_node: u64,
    next_block: u64,
    next_edge: u64,
    current_function: Option<CfgFunctionId>,
    current_body: Option<crate::analysis_neutral::ids::MirBodyId>,
    current_body_stable_key: Option<StableKeyId>,
    current_block: Option<BasicBlockId>,
    next_body_block_ordinal: u32,
    next_synthetic_node_ordinal: u32,
    node_index_by_id: BTreeMap<CfgNodeId, usize>,
    block_index_by_id: BTreeMap<BasicBlockId, usize>,
    normal_exit_block_by_function: BTreeMap<CfgFunctionId, BasicBlockId>,
    exceptional_exit_block_by_function: BTreeMap<CfgFunctionId, BasicBlockId>,
}

struct CfgNodeDraft {
    function: CfgFunctionId,
    body: MirBodyId,
    operation: Option<MirOpId>,
    kind: CfgNodeKind,
    span: Option<Span>,
    generated: bool,
    operation_ordinal: u32,
    stable_key: StableKeyId,
}

impl Default for CfgBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl CfgBuilder {
    pub fn new() -> Self {
        Self {
            output: CfgOutput::empty(),
            next_function: 1,
            next_node: 1,
            next_block: 1,
            next_edge: 1,
            current_function: None,
            current_body: None,
            current_body_stable_key: None,
            current_block: None,
            next_body_block_ordinal: 0,
            next_synthetic_node_ordinal: 0,
            node_index_by_id: BTreeMap::new(),
            block_index_by_id: BTreeMap::new(),
            normal_exit_block_by_function: BTreeMap::new(),
            exceptional_exit_block_by_function: BTreeMap::new(),
        }
    }

    pub fn start_function(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        body: &MirBody,
        include_exceptional_exit: bool,
    ) -> CfgFunctionId {
        let function_id = self.alloc_function_id();
        let body_stable_key = body.stable_key;
        let owner_stable_key = body.owner_stable_key;
        self.current_function = Some(function_id);
        self.current_body = Some(body.id);
        self.current_body_stable_key = Some(body_stable_key);
        self.next_body_block_ordinal = 0;
        self.next_synthetic_node_ordinal = 0;

        let entry_node = self.new_node(CfgNodeDraft {
            function: function_id,
            body: body.id,
            operation: None,
            kind: CfgNodeKind::Entry,
            span: Some(body.span.clone()),
            generated: true,
            operation_ordinal: 0,
            stable_key: stable_key_ref(
                interner,
                FactFamily::CfgNode,
                [
                    ("body", KeyPart::Key(body_stable_key)),
                    ("kind", KeyPart::Text("entry")),
                ],
            ),
        });
        let entry_block = self.new_block(
            function_id,
            BasicBlockKind::Entry,
            Some(entry_node),
            Some(entry_node),
            0,
            stable_key_ref(
                interner,
                FactFamily::BasicBlock,
                [
                    ("body", KeyPart::Key(body_stable_key)),
                    ("kind", KeyPart::Text("entry")),
                ],
            ),
        );

        let normal_exit_node = self.new_node(CfgNodeDraft {
            function: function_id,
            body: body.id,
            operation: None,
            kind: CfgNodeKind::ExitNormal,
            span: Some(body.span.clone()),
            generated: true,
            operation_ordinal: u32::MAX - 1,
            stable_key: stable_key_ref(
                interner,
                FactFamily::CfgNode,
                [
                    ("body", KeyPart::Key(body_stable_key)),
                    ("kind", KeyPart::Text("exit-normal")),
                ],
            ),
        });
        let normal_exit_block = self.new_block(
            function_id,
            BasicBlockKind::ExitNormal,
            Some(normal_exit_node),
            Some(normal_exit_node),
            u32::MAX - 1,
            stable_key_ref(
                interner,
                FactFamily::BasicBlock,
                [
                    ("body", KeyPart::Key(body_stable_key)),
                    ("kind", KeyPart::Text("exit-normal")),
                ],
            ),
        );
        self.normal_exit_block_by_function
            .insert(function_id, normal_exit_block);

        let exceptional_exit_node = include_exceptional_exit.then(|| {
            self.new_node(CfgNodeDraft {
                function: function_id,
                body: body.id,
                operation: None,
                kind: CfgNodeKind::ExitExceptional,
                span: Some(body.span.clone()),
                generated: true,
                operation_ordinal: u32::MAX,
                stable_key: stable_key_ref(
                    interner,
                    FactFamily::CfgNode,
                    [
                        ("body", KeyPart::Key(body_stable_key)),
                        ("kind", KeyPart::Text("exit-exceptional")),
                    ],
                ),
            })
        });
        if let Some(node) = exceptional_exit_node {
            let exceptional_exit_block = self.new_block(
                function_id,
                BasicBlockKind::ExitExceptional,
                Some(node),
                Some(node),
                u32::MAX,
                stable_key_ref(
                    interner,
                    FactFamily::BasicBlock,
                    [
                        ("body", KeyPart::Key(body_stable_key)),
                        ("kind", KeyPart::Text("exit-exceptional")),
                    ],
                ),
            );
            self.exceptional_exit_block_by_function
                .insert(function_id, exceptional_exit_block);
        }

        self.output.functions.push(CfgFunctionFact {
            id: function_id,
            body: body.id,
            function: body.function,
            language: body.language,
            file: body.file,
            span: body.span.clone(),
            entry_node,
            normal_exit_node,
            exceptional_exit_node,
            stable_key: stable_key_ref(
                interner,
                FactFamily::CfgFunction,
                [
                    ("body", KeyPart::Key(body_stable_key)),
                    ("owner", KeyPart::Key(owner_stable_key)),
                ],
            ),
            status: CfgStatus::Resolved,
            precision: CfgPrecision::ExactLowered,
        });
        self.current_block = Some(entry_block);
        function_id
    }

    pub fn current_block(&self) -> BasicBlockId {
        self.current_block
            .expect("CfgBuilder::start_function must be called before current_block")
    }

    pub fn normal_exit_block(&self) -> BasicBlockId {
        let function = self.expect_function();
        self.normal_exit_block_by_function
            .get(&function)
            .copied()
            .expect("CfgBuilder::start_function must create a normal exit block")
    }

    pub fn exceptional_exit_block(&self) -> Option<BasicBlockId> {
        let function = self.expect_function();
        self.exceptional_exit_block_by_function
            .get(&function)
            .copied()
    }

    pub fn start_block(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        kind: BasicBlockKind,
    ) -> BasicBlockId {
        let function = self.expect_function();
        let ordinal = self.next_body_block_ordinal;
        self.next_body_block_ordinal += 1;
        let body_key = self.expect_body_stable_key();
        let block_id = self.new_block(
            function,
            kind,
            None,
            None,
            ordinal,
            stable_key_ref(
                interner,
                FactFamily::BasicBlock,
                [
                    ("body", KeyPart::Key(body_key)),
                    ("ordinal", KeyPart::Text(&ordinal.to_string())),
                    ("kind", KeyPart::Text(&format!("{kind:?}"))),
                ],
            ),
        );
        self.current_block = Some(block_id);
        block_id
    }

    pub fn append_operation_node(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        operation: Option<&MirOperation>,
        kind: CfgNodeKind,
        span: Option<Span>,
    ) -> CfgNodeId {
        let function = self.expect_function();
        let body = self
            .current_body
            .expect("CfgBuilder::start_function must be called before append_operation_node");
        let block = self.current_block();
        let block_key = self.block(block).stable_key;
        let previous_last = self.block_mut(block).last_node;
        let operation_ordinal = operation.map_or_else(
            || self.alloc_synthetic_node_ordinal(),
            |operation| operation.ordinal,
        );
        let operation_id = operation.map(|operation| operation.id);
        let synthetic_operation_key = format!("synthetic:{operation_ordinal}");
        let operation_key = operation
            .map_or(KeyPart::Text(&synthetic_operation_key), |operation| {
                KeyPart::Key(operation.stable_key)
            });
        let body_key = self.expect_body_stable_key();
        let node_id = self.new_node(CfgNodeDraft {
            function,
            body,
            operation: operation_id,
            kind,
            span,
            generated: false,
            operation_ordinal,
            stable_key: stable_key_ref(
                interner,
                FactFamily::CfgNode,
                [
                    ("body", KeyPart::Key(body_key)),
                    ("block", KeyPart::Key(block_key)),
                    ("operation", operation_key),
                    ("ordinal", KeyPart::Text(&operation_ordinal.to_string())),
                    ("kind", KeyPart::Text(&format!("{kind:?}"))),
                ],
            ),
        });
        self.node_mut(node_id).block = block;

        {
            let block_fact = self.block_mut(block);
            if block_fact.first_node.is_none() {
                block_fact.first_node = Some(node_id);
            }
            block_fact.last_node = Some(node_id);
        }

        if let Some(previous) = previous_last {
            self.add_node_edge(
                interner,
                previous,
                node_id,
                block,
                block,
                CfgEdgeKind::Normal,
            );
        }
        node_id
    }

    pub fn add_edge(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        from_block: BasicBlockId,
        to_block: BasicBlockId,
        kind: CfgEdgeKind,
    ) -> CfgEdgeId {
        let from_node = self
            .block(from_block)
            .last_node
            .expect("source block must have at least one node before adding an edge");
        let to_node = self
            .block(to_block)
            .first_node
            .expect("target block must have at least one node before adding an edge");
        self.add_node_edge(interner, from_node, to_node, from_block, to_block, kind)
    }

    pub fn mark_unreachable(&mut self, block: BasicBlockId) {
        self.block_mut(block).reachable = false;
    }

    pub fn finish_function(&mut self) {
        self.current_function = None;
        self.current_body = None;
        self.current_body_stable_key = None;
        self.current_block = None;
        self.next_body_block_ordinal = 0;
        self.next_synthetic_node_ordinal = 0;
    }

    pub fn finish(mut self, interner: &crate::internal_core::StableKeyInterner) -> CfgOutput {
        self.refresh_reachability();
        self.output.normalized(interner)
    }

    fn refresh_reachability(&mut self) {
        let functions = self
            .output
            .functions
            .iter()
            .map(|function| function.id)
            .collect::<Vec<_>>();
        let mut entry_by_function = BTreeMap::new();
        for block in &self.output.blocks {
            if block.kind == BasicBlockKind::Entry {
                entry_by_function.insert(block.cfg_function, block.id);
            }
        }
        let mut successors = BTreeMap::<(CfgFunctionId, BasicBlockId), Vec<BasicBlockId>>::new();
        for edge in &self.output.edges {
            if edge.view == CfgView::NormalControl && edge.from_block != edge.to_block {
                successors
                    .entry((edge.cfg_function, edge.from_block))
                    .or_default()
                    .push(edge.to_block);
            }
        }
        for targets in successors.values_mut() {
            targets.sort();
            targets.dedup();
        }

        let mut reachable_by_function = BTreeMap::new();
        for function in functions {
            reachable_by_function.insert(
                function,
                Self::reachable_blocks(function, &entry_by_function, &successors),
            );
        }
        for block in &mut self.output.blocks {
            block.reachable = reachable_by_function
                .get(&block.cfg_function)
                .is_some_and(|reachable| reachable.contains(&block.id));
        }
    }

    fn reachable_blocks(
        function: CfgFunctionId,
        entry_by_function: &BTreeMap<CfgFunctionId, BasicBlockId>,
        successors: &BTreeMap<(CfgFunctionId, BasicBlockId), Vec<BasicBlockId>>,
    ) -> BTreeSet<BasicBlockId> {
        let Some(&entry) = entry_by_function.get(&function) else {
            return BTreeSet::new();
        };
        let mut seen = BTreeSet::new();
        let mut stack = vec![entry];
        while let Some(block) = stack.pop() {
            if !seen.insert(block) {
                continue;
            }
            if let Some(block_successors) = successors.get(&(function, block)) {
                stack.extend(block_successors.iter().rev().copied());
            }
        }
        seen
    }

    fn add_node_edge(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        from: CfgNodeId,
        to: CfgNodeId,
        from_block: BasicBlockId,
        to_block: BasicBlockId,
        kind: CfgEdgeKind,
    ) -> CfgEdgeId {
        let function = self.expect_function();
        let body_key = self.expect_body_stable_key();
        let from_block_key = self.block(from_block).stable_key;
        let to_block_key = self.block(to_block).stable_key;
        let from_node_key = self.node(from).stable_key;
        let to_node_key = self.node(to).stable_key;
        let edge_id = self.alloc_edge_id();
        self.output.edges.push(CfgEdgeFact {
            id: edge_id,
            cfg_function: function,
            view: CfgView::NormalControl,
            from,
            to,
            from_block,
            to_block,
            kind,
            label: None,
            stable_key: stable_key_ref(
                interner,
                FactFamily::CfgEdge,
                [
                    ("body", KeyPart::Key(body_key)),
                    ("from_block", KeyPart::Key(from_block_key)),
                    ("to_block", KeyPart::Key(to_block_key)),
                    ("from_node", KeyPart::Key(from_node_key)),
                    ("to_node", KeyPart::Key(to_node_key)),
                    ("kind", KeyPart::Text(&format!("{kind:?}"))),
                ],
            ),
            status: CfgStatus::Resolved,
            precision: CfgPrecision::ExactLowered,
        });
        edge_id
    }

    fn new_node(&mut self, draft: CfgNodeDraft) -> CfgNodeId {
        let id = self.alloc_node_id();
        let index = self.output.nodes.len();
        self.output.nodes.push(CfgNodeFact {
            id,
            cfg_function: draft.function,
            body: draft.body,
            operation: draft.operation,
            block: BasicBlockId(0),
            kind: draft.kind,
            span: draft.span,
            generated: draft.generated,
            operation_ordinal: draft.operation_ordinal,
            stable_key: draft.stable_key,
            status: CfgStatus::Resolved,
            precision: CfgPrecision::ExactLowered,
        });
        self.node_index_by_id.insert(id, index);
        id
    }

    fn new_block(
        &mut self,
        function: CfgFunctionId,
        kind: BasicBlockKind,
        first_node: Option<CfgNodeId>,
        last_node: Option<CfgNodeId>,
        reverse_postorder: u32,
        stable_key: StableKeyId,
    ) -> BasicBlockId {
        let id = self.alloc_block_id();
        for node_id in [first_node, last_node].into_iter().flatten() {
            self.node_mut(node_id).block = id;
        }
        let index = self.output.blocks.len();
        self.output.blocks.push(BasicBlockFact {
            id,
            cfg_function: function,
            kind,
            first_node,
            last_node,
            reachable: true,
            reverse_postorder,
            stable_key,
            status: CfgStatus::Resolved,
            precision: CfgPrecision::ExactLowered,
        });
        self.block_index_by_id.insert(id, index);
        id
    }

    fn block(&self, id: BasicBlockId) -> &BasicBlockFact {
        let index = self
            .block_index_by_id
            .get(&id)
            .copied()
            .expect("block id must refer to a builder-owned block");
        &self.output.blocks[index]
    }

    fn node(&self, id: CfgNodeId) -> &CfgNodeFact {
        let index = self
            .node_index_by_id
            .get(&id)
            .copied()
            .expect("node id must refer to a builder-owned node");
        &self.output.nodes[index]
    }

    fn block_mut(&mut self, id: BasicBlockId) -> &mut BasicBlockFact {
        let index = self
            .block_index_by_id
            .get(&id)
            .copied()
            .expect("block id must refer to a builder-owned block");
        &mut self.output.blocks[index]
    }

    fn node_mut(&mut self, id: CfgNodeId) -> &mut CfgNodeFact {
        let index = self
            .node_index_by_id
            .get(&id)
            .copied()
            .expect("node id must refer to a builder-owned node");
        &mut self.output.nodes[index]
    }

    fn expect_function(&self) -> CfgFunctionId {
        self.current_function
            .expect("CfgBuilder::start_function must be called before adding CFG rows")
    }

    fn expect_body_stable_key(&self) -> StableKeyId {
        self.current_body_stable_key
            .as_ref()
            .copied()
            .expect("CfgBuilder::start_function must be called before adding CFG rows")
    }

    fn alloc_synthetic_node_ordinal(&mut self) -> u32 {
        let ordinal = self.next_synthetic_node_ordinal;
        self.next_synthetic_node_ordinal += 1;
        ordinal
    }

    fn alloc_function_id(&mut self) -> CfgFunctionId {
        let id = CfgFunctionId(self.next_function);
        self.next_function += 1;
        id
    }

    fn alloc_node_id(&mut self) -> CfgNodeId {
        let id = CfgNodeId(self.next_node);
        self.next_node += 1;
        id
    }

    fn alloc_block_id(&mut self) -> BasicBlockId {
        let id = BasicBlockId(self.next_block);
        self.next_block += 1;
        id
    }

    fn alloc_edge_id(&mut self) -> CfgEdgeId {
        let id = CfgEdgeId(self.next_edge);
        self.next_edge += 1;
        id
    }
}

fn stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    family: FactFamily,
    parts: &[(&str, String)],
) -> StableKeyId {
    interner.intern(semantic_stable_key(family, parts).into_string())
}

/// Like [`stable_key`], for a key that embeds another key's identity.
fn stable_key_ref<const N: usize>(
    interner: &crate::internal_core::StableKeyInterner,
    family: FactFamily,
    parts: [(&str, KeyPart<'_>); N],
) -> StableKeyId {
    crate::analysis_api::stable_key_from_key_parts(interner, family, parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::ids::{MirBodyId, MirOpId, PlaceId};
    use crate::analysis_neutral::mir_body::{MirBody, MirStatus};
    use crate::analysis_neutral::mir_op::{AssignMode, MirOperation, MirOperationKind, MirValue};
    use crate::internal_core::{FileId, FunctionId, Language};

    fn span() -> Span {
        Span::new(FileId::from_raw(1), 1, 2, 1, 1, 1, 2)
    }

    fn body(interner: &crate::internal_core::StableKeyInterner) -> MirBody {
        MirBody {
            id: MirBodyId(1),
            language: Language::Go,
            file: FileId::from_raw(1),
            function: FunctionId::from_raw(1),
            package: None,
            module: None,
            owner_stable_key: interner.intern("owner".to_string()),
            span: span(),
            stable_key: interner.intern("body:one".to_string()),
            status: MirStatus::Resolved,
        }
    }

    fn op(
        interner: &crate::internal_core::StableKeyInterner,
        id: u64,
        ordinal: u32,
    ) -> MirOperation {
        MirOperation {
            id: MirOpId(id),
            body: MirBodyId(1),
            ordinal,
            span: span(),
            kind: MirOperationKind::Assign {
                place: PlaceId(1),
                value: MirValue::Place(PlaceId(2)),
                mode: AssignMode::Overwrite,
            },
            stable_key: interner.intern(format!("op:{ordinal}")),
            status: MirStatus::Resolved,
        }
    }

    #[test]
    fn builder_creates_virtual_entry_and_exit_per_function() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let mut builder = CfgBuilder::new();
        let function = builder.start_function(&interner, &body(&interner), true);
        builder.finish_function();
        let output = builder.finish(&interner);

        let function_fact = output
            .functions
            .iter()
            .find(|fact| fact.id == function)
            .expect("function row");
        assert!(
            output
                .nodes
                .iter()
                .any(|node| node.id == function_fact.entry_node)
        );
        assert!(
            output
                .nodes
                .iter()
                .any(|node| node.id == function_fact.normal_exit_node)
        );
        assert!(function_fact.exceptional_exit_node.is_some());
    }

    #[test]
    fn builder_recomputes_reachability_before_finish() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let mut builder = CfgBuilder::new();
        builder.start_function(&interner, &body(&interner), true);
        let entry = builder.current_block();
        let normal_exit = builder.normal_exit_block();
        let exceptional_exit = builder
            .exceptional_exit_block()
            .expect("exceptional exit block");
        builder.add_edge(&interner, entry, normal_exit, CfgEdgeKind::Normal);
        builder.finish_function();
        let output = builder.finish(&interner);

        let entry_block = output
            .blocks
            .iter()
            .find(|block| block.id == entry)
            .expect("entry block");
        let normal_exit_block = output
            .blocks
            .iter()
            .find(|block| block.id == normal_exit)
            .expect("normal exit block");
        let exceptional_exit_block = output
            .blocks
            .iter()
            .find(|block| block.id == exceptional_exit)
            .expect("exceptional exit block");

        assert!(entry_block.reachable);
        assert!(normal_exit_block.reachable);
        assert!(!exceptional_exit_block.reachable);
    }

    #[test]
    fn builder_assigns_deterministic_ids_for_straight_line_graphs() {
        fn build_keys() -> Vec<String> {
            let interner = crate::internal_core::StableKeyInterner::default();
            let mut builder = CfgBuilder::new();
            builder.start_function(&interner, &body(&interner), false);
            builder.append_operation_node(
                &interner,
                Some(&op(&interner, 1, 1)),
                CfgNodeKind::Operation,
                Some(span()),
            );
            builder.append_operation_node(
                &interner,
                Some(&op(&interner, 2, 2)),
                CfgNodeKind::Operation,
                Some(span()),
            );
            builder.finish_function();
            builder
                .finish(&interner)
                .nodes
                .into_iter()
                .map(|node| {
                    format!(
                        "{}:{}",
                        node.id.0,
                        interner.resolve(node.stable_key).as_ref()
                    )
                })
                .collect()
        }

        assert_eq!(build_keys(), build_keys());
    }

    #[test]
    fn builder_models_if_else_join_loop_return_unreachable_and_short_circuit_edges() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let mut builder = CfgBuilder::new();
        builder.start_function(&interner, &body(&interner), false);
        let entry = builder.current_block();
        let condition = builder.start_block(&interner, BasicBlockKind::Branch);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 1, 1)),
            CfgNodeKind::Condition,
            Some(span()),
        );
        let then_block = builder.start_block(&interner, BasicBlockKind::StraightLine);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 2, 2)),
            CfgNodeKind::Operation,
            Some(span()),
        );
        let else_block = builder.start_block(&interner, BasicBlockKind::StraightLine);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 3, 3)),
            CfgNodeKind::Operation,
            Some(span()),
        );
        let loop_header = builder.start_block(&interner, BasicBlockKind::LoopHeader);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 4, 4)),
            CfgNodeKind::Condition,
            Some(span()),
        );
        let return_block = builder.start_block(&interner, BasicBlockKind::StraightLine);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 5, 5)),
            CfgNodeKind::Return,
            Some(span()),
        );
        let unreachable = builder.start_block(&interner, BasicBlockKind::Unreachable);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 6, 6)),
            CfgNodeKind::Operation,
            Some(span()),
        );
        builder.mark_unreachable(unreachable);

        builder.add_edge(&interner, entry, condition, CfgEdgeKind::Normal);
        builder.add_edge(&interner, condition, then_block, CfgEdgeKind::True);
        builder.add_edge(&interner, condition, else_block, CfgEdgeKind::False);
        builder.add_edge(
            &interner,
            then_block,
            loop_header,
            CfgEdgeKind::ShortCircuit,
        );
        builder.add_edge(&interner, else_block, loop_header, CfgEdgeKind::Normal);
        builder.add_edge(&interner, loop_header, loop_header, CfgEdgeKind::LoopBack);
        builder.add_edge(&interner, loop_header, return_block, CfgEdgeKind::LoopExit);
        builder.finish_function();
        let output = builder.finish(&interner);

        assert!(
            output
                .edges
                .iter()
                .any(|edge| edge.kind == CfgEdgeKind::True)
        );
        assert!(
            output
                .edges
                .iter()
                .any(|edge| edge.kind == CfgEdgeKind::False)
        );
        assert!(
            output
                .edges
                .iter()
                .any(|edge| edge.kind == CfgEdgeKind::LoopBack)
        );
        assert!(
            output
                .edges
                .iter()
                .any(|edge| edge.kind == CfgEdgeKind::ShortCircuit)
        );
        assert!(
            output
                .blocks
                .iter()
                .any(|block| block.kind == BasicBlockKind::Unreachable && !block.reachable)
        );
    }
}
