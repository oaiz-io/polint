---
phase: 72-package-graph-measurement-baseline
plan: 02
subsystem: call-graph
tags: [exact-claims, tiers, ts-types, held-out, refined-calls]

requires: [72-01]
provides:
  - A call answered only by a name match is open, with an unresolved row, on the base and the refined plan
  - A summary-assisted edge claims no more than its base target
  - A refined answer whose edges claim nothing keeps the base unresolved row
  - A typed target is exact only where the callee binding holds it alone; every other typed target is a conservative candidate
  - Wrong exact targets 0 on every set and plan, the typed plan included
affects: [phase-73, phase-75, phase-76]

key-files:
  created:
    - crates/polint/src/analysis_kernel/name_match_tests.rs
    - crates/polint/src/analysis_kernel/ts_types_tests.rs
  modified:
    - crates/polint/src/analysis_neutral/calls/facts.rs
    - crates/polint/src/analysis_neutral/calls/provider.rs
    - crates/polint/src/analysis_neutral/calls/unresolved.rs
    - crates/polint/src/analysis_neutral/refined_calls/summaries.rs
    - crates/polint/src/analysis_neutral/refined_calls/provider.rs
    - crates/polint/src/analysis_neutral/refined_calls/ts_types.rs
    - crates/polint/src/ts/types/facts.rs
    - crates/polint/src/ts/types/lower.rs
    - crates/polint/src/ts-sidecar/polint-ts-types/index.js
    - research/ts-type-sidecar/wire-protocol.md
    - tests/js-analysis-bench/report.py
    - tests/js-analysis-bench/selftest.py
    - research/package-graphs/DIAGNOSIS.md

key-decisions:
  - "A site is resolved only when a target claims a tier; a heuristic name match alone leaves the site open with a `MissingSemanticReference` row of shape `unclaimed-targets`."
  - "`ClaimTier` is ordered so that a greater tier claims less; a derived edge takes the greater of its own tier and its base target's."
  - "The sidecar writes `declared` only when the callee is the function itself or names a binding with one value declaration that holds it and that the scan never sees written or overridden. Everything else it resolves is `inferred`, a conservative candidate."
  - "A member name written through a receiver the checker cannot resolve to a value taints every member binding of that name."
  - "The bench stops on a typed candidate that claims exact rather than rereading its tier."
  - "The axios cost of DIAGNOSIS section 5 does not reproduce, so there is no commit to bisect."

requirements-completed: [PG-02]
---

# Plan 72-02 summary: no wrong exact claims

Pull requests: #144 (the `same-name-member` construct and its recording) and
the exact-claims pull request on top of it (Tasks 2 to 5).

## Untyped plans

The measurement baseline found the same defect in held-out acorn
(`Parser.parse`, `Parser.parseExpressionAt`, `Parser.tokenizer`). It caused 3
wrong exact targets on the refined plan and 3 answers with other targets and
no unresolved marker on each plan. `Ctor.m(...)` inside a function named `m`
was resolved by name to the caller, with `Heuristic` precision. The call layer
counted that answer as complete, and the summary re-stated it as `SetupAware`.

| measure | before base / refined | after base / refined |
|---|---|---|
| wrong exact targets, tune | 0 / 0 | 0 / 0 |
| wrong exact targets, held-out | 0 / 3 | 0 / 0 |
| other targets and no unresolved marker, held-out | 3 / 3 | 0 / 0 |
| false-complete call lines, constructs (87 lines) | 2 / 2 | 0 / 0 |
| tune recall, every tier | 42.6% / 44.5% | 42.6% / 42.6% |
| held-out recall, every tier | 23.1% / 23.6% | 23.1% / 23.5% |

The 65 refined exact edges lost on the tune set (zod 51, axios 11,
underscore 3) were `Heuristic` base targets the summary had promoted. Each
was right on the recorded run, but no binding backed it. They now count apart,
and recall over all targets is unchanged. On zod, 159 of the 160 `Heuristic`
base edges are static-method calls through a class name. The class static
member is construct 14 of TIERS.md, which `75-01-PLAN.md` lowers. Once it is
lowered, these edges come back exact with evidence.

