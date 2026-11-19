//! Lookup indexes behind the call-graph and Go type fact views.
//!
//! Built once per database on first use and dropped whenever a fact store
//! changes, like the other fact-view indexes.

use std::collections::HashMap;

use crate::analysis::refined_calls::facts::RefinedCallEdgeFact;
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallPrecision, CallTargetStatus, ClaimTier,
};
use crate::analysis_neutral::ids::CallSiteId;
use crate::core::{AnalysisDb, FileId, FunctionId, Language, Span, SymbolId};

/// The call graph rules read: the refined call layer's edges that claim a
/// target, by caller and by callee.
///
/// The refined layer holds every base call target and each refinement tier's
/// edges, so one site may name the same callee more than once; the index keeps
/// the strongest claim for each site and callee.
#[derive(Debug, Clone, Default)]
pub(crate) struct CallGraphIndex {
    /// Indices into the refined call-edge family, in its storage order.
    pub(crate) edges: Vec<usize>,
    /// For each edge, the index of its call site in the call-site family.
    pub(crate) sites: Vec<usize>,
    /// For each edge, the precision it claims.
    pub(crate) precisions: Vec<CallPrecision>,
    pub(crate) by_caller: HashMap<FunctionId, Vec<u32>>,
    pub(crate) by_callee: HashMap<FunctionId, Vec<u32>>,
}

/// What an edge calls, for keeping one edge per site and callee.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CalleeKey<'a> {
    Function(FunctionId),
    Symbol(SymbolId),
    Label(&'a str),
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
        let mut by_key = HashMap::<(CallSiteId, CalleeKey<'_>), u32>::new();
        for (row, edge) in db.refined_call_edges().iter().enumerate() {
            let (Some(precision), Some(callee)) = (claimed_precision(edge), callee_key(edge))
            else {
                continue;
            };
            let Some(&site) = site_index.get(&edge.site) else {
                continue;
            };
            if let Some(&kept) = by_key.get(&(edge.site, callee.clone())) {
                let kept = kept as usize;
                if claim_rank(precision) < claim_rank(index.precisions[kept]) {
                    index.edges[kept] = row;
                    index.precisions[kept] = precision;
                }
                continue;
            }
            let id = u32::try_from(index.edges.len()).expect("fewer than 2^32 call edges");
            index.edges.push(row);
            index.sites.push(site);
            index.precisions.push(precision);
            index.by_caller.entry(edge.caller).or_default().push(id);
            if let CalleeKey::Function(callee) = callee {
                index.by_callee.entry(callee).or_default().push(id);
            }
            by_key.insert((edge.site, callee), id);
        }
        index
    }

    pub(crate) fn edge<'a>(&self, db: &'a AnalysisDb, edge: u32) -> IndexedCallEdge<'a> {
        let edge = edge as usize;
        let refined = &db.refined_call_edges()[self.edges[edge]];
        let site = &db.call_sites()[self.sites[edge]];
        IndexedCallEdge {
            caller: refined.caller,
            function: refined.target_function,
            symbol: refined.target_symbol,
            label: refined.synthetic_target.as_deref().unwrap_or_default(),
            file: site.file,
            span: &site.span,
            precision: self.precisions[edge],
            algorithm: refined.algorithm,
        }
    }
}

/// One call-graph edge as the refined call layer recorded it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IndexedCallEdge<'a> {
    pub(crate) caller: FunctionId,
    pub(crate) function: Option<FunctionId>,
    pub(crate) symbol: Option<SymbolId>,
    /// The callee's label when it is neither a function nor a symbol.
    pub(crate) label: &'a str,
    pub(crate) file: FileId,
    pub(crate) span: &'a Span,
    /// The precision the edge claims, after its status capped it.
    pub(crate) precision: CallPrecision,
    pub(crate) algorithm: CallAlgorithm,
}

/// The precision a refined edge claims for its target, or `None` when it
/// claims no target. A resolved edge claims its precision; an ambiguous edge
/// names one of several candidates, so it claims no more than conservative.
fn claimed_precision(edge: &RefinedCallEdgeFact) -> Option<CallPrecision> {
    match edge.status {
        CallTargetStatus::Resolved => Some(edge.precision),
        CallTargetStatus::Ambiguous if edge.precision.tier() == ClaimTier::Exact => {
            Some(CallPrecision::Conservative)
        }
        CallTargetStatus::Ambiguous => Some(edge.precision),
        CallTargetStatus::Unresolved
        | CallTargetStatus::Unsupported
        | CallTargetStatus::SetupMissing
        | CallTargetStatus::BudgetExceeded
        | CallTargetStatus::Rejected
        | CallTargetStatus::Unreachable => None,
    }
}

/// The callee of an edge that is a call-graph edge: a function, a symbol, or
/// a label other than a type conversion, which is written like a call but
/// calls nothing.
fn callee_key(edge: &RefinedCallEdgeFact) -> Option<CalleeKey<'_>> {
    if let Some(function) = edge.target_function {
        return Some(CalleeKey::Function(function));
    }
    if let Some(symbol) = edge.target_symbol {
        return Some(CalleeKey::Symbol(symbol));
    }
    edge.synthetic_target
        .as_deref()
        .filter(|label| !label.is_empty() && !label.starts_with("go:conversion:"))
        .map(CalleeKey::Label)
}

/// Orders precisions by the claim they make, strongest first, and by the
/// precision within one tier.
fn claim_rank(precision: CallPrecision) -> (ClaimTier, CallPrecision) {
    (precision.tier(), precision)
}

/// The typed Go frontend's rows, joined to the syntax functions and grouped
/// by the names rules ask about.
#[derive(Debug, Clone, Default)]
pub(crate) struct GoTypesIndex {
    pub(crate) function_by_id: HashMap<FunctionId, usize>,
    /// The syntax function each typed function identity (`pkg.F`,
    /// `(*pkg.T).M`) names, and back.
    pub(crate) function_by_qualified: HashMap<String, FunctionId>,
    pub(crate) qualified_by_function: HashMap<FunctionId, String>,
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
            index
                .function_by_qualified
                .entry(function.qualified.clone())
                .or_insert(syntax.id);
            index
                .qualified_by_function
                .entry(syntax.id)
                .or_insert_with(|| function.qualified.clone());
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
