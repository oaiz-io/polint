//! Data-flow questions for [`DataFlow::flows`](crate::sdk::facts::DataFlow::flows):
//! where tracked values come from, where they must not arrive and what cleans
//! them, answered with each flow's path, precision and what limited it.

use crate::core::FileId;
use crate::diagnostics::{Diagnostic, StructuredEvidenceV1, TextRange};

/// A kind of value a [`FlowSpec`] can declare unable to carry what it tracks: a
/// value of such a kind never holds taint and never reaches a sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum FlowValueKind {
    /// `context.Context` values.
    Context,
    /// Booleans.
    Boolean,
    /// Numbers: integers, floats, complex numbers, and named types of them.
    Number,
}

/// Where tracked values come from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct FlowSource {
    pub(crate) kind: FlowSourceKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum FlowSourceKind {
    Model(String),
    CallResult(String),
    CallArgumentPointee(String, u16),
    ParameterOfType(String),
    Named(Vec<String>),
    CallbackParameter(String, u16, u16),
}

impl FlowSource {
    /// The sources the data-flow models of one kind name: the built-in
    /// `http_request` (request data a gin or net/http handler reads) and
    /// `message_payload` (a Watermill handler's message), or a repository
    /// model's kind (`[[go_flow_source]]` in `.polint/models/*.toml`).
    pub fn model(kind: impl Into<String>) -> Self {
        Self {
            kind: FlowSourceKind::Model(kind.into()),
        }
    }

    /// The result of a call to `function`: a qualified name
    /// (`example.com/app/auth.Token`), a method as `Type.Method`, or a last name.
    pub fn call_result(function: impl Into<String>) -> Self {
        Self {
            kind: FlowSourceKind::CallResult(function.into()),
        }
    }

    /// What argument `argument` (counted from 0, without the receiver) of a call
    /// to `function` points to after the call, such as a decoder's target.
    pub fn call_argument_pointee(function: impl Into<String>, argument: usize) -> Self {
        Self {
            kind: FlowSourceKind::CallArgumentPointee(function.into(), clamp(argument)),
        }
    }

    /// Every parameter of a type, written as the type checker prints it
    /// (`*net/http.Request`).
    pub fn parameter_of_type(type_name: impl Into<String>) -> Self {
        Self {
            kind: FlowSourceKind::ParameterOfType(type_name.into()),
        }
    }

    /// Parameters, address-taken locals, package variables and struct fields
    /// read whose names contain one of `names`, ignoring case. This is a
    /// heuristic: names are not types.
    pub fn named<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            kind: FlowSourceKind::Named(names.into_iter().map(Into::into).collect()),
        }
    }

    /// Parameter `parameter` of a function literal passed as argument
    /// `argument` to `function`, such as a transaction callback's handle.
    pub fn callback_parameter(
        function: impl Into<String>,
        argument: usize,
        parameter: usize,
    ) -> Self {
        Self {
            kind: FlowSourceKind::CallbackParameter(
                function.into(),
                clamp(argument),
                clamp(parameter),
            ),
        }
    }
}

/// Where tracked values must not arrive.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct FlowSink {
    pub(crate) kind: FlowSinkKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum FlowSinkKind {
    Model(String),
    Call(String),
    CallArgument(String, u16),
    Returned,
}

impl FlowSink {
    /// The sinks the data-flow models of one kind name: the built-in `sql`
    /// (GORM and `database/sql` query text), `exec` (commands run), `log` and
    /// `publish` (Watermill), or a repository model's kind
    /// (`[[go_flow_sink]]` in `.polint/models/*.toml`).
    pub fn model(kind: impl Into<String>) -> Self {
        Self {
            kind: FlowSinkKind::Model(kind.into()),
        }
    }

    /// Any argument, or the receiver, of a call to `function`.
    pub fn call(function: impl Into<String>) -> Self {
        Self {
            kind: FlowSinkKind::Call(function.into()),
        }
    }

    /// Argument `position` (counted from 0, without the receiver) of a call to
    /// `function`.
    pub fn call_argument(function: impl Into<String>, position: usize) -> Self {
        Self {
            kind: FlowSinkKind::CallArgument(function.into(), clamp(position)),
        }
    }

    /// A value the function that holds a source returns.
    pub fn returned() -> Self {
        Self {
            kind: FlowSinkKind::Returned,
        }
    }
}

fn clamp(position: usize) -> u16 {
    u16::try_from(position).unwrap_or(u16::MAX)
}

/// A data-flow question: sources, sinks, sanitizers, and the kinds of values
/// that cannot carry what is tracked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct FlowSpec {
    pub(crate) sources: Vec<FlowSource>,
    pub(crate) sinks: Vec<FlowSink>,
    pub(crate) sanitizers: Vec<String>,
    pub(crate) untracked: Vec<FlowValueKind>,
    pub(crate) deeper_paths: bool,
}

