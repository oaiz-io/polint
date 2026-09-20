# Plan: TypeScript type sidecar as a typed call-graph tier

Date: 2026-09-17
Reads with: [`research/strategy/02-gap-analysis.md`](../strategy/02-gap-analysis.md)
gap item 2, [`03-build-plan.md`](../strategy/03-build-plan.md) Stage 1,
`research/static-analysis-2.0/OPEN-QUESTIONS.md` Q20 to Q23.

## 1. Conclusion first

polint gets a type-directed TS/JS call-graph tier by copying the Go semantic
sidecar architecture one-for-one and swapping the toolchain: a Node process
running the TypeScript compiler API emits NDJSON rows for projects, callables,
call sites, callees, receiver types and per-file `any` density; a new
`polint.ts.types` provider lowers those rows into a fact store; and a new
`ts_types` refinement in `polint.refined_calls` turns them into
`refined_call_edges` on a new `TypeDirected` tier that ranks above the
function-token and points-to tiers. The Andersen path is untouched and remains
the fallback for every site types cannot answer.

The Go tier is the template because it already solved every problem this one
has: an out-of-process toolchain that may be absent, a wire protocol that must
fail as a typed error rather than a panic, a cache key that must fold the
toolchain version, and a scope list that keeps the sidecar from emitting rows
the kernel would drop.

## 2. What the Go tier does, verified against the tree at 82a3c129

| Concern | Go implementation | Evidence |
|---|---|---|
| Sidecar source of truth | `tools/polint-go-symbols/`, with a copy under `crates/polint/src/go-sidecar/` embedded by `include_str!` | `go/semantic/process.rs:18-35` |
| Materialization | embedded sources written to a private per-user cache dir, then `go build`-ed on demand behind a receipt-verified immutable binary cache | `go/semantic/process.rs:257-393`, `go/embedded_cache.rs` |
| Process bounding | `run_bounded` with a fresh process group, cancellable non-blocking readers, and process-tree kill on timeout | `go/process_runner.rs:39` |
| Scope narrowing | client writes a newline-delimited `--scope-files` temp list so the sidecar never emits rows the kernel would drop | `go/semantic/client.rs:149-214` |
| Raw-output cache | `run_cached` persists the NDJSON under `go_semantic_sidecar_cache_key(sidecar_digest, go_version, upstream_digest, lifecycle)` | `go/semantic/cache_key.rs:32` |
| Protocol discipline | NDJSON framed `session_begin` / `phase` / rows / `session_end`; unknown kind, row-before-begin, row-after-end and missing terminator are typed errors | `go/semantic/protocol.rs` |
| Store resilience | duplicate structural keys collapse keep-first, invalid harvest rows drop with counted diagnostics, rather than zeroing the fact set | `go/semantic/store.rs:340-380` |
| Failure policy | provider failure is always diagnostic + empty store + `ProviderExecution::Failed`, never a panic | `go/semantic/provider.rs:259-320` |
| Join back to native facts | sidecar callsites and functions are matched to `CallSiteFact` / `FunctionFact` by `FileId` + byte span, with a caller tie-break | `analysis_neutral/refined_calls/provider.rs:360-455` |

## 3. Where the TS tier slots in

`polint.refined_calls` already consumes `solver_derived_edges` plus Go semantic
rows and emits `refined_call_edges` carrying a tier enum
(`analysis_neutral/refined_calls/facts.rs:35`: `DirectOnly`,
`DirectPlusFramework`, `TypeValueFunctionToken`, `SummaryAssisted`,
`PointsToAssisted`, `ExtensionModel`, `AllAccepted`).

Today TS reaches that provider only through `refined_calls/ts_js.rs`, which
emits `PointsToAssisted` low-confidence rows for sites the solver left
unresolved. The typed tier is:

1. a new `RefinedCallTier::TypeDirected` variant, ordered above
   `TypeValueFunctionToken` and `PointsToAssisted` so tier comparison ranks it
   higher, with labels added in `analysis_neutral/host.rs:1401-1409` and
   `core/labels.rs:270-279`;
2. a new `refined_calls/ts_types.rs` refinement that turns lowered sidecar
   rows into edges;
3. no change to `ts_js.rs`: a site the typed tier resolved still produces its
   points-to row, and consumers pick by tier. Deleting the fallback would make
   the tier a dual path that cannot be measured against itself.

## 4. Plumbing shape

Mirrors Go one-for-one.

