# Fundamental performance research: final report

Excalidraw (MIT, `0dbd2a39319d41fda37b2945dea0dcbd58d6a564`) deep analysis: 233.739 → 78.136 s (2.99×), warm median peak RSS 5.441 → 4.213 GiB (-22.6%). All headline wall/RSS values are medians of three fresh-process warm runs. The retained implementation leaves analysis semantics unchanged. No end-to-end 10× result was demonstrated.

gohugoio-hugo-scale Go deep request (Apache-2.0, `3f35721fb2c75a1f7cc5a7a14400b66e73d4b06e`): 312.883 → 91.422 s (3.42×); analysis-process peak RSS 4.855 → 4.862 GiB. Existing setup/provider failures constrain coverage; see the workload limitations below.

## Acceptance and accuracy

The original fixed gate remains **red**, inherited from d10432aa. The continuation explicitly authorizes a draft PR with that failure disclosed. Neither the baseline JSON nor the 0.005 tolerance was changed. This is a production-oriented performance candidate, with release readiness blocked by the inherited accuracy failure and a missing optional-corpus CLI snapshot. That snapshot is also absent at the starting revision; this failure is not a new analysis mismatch. The original all-green acceptance condition has not been met.

| Public suite / pinned SHA / license | Precision, before → after | Recall, before → after | F1, before → after | TP / FP / FN, unchanged | Unknowns, unchanged |
|---|---:|---:|---:|---:|---:|
| Jelly / b799ed4f0d68c670fe398830aaa51dd5c628cf74 / BSD-3-Clause | 0.968872 → 0.968872 | 0.168357 → 0.168357 | 0.286866 → 0.286866 | 249 / 8 / 1230 | 913 |
| golang/tools RTA / 7743a285e3d261ca235408e013ec5c14cb5170e4 / BSD-3-Clause | 0.049933 → 0.049933 | 1.000000 → 1.000000 | 0.095116 → 0.095116 | 37 / 704 / 0 | 0 |

Jelly's committed F1 is 0.790193; passing requires at least 0.785193. Go remains above its committed baseline. Graph coverage, false positives, recall and precision are unchanged by this patch. The 76 Jelly / 5 Go accuracy cases are distinct from whole-checkout performance workloads below. The gate's cost columns are not used as speed comparisons.

The rejected private callable-model restoration reached Jelly F1 0.765784, precision 0.963115, recall 0.635565 (TP 940, FP 36, FN 539). It failed the fixed gate and is **excluded from the commit and all speed claims**. Remaining identified gaps include class-constructor caller attribution and CommonJS default interop. Most focused callable fixtures passed; the interop fixture remained unresolved. The final identifier-name fixture repair was not corpus-verified before exclusion; 0.765784 is the last measured restoration result, not an accepted result for that later patch. Source and diffs are preserved in `rejected-accuracy-restoration/`. The prior removal of language-model coverage in 3e25db23 requires a separate accuracy repair.

## Measurements

AMD EPYC-Rome, 8 visible cores, 15.2 GiB visible host RAM; Linux x86_64; Rust/Cargo 1.95.0, Go 1.26.5. The container has a **six-CPU quota and a 12 GiB memory limit** (`cpu.max = 600000 100000`, `memory.max = 12884901888`), observed during final validation. This run did not modify those limits; their earlier history was not sampled. Shared host: load is recorded per process. No heavy builds/tests from this work ran concurrently with these measurements. Rust release settings, dependencies and compiler flags are unchanged. Debug/test symbols were disabled for disk conservation only. No special semantic-store override: `POLINT_CACHE_STORE` unset; no `polint-store-stamp.json` before reset, before analysis, or afterward. Each cold sample clears only the selected analysis cache; it does not flush the OS page cache or Go module/compiler caches. Each subsequent warm sample preserves the default cache and runs in a new process.

