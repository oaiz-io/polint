# Static Analysis Research Roadmap

Date: 2026-05-16

## Research And Implementation TODO

- [x] Semantic index baseline implemented: `Symbols<'_>` and `References<'_>` are available through the SDK.
- [x] Call graph research completed: see `research/call-graphs/`.
- [x] Data-flow research completed: see `research/data-flow/`.
- [x] Agent extension surface research completed: see `research/agent-extension-surface/`.
- [x] Analysis kernel research completed: fact layers, scheduling, provenance, precision, validation, extension merges, cache keys, and invalidation. See `research/analysis-kernel/`.
- [x] Evaluation harness research completed: external-benchmark-first strategy, default-vs-agent-extended metrics, fixtures, ground truth, graph/path quality, runtime, memory, regression gates, and benchmark adapters. See `research/evaluation-harness/`.
- [x] Framework, lifecycle, and entrypoint modeling research completed: routes, jobs, queues, CLIs, MCP tools, serverless handlers, callbacks, decorators, generated dispatch, native Rust provider path, and validation strategy. See `research/framework-entrypoints/`.
- [x] Semantic index deep research completed: scopes, aliases, generated symbols, type-aware resolution, unresolved references, extension-provided resolution facts, and export identity. See `research/semantic-index/`.
- [x] Module/package/dependency/repo topology graph research completed: package managers, lockfiles, workspaces, import-to-package resolution, source sets, build targets, repo topology overlays, and extension facts. See `research/module-graph/`.
- [x] CFG and control dependence research completed: operation nodes, basic blocks, typed normal/abrupt/exceptional edges, dominance/postdominance, control dependence, path evidence, and extension overlays. See `research/cfg-control-flow/`.
- [x] Type, value, points-to, and alias analysis research completed: native type/value/place facts, flow narrowing, summaries, bounded Andersen-style points-to, alias provider stack, precision/cost ladder, and extension hooks. See `research/type-alias-points-to/`.
- [x] Function effects and summaries research completed: typed summary domains, SCC fixpoint, provenance, precision, cache keys, extension validation, and implementation path. See `research/effects-summaries/`.
- [x] Abstract interpretation domains research completed: reduced-product domain kernel, lattice/transfer interfaces, widening/narrowing, domain priorities, validation, extension-safe domain products, and benchmark strategy. See `research/abstract-interpretation/`.
- [x] Implementation-ready Rust bootstrap design completed: semantic MIR, place identity, direct call facts, P0 domains, direct summaries, minimal cache/invalidation, model-extension sinks, and current code review. See `research/implementation-bootstrap/`.
- [x] Call graph implementation design revised against the semantic bootstrap, analysis kernel, summaries, type/value/alias facts, framework models, abstract domains, and evaluation harness. See `research/call-graphs/implementation/BOOTSTRAP-INTEGRATION.md`.
- [x] Data-flow implementation design revised against the analysis kernel, call graph, CFG, summaries, abstract domains, and evaluation harness. See `research/data-flow/implementation/BOOTSTRAP-INTEGRATION.md`.
- [x] Program slicing, path explanation, and evidence research completed: PDG/SDG slicing, thin slices, chops, path ranking, SARIF/JSON evidence, provenance, extension merges, and native implementation path. See `research/program-slicing-evidence/`.
- [x] Incremental query engine and caching research completed: native layered cache/query design, input snapshots, shape digests, dependency indexes, invalidation planning, summary SCC caching, extension quarantine, and future red-green/relation paths. See `research/incremental-query-engine/`.
- [x] Rule SDK, query ergonomics, and AI-agent authoring research completed: typed Rust rule surface, rule manifests, narrow `RuleCtx`, model/provider boundary, `polint test`, inspect/explain/diff tooling, and fixture-first agent workflow. See `research/agent-rule-authoring/`.
- [x] Local semantic store research completed: SQLite/rusqlite primary store, typed graph adjacency/evidence indexes, registry-ready summary manifests without a registry now, Tantivy lexical search path, sqlite-vec experimental boundary, and validation/benchmark gates. See `research/local-semantic-store/`.
- [x] Code-preserving rule-build research completed: what a repo-local rule host actually compiles today, seven code-preserving build/distribution/execution alternatives with explicit rejection reasons, the recommended prebuilt-host + thin-SDK + fact-snapshot architecture, security and trust boundaries, and an implementation-grade phased plan. Product decision: propose an intentionally breaking 0.3.0 rule-pack manifest/build/execution migration with no legacy backend, automatic fallback, or multi-minor deprecation window. Rules stay real, typed Rust and their `.rs` source/API remains stable; a DSL remains rejected as the primary model. See `research/code-preserving-rule-build/`.
- [x] Rule-host build cost measured: `polint-bench build-cost` / `make build-cost`, with a committed baseline at `research/evaluation-harness/baselines/build-cost.json`. Next steps for that track are its Phase A2 (widen the repo matrix), A5, A6, and Phase B (dependency closure and feature-leak guards) in `research/code-preserving-rule-build/IMPLEMENTATION-PLAN.md` §8.
- [x] Package-graph direction and iteration 1 planned (2026-11-17): the diagnosis of the JS/TS classical layer corrected and measured (a budget latch discards the points-to solve on every package above about a thousand call sites; no call rule; lowering gaps listed from twenty traced edges), Jelly and ACG measured on the bench (Jelly static 84.5%), the held-out set pinned, the closed lists of tiers, assumptions and constructs written, storage option A checked against the solver, targets revised, and the pull-request sequence planned as GSD phases 72 to 79. See `research/package-graphs/` (`DIAGNOSIS.md`, `BASELINES.md`, `HELD-OUT.md`, `TIERS.md`, `STORAGE-A.md`, `TARGETS.md`, `implementation/README.md`). Next step: Phase 72, the measurement baseline.
- [ ] PR 1: Introduce the private analysis kernel facade and provider manifests without changing behavior.
- [ ] PR 2: Add provenance, precision, validation, and merge metadata for existing fact families.
- [ ] PR 3: Add the internal evaluation harness MVP and native fixtures.
- [ ] PR 4: Add `InputSnapshot`, typed cache keys, and layer-cache instrumentation.
- [ ] PR 5: Add the SQLite/rusqlite local semantic store skeleton, migrations, store manifest, and persistent layer entries for existing cheap facts with conservative invalidation.
- [ ] PR 6: Add generated rule manifests, `polint inspect rule`, and the first `polint test` fixture runner.
- [ ] PR 7: Deepen the semantic index with scopes, imports, resolution status, and explicit unknowns.
- [ ] PR 8: Ship the first layered module/package/dependency topology graph.
- [ ] PR 9: Add private semantic MIR and normalized place identity for Go and TS/JS.
- [ ] PR 10: Add local CFG, dominance/postdominance, and control-dependence facts.
- [ ] PR 11: Add direct call-site, direct target, and unresolved-call facts.
- [ ] PR 12: Add the P0 abstract-domain kernel over MIR/CFG.
- [ ] PR 13: Add the summary kernel and direct function summaries.
- [ ] PR 14: Add the demand query layer, summary SCC cache, and extension-aware cache quarantine.
- [ ] PR 15: Add the Rust extension/provider sink and model lifecycle.
- [ ] PR 16: Add framework entrypoint, lifecycle, dispatch, and trust-boundary facts.
- [ ] PR 17: Add the P0 type/value/place/alias substrate.
- [ ] PR 18: Add refined call graph providers.
- [ ] PR 19: Add local plus summary-projected data flow.
- [ ] PR 20: Add slicing, path explanation, and structured evidence bundles.
- [ ] PR 21: Add external benchmark adapters and promotion gates for graph/flow/evidence claims.
- [ ] PR 22: Promote validated SDK query views and agent authoring ergonomics.

This roadmap records the completed research and the next implementation sequence needed to turn polint into a native, multi-language static-analysis engine that AI agents can write high-value repo-local rules and analysis extensions on top of.

The order is dependency-driven. Later topics should consume the facts, precision labels, and implementation lessons from earlier topics instead of reinventing their own substrate.

## Product Thesis: Agent-Extensible Static Analysis

Most traditional static-analysis tools were designed around a hard constraint:

> The analyzer must be a mostly black-box product that works across arbitrary codebases with little or no codebase-specific help.

That assumption drives a huge amount of classic design:

- auto-discover every endpoint;
- infer every framework convention;
- ship universal source/sink models;
- hide solver internals;
- keep the configuration surface small;
- make conservative guesses that are acceptable across many users;
- treat repo-specific knowledge as a last-mile configuration problem.

polint has a different operating model.

The primary user is not only a human clicking through a UI. The primary user is an AI coding agent such as Claude Code, Codex, and future agentic development systems that can read the repository, inspect conventions, run commands, update files, and generate repo-local code. Current agent tooling is explicitly built around reading codebases, editing files, running commands/tests, and integrating with developer tooling. polint should use that fact as a first-principles design input.

The product is therefore less like a sealed universal analyzer and more like:

```text
sane native analysis defaults
  + typed facts
  + explicit uncertainty
  + repo-local rules
  + repo-local analysis models
  + agent-authored extensions
  + validation/benchmark feedback
```

This is a fundamental shift. polint does not need to perfectly auto-discover every API endpoint, framework lifecycle, route, source, sink, sanitizer, or domain convention in every codebase. It needs enough default analysis to expose the right facts and uncertainty, then a powerful extension surface so an agent can add accurate repo-specific knowledge.

