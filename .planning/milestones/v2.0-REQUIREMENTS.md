# Requirements: polint v2.0 Static Analysis 2.0 Implementation

**Defined:** 2026-07-07
**Core Value:** Make it easy to express a repo-specific engineering policy as a small rule and run it locally, in CI, and with AI coding agents.

**Milestone goal:** Build the Static Analysis 2.0 implementation foundation that turns polint's private analysis engine into a durable, queryable local semantic layer for custom Rust rules, agentic review, and future local graph exploration.

**Source context:** v1.2 built the private analysis engine substrate. v1.3 and v1.4 promoted graph precision and policy-query ergonomics. The locked Static Analysis 2.0 research says the next implementation step is a private local semantic store, durable summaries, invalidation, bounded queryability, and search foundations. `polint check` and `polint review` remain the core products. A remote registry of pre-computed package summaries is explicitly deferred.

## Product and Architecture Contract

Static Analysis 2.0 is not a replacement for repo-local Rust rules. It is the layer that helps AI agents and developers understand, explore, and enforce code and architecture quality in large codebases.

The long-term promise is that polint becomes a local semantic layer for AI-assisted engineering: an agent can ask what uses an API, why a path exists, whether user-controlled data can reach a sink, which unknowns block certainty, and then turn recurring findings into repo-local Rust rules enforced by `polint check` and `polint review`.

The v2.0 implementation must therefore optimize for:

1. Durable, validated local analysis state.
2. Honest graph and summary query results with precision, provenance, unknowns, and budgets.
3. Fast warm `polint review` through summary reuse and invalidation.
4. Registry-ready package summary manifests without a remote registry.
5. A future CLI query surface over the program graph without exposing raw internal storage or solver APIs.

## Milestone Outcome Gates

The locked Static Analysis 2.0 research defines falsifiable goals: scale (memory proportional to working set, not repo), latency (warm runs proportional to change), accuracy (measured real-app callgraph F1), and honesty. v2.0 is the storage-and-reuse slice of that program. It must measurably move scale and latency, not only land infrastructure. Every roadmap phase must name which outcome gate it advances; a phase that advances none is plumbing and should be folded into one that does.

1. **Scale gate.** Peak RSS on the large-monorepo benchmark stays proportional to the analyzed working set. Store ingest must not resurrect the eager whole-repo pipeline or whole-repo source loading that previously caused 30GB+ OOM (fixed via capability gating and rule-scoped discovery; current baseline ~1GB peak on the reference monorepo). Initial regression budget until warm reuse lands: at most +20% peak RSS and +25% cold wall-clock versus the store-disabled baseline; budgets are revisable only with a recorded decision.
2. **Latency gate.** By milestone end, warm `polint review` on a small diff re-analyzes only the invalidation frontier (changed functions/SCCs plus transitive summary dependents), with the recompute set instrumented and a p50/p95 warm-latency target set from the Phase 0 baseline and then enforced.
3. **Honesty gate.** Unknown, partial, setup-missing, unsupported, and budget-exceeded states remain visible end to end through store, query, review, and CLI surfaces — now durably persisted, never collapsed.
4. **Accuracy visibility gate.** v2.0 does not have to raise callgraph F1 (that is the type-directed tier workstream in the locked research), but it must measure and surface recall/precision of the persisted graph on real-repo benchmarks so `polint graph` answers are never overtrusted and the next milestone's accuracy work starts from a recorded baseline.

## Locked Technology Decisions

- Use `rusqlite` with bundled SQLite as the primary embedded local semantic store.
- Keep the store private under the analysis kernel; `rusqlite` types, SQL, tables, row IDs, provider generation IDs, and parser/solver internals must not become public contracts.
- Add `blake3` for content-addressed summary and payload identities where strong content IDs are needed. Do not silently replace existing cache keys.
- Add Tantivy only in the lexical-search phase, after stable semantic document IDs exist.
- Keep vector search deferred behind an unstable/experimental boundary requiring model, chunker, dimension, metric, normalization, provenance, and content-digest lockfiles.
- Do not build a remote package-summary registry in v2.0. Build local registry-ready seams only.
- Do not add public SQL, Cypher, Datalog, QL, SPARQL, a generic graph shell, or public raw graph SDK views.

