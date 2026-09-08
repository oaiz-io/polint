# 04 — Evaluation as a Weapon: Making polint the Most Rigorously Measured Engine

Date: 2026-09-01
Researcher: Claude Fable 5.1 (delegated via Hermes)
Revised 2026-09-02: time estimates removed, capability roadmap added.
Reads with: [01-capability-ladder.md](01-capability-ladder.md) section 7 (verification instruments), [03-build-plan.md](03-build-plan.md) (where each instrument lands).

## TL;DR

- polint already has better measurement *machinery* than most engines: pinned suite manifests, a matcher and metrics module with F0.5 to F3, an F1 regression gate with cost columns, a determinism gate, a golden corpus, a capability matrix, and a policy that public claims cite measured reports only. It has almost no measurement *enforcement*: the accuracy gate never runs in CI because oracle clones are never fetched, there is one data-flow fixture, and no taint corpus exists.
- The weapon is a four-instrument program, all reproducible from a fresh clone by one command: capability probes per ladder level, two real-application oracle lanes that are never blended, cost curves versus size, and soundness mutation tests. Each instrument has a CI tier (pull request, manual on-demand run, release). There are no scheduled jobs; the founder's standing rule is that heavy measurement runs only when someone asks for it.
- Ground truth for Go and TS/JS is available without inventing it: the Jelly PLDI 2024 dynamic call-graph artifact for Node projects, `golang.org/x/tools` call-graph references plus curated required edges for real Go repositories, SecBench.js executable exploits, and CVE-backed Go vulnerabilities with reachability ground truth. Curated required-finding sets on grafana, hugo and excalidraw close the gap for policy-style findings.
- False-positive and false-negative budgets are set per policy family, not per engine: review-time rules get a stricter false-positive budget (diff-time findings are acted on or ignored within minutes), whole-repository rules get a stricter recall budget. Google's Tricorder discipline of an effective false-positive rate under 10 percent is the reference point.
- Differential testing against CodeQL, Semgrep and Opengrep on public corpora, with adjudicated disagreements and both engines' outputs published, is the only way to produce numbers competitors cannot refute. Vendor-run comparisons on private corpora are exactly what polint should refuse to publish.
- Soundness spot checks are mutation-based: inject a bug that a claimed level must catch, apply semantics-preserving transformations that must not change the finding set, and run the engine with tiers toggled to check that precision never depends on an accident.
- Publication standard: pinned commits, public scripts, per-project breakdown, both oracle lanes, cost columns, confidence intervals from repeated runs, budget and timeout reporting, and a pre-registered analysis plan. Anything less is marketing.
- First three actions, all in Stage 0 of the build plan: a manual-trigger workflow that clones the oracles, prints accuracy and speed metrics, uploads them as artifacts, and fails on skip; a 60-case L4 probe seed with must-not-report twins; and a real SecBench.js scoring adapter.

## 1. What exists today, honestly

| Asset | State | Evidence |
|---|---|---|
| Suite manifests with pinned commits, tiers, seeds | present for Jelly micro, Go x/tools RTA, gosec samples, SecBench.js smoke, grafana, hugo, excalidraw, private devloupe | `research/evaluation-harness/suites/*.toml`, `BENCHMARK-SUITE.md` |
| Metrics, matcher, report normalization, F0.5 to F3, false-positive traps, forbidden assertions | present | `crates/polint-eval/src/harness/{metrics,matcher,report}.rs`; `AssertionMode::Forbidden` used by zero fixtures |
| F1 regression gate with runtime and peak RSS cost columns | present; runs only when `research/evaluation-harness/repos/` contains clones; CI never clones | `crates/polint-eval/src/harness/external/mod.rs:95-235`; `.github/workflows/ci.yml` |
| Committed accuracy baseline | Jelly micro recall 0.6646, precision 0.9742; Go x/tools recall 1.0, precision 0.0438 against a partial oracle | `research/evaluation-harness/baselines/persisted-graph-accuracy.json` |
| Scale corpus run | one result: excalidraw full pipeline OOM at 12 GB after about 1,026 s; hugo and grafana skipped after the failure | `scale-corpus-run.json` |
| Native fixtures | 53 cases, 381 expected rows, 223 of them invariants; one data-flow case; three synthetic-observed cases | `tests/eval-fixtures/` |
| Golden corpus and capability matrix | example self-pairs locked byte-for-byte; per view per language presence matrix | `tests/golden-corpus/`, `tests/capability-matrix/` |
| Determinism gate | N=10 seeded provider-order and row-order permutations, byte-identical JSON | `crates/polint-eval/src/harness/determinism_gate.rs` |
| gosec and SecBench.js adapters | enumeration stubs with placeholder labels; no scoring | `harness/external/gosec.rs`, `secbench_js.rs` |
| Public claim policy | measured reports only; no blended scores | `research/evaluation-harness/FINAL-REPORT.md` |

