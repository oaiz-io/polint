//! Hand-built constraint fixtures for the TS/JS points-to solve.
//!
//! A fixture describes a small program as named places, copies, field
//! accesses, allocations and calls, and translates it into the solver's inputs
//! without parsing. The tests pin the solve's contract:
//!
//! - a budget event keeps every set and edge derived before it and marks only
//!   what it did not finish (`research/package-graphs/DIAGNOSIS.md`, section 1);
//! - a call connects its arguments, result and receiver to the places of every
//!   function its callee may hold (DIAGNOSIS section 2);
//! - the solution depends on the constraint set alone, not on its order, and
//!   grows monotonically with it;
//! - a value fed by a heuristic input never yields an exact edge.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::analysis_neutral::ids::SemanticConstraintId;
use crate::analysis_neutral::points_to::facts::{PointsToBudgetStatus, PointsToSetFact};
use crate::analysis_neutral::solver::budget::BudgetReason;
use crate::internal_core::StableKeyInterner;

/// One call site in a fixture. `callee` is the place whose value is called.
#[derive(Default)]
struct Call<'a> {
    callee: &'a str,
    receiver: Option<&'a str>,
    arguments: Vec<&'a str>,
    result: Option<&'a str>,
}

struct Site {
    caller: SemanticNodeId,
    node: SemanticNodeId,
    constraint_key: StableKeyId,
}

#[derive(Clone)]
struct FixtureConstraint {
    key: String,
    kind: ConstraintKind,
    precision: PointsToPrecision,
}

