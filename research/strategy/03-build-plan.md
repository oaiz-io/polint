# 03 — Build Plan: Six Stages From Partial L4 to Certified L4 With Selective L5 and L6

Date: 2026-09-01
Researcher: Claude Fable 5.1 (delegated via Hermes)
Revised 2026-09-02: time estimates removed, capability roadmap added.
Inputs: [01-capability-ladder.md](01-capability-ladder.md) (levels and axes), [02-gap-analysis.md](02-gap-analysis.md) (prioritized gaps). Measurement contract: [04-evaluation-as-a-weapon.md](04-evaluation-as-a-weapon.md).

## TL;DR

- Three plans currently describe polint's future and disagree: the 22-PR research backlog (`research/ROADMAP.md`), the active v2.0 milestone (phases 63 to 71, `.planning/ROADMAP.md`), and the architecture-review milestones M0 to M5 (`docs/architecture-review/PLAN.md`, largely landed in August). This report folds them into one sequence of six dependency-ordered stages, Stage 0 to Stage 5; there is no calendar, only prerequisites.
- The research backlog is not "PRs 1 to 12 unimplemented". Every PR from 1 to 22 shipped in some form through v1.2 phases 20 to 41 and the August refactor; what remains is a residue per PR (section 2). The plan schedules the residue, not the PR titles.
- Order of work: instrument first (Stage 0), certify L4 for Go and TS/JS (Stage 1), persist summaries and make warm review real (Stage 2), fix the scale envelope and ship the graph surface (Stage 3), buy L5 selectively (Stage 4), buy L6 selectively and publish head-to-head results (Stage 5).
- The v2.0 phases survive but move: Phase 65 closes in Stage 0 in its restart form, Phase 66 lands in Stage 1, Phase 67 is the Stage 2 keystone, Phase 68 rides with it, Phase 69 and 71 land in Stage 3, Phase 70 stays the designated cut.
- Every stage has an exit criterion stated as a ladder placement proven by probes, oracle lanes and cost curves. A stage that does not move a placement or an axis grade is plumbing and is folded into one that does, which is the milestone rule already written in `.planning/REQUIREMENTS.md`.
- The four hardest technical risks are fixpoint termination under tabulation with widening, summary precision collapse under k-limiting and budgets, cache invalidation correctness once summaries persist, and cross-language fact identity for the TS-to-Go boundary. Each gets a named mitigation and a kill criterion (section 5).
- Delivery rules are inherited from the Phase 65 forensic: at most 1,500 changed lines and 25 files per PR, one storage invariant or one prerequisite per PR, no dual code paths, no expectation edits to pass a test, measure before optimizing.
- Explicitly not built: new languages, a query language, ML detection, a remote summary registry, GPU or distributed solving, a daemon as a requirement.
- Two decisions are needed from the founder before Stage 1 starts: whether the TS type sidecar may depend on Node and the TypeScript 6 API, and whether the thin-SDK rule build (0.3.0) is a Stage 3 track or a separate product release.

## 1. Starting position (verified 2026-09-01)

| Dimension | State | Source |
|---|---|---|
| Engine size | about 244k lines in `crates/polint/src`; `analysis_neutral` 88k, `analysis_kernel` 36k, `ts` 31k, `go` 21k | line counts in this session |
| Level machinery | L3 complete; L4 partial (RTA, Andersen, four summary domains, matched taint search); L5 seeds; L6 absent | report 01 section 5 |
| Accuracy baselines | Jelly micro recall 66.5 percent, precision 97.4 percent; Go x/tools recall 100 percent against a partial oracle | `research/evaluation-harness/baselines/persisted-graph-accuracy.json` |
| Scale | excalidraw (86,527 LOC) full pipeline killed at 12 GB after about 1,026 s; capability-gated runs 2 to 3 s on private benchmark repos | `scale-corpus-run.json`; PR #104 description |
| Store | schema v5, two provider mirrors, maintenance-only | `analysis_kernel/store/migrations.rs` |
| Rule host | cold build 187 s, 225 units, 582.7 MB; machine-global host sharing since v0.3.2 | `build-cost.json`; PR #103 |
| CI gates | fmt, clippy, docs, deny, MSRV, feature matrix, tests on three platforms, polyglot canary, leak gate, determinism N=10, store boundary; no accuracy gate | `.github/workflows/ci.yml` |
| Tests | 2,071 unit, 240 integration, 342 in `polint-eval`; 53 native fixture cases, one data-flow case | counts in this session |

