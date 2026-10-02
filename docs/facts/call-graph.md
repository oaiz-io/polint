# Call Graph

`CallGraph<'_>` is a preview SDK view over resolved call edges between
functions. Requesting it derives the `call_graph` capability.

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
Unresolved call sites are not edges; `polint unknowns --cap calls` lists them.
Type conversions written like calls (`Kind(raw)`) are not edges either.

External labels name what the scanned sources do not declare:
`go:func:fmt.Println` or `go:func:(*net/http.Client).Do` for a dependency's
function or method, `go:builtin:panic` for a builtin, and the abstract callee a
type-hierarchy edge names (`go:interface-method:...`, `go:func-value:...`).

### Precision and algorithm

| Precision | Meaning |
|-----------|---------|
| `Exact` | The callee is fixed by the program text and its types. |
| `SetupAware` | A complete answer for the analyzed configuration (build tags, loaded packages). |
| `Conservative` | One of the candidates of an analysis that could not narrow them. |
| `Heuristic` | Resolved from names rather than types. |

`CallEdgePrecision::at_least(minimum)` compares precisions; `Exact` is the
highest.

| Algorithm | Edges |
|-----------|-------|
| `Static` | Go static calls: functions, methods on concrete receivers. `Exact`. |
| `VariableTypeAnalysis` | Go interface-method and function-value calls, to every concrete type or function literal that flows to the call. `SetupAware`. |
| `ClassHierarchy` | Go dynamic calls variable-type analysis gives no callee for (typically an interface parameter of a function nothing in the program calls): every implementation in the program. `Conservative`. |
| `TypeHierarchy` | Go dynamic calls whose class-hierarchy answer lists more than 16 candidates, or that neither analysis resolves: one edge to the abstract callee. `Conservative`. |
| `Syntactic` | Calls resolved from names, imports and declarations: TypeScript and JavaScript calls, and Go calls in files the typed frontend did not load. `SetupAware` when a resolved symbol backs the callee, `Heuristic` when only the name does. |

## Go setup and limits

Go edges come from the typed Go frontend, which loads each module with the Go
toolchain from its nearest `go.mod` (or the `[languages.go]` `module_roots`);
see [the consumer setup guide](../CONSUMER-SETUP.md). Its answers have these
limits:

- Variable-type and class-hierarchy answers are over-approximations of the
  program as loaded, except for calls made through reflection or `unsafe`.
- Dependencies are loaded from export data, so a callee whose concrete type
  exists only inside a dependency's code is not a candidate.
- `_test.go` files are type-checked only when `[languages.go] include_tests`
  is `true`; calls in Go files the frontend did not load (and every Go call when
  no module root covers the files) resolve from names, with `Syntactic`
  algorithm and at most `SetupAware` precision.
- Calls into code reached through a framework (a router invoking a handler)
  are not call edges of the caller.

`ctx.completeness().status_for("call_graph")` reports whether the run
finished, hit a budget, or left unknown call sites.

## Optional use

A rule that can do without the graph requests `Option<CallGraph<'_>>`. When the
`call_graph` capability is unavailable the parameter is `None` and the rule
still runs; a rule that requests `CallGraph<'_>` directly is not run and the
check reports a `polint/capability` diagnostic instead.
