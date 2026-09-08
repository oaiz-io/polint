# 02 — Gap Analysis: polint Versus the Strongest Engine at Each Rung

Date: 2026-09-01
Researcher: Claude Fable 5.1 (delegated via Hermes)
Revised 2026-09-02: time estimates removed, capability roadmap added.
Reads with: [01-capability-ladder.md](01-capability-ladder.md) (levels L0 to L7, axes A to G). Sequenced by: [03-build-plan.md](03-build-plan.md).

## TL;DR

- The July 2026 architecture review (`docs/architecture-review/`) was accurate for its commit and is now partly stale: the August refactor deleted the recognizer bank, made the IR real, replaced unmatched BFS with matched call and return taint search, shipped evidence across the rule-host boundary, interned identities, and wired an F1 gate. Section 2 lists what still holds so the gap list below is current, not inherited.
- At **L2** polint matches or exceeds incumbents for Go and TS/JS name resolution, with explicit unresolved states that CodeQL and Semgrep do not expose to rule authors.
- At **L3** polint is complete on machinery but thin on domains: no intervals, no typestate, no interprocedural lifting; guard and cleanup policies prove same-function order, not dominance.
- At **L4** the gaps versus CodeQL, Semgrep Pro and Infer are concrete and finite: no type-directed call-graph tier for TypeScript, path enumeration instead of tabulation (depth 8 defaults), whole-parameter summaries instead of access paths, about fifteen hard-coded framework recognizers instead of models-as-data, and no taint benchmark. These five items are the entire distance to "L4 certified".
- At **L5** polint has the right solver (field-sensitive Andersen) and lacks what makes it pay on real JavaScript: object sensitivity where fan-in is high, indirection bounding as the scale knob, and dependency summaries so `node_modules` is never solved.
- At **L6** nothing exists and nothing should be built until branch predicates are bound in the IR; the prerequisite is cheap and unblocks both path feasibility and better L3 domains.
- On the axes, polint leads on authoring, honesty and evidence and trails badly on scale, latency, framework modeling and measurement. The scale gap is the one that makes the level gaps invisible to users: deep tiers cannot run on a medium repository.
- The moat is real and specific: rules as compiler-verified typed Rust, capability derivation from signatures, per-finding honesty and replayable evidence, local-first execution with no cloud, and an agent authoring loop. No incumbent can copy the first two without abandoning its query language, and the licensing turbulence around Semgrep and CodeQL's non-open-source restrictions make local-first typed rules a buying criterion, not a nicety.
- Hygiene polint must match to be taken seriously: cross-file taint that composes through dependencies, models as data, a sound-modulo-declared-unknowns summary contract, measured precision on public corpora, and a memory envelope.
- The prioritized gap list (section 7) has twelve items; the first five are the L4 certification set and are the build plan's first two stages.

## 1. Method

Every internal claim was checked against the tree at `9b6ac59d` (v0.3.3, 2026-09-01). External claims come from official documentation, papers already indexed in `research/`, and three web checks made in this session (TypeScript 7 API status, the Opengrep fork, GitHub Code Security pricing). Everything else external is labeled as prior knowledge or assumption.

## 2. What the August refactor changed since the July review

The July review is the most-cited critical document in the repository, so the current state is stated explicitly.

