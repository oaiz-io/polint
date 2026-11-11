# JS/TS analysis benchmark

polint 0.5.0. Written by `python3 tests/js-analysis-bench/bench.py report`;
see `tests/js-analysis-bench/README.md` for the measures, the decisions and their limits.
There is no overall accuracy number on purpose: read the rows.

- **base** is what a rule gets from `CallGraph` (capabilities `syntax, call_graph, symbols, references`).
- **refined** is the refined call-edge layer (adds `calls, control_flow, dataflow`). It has no public view;
  it is read through the internal benchmark dump only.
- Precision is measured against hand labels only. **Observed calls give recall, never precision**:
  a run sees the paths it exercised, so an edge polint reports that the run did not take is not wrong.

## Constructs (hand labels)

Counts are summed over each construct's cases. `held out` cases must not be used to tune analysis;
compare their rows with the other cases to see whether a change generalises.

| construct | cases (held out) | call recall base | call recall refined | callback recall base | callback recall refined | precision base | precision refined | hidden paths base: reported / false-complete / silent | hidden paths refined | stops at import binding (base) |
|---|---|---|---|---|---|---|---|---|---|---|
| callback | 2 (0) | 3/7 (43%) | 5/7 (71%) | 0/3 (0%) | 0/3 (0%) | 3/3 (100%) | 5/5 (100%) | – | – | 0 |
| cjs-exports-property | 2 (0) | 0/4 (0%) | 3/4 (75%) | – | – | – | 3/3 (100%) | – | – | 0 |
| cjs-module-exports | 2 (1) | 1/6 (17%) | 5/6 (83%) | – | – | 1/1 (100%) | 5/5 (100%) | – | – | 0 |
| class-method | 2 (1) | 3/9 (33%) | 6/9 (67%) | – | – | 3/3 (100%) | 6/6 (100%) | – | – | 0 |
| closure | 2 (0) | 3/7 (43%) | 6/7 (86%) | – | – | 3/3 (100%) | 6/6 (100%) | – | – | 0 |
| dynamic-import | 2 (0) | 1/2 (50%) | 1/2 (50%) | – | – | 1/1 (100%) | 1/1 (100%) | 1 / 0 / 0 | 1 / 0 / 0 | 0 |
| dynamic-property | 2 (1) | 0/2 (0%) | 0/2 (0%) | – | – | – | – | 1 / 0 / 0 | 1 / 0 / 0 | 0 |
| esm-reexport | 2 (1) | 0/4 (0%) | 4/4 (100%) | – | – | – | 4/4 (100%) | – | – | 3 |
| eval | 2 (1) | – | – | – | – | – | – | 2 / 0 / 0 | 2 / 0 / 0 | 0 |
| object-of-functions | 2 (0) | 1/5 (20%) | 5/5 (100%) | – | – | 1/1 (100%) | 5/5 (100%) | – | – | 0 |
| promise | 2 (1) | 3/4 (75%) | 3/4 (75%) | 0/1 (0%) | 0/1 (0%) | 3/3 (100%) | 3/3 (100%) | – | – | 0 |
| prototype | 2 (1) | 2/7 (29%) | 4/7 (57%) | – | – | 2/2 (100%) | 4/4 (100%) | – | – | 0 |

**False-complete count** (a labelled hidden path where polint gives a resolved answer with no unresolved marker): base 0, refined 0, over 4 labelled hidden paths.

### Per case

