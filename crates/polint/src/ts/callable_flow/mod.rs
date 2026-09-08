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
    pub(super) caller: FunctionId,
    pub(super) target_function: FunctionId,
    pub(super) kind: TsCallableFlowKind,
    pub(super) binding: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum TsCallableFlowKind {
    Token,
    Receiver,
}

#[cfg(all(test, feature = "lang-typescript"))]
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
            let site = output
                .db
                .call_sites()
                .iter()
                .find(|site| site.span.start_byte == start)
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
                eprintln!("flows: {:?}", super::collect_callable_flows(&output.db, &programs));
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
}
