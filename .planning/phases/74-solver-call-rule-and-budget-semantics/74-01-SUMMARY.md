---
phase: 74-solver-call-rule-and-budget-semantics
plan: 01
subsystem: points-to
tags: [solver, call-rule, budget, points-to, ts-frontend, js-bench]

requires: [73-01]
provides:
  - A budget event keeps every set and derived edge computed before it; only sets still pending, and the sets they feed, are marked incomplete
  - The call rule inside the solver binds arguments to parameters, the callee's return to the call result and the receiver to `this`
  - Every function has stable parameter, rest, `this` and return places
  - Field slots keyed by (object, field) with stable ids; the solve is independent of constraint order
  - A set fed by an input the solve cannot see is raised to `Heuristic`, and a heuristic edge never resolves its site
  - Default budget ceilings a real package solves within (axios, async and zod report `WithinBudget`)
affects: [phase-75, phase-76]

key-files:
  created:
    - crates/polint/src/ts/points_to/solver_composition.rs
  modified:
    - crates/polint/src/analysis_neutral/points_to/solver.rs
    - crates/polint/src/analysis_neutral/points_to/vars.rs
    - crates/polint/src/analysis_neutral/semantic_graph/constraints.rs
    - crates/polint/src/analysis_neutral/semantic_graph/validate.rs
    - crates/polint/src/analysis_neutral/semantic_graph/build.rs
    - crates/polint/src/analysis_neutral/semantic_graph/cache_key.rs
    - crates/polint/src/analysis_neutral/solver/budget.rs
    - crates/polint/src/analysis_neutral/refined_calls/ts_js.rs
    - crates/polint/src/analysis/unknown_taxonomy/collect.rs
    - crates/polint/src/ts/points_to.rs
    - crates/polint/src/ts/semantic_graph_build.rs
    - crates/polint/src/ts/binding/direct.rs
    - crates/polint/src/ts/object_model/extract.rs
    - crates/polint/src/ts/scope/extract.rs
    - crates/polint/src/ts/inventory/
    - tests/js-analysis-bench/baseline/

key-decisions:
  - "The composition tests live in the crate (`ts/points_to/solver_composition.rs`): `analysis_neutral` and `ts` are crate-private, and an integration test under `crates/polint/tests/` cannot reach them without widening the public surface."
  - "The engine's own step budget and the points-to worklist budget are separate ceilings; the points-to defaults are ceilings against runaway, not precision knobs."
  - "A set fed by an input the solve cannot see (a field slot of an object written outside the analysed code, a parameter of a function called from outside) is `Heuristic`, so its edges stay out of the exact tier."
  - "Only a call-rule edge below `Heuristic` resolves a site. A heuristic edge keeps its target and the site stays open."
  - "A body-less declaration (an overload signature or an ambient `declare`) is not a binding target; the implementation is."
  - "An alias row is recorded only when neither the declared name nor the name it reads is ever written or redeclared."

requirements-completed: [PG-05]
---

# Plan 74-01 summary: solver call rule and budget semantics

Pull request: #147, stacked on #146.

## What changed

- **Budget.** The global latch is gone. When the worklist budget runs out,
  the solve stops and keeps every set and edge it has derived. Only the sets
  still pending, and the sets they feed, become `BudgetExceeded`. The run says
  which ceiling it hit. The `unknowns` row for a budget event names the
  package and the number of sites it left incomplete. The defaults were raised
  to ceilings a real package solves within: `max_steps` 50,000,000,
  `max_objects_per_var` 4,096 and `max_dynamic_vars` 4,000,000.
- **Call rule.** `CallConstraint` names the callee, the arguments in order,
  the result and the receiver. When a callee set gains a function, the solver
  binds `param_i <- arg_i`, `result <- return` and `this <- receiver`. Spread
  arguments and rest parameters bind heuristically.
- **Places.** Every function has one place per parameter position, plus a
  rest, a `this` and a return place. Their identities come from the
  function's identity.
- **Slots and order.** Field slots are keyed by (object identity, field), with
  a collision probe. The solver sorts its constraints, so any permutation of
  the input gives the same result.
- **Exact-claim fixes found once sets survived the solve.** Each has a
  fixture and is described in the plan's `<plan_corrections>`:
  - open inputs are raised to `Heuristic`;
  - overload and ambient signatures are not binding targets;
  - a heuristic edge leaves its site open;
  - a written name is no alias of its initializer.