The final matrix re-measures both retained original and optimized release binaries. The rebuilt optimized binary is byte-identical to `bin/wave2-polint-tests` (SHA-256 ce516ef31d652c887e909c0d0d268097ee70e84ec8359598a604facda73d30b3). Source checksums prove the optimized source equals the saved pure-performance patch. Public corpus tracked files remain clean at the same SHAs. Whole pinned checkouts use default discovery except Hugo, which uses identical `include = ["**/*.go"]` on both binaries, matching the Go workload: default all-language discovery fails on non-UTF-8 `media/testdata/fake.js`. The syntax workload requests file/function/complexity metrics; deep requests the full kernel provider plan. These are full-kernel requests with existing setup/provider failures retained, not claims that every semantic capability succeeds. Hugo's first, compiler-contended attempt had a cached `GoSubprocessTimeout` diagnostic after the 120-second sidecar limit; that attempt is excluded in full. A retry instead reported a generated Go build file outside the repository. These are inherited symbol/reference setup failures, and changing setup outcomes cannot support a paired speed claim (`hugo-symbol-sidecar-diagnostics.json` preserves the excluded timeout observation). Completed comparisons require equal provider and diagnostic digests. Hugo's accepted original cold trace did not execute the solver, refined-call, data-flow or evidence providers after setup failures; the comparison describes the executed CFG/alias path and retained diagnostics. No consumer rule packs are benchmarked.

### Syntactic workloads

| Suite / SHA / license | Warm wall s, before → after | Speedup | Warm analysis-process peak RSS GiB, before → after | RSS Δ | Cold wall s, before → after | Cold analysis-process peak RSS GiB, before → after |
|---|---:|---:|---:|---:|---:|---:|
| jelly-callgraph-micro / `b799ed4f0d68c670fe398830aaa51dd5c628cf74` / BSD-3-Clause | 0.103 → 0.106 | 0.972× | 0.061 → 0.061 | 0.435% | 0.233 → 0.253 | 0.035 → 0.035 |
| go-x-tools-rta-callgraph / `7743a285e3d261ca235408e013ec5c14cb5170e4` / BSD-3-Clause | 1.512 → 1.428 | 1.059× | 0.183 → 0.184 | 0.231% | 3.193 → 3.087 | 0.232 → 0.233 |
| excalidraw-excalidraw-scale / `0dbd2a39319d41fda37b2945dea0dcbd58d6a564` / MIT | 0.188 → 0.175 | 1.075× | 0.061 → 0.061 | 0.169% | 0.260 → 0.223 | 0.052 → 0.052 |
| gohugoio-hugo-scale / `3f35721fb2c75a1f7cc5a7a14400b66e73d4b06e` / Apache-2.0 | 0.682 → 0.618 | 1.104× | 0.096 → 0.096 | 0.044% | 1.157 → 1.130 | 0.109 → 0.109 |

### Deep workloads

| Suite / SHA / license | Warm wall s, before → after | Speedup | Warm analysis-process peak RSS GiB, before → after | RSS Δ | Cold wall s, before → after | Cold analysis-process peak RSS GiB, before → after |
|---|---:|---:|---:|---:|---:|---:|
| jelly-callgraph-micro / `b799ed4f0d68c670fe398830aaa51dd5c628cf74` / BSD-3-Clause | 60.478 → 34.807 | 1.738× | 3.121 → 2.402 | -23.036% | 59.182 → 33.274 | 3.117 → 2.406 |
| go-x-tools-rta-callgraph / `7743a285e3d261ca235408e013ec5c14cb5170e4` / BSD-3-Clause | N/A → N/A | N/A | N/A → N/A | N/A | N/A → N/A | N/A → N/A |
| excalidraw-excalidraw-scale / `0dbd2a39319d41fda37b2945dea0dcbd58d6a564` / MIT | 233.739 → 78.136 | 2.991× | 5.441 → 4.213 | -22.575% | 214.895 → 76.511 | 5.435 → 4.212 |
| gohugoio-hugo-scale / `3f35721fb2c75a1f7cc5a7a14400b66e73d4b06e` / Apache-2.0 | 312.883 → 91.422 | 3.422× | 4.855 → 4.862 | 0.145% | 433.043 → 128.522 | 4.891 → 4.892 |

### Deep warm repetitions

