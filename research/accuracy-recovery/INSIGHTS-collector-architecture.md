# Collector restoration: source-level feasibility

Agent / angle: collector architecture and real port size  
Question: how much deleted production code remains reusable, and where does the rejected port lose model information at today's boundaries?  
Date: 2026-09-08  
Status: reviewed source findings; implementation estimates remain estimates

## Protocol and evidence boundary

Scope: read the preserved reports first; compare the deleted Rust source, archived rejected source, and HEAD. Include JS/TS callable-flow implementation, private projection/solver integration, focused fixtures, and cache contracts. Exclude Go modifications, baseline changes, external architectural rewrites, product edits, builds, and mutation experiments. Source comparison used Python `difflib.SequenceMatcher(autojunk=False)` over exact lines and regex-indexed function declaration chunks. It is a textual reuse measurement, not a semantic equivalence proof.

Sources are local primary implementation evidence, accessed 2026-09-08:

| Source | Revision/path | Role |
| --- | --- | --- |
| Current checkout | `d713dbd2e1c16ee5d89c3341db29fc585ee94c1b` | Current architecture |
| Deletion | `3e25db23fd28dfe843c989f0b0e58486634d141c` | Retired competing frontends |
| Deleted source | `7730a730dc6e912d9a11ac41b40259ea65c8cf37:crates/polint/src/analysis/calls/ts_value_flows.rs` | Last pre-deletion collector |
| Preserved disposition | `.context/perf-research/accuracy-restoration-RESEARCH.md`, `FINAL-REPORT.md` | Corpus-verified 0.765784 boundary and later unverified repair |
| Archived source | `.context/perf-research/rejected-accuracy-restoration/callable_flow/{extract,mod}.rs` | Rejected private port |
| Archived integration | `.context/perf-research/rejected-accuracy-restoration/tracked.patch` and `files/crates/polint/src/` | Integration changes |

No new F1 result was produced by this subtask. Per-case recovery opportunity must come from the separate FN audit; lines of source do not quantify TP opportunity.

## High-confidence findings

### Paths (a) and (b) share nearly the entire language model

The rejected collector is already a port of the deleted production implementation. Starting another port is largely repeating completed adaptation work.

| Measurement | Count | Interpretation |
| --- | ---: | --- |
| Deleted whole file | 11,883 lines | Production plus tests |
| Before `#[cfg(test)]` | 8,520 lines | Includes the separator after the 8,519 production lines named in the mission |
| Deleted test section | 3,363 lines, 67 `#[test]` functions | Substantial reusable behavior specifications |
| Archived `extract.rs` | 8,376 lines | Production private collector |
| Exact line matches old production → archive | 8,261 | 96.96% of old production, 98.63% of archived extract |
| Old/new `fn` declarations | 262 / 260 | Includes methods with repeated names |
| Exact function declaration-to-next-declaration chunks | 241 / 262 | Conservative textual lower bound; chunks include trailing comments/impl boundaries |
| Archived `mod.rs` | 200 lines, four test functions | One eight-case model matrix, negative-result matrix, three-case import matrix, parser recovery guard |
| Tracked integration | Eight files, +368 / -54 lines | Includes focused tests; excludes the two new collector files |

Only four old function names disappear: `resolve_ts_value_flow_targets`, `module_resolve_options`, `value_flow_algorithm`, `value_flow_target_stable_key`. Two replacement names appear: `collect_callable_flows`, `value_flow_kind`. Most changes are imports, borrowed AST lifetimes, output-row types, and the resolver boundary. The archive additionally fixes array element collection (`extract.rs:6347`), rejects call/member expressions masquerading as function expressions due to equal start offsets (`6697`), and uses the function-span index (`6744`). These changes need preserving, not blindly replacing with historical text.

The deletion also removed `analysis/calls/js_points_to/{harvest,mod,provider,solver}.rs`, 2,453 lines in total. That separate heap frontend/solver is outside the 8,519-line collector. Restoring it would create another algorithm and provider path, with extra precision/deduplication and lifecycle obligations. Its effect is not isolated by the available 0.766 measurement.

HEAD's `ts/token_flow.rs` is only 600 lines and describes two token-flow kinds (`ParameterArgument`, `ReturnValue`, lines 24–27). The collector call sequence at 96–100 indexes simple parameter calls, local aliases, returned functions and local return assignments. This is source evidence of a much smaller replacement, independent of the measured recall collapse.

### Actual integration work already represented by the archive