The July review's summary still holds after the August refactor: "no test anywhere asserts a precision, recall, or F1 value" in CI (`docs/architecture-review/08-evaluation-and-correctness.md`). The gate exists now; the clones do not.

## 2. Design principles

1. **Two oracle lanes, never blended.** Dynamic traces under-approximate (an unexercised edge is not a false positive); curated required sets under-approximate recall. Report `recall@dynamic`, `precision@required`, and `unclassified_extra` separately, as `research/static-analysis-2.0/OPEN-QUESTIONS.md` Q3 already decided. The Go x/tools "4.4 percent precision" row is the cautionary example: a partial `WANT` oracle scored as if closed.
2. **Cost columns on every accuracy row.** The Jelly iteration log dropped its runtime column at iteration 57 while runtime grew about 110 times (`docs/architecture-review/06`, a.4). Every accuracy number carries wall-clock, peak RSS and budget-exhaustion counts, or it is not a number.
3. **Level claims are probe claims.** A rung is certified by probes with must-not-report twins, not by the presence of a solver.
4. **Real applications are the headline; micro suites are regression nets.** Q38 froze Jelly micro as a net. Headline metrics come from the real-application lanes.
5. **Reproducible by a stranger.** `make bench` from a fresh clone, pinned commits, no private repositories in published tables (the devloupe monorepo stays local-only, as its manifest says).
6. **Default versus extended is a first-class dimension.** The product thesis is that agent-authored models raise recall; every model or extension lands with a measured delta and a held-out subset (Phase 51 machinery already exists).

## 3. The suite

### 3.1 Tier 1: capability probes (per level, per language)

| Level | Positive probes (must report) | Must-not-report twins | Initial size |
|---|---|---|---|
| L1 | banned call by literal name; argument shape | same name as a local identifier shadow (L2 twin) | 20 per language |
| L2 | forbidden import through alias and re-export; deprecated symbol via rename | same-named unrelated symbol in another package | 30 |
| L3 | nil after check on the other branch; use before init; missing cleanup on one exit; guard missing before sensitive call; `defer` and `finally` run on every exit | guard present on all paths; cleanup in every exit including panic and throw | 40 |
| L4 | taint through two helpers and a framework route; secret to logger via wrapper; dangerous API reachable from an unauthenticated root across packages; sanitizer kills on the crossed path | unrealizable path (enter from call site A, exit toward B); sanitizer on the crossed path; distinct sanitizer names so name matching cannot pass | 60 seed, 150 to 200 by Stage 1 |
| L5 | callback stored in a map and invoked later; middleware chain dispatch; taint in `req.body.name` but not `req.body.id`; class and prototype dispatch | two fields of one object with only one tainted; two callers of one helper where only one passes taint | 40 |
| L6 | defect only when two flags interact; double close on the error branch only | infeasible branch combination; guard-style sanitizer on the taken branch | 30 |

Probe rules: every probe is a real `#[polint::rule]` over public views so probes double as SDK contract tests; every probe carries a source-of-truth comment; each level's set is held out in part and rotated so the engine cannot be tuned to the probes (the Jelly lesson). Certification thresholds are in report 01 section 7.

### 3.2 Tier 2: real-application oracle lanes

