# JS/TS analysis benchmark

polint 0.5.3. Written by `python3 tests/js-analysis-bench/bench.py report`;
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
| same-name-member | 2 (1) | 2/4 (50%) | 2/4 (50%) | – | – | 2/4 (50%) | 2/4 (50%) | – | – | 0 / 0 | 0 |

**False-complete count** (a labelled hidden path where polint gives a resolved answer with no unresolved marker): base 0, refined 0, over 4 labelled hidden paths.

**False-complete call lines** (a labelled call line where polint gives resolved targets that miss a labelled callee, with no unresolved marker on the line): base 0, refined 0, over 87 labelled call lines.

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
| class-method/instance-and-static | no | 2/5 (40%) | 5/5 (100%) | – | – | 2/2 (100%) | 5/5 (100%) | – | – | 83% | yes |
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
| same-name-member/cjs-static-method | yes | 1/2 (50%) | 1/2 (50%) | – | – | 1/2 (50%) | 1/2 (50%) | – | – | 67% | yes |
| same-name-member/wrapper-of-static-method | no | 1/2 (50%) | 1/2 (50%) | – | – | 1/2 (50%) | 1/2 (50%) | – | – | 50% | yes |

## Packages (observed calls: recall only)

Recall is over observed edges at call expressions (`call` and `callback`); `implicit` calls (getters,
coercions, iterator and thenable protocol) have no call expression and are listed by kind only.

Coverage is the share of the analysed files' functions (syntax census, classes and static blocks left out)
that the recording entered.

| package | format | files analysed / selected | source KB | coverage | observed edges | recall base | recall refined | stops at import binding (base) | not mappable to a polint site or function | unresolved call share base | call sites without target base / refined | unresolved reference share |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| axios 1.21.1 | ESM source, Node and browser adapters | 72 / 72 | 282 | 300/534 (56%) | 675 | 270/675 (40%) | 270/675 (40%) | 1 | 20 | 67% | 74% / 74% | 9% |
| zod 4.6.7 | TypeScript ESM source | 125 / 125 | 1354 | 566/2469 (23%) (flagged) | 1308 | 719/1308 (55%) | 719/1308 (55%) | 0 | 9 | 54% | 57% / 57% | 5% |
| commander 14.0.3 | CommonJS only | 7 / 7 | 127 | 218/296 (74%) | 318 | 21/318 (7%) | 119/318 (37%) | 0 | 1 | 92% | 95% / 77% | 10% |
| async 3.2.6 | CommonJS transpiled from ESM | 129 / 129 | 321 | 308/504 (61%) | 637 | 324/637 (51%) | 328/637 (51%) | 0 | 1 | 63% | 73% / 72% | 27% |
| chalk 6.0.1 | ESM only, with subpath imports | 2 / 5 | 10 | 25/28 (89%) | 72 | 17/72 (24%) | 17/72 (24%) | 0 | 47 | 52% | 70% / 70% | 9% |
| dayjs 1.11.23 | minified UMD | 1 / 1 | 7 | 55/57 (96%) | 117 | 0/117 (0%) | 0/117 (0%) | 0 | 117 | 100% | 100% / 100% | 7% |
| underscore 1.13.8 | bundled UMD (Rollup) | 1 / 1 | 69 | 159/186 (85%) | 262 | 157/262 (60%) | 158/262 (60%) | 0 | 0 | 56% | 58% / 58% | 6% |

### Claimed targets the run never called

A claimed target is resolved and not heuristic. It counts here when the run executed the site, the target is
in an instrumented file, and the run never called it from that site. On a **single-target** answer that is a
false precise edge: every execution of the site went elsewhere. On a **multi-target** answer it is an upper
bound only, since the run may not have taken the paths to the other targets. A site matched to a polint site
by its end byte can also inflate both counts.

| package | base: single / multi | refined: single / multi |
|---|---|---|
| axios | 0 / 0 | 0 / 0 |
| zod | 0 / 0 | 0 / 0 |
| commander | 0 / 0 | 0 / 0 |
| async | 0 / 0 | 0 / 0 |
| chalk | 0 / 0 | 0 / 0 |
| dayjs | 0 / 0 | 0 / 0 |
| underscore | 0 / 0 | 0 / 0 |
| **total** | **0 / 0** | **0 / 0** |

| package | plan | site | site text | single-target answer | the run called |
|---|---|---|---|---|---|
| none | | | | | |

The typed plan (refined with the TypeScript type sidecar) was not run: no TypeScript compiler was given.

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

| package | callee not a polint function | found | no target, marked unresolved | no target, no unresolved marker | site not a polint call site | site or callee in a file polint does not analyse | stops at import binding |
|---|---|---|---|---|---|---|---|
| axios | 9 / 9 | 270 / 270 | 376 / 376 | 8 / 9 | 11 / 11 | 0 / 0 | 1 / 0 |
| zod | 6 / 6 | 719 / 719 | 579 / 579 | 1 / 1 | 3 / 3 | 0 / 0 | 0 / 0 |
| commander | 1 / 1 | 21 / 119 | 296 / 198 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| async | 1 / 1 | 324 / 328 | 312 / 308 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| chalk | 0 / 0 | 17 / 17 | 8 / 8 | 0 / 0 | 0 / 0 | 47 / 47 | 0 / 0 |
| dayjs | 1 / 1 | 0 / 0 | 0 / 0 | 0 / 0 | 116 / 116 | 0 / 0 | 0 / 0 |
| underscore | 0 / 0 | 157 / 158 | 105 / 104 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |

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
| commander | base | callee not a polint function | lib/help.js:331 `option.argChoices.map((choice) => JSON.stringify(choice))` | member / callback | lib/help.js:331 <anonymous> | no target, marked unresolved |
| commander | base | found | lib/command.js:649 `knownBy(command)` | identifier / call | lib/command.js:645 knownBy | targets lib/command.js:645 <polint:anonymous:20982:21047>, no unresolved marker |
| commander | base | no target, marked unresolved | index.js:7 `new Command()` | new / call | lib/command.js:20 constructor | no target, marked unresolved |
| commander | refined | callee not a polint function | lib/help.js:331 `option.argChoices.map((choice) => JSON.stringify(choice))` | member / callback | lib/help.js:331 <anonymous> | no target, marked unresolved |
| commander | refined | found | index.js:7 `new Command()` | new / call | lib/command.js:20 constructor | targets lib/command.js:13 Command, marked unresolved |
| commander | refined | no target, marked unresolved | lib/argument.js:144 `arg.name()` | member / call | lib/argument.js:48 name | no target, marked unresolved |
| async | base | callee not a polint function | memoize.js:67 `hasher(...args)` | identifier / call | memoize.js:62 <anonymous> | no target, marked unresolved |
| async | base | found | applyEach.js:9 `_interopRequireDefault(_applyEach)` | identifier / call | applyEach.js:15 _interopRequireDefault | targets applyEach.js:15 _interopRequireDefault, no unresolved marker |
| async | base | no target, marked unresolved | applyEach.js:56 `(0, _applyEach2.default)(_map2.default)` | expression / call | internal/applyEach.js:7 default | no target, marked unresolved |
| async | refined | callee not a polint function | memoize.js:67 `hasher(...args)` | identifier / call | memoize.js:62 <anonymous> | no target, marked unresolved |
| async | refined | found | applyEach.js:9 `_interopRequireDefault(_applyEach)` | identifier / call | applyEach.js:15 _interopRequireDefault | targets applyEach.js:15 _interopRequireDefault, no unresolved marker |
| async | refined | no target, marked unresolved | applyEach.js:56 `(0, _applyEach2.default)(_map2.default)` | expression / call | internal/applyEach.js:7 default | no target, marked unresolved |
| chalk | base | found | source/index.js:37 `assertValidLevel(options.level)` | identifier / call | source/index.js:17 assertValidLevel | targets source/index.js:17 assertValidLevel, no unresolved marker |
| chalk | base | no target, marked unresolved | source/index.js:125 `converters[this.level](first, second, third)` | computed-member / call | source/index.js:91 ansi | no target, marked unresolved |
| chalk | base | site or callee in a file polint does not analyse | source/index.js:91 `style.ansi(ansiStyles.rgbToAnsi(red, green, blue))` | member / call | source/vendor/ansi-styles/index.js:4 <anonymous> | no target, marked unresolved |
| chalk | refined | found | source/index.js:37 `assertValidLevel(options.level)` | identifier / call | source/index.js:17 assertValidLevel | targets source/index.js:17 assertValidLevel, no unresolved marker |
| chalk | refined | no target, marked unresolved | source/index.js:125 `converters[this.level](first, second, third)` | computed-member / call | source/index.js:91 ansi | no target, marked unresolved |
| chalk | refined | site or callee in a file polint does not analyse | source/index.js:91 `style.ansi(ansiStyles.rgbToAnsi(red, green, blue))` | member / call | source/vendor/ansi-styles/index.js:4 <anonymous> | no target, marked unresolved |
| dayjs | base | callee not a polint function | dayjs.min.js:1 `function(t,e){"object"==typeof exports&&"undefined"!=typeof module?module.exports=e():"fu…` | expression / call | dayjs.min.js:1 <anonymous> | no target, marked unresolved |
| dayjs | base | site not a polint call site | dayjs.min.js:1 `m(r,2,"0")` | identifier / call | dayjs.min.js:1 m | no call site |
| dayjs | refined | callee not a polint function | dayjs.min.js:1 `function(t,e){"object"==typeof exports&&"undefined"!=typeof module?module.exports=e():"fu…` | expression / call | dayjs.min.js:1 <anonymous> | no target, marked unresolved |
| dayjs | refined | site not a polint call site | dayjs.min.js:1 `m(r,2,"0")` | identifier / call | dayjs.min.js:1 m | no call site |
| underscore | base | found | underscore-umd.js:1 `function (global, factory) { typeof exports === 'object' && typeof module !== 'undefined'…` | expression / call | underscore-umd.js:1 <anonymous> | targets underscore-umd.js:1 <polint:anonymous:1:451>, marked unresolved |
| underscore | base | no target, marked unresolved | underscore-umd.js:304 `getLength(obj)` | identifier / call | underscore-umd.js:223 <anonymous> | no target, marked unresolved |
| underscore | refined | found | underscore-umd.js:1 `function (global, factory) { typeof exports === 'object' && typeof module !== 'undefined'…` | expression / call | underscore-umd.js:1 <anonymous> | targets underscore-umd.js:1 <polint:anonymous:1:451>, marked unresolved |
| underscore | refined | no target, marked unresolved | underscore-umd.js:304 `getLength(obj)` | identifier / call | underscore-umd.js:223 <anonymous> | no target, marked unresolved |

