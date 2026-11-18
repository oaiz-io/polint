//! Callable constructs exercised through the kernel and value-flow lowering.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_api::FunctionFact;
use crate::analysis_neutral::calls::facts::{CallPrecision, CallTargetStatus};

/// A class *is* its constructor callable: a call in a `constructor(){}` body
/// belongs to the class function, at every boundary that can disagree — the
/// MIR body owner recorded on the call site, and the caller of each refined
/// edge (the `fun2fun` mirror). Members that own their own body in the same
/// model — methods, static blocks, field initializers, nested closures —
/// must keep their attribution, so they are asserted as controls here.
#[test]
fn class_constructor_bodies_are_owned_by_the_class_function() {
    // (source, unique call snippet, owning function text, resolved target text)
    let cases: &[(&str, &str, &str, &str)] = &[
        // micro/classes: `f1()` in `class C1`'s constructor.
        (
            "function f1() {}\nclass C1 {\n    constructor() {\n        f1();\n    }\n}\nnew C1;\n",
            "f1();",
            "class C1 {\n    constructor() {\n        f1();\n    }\n}",
            "function f1() {}",
        ),
        // micro/private: a private field's callable, invoked in the constructor.
        (
            "class C {\n    #foo = () => {};\n    constructor() {\n        this.#foo();\n    }\n}\nnew C();\n",
            "this.#foo();",
            "class C {\n    #foo = () => {};\n    constructor() {\n        this.#foo();\n    }\n}",
            "() => {}",
        ),
        // micro/super: `super.m()` in a subclass constructor.
        (
            "class A {\n    m() {}\n}\nclass B extends A {\n    constructor() {\n        super.m();\n    }\n}\nnew B();\n",
            "super.m();",
            "class B extends A {\n    constructor() {\n        super.m();\n    }\n}",
            "m() {}",
        ),
        // micro/super: `super(callback)` flows into the superclass constructor,
        // whose body's calls belong to the *superclass* function.
        (
            "class A {\n    constructor(x) {\n        x();\n    }\n}\nclass B extends A {\n    constructor() {\n        super(() => {});\n    }\n}\nnew B();\n",
            "x();",
            "class A {\n    constructor(x) {\n        x();\n    }\n}",
            "() => {}",
        ),
        // micro/super5: a class *expression* returned from a function, whose
        // constructor immediately invokes a nested arrow.
        (
            "class A {\n    m() {}\n}\nfunction b() {\n    return class B extends A {\n        constructor() {\n            super();\n            (() => { super.m(); })();\n        }\n    };\n}\nnew (b())();\n",
            "() => { super.m(); })()",
            "class B extends A {\n        constructor() {\n            super();\n            (() => { super.m(); })();\n        }\n    }",
            "() => { super.m(); }",
        ),
        // Control: a method body keeps its own owner.
        (
            "function f1() {}\nclass C {\n    m() {\n        f1();\n    }\n}\nnew C().m();\n",
            "f1();",
            "m() {\n        f1();\n    }",
            "function f1() {}",
        ),
        // Control: a static block is its own function.
        (
            "function f1() {}\nclass C {\n    static {\n        f1();\n    }\n}\n",
            "f1();",
            "static {\n        f1();\n    }",
            "function f1() {}",
        ),
        // Control: a field initializer is its own function.
        (
            "function f1() {}\nclass C {\n    p = (f1(), 1);\n}\nnew C();\n",
            "f1(), 1",
            "p = (f1(), 1);",
            "function f1() {}",
        ),
        // Control: a closure nested in a constructor owns its own body.
        (
            "function f1() {}\nclass C {\n    constructor() {\n        const g = () => {\n            f1();\n        };\n        g();\n    }\n}\nnew C();\n",
            "f1();",
            "() => {\n            f1();\n        }",
            "function f1() {}",
        ),
        // Control: the constructor of a class *expression* bound to a variable.
        (
            "function f1() {}\nconst C = class {\n    constructor() {\n        f1();\n    }\n};\nnew C();\n",
            "f1();",
            "class {\n    constructor() {\n        f1();\n    }\n}",
            "function f1() {}",
        ),
    ];
    for (source, call, caller_text, target_text) in cases {
        assert_eq!(
            source.matches(call).count(),
            1,
            "call snippet {call:?} must be unique in {source}"
        );
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("main.js"), source).unwrap();
        let output = crate::eval::observed::run_kernel_for_repo_for_test(repo.path()).unwrap();
        let text = |function: &FunctionFact| {
            source[function.span.start_byte as usize..function.span.end_byte as usize].to_string()
        };
        let functions = output
            .db
            .functions()
            .iter()
            .map(|function| (function.id, function))
            .collect::<BTreeMap<_, _>>();
        let start = source.find(call).unwrap() as u32;
        // `f()` and `f()()` share a start; take the outermost call that still
        // fits inside the snippet.
        let site = output
            .db
            .call_sites()
            .iter()
            .filter(|site| {
                site.span.start_byte == start && site.span.end_byte <= start + call.len() as u32
            })
            .max_by_key(|site| site.span.end_byte)
            .unwrap_or_else(|| panic!("no call site for {call:?} in {source}"));
        assert_eq!(
            text(functions[&site.caller]),
            *caller_text,
            "call site owner for {call:?} in {source}"
        );
        let resolved = output
            .db
            .refined_call_edges()
            .iter()
            .filter(|edge| edge.site == site.id && edge.status == CallTargetStatus::Resolved)
            .filter_map(|edge| {
                Some((
                    text(functions.get(&edge.caller)?),
                    text(functions.get(&edge.target_function?)?),
                ))
            })
            .collect::<BTreeSet<_>>();
        assert!(
            resolved.contains(&(caller_text.to_string(), target_text.to_string())),
            "expected {caller_text:?} -> {target_text:?} for {call:?} in {source}, got {resolved:?}"
        );
        // Owner equality: no refined edge for this site may claim a different
        // caller. A second owner is a false positive in the fun2fun mirror.
        assert!(
            resolved.iter().all(|(caller, _)| caller == caller_text),
            "one owner per call site; got {resolved:?} for {call:?} in {source}"
        );
    }
}

