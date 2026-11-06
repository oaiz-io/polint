# Deep Go analysis in the one-file-edit tier

**Researched:** 2026-11-02 against `main` at `338b570b` (v0.5.0). Target: a one-file edit on the OAIZ `core` benchmark re-answers deep questions in ≤ 15 s (calls) and ≤ 20 s (dataflow). Today: 41.7 s and 68.2 s, with a whole-program variable-type-analysis (VTA) floor of 15.8–30 s inside the Go semantic sidecar.

## Facts verified in code

- Sidecar driver: `crates/polint/src/go-sidecar/polint-go-frontend/internal/semantic/emit.go:206-366` (phases `packages_load`, `ssa_build`, `emit_rows`, `type_facts`, `routes`, `call_graph`, `flow_bodies`); load mode roots-from-source, deps from export data (`emit.go:371-382`); `ssautil.Packages` unless a `main` root forces `AllPackages` (`emit.go:395-403`).
- VTA: `typed.go:184-194` runs one whole-program `vta.CallGraph(ssautil.AllFunctions(prog), nil)`; `nil` initial means x/tools builds its own CHA via `chautil.LazyCallees` (`x/tools@v0.50.0/go/callgraph/vta/initial.go:20-22`). `typed.go:298-345` builds a *second* CHA index (`hierarchyCallees`) over the same function set for the fallback. VTA internals are whole-graph Tarjan SCC + propagation in reverse topological order (`vta/propagation.go:25-77,121-155`); no per-package or incremental API, all unexported.
- Flow programs embed VTA candidates per call statement (`flow.go:476`), so flow bodies are VTA-dependent today.
- `--scope-files` filters rows *after* computation (`emit.go:462-494`); it never narrows the program.
- Sidecar NDJSON cache key = frontend digest + Go version + **Go-syntax output digest (whole scan)** + lifecycle digest (`go/semantic/cache_key.rs:35-50`; `go/semantic/provider.rs:518`). One edited file changes the key, so the whole sidecar reruns.
- Call-resolution cache key = serialized `InputSnapshot` (every file digest) + sidecar identity + models + version (`analysis_kernel/call_cache.rs:107-128`); it stands in for 15 providers (`call_cache.rs:50-66`); a data-flow plan still runs the sidecar provider (`call_cache.rs:87-91`).
- No deep Rust provider writes a layer: grep for `analysis_cache()|write_layer|LayerKey` under `analysis_neutral/{semantic_mir,cfg,calls,refined_calls}` and `go/semantic` returns nothing. Persisted layers are syntax, module graph, symbol graph, metrics and the sidecar NDJSON.
- Taint solver: demand-driven per question, 48-frame cap, per-package step budget and deadline (`analysis_neutral/taint/solver.rs:167-184`); `Summary` already records `dependents: BTreeSet<SummaryKey>` (`solver.rs:313-320`), a ready-made hook for an invalidation frontier.
- Harness: `scripts/deep-gate/probe.sh` (cold, and warm via `KEEP_CACHE`), `gate.sh` (dense-id sweep, provider-set check, digest oracle, fact-row oracle), `scripts/bench-runner/bench-run.sh`. There is **no edit cell**; the edit measurements were done by hand. No `watch`/daemon command exists.
- Design doc `thoughts/perf-5.0-levers/13-deep-analysis.md` lives only on `origin/research/deep-analysis` (§4.4 cache table, §5 step 2 shards, Appendix A numbers).

## 1. What dominates a one-file edit today

The edit tier is indistinguishable from cold: calls 41.7 s edit vs 43.5 s cold vs 5.7–5.9 s warm (step 2 summary); dataflow 68.2 s edit vs 70.0 s cold vs 15.0 s warm (step 3). Nothing deep is reused because both cache keys above are whole-scan.

| Stage | Calls edit | Dataflow edit | Evidence |
|---|---|---|---|
| Sidecar `packages_load` (export data; build cache recompiles only dependents) | ~3.3 s | ~3.3 s | design doc §2.2 table, "after a one-file edit" row |
| Sidecar `ssa_build` + constant-branch pruning | ~1 s | ~1 s | same table; `prune.go:21` |
| Sidecar `call_graph` (VTA + its lazy CHA + duplicate CHA index) | **15.8 s floor** | part of 26–30 s | step 2/3 summaries; design-doc harness measured VTA alone at 5.8 s and CHA at 20.8 s |
| Sidecar `emit_rows`/`type_facts`/`routes`/`flow_bodies` + JSON encode | few s | ~10 s (flow bodies) | 26–30 s dataflow floor minus the calls floor |
| Rust side: NDJSON decode, MIR/CFG per file, calls join, SCC fixpoint, identity, refined calls | ~26 s | ~30 s | 41.7 − 15.8; nothing persisted |
| Taint solver, five questions | — | ~9 s | warm dataflow 15.0 s − warm shallow ~5.9 s |