struct Fixture {
    interner: StableKeyInterner,
    nodes: BTreeMap<String, SemanticNodeId>,
    names: BTreeMap<SemanticNodeId, String>,
    functions: BTreeSet<SemanticNodeId>,
    constraints: Vec<FixtureConstraint>,
    sites: BTreeMap<String, Site>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            interner: crate::internal_core::test_stable_key_interner(),
            nodes: BTreeMap::new(),
            names: BTreeMap::new(),
            functions: BTreeSet::new(),
            constraints: Vec::new(),
            sites: BTreeMap::new(),
        }
    }

    fn node(&mut self, name: &str) -> SemanticNodeId {
        if let Some(node) = self.nodes.get(name) {
            return *node;
        }
        let node = SemanticNodeId(self.nodes.len() as u64 + 1);
        self.nodes.insert(name.to_string(), node);
        self.names.insert(node, name.to_string());
        node
    }

    fn node_key(&self, node: SemanticNodeId) -> StableKeyId {
        self.interner
            .intern(format!("fixture:node:{}", self.names[&node]))
    }

    fn function(&mut self, name: &str) -> SemanticNodeId {
        let node = self.node(name);
        self.functions.insert(node);
        node
    }

    fn param(&mut self, function: &str, index: usize) -> String {
        let name = format!("{function}#param{index}");
        self.node(&name);
        name
    }

    fn this_of(&mut self, function: &str) -> String {
        let name = format!("{function}#this");
        self.node(&name);
        name
    }

    fn return_of(&mut self, function: &str) -> String {
        let name = format!("{function}#return");
        self.node(&name);
        name
    }

    fn push(&mut self, key: String, kind: ConstraintKind, precision: PointsToPrecision) {
        self.constraints.push(FixtureConstraint {
            key,
            kind,
            precision,
        });
    }

    fn copy(&mut self, dst: &str, src: &str) {
        self.copy_with(dst, src, PointsToPrecision::FlowInsensitive);
    }

    fn heuristic_copy(&mut self, dst: &str, src: &str) {
        self.copy_with(dst, src, PointsToPrecision::Heuristic);
    }

    fn copy_with(&mut self, dst: &str, src: &str, precision: PointsToPrecision) {
        let kind = ConstraintKind::CopyEdge {
            dst: self.node(dst),
            src: self.node(src),
        };
        self.push(format!("copy:{dst}<-{src}"), kind, precision);
    }

    fn alloc(&mut self, dst: &str, object: &str) {
        let kind = ConstraintKind::Alloc {
            dst: self.node(dst),
            object: self.node(object),
        };
        self.push(
            format!("alloc:{dst}<-{object}"),
            kind,
            PointsToPrecision::FlowInsensitive,
        );
    }

    fn store(&mut self, base: &str, field: &str, src: &str) {
        let kind = ConstraintKind::FieldStore {
            base: self.node(base),
            field: field.to_string(),
            src: self.node(src),
        };
        self.push(
            format!("store:{base}.{field}<-{src}"),
            kind,
            PointsToPrecision::FlowInsensitive,
        );
    }

    fn load(&mut self, dst: &str, base: &str, field: &str) {
        let kind = ConstraintKind::FieldLoad {
            dst: self.node(dst),
            base: self.node(base),
            field: field.to_string(),
        };
        self.push(
            format!("load:{dst}<-{base}.{field}"),
            kind,
            PointsToPrecision::FlowInsensitive,
        );
    }

    /// Adds a call site. Today's `CallConstraint` names only the site, so the
    /// callee reaches the site through a copy into the site's node; the
    /// receiver, arguments and result have no form the solver reads.
    fn call(&mut self, caller: &str, site: &str, call: Call<'_>) {
        let caller = self.node(caller);
        let node = self.node(site);
        let constraint_key = format!("call:{site}");
        self.push(
            constraint_key.clone(),
            ConstraintKind::CallConstraint { callsite: node },
            PointsToPrecision::FlowInsensitive,
        );
        self.copy(site, call.callee);
        for place in call
            .receiver
            .into_iter()
            .chain(call.arguments.iter().copied())
            .chain(call.result)
        {
            self.node(place);
        }
        let constraint_key = self.interner.intern(constraint_key);
        self.sites.insert(
            site.to_string(),
            Site {
                caller,
                node,
                constraint_key,
            },
        );
    }

    fn inputs(&self) -> TsPointsToInputs {
        self.inputs_from(&self.constraints)
    }

    fn inputs_from(&self, constraints: &[FixtureConstraint]) -> TsPointsToInputs {
        let ids = self
            .constraints
            .iter()
            .enumerate()
            .map(|(index, constraint)| (constraint.key.as_str(), index as u64))
            .collect::<BTreeMap<_, _>>();
        TsPointsToInputs {
            constraints: constraints
                .iter()
                .map(|constraint| ConstraintFact {
                    id: SemanticConstraintId(ids[constraint.key.as_str()]),
                    kind: constraint.kind.clone(),
                    status: PointsToStatus::Present,
                    precision: constraint.precision,
                    stable_key: self.interner.intern(constraint.key.as_str()),
                })
                .collect(),
            callable_objects: self
                .functions
                .iter()
                .map(|&node| (object_for_node(node), (node, self.node_key(node))))
                .collect(),
            callsites: self
                .sites
                .values()
                .map(|site| TsPointsToCallsite {
                    caller_node: site.caller,
                    callsite_node: site.node,
                    callsite_stable_key: self.node_key(site.node),
                    constraint_stable_key: site.constraint_key,
                    precision: PointsToPrecision::FlowInsensitive,
                })
                .collect(),
        }
    }

    fn solve(&self, budget: &SolverBudget) -> TsPointsToSolveResult {
        solve_ts_points_to(&self.interner, &self.inputs(), budget)
    }

    fn set<'r>(&self, result: &'r TsPointsToSolveResult, place: &str) -> &'r PointsToSetFact {
        let variable = var_for_node(self.nodes[place]);
        result
            .points_to
            .sets
            .iter()
            .find(|set| set.variable == variable)
            .unwrap_or_else(|| panic!("no points-to set for `{place}`"))
    }

    fn objects(&self, set: &PointsToSetFact) -> BTreeSet<String> {
        set.objects
            .iter()
            .map(|object| self.names[&SemanticNodeId(object.0)].clone())
            .collect()
    }

    /// The functions a site's derived edges reach, with each edge's status and
    /// precision.
    fn targets(
        &self,
        result: &TsPointsToSolveResult,
        site: &str,
    ) -> BTreeMap<String, (PointsToStatus, PointsToPrecision)> {
        let site_key = self.node_key(self.sites[site].node);
        result
            .derived_edges
            .iter()
            .filter(|edge| {
                edge.provenance
                    .contributing_facts
                    .iter()
                    .any(|fact| fact.stable_key == site_key)
            })
            .map(|edge| {
                (
                    self.names[&edge.target].clone(),
                    (edge.status, edge.precision),
                )
            })
            .collect()
    }
}

