# 01 — The Summit Definition: A Capability Ladder for Static-Analysis Engines

Date: 2026-09-01
Researcher: Claude Fable 5.1 (delegated via Hermes)
Revised 2026-09-02: time estimates removed, capability roadmap added.
Status: strategy input for the milestone after v2.0. Companion reports: [02-gap-analysis.md](02-gap-analysis.md), [03-build-plan.md](03-build-plan.md), [04-evaluation-as-a-weapon.md](04-evaluation-as-a-weapon.md), [05-moat-economics.md](05-moat-economics.md).

## TL;DR

- "World's most capable" is not one number. It is a position on a ladder of eight **conclusion levels** (L0 lexical to L7 verification-grade) crossed with seven **orthogonal axes** (honesty, scale, latency, framework modeling, evidence, authoring, measurement). Every level is defined by the class of real bugs it can conclude that the level below cannot, and every placement must be provable by a probe suite, not by a feature list.
- The literature draws the level boundaries for us: name binding (L2), flow sensitivity and local lattices (L3), interprocedural summaries and IFDS/IDE realizable paths (L4), heap/points-to with context, field and object sensitivity (L5), path conditions and under-approximate bug proofs (L6), and sound abstract interpretation or deductive verification (L7).
- No shipping engine sits at L7 for general code. The commercial and open-source leaders (CodeQL, Semgrep Pro, Infer, Coverity, Sonar) cluster at L4 with selective L5 and L6 machinery. Verification tools (Kani, Prusti, Verus, Astrée, Frama-C) reach L7 only with specs, harnesses, or restricted languages.
- polint today, verified from the code at `crates/polint/src`, is **solid L3 and partial L4 for Go and TS/JS**: a real IR with blocks and terminators, six local abstract domains, four summary domains with SCC closure, a matched call/return IFDS-style taint path search with sanitizer kills, Go RTA, and a field-sensitive Andersen solver for TS/JS. It has L5 seeds (Andersen, access-path vocabulary) and nothing at L6 or L7.
- On the axes, polint is unusually strong on honesty (per-fact precision, status, budget and unknown taxonomies), authoring (typed Rust rules with compiler-verified capabilities, fixtures, inspect/explain), and evidence (validated `evidence_v1` envelopes on policy findings). It is weak on scale (an 86k-LOC TypeScript repo OOMs the full pipeline at 12 GB), latency (no summary persistence, no demand-driven queries), framework modeling (about fifteen hard-coded recognizers, private model files), and measurement (no accuracy gate runs in CI, no taint benchmark).
- The distance from "partial L4" to "certified L4" is the highest-value distance on the whole ladder: it is where CodeQL and Semgrep Pro earn their keep, where taint policies become trustworthy, and where the honesty axis becomes a moat instead of an apology.
- L5 is worth buying selectively (object sensitivity for callback-heavy TS, access-path sensitivity in summaries), not uniformly. L6 is worth buying only after branch predicates exist in the IR; today they are lowered as `None`.
- Placement is verified by four instruments defined in this report and detailed in report 04: per-level capability probes, recall against dynamic oracles, precision against curated required sets, and soundness mutation tests.
- The bar polint should hold itself to: certified L4 on Go and TS/JS with published probe pass rates, selective L5, and top-of-class scores on all seven axes, at the end of the six-stage build plan. That is a position no current engine holds for these two languages.

## 1. Why a ladder, and why these rungs

Every engine markets a feature list. Feature lists cannot be compared, and the same word ("taint", "interprocedural", "call graph") covers a range of precision that spans two orders of magnitude in real bug yield. The program-analysis literature, however, already stratifies analyses by the *machinery* they use, and the machinery determines the *classes of bugs* an engine can conclude. That stratification is objective and testable:

| Boundary | Defining machinery | Defining literature |
|---|---|---|
| L0 to L1 | parsing to a syntax tree; pattern matching with metavariables | tree-sitter / ast-grep / Semgrep pattern semantics |
| L1 to L2 | name binding, scopes, imports, declared types across files | classic compiler front-end theory; SCIP / LSIF symbol identity |
| L2 to L3 | control-flow graphs, monotone dataflow frameworks, abstract interpretation with widening | Kildall 1973; Cousot and Cousot, POPL 1977 |
| L3 to L4 | call graphs (CHA, RTA, XTA), functional summaries, IFDS/IDE realizable-path reachability | Sharir and Pnueli 1981; Reps, Horwitz, Sagiv POPL 1995; Tip and Palsberg OOPSLA 2000 |
| L4 to L5 | inclusion-based points-to, field, object and context sensitivity, k-limited access paths | Andersen 1994; Milanova et al. TOSEM 2005; Smaragdakis et al. POPL 2011 |
| L5 to L6 | path conditions, symbolic execution, SAT/SMT feasibility, under-approximate (incorrectness) reasoning | Das, Lerner, Seigle PLDI 2002 (ESP); Xie and Aiken (Saturn); O'Hearn POPL 2020 (Incorrectness Logic) |
| L6 to L7 | soundness proofs of absence, deductive verification, bounded model checking | Astrée (Blanchet et al. PLDI 2003); Frama-C/Eva; Kani; Prusti (OOPSLA 2019); Verus (OOPSLA 2023) |

The cross-cutting caveat from the soundiness manifesto (Livshits et al., CACM 2015) applies to every rung: production analyzers are "soundy" by design, so a level claim is always "for the constructs the engine models, with the unknowns it reports". That is precisely why the honesty axis is graded separately.

## 2. The conclusion levels

Each level lists what it can newly conclude, an example bug class, exemplar tools, and the cost class. "Newly" means the level below cannot conclude it without guessing.

### L0 Lexical
- Machinery: regular expressions over text.
- Concludes: presence of a token or string.
- Bug classes: hard-coded secrets by pattern, banned words.
- Cost: linear, trivially parallel.

### L1 Syntactic
- Machinery: concrete or abstract syntax trees, structural pattern matching with metavariables, per file.
- Newly concludes: shape of an expression or statement, independent of formatting; argument positions; nesting.
- Bug classes: calling an API by literal name with a dangerous argument shape; forbidden constructs; style and convention rules.
- Cannot conclude: whether `db` in `db.exec(...)` is the database client or an unrelated local, whether an import alias renames a forbidden module.
- Exemplars: ast-grep, Semgrep Community Edition (pattern mode), most ESLint core rules.

### L2 Semantic-resolved
- Machinery: scopes, symbol tables, cross-file import resolution, declared types, module and package graph.
- Newly concludes: which declaration a name refers to; which package a file imports through aliases and re-exports; who references an exported symbol.
- Bug classes: layering violations through aliased imports; use of a deprecated symbol regardless of spelling; unused exports; architectural boundary rules; typed API misuse where the type is declared.
- Exemplars: typescript-eslint typed rules, `go vet` analyzers on `go/types`, polint's `Symbols`, `References`, `ResolvedImports`, `ModuleGraphFacts` views.

### L3 Intraprocedural flow-sensitive
- Machinery: per-function CFG with normal and exceptional edges, dominance, reaching definitions, local abstract domains (nullness, constants, initializedness, typestate), local taint.
- Newly concludes: facts that depend on statement order and branches inside one function.
- Bug classes: nil dereference after a nil check on the other branch; use before initialization; missing cleanup on one exit path; guard missing before a sensitive call in the same function; redundant or impossible conditions.
- Exemplars: `go vet` nilness (SSA-based), Clang-Tidy dataflow checks, NullAway intraprocedural core, polint's `domains` kernel and `ControlFlow` view.

### L4 Interprocedural with summaries and realizable paths
- Machinery: call graph (CHA, RTA, XTA or type-directed), functional or IFDS/IDE summaries, matched call and return edges, framework models (sources, sinks, sanitizers, propagators) as data.
- Newly concludes: properties that cross function and file boundaries without merging distinct call sites (the balanced-parenthesis property of IFDS).
- Bug classes: SQL injection through two helper functions and a framework route; secrets flowing to a logger via a wrapper; dangerous API reachable from an unauthenticated entrypoint; resource leaks across calls.
- Exemplars: CodeQL dataflow and taint libraries, Semgrep Pro cross-file taint, Infer's compositional summaries, Meta Zoncolan and Pysa, Checkmarx, Snyk Code, SonarQube taint engine (commercial editions).

