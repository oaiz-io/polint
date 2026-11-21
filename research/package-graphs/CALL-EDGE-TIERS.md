# Call Edge Tiers and Assumptions (internal)

These are the semantics of refined call edges derived by the TypeScript /
JavaScript points-to solve. They are internal fact semantics: no SDK view
exposes them, and their shape may change. The closed lists they implement are
in [TIERS.md](TIERS.md).

## Tiers

Every call edge claims one tier, read from its `CallPrecision`:

| Precision | Tier | Claim |
|-----------|------|-------|
| `Exact`, `SetupAware` | exact | The call reaches this function, and no other where the site is complete. |
| `Conservative`, `Ambiguous` | conservative | The call may reach this function. |
| `Heuristic` | apart | Resolved from names; counted on its own. |
| `Unknown`, `Unsupported` | none | No claim. |

A site is **complete** when it has a resolved target and no unresolved row,
and **open** otherwise.

## What the solve claims per site

After the solve, each site whose callee set it derived gets one claim:

- **Exact**: the set is closed to values from outside the package, holds one
  function, and the solve had no budget event. Its edge has precision `Exact`
  and lists the assumptions below.
- **Complete**: the set is closed and holds several functions. The site
  reaches some of them and nothing else; each edge is `Conservative`.
- **Open**: a value the constraints do not describe reaches the set. Each
  edge is `Conservative`, and the site gets one unresolved row with reason
  `Inflow(kind)` and evidence `solver_open`, `inflow:<kind>`.
- **Unassigned**: any edge after a budget event, and edges of other solves
  (Go). The set's precision decides: flow-insensitive and locally
  flow-sensitive sets give `Conservative`, summary-projected and heuristic
  sets `Heuristic`. Edges from repo or framework models are `Conservative`.

An exact points-to edge must list at least one assumption, and only such an
edge lists any; validation rejects other edges, and the benchmark report stops
on them.

### Inflow kinds

How an outside value reaches a set, most specific first. A set reached in
several ways reports the first.

| Kind | Source |
|------|--------|
| `inward_parameter` | A parameter or rest parameter of a function code outside can call. |
| `inward_this` | `this` of a function code outside can call. |
| `escaped_object` | A field of an object code outside can write. |
| `open_result` | The result of a call whose callees the solve could not resolve. |
| `outside_value` | A place nothing in the package defines, such as an import. |

A set whose precision is already above locally flow-sensitive when it opens is
raised to `Heuristic` and records no inflow.

## Assumptions

| Id | Scope | Meaning |
|----|-------|---------|
| `sealed_bindings` | package-internal | No `eval` reference and no `with` statement, so every write of a binding is seen. |
| `within_budget` | package-internal | The solve reached a fixed point with no budget event. |
| `no_inward_flow` | boundary | No value from outside the package reaches the callee expression. |
| `prototype_closed` | boundary | No code outside writes a prototype the receiver inherits through. Recorded when the callee came through a field. |
| `no_outside_field_write` | boundary | No other package stores into a field the callee was read through. Recorded when the callee came through a field. |

A boundary assumption can be broken by a package the graph is stitched with,
so stitching re-checks it; a package-internal one holds whatever the package
is stitched with.

The list also names `single_literal`, `declaration_only`, `unique_reference`,
`declared_target`, `export_link` and `export_not_rewritten`. The solve does not
record them on its edges: its exact claim rests on closed sets rather than on
how a binding was declared or imported, and the direct tier that does rely on
them does not record assumptions yet. The type step claims nothing exact, so
no edge carries `declared_target`.

## Limits

- The claim is per site, not per calling context.
- Go solver edges are never exact; they keep the set's precision.
