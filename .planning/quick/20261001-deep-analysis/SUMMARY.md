---
status: complete
---

This task answered the owner's raised target after v0.4.4: full analysis with data flow and
control flow throughout the whole application. It wrote `thoughts/perf-5.0-levers/13-deep-analysis.md`
and added a post-v0.4.4 update, a correction marker, a file row and an expected-impact row to the
README index.

The report gives the failure chain at v0.4.4, the inventory of missing components, the design
decisions (the sidecar boundary, the middle layer built inside the deep stack, per-unit shards with
export-interface keys), a gated build plan with named OAIZ rules per step, the honest budgets, and
one recommendation: five step-0 repairs now, then typed calls → units and routes → interprocedural
dataflow, each behind a measured gate, with deep providers off the default `check` profile until the
edit-tier gate passes.

Six throwaway measurements back it, all on the private `--shared` OAIZ clone at `2956a791a7`; raw
evidence is in `/opt/data/polint-deep-analysis/`, and nothing was committed.
- **A Go load-mode harness.** The semantic sidecar's whole-module load is 7.4 GB and 21 s because it
  type-checks every dependency from source and builds SSA for all of them; export-data loading gives
  the same in-repo SSA at 1.5 GB and 5.7 s warm, and a one-file edit costs nothing measurable.
- **Whole-module call graphs.** SSA resolves 77% of 199k call sites statically; x/tools' VTA covers
  the interface invokes in 5.8 s.
- **A reproduction of the prior package-scope scan**, with every unknown row classified.
- **The symbol-graph failure.** The Go symbols sidecar never loaded on this host (a synthetic
  `go.work` version bug); with a sidecar built under Go 1.27.1 the scan's unknowns drop from 1,112
  to 64, which corrects the prior round's headline finding.
- **`calls` versus `control_flow`** on the same files: 38 s / 6.1 GB against 6.6 s / 0.5 GB, from
  the abstract-domains materialization.
- **A TS cell** of 100 files: 22.6 s cold, 28.8 s warm, 2.1 GB.

A case-insensitive grep for the forbidden consumer name returned zero hits across the diff.