/// Iteration, collections and callbacks of built-ins are callable models the
/// value flow does not describe: their edges stay heuristic. Destructuring
/// and a returned parameter are described by the value flow, which answers
/// them on the exact tier.
#[test]
fn callable_models_reach_the_shared_solver_on_the_tier_that_describes_them() {
    use CallPrecision::{Heuristic, SetupAware};
    let cases = [
        (
            "const list = [target]; for (const callable of list) { callable(); }",
            SetupAware,
        ),
        (
            "const box = { method: target }; const { method: callable } = box; callable();",
            SetupAware,
        ),
        (
            "function pass(fn) { return fn; } const callable = pass(target); callable();",
            SetupAware,
        ),
        (
            "const list = new Set([target]); for (const callable of list) { callable(); }",
            Heuristic,
        ),
        (
            "const list = [target]; list.forEach(callable => callable());",
            Heuristic,
        ),
        (
            "Promise.resolve(target).then(callable => callable());",
            Heuristic,
        ),
        (
            "const box = { nested: { method: target } }; const { nested: { method: callable } } = box; callable();",
            SetupAware,
        ),
        (
            "function* sequence() { yield target; } for (const callable of sequence()) { callable(); }",
            Heuristic,
        ),
    ];
    for (suffix, precision) in cases {
        let repo = tempfile::tempdir().unwrap();
        let source = format!("function target() {{}} function decoy() {{}} {suffix}");
        std::fs::write(repo.path().join("main.js"), &source).unwrap();
        let output = crate::eval::observed::run_kernel_for_repo_for_test(repo.path()).unwrap();
        let target = output
            .db
            .functions()
            .iter()
            .find(|function| function.name == "target")
            .unwrap();
        let start = source.rfind("callable()").unwrap() as u32;
        let site = output
            .db
            .call_sites()
            .iter()
            .find(|site| site.span.start_byte == start)
            .unwrap();
        let edges = output
            .db
            .refined_call_edges()
            .iter()
            .filter(|edge| edge.site == site.id && edge.status == CallTargetStatus::Resolved)
            .collect::<Vec<_>>();
        assert_eq!(
            edges
                .iter()
                .filter_map(|edge| edge.target_function)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([target.id]),
            "{suffix}"
        );
        assert!(
            edges.iter().all(|edge| edge.precision == precision),
            "{suffix}: {edges:?}"
        );
    }
}

#[test]
fn invocation_results_do_not_inherit_the_callee_function() {
    for source in [
        "const callable = (() => 0)(); callable();",
        "const callable = (function make() { return 0; })(); callable();",
        "function target() {} const list = [target]; function invoke(target) { const values = [target]; for (const callable of values) callable(); } invoke(0);",
    ] {
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("main.js"), source).unwrap();
        let output = crate::eval::observed::run_kernel_for_repo_for_test(repo.path()).unwrap();
        let start = source.rfind("callable()").unwrap() as u32;
        let site = output
            .db
            .call_sites()
            .iter()
            .find(|site| site.span.start_byte == start)
            .unwrap();
        assert!(
            output
                .db
                .refined_call_edges()
                .iter()
                .all(|edge| { edge.site != site.id || edge.status != CallTargetStatus::Resolved }),
            "an opaque or non-callable result must remain unresolved: {source}"
        );
    }
}

#[test]
fn module_callable_shapes_reuse_resolved_import_facts() {
    let cases = [
        (
            "main.js",
            "const { target: callable } = require('./helper.js'); callable();",
            "helper.js",
            "function target() {} exports.target = target;",
        ),
        (
            "main.mjs",
            "import { target as callable } from './helper.mjs'; callable();",
            "helper.mjs",
            "export function target() {}",
        ),
        (
            "main.js",
            "const __importDefault = mod => mod && mod.__esModule ? mod : { default: mod }; const helper = __importDefault(require('./helper.js')); const callable = helper.default; callable();",
            "helper.js",
            "function target() {} module.exports = target;",
        ),
    ];
    for (main_path, source, helper_path, helper_source) in cases {
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join(main_path), source).unwrap();
        std::fs::write(repo.path().join(helper_path), helper_source).unwrap();
        let output = crate::eval::observed::run_kernel_for_repo_for_test(repo.path()).unwrap();
        let target = output
            .db
            .functions()
            .iter()
            .find(|function| function.name == "target")
            .unwrap();
        let main = output
            .db
            .files()
            .iter()
            .find(|file| file.relative_path == main_path)
            .unwrap();
        let start = source.rfind("callable()").unwrap() as u32;
        let site = output
            .db
            .call_sites()
            .iter()
            .find(|site| site.file == main.id && site.span.start_byte == start)
            .unwrap();
        assert!(
            output.db.refined_call_edges().iter().any(|edge| {
                edge.site == site.id
                    && edge.target_function == Some(target.id)
                    && edge.status == CallTargetStatus::Resolved
            }),
            "imported callable must resolve: {source}"
        );
    }
}

/// The CommonJS default-interop preamble TypeScript and Babel emit is two
/// things at once: a callable the file itself declares (so calls *to*
/// `__importDefault(...)` resolve to it), and a wrapper over the module it
/// receives (so reads *through* it resolve). Which wrapper shape applies is
/// decided by the wrapped module — an ESM-marked module with `exports.default`
/// passes through, a bare `module.exports = fn` is wrapped as `{ default: fn }`.
#[test]
fn commonjs_default_interop_helper_and_wrapper_shapes_resolve() {
    const HELPER: &str = "var __importDefault = (this && this.__importDefault) || function (mod) {\n    return (mod && mod.__esModule) ? mod : { \"default\": mod };\n};\n";
    const HELPER_TEXT: &str =
        "function (mod) {\n    return (mod && mod.__esModule) ? mod : { \"default\": mod };\n}";
    // (main tail after the helper preamble, helper module source, call snippet, target text)
    let cases: &[(&str, &str, &str, &str)] = &[
        // The helper call itself — client4/client5's `__importDefault(require(…))`.
        (
            "const lib = __importDefault(require('./helper.js'));\n",
            "function target() {}\nmodule.exports = target;\n",
            "__importDefault(require('./helper.js'))",
            HELPER_TEXT,
        ),
        // Marked ESM default: the helper forwards the namespace unchanged.
        (
            "const lib = __importDefault(require('./helper.js'));\nlib.default();\n",
            "Object.defineProperty(exports, \"__esModule\", { value: true });\nexports.default = function foo() {};\n",
            "lib.default()",
            "function foo() {}",
        ),
        // Unmarked CommonJS callable: the helper wraps it as `{ default: mod }`.
        (
            "const lib = __importDefault(require('./helper.js'));\nlib.default();\n",
            "function target() {}\nmodule.exports = target;\n",
            "lib.default()",
            "function target() {}",
        ),
    ];
    for (tail, helper_source, call, target_text) in cases {
        let source = format!("{HELPER}{tail}");
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("main.js"), &source).unwrap();
        std::fs::write(repo.path().join("helper.js"), helper_source).unwrap();
        let output = crate::eval::observed::run_kernel_for_repo_for_test(repo.path()).unwrap();
        let sources = BTreeMap::from([("main.js", source.as_str()), ("helper.js", *helper_source)]);
        let paths = output
            .db
            .files()
            .iter()
            .map(|file| (file.id, file.relative_path.clone()))
            .collect::<BTreeMap<_, _>>();
        let text = |function: &FunctionFact| {
            let path = paths.get(&function.file).expect("function file");
            sources[path.as_str()]
                [function.span.start_byte as usize..function.span.end_byte as usize]
                .to_string()
        };
        let functions = output
            .db
            .functions()
            .iter()
            .map(|function| (function.id, function))
            .collect::<BTreeMap<_, _>>();
        let main = output
            .db
            .files()
            .iter()
            .find(|file| file.relative_path == "main.js")
            .unwrap();
        let start = source.find(call).unwrap() as u32;
        let site = output
            .db
            .call_sites()
            .iter()
            .filter(|site| {
                site.file == main.id
                    && site.span.start_byte == start
                    && site.span.end_byte <= start + call.len() as u32
            })
            .max_by_key(|site| site.span.end_byte)
            .unwrap_or_else(|| panic!("no call site for {call:?} in {source}"));
        let targets = output
            .db
            .refined_call_edges()
            .iter()
            .filter(|edge| edge.site == site.id && edge.status == CallTargetStatus::Resolved)
            .filter_map(|edge| Some(text(functions.get(&edge.target_function?)?)))
            .collect::<BTreeSet<_>>();
        assert!(
            targets.contains(*target_text),
            "expected {target_text:?} for {call:?} in {source}, got {targets:?}"
        );
    }

    // Negative controls: a guard whose branches are not literal callables
    // binds nothing, and a shadowing parameter must not reach the module's
    // helper.
    for (source, call) in [
        (
            format!(
                "{HELPER}function use(__importDefault) {{\n    __importDefault();\n}}\nuse(0);\n"
            ),
            "__importDefault();",
        ),
        (
            "const maybe = (globalThis.a && globalThis.b) || globalThis.c;\nmaybe();\n".to_string(),
            "maybe()",
        ),
    ] {
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("main.js"), &source).unwrap();
        let output = crate::eval::observed::run_kernel_for_repo_for_test(repo.path()).unwrap();
        let start = source.find(call).unwrap() as u32;
        let site = output
            .db
            .call_sites()
            .iter()
            .find(|site| site.span.start_byte == start)
            .unwrap_or_else(|| panic!("no call site for {call:?} in {source}"));
        assert!(
            output
                .db
                .refined_call_edges()
                .iter()
                .all(|edge| { edge.site != site.id || edge.status != CallTargetStatus::Resolved }),
            "a shadowed or non-callable guard must stay unresolved: {source}"
        );
    }
}

