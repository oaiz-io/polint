# Roadmap: polint

## Milestones

- [x] **v1.0 MVP** - repo-local static analysis framework for Go and TypeScript/JavaScript, shipped 2026-05-02. Archive: [v1.0 roadmap](milestones/v1.0-ROADMAP.md).
- [x] **v1.1 Capability Fulfillment** - capability planning, resolved imports/module graph, and symbol/reference foundations for Go and TS/JS.
- [x] **v1.2 Static Analysis Engine Implementation** - private, validated, cache-aware, agent-extensible analysis engine substrate; 22 phases and 136 plans shipped 2026-05-27. Archive: [v1.2 roadmap](milestones/v1.2-ROADMAP.md).
- [x] **v1.3 Graph Engine Precision** - shared semantic graph, reachability/root semantics, Go RTA, JS/TS token/object models, adaptation, unknown taxonomy, budgets, and benchmark promotion gates. Archive: [v1.3 roadmap](milestones/v1.3-ROADMAP.md).
- [x] **v1.4 Policy Query Surface** - preview SDK views and typed query objects for realistic repo-local policies over calls, control flow, and data flow. Completed 2026-06-20. Archive: [v1.4 roadmap](milestones/v1.4-ROADMAP.md).
- [~] **v2.0 Static Analysis 2.0 Implementation** - re-scoped on 2026-11-02 after phases 63–65 shipped (benchmarks, store foundation, generation manifests). The surviving requirements are carried into v2.1 below; provider mirroring, whole-fact SQLite ingest and lexical search are cut. Archive: [v2.0 roadmap](milestones/v2.0-ROADMAP.md), [v2.0 requirements](milestones/v2.0-REQUIREMENTS.md).
- [ ] **v2.1 Engine Depth** - deep analysis in the inner loop, TypeScript/JavaScript parity with Go, an agent-queryable graph, and a thin rule SDK. 18 phases (72–89), one pull request each.

## Current Status

**Position on 2026-11-02 (`main` at `338b570b`, v0.5.0).** The engine substrate is complete and well-measured: typed Go call graph, routes and interprocedural data flow shipped in #133; the perf campaign took OAIZ full-repo warm from about 10 s to under 6 s with byte-identical output. Three gaps separate polint from "the most powerful static analysis engine":