## Tiers

Recall over the same observed edges as above, split by the tier of the edge that found the callee
(`research/package-graphs/TIERS.md`): `Exact` and `SetupAware` are exact, except typed-hierarchy edges
that dispatch to an `implementation` or `union_member`, which are candidates and count as conservative;
`Conservative` and `Ambiguous` are conservative; `Heuristic` is apart and never counts in the first three
columns. **exact-complete** also needs the site to be complete: at least one exact or conservative target and
no unresolved row.

| package | plan | observed edges | exact-complete | exact | exact + conservative | apart only | apart edges |
|---|---|---|---|---|---|---|---|
| axios | base | 675 | 259/675 (38%) | 259/675 (38%) | 259/675 (38%) | 11/675 (2%) | 66 |
| axios | refined | 675 | 259/675 (38%) | 259/675 (38%) | 259/675 (38%) | 11/675 (2%) | 132 |
| zod | base | 1308 | 668/1308 (51%) | 668/1308 (51%) | 668/1308 (51%) | 51/1308 (4%) | 160 |
| zod | refined | 1308 | 668/1308 (51%) | 668/1308 (51%) | 668/1308 (51%) | 51/1308 (4%) | 320 |
| commander | base | 318 | 21/318 (7%) | 21/318 (7%) | 21/318 (7%) | 0/318 (0%) | 0 |
| commander | refined | 318 | 21/318 (7%) | 21/318 (7%) | 21/318 (7%) | 98/318 (31%) | 161 |
| async | base | 637 | 324/637 (51%) | 324/637 (51%) | 324/637 (51%) | 0/637 (0%) | 0 |
| async | refined | 637 | 324/637 (51%) | 324/637 (51%) | 324/637 (51%) | 4/637 (1%) | 8 |
| chalk | base | 72 | 17/72 (24%) | 17/72 (24%) | 17/72 (24%) | 0/72 (0%) | 0 |
| chalk | refined | 72 | 17/72 (24%) | 17/72 (24%) | 17/72 (24%) | 0/72 (0%) | 1 |
| dayjs | base | 117 | 0/117 (0%) | 0/117 (0%) | 0/117 (0%) | 0/117 (0%) | 0 |
| dayjs | refined | 117 | 0/117 (0%) | 0/117 (0%) | 0/117 (0%) | 0/117 (0%) | 0 |
| underscore | base | 262 | 154/262 (59%) | 154/262 (59%) | 154/262 (59%) | 3/262 (1%) | 4 |
| underscore | refined | 262 | 154/262 (59%) | 154/262 (59%) | 154/262 (59%) | 4/262 (2%) | 10 |
| **total** | base | 3389 | 1443/3389 (43%) | 1443/3389 (43%) | 1443/3389 (43%) | 65/3389 (2%) | 230 |
| **total** | refined | 3389 | 1443/3389 (43%) | 1443/3389 (43%) | 1443/3389 (43%) | 168/3389 (5%) | 632 |

Targets per site that claims any in the tier: mean / p95 (nearest rank) / max.

| package | plan | exact | conservative | exact + conservative | apart |
|---|---|---|---|---|---|
| axios | base | 1.0 / 1 / 1 (482 sites) | – | 1.0 / 1 / 1 (482 sites) | 1.0 / 1 / 1 (66 sites) |
| axios | refined | 1.0 / 1 / 1 (482 sites) | – | 1.0 / 1 / 1 (482 sites) | 1.0 / 1 / 1 (66 sites) |
| zod | base | 1.0 / 1 / 1 (2423 sites) | – | 1.0 / 1 / 1 (2423 sites) | 1.0 / 1 / 1 (160 sites) |
| zod | refined | 1.0 / 1 / 1 (2423 sites) | – | 1.0 / 1 / 1 (2423 sites) | 1.0 / 1 / 1 (160 sites) |
| commander | base | 1.0 / 1 / 1 (42 sites) | – | 1.0 / 1 / 1 (42 sites) | – |
| commander | refined | 1.0 / 1 / 1 (42 sites) | – | 1.0 / 1 / 1 (42 sites) | 1.0 / 1 / 1 (161 sites) |
| async | base | 1.0 / 1 / 1 (440 sites) | – | 1.0 / 1 / 1 (440 sites) | – |
| async | refined | 1.0 / 1 / 1 (440 sites) | – | 1.0 / 1 / 1 (440 sites) | 1.0 / 1 / 1 (8 sites) |
| chalk | base | 1.0 / 1 / 1 (18 sites) | – | 1.0 / 1 / 1 (18 sites) | – |
| chalk | refined | 1.0 / 1 / 1 (18 sites) | – | 1.0 / 1 / 1 (18 sites) | 1.0 / 1 / 1 (1 sites) |
| dayjs | base | – | – | – | – |
| dayjs | refined | – | – | – | – |
| underscore | base | 1.0 / 1 / 1 (187 sites) | – | 1.0 / 1 / 1 (187 sites) | 1.0 / 1 / 1 (4 sites) |
| underscore | refined | 1.0 / 1 / 1 (187 sites) | – | 1.0 / 1 / 1 (187 sites) | 1.0 / 1 / 1 (6 sites) |

## Precision by tier

At executed sites only: a site counts once the run was seen calling a polint function from it, and a
claimed target counts when it is a function in an instrumented file. **precision** is the share of those
targets the run called there; it is a lower bound, since a run takes only some paths. A target the run never
called on a single-target answer in the exact tier is a **wrong exact target**; the gate is 0.

