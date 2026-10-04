# 13 — Full-application dataflow and control flow: why not today, what is missing, how to get there (2026-10-01)

Situation: **v0.4.4** (`db8d7ea2`) is the engine of record. The owner's target, stated after
[12](12-next-direction.md), is no longer "capability, middle layer first, deep later":

> Enable ways to utilize all the rules fully and do full analysis with data flow, control flow, etc.,
> throughout the full application.

Status: **research. Nothing is built.** One recommendation, with the first three steps and the gate
before each next one. All source citations are to the v0.4.4 tree; all measurements are from this
file's own runs unless marked **(quoted)**. Every projection is marked **(estimate)**.

## Summary

**Full-application dataflow and control flow is reachable for the Go backend within one engine
generation, if the deep stack is rebuilt as a per-package pipeline that consumes the Go toolchain's
own resolution instead of re-deriving it from text. The typed middle layer is not a detour on that
road; it is the first two steps of it.** The TS frontend is a second, later leg with a weaker floor.

The five findings that change the picture from [12](12-next-direction.md):

1. **(M) The headline "1,072 unresolved member calls" was an environment artifact, not an analysis
   limit.** On this host the Go *symbols* sidecar never loads OAIZ: it writes its own
   `runtime.Version()` (1.27.0) into a synthetic `go.work`, OAIZ's module says `go 1.27.1`, and Go
   refuses. polint reports that only as one `setup_missing` reference row per file and nothing at
   info log level. Every dotted Go call then falls through to `dynamic_property`. With a symbols
   sidecar built under Go 1.27.1, the same 41-file scan leaves **64** unknowns instead of 1,112, and
   **24** `dynamic_property` rows instead of 1,072, all of them type conversions such as
   `dto.CatalogItemType(x)` (§2.3). The prior round's finding 3 and this brief's starting point are
   corrected accordingly.
2. **(M) The Go semantic sidecar's cost is a load-mode choice, not an inherent cost of types or SSA.**
   It type-checks every transitive dependency *from source* (`NeedDeps|NeedSyntax`) and, because
   OAIZ has `package main`, builds SSA bodies for all 1,610 of them. Measured on the whole 380-package
   core module: **7.4 GB, 21 s**. Loading dependencies from export data instead, as the symbols
   sidecar and gopls do, gives the *same* SSA for the *same* 64,275 in-repo function bodies at
   **1.5 GB, 5.7 s warm** (§2.2). The comment justifying `NeedDeps` cites an RTA harness that release
   builds never run.
3. **(M) The toolchain already resolves almost everything a rule needs.** Over the whole core module,
   SSA resolves **77.2%** of 199,121 call sites statically (including 64,761 method calls on concrete
   receivers); **6.4%** are interface invokes, **0.7%** function values, and 15.7% builtins. x/tools'
   VTA call graph over the same program takes **5.8 s** and gives a single target to 5,720 of the
   7,229 invoke sites it covers (§2.2, §3.B). polint's Rust side uses none of this for call
   resolution: it lowers every callee as text and re-resolves it by symbol name (§2.3).
4. **(M) A `calls` or `dataflow` request has its own cliff inside polint, independent of the sidecar.**
   On the 41-file scope with a warm sidecar cache: `control_flow` 6.6 s / 0.53 GB; `calls`
   **38 s / 6.1 GB**, of which the abstract-domains solver is 29 s, +2.7 GB and 1.08 M facts. The
   compact materialization exists but is enabled only when every deep rule asks for `control_flow`
   alone (§2.4).
5. **(R) The solvers behind the deep views are not the algorithms their names suggest.** "IFDS" is
   breadth-first path enumeration that never enters callee bodies; interprocedural TITO summaries
   are never composed; points-to has one global 10k-step latch that turns every alias answer to
   Unknown; the domains solver uses unbounded call strings under one 10k-iteration cap; dominance
   gives up at 250k pairs; the CFG has no loop structure. None of it is parallel or persisted (§2.5).

**What is missing** (§3): typed per-site call facts and a real call graph for Go (both available
from the toolchain the sidecar already runs); framework models as data for the consumers' actual
stack (gin, GORM, Watermill, decorator-wrapped generic handlers; none is recognized today); per-unit
sharding, persistence and parallelism for every deep provider; a tabulation-based interprocedural
dataflow solver with composed summaries and models-as-data; loop and exception structure in the CFG;
and a rule API that returns graph facts with identifiers rather than five fixed query shapes with
hidden locations.

**Recommendation** (§7): one pipeline, built in this order, each step proven by rewriting named OAIZ
rules and passing a measured gate:

0. **Now:** fix the symbols-sidecar `go.work` version bug; switch the semantic sidecar to export-data
   dependencies and `ssautil.Packages`; flip `include_tests` to opt-in for deep providers; make
   `polint unknowns` fail loudly when a provider is blocked. Gate: catalog `control_flow` cold
   ≤ 15 s / ≤ 2.5 GB tree peak (today 29.6 s / 7.7 GB).
1. **Typed call facts and a public call graph** (3–4 weeks): the sidecar emits per-site resolved
   callees, receiver types, method sets and VTA edges; the Rust `calls` layer consumes them; ship
   `CallGraph<'_>` and `GoTypes<'_>` views with identifiers; make compact domains the default.
   Gate: whole-core `calls` cold ≤ 60 s / ≤ 4 GB; unresolved non-conversion call sites ≤ 1 per 100
   in-repo sites; four named rules rewritten with zero unexplained diffs.
2. **Units, shards and routes** (5–7 weeks): per-package Go units lowered in parallel and persisted as
   binary shards; cross-unit summaries on the unit DAG; gin routes with group/`Use` semantics as a
   data-defined model and a `Routes<'_>` view. Gate: whole-core `calls` warm ≤ +5 s over today's
   shallow warm; one-file edit ≤ 15 s; byte-identical reports across job counts; endpoint-authority
   rewritten on routes + reachability.
3. **Interprocedural dataflow** (6–8 weeks): IFDS/IDE tabulation over per-unit ICFGs with composed
   TITO summaries, k-limited access paths, sources/sinks/sanitizers as data, a taint corpus built
   from the consumers' rules, SARIF `codeFlows` from rule results. Gate: corpus precision ≥ 90%,
   recall ≥ 70%; whole-core `dataflow` cold ≤ 120 s / ≤ 6 GB; edit ≤ 20 s.

The deep providers stay off the agent edit loop until the step-2 edit gate passes; `review` and CI
profiles carry them first. The 6-core / 22 GB envelope holds at every step if the gates hold; it does
not hold for today's design at any scope above one package.

## 1. Sources, method, and what was measured

**Read.** The prior report ([12](12-next-direction.md)) and its raw evidence; the v0.4.4 archive
(`/opt/data/polint-next-direction/src-v044/`, since `/workspace/polint` is still at v0.4.0+1);
`research/strategy/04-full-app-deep-capability.md` and its plan; `research/incremental-query-engine`,
`local-semantic-store`, `data-flow`, `call-graphs`, `effects-summaries`, `abstract-interpretation`,
`cfg-control-flow`, `ts-type-sidecar`; `docs/architecture-review/04–06, 10`; `docs/facts/*`; both
consumer rule packs, read-only. Nine read-only audit agents swept the Go sidecars and RTA, the
language-neutral engine (MIR, CFG, calls, refined calls, solver, points-to, IFDS, summaries, domains,
demand, entrypoints, evidence), the kernel and caches, the SDK, the TS pipeline, the prior research,
and the rule packs; every load-bearing claim below was spot-checked against the archive.

