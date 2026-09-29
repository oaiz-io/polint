---
quick_id: 260929-iaq
type: quick
status: in-progress
description: metrics-trigger cliff — keep syntax metadata deferred for metrics-only rule checks with identical stable-key ids, and memoize the warm metrics projection
---

# Quick Task 260929-iaq Plan

Work only in `/workspace/polint-metrics-cliff` on `perf/metrics-cliff` (cut from `origin/main`
at `dc3ebdf9`, v0.4.2). Run the GSD quick workflow inline; the owner's unattended brief supplies
the decisions, gates and kill / green-light rules. The design doc is
`thoughts/perf-5.0-levers/10-next-lever.md` §6.1 (research/perf-levers, PR #130). Raw evidence and
harness scripts live outside the repository in `/opt/data/polint-perf5/metrics-cliff/`.

## Tasks

1. Attribute (throwaway spans, never committed): split the warm `polint.metrics` stage
   (`CanonicalMetricsContext::from_db`, `metrics_layer_key`, layer read and validation, restore,
   `refresh_metric_metadata`) and the syntax-metadata cost the deferral loses, on the OAIZ full
   repository, host-direct, 0.4.2 base. Record in `attr.md`.
2. Deferral with metrics: `run_with` keeps syntax metadata deferred when the only requested
   trigger capabilities are the metric ones; the metric facts' own metadata is deferred with it and
   recorded after the syntax rows, so any recording (validation, fallback identity) interns every
   key in the order an eager run does. Gates: eager-versus-deferred identity tests on a metrics plan
   (functions, diagnostics, report, metadata rows with stable-key ids), golden tests, and a
   debug-build run with validation on the OAIZ full repository, normalized-identical.
3. Warm metrics memo: key what the canonical re-projection produces on the syntax providers'
   native output digests (the shape of `27bd045c`), so a warm hit skips it. One test per
   invalidation dimension (syntax change, metrics parameters), each forcing the miss path.
4. Gates after every landed change: `cargo fmt --check`, `cargo clippy --workspace`, grouped
   sequential `cargo test --workspace --locked -- --test-threads 4` with disk cleanup between
   groups, `--format json` byte-stability.
5. Interleaved BASE (`dc3ebdf9` built fresh) vs FINAL A/B on the OAIZ bench copy, three or more
   rounds, medians, stdout sha256 and normalized-diagnostics identity: full repository warm and
   cold, core warm, frontend warm, code-health warm.
6. Kill or green-light per the brief; write `outcome.md`; on green, push the branch (no PR).

## Verification

- Stable-key ids identical between eager and deferred runs of a metrics plan once recorded.
- Normalized-identical OAIZ output, BASE vs FINAL, in every measured workload.
- Gates in task 4 green on the final tip.
- OAIZ hygiene: only basenames, rule ids, timings and counts leave the bench copy.
