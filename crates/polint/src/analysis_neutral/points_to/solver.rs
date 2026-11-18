//! Inclusion-based points-to solve over the language-neutral constraints.
//!
//! The solve is a difference-propagation worklist over constraints sorted by
//! stable key and kind, and a heap slot's variable is a hash of its object and
//! field, so the result does not depend on the order constraints arrive in.
//!
//! A budget event ends the solve and keeps what it derived. A set is
//! `BudgetExceeded` when its variable still had objects to propagate, or can
//! be reached from such a variable through a copy, a field load or store, or
//! a call; every other set finished and is `Present`. Each set carries the
//! weakest precision of the inputs that reached it, so a heuristic input
//! never yields an exact set. A set that may also hold a value the
//! constraints do not describe, such as a parameter nothing binds, a place
//! nothing defines or the result of a call the solve could not resolve, is at
//! most `Heuristic`: it keeps its objects, but they are not all it may hold.
//!
//! Calls are a second input beside the constraints. When a call's callee set
//! gains a function with a signature, the call rule copies each argument into
//! the parameter at its position, the receiver into `this` and the return into
//! the call's result. Those copies enter the same worklist, so the solve stays
//! the least fixed point of constraints and calls together.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::facts::{
    PointsToBudgetStatus, PointsToConstraintFact, PointsToConstraintKind, PointsToPrecision,
    PointsToSetFact, PointsToStatus,
};
use super::store::PointsToOutput;
use super::vars;
use crate::analysis_api::{FactFamily, stable_key_from_parts, stable_key_text_from_parts};
use crate::analysis_neutral::ids::{ObjectTokenId, PointsToSetId, PtVarId};
use crate::analysis_neutral::solver::budget::BudgetReason;
use crate::internal_core::StableKeyId;

/// Ceilings against runaway input, not precision knobs: a solve that reaches
/// one stops, keeps what it derived and names the sets it left incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointsToBudget {
    /// Worklist pops.
    pub max_steps: usize,
    pub max_objects_per_var: usize,
    /// Heap slots the solve may create.
    pub max_dynamic_vars: usize,
}

impl Default for PointsToBudget {
    fn default() -> Self {
        Self {
            max_steps: 50_000_000,
            max_objects_per_var: 4_096,
            max_dynamic_vars: 4_000_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointsToSolveResult {
    pub sets: Vec<PointsToSetFact>,
    pub budget_status: PointsToBudgetStatus,
    pub budget_reasons: BTreeSet<String>,
    /// For each variable a call-rule copy reached, the bindings that brought
    /// values to it: the call's stable key and the signature place it bound.
    /// At most [`MAX_BINDINGS_PER_VAR`] per variable, the first by key text.
    pub call_bindings: BTreeMap<PtVarId, Vec<CallBinding>>,
    pub stats: PointsToSolveStats,
}

/// The size of one solve: what went in and how much work it took. Counts
/// only, so two solves of the same input compare equal; time and memory are
/// logged beside them by [`solve_points_to_logged`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PointsToSolveStats {
    /// Input constraints by kind name.
    pub constraints: BTreeMap<&'static str, usize>,
    pub calls: usize,
    /// Variables the constraints and calls name, and heap slots.
    pub variables: usize,
    /// Objects the constraints name.
    pub objects: usize,
    pub slots: usize,
    pub copy_edges: usize,
    /// Worklist pops.
    pub pops: usize,
    pub largest_set: usize,
    pub sets_over_64: usize,
    pub sets_over_1024: usize,
}

/// One call-rule copy: the call that made it and the signature place it
/// bound (a parameter, `this`, or the return the result reads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CallBinding {
    pub call: StableKeyId,
    pub place: StableKeyId,
}

/// Bindings named per variable; the bound keeps provenance small on long
/// copy chains.
pub const MAX_BINDINGS_PER_VAR: usize = 4;

/// A call the solve connects to its callees. `callee` is the variable whose
/// set is the site's callees.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PointsToCall {
    pub stable_key: StableKeyId,
    pub callee: PtVarId,
    pub receiver: Option<PtVarId>,
    pub arguments: Vec<PtVarId>,
    /// Arguments from this position on are spread from an array and are not
    /// bound by position.
    pub spread_from: Option<usize>,
    pub result: Option<PtVarId>,
    pub precision: PointsToPrecision,
}

/// One place of a function's signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SignaturePlace {
    pub var: PtVarId,
    pub stable_key: StableKeyId,
}

/// The places of one function the call rule binds. A place the frontend does
/// not model is absent and receives nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PointsToSignature {
    pub params: BTreeMap<usize, SignaturePlace>,
    pub rest: Option<SignaturePlace>,
    pub this: Option<SignaturePlace>,
    pub ret: Option<SignaturePlace>,
}

/// The calls of a solve and the signatures of the function objects they may
/// reach, keyed by the function's object.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PointsToCallInputs {
    pub calls: Vec<PointsToCall>,
    pub signatures: BTreeMap<ObjectTokenId, PointsToSignature>,
}

pub fn solve_points_to(
    interner: &crate::internal_core::StableKeyInterner,
    constraints: &[PointsToConstraintFact],
    budget: PointsToBudget,
) -> PointsToSolveResult {
    solve_points_to_with_calls(
        interner,
        constraints,
        &PointsToCallInputs::default(),
        budget,
    )
}

pub fn solve_points_to_with_calls(
    interner: &crate::internal_core::StableKeyInterner,
    constraints: &[PointsToConstraintFact],
    calls: &PointsToCallInputs,
    budget: PointsToBudget,
) -> PointsToSolveResult {
    solve_timed(interner, constraints, calls, budget).0
}