Two things are worth stating plainly. First, the VTA "floor" of 15.8 s is 2.7× the 5.8 s the same algorithm took in the research harness on the same program; the sidecar runs under a soft `GOMEMLIMIT = min(available/4, 2 GiB)` (`client.rs:369`) against a program whose export-data load already holds 1.5–1.8 GB, so GC pressure during propagation is the leading hypothesis, with the duplicate CHA index second. Second, the Rust side at ~26 s is larger than the sidecar floor and is entirely recomputation of unchanged units.

## 2. Options

### (a) Per-package sidecar shards keyed by export-data digests

Feasible for every row family except the two whole-program ones. Package rows, functions, static call sites, dead calls, type facts, method sets, params and flow bodies are functions of one package's source plus the export interfaces of its imports, exactly the Go build cache's rule. `go list -export -json` already yields each package's `Export` file, whose content hash is the interface digest. Routes (walk from `main`) and VTA candidate lists are whole-program and stay as one "dynamic" artifact.

Required change: flow bodies must stop embedding callees (`flow.go:476`) and carry the call-site key instead, with candidates joined from `call_edges` rows on the Rust side (the calls layer already joins by span). Then the per-package shard is VTA-independent and a shard cache keyed on (package source digest, import export digests, sidecar digest, lifecycle digest) replaces the whole-scan NDJSON key. On an edit the sidecar still loads the program (3–4 s) and builds SSA (1 s), but emits and polint decodes only the changed shards plus the dynamic artifact.

**Modular VTA is not feasible as-is.** x/tools VTA labels a single global propagation graph; a function's labels depend on types reaching its parameters from every caller and flowing back through returns, so a body-only edit in package P can change candidate sets in P's callers *and* callees. There is no boundary summary; the internals are unexported. What *is* feasible is incremental propagation (see d4). RTA/CHA-first with VTA only on the frontier does not give a consistent graph: the untouched sites would carry last run's VTA answers, which breaks "edit run == cold run" (REV-03, gate "byte-identical reports").

### (b) Persist per-unit call-resolution and data-flow summaries; recompute only the frontier

This is the Phase 67 invalidation frontier applied to the call layer. The foundations from step 1 exist: per-file parallel MIR lowering with interner overlays folded in file order, compact identities, exact SCC fixpoint, indexed joins. What was explicitly deferred in step 2 ("per-unit binary shards … not built") is the persistence: unit (package) shards for MIR/CFG/call sites/derived rows keyed on the unit's file digests + its sidecar shard digest + provider version + config digest, and a call-resolution cache keyed per package rather than per `InputSnapshot`. Unchanged units restore; changed units and units whose sidecar shard changed recompute; the cross-unit edge merge is a sort. This removes most of the ~26 s Rust-side cost. For data flow, persist `(function, entry)` summaries keyed on flow-body digest + callee summary digests; the solver's `dependents` set gives the transitive recompute set, and equality backdating (already implemented for cycles) stops propagation. Caveat: on the bench every whole-core answer is incomplete (`CallDepth` unknowns), and budget-truncated summaries must be persisted with their unknowns so a restored answer is byte-identical to a recomputed one; the budget counters have to be per-unit and deterministic, not deadline-based, for the persisted path.

### (c) Long-lived sidecar daemon

What it would take: a `--serve` loop in the sidecar reading edit requests, a Rust client keeping the process across runs, and a `polint watch`. What it saves: process start, export-data load (~3 s) and JSON encode/decode. What it does not save: `go/packages` has no incremental reload and `ssa.Program` cannot replace a package, so the program is rebuilt anyway (~4–5 s) and whole-program VTA still runs. Determinism risk is real: `emit.go:287-330` documents that emission depends on the program's runtime-type set growing during the run; a warm program carries instantiation wrappers from earlier sessions, so rows can differ from a fresh run. The sidecar run identity would no longer be a function of inputs alone; it would need a recompute-and-diff gate and 1.5–4 GB permanently resident. Verdict: a latency polish after (a)/(b), not a lever for the 15 s target.

### (d) Cheaper levers inside the sidecar

