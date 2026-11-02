# TS/JS deep-analysis parity with Go

**Researched:** 2026-11-02 against `main` at `338b570b` (v0.5.0). Read-only code research; every claim carries a path.

## 1. What exists today

**Go reference pipeline.** One sidecar binary (`crates/polint/src/go-sidecar/polint-go-frontend/main.go:20-32`) takes `--call-graph`, `--routes`, `--dataflow`, `--route-models`, `--scope-files` and emits NDJSON `polint-go-semantic-4` rows: `callsite` (`emit.go:718-750`: `static_callee`, `receiver_type`, `via_value`, `mode`, `status`, `reason`), `dynamic_dispatch`/call-edge rows (lowered to `GoSemanticCallEdgeFact` at `go/semantic/facts.rs:247-283`: `algorithm` VTA/CHA, candidate `callees`, `abstract_callee`, `candidates` count), `route`/`route_serve`/`route_budget` (`routes.go:1494-1550`), and `flow_body`. The flow body schema is documented at `flow.go:13-60`: ops `copy|load|addr|store|alias|alloc|closure|call|ret`, path steps `f:Name`, `[]`, `*`, `r:N`; operands are slot indices or `-1-k` constants; `classes` is one char per slot (`c` context, `b` bool, `n` number). The Rust IR it lowers into (`analysis_neutral/taint/ir.rs:16-27, 95-160`) adds `Step::Capture(FnId,u16)` for closure free variables, `Callee::{Body,External(String)}`, `Algorithm::{Static,Vta,Cha,Unknown}`, `CallMode::{Call,Go,Defer}`, and `FlowFunction { name, unit, file, params, param_types, free, results, names, globals, consts, classes, stmts }`. The solver (`taint/solver.rs`) is summary-based with per-unit step budgets and a 120 s deadline (`flow_queries.rs:20-23`).