### L5 Whole-program heap with sensitivity
- Machinery: inclusion-based points-to (Andersen) with field sensitivity, object or call-site sensitivity (k-CFA, k-object), k-limited access paths in summaries, selective sensitivity (Zipper, Scaler).
- Newly concludes: which callback stored in a structure is invoked later; which object field carries taint when two fields of the same object differ; that a shared helper does not smear taint between unrelated callers.
- Bug classes: callback-in-registry dispatch (Express middleware, event emitters), prototype and class dispatch in JS, taint through `req.body.name` but not `req.body.id`, dependency-injection dispatch.
- Exemplars: Doop, WALA, SVF, TAJS, Jelly (JS), CodeQL's JS/TS type tracking with access paths; polint's TS/JS Andersen solver is L5 machinery without context sensitivity.
- Known numbers (as cited in the repo's own research, `research/static-analysis-2.0/05-type-directed-callgraph.md` and `06-selective-precision-and-demand.md`): XTA shrinks type sets about 88 percent versus RTA at about 12 times CHA cost (Tip and Palsberg, OOPSLA 2000); Zipper keeps 98.8 percent of 2-object precision at 3.4 times average speedup (OOPSLA 2018).

### L6 Path-sensitive, symbolic, under-approximate
- Machinery: path conditions on branch predicates, symbolic execution with SAT/SMT or lightweight feasibility, incorrectness logic (every report is a witnessed path).
- Newly concludes: that a defect occurs only under a feasible combination of branch outcomes; that a reported path is feasible, not merely reachable.
- Bug classes: double close only when an error flag is set; null dereference only when two options interact; eliminating false positives on infeasible paths.
- Exemplars: Clang Static Analyzer, Coverity, Infer Pulse (under-approximate), KLEE-style engines (dynamic symbolic, adjacent).
- Precondition: branch predicates bound to places in the IR. polint lowers `predicate_place_key: None` in both frontends today (`crates/polint/src/go/mir/lower.rs:1564`, `crates/polint/src/ts/mir/lower.rs:3670`), so L6 is structurally unreachable until that changes.

### L7 Verification-grade
- Machinery: sound abstract interpretation with proofs of absence for a bug class (Astrée, Frama-C/Eva, IKOS, Goblint), deductive verification (Prusti, Verus, Creusot, Dafny), bounded model checking (Kani).
- Newly concludes: absence of a bug class in a module, or functional correctness against a specification.
- Cost: specifications, harnesses or annotations from the user; restricted language subsets; minutes to hours per unit.
- Relevance to polint: out of scope for Go and TS/JS as a product level. Useful as the ceiling that keeps honesty claims calibrated.

## 3. The orthogonal axes

A level says what an engine can conclude. The axes say whether anyone can trust, afford, and extend it. Each axis is graded 0 to 3.

| Axis | 0 | 1 | 2 | 3 |
|---|---|---|---|---|
| A. Honesty (unsoundness accounting) | none | logged in a debug channel | per-finding precision and status, unknown taxonomy | plus a rule-level completeness query: a rule can ask whether "no finding" means clean or budget exceeded |
| B. Scale envelope | whole program in RAM, no budgets | budgets on solver work | memory proportional to working set through summaries on disk | bounded envelope enforced at runtime (time and memory), degradation reported |
| C. Latency and incrementality | batch only | parse cache | summary-frontier warm runs proportional to the change | demand-driven, editor-latency queries |
| D. Framework modeling | hard-coded recognizers | models as data shipped by vendor | repo-local models with validation | agent-authored models validated against fixtures and benchmarks with provenance |
| E. Evidence | verdict only | textual trace | replayable path with precision, unknowns, truncation accounting | plus slices and counter-evidence hooks for agents |
| F. Authoring and extensibility | none | DSL or YAML | typed code with compile-time capability checks | typed code plus agent loop (scaffold, test, inspect, explain, diff) |
| G. Measurement rigor | none | vendor micro-benchmarks | public reproducible harness with pinned corpora | plus third-party oracles, differential testing and soundness mutation tests gated in CI |

The axes are where polint's product thesis lives (`research/ROADMAP.md`, "Product Thesis: Agent-Extensible Static Analysis"). A engine that is L4 with axes at 3 beats an engine that is L5 with axes at 1 for every buyer who has to act on findings.

## 4. Placement of leading engines

Placement is by demonstrated machinery from public documentation and papers; where a claim could not be verified in this session it is marked. Level ranges show "core level (selective higher machinery)".

| Engine | Level | A | B | C | D | E | F | G | Notes and evidence class |
|---|---|---|---|---|---|---|---|---|---|
| CodeQL (GitHub) | L4 (L5 access paths and type tracking; L6-lite range analysis and barrier guards) | 2 | 2 | 1 | 2 | 2 | 1 (QL) | 2 | Relational store built by extractors; models-as-data extensions; incremental evaluation published as research (arXiv:2308.09660) not as the default product path; license restricts non-open-source use outside GitHub Advanced Security |
| Semgrep Pro | L4 (cross-file, cross-function taint) | 1 | 2 | 2 | 2 | 1 | 1 (YAML) | 1 | Per-language depth varies; Assistant triage reported at about 96 percent agreement (`research/static-analysis-2.0/07-ml-integration.md`) |
| Semgrep CE / Opengrep | L1 to L2 (intra-file taint) | 1 | 2 | 2 | 1 | 1 | 1 | 1 | Opengrep is the January 2025 community fork after the Semgrep licensing changes; cross-file depth in Opengrep is unverified in this session |
| ast-grep | L1 | 0 | 3 | 2 | 0 | 0 | 1 (YAML plus API) | 0 | Fast structural search; explicitly not a semantic engine |
| Infer (Meta) | L4 compositional (L6 under-approximate via Pulse) | 2 | 3 | 2 | 1 | 2 | 1 (OCaml checkers) | 2 | Existence proof for summaries at scale and diff-time deployment (CACM 2019); no Go or TS/JS |
| Doop / WALA / SVF | L5 | 1 | 1 | 0 | 1 | 1 | 1 | 2 | Research points-to engines; context sensitivity as configuration; batch only |
| Joern | L4 to L5 heuristic (code property graph) | 1 | 1 | 0 | 1 | 2 | 1 (Scala DSL) | 1 | JoernTI ships verified neural type inference |
| Jelly (Aarhus) | L5 (JS) | 2 | 1 | 0 | 1 | 1 | 0 | 3 | Dynamic-trace oracle on 141 Node projects; static-only recall about 75.9 percent, 88.1 percent with approximate interpretation, as cited in `research/static-analysis-2.0/01-benchmarking-and-measurement.md` |
| SonarQube (commercial editions) | L2 to L4 (taint in paid editions) | 1 | 2 | 2 | 1 | 1 | 1 (Java plugin API) | 1 | Breadth and enterprise distribution; custom rules limited by edition |
| Snyk Code / Checkmarx / Coverity / Fortify | L4 (Coverity L6) | 1 | 2 | 2 | 1 | 1 | 0 to 1 | 1 | Closed engines; custom rules range from none (Veracode) to proprietary query languages (CxQL, CodeXM) |
| Clang Static Analyzer | L6 (intra-TU, cross-TU optional) | 1 | 1 | 1 | 1 | 2 | 1 (C++ checkers) | 2 | Path-sensitive symbolic execution; the reference for L6 cost and false-positive posture |
| Kani / Prusti / Verus / Astrée / Frama-C | L7 | 3 | 1 | 0 | 0 | 2 | 1 to 2 | 3 | Specs or harnesses required; Rust or C only; SV-COMP-style validation culture |
| AI reviewers (Cursor Bugbot, Copilot code review, CodeRabbit) | no fixed level | 0 | 3 | 3 | 2 | 1 | 2 (natural-language rules) | 0 to 1 | Heuristic LLM reading of diffs, sometimes with repo indexing; no soundness posture; independent precision numbers scarce and vendor-run |
| **polint (2026-09-01)** | **L3 solid; L4 partial; L5 seeds** | **2** | **1** | **1** | **1** | **2** | **3** | **1** | Detailed in section 5 |

Two placements deserve a defense. CodeQL is placed at L4 core rather than L5 because its shared dataflow library is context-insensitive by default with bounded access paths and call-context tracking only where the query opts in; that is L4 machinery with L5 features. AI reviewers get no level because their conclusions are not reproducible functions of the program; they are graded on axes only, where they score high on latency and framework awareness and zero on honesty and measurement.

## 5. Where polint stands, verified from the code

This section reads the crate, not the research documents. Line references are to the tree at commit `9b6ac59d` (v0.3.3).

### 5.1 Level machinery present

| Level | Present | Evidence |
|---|---|---|
| L1 | yes | tree-sitter Go and Oxc TS/JS syntax facts; `StringLiterals`, `JsxAttributes`, `Functions`, `Imports` views (`crates/polint/src/sdk/facts.rs`) |
| L2 | yes | symbol graph, references with resolution status and precision, resolved imports, module topology (`crates/polint/src/symbol_graph/`, `module_graph/`); Go symbols from an embedded `go/packages` sidecar with `NeedTypes` and `NeedTypesInfo` (`tools/polint-go-symbols/internal/symbols/emit.go:197-199`) |
| L3 | yes | neutral IR with `MirBlock` and terminators `Goto`, `Branch`, `Switch`, `Return`, `Throw`, `Call { unwind }`, `Suspend`, `Unreachable`, `Unsupported` (`crates/polint/src/ir/body.rs:51-93`); CFG edge kinds including `Throw`, `ImplicitThrow`, `Panic`, `Finally`, `Cleanup`, `Defer` (`crates/polint/src/analysis_neutral/cfg/facts.rs:180-199`); six local domains: reachability, nilness, truthiness, constants, strings, initializedness (`crates/polint/src/analysis_neutral/domains/facts.rs:10-17`); dominance and control dependence in `cfg/derived.rs` |
| L4 | partial | direct call facts plus Go RTA fixpoint (`analysis_neutral/solver/go_rta/fixpoint.rs`); four summary domains `ControlEffects`, `CallEffects`, `MemoryEffects`, `DataFlowTito` with SCC closure (`analysis_neutral/summaries/core.rs:37-445`); matched call and return taint path search with a call stack in the exploded state and sanitizer kills (`analysis_neutral/ifds/mod.rs:191-447`); trust-boundary sources from framework recognizers; `DataFlow::forbidden`, `Calls::forbidden_reachable`, `ControlFlow::missing_guard` and `missing_cleanup` policy views |
| L5 | seeds | field-sensitive, context-insensitive Andersen solver as the sole TS/JS indirect-call resolver (`analysis_neutral/points_to/solver.rs`, budget 10,000 steps, 64 objects per variable); access-path vocabulary with depth (`analysis_neutral/access_paths/facts.rs`); alias query index answering NoAlias/MayAlias/MustAlias/Unknown (`analysis_neutral/aliases/query.rs`) |
| L6 | absent | branch predicates unbound (`predicate_place_key: None`); no path conditions, no SMT |
| L7 | absent | by design |

### 5.2 What "partial L4" means precisely

- The taint decision procedure is a bounded path enumeration over the exploded state `(node, Tainted, call_stack)`, not a tabulation with summary edges. It guarantees realizable paths (return must pop the matching call site, `ifds/mod.rs:365-390`) but pays for it in search cost, so the SDK defaults are depth 8 and 20 paths (`crates/polint/src/sdk/policy.rs:327-334`). Real vulnerability chains routinely exceed eight interprocedural hops.
- Summaries carry whole-parameter TITO facts (`DataFlowTito`), not k-limited access paths. `param0.body.name -> return.html` is not expressible, so taint through structured request objects is either over-approximated or lost.
- The call graph for TS/JS is direct binding plus Andersen points-to. There is no type-directed tier: no TypeScript type checker is consulted anywhere in the crate. The repo's own review named this "the single highest-leverage strategic correction" (`research/static-analysis-2.0/00-critical-review.md`, critique 1).
- Framework modeling is hard-coded: Go recognizers cover `net/http`, `cobra`, `chi`, `gin`; TS recognizers cover `express`, `fastify`, `koa`, `hapi`, `next`, `nest`, `remix`, `nuxt`, `commander`, `yargs`, and the MCP SDK (`analysis_neutral/entrypoints/recognizers_{go,ts}.rs`). The adaptation TOML model layer exists but is private and undocumented.
- Control-flow policies are same-function only (`GuardQuery::max_depth` above 1 does nothing; `docs/facts/control-flow.md`).
- Budgets are honest but small and global: `max_steps` 10,000 and `max_outer_iterations` 64 per run (`analysis_neutral/solver/budget.rs:111-114`). A large repository exhausts them by size alone.

### 5.3 Axis placement

| Axis | Grade | Why |
|---|---|---|
| A. Honesty | 2, close to 3 | every fact family carries precision, status, provenance and budget state; unknown taxonomy via `polint inspect unknowns`; data-flow queries surface `BudgetExceeded` and `Unknown` as violation rows. Missing: a rule-level completeness accessor; `RuleCtx` exposes `capability_support()` only (`crates/polint/src/core/rule.rs:197`) |
| B. Scale | 1 | full pipeline on excalidraw v0.17.6 (86,527 LOC) was killed after about 1,026 s at 12 GB or more RSS (`research/evaluation-harness/baselines/scale-corpus-run.json`); capability-gated syntactic runs are cheap (private monorepo about 1 GB, 7.4 s cold; two private benchmark repos about 2 to 3 s after the v0.3.3 speedups). Whole-program analysis is single-threaded (four `rayon` sites, none in `analysis_neutral`) |
| C. Latency | 1 | per-file syntax cache and whole-layer caches; semantic store is schema v5 with generations, run manifests and two provider mirrors but production runs are maintenance-only with no reuse (`analysis_kernel/store/`); no summary persistence, no demand queries |
| D. Framework modeling | 1 | hard-coded recognizers plus private adaptation models; extension protocol host-side only |
| E. Evidence | 2 | policy findings carry validated `evidence_v1` envelopes with paths, unknowns, omitted regions and replay keys, kept across the rule-host boundary (`crates/polint/src/diagnostics/mod.rs:237-262`); no slices for agents |
| F. Authoring | 3 | `#[polint::rule]` derives capabilities from typed views; `polint new-rule`, `polint test`, `polint inspect rule`, `polint facts`, `polint unknowns`, `polint explain`; ten policy templates. Cost: a cold rule-host build compiles 225 units in 187 s (`research/evaluation-harness/baselines/build-cost.json`) |
| G. Measurement | 1 | Jelly micro and Go x/tools adapters with an F1 gate exist (`crates/polint-eval/src/harness/external/mod.rs`) but the gate only runs when clones are present, which CI never provides; 53 native fixture cases, one for data flow; no taint benchmark; committed baseline: Jelly recall 66.5 percent at 97.4 percent precision, Go x/tools recall 100 percent at 4.4 percent "precision" against a partial oracle (`research/evaluation-harness/baselines/persisted-graph-accuracy.json`) |

### 5.4 The honest one-line placement

polint is an L3-complete, L4-partial engine for Go and TypeScript/JavaScript with L5 machinery in place for TS/JS, whose axes are inverted relative to incumbents: best-in-class on authoring, honesty and evidence, below par on scale, latency, framework modeling and measurement.

## 6. What each next rung buys, in bug classes

The founder's constraint is depth, not breadth. This table is the depth agenda expressed as bug classes that repo-local policies want to enforce and cannot today.

| Rung to earn | Newly enforceable repo policies (examples from the shipped templates and README) | Blocking gap today |
|---|---|---|
| L4 certified | request data to shell through two helpers and a router; secret to log through a wrapper; raw admin API reachable from a production root across packages; dangerous HTML through a template helper | depth-8 path search, whole-parameter summaries, no TS types, hard-coded frameworks, no taint benchmark |
| L5 selective | Express middleware and event-emitter dispatch; taint in `req.body.name` but not `req.body.id`; ORM callbacks; DI containers | no object sensitivity, no access-path summaries |
| L6 selective | guard-before-write proven on every path, not by same-function ordering; cleanup on every exit including error paths; findings suppressed on infeasible branches | branch predicates unbound in the IR |
| Interprocedural L3 domains (IDE lifting) | nil flows across calls, constant configuration values across modules | domain solver is intraprocedural |

## 7. Verification instruments for a placement claim

A placement is a claim about machinery; it must be falsifiable by a third party. Four instruments, specified in report 04:

1. **Capability probes.** Small programs per level and language where the bug is findable only with that level's machinery, paired with a must-not-report twin (an unrealizable path, an infeasible branch, a distinct field). A level is "certified" when the engine passes at least 90 percent of positives and 100 percent of must-not-report twins for that level.
2. **Recall against dynamic oracles.** Call-graph recall against execution traces (Jelly's NodeProf artifact for JS; `go test` instrumented traces or `x/tools` RTA and VTA references for Go). Dynamic oracles under-approximate, so they are a recall floor, never a precision judge.
3. **Precision against curated required sets.** Hand-adjudicated required-edge and required-finding sets on real repositories, reported as a separate lane and never blended with the dynamic lane (the repo already decided this, `research/static-analysis-2.0/OPEN-QUESTIONS.md` Q3).
4. **Soundness mutation.** Inject a bug that the claimed level must catch and a transformation that must not create a finding (rename, split into a helper, move across a file, wrap in a callback). A level claim survives only if the injected bug is caught and the transformations preserve the finding set.

## 8. Placement targets for the plan

| Milestone (dependency order) | Level target | Axis targets (A to G) | Proof |
|---|---|---|---|
| After Stage 0 | L3 certified, L4 partial with measured probes | 3, 1, 1, 2, 2, 3, 2 | probes for L1 to L3 pass; on-demand accuracy gate (manual workflow run) scores cloned oracles with runtime and peak RSS recorded |
| After Stage 2 | L4 certified for Go and TS/JS | 3, 2, 2, 2, 2, 3, 3 | taint probe suite and real-app taint corpus published; TS type tier measured; summaries persisted |
| After Stage 4 | L4 certified plus selective L5 | 3, 3, 3, 3, 3, 3, 3 | object-sensitive probes pass; grafana-scale runs under a fixed envelope |
| After Stage 5 | selective L6 | hold | path-feasibility probes; under-approximate review mode; differential results versus CodeQL and Semgrep published |

Report 03 sequences the work; report 04 defines the instruments; report 02 lists, per rung, exactly what the strongest engine does that polint does not.

## Assumptions and open questions

- Placements of closed commercial engines (Checkmarx, Coverity, Fortify, Snyk Code, Veracode) rest on public documentation and prior knowledge, not on runs performed in this session. Treat them as **assumptions** to be replaced by differential runs (report 04).
- Opengrep's interprocedural depth as of September 2026 is **unverified**; it is placed at Semgrep Community Edition level.
- Whether GitHub still ships stack-graphs-based precise navigation is **unverified** and immaterial to placement.
- The Jelly real-application recall figures are cited as the repo cites them; the numbers should be re-read from the PLDI 2024 paper before any public comparison.
- The Go x/tools "precision" figure is an artifact of a partial oracle, not a measurement of polint; it is listed to show why oracle design is part of the ladder.

## References

- `research/static-analysis-2.0/00-critical-review.md`, `01-benchmarking-and-measurement.md`, `05-type-directed-callgraph.md`, `06-selective-precision-and-demand.md`, `09-competitive-landscape.md`, `OPEN-QUESTIONS.md`
- `docs/architecture-review/04-analysis-core-capabilities.md`, `10-sota-landscape-and-bar.md` (July 2026 review; several findings superseded by the August refactor, see report 02)
- `crates/polint/src/ir/body.rs`, `crates/polint/src/analysis_neutral/ifds/mod.rs`, `crates/polint/src/analysis_neutral/domains/facts.rs`, `crates/polint/src/analysis_neutral/summaries/core.rs`, `crates/polint/src/analysis_neutral/points_to/solver.rs`, `crates/polint/src/analysis_neutral/solver/budget.rs`, `crates/polint/src/sdk/policy.rs`, `crates/polint/src/core/rule.rs`
- `research/evaluation-harness/baselines/persisted-graph-accuracy.json`, `scale-corpus-run.json`, `build-cost.json`
- Reps, Horwitz, Sagiv, "Precise interprocedural dataflow analysis via graph reachability", POPL 1995
- Tip and Palsberg, "Scalable propagation-based call graph construction algorithms", OOPSLA 2000, http://web.cs.ucla.edu/~palsberg/paper/oopsla00.pdf
- Smaragdakis, Bravenboer, Lhoták, "Pick your contexts well", POPL 2011
- Li, Tan, Møller, Smaragdakis, "Precision-guided context sensitivity" (Zipper), OOPSLA 2018, https://cs.au.dk/~amoeller/papers/zipper/paper.pdf
- Livshits et al., "In defense of soundiness: a manifesto", CACM 2015, http://soundiness.org
- O'Hearn, "Incorrectness Logic", POPL 2020
- Distefano, Fähndrich, Logozzo, O'Hearn, "Scaling static analyses at Facebook", CACM 2019
- Szabó et al., "Incrementalizing production CodeQL analyses", ESEC/FSE 2023, https://arxiv.org/abs/2308.09660
- Laursen et al., "Reducing static analysis unsoundness with approximate interpretation", PLDI 2024, https://dl.acm.org/doi/10.1145/3656424
