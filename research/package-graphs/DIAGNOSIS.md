# Diagnosis: why the classical layer misses what it misses

Date: 2026-11-17. Baseline: polint v0.5.3 (`7aa3b3cf`) plus the direction
commit `46d237c0`. Everything measured here was measured on that checkout with
a fresh release build; the commands are in [Measurements](#measurements).

Research question: is the reading in
[ITERATION-1-CLASSICAL.md](ITERATION-1-CLASSICAL.md) right that "the solver is
not the gap; the gap is that JS/TS value flow is not lowered into constraints
completely and uniformly"?

## Verdict

**Corrected.** The reading was half right about lowering and wrong about the
solver. In order of weight:

1. **The points-to layer is discarded wholesale on every package above about a
   thousand call sites.** The TS/JS Andersen solve carries three global
   budgets sized for fixtures (`max_steps` 10,000, `max_objects_per_var` 64,
   `max_dynamic_vars` 512). When any of them trips, the solve marks *every*
   points-to set `BudgetExceeded`, and the edge derivation skips every set
   whose status is not `Present`. The result is zero resolved points-to edges
   for the whole package. Measured: axios, async and zod have **0** resolved
   `PointsToAssisted` edges; `polint inspect unknowns` on axios reports
   `budget_exceeded: points_to.max_dynamic_vars,solver.max_steps`. The walker's
   rows reach the call graph only through this solve, so they are lost with
   it. Only commander (809 call sites) stays inside the budget, which is why
   it is the one package where `refined` beats `base`. Confidence: high
   (code read and measured; see [1](#1-the-budget-latch)).
2. **The solver has no interprocedural rule.** It propagates copies, field
   loads and field stores. It has no rule "when a call site's callee set gains
   function f, copy the arguments into f's parameters, f's return into the
   site's result, and the receiver into f's `this`". That rule is the whole of
   what makes an inclusion-based analysis a call-graph analysis for a
   higher-order language. Today the walker does that job instead, by
   recognisers, with a depth cap of 16, a per-walk node budget of 4,096, four
   rounds for module summaries and a single pass for function returns. The
   walker is not a fixed point and cannot be made one by adding recognisers.
   Confidence: high ([2](#2-no-call-rule)).
3. **Lowering is incomplete**, as ITERATION-1 said, but the list is more
   specific than "assignment, property, call, return": closures that capture
   parameters are never walked with the binding; function objects with
   properties (`_.init = init`) are not objects; rest parameters, `pop`,
   `filter`/`slice`/`reverse` and other platform calls have no model;
   `Function.prototype.call/apply` is unmodelled; a `(0, f)(x)` callee is
   `Unknown`; `this` inside an object-literal method has no receiver; an
   unknown computed key has no "any property" fallback. The 20 traces below
   put each miss on one of these. Confidence: high per trace, listed.
4. **The typed plan's 15 wrong exact targets are a join defect, not a checker
   defect.** When the checker's declaration span matches no polint function,
   the join falls back to the narrowest *enclosing* function around the
   declaration's name, and the edge keeps `Exact` precision. `this.emit(...)`
   in a subclass of `stream.Transform` becomes an exact edge to the class
   itself; `isFunction$1(x)` becomes an exact edge to the wrong of two
   same-named initialisers. Confidence: high ([4](#4-the-typed-plans-wrong-exact-targets)).
5. **A cost regression is hiding in the refined plan.** On this checkout the
   axios refined dump takes 105 to 112 s (committed baseline: 2.1 s), 91 to
   95 s of it in the `polint.semantic_graph` provider, which is where the
   walker runs. The machine was loaded (load average 9 to 26) during every
   run here, but commander reproduced its committed time within 10% in the
   same conditions. The committed cost section carries commit `956786ec`,
   before #139 landed. Confidence: medium that the regression is real and
   caused by #139's wider import resolution turning on the walker's
   cross-module passes on axios; WP0 must re-measure on a quiet machine
   before anything else ([5](#5-the-walkers-cost-on-axios)).
   **Re-measured: it does not reproduce.** Axios refined is 2.0 to 2.6 s over
   five runs on the same walker code, see §5.

What stays true from ITERATION-1: more type information does not find the
misses; the four groups of causes are the right groups; the solver's
constraint vocabulary is usable as the target of lowering.

What changes in the plan: the solver is fixed first (no global latch, the
call rule, shared sets), then lowering is moved onto it construct by
construct, and the walker is retired as each construct lands. Details in
[implementation/README.md](implementation/README.md).

## 1. The budget latch

Code (all `crates/polint/src/`):

- `analysis_neutral/points_to/solver.rs:13-28` — `PointsToBudget` defaults
  10,000 / 64 / 512.
- `solver.rs:227-247` (`add_all`) — exceeding `max_objects_per_var` sets
  `budget_exceeded` for the whole solve and returns; `solver.rs:249-272`
  (`object_slot`) — the 513th `(object, field)` slot sets it; `solver.rs:355`
  — the 10,001st worklist pop ends the solve.
- `solver.rs:366-410` (`to_result`) — one `status` for every set:
  `BudgetExceeded` when the flag is set.
- `ts/points_to.rs:148-153` — `if set.status != PointsToStatus::Present {
  continue; }`: no derived edge from any set of an exhausted solve.
- `analysis_neutral/solver/engine.rs:158-220` — the TS policy's edges and its
  budget status do enter the solver output (the exclusion in the comments is
  for the neutral `PointsToPolicy`, whose `points_to` is `Some`).
- `analysis_neutral/refined_calls/ts_js.rs:73-110` — a site is "solver
  resolved" only through a present derived edge; every other unresolved site
  gets an `Unresolved` row.

Measured on the four packages (fresh dumps, `compare.py` over the base and
refined dumps; the committed report counts sites differently, see
[Measurements](#measurements)):

| package | call sites | `PointsToAssisted` resolved | of which `SetupAware` (field path) | of which `Heuristic` (walker rows) | sites with a target, base → refined |
|---|---|---|---|---|---|
| commander | 809 | 161 | 0 | 161 | 42 → 185 |
| async | 3,462 | 0 | 0 | 0 | 1,172 → 1,172 |
| axios | 10,165 | 0 | 0 | 0 | 2,683 → 2,683 |
| zod | 62,844 | 0 | 0 | 0 | 26,516 → 26,583 |

Two consequences the earlier documents got wrong:

- "On zod the refined layer resolves 274 call sites more than base, none on an
  executed edge" was read as the points-to layer resolving unexecuted sites.
  The sites it resolves are in `src/**/tests/**` (vitest files the driver
  never runs), and they come from the `DirectPlusFramework` tier (the
  test-framework recogniser), which the refined plan enables and the base
  plan does not. In the fresh dump: 67 gained sites, all in test files, all
  `FrameworkModel`, 0 executed. The points-to layer contributed none. The
  exact count differs from the report's 274 because the report counts sites
  "without target" over its own site key; the composition is the same.
  Confidence: high on composition, medium on the count.
- "Refined recall equals base recall on zod, axios and async" is not a
  statement about what the walker or the solver can find. It is the latch.
  Nothing above commander's size has ever been measured with the points-to
  layer on.

Even where the solve survives (commander), the field-sensitive path adds no
edge: all 161 resolved points-to edges are `Heuristic`, which means a walker
row sits on the site. A site resolved by object-model constraints alone would
be `SetupAware` (`ts/points_to.rs:200`, `refined_calls/provider.rs:664-675`).
There are none.

## 2. No call rule

`solver.rs:93-106` is the whole propagation: copy, load, store, field load,
field store. `ConstraintKind::CallConstraint` (`semantic_graph/constraints.rs:68`)
is read only to pick which points-to sets become call edges
(`ts/points_to.rs:148-205`); it adds no constraint. MIR's `Call` operation
lowers the callee as `MirValue::Unknown { evidence: <callee text> }`
(`ts/mir/lower.rs:3734-3739`), so no MIR-derived constraint can connect a
call to its arguments either. `project_value_constraints`
(`semantic_graph/build.rs:356-412`) projects `PlaceRef` and `CallReturn`
copies between places, with no place for a parameter of a callee.

So the inclusion-based solver resolves exactly the calls whose callee value
reaches the site through copies and fields that start at a function literal
in the same lexical environment, plus what the walker hands it as a finished
`site <- function` copy. Everything that crosses a call boundary in either
direction (argument to parameter, return to result, receiver to `this`,
captured variable to closure) is the walker's job, and the walker does it with
recognisers that are not a fixed point:

- `ts/callable_flow/extract.rs:3656-3663` — argument flow into parameters only
  for an identifier callee that is a same-file function declaration
  (`function_declarations`); `extract.rs:4088-4099` binds a fresh callee env
  and walks the body once.
- `extract.rs:2275-2422` (`collect_expression`) — `FunctionExpression` and
  `ArrowFunctionExpression` fall through to `_ => {}`: a closure body that
  captures a parameter is never walked with that parameter bound.
- `extract.rs:500-564` — module export summaries: four rounds, and the pass
  runs with `function_returns` empty ("Function-return seeding is not active
  while building export summaries"); `extract.rs:574-623` — function return
  summaries: one pass, "return chains across summarized functions are not
  threaded".
- `extract.rs:4545-4555` — invocation depth 16; `extract.rs:53-61` — walk depth
  64 and a budget of 4,096 node visits per outermost walk;
  `extract.rs:204` — keyed replay after 4,096 walks.

Each of those is a place where a value stops flowing for a reason that has
nothing to do with the program.

## 3. Twenty traces

Ten from "function values that move", ten from "objects built at run time";
zod, axios, async and commander. Each row names the observed edge, what the
engine answers on the typed plan, and the exact missing piece. "Missing" is
stated in constraint terms where a constraint would do, and as a model where
only a model does. Confidence is high unless marked.

### Function values that move

| # | site → observed callee | what the engine does | missing |
|---|---|---|---|
| 1 | zod `core.ts:88` `initializer(inst, def)` → `classic/errors.ts:50 initializer` (also `schemas.ts:390`) | `initializer` is a parameter of `$constructor`, which every schema module calls as `core.$constructor("X", (inst, def) => ...)`. Argument→parameter flow exists only for same-file identifier callees (`extract.rs:3656`), and the solver has no call rule. No target. | The call rule: at a resolved site, `arg_i → param_i` of each callee, across modules. |
| 2 | zod `core.ts:124` `fn()` → `memoizer.ts:228` arrow | `for (const fn of deferred)` where `deferred` is `inst._zod.deferred`, an array a method in another module pushes closures into. `inst` is a parameter; no object reaches it, so `_zod`/`deferred` loads find nothing. | Parameter object flow (call rule with `this`), `Array.prototype.push` as an element store, `for…of` as an element load. |
| 3 | zod `classic/errors.ts:40` `make(this)` → `errors.ts:58` arrow | `_lazyMethod(proto, key, make)` is a same-file declaration called with an arrow, so the parameter is bound (`extract.rs:4088`). The call `make(this)` is inside the `get()` of the descriptor literal, a closure; `collect_expression` does not enter closure bodies, so the binding never reaches the site. | Closures: a function body is lowered once with its captured variables as places shared with the enclosing body; no "walk with env". |
| 4 | zod `ZodError.ts:264` `mapper(issue)` → `ZodError.ts:220` function expression | `const mapper = _mapper \|\| function (issue) {...}`; `_mapper` is a parameter. The walker keeps "unknown ∪ literal" as unfollowed and emits nothing (right for an exact claim, wrong to say nothing). | The conservative tier: `mapper` may be the literal; emit the edge as conservative and leave the site open with reason "parameter". |
| 5 | zod `parseUtil.ts:33` `map(fullIssue, …)` → `locales/en.ts:4 errorMap` | `maps = errorMaps.filter(m => !!m).slice().reverse()`; `errorMaps` is a parameter built from `ctx.*` and `getErrorMap()` (a module-level registry written by `setErrorMap` from another module). | Platform models for `filter`/`slice`/`reverse` (elements pass through), the call rule, and a module-level variable as a heap cell written across modules. |
| 6 | axios `fetch.js:66` `fn(...args)` → `fetch.js:122` and `:146` arrows | `const test = (fn, ...args) => { try { return !!fn(...args) } ... }`; called as `isRequestSupported && … && test(() => {...})`. `test` is a `const` arrow, not a `function` declaration, so `function_declarations` has no entry and no argument is bound. | Arguments bound by the call rule for any callee value, not by callee syntax; spread arguments as "all remaining parameters". |
| 7 | axios `http.js:569` `asyncExecutor(_resolve, _reject, …)` → `http.js:674 dispatchHttpRequest` | `wrapAsync(async function dispatchHttpRequest …)`; the parameter is called inside the executor closure of `new Promise((resolve, reject) => {...})`. Closure bodies are not walked with the binding (#3), and `new Promise(executor)` has no model. | Closures as in #3; platform model: `Promise` constructor calls its executor with two fresh functions. |
| 8 | async `asyncify.js:111` `callback(error, value)` → `internal/map.js:22` arrow | `callback` comes from `args.pop()` in the closure `asyncify` returns, which is invoked through `_iteratee = wrapAsync(iteratee)` in another module. Rest parameters, `pop`, a returned closure and a cross-module call chain; the walker's single return pass and depth bound stop it. | Rest parameters and `pop` (element load), return→result flow across modules, the call rule. |
| 9 | async `auto.js:239` `callback(null, results)` → `internal/once.js:8 wrapper` | `callback = (0, _once2.default)(callback)`: the callee is a sequence expression, which the call extractor reports as `Unknown` (`sequence_interop`, 154 edges on async), so nothing flows back from `once`. | Extraction: `(0, e)(…)` is a call to `e` (WP1); then return→result flow from `once`'s nested declaration `wrapper`. |
| 10 | commander `command.js:1488` `fn()` → `command.js:1363`, `:1515`, `:1603` arrows | `_chainOrCall(promise, fn)` is a method; callers write `this._chainOrCall(x, () => ...)`. The class walk binds `this` and resolves the member call (heuristic), but argument flow into a method's parameters does not exist. | The call rule for member calls. |

### Objects built at run time

| # | site → observed callee | what the engine does | missing |
|---|---|---|---|
| 11 | zod `classic/errors.ts:51` `$ZodError.init(inst, issues)` → `core.ts:73 init` | `$ZodError` is `$constructor(...)`'s return `_`, a function declaration with `Object.defineProperty(_, "init", { value: init })` (`core.ts:135`). A function is not an object with fields for the walker; the export is a call result, so the export summary carries nothing (function returns are not seeded in that pass). | Function objects as heap objects (`Alloc` for every function literal and declaration, `FieldStore` for `defineProperty` with a constant key), return→result flow, imports as copies of the exported place. |
| 12 | zod `from-json-schema.ts:710` `objectSchema.passthrough()` → `schemas.ts:1692 passthrough` (and the `util.ts:1179` getter) | The instance's methods are installed by `installMembers(proto, protoMembers)` through `defineBound`, a `for…in` over the member literal and `Object.defineProperty(proto, key, { get() { … fn.bind(this) } })`. Nothing in the engine reads a `for…in` over a literal as "each of these keys". | Two things: a conservative property-name fallback (any function stored under `passthrough` anywhere may be the target) so the site is not empty; and either a general rule "`for…in` over an object whose keys are known enumerates those keys" or a zod package model. The getter is an implicit edge the bench reports apart. |
| 13 | zod `schemas.ts:1492` `inst.check(…)` → `schemas.ts:185 check` | `inst` is the initializer's parameter; the object is `this` of `_` at `new ZodString(def)`, whose members come from #12's installation. | As #12 plus `this` binding at `new` (fresh object whose prototype is the constructor's `prototype`). |
| 14 | zod `schemas.ts:298` `parse.parse(this, …)` → `core/parse.ts:23 fn` | `import * as parse`; `export const parse = _parse(errors.$ZodRealError)`; `_parse` returns the arrow `fn`. The direct tier stops because the export is a call result, not a literal (correct). The walker's export-summary pass runs with function returns disabled, so `parse.parse` has no callables. | Return→result flow as a constraint, solved to a fixed point with everything else; no separate "summary pass". |
| 15 | zod `schemas.ts:1740` `new ZodObject(def)` → `core.ts:118 _` (63 edges on zod) | `export const ZodObject = core.$constructor("ZodObject", …)`; the class is `_`, returned through `return _ as any`. Same block as #14, across a module boundary. The typed plan goes further and claims `util.ts:558 shallowClone` at one such site (see §4). | As #14, plus `new` on a function value binds `this` to a fresh object. |
| 16 | zod `schemas.ts:264` `this.clone()` → `schemas.ts:202 clone` (30 `this_member` edges on zod) | Both are methods of the object literal passed as `$constructor`'s third argument. The walker binds `this` for class bodies, export-alias objects and prototype assignments, not for an object literal that is an argument. | `this` in a method of an object literal: the literal itself as the receiver, conservative (Jelly and ACG do this); exact once the call rule binds `this` from the real receivers. |
| 17 | axios `adapters.js:88` `adapter.get(config)` → `fetch.js:718 getFetch` | `adapter = knownAdapters[String(nameOrAdapter).toLowerCase()]`: a computed key over a literal with three known properties. The walker resolves computed keys only from constants or `for…in` names. | An unknown key on an object with known fields loads the union of its fields, as a conservative edge, with the site open ("computed key"). |
| 18 | axios `fetch.js:209` `headers.getContentLength()` → `AxiosHeaders.js:478 get`, `:203 value` | The method is defined by `Object.defineProperty(obj, methodName + accessorName, { value: function … })` with a name built from strings at run time. No static rule names it. | Nothing classical. The site must be open with reason "computed key"; stage 2 or a package model finds it. (Honest gap; 2 of 675 axios edges.) |
| 19 | axios `Axios.js:297` `dispatchRequest.call(this, newConfig)` → `dispatchRequest.js:66` (96 `call_apply_bind` edges) | `dispatchRequest` is a default import of a function declaration. `.call` is marked `CallApplyBind` and left. | Platform model: `f.call(t, a…)` calls `f` with `this = t` and the rest as arguments; `apply` likewise with an array; `bind` allocates a bound function object whose call forwards. |
| 20 | commander `command.js:601` `target.parseArg(value, previous)` → `option.js:183` arrow | `this.parseArg = (arg, previous) => …` is stored in `Option.choices()` and `Argument.choices()`; `target` is a parameter that holds either. The typed plan claims `argument.js:100` alone and is wrong (§4). | `this.p = f` inside a method is a `FieldStore` on every receiver that reaches `this`; a parameter of union type gives a conservative edge to both. |

Reading the twenty together: fourteen need the call rule or the heap rules it
implies (1, 2, 5, 6, 7, 8, 10, 11, 13, 14, 15, 16, 20 and half of 12); three
need platform models (5, 7, 19); two need the conservative tier to say
"maybe" where the exact tier says "unknown" (4, 17); two need extraction
(9, and the `sequence_interop` family); one is honestly open (18). None needs
a type.

## 4. The typed plan's wrong exact targets

`analysis_neutral/refined_calls/ts_types.rs:440-498` (`core_function_for`):
when the checker's declaration span is not a polint function span, the join
takes the functions whose name matches the declared name, or *all* functions
when none matches, and picks the narrowest one containing the declaration's
name position. `precision_for` (`:319-326`) keeps `Declared` at `Exact`.

- underscore `isFunction$1(obj[key])` (7 sites): `var isFunction =
  tagTester('Function')` is later reassigned in an `if`. The checker's
  declaration is the `var`; its name position lies inside no function but the
  module, and the name-matched pool is empty, so the narrowest enclosing
  function is the arrow returned by `tagTester` — one of the two values, as an
  exact edge.
- axios `this.emit('progress', …)` and `this.push(_chunk)`: `emit`/`push` are
  declared in `@types/node`; the checker's declaration for the call lands on
  the class `AxiosTransformStream` and the join returns the class.
- commander `this.emit('option:…')`, `this._exitCallback(...)`: the same, on
  `Command extends EventEmitter` and on an instance field holding a closure.
- zod `new ZodDefault({...})`: the checker's declaration is the `const
  ZodDefault = core.$constructor(...)` initialiser; the fallback picks
  `shallowClone`, an unrelated function around the anchor.
- commander `target.parseArg(...)`: `target` is `Argument | Option` by
  declaration; the `Declared` dispatch returns one declaration and the join
  one function.

The fix is a join rule, not a checker change: a `Declared` target whose
declaration is not a function (a class, a variable, an external declaration)
yields no exact edge, and a declared receiver that is a union or a structural
type yields `UnionMember` candidates at conservative precision. WP4 carries
it with these fifteen as fixtures.

## 5. The walker's cost on axios

Fresh dumps on this checkout (release build, `RUST_LOG=polint::kernel::stage=info`):

| package | refined dump, committed report | refined dump here | largest stage here |
|---|---|---|---|
| commander | 1.7 s | 1.8 s | `semantic_graph` 0.9 s |
| async | 1.9 s | 5.8 s | — |
| axios | 2.1 s | 105 s, 112 s (two runs) | `semantic_graph` 91.5 s, 94.8 s |
| zod | 48.8 s | 85.7 s | — |

Load average during the runs was 9 to 26 (other builds on the host), so the
async and zod figures are not comparable. The axios figure is 50× and stable
across two runs, with commander unaffected in the same minute, so load does
not explain it. The committed cost section names commit `956786ec`; the
walker's inputs changed in #139 (import links through barrels, single-literal
bindings), after which the cross-module summary passes in `extract.rs:500-623`
run over far more resolved imports on axios (72 files, ESM barrels). WP0 must
re-measure on a quiet machine and bisect if it holds. If it holds, the refined
plan at v0.5.3 is 50× the base plan on axios, which makes the cost targets in
ITERATION-1 unreachable by tuning and reachable only by retiring the walker.

**Re-measured: it does not hold.** Five `bench.py report --no-cost` runs on the
measurement-baseline branches (release build, the same `lib/**/*.js` set of 72
files, and no change to `semantic_graph/` or `ts/` lowering since `7aa3b3cf`).
The host was loaded throughout, with a load average of 100 to 190:

| run | axios base dump | axios refined dump | `semantic_graph` |
|---|---|---|---|
| 1 | 0.64 s | 2.03 s | 0.21 s |
| 2 | 0.67 s | 2.32 s | 0.28 s |
| 3 | 0.74 s | 2.60 s | 0.25 s |
| 4 | 0.63 s | 2.19 s | 0.21 s |
| 5 | 0.70 s | 2.41 s | 0.24 s |

The refined dump is 2.0 to 2.6 s against the committed 2.1 s, and the walker's
share is about 10%. The refined plan is 3.2 to 3.5 times the base plan. That
ratio comes from the providers the refined plan adds (`evidence`,
`type_value_alias`, `abstract_domains`), not from the walker. The 105 s figure
did not reappear on the same code, so there is no commit to bisect, and the
walker's retirement is justified by what it misses, not by its cost. The cost targets are
judged on the cost baseline, taken separately with the load average recorded
beside it.

## Measurements

All commands run from the repository root with
`CARGO_TARGET_DIR=/home/dev/.t3-stage/worktrees/polint/fix-rule-host-disk-usage/target`
(the one shared build directory). Scratch output lived in `/tmp/polint-diag`
and was deleted.

```
cargo build --release --locked -p polint --bin polint
cargo test -p polint --lib --all-features --locked --release --no-run
# per package and plan, as tests/js-analysis-bench/report.py run_dump does:
RUST_LOG=polint::kernel::stage=info POLINT_JS_BENCH_MANIFEST=<tree>\t<out> \
  POLINT_JS_BENCH_CAPS=<plan caps> <test binary> \
  eval::js_bench::tests::dump_js_bench_facts --exact --ignored --test-threads 1
# base caps: syntax,call_graph,symbols,references
# refined caps: syntax,calls,control_flow,dataflow,call_graph,symbols,references
polint inspect unknowns --no-cache .        # in the axios tree
```

The comparison script read both dumps and the observed oracle, counted
refined edges by `(tier, algorithm, status, precision, has_function)`, listed
sites with a target in the refined dump and not the base dump, grouped them
by file, and intersected them with the oracle's executed sites. Its logic is
eleven lines of Python over the dump schema (`polint-js-bench-facts-2`); it
is reproduced in WP0's bench change as a committed measure, which is why it
was not kept as a script.

Site counts here are counts of `call_sites` rows in the dump. The committed
report counts sites after mapping to observed spans and over instrumented
files only, hence zod 62,844 here against 48,838 there.

## What was not checked

- The walker was read at the entry points named above and in `collect_call_expression`,
  `collect_function_flow_invocation`, `collect_expression`, the emit functions
  and the summary passes, not line by line through all 9,649 lines. The
  traces rely on those paths; a recogniser elsewhere that would have found
  one of the twenty edges is possible but would not change the verdict,
  because the solve that carries its output is discarded on three of the four
  packages.
- The budget latch was confirmed on axios through `inspect unknowns`. On zod
  and async it is inferred from the zero resolved points-to edges and the
  same code path; the `unknowns` command was not run on them.
- The cost regression was not bisected.