/// Resolved-target texts for the unique call site starting at `call` in
/// `main.ts`, or `main.js` when there is no `main.ts`. Returning the whole set (not a membership check) makes every
/// fixture below a precision control too: an extra target fails the same way
/// a missing one does.
fn resolved_targets_at(files: &[(&str, &str)], call: &str) -> BTreeSet<String> {
    targets_at(files, call, false)
}

fn claimed_targets_at(files: &[(&str, &str)], call: &str) -> BTreeSet<String> {
    targets_at(files, call, true)
}

fn targets_at(files: &[(&str, &str)], call: &str, claimed_only: bool) -> BTreeSet<String> {
    let repo = tempfile::tempdir().unwrap();
    for (name, body) in files {
        std::fs::write(repo.path().join(name), body).unwrap();
    }
    let sources = files.iter().copied().collect::<BTreeMap<_, _>>();
    let main_path = if sources.contains_key("main.ts") {
        "main.ts"
    } else {
        "main.js"
    };
    let source = sources[main_path];
    assert_eq!(
        source.matches(call).count(),
        1,
        "call snippet {call:?} must be unique in {source}"
    );
    let output = crate::eval::observed::run_kernel_for_repo_for_test(repo.path()).unwrap();
    let paths = output
        .db
        .files()
        .iter()
        .map(|file| (file.id, file.relative_path.clone()))
        .collect::<BTreeMap<_, _>>();
    let functions = output
        .db
        .functions()
        .iter()
        .map(|function| (function.id, function))
        .collect::<BTreeMap<_, _>>();
    let main = output
        .db
        .files()
        .iter()
        .find(|file| file.relative_path == main_path)
        .unwrap();
    let start = source.find(call).unwrap() as u32;
    // `arr.reduce(fn)` and `arr.reduce(fn)()` share a start byte; the snippet
    // names the outermost call that fits inside it.
    let site = output
        .db
        .call_sites()
        .iter()
        .filter(|site| {
            site.file == main.id
                && site.span.start_byte == start
                && site.span.end_byte <= start + call.len() as u32
        })
        .max_by_key(|site| site.span.end_byte)
        .unwrap_or_else(|| panic!("no call site for {call:?} in {source}"));
    output
        .db
        .refined_call_edges()
        .iter()
        .filter(|edge| edge.site == site.id && edge.status == CallTargetStatus::Resolved)
        .filter(|edge| {
            !claimed_only
                || matches!(
                    edge.precision.tier(),
                    crate::analysis_neutral::calls::facts::ClaimTier::Exact
                        | crate::analysis_neutral::calls::facts::ClaimTier::Conservative
                )
        })
        .filter_map(|edge| {
            let target = functions.get(&edge.target_function?)?;
            let body = sources[paths.get(&target.file)?.as_str()];
            Some(body[target.span.start_byte as usize..target.span.end_byte as usize].to_string())
        })
        .collect()
}

