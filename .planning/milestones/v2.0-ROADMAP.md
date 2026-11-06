# Roadmap: polint

## Milestones

- [x] **v1.0 MVP** - repo-local static analysis framework for Go and TypeScript/JavaScript, shipped 2026-05-02. Archive: [v1.0 roadmap](milestones/v1.0-ROADMAP.md).
- [x] **v1.1 Capability Fulfillment** - capability planning, resolved imports/module graph, and symbol/reference foundations for Go and TS/JS.
- [x] **v1.2 Static Analysis Engine Implementation** - private, validated, cache-aware, agent-extensible analysis engine substrate; 22 phases and 136 plans shipped 2026-05-27. Archive: [v1.2 roadmap](milestones/v1.2-ROADMAP.md).
- [x] **v1.3 Graph Engine Precision** - shared semantic graph, reachability/root semantics, Go RTA, JS/TS token/object models, adaptation, unknown taxonomy, budgets, and benchmark promotion gates. Archive: [v1.3 roadmap](milestones/v1.3-ROADMAP.md).
- [x] **v1.4 Policy Query Surface** - preview SDK views and typed query objects for realistic repo-local policies over calls, control flow, and data flow. Completed 2026-06-20. Archive: [v1.4 roadmap](milestones/v1.4-ROADMAP.md).
- [ ] **v2.0 Static Analysis 2.0 Implementation** - durable, queryable local semantic layer: private SQLite/rusqlite store, summary persistence and invalidation frontier, warm `polint review` payoff, internal query engine, exploratory `polint graph` CLI, lexical-search boundary, and scale/recovery gates. 9 phases (63-71).

## Current Status

**Milestone:** v2.0 Static Analysis 2.0 Implementation (active)
**Phases planned:** 9 (Phase 63 - Phase 71)
**Requirements coverage:** 67/67 mapped
**Granularity:** coarse phases; plans generated per phase via plan-phase

Phase numbering continues from v1.4's last phase 62. All new store/query modules stay `pub(crate)`; v1.2-v1.4 promotion discipline applies — the only public promotion in this milestone is the gated `polint graph` CLI surface in Phase 69. Every phase names the milestone outcome gate it advances (scale, latency, honesty, accuracy visibility — see `.planning/REQUIREMENTS.md` Milestone Outcome Gates); the BENCH-03 regression gates run at every phase boundary from Phase 64 onward. Phase 70 is the designated scope-cut if the milestone runs long. Phases 69 and 70 may run in parallel after Phase 68.

## Phases (v2.0)

- [x] **Phase 63: Ground Truth and Performance Baseline** - Real-repo benchmark suite, RSS/latency/store-size curves, store-disabled baselines, regression-gate wiring, persisted-graph recall baseline. *(Outcome gates: all — makes them measurable)* (completed 2026-07-09)
- [x] **Phase 64: Store Foundation and Boundary Proof** - rusqlite bundled, private store facade, migrations, connection policy, generation lease, zero-cost disabled path, no-leak gates. *(Outcome gate: scale — zero-overhead discipline)* (completed 2026-07-10)
- [ ] **Phase 65: Generation Manifest and Metadata Mirroring** - Input snapshots, provider manifests, layer entries/dependencies, validation events, complete-generation commit discipline, invalidation dependency indexes. *(Outcome gate: latency — invalidation vocabulary)*
- [ ] **Phase 66: Validated Fact and Graph Index Ingest** - Normalized facts, adjacency/evidence/unknown/budget indexes, streaming bounded-batch ingest, pipeline-gating preservation, deterministic ordering. *(Outcome gates: scale + honesty)*
- [ ] **Phase 67: Summary Persistence, Invalidation Frontier, and Warm Review** - Summary manifests, blake3 content-addressed payloads, frontier recomputation, warm `polint review` latency win, O(working set) property. *(Outcome gates: scale + latency — the keystone phase)*
- [ ] **Phase 68: Internal Query Engine and Envelope** - Private used-by/neighbors/callers/callees/path/taint services, status vocabulary, filters, query correctness fixtures. *(Outcome gate: honesty)*
- [ ] **Phase 69: Public Graph CLI Promotion** - Gated `polint graph` commands, agent-shaped JSON envelopes, honest docs, schema snapshots, recall context, full leak-gate sweep. *(Outcome gates: honesty + accuracy visibility)*
- [ ] **Phase 70: Lexical Search Boundary** - SearchCorpus, Tantivy index over stable store document IDs, crash-safe rebuild/swap, vector search kept deferred. **Designated scope-cut** if the milestone runs long. *(Outcome gate: none — cut candidate by design)*
- [ ] **Phase 71: Recovery, Pruning, and Scale Gates** - Full determinism matrix, crash/recovery suite, 100k/500k/1M+ row benchmarks, prune/vacuum/WAL policy, external probe re-proof, milestone outcome-gate report. *(Outcome gates: all — closeout proof)*

