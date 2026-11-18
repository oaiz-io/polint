---
phase: 75-lower-js-ts-value-flow
plan: 01
subsystem: ts-value-flow
requirements-completed: []
status: executed-with-acceptance-gaps
---

# Value-flow lowering, measured fallback, mutations and gate

Tasks 1–4 add private constraint rows for calls, captures, returns, objects, modules and platform operations. Task 5 migrated the construct tests and completed the measured retention exit. Acceptance gaps remain; this document does not mark all PG-06 targets met.

## Walker verdict

Fresh projection-on/off reports compared all **3,382** observed tune edges by stable site, observed target and kind. Lost with the walker off: **105**; gained: **0**. Verdict: **kept: 105 edges nothing else finds**. All are outside exact/conservative; neither tier's recall changes. Both ablations have zero wrong exact targets and false-complete answers.

| package | cause | kept: edges nothing else finds |
| --- | --- | --- |
| async | this_member | 4 |
| axios | this_member | 3 |
| commander | local_member | 6 |
| commander | parameter_call | 7 |
| commander | this_member | 85 |

The receiver recognizers remain isolated in collect_class_body_call_flows / collect_class_method_bodies, collect_prototype_method_this_flows and collect_export_object_method_flows. The downstream member/parameter propagation remains in collect_function_parameter_flows and collect_function_flow_invocation. These are general construct recognizers, not benchmark-name checks. Counts are for the whole-projection ablation, not independently measured counts per helper; no helper is claimed redundant. The walker directory and wiring remain. The complete stable-site/target list is [75-05-WALKER-EDGES.md](75-05-WALKER-EDGES.md); the machine-readable list, mutation assertions and gate results are [75-05-EVIDENCE.json](75-05-EVIDENCE.json).

## Continuation audit

The two inherited completed commits were retained: 3b980ad7 moves callable construct tests; 417b0093 adds the kernel unknown-read regression. Commit 167b1c52 adds the missing row-level mutation guard. Every inherited partial rest edit was reverted by exact path: crates/polint/src/analysis_neutral/semantic_graph/cache_key.rs; crates/polint/src/ts/constraints/callable_values.rs, described.rs, lower.rs and mod.rs; crates/polint/src/ts/semantic_graph_build/value_flow.rs. The incomplete rest work failed two tests and did not establish safe positional flow. The migrated rest case still relies on the retained fallback.

crates/polint/src/ts/semantic_graph_build.rs was restored after each local projection-off run. tests/js-analysis-bench/report.py temporarily expanded all example samples for complete edge comparison and was restored before baseline regeneration. No switch or sample-limit change is committed. The protected tests/fixtures/public-surface-leak-probe/Cargo.lock remains excluded. No held-out source, driver or recording, public surface, branch base or PR #140 was changed.

The full gate additionally caught a Task 4 reverse dependency: neutral cache metadata imported the TS model loader. Commit d8b2c312 moved only the shared bytes/digest recipe to analysis_neutral/adaptation/platform.rs, re-exported them privately to TS and rewired cache metadata. Both the SOURCE declaration and digest function body are byte-for-byte unchanged from 0b8e2851; seven cache-key tests and both layering guards pass. The ablations precede this identity-preserving move; the final baseline/reproducibility runs use the refactored code. Commit 3d1c021d refreshed the stale dense-ID audit baseline after classifying three added non-test byte-offset tuple renderings, two removed old byte-offset renderings, and the remaining test assertions/relocations. All 55 signature changes are listed in its commit message; no gate was weakened. The initial workspace failure and the full post-fix rerun are both retained below.

## Held-out recall lost and restored

The Task 3 bench run had refined every-tier recall of acorn 1102/1141 and node-forge 2750/3131. By the end of the phase the committed baseline had 190/1141 and 1900/3131, while the other six held-out packages and every tune package rose. The loss was bisected on bench numbers alone. No held-out source was read to find the cause or design the fix.

**Cause.** Commit d571ae24 ("walk each prototype once per load and stop at an escaped object"). Its parent 07d12b07 still finds the edges; d571ae24 loses them. The reverted partial edits after the account change are not involved: they never reached a committed tree.

