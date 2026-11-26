---
phase: 76-solver-scale-and-tiers
plan: 01
subsystem: points-to-solver
requirements-completed: []
status: executed-with-acceptance-gaps
---

# Solver scale, tiers with assumptions, cost and the held-out gate

The solver runs over dense indices with compact sets, merges copy cycles, bounds field hops, and claims a tier for every derived TS/JS call edge with its assumptions. The tune set solves with no budget event. Wrong exact targets are 0 on both sets, and exact-tier precision is 100%. Three cost targets are missed and the held-out gate is not met; each miss is stated below with its cause. **Per the plan, the missed cost targets block Phase 78's three-tree measurement until they are met.**

## Commits

| commit | change |
|---|---|
| e92555f0, 14c6fa0e | Solve statistics in the log and a `## Solver` table in the bench report; stored solver inputs and an ignored release test that solves them |
| d40745ed | Dense indices, sorted-vector/bitset sets, index-based worklist |
| dd450c09 | `max_indirection_depth`, sites past it open with `indirection_bound` |
| 57b19a53, 9138b666 | Copy-cycle merge, batched Tarjan walks; the cycle test catches a merge of an outside variable |
| 580358f7, dbec734b, 942c4dee | Edge claims, inflow kinds, assumptions; a tier per derived call edge |
| 255d5374, 0d3eb7ce, cd16b2a7 | Inflow as an unresolved reason; assumptions on `RefinedCallEdgeFact`; tier from the claim |
| d6b0800e, e053a2cc, 5157a23d | Bench check that exact points-to edges name assumptions; tier semantics documented as internal |
| f6611fcd | Dense-id sweep re-baselined for the tier tests |
| b1a57853 | Baseline with the prototype walk restored |
| 81f6abdd | Adaptation caps no longer applied to platform model edges |
| 263da215 | Baseline with platform model edges uncapped |

## Budget events on the tune set: fixed

Before 81f6abdd, async, axios and underscore refined reported `AdaptationMaxTargetsPerSource` / `AdaptationMaxModelDerivedEdges`. Those caps bound repo-local model facts, but the solver also counted the platform model edges that value-flow lowering derives from the shipped platform model (evidence `[platform:<model>, platform_digest:<digest>]`). The solver now recognises a platform model edge by its exact evidence shape and current platform digest, and holds only adaptation edges to the caps. Two tests fix the behaviour: twenty platform targets on one source are all kept while adaptation edges stop at 16; a model edge with a foreign platform digest stays capped.

Every tune package now reports `WithinBudget`. async's exact + conservative recall rises from 70% to 76% (tune total 66% → 68%). Its conservative precision falls from 40% to 17% because platform targets are no longer cut: a site can now carry up to 183 targets. That is the honest conservative answer ("may reach"); exact claims are unchanged.

## Cost

Dump wall time of `polint` fact extraction, release build. Load average is stated with each column: this machine is shared and was never quiet, so the plan's "quiet" condition was not met for any run.

| package | Phase 75 baseline (load 125–181): base / refined ms | this baseline (load 16–67): base / refined ms | refined / base | target |
|---|---|---|---|---|
| axios | 2,363 / 11,706 | 768 / 3,339 | 4.3x | ≤ 1,500 ms, ≤ 3x — **missed** |
| zod | 17,675 / 44,592 (peak 1,305 MB) | 4,804 / 18,498 (peak 1,346 MB) | 3.9x | ≤ 20,000 ms and 2 GB — **met**; ≤ 3x — **missed** |
| commander | 530 / 4,409 | 268 / 1,964 | 7.3x | ≤ 3x — missed |
| async | 1,455 / 29,982 | 465 / 12,476 | 26.8x | ≤ 3x — **missed** |
| chalk | 183 / 558 | 74 / 260 | 3.5x | ≤ 3x — missed |
| dayjs | 176 / 727 | 86 / 295 | 3.4x | ≤ 3x — missed |
| underscore | 592 / 2,604 | 221 / 885 | 4.0x | ≤ 3x — missed |