| July 2026 finding (`docs/architecture-review/`) | September 2026 state | Evidence |
|---|---|---|
| MIR "is not an IR": no blocks, terminators, operators | Real IR: `MirBlock`, terminators `Goto`, `Branch`, `Switch`, `Return`, `Throw`, `Call { unwind }`, `Suspend`, `Unreachable`; values `BinOp`, `Aggregate`, `Closure` | `crates/polint/src/ir/body.rs:51-93`, `ir/op.rs:70-90` |
| CFG recovers loops by substring matching source text | One language-neutral `lower_cfg` over MIR terminators; no `contains_token` remains | `analysis_neutral/cfg/lower.rs` (no matches for `contains_token`); `.swarm/W3.4-LAND.md` |
| 11,898-line TS recognizer bank (`ts_value_flows`) and a parallel `js_points_to` Oxc pipeline | Deleted; Andersen points-to is the sole TS/JS indirect-call resolver | `.swarm/W3.5-LAND.md`, `.swarm/W4.1-LAND.md` |
| Interprocedural dataflow is BFS with no call stack; unrealizable paths reportable | Matched call and return search with a call stack in the exploded state; sanitizer kills as transfer functions | `analysis_neutral/ifds/mod.rs:365-390` |
| Reachability filter masks a 53 percent precision resolver as 96 percent | Filter deleted; evaluation scores the complete edge set; committed Jelly baseline is now recall 66.5 percent at 97.4 percent precision | `.swarm/W4.4-LAND.md`; `research/evaluation-harness/baselines/persisted-graph-accuracy.json` |
| Evidence stripped at the rule-host boundary | `evidence_v1` validated and kept; invalid envelopes become internal diagnostics | `crates/polint/src/diagnostics/mod.rs:237-262` |
| Zero interner, 229 `stable_key: String` fields | `StableKeyInterner` scoped to the host; retained fact rows carry `StableKeyId` | `crates/polint/src/internal_core/stable_key.rs`; `.swarm/READY-TO-SHIP.md` correction |
| `analysis/slicing` orphaned; `demand` unreferenced | `slicing` still has no callers outside itself; `demand` is referenced only from summary closure and run reports | grep in this session |
| SQLite store has one table | Schema v5: generations, active generation, run manifests, metrics mirror, Go syntax mirror; still maintenance-only in production | `analysis_kernel/store/migrations.rs`; `.planning/phases/65-*/65-06-SUMMARY.md` |
| No F1 gate anywhere | F1 regression gate with cost columns exists, but only runs when oracle clones are present; CI never clones them | `crates/polint-eval/src/harness/external/mod.rs:95-235`; `.github/workflows/ci.yml` gates job |
| `validate_fact_metadata` runs unconditionally | Gated; tests assert both polarities | `analysis_kernel/mod.rs:452, 1081-1116` |
| Whole-program analysis single-threaded | Still true: four `rayon` sites (file read, Go parse, TS parse, rule execution), none in `analysis_neutral` | grep in this session |
| Branch predicates hard-coded to `None` | Still true in both lowerers | `go/mir/lower.rs:1564`, `ts/mir/lower.rs:3670` |
| Crate split into eight crates | Not done; module reorganization plus a layering test | `.swarm/DEFERRED-AFTER-SHIP.md` item 1 |

Net: the engine's L3 and L4 machinery is principled now. The remaining gaps are precision tiers, models, scale, and measurement, not architecture.

## 3. Level-by-level gaps

### 3.1 L2 Semantic resolution

| Capability | Best in class | polint | Gap |
|---|---|---|---|
| Cross-file symbol resolution, Go | `go/types` via gopls and CodeQL's Go extractor | embedded `go/packages` sidecar with `NeedTypes` and `NeedTypesInfo`; `ExactSemantic` precision when setup is available | none material; requires Go 1.25 or newer on `PATH`, surfaced as `polint/capability` diagnostics |
| Cross-file symbol resolution, TS/JS | TypeScript language service; CodeQL TS extractor uses the TS compiler | Oxc semantic plus `oxc_resolver`; no type checker; `Heuristic` and `Unresolved` states exposed | type-level resolution (overloads, generics, declaration files) absent; the repo's own docs list "no declaration-file or project-reference precision claims" (`docs/facts/symbols-and-references.md`) |
| Module and package topology | Sourcegraph SCIP for identity; CodeQL for imports | workspace roots, lockfiles, pnpm and Go MVS-style facts with resolution status | none material |
| Honest unresolved states for rule authors | none of the incumbents expose this as a typed contract | `References::unresolved()`, `ambiguous()`, `polint unknowns --cap references` | polint leads |

### 3.2 L3 Intraprocedural

| Capability | Best in class | polint | Gap |
|---|---|---|---|
| CFG with exceptional and cleanup edges | Clang, `go/ssa`, CodeQL | terminator-driven CFG with `Throw`, `Panic`, `Finally`, `Defer` edge kinds | verify `defer`-at-exit and `finally` bodies execute on every exit in the probe suite; unknown until probed |
| Local abstract domains | Clang SA (symbolic values, ranges), CodeQL range analysis, Frama-C intervals | reachability, nilness, truthiness, constants, strings, initializedness; literal sets capped small | no intervals or ranges, no typestate or resource-state domain, no relational facts |
| Guard and lifecycle policies | CodeQL guards via dominance (`BarrierGuard`), Infer resource leaks | same-function operation order; `max_depth` above 1 ignored | dominance-based proof, interprocedural guard, per-exit cleanup proof |
| Path-sensitive refinement of domains | Clang SA, Coverity | none (predicates unbound) | prerequisite for L6 |

