---
phase: 77-types-in-the-solver
plan: 01
subsystem: type-directed-calls
requirements-completed: [PG-08]
status: executed-with-deviations
---

# Types weigh the value-flow answer; they never claim exact

The checker's facts now act on the refined answer in one step after the solve
(`refined_calls/type_assignment.rs`). Declared types say what a call may run,
not which value flows to it, so the step claims nothing exact. It moves an exact
edge the checker's declared kind or arity says cannot run to conservative and
keeps it (`type_mismatch`), adds methods of the receiver's declared class at an
open site (`type_seed`), and lists the checker's targets at a site no tier
answered, conservative and open. Every other typed row is dropped. The typed
plan claims no wrong exact target on either set, and the bench fails when it
finds fewer observed edges than the refined plan in any tier.

## Commits

| commit | change |
|---|---|
| e71ec6af | The sidecar sends whether a receiver is an intersection or an object literal |
| 608cb4c0 | A typed answer is exact only where it names the function the call runs (the join rule; the fifteen wrong targets as fixtures in #151) |
| a959ca95 | Bench reports the type tier's join counts per package |
| 2f8338fb | The sidecar sends a callable's required parameters and a call's declared parameter count |
| 63275321 | Declared kind and arity joined to native functions and call sites |
| 9cd6adc4 | The type step: mismatch, seed, the checker's answer at an unanswered site |
| 783859be | Bench counts the type step's effects; `types_never_lose` fails the run when typed finds less than refined |
| 95ff6852 | Whether a call constructs is read from the checker's syntax (`new`, `super(...)`) |
| 7eb024aa | TIERS.md, CALL-EDGE-TIERS.md and WP4 describe the step as built |
| ca48ae8e | Baseline with the type step |
| 945cba4f | Dense-id sweep re-baselined: the sidecar digest renders the two parameter counts; the rest are test assertions |

## Deviations from the plan

- **No `TypeConstraint`, no `ts/constraints/types.rs`.** Types do not enter
  the solve as constraints. A filter inside the solve would remove targets from
  points-to sets before tiering, and a wrong filter would then delete an edge
  rather than demote it; seeds inside the solve would let a declared type add
  exact targets. The step runs after the solve and every other tier, on edges
  that already carry their tier, which keeps "never added by type alone" and
  "kept as conservative" true by construction. The verify path
  `ts::constraints::types` and a `solver_composition` test file do not exist
  for this reason; the step's tests are in `type_assignment.rs`, `ts_types.rs`
  and `analysis_kernel/ts_types_tests.rs`.
- **Arity added to the sidecar.** The mismatch filter needed what the checker
  knows about parameters: `required_parameters` on a callable (null for a JS
  file and an accessor) and `signature_parameters` on a call site (null for a
  rest parameter, a JS declaration file or no resolved declaration). Arity
  demotes only at a site the checker resolved to one signature.
- **Exact recall trade-off.** The typed tier used to claim exact where it named
  the target and the solver did not answer. Those sites now move from typed
  exact to conservative and stay open. Typed exact recall equals refined exact
  recall on both sets; the plan's "typed finds every edge the no-types plan
  finds" holds in every tier, and the fifteen wrong exact targets are gone.
- **The never-lose check found a native kind defect.** The first full run failed
  `types_never_lose` on axios, commander and pako. The type step read whether a
  call constructs from the native call kind, which guesses construction from a
  capitalised callee name: a plain call of zod's `OK`/`DIRTY` arrows counted as
  constructing (57 false `constructs_a_non_constructor`), and `super(...)`, kind
  `Function`, did not count (false `calls_a_class` on axios and commander). The
  sidecar now reports `super` as its own call kind and the step uses the
  checker's syntax. The native extractor is unchanged: its kind feeds the base
  and refined tiers and changing it is a separate fix. pako, held out, passed
  after the general fix; its source was not read.
- **Wrong targets beyond exact.** The plan's acceptance asks for typed wrong
  single and multi 0 everywhere. Those counts include conservative answers, where
  the refined plan itself has hundreds of multi-target wrongs (async 764). Typed
  equals refined on both counts on every tune package except commander, where the
  checker's answer at `target.parseArg(value, previous)` (declared
  `Option | Argument`) lists Argument's arrow alone and the run called Option's:
  one conservative single target at a site the step leaves open. The sidecar's
  union answer for a property assigned in JS constructors can list fewer members
  than run; that is a known gap.

