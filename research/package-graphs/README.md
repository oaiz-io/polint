# Package Graphs: Direction

Date: 2026-11-17

Baseline: polint v0.5.3 (`7aa3b3cf`).

This folder states what the polint analysis engine is building toward, and
what the current iteration must deliver. It extends
[`research/static-analysis-2.0/`](../static-analysis-2.0/README.md); the
section [What this changes](#what-this-changes-in-earlier-research) lists the
earlier decisions it moves.

- [PACKAGE-GRAPH.md](PACKAGE-GRAPH.md): the stored graph of one package, its
  boundary, stitching, and the correction layer.
- [ITERATION-1-CLASSICAL.md](ITERATION-1-CLASSICAL.md): the current iteration.
  Where the engine stands, the work packages, the success criteria and the
  target benchmarks.
- [VALIDATION.md](VALIDATION.md): the source trail and what was not checked.

The research done before the iteration starts, 2026-11-17:

- [DIAGNOSIS.md](DIAGNOSIS.md): why the classical layer misses what it
  misses, checked in the code and measured. The first diagnosis was
  corrected: the points-to solve is discarded by a budget latch on every
  package above about a thousand call sites, the solver has no call rule,
  and lowering is incomplete in listed ways. Twenty missed edges traced.
- [BASELINES.md](BASELINES.md): Jelly and ACG run on the same packages and
  scored by the same oracle. Jelly static 84.5%, at 13 wrong single-target
  and 5,811 wrong multi-target claims.
- [HELD-OUT.md](HELD-OUT.md): the held-out set, pinned, with the recording
  protocol and what the Jelly PLDI 2024 artifact does and does not offer.
- [TIERS.md](TIERS.md): the closed lists: `CallPrecision` to the two tiers,
  the assumptions an exact claim may carry, the constructs WP2 must lower
  against what MIR carries.
- [CALL-EDGE-TIERS.md](CALL-EDGE-TIERS.md): what the points-to solve claims
  per call site, the inflow kinds of an open site, and the assumptions it
  records on an exact edge.
- [STORAGE-A.md](STORAGE-A.md): the monotonicity argument against this
  solver's real behaviour, and what the stitch must re-derive.
- [TARGETS.md](TARGETS.md): every target kept, replaced or added, with the
  reason.
- [implementation/README.md](implementation/README.md): the pull-request
  sequence for WP0 to WP6 and the map to the GSD phases under
  `.planning/phases/72-…` to `79-…`.

## The goal

A whole program is the application plus every library it depends on, and most
of the code is library code. polint builds one **package graph** for each
package version: its call graph plus an explicit boundary. Package graphs are
built once, stored, and **stitched** into the graph of the whole program.

A published library version never changes. So the cost of making its graph
accurate is paid once, ahead of time, and every program that uses that version
gets the result. This removes the two limits of whole-program analysis:

1. It analyses the same library code again in every run.
2. Its accuracy stops at what one automatic run can find.

The target is a call graph that is complete and exact enough to build on
without rules that work around a weak graph.

## The accuracy ladder

Accuracy is raised in stages, cheapest first. A later stage handles only what
an earlier stage could not settle, and every stage reports what it left open.

| Stage | What it does | Cost | Where it runs |
|---|---|---|---|
| 0. Extraction | Every function and every call site is in the graph. | Parse | polint |
| 1. Classical analysis | Symbol resolution, types, points-to, class hierarchy. Deterministic; nothing is executed. | Seconds per package | polint |
| 2. Recorded execution | Run the package's own tests and load its modules under a call recorder in a sandbox. Gives edges that were seen to happen, and property writes that static analysis cannot see. | Minutes of compute, no model | polint supplies the recorder and the import; the runner is outside |
| 3. Confidence scoring | A fast model scores every edge and every open call site: how likely is this edge wrong, how likely is a target missing here. The scores order the review queue. They never remove an edge. | Milliseconds per edge | polint supplies the features; the model may live outside |
| 4. Agent review | Agents take the queue from the top, read the code, write and run a test where that settles it, and submit a correction with its evidence. | Seconds and cents per item | Outside polint |
| 5. Package models | When the engine misreads the same pattern of one package again and again (a reflection idiom, a registry, a factory), the fix is a model of that pattern or a change to the engine. The edges are then derived, not labelled. | One engineering change per pattern | polint |

Stage 5 feeds stage 1. A correction from stage 4 that the engine later derives
by itself is retired and kept as a regression fixture. This is how expensive
labels turn into a better cheap engine.

## Invariants

These hold for every change to the engine. They are the short form of this
direction; [`AGENTS.md`](../../AGENTS.md) carries the same list.

1. **Evidence on everything.** Every edge says which stage and algorithm
   produced it and at which precision tier. Every call site without a complete
   answer says so and says why. No silent edge and no silent gap.
2. **Engine output is reproducible.** The graph the engine writes is a pure
   function of the package content, the engine version and the model set.
3. **Corrections are a separate layer.** Anything a recorded run, a model or an
   agent adds or rejects is stored beside the engine output, keyed by stable
   identity, with its evidence. It survives a re-scan with a newer engine. A
   correction whose anchor changed is reported as stale and is not applied.
4. **Cheapest first.** Do not send to a later stage what an earlier stage can
   settle. Improve the earlier stage instead.
5. **Graphs compose.** Stitching the graphs of A and B gives the same edges as
   analysing A and B together at the same stage. This is tested, not assumed.
6. **Two tiers, never mixed.** An *exact* edge claims the call reaches this
   function and, where the site is marked complete, no other. A *conservative*
   edge claims the call may reach it. Both are reported and measured apart.
   A wrong exact claim is a defect of the highest order.
7. **Tune set and held-out set.** No stage is tuned on the held-out packages.
   This matters more as models and agents enter, because they overfit faster
   than hand-written analysis does.

## What polint owns

polint is the engine and the formats:

- extraction and classical analysis (stages 0 and 1);
- the package graph format, its export and the stitcher;
- the correction format and the code that applies it;
- the evidence model and the features a scoring model reads;
- the call recorder and the import of recorded edges;
- the package model format.

A service that scans packages at scale, stores their graphs, schedules
sandboxed runs and runs review agents is a consumer of polint. It is not part
of this repository.

## What this changes in earlier research

1. **Package graphs become a primary output.**
   [`static-analysis-2.0/03-summary-store.md`](../static-analysis-2.0/03-summary-store.md)
   made dependency summaries keyed by (package, version) the keystone, and
   kept a remote registry out of the first build. The registry stays out of
   this repository. What changes is the priority: a self-contained package
   graph that can be exported, stored anywhere and stitched is now a
   deliverable of the engine, not only a seam.
2. **Scoring is routing, not pruning.**
   [`static-analysis-2.0/07-ml-integration.md`](../static-analysis-2.0/07-ml-integration.md)
   rules out ML call-graph pruning because it spends recall. That decision
   stands. A confidence score here decides what a reviewer looks at next; an
   edge changes only through a correction that carries evidence.
3. **Agent review is a stage of graph construction.** The 2.0 track placed
   language models at offline summary mining and at triage of findings. This
   direction adds review of the graph itself, behind the cheaper stages.
4. **Per-package models join repo-local models.**
   [`call-graphs/AGENT-EXTENSIBLE-CALL-GRAPHS.md`](../call-graphs/AGENT-EXTENSIBLE-CALL-GRAPHS.md)
   describes models written for one repository. The same mechanism serves a
   model written once for one library and shipped with its package graph.

The tiered resolution, the summary store, the local semantic store and the
honesty rules of the 2.0 track are unchanged.