/// One residual fixture: the source files, the call snippet naming a site in
/// `main.js`, and the exact resolved target texts at that site.
type ResidualFixture<'a> = (&'a [(&'a str, &'a str)], &'a str, &'a [&'a str]);

/// The residual value-flow repairs, one fixture each, asserting the exact
/// resolved target set at a single call site.
#[test]
fn residual_value_flow_fixtures_resolve_exactly() {
    let cases: &[ResidualFixture<'_>] = &[
        // A call site's span absorbs the callee's grouping parentheses, so a
        // model row keyed on the bare Oxc call span must still find it.
        (
            &[(
                "main.js",
                "const creator = (() => ({val: () => {}}));\nvar x = ( creator());\nconst q = \"val\";\n(x[q] ());\n",
            )],
            "( creator())",
            &["() => ({val: () => {}})"],
        ),
        // …and the object that call returns flows into `x`, so the computed
        // constant-key call resolves.
        (
            &[(
                "main.js",
                "const creator = (() => ({val: () => {}}));\nvar x = ( creator());\nconst q = \"val\";\n(x[q] ());\n",
            )],
            "(x[q] ())",
            &["() => {}"],
        ),
        // A returned object is as reachable inside one module as across a
        // `require` boundary.
        (
            &[(
                "main.js",
                "function make() { return {m: () => {}}; }\nmake().m();\n",
            )],
            "make().m()",
            &["() => {}"],
        ),
        // `reduce` with no initial value over a single-element array returns
        // that element without ever calling the reducer.
        (
            &[("main.js", "[() => { return 1; }].reduce(() => void 0)();\n")],
            "[() => { return 1; }].reduce(() => void 0)()",
            &["() => { return 1; }"],
        ),
        // …and over an empty array it returns the initial value.
        (
            &[(
                "main.js",
                "[].reduce(() => void 0, () => { return 2; })();\n",
            )],
            "[].reduce(() => void 0, () => { return 2; })()",
            &["() => { return 2; }"],
        ),
        // `new` on a callable *value*, not a class name.
        (
            &[("main.js", "var F = function () {};\nlet t = new F;\n")],
            "new F",
            &["function () {}"],
        ),
        // A native collection callback reached by value: the array literal's
        // elements flow into the parameter of the arrow the caller passed.
        (
            &[(
                "main.js",
                "function doit(f) {\n    [() => { return 3; }].forEach(f);\n}\ndoit(g => g());\n",
            )],
            "g()",
            &["() => { return 3; }"],
        ),
        // A class is a callable value: `exports.default = T` exports its
        // constructor, so `new lib.default()` reaches the class.
        (
            &[
                (
                    "main.js",
                    "const lib = require('./lib.js');\nnew lib.default();\n",
                ),
                ("lib.js", "class T {}\nexports.default = T;\n"),
            ],
            "new lib.default()",
            &["class T {}"],
        ),
    ];
    for (files, call, expected) in cases {
        assert_eq!(
            resolved_targets_at(files, call),
            expected
                .iter()
                .map(|text| (*text).to_string())
                .collect::<BTreeSet<_>>(),
            "{call:?} in {}",
            files[0].1
        );
    }

    // Negative controls: nothing may be invented where the value is unknown.
    for (files, call) in [
        // Reducing an empty array with no initial value throws at runtime.
        // The reducer returns a *callable*: with a non-callable return this
        // control passed no matter what the model did with the reducer.
        (
            [("main.js", "[].reduce(() => () => 0)();\n")],
            "[].reduce(() => () => 0)()",
        ),
        // A non-callable element and a non-callable reducer result.
        (
            [("main.js", "[1].reduce(() => 0)();\n")],
            "[1].reduce(() => 0)()",
        ),
        // `new` on an unknown value.
        ([("main.js", "new Unknown();\n")], "new Unknown()"),
        // A collection callback that is not callable here.
        (
            [(
                "main.js",
                "function doit(f) {\n    [() => { return 3; }].forEach(f);\n}\ndoit(0);\nfunction other(g) { g(); }\n",
            )],
            "g()",
        ),
    ] {
        assert!(
            resolved_targets_at(&files, call).is_empty(),
            "{call:?} must stay unresolved in {}",
            files[0].1
        );
    }
}

/// Jelly names an object shorthand method by its key *contents*: the function
/// span of `{ "m"() {} }` starts at `m`, not at the opening quote. The
/// FunctionFact, the MIR body and the model rows must all use that span or
/// the edge is scored against a function the oracle does not have.
#[test]
fn string_keyed_object_method_span_starts_at_the_key_contents() {
    let source = "const o = { \"m\"() {} };\no.m();\n";
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join("main.js"), source).unwrap();
    let output = crate::eval::observed::run_kernel_for_repo_for_test(repo.path()).unwrap();
    let method = output
        .db
        .functions()
        .iter()
        .find(|function| {
            source[function.span.start_byte as usize..function.span.end_byte as usize]
                .ends_with("() {}")
        })
        .expect("the shorthand method has a function fact");
    assert_eq!(
        method.span.start_byte as usize,
        source.find("m\"() {}").unwrap(),
        "span must start at the key contents, not the quote"
    );
    assert_eq!(
        resolved_targets_at(&[("main.js", source)], "o.m()"),
        BTreeSet::from(["m\"() {}".to_string()])
    );
}

