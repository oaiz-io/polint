# JS/TS Call-Graph Recall: Missed-Edge Buckets

Date: 2026-11-15

Baseline: v0.5.1. Measured with `make js-bench` on the seven pinned packages in
`tests/js-analysis-bench/inputs.toml`. Observed edges come from the runtime
tracer in `tests/js-analysis-bench/observed/`. Each missed observed edge is put
in one cause bucket by `report.py`. That makes the buckets a heuristic triage of
the call site and the observed callee, not a proof of why polint missed the
edge.

The plans:

- **base**: `syntax,call_graph,symbols,references`
- **refined**: base plus `calls,control_flow,dataflow`
- **typed**: refined plus `type_sidecar`, with TypeScript checker facts

Wrong-complete means a call site where polint names other targets and does not
mark the site unresolved. It is 0 for every package, plan, and construct
fixture, both at v0.5.1 and after this change. No fix in this branch may raise
it.

## Recall Before and After

Found / observed edges, v0.5.1 → this branch.

| package | observed | base | refined | typed |
|---|---|---|---|---|
| async | 637 | 319 → 324 | 323 → 328 | 338 |
| axios | 675 | 99 → 270 | 99 → 270 | 511 |
| chalk | 72 | 2 → 17 | 5 → 17 | 17 |
| commander | 318 | 14 → 21 | 117 → 119 | 248 |
| dayjs | 117 | 0 | 0 | not run → 0 |
| underscore | 262 | 156 → 157 | 157 → 158 | 202 |
| zod | 1308 | 217 → 719 | 217 → 719 | 721 → 764 |
| **total** | 3389 | 807 → 1508 (24% → 44%) | 918 → 1611 (27% → 48%) | 2037 → 2080 (60% → 61%) |

## Cost Per Plan

Wall time and peak RSS of one fact dump per package, TypeScript sidecar
included. Single runs on a shared host, so treat differences under about 10%
as noise.

| package | dump s refined / typed, v0.5.1 → now | peak MB refined / typed, v0.5.1 → now |
|---|---|---|
| axios | 2.6 / 3.7 → 2.3 / 3.3 | 281.8 / 304.6 → 284.8 / 305.0 |
| zod | 93.9 / 110.3 → 79.4 / 102.0 | 3826.0 / 4358.4 → 3898.7 / 4396.1 |
| commander | 1.7 / 2.3 → 1.7 / 2.5 | 144.8 / 234.1 → 145.1 / 234.4 |
| async | 2.0 / 2.8 → 2.0 / 2.9 | 264.1 / 282.4 → 263.5 / 282.7 |
| chalk | 0.1 / 0.6 → 0.1 / 0.6 | 40.6 / 213.8 → 40.4 / 212.5 |
| dayjs | 0.1 / 0.2 → 0.1 / 0.6 | 35.8 / 97.7 → 34.9 / 222.5 |
| underscore | 0.7 / 1.2 → 0.7 / 1.2 | 121.2 / 224.8 → 118.0 / 225.0 |

The fixes in this branch are lookups in tables the symbol graph already builds.
They add no measurable time or memory: every package is within noise of
v0.5.1. zod refined ran faster this time, which is single-run variance, not a
claimed speed-up. The dayjs typed cost rises because the typed plan now runs on
dayjs (see below).

The typed plan is still the biggest recall lever on code the TypeScript checker
can type. On zod it costs about 28% more time and 13% more memory than refined.
On small JavaScript packages its fixed cost is the sidecar start, about 0.5 s
and 100 to 170 MB. After this branch, zod base (719) is close to zod typed
(764), so most of what the checker added on zod was import and namespace
resolution that the symbol graph can do by itself.

The refined layer adds nothing over base on zod or axios. It costs 79 s and
3.9 GB on zod, and none of its targets lands on an observed edge that base
misses.

At v0.5.1 the dayjs typed plan did not run. TypeScript's wildcard `include`
never matches `*.min.js`, so the generated tsconfig had no inputs. The bench now
lists the sources explicitly. It now runs and finds the same 0
edges as base. That is a bench harness change in this branch, not a recall
change.

## Missed Edges by Cause

### Base plan