/// Solves like [`solve_points_to_with_calls`] and logs the solve's
/// [`PointsToSolveStats`], its time per step and its memory under the
/// `polint::points_to` tracing target, named by `pass`.
pub fn solve_points_to_logged(
    pass: &'static str,
    interner: &crate::internal_core::StableKeyInterner,
    constraints: &[PointsToConstraintFact],
    calls: &PointsToCallInputs,
    budget: PointsToBudget,
) -> PointsToSolveResult {
    let rss_before = crate::measure::current_rss_bytes();
    let (result, timings) = solve_timed(interner, constraints, calls, budget);
    let stats = &result.stats;
    let rss_after = crate::measure::current_rss_bytes();
    let constraint_counts = stats
        .constraints
        .iter()
        .map(|(kind, count)| format!("{kind}:{count}"))
        .collect::<Vec<_>>()
        .join(",");
    tracing::info!(
        target: "polint::points_to",
        pass,
        constraints = %constraint_counts,
        calls = stats.calls,
        variables = stats.variables,
        objects = stats.objects,
        slots = stats.slots,
        copy_edges = stats.copy_edges,
        pops = stats.pops,
        largest_set = stats.largest_set,
        sets_over_64 = stats.sets_over_64,
        sets_over_1024 = stats.sets_over_1024,
        initialize_ms = timings[0].as_millis() as u64,
        run_ms = timings[1].as_millis() as u64,
        result_ms = timings[2].as_millis() as u64,
        rss_delta_mb = rss_after.saturating_sub(rss_before) / (1024 * 1024),
        peak_rss_mb = crate::measure::peak_rss_bytes() / (1024 * 1024),
        budget = ?result.budget_status,
        "points_to solve"
    );
    result
}

/// The solve and the wall time of its three steps: reading the input,
/// the worklist, and building the result.
fn solve_timed(
    interner: &crate::internal_core::StableKeyInterner,
    constraints: &[PointsToConstraintFact],
    calls: &PointsToCallInputs,
    budget: PointsToBudget,
) -> (PointsToSolveResult, [std::time::Duration; 3]) {
    let started = std::time::Instant::now();
    let mut solver = Solver::new(interner, budget);
    solver.initialize(constraints);
    solver.initialize_calls(calls);
    let initialized = started.elapsed();
    solver.run();
    let ran = started.elapsed();
    let mut result = solver.into_result();
    let finished = started.elapsed();
    result.stats.constraints = constraint_counts(constraints);
    result.stats.calls = calls.calls.len();
    (result, [initialized, ran - initialized, finished - ran])
}

fn constraint_counts(constraints: &[PointsToConstraintFact]) -> BTreeMap<&'static str, usize> {
    use PointsToConstraintKind as Kind;
    let mut counts = BTreeMap::new();
    for constraint in constraints {
        let kind = match constraint.kind {
            Kind::AddressOf { .. } => "address_of",
            Kind::Copy { .. } => "copy",
            Kind::Load { .. } => "load",
            Kind::Store { .. } => "store",
            Kind::FieldLoad { .. } => "field_load",
            Kind::FieldStore { .. } => "field_store",
            Kind::ElementLoad { .. } => "element_load",
            Kind::ElementStore { .. } => "element_store",
            Kind::CallReturn { .. } => "call_return",
            Kind::SummaryFlow { .. } => "summary_flow",
        };
        *counts.entry(kind).or_insert(0) += 1;
    }
    counts
}

const CONTENTS_SLOT: &str = "$contents";

/// A load from, or a store into, a field of the objects a base points to.
/// `var` is the load's destination or the store's source.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Projection {
    field: String,
    var: PtVarId,
    precision: PointsToPrecision,
}

type Edges = BTreeMap<PtVarId, BTreeMap<PtVarId, PointsToPrecision>>;

struct Solver<'a> {
    interner: &'a crate::internal_core::StableKeyInterner,
    budget: PointsToBudget,
    sets: BTreeMap<PtVarId, BTreeSet<ObjectTokenId>>,
    /// Objects a variable has received and not yet propagated. A variable has
    /// an entry exactly while it is on the queue.
    pending: BTreeMap<PtVarId, BTreeSet<ObjectTokenId>>,
    queue: VecDeque<PtVarId>,
    /// Inclusion edges, `src -> dst`, with the precision of the constraint
    /// that made each one.
    copy_edges: Edges,
    /// Edges that carry precision and incompleteness but no objects: a base
    /// to its loads' destinations and to the slots its stores write.
    dependency_edges: Edges,
    loads: BTreeMap<PtVarId, BTreeSet<Projection>>,
    stores: BTreeMap<PtVarId, BTreeSet<Projection>>,
    seed_precision: BTreeMap<PtVarId, PointsToPrecision>,
    slots: BTreeMap<ObjectTokenId, BTreeMap<String, PtVarId>>,
    slot_keys: BTreeMap<PtVarId, (ObjectTokenId, String)>,
    slots_by_field: BTreeMap<String, BTreeSet<PtVarId>>,
    variable_identities: BTreeMap<PtVarId, BTreeSet<String>>,
    object_identities: BTreeMap<ObjectTokenId, BTreeSet<String>>,
    /// Calls sorted by stable key text, and the calls of each callee variable.
    calls: Vec<PointsToCall>,
    calls_by_callee: BTreeMap<PtVarId, Vec<usize>>,
    signatures: BTreeMap<ObjectTokenId, PointsToSignature>,
    /// The call-rule copies into each variable.
    bindings: BTreeMap<PtVarId, BTreeSet<CallBinding>>,
    steps: usize,
    halted: bool,
    /// Variables whose propagation a budget event cut short.
    interrupted: BTreeSet<PtVarId>,
    budget_reasons: BTreeSet<String>,
}