| File | Archived change | Retain/check when implementing |
| --- | --- | --- |
| `ts/callable_flow/{mod,extract}.rs` | Private AST model → `TsCallableFlow` rows | Keep private; no direct `CallTargetFact` provider |
| `ts/semantic_graph.rs` | `analyze_parsed_ts_file` borrows the existing parsed source | Preserve recovery gating |
| `ts/semantic_graph_build.rs` | Parse each file once for this collection, retain arenas through summary rounds, project model rows to graph | `collect_ts_file_analyses:290`; `project_ts_callable_flows:602` in archive |
| `analysis_neutral/semantic_graph/build.rs` | Constraint precision setter | Preserve heuristic label |
| `ts/points_to.rs` | Carry callsite precision, use namespaced variables, remove broken field-load stable-key join | Shared solver stays responsible for edges |
| `analysis_neutral/points_to/vars.rs` | Semantic-node variable namespace | Prevent collision with dynamic heap slots |
| `ts/mir/lower.rs` | Deduplicate body owner spans | Prevent duplicate MIR identities |
| `analysis_neutral/calls/extract.rs` | Preserve literal `fn`/`callable`/`callback` identifier evidence | Later fixture repair lacks corpus verification |
| `ts/mod.rs` | Private collector module | No public SDK expansion |

The graph projection explicitly copies a property-load result into the callsite (`archived semantic_graph_build.rs:735–751`). Full constraint keys include their kinds, so the earlier solver-side attempt to equate a FieldLoad key with a CallConstraint key could not express this flow reliably.

### Constructor caller metadata is dropped at a concrete boundary

The archive sets `caller_override` only for constructor bodies (`extract.rs:680–773`), and each emitter saves that owner in `TsCallableFlow.caller` (`6581`, `6619`, `6650`, `6689`). The projection at archived `semantic_graph_build.rs:602–653` reads the site and target but never reads `flow.caller`. HEAD `ts/points_to.rs:90` constructs the source from `site.caller`. Finally, `analysis_neutral/refined_calls/provider.rs:255–258` rejects a derived edge if its source function differs from `site.caller`.

Therefore, merely carrying the override into the solver is insufficient: refined projection would discard it. Correct canonical TS constructor ownership through MIR → callsite → semantic graph → solver → refined edges, or introduce an explicit validated semantic ownership representation shared by those stages. Avoid late benchmark export remapping, silent removal of validation, or duplicate caller edges as a shortcut.

The independent report audit now bounds the simplest repair: `.context/accuracy-research/caller-counterfactual.json` identifies eight exact FP/FN caller swaps in classes (1), private (4), super (2), and super5 (1). Correcting just these would yield TP948 / FP28 / FN531, F1 **0.772301**. This is a counterfactual, not a run. At FP28 it still needs TP975, **27 additional true positives**, to pass. Constructor ownership plus the unquantified CJS fixture is therefore not an evidence-backed sufficient plan.

Do not attribute every class-contained call to the class. HEAD `ts/mir/lower.rs:954–1000` and archive `extract.rs:683–737` deliberately preserve separate field-initializer and static-block owners. Named methods and nested callbacks also retain their own owners. The archived emitter's broad introductory comment about class fields is less precise than its implementation.

### CommonJS helper identity is incomplete for the unmarked callable-export branch

The recognized helper names at archived `extract.rs:4925–4943` return the argument shape unchanged. `collection_targets_from_call:4960–4966` forwards bare callable exports; `object_targets_from_call:5154–5161` forwards only their object shape. Thus `__importDefault(require(...)).default` lacks the default property when `module.exports = target` produces a bare callable export. Source inspection supports this mechanism; the focused fixture is recorded failing. This is not evidence that canonical module resolution itself failed.

The historical passing interop test (`old ts_value_flows.rs:11134–11173`) instead writes `__esModule=true` and `exports.default = function foo() {}`. Identity forwarding is appropriate for that represented shape. The archived failing fixture (`callable_flow/mod.rs:142–146`) writes `module.exports = target`; it exercises the wrapper-producing branch and is a different behavior. Preserve both positive fixtures and add shadowed helper names/unmarked object exports negatives. Name-only helper recognition is heuristic and should remain labeled accordingly.

### Unfinished cache/provenance obligations

The eight-file integration patch changes no cache-key recipes. HEAD `analysis_neutral/semantic_graph/cache_key.rs:49–70` lists model projection versions; `analysis_neutral/solver/cache_key.rs:35–48` includes `ts_points_to_projection_v1`. New collector semantics and ownership/projection changes require explicit deterministic invalidation, including relevant module resolution inputs. Existing `analysis_kernel/provider.rs:1706–1707` already lists resolved imports/module nodes at the relevant manifests, but implementation should verify the minimal requesting plan, not assume the full corpus request proves every lifecycle path.

Concrete cache work: add a `ts_callable_shape_projection_v1` parameter to semantic-graph digest and its parts-list test; bump `ts_points_to_projection_v1` for semantic variable/precision/ownership projection behavior; version TS MIR/call extraction if canonical owner identity changes; version refined projection only if its interpretation changes. Assert old/new parameter digests differ, then compare cold/warm semantic edge sets. Do not alter Go algorithm strings. Retain the current recovery exclusion and four module-summary rounds (`extract.rs:24,236`). Invocation walks have `invocation_depth > 16` guards (`541,4005,5130`), expression/shape recursion has `depth > 8` guards (e.g. `1225,1546,3691`), and property/key unions truncate at eight (`2564,2612,2821,8339`). Raising bounds to buy TP changes runtime/precision semantics and requires separate evidence, not an unreviewed gate workaround.