| Lane | Corpus | Oracle type | Use | Bias to declare |
|---|---|---|---|---|
| JS/TS call graph, dynamic | 8 projects from the Jelly PLDI 2024 artifact (NodeProf traces; 141 projects, 36 with full dynamic graphs as cited in `research/static-analysis-2.0/01-benchmarking-and-measurement.md`), stratified by size and module system | dynamic traces | recall floor | test-coverage bias; treat extra edges as unclassified, not false |
| Go call graph, reference and curated | `golang.org/x/tools` RTA and VTA outputs as consistency references; curated required edges on `gin`, `hugo`, one `grafana` package set | tool reference plus human adjudication | recall against reference, precision against required | reference tools are themselves approximate; adjudication log published |
| TS/JS taint, executable | SecBench.js cases with runnable exploits; adapter scores real findings instead of placeholder labels | executable exploit | precision and recall for source-to-sink | npm-package bias; server-side only |
| Go taint, CVE-backed | Go modules with published vulnerabilities and known vulnerable call paths (the reachability question `govulncheck` answers), plus injected variants | CVE plus adjudicated path | recall of reachable flows; precision on unreachable twins | selection bias toward library CVEs |
| Policy findings on scale repos | grafana, hugo, excalidraw with 30 to 50 hand-adjudicated required findings per shipped template family | curated required set | precision at required; recall of required | adjudicator bias; publish the adjudication rubric |

### 3.3 Tier 3: cost curves

Reuse the existing `CurvePoint` telemetry (`crates/polint-eval/src/harness/bench/curve.rs`): peak RSS, cold and warm wall-clock, cache and store size, budget-exhaustion counts, keyed by repository size and diff size. Publish F1-versus-size and RSS-versus-size for both lanes. The regression budgets stay as locked (plus 20 percent RSS, plus 25 percent cold wall-clock) until warm reuse lands, then warm review must beat the baseline.

## 4. Budgets

| Rule family | False-positive budget | False-negative budget | Rationale |
|---|---|---|---|
| Review-time rules (`polint review`) | effective false-positive rate under 10 percent of surfaced findings on the policy lane | recall at least 70 percent on the taint corpus | diff-time findings are read by a human or an agent within minutes; noise kills adoption (Tricorder's usefulness gate, CACM 2018; Meta's diff-time fix-rate finding, CACM 2019) |
| Whole-repository security rules | under 20 percent | at least 80 percent on probes, at least 70 percent on real lanes | batch findings tolerate triage; missing a real flow is the costlier error |
| Architecture and convention rules (L2) | under 2 percent | at least 95 percent | these are near-exact; noise here is a bug |
| Heuristic-labeled results | reported separately; never counted in the exact budgets | | the honesty contract already labels them |

"Effective" false positive means a finding the adjudicator would not act on, including true-but-useless findings; that is the definition Google used and it is the one buyers use.

## 5. Differential testing against other engines

Protocol, designed so the result cannot be dismissed as vendor-run:

1. Corpora: only public repositories at pinned commits from the lanes above. CodeQL's CLI license permits analysis of open-source code, which is why the corpora must be open source; Semgrep Community Edition and Opengrep are usable freely.
2. Configuration: each engine's default security suite for Go and JS/TS, plus a polint template pack configured from the same source and sink lists; configurations published.
3. Normalization: SARIF from every engine mapped to `(rule family, file, line range, source, sink)` tuples; matching by family and overlapping range.
4. Adjudication: every disagreement (finding in one engine only) is adjudicated by two reviewers against the oracle lane; the adjudication table is published with the run.
5. Reporting: per project, per family, both lanes, cost columns, plus the raw SARIF of every engine so anyone can re-adjudicate.
6. Cadence: once per release, and once on demand when a competitor ships a relevant change.

The purpose is not a scoreboard. It is to locate polint's true gaps with an external instrument and to make the published placement (report 01, section 4) falsifiable.

## 6. Soundness spot checks

Three mutation families, all automated, run on demand (the same manual workflow) on the probe suite and a sample of the real lanes:

| Family | Method | What a failure means |
|---|---|---|
| Bug injection by level | inject a taint path, a nil dereference, a missing cleanup, or a callback dispatch that only level N machinery can see; the engine claiming level N must report it | a false level claim |
| Semantics-preserving transformations | rename identifiers, split a function into a helper, move a helper across files or packages, wrap a call in a callback or arrow, reorder independent statements, add a dead branch; the finding set must be unchanged | a recognizer-shaped dependency or a missing edge kind |
| Tier toggling | run with the typed tier, the heap tier, and sensitivity options toggled; precision must never depend on a lower tier being absent; unknown counts must move in the expected direction | a precision result that is an artifact of an accident |

