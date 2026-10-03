//! The taint solver: sources to sinks over function bodies, with summaries per
//! (body, entry fact) reused across callers.
//!
//! Within a body, facts close over the body's moves (SSA registers make this
//! flow-sensitive for values; memory is handled through the edges' aliasing
//! rules). At a call, a fact on an argument enters each candidate callee as a
//! fact on its parameter; the callee's summary for that entry says which facts
//! leave it (on its results, on what its pointer parameters point to, on its
//! closure bindings, on globals) and which sinks it reaches. Summaries are
//! computed on demand and memoized; a recursive cycle reads a partial summary
//! and is recomputed until no summary changes, and a recomputed summary equal
//! to the previous one does not wake its dependents. Taint that reaches a
//! source body's results or memory parameters returns to every caller.
//!
//! Facts carry no provenance while the solver runs, and a summary keeps only
//! its exits, the sinks it reaches and the summaries it entered. A reported
//! flow's path is recovered afterwards by re-running the bodies it passes with
//! provenance on: every summary they read is settled by then, so the re-run
//! reaches the same facts.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Instant;

use crate::analysis_neutral::taint::index::{FxHashMap, FxHashSet, ProgramIndex};
use crate::analysis_neutral::taint::ir::{
    Algorithm, Callee, FlowFunction, FlowProgram, FnId, NO_BODY, Operand, Pos, Slot, Step,
    StmtKind, ValueKinds,
};
use crate::analysis_neutral::taint::path::{Fact, Path};

/// Matches a function by name: its qualified name, or its last name (`Exec`),
/// or a `Type.Method` suffix.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct NamePattern(pub(crate) String);

impl NamePattern {
    pub(crate) fn matches(&self, qualified: &str) -> bool {
        let pattern = self.0.as_str();
        if pattern.is_empty() {
            return false;
        }
        if qualified == pattern {
            return true;
        }
        let bare = qualified
            .trim_start_matches('(')
            .replace(")", "")
            .trim_start_matches('*')
            .to_string();
        if bare == pattern || bare.ends_with(&format!("/{pattern}")) {
            return true;
        }
        // `Type.Method` or `Method`/`Func`: compare the trailing segments.
        let parts = bare.rsplit('/').next().unwrap_or(&bare);
        let segments = parts.split('.').collect::<Vec<_>>();
        let wanted = pattern.split('.').collect::<Vec<_>>();
        segments.len() >= wanted.len() && segments[segments.len() - wanted.len()..] == wanted[..]
    }
}

/// Matches a called function: by a name pattern, or by a model target.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Matcher {
    Name(NamePattern),
    Target(crate::analysis_neutral::taint::models::Target),
}

impl Matcher {
    pub(crate) fn name(pattern: &str) -> Matcher {
        Matcher::Name(NamePattern(pattern.to_string()))
    }

    pub(crate) fn matches(&self, qualified: &str) -> bool {
        match self {
            Matcher::Name(pattern) => pattern.matches(qualified),
            Matcher::Target(target) => target.matches(qualified),
        }
    }
}

/// Which arguments of a call are sinks (the receiver is never counted).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Arguments {
    All,
    Only(Vec<u16>),
    From(u16),
}

impl Arguments {
    pub(crate) fn contains(&self, position: u16) -> bool {
        match self {
            Arguments::All => true,
            Arguments::Only(positions) => positions.contains(&position),
            Arguments::From(first) => position >= *first,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum SourceSpec {
    /// The result of a call to a matching callee (`result` of several).
    CallResult {
        callee: Matcher,
        result: Option<u16>,
    },
    /// What a pointer argument of a matching call points to after the call.
    CallArgumentPointee { callee: Matcher, argument: u16 },
    /// A parameter of a matching function, or of any function, by position,
    /// type or name.
    Parameter {
        function: Option<Matcher>,
        index: Option<u16>,
        type_name: Option<String>,
    },
    /// Values named like one of `names` (case-insensitive substring): parameters,
    /// address-taken locals, globals and struct-field loads.
    Named { names: Vec<String> },
    /// A parameter of a closure passed as `argument` to a matching call (a
    /// transaction callback's handle).
    CallbackParameter {
        callee: Matcher,
        argument: u16,
        parameter: u16,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum SinkSpec {
    /// An argument of a call to a matching callee: declared parameter positions
    /// (receiver excluded), and the receiver when `receiver` is set.
    CallArgument {
        callee: Matcher,
        arguments: Arguments,
        receiver: bool,
    },
    /// A value the body holding the source returns.
    Return,
}

/// What an external (bodyless) callee does with taint, when a model says.
#[derive(Clone, Debug, Default)]
pub(crate) struct ExternalModels {
    /// Callees whose results never carry their arguments' taint.
    pub(crate) sanitizers: Vec<Matcher>,
    /// Callees through which taint does not flow at all (comparisons, tests):
    /// neither the result nor anything else gets it.
    pub(crate) opaque: Vec<Matcher>,
    /// Callees that move taint from an argument (`None`: the receiver) to what
    /// another argument points to (`json.Unmarshal(data, &v)`).
    pub(crate) into_argument: Vec<(Matcher, Option<u16>, u16)>,
}

#[derive(Clone, Debug)]
pub(crate) struct Query {
    pub(crate) sources: Vec<SourceSpec>,
    pub(crate) sinks: Vec<SinkSpec>,
    /// Callees whose result is clean whatever goes in (query barriers).
    pub(crate) sanitizers: Vec<Matcher>,
    /// Field steps an access path keeps.
    pub(crate) k: usize,
    /// The most calls a flow returns up through from its source.
    pub(crate) max_return_depth: usize,
    /// The most nested callee summaries one summary may wait on.
    pub(crate) max_call_depth: usize,
    /// Facts a unit (package) may process before its bodies stop.
    pub(crate) unit_budget: u64,
    pub(crate) deadline: Option<Instant>,
    /// Whether a closure passed to a body-less callee is treated as called by it.
    pub(crate) callbacks: bool,
    /// Kinds of values that cannot carry what the query tracks.
    pub(crate) untracked: ValueKinds,
}

impl Default for Query {
    fn default() -> Self {
        Query {
            sources: Vec::new(),
            sinks: Vec::new(),
            sanitizers: Vec::new(),
            k: 2,
            max_return_depth: 8,
            max_call_depth: 48,
            unit_budget: 2_000_000,
            deadline: None,
            callbacks: true,
            untracked: ValueKinds::NONE,
        }
    }
}

/// How sure a step of a flow is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Precision {
    /// Through statically known calls and modelled library functions.
    Exact,
    /// Through a call variable-type analysis resolved.
    SetupAware,
    /// Through a class-hierarchy candidate, a global, or a library function
    /// without a model (its arguments assumed to reach its result).
    Conservative,
    /// Through a closure assumed called by a library function.
    Heuristic,
}

/// Why part of the program was not fully analysed.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Unknown {
    /// A unit (package) ran out of its step budget.
    UnitBudget(String),
    /// The run deadline passed.
    Deadline,
    /// Nested calls deeper than the limit were not entered, or a recursive
    /// cycle did not settle within its recomputation limit.
    CallDepth(String),
    /// A dynamic call had no known candidate.
    UnresolvedCall { function: String, line: u32 },
}

/// Where a fact leaves a body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Exit {
    Return(u16, Path),
    Param(u16, Path),
    Free(u16, Path),
    Global(u32, Path),
}

/// Where a fact enters a body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Entry {
    Param(u16, Path),
    Free(u16, Path),
}

type SummaryKey = (FnId, Entry);

/// The most times one summary is recomputed in a query.
const MAX_RECOMPUTES: u32 = 64;

/// Field steps a summary's entry path keeps.
const MAX_ENTRY_FIELDS: usize = 1;

/// The most origins a path recovery follows back from one fact.
const MAX_TRACE: usize = 10_000;

/// How a fact came to hold, in a frame that records provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    /// The query's source with this index seeded it.
    Source(usize),
    /// It entered the body (summary frames).
    Entry,
    /// A move inside the body.
    Move { from: Fact, stmt: u32 },
    /// A callee's summary returned it.
    CallReturn { from: Fact, stmt: u32 },
    /// A body-less callee passed it through, by a model or by default.
    External {
        from: Fact,
        stmt: u32,
        modelled: bool,
    },
    /// Another top frame's body returned it at this call.
    ReturnedFrom { frame: usize, exit: Exit, stmt: u32 },
    /// A global another top frame wrote: that frame and the fact it wrote from.
    Global { frame: usize, fact: Fact },
}