impl<'a> Solver<'a> {
    fn new(interner: &'a crate::internal_core::StableKeyInterner, budget: PointsToBudget) -> Self {
        Self {
            interner,
            budget,
            sets: BTreeMap::new(),
            pending: BTreeMap::new(),
            queue: VecDeque::new(),
            copy_edges: BTreeMap::new(),
            dependency_edges: BTreeMap::new(),
            loads: BTreeMap::new(),
            stores: BTreeMap::new(),
            seed_precision: BTreeMap::new(),
            slots: BTreeMap::new(),
            slot_keys: BTreeMap::new(),
            slots_by_field: BTreeMap::new(),
            variable_identities: BTreeMap::new(),
            object_identities: BTreeMap::new(),
            calls: Vec::new(),
            calls_by_callee: BTreeMap::new(),
            signatures: BTreeMap::new(),
            bindings: BTreeMap::new(),
            steps: 0,
            halted: false,
            interrupted: BTreeSet::new(),
            budget_reasons: BTreeSet::new(),
        }
    }

    fn initialize(&mut self, constraints: &[PointsToConstraintFact]) {
        let mut sorted = constraints
            .iter()
            .map(|constraint| (self.interner.resolve(constraint.stable_key), constraint))
            .collect::<Vec<_>>();
        sorted.sort_by(|(left_key, left), (right_key, right)| {
            left_key
                .cmp(right_key)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.precision.cmp(&right.precision))
        });
        for (key, constraint) in sorted {
            self.record_constraint_identities(&key, &constraint.kind);
            let precision = constraint.precision;
            match &constraint.kind {
                PointsToConstraintKind::AddressOf { dst, object } => {
                    self.seed(*dst, *object, precision);
                }
                PointsToConstraintKind::CallReturn { dst, value } => {
                    self.seed(*dst, vars::value_fact_object(*value), precision);
                }
                PointsToConstraintKind::Copy { dst, src }
                | PointsToConstraintKind::SummaryFlow { dst, src, .. } => {
                    self.add_copy_edge(*src, *dst, precision);
                }
                PointsToConstraintKind::Load { dst, pointer } => {
                    self.add_load(*pointer, CONTENTS_SLOT.to_string(), *dst, precision);
                }
                PointsToConstraintKind::FieldLoad { dst, base, field } => {
                    self.add_load(*base, field.clone(), *dst, precision);
                }
                PointsToConstraintKind::ElementLoad { dst, base, index } => {
                    self.add_load(*base, format!("[{index}]"), *dst, precision);
                }
                PointsToConstraintKind::Store { pointer, src } => {
                    self.add_store(*pointer, CONTENTS_SLOT.to_string(), *src, precision);
                }
                PointsToConstraintKind::FieldStore { base, field, src } => {
                    self.add_store(*base, field.clone(), *src, precision);
                }
                PointsToConstraintKind::ElementStore { base, index, src } => {
                    self.add_store(*base, format!("[{index}]"), *src, precision);
                }
            }
        }
    }

    fn initialize_calls(&mut self, inputs: &PointsToCallInputs) {
        let mut calls = inputs
            .calls
            .iter()
            .map(|call| (self.interner.resolve(call.stable_key), call))
            .collect::<Vec<_>>();
        calls.sort_by(|(left_key, left), (right_key, right)| {
            left_key.cmp(right_key).then_with(|| left.cmp(right))
        });
        calls.dedup_by(|(_, left), (_, right)| left == right);
        for (index, (key, call)) in calls.into_iter().enumerate() {
            self.record_variable_identity(call.callee, &key, "call_callee");
            if let Some(receiver) = call.receiver {
                self.record_variable_identity(receiver, &key, "call_receiver");
            }
            for argument in &call.arguments {
                self.record_variable_identity(*argument, &key, "call_argument");
            }
            if let Some(result) = call.result {
                self.record_variable_identity(result, &key, "call_result");
                // The result holds only what the callees return, so it is as
                // weak and as incomplete as the callee set.
                add_edge(
                    &mut self.dependency_edges,
                    call.callee,
                    result,
                    call.precision,
                );
            }
            self.calls_by_callee
                .entry(call.callee)
                .or_default()
                .push(index);
            self.calls.push(call.clone());
        }
        for signature in inputs.signatures.values() {
            let places = signature
                .params
                .values()
                .map(|place| (place, "function_parameter"))
                .chain(signature.rest.iter().map(|place| (place, "function_rest")))
                .chain(signature.this.iter().map(|place| (place, "function_this")))
                .chain(signature.ret.iter().map(|place| (place, "function_return")));
            for (place, role) in places {
                let key = self.interner.resolve(place.stable_key);
                self.record_variable_identity(place.var, &key, role);
            }
        }
        self.signatures = inputs.signatures.clone();
    }

    fn seed(&mut self, dst: PtVarId, object: ObjectTokenId, precision: PointsToPrecision) {
        raise(
            self.seed_precision.entry(dst).or_insert(precision),
            precision,
        );
        enqueue(
            &self.sets,
            &mut self.pending,
            &mut self.queue,
            dst,
            std::iter::once(object),
        );
    }

    fn add_load(
        &mut self,
        base: PtVarId,
        field: String,
        dst: PtVarId,
        precision: PointsToPrecision,
    ) {
        add_edge(&mut self.dependency_edges, base, dst, precision);
        self.loads.entry(base).or_default().insert(Projection {
            field,
            var: dst,
            precision,
        });
    }

    fn add_store(
        &mut self,
        base: PtVarId,
        field: String,
        src: PtVarId,
        precision: PointsToPrecision,
    ) {
        self.stores.entry(base).or_default().insert(Projection {
            field,
            var: src,
            precision,
        });
    }

    fn run(&mut self) {
        while let Some(&var) = self.queue.front() {
            if self.steps >= self.budget.max_steps {
                self.halt(BudgetReason::SolverMaxSteps);
                break;
            }
            self.queue.pop_front();
            self.steps += 1;
            let delta = self.pending.remove(&var).unwrap_or_default();
            self.process(var, delta);
            if self.halted {
                break;
            }
        }
    }

    fn process(&mut self, var: PtVarId, delta: BTreeSet<ObjectTokenId>) {
        let held = self.sets.get(&var);
        let new = delta
            .into_iter()
            .filter(|object| held.is_none_or(|held| !held.contains(object)))
            .collect::<BTreeSet<_>>();
        if new.is_empty() {
            return;
        }
        if held.map_or(0, BTreeSet::len) + new.len() > self.budget.max_objects_per_var {
            self.interrupted.insert(var);
            self.halt(BudgetReason::PointsToMaxObjectsPerVar);
            return;
        }
        self.sets
            .entry(var)
            .or_default()
            .extend(new.iter().copied());

        if let Some(dsts) = self.copy_edges.get(&var) {
            for dst in dsts.keys() {
                enqueue(
                    &self.sets,
                    &mut self.pending,
                    &mut self.queue,
                    *dst,
                    new.iter().copied(),
                );
            }
        }
        let loads = self.loads.get(&var).cloned().unwrap_or_default();
        for load in &loads {
            for object in &new {
                let Some(slot) = self.object_slot(*object, &load.field) else {
                    self.interrupted.insert(var);
                    return;
                };
                self.add_copy_edge(slot, load.var, load.precision);
            }
        }
        let stores = self.stores.get(&var).cloned().unwrap_or_default();
        for store in &stores {
            for object in &new {
                let Some(slot) = self.object_slot(*object, &store.field) else {
                    self.interrupted.insert(var);
                    return;
                };
                add_edge(&mut self.dependency_edges, var, slot, store.precision);
                self.add_copy_edge(store.var, slot, store.precision);
            }
        }
        self.apply_call_rule(var, &new);
    }

    /// Connects every call whose callee is `callee` to the functions among
    /// `new` that have a signature: argument to parameter by position,
    /// receiver to `this`, return to result. A parameter the call fills from
    /// a spread, and the rest parameter, receive values the solve does not
    /// model, so they are made heuristic instead of bound.
    fn apply_call_rule(&mut self, callee: PtVarId, new: &BTreeSet<ObjectTokenId>) {
        let Some(indices) = self.calls_by_callee.get(&callee).cloned() else {
            return;
        };
        for index in indices {
            let call = self.calls[index].clone();
            for object in new {
                let Some(signature) = self.signatures.get(object).cloned() else {
                    continue;
                };
                let bound = call.spread_from.map_or(call.arguments.len(), |spread| {
                    spread.min(call.arguments.len())
                });
                for (position, argument) in call.arguments[..bound].iter().enumerate() {
                    if let Some(param) = signature.params.get(&position) {
                        self.bind(index, *argument, *param, call.precision);
                    }
                }
                if call.spread_from.is_some() {
                    for param in signature.params.range(bound..).map(|(_, param)| param) {
                        add_edge(
                            &mut self.dependency_edges,
                            callee,
                            param.var,
                            PointsToPrecision::Heuristic,
                        );
                    }
                }
                if let Some(rest) = signature.rest
                    && !call.arguments.is_empty()
                {
                    add_edge(
                        &mut self.dependency_edges,
                        callee,
                        rest.var,
                        PointsToPrecision::Heuristic,
                    );
                }
                if let (Some(receiver), Some(this)) = (call.receiver, signature.this) {
                    self.bind(index, receiver, this, call.precision);
                }
                if let (Some(result), Some(ret)) = (call.result, signature.ret) {
                    self.add_copy_edge(ret.var, result, call.precision);
                    self.bindings
                        .entry(result)
                        .or_default()
                        .insert(CallBinding {
                            call: call.stable_key,
                            place: ret.stable_key,
                        });
                }
            }
        }
    }

    /// Copies `src` into a signature place for a call, and makes the place as
    /// weak and as incomplete as the callee set it was reached through.
    fn bind(
        &mut self,
        index: usize,
        src: PtVarId,
        place: SignaturePlace,
        precision: PointsToPrecision,
    ) {
        let call = &self.calls[index];
        let (callee, key) = (call.callee, call.stable_key);
        add_edge(&mut self.dependency_edges, callee, place.var, precision);
        self.add_copy_edge(src, place.var, precision);
        self.bindings
            .entry(place.var)
            .or_default()
            .insert(CallBinding {
                call: key,
                place: place.stable_key,
            });
    }

    fn add_copy_edge(&mut self, src: PtVarId, dst: PtVarId, precision: PointsToPrecision) {
        if !add_edge(&mut self.copy_edges, src, dst, precision) {
            return;
        }
        if let Some(objects) = self.sets.get(&src) {
            enqueue(
                &self.sets,
                &mut self.pending,
                &mut self.queue,
                dst,
                objects.iter().copied(),
            );
        }
    }

    fn halt(&mut self, reason: BudgetReason) {
        self.halted = true;
        self.budget_reasons.insert(reason.as_str().to_string());
    }

    /// The variable of `object.field`, created on first use. `None` when the
    /// slot ceiling is reached; the solve has then stopped.
    fn object_slot(&mut self, object: ObjectTokenId, field: &str) -> Option<PtVarId> {
        if let Some(var) = self.slots.get(&object).and_then(|slots| slots.get(field)) {
            return Some(*var);
        }
        if self.slot_keys.len() >= self.budget.max_dynamic_vars {
            self.halt(BudgetReason::PointsToMaxDynamicVars);
            return None;
        }
        let mut hash = slot_hash(object, field);
        let var = loop {
            let candidate = vars::slot_var(hash);
            if !self.slot_keys.contains_key(&candidate) {
                break candidate;
            }
            hash = hash.wrapping_add(1);
        };
        let slot_identity = stable_key_text_from_parts(
            FactFamily::PointsToSet,
            &[
                ("object", self.semantic_object_identity(object)),
                ("slot", field.to_string()),
            ],
        );
        self.variable_identities
            .entry(var)
            .or_default()
            .insert(slot_identity);
        self.slots
            .entry(object)
            .or_default()
            .insert(field.to_string(), var);
        self.slot_keys.insert(var, (object, field.to_string()));
        self.slots_by_field
            .entry(field.to_string())
            .or_default()
            .insert(var);
        Some(var)
    }

    /// Variables whose sets may still be missing objects: the ones left on
    /// the queue or cut short, and everything they can reach. A store through
    /// an incomplete base may write a field of any object, so every slot of
    /// that field is incomplete too; likewise a call through an incomplete
    /// callee that passes values may reach any function, so every parameter
    /// and `this` is incomplete.
    fn incomplete_variables(&self) -> BTreeSet<PtVarId> {
        let mut incomplete = BTreeSet::new();
        let mut work = self
            .pending
            .keys()
            .chain(self.interrupted.iter())
            .copied()
            .collect::<Vec<_>>();
        let mut passed_values = false;
        while let Some(var) = work.pop() {
            if !incomplete.insert(var) {
                continue;
            }
            for edges in [&self.copy_edges, &self.dependency_edges] {
                if let Some(dsts) = edges.get(&var) {
                    work.extend(dsts.keys().copied());
                }
            }
            for store in self.stores.get(&var).into_iter().flatten() {
                if let Some(slots) = self.slots_by_field.get(&store.field) {
                    work.extend(slots.iter().copied());
                }
            }
            if !passed_values && self.passes_values(var) {
                passed_values = true;
                work.extend(self.signature_inputs());
            }
        }
        incomplete
    }

    /// Variables whose sets may hold values the constraints do not describe,
    /// given the precision of every set before openness is applied.
    ///
    /// The roots are the variables read as a source that nothing defines (no
    /// seed, no copy into them, no slot, no call binding), and the results of
    /// calls whose callee set is unresolved. Openness spreads along copies and
    /// dependencies, and from a store through an open base to every slot of
    /// its field.
    ///
    /// Code the constraints do not describe can also read values: an object
    /// escapes when it is passed to a call whose callees are unresolved,
    /// heuristic or open, passed where no parameter receives it, stored
    /// through an open base, or held by a slot of an escaped object or by the
    /// return of an escaped function. Such code may write every slot of an
    /// escaped object and call an escaped function with anything, so those
    /// slots and that function's parameters, rest parameter and `this` are
    /// open. Openness and escape feed each other until neither grows.
    fn open_variables(
        &self,
        precisions: &BTreeMap<PtVarId, PointsToPrecision>,
    ) -> BTreeSet<PtVarId> {
        let defined = |var: &PtVarId| {
            self.seed_precision.contains_key(var)
                || self.slot_keys.contains_key(var)
                || self.bindings.contains_key(var)
        };
        let mut targets = BTreeSet::new();
        for edges in [&self.copy_edges, &self.dependency_edges] {
            for dsts in edges.values() {
                targets.extend(dsts.keys().copied());
            }
        }
        let read = self
            .copy_edges
            .keys()
            .chain(self.dependency_edges.keys())
            .chain(self.loads.keys())
            .chain(self.stores.keys())
            .chain(self.stores.values().flatten().map(|store| &store.var))
            .chain(self.calls.iter().map(|call| &call.callee))
            .copied()
            .filter(|var| !defined(var) && !targets.contains(var));
        let unresolved_results = self
            .calls
            .iter()
            .filter(|call| self.callees_unresolved(call))
            .filter_map(|call| call.result);
        let mut work = read.chain(unresolved_results).collect::<Vec<_>>();
        let mut escaping = Vec::new();
        for call in &self.calls {
            let heuristic = precisions
                .get(&call.callee)
                .is_some_and(|precision| *precision == PointsToPrecision::Heuristic);
            if heuristic || self.callees_unresolved(call) {
                escaping.extend(call.receiver.iter().chain(&call.arguments).copied());
            } else {
                escaping.extend(self.unbound_arguments(call));
            }
        }
        let mut open = BTreeSet::new();
        let mut escaped = BTreeSet::new();
        let mut objects = Vec::new();
        loop {
            for var in escaping.drain(..) {
                objects.extend(self.sets.get(&var).into_iter().flatten().copied());
            }
            if let Some(object) = objects.pop() {
                if !escaped.insert(object) {
                    continue;
                }
                for slot in self
                    .slots
                    .get(&object)
                    .into_iter()
                    .flat_map(|slots| slots.values())
                {
                    work.push(*slot);
                    escaping.push(*slot);
                }
                if let Some(signature) = self.signatures.get(&object) {
                    work.extend(
                        signature
                            .params
                            .values()
                            .chain(signature.rest.iter())
                            .chain(signature.this.iter())
                            .map(|place| place.var),
                    );
                    escaping.extend(signature.ret.iter().map(|place| place.var));
                }
                continue;
            }
            let Some(var) = work.pop() else {
                break;
            };
            if !open.insert(var) {
                continue;
            }
            for edges in [&self.copy_edges, &self.dependency_edges] {
                if let Some(dsts) = edges.get(&var) {
                    work.extend(dsts.keys().copied());
                }
            }
            for store in self.stores.get(&var).into_iter().flatten() {
                escaping.push(store.var);
                if let Some(slots) = self.slots_by_field.get(&store.field) {
                    work.extend(slots.iter().copied());
                }
            }
            for index in self.calls_by_callee.get(&var).into_iter().flatten() {
                let call = &self.calls[*index];
                escaping.extend(call.receiver.iter().chain(&call.arguments).copied());
            }
        }
        open
    }

    /// True when a call's callee set is missing or holds an object without a
    /// signature, so the call may run code the constraints do not describe.
    fn callees_unresolved(&self, call: &PointsToCall) -> bool {
        self.sets.get(&call.callee).is_none_or(|objects| {
            objects
                .iter()
                .any(|object| !self.signatures.contains_key(object))
        })
    }

    /// The arguments of a call that some callee receives in no parameter:
    /// spread arguments, and arguments at a position its signature does not
    /// bind. Such a callee can still read them, through its rest parameter or
    /// `arguments`.
    fn unbound_arguments<'c>(
        &'c self,
        call: &'c PointsToCall,
    ) -> impl Iterator<Item = PtVarId> + 'c {
        let bound = call.spread_from.map_or(call.arguments.len(), |spread| {
            spread.min(call.arguments.len())
        });
        let callees = self.sets.get(&call.callee);
        call.arguments
            .iter()
            .enumerate()
            .filter(move |(position, _)| {
                *position >= bound
                    || callees.into_iter().flatten().any(|object| {
                        self.signatures
                            .get(object)
                            .is_none_or(|signature| !signature.params.contains_key(position))
                    })
            })
            .map(|(_, argument)| *argument)
    }

    /// True when a call through `callee` passes a receiver or an argument.
    fn passes_values(&self, callee: PtVarId) -> bool {
        self.calls_by_callee.get(&callee).is_some_and(|indices| {
            indices.iter().any(|index| {
                let call = &self.calls[*index];
                call.receiver.is_some() || !call.arguments.is_empty()
            })
        })
    }

    /// Every place a call fills: parameters, rest parameters and `this`.
    fn signature_inputs(&self) -> impl Iterator<Item = PtVarId> + '_ {
        self.signatures.values().flat_map(|signature| {
            signature
                .params
                .values()
                .chain(signature.rest.iter())
                .chain(signature.this.iter())
                .map(|place| place.var)
        })
    }

    /// The bindings that reached each variable along copy edges, the first
    /// [`MAX_BINDINGS_PER_VAR`] by key text. Keeping the first few of a union
    /// is a join, so the result does not depend on the order of the walk.
    fn reached_bindings(&self) -> BTreeMap<PtVarId, Vec<CallBinding>> {
        let text = |binding: &CallBinding| {
            (
                self.interner.resolve(binding.call),
                self.interner.resolve(binding.place),
            )
        };
        let merge = |held: &mut Vec<CallBinding>, incoming: &[CallBinding]| -> bool {
            let before = held.clone();
            held.extend(incoming.iter().copied());
            held.sort_by_cached_key(text);
            held.dedup();
            held.truncate(MAX_BINDINGS_PER_VAR);
            *held != before
        };
        let mut reached = BTreeMap::<PtVarId, Vec<CallBinding>>::new();
        let mut work = VecDeque::new();
        for (var, bindings) in &self.bindings {
            let bindings = bindings.iter().copied().collect::<Vec<_>>();
            if merge(reached.entry(*var).or_default(), &bindings) {
                work.push_back(*var);
            }
        }
        while let Some(var) = work.pop_front() {
            let from = reached.get(&var).cloned().unwrap_or_default();
            for dst in self
                .copy_edges
                .get(&var)
                .into_iter()
                .flat_map(BTreeMap::keys)
            {
                if !self.sets.contains_key(dst) {
                    continue;
                }
                if merge(reached.entry(*dst).or_default(), &from) {
                    work.push_back(*dst);
                }
            }
        }
        reached.retain(|var, _| self.sets.contains_key(var));
        reached
    }

    /// The weakest precision of the inputs that reached each set, carried
    /// along every edge from a non-empty set.
    fn precisions(&self) -> BTreeMap<PtVarId, PointsToPrecision> {
        let mut precision = self
            .seed_precision
            .iter()
            .filter(|(var, _)| self.sets.contains_key(var))
            .map(|(var, precision)| (*var, *precision))
            .collect::<BTreeMap<_, _>>();
        let mut work = self.sets.keys().copied().collect::<VecDeque<_>>();
        while let Some(var) = work.pop_front() {
            if !self.sets.contains_key(&var) {
                continue;
            }
            let from = precision
                .get(&var)
                .copied()
                .unwrap_or(PointsToPrecision::FlowInsensitive);
            for edges in [&self.copy_edges, &self.dependency_edges] {
                for (dst, edge) in edges.get(&var).into_iter().flatten() {
                    let reached = from.max(*edge);
                    let current = precision
                        .entry(*dst)
                        .or_insert(PointsToPrecision::FlowInsensitive);
                    if reached > *current {
                        *current = reached;
                        work.push_back(*dst);
                    }
                }
            }
        }
        precision
    }

    fn record_constraint_identities(&mut self, key: &str, kind: &PointsToConstraintKind) {
        use PointsToConstraintKind as Kind;
        match kind {
            Kind::AddressOf { dst, object } => {
                self.record_variable_identity(*dst, key, "address_of_destination");
                self.record_object_identity(*object, key, "address_of_object");
            }
            Kind::Copy { dst, src } => {
                self.record_variable_identity(*dst, key, "copy_destination");
                self.record_variable_identity(*src, key, "copy_source");
            }
            Kind::Load { dst, pointer } => {
                self.record_variable_identity(*dst, key, "load_destination");
                self.record_variable_identity(*pointer, key, "load_pointer");
            }
            Kind::Store { pointer, src } => {
                self.record_variable_identity(*pointer, key, "store_pointer");
                self.record_variable_identity(*src, key, "store_source");
            }
            Kind::FieldLoad { dst, base, .. } | Kind::ElementLoad { dst, base, .. } => {
                self.record_variable_identity(*dst, key, "projection_load_destination");
                self.record_variable_identity(*base, key, "projection_load_base");
            }
            Kind::FieldStore { base, src, .. } | Kind::ElementStore { base, src, .. } => {
                self.record_variable_identity(*base, key, "projection_store_base");
                self.record_variable_identity(*src, key, "projection_store_source");
            }
            Kind::CallReturn { dst, value } => {
                self.record_variable_identity(*dst, key, "call_return_destination");
                self.record_object_identity(
                    vars::value_fact_object(*value),
                    key,
                    "call_return_object",
                );
            }
            Kind::SummaryFlow { dst, src, .. } => {
                self.record_variable_identity(*dst, key, "summary_destination");
                self.record_variable_identity(*src, key, "summary_source");
            }
        }
    }

    fn record_variable_identity(&mut self, variable: PtVarId, key: &str, role: &str) {
        let fragment = stable_key_text_from_parts(
            FactFamily::PointsToSet,
            &[("relation", key.to_string()), ("role", role.to_string())],
        );
        self.variable_identities
            .entry(variable)
            .or_default()
            .insert(fragment);
    }

    fn record_object_identity(&mut self, object: ObjectTokenId, key: &str, role: &str) {
        let fragment = stable_key_text_from_parts(
            FactFamily::PointsToSet,
            &[("relation", key.to_string()), ("role", role.to_string())],
        );
        self.object_identities
            .entry(object)
            .or_default()
            .insert(fragment);
    }

    fn semantic_variable_identity(&self, variable: PtVarId) -> String {
        semantic_relation_identity(
            self.variable_identities
                .get(&variable)
                .expect("points-to set variable must originate in a constraint or object slot"),
        )
    }

    fn semantic_object_identity(&self, object: ObjectTokenId) -> String {
        semantic_relation_identity(
            self.object_identities
                .get(&object)
                .expect("points-to object must originate in an address or call-return constraint"),
        )
    }

    fn into_result(self) -> PointsToSolveResult {
        let budget_status = if self.halted {
            PointsToBudgetStatus::BudgetExceeded
        } else {
            PointsToBudgetStatus::WithinBudget
        };
        let incomplete = self.incomplete_variables();
        let mut precisions = self.precisions();
        for variable in self.open_variables(&precisions) {
            if self.sets.contains_key(&variable) {
                raise(
                    precisions
                        .entry(variable)
                        .or_insert(PointsToPrecision::Heuristic),
                    PointsToPrecision::Heuristic,
                );
            }
        }
        let call_bindings = self.reached_bindings();
        let empty = BTreeSet::new();
        let variables = self
            .sets
            .keys()
            .chain(incomplete.iter())
            .copied()
            .collect::<BTreeSet<_>>();
        let sets = variables
            .into_iter()
            .map(|variable| PointsToSetFact {
                id: PointsToSetId(0),
                variable,
                objects: self
                    .sets
                    .get(&variable)
                    .unwrap_or(&empty)
                    .iter()
                    .copied()
                    .collect(),
                status: if incomplete.contains(&variable) {
                    PointsToStatus::BudgetExceeded
                } else {
                    PointsToStatus::Present
                },
                precision: precisions
                    .get(&variable)
                    .copied()
                    .unwrap_or(PointsToPrecision::FlowInsensitive),
                budget: budget_status,
                stable_key: stable_key_from_parts(
                    self.interner,
                    FactFamily::PointsToSet,
                    &[
                        ("variable", self.semantic_variable_identity(variable)),
                        (
                            "budget",
                            points_to_budget_status_label(budget_status).to_string(),
                        ),
                    ],
                ),
            })
            .collect();
        let set_sizes = self.sets.values().map(BTreeSet::len);
        let stats = PointsToSolveStats {
            variables: self
                .variable_identities
                .keys()
                .chain(self.slot_keys.keys())
                .chain(self.sets.keys())
                .collect::<BTreeSet<_>>()
                .len(),
            objects: self.object_identities.len(),
            slots: self.slot_keys.len(),
            copy_edges: self.copy_edges.values().map(BTreeMap::len).sum(),
            pops: self.steps,
            largest_set: set_sizes.clone().max().unwrap_or(0),
            sets_over_64: set_sizes.clone().filter(|size| *size > 64).count(),
            sets_over_1024: set_sizes.filter(|size| *size > 1024).count(),
            ..PointsToSolveStats::default()
        };
        PointsToSolveResult {
            sets,
            budget_status,
            budget_reasons: self.budget_reasons,
            call_bindings,
            stats,
        }
    }
}

