use crate::analysis_neutral::taint::index::ProgramIndex;
use crate::analysis_neutral::taint::ir::{
    Algorithm, CallMode, Callee, FlowFunction, FlowProgram, Operand, Pos, Slot, Step, Stmt,
    StmtKind,
};
use crate::analysis_neutral::taint::solver::{
    Arguments, ExternalModels, Matcher, Query, SinkSpec, Solver, SourceSpec,
};

/// Builds one body: named parameters first, then statements.
struct Body {
    function: FlowFunction,
    line: u32,
}

impl Body {
    fn new(name: &str, params: &[&str]) -> Body {
        let mut function = FlowFunction {
            name: name.to_string(),
            unit: "example.com/app".to_string(),
            file: "app.go".to_string(),
            ..FlowFunction::default()
        };
        for (slot, param) in params.iter().enumerate() {
            function.params.push(slot as Slot);
            function.param_types.push("string".to_string());
            function.names.insert(slot as Slot, (*param).to_string());
        }
        function.slots = params.len() as u32;
        Body { function, line: 1 }
    }

    fn free(mut self, name: &str) -> Body {
        let slot = self.slot();
        self.function.free.push(slot);
        self.function.names.insert(slot, name.to_string());
        self
    }

    fn slot(&mut self) -> Slot {
        let slot = self.function.slots;
        self.function.slots += 1;
        slot
    }

    fn push(&mut self, kind: StmtKind) {
        self.line += 1;
        self.function.stmts.push(Stmt {
            kind,
            pos: Pos {
                line: self.line,
                col: 1,
                ..Pos::default()
            },
        });
    }

    fn call(&mut self, callee: &str, args: &[Operand]) -> Slot {
        let dst = self.slot();
        self.push(StmtKind::Call {
            dst: Some(dst),
            callees: vec![Callee::External(callee.to_string())],
            algorithm: Algorithm::Static,
            builtin: None,
            closure: None,
            args: args.to_vec(),
            mode: CallMode::Call,
            interface: None,
        });
        dst
    }

    fn ret(&mut self, values: &[Operand]) {
        self.push(StmtKind::Return {
            values: values.to_vec(),
        });
        self.function.results = values.len() as u16;
    }
}

fn s(slot: Slot) -> Operand {
    Operand::Slot(slot)
}

fn program(bodies: Vec<Body>) -> FlowProgram {
    let mut program = FlowProgram::default();
    for body in bodies {
        program.add(body.function);
    }
    program.link();
    program
}

fn query(sources: Vec<SourceSpec>, sink: &str, sanitizers: &[&str]) -> Query {
    Query {
        sources,
        sinks: vec![SinkSpec::CallArgument {
            callee: Matcher::name(sink),
            arguments: Arguments::All,
            receiver: true,
        }],
        sanitizers: sanitizers.iter().map(|name| Matcher::name(name)).collect(),
        ..Query::default()
    }
}

fn named(name: &str) -> Vec<SourceSpec> {
    vec![SourceSpec::Named {
        names: vec![name.to_string()],
    }]
}

fn run(
    program: &FlowProgram,
    query: &Query,
) -> crate::analysis_neutral::taint::solver::QueryOutput {
    let index = ProgramIndex::new(program);
    let models = ExternalModels::default();
    Solver::new(program, &index, &models, query).run()
}

fn flows(program: &FlowProgram, query: &Query) -> usize {
    run(program, query).flows.len()
}

/// sink(value) inside a helper called with the token.
fn helper_program(sanitize: bool) -> FlowProgram {
    let mut helper = Body::new("app.helper", &["value"]);
    let arg = if sanitize {
        s(helper.call("app.sanitize", &[s(0)]))
    } else {
        s(0)
    };
    helper.call("app.sink", &[arg]);
    helper.ret(&[]);
    let mut entry = Body::new("app.entry", &["token"]);
    entry.call("app.helper", &[s(0)]);
    entry.ret(&[]);
    program(vec![helper, entry])
}

