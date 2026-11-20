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
//!
//! A depth bound limits how many field loads and stores a value may cross.
//! The bounded solve runs in levels: a copy into or out of a heap slot
//! carries its objects to the next level, and every other edge carries them
//! within the level. Objects that would reach a level past the bound are
//! dropped, and the sets they would have reached, with everything those can
//! reach, are `IndirectionBound`: what they hold was derived, and they may
//! hold more. Without a bound the solve is one level.
//!
//! Every variable on a cycle of copies ends with the same set, so the solve
//! merges a cycle into one representative once it finds it: the cycle's set
//! is kept and propagated once, along the edges of all its variables. A cycle
//! is looked for when a copy ends at a set equal to its source's, and only
//! over copies within a level. Merging changes how often objects are carried,
//! not the sets the solve ends with.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use super::facts::{
    PointsToBudgetStatus, PointsToConstraintFact, PointsToConstraintKind, PointsToPrecision,
    PointsToSetFact, PointsToStatus,
};
use super::sets::ObjectSet;
use super::store::PointsToOutput;
use super::tiers::{Inflow, InflowSet};
use super::vars;
use crate::analysis_api::{FactFamily, stable_key_from_parts, stable_key_text_from_parts};
use crate::analysis_neutral::ids::{ObjectTokenId, PointsToSetId, PtVarId};
use crate::analysis_neutral::solver::budget::BudgetReason;
use crate::internal_core::StableKeyId;
use rayon::prelude::*;

/// Ceilings against runaway input, not precision knobs: a solve that reaches
/// one stops, keeps what it derived and names the sets it left incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointsToBudget {
    /// Worklist pops.
    pub max_steps: usize,
    pub max_objects_per_var: usize,
    /// Heap slots the solve may create.
    pub max_dynamic_vars: usize,
    /// Field loads and stores a value may cross; `None` for no bound.
    pub max_indirection_depth: Option<usize>,
}