| package | plan | exact: precision, single / multi not called | conservative: precision, single / multi not called | apart: precision, single / multi not called |
|---|---|---|---|---|
| axios | base | 259/259 (100%), 0 / 0 | – | 11/11 (100%), 0 / 0 |
| axios | refined | 259/259 (100%), 0 / 0 | – | 11/11 (100%), 0 / 0 |
| zod | base | 668/668 (100%), 0 / 0 | – | 51/51 (100%), 0 / 0 |
| zod | refined | 668/668 (100%), 0 / 0 | – | 51/51 (100%), 0 / 0 |
| commander | base | 21/21 (100%), 0 / 0 | – | – |
| commander | refined | 21/21 (100%), 0 / 0 | – | 107/107 (100%), 0 / 0 |
| async | base | 324/324 (100%), 0 / 0 | – | – |
| async | refined | 324/324 (100%), 0 / 0 | – | 5/5 (100%), 0 / 0 |
| chalk | base | 17/17 (100%), 0 / 0 | – | – |
| chalk | refined | 17/17 (100%), 0 / 0 | – | 1/1 (100%), 0 / 0 |
| dayjs | base | – | – | – |
| dayjs | refined | – | – | – |
| underscore | base | 154/154 (100%), 0 / 0 | – | 3/3 (100%), 0 / 0 |
| underscore | refined | 154/154 (100%), 0 / 0 | – | 4/4 (100%), 0 / 0 |
| **total** | base | **1443/1443 (100%), 0 / 0** | – | **65/65 (100%), 0 / 0** |
| **total** | refined | **1443/1443 (100%), 0 / 0** | – | **179/179 (100%), 0 / 0** |

| package | plan | wrong exact target at | site text | target | the run called |
|---|---|---|---|---|---|
| none | | | | | |

## Open sites

A site is open when it has no exact or conservative target, or the answer marks it unresolved. Each open
site counts under every reason its answer gives; `none recorded` is an open site whose answer gives none,
which the engine must not produce.

| package | plan | call sites | complete | open | open with a claimed target | open by reason |
|---|---|---|---|---|---|---|
| axios | base | 2075 | 482/2075 (23%) | 1593 | 0 | DynamicProperty 852, MissingSemanticReference 513, none recorded 194, CallApplyBind 34 |
| axios | refined | 2075 | 482/2075 (23%) | 1593 | 0 | DynamicProperty 852, MissingSemanticReference 513, none recorded 194, CallApplyBind 34 |
| zod | base | 5967 | 2423/5967 (41%) | 3544 | 0 | DynamicProperty 2252, MissingSemanticReference 929, none recorded 321, CallApplyBind 42, Reflection 1 |
| zod | refined | 5967 | 2423/5967 (41%) | 3544 | 0 | DynamicProperty 2252, MissingSemanticReference 929, none recorded 321, CallApplyBind 42, Reflection 1 |
| commander | base | 809 | 42/809 (5%) | 767 | 0 | DynamicProperty 588, MissingSemanticReference 153, none recorded 25, CallApplyBind 1 |
| commander | refined | 809 | 42/809 (5%) | 767 | 0 | DynamicProperty 588, MissingSemanticReference 153, none recorded 25, CallApplyBind 1 |
| async | base | 1605 | 440/1605 (27%) | 1165 | 0 | MissingSemanticReference 900, none recorded 156, DynamicProperty 95, CallApplyBind 14 |
| async | refined | 1605 | 440/1605 (27%) | 1165 | 0 | MissingSemanticReference 900, none recorded 155, DynamicProperty 95, CallApplyBind 14, UnsupportedSyntax 1 |
| chalk | base | 60 | 18/60 (30%) | 42 | 0 | DynamicProperty 24, none recorded 11, MissingSemanticReference 7 |
| chalk | refined | 60 | 18/60 (30%) | 42 | 0 | DynamicProperty 24, none recorded 11, MissingSemanticReference 7 |
| dayjs | base | 1 | 0/1 (0%) | 1 | 0 | MissingSemanticReference 1 |
| dayjs | refined | 1 | 0/1 (0%) | 1 | 0 | MissingSemanticReference 1 |
| underscore | base | 455 | 187/455 (41%) | 268 | 0 | MissingSemanticReference 169, DynamicProperty 44, CallApplyBind 41, none recorded 14 |
| underscore | refined | 455 | 187/455 (41%) | 268 | 0 | MissingSemanticReference 169, DynamicProperty 44, CallApplyBind 41, none recorded 14 |

## Budget events

The solver's budget status for each dump and every budget it reported as reached. The target is none on
the tune set, and a budget event must never discard an edge already derived.

| package | plan | status | reasons |
|---|---|---|---|
| axios | base | NotRun | – |
| axios | refined | BudgetExceeded | points_to.max_dynamic_vars |
| zod | base | NotRun | – |
| zod | refined | BudgetExceeded | points_to.max_dynamic_vars, solver.max_steps |
| commander | base | NotRun | – |
| commander | refined | WithinBudget | – |
| async | base | NotRun | – |
| async | refined | WithinBudget | – |
| chalk | base | NotRun | – |
| chalk | refined | WithinBudget | – |
| dayjs | base | NotRun | – |
| dayjs | refined | WithinBudget | – |
| underscore | base | NotRun | – |
| underscore | refined | WithinBudget | – |

## Refined edges

Every refined row by the tier the bench gives it, its algorithm, status and precision, and whether it
names a function. Rows with a status other than `Resolved` or `Ambiguous` claim nothing.

| package | plan | tier | algorithm | status | precision | names a function | rows |
|---|---|---|---|---|---|---|---|
| axios | refined | none | PointsTo | Unresolved | Unknown | no | 1399 |
| axios | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 482 |
| axios | refined | exact | DirectReference | Resolved | SetupAware | yes | 384 |
| axios | refined | exact | ImportBinding | Resolved | SetupAware | yes | 92 |
| axios | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 66 |
| axios | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 66 |
| axios | refined | exact | ImportBinding | Resolved | SetupAware | no | 8 |
| axios | refined | exact | SummaryAssisted | Resolved | SetupAware | no | 8 |
| axios | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 6 |
| zod | refined | none | PointsTo | Unresolved | Unknown | no | 3223 |
| zod | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 2423 |
| zod | refined | exact | ImportBinding | Resolved | SetupAware | yes | 1285 |
| zod | refined | exact | DirectReference | Resolved | SetupAware | yes | 1013 |
| zod | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 160 |
| zod | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 160 |
| zod | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 125 |
| zod | refined | exact | ImportBinding | Resolved | SetupAware | no | 1 |
| zod | refined | exact | SummaryAssisted | Resolved | SetupAware | no | 1 |
| zod | refined | none | PointsTo | Unresolved | Unsupported | no | 1 |
| commander | refined | none | PointsTo | Unresolved | Unknown | no | 599 |
| commander | refined | apart | PointsTo | Resolved | Heuristic | yes | 161 |
| commander | refined | none | SyntaxOnly | Unresolved | Unknown | no | 143 |
| commander | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 42 |
| commander | refined | exact | DirectReference | Resolved | SetupAware | yes | 41 |
| commander | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 1 |
| async | refined | none | PointsTo | Unresolved | Unknown | no | 1003 |
| async | refined | exact | DirectReference | Resolved | SetupAware | yes | 440 |
| async | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 440 |
| async | refined | apart | PointsTo | Resolved | Heuristic | yes | 8 |
| async | refined | none | SyntaxOnly | Unresolved | Unknown | no | 6 |
| async | refined | none | FrameworkModel | Unsupported | Conservative | no | 1 |
| chalk | refined | none | PointsTo | Unresolved | Unknown | no | 31 |
| chalk | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 18 |
| chalk | refined | exact | DirectReference | Resolved | SetupAware | yes | 16 |
| chalk | refined | exact | ImportBinding | Resolved | SetupAware | yes | 2 |
| chalk | refined | exact | PointsTo | Resolved | SetupAware | yes | 2 |
| chalk | refined | apart | PointsTo | Resolved | Heuristic | yes | 1 |
| dayjs | refined | none | PointsTo | Unresolved | Unknown | no | 1 |
| underscore | refined | none | PointsTo | Unresolved | Unknown | no | 252 |
| underscore | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 187 |
| underscore | refined | exact | DirectReference | Resolved | SetupAware | yes | 186 |
| underscore | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 4 |
| underscore | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 4 |
| underscore | refined | apart | PointsTo | Resolved | Heuristic | yes | 2 |
| underscore | refined | none | SyntaxOnly | Unresolved | Unknown | no | 2 |
| underscore | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 1 |