**Measured for this file.** Host: the shared 6-core / 22 GiB container, 4 jobs, sequential runs
under a process-tree RSS sampler with a kill ceiling. Scope: the `--shared` OAIZ clone at
`2956a791a7` under `/opt/data/polint-next-direction/oaiz-bench/`; `/workspace/oaiz` was never
written. Driver: the cost-dispatch driver (polint 0.4.3 + #132; deep code identical to v0.4.4).
Raw evidence and scripts: `/opt/data/polint-deep-analysis/` (its `README.md` indexes them). None
of it is committed.

| # | What | How |
|---|---|---|
| M1 | Go load cost by mode and scope | a throwaway Go program (`loadcost`) that loads packages with `go/packages` either as the semantic sidecar does (`NeedDeps|NeedSyntax|NeedTypesInfo`, optionally `ssautil.AllPackages`) or with dependencies from export data, builds SSA, and counts call-site resolution classes. Scopes: the 9-package catalog context (41 files) and the whole `core` module (380 packages, 2,846 files, 623k lines); tests off and on; after a one-file edit. |
| M2 | Whole-core call graphs | the same harness building x/tools CHA then VTA over the export-mode program |
| M3′ | Reproduction of the prior M3 | `polint unknowns --cap control_flow core/internal/catalog`, cold and warm, with the 1,112 rows captured and every `dynamic_property` row classified by its callee text |
| M4 | The symbol-graph failure | `--cap references` and `--cap symbols` on the same scope; the symbols sidecar run by hand; the scans repeated with a sidecar built under Go 1.27.1 and a fresh cache |
| M5 | `calls` versus `control_flow` | both capabilities on the same 41 files, warm sidecar cache, with per-provider stage logs |
| M6 | A TS cell | `unknowns --cap calls oaiz-ui` (100 TS files), cold and warm |

**Environment notes.** Disk was 27 GB free at start, not the 62 GB the brief assumed; it was
monitored and no run was lost to it. Two harness runs overlapped another measurement and were
discarded and re-run alone (the include-tests load and the edit-tier reload); only the clean runs
are cited. The driver's cold `control_flow` scan took 53 s on its first run under foreign load and
28–30 s on two later cold runs; the latter are cited. The include-tests setting was written into
the bench clone's `.polint.toml` for the duration of each scan and restored after every run.

## 2. Why it is not working today

### 2.1 Every provider treats the whole module as its unit, and nothing deep is kept

- **The sidecar is invoked once per run on `./...` of every module root**, whatever paths the scan
  names; `--scope-files` only filters output rows (`go/semantic/client.rs:198-201`;
  `go/lifecycle.rs:44-49, 101-104`). The "41-file package" scan of the prior round therefore loaded
  all 380 core packages and their 1,623 dependencies. Its session row says so: `packages=380
  compiled_go_files=2846 deps_with_types=1623 peak_heap_bytes=6.9e9`.
- **Every deep provider is whole-program and sequential.** `analysis_neutral/` has zero uses of
  rayon, threads or locks; the MIR, CFG, calls, solver, points-to, data-flow and domains providers each
  make one pass over all bodies (`ir/body.rs:110-120` holds the flat whole-program tables;
  `analysis_neutral/types/provider.rs:103-114` solves points-to once for the program).
- **Nothing deep persists.** Only `polint.go.syntax` and `polint.ts.syntax` use the layer cache; all
  fourteen deep providers are `InMemoryDerived` and recompute every run
  (`analysis_kernel/provider.rs:1353-2002`). The one warm-run reuse is the sidecar's NDJSON file,
  keyed on the *whole-scan* `go.syntax` output digest plus the lifecycle digest
  (`go/semantic/cache_key.rs:32-47`): any Go edit anywhere re-runs the sidecar.
- **One deep rule pays for 22 of 24 providers** (`analysis_kernel/provider.rs:1067-1105`): six base
  providers plus sixteen deep ones for `calls` or `control_flow`, all twenty-four for `dataflow`.

What that costs on the 41-file scope, warm sidecar cache, with a working symbol graph (M4/M5; stage
logs in `unknowns-fixed3-*-warm.stderr`):

| Provider | `control_flow` | `calls` |
|---|---|---|
| `semantic_mir` | 0.34 s | 0.80 s |
| `cfg` | 1.72 s, +167 MB | 2.94 s, +167 MB |
| `abstract_domains` | 0.49 s, 173k facts | **29.4 s, +2.7 GB, 1.08 M facts** |
| `type_value_alias` | 0.96 s | 0.57 s |
| `solver` + `refined_calls` | 0.50 s | 0.33 s |
| **Run** | **6.6 s, 0.53 GB** | **38.0 s, 6.1 GB tree peak** |

### 2.2 The Go semantic sidecar: what it resolves, what it loads, what that costs

**What it resolves (M1, whole core module, non-test).** SSA over the 380 root packages: 64,275
function bodies (3,654 closures), 199,121 call instructions.

| Call-site class (SSA) | Count | Share |
|---|---|---|
| static callee (`StaticCallee() != nil`) | 153,815 | 77.2% (64,761 are methods on concrete or pointer receivers) |
| interface invoke | 12,725 | 6.4% |
| builtin (`len`, `append`, …) | 31,188 | 15.7% |
| function value (incl. closures called through a variable) | 1,393 | 0.7% |
| `go` statements / `defer` statements | 47 / 2,674 | — |
| interface types declared in root packages | 979 | — |

The sidecar classifies exactly this way (`emit.go:628-645`): `resolved_static` with the callee's
full name, `unresolved_dynamic` plus a `dynamic_dispatch` row carrying `invoke:<iface>:<method>` or
`func_value:<signature>` (`emit.go:680-714`). Package-qualified calls, concrete-receiver methods,
pointer receivers, embedded promotions and bound method values are all `resolved_static`. Generics
are instantiated (`ssa.InstantiateGenerics`), so a call on a type parameter resolves per
instantiation. Closures are walked through `AnonFuncs`; `go`/`defer` are ordinary call instructions.
Not emitted: interface declarations, types of places, `go`/`defer` markers, or per-site receiver
types (only `receiver_type` rows for method declarations).

**What it loads (`emit.go:202-224`).** `NeedDeps|NeedSyntax|NeedTypes|NeedTypesInfo` on the whole
transitive graph: every dependency, standard library included, is parsed and type-checked from
source with full `types.Info`. If any root is `package main`, `ssautil.AllPackages` + `prog.Build()`
builds SSA bodies for that entire closure (`emit.go:245-247`). The comment justifying `NeedDeps`
(`emit.go:207-212`) says it prevents `rta.Analyze` panicking on `reflect`; `rta.Analyze` runs only
under `--rta-edges`, which only the polint-eval harness passes (`emit.go:35-42, 314`). `Tests: true`
is the default (`go/lifecycle.rs:121-124`) and adds every `P.test` variant. There is no memory limit
(`GOMEMLIMIT`, `GOGC`, rlimit: not found); the kernel's memory ceiling samples only its own RSS
(`analysis_kernel/resource.rs:97-105`), so the sidecar is invisible to it.

**What that costs (M1).** Go build cache warm; GOMAXPROCS=4; "HWM" is the harness process's own
peak RSS.

| Scope | Load mode | Tests | Load | SSA | Total | HWM | Packages typed / with syntax |
|---|---|---|---|---|---|---|---|
| catalog (9 pkgs, 41 files) | export data | off | 1.4 s | 0.0 s | 1.5 s | **57 MB** | 859 / 9 |
| catalog | sidecar mode, `Packages` | off | 9.5 s | 0.1 s | 9.7 s | 1.73 GB | 859 / 858 |
| core `./...` (380 pkgs, 2,846 files) | export data | off | 3.2 s | 1.1 s | **5.7 s** | **1.52 GB** | 1,623 / 368 |
| core `./...` | export data + CHA + VTA | off | 7.0 s | 2.8 s | 40.8 s (CHA 20.8 s incl. the harness's own edge counting; VTA 5.8 s) | 1.79 GB | — |
| core `./...` | sidecar mode, `Packages` | off | 15.2 s | 1.3 s | 17.9 s | 4.40 GB | 1,623 / 1,610 |
| core `./...` | **sidecar mode, `AllPackages` (what the sidecar does: `core/cmd` is `main`)** | off | 11.7 s | 4.6 s | **21.0 s** | **7.43 GB** | 271,888 bodies, 760k calls |
| core `./...` | export data | **on** | 11.7 s | 5.4 s | 20.6 s | 4.76 GB | 2,622 / 1,356; 1,089 roots, 8,550 files, 187,986 bodies, 898,551 calls |
| core `./...` | export data, after a one-file edit in `catalog/domain` | off | 3.3 s | 0.9 s | 5.7 s | 1.53 GB | 13 dependents recompiled by the build cache |

The sidecar's own run inside polint matches the `AllPackages` row: 43.9 s elapsed and 6.9 GB peak
heap for the same 380 packages (its session row), 7.3–7.9 GB tree RSS as sampled (M3′, M4). Half of
that is type-checking 1,610 dependencies from source; the other half is SSA bodies for 272k
dependency functions that no polint consumer reads. The export-data row is the same program, the
same 64,275 bodies and the same resolution counts at a fifth of the memory and a quarter of the time.
With tests on, export mode holds 4.8 GB for 188k bodies; the sidecar's test-on run on this host
passed 12 GB and was killed in the prior round. The first cold export-mode run pays the Go build
cache (303 s to compile 1,623 dependencies, 233 s with tests), which the consumer's `go build` and
`go test` already pay and share.

### 2.3 The resolution is thrown away, and then the fallback silently failed

**How a Go call reaches a target in polint.**

1. `go/mir/lower.rs:1562-1568` lowers every callee as `ValueDraft::Unknown { evidence: <source text> }`.
2. `analysis_neutral/calls/extract.rs:324-347` turns any dotted evidence into
   `CallCallee::Member { base: PlaceId(u64::MAX), property }` with no receiver place. `pkg.F()`,
   `x.f()`, `s.inner.f()` and `T[int]{}.f()` are indistinguishable.
3. `calls/direct.rs:166-200, 384-406` resolves a `Member` only if exactly one symbol-graph reference
   named `property` overlaps the call span, is `Resolved`, has one target, has precision
   `ExactSemantic|ExactLocal|ModuleLinked`, and the symbol is a function, method, class or import.
4. Otherwise `calls/unresolved.rs:79-81` labels it `dynamic_property`.
5. The sidecar's `resolved_static` rows are consumed only to grow RTA reachability
   (`go/rta/inputs.rs:186-212`); `refined_calls/go.rs:19-72` refines only sites with a receiver
   place, which Go member sites never have, so it adds no target.

So member-call resolution rests entirely on the Go *symbols* sidecar (`polint-go-symbols`, a
`go/types` index without `NeedDeps`), joined by name and span. When that index exists, step 3 works
well: every selection carries `Selection.Obj()` and every cross-package use gets a symbol row with an
empty file (`polint-go-symbols/internal/symbols/emit.go:830-852, 1066-1074`).

**Why it did not exist on this host (M4).** `--cap references` on the catalog scope returns 41 rows,
one per file: `setup_missing`, "reference did not resolve to exactly one public symbol"
(`analysis/unknown_taxonomy/collect.rs:151-176`). The symbols sidecar run by hand says why:

```
go: module core listed in go.work file requires go >= 1.27.1, but go.work lists go 1.27.0;
to download and use go 1.27.1: go work use
```

The sidecar writes its own `runtime.Version()` into the synthetic `go.work`
(`symbols/emit.go:495-503`); the host binary is Go 1.27.0, OAIZ's `core/go.mod` says `go 1.27.1`.
The semantic sidecar derives the version from the modules (`polint-go-frontend/.../emit.go`,
`syntheticGoWorkVersion`) and loads fine, so deep scans *run* and every dotted call becomes
`dynamic_property`. Nothing is logged at info level; the failure is visible only as the per-file
rows above or as capability evidence in `polint check`, and the failed symbol graph is then
layer-cached.

**What the 1,072 rows actually were (M3′).** Classified by callee text:

| Class | Rows | Examples |
|---|---|---|
| package-qualified function | **598** | `customerrors.*` 207, `strings.*` 96, `domain.*` 54, `slices.*` 48, `server.*` 39, `dto.*` 18, `decorator.*` 16, `time.*` 14, `auth.*` 14, `uuid.*` 13 |
| method on a local or field receiver | 240 | `args.Get` 45 (testify mock), `h.*` 39, `repo.*` 26, `apiRoutes.Use` 16, `query.*` 16 |
| nested field receiver | 156 | `h.Router.GET` 59, `r.mu.Lock` 40 |
| gin context | 77 | `c.JSON`, `c.Status` |
| chained call | 1 | — |

**With a working symbols sidecar** (built under Go 1.27.1, passed through `POLINT_GO_SYMBOLS`, fresh
cache): the symbol graph loads the scope in 1.3 s (3,226 symbols, 13,710 references, 445
cross-package symbols), `--cap references` returns **0** rows, and `--cap control_flow` returns
**64**: 24 `dynamic_property` that are all type conversions lowered as calls
(`dto.CatalogSortBy(x)`, `domain.ItemType(y)`, `kernelmap.ItemType(z)`), 3 `function_value` on a
local `compare` variable, and 37 `framework_dispatch`/`unsupported_syntax` rows on one line of the
testify mock. The prior round's conclusion that "type facts and framework models are what resolve
those calls" is therefore wrong at the symbol level; what remains structurally unresolved is
*dispatch*: interface calls resolve to the abstract method symbol (`direct.rs:384-389` has no
interface guard), and concrete targets come only from RTA, which matches by method name alone with
no implements check and floors precision at Heuristic (`solver/go_rta/snapshot.rs:140-191`,
`dispatch.rs:99-108, 188-205`).

### 2.4 The `calls` cliff inside polint

`AbstractDomainsProvider::run` picks the compact `SummaryInputs` materialization only when every deep
rule requests `control_flow` and none requests `calls` or `dataflow`
(`analysis_kernel/provider.rs:485-494`). Any `calls` or `dataflow` rule gets the full whole-program
call-string IDE solve: on 41 files, 29.4 s, +2.7 GB, 1.08 M facts, for an output that only the
summaries module reads (§2.1 table). The domains solver itself has unbounded call strings, a FIFO
worklist, one global `widening_fuel = 8`, never calls the domains' `widen`, and stops at
`max_iterations = 10_000` by marking every state `Top(BudgetExceeded)`
(`analysis_neutral/domains/solver.rs:62-65, 84-85, 242-305, 616-691`). The prior full-app research
measured that cap tripping at 45 files **(quoted)**. This is the single largest in-process cost on the
`calls` path and it has no sidecar component.

### 2.5 The solvers are not what their names suggest

| Subsystem | What the code does (v0.4.4) | Where |
|---|---|---|
| MIR | basic blocks + terminators per function; **not SSA**, no phi; callee always `Unknown{text}` | `ir/body.rs:51-90`, `go/mir/lower.rs:1562` |
| CFG | per-operation nodes and blocks; dominators and post-dominators (Cooper–Harvey–Kennedy); **no loop structure** (`LoopHeader`/`LoopBack` exist but production lowering never emits them); only the `NormalControl` view; dominance pairs capped at 250,000 then tree-only | `cfg/lower.rs:190-196`, `cfg/derived.rs:66-366`, `cfg/budget.rs:30` |
| calls / direct | text-shape classification; symbol-name join; lexical unique-name fallback for TS | `calls/extract.rs`, `calls/direct.rs:18-124` |
| refined calls | eight tiers merged, never reconciled; the Go tier re-labels, adds no targets; no CHA or VTA implementation anywhere | `refined_calls/provider.rs:67-158`, `go.rs:19-134` |
| solver | `derive_edges` is a per-source BFS copy-closure, O(V·E) with quadratic output; Go RTA by method name, 256 address-taken cap, 10k global worklist steps with early return | `solver/engine.rs:276-459`, `go_rta/fixpoint.rs:147-191` |
| points-to | Andersen, field-sensitive, context- and flow-insensitive, whole program, **no cycle elimination**; one latch (10k steps / 64 objects per var / 512 dynamic vars) turns every alias answer to Unknown; 64 alias pairs answered per program | `points_to/solver.rs:23-25, 228-230`, `aliases/provider_stack.rs:8` |
| "IFDS" | breadth-first enumeration of explicit paths over the value-flow graph with a cloned visited set per path; a single `Tainted` fact; the ICFG is built per (source, sink) pair and used only for a boundary check; **taint never enters callee bodies** | `ifds/mod.rs:200-343, 456-510` |
| data flow | every call forwards every argument to its return; operands of `BinOp`/`Aggregate`/`Closure` are dropped; synthetic callee-input/output nodes are dead ends; interprocedural flow only through per-site TITO bridges | `data_flow/local.rs:196-228, 433-450`, `direct_calls.rs:117-294` |
| summaries | TITO captures only direct parameter→return copies; SCC closure works on digest *strings* and **never composes TITO** ("unchanged for now") | `summaries/builder.rs:755-840`, `closure.rs:569-573, 623` |
| slicing, demand | dead in production; `demand/` is a memo table "to be wired by Plan 04" | `slicing/*`, `demand/engine.rs:84-89` |
| entrypoints | Go: net/http, chi, cobra, `testing`; **gin, echo, fiber, gorilla, gRPC: unrecognized**. TS: express, MCP, commander, yargs, jest/vitest/mocha; **fastify, koa, nest, next: unrecognized**; React, TanStack, Hono: absent | `entrypoints/recognizers_go.rs:38-65`, `recognizers_ts.rs:41-89` |
| evidence | built, ranked, rendered to SARIF `codeFlows` only from engine-filled `evidence_v1`; `polint unknowns` prints zero rows when the pipeline is blocked | `evidence/*`, `cli/mod.rs:2550-2600` |

Budgets are global per run, not per unit, and none is wall-clock; the only ceiling is memory
(§2.1). Identity keys include byte spans, so every edit moves every key in the file.

### 2.6 Frameworks and entry points versus the consumers' stack

OAIZ (Part A of the pack audit): gin in 147 files with 299 route registrations across 128 port
files, 34 `Group` calls and 26 auth-middleware references; GORM in 219 files; Watermill through an
in-house `pubsub` wrapper in 121 files with an outbox; 440 generic `CommandHandler[…]`/`QueryHandler[…]`
declarations wrapped by 426 `decorator.Apply*` calls; 545 interfaces (122 repository ports); 13
`package main`. The Go+TS monorepo: the same stack (gin 101, GORM 190, Watermill 87, 261 decorated
handlers). polint recognizes none of it: every OAIZ route is an `UnresolvedFrameworkFact`, so the
deep stack's roots are `main`, `init` and exported functions, and HTTP request sources never attach
to a handler.

### 2.7 The TS side

- `oxc_semantic` is used only in the symbol graph and the per-file extractors; MIR lowering,
  callable flow and points-to work on name strings (`ts/mir/lower.rs:3301-3335`,
  `ts/callable_flow/extract.rs:7293-7347`). Destructured parameters are dropped; `try`, `switch`,
  `for-of`, spread, computed keys and dynamic `require` are `Unsupported` markers with havoc.
- The type-directed tier runs the repository's own `tsc` through a Node sidecar, refuses TS ≥ 7, is
  keyed on the whole-scan `ts.syntax` digest, and costs about 1.5 ms per call site **(quoted,
  PR #121)**. OAIZ pins TypeScript 6 preview aliases, which the sidecar accepts.
- **M6:** `unknowns --cap calls oaiz-ui` (100 files, 14.6k lines): **22.6 s / 2.1 GB cold, 28.8 s /
  2.1 GB warm**, 489 unknowns (261 `dynamic_property`, 228 `missing_semantic_reference`). Warm is
  not faster because nothing deep is cached. Linear extrapolation to the 2,461-file frontend is
  about 10 minutes **(estimate; the prior round measured `semantic_mir` alone at 203 s for 2,381
  files on v0.3.10, quoted)**. The frontend's frameworks (React 19, TanStack Query, Next) have no
  recognizers, so there are no TS entry points for the consumers either.

### 2.8 What the rule API exposes versus what rules need

The deep views are five fixed query shapes over a closed pattern vocabulary
(`sdk/policy.rs:328-787`, `sdk/facts.rs:878-960`): `Calls::forbidden_reachable(ReachQuery)`,
`ControlFlow::{missing_guard, guard_outcomes, missing_cleanup}`, `DataFlow::forbidden(FlowQuery)`,
`Events::matching`. A `PolicyViolation` hides `file()`, `range()` and `evidence()` behind
`pub(crate)` (`policy.rs:198-211`); no deep result carries an identifier a rule could join to a
`FunctionId` or `SymbolId`; `Cfg<'_>` and `CallGraph<'_>` have no methods and map to `Unsupported`
(`facts.rs:852-864`); capabilities cannot be optional, and a rule with one unavailable capability
never runs (`core/rule.rs:445-449`); rules cannot read unknowns; `Fix` is a single replacement
with no range (`internal_core/diagnostic.rs:128-131`).

Against that surface, the pack audit's twenty rules that need depth (Appendix B) can be written
today: **zero**. The seven that need a gin route model with group inheritance have no route fact;
the eight that need reachability through repository interfaces or decorator-wrapped handlers have
no call-graph view and heuristic-only dispatch; the five that need dominance on a `Handle` body can
use `guard_outcomes` only when the guard and the operation are plain calls in one function; the four
that need `*gorm.DB` alias/def-use have no alias query; the implements proof has no type view.

### 2.9 Caching and incrementality, as built

What persists: syntax layers (JSON, 64 MiB read limit), module graph, symbol graph, metrics, and the
two sidecars' NDJSON. What does not: every deep provider. `provider_version = CARGO_PKG_VERSION`
cold-starts every layer on every release. Keys fold whole-scan digests, so one edit re-runs the
semantic sidecar (43.9 s here) and every deep provider. The 2026-09-19 full-app plan's remedy
(W5 identity arena, W6 per-unit lowering, W7 shards, W8 cross-unit joins, W9 wall-clock budget) is
unbuilt except W1 (indexes) and W4 (dominators); its gates G1–G10 have never been run, because the
consumer corpora were not on the host. Memory from this series records that the symbol sidecar's
Export keys collide on test variants and that `include_tests = true` costs 2.4× for byte-identical
findings.

## 3. What is missing: the inventory

Each row: what exists at v0.4.4, what full-application analysis needs, where to take it from, and a
size class (S < 1 week, M 2–5 weeks, L 1–3 months; all estimates).

| # | Component | Today | Needed | Prior art to take | Size |
|---|---|---|---|---|---|
| A | **Typed per-site call facts (Go)** | sidecar emits `resolved_static` names, consumed only by RTA seeding; Rust re-resolves by symbol name | the sidecar emits, per call site: resolved callee (static), `invoke` interface + method, func-value signature, receiver type, and per-place types; the Rust `calls` layer consumes them by `(file, span)` as the existing RTA join already does (`go/rta/inputs.rs:48-56`) | `go/ssa` `CallCommon.StaticCallee`, `IsInvoke`; `go/types` `Selection` | S–M |
| B | **Call graph (Go)** | RTA by method name over sidecar rows; no CHA/VTA; interface calls resolve to the abstract method | CHA then VTA computed in the sidecar where SSA already lives (x/tools `callgraph/cha`, `callgraph/vta`: 5.8 s whole core, 182k edges, 79% of covered invoke sites single-target); edges carry algorithm and precision; entry roots from framework models | x/tools callgraph; govulncheck's `vta(cha)` chain; CodeQL's per-call-site dispatch tiers | M |
| C | **Call graph (TS)** | declared-type tier from the `tsc` sidecar + name-keyed callable flow + Andersen with a global latch | the type tier as the primary resolver with per-project keys; Andersen per project with SCC collapse and per-unit budgets | PR #121's design; Jelly's approximate interpretation | M–L |
| D | **Framework models as data** | hard-coded recognizers; gin, GORM, Watermill, decorator handlers, React, TanStack, Next: none | a TOML model vocabulary: route registrars with receiver-copy semantics (`Group`/`Use`), handler shape, middleware chains, ORM chain terminals, publish/subscribe registrars, DI/decorator wrappers (`decorator.ApplyX(handler)` returns the handler), test harness (`httptest`); built-in defaults for gin, chi, net/http, GORM, Watermill; unrecognized forms stay explicit unknowns | CodeQL models-as-data; Semgrep Pro framework models; polint's own `.polint/models` loader | M |
| E | **Units, shards, parallelism** | whole program, sequential, un-persisted; whole-scan cache keys | unit = Go package / TS tsconfig project; per-unit lowering (structure facts, MIR, CFG, call sites with resolved callees, local summaries) in parallel with deterministic merge; one binary shard per unit keyed by unit content digest + imported units' interface digests + provider version + config digest; cross-unit facts computed on the unit DAG (petgraph; SCC-collapsed for TS cycles) | go/analysis facts (per-package analysis with exported facts, cached by the build cache); gopls; the 2026-09-19 plan's W6–W8 | L |
| F | **Interprocedural dataflow** | BFS path enumeration; TITO never composed; every call forwards all arguments | IFDS/IDE tabulation (path edges, summary edges) over per-unit ICFGs with the unit DAG supplying callee summaries bottom-up; k-limited access paths (k = 2–3); sources/sinks/sanitizers/propagators as data; per-unit budgets and a wall-clock deadline; a taint corpus built from the consumers' rules | Reps–Horwitz–Sagiv IFDS, Sagiv–Reps–Horwitz IDE; FlowDroid; CodeQL's `DataFlow::Global` with `FlowState` and summary models; Boomerang/SPDS for demand-driven field sensitivity | L |
| G | **CFG completeness** | no loops, no `panic`/`defer`/exception edges beyond `Unsupported`, `NormalControl` only; dominance pair cap | loop headers and back edges; `defer` as exit-path edges; `panic`/`os.Exit` as terminating; try/catch for TS; dominance as a tree with on-demand ancestor queries (no pair materialization) | go/ssa block structure; rustc MIR's explicit `Drop`/unwind edges | S–M |
| H | **Whole-app entry points** | `main`, `init`, exported functions, tests; routes/subscribers/jobs unrecognized | roots from D plus `main`/`init`/tests; trust boundaries per root kind; per-root reachability memoized per query | existing `reachability` provider | S (after D) |
| I | **Rule API** | five fixed shapes; hidden locations; no identifiers; `Cfg`/`CallGraph` empty; no soft capabilities | `CallGraph<'_>`: `callees(FunctionId)`, `callers`, `reachable(roots, target)`, `paths(...)`, each edge with precision/algorithm/evidence; `GoTypes<'_>`: `type_of(place)`, `method_set`, `implements(T, I)`, `receiver(FunctionId)`; `Routes<'_>`: route → handler → effective middleware; `Cfg<'_>`: `dominates(a, b)`, `on_every_path(from, to, pred)`; `DataFlow<'_>`: `flows(sources, sinks, config)` with user-defined patterns, each flow with its path and its unknowns; `Option<View<'_>>` for soft capabilities; `ctx.unknowns()` | the prelude's dense-ID views (`Symbols`, `References`, `ModuleGraphFacts::reachable_from`) | M |
| J | **Provenance and edits** | `codeFlows` only engine-filled; `Fix` single replacement | rule-attached paths rendered as `codeFlows`; per-finding unknowns and budgets; ranged multi-edit fixes | SARIF; go/analysis `SuggestedFix` | M |
| K | **Budgets** | global per run, no wall clock, sidecar unbounded | per-unit step and memory budgets, a run deadline, `GOMEMLIMIT` on the sidecar, every budget trip a reported unknown | the plan's W9 | S |

## 4. Design decisions

### 4.1 In-process Rust versus a sidecar for Go, with numbers

A Go type checker in Rust is rewrite-class: `go/types` with generics, the module loader, build
constraints and cgo handling have no Rust equivalent, and the prior research recorded the same
exclusion twice. The question is only *where the boundary sits*. Measured on the whole core module
(§2.2): the toolchain gives types, SSA and a VTA call graph for 623k lines in **about 13 s warm and
1.8 GB** from export data (load 7.0 s + SSA 2.8 s + VTA 5.8 s, with the harness's own CHA counting
removed), and a one-file edit costs nothing measurable because the build cache recompiles only the
13 dependents. polint's Rust lowering of the same code is unmeasured at this scope on v0.4.4; at
v0.3.10 `semantic_mir` alone was 565 s and +10.4 GB for 4,752 files **(quoted)**, superlinear, and
the W1 indexes have not been re-measured at consumer scale.

So the sidecar stays, and it should do *more*, not less: resolution, types, method sets, and the
call graph, which are the parts the toolchain does well and polint re-derives badly. What must not
cross the boundary is bulk: the current NDJSON for the 41-file scope is **35 MB** (19k callsite
rows plus 16.5k `instantiated_type`, 14.6k `receiver_type` and 7.5k `dynamic_dispatch` rows dumped
program-wide), which is 0.85 MB per in-scope file; at that density the whole core would be about
2.4 GB of JSON **(estimate)**. The boundary therefore has to be per-package shards in a binary
codec (direction A's L5 becomes a prerequisite here), with program-wide sets (instantiated types,
address-taken functions) emitted once per module. The language-neutral MIR stays in Rust because the
rule API, the TS leg and the dataflow solver need one IR; the Go MIR is enriched with the sidecar's
per-site facts rather than replaced by SSA, which is what the 2026-09-19 plan concluded too and
which this file's numbers do not overturn.

For TS the equivalent split already exists (Oxc in process; `tsc` in a sidecar for types) and the
question is scale, not placement.

### 4.2 The middle layer as part of the deep stack, not before it

The owner's objection to [12](12-next-direction.md) is that a middle layer first is a detour. The
measurements say it need not be one, if the layer is built as the *first outputs of the same
per-unit pipeline*:

- **Type facts cost nothing extra.** Export-data loading gives `types.Info` for every in-repo package
  at 1.5 GB / 5.7 s for the whole backend; the symbols sidecar already loads this way. Receiver
  types, method sets, struct fields with tags, interface satisfaction and generic instantiations are
  one emitter pass over data the sidecar already holds.
- **Structure facts are the same lowering pass.** Call sites with argument spans, enclosing
  functions, composite-literal keys, `go`/`defer` markers and body ranges fall out of the per-unit
  MIR lowering that step 2 builds anyway; exposing them as views is the SDK work, not analysis work.
- **Framework models are the deep stack's roots and sinks.** A gin route model is what makes HTTP
  request sources attach to handlers; a GORM chain model is what makes a write sink identifiable.
  Building them as data in step 2 serves both the shallow rules (endpoint authority) and the taint
  solver (step 3).

What changes versus [12](12-next-direction.md) is the order inside the layer: type-resolved call
facts (its B4) come first, because they are measured cheap and unblock every deeper step, and the
syntax-tier structure facts (its B1) ride along the unit lowering rather than preceding it. The
runtime dividend [12](12-next-direction.md) projected for B1 (the consumers' scanner code) is still
unmeasured and still real; it arrives in step 2 instead of step 1.

### 4.3 Eager per unit, demand-driven across units

- **Eager per unit.** Structure facts, MIR, CFG with dominators, call sites with resolved callees,
  local summaries (control, call, memory, TITO) and the unit's part of the call graph are computed for
  every unit in scope, in parallel, and persisted. Cost is linear in units and cached by content.
- **Demand-driven across units.** Reachability from a root set, taint from a source set, and
  dominance questions are answered per rule query, loading only the shards on the path, with results
  memoized per (query digest, shard digests). This is the CodeQL shape (eager extraction, demand
  queries) and the go/analysis shape (per-package facts, dependency order).
- **Not** a whole-program IR in memory, which is what every deep provider is today and what the
  7-GB and 1.2 M-fact numbers come from.

### 4.4 The caching layer

| What | Key | Invalidated by |
|---|---|---|
| sidecar shard per Go package | package source digest + digests of imported packages' *export interfaces* (what `go list -export` already computes) + sidecar binary digest + toolchain version + lifecycle digest (tests, tags, roots) | an edit to the package, or an interface change in an import; a body-only edit in an import does not invalidate (the build cache's own rule) |
| unit shard (structure facts, MIR, CFG, call sites, local summaries) | unit source digest + sidecar shard digest + provider version + config digest (rule options that affect lowering) | the same, plus engine release |
| cross-unit summaries (SCC-ordered) | the member units' shard digests + callee summary digests | bottom-up: only SCCs whose inputs changed recompute; equality backdating stops propagation |
| call graph | shard digests of all units + model digest | any unit change re-merges edges (cheap: edges are per-shard, the merge is a sort) |
| query memos (reachability, taint, dominance answers) | query digest + the shard digests it read (recorded during evaluation) | any read shard changes |

This is the 2026-09-19 plan's W7/W8 with one change: the sidecar shard key is per package from
day one, because the export-data measurement shows the Go side of an edit is already incremental.
The 64 MiB layer read limit and JSON payloads must go in the same step (direction A's A1/A3).

### 4.5 The Rust solver architecture that fits

- **Graphs:** petgraph `StableGraph` for the unit DAG and the call graph, SCC via `tarjan_scc`
  (already used by summaries), dense `u32` node ids assigned after sorting by stable key text, as the
  solver store does today (`solver/store.rs:44-53`).
- **Determinism:** rayon over units with results collected into a `Vec` indexed by unit ordinal and
  merged in ordinal order; no shared mutable state during lowering; every cross-unit pass single
  threaded over a sorted worklist or parallel per SCC level.
- **IFDS/IDE:** a tabulation solver over per-unit exploded supergraphs with summary edges stored per
  (callee, entry fact) and reused across callers; facts are access paths with a depth cap; sources,
  sinks, sanitizers and propagators are data loaded from models; per-unit step budgets and one run
  deadline, both reported as unknowns.
- **Abstract domains:** keep the reduced product, but run it per function with k-limited call-string
  context (k = 1) and real widening, feeding summaries; never the whole-program ICFG of today.
- **Dominance:** tree plus on-demand ancestor queries (depth + parent pointers), no pair sets.
- **Precision labels:** every edge and flow carries algorithm, precision and evidence, as the refined
  calls fact already does (`refined_calls/facts.rs:11-32`); rules filter on them.

### 4.6 Prior art worth taking, grounded in polint's constraints

| Source | Take | Leave |
|---|---|---|
| `golang.org/x/tools/go/ssa`, `go/callgraph/{cha,vta}` | the call graph itself, computed in the sidecar; `ssa.InstantiateGenerics`; `StaticCallee`/`IsInvoke` classification | `rta` (needs `main` and full-closure SSA; the thing that forced `NeedDeps`) |
| `go/analysis` and its `Fact` mechanism | per-package analysis in dependency order with exported facts, cached by the build system; the unit/shard model | running polint rules as Go analyzers (OAIZ left that model) |
| gopls | export-data loading; per-package invalidation keyed on import interfaces | — |
| CodeQL | models-as-data for sources/sinks/summaries; eager extraction + demand queries; `codeFlows` evidence; precision tiers per call edge | a Datalog engine (recorded exclusion) |
| IFDS/IDE (Reps, Horwitz, Sagiv), FlowDroid, Heros | tabulation with summary edges; access paths with k-limiting; sanitizer kills as transfer functions | whole-program eager tabulation across every unit at once |
| Boomerang/SPDS | demand-driven field-sensitive alias questions from a query point | — |
| rustc MIR + `rustc_mir_dataflow` | explicit control flow with unwind edges; a dataflow framework parameterized by direction, lattice and transfer function; "MIR is not SSA" is a legitimate choice if the dataflow framework is good | borrowck-grade region analysis |
| Salsa / red-green | equality backdating for summaries; durable keys per layer | Salsa itself as a dependency (recorded exclusion) |
| Jelly, PR #121 | type-directed TS call resolution with explicit `any` density gates | re-tuning recognizers against Jelly micro |

### 4.7 The case against, as strong as it can be made

1. **Demand is absent.** 106 rules, zero deep requests; the 87k lines of deep machinery shipped
   between May and September unused. The two packs' measured walls are routes, typed structure and
   tests; a middle layer answers them at a fraction of this plan's cost, which is what
   [12](12-next-direction.md) recommended.
2. **The precedent is bad.** The 2026-09-19 plan has ten gates that were never run and seven
   workstreams that were never built; the gate corpora were not on the host. Another plan of the same
   shape, without a consumer pulling, risks the same outcome.
3. **Precision on real code is unmeasured.** The L4 interprocedural probes pass 4 of 10 **(quoted)**;
   there is no taint corpus; Google's bar is under 10% effective false positives, and diff-time
   findings are what get fixed. A whole-application taint result that is mostly "unknown" or mostly
   wrong is worse than no result.
4. **The agent loop runs the full repo on every edit.** Deep providers that add 30–60 s to that loop
   will be turned off, and a profile nobody runs is capability nobody has.
5. **Do it differently:** write the deep Go policies as `go/analysis` analyzers in a Go sidecar
   "rule host", where SSA, VTA and `go/types` are native, and keep polint for the shallow rules.
   This is the cheapest path to a correct Go call graph today.

Why the recommendation still stands: (1) and (2) argue for *how* (measured gates on named consumer
rules, deep off the edit loop until the edit gate passes), not against the target the owner has
set; (3) is exactly what step 3's corpus gate exists to measure before anything ships; (4) is
addressed by sharding and the edit-tier gate in step 2; and (5) abandons the single-SDK,
multi-language product and the TS leg entirely, and OAIZ already left `go/analysis` once. The honest
version of the recommendation is that step 0 and step 1 are cheap and certain, step 2 is the
expensive one and carries the plan, and step 3 is where the "most powerful" claim is earned or
killed by a corpus.

## 5. The build plan

Each step: what is built, which OAIZ rules become writable (ids from the pack audit; the Go+TS
monorepo's equivalents in brackets), the measured gate before the next step, and the kill
criterion. Gates are on the OAIZ `core` module (380 packages, 2,846 files) unless stated, 4 jobs,
the methodology in [09](09-methodology.md).

### Step 0: stop the bleeding (days)

- Symbols sidecar: derive the synthetic `go.work` version from the modules, as the semantic sidecar
  does; surface sidecar setup failures as a visible diagnostic and a warn-level log, not only
  per-file unknown rows; do not layer-cache a setup-missing symbol graph.
- Semantic sidecar: drop `NeedDeps`; use `ssautil.Packages` always (never `AllPackages`); set
  `GOMEMLIMIT`; make `include_tests` opt-in for the semantic sidecar; report its RSS in the stage
  log.
- `polint unknowns`: a blocked or failed provider is an error row, never an empty list.
- Default the abstract-domains provider to compact materialization; the full mode only for a rule
  that declares it needs per-point domain facts.
- Merge #126 and raise the 64 MiB layer read limit (direction A's A1).

**Gate 0:** catalog `control_flow` cold ≤ 15 s and ≤ 2.5 GB tree peak (today 29.6 s / 7.7 GB);
catalog `calls` warm ≤ 8 s and ≤ 1 GB (today 38 s / 6.1 GB); `--cap references` returns 0
`setup_missing` rows on a host whose Go is older than the module's; reports byte-identical for
every rule that requests no deep capability. **Kill:** none; this is repair.

### Step 1: typed call facts and a public call graph (3–4 weeks)

**Build.** The sidecar emits per call site: static callee (package path, receiver type, name),
invoke interface and method, func-value signature, receiver type, and `go`/`defer` flags; per
package: method sets, interface declarations, implements pairs for in-scope types, generic
instantiations; and CHA/VTA edges with algorithm labels. The Rust `calls` layer joins these by
`(file, span)` into `CallTarget`s with `Exact`/`SetupAware` precision for static callees and
`TypeHierarchy` edges for invokes; RTA remains only as a fallback label. SDK: `CallGraph<'_>`
(`callees`, `callers`, `reachable`, `paths` with depth and path caps; edges carry precision and
algorithm) and `GoTypes<'_>` (`receiver`, `method_set`, `implements`, `type_of` for parameters and
fields), both with dense ids joinable to `Functions` and `Symbols`; `Option<View<'_>>` soft
capabilities.

**Rules writable:** `local/backend-endpoint-authority` handler→gate reachability without the
one-package substring fixpoint; `local/actor-handler-inputs` on generic instantiation arguments;
`local/backend-gorm-adapter-tests` constructor detection on `*gorm.DB` field types;
[`backend-domain-contract-boundary`'s implements proof, retiring the 3,709-line private parser;
`backend-handler-deny-uses-access-record` with exact callee identity].

**Gate 1:** whole-core `calls` cold ≤ 60 s and ≤ 4 GB tree peak; warm ≤ 20 s; unresolved call sites
that are not type conversions ≤ 1% of in-repo sites (`polint unknowns --cap calls` on the whole
core); VTA edge set reproduced byte-identically across job counts; the four named rules rewritten on
scratch copies with zero unexplained diagnostic diffs. **Kill:** if the Rust-side providers
(`semantic_mir`, `cfg`, `solver`) cannot meet the cold gate on the whole core even with the sidecar
fixed, step 2's unit split must precede any rule work, and step 1 ships only the sidecar change.

### Step 2: units, shards, and routes (5–7 weeks)

**Build.** Units = Go packages (TS: tsconfig projects, deferred to step 5). Per-unit lowering of
structure facts, MIR, CFG, call sites, local summaries, in parallel, persisted as one binary shard per
unit with the keys of §4.4; the unit DAG in petgraph; cross-unit call-graph merge and SCC-ordered
summary closure with equality backdating; the sidecar invoked per module but emitting per-package
shards keyed on export-interface digests. Framework models as data: gin routes with `Group`/`Use`
receiver-copy semantics across registrar helpers, chi and net/http defaults, Watermill
subscribers, `decorator.Apply*` pass-through, `httptest` roots; `Routes<'_>` and a structure-facts
view (receivers, parameters, fields with tags, call arguments, body ranges).

**Rules writable:** `local/backend-endpoint-authority` in full (route inventory with effective
middleware, group inheritance through helpers, admin-gate reachability); `local/backend-http-handler-tests`
and `local/backend-domain-tests` through test→handler reachability instead of name conventions;
`review/core-public-entrypoint-review` as an exact route check; [`backend-routes-require-authentication`,
`backend-mutating-routes-require-csrf`, `backend-active-school-routes-require-guard`,
`backend-background-work-uses-watermill`].

**Gate 2:** whole-core `calls` warm no-change ≤ today's shallow warm + 5 s; one-file edit ≤ 15 s
end to end (sidecar shard + unit shard + affected SCCs + queries); cold ≤ 90 s / ≤ 4 GB; reports
byte-identical at 1, 4 and 6 jobs and across ten run permutations; route inventory a superset of the
current rule's with every difference explained; endpoint-authority under 1 s of rule CPU. **Kill:**
if the edit tier cannot get under 15 s, deep providers stay out of `check`'s default profile and
live in `review`/CI only, and step 3 is re-scoped to the review path.

### Step 3: interprocedural dataflow (6–8 weeks)

**Build.** IFDS/IDE tabulation over per-unit ICFGs with summary edges reused across callers and
composed bottom-up on the unit DAG; access paths with k = 2 (3 behind a budget); models-as-data for
sources (HTTP request parts from `Routes`, message payloads from subscribers, configured secret
names), sinks (GORM terminals, publish, exec, log), sanitizers and propagators; per-unit step budgets
and a run deadline; `DataFlow<'_>::flows` with user patterns, each flow carrying its path, precision
and unknowns; rule-attached `codeFlows`; a taint corpus of at least 40 cases derived from the two
packs' error-flow, transaction, context and tenant-scope rules, with must-not-report twins.

**Rules writable:** `local/no-swallowed-publish-errors` (publish error value reaches a `return` on
every path); [`sync-append-tx-scoped` (appender argument aliases the transaction callback parameter),
`backend-context-propagation` (`*gorm.DB` receiver derived from `WithContext(ctx)`),
`backend-tenant-queries-require-school-scope` (chain predicate on the tenant column, through scopes
and helpers), `backend-adapter-typed-errors`, `lifecycle-purge-checks-legal-hold` (dominated by a
legal-hold check)].

**Gate 3:** corpus precision ≥ 90% and recall ≥ 70%; L4 probes ≥ 9 of 10 positives with all twins
clean; whole-core `dataflow` cold ≤ 120 s / ≤ 6 GB, warm ≤ 25 s, edit ≤ 20 s; every budget trip
visible as an unknown on the affected flow. **Kill:** if precision on the corpus stays under 80%
after two iterations, ship the solver only behind `review` with the corpus published, and move the
effort to step 4's CFG queries, which the control-flow rules need regardless.

### Steps 4–6 (after the recommendation's horizon)

4. **CFG completeness and general control-flow queries** (3–4 weeks): loops, `defer`/`panic`
   edges, try/catch; `Cfg<'_>::dominates`/`on_every_path`; interprocedural guard proof through the
   call graph. Rules: [`audit-auth-events-recorded`, `backend-handler-deny-uses-access-record` in
   full].
5. **The TS leg** (6–10 weeks): tsconfig projects as units with the same shards; the `tsc` sidecar
   as the primary resolver with per-project keys; React/TanStack/Next models; the frontend at
   `calls` under 120 s / 6 GB cold. Rules: [`frontend-no-raw-fetch` with binding resolution],
   `local/frontend-api-client-imports` with re-export following.
6. **Provenance and edits** (3–4 weeks): per-finding unknowns and budgets in every format; ranged
   multi-edit fixes with an apply path.

## 6. The honest cost

**Today, as shipped**, deep capabilities are unusable at any scope above one package: 7.7 GB and
30 s cold for `control_flow` on 41 files; 6.1 GB and 38 s warm for `calls` on the same files; the
whole backend killed at 18 GB in the last full attempt **(quoted, v0.3.10)**.

**Projected budgets if steps 0–3 land** (all estimates; the gates above are what make them binding):

| OAIZ `core` module, 4 jobs | Today (shallow, v0.4.4) | Deep ON after step 1 | after step 2 | after step 3 |
|---|---|---|---|---|
| cold, wall | 10.5–10.7 s (full repo) | ≤ 60 s (gate) | ≤ 90 s (gate; shards written) | ≤ 120 s (gate) |
| cold, tree peak | ~0.5 GB | ≤ 4 GB (sidecar 1.5–2 GB, polint ≤ 2 GB) | ≤ 4 GB | ≤ 6 GB |
| warm no-change | 5.4 s (full repo) | ≤ 20 s (sidecar cached; deep recomputed) | ≤ shallow + 5 s | ≤ 25 s |
| one-file edit | 7.1 s (full repo) | sidecar re-run ~15 s + recompute | ≤ 15 s | ≤ 20 s |
| disk for shards | ~190 MB of JSON layers | + sidecar NDJSON | + ~0.3–0.6 GB binary shards (estimate from 35 MB per 41 files, after dropping program-wide dumps) | + summaries |

What stays possible on the 6-core / 22 GB box: all of it, with headroom for a second process, *if*
the gates hold. What is not possible on that box: today's design at module scope, and the TS
frontend's deep tiers at any point before step 5.

The include-tests question is a product decision with a measured price: the typed layer with tests
is 4.8 GB / 21 s warm instead of 1.5 GB / 6 s. The test-coverage rules want test facts, not test
*bodies* in the deep solvers; the recommendation is test facts from the syntax tier and the
symbols sidecar, with test variants excluded from the semantic and dataflow units by default.

## 7. Recommendation

Do the five step-0 repairs now, then build the deep stack as one per-unit pipeline in the order
typed calls → units and routes → dataflow, each step gated on named consumer rules and the measured
budgets above, with deep providers held out of the default `check` profile until gate 2's edit tier
passes.

- **Before step 1:** gate 0, which is repair and takes days. Its most important line is that the
  catalog `calls` request drops from 38 s / 6.1 GB to under 8 s / 1 GB warm by defaulting the
  domains materialization, and that the symbols sidecar loads OAIZ on any host.
- **Before step 2:** gate 1. The signal that matters is whether the Rust-side providers can hold
  60 s / 4 GB on the whole core once the sidecar is scoped; if not, step 2 moves ahead of any rule
  work, and that is known in week 3, not month 3.
- **Before step 3:** gate 2's edit tier (≤ 15 s) and byte-identical reports across job counts and
  permutations. Without that, dataflow would be a CI-only capability, which should be decided
  consciously rather than discovered.

## 8. Still unmeasured

- The Rust-side deep providers on the whole core at v0.4.4 with a scoped sidecar: step 1's gate is
  the first time that number exists; everything above 41 files in this file is a quoted v0.3.10
  measurement or an estimate.
- Shard sizes in a binary codec, and the warm cost of restoring them; the per-file density above
  comes from JSON rows that include program-wide dumps.
- The precision of any deep tier on real code. Gate 3's corpus does not exist yet.
- Whether the sidecar's export-data mode changes any sidecar row for in-repo packages (it should
  not, since the roots are still loaded from source; M1 shows identical SSA counts, not identical
  rows).
- The TS frontend at v0.4.4: M6 is 100 files; the 2,461-file number is linear extrapolation.
- The runtime dividend of structure facts on the consumers' scanner code, still owed from
  [12](12-next-direction.md) step 1.
- Adoption. OAIZ and the Go+TS monorepo are separate owners; the rules named per step are the
  pull, and none has been rewritten yet.

## Appendix A: measured numbers in one place

| Measurement | Result |
|---|---|
| OAIZ `core`: packages / non-test files / lines / dependency packages (with tests) | 380 / 2,846 / 623,274 / 1,623 (2,622) |
| catalog context: packages / files / lines / dependency packages | 9 / 41 / 7,608 / 859 (257 std, 60 in-module) |
| SSA call sites, whole core: total / static / invoke / builtin / func value | 199,121 / 153,815 / 12,725 / 31,188 / 1,393 |
| CHA whole core: nodes / edges; invoke sites by target count 1 / 2–3 / 4–10 / >10 | 106,846 / 753,004; 1,167 / 3,142 / 5,683 / 2,215 |
| VTA whole core: edges; invoke sites by target count 1 / 2–3 / 4–10 / >10; time | 182,565; 5,720 / 669 / 384 / 456; 5.8 s |
| export-data load, core, tests off: warm total / HWM; after one-file edit | 5.7 s / 1.52 GB; 5.7 s / 1.53 GB |
| export-data load, core, tests on: warm total / HWM | 20.6 s / 4.76 GB (cold 241 s incl. build cache) |
| sidecar-mode load, core, `AllPackages`: total / HWM / bodies | 21.0 s / 7.43 GB / 271,888 |
| sidecar inside polint, core, tests off: elapsed / peak heap / tree RSS | 43.9 s / 6.9 GB / 7.3–7.9 GB |
| `control_flow`, catalog, broken symbol graph: cold / warm / unknowns | 28–30 s, 7.7–8.2 GB / 3.8 s, 0.45 GB / 1,112 (1,072 `dynamic_property`) |
| `control_flow`, catalog, working symbol graph: cold / warm / unknowns | 29.6 s, 7.69 GB / 6.6 s, 0.53 GB / 64 (24 conversions) |
| `calls`, catalog, working symbol graph, warm | 38.0 s, 6.10 GB; `abstract_domains` 29.4 s, +2.7 GB, 1.08 M facts |
| symbols sidecar on the catalog scope, Go 1.27.1 | 1.3 s; 3,226 symbols, 13,710 references, 445 cross-package symbols |
| sidecar NDJSON for the catalog scope | 35 MB; 19,089 callsite, 16,512 instantiated_type, 14,644 receiver_type, 7,537 dynamic_dispatch rows |
| TS cell `oaiz-ui` `calls`: cold / warm / unknowns | 22.6 s, 2.12 GB / 28.8 s, 2.11 GB / 489 |

## Appendix B: the twenty rules that need depth, and the query each would write

From the read-only pack audit. OAIZ ids are `local/…` and `review/…`; the Go+TS monorepo's ids are
shown without their vendor prefix.

| Rule | Today | Query a rule author would write | Facts needed | Step |
|---|---|---|---|---|
| `local/backend-endpoint-authority` | regex routes; substring "reachability" in one package; disclaims being a call graph | for each route (method, path, handler) with effective middleware chain M: level(M) = declared; if admin, every path from the handler to any repository write passes a gate call | routes with group inheritance; call graph through decorator generics and repository interfaces; dominance | 1–2 |
| `local/no-swallowed-publish-errors` | `if … Publish … != nil {` header and a `return` line | the error result of every `Publisher.Publish` call reaches a `return` on every path | callee type; error-value dataflow; CFG | 3 |
| `local/actor-handler-inputs` | regex on struct bodies | each `decorator.{Command,Query}Handler[T, _]` instantiation's T has exactly one `access.Actor` field and no caller-UUID field | generic instantiation; field types | 1 |
| `local/backend-http-handler-tests`, `-gorm-adapter-tests`, `-domain-tests` | test-name conventions; `_test.go` read from disk | for each handler H: some test in the sibling `service/` reaches H through the router with a 401/403 subtest | test→code call graph; routes; `*gorm.DB` field types | 2 |
| `local/backend-integration-tenant-isolation-tests` | regex over `registry.go` literals | for each registered provider: a test in its package reaches `httptest.NewServer` and asserts the credential header | literal facts; test call graph | 2 |
| `review/gorm-adapter-query-index-review` | diff-gated `contains(".Where(")` | columns in each GORM chain on model M ⊆ an index declared for M's table | ORM chain model; SQL migration facts | 3 |
| `review/core-public-entrypoint-review`, `review/actor-boundary-review` | token ordering; keyword presence | routes whose effective chain lacks auth; handlers where an org id reaching a repository call does not originate from `access.Actor` | routes; dataflow | 2–3 |
| `review/iac-secret-holder-before-use` | regex tokens vs changed ranges | a const added in this diff has no reference also added in `iac/**` | cross-file references | 1 |
| `backend-routes-require-authentication` (+ CSRF, active-school) | guard must be an argument of the same registration call | effective chain contains the guard; mutating ⇒ CSRF | routes with group `Use` | 2 |
| `backend-handler-deny-uses-access-record` | file contains `Forbidden` and not `access.Record(` | every path returning a Forbidden error is dominated by `access.Record` (depth ≤ 2) | dominance; intra-package call graph | 1, 4 |
| `backend-tenant-queries-require-school-scope` | tenant column within 19 lines of `.Model(` | each GORM terminal on a tenant table has a predicate on the tenant column, through scopes and helpers | ORM chain; `*gorm.DB` dataflow | 3 |
| `backend-context-propagation` | `.db.X(` on a line without `WithContext(` | the `*gorm.DB` receiver is derived from `WithContext(ctx)` | local alias/dataflow; types | 3 |
| `backend-background-work-uses-watermill`, `-subscribers-use-app-handlers` | `go func(` / `.Create(` tokens | goroutine closures in app/service reach only `Publish`; subscriber handlers reach an app `Handle` and no GORM write | call graph with closures and interface dispatch | 2 |
| `sync-append-tx-scoped`, `backend-transaction-policy` | arg name contains `tx`; `rfind` + brace counting | the appender argument aliases the enclosing `Transaction` callback parameter; `Handle` does not call another `Handle` | alias/dataflow; AST nesting; callee types | 3 |
| `lifecycle-purge-checks-legal-hold` | legal-hold token "nearby" | every delete on a retained table is dominated by a legal-hold check that returned no-hold | dominance; ORM table identity | 4 |
| `audit-auth-events-recorded` | three hard-coded files; bespoke scanners | every exit of `Handle` is preceded on its path by exactly one `authAudit.record*` with matching outcome | CFG; call identity on a field | 4 |
| `backend-adapter-typed-errors`, `-domain-typed-errors` | every `fmt.Errorf` flagged | an error returned from an exported adapter method is constructed by `customerrors.*` or passes `MapDatabaseError` | error dataflow; callee types | 3 |
| `backend-domain-contract-boundary` (+ 3,709-line private parser) | hand-rolled implements proof; nine spoof fixtures | T in a consumer adapter implements the consumer-local interface I and no foreign contract | method sets; implements | 1 |
| `frontend-no-raw-fetch` | lexical scan; "aliases escape by design" | a call whose binding resolves to global `fetch`/`WebSocket` or a forbidden import, through aliases and re-exports | TS bindings; module graph | 5 |
| `backend-http-route-component-tests`, `service-http-component-test-evidence` | test name contains handler name | as the handler-tests row | test call graph; routes | 2 |
