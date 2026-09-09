# FN taxonomy and remaining recovery budget

Agent / angle: per-case and edge-level Jelly accuracy audit.  
Question: which measured cases account for HEAD's recall collapse, what did the rejected port recover, and what remains worth fixing tonight?  
Date: 2026-09-08. Status: reviewed by arithmetic reconciliation; implementation causality remains explicitly bounded.

## Protocol and sources

Compare the existing 76-case, 1,479-edge release-tier reports without rerunning or changing the gate. Use `(case_id, graph, from-span, to-span)` as identity, independently reconstruct TP/FP/FN using set operations, then inspect pinned fixture source at missing spans. Include both `fun2fun` and `call2fun`; they are separately scored assertions, not counts of distinct calls. Do not interpret case names or the gate's unknown counters as causal model ablations.

Read the preserved `.context/perf-research/accuracy-restoration-RESEARCH.md` and `FINAL-REPORT.md` first. Raw primary evidence copied read-only from `/workspace/polint-perf-research/.context/perf-research/`:

| Report directory | Role | TP / FP / FN |
| --- | --- | ---: |
| `baseline-gate` | HEAD-equivalent pre-port debug | 249 / 8 / 1230 |
| `baseline-release-gate` | Debug/release cross-check | 249 / 8 / 1230 |
| `post-matrix-gate` | Final pure-performance equivalent | 249 / 8 / 1230 |
| `accuracy-port-gate-2` | Measured rejected port | 940 / 36 / 539 |
| `night2-accuracy-initial` | Measured port reproduction | 940 / 36 / 539 |

Each source file is `reports/jelly-callgraph-micro-baseline.json`. Exact source paths and SHA-256 hashes are in [inputs.json](../../.context/accuracy-research/taxonomy/inputs.json). Expected and observed graph-edge lists are identical among all three non-port reports and between both port reports; runtime/other report metadata is not claimed identical. See [validation.json](../../.context/accuracy-research/taxonomy/validation.json).

Corpus inspection used the **original sibling checkout named in the reports**, `/workspace/polint-perf-research/research/evaluation-harness/repos/jelly`, commit `b799ed4f0d68c670fe398830aaa51dd5c628cf74`, source URL <https://github.com/cs-au-dk/jelly>, BSD-3-Clause. [corpus-sources.json](../../.context/accuracy-research/taxonomy/corpus-sources.json) records existence and hashes for all 131 source paths referenced by either report. No dependency installation or corpus edits occurred.

Reproduction: `python .context/accuracy-research/scratch/taxonomy.py`. This is a research-only data analysis script; it never edits product code or baseline inputs. Outputs: [family-summary.json](../../.context/accuracy-research/taxonomy/family-summary.json), [cases.csv](../../.context/accuracy-research/taxonomy/cases.csv), and [edges.json](../../.context/accuracy-research/taxonomy/edges.json). The edge file includes graph identity, expected/head-observed/port-observed booleans, source existence and exact surrounding fixture lines.

## High-confidence findings

The rejected port recovers **691 expected edges and loses none of HEAD's 249 TP**. It removes 2 old FP, preserves 6, and introduces 30 new FP: **28 net additional FP** (8 to 36). This establishes broad missing modeled-flow coverage at HEAD; it does not isolate the marginal contribution of each model because many models and integration fixes changed together. Unknowns remain exactly 913 in both reports despite 691 recovered edges, and all `categorized_failures` counters remain zero. Neither is a usable FN taxonomy.

The mutually exclusive table below assigns each complete case to one fixture family. It sums exactly, but **is not an additive causal attribution**. For example, an array callback fixture also needs higher-order parameter flow; a class fixture can fail because of caller identity rather than class-value propagation; an interop fixture first needs to recognize the helper function itself.