#[test]
fn taint_reaches_a_sink_through_a_helper() {
    let program = helper_program(false);
    let output = run(&program, &query(named("token"), "sink", &["sanitize"]));
    assert_eq!(output.flows.len(), 1);
    let flow = &output.flows[0];
    assert_eq!(program.function(flow.source_function).name, "app.entry");
    assert_eq!(program.function(flow.sink_function).name, "app.helper");
    // The source, the call into the helper, and the sink.
    assert!(flow.steps.len() >= 2, "{} steps", flow.steps.len());
}

#[test]
fn a_sanitizer_inside_the_helper_stops_it() {
    let program = helper_program(true);
    assert_eq!(
        flows(&program, &query(named("token"), "sink", &["sanitize"])),
        0
    );
}

/// carrier := carrier{field: token}; sink(carrier.<read>) as SSA writes it:
/// the local and the literal are memory cells.
fn field_program(read: &str) -> FlowProgram {
    let mut program = FlowProgram::default();
    let field = program.symbol("field");
    let other = program.symbol("other");
    let read = if read == "field" { field } else { other };
    let mut entry = Body::new("app.entry", &["token"]);
    let carrier = entry.slot();
    entry.function.names.insert(carrier, "carrier".to_string());
    entry.push(StmtKind::Alloc { dst: carrier });
    let literal = entry.slot();
    entry.push(StmtKind::Alloc { dst: literal });
    let field_addr = entry.slot();
    entry.push(StmtKind::Alias {
        dst: field_addr,
        dst_path: vec![Step::Deref],
        base: s(literal),
        path: vec![Step::Deref, Step::Field(field)],
    });
    entry.push(StmtKind::Store {
        target: s(field_addr),
        path: vec![Step::Deref],
        value: s(0),
    });
    let other_addr = entry.slot();
    entry.push(StmtKind::Alias {
        dst: other_addr,
        dst_path: vec![Step::Deref],
        base: s(literal),
        path: vec![Step::Deref, Step::Field(other)],
    });
    entry.push(StmtKind::Store {
        target: s(other_addr),
        path: vec![Step::Deref],
        value: Operand::Const(0),
    });
    let value = entry.slot();
    entry.push(StmtKind::Load {
        dst: value,
        base: s(literal),
        step: Step::Deref,
    });
    entry.push(StmtKind::Store {
        target: s(carrier),
        path: vec![Step::Deref],
        value: s(value),
    });
    let read_addr = entry.slot();
    entry.push(StmtKind::Alias {
        dst: read_addr,
        dst_path: vec![Step::Deref],
        base: s(carrier),
        path: vec![Step::Deref, Step::Field(read)],
    });
    let read_value = entry.slot();
    entry.push(StmtKind::Load {
        dst: read_value,
        base: s(read_addr),
        step: Step::Deref,
    });
    entry.call("app.sink", &[s(read_value)]);
    entry.ret(&[]);
    entry.function.consts.push("x".to_string());
    program.add(entry.function);
    program.link();
    program
}

#[test]
fn taint_follows_a_struct_field_through_memory() {
    assert_eq!(
        flows(&field_program("field"), &query(named("token"), "sink", &[])),
        1
    );
}

#[test]
fn another_field_of_the_same_struct_is_clean() {
    assert_eq!(
        flows(&field_program("other"), &query(named("token"), "sink", &[])),
        0
    );
}

#[test]
fn taint_returns_from_a_carrying_helper() {
    let mut carry = Body::new("app.carry", &["value"]);
    carry.ret(&[s(0)]);
    let mut clean = Body::new("app.clean", &["value"]);
    clean.ret(&[Operand::Const(0)]);
    clean.function.consts.push("safe".to_string());
    let mut entry = Body::new("app.entry", &["token"]);
    let carried = entry.call("app.carry", &[s(0)]);
    entry.call("app.sink", &[s(carried)]);
    let cleaned = entry.call("app.clean", &[s(0)]);
    entry.call("app.sinkClean", &[s(cleaned)]);
    entry.ret(&[]);
    let program = program(vec![carry, clean, entry]);
    assert_eq!(flows(&program, &query(named("token"), "sink", &[])), 1);
    assert_eq!(flows(&program, &query(named("token"), "sinkClean", &[])), 0);
}

