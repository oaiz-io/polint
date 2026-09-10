# Guard outcomes

A synthetic Go corpus for `ControlFlow::guard_outcomes`, the query that answers
*covered / violation / unknown* per protected operation instead of returning a
violation list.

`CheckAccess` is the guard; `SaveRecord` is the protected operation. Each
function in [`cases.go`](cases.go) is one control-flow shape. The rule sets
`require_checked_error = true`, so a covered operation needs a guard whose block
dominates it, a nil-comparison branch on the guard's returned error, and an error
edge that cannot reach the operation. It also sets `argument_binding`, so a
covered operation must additionally be the one the guard authorized.

Run it:

```bash
polint check --format json --fail-on none
polint test --rule local/guard-outcomes
```

## What the corpus pins

| Function | Outcome | Reason |
|---|---|---|
| `validGuard` | covered | `checked_error_exits` |
| `missingGuard` | violation | `guard_missing` |
| `writeBeforeGuard` | violation | `guard_not_ordered_before` |
| `ignoredGuardError` | violation | `result_never_tested` |
| `conditionalGuard` | violation | `guard_does_not_dominate` |
| `logsErrorWithoutExit` | violation | `error_path_reaches_operation` |
| `checksDifferentError` | unknown | `ambiguous_error_definition` |
| `authorizesDifferentActor` | unknown | `alias_indeterminate` |
| `replacesActor` | violation | `identity_reassigned` |
| `guardInUnusedClosure` | violation | `guard_does_not_dominate` |
| `validTransactionCallback` | covered | `checked_error_exits` |
| `aliasTenantFlow` | violation | `guard_missing` |
| `unrelatedNearbyTenant` | violation | `guard_missing` |

`authorizesDifferentActor` and `replacesActor` are where identity binding earns
its place. On control flow alone both are *covered*: the guard runs and its error
is checked. The rule sets `argument_binding = (1, 1)`, so the query also asks
whether the authorized actor is the consumed actor.

- `replacesActor` is refuted: `actor = other` between the guard and the
  operation rebinds the authorized place from a provably different root.
- `authorizesDifferentActor` is undecided: the two places are distinct and no
  alias answer relates them, so the query reports `alias_indeterminate` rather
  than guessing. That is the common real-world answer, and it is still a strict
  improvement over claiming coverage.

Every covered result carries `identity_binding` evidence saying how identity was
established — `unchecked` when the query sets no `argument_binding`. See
[docs/facts/control-flow.md](../../docs/facts/control-flow.md).

`guardInUnusedClosure` reports `guard_does_not_dominate` rather than
`guard_missing`: the Go frontend keeps a short function literal inside its
enclosing function, so the guard is visible but does not dominate the operation.
Both readings refute the contract.

The `.polint/tests` fixture keeps its own copy of `cases.go`, because
`polint test` copies a case directory into a temporary repo. `cargo test -p
polint --test golden` fails if the two copies drift.
