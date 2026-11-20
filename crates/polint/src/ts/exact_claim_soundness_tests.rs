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

    // A served edge names a function, or only the symbol it binds; a rule
    // reads either as the exact callee.
    let symbol_name = db
        .symbols()
        .iter()
        .map(|symbol| (symbol.id, symbol.name.to_string()))
        .collect::<BTreeMap<_, _>>();
    let index = db.call_graph_index();
    for id in 0..u32::try_from(index.edges.len()).expect("few edges") {
        let edge = index.edge(db, id);
        if edge.span.start_byte != start
            || edge.span.end_byte != end
            || edge.precision.tier() != ClaimTier::Exact
        {
            continue;
        }
        if let Some(function) = edge.function {
            targets.served.insert(function_name[&function].clone());
        } else if let Some(symbol) = edge.symbol {
            targets.served.insert(symbol_name[&symbol].clone());
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

/// Node prints `b`: an eval in a nested function writes the module binding.
#[test]
fn a_call_after_a_nested_eval_writes_the_module_binding_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  let f=a; function g(){ eval(\"f=b\"); } g(); f();\n";
    assert_eq!(
        exact_targets("eval-nested-write.cjs", source, "f()"),
        none()
    );
}

/// Node prints `b`: a `var` of non-strict eval code in a block joins the
/// function scope, where it shadows the module binding.
#[test]
fn a_call_after_eval_declares_a_shadow_from_a_block_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  let f=a; function g(){ { eval(\"var f=b\"); } f(); } g();\n";
    assert_eq!(
        exact_targets("eval-block-shadow.cjs", source, "f()"),
        none()
    );
}

/// Node prints `b`: a closure reads the binding the eval wrote.
#[test]
fn a_closure_call_of_a_binding_eval_writes_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  function g(){ let f=a; eval(\"f=b\"); return function(){ f(); }; } g()();\n";
    assert_eq!(exact_targets("eval-closure.cjs", source, "f()"), none());
}

/// Node prints `b`: the eval writes a field of an object it names.
#[test]
fn a_method_call_of_an_object_eval_writes_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  const o={m:a}; function g(){ eval(\"o.m=b\"); } g(); o.m();\n";
    assert_eq!(exact_targets("eval-field.cjs", source, "o.m()"), none());
}

/// Node prints `b`: eval code the call does not spell out may write any
/// binding its call can see, also in a module.
#[test]
fn a_call_after_an_eval_of_unknown_code_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  let f=a; function g(code){ eval(code); } g(\"f=b\"); f();\n";
    assert_eq!(exact_targets("eval-unknown.mjs", source, "f()"), none());
}

/// Node prints `b`: the eval writes the CommonJS `exports` object.
#[test]
fn a_call_of_an_export_eval_writes_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  exports.run=a; function g(){ eval(\"exports.run=b\"); } g(); exports.run();\n";
    assert_eq!(
        exact_targets("eval-exports.cjs", source, "exports.run()"),
        none()
    );
}

/// Node prints `b`: the eval writes a field of the method's `this`.
#[test]
fn a_method_call_after_eval_writes_this_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  const o={m:a, set(){ eval(\"this.m=b\"); }}; o.set(); o.m();\n";
    assert_eq!(exact_targets("eval-this.mjs", source, "o.m()"), none());
}

/// Node prints `b`: a `var` of non-strict eval code shadows the global the
/// `Object.assign` model describes.
#[test]
fn a_call_after_eval_shadows_a_modelled_global_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  function g(){ const o={}; eval(\"var Object={assign(t){t.m=b}}\"); \
                  Object.assign(o,{m:a}); o.m(); } g();\n";
    assert_eq!(
        exact_targets("eval-global-shadow.cjs", source, "o.m()"),
        none()
    );
}

/// Node prints `b` for each callee: a type assertion, `satisfies` or a
/// non-null assertion is erased before the code runs, so the call is still a
/// direct `eval` and writes the binding.
#[test]
fn a_call_after_an_eval_behind_an_erased_type_wrapper_claims_no_exact_target() {
    for (path, callee) in [
        ("eval-as.mts", "(eval as any)"),
        ("eval-satisfies.mts", "(eval satisfies any)"),
        ("eval-non-null.mts", "eval!"),
        ("eval-non-null-parens.mts", "(eval!)"),
        ("eval-nested-as.mts", "((eval as any) as any)"),
        ("eval-angle.ts", "(<any>eval)"),
    ] {
        let source = format!(
            "function a(){{console.log('a')}} function b(){{console.log('b')}} \
             let f=a; {callee}('f=b'); f();\n"
        );
        assert_eq!(exact_targets(path, &source, "f()"), none(), "{callee}");
    }
}

/// Node prints `a`: an eval whose code names no binding leaves every call of
/// the file exact.
#[test]
fn an_eval_that_names_no_binding_keeps_the_calls_of_its_file_exact() {
    let source =
        "function a(){console.log('a')} function isolated(){eval('void 0')} isolated(); a();\n";
    assert_eq!(exact_targets("eval-isolated.cjs", source, "a()"), only("a"));
    assert_eq!(
        exact_targets("eval-isolated.cjs", source, "isolated()"),
        only("isolated")
    );
}

/// Node prints `h`: eval code the call does not spell out reaches only the
/// scopes around its call, not a local of another function.
#[test]
fn an_eval_of_unknown_code_keeps_a_local_of_another_function_exact() {
    let source = "function sibling(){ function h(){console.log('h')} h(); } \
                  function g(code){ eval(code); } g(\"1\"); sibling();\n";
    assert_eq!(exact_targets("eval-sibling.mjs", source, "h()"), only("h"));
}

/// Node prints `b` twice: the eval writes the binding it names and no other.
#[test]
fn an_eval_keeps_a_binding_its_code_does_not_name_exact() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  function w(){ let f=a, k=b; eval(\"f=b\"); f(); k(); } w();\n";
    assert_eq!(exact_targets("eval-named.cjs", source, "f()"), none());
    assert_eq!(exact_targets("eval-named.cjs", source, "k()"), only("b"));
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

/// Node prints `b`: running the block also assigns its function to the `var`.
#[test]
fn a_call_of_a_var_a_block_function_overwrites_claims_no_exact_target() {
    let source = "function a(){console.log('a')} var f=a; \
                  if(true){function f(){console.log('b')}} f();\n";
    assert_eq!(exact_targets("annex.cjs", source, "f()"), none());
}

/// Node prints `b`: the block function overwrites the outer declaration too.
#[test]
fn a_call_of_a_declaration_a_block_function_overwrites_claims_no_exact_target() {
    let source = "function a(){console.log('a')} function f(){console.log('a2')} \
                  if(true){function f(){console.log('b')}} f();\n";
    assert_eq!(exact_targets("annex-declared.cjs", source, "f()"), none());
}

/// Node prints `b`: the same happens in a function scope.
#[test]
fn a_call_in_a_function_whose_var_a_block_function_overwrites_claims_no_exact_target() {
    let source = "function a(){console.log('a')} \
                  function g(){ var f=a; { function f(){console.log('b')} } f(); } g();\n";
    assert_eq!(exact_targets("annex-function.cjs", source, "f()"), none());
}

/// Node prints `a`: an alias read before the block runs keeps its value. It
/// may still claim no exact target, but never `b`.
#[test]
fn an_alias_read_before_the_block_function_runs_claims_no_wrong_target() {
    let source = "function a(){console.log('a')} var f=a; var h=f; \
                  if(true){function f(){console.log('b')}} h();\n";
    let targets = exact_targets("annex-alias.cjs", source, "h()");
    assert!(targets == none() || targets == only("a"), "{targets:?}");
}

/// In strict code a block function stays in its block, so the call keeps its
/// exact target.
#[test]
fn a_block_function_of_strict_code_leaves_the_outer_binding_exact() {
    let source = "'use strict'; function a(){console.log('a')} let f=a; \
                  if(true){function f(){console.log('b')}} f();\n";
    assert_eq!(exact_targets("annex-strict.cjs", source, "f()"), only("a"));
}

/// Node prints `TypeError`: an arrow cannot be constructed, so `new` throws
/// before the arrow runs.
#[test]
fn a_new_of_an_arrow_claims_no_exact_target() {
    let source = "const arrow=()=>console.log('arrow entered'); \
                  try{new arrow()}catch(e){console.log(e.name)}\n";
    assert_eq!(
        exact_targets("new-arrow.mjs", source, "new arrow()"),
        none()
    );
}

/// Node prints `TypeError`: a class can only be constructed.
#[test]
fn a_call_of_a_class_claims_no_exact_target() {
    let source = "class C {constructor(){console.log('constructor entered')}} \
                  try{C()}catch(e){console.log(e.name)}\n";
    assert_eq!(exact_targets("call-class.mjs", source, "C()"), none());
}

/// Node prints `TypeError`: a class without a constructor is still a class.
#[test]
fn a_call_of_a_class_with_an_implicit_constructor_claims_no_exact_target() {
    let source = "class C {m(){}} try{C()}catch(e){console.log(e.name)}\n";
    assert_eq!(exact_targets("call-implicit.mjs", source, "C()"), none());
}

/// Node prints `TypeError`: a generator cannot be constructed.
#[test]
fn a_new_of_a_generator_claims_no_exact_target() {
    let source = "function* g(){console.log('generator entered')} \
                  try{new g()}catch(e){console.log(e.name)}\n";
    assert_eq!(
        exact_targets("new-generator.mjs", source, "new g()"),
        none()
    );
}

/// Node prints `TypeError`: an async function cannot be constructed.
#[test]
fn a_new_of_an_async_function_claims_no_exact_target() {
    let source = "async function a(){console.log('async entered')} \
                  try{new a()}catch(e){console.log(e.name)}\n";
    assert_eq!(exact_targets("new-async.mjs", source, "new a()"), none());
}

/// Node prints `TypeError`: a method of an object literal cannot be
/// constructed.
#[test]
fn a_new_of_an_object_method_claims_no_exact_target() {
    let source = "const o={m(){console.log('method entered')}}; const m=o.m; \
                  try{new m()}catch(e){console.log(e.name)}\n";
    assert_eq!(exact_targets("new-method.mjs", source, "new m()"), none());
}

/// Node prints `TypeError`: neither can a method of a class.
#[test]
fn a_new_of_a_class_method_claims_no_exact_target() {
    let source = "class C {m(){console.log('method entered')}} const m=C.prototype.m; \
                  try{new m()}catch(e){console.log(e.name)}\n";
    assert_eq!(
        exact_targets("new-class-method.mjs", source, "new m()"),
        none()
    );
}

/// Node prints `entered` twice: a plain function can be called and
/// constructed, so both sites keep their exact target.
#[test]
fn a_plain_function_is_exact_when_called_and_constructed() {
    let source = "function F(){console.log('entered')} new F(); F();\n";
    assert_eq!(exact_targets("plain-new.mjs", source, "new F()"), only("F"));
    assert_eq!(exact_targets("plain-call.mjs", source, "F()"), only("F"));
}

/// Node prints `constructor entered`: constructing a class runs its
/// constructor, which the site may claim exactly, and never anything else.
#[test]
fn a_new_of_a_class_claims_no_wrong_target() {
    let source = "class C {constructor(){console.log('constructor entered')}} new C();\n";
    let targets = exact_targets("new-class.mjs", source, "new C()");
    assert!(targets == none() || targets == only("C"), "{targets:?}");
}

/// Node prints `b`: an assignment replaces the function a declaration bound.
#[test]
fn a_call_of_a_reassigned_declaration_claims_no_exact_target() {
    let source = "function b(){console.log('b')} function g(){console.log('g')} \
                  g=b; g();\n";
    assert_eq!(exact_targets("reassign.cjs", source, "g()"), none());
}

/// Node prints `b`: the binding is written in another function, which runs
/// before the call.
#[test]
fn a_call_of_a_declaration_another_function_writes_claims_no_exact_target() {
    let source = "function b(){console.log('b')} function later(){console.log('later')} \
                  function swap(){ later=b; } swap(); later();\n";
    assert_eq!(
        exact_targets("reassign-elsewhere.cjs", source, "later()"),
        none()
    );
}

/// Node prints `Z`: an assignment replaces the class a declaration bound.
#[test]
fn a_new_of_a_reassigned_class_claims_no_exact_target() {
    let source = "class K {constructor(){console.log('K')}} \
                  class Z {constructor(){console.log('Z')}} K=Z; new K();\n";
    assert_eq!(
        exact_targets("reassign-class.cjs", source, "new K()"),
        none()
    );
}

/// Node prints `b`: the direct `eval` writes the local declaration.
#[test]
fn a_call_of_a_declaration_eval_writes_claims_no_exact_target() {
    let source = "function b(){console.log('b')} globalThis.b=b; \
                  function e(){ function local(){console.log('local')} eval(\"local=b\"); local(); } e();\n";
    assert_eq!(
        exact_targets("eval-declared.cjs", source, "local()"),
        none()
    );
}

/// Node prints `Z`: the `with` object shadows the class.
#[test]
fn a_new_inside_with_claims_no_exact_target() {
    let source = "class K {constructor(){console.log('K')}} \
                  class Z {constructor(){console.log('Z')}} with ({K:Z}) { new K(); }\n";
    assert_eq!(exact_targets("with-new.cjs", source, "new K()"), none());
}

/// Node prints `b`: the store runs the inherited setter, which drops the
/// value, and the read runs the inherited getter.
#[test]
fn a_call_of_a_property_an_inherited_setter_takes_claims_no_wrong_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  const o=Object.create({set w(v){}, get w(){return b}}); o.w=a; o.w();\n";
    let targets = exact_targets("inherited-setter.cjs", source, "o.w()");
    assert!(targets == none() || targets == only("b"), "{targets:?}");
}

/// Node prints `b`: the same through the setter of a class.
#[test]
fn a_call_of_a_property_a_class_setter_takes_claims_no_wrong_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  class L { set x(v){} get x(){return b} } const l=new L(); l.x=a; l.x();\n";
    let targets = exact_targets("class-setter.cjs", source, "l.x()");
    assert!(targets == none() || targets == only("b"), "{targets:?}");
}

/// Node prints `b`: the same through a setter `Object.defineProperty` gives
/// the prototype.
#[test]
fn a_call_of_a_property_a_defined_setter_takes_claims_no_wrong_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  const P={}; Object.defineProperty(P,'w',{set(v){}, get(){return b}}); \
                  const o=Object.create(P); o.w=a; o.w();\n";
    let targets = exact_targets("defined-setter.cjs", source, "o.w()");
    assert!(targets == none() || targets == only("b"), "{targets:?}");
}

/// Node prints `b`: in sloppy code `arguments[0]` and the parameter are one
/// binding, so the store replaces the function the caller passed.
#[test]
fn a_call_of_a_parameter_arguments_writes_claims_no_wrong_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  function g(f){ arguments[0]=b; f(); } g(a);\n";
    let targets = exact_targets("arguments-write.cjs", source, "f()");
    assert!(targets == none() || targets == only("b"), "{targets:?}");
}

/// Node prints `b`: the other way round, an assignment to the parameter is
/// read through `arguments`.
#[test]
fn a_call_through_arguments_of_a_written_parameter_claims_no_wrong_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  function g(f){ f=b; arguments[0](); } g(a);\n";
    let targets = exact_targets("arguments-read.cjs", source, "arguments[0]()");
    assert!(targets == none() || targets == only("b"), "{targets:?}");
}

/// Node prints `b`: the `arguments` object escapes to a function that writes
/// the parameter through it.
#[test]
fn a_call_of_a_parameter_whose_arguments_escape_claims_no_wrong_target() {
    let source = "function a(){console.log('a')} function b(){console.log('b')} \
                  function w(x){ x[0]=b; } function g(f){ w(arguments); f(); } g(a);\n";
    let targets = exact_targets("arguments-escape.cjs", source, "f()");
    assert!(targets == none() || targets == only("b"), "{targets:?}");
}

/// Node prints `a`: with no write to the class's prototype link, `super()`
/// runs the constructor the class extends, and the edge stays exact.
#[test]
fn a_super_call_of_a_class_whose_prototype_link_is_not_written_stays_exact() {
    let source = "class A {constructor(){console.log('a')}} \
                  class B extends A {constructor(){super()}} new B();\n";
    let targets = exact_targets("super-plain.mjs", source, "super()");
    assert!(targets == only("A"), "{targets:?}");
}

/// Node prints `c`: `super()` runs the constructor's current prototype, and
/// `Object.setPrototypeOf` replaced it.
#[test]
fn a_super_call_after_set_prototype_of_claims_no_wrong_target() {
    let source = "class A {constructor(){console.log('a')}} class C {constructor(){console.log('c')}} \
                  class B extends A {constructor(){super()}} Object.setPrototypeOf(B,C); new B();\n";
    let targets = exact_targets("super-set-prototype.mjs", source, "super()");
    assert!(targets == none() || targets == only("C"), "{targets:?}");
}

/// Node prints `c`: the same through a write of `__proto__`.
#[test]
fn a_super_call_after_a_proto_write_claims_no_wrong_target() {
    let source = "class A {constructor(){console.log('a')}} class C {constructor(){console.log('c')}} \
                  class B extends A {constructor(){super()}} B.__proto__ = C; new B();\n";
    let targets = exact_targets("super-proto-write.cjs", source, "super()");
    assert!(targets == none() || targets == only("C"), "{targets:?}");
}

/// Node prints `r`: the same through `Reflect.setPrototypeOf`.
#[test]
fn a_super_call_after_reflect_set_prototype_of_claims_no_wrong_target() {
    let source = "class A {constructor(){console.log('a')}} class R {constructor(){console.log('r')}} \
                  class B extends A {constructor(){super()}} Reflect.setPrototypeOf(B, R); new B();\n";
    let targets = exact_targets("super-reflect.mjs", source, "super()");
    assert!(targets == none() || targets == only("R"), "{targets:?}");
}

/// Node prints `x`: `super.run()` reads `run` from the current prototype of
/// the method's home object, `B.prototype`.
#[test]
fn a_super_method_call_after_the_prototype_object_changes_claims_no_wrong_target() {
    let source = "class A {run(){console.log('a')}} const X = {run: function x(){console.log('x')}}; \
                  class B extends A {run(){super.run()}} Object.setPrototypeOf(B.prototype, X); \
                  new B().run();\n";
    let targets = exact_targets("super-method.mjs", source, "super.run()");
    assert!(targets == none() || targets == only("x"), "{targets:?}");
}

/// Node prints `x`: the same through a write of the prototype object's
/// `__proto__`.
#[test]
fn a_super_method_call_after_a_prototype_proto_write_claims_no_wrong_target() {
    let source = "class A {run(){console.log('a')}} \
                  class B extends A {run(){super.run()}} \
                  B.prototype.__proto__ = {run: function x(){console.log('x')}}; new B().run();\n";
    let targets = exact_targets("super-method-proto.cjs", source, "super.run()");
    assert!(targets == none() || targets == only("x"), "{targets:?}");
}

/// Node prints `x`: the same where another function changes the prototype.
#[test]
fn a_super_method_call_after_a_function_changes_the_prototype_claims_no_wrong_target() {
    let source = "class A {run(){console.log('a')}} class B extends A {run(){super.run()}} \
                  function swap(k){ Object.setPrototypeOf(k.prototype, {run: function x(){console.log('x')}}); } \
                  swap(B); new B().run();\n";
    let targets = exact_targets("super-method-swap.mjs", source, "super.run()");
    assert!(targets == none() || targets == only("x"), "{targets:?}");
}

/// Node prints `c`: a static method's home object is the class itself.
#[test]
fn a_static_super_method_call_after_set_prototype_of_claims_no_wrong_target() {
    let source = "class A {static run(){console.log('a')}} \
                  class B extends A {static run(){super.run()}} \
                  Object.setPrototypeOf(B, {run: function c(){console.log('c')}}); B.run();\n";
    let targets = exact_targets("super-static.mjs", source, "super.run()");
    assert!(targets == none() || targets == only("c"), "{targets:?}");
}

/// Node prints `w`: the parent's method was replaced before the call.
#[test]
fn a_super_method_call_of_a_replaced_parent_method_claims_no_wrong_target() {
    let source = "class A {run(){console.log('a')}} class B extends A {run(){super.run()}} \
                  A.prototype.run = function w(){console.log('w')}; new B().run();\n";
    let targets = exact_targets("super-replaced.mjs", source, "super.run()");
    assert!(targets == none() || targets == only("w"), "{targets:?}");
}

/// Node prints `x`: the home object of a method of an object literal is the
/// literal, also inside a class.
#[test]
fn a_super_method_call_of_an_object_method_inside_a_class_claims_no_wrong_target() {
    let source = "class A {n(){console.log('a')}} \
                  class B extends A { m(){ const o = { __proto__: {n: function x(){console.log('x')}}, \
                  n(){ super.n() } }; o.n(); } } new B().m();\n";
    let targets = exact_targets("super-object-in-class.mjs", source, "super.n()");
    assert!(targets == none() || targets == only("x"), "{targets:?}");
}

/// Node prints `q`: `super.n()` in an object method reads the literal's
/// current prototype.
#[test]
fn a_super_method_call_of_an_object_method_after_set_prototype_of_claims_no_wrong_target() {
    let source = "const P = {n: function p(){console.log('p')}}; \
                  const o = { __proto__: P, n(){ super.n() } }; \
                  Object.setPrototypeOf(o, {n: function q(){console.log('q')}}); o.n();\n";
    let targets = exact_targets("super-object.mjs", source, "super.n()");
    assert!(targets == none() || targets == only("q"), "{targets:?}");
}