#[test]
fn taint_in_a_captured_variable_reaches_the_closure_body() {
    // entry(token): cell := alloc; *cell = token; run := closure body [cell]; run()
    let mut closure = Body::new("app.entry$1", &[]).free("token");
    let loaded = closure.slot();
    closure.push(StmtKind::Load {
        dst: loaded,
        base: s(0),
        step: Step::Deref,
    });
    closure.call("app.sink", &[s(loaded)]);
    closure.ret(&[]);
    let mut entry = Body::new("app.entry", &["token"]);
    let cell = entry.slot();
    entry.push(StmtKind::Alloc { dst: cell });
    entry.push(StmtKind::Store {
        target: s(cell),
        path: vec![Step::Deref],
        value: s(0),
    });
    let run_value = entry.slot();
    entry.push(StmtKind::Closure {
        dst: run_value,
        function: Some(Callee::External("app.entry$1".to_string())),
        bindings: vec![s(cell)],
    });
    let result = entry.slot();
    entry.push(StmtKind::Call {
        dst: Some(result),
        callees: vec![Callee::External("app.entry$1".to_string())],
        algorithm: Algorithm::Static,
        builtin: None,
        closure: Some(run_value),
        args: vec![],
        mode: CallMode::Call,
        interface: None,
    });
    entry.ret(&[]);
    let program = program(vec![closure, entry]);
    assert_eq!(flows(&program, &query(named("token"), "sink", &[])), 1);
}

#[test]
fn taint_written_through_a_pointer_parameter_reaches_the_caller() {
    // fill(p, v) { *p = v }; entry(token) { cell := alloc; fill(cell, token); sink(*cell) }
    let mut fill = Body::new("app.fill", &["p", "v"]);
    fill.push(StmtKind::Store {
        target: s(0),
        path: vec![Step::Deref],
        value: s(1),
    });
    fill.ret(&[]);
    let mut entry = Body::new("app.entry", &["token"]);
    let cell = entry.slot();
    entry.push(StmtKind::Alloc { dst: cell });
    entry.call("app.fill", &[s(cell), s(0)]);
    let loaded = entry.slot();
    entry.push(StmtKind::Load {
        dst: loaded,
        base: s(cell),
        step: Step::Deref,
    });
    entry.call("app.sink", &[s(loaded)]);
    entry.ret(&[]);
    let program = program(vec![fill, entry]);
    assert_eq!(flows(&program, &query(named("token"), "sink", &[])), 1);
}

#[test]
fn taint_from_a_source_inside_a_callee_returns_to_the_caller() {
    // read() { return source() }; entry() { sink(read()) }
    let mut read = Body::new("app.read", &[]);
    let value = read.call("app.source", &[]);
    read.ret(&[s(value)]);
    let mut entry = Body::new("app.entry", &[]);
    let got = entry.call("app.read", &[]);
    entry.call("app.sink", &[s(got)]);
    entry.ret(&[]);
    let program = program(vec![read, entry]);
    let sources = vec![SourceSpec::CallResult {
        callee: Matcher::name("source"),
        result: None,
    }];
    let output = run(&program, &query(sources, "sink", &[]));
    assert_eq!(output.flows.len(), 1);
    assert_eq!(
        program.function(output.flows[0].source_function).name,
        "app.read"
    );
    assert_eq!(
        program.function(output.flows[0].sink_function).name,
        "app.entry"
    );
}

