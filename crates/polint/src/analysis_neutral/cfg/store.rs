use crate::analysis_neutral::cfg::facts::{
    BasicBlockFact, CfgEdgeFact, CfgFunctionFact, CfgNodeFact, ControlDependenceFact,
    DominatorFact, PostDominatorFact, ReachabilityFact, UnsupportedControlFlowFact,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CfgOutput {
    pub functions: Vec<CfgFunctionFact>,
    pub nodes: Vec<CfgNodeFact>,
    pub blocks: Vec<BasicBlockFact>,
    pub edges: Vec<CfgEdgeFact>,
    pub reachability: Vec<ReachabilityFact>,
    pub dominators: Vec<DominatorFact>,
    pub postdominators: Vec<PostDominatorFact>,
    pub control_dependence: Vec<ControlDependenceFact>,
    pub unsupported: Vec<UnsupportedControlFlowFact>,
}

/// How far a separately lowered part of a CFG output moves when it is appended
/// behind the parts lowered before it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CfgIdOffsets {
    pub(crate) functions: u64,
    pub(crate) nodes: u64,
    pub(crate) blocks: u64,
    pub(crate) edges: u64,
}

impl CfgOutput {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Appends a part lowered on its own, renumbering its builder-local ids
    /// behind the rows already here and moving its stable keys through `remap`.
    ///
    /// A builder numbers each id family from one and allocates exactly one row
    /// per id, so the offsets are the row counts so far, and appending parts in
    /// lowering order gives every row the id one builder lowering all of them in
    /// that order would have given it. Only the builder's own families are
    /// renumbered: the derived families and unsupported rows are added after the
    /// parts are joined.
    pub(crate) fn append_lowered_part(
        &mut self,
        mut part: CfgOutput,
        remap: impl Fn(crate::internal_core::StableKeyId) -> crate::internal_core::StableKeyId,
    ) {
        use crate::analysis_neutral::cfg::ids::{
            BasicBlockId, CfgEdgeId, CfgFunctionId, CfgNodeId,
        };
        debug_assert!(
            part.reachability.is_empty()
                && part.dominators.is_empty()
                && part.postdominators.is_empty()
                && part.control_dependence.is_empty()
                && part.unsupported.is_empty(),
            "a lowered part carries builder rows only"
        );
        let offsets = CfgIdOffsets {
            functions: self.functions.len() as u64,
            nodes: self.nodes.len() as u64,
            blocks: self.blocks.len() as u64,
            edges: self.edges.len() as u64,
        };
        let function = |id: CfgFunctionId| CfgFunctionId(id.0 + offsets.functions);
        let node = |id: CfgNodeId| CfgNodeId(id.0 + offsets.nodes);
        let block = |id: BasicBlockId| BasicBlockId(id.0 + offsets.blocks);
        for row in &mut part.functions {
            row.id = function(row.id);
            row.entry_node = node(row.entry_node);
            row.normal_exit_node = node(row.normal_exit_node);
            row.exceptional_exit_node = row.exceptional_exit_node.map(node);
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut part.nodes {
            row.id = node(row.id);
            row.cfg_function = function(row.cfg_function);
            row.block = block(row.block);
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut part.blocks {
            row.id = block(row.id);
            row.cfg_function = function(row.cfg_function);
            row.first_node = row.first_node.map(node);
            row.last_node = row.last_node.map(node);
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut part.edges {
            row.id = CfgEdgeId(row.id.0 + offsets.edges);
            row.cfg_function = function(row.cfg_function);
            row.from = node(row.from);
            row.to = node(row.to);
            row.from_block = block(row.from_block);
            row.to_block = block(row.to_block);
            row.stable_key = remap(row.stable_key);
        }
        self.functions.append(&mut part.functions);
        self.nodes.append(&mut part.nodes);
        self.blocks.append(&mut part.blocks);
        self.edges.append(&mut part.edges);
    }

    pub fn normalized(mut self, interner: &crate::internal_core::StableKeyInterner) -> Self {
        self.functions
            .sort_by_cached_key(|row| interner.resolve(row.stable_key));
        self.nodes.sort_by_cached_key(|row| {
            (
                row.cfg_function,
                row.operation_ordinal,
                interner.resolve(row.stable_key),
            )
        });
        self.blocks
            .sort_by_cached_key(|row| interner.resolve(row.stable_key));
        self.edges.sort_by_cached_key(|row| {
            (
                row.cfg_function,
                row.view,
                row.from_block,
                row.to_block,
                row.kind,
                interner.resolve(row.stable_key),
            )
        });
        self.reachability
            .sort_by_cached_key(|row| interner.resolve(row.stable_key));
        self.dominators
            .sort_by_cached_key(|row| interner.resolve(row.stable_key));
        self.postdominators
            .sort_by_cached_key(|row| interner.resolve(row.stable_key));
        self.control_dependence
            .sort_by_cached_key(|row| interner.resolve(row.stable_key));
        self.unsupported
            .sort_by_cached_key(|row| interner.resolve(row.stable_key));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::cfg::facts::{
        BasicBlockKind, CfgNodeKind, CfgPrecision, CfgStatus,
    };
    use crate::analysis_neutral::cfg::ids::{BasicBlockId, CfgFunctionId, CfgNodeId};
    use crate::analysis_neutral::ids::MirBodyId;
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    fn span() -> Span {
        Span::new(FileId::from_raw(1), 1, 2, 1, 1, 1, 2)
    }

    fn function(
        interner: &crate::internal_core::StableKeyInterner,
        id: u64,
        stable_key: &str,
    ) -> CfgFunctionFact {
        CfgFunctionFact {
            id: CfgFunctionId(id),
            body: MirBodyId(id),
            function: FunctionId::from_raw(id),
            language: Language::Go,
            file: FileId::from_raw(1),
            span: span(),
            entry_node: CfgNodeId(1),
            normal_exit_node: CfgNodeId(2),
            exceptional_exit_node: None,
            stable_key: interner.intern(stable_key),
            status: CfgStatus::Resolved,
            precision: CfgPrecision::ExactSyntax,
        }
    }

    fn node(
        interner: &crate::internal_core::StableKeyInterner,
        function: u64,
        ordinal: u32,
        stable_key: &str,
    ) -> CfgNodeFact {
        CfgNodeFact {
            id: CfgNodeId(u64::from(ordinal)),
            cfg_function: CfgFunctionId(function),
            body: MirBodyId(function),
            operation: None,
            block: BasicBlockId(function),
            kind: CfgNodeKind::Operation,
            span: Some(span()),
            generated: false,
            operation_ordinal: ordinal,
            stable_key: interner.intern(stable_key),
            status: CfgStatus::Resolved,
            precision: CfgPrecision::ExactSyntax,
        }
    }

    fn block(
        interner: &crate::internal_core::StableKeyInterner,
        stable_key: &str,
    ) -> BasicBlockFact {
        BasicBlockFact {
            id: BasicBlockId(1),
            cfg_function: CfgFunctionId(1),
            kind: BasicBlockKind::StraightLine,
            first_node: Some(CfgNodeId(1)),
            last_node: Some(CfgNodeId(1)),
            reachable: true,
            reverse_postorder: 0,
            stable_key: interner.intern(stable_key),
            status: CfgStatus::Resolved,
            precision: CfgPrecision::ExactSyntax,
        }
    }

    #[test]
    fn normalized_sorts_families_without_deduplicating() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let output = CfgOutput {
            functions: vec![function(&interner, 2, "b"), function(&interner, 1, "a")],
            nodes: vec![node(&interner, 1, 2, "b"), node(&interner, 1, 1, "a")],
            blocks: vec![
                block(&interner, "b"),
                block(&interner, "a"),
                block(&interner, "a"),
            ],
            ..CfgOutput::empty()
        }
        .normalized(&interner);

        assert_eq!(
            output
                .functions
                .iter()
                .map(|fact| interner.resolve(fact.stable_key).to_string())
                .collect::<Vec<_>>(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(
            output
                .nodes
                .iter()
                .map(|fact| fact.operation_ordinal)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            output
                .blocks
                .iter()
                .map(|fact| interner.resolve(fact.stable_key).to_string())
                .collect::<Vec<_>>(),
            vec!["a".to_string(), "a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn empty_output_contains_no_rows() {
        let output =
            CfgOutput::empty().normalized(&crate::internal_core::StableKeyInterner::default());
        assert!(output.functions.is_empty());
        assert!(output.control_dependence.is_empty());
    }
}