| cause | async | axios | chalk | commander | dayjs | underscore | zod | total |
|---|---|---|---|---|---|---|---|---|
| module_member |  | 190 |  |  |  |  | 466 → 90 | 656 → 280 |
| local_call | 24 → 19 | 156 → 42 | 13 → 0 | 15 → 8 |  | 53 → 52 | 88 → 31 | 349 → 152 |
| parameter_call | 59 | 17 |  | 8 |  | 27 | 114 | 225 |
| parameter_member |  | 12 |  | 71 |  |  | 134 → 90 | 217 → 173 |
| local_member | 1 | 38 |  | 22 |  | 1 | 111 | 173 |
| this_member | 4 | 9 |  | 125 |  |  | 31 | 169 |
| sequence_interop | 157 |  |  |  |  |  |  | 157 |
| callback | 15 | 41 → 40 |  | 53 |  | 5 | 43 | 157 → 156 |
| not_extracted | 1 | 20 |  | 1 | 117 |  | 9 | 148 |
| call_apply_bind | 45 | 32 |  |  |  | 19 |  | 96 |
| import_binding |  | 57 → 1 | 2 → 0 |  |  |  | 25 → 0 | 84 → 1 |
| constructor | 2 | 2 |  | 7 |  |  | 63 | 74 |
| not_analysed |  |  | 47 |  |  |  |  | 47 |
| computed_key |  | 2 | 8 |  |  |  | 5 | 15 |
| object_literal_member | 10 |  |  |  |  |  | 2 | 12 |
| call_result_member |  |  |  | 2 |  | 1 |  | 3 |

### Typed plan

| cause | async | axios | chalk | commander | dayjs | underscore | zod | total |
|---|---|---|---|---|---|---|---|---|
| parameter_call | 59 | 17 |  | 8 |  | 26 | 114 | 224 |
| callback | 15 | 39 |  | 53 |  | 5 | 43 | 155 |
| sequence_interop | 154 |  |  |  |  |  |  | 154 |
| local_member | 1 | 22 |  | 2 |  |  | 101 | 126 |
| parameter_member |  | 12 |  | 4 |  |  | 86 → 85 | 102 → 101 |
| call_apply_bind | 45 | 32 |  |  |  | 19 |  | 96 |
| module_member |  | 3 |  |  |  |  | 90 → 64 | 93 → 67 |
| local_call | 17 | 14 |  | 1 |  | 9 | 45 → 29 | 86 → 70 |
| constructor | 2 |  |  |  |  |  | 63 | 65 |
| not_analysed |  |  | 47 |  |  |  |  | 47 |
| this_member |  | 3 |  | 1 |  |  | 30 | 34 |
| not_extracted | 1 | 20 |  | 1 | 0 → 117 |  | 9 | 31 → 148 |
| computed_key |  | 2 | 8 |  |  |  | 5 | 15 |
| object_literal_member | 5 |  |  |  |  |  | 1 | 6 |
| call_result_member |  |  |  |  |  | 1 |  | 1 |

Two cells are artefacts, not real changes. At v0.5.1 the dayjs typed plan did
not run, so its typed row has no causes and shows as `0 → 117`.
chalk `not_analysed` counts edges into `source/vendor/`, which polint's file
discovery skips as a vendor directory.

## Ranking: Edges Won Per Effort

Effort is a rough estimate: **S** is under a day with no new fact family, **M**
is a few days inside one module, and **L** needs the callable-flow walker or the
refined solver. For a done fix, the edge count is the base-plan gain measured on
the bench. For a planned fix, it is the remaining base-plan misses in the
buckets the fix targets. Those are upper bounds, because one bucket often mixes
several shapes.

| Rank | Fix | Buckets | Edges | Effort | Status |
|---|---|---|---|---|---|
| 1 | Follow imports through barrel re-exports to the declaring symbol | import_binding, module_member | +447 for ranks 1 to 3 together | S | done |
| 2 | Resolve `ns.name(...)` on namespace imports | module_member | (in rank 1) | S | done |
| 3 | Bindings that only ever hold one function literal; overload sets | local_call | (in rank 1) | S | done |
| 4 | Calls to a function merged with a same-named interface | module_member | +254 (zod) | S | done |
| 5 | Members of an imported default object literal | module_member | 190 (axios) | M | walker plan |
| 6 | CJS interop: `(0, x.default)(...)`, `_interopRequireDefault`, `exports.default =` | sequence_interop | 157 (async) | M | walker plan |
| 7 | `this.method()` inside classes and prototype methods | this_member | 169 | M | walker plan |
| 8 | Callbacks invoked by builtins (`forEach`, `then`, `map`, ...) | callback | 156 | M | walker plan |
| 9 | Code inside UMD / IIFE wrappers | not_extracted | 117 (dayjs) | M | walker plan |
| 10 | Parameter calls, parameter members, `call`/`apply`/`bind` on parameters | parameter_call, parameter_member, call_apply_bind | 494 | L | walker plan |
| 11 | Constructors made by factories (`$constructor`) | constructor | 74 | L | walker plan |
| 12 | Refined solver yields nothing on zod and axios | all | unknown | L | solver |

