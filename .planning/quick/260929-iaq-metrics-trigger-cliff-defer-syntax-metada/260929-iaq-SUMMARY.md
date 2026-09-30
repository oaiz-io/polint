---
quick_id: 260929-iaq
status: complete
date: 2026-09-29
branch: perf/metrics-cliff
---

# Quick Task 260929-iaq: metrics-trigger cliff — Summary

**Date:** 2026-09-29
**Status:** Implemented, measured, gated; branch pushed, no PR, not merged. Verdict: green-light.

## Completed

- Attributed the cliff on the OAIZ full repository with throwaway spans (never committed). The warm
  metrics stage was the canonical re-projection (~0.71 s), metric metadata (~0.56 s) and the layer's
  own I/O (~0.35 s). The lost deferral was ~1.0 s of Go and ~0.7 s of TypeScript syntax metadata,
  plus ~0.17 s of finishing the stable-key owner maps; the canonical Go projection cost ~0.6 s on
  every cold or edited run.
- Rule checks whose only trigger capabilities are the metric ones keep syntax metadata deferred. The
  metric facts' metadata is deferred with it and recorded after the syntax rows, so any recording
  interns every key in the eager order: the same rows and the same stable-key ids.
- The same checks skip the canonical Go projection, which nothing they run reads.
- The metrics provider records which layer an earlier run validated or wrote for one set of inputs,
  keyed by a digest of every field the projection reads, so a warm hit skips the projection.
- Eager-versus-deferred identity tests on a metrics plan: kernel level on real Go and TypeScript
  sources, cold and warm caches; database level with isolated stable-key interners, so the ids are
  really compared (mutation-checked). One memo test per invalidation dimension, each
  mutation-checked. A debug-build run with validation on the OAIZ full repository wrote a report
  byte-identical to release.

## Final A/B (v0.4.2 built from source vs `01ff2227`; OAIZ, host-direct, 3 interleaved rounds, medians)

| Workload | Tier | BASE (s) | FINAL (s) | Delta | Speedup |
|---|---|---:|---:|---:|---:|
| full repo | cold | 13.70 | 10.40 | −24.0% | 1.32x |
| full repo | warm | 9.05 | 6.00 | −33.7% | 1.51x |
| full repo | edit | 9.94 | 7.16 | −28.0% | 1.39x |
| code-health | cold | 3.94 | 2.56 | −35.0% | 1.54x |
| code-health | warm | 1.99 | 1.01 | −49.4% | 1.98x |
| code-health | edit | 2.74 | 1.52 | −44.6% | 1.80x |
| core | cold | 4.46 | 4.28 | −3.9% | 1.04x |
| core | warm | 2.94 | 2.97 | +1.2% | 0.99x |
| frontend | cold | 1.48 | 1.49 | +0.3% | 1.00x |
| frontend | warm | 1.21 | 1.22 | +1.2% | 0.99x |

Reports byte-identical in every sample; warm peak RSS on the full repository 963 → 524 MB. Core and
frontend are syntax-only profile runs no changed path touches; their deltas are within the samples'
spread. Through the real driver (each arm's own driver and engine), full-repository warm went from
9.67 s to 6.38 s. Numbers, protocol, and retakes:
`/opt/data/polint-perf5/metrics-cliff/outcome.md`.

## What is left

Measured in the same session, the FINAL engine with and without the metrics rule: warm ~0 at the
wall, cold +1.5 s, edit +1.0 s. The rest is the metrics provider's miss path (projection, output
digest, a 47.6 MB layer write), which nothing in a metrics-only rule check reads the identity of.
Deriving the metrics straight from the restored facts is the next lever. It is an owner decision
because it changes what the metrics layer is for.

## Commits

| Commit | Change |
|---|---|
| `9df9345d` | docs(quick-260929-iaq): plan the metrics-trigger cliff fix |
| `49679038` | perf(kernel): keep syntax metadata deferred for rule checks that request only metrics |
| `e9e988b2` | perf(kernel): skip the canonical Go projection for metrics-only rule checks |
| `01ff2227` | perf(metrics): memoize the metrics layer by its inputs so warm runs skip the projection |
| `9b79285e` | docs(kernel): keep each deferral helper's doc comment on its own item |

## Gates

- Per change: fmt; clippy (workspace, all targets, all features, `-D warnings`); the kernel and
  metrics suites; an OAIZ full-repository A/B with a byte-identical report. `49679038` and `e9e988b2`
  also passed the `polint` lib suite (2,724) and the 9 integration targets.
- `01ff2227`: the full suite — lib 2,727 passed; integration 71 passed (9 targets, golden and
  golden_corpus included); cli 187/187 in 10 chunks; `polint-eval`/`polint-bench`/`polint-macros`
  45 passed; no solo re-runs.
- `9b79285e` (comments only on top of `01ff2227`): fmt; clippy; lib 2,727; integration 71; rustdoc
  `-D warnings`; MSRV 1.95 check; language-features matrix (no languages / Go only / TypeScript only:
  check, clippy `-D warnings`, lib 2,046 / 2,115 / 2,241 passed, feature-specific cli tests).
- A debug build with validation on the OAIZ full repository (BASE and FINAL, cold and warm):
  byte-identical to release, no `polint/internal` diagnostics, no assertion failures.
