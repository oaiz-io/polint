//! Lookup indexes behind the call-graph and Go type fact views.
//!
//! Built once per database on first use and dropped whenever a fact store
//! changes, like the other fact-view indexes.

use std::collections::HashMap;

use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallPrecision, CallTargetFact, CallTargetStatus,
};
use crate::analysis_neutral::ids::CallSiteId;
use crate::core::{AnalysisDb, FileId, FunctionId, Language, Span, SymbolId};

/// Call targets that are edges of the call graph, by caller and by callee.
#[derive(Debug, Clone, Default)]
pub(crate) struct CallGraphIndex {
    /// Indices into the call-target family, in its storage order.
    pub(crate) edges: Vec<usize>,
    /// For each edge, the index of its call site in the call-site family.
    pub(crate) sites: Vec<usize>,
    pub(crate) by_caller: HashMap<FunctionId, Vec<u32>>,
    pub(crate) by_callee: HashMap<FunctionId, Vec<u32>>,
}

impl CallGraphIndex {
    pub(crate) fn build(db: &AnalysisDb) -> Self {
        let site_index = db
            .call_sites()
            .iter()
            .enumerate()
            .map(|(index, site)| (site.id, index))
            .collect::<HashMap<CallSiteId, usize>>();
        let mut index = Self::default();
        for (target_index, target) in db.call_targets().iter().enumerate() {
            if !is_call_graph_edge(target) {
                continue;
            }
            let Some(&site) = site_index.get(&target.site) else {
                continue;
            };
            let edge = u32::try_from(index.edges.len()).expect("fewer than 2^32 call edges");
            index.edges.push(target_index);
            index.sites.push(site);
            index.by_caller.entry(target.caller).or_default().push(edge);
            if let Some(callee) = target.target_function {
                index.by_callee.entry(callee).or_default().push(edge);
            }
        }
        index
    }

    pub(crate) fn edge<'a>(&self, db: &'a AnalysisDb, edge: u32) -> IndexedCallEdge<'a> {
        let edge = edge as usize;
        let target = &db.call_targets()[self.edges[edge]];
        let site = &db.call_sites()[self.sites[edge]];
        IndexedCallEdge {
            caller: target.caller,
            function: target.target_function,
            symbol: target.target_symbol,
            label: target.synthetic_target.as_deref().unwrap_or_default(),
            file: site.file,
            span: &site.span,
            precision: target.precision,
            algorithm: target.algorithm,
        }
    }
}

/// One call-graph edge as the call layer recorded it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IndexedCallEdge<'a> {
    pub(crate) caller: FunctionId,
    pub(crate) function: Option<FunctionId>,
    pub(crate) symbol: Option<SymbolId>,
    /// The callee's label when it is neither a function nor a symbol.
    pub(crate) label: &'a str,
    pub(crate) file: FileId,
    pub(crate) span: &'a Span,
    pub(crate) precision: CallPrecision,
    pub(crate) algorithm: CallAlgorithm,
}

/// A resolved call target is a call-graph edge unless it names a type
/// conversion, which is written like a call but calls nothing.
fn is_call_graph_edge(target: &CallTargetFact) -> bool {
    target.status == CallTargetStatus::Resolved
        && (target.target_function.is_some()
            || target.target_symbol.is_some()
            || target
                .synthetic_target
                .as_deref()
                .is_some_and(|label| !label.starts_with("go:conversion:")))
}

/// The typed Go frontend's rows, joined to the syntax functions and grouped
/// by the names rules ask about.
#[derive(Debug, Clone, Default)]
pub(crate) struct GoTypesIndex {
    pub(crate) function_by_id: HashMap<FunctionId, usize>,
    pub(crate) params_by_function: HashMap<String, Vec<usize>>,
    pub(crate) fields_by_owner: HashMap<String, Vec<usize>>,
    pub(crate) instantiations_by_generic: HashMap<String, Vec<usize>>,
    pub(crate) method_set_by_type: HashMap<String, usize>,
    pub(crate) implements_by_type: HashMap<String, Vec<usize>>,
    pub(crate) implementers_by_interface: HashMap<String, Vec<usize>>,
}

impl GoTypesIndex {
    pub(crate) fn build(db: &AnalysisDb) -> Self {
        let mut index = Self::default();
        let mut syntax_functions = HashMap::<(crate::core::FileId, u32, u32), Vec<_>>::new();
        for function in db
            .functions()
            .iter()
            .filter(|function| function.language == Language::Go)
        {
            syntax_functions
                .entry((
                    function.file,
                    function.span.start_byte,
                    function.span.end_byte,
                ))
                .or_default()
                .push(function);
        }
        for (position, function) in db.go_semantic_functions().iter().enumerate() {
            let (Some(file), Some(span)) = (function.file, function.span.as_ref()) else {
                continue;
            };
            let Some(syntax) = syntax_functions
                .get(&(file, span.start_byte, span.end_byte))
                .and_then(|candidates| {
                    candidates
                        .iter()
                        .find(|candidate| candidate.name == function.name)
                })
            else {
                continue;
            };
            index.function_by_id.entry(syntax.id).or_insert(position);
        }
        for (position, param) in db.go_semantic_params().iter().enumerate() {
            index
                .params_by_function
                .entry(param.function.clone())
                .or_default()
                .push(position);
        }
        for (position, field) in db.go_semantic_fields().iter().enumerate() {
            index
                .fields_by_owner
                .entry(field.owner.clone())
                .or_default()
                .push(position);
        }
        for (position, instantiation) in db.go_semantic_instantiations().iter().enumerate() {
            index
                .instantiations_by_generic
                .entry(instantiation.generic.clone())
                .or_default()
                .push(position);
        }
        for (position, method_set) in db.go_semantic_method_sets().iter().enumerate() {
            index
                .method_set_by_type
                .entry(method_set.type_name.clone())
                .or_insert(position);
        }
        for (position, implements) in db.go_semantic_implements().iter().enumerate() {
            index
                .implements_by_type
                .entry(implements.type_name.clone())
                .or_default()
                .push(position);
            index
                .implementers_by_interface
                .entry(implements.interface.clone())
                .or_default()
                .push(position);
        }
        index
    }
}