Examples:

- Instead of hardcoding every web framework route pattern, expose `Entrypoints<'_>` and let an agent add a repo-local model for `apps/api/router.ts`.
- Instead of pretending a generic source/sink model is complete, let an agent define the codebase's actual untrusted boundaries, privileged APIs, sanitizers, and guard functions.
- Instead of forcing one global call graph algorithm, let an agent choose or extend the relevant provider tier for the repo's language/framework style.
- Instead of making data flow a closed taint engine, let an agent add function summaries, additional flow steps, barriers, framework dispatch edges, and domain-specific return-side flows.
- Instead of reporting "unknown" as a dead end, make unknowns actionable: the agent can inspect unresolved calls and write a model that removes that uncertainty.

The research goal is therefore not just "find the best static-analysis algorithms." The goal is to design an analysis framework where algorithms have **agent-extensible lifecycle hooks** and every extension remains testable, cached, provenance-labeled, and honest about precision.

## Design Consequences

This thesis changes how we should research and implement every analysis family.

### 1. Extension Points Are Product Surface

Classic analyzers hide most modeling behind tool internals. polint should expose carefully designed extension points for:

- entrypoints and framework lifecycle;
- call resolution;
- source/sink/sanitizer/barrier models;
- additional data-flow steps;
- function summaries and effects;
- dependency/module boundaries;
- type/value/alias hints;
- generated-code conventions;
- domain-specific evidence formatting.

These extension points can be more sophisticated than a normal linter config because the main author may be an AI agent that can inspect code, write Rust rule/model code, run tests, and iterate.

### 2. Defaults Should Be Sane, Not Maximal

The default engine should be conservative and useful:

- parse files;
- emit symbols/references/imports;
- emit syntactic call sites;
- resolve high-confidence direct edges;
- emit local CFG/data-flow facts;
- mark unresolved or unsupported behavior explicitly.

It should not try to become a fully universal black-box SAST product by default. Expensive or risky inference should be opt-in, rule-requested, or agent-modeled.

### 3. Unknowns Are Integration Tasks

In old tools, unresolved calls or unknown framework behavior often become hidden false negatives. In polint, they should become visible facts:

```text
unresolved call: dynamic router dispatch
missing source model: request context wrapper
unknown sanitizer: project-specific validator
unsupported lifecycle: custom job scheduler
```

The agent can then decide whether to write a model, add a summary, or accept lower precision.

### 4. More Complex APIs Are Acceptable If They Are Typed And Testable

Many static-analysis products avoid complex extension APIs because humans will not maintain them. polint can support a higher-capability integration surface if it is:

- strongly typed;
- generated from templates;
- validated through fixtures;
- explained through diagnostics;
- versioned and cache-keyed;
- benchmarked for precision/cost impact;
- isolated from unstable internal implementation details.

The public SDK should stay ergonomic for simple rules, but the advanced model/extension SDK can be richer.

### 5. The Evaluation Target Changes

We should evaluate two modes:

1. **Default mode:** how good are the built-in facts without repo-specific help?
2. **Agent-extended mode:** how much precision/recall improves after an agent adds repo-local models/extensions?

This second mode is the product differentiator. Research should measure the delta from agent-authored integrations, not only generic algorithm quality.

## Context Sources For This Thesis

This roadmap is grounded in the local product definition and current agent tooling:

- `README.md`: polint is "AI-agent-native" and turns repo-specific engineering instructions into executable lint rules.
- `docs/INITIAL_PROMPT.md`: polint is a framework for custom, codebase-specific static-analysis rules in an AI-native development world, not a packaged universal ruleset.
- OpenAI Codex documentation: Codex is a coding agent that can read, modify, and run code in repositories, answer codebase questions, and work in cloud/local coding environments: <https://platform.openai.com/docs/codex>
- Anthropic Claude Code documentation: Claude Code is an agentic coding tool that lives in the terminal, can build features, debug issues, and navigate codebases: <https://docs.anthropic.com/en/docs/claude-code/overview>

These sources support the core assumption: the user of polint's advanced analysis surface can be an agent capable of inspecting the repository and generating repo-local integration code.

## Current Research Baseline

| Track | Status | Folder | What It Gives Us |
|---|---|---|---|
| R-1. Semantic Index | Deep research complete, implement vertical slice next | `docs/facts/symbols-and-references.md`, `crates/polint/src/sdk/facts.rs`, `research/semantic-index/` | Stable symbol/reference baseline exists today. Research now recommends deepening it into `ScopeFact`, `ImportFact`, `AliasFact`, `ResolutionFact`, stable keys, xref indexes, explicit unknowns, and validated extension-provided semantic facts. |
| R0. Call Graphs | Done, bootstrap integration revised | `research/call-graphs/` | Call-site/call-target fact model, algorithm tiers, unresolved-call model, repo-local call model layer, default-vs-extended evaluation, cost/accuracy tradeoffs across Go, TS/JS, Java, Python, and revised `analysis::calls` integration in `implementation/BOOTSTRAP-INTEGRATION.md`. |
| R1. Data Flow | Done, bootstrap integration revised | `research/data-flow/` | Data-flow fact model, local/sparse/summarized/global flow strategy, IFDS/IDE timing, source/sink/sanitizer/summary model layer, call-graph dependency, agent-era domain lessons, default-vs-extended evaluation, and revised `analysis::data_flow` integration in `implementation/BOOTSTRAP-INTEGRATION.md`. |
| R2. Agent Extension Surface | Done, implement first vertical slice | `research/agent-extension-surface/` | Recommended Rust-code extension lifecycle for agent-authored engine improvements: process-isolated extension crates, typed sinks, provenance, validation, extension-aware capability planning, default-vs-extended deltas. |
| R3. Analysis Kernel | Done, implement before call graph/data flow | `research/analysis-kernel/` | Hybrid internal kernel recommendation: deterministic provider DAG, typed fact layers, sidecar provenance, validation/merge gates, layer-specific cache keys, relation/fixpoint sub-engine for recursive analyses, explicit unknowns, and extension-aware capability support. |
| R4. Evaluation Harness | Done, implement before call graph/data flow | `research/evaluation-harness/` | External-benchmark-first evaluation strategy, suite adapters, canonical expected/observed schema, default-vs-extension deltas, graph/path/fact/diagnostic metrics, performance/cache baselines, and native fixtures for engine invariants. |
| R5. Framework, Lifecycle, And Entrypoint Modeling | Done, implement first fact-family vertical slice | `research/framework-entrypoints/` | Native framework boundary layer recommendation: `Entrypoints<'_>`, trust-boundary facts, framework dispatch overlays, explicit unknowns, Go and TS/JS first recognizers, MCP as a first-class boundary, repo-local Rust providers, validation fixtures, and default-vs-extension metrics. |
| R6. Module, Package, Dependency, And Repo Topology Graph | Done, implement before serious call graph/data-flow integration | `research/module-graph/` | Layered native topology model: workspace roots, packages/projects/source sets, declared requirements, lockfile/native/tool-reported resolved edges, import-to-package facts, build target overlays, repo topology, package-manager coverage, precision labels, cache keys, and extension merge rules. |
| R7. CFG And Control Dependence | Done, implement before type/value/alias and serious data-flow integration | `research/cfg-control-flow/` | Native CFG model: operation nodes, basic blocks, typed normal/abrupt/exceptional/cleanup edges, graph views, reachability, dominators, postdominators, control dependence, path evidence, extension overlays, Go/TS first implementation path, and differential validation plan. |
| R8. Type, Value, Points-To, And Alias Analysis | Done, implement type/value/place substrate before global call/data-flow precision | `research/type-alias-points-to/` | Native layered analysis plan: places/access paths, declared/inferred/narrowed type facts, abstract values/allocation tokens, local flow, summaries, bounded Andersen-style points-to, alias provider stack, precision/cost ladder, and agent-authored Rust extension sinks. |
| R9. Function Effects And Summaries | Done, implement summary kernel before serious global call/data-flow/alias precision | `research/effects-summaries/` | Summary kernel recommendation: typed summary domains, `SummaryKey`, precision/status/provenance, local summaries, SCC fixpoint/widening, extension summary validation, memory/effect/product lattices, TITO summaries, external effects, and SDK view path. |
| R10. Abstract Interpretation Domains | Done, implement P0 domain kernel before revisiting call/data-flow implementation details | `research/abstract-interpretation/` | Reduced-product abstract-domain kernel recommendation: deterministic solver, semantic operation layer, lattice/transfer traits, widening/narrowing policy, domain priority ladder, `Nilness<'_>`, `Constants<'_>`, `StringValues<'_>`, `Typestate<'_>` candidate views, extension validation, and benchmark gates. |
| R11. Implementation Bootstrap Rust Design | Done, use as first coding plan | `research/implementation-bootstrap/` | Private Rust `analysis` module recommendation, semantic store boundaries, stable IDs and metadata, MIR/place/direct-call/P0-domain/direct-summary sequence, semantic cache keys, extension sinks, local code review, and public SDK promotion gates. |
| R12. Program Slicing, Path Explanation, And Evidence | Done, implement after semantic bootstrap, CFG/control dependence, def-use, direct calls, and summaries exist | `research/program-slicing-evidence/` | Native evidence/slicing recommendation: typed evidence nodes and edges, thin/full slices, chops, context-matched paths, summary expansion, JSON/SARIF rendering, provenance, uncertainty, extension merge validation, and diagnostic evidence bundles. |
| R13. Incremental Query Engine And Caching | Done, implement minimal dependency-digest/layer-cache slice with semantic bootstrap, then full demand query engine before expensive global analyses | `research/incremental-query-engine/` | Native layered incrementality plan: input snapshots, layer/query/summary/diagnostic keys, shape digests, dependency indexes, invalidation planner, extension-aware cache validation/quarantine, summary SCC backdating, future watch/daemon red-green mode, and optional relation/differential backend. |
| R14. Rule SDK, Query Ergonomics, And AI-Agent Authoring | Done, implement rule manifests, fact/unknown inspection, and `polint test` with the bootstrap | `research/agent-rule-authoring/` | Typed Rust `#[polint::rule]` surface, macro-derived inspectable manifests, narrow `RuleCtx`, domain query builders, model pack versus provider extension boundary, fixture runner, model/provider delta tests, and agent inspect/explain/diff workflow. |
| R15. Local Semantic Store, Graph Queries, And Search Boundary | Done, implement before public graph-query CLI and before durable summary-store rollout | `research/local-semantic-store/` | SQLite/rusqlite-backed embedded semantic store, typed graph adjacency/evidence tables, content-addressed summary manifests, registry-ready seams, no remote registry now, Tantivy lexical search path, sqlite-vec experimental vector boundary, and store/query validation gates. |

