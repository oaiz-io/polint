---
quick_id: 261003-u2r
status: complete
date: 2026-10-03
branch: perf/deep-analysis
---

# Quick Task 261003-u2r: deep-analysis step 2 — Summary

**Date:** 2026-10-03
**Status:** Implemented and measured; gate 2 PASS on five criteria, the one-file edit tier FAILED and the kill
path is taken. Branch pushed, no PR.

## Completed

- **Routes in the typed frontend.** The semantic sidecar interprets route setup from every program's `main`:
  routers followed through groups, `Use` calls in program order, registrar helpers, struct fields and package
  variables; gin, chi, net/http (with `httptest`) and Watermill built in; a repository adds models as
  `[[go_route]]` tables (part of the sidecar cache key; invalid tables are `polint/route-model` warnings).
  Functions that lead to route setup are found through calls of function values (value-taken functions of the
  same signature) and interface calls (their implementations), so setup reached through a table of run modes is
  interpreted. A step-budget stop is a routes-only unknown.
- **SDK.** `Routes<'_>` with `iter`, `http`, `messages`, `handled_by`, `served_from`, `complete`; each route
  carries its handlers and middleware chain (functions, literals, factories, fields) with path and middleware
  completeness. `docs/facts/routes.md`, outside-user CLI test, prelude +5 with a promotion record.
- **Layer cache.** A miss used to compare the stale manifests' dependency edges by walking the layer keys'
  shared digest lists on every comparison (quadratic: a one-file edit spent 176 s in three graph layers); keys
  that share their lists now compare without walking them.

## Gate 2 (whole OAIZ `core`)

| criterion | gate | measured |
|---|---|---|
| warm no-change | ≤ 10.4 s | 5.7–5.9 s |
| one-file edit | ≤ 15 s | 41.7 s; the sidecar's whole-program variable-type analysis alone sets a 15.8 s floor → **FAIL, kill path** |
| cold | ≤ 90 s / ≤ 4 GB | 43.5 s / ≤ 3.84 GB |
| byte-identical reports | 1/4/6 jobs + ten permutations | identical (driver and host) |
| route inventory | superset, every difference explained | 292/292 + 13 extra, 93 reclassified, all explained |
| endpoint-authority rule CPU | < 1 s | 0.49 s |

Kill path: deep providers stay out of `check`'s default profile (review/CI only; no default-profile rule requests
a deep capability), and step 3 is re-scoped to the review path: its edit criterion is reported, not binding.

## Deviations

- Per-unit binary shards and per-package sidecar shards are not built: their purpose is the edit tier, which the
  whole-program variable-type-analysis floor keeps above 15 s with or without them; the warm criterion is met by
  the call-resolution cache.
- The unit DAG and SCC-ordered summary closure move to step 3, where the data-flow summaries are built.

## Verification

- Full gate suite on the final commit: fmt, clippy (all features and the none / Go / TS sets), rustdoc, lib,
  doctests, every integration target alone, the cli suite in batches, the go sidecar tests. It found two lint
  findings in the layer-key tests (redundant clones, a Debug-formatted duration the dense-id sweep rejects), fixed in
  the commit. Every cli failure was the shared host running out of something (disk, memory for a rule host's compile,
  the container's task limit, or another session's `/tmp/.gitignore` hiding every temp repo) and passed when re-run
  alone with temp dirs off `/tmp`. The golden characterization suite could not finish: all three attempts died
  linking a rule host with the disk at 0-5 GB free (ld/lld bus errors). Step 2 changes no path those outputs
  exercise; golden is required to pass at the step-3 tip, which contains all of step 2.
- Mutation checks: the route-relevance fix (function values and interface calls) and the layer-key order fast
  path (scale test 1.0 s vs 55 s) each fail their tests when reverted.
