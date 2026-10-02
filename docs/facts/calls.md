# Calls Facts

`Calls<'_>` is a preview SDK view for policy-level reachable-call checks.
Requesting it derives the supported `calls` capability.

See [policy-queries.md](policy-queries.md) for the shared query-object style,
evidence header, precision/status vocabulary, unknown semantics, and template
starter workflow.

Phase 56 implements `Calls::forbidden_reachable(ReachQuery)` over private
refined-call and reachability facts. The public API returns policy violations;
it does not expose raw call-graph nodes, dense IDs, solver internals, or
provider data structures.

```rust
#[polint::rule(id = "local/no-dangerous-reachable", description = "Reachability policy", severity = "error")]
pub(crate) fn no_dangerous_reachable(ctx: &mut RuleCtx<'_>, calls: Calls<'_>) -> RuleResult {
    let mut query = ReachQuery::new(EventPattern::call("dangerous_exec"));
    query.roots = vec![EventPattern::call("http_handler")];
    query.include_tests = false;
    query.max_depth = 20;
    query.max_paths = 20;
    query.minimum_precision = PolicyPrecision::Conservative;
    query.minimum_confidence = PolicyConfidence::Low;

    for violation in calls.forbidden_reachable(query) {
        ctx.report(violation.diagnostic(ctx.rule_id(), "dangerous call is reachable"));
    }

    Ok(())
}
```

## Query Vocabulary

- `ReachQuery::new(target)` requires one target `EventPattern`.
- `query.roots` constrains reachability roots when non-empty. Phase 56 matches
  roots by root kind labels, function names, and symbol names that already exist
  in stored facts.
- `query.include_tests` defaults to `false`.
- `query.max_depth` and `query.max_paths` bound search and returned results.
- `query.minimum_precision` and `query.minimum_confidence` filter the private
  refined-call edges used for traversal.

Returned diagnostics include the common policy evidence header documented in
[evidence.md](evidence.md), plus target, root, path, depth, call status, call
precision, and confidence evidence where available. Budget truncation is
surfaced as budget evidence instead of being treated as a complete absence
proof.

`Calls<'_>` is not the `CallGraph<'_>` view. `CallGraph<'_>`
([call-graph.md](call-graph.md)) exposes the call edges themselves, keyed by
the `FunctionId`s of `Functions<'_>`, for rules that walk the graph; `Calls<'_>`
answers reachability policies written as event patterns.

## Template Starter

`polint new-rule go no-raw-admin-reachable --template raw-reachable-api`
scaffolds a reachable-call policy using `Calls<'_>` and `ReachQuery`. The
generated target and root call names are placeholders for repo-local APIs; the
template is a starting point, not a bundled rule.
