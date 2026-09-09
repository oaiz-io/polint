# Validation and evidence inventory

## What this research validated

- Initial checkout/branch: `accuracy/astra-f1`, `d713dbd2e1c16ee5d89c3341db29fc585ee94c1b`; product tree initially clean.
- All 76 cases / 1,479 expected edges independently reconstructed from saved reports; TP/FP/FN exactly reproduce 249/8/1230 and 940/36/539. Recovered 691 TP, lost zero TP; removed 2 old FP and added 30 new FP.
- Debug/release/post-performance non-port reports have equal expected/observed edge sets; the two measured port reports also agree. This does not claim whole-report hashes are equal: runtime and metadata differ.
- All five copied report SHA-256s independently verified. Full per-case taxonomy and edge ledger sum to reported totals.
- Original sibling corpus's current source availability and source hashes inspected: 131 edge-referenced paths, 42 absent; 342 FN point to 34 distinct missing target files. All those edges are in helloworld. No historical dependency-tree snapshot proves report-time filesystem availability; the exact causal setup history remains unverified.
- Textual collector reuse reproduced: 8,261 matching lines of 8,520 pre-test lines (including one separator), 96.96%; archive extraction 8,376 lines. This is textual similarity, not semantic equivalence.
- Eight wrong-caller pairs inspected in source and evaluated with explicit score-only arithmetic; no proposed repair was executed.
- Independent skeptical review reconciled counts, qualified causal/ceiling claims, and verified the actual gate package and baseline-writer behavior.

No new gate execution, collector restoration, build, corpus dependency installation, performance benchmark, product edit, or PR occurred. Therefore the archived late identifier repair, constructor correction and all proposed gains remain **unverified implementation hypotheses**. The handoff gate helper was syntax-checked, not run end-to-end.

## Reproduce the research data

From this checkout:

```sh
python3 .context/accuracy-research/scratch/taxonomy.py
python3 .context/accuracy-research/scratch/size_collector.py
python3 .context/accuracy-research/scratch/caller_counterfactual.py
```

The taxonomy script reads pinned sources and saved reports from the original sibling worktree, captures inputs locally, and emits deterministic case/edge counts. The caller script also reads the original sibling report. If that worktree is removed, use the captured report identified by its SHA-256; do not silently substitute a later run. Scripts are research-only and write solely under `.context/accuracy-research/`.

## Unchanged gate procedure for the implementation crew

1. Verify both corpus commits and source/dependency inventory; preserve missing-dependency observations so newly available inputs cannot masquerade as a collector gain. Keep all 76 cases and 1,479 expected edges. Record code revision/diff, archived source hashes, compiler/build profile and baseline/suite/gate hashes. Use the existing baseline bytes and tolerance. Do not install dependencies as part of the paired recovery measurements.
2. **Unset** `POLINT_WRITE_GRAPH_BENCH`, `POLINT_CACHE_STORE`, and temporary-directory overrides. `POLINT_WRITE_GRAPH_BENCH=0` still enables writes because the test checks presence, not its value. The old research `run_gate.py` enabled that variable and restored baseline bytes afterward; do not reuse that behavior when the candidate may turn green.
3. Run the exact gate in each Rust build profile, always with suite tier `release`. The physically separate `polint-eval/src/harness` is included into `polint` under `cfg(test)`; the correct package is **`-p polint`**.

```sh
env -u POLINT_WRITE_GRAPH_BENCH -u POLINT_CACHE_STORE -u TMPDIR -u TMP -u TEMP \
  POLINT_REQUIRE_BENCH_CORPUS=1 POLINT_GRAPH_BENCH_TIER=release \
  cargo test -p polint --lib --all-features --locked \
  eval::external::tests::external_graph_baseline_reports_can_be_generated \
  -- --exact --nocapture

env -u POLINT_WRITE_GRAPH_BENCH -u POLINT_CACHE_STORE -u TMPDIR -u TMP -u TEMP \
  POLINT_REQUIRE_BENCH_CORPUS=1 POLINT_GRAPH_BENCH_TIER=release \
  cargo test -p polint --lib --all-features --locked --release \
  eval::external::tests::external_graph_baseline_reports_can_be_generated \
  -- --exact --nocapture
```

Alternatively, the research handoff helper below runs these commands, verifies corpus pins/protected hashes, requires one executed test and one fresh report directory, and copies reports from the retained `/tmp` directory into the allowed artifact tree. It does not enable or restore the baseline writer. The helper has only been syntax-checked during this research:

```sh
python3 .context/accuracy-research/scratch/run_unchanged_gate.py --release
python3 .context/accuracy-research/scratch/run_unchanged_gate.py
```

