# JS/TS analysis benchmark

polint 0.5.1. Written by `python3 tests/js-analysis-bench/bench.py report`;
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

| construct | cases (held out) | call recall base | call recall refined | callback recall base | callback recall refined | precision base | precision refined | hidden paths base: reported / false-complete / silent | hidden paths refined | false-complete call lines base / refined | stops at import binding (base) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| callback | 2 (0) | 3/7 (43%) | 5/7 (71%) | 0/3 (0%) | 0/3 (0%) | 3/3 (100%) | 5/5 (100%) | – | – | 0 / 0 | 0 |
| cjs-exports-property | 2 (0) | 0/4 (0%) | 3/4 (75%) | – | – | – | 3/3 (100%) | – | – | 0 / 0 | 0 |
| cjs-module-exports | 2 (1) | 1/6 (17%) | 5/6 (83%) | – | – | 1/1 (100%) | 5/5 (100%) | – | – | 0 / 0 | 0 |
| class-method | 2 (1) | 3/9 (33%) | 6/9 (67%) | – | – | 3/3 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| closure | 2 (0) | 3/7 (43%) | 6/7 (86%) | – | – | 3/3 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| dynamic-import | 2 (0) | 1/2 (50%) | 1/2 (50%) | – | – | 1/1 (100%) | 1/1 (100%) | 1 / 0 / 0 | 1 / 0 / 0 | 0 / 0 | 0 |
| dynamic-property | 2 (1) | 0/2 (0%) | 0/2 (0%) | – | – | – | – | 1 / 0 / 0 | 1 / 0 / 0 | 0 / 0 | 0 |
| esm-reexport | 2 (1) | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 0 / 0 | 0 |
| eval | 2 (1) | – | – | – | – | – | – | 2 / 0 / 0 | 2 / 0 / 0 | 0 / 0 | 0 |
| module-initializer | 6 (2) | 13/23 (57%) | 17/23 (74%) | – | – | 13/13 (100%) | 17/17 (100%) | – | – | 0 / 0 | 0 |
| object-of-functions | 2 (0) | 1/5 (20%) | 5/5 (100%) | – | – | 1/1 (100%) | 5/5 (100%) | – | – | 0 / 0 | 0 |
| promise | 2 (1) | 4/4 (100%) | 4/4 (100%) | 0/1 (0%) | 0/1 (0%) | 4/4 (100%) | 4/4 (100%) | – | – | 0 / 0 | 0 |
| prototype | 2 (1) | 2/7 (29%) | 4/7 (57%) | – | – | 2/2 (100%) | 4/4 (100%) | – | – | 0 / 0 | 0 |
| returned-function | 4 (2) | 8/23 (35%) | 14/23 (61%) | – | – | 8/8 (100%) | 14/14 (100%) | – | – | 0 / 0 | 0 |

**False-complete count** (a labelled hidden path where polint gives a resolved answer with no unresolved marker): base 0, refined 0, over 4 labelled hidden paths.

**False-complete call lines** (a labelled call line where polint gives resolved targets that miss a labelled callee, with no unresolved marker on the line): base 0, refined 0, over 83 labelled call lines.

