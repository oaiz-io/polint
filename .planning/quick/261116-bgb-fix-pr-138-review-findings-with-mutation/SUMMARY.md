---
status: incomplete
updated: 2026-11-16
validation: passed
remaining: commit blocked by filesystem permissions
---

# PR #138 review-fix validation

Branch remains `perf/js-callgraph-speed`, based on `5efded2662d11df065037aa3a390fee746082569`. No commits, pushes, merges, or branch changes were made during this resume.

## Findings

1. Fixed indexed evidence payload retention with one-row streaming. `indexed_family_streams_large_payloads` passes; restoring the original 4096-row batching failed at 571,408,384 bytes of additional peak RSS against a 64 MiB limit. `indexed_family_emission_matches_full_sort_across_batches_and_headers` also passes on 8200 production-emitted rows with repeated IDs, distinct tie payloads, escaped/Unicode text and both headers.
2. Fixed unnecessary index construction with a shared lazy completeness index and deferred TS refinement indexes. `completeness_skips_call_site_index_without_unknown_edges` passes and the eager mutation failed at 5 builds versus 0. `aggregate_unknowns_include_policy_capabilities` passes and the per-collector mutation failed at 5 versus 1. `empty_and_go_only_refinements_skip_expensive_indexes` passes and eager TS indexing failed at 1 versus 0. The active unresolved TS test still checks its edge and callable-place key. Release empty/resolved microbenchmarks passed with zero unused expensive index builds; see BENCHMARK.md.
3. Fixed the insufficient error-order regression. `packed_parts_report_the_first_failing_row` places failures at 127, 191 and 2999 in different chunks of the first batch, plus later failures, and passes with 1, 2 and 4 workers. Reversing chunk-result selection failed with Err(2999) instead of Err(127). Production error ordering remains unchanged.

All six regression/equivalence checks passed again in the resumed Linux library suite. Original compiled mutation failures are preserved in mutation-logs/ and described in MUTATIONS.md. The four Rust source hashes exactly match SOURCE-SHA256.txt and the Linux validation snapshot.

## Benchmark

`make js-bench` passed, exit 0. Every report field except cost/version is identical to the committed PR-head baseline. Overall time: 163.368 s. Zod refined fact dump: 35.254 s, peak RSS 5,645,402,112 bytes (5.65 GB / 5.26 GiB); base: 8.401 s, 1,791,967,232 bytes. See BENCHMARK.md and validation-logs/. These local macOS ARM64 samples ran concurrently with Linux validation and are not directly comparable to the historical AX102 measurements.

The live PR body was read with gh and exactly matches the saved corrected body in /tmp/pr138-body.md: historical output equivalence is attributed, unavailable raw artifacts are identified, sorted JSON limitations are explicit, small-sample soundness claims are qualified, and the original memory claim is narrowed. Remote PR head remains the reviewed head.

## Checks

Final saved checks are green for fmt, MSRV, all five Clippy feature combinations with -D warnings, feature-aware library/CLI tests, parser dependency isolation, all-feature library tests, internal architecture, rustdoc -D warnings, cargo deny check all, canary, certification, public surface, determinism, semantic store, cargo install smoke and SARIF. See CHECKS.md. No source changed after these checks; hashes were verified during this resume.

Fresh Linux `cargo test --workspace --all-features --locked` PASSED, exit 0. The run includes 2854 passing library tests (21 ignored), all 193 CLI tests, all 11 golden tests, all remaining integrations, workspace packages, macro tests, and doc tests. TypeScript 5 was required with POLINT_REQUIRE_TS_TYPESCRIPT=1. The Linux snapshot exactly matches the final source checksums. Complete output and exit status are preserved in `validation-logs/workspace-tests-linux.log` and `validation-logs/workspace-tests-linux-exit.txt`. The earlier native macOS golden-cost failure is recorded in CHECKS.md; no golden costs or assertions were changed.

## Commit blocker

`git add crates/polint/src/analysis_neutral/evidence/provider.rs` failed with:

```
fatal: Unable to create '/Users/emilwareus/Development/polint-review/main/.git/worktrees/pr138-speed/index.lock': Operation not permitted
```

The Git metadata directory is outside the session's writable roots. Approval is unavailable. No workaround was attempted. Prepared per-finding commit messages, including observed mutation failures, are in COMMIT-MESSAGES.md; a fourth message covers measured benchmark costs and validation evidence. Changes remain uncommitted. Do not push or merge.

The required validation is complete. The task remains incomplete only because the requested commits cannot be created within the allowed filesystem roots. Prepared commit messages, including each observed mutation failure, are in COMMIT-MESSAGES.md. No finding was rejected.