Add the parser robustness check the July review asked for: mutate real files into malformed inputs; the frontend must produce diagnostics with spans and never panic. Fuzzing is cheap here and absent today.

## 7. Publishing numbers competitors cannot refute

Checklist for any public number:

- pinned corpus commits and polint version; `make bench` reproduces the table from a fresh clone;
- both oracle lanes reported separately, extra edges labeled unclassified;
- cost columns on the same table (wall-clock, peak RSS, budget events);
- per-project breakdown, not only aggregates; medians over at least three runs with ranges;
- budgets, timeouts and unknown counts stated, including what the engine refused to claim;
- pre-registered analysis plan committed before the run (which metrics, which thresholds, which corpora);
- adjudication logs for curated sets; raw outputs archived;
- no private repositories in published tables; the devloupe reference stays local.

This is the standard the SV-COMP community and the artifact-evaluation culture already hold verification tools to; no SAST vendor meets it, which is the point.

## 8. CI wiring

| Tier | Runs | Fails on |
|---|---|---|
| Pull request (under 10 minutes) | golden corpus, capability matrix, determinism N=10, leak gate, probe suite for L1 to L4, store boundary | any probe regression, any golden diff, any leak |
| Manual (on-demand) | clones oracles at pinned commits; F1 gate on both call-graph suites; taint corpus; mutation families; cost curves on hugo and excalidraw; prints an accuracy-and-speed table to the run summary and uploads per-case JSON, comparison rows and the environment manifest as downloadable artifacts | F1 drop beyond tolerance, budget breach, mutation failure, silent skip |
| Release | full lanes including grafana; differential runs; published report regenerated | any regression versus the last published report |

A skipped gate is a failed gate. The current early `return` when clones are absent (`harness/external/mod.rs`) becomes a loud failure in the manual tier and a loud "skipped" annotation in the pull-request tier.

## 9. Open questions

- Oracle licensing: Jelly artifact traces and SecBench.js are usable; the exact license terms of the Jelly artifact for redistribution of derived tables should be confirmed before publishing. **Open.**
- Adjudicator capacity: curated required sets need two reviewers per disagreement; the plan assumes the founder plus one agent-assisted pass with human sign-off. **Assumption.**
- Which CodeQL and Semgrep versions to pin for differential runs, and whether Semgrep Pro can be included under its terms; if not, Community Edition and Opengrep are the comparators. **Open.**
- Whether the Go taint lane should reuse `govulncheck`'s vulnerability database as its case source; it is the most credible reachability oracle for Go but biased toward library CVEs. **Open.**

## References

- `research/evaluation-harness/FINAL-REPORT.md`, `RECOMMENDED_IMPLEMENTATION.md`, `VALIDATION.md`, `suites/BENCHMARK-SUITE.md`, `baselines/README.md`
- `research/static-analysis-2.0/01-benchmarking-and-measurement.md`, `OPEN-QUESTIONS.md` (Q1 to Q5, Q38)
- `docs/architecture-review/08-evaluation-and-correctness.md`
- `crates/polint-eval/src/harness/` (external adapters, metrics, matcher, determinism gate, bench curves and gates)
- Sadowski et al., "Lessons from building static analysis tools at Google", CACM 2018; Sadowski et al., Tricorder, ICSE 2015
- Distefano et al., "Scaling static analyses at Facebook", CACM 2019
- Laursen et al., PLDI 2024 (Jelly dynamic oracles), https://dl.acm.org/doi/10.1145/3656424, artifact https://zenodo.org/records/10930752
- Pauck, Bodden, Wehrheim, "Do Android taint analysis tools keep their promises?", ESEC/FSE 2018
- Klinger, Christakis, Wüstholz, "Differentially testing soundness and precision of program analyzers", ISSTA 2019
- Sui et al., "On the recall of static call graph construction in practice", ICSE 2020
- Livshits et al., soundiness manifesto, CACM 2015