| Fixture family | Cases | Expected | HEAD FN | Port FN | TP recovered | Port FP |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Missing dependency source: helloworld | 1 | 342 | 342 | 342 | 0 | 2 |
| Classes / `this` / prototypes | 15 | 313 | 225 | 29 | 196 | 12 |
| Collections / array builtins | 8 | 129 | 123 | 36 | 87 | 4 |
| Dynamic properties / objects / accessors / other | 18 | 142 | 111 | 28 | 83 | 2 |
| Iterators / generators | 3 | 119 | 103 | 6 | 97 | 4 |
| Destructuring / rest / spread | 5 | 117 | 101 | 22 | 79 | 9 |
| Promises / async | 4 | 96 | 90 | 8 | 82 | 2 |
| Cross-module / callbacks / interop | 14 | 84 | 64 | 44 | 20 | 0 |
| Higher-order calls / defaults / arguments | 7 | 119 | 57 | 12 | 45 | 1 |
| Source-location stress | 1 | 18 | 14 | 12 | 2 | 0 |
| **Total** | **76** | **1479** | **1230** | **539** | **691** | **36** |

### 342 residual edges cannot be modeled from absent source

All 342 expected edges in `tests/helloworld/app.json` target `tests/helloworld/node_modules/*`; all remain FN in both runs. The original report checkout currently lacks these referenced dependency files (42 absent edge-referenced source paths overall). Only eight edges originate in `app.js`; 334 originate within dependencies. Every other residual FN in the entire corpus targets a present source file.

The adapter's `prepare_case` in `crates/polint-eval/src/harness/external/jelly_callgraph.rs:70` only selects graph-listed files if `path.is_file()` and the extension is supported. Missing dependencies are thus excluded from analysis inputs while their oracle edges remain expected. This is a source-availability limit, not evidence that 342 CommonJS or collection edges can be won by one language-model change. Recorded manifests plus today's checked-out source support this interpretation; no historical dependency-tree snapshot independently proves the exact filesystem at report runtime.

Do not remove these cases, weaken the scorer, install a different dependency tree to claim an unchanged-input win, or edit expected edges. Tonight can clear the bar using present sources: the port has **197 in-scope FN**, of which 107 are call2fun and 90 fun2fun. With the same source universe, all 1,137 in-scope expected edges recovered and zero FP would yield a theoretical ceiling **0.869266**; retaining all 36 FP yields **0.857466**. These are arithmetic upper bounds, not a predicted achievable score.

The port's two helloworld FP are a concrete precision trap: `app.js:7:5:7:33` (`console.log("Response sent")`) points to its own enclosing request callback `app.js:5:14:9:2`, with the corresponding callback self-edge. (The first rough inspection mistook this line for `res.send`; source-span inspection corrects it.)

## Residual fixture opportunities

These rows describe actual missing edges and source constructs. Their counts are **maximum available assertions in the named slice**, not validated gains or probabilities. They should become focused regressions before changes. Distinct rows below avoid overlapping cases except where explicitly splitting one case; interactions can still alter precision elsewhere.

