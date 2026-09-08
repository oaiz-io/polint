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
    use std::collections::BTreeSet;

    use crate::analysis_neutral::calls::facts::{CallPrecision, CallTargetStatus};

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
