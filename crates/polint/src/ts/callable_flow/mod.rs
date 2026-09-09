//! Bounded frontend callable-shape facts, projected through the semantic graph.

use crate::analysis_neutral::ids::CallSiteId;
use crate::internal_core::FunctionId;

#[cfg(feature = "lang-typescript")]
mod extract;
#[cfg(feature = "lang-typescript")]
pub(super) use extract::collect_callable_flows;

/// A bounded, heuristic callable flow extracted from the frontend AST. The graph
/// projection owns stable identity and solver constraints; this is not a call target.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct TsCallableFlow {
    pub(super) site: CallSiteId,
    pub(super) target_function: FunctionId,
    pub(super) kind: TsCallableFlowKind,
    pub(super) binding: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum TsCallableFlowKind {
    Token,
    Receiver,
}

// Every fixture below drives the kernel through `crate::eval::observed`, and
// that harness is itself gated on both language features (see `lib.rs`), so this
// module has to carry the identical condition — a `lang-typescript`-only build
// has no `crate::eval` to reach.
#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
mod tests {
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
                source[function.span.start_byte as usize..function.span.end_byte as usize]
                    .to_string()
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

    #[test]
    fn callable_models_reach_the_shared_solver_with_heuristic_precision() {
        let cases = [
            "const list = [target]; for (const callable of list) { callable(); }",
            "const box = { method: target }; const { method: callable } = box; callable();",
            "function pass(fn) { return fn; } const callable = pass(target); callable();",
            "const list = new Set([target]); for (const callable of list) { callable(); }",
            "const list = [target]; list.forEach(callable => callable());",
            "Promise.resolve(target).then(callable => callable());",
            "const box = { nested: { method: target } }; const { nested: { method: callable } } = box; callable();",
            "function* sequence() { yield target; } for (const callable of sequence()) { callable(); }",
        ];
        for suffix in cases {
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
            if edges.is_empty() {
                eprintln!("diagnostics: {:?}", output.diagnostics);
                eprintln!("functions: {:?}", output.db.functions());
                eprintln!("sites: {:?}", output.db.call_sites());
                eprintln!("nodes: {:?}", output.db.semantic_nodes());
                eprintln!("constraints: {:?}", output.db.semantic_constraints());
                let file = &output.db.files()[0];
                let arena = oxc_allocator::Allocator::default();
                let parsed = crate::ts::parse::parse_ts_file(&arena, file);
                let programs = std::collections::BTreeMap::from([(file.id, parsed.program())]);
                eprintln!(
                    "flows: {:?}",
                    super::collect_callable_flows(&output.db, &programs)
                );
            }
            assert_eq!(
                edges
                    .iter()
                    .filter_map(|edge| edge.target_function)
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from([target.id]),
                "{suffix}"
            );
            assert!(
                edges
                    .iter()
                    .all(|edge| edge.precision == CallPrecision::Heuristic),
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
                output.db.refined_call_edges().iter().all(|edge| {
                    edge.site != site.id || edge.status != CallTargetStatus::Resolved
                }),
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
            let sources =
                BTreeMap::from([("main.js", source.as_str()), ("helper.js", *helper_source)]);
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
                "const maybe = (globalThis.a && globalThis.b) || globalThis.c;\nmaybe();\n"
                    .to_string(),
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
                output.db.refined_call_edges().iter().all(|edge| {
                    edge.site != site.id || edge.status != CallTargetStatus::Resolved
                }),
                "a shadowed or non-callable guard must stay unresolved: {source}"
            );
        }
    }

    /// Resolved-target texts for the unique call site starting at `call` in
    /// `main.js`. Returning the whole set (not a membership check) makes every
    /// fixture below a precision control too: an extra target fails the same way
    /// a missing one does.
    fn resolved_targets_at(files: &[(&str, &str)], call: &str) -> BTreeSet<String> {
        let repo = tempfile::tempdir().unwrap();
        for (name, body) in files {
            std::fs::write(repo.path().join(name), body).unwrap();
        }
        let sources = files.iter().copied().collect::<BTreeMap<_, _>>();
        let source = sources["main.js"];
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
            .find(|file| file.relative_path == "main.js")
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
            .filter_map(|edge| {
                let target = functions.get(&edge.target_function?)?;
                let body = sources[paths.get(&target.file)?.as_str()];
                Some(
                    body[target.span.start_byte as usize..target.span.end_byte as usize]
                        .to_string(),
                )
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
                "ts::callable_flow::tests::recursive_object_methods_do_not_abort_analysis",
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
            // Class constructor, destructured parameter. The speculative class
            // walk has no arguments, so the parameter is genuinely unknown here
            // and uncertainty is the right answer — but the module's `callback`
            // is not it.
            (
                "function callback() {}\nclass C {\n  constructor({callback}) { callback(); }\n}\nnew C({callback: function right() {}});\n",
                "callback();",
                &[],
                "function callback() {}",
            ),
            // Class constructor, array-destructured parameter.
            (
                "function callback() {}\nclass C {\n  constructor([callback]) { callback(); }\n}\nnew C([function right() {}]);\n",
                "callback();",
                &[],
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
                &[],
                "function callback() {}",
            ),
            // Controls: the plain identifier and default-valued forms were
            // already shadowed, and must stay that way.
            (
                "function callback() {}\nclass C {\n  constructor(callback) { callback(); }\n}\nnew C(function right() {});\n",
                "callback();",
                &[],
                "(already unresolved)",
            ),
            (
                "function callback() {}\nclass C {\n  constructor(callback = function dflt() {}) { callback(); }\n}\nnew C(function right() {});\n",
                "callback();",
                &[],
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
                resolved_targets_at(&[("main.js", &main), ("lib.js", lib)], call),
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
}
