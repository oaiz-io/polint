# Storage option A against this solver

Date: 2026-11-17. Answers PACKAGE-GRAPH.md open question 1 as far as reading
the solver allows; the measurement part stays with WP5.

Research question: does the monotonicity argument in
[PACKAGE-GRAPH.md](PACKAGE-GRAPH.md) ("solving the union starting from the
stored per-package solutions reaches the same least solution as solving from
nothing") hold for the solver polint has, and what must be re-derived at
stitch time?

## Short answer

The argument holds for the constraint rules (`Copy`, `Load`, `Store`,
`FieldLoad`, `FieldStore`, `AddressOf`) and will hold for the call rule WP3
adds, because each rule only adds members to sets. It does not hold for the
engine as a whole today, for five reasons, all of which are fixable and all
of which WP3 fixes before WP5 stores anything:

1. **Budgets are latches, not bounds.** `points_to/solver.rs` ends the solve
   and marks every set `BudgetExceeded` at 10,000 worklist pops, at the 65th
   object in any set, or at the 513th `(object, field)` slot. A package
   solved under a latch is not a fixed point, and a stitch that adds
   constraints can move a package from "within budget" to "nothing" (the
   union has more slots than either part). Stored solutions from such a solve
   are not a lower bound of anything.
2. **Slot identities are allocated in solve order.** `object_slot` numbers a
   field slot `dynamic_var(self.object_slots.len())` when it is first touched
   (`solver.rs:254`). The number depends on worklist order, so two solves of
   the same package, or a package solve and a union solve, give different
   `PtVarId`s to the same `(object, field)`. The output is normalised to
   stable keys (`semantic_variable_identity`, `solver.rs:339-345`), which is
   why results are byte-stable; but a stored *solved state* keyed by
   `PtVarId` cannot be resumed from. The stored form must be keyed by
   `(object identity, field)`.
3. **The walker is not a fixed point.** Its rows depend on four module
   rounds, a return pass that runs with returns disabled, invocation depth 16,
   a 4,096-visit budget per walk and keyed replays (DIAGNOSIS §2). Adding a
   module can change what an earlier round saw. Walker rows stored as
   constraints would make the stored graph depend on the order and size of
   the scan. They must not enter the package graph; either the construct is
   lowered (WP2) or the row is dropped.
4. **`derive_edges` carries a per-source budget and a "weakest path"
   status** (`solver/engine.rs:276-345`). The transitive closure of
   `CopyEdge`s is per source, cut at `max_steps`, and the status of an edge
   is the weakest along the path found first. Both make the derived edge set
   depend on budget and traversal order. Neither feeds the call graph today
   (only `call_constraint` provenance becomes a refined edge,
   `refined_calls/provider.rs:320`), so this closure can be dropped rather
   than stored.
5. **Tier and completeness are not monotone.** "Exact and complete" is a
   claim about absence (no other target, no inflow). The union can only add
   targets, so a site that is complete in a package alone may be open in the
   union. This is the known exception in PACKAGE-GRAPH.md; it is why
   assumptions are stored per claim ([TIERS.md](TIERS.md) (b)) and re-checked.

With 1 to 4 fixed, option A is: store every constraint of the package (after
lowering, with stable node identities) plus the solved points-to set of
every node that a stitch can read: callee expressions, boundary places
(exported values, parameters and `this` of exported functions, imported
places, returned and escaping objects), and the fields of objects reachable
from them. The internal solution of a package is then a valid starting point
for the union solve: every set only grows.

## What the stitch must re-derive

| item | why it cannot be copied |
|---|---|
| the fixed point over the union | link constraints add members; the call rule adds callee bodies' flows across the link |
| every site's completeness and tier | depends on absence; re-evaluated from the union solution against the stored assumptions |
| boundary assumptions `export_link`, `export_not_rewritten`, `prototype_closed`, `no_inward_flow`, `no_outside_field_write` | they quantify over the other packages |
| slot variables for `(object, field)` | identities are per solve; recreated from stable identities |
| refined-edge stable keys and provenance that embed slot identities or solver steps | run-local today (`solver_step`, dynamic vars); the package graph keys edges by (site identity, function identity, stage, algorithm) and nothing run-local |
| budget status | must be "none" for every input graph; a stitch of a budget-exceeded graph is refused, not approximated |

What can be copied: the per-package constraint set, the per-package solution
as a lower bound, package-internal assumptions, the typed tier's edges
(compiled per package against its own `tsconfig`), model edges from the
package's own models, and every extraction fact (functions, call sites,
identities).

## Monotonicity of the planned call rule

The call rule WP3 adds is: when `callee(site) ∋ f`, add `Copy(param_i(f) ←
arg_i(site))`, `Copy(result(site) ← return(f))`, `Copy(this(f) ←
receiver(site))`. Each application adds copy constraints; constraints only
add members; so the solve stays a least fixed point of a monotone system and
the composition argument applies. Two cautions:

- Context insensitivity makes the union's solution coarser than either part
  when a library function is called from two packages with different
  arguments; that is the same coarseness whole-program analysis has, so
  "stitched equals whole" still holds. It is a precision question for the
  selective-precision work (`static-analysis-2.0/06`), not a composition
  question.
- Platform model rows (`ModelEdge`) are constraints too and must be in the
  union with the same digest on both sides of the composition test.

## Size, first estimate

No constraint count is measurable today for a real package because the
lowering is incomplete. Reference points: polint's full fact store is
6.9 MB gzip per MB of source on zod ([ITERATION-1](ITERATION-1-CLASSICAL.md)),
and Jelly's call graph for zod (functions, calls, edges) is 0.18 MB gzip per
MB of source ([BASELINES.md](BASELINES.md)). A constraint set is a few
constraints per expression; at a few tens of bytes each after dictionary
compression, zod's 3.1 MB of source (of which 1.7 MB is test files the
bench analyses but never runs) should give a graph in the low megabytes.
The 2 MB per MB target is kept as a ceiling, not a prediction; WP5 measures
and replaces it.

## Recommendation

Build option A with the four fixes above as preconditions: no latches (a
budget event is a reported failure of the package graph, never a partial
answer); stable slot identities; no walker rows in the graph; no stored
copy-closure. Store the solution only for stitch-readable nodes. Keep the
composition test as the gate: analyse the fixture as one program, analyse
its packages apart and stitch, compare edge sets at both tiers and the open
sites with their reasons. Measure size and stitch time on the three real
trees in WP5 before deciding whether option B is needed.
