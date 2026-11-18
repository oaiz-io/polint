//! Bounded frontend callable-shape facts, projected through the semantic graph.

use crate::analysis_neutral::ids::CallSiteId;
use crate::internal_core::FunctionId;

#[cfg(feature = "lang-typescript")]
mod extract;
#[cfg(feature = "lang-typescript")]
pub(super) use extract::cannot_be_callable;
#[cfg(feature = "lang-typescript")]
pub(super) use extract::collect_callable_flows;

/// A bounded, heuristic callable flow extracted from the frontend AST. The graph
/// projection owns stable identity and solver constraints; this is not a call target.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct TsCallableFlow {
    pub(super) site: CallSiteId,
    pub(super) target_function: FunctionId,
    pub(super) kind: TsCallableFlowKind,
    pub(super) binding: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum TsCallableFlowKind {
    Token,
    Receiver,
}