| Suite / SHA / license | Original seconds (three runs) | Optimized seconds (three runs) |
|---|---|---|
| jelly-callgraph-micro / `b799ed4f0d68c670fe398830aaa51dd5c628cf74` / BSD-3-Clause | 60.478, 60.133, 61.426 | 34.395, 35.067, 34.807 |
| go-x-tools-rta-callgraph / `7743a285e3d261ca235408e013ec5c14cb5170e4` / BSD-3-Clause | N/A | N/A |
| excalidraw-excalidraw-scale / `0dbd2a39319d41fda37b2945dea0dcbd58d6a564` / MIT | 224.176, 233.739, 239.355 | 78.136, 78.925, 77.944 |
| gohugoio-hugo-scale / `3f35721fb2c75a1f7cc5a7a14400b66e73d4b06e` / Apache-2.0 | 312.883, 317.260, 312.681 | 91.844, 91.422, 90.837 |


Wall is whole-process elapsed time; RSS is the analysis process high-water mark from `getrusage(RUSAGE_SELF)`; it does not include separate child-process memory. Warm wall and RSS medians are computed independently. Cold samples are single observations. Go module/compiler-cache history is not reset, so differences between Go cold observations cannot be attributed solely to engine changes; headline ratios use the three warm repetitions. Raw samples, load, hashes, configs, budgets, source counts, and cache sizes are under `final-matrix/`; `final-matrix-summary.json` contains the numerical audit. Times are never combined with accuracy into a score. Syntactic providers were not optimized; their small timing differences are observed variation, not a claimed mechanism-level syntax speedup.

The initial evening batch overlapped unrelated host compilation and is excluded in full under `contended-matrix-20260907T2051/`. The replacement protocol waits for quiet compiler activity, monitors process ancestry during each timed run, and excludes/retries samples with observed external compilation. Compilation belonging to the analyzed Go workload is included as actual work. Rejected samples remain recorded; exclusion does not depend on whether a timing is favorable.

Incomplete measurement attempts:

- matrix-baseline-deep-go-x-tools-rta-callgraph: exit 247; any incomplete cold/warm series is N/A, not a speedup baseline.
- matrix-final-deep-go-x-tools-rta-callgraph: exit 247; any incomplete cold/warm series is N/A, not a speedup baseline.

The golang/tools original and optimized cold attempts ended with SIGKILL after 6,672.845 and 5,977.951 seconds respectively, before a complete point or diagnostic digest was emitted. The cgroup OOM-kill counter increased during the optimized attempt, and its memory peak reached the 12 GiB limit. This supports memory exhaustion as the cause; the counter is container-wide and does not identify a killed PID. Neither termination time is a performance measurement or an upper/lower-bound speedup claim. There are no valid warm samples or whole-run RSS measurements for golang/tools deep analysis. The last completed providers through extensions have equal digests on both attempts; that is partial provider evidence, not a complete determinism proof for this workload. The fixed five-case Go accuracy gate is separate and remains unchanged.

## Reproducing the workload

The measurement scripts and frozen executables are retained under `.context/perf-research/` as requested. The underlying entry is an internal libtest harness, not a supported CLI or SDK feature. Build it at each revision with `cargo test -p polint --lib --all-features --locked --release --no-run`, then use the test executable printed by Cargo:

```sh
unset POLINT_CACHE_STORE
export POLINT_PERF_CHILD_REPO=/absolute/path/to/the/pinned/checkout
export POLINT_PERF_CHILD_COLD_ONLY=1
export RUST_LOG=polint::kernel::stage=info
<test-executable> eval::bench::runner::tests::perf_child_measure_entry --exact --nocapture
```

Use the same Go PATH and corpus configuration above. For cold, clear only that checkout's `.polint/cache`; run three more fresh processes preserving it for warm. Check store stamps before clearing, before analysis and afterward. Add `POLINT_PERF_CHILD_CAPABILITIES=file_metrics,function_metrics,complexity_metrics` for the syntactic workload; leave it unset for deep. The harness's `COLD_ONLY` flag selects one kernel invocation per process; actual cold/warm classification comes from the external cache treatment. The reported medians use external whole-process wall time, not the duplicated internal cold/warm fields in single-run mode.

## Retained mechanisms

1. **Alias preparation:** group access paths by base after existing duplicate-ID resolution, calculate each distinct operand identity once, and sort the original operand sequence by compact lexical ranks. Equal text retains stable ties and adjacent-only deduplication.
2. **Data-flow joins:** immutable callsite/target-summary indexes plus append-aware node/edge indexes remove repeated global scans while preserving first-match lookup and original insertion order. Local flow indexes node keys, bodies and places.
3. **CFG/evidence joins:** group MIR rows by body and index IDs once; index immutable CFG edge/block/function lookups for control-dependence evidence. Dominance, reachability, graph semantics and solver budgets are unchanged.
4. **Evidence digest materialization:** sort small IDs/references, then expand one node/edge payload at a time. Decimal textual ID order reproduces full payload ordering; duplicate IDs fall back to complete payload comparison. Existing digest bytes and cache schemas remain intact.

