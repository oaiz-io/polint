# Capability Support

polint derives rule capabilities from typed fact-view parameters in
`#[polint::rule]` function signatures. This planning is an engine detail; normal
users should see it through `polint check` diagnostics rather than a separate
debug command.

## Supported Facts

Current rule-author fact views include:

- `Imports<'_>`
- `ResolvedImports<'_>`
- `ModuleGraphFacts<'_>`
- `GoTests<'_>`
- `BranchObligations<'_>`
- `FileMetrics<'_>`
- `Functions<'_>`
- `FunctionMetrics<'_>`
- `ComplexityMetrics<'_>`
- `Symbols<'_>`
- `References<'_>`
- `StringLiterals<'_>`
- `JsxAttributes<'_>`
- `TsClasses<'_>`
- `TsComponents<'_>`

`References<'_>` implies symbol identity internally, so rules that request only
references still cause polint to derive the `symbols` capability needed to bind
resolved `ReferenceFact::target` values.

## Policy Query Preview Facts

v1.4 adds a small policy-query vocabulary to the public SDK:

See [policy-queries.md](policy-queries.md) for the shared policy-query syntax,
evidence, unknown, budget, and template semantics.

- `Events<'_>` derives capability `events`
- `Calls<'_>` derives capability `calls`
- `ControlFlow<'_>` derives capability `control_flow`
- `DataFlow<'_>` derives capability `dataflow`

`Events<'_>`, `Calls<'_>`, `ControlFlow<'_>`, and `DataFlow<'_>` are preview.
`Events<'_>` is syntax-first for direct call-event matching and upgrades when
deeper call facts are already present. `ControlFlow<'_>` uses refined call facts
and CFG-backed operation order for same-function guard/cleanup checks, with
MIR/source ordering only as fallback when CFG rows are absent. `Calls<'_>` and
`DataFlow<'_>` use the deeper provider-backed pipelines for reachable-call
checks and bounded source/sink/barrier data-flow checks. A rule can compile,
appear in
`polint inspect rule --format json`, show derived fact views, and execute
without `polint/capability` diagnostics when it requests these supported preview
views.

```rust
#[polint::rule(id = "local/no-secret-logs", description = "Secret logs", severity = "error")]
pub(crate) fn no_secret_logs(ctx: &mut RuleCtx<'_>, flow: DataFlow<'_>) -> RuleResult {
    let mut query = FlowQuery::new(
        SourcePattern::secret_like(["token", "password"]),
        SinkPattern::logger(),
    );
    query.barriers = BarrierPattern::call_any(["redact", "mask_secret"]);
    query.minimum_precision = PolicyPrecision::Heuristic;

    for violation in flow.forbidden(query) {
        ctx.report(violation.diagnostic(ctx.rule_id(), "secret reaches logs"));
    }

    Ok(())
}
```

This is the only public query-object style for the preview surface:
`Query::new(required...)`, explicit option fields, one view method, and
`PolicyViolation::diagnostic(...)`.

Policy diagnostics share a normalized evidence header: `policy_query`,
`policy_query_version`, `query_digest`, `policy_status`, and
`policy_precision`. Cap-filtered unknown reports are supported for preview
policy capabilities through
`polint unknowns --cap events|calls|control_flow|dataflow --format json`.

## Reserved Capabilities

Reserved raw capabilities such as `cfg`, `call_graph`, `coverage_facts`, and
`test_suite_metrics` must stay unsupported until a rule can consume real public
SDK facts for them. `Cfg<'_>` and `CallGraph<'_>` are not aliases for
`ControlFlow<'_>` or `Calls<'_>`. `DataFlow<'_>` is a policy query view, not a
raw graph view.

Rules that request unsupported or setup-missing hard capabilities produce
`polint/capability` diagnostics during `polint check` and are not executed with
placeholder facts.

## Completeness at Rule Execution

Capability support answers whether polint can provide a fact family. It does
not by itself prove that a particular run finished without truncation or
unknown regions. Rules can inspect `ctx.completeness()` for that distinction:

```rust
let status = ctx.completeness().status_for("calls");
if status != CapabilityCompletenessStatus::Complete {
    let reason = ctx
        .completeness()
        .reason_for("calls")
        .unwrap_or("completeness information is unavailable");
    // Report or annotate the policy result with `status.as_str()` and `reason`.
}
```

Each directly requested capability is reported as `complete`,
`budget_exceeded`, `provider_failed`, `degraded`, or `unknown`. Budget status
includes relevant provider stops and recorded solver/data-flow budget rows.
`degraded` always carries a reason from the unknown taxonomy. `unknown` means
the host could not prove completeness; a rule must not treat it as a clean
repository. `CompletenessView::is_complete()` checks every capability requested
by the current rule, and `budget_exceeded()` reports whether any of them hit a
budget.
