# Semantic store re-scoping (Phases 65–71)

**Researched:** 2026-11-02 against `main` at `338b570b` (v0.5.0). Read-only code research; every claim carries a path.

## 1. What exists today

### The SQLite store (Phases 64–65)

- **Test-only.** `rusqlite` is a `[dev-dependencies]` entry (`crates/polint/Cargo.toml:80-87`); every store submodule is `#[cfg(test)]` and the non-test `SemanticStore::maintain` is a `debug_assert!(!enabled)` returning `StoreStatus::Disabled` (`crates/polint/src/analysis_kernel/store/mod.rs:6-11, 24-37, 175-183`). `Cache::semantic_store_enabled` defaults to `false` and can only be flipped by `with_semantic_store_enabled_for_test` (`crates/polint/src/cache/mod.rs:118-166`). The kernel's only production touchpoint is the `maintain` call that feeds a status into `KernelRunReport` (`analysis_kernel/mod.rs:916-932`).
- **Schema v5 tables** (`store/migrations.rs:8, 42-86`): `_polint_schema_migrations`, `generations` (pending/complete), `active_generation` (singleton FK to a complete generation), `run_manifests` + `run_manifest_sources` (hashed workspace root, config digest, per-source digest), `metrics_provider_mirror/_members/_blockers/_sources/_functions`, `go_syntax_provider_mirror/_members/_blockers/_sources/_parser`. No fact rows, no graph adjacency, no summary payloads, no blake3 (`Cargo.lock` has no blake3; no tantivy).
- **Lifecycle:** writer lease, WAL, FK on, busy timeout, read-only connections (`store/connection.rs:43-101, 156`); reserve → publish-in-one-immediate-transaction → authenticated active read with exact-match (`store/generation.rs:195, 211, 385-495`).
- **Phase 65 is further along than STATE.md says.** STATE.md reports "R1 and R2 complete", but plans 65-03 (sealed provider outcomes), 65-04 (validation ownership/cold-warm parity), 65-05 (metrics mirror, schema v4) and 65-06 (Go syntax mirror, schema v5) have SUMMARY and passed VERIFICATION files and are on `main` via PR #94. The accurate status: R1–R4 done plus the first R5 increment; remaining R5 increments (TS syntax, other providers) and R6 (production enablement/measured reuse) are open. `.planning/forensics/report-20260719-phase-65-scope-collapse.md` explains why the phase was restarted as slices.

### The file-based caches that already overlap Phases 65–66

| Cache | Persists | Keyed by | Invalidation |
|---|---|---|---|
| Layer cache (`analysis_kernel/incremental/layer_cache.rs:41-55, 168-230`) | JSON payload + manifest with `LayerKey`, output/payload digests, sorted dependency edges, precision, validation | `LayerKey` = provider id/version, schema, parameter/lifecycle/config/toolchain digests, per-file input digests, upstream layer digests (`incremental/keys.rs:52-64`) | Manifest dependency edges fed to `InvalidationPlan` (Reuse/Verify/Recompute/Drop/Quarantine, `incremental/invalidation.rs:20-56`). Used only by Go/TS syntax, module graph, symbol graph, metrics. The `semantic_mir/cfg/calls/direct_summaries` key constructors exist but are unused in product code. |
| Call-resolution cache (`analysis_kernel/call_cache.rs`) | One JSON entry with call sites/targets, unresolved calls, entrypoints, trust boundaries, dispatch edges, reachability roots, refined edges, plus each cached provider's output digest and diagnostics (`:140-160`) | Hash of the *entire* `InputSnapshot` (every file digest, config, lifecycle, rules, models, extensions) + Go syntax output digest + sidecar identity + models digest + polint version (`:97-116`) | Whole-entry: any changed file is a miss; `ENTRIES_KEPT = 2`; damaged entry deleted (`:43, 216-230`). Go-only plans only. |
| Go sidecar cache (`go/semantic/client.rs:106-135`, `go/semantic/cache_key.rs:32-47`) | Whole-run NDJSON of typed facts (and flow programs with `--dataflow`) | sidecar digest, go version, **whole-run** upstream syntax digest, lifecycle | Whole-entry miss on any Go file change. |
| SCC closure digest cache (`analysis/summaries/provider.rs:134-175`) | Previous SCC output digests for backdating only | config/rule/plan digests | n/a (telemetry only) |