## 2. Reconciling the three plans

### 2.1 Research backlog PR 1 to 22: shipped form and residue

The one-to-one mapping is PR n to v1.2 Phase n plus 19 (`.planning/milestones/v1.3-ROADMAP.md`, archived phase table).

| PR | Shipped as | Residue that this plan schedules |
|---|---|---|
| 1 kernel facade | Phase 20; provider manifests executed by the kernel after the August scheduler work | none |
| 2 provenance metadata | Phase 21; interned `FactMeta` | none |
| 3 evaluation harness MVP | Phase 22; `polint-eval` crate | gates run only when oracle clones exist (Stage 0) |
| 4 input snapshots, cache keys | Phase 23 | `InputSnapshot` judged not ready for persistence (`research/local-semantic-store/IDENTITY-READINESS.md`); handled per provider in Phase 65 restart |
| 5 persistent layer cache | Phase 24 (layer cache); SQLite facts not persisted | Phases 66 and 67 (Stage 1, Stage 2) |
| 6 rule manifest, inspect, test | Phase 25 | none |
| 7 semantic index deepening | Phase 26 | TS type-level resolution (Stage 1 sidecar) |
| 8 module topology | Phase 27 | none |
| 9 semantic MIR | Phase 28, rebuilt as a real IR in August | branch predicates unbound (Stage 0) |
| 10 CFG and control dependence | Phase 29, rebuilt on terminators in August | probe `defer` and `finally` semantics (Stage 0) |
| 11 direct call facts | Phase 30 | none |
| 12 P0 abstract domains | Phase 31: six domains | intervals, typestate, interprocedural lifting (Stage 4) |
| 13 summary kernel | Phase 32: four domains, SCC closure | access-path TITO, effects view (Stage 1, Stage 4) |
| 14 demand queries, SCC cache | Phase 33: trace-only engine; closure processes every SCC | real demand queries over persisted summaries (Stage 2, Stage 3) |
| 15 extension sink | Phase 34: host side and protocol | author-side surface and CLI (Stage 4) |
| 16 entrypoints and trust boundaries | Phase 35: hard-coded recognizers | models as data (Stage 1), agent-authored models (Stage 4) |
| 17 type, value, alias substrate | Phase 36 | TS types (Stage 1), VTA-grade Go narrowing (Stage 4) |
| 18 refined call providers | Phase 37; Go RTA, TS Andersen | typed tier (Stage 1), selective sensitivity (Stage 4) |
| 19 data flow | Phase 38; matched IFDS search in August | tabulation, access paths (Stage 1) |
| 20 slicing and evidence | Phase 39; evidence shipped to users in August; slicing orphaned | slices for agents (Stage 5) or deletion |
| 21 benchmark adapters, promotion gates | Phase 40 | CI enforcement, taint corpus, differential runs (Stage 0, Stage 5) |
| 22 public query views | Phase 41 and v1.4 policy views | `Effects` and evidence views after L4 certification (Stage 4) |

### 2.2 v2.0 phases 63 to 71

| Phase | Status | Placement in this plan |
|---|---|---|
| 63 ground truth and baselines | complete, but baselines are fixture-sized and the graph gate is not in CI | Stage 0 completes it: on-demand clones, real scale numbers |
| 64 store foundation | complete | none |
| 65 generation manifest and mirroring | restart R1 to R4 accepted; first R5 increment (Go syntax) landed; TS syntax mirror and R6 open | Stage 0 closes R5 and R6 under the restart budgets |
| 66 validated fact and graph ingest | pending | Stage 1, restricted to the families L4 certification needs |
| 67 summary persistence, frontier, warm review | pending; the keystone | Stage 2 |
| 68 internal query engine | pending | Stage 2 with 67 |
| 69 public graph CLI | pending | Stage 3 |
| 70 lexical search | pending; designated cut | Stage 3 only if Stage 2 leaves capacity; otherwise v2.1 |
| 71 recovery, pruning, scale gates | pending | Stage 3 |