#[test]
fn taint_through_a_global_reaches_its_readers() {
    let mut write = Body::new("app.write", &["token"]);
    let global = write.slot();
    write
        .function
        .globals
        .insert(global, "app.cache".to_string());
    write.push(StmtKind::Store {
        target: s(global),
        path: vec![Step::Deref],
        value: s(0),
    });
    write.ret(&[]);
    let mut read = Body::new("app.read", &[]);
    let global = read.slot();
    read.function
        .globals
        .insert(global, "app.cache".to_string());
    let value = read.slot();
    read.push(StmtKind::Load {
        dst: value,
        base: s(global),
        step: Step::Deref,
    });
    read.call("app.sink", &[s(value)]);
    read.ret(&[]);
    let program = program(vec![write, read]);
    assert_eq!(flows(&program, &query(named("token"), "sink", &[])), 1);
}

/// even(x) calls odd(x), odd(x) calls even(x) and the sink: a recursive cycle
/// whose summaries settle.
#[test]
fn a_recursive_cycle_settles_and_reports_once() {
    let mut even = Body::new("app.even", &["x"]);
    even.call("app.odd", &[s(0)]);
    even.ret(&[s(0)]);
    let mut odd = Body::new("app.odd", &["x"]);
    odd.call("app.even", &[s(0)]);
    odd.call("app.sink", &[s(0)]);
    odd.ret(&[s(0)]);
    let mut entry = Body::new("app.entry", &["token"]);
    entry.call("app.even", &[s(0)]);
    entry.ret(&[]);
    let program = program(vec![even, odd, entry]);
    let output = run(&program, &query(named("token"), "sink", &[]));
    assert_eq!(output.flows.len(), 1);
}

/// A chain where every summary is computed once: no callee changes after its
/// caller read it, so nothing is recomputed.
#[test]
fn an_unchanged_summary_does_not_wake_its_callers() {
    let mut bodies = Vec::new();
    for level in 0..5 {
        let mut body = Body::new(&format!("app.level{level}"), &["x"]);
        if level < 4 {
            body.call(&format!("app.level{}", level + 1), &[s(0)]);
        } else {
            body.call("app.sink", &[s(0)]);
        }
        body.ret(&[s(0)]);
        bodies.push(body);
    }
    let mut entry = Body::new("app.entry", &["token"]);
    entry.call("app.level0", &[s(0)]);
    entry.ret(&[]);
    bodies.push(entry);
    let program = program(bodies);
    let output = run(&program, &query(named("token"), "sink", &[]));
    assert_eq!(output.flows.len(), 1);
    assert_eq!(output.summaries_computed, 5);
    assert_eq!(output.summaries_recomputed, 0);
}

/// entry(token, ctx) { derived := library(ctx, token); sink(derived) }, where
/// `derived` is a context: a query that does not track contexts reports nothing.
#[test]
fn an_untracked_kind_of_value_carries_no_taint() {
    let mut entry = Body::new("app.entry", &["token", "ctx"]);
    let derived = entry.call("lib.WithValue", &[s(1), s(0)]);
    entry.call("app.sink", &[s(derived)]);
    entry.ret(&[]);
    entry.function.classes = vec![b'.'; entry.function.slots as usize];
    entry.function.classes[1] = b'c';
    entry.function.classes[derived as usize] = b'c';
    let program = program(vec![entry]);
    let tracked = query(named("token"), "sink", &[]);
    assert_eq!(flows(&program, &tracked), 1);
    let untracked = Query {
        untracked: crate::analysis_neutral::taint::ir::ValueKinds::CONTEXT,
        ..query(named("token"), "sink", &[])
    };
    assert_eq!(flows(&program, &untracked), 0);
    let numbers_only = Query {
        untracked: crate::analysis_neutral::taint::ir::ValueKinds::NUMBER,
        ..query(named("token"), "sink", &[])
    };
    assert_eq!(flows(&program, &numbers_only), 1);
}