fn budget(edit: impl FnOnce(&mut SolverBudget)) -> SolverBudget {
    let mut budget = SolverBudget::default();
    edit(&mut budget);
    budget
}

/// A fixed-seed xorshift generator, so the permutations are the same on
/// every run.
struct Shuffle(u64);

impl Shuffle {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn permute<T>(&mut self, items: &mut [T]) {
        for index in (1..items.len()).rev() {
            let other = (self.next() % (index as u64 + 1)) as usize;
            items.swap(index, other);
        }
    }
}

const EXACT: (PointsToStatus, PointsToPrecision) =
    (PointsToStatus::Present, PointsToPrecision::FlowInsensitive);

/// `f` flows to `c` through `a` and `b`, is stored in `o.k` and called back
/// out of it; a second, longer path later gives `x` a second function. Under
/// a one-object ceiling the second object of `x` is a budget event that comes
/// after the short path has finished.
fn budget_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.function("main");
    fixture.function("f");
    fixture.function("g");
    fixture.copy("a", "f");
    fixture.copy("b", "a");
    fixture.copy("c", "b");
    fixture.alloc("o", "o:object");
    fixture.store("o", "k", "c");
    fixture.load("d", "o", "k");
    fixture.call(
        "main",
        "d()",
        Call {
            callee: "d",
            ..Call::default()
        },
    );
    fixture.copy("x", "a");
    fixture.copy("g1", "g");
    for step in 2..10 {
        fixture.copy(&format!("g{step}"), &format!("g{}", step - 1));
    }
    fixture.copy("x", "g9");
    fixture
}

/// DIAGNOSIS section 1: a budget event blanks every set, so no edge survives
/// it. The solve must keep what it derived before the event.
#[test]
fn budget_event_keeps_sets_and_edges_derived_before_it() {
    let fixture = budget_fixture();
    let unbounded = fixture.solve(&SolverBudget::default());
    assert_eq!(
        budget_status(&unbounded.points_to),
        BudgetStatus::WithinBudget
    );
    assert_eq!(
        fixture.objects(fixture.set(&unbounded, "x")),
        BTreeSet::from(["f".to_string(), "g".to_string()])
    );

    let result = fixture.solve(&budget(|budget| budget.points_to.max_objects_per_var = 1));
    assert_eq!(
        result.points_to.budget_status,
        PointsToBudgetStatus::BudgetExceeded
    );
    assert_eq!(
        result.points_to.budget_reasons,
        BTreeSet::from([BudgetReason::PointsToMaxObjectsPerVar.as_str().to_string()])
    );
    for place in ["a", "b", "c", "d", "d()"] {
        let set = fixture.set(&result, place);
        assert_eq!(set.status, PointsToStatus::Present, "`{place}` is complete");
        assert_eq!(
            fixture.objects(set),
            BTreeSet::from(["f".to_string()]),
            "`{place}` keeps what it held before the event"
        );
    }
    assert_eq!(
        fixture.set(&result, "x").status,
        PointsToStatus::BudgetExceeded,
        "the set whose growth was stopped is the one marked incomplete"
    );
    assert_eq!(
        fixture.targets(&result, "d()"),
        BTreeMap::from([("f".to_string(), EXACT)]),
        "the edge derived before the event survives it"
    );
}

/// The degenerate budget stays honest: no step, no set, no edge, and the
/// run says why.
#[test]
fn zero_step_budget_derives_nothing_and_says_so() {
    let fixture = budget_fixture();
    let result = fixture.solve(&budget(|budget| budget.max_steps = 0));
    assert_eq!(
        result.points_to.budget_status,
        PointsToBudgetStatus::BudgetExceeded
    );
    assert_eq!(
        result.points_to.budget_reasons,
        BTreeSet::from([BudgetReason::SolverMaxSteps.as_str().to_string()])
    );
    assert!(
        result
            .points_to
            .sets
            .iter()
            .all(|set| set.objects.is_empty()),
        "no object is placed without a step"
    );
    assert!(result.derived_edges.is_empty());
}