#[test]
fn recovered_parse_does_not_seed_callable_shape_models() {
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join("main.js"), "function target() {} const list = [target]; for (const callable of list) callable(); const = broken;").unwrap();
    let output = crate::eval::observed::run_kernel_for_repo_for_test(repo.path()).unwrap();
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "parser/ts")
    );
    let interner = output.db.stable_key_interner();
    assert!(output.db.semantic_constraints().iter().all(|constraint| {
        !interner
            .resolve(constraint.stable_key)
            .contains("callable_shape")
    }));
}
/// A valid but *uncalled* self-recursive object method must not take the
/// process down. The all-callable return-summary pass walks every callable
/// the file declares, so it reaches `f` even though nothing invokes it, and
/// the read-only `object_targets_from_call` ->
/// `object_targets_from_return_expression` chain follows `o.f()` back into
/// `f` forever. The `&mut self` `invocation_depth` counter cannot bound that
/// chain because every hop on it borrows `&self`.
///
/// Runs the analysis in a child process: a stack overflow aborts with
/// SIGABRT rather than unwinding, so an in-process assertion would be killed
/// alongside the regression it is meant to report.
#[test]
fn recursive_object_methods_do_not_abort_analysis() {
    const CHILD_SOURCE: &str = "POLINT_RECURSIVE_CALLABLE_CHILD_SOURCE";
    if let Ok(source) = std::env::var(CHILD_SOURCE) {
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("main.js"), source).unwrap();
        crate::eval::observed::run_kernel_for_repo_for_test(repo.path())
            .expect("the kernel completes over a recursive callable shape");
        return;
    }
    for source in [
        // Direct self-recursion through the receiver the method lives on.
        "const o = { f() { return o.f(); } };\n",
        // Two-object mutual recursion.
        "const a = { f() { return b.g(); } };\nconst b = { g() { return a.f(); } };\n",
        // A deeper three-object cycle, so a fixed small depth cap that merely
        // happens to clear the two-object shape still fails here.
        "const a = { f() { return b.g(); } };\nconst b = { g() { return c.h(); } };\nconst c = { h() { return a.f(); } };\n",
        // The cycle closes through a returned local rather than a direct call.
        "const o = { f() { const next = o.f(); return next; } };\n",
        // Reading the *shape* of the returned value takes the second,
        // read-only cycle: `object_targets_from_call` follows `f`'s returned
        // call back into `f` looking for the object `x` would be.
        "const o = { f() { return o.f(); } };\nconst x = o.f();\nx.m();\n",
    ] {
        let child = std::process::Command::new(
            std::env::current_exe().expect("the running test binary"),
        )
        .args([
            "--exact",
            "ts::constraints::callable_values::recursive_object_methods_do_not_abort_analysis",
            "--nocapture",
        ])
        .env(CHILD_SOURCE, source)
        .output()
        .expect("spawn the analysis child");
        assert!(
            child.status.success(),
            "analysis must terminate for {source:?}, got {:?}\n{}{}",
            child.status,
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr),
        );
    }
}
/// A callee's parameters shadow the scope its body is walked in. Two walks
/// inherit an outer scope and used to let a parameter fall through to the
/// module symbol it shadows: the argument-independent return summary, which
/// clones the module env, and the speculative class-body walk, which
/// enumerated only plain identifier parameters and so missed every
/// destructured and rest name.
///
/// Each case pairs a module-level definition with a parameter of the same
/// name. Asserting the exact resolved set makes every case a precision
/// control: the shadowed module symbol must not appear, whether or not the
/// argument flow reaches the real one.
#[test]
fn parameters_shadow_module_scope_in_every_binding_form() {
    // (source, call snippet, exact resolved targets, what it used to report)
    let cases: &[(&str, &str, &[&str], &str)] = &[
        // Return summary, simple parameter. `identity` returns its
        // parameter, so its summary must not claim it returns the module's
        // `callback` — that invalid summary used to overwrite the
        // argument-dependent answer at the call site.
        (
            "const callback = function wrong() {};\nfunction identity(callback) { return callback; }\nconst result = identity(function right() {});\nresult();\n",
            "result()",
            &["function right() {}"],
            "function wrong() {}",
        ),
        // Return summary, destructured parameter.
        (
            "const callback = function wrong() {};\nfunction identity({callback}) { return callback; }\nconst result = identity({callback: function right() {}});\nresult();\n",
            "result()",
            &["function right() {}"],
            "function wrong() {}",
        ),
        // Return summary, rest parameter.
        (
            "const callback = function wrong() {};\nfunction identity(...callback) { return callback[0]; }\nconst result = identity(function right() {});\nresult();\n",
            "result()",
            &["function right() {}"],
            "function wrong() {}",
        ),
        // Constructor and method parameters receive arguments through
        // the row call, including object and array destructuring. Rest
        // bindings remain open without positional element rows.
        (
            "function callback() {}\nclass C {\n  constructor({callback}) { callback(); }\n}\nnew C({callback: function right() {}});\n",
            "callback();",
            &["function right() {}"],
            "function callback() {}",
        ),
        // Class constructor, array-destructured parameter.
        (
            "function callback() {}\nclass C {\n  constructor([callback]) { callback(); }\n}\nnew C([function right() {}]);\n",
            "callback();",
            &["function right() {}"],
            "function callback() {}",
        ),
        // Class constructor, rest parameter.
        (
            "function callback() {}\nclass C {\n  constructor(...callback) { callback[0](); }\n}\nnew C(function right() {});\n",
            "callback[0]();",
            &[],
            "function callback() {}",
        ),
        // A destructured parameter on an ordinary method, not just the
        // constructor.
        (
            "function callback() {}\nclass C {\n  m({callback}) { callback(); }\n}\nnew C().m({callback: function right() {}});\n",
            "callback();",
            &["function right() {}"],
            "function callback() {}",
        ),
        // Controls: the plain identifier and default-valued forms were
        // already shadowed, and must stay that way.
        (
            "function callback() {}\nclass C {\n  constructor(callback) { callback(); }\n}\nnew C(function right() {});\n",
            "callback();",
            &["function right() {}"],
            "(already unresolved)",
        ),
        (
            "function callback() {}\nclass C {\n  constructor(callback = function dflt() {}) { callback(); }\n}\nnew C(function right() {});\n",
            "callback();",
            &["function right() {}", "function dflt() {}"],
            "(already unresolved)",
        ),
    ];
    for (source, call, expected, previously) in cases {
        assert_eq!(
            resolved_targets_at(&[("main.js", source)], call),
            expected
                .iter()
                .map(|text| (*text).to_string())
                .collect::<BTreeSet<_>>(),
            "{call:?} in {source} (previously reported {previously})"
        );
    }
}
/// The CommonJS interop preambles branch on the module's `__esModule`
/// marker, not on whether it happens to expose a `default` property. Reading
/// one for the other is wrong in both directions, so each case below asserts
/// the exact resolved set: an unmarked module is wrapped even when it exports
/// `default`, and the wrapper nests the whole namespace under `default` even
/// when the module value is not callable.
#[test]
fn commonjs_interop_follows_the_es_module_marker() {
    const HELPERS: &str = "var __importDefault = (this && this.__importDefault) || function (mod) { return (mod && mod.__esModule) ? mod : { \"default\": mod }; };\nvar __importStar = (this && this.__importStar) || function (mod) { if (mod && mod.__esModule) return mod; var r = {}; for (var k in mod) r[k] = mod[k]; r.default = mod; return r; };\n";
    // A transpiled ES module: marked, with both a default and a named export.
    const MARKED: &str = "Object.defineProperty(exports, \"__esModule\", { value: true });\nexports.default = function foo() {};\nexports.m = function m() {};\n";
    // Unmarked CommonJS that happens to export `default`.
    const UNMARKED_DEFAULT: &str = "exports.default = function target() {};\n";
    // Unmarked CommonJS property bag.
    const UNMARKED_BAG: &str = "exports.m = function m() {};\n";
    // Unmarked CommonJS whose module *value* is callable.
    const UNMARKED_FN: &str = "function target() {}\nmodule.exports = target;\n";

    // (lib.js, main.js tail after the preamble, call snippet, exact targets, note)
    let cases: &[(&str, &str, &str, &[&str], &str)] = &[
        // Unmarked, so the helper wraps: `lib.default` is the namespace
        // object, and calling it throws. Reported `target` before.
        (
            UNMARKED_DEFAULT,
            "const lib = __importDefault(require('./lib.js'));\nlib.default();\n",
            "lib.default()",
            &[],
            "invented a target for a call that throws",
        ),
        // The call that *is* reachable on that module: through the wrapper's
        // `default` and then the module's own.
        (
            UNMARKED_DEFAULT,
            "const lib = __importDefault(require('./lib.js'));\nlib.default.default();\n",
            "lib.default.default()",
            &["function target() {}"],
            "missed the reachable call",
        ),
        // A property bag has no callable module value, but `default` still
        // holds the namespace, so this member call is valid.
        (
            UNMARKED_BAG,
            "const lib = __importDefault(require('./lib.js'));\nlib.default.m();\n",
            "lib.default.m()",
            &["function m() {}"],
            "left the namespace unnested",
        ),
        // The default helper returns `{ default: mod }` and nothing else, so
        // the namespace's own members are not on the wrapper.
        (
            UNMARKED_BAG,
            "const lib = __importDefault(require('./lib.js'));\nlib.m();\n",
            "lib.m()",
            &[],
            "exposed a member the default helper does not copy",
        ),
        // The star helper does copy them — the one place the two helpers
        // disagree.
        (
            UNMARKED_BAG,
            "const lib = __importStar(require('./lib.js'));\nlib.m();\n",
            "lib.m()",
            &["function m() {}"],
            "unchanged",
        ),
        (
            UNMARKED_BAG,
            "const lib = __importStar(require('./lib.js'));\nlib.default.m();\n",
            "lib.default.m()",
            &["function m() {}"],
            "left the namespace unnested",
        ),
        // Marked: both helpers return the namespace unchanged.
        (
            MARKED,
            "const lib = __importDefault(require('./lib.js'));\nlib.default();\n",
            "lib.default()",
            &["function foo() {}"],
            "unchanged",
        ),
        (
            MARKED,
            "const lib = __importDefault(require('./lib.js'));\nlib.m();\n",
            "lib.m()",
            &["function m() {}"],
            "unchanged",
        ),
        // Unmarked with a callable module value: `default` reaches it.
        (
            UNMARKED_FN,
            "const lib = __importDefault(require('./lib.js'));\nlib.default();\n",
            "lib.default()",
            &["function target() {}"],
            "unchanged",
        ),
        // Babel's older output requires into a variable first, then wraps
        // that variable. The marker lives on the module, not on the
        // expression handed to the helper, so this shape has to land on the
        // same branch as the inline `require(...)` form.
        (
            MARKED,
            "const mod = require('./lib.js');\nconst lib = __importDefault(mod);\nlib.default();\n",
            "lib.default()",
            &["function foo() {}"],
            "read the marker off the variable's module",
        ),
        (
            UNMARKED_BAG,
            "const mod = require('./lib.js');\nconst lib = __importDefault(mod);\nlib.default.m();\n",
            "lib.default.m()",
            &["function m() {}"],
            "same wrapping decision through a variable",
        ),
        // ...and the impossible call stays impossible through a variable too.
        (
            UNMARKED_DEFAULT,
            "const mod = require('./lib.js');\nconst lib = __importDefault(mod);\nlib.default();\n",
            "lib.default()",
            &[],
            "the wrapped namespace is still not callable",
        ),
        // A parameter shadowing the module name must not inherit its
        // identity: the helper then has no module to read a marker from.
        (
            MARKED,
            "const mod = require('./lib.js');\nfunction use(mod) { return __importDefault(mod); }\nconst lib = use(0);\nlib.default();\n",
            "lib.default()",
            &[],
            "a shadowed module name carries no marker",
        ),
    ];
    for (lib, tail, call, expected, note) in cases {
        let main = format!("{HELPERS}{tail}");
        assert_eq!(
            // Flow-insensitive rows retain both helper branches. A call
            // that throws may therefore carry a heuristic candidate, but
            // must never claim an exact or conservative target.
            if expected.is_empty() {
                claimed_targets_at(&[("main.js", &main), ("lib.js", lib)], call)
            } else {
                resolved_targets_at(&[("main.js", &main), ("lib.js", lib)], call)
            },
            expected
                .iter()
                .map(|text| (*text).to_string())
                .collect::<BTreeSet<_>>(),
            "{call:?} against {lib:?} ({note})"
        );
    }
}
/// `reduce`/`reduceRight` produce exactly one of three values, and which one
/// is decided by the array's length and whether an initial value was passed.
/// Unioning all three invents targets the program cannot produce: the reducer
/// does not run at all on an empty array, nor on a single-element array with
/// no initial value, and the elements are not the result once it does run.
///
/// Each case asserts the exact resolved set, so an invented target fails the
/// same way a lost one does. The reducers all return a callable — a reducer
/// returning a non-callable cannot catch this at all, since it contributes
/// nothing either way.
#[test]
fn reduce_results_follow_array_cardinality() {
    // (source, call snippet, exact targets, why)
    let cases: &[(&str, &str, &[&str], &str)] = &[
        // Empty and no initial value: `reduce` throws before the reducer runs.
        (
            "[].reduce(() => () => 2)();\n",
            "[].reduce(() => () => 2)()",
            &[],
            "an empty reduce throws; nothing is produced",
        ),
        (
            "[].reduceRight(() => () => 2)();\n",
            "[].reduceRight(() => () => 2)()",
            &[],
            "reduceRight throws on the same input",
        ),
        // One element and no initial value: that element is returned and the
        // reducer never runs.
        (
            "[() => 1].reduce(() => () => 2)();\n",
            "[() => 1].reduce(() => () => 2)()",
            &["() => 1"],
            "the lone element is the result; the reducer never runs",
        ),
        (
            "[() => 1].reduceRight(() => () => 2)();\n",
            "[() => 1].reduceRight(() => () => 2)()",
            &["() => 1"],
            "same, from the right",
        ),
        // Two elements: the reducer runs and its return is the result, so the
        // elements are not.
        (
            "[() => 1, () => 3].reduce(() => () => 2)();\n",
            "[() => 1, () => 3].reduce(() => () => 2)()",
            &["() => 2"],
            "the reducer runs, so its return is the only result",
        ),
        // Empty with an initial value: that value is returned untouched.
        (
            "[].reduce(() => () => 2, () => 4)();\n",
            "[].reduce(() => () => 2, () => 4)()",
            &["() => 4"],
            "the initial value passes through",
        ),
        // One element with an initial value: the reducer runs once.
        (
            "[() => 1].reduce(() => () => 2, () => 4)();\n",
            "[() => 1].reduce(() => () => 2, () => 4)()",
            &["() => 2"],
            "an initial value makes the reducer run at length 1",
        ),
        // Length not statically known: every branch stays possible, so the
        // conservative union is kept.
        (
            "function pick(a) { return a; }\nconst arr = pick([() => 1]);\narr.reduce(() => () => 2)();\n",
            "arr.reduce(() => () => 2)()",
            &["() => 1", "() => 2"],
            "an unknown length keeps the union",
        ),
        (
            "const rest = [() => 5];\n[...rest].reduce(() => () => 2)();\n",
            "[...rest].reduce(() => () => 2)()",
            &["() => 2", "() => 5"],
            "a spread contributes an unknown element count",
        ),
        // A receiver that defines its own `reduce` is not an array. This
        // branch runs before ordinary method handling, so without the guard
        // it swallowed the call and answered with the array model.
        (
            "const o = { reduce(f) { return f; } };\no.reduce(() => 1)();\n",
            "o.reduce(() => 1)()",
            &["() => 1"],
            "a custom reduce returns its argument",
        ),
        (
            "const o = { reduce(f) { return () => 7; } };\no.reduce(() => () => 2)();\n",
            "o.reduce(() => () => 2)()",
            &["() => 7"],
            "a custom reduce's own return, not the reducer's",
        ),
    ];
    for (source, call, expected, why) in cases {
        assert_eq!(
            resolved_targets_at(&[("main.js", source)], call),
            expected
                .iter()
                .map(|text| (*text).to_string())
                .collect::<BTreeSet<_>>(),
            "{call:?}: {why}"
        );
    }
}