### 3.3 L4 Interprocedural (the certification set)

| Capability | Best in class (evidence) | polint today (evidence) | Gap and cost |
|---|---|---|---|
| Type-directed call-graph tier | CodeQL Go and TS extractors consume compiler types; Go RTA in `x/tools`; XTA-grade resolution is near-linear (Tip and Palsberg OOPSLA 2000) | Go: RTA over sidecar types. TS: no type checker anywhere; direct binding plus Andersen | TS type sidecar. TypeScript 7 (native Go compiler) ships no stable programmatic API until 7.1 ([Microsoft guidance via typescript-go tracking](https://github.com/microsoft/typescript-go), [community migration notes](https://www.sitepoint.com/typescript-70-rc-the-go-rewrite-migration-guide/)), so the sidecar must run the TypeScript 6 API in Node first, exactly as `research/static-analysis-2.0/OPEN-QUESTIONS.md` Q20 decided. Cost: one sidecar, one provider, tier labels on edges |
| Taint decision procedure | CodeQL's shared dataflow library and IFDS solvers (Heros, PhASAR) tabulate summary edges once per callee and answer many queries | bounded path enumeration in the exploded state; defaults depth 8, 20 paths (`sdk/policy.rs:327-334`); engine defaults depth 32, 256 paths (`ifds/mod.rs:37-42`) | replace enumeration with tabulation for the verdict; keep enumeration only to reconstruct a witness path for evidence. Cost: medium; the ICFG and matched boundaries already exist |
| Access-path-sensitive summaries | CodeQL bounded access paths (field flow); Pysa and Zoncolan taint-in-taint-out on parameter paths | `DataFlowTito` at whole-parameter granularity; access-path vocabulary exists with depth but summaries do not use it | k-limited (k=2 or 3) access paths in TITO summaries; the largest single precision lever for request-object taint |
| Framework models as data | CodeQL models-as-data extensions with threat models; Semgrep taint propagators and labels; Pysa `.pysa` models | hard-coded recognizers for `net/http`, `cobra`, `chi`, `gin`, `express`, `fastify`, `koa`, `hapi`, `next`, `nest`, `remix`, `nuxt`, `commander`, `yargs`, MCP SDK; private adaptation TOML | promote models to a documented repo-local artifact with validation, provenance and default-versus-extended reporting; add sink, propagator and sanitizer rows, not only entrypoints |
| Sanitizer semantics | CodeQL barrier guards (dominance-based); Semgrep `pattern-sanitizers` with `by-side-effect` | sanitizer kills on edges, models, nodes and query-named call sites (`ifds/mod.rs:393-427`) | guard-style sanitizers (validate then use on the guarded branch) require branch predicates; defer to L6 prerequisite |
| Source and sink taxonomy | CodeQL per-CWE query suites; Semgrep registry | `http_request`, `secret_like` sources; `call`, `logger` sinks; templates for ten policies edited per repo | this is a product choice (repo-local, no bundled catalog) and remains right; the gap is the model surface, not a catalog |
| Interprocedural constants and nullness | CodeQL, Infer, Checker Framework | domain solver is intraprocedural | IDE lifting once tabulation exists; domains already law-tested |
| Dependency handling | CodeQL analyzes library models, not bodies; Infer summaries; JAM per-package call graphs | dependency bodies analyzed like application code when reachable; no persisted summaries | package-boundary summaries persisted by (package, version) is v2.0 Phase 67; without it depth means cost |
| Taint benchmark | CodeQL query tests per query; Semgrep rule tests; Pysa integration tests; academic corpora (SecBench.js, TaintBench) | one data-flow fixture with four partial edge assertions; ten template self-tests where fixtures name-match the barrier list | the L4 probe suite and a real-app taint corpus (report 04) |

### 3.4 L5 Heap and sensitivity

| Capability | Best in class | polint today | Gap |
|---|---|---|---|
| Points-to for JS/TS | Jelly (Andersen with approximate interpretation), TAJS, CodeQL type tracking | field-sensitive Andersen, context-insensitive, budgets 10,000 steps and 64 objects per variable | selective object sensitivity (Zipper-style on high fan-in of function-valued parameters, `OPEN-QUESTIONS.md` Q25); indirection-bounded propagation as the scale knob (ECOOP 2024, Q8) |
| Go dispatch | `x/tools` RTA and VTA; CodeQL Go | RTA fixpoint in Rust over sidecar method sets, address-taken and dispatch signals | VTA-grade narrowing for function values and interface dispatch when precision on real repos demands it |
| Dependency summaries | JAM modular call graphs; CodeQL library models; Stubbifier's finding that about 56 percent of dependency code is unreachable (cited in `research/static-analysis-2.0/03-summary-store.md`) | none persisted | Phase 67 keystone; the memory win and the recall win are the same feature |
| Approximate interpretation of module initialization | Jelly PLDI 2024 (+12 points recall as cited in the repo) | none | opt-in, sandboxed, explicitly heuristic (Q39); not before L4 certification |

### 3.5 L6 Path sensitivity

| Capability | Best in class | polint today | Gap |
|---|---|---|---|
| Branch predicates in IR | Clang SA, Coverity, Infer Pulse | `predicate_place_key: None` in both lowerers | bind predicate places (cheap; lowerers already see the condition) |
| Feasibility checking | SMT-backed (Clang SA constraint manager), incorrectness logic (Pulse) | none | lightweight feasibility over nullness, constants and intervals first; SMT only if probes show residual false positives |
| Under-approximate review mode | Pulse: every report is a witnessed path | `DataFlowPathStatus::Found` paths are witnesses over may-edges, not feasibility-checked | a review-time precision-first mode that reports only feasibility-checked paths (Q34 already prefers under-approximate defaults for review rules) |

## 4. Axis gaps

| Axis | Best in class | polint | Gap statement |
|---|---|---|---|
| A. Honesty | polint is already near the top; Infer reports summary precision; CodeQL has `@precision` metadata per query but not per finding | per-fact precision, status, budget reasons, unknown taxonomy; policy findings carry status | add a rule-level completeness accessor so a rule can distinguish "clean" from "budget exceeded" when it finds nothing |
| B. Scale | CodeQL disk-backed store; Infer compositional summaries; Glean stacked fact databases; Graspan out-of-core | full pipeline OOM at 12 GB on 86k LOC; single-threaded analysis; global solver budgets exhausted by size; no runtime memory or time envelope | summaries on disk (Phase 67), parallel per-file and per-SCC stages, runtime envelope with reported degradation |
| C. Latency | rust-analyzer and Salsa red-green; Infer diff-time; Semgrep per-file parallelism | syntax and layer caches; no summary reuse; no demand queries; rule host spawn cost | invalidation frontier warm review (REV-01 to REV-03), demand-driven policy queries |
| D. Framework modeling | CodeQL models-as-data plus threat models; Semgrep propagators; Pysa models | hard-coded recognizers; private TOML | models as data with validation and provenance, then agent-authored models measured by default-versus-extended deltas |
| E. Evidence | CodeQL path queries; Snyk CodeReduce slices for LLM fixes (cited in `09-competitive-landscape.md`) | replayable `evidence_v1` with paths, unknowns, omitted regions, replay keys | slices and counter-evidence for agents; summary-segment expansion on demand (Q35) |
| F. Authoring | polint leads; Clippy and `go/analysis` prove typed-code lints scale; CodeQL and Semgrep are DSLs | typed Rust rules, capability derivation, `polint test`, `inspect`, `explain`, templates | cold rule-host compile of 187 s and 582.7 MB (`build-cost.json`); extension author-side surface unbuilt; models private |
| G. Measurement | Jelly's dynamic oracles; SV-COMP culture; CodeQL and Semgrep rule tests | F1 gate exists but is skipped in CI; 53 fixtures; no taint corpus; scale run recorded one OOM | on-demand cloned-oracle gate (manual workflow trigger), probe suite, taint corpus, differential runs, soundness mutations (report 04) |

## 5. Hygiene: what polint must match

These are table stakes for any engine claiming L4. They are listed so the moat argument in section 6 cannot be used to avoid them.

| Requirement | Why incumbents have it | polint status |
|---|---|---|
| Cross-file, cross-package taint composing through dependency boundaries | CodeQL, Semgrep Pro, Infer all do it; buyers test exactly this first | present within budgets; dependency summaries absent |
| Framework knowledge loadable as data | every framework release would otherwise be an engine release | absent as a public artifact |
| Summary contract that is sound modulo declared unknowns | Infer's compositional model; CodeQL's flow summaries | summary top reasons exist (`SummaryTopReason`); no k-limited access paths; no published contract |
| Memory and time envelope | CI runners have 7 to 16 GB | absent at runtime; budgets bound work, not resources |
| Published precision and recall on public corpora | CodeQL and Semgrep publish query test suites; Jelly publishes oracles | none published |
| Realizable paths only | IFDS's balanced-parenthesis property | present |
| Determinism across parallelism and cache state | required for CI trust | present and gated (N=10 permutation gate) |

## 6. polint's moat: what the strongest engines cannot do

| Moat | What polint does | Why incumbents cannot copy it cheaply | Evidence and caveat |
|---|---|---|---|
| Rules as compiler-verified typed code | `#[polint::rule]` derives capabilities from typed fact-view parameters; a wrong rule fails to compile; an unsupported capability refuses to run | CodeQL's QL and Semgrep's YAML are their compatibility surfaces and their moats; making YAML type-check or QL fail fast on a wrong join would be a different product | `crates/polint-macros/src/lib.rs`; `AGENTS.md` rule-authoring contract. Caveat: 187 s cold compile until the thin-SDK build lands |
| Repo-local policy, not CWE catalog | no bundled rules; templates are scaffolds | vendors sell coverage; a registry cannot know a repository's conventions | `README.md`; the product thesis in `research/ROADMAP.md`. Caveat: policy rules still need L4 depth to be worth more than ESLint |
| Per-finding honesty and replayable evidence | precision, status, budget reason, unknown reasons, `evidence_v1` with omitted regions and replay keys | incumbents expose verdicts and traces; none exposes "where I gave up" as a per-finding contract | `sdk/policy.rs`, `diagnostics/mod.rs`. Caveat: no completeness accessor yet |
| Local-first with no cloud dependency | everything runs in the repository; caches on disk; sidecars are local toolchains | Semgrep's essential features moved behind its SaaS platform per the December 2024 change that triggered the Opengrep fork on 23 January 2025 by more than ten vendors ([Socket](https://socket.dev/blog/opengrep-forks-semgrep), [The New Stack](https://thenewstack.io/opengrep-launches-as-free-fork-after-semgrep-license-shift/)); CodeQL's CLI license restricts use on non-open-source code outside GitHub Code Security, priced at 30 dollars per active committer per month since 1 April 2025 ([GitHub changelog](https://github.blog/changelog/2025-04-01-github-secret-protection-and-github-code-security-for-github-enterprise/)) | Caveat: Go semantics need a Go toolchain on `PATH`; TS types will need Node |
| Agent authoring loop | `new-rule` with fixtures, `test`, `inspect rule`, `facts sample`, `unknowns`, `explain`, `ai-friendly` output, generated skill text | incumbents add MCP servers and assistants on top of DSLs; the loop closes only if the artifact is verifiable, which typed code is | `cli/mod.rs` commands. Caveat: extension author-side surface and model authoring are not yet public |
| Determinism as a product property | byte-identical output across cold, warm, parallel and shuffled provider order | most engines do not gate this | determinism gate in CI |

The moat argument is conditional: it holds only if L4 is certified and the scale axis is fixed. A moat around a Semgrep-tier engine is a moat around a small market.

## 7. Prioritized gap list

Ordered by leverage divided by cost; the first five are the L4 certification set.

| # | Gap | Rung or axis | Leverage | Cost | Depends on |
|---|---|---|---|---|---|
| 1 | Taint probe suite and real-app taint corpus with CI gate | G | makes every other item measurable | small to medium | oracle clones fetched by the on-demand workflow |
| 2 | TS type sidecar (TypeScript 6 API in Node) as a typed call-graph tier before the heap | L4 | largest real-world recall lever for TS (repo's own critique 1) | medium | none |
| 3 | IFDS tabulation with summary edges; enumeration only for witnesses | L4 | removes depth-8 defaults; enables IDE lifting | medium | none |
| 4 | k-limited access paths in TITO summaries | L4 | request-object taint precision | medium | 3 |
| 5 | Framework models as data with validation, then agent-authored models | L4, D | recall on real code; the extension thesis becomes real | medium | none |
| 6 | Summary persistence and invalidation frontier (v2.0 Phase 67) | B, C | O(working set) memory; warm review; dependency summaries | large | Phase 65 close, Phase 66 |
| 7 | Runtime memory and time envelope; parallel per-file and per-SCC stages | B | medium repos stop OOMing; CI runners usable | small to medium | none |
| 8 | Rule-level completeness accessor | A | closes the "absence of evidence" hole | small | none |
| 9 | Selective object sensitivity and indirection bounding for TS points-to | L5 | callback-heavy code precision without global cost | medium | 2, 6 |
| 10 | Branch predicates bound in IR; guard-style sanitizers; dominance-based guard policies | L3, L6 prerequisite | unlocks path feasibility and better L3 domains | small then medium | none |
| 11 | Thin-SDK prebuilt rule host (0.3.0 code-preserving build) | F | 187 s cold compile to seconds; offline first run | large | none |
| 12 | Differential testing versus CodeQL, Semgrep and Opengrep on public corpora, published | G | unrefutable head-to-head numbers | medium | 1 |

## 8. What competitors do that polint should not copy

- A universal vulnerability catalog. It is a compliance sale, and it contradicts the repo-local thesis.
- A query language. Rejected in `research/agent-rule-authoring/decisions/001-typed-rust-rules-not-dsl-first.md`; the compiler is the verifier.
- ML detection in the core. Detection stays symbolic; ML at the edges only with verification (`research/static-analysis-2.0/07-ml-integration.md`).
- Whole-database batch builds as the only mode. CodeQL's build cost is the reason it lives in CI rather than at diff time.

## Assumptions and open questions

- Commercial engine internals (Checkmarx, Coverity, Fortify, Snyk Code) are characterized from public documentation and prior knowledge; no runs were performed. **Assumption.**
- Opengrep's cross-file capability as of September 2026 is **unverified**.
- Whether `defer` and `finally` bodies execute on every exit in polint's CFG must be checked by probes; the edge kinds exist, the semantics are unproven here.
- The TS type sidecar's dependency on Node and the TypeScript 6 API is a maintenance liability until TypeScript 7.1 exposes a stable API; the timing is Microsoft's, not polint's.

## References

- `docs/architecture-review/00-TARGET-ARCHITECTURE.md`, `04-analysis-core-capabilities.md`, `05-incrementality-and-store.md`, `06-performance-and-scale.md`, `07-extension-surface.md`, `08-evaluation-and-correctness.md`
- `.swarm/W3.4-LAND.md`, `W3.5-LAND.md`, `W4.1-LAND.md`, `W4.3-LAND.md`, `W4.4-LAND.md`, `READY-TO-SHIP.md`, `DEFERRED-AFTER-SHIP.md`
- `research/static-analysis-2.0/OPEN-QUESTIONS.md` (Q8, Q20, Q25, Q34, Q35, Q39)
- `research/code-preserving-rule-build/FINAL-REPORT.md` section 3.3.2 and 9.3
- CodeQL models-as-data documentation, https://codeql.github.com/docs/codeql-language-guides/customizing-library-models-for-javascript/
- Opengrep launch coverage, https://socket.dev/blog/opengrep-forks-semgrep and https://thenewstack.io/opengrep-launches-as-free-fork-after-semgrep-license-shift/
- GitHub Code Security and Secret Protection pricing, https://github.blog/changelog/2025-04-01-github-secret-protection-and-github-code-security-for-github-enterprise/
- TypeScript 7 native compiler and API timing, https://github.com/microsoft/typescript-go
- Tip and Palsberg, OOPSLA 2000; Li et al. (Zipper), OOPSLA 2018; Chakraborty et al., indirection-bounded call graphs, ECOOP 2024; Laursen et al., PLDI 2024
