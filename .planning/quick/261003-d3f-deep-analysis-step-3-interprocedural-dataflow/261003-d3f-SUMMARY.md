---
quick_id: 261003-d3f
status: complete
date: 2026-10-04
branch: perf/deep-analysis
---

# Quick Task 261003-d3f: deep-analysis step 3 — Summary

**Date:** 2026-10-04
**Status:** Implemented and measured; gate 3 PASS (corpus, L4 probes, cold, warm, budget visibility); the one-file
edit tier is reported and fails (68.2 s, not binding after the gate-2 kill). Branch pushed, no PR.

## Completed

- **Flow programs.** The semantic sidecar (`--dataflow`) emits one flow program per function it builds from SSA:
  value slots, copies, loads and stores through field, element and pointer steps, calls with candidate callees and
  the algorithm that found them, returns and closures. Statically dead branches are pruned before variable-type
  analysis, and calls through a function value are marked so their edges are may-call edges.
- **Solver.** Access paths of two field steps (three on request), per-(function, entry) summaries reused across
  callers, recursive cycles recomputed until nothing changes with equality backdating, returns to callers, package
  variables, closures, library functions by model or by a conservative default, per-package step budgets and a
  deadline, path recovery to one flow per sink site, precision labels.
- **Models as data.** Built-in sources, sinks, sanitizers, opaque functions and propagators; repository
  `[[go_flow_*]]` tables in `.polint/models/*.toml`, part of the analysis digests.
- **SDK.** `DataFlow<'_>::flows(&FlowSpec)` → `FlowAnswer` (flows with source, sink, steps, precision and unknowns;
  run-wide unknowns; `is_complete`); `Flow::diagnostic` with located path evidence rendered as SARIF code flows;
  `DataFlow::forbidden` answered by the solver for Go. Docs (`docs/facts/data-flow.md`), promotion record,
  outside-user CLI test, generated skill text.
- **Caching.** A Go-only data-flow plan builds no value-flow graph, CFG bodies, domains or summaries, and restores
  the call-resolution cache on unchanged sources while still loading flow programs.
- **Taint corpus.** `tests/taint-corpus`: 53 cases and 54 must-not-report twins from the consumers' error-flow,
  transaction, context and tenant-scope rule families plus injection, precision traps and documented limits.

## Gate 3 (whole OAIZ `core`, five data-flow questions)

| criterion | gate | measured |
|---|---|---|
| taint corpus | precision ≥ 90 %, recall ≥ 70 % | 0.946 / 1.000 (3 false positives = documented limitation twins) |
| L4 probes | ≥ 9/10 Go positives, all twins clean | 10/10, twins 20/20 |
| cold | ≤ 120 s / ≤ 6 GB | 70.0 s median / ≤ 4.25 GB |
| warm | ≤ 25 s | 15.0 s (36-44 s before the call-cache fix), reports byte-identical |
| one-file edit | ≤ 20 s (reported, not binding) | 68.2 s → FAIL; whole-program sidecar floor 26-30 s |
| budget trips | visible as unknowns | yes: every whole-core answer reports `CallDepth` unknowns, flows carry their own |

## Deviations

- The solver runs over SSA flow programs the sidecar emits, not the tree-sitter MIR (which drops literal arguments,
  receivers, `var` initializers and multi-value assignments); summaries are demand-driven per question rather than
  persisted per unit; one flow per sink site; sources modelled by framework API rather than derived from routes.
- Summaries nest at most 48 frames: on the bench every whole-core answer is incomplete; reported, not hidden.

## Verification

- Full gate suite at the step's code tip: fmt, clippy (all features and none / Go / TS), rustdoc, lib, doctests,
  every integration target alone (golden included, owed since step 2), the cli suite in batches (192), other
  crates, MSRV, the none / Go / TS matrix, the Go sidecar's tests. It found two real defects, fixed: flow evidence
  rejected by the evidence schema (no SARIF code flows) and an internal fact-family name in public docs.
- Mutation checks: summary equality backdating (both directions), budget unknowns, untracked value kinds, the
  summary-tier confidence cap, value-call may-edges (Rust and sidecar), compact domains, and the call-cache restore
  of flow programs; each caught.
- `calls` on the bench is unchanged except 309 edges through function values relabelled may-call (variable-type);
  `polint unknowns --cap calls` byte-identical to step 2.
