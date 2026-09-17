# Measurement plan and record

Every number in this file is either measured on the host described below or
explicitly labeled `unmeasured`. No number is an estimate.

## Host

Recorded at measurement time; see the table below for the actual values.

## Plan

### Accuracy

1. **Capability probes** (`tests/capability-probes/`). Run the roll-up on
   `main` and on this branch and compare per-level, per-language rates. The
   suite tests conclusions rather than fact presence, so a typed tier that adds
   edges without changing conclusions shows up as no movement — which is
   itself a result worth recording. New L4 TypeScript probes are added whose
   positive case needs a type-directed conclusion and whose twins must stay
   quiet.
2. **Jelly call-graph lane** (`research/evaluation-harness/suites/jelly-callgraph-micro.toml`,
   oracle at commit `b799ed4f0d68c670fe398830aaa51dd5c628cf74`). Run the
   release tier on `main` and on this branch; report recall, precision and F1
   with the committed baseline (recall 0.6646, precision 0.9742) as the
   reference point. The oracle is 149 `.js`, 23 `.ts`, 21 `.mjs`, 1 `.jsx`
   files, so the TS-only subset is small and the JS majority is only reachable
   through the compiler's JS inference.
3. **Tier attribution.** Count `refined_call_edges` by tier on a real TS repo
   with the tier on and off, so the typed tier's contribution is visible even
   where it does not move an oracle score.

### Speed

1. **Sidecar invocation cost**, cold and warm, on a real TypeScript repository
   (Jelly's own `src/`: 84 files, 23,715 lines, with a `tsconfig.json` and its
   own `node_modules/typescript`). Cold means a fresh `POLINT_CACHE_DIR` and
   `POLINT_CACHE_STORE=off` is not sufficient on its own — a stale cache dir
   replays previous provider failures.
2. **Total pipeline impact**: full scan wall time with the typed tier enabled
   versus the same scan with the sidecar unavailable (fallback path), measured
   by alternating the two binaries sample-by-sample rather than in blocks. This
   host is shared, and block sampling on it has previously invented 20 to 135%
   deltas that interleaved sampling erased.

### Honesty rules applied

- Every accuracy claim names the suite, the commit, and the tier.
- Every speed claim names cold or warm, the repo, and the sample count.
- Anything not run is written as `unmeasured` with the reason.

## Results

See the tables below. Filled in by the implementing session; each row names
the command that produced it.