Target attribution must remain declaration-based: `function_for_expression:6697–6715` restricts AST variants to actual callable declarations/expressions. `function_for_span:6734–6742` first seeks exact endpoints but falls back to the first same-start fact. That fallback and `function_for_class_method:6718–6726`'s containing-span fallback are concrete audit targets for nested/returned class identity. Add negatives where a non-callable invocation result shares its start with a callable expression, and where inner and outer callables overlap. Do not treat an arbitrary enclosing function or same-name declaration as a proved target.

The old collector distinguished speculative receiver walks with `ThisMethodFlow`; the archive retains `Receiver` versus `Token` in model identity (`extract.rs:472`). Verify reachability/provenance consumers still receive the distinction that matters. A string embedded in a constraint key does not itself prove equivalent reachability behavior. This is a risk lead, not a quantified FP explanation.

## Recommendation and bounded estimates

These are engineering estimates for an experienced crew with build/corpus assets available, not measured implementation durations. Compilation/resource contention is additional contingency.

| Path | Work estimate | F1 ceiling assessment | Risk |
| --- | --- | --- | --- |
| (a) Continue archived private port | 10–16 engineer-hours; about 8–12 elapsed hours with two people splitting integration/fixtures and edge triage | 0.765784 is measured for an earlier archived state; 0.785193–0.80 is a plausible immediate target, not a predicted result; empirical ceiling unknown | Best starting evidence; ownership fix crosses several contracts and latest archive was not corpus-run |
| (b) Fresh historical collector port | 16–28 engineer-hours | No demonstrated ceiling advantage over (a): 97% of old production lines already survive there; historical 0.790193 baseline is not proof this isolated collector alone produces it | Duplicates adaptation effort and risks resurrecting obsolete resolver/direct provider/heap solver |
| (c) Replace models with fully generic shared-solver constraints now | 40–80+ engineer-hours for equivalent model inventory and validation | Unknown; extensibility advantage does not establish an F1 gain | Too large for tonight: collection ordering, promises, callable/object products and ownership must all be designed and tested |
| (c) Staged hybrid: archived private shape extraction + canonical shared solver, gradually lower reusable shapes into native constraints after recovery | Same initial recovery envelope as (a), then separate refactor | Same immediate ceiling as (a); no promised extra TP | Recommended architectural direction; keep one edge-producing solver and private model boundary |

Choose (a) as the first implementation, following the staged hybrid boundary. A fresh historical restoration adds little reusable behavior because the models are already present. Reserve 2–4 contingency hours if corrected ownership reveals more frontier gaps than its bucket suggests. Stop claiming tonight is sufficient if measured recoverable edges do not cover the bar.

At N=1,479 oracle edges, FP=36, the threshold requires TP>=980 (+40 over 940). At fixed measured precision 940/976 it requires TP>=981 (+41). +45 TP is a useful initial cushion, not the exact minimum. An unverified single fix must not be budgeted as enough without corpus edge attribution.

## First executable work breakdown

1. On the implementation branch, reconstruct only the archived collector/integration hunks against HEAD; preserve current narrow visibility and recent source changes. Record archive SHA-256s and baseline/tolerance hashes. Run the original corpus gate immediately, because the archive includes a later unverified identifier repair. Capture per-case reports as the actual starting point.
2. Add a TS constructor-ownership fixture that checks caller and target together through the real kernel. Include class declaration/expression, `super(callback)`, static method/block, field initializer, nested arrow and ordinary method controls. Repair the earliest canonical owner boundary in `ts/mir/lower.rs` / callsite extraction; keep solver/refined owner equality validation. Independently diagnose CJS object/callable shape at `extract.rs:4925–4966,5150–5161` with both interop branches.
3. Restore the highest-value historical behavior tests as real-kernel/refined-edge fixtures, verify model provenance and cache invalidation, then run unchanged debug and release corpus gates. Compare TP/FP/FN edge sets per case with the reconstructed starting point. Select subsequent fixes from measured deficits, not model names or total F1 alone.

The remaining historical 67-test suite provides specific fixture sources: constructors/super/private members (`11175`), returned classes (`11241`), class shadowing/non-overproduction (`11350`), destructuring (`11413`), default-not-applied-to-present-unknown (`11528`), delayed closure invocation (`11562`), promise allSettled (`10859`), async generators (`10917`), receiver side effects (`10992`), and cross-file CJS/ESM (`11025`, `11081`). Adapt assertions to refined edges and canonical caller identity; old low-level fabricated `AnalysisDb`/`CallTargetFact` fixtures are not drop-in tests for today's pipeline.

## Open questions

- How many current port FNs are solely owner mismatch, versus missing targets or unrelated reachability? The per-case audit must bound this before committing to a one-night outcome.
- How much FP growth is model imprecision versus lost receiver provenance or duplicate direct/derived edges? The 8→36 aggregate change cannot answer that alone.
- How much memory does keeping all Oxc arenas alive through four summary rounds add on a large checkout? Parse sharing avoids repeated parsing but does not establish a peak-memory improvement.
- Does the latest archive preserve the measured 0.765784 on the exact gate? This research deliberately does not assign a corpus score to unverified code.