| Fixture / slice | Port FN pool | Exact source evidence and implementation question |
| --- | ---: | --- |
| `micro/arrays3.js` | 6 | Three reduce-result calls at lines 2, 9, 11. Array elements and explicit initial accumulator can be callable even when reducer returns `void 0`. Model accumulator fallback; do not invent arbitrary callable returns. |
| `micro/arrays4.js` | 4 | Both `doit(f => f())` callbacks call the element at line 3. `forEach(f)` followed by `f = undefined` tests mutation and callback-argument flow across an enclosing function. |
| `micro/arrays5.js` | 8 | `weirdArray.forEach = weirdArray.sort`; callback parameters invoked at lines 10, 12, 13 lack element targets. Requires method-value aliases and callback argument roles, not merely recognizing a property name. |
| `micro/arrays2.js` | 5 | `map` callback's `array[2]()` and `this.p()` (lines 14-15), plus `z()` after reading mapped element 1 (line 20). Separate whole-array callback argument, `thisArg`, and result-index channels. |
| `micro/arrays.js` | 4 | Array hole and unknown-key writes; calls `y()`/`z()` at lines 5/7. |
| `micro/more1.js` | 3 | Spread-array index calls at lines 56-58; all three are call2fun misses with target fun2fun edges already present. |
| `approx/natives.js`, `micro/mix.js` | 6 | Array nested property/element reads at natives lines 96/101; `Map` set/get callable at mix line 6. |
| `micro/client4.js` + `client5.js`: helper invocations | 6 | Four call2fun and two module-to-helper fun2fun edges to `__importDefault`, initialized as `(this && this.__importDefault) || function(mod){...}`. Restore logical-expression callable alternatives first; do not label these six as import resolution failures. |
| `micro/client4.js`: default class + method | 4 | `new timer_1.default()` line 7 and `timer.elapsed()` line 9 target `lib4.js`; tests wrapper object/default interop and imported constructor instance shape. |
| `micro/dyn-import.mjs`, `import12.mjs` | 6 | Dynamic import default/named calls (4) and package conditional export `node-exports/index.mjs` (2). Resolver lifecycle and module summaries may both matter. |
| `micro/client-this`, `client1`, `client9`, two `mochatest` cases | 18 | Cross-file callback invocation and callable-returned closure targets. Includes library callback `f(a)`, filter returned closure, exported `apply(f,x,y)` invoking exported `plus`, and pirates require-hook callback. |
| `micro/default-parameter.js` | 4 | Default function parameter called inside `f`, and default initializer `f()` inside `g`; both value propagation and ownership of initializer calls matter. |
| `micro/arguments.js` | 6 | `arguments[0]` write aliases parameter, `arguments.callee`, and function returning itself. These are separate semantic features. |
| `micro/fun.js` | 2 | Parenthesized outer call `(baz3(callback)())` at line 24; candidate flow or span-identity issue. |
| `micro/destructuring.js` | 3 | `q()` line 21 and `d.baz()` line 38; 2 call2fun + 1 fun2fun. |
| `approx/deconstruction.js` | 4 | Actual remaining misses are computed-key writes `a[p] = function(){}` followed by `a.p()` at lines 28/37, rather than generic destructuring failure. |
| `micro/rest.js` | 7 | Unknown index and computed property writes flowing through rest; 4 call2fun + 3 fun2fun. |
| `micro/spread.js` | 8 | Calls at lines 3,4,16,17,26-29; **all eight are call2fun** while their expected fun2fun targets are already observed. Check callsite/value attribution before assuming all are missing spread models. |
| `micro/generators.js` | 6 | Value sent through `next(callback)` and generator return values read via `next().value()` (lines 31,68,74). `micro/iterators` itself has **zero FN**, so broad iterator reimplementation is poor first priority. |
| `micro/promises2.js` | 4 | Promise resolve callback flows via helper then await (`f1()`, `f2()`); plus parenthesized async IIFE call span. |
| `micro/asyncawait.js`, `promiseall.js` | 4 | Async-IIFE and async-generator call spans (2), nested `Promise.all` result→forEach→callback (2). `micro/promises` itself has **zero FN**. |
| `approx/srcLoc.js` | 12 | Parenthesized creators/returns and dynamic method names. The case deliberately stresses instrumentation source locations; edge absence alone does not distinguish lost value flow from unavailable/mismatched identity records. |

Remaining rows in the complete census include 29 class/this/prototype FN (the root researcher separately audits constructor caller identity), dynamic object/getter aliases, and source callback targets. In particular `client2` prototype method (2), `client3` sequence-expression CommonJS call (2), and `approx/library` computed export/callback properties (6) account for the remaining 10 cross-module FN outside the explicit slices above.

## Gate math and prioritization

With 1,479 expected edges, `F1 = 2*TP / (1479 + TP + FP)`. At the current FP=36, the unchanged 0.785193 bar requires TP=980: **40 additional TP**, yielding 0.785571. At FP=40 it requires +42; at FP=45, +46; at FP=50, +49. At constant current precision the increment is approximately 41 TP; the user's rough +45 estimate is a reasonable safety allowance but should not be the acceptance calculation.

Prefer an audited continuation of the port, then a bounded portfolio of focused fixes: constructor caller identity (separate root analysis), logical helper initializers + CommonJS wrapper shape, array reduce/callback result flow, defaults, and residual spread/destructuring identity/value handling. Avoid spending tonight rewriting broad Promise/iterator models whose dedicated fixtures already recovered 56/56 and 65/65 respectively. Budget for a **50–60 expected-edge opportunity pool** rather than betting that the first 40 candidates all materialize without FP.

No additive causal gain can be certified without implementation-level ablation runs. The requested family taxonomy is measured; suggested model→fixture mapping is a source-grounded hypothesis. Maintain exact old TP containment and inspect every new FP after each cohort, then rerun the original full gate in both profiles. Do not use focused green tests as a substitute for the final corpus score.

