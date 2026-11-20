//! Call sites whose run-time target the syntax alone does not decide must not
//! claim an exact target.
//!
//! Each case is a small program checked in Node: the comment of each test
//! says what Node prints. The three answers a rule or a consumer can read
//! are checked together: the whole-program solve, the stored package graph
//! and the edges the call graph serves.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::analysis_kernel::{AnalysisKernel, KernelInput, KernelOutput};
use crate::analysis_neutral::calls::facts::ClaimTier;
use crate::analysis_neutral::points_to::tiers::EdgeClaim;
use crate::analysis_neutral::semantic_graph::facts::NodeKind;
use crate::analysis_neutral::solver::budget::SolverBudget;
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::load_config;
use crate::ts::points_to::package_graph::export;
use crate::ts::points_to::{TsPointsToInputs, site_answers, solve_ts_points_to};

const REFINED: &[&str] = &[
    "syntax",
    "calls",
    "control_flow",
    "dataflow",
    "call_graph",
    "symbols",
    "references",
];

fn run(root: &Path) -> KernelOutput {
    let mut loaded = load_config(root).expect("default config loads");
    loaded.respect_gitignore = false;
    let plan = AnalysisPlan::from_capability_names_for_test(REFINED);
    AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &Cache::new("", false),
        config_digest: "config",
        rule_digest: "rules",
        plan: &plan,
        parallel: false,
    })
    .expect("kernel should run")
}

/// The exact targets every answer gives the last call site whose text is `site`
/// in `source`: `whole`, `stored` and `served`, each as the names of the
/// functions it claims exactly.
#[derive(Debug, Default, PartialEq, Eq)]
struct ExactTargets {
    whole: BTreeSet<String>,
    stored: BTreeSet<String>,
    served: BTreeSet<String>,
}

fn exact_targets(path: &str, source: &str, site: &str) -> ExactTargets {
    let temp = tempfile::tempdir().expect("temp dir");
    std::fs::write(temp.path().join(path), source).expect("write source");
    let start = u32::try_from(source.rfind(site).expect("the site is in the source"))
        .expect("small source");
    let end = start + u32::try_from(site.len()).expect("small site");
    let output = run(temp.path());
    let db = &output.db;

    let function_name = db
        .functions()
        .iter()
        .map(|function| (function.id, function.name.to_string()))
        .collect::<BTreeMap<_, _>>();
    let site_ids = db
        .call_sites()
        .iter()
        .filter(|call| call.span.start_byte == start && call.span.end_byte == end)
        .map(|call| call.id)
        .collect::<BTreeSet<_>>();
    assert!(!site_ids.is_empty(), "no call site spans {site:?}");

    let mut targets = ExactTargets::default();
    let node_name = db
        .semantic_nodes()
        .iter()
        .filter_map(|node| match node.kind {
            NodeKind::Function(id) => Some((node.id, function_name[&id].clone())),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let site_nodes = db
        .semantic_nodes()
        .iter()
        .filter_map(|node| match node.kind {
            NodeKind::Callsite(id) if site_ids.contains(&id) => Some(node.id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let interner = db.stable_key_interner();
    let inputs = TsPointsToInputs::from_db(db);
    let result = solve_ts_points_to(&interner, &inputs, &SolverBudget::default());
    for answer in site_answers(&inputs, &result) {
        if site_nodes.contains(&answer.callsite) && matches!(answer.claim, EdgeClaim::Exact(_)) {
            targets.whole.extend(
                answer
                    .targets
                    .iter()
                    .map(|target| node_name[target].clone()),
            );
        }
    }

    let graph = export(db, temp.path(), None).expect("the package graph exports");
    let graph_function = graph
        .functions
        .iter()
        .map(|function| (function.node, function.name.to_string()))
        .collect::<BTreeMap<_, _>>();
    let graph_sites = graph
        .callsites
        .iter()
        .filter(|entry| {
            let caller = graph
                .functions
                .iter()
                .find(|function| function.node == entry.caller)
                .expect("a call site's caller is a function");
            caller.identity.start + entry.identity.start == start
                && caller.identity.start + entry.identity.end == end
        })
        .map(|entry| entry.node)
        .collect::<BTreeSet<_>>();
    for entry in &graph.sites {
        if graph_sites.contains(&entry.callsite) && matches!(entry.claim, EdgeClaim::Exact(_)) {
            targets.stored.extend(
                entry
                    .targets
                    .iter()
                    .map(|target| graph_function[target].clone()),
            );
        }
    }

    let index = db.call_graph_index();
    for id in 0..u32::try_from(index.edges.len()).expect("few edges") {
        let edge = index.edge(db, id);
        if edge.span.start_byte == start
            && edge.span.end_byte == end
            && edge.precision.tier() == ClaimTier::Exact
            && let Some(function) = edge.function
        {
            targets.served.insert(function_name[&function].clone());
        }
    }
    targets
}

fn none() -> ExactTargets {
    ExactTargets::default()
}

fn only(name: &str) -> ExactTargets {
    let names = BTreeSet::from([name.to_string()]);
    ExactTargets {
        whole: names.clone(),
        stored: names.clone(),
        served: names,
    }
}

/// The control case: with nothing dynamic in the way, the same call is exact
/// in every answer.
#[test]
fn a_plain_lexical_call_is_exact() {
    let source = "function a(){console.log('a')} let f=a; f();\n";
    assert_eq!(exact_targets("plain.cjs", source, "f()"), only("a"));
}

/// Node prints `b`: the call reads `f` from the object of the `with`.
#[test]
fn a_call_inside_with_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  let f=a; with ({f:b}) { f(); }\n";
    assert_eq!(exact_targets("with.cjs", source, "f()"), none());
}

/// Node prints `b`: the object of the `with` shadows the declaration too.
#[test]
fn a_direct_call_of_a_declaration_inside_with_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  with ({a:b}) { a(); }\n";
    assert_eq!(exact_targets("with-declared.cjs", source, "a()"), none());
}

/// Node prints `b`: a function declared in the body of a `with` keeps its
/// object in scope after the `with` has run.
#[test]
fn a_call_in_a_function_declared_inside_with_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  let o={f:b}; let f=a; with (o) { var h = function(){ f(); }; } h();\n";
    assert_eq!(exact_targets("with-closure.cjs", source, "f()"), none());
}

/// Node prints `b`: the direct `eval` writes the binding.
#[test]
fn a_call_after_eval_writes_the_binding_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  let f=a; eval(\"f=b\"); f();\n";
    assert_eq!(exact_targets("eval-write.cjs", source, "f()"), none());
}

/// Node prints `b`: the direct `eval` declares a `var` that shadows the binding.
#[test]
fn a_call_after_eval_declares_a_shadow_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  let f=a; function g(){ eval(\"var f=b\"); f(); } g();\n";
    assert_eq!(exact_targets("eval-shadow.cjs", source, "f()"), none());
}

/// Node prints `b`: an assignment in the body of a `with` writes the property
/// of its object.
#[test]
fn a_method_written_through_with_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  const o={g:a}; with (o) { g=b; } o.g();\n";
    assert_eq!(exact_targets("with-write.cjs", source, "o.g()"), none());
}

/// Node prints `b`: the object of the `with` came in as a parameter.
#[test]
fn a_method_written_through_a_with_parameter_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  const o={g:a}; function w(x){ with (x) { g=b; } } w(o); o.g();\n";
    assert_eq!(exact_targets("with-param.cjs", source, "o.g()"), none());
}