## Locked Milestone Decisions

Approved 2026-07-09. Changing any of these requires a recorded decision, not a silent edit.

- **Regression budgets:** at most +20% peak RSS and +25% cold wall-clock versus the store-disabled baseline until warm reuse lands (Phase 67); after that, warm `review` must beat the baseline on the frontier benchmark.
- **Benchmark suite:** pinned-commit manifest with `grafana/grafana` as the primary large polyglot (Go+TS) scale target, `gohugoio/hugo` (Go medium), `excalidraw/excalidraw` (TS medium), the existing Jelly and Go x/tools oracle suites for micro fixtures and the recall/precision baseline, and the private devloupe monorepo as a local-only, non-CI reference point (~1GB peak RSS, cold 7.4s / warm 4.6s known baseline).
- **Search scope-cut:** Phase 70 (Tantivy lexical search) is the designated cut if the milestone runs long. Cutting it moves SEARCH-01..05 to v2.1 unchanged; no other phase depends on it.

## v2.0 Requirements

Requirements for the v2.0 milestone. Each requirement should map to exactly one roadmap phase after roadmap generation.

### Product Boundaries

- [x] **PROD-01**: `polint check` remains the repo-local Rust policy command. Store creation, refresh, corruption, or invalid schema must not silently change diagnostics, output formats, exit semantics, or public rule behavior.
- [ ] **PROD-02**: `polint review` remains the diff-focused agentic review workflow and should be the first workflow to benefit from warm summary reuse and invalidation-frontier recomputation.
- [ ] **PROD-03**: `polint graph` is an exploratory local understanding surface for users and agents. It is not a CI pass/fail interface and must not become a second rule system.
- [ ] **PROD-04**: Recurring graph findings become enforcement by writing repo-local Rust rules consumed by `polint check` and `polint review`, not by making graph queries into hidden policies.
- [ ] **PROD-05**: Public docs, generated skill text, examples, SDK exports, and CLI output must describe Static Analysis 2.0 as durable local analysis infrastructure, not as a remote registry product or bundled production ruleset.

### Ground Truth and Performance Baseline

- [x] **BENCH-01**: Extend the existing benchmark/promotion-gate harness with a real-repo suite (at least one production-scale monorepo plus representative OSS repos per language) that records peak RSS, cold/warm wall-clock, cache/store size, and budget-exhaustion telemetry as curves versus repo size and diff size. This lands before any store phase is considered complete, so the problem stays visible and gateable.
- [x] **BENCH-02**: Record the store-disabled baseline for `polint check` and `polint review` (peak RSS, cold/warm latency, diagnostics parity) before persistence is enabled by default. All later phases report deltas against this baseline.
- [x] **BENCH-03**: Every store phase runs the scale/latency regression gates from the Milestone Outcome Gates section. Until warm reuse lands, the gate is the regression budget over the store-disabled baseline; after warm reuse lands, warm `review` must beat the baseline on the frontier benchmark.
- [x] **BENCH-04**: Persisted-graph recall/precision is measured against the existing real-repo callgraph benchmarks and surfaced in benchmark reports, giving `polint graph` answers a recorded accuracy baseline and the next milestone's accuracy work a starting point.

### Pipeline Cost and Memory Discipline

- [ ] **PERF-01**: Store ingest respects the capability-gated semantic pipeline and rule-scoped discovery. Enabling persistence must not resurrect the eager whole-repo pipeline or whole-repo source loading for runs whose rules do not request deep facts; what the store persists follows what the run legitimately computed.
- [ ] **PERF-02**: Store ingest streams in bounded, sorted batches. Building a commit plan must not require holding the full generation's rows, payloads, or source text resident at once. Peak ingest memory is measured in the benchmark suite.
- [x] **PERF-03**: When persistence is disabled, unavailable, or skipped, `polint check` and `polint review` take a zero-cost path: no store I/O, no schema checks on the hot path, and no behavior drift.
- [ ] **PERF-04**: Once dependency package summaries persist and validate, dependency bodies are not re-parsed or re-summarized while their (package, version, schema, toolchain, config) identity matches. This is the O(working set) memory property from the locked research, verified by fixture (dependency source removed or altered without identity change is never re-read) and by benchmark.