Deep-analysis facts: taint summaries are **in-memory per query** (`analysis_neutral/taint/solver.rs:235, 313-321, 452-470`; no `Serialize` anywhere in `taint/`), keyed `(FnId, Entry)`, with per-package (`unit`) step budgets (`taint/index.rs:251-262`, `FlowUnknown::UnitBudget` in `sdk/dataflow.rs:233-259`). Flow programs carry `unit` (package) and `file` per function (`taint/ir.rs:155-162`), i.e. the unit identity for per-package summaries already exists in the IR.

**What this means for Phases 65–66:** META-01's "mirror layer keys/dependency indexes" and META-04's dependency classes are already modelled on disk by the layer-cache manifests (`DependencyKind`/`ShapeKind` in `incremental/dependency_index.rs:22-56`). STORE-04/05 (complete-generation discipline) exist but mirror only metadata no production path reads. The store currently duplicates identity bookkeeping without yet owning a single fact.

### The measured problem the plan must now serve

From the deep-analysis quick tasks (`.planning/quick/261003-u2r-*`, `261003-d3f-*`, `261007-r1v-*`): on OAIZ `core` (2,846 Go files) warm no-change `calls` is 5.7 s and `dataflow` 15 s thanks to the call cache, but **one-file edit is 41.7 s (calls) and 68.2 s (dataflow)**, and the floor is the sidecar's whole-program `vta.CallGraph` (`go-sidecar/.../semantic/typed.go:189`): 15.8 s / 26–30 s. The gate-2 kill path moved deep providers out of `check`'s default profile. Priority (1) is therefore bounded by the sidecar, not by Rust-side persistence; persistence only pays off together with per-unit sidecar output.

## 2. Phase-by-phase verdict

| Phase | Verdict | Surviving requirement IDs | Why |
|---|---|---|---|
| 65 R3–R4 | **done** | (contributions to META-01) | Sealed outcomes + parity are kernel improvements already on main. |
| 65 R5 (remaining providers) | **cut** | — | Mirroring TS syntax/metrics/module-graph metadata into SQLite advances no priority; the layer cache already keys and invalidates these. Don't add provider mirrors that nothing reads. |
| 65 R6 (production enablement) | **merge into keystone** | STORE-04, STORE-05 (re-scoped to the summary index), META-01, META-04 | Enablement only makes sense once the store holds something a warm run reads. |
| 66 (fact/graph ingest) | **cut as a phase; keep META-07, PERF-01/02 as constraints** | META-03, META-05, META-06, META-07, PERF-01, PERF-02 move to 67/68 as acceptance criteria | Ingesting every normalized fact family is weeks of schema work that priority (2) does not need: `CallGraph<'_>` already answers callers/callees/reachable/paths in memory (`sdk/call_graph.rs:167-239`), `References<'_>`/`Symbols<'_>` give used-by (`sdk/facts.rs:469-549`), `DataFlow::flows` gives taint. Graph queries can run over the live `AnalysisDb` behind the call cache. Persist graph adjacency only if `polint graph` latency on the warm path proves inadequate. |
| 67 (summaries + frontier + warm review) | **keep, becomes the first and only "store" phase; reorder before anything else** | SUM-01, SUM-02, SUM-04, SUM-05, SUM-06, SUM-07, PERF-04, REV-01, REV-02, REV-03, PROD-02, VAL-04. Defer SUM-03 (BLOB vs file benchmark) by choosing files now. | Directly serves priority (1). |
| 68 (query engine) | **keep, shrink to an in-memory service over SDK views** | QUERY-01..06, QUERY-08, VAL-05; drop QUERY-07 (search) | Priority (2). Envelope + status vocabulary is a few days; path/taint already exist. |
| 69 (public `polint graph`) | **keep, promote per command** | CLI-01..07 (CLI-02 minus search), PROD-03/04/05, VAL-06 | Priority (2)+(3). |
| 70 (Tantivy search) | **cut to a later milestone** | SEARCH-01..05 deferred unchanged | Already the designated cut; no priority touches it. |
| 71 (recovery/scale) | **merge the relevant gates into 67/69; drop the 1M-row SQLite benchmarks** | VAL-01 (determinism matrix), VAL-04, VAL-08 (cache status/prune covers summary store), VAL-09; drop VAL-03 WAL/checkpoint kill tests and VAL-07 row-scale benchmarks unless SQLite is actually adopted | Closeout gates belong to the phases that produce the artifacts. |

