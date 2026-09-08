---
type: quick-summary
status: complete-with-limitations
---

# Deep-analysis performance research

Excalidraw (MIT, `0dbd2a39319d41fda37b2945dea0dcbd58d6a564`) deep analysis: 233.739 → 78.136 s (2.99×), warm median peak RSS 5.441 → 4.213 GiB (-22.6%).

Retained safe Rust indexes for alias, CFG, local/direct-call data flow and control-dependence evidence, plus lazy evidence payload expansion with byte-equivalence tests. No analysis coverage, precision, budgets, scheduling, API or release-profile change. The failed callable-model restoration is archived and excluded.

The unchanged exact gate remains red on starting revision and final candidate: Jelly F1 0.286866 versus committed 0.790193; Go F1 0.095116 unchanged. The September 7 continuation explicitly authorizes draft publication with this inherited failure disclosed.

Clippy (workspace, all targets/features, locked, warnings denied), formatting and diff whitespace checks pass. All-feature release validation completed every workspace target. Default-profile coverage combines a completed polint library run with targeted recovery of every remaining target after the build directory disappeared during the long library test. The only remaining failures in either profile are the inherited Jelly gate and an absent Excalidraw CLI snapshot; the single original default-profile command did not complete all targets. No scale snapshots exist at the starting revision and the test harness is unchanged. No baseline or discovery changes are included. The workspace is not fully green. Semantic accuracy case bytes match the starting revision. Deep measurements retain existing provider/setup failures; documented Go sidecar setup errors limit their semantic-coverage claim. Golang/tools deep attempts were killed without completing a sample; no speedup or RSS ratio is available. Actual container limits are six CPUs and 12 GiB RAM.

Research evidence, raw samples, excluded experiments, reproducibility scripts and the final publication result live under `.context/perf-research/`. The PR body carries the report for remote review. Next structural lever: share composite stable-key structure and stream canonical bytes; No end-to-end 10× result was demonstrated.
