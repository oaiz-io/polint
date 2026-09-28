---
quick_id: 260927-fk1
status: complete
date: 2026-09-27
branch: performance-improvements-5.0
---

# Quick Task 260927-fk1: perf-5 campaign — Summary

**Date:** 2026-09-27
**Status:** Implemented, measured, gated; branch pushed, no PR, not merged.

## Completed

- Measured a Phase-1 baseline before any change: Plinty and OAIZ (the owner's `polint-core` and
  `polint-frontend` commands, plus the full-repository scan as a record), ice-cold, cold and warm tiers,
  three runs each, through the real driver at the container's 4 jobs.
- Profiled before optimizing (local instrumented builds, never committed) and landed five measured,
  byte-identical optimizations, one per commit:
  - the SQLite semantic-store backend compiles only in tests (−15.6% OAIZ rule-host build);
  - the canonical Go syntax digest is memoized by native identity (warm projection 230/439 → 2 ms);
  - syntax-only rule checks restore syntax facts without building their metadata;
  - Go string-literal extraction prunes import subtrees instead of climbing parents per literal;
  - syntax-only rule checks skip the canonical Go projection nobody reads.
- A read-only code review of the branch found no product defect; its test and gating findings were
  fixed in a follow-up commit, and the CI feature matrix, rustdoc and MSRV jobs were added to the gates.
- Interleaved BASE (`bbd1b785`) vs FINAL (`e7bf1e52`) A/B through the real driver, three rounds, every
  tier, every workload: stdout byte-identical to the baseline in every run.

## Final A/B (medians of 3 interleaved rounds, 4 jobs)

| Workload | Tier | BASE `bbd1b785` (s) | FINAL `e7bf1e52` (s) | Delta | Speedup |
|---|---|---:|---:|---:|---:|
| Plinty full repo | ice | 259.20 | 224.35 | -13.4% | 1.16x |
| Plinty full repo | cold | 3.57 | 2.73 | -23.8% | 1.31x |
| Plinty full repo | warm | 2.26 | 1.44 | -36.1% | 1.56x |
| OAIZ core | ice | 263.81 | 228.77 | -13.3% | 1.15x |
| OAIZ core | cold | 7.51 | 6.15 | -18.1% | 1.22x |
| OAIZ core | warm | 5.97 | 4.73 | -20.8% | 1.26x |
| OAIZ frontend | ice | 258.98 | 223.98 | -13.5% | 1.16x |
| OAIZ frontend | cold | 2.10 | 1.53 | -27.2% | 1.37x |
| OAIZ frontend | warm | 1.74 | 1.37 | -21.3% | 1.27x |

Stdout byte-identical to the Phase-1 baseline in all 54 runs; warm analysis peak RSS 208 → 154 MB (Plinty), 346 → 282 MB (OAIZ core), 185 → 134 MB (OAIZ frontend).

## Not shipped (owner decisions, measured)

Prebuilt engine / thin SDK (ice-cold), rule-result memoization (blocked by rules reading the filesystem),
per-file rule execution, binary layer-cache encoding + faster integrity hash (protocol bump), rule-host
builds without thin-local LTO (−22% build, +5–11% analysis), engine crate split. Details and expected
returns: `/opt/data/polint-perf5/REPORT.md`.

## Consumer-side finding

OAIZ's pack computes spans with a quadratic helper; fixing it on a scratch copy took the full-repository
scan from 598.7 s to 12.1 s and `polint-core` warm from 5.79 s to 3.86 s with identical output.

## Commits

| Commit | Change |
|---|---|
| `373e1ed4` | perf(build): compile the semantic store's SQLite backend only in tests |
| `90bd0f50` | docs: campaign plan |
| `27bd045c` | perf(kernel): memoize the canonical Go syntax digest by native identity |
| `9b1f116a` | perf(kernel): defer syntax-fact metadata for rule checks that read none of it |
| `4eb0ebb0` | perf(go): find import paths by pruning the walk, not by climbing parents |
| `ad82246a` | perf(kernel): skip the canonical Go projection when nothing reads the identity |
| `e7bf1e52` | test(kernel): validate deferred-metadata runs on a recorded copy |

## Gates on the final tip

fmt; clippy (workspace, all targets, all features, `-D warnings`); `polint` lib 2720 passed; integration
71 passed (9 targets); cli 187 passed; `polint-eval`/`polint-bench`/`polint-macros` 45 passed; rustdoc
`-D warnings`; MSRV 1.95 check; language-features matrix (no languages / Go only / TypeScript only:
check, clippy, lib 2042 / 2110 / 2236 passed, feature-specific cli tests).