Everything uses safe Rust and private implementation boundaries. Hash maps are lookup-only; output order never follows hash iteration. Parallel scheduling is unchanged. No API, CI, baseline, profile, dependency or allocator changes. Private debug timing supports provider attribution.

## Evidence, limitations and rejected ideas

Earlier Excalidraw (MIT, same pinned SHA) single-run component attribution: alias preparation 75.253 → 0.324 s, local flow 12.054 → 0.464 s, direct-call flow 31.468 → 0.560 s, CFG lower/normalize 12.694 → 1.156 s. These are exploratory stage timings, not additional end-to-end medians. Raw profiles: `baseline-substages/`, `wave1-substages/`, `wave2-cfg-substages/`, `wave2-evidence-substages/`.

Provider/diagnostic digest audits accompany completed before/after series. Canonical semantic case bytes for both fixed accuracy suites are identical after excluding timing fields. The report-level hash itself includes comparison-row cost columns and changes with runtime; it is not claimed byte-identical. Evidence's pre-existing duplicate control-dependence identity error discards that provider's computed digest on Excalidraw and Jelly; aggregate provider equality alone cannot prove its digest. The new differential test separately compares complete sorted payload streams of actual node/edge row types, including duplicate/sparse IDs, decimal-prefix boundaries, maximum u64, Unicode and escaping. Existing parser/setup/evidence diagnostics and budget caps are retained, not repaired or hidden.

`perf`, Valgrind and cargo-flamegraph were unavailable; provider/substep wall instrumentation supplied attribution. It is not CPU sampling. Parse/allocator/release-flag experiments were rejected because parsing is a tiny fraction of the measured deep workload and profile changes were prohibited. Solver/SCC/bitset and dominance rewrites were deferred: the shared solver is under 3% of the reduced Excalidraw profile and CFG relations about 2.6 s, with meaningful compatibility risk around nonterminating graphs. No 10× projection is treated as a result.

The interrupted 495.614 s golang/tools CFG timing from night one is profiling only: no completed process sample exists, and builds overlapped later providers. The initial Hugo all-language failure is not a performance baseline. The failed accuracy port contributes no speedup numbers. Initial TMPDIR-related fixture failures were environmental; normal temporary-directory behavior was restored. Infrastructure interruptions delayed execution well beyond the requested overnight publication window.

## Validation

The fixed accuracy command (with the Go PATH above and `POLINT_CACHE_STORE` unset) is:

```sh
POLINT_REQUIRE_BENCH_CORPUS=1 POLINT_WRITE_GRAPH_BENCH=1 POLINT_GRAPH_BENCH_TIER=release cargo test -p polint --lib --all-features --locked eval::external::tests::external_graph_baseline_reports_can_be_generated -- --nocapture
```

- Exact original corpus command: completed and rerun after the final matrix; inherited Jelly failure, F1 delta **0.000000** for both measured suites. `final-gate/`, `post-matrix-gate/`, `post-matrix-gate-equivalence.json` and `final-accuracy-equivalence.json` preserve evidence; original debug/release failures are under `baseline-gate/` and `baseline-release-gate/`.
- Default-profile validation covers **all workspace targets across the original run and targeted recovery**. `cargo test --workspace --locked --no-fail-fast` completed the full polint library suite: 2,467 passed, the inherited Jelly gate failed, and 14 were ignored; the long real-corpus sweep passed. The target directory disappeared while that library test was running, preventing 36 subsequent targets from executing. A rebuild and targeted retries then executed every missing target, with only the inherited missing Excalidraw snapshot failure. No library test was filtered out, and no test or oracle was changed. The single original command did not complete every target; the combined validation is **not fully green** because of Jelly and the missing snapshot. Exact recovery commands and source-identity checks are in `final-debug-recovery.json`; raw original failures remain in `final-debug-workspace.json` and `logs/final-workspace-debug.log`. See `artifact-loss-RESEARCH.md` for the recovery rationale.
- `cargo test --workspace --release --all-features --locked --no-fail-fast`: all targets completed; the same Jelly test and `characterization_goldens_match_cli` failed. The CLI failure is a missing Excalidraw expected snapshot: no scale snapshots exist in d10432aa and the harness is byte-identical to that revision. All example CLI diagnostic/cost comparisons passed before that missing-file assertion. No snapshot, gate or test-discovery change is included (`inherited-golden-fixture-failure.json`). Full results are in `logs/final-workspace-release.log`. This command is also **not fully green** because of those two inherited failures.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: passed.
- `cargo fmt --all` and final `cargo fmt --all -- --check`: passed. `git diff --check`: passed.
- No parallel scheduling change. Identity comparisons and seeded determinism tests protect the retained algorithms.