4. Require the log to show **one executed named gate**, no skipped corpus, and a complete fresh JSON report for each suite. With the writer unset, `external/mod.rs:78–82` retains a temporary output directory; never interpret a stale `.context/graph-benchmarks` report as this run's output. Preserve per-case JSON and the command/exit status even when red. Build failure or zero selected tests is not an accuracy result.
5. Independently compare expected/observed graph-edge sets against the captured starting state. Record TP gained/lost, FP added/removed, FN, precision, recall, F1 and unknowns. Verify Model/Heuristic labeling on new production edges; the oracle's `AssertionMode::Exact` is an exact edge-identity comparison, not a production precision claim. Check both call2fun and fun2fun. Do not use the report's zero-valued failure-category fields as diagnosis.
6. Pass every existing gate assertion, including time/RSS cost-column checks after F1. Gate Go without changing it. Note: the current checked-in Go row has P=0.04378698/R=1 (F1≈0.083900), while the supplied observed Go score is 0.095116; both the prior report and the mission agree it is not a regression. Use the actual committed bytes, not a restated rounded Go baseline.
7. Validate the affected cold/warm cache path and deterministic outputs across one and multiple threads. A fresh unchanged source run after an algorithm bump must not reuse incompatible old facts. Then run required formatting, lint, and focused MIR/calls/graph/solver/refined-call/TS adapter tests. Broaden to workspace checks as required by the implementation workflow, keeping normal temp directories so temporary rule-host Cargo projects do not inherit this workspace accidentally. Do not update snapshots or fixtures merely to hide new mismatches.
8. Compare protected file hashes afterward; preserve the reports and implementation diff. A final accepted implementation needs both profiles green and no product-contract regression. If red, report the remaining named edges and actual score. Never edit tolerance/baseline, change corpus tiers, suppress expected edges, or bypass identity/refined validation to cross the bar.

## Primary source inventory

All accessed 2026-09-08. No external ecosystem recommendation or new algorithm claim required a web/paper search: primary evidence here is the exact repository history, saved execution reports and pinned fixture implementation.

| Source | Exact identity | Key paths / purpose |
| --- | --- | --- |
| Current polint | `d713dbd2e1c16ee5d89c3341db29fc585ee94c1b` | `ts/token_flow.rs`; `ts/semantic_graph_build.rs`; `ts/points_to.rs`; `analysis_neutral/refined_calls/provider.rs`; cache recipes |
| Deletion | `3e25db23fd28dfe843c989f0b0e58486634d141c` | Collector and separate heap removal |
| Pre-deletion parent | `7730a730dc6e912d9a11ac41b40259ea65c8cf37` | `crates/polint/src/analysis/calls/ts_value_flows.rs`, 67 historical tests |
| Rejected archive | `.context/perf-research/rejected-accuracy-restoration/` | `tracked.patch`, `full-working-tree.patch`, private collector and saved integration files; unverified late repair explicitly retained |
| Prior research | `.context/perf-research/accuracy-restoration-RESEARCH.md`, `FINAL-REPORT.md` | Measurement boundary and disposition, read first |
| Jelly | `b799ed4f0d68c670fe398830aaa51dd5c628cf74`; https://github.com/cs-au-dk/jelly; BSD-3-Clause | `tests/{micro,approx,helloworld,mochatest}` pinned sources and oracle spans |
| Go corpus (gate only) | `7743a285e3d261ca235408e013ec5c14cb5170e4`; https://github.com/golang/tools; BSD-3-Clause | Existing acceptance corpus; no Go analysis work |
| Accuracy gate | `crates/polint-eval/src/harness/external/mod.rs` at HEAD | Fixed tolerance, full suite tier, both suites, optional baseline writer, cost checks |
| Expected/observed normalization | `crates/polint-eval/src/harness/external/jelly_callgraph.rs`, `observed.rs` | File selection, edge/span identity, caller/target projection |

Raw provenance: `.context/accuracy-research/taxonomy/inputs.json` (five reports and SHA-256s), `corpus-sources.json` (all source hashes/absence), `validation.json` (reconciliation), `cases.csv`/`cases.json`/`edges.json` (full classified ledger), `collector-reuse.json` (historical/archive hashes), `caller-counterfactual.json` (eight exact pairs). `PROTOCOL.md` records inclusion/exclusion decisions. Source comments' historical project identifiers are cited as encountered, not copied into proposed shipped behavior.

Research-specific `AGENTS.md` permits direct research edits and parallel review. The user's narrower artifact paths take precedence over the generic research template's index/roadmap updates: only this research directory and `.context/accuracy-research/` were written. No root index, roadmap, or planning files were changed.