### 2.3 Architecture-review milestones M0 to M5

M0 to M4 landed on the integration branch in August and merged as PR #96. M5 residue: crate split (deferred, layering test in place), persistent store (Phase 67), demand queries (Phase 68), shareable rule packs, Python (excluded), external-index frontends (excluded as breadth), framework models as data (Stage 1 and Stage 4). The plan adopts M5's dependency order and drops its language items.

## 3. Delivery rules

Inherited from `.planning/forensics/report-20260719-phase-65-scope-collapse.md` and `docs/architecture-review/HANDOFF.md`; restated because they are the reason the plan has small phases.

1. A PR changes structure or behavior, never both; at most 1,500 changed lines and 25 files; one storage invariant or one prerequisite per PR.
2. No dual paths: the old code path is deleted in the same PR that lands the new one.
3. No expectation edits to make a test pass; golden or baseline updates ship separately with a stated behavior change.
4. Measure first: every accuracy change records runtime and peak RSS; every performance change records the accuracy gate result.
5. One-way doors (on-disk schemas, public SDK types, wire protocols) get a written decision before landing.
6. No new languages, no DSL, no ML detection, no remote registry; these are recorded exclusions, not deferrals.
7. Every stage names the ladder rung or axis it moves and the probe that proves it.

## 4. The stages

Stages are ordered by dependency, not by calendar: a stage starts when the earlier stages it depends on have met their exit criteria. Track letters: A accuracy, B persistence and latency, C evaluation, D authoring and product, E scale.

### Stage 0 — Instrument and close the store slice

Goal: make every later claim falsifiable and remove the two blockers that hide the engine (scale OOM, silent accuracy).

| Item | Track | Source | Detail |
|---|---|---|---|
| On-demand oracle gate | C | PR 21 residue; `docs/architecture-review/08` | a manual-trigger GitHub Actions workflow (no schedule) clones Jelly and `golang/tools` at pinned commits and runs the existing F1 gate (`crates/polint-eval/src/harness/external/mod.rs`); it prints accuracy and speed metrics to the run summary, uploads them as downloadable artifacts, and a skip is a failure, not a silent return |
| Capability probe suite v1 | C | report 04 | L1 to L3 probes for Go and TS/JS plus a 60-case L4 seed; each positive has a must-not-report twin |
| Taint corpus v0 | C | report 04 | make the SecBench.js adapter score real findings; add 40 curated Go and TS cases with CVE or exploit backing |
| Scale root cause | E | `scale-corpus-run.json` | profile the excalidraw full pipeline; add a runtime memory ceiling and a wall-clock budget that degrade with reported reasons; parallelize the per-file stages (symbol graph, MIR lowering, CFG, metrics) with the existing sort-by-path determinism pattern |
| Completeness accessor | A | report 02 item 8 | `RuleCtx` gains a way to ask whether the run was complete for the requested capabilities and whether any query hit a budget, so "no findings" can be labeled |
| Bind branch predicates | A | report 02 item 10 | both lowerers set `predicate_place_key` from the condition expression; no consumer change yet |
| Phase 65 close | B | `.planning/phases/65-*` | TS syntax mirror as the second R5 increment; R6 private enablement with one measured cold-warm pair; STORE-04, STORE-05, META-01, META-04 recorded |

Exit criteria:
- L3 certified: at least 95 percent of L3 probes pass, 100 percent of must-not-report twins hold, for both languages.
- The on-demand gate fails on a 2-point F1 drop on either oracle suite; the committed baselines and the run artifacts carry runtime and peak RSS for every case.
- excalidraw full pipeline completes under 6 GB and 300 s on the reference host, with any degradation listed as budget facts.
- Phase 65 requirements marked complete under the restart budgets.

### Stage 1 — L4 certification, part one: resolution and decision

Goal: the five-item L4 set from report 02, minus persistence.