Honesty (priority 3) is already partially durable: unknowns are typed and surfaced via `polint unknowns` (`cli/mod.rs:190-191, 310-322`, `docs/schemas/polint-unknowns-v1.json`) and `FlowAnswer.unknowns`/`is_complete` (`sdk/dataflow.rs:329-345`). What is missing is *persisting* budget/unknown state with summaries so a warm run reports the same unknowns as cold (REV-03) and `polint graph` can say `budget_exceeded` without recomputing.

## 3. Phase 67 keystone: minimum design

**Unit** = Go package (sidecar `unit`), TS module later. **Summary** = the per-(function, entry) taint summary exits plus the per-unit call-fact slice (sites/targets/refined edges/unresolved for functions in the unit) plus the unit's recorded unknowns/budget events.

Layout under `.polint/cache/summaries/` (file-based, same conventions as `layer_cache.rs:971-990` temp-then-rename):

```
summaries/
  index.json                 # manifest: schema, polint version, sidecar identity, models digest,
                             #   units: { unit_id -> { input_digest, deps: [unit_id], payload: blake3 } }
  payloads/<blake3>.json     # content-addressed unit summary (call slice + taint exits + unknowns)
```

- `unit input_digest` = hash of (sorted file digests of the unit from `InputSnapshot.files`, lifecycle digest, config digest, models digest, sidecar identity, schema). Typed wrapper `SummaryPayloadDigest(blake3)` distinct from `Digest` so SUM-02's confusion rule holds.
- `deps` = the unit's import edges (already in `ModuleGraphFacts`) plus callee units seen in summary exits.
- **Frontier**: given the changed-file set (`review` already has `ChangedFiles`, `runner/mod.rs:450-452`), map files → units (`FlowFunction.file`/`unit`), mark those dirty, then walk the reverse `deps` index to transitive dependents; everything else is `Reuse`. This is the existing `InvalidationPlan` algebra (`incremental/invalidation.rs`) applied to unit nodes; add a `CacheNode::Summary` row per unit (`dependency_index.rs:11-19` already has the variant).
- **Plug-in points:** (a) `CallCacheState::decide` (`analysis_kernel/mod.rs:531-565`) gains a partial-hit branch: restore the call slices of reused units and run the calls layer only for frontier units; (b) `Solver::new` (`taint/solver.rs:483-500`) is seeded with persisted `summaries` for non-frontier `(FnId, Entry)` keys marked `done`, so `compute` only runs for frontier keys; (c) persisted unknowns are merged into `FlowAnswer.unknowns` so REV-03 parity holds.
- **The sidecar must emit per-unit output** (export-data load is already in place: `load_mode_test.go:12`; VTA is whole-program at `typed.go:189`). The sidecar needs a `--units <changed packages>` mode that reuses persisted per-package typed facts and runs VTA only over the frontier closure. Without this, the Rust side wins ~0 on edit latency. This is the real risk item of the keystone and should be its first spike.

**SQLite or files?** For the first win, **files + index.json suffice**: one writer per run (the kernel), atomic rename per payload, pruning by GC of unreferenced blake3 payloads at `polint cache prune`, no cross-process query. Trade-offs honestly: concurrent `polint` runs can race on `index.json` (last writer wins; mitigate with the existing sidecar-style lock file pattern, `go/semantic/process.rs:427-443`); crash-safety is per-file atomic, not transactional across index+payloads (a missing payload must be treated as a miss, like `call_cache::restore:216-230`); there is no indexed query. SQLite earns its place only when `polint graph` wants cross-run queries without re-running analysis or when the index exceeds a few MB; at that point the existing generation lease and migrations (Phase 64/65 code) are the right home, and `index.json` becomes the `generations`+`unit_summaries` tables. Keep `rusqlite` dev-only until then.

## 4. Query engine and `polint graph`

Minimum commands: `polint graph callers|callees|used-by|path|reach|taint`, with `--max-depth`, `--max-paths`, `--min-precision`, `--include-tests`, `--path <glob>`, `--format json|text`, `--no-cache`. Envelope mirrors existing agent JSON (`version`, `schema`, `tool`, as in `polint-unknowns-v1.json`) plus `command`, `query`, `status`, `precision`, `nodes`, `edges`, `paths`, `unknowns`, `budgets`, `summary`. Status vocabulary: `complete | partial | not_found | unknown | budget_exceeded | unsupported | setup_missing`, derived from `FlowAnswer::is_complete`, `FlowUnknown` variants and the capability blocker set (`runtime_capability_blockers`, `analysis_kernel/mod.rs:911-915`).

