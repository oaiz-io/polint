//! Direct call resolution over real TS/JS sources, end to end through the
//! analysis kernel: each case names one call site and the exact set of
//! functions the direct resolver may give it a precise edge to. A few cases
//! check the refined edges of the same site, which every resolver feeds.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_neutral::calls::facts::{
    CallPrecision, CallProvenance, CallTargetStatus, ClaimTier,
};

/// Which edges of a call site a case reads.
#[derive(Clone, Copy)]
enum Edges {
    /// Precise native direct targets.
    Direct,
    /// Refined edges that claim a target in any tier.
    Refined,
}

/// The texts of the functions that `call` (a snippet unique in `main`) has
/// precise direct edges to.
fn direct_targets_at(files: &[(&str, &str)], main: &str, call: &str) -> BTreeSet<String> {
    targets_at(files, main, call, Edges::Direct)
}

/// The texts of the functions a refined edge at `call` claims, in any tier.
fn refined_targets_at(files: &[(&str, &str)], main: &str, call: &str) -> BTreeSet<String> {
    targets_at(files, main, call, Edges::Refined)
}

fn targets_at(files: &[(&str, &str)], main: &str, call: &str, edges: Edges) -> BTreeSet<String> {
    let repo = tempfile::tempdir().unwrap();
    for (name, body) in files {
        std::fs::write(repo.path().join(name), body).unwrap();
    }
    let sources = files.iter().copied().collect::<BTreeMap<_, _>>();
    let source = sources[main];
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
    let main_file = output
        .db
        .files()
        .iter()
        .find(|file| file.relative_path == main)
        .unwrap();
    let start = source.find(call).unwrap() as u32;
    let site = output
        .db
        .call_sites()
        .iter()
        .filter(|site| {
            site.file == main_file.id
                && site.span.start_byte == start
                && site.span.end_byte <= start + call.len() as u32
        })
        .max_by_key(|site| site.span.end_byte)
        .unwrap_or_else(|| panic!("no call site for {call:?} in {source}"));
    let targets = match edges {
        Edges::Direct => output
            .db
            .call_targets()
            .iter()
            .filter(|target| {
                target.site == site.id
                    && target.status == CallTargetStatus::Resolved
                    && target.provenance == CallProvenance::NativeDirect
                    && target.precision != CallPrecision::Heuristic
            })
            .filter_map(|target| target.target_function)
            .collect::<Vec<_>>(),
        Edges::Refined => output
            .db
            .refined_call_edges()
            .iter()
            .filter(|edge| {
                edge.site == site.id
                    && edge.status == CallTargetStatus::Resolved
                    && edge.precision.tier() != ClaimTier::NoClaim
            })
            .filter_map(|edge| edge.target_function)
            .collect(),
    };
    targets
        .into_iter()
        .filter_map(|target| {
            let function = functions.get(&target)?;
            let body = sources[paths.get(&function.file)?.as_str()];
            Some(
                body[function.span.start_byte as usize..function.span.end_byte as usize]
                    .to_string(),
            )
        })
        .collect()
}

fn set(texts: &[&str]) -> BTreeSet<String> {
    texts.iter().map(|text| (*text).to_string()).collect()
}

const UTIL: (&str, &str) = ("util.ts", "export function parse() { return \"util\"; }\n");
const UTIL_PARSE: &str = "function parse() { return \"util\"; }";

/// The namespace member callee is recorded from the AST, so whatever separates
/// `ns` from `.parse` the inner call reaches `util.parse` and the outer
/// `obj.parse` never does.
#[test]
fn namespace_member_calls_are_matched_by_callee_span() {
    for separator in [" ", "/* note */", "\n", "\u{2028}", " \t\u{a0}"] {
        let main = format!(
            "import * as ns from \"./util\";\n\
             const obj = {{ parse(value: unknown) {{ return value; }} }};\n\
             obj.parse(ns{separator}.parse());\n"
        );
        let files = [UTIL, ("main.ts", main.as_str())];
        let outer = format!("obj.parse(ns{separator}.parse())");
        let inner = format!("ns{separator}.parse()");

        assert!(
            !direct_targets_at(&files, "main.ts", &outer).contains(UTIL_PARSE),
            "separator {separator:?}"
        );
        assert_eq!(
            direct_targets_at(&files, "main.ts", &inner),
            set(&[UTIL_PARSE]),
            "separator {separator:?}"
        );
    }
}