/// A returned or reassigned callable is answered with every source that can
/// produce it, or with nothing when one of those sources is not followed: an
/// empty answer leaves the call unresolved, while a partial one would report
/// a call as complete that reaches further targets at run time.
#[test]
fn returned_callables_are_complete_or_left_unresolved() {
    const HEADER: &str = "function a() {}\nfunction b() {}\nfunction c() {}\n";
    // (source after the shared declarations, call snippet, exact targets, why)
    let cases: &[(&str, &str, &[&str], &str)] = &[
        (
            "function pick(k, fn) {\n  if (k === 1) return a;\n  if (k === 2) return b;\n  return fn;\n}\nconst f = pick(3, c);\nf();\n",
            "f();",
            &["function a() {}", "function b() {}", "function c() {}"],
            "every branch return is part of the answer, not the first that resolves",
        ),
        (
            "function resolve(k, fn) {\n  if (k) return a;\n  return fn;\n}\nfunction apply(k, fn) {\n  fn = resolve(k, fn);\n  return fn();\n}\napply(1, b);\n",
            "fn();",
            &["function a() {}", "function b() {}"],
            "a reassigned parameter joins the value it is replaced with",
        ),
        (
            "function apply(fn) {\n  fn = wrap(fn);\n  return fn();\n}\napply(a);\n",
            "fn();",
            &[],
            "a reassignment from an unknown call drops the stale parameter binding",
        ),
        (
            "function apply(fn) {\n  fn = fn || b;\n  return fn();\n}\napply(a);\n",
            "fn();",
            &["function a() {}", "function b() {}"],
            "an `||` joins both operands: the value flow does not follow truthiness, so the default joins the bound function",
        ),
        (
            "function pick(k, fn) {\n  if (k) return a;\n  return lookup(fn);\n}\nconst f = pick(1, b);\nf();\n",
            "f();",
            &[],
            "a return this walk cannot follow leaves the call unresolved",
        ),
        (
            "function pick(k, fn) {\n  switch (k) {\n    case 1:\n      return fn;\n  }\n  return a;\n}\nconst f = pick(1, b);\nf();\n",
            "f();",
            &["function a() {}", "function b() {}"],
            "a return inside a switch joins the later return",
        ),
        (
            "function pick(k, fn) {\n  if (k) return a;\n  return lookup(fn);\n}\nfunction main() {\n  const f = pick(1, b);\n  f();\n}\nmain();\n",
            "f();",
            &[],
            "an unknown return also blocks the returned value bound inside a function",
        ),
        (
            "function pick(k, fn) {\n  switch (k) {\n    case 1:\n      return fn;\n  }\n  return a;\n}\nfunction main() {\n  const f = pick(1, b);\n  f();\n}\nmain();\n",
            "f();",
            &["function a() {}", "function b() {}"],
            "a returned parameter joins the other returns of the function, inside a caller too",
        ),
        (
            "function make() {\n  let g = a;\n  g = wrap(g);\n  return g;\n}\nfunction main() {\n  const f = make();\n  f();\n}\nmain();\n",
            "f();",
            &[],
            "a reassigned local alias no longer names its first value",
        ),
        (
            "function pick(k) {\n  if (k) return a;\n  return null;\n}\nconst f = pick(1);\nf();\n",
            "f();",
            &["function a() {}"],
            "a returned primitive cannot be called, so it does not block the answer",
        ),
        (
            "function apply(fn) {\n  var fn = lookup();\n  return fn();\n}\napply(a);\n",
            "fn();",
            &[],
            "a `var` redeclaring a parameter replaces its value",
        ),
        (
            "function apply(fn) {\n  return fn();\n  function fn() {}\n}\napply(a);\n",
            "fn();",
            &["function a() {}", "function fn() {}"],
            "a function declaration named like a parameter shares its binding; the value flow does not order the hoisted declaration after the argument, so the argument joins it",
        ),
        (
            "function apply(fn) {\n  if (flag) {\n    function fn() {}\n  }\n  return fn();\n}\napply(a);\n",
            "fn();",
            &["function a() {}"],
            "a function declaration in a block named like a parameter binds the block's name, not the parameter",
        ),
        (
            "function apply(fn, fns) {\n  for (var fn of fns) {}\n  return fn();\n}\napply(a, [lookup()]);\n",
            "fn();",
            &[],
            "a `var` in a `for…of` head writes the parameter it redeclares",
        ),
        (
            "function apply(fn) {\n  if (flag) {\n    var fn = b;\n  }\n  return fn();\n}\napply(a);\n",
            "fn();",
            &["function a() {}", "function b() {}"],
            "a `var` redeclaration that may not run joins the parameter's value",
        ),
        (
            "function pick(k) {\n  if (k) return a;\n  return lookup();\n}\npick(1)();\n",
            "pick(1)();",
            &[],
            "a call through a call result with an unfollowed return stays unresolved",
        ),
        (
            "var o = {\n  get f() {\n    if (flag) return a;\n    return lookup();\n  },\n};\no.f();\n",
            "o.f();",
            &[],
            "a getter with an unfollowed return leaves its call unresolved",
        ),
        (
            "var o = {\n  get f() {\n    if (flag) return a;\n    return lookup();\n  },\n};\no[\"f\"]();\n",
            "o[\"f\"]();",
            &[],
            "a computed getter call with an unfollowed return stays unresolved",
        ),
        (
            "function f1() {\n  if (flag) return b;\n  return f2();\n}\nfunction f2() {\n  return f3();\n}\nfunction f3() {\n  return f4();\n}\nfunction f4() {\n  return f5();\n}\nfunction f5() {\n  return f6();\n}\nfunction f6() {\n  return f7();\n}\nfunction f7() {\n  return f8();\n}\nfunction f8() {\n  return f9();\n}\nfunction f9() {\n  return f10();\n}\nfunction f10() {\n  return f11();\n}\nfunction f11() {\n  return f12();\n}\nfunction f12() {\n  return f13();\n}\nfunction f13() {\n  return f14();\n}\nfunction f14() {\n  return f15();\n}\nfunction f15() {\n  return f16();\n}\nfunction f16() {\n  return f17();\n}\nfunction f17() {\n  return f18();\n}\nfunction f18() {\n  return f19();\n}\nfunction f19() {\n  return f20();\n}\nfunction f20() {\n  return a;\n}\nf1()();\n",
            "f1()();",
            &["function a() {}", "function b() {}"],
            "the value flow follows a return chain of any depth",
        ),
        (
            "function main() {\n  var o = {\n    m() {\n      return lookup();\n    },\n  };\n  var f = a;\n  {\n    let f = o.m();\n    f();\n  }\n}\nmain();\n",
            "f();",
            &[],
            "a method whose only return is unfollowed does not keep a shadowed binding",
        ),
    ];
    // TypeScript assignment targets may wrap the written name.
    let ts_cases: &[(&str, &str, &[&str], &str)] = &[
        (
            "function make() {\n  let g = a;\n  (g as any) = wrap(g);\n  return g;\n}\nfunction main() {\n  const f = make();\n  f();\n}\nmain();\n",
            "f();",
            &[],
            "a write through a type assertion still replaces the local alias",
        ),
        (
            "function make() {\n  let g = a;\n  g! = wrap(g);\n  return g;\n}\nfunction main() {\n  const f = make();\n  f();\n}\nmain();\n",
            "f();",
            &[],
            "a write through a non-null assertion still replaces the local alias",
        ),
        (
            "function make() {\n  let g = a;\n  (g) = wrap(g);\n  return g;\n}\nfunction main() {\n  const f = make();\n  f();\n}\nmain();\n",
            "f();",
            &[],
            "a parenthesized write still replaces the local alias",
        ),
    ];
    let all_cases = cases
        .iter()
        .map(|case| ("main.js", case))
        .chain(ts_cases.iter().map(|case| ("main.ts", case)));
    for (path, (body, call, expected, why)) in all_cases {
        let source = format!("{HEADER}{body}");
        assert_eq!(
            // An open row call retains known operands as heuristic
            // candidates; only claim tiers count as resolved answers here.
            if expected.is_empty() {
                claimed_targets_at(&[(path, &source)], call)
            } else {
                resolved_targets_at(&[(path, &source)], call)
            },
            expected
                .iter()
                .map(|text| (*text).to_string())
                .collect::<BTreeSet<_>>(),
            "{call:?}: {why}"
        );
    }
}