#[test]
fn a_unit_budget_stop_is_an_unknown_on_the_flow_and_the_run() {
    let program = helper_program(false);
    let mut stopped = query(named("token"), "sink", &[]);
    stopped.unit_budget = 2;
    let output = run(&program, &stopped);
    assert!(output.unknowns.iter().any(|unknown| matches!(
        unknown,
        crate::analysis_neutral::taint::solver::Unknown::UnitBudget(_)
    )));
}

/// A body cut off by its package's step budget after it reached a sink still
/// reports that flow, with the budget stop among the flow's unknowns.
#[test]
fn a_flow_through_a_body_the_budget_cut_carries_the_budget_unknown() {
    let mut entry = Body::new("app.entry", &["token"]);
    entry.call("app.sink", &[s(0)]);
    for _ in 0..50 {
        let copy = entry.slot();
        entry.push(StmtKind::Copy {
            dst: copy,
            src: s(0),
        });
    }
    entry.ret(&[]);
    let program = program(vec![entry]);
    let mut cut = query(named("token"), "sink", &[]);
    cut.unit_budget = 3;
    let output = run(&program, &cut);
    assert_eq!(output.flows.len(), 1);
    let budget = |unknown: &crate::analysis_neutral::taint::solver::Unknown| {
        matches!(
            unknown,
            crate::analysis_neutral::taint::solver::Unknown::UnitBudget(unit) if unit == "example.com/app"
        )
    };
    assert!(output.flows[0].unknowns.iter().any(budget));
    assert!(output.unknowns.iter().any(budget));
    let whole = run(&program, &query(named("token"), "sink", &[]));
    assert_eq!(whole.flows.len(), 1);
    assert!(!whole.flows[0].unknowns.iter().any(budget));
}

/// even(x) { odd(x); return x }; odd(x) { sink(even(x)) }: odd first reads
/// even's summary while it is still being computed (no exits yet), so the flow
/// exists only once odd is recomputed after even finishes.
#[test]
fn a_summary_read_before_its_cycle_finished_is_recomputed() {
    let mut even = Body::new("app.even", &["x"]);
    even.call("app.odd", &[s(0)]);
    even.ret(&[s(0)]);
    let mut odd = Body::new("app.odd", &["x"]);
    let back = odd.call("app.even", &[s(0)]);
    odd.call("app.sink", &[s(back)]);
    odd.ret(&[]);
    let mut entry = Body::new("app.entry", &["token"]);
    entry.call("app.even", &[s(0)]);
    entry.ret(&[]);
    let program = program(vec![even, odd, entry]);
    let output = run(&program, &query(named("token"), "sink", &[]));
    assert_eq!(output.flows.len(), 1);
    assert_eq!(output.summaries_recomputed, 1);
}

/// The eager pass summarizes every parameter of every body; a carrying helper's
/// parameter reaches its result, a cleaning one's does not.
#[test]
fn precompute_summarizes_every_parameter() {
    let mut carry = Body::new("app.carry", &["value"]);
    carry.ret(&[s(0)]);
    let mut clean = Body::new("app.clean", &["value"]);
    clean.ret(&[Operand::Const(0)]);
    clean.function.consts.push("safe".to_string());
    let mut entry = Body::new("app.entry", &["token", "other"]);
    let carried = entry.call("app.carry", &[s(0)]);
    entry.ret(&[s(carried)]);
    let program = program(vec![carry, clean, entry]);
    let index = ProgramIndex::new(&program);
    let models = ExternalModels::default();
    let query = Query::default();
    let order = [0, 1, 2];
    let output = Solver::new(&program, &index, &models, &query).precompute(&order);
    assert_eq!(output.summaries, 4);
    let exits = |function: u32, param: u16| {
        output.exits[&(
            function,
            crate::analysis_neutral::taint::solver::Entry::Param(
                param,
                crate::analysis_neutral::taint::path::Path::EMPTY,
            ),
        )]
            .clone()
    };
    assert_eq!(exits(0, 0).len(), 1);
    assert!(exits(1, 0).is_empty());
    assert_eq!(exits(2, 0).len(), 1);
    assert!(exits(2, 1).is_empty());
}