/// `function g(p, q) { q() }`, called as `g(h1, h2)`.
fn parameter_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.function("main");
    fixture.function("g");
    fixture.function("h1");
    fixture.function("h2");
    fixture.param("g", 0);
    let q = fixture.param("g", 1);
    fixture.call(
        "g",
        "q()",
        Call {
            callee: &q,
            ..Call::default()
        },
    );
    fixture.call(
        "main",
        "g(h1, h2)",
        Call {
            callee: "g",
            arguments: vec!["h1", "h2"],
            ..Call::default()
        },
    );
    fixture
}

/// DIAGNOSIS section 2: an argument never reaches the callee's parameter, so
/// a call of a parameter has no target.
#[test]
fn call_binds_each_argument_to_its_parameter() {
    let fixture = parameter_fixture();
    let result = fixture.solve(&SolverBudget::default());
    assert_eq!(
        fixture.targets(&result, "g(h1, h2)"),
        BTreeMap::from([("g".to_string(), EXACT)])
    );
    assert_eq!(
        fixture.targets(&result, "q()"),
        BTreeMap::from([("h2".to_string(), EXACT)]),
        "the second argument reaches the second parameter and only it"
    );
}

/// `function m() { return h }`, called as `m()()`.
fn return_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.function("main");
    fixture.function("m");
    fixture.function("h");
    let returned = fixture.return_of("m");
    fixture.copy(&returned, "h");
    fixture.call(
        "main",
        "m()",
        Call {
            callee: "m",
            result: Some("m():result"),
            ..Call::default()
        },
    );
    fixture.call(
        "main",
        "m()()",
        Call {
            callee: "m():result",
            ..Call::default()
        },
    );
    fixture
}

/// DIAGNOSIS section 2: a callee's return value never reaches the call's
/// result, so calling what a call returns has no target.
#[test]
fn call_result_holds_what_the_callee_returns() {
    let fixture = return_fixture();
    let result = fixture.solve(&SolverBudget::default());
    assert_eq!(
        fixture.targets(&result, "m()"),
        BTreeMap::from([("m".to_string(), EXACT)])
    );
    assert_eq!(
        fixture.targets(&result, "m()()"),
        BTreeMap::from([("h".to_string(), EXACT)])
    );
}

/// `o = { k, j }` with `function k() { this.j() }`, called as `o.k()`.
fn receiver_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.function("main");
    fixture.function("k");
    fixture.function("j");
    fixture.alloc("o", "o:object");
    fixture.store("o", "k", "k");
    fixture.store("o", "j", "j");
    fixture.load("o.k", "o", "k");
    fixture.call(
        "main",
        "o.k()",
        Call {
            callee: "o.k",
            receiver: Some("o"),
            ..Call::default()
        },
    );
    let this = fixture.this_of("k");
    fixture.load("this.j", &this, "j");
    fixture.call(
        "k",
        "this.j()",
        Call {
            callee: "this.j",
            ..Call::default()
        },
    );
    fixture
}

/// DIAGNOSIS section 2: the receiver of a method call never reaches the
/// method's `this`, so a call through `this` has no target.
#[test]
fn call_receiver_becomes_the_callee_this() {
    let fixture = receiver_fixture();
    let result = fixture.solve(&SolverBudget::default());
    assert_eq!(
        fixture.targets(&result, "o.k()"),
        BTreeMap::from([("k".to_string(), EXACT)])
    );
    assert_eq!(
        fixture.targets(&result, "this.j()"),
        BTreeMap::from([("j".to_string(), EXACT)])
    );
}

/// `t` holds `h` only through a heuristic input, and `u` holds `h` exactly.
fn heuristic_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.function("main");
    fixture.function("h");
    fixture.function("h2");
    fixture.heuristic_copy("t", "h");
    fixture.copy("t2", "t");
    fixture.call(
        "main",
        "t2()",
        Call {
            callee: "t2",
            ..Call::default()
        },
    );
    fixture.copy("u", "h");
    fixture.call(
        "main",
        "u()",
        Call {
            callee: "u",
            ..Call::default()
        },
    );
    fixture.copy("v", "h2");
    fixture.heuristic_copy("v", "h");
    fixture.call(
        "main",
        "v()",
        Call {
            callee: "v",
            ..Call::default()
        },
    );
    fixture
}

