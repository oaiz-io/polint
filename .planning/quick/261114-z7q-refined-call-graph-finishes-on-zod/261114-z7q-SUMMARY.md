---
quick_id: 261114-z7q
status: complete
date: 2026-11-14
branch: perf/refined-call-graph-zod
---

# Quick Task 261114-z7q: the refined JS/TS call graph finishes on zod — Summary

**Date:** 2026-11-14
**Status:** Complete, local only (no push, no pull request). The refined dump of zod finishes in 81 s (budget 180 s;
it timed out at 600 s and 900 s before). Every other package and plan scores exactly as before, and their dumps
are byte-identical to the dumps before the fix.
Scratch and raw evidence outside the repository: `/tmp/zodperf/` (`out/`, `scripts/`, `bin/`).

## Cause

The finding and growth law in the PLAN, measured before any change: `collect_function_flow_invocation` re-walked
every followed call, bounded only by its 17-level depth cap. zod's `deepPartialify` (`src/v3/types.ts:2424`)
follows four calls into itself (k = 4). The walk performed `I(d0) ~ k^(18-d0) / (k-1)` invocations per entry:
20.3M invocations in 39.8 s, a level ratio of 4.00, every invocation with the same argument key, and
5.7 x 10^9 invocations (about 3.1 h) for the depth-1 return summary alone.

## Fix

`crates/polint/src/ts/callable_flow/extract.rs`, in two commits:

1. `5f97f2b4`: an exact invocation memo scoped to one call tree. The memo records a walk's outcome (the returned
   targets, the env after the walk, the guard budget after it) under its complete input (function and body, callee
   env, invocation depth, `this_method_walk`, `current_super`, the observable walk-guard state). A walk with an
   equal input replays the outcome. It is cleared when the outermost walk returns.
2. `b019a14e`: the memo's own cost. Keying every nested walk copied the callee env into the key, and recording
   every outcome copied it twice. That cost commander's refined dump 22% (median 2,047 vs 1,680 ms, six
   interleaved rounds against the before binary). The commit message says "about 7%". That is the figure for an
   intermediate build that hashed the key but still keyed every walk (1,778 vs 1,657 ms), not for `5f97f2b4`.
   History is left as it is, because the committed baseline records `b019a14e` as its `git_head`. The memo now
   differs from the PLAN's task 2:
   - It is keyed by a SipHash of the input and the starting env.
   - An input's first walk is only marked as seen (no env copy). Its second walk records the outcome, and later
     walks replay it. A replay compares the stored input and starting env with the current ones, so a hash
     collision costs a walk, never a wrong replay.
   - A call tree keys its walks only after `INVOCATION_KEYING_AFTER_WALKS` (4096) of them, so a small tree
     pays nothing for the memo. On commander the callable-flow phase takes a median 776 ms at T = 4096
     (10 runs) and 776 ms with keying disabled (16 runs). With the threshold at 256 it takes 839 ms, and at
     1024 it takes 811 ms.

**Invariant kept.** A walk is replayed only from a record of an equal input (function, body, starting env, depth,
walk flags, observable guard state), recorded and replayed under the same `table_generation`. A walk that changed a
table it reads is never recorded. Rows are append-only and deduplicated at the end. So a replay produces exactly the
rows, env, returned targets and guard budget a re-walk would produce: no fact is added or lost, and wrong-complete
cannot move.

**Cost after.** A call tree walks at most T = 4096 bodies unkeyed. Past that, while the tables a walk reads are
unchanged, each distinct input is walked at most twice: the second walk records the outcome, and later arrivals
replay it. On `deepPartialify` every level repeats the same inputs, so the walk count grows linearly with the
depth instead of as `k^D`. The regression test measures this growth. zod's callable-flow phase takes 2.8–2.9 s
of the 81 s dump.

## Exactness

- Fact dumps (base and refined) of all six small packages are byte-identical to the dumps before the fix, from
  the final release binary.
- zod refined is byte-identical across four builds: the first memo, a verify build that re-walked on every hit
  and asserted the same outcome, the threshold-256 build, and the final build.
- Constructs: labelled-edge precision is 38/38 (base) and 73/73 (refined) before and after. False-complete call
  lines and hidden paths are 0 / 0.

## Before / after (bench report at `9a00a831` vs `d0ccaa6e`; one dump per cell)