## Sites gained by the refined plan

Sites where a refined plan claims an exact or conservative target and the base plan claims none, by file.
**test path** marks a file in a test or benchmark folder or named `*.test.*` / `*.spec.*`; **executed** counts
the gained sites the run called a polint function from, the only ones recall can credit.

| package | plan | sites gained | in test paths | executed | top files |
|---|---|---|---|---|---|
| axios | refined | 0 | 0 | 0 | – |
| zod | refined | 0 | 0 | 0 | – |
| commander | refined | 0 | 0 | 0 | – |
| async | refined | 0 | 0 | 0 | – |
| chalk | refined | 0 | 0 | 0 | – |
| dayjs | refined | 0 | 0 | 0 | – |
| underscore | refined | 0 | 0 | 0 | – |

## Analysed set: globs alone against shipped source

The analysed set of every package is its shipped source: the `analysed` globs without test and benchmark
paths. Reports written before this rule analysed what the globs alone select; each package listed here is
scored both ways once, so this report reads against them. Every other table uses the shipped set.

| package | set | files | source KB | plan | recall | exact / exact + conservative | wrong targets single / multi | wrong exact targets | call sites | dump s | dump peak MB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| zod | globs alone | 332 | 3088 | base | 719/1308 (55%) | 668 / 668 | 0 / 0 | 0 | 48838 | 12.2 | 1364 |
| zod | globs alone | 332 | 3088 | refined | 719/1308 (55%) | 668 / 668 | 0 / 0 | 0 | 48838 | 45.8 | 4011 |
| zod | shipped | 125 | 1354 | base | 719/1308 (55%) | 668 / 668 | 0 / 0 | 0 | 5967 | 3.7 | 534 |
| zod | shipped | 125 | 1354 | refined | 719/1308 (55%) | 668 / 668 | 0 / 0 | 0 | 5967 | 12.2 | 1015 |

## Outside tools

Jelly at `62e60eb4fd5b991c5bf4cfc992f018984f560ea4`, run by `bench.py outside` in the bench sandbox and scored
from the committed outputs under `baseline/outside/` with the bench's site and callee matching.
Recall is over observed `call` and `callback` edges. Wrong targets are claimed targets at executed
sites that the run never called there, split by answers with one target and with several.

| package | polint base | polint refined | jelly-static | jelly-approx |
|---|---|---|---|---|
| async | 50.9% | 51.5% | 55.7% | 55.7% |
| axios | 40.0% | 40.0% | 89.2% | 89.3% |
| chalk | 23.6% | 23.6% | 86.1% | 86.1% |
| commander | 6.6% | 37.4% | 96.5% | 96.5% |
| dayjs | 0.0% | 0.0% | 77.8% | 87.2% |
| underscore | 59.9% | 60.3% | 96.2% | 98.9% |
| zod | 55.0% | 55.0% | 91.3% | 91.7% |
| **total** | **44.5%** | **47.5%** | **84.5%** | **85.2%** |

### jelly-static

| package | observed | found | recall | functions | calls | edges | targets per site, mean / p95 / max | wrong single | wrong multi | executed sites | wall s | exit |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| async | 637 | 355 | 55.7% | 504 | 1610 | 14352 | 14.53 / 76 / 82 | 0 | 3679 | 571 | 1.3 | 0 |
| axios | 675 | 602 | 89.2% | 535 | 2281 | 1448 | 1.09 / 2 / 20 | 12 | 78 | 620 | 1.3 | 0 |
| chalk | 72 | 62 | 86.1% | 53 | 148 | 72 | 1.07 / 1 / 6 | 0 | 0 | 63 | 0.7 | 0 |
| commander | 318 | 307 | 96.5% | 296 | 814 | 517 | 1.03 / 1 / 5 | 0 | 4 | 311 | 1.8 | 0 |
| dayjs | 117 | 91 | 77.8% | 57 | 181 | 97 | 1.01 / 1 / 2 | 0 | 0 | 116 | 0.6 | 0 |
| underscore | 262 | 252 | 96.2% | 186 | 455 | 1120 | 3.37 / 26 / 27 | 1 | 330 | 233 | 0.8 | 0 |
| zod | 1308 | 1194 | 91.3% | 2421 | 6502 | 8375 | 1.83 / 1 / 252 | 0 | 1709 | 1075 | 4.2 | 0 |

### jelly-approx

| package | observed | found | recall | functions | calls | edges | targets per site, mean / p95 / max | wrong single | wrong multi | executed sites | wall s | exit |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| async | 637 | 355 | 55.7% | 504 | 1610 | 14352 | 14.53 / 76 / 82 | 0 | 3679 | 571 | 3.1 | 0 |
| axios | 675 | 603 | 89.3% | 535 | 2304 | 1474 | 1.09 / 2 / 20 | 12 | 78 | 620 | 3.2 | 0 |
| chalk | 72 | 62 | 86.1% | 53 | 148 | 72 | 1.07 / 1 / 6 | 0 | 0 | 63 | 1.3 | 0 |
| commander | 318 | 307 | 96.5% | 296 | 814 | 517 | 1.03 / 1 / 5 | 0 | 4 | 311 | 2.9 | 0 |
| dayjs | 117 | 102 | 87.2% | 57 | 181 | 108 | 1.01 / 1 / 2 | 0 | 0 | 116 | 1.2 | 0 |
| underscore | 262 | 259 | 98.9% | 186 | 455 | 2232 | 6.38 / 34 / 116 | 0 | 638 | 233 | 1.6 | 0 |
| zod | 1308 | 1200 | 91.7% | 2421 | 6651 | 9079 | 1.91 / 1 / 252 | 0 | 1867 | 1075 | 9.6 | 0 |

Deviations from polint's scoring:

- Jelly has no tiers and no notion of a complete answer or of a heuristic edge, so every Jelly edge counts as claimed, its edges to external callbacks included. Its wrong-target counts are an upper bound and are not comparable with polint's exact tier.
- Jelly's per-file module entries are left out of callee matching, as polint's `module` functions are.
- Jelly is given the same shipped source files polint analyses, as entry files; `-b <package>` makes its paths relative to the package root.
- Jelly models `_interopRequireDefault` and similar helpers natively instead of analysing their bodies; polint analyses the helper as code. On async this alone accounts for most of Jelly's misses.
- polint's outcome classes ("stops at import binding", "marked unresolved") have no counterpart in Jelly's output.
- Wall time is the docker wall time of one run, container start included, on the machine that recorded the committed outputs; it is not measured with polint's cost runs.
- ACG (js-callgraph) is not run: its parser rejects ES2020 and drops such files silently, so it cannot analyse these packages.

## Held-out packages

Packages no analysis change may be tuned on (`research/package-graphs/HELD-OUT.md`). They get the tune
set's measures with no gap ranking, no cause and no example of a missed edge. Wrong exact targets and
answers with other targets and no unresolved marker keep their examples: either is a defect to fix.
A package whose recording entered under 40% of its functions is flagged and left out
of the held-out recall.

| package | format | files analysed / selected | source KB | census functions | entered | coverage | observed edges | recall base | recall refined |
|---|---|---|---|---|---|---|---|---|---|
| co 4.6.0 | CommonJS, generator and promise control flow | 1 / 1 | 5 | 20 | 20 | 20/20 (100%) | 22 | 12/22 (55%) | 12/22 (55%) |
| pako 2.1.0 | minified UMD (Rollup and terser) | 1 / 1 | 47 | 99 | 94 | 94/99 (95%) | 258 | 0/258 (0%) | 0/258 (0%) |
| nanostores 1.5.5 | ESM source | 14 / 14 | 18 | 97 | 91 | 91/97 (94%) | 103 | 32/103 (31%) | 32/103 (31%) |
| express 5.3.0 | CommonJS framework with dependencies | 7 / 7 | 62 | 109 | 109 | 109/109 (100%) | 166 | 22/166 (13%) | 48/166 (29%) |
| immer 11.2.10 | TypeScript source | 17 / 17 | 96 | 181 | 142 | 142/181 (78%) | 295 | 220/295 (75%) | 220/295 (75%) |
| js-yaml 4.1.0 | CommonJS source, table-driven schema compilation | 25 / 25 | 110 | 146 | 143 | 143/146 (98%) | 340 | 229/340 (67%) | 263/340 (77%) |
| acorn 8.19.0 | bundled ESM output | 1 / 1 | 235 | 358 | 333 | 333/358 (93%) | 1141 | 176/1141 (15%) | 182/1141 (16%) |
| node-forge 1.4.3 | CommonJS, large | 49 / 49 | 972 | 960 | 649 | 649/960 (68%) | 3131 | 577/3131 (18%) | 577/3131 (18%) |

