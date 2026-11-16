# Fix PR #138 review findings

Scope: current perf/js-callgraph-speed branch only; commit fixes, no push or merge.

1. Restore one-row evidence payload streaming. Test production emission ordering/header merging and bounded payload retention, then restore batching to prove regression failure.
2. Share a lazy call-site index across completeness collectors. Defer TS refinement index construction until an eligible unresolved TS call exists. Test empty, Go-only, fully resolved and active refinement paths, with mutation proofs. Add empty/resolved benchmarks.
3. Strengthen first-error regression with several failures in different chunks of the first batch. Deliberately select a later chunk error and prove failure.
4. Run CI feature checks, fmt, clippy, full workspace tests, deny, docs, local executable gates and make js-bench. Compare every non-cost accuracy field against the pre-fix baseline. Record time and peak RSS. Correct unsupported PR-body claims via gh pr edit 138 --body-file.

Each fix commit records its regression and observed mutation result. Validation and benchmark evidence go in SUMMARY.md and the final reply.