/// Where a sink was reached: the query's sink, the call (or return) and the
/// fact that reached it.
#[derive(Clone, Copy, Debug)]
struct Hit {
    sink: usize,
    stmt: u32,
    fact: Fact,
    /// The call argument the fact reached (a method's receiver is 0), or none
    /// for a return.
    argument: Option<u16>,
}

/// A fact that entered a callee summary at a call, kept when the callee may
/// reach a sink.
#[derive(Clone, Copy, Debug)]
struct Nested {
    stmt: u32,
    fact: Fact,
    callee: FnId,
    entry: Entry,
    /// Entered because a library function was assumed to call the closure.
    heuristic: bool,
}

/// A body analysed from some facts: a top frame (a source body, or a caller
/// taint returned to) or a summary's frame.
#[derive(Clone, Debug, Default)]
struct Frame {
    function: FnId,
    facts: FxHashSet<Fact>,
    /// Each fact's origin and the batch of starting facts it was first derived
    /// from, in a frame re-run to recover paths.
    origins: Option<FxHashMap<Fact, (Origin, u32)>>,
    /// A top frame's starting facts with their origins and the return depth
    /// they arrived at.
    start: Vec<(Fact, Origin, u32)>,
    exits: BTreeSet<Exit>,
    /// A global a callee wrote, with the fact passed to that callee and the call.
    global_writes: Vec<(Exit, Fact, u32)>,
    hits: Vec<Hit>,
    nested: Vec<Nested>,
    unknowns: BTreeSet<Unknown>,
}

struct Summary {
    frame: usize,
    exits: BTreeSet<Exit>,
    done: bool,
    /// Whether a summary read it before its first computation finished (a
    /// recursive cycle), so that reader saw only part of it.
    read_partial: bool,
    dependents: BTreeSet<SummaryKey>,
}

/// What the query's and the models' matchers say about a call, by the names
/// it is known by.
#[derive(Clone, Debug, Default)]
struct CallMatch {
    sinks: Vec<usize>,
    sources: Vec<usize>,
    /// A sanitizer or an opaque function: nothing flows through the call.
    blocked: bool,
    /// Modelled moves into an argument's pointee: (from argument or receiver, to).
    propagators: Vec<(Option<u16>, u16)>,
}

impl CallMatch {
    fn of(name: &str, query: &Query, models: &ExternalModels) -> CallMatch {
        CallMatch {
            sinks: query
                .sinks
                .iter()
                .enumerate()
                .filter(|(_, sink)| match sink {
                    SinkSpec::CallArgument { callee, .. } => callee.matches(name),
                    SinkSpec::Return => false,
                })
                .map(|(index, _)| index)
                .collect(),
            sources: query
                .sources
                .iter()
                .enumerate()
                .filter(|(_, source)| match source {
                    SourceSpec::CallResult { callee, .. }
                    | SourceSpec::CallArgumentPointee { callee, .. }
                    | SourceSpec::CallbackParameter { callee, .. } => callee.matches(name),
                    SourceSpec::Parameter { .. } | SourceSpec::Named { .. } => false,
                })
                .map(|(index, _)| index)
                .collect(),
            blocked: query
                .sanitizers
                .iter()
                .chain(models.sanitizers.iter())
                .chain(models.opaque.iter())
                .any(|matcher| matcher.matches(name)),
            propagators: models
                .into_argument
                .iter()
                .filter(|(matcher, _, _)| matcher.matches(name))
                .map(|(_, from, to)| (*from, *to))
                .collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.sinks.is_empty()
            && self.sources.is_empty()
            && !self.blocked
            && self.propagators.is_empty()
    }

    fn merge(&mut self, other: &CallMatch) {
        self.sinks.extend(other.sinks.iter().copied());
        self.sources.extend(other.sources.iter().copied());
        self.blocked |= other.blocked;
        self.propagators.extend(other.propagators.iter().copied());
    }

    fn finish(&mut self) {
        self.sinks.sort_unstable();
        self.sinks.dedup();
        self.sources.sort_unstable();
        self.sources.dedup();
        self.propagators.sort_unstable();
        self.propagators.dedup();
    }
}

/// One flow from a source to a sink.
#[derive(Clone, Debug)]
pub(crate) struct Flow {
    pub(crate) source: SourceSpec,
    pub(crate) source_function: FnId,
    pub(crate) source_pos: Pos,
    pub(crate) sink: SinkSpec,
    pub(crate) sink_function: FnId,
    pub(crate) sink_pos: Pos,
    /// The call argument the taint reaches the sink as (a method's receiver is
    /// 0), or none when the sink is a return.
    pub(crate) sink_argument: Option<u16>,
    /// The statements the taint passes, source to sink: (body, statement).
    pub(crate) steps: Vec<(FnId, Pos)>,
    pub(crate) precision: Precision,
    pub(crate) unknowns: BTreeSet<Unknown>,
}

/// The eager pass's result: every summary it computed, by (body, entry).
#[derive(Debug, Default)]
pub(crate) struct Precomputed {
    pub(crate) exits: BTreeMap<(FnId, Entry), Vec<Exit>>,
    pub(crate) summaries: usize,
    pub(crate) recomputed: usize,
    pub(crate) steps: u64,
    pub(crate) unknowns: BTreeSet<Unknown>,
}

#[derive(Debug, Default)]
pub(crate) struct QueryOutput {
    pub(crate) flows: Vec<Flow>,
    pub(crate) unknowns: BTreeSet<Unknown>,
    /// How many summaries were computed, and how many recomputed after a
    /// callee in their cycle changed.
    pub(crate) summaries_computed: usize,
    pub(crate) summaries_recomputed: usize,
    /// Facts processed.
    pub(crate) steps: u64,
}

/// A fact followed back to where it began: its source (none when it entered a
/// summary's body), the body it began in, the statements it passed (first
/// first), the frames it passed and the least precise step.
#[derive(Clone, Debug)]
struct Walk {
    source: Option<usize>,
    function: FnId,
    steps: Vec<(FnId, Pos)>,
    frames: Vec<usize>,
    precision: Precision,
}

/// One query over a program.
pub(crate) struct Solver<'p> {
    program: &'p FlowProgram,
    index: &'p ProgramIndex,
    query: &'p Query,
    /// What each call statement matches, for the calls that match something.
    matches: FxHashMap<(FnId, u32), CallMatch>,
    frames: Vec<Frame>,
    summaries: FxHashMap<SummaryKey, Summary>,
    unit_steps: Vec<u64>,
    exhausted: Vec<bool>,
    deadline_passed: bool,
    steps: u64,
    stack: Vec<SummaryKey>,
    recompute: VecDeque<SummaryKey>,
    queued: FxHashSet<SummaryKey>,
    recomputes: FxHashMap<SummaryKey, u32>,
    summaries_computed: usize,
    summaries_recomputed: usize,
    /// Bodies a sink is reachable from (through calls, and through closures
    /// they create); a frame records the summaries it enters only for these.
    sink_reaching: Vec<bool>,
    /// Whether frames being computed record their facts' origins.
    tracing: bool,
    traced_tops: FxHashMap<usize, usize>,
    traced_summaries: FxHashMap<SummaryKey, usize>,
    /// The batch of starting facts a traced top frame is closing.
    batch: u32,
}

