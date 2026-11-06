# Requirements: polint v2.1 Engine Depth

**Defined:** 2026-11-02
**Core Value:** Make it easy to express a repo-specific engineering policy as a small rule and run it locally, in CI, and with AI coding agents.

**Milestone goal:** Make polint's deep analysis usable in the inner loop, bring TypeScript/JavaScript to parity with Go, give AI agents an honest queryable graph, and cut the rule-pack build cost, without giving up byte-identical, deterministic output.

**Source context:** v1.2–v1.4 built and promoted the private engine. v2.0 shipped benchmarks, the store foundation and generation manifests (phases 63–65) and was then overtaken by the perf campaign and the Go deep-analysis steps (#129–#133). The research in `.planning/research/v2.1-engine/` measured the remaining gaps and produced this plan. `polint check` and `polint review` remain the core products; `polint graph` is an exploration surface, not a second rule system.

## Milestone Outcome Gates

See `ROADMAP.md` "Milestone outcome gates (v2.1)" for G1–G8 (edit tier, inner loop, byte identity, TS parity, agent graph, honesty, ice-cold build, regression budget). Every phase names the gates it advances.

## Locked Decisions

Approved 2026-11-02. Changing any of these requires a recorded decision, not a silent edit.

- **Files before SQLite.** Persisted unit summaries live in `.polint/cache/summaries/` (manifest + blake3 content-addressed payloads, temp-then-rename). The test-only SQLite store stays as it is; re-open the SQLite decision only with a measured need for cross-run queries or an index above a few MB.
- **Byte identity is the universal gate.** Reports identical at 1/4/6 jobs and ten permutations; restored answers carry the same unknowns and budget state as recomputed ones; rules requesting no deep capability see output identical to v0.5.0.
- **Measurement host and workloads.** OAIZ `core` and full repo at 4 jobs for the edit tier (private, local-only); the public corpora through `make bench-run` for regression budgets; `tests/taint-corpus` (Go) and the new `tests/taint-corpus-ts` for precision/recall.
- **Deep providers stay review/CI-only until Phase 78 measures otherwise.** The gate-2 kill criteria carry forward unchanged.
- **Track S is conditional.** Phases 86–88 run only if the Phase 85 spike is green under its recorded kill lines.
- **Cuts from v2.0, recorded:** provider metadata mirroring into SQLite (65 R5 remainder), whole-fact SQLite ingest (66), Tantivy lexical search (70, SEARCH-01..05), SQLite row-scale and WAL benchmarks (71 VAL-03/VAL-07). Remote registry stays deferred.

## v2.1 Requirements

Each requirement maps to exactly one roadmap phase.

### Kernel truth (K)

- [ ] **KRN-01**: Semantic index ids merged across files are rebased so no two files share a scope/symbol/export id; schema rotated to `symbol-graph-facts-3`; a two-file regression fails without the rebase. (#86)
- [ ] **KRN-02**: The Go RTA solver's identity and hard dependencies include the reachability roots it consumes; a root change invalidates the solver layer in a test. (#91)
- [ ] **KRN-03**: The semantic-graph output digest folds only inputs its builders read; each removed input has a mutation test proving it is unread. (#90)
- [ ] **KRN-04**: No provider output identity is inferred from fact rows; a cache-write failure keeps the computed digest and reports telemetry; `ProviderOutcomeStatus` flows through dependency inputs. (#87)
- [ ] **KRN-05**: Syntax-layer manifests carry the complete dependency edge set derived from the layer key (source, config, lifecycle, toolchain, parameters) and reads require exact-set equality, TS syntax included. (#89)
- [ ] **KRN-06**: Capability support handed to rules, `explain` and `inspect` is derived from sealed provider outcomes, so a failed or downgraded provider revokes the capabilities it backs. (#88)

### Go edit tier (E)

- [ ] **EDIT-01**: The Go sidecar reports sub-phase timings for the call-graph phase (CHA index, graph build, SCC, propagate, construct) and GC CPU share in its stage log.
- [ ] **EDIT-02**: The deep-gate harness has an edit cell (`EDIT=<patch>`) that applies a one-file change and records calls/dataflow/warm/cold reproducibly; VTA runs over a single CHA and, behind a flag, a reachable-function subset, with `call_edges` byte-identical to v0.5.0.
- [ ] **EDIT-03**: Flow bodies reference call sites by key and take callee candidates from `call_edges` on the Rust side, so static sidecar rows are independent of VTA.
- [ ] **EDIT-04**: Static sidecar rows are emitted and cached per package, keyed on package source digest, import export-data digests, sidecar digest and lifecycle digest; only VTA edges, routes and budgets form the whole-program artifact.
- [ ] **EDIT-05**: MIR/CFG/call-site/derived rows are persisted per unit (Go package) and restored for unchanged units; the recompute set is the changed units plus the reverse-dependency closure, instrumented in `KernelRunReport`.
- [ ] **EDIT-06**: The call-resolution cache restores partially: unchanged units' call slices are reused and the calls layer runs only for the frontier; `calls` one-file edit on OAIZ `core` ≤ 15 s at 4 jobs with warm no-change ≤ 6 s.
- [ ] **EDIT-07**: VTA propagation is incremental: per-SCC type sets are persisted and only SCCs reachable from changed nodes are re-propagated, with the resulting edge set identical to whole-program VTA on every corpus.
- [ ] **EDIT-08**: `(function, entry)` taint summaries are persisted with their unknowns and budget events and invalidated through the dependents set; `dataflow` one-file edit on OAIZ `core` ≤ 20 s; restored and recomputed answers have identical `is_complete` and unknown counts.
- [ ] **EDIT-09**: The gate-2 criteria are re-measured on the Phase 77 tip and the result (promotion or continued restriction) is recorded with numbers.

### Persistence (STORE, carried from v2.0 Phase 67)

- [ ] **STORE-10**: Unit summary manifests record schema, polint version, sidecar identity, models digest, per-unit input digest, dependency list and payload id (SUM-01 re-scoped).
- [ ] **STORE-11**: Payloads are blake3 content-addressed behind a `SummaryPayloadDigest` type that cannot be confused with cache invalidation digests; a missing or corrupt payload is a miss, never a crash (SUM-02, SUM-05).
- [ ] **STORE-12**: Must-recompute / must-reuse fixtures cover file edit, dependency edit, config edit, models edit, sidecar upgrade and missing payload; `polint cache status/clean/prune` accounts for summary payloads (VAL-04, VAL-08).
- [ ] **STORE-13**: Persisted budget and unknown state is durable end to end: a restored answer reports the same unknowns as the run that produced it (REV-03).

### Agent graph (GRAPH)

- [ ] **GRAPH-01**: `polint graph callers|callees|used-by|reach` answer over SDK views from the warm cache with `--max-depth`, `--min-precision`, `--include-tests`, `--path`, `--format json|text`, `--no-cache` (QUERY-01, CLI-01).
- [ ] **GRAPH-02**: One versioned envelope (`version`, `schema`, `tool`, `command`, `query`, `status`, `precision`, `nodes`, `edges`, `paths`, `unknowns`, `budgets`, `summary`) with status ∈ `complete|partial|not_found|unknown|budget_exceeded|unsupported|setup_missing`; `not_found` requires evidence; schema snapshot in `docs/schemas/polint-graph-v1.json` (QUERY-02, QUERY-03, CLI-03).
- [ ] **GRAPH-03**: Results carry stable semantic ids, repo-relative paths, spans, precision, provenance and evidence ids and never store row ids, provider/parser/solver ids or SQL names; the public-surface leak gate covers the new command (QUERY-06, CLI-06).
- [ ] **GRAPH-04**: `polint graph path` ranks bounded paths with cycle handling and `polint graph taint` applies barriers and models, both for Go and TS, reading persisted summaries (QUERY-04, QUERY-05).
- [ ] **GRAPH-05**: `budget_exceeded` and `setup_missing` are surfaced from persisted state without recomputation, and taint answers match `DataFlow::flows` on both corpora.

### TypeScript parity (TSP)

- [ ] **TSP-01**: Flow programs, route facts and model loaders are language-neutral: per-language accessors, `RouteFact` with a `language` field, `[[ts_flow_*]]`/`[[ts_route]]` tables, Go builtins moved to Go model data; no behavior change for Go.
- [ ] **TSP-02**: TypeScript type-sidecar callee rows feed `polint.calls` as typed call inputs so calls, refined calls and flow callees agree.
- [ ] **TSP-03**: An Oxc-based Rust lowering produces TS/JS flow programs (slots, captured cells, closures, destructuring, spread, optional chaining, await/then, classes, `this`, module globals) with no external toolchain.
- [ ] **TSP-04**: A TS taint corpus of ≥ 20 cases with twins runs through the existing harness at precision ≥ 0.90 and recall ≥ 0.70 without Node, with built-in Express/SQL/child_process/logging models.
- [ ] **TSP-05**: With the type sidecar present, `declared_type` rows enrich parameter types, classes and callees and enable type-keyed models; recall ≥ 0.80 at precision ≥ 0.90; results byte-identical to TSP-04 without Node; sidecar cold cost ≤ +20%.
- [ ] **TSP-06**: `Routes<'_>` inventories Express, Fastify and Hono endpoints through `[[ts_route]]` models with 100% expected-row match on fixtures and two-run byte identity.
- [ ] **TSP-07** *(optional, closeout)*: Next.js file routes and NestJS decorator routes are modelled, or recorded as deferred with unmodelled frameworks visible in `polint unknowns`.

### Thin SDK (SDK)

- [ ] **SDK-01**: The E1 closure and E3 snapshot-tax spike is measured and a go/no-go decision is recorded against the kill lines (E1 ≤ 35 s / ≤ 0.6 GB green, > 52 s / > 1 GB kill; E3 ≤ 5% warm green, > 15% kill; E2 > 20% of `analysis_kernel` kill).
- [ ] **SDK-02**: `polint-sdk`, `polint-engine` and the `polint` facade exist; rule packs compile unchanged through `package = "polint-sdk"`; the architecture test and docs state the three-package contract.
- [ ] **SDK-03**: A `PSNAP1` fact snapshot with per-family sections and a content digest is written once per run under the cache and read by the rule process; the family audit covers every macro-mapped view.
- [ ] **SDK-04**: Rule processes speak a length-prefixed stdio protocol (Hello, RunRequest, PolicyQuery/PolicyResult, RunResponse) with `sdk_abi` versioning and hard-exit mismatch codes; policy views answer over RPC with byte-identical results.
- [ ] **SDK-05**: `polint check`, `review` and `test` drive the protocol; warm runs spawn no `cargo`; `polint test --unit` is unchanged; the rule process cwd stays the repo root.
- [ ] **SDK-06**: The old rule-host path is removed, unmigrated manifests get a migration diagnostic, and `CONSUMER-SETUP.md`, `CACHE.md` and `GITHUB-ACTION.md` describe the new build; cold ≤ 20 s on the 2-vCPU rig and offline runs pass.

### Product and validation (PROD, VAL)

- [ ] **PROD-10**: Deep capabilities in the default `check` path are documented honestly in `docs/facts/`, the generated skill text and `CONSUMER-SETUP.md`, including which tiers remain review/CI-only.
- [ ] **PROD-11**: `polint graph` is documented as an exploration surface, not a CI pass/fail interface or a second rule system; recurring findings become repo-local rules (PROD-03, PROD-04).
- [ ] **VAL-10**: A determinism matrix (jobs × permutations × cold/warm/edit) passes on every corpus at milestone end (VAL-01).
- [ ] **VAL-11**: The closeout benchmark report states a measured value and a pass/fail/dropped status for each of G1–G8 against the v0.5.0 baselines, and external temp-repo tests re-prove that repo-local rules import only `polint::sdk::prelude::*` and register through `polint::runner::run_cli` (VAL-09).

## Carried forward from v2.0

| v2.0 ID | Disposition | v2.1 owner |
|---|---|---|
| PROD-01, PERF-03, STORE-01/02/03/06/07/08, VAL-02, BENCH-01..04 | shipped in phases 63–64; remain in force | — |
| STORE-04, STORE-05, META-01, META-04 | shipped in Phase 65 as sealed outcomes, parity and generation discipline; the summary index reuses the vocabulary | STORE-10 |
| META-02, META-03, META-05, META-06, META-07, PERF-01, PERF-02 | whole-fact ingest cut; the constraints (capability-gated persistence, bounded batches, deterministic ordering) apply to the summary store | STORE-10..12 |
| SUM-01, SUM-02, SUM-04..07, PERF-04, REV-01..03, PROD-02, VAL-04 | the keystone, delivered file-based | EDIT-05..08, STORE-10..13 |
| SUM-03 | decided: files, not BLOBs | locked decision |
| QUERY-01..06, QUERY-08, VAL-05 | in-memory query services over SDK views | GRAPH-01..05 |
| QUERY-07, SEARCH-01..05 | cut (lexical search) | later milestone |
| CLI-01..07, PROD-03..05, VAL-06 | promoted per command | GRAPH-01..03, PROD-11 |
| VAL-01, VAL-08, VAL-09 | closeout | VAL-10, VAL-11, STORE-12 |
| VAL-03, VAL-07 | cut (SQLite WAL and row-scale) | re-open with SQLite |

## Out of Scope (this milestone)

- Remote package-summary registry; vector search; public SQL/Cypher/Datalog surfaces.
- Python/Java adapters.
- `WorkspaceFiles` fact view (#52) and rule-authoring ergonomics (#9): a rule-authoring milestone after v2.1.
- Long-lived sidecar daemon / `polint watch`: a latency polish after the edit tier lands, not a lever for it.

---
*Requirements defined: 2026-11-02 from `.planning/research/v2.1-engine/`*