/// Recursive calls add rows for their syntax, without expanding call trees.
fn recursive_rows(self_calls: usize) -> usize {
    let calls = (0..self_calls)
        .map(|index| format!("  walk(node.child{index});\n"))
        .collect::<String>();
    let source = format!("function walk(node) {{\n{calls}  return node;\n}}\nwalk({{}});\n");
    let file = crate::analysis_api::SourceFile::new(
        crate::internal_core::FileId::from_raw(0),
        std::path::PathBuf::from("main.js"),
        "main.js".to_string(),
        crate::internal_core::Language::TypeScript,
        source.into(),
        "test-hash".to_string(),
    );
    let flows = super::lower_ts_source_value_flows(&file);
    assert!(flows.lowered);
    assert_eq!(flows.calls.len(), self_calls + 1);
    flows.rows.len()
}

#[test]
fn recursive_member_calls_add_rows_linearly() {
    let single = recursive_rows(1);
    let double = recursive_rows(2);
    assert!(double > single);
    assert_eq!(recursive_rows(3) - double, double - single);
}

#[test]
fn many_recursive_member_calls_keep_the_row_count_linear() {
    let single = recursive_rows(1);
    let extra = recursive_rows(2) - single;
    for count in [17, 32] {
        assert_eq!(recursive_rows(count), single + (count - 1) * extra);
    }
}

