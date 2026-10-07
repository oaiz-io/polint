//! Answers data-flow questions with the taint solver over the Go program's flow
//! bodies and the data-flow models.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::analysis_neutral::taint::ir::{FlowProgram, FnId, Pos, ValueKinds};
use crate::analysis_neutral::taint::models::Models;
use crate::analysis_neutral::taint::solver::{
    self, Arguments, ExternalModels, Matcher, Precision, Query, SinkSpec, Solver, SourceSpec,
    Unknown,
};
use crate::core::{AnalysisDb, FileId};
use crate::sdk::dataflow::{
    Flow, FlowAnswer, FlowPrecision, FlowSinkKind, FlowSourceKind, FlowSpec, FlowStep, FlowUnknown,
    FlowValueKind,
};

/// The steps one package may take answering one question.
const UNIT_BUDGET: u64 = 2_000_000;

/// How long one question may take.
const DEADLINE: Duration = Duration::from_secs(120);

/// Answers `spec` over the run's Go flow program. Without one, a scan that has
/// Go files answers no flows and one [`FlowUnknown::NoProgram`], so the empty
/// answer is visibly not a proof; a scan without Go files answers nothing.
pub(crate) fn flows(db: &AnalysisDb, spec: &FlowSpec) -> FlowAnswer {
    let Some(program) = db.go_flow_program() else {
        let has_go_files = db
            .files()
            .iter()
            .any(|file| file.language == crate::core::Language::Go);
        return FlowAnswer {
            flows: Vec::new(),
            unknowns: if has_go_files {
                vec![FlowUnknown::NoProgram]
            } else {
                Vec::new()
            },
        };
    };
    let models = db.go_flow_models().map(|loaded| &loaded.models);
    let query = query(spec, models, Instant::now() + DEADLINE);
    if query.sources.is_empty() || query.sinks.is_empty() {
        return FlowAnswer::default();
    }
    let external = models.map(Models::external).unwrap_or_default();
    let output = solve(program.program(), program.index(), &external, &query);
    let files: BTreeMap<&str, FileId> = db
        .files()
        .iter()
        .map(|file| (file.relative_path.as_str(), file.id))
        .collect();
    let program = program.program();
    FlowAnswer {
        flows: output
            .flows
            .iter()
            .map(|flow| sdk_flow(program, &files, flow))
            .collect(),
        unknowns: output.unknowns.iter().map(sdk_unknown).collect(),
    }
}

fn solve(
    program: &FlowProgram,
    index: &crate::analysis_neutral::taint::index::ProgramIndex,
    external: &ExternalModels,
    query: &Query,
) -> solver::QueryOutput {
    let started = Instant::now();
    let output = Solver::new(program, index, external, query).run();
    tracing::debug!(
        target: "polint::dataflow",
        elapsed_ms = started.elapsed().as_millis() as u64,
        flows = output.flows.len(),
        summaries = output.summaries_computed,
        recomputed = output.summaries_recomputed,
        steps = output.steps,
        "data-flow question answered"
    );
    output
}

/// The solver's query for a question.
fn query(spec: &FlowSpec, models: Option<&Models>, deadline: Instant) -> Query {
    let mut sources = Vec::new();
    for source in &spec.sources {
        match &source.kind {
            FlowSourceKind::Model(kind) => {
                if let Some(models) = models {
                    sources.extend(models.sources_of_kind(kind));
                }
            }
            FlowSourceKind::CallResult(function) => sources.push(SourceSpec::CallResult {
                callee: Matcher::name(function),
                result: None,
            }),
            FlowSourceKind::CallArgumentPointee(function, argument) => {
                sources.push(SourceSpec::CallArgumentPointee {
                    callee: Matcher::name(function),
                    argument: *argument,
                });
            }
            FlowSourceKind::ParameterOfType(type_name) => sources.push(SourceSpec::Parameter {
                function: None,
                index: None,
                type_name: Some(type_name.clone()),
            }),
            FlowSourceKind::Named(names) => sources.push(SourceSpec::Named {
                names: names.clone(),
            }),
            FlowSourceKind::CallbackParameter(function, argument, parameter) => {
                sources.push(SourceSpec::CallbackParameter {
                    callee: Matcher::name(function),
                    argument: *argument,
                    parameter: *parameter,
                });
            }
        }
    }
    let mut sinks = Vec::new();
    for sink in &spec.sinks {
        match &sink.kind {
            FlowSinkKind::Model(kind) => {
                if let Some(models) = models {
                    sinks.extend(models.sinks_of_kind(kind));
                }
            }
            FlowSinkKind::Call(function) => sinks.push(SinkSpec::CallArgument {
                callee: Matcher::name(function),
                arguments: Arguments::All,
                receiver: true,
            }),
            FlowSinkKind::CallArgument(function, position) => sinks.push(SinkSpec::CallArgument {
                callee: Matcher::name(function),
                arguments: Arguments::Only(vec![*position]),
                receiver: false,
            }),
            FlowSinkKind::Returned => sinks.push(SinkSpec::Return),
        }
    }
    let untracked = spec.untracked.iter().fold(ValueKinds::NONE, |kinds, kind| {
        kinds.union(match kind {
            FlowValueKind::Context => ValueKinds::CONTEXT,
            FlowValueKind::Boolean => ValueKinds::BOOLEAN,
            FlowValueKind::Number => ValueKinds::NUMBER,
        })
    });
    Query {
        sources,
        sinks,
        sanitizers: spec
            .sanitizers
            .iter()
            .map(|name| Matcher::name(name))
            .collect(),
        k: if spec.deeper_paths { 3 } else { 2 },
        unit_budget: UNIT_BUDGET,
        deadline: Some(deadline),
        untracked,
        ..Query::default()
    }
}

fn sdk_flow(program: &FlowProgram, files: &BTreeMap<&str, FileId>, flow: &solver::Flow) -> Flow {
    let step = |function: FnId, pos: Pos| {
        let body = program.function(function);
        FlowStep {
            file: files.get(body.file.as_str()).copied(),
            path: body.file.clone(),
            line: pos.line,
            column: pos.col,
            function: body.name.clone(),
        }
    };
    Flow {
        source: step(flow.source_function, flow.source_pos),
        sink: step(flow.sink_function, flow.sink_pos),
        sink_argument: flow.sink_argument.map(usize::from),
        steps: flow
            .steps
            .iter()
            .map(|(function, pos)| step(*function, *pos))
            .collect(),
        precision: match flow.precision {
            Precision::Exact => FlowPrecision::Exact,
            Precision::SetupAware => FlowPrecision::SetupAware,
            Precision::Conservative => FlowPrecision::Conservative,
            Precision::Heuristic => FlowPrecision::Heuristic,
        },
        unknowns: flow.unknowns.iter().map(sdk_unknown).collect(),
    }
}

fn sdk_unknown(unknown: &Unknown) -> FlowUnknown {
    match unknown {
        Unknown::UnitBudget(unit) => FlowUnknown::UnitBudget { unit: unit.clone() },
        Unknown::Deadline => FlowUnknown::Deadline,
        Unknown::CallDepth(function) => FlowUnknown::CallDepth {
            function: function.clone(),
        },
        Unknown::UnresolvedCall { function, line } => FlowUnknown::UnresolvedCall {
            function: function.clone(),
            line: *line,
        },
    }
}