Already there: `CallGraph::callers/callees/reachable/paths` with `CallEdgePrecision` filters (`sdk/call_graph.rs:43-60, 101-125, 167-239`), `DataFlow::flows` with path steps and SARIF code flows, `References<'_>` for used-by, `polint facts sample`/`polint unknowns` as envelope precedents, `polint explain` for capability planning. Days: envelope struct, status derivation, `callers/callees/used-by/reach` over SDK views, JSON schema snapshot, leak-gate extension (`tests/public_surface_leak.rs`). Weeks: `path` and `taint` with cycle/barrier semantics and bounded ranking (QUERY-05), and the TS side where the call cache does not apply (`call_cache.rs:21-22`).

## 5. Recommended PR sequence (input to the roadmap)

1. **Planning truth-up**: record R3–R5a done, cut R5b/66/70, fold 71 gates, re-state the latency gate as "one-file edit `review` ≤ 15 s calls / ≤ 20 s dataflow on OAIZ core".
2. **Sidecar per-unit spike**: `--units` mode emitting per-package typed facts/flow programs and VTA over a closure; persisted per-package NDJSON. Gate: edit-tier sidecar time < 5 s on OAIZ core, byte-identical whole-program output when all units are requested.
3. **Summary store + frontier (Rust side)**: `summaries/index.json` + blake3 payloads, unit dependency index, frontier computation, partial call-cache restore, solver seeding. Gate: must-recompute/must-reuse fixtures (VAL-04 classes), warm review byte-identical to cold (REV-03), recompute set instrumented in `KernelRunReport`.
4. **Warm `polint review` measurement**: wire into `polint-eval` bench (`harness/bench/gate.rs:160`) with a frontier benchmark. Gate: one-file edit ≤ 15 s / ≤ 20 s on OAIZ core, RSS within +20% of baseline.
5. **Query envelope + `polint graph callers/callees/used-by/reach`**: in-memory services over SDK views, schema snapshot `polint-graph-v1.json`, leak gate. Gate: fixture matrix from PITFALLS must-test rows, docs under `docs/facts/`.
6. **`polint graph path/taint`** over persisted summaries with budgets and barriers. Gate: taint corpus parity with `DataFlow::flows` (53/54 cases), `budget_exceeded` surfaced from persisted unknowns.

Only after PR 6, decide with measurements whether `index.json` moves into the SQLite store (reviving STORE-04/05 and VAL-03/07/08 for it).

## Facts verified in code

- Store test-only and disabled in production: `crates/polint/src/analysis_kernel/store/mod.rs:6-37, 175-183`; `crates/polint/Cargo.toml:80-87`; `crates/polint/src/cache/mod.rs:118-166`; `crates/polint/src/analysis_kernel/mod.rs:916-932`.
- Store tables and schema v5: `crates/polint/src/analysis_kernel/store/migrations.rs:8, 42-86`; lease/connection policy: `store/connection.rs:43-101, 156`; generation lifecycle: `store/generation.rs:195-495`.
- Phase 65 plans 01–06 complete on main: `.planning/phases/65-*/65-0{1..6}-SUMMARY.md`, `65-0{3,5,6}-VERIFICATION.md` (`status: passed`), PR #94.
- Layer cache manifest/key/invalidation: `incremental/layer_cache.rs:41-55, 168-230, 971-990`; `incremental/keys.rs:52-64, 112-121`; `incremental/dependency_index.rs:11-56`; `incremental/invalidation.rs:20-56`.
- Call-resolution cache whole-snapshot key and restore: `analysis_kernel/call_cache.rs:21-22, 43, 97-116, 140-160, 216-230`; `analysis_kernel/mod.rs:531-565`.
- Sidecar whole-run cache and VTA: `go/semantic/client.rs:106-135`; `go/semantic/cache_key.rs:32-47`; `go/semantic/provider.rs:518`; `go-sidecar/polint-go-frontend/internal/semantic/typed.go:189`; `load_mode_test.go:12`.
- Taint summaries in memory, per unit budgets: `analysis_neutral/taint/solver.rs:235, 313-321, 452-500`; `taint/index.rs:251-262`; `taint/ir.rs:155-162`.
- SDK query surfaces: `sdk/call_graph.rs:43-60, 101-239`; `sdk/dataflow.rs:215-345`; `sdk/facts.rs:469-549, 1024`; CLI/envelopes: `cli/mod.rs:174-202, 264-322`; `docs/schemas/polint-unknowns-v1.json`.
- Regression gate code: `crates/polint-eval/src/harness/bench/gate.rs:84-224`.