### Held-out gate

Recall per tier pooled over the tune set and over the unflagged held-out packages (acorn, co, express, immer, js-yaml, nanostores, node-forge, pako; flagged: none). The gate holds when held-out recall is at most 5 points below the tune set's in every tier,
and no held-out package, flagged or not, has a wrong exact target or an answer with other targets and no
unresolved marker.

| plan | tier | tune | held-out | points below tune | within |
|---|---|---|---|---|---|
| base | exact-complete | 42.6% | 23.1% | 19.5 | no |
| base | exact | 42.6% | 23.1% | 19.5 | no |
| base | exact + conservative | 42.6% | 23.1% | 19.5 | no |
| refined | exact-complete | 42.6% | 23.5% | 19.1 | no |
| refined | exact | 42.6% | 23.5% | 19.1 | no |
| refined | exact + conservative | 42.6% | 23.5% | 19.1 | no |

| plan | wrong exact targets | other targets, no unresolved marker |
|---|---|---|
| base | 0 | 0 |
| refined | 0 | 0 |

### Claimed targets the run never called

A claimed target is resolved and not heuristic. It counts here when the run executed the site, the target is
in an instrumented file, and the run never called it from that site. On a **single-target** answer that is a
false precise edge: every execution of the site went elsewhere. On a **multi-target** answer it is an upper
bound only, since the run may not have taken the paths to the other targets. A site matched to a polint site
by its end byte can also inflate both counts.

| package | base: single / multi | refined: single / multi |
|---|---|---|
| co | 0 / 0 | 0 / 0 |
| pako | 0 / 0 | 0 / 0 |
| nanostores | 0 / 0 | 0 / 0 |
| express | 0 / 0 | 0 / 0 |
| immer | 0 / 0 | 0 / 6 |
| js-yaml | 0 / 0 | 0 / 0 |
| acorn | 0 / 0 | 0 / 0 |
| node-forge | 0 / 0 | 0 / 0 |
| **total** | **0 / 0** | **0 / 6** |

| package | plan | site | site text | single-target answer | the run called |
|---|---|---|---|---|---|
| none | | | | | |

### Other targets and no unresolved marker (every case)

The observed callee is not among polint's targets for the site, and the site carries no unresolved marker.
The polint site column shows which call expression the observed site was matched to.

| package | plan | observed site | syntax / kind | observed callee | polint answer | observed site text | polint site text |
|---|---|---|---|---|---|---|---|
| none | | | | | | | |

## Tiers (held-out)

Recall over the same observed edges as above, split by the tier of the edge that found the callee
(`research/package-graphs/TIERS.md`): `Exact` and `SetupAware` are exact, except typed-hierarchy edges
that dispatch to an `implementation` or `union_member`, which are candidates and count as conservative;
`Conservative` and `Ambiguous` are conservative; `Heuristic` is apart and never counts in the first three
columns. **exact-complete** also needs the site to be complete: at least one exact or conservative target and
no unresolved row.

| package | plan | observed edges | exact-complete | exact | exact + conservative | apart only | apart edges |
|---|---|---|---|---|---|---|---|
| co | base | 22 | 12/22 (55%) | 12/22 (55%) | 12/22 (55%) | 0/22 (0%) | 0 |
| co | refined | 22 | 12/22 (55%) | 12/22 (55%) | 12/22 (55%) | 0/22 (0%) | 0 |
| pako | base | 258 | 0/258 (0%) | 0/258 (0%) | 0/258 (0%) | 0/258 (0%) | 0 |
| pako | refined | 258 | 0/258 (0%) | 0/258 (0%) | 0/258 (0%) | 0/258 (0%) | 0 |
| nanostores | base | 103 | 32/103 (31%) | 32/103 (31%) | 32/103 (31%) | 0/103 (0%) | 0 |
| nanostores | refined | 103 | 32/103 (31%) | 32/103 (31%) | 32/103 (31%) | 0/103 (0%) | 0 |
| express | base | 166 | 22/166 (13%) | 22/166 (13%) | 22/166 (13%) | 0/166 (0%) | 0 |
| express | refined | 166 | 24/166 (14%) | 24/166 (14%) | 24/166 (14%) | 24/166 (14%) | 25 |
| immer | base | 295 | 220/295 (75%) | 220/295 (75%) | 220/295 (75%) | 0/295 (0%) | 1 |
| immer | refined | 295 | 220/295 (75%) | 220/295 (75%) | 220/295 (75%) | 0/295 (0%) | 2 |
| js-yaml | base | 340 | 227/340 (67%) | 227/340 (67%) | 227/340 (67%) | 2/340 (1%) | 2 |
| js-yaml | refined | 340 | 247/340 (73%) | 247/340 (73%) | 247/340 (73%) | 16/340 (5%) | 18 |
| acorn | base | 1141 | 176/1141 (15%) | 176/1141 (15%) | 176/1141 (15%) | 0/1141 (0%) | 3 |
| acorn | refined | 1141 | 176/1141 (15%) | 176/1141 (15%) | 176/1141 (15%) | 6/1141 (1%) | 54 |
| node-forge | base | 3131 | 570/3131 (18%) | 570/3131 (18%) | 570/3131 (18%) | 7/3131 (0%) | 9 |
| node-forge | refined | 3131 | 570/3131 (18%) | 570/3131 (18%) | 570/3131 (18%) | 7/3131 (0%) | 18 |
| **total** | base | 5456 | 1259/5456 (23%) | 1259/5456 (23%) | 1259/5456 (23%) | 9/5456 (0%) | 15 |
| **total** | refined | 5456 | 1281/5456 (23%) | 1281/5456 (23%) | 1281/5456 (23%) | 53/5456 (1%) | 117 |

Targets per site that claims any in the tier: mean / p95 (nearest rank) / max.

| package | plan | exact | conservative | exact + conservative | apart |
|---|---|---|---|---|---|
| co | base | 1.0 / 1 / 1 (12 sites) | – | 1.0 / 1 / 1 (12 sites) | – |
| co | refined | 1.0 / 1 / 1 (12 sites) | – | 1.0 / 1 / 1 (12 sites) | – |
| pako | base | – | – | – | – |
| pako | refined | – | – | – | – |
| nanostores | base | 1.0 / 1 / 1 (36 sites) | – | 1.0 / 1 / 1 (36 sites) | – |
| nanostores | refined | 1.0 / 1 / 1 (36 sites) | – | 1.0 / 1 / 1 (36 sites) | – |
| express | base | 1.0 / 1 / 1 (24 sites) | – | 1.0 / 1 / 1 (24 sites) | – |
| express | refined | 1.0 / 1 / 1 (26 sites) | – | 1.0 / 1 / 1 (26 sites) | 1.0 / 1 / 1 (25 sites) |
| immer | base | 1.0 / 1 / 1 (299 sites) | – | 1.0 / 1 / 1 (299 sites) | 1.0 / 1 / 1 (1 sites) |
| immer | refined | 1.03 / 1 / 2 (299 sites) | – | 1.03 / 1 / 2 (299 sites) | 1.0 / 1 / 1 (1 sites) |
| js-yaml | base | 1.0 / 1 / 1 (254 sites) | – | 1.0 / 1 / 1 (254 sites) | 1.0 / 1 / 1 (2 sites) |
| js-yaml | refined | 1.0 / 1 / 1 (274 sites) | – | 1.0 / 1 / 1 (274 sites) | 1.0 / 1 / 1 (16 sites) |
| acorn | base | 1.0 / 1 / 1 (197 sites) | – | 1.0 / 1 / 1 (197 sites) | 1.0 / 1 / 1 (3 sites) |
| acorn | refined | 1.0 / 1 / 1 (197 sites) | – | 1.0 / 1 / 1 (197 sites) | 1.0 / 1 / 1 (51 sites) |
| node-forge | base | 1.0 / 1 / 1 (664 sites) | – | 1.0 / 1 / 1 (664 sites) | 1.0 / 1 / 1 (9 sites) |
| node-forge | refined | 1.0 / 1 / 1 (664 sites) | – | 1.0 / 1 / 1 (664 sites) | 1.0 / 1 / 1 (9 sites) |