These tracks are not implementation endpoints. They are inputs to the implementation roadmap below.

## Final Research Review

The research tracks converge on one implementation thesis: polint should not
start by building a public call graph, a public data-flow graph, or a generic
query language. The first product-quality asset is a private analysis kernel
that can produce typed, provenance-labeled facts with stable identities,
precision/status metadata, validation gates, cache keys, and evaluation results.
Every higher analysis family should consume that substrate instead of creating
its own graph, cache, ID space, extension format, or uncertainty model.

The most important findings across the corpus are:

- **The kernel is the product boundary.** `research/analysis-kernel/` shows that
  fact layers, provider manifests, scheduling, validation, merge policy, and
  provenance must be shared from the beginning. Otherwise call graphs, data
  flow, framework models, summaries, and extensions will fork into incompatible
  subsystems.
- **Semantic operations and places come before global analyses.**
  `research/implementation-bootstrap/`, `research/cfg-control-flow/`,
  `research/call-graphs/implementation/BOOTSTRAP-INTEGRATION.md`, and
  `research/data-flow/implementation/BOOTSTRAP-INTEGRATION.md` all reach the
  same conclusion: local MIR, normalized places, CFG, direct calls, and direct
  summaries are the safe cycle breaker. Full call graph and data-flow precision
  should be later consumers, not bootstrap prerequisites.
- **Summaries are the scaling boundary.** `research/effects-summaries/` and
  `research/incremental-query-engine/` both show that global call graph, data
  flow, alias, evidence, and extension queries become either too expensive or
  too local without typed summaries and summary-aware cache invalidation.
- **Agent extensibility changes the static-analysis design.**
  `research/agent-extension-surface/`, `research/framework-entrypoints/`, and
  `research/agent-rule-authoring/` show that polint can expose a more powerful
  integration surface than classic black-box analyzers because agents can
  inspect repos and write Rust models. That power must be balanced by typed
  sinks, validation, provenance, precision ceilings, fixtures, and
  default-vs-extended evaluation.
- **Unknowns are first-class facts.** The call graph, data-flow, framework,
  semantic-index, and type/alias research all warn against hiding dynamic or
  unsupported behavior. `Unresolved`, `Unsupported`, `SetupMissing`,
  `BudgetExceeded`, and `Unknown` states should be observable so agents can add
  models or summaries intentionally.
- **Evaluation must arrive before public precision claims.**
  `research/evaluation-harness/` argues for an external-benchmark-first
  strategy combined with native fixtures. Public `Calls<'_>`, `DataFlow<'_>`,
  `Effects<'_>`, and `Evidence<'_>` views should not be promoted until fixtures
  prove determinism, provenance, invalidation, extension behavior, and measured
  precision/recall.

The main implementation risk is skipping the foundation because the visible
features are more exciting. Building full call graphs or taint paths before the
kernel, evaluation harness, cache keys, MIR/place model, CFG, summaries, and
extension validation would almost certainly create a second analysis engine
that has to be rewritten. The roadmap below intentionally ships the boring
substrate first, but each PR still delivers a reviewable product increment.

## Implementation Roadmap: One PR Per Step

Each step below is intended to be one independently reviewable PR. A PR is
shippable only if it compiles, keeps existing user behavior working, includes
tests or fixtures for the new contract, and does not expose a broad public API
before the relevant facts are validated. Internal commands may be hidden or
preview-gated until the public CLI contract is intentionally accepted.

### Foundation PRs

| PR | Shipped Scope | Why This Comes Now | Acceptance Gate | Research References |
|---|---|---|---|---|
| 1. Private analysis kernel facade | Move current analysis orchestration behind an internal kernel boundary; add provider manifests for existing source, Go syntax, TS/JS syntax, module graph, symbol graph, and metrics providers; preserve current behavior. | Establishes ownership before adding new fact families. | Existing tests pass; runner delegates to kernel; provider order can be inspected in an internal/debug path; no new public SDK surface. | [`analysis-kernel/RECOMMENDED_IMPLEMENTATION.md`](analysis-kernel/RECOMMENDED_IMPLEMENTATION.md), [`implementation-bootstrap/RECOMMENDED_IMPLEMENTATION.md`](implementation-bootstrap/RECOMMENDED_IMPLEMENTATION.md) |
| 2. Provenance, precision, and validation metadata | Add internal `FactMeta`, `Producer`, `Precision`, `Confidence`, `ValidationStatus`, stable-key side tables, and validation/merge gates for current facts. | New facts need shared truth labels before call/data-flow/extensions can be trusted. | New kernel-produced facts have metadata; duplicate/conflicting stable keys fail deterministically; debug JSON can show provenance for files/imports/symbols/references. | [`analysis-kernel/FINAL-REPORT.md`](analysis-kernel/FINAL-REPORT.md), [`analysis-kernel/RECOMMENDED_IMPLEMENTATION.md`](analysis-kernel/RECOMMENDED_IMPLEMENTATION.md), [`semantic-index/FINAL-REPORT.md`](semantic-index/FINAL-REPORT.md) |
| 3. Internal evaluation harness MVP | Add hidden/internal evaluation model, deterministic expected/observed JSON, generic matchers, metrics, and a native fixture adapter for kernel/provenance/cache/extension invariants. | Prevents future precision claims from becoming anecdotal. | Fixtures can assert facts, graph edges, paths, diagnostics, invariants, runtime budgets; deterministic output hash excludes timestamps and machine paths. | [`evaluation-harness/FINAL-REPORT.md`](evaluation-harness/FINAL-REPORT.md), [`evaluation-harness/RECOMMENDED_IMPLEMENTATION.md`](evaluation-harness/RECOMMENDED_IMPLEMENTATION.md), [`evaluation-harness/VALIDATION.md`](evaluation-harness/VALIDATION.md) |
| 4. Input snapshots and cache-key vocabulary | Add `InputSnapshot`, `Digest`, `LayerKey`, `QueryKey`, `SummaryKey`, `DiagnosticKey`, provider output metadata, cache stats, and lifecycle/toolchain/rule/model digest plumbing. | Cache correctness depends on complete inputs before persistent reuse. | Snapshot tests cover file text, config, Go lifecycle inputs, TS/JS lifecycle inputs, rule digests, model digests, and official tool invocation digests where present. | [`incremental-query-engine/FINAL-REPORT.md`](incremental-query-engine/FINAL-REPORT.md), [`incremental-query-engine/RECOMMENDED_IMPLEMENTATION.md`](incremental-query-engine/RECOMMENDED_IMPLEMENTATION.md), [`module-graph/RECOMMENDED_IMPLEMENTATION.md`](module-graph/RECOMMENDED_IMPLEMENTATION.md) |
| 5. Persistent layer cache for existing cheap facts | Persist layer manifests/blobs for parse/syntax, imports, existing module facts, symbols/references, and metrics; add `DependencyIndex`, `ChangeSet`, and conservative invalidation. | Delivers real repeated-run value before expensive global analyses exist. | Syntax cache is not invalidated by unrelated rule edits; module/symbol layers invalidate on import/lifecycle/config changes; cache stats report hits/misses; stale reuse tests fail safely. | [`incremental-query-engine/RECOMMENDED_IMPLEMENTATION.md`](incremental-query-engine/RECOMMENDED_IMPLEMENTATION.md), [`analysis-kernel/RECOMMENDED_IMPLEMENTATION.md`](analysis-kernel/RECOMMENDED_IMPLEMENTATION.md), [`semantic-index/RECOMMENDED_IMPLEMENTATION.md`](semantic-index/RECOMMENDED_IMPLEMENTATION.md) |
| 6. Rule manifest, inspect, and test skeleton | Extend the rule macro metadata path; generate `RuleManifest`; add `polint inspect rule --format json`; add the first `polint test` fixture runner using temp repos and public SDK imports only. | Gives agents the authoring/debug loop needed before advanced facts arrive. | Temp-repo tests compile generated rules using `polint::sdk::prelude::*`; manifest includes derived fact views/capabilities/options; fixture runner asserts JSON diagnostics. | [`agent-rule-authoring/FINAL-REPORT.md`](agent-rule-authoring/FINAL-REPORT.md), [`agent-rule-authoring/RECOMMENDED_IMPLEMENTATION.md`](agent-rule-authoring/RECOMMENDED_IMPLEMENTATION.md), [`agent-rule-authoring/VALIDATION.md`](agent-rule-authoring/VALIDATION.md) |