### Store Foundation

- [x] **STORE-01**: Add a private SQLite/rusqlite semantic-store facade owned by the analysis kernel, with `pub(crate)` boundaries and no escaped `rusqlite` connection, statement, row, or SQL-string types.
- [x] **STORE-02**: Support migrations, schema versioning through `PRAGMA user_version`, controlled diagnostics for future/invalid schemas, and safe rebuild or skipped-persistence behavior.
- [x] **STORE-03**: Use explicit connection policy: foreign keys enabled, WAL where appropriate, bounded busy timeout, one writer boundary, and separate read-only query connections.
- [ ] **STORE-04**: Persist store manifest, active generation, pending generation, complete generation, schema version, workspace/config identity, and store stats.
- [ ] **STORE-05**: Commit only complete validated generations. A crash, failed migration, failed payload write, or failed search rebuild must leave either the old complete generation readable or require an explicit rebuild diagnostic.
- [x] **STORE-06**: Providers and rule execution do not receive SQL connections. They communicate through typed kernel/store methods and existing provider output structures.
- [x] **STORE-07**: Store failure during `polint check` produces controlled internal diagnostics, rebuilds, or skipped persistence; it must not produce partial policy answers with confident output.
- [x] **STORE-08**: Two concurrent `polint` processes against the same store serialize safely through a generation lease, or the loser falls back to read-only/skipped persistence with a clear diagnostic. Concurrent invocations must never corrupt, interleave, or partially overwrite generations.

### Metadata, Facts, and Invalidation

- [ ] **META-01**: Mirror existing kernel identity vocabulary in the store: `InputSnapshot`, provider manifests, layer keys, summary keys, query keys, provider output metadata, validation events, and dependency indexes.
- [ ] **META-02**: Persist normalized validated facts and indexes for files, packages/modules, imports/exports, resolutions, symbols, definitions, references, functions, calls, evidence, summaries, unknown regions, and budget events as they become available. Whole-program data-flow/taint results are never eagerly materialized: persist summaries and graph adjacency, compute path/taint answers demand-driven at query time over those persisted rows, and persist only bounded query results/traces keyed by existing query keys.
- [ ] **META-03**: Every fact-like row carries stable semantic identity, repository-relative path identity where applicable, fact family, provider/schema identity, precision, confidence/status, provenance, validation state, dependency metadata, and generation.
- [ ] **META-04**: Invalidation dependencies include source files, packages/projects, provider manifests, requested capabilities, language lifecycle inputs, config, schema, summary keys, query options, budget profiles, search manifests, and future model/extension digests where relevant.
- [ ] **META-05**: Deterministic public/query output never depends on SQLite `rowid`, insertion order, unordered Rust maps, parallel provider completion order, or Tantivy internal document IDs.
- [ ] **META-06**: The store does not persist full AST, source, MIR, CFG, or whole raw graph dumps as the product storage model. Persist normalized facts, summaries, indexes, compact evidence, digests, spans, and payload references.
- [ ] **META-07**: `unknown`, `unsupported`, `setup_missing`, `partial`, and `budget_exceeded` states remain durable and queryable. They must never collapse into `not_found` or an empty result.

### Summary Persistence and Registry-Ready Seams

- [ ] **SUM-01**: Persist summary manifests for dependency package summaries and application function/SCC summaries with package/version identity, schema version, toolchain/frontend identity, config digest, provenance, validation metadata, and precision/status.
- [ ] **SUM-02**: Use content-addressed payload IDs for summary payloads and registry-ready package summary seams. Introduce typed digest wrappers so cache invalidation keys and content-addressed payload digests cannot be confused.
- [ ] **SUM-03**: Decide summary payload layout through validation before locking it in: SQLite BLOBs, adjacent content-addressed files, or a hybrid must be benchmarked for DB size, WAL growth, crash behavior, restore behavior, and read latency.
- [ ] **SUM-04**: Implement the invalidation frontier so warm runs recompute changed functions/SCCs plus transitive summary dependents while reusing valid dependency and unaffected application summaries.
- [ ] **SUM-05**: Enable summary reuse only after from-scratch parity, recompute-and-diff checks, manifest validation, and stale-reuse prevention fixtures pass.
- [ ] **SUM-06**: Store summary-derived facts with explicit precision, confidence/status, provenance, digest identity, validation state, and trust placeholders. Heuristic summaries must remain labeled heuristic.
- [ ] **SUM-07**: Build no remote registry, publish protocol, fetch protocol, auth/signing layer, or central corpus in v2.0. The milestone only preserves local manifest/payload seams that can support a future registry.

