//! Value flow of one TypeScript or JavaScript file, lowered from oxc's
//! semantic model into rows the points-to solver reads.
//!
//! The lowering is total: every expression of a lowered file is described by
//! rows, or is an unknown operation whose result is open and whose operands
//! escape. Openness then follows escape: a value that reaches code the rows do
//! not describe may be read and called by that code, so the solver opens what
//! it can reach. A file with `with`, a direct `eval` or a partial parse is not
//! lowered at all, and its calls pass open values.
//!
//! [`described`] decides which call sites the rows describe completely, so
//! that the graph names the rows' callee there and keeps the earlier callee
//! everywhere else.

pub(crate) mod described;
#[cfg(feature = "lang-typescript")]
mod lower;

#[cfg(feature = "lang-typescript")]
pub(crate) use lower::{lower_ts_program_value_flows, lower_ts_source_value_flows};

/// A byte range of the lowered file: a function's span or a call site's span,
/// as the frontend records them.
pub(crate) type FlowSpan = (u32, u32);

/// A variable of the lowered rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum FlowVar {
    /// A binding of the file, by oxc symbol index. Top-level bindings of a
    /// script are file-scoped, as in a CommonJS module.
    Local(u32),
    /// The value of one expression.
    Temp(u32),
    /// The function object of the function with this span.
    Function(FlowSpan),
    /// A positional parameter of the function with this span.
    Param(FlowSpan, u8),
    /// The `this` of the function with this span.
    This(FlowSpan),
    /// The returned value of the function with this span.
    Return(FlowSpan),
    /// A value the rows do not describe.
    Open,
    /// Values that reach code the rows do not describe: they escape.
    Sink,
    /// Primitive values only: no object, no function.
    Prim,
}

/// One flow row of a lowered file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FlowRow {
    /// `dst` holds every value of `src`.
    Copy { dst: FlowVar, src: FlowVar },
    /// `dst` holds the object allocated at `object`.
    Alloc { dst: FlowVar, object: FlowSpan },
    /// `dst` holds the values of `field` of the objects of `base`.
    Load {
        dst: FlowVar,
        base: FlowVar,
        field: Box<str>,
    },
    /// `field` of the objects of `base` holds every value of `src`.
    Store {
        base: FlowVar,
        field: Box<str>,
        src: FlowVar,
    },
}

/// One call site of a lowered file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FlowCall {
    /// The call site's span, as the frontend records it.
    pub(crate) site: FlowSpan,
    pub(crate) callee: FlowVar,
    /// The value `this` is bound to, when the call passes one.
    pub(crate) receiver: Option<FlowVar>,
    pub(crate) arguments: Vec<FlowVar>,
    /// The first argument written as a spread: it and every later argument
    /// reach the callee at positions the rows do not know.
    pub(crate) spread_from: Option<usize>,
    pub(crate) result: FlowVar,
    /// The object a `new` allocates, passed as the receiver.
    pub(crate) constructs: Option<FlowSpan>,
}

/// A function whose body was lowered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LoweredFunction {
    pub(crate) span: FlowSpan,
    /// Positional parameters, each a [`FlowVar::Param`].
    pub(crate) params: u8,
}

/// The lowered value flow of one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TsValueFlows {
    /// False when the file was not lowered; it then has no rows.
    pub(crate) lowered: bool,
    pub(crate) rows: Vec<FlowRow>,
    pub(crate) calls: Vec<FlowCall>,
    pub(crate) functions: Vec<LoweredFunction>,
}

/// Lowers a file that was not lowered during extraction.
#[cfg(not(feature = "lang-typescript"))]
pub(crate) fn lower_ts_source_value_flows(_file: &crate::analysis_api::SourceFile) -> TsValueFlows {
    TsValueFlows::default()
}