| Item | Track | Source | Detail |
|---|---|---|---|
| TS type sidecar | A | ACC-FUT-01; Q20, Q21, Q22 | Node process on the TypeScript 6 compiler API emitting per-call-site receiver type, resolved signature and member resolution in batched per-project dumps; new provider feeding the calls provider as an XTA-grade tier before points-to; `any` density gates per Q22; tier label on every edge. TypeScript 7 has no stable programmatic API until 7.1 (see report 02), so the Node path is the only option until 7.1 ships |
| IFDS tabulation | A | report 02 item 3 | summary edges per callee computed once and reused across queries; path enumeration retained only to reconstruct a witness for `evidence_v1`; `FlowQuery` defaults raised because depth no longer multiplies cost |
| Access-path summaries | A | report 02 item 4 | `DataFlowTito` carries k-limited access paths (k=2 default, k=3 opt-in) using the existing `AccessPathProjection` vocabulary; top reason when the limit is hit |
| Models as data v1 | A, D | PR 16 residue; W5.7 | promote the private adaptation TOML to a documented repo-local artifact with source, sink, sanitizer, propagator and entrypoint rows; validation against the semantic graph; default-versus-extended delta in the benchmark report |
| Phase 66 ingest, restricted | B | v2.0 | persist files, symbols, references, functions, calls and unknown regions as validated rows; nothing eager for data flow |
| Taint corpus v1 | C | report 04 | 150 to 200 labeled cases across the ten shipped templates with distinct sanitizer names and explicit false-positive traps |

Exit criteria:
- L4 probes: at least 90 percent positives, 100 percent must-not-report twins, both languages.
- Taint corpus v1: precision at least 90 percent, recall at least 70 percent, no unrealizable path in any reported witness.
- Real-app TS call-graph recall on the Jelly real-application lane improves by a recorded margin with the typed tier on versus off; precision does not drop below the committed floor.
- Every taint finding names its tier (typed, field, heap) and sanitizer evidence.

### Stage 2 — Keystone: summaries persist, warm review is real

Goal: v2.0 Phase 67 and 68 as written, on top of the Stage 1 summaries.

| Item | Track | Source |
|---|---|---|
| Summary manifests and content-addressed payloads | B | SUM-01, SUM-02, SUM-03 |
| Invalidation frontier with must-recompute and must-reuse fixtures | B | SUM-04, SUM-05, REV-01, VAL-04 |
| Dependency package summaries keyed by package and version | B, E | PERF-04; `research/static-analysis-2.0/03-summary-store.md` tier 1 |
| Warm review parity and latency targets | B | REV-02, REV-03, PROD-02 |
| Internal query engine and envelope | B | QUERY-01 to QUERY-08 |
| Runtime envelope enforced with reported degradation | E | report 02 item 7 |

Exit criteria:
- Warm `polint review` on the frontier benchmark recomputes only changed functions and SCCs plus transitive dependents; recompute set asserted by fixture.
- Warm output byte-identical to cold; determinism gate extended with cold, warm, restored-store and process-restart runs.
- Peak RSS on `gohugoio/hugo` and `excalidraw/excalidraw` full pipelines proportional to the analyzed working set: a run that touches 10 percent of functions uses under 40 percent of cold peak.
- Dependency bodies are never re-parsed once summarized (PERF-04 fixture).

### Stage 3 — Scale envelope, graph surface, rule-host cost

| Item | Track | Source |
|---|---|---|
| Parallel per-SCC summary closure and parallel solver policies where deterministic merge is proven | E | `docs/architecture-review/06` b.9 |
| `grafana/grafana` full pipeline inside the envelope | E | Phase 71, VAL-07 |
| Recovery, pruning, WAL policy, scale gates | B | Phase 71 |
| Public `polint graph` commands with agent-shaped JSON and recall context | D | Phase 69, CLI-01 to CLI-07 |
| Thin-SDK prebuilt rule host (0.3.0 code-preserving build) | D | `research/code-preserving-rule-build/IMPLEMENTATION-PLAN.md` |
| Lexical search | D | Phase 70, only if Stage 2 leaves capacity |

Exit criteria:
- `grafana/grafana` (1.55 million LOC) full pipeline completes on a 16 GB host inside the envelope, with curves published.
- Cold `polint check` with one rule on a small repository under 20 s, zero Cargo spawns on unchanged rules (budgets from `research/code-preserving-rule-build/FINAL-REPORT.md` section 9.3).
- `polint graph` commands pass the leak, determinism and correctness gates; unknown counts render by default.

