# Call Graph

`CallGraph<'_>` is a preview SDK view over resolved call edges between
functions. Requesting it derives the `call_graph` capability.

It serves the best call edges the engine has. Every analysis stage adds to the
same graph: calls resolved from names, imports and declarations; Go's typed
frontend; value-flow analysis of the functions that reach a call through
variables, parameters, returns, object properties and imports; function
summaries; the declared types of receivers; and package models. There is one
call graph, and each edge says how far it can be trusted (see
[Tiers](#tiers)), so a rule chooses what it walks.

Use it when a policy is about the graph itself: what a handler calls, who calls
a gate, whether one function reaches another within a depth bound, and the
paths between them. Functions are the `FunctionId`s of the `Functions<'_>`
view, so a rule finds its roots and targets there and asks the graph about
them. For reachability policies written as event patterns, `Calls<'_>`
([calls.md](calls.md)) remains the query-object view.

## Example

```rust
use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/handlers-reach-admin-gate",
    description = "Every handler must reach the admin gate.",
    severity = "error"
)]
fn handlers_reach_admin_gate(
    ctx: &mut RuleCtx<'_>,
    functions: Functions<'_>,
    graph: CallGraph<'_>,
) -> RuleResult {
    let Some(gate) = functions.iter().find(|function| function.name == "requireAdmin") else {
        return Ok(());
    };
    let walk = CallGraphWalk::new(8).with_min_precision(CallEdgePrecision::SetupAware);
    for handler in functions.iter().filter(|function| function.name.starts_with("Handle")) {
        if graph.reachable(handler.id, walk).contains(gate.id) {
            continue;
        }
        ctx.report(Diagnostic::error(
            ctx.rule_id(),
            ctx.file_path(handler.file),
            handler.span.diagnostic_range(),
            format!("{} does not reach requireAdmin", handler.name),
        ));
    }
    Ok(())
}
```

## Query methods

| Method | Meaning |
|--------|---------|
| `callees(caller)` | Every edge out of `caller`. |
| `callers(callee)` | Every edge into `callee` (a function in the scanned sources). |
| `edges()` | Every edge. |
| `reachable(root, walk)` | The functions and external callees reachable from `root`, breadth first, as a `CallGraphReach`. |
| `paths(from, to, walk, max_paths)` | Up to `max_paths` acyclic paths from `from` to `to`, each a list of edges. |

`CallGraphWalk::new(max_depth)` bounds a walk to `max_depth` edges;
`with_min_precision(minimum)` restricts it to edges at least that precise.
`CallGraphReach` answers `contains(function)`, `functions()`,
`externals()` (the labels of the external callees reached), and
`path_to(function)`, one shortest path as the edges it takes.

Every method returns its results in a deterministic order: the same sources
and configuration give the same answers on every run and at every job count.

## Edges

A `CallGraphEdge` is one resolved candidate callee of one call site:

- `caller`: the calling function;
- `callee`: `Function(FunctionId)` for a function in the scanned sources,
  `Symbol(SymbolId)` when a symbol but no function fact backs the callee, or
  `External(label)` for anything else;
- `file` and `span`: the call expression;
- `precision` and `algorithm`: how certain the edge is and how it was resolved.

A dynamic call with several candidates contributes one edge per candidate.
When two stages answer the same call site with the same callee, the view keeps
one edge, at the stronger of their claims.

Unresolved call sites are not edges, and neither is an answer that names a
candidate without claiming it (rejected, unreachable, or missing setup). Open
sites stay visible: `polint unknowns --cap calls` lists them, and
`ctx.completeness()` reports them. A site can have edges and still be open,
when an analysis found some callees but could not show they are all of them.
Type conversions written like calls (`Kind(raw)`) are not edges either.

External labels name what has no function of its own in the scanned sources:
`go:func:fmt.Println` or `go:func:(*net/http.Client).Do` for a dependency's
function or method, `go:builtin:panic` for a builtin, `go:func:example.com/app.Run$1`
for a function literal, and the abstract callee a type-hierarchy edge names
(`go:interface-method:...`, `go:func-value:...`).

A function literal is part of the function that declares it: the calls written
inside a literal are edges of that function. A call *of* a literal (a helper
invoking the callback it was given) is an edge to the literal's label, so a walk
does not continue from it into the declaring function's other calls.

### Tiers

Every edge belongs to one tier, read from its precision:

| Tier | Precision | What the edge claims |
|------|-----------|----------------------|
| Exact | `Exact`, `SetupAware` | The call reaches this callee. |
| Conservative | `Conservative` | The call may reach this callee; it is one of the candidates an analysis could not narrow. |
| Heuristic | `Heuristic` | A guess from names, apart from the other two. It claims nothing. |

| Precision | Meaning |
|-----------|---------|
| `Exact` | The callee is fixed by the program text and its types, or by a value-flow answer with a single function. |
| `SetupAware` | A complete answer for the analyzed configuration (build tags, loaded packages, resolved imports). |
| `Conservative` | One of the candidates of an analysis that could not narrow them, or the one answer of a site the engine marks ambiguous. |
| `Heuristic` | Resolved from names rather than types or value flow. |

An answer the engine marks ambiguous is never served as exact: the view
reports it as `Conservative` whatever its analysis claimed.

`CallEdgePrecision::at_least(minimum)` compares precisions; `Exact` is the
highest. A walk with `with_min_precision(CallEdgePrecision::SetupAware)` stays
in the exact tier; `with_min_precision(CallEdgePrecision::Conservative)` adds
the conservative tier; a walk with no minimum also follows heuristic edges.

| Algorithm | Edges |
|-----------|-------|
| `Static` | Go static calls: functions, methods on concrete receivers. `Exact`. |
| `VariableTypeAnalysis` | Go interface-method and function-value calls, to every concrete type or function literal that flows to the call. `SetupAware`. |
| `ClassHierarchy` | Go dynamic calls variable-type analysis gives no callee for (typically an interface parameter of a function nothing in the program calls): every implementation in the program. `Conservative`. |
| `TypeHierarchy` | Go dynamic calls whose class-hierarchy answer lists more than 16 candidates, or that neither analysis resolves: one edge to the abstract callee. `Conservative`. |
| `Syntactic` | Calls resolved from names, imports and declarations: TypeScript and JavaScript calls, and Go calls in files the typed frontend did not load. `SetupAware` when a resolved symbol backs the callee, `Heuristic` when only the name does. |
| `PointsTo` | TypeScript and JavaScript calls of a function value: a callback, a parameter, a variable, a returned function, an object property or an imported binding. `Exact` when one function reaches the call, `Conservative` when several do. |
| `Summary` | Calls whose callee a function summary confirms. The edge claims no more than the summary or the call's own answer. |
| `Model` | Calls a framework model or a repository model declares, for behaviour the code alone does not show (a framework invoking a handler). A model answer the engine cannot narrow is `Conservative`. |

`TypeHierarchy` edges also come from the TypeScript type checker when the type
sidecar is on: a call whose receiver's declared type names the method. They are
`Conservative` when the receiver may hold another implementation.

## Go setup and limits

Go edges come from the typed Go frontend, which loads each module with the Go
toolchain from its nearest `go.mod` (or the `[languages.go]` `module_roots`);
see [the consumer setup guide](../CONSUMER-SETUP.md). Its answers have these
limits:

- Variable-type and class-hierarchy answers are over-approximations of the
  program as loaded, except for calls made through reflection or `unsafe`.
  Neither distinguishes calling contexts: a helper that calls the function
  value it was given has an edge to every function or method value any caller
  passes it.
- Dependencies are loaded from export data, so a callee whose concrete type
  exists only inside a dependency's code is not a candidate.
- `_test.go` files are type-checked only when `[languages.go] include_tests`
  is `true`; calls in Go files the frontend did not load (and every Go call when
  no module root covers the files) resolve from names, with `Syntactic`
  algorithm and at most `SetupAware` precision.
- Calls into code reached through a framework (a router invoking a handler)
  are not call edges of the caller unless a model declares them (`Model`).
- A dynamic call lists at most `[solver.go] max_candidates_per_callsite`
  candidates (default 128). A call with more keeps the first ones in label
  order and records a `budget_exceeded` unknown for the rest, so the calls
  analysis reports itself incomplete.

`ctx.completeness().status_for("call_graph")` reports whether the run
finished, hit a budget, or left unknown call sites.

## TypeScript and JavaScript call sites

Every call written in a TS/JS function is a call site of that function,
including calls in parameter defaults (`function f(x = g())`), destructuring
defaults and computed keys (`const { a = g() } = o`), and the operands of
unary and binary expressions (`!function () { ... }()`). A call through a
comma sequence, `(0, m.f)(x)` as compilers emit for imports, is a call of its
last operand, `m.f`, run with `this` undefined.

## Optional use

A rule that can do without the graph requests `Option<CallGraph<'_>>`. When the
`call_graph` capability is unavailable the parameter is `None` and the rule
still runs; a rule that requests `CallGraph<'_>` directly is not run and the
check reports a `polint/capability` diagnostic instead.
