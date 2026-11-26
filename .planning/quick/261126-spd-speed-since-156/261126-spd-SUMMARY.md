---
quick_id: 261126-spd
status: complete
date: 2026-11-26
branch: perf/speed-since-156
---

# Quick Task 261126-spd: JS/TS analysis speed since #156 — Summary

**Status:** Complete. Five algorithmic costs that #156 introduced or now hit
at scale are removed. Every bench fact dump is byte-identical to `main`, and
no package is slower in wall time on the base or refined plan.

## Commits

| Commit | Change |
| --- | --- |
| `fae7f289` | `SourceTextIndex::line_col` finds a column from per-64-byte continuation-byte counts instead of counting the characters of the line (quadratic on minified files) |
| `e4e109f2` | TS MIR lowering shares each file's closure tables between its functions (they were cloned into every function) and finds branch operations by position |
| `00d22767` | Entrypoint recognizers look MIR places up through the store's id index instead of scanning every place per call site |
| `ec6174fe` | Callable-flow walk: the tables of `FlowEnv` and `ObjectTargets` are shared until written instead of copied per callee walk and per member read |
| `0a4db389` | Callable-flow passes parse and walk each file on its own worker; the serial second parse of every file is gone |
| `e6950369` | Dense-id sweep re-baselined for three test-only `{:?}` hits in the new tests |

## Verification

- `make js-bench JS_BENCH_ARGS="--no-cost --no-types --out DIR --keep-dumps DIR"`
  on `origin/main` (`e39d74bc`) and on the branch: all 247 JSON fact dumps are
  byte-identical (`cmp`); the manifests differ only in the temporary directory;
  `report.json` differs only in timing and memory fields.
- Speed, interleaved main/branch on whole package trees with `perf stat`
  instruction counts, an untimed warm-up before each pair and runs started at a
  one-minute load below 12 (3–12 seen): fewer instructions on every package and
  plan (base −7% to −70%, refined −4% to −53%); median wall time lower on every
  package and plan. Extra load-gated sets for the cases a busy first matrix left
  open: dayjs base 20/20 pairs faster (load 3–6), zod base 6/6, async and dayjs
  refined 6/6, pako refined 4/4.
- Peak RSS: within ±1% except axios refined, +3.5–4% (1,804 → 1,868–1,881 MB).
- `make check`: `lint`, `doc`, `install-smoke` and `deny` pass. `cargo test
  --workspace --all-features --no-fail-fast` fails the same 26 tests on the
  branch and on unmodified `main` on this machine (Go RTA, Go types, routes
  and data-flow views, the Go golden and taint corpus: they depend on the local
  Go toolchain); everything else passes. The dense-id sweep baseline gains
  three test-only hits from the new tests (`e6950369`).

The investigation behind these fixes (per-stage breakdown, profiles, and the
further fixes not taken here) is summarised in the pull request description.