/// A heuristic input reaching a callee set makes every edge read from that
/// set heuristic, so it never becomes an exact claim.
#[test]
fn heuristic_input_makes_every_edge_it_reaches_heuristic() {
    let fixture = heuristic_fixture();
    let result = fixture.solve(&SolverBudget::default());
    let heuristic = (PointsToStatus::Present, PointsToPrecision::Heuristic);
    assert_eq!(
        fixture.targets(&result, "t2()"),
        BTreeMap::from([("h".to_string(), heuristic)])
    );
    assert_eq!(
        fixture.targets(&result, "u()"),
        BTreeMap::from([("h".to_string(), EXACT)]),
        "an exact path stays exact"
    );
    assert_eq!(
        fixture.targets(&result, "v()"),
        BTreeMap::from([("h".to_string(), heuristic), ("h2".to_string(), heuristic)]),
        "a set fed by a heuristic input is heuristic as a whole"
    );
}

/// Copies and calls only: no field slot, no budget event.
fn copy_only_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.function("main");
    for index in 0..6 {
        fixture.function(&format!("f{index}"));
    }
    for index in 0..6 {
        fixture.copy(&format!("a{index}"), &format!("f{index}"));
        fixture.copy(&format!("b{}", index % 3), &format!("a{index}"));
    }
    for index in 0..3 {
        fixture.call(
            "main",
            &format!("b{index}()"),
            Call {
                callee: &format!("b{index}"),
                ..Call::default()
            },
        );
    }
    fixture
}

/// Solves the fixture's projected constraints under 50 permutations and
/// asserts each solve equals the solve in the original order, slot variable
/// ids included.
fn assert_order_independent(fixture: &Fixture, budget: &SolverBudget) {
    let inputs = fixture.inputs();
    let constraints = project_constraints(&fixture.interner, &inputs);
    let expected = solve_points_to(&fixture.interner, &constraints, budget.points_to_budget());
    let expected_edges = fixture.solve(budget).derived_edges;
    let mut shuffle = Shuffle(0x9e37_79b9_7f4a_7c15);
    for round in 0..50 {
        let mut permuted = constraints.clone();
        shuffle.permute(&mut permuted);
        let solved = solve_points_to(&fixture.interner, &permuted, budget.points_to_budget());
        assert_eq!(solved, expected, "permutation {round} changed the solve");

        let mut source = fixture.constraints.clone();
        shuffle.permute(&mut source);
        let edges = solve_ts_points_to(&fixture.interner, &fixture.inputs_from(&source), budget)
            .derived_edges;
        assert_eq!(
            edges, expected_edges,
            "permutation {round} changed the edges"
        );
    }
}

#[test]
fn order_independence_copy_only() {
    assert_order_independent(&copy_only_fixture(), &SolverBudget::default());
}

#[test]
fn order_independence_with_fields_calls_and_budget() {
    let tight = budget(|budget| budget.points_to.max_objects_per_var = 1);
    assert_order_independent(&budget_fixture(), &SolverBudget::default());
    assert_order_independent(&budget_fixture(), &tight);
    assert_order_independent(&parameter_fixture(), &SolverBudget::default());
    assert_order_independent(&return_fixture(), &SolverBudget::default());
    assert_order_independent(&receiver_fixture(), &SolverBudget::default());
    assert_order_independent(&heuristic_fixture(), &SolverBudget::default());
    let mut fields = Fixture::new();
    synthetic_package(&mut fields, 0, 4);
    assert_order_independent(&fields, &SolverBudget::default());
}