impl FlowSpec {
    /// A question with no sources and no sinks yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a source.
    pub fn source(mut self, source: FlowSource) -> Self {
        self.sources.push(source);
        self
    }

    /// Adds a sink.
    pub fn sink(mut self, sink: FlowSink) -> Self {
        self.sinks.push(sink);
        self
    }

    /// Adds a sanitizer: a function whose result carries none of its
    /// arguments' taint (named like [`FlowSource::call_result`]'s function).
    /// The data-flow models' sanitizers always apply.
    pub fn sanitizer(mut self, function: impl Into<String>) -> Self {
        self.sanitizers.push(function.into());
        self
    }

    /// Declares a kind of value unable to carry what is tracked. Injection
    /// questions usually declare contexts, booleans and numbers; a question
    /// about a context or a value stored in one must not.
    pub fn untracked(mut self, kind: FlowValueKind) -> Self {
        if !self.untracked.contains(&kind) {
            self.untracked.push(kind);
        }
        self
    }

    /// Keeps three field steps of an access path instead of two, for flows
    /// through nested structs; slower, and still bounded by the step budgets.
    pub fn deeper_paths(mut self) -> Self {
        self.deeper_paths = true;
        self
    }
}

/// How sure a flow is: its least certain step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum FlowPrecision {
    /// Every call on the path has one known callee, and every library function
    /// it passes is modelled.
    Exact,
    /// A call on the path was resolved by variable-type analysis.
    SetupAware,
    /// The path passes a class-hierarchy call candidate, a call with no known
    /// callee, a package variable, or a library function without a model
    /// (assumed to pass its arguments to its result).
    Conservative,
    /// The path enters a function literal assumed to be called by the library
    /// function it was passed to.
    Heuristic,
}

/// What limited the analysis of part of the program.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum FlowUnknown {
    /// A package used up its step budget; flows through it may be missing.
    UnitBudget {
        /// The package path.
        unit: String,
    },
    /// The question's deadline passed; flows may be missing anywhere.
    Deadline,
    /// Calls nested deeper than the limit, or a recursive cycle that did not
    /// settle, were not followed into this function.
    CallDepth {
        /// The function not entered.
        function: String,
    },
    /// A dynamic call with no known callee: taint passed to it reaches only its
    /// result.
    UnresolvedCall {
        /// The function the call is written in.
        function: String,
        /// The call's line.
        line: u32,
    },
}

/// A place on a flow's path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct FlowStep {
    /// The scanned file, when the place is in one.
    pub file: Option<FileId>,
    /// The file's repository-relative path; empty for a function the compiler
    /// synthesized.
    pub path: String,
    /// The 1-based line, 0 when unknown.
    pub line: u32,
    /// The 1-based column, 0 when unknown.
    pub column: u32,
    /// The qualified name of the function the place is in.
    pub function: String,
}

/// One flow from a source to a sink.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Flow {
    /// Where the source is read.
    pub source: FlowStep,
    /// The sink: the call (or return) the tracked value reaches.
    pub sink: FlowStep,
    /// The call argument the value reaches the sink as (a method's receiver is
    /// 0), or none when the sink is a return.
    pub sink_argument: Option<usize>,
    /// The places the value passes, source first and sink last.
    pub steps: Vec<FlowStep>,
    /// The least certain step.
    pub precision: FlowPrecision,
    /// What limited the analysis of the functions the flow passes.
    pub unknowns: Vec<FlowUnknown>,
}

impl Flow {
    /// A diagnostic at the sink whose evidence is the flow's path, one located
    /// step per place: SARIF output renders it as a code flow.
    pub fn diagnostic(&self, rule_id: &str, message: impl Into<String>) -> Diagnostic {
        let range = TextRange::point(self.sink.line.max(1), self.sink.column.max(1));
        let mut diagnostic =
            Diagnostic::error(rule_id.to_string(), self.sink.path.clone(), range, message)
                .with_evidence("flow_source", step_label(&self.source))
                .with_evidence("flow_precision", precision_label(self.precision))
                .with_evidence("flow_steps", self.steps.len().to_string());
        if !self.unknowns.is_empty() {
            diagnostic = diagnostic.with_evidence(
                "flow_unknowns",
                self.unknowns
                    .iter()
                    .map(unknown_label)
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        if let Ok(evidence) = flow_evidence(self) {
            diagnostic = diagnostic.with_structured_evidence_v1(evidence);
        }
        diagnostic
    }
}

/// A [`DataFlow::flows`](crate::sdk::facts::DataFlow::flows) answer: the flows,
/// and what limited the search anywhere in the program. A flow a budget or the
/// deadline cut off is missing from `flows`, and its cause is in `unknowns`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FlowAnswer {
    /// The flows, one per sink reached, ordered by sink.
    pub flows: Vec<Flow>,
    /// Everything that limited the search, whether or not a flow passed it.
    pub unknowns: Vec<FlowUnknown>,
}

impl FlowAnswer {
    /// Whether nothing limited the search, so the absence of a flow is an
    /// answer.
    pub fn is_complete(&self) -> bool {
        !self.unknowns.iter().any(|unknown| {
            matches!(
                unknown,
                FlowUnknown::UnitBudget { .. }
                    | FlowUnknown::Deadline
                    | FlowUnknown::CallDepth { .. }
            )
        })
    }
}

fn step_label(step: &FlowStep) -> String {
    format!("{}:{}:{}", step.path, step.line, step.column)
}

pub(crate) fn precision_label(precision: FlowPrecision) -> &'static str {
    match precision {
        FlowPrecision::Exact => "exact",
        FlowPrecision::SetupAware => "setup_aware",
        FlowPrecision::Conservative => "conservative",
        FlowPrecision::Heuristic => "heuristic",
    }
}