| refined, every tier | 07d12b07 | d571ae24 … baseline at 2cbffa6c | 6c3ba813 (fix) |
| --- | --- | --- | --- |
| acorn found / 1141 | 1102 | 190 | 1119 |
| acorn apart | 923 | 9 | 929 |
| node-forge found / 3131 | 2750 | 1900 | 2762 |
| node-forge apart | 1961 | 1108 | 1970 |
| wrong exact (single target), all packages | 0 | 0 | 0 |

The lost edges were right. In the apart tier, which held almost all of them, precision was 0.9968 for acorn (926 claimed, 923 observed) and 0.9549 for node-forge (2061 claimed, 1968 observed) before d571ae24. After it, acorn had 12 claimed and 9 observed (0.75), and node-forge 1162 claimed and 1108 observed (0.9535). Exact recall and exact claims were unchanged.

**General construct.** A method called on an escaped object whose method is inherited through the prototype chain. Typical cases are an instance a module returns or exports, and `this` inside the methods of such an instance. An escaped object may gain fields from outside code, so the described-flow pre-solve answers a load from it with Unknown. d571ae24 stopped the prototype walk at the escaped object, so the inherited methods were never offered as candidates and the call kept only the open answer. The walk must go on past the escaped object and add Unknown beside the inherited candidates, which keeps the answer open (never exact) and still names the targets that do exist.

**Construct cases** (74865854), written from the construct and not from any package source:

- `constructs/prototype/escaped-instance` (tune constructs)
- `constructs/prototype-chain/escaped-subclass-instance` (held-out constructs)

**Fix.**

- 0a245115 continues the walk past an escaped object and restores the unit test `an_escaped_object_inherits_its_prototype_methods_beside_an_unknown_value`.
- On its own it timed out zod's globs-alone refined dump (600 s). Instrumentation showed the reason: round 1 had 510k lookups over 2.5M tokens, and every load re-walked a wide abstract prototype graph (depth 16, prototype sets up to 185 parents), hundreds of millions of steps. 07d12b07 times out on the same dump for the same reason. d571ae24 had bounded the cost only by cutting the walk at escaped objects, which is what lost the edges.
- 6c3ba813 keeps one walk result per object and field for the rest of a solve round, so every load of a field from an object shares one walk and the work stays linear in the prototype graph. Results are dropped after each propagation. A cycle or the depth limit leaves the walk open and is not memoized. Internal-slot flags are kept per field id rather than rescanned per load.

Recall with the fix, against the baseline at 2cbffa6c (refined found): acorn 190 → 1119, node-forge 1900 → 2762, dayjs 62 → 88, pako 246 → 255, axios 582 → 602, immer 251 → 253, js-yaml 273 → 276, async 587 → 591; the other packages are unchanged. In the apart tier, precision with the fix is 0.9968 for acorn (932 claimed, 929 observed) and 0.9551 for node-forge (2070 claimed, 1977 observed), the values before the loss. Wrong exact single-target claims stay 0 on every package. Exact found is unchanged on every package.

**Cost.** In the refreshed baseline (8f0f4d30), zod shipped refined takes 44.6 s wall and 1244 MiB, and zod globs alone 157.4 s and 4532 MiB (semantic_graph 34.8 s, solver 27.6 s; every-tier recall 90.98%), where 07d12b07 timed out at 600 s. Axios refined/base in that run is 11706/2363 ms = 4.95x, at load averages 111.48 99.40 101.01 at the start and 51.18 71.34 95.79 at the end. An interleaved re-measurement of axios at load about 40–47 gave a median refined/base of 4.69x before the fix (2cbffa6c: 4458/1035, 4953/1057, 6067/1113 ms) and 5.22x with it (6c3ba813: 5215/1168, 5297/1015, 7816/1373 ms). The fix adds about 7% to refined axios time. Earlier with 6c3ba813 at load about 34: 4249/1012, 4473/878 and 4386/912 ms, about 4.8x.

