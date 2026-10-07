---
quick_id: 261007-r1v
status: complete
date: 2026-10-07
branch: perf/deep-analysis
---

# Quick Task 261007-r1v: final review fixes for the deep-analysis build — Summary

**Date:** 2026-10-07
**Status:** Review complete; two findings fixed on the branch; every load-bearing claim re-derived on the fixed tip.
Raw evidence outside the repository: `/opt/data/polint-deep/final-review.md`, `/opt/data/polint-deep/review/`.

## Findings

1. **Data-flow setup gap (should-fix, fixed; `3a9a84d8`).** On a Go-only scan whose typed frontend loaded no
   package, a rule requesting `dataflow` ran against no flow program: `DataFlow::flows` answered an empty answer with
   `is_complete() == true` and `DataFlow::forbidden` answered nothing (v0.4.4 answered it from the value-flow graph,
   which a Go-only data-flow plan no longer builds). Such rules are now blocked with the `polint/capability`
   `setup_missing` diagnostic `go_types` and `routes` get, `polint unknowns` reports the same cause, and in a mixed
   scan `flows` reports `FlowUnknown::NoProgram`. Kernel and SDK unit tests, an outside-user `polint check` test;
   both new tests fail with the fix reverted (mutation-checked).
2. **Route completeness documentation (fixed; `bfa45724`).** The `use` role applies gin's semantics to every
   framework and Watermill's per-handler `AddMiddleware` is not modelled, so a subscription can report an empty chain
   with `middleware_complete` true. Stated in `docs/facts/routes.md`; the move of framework semantics into model
   parameters is the route-model follow-up, not this branch.

No blocking finding. Nits noted, not changed: `CallGraph::edges` has an `expect` on the edge count (a rule panic
becomes a diagnostic); `CallEdgePrecision::at_least` reads as a reversed comparison until one sees the enum order.

## Re-derived on the fixed tip (OAIZ bench, 4 jobs; the box was shared with two other sessions, so timing samples
carry their foreign load; memory peaks are not affected by contention)

| claim | re-derived |
|---|---|
| reports byte-identical for rules requesting no deep capability | base 691 vs final 649 diagnostics; the 43 base-only rows are `parser/go` on `new(` lines in 43 files, the 1 final-only row is the metrics signal on a file that parsed completely for the first time; summary differs in that one counter only; the other 648 diagnostics identical and in the same order; each arm identical cold vs warm |
| no deep provider in the default `check` profile | the OAIZ host on the full bench schedules `polint.go.syntax`, `ts.syntax`, `module_graph`, `symbol_graph`, `metrics` only (11 s, 0.67 GB) |
| taint corpus 0.946 / 1.000; L4 10/10, twins 20/20 | 53 cases: tp 53, fp 3, fn 0 → 0.946 / 1.000; L4 Go 10/10, 20/20 |
| mutation checks | budget unknown dropped, value calls ignored, summary confidence cap dropped, and the two review fixes reverted: all five caught |
| call-graph edges identical across job counts | 140,175 edges (static 103,799, VTA 18,505, CHA 17,055, type-hierarchy 811, syntactic 5) byte-identical at jobs 1, jobs 4, and warm jobs 4 on the jobs-1 cache |
| route inventory | 292/292 original sites found, 0 original-only, 13 typed-only, 93 reclassified protected; endpoint-authority 4 diagnostics |
| gate 0 (a) catalog `control_flow` cold | final 11.0 s (clean) / 11.8–15.5 s (contended), 2.0 GB; base 31–52 s, 7.5–8.3 GB |
| gate 0 (b) catalog `calls` warm | final 0.2 s / 0.01 GB (the call-resolution cache restores everything), 37 rows = base's 64 minus the 24 dynamic-property and 3 function-value sites the typed frontend resolves; base 25–33 s / 6.10 GB |
| gate 2 whole-core `calls` | cold 116 s under 6.5 foreign cores and 741 s of throttling (3.80 GB); warm 6.7 s / 0.81 GB |
| gate 3 whole-core `dataflow` | cold 68.7 s / 4.17 GB, warm 25.4 s under 5.9 foreign cores / 3.41 GB; both reports md5 `9cf8f6e4`, the implementer's; all five questions `complete=false` with 555–713 `CallDepth` and 56–138 `UnresolvedCall` unknowns |

## Gate suite on the final tip
`/opt/data/polint-deep/review/suite/summary.txt` at `bfa45724`: fmt, clippy (all features and none/Go/TS), rustdoc,
lib 2,840 / none 2,128 / Go 2,222 / TS 2,323, doctests, every integration target alone (capability_matrix,
consumer_api_compat, github_action_cache, golden 11/11, golden_corpus, internal_architecture, module_layering,
public_surface_leak, rule_host_store), other crates, MSRV 1.95.0, the Go sidecar's build, vet and tests: all green.
The cli tests on the changed surfaces found the new outside-user test over-strict (it counted one capability
diagnostic where the dependency capabilities add their own setup rows without a module root); fixed in `00029e34`
and green with the existing data-flow cli test. The 192-test cli batch suite is left to CI.

## CI on the pull request (#133)
The first CI run on the branch (none had run before the PR) failed two jobs: the language-feature matrix runs the
library suite without a Go toolchain, where ten new typed-frontend tests failed instead of skipping and the warm-restore
symbol test expected a layer-cache hit for a setup-missing graph that this branch deliberately never caches; and the
macOS lib job, where the subprocess drain test's 2.5 s bound was under the hosted runner's writer time (2.8 s) while the
fixed-sleep behaviour it guards against needs at least 5.12 s. Fixed in `bd696dbc` (expect the recompute, bound 4.5 s) and
`6c62513c` (the hosted runner has a Go toolchain but cannot build the embedded frontend, so the skip reads the
kernel's result: a run whose typed frontend loaded no package skips with a note, the sidecar-backed symbol tests'
convention). Locally the Go-only suite passes 2,222 tests without a toolchain, the guarded tests skipping (10) or
passing (4).
