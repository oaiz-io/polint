# Iteration 1: the pull-request sequence

Date: 2026-11-17. One pull request per work package, stacked on
`research/package-graph-pipeline` until #141 merges, then rebased on `main`.
Each is a GSD phase under `.planning/phases/`; the phase's `PLAN.md` carries
the files, the order of steps, the fixtures and tests that fail before the
change, the exit criteria as commands with expected results, the mutation
checks, and what the phase must not touch. This file is the map.

| PR | phase | work package | depends on | what it proves |
|---|---|---|---|---|
| 1 | [72](../../../.planning/phases/72-package-graph-measurement-baseline/72-01-PLAN.md) | WP0 measure first | #141 | the bench reports tiers, precision, open sites, budget events, outside-tool columns and the held-out set; the v0.5.3 baseline of this iteration is committed, with the axios cost regression confirmed or refuted |
| 2 | [73](../../../.planning/phases/73-read-all-the-code/73-01-PLAN.md) | WP1 read all the code | 72 | `not_extracted`, `not_analysed`, `sequence_interop` are 0 on the tune set apart from written exclusions |
| 3 | [74](../../../.planning/phases/74-solver-call-rule-and-budget-semantics/74-01-PLAN.md) | WP2a solver semantics | 72 (parallel with 73) | no global latch; a budget event keeps derived edges; the call rule; per-function places; stable slot identities; composition fixtures solve order-independently |
| 4 | [75](../../../.planning/phases/75-lower-js-ts-value-flow/75-01-PLAN.md) | WP2b lowering | 73, 74 | the closed construct list lowered in order of edges won; the walker retired; `Heuristic` edges 0 |
| 5 | [76](../../../.planning/phases/76-solver-scale-and-tiers/76-01-PLAN.md) | WP3 scale and tiers | 75 | fixed point with no budget event on the tune set; cost targets; tier assignment with assumption ids; held-out gate |
| 6 | [77](../../../.planning/phases/77-types-in-the-solver/77-01-PLAN.md) | WP4 types | 76 | typed ⊇ refined; 0 wrong exact; the join rule |
| 7 | [78](../../../.planning/phases/78-package-graph-v0-and-stitch/78-01-PLAN.md) | WP5 package graph v0 | 76 (77 optional) | export, stitch, composition test on the fixture and three trees, identity stability, size and stitch time measured |
| 8 | [79](../../../.planning/phases/79-correction-layer-proof/79-01-PLAN.md) | WP6 correction layer | 78 | recorded edges applied as corrections; 100% recall on one package; stale detection |

Order of execution: 72, then 73 and 74 in parallel, then 75, 76, 77, 78, 79.
77 and 78 may run in parallel after 76.

Rules every pull request follows (from `AGENTS.md` and
[../README.md](../README.md)): evidence on every edge; two tiers never
mixed; wrong exact claims stay at 0 on every plan; no tuning on the held-out
set and no recogniser for one benchmark package; a model for one library, the
engine for two; public surface unchanged (no CLI command, flag, SDK view or
output format; everything crate-private; the leak gate green); the bench's
`make js-bench` run and its report diff attached to the pull request; the
Rust and polint skills consulted before Rust changes; no project-management
references in shipped comments.

Every phase's exit includes the standard gate from `Makefile`:
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
--all-features --locked -- -D warnings`, `cargo test --workspace --locked`,
`cargo test -p polint --test public_surface_leak --locked`, and
`make js-bench JS_BENCH_ARGS="--no-cost"` with its report byte-stable across
two runs. Cost figures are produced once per phase on a quiet machine with
`make js-bench` and compared with the previous phase's committed report.

What only a human can do, stated once: nothing in this sequence. Merging
#141 and each stacked pull request is Emil's call, and the stack does not
wait for it; each phase rebases on the previous phase's branch.
