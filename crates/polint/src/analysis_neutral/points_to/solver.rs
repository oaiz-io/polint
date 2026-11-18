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

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use super::facts::{
    PointsToBudgetStatus, PointsToConstraintFact, PointsToConstraintKind, PointsToPrecision,
    PointsToSetFact, PointsToStatus,
};
use super::sets::ObjectSet;
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
/// `var` is the load's destination or the store's source; `field` is the
/// field's index, and indices follow field-name order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Projection {
    field: u32,
    var: u32,
    precision: PointsToPrecision,
}

/// One part of a variable's stable identity: a constraint or call that names
/// it in a role, or the object and field of the heap slot it is.
#[derive(Debug, Clone, Copy)]
enum IdentityPart {
    Relation(StableKeyId, &'static str),
    Slot { object: u32, field: u32 },
}

/// An edge to another variable, with the precision of the constraint that
/// made it. A variable's edges are kept in the order of their targets' ids.
type Edge = (u32, PointsToPrecision);

/// What the solve keeps of one variable, at the variable's dense index.
#[derive(Debug, Default)]
struct Var {
    set: ObjectSet,
    /// Objects received and not yet propagated; `Some` exactly while the
    /// variable is on the queue.
    pending: Option<Vec<u32>>,
    /// Inclusion edges, `self -> target`.
    copies: Vec<Edge>,
    /// Edges that carry precision and incompleteness but no objects: a base
    /// to its loads' destinations and to the slots its stores write.
    dependencies: Vec<Edge>,
    /// Ordered by field, then by the variable's id, then by precision.
    loads: Vec<Projection>,
    stores: Vec<Projection>,
    seed_precision: Option<PointsToPrecision>,
    /// The object and field of a heap slot.
    slot: Option<(u32, u32)>,
    /// The calls whose callee set this is.
    calls: Vec<u32>,
    identity: Vec<IdentityPart>,
    /// The call-rule copies into the variable.
    bindings: BTreeSet<CallBinding>,
    /// A budget event cut the variable's propagation short.
    interrupted: bool,
}

#[derive(Debug)]
struct Object {
    id: ObjectTokenId,
    identity: Vec<(StableKeyId, &'static str)>,
    /// Index of the function's signature.
    signature: Option<u32>,
    /// The object's heap slots.
    slots: Vec<u32>,
}

#[derive(Debug, Clone, Copy)]
struct Place {
    var: u32,
    stable_key: StableKeyId,
}

#[derive(Debug)]
struct Signature {
    /// By position.
    params: Vec<(usize, Place)>,
    rest: Option<Place>,
    this: Option<Place>,
    ret: Option<Place>,
}

impl Signature {
    fn param(&self, position: usize) -> Option<Place> {
        self.params
            .binary_search_by_key(&position, |(at, _)| *at)
            .ok()
            .map(|index| self.params[index].1)
    }

    /// The places a call fills: parameters, the rest parameter and `this`.
    fn inputs(&self) -> impl Iterator<Item = u32> + '_ {
        self.params
            .iter()
            .map(|(_, place)| place)
            .chain(self.rest.iter())
            .chain(self.this.iter())
            .map(|place| place.var)
    }
}

#[derive(Debug)]
struct Call {
    stable_key: StableKeyId,
    callee: u32,
    receiver: Option<u32>,
    arguments: Vec<u32>,
    spread_from: Option<usize>,
    result: Option<u32>,
    precision: PointsToPrecision,
}

impl Call {
    /// Arguments before this position are bound by position.
    fn bound(&self) -> usize {
        self.spread_from.map_or(self.arguments.len(), |spread| {
            spread.min(self.arguments.len())
        })
    }
}

/// The solve runs over dense indices: variables in the order they are first
/// named, objects in object-id order and fields in name order. Every list
/// the worklist walks is kept in id order, so the order of the walk, and
/// with it a result cut short by a budget, is that of the ids.
struct Solver<'a> {
    interner: &'a crate::internal_core::StableKeyInterner,
    budget: PointsToBudget,
    var_ids: Vec<PtVarId>,
    var_index: HashMap<PtVarId, u32>,
    vars: Vec<Var>,
    objects: Vec<Object>,
    object_index: HashMap<ObjectTokenId, u32>,
    fields: Vec<String>,
    field_index: HashMap<String, u32>,
    slots: HashMap<(u32, u32), u32>,
    slots_by_field: Vec<Vec<u32>>,
    /// Calls sorted by stable key text.
    calls: Vec<Call>,
    /// In object-id order.
    signatures: Vec<Signature>,
    queue: VecDeque<u32>,
    /// Reused to carry a set's objects along a new edge.
    scratch: Vec<u32>,
    steps: usize,
    halted: bool,
    budget_reasons: BTreeSet<String>,
}

