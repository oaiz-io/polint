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

impl CfgOutput {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn normalized(mut self, interner: &crate::internal_core::StableKeyInterner) -> Self {
        self.functions
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        self.nodes.sort_by(|row, other| {
            row.cfg_function
                .cmp(&other.cfg_function)
                .then_with(|| row.operation_ordinal.cmp(&other.operation_ordinal))
                .then_with(|| interner.compare_canonical(row.stable_key, other.stable_key))
        });
        self.blocks
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        self.edges.sort_by(|row, other| {
            row.cfg_function
                .cmp(&other.cfg_function)
                .then_with(|| row.view.cmp(&other.view))
                .then_with(|| row.from_block.cmp(&other.from_block))
                .then_with(|| row.to_block.cmp(&other.to_block))
                .then_with(|| row.kind.cmp(&other.kind))
                .then_with(|| interner.compare_canonical(row.stable_key, other.stable_key))
        });
        self.reachability
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        self.dominators
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        self.postdominators
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        self.control_dependence
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        self.unsupported
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
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