### Semantic Backbone PRs

| PR | Shipped Scope | Why This Comes Now | Acceptance Gate | Research References |
|---|---|---|---|---|
| 7. Semantic index deepening | Add `ScopeFact`, richer `ImportFact`, `ResolutionFact`, alias/generated-symbol hooks, explicit unresolved references, stable export identities, and language-owned providers for Go and TS/JS. | MIR, calls, module topology, type facts, and rules need stronger name identity than the current baseline. | Fixtures cover resolved, ambiguous, unresolved, generated, alias, import/export, and cross-file references; unknowns are visible and precision-labeled. | [`semantic-index/FINAL-REPORT.md`](semantic-index/FINAL-REPORT.md), [`semantic-index/RECOMMENDED_IMPLEMENTATION.md`](semantic-index/RECOMMENDED_IMPLEMENTATION.md), [`semantic-index/VALIDATION.md`](semantic-index/VALIDATION.md) |
| 8. Layered module/package/topology graph | Add workspace roots, packages/projects/source sets, declared requirements, lockfile/tool-reported resolved edges, import-to-package facts, and repo topology overlays for Go and TS/JS first. | Package/source-set boundaries are needed for import resolution, architecture rules, entrypoints, cache digests, and cross-package call/data-flow. | Go monorepo module-root inference works; TS/JS package/workspace facts are deterministic; import-to-package facts distinguish source/test/generated/vendor/external where known. | [`module-graph/FINAL-REPORT.md`](module-graph/FINAL-REPORT.md), [`module-graph/RECOMMENDED_IMPLEMENTATION.md`](module-graph/RECOMMENDED_IMPLEMENTATION.md), [`module-graph/VALIDATION.md`](module-graph/VALIDATION.md) |
| 9. Private semantic MIR and place identity | Add `analysis::mir` and `analysis::places`; lower Go and TS/JS function bodies into a small owned operation set; assign `PlaceId` and stable place keys; emit explicit unsupported operations. | This is the real semantic bootstrap for CFG, calls, domains, summaries, and data flow. | MIR snapshots are deterministic; parser AST references do not escape lowering; places cover locals, parameters, globals, temporaries, fields/properties, indexes, call returns, and unknown roots. | [`implementation-bootstrap/FINAL-REPORT.md`](implementation-bootstrap/FINAL-REPORT.md), [`implementation-bootstrap/RECOMMENDED_IMPLEMENTATION.md`](implementation-bootstrap/RECOMMENDED_IMPLEMENTATION.md), [`abstract-interpretation/implementation/MIR-CONTRACT.md`](abstract-interpretation/implementation/MIR-CONTRACT.md) |
| 10. Local CFG and control dependence | Build CFG nodes/blocks/edges over MIR, including normal, abrupt, exceptional/cleanup where available; compute reachability, dominators, postdominators, and control dependence. | Data flow and evidence need path-sensitive local structure; rules need control-dependence without raw AST traversal. | Fixtures cover branches, loops, returns, short-circuiting, panics/throws, `defer`/`finally`-like behavior where supported, unreachable code, and unsupported constructs. | [`cfg-control-flow/FINAL-REPORT.md`](cfg-control-flow/FINAL-REPORT.md), [`cfg-control-flow/RECOMMENDED_IMPLEMENTATION.md`](cfg-control-flow/RECOMMENDED_IMPLEMENTATION.md), [`cfg-control-flow/VALIDATION.md`](cfg-control-flow/VALIDATION.md) |
| 11. Direct call facts | Add `CallSiteFact`, `CallTargetFact`, `UnresolvedCallFact`, direct/static resolution from semantic references, call indexes, and debug snapshots. Keep public `CallGraph<'_>` unsupported. | Direct calls break the cycle between calls, summaries, and data flow without needing whole-program inference. | Fixtures cover direct functions, methods, constructors, member calls, function values as unresolved/unknown, unsupported dynamic calls, and precise statuses. | [`call-graphs/FINAL-REPORT.md`](call-graphs/FINAL-REPORT.md), [`call-graphs/RECOMMENDED_IMPLEMENTATION.md`](call-graphs/RECOMMENDED_IMPLEMENTATION.md), [`call-graphs/implementation/BOOTSTRAP-INTEGRATION.md`](call-graphs/implementation/BOOTSTRAP-INTEGRATION.md) |

### Interprocedural Substrate PRs

| PR | Shipped Scope | Why This Comes Now | Acceptance Gate | Research References |
|---|---|---|---|---|
| 12. P0 abstract-domain kernel | Add lattice/transfer traits, product state, deterministic worklist solver, and first local domains: reachability, nullish/nilness, truthiness, constants, simple string facts, and initializedness where cheap. | These domains improve calls, summaries, data-flow precision, and future policy rules without whole-program cost. | Domain-law tests cover partial order/join/widening behavior; transfer monotonicity tests exist; fixtures expose top/unknown/budget states honestly. | [`abstract-interpretation/FINAL-REPORT.md`](abstract-interpretation/FINAL-REPORT.md), [`abstract-interpretation/RECOMMENDED_IMPLEMENTATION.md`](abstract-interpretation/RECOMMENDED_IMPLEMENTATION.md), [`abstract-interpretation/VALIDATION.md`](abstract-interpretation/VALIDATION.md) |
| 13. Summary kernel and direct summaries | Add `SummaryKey`, `SummaryStore`, typed summary domains, local/direct summaries for calls, control effects, return/TITO, memory-touch approximations, resource/external effects, and summary metadata. | Summaries are required before scalable interprocedural calls, data flow, alias, and evidence. | Direct summary snapshots are deterministic; summary status/precision/provenance is present; missing callees produce unknown/havoc summaries rather than silent certainty. | [`effects-summaries/FINAL-REPORT.md`](effects-summaries/FINAL-REPORT.md), [`effects-summaries/RECOMMENDED_IMPLEMENTATION.md`](effects-summaries/RECOMMENDED_IMPLEMENTATION.md), [`effects-summaries/VALIDATION.md`](effects-summaries/VALIDATION.md) |
| 14. Demand queries and summary SCC cache | Add the internal demand query layer for expensive views, summary SCC scheduling/cache, extension-aware cache quarantine, and query trace/debug output. | Expensive global providers should not become eager whole-repo work by default. | Body edits, public API edits, summary SCC edits, rule option edits, model edits, and extension code edits invalidate the correct layers; stale extension output is quarantined. | [`incremental-query-engine/RECOMMENDED_IMPLEMENTATION.md`](incremental-query-engine/RECOMMENDED_IMPLEMENTATION.md), [`effects-summaries/RECOMMENDED_IMPLEMENTATION.md`](effects-summaries/RECOMMENDED_IMPLEMENTATION.md), [`analysis-kernel/RECOMMENDED_IMPLEMENTATION.md`](analysis-kernel/RECOMMENDED_IMPLEMENTATION.md) |
| 15. Rust extension/provider sink | Add the first advanced extension boundary for repo-local Rust model/provider code, typed sinks, declared read sets, validation, precision ceilings, provenance, activation status, and fixture requirements. | Agent-authored engine improvements must be possible before framework/call/data-flow modeling grows large. | Invalid extension facts are rejected before merge; extension digests affect cache keys; default-vs-extended eval reports changed facts and unknown reduction. | [`agent-extension-surface/FINAL-REPORT.md`](agent-extension-surface/FINAL-REPORT.md), [`agent-extension-surface/RECOMMENDED_IMPLEMENTATION.md`](agent-extension-surface/RECOMMENDED_IMPLEMENTATION.md), [`agent-rule-authoring/RECOMMENDED_IMPLEMENTATION.md`](agent-rule-authoring/RECOMMENDED_IMPLEMENTATION.md) |
| 16. Framework entrypoints and trust boundaries | Add native facts for entrypoints, routes, handlers, callbacks, jobs, CLIs, MCP tools/resources/prompts, tests, generated dispatch, and trust boundaries; include Go and TS/JS default recognizers plus extension overlays. | Call graph and data flow are wrong if reachable roots and external inputs/outputs are wrong. | Fixtures cover HTTP, CLI/env/stdin, jobs/queues where modeled, test entrypoints, unresolved framework dispatch, and extension-improved discovery. | [`framework-entrypoints/FINAL-REPORT.md`](framework-entrypoints/FINAL-REPORT.md), [`framework-entrypoints/RECOMMENDED_IMPLEMENTATION.md`](framework-entrypoints/RECOMMENDED_IMPLEMENTATION.md), [`framework-entrypoints/VALIDATION.md`](framework-entrypoints/VALIDATION.md) |

### Precision PRs