/// A synthetic package: `objects` objects with five function-valued fields
/// each, every field read back and passed down a chain of six copies to a
/// call site.
fn synthetic_package(fixture: &mut Fixture, first_object: usize, objects: usize) {
    const FUNCTIONS: usize = 200;
    const FIELDS: usize = 5;
    const CHAIN: usize = 6;
    for function in 0..FUNCTIONS {
        fixture.function(&format!("fn{function}"));
    }
    for object in first_object..first_object + objects {
        let base = format!("p{object}");
        fixture.alloc(&base, &format!("obj{object}"));
        for field in 0..FIELDS {
            let slot = object * FIELDS + field;
            let name = format!("f{field}");
            fixture.store(&base, &name, &format!("fn{}", slot % FUNCTIONS));
            fixture.load(&format!("l{slot}"), &base, &name);
            fixture.copy(&format!("c{slot}_0"), &format!("l{slot}"));
            for link in 1..CHAIN {
                fixture.copy(&format!("c{slot}_{link}"), &format!("c{slot}_{}", link - 1));
            }
            fixture.call(
                &format!("fn{}", object % FUNCTIONS),
                &format!("site{slot}"),
                Call {
                    callee: &format!("c{slot}_{}", CHAIN - 1),
                    ..Call::default()
                },
            );
        }
    }
}

/// A package-sized constraint set needs more field slots and steps than the
/// budget allowed, so every package of that size lost its whole solve
/// (DIAGNOSIS section 1). Under the default budget it must solve in full.
#[test]
fn package_sized_set_solves_within_the_default_budget() {
    let mut fixture = Fixture::new();
    synthetic_package(&mut fixture, 0, 600);
    for cross in 0..2_000 {
        let slot = (cross * 7) % 3_000;
        fixture.copy(&format!("x{cross}"), &format!("c{slot}_5"));
    }
    let copies = fixture
        .constraints
        .iter()
        .filter(|constraint| matches!(constraint.kind, ConstraintKind::CopyEdge { .. }))
        .count();
    assert!(copies >= 20_000, "{copies} copies");

    let result = fixture.solve(&SolverBudget::default());
    assert_eq!(
        result.points_to.budget_status,
        PointsToBudgetStatus::WithinBudget,
        "{:?}",
        result.points_to.budget_reasons
    );
    assert_eq!(result.derived_edges.len(), 3_000);
    assert!(
        result
            .derived_edges
            .iter()
            .all(|edge| (edge.status, edge.precision) == EXACT)
    );

    let mut shuffle = Shuffle(0x2545_f491_4f6c_dd1d);
    for _ in 0..3 {
        let mut source = fixture.constraints.clone();
        shuffle.permute(&mut source);
        let permuted = solve_ts_points_to(
            &fixture.interner,
            &fixture.inputs_from(&source),
            &SolverBudget::default(),
        );
        assert_eq!(permuted.derived_edges, result.derived_edges);
    }
}

/// Adding constraints never removes a set's object or an edge: the property
/// stitching package graphs relies on. The larger set needs more field slots
/// than the old budget allowed, so a latch fails this test.
#[test]
fn superset_of_constraints_solves_to_a_superset() {
    let mut fixture = Fixture::new();
    synthetic_package(&mut fixture, 0, 96);
    let smaller = fixture.solve(&SolverBudget::default());
    assert_eq!(
        smaller.points_to.budget_status,
        PointsToBudgetStatus::WithinBudget
    );

    synthetic_package(&mut fixture, 96, 10);
    for slot in (0..480).step_by(10) {
        fixture.copy(&format!("c{slot}_0"), "fn199");
    }
    let larger = fixture.solve(&SolverBudget::default());
    assert_eq!(
        larger.points_to.budget_status,
        PointsToBudgetStatus::WithinBudget,
        "{:?}",
        larger.points_to.budget_reasons
    );

    let larger_sets = larger
        .points_to
        .sets
        .iter()
        .map(|set| (set.variable, set))
        .collect::<BTreeMap<_, _>>();
    for set in &smaller.points_to.sets {
        let grown = larger_sets
            .get(&set.variable)
            .unwrap_or_else(|| panic!("set {:?} is missing", set.variable));
        assert_eq!(grown.status, PointsToStatus::Present);
        assert!(
            set.objects
                .iter()
                .all(|object| grown.objects.contains(object)),
            "set {:?} lost an object",
            set.variable
        );
    }
    let larger_edges = larger
        .derived_edges
        .iter()
        .map(|edge| edge.stable_key)
        .collect::<BTreeSet<_>>();
    assert!(
        smaller
            .derived_edges
            .iter()
            .all(|edge| larger_edges.contains(&edge.stable_key))
    );
    assert!(larger.derived_edges.len() > smaller.derived_edges.len());
}