## Typed plan

Measured with TypeScript 5.9.3, before Task 5 and after it:

| measure | before | after |
|---|---|---|
| wrong exact targets, tune | 16 single, 1 multi | 0 |
| wrong exact targets, held-out | 213 | 0 |
| false-complete, constructs (paths / call lines) | 0 / 0 | 0 / 0 |
| tune recall: exact-complete / exact / exact + conservative | 45.8% / 59.3% / 59.6% | 45.6% / 54.2% / 59.9% |
| held-out recall: exact-complete / exact / exact + conservative | 24.1% / 29.5% / 29.5% | 23.7% / 28.6% / 29.6% |
| zod typed dump: peak memory | 1089 MB | 1090 MB |

Exact recall falls where an answer was never backed by its binding, such as a
reassigned variable, a property assigned twice or a factory's result. Those
targets are now conservative candidates. Recall over exact and conservative
together rises. The held-out cases were read only after the general fix, to
confirm it, and none was tuned on. The last held-out wrong exact target was
nanostores `$atom.off()`, where `store.off = ...` replaces the member through
an untyped parameter. It was fixed by the general write rule, and the fixture
for it is a stand-alone `box.close()` case.

The dump times are not comparable between the runs. The server is shared,
and zod typed took 15.6 s on one run and 24.8 s on a run of the same code at a
higher load.

## Mutation checks

Each fix was reverted alone, and the named tests were run:

| mutation | result |
|---|---|
| summary-assisted precision without the cap | caught by `a_summary_assisted_edge_claims_no_more_than_its_base_target` and `the_refined_answer_for_a_name_matched_call_keeps_its_unresolved_row` |
| `resolved_sites` without the claim-tier condition | caught by `a_member_call_matched_only_by_property_name_stays_unresolved` |
| no `unclaimed_site_row` | caught by `a_member_call_matched_only_by_property_name_stays_unresolved` |
| unclaimed-site edges dropped | caught by `a_site_whose_refined_edges_claim_nothing_keeps_its_unresolved_row` |
| `inferred` claims exact | caught by `an_inferred_target_may_be_reached_but_is_never_claimed_exactly` and `a_binding_that_can_hold_another_value_is_never_an_exact_answer` |
| a declared target at a union site claims exact | caught by `a_declared_target_at_a_union_site_is_one_candidate_not_the_answer` |
| same-start join restored | caught by `a_call_that_only_shares_its_start_with_a_native_call_does_not_join_it` |
| containment join restored | caught by four join tests |
| sidecar always writes `declared` | caught by `a_binding_that_can_hold_another_value_is_never_an_exact_answer` |
| the fallback reads the receiver's symbol | caught after `helper.extra()` was added to the fixture |
| no write record | caught by `a_binding_that_can_hold_another_value_is_never_an_exact_answer` |
| no override record | caught by `a_binding_that_can_hold_another_value_is_never_an_exact_answer` |
| construction named through the signature only | caught by `a_call_the_checker_answers_keeps_its_exact_target` |
| the report accepts an exact typed candidate | caught by `selftest` |

## Deviations from the plan

- **Task 5 was added to this plan** when the typed plan was measured. A
  wrong exact claim is a stop-and-fix inside the phase that measures it.
  Cause (f), the untyped-receiver write, was added after the first rerun
  left one held-out wrong exact target.

## Still open

- The typed rules have known gaps. A write the scan cannot name does not
  taint a binding. This covers computed-key writes, `Object.defineProperty`,
  `Object.assign` and writes outside the scanned files. Each is a source of a
  possible wrong exact target that no measured package shows today.
- The typed rules are also conservative in two places. A destructured
  binding gives `inferred`, and an override anywhere taints the base member.
  Both cost exact recall, not correctness.
- The cost baseline is committed, taken at a 1-minute load average of 6.5 to
  14.7 (5-minute average 12 to 15). Its figures are upper bounds; a
  quiet-machine rerun is open.