| PR | Shipped Scope | Why This Comes Now | Acceptance Gate | Research References |
|---|---|---|---|---|
| 17. P0 type/value/place/alias substrate | Add declared/inferred/narrowed type facts, value/allocation facts, access-path facts, local narrowing, and an alias provider stack returning `NoAlias`, `MayAlias`, `MustAlias`, `PartialAlias`, or `Unknown`. Use official language tooling where it is the compatibility source, but normalize into polint-owned facts. | Refined calls and high-value data flow need type/value/place information, but not a mandatory whole-repo points-to graph. | Fixtures cover receiver narrowing, function values, object/property allocations, field sensitivity limits, unresolved aliases, and official-tool input digests when used. | [`type-alias-points-to/FINAL-REPORT.md`](type-alias-points-to/FINAL-REPORT.md), [`type-alias-points-to/RECOMMENDED_IMPLEMENTATION.md`](type-alias-points-to/RECOMMENDED_IMPLEMENTATION.md), [`type-alias-points-to/VALIDATION.md`](type-alias-points-to/VALIDATION.md) |
| 18. Refined call graph providers | Add opt-in refined call providers over direct calls, entrypoints, summaries, type/value facts, function tokens, receiver types, and bounded points-to constraints. Keep unresolved and budget-exceeded statuses explicit. | This is where the engine starts building real whole-repo call graphs without guessing as a default baseline. | Native fixtures and eval suites measure direct vs refined edges; precision/status is attached to every edge; dynamic dispatch and framework edges retain provenance. | [`call-graphs/FINAL-REPORT.md`](call-graphs/FINAL-REPORT.md), [`call-graphs/RECOMMENDED_IMPLEMENTATION.md`](call-graphs/RECOMMENDED_IMPLEMENTATION.md), [`call-graphs/VALIDATION.md`](call-graphs/VALIDATION.md) |
| 19. Local plus summary-projected data flow | Add `DataFlowNodeFact`, `DataFlowEdgeFact`, local value-flow graph, direct-call interprocedural edges, summary-projected edges, source/sink/sanitizer/barrier model sinks, budgets, unknown/havoc facts, and query-scoped path search. | Data flow becomes useful after CFG, calls, summaries, entrypoints, extension models, and alias/type facts exist. | Fixtures cover local flow, parameter/return flow, sanitizer/barrier behavior, missing summaries, extension-added flows, false-positive traps, and deterministic budget handling. | [`data-flow/FINAL-REPORT.md`](data-flow/FINAL-REPORT.md), [`data-flow/RECOMMENDED_IMPLEMENTATION.md`](data-flow/RECOMMENDED_IMPLEMENTATION.md), [`data-flow/implementation/BOOTSTRAP-INTEGRATION.md`](data-flow/implementation/BOOTSTRAP-INTEGRATION.md) |
| 20. Slicing, paths, and evidence bundles | Add internal evidence nodes/edges, thin/full slices, chops, ranked paths, summary expansion handles, provenance-rich diagnostic evidence, and JSON/SARIF evidence rendering. | Humans and agents need explainable findings, not just graph reachability. | Fixtures cover local dependence, interprocedural direct-call evidence, summary expansion, extension evidence, uncertainty markers, deterministic ranking, and compact path limits. | [`program-slicing-evidence/FINAL-REPORT.md`](program-slicing-evidence/FINAL-REPORT.md), [`program-slicing-evidence/RECOMMENDED_IMPLEMENTATION.md`](program-slicing-evidence/RECOMMENDED_IMPLEMENTATION.md), [`program-slicing-evidence/VALIDATION.md`](program-slicing-evidence/VALIDATION.md) |

### Promotion PRs

| PR | Shipped Scope | Why This Comes Now | Acceptance Gate | Research References |
|---|---|---|---|---|
| 21. External benchmark adapters and promotion gates | Add high-value external adapters in the order justified by the harness research: native fixtures first, then OWASP/SecBench/CodeQL/Pysa-style suites where supported. Record default-vs-extension deltas, runtime, memory, cache, and unknown metrics. | Public precision claims need independent evidence. | Reports show TP/FP/FN, precision/recall/F-score, unknown counts, graph/path metrics, runtime/memory, cache reuse, extension overhead, accepted/rejected extension facts. | [`evaluation-harness/FINAL-REPORT.md`](evaluation-harness/FINAL-REPORT.md), [`evaluation-harness/RECOMMENDED_IMPLEMENTATION.md`](evaluation-harness/RECOMMENDED_IMPLEMENTATION.md), [`data-flow/VALIDATION.md`](data-flow/VALIDATION.md), [`call-graphs/VALIDATION.md`](call-graphs/VALIDATION.md) |
| 22. Public SDK query views and agent ergonomics | Promote validated typed views such as `Calls<'_>`, `CallGraph<'_>`, `DataFlow<'_>`, `Effects<'_>`, `Evidence<'_>`, selected domain views, and bounded query builders. Expand `polint facts`, `polint unknowns`, `polint explain`, `polint diff`, and `polint eval` only where contracts are ready. | This is the moment where internal capability becomes agent-authorable product surface. | Temp-repo rule tests consume public SDK views only; docs explain limits/heuristics; public commands have stable JSON; expensive queries require limits or explicit unbounded mode. | [`agent-rule-authoring/FINAL-REPORT.md`](agent-rule-authoring/FINAL-REPORT.md), [`agent-rule-authoring/RECOMMENDED_IMPLEMENTATION.md`](agent-rule-authoring/RECOMMENDED_IMPLEMENTATION.md), [`ROADMAP.md`](ROADMAP.md) |

## Implementation Guardrails

- Keep new analysis modules private by default. Public promotion happens only in
  PR 22 or in a deliberately scoped earlier PR.
- Prefer official language tooling as compatibility input when it is the
  language-native source of truth, but normalize outputs into polint-owned facts
  with provenance and cache digests. Do not make random OSS analyzers runtime
  dependencies.
- Treat rule packs, model packs, and provider extensions as external consumers.
  They must use `polint::sdk::prelude::*` or explicit advanced extension
  surfaces, not internal modules.
- Every fact family must have stable IDs, precision/status/provenance,
  deterministic ordering, cache inputs, validation fixtures, and unknown states.
- Every extension-created fact must declare reads, pass validation, carry a
  precision ceiling, participate in cache keys, and be separable in default vs
  extended evaluation reports.
- Do not expose raw whole-program graphs as the primary SDK. Expose typed views
  and bounded query builders first; keep debug snapshots internal or preview
  gated.
- Do not block useful local rules on global precision. Ship local facts early,
  but label their limits honestly.

## Completed Research Order

### 1. Analysis Kernel: Fact Layers, Scheduling, Provenance, And Invalidation

**Folder:** `research/analysis-kernel/`

Status: researched in `research/analysis-kernel/`.

This was the highest-priority research topic before implementation. If polint is going to become the most capable static-analysis engine we can build, it must not grow as disconnected feature slices. Entrypoints, CFG, call graphs, data flow, effects, and rules should all plug into one shared analysis kernel.

The kernel is the substrate:

```text
typed fact families
  + fact layers
  + provenance/confidence/precision
  + extension-emitted facts
  + validation gates
  + dependency tracking
  + fixpoint scheduling
  + incremental invalidation
  + evidence paths
```

Research questions:

- What is the right internal model for fact families, fact layers, derived facts, extension-emitted facts, and native facts?
- How should polint schedule analyses that depend on each other without hardcoding one-off phase order for every feature?
- Should the engine use a demand-driven query model, an eager pipeline, a Datalog-like relation engine, Salsa-like incremental queries, or a hybrid?
- How should provenance, confidence, precision, and validation status attach to every fact without bloating rule ergonomics?
- How should extension facts merge with native facts: augment, override, suppress, replace, or conflict?
- How should cache keys and invalidation work when source code, config, rule code, extension code, SDK versions, and input facts change?
- What graph/storage shape supports cross-language facts without leaking AST/parser internals?
- How should unknowns, low-confidence facts, and unsupported behavior be represented as first-class facts?

Deliverables completed:

- Analysis kernel architecture recommendation.
- Fact layer model: native, derived, extension, synthetic, heuristic, validated.
- Scheduling model for native analyzers, extension providers, and rules.
- Provenance/confidence/precision schema shared by call graph, data flow, effects, and diagnostics.
- Cache and invalidation strategy for source files, rules, extensions, and derived facts.
- Extension merge/conflict policy.
- Minimal internal APIs needed for the first implementation without freezing a too-small public API.
- Decision on whether to research/use Salsa, Datalog/Souffle-like relations, petgraph-backed relations, or a custom query scheduler.

Core decision: use a hybrid internal kernel. Start with a deterministic provider DAG and typed fact layers, add sidecar provenance/validation/merge gates, split cache keys by layer, and add an internal relation/fixpoint sub-engine for recursive analyses. Copy Salsa/rust-analyzer invalidation concepts and Souffle/CodeQL relation concepts without adopting either as the first public or storage architecture.

### 2. Evaluation Harness And Ground Truth

**Folder:** `research/evaluation-harness/`

Status: researched in `research/evaluation-harness/`.

This must be implemented before or in parallel with the first serious call graph/data-flow implementation. If the goal is "most capable," every architecture choice must be measurable. The harness should measure both default analysis and agent-extended analysis.

Research questions:

- How should polint measure precision, recall, runtime, memory, unresolved facts, graph deltas, path quality, and false positives/false negatives?
- What benchmark suites, fixtures, or synthetic corpora should be imported or recreated for Go, TS/JS, Java, and Python?
- How should dynamic traces, tests, coverage, or framework runtime metadata be used as partial ground truth?
- How should we measure whether an agent-authored extension improved analysis instead of hiding uncertainty?
- What fixture format should assert facts, graph edges, data-flow paths, evidence paths, and diagnostics?
- How should evaluation represent default mode versus extension-enabled mode?
- How should benchmark results stay reproducible in CI and useful to agents?