The Phase 75 column was measured under about three times the load, so it overstates the before cost. The stored-input solve isolates the solver from load and from the other providers (load 14–20):

| input | before: input / worklist / result ms (total) | after | resident growth before → after | process peak before → after |
|---|---|---|---|---|
| zod refined | 347 / 341 / 558 (1,246) | 88 / 87 / 427 (602) | 207 → 76 MB | 265 → 168 MB |
| zod, globs alone | 1,835 / 916 / 1,900 (4,651) | 484 / 240 / 1,473 (2,197) | 690 → 266 MB | 917 → 607 MB |
| async refined | 53 / 537 / 275 (865) | 11 / 137 / 189 (337) | 72 → 33 MB | 89 → 73 MB |
| axios refined | 78 / 20 / 57 (155) | 19 / 5 / 46 (70) | 39 → 13 MB | 73 → 73 MB |

### Repeated measurement against the gates

Three repetitions each, base and refined interleaved, load average 27–90 (median):

| package | base ms | refined ms | refined / base |
|---|---|---|---|
| async | 593 | 12,695 | 21x |
| axios | 901 | 3,624 | 4.0x |
| zod | 4,639 | 18,080 | 3.9x |

An earlier async refined figure of 37 s was taken at load ~90; at lower load it is 12.7 s. That is still **21x base, against the "refined at most 3 times base" gate: missed**.

### Profile of the misses

No sampling profiler (`perf`, `cargo flamegraph`) is available on this machine, so the profile is the per-stage time the dump logs. async refined, wall 12,476 ms:

| stage | ms | peak MB |
|---|---|---|
| evidence | 4,109 | 1,254 |
| data_flow | 2,302 | 1,254 |
| solver | 1,461 | 356 |
| semantic_graph | 950 | 274 |
| abstract_domains | 417 | 258 |
| refined_calls | 325 | 361 |
| cfg | 269 | 141 |

The solver is not the cost. async has about 46,000 heuristic (`SummaryProjected`) call rows: wrapper functions return whatever they are given, and without context sensitivity every caller of a wrapper sees every value any caller passed. `data_flow/direct_calls.rs` expands each row per argument, and `evidence` mirrors and hashes every data-flow fact, so both scale with those rows. The same rows are why async still has 45,951 apart edges and so misses "heuristic edges on the tune set: 0". The fix is context-sensitive wrapper returns (or summaries keyed by argument), which is engine work beyond this phase.

axios (3.3 s against 1.5 s) spreads over `evidence` 0.5 s, `semantic_graph` 0.5 s and the solver 0.4 s with no single stage dominating. commander, zod, pako and dayjs exceed the "semantic_graph at most 10% of refined time" target (54%, 28%, 63%, 26%).

## Tiers

Refined plan, `--no-cost` content identical in the full run:

| set | exact-complete | exact | exact + conservative | apart only | exact-tier precision | wrong exact targets |
|---|---|---|---|---|---|---|
| tune (3,382 edges) | 1,436 (42%) | 1,608 (48%) | 2,296 (68%) | 697 (21%) | 1,608/1,608 (100%) | 0 |
| held-out (5,453 edges) | 1,759 (32%) | 1,759 (32%) | 4,556 (84%) | 269 (5%) | 1,759/1,759 (100%) | 0 |

Exact-tier recall without types is 48% on the tune set, against the 60% target: missed. Every exact points-to edge names its assumptions; the bench stops on one that does not.

## Held-out gate

| plan | tier | tune | held-out | points below tune | within 5 |
|---|---|---|---|---|---|
| refined | exact-complete | 42.5% | 32.3% | 10.2 | no |
| refined | exact | 47.5% | 32.3% | 15.3 | no |
| refined | exact + conservative | 67.9% | 83.5% | −15.7 | yes |

Per package, refined exact tier against the tune set's 47.5%, with each exact-tier miss put in a bucket from bench numbers only (no held-out source was read):