## Phase Details

### Phase 63: Ground Truth and Performance Baseline

**Goal:** The scale, latency, and accuracy problems become visible and gateable before any store code lands: baselines are recorded, curves are produced, and regression gates are wired so every later phase can prove it moved an outcome gate.

**Depends on:** Existing internal eval harness and external benchmark adapters (`eval/external`: jelly_callgraph, go_x_tools_callgraph, gosec, secbench_js); v1.3 promotion-gate infrastructure.

**Requirements:** BENCH-01, BENCH-02, BENCH-03, BENCH-04

**Plans:** 4/4 plans complete

Plans:
**Wave 1**

- [x] 63-01-PLAN.md — Real-repo suite manifests (grafana/hugo/excalidraw/devloupe) + perf measurement substrate (peak RSS, cold/warm, curve types)

**Wave 2** *(blocked on Wave 1 completion)*

- [x] 63-02-PLAN.md — Whole-repo perf runner (check + review) + curve JSON + markdown benchmark report

**Wave 3** *(blocked on Wave 2 completion)*

- [x] 63-03-PLAN.md — Store-disabled check/review baselines + pre-store graph recall/precision accuracy baseline

**Wave 4** *(blocked on Wave 3 completion)*

- [x] 63-04-PLAN.md — Regression-gate wiring (+20% peak RSS / +25% cold wall-clock budgets, Fail-not-silent)

**Success Criteria** (what must be TRUE):

1. A pinned-commit benchmark suite manifest exists covering the locked repo set: `grafana/grafana` (primary large polyglot Go+TS), `gohugoio/hugo` (Go medium), `excalidraw/excalidraw` (TS medium), the existing Jelly and Go x/tools oracle suites (micro + recall), and the private devloupe monorepo documented as a local-only, non-CI reference (known baseline: ~1GB peak RSS, cold 7.4s / warm 4.6s).
2. The harness produces peak RSS, cold/warm wall-clock, cache/store size, and budget-exhaustion telemetry as machine-readable curves versus repo size and diff size, plus a markdown report.
3. Store-disabled baselines for `polint check` and `polint review` are recorded and committed as the reference for the locked regression budgets (≤ +20% peak RSS, ≤ +25% cold wall-clock).
4. Persisted-graph recall/precision baseline is recorded from the Jelly and Go x/tools callgraph adapters and appears in the benchmark report (accuracy-visibility gate).
5. Regression-gate wiring exists: a later phase exceeding a budget fails its gate rather than passing silently.

**Research flags:** none — extends the existing eval harness and adapters.

---

### Phase 64: Store Foundation and Boundary Proof

**Goal:** A private, crash-safe SQLite store facade exists with migrations and connection discipline, `polint check` behavior is provably unchanged, and the store costs nothing when disabled.

**Depends on:** Phase 63 (baselines recorded before persistence lands).

**Requirements:** STORE-01, STORE-02, STORE-03, STORE-06, STORE-07, STORE-08, PERF-03, PROD-01, VAL-02

**Success Criteria** (what must be TRUE):

1. `rusqlite` (bundled) and the `pub(crate)` store facade land under `analysis_kernel/store/`; no `rusqlite` connection, statement, row, or SQL-string type escapes the module (leak test).
2. Migrations run through `PRAGMA user_version` with fixtures for empty DB, previous schema, idempotent re-run, future-schema refusal, and invalid-schema rebuild diagnostics (VAL-02).
3. Connection policy is explicit: foreign keys on, WAL, bounded busy timeout, one writer boundary, separate read-only connections; two concurrent `polint` processes serialize through a generation lease or fall back to read-only/skipped persistence with a clear diagnostic (STORE-08).
4. `polint check` output is byte-identical with the store enabled, disabled, and corrupted; disabled/skip paths perform no store I/O or schema checks on the hot path (PERF-03, PROD-01, STORE-07).
5. Providers and rule execution receive no SQL connections (STORE-06); the public-surface leak gate is extended to store/SQL/table-name namespaces and runs from this phase onward.

**Research flags:** none — standard rusqlite facade/migration patterns.

---