Deliverables completed:

- Standard benchmark schema.
- Fixture taxonomy by language and analysis family.
- Metrics report template.
- Default-vs-agent-extended delta report.
- Ground-truth strategy for call edges, entrypoints, data-flow paths, and effects.
- Regression harness recommendation.
- Acceptance thresholds for introducing new fact families or public SDK views.
- Repository index for benchmark/code repositories inspected.
- Paper/source index for external benchmark and harness research.
- Pseudo-code for diagnostic/fact/graph/path scoring, tier scheduling, baselines, determinism, cache invalidation, and extension safety gates.

Core decision: use an external-benchmark-first harness, not an external-only harness. OWASP, SecBench.js, RealVuln, gosec, CodeQL tests, Pyre/Pysa, DroidBench, SecuriBench Micro, CryptoAPI-Bench, Juliet/SARD, Jelly, CrossCommitVuln-Bench, and SecCodeBench should supply independent outcome evidence where they fit. Native polint fixtures are still required for provenance, provider scheduling, cache invalidation, extension validation/merge behavior, stable fact keys, unknown facts, and typed SDK invariants.

### 3. Semantic Index: Symbols, References, Scopes, Imports

**Folder:** `research/semantic-index/`

Status: researched in `research/semantic-index/`.

The baseline symbol/reference layer has already been implemented. This deepening track is now complete enough to guide the next implementation slice because framework entrypoint recovery, module graph, call graphs, data flow, type inference, effects, and AI-authored extensions all depend on improving scopes, aliases, generated symbols, type-aware resolution, and explicit unresolved-resolution facts over time.

Research questions:

- How should polint model declarations, symbols, references, lexical scopes, imports, exports, aliases, fields, methods, packages, modules, and generated symbols?
- What precision tiers are needed for Go, TS/JS, Java, and Python?
- How do CodeQL, rust-analyzer, TypeScript, Go `x/tools`/gopls, Pyright, Ty, Pyrefly, JDT, Soot/SootUp/WALA, Semgrep, SCIP, LSIF, and Kythe store, query, and export symbol/reference facts?
- How should unresolved or ambiguous references be represented?

Deliverables completed:

- `SymbolFact`, `ReferenceFact`, `ScopeFact`, `ImportFact`, and `ResolutionFact` proposal.
- Language-specific resolution ladders.
- Accuracy/cost report for syntactic, binder, type-aware, and package-aware resolution.
- SDK shape for `Symbols<'_>`, `References<'_>`, `Scopes<'_>`, `Imports<'_>`.
- Extension-hook proposal for agent-supplied aliases, generated symbols, framework-specific references, and resolution overrides with provenance.

Core decision: implement semantic indexing as language-owned native Rust providers that emit normalized typed facts with stable identities, provenance, precision, explicit resolution status, and validated extension merges. Use compiler/LSP systems as semantic truth references, CodeQL-like fixpoints for recursive derived relations, and SCIP/Kythe-like identities for export only.

### 4. Agent Extension Surface And Model Lifecycle

**Folder:** `research/agent-extension-surface/`

This product-specific research track is now complete enough to guide implementation. It should remain active during implementation because it changes how all later algorithms expose hooks.

Research questions:

- What is the right split between simple repo-local rules and advanced repo-local analysis models?
- Should analysis extensions be Rust code, config/TOML, generated model files, declarative facts, or a combination?
- How can agents add call edges, entrypoints, source/sink models, summaries, data-flow steps, and framework semantics without corrupting engine invariants?
- How should extension facts be validated, tested, cache-keyed, versioned, and attributed?
- What guardrails prevent an agent-generated model from silently converting unknown behavior into false certainty?
- How do CodeQL model packs, Semgrep rules, Pysa models, TypeScript language service plugins, ESLint custom rules, rust-analyzer assists, and MCP-style tool integrations inform the design?

Deliverables:

- Extension API taxonomy: rule-only, model-only, provider-hook, generated facts, and debug-only extensions.
- Proposed repo layout for `.polint/models/` or equivalent.
- Validation lifecycle: generate -> bind to facts -> test fixtures -> benchmark delta -> activate.
- Capability model for extension-provided facts.
- Provenance and trust model for AI-authored extensions.

### 5. Framework, Lifecycle, And Entrypoint Modeling

**Folder:** `research/framework-entrypoints/`

Status: researched in `research/framework-entrypoints/`.

This is the first domain-specific research track after the kernel/evaluation work. Call graphs and data flow are often wrong because entrypoints are wrong. This is especially important for web apps, tests, CLIs, background jobs, MCP tools, serverless functions, decorators, annotations, and framework routers.

For polint, the target is not to auto-discover every framework pattern in the world. The target is:

```text
default recognizers for common patterns
  + visible unknowns
  + repo-local agent-authored framework models
```

Research questions:

- How should polint discover externally reachable functions?
- How should frameworks declare custom entrypoint/source/sink/route semantics?
- What should the agent extension format look like for a repo's actual routes, handlers, jobs, queues, tool definitions, tests, and generated dispatch?
- How do CodeQL model packs, Semgrep framework rules, FlowDroid lifecycle modeling, Pysa models, MCP-BiFlow, Spring analyzers, Go HTTP analyzers, and JS router analyzers approach this?
- How can AI agents author or refine framework models without making unsound claims?

Deliverables completed:

- `Entrypoints<'_>` and framework boundary fact proposal.
- Model format for routes, handlers, callbacks, decorators, lifecycle methods, MCP tools/resources/prompts, CLIs, jobs, tests, and generated dispatch.
- Trust-boundary model for HTTP, MCP, CLI/env/stdin, queues/jobs, request/response, and return-side agent-visible outputs.
- Native Rust implementation path for built-in providers plus repo-local agent-authored providers.
- Go and TS/JS first-scope recommendations, with Python and Java/JVM kept as future adapter research input.
- Validation plan for fact precision/recall, unknown reduction, extension delta, cache determinism, and provider cost.

Core decision: implement a native framework boundary layer before full call graph and data flow. Recover framework/protocol boundary facts with provenance; do not claim exact runtime behavior. Feed validated facts into call graph and data flow as optional synthetic dispatch and trust-boundary overlays.

### 6. Module, Package, Dependency, And Repo Topology Graph

**Folder:** `research/module-graph/`

Status: researched in `research/module-graph/`.

This should follow semantic indexing because imports and package roots are part of name resolution, but it deserves its own research track because repo-local policies often target architecture boundaries.

Research questions:

- How should polint represent Go modules/packages, TS/JS npm/workspace modules, Java packages/classpaths, Python packages/import paths, and monorepo workspace roots?
- How do we model dependency direction, layer boundaries, public/private APIs, test-only imports, generated code, vendored code, and external dependencies?
- How should lifecycle inputs participate in cache digests?

Deliverables completed:

- Layered fact model for `WorkspaceRootFact`, `PackageFact`, `SourceSetFact`, `DependencyRequirementFact`, `ResolvedDependencyFact`, `ImportToPackageFact`, and `RepoTopologyFact`.
- Language reports for Go, TS/JS, Python, Java/JVM, and Cargo.
- Package manager coverage matrix for Go modules, npm, pnpm, Yarn, Bun, pip, uv, Poetry, PDM, conda, Maven, Gradle, Bazel, Pants, and Cargo.
- OSS repository index with local source evidence from Go, gopls, TypeScript, Oxc resolver, npm Arborist, pnpm, Yarn Berry, Bun, uv, pip/resolvelib, Poetry, PDM, conda, Maven Resolver, Gradle, Bazel, Pants, Nx, Turborepo, and Cargo.
- Research paper/source index covering dependency solving, package-calculus models, lockfile design, and build-system theory.
- Native Rust implementation path with parsers, providers, validation, cache keys, extension merge rules, and SDK promotion timing.
- Benchmark and validation plan for roots, packages, declared edges, resolved edges, import ownership, source sets, unknowns, and extension deltas.

Core decision: build layered topology facts, not one universal dependency graph. Parse manifests and lockfiles natively first, implement Go MVS as the first full native resolver, support TS/JS/Python/JVM common managers through exact static facts and lockfile readers, and represent Gradle/Bazel/Pants dynamic build logic as conservative or tool-reported/extension-provided facts until exact native modeling exists.

### 7. CFG: Control Flow Graphs And Control Dependence

**Folder:** `research/cfg-control-flow/`

Status: researched in `research/cfg-control-flow/`.

Data-flow precision depends on CFG quality. This must happen before serious abstract interpretation and before high-confidence interprocedural data flow.

Research questions:

- How should polint represent basic blocks, statements, branches, loops, returns, panic/throw, exceptions, `defer`, `finally`, short-circuiting, async/await, generators, closures, and early exits?
- What is the smallest cross-language CFG that supports local data-flow without leaking unstable AST internals?
- How do Oxc, TypeScript, Go SSA, Rust MIR, Checker Framework, Soot/Jimple, WALA, Pyre, and CodeQL represent control flow?
- How should control dependence be exposed for diagnostics and rules?

Deliverables completed:

- `Cfg<'_>` SDK view proposal.
- Local CFG algorithms in Python-ish pseudocode.
- Precision limits per language.
- Fixture plan for branches, loops, exceptions, async, defer/finally.
- Extension points for framework-specific control transfers such as routers, schedulers, callbacks, generated handlers, and test harnesses.
- OSS repository index covering Go SSA/cfg, Oxc, TypeScript, ESLint, CodeQL, Pyright, Pyre, CPython, mypy, Soot, SootUp, WALA, Checker Framework, OPAL, LLVM/MLIR, Joern, Semgrep, TAJS, and Jelly.
- Research paper/source index covering control dependence, SSA/control-dependence construction, Java exception CFGs, Checker Framework dataflow, CodeQL/QL, TAJS, LLVM/MLIR, JLS/JVMS, and Python bytecode semantics.
- Native Rust implementation path for internal fact schema, shared builder, Go/TS providers, derived dominance/postdominance/control-dependence, extension overlays, cache keys, and SDK promotion.

Core decision: build layered native CFG facts, not one universal AST-walk graph. Represent operation nodes, basic blocks, typed normal/abrupt/exceptional/cleanup edges, and virtual exits. Compute reachability, dominators, postdominators, and control dependence as derived facts over explicit graph views. Keep call graph/framework dispatch separate from local CFG, and allow agent-authored extension overlays only through validated, provenance-labeled sinks.

### 8. Type, Value, Points-To, And Alias Analysis

**Folder:** `research/type-alias-points-to/`

Status: researched in `research/type-alias-points-to/`.

This research feeds call graphs and data flow. It should not be treated as a data-flow subchapter; alias/points-to precision determines whether method dispatch, heap flow, field flow, and summaries are useful.

Research questions:

- Which type facts can be extracted cheaply per language?
- How should possible values, allocation sites, object identities, function tokens, receiver types, and aliases be represented?
- Where do Andersen, Steensgaard, RTA/VTA, object sensitivity, field sensitivity, access paths, shape analysis, and type narrowing fit?
- How do TypeScript, Go `x/tools`, Pyright/Pyre, WALA, Doop, Soot Spark, OPAL, TAJS, Jelly, and PoTo handle the precision/cost frontier?

Deliverables completed:

- `Types<'_>`, `Values<'_>`, `Aliases<'_>`, and `PointsTo<'_>` research recommendation.
- Cost/accuracy table for type inference, value facts, points-to, alias, context sensitivity, and sparse refinement tiers.
- Native implementation path for place/access-path facts, type facts, value/allocation facts, local narrowing, summaries, bounded Andersen-style points-to, and alias provider queries.
- How points-to/value facts should feed call graph and data-flow providers without making call graphs/data flow submodules of alias analysis.
- Agent-supplied type/value/summary/points-to/alias fact model with strict provenance, validation, precision ceilings, cache keys, and conflict handling.
- OSS implementation reports for Ty/Ruff, Pyrefly, Pyright, Pyre/Pysa, mypy, pytype, TypeScript, Oxc, Flow, TAJS, Jelly, CodeQL, Go tools, Staticcheck, Doop, WALA, Soot/SootUp, OPAL, Checker Framework, LLVM, SVF, Rust borrowck/Polonius, rust-analyzer, Souffle, and Joern.
- Paper/source index covering Andersen/Steensgaard/Shapiro-Horwitz, SVF, Doop, TAJS, Flow, IFDS, abstract interpretation, PoTo, CodeQL, LLVM AliasAnalysis/MemorySSA, Ty, Pyrefly, Oxc, WALA, and current trend preprints.

Core decision: do not build a mandatory whole-repo alias graph first. Implement layered native facts: `PlaceFact`, `TypeFact`, `NarrowedTypeFact`, `ValueFact`, `AllocationTokenFact`, `SummaryFact`, requestable bounded points-to constraints, and an alias provider stack that returns `NoAlias`, `MayAlias`, `MustAlias`, `PartialAlias`, or `Unknown` with evidence. Use third-party analyzers such as Ty/Pyright/Pyrefly/CodeQL/WALA/Soot/SVF/LLVM as references and validation oracles, not runtime core dependencies. Official language tooling such as the Go toolchain, JVM/JDK metadata, `javac`, TypeScript compiler behavior, and official Python metadata may be used as provider inputs when that is the most accurate compatibility path, as long as outputs normalize into polint-owned facts with provenance/cache boundaries.

### 9. Function Effects And Summaries

**Folder:** `research/effects-summaries/`

Status: researched in `research/effects-summaries/`.

Effects are the bridge between local facts and practical repo rules. They also provide a compact substrate for interprocedural data flow and agent explanations.

Research questions:

- How should polint summarize reads, writes, mutations, calls, allocations, returns, throws, panics, logging, network, file system, process execution, environment access, database calls, and external tool calls?
- What is the relationship between effect summaries, data-flow summaries, and call graph edges?
- How can agents add or correct project-specific summaries for wrappers, adapters, generated clients, RPC layers, and framework APIs?
- How do Pysa, CodeQL summaries, CFTaint, Infer, Checker Framework, WALA slicing, and effect systems model this?

Deliverables completed:

- `Effects<'_>` SDK view proposal.
- Function summary schema.
- Query examples: "this handler writes to disk", "this function logs secrets", "this utility is pure", "this path can execute shell".
- Model format for agent-authored summaries and effect overrides, including benchmark-required proof before high-confidence activation.
- Algorithm analysis covering functional summaries, IFDS, IDE, WPDS, abstract-interpretation summaries, SCC fixpoints, widening, and demand summaries.
- Implementation reports for CodeQL, Pysa, Infer/Pulse/RacerD, LLVM/MLIR, Go tooling, WALA, Soot, OPAL, Heros, PhASAR, Joern, and Semgrep.
- Recommended native implementation path: internal summary kernel, typed domains, extension provider sink, validation gates, and typed SDK views.

Core decision: summaries are an internal typed product lattice plus kernel metadata, not one generic effect bag and not the first public rule primitive. Implement `SummaryKey`, `SummaryStore`, precision/status/provenance, local summaries, SCC closure, and typed domains such as `ControlEffects`, `CallEffects`, `DataFlowTito`, `MemoryEffects`, `AliasEscapeEffects`, `ResourceEffects`, `TaintEffects`, `ConcurrencyEffects`, and `ExternalEffects`. Let AI agents add Rust-code summary providers, but activate their summaries only after validation and precision downgrading/merge checks.

### 10. Abstract Interpretation Domains

**Folder:** `research/abstract-interpretation/`

Status: researched in `research/abstract-interpretation/`.

This is where polint gets beyond call/data-flow and into precise semantic rule facts.

Research questions:

- Which domains are most valuable first: nullness/nilness, constants, string values, numeric ranges, initializedness, resource state, typestate, permissions, path conditions, option/result state?
- How do Checker Framework, Infer, Rust MIR analyses, TypeScript narrowing, Pyre, Clang Static Analyzer, Goblint, and abstract interpretation literature balance precision and cost?
- What domains can be local-only, summary-based, or interprocedural?
- Which domains benefit from agent-supplied invariants or repo-specific typestate definitions?

Deliverables completed:

- Domain priority list.
- Generic lattice/transfer interface recommendation.
- `Nilness<'_>`, `Constants<'_>`, `StringValues<'_>`, `Typestate<'_>` candidate fact views.
- Safe model format for repo-specific invariants, guard functions, state transitions, and domain-specific validators.
- Algorithm analysis covering worklist fixpoints, reduced products, widening/narrowing, trace partitioning, abstract garbage collection/scoped forgetting, summary fixpoints, and extension validation.
- Tool implementation reports for Infer/Pulse, Clang Static Analyzer, rustc MIR dataflow, Checker Framework, TypeScript/Pyright/Flow, Pyre/mypy/Ty, Goblint, Eva plus contextual Astrée, Apron/ELINA/IKOS, CodeQL, Semgrep, TAJS, and Jelly.
- Language-specific domain strategy for Go, TS/JS, Python, JVM, and Rust-inspired kernel design.
- Benchmark and validation strategy for domain laws, transfer monotonicity, top-rate tracking, deterministic output, default-vs-agent-extended deltas, and external benchmark suites.

Core decision: abstract interpretation should be implemented as a native reduced-product domain kernel, not one monolithic value analysis and not a default whole-program symbolic executor. Start with reachability, nilness/nullish, truthiness, constants, string facts, initializedness, intervals, shapes, and typestate/resource domains. Add relational numeric precision through selected packed domains later. Let AI agents add repo-specific Rust guard, summary, invariant, and typestate products, but validate them through lattice-law tests, monotonicity checks, deterministic merge policies, provenance, cache keys, and suppressive-model review.

### 11. Program Slicing, Path Explanation, And Evidence

**Folder:** `research/program-slicing-evidence/`

Status: researched in `research/program-slicing-evidence/`.

AI agents and humans need explanations, not just true/false answers. Slicing
connects diagnostics to evidence. The completed research recommends building
`analysis::evidence` and `analysis::slicing` as internal query layers over the
semantic operation store, CFG/control dependence, def-use/data dependence, call
graph, summaries, data-flow, type/value/alias facts, and extension/model facts.

Research questions:

- How should polint compute backward slices, forward slices, chop queries, control dependence, data dependence, and call/data-flow path explanations?
- How do Joern, CodeQL path queries, WALA slicer, System Dependence Graphs, and modern SAST tools present evidence?
- How can slices stay small enough to be useful for AI agents?
- How should evidence distinguish engine-derived facts from agent-authored model facts?

Deliverables completed:

- Standard vocabulary for slices, paths, evidence nodes/edges, provenance, precision, and status.
- Paper index covering PDG, SDG/interprocedural slicing, thin slicing, slicing taxonomy, SliceFormer, SliceMate, and SliceT5.
- Repository index and source-code findings for WALA, CodeQL, Joern, Semgrep, Frama-C, and JavaSlicer.
- Algorithm analysis for PDG/SDG reachability, context-matched interprocedural traversal, chops, thin slices, ranked paths, summary expansion, and extension merges.
- Recommended implementation path for internal `EvidenceBundle`, `SliceQuery`, `PathQuery`, JSON/SARIF rendering, and future SDK views.
- Validation plan for local dependence, thin/full slices, source-to-sink paths, interprocedural context, extension evidence, determinism, cache invalidation, and external benchmarks.

Core decision: evidence is the user-facing form of the engine. Build structured
evidence bundles before exposing a public raw graph API. Default diagnostics
should show a small thin slice or ranked path, while preserving richer JSON/debug
detail, provenance, unknowns, and summary expansion handles. Do not build
executable slices first, do not materialize a whole-program SDG for every run,
and do not treat LLM-generated slices as trusted facts without validation.

### 12. Incremental Analysis, Query Engine, And Caching

**Folder:** `research/incremental-query-engine/`

Status: researched in `research/incremental-query-engine/`.

This should be implemented before expensive global analyses. Re-running full
call/data-flow on every agent edit will not scale, and stale caches are worse
than slow caches because polint's product depends on agent-authored Rust
extensions changing analysis facts safely.

Research questions:

- How should facts, summaries, queries, and diagnostics be invalidated after file edits?
- How should agent-authored models invalidate dependent call graph, data-flow, effect, and diagnostic results?
- How do Salsa, rust-analyzer, FlowLog, Souffle, IncIDFA, CodeQL databases, TypeScript incremental compiler, and Pyre incremental checking work?
- Which parts of polint should be demand-driven versus eagerly materialized?

Deliverables completed:

- Dependency graph strategy for inputs, layers, queries, summaries, diagnostics,
  extensions, official tool invocations, and model files.
- Cache key strategy for source text, shape digests, lifecycle inputs, rule
  options, provider/schema versions, extension code, validation status, and
  official tool invocations.
- Query scheduling model: eager cheap layers plus demand queries for expensive
  CFG, call graph, alias, summary, data-flow, and evidence views.
- Incremental benchmark plan for no-op warm runs, body edits, public API edits,
  lifecycle edits, rule option edits, extension/model edits, and summary SCC
  changes.
- Extension digest, declared read-set, validation digest, precision ceiling, and
  quarantine strategy.
- OSS implementation index for Salsa, rust-analyzer, gopls, TypeScript,
  Pyright, Pyrefly, Pyre/Pysa, Bazel Skyframe, Buck2 DICE, Souffle, and Ty.
- Paper index for Incremental CodeQL, IncIDFA, FlowLog, Adapton, Demanded
  Abstract Interpretation, incremental typing, Differential Dataflow, and
  Naiad.

Core decision: build a native layered incremental substrate first. Start with
`InputSnapshot`, `LayerKey`, `LayerCacheManifest`, `DependencyIndex`,
`ChangeSet`, `InvalidationPlan`, and cache stats. Add a demand query engine,
summary SCC cache, diagnostic cache, extension-aware quarantine, daemon
red-green mode, and relation/fixpoint backend only after the layer cache is
correct and benchmarks justify more machinery. Copy Salsa, Skyframe, DICE,
TypeScript, gopls, Pyright, Pyrefly, CodeQL, Souffle, and Differential Dataflow
ideas without adopting any one of them as the first core dependency.

### 13. Rule SDK, Query Ergonomics, And AI-Agent Authoring

**Folder:** `research/agent-rule-authoring/`

Status: researched in `research/agent-rule-authoring/`.

This happened after the core fact family, extension-surface, kernel,
incremental, and evidence research because the SDK shape is the product surface.
The research validates the existing typed-rule direction and adds the missing
authoring loop for AI agents.

Research questions:

- What typed fact views should agents use directly?
- How should rules express source/sink/sanitizer matchers, graph queries, summaries, evidence, and uncertainty budgets?
- How should generated rules declare required facts without manual capabilities?
- How does an agent decide whether to write a rule, a model, a summary, or a provider extension?
- How do CodeQL, Semgrep, Pysa models, Joern queries, ESLint, Rust Clippy, and custom policy engines make rules authorable?

Deliverables completed:

- Public SDK ergonomics report covering CodeQL, Semgrep, ESLint,
  typescript-eslint, Go `analysis`, Joern, Pysa, Ruff, Clippy, OpenRewrite, and
  current polint.
- Recommended typed Rust rule shape using only `polint::sdk::prelude::*`.
- Rule manifest plan: id, metadata, typed fact views, derived capabilities,
  precision, option schema, fixability, stability, SDK version, and limitations.
- Narrow `RuleCtx` plan: diagnostics, options, path/source metadata,
  setup/capability status, and future structured fixes; no broad fact access.
- Domain query builder strategy for imports, modules, symbols, references,
  calls, data flow, effects, summaries, and evidence.
- Decision tree for rule vs model vs summary vs provider extension vs fixture.
- `polint test` fixture runner plan with inline markers, JSON snapshots,
  before/after fix snapshots, model delta tests, and provider fact snapshots.
- Agent inspect/explain/diff/eval CLI plan.
- Paper index covering QLCoder, KNighter, IRIS, SemTaint, and RuleLLM.
- OSS repository index and subagent synthesis.

Core decision: keep typed Rust `#[polint::rule]` functions as the primary
public authoring surface. Do not build a CodeQL clone, Semgrep YAML clone,
Joern raw graph shell, or broad `RuleCtx` fact database first. Add generated
rule manifests, fact/unknown inspection, model packs, provider extension
boundaries, domain query builders, and a fixture-first `polint test` workflow so
agents can author executable artifacts with evidence.

## Cross-Cutting Research Standard

Every new research folder should follow the same structure:

```text
research/<topic>/
  README.md
  FINAL-REPORT.md
  RECOMMENDED_IMPLEMENTATION.md
  RESEARCH-ANALYSIS.md
  REPO-INDEX.md
  PAPER-INDEX.md
  STANDARD.md
  VALIDATION.md
  algorithms/
  implementation/
  languages/
  oss/
  papers/
  repos/        # gitignored
```

Each report should explicitly cover:

- algorithms;
- accuracy and failure modes;
- time and memory complexity;
- OSS implementations inspected;
- research papers read;
- per-language differences;
- what polint should copy;
- what polint should avoid;
- public SDK implications;
- agent extension implications;
- default mode vs agent-extended mode;
- benchmark and validation plan.

## High-Level Dependency Map

```text
Analysis Kernel
  -> Provenance/Validation Metadata
  -> Evaluation Harness
  -> InputSnapshot + Layer Cache
  -> Rule Manifest + Agent Test Harness
  -> Semantic Index Deepening
  -> Module Graph
  -> Semantic MIR + Place Identity
  -> CFG + Control Dependence
  -> Direct Call Facts
  -> P0 Local Abstract Domains
  -> Summary Kernel
  -> Demand Query + Summary Cache
  -> Rust Extension/Provider Sinks
  -> Framework/Entrypoint Modeling
  -> Type/Value/Alias Substrate
  -> Refined Call Graph Providers
  -> Data Flow
  -> Slicing/Evidence
  -> External Benchmark Adapters
  -> Public SDK Query Views

Incremental Query Engine research is complete. The first implementation should
include the minimal dependency-digest/layer-cache/invalidation slice before the
semantic bootstrap starts depending on new one-off caches. The full demand
query engine should land before expensive global call graph, data-flow, alias,
and evidence queries become normal rule dependencies.
Evaluation Harness runs across all tracks and gates public precision claims.
Framework/Entrypoint Modeling feeds Call Graphs, Data Flow, Effects, and Slicing.
Agent-authored models and extensions feed every analysis family, but must bind to typed facts and validation results.
```

## Research Lens For Every Track

When researching a topic, answer two different questions:

1. **What can the native engine infer by default across many repositories?**
2. **What can an AI agent accurately add after inspecting this specific repository?**

The second question is where polint can outperform older static-analysis products. We should intentionally design for a workflow where an agent:

```text
observes unresolved/low-confidence analysis facts
  -> inspects the repo-specific framework/convention
  -> writes a rule/model/extension
  -> runs fixtures and benchmarks
  -> activates the model with provenance and cache digests
  -> improves future analysis for humans and agents
```

That means research should prefer architectures with clear typed extension points over architectures that are theoretically elegant but closed to repo-specific augmentation.

## Recommended Next Implementation PR

Start with **PR 1: Private analysis kernel facade**.

```text
crates/polint/src/analysis_kernel/
```

Reason: the broad research stack is complete enough to build, but the first PR
should not start with MIR or call graph code. It should move the existing
analysis orchestration behind a private kernel boundary, add provider manifests
for the current source/Go/TS/module/symbol/metrics passes, and preserve current
behavior. That gives every later PR a single place to attach provenance,
validation, cache keys, scheduling, extension outputs, and evaluation stats.

The first PR should ship:

```text
AnalysisKernel
KernelInput
KernelOutput
ProviderManifest
ProviderId
ProviderKind
LanguageScope
CachePolicy
schema versions
provider-order tests
```

Do not expose a public graph/query API in this PR. The whole point is to create
the private boundary that later PRs can build on without forcing a rewrite.