## Complete case census

`H FN` = HEAD-equivalent FN; `P FN` = measured rejected-port FN. Every row has zero lost HEAD TP.

| Case (under `tests/`, `.json`) | Family | Expected | H FN | P FN | Recovered | P FP |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `approx/computedProperties` | dynamic/object/accessor/other | 26 | 22 | 0 | 22 | 1 |
| `approx/deconstruction` | destructuring/rest/spread | 18 | 12 | 4 | 8 | 0 |
| `approx/dynamic` | dynamic/object/accessor/other | 8 | 8 | 6 | 2 | 0 |
| `approx/function` | dynamic/object/accessor/other | 2 | 2 | 2 | 0 | 0 |
| `approx/library` | cross-module/callbacks/interop | 10 | 10 | 6 | 4 | 0 |
| `approx/natives` | collections/array builtins | 33 | 33 | 4 | 29 | 1 |
| `approx/simple` | dynamic/object/accessor/other | 37 | 24 | 6 | 18 | 0 |
| `approx/srcLoc` | source-location stress | 18 | 14 | 12 | 2 | 0 |
| `approx/this` | classes/this/prototypes | 32 | 20 | 9 | 11 | 1 |
| `helloworld/app` | absent dependency source (helloworld) | 342 | 342 | 342 | 0 | 2 |
| `micro/accessors` | dynamic/object/accessor/other | 2 | 2 | 0 | 2 | 0 |
| `micro/accessors3` | dynamic/object/accessor/other | 4 | 4 | 1 | 3 | 1 |
| `micro/accessors5` | dynamic/object/accessor/other | 2 | 2 | 0 | 2 | 0 |
| `micro/accessors6` | dynamic/object/accessor/other | 2 | 2 | 2 | 0 | 0 |
| `micro/arguments` | higher-order/call/default/arguments | 16 | 12 | 6 | 6 | 1 |
| `micro/arrays` | collections/array builtins | 6 | 6 | 4 | 2 | 0 |
| `micro/arrays2` | collections/array builtins | 11 | 11 | 5 | 6 | 2 |
| `micro/arrays3` | collections/array builtins | 6 | 6 | 6 | 0 | 0 |
| `micro/arrays4` | collections/array builtins | 7 | 4 | 4 | 0 | 0 |
| `micro/arrays5` | collections/array builtins | 11 | 8 | 8 | 0 | 0 |
| `micro/assign1` | dynamic/object/accessor/other | 6 | 6 | 0 | 6 | 0 |
| `micro/assign2` | dynamic/object/accessor/other | 2 | 2 | 2 | 0 | 0 |
| `micro/asyncawait` | promises/async | 29 | 28 | 2 | 26 | 1 |
| `micro/bind` | higher-order/call/default/arguments | 3 | 3 | 0 | 3 | 0 |
| `micro/call-expressions` | higher-order/call/default/arguments | 45 | 0 | 0 | 0 | 0 |
| `micro/call` | higher-order/call/default/arguments | 2 | 2 | 0 | 2 | 0 |
| `micro/classes` | classes/this/prototypes | 77 | 46 | 7 | 39 | 3 |
| `micro/classes2` | classes/this/prototypes | 76 | 52 | 0 | 52 | 0 |
| `micro/classes3` | classes/this/prototypes | 4 | 2 | 2 | 0 | 0 |
| `micro/client-this` | cross-module/callbacks/interop | 9 | 6 | 6 | 0 | 0 |
| `micro/client1` | cross-module/callbacks/interop | 6 | 6 | 4 | 2 | 0 |
| `micro/client2` | cross-module/callbacks/interop | 4 | 2 | 2 | 0 | 0 |
| `micro/client3` | cross-module/callbacks/interop | 2 | 2 | 2 | 0 | 0 |
| `micro/client4` | cross-module/callbacks/interop | 7 | 7 | 7 | 0 | 0 |
| `micro/client5` | cross-module/callbacks/interop | 7 | 7 | 3 | 4 | 0 |
| `micro/client9` | cross-module/callbacks/interop | 4 | 2 | 2 | 0 | 0 |
| `micro/create` | dynamic/object/accessor/other | 4 | 4 | 0 | 4 | 0 |
| `micro/default-parameter` | higher-order/call/default/arguments | 6 | 4 | 4 | 0 | 0 |
| `micro/defineProperties` | dynamic/object/accessor/other | 3 | 3 | 0 | 3 | 0 |
| `micro/defineProperty` | dynamic/object/accessor/other | 15 | 9 | 0 | 9 | 0 |
| `micro/destructuring` | destructuring/rest/spread | 21 | 21 | 3 | 18 | 0 |
| `micro/dpr-this` | dynamic/object/accessor/other | 8 | 6 | 2 | 4 | 0 |
| `micro/dyn-import` | cross-module/callbacks/interop | 4 | 4 | 4 | 0 | 0 |
| `micro/for-in` | iterators/generators | 4 | 4 | 0 | 4 | 0 |
| `micro/fun` | higher-order/call/default/arguments | 45 | 36 | 2 | 34 | 0 |
| `micro/generators` | iterators/generators | 50 | 34 | 6 | 28 | 0 |
| `micro/import1` | cross-module/callbacks/interop | 10 | 2 | 0 | 2 | 0 |
| `micro/import12` | cross-module/callbacks/interop | 2 | 2 | 2 | 0 | 0 |
| `micro/iterators` | iterators/generators | 65 | 65 | 0 | 65 | 4 |
| `micro/mix` | collections/array builtins | 6 | 6 | 2 | 4 | 0 |
| `micro/more1` | collections/array builtins | 49 | 49 | 3 | 46 | 1 |
| `micro/obj` | dynamic/object/accessor/other | 2 | 2 | 0 | 2 | 0 |
| `micro/obj2` | dynamic/object/accessor/other | 7 | 5 | 5 | 0 | 0 |
| `micro/oneshot` | higher-order/call/default/arguments | 2 | 0 | 0 | 0 | 0 |
| `micro/private` | classes/this/prototypes | 12 | 10 | 6 | 4 | 5 |
| `micro/promiseall` | promises/async | 2 | 2 | 2 | 0 | 0 |
| `micro/promises` | promises/async | 56 | 56 | 0 | 56 | 0 |
| `micro/promises2` | promises/async | 9 | 4 | 4 | 0 | 1 |
| `micro/prototypes` | classes/this/prototypes | 4 | 2 | 0 | 2 | 0 |
| `micro/prototypes2` | classes/this/prototypes | 4 | 2 | 0 | 2 | 0 |
| `micro/prototypes3` | classes/this/prototypes | 8 | 6 | 2 | 4 | 0 |
| `micro/receiver-callee-mixup` | classes/this/prototypes | 8 | 8 | 0 | 8 | 0 |
| `micro/rest` | destructuring/rest/spread | 44 | 38 | 7 | 31 | 7 |
| `micro/rest2` | destructuring/rest/spread | 4 | 2 | 0 | 2 | 0 |
| `micro/spawn-cwd` | cross-module/callbacks/interop | 2 | 2 | 0 | 2 | 0 |
| `micro/spread` | destructuring/rest/spread | 30 | 28 | 8 | 20 | 2 |
| `micro/super` | classes/this/prototypes | 26 | 24 | 2 | 22 | 2 |
| `micro/super2` | classes/this/prototypes | 10 | 8 | 0 | 8 | 0 |
| `micro/super3` | classes/this/prototypes | 8 | 6 | 0 | 6 | 0 |
| `micro/super4` | classes/this/prototypes | 18 | 16 | 0 | 16 | 0 |
| `micro/super5` | classes/this/prototypes | 12 | 9 | 1 | 8 | 1 |
| `micro/templateliterals` | dynamic/object/accessor/other | 8 | 6 | 0 | 6 | 0 |
| `micro/this` | classes/this/prototypes | 14 | 14 | 0 | 14 | 0 |
| `micro/throw` | dynamic/object/accessor/other | 4 | 2 | 2 | 0 | 0 |
| `mochatest/test-with-hook` | cross-module/callbacks/interop | 11 | 6 | 4 | 2 | 0 |
| `mochatest/test` | cross-module/callbacks/interop | 6 | 6 | 2 | 4 | 0 |