### Stage 4 — Selective L5 and interprocedural domains

| Item | Track | Source |
|---|---|---|
| Selective object sensitivity for TS points-to: top 5 percent of functions by function-valued fan-in, at least three callers, 15 percent heap growth cap | A | Q25; Zipper OOPSLA 2018 |
| Indirection-bounded propagation as the primary scale knob, token cap as a fuse | A, E | Q8; ECOOP 2024 |
| VTA-grade narrowing for Go function values and interface dispatch, measured on real Go repos | A | Q2 |
| IDE lifting of constants and nilness across calls on the tabulation engine | A | PR 12 residue |
| Typestate and resource domain; dominance-based guard and per-exit cleanup proofs | A | PR 12 residue; `docs/facts/control-flow.md` deferrals |
| Agent-authored models with default-versus-extended measurement; extension author-side surface and `polint extension` commands | D | PR 15 and 16 residue |
| `Effects` and evidence SDK views after gates | D | PR 22 residue |

Exit criteria:
- L5 probes: at least 80 percent positives, 100 percent must-not-report twins for TS/JS; Go dispatch probes at least 90 percent.
- Real-app false-positive budget per policy family met (report 04); precision floor unchanged with sensitivity on.
- A documented agent workflow produces a model pack for one framework not in the built-in recognizers and the delta is measured.

### Stage 5 — Selective L6 and publication

| Item | Track | Source |
|---|---|---|
| Path feasibility over nullness, constants and intervals using bound predicates; guard-style sanitizers | A | report 02 section 3.5 |
| Under-approximate review mode: report only feasibility-checked witnesses for review rules | A | Q34; Incorrectness Logic POPL 2020 |
| Differential testing against CodeQL, Semgrep and Opengrep on the public corpora; adjudicated disagreements; published tables | C | report 04 |
| Soundness mutation suite gated in CI | C | report 04 |
| Cross-language boundary contract spike: TS client to Go handler over an HTTP route model | A | `docs/architecture-review/PLAN.md` frontier |

Exit criteria:
- L6 probes: infeasible-branch twins never reported; feasible-flag positives found at 80 percent or more.
- Head-to-head report published with pinned commits, scripts, per-project breakdown and both oracle lanes.
- Boundary spike yields a written contract model and a go or no-go decision, not a feature.

## 5. Hardest technical risks

| Risk | Where it bites | Mitigation | Kill criterion |
|---|---|---|---|
| Fixpoint termination and cost under tabulation | IFDS summary edges over recursive SCCs; IDE lifting with lattices that are tall (strings, constants) | finite-height domains only in the IDE path; widening fuel already in `domains/lattice.rs`; per-SCC iteration caps that latch `BudgetExceeded`; the existing determinism gate extended to solver step counts | any probe run exceeding 10 times the enumeration baseline in wall-clock without a precision gain |
| Summary precision collapse | k-limited access paths and budgets turn summaries to top; taint smears across whole parameters | k=2 default with measured top rate per family; `SummaryTopReason` surfaced in unknown taxonomy; precision floors on the taint corpus gate every summary change; selective sensitivity only where fan-in justifies it | top rate above 20 percent of summaries on the real-app corpus, or precision below floor |
| Cache invalidation correctness with persisted summaries | stale reuse after edits to callees, config, models, toolchains | Merkle-shaped summary keys (Q12); from-scratch parity and recompute-and-diff gates (SUM-05); stale-reuse mutation fixtures per input class (VAL-04); complete-generation commit discipline already landed in Phase 65 restart | any parity failure blocks default reuse; reuse stays private until the mutation matrix is green |
| Cross-language fact identity | TS-to-Go boundary contract, and any future frontend | stable-key recipes per family already interned; language-neutral IR; boundary facts as explicit contract rows with provenance rather than synthetic call edges; SCIP-style monikers considered for export only | if a boundary edge cannot carry precision and unknown reasons, it is not emitted |
| TS type sidecar dependency | Node and TypeScript 6 API until 7.1 exposes a stable API | batched dumps keyed by content and compiler options; sidecar version in cache keys; `any` density gates; the heap tier remains a full fallback | if sidecar cost exceeds 2 times parse time on the real-app lane, the tier becomes opt-in |
| Scope collapse (Phase 65 pattern) | any stage | delivery rules in section 3; each stage has at most six items; review findings get a disposition, not automatic implementation | a PR over budget is split before it continues |
| Benchmark overfitting (Jelly pattern) | probes and micro suites | probes are held out per level with a rotation; real-app lanes are the headline; micro suites are regression nets only (Q38) | micro F1 rising while the real-app lane is flat for two phases |
| Memory regressions from persistence | ingest and payload layout | bounded sorted batches (PERF-02); payload layout locked by benchmark (SUM-03); the plus 20 percent RSS gate | gate red blocks the phase |

