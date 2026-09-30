---
quick_id: 260930-1ep
status: complete
date: 2026-09-30
branch: perf/cost-ordered-dispatch
---

# Quick Task 260930-1ep: cost-ordered rule dispatch — Summary

**Date:** 2026-09-30
**Status:** Implemented, measured, gated; branch pushed, no PR, not merged. Verdict: green-light.

## Completed

- Attributed the full-repository rules phase on OAIZ with throwaway spans (never committed).
  `local/backend-endpoint-authority` (~2.45 s) bounds it in every sample; rayon's parallel iterator
  started it 0.01–1.01 s late, anywhere from 2nd to 20th of 23 rules. A pull queue in registration
  order is worse (it is 9th registered: always ~0.8 s late); a queue ordered by one run's times
  starts it within 0.021 s every time and the phase collapses to the rule's own time. Its own time
  does not depend on when it starts. 69 samples across ten different dispatch orders wrote the same
  report bytes.
- The rule host records how long each rule took in a small analysis-cache entry, one per rule plan
  and scope (keyed by the config and rule digests), outside every cache key, digest and report.
- The next pass of the same plan runs a pull queue over the rayon workers: rules without a recorded
  time first, then the longest recorded time down, ties in registration order; one rule per worker
  at a time; rows land in registration-order slots before the unchanged merge and dedupe.
- Without a readable entry (fresh cache, `--no-cache`, missing, corrupt or stale-schema entry, or
  times naming none of the pass's rules) dispatch stays today's parallel iterator over registration
  order. A corrupt entry is evicted and rewritten by the next pass.
- Tests: dispatch order (longest first, unknown first, ties, sequential passes, every recorded order
  of a 12-rule set), the fallback (no times, times naming no rule), a permutation test (12 shuffled
  orders on 1 and 4 workers: identical diagnostics and observed-event counts), panic isolation at
  every queue position, the entry (round trip, per plan and scope, corrupt, malformed, stale schema,
  disabled cache, empty pass), and an outside-user cli test (generated rule pack, real facts,
  `polint check --format json`): the cold, timed, untimed and after-corrupt reports are identical,
  every other cache file is byte-identical with and without the entry, and each run reports the
  order it started rules in. Six mutations (ordering reversed, order ignored by the pass, queue
  without times, reversed fallback, stale schema accepted, runner not passing the times) each fail
  a test.

## Final A/B (v0.4.3 built from source vs this change; OAIZ, host-direct, 4 jobs, interleaved)

Full repository: 15 rounds (7 planned + 8 added after the edit median of the first 7 came in 5 ms
under the 0.4 s bar), cold 25 rounds. Profiles: 5 rounds. Medians; spread = max − min.

| Workload | Tier | BASE (spread) | FINAL (spread) | Delta |
|---|---|---:|---:|---:|
| full repo | warm | 6.12 (1.33) | 5.41 (0.46) | −0.71 s (−11.6%) |
| full repo | edit | 7.86 (1.33) | 7.14 (0.56) | −0.72 s (−9.2%) |
| full repo | cold | 10.54 (1.43) | 10.74 (2.62) | +0.19 s (same dispatch; see below) |
| full repo | rules phase, warm + edit | 3.24 (1.21) | 2.51 (0.37) | −0.72 s |
| core | warm | 3.03 (0.20) | 2.99 (0.34) | −1.4% |
| frontend | warm | 1.22 (0.11) | 1.21 (0.16) | −0.8% |
| code-health | warm | 0.889 (0.14) | 0.904 (0.12) | +1.7% |

- Every sample of every workload wrote the same report bytes in both arms.
- The bounding rule started ≤ 0.023 s into the phase in all 30 FINAL warm and edit samples (BASE:
  late by > 0.5 s in 17 of 30), and its own time is unchanged (2.50 vs 2.52 s).
- Cold has no recorded times, so both arms run the same parallel iterator; the pooled +0.19 s is
  its lottery (late starts 13/25 vs 9/25, Fisher p = 0.39; a cold-only set of 10 rounds gave
  −0.02 s).
- Provider cache counters are identical in both arms on cold, warm and a second warm run.
- Through each arm's own driver (5 rounds): warm 6.52 → 5.74 s, edit 8.33 → 7.58 s.
- The A/B measured the change before a vocabulary gate forced renaming its start-order enum and log
  field; a three-arm confirmation measured the renamed build the same (warm 5.45 vs 5.42 s, edit
  7.21 vs 7.20 s).

Numbers, protocol, every sample: `/opt/data/polint-perf5/cost-dispatch/outcome.md`.

## Commits

| Commit | Change |
|---|---|
| `0fcd8522` | docs(quick-260930-1ep): plan cost-ordered rule dispatch |
| `d33663cc` | perf(rules): start the slowest rules first from the previous run's times |

## Gates

Sequential, grouped, with the cli scratch and the debug target cleaned between groups.

- Final tree `d33663cc` (tree `a4cb54ba`): fmt; clippy (workspace, all targets, all features,
  `-D warnings`); `polint` lib 2,746 passed / 0 failed / 18 ignored; all nine integration targets
  with `--no-fail-fast` — capability_matrix 4, consumer_api_compat 6, github_action_cache 26,
  golden 11, golden_corpus 3, internal_architecture 10, module_layering 1, public_surface_leak 8,
  rule_host_store 2 (71 passed); doctests 1; rustdoc `-D warnings`; MSRV 1.95
  `check --all-targets --all-features`; language-features matrix — no languages / Go only /
  TypeScript only: check, clippy `-D warnings`, lib 2,065 / 2,134 / 2,260 passed, and the
  feature-specific cli filters.
- One revision earlier (tree `95c13436`, before the dense-id fix; not pushed): the cli suite
  188/188 in 10 chunks and `polint-eval` / `polint-bench` / `polint-macros` 45 passed. The final
  tree differs from it only in `core/tests/rule_dispatch.rs` (two assertion messages), which only
  the lib's own unit-test binary compiles.
- Two gate findings were fixed on the way: the framework leak gate bans the word `dispatch` in
  `runner/mod.rs` (renamed the enum and log field to `RuleStartOrder` / `start_order`), and the
  dense-id sweep pins every `{:?}` under `crates/polint/src` (dropped two such assertion messages).
- Mutation checks: six mutations of the ordering, the fallback and the runner wiring each fail a
  test (`/opt/data/polint-perf5/cost-dispatch/logs/mutations/`).