## Precision by tier (held-out)

At executed sites only: a site counts once the run was seen calling a polint function from it, and a
claimed target counts when it is a function in an instrumented file. **precision** is the share of those
targets the run called there; it is a lower bound, since a run takes only some paths. A target the run never
called on a single-target answer in the exact tier is a **wrong exact target**; the gate is 0.

| package | plan | exact: precision, single / multi not called | conservative: precision, single / multi not called | apart: precision, single / multi not called |
|---|---|---|---|---|
| co | base | 12/12 (100%), 0 / 0 | – | – |
| co | refined | 12/12 (100%), 0 / 0 | – | – |
| pako | base | – | – | – |
| pako | refined | – | – | – |
| nanostores | base | 32/32 (100%), 0 / 0 | – | – |
| nanostores | refined | 32/32 (100%), 0 / 0 | – | – |
| express | base | 22/22 (100%), 0 / 0 | – | – |
| express | refined | 24/24 (100%), 0 / 0 | – | 24/24 (100%), 0 / 0 |
| immer | base | 220/220 (100%), 0 / 0 | – | – |
| immer | refined | 220/226 (97%), 0 / 6 | – | – |
| js-yaml | base | 227/227 (100%), 0 / 0 | – | 2/2 (100%), 0 / 0 |
| js-yaml | refined | 247/247 (100%), 0 / 0 | – | 16/16 (100%), 0 / 0 |
| acorn | base | 176/176 (100%), 0 / 0 | – | 0/3 (0%), 3 / 0 |
| acorn | refined | 176/176 (100%), 0 / 0 | – | 47/50 (94%), 3 / 0 |
| node-forge | base | 570/570 (100%), 0 / 0 | – | 7/7 (100%), 0 / 0 |
| node-forge | refined | 570/570 (100%), 0 / 0 | – | 7/7 (100%), 0 / 0 |
| **total** | base | **1259/1259 (100%), 0 / 0** | – | **9/12 (75%), 3 / 0** |
| **total** | refined | **1281/1287 (100%), 0 / 6** | – | **94/97 (97%), 3 / 0** |

| package | plan | wrong exact target at | site text | target | the run called |
|---|---|---|---|---|---|
| none | | | | | |

## Open sites (held-out)

A site is open when it has no exact or conservative target, or the answer marks it unresolved. Each open
site counts under every reason its answer gives; `none recorded` is an open site whose answer gives none,
which the engine must not produce.

| package | plan | call sites | complete | open | open with a claimed target | open by reason |
|---|---|---|---|---|---|---|
| co | base | 46 | 12/46 (26%) | 34 | 0 | CallApplyBind 12, MissingSemanticReference 11, DynamicProperty 7, none recorded 4 |
| co | refined | 46 | 12/46 (26%) | 34 | 0 | CallApplyBind 12, MissingSemanticReference 11, DynamicProperty 7, none recorded 4 |
| pako | base | 1 | 0/1 (0%) | 1 | 0 | MissingSemanticReference 1 |
| pako | refined | 1 | 0/1 (0%) | 1 | 0 | MissingSemanticReference 1 |
| nanostores | base | 162 | 36/162 (22%) | 126 | 0 | MissingSemanticReference 60, DynamicProperty 56, none recorded 10 |
| nanostores | refined | 162 | 36/162 (22%) | 126 | 0 | MissingSemanticReference 60, DynamicProperty 56, none recorded 10 |
| express | base | 463 | 24/463 (5%) | 439 | 0 | DynamicProperty 192, MissingSemanticReference 184, none recorded 52, CallApplyBind 11, UnknownCallee 1 |
| express | refined | 463 | 26/463 (6%) | 437 | 0 | DynamicProperty 192, MissingSemanticReference 182, none recorded 52, CallApplyBind 11, UnknownCallee 1 |
| immer | base | 570 | 299/570 (52%) | 271 | 0 | DynamicProperty 108, MissingSemanticReference 106, none recorded 39, CallApplyBind 18 |
| immer | refined | 570 | 299/570 (52%) | 271 | 0 | DynamicProperty 108, MissingSemanticReference 106, none recorded 39, CallApplyBind 18 |
| js-yaml | base | 709 | 254/709 (36%) | 455 | 0 | DynamicProperty 212, MissingSemanticReference 159, none recorded 55, CallApplyBind 29 |
| js-yaml | refined | 709 | 274/709 (39%) | 435 | 0 | DynamicProperty 193, MissingSemanticReference 158, none recorded 55, CallApplyBind 29 |
| acorn | base | 1809 | 197/1809 (11%) | 1612 | 0 | DynamicProperty 1539, MissingSemanticReference 47, none recorded 19, CallApplyBind 6, DynamicImport 1 |
| acorn | refined | 1809 | 197/1809 (11%) | 1612 | 0 | DynamicProperty 1539, MissingSemanticReference 47, none recorded 19, CallApplyBind 6, DynamicImport 1 |
| node-forge | base | 5278 | 664/5278 (13%) | 4614 | 0 | DynamicProperty 3291, MissingSemanticReference 1077, none recorded 231, CallApplyBind 15 |
| node-forge | refined | 5278 | 664/5278 (13%) | 4614 | 0 | DynamicProperty 3291, MissingSemanticReference 1077, none recorded 231, CallApplyBind 15 |

## Budget events (held-out)

The solver's budget status for each dump and every budget it reported as reached. The target is none on
the tune set, and a budget event must never discard an edge already derived.

| package | plan | status | reasons |
|---|---|---|---|
| co | base | NotRun | – |
| co | refined | WithinBudget | – |
| pako | base | NotRun | – |
| pako | refined | WithinBudget | – |
| nanostores | base | NotRun | – |
| nanostores | refined | WithinBudget | – |
| express | base | NotRun | – |
| express | refined | WithinBudget | – |
| immer | base | NotRun | – |
| immer | refined | WithinBudget | – |
| js-yaml | base | NotRun | – |
| js-yaml | refined | WithinBudget | – |
| acorn | base | NotRun | – |
| acorn | refined | WithinBudget | – |
| node-forge | base | NotRun | – |
| node-forge | refined | BudgetExceeded | points_to.max_dynamic_vars |

## Refined edges (held-out)

Every refined row by the tier the bench gives it, its algorithm, status and precision, and whether it
names a function. Rows with a status other than `Resolved` or `Ambiguous` claim nothing.