impl<'a> Solver<'a> {
    fn new(interner: &'a crate::internal_core::StableKeyInterner, budget: PointsToBudget) -> Self {
        Self {
            interner,
            budget,
            var_ids: Vec::new(),
            var_index: HashMap::new(),
            vars: Vec::new(),
            objects: Vec::new(),
            object_index: HashMap::new(),
            fields: Vec::new(),
            field_index: HashMap::new(),
            slots: HashMap::new(),
            slots_by_field: Vec::new(),
            calls: Vec::new(),
            signatures: Vec::new(),
            queue: VecDeque::new(),
            scratch: Vec::new(),
            steps: 0,
            halted: false,
            budget_reasons: BTreeSet::new(),
        }
    }

    /// The dense index of `id`, assigned on first use.
    fn var(&mut self, id: PtVarId) -> u32 {
        if let Some(index) = self.var_index.get(&id) {
            return *index;
        }
        let index = u32::try_from(self.vars.len()).expect("points-to variables exceed u32");
        self.var_ids.push(id);
        self.var_index.insert(id, index);
        self.vars.push(Var::default());
        index
    }

    fn name_var(&mut self, id: PtVarId, key: StableKeyId, role: &'static str) -> u32 {
        let var = self.var(id);
        self.vars[var as usize]
            .identity
            .push(IdentityPart::Relation(key, role));
        var
    }

    fn name_object(&mut self, id: ObjectTokenId, key: StableKeyId, role: &'static str) -> u32 {
        let object = self.object_index[&id];
        self.objects[object as usize].identity.push((key, role));
        object
    }

    fn field(&self, name: &str) -> u32 {
        self.field_index[name]
    }

