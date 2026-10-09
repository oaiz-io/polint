---
quick_id: 261002-t1c
status: complete
date: 2026-10-02
branch: perf/deep-analysis
---

# Quick Task 261002-t1c: deep-analysis step 1 — Summary

**Date:** 2026-10-02
**Status:** Implemented, measured, gated (gate 1 PASS on every line after the kill path); branch
pushed, no PR.

## Completed

- **Typed Go call facts.** The semantic sidecar emits per-site static callees, variable-type-analysis
  candidates for interface and function-value calls (class-hierarchy fallback; a type-hierarchy edge
  to the abstract callee above 16 candidates), `go`/`defer` modes, builtins, conversions, and per
  package interfaces, implements pairs, method sets, fields with tags, parameters and
  instantiations. The calls layer joins them by call-expression span; typed answers decide the sites
  they cover. A call of a function literal is an edge to the literal, not to its declaring function;
  generic instances are named with aliases resolved, so labels do not depend on SSA scheduling. A
  typed dynamic call keeps at most `[solver.go] max_candidates_per_callsite` candidates and records a
  budget stop for the rest.
- **Unit split.** Compact identities (long part values embedded as digests), per-file parallel Go MIR
  lowering with interner overlays, parallel CFG lowering and derived rows with identical ids and keys,
  an exact numbered-part SCC fixpoint (differential-tested against the string join), indexed joins
  for refined calls, summaries, semantic graph, identity and domains. Call-only plans skip Go
  points-to, derived CFG relations, abstract domains, summaries and Go CFG bodies when typed facts
  exist; Go symbols load only on direct request; the sidecar builds its call graph only for plans that
  read calls. A whole-scan call-resolution cache restores Go-only call plans; the sidecar's soft memory
  limit is `min(available / 4, 2 GiB)`.
- **SDK.** `CallGraph<'_>` and `GoTypes<'_>`, requestable as `Option<View<'_>>`; `go_types` is
  setup-missing when the typed frontend loaded no package. `docs/facts/call-graph.md`,
  `docs/facts/go-semantic-types.md`; outside-user temp-repo tests.
- **Rules.** The four named rules rewritten on scratch copies: endpoint-authority +4 diagnostics, all
  cross-package exact call chains the textual fixpoint cannot see (true positives by the rule's own
  policy); the other three identical, with side-by-side decision diffs showing the typed decisions
  agree with or improve on the textual ones.

## Gate 1 (whole OAIZ `core`, 2,846 files, 117,307 in-repo Go call sites)

| criterion | gate | measured |
|---|---|---|
| `calls` cold | ≤ 60 s, ≤ 4 GB | 44.1 s median of clean samples, ≤ 3.83 GB in all 18 samples |
| `calls` warm | ≤ 20 s | 5.7 s, ≤ 0.81 GB |
| unresolved non-conversion sites | ≤ 1 % | 0.48 % |
| VTA edges across job counts | byte-identical | all 140,175 edges identical at 1, 4 and 6 jobs |
| four rules rewritten | zero unexplained diffs | +4 explained, else identical |

Before the split the step-0 driver was killed at 465 s / 9.02 GB; the first complete run of this step
took 342 s / 10.85 GB.

## Verification

- Full gate suite on `e01d2bad`: fmt, clippy (all features), rustdoc, lib, every integration target
  alone, doctests, the cli suite in batches, the other crates, MSRV, the none / Go / TS matrix. It
  found clippy errors in the single-language sets, one deliberate precision change in a cli
  expectation, and the lost candidate cap; each is fixed in its own commit and re-verified (lint
  matrix, lib default / none / Go / TS, the two cli tests).
- Mutation checks: the literal-callee fix, the instance-naming fix, the SCC fixpoint and the candidate
  cap each fail their tests when reverted.