- **Provider.** `polint.ts.types` in `PROVIDER_MANIFESTS`
  (`analysis_kernel/provider.rs`), placed immediately after
  `polint.go.semantic` so the declared-order list and the topological schedule
  stay in the same relative order; `run_named_provider`,
  `providers_enabled_by_boolean_gates`, `providers_enabled_by_capability_closure`
  seeds and `outcome::hard_dependencies` all gain the id. Inputs are
  `source_files`, `functions`, `call_sites` and the `ts.*` lifecycle keys;
  output is the `ts_type_*` fact vocabulary consumed by `polint.refined_calls`.
- **Fact family.** `FactFamily::TsTypes` (+ label) in
  `analysis_api/metadata.rs`, store registration in `core/db.rs`, following
  `FactFamily::GoSemantic`, which is a store registry key rather than a
  per-row metadata family.
- **Module tree.** `crates/polint/src/ts/types/{mod,lifecycle,protocol,process,
  client,cache_key,facts,lower,store,validate,provider,diagnostics}.rs`, all
  `pub(crate)`.
- **Lifecycle.** Reads the existing free-form `[languages.ts]` map
  (`config/mod.rs:180-186`), which already feeds the cache key and the input
  snapshot through `ts_js.resolver_options`, next to `ts_js.config_files`,
  which already digests `tsconfig.json` `extends` chains
  (`analysis_kernel/incremental/input_snapshot.rs:85-130`).
- **tsconfig discovery.** Per Q23, topology stays in `ts/module_graph`: the
  lifecycle walks from each discovered TS file to its nearest `tsconfig.json`
  exactly as `nearest_tsconfig_path` does, instead of adding a config flag.
- **Cache.** `ts_types_sidecar_cache_key(sidecar_digest, typescript_version,
  upstream_digest, lifecycle)` keys the persisted raw NDJSON in the shared
  sidecar cache dir; `ts_types_input_digest` folds the same inputs into the
  provider output digest.

### 4.1 Extraction prerequisite

`run_bounded` lived in `go/process_runner.rs` and hardcoded
`GoSubprocessTimeout` in its timeout message. A TS sidecar must not depend on
`crate::go` and must not report a Go timeout code, so the first commit moves
the runner to a neutral `crate::subprocess` parameterized by timeout code, and
moves the private-cache materializer alongside it parameterized by the
sidecar-family directory name. `go::process_runner` and `go::embedded_cache`
remain as thin re-export adapters so the Go call sites are unchanged. That
commit is structure-only: no behavior change, no new tests, existing tests
green.

## 5. Wire protocol

`polint-ts-types-1`, NDJSON, same framing discipline as the Go tier. Full
contract in [wire-protocol.md](wire-protocol.md). Row kinds:

| Kind | Carries |
|---|---|
| `project` | tsconfig path, compiler-options digest, TypeScript version |
| `callable` | declaration identity, file, span, enclosing project |
| `callsite` | span, enclosing callable, call kind, resolution status |
| `callee` | in-scope callable id, or an external moniker for `node_modules` / `lib.*.d.ts` so declarations outside scope never cross the wire |
| `receiver` | printed type capped in length, `is_any` / `is_unknown`, union size |
| `any_density` | per-file unknown-receiver ratio for the Q22 gates |
| `diagnostic` | sidecar-side setup and capability problems |
| `phase` | stage timings and workload sizes for the run report |

`--scope-files` bounds **emission**, not the program: type resolution still
needs the full import closure, so the sidecar builds the whole program and
filters rows on the way out. That is exactly the Go `scope_files` contract.

## 6. `any` density gates (Q22)

Q22 decided that `any` is never a high-confidence typed edge. As implemented:

- a receiver row with `is_any` or `is_unknown` never produces a typed edge;
- a file at or above 25% unknown/`any` receivers is marked degraded and its
  typed edges drop to medium confidence;
- a file at or above 50% defers its non-exact receiver sites to the field and
  heap tiers entirely, while exact non-`any` sites in the same file still
  produce typed edges.

The thresholds are constants in `ts/types/lifecycle.rs` with the file-level
ratio computed by the sidecar, so the gate is a property of the measured file
rather than a per-call guess.

## 7. TypeScript acquisition

Pre-approved by the founder; recorded here because it is a one-way door for
users (it decides what a scan requires on `PATH`).

Resolution order:

1. `POLINT_TS_TYPESCRIPT` — an explicit path to a `typescript` package
   directory or its `lib/typescript.js`.
2. The analyzed repository's own `node_modules/typescript`, resolved from the
   nearest `node_modules` walking up from the project root.
3. A global install (`npm root -g`).

Using the repository's own compiler means matching type semantics with what the
repository itself compiles, no install step, and no network at scan time.
Nothing found, or major version >= 7, produces a `polint/capability`
diagnostic and the heap tier stands alone. `typescript_version` folds into the
cache key so an upgrade invalidates stored rows.