## Numbers (untyped; base / refined)

| measure | before | after |
|---|---|---|
| tune recall, every tier, base | 44.44% | 44.44% |
| tune recall, every tier, refined | 44.44% | 44.56% |
| held-out recall, every tier, base | 27.11% | 27.11% |
| held-out recall, every tier, refined | 27.51% | 27.51% |
| wrong exact targets, tune and held-out, every plan | 0 | 0 |
| false-complete, every plan | 0 | 0 |
| budget events, tune (refined) | axios, zod `BudgetExceeded` | none; every package `WithinBudget` |

Refined recall per tune package:

| package | before | after |
|---|---|---|
| async | 51.49% | 51.49% |
| axios | 40.44% | 45.33% |
| chalk | 59.72% | 59.72% |
| commander | 37.42% | 37.42% |
| dayjs | 31.62% | 31.62% |
| underscore | 60.31% | 60.31% |
| zod | 55.12% | 55.12% |

Only axios moves. Recall here is single-target recall, and the call rule's
edges reach it only where a site's whole callee set is closed. Most new
edges sit on sites that stay open, because arguments are not lowered yet:
`arguments` stays empty in real builds. Return places are also fed only by
returns the frontend projects, and those are heuristic. Value-flow lowering
is the next plan.

## Typed plan (TypeScript 5.9.3)

| measure | before | after |
|---|---|---|
| tune recall: exact-complete / exact / exact + conservative | 47.51% / 56.86% / 62.73% | 44.88% / 56.98% / 62.85% |
| held-out recall: exact-complete / exact / exact + conservative | 27.80% / 33.17% / 34.38% | 27.62% / 33.17% / 34.38% |
| typed axios recall | 76.59% | 77.19% |
| wrong exact targets, every set | 0 | 0 |
| false-complete, every set | 0 | 0 |

The exact-complete drop comes from the fix that keeps a heuristic edge from
resolving its site. Before the fix, three typed axios sites were
false-complete. Fixing that moves 119 tune edges from exact-complete to
exact on an open site:

- commander 110 → 21;
- axios 302 → 276;
- async 328 → 324.

In commander, 126 of the 134 sites that lose completeness are `this.m()`
inside a `Command` method. `this` there is fed by receivers the solve cannot
see, and a subclass may override `m`, so those sites were never proven
complete. The exact targets stay. The `<plan_corrections>` entry "Heuristic
edges and the unresolved marker" records the measurement.

## Cost (zod, `bench.py report` with cost)

| measure | before | after |
|---|---|---|
| cold check | 4291 ms / 524 MB | 3985 ms / 525 MB |
| warm check | 2734 ms | 2513 ms |
| fact dump, base | 3762 ms / 534 MB | 4838 ms / 534 MB |
| fact dump, refined | 11383 ms / 1023 MB | 15458 ms / 1056 MB |
| solver stage (refined dump) | 176 ms | 411 ms |

The 1-, 5- and 15-minute load averages were 7.54 / 50.99 / 53.35 at the start
of the cost run and 8.85 / 27.04 / 42.82 at its end. Before, the solve on zod
stopped at the budget and blanked its sets. Now it runs to its fixpoint, so
part of the solver-stage rise is work the solve now does. Every other stage
rose too, which matches the load. On this shared server, the plan's
refined-time acceptance cannot be checked against a quiet baseline. Take the
times as upper bounds.

## Reproducibility

Two `bench.py report --no-cost` runs give equal `report.json` once wall time,
peak memory and dump cost are removed (`EQUAL`). Solve-order independence is
tested by `order_independence_with_fields_calls_and_budget` (permutations of
the constraint list) and `a_slot_keeps_its_variable_when_other_slots_are_added`.

## Stitched equals whole

There is no composition test yet. It comes with the package graph export and
stitch. This plan changes the solver, not what a package graph exposes at
its boundary.

## Mutation checks