Ranks 1 to 4 are in this branch. They change the symbol graph
(`crates/polint/src/ts/symbol_graph.rs`, `crates/polint/src/symbol_graph/`), the
direct-call resolver (`crates/polint/src/analysis_neutral/calls/direct.rs`), and
the plumbing that hands symbol-graph links to that resolver. They do not touch
`callable_flow/extract.rs` or the refined solver, which a parallel performance
branch is changing. Ranks 5 to 11 need that walker and rank 12 needs the
solver. They are planned below for after the performance branch merges.

## Fixed in This Branch

- **Barrel re-exports.** An import of a name that a barrel re-exports with
  `export { x } from` or `export * from` now links to the declaring symbol
  instead of the barrel's binding. A name a barrel binds with
  `export * as ns from` names a module namespace object, not a symbol, so it
  stays unlinked, and it shadows a same-named name from an `export *` source. This removed almost all
  of the `stops-at-import-binding` silent outcomes: axios 57 → 1, zod 25 → 2,
  chalk 2 → 0.
- **Namespace member calls.** `ns.name(...)`, where `ns` comes from
  `import * as ns` and the barrels behind it, now resolves `name` through the
  module's export table. The frontend records each such call with the span of
  its callee member expression, and the resolver matches a call site to it by
  that span, so nothing is rebuilt from source text.
- **Single-literal bindings and overloads.** The TS frontend records, from the
  AST and the Oxc symbol table, which function each binding names: the literal
  initializer of a `const`, or of a `let`/`var` that is never written, never
  redeclared, and not a script's top-level binding; or the implementation of a
  function declaration that is never written (the declaration whose AST node
  has a body, so an overload signature is never the target). A call through a
  binding with exactly one such function is a direct target. A file that
  mentions `eval` or contains `with` gets no rows, because either can rebind a
  local by name.
- **Interface merged with a function.** Oxc gives `interface X` and
  `function X` one symbol, of kind interface. The row for that symbol comes
  from its own declarations, in its own scope, so a call through it targets
  that scope's function and never a same-named function elsewhere. A merged
  `let` or `var` gets a row only when it passes the single-value checks above.
  zod declares `interface $constructor` and `function $constructor` in
  `core.ts` and calls `$constructor(...)` 252 times, so this one shape
  accounts for zod base module_member 342 → 90.

Every fix has a unit test that failed before the fix.

## Walker Plan (After the Performance Branch Merges)

> Superseded on 2026-11-17 by
> [`../package-graphs/ITERATION-1-CLASSICAL.md`](../package-graphs/ITERATION-1-CLASSICAL.md).
> The constructs and fixtures listed here are still the list of what to cover.
> They are now covered by lowering value flow into constraints (WP1 and WP2
> there), not by extending the walker.

Line numbers are for `crates/polint/src/ts/callable_flow/extract.rs` at
`2f433fea`. They will move once the performance branch lands. Each item names a
construct fixture to add under `tests/js-analysis-bench/constructs/` first, so
that the fixture fails before the fix and the bench shows the gain.

### 1. Default-exported object literal members (`module_member`, ~190 edges, axios)

Shape: `import utils from './utils.js'` with `export default { isFunction,
forEach, ... }` at `lib/utils.js:1137`, called as `utils.forEach(...)`.

- In `bind_imported_value` (2810), when the imported default is an object
  literal, bind each shorthand or `key: ident` property to the identifier's
  value. `collect_variable_declaration` (2045) already does this for local
  object literals.
- Some members are made by a factory, such as
  `const isFunction = typeOfTest('function')` (`lib/utils.js:273`, factory at
  194). Those need returned-function flow, which the refined solver has.
  Expect them to resolve in refined only.
- Fixture: `constructs/default-object-import/`. Of the 60 members in the axios
  object, 41 are plain functions, so most of the 190 edges should resolve in
  base. The other 19 need refined.

### 2. CommonJS interop (`sequence_interop`, ~155 edges, async)

Shape (async, Babel output):

```js
var _applyEach2 = _interopRequireDefault(require('./internal/applyEach.js'));
exports.default = (0, _applyEach2.default)(_map2.default);
```

- Unwrap `(0, expr)` to `expr` in the callee. The text form is in
  `crates/polint/src/ts/mir/lower.rs:4593` (`callee_text`). The walker
  counterpart is in `collect_call_expression` (3353).
- Treat `_interopRequireDefault(require(p))` and `__importDefault(require(p))`
  as identity when the module sets `__esModule`. `.default` then resolves to
  the module's `exports.default`.
- Record `exports.default = f` and `exports.name = f` as module exports in
  `collect_commonjs_export_assignment` (2672).
- Fixture: `constructs/cjs-interop-default/`, a Babel-style two-file module.
  Expected edges: the `.default` call, plus the `exports.name` call.