1. Measure the `call_graph` phase split (CHA index, graph build, SCC, propagate, construct) and GC CPU share; raise the memory limit for that phase if GC dominates. Potential: 15.8 → ~6–8 s on the harness evidence.
2. Compute CHA once (`cha.CallGraph`) and pass it as `initial`; drop the duplicate `hierarchyCallees` index.
3. Restrict `funcs` to functions reachable from the root packages' entry set (RTA-style pruning before VTA); VTA cost is linear in graph size; sound as long as the pruned set is the same set the rules see.
4. Incremental VTA: vendor the ~1.5k-line VTA package (BSD), give nodes stable names (function name + instruction ordinal; generic instances already canonicalised by `names.go`), persist per-SCC type sets keyed on SCC content digest + predecessor-set digests, repropagate only SCCs reachable from changed nodes with equality backdating. Graph construction still needs all bodies (SSA, ~1 s). This is the only path that makes VTA proportional to the edit; it is research-grade (3–5 weeks) and must ship behind a recompute-and-diff gate on every corpus.

## 3. Gain, risk, effort, invariants, gates

| Option | Gain vs target | Risk | Effort | Must stay byte-identical | Reusable gates |
|---|---|---|---|---|---|
| (d1–d3) sidecar tuning | VTA 15.8 → ~6–8 s; edit → ~30 s / ~55 s | Low; GC hypothesis may be partial | 3–5 days | call_edges rows (140,175 edges at 1/4/6 jobs), `polint unknowns --cap calls` | `gate.sh` digest + fact-row oracle, step-1 edge-identity method |
| (a) per-package sidecar shards | sidecar edit ≈ load+SSA+VTA+changed shards; decode cost ∝ edit | Medium: flow-body/edge split changes the wire schema; export-digest correctness | 2–3 weeks | all sidecar rows after join; `go_types`, `routes` views | fact-row dump (`eval::fact_rows_dump`), outside-user CLI tests, sidecar Go tests |
| (b) Rust per-unit shards + frontier | Rust side ~26 s → frontier only (seconds); calls edit ≈ 5 + VTA | Medium-high: stable-key compactness (gate-0 probe: 8.3 GB of key text before compaction), provider_version cold-starts | 3–4 weeks | reports at 1/4/6 jobs and ten permutations; provider digests | `cli.rs` jobs/permutation tests, call-cache mutation tests, must-recompute/must-reuse fixtures (to write) |
| (d4) incremental VTA + persisted taint summaries | dataflow edit → ≤ 20 s plausible; calls ≤ 15 s | High: node identity, generics, equality across runs | 3–5 weeks | VTA edge set vs whole-program VTA; taint corpus 0.946/1.000; L4 10/10 | `tests/taint-corpus`, recompute-and-diff |
| (c) daemon | −5 to −8 s per edit | High determinism risk, memory residency | 2 weeks | everything; needs new diff gate | none today |

## 4. Recommended sequence

**PR 1: instrument and tune the sidecar; add the edit cell.** Sub-phase rows for `call_graph` (CHA index, graph, SCC, propagate, construct, GC CPU) in the stage log; single CHA passed as `initial`; GC-limit experiment; reachable-function pruning behind a flag; `probe.sh` gains an `EDIT=<patch>` cell that applies a one-file change and runs warm against `KEEP_CACHE`. Gate: call_edges and `polint unknowns --cap calls` byte-identical to v0.5.0 on OAIZ core; VTA phase ≤ 8 s; the edit cell is reproducible by the harness.

**PR 2: split the sidecar output.** Flow bodies carry site keys, callees joined from `call_edges`; per-package static shards keyed on (source digest, import export digests, sidecar, lifecycle); the dynamic artifact (call_edges, routes) keyed as today. Gate: on the edit cell, shards emitted/decoded = changed package + packages whose import interface changed (instrumented count); all joined facts byte-identical to cold; sidecar Go tests and the outside-user CLI tests green.

**PR 3: Rust per-unit shards and a per-package call-resolution cache.** Unit shards for MIR/CFG/call sites/derived rows; frontier = changed units + units with a changed sidecar shard; cross-unit merge by sort. Gate: calls edit ≤ 15 s on OAIZ core at 4 jobs; must-recompute/must-reuse fixtures; reports byte-identical at 1/4/6 jobs and ten permutations; warm no-change unchanged (≤ 6 s).

**PR 4: incremental VTA and persisted taint summaries.** Vendored propagation with per-SCC persisted labels and equality backdating; `(function, entry)` summaries persisted with their unknowns, dependents-driven invalidation. Gate: VTA edge set identical to whole-program VTA on OAIZ core and the taint corpus after every edit fixture; dataflow edit ≤ 20 s; taint corpus precision/recall unchanged; `is_complete` and unknown counts byte-identical between restored and recomputed answers.

If PR 1 shows propagation itself (not GC or the duplicate index) is the 15 s, PR 4's incremental VTA moves ahead of PR 3, because no amount of Rust-side sharding gets calls under 15 s with a 15 s sidecar floor.