### Phase 65: Generation Manifest and Metadata Mirroring

**Goal:** The store speaks the kernel's existing identity vocabulary — snapshots, manifests, layer keys, dependency indexes — and commits only complete validated generations, so invalidation and recovery have one source of truth before facts are broadly ingested.

**Depends on:** Phase 64.

**Requirements:** STORE-04, STORE-05, META-01, META-04

**Success Criteria** (what must be TRUE):

1. Store manifest, input snapshots, provider manifests, provider generations, layer entries/dependencies, validation events, and store stats persist, with active/pending/complete generation selection (STORE-04).
2. Only complete validated generations become readable; a crash, failed migration, failed payload write, or failed rebuild leaves either the old complete generation readable or an explicit rebuild diagnostic — never mixed rows (STORE-05).
3. The store mirrors `InputSnapshot`, provider manifests, layer/summary/query keys, and `FactMeta` vocabulary as first-class columns; no second identity or invalidation system appears (META-01).
4. Invalidation dependency indexes cover source files, packages/projects, provider manifests, requested capabilities, lifecycle inputs, config, schema, summary keys, query options, budget profiles, and future model/extension digests, with must-invalidate and must-preserve-hit fixtures (META-04).

**Research flags:** none — mirrors existing kernel metadata patterns.

---

### Phase 66: Validated Fact and Graph Index Ingest

**Goal:** Normalized validated facts and graph indexes persist with full identity metadata and deterministic ordering, through a streaming ingest that provably does not regress the capability-gated pipeline or the rule-scoped discovery memory wins.

**Depends on:** Phase 65.

**Requirements:** META-02, META-03, META-05, META-06, META-07, PERF-01, PERF-02

**Success Criteria** (what must be TRUE):

1. Files, packages/modules, imports/exports, resolutions, symbols, definitions, references, functions, calls, evidence, summary metadata, unknown regions, and budget events persist as normalized rows and adjacency/evidence indexes; whole-program data-flow/taint rows are never eagerly materialized (META-02).
2. Every fact-like row carries stable semantic identity, repo-relative path, fact family, provider/schema identity, precision, confidence/status, provenance, validation state, dependency metadata, and generation (META-03).
3. Deterministic output never depends on `rowid`, insertion order, unordered maps, or provider completion order — proven by provider-order shuffle and Rayon worker-count permutation tests (META-05).
4. No full AST/source/MIR/CFG dumps persist (META-06); unknown/unsupported/setup-missing/partial/budget-exceeded states are durable and queryable, never collapsed (META-07).
5. Ingest follows what the run legitimately computed: capability-gated pipeline and rule-scoped discovery are preserved with the store enabled (PERF-01), ingest streams in bounded sorted batches with measured peak memory (PERF-02), and the Phase 63 regression gates pass on the benchmark suite.

**Research flags:** none — reuses existing fact metadata and stable-key discipline.

---

### Phase 67: Summary Persistence, Invalidation Frontier, and Warm Review

**Goal:** The milestone keystone: summaries persist with registry-ready manifests, warm runs recompute only the invalidation frontier, dependency bodies are never re-parsed once summarized, and `polint review` shows a measured warm-latency win.

**Depends on:** Phase 66; Phase 63 (frontier benchmark and latency targets).

**Requirements:** SUM-01, SUM-02, SUM-03, SUM-04, SUM-05, SUM-06, SUM-07, PERF-04, REV-01, REV-02, REV-03, PROD-02, VAL-04

**Success Criteria** (what must be TRUE):

1. Summary manifests persist for dependency package summaries and application function/SCC summaries with package/version identity, schema version, toolchain/frontend identity, config digest, provenance, validation metadata, and precision/status (SUM-01); payloads use blake3 content addressing behind typed digest wrappers that cannot be confused with cache invalidation keys (SUM-02).
2. Payload layout (SQLite BLOBs vs adjacent content-addressed files vs hybrid) is locked by benchmark evidence covering DB size, WAL growth, crash behavior, restore behavior, and read latency (SUM-03).
3. Warm runs recompute exactly the invalidation frontier — changed functions/SCCs plus transitive summary dependents — with the recompute set instrumented and asserted in must-recompute and must-reuse fixtures (SUM-04, REV-01); stale-reuse mutation fixtures cover every upstream input class (VAL-04).
4. Summary reuse ships only behind from-scratch parity, recompute-and-diff, manifest validation, and stale-reuse prevention (SUM-05); warm review output is byte-identical to cold (REV-03).
5. Warm `polint review` on the frontier benchmark meets the p50/p95 target set from the Phase 63 baseline, and internal diagnostics report summary hit/miss/stale/invalid counts (REV-02, PROD-02).
6. Dependency bodies are not re-parsed or re-summarized while their (package, version, schema, toolchain, config) identity matches — verified by fixture and benchmark (PERF-04); summary-derived facts stay labeled with precision/provenance/trust placeholders (SUM-06); no registry protocol of any kind exists (SUM-07).