| package | plan | tier | algorithm | status | precision | names a function | rows |
|---|---|---|---|---|---|---|---|
| co | refined | none | PointsTo | Unresolved | Unknown | no | 30 |
| co | refined | exact | DirectReference | Resolved | SetupAware | yes | 12 |
| co | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 12 |
| pako | refined | none | PointsTo | Unresolved | Unknown | no | 1 |
| nanostores | refined | none | PointsTo | Unresolved | Unknown | no | 116 |
| nanostores | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 36 |
| nanostores | refined | exact | DirectReference | Resolved | SetupAware | yes | 22 |
| nanostores | refined | exact | ImportBinding | Resolved | SetupAware | yes | 14 |
| nanostores | refined | exact | PointsTo | Resolved | SetupAware | yes | 14 |
| express | refined | none | PointsTo | Unresolved | Unknown | no | 361 |
| express | refined | apart | PointsTo | Resolved | Heuristic | yes | 25 |
| express | refined | none | SyntaxOnly | Unresolved | Unknown | no | 25 |
| express | refined | exact | DirectReference | Resolved | SetupAware | yes | 24 |
| express | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 24 |
| express | refined | exact | PointsTo | Resolved | SetupAware | yes | 2 |
| immer | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 299 |
| immer | refined | none | PointsTo | Unresolved | Unknown | no | 232 |
| immer | refined | exact | ImportBinding | Resolved | SetupAware | yes | 173 |
| immer | refined | exact | PointsTo | Resolved | SetupAware | yes | 172 |
| immer | refined | exact | DirectReference | Resolved | SetupAware | yes | 124 |
| immer | refined | exact | ImportBinding | Resolved | SetupAware | no | 6 |
| immer | refined | exact | SummaryAssisted | Resolved | SetupAware | no | 6 |
| immer | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 2 |
| immer | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 1 |
| immer | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 1 |
| js-yaml | refined | none | PointsTo | Unresolved | Unknown | no | 366 |
| js-yaml | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 254 |
| js-yaml | refined | exact | DirectReference | Resolved | SetupAware | yes | 252 |
| js-yaml | refined | exact | PointsTo | Resolved | SetupAware | yes | 20 |
| js-yaml | refined | apart | PointsTo | Resolved | Heuristic | yes | 14 |
| js-yaml | refined | none | SyntaxOnly | Unresolved | Unknown | no | 14 |
| js-yaml | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 2 |
| js-yaml | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 2 |
| js-yaml | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 2 |
| acorn | refined | none | PointsTo | Unresolved | Unknown | no | 1587 |
| acorn | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 197 |
| acorn | refined | exact | DirectReference | Resolved | SetupAware | yes | 136 |
| acorn | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 61 |
| acorn | refined | apart | PointsTo | Resolved | Heuristic | yes | 48 |
| acorn | refined | none | SyntaxOnly | Unresolved | Unknown | no | 6 |
| acorn | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 3 |
| acorn | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 3 |
| node-forge | refined | none | PointsTo | Unresolved | Unknown | no | 4383 |
| node-forge | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 664 |
| node-forge | refined | exact | DirectReference | Resolved | SetupAware | yes | 610 |
| node-forge | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 54 |
| node-forge | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 9 |
| node-forge | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 9 |

## Sites gained by the refined plan (held-out)

Sites where a refined plan claims an exact or conservative target and the base plan claims none, by file.
**test path** marks a file in a test or benchmark folder or named `*.test.*` / `*.spec.*`; **executed** counts
the gained sites the run called a polint function from, the only ones recall can credit.

| package | plan | sites gained | in test paths | executed | top files |
|---|---|---|---|---|---|
| co | refined | 0 | 0 | 0 | – |
| pako | refined | 0 | 0 | 0 | – |
| nanostores | refined | 0 | 0 | 0 | – |
| express | refined | 2 | 0 | 2 | lib/application.js 1, lib/response.js 1 |
| immer | refined | 0 | 0 | 0 | – |
| js-yaml | refined | 20 | 0 | 20 | lib/loader.js 7, lib/dumper.js 5, lib/snippet.js 5 |
| acorn | refined | 0 | 0 | 0 | – |
| node-forge | refined | 0 | 0 | 0 | – |

## Exports (against the loaded module's export keys)

`Symbols` is the exported-symbol answer a rule can read; export facts are the module export rows.
Both are compared with the export keys the loaded module actually has. A file that fails to load
on its own is left out of both and counted under `load errors`.

| tree | files exact (Symbols) | name recall (Symbols) | name precision (Symbols) | files exact (export facts) | name recall (export facts) | name precision (export facts) | export fact rows by kind/status | load errors |
|---|---|---|---|---|---|---|---|---|
| package axios | 4/72 | 27/162 (17%) | 27/76 (36%) | 5/72 | 95/162 (59%) | 95/95 (100%) | Default/Resolved 67, Named/Resolved 28 | 0 |
| package zod | 10/122 | 940/3935 (24%) | 940/1739 (54%) | 15/122 | 1280/3935 (33%) | 1280/2042 (63%) | Default/Resolved 65, Named/Resolved 2068, Namespace/Unresolved 226, StarReexport/Unresolved 46 | 0 |
| package commander | 0/7 | 0/22 (0%) | – | 7/7 | 22/22 (100%) | 22/22 (100%) | CommonJsExportsProperty/Unsupported 22 | 0 |
| package async | 0/4 | 0/116 (0%) | – | 4/129 | 116/116 (100%) | 116/241 (48%) | CommonJsExportsProperty/Unsupported 347, CommonJsModuleExports/Unsupported 125 | 0 |
| package chalk | 1/5 | 4/29 (14%) | 4/7 (57%) | 1/5 | 16/29 (55%) | 16/16 (100%) | Default/Resolved 1, Named/Resolved 6, Namespace/Unresolved 9 | 0 |
| package dayjs | 0/1 | 0/7 (0%) | – | 0/1 | 0/7 (0%) | – | – | 0 |
| package underscore | 0/1 | 0/147 (0%) | – | 0/1 | 0/147 (0%) | – | – | 0 |
| held-out package co | 0/1 | 0/3 (0%) | – | 0/1 | 0/3 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| held-out package pako | 0/1 | 0/10 (0%) | – | 0/1 | 0/10 (0%) | – | – | 0 |
| held-out package nanostores | 12/14 | 31/62 (50%) | 31/31 (100%) | 14/14 | 62/62 (100%) | 62/62 (100%) | Named/Resolved 31, Namespace/Unresolved 31 | 0 |
| held-out package express | 0/6 | 0/122 (0%) | – | 2/6 | 19/122 (16%) | 19/19 (100%) | CommonJsExportsProperty/Unsupported 19, CommonJsModuleExports/Unsupported 6 | 0 |
| held-out package immer | 7/16 | 77/151 (51%) | 77/109 (71%) | 7/16 | 88/151 (58%) | 88/129 (68%) | Named/Resolved 113, Namespace/Unresolved 20, StarReexport/Unresolved 11 | 0 |
| held-out package js-yaml | 0/21 | 0/199 (0%) | – | 0/21 | 0/199 (0%) | – | CommonJsModuleExports/Unsupported 21 | 0 |
| held-out package acorn | 0/1 | 19/22 (86%) | 19/22 (86%) | 1/1 | 22/22 (100%) | 22/22 (100%) | Named/Resolved 22 | 0 |
| held-out package node-forge | 0/43 | 0/652 (0%) | – | 1/43 | 2/652 (0%) | 2/2 (100%) | CommonJsExportsProperty/Unsupported 2, CommonJsModuleExports/Unsupported 44 | 4 |
| case cjs-exports-property/function-expressions | 0/1 | 0/2 (0%) | – | 1/1 | 2/2 (100%) | 2/2 (100%) | CommonJsExportsProperty/Unsupported 2 | 0 |
| case cjs-exports-property/renamed-destructuring | 0/1 | 0/2 (0%) | – | 1/1 | 2/2 (100%) | 2/2 (100%) | CommonJsExportsProperty/Unsupported 2 | 0 |
| case cjs-module-exports/function-with-property | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case cjs-module-exports/object-literal | 0/1 | 0/2 (0%) | – | 0/1 | 0/2 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case class-method/subclass-override | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case dynamic-import/computed-specifier | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case dynamic-import/literal-specifier | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case esm-reexport/named-barrel | 2/3 | 2/4 (50%) | 2/2 (100%) | 3/3 | 4/4 (100%) | 4/4 (100%) | Named/Resolved 2, Namespace/Unresolved 2 | 0 |
| case esm-reexport/star-and-alias | 2/3 | 2/4 (50%) | 2/2 (100%) | 2/3 | 3/4 (75%) | 3/3 (100%) | Named/Resolved 2, Namespace/Unresolved 1, StarReexport/Unresolved 1 | 0 |
| case module-initializer/arrow-in-initializer | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case module-initializer/chained-call | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case module-initializer/class-expression | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case module-initializer/exported-call | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case module-initializer/object-literal-methods | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case same-name-member/cjs-static-method | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case same-name-member/wrapper-of-static-method | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |

## Cost

The only part of this report that changes between runs. Machine: Linux-6.8.0-147-generic-x86_64-with-glibc2.39, 32 CPUs, commit 4a16e3b4468c, load average 11.61 / 13.82 / 22.31 at the start and 9.91 / 12.32 / 19.86 at the end (1, 5, 15 minutes).