#[test]
fn non_ascii_source_resolves_without_slicing_inside_a_character() {
    let main = "import * as ns from \"./util\";\n\
                const café = () => \"é\";\u{2028}café();\n\
                const ünïcode = ns.parse();\n";
    let files = [UTIL, ("main.ts", main)];

    assert_eq!(
        direct_targets_at(&files, "main.ts", "café();"),
        set(&["() => \"é\""])
    );
    assert_eq!(
        direct_targets_at(&files, "main.ts", "ns.parse()"),
        set(&[UTIL_PARSE])
    );
}

/// An interface merged with a function in an inner scope names that scope's
/// function, not a same-named function outside it.
#[test]
fn interface_merged_function_resolves_in_its_own_scope() {
    let main = "function make() { return \"outer\"; }\n\
                function run() {\n\
                \x20 interface make { x: number }\n\
                \x20 function make() { return \"inner\"; }\n\
                \x20 return make();\n\
                }\n\
                run();\n";

    assert_eq!(
        direct_targets_at(&[("main.ts", main)], "main.ts", "make();"),
        set(&["function make() { return \"inner\"; }"])
    );
}

/// A `let` merged with an interface keeps no single value once assigned.
#[test]
fn reassigned_let_merged_with_an_interface_has_no_precise_target() {
    let main = "interface run { x: number }\n\
                let run = () => 1;\n\
                run = other;\n\
                run();\n\
                function other() { return 2; }\n\
                export {};\n";

    assert_eq!(
        direct_targets_at(&[("main.ts", main)], "main.ts", "run();"),
        set(&[])
    );
}

/// An overload signature whose return type is an object literal ends in `}`
/// like a body; the call still reaches the implementation.
#[test]
fn overload_with_an_object_literal_return_type_targets_the_implementation() {
    let main = "function pick(x: string): { value: string };\n\
                function pick(x: any) { return { value: x }; }\n\
                pick(\"a\");\n";

    assert_eq!(
        direct_targets_at(&[("main.ts", main)], "main.ts", "pick(\"a\")"),
        set(&["function pick(x: any) { return { value: x }; }"])
    );
}

/// An overload signature has no body, so no call reaches it: every refined
/// edge, through an import, a namespace member or a local binding, names the
/// implementation alone.
#[test]
fn refined_edges_reach_the_implementation_of_an_overload() {
    let util = "export function issue(a: string, b: number): string;\n\
                export function issue(a: string): string;\n\
                export function issue(...args: any[]): string { return String(args[0]); }\n";
    let main = "import * as util from \"./util\";\n\
                import { issue } from \"./util\";\n\
                function pick(x: string): string;\n\
                function pick(x: any) { return x; }\n\
                export function run() {\n\
                \x20 util.issue(\"a\");\n\
                \x20 issue(\"b\");\n\
                \x20 pick(\"c\");\n\
                }\n";
    let files = [("util.ts", util), ("main.ts", main)];
    let implementation = "function issue(...args: any[]): string { return String(args[0]); }";

    assert_eq!(
        refined_targets_at(&files, "main.ts", "util.issue(\"a\")"),
        set(&[implementation])
    );
    assert_eq!(
        refined_targets_at(&files, "main.ts", "issue(\"b\")"),
        set(&[implementation])
    );
    assert_eq!(
        refined_targets_at(&files, "main.ts", "pick(\"c\")"),
        set(&["function pick(x: any) { return x; }"])
    );
}

/// `eval` bound to a parameter may still be the global `eval`, which can
/// assign `f` from a string.
#[test]
fn eval_parameter_leaves_a_local_let_without_a_precise_target() {
    let main = "function run(eval) {\n\
                \x20 let f = () => 1;\n\
                \x20 eval(\"f = () => 2\");\n\
                \x20 return f();\n\
                }\n\
                module.exports = run;\n";

    assert_eq!(
        direct_targets_at(&[("main.cjs", main)], "main.cjs", "f();"),
        set(&[])
    );
}

#[test]
fn with_statement_leaves_a_local_const_without_a_precise_target() {
    let main = "function outer(scope) {\n\
                \x20 const local = () => 1;\n\
                \x20 with (scope) { local(); }\n\
                }\n\
                outer({});\n";

    assert_eq!(
        direct_targets_at(&[("main.js", main)], "main.js", "local();"),
        set(&[])
    );
}

