---
quick_id: 261102-v2x
status: complete
date: 2026-11-02
branch: planning/v2.1-engine-roadmap
---

# Quick Task 261102-v2x: v2.1 engine research and roadmap rewrite — Summary

**Date:** 2026-11-02 on `main` at `338b570b` (v0.5.0). Planning-only; no product code changed.

## Completed

1. **Position assessment.** Engine substrate complete (v1.2–v1.4, #133 Go deep analysis, #129–#132 perf campaign). Measured gaps: one-file edit tier equals cold (41.7 s calls / 68.2 s dataflow on OAIZ `core`), TS/JS a tier behind Go (no flows, typed calls or routes; gated Jelly F1 0.79), v2.0 store stalled with phases 63–65 shipped, six open kernel identity/outcome bugs, no `polint graph`.
2. **Research report** in `.planning/research/v2.1-engine/` (index + five notes, each with file:line evidence from four read-only code investigations): Go edit-tier latency, TS/JS deep parity, semantic store re-scoping, thin-SDK prebuilt engine, kernel correctness issues #86–#91.
3. **Plan rewrite.** v2.0 roadmap/requirements archived to `.planning/milestones/v2.0-*` with a disposition entry in `MILESTONES.md`. New `ROADMAP.md`: milestone v2.1 Engine Depth, 18 phases (72–89), one pull request each, with goal, scope, outcome, gate and kill criterion; eight milestone gates G1–G8. New `REQUIREMENTS.md`: 38 requirements mapped 1:1 to phases plus a carried-forward table for every v2.0 ID. `PROJECT.md` and `STATE.md` updated to the new position.

## Key findings that changed the plan

- STATE.md under-reported Phase 65: plans 01–06 are on `main` (PR #94). The SQLite store is test-only and owns no fact; files + an index are enough for the first persistence win.
- The VTA "floor" (15.8 s) is 2.7× the research-harness number for the same algorithm; GC pressure under the sidecar memory limit and a duplicate CHA index are the leading hypotheses, so instrumentation comes before sharding.
- The edit tier has never been measured by the harness; the first engine PR adds the edit cell.
- TS flow programs should be lowered in Rust from Oxc (no Node), with the type sidecar as enrichment only.
- #86, #90, #91 block both persistence and a public graph CLI; they go first.

## Not done / follow-ups

- Nothing executed from the plan; Phase 72 is next.
- Stale GitHub items to close at Phase 89 or earlier: #49, PR #126.