### Warm Review Payoff

The invalidation frontier is the practical payoff of this milestone. It is a first-class deliverable with its own falsifiable requirements, not an emergent property of summary persistence.

- [ ] **REV-01**: Warm `polint review` recomputes exactly the invalidation frontier: changed functions/SCCs plus transitive summary dependents, reusing valid dependency and unaffected application summaries. The recompute set is instrumented and asserted in fixtures covering both must-recompute and must-reuse cases.
- [ ] **REV-02**: Warm `polint review` on the frontier benchmark meets the p50/p95 latency target set from the Phase 0 baseline, and internal diagnostics report summary hit/miss/stale/invalid counts so reuse quality is observable.
- [ ] **REV-03**: Warm review output is byte-identical to cold review output for the same inputs, including findings, evidence, unknowns, setup gaps, and budget states. Warm reuse ships only behind this parity gate.

### Internal Query Engine

- [ ] **QUERY-01**: Add private internal query services over complete store generations for used-by, neighbors, callers, callees, path, taint-style reachability, and search candidate resolution.
- [ ] **QUERY-02**: Standardize one internal query result envelope that can later render CLI JSON with `version`, `schema`, `command`, `query`, `status`, `precision`, `nodes`, `edges`, `paths`, `findings`, `unknowns`, `budgets`, and `summary`.
- [ ] **QUERY-03**: Query status values include at least `complete`, `partial`, `not_found`, `unknown`, `budget_exceeded`, `unsupported`, and `setup_missing`; `not_found` is valid only when the available evidence is complete enough for that claim.
- [ ] **QUERY-04**: Query filters cover path globs, tests included/excluded, minimum precision, provenance, unknown handling, max depth, max paths, and result limits where relevant.
- [ ] **QUERY-05**: Path and taint-style queries are bounded, cycle-aware, deterministic, evidence-backed, and explicit about barriers, summaries, unknown regions, and budget exhaustion.
- [ ] **QUERY-06**: Query results use stable semantic IDs, repository-relative paths, spans, precision, provenance, evidence IDs, and status fields. Raw store row IDs, provider IDs, parser IDs, solver IDs, and SQL names never appear.
- [ ] **QUERY-07**: Search results are candidates over stable store document IDs. Search does not create semantic facts and must not become an input to deterministic `polint check` behavior.
- [ ] **QUERY-08**: Query correctness fixtures land before public CLI promotion and cover cross-file refs, cross-package imports, direct/refined calls, cycles, paths, taint barriers, summary boundaries, setup gaps, unknown-preserving no-results, and budget exhaustion.

### Local CLI Graph Surface

- [ ] **CLI-01**: Promote selected `polint graph` commands only after the underlying internal query fixtures, no-leak gates, determinism gates, docs, and benchmark gates pass.
- [ ] **CLI-02**: Initial graph commands should cover used-by, neighbors, callers, callees, path, taint-style reachability, and lexical search as phase gates allow.
- [ ] **CLI-03**: Graph commands provide agent-friendly JSON as the design center and human output as a convenience renderer over the same private query envelope.
- [ ] **CLI-04**: The public graph CLI exposes bounded purpose-built commands and structured filters, not SQL, table inspection, Cypher, Datalog, QL, SPARQL, or a generic graph shell.
- [ ] **CLI-05**: CLI docs explain limits, precision, unknowns, budgets, summary-backed evidence, and the exploration-to-policy workflow honestly.
- [ ] **CLI-06**: Promoted CLI JSON schemas have snapshots and compatibility notes. Internal store schema changes must not force public JSON schema changes.
- [ ] **CLI-07**: Graph command docs and benchmark reports carry the measured recall/precision context from BENCH-04, and unknown counts render by default, so users and agents can calibrate trust in graph answers instead of reading them as complete.

### Search Boundary

