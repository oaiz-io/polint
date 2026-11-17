# Validation

Date: 2026-11-17

## Checked against the repository at `7aa3b3cf`

| Claim | Where it was checked |
|---|---|
| Recall, wrong-target and cost numbers | `tests/js-analysis-bench/baseline/report.json` and `report.md`. The scores were also reproduced on `46dc204e` with `bench.py report --no-cost` and TypeScript 5.9.3; they matched on every field outside cost. |
| Missed-edge counts per cause, per plan | Summed from `packages.*.<plan>.causes.counts` in `report.json`. The grouping into four groups is an interpretation. |
| Refined resolves 274 more call sites than base on zod, none on an executed edge | `packages.zod.<plan>.unresolved_calls.without_target` (43,508 and 43,234) and `found` (719 and 719). |
| The constraint vocabulary and the points-to solver exist | `crates/polint/src/analysis_neutral/semantic_graph/constraints.rs`, `crates/polint/src/analysis_neutral/points_to/`. |
| JS/TS flow constraints come from direct bindings and the bounded walker | `crates/polint/src/ts/semantic_graph_build.rs` (`project_ts_callable_flows`, `direct_binding_emits_copy_edge`), `crates/polint/src/ts/callable_flow/mod.rs` and the limits at the top of `extract.rs`. |
| Model edges are an accepted constraint kind | `ConstraintKind::ModelEdge` in `semantic_graph/constraints.rs`. |
| Extraction gaps listed in WP1 | "Decisions recorded by this run" in `baseline/report.md`, and the `not_extracted`, `not_analysed`, `sequence_interop` causes. |

## Not checked

- The 11,000 lines of `ts/callable_flow/extract.rs` were not read in full.
  The statement that complete lowering is the gap, and not the solver, is a
  reading with medium confidence. The first step of WP2 is to confirm it:
  take twenty missed edges from the two largest groups and trace each to the
  missing constraint.
- No outside tool was run on the bench. Jelly's and Feldthaus's numbers are
  quoted from their papers and are averages over other programs.
- The argument that stitching equals whole-program analysis is the standard
  monotonicity argument for inclusion-based analysis. It is not proven for
  polint's solver, which is why the composition test is a gate.
- The cost, stitch and size targets are estimates, not derived from a
  measurement.

## Sources

Accessed 2026-11-17.

| Source | Used for | Note |
|---|---|---|
| Laursen, Xu, Møller. "Reducing Static Analysis Unsoundness with Approximate Interpretation." PLDI 2024. https://cs.au.dk/~amoeller/papers/approx | Recall 75.9% to 88.1% on 36 projects with dynamic call graphs; dynamic property access as the main cause of missed edges | Figures read from the paper's abstract page. |
| Jelly. https://github.com/cs-au-dk/jelly | `--approx`, dynamic call graphs, comparison of call graphs | Tool documentation. |
| Feldthaus, Schäfer, Sridharan, Dolby, Tip. "Efficient Construction of Approximate Call Graphs for JavaScript IDE Services." ICSE 2013. https://2013.icse-conferences.org/content/efficient-construction-approximate-call-graphs-javascript-ide-services.html | Field-based analysis; at least 85% recall and at least 66% precision | The percentages were seen in a secondary copy of the abstract. Read the paper before citing them elsewhere. A replication (https://ir.cwi.nl/pub/23111) reports lower precision when whole edges are compared. |
| Nielsen, Torp, Møller. "Modular Call Graph Construction for Security Scanning of Node.js Applications." ISSTA 2021. https://cs.au.dk/~amoeller/papers/jam | Per-module call graphs that compose | Already cited in `static-analysis-2.0/03-summary-store.md`. The composition mechanics were not re-read in this pass. |
| FASTEN project slides, 2020. https://de.slideshare.net/FastenProject/fasten-presentation-at-sfscon-november-2020 | One stored partial call graph per package version, with unresolved calls, stitched against a resolved dependency tree | Slides, not a paper; status markers are dated. |
| TypeScript 7 release coverage. https://visualstudiomagazine.com/articles/2026/07/08/typescript-7-arrives-to-rock-vs-code-with-go-powered-speed.aspx and https://www.infoworld.com/article/4194567/revving-up-microsofts-10x-faster-typescript-7.html | General availability in July 2026; stable programmatic API in 7.1 | Secondary sources. `static-analysis-2.0/OPEN-QUESTIONS.md` records the same from the TypeScript blog. |
| Oxc. "Type-aware linting stable." https://oxc.rs/blog/2026-07-22-type-aware-linting-stable | A Rust tool that runs type-aware work on the Go port of the checker | Supports keeping a sidecar over writing a checker. |

Earlier research in this repository that this direction builds on:
`research/static-analysis-2.0/` (03, 06, 07 and OPEN-QUESTIONS),
`research/call-graphs/AGENT-EXTENSIBLE-CALL-GRAPHS.md`,
`research/call-graphs/JS-RECALL-BUCKETS.md`,
`research/type-alias-points-to/`, `research/effects-summaries/`.
