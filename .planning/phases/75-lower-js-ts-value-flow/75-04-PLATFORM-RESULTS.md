# Task 4 — Platform model implementation results

Scope: Task 4 of `75-01-PLAN.md`; Tasks 1–3 remain the foundation and Task 5 is not completed by this work.

## Implementation commits

- `2b5abb1c` test(ts): require conservative platform callback edges
- `3d3f9c20` feat(ts): lower embedded platform behaviors into model constraints
- `84ef6171` test(ts): cover generator spreads and open platform boundaries
- `2b675ec7` fix(ts): preserve open shapes and defer bound target calls

## Schema and lowering

`crates/polint/models/platform/javascript.toml` is embedded with `include_str!` and validated once. Schema version 1 contains `[[methods]]` with `names` selectors (`global:`, `member:`, `new:`), `calls = [{ param, args, this, args_from }]`, `returns`, optional `allocate`, `stores = [{ base, field, value }]`, and a closed `operation` vocabulary. Value paths name `$this`, `$paramN`, `$callN`, `$result`, `$prim`, `$open`, and validated heap fields. The loader rejects unknown fields, versions, paths, forward callback-result references and duplicate selectors.

Corrections to the plan sketch:

- The existing rows cannot express heap destinations or variable argument lists through `returns` alone. Explicit stores and allocation describe those shapes; closed operations instantiate forwarding, binding, cardinality, descriptor copying, event slots, promise aggregates and transitive element flow.
- The private loader lives in `ts/constraints/platform.rs`, with instantiation in `ts/constraints/lower/platform.rs`. The repo-local loader's static source/target-pattern format cannot name callback parameters or heap updates. Both paths converge on validated `ConstraintKind::ModelEdge` and the existing solver adaptation budgets; no public loader or SDK surface is added.
- Callback signatures and uncertain platform results stay open. Callback/forwarding edges use Conservative precision, with an explicit unresolved marker. Internal call constraints propagate arguments and returns without projecting an exact edge at the source site. No total/exact bind implementation is claimed: local binding copies retain a target, receiver and leading arguments, and instantiate forwarding when the returned function is called. Binding itself emits no edge to the target.
- The data digest and a platform projection algorithm label participate in the semantic-graph parameter digest. A test changes model behavior and observes a changed cache key.
- Async results and generator results allocate promise/iterator shapes. Await loads `$fulfilled`; yields and iteration use `$elements`. Spread-generator destructuring uses aggregate elements with an open guard when position/order is unavailable.
- Review found that copying the known part of a union object shape could falsely close a property. The failing test demonstrated `Object.assign({}, condition ? {run: target} : unknown)` yielding an exact target. Partial/aliased shape operations retain an escape guard; all four Object copy/descriptor variants have regression coverage.

## Validation