fn unknown_label(unknown: &FlowUnknown) -> String {
    match unknown {
        FlowUnknown::UnitBudget { unit } => format!("unit budget: {unit}"),
        FlowUnknown::Deadline => "deadline".to_string(),
        FlowUnknown::CallDepth { function } => format!("call depth: {function}"),
        FlowUnknown::UnresolvedCall { function, line } => {
            format!("unresolved call: {function} line {line}")
        }
    }
}

/// The flow as structured evidence: a path whose edges each carry the location
/// of one step.
pub(crate) fn flow_evidence(flow: &Flow) -> Result<StructuredEvidenceV1, String> {
    const MAX_EDGES: usize = 96;
    let precision = match flow.precision {
        FlowPrecision::Exact => "Exact",
        FlowPrecision::SetupAware => "SetupAware",
        FlowPrecision::Conservative => "Conservative",
        FlowPrecision::Heuristic => "Heuristic",
    };
    let status = if matches!(
        flow.precision,
        FlowPrecision::Exact | FlowPrecision::SetupAware
    ) {
        "Exact"
    } else {
        "Heuristic"
    };
    let confidence = match flow.precision {
        FlowPrecision::Exact => "High",
        FlowPrecision::SetupAware | FlowPrecision::Conservative => "Medium",
        FlowPrecision::Heuristic => "Low",
    };
    let located: Vec<&FlowStep> = flow
        .steps
        .iter()
        .filter(|step| !step.path.is_empty() && step.line > 0)
        .collect();
    let total = located.len();
    let rendered = total.min(MAX_EDGES);
    let edges: Vec<serde_json::Value> = located
        .iter()
        .take(rendered)
        .enumerate()
        .map(|(index, step)| {
            serde_json::json!({
                "id": index as u64,
                "stable_key": format!("edge:flow:{index}:{}", step_label(step)),
                "kind": "DataTaint",
                "status": status,
                "precision": precision,
                "provenance": "Query",
                "validation": "RendererValidated",
                "confidence": confidence,
                "summary_stable_key": null,
                "expansion": { "state": "none" },
                "location": {
                    "uri": step.path,
                    "range": {
                        "start_line": step.line,
                        "start_col": step.column.max(1),
                        "end_line": step.line,
                        "end_col": step.column.max(1),
                    },
                },
            })
        })
        .collect();
    let path_key = located
        .iter()
        .map(|step| step_label(step))
        .collect::<Vec<_>>()
        .join(" -> ");
    let key = crate::cache::stable_hash(&[path_key.as_str()]);
    let value = serde_json::json!({
        "version": 1,
        "bundle": {
            "id": 0,
            "stable_key": format!("bundle:flow:{key}"),
            "diagnostic_stable_key": format!("flow:{key}"),
            "status": status,
            "precision": precision,
            "provenance": "Query",
            "validation": "RendererValidated",
            "confidence": confidence,
            "replay_key": format!("replay:flow:{key}"),
        },
        "paths": [{
            "id": 0,
            "stable_key": format!("path:flow:{key}"),
            "rank": 0,
            "status": status,
            "hidden_node_count": 0,
            "nodes": (0..=rendered as u64).collect::<Vec<_>>(),
            "edges": edges,
            "omitted_regions": [],
            "total_edges": total as u64,
            "rendered_edges": rendered as u64,
            "edges_truncated": total > rendered,
        }],
        "unknowns": [],
        "omitted_regions": [],
        "limits": {
            "max_paths": 5,
            "max_edges_per_path": MAX_EDGES as u64,
            "max_unknowns": 32,
            "max_omitted_regions": 32,
            "total_paths": 1,
            "rendered_paths": 1,
            "paths_truncated": false,
            "total_unknowns": 0,
            "rendered_unknowns": 0,
            "unknowns_truncated": false,
            "total_omitted_regions": 0,
            "rendered_omitted_regions": 0,
            "omitted_regions_truncated": false,
        },
    });
    StructuredEvidenceV1::try_from_value(value)
}
