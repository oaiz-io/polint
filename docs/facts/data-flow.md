# Data-Flow Facts

`DataFlow<'_>` is a v1.4 preview SDK view for source-to-sink policy queries.
Requesting it derives the supported `dataflow` capability.

See [policy-queries.md](policy-queries.md) for the shared query-object style,
evidence header, precision/status vocabulary, unknown semantics, and template
starter workflow.

The public surface is intentionally policy-level. A rule either asks a
bounded question with one `FlowQuery` and `flow.forbidden(query)`, or, for Go,
an interprocedural one with a `FlowSpec` and `flow.flows(&spec)` (see
[Flows](#flows)). polint does not expose raw data-flow nodes, graph edges,
solver IDs, summaries, provider rows, MIR IDs, or `AnalysisDb` to rule authors.

```rust
#[polint::rule(id = "local/no-secret-logs", description = "Secret logs", severity = "error")]
pub(crate) fn no_secret_logs(ctx: &mut RuleCtx<'_>, flow: DataFlow<'_>) -> RuleResult {
    let mut query = FlowQuery::new(
        SourcePattern::secret_like(["token", "password", "apiKey"]),
        SinkPattern::logger(),
    );
    query.barriers = BarrierPattern::call_any(["redact", "mask_secret"]);
    query.minimum_precision = PolicyPrecision::Heuristic;
    query.max_depth = 8;
    query.max_paths = 20;

    for violation in flow.forbidden(query) {
        ctx.report(violation.diagnostic(
            ctx.rule_id(),
            "secret-like value reaches logging without redaction",
        ));
    }

    Ok(())
}
```

## Query Shape

- `FlowQuery::new(source, sink)` requires one `SourcePattern` and one
  `SinkPattern`.
- `query.barriers` accepts `BarrierPattern::none()` or
  `BarrierPattern::call_any(["redact", "mask_secret"])`.
- `query.max_depth` and `query.max_paths` cap the private path search.
- `query.minimum_precision` filters found paths by their precision. Unknown,
  budget-exceeded, and unsupported results remain visible even when their
  precision is below the found-path threshold, so uncertainty is not turned into
  a silent pass.

`FlowQuery` has no alternate builder, string query language, closure filter, or
public graph traversal API; `FlowSpec` is the separate question type of
[`flows`](#flows).

## Template Starters

`polint new-rule ts <name> --template <id>` can scaffold data-flow policy
starters for `request-to-shell`, `secret-to-log`, `pii-to-analytics`, `ssrf`,
`dangerous-html`, `unsafe-deserialization`, and `user-file-path`. The current
Go template set is limited to non-data-flow starters:
`sensitive-write-guard`, `transaction-cleanup`, and `raw-reachable-api`. Policy
templates are not currently generated for `js` or `generic` rules. Each
generated data-flow rule uses `DataFlow<'_>`, `FlowQuery`, explicit
source/sink/barrier patterns, and positive/negative fixtures under
`.polint/tests/rules/`.

Templates are editable repo-local examples. They use the backed primitives below
and intentionally do not claim a complete built-in taxonomy for PII, SSRF, HTML,
deserialization, analytics, or file paths.

## Supported Patterns

`forbidden` backs these patterns. With the Go program's flow bodies loaded (any
plan that requests `dataflow` on Go sources), Go results come from the same
solver as [`flows`](#flows): one result per sink reached, its evidence path
located step by step; other languages keep the value-flow graph search below.

- `SourcePattern::http_request()` matches trust-boundary source models for HTTP
  route params, query strings, request bodies, request headers, and cookies. The
  private provider introduces these source models into matching handler
  parameter places before bounded path search.
- `SourcePattern::secret_like([...])` matches explicit source names supplied by
  the rule author against source labels and MIR place names/projections. This is
  heuristic name matching, not exact secret detection.
- `SinkPattern::call("target")` matches exact call target candidates from
  existing call/refined-call facts and checks whether a source reaches an
  argument or receiver place for that call.
- `SinkPattern::call_argument("target", position)` narrows the same match to one
  zero-based argument position. Positions index the call's source-order
  arguments and never its receiver, so "reaches *some* argument" becomes
  "reaches argument N" without text-matching argument names. Variadic packing is
  not modelled: a position past a variadic callee's fixed parameters names
  whichever source argument sits there. The position participates in the query
  digest, so results for different positions never share a cache entry.
- `SinkPattern::logger()` matches a small heuristic logger target family such as
  `console.log`, `log.Print`, `log.Printf`, `log.Println`, `logger.info`,
  `logger.warn`, and `logger.error`.
- `BarrierPattern::call_any([...])` suppresses a found violation when the found
  path crosses a matching sanitizer/barrier call. If any found path reaches the
  sink without such a call, that uncovered path is reported.

## Result Evidence

Diagnostics built through `violation.diagnostic(...)` include the common policy
evidence header documented in [evidence.md](evidence.md), plus data-flow
evidence such as:

- `policy=forbidden_flow`
- `policy_query=data_flow.forbidden`
- `query_digest`
- `source`
- `sink`
- `path_status`
- `path`
- `path_edge_count`
- `barrier_status`
- `required_barrier` when configured
- `supported_scope=bounded_private_data_flow`
- `requested_max_depth`
- `requested_max_paths`
- `budget_reason` when a cap prevents a complete answer

Found flows that depend on heuristic source or sink patterns report heuristic
status/precision honestly. Unknown paths and budget-exceeded paths produce
visible policy results instead of silently passing.

## Limits

`forbidden` is useful for repo-local policies such as secret-to-log and
request-to-dangerous-call checks, but it is still preview:

- It does not prove perfect sanitizer semantics or taint-killing transfer
  functions.
- It does not yet cover the full planned sink taxonomy for SQL, raw HTML/JSX,
  SSRF URLs, file paths, analytics, PII, and outbound network clients.
- It does not expose context-sensitivity controls.
- Extension/model-pack authoring remains internal.
- Raw `Cfg<'_>` and raw data-flow graph APIs remain reserved. Call edges are
  public through the separate `CallGraph<'_>` view ([call-graph.md](call-graph.md)).

## Flows

`flow.flows(&spec)` answers an interprocedural question over a Go program and
returns a `FlowAnswer`: every sink the tracked values reach, one `Flow` per sink
site, and the unknowns that limited the search anywhere.

```rust
#[polint::rule(id = "local/request-to-sql", description = "Request data must not build SQL text", severity = "error")]
pub(crate) fn request_to_sql(ctx: &mut RuleCtx<'_>, flow: DataFlow<'_>) -> RuleResult {
    let spec = FlowSpec::new()
        .source(FlowSource::model("http_request"))
        .sink(FlowSink::model("sql"))
        .sanitizer("example.com/app/sqlsafe.Quote")
        .untracked(FlowValueKind::Context)
        .untracked(FlowValueKind::Boolean)
        .untracked(FlowValueKind::Number);
    let answer = flow.flows(&spec);
    for found in &answer.flows {
        if found.precision <= FlowPrecision::SetupAware {
            ctx.report(found.diagnostic(ctx.rule_id(), "request data builds SQL text"));
        }
    }
    Ok(())
}
```

### Questions

- Sources: `FlowSource::model(kind)` (the built-in `http_request` and
  `message_payload`, or a repository kind), `call_result(function)`,
  `call_argument_pointee(function, argument)` (what a pointer argument points to
  after the call, such as a binder's target), `parameter_of_type(type)`,
  `named(names)` (parameters, address-taken locals, package variables and
  struct fields read whose names contain one of the names, ignoring case: a
  heuristic) and `callback_parameter(function, argument, parameter)` (a
  transaction callback's handle).
- Sinks: `FlowSink::model(kind)` (the built-in `sql`, `exec`, `log` and
  `publish`, or a repository kind), `call(function)` (any argument or the
  receiver), `call_argument(function, position)` (counted from 0, without the
  receiver) and `returned()` (a value the function holding the source returns).
- Functions are named by qualified name (`example.com/app.F`,
  `(*example.com/app.Store).Find`), by `Type.Method`, or by last name.
- `sanitizer(function)`: the result of a call to it carries none of its
  arguments' taint. The models' sanitizers always apply.
- `untracked(kind)`: values of that kind (`Context`, `Boolean`, `Number`) never
  carry taint and never reach a sink. Injection questions usually declare all
  three; a question about a context, or a value stored in one, must not declare
  `Context`.
- `deeper_paths()`: three field steps per access path instead of two.

### Answers

A `Flow` has its `source` and `sink` (`FlowStep`: file, path, line, column,
function), the `sink_argument` it reaches, its `steps` source to sink, its
`precision` and its `unknowns`. `flow.diagnostic(rule_id, message)` reports at
the sink with the path as structured evidence, one located step per edge, which
SARIF output renders as a code flow.

`precision` is the least certain step:

- `Exact`: every call passed has one known callee, and every library function
  passed is modelled.
- `SetupAware`: a call was resolved by variable-type analysis.
- `Conservative`: the path passes a class-hierarchy candidate, a call with no
  known callee, a package variable, or a library function without a model,
  which is assumed to pass its arguments to its result.
- `Heuristic`: the path enters a function literal assumed to be called by the
  library function it was passed to.

`FlowAnswer::unknowns` and `Flow::unknowns` name what limited the search: a
package's step budget (`UnitBudget`), the question's deadline (`Deadline`),
calls nested too deep or a recursive cycle that did not settle (`CallDepth`),
and dynamic calls with no known callee (`UnresolvedCall`). A flow a budget cut
off is missing from the answer; `is_complete()` is false whenever a budget, the
deadline or the depth limit cut anything, so an empty answer is then "not
proved", not "no flow".

### How Go is analysed

The typed Go frontend lowers every function the program builds (its packages,
closures, generic instances and the wrappers the compiler synthesizes) from SSA
into a flow program: value slots, and the copies, loads and stores through
field, element and pointer steps, calls with their candidate callees (the static
callee, else variable-type analysis, else the class hierarchy up to a limit),
returns and closures. Branches whose condition is a constant are pruned first.
The solver then:

- follows a value through a function and through memory: an access path is a
  slot and up to two field steps (three with `deeper_paths`); a path cut there
  covers everything below it;
- enters a callee with the tracked part of an argument and reuses that callee's
  summary for every caller entering it the same way; summaries of recursive
  functions are recomputed until none changes, and one recomputed to the same
  answer wakes no caller;
- returns taint that reaches a source function's results or what its pointer
  parameters point to to every caller of it, and taint written to a package
  variable to every function using it;
- reports one flow per sink site, through the summary nearest the source.

Memory is not flow-sensitive: a field, element or map entry that once held a
tracked value keeps it after it is overwritten, and a map's keys and values
share one place. Values flow into a function literal through its captured
variables; one passed to a library function is assumed called by it
(`Heuristic`). Questions are answered at rule time, a few seconds each on a
program of hundreds of packages; each package has a step budget of two million
facts per question and each question a two-minute deadline.

TypeScript programs get no flows from `flows` yet.

### Models

The built-in models (request data a gin or net/http handler reads, Watermill
message payloads, GORM and `database/sql` query text, commands run, standard
and structured logging, Watermill publishing, `strconv` and `uuid` parsers as
sanitizers, comparisons as opaque, JSON and XML decoding as propagators) apply
alongside a repository's own tables in `.polint/models/*.toml`:

```toml
[[go_flow_source]]
kind = "tenant"
receivers = ["example.com/app/auth.Actor"]
methods = ["TenantID"]

[[go_flow_sink]]
kind = "sql"
function = "example.com/app/db.Raw"
arguments = [0]

[[go_flow_sanitizer]]
function = "example.com/app/sqlsafe.Quote"

[[go_flow_opaque]]
function = "example.com/app/text.Equal"

[[go_flow_propagator]]
function = "example.com/app/codec.Decode"
from = 0
to = 1
```

A table names functions by `function` (or `functions`), or by `receivers` and
`methods` (a method of a named type, through a pointer or not, or an interface
method). Sources take `output = "result"` (the default) or
`output = "argument"` with an `argument`, or a `parameter_type`; sinks take
`arguments` or `arguments_from`; propagators move taint from argument `from`
(or `"receiver"`) into what argument `to` points to. An invalid table is a
`polint/flow-model` warning and is left out; the other models still apply. The
models take part in the run's analysis digests.