| package | plan | wall s | peak RSS MiB | edge recall | marked-unresolved share | wrong-complete |
|---|---|---|---|---|---|---|
| async | base | 0.45 / 0.41 | 88 / 89 | 319/637 / 319/637 | 63.2% / 63.2% | 0 / 0 |
| async | refined | 1.95 / 1.92 | 251 / 250 | 323/637 / 323/637 | 62.9% / 62.9% | 0 / 0 |
| axios | base | 0.60 / 0.62 | 111 / 112 | 99/675 / 99/675 | 75.7% / 75.7% | 0 / 0 |
| axios | refined | 2.14 / 2.16 | 268 / 266 | 99/675 / 99/675 | 75.7% / 75.7% | 0 / 0 |
| chalk | base | 0.05 / 0.04 | 30 / 31 | 2/72 / 2/72 | 75.0% / 75.0% | 0 / 0 |
| chalk | refined | 0.09 / 0.09 | 39 / 39 | 5/72 / 5/72 | 73.3% / 73.3% | 0 / 0 |
| commander | base | 0.28 / 0.24 | 57 / 57 | 14/318 / 14/318 | 94.4% / 94.4% | 0 / 0 |
| commander | refined | 1.73 / 1.69 | 137 / 139 | 117/318 / 117/318 | 75.3% / 75.3% | 0 / 0 |
| dayjs | base | 0.04 / 0.04 | 30 / 31 | 0/117 / 0/117 | 100% / 100% | 0 / 0 |
| dayjs | refined | 0.10 / 0.09 | 34 / 35 | 0/117 / 0/117 | 100% / 100% | 0 / 0 |
| underscore | base | 0.19 / 0.19 | 50 / 50 | 156/262 / 156/262 | 55.2% / 55.2% | 0 / 0 |
| underscore | refined | 0.64 / 0.67 | 113 / 117 | 157/262 / 157/262 | 54.7% / 54.7% | 0 / 0 |
| zod | base | 11.0 / 10.9 | 1,261 / 1,271 | 217/1308 / 217/1308 | 69.5% / 69.5% | 0 / 0 |
| zod | refined | timed out at 600 / 81.5 | 2,337 at kill / 3,634 | n/a / 217/1308 | n/a / 69.5% | n/a / 0 |

Export-fact name precision and recall are unchanged on every package (zod 0.627 / 0.325).

**Noise check.** One bench sample per cell is too few to show a slowdown, so the before binary (`d3f77c0c`) and
the final binary ran six interleaved rounds on every small package and plan. Median wall ms, before vs final:

- async: base 423 / 416, refined 1934 / 1929
- axios: base 617 / 602, refined 2206 / 2174
- chalk: base 52 / 49, refined 94 / 96
- commander: base 251 / 247, refined 1682 / 1663
- dayjs: base 40 / 39, refined 92 / 92
- underscore: base 200 / 204, refined 642 / 642

Every difference is within the before binary's own range. Peak RSS is 1–2 MiB higher in every cell, base
included. The base plan never runs the callable-flow walk, so the shift belongs to the binary, not the walk.

## Tests and checks

- `ts::callable_flow::tests::self_call_walks_grow_linearly_past_the_keying_threshold`: the walks of two
  self-calls are bounded by 8T, and a third self-call adds at most 2T. A walk limit of 16T makes the unbounded
  walk panic. It passes in 0.86 s, and fails in 0.55 s with keying disabled ("walked more callee bodies than the
  limit").
- zod `dump_budget_s = 180` (`159de83c`): `bench.py report` exits 1 when a dump runs over its budget or times
  out. `make js-bench` at `d0ccaa6e` exited 0.
- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` on 1.95: clean with all features, the default
  features, no features, `lang-go` only and `lang-typescript` only.
- `cargo test -p polint --lib --all-features`: 2,826 passed, 20 failed. Every failure is in the before list and
  is a Go export-data-v5 load failure of the local toolchain (go1.27.4).
- `cargo test --workspace --all-features --locked --no-fail-fast` at `d0ccaa6e`: 3,133 passed, 26 failed,
  20 ignored, across the same 40 test binaries as the run before the fix (3,131 / 27 / 20). Every failure also
  fails before the fix, in four targets: the library, `capability_matrix`, `cli` and `golden`. The failure output
  is the Go package loader rejecting the local toolchain's export data ("export data version 5 is greater than
  maximum supported version 4"). `dense_id_sweep_matches_baseline` failed before the fix and passes after
  `53dc3a61`. No test fails after the fix that passed before it.

## Next largest cost and recall gaps

- **The refined-call solver.** It is 58.2% of zod's refined dump in the after-fix profile
  (`derive_refined_calls_with_cache_stats`): `solver_resolved_site` takes 56.1%, and 46% sits in the scan over
  every `ConstraintFact`. For each of zod's 33,960 unresolved calls, `derive_ts_js_refinements` does a linear
  `find` over the 48,838 call sites. `solver_resolved_site` then scans every semantic node and every constraint,
  resolving each constraint's stable key to a string, and every solver edge's contributing facts. That is
  `Theta(U * (S + N + C + E*F))`, quadratic in repository size. Indexing sites, call-site nodes and call
  constraints by id once would make it linear.
- **Memory.** zod's refined dump peaks at 3.6 GiB vs 1.3 GiB for base. The memo does not cause it (the
  first-memo build peaks the same, 3,632 MiB); it is not attributed yet.
- **Recall.** Member calls on object-literal functions and namespace imports: 1,053 of 2,582 missed base edges,
  724 on zod (a separate task). Refined recall on zod equals base recall (217/1308).
- **Unknown reporting.** Sites the base plan stops at an import binding (57 on axios, 25 on zod) have no target
  and no unresolved marker on refined. They are silent misses, not wrong-complete.

## Commits (on `d3f77c0c`, the merge of `fix/exported-initializer-calls` into `fix/refined-false-complete`)

- `9a00a831` bench(js): record the combined baseline before the refined-walk fix
- `53dc3a61` test(arch): re-baseline the dense-id sweep for a callable-flow test message
- `5f97f2b4` perf(ts): walk a callee body once per input in one call tree
- `159de83c` bench(js): a time budget on zod's fact dumps
- `b019a14e` perf(ts): key callee-body walks only past a call tree's first 4096
- `d0ccaa6e` bench(js): record the baseline with zod's refined dump finishing