| package | observed | exact | points below tune | conservative only | apart only | not found | largest open reasons |
|---|---|---|---|---|---|---|---|
| acorn | 1,141 | 179 (16%) | 31.9 | 934 | 6 | 22 | DynamicProperty 1,539, Inflow(InwardThis) 1,099, Inflow(InwardParameter) 185 |
| express | 166 | 33 (20%) | 27.7 | 10 | 24 | 99 | DynamicProperty 192, MissingSemanticReference 175 |
| node-forge | 3,131 | 777 (25%) | 22.7 | 1,746 | 239 | 369 | DynamicProperty 3,152, Inflow(EscapedObject) 1,097, Inflow(InwardThis) 551, Inflow(InwardParameter) 411 |
| nanostores | 103 | 32 (31%) | 16.5 | 39 | 0 | 32 | MissingSemanticReference 61, DynamicProperty 56, Inflow(EscapedObject) 15 |
| co | 22 | 12 (55%) | −7.0 | 10 | 0 | 0 | — |
| immer | 295 | 226 (77%) | −29.1 | 27 | 0 | 42 | — |
| js-yaml | 338 | 265 (78%) | −30.9 | 11 | 0 | 62 | — |
| pako | 257 | 235 (91%) | −43.9 | 20 | 0 | 2 | — |

Cause buckets of the gap, and the construct families for the next phase's fixture list:

- **Inward `this` in methods of an exported constructor or class** (acorn, node-forge). The callee is found but the site is open: an exported method can be called with any `this`. Types (Phase 77) or the stitch (Phase 78) can close it; the solver must not.
- **Calls through an object that escapes** (node-forge, nanostores). Outside code may write the field, so the set stays open.
- **Parameters of exported functions** (acorn, node-forge). Same reasoning, `inward_parameter`.
- **Not found: dynamic property and missing semantic reference** (express). These sites get no target at all; they are lowering gaps, not tier gaps.

No change was keyed on a held-out package. The `none recorded` open sites (open sites whose answer gives no reason) exist in the base plan too and were not introduced here; they remain a defect to remove.

## Mutation checks

| mutation | guard | result |
|---|---|---|
| M1: flow the whole representative set along every copy instead of the new objects | stored zod solve, `POLINT_POINTS_TO_MAX_GROWTH_MB` | **not caught**: peak growth 116–118 MB against 116 MB, 650 ms against 889–1,153 ms at higher load. The clone is freed on each edge and zod's copy edges carry small sets, so neither memory nor time moves. |
| M2: an escaped function's parameters are not inflow | `only_an_escaped_function_has_open_parameters`, `an_escaped_place_is_open_with_the_way_it_escaped` | caught, both fail |
| M3: exact claim without `within_budget` under a tripped budget | `an_exact_claim_names_its_assumptions_and_needs_the_whole_budget` (`max_steps = 10`) | caught |
| M4: cycle merge also takes in each cycle's successors | `a_copy_cycle_is_merged_and_carries_later_objects_along_every_edge` | caught |

M1 shows the memory assertion guards a retained clone (the old solver kept cloned sets), not a transient one; a transient clone costs neither peak memory nor measurable time on the bench inputs.

## Verification

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: clean.
- `cargo test --workspace --locked`: all 41 test runs pass, including `public_surface_leak` and the `solver_composition` tests.
- `make js-bench JS_BENCH_ARGS="--no-cost"` twice: `report.json` identical with wall time, peak memory and per-dump cost removed (the convention since Phase 73; the stage-time and solver-time columns are cost and differ). Load 19–68.
- `make js-bench`: identical to the `--no-cost` runs outside cost; baseline committed in 263da215 (load 16–67).

## Blocked

Phase 78's three-tree measurement is blocked by the missed cost targets (async and axios wall, refined at most 3x base, semantic_graph share), as the plan requires. The async miss needs context-sensitive wrapper returns; the gate stays as written.