| case | held out | calls base | calls refined | callbacks base | callbacks refined | precision base | precision refined | hidden paths base | hidden paths refined | unresolved call share base | labels = observed |
|---|---|---|---|---|---|---|---|---|---|---|---|
| callback/builtin-invoked | no | 1/1 (100%) | 1/1 (100%) | 0/3 (0%) | 0/3 (0%) | 1/1 (100%) | 1/1 (100%) | – | – | 75% | yes |
| callback/higher-order-parameter | no | 2/6 (33%) | 4/6 (67%) | – | – | 2/2 (100%) | 4/4 (100%) | – | – | 71% | yes |
| cjs-exports-property/function-expressions | no | 0/2 (0%) | 1/2 (50%) | – | – | – | 1/1 (100%) | – | – | 100% | yes |
| cjs-exports-property/renamed-destructuring | no | 0/2 (0%) | 2/2 (100%) | – | – | – | 2/2 (100%) | – | – | 100% | yes |
| cjs-module-exports/function-with-property | yes | 0/3 (0%) | 2/3 (67%) | – | – | – | 2/2 (100%) | – | – | 100% | yes |
| cjs-module-exports/object-literal | no | 1/3 (33%) | 3/3 (100%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 80% | yes |
| class-method/instance-and-static | no | 2/5 (40%) | 5/5 (100%) | – | – | 2/2 (100%) | 5/5 (100%) | – | – | 67% | yes |
| class-method/subclass-override | yes | 1/4 (25%) | 1/4 (25%) | – | – | 1/1 (100%) | 1/1 (100%) | – | – | 80% | yes |
| closure/captured-callback | no | 1/4 (25%) | 3/4 (75%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 75% | yes |
| closure/factory-returned-function | no | 2/3 (67%) | 3/3 (100%) | – | – | 2/2 (100%) | 3/3 (100%) | – | – | 25% | yes |
| dynamic-import/computed-specifier | no | – | – | – | – | – | – | reported-unknown | reported-unknown | 100% | yes |
| dynamic-import/literal-specifier | no | 1/2 (50%) | 1/2 (50%) | – | – | 1/1 (100%) | 1/1 (100%) | – | – | 50% | yes |
| dynamic-property/computed-key-known-set | no | 0/2 (0%) | 0/2 (0%) | – | – | – | – | – | – | 100% | yes |
| dynamic-property/key-from-input | yes | – | – | – | – | – | – | reported-unknown | reported-unknown | 100% | yes |
| esm-reexport/named-barrel | no | 0/2 (0%) | 2/2 (100%) | – | – | – | 2/2 (100%) | – | – | 50% | yes |
| esm-reexport/star-and-alias | yes | 0/2 (0%) | 2/2 (100%) | – | – | – | 2/2 (100%) | – | – | 50% | yes |
| eval/direct-eval | no | – | – | – | – | – | – | reported-unknown | reported-unknown | 100% | yes |
| eval/function-constructor | yes | – | – | – | – | – | – | reported-unknown | reported-unknown | 100% | yes |
| object-of-functions/literal-methods | no | 0/2 (0%) | 2/2 (100%) | – | – | – | 2/2 (100%) | – | – | 100% | yes |
| object-of-functions/registry-built-later | no | 1/3 (33%) | 3/3 (100%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 67% | yes |
| promise/async-await | yes | 2/3 (67%) | 2/3 (67%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 33% | yes |
| promise/then-chain | no | 1/1 (100%) | 1/1 (100%) | 0/1 (0%) | 0/1 (0%) | 1/1 (100%) | 1/1 (100%) | – | – | 60% | no |
| prototype/constructor-function | no | 1/3 (33%) | 3/3 (100%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 67% | yes |
| prototype/inherited-method | yes | 1/4 (25%) | 1/4 (25%) | – | – | 1/1 (100%) | 1/1 (100%) | – | – | 60% | yes |

## Packages (observed calls: recall only)

Recall is over observed edges at call expressions (`call` and `callback`); `implicit` calls (getters,
coercions, iterator and thenable protocol) have no call expression and are listed by kind only.

| package | format | files analysed / selected | source KB | observed edges | recall base | recall refined | stops at import binding (base) | not mappable to a polint site or function | unresolved call share base | call sites without target base / refined | unresolved reference share |
|---|---|---|---|---|---|---|---|---|---|---|---|
| axios 1.21.1 | ESM source, Node and browser adapters | 72 / 72 | 282 | 675 | 86/675 (13%) | 86/675 (13%) | 40 | 153 | 77% | 90% / 90% | 9% |
| zod 4.6.7 | TypeScript ESM source | 332 / 332 | 3088 | 1308 | 100/1308 (8%) | timed out after 600 s | 17 | 799 | 69% | 96% / n/a | 7% |
| commander 14.0.3 | CommonJS only | 7 / 7 | 127 | 318 | 14/318 (4%) | 117/318 (37%) | 0 | 1 | 94% | 98% / 78% | 10% |
| async 3.2.6 | CommonJS transpiled from ESM | 129 / 129 | 321 | 637 | 319/637 (50%) | 323/637 (51%) | 0 | 1 | 63% | 73% / 73% | 27% |
| chalk 6.0.1 | ESM only, with subpath imports | 2 / 5 | 10 | 72 | 1/72 (1%) | 4/72 (6%) | 2 | 48 | 76% | 98% / 93% | 9% |
| dayjs 1.11.23 | minified UMD | 1 / 1 | 7 | 117 | 0/117 (0%) | 0/117 (0%) | 0 | 117 | 100% | 100% / 100% | 7% |
| underscore 1.13.8 | bundled UMD (Rollup) | 1 / 1 | 69 | 262 | 156/262 (60%) | 157/262 (60%) | 0 | 0 | 55% | 58% / 58% | 6% |

### Package recall by observed call kind and syntax

| package | kind / syntax | observed | base | refined |
|---|---|---|---|---|
| axios | kind: call | 632 | 86/632 (14%) | 86/632 (14%) |
| axios | kind: callback | 43 | 0/43 (0%) | 0/43 (0%) |
| axios | kind: implicit | 6 | 0/6 (0%) | 0/6 (0%) |
| axios | syntax: computed-member | 2 | 0/2 (0%) | 0/2 (0%) |
| axios | syntax: expression | 8 | 5/8 (62%) | 5/8 (62%) |
| axios | syntax: identifier | 317 | 75/317 (24%) | 75/317 (24%) |
| axios | syntax: member | 326 | 1/326 (0%) | 1/326 (0%) |
| axios | syntax: new | 21 | 5/21 (24%) | 5/21 (24%) |
| axios | syntax: super | 1 | 0/1 (0%) | 0/1 (0%) |
| zod | kind: call | 1265 | 100/1265 (8%) | n/a |
| zod | kind: callback | 43 | 0/43 (0%) | n/a |
| zod | kind: implicit | 18 | 0/18 (0%) | n/a |
| zod | syntax: computed-member | 5 | 0/5 (0%) | n/a |
| zod | syntax: expression | 1 | 0/1 (0%) | n/a |
| zod | syntax: identifier | 408 | 99/408 (24%) | n/a |
| zod | syntax: member | 829 | 0/829 (0%) | n/a |
| zod | syntax: new | 65 | 1/65 (2%) | n/a |
| commander | kind: call | 264 | 14/264 (5%) | 117/264 (44%) |
| commander | kind: callback | 54 | 0/54 (0%) | 0/54 (0%) |
| commander | syntax: identifier | 35 | 13/35 (37%) | 20/35 (57%) |
| commander | syntax: member | 274 | 0/274 (0%) | 90/274 (33%) |
| commander | syntax: new | 8 | 1/8 (12%) | 7/8 (88%) |
| commander | syntax: super | 1 | 0/1 (0%) | 0/1 (0%) |
| async | kind: call | 622 | 319/622 (51%) | 323/622 (52%) |
| async | kind: callback | 15 | 0/15 (0%) | 0/15 (0%) |
| async | kind: implicit | 4 | 0/4 (0%) | 0/4 (0%) |
| async | syntax: expression | 157 | 0/157 (0%) | 0/157 (0%) |
| async | syntax: identifier | 403 | 319/403 (79%) | 319/403 (79%) |
| async | syntax: member | 73 | 0/73 (0%) | 4/73 (5%) |
| async | syntax: new | 4 | 0/4 (0%) | 0/4 (0%) |
| chalk | kind: call | 71 | 1/71 (1%) | 4/71 (6%) |
| chalk | kind: callback | 1 | 0/1 (0%) | 0/1 (0%) |
| chalk | kind: implicit | 1 | 0/1 (0%) | 0/1 (0%) |
| chalk | syntax: computed-member | 10 | 0/10 (0%) | 0/10 (0%) |
| chalk | syntax: identifier | 43 | 1/43 (2%) | 4/43 (9%) |
| chalk | syntax: member | 19 | 0/19 (0%) | 0/19 (0%) |
| dayjs | kind: call | 115 | 0/115 (0%) | 0/115 (0%) |
| dayjs | kind: callback | 2 | 0/2 (0%) | 0/2 (0%) |
| dayjs | kind: implicit | 3 | 0/3 (0%) | 0/3 (0%) |
| dayjs | syntax: computed-member | 1 | 0/1 (0%) | 0/1 (0%) |
| dayjs | syntax: expression | 5 | 0/5 (0%) | 0/5 (0%) |
| dayjs | syntax: identifier | 35 | 0/35 (0%) | 0/35 (0%) |
| dayjs | syntax: member | 75 | 0/75 (0%) | 0/75 (0%) |
| dayjs | syntax: new | 1 | 0/1 (0%) | 0/1 (0%) |
| underscore | kind: call | 257 | 156/257 (61%) | 157/257 (61%) |
| underscore | kind: callback | 5 | 0/5 (0%) | 0/5 (0%) |
| underscore | syntax: expression | 2 | 2/2 (100%) | 2/2 (100%) |
| underscore | syntax: identifier | 234 | 153/234 (65%) | 154/234 (66%) |
| underscore | syntax: member | 25 | 0/25 (0%) | 0/25 (0%) |
| underscore | syntax: new | 1 | 1/1 (100%) | 1/1 (100%) |

### Why observed edges were missed (base / refined)

| package | callee not a polint function | found | no target, marked unresolved | no target, no unresolved marker | other targets, no unresolved marker | site not a polint call site | site or callee in a file polint does not analyse | stops at import binding |
|---|---|---|---|---|---|---|---|---|
| axios | 12 / 12 | 86 / 86 | 388 / 388 | 5 / 45 | 3 / 3 | 141 / 141 | 0 / 0 | 40 / 0 |
| zod | 6 / n/a | 100 / n/a | 391 / n/a | 1 / n/a | 0 / n/a | 793 / n/a | 0 / n/a | 17 / n/a |
| commander | 1 / 1 | 14 / 117 | 303 / 200 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| async | 1 / 1 | 319 / 323 | 317 / 313 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| chalk | 0 / 0 | 1 / 4 | 21 / 20 | 0 / 0 | 0 / 0 | 1 / 1 | 47 / 47 | 2 / 0 |
| dayjs | 1 / 1 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 116 / 116 | 0 / 0 | 0 / 0 |
| underscore | 0 / 0 | 156 / 157 | 106 / 102 | 0 / 0 | 0 / 3 | 0 / 0 | 0 / 0 | 0 / 0 |

## Exports (against the loaded module's export keys)

`Symbols` is the exported-symbol answer a rule can read; export facts are the module export rows.
Both are compared with the export keys the loaded module actually has.

| tree | files exact (Symbols) | name recall (Symbols) | name precision (Symbols) | files exact (export facts) | name recall (export facts) | name precision (export facts) | export fact rows by kind/status |
|---|---|---|---|---|---|---|---|
| package axios | 4/72 | 27/162 (17%) | 27/76 (36%) | 5/72 | 95/162 (59%) | 95/95 (100%) | Default/Resolved 67, Named/Resolved 28 |
| package zod | 10/122 | 940/3935 (24%) | 940/1739 (54%) | 15/122 | 1280/3935 (33%) | 1280/2042 (63%) | Default/Resolved 73, Named/Resolved 2090, Namespace/Unresolved 226, StarReexport/Unresolved 46 |
| package commander | 0/7 | 0/22 (0%) | – | 7/7 | 22/22 (100%) | 22/22 (100%) | CommonJsExportsProperty/Unsupported 22 |
| package async | 0/4 | 0/116 (0%) | – | 4/129 | 116/116 (100%) | 116/241 (48%) | CommonJsExportsProperty/Unsupported 347, CommonJsModuleExports/Unsupported 125 |
| package chalk | 1/5 | 4/29 (14%) | 4/7 (57%) | 1/5 | 16/29 (55%) | 16/16 (100%) | Default/Resolved 1, Named/Resolved 6, Namespace/Unresolved 9 |
| package dayjs | 0/1 | 0/7 (0%) | – | 0/1 | 0/7 (0%) | – | – |
| package underscore | 0/1 | 0/147 (0%) | – | 0/1 | 0/147 (0%) | – | – |
| case cjs-exports-property/function-expressions | 0/1 | 0/2 (0%) | – | 1/1 | 2/2 (100%) | 2/2 (100%) | CommonJsExportsProperty/Unsupported 2 |
| case cjs-exports-property/renamed-destructuring | 0/1 | 0/2 (0%) | – | 1/1 | 2/2 (100%) | 2/2 (100%) | CommonJsExportsProperty/Unsupported 2 |
| case cjs-module-exports/function-with-property | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 |
| case cjs-module-exports/object-literal | 0/1 | 0/2 (0%) | – | 0/1 | 0/2 (0%) | – | CommonJsModuleExports/Unsupported 1 |
| case class-method/subclass-override | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 |
| case dynamic-import/computed-specifier | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 |
| case dynamic-import/literal-specifier | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 |
| case esm-reexport/named-barrel | 2/3 | 2/4 (50%) | 2/2 (100%) | 3/3 | 4/4 (100%) | 4/4 (100%) | Named/Resolved 2, Namespace/Unresolved 2 |
| case esm-reexport/star-and-alias | 2/3 | 2/4 (50%) | 2/2 (100%) | 2/3 | 3/4 (75%) | 3/3 (100%) | Named/Resolved 2, Namespace/Unresolved 1, StarReexport/Unresolved 1 |

## Cost

The only part of this report that changes between runs. Machine: Linux-6.8.0-147-generic-x86_64-with-glibc2.39, 32 CPUs, commit 338b570b5914.

`polint check` (release) with the census rule pack, which asks for `Functions`, `CallGraph`, `Symbols`, `References`; median of 3 runs. `analysis` is the in-process cost record (`POLINT_GOLDEN_COST_PATH`), `process` is the whole CLI as `bench_matrix.measure` sees it. Cold has no `.polint/cache`; warm reruns over the cache the cold run left. Stored bytes are `.polint/cache` after a cold run, raw and gzip-compressed. Per MB means per 1,000,000 source bytes. Process peak RSS comes from `wait4`, which on Linux counts the launching Python process's resident size at fork, so no process figure here is below the launcher's peak of 27 MB; the analysis column has no such floor.

| tree | source KB | cold analysis ms (per MB) | cold process ms | warm analysis ms (per MB) | warm process ms | cold peak RSS MB (analysis / process) | warm peak RSS MB | stored bytes per MB (gzip) |
|---|---|---|---|---|---|---|---|---|
| constructs (all cases) | 8 | 68 (8222) | 185 | 32 (3869) | 153 | 73 / 73 | 72 / 72 | 108277630 (11093349) |
| axios | 282 | 608 (2156) | 742 | 421 (1493) | 549 | 90 / 90 | 92 / 92 | 75350157 (5339729) |
| zod | 3088 | 13217 (4280) | 13484 | 12521 (4055) | 12765 | 1012 / 1012 | 992 / 992 | 95877087 (6780763) |
| commander | 127 | 580 (4582) | 796 | 343 (2709) | 559 | 74 / 74 | 73 / 73 | 44222052 (3192769) |
| async | 321 | 962 (3001) | 1369 | 869 (2711) | 1175 | 79 / 79 | 79 / 79 | 44203329 (3457642) |
| chalk | 10 | 156 (16385) | 439 | 129 (13549) | 454 | 74 / 74 | 73 / 73 | 86943598 (6904002) |
| dayjs | 7 | 73 (10194) | 275 | 43 (6005) | 250 | 74 / 74 | 73 / 73 | 221683284 (12812317) |
| underscore | 69 | 374 (5427) | 595 | 254 (3686) | 479 | 73 / 73 | 73 / 73 | 91000174 (5822567) |

Fact dump (no cache, one tree per process; peak RSS is the process figure, with the same floor): this is
the only way to time the refined plan, because no public view asks for it.

| tree | base ms | base peak RSS MB | refined ms | refined peak RSS MB |
|---|---|---|---|---|
| axios | 655 | 105 | 2257 | 234 |
| zod | 11514 | 1270 | 600181 (timed out after 600 s) | 2321 |
| commander | 301 | 62 | 1806 | 144 |
| async | 460 | 93 | 2113 | 267 |
| chalk | 53 | 32 | 96 | 40 |
| dayjs | 38 | 33 | 96 | 35 |
| underscore | 224 | 53 | 743 | 118 |

## Decisions recorded by this run

- Construct edges are identified by (file, line of the call expression's start, callee label); labels cannot express how many sites on one line call the same callee, so precision is counted per (line, callee).
- When one line labels a callee both as a direct call and as a callback, the callback counts as found only when a second distinct call site on that line reaches it.
- Predicted edges to functions without an @fn label are counted apart and are neither true nor false positives.
- A labelled hidden path (@unknown) is 'reported-unknown' when a call site on that line carries an unresolved marker, 'false-complete' when it has resolved targets and no marker, and 'silent' when polint has no call site there or one with neither. For the refined column the refined layer's own status decides; sites it has no row for keep the base layer's marker.
- A base edge whose target is an import binding rather than a function is a miss, counted as 'stops at import binding'.
- Observed package edges are mapped to polint by exact call-expression span (falling back to the polint call site in the same file ending at the same byte) and by callee end byte (innermost polint function in the same file ending there).
- Observed edges whose site or callee has no polint counterpart stay in the denominator: recall is over every observed edge.
- Export accuracy compares exported names per file; star re-export rows and the CommonJS whole-value `module.exports` row carry no name and are left out of the export-fact names but counted by status.
- Refined edges are counted when they carry a target function, whatever their status.
- Polint models a class as one function spanning the class body and has no separate constructor function, so a `Class.constructor` label and an observed constructor callee map to the class function.
- Observed implicit edges (accessors, coercions, iterator protocol) have a position but no call expression; they are reported by kind but left out of the headline package recall, which is over calls and callbacks.
- Packages are analysed with polint's default discovery, so its default excludes (for example `**/vendor/**`) apply; observed edges touching such files count as 'site or callee in a file polint does not analyse'.
- Missing call sites and functions are not worked around: functions under a unary operator, functions declared inside try, catch or if blocks, and calls in exported variable initializers are reported as misses.
- Process peak RSS is measured with `wait4` like the matrix runner does. Linux carries the parent's resident size into the child's peak at fork, so every process is launched before the report loads any fact dump, and the launcher's own peak is recorded with the cost section. The in-process analysis peak has no such floor.
- Each fact dump has a wall-clock limit (`--dump-timeout`, 600 s by default); a tree whose dump exceeds it is reported as timed out for that plan and has no scores in that column.