The recorder attribution fix and the re-recordings it causes moved to their own pull request (#150, stacked below this one), as `research/package-graphs/HELD-OUT.md` rule 2 requires. This pull request no longer changes a held-out recording.

## Recall and wrong answers

All fresh plans are without types (typescript: null); typed results are **unmeasured**, not copied from earlier work. Tune denominator 3,382 observed edges, held-out 5,453. All eight held-out packages are counted; none is coverage-flagged. Wrong exact single-target claims and false-complete answers remain **0** on every measured plan and package, including the 272 construct call lines and 5 hidden-path checks. Numbers are from the baseline refreshed with the prototype walk restored (8f0f4d30).

| no-types plan | tier | tune | held-out | gap below tune |
| --- | --- | --- | --- | --- |
| base | exact | 44.53% | 27.12% | 17.4 pp |
| base | exact_complete | 44.53% | 27.12% | 17.4 pp |
| base | exact_conservative | 44.53% | 27.12% | 17.4 pp |
| refined | exact | 48.49% | 32.48% | 16.0 pp |
| refined | exact_complete | 48.49% | 32.48% | 16.0 pp |
| refined | exact_conservative | 53.52% | 33.61% | 19.9 pp |
| base | apart_only | 1.86% | 0.28% | 1.6 pp |
| base | every tier | 46.39% | 27.40% | 19.0 pp |
| refined | apart_only | 33.74% | 54.87% | −21.1 pp |
| refined | every tier | 87.26% | 88.48% | −1.2 pp (held-out above tune) |

| package | Phase 74 every tier | final every tier | Phase 74 exact | final exact | final exact+conservative | both baseline comparisons |
| --- | --- | --- | --- | --- | --- | --- |
| async | 51.49% | 92.78% | 50.86% | 51.49% | 54.63% | met |
| axios | 45.33% | 89.19% | 39.41% | 45.63% | 53.19% | met |
| chalk | 59.72% | 77.78% | 59.72% | 59.72% | 61.11% | met |
| commander | 37.42% | 57.55% | 6.60% | 11.32% | 26.73% | met |
| dayjs | 31.62% | 78.57% | 27.35% | 30.36% | 32.14% | met |
| underscore | 60.31% | 96.15% | 58.78% | 81.92% | 88.08% | met |
| zod | 55.12% | 90.29% | 51.22% | 51.83% | 54.20% | met |

The retained Phase 74 report's headline recall includes every tier; its separate exact values are lower. The Phase 74 summary's single-target wording is ambiguous, so this table compares both report fields consistently. Both comparisons meet or exceed the baseline on all seven packages. An earlier continuation interpretation compared the old headline against the new exact tier and incorrectly reported three regressions; that interpretation is corrected here. Openness is preserved at export boundaries.

Missed edges per cause bucket, before lowering and final (every tier). The retained pre-lowering report has 3,389 tune edges; the final report has 3,382 after the recorder-attribution correction documented in Task 3. The held-out denominator is unchanged in this continuation:

| miss cause (every tier) | Phase 74 | final |
| --- | --- | --- |
| call_apply_bind | 94 | 55 |
| call_result_member | 3 | 3 |
| callback | 162 | 34 |
| computed_key | 18 | 17 |
| constructor | 69 | 1 |
| iife | 1 | 0 |
| import_binding | 1 | 0 |
| local_call | 152 | 30 |
| local_member | 196 | 76 |
| module_member | 424 | 5 |
| object_literal_member | 20 | 2 |
| other | 7 | 0 |
| parameter_call | 236 | 6 |
| parameter_member | 187 | 112 |
| this_member | 107 | 90 |

Every-tier recall below 65% remains only for commander, with these complete cause counts (dayjs rose to 78.57% with the prototype walk restored):

- commander: `call_result_member` 2, `callback` 5, `local_member` 16, `parameter_call` 1, `parameter_member` 71, `this_member` 40.

Remaining construct-family gaps:

| construct family still below full recall | found/labelled edges |
| --- | --- |
| accessor | 3/6 |
| cjs-interop | 3/4 |
| class-features | 12/13 |
| closure | 9/11 |
| computed-key-unknown | 4/6 |
| dynamic-import | 1/2 |
| dynamic-property | 0/2 |
| literal-spread | 5/7 |

class-method, module-initializer and prototype now reach full recall.

## Exit targets and cost

- Heuristic edges 0: **not met on any tune package**; counts below. Open export-boundary sets depend on Phase 76's tiers/assumptions; the measured fallback also remains Heuristic. No precision label was promoted to hit a target.
- Provider share <=10%: individual verdicts below, using semantic_graph elapsed divided by the sum of refined stage times.
- Axios <=3x base: **not met**. The earlier 2.99x was measured under high load; at moderate load the ratio is about 4.7x before and 5.2x after the prototype-walk fix (figures below).
- Recall >=Phase 74 on every tune package: **met, 7/7**, comparing both every-tier and exact recall to the same retained baseline fields.
- Every-tier recall >65%: commander remains below; all residual causes are listed below, meeting the alternative evidence requirement.
- Held-out recall and gap: **reported**. The 5-point margin is **not met** for exact (16.0 points) and exact+conservative (19.9 points). Every-tier held-out recall is 1.2 points above tune.

| package | Heuristic rows (target 0) | semantic_graph / stage total | share (<=10%) | share target |
| --- | --- | --- | --- | --- |
| async | 46411 | 1807/25869 ms | 6.99% | met |
| axios | 538 | 1592/10716 ms | 14.86% | not met |
| chalk | 14 | 50/468 ms | 10.68% | not met |
| commander | 146 | 1819/4041 ms | 45.01% | not met |
| dayjs | 60 | 144/635 ms | 22.68% | not met |
| underscore | 424 | 125/2377 ms | 5.26% | met |
| zod | 5143 | 9234/41726 ms | 22.13% | not met |

Measured in the refreshed baseline (8f0f4d30). Zod shipped refined: **44.6 s / 1244 MiB**; globs alone **157.4 s / 4532 MiB**, every-tier recall 90.98%. These are separate analysis sets. Axios refined/base in the baseline run: **4.95x** (11706/2363 ms); interleaved at load about 40–47 the median is 5.22x (4.69x before the prototype-walk fix). Target <=3x **not met**. Load averages (1/5/15 min) of the baseline run: start **111.48 99.40 101.01**, end **51.18 71.34 95.79**. Costs are observations on this shared, loaded server; no quiet window was awaited. The earlier figures (zod 64.8 s, globs alone 96.4 s, axios 2.99x) were measured before the walk was restored and under heavier load.

Baseline regenerated with stage times via make js-bench JS_BENCH_ARGS=--no-types, committed as 07be066b and refreshed after the prototype-walk fix as 8f0f4d30.

Task 3 exited with these unmet targets at its measurement: local_member 196 ->107 (target <=98), this_member 107 ->107 (<=53), parameter_member 187 ->119 (<=93), at its own measurement. Final counts are in the bucket table: local_member now meets <=98 after Task 4; this_member and parameter_member remain above their limits. Task 4 call_apply_bind 94 ->56 (target <=23), remaining async 39, axios 11, underscore 6. All three report adaptation.max_targets_per_source at cap 16, beside computed receivers and higher-order flows; not every miss is attributed solely to the cap. The budget policy is unchanged.

## Mutation checks

| mutation | mutated location | failed test(s) | failure location |
| --- | --- | --- | --- |
| member callee -> Unknown | `crates/polint/src/ts/constraints/lower.rs:1560` | `lower::tests::object_fields_flow_through_parameters` (11 failed) | `crates/polint/src/ts/constraints/lower.rs:1902:13:` |
| do not share captured places | `crates/polint/src/ts/constraints/lower.rs:284` | `callable_values::returned_callables_are_complete_or_left_unresolved` (4 failed) | `crates/polint/src/ts/constraints/callable_values.rs:1268:9:` |
| drop $proto lookup | `crates/polint/src/ts/constraints/described.rs:533` | `lower::tests::prototype_chains_and_own_shadowing_are_described` (4 failed) | `crates/polint/src/ts/constraints/lower.rs:1902:13:` |
| unknown key loads nothing | `crates/polint/src/ts/constraints/lower.rs:1461` | `lower::tests::unknown_computed_reads_open_callbacks_reachable_from_the_base` (1 failed) | `crates/polint/src/ts/constraints/lower.rs:2150:9:` |
| remove reduce cardinality operation | `crates/polint/models/platform/javascript.toml:29` | `callable_values::reduce_results_follow_array_cardinality` (1 failed) | `crates/polint/src/ts/constraints/callable_values.rs:1100:9:` |
| ignore model digest in cache key | `crates/polint/src/analysis_neutral/semantic_graph/cache_key.rs:79` | `analysis_neutral::semantic_graph::cache_key::tests::platform_behavior_changes_the_semantic_graph_cache_key` (1 failed) | `crates/polint/src/analysis_neutral/semantic_graph/cache_key.rs:95:9:` |

All mutations ran alone and were reverted using git checkout -- <exact file>. Mutation 4 initially survived all 62 tests, including the kernel regression in 417b0093, because the fallback masked the missing guard. A new direct-row test in 167b1c52 passed normally and failed under the mutation. The current unknown-key implementation opens the result and escapes the base; it does not enumerate the known fields or a wildcard field. The regression tests that actual safety behavior, not unimplemented conservative field coverage. Mutation 3 disables the actual $proto lookup in ts/constraints/described.rs. Mutation 6 holds the cache-key digest constant while the existing test changes model text.

## Full gate

Prechecks: cargo test -p polint --lib ts::constraints:: --locked passed all 62 tests before the new test. The new direct-row test passed alone before its commit. An initial fresh-on benchmark build was interrupted (rc 130) when concurrent mutation work was discovered; it produced no accepted report and was replaced by the stable-source runs below.

| command | result |
| --- | --- |
| `cargo fmt --all -- --check` | passed |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | passed |
| `cargo test --workspace --locked --no-fail-fast` | failed (rc 101); 3325 passed, 3 failed, 22 ignored |
| `cargo test -p polint --test public_surface_leak --locked` | passed; 8 passed, 0 failed, 0 ignored |
| `cargo test -p polint --lib solver_composition --locked` | passed; 17 passed, 0 failed, 0 ignored |
| `cargo fmt --all -- --check` | passed |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | passed |
| `cargo test --workspace --locked --no-fail-fast` | passed; 3328 passed, 0 failed, 22 ignored |
| `cargo test -p polint --test public_surface_leak --locked` | passed; 8 passed, 0 failed, 0 ignored |
| `cargo test -p polint --lib solver_composition --locked` | passed; 17 passed, 0 failed, 0 ignored |
| `python3 tests/js-analysis-bench/bench.py report --no-cost --no-types --out /tmp/js-bench-75e-resume-off` | rc 0 |
| `python3 tests/js-analysis-bench/bench.py report --no-cost --no-types --out /tmp/js-bench-75e-resume-on` | rc 0 |
| `make js-bench JS_BENCH_ARGS=--no-types` | rc 0 |
| `python3 tests/js-analysis-bench/bench.py report --no-cost --no-types --out /tmp/js-bench-75e-determinism-a` | rc 0 |
| `python3 tests/js-analysis-bench/bench.py report --no-cost --no-types --out /tmp/js-bench-75e-determinism-b` | rc 0 |
| `cmp /tmp/js-bench-75e-determinism-a/report.json /tmp/js-bench-75e-determinism-b/report.json` | rc 1 |
| `cmp /tmp/pr75-determinism-a.json /tmp/pr75-determinism-b.json` | rc 0 |

Every Cargo command (including mutations and reruns) used GOTOOLCHAIN=go1.25.12 and the required shared CARGO_TARGET_DIR. The known concurrent Go frontend flake did not occur in either workspace run. The no-cost format retains dump timing/RSS/provider timings: raw byte equality is not met. A second cmp removes only dump_cost and compares all analysis fields byte for byte; no examples or analysis rows are discarded.

solver_composition is a crate-private library-test filter, not an integration target. Its tests cover propagation, openness, budgets, order independence and monotonicity. A passing suite does not prove actual package export/stitch-equals-whole; that operation is absent from this suite and remains unverified here.

## Self-check

All requested mutation and gate commands ran; fresh measurements and baseline regeneration are recorded. The walker was retained at a measured exit. Unmet tier, timing, raw-byte and actual-stitch targets are recorded with evidence. Both per-package recall baseline comparisons are met. PR remains draft on the required feature branch, with no merge, release, new branch or public API change. Central STATE/ROADMAP completion is left to the parent orchestrator.