impl Default for PointsToBudget {
    fn default() -> Self {
        Self {
            max_steps: 50_000_000,
            max_objects_per_var: 4_096,
            max_dynamic_vars: 4_000_000,
            max_indirection_depth: None,
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
    /// How each non-empty set was reached, for the sets where that is not
    /// the default: some value from outside may reach it, or some object
    /// reached it through a heap slot.
    pub grounds: BTreeMap<PtVarId, SetGrounds>,
    pub stats: PointsToSolveStats,
}

/// How a set was reached, beside the objects it holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SetGrounds {
    /// The most specific way a value from outside may reach the set. Named
    /// only for a set whose inputs were otherwise at most local
    /// flow-sensitive; a weaker set is heuristic on its own account.
    pub inflow: Option<Inflow>,
    /// Some object reached the set through a heap slot, so the set relies on
    /// what the package stores in that field.
    pub through_slot: bool,
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
    /// Variables merged into another on a copy cycle.
    pub collapsed: usize,
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
        collapsed = stats.collapsed,
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
    /// Objects received for the next level; `Some` exactly while the
    /// variable is on the deferred list.
    deferred: Option<Vec<u32>>,
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
    /// The depth bound dropped objects the variable would have received.
    bounded: bool,
    /// The variables merged into this one, when it represents a copy cycle.
    merged: Vec<u32>,
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
    /// The representative of each variable's copy cycle, the variable itself
    /// when it is on none. A representative holds the set, the pending and
    /// the deferred objects of its whole cycle.
    reps: Vec<u32>,
    /// Copy edges already queued for a cycle check.
    checked: HashSet<(u32, u32)>,
    /// Copy edges to check for a cycle at the next walk.
    candidates: Vec<(u32, u32)>,
    /// The step from which the next cycle walk may run: a walk waits for as
    /// many steps as the last one visited variables, so walking never costs
    /// more than the propagation between walks.
    next_walk: usize,
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
    /// Variables holding objects for the next level, in the order they
    /// received them.
    deferred: Vec<u32>,
    /// The level the queue is at: the field loads and stores its objects
    /// crossed.
    level: usize,
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
            reps: Vec::new(),
            checked: HashSet::new(),
            candidates: Vec::new(),
            next_walk: 0,
            objects: Vec::new(),
            object_index: HashMap::new(),
            fields: Vec::new(),
            field_index: HashMap::new(),
            slots: HashMap::new(),
            slots_by_field: Vec::new(),
            calls: Vec::new(),
            signatures: Vec::new(),
            queue: VecDeque::new(),
            deferred: Vec::new(),
            level: 0,
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
        self.reps.push(index);
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

    /// Drains the queue level by level until no level is left or the depth
    /// bound is reached.
    fn run(&mut self) {
        loop {
            self.drain();
            if self.halted || self.deferred.is_empty() {
                return;
            }
            self.level += 1;
            let past_bound = self
                .budget
                .max_indirection_depth
                .is_some_and(|depth| self.level > depth);
            let mut dropped = false;
            for var in std::mem::take(&mut self.deferred) {
                let var = self.reps[var as usize];
                let Some(objects) = self.vars[var as usize].deferred.take() else {
                    continue;
                };
                if past_bound {
                    let mut missing = Vec::new();
                    self.vars[var as usize].set.missing(&objects, &mut missing);
                    if !missing.is_empty() {
                        self.vars[var as usize].bounded = true;
                        dropped = true;
                    }
                } else {
                    self.enqueue(var, &objects);
                }
            }
            if past_bound {
                if dropped {
                    self.budget_reasons.insert(
                        BudgetReason::PointsToMaxIndirectionDepth
                            .as_str()
                            .to_string(),
                    );
                }
                return;
            }
        }
    }

    fn drain(&mut self) {
        let mut new = Vec::new();
        while let Some(&var) = self.queue.front() {
            if self.steps >= self.budget.max_steps {
                self.halt(BudgetReason::SolverMaxSteps);
                break;
            }
            self.queue.pop_front();
            self.steps += 1;
            let var = self.reps[var as usize];
            let mut delta = self.vars[var as usize].pending.take().unwrap_or_default();
            delta.sort_unstable();
            delta.dedup();
            new.clear();
            self.vars[var as usize].set.missing(&delta, &mut new);
            self.process(var, &new);
            if self.halted {
                break;
            }
            self.collapse_cycles();
        }
    }

    /// Adds `new` to the set of the representative `var` and carries it along
    /// every edge out of `var` and the variables merged into it.
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
        for member in 0..=self.vars[at].merged.len() {
            let member = member
                .checked_sub(1)
                .map_or(var, |index| self.vars[at].merged[index]);
            if !self.carry(member, new) {
                return;
            }
        }
    }

