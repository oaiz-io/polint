//! Bounded frontend callable-shape facts, projected through the semantic graph.

use crate::analysis_neutral::ids::CallSiteId;
use crate::internal_core::FunctionId;

#[cfg(feature = "lang-typescript")]
mod extract;
#[cfg(feature = "lang-typescript")]
pub(super) use extract::cannot_be_callable;
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

#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
mod tests {
    use std::collections::BTreeMap;

    /// Callee bodies walked collecting the callable flows of a file whose one
    /// function calls itself at `self_calls` sites, counted on a one-thread
    /// pool so the walks of every pass are seen. A walk limit fails an
    /// exponential walk in bounded time.
    fn self_call_walks(self_calls: usize) -> usize {
        let calls = (0..self_calls)
            .map(|index| format!("  if (node.k === {index}) return walk(node.child{index});\n"))
            .collect::<String>();
        let source = format!("function walk(node) {{\n{calls}  return node;\n}}\nwalk({{}});\n");
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("main.js"), source).unwrap();
        let output = crate::eval::observed::run_kernel_for_repo_for_test(repo.path()).unwrap();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        pool.install(|| {
            let file = &output.db.files()[0];
            let arena = oxc_allocator::Allocator::default();
            let parsed = crate::ts::parse::parse_ts_file(&arena, file);
            let programs = BTreeMap::from([(file.id, parsed.program())]);
            super::extract::limit_body_walks(super::extract::body_walks() + 100_000);
            let before = super::extract::body_walks();
            super::collect_callable_flows(&output.db, &programs);
            super::extract::body_walks() - before
        })
    }

    /// A function that calls itself at `k` sites is not walked once per path
    /// through its call tree, about `k^d` walks for `d` levels of nesting,
    /// and not even once per level before keying starts: a recursive call is
    /// keyed from the start, so each level walks the body a bounded number of
    /// times and a further self-call adds a bounded number of walks.
    #[test]
    fn a_recursive_function_is_walked_a_bounded_number_of_times_per_level() {
        for self_calls in [1, 2, 4, 8] {
            let walks = self_call_walks(self_calls);
            assert!(walks > 0, "the self-call is followed into the callee body");
            assert!(
                walks <= 64 * (self_calls + 1),
                "{self_calls} self-calls walked {walks} callee bodies"
            );
        }
    }
}