/// Adds `src -> dst` or weakens its precision. True when the edge is new.
fn add_edge(edges: &mut Edges, src: PtVarId, dst: PtVarId, precision: PointsToPrecision) -> bool {
    match edges.entry(src).or_default().entry(dst) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(precision);
            true
        }
        std::collections::btree_map::Entry::Occupied(mut entry) => {
            raise(entry.get_mut(), precision);
            false
        }
    }
}

fn raise(current: &mut PointsToPrecision, precision: PointsToPrecision) {
    *current = (*current).max(precision);
}

/// Queues the objects of `objects` that `dst` does not hold yet.
fn enqueue(
    sets: &BTreeMap<PtVarId, BTreeSet<ObjectTokenId>>,
    pending: &mut BTreeMap<PtVarId, BTreeSet<ObjectTokenId>>,
    queue: &mut VecDeque<PtVarId>,
    dst: PtVarId,
    objects: impl Iterator<Item = ObjectTokenId>,
) {
    let held = sets.get(&dst);
    let mut objects = objects
        .filter(|object| held.is_none_or(|held| !held.contains(object)))
        .peekable();
    if objects.peek().is_none() {
        return;
    }
    match pending.entry(dst) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(objects.collect());
            queue.push_back(dst);
        }
        std::collections::btree_map::Entry::Occupied(mut entry) => entry.get_mut().extend(objects),
    }
}