**Research flags:** payload layout benchmark design (SUM-03) — needs deeper research during plan-phase.

---

### Phase 68: Internal Query Engine and Envelope

**Goal:** Query semantics are proven privately — used-by, neighbors, callers, callees, path, and taint services over complete generations, with one honest envelope and correctness fixtures — before any public CLI exists.

**Depends on:** Phase 66 (facts/adjacency); Phase 67 (summary-boundary query behavior).

**Requirements:** QUERY-01, QUERY-02, QUERY-03, QUERY-04, QUERY-05, QUERY-06, QUERY-07, QUERY-08, VAL-05

**Success Criteria** (what must be TRUE):

1. Private query services answer used-by, neighbors, callers, callees, path, taint-style reachability, and search-candidate resolution over complete store generations only (QUERY-01).
2. One internal envelope carries `version`, `schema`, `command`, `query`, `status`, `precision`, `nodes`, `edges`, `paths`, `findings`, `unknowns`, `budgets`, `summary`; status vocabulary includes `complete`, `partial`, `not_found`, `unknown`, `budget_exceeded`, `unsupported`, `setup_missing`, and `not_found` requires sufficient evidence for the claim (QUERY-02, QUERY-03).
3. Filters cover path globs, tests on/off, minimum precision, provenance, unknown handling, max depth, max paths, and limits (QUERY-04); path/taint queries are bounded, cycle-aware, deterministic, evidence-backed, and explicit about barriers, summaries, unknowns, and budgets (QUERY-05).
4. Results carry stable semantic IDs, repo-relative paths, spans, precision, provenance, evidence IDs, and status — never store row IDs, provider/parser/solver IDs, or SQL names (QUERY-06); search results are candidates only and never feed deterministic `check` (QUERY-07).
5. Correctness fixtures cover cross-file refs, cross-package imports, direct/refined calls, cycles, paths, taint barriers, summary boundaries, setup gaps, unknown-preserving no-results, and budget exhaustion (QUERY-08); unknown/budget behavior remains visible in all query output (VAL-05).

**Research flags:** path/taint traversal model, cycle handling, barriers, ranking, and budget semantics — needs deeper research during plan-phase.

---

### Phase 69: Public Graph CLI Promotion

**Goal:** Selected `polint graph` commands go public behind every gate — determinism, correctness, no-leak, docs, benchmarks — with agent-shaped JSON, honest limits, and measured recall context, without becoming a CI gate or a second rule system.

**Depends on:** Phase 68. May run in parallel with Phase 70.

**Requirements:** CLI-01, CLI-02, CLI-03, CLI-04, CLI-05, CLI-06, CLI-07, PROD-03, PROD-04, PROD-05, VAL-06

**Success Criteria** (what must be TRUE):

1. `polint graph` commands (used-by, neighbors, callers, callees, path, taint-style reachability; search as Phase 70 allows) are promoted individually, each only after its fixtures, no-leak, determinism, docs, and benchmark gates pass (CLI-01, CLI-02).
2. JSON is the design center; human output renders from the same private envelope (CLI-03); commands are purpose-built with structured filters — no SQL, table inspection, Cypher, Datalog, QL, SPARQL, or generic graph shell (CLI-04).
3. Docs explain limits, precision, unknowns, budgets, summary-backed evidence, and the exploration-to-policy workflow; `polint graph` has no CI pass/fail semantics and public docs describe Static Analysis 2.0 as durable local infrastructure, not a registry product (CLI-05, PROD-03, PROD-04, PROD-05).
4. Promoted JSON schemas have snapshots and compatibility notes; internal store schema changes do not force public schema changes (CLI-06).
5. Graph docs and benchmark reports carry the measured recall/precision context from Phase 63, and unknown counts render by default (CLI-07).
6. Leak gates prove SQL, table names, row IDs, provider generation IDs, parser IDs, solver IDs, raw graph internals, and payload formats are absent from SDK prelude, CLI JSON, README, docs/facts, examples, and generated skill text (VAL-06).

**Research flags:** first stable public graph JSON schema and per-command promotion checklist — needs deeper research during plan-phase.

