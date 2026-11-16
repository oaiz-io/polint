---
quick_id: 261115-p8s
status: complete
date: 2026-11-15
branch: perf/js-callgraph-speed
---

# Quick Task 261115-p8s: JS/TS analysis speed — Summary

**Date:** 2026-11-15

**Status:** Complete. The refined zod fact dump drops from 81.5 s to 37.9 s, and results are unchanged:

- **Dumps:** all 82 bench dumps (41 cases × base and refined plans) are byte-identical to the `e0d4199f` dumps.
- **Digests:** refined zod provider digests are identical.
- **js-bench:** only the Cost section and the version changed. Recall, unresolved and wrong-complete are unchanged.

Profile, ranked causes and what was left alone: [261115-p8s-FINDINGS.md](./261115-p8s-FINDINGS.md).

Scratch and raw evidence are kept outside the repository, under `/home/dev/polint-perf/`:

- `prof/`: stage logs and perf stacks;
- `dumps-*`: reference and per-commit dumps.

## Commits

| Commit | Change |
| --- | --- |
| `347e7c32` | Provider stage timings (wall, RSS, keys, digest) from the JS bench dump under `RUST_LOG=polint::kernel::stage` |
| `abe8a601` | JS/TS call refinement indexes call sites, places and solver-resolved sites once per run (it was quadratic: 39 s of refined zod) |
| `6837a20b` | Unknown collection looks up nodes and call sites through maps instead of a linear scan per edge |
| `c43a0d70` | Narrowing groups places by function instead of scanning every place for each operation |
| `222c9e96` | Four provider output digests write packed parts in parallel bounded batches instead of one heap string per row |
| `f5559d1b` | Evidence and data-flow normalization sorts and validates under one interner read view |
| `9680264f` | Evidence and data-flow fact metadata is built in parallel and recorded in row order |

## Before / after (from `make js-bench`)

The fact dump runs with no cache, one tree per process, and peak RSS taken from `wait4`.

- **Before:** the committed report for the v0.5.x baseline.
- **After:** this branch.

| tree | base ms before → after | base RSS MB before → after | refined ms before → after | refined RSS MB before → after |
| --- | --- | --- | --- | --- |
| axios | 618 → 627 | 117 → 117 | 2158 → 1913 | 279 → 295 |
| zod | 10898 → 10861 | 1333 → 1333 | 81463 → 37860 | 3811 → 3784 |
| commander | 235 → 236 | 59 → 60 | 1691 → 1591 | 146 → 154 |
| async | 410 → 409 | 93 → 92 | 1924 → 1748 | 262 → 296 |
| chalk | 42 → 56 | 32 → 32 | 93 → 91 | 40 → 42 |
| dayjs | 41 → 40 | 32 → 32 | 93 → 110 | 37 → 37 |
| underscore | 191 → 192 | 53 → 54 | 674 → 599 | 122 → 133 |

Small-tree differences of a few milliseconds, and RSS differences of a few tens of MB, are run-to-run noise. On
those trees the parallel metadata and digest work starts the rayon pool earlier. `polint check` cost (census pack)
is unchanged within noise; zod cold is 10.5 s before and 10.8 s after.

## Targets

| Target | Result |
| --- | --- |
| Refined zod under 2× base zod | **Not met.** 37.9 s vs 10.9 s (3.5×; it was 7.5×). |
| Refined memory near base | **Not met.** 3.78 GB vs 1.33 GB. Peak is set by long-lived fact rows and 4.9 M interned keys (FINDINGS). |
| Base zod as low as possible | **Not reduced.** Base zod is `symbol_graph` and `semantic_mir` per-node work with no algorithmic hotspot (FINDINGS). |

## Checks (raw, on `9680264f`)

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | clean |
| Clippy `-D warnings`, five sets: workspace `--all-features`; `-p polint` default; `--no-default-features`; `--features lang-go`; `--features lang-typescript` | clean |
| `cargo test --workspace --all-features --locked` (debug, Go 1.25, `POLINT_REQUIRE_TS_TYPESCRIPT=1`, TypeScript 5.9.3) | all pass (2850 lib tests and the integration suites) |
| `cargo deny check` (0.20.2) | advisories, bans, licenses and sources ok |
| `make js-bench` | only the Cost section and the version changed |
