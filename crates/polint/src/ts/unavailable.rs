//! Compile-time fallbacks for TypeScript analysis implementation modules.

pub mod semantic_graph {
    use crate::analysis_api::SourceFile;
    use crate::ts::inventory::store::TsInventoryOutput;
    use crate::ts::object_model::store::TsObjectModelOutput;
    use crate::ts::scope::store::TsScopeOutput;

    #[derive(Debug)]
    pub struct TsFileAnalysis {
        pub file: crate::internal_core::FileId,
        pub inventory: TsInventoryOutput,
        pub scope: TsScopeOutput,
        pub object_model: TsObjectModelOutput,
        pub(crate) value_flows: crate::ts::constraints::TsValueFlows,
    }

    pub fn analyze_ts_file(
        _interner: &crate::internal_core::StableKeyInterner,
        file: &SourceFile,
    ) -> TsFileAnalysis {
        TsFileAnalysis {
            file: file.id,
            inventory: TsInventoryOutput::default(),
            scope: TsScopeOutput::default(),
            object_model: TsObjectModelOutput::default(),
            value_flows: crate::ts::constraints::TsValueFlows::default(),
        }
    }
}
