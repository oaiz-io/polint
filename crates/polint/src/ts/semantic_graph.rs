//! One-file TypeScript semantic extraction used by graph adapters.

use crate::analysis_api::SourceFile;
use crate::internal_core::StableKeyInterner;
use oxc_semantic::SemanticBuilder;

use crate::ts::constraints::{TsValueFlows, lower_ts_program_value_flows};
use crate::ts::inventory::extract::{
    extract_ts_inventory_from_program, mark_inventory_partial_ast,
};
use crate::ts::inventory::store::TsInventoryOutput;
use crate::ts::object_model::extract::{
    extract_ts_object_model_from_program, mark_object_model_partial_ast,
};
use crate::ts::object_model::store::TsObjectModelOutput;
use crate::ts::parse::ParsedTsSource;
use crate::ts::scope::extract::{extract_ts_scope_from_program, mark_scope_partial_ast};
use crate::ts::scope::store::TsScopeOutput;

#[derive(Debug)]
pub struct TsFileAnalysis {
    pub file: crate::internal_core::FileId,
    pub inventory: TsInventoryOutput,
    pub scope: TsScopeOutput,
    pub object_model: TsObjectModelOutput,
    pub(crate) value_flows: TsValueFlows,
}

pub(super) fn analyze_parsed_ts_file(
    interner: &StableKeyInterner,
    file: &SourceFile,
    parsed: &ParsedTsSource<'_>,
) -> TsFileAnalysis {
    let source = file.source.as_ref();
    if parsed.is_catastrophic() {
        return TsFileAnalysis {
            file: file.id,
            inventory: TsInventoryOutput::default(),
            scope: TsScopeOutput::default(),
            object_model: TsObjectModelOutput::default(),
            value_flows: TsValueFlows::default(),
        };
    }

    let semantic = SemanticBuilder::new().build(parsed.program()).semantic;
    let nodes = semantic.nodes();
    let mut inventory =
        extract_ts_inventory_from_program(interner, file, source, parsed.program(), nodes)
            .normalized(interner);
    let mut scope = extract_ts_scope_from_program(
        interner,
        file,
        source,
        parsed.program(),
        semantic.scoping(),
        nodes,
    )
    .normalized(interner);
    let mut object_model = extract_ts_object_model_from_program(
        interner,
        file,
        source,
        parsed.program(),
        semantic.scoping(),
        nodes,
        &inventory,
    );
    if !parsed.fully_parsed {
        mark_inventory_partial_ast(&mut inventory);
        mark_scope_partial_ast(&mut scope);
        mark_object_model_partial_ast(&mut object_model);
    }
    let value_flows = if parsed.fully_parsed {
        lower_ts_program_value_flows(source, parsed.program(), semantic.scoping())
    } else {
        TsValueFlows::default()
    };

    TsFileAnalysis {
        file: file.id,
        inventory,
        scope,
        object_model,
        value_flows,
    }
}