`polint check` (release) with the census rule pack, which asks for `Functions`, `CallGraph`, `Symbols`, `References`; median of 3 runs. `analysis` is the in-process cost record (`POLINT_GOLDEN_COST_PATH`), `process` is the whole CLI as `bench_matrix.measure` sees it. Cold has no `.polint/cache`; warm reruns over the cache the cold run left. Stored bytes are `.polint/cache` after a cold run, raw and gzip-compressed. Per MB means per 1,000,000 source bytes. Process peak RSS comes from `wait4`, which on Linux counts the launching Python process's resident size at fork, so no process figure here is below the launcher's peak of 27 MB; the analysis column has no such floor.

| tree | source KB | cold analysis ms (per MB) | cold process ms | warm analysis ms (per MB) | warm process ms | cold peak RSS MB (analysis / process) | warm peak RSS MB | stored bytes per MB (gzip) |
|---|---|---|---|---|---|---|---|---|
| constructs (all cases) | 13 | 82 (6111) | 201 | 47 (3503) | 167 | 74 / 74 | 73 / 73 | 112192801 (10904755) |
| axios | 282 | 646 (2291) | 767 | 422 (1496) | 541 | 101 / 101 | 102 / 102 | 75833297 (5409101) |
| zod | 1354 | 3982 (2940) | 4128 | 2534 (1871) | 2684 | 525 / 525 | 667 / 667 | 127740295 (10455119) |
| commander | 127 | 242 (1912) | 374 | 183 (1446) | 303 | 73 / 73 | 73 / 73 | 44266921 (3200455) |
| async | 321 | 462 (1441) | 584 | 299 (933) | 423 | 78 / 78 | 78 / 78 | 44436456 (3481011) |
| chalk | 10 | 44 (4621) | 162 | 23 (2416) | 139 | 72 / 72 | 73 / 73 | 87413717 (6979834) |
| dayjs | 7 | 37 (5167) | 158 | 17 (2374) | 137 | 73 / 73 | 73 / 73 | 222145650 (12891077) |
| underscore | 69 | 209 (3033) | 332 | 151 (2191) | 271 | 73 / 73 | 73 / 73 | 91434282 (5881609) |

Fact dump (no cache, one tree per process; peak RSS is the process figure, with the same floor): this is
the only way to time the refined plan, because no public view asks for it.

| tree | base ms | base peak RSS MB | refined ms | refined peak RSS MB | budget s |
|---|---|---|---|---|---|
| axios | 705 | 121 | 2118 | 298 | none |
| zod | 3690 | 534 | 12206 | 1015 | 180 |
| commander | 265 | 60 | 1766 | 159 | none |
| async | 469 | 94 | 2032 | 297 | none |
| chalk | 46 | 33 | 99 | 44 | none |
| dayjs | 43 | 32 | 105 | 35 | none |
| underscore | 222 | 53 | 713 | 131 | none |

### Stage times

Every fact dump runs with `RUST_LOG=polint::kernel::stage=info`; each provider's `stage done` lines are summed. Stage times are as noisy as the dump times above. `semantic_graph share` is the `polint.semantic_graph` stage's part of the refined plan's summed stage time. Peak MB is the process peak when the stage ended.

| tree | plan | stage total s | largest stages: s (peak MB) | semantic_graph share |
|---|---|---|---|---|
| axios | base | 0.6 | `symbol_graph` 0.3 (87), `semantic_mir` 0.2 (87), `cfg` 0.1 (104) | n/a |
| axios | refined | 1.9 | `evidence` 0.3 (260), `type_value_alias` 0.3 (222), `symbol_graph` 0.3 (86) | 11% |
| zod | base | 3.4 | `symbol_graph` 2.0 (509), `semantic_mir` 1.1 (509), `cfg` 0.2 (509) | n/a |
| zod | refined | 11.5 | `semantic_graph` 3.5 (781), `symbol_graph` 2.0 (509), `type_value_alias` 1.5 (781) | 30% |
| commander | base | 0.2 | `semantic_mir` 0.1 (45), `symbol_graph` 0.1 (41), `cfg` 0.0 (54) | n/a |
| commander | refined | 1.7 | `semantic_graph` 0.9 (144), `abstract_domains` 0.1 (115), `cfg` 0.1 (79) | 53% |
| async | base | 0.4 | `symbol_graph` 0.2 (66), `semantic_mir` 0.1 (66), `cfg` 0.1 (82) | n/a |
| async | refined | 1.9 | `abstract_domains` 0.4 (248), `cfg` 0.2 (135), `symbol_graph` 0.2 (67) | 8% |
| chalk | base | 0.0 | `module_graph` 0.0 (23), `symbol_graph` 0.0 (29), `cfg` 0.0 (30) | n/a |
| chalk | refined | 0.1 | `cfg` 0.0 (31), `evidence` 0.0 (40), `module_graph` 0.0 (23) | 5% |
| dayjs | base | 0.0 | `symbol_graph` 0.0 (28), `calls` 0.0 (29), `cfg` 0.0 (29) | n/a |
| dayjs | refined | 0.1 | `semantic_graph` 0.1 (32), `symbol_graph` 0.0 (29), `solver` 0.0 (33) | 63% |
| underscore | base | 0.2 | `symbol_graph` 0.1 (42), `semantic_mir` 0.1 (42), `cfg` 0.0 (47) | n/a |
| underscore | refined | 0.6 | `cfg` 0.1 (77), `evidence` 0.1 (122), `symbol_graph` 0.1 (43) | 6% |
| co | base | 0.0 | `symbol_graph` 0.0 (25), `cfg` 0.0 (28), `semantic_mir` 0.0 (27) | n/a |
| co | refined | 0.0 | `cfg` 0.0 (29), `type_value_alias` 0.0 (34), `evidence` 0.0 (36) | 6% |
| pako | base | 0.5 | `symbol_graph` 0.5 (58), `ts.syntax` 0.0 (24), `calls` 0.0 (58) | n/a |
| pako | refined | 3.3 | `semantic_graph` 2.8 (59), `symbol_graph` 0.5 (59), `solver` 0.0 (59) | 85% |
| nanostores | base | 0.1 | `symbol_graph` 0.0 (31), `module_graph` 0.0 (24), `cfg` 0.0 (34) | n/a |
| nanostores | refined | 0.2 | `cfg` 0.0 (39), `symbol_graph` 0.0 (32), `evidence` 0.0 (57) | 9% |
| express | base | 0.1 | `symbol_graph` 0.0 (35), `semantic_mir` 0.0 (38), `cfg` 0.0 (43) | n/a |
| express | refined | 0.4 | `cfg` 0.1 (53), `type_value_alias` 0.1 (90), `abstract_domains` 0.1 (73) | 7% |
| immer | base | 0.2 | `symbol_graph` 0.1 (54), `semantic_mir` 0.0 (54), `cfg` 0.0 (54) | n/a |
| immer | refined | 0.7 | `cfg` 0.1 (71), `symbol_graph` 0.1 (54), `evidence` 0.1 (132) | 6% |
| js-yaml | base | 0.3 | `symbol_graph` 0.1 (52), `semantic_mir` 0.1 (52), `cfg` 0.1 (62) | n/a |
| js-yaml | refined | 1.0 | `abstract_domains` 0.2 (126), `evidence` 0.1 (166), `type_value_alias` 0.1 (166) | 6% |
| acorn | base | 0.6 | `semantic_mir` 0.3 (74), `symbol_graph` 0.2 (63), `cfg` 0.1 (96) | n/a |
| acorn | refined | 2.1 | `type_value_alias` 0.4 (264), `semantic_graph` 0.4 (264), `semantic_mir` 0.3 (75) | 18% |
| node-forge | base | 1.8 | `symbol_graph` 0.9 (191), `semantic_mir` 0.6 (191), `cfg` 0.2 (250) | n/a |
| node-forge | refined | 5.8 | `type_value_alias` 1.2 (553), `evidence` 1.1 (634), `symbol_graph` 0.9 (192) | 11% |
| zod, globs alone | base | 10.8 | `symbol_graph` 4.4 (877), `semantic_mir` 3.9 (877), `calls` 1.5 (1015) | n/a |
| zod, globs alone | refined | 40.9 | `semantic_graph` 7.7 (2370), `type_value_alias` 6.5 (2370), `evidence` 5.9 (3034) | 19% |

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