**TS today.** Three layers, none producing flows:
- Oxc syntax/semantic facts (`docs/facts/ts-js.md`: "These facts do not use TypeScript type checking").
- A Rust MIR lowering `ts/mir/lower.rs` (Oxc AST → neutral `MirOperationKind` at `ir/op.rs:21-58`; places carry `Property`, `IndexKnown`, `Deref`, `AwaitResult`, `CallReturn` projections at `ir/places.rs:53-62`; it handles `AwaitExpression` (line 1270) and spread (1323, 1336)). This feeds CFG/summaries, not the taint solver.
- The Jelly-style Andersen heap (`ts/points_to.rs`, `ts/object_model/`, `refined_calls/ts_js.rs`), producing `PointsToAssisted` refined-call edges.
- The type sidecar (PR #121): `ts-sidecar/polint-ts-types/index.js`, JSDoc CommonJS, no build step; requires `node` on PATH plus a TypeScript 4–6 install (TS 7 refused, `ts/types/process.rs:24-30`); resolution order env → `[languages.ts] typescript_path` → repo `node_modules/typescript` → global npm (`docs/CONSUMER-SETUP.md:120-133`). Config keys `type_sidecar`, `type_projects`, `typescript_path`, `type_timeout_ms` (`ts/types/lifecycle.rs:28-57`); default budget 300 s. Raw NDJSON cached under `ts_types_sidecar_cache_key(sidecar_digest, ts_version, upstream_digest, lifecycle)` (`cache_key.rs:40-60`). It emits `project`, `callable`, `callsite`, `callee` (with `dispatch` ∈ declared/declared_signature/implementation/union_member, external monikers `node_modules:<pkg>/<sub>#<name>`), `receiver` (printed type, `is_any`, `union_size`), `any_density`, `diagnostic` (`research/ts-type-sidecar/wire-protocol.md`). It is consumed **only** by `refined_calls/ts_types.rs` as a `TypeDirected` tier; `polint.calls` receives `TypedCallInputs` from Go alone (`analysis_kernel/provider.rs:355-363`), although `TypedCallInputs::new(Language)` is already language-neutral (`analysis_neutral/calls/typed.rs:31-72`).

**Measured numbers.** Sidecar on a 24k-line repo: 13.5 s cold (`create_program` 1.0 s, `walk_callsites` 12.3 s ≈ 1.55 ms/site), 42 ms warm; call sites with a resolved target 828 → 3,338 (`research/ts-type-sidecar/measurement.md:65, 149-160, 215-219`). Jelly micro lane: committed baseline recall 0.6646 / precision 0.9742 (`research/evaluation-harness/baselines/persisted-graph-accuracy.json`), measured 0.6619/0.9703/F1 0.787 in Sept, byte-identical with the typed tier on or off (`measurement.md:255-269`). The "F1 88.75%" in `performance/2026-06-17-jelly-fn-categorization-and-wins.md:3-4` is from the "release graph benchmark" on a June branch; the committed lane does not reproduce it, and that doc says ~42% of remaining FNs are Express object-model cases. **Treat 0.79 as the gated number and 0.89 as unverified.**

## 2. Semantic frontend options

What must be produced: `flow_body` rows (or the in-memory `FlowProgram`), per-site callee candidates, and route facts.

**TS construct → flow op mapping (common to all options)**
- Local binding → slot; reassignment → extra `copy` (flow-insensitive per variable; the solver's memory is already flow-insensitive, `docs/facts/data-flow.md:239`). Captured `let`/`const` → address-taken cell in `names`, `closure` bindings = cells, `Step::Capture`.
- `a.b`, `a?.b` → `load Field(b)`; `a[k]` → `load Elem`; `a.b = v` → `store [Field b]`; `{...}`/`[...]`/`new C()` → `alloc` (+ constructor call `(C).constructor` receiver-first so `is_method_name` at `solver.rs:1948` holds).
- Destructuring → sequence of `load Field`/`load Elem`; rest → `copy`. Spread args → `load Elem` then pass; object spread → `copy` into the alloc (over-approximate).
- `await x` → `copy`; async `return v` → plain `ret` (the solver ignores `CallMode`; `go`/`defer` are already treated as ordinary calls, so promises as "deferred calls" cost nothing). `p.then(cb)` → desugar to `call cb(p_result)`; `Promise.all/resolve` → propagator models.
- `this` → param 0; template literal/concat → `copy` (Go does the same for string concat).
- Calls: `callees` from the typed tier (`declared` → `static`, `implementation`/`union_member` → `cha`/`vta`, externals → `Callee::External(moniker)`); unresolved → `Algorithm::Unknown` (solver reports `UnresolvedCall`).
- `unit` = nearest tsconfig project or `package.json` dir; `globals` = module-level bindings `src/x.ts#name`.

**(a) Extend the Node sidecar** (`polint-ts-types-2` adds `flow_body`). Precision: best; the checker names external targets (`express#Request.query`) and parameter types for `param_types`/`classes` for free. Determinism: fine (rows sorted by file/offset; TS version in cache key). Cost: adds a bound-AST walk per function to a run that is already 13.5 s cold; flows become impossible without Node, so JS repos without a tsconfig get nothing. Maintenance: lowering logic in untyped JS with no Rust unit tests, two lowerings (Go in Go, TS in JS) to keep aligned with `ir.rs`.

**(b) Rust-native lowering from Oxc + oxc_semantic**, type sidecar only for callees. `ts/mir/lower.rs` already resolves scopes, awaits, spreads, destructuring and property places; a second pass from MIR (or directly from Oxc) to `FlowProgram` is mostly mechanical. Works with no Node (callees from symbol graph + Jelly heap, `Algorithm::Unknown` otherwise), deterministic, testable with Rust fixtures, no new wire schema. Weakness: without types, sinks/sources keyed on library types (`req.query`) cannot be matched by name alone; only call-shaped models (`db.query(...)`, `child_process.exec`) and import-resolved monikers match.

**(c) Hybrid (recommended):** (b) for the program; sidecar gains two cheap row kinds: `declared_type` for parameters/receivers (printed type + moniker, already computed for `receiver` rows via `checker.getTypeAtLocation`, `index.js:1284-1301`) and the callee rows that already exist. Rust joins by `(file,start,end)` like `TypedCallInputs`. Result: byte-identical program with or without Node; Node only raises callee precision and type-keyed model matching. Cost stays within the current sidecar budget because no new program walk is needed beyond parameters.

## 3. Route models for TS

The Go interpreter is models-as-data: roles `router|route|group|use|mount|subscriber|passthrough|serve` (`routes.go:19-74`), bounded abstract interpretation from `main`/`init`, struct fields and package variables as one location per field (`routes.go:392-403`). Ported to TS over the flow program:

| Framework | Fit to existing roles | Order |
|---|---|---|
| Express | `express()`/`express.Router()` = router; `app.get/post/...` = route (`method_from_name`, `path_argument 0`, `handlers_from 1`); `app.use(mw)`/`app.use(path, router)` = use/mount with **optional path arg** (new `path_optional`); `app.listen`/`http.createServer(app)` = serve | 1 |
| Fastify | `fastify()` router; `.get(...)`, `.route({method,url,handler})` (new `options_argument` reading object keys); `.register(plugin,{prefix})` = group with callback | 1 |
| Hono | `new Hono()`; `app.get`; `app.route(path, sub)` = mount; `app.use` | 1 |
| Next.js app/pages | file-path routes (`app/api/x/route.ts` exporting `GET`); needs a `file_pattern` role, no interpretation | 2 |
| NestJS | `@Controller('p')` + `@Get(':id')`, `@UseGuards` = decorators; sidecar already tags `call_kind: decorator`; new role `decorator_route`/`decorator_prefix` | 3 |
| tRPC | `router({ name: procedure.input().query(h) })`, object-literal keyed, transport `rpc`; needs chained-builder passthrough modelling | 4 |

`[[ts_route]]` keeps the Go field set (`go/route_models.rs:28-70`) with TS identities: `function = "express#default"`, `receivers = ["express#Router","express#Application"]`, `methods = ["get","post"]`, plus `path_optional`, `options_argument`, `file_pattern`, `decorator`. Transport gains `Rpc`.

## 4. Rust solver and SDK changes for language neutrality

- `db.go_flow_program()` and the Go-file gate in `flow_queries.rs:28-43` → `flow_programs()` per language or one merged program with `FlowFunction.language`; `FlowUnknown::NoProgram` text names "the typed Go frontend" (`sdk/dataflow.rs:373`).
- Models: `Target::matches`/`split_method` assume `(pkg.T).M` and `[T]` generics (`taint/models.rs:14-55`); keep the `(`-prefix method convention for TS names and define the TS qualified-name scheme (`src/a.ts#f`, `(src/a.ts#C).m`, `node_modules:express#Request.query`). Loader `go/flow_models.rs:1-5` only reads `[[go_flow_*]]`; add `[[ts_flow_*]]` and a built-in `ts_flow_models.toml`.
- Builtins hard-coded at `solver.rs:1204-1215` (`append|min|max|copy`); TS must emit none or models instead.
- `ValueKinds`/`FlowValueKind::Context` is Go-named (`ir.rs:170-180`, `sdk/dataflow.rs:12-19`); document as "Go context" or add TS classes.
- Capability wiring: `dataflow` is tied to `GO_SEMANTIC` (`analysis_kernel/call_cache.rs:101`); `routes` → `polint.go.semantic` (`analysis_kernel/provider.rs:1354`, `mod.rs:1154`). Add a `polint.ts.flow` provider (Rust) and make both languages satisfy the capability.
- Routes SDK is `GoSemanticRouteFact`/`GoRouteFunction` (`go/semantic/facts.rs:87-139`, `sdk/routes.rs:8-16`); rename to neutral `RouteFact` with a `language` field; store/validate paths follow.
- Typed calls: feed TS callee rows into `polint.calls` as `TypedCallInputs::new(Language::TypeScript)` so calls, refined calls and flow callees agree (today TS only reaches `refined_calls`).
- Docs: `data-flow.md:219-247` ("How Go is analysed", "TypeScript programs get no flows"), `routes.md:98-110`, `call-graph.md:110`, `CONSUMER-SETUP.md`.

**Reusable tests/corpora.** `tests/taint-corpus/corpus.toml` (53 Go cases, positive + twins, `// want-flow`) with harness `polint-eval/src/harness/taint_corpus.rs:18-19` gating precision ≥ 0.9 / recall ≥ 0.7; its case schema is language-agnostic but the harness asserts a Go program (line 218). Eval fixtures: `tests/eval-fixtures/{jelly,ts-object-model,ts-tokens,data-flow,refined-calls,determinism}`; external lanes `jelly_callgraph.rs` (F1 regression tolerance 0.005, `external/mod.rs:22`) and `secbench_js.rs` (smoke only, needs local clone). CI already installs Node 22 + `typescript@5` with `POLINT_REQUIRE_TS_TYPESCRIPT=1` (`.github/workflows/ci.yml:167-184`).

## 5. Recommended PR sequence (input to the roadmap)

1. **Neutralize contracts (no behavior change).** Per-language flow program accessor, `RouteFact` rename, generic model loader accepting `[[ts_*]]`, TS callee rows wired into `polint.calls` as `TypedCallInputs`. Gate: Go taint corpus numbers unchanged; golden corpus byte-identical; Jelly lane within 0.005 F1.
2. **TS flow program in Rust (Oxc), no Node.** Closures, destructuring, await, classes, module globals; callees from symbol graph/Jelly heap; built-in `ts_flow_models.toml` (Express `req.query/body/params` sources via import-resolved monikers; `pg`/`mysql2`/`knex` sql sinks, `child_process` exec, logger). New `tests/taint-corpus-ts` (≥ 20 cases mirroring Go categories) run through the same harness. Gate: precision ≥ 0.9, recall ≥ 0.7 without Node; `flows` on TS repos no longer returns `NoProgram`.
3. **Type-directed enrichment (`polint-ts-types-2`).** `declared_type` rows → `param_types`, `classes`, type-keyed source matching (`parameter_type = "express#Request"`), union/implementation candidates into flow callees. Gate: TS corpus recall ≥ 0.8 at precision ≥ 0.9; results byte-identical to PR 2 when Node is absent; cold sidecar ≤ +20% over 13.5 s on the 24k-line repo; warm unchanged.
4. **`[[ts_route]]` + Express/Fastify/Hono interpreter** in Rust over the TS flow program. Gate: route fixture repos under `tests/eval-fixtures/routes-ts/` with 100% expected-row match; route inventory on one real Express app checked against its documented endpoints; two-run byte identity via `determinism_gate.rs`.
5. **Next.js file routes, NestJS decorators, (tRPC).** Gate: fixture expected rows; unmodelled frameworks surface in `polint unknowns`.

## Facts verified in code

- Flow-body wire schema and ops: `polint-go-frontend/internal/semantic/flow.go:13-60`, `405-465`; Rust IR: `analysis_neutral/taint/ir.rs:16-27, 95-160`.
- Go-only gating of flows: `flow_queries.rs:28-43`; `NoProgram` wording `sdk/dataflow.rs:373`; capability ties `analysis_kernel/call_cache.rs:101`, `analysis_kernel/provider.rs:1354`.
- Go-specific solver assumptions: builtins `taint/solver.rs:1201-1215`; method naming `solver.rs:929, 1948`; model targets `taint/models.rs:14-55`; `[[go_flow_*]]` loader `go/flow_models.rs:1-5`.
- Route models/roles: `routes.go:19-74, 392-403`; `go/route_models.rs:28-70`; SDK `sdk/routes.rs:8-16`; facts `go/semantic/facts.rs:87-139`.
- Typed call facts: `emit.go:718-750`; `go/semantic/facts.rs:219-283`; neutral `analysis_neutral/calls/typed.rs:31-72`; Go-only wiring `analysis_kernel/provider.rs:355-363`.
- TS sidecar: protocol `research/ts-type-sidecar/wire-protocol.md`; toolchain rules `ts/types/process.rs:8-30`; config `ts/types/lifecycle.rs:28-57`; cache `ts/types/cache_key.rs:40-60`; consumer `docs/CONSUMER-SETUP.md:107-202`; checker calls `index.js:716-1301`.
- TS MIR lowering: `ts/mir/lower.rs:1270, 1323, 1336`; places `ir/places.rs:53-62`.
- Measurements: `research/ts-type-sidecar/measurement.md:65, 149-160, 215-219, 255-269`; baseline `research/evaluation-harness/baselines/persisted-graph-accuracy.json`; `performance/2026-06-17-jelly-fn-categorization-and-wins.md:3-4, 46-66`.
- Corpora: `tests/taint-corpus/corpus.toml` (53 cases); `polint-eval/src/harness/taint_corpus.rs:18-19, 218`; `polint-eval/src/harness/external/{mod.rs:22, jelly_callgraph.rs, secbench_js.rs:68-76}`; CI `.github/workflows/ci.yml:167-184`.