/// FNV-1a over the object id and the field name.
fn slot_hash(object: ObjectTokenId, field: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    object
        .0
        .to_le_bytes()
        .iter()
        .chain(field.as_bytes())
        .fold(OFFSET, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
        })
}

fn semantic_relation_identity(fragments: &BTreeSet<String>) -> String {
    fragments
        .iter()
        .map(|fragment| format!("{}:{fragment}", fragment.len()))
        .collect()
}

fn points_to_budget_status_label(status: PointsToBudgetStatus) -> &'static str {
    match status {
        PointsToBudgetStatus::WithinBudget => "within_budget",
        PointsToBudgetStatus::BudgetExceeded => "budget_exceeded",
        PointsToBudgetStatus::NotRun => "not_run",
    }
}

pub fn output_with_solved_sets(
    interner: &crate::internal_core::StableKeyInterner,
    constraints: Vec<PointsToConstraintFact>,
    budget: PointsToBudget,
) -> PointsToOutput {
    let solve = solve_points_to_logged(
        "type_value_alias",
        interner,
        &constraints,
        &PointsToCallInputs::default(),
        budget,
    );
    PointsToOutput {
        constraints,
        sets: solve.sets,
    }
    .normalized(interner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::ids::{PointsToConstraintId, ValueFactId};

    #[test]
    fn solver_handles_core_constraint_vocabulary_deterministically() {
        let constraints = vec![
            constraint(
                "addr-a",
                PointsToConstraintKind::AddressOf {
                    dst: PtVarId(1),
                    object: ObjectTokenId(10),
                },
            ),
            constraint(
                "copy",
                PointsToConstraintKind::Copy {
                    dst: PtVarId(2),
                    src: PtVarId(1),
                },
            ),
            constraint(
                "field-store",
                PointsToConstraintKind::FieldStore {
                    base: PtVarId(1),
                    field: "name".to_string(),
                    src: PtVarId(2),
                },
            ),
            constraint(
                "field-load",
                PointsToConstraintKind::FieldLoad {
                    dst: PtVarId(3),
                    base: PtVarId(1),
                    field: "name".to_string(),
                },
            ),
            constraint(
                "element-store",
                PointsToConstraintKind::ElementStore {
                    base: PtVarId(1),
                    index: "dynamic".to_string(),
                    src: PtVarId(2),
                },
            ),
            constraint(
                "element-load",
                PointsToConstraintKind::ElementLoad {
                    dst: PtVarId(4),
                    base: PtVarId(1),
                    index: "dynamic".to_string(),
                },
            ),
            constraint(
                "call-return",
                PointsToConstraintKind::CallReturn {
                    dst: PtVarId(5),
                    value: crate::analysis_neutral::ids::ValueFactId(42),
                },
            ),
            constraint(
                "summary-flow",
                PointsToConstraintKind::SummaryFlow {
                    dst: PtVarId(6),
                    src: PtVarId(5),
                    summary_key: "return".to_string(),
                },
            ),
            constraint(
                "load",
                PointsToConstraintKind::Load {
                    dst: PtVarId(7),
                    pointer: PtVarId(1),
                },
            ),
            constraint(
                "store",
                PointsToConstraintKind::Store {
                    pointer: PtVarId(1),
                    src: PtVarId(2),
                },
            ),
        ];
        let first = solve_points_to(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            PointsToBudget::default(),
        );
        let second = solve_points_to(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            PointsToBudget::default(),
        );

        assert_eq!(first, second);
        assert_eq!(first.budget_status, PointsToBudgetStatus::WithinBudget);
        assert!(
            first.sets.iter().any(|set| {
                set.variable == PtVarId(2) && set.objects.contains(&ObjectTokenId(10))
            })
        );
        assert!(first.sets.iter().any(|set| {
            set.variable == PtVarId(6)
                && set
                    .objects
                    .contains(&vars::value_fact_object(ValueFactId(42)))
        }));
    }

    #[test]
    fn solver_reports_budget_exhaustion_as_unknown_budget_status() {
        let constraints = vec![
            constraint(
                "addr-a",
                PointsToConstraintKind::AddressOf {
                    dst: PtVarId(1),
                    object: ObjectTokenId(10),
                },
            ),
            constraint(
                "addr-b",
                PointsToConstraintKind::AddressOf {
                    dst: PtVarId(1),
                    object: ObjectTokenId(11),
                },
            ),
        ];
        let result = solve_points_to(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            PointsToBudget {
                max_objects_per_var: 1,
                ..PointsToBudget::default()
            },
        );

        assert_eq!(result.budget_status, PointsToBudgetStatus::BudgetExceeded);
        assert_eq!(
            result.budget_reasons,
            BTreeSet::from(["points_to.max_objects_per_var".to_string()])
        );
        assert!(
            result
                .sets
                .iter()
                .any(|set| set.status == PointsToStatus::BudgetExceeded)
        );
    }

    #[test]
    fn solver_reports_dynamic_var_budget_reason() {
        let constraints = vec![
            constraint(
                "addr",
                PointsToConstraintKind::AddressOf {
                    dst: PtVarId(1),
                    object: ObjectTokenId(10),
                },
            ),
            constraint(
                "load",
                PointsToConstraintKind::Load {
                    dst: PtVarId(2),
                    pointer: PtVarId(1),
                },
            ),
        ];
        let result = solve_points_to(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            PointsToBudget {
                max_dynamic_vars: 0,
                ..PointsToBudget::default()
            },
        );

        assert_eq!(result.budget_status, PointsToBudgetStatus::BudgetExceeded);
        assert_eq!(
            result.budget_reasons,
            BTreeSet::from(["points_to.max_dynamic_vars".to_string()])
        );
    }

    #[test]
    fn solver_reports_step_budget_reason() {
        let constraints = vec![constraint(
            "addr",
            PointsToConstraintKind::AddressOf {
                dst: PtVarId(1),
                object: ObjectTokenId(10),
            },
        )];
        let result = solve_points_to(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            PointsToBudget {
                max_steps: 0,
                ..PointsToBudget::default()
            },
        );

        assert_eq!(result.budget_status, PointsToBudgetStatus::BudgetExceeded);
        assert_eq!(
            result.budget_reasons,
            BTreeSet::from(["solver.max_steps".to_string()])
        );
    }

    fn constraint(stable_key: &str, kind: PointsToConstraintKind) -> PointsToConstraintFact {
        PointsToConstraintFact {
            id: PointsToConstraintId(0),
            kind,
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }
}
