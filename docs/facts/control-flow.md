# Control-Flow Facts

`ControlFlow<'_>` is a preview SDK view for guard and lifecycle policies.
Requesting it derives the supported `control_flow` capability.

See [policy-queries.md](policy-queries.md) for the shared query-object style,
evidence header, precision/status vocabulary, unknown semantics, and template
starter workflow.

Same-function call-event guard and lifecycle queries use refined call facts and
CFG dominance or post-dominance when those relations are available. MIR
operation order and source spans remain fallback ordering sources when CFG rows
are absent. The API remains preview because interprocedural proof, write-field
events, resource identity pairing, and per-exit cleanup proof are deferred.

`missing_guard` and `missing_cleanup` return violations. `guard_outcomes`
returns one [`PolicyResult`] per protected operation, so *proved*, *refuted*,
and *could not decide* are separate answers rather than presence or absence in
a list. See [Guard outcomes](#guard-outcomes).

```rust
#[polint::rule(id = "local/require-auth-before-dangerous-call", description = "Auth guard", severity = "error")]
pub(crate) fn require_auth_before_dangerous_call(
    ctx: &mut RuleCtx<'_>,
    control: ControlFlow<'_>,
) -> RuleResult {
    let query = GuardQuery::new(
        EventPattern::call("dangerous_exec"),
        GuardPattern::call_any(["authorize", "require_admin"]),
    );

    for violation in control.missing_guard(query) {
        ctx.report(violation.diagnostic(ctx.rule_id(), "dangerous calls require authorization"));
    }

    Ok(())
}
```

```rust
#[polint::rule(id = "local/transaction-cleanup", description = "Transaction lifecycle", severity = "error")]
pub(crate) fn transaction_cleanup(ctx: &mut RuleCtx<'_>, control: ControlFlow<'_>) -> RuleResult {
    let mut query = LifecycleQuery::new(
        EventPattern::call("Begin"),
        EventPattern::call("Rollback"),
    );
    query.require_error_cleanup = true;

    for violation in control.missing_cleanup(query) {
        ctx.report(violation.diagnostic(ctx.rule_id(), "transaction begin requires cleanup"));
    }

    Ok(())
}
```

```rust
#[polint::rule(id = "local/guard-outcomes", description = "Guard outcomes", severity = "error")]
pub(crate) fn guard_outcomes(ctx: &mut RuleCtx<'_>, control: ControlFlow<'_>) -> RuleResult {
    let mut query = GuardQuery::new(
        EventPattern::call("SaveRecord"),
        GuardPattern::call_any(["CheckAccess"]),
    );
    query.require_checked_error = true;

    for result in control.guard_outcomes(query) {
        // `diagnostic` is `None` for a covered operation: there is nothing to
        // report as a finding. Report coverage from the outcome itself.
        if let Some(diagnostic) = result.diagnostic(ctx.rule_id(), "guard contract not satisfied") {
            ctx.report(diagnostic);
        }
    }

    Ok(())
}
```

## Guard Outcomes

`ControlFlow::guard_outcomes` answers one question per protected operation:
does the required guard cover it?

| `PolicyOutcome` | Meaning | `diagnostic()` |
|---|---|---|
| `Covered` | The contract was proved within the documented scope. | `None` |
| `Violation` | The contract was refuted. | `Some(_)` |
| `Unknown` | The operation was examined and the engine could not decide. | `Some(_)` |
| `NotAnalyzed` | The operation was never examined. Queries never return it. | `Some(_)` |

Every result carries `reason` evidence naming what decided it:

| `reason` | Outcome |
|---|---|
| `guard_dominates_operation` | Covered without `require_checked_error`. |
| `checked_error_exits` | Covered: the error was tested and its path cannot reach the operation. |
| `guard_missing` | Violation: no matching guard call in this function. |
| `guard_not_ordered_before` | Violation: the only matching guard runs after the operation. |
| `guard_does_not_dominate` | Violation: the guard runs on only some paths to the operation. |
| `result_never_tested` | Violation: the guard ran but nothing tested its returned error. |
| `error_path_reaches_operation` | Violation: the error was tested but the error path falls through to the operation. |
| `identity_mismatch` | Violation: the guard's argument and the operation's argument provably do not alias. |
| `identity_reassigned` | Violation when the rebinding source is a provably different root, Unknown otherwise. |
| `ambiguous_error_definition` | Unknown: the tested place carried the guard's error and was then overwritten. |
| `alias_indeterminate` | Unknown: no decisive alias answer relates the two arguments. |
| `argument_position_out_of_range` | Unknown: a bound position is past the call's argument list. |
| `cross_body_identity` | Unknown: the guard and the operation are not in one MIR body. |
| `partial_nil_test` | Unknown: the comparison does not prove non-nil on its other edge. |
| `unrecognized_error_test` | Unknown: the branch on the guard's error is not a nil comparison. |
| `missing_branch_block` | Unknown: the error test has no CFG node. |
| `missing_error_edge` | Unknown: the branch block has no `True`/`False` successor. |
| `missing_guard_operation` | Unknown: the guard call has no MIR operation. |
| `missing_block_ids`, `empty_relation` | Unknown: see [Unestablished dominance](#unestablished-dominance). |
| `guard_outcome_error_path_budget` | Unknown, `policy_status = budget_exceeded`: the error-path search hit its block cap. |

### What Covered Means

`Covered` is scoped to the **control dimension**. With
`require_checked_error = true` it means all four of:

1. the guard call's basic block dominates the operation's block;
2. a branch tests the value the guard's result reached, and that branch's block
   also dominates the operation;
3. the branch's nil test proves non-nil on its other edge; and
4. the block the error edge enters cannot reach the operation's block in the
   normal-control view.

It does **not**, on its own, mean the guard authorized the value the operation
consumed. Every covered result carries `identity_binding` evidence saying which
identity claim it makes; without `argument_binding` that value is `unchecked`,
and a function that checks one actor and then acts on a different one is
`Covered` with `identity_binding = unchecked`.

### Argument Identity

`GuardQuery::argument_binding` relates one guard argument to one protected-call
argument by zero-based source position:

```rust
query.argument_binding = Some(ArgumentBinding::new(1, 1));
```

The query then answers identity with the cheapest sufficient evidence:

| `identity_binding` | Established by |
|---|---|
| `same_place` | The two arguments are the same place. |
| `projection_extension` | The consumed place is a projection of the authorized one, such as `actor.TenantID` after `actor`. |
| `must_alias` | An alias answer says the two places must alias. |

A `NoAlias` answer refutes the binding (`identity_mismatch`). Every other alias
answer — may-alias, partial-alias, unknown, or **no row at all** — is
`alias_indeterminate`, not coverage. In real Go that is a common answer:
interface receivers, method values, and struct-embedded actors all depend on
pointer-target precision the engine does not have.

Alias answers are flow-insensitive, so a bound identity is checked again against
the operations between the guard and the protected call. A write to either
bound place's root there answers `identity_reassigned`: a violation when the
write's source is a place with a provably different root, unknown otherwise.

Positions index a call's source-order arguments and never its receiver. Variadic
packing is not modelled, so a position past a variadic callee's fixed parameters
names whichever source argument sits there. Cross-package identity is not
attempted.

### Documented Limits

These are real limits of the model, not gaps in a particular run:

- **Same function only.** `max_depth` above `1` does not enable interprocedural
  search. A guard that lives in a helper is not seen, so the operation reads as
  a violation.
- **`panic`, `log.Fatal`, and `os.Exit` are not exits.** Treating a call as
  terminating needs a callee-effect model that does not exist. An error path
  that ends in one of them still "reaches" the operation and is reported as
  `error_path_reaches_operation`. This is a known false positive and the reason
  `require_checked_error` is opt-in.
- **The nil test is syntactic.** Only a comparison against a literal nil is
  recognised. `errors.Is(err, target)`, `err != nil || other`, and
  `switch { case err != nil: }` are not, and answer
  `unrecognized_error_test`.
- **Go block scope is not modelled.** Two `err` variables in nested scopes are
  the same place, so a shadowed rebinding answers `ambiguous_error_definition`
  rather than being called a different value.
- **`defer`-based recovery, `errgroup`, and channel-carried errors are out of
  scope** and answer `Unknown`.
- **One guard, one operation.** When several guards match, the engine reports
  the most favourable answer, preferring proved over undecided over refuted.

## Query Vocabulary

- `GuardQuery::new(event, guard)` requires an event and one guard pattern.
- `LifecycleQuery::new(start, cleanup)` requires a start event and cleanup
  event.
- `GuardPattern::call_any([...])` is an explicit list of canonical call names.
- `EventPattern::call(...)` is supported for guard and lifecycle queries.
  `EventPattern::write_field(...)` is still preview vocabulary and returns no
  control-flow results.
- `max_paths` caps returned violations and reports budget evidence when
  truncated.
- `minimum_precision` filters the private call facts considered by the query.
- `max_depth` is present for a stable query shape, but the current engine only evaluates
  same-function depth. Values above `1` do not enable interprocedural search yet.
- `require_error_cleanup` is surfaced as evidence. A cleanup call clears a start
  when it post-dominates that start, which one call reaching every exit does.
  Cleanup split across exits is not yet proved: a function that calls cleanup
  separately on each path, or from a `finally` block, is still reported.
- `report_unknown_coverage` (default `false`) decides what happens when the
  dominance or post-dominance relation cannot answer. See
  [Unestablished dominance](#unestablished-dominance). `guard_outcomes` always
  reports unestablished dominance and ignores this field.
- `require_checked_error` (default `false`) is read only by `guard_outcomes`.
  See [What covered means](#what-covered-means).

## Unestablished Dominance

A guard clears an event only when the guard's basic block dominates the event's
block; a cleanup clears a start only when it post-dominates the start. Two
inputs can be missing:

- `missing_block_ids` — one of the two calls has no CFG node, so it maps to no
  basic block.
- `empty_relation` — the run produced no rows for the relation at all, so
  dominance was never computed.

With `report_unknown_coverage = false` (the default) both cases suppress the
result, which keeps the ordering-only behavior a query has without CFG relation
facts. Absence of a diagnostic under that default therefore means "no result",
not "proved covered".

With `report_unknown_coverage = true` those cases become an explicit result with
`policy_status = unknown`, `policy_precision = unknown`, and
`dominance_evidence` naming the missing input. Such a result carries no
`uncovered_path` and no `evidence_v1`: there is no path to claim when the
relation that would have proved coverage was unavailable.

Emitted results always carry `dominance_evidence`:

| Value | Meaning |
|---|---|
| `dominator_relation` | Candidates existed and the relation refuted every one. |
| `no_guard_candidate` | No matching guard call was ordered before the event. |
| `no_cleanup_candidate` | No matching cleanup call was ordered after the start. |
| `missing_block_ids` | A call had no CFG node (unknown result only). |
| `empty_relation` | The relation had no rows (unknown result only). |

Returned diagnostics include the common policy evidence header documented in
[evidence.md](evidence.md), plus target, function, control scope, required
guard or cleanup, uncovered path, order-source, call status, call precision,
confidence when available, and budget evidence when truncation occurs. Results
remain conservative and heuristic because matching calls and lifecycle identity
are not semantic resource proofs. The `order_source` evidence is
`cfg_operation_order` when CFG rows are available, otherwise
`mir_operation_order` or `source_span`.

`ControlFlow<'_>` is not the raw `Cfg<'_>` view. `Cfg<'_>` remains a reserved
raw capability and is not the supported rule-authoring path for guard or
lifecycle policies.

`examples/go-guard-outcomes/` is a synthetic Go corpus that pins one outcome per
shape, including the two that are honestly undecidable.

## Scope And Budget

Requesting `control_flow` requests cross-file analysis, so the run loads every
discovered file no matter how narrow a rule's `files` list is; `files` narrows
reporting only. A `polint/scope` note reports the difference. For Go, the
analysis is bounded by `[languages.go] semantic_timeout_ms` (and the
`POLINT_GO_SEMANTIC_TIMEOUT_MS` override), and exhausting it blocks the rule
with `polint/capability` diagnostics rather than answering from partial facts.
See [Bounding a Go semantic scan](../CONSUMER-SETUP.md#bounding-a-go-semantic-scan).

## Template Starters

`polint new-rule go require-sensitive-write-guard --template
sensitive-write-guard` scaffolds a guard-before-sensitive-call policy.
`polint new-rule go require-transaction-cleanup --template transaction-cleanup`
scaffolds a same-function cleanup policy. Generated templates use
`ControlFlow<'_>` and the query-object style shown above, with fixtures that
users can edit to their local guard, write, begin, and cleanup APIs.