#[test]
fn reassigned_function_declaration_has_no_precise_target() {
    let main = "function make() { return 1; }\n\
                make = function () { return 2; };\n\
                make();\n\
                export {};\n";

    assert_eq!(
        direct_targets_at(&[("main.js", main)], "main.js", "make();"),
        set(&[])
    );
}

/// In non-strict code a function declared in a block is also assigned to the
/// same-named `var` of the enclosing function when the block runs, so the
/// call after the block reaches the block's function, not the literal.
#[test]
fn sloppy_block_function_leaves_a_same_named_var_without_a_precise_target() {
    let main = "function run() {\n\
                \x20 var f = () => \"arrow\";\n\
                \x20 { function f() { return \"decl\"; } }\n\
                \x20 return f();\n\
                }\n\
                module.exports = run;\n";

    assert_eq!(
        direct_targets_at(&[("main.cjs", main)], "main.cjs", "f();"),
        set(&[])
    );
}

#[test]
fn sloppy_block_function_leaves_a_same_named_function_without_a_precise_target() {
    let nested = "function run() {\n\
                  \x20 function f() { return \"outer\"; }\n\
                  \x20 { function f() { return \"inner\"; } }\n\
                  \x20 return f();\n\
                  }\n\
                  module.exports = run;\n";
    let branches = "function run(x) {\n\
                    \x20 if (x) { function g() { return \"a\"; } } else { function g() { return \"b\"; } }\n\
                    \x20 return g();\n\
                    }\n\
                    module.exports = run;\n";

    assert_eq!(
        direct_targets_at(&[("main.cjs", nested)], "main.cjs", "f();"),
        set(&[])
    );
    assert_eq!(
        direct_targets_at(&[("main.cjs", branches)], "main.cjs", "g();"),
        set(&[])
    );
}

/// Strict code keeps a block's function inside its block, so the call after
/// the block still reaches the outer function.
#[test]
fn strict_block_function_leaves_the_outer_function_as_the_target() {
    let directive = "function run() {\n\
                     \x20 \"use strict\";\n\
                     \x20 function f() { return \"outer\"; }\n\
                     \x20 { function f() { return \"inner\"; } }\n\
                     \x20 return f();\n\
                     }\n\
                     module.exports = run;\n";
    let module = "function run() {\n\
                  \x20 function f() { return \"outer\"; }\n\
                  \x20 { function f() { return \"inner\"; } }\n\
                  \x20 return f();\n\
                  }\n\
                  export default run;\n";

    assert_eq!(
        direct_targets_at(&[("main.cjs", directive)], "main.cjs", "f();"),
        set(&["function f() { return \"outer\"; }"])
    );
    assert_eq!(
        direct_targets_at(&[("main.mjs", module)], "main.mjs", "f();"),
        set(&["function f() { return \"outer\"; }"])
    );
}

#[test]
fn call_inside_a_sloppy_block_reaches_the_block_function() {
    let main = "function run() {\n\
                \x20 function f() { return \"outer\"; }\n\
                \x20 { function f() { return \"inner\"; } f(); }\n\
                }\n\
                module.exports = run;\n";

    assert_eq!(
        direct_targets_at(&[("main.cjs", main)], "main.cjs", "f();"),
        set(&["function f() { return \"inner\"; }"])
    );
}

/// A `let` between the block and the function scope keeps the block's function
/// in its block, so the call after it reaches the file's `f`, which the
/// block's function must not stand in for.
#[test]
fn block_function_under_a_same_named_block_binding_has_no_precise_target() {
    let main = "function f() { return \"file\"; }\n\
                function run() {\n\
                \x20 { let f = 1; { function f() { return \"block\"; } } }\n\
                \x20 return f();\n\
                }\n\
                module.exports = run;\n";

    assert_eq!(
        direct_targets_at(&[("main.cjs", main)], "main.cjs", "f();"),
        set(&[])
    );
}

#[test]
fn named_function_expression_keeps_its_target_under_a_same_named_binding() {
    let main = "function run() {\n\
                \x20 let fact = 1;\n\
                \x20 return (function fact(n) { return n ? fact(n - 1) : fact; })(3);\n\
                }\n\
                module.exports = run;\n";

    assert_eq!(
        direct_targets_at(&[("main.cjs", main)], "main.cjs", "fact(n - 1)"),
        set(&["function fact(n) { return n ? fact(n - 1) : fact; }"])
    );
}