## 6. Stage view by track

| Stage | Track A accuracy | Track B persistence and latency | Track C evaluation | Track D authoring and product | Track E scale |
|---|---|---|---|---|---|
| Stage 0 | completeness accessor; predicates bound | Phase 65 close | on-demand oracle gate; probes v1; taint v0 | | excalidraw root cause; envelope; per-file parallelism |
| Stage 1 | TS type tier; tabulation; access paths | Phase 66 restricted ingest | taint v1; L4 probes | models as data v1 | |
| Stage 2 | | Phase 67 keystone; Phase 68 | frontier and parity fixtures | | envelope enforced; hugo and excalidraw inside envelope |
| Stage 3 | | Phase 71 gates | scale curves published | `polint graph`; thin-SDK build | grafana inside envelope; parallel SCC closure |
| Stage 4 | selective object sensitivity; VTA; IDE lifting; typestate | | L5 probes; FP budgets | agent-authored models; extension CLI; `Effects` view | |
| Stage 5 | feasibility; under-approximate mode; boundary spike | | differential publication; mutation suite in CI | | |

## 7. Explicitly not built

- Additional language frontends and external-index frontends; the founder's constraint is depth.
- A query language, graph shell, or public raw graph SDK views (`.planning/REQUIREMENTS.md` out-of-scope table).
- ML detection in the core; ML remains propose-then-verify at the edges and is not scheduled before Stage 4.
- A remote package-summary registry; local registry-ready seams only (SUM-07).
- GPU or distributed solving; a daemon as a requirement; vector search before lockfiles exist.

## 8. Decisions needed from the founder

1. Approve the TS type sidecar's dependency on Node and the TypeScript 6 compiler API until TypeScript 7.1 exposes a stable one, with the heap tier as fallback.
2. Decide whether the thin-SDK 0.3.0 build ships as a Stage 3 track inside this plan or as its own release train; it is the largest authoring-moat item and the only one that changes the rule-pack manifest contract.
3. Confirm Phase 70 as the designated cut and that `polint graph` (Phase 69) is not a CI gate.
4. Confirm that the cross-language boundary work in Stage 5 is a spike with a written go or no-go, not a committed feature.

## References

- `research/ROADMAP.md` (PR backlog and product thesis); `.planning/ROADMAP.md` and `.planning/REQUIREMENTS.md` (v2.0); `.planning/milestones/v1.3-ROADMAP.md` (phase table); `docs/architecture-review/PLAN.md`, `HANDOFF.md`, `M5-BEFORE-MERGE.md`; `.swarm/DECISION-2026-08-10-PRE-SHIP.md`, `DEFERRED-AFTER-SHIP.md`
- `.planning/forensics/report-20260719-phase-65-scope-collapse.md`; `research/local-semantic-store/RESTART-PLAN.md`, `IDENTITY-READINESS.md`
- `research/static-analysis-2.0/OPEN-QUESTIONS.md` (Q2, Q8, Q12, Q20 to Q22, Q25, Q34, Q38)
- `research/code-preserving-rule-build/FINAL-REPORT.md`, `IMPLEMENTATION-PLAN.md`
- Reps, Horwitz, Sagiv, POPL 1995; Sagiv, Reps, Horwitz, "Precise interprocedural dataflow analysis with applications to constant propagation" (IDE), TCS 1996; Li et al., Zipper, OOPSLA 2018; Chakraborty et al., ECOOP 2024; O'Hearn, POPL 2020; Salsa algorithm reference, https://salsa-rs.github.io/salsa/reference/algorithm.html