| case | plan | line | resolved to | missed |
|---|---|---|---|---|
| none | | | | |

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
| esm-reexport/named-barrel | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 50% | yes |
| esm-reexport/star-and-alias | yes | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 0% | yes |
| eval/direct-eval | no | – | – | – | – | – | – | reported-unknown | reported-unknown | 100% | yes |
| eval/function-constructor | yes | – | – | – | – | – | – | reported-unknown | reported-unknown | 100% | yes |
| module-initializer/arrow-in-initializer | no | 2/4 (50%) | 3/4 (75%) | – | – | 2/2 (100%) | 3/3 (100%) | – | – | 60% | yes |
| module-initializer/chained-call | yes | 3/5 (60%) | 3/5 (60%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 50% | yes |
| module-initializer/class-expression | yes | 1/4 (25%) | 2/4 (50%) | – | – | 1/1 (100%) | 2/2 (100%) | – | – | 80% | yes |
| module-initializer/exported-call | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 0% | yes |
| module-initializer/local-initializer | no | 3/4 (75%) | 4/4 (100%) | – | – | 3/3 (100%) | 4/4 (100%) | – | – | 25% | yes |
| module-initializer/object-literal-methods | no | 2/4 (50%) | 3/4 (75%) | – | – | 2/2 (100%) | 3/3 (100%) | – | – | 50% | yes |
| object-of-functions/literal-methods | no | 0/2 (0%) | 2/2 (100%) | – | – | – | 2/2 (100%) | – | – | 100% | yes |
| object-of-functions/registry-built-later | no | 1/3 (33%) | 3/3 (100%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 67% | yes |
| promise/async-await | yes | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| promise/then-chain | no | 1/1 (100%) | 1/1 (100%) | 0/1 (0%) | 0/1 (0%) | 1/1 (100%) | 1/1 (100%) | – | – | 60% | no |
| prototype/constructor-function | no | 1/3 (33%) | 3/3 (100%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 67% | yes |
| prototype/inherited-method | yes | 1/4 (25%) | 1/4 (25%) | – | – | 1/1 (100%) | 1/1 (100%) | – | – | 60% | yes |
| returned-function/branch-returns | no | 1/4 (25%) | 4/4 (100%) | – | – | 1/1 (100%) | 4/4 (100%) | – | – | 50% | yes |
| returned-function/conditional-default | yes | 3/9 (33%) | 3/9 (33%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 50% | yes |
| returned-function/stored-in-property | yes | 1/4 (25%) | 1/4 (25%) | – | – | 1/1 (100%) | 1/1 (100%) | – | – | 50% | yes |
| returned-function/two-levels | no | 3/6 (50%) | 6/6 (100%) | – | – | 3/3 (100%) | 6/6 (100%) | – | – | 25% | yes |

## Packages (observed calls: recall only)

Recall is over observed edges at call expressions (`call` and `callback`); `implicit` calls (getters,
coercions, iterator and thenable protocol) have no call expression and are listed by kind only.

| package | format | files analysed / selected | source KB | observed edges | recall base | recall refined | stops at import binding (base) | not mappable to a polint site or function | unresolved call share base | call sites without target base / refined | unresolved reference share |
|---|---|---|---|---|---|---|---|---|---|---|---|
| axios 1.21.1 | ESM source, Node and browser adapters | 72 / 72 | 282 | 675 | 270/675 (40%) | 270/675 (40%) | 1 | 20 | 64% | 74% / 74% | 9% |
| zod 4.6.7 | TypeScript ESM source | 332 / 332 | 3088 | 1308 | 719/1308 (55%) | 719/1308 (55%) | 0 | 9 | 65% | 89% / 89% | 7% |
| commander 14.0.3 | CommonJS only | 7 / 7 | 127 | 318 | 21/318 (7%) | 119/318 (37%) | 0 | 1 | 92% | 95% / 77% | 10% |
| async 3.2.6 | CommonJS transpiled from ESM | 129 / 129 | 321 | 637 | 324/637 (51%) | 328/637 (51%) | 0 | 1 | 63% | 73% / 72% | 27% |
| chalk 6.0.1 | ESM only, with subpath imports | 2 / 5 | 10 | 72 | 17/72 (24%) | 17/72 (24%) | 0 | 47 | 52% | 70% / 70% | 9% |
| dayjs 1.11.23 | minified UMD | 1 / 1 | 7 | 117 | 0/117 (0%) | 0/117 (0%) | 0 | 117 | 100% | 100% / 100% | 7% |
| underscore 1.13.8 | bundled UMD (Rollup) | 1 / 1 | 69 | 262 | 157/262 (60%) | 158/262 (60%) | 0 | 0 | 55% | 58% / 58% | 6% |

### Claimed targets the run never called

A claimed target is resolved and not heuristic. It counts here when the run executed the site, the target is
in an instrumented file, and the run never called it from that site. On a **single-target** answer that is a
false precise edge: every execution of the site went elsewhere. On a **multi-target** answer it is an upper
bound only, since the run may not have taken the paths to the other targets. A site matched to a polint site
by its end byte can also inflate both counts.

| package | base: single / multi | refined: single / multi | typed: single / multi |
|---|---|---|---|
| axios | 0 / 0 | 0 / 0 | 2 / 2 |
| zod | 0 / 0 | 0 / 0 | 1 / 0 |
| commander | 0 / 0 | 0 / 0 | 5 / 0 |
| async | 0 / 0 | 0 / 0 | 0 / 0 |
| chalk | 0 / 0 | 0 / 0 | 0 / 0 |
| dayjs | 0 / 0 | 0 / 0 | 0 / 0 |
| underscore | 0 / 0 | 0 / 0 | 7 / 0 |
| **total** | **0 / 0** | **0 / 0** | **15 / 2** |

| package | plan | site | site text | single-target answer | the run called |
|---|---|---|---|---|---|
| axios | typed | lib/helpers/AxiosTransformStream.js:86 | `this.emit('progress', internals.bytesSeen)` | lib/helpers/AxiosTransformStream.js:8 AxiosTransformStream | lib/helpers/progressEventReducer.js:41 <polint:anonymous:1209:1307> |
| axios | typed | lib/helpers/AxiosTransformStream.js:88 | `this.push(_chunk)` | lib/helpers/AxiosTransformStream.js:8 AxiosTransformStream | lib/adapters/http.js:1382 <polint:anonymous:47809:48564> |
| zod | typed | src/v4/classic/schemas.ts:2402 | `new ZodDefault({ type: "default", innerType: innerType as any as core.$ZodType, get defau…` | src/v4/core/util.ts:558 shallowClone | src/v4/core/core.ts:118 _ |
| commander | typed | lib/command.js:535 | `this._exitCallback(new CommanderError(exitCode, code, message))` | lib/command.js:13 Command | lib/command.js:512 <polint:anonymous:17297:17494> |
| commander | typed | lib/command.js:601 | `target.parseArg(value, previous)` | lib/argument.js:100 <polint:anonymous:2195:2491> | lib/option.js:183 <polint:anonymous:4994:5290> |
| commander | typed | lib/command.js:1787 | `this.emit('option:${activeVariadicOption.name()}', arg)` | lib/command.js:13 Command | lib/command.js:720 <polint:anonymous:23441:23614> |
| commander | typed | lib/command.js:1799 | `this.emit('option:${option.name()}', value)` | lib/command.js:13 Command | lib/command.js:720 <polint:anonymous:23441:23614> |
| commander | typed | lib/command.js:1812 | `this.emit('option:${option.name()}')` | lib/command.js:13 Command | lib/command.js:720 <polint:anonymous:23441:23614>, lib/command.js:2204 <polint:anonymous:70854:70968> |
| underscore | typed | underscore-umd.js:589 | `isFunction$1(obj[key])` | underscore-umd.js:116 <polint:anonymous:4470:4532> | underscore-umd.js:141 <polint:anonymous:5181:5250> |
| underscore | typed | underscore-umd.js:759 | `isFunction$1(value)` | underscore-umd.js:116 <polint:anonymous:4470:4532> | underscore-umd.js:141 <polint:anonymous:5181:5250> |
| underscore | typed | underscore-umd.js:982 | `isFunction$1(prop)` | underscore-umd.js:116 <polint:anonymous:4470:4532> | underscore-umd.js:141 <polint:anonymous:5181:5250> |
| underscore | typed | underscore-umd.js:1036 | `isFunction$1(func)` | underscore-umd.js:116 <polint:anonymous:4470:4532> | underscore-umd.js:141 <polint:anonymous:5181:5250> |
| underscore | typed | underscore-umd.js:1457 | `isFunction$1(path)` | underscore-umd.js:116 <polint:anonymous:4470:4532> | underscore-umd.js:141 <polint:anonymous:5181:5250> |
| underscore | typed | underscore-umd.js:1653 | `isFunction$1(iteratee)` | underscore-umd.js:116 <polint:anonymous:4470:4532> | underscore-umd.js:141 <polint:anonymous:5181:5250> |
| underscore | typed | underscore-umd.js:1672 | `isFunction$1(iteratee)` | underscore-umd.js:116 <polint:anonymous:4470:4532> | underscore-umd.js:141 <polint:anonymous:5181:5250> |

### What types add (refined vs typed, TypeScript 5.9.3)

**typed** is the refined plan with `[languages.ts] type_sidecar = true` over a copy of each tree with a
tsconfig (`allowJs`, `nodenext`). The type tier only feeds the refined layer, so base is unchanged by it.
Dump time and peak RSS are one fact dump each, sidecar included.

| package | recall refined | recall typed | false-complete refined / typed | type callsite rows | dump s refined / typed | dump peak MB refined / typed |
|---|---|---|---|---|---|---|
| axios | 270/675 (40%) | 511/675 (76%) | 0 / 0 | 2098 | 2.2 / 3.3 | 285.5 / 308.0 |
| zod | 719/1308 (55%) | 764/1308 (58%) | 0 / 0 | 49041 | 86.3 / 129.1 | 3897.8 / 4418.0 |
| commander | 119/318 (37%) | 248/318 (78%) | 0 / 0 | 814 | 1.8 / 2.4 | 145.5 / 230.8 |
| async | 328/637 (51%) | 338/637 (53%) | 0 / 0 | 1605 | 3.1 / 3.3 | 267.6 / 281.8 |
| chalk | 17/72 (24%) | 17/72 (24%) | 0 / 0 | 60 | 0.1 / 0.6 | 41.1 / 216.0 |
| dayjs | 0/117 (0%) | 0/117 (0%) | 0 / 0 | 181 | 0.1 / 0.9 | 36.2 / 225.3 |
| underscore | 158/262 (60%) | 202/262 (77%) | 0 / 0 | 455 | 1.2 / 2.5 | 121.0 / 228.3 |

Constructs, typed: false-complete hidden paths 0, false-complete call lines 0.

### Package recall by observed call kind and syntax

| package | kind / syntax | observed | base | refined |
|---|---|---|---|---|
| axios | kind: call | 632 | 269/632 (43%) | 269/632 (43%) |
| axios | kind: callback | 43 | 1/43 (2%) | 1/43 (2%) |
| axios | kind: implicit | 6 | 0/6 (0%) | 0/6 (0%) |
| axios | syntax: computed-member | 2 | 0/2 (0%) | 0/2 (0%) |
| axios | syntax: expression | 8 | 8/8 (100%) | 8/8 (100%) |
| axios | syntax: identifier | 317 | 246/317 (78%) | 246/317 (78%) |
| axios | syntax: member | 326 | 1/326 (0%) | 1/326 (0%) |
| axios | syntax: new | 21 | 15/21 (71%) | 15/21 (71%) |
| axios | syntax: super | 1 | 0/1 (0%) | 0/1 (0%) |
| zod | kind: call | 1265 | 719/1265 (57%) | 719/1265 (57%) |
| zod | kind: callback | 43 | 0/43 (0%) | 0/43 (0%) |
| zod | kind: implicit | 18 | 0/18 (0%) | 0/18 (0%) |
| zod | syntax: computed-member | 5 | 0/5 (0%) | 0/5 (0%) |
| zod | syntax: expression | 1 | 1/1 (100%) | 1/1 (100%) |
| zod | syntax: identifier | 408 | 244/408 (60%) | 244/408 (60%) |
| zod | syntax: member | 829 | 472/829 (57%) | 472/829 (57%) |
| zod | syntax: new | 65 | 2/65 (3%) | 2/65 (3%) |
| commander | kind: call | 264 | 21/264 (8%) | 119/264 (45%) |
| commander | kind: callback | 54 | 0/54 (0%) | 0/54 (0%) |
| commander | syntax: identifier | 35 | 20/35 (57%) | 22/35 (63%) |
| commander | syntax: member | 274 | 0/274 (0%) | 90/274 (33%) |
| commander | syntax: new | 8 | 1/8 (12%) | 7/8 (88%) |
| commander | syntax: super | 1 | 0/1 (0%) | 0/1 (0%) |
| async | kind: call | 622 | 324/622 (52%) | 328/622 (53%) |
| async | kind: callback | 15 | 0/15 (0%) | 0/15 (0%) |
| async | kind: implicit | 4 | 0/4 (0%) | 0/4 (0%) |
| async | syntax: expression | 157 | 0/157 (0%) | 0/157 (0%) |
| async | syntax: identifier | 403 | 324/403 (80%) | 324/403 (80%) |
| async | syntax: member | 73 | 0/73 (0%) | 4/73 (5%) |
| async | syntax: new | 4 | 0/4 (0%) | 0/4 (0%) |
| chalk | kind: call | 71 | 17/71 (24%) | 17/71 (24%) |
| chalk | kind: callback | 1 | 0/1 (0%) | 0/1 (0%) |
| chalk | kind: implicit | 1 | 0/1 (0%) | 0/1 (0%) |
| chalk | syntax: computed-member | 10 | 0/10 (0%) | 0/10 (0%) |
| chalk | syntax: identifier | 43 | 17/43 (40%) | 17/43 (40%) |
| chalk | syntax: member | 19 | 0/19 (0%) | 0/19 (0%) |
| dayjs | kind: call | 115 | 0/115 (0%) | 0/115 (0%) |
| dayjs | kind: callback | 2 | 0/2 (0%) | 0/2 (0%) |
| dayjs | kind: implicit | 3 | 0/3 (0%) | 0/3 (0%) |
| dayjs | syntax: computed-member | 1 | 0/1 (0%) | 0/1 (0%) |
| dayjs | syntax: expression | 5 | 0/5 (0%) | 0/5 (0%) |
| dayjs | syntax: identifier | 35 | 0/35 (0%) | 0/35 (0%) |
| dayjs | syntax: member | 75 | 0/75 (0%) | 0/75 (0%) |
| dayjs | syntax: new | 1 | 0/1 (0%) | 0/1 (0%) |
| underscore | kind: call | 257 | 157/257 (61%) | 158/257 (61%) |
| underscore | kind: callback | 5 | 0/5 (0%) | 0/5 (0%) |
| underscore | syntax: expression | 2 | 2/2 (100%) | 2/2 (100%) |
| underscore | syntax: identifier | 234 | 154/234 (66%) | 155/234 (66%) |
| underscore | syntax: member | 25 | 0/25 (0%) | 0/25 (0%) |
| underscore | syntax: new | 1 | 1/1 (100%) | 1/1 (100%) |

### Why observed edges were missed (base / refined)

| package | callee not a polint function | found | no target, marked unresolved | no target, no unresolved marker | other targets, marked unresolved | site not a polint call site | site or callee in a file polint does not analyse | stops at import binding |
|---|---|---|---|---|---|---|---|---|
| axios | 9 / 9 / 9 | 270 / 270 / 511 | 376 / 376 / 132 | 8 / 9 / 0 | 0 / 0 / 12 | 11 / 11 / 11 | 0 / 0 / 0 | 1 / 0 / 0 |
| zod | 6 / 6 / 6 | 719 / 719 / 764 | 579 / 579 / 532 | 1 / 1 / 1 | 0 / 0 / 2 | 3 / 3 / 3 | 0 / 0 / 0 | 0 / 0 / 0 |
| commander | 1 / 1 / 1 | 21 / 119 / 248 | 296 / 198 / 63 | 0 / 0 / 0 | 0 / 0 / 6 | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| async | 1 / 1 / 1 | 324 / 328 / 338 | 312 / 308 / 296 | 0 / 0 / 0 | 0 / 0 / 2 | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| chalk | 0 / 0 / 0 | 17 / 17 / 17 | 8 / 8 / 8 | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 | 47 / 47 / 47 | 0 / 0 / 0 |
| dayjs | 1 / 1 / 1 | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 | 116 / 116 / 116 | 0 / 0 / 0 | 0 / 0 / 0 |
| underscore | 0 / 0 / 0 | 157 / 158 / 202 | 105 / 104 / 53 | 0 / 0 / 0 | 0 / 0 / 7 | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |

### Missed edges ranked by gap

Every observed call or callback edge polint did not find is attributed to one gap: the first in the order
unary IIFEs, try / catch / if blocks, exported initializers, constructors, callbacks to built-ins, computed
keys, prototype and class methods, then the remainders. Extraction gaps come first because no resolution
change finds an edge whose call site or callee polint does not extract; a site or callee polint misses is
charged to the outermost function around it that polint does not extract. **attributed** counts each edge
once; **applying** counts every gap whose condition holds. The recall gain is an upper bound: it assumes
every attributed edge is found once the gap is fixed. The syntax census behind this is described in the README.

Base: 1881 missed of 3389 observed edges. Refined: 1778 missed of 3389.

| rank | gap | attributed base | applying base | recall gain base (upper bound) | attributed refined | recall gain refined (upper bound) | base by package |
|---|---|---|---|---|---|---|---|
| 1 | _other member calls (object-literal functions, namespaces)_ | 633 | 0 | 18.7% | 632 | 18.6% | zod 304, axios 247, async 52, underscore 20, commander 10 |
| 2 | _other resolution misses_ | 532 | 0 | 15.7% | 530 | 15.6% | async 235, zod 145, underscore 79, axios 58, commander 15 |
| 3 | prototype and class methods, incl. inheritance | 273 | 277 | 8.1% | 180 | 5.3% | commander 210, axios 34, zod 20, async 8, underscore 1 |
| 4 | callbacks invoked by built-ins | 151 | 162 | 4.5% | 151 | 4.5% | commander 53, zod 43, axios 37, async 13, underscore 5 |
| 5 | functions under a unary operator (unary IIFEs) | 118 | 118 | 3.5% | 118 | 3.5% | dayjs 117, axios 1 |
| 6 | class constructors and `new` | 81 | 83 | 2.4% | 75 | 2.2% | zod 63, commander 8, axios 6, async 4 |
| 7 | _file not analysed (discovery excludes)_ | 47 | 0 | 1.4% | 47 | 1.4% | chalk 47 |
| 8 | functions declared in try / catch / if blocks | 17 | 17 | 0.5% | 17 | 0.5% | axios 16, commander 1 |
| 9 | computed-key member calls | 15 | 18 | 0.4% | 15 | 0.4% | chalk 8, zod 5, axios 2 |
| 10 | _other call sites or functions not extracted_ | 13 | 0 | 0.4% | 13 | 0.4% | zod 9, axios 3, async 1 |
| 11 | _identifier call stops at import binding_ | 1 | 0 | 0.0% | 0 | 0.0% | axios 1 |
| 12 | calls in exported initializers | 0 | 0 | 0.0% | 0 | 0.0% | – |

Rows in italics are remainders, not one of the named extraction or resolution gaps. Of the base
prototype and class method misses, 2 go through `super` or a `this`
call into another class; that split reads the receiver syntactically and is approximate.
'Other resolution misses' by call syntax and callee kind (base): identifier call to arrow 239, expression call to declaration 134, identifier call to expression 96, identifier call to declaration 39, expression call to arrow 14, expression call to expression 9, identifier call to method 1. An `expression` call is one whose callee is neither a name nor a member, such as the `(0, f)(x)`
form compiled CommonJS uses.

### Other targets and no unresolved marker (every case)

The observed callee is not among polint's targets for the site, and the site carries no unresolved marker.
The polint site column shows which call expression the observed site was matched to.

| package | plan | observed site | syntax / kind | observed callee | polint answer | observed site text | polint site text |
|---|---|---|---|---|---|---|---|
| none | | | | | | | |

### One example per package, plan and outcome

`report.json` keeps up to 5 per outcome (all of them for 'other targets, no unresolved marker') and up to
3 per gap.

| package | plan | outcome | observed site | syntax / kind | observed callee | polint answer |
|---|---|---|---|---|---|---|
| axios | base | callee not a polint function | lib/adapters/fetch.js:432 `trackRequestStream(_request.body, onProgress, flush)` | identifier / call | lib/adapters/fetch.js:381 trackRequestStream | no target, marked unresolved |
| axios | base | found | lib/adapters/adapters.js:80 `isResolvedHandle(nameOrAdapter)` | identifier / call | lib/adapters/adapters.js:52 isResolvedHandle | targets lib/adapters/adapters.js:52 isResolvedHandle, no unresolved marker |
| axios | base | no target, marked unresolved | lib/adapters/adapters.js:53 `utils.isFunction(adapter)` | member / call | lib/utils.js:194 <anonymous> | no target, marked unresolved |
| axios | base | no target, no unresolved marker | lib/adapters/fetch.js:535 `AxiosHeaders.from(response.headers)` | member / call | lib/core/AxiosHeaders.js:423 from | no target, no unresolved marker |
| axios | base | site not a polint call site | lib/adapters/fetch.js:382 `trackStream( stream, DEFAULT_CHUNK_SIZE, (loadedBytes) => { if (hasMaxBodyLength && loade…` | identifier / call | lib/helpers/trackStream.js:45 trackStream | no call site |
| axios | base | stops at import binding | lib/core/AxiosHeaders.js:244 `parseHeaders(header)` | identifier / call | lib/helpers/parseHeaders.js:41 <anonymous> | import binding, no unresolved marker |
| axios | refined | callee not a polint function | lib/adapters/fetch.js:432 `trackRequestStream(_request.body, onProgress, flush)` | identifier / call | lib/adapters/fetch.js:381 trackRequestStream | no target, marked unresolved |
| axios | refined | found | lib/adapters/adapters.js:80 `isResolvedHandle(nameOrAdapter)` | identifier / call | lib/adapters/adapters.js:52 isResolvedHandle | targets lib/adapters/adapters.js:52 isResolvedHandle, no unresolved marker |
| axios | refined | no target, marked unresolved | lib/adapters/adapters.js:53 `utils.isFunction(adapter)` | member / call | lib/utils.js:194 <anonymous> | no target, marked unresolved |
| axios | refined | no target, no unresolved marker | lib/adapters/fetch.js:535 `AxiosHeaders.from(response.headers)` | member / call | lib/core/AxiosHeaders.js:423 from | no target, no unresolved marker |
| axios | refined | site not a polint call site | lib/adapters/fetch.js:382 `trackStream( stream, DEFAULT_CHUNK_SIZE, (loadedBytes) => { if (hasMaxBodyLength && loade…` | identifier / call | lib/helpers/trackStream.js:45 trackStream | no call site |
| axios | typed | callee not a polint function | lib/adapters/fetch.js:432 `trackRequestStream(_request.body, onProgress, flush)` | identifier / call | lib/adapters/fetch.js:381 trackRequestStream | targets lib/adapters/fetch.js:214 <polint:anonymous:6074:23281>, marked unresolved |
| axios | typed | found | lib/adapters/adapters.js:53 `utils.isFunction(adapter)` | member / call | lib/utils.js:194 <anonymous> | targets lib/utils.js:194 <polint:anonymous:5832:5864>, marked unresolved |
| axios | typed | no target, marked unresolved | lib/adapters/adapters.js:88 `adapter.get(config)` | member / call | lib/adapters/fetch.js:718 getFetch | no target, marked unresolved |
| axios | typed | other targets, marked unresolved | lib/adapters/http.js:909 `AxiosHeaders.from(config.headers).normalize()` | member / call | lib/core/AxiosHeaders.js:478 get | targets lib/core/AxiosHeaders.js:361 AxiosHeaders.normalize, marked unresolved |
| axios | typed | site not a polint call site | lib/adapters/fetch.js:382 `trackStream( stream, DEFAULT_CHUNK_SIZE, (loadedBytes) => { if (hasMaxBodyLength && loade…` | identifier / call | lib/helpers/trackStream.js:45 trackStream | no call site |
| zod | base | callee not a polint function | src/v3/ZodError.ts:315 `mapper(sub)` | identifier / call | src/v3/ZodError.ts:308 <anonymous> | no target, marked unresolved |
| zod | base | found | src/v3/ZodError.ts:272 `processError(this)` | identifier / call | src/v3/ZodError.ts:224 processError | targets src/v3/ZodError.ts:224 <polint:anonymous:5894:7736>, no unresolved marker |
| zod | base | no target, marked unresolved | src/v3/ZodError.ts:264 `mapper(issue)` | identifier / call | src/v3/ZodError.ts:220 <anonymous> | no target, marked unresolved |
| zod | base | no target, no unresolved marker | src/v4/core/json-schema-processors.ts:750 `JSON.stringify(value, (_, val) => { if (typeof val !== "bigint") return val; unrepresenta…` | member / callback | src/v4/core/json-schema-processors.ts:750 <anonymous> | no target, no unresolved marker |
| zod | base | site not a polint call site | src/v4/classic/schemas.ts:2406 `util.shallowClone(defaultValue)` | member / call | src/v4/core/util.ts:558 shallowClone | no call site |
| zod | refined | callee not a polint function | src/v3/ZodError.ts:315 `mapper(sub)` | identifier / call | src/v3/ZodError.ts:308 <anonymous> | no target, marked unresolved |
| zod | refined | found | src/v3/ZodError.ts:272 `processError(this)` | identifier / call | src/v3/ZodError.ts:224 processError | targets src/v3/ZodError.ts:224 <polint:anonymous:5894:7736>, no unresolved marker |
| zod | refined | no target, marked unresolved | src/v3/ZodError.ts:264 `mapper(issue)` | identifier / call | src/v3/ZodError.ts:220 <anonymous> | no target, marked unresolved |
| zod | refined | no target, no unresolved marker | src/v4/core/json-schema-processors.ts:750 `JSON.stringify(value, (_, val) => { if (typeof val !== "bigint") return val; unrepresenta…` | member / callback | src/v4/core/json-schema-processors.ts:750 <anonymous> | no target, no unresolved marker |
| zod | refined | site not a polint call site | src/v4/classic/schemas.ts:2406 `util.shallowClone(defaultValue)` | member / call | src/v4/core/util.ts:558 shallowClone | no call site |
| zod | typed | callee not a polint function | src/v3/ZodError.ts:315 `mapper(sub)` | identifier / call | src/v3/ZodError.ts:308 <anonymous> | no target, marked unresolved |
| zod | typed | found | src/v3/ZodError.ts:272 `processError(this)` | identifier / call | src/v3/ZodError.ts:224 processError | targets src/v3/ZodError.ts:224 <polint:anonymous:5894:7736>, no unresolved marker |
| zod | typed | no target, marked unresolved | src/v3/ZodError.ts:264 `mapper(issue)` | identifier / call | src/v3/ZodError.ts:220 <anonymous> | no target, marked unresolved |
| zod | typed | no target, no unresolved marker | src/v4/core/json-schema-processors.ts:750 `JSON.stringify(value, (_, val) => { if (typeof val !== "bigint") return val; unrepresenta…` | member / callback | src/v4/core/json-schema-processors.ts:750 <anonymous> | no target, no unresolved marker |
| zod | typed | other targets, marked unresolved | src/v4/classic/schemas.ts:2402 `new ZodDefault({ type: "default", innerType: innerType as any as core.$ZodType, get defau…` | new / call | src/v4/core/core.ts:118 _ | targets src/v4/core/util.ts:558 shallowClone, marked unresolved |
| zod | typed | site not a polint call site | src/v4/classic/schemas.ts:2406 `util.shallowClone(defaultValue)` | member / call | src/v4/core/util.ts:558 shallowClone | no call site |
| commander | base | callee not a polint function | lib/help.js:331 `option.argChoices.map((choice) => JSON.stringify(choice))` | member / callback | lib/help.js:331 <anonymous> | no target, marked unresolved |
| commander | base | found | lib/command.js:649 `knownBy(command)` | identifier / call | lib/command.js:645 knownBy | targets lib/command.js:645 <polint:anonymous:20982:21047>, no unresolved marker |
| commander | base | no target, marked unresolved | index.js:7 `new Command()` | new / call | lib/command.js:20 constructor | no target, marked unresolved |
| commander | refined | callee not a polint function | lib/help.js:331 `option.argChoices.map((choice) => JSON.stringify(choice))` | member / callback | lib/help.js:331 <anonymous> | no target, marked unresolved |
| commander | refined | found | index.js:7 `new Command()` | new / call | lib/command.js:20 constructor | targets lib/command.js:13 Command, no unresolved marker |
| commander | refined | no target, marked unresolved | lib/argument.js:144 `arg.name()` | member / call | lib/argument.js:48 name | no target, marked unresolved |
| commander | typed | callee not a polint function | lib/help.js:331 `option.argChoices.map((choice) => JSON.stringify(choice))` | member / callback | lib/help.js:331 <anonymous> | no target, marked unresolved |
| commander | typed | found | index.js:7 `new Command()` | new / call | lib/command.js:20 constructor | targets lib/command.js:13 Command, no unresolved marker |
| commander | typed | no target, marked unresolved | lib/command.js:646 `cmd.name()` | member / call | lib/command.js:2331 name | no target, marked unresolved |
| commander | typed | other targets, marked unresolved | lib/command.js:601 `target.parseArg(value, previous)` | member / call | lib/option.js:183 parseArg | targets lib/argument.js:100 <polint:anonymous:2195:2491>, marked unresolved |
| async | base | callee not a polint function | memoize.js:67 `hasher(...args)` | identifier / call | memoize.js:62 <anonymous> | no target, marked unresolved |
| async | base | found | applyEach.js:9 `_interopRequireDefault(_applyEach)` | identifier / call | applyEach.js:15 _interopRequireDefault | targets applyEach.js:15 _interopRequireDefault, no unresolved marker |
| async | base | no target, marked unresolved | applyEach.js:56 `(0, _applyEach2.default)(_map2.default)` | expression / call | internal/applyEach.js:7 default | no target, marked unresolved |
| async | refined | callee not a polint function | memoize.js:67 `hasher(...args)` | identifier / call | memoize.js:62 <anonymous> | no target, marked unresolved |
| async | refined | found | applyEach.js:9 `_interopRequireDefault(_applyEach)` | identifier / call | applyEach.js:15 _interopRequireDefault | targets applyEach.js:15 _interopRequireDefault, no unresolved marker |
| async | refined | no target, marked unresolved | applyEach.js:56 `(0, _applyEach2.default)(_map2.default)` | expression / call | internal/applyEach.js:7 default | no target, marked unresolved |
| async | typed | callee not a polint function | memoize.js:67 `hasher(...args)` | identifier / call | memoize.js:62 <anonymous> | no target, marked unresolved |
| async | typed | found | applyEach.js:9 `_interopRequireDefault(_applyEach)` | identifier / call | applyEach.js:15 _interopRequireDefault | targets applyEach.js:15 _interopRequireDefault, no unresolved marker |
| async | typed | no target, marked unresolved | applyEach.js:56 `(0, _applyEach2.default)(_map2.default)` | expression / call | internal/applyEach.js:7 default | no target, marked unresolved |
| async | typed | other targets, marked unresolved | eachOf.js:181 `eachOfImplementation(coll, (0, _wrapAsync2.default)(iteratee), callback)` | identifier / call | eachOf.js:66 eachOfGeneric | targets eachOf.js:38 eachOfArrayLike, marked unresolved |
| chalk | base | found | source/index.js:37 `assertValidLevel(options.level)` | identifier / call | source/index.js:17 assertValidLevel | targets source/index.js:17 assertValidLevel, no unresolved marker |
| chalk | base | no target, marked unresolved | source/index.js:125 `converters[this.level](first, second, third)` | computed-member / call | source/index.js:91 ansi | no target, marked unresolved |
| chalk | base | site or callee in a file polint does not analyse | source/index.js:91 `style.ansi(ansiStyles.rgbToAnsi(red, green, blue))` | member / call | source/vendor/ansi-styles/index.js:4 <anonymous> | no target, marked unresolved |
| chalk | refined | found | source/index.js:37 `assertValidLevel(options.level)` | identifier / call | source/index.js:17 assertValidLevel | targets source/index.js:17 assertValidLevel, no unresolved marker |
| chalk | refined | no target, marked unresolved | source/index.js:125 `converters[this.level](first, second, third)` | computed-member / call | source/index.js:91 ansi | no target, marked unresolved |
| chalk | refined | site or callee in a file polint does not analyse | source/index.js:91 `style.ansi(ansiStyles.rgbToAnsi(red, green, blue))` | member / call | source/vendor/ansi-styles/index.js:4 <anonymous> | no target, marked unresolved |
| chalk | typed | found | source/index.js:37 `assertValidLevel(options.level)` | identifier / call | source/index.js:17 assertValidLevel | targets source/index.js:17 assertValidLevel, no unresolved marker |
| chalk | typed | no target, marked unresolved | source/index.js:125 `converters[this.level](first, second, third)` | computed-member / call | source/index.js:91 ansi | no target, marked unresolved |
| chalk | typed | site or callee in a file polint does not analyse | source/index.js:91 `style.ansi(ansiStyles.rgbToAnsi(red, green, blue))` | member / call | source/vendor/ansi-styles/index.js:4 <anonymous> | no target, marked unresolved |
| dayjs | base | callee not a polint function | dayjs.min.js:1 `function(t,e){"object"==typeof exports&&"undefined"!=typeof module?module.exports=e():"fu…` | expression / call | dayjs.min.js:1 <anonymous> | no target, marked unresolved |
| dayjs | base | site not a polint call site | dayjs.min.js:1 `m(r,2,"0")` | identifier / call | dayjs.min.js:1 m | no call site |
| dayjs | refined | callee not a polint function | dayjs.min.js:1 `function(t,e){"object"==typeof exports&&"undefined"!=typeof module?module.exports=e():"fu…` | expression / call | dayjs.min.js:1 <anonymous> | no target, marked unresolved |
| dayjs | refined | site not a polint call site | dayjs.min.js:1 `m(r,2,"0")` | identifier / call | dayjs.min.js:1 m | no call site |
| dayjs | typed | callee not a polint function | dayjs.min.js:1 `function(t,e){"object"==typeof exports&&"undefined"!=typeof module?module.exports=e():"fu…` | expression / call | dayjs.min.js:1 <anonymous> | targets dayjs.min.js:1 <polint:module>, marked unresolved |
| dayjs | typed | site not a polint call site | dayjs.min.js:1 `m(r,2,"0")` | identifier / call | dayjs.min.js:1 m | no call site |
| underscore | base | found | underscore-umd.js:1 `function (global, factory) { typeof exports === 'object' && typeof module !== 'undefined'…` | expression / call | underscore-umd.js:1 <anonymous> | targets underscore-umd.js:1 <polint:anonymous:1:451>, no unresolved marker |
| underscore | base | no target, marked unresolved | underscore-umd.js:304 `getLength(obj)` | identifier / call | underscore-umd.js:223 <anonymous> | no target, marked unresolved |
| underscore | refined | found | underscore-umd.js:1 `function (global, factory) { typeof exports === 'object' && typeof module !== 'undefined'…` | expression / call | underscore-umd.js:1 <anonymous> | targets underscore-umd.js:1 <polint:anonymous:1:451>, no unresolved marker |
| underscore | refined | no target, marked unresolved | underscore-umd.js:304 `getLength(obj)` | identifier / call | underscore-umd.js:223 <anonymous> | no target, marked unresolved |
| underscore | typed | found | underscore-umd.js:1 `function (global, factory) { typeof exports === 'object' && typeof module !== 'undefined'…` | expression / call | underscore-umd.js:1 <anonymous> | targets underscore-umd.js:1 <polint:anonymous:1:451>, no unresolved marker |
| underscore | typed | no target, marked unresolved | underscore-umd.js:602 `keysFunc(source)` | identifier / call | underscore-umd.js:288 keys | no target, marked unresolved |
| underscore | typed | other targets, marked unresolved | underscore-umd.js:589 `isFunction$1(obj[key])` | identifier / call | underscore-umd.js:141 isFunction | targets underscore-umd.js:116 <polint:anonymous:4470:4532>, marked unresolved |

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
| case module-initializer/arrow-in-initializer | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 |
| case module-initializer/chained-call | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 |
| case module-initializer/class-expression | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 |
| case module-initializer/exported-call | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 |
| case module-initializer/object-literal-methods | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 |

## Cost

The only part of this report that changes between runs. Machine: Linux-6.8.0-147-generic-x86_64-with-glibc2.39, 32 CPUs, commit 5baf5f78cca2.

`polint check` (release) with the census rule pack, which asks for `Functions`, `CallGraph`, `Symbols`, `References`; median of 3 runs. `analysis` is the in-process cost record (`POLINT_GOLDEN_COST_PATH`), `process` is the whole CLI as `bench_matrix.measure` sees it. Cold has no `.polint/cache`; warm reruns over the cache the cold run left. Stored bytes are `.polint/cache` after a cold run, raw and gzip-compressed. Per MB means per 1,000,000 source bytes. Process peak RSS comes from `wait4`, which on Linux counts the launching Python process's resident size at fork, so no process figure here is below the launcher's peak of 31 MB; the analysis column has no such floor.

| tree | source KB | cold analysis ms (per MB) | cold process ms | warm analysis ms (per MB) | warm process ms | cold peak RSS MB (analysis / process) | warm peak RSS MB | stored bytes per MB (gzip) |
|---|---|---|---|---|---|---|---|---|
| constructs (all cases) | 13 | 93 (7271) | 208 | 49 (3831) | 159 | 73 / 73 | 73 / 73 | 112394214 (10946990) |
| axios | 282 | 633 (2245) | 750 | 417 (1479) | 538 | 100 / 100 | 101 / 101 | 75833297 (5409069) |
| zod | 3088 | 10792 (3495) | 10984 | 10818 (3504) | 11002 | 1061 / 1061 | 1041 / 1041 | 97353526 (6935648) |
| commander | 127 | 247 (1951) | 357 | 175 (1382) | 290 | 73 / 73 | 73 / 73 | 44266921 (3200439) |
| async | 321 | 442 (1379) | 563 | 297 (927) | 412 | 78 / 78 | 78 / 78 | 44436456 (3481001) |
| chalk | 10 | 43 (4516) | 157 | 23 (2416) | 134 | 73 / 73 | 73 / 73 | 87413717 (6979414) |
| dayjs | 7 | 39 (5446) | 150 | 18 (2514) | 128 | 73 / 73 | 73 / 73 | 222145650 (12890518) |
| underscore | 69 | 207 (3004) | 322 | 151 (2191) | 269 | 73 / 73 | 73 / 73 | 91434297 (5881609) |

Fact dump (no cache, one tree per process; peak RSS is the process figure, with the same floor): this is
the only way to time the refined plan, because no public view asks for it.

| tree | base ms | base peak RSS MB | refined ms | refined peak RSS MB | budget s |
|---|---|---|---|---|---|
| axios | 618 | 119 | 2247 | 285 | none |
| zod | 13281 | 1344 | 86311 | 3898 | 180 |
| commander | 291 | 61 | 1762 | 145 | none |
| async | 441 | 93 | 3067 | 268 | none |
| chalk | 49 | 31 | 94 | 41 | none |
| dayjs | 44 | 31 | 138 | 36 | none |
| underscore | 379 | 53 | 1221 | 121 | none |

## Decisions recorded by this run

- Construct edges are identified by (file, line of the call expression's start, callee label); labels cannot express how many sites on one line call the same callee, so precision is counted per (line, callee).
- When one line labels a callee both as a direct call and as a callback, the callback counts as found only when a second distinct call site on that line reaches it.
- Predicted edges to functions without an @fn label are counted apart and are neither true nor false positives.
- A labelled hidden path (@unknown) is 'reported-unknown' when a call site on that line carries an unresolved marker, 'false-complete' when it has resolved targets and no marker, and 'silent' when polint has no call site there or one with neither. For the refined column the refined layer's own status decides; sites it has no row for keep the base layer's marker.
- A labelled call line is false-complete when polint resolves a call site on it to labelled functions that miss a labelled `@calls` callee and no call site on the line carries an unresolved marker. Lines are the unit of the labels, so on a line with several call sites the targets and markers of all of them count together.
- A base edge whose target is an import binding rather than a function is a miss, counted as 'stops at import binding'.
- Observed package edges are mapped to polint by exact call-expression span (falling back to the polint call site in the same file ending at the same byte) and by callee span (the same span, else the innermost polint function in the same file ending at the same byte that does not start inside the observed span).
- Observed edges whose site or callee has no polint counterpart stay in the denominator: recall is over every observed edge.
- Export accuracy compares exported names per file; star re-export rows and the CommonJS whole-value `module.exports` row carry no name and are left out of the export-fact names but counted by status.
- Refined edges are counted when they carry a target function, whatever their status.
- Polint models a class as one function spanning the class body and has no separate constructor function, so a `Class.constructor` label and an observed constructor callee map to the class function.
- Observed implicit edges (accessors, coercions, iterator protocol) have a position but no call expression; they are reported by kind but left out of the headline package recall, which is over calls and callbacks.
- Packages are analysed with polint's default discovery, so its default excludes (for example `**/vendor/**`) apply; observed edges touching such files count as 'site or callee in a file polint does not analyse'.
- Missing call sites and functions are not worked around: functions under a unary operator, functions declared inside try, catch or if blocks, and calls in exported variable initializers are reported as misses.
- An observed callee maps to the polint function with the same span before the innermost one ending at the same byte: a concise arrow returning an arrow, `(a) => (b) => ...`, ends at the same byte as the arrow it returns.
- Every missed observed edge is attributed to one gap from a syntax census: Oxc's own function and call spans with their syntactic context, parsed again apart from the analysis. A site or callee polint does not extract is charged to the outermost function around it that polint does not extract, else to the outermost enclosing function or the call in a gap; edges polint maps go to constructors, callbacks to built-ins, computed keys and prototype or class methods, in that order. A computed-key miss is counted whether or not the key set is known, so that row is an upper bound for computed keys over a known set.
- A census function counts as extracted when polint has a function in the same file ending at the same byte, or, for a class constructor, when polint has the class.
- Every package edge with other targets and no unresolved marker is kept as an example; other outcomes and gaps keep a bounded sample, in observed-edge order.
- Process peak RSS is measured with `wait4` like the matrix runner does. Linux carries the parent's resident size into the child's peak at fork, so every process is launched before the report loads any fact dump, and the launcher's own peak is recorded with the cost section. The in-process analysis peak has no such floor.
- Each fact dump has a wall-clock limit (`--dump-timeout`, 600 s by default); a tree whose dump exceeds it is reported as timed out for that plan and has no scores in that column.
- A package with `dump_budget_s` in `inputs.toml` has a time budget for each of its fact dumps; the report is written either way, and the run exits with status 1 naming every dump that ran over its budget or timed out.