### 3. `this.method()` in classes and prototypes (`this_member`, ~160 edges)

Shape: `this._registerCommand(cmd)` inside `class Command` (commander, 125 in
base, 40 in refined). zod has 31 that stay missed in typed.

- `walk_method_body_with_this` (730) and `collect_class_method_bodies` (963)
  bind `this` to the class only for method bodies. Extend the binding to arrow
  functions nested in a method, and to subclass methods. For subclasses, list
  inherited methods and mark the site unresolved when a subclass may override
  the method.
- `collect_prototype_method_this_flows` (681) covers `F.prototype.m = function`
  but not `Object.assign(F.prototype, {...})`.
- Fixture: `constructs/this-method-nested-arrow/`, a method with an arrow that
  calls `this.other()`. Expected: about 85 commander edges in base. commander
  refined already finds most of them.

### 4. Parameters (`parameter_call`, `parameter_member`, `call_apply_bind`, ~490 edges)

Shapes: `initializer(inst, def)` in zod `core.ts:88`, `arg.name()` in commander,
and `asyncFn.apply(this, args)` in async.

- `collect_function_parameter_flows` (4088) flows arguments to parameters only
  for direct calls with one known target. Extend it to every call site whose
  targets are all known, merging the argument sets.
- `collect_function_prototype_method_call` (4423) and
  `collect_bound_function_invocation` (4482) handle `f.call(...)` and
  `f.apply(...)` only when `f` is a known function. When `f` is a parameter,
  read it from the parameter's flow set.
- Parameter members (`arg.name()`) need the parameter's class from its
  callers, or the type sidecar. typed already recovers most of them
  (commander 71 → 4).
- This is the largest bucket and the riskiest for wrong-complete. Merged
  argument sets must keep the unresolved marker whenever any caller passes an
  unknown value.
- Fixture: `constructs/parameter-callback-two-callers/`. Expected: about 150
  edges in refined. base cannot get these without flow.

### 5. Callbacks invoked by builtins (`callback`, ~150 edges)

Shape: `arr.forEach(fn)` or `promise.then(onFulfilled)`. The construct
fixture's callback recall is 0/3, even in refined.

- `collect_native_object_call` (3953) and `collect_callback_parameter_flows`
  (5412) model a few builtins as callers of their argument. Add the array
  iteration methods, `Promise.prototype.then`/`catch`/`finally`, `setTimeout`,
  and `queueMicrotask`, and emit an edge from the call site to the callback.
- Whether the bench counts these as caller → callback edges depends on the
  tracer, which records the builtin frame. Check `observed/` before you count
  on the gain.
- Fixture: the existing callback construct. Expected: 3/3 constructs and about
  100 package edges.

### 6. Factory-made constructors (`constructor`, ~70 edges, zod)

Shape: `new ZodObject(def)`, where `ZodObject = $constructor("ZodObject",
init)` returns the class `_` at `core.ts:118`.

- This needs the return value of `$constructor` flowing to the binding. That is
  refined-solver work: returned-function flow, plus `this` binding in the
  returned class.
- `collect_this_object_member_flows` (5453) and `emit_member_targets` (7520)
  then need to bind `new X(...)` to the class's constructor.
- Fixture: `constructs/factory-class/`. Expected: about 60 zod edges in refined
  and typed.

### 7. UMD and IIFE wrappers (`not_extracted`, 117 edges, dayjs)

Shape: the dayjs bench input is `dayjs.min.js`, one minified UMD bundle:
`!function(t,e){"object"==typeof exports&&...}(this,function(){...})`.

- polint extracts no call sites inside the factory function. The site and the
  callee report as "not a polint call site" or "not a polint function". The
  wrapper's factory argument must be walked as a function body, in the
  extractor and in MIR lowering.
- A cheaper alternative is to point the bench at `dayjs/esm/`, which is the
  same code unminified. That measures the library rather than the bundle
  shape.
- Fixture: `constructs/umd-wrapper/`. Expected: most of the 117 edges.

### 8. Refined adds nothing on zod and axios

zod refined equals base at 719/1308, and axios refined equals base at 270/675.
The zod refined dump takes 79 s and 3.9 GB. One possible cause, not verified
here, is the solver hitting a work budget on the largest modules and falling
back to base targets. Check this on the performance branch before changing any
fact shape, because that branch changes the same code.

### Not planned

- **chalk `not_analysed` (47).** These edges call into `source/vendor/`, which
  polint's discovery skips by design. A repository that wants them can include
  the vendor tree in its own config.
- **`computed_key` (13).** `obj[key]()` with a key that is not constant. Keep
  these unresolved.