| mutation | result |
|---|---|
| derived sets blanked in `to_result` again | caught: `budget_event_keeps_sets_and_edges_derived_before_it`, `call_through_an_incomplete_set_keeps_its_edges_and_is_counted` |
| call rule skips `result <- return` | caught: `call_result_holds_what_the_callee_returns` |
| arguments bound to parameters shifted by one | caught: `call_binds_each_argument_to_its_parameter`, `spread_argument_makes_the_parameters_it_fills_heuristic`, `heuristic_callee_binds_heuristically` |
| slot ids allocated in encounter order | caught: `a_slot_keeps_its_variable_when_other_slots_are_added` |
| `max_dynamic_vars` back to 512 | caught: `package_sized_set_solves_within_the_default_budget`, `superset_of_constraints_solves_to_a_superset`, both budget-default tests |
| open sets not raised in `into_result` | caught: `input_the_solve_cannot_see_keeps_edges_out_of_the_exact_tier`, `a_heuristic_callee_set_leaves_the_site_open` |
| binding resolver returns a body-less declaration | caught: `refined_edges_reach_the_implementation_of_an_overload` |
| heuristic solver edge resolves its site | caught: `a_heuristic_solver_edge_leaves_its_site_unresolved` |
| alias row recorded for a written name | caught: `a_written_alias_is_no_alias_of_its_initializer` |

Two expectations in the plan were wrong on the first run, and the encounter
and heuristic-resolve mutations survived it. The permutation test cannot see
encounter-order ids, because permuting a fixture does not change which slot
comes first in sorted order. The end-to-end heuristic fixture cannot see the
heuristic-resolve mutation untyped, because the refined provider re-adds the
unresolved marker when no exact or conservative tier is claimed. Each now
has a test that catches it. The plan's mutation expectations were corrected
in the same pull request.

## Gate

`GOTOOLCHAIN=go1.25.12` is set for every cargo test and gate step. The local
default toolchain, go1.27.4, writes export data v5, which the Go tests in
this tree do not read.

The full local gate ran with that toolchain. Results:

| step | result |
|---|---|
| `cargo fmt --all -- --check` | ok |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | ok |
| `cargo doc --workspace --no-deps --all-features --locked` | ok |
| `cargo test --workspace --all-features --locked --no-fail-fast` | 3273 passed, 1 failed (`dense_id_sweep_matches_baseline`), 22 ignored |
| TypeScript sidecar, `cargo test -p polint --lib --all-features` with `POLINT_TS_TYPESCRIPT` | 2961 passed, 21 ignored |
| TypeScript sidecar, `cargo test -p polint --test '*' --all-features` with `POLINT_TS_TYPESCRIPT` | 265 passed, 1 ignored |
| public-surface leak gate | 8 passed |
| `eval::determinism_gate` | 12 passed |
| `construct_function_keys` (release) | passed |
| `dense_id_sweep`, after the re-baseline | 4 passed |
| held-out guard against the base branch | ok (42 changed paths) |
| `bench.py selftest` | ok |

The one workspace failure was the dense-id sweep. All of its 13 new hits and
3 removed hits are `Debug` renderings of ids inside test assertion messages,
in the new and changed tests. The baseline was regenerated with
`POLINT_UPDATE_DENSE_ID_BASELINE=1` and committed with each signature listed
in the commit message. After that the sweep passes. The baseline file was the
only change.

CI runs on this pull request against its stacked base.

## Deviations from the plan

All are recorded in the plan's `<plan_corrections>`, with the reason for
each:

- test location;
- the budget split;
- call-rule inputs;
- the callee variable;
- heuristic taint;
- parameter places;
- the `CallConstraint` projection;
- return places;
- the mechanical `go/` change;
- spread and rest;
- binding provenance;
- partial status;
- order independence;
- inputs the solve cannot see;
- overload signatures;
- heuristic edges and the unresolved marker;
- written aliases;
- the mutation expectations.

## Still open

- `this` and class-method completeness: the commander `this.m()` sites above.
  Class lowering is the next plan's work.
- Argument lowering. `arguments` stays empty in real builds, so the call rule
  binds nothing there yet.
- Local `let` writes inside a function body are not lowered. A call through a
  written `let` has no target instead of a partial one.
- Return places are fed only through projected returns, which are heuristic.
- An ambient-only `declare` function has no binding target.
- Held-out refined recall is 17.0 points below tune, outside the 5-point
  margin.
- The refined-time acceptance needs a quiet-machine cost baseline. The one
  above was taken under load.
