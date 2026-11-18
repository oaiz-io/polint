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
    spread_from: Option<usize>,
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
    signatures: BTreeMap<ObjectTokenId, PointsToSignature>,
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
            signatures: BTreeMap::new(),
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
        let place = self.signature_place(&name);
        self.signature(function).params.insert(index, place);
        name
    }

    fn this_of(&mut self, function: &str) -> String {
        let name = format!("{function}#this");
        let place = self.signature_place(&name);
        self.signature(function).this = Some(place);
        name
    }

    fn return_of(&mut self, function: &str) -> String {
        let name = format!("{function}#return");
        let place = self.signature_place(&name);
        self.signature(function).ret = Some(place);
        name
    }

    fn signature_place(&mut self, name: &str) -> SignaturePlace {
        let node = self.node(name);
        SignaturePlace {
            var: var_for_node(node),
            stable_key: self.node_key(node),
        }
    }

    fn signature(&mut self, function: &str) -> &mut PointsToSignature {
        let object = object_for_node(self.node(function));
        self.signatures.entry(object).or_default()
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

    /// Adds a call site: its callee, receiver, arguments and result.
    fn call(&mut self, caller: &str, site: &str, call: Call<'_>) {
        let caller = self.node(caller);
        let node = self.node(site);
        let constraint_key = format!("call:{site}");
        let kind = ConstraintKind::CallConstraint {
            callsite: node,
            callee: Some(self.node(call.callee)),
            receiver: call.receiver.map(|place| self.node(place)),
            arguments: call
                .arguments
                .iter()
                .map(|place| self.node(place))
                .collect(),
            spread_from: call.spread_from,
            result: call.result.map(|place| self.node(place)),
        };
        self.push(
            constraint_key.clone(),
            kind,
            PointsToPrecision::FlowInsensitive,
        );
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
            signatures: self.signatures.clone(),
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

/// A call through a set the budget left incomplete may reach a function the
/// solve never saw in it, so every parameter it could fill is incomplete, the
/// one it did bind included.
#[test]
fn call_through_an_incomplete_callee_leaves_parameters_incomplete() {
    let mut fixture = budget_fixture();
    fixture.function("h");
    for function in ["f", "g"] {
        let param = fixture.param(function, 0);
        fixture.call(
            function,
            &format!("{param}()"),
            Call {
                callee: &param,
                ..Call::default()
            },
        );
    }
    fixture.call(
        "main",
        "x(h)",
        Call {
            callee: "x",
            arguments: vec!["h"],
            ..Call::default()
        },
    );
    let result = fixture.solve(&budget(|budget| budget.points_to.max_objects_per_var = 1));
    for place in ["f#param0", "g#param0"] {
        assert_eq!(
            fixture.set(&result, place).status,
            PointsToStatus::BudgetExceeded,
            "`{place}` may receive `h` through the incomplete callee set"
        );
    }
}

/// A call through a set the budget left incomplete keeps the edges it has,
/// marked incomplete, and is counted as a site left incomplete.
#[test]
fn call_through_an_incomplete_set_keeps_its_edges_and_is_counted() {
    let mut fixture = budget_fixture();
    fixture.call(
        "main",
        "x()",
        Call {
            callee: "x",
            ..Call::default()
        },
    );
    let unbounded = fixture.solve(&SolverBudget::default());
    assert_eq!(unbounded.incomplete_sites, 0);

    let result = fixture.solve(&budget(|budget| budget.points_to.max_objects_per_var = 1));
    assert_eq!(
        fixture.targets(&result, "x()"),
        BTreeMap::from([(
            "f".to_string(),
            (
                PointsToStatus::BudgetExceeded,
                PointsToPrecision::FlowInsensitive
            )
        )]),
        "the edge derived before the event is kept and marked incomplete"
    );
    assert_eq!(
        fixture.targets(&result, "d()"),
        BTreeMap::from([("f".to_string(), EXACT)])
    );
    assert_eq!(result.incomplete_sites, 1, "only `x()` is left incomplete");
}

/// The degenerate budget stays honest: no step, no set, no edge, and the
/// run says why.
#[test]
fn zero_step_budget_derives_nothing_and_says_so() {
    let fixture = budget_fixture();
    let result = fixture.solve(&budget(|budget| budget.points_to.max_steps = 0));
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
    assert_eq!(
        result.incomplete_sites, 1,
        "the one call site is incomplete"
    );
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
    let site_key = fixture.node_key(fixture.sites["q()"].node);
    let edge = result
        .derived_edges
        .iter()
        .find(|edge| {
            edge.provenance
                .contributing_facts
                .iter()
                .any(|fact| fact.stable_key == site_key)
        })
        .expect("q() has an edge");
    let facts = edge
        .provenance
        .contributing_facts
        .iter()
        .map(|fact| fixture.interner.resolve(fact.stable_key).to_string())
        .collect::<BTreeSet<_>>();
    assert!(
        facts.contains("call:g(h1, h2)") && facts.contains("fixture:node:g#param1"),
        "the edge names the call and the parameter it bound: {facts:?}"
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

/// `function g(p) { p() }`, called as `g(h1)` and as `g(...xs)`.
fn spread_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.function("main");
    fixture.function("g");
    fixture.function("h1");
    fixture.function("h2");
    let p = fixture.param("g", 0);
    fixture.call(
        "g",
        "p()",
        Call {
            callee: &p,
            ..Call::default()
        },
    );
    fixture.call(
        "main",
        "g(h1)",
        Call {
            callee: "g",
            arguments: vec!["h1"],
            ..Call::default()
        },
    );
    fixture.alloc("xs", "xs:array");
    fixture.store("xs", "0", "h2");
    fixture.call(
        "main",
        "g(...xs)",
        Call {
            callee: "g",
            arguments: vec!["xs"],
            spread_from: Some(0),
            ..Call::default()
        },
    );
    fixture
}

/// A spread argument fills parameters the solve does not bind, so those
/// parameters may hold values it never saw: what they do hold is heuristic.
#[test]
fn spread_argument_makes_the_parameters_it_fills_heuristic() {
    let fixture = spread_fixture();
    let result = fixture.solve(&SolverBudget::default());
    assert_eq!(
        fixture.targets(&result, "p()"),
        BTreeMap::from([(
            "h1".to_string(),
            (PointsToStatus::Present, PointsToPrecision::Heuristic)
        )]),
        "the spread array is not bound to `p`, and `p` is not exact"
    );
}

/// `function g(p) { p() }` reached only through a heuristic callee set.
fn heuristic_callee_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.function("main");
    fixture.function("g");
    fixture.function("h");
    let p = fixture.param("g", 0);
    fixture.call(
        "g",
        "p()",
        Call {
            callee: &p,
            ..Call::default()
        },
    );
    fixture.heuristic_copy("f", "g");
    fixture.call(
        "main",
        "f(h)",
        Call {
            callee: "f",
            arguments: vec!["h"],
            ..Call::default()
        },
    );
    fixture
}

/// A call whose callee set is heuristic binds heuristically: the parameter
/// it fills, and every edge read from it, are heuristic.
#[test]
fn heuristic_callee_binds_heuristically() {
    let fixture = heuristic_callee_fixture();
    let result = fixture.solve(&SolverBudget::default());
    let heuristic = (PointsToStatus::Present, PointsToPrecision::Heuristic);
    assert_eq!(
        fixture.targets(&result, "f(h)"),
        BTreeMap::from([("g".to_string(), heuristic)])
    );
    assert_eq!(
        fixture.targets(&result, "p()"),
        BTreeMap::from([("h".to_string(), heuristic)])
    );
}

/// Places whose value the constraints do not describe, each beside a
/// function: `w` has no input at all, `p` is a parameter of `g`, which nothing
/// calls, and `r` is the result of calling `w`.
///
/// - `o.f = h; o.f = w; o.f()`
/// - `v = h; v = w; v()`
/// - `function g(p) { q = h; q = p; q() }`
/// - `r = w(); s = h; s = r; s()`
/// - `u = h; u()`
fn open_input_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.function("main");
    fixture.function("g");
    fixture.function("h");
    fixture.alloc("o", "o:object");
    fixture.store("o", "f", "h");
    fixture.store("o", "f", "w");
    fixture.load("o.f", "o", "f");
    fixture.call(
        "main",
        "o.f()",
        Call {
            callee: "o.f",
            receiver: Some("o"),
            ..Call::default()
        },
    );
    fixture.copy("v", "h");
    fixture.copy("v", "w");
    fixture.call(
        "main",
        "v()",
        Call {
            callee: "v",
            ..Call::default()
        },
    );
    let p = fixture.param("g", 0);
    fixture.copy("q", "h");
    fixture.copy("q", &p);
    fixture.call(
        "g",
        "q()",
        Call {
            callee: "q",
            ..Call::default()
        },
    );
    fixture.call(
        "main",
        "w()",
        Call {
            callee: "w",
            result: Some("r"),
            ..Call::default()
        },
    );
    fixture.copy("s", "h");
    fixture.copy("s", "r");
    fixture.call(
        "main",
        "s()",
        Call {
            callee: "s",
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
    fixture
}

/// A set that also receives a value the constraints do not describe may hold
/// more than the solve found, so an edge read from it is never exact. The
/// target stays, below the exact tier.
#[test]
fn input_the_solve_cannot_see_keeps_edges_out_of_the_exact_tier() {
    let fixture = open_input_fixture();
    let result = fixture.solve(&SolverBudget::default());
    let heuristic = (PointsToStatus::Present, PointsToPrecision::Heuristic);
    for site in ["o.f()", "v()", "q()", "s()"] {
        assert_eq!(
            fixture.targets(&result, site),
            BTreeMap::from([("h".to_string(), heuristic)]),
            "`{site}` reads a set with an input the solve cannot see"
        );
    }
    assert_eq!(
        fixture.targets(&result, "w()"),
        BTreeMap::new(),
        "an unresolved callee has no target"
    );
    assert_eq!(
        fixture.targets(&result, "u()"),
        BTreeMap::from([("h".to_string(), EXACT)]),
        "a set whose every input is described stays exact"
    );
}

/// Functions and objects handed to code the solve cannot see, beside one that
/// stays inside the program. `w` has no input at all, so a call of it may run
/// anything.
///
/// - `function g(p) { p() }; g(h1); w(h2)`: `g` never leaves the program.
/// - `function cb(c) { c() }; cb(h); w(cb)`: `cb` is passed out.
/// - `o.f = h; w(o); o.f()`: `o` is passed out.
/// - `function slot_cb(a) { a() }; o2.k = slot_cb; w(o2); slot_cb(h)`: held
///   by an object that is passed out.
/// - `function make() { return inner }; function inner(b) { b() }; w(make);
///   inner(h)`: returned by a function that is passed out.
/// - `function one(x) {}; one(h, extra_cb); function extra_cb(e) { e() };
///   extra_cb(h)`: passed where no parameter receives it.
/// - `function sink(s) { s.f = store_cb }; function store_cb(t) { t() };
///   store_cb(h)`: stored into an object nothing describes.
/// - `function heur_cb(d) { d() }; heur_cb(h); f = heuristic g2; f(heur_cb)`:
///   passed to a callee set read from a heuristic input.
fn escape_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    for function in [
        "main", "g", "h", "h1", "h2", "cb", "slot_cb", "make", "inner", "one", "extra_cb", "sink",
        "store_cb", "heur_cb", "g2",
    ] {
        fixture.function(function);
    }
    let call = |fixture: &mut Fixture, caller: &str, site: &str, callee: &str, arguments| {
        fixture.call(
            caller,
            site,
            Call {
                callee,
                arguments,
                ..Call::default()
            },
        );
    };
    let p = fixture.param("g", 0);
    call(&mut fixture, "g", "p()", &p, vec![]);
    call(&mut fixture, "main", "g(h1)", "g", vec!["h1"]);
    call(&mut fixture, "main", "w(h2)", "w", vec!["h2"]);

    let c = fixture.param("cb", 0);
    call(&mut fixture, "cb", "c()", &c, vec![]);
    call(&mut fixture, "main", "cb(h)", "cb", vec!["h"]);
    call(&mut fixture, "main", "w(cb)", "w", vec!["cb"]);

    fixture.alloc("o", "o:object");
    fixture.store("o", "f", "h");
    call(&mut fixture, "main", "w(o)", "w", vec!["o"]);
    fixture.load("o.f", "o", "f");
    call(&mut fixture, "main", "o.f()", "o.f", vec![]);

    let a = fixture.param("slot_cb", 0);
    call(&mut fixture, "slot_cb", "a()", &a, vec![]);
    fixture.alloc("o2", "o2:object");
    fixture.store("o2", "k", "slot_cb");
    call(&mut fixture, "main", "w(o2)", "w", vec!["o2"]);
    call(&mut fixture, "main", "slot_cb(h)", "slot_cb", vec!["h"]);

    let make_return = fixture.return_of("make");
    fixture.copy(&make_return, "inner");
    let b = fixture.param("inner", 0);
    call(&mut fixture, "inner", "b()", &b, vec![]);
    call(&mut fixture, "main", "w(make)", "w", vec!["make"]);
    call(&mut fixture, "main", "inner(h)", "inner", vec!["h"]);

    fixture.param("one", 0);
    call(
        &mut fixture,
        "main",
        "one(h, extra_cb)",
        "one",
        vec!["h", "extra_cb"],
    );
    let e = fixture.param("extra_cb", 0);
    call(&mut fixture, "extra_cb", "e()", &e, vec![]);
    call(&mut fixture, "main", "extra_cb(h)", "extra_cb", vec!["h"]);

    let s = fixture.param("sink", 0);
    fixture.store(&s, "f", "store_cb");
    let t = fixture.param("store_cb", 0);
    call(&mut fixture, "store_cb", "t()", &t, vec![]);
    call(&mut fixture, "main", "store_cb(h)", "store_cb", vec!["h"]);

    let d = fixture.param("heur_cb", 0);
    call(&mut fixture, "heur_cb", "d()", &d, vec![]);
    call(&mut fixture, "main", "heur_cb(h)", "heur_cb", vec!["h"]);
    fixture.param("g2", 0);
    fixture.heuristic_copy("f", "g2");
    call(&mut fixture, "main", "f(heur_cb)", "f", vec!["heur_cb"]);
    fixture
}

/// A parameter is open only when its function can be called by code the
/// solve cannot see: a function that escapes, through an argument, a slot of
/// an escaped object, the return of an escaped function, an argument no
/// parameter receives, or a store into an object nothing describes. A call of
/// an unresolved callee elsewhere does not open every parameter.
#[test]
fn only_an_escaped_function_has_open_parameters() {
    let fixture = escape_fixture();
    let result = fixture.solve(&SolverBudget::default());
    let heuristic = (PointsToStatus::Present, PointsToPrecision::Heuristic);
    assert_eq!(
        fixture.targets(&result, "p()"),
        BTreeMap::from([("h1".to_string(), EXACT)]),
        "`g` never leaves the program, so its parameter holds only `h1`"
    );
    for site in ["c()", "o.f()", "a()", "b()", "e()", "t()", "d()"] {
        assert_eq!(
            fixture.targets(&result, site),
            BTreeMap::from([("h".to_string(), heuristic)]),
            "`{site}` reads a place that code the solve cannot see may fill"
        );
    }
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
    let calls = project_calls(&inputs);
    let solve = |constraints: &[PointsToConstraintFact], calls: &PointsToCallInputs| {
        solve_points_to_with_calls(
            &fixture.interner,
            constraints,
            calls,
            budget.points_to_budget(),
        )
    };
    let expected = solve(&constraints, &calls);
    let expected_edges = fixture.solve(budget).derived_edges;
    let mut shuffle = Shuffle(0x9e37_79b9_7f4a_7c15);
    for round in 0..50 {
        let mut permuted = constraints.clone();
        shuffle.permute(&mut permuted);
        let mut permuted_calls = calls.clone();
        shuffle.permute(&mut permuted_calls.calls);
        let solved = solve(&permuted, &permuted_calls);
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
    assert_order_independent(&spread_fixture(), &SolverBudget::default());
    assert_order_independent(&heuristic_callee_fixture(), &SolverBudget::default());
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

/// The variable of the heap slot that holds `function`: the one set with that
/// object whose variable is no fixture place.
fn slot_holding(fixture: &Fixture, result: &TsPointsToSolveResult, function: &str) -> PtVarId {
    let places = fixture
        .nodes
        .values()
        .map(|node| var_for_node(*node))
        .collect::<BTreeSet<_>>();
    let object = object_for_node(fixture.nodes[function]);
    let slots = result
        .points_to
        .sets
        .iter()
        .filter(|set| !places.contains(&set.variable) && set.objects.contains(&object))
        .map(|set| set.variable)
        .collect::<Vec<_>>();
    assert_eq!(slots.len(), 1, "slots holding `{function}`: {slots:?}");
    slots[0]
}

/// A heap slot's variable depends on its object and field alone, so a package
/// graph's slot keeps its id when the graph is solved together with more
/// slots, including slots the solve reaches first.
#[test]
fn a_slot_keeps_its_variable_when_other_slots_are_added() {
    let alone = {
        let mut fixture = Fixture::new();
        fixture.function("fnf");
        fixture.alloc("p", "obj");
        fixture.store("p", "f", "fnf");
        fixture
    };
    let with_others = {
        let mut fixture = Fixture::new();
        fixture.function("fnf");
        fixture.node("p");
        fixture.node("obj");
        fixture.function("fna");
        fixture.store("p", "a", "fna");
        fixture.store("p", "e", "fna");
        fixture.alloc("p", "obj");
        fixture.store("p", "f", "fnf");
        fixture.store("p", "z", "fna");
        fixture.alloc("q", "objq");
        fixture.store("q", "a", "fna");
        fixture
    };
    let budget = SolverBudget::default();
    let alone_result = alone.solve(&budget);
    let others_result = with_others.solve(&budget);
    assert_eq!(
        slot_holding(&alone, &alone_result, "fnf"),
        slot_holding(&with_others, &others_result, "fnf"),
        "p.f changed its variable when other slots were added"
    );
}