    fn initialize(&mut self, constraints: &[PointsToConstraintFact]) {
        use PointsToConstraintKind as Kind;
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
        let mut objects = BTreeSet::new();
        let mut fields = BTreeSet::new();
        for (_, constraint) in &sorted {
            match &constraint.kind {
                Kind::AddressOf { object, .. } => {
                    objects.insert(*object);
                }
                Kind::CallReturn { value, .. } => {
                    objects.insert(vars::value_fact_object(*value));
                }
                kind => {
                    fields.extend(projection_field(kind));
                }
            }
        }
        for (index, id) in objects.into_iter().enumerate() {
            self.object_index.insert(id, index as u32);
            self.objects.push(Object {
                id,
                identity: Vec::new(),
                signature: None,
                slots: Vec::new(),
            });
        }
        for (index, name) in fields.into_iter().enumerate() {
            self.field_index.insert(name.clone(), index as u32);
            self.fields.push(name);
            self.slots_by_field.push(Vec::new());
        }
        for (_, constraint) in sorted {
            let key = constraint.stable_key;
            let precision = constraint.precision;
            match &constraint.kind {
                Kind::AddressOf { dst, object } => {
                    let dst = self.name_var(*dst, key, "address_of_destination");
                    let object = self.name_object(*object, key, "address_of_object");
                    self.seed(dst, object, precision);
                }
                Kind::CallReturn { dst, value } => {
                    let dst = self.name_var(*dst, key, "call_return_destination");
                    let object = self.name_object(
                        vars::value_fact_object(*value),
                        key,
                        "call_return_object",
                    );
                    self.seed(dst, object, precision);
                }
                Kind::Copy { dst, src } => {
                    let dst = self.name_var(*dst, key, "copy_destination");
                    let src = self.name_var(*src, key, "copy_source");
                    self.add_copy_edge(src, dst, precision);
                }
                Kind::SummaryFlow { dst, src, .. } => {
                    let dst = self.name_var(*dst, key, "summary_destination");
                    let src = self.name_var(*src, key, "summary_source");
                    self.add_copy_edge(src, dst, precision);
                }
                Kind::Load { dst, pointer } => {
                    let dst = self.name_var(*dst, key, "load_destination");
                    let pointer = self.name_var(*pointer, key, "load_pointer");
                    self.add_load(pointer, self.field(CONTENTS_SLOT), dst, precision);
                }
                Kind::FieldLoad { dst, base, .. } | Kind::ElementLoad { dst, base, .. } => {
                    let field = self.field(&projection_field(&constraint.kind).expect("load"));
                    let dst = self.name_var(*dst, key, "projection_load_destination");
                    let base = self.name_var(*base, key, "projection_load_base");
                    self.add_load(base, field, dst, precision);
                }
                Kind::Store { pointer, src } => {
                    let pointer = self.name_var(*pointer, key, "store_pointer");
                    let src = self.name_var(*src, key, "store_source");
                    self.add_store(pointer, self.field(CONTENTS_SLOT), src, precision);
                }
                Kind::FieldStore { base, src, .. } | Kind::ElementStore { base, src, .. } => {
                    let field = self.field(&projection_field(&constraint.kind).expect("store"));
                    let base = self.name_var(*base, key, "projection_store_base");
                    let src = self.name_var(*src, key, "projection_store_source");
                    self.add_store(base, field, src, precision);
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
        for (index, (_, call)) in calls.into_iter().enumerate() {
            let key = call.stable_key;
            let callee = self.name_var(call.callee, key, "call_callee");
            let receiver = call
                .receiver
                .map(|receiver| self.name_var(receiver, key, "call_receiver"));
            let arguments = call
                .arguments
                .iter()
                .map(|argument| self.name_var(*argument, key, "call_argument"))
                .collect();
            let result = call.result.map(|result| {
                let result = self.name_var(result, key, "call_result");
                // The result holds only what the callees return, so it is as
                // weak and as incomplete as the callee set.
                add_edge(
                    &self.var_ids,
                    &mut self.vars[callee as usize].dependencies,
                    result,
                    call.precision,
                );
                result
            });
            self.vars[callee as usize].calls.push(index as u32);
            self.calls.push(Call {
                stable_key: key,
                callee,
                receiver,
                arguments,
                spread_from: call.spread_from,
                result,
                precision: call.precision,
            });
        }
        for (object, signature) in &inputs.signatures {
            let mut place = |place: &SignaturePlace, role: &'static str| Place {
                var: self.name_var(place.var, place.stable_key, role),
                stable_key: place.stable_key,
            };
            let signature = Signature {
                params: signature
                    .params
                    .iter()
                    .map(|(position, param)| (*position, place(param, "function_parameter")))
                    .collect(),
                rest: signature.rest.map(|rest| place(&rest, "function_rest")),
                this: signature.this.map(|this| place(&this, "function_this")),
                ret: signature.ret.map(|ret| place(&ret, "function_return")),
            };
            let index = self.signatures.len() as u32;
            self.signatures.push(signature);
            if let Some(object) = self.object_index.get(object) {
                self.objects[*object as usize].signature = Some(index);
            }
        }
    }

    fn seed(&mut self, dst: u32, object: u32, precision: PointsToPrecision) {
        let seed = &mut self.vars[dst as usize].seed_precision;
        *seed = Some(seed.map_or(precision, |held| held.max(precision)));
        self.enqueue(dst, &[object]);
    }

    fn add_load(&mut self, base: u32, field: u32, dst: u32, precision: PointsToPrecision) {
        add_edge(
            &self.var_ids,
            &mut self.vars[base as usize].dependencies,
            dst,
            precision,
        );
        let load = Projection {
            field,
            var: dst,
            precision,
        };
        insert_projection(&self.var_ids, &mut self.vars[base as usize].loads, load);
    }

    fn add_store(&mut self, base: u32, field: u32, src: u32, precision: PointsToPrecision) {
        let store = Projection {
            field,
            var: src,
            precision,
        };
        insert_projection(&self.var_ids, &mut self.vars[base as usize].stores, store);
    }

    fn run(&mut self) {
        let mut new = Vec::new();
        while let Some(&var) = self.queue.front() {
            if self.steps >= self.budget.max_steps {
                self.halt(BudgetReason::SolverMaxSteps);
                break;
            }
            self.queue.pop_front();
            self.steps += 1;
            let mut delta = self.vars[var as usize].pending.take().unwrap_or_default();
            delta.sort_unstable();
            delta.dedup();
            new.clear();
            self.vars[var as usize].set.missing(&delta, &mut new);
            self.process(var, &new);
            if self.halted {
                break;
            }
        }
    }

    /// Adds `new` to the set of `var` and carries it along every edge out of
    /// `var`: copies, the slots its loads and stores reach, and calls.
    fn process(&mut self, var: u32, new: &[u32]) {
        if new.is_empty() {
            return;
        }
        let at = var as usize;
        if self.vars[at].set.len() + new.len() > self.budget.max_objects_per_var {
            self.vars[at].interrupted = true;
            self.halt(BudgetReason::PointsToMaxObjectsPerVar);
            return;
        }
        self.vars[at].set.add_new(new);
        for index in 0..self.vars[at].copies.len() {
            let dst = self.vars[at].copies[index].0;
            self.enqueue(dst, new);
        }
        for index in 0..self.vars[at].loads.len() {
            let load = self.vars[at].loads[index];
            for object in new {
                let Some(slot) = self.object_slot(*object, load.field) else {
                    self.vars[at].interrupted = true;
                    return;
                };
                self.add_copy_edge(slot, load.var, load.precision);
            }
        }
        for index in 0..self.vars[at].stores.len() {
            let store = self.vars[at].stores[index];
            for object in new {
                let Some(slot) = self.object_slot(*object, store.field) else {
                    self.vars[at].interrupted = true;
                    return;
                };
                add_edge(
                    &self.var_ids,
                    &mut self.vars[at].dependencies,
                    slot,
                    store.precision,
                );
                self.add_copy_edge(store.var, slot, store.precision);
            }
        }
        self.apply_call_rule(var, new);
    }

    /// Connects every call whose callee is `callee` to the functions among
    /// `new` that have a signature: argument to parameter by position,
    /// receiver to `this`, return to result. A parameter the call fills from
    /// a spread, and the rest parameter, receive values the solve does not
    /// model, so they are made heuristic instead of bound.
    fn apply_call_rule(&mut self, callee: u32, new: &[u32]) {
        for call_at in 0..self.vars[callee as usize].calls.len() {
            let index = self.vars[callee as usize].calls[call_at] as usize;
            let (precision, bound, spread, receiver, result) = {
                let call = &self.calls[index];
                (
                    call.precision,
                    call.bound(),
                    call.spread_from.is_some(),
                    call.receiver,
                    call.result,
                )
            };
            let passes_arguments = !self.calls[index].arguments.is_empty();
            for object in new {
                let Some(signature) = self.objects[*object as usize].signature else {
                    continue;
                };
                let signature = signature as usize;
                for position in 0..bound {
                    let argument = self.calls[index].arguments[position];
                    if let Some(param) = self.signatures[signature].param(position) {
                        self.bind(index, argument, param, precision);
                    }
                }
                if spread {
                    let params = &self.signatures[signature].params;
                    let from = params.partition_point(|(position, _)| *position < bound);
                    for at in from..params.len() {
                        let param = self.signatures[signature].params[at].1;
                        add_edge(
                            &self.var_ids,
                            &mut self.vars[callee as usize].dependencies,
                            param.var,
                            PointsToPrecision::Heuristic,
                        );
                    }
                }
                if let Some(rest) = self.signatures[signature].rest
                    && passes_arguments
                {
                    add_edge(
                        &self.var_ids,
                        &mut self.vars[callee as usize].dependencies,
                        rest.var,
                        PointsToPrecision::Heuristic,
                    );
                }
                if let (Some(receiver), Some(this)) = (receiver, self.signatures[signature].this) {
                    self.bind(index, receiver, this, precision);
                }
                if let (Some(result), Some(ret)) = (result, self.signatures[signature].ret) {
                    self.add_copy_edge(ret.var, result, precision);
                    self.vars[result as usize].bindings.insert(CallBinding {
                        call: self.calls[index].stable_key,
                        place: ret.stable_key,
                    });
                }
            }
        }
    }

    /// Copies `src` into a signature place for a call, and makes the place as
    /// weak and as incomplete as the callee set it was reached through.
    fn bind(&mut self, index: usize, src: u32, place: Place, precision: PointsToPrecision) {
        let (callee, key) = (self.calls[index].callee, self.calls[index].stable_key);
        add_edge(
            &self.var_ids,
            &mut self.vars[callee as usize].dependencies,
            place.var,
            precision,
        );
        self.add_copy_edge(src, place.var, precision);
        self.vars[place.var as usize].bindings.insert(CallBinding {
            call: key,
            place: place.stable_key,
        });
    }

    fn add_copy_edge(&mut self, src: u32, dst: u32, precision: PointsToPrecision) {
        if !add_edge(
            &self.var_ids,
            &mut self.vars[src as usize].copies,
            dst,
            precision,
        ) {
            return;
        }
        if self.vars[src as usize].set.is_empty() {
            return;
        }
        let mut objects = std::mem::take(&mut self.scratch);
        objects.clear();
        objects.extend(self.vars[src as usize].set.iter());
        self.enqueue(dst, &objects);
        self.scratch = objects;
    }

    /// Queues the objects of `objects` that `dst` does not hold yet.
    fn enqueue(&mut self, dst: u32, objects: &[u32]) {
        let Var { set, pending, .. } = &mut self.vars[dst as usize];
        match pending {
            Some(pending) => set.missing(objects, pending),
            None => {
                let mut missing = Vec::new();
                set.missing(objects, &mut missing);
                if !missing.is_empty() {
                    *pending = Some(missing);
                    self.queue.push_back(dst);
                }
            }
        }
    }

    fn halt(&mut self, reason: BudgetReason) {
        self.halted = true;
        self.budget_reasons.insert(reason.as_str().to_string());
    }

    /// The variable of `object.field`, created on first use. `None` when the
    /// slot ceiling is reached; the solve has then stopped.
    fn object_slot(&mut self, object: u32, field: u32) -> Option<u32> {
        if let Some(var) = self.slots.get(&(object, field)) {
            return Some(*var);
        }
        if self.slots.len() >= self.budget.max_dynamic_vars {
            self.halt(BudgetReason::PointsToMaxDynamicVars);
            return None;
        }
        let mut hash = slot_hash(
            self.objects[object as usize].id,
            &self.fields[field as usize],
        );
        let id = loop {
            let candidate = vars::slot_var(hash);
            let taken = self
                .var_index
                .get(&candidate)
                .is_some_and(|var| self.vars[*var as usize].slot.is_some());
            if !taken {
                break candidate;
            }
            hash = hash.wrapping_add(1);
        };
        let var = self.var(id);
        let slot = &mut self.vars[var as usize];
        slot.identity.push(IdentityPart::Slot { object, field });
        slot.slot = Some((object, field));
        self.slots.insert((object, field), var);
        self.slots_by_field[field as usize].push(var);
        self.objects[object as usize].slots.push(var);
        Some(var)
    }

    /// Variables whose sets may still be missing objects: the ones left on
    /// the queue or cut short, and everything they can reach. A store through
    /// an incomplete base may write a field of any object, so every slot of
    /// that field is incomplete too; likewise a call through an incomplete
    /// callee that passes values may reach any function, so every parameter
    /// and `this` is incomplete.
    fn incomplete_variables(&self) -> Vec<bool> {
        let mut incomplete = vec![false; self.vars.len()];
        let mut work = (0..self.vars.len() as u32)
            .filter(|var| {
                let var = &self.vars[*var as usize];
                var.pending.is_some() || var.interrupted
            })
            .collect::<Vec<_>>();
        let mut passed_values = false;
        while let Some(var) = work.pop() {
            if std::mem::replace(&mut incomplete[var as usize], true) {
                continue;
            }
            let data = &self.vars[var as usize];
            work.extend(
                data.copies
                    .iter()
                    .chain(&data.dependencies)
                    .map(|(dst, _)| *dst),
            );
            for store in &data.stores {
                work.extend(&self.slots_by_field[store.field as usize]);
            }
            if !passed_values && self.passes_values(var) {
                passed_values = true;
                work.extend(self.signatures.iter().flat_map(Signature::inputs));
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
    fn open_variables(&self, precisions: &[PointsToPrecision]) -> Vec<bool> {
        let mut targets = vec![false; self.vars.len()];
        for var in &self.vars {
            for (dst, _) in var.copies.iter().chain(&var.dependencies) {
                targets[*dst as usize] = true;
            }
        }
        let undefined = |var: u32| {
            let data = &self.vars[var as usize];
            data.seed_precision.is_none()
                && data.slot.is_none()
                && data.bindings.is_empty()
                && !targets[var as usize]
        };
        let mut work = Vec::new();
        for (index, var) in self.vars.iter().enumerate() {
            let read = !var.copies.is_empty()
                || !var.dependencies.is_empty()
                || !var.loads.is_empty()
                || !var.stores.is_empty()
                || !var.calls.is_empty();
            if read {
                work.push(index as u32);
            }
            work.extend(var.stores.iter().map(|store| store.var));
        }
        work.retain(|var| undefined(*var));
        work.extend(
            self.calls
                .iter()
                .filter(|call| self.callees_unresolved(call))
                .filter_map(|call| call.result),
        );
        let mut escaping = Vec::new();
        for call in &self.calls {
            let heuristic = precisions[call.callee as usize] == PointsToPrecision::Heuristic;
            if heuristic || self.callees_unresolved(call) {
                escaping.extend(call.receiver.iter().chain(&call.arguments).copied());
            } else {
                escaping.extend(self.unbound_arguments(call));
            }
        }
        let mut open = vec![false; self.vars.len()];
        let mut escaped = vec![false; self.objects.len()];
        let mut objects = Vec::new();
        loop {
            for var in escaping.drain(..) {
                objects.extend(self.vars[var as usize].set.iter());
            }
            if let Some(object) = objects.pop() {
                if std::mem::replace(&mut escaped[object as usize], true) {
                    continue;
                }
                let slots = &self.objects[object as usize].slots;
                work.extend(slots);
                escaping.extend(slots);
                if let Some(signature) = self.objects[object as usize].signature {
                    let signature = &self.signatures[signature as usize];
                    work.extend(signature.inputs());
                    escaping.extend(signature.ret.iter().map(|place| place.var));
                }
                continue;
            }
            let Some(var) = work.pop() else {
                break;
            };
            if std::mem::replace(&mut open[var as usize], true) {
                continue;
            }
            let data = &self.vars[var as usize];
            work.extend(
                data.copies
                    .iter()
                    .chain(&data.dependencies)
                    .map(|(dst, _)| *dst),
            );
            for store in &data.stores {
                escaping.push(store.var);
                work.extend(&self.slots_by_field[store.field as usize]);
            }
            for index in &data.calls {
                let call = &self.calls[*index as usize];
                escaping.extend(call.receiver.iter().chain(&call.arguments).copied());
            }
        }
        open
    }

    /// True when a call's callee set is empty or holds an object without a
    /// signature, so the call may run code the constraints do not describe.
    fn callees_unresolved(&self, call: &Call) -> bool {
        let callees = &self.vars[call.callee as usize].set;
        callees.is_empty()
            || callees
                .iter()
                .any(|object| self.objects[object as usize].signature.is_none())
    }

    /// The arguments of a call that some callee receives in no parameter:
    /// spread arguments, and arguments at a position its signature does not
    /// bind. Such a callee can still read them, through its rest parameter or
    /// `arguments`.
    fn unbound_arguments<'c>(&'c self, call: &'c Call) -> impl Iterator<Item = u32> + 'c {
        let bound = call.bound();
        let callees = &self.vars[call.callee as usize].set;
        call.arguments
            .iter()
            .enumerate()
            .filter(move |(position, _)| {
                *position >= bound
                    || callees.iter().any(|object| {
                        self.objects[object as usize]
                            .signature
                            .is_none_or(|signature| {
                                self.signatures[signature as usize]
                                    .param(*position)
                                    .is_none()
                            })
                    })
            })
            .map(|(_, argument)| *argument)
    }

    /// True when a call through `callee` passes a receiver or an argument.
    fn passes_values(&self, callee: u32) -> bool {
        self.vars[callee as usize].calls.iter().any(|index| {
            let call = &self.calls[*index as usize];
            call.receiver.is_some() || !call.arguments.is_empty()
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
        let mut reached = vec![Vec::new(); self.vars.len()];
        let mut work = VecDeque::new();
        for (var, data) in self.vars.iter().enumerate() {
            if data.bindings.is_empty() {
                continue;
            }
            let bindings = data.bindings.iter().copied().collect::<Vec<_>>();
            if merge(&mut reached[var], &bindings) {
                work.push_back(var);
            }
        }
        while let Some(var) = work.pop_front() {
            let from = reached[var].clone();
            for (dst, _) in &self.vars[var].copies {
                let dst = *dst as usize;
                if self.vars[dst].set.is_empty() {
                    continue;
                }
                if merge(&mut reached[dst], &from) {
                    work.push_back(dst);
                }
            }
        }
        reached
            .into_iter()
            .enumerate()
            .filter(|(var, bindings)| !bindings.is_empty() && !self.vars[*var].set.is_empty())
            .map(|(var, bindings)| (self.var_ids[var], bindings))
            .collect()
    }

    /// The weakest precision of the inputs that reached each set, carried
    /// along every edge from a non-empty set.
    fn precisions(&self) -> Vec<PointsToPrecision> {
        let mut precision = self
            .vars
            .iter()
            .map(|var| {
                var.seed_precision
                    .filter(|_| !var.set.is_empty())
                    .unwrap_or(PointsToPrecision::FlowInsensitive)
            })
            .collect::<Vec<_>>();
        let mut work = (0..self.vars.len())
            .filter(|var| !self.vars[*var].set.is_empty())
            .collect::<VecDeque<_>>();
        while let Some(var) = work.pop_front() {
            if self.vars[var].set.is_empty() {
                continue;
            }
            let from = precision[var];
            let data = &self.vars[var];
            for (dst, edge) in data.copies.iter().chain(&data.dependencies) {
                let reached = from.max(*edge);
                let current = &mut precision[*dst as usize];
                if reached > *current {
                    *current = reached;
                    work.push_back(*dst as usize);
                }
            }
        }
        precision
    }

    fn object_identity(&self, object: u32) -> String {
        let object = &self.objects[object as usize];
        assert!(
            !object.identity.is_empty(),
            "points-to object must originate in an address or call-return constraint"
        );
        semantic_relation_identity(
            &object
                .identity
                .iter()
                .map(|(key, role)| self.relation_fragment(*key, role))
                .collect(),
        )
    }

    fn variable_identity(&self, var: usize, objects: &mut HashMap<u32, String>) -> String {
        let parts = &self.vars[var].identity;
        assert!(
            !parts.is_empty(),
            "points-to set variable must originate in a constraint or object slot"
        );
        let fragments = parts
            .iter()
            .map(|part| match part {
                IdentityPart::Relation(key, role) => self.relation_fragment(*key, role),
                IdentityPart::Slot { object, field } => {
                    let object = objects
                        .entry(*object)
                        .or_insert_with(|| self.object_identity(*object))
                        .clone();
                    stable_key_text_from_parts(
                        FactFamily::PointsToSet,
                        &[
                            ("object", object),
                            ("slot", self.fields[*field as usize].clone()),
                        ],
                    )
                }
            })
            .collect();
        semantic_relation_identity(&fragments)
    }

    fn relation_fragment(&self, key: StableKeyId, role: &str) -> String {
        stable_key_text_from_parts(
            FactFamily::PointsToSet,
            &[
                ("relation", self.interner.resolve(key).to_string()),
                ("role", role.to_string()),
            ],
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
        let open = self.open_variables(&precisions);
        for (var, data) in self.vars.iter().enumerate() {
            if open[var] && !data.set.is_empty() {
                raise(&mut precisions[var], PointsToPrecision::Heuristic);
            }
        }
        let call_bindings = self.reached_bindings();
        let mut variables = (0..self.vars.len())
            .filter(|var| incomplete[*var] || !self.vars[*var].set.is_empty())
            .collect::<Vec<_>>();
        variables.sort_by_key(|var| self.var_ids[*var]);
        let mut object_identities = HashMap::new();
        let sets = variables
            .into_iter()
            .map(|var| PointsToSetFact {
                id: PointsToSetId(0),
                variable: self.var_ids[var],
                objects: self.vars[var]
                    .set
                    .iter()
                    .map(|object| self.objects[object as usize].id)
                    .collect(),
                status: if incomplete[var] {
                    PointsToStatus::BudgetExceeded
                } else {
                    PointsToStatus::Present
                },
                precision: precisions[var],
                budget: budget_status,
                stable_key: stable_key_from_parts(
                    self.interner,
                    FactFamily::PointsToSet,
                    &[
                        (
                            "variable",
                            self.variable_identity(var, &mut object_identities),
                        ),
                        (
                            "budget",
                            points_to_budget_status_label(budget_status).to_string(),
                        ),
                    ],
                ),
            })
            .collect();
        let set_sizes = self
            .vars
            .iter()
            .map(|var| var.set.len())
            .filter(|size| *size > 0);
        let stats = PointsToSolveStats {
            variables: self.vars.len(),
            objects: self.objects.len(),
            slots: self.slots.len(),
            copy_edges: self.vars.iter().map(|var| var.copies.len()).sum(),
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

/// The field a load or store projects, by name.
fn projection_field(kind: &PointsToConstraintKind) -> Option<String> {
    use PointsToConstraintKind as Kind;
    match kind {
        Kind::Load { .. } | Kind::Store { .. } => Some(CONTENTS_SLOT.to_string()),
        Kind::FieldLoad { field, .. } | Kind::FieldStore { field, .. } => Some(field.clone()),
        Kind::ElementLoad { index, .. } | Kind::ElementStore { index, .. } => {
            Some(format!("[{index}]"))
        }
        _ => None,
    }
}

/// Adds the edge to `dst` or weakens its precision, keeping `edges` in the
/// order of the targets' ids. True when the edge is new.
fn add_edge(
    var_ids: &[PtVarId],
    edges: &mut Vec<Edge>,
    dst: u32,
    precision: PointsToPrecision,
) -> bool {
    let target = var_ids[dst as usize];
    match edges.binary_search_by(|(held, _)| var_ids[*held as usize].cmp(&target)) {
        Ok(at) => {
            raise(&mut edges[at].1, precision);
            false
        }
        Err(at) => {
            edges.insert(at, (dst, precision));
            true
        }
    }
}

/// Adds a load or store unless held, ordered by field, the variable's id and
/// precision.
fn insert_projection(var_ids: &[PtVarId], projections: &mut Vec<Projection>, new: Projection) {
    let key = |projection: &Projection| {
        (
            projection.field,
            var_ids[projection.var as usize],
            projection.precision,
        )
    };
    if let Err(at) = projections.binary_search_by_key(&key(&new), key) {
        projections.insert(at, new);
    }
}

fn raise(current: &mut PointsToPrecision, precision: PointsToPrecision) {
    *current = (*current).max(precision);
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