- [ ] **SEARCH-01**: Define a `SearchCorpus` over stable semantic-store document IDs before adding a search engine dependency.
- [ ] **SEARCH-02**: Add Tantivy in the lexical-search phase for symbols, evidence text, diagnostic text, summaries, and selected snippets after store IDs and query envelopes are stable.
- [ ] **SEARCH-03**: Tantivy internal document IDs, segment state, and index layout remain private. Results map back to stable semantic document IDs and evidence/spans from the store.
- [ ] **SEARCH-04**: Search indexes are derived artifacts tied to store manifest/content digests and complete generations. Rebuild and swap must be crash-safe.
- [ ] **SEARCH-05**: Stable vector search is deferred. Any experimental vector work must stay off by default and require explicit model, chunker, dimension, metric, normalization, provenance, and content-digest metadata.

### Validation, Recovery, and Scale

- [ ] **VAL-01**: Cold build, warm build, restored-store build, partial invalidation, process restart, randomized provider order, and different Rayon worker counts produce byte-identical normalized policy and query JSON where semantics are unchanged.
- [x] **VAL-02**: Migration tests cover empty DB, previous schema, idempotent migration, future-schema refusal, invalid-schema rebuild path, and controlled diagnostics.
- [ ] **VAL-03**: Crash/recovery tests kill the process during SQLite ingest transaction, summary payload write, migration, WAL checkpoint, and search rebuild. Recovery must expose only a complete generation or a rebuild-needed diagnostic.
- [ ] **VAL-04**: Stale-reuse mutation fixtures cover source edits, package/lifecycle config changes, provider manifest changes, requested-capability changes, schema changes, summary dependency changes, query option changes, and budget-profile changes.
- [ ] **VAL-05**: Unknown, unsupported, setup-missing, partial, and budget-exceeded behavior is covered by fixtures and remains visible in graph/query/review output.
- [ ] **VAL-06**: Public-boundary leak gates prove SQL, table names, raw row IDs, provider generation IDs, parser IDs, solver IDs, raw graph internals, and store payload formats are absent from SDK prelude, CLI JSON, README, docs/facts, examples, and generated skill text.
- [ ] **VAL-07**: Scale benchmarks cover ingest/query p50 and p95, DB and WAL size, RSS, pruning/vacuum cost, recursive CTE versus Rust traversal, and payload BLOB versus adjacent file behavior at 100k, 500k, and 1M+ row scales where practical.
- [ ] **VAL-08**: Cache/status/clean/prune behavior accounts for semantic-store generations, payloads, search indexes, stale rows, WAL/checkpoint policy, and orphaned payload cleanup.
- [ ] **VAL-09**: External temp-repo tests continue to prove repo-local rules import only `polint::sdk::prelude::*`, register through `polint::runner::run_cli`, consume public typed views, and observe unchanged `polint check --format json` behavior.

## Future Requirements

Deferred to later milestones unless explicitly pulled forward.

### Remote Registry

- **REG-FUT-01**: Add remote package-summary publish/fetch only after local manifest identity, recompute-and-diff, validation metadata, trust hooks, and poisoning/revocation models are proven.
- **REG-FUT-02**: Add signing, provenance verification, trust policy, corpus management, and cache-poisoning defenses before any remote registry affects analysis answers.
- **REG-FUT-03**: Add registry import/export CLI only with explicit user action and clear offline fallback behavior.

### Stable Query and Agent Surfaces

- **QUERY-FUT-01**: Promote a stable graph/query JSON schema after the exploratory CLI has survived at least one milestone of usage and compatibility review.
- **QUERY-FUT-02**: Add MCP/LSP/editor integrations over the query envelope once the local CLI schema is stable.
- **QUERY-FUT-03**: Promote deliberately scoped SDK fact views for graph/security queries only after CLI semantics, evidence, and no-leak gates are stable.

### Accuracy Program (next milestone headline)

The locked research names the type-directed callgraph tier as the largest real-world F1 lever. v2.0 deliberately excludes it so the store foundation stays focused, but it is the default headline for the milestone after v2.0. v2.0's only accuracy obligation is BENCH-04/CLI-07: record the baseline and keep graph answers honest about it.