---

### Phase 70: Lexical Search Boundary

**Goal:** Tantivy lexical search lands as a derived artifact over stable semantic-store document IDs — candidates only, crash-safe rebuild, no new public truth source.

**Depends on:** Phase 68 (stable document IDs and envelopes). May run in parallel with Phase 69. **Designated scope-cut:** if the milestone runs long, this phase moves to v2.1 by recorded decision; no other phase depends on it (Phase 71 search-rebuild crash tests apply only if this phase ships).

**Requirements:** SEARCH-01, SEARCH-02, SEARCH-03, SEARCH-04, SEARCH-05

**Success Criteria** (what must be TRUE):

1. `SearchCorpus` over stable semantic-store document IDs is defined before the Tantivy dependency is added (SEARCH-01).
2. Tantivy lexical search covers symbols, evidence text, diagnostic text, summaries, and selected snippets (SEARCH-02); Tantivy `DocId`s, segment state, and index layout stay private, and results map back to stable store document IDs and evidence spans (SEARCH-03).
3. Search indexes are derived artifacts tied to store manifest/content digests and complete generations; rebuild-and-swap is crash-safe and deterministic (SEARCH-04).
4. Vector search remains deferred and off by default, requiring explicit model/chunker/dimension/metric/normalization/provenance/content-digest lockfiles before any experiment (SEARCH-05).

**Research flags:** Tantivy code tokenization, field schema, rebuild lifecycle, and candidate wording — needs deeper research during plan-phase.

---

### Phase 71: Recovery, Pruning, and Scale Gates

**Goal:** Default store reuse becomes credible: the full determinism matrix, crash/recovery suite, large-scale benchmarks, pruning/WAL policy, and public-boundary re-proof all pass, and the milestone outcome-gate report shows scale, latency, honesty, and accuracy-visibility green.

**Depends on:** Phase 67 (summaries), Phase 69 (public surface); Phase 70 if shipped.

**Requirements:** VAL-01, VAL-03, VAL-07, VAL-08, VAL-09

**Success Criteria** (what must be TRUE):

1. Cold build, warm build, restored-store build, partial invalidation, process restart, randomized provider order, and different Rayon worker counts produce byte-identical normalized policy and query JSON where semantics are unchanged (VAL-01).
2. Crash/recovery tests kill the process during ingest transaction, summary payload write, migration, WAL checkpoint, and (if Phase 70 shipped) search rebuild; recovery exposes only a complete generation or a rebuild-needed diagnostic (VAL-03).
3. Scale benchmarks cover ingest/query p50/p95, DB and WAL size, RSS, pruning/vacuum cost, recursive-CTE-vs-Rust-traversal, and BLOB-vs-file behavior at 100k/500k/1M+ row scales, with decisions recorded (VAL-07).
4. `polint cache status/clean/prune` accounts for store generations, payloads, search indexes, stale rows, WAL/checkpoint policy, and orphaned payload cleanup (VAL-08).
5. External temp-repo tests re-prove that repo-local rules import only `polint::sdk::prelude::*`, register through `polint::runner::run_cli`, and observe unchanged `polint check --format json` behavior (VAL-09); the final benchmark report states each milestone outcome gate's status against the Phase 63 baselines.

**Research flags:** pruning/vacuum/checkpoint policy and large-store benchmark thresholds — needs deeper research during plan-phase.

---

## Requirement Coverage

| Phase | Requirements |
|-------|--------------|
| 63 | BENCH-01, BENCH-02, BENCH-03, BENCH-04 |
| 64 | STORE-01, STORE-02, STORE-03, STORE-06, STORE-07, STORE-08, PERF-03, PROD-01, VAL-02 |
| 65 | STORE-04, STORE-05, META-01, META-04 |
| 66 | META-02, META-03, META-05, META-06, META-07, PERF-01, PERF-02 |
| 67 | SUM-01..07, PERF-04, REV-01..03, PROD-02, VAL-04 |
| 68 | QUERY-01..08, VAL-05 |
| 69 | CLI-01..07, PROD-03, PROD-04, PROD-05, VAL-06 |
| 70 | SEARCH-01..05 |
| 71 | VAL-01, VAL-03, VAL-07, VAL-08, VAL-09 |

67/67 v2.0 requirements mapped; no orphans, no double-mapping. BENCH-03 is owned by Phase 63 (gate wiring) and enforced as a phase-boundary gate from Phase 64 onward.

---
*Roadmap generated: 2026-07-09 after v2.0 requirements approval*