## Join counts

Unmatched callees (a typed declaration with no native function by span): zod 80,
every other package 0. They get no edge.

## Type step effects

On the tune set, after the construction fix, no `type_mismatch` and no
`type_seed` fires; before it, 59 mismatches fired and every one was false. The
typed plan's gain is the checker's targets at sites no other tier answered.

## Results

Baseline `tests/js-analysis-bench/baseline/` at ca48ae8e, TypeScript 5.9.3,
load average 11.66 before and 13.62 after.

| package | exact refined / typed | exact + conservative refined | typed | target 85% |
|---|---|---|---|---|
| axios | 302 / 302 | 569/675 (84%) | 603/675 (89%) | met |
| zod | 674 / 674 | 763/1308 (58%) | 771/1308 (59%) | 418 found only apart; 112 no target, marked unresolved |
| commander | 34 / 34 | 85/318 (27%) | 298/318 (94%) | met |
| async | 325 / 325 | 485/637 (76%) | 490/637 (77%) | 141 found only apart |
| chalk | 43 / 43 | 56/72 (78%) | 56/72 (78%) | 16 no target, marked unresolved (computed-key and other member calls) |
| dayjs | 34 / 34 | 88/112 (79%) | 88/112 (79%) | 24 no target, marked unresolved (member calls on object-literal functions) |
| underscore | 196 / 196 | 250/260 (96%) | 250/260 (96%) | met |
| **total** | 1608 (47.5%) | 2296 (67.9%) | 2556 (75.6%) | **not met** |

Over every tier including apart, typed finds 3126/3382 (92.4%). The 85% target
for exact plus conservative is not met: zod and async lose most of the gap to
edges the refined plan finds only as heuristic, which the type step does not
promote; chalk and dayjs lose theirs to member calls neither the solver nor the
checker answers. Held out, exact plus conservative goes from 83.5% to 84.3%.
Wrong exact targets: 0 on every package on both sets. False-complete call lines:
0. Conservative wrong targets, single / multi: tune 1 / 1083 refined and
2 / 1083 typed (the commander union site above); held out 0 / 71 refined and
0 / 75 typed, from the checker's targets at sites no other tier answered. The
held-out sources were not read to explain the four.

## Mutation checks

- **Enclosing-function fallback restored in the callee join** (a declaration no
  native function is named for joins the narrowest function around it). The
  unit test `a_declaration_no_native_function_is_named_for_gets_no_enclosing_function`
  fails. On the bench, zod's unmatched callees go from 80 to 0 and its
  conservative `TypeHierarchy` rows from 258 to 335; no tier's recall and no
  wrong-target count moves. The underscore- and axios-shaped fixtures in #151 no
  longer detect it: they caught the fallback as a false-complete line when the
  typed tier claimed exact, and the type step now caps every typed row at
  conservative and leaves the site open, so a wrong joined target cannot make a
  site complete. The unit test and the join counts are the guard. Reverted.
- **Union member promoted to `SetupAware`.** Moot: the type step caps every
  typed row at conservative after the join, so no promotion inside the join can
  reach the exact tier.
- **Mismatch as deletion** and **skipping the typed ≥ refined check** are
  guarded by `type_assignment.rs` (a demoted edge stays, conservative) and by
  `selftest.py check_types_never_lose` (a deleted or demoted observed edge fails
  the run). The check is not idle: it failed the first full run on the native
  kind defect above.

## Verification

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: clean.
- `cargo test --workspace --locked --no-fail-fast` with `GOTOOLCHAIN=go1.25.12`,
  `POLINT_TS_TYPESCRIPT` and `POLINT_REQUIRE_TS_TYPESCRIPT=1`: 3358 passed,
  0 failed, 23 ignored over 41 test binaries. The first run failed
  `internal_architecture::dense_id_sweep_matches_baseline`; the hits are
  classified in 945cba4f.
- `cargo test -p polint --test public_surface_leak --locked`: passed.
- `python3 tests/js-analysis-bench/selftest.py`: ok.
- Two `bench.py report --no-cost` runs: `report.json` is identical once the
  dump time and peak RSS fields, which the dump records even without cost, are
  stripped; both equal the committed baseline under the same stripping.
