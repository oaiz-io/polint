# Policy Query Preview

The v1.4 policy-query surface is a preview SDK for repo-local semantic
policies. It promotes useful call, control-flow, and data-flow behavior without
exposing raw CFGs, call graphs, data-flow graphs, solver internals, provider
rows, dense IDs, or `AnalysisDb`.

The public style has one shape:

1. Request a typed preview view in the `#[polint::rule]` function signature.
2. Construct one plain query object with `Query::new(required...)`.
3. Set explicit option fields when needed.
4. Run one method on the view.
5. Report each returned `PolicyViolation` through
   `violation.diagnostic(ctx.rule_id(), "...")`.

There is no string query language, fluent builder DSL, closure filter DSL, or
public graph traversal API for this preview surface.

## Views And Capabilities

| View | Capability | Backed Query |
|------|------------|--------------|
| `Events<'_>` | `events` | `events.matching(EventPattern::call(...))` |
| `Calls<'_>` | `calls` | `calls.forbidden_reachable(ReachQuery)` |
| `ControlFlow<'_>` | `control_flow` | `control.missing_guard(GuardQuery)` and `control.missing_cleanup(LifecycleQuery)` |
| `DataFlow<'_>` | `dataflow` | `flow.forbidden(FlowQuery)` |

Rules request these views exactly like stable fact views:

```rust
use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/no-secret-logs",
    description = "Secret-like values must not reach logs.",
    severity = "error"
)]
pub(crate) fn no_secret_logs(ctx: &mut RuleCtx<'_>, flow: DataFlow<'_>) -> RuleResult {
    let mut query = FlowQuery::new(
        SourcePattern::secret_like(["token", "password", "apiKey"]),
        SinkPattern::logger(),
    );
    query.barriers = BarrierPattern::call_any(["redact", "mask_secret"]);
    query.minimum_precision = PolicyPrecision::Heuristic;
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

## Query Objects

- `ReachQuery::new(EventPattern::call("target"))` checks whether a forbidden
  call/event is reachable. Options include `roots`, `include_tests`,
  `max_depth`, `max_paths`, `minimum_precision`, and `minimum_confidence`.
- `GuardQuery::new(event, guard)` checks whether a sensitive call event is
  missing a prior guard in the same function. Options include `max_depth`,
  `max_paths`, and `minimum_precision`.
- `LifecycleQuery::new(start, cleanup)` checks whether a start/acquire call is
  missing later cleanup in the same function. Options include
  `require_error_cleanup`, `max_depth`, `max_paths`, and `minimum_precision`.
- `FlowQuery::new(source, sink)` checks whether a source can reach a sink.
  Options include `barriers`, `max_depth`, `max_paths`, and
  `minimum_precision`.

`ControlFlow<'_>` queries are backed by refined call facts and CFG dominance or
post-dominance when relation rows exist. MIR operation order and source spans
are fallback ordering sources; raw CFG traversal remains private.

## Realistic Examples

These examples show the rule code and the application code being analyzed. The
comments in the application snippets mark the local convention the rule is
enforcing.

### Request Data To Shell Execution

Rule:

```rust
use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/no-request-to-shell",
    description = "Request data must not reach shell execution.",
    severity = "error"
)]
pub(crate) fn no_request_to_shell(ctx: &mut RuleCtx<'_>, flow: DataFlow<'_>) -> RuleResult {
    let mut query = FlowQuery::new(
        SourcePattern::http_request(),
        SinkPattern::call("exec"),
    );
    query.barriers = BarrierPattern::call_any(["validate_command", "allow_command"]);
    query.max_depth = 24;
    query.max_paths = 128;

    for violation in flow.forbidden(query) {
        ctx.report(violation.diagnostic(
            ctx.rule_id(),
            "request data reaches shell execution without validation",
        ));
    }

    Ok(())
}
```

Application code:

```ts
import express from "express";

const app = express();

function exec(command: string) {}
function validate_command(command: string): string { return command; }

app.get("/run", function handler(req, res) {
  // Violation: request-controlled data flows straight into the shell wrapper.
  exec(String(req.query.cmd));
});

app.get("/safe-run", function handler(req, res) {
  // Allowed: the request value crosses the local validation barrier first.
  exec(validate_command(String(req.query.cmd)));
});
```

### Sensitive Write Requires A Guard

Rule:

```rust
use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/guard-sensitive-write",
    description = "Balance writes require authorization.",
    severity = "error"
)]
pub(crate) fn guard_sensitive_write(
    ctx: &mut RuleCtx<'_>,
    control: ControlFlow<'_>,
) -> RuleResult {
    let mut query = GuardQuery::new(
        EventPattern::call("writeBalance"),
        GuardPattern::call_any(["authorize", "validate_payment"]),
    );
    query.max_paths = 10;

    for violation in control.missing_guard(query) {
        ctx.report(violation.diagnostic(
            ctx.rule_id(),
            "sensitive write requires authorization first",
        ));
    }

    Ok(())
}
```

Application code:

```go
package billing

func authorize(userID string) {}
func writeBalance(userID string, cents int) {}

func refundWithoutGuard(userID string, cents int) {
	// Violation: the sensitive write has no prior guard in this function.
	writeBalance(userID, -cents)
}

func refundWithGuard(userID string, cents int) {
	// Allowed: the guard call occurs before the sensitive write.
	authorize(userID)
	writeBalance(userID, -cents)
}
```

### Raw Admin API Reachability

Rule:

```rust
use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/no-raw-admin-reachable",
    description = "Production roots must not reach raw admin APIs.",
    severity = "error"
)]
pub(crate) fn no_raw_admin_reachable(ctx: &mut RuleCtx<'_>, calls: Calls<'_>) -> RuleResult {
    let mut query = ReachQuery::new(EventPattern::call("dangerousAdmin"));
    query.roots = vec![EventPattern::call("main")];
    query.include_tests = false;
    query.max_depth = 8;
    query.max_paths = 10;

    for violation in calls.forbidden_reachable(query) {
        ctx.report(violation.diagnostic(
            ctx.rule_id(),
            "raw admin API is reachable from a production root",
        ));
    }

    Ok(())
}
```

Application code:

```go
package main

func main() {
	// Violation: production startup reaches the raw admin function through
	// a helper call chain.
	handler()
}

func handler() {
	dangerousAdmin()
}

func dangerousAdmin() {}
```

## Pattern Vocabulary

- `EventPattern::call("target")` matches direct call-event candidates. Events
  are syntax-first and report syntax precision with function-level ranges when
  no deeper policy view is requested; they upgrade to call/refined-call facts
  when those facts are already available. `EventPattern::write_field("field")`
  is reserved preview vocabulary and currently returns no backed matches.
- `SourcePattern::http_request()` matches supported HTTP trust-boundary source
  models such as path params, query strings, request bodies, request headers,
  and cookies.
- `SourcePattern::secret_like([...])` heuristically matches explicit names
  against source labels and local place names/projections.
- `SinkPattern::call("target")` matches exact call target candidates and checks
  whether a source reaches a call argument or receiver.
- `SinkPattern::logger()` matches a small heuristic logger family such as
  `console.log`, `log.Print`, `logger.info`, and similar names.
- `GuardPattern::call_any([...])` and `BarrierPattern::call_any([...])` match
  explicit call-name lists. Barrier matching is conservative; generated
  templates should still be reviewed against local sanitizer behavior.

## Evidence

`PolicyViolation::diagnostic(...)` emits a normalized scalar evidence header:

- `policy_query`
- `policy_query_version`
- `query_digest`
- `policy_status`
- `policy_precision`

Query-specific evidence is flat and family-specific. Reachability diagnostics
include `root`, `target`, `path`, and `depth`. Control-flow diagnostics include
`required_guard` or `required_cleanup`, `control_scope`, `uncovered_path`, and
budget fields. Event diagnostics include `event`, `target`, and sometimes
`function` and `event_source`. Data-flow diagnostics include `source`, `sink`,
`path_status`, `path`, `barrier_status`, `required_barrier`, and budget fields.

## Precision, Status, And Unknowns

Policy results must be read with their evidence:

- `policy_status` can be `exact`, `heuristic`, `unknown`, `unsupported`, or
  `budget_exceeded`.
- `policy_precision` can be `exact`, `setup_aware`, `syntax`,
  `conservative`, `heuristic`, or `unknown`.
- `PolicyConfidence` is currently surfaced by reachable-call evidence where
  applicable.
- Budget limits are visible through policy evidence instead of being treated as
  complete absence proofs.
- Cap-filtered unknown reports are available through
  `polint unknowns --cap events|calls|control_flow|dataflow --format json`.
- An empty result list is not a proof. It can mean "no operation matched", "the
  relation the query needed was never computed", or "every operation was
  cleared". Control-flow queries can separate the middle case from the others
  with `report_unknown_coverage`; see
  [control-flow.md](control-flow.md#unestablished-dominance).
- The run report says which of those happened. `summary.rules[].observed_events`
  counts the operations a rule's policy queries examined, and
  `summary.rules[].outcome` separates "ran and matched nothing" from "never ran
  because a provider failed". See
  [AGENT-PLAYBOOK.md](../AGENT-PLAYBOOK.md#reading-summary-an-empty-report-is-not-a-proof).

Unsupported preview vocabulary returns no backed policy matches until a later
phase promotes real facts. Setup gaps should produce `polint/capability`
diagnostics rather than running rules with placeholder facts.

## Template Starters

`polint new-rule <lang> <name> --template <id>` creates repo-local policy
scaffolds using the same query-object style for the supported language/template
pairs. TypeScript supports:

- `request-to-shell`
- `secret-to-log`
- `pii-to-analytics`
- `sensitive-write-guard`
- `transaction-cleanup`
- `raw-reachable-api`
- `ssrf`
- `dangerous-html`
- `unsafe-deserialization`
- `user-file-path`

Go currently supports:

- `sensitive-write-guard`
- `transaction-cleanup`
- `raw-reachable-api`

Policy templates are not currently generated for `js` or `generic` rules.

Templates include positive and negative fixture cases under `.polint/tests/`.
They are starting points to edit for local APIs, not built-in rules enabled by
polint. Generated data-flow starters use bounded `max_depth` and `max_paths`
settings large enough for the included HTTP request fixtures; tighten or raise
those caps based on the size and precision needs of the local codebase.

## Limits

The preview surface is intentionally narrower than the private analysis engine:

- Raw CFG, call graph, semantic graph, data-flow graph, provider, solver, MIR,
  and evidence-store APIs are not public rule-authoring APIs.
- Current control-flow queries are same-function call-event checks.
- Current data-flow queries are bounded and conservative.
- Name-based source, sink, guard, and barrier patterns are heuristic unless the
  underlying evidence says otherwise.
- Broader domain taxonomies such as SQL, SSRF, HTML, deserialization, analytics,
  PII, and file paths are represented by editable templates over current backed
  primitives, not by complete built-in detection.