impl<'p> Solver<'p> {
    pub(crate) fn new(
        program: &'p FlowProgram,
        index: &'p ProgramIndex,
        models: &'p ExternalModels,
        query: &'p Query,
    ) -> Solver<'p> {
        let mut solver = Solver {
            program,
            index,
            query,
            matches: FxHashMap::default(),
            frames: Vec::new(),
            summaries: FxHashMap::default(),
            unit_steps: vec![0; index.units.len()],
            exhausted: vec![false; index.units.len()],
            deadline_passed: false,
            steps: 0,
            stack: Vec::new(),
            recompute: VecDeque::new(),
            queued: FxHashSet::default(),
            recomputes: FxHashMap::default(),
            summaries_computed: 0,
            summaries_recomputed: 0,
            sink_reaching: Vec::new(),
            tracing: false,
            traced_tops: FxHashMap::default(),
            traced_summaries: FxHashMap::default(),
            batch: 0,
        };
        solver.matches = solver.match_calls(models);
        solver.sink_reaching = solver.bodies_reaching_sinks();
        solver
    }

    fn callee_name(&self, callee: &'p Callee) -> &'p str {
        match callee {
            Callee::Body(id) => &self.program.function(*id).name,
            Callee::External(name) => name,
        }
    }

    /// Matches every call statement once per distinct callee name: the names a
    /// call is known by are each candidate callee's and, for an interface call,
    /// the interface method's.
    fn match_calls(&self, models: &ExternalModels) -> FxHashMap<(FnId, u32), CallMatch> {
        let mut by_name: FxHashMap<&'p str, usize> = FxHashMap::default();
        let mut name_matches: Vec<CallMatch> = Vec::new();
        let mut matches = FxHashMap::default();
        let program = self.program;
        for (id, function) in program.functions.iter().enumerate() {
            for (stmt_index, stmt) in function.stmts.iter().enumerate() {
                let StmtKind::Call {
                    callees,
                    interface,
                    builtin: None,
                    ..
                } = &stmt.kind
                else {
                    continue;
                };
                let mut call = CallMatch::default();
                let names = callees
                    .iter()
                    .map(|callee| self.callee_name(callee))
                    .chain(interface.as_deref());
                for name in names {
                    let entry = *by_name.entry(name).or_insert_with(|| {
                        name_matches.push(CallMatch::of(name, self.query, models));
                        name_matches.len() - 1
                    });
                    call.merge(&name_matches[entry]);
                }
                if !call.is_empty() {
                    call.finish();
                    matches.insert((id as FnId, stmt_index as u32), call);
                }
            }
        }
        matches
    }

    /// Bodies with a sink call, and every body that may call one of them or
    /// create a closure of one.
    fn bodies_reaching_sinks(&self) -> Vec<bool> {
        let count = self.program.functions.len();
        let mut reaching = vec![false; count];
        let mut work = Vec::new();
        if self.query.sinks.contains(&SinkSpec::Return) {
            reaching = vec![true; count];
            return reaching;
        }
        for ((function, _), call) in &self.matches {
            if !call.sinks.is_empty() && !reaching[*function as usize] {
                reaching[*function as usize] = true;
                work.push(*function);
            }
        }
        while let Some(id) = work.pop() {
            let callers = self.index.callers[id as usize]
                .iter()
                .map(|(caller, _)| *caller);
            let creators = self.index.creators[id as usize].iter().copied();
            for caller in callers.chain(creators) {
                if !reaching[caller as usize] {
                    reaching[caller as usize] = true;
                    work.push(caller);
                }
            }
        }
        reaching
    }

    /// Whether a call's candidates name a method, and whether some candidate has
    /// no body (or no candidate is known).
    fn call_shape(&self, callees: &'p [Callee], interface: &'p Option<String>) -> (bool, bool) {
        let method = callees
            .iter()
            .any(|callee| is_method_name(self.callee_name(callee)))
            || interface.as_deref().is_some_and(is_method_name);
        let external = callees.is_empty()
            || callees
                .iter()
                .any(|callee| matches!(callee, Callee::External(_)));
        (method, external)
    }

    /// Runs the query: seeds the sources, closes every source body, returns taint
    /// up to callers, and reports the flows that reach sinks.
    pub(crate) fn run(mut self) -> QueryOutput {
        let mut top_frames: BTreeMap<FnId, usize> = BTreeMap::new();
        let mut pending: VecDeque<(usize, Vec<Fact>, usize)> = VecDeque::new();
        for (function, facts) in self.seeds() {
            let frame = self.new_frame(function);
            top_frames.insert(function, frame);
            let mut fresh = Vec::new();
            for (fact, source) in facts {
                if self.frames[frame].facts.insert(fact) {
                    self.frames[frame]
                        .start
                        .push((fact, Origin::Source(source), 0));
                    fresh.push(fact);
                }
            }
            pending.push_back((frame, fresh, 0));
        }
        let mut returned: BTreeSet<(usize, Exit)> = BTreeSet::new();
        while let Some((frame, facts, depth)) = pending.pop_front() {
            self.close(frame, facts, true);
            self.settle();
            if depth >= self.query.max_return_depth {
                continue;
            }
            // Taint leaving a top body returns to every call of it, and taint in
            // a global reaches every body that uses the global.
            let exits = self.frames[frame].exits.iter().copied().collect::<Vec<_>>();
            let function = self.frames[frame].function;
            for exit in exits {
                if !returned.insert((frame, exit)) {
                    continue;
                }
                let mut targets: BTreeMap<FnId, Vec<(Fact, Origin)>> = BTreeMap::new();
                if let Exit::Global(global, path) = exit {
                    let Some(writer) = self.fact_for_exit(frame, exit) else {
                        continue;
                    };
                    for (user, slot) in &self.index.global_users[global as usize] {
                        targets.entry(*user).or_default().push((
                            Fact::new(*slot, path),
                            Origin::Global {
                                frame,
                                fact: writer,
                            },
                        ));
                    }
                } else {
                    for (caller, stmt) in &self.index.callers[function as usize] {
                        for fact in self.exit_into_caller(*caller, *stmt, function, exit) {
                            targets.entry(*caller).or_default().push((
                                fact,
                                Origin::ReturnedFrom {
                                    frame,
                                    exit,
                                    stmt: *stmt,
                                },
                            ));
                        }
                    }
                }
                for (target, facts) in targets {
                    let target_frame = match top_frames.get(&target) {
                        Some(existing) => *existing,
                        None => {
                            let created = self.new_frame(target);
                            top_frames.insert(target, created);
                            created
                        }
                    };
                    let mut fresh = Vec::new();
                    for (fact, origin) in facts {
                        if self.frames[target_frame].facts.insert(fact) {
                            self.frames[target_frame]
                                .start
                                .push((fact, origin, depth as u32 + 1));
                            fresh.push(fact);
                        }
                    }
                    if !fresh.is_empty() {
                        pending.push_back((target_frame, fresh, depth + 1));
                    }
                }
            }
        }
        let mut output = QueryOutput {
            summaries_computed: self.summaries_computed,
            summaries_recomputed: self.summaries_recomputed,
            steps: self.steps,
            ..QueryOutput::default()
        };
        for frame in &self.frames {
            output.unknowns.extend(frame.unknowns.iter().cloned());
        }
        for (unit, exhausted) in self.exhausted.iter().enumerate() {
            if *exhausted {
                output
                    .unknowns
                    .insert(Unknown::UnitBudget(self.index.units[unit].clone()));
            }
        }
        if self.deadline_passed {
            output.unknowns.insert(Unknown::Deadline);
        }
        self.report(&top_frames, &mut output);
        output
            .flows
            .sort_by(|left, right| flow_order(left).cmp(&flow_order(right)));
        output
    }

    /// Computes, in `order`, every body's summary for each of its parameters as
    /// a whole tainted value: the query-independent part of the analysis (which
    /// results, pointer parameters, closure bindings and globals each parameter
    /// reaches). `order` is bottom-up, so a body's callees are summarized before
    /// it and a body waits on a summary only inside a cycle.
    pub(crate) fn precompute(mut self, order: &[FnId]) -> Precomputed {
        for function in order {
            if self.deadline_passed {
                break;
            }
            let params = self.program.function(*function).params.len();
            for position in 0..params {
                let entry = Entry::Param(position as u16, Path::EMPTY);
                if !self.summaries.contains_key(&(*function, entry)) {
                    let _ = self.summary_exits(*function, entry, None);
                }
            }
        }
        let mut precomputed = Precomputed::default();
        for frame in &self.frames {
            precomputed.unknowns.extend(frame.unknowns.iter().cloned());
        }
        for (unit, exhausted) in self.exhausted.iter().enumerate() {
            if *exhausted {
                precomputed
                    .unknowns
                    .insert(Unknown::UnitBudget(self.index.units[unit].clone()));
            }
        }
        if self.deadline_passed {
            precomputed.unknowns.insert(Unknown::Deadline);
        }
        precomputed.summaries = self.summaries.len();
        precomputed.recomputed = self.summaries_recomputed;
        precomputed.steps = self.steps;
        precomputed.exits = self
            .summaries
            .iter()
            .map(|(key, summary)| (*key, summary.exits.iter().copied().collect()))
            .collect();
        precomputed
    }

    fn new_frame(&mut self, function: FnId) -> usize {
        self.frames.push(Frame {
            function,
            origins: self.tracing.then(FxHashMap::default),
            ..Frame::default()
        });
        self.frames.len() - 1
    }

    /// The facts a caller gets at one call of `callee` when taint leaves the
    /// callee at `exit`.
    fn exit_into_caller(&self, caller: FnId, stmt: u32, callee: FnId, exit: Exit) -> Vec<Fact> {
        let function = self.program.function(caller);
        let StmtKind::Call {
            dst, args, closure, ..
        } = &function.stmts[stmt as usize].kind
        else {
            return Vec::new();
        };
        let results = self.program.function(callee).results;
        let k = self.query.k;
        match exit {
            Exit::Return(position, path) => dst
                .map(|dst| {
                    if results > 1 {
                        Fact::new(dst, Path::join(&[Step::Result(position)], path.steps(), k))
                    } else {
                        Fact::new(dst, path)
                    }
                })
                .into_iter()
                .collect(),
            Exit::Param(position, path) => args
                .get(position as usize)
                .and_then(|arg| arg.slot())
                .map(|slot| Fact::new(slot, path))
                .into_iter()
                .collect(),
            Exit::Free(position, path) => closure
                .map(|closure| {
                    Fact::new(
                        closure,
                        Path::join(&[Step::Capture(callee, position)], path.steps(), k),
                    )
                })
                .into_iter()
                .collect(),
            Exit::Global(..) => Vec::new(),
        }
    }

    /// The sources' facts, by the body they hold in, each with its source (the
    /// first source, when several seed the same fact).
    fn seeds(&self) -> BTreeMap<FnId, Vec<(Fact, usize)>> {
        let mut seeds: BTreeMap<FnId, Vec<(Fact, usize)>> = BTreeMap::new();
        for ((function, stmt), call) in &self.matches {
            for source in &call.sources {
                for (target, fact) in self.call_source_facts(*function, *stmt, *source) {
                    seeds.entry(target).or_default().push((fact, *source));
                }
            }
        }
        for (source_index, source) in self.query.sources.iter().enumerate() {
            if !matches!(
                source,
                SourceSpec::Parameter { .. } | SourceSpec::Named { .. }
            ) {
                continue;
            }
            for (id, function) in self.program.functions.iter().enumerate() {
                for fact in self.body_source_facts(id as FnId, function, source) {
                    seeds
                        .entry(id as FnId)
                        .or_default()
                        .push((fact, source_index));
                }
            }
        }
        for facts in seeds.values_mut() {
            facts.sort_unstable();
            facts.dedup_by_key(|(fact, _)| *fact);
        }
        seeds
    }

    /// The facts a call-matched source seeds at one call.
    fn call_source_facts(&self, function_id: FnId, stmt: u32, source: usize) -> Vec<(FnId, Fact)> {
        let k = self.query.k;
        let function = self.program.function(function_id);
        let StmtKind::Call {
            dst,
            args,
            callees,
            interface,
            ..
        } = &function.stmts[stmt as usize].kind
        else {
            return Vec::new();
        };
        let (method, _) = self.call_shape(callees, interface);
        let mut out = Vec::new();
        match &self.query.sources[source] {
            SourceSpec::CallResult { result, .. } => {
                if let Some(dst) = dst {
                    let path = match result {
                        Some(position) => Path::limited([Step::Result(*position)], k),
                        None => Path::EMPTY,
                    };
                    out.push((function_id, Fact::new(*dst, path)));
                }
            }
            SourceSpec::CallArgumentPointee { argument, .. } => {
                let position = *argument as usize + usize::from(method);
                if let Some(Operand::Slot(slot)) = args.get(position) {
                    out.push((
                        function_id,
                        Fact::new(*slot, Path::limited([Step::Deref], k)),
                    ));
                }
            }
            SourceSpec::CallbackParameter {
                argument,
                parameter,
                ..
            } => {
                let position = *argument as usize + usize::from(method);
                let body = &self.index.bodies[function_id as usize];
                if let Some(Operand::Slot(slot)) = args.get(position)
                    && let Some(closure_stmt) = body.closure_of[*slot as usize]
                    && let StmtKind::Closure {
                        function: Some(Callee::Body(target)),
                        ..
                    } = &function.stmts[closure_stmt as usize].kind
                    && let Some(param) = self
                        .program
                        .function(*target)
                        .params
                        .get(*parameter as usize)
                {
                    out.push((*target, Fact::whole(*param)));
                }
            }
            SourceSpec::Parameter { .. } | SourceSpec::Named { .. } => {}
        }
        out
    }

    /// The facts a parameter or named source seeds in one body.
    fn body_source_facts(
        &self,
        id: FnId,
        function: &FlowFunction,
        source: &SourceSpec,
    ) -> Vec<Fact> {
        let k = self.query.k;
        let mut out = Vec::new();
        match source {
            SourceSpec::Parameter {
                function: pattern,
                index,
                type_name,
            } => {
                if pattern
                    .as_ref()
                    .is_none_or(|pattern| pattern.matches(&function.name))
                {
                    let receiver = usize::from(function.is_method());
                    for (position, slot) in function.params.iter().enumerate() {
                        if position < receiver && index.is_some() {
                            continue;
                        }
                        let declared = position - receiver.min(position);
                        if index.is_some_and(|index| index as usize != declared) {
                            continue;
                        }
                        if type_name.as_ref().is_some_and(|wanted| {
                            function.param_types.get(position) != Some(wanted)
                        }) {
                            continue;
                        }
                        out.push(Fact::whole(*slot));
                    }
                }
            }
            SourceSpec::Named { names } => {
                let matches = |name: &str| {
                    let lower = name.to_ascii_lowercase();
                    names.iter().any(|wanted| {
                        !wanted.is_empty() && lower.contains(&wanted.to_ascii_lowercase())
                    })
                };
                // A free variable is named after the captured variable, whose
                // taint the closure's bindings already carry in.
                for (slot, name) in &function.names {
                    if function.free.contains(slot) || !matches(name) {
                        continue;
                    }
                    let path = if function.params.contains(slot) {
                        Path::EMPTY
                    } else {
                        Path::limited([Step::Deref], k)
                    };
                    out.push(Fact::new(*slot, path));
                }
                for (slot, name) in &function.globals {
                    let short = name.rsplit('.').next().unwrap_or(name);
                    if matches(short) {
                        out.push(Fact::new(*slot, Path::limited([Step::Deref], k)));
                    }
                }
                let body = &self.index.bodies[id as usize];
                for (sym, loads) in &body.field_loads {
                    if matches(self.program.symbol_name(*sym)) {
                        out.extend(loads.iter().map(|dst| Fact::whole(*dst)));
                    }
                }
                // A field read through its address: what the address points to.
                for (sym, addrs) in &body.field_addrs {
                    if matches(self.program.symbol_name(*sym)) {
                        out.extend(
                            addrs
                                .iter()
                                .map(|dst| Fact::new(*dst, Path::limited([Step::Deref], k))),
                        );
                    }
                }
            }
            SourceSpec::CallResult { .. }
            | SourceSpec::CallArgumentPointee { .. }
            | SourceSpec::CallbackParameter { .. } => {}
        }
        out
    }

    /// Counts one step against the body's unit and the run deadline; true when
    /// either has run out.
    fn out_of_budget(&mut self, function: FnId) -> bool {
        if self.deadline_passed {
            return true;
        }
        let unit = self.index.unit_of[function as usize] as usize;
        if self.exhausted[unit] {
            return true;
        }
        self.unit_steps[unit] += 1;
        if self.unit_steps[unit] > self.query.unit_budget {
            self.exhausted[unit] = true;
            return true;
        }
        self.steps += 1;
        if self.steps.is_multiple_of(4096)
            && let Some(deadline) = self.query.deadline
            && Instant::now() >= deadline
        {
            self.deadline_passed = true;
            return true;
        }
        false
    }

    /// Whether a slot of a body holds a kind of value the query does not track.
    fn untracked(&self, function: FnId, slot: Slot) -> bool {
        let classes = &self.program.function(function).classes;
        classes
            .get(slot as usize)
            .is_some_and(|class| self.query.untracked.contains_class(*class))
    }

    /// Adds a fact to a frame unless it or a shorter path that covers it is
    /// there, or it is a whole value of an untracked kind; returns whether it was
    /// added.
    fn add(&mut self, frame: usize, fact: Fact, origin: Origin) -> bool {
        if fact.path.is_empty() && self.untracked(self.frames[frame].function, fact.slot) {
            return false;
        }
        let frame = &mut self.frames[frame];
        if (0..=fact.path.len()).any(|len| {
            frame
                .facts
                .contains(&Fact::new(fact.slot, fact.path.prefix(len)))
        }) {
            return false;
        }
        frame.facts.insert(fact);
        if let Some(origins) = frame.origins.as_mut() {
            origins.insert(fact, (origin, self.batch));
        }
        true
    }

    /// Closes a frame over new facts: every fact the body's moves, calls and
    /// returns derive from them. A traced re-run is not counted against the
    /// budgets: it repeats work the budgets already allowed.
    fn close(&mut self, frame: usize, start: Vec<Fact>, top: bool) {
        let function_id = self.frames[frame].function;
        let index = self.index;
        let query = self.query;
        let body = &index.bodies[function_id as usize];
        let k = query.k;
        let return_sink = if top {
            query
                .sinks
                .iter()
                .position(|sink| *sink == SinkSpec::Return)
        } else {
            None
        };
        let mut work: VecDeque<Fact> = start.into();
        let mut derived: Vec<(Fact, Origin)> = Vec::new();
        while let Some(fact) = work.pop_front() {
            if !self.tracing && self.out_of_budget(function_id) {
                let unknown = if self.deadline_passed {
                    Unknown::Deadline
                } else {
                    let unit = index.unit_of[function_id as usize] as usize;
                    Unknown::UnitBudget(index.units[unit].clone())
                };
                self.frames[frame].unknowns.insert(unknown);
                return;
            }
            let slot = fact.slot as usize;
            // Moves out of the fact's slot.
            for edge_id in &body.by_src[slot] {
                let edge = &body.edges[*edge_id as usize];
                if let Some(path) =
                    forward(&fact.path, edge.src_path.steps(), edge.dst_path.steps(), k)
                {
                    derived.push((
                        Fact::new(edge.dst, path),
                        Origin::Move {
                            from: fact,
                            stmt: edge.stmt,
                        },
                    ));
                }
            }
            // Moves into the fact's slot: memory below the destination is shared
            // with the source; an alias is the same place both ways.
            for edge_id in &body.by_dst[slot] {
                let edge = &body.edges[*edge_id as usize];
                let path = if edge.alias {
                    forward(&fact.path, edge.dst_path.steps(), edge.src_path.steps(), k)
                } else {
                    backward(&fact.path, edge.dst_path.steps(), edge.src_path.steps(), k)
                };
                if let Some(path) = path {
                    derived.push((
                        Fact::new(edge.src, path),
                        Origin::Move {
                            from: fact,
                            stmt: edge.stmt,
                        },
                    ));
                }
            }
            // Leaving the body.
            for (stmt, position) in &body.returns[slot] {
                self.frames[frame]
                    .exits
                    .insert(Exit::Return(*position, fact.path));
                if let Some(sink) = return_sink {
                    self.frames[frame].hits.push(Hit {
                        sink,
                        stmt: *stmt,
                        fact,
                        argument: None,
                    });
                }
            }
            if fact.path.has_memory_step() {
                if let Some(position) = body.param_of[slot] {
                    self.frames[frame]
                        .exits
                        .insert(Exit::Param(position, fact.path));
                }
                if let Some(position) = body.free_of[slot] {
                    self.frames[frame]
                        .exits
                        .insert(Exit::Free(position, fact.path));
                }
            }
            if let Some(global) = body.global_of[slot]
                && fact.path.steps().first() == Some(&Step::Deref)
            {
                self.frames[frame]
                    .exits
                    .insert(Exit::Global(global, fact.path));
            }
            // Calls.
            for (stmt, position) in &body.arg_uses[slot] {
                self.call(
                    frame,
                    function_id,
                    *stmt,
                    Some(*position),
                    fact,
                    &mut derived,
                );
            }
            for stmt in &body.closure_uses[slot] {
                self.call(frame, function_id, *stmt, None, fact, &mut derived);
            }
            for (fact, origin) in derived.drain(..) {
                if self.add(frame, fact, origin) {
                    work.push_back(fact);
                }
            }
        }
    }

    /// A fact reaches call `stmt` as argument `position`, or as the called closure
    /// value when `position` is `None`.
    fn call(
        &mut self,
        frame: usize,
        function_id: FnId,
        stmt: u32,
        position: Option<u16>,
        fact: Fact,
        derived: &mut Vec<(Fact, Origin)>,
    ) {
        let program = self.program;
        let query = self.query;
        let function = program.function(function_id);
        let StmtKind::Call {
            dst,
            callees,
            algorithm,
            builtin,
            closure,
            args,
            interface,
            ..
        } = &function.stmts[stmt as usize].kind
        else {
            return;
        };
        let k = query.k;
        if let Some(builtin) = builtin {
            if let Some(dst) = dst
                && position.is_some()
                && matches!(builtin.as_str(), "append" | "min" | "max")
            {
                derived.push((
                    Fact::new(*dst, fact.path),
                    Origin::Move { from: fact, stmt },
                ));
            }
            if builtin == "copy"
                && position == Some(1)
                && let Some(Operand::Slot(target)) = args.first()
            {
                let path = forward(&fact.path, &[Step::Elem], &[Step::Elem], k)
                    .unwrap_or_else(|| Path::limited([Step::Elem], k));
                derived.push((Fact::new(*target, path), Origin::Move { from: fact, stmt }));
            }
            return;
        }
        let (method, external) = self.call_shape(callees, interface);
        let matched = self.matches.get(&(function_id, stmt));
        // Sinks at this call.
        if let (Some(position), Some(matched)) = (position, matched) {
            for sink in &matched.sinks {
                let SinkSpec::CallArgument {
                    arguments,
                    receiver,
                    ..
                } = &query.sinks[*sink]
                else {
                    continue;
                };
                let hit = if method && position == 0 {
                    *receiver
                } else {
                    arguments.contains(if method { position - 1 } else { position })
                };
                if hit && !self.untracked(function_id, fact.slot) {
                    self.frames[frame].hits.push(Hit {
                        sink: *sink,
                        stmt,
                        fact,
                        argument: Some(position),
                    });
                }
            }
        }
        if matched.is_some_and(|matched| matched.blocked) {
            return;
        }
        let propagators = matched
            .map(|matched| matched.propagators.clone())
            .unwrap_or_default();
        if callees.is_empty() && *algorithm == Algorithm::Unknown && interface.is_none() {
            self.frames[frame].unknowns.insert(Unknown::UnresolvedCall {
                function: function.name.clone(),
                line: function.stmts[stmt as usize].pos.line,
            });
        }
        for callee in callees {
            let Callee::Body(target) = callee else {
                continue;
            };
            let entries: Vec<Entry> = match (position, closure) {
                (Some(position), _) => vec![Entry::Param(position, entry_path(fact.path.steps()))],
                (None, Some(_)) => match fact.path.steps().first() {
                    // Taint in this closure's bindings enters its free variable.
                    Some(Step::Capture(body, index)) if body == target => {
                        vec![Entry::Free(*index, entry_path(&fact.path.steps()[1..]))]
                    }
                    Some(_) => Vec::new(),
                    // The whole closure value is tainted: every binding is.
                    None => (0..program.function(*target).free.len())
                        .map(|index| Entry::Free(index as u16, Path::EMPTY))
                        .collect(),
                },
                (None, None) => Vec::new(),
            };
            for entry in entries {
                let Some(exits) = self.summary_exits(*target, entry, Some(frame)) else {
                    continue;
                };
                if self.sink_reaching[*target as usize] {
                    self.frames[frame].nested.push(Nested {
                        stmt,
                        fact,
                        callee: *target,
                        entry,
                        heuristic: false,
                    });
                }
                for exit in exits {
                    if let Exit::Global(..) = exit {
                        if self.frames[frame].exits.insert(exit) {
                            self.frames[frame].global_writes.push((exit, fact, stmt));
                        }
                        continue;
                    }
                    for out in self.exit_into_caller(function_id, stmt, *target, exit) {
                        derived.push((out, Origin::CallReturn { from: fact, stmt }));
                    }
                }
            }
        }
        if !external {
            return;
        }
        // A library function, or a call with no known candidate: a modelled move
        // into what another argument points to, or else its result carries its
        // arguments (and its receiver).
        for (from, to) in &propagators {
            let from_position = match from {
                Some(argument) => *argument as usize + usize::from(method),
                None => 0,
            };
            if position.map(usize::from) == Some(from_position)
                && let Some(Operand::Slot(target)) = args.get(*to as usize + usize::from(method))
            {
                derived.push((
                    Fact::new(*target, Path::limited([Step::Deref], k)),
                    Origin::External {
                        from: fact,
                        stmt,
                        modelled: true,
                    },
                ));
            }
        }
        if propagators.is_empty()
            && let Some(dst) = dst
            && position.is_some()
        {
            derived.push((
                Fact::whole(*dst),
                Origin::External {
                    from: fact,
                    stmt,
                    modelled: false,
                },
            ));
        }
        // A closure passed to a library function may be called by it: one whose
        // bindings carry taint is entered with that taint on its free variable.
        // The capture step names the closure's body.
        if query.callbacks
            && position.is_some()
            && let Some(capture) = fact
                .path
                .steps()
                .iter()
                .position(|step| matches!(step, Step::Capture(..)))
            && let Step::Capture(target, index) = fact.path.steps()[capture]
            && target != NO_BODY
        {
            let entry = Entry::Free(index, entry_path(&fact.path.steps()[capture + 1..]));
            if self.summary_exits(target, entry, Some(frame)).is_some()
                && self.sink_reaching[target as usize]
            {
                self.frames[frame].nested.push(Nested {
                    stmt,
                    fact,
                    callee: target,
                    entry,
                    heuristic: true,
                });
            }
        }
    }

    /// The exits of `target`'s summary for `entry`, computing it if needed. A
    /// summary still being computed (a recursive cycle) answers with what it has
    /// so far, and the asking summary is recomputed when it grows; a top frame
    /// asking gets the summary once its cycle has settled. A traced re-run only
    /// reads summaries that exist.
    fn summary_exits(
        &mut self,
        target: FnId,
        entry: Entry,
        asking_frame: Option<usize>,
    ) -> Option<Vec<Exit>> {
        let key = (target, entry);
        let asking = self.stack.last().copied();
        if let Some(summary) = self.summaries.get_mut(&key) {
            if let Some(asking) = asking {
                summary.dependents.insert(asking);
                if !summary.done {
                    summary.read_partial = true;
                }
            }
            return Some(summary.exits.iter().copied().collect());
        }
        if self.tracing {
            return None;
        }
        if self.stack.len() >= self.query.max_call_depth {
            if let Some(asking_frame) = asking_frame {
                let name = self.program.function(target).name.clone();
                self.frames[asking_frame]
                    .unknowns
                    .insert(Unknown::CallDepth(name));
            }
            return None;
        }
        let frame = self.new_frame(target);
        self.summaries.insert(
            key,
            Summary {
                frame,
                exits: BTreeSet::new(),
                done: false,
                read_partial: false,
                dependents: asking.into_iter().collect(),
            },
        );
        self.compute(key);
        if self.stack.is_empty() {
            self.settle();
        }
        Some(self.summaries[&key].exits.iter().copied().collect())
    }

    fn entry_fact(&self, target: FnId, entry: Entry) -> Option<Fact> {
        let function = self.program.function(target);
        match entry {
            Entry::Param(position, path) => function
                .params
                .get(position as usize)
                .map(|slot| Fact::new(*slot, path)),
            Entry::Free(position, path) => function
                .free
                .get(position as usize)
                .map(|slot| Fact::new(*slot, path)),
        }
    }

    /// (Re)computes a summary's frame from its entry fact. The frame keeps its
    /// hits, the summaries it entered and its unknowns; its facts are dropped.
    fn compute(&mut self, key: SummaryKey) {
        let (target, entry) = key;
        self.summaries_computed += 1;
        let frame = self.summaries[&key].frame;
        self.frames[frame] = Frame {
            function: target,
            ..Frame::default()
        };
        if let Some(fact) = self.entry_fact(target, entry)
            && !(fact.path.is_empty() && self.untracked(target, fact.slot))
        {
            self.frames[frame].facts.insert(fact);
            self.stack.push(key);
            self.close(frame, vec![fact], false);
            self.stack.pop();
        }
        let exits = std::mem::take(&mut self.frames[frame].exits);
        self.frames[frame].facts = FxHashSet::default();
        self.frames[frame].global_writes = Vec::new();
        let summary = self.summaries.get_mut(&key).expect("summary exists");
        let first = !summary.done;
        summary.done = true;
        let changed = summary.exits != exits;
        summary.exits = exits;
        // A reader is out of date when it read a part of the first computation
        // (a cycle), or when a recomputation changed the summary. A recomputation
        // that comes out equal wakes nobody.
        let stale = if first {
            changed && summary.read_partial
        } else {
            changed
        };
        if stale {
            for dependent in &summary.dependents {
                if *dependent != key && self.queued.insert(*dependent) {
                    self.recompute.push_back(*dependent);
                }
            }
        }
    }

    /// Recomputes the summaries whose callees changed after they read them,
    /// until none changes. Exits only grow, so this ends; a summary recomputed
    /// more than [`MAX_RECOMPUTES`] times is not recomputed again and its frame
    /// records the cut.
    fn settle(&mut self) {
        while let Some(key) = self.recompute.pop_front() {
            self.queued.remove(&key);
            let count = self.recomputes.entry(key).or_insert(0);
            *count += 1;
            if *count > MAX_RECOMPUTES {
                let frame = self.summaries[&key].frame;
                let name = self.program.function(key.0).name.clone();
                self.frames[frame].unknowns.insert(Unknown::CallDepth(name));
                continue;
            }
            self.summaries_recomputed += 1;
            self.compute(key);
        }
    }

    /// The summaries that reach a sink: their own, or one in a summary they
    /// entered (transitively).
    fn reaching_summaries(&self) -> FxHashSet<SummaryKey> {
        let mut entered_from: FxHashMap<SummaryKey, Vec<SummaryKey>> = FxHashMap::default();
        let mut reaching = FxHashSet::default();
        let mut work = Vec::new();
        for (key, summary) in &self.summaries {
            let frame = &self.frames[summary.frame];
            if !frame.hits.is_empty() && reaching.insert(*key) {
                work.push(*key);
            }
            for nested in &frame.nested {
                entered_from
                    .entry((nested.callee, nested.entry))
                    .or_default()
                    .push(*key);
            }
        }
        while let Some(key) = work.pop() {
            for caller in entered_from.get(&key).into_iter().flatten() {
                if reaching.insert(*caller) {
                    work.push(*caller);
                }
            }
        }
        reaching
    }

    /// A top frame re-run with provenance, from its starting facts in the order
    /// of the return depth they arrived at: each batch is closed before the next,
    /// so a fact's origins lead only to starting facts of its own depth or less,
    /// and a starting fact returned from another frame leads there to facts of a
    /// smaller depth. Following origins across frames therefore ends at a source.
    fn traced_top(&mut self, frame: usize) -> usize {
        if let Some(traced) = self.traced_tops.get(&frame) {
            return *traced;
        }
        let function = self.frames[frame].function;
        let mut start = self.frames[frame].start.clone();
        start.sort_by_key(|(_, _, depth)| *depth);
        let unknowns = self.frames[frame].unknowns.clone();
        let tracing = std::mem::replace(&mut self.tracing, true);
        let traced = self.new_frame(function);
        self.traced_tops.insert(frame, traced);
        self.frames[traced].unknowns = unknowns;
        let mut index = 0;
        while index < start.len() {
            let depth = start[index].2;
            self.batch = depth;
            let mut facts = Vec::new();
            while index < start.len() && start[index].2 == depth {
                let (fact, origin, _) = start[index];
                index += 1;
                if !self.holds(traced, fact) {
                    self.frames[traced].facts.insert(fact);
                    if let Some(origins) = self.frames[traced].origins.as_mut() {
                        origins.insert(fact, (origin, depth));
                    }
                    facts.push(fact);
                }
            }
            self.close(traced, facts, true);
        }
        self.batch = 0;
        self.tracing = tracing;
        traced
    }

    /// A summary's frame re-run with provenance, from its entry fact.
    fn traced_summary(&mut self, key: SummaryKey) -> Option<usize> {
        if let Some(traced) = self.traced_summaries.get(&key) {
            return Some(*traced);
        }
        let fact = self.entry_fact(key.0, key.1)?;
        let unknowns = self.frames[self.summaries.get(&key)?.frame]
            .unknowns
            .clone();
        let tracing = std::mem::replace(&mut self.tracing, true);
        let traced = self.new_frame(key.0);
        self.traced_summaries.insert(key, traced);
        self.frames[traced].unknowns = unknowns;
        self.frames[traced].facts.insert(fact);
        if let Some(origins) = self.frames[traced].origins.as_mut() {
            origins.insert(fact, (Origin::Entry, 0));
        }
        self.close(traced, vec![fact], false);
        self.tracing = tracing;
        Some(traced)
    }

    /// Reports one flow per sink site (a sink at a call or return) the query
    /// reaches: from a top frame's own hit, or else from the summary nearest to
    /// a top frame by calls that reaches it. Summaries are discovered breadth
    /// first from the top frames' calls, each once, remembering the call that
    /// first entered it; a flow's path is rebuilt along those calls.
    fn report(&mut self, top_frames: &BTreeMap<FnId, usize>, output: &mut QueryOutput) {
        let reaching = self.reaching_summaries();
        let mut entered: FxHashMap<SummaryKey, (Parent, Nested)> = FxHashMap::default();
        let mut order: Vec<SummaryKey> = Vec::new();
        let mut parents: VecDeque<Parent> = top_frames
            .values()
            .map(|frame| Parent::Top(*frame))
            .collect();
        while let Some(parent) = parents.pop_front() {
            let frame = match parent {
                Parent::Top(frame) => frame,
                Parent::Summary(key) => self.summaries[&key].frame,
            };
            for nested in &self.frames[frame].nested {
                let key = (nested.callee, nested.entry);
                if reaching.contains(&key) && !entered.contains_key(&key) {
                    entered.insert(key, (parent, *nested));
                    order.push(key);
                    parents.push_back(Parent::Summary(key));
                }
            }
        }
        let mut reported: BTreeSet<(FnId, u32, usize)> = BTreeSet::new();
        for frame in top_frames.values().copied() {
            let function = self.frames[frame].function;
            for hit in self.frames[frame].hits.clone() {
                let site = (function, hit.stmt, hit.sink);
                if reported.contains(&site) {
                    continue;
                }
                if let Some(flow) = self.top_flow(frame, hit) {
                    reported.insert(site);
                    output.flows.push(flow);
                }
            }
        }
        for key in order {
            let frame = self.summaries[&key].frame;
            let function = self.frames[frame].function;
            for hit in self.frames[frame].hits.clone() {
                let site = (function, hit.stmt, hit.sink);
                if reported.contains(&site) {
                    continue;
                }
                if let Some(flow) = self.summary_flow(key, hit, &entered) {
                    reported.insert(site);
                    output.flows.push(flow);
                }
            }
        }
    }

    /// The flow of a hit in a top frame.
    fn top_flow(&mut self, frame: usize, hit: Hit) -> Option<Flow> {
        let traced = self.traced_top(frame);
        let walk = self.walk(traced, hit.fact)?;
        self.flow(walk, Vec::new(), Precision::Exact, traced, hit)
    }

    /// The flow of a hit in a summary's frame: the steps inside it, and inside
    /// each summary up the calls that first entered it, to the top frame and the
    /// source there.
    fn summary_flow(
        &mut self,
        key: SummaryKey,
        hit: Hit,
        entered: &FxHashMap<SummaryKey, (Parent, Nested)>,
    ) -> Option<Flow> {
        let hit_frame = self.traced_summary(key)?;
        let local = self.walk(hit_frame, hit.fact)?;
        let mut precision = local.precision;
        let mut frames = local.frames;
        // Segments from the sink back to the source, each in source order.
        let mut segments = vec![local.steps];
        let mut current = key;
        for _ in 0..MAX_TRACE {
            let (parent, nested) = *entered.get(&current)?;
            if nested.heuristic {
                precision = precision.max(Precision::Heuristic);
            }
            let parent_frame = match parent {
                Parent::Top(frame) => self.traced_top(frame),
                Parent::Summary(parent_key) => self.traced_summary(parent_key)?,
            };
            precision = precision.max(self.call_precision(parent_frame, nested.stmt));
            let caller = self.frames[parent_frame].function;
            segments.push(vec![(
                caller,
                self.program.function(caller).stmts[nested.stmt as usize].pos,
            )]);
            let walk = self.walk(parent_frame, nested.fact)?;
            precision = precision.max(walk.precision);
            frames.extend(walk.frames.iter().copied());
            match parent {
                Parent::Top(_) => {
                    let middle = segments.into_iter().rev().flatten().collect();
                    let walk = Walk { frames, ..walk };
                    return self.flow(walk, middle, precision, hit_frame, hit);
                }
                Parent::Summary(parent_key) => {
                    segments.push(walk.steps);
                    current = parent_key;
                }
            }
        }
        None
    }

    /// A flow from the walk back to its source, the steps after it, and the hit.
    fn flow(
        &self,
        origin: Walk,
        middle: Vec<(FnId, Pos)>,
        precision: Precision,
        hit_frame: usize,
        hit: Hit,
    ) -> Option<Flow> {
        let source = origin.source?;
        let mut steps = origin.steps;
        steps.extend(middle);
        let sink_function = self.frames[hit_frame].function;
        let sink_pos = self.program.function(sink_function).stmts[hit.stmt as usize].pos;
        steps.push((sink_function, sink_pos));
        steps.dedup();
        let source_pos = steps.first().map(|(_, pos)| *pos).unwrap_or_default();
        let mut unknowns = BTreeSet::new();
        for frame in origin.frames.iter().chain([&hit_frame]) {
            unknowns.extend(self.frames[*frame].unknowns.iter().cloned());
        }
        Some(Flow {
            source: self.query.sources[source].clone(),
            source_function: origin.function,
            source_pos,
            sink: self.query.sinks[hit.sink].clone(),
            sink_function,
            sink_pos,
            sink_argument: hit.argument,
            steps,
            precision: precision.max(origin.precision),
            unknowns,
        })
    }

    /// Whether a frame holds a fact, or a shorter path that covers it.
    fn holds(&self, frame: usize, fact: Fact) -> bool {
        let facts = &self.frames[frame].facts;
        (0..=fact.path.len())
            .any(|len| facts.contains(&Fact::new(fact.slot, fact.path.prefix(len))))
    }

    /// A fact's origin in a traced frame, with the batch it was derived in: its
    /// own, or that of the shortest path covering it (a re-run may reach a
    /// covering path first).
    fn origin(&self, frame: usize, fact: Fact) -> Option<(Fact, Origin, u32)> {
        let origins = self.frames[frame].origins.as_ref()?;
        if let Some((origin, batch)) = origins.get(&fact) {
            return Some((fact, *origin, *batch));
        }
        (0..fact.path.len()).find_map(|len| {
            let prefix = Fact::new(fact.slot, fact.path.prefix(len));
            origins
                .get(&prefix)
                .map(|(origin, batch)| (prefix, *origin, *batch))
        })
    }

    /// Follows a fact in a traced frame back to its source, or to its body's
    /// entry in a summary's frame.
    fn walk(&mut self, frame: usize, fact: Fact) -> Option<Walk> {
        let mut steps = Vec::new();
        let mut frames = vec![frame];
        let mut precision = Precision::Exact;
        let mut current = (frame, fact);
        let mut visited: FxHashSet<(usize, Fact)> = FxHashSet::default();
        for _ in 0..MAX_TRACE {
            let function = self.frames[current.0].function;
            if !visited.insert(current) {
                return None;
            }
            let (held, origin, _) = self.origin(current.0, current.1)?;
            let pos = |stmt: u32| self.program.function(function).stmts[stmt as usize].pos;
            match origin {
                Origin::Source(source) => {
                    steps.reverse();
                    steps.insert(0, (function, self.source_pos(function, held)));
                    return Some(Walk {
                        source: Some(source),
                        function,
                        steps,
                        frames,
                        precision,
                    });
                }
                Origin::Entry => {
                    steps.reverse();
                    return Some(Walk {
                        source: None,
                        function,
                        steps,
                        frames,
                        precision,
                    });
                }
                Origin::Move { from, stmt } => {
                    push_step(&mut steps, (function, pos(stmt)));
                    current = (current.0, from);
                }
                Origin::External {
                    from,
                    stmt,
                    modelled,
                } => {
                    if !modelled {
                        precision = precision.max(Precision::Conservative);
                    }
                    push_step(&mut steps, (function, pos(stmt)));
                    current = (current.0, from);
                }
                Origin::CallReturn { from, stmt } => {
                    precision = precision.max(self.call_precision(current.0, stmt));
                    push_step(&mut steps, (function, pos(stmt)));
                    current = (current.0, from);
                }
                Origin::ReturnedFrom { frame, exit, stmt } => {
                    precision = precision.max(self.call_precision(current.0, stmt));
                    push_step(&mut steps, (function, pos(stmt)));
                    let traced = self.traced_top(frame);
                    frames.push(traced);
                    current = (traced, self.fact_for_exit(traced, exit)?);
                }
                Origin::Global { frame, fact } => {
                    precision = precision.max(Precision::Conservative);
                    let traced = self.traced_top(frame);
                    frames.push(traced);
                    current = (traced, fact);
                }
            }
        }
        None
    }

    fn call_precision(&self, frame: usize, stmt: u32) -> Precision {
        let function = self.program.function(self.frames[frame].function);
        match &function.stmts[stmt as usize].kind {
            StmtKind::Call { algorithm, .. } => match algorithm {
                Algorithm::Static => Precision::Exact,
                Algorithm::Vta => Precision::SetupAware,
                Algorithm::Cha | Algorithm::Unknown => Precision::Conservative,
            },
            _ => Precision::Exact,
        }
    }

    /// The fact in a frame that produced an exit: a returned slot holding it, the
    /// parameter or free variable, or the global's slot (or, for a global a
    /// callee wrote, the fact passed to that callee).
    fn fact_for_exit(&self, frame: usize, exit: Exit) -> Option<Fact> {
        let function_id = self.frames[frame].function;
        let function = self.program.function(function_id);
        let body = &self.index.bodies[function_id as usize];
        match exit {
            // In a traced frame, the returned fact derived from the earliest batch.
            Exit::Return(position, path) => body
                .returns
                .iter()
                .enumerate()
                .filter(|(_, uses)| uses.iter().any(|(_, at)| *at == position))
                .map(|(slot, _)| Fact::new(slot as Slot, path))
                .filter(|fact| self.holds(frame, *fact))
                .min_by_key(|fact| {
                    self.origin(frame, *fact)
                        .map_or(u32::MAX, |(_, _, batch)| batch)
                }),
            Exit::Param(position, path) => function
                .params
                .get(position as usize)
                .map(|slot| Fact::new(*slot, path)),
            Exit::Free(position, path) => function
                .free
                .get(position as usize)
                .map(|slot| Fact::new(*slot, path)),
            Exit::Global(global, path) => body
                .global_of
                .iter()
                .position(|candidate| *candidate == Some(global))
                .map(|slot| Fact::new(slot as Slot, path))
                .or_else(|| {
                    self.frames[frame]
                        .global_writes
                        .iter()
                        .find(|(written, _, _)| *written == exit)
                        .map(|(_, fact, _)| *fact)
                }),
        }
    }

    /// Where a source fact is introduced: its call, its load, or the body start.
    fn source_pos(&self, function: FnId, fact: Fact) -> Pos {
        let body = self.program.function(function);
        for stmt in &body.stmts {
            match &stmt.kind {
                StmtKind::Call { dst: Some(dst), .. } if *dst == fact.slot => return stmt.pos,
                StmtKind::Call { args, .. } if args.contains(&Operand::Slot(fact.slot)) => {
                    return stmt.pos;
                }
                StmtKind::Load { dst, .. } | StmtKind::Alias { dst, .. } if *dst == fact.slot => {
                    return stmt.pos;
                }
                _ => {}
            }
        }
        body.stmts.first().map(|stmt| stmt.pos).unwrap_or_default()
    }
}

