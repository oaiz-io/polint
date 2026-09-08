# Jelly recall recovery: implementation decision

Continue the rejected private collector port, with measured fixes at its graph/ownership/module-shape boundaries. The archived collector already preserves **8,261 exact lines of the deleted implementation (96.96% of the old production section)**. A fresh restoration offers no demonstrated higher F1 ceiling. The immediate objective is the unchanged **0.785192926** threshold; allow **10–16 engineer-hours**, approximately **8–12 elapsed hours for two experienced implementers**, plus **2–4 hours contingency**. This is a planning estimate, not a promise that the next patch passes.

## Root cause and measurement boundary

Commit `3e25db23` removed the production callable-flow collector and separate heap frontend, replacing them with the canonical graph/shared solver but only a small TS token-flow collector (600 lines; parameter/return token models). Missing language-model inputs, rather than parser speed or build mode, explain the observed recall collapse: HEAD-equivalent reports score **249 TP / 8 FP / 1,230 FN**, F1 **0.286866**, while the rejected private port recovers **691 expected edges and loses zero existing TP**, reaching **940 / 36 / 539**, F1 **0.765784**. The port already recovers most iterator, promise, collection, class, and higher-order coverage. Its remaining gap combines lost constructor caller metadata, incomplete CommonJS wrapper/value shapes, smaller value-flow and identity gaps, and **342 expected edges into absent dependency source**. The later archived identifier-name repair has no corpus score. These statements use saved reports and inspected source; this mission did not rerun or modify the collector.

The before/after sets are supported by saved debug, release, and post-performance reports with equal semantic edge sets within each state. They are HEAD-equivalent evidence supplied by the earlier performance audit, not newly built measurements at `d713dbd2`. Sources and hashes: [VALIDATION.md](VALIDATION.md), `.context/accuracy-research/taxonomy/inputs.json`.

## Exact false-negative taxonomy

This is a **mutually exclusive case-family partition** of actual gate edges. Labels describe source fixtures, not isolated causal ablations: a collection case can fail because a callback binding or span is missing. Every case and edge is listed in the machine-readable ledger. Counts combine the gate's distinct `call2fun` and `fun2fun` assertions; one resolved source call can recover one or several scored edges.

| Fixture/model family | HEAD FN | Port FN | Recovered TP | Port FP |
| --- | ---: | ---: | ---: | ---: |
| Absent dependency source: helloworld | 342 | 342 | 0 | 2 |
| Classes, `this`, prototypes | 225 | 29 | 196 | 12 |
| Collections and array builtins | 123 | 36 | 87 | 4 |
| Dynamic properties, objects, accessors, other | 111 | 28 | 83 | 2 |
| Iterators and generators | 103 | 6 | 97 | 4 |
| Destructuring, rest, spread | 101 | 22 | 79 | 9 |
| Promises and async | 90 | 8 | 82 | 2 |
| Cross-module calls, callbacks, interop | 64 | 44 | 20 | 0 |
| Higher-order calls, defaults, arguments | 57 | 12 | 45 | 1 |
| Source-location stress | 14 | 12 | 2 | 0 |
| **Total** | **1,230** | **539** | **691** | **36** |

Cross-module residual 44: **10** in `client4`/`client5` interop fixtures, **18** in callback-oriented cases, **6** dynamic/import12 edges, **10** other module/hook edges. Six of the ten interop-fixture misses are calls to the `__importDefault` helper itself; four concern default constructor/method flow. Do not call all ten module-resolution failures. Likewise, `approx/deconstruction`'s four residual misses concern computed-key writes; they are not four free destructuring wins. Unknown facts remain **913 in both states** and the report's built-in categorized-failure counters are zero: neither supplies an FN taxonomy.

### The 342-edge dependency block

All 342 misses in `tests/helloworld/app.json` target missing `tests/helloworld/node_modules/*` files. The inspected original corpus currently has 42 missing paths among 131 source paths referenced by report edges, including 34 distinct missing FN target files; no other FN targets an absent file. This is a source-availability explanation supported by today’s preserved checkout, not an independently snapshotted proof of the filesystem at historical report time. `prepare_case` includes graph-listed paths only when `path.is_file()` (`crates/polint-eval/src/harness/external/jelly_callgraph.rs:62–75`). Installing pinned dependencies would be a distinct corpus-setup experiment, not a collector fix; no dependencies were installed here.