- **ACC-FUT-01**: Type-directed callgraph tier — Go types and TS types via a type sidecar (XTA-grade, near-linear) resolving call sites through the cheapest sufficient tier, with field-based/value-flow fallback and points-to only for the untyped residue.
- **ACC-FUT-02**: Selective context sensitivity and demand-driven precision refinement after the type-directed tier lands.
- **ACC-FUT-03**: Verified ML at the edges (type/callable-shape inference for unresolved sites, callee ranking, LLM package summaries) only with symbolic verification and honesty labels, per the locked ML verdict.

### Security Graph and Higher Precision

- **SEC-FUT-01**: Add deeper security graph analysis for user-controlled data, SQL injection, SSRF, command injection, auth boundaries, and sanitizer/barrier reasoning after path and taint-style query semantics are validated locally.
- **SEC-FUT-02**: Add higher-precision context sensitivity or solver modes behind explicit budgets and benchmark gates.
- **SEC-FUT-03**: Add framework/domain packs only after they can emit validated facts or documented heuristic summaries without hiding unknowns.

### Search and Vector

- **SEARCH-FUT-01**: Promote vector search only after deterministic model/chunker/provenance lockfiles, embedding invalidation, privacy controls, and replayable index builds exist.
- **SEARCH-FUT-02**: Add hybrid lexical/vector search as exploration only; search results must still not be policy truth without graph/provider verification.

## Out of Scope

Explicitly excluded from v2.0 to prevent scope creep.

| Feature | Reason |
|---------|--------|
| Remote package-summary registry | The user explicitly deferred it. v2.0 builds local registry-ready seams only. |
| Public SQL, Cypher, Datalog, QL, SPARQL, or generic graph shell | Exposes unstable internals and creates a large support contract before command-specific semantics are proven. |
| Public raw graph SDK views | Raw graph APIs freeze internal storage and solver choices. Promote only deliberate typed views later. |
| `polint graph` as CI gate | Enforcement belongs in Rust rules through `polint check` and `polint review`. |
| Stable vector search | Requires model/chunker/provenance lockfiles and deterministic invalidation not needed for the first implementation. |
| Live embeddings or model downloads in deterministic commands | Would compromise offline reproducibility and policy determinism. |
| Full AST/source/MIR/CFG dumps as the store | Bloats local state, increases privacy/support risk, and exposes the wrong abstraction. |
| Dual graph store or graph database by default | Risks drift between facts, summaries, and query indexes. SQLite plus bounded Rust traversal is the v2.0 default. |
| Parser/frontend replacement | Continue using Oxc for TS/JS and tree-sitter-go plus existing semantic providers for Go. |
| Daemon/server requirement | v2.0 is an offline embedded local CLI foundation. |

## Traceability

Mapped in `.planning/ROADMAP.md` (phases 63-71); full per-requirement table lives in the roadmap's Requirement Coverage section.

| Requirement Area | Phase | Status |
|------------------|-------|--------|
| Product Boundaries | 64 (PROD-01), 67 (PROD-02), 69 (PROD-03/04/05) | Mapped |
| Ground Truth and Performance Baseline | 63 (gates enforced 64-71) | Mapped |
| Pipeline Cost and Memory Discipline | 64 (PERF-03), 66 (PERF-01/02), 67 (PERF-04) | Mapped |
| Store Foundation | 64 (STORE-01/02/03/06/07/08), 65 (STORE-04/05) | Mapped |
| Metadata, Facts, and Invalidation | 65 (META-01/04), 66 (META-02/03/05/06/07) | Mapped |
| Summary Persistence and Registry-Ready Seams | 67 | Mapped |
| Warm Review Payoff | 67 | Mapped |
| Internal Query Engine | 68 | Mapped |
| Local CLI Graph Surface | 69 | Mapped |
| Search Boundary | 70 (designated scope-cut) | Mapped |
| Validation, Recovery, and Scale | 64 (VAL-02), 67 (VAL-04), 68 (VAL-05), 69 (VAL-06), 71 (VAL-01/03/07/08/09) | Mapped |

---
*Requirements drafted: 2026-07-07*
*Outcome gates, BENCH/PERF/REV requirements, and locked decisions added: 2026-07-08/09*
*Approved and mapped to roadmap phases 63-71: 2026-07-09*