/// Repeated calls, heap updates and deep call chains reach the same shared
/// binding places without re-entering the frontend's function bodies.
#[test]
fn repeated_calls_and_heap_updates_keep_their_callable_targets() {
    const PRELUDE: &str = "function a() {}\nfunction b() {}\n";
    let cases: &[(&str, &str, bool)] = &[
        (
            "a callee called three times with nothing changed between the calls",
            "function leaf(f) { f(); }\n\
             function use(f) { leaf(f); }\n\
             function main(f) { use(f); use(f); use(f); }\n\
             main(a);\n",
            true,
        ),
        (
            "a prototype method assigned between the second and third call",
            "function Foo() {}\n\
             function use() { const o = new Foo(); o.m(); }\n\
             function main() { use(); use(); Foo.prototype.m = a; use(); }\n\
             main();\n",
            false,
        ),
        (
            "prototype methods assigned during the recorded walk",
            "function Foo() {}\n\
             function use() {\n\
             \x20 const o = new Foo();\n\
             \x20 o.n4();\n\
             \x20 Foo.prototype.n4 = o.n3;\n\
             \x20 Foo.prototype.n3 = o.n2;\n\
             \x20 Foo.prototype.n2 = o.n1;\n\
             \x20 Foo.prototype.n1 = o.m;\n\
             \x20 Foo.prototype.m = a;\n\
             }\n\
             function main() { use(); use(); use(); use(); use(); use(); use(); }\n\
             main();\n",
            false,
        ),
        (
            "a class declared between the second and third call",
            "function use() { const o = new Foo(); o.m(); }\n\
             function main() { use(); use(); const Foo = class { m() {} }; use(); }\n\
             main();\n",
            false,
        ),
        (
            "a class with static members declared between the second and third call",
            "function use() { Foo.z = b; Foo.m(); }\n\
             function main() { use(); use(); const Foo = class { static m() {} }; use(); }\n\
             main();\n",
            false,
        ),
        (
            "a static member assigned between the second and third call",
            "class Foo {}\n\
             function use() { Foo.z = b; Foo.m(); }\n\
             function main() { use(); use(); Foo.m = a; use(); }\n\
             main();\n",
            false,
        ),
        (
            "the same callee at another invocation depth",
            &format!(
                "function leaf(f) {{ f(); }}\n\
                 function use() {{ leaf(a); }}\n\
                 {}function d15() {{ use(); }}\n\
                 function main() {{ d1(); d1(); use(); }}\n\
                 main();\n",
                (1..15)
                    .map(|depth| format!("function d{depth}() {{ d{}(); }}\n", depth + 1))
                    .collect::<String>()
            ),
            false,
        ),
        (
            "the same callee under another walk guard",
            "function use(x, k) { x.f(k); }\n\
             function main(bb) {\n\
             \x20 const o = { f(cb) { cb(); use(this, bb); use(this, bb); } };\n\
             \x20 o.f(a);\n\
             \x20 use(o, bb);\n\
             }\n\
             main(b);\n",
            false,
        ),
        (
            "the same callee under another super",
            "function main(pp) {\n\
             \x20 const t = { h() { super.m1(); } };\n\
             \x20 const r = { m3() { q.g(); (t || t).h(); } };\n\
             \x20 const r2 = { m3: r.m3 };\n\
             \x20 const q = { g() { r.m3(); (t || t).h(); (t || t).h(); } };\n\
             \x20 Object.setPrototypeOf(r, pp);\n\
             \x20 r2.m3();\n\
             \x20 q.g();\n\
             }\n\
             const p = { m1() {} };\n\
             main(p);\n",
            false,
        ),
        (
            "a callee that spends the walk budget, then a walk that needs it",
            &format!(
                "function use(x) {{ x.g1(x); }}\n\
                 function main(cb) {{\n\
                 \x20 const o = {{\n\
                 {}\x20   g13() {{}},\n\
                 \x20   mk() {{ return {{ run(j) {{ j(); }} }}; }},\n\
                 \x20   f(k) {{ use(o); o.mk().run(k); }},\n\
                 \x20 }};\n\
                 \x20 o.f(cb);\n\
                 \x20 o.f(cb);\n\
                 \x20 o.f(cb);\n\
                 }}\n\
                 main(a);\n",
                (1..13)
                    .map(|level| format!(
                        "    g{level}(y) {{ y.g{next}(y); y.g{next}(y); }},\n",
                        next = level + 1
                    ))
                    .collect::<String>()
            ),
            true,
        ),
    ];
    let expectations: &[(&str, &[&str])] = &[
        ("f()", &["function a() {}"]),
        ("o.m()", &["function a() {}"]),
        ("o.n4()", &["function a() {}"]),
        ("o.m()", &[]),
        ("Foo.m()", &[]),
        ("Foo.m()", &["function a() {}"]),
        ("f()", &["function a() {}"]),
        ("cb()", &["function a() {}", "function b() {}"]),
        ("r2.m3()", &["m3() { q.g(); (t || t).h(); }"]),
        ("j()", &["function a() {}"]),
    ];
    assert_eq!(cases.len(), expectations.len());
    for ((case, body, _), (call, expected)) in cases.iter().zip(expectations) {
        let source = format!("{PRELUDE}{body}");
        assert_eq!(
            if expected.is_empty() {
                claimed_targets_at(&[("main.js", &source)], call)
            } else {
                resolved_targets_at(&[("main.js", &source)], call)
            },
            expected
                .iter()
                .map(|text| (*text).to_string())
                .collect::<BTreeSet<_>>(),
            "{case}"
        );
    }
}

#[test]
fn an_unknown_computed_read_keeps_reachable_callback_parameters_open() {
    let source = "function a() {} function b() {} function invoke(fn) { fn(); } const table = {run: invoke}; table[unknownKey](b); invoke(a);";
    let files = [("main.js", source)];
    assert!(resolved_targets_at(&files, "fn()").contains("function a() {}"));
    assert!(claimed_targets_at(&files, "fn()").is_empty());
}