1. **Deep analysis is not in the inner loop.** A one-file edit on OAIZ `core` costs 41.7 s for `calls` and 68.2 s for `dataflow`, because every deep cache key is whole-scan. Deep capabilities are therefore review/CI-only.
2. **TypeScript/JavaScript is a tier behind Go.** No flow programs, no typed call facts in `polint.calls`, no route models. The gated Jelly call-graph number is F1 0.79.
3. **Nothing deep is queryable by an agent without writing a rule.** No `polint graph`, and six known identity/outcome bugs (#86–#91) make persisted or published graph answers untrustworthy.

The research behind this roadmap is in `.planning/research/v2.1-engine/`. Every phase below is one pull request with a stated outcome, a measurable gate, and a kill criterion where the outcome is uncertain.

## Milestone outcome gates (v2.1)

Falsifiable, measured on the locked harness (OAIZ `core` and full repo at 4 jobs on the measurement host; the public corpora through `make bench-run`).

| Gate | Target | Measured by |
|---|---|---|
| **G1 Edit tier** | One-file edit on OAIZ `core`: `calls` ≤ 15 s, `dataflow` ≤ 20 s; warm no-change ≤ 6 s / ≤ 16 s; cold within +10% of v0.5.0 | the harness edit cell (Phase 74) |
| **G2 Inner loop** | Deep capabilities run in the default `check` profile when a rule asks for them, with the full-repo warm run ≤ 10 s and edit ≤ 20 s on OAIZ; otherwise they stay review/CI-only (kill criterion carried from the gate-2 decision) | Phase 78 promotion report |
| **G3 Byte identity** | Reports identical at 1/4/6 jobs and ten permutations; restored answers carry the same unknowns and budget state as recomputed ones; rules requesting no deep capability see byte-identical output to v0.5.0 | `scripts/deep-gate/gate.sh`, golden corpus, cli jobs/permutation tests |
| **G4 TS parity** | TS taint corpus ≥ 0.90 precision / ≥ 0.70 recall without Node and ≥ 0.80 recall with the type sidecar; Express/Fastify/Hono route fixtures 100% expected-row match; Jelly lane within 0.005 F1 of baseline | `polint-eval` taint-corpus and route harnesses |
| **G5 Agent graph** | `polint graph callers/callees/used-by/reach/path/taint` answer from the warm cache in ≤ 2 s on OAIZ `core`, with a versioned JSON envelope, the seven-value status vocabulary, and `budget_exceeded`/`setup_missing` never collapsed | schema snapshot + fixture matrix (Phases 79, 84) |
| **G6 Honesty** | No public answer (rule view or CLI) claims completeness after a budget trip, setup gap, or provider failure; capability support reflects sealed provider outcomes | Phases 73, 84 tests |
| **G7 Ice-cold build** | A rule pack's closure builds in ≤ 35 s / ≤ 0.6 GB on the 4-job rig (only if the Phase 85 spike is green; otherwise the gate is dropped with a recorded decision) | `polint-bench build-cost` |
| **G8 Regression budget** | Peak RSS ≤ +20% and cold wall-clock ≤ +25% versus the v0.5.0 baselines on every public corpus | `make bench-run`, `polint-eval` gate |

## Phases (v2.1), in merge order

Tracks: **K** kernel truth, **E** Go edit tier, **G** agent graph, **T** TypeScript parity, **S** thin SDK, **C** closeout. Phases are listed in the order they should merge. Track S (85–88) only depends on 72–73 and may run in parallel with E/G/T on a second track; everything else is sequential as listed.

- [ ] **Phase 72 (K): Identity and invalidation fixes** - #86 cross-file semantic ID collisions, #91 Go reachability roots in solver identity, #90 unconsumed inputs out of semantic-graph identity. *(Gates: G3, G5, G6)*
- [ ] **Phase 73 (K): Outcome truthfulness** - #87 no inferred provider success, #89 authenticated syntax-layer manifests, #88 effective capability support from sealed outcomes. *(Gates: G3, G6)*
- [ ] **Phase 74 (E): Sidecar instrumentation, VTA tuning, and the edit cell** - call-graph sub-phase timings and GC share, single CHA passed to VTA, reachable-function pruning, `probe.sh` edit cell. *(Gate: G1 measurable)*
- [ ] **Phase 75 (E): Per-package sidecar shards** - flow bodies carry site keys; static rows emitted and cached per package keyed on source + import export digests; one dynamic artifact for VTA edges and routes. *(Gates: G1, G3)*
- [ ] **Phase 76 (E): Per-unit summary store and invalidation frontier** - the v2.0 Phase 67 keystone, file-based: unit shards for MIR/CFG/call sites, `summaries/index.json` + blake3 payloads, frontier from the changed-file set, partial call-cache restore. *(Gates: G1 calls ≤ 15 s, G3, G8)*
- [ ] **Phase 77 (E): Incremental VTA and persisted taint summaries** - vendored VTA propagation with per-SCC persisted labels, `(function, entry)` summaries persisted with their unknowns, dependents-driven recompute. *(Gates: G1 dataflow ≤ 20 s, G3, G6)*
- [ ] **Phase 78 (E): Deep capabilities in the default check path** - re-run the gate-2 kill criteria on the new tiers and, if green, let `check` schedule `calls`/`routes`/`dataflow` when a rule requests them; otherwise record the decision. *(Gate: G2)*
- [ ] **Phase 79 (G): Graph envelope and `polint graph callers|callees|used-by|reach`** - in-memory query services over SDK views, `polint-graph-v1.json` schema, status vocabulary, leak-gate extension. *(Gates: G5, G6)*
- [ ] **Phase 80 (T): Language-neutral flow, route, and model contracts** - per-language flow programs, `RouteFact`, `[[ts_flow_*]]`/`[[ts_route]]` loaders, TS callee rows into `polint.calls`. No behavior change. *(Gates: G3, G4 prerequisite)*
- [ ] **Phase 81 (T): TypeScript flow program in Rust and the TS taint corpus** - Oxc-based lowering to the flow IR with closures, destructuring, await, classes and module globals; built-in TS models; ≥ 20-case corpus. *(Gate: G4 without Node)*
- [ ] **Phase 82 (T): Type-directed enrichment from the TypeScript sidecar** - `declared_type` rows, `param_types`/`classes`, union and implementation callees, type-keyed source matching. *(Gate: G4 with types)*
- [ ] **Phase 83 (T): TypeScript routes for Express, Fastify, and Hono** - models-as-data route interpreter over the TS flow program; `Routes<'_>` answers for TS. *(Gate: G4 routes)*
- [ ] **Phase 84 (G): `polint graph path|taint` over persisted summaries** - bounded path ranking, barriers, `budget_exceeded` from persisted unknowns, both languages. *(Gates: G5, G6)*
- [ ] **Phase 85 (S): Thin-SDK spike (E1 closure, E3 snapshot tax)** - throwaway measurement branch; merges only the decision record. *(Gate: G7 go/no-go)*
- [ ] **Phase 86 (S): SDK extraction** - `polint-sdk` + `polint-engine` + `polint` facade; packs compile unchanged through the package rename. *(Gate: G7 closure)*
- [ ] **Phase 87 (S): Fact snapshot and rule-process protocol** - `PSNAP1` snapshot, stdio frames, policy views as RPC, `sdk_abi` versioning. *(Gates: G3, G7)*
- [ ] **Phase 88 (S): Runner and CLI cutover** - host drives the protocol, zero `cargo` spawns on warm runs, `polint test` per-case snapshots, docs and GitHub Action updated. *(Gate: G7)*
- [ ] **Phase 89 (C): Milestone closeout** - Next.js/NestJS route models if time allows, determinism matrix, benchmark report against every gate, archive. *(Gates: all)*

## Phase Details

### Phase 72 (K): Identity and invalidation fixes
**Goal:** Semantic identities and solver cache identity are correct, so persisted and published graph answers can be trusted.
**Depends on:** nothing.
**Research:** `research/v2.1-engine/05-kernel-correctness-issues.md`.
**Requirements:** KRN-01, KRN-02, KRN-03.
**Scope:**
- Port `d2bb4f57` (offset-rebase of merged semantic ids and every parent/scope/export reference) to `analysis_neutral/symbol_graph/semantic.rs`; rotate the schema to `symbol-graph-facts-3`; two-file regression that fails without the rebase.
- Add `reachability` to the solver's hard dependencies and fold a sorted digest of the consumed Go root set into `solver_output_digest`.
- Then strip `identity`, `abstract_domains`, `entrypoints`, `reachability`, `module_topology` from `semantic_graph_output_digest` with a mutation test per removed input.
**Outcome:** issues #86, #90, #91 closed.
**Gate:** golden corpus and Jelly/Go x/tools lanes byte-identical; each new test fails with its fix reverted; warm layer reuse rate on OAIZ `core` unchanged or higher.

### Phase 73 (K): Outcome truthfulness
**Goal:** A provider that failed, skipped, or could not persist never looks like it succeeded, and rules see the capability support that actually holds.
**Depends on:** Phase 72.
**Research:** `research/v2.1-engine/05-kernel-correctness-issues.md`.
**Requirements:** KRN-04, KRN-05, KRN-06.
**Scope:**
- Remove the `"native_trusted"` digest fallback; adapters keep the pre-write digest and report write failure as telemetry; `ProviderOutcomeStatus` flows through dependency inputs.
- One canonical dependency-edge builder from layer key + snapshot; exact-set equality on read; TS syntax included; one cache-protocol bump.
- Effective `CapabilitySupportView` derived from sealed outcomes, returned in `KernelOutput`, used by `RuleCtx`, `explain` and `inspect` JSON.
**Outcome:** issues #87, #88, #89 closed.
**Gate:** `ts_syntax_layer_write_failure_*` asserts a real digest plus telemetry; a config-only change invalidates a syntax layer in a test; `polint explain` shows a revoked capability after an injected provider failure; golden corpus byte-identical.

### Phase 74 (E): Sidecar instrumentation, VTA tuning, and the edit cell
**Goal:** Know where the 15.8 s VTA floor comes from, take the cheap wins, and make the edit tier reproducible by the harness.
**Depends on:** nothing (may run in parallel with 72–73).
**Research:** `research/v2.1-engine/01-go-edit-tier-latency.md` §2(d).
**Requirements:** EDIT-01, EDIT-02.
**Scope:**
- Stage-log rows for the `call_graph` phase: CHA index, graph build, SCC, propagate, construct, GC CPU share.
- Compute CHA once with `cha.CallGraph` and pass it as VTA's `initial`; drop the duplicate `hierarchyCallees` index.
- Memory-limit experiment for the call-graph phase; reachable-function pruning before VTA behind a flag.
- `scripts/deep-gate/probe.sh` gains `EDIT=<patch>`: apply a one-file change, run warm against `KEEP_CACHE`, record calls/dataflow/warm/cold.
**Outcome:** VTA phase ≤ 8 s on OAIZ `core` (or a recorded finding that propagation itself is the floor, which moves Phase 77 ahead of 76); every later edit-tier number comes from the harness.
**Gate:** `call_edges` rows (140,175 on OAIZ `core`) and `polint unknowns --cap calls` byte-identical to v0.5.0; edit cell reproducible across three runs.

### Phase 75 (E): Per-package sidecar shards
**Goal:** The sidecar emits, and polint decodes, only what a one-file edit changed.
**Depends on:** Phase 74.
**Research:** `research/v2.1-engine/01-go-edit-tier-latency.md` §2(a).
**Requirements:** EDIT-03, EDIT-04.
**Scope:**
- Flow bodies carry the call-site key instead of VTA candidates; candidates are joined from `call_edges` on the Rust side.
- Static rows (packages, functions, call sites, type facts, method sets, params, flow bodies) emitted per package and cached keyed on (package source digest, import export-data digests, sidecar digest, lifecycle digest), replacing the whole-scan NDJSON key.
- One dynamic artifact (VTA edges, routes, budgets) keyed as today.
**Outcome:** on the edit cell, shards emitted and decoded = changed package + packages whose import interface changed.
**Gate:** instrumented shard count matches the expectation on three edit fixtures; all joined facts byte-identical to cold; sidecar Go tests, outside-user CLI tests, `go_types` and `routes` views unchanged.

### Phase 76 (E): Per-unit summary store and invalidation frontier
**Goal:** The Rust side recomputes only the units a change touched. This is the v2.0 Phase 67 keystone, delivered file-based.
**Depends on:** Phases 72, 73, 75.
**Research:** `research/v2.1-engine/03-semantic-store-rescoping.md` §3; `01-go-edit-tier-latency.md` §2(b).
**Requirements:** EDIT-05, EDIT-06, STORE-10, STORE-11, STORE-12.
**Scope:**
- Unit (Go package) shards for MIR/CFG/call sites/derived rows keyed on the unit's file digests + sidecar shard digest + provider version + config digest.
- `.polint/cache/summaries/index.json` (schema, polint version, sidecar identity, models digest, per-unit input digest, dependency list, payload id) and `payloads/<blake3>.json` written temp-then-rename; `SummaryPayloadDigest` is a distinct type from cache `Digest`.
- Frontier: changed files → units → reverse-dependency closure, expressed through the existing `InvalidationPlan` algebra with `CacheNode::Summary`.
- Partial call-cache restore in `CallCacheState::decide`; cross-unit edge merge by sort; recompute set instrumented in `KernelRunReport`; `polint cache status/prune` accounts for payloads.
**Outcome:** `calls` edit on OAIZ `core` ≤ 15 s at 4 jobs; warm no-change still ≤ 6 s.
**Gate:** must-recompute / must-reuse fixtures (file edit, dependency edit, config edit, models edit, sidecar upgrade, missing payload); reports byte-identical at 1/4/6 jobs and ten permutations; RSS within G8. **Kill:** if the edit tier cannot reach ≤ 20 s with Phase 74's VTA number, stop and reorder Phase 77 first.

### Phase 77 (E): Incremental VTA and persisted taint summaries
**Goal:** Data-flow answers after an edit are proportional to the edit, and restored answers are indistinguishable from recomputed ones.
**Depends on:** Phase 76.
**Research:** `research/v2.1-engine/01-go-edit-tier-latency.md` §2(d4), §2(b).
**Requirements:** EDIT-07, EDIT-08, STORE-13.
**Scope:**
- Vendor x/tools VTA propagation (BSD); stable node names (function + instruction ordinal, generic instances canonicalised); per-SCC persisted type sets keyed on SCC content digest + predecessor digests; re-propagate only SCCs reachable from changed nodes with equality backdating.
- Persist `(function, entry)` taint summaries with their unknowns and budget events in the unit payload; per-unit deterministic step budgets on the persisted path (no deadline-dependent truncation); dependents-driven invalidation through the solver's `dependents` set.
**Outcome:** `dataflow` edit on OAIZ `core` ≤ 20 s.
**Gate:** VTA edge set identical to whole-program VTA on OAIZ `core` and the taint corpus after every edit fixture; taint corpus precision/recall unchanged (0.946 / 1.000); `is_complete` and unknown counts byte-identical between restored and recomputed answers. **Kill:** if the incremental edge set diverges from whole-program VTA on any corpus, ship persisted summaries alone and record the VTA decision.

### Phase 78 (E): Deep capabilities in the default check path
**Goal:** Developers get typed calls, routes and data flow in `polint check`, not only in review and CI.
**Depends on:** Phase 77.
**Requirements:** EDIT-09, PROD-10.
**Scope:**
- Re-run the gate-2 criteria (warm no-change, one-file edit, cold, byte identity, route inventory, rule CPU) on the Phase 77 tip for OAIZ `core` and the full repo.
- If green: remove the review/CI-only restriction so `check` schedules deep providers when a rule requests them; update `docs/facts/{calls,call-graph,data-flow,routes}.md`, the generated skill text and `CONSUMER-SETUP.md`.
- If red: record the decision in `REQUIREMENTS.md` with the numbers and keep the restriction.
**Outcome:** a documented, measured decision; if green, G2 holds.
**Gate:** full-repo warm ≤ 10 s and edit ≤ 20 s on OAIZ with a deep-capable default profile; rules requesting no deep capability still byte-identical to v0.5.0.

### Phase 79 (G): Graph envelope and `polint graph callers|callees|used-by|reach`
**Goal:** An agent can ask the warm cache who calls what, what uses a symbol, and what is reachable, and get an honest, versioned answer.
**Depends on:** Phases 72, 73, 76.
**Research:** `research/v2.1-engine/03-semantic-store-rescoping.md` §4.
**Requirements:** GRAPH-01, GRAPH-02, GRAPH-03, PROD-11.
**Scope:**
- In-memory query services over `CallGraph<'_>`, `References<'_>`, `Symbols<'_>`; flags `--max-depth`, `--min-precision`, `--include-tests`, `--path <glob>`, `--format json|text`, `--no-cache`.
- Envelope `version`, `schema`, `tool`, `command`, `query`, `status`, `precision`, `nodes`, `edges`, `paths`, `unknowns`, `budgets`, `summary`; status ∈ `complete|partial|not_found|unknown|budget_exceeded|unsupported|setup_missing`; `not_found` requires sufficient evidence.
- Results carry stable semantic ids, repo-relative paths, spans, precision, provenance and evidence ids; never store row ids, provider ids or SQL names.
- `docs/schemas/polint-graph-v1.json`, `docs/GRAPH.md`, `public_surface_leak` extension, generated skill text.
**Outcome:** four commands usable by agents today, Go typed and TS syntactic tiers.
**Gate:** fixture matrix (cross-file, cross-package, cycles, setup gap, unknown-preserving no-result, budget trip); schema snapshot test; answers ≤ 2 s on warm OAIZ `core`; leak gate green.

### Phase 80 (T): Language-neutral flow, route, and model contracts
**Goal:** Nothing in the solver, SDK or model loaders assumes Go, so TS can plug into the same pipeline.
**Depends on:** Phase 75.
**Research:** `research/v2.1-engine/02-ts-js-deep-parity.md` §4.
**Requirements:** TSP-01, TSP-02.
**Scope:**
- Per-language flow program accessor and `FlowFunction.language`; `FlowUnknown::NoProgram` text no longer names the Go frontend.
- `RouteFact` with a `language` field replaces `GoSemanticRouteFact` in the SDK; store/validate paths follow.
- Model loader accepts `[[ts_flow_*]]` and `[[ts_route]]` tables; TS qualified-name scheme documented; hard-coded Go builtins moved to the Go model table.
- TS sidecar callee rows fed to `polint.calls` as `TypedCallInputs::new(Language::TypeScript)`.
**Outcome:** no behavior change; the contracts are ready.
**Gate:** Go taint corpus unchanged; golden corpus byte-identical; Jelly lane within 0.005 F1; `public_surface_leak` green with the renamed types.

### Phase 81 (T): TypeScript flow program in Rust and the TS taint corpus
**Goal:** `DataFlow<'_>::flows` answers for TypeScript and JavaScript without any external toolchain.
**Depends on:** Phase 80.
**Research:** `research/v2.1-engine/02-ts-js-deep-parity.md` §2(b), §2 mapping table.
**Requirements:** TSP-03, TSP-04.
**Scope:**
- Oxc-based lowering to the flow IR: slots, captured cells, closures (`Step::Capture`), destructuring, spread, optional chaining, `await`/`then` desugaring, classes and constructors, `this`, module globals; `unit` = nearest tsconfig project or package dir.
- Callees from the symbol graph and the Jelly heap; unresolved → `Algorithm::Unknown`.
- Built-in `ts_flow_models.toml`: Express request sources, SQL sinks (`pg`, `mysql2`, `knex`), `child_process`, logging sinks, common sanitizers.
- `tests/taint-corpus-ts` with ≥ 20 cases and twins mirroring the Go categories, run through the existing harness.
**Outcome:** TS repos no longer get `NoProgram`.
**Gate:** TS corpus precision ≥ 0.90 / recall ≥ 0.70 without Node; Go corpus unchanged; determinism gate green.

### Phase 82 (T): Type-directed enrichment from the TypeScript sidecar
**Goal:** Types raise TS callee precision and allow type-keyed source and sink models.
**Depends on:** Phase 81.
**Requirements:** TSP-05.
**Scope:**
- `polint-ts-types-2`: `declared_type` rows for parameters and receivers; join by `(file, start, end)` into `param_types`/`classes`; union-member and implementation callees into flow callees.
- Type-keyed matching in models (`parameter_type = "express#Request"`).
**Outcome:** better recall with Node present, identical results without it.
**Gate:** TS corpus recall ≥ 0.80 at precision ≥ 0.90 with the sidecar; byte-identical to Phase 81 without Node; cold sidecar ≤ +20% over 13.5 s on the 24k-line reference repo; warm unchanged.

### Phase 83 (T): TypeScript routes for Express, Fastify, and Hono
**Goal:** `Routes<'_>` inventories TS HTTP endpoints with handlers and middleware chains.
**Depends on:** Phase 81.
**Research:** `research/v2.1-engine/02-ts-js-deep-parity.md` §3.
**Requirements:** TSP-06.
**Scope:**
- Route interpreter in Rust over the TS flow program with the Go role set plus `path_optional`, `options_argument`; `[[ts_route]]` models; built-in Express, Fastify and Hono tables.
- `tests/eval-fixtures/routes-ts/` with expected rows; one real Express app checked against its documented endpoints.
**Outcome:** TS route inventory with completeness flags and unknowns.
**Gate:** 100% expected-row match on fixtures; two-run byte identity; unmodelled frameworks visible in `polint unknowns`.

### Phase 84 (G): `polint graph path|taint` over persisted summaries
**Goal:** Agents can ask whether data from a source reaches a sink and why a path exists, in either language, from the warm cache.
**Depends on:** Phases 77, 79, 81.
**Requirements:** GRAPH-04, GRAPH-05.
**Scope:**
- `path` with bounded ranking and cycle handling; `taint` with barriers, model selection and `--max-paths`; both read persisted summaries and report `budget_exceeded` from persisted unknowns without recomputation.
- Evidence ids link to SARIF code flows produced by `Flow::diagnostic`.
**Outcome:** the full G5 command set.
**Gate:** taint corpus parity with `DataFlow::flows` (Go 53/54, TS corpus); `budget_exceeded` surfaced from a persisted budget trip without re-running the solver; answers ≤ 2 s warm.

### Phase 85 (S): Thin-SDK spike
**Goal:** Decide, with numbers, whether the prebuilt-engine architecture is worth 6–9 weeks.
**Depends on:** Phases 72, 73 (may run in parallel with E/G/T).
**Research:** `research/v2.1-engine/04-thin-sdk-prebuilt-engine.md` §3.
**Requirements:** SDK-01.
**Scope:**
- E1: throwaway `polint-sdk` crate from `sdk/`, `core/{rule,db,capability,labels,metadata}.rs`, `policy_queries.rs`, `rule_manifest.rs`, `rule_error.rs`, `runner/`; a copy of the OAIZ pack pointed at it through `package = "polint-sdk"`; `cargo --timings` at 4 and 2 jobs.
- E3: serialize what the OAIZ rules read at rule start with serde_json and one binary codec; also measure "rule process reads validated layer blobs".
- E2 probe: how much of `core/db.rs` the read model needs.
**Outcome:** a decision record merged to `.planning/research/v2.1-engine/04-*`; no product code.
**Gate (go):** E1 ≤ 35 s and ≤ 0.6 GB with rule sources unchanged; E3 ≤ 5% of OAIZ `core` warm. **Kill:** E1 > 52 s or > 1 GB or source changes needed; E3 > 15% of warm; E2 needs > 20% of `analysis_kernel`. If killed, Phases 86–88 are dropped and G7 is removed with a recorded decision.

### Phase 86 (S): SDK extraction
**Goal:** Rule packs compile against a small SDK crate while `polint::sdk::prelude::*`, `#[polint::rule]` and `polint::runner::run_cli` keep their paths.
**Depends on:** Phase 85 green.
**Requirements:** SDK-02.
**Scope:** `polint-sdk`, `polint-engine`, `polint` facade; `core/db.rs` split into read model and engine; `OnceLock` indexes rebuildable SDK-side; `internal_architecture` test updated to the three-package contract; `ARCHITECTURE.md` and `API-VISIBILITY-PLAN.md` updated.
**Outcome:** packs build unchanged through the package rename.
**Gate:** closure ≤ 35 s; goldens, determinism and `public_surface_leak` unchanged.

### Phase 87 (S): Fact snapshot and rule-process protocol
**Goal:** The host computes facts once and hands rules a snapshot; policy views answer over RPC.
**Depends on:** Phase 86.
**Requirements:** SDK-03, SDK-04.
**Scope:** `PSNAP1` container with per-family sections and `content_digest`; `<cache>/snapshots/`; length-prefixed stdio frames (Hello, RunRequest, PolicyQuery/PolicyResult, RunResponse); `RuleOptionsWire`; `sdk_abi` with hard-exit mismatch codes; family audit for all 26 macro-mapped views including `Routes`, `GoTypes`, `CallGraph`, `Cfg`, `TsComponents`.
**Outcome:** a rule process that never links the engine.
**Gate:** byte-identical to a pre-L1 oracle across examples and temp-repo tests, 20× reversed-order determinism; snapshot tax within the E3 budget.

### Phase 88 (S): Runner and CLI cutover
**Goal:** `polint check` runs rules through the protocol with no `cargo` on warm runs, and users migrate with one documented step.
**Depends on:** Phase 87.
**Requirements:** SDK-05, SDK-06.
**Scope:** `run_local_rule_host_kind` drives the protocol; fingerprint keyed on `sdk_abi`; `polint test` snapshot per case, `--unit` unchanged; rule process cwd stays the repo root; old path deleted; migration test for unmigrated manifests; `CONSUMER-SETUP.md`, `CACHE.md`, `GITHUB-ACTION.md` updated.
**Outcome:** ice-cold rule-pack cost drops from ~200 s to the closure number.
**Gate:** zero `cargo` spawns on warm runs; cold ≤ 20 s on the 2-vCPU rig; offline run green; fixture suite green.

### Phase 89 (C): Milestone closeout
**Goal:** Prove every gate and leave the next milestone a recorded baseline.
**Depends on:** all.
**Requirements:** VAL-10, VAL-11, TSP-07 (optional).
**Scope:** Next.js file routes and NestJS decorators if time allows; determinism matrix (jobs × permutations × cold/warm/edit) on every corpus; `make bench-run SCALE=1` report with G1–G8 status; archive to `milestones/`; close #49 and stale PRs (#115, #126).
**Outcome:** v2.1 shipped with a benchmark report.
**Gate:** every G1–G8 row has a measured value and a pass/fail/dropped status.

## Requirement Coverage

| Phase | Requirements |
|-------|--------------|
| 72 | KRN-01, KRN-02, KRN-03 |
| 73 | KRN-04, KRN-05, KRN-06 |
| 74 | EDIT-01, EDIT-02 |
| 75 | EDIT-03, EDIT-04 |
| 76 | EDIT-05, EDIT-06, STORE-10, STORE-11, STORE-12 |
| 77 | EDIT-07, EDIT-08, STORE-13 |
| 78 | EDIT-09, PROD-10 |
| 79 | GRAPH-01, GRAPH-02, GRAPH-03, PROD-11 |
| 80 | TSP-01, TSP-02 |
| 81 | TSP-03, TSP-04 |
| 82 | TSP-05 |
| 83 | TSP-06 |
| 84 | GRAPH-04, GRAPH-05 |
| 85 | SDK-01 |
| 86 | SDK-02 |
| 87 | SDK-03, SDK-04 |
| 88 | SDK-05, SDK-06 |
| 89 | VAL-10, VAL-11, TSP-07 |

41/41 v2.1 requirements mapped; no orphans, no double-mapping. The v2.0 requirements carried into v2.1 are listed in `REQUIREMENTS.md` under "Carried forward from v2.0".

---
*Roadmap rewritten: 2026-11-02 from the v2.1 engine research (`.planning/research/v2.1-engine/`)*