### Remaining provider costs

Median provider wall time from the three accepted warm deep runs. These are stage timings, not CPU samples; independent medians need not sum to the process median. The five most expensive optimized providers are shown.

jelly-callgraph-micro / `b799ed4f0d68c670fe398830aaa51dd5c628cf74` / BSD-3-Clause

| Provider | Original s | Optimized s |
|---|---:|---:|
| `polint.evidence` | 7.515 | 5.642 |
| `polint.type_value_alias` | 19.122 | 5.420 |
| `polint.cfg` | 7.850 | 5.163 |
| `polint.data_flow` | 11.019 | 3.708 |
| `polint.abstract_domains` | 2.924 | 2.938 |

excalidraw-excalidraw-scale / `0dbd2a39319d41fda37b2945dea0dcbd58d6a564` / MIT

| Provider | Original s | Optimized s |
|---|---:|---:|
| `polint.type_value_alias` | 103.490 | 13.036 |
| `polint.evidence` | 13.969 | 9.689 |
| `polint.cfg` | 20.995 | 8.579 |
| `polint.refined_calls` | 8.050 | 8.226 |
| `polint.semantic_mir` | 8.040 | 7.800 |

gohugoio-hugo-scale / `3f35721fb2c75a1f7cc5a7a14400b66e73d4b06e` / Apache-2.0

| Provider | Original s | Optimized s |
|---|---:|---:|
| `polint.go.semantic` | 30.464 | 30.420 |
| `polint.cfg` | 90.231 | 17.594 |
| `polint.type_value_alias` | 165.926 | 12.230 |
| `polint.abstract_domains` | 11.778 | 11.966 |
| `polint.semantic_mir` | 11.163 | 11.302 |


## Next lever

The remaining Excalidraw profile holds roughly 3 GiB of recursive identity text from 2.6 MB of source. Represent composite keys as shared interned structure and stream the existing canonical bytes into hashes, with exact textual ordering and equality proofs. This reaches construction, sorting, hashing and retained memory together. A replacement hash-map crate alone does not remove duplicated substructure. See `NEXT-LEVER.md`; Go additionally needs sidecar loading/timeout diagnosis and a new profile with successful semantic setup; a 120-second symbol-provider stage here is a timeout, not evidence of 120 seconds of symbol-graph computation. The symbol and semantic Go sidecars also perform separate `packages.Load` calls. Sharing a rich load for combined requests is a follow-up hypothesis, not a measured gain: their pinned x/tools versions differ, and the semantic frontend requires `NeedDeps` plus full SSA for RTA correctness. Accuracy-model restoration remains separate and higher priority for release acceptance.

A standalone follow-up verified an exact composable summary for the existing FNV-1a recurrence: a shared byte fragment can be represented by a multiplier and 256 correction values, preserving its effect on any incoming hash state. The algebra and 22,776 differential byte-stream checks are documented in `NEXT-LEVER.md` and `fnv_chunk_proof.json`. This could avoid revisiting large shared identity fragments while retaining digest bytes. Each table costs 2 KiB, so selective caching and a shared identity representation are prerequisites; no production integration or speedup is claimed.

## Publication

Prepared on `perf/astra-10x`, based on d10432aa. Publication results are appended below after the commit/push/draft-PR operations. Never merge.

Commit: `cebc7c55230df9635c0e2d2a70f3beaf92ff0364`. Push exit: 0. Draft PR: https://github.com/oaiz-io/polint/pull/113.
