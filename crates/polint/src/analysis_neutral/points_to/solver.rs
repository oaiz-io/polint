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
//! never yields an exact set.

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
}

pub fn solve_points_to(
    interner: &crate::internal_core::StableKeyInterner,
    constraints: &[PointsToConstraintFact],
    budget: PointsToBudget,
) -> PointsToSolveResult {
    let mut solver = Solver::new(interner, budget);
    solver.initialize(constraints);
    solver.run();
    solver.into_result()
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
    /// that field is incomplete too.
    fn incomplete_variables(&self) -> BTreeSet<PtVarId> {
        let mut incomplete = BTreeSet::new();
        let mut work = self
            .pending
            .keys()
            .chain(self.interrupted.iter())
            .copied()
            .collect::<Vec<_>>();
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
        }
        incomplete
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
        let precisions = self.precisions();
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
        PointsToSolveResult {
            sets,
            budget_status,
            budget_reasons: self.budget_reasons,
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
    let solve = solve_points_to(interner, &constraints, budget);
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