- Tests were added and run failing before implementation: builtin callbacks had no conservative edge. Additional failing tests covered spread-generator destructuring and partial object shapes.
- Focused platform tests: 14 passed. Adaptation tests: 24 passed; also covered by the complete library run.
- `cargo test -p polint --lib --locked`: 3,012 passed, 21 ignored, 0 failed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo test --locked --test public_surface_leak -p polint`: 8 passed.
- `rustfmt --edition 2024 --check` passed for all nine touched Rust files; `git diff --check` passed.
- The required benchmark command completed with exit 0. No release build or benchmark was started before `/tmp/js-bench-75c4/report.json` existed. The final reports are `/tmp/js-bench-75d/report.json` and `report.md`.
- All Cargo commands inherited the required shared target directory and Go toolchain. The protected fixture lockfile was excluded from every commit. No push, branch or worktree creation was performed.

## Acceptance

| Criterion | Result |
| --- | --- |
| Callback family, no types | 9/9 calls and 4/4 callbacks |
| Promise family, no types | 4/4 calls and 1/1 callback |
| Call/apply/bind family, no types | 5/5 calls; bind has no extra installation edge |
| Iteration family, no types | 8/8 calls, including spread-generator destructuring |
| Callback bucket falls by at least 3/4 | Met: 162 → 35 (78.4% reduction; limit 40) |
| Call/apply/bind bucket falls by at least 3/4 | Not met: 94 → 56 (40.4% reduction; limit 23) |
| Wrong exact / false-complete | 0 on base and refined, tune and held-out; construct false-complete also 0 |
| Model digest invalidates cache | Passed |
| Builtin receiver abstention | Passed; shadowed globals also covered |
| Zod stringify replacer | Conservative callback plus unresolved marker |
| Zod globs-only refined cost | 55.116 → 56.218 seconds, 4,459 → 4,493 MiB; no timeout |

The configured benchmark plans were base and refined (no types). Required families include their held-out construct cases. Held-out package source was not read or used for tuning.

The stringify evidence is `src/v4/core/json-schema-processors.ts:750`, source span `31898..32028`: the inline replacer at `31920..32027` is a Resolved/Conservative edge, alongside an Unresolved/Unknown marker. The requirement's `replacer` is an inline arrow in the pinned package. Tune facts retained at `/tmp/platform-zod-refined.json` corroborate it.

The residual call/apply/bind bucket is async 39, axios 11 and underscore 6. All three report `BudgetExceeded` with `adaptation.max_targets_per_source`; the shared cap is 16. Remaining examples also include a computed receiver (`this[methodName].call`) and higher-order/module flows (`asyncFn.apply`, `sourceFunc.apply`, `func.apply`). This target is stopped with that evidence; the solver's budget policy is unchanged. The report does not attribute every residual edge exclusively to the cap.

Exact recall is unchanged: base tune/held-out 44.53%/27.12%, refined 48.49%/32.48%. Refined exact-or-conservative recall increases from 48.49% to 53.49% on tune and from 32.48% to 33.45% on the counted held-out set. The broader held-out gap remains outside the later whole-plan target.

## Comparison output

```text
== /tmp/js-bench-75c4/report.json
 base: tune 44.53 held 27.12 wrong_exact 0 false_complete(ho) 0
 refined: tune 48.49 held 32.48 wrong_exact 0 false_complete(ho) 0
 tune apart_only found 1055 tune false-complete {'base': 0, 'refined': 0}
 buckets {'local_member': 107, 'module_member': 17, 'constructor': 1, 'this_member': 107, 'parameter_member': 119, 'parameter_call': 10, 'callback': 162, 'local_call': 36, 'computed_key': 17, 'call_apply_bind': 94, 'object_literal_member': 14}
 zod refined 15318 1207 MB ['peak_rss_bytes', 'wall_ms']
== /tmp/js-bench-75d/report.json
 base: tune 44.53 held 27.12 wrong_exact 0 false_complete(ho) 0
 refined: tune 48.49 held 32.48 wrong_exact 0 false_complete(ho) 0
 tune apart_only found 1092 tune false-complete {'base': 0, 'refined': 0}
 buckets {'local_member': 90, 'module_member': 7, 'constructor': 1, 'this_member': 107, 'parameter_member': 119, 'parameter_call': 8, 'callback': 35, 'local_call': 30, 'computed_key': 17, 'call_apply_bind': 56, 'object_literal_member': 8}
 zod refined 16255 1214 MB ['peak_rss_bytes', 'wall_ms']
```

## Retained fallback and remaining work

No builtin recognizer was removed from `ts/callable_flow/extract.rs`. Full replacement parity is not established: bound values through call results/heap slots, alias/partial descriptor shapes, iterator ordering/rest/resume values, collection positional precision, and dynamic event names still need the fallback or stay open. Task 5 owns disabling and measuring the legacy projection before retirement. The former `Object.create`, `Object.setPrototypeOf`, and data-descriptor cases in the new row lowerer's `builtin_call` were replaced by model data; CommonJS `require` remains frontend module handling.

The platform model intentionally over-approximates promise settlement, collection element/key unions, callback predicates and getter timing. Getter return flow is instantiated eagerly with conservative evidence; exact property-read timing and complete accessor behavior remain outside this result. EventEmitter registration/emission is keyed only by constant event names. There is no recognizer keyed to a benchmark package and no CLI, SDK or output-format addition.

Next work should address the capped model expansion and remaining callee/receiver flows, then measure fallback-only edges before retiring any legacy builtin family. This result does not mark the whole plan or phase complete.