/// How a summary was first entered: from a top frame or from another summary.
#[derive(Clone, Copy, Debug)]
enum Parent {
    Top(usize),
    Summary(SummaryKey),
}

fn push_step(steps: &mut Vec<(FnId, Pos)>, step: (FnId, Pos)) {
    if steps.last() != Some(&step) {
        steps.push(step);
    }
}

fn flow_order(flow: &Flow) -> (FnId, u32, u32, FnId, u32, &SinkSpec, &SourceSpec) {
    (
        flow.sink_function,
        flow.sink_pos.line,
        flow.sink_pos.col,
        flow.source_function,
        flow.source_pos.line,
        &flow.sink,
        &flow.source,
    )
}

/// The path a summary is keyed on for a fact entering a body: the fact's steps
/// up to and including the first that is not a dereference. A longer path enters
/// as this shorter one, which covers it, so a body has at most one summary per
/// field of each parameter rather than one per path through its fields.
fn entry_path(steps: &[Step]) -> Path {
    let cut = steps
        .iter()
        .position(|step| !matches!(step, Step::Deref))
        .map_or(steps.len(), |index| index + 1);
    Path::limited(steps[..cut].iter().copied(), MAX_ENTRY_FIELDS)
}

/// A method's qualified name is `(T).M` or `(*T).M`.
pub(crate) fn is_method_name(name: &str) -> bool {
    name.starts_with('(')
}

/// The path a fact on `fact_path` gives the destination of a move from
/// `src_path` to `dst_path`: the rest below the source prefix, or the whole
/// destination prefix when the fact covers the source prefix.
fn forward(fact_path: &Path, src_path: &[Step], dst_path: &[Step], k: usize) -> Option<Path> {
    if let Some(rest) = fact_path.strip(src_path) {
        return Some(Path::join(dst_path, rest, k));
    }
    if fact_path.covers(src_path) {
        return Some(Path::limited(dst_path.iter().copied(), k));
    }
    None
}

/// The path at the source of a move for a fact below a memory step of its
/// destination, which both now share.
fn backward(fact_path: &Path, dst_path: &[Step], src_path: &[Step], k: usize) -> Option<Path> {
    let rest = fact_path.strip(dst_path)?;
    if !rest.iter().any(|step| step.is_memory()) {
        return None;
    }
    Some(Path::join(src_path, rest, k))
}