With these exact source inputs frozen, and without inventing absent-source functions, at most 1,137 expected edges are recoverable: **F1 ceiling 0.869266 at zero FP**, or **0.857466 retaining 36 FP**. This is an input-conditioned arithmetic bound, not an intrinsic ceiling for polint or this architecture. There remain **197 source-present FN**, enough in aggregate for the fixed gate. It is unnecessary to chase the dependency block tonight.

### What the two named gaps can actually buy

Source inspection locates the constructor boundary: the archived collector writes `TsCallableFlow.caller`, graph projection drops it, TS points-to uses `site.caller`, and refined-call projection rejects solver edges whose source disagrees with `site.caller`. Therefore an override only at the solver boundary will fail. Correct constructor ownership consistently, preserving static-block, field-initializer, method, and nested-callback ownership.

Eight exact false-caller FP/FN pairs are visible: `classes` 1, `private` 4, `super` 2, `super5` 1. A **score-only counterfactual**, changing those eight pairs without other changes, yields **948 TP / 28 FP / 531 FN, F1 0.772301**. This is not an executed repair. Even that ideal result still needs **27 additional TP with FP fixed at 28**. Constructor attribution plus a single CommonJS wrapper repair is not a sufficient implementation plan.

The wrapper failure has concrete shape evidence: recognized `__importDefault` forwards its argument unchanged; a bare `module.exports = function` is callable but has no `.default` property. The historical passing test exercised the different `__esModule=true; exports.default=...` branch. Restore both behaviors with shape-aware, heuristic modeling, plus helper-call binding. Never weaken the gate renderer or ownership validation to manufacture these edges.

## Compared paths

| Path | Effort estimate | F1/ceiling evidence | Risk and decision |
| --- | --- | --- | --- |
| **(a) Continue rejected port** | **10–16 engineer-hours; 8–12 elapsed with two people**, plus 2–4 contingency | Last measured 0.765784; immediate 0.785–0.80 target is plausible but unverified; fixed-input upper bound 0.869266 | **Choose.** Preserves measured +691 TP; remaining integration and fixture gaps are inspectable. |
| (b) Fresh port of deleted production collector | 16–28 engineer-hours | No demonstrated advantage over (a); 97% of old production is already reused there | Repeats adaptation, risks restoring obsolete resolver/direct provider and separate heap solver. |
| (c) Generic constraint rewrite now | 40–80+ engineer-hours | Unknown practical ceiling; same fixed-input bound | Too much semantic-model and ordering work for tonight. |
| (c) Staged hybrid | Same immediate work as (a); later refactor separate | Same immediate measured point and ceiling as (a) | **Architectural direction:** private bounded shape/model extraction feeds canonical constraints; one shared solver produces edges. |

The original file is 11,883 lines including 3,363 test-section lines and 67 tests. The archive has 8,376 extraction lines plus 200 module/test lines and eight changed glue files (+368/−54). Of 262 old function declarations, 241 declaration-to-next-declaration chunks are byte-identical. Only four helper names were removed and two introduced. Separately deleted heap code adds 2,453 lines; its isolated accuracy contribution has not been measured. The archived tests reduce the old 67 tests to four functions containing matrices: recover the relevant behavior specifications through today's kernel, not old provider-specific assertions.

Historical performance notes report still higher scores on earlier revisions, but use different implementation/evaluation conditions. They are fixture leads, not a forecast or proof of today's achievable F1.

## Top three risks

1. **Identity/ownership consistency:** constructor corrections can break stable IDs, nested-call ownership, solver joins, and refined validation. Carry the fix from its earliest canonical boundary and test caller and target together.
2. **Precision and score optimism:** 28 new FP accompany the recovered TP; array/rest broad unions and name-only interop recognition can add more. A fixture count is an opportunity ceiling, not predicted payoff. Recompute the required TP from actual FP after each slice.
3. **Archive/cache/corpus provenance:** the final archive is unmeasured, omits cache-version changes, and raw reports depend on a specific incomplete dependency checkout. Hash inputs, version changed semantics, and validate cold/warm and debug/release equivalence before acceptance.

The exact first three implementation steps and the remaining targeted work queue are in [RECOMMENDED_IMPLEMENTATION.md](RECOMMENDED_IMPLEMENTATION.md). No final solution, PR, baseline edit, or Go change was made in this research mission.