The sidecar is authored as `// @ts-check` JSDoc-typed CommonJS rather than
TypeScript that needs `tsc`, so there is no build step at scan time and no
checked-in build artifact. This deviates from the Go tier, which compiles its
sidecar on demand; for Node the compile step would be the only reason to need a
compiler for the compiler, so it is removed rather than mirrored.

## 8. Failure policy

Every one of these produces a diagnostic and an empty store, and the Andersen
tier answers the repository alone:

- no TypeScript module found anywhere in the resolution order;
- TypeScript major version >= 7 (no stable programmatic API before 7.1);
- `node` absent from `PATH`;
- sidecar timeout (own budget, `POLINT_TS_TYPES_TIMEOUT_MS` override);
- non-zero exit, unparseable NDJSON, schema mismatch, missing terminator;
- no `tsconfig.json` anywhere under the analyzed roots.

None of them panics, none of them fails the run, and none of them silently
produces zero edges without a diagnostic.

## 9. Risks and kill criteria

| Risk | Mitigation | Kill criterion |
|---|---|---|
| The TypeScript program build dominates scan time on large repos | one program per tsconfig, reused across every file in that project; raw NDJSON cached by content digest; sidecar skipped entirely when no rule requests `calls`/`dataflow` | typed tier cold cost above the Andersen tier's total on a 20k-line TS repo without a recall gain to pay for it |
| Node/TypeScript absence makes the tier invisible in practice | repo-own compiler first, so any repo that can typecheck itself can run the tier; explicit capability diagnostic otherwise | none: the fallback is the status quo |
| Typed edges wrong where `any` leaks | Q22 density gates; `is_any` never produces an edge | precision on the Jelly lane drops below the committed floor |
| Span join to native call sites fails and edges are dropped silently | join by `FileId` + byte span with a caller tie-break, mirroring Go; unmatched rows counted and reported as a provider counter | unmatched fraction high enough that the tier cannot be measured |
| TypeScript 7 changes the API under us | version gate refuses >= 7 rather than guessing | revisit when 7.1 ships a stable API |

## 10. Where the implementation deviated from this plan

Recorded because the plan was written first and reviewed as a design, so the
differences are the interesting part.

| Plan said | Shipped | Why |
|---|---|---|
| Sidecar source of truth in `tools/`, embedded copy under `crates/` | One copy, `crates/polint/src/ts-sidecar/polint-ts-types/index.js` | The Go tier's two copies have no sync test between them and `go.work` already references a path that no longer exists. One copy cannot drift |
| Extraction commit moves `run_bounded` | It also moves the embedded source cache, parameterized by a sidecar family | The TS sidecar needs the verified private cache too, and reaching into `crate::go` for it is exactly what the layering test forbids |
| Provider emits a setup diagnostic when the tier cannot run | Quiet unless `.polint.toml` names the tier; the provider row still reports zero rows either way | A JS repository with no TypeScript install is not misconfigured. This mirrors the Go tier, which only reports missing module roots when module roots were configured |
| Receiver type per call site drives the tier | Receiver type **plus** a rapid-type expansion over instantiated classes | Measured: `getResolvedSignature` answers an interface-typed call with the interface's own method signature, which has no body. Without the expansion the tier resolves nothing for exactly the dispatch it exists to resolve |
| `refined_calls` gains `polint.ts.types` as a dependency | It gains the fact inputs but **not** a hard dependency | Listing it in `hard_dependencies` made a failed sidecar block the whole refined-call provider, which is the opposite of a fallback |
| Reuse `module_graph` nearest-tsconfig machinery | `nearest_tsconfig_path` promoted to `pub(crate)` and called directly | Copying the walk would be a second notion of where a project starts |
| L4 capability probes for the typed tier | Not added; dedicated end-to-end tests instead | A `tsconfig.json` in the probe repository would make the certification gate answer differently on hosts with and without a TypeScript install |

## 11. Decisions taken

1. Node + TypeScript compiler API now; `typescript-go` only after a stable
   programmatic API exists (Q20).
2. Batched per-project dump, not per-call-site RPC and not one whole-program
   type table (Q21).
3. `any` is never a high-confidence typed edge; density gates at 25% and 50%
   (Q22).
4. `ts/module_graph` keeps ownership of topology and tsconfig discovery; the
   sidecar consumes scoped project units (Q23).
5. The sidecar is JSDoc-typed CommonJS with no build step.
6. The Andersen tier is kept, not shadowed or retired: Q27 requires a tier to
   shadow a behavior with equal-or-better benchmark results before the old one
   is retired, and that evidence is exactly what this PR's measurement
   produces for the first time.
