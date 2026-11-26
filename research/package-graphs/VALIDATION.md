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

## Checked on 2026-11-17, after the direction was written

| Claim | Where it was checked |
|---|---|
| The points-to solve is discarded on packages above about 1,000 call sites by a global budget latch | `analysis_neutral/points_to/solver.rs` (`add_all`, `object_slot`, `to_result`), `ts/points_to.rs:148-153`; fresh base and refined dumps of axios, async, zod and commander on `7aa3b3cf` (0 resolved `PointsToAssisted` edges on the first three, 161 on commander); `polint inspect unknowns --no-cache .` on axios reports `budget_exceeded: points_to.max_dynamic_vars,solver.max_steps`. [DIAGNOSIS.md](DIAGNOSIS.md) §1. |
| The solver has no interprocedural rule | `solver.rs:93-106`, `ts/points_to.rs`, `ts/mir/lower.rs:3734` (call callee lowered as `Unknown`). DIAGNOSIS §2. |
| The walker binds arguments only for same-file identifier callees and never walks closure bodies with the binding | `ts/callable_flow/extract.rs:3656-3663, 4088-4099, 2275-2422`. |
| The 274 extra zod sites are test-framework rows in test files | fresh dumps: 67 gained sites, all `src/**/tests/**`, tier `DirectPlusFramework`, 0 executed. The count differs from the report's by its site key; the composition is what was checked. |
| Twenty missed edges traced to a missing constraint, model or extraction | the package sources under `research/evaluation-harness/repos/js-analysis-bench/` and the walker paths above. DIAGNOSIS §3. |
| The typed plan's 15 wrong exact targets come from the sidecar join | `refined_calls/ts_types.rs:440-498, 319-326`, the fifteen rows of `baseline/report.md`, the underscore and axios sources. DIAGNOSIS §4. |
| Jelly static 84.5%, approx 85.4%, ACG 31.5% on this bench | run in the bench's docker sandbox, scored with the bench's matching rules reproduced; gitignored `research/package-graphs/repos/jelly-baseline/`. [BASELINES.md](BASELINES.md). |
| The held-out pins, licences, dependency counts | npm registry metadata for each `<name>/<version>`, 2026-11-17. [HELD-OUT.md](HELD-OUT.md). |
| The Jelly PLDI 2024 artifact is one OVA with nothing separately downloadable | Zenodo records 10796243 and 10930752; the OVA was not downloaded. |
| MIR coverage and the `push_unsupported` labels | `ts/mir/lower.rs` arms and labels, counted. [TIERS.md](TIERS.md) (c). |
| Budget defaults 10,000 / 64 / 512 | `points_to/solver.rs:20-28`, `solver/budget.rs:106-127`. |

## Not checked

- The walker was read at its entry points and the paths the traces use, not
  line by line through all 9,649 lines.
- The budget latch was confirmed with `inspect unknowns` on axios only; on
  zod and async it is inferred from zero resolved points-to edges on the
  same code path.
- The axios cost regression (105 to 112 s against a committed 2.1 s) was
  measured twice on a loaded host and not bisected. WP0 confirms or refutes
  it on a quiet machine.
- The composition argument is checked against the solver's code, not
  proven; the composition test stays the gate.
- The stitch and size targets are still estimates; WP5 measures them.
- The held-out drivers were not written and no held-out package was run.
- The walker's timings and the dumps here come from a test binary built with
  `--all-features`, as the bench's own `report.py` builds it.

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
| Jelly 0.13.0 at commit `62e60eb4fd5b991c5bf4cfc992f018984f560ea4`, https://github.com/cs-au-dk/jelly | The static and `--approx` baselines; the position format (`src/output/analysisstatereporter.ts`); the dynamic mode (`src/dynamic/dyn.ts`, `--dynamic`, `--npm-test`) | Run inside the bench sandbox; BSD-3-Clause. |
| `@persper/js-callgraph` 1.3.2, https://github.com/Persper/js-callgraph | The field-based (ACG) baseline | esprima 4.0.1 rejects ES2020; results are partial and labelled. |
| Zenodo records 10796243 (v1, 43 GB OVA) and 10930752 (v3, 48.3 GB OVA), concept DOI 10.5281/zenodo.10796242 | What the PLDI 2024 artifact offers | Not downloaded. |
| Zenodo record 12720724, `benchmarks.tar.gz`, CC-BY-4.0 | The ECOOP 2024 indirection-bounding benchmarks, the smaller licensed download from the same group | Contents beyond "all benchmarks" unconfirmed. |
| npm registry metadata, `https://registry.npmjs.org/<name>/<version>` | Held-out pins, integrity values, licences, dependency counts | Read 2026-11-17. |

Earlier research in this repository that this direction builds on:
`research/static-analysis-2.0/` (03, 06, 07 and OPEN-QUESTIONS),
`research/call-graphs/AGENT-EXTENSIBLE-CALL-GRAPHS.md`,
`research/call-graphs/JS-RECALL-BUCKETS.md`,
`research/type-alias-points-to/`, `research/effects-summaries/`.