    /// Carries `new` along the edges out of `var`: copies, the slots its
    /// loads and stores reach, and calls. False when a budget event stopped
    /// it.
    fn carry(&mut self, var: u32, new: &[u32]) -> bool {
        let at = var as usize;
        for index in 0..self.vars[at].copies.len() {
            let dst = self.vars[at].copies[index].0;
            self.flow(var, dst, new);
            self.watch_for_cycle(var, dst);
        }
        for index in 0..self.vars[at].loads.len() {
            let load = self.vars[at].loads[index];
            for object in new {
                let Some(slot) = self.object_slot(*object, load.field) else {
                    self.vars[at].interrupted = true;
                    return false;
                };
                self.add_copy_edge(slot, load.var, load.precision);
            }
        }
        for index in 0..self.vars[at].stores.len() {
            let store = self.vars[at].stores[index];
            for object in new {
                let Some(slot) = self.object_slot(*object, store.field) else {
                    self.vars[at].interrupted = true;
                    return false;
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
        true
    }

    /// Queues the copy `src -> dst` for a cycle check the first time both
    /// ends hold the same objects after `src` propagated: on a cycle every
    /// set ends equal, and an edge back into a variable that already passed
    /// its objects on is where a cycle closes.
    fn watch_for_cycle(&mut self, src: u32, dst: u32) {
        let (from, to) = (self.reps[src as usize], self.reps[dst as usize]);
        if from == to || self.crosses_field(src, dst) {
            return;
        }
        let (held, reached) = (&self.vars[from as usize].set, &self.vars[to as usize].set);
        if held.len() == reached.len() && held == reached && self.checked.insert((src, dst)) {
            self.candidates.push((src, dst));
        }
    }

    /// Merges every copy cycle reachable from the queued candidates, in one
    /// walk once enough steps have passed since the last.
    fn collapse_cycles(&mut self) {
        if self.candidates.is_empty() || self.steps < self.next_walk {
            return;
        }
        let roots = self
            .candidates
            .drain(..)
            .map(|(_, dst)| self.reps[dst as usize])
            .collect::<Vec<_>>();
        let (cycles, visited) = self.cycles_from(&roots);
        self.next_walk = self.steps + visited;
        for cycle in cycles {
            self.merge(cycle);
        }
    }

    /// The copy cycles among the representatives reachable from `roots`, each
    /// of more than one representative, and how many representatives the walk
    /// visited: Tarjan's strongly connected components over copies that stay
    /// within a level. The components are disjoint, so each can be merged on
    /// its own.
    fn cycles_from(&self, roots: &[u32]) -> (Vec<Vec<u32>>, usize) {
        struct Visit {
            index: u32,
            low: u32,
            on_stack: bool,
        }
        let mut visits: HashMap<u32, Visit> = HashMap::new();
        let mut stack = Vec::new();
        let mut frames: Vec<(u32, Vec<u32>, usize)> = Vec::new();
        let mut cycles = Vec::new();
        let open = |var: u32, visits: &mut HashMap<u32, Visit>, stack: &mut Vec<u32>| {
            let index = visits.len() as u32;
            visits.insert(
                var,
                Visit {
                    index,
                    low: index,
                    on_stack: true,
                },
            );
            stack.push(var);
            (var, self.successors(var), 0)
        };
        for root in roots {
            if visits.contains_key(root) {
                continue;
            }
            frames.push(open(*root, &mut visits, &mut stack));
            while let Some((var, successors, next)) = frames.last_mut() {
                let var = *var;
                if let Some(&successor) = successors.get(*next) {
                    *next += 1;
                    match visits.get(&successor) {
                        None => frames.push(open(successor, &mut visits, &mut stack)),
                        Some(visit) if visit.on_stack => {
                            let index = visit.index;
                            let low = &mut visits.get_mut(&var).expect("visited").low;
                            *low = (*low).min(index);
                        }
                        Some(_) => {}
                    }
                    continue;
                }
                frames.pop();
                let (index, low) = {
                    let visit = &visits[&var];
                    (visit.index, visit.low)
                };
                if let Some((parent, _, _)) = frames.last() {
                    let parent = &mut visits.get_mut(parent).expect("visited").low;
                    *parent = (*parent).min(low);
                }
                if low == index {
                    let mut cycle = Vec::new();
                    while let Some(member) = stack.pop() {
                        visits.get_mut(&member).expect("visited").on_stack = false;
                        cycle.push(member);
                        if member == var {
                            break;
                        }
                    }
                    if cycle.len() > 1 {
                        cycles.push(cycle);
                    }
                }
            }
        }
        (cycles, visits.len())
    }

    /// The representatives that the copies out of the representative `var`
    /// and its merged variables reach within a level.
    fn successors(&self, var: u32) -> Vec<u32> {
        let members = std::iter::once(var).chain(self.vars[var as usize].merged.iter().copied());
        let mut successors = Vec::new();
        for member in members {
            for (dst, _) in &self.vars[member as usize].copies {
                let to = self.reps[*dst as usize];
                if to != var && !self.crosses_field(member, *dst) {
                    successors.push(to);
                }
            }
        }
        successors
    }

    /// Merges the representatives of `cycle` into the lowest of them. Every
    /// variable of a cycle ends with the same set, so they share one. The
    /// shared set keeps the objects every member has already carried along
    /// its own edges; the rest are queued again so they reach every member's
    /// edges. Carrying an object twice along an edge changes nothing.
    fn merge(&mut self, cycle: Vec<u32>) {
        let rep = *cycle.iter().min().expect("a cycle has members");
        let smallest = *cycle
            .iter()
            .min_by_key(|var| self.vars[**var as usize].set.len())
            .expect("a cycle has members");
        let carried = self.vars[smallest as usize]
            .set
            .iter()
            .filter(|object| {
                cycle
                    .iter()
                    .all(|var| self.vars[*var as usize].set.contains(*object))
            })
            .collect::<Vec<_>>();
        let queued = self.vars[rep as usize].pending.is_some();
        let listed = self.vars[rep as usize].deferred.is_some();
        let mut held = Vec::new();
        let mut deferred: Option<Vec<u32>> = None;
        for var in &cycle {
            let data = &mut self.vars[*var as usize];
            held.extend(data.set.iter());
            held.extend(data.pending.take().into_iter().flatten());
            if let Some(objects) = data.deferred.take() {
                deferred.get_or_insert_default().extend(objects);
            }
        }
        let mut set = ObjectSet::default();
        set.add_new(&carried);
        held.sort_unstable();
        held.dedup();
        held.retain(|object| !set.contains(*object));
        for var in cycle.iter().copied().filter(|var| *var != rep) {
            let data = &mut self.vars[var as usize];
            data.set = ObjectSet::default();
            let members = std::mem::take(&mut data.merged);
            for member in std::iter::once(var).chain(members) {
                self.reps[member as usize] = rep;
                self.vars[rep as usize].merged.push(member);
            }
        }
        let data = &mut self.vars[rep as usize];
        data.set = set;
        if !held.is_empty() {
            data.pending = Some(held);
            if !queued {
                self.queue.push_back(rep);
            }
        }
        if let Some(objects) = deferred {
            data.deferred = Some(objects);
            if !listed {
                self.deferred.push(rep);
            }
        }
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
        let held = &self.vars[self.reps[src as usize] as usize].set;
        if held.is_empty() {
            return;
        }
        let mut objects = std::mem::take(&mut self.scratch);
        objects.clear();
        objects.extend(held.iter());
        self.flow(src, dst, &objects);
        self.scratch = objects;
    }

    /// Carries `objects` along the copy `src -> dst`. Under a depth bound a
    /// copy into or out of a heap slot crosses a field store or load, so the
    /// objects arrive at the next level.
    fn flow(&mut self, src: u32, dst: u32, objects: &[u32]) {
        if !self.crosses_field(src, dst) {
            self.enqueue(dst, objects);
            return;
        }
        let dst = self.reps[dst as usize];
        let Var { set, deferred, .. } = &mut self.vars[dst as usize];
        if hold_missing(set, deferred, objects) {
            self.deferred.push(dst);
        }
    }

    /// True when a depth bound is set and the copy `src -> dst` reads or
    /// writes a heap slot, so it carries objects to the next level.
    fn crosses_field(&self, src: u32, dst: u32) -> bool {
        self.budget.max_indirection_depth.is_some()
            && (self.vars[src as usize].slot.is_some() || self.vars[dst as usize].slot.is_some())
    }

    /// Queues the objects of `objects` that `dst` does not hold yet.
    fn enqueue(&mut self, dst: u32, objects: &[u32]) {
        let dst = self.reps[dst as usize];
        let Var { set, pending, .. } = &mut self.vars[dst as usize];
        if hold_missing(set, pending, objects) {
            self.queue.push_back(dst);
        }
    }

    /// The set of `var`: its representative's.
    fn set(&self, var: usize) -> &ObjectSet {
        &self.vars[self.reps[var] as usize].set
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
    /// the queue, holding objects for a later level, or cut short, and
    /// everything they can reach. A store through an incomplete base may
    /// write a field of any object, so every slot of that field is incomplete
    /// too; likewise a call through an incomplete callee that passes values
    /// may reach any function, so every parameter and `this` is incomplete.
    fn incomplete_variables(&self) -> Vec<bool> {
        self.reach(|var| var.pending.is_some() || var.deferred.is_some() || var.interrupted)
    }

    /// Variables that may lack values the depth bound stopped: the ones it
    /// dropped objects for, and everything they can reach as
    /// [`Self::incomplete_variables`] spreads.
    fn bounded_variables(&self) -> Vec<bool> {
        self.reach(|var| var.bounded)
    }

    /// The variables `root` selects and everything they can reach: along
    /// copies and dependencies, from a store to every slot of its field, and
    /// from a callee that passes values to every signature input.
    fn reach(&self, root: impl Fn(&Var) -> bool) -> Vec<bool> {
        let mut incomplete = vec![false; self.vars.len()];
        let mut work = (0..self.vars.len() as u32)
            .filter(|var| root(&self.vars[*var as usize]))
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

    /// The ways values the constraints do not describe may reach each
    /// variable's set, given the precision of every set before openness is
    /// applied. A variable is open when its set is not empty.
    ///
    /// The roots are the variables read as a source that nothing defines (no
    /// seed, no copy into them, no slot, no call binding), which hold an
    /// outside value, and the results of calls whose callee set is
    /// unresolved, which hold an open result. Openness spreads along copies
    /// and dependencies, and from a store through an open base to every slot
    /// of its field, keeping its kinds.
    ///
    /// Code the constraints do not describe can also read values: an object
    /// escapes when it is passed to a call whose callees are unresolved,
    /// heuristic or open, passed where no parameter receives it, stored
    /// through an open base, or held by a slot of an escaped object or by the
    /// return of an escaped function. Such code may write every slot of an
    /// escaped object and call an escaped function with anything, so those
    /// slots hold an escaped object's field, the function's parameters and
    /// rest parameter an inward parameter and its `this` an inward `this`.
    /// Openness and escape feed each other until neither grows.
    fn open_variables(&self, precisions: &[PointsToPrecision]) -> Vec<InflowSet> {
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
        let outside = InflowSet::of(Inflow::OutsideValue);
        let mut work = work
            .into_iter()
            .map(|var| (var, outside))
            .collect::<Vec<_>>();
        work.extend(
            self.calls
                .iter()
                .filter(|call| self.callees_unresolved(call))
                .filter_map(|call| call.result)
                .map(|var| (var, InflowSet::of(Inflow::OpenResult))),
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
        let mut open = vec![InflowSet::default(); self.vars.len()];
        let mut escaped = vec![false; self.objects.len()];
        let mut objects = Vec::new();
        loop {
            for var in escaping.drain(..) {
                objects.extend(self.set(var as usize).iter());
            }
            if let Some(object) = objects.pop() {
                if std::mem::replace(&mut escaped[object as usize], true) {
                    continue;
                }
                let slots = &self.objects[object as usize].slots;
                let field = InflowSet::of(Inflow::EscapedObject);
                work.extend(slots.iter().map(|slot| (*slot, field)));
                escaping.extend(slots);
                if let Some(signature) = self.objects[object as usize].signature {
                    let signature = &self.signatures[signature as usize];
                    let parameter = InflowSet::of(Inflow::InwardParameter);
                    work.extend(
                        signature
                            .params
                            .iter()
                            .map(|(_, place)| place)
                            .chain(signature.rest.iter())
                            .map(|place| (place.var, parameter)),
                    );
                    work.extend(
                        signature
                            .this
                            .iter()
                            .map(|place| (place.var, InflowSet::of(Inflow::InwardThis))),
                    );
                    escaping.extend(signature.ret.iter().map(|place| place.var));
                }
                continue;
            }
            let Some((var, inflow)) = work.pop() else {
                break;
            };
            let added = open[var as usize].missing(inflow);
            if added.is_empty() {
                continue;
            }
            let opened = open[var as usize].is_empty();
            open[var as usize].insert_all(added);
            let data = &self.vars[var as usize];
            work.extend(
                data.copies
                    .iter()
                    .chain(&data.dependencies)
                    .map(|(dst, _)| (*dst, added)),
            );
            for store in &data.stores {
                work.extend(
                    self.slots_by_field[store.field as usize]
                        .iter()
                        .map(|slot| (*slot, added)),
                );
            }
            if opened {
                escaping.extend(data.stores.iter().map(|store| store.var));
                for index in &data.calls {
                    let call = &self.calls[*index as usize];
                    escaping.extend(call.receiver.iter().chain(&call.arguments).copied());
                }
            }
        }
        open
    }

    /// The variables some object reached through a heap slot: the slots, and
    /// every variable a copy reaches from one.
    fn through_slot_variables(&self) -> Vec<bool> {
        let mut reached = vec![false; self.vars.len()];
        let mut work = (0..self.vars.len() as u32)
            .filter(|var| self.vars[*var as usize].slot.is_some())
            .collect::<Vec<_>>();
        while let Some(var) = work.pop() {
            if std::mem::replace(&mut reached[var as usize], true) {
                continue;
            }
            work.extend(self.vars[var as usize].copies.iter().map(|(dst, _)| *dst));
        }
        reached
    }

    /// True when a call's callee set is empty or holds an object without a
    /// signature, so the call may run code the constraints do not describe.
    fn callees_unresolved(&self, call: &Call) -> bool {
        let callees = self.set(call.callee as usize);
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
        let callees = self.set(call.callee as usize);
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
                if self.set(dst).is_empty() {
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
            .filter(|(var, bindings)| !bindings.is_empty() && !self.set(*var).is_empty())
            .map(|(var, bindings)| (self.var_ids[var], bindings))
            .collect()
    }

    /// The weakest precision of the inputs that reached each set, carried
    /// along every edge from a non-empty set.
    fn precisions(&self) -> Vec<PointsToPrecision> {
        let mut precision = (0..self.vars.len())
            .map(|var| {
                self.vars[var]
                    .seed_precision
                    .filter(|_| !self.set(var).is_empty())
                    .unwrap_or(PointsToPrecision::FlowInsensitive)
            })
            .collect::<Vec<_>>();
        let mut work = (0..self.vars.len())
            .filter(|var| !self.set(*var).is_empty())
            .collect::<VecDeque<_>>();
        while let Some(var) = work.pop_front() {
            if self.set(var).is_empty() {
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

    fn variable_identity(&self, var: usize, objects: &HashMap<u32, String>) -> String {
        let parts = &self.vars[var].identity;
        assert!(
            !parts.is_empty(),
            "points-to set variable must originate in a constraint or object slot"
        );
        let fragments = parts
            .iter()
            .map(|part| match part {
                IdentityPart::Relation(key, role) => self.relation_fragment(*key, role),
                IdentityPart::Slot { object, field } => stable_key_text_from_parts(
                    FactFamily::PointsToSet,
                    &[
                        ("object", objects[object].as_str()),
                        ("slot", self.fields[*field as usize].as_str()),
                    ],
                ),
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
        let bounded = self.bounded_variables();
        let mut precisions = self.precisions();
        let open = self.open_variables(&precisions);
        let through_slot = self.through_slot_variables();
        let mut grounds = BTreeMap::new();
        for var in 0..self.vars.len() {
            if self.set(var).is_empty() {
                continue;
            }
            let inflow = open[var]
                .first()
                .filter(|_| precisions[var] <= PointsToPrecision::LocalFlowSensitive);
            if inflow.is_some() || through_slot[var] {
                grounds.insert(
                    self.var_ids[var],
                    SetGrounds {
                        inflow,
                        through_slot: through_slot[var],
                    },
                );
            }
            if !open[var].is_empty() {
                raise(&mut precisions[var], PointsToPrecision::Heuristic);
            }
        }
        let call_bindings = self.reached_bindings();
        let mut variables = (0..self.vars.len())
            .filter(|var| incomplete[*var] || bounded[*var] || !self.set(*var).is_empty())
            .collect::<Vec<_>>();
        variables.sort_by_key(|var| self.var_ids[*var]);
        // Identities are written in parallel and interned in variable order, so
        // the interner sees the same keys in the same order as a sequential pass.
        let object_identities = variables
            .iter()
            .flat_map(|var| &self.vars[*var].identity)
            .filter_map(|part| match part {
                IdentityPart::Slot { object, .. } => Some(*object),
                IdentityPart::Relation(..) => None,
            })
            .collect::<BTreeSet<_>>()
            .into_par_iter()
            .map(|object| (object, self.object_identity(object)))
            .collect::<HashMap<_, _>>();
        let budget_label = points_to_budget_status_label(budget_status);
        let keys = variables
            .par_iter()
            .map(|&var| {
                stable_key_text_from_parts(
                    FactFamily::PointsToSet,
                    &[
                        (
                            "variable",
                            self.variable_identity(var, &object_identities).as_str(),
                        ),
                        ("budget", budget_label),
                    ],
                )
            })
            .collect::<Vec<_>>();
        let sets = variables
            .into_iter()
            .zip(keys)
            .map(|(var, key)| PointsToSetFact {
                id: PointsToSetId(0),
                variable: self.var_ids[var],
                objects: self
                    .set(var)
                    .iter()
                    .map(|object| self.objects[object as usize].id)
                    .collect(),
                status: if incomplete[var] {
                    PointsToStatus::BudgetExceeded
                } else if bounded[var] {
                    PointsToStatus::IndirectionBound
                } else {
                    PointsToStatus::Present
                },
                precision: precisions[var],
                budget: budget_status,
                stable_key: self.interner.intern(key),
            })
            .collect();
        let set_sizes = (0..self.vars.len())
            .map(|var| self.set(var).len())
            .filter(|size| *size > 0);
        let stats = PointsToSolveStats {
            variables: self.vars.len(),
            objects: self.objects.len(),
            slots: self.slots.len(),
            copy_edges: self.vars.iter().map(|var| var.copies.len()).sum(),
            pops: self.steps,
            collapsed: (0..self.reps.len())
                .filter(|var| self.reps[*var] as usize != *var)
                .count(),
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
            grounds,
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

/// Adds the members of `objects` that `set` lacks to `held`. True when
/// `held` was empty and now holds some, so its variable is to be listed.
fn hold_missing(set: &ObjectSet, held: &mut Option<Vec<u32>>, objects: &[u32]) -> bool {
    match held {
        Some(held) => {
            set.missing(objects, held);
            false
        }
        None => {
            let mut missing = Vec::new();
            set.missing(objects, &mut missing);
            if missing.is_empty() {
                return false;
            }
            *held = Some(missing);
            true
        }
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

    #[test]
    fn a_copy_cycle_is_merged_and_carries_later_objects_along_every_edge() {
        let copy = |key: &str, src: u64, dst: u64| {
            constraint(
                key,
                PointsToConstraintKind::Copy {
                    dst: PtVarId(dst),
                    src: PtVarId(src),
                },
            )
        };
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
                    dst: PtVarId(5),
                    object: ObjectTokenId(11),
                },
            ),
            copy("copy-1-2", 1, 2),
            copy("copy-2-3", 2, 3),
            copy("copy-3-1", 3, 1),
            copy("copy-2-4", 2, 4),
            copy("copy-5-6", 5, 6),
            copy("copy-6-7", 6, 7),
            copy("copy-7-3", 7, 3),
            copy("copy-1-8", 1, 8),
            copy("copy-3-9", 3, 9),
        ];
        // Each copy out of the cycle reaches a variable that holds an object
        // of its own; merging it into the cycle would carry that object back.
        let constraints = constraints
            .into_iter()
            .chain([(4, 12), (8, 13), (9, 14)].map(|(var, object)| {
                constraint(
                    &format!("addr-{var}"),
                    PointsToConstraintKind::AddressOf {
                        dst: PtVarId(var),
                        object: ObjectTokenId(object),
                    },
                )
            }))
            .collect::<Vec<_>>();
        let result = solve_points_to(
            &crate::internal_core::test_stable_key_interner(),
            &constraints,
            PointsToBudget::default(),
        );

        assert_eq!(result.budget_status, PointsToBudgetStatus::WithinBudget);
        assert_eq!(result.stats.collapsed, 2);
        let objects = |var: u64| {
            result
                .sets
                .iter()
                .find(|set| set.variable == PtVarId(var))
                .map(|set| (set.status, set.objects.clone()))
        };
        for var in [1, 2, 3] {
            assert_eq!(
                objects(var),
                Some((
                    PointsToStatus::Present,
                    vec![ObjectTokenId(10), ObjectTokenId(11)]
                )),
                "variable {var}"
            );
        }
        for (var, own) in [(4, 12), (8, 13), (9, 14)] {
            assert_eq!(
                objects(var),
                Some((
                    PointsToStatus::Present,
                    vec![ObjectTokenId(10), ObjectTokenId(11), ObjectTokenId(own)]
                )),
                "variable {var}"
            );
        }
        assert_eq!(
            objects(6),
            Some((PointsToStatus::Present, vec![ObjectTokenId(11)]))
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
