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
| axios 1.21.1 | ESM source, Node and browser adapters | 72 / 72 | 282 | 675 | 89/675 (13%) | 89/675 (13%) | 40 | 153 | 77% | 90% / 90% | 9% |
| zod 4.6.7 | TypeScript ESM source | 332 / 332 | 3088 | 1308 | 100/1308 (8%) | timed out after 600 s | 17 | 799 | 69% | 96% / n/a | 7% |
| commander 14.0.3 | CommonJS only | 7 / 7 | 127 | 318 | 14/318 (4%) | 117/318 (37%) | 0 | 1 | 94% | 98% / 78% | 10% |
| async 3.2.6 | CommonJS transpiled from ESM | 129 / 129 | 321 | 637 | 319/637 (50%) | 323/637 (51%) | 0 | 1 | 63% | 73% / 73% | 27% |
| chalk 6.0.1 | ESM only, with subpath imports | 2 / 5 | 10 | 72 | 1/72 (1%) | 4/72 (6%) | 2 | 48 | 76% | 98% / 93% | 9% |
| dayjs 1.11.23 | minified UMD | 1 / 1 | 7 | 117 | 0/117 (0%) | 0/117 (0%) | 0 | 117 | 100% | 100% / 100% | 7% |
| underscore 1.13.8 | bundled UMD (Rollup) | 1 / 1 | 69 | 262 | 156/262 (60%) | 157/262 (60%) | 0 | 0 | 55% | 58% / 58% | 6% |

### Package recall by observed call kind and syntax

| package | kind / syntax | observed | base | refined |
|---|---|---|---|---|
| axios | kind: call | 632 | 89/632 (14%) | 89/632 (14%) |
| axios | kind: callback | 43 | 0/43 (0%) | 0/43 (0%) |
| axios | kind: implicit | 6 | 0/6 (0%) | 0/6 (0%) |
| axios | syntax: computed-member | 2 | 0/2 (0%) | 0/2 (0%) |
| axios | syntax: expression | 8 | 8/8 (100%) | 8/8 (100%) |
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
| axios | 12 / 12 | 89 / 89 | 388 / 388 | 5 / 45 | 0 / 0 | 141 / 141 | 0 / 0 | 40 / 0 |
| zod | 6 / n/a | 100 / n/a | 391 / n/a | 1 / n/a | 0 / n/a | 793 / n/a | 0 / n/a | 17 / n/a |
| commander | 1 / 1 | 14 / 117 | 303 / 200 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| async | 1 / 1 | 319 / 323 | 317 / 313 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| chalk | 0 / 0 | 1 / 4 | 21 / 20 | 0 / 0 | 0 / 0 | 1 / 1 | 47 / 47 | 2 / 0 |
| dayjs | 1 / 1 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 116 / 116 | 0 / 0 | 0 / 0 |
| underscore | 0 / 0 | 156 / 157 | 106 / 102 | 0 / 0 | 0 / 3 | 0 / 0 | 0 / 0 | 0 / 0 |

### Missed edges ranked by gap

Every observed call or callback edge polint did not find is attributed to one gap: the first in the order
unary IIFEs, try / catch / if blocks, exported initializers, constructors, callbacks to built-ins, computed
keys, prototype and class methods, then the remainders. Extraction gaps come first because no resolution
change finds an edge whose call site or callee polint does not extract; a site or callee polint misses is
charged to the outermost function around it that polint does not extract. **attributed** counts each edge
once; **applying** counts every gap whose condition holds. The recall gain is an upper bound: it assumes
every attributed edge is found once the gap is fixed. The syntax census behind this is described in the README.

Base: 2710 missed of 3389 observed edges. Refined: 1391 missed of 2081, without zod (no refined dump).

| rank | gap | attributed base | applying base | recall gain base (upper bound) | attributed refined | recall gain refined (upper bound) | base by package |
|---|---|---|---|---|---|---|---|
| 1 | calls in exported initializers | 909 | 909 | 26.8% | 120 | 5.8% | zod 789, axios 119, chalk 1 |
| 2 | _other resolution misses_ | 660 | 0 | 19.5% | 508 | 24.4% | async 240, zod 176, axios 129, underscore 80, commander 22, chalk 13 |
| 3 | _other member calls (object-literal functions, namespaces)_ | 406 | 0 | 12.0% | 289 | 13.9% | axios 208, zod 116, async 52, underscore 20, commander 10 |
| 4 | prototype and class methods, incl. inheritance | 246 | 277 | 7.3% | 148 | 7.1% | commander 210, axios 22, async 8, zod 5, underscore 1 |
| 5 | callbacks invoked by built-ins | 125 | 163 | 3.7% | 98 | 4.7% | commander 53, axios 27, zod 27, async 13, underscore 5 |
| 6 | functions under a unary operator (unary IIFEs) | 118 | 118 | 3.5% | 118 | 5.7% | dayjs 117, axios 1 |
| 7 | class constructors and `new` | 88 | 94 | 2.6% | 19 | 0.9% | zod 63, axios 13, commander 8, async 4 |
| 8 | _identifier call stops at import binding_ | 52 | 0 | 1.5% | 0 | 0.0% | axios 33, zod 17, chalk 2 |
| 9 | _file not analysed (discovery excludes)_ | 47 | 0 | 1.4% | 47 | 2.3% | chalk 47 |
| 10 | _other call sites or functions not extracted_ | 31 | 0 | 0.9% | 21 | 1.0% | axios 20, zod 10, async 1 |
| 11 | functions declared in try / catch / if blocks | 14 | 47 | 0.4% | 14 | 0.7% | axios 13, commander 1 |
| 12 | computed-key member calls | 14 | 18 | 0.4% | 9 | 0.4% | chalk 8, zod 5, axios 1 |

Rows in italics are remainders, not one of the named extraction or resolution gaps. Of the base
prototype and class method misses, 2 go through `super` or a `this`
call into another class; that split reads the receiver syntactically and is approximate.
'Other resolution misses' by call syntax and callee kind (base): identifier call to arrow 372, expression call to declaration 134, identifier call to expression 93, identifier call to declaration 37, expression call to arrow 14, expression call to expression 9, identifier call to method 1. An `expression` call is one whose callee is neither a name nor a member, such as the `(0, f)(x)`
form compiled CommonJS uses.

### Other targets and no unresolved marker (every case)

The observed callee is not among polint's targets for the site, and the site carries no unresolved marker.
The polint site column shows which call expression the observed site was matched to.

| package | plan | observed site | syntax / kind | observed callee | polint answer | observed site text | polint site text |
|---|---|---|---|---|---|---|---|
| underscore | refined | underscore-umd.js:1371 | identifier / call | underscore-umd.js:727 <anonymous> | targets underscore-umd.js:710 identity, no unresolved marker | `iteratee(obj[currentKey], currentKey, obj)` | `iteratee(obj[currentKey], currentKey, obj)` |
| underscore | refined | underscore-umd.js:1371 | identifier / call | underscore-umd.js:1464 <anonymous> | targets underscore-umd.js:710 identity, no unresolved marker | `iteratee(obj[currentKey], currentKey, obj)` | `iteratee(obj[currentKey], currentKey, obj)` |
| underscore | refined | underscore-umd.js:1371 | identifier / call | underscore-umd.js:1582 <anonymous> | targets underscore-umd.js:710 identity, no unresolved marker | `iteratee(obj[currentKey], currentKey, obj)` | `iteratee(obj[currentKey], currentKey, obj)` |

### One example per package, plan and outcome

`report.json` keeps up to 5 per outcome (all of them for 'other targets, no unresolved marker') and up to
3 per gap.

| package | plan | outcome | observed site | syntax / kind | observed callee | polint answer |
|---|---|---|---|---|---|---|
| axios | base | callee not a polint function | lib/adapters/fetch.js:432 `trackRequestStream(_request.body, onProgress, flush)` | identifier / call | lib/adapters/fetch.js:381 trackRequestStream | no target, marked unresolved |
| axios | base | found | lib/adapters/fetch.js:113 `( (encoder) => (str) => encoder.encode(str) )(new TextEncoder())` | expression / call | lib/adapters/fetch.js:114 <anonymous> | targets lib/adapters/fetch.js:114 <polint:anonymous:3294:3347>, no unresolved marker (site matched by same end byte: '(encoder) => (str) => encoder.encode(str) )(new TextEncoder())') |
| axios | base | no target, marked unresolved | lib/adapters/adapters.js:53 `utils.isFunction(adapter)` | member / call | lib/utils.js:194 <anonymous> | no target, marked unresolved |
| axios | base | no target, no unresolved marker | lib/adapters/fetch.js:535 `AxiosHeaders.from(response.headers)` | member / call | lib/core/AxiosHeaders.js:423 from | no target, no unresolved marker |
| axios | base | site not a polint call site | lib/adapters/fetch.js:382 `trackStream( stream, DEFAULT_CHUNK_SIZE, (loadedBytes) => { if (hasMaxBodyLength && loade…` | identifier / call | lib/helpers/trackStream.js:45 trackStream | no call site |
| axios | base | stops at import binding | lib/adapters/fetch.js:426 `progressEventDecorator( requestContentLength, progressEventReducer(asyncDecorator(onUploa…` | identifier / call | lib/helpers/progressEventReducer.js:37 progressEventDecorator | import binding, no unresolved marker |
| axios | refined | callee not a polint function | lib/adapters/fetch.js:432 `trackRequestStream(_request.body, onProgress, flush)` | identifier / call | lib/adapters/fetch.js:381 trackRequestStream | no target, marked unresolved |
| axios | refined | found | lib/adapters/fetch.js:113 `( (encoder) => (str) => encoder.encode(str) )(new TextEncoder())` | expression / call | lib/adapters/fetch.js:114 <anonymous> | targets lib/adapters/fetch.js:114 <polint:anonymous:3294:3347>, no unresolved marker (site matched by same end byte: '(encoder) => (str) => encoder.encode(str) )(new TextEncoder())') |
| axios | refined | no target, marked unresolved | lib/adapters/adapters.js:53 `utils.isFunction(adapter)` | member / call | lib/utils.js:194 <anonymous> | no target, marked unresolved |
| axios | refined | no target, no unresolved marker | lib/adapters/fetch.js:426 `progressEventDecorator( requestContentLength, progressEventReducer(asyncDecorator(onUploa…` | identifier / call | lib/helpers/progressEventReducer.js:37 progressEventDecorator | import binding, no unresolved marker |
| axios | refined | site not a polint call site | lib/adapters/fetch.js:382 `trackStream( stream, DEFAULT_CHUNK_SIZE, (loadedBytes) => { if (hasMaxBodyLength && loade…` | identifier / call | lib/helpers/trackStream.js:45 trackStream | no call site |
| zod | base | callee not a polint function | src/v3/ZodError.ts:315 `mapper(sub)` | identifier / call | src/v3/ZodError.ts:308 <anonymous> | no target, marked unresolved |
| zod | base | found | src/v4/classic/errors.ts:58 `_lazyMethod(proto, "format", (self) => (mapper: any) => core.formatError(self, mapper))` | identifier / call | src/v4/classic/errors.ts:35 _lazyMethod | targets src/v4/classic/errors.ts:35 _lazyMethod, no unresolved marker |
| zod | base | no target, marked unresolved | src/v3/ZodError.ts:272 `processError(this)` | identifier / call | src/v3/ZodError.ts:224 processError | no target, marked unresolved |
| zod | base | no target, no unresolved marker | src/v4/core/json-schema-processors.ts:750 `JSON.stringify(value, (_, val) => { if (typeof val !== "bigint") return val; unrepresenta…` | member / callback | src/v4/core/json-schema-processors.ts:750 <anonymous> | no target, no unresolved marker |
| zod | base | site not a polint call site | src/v3/ZodError.ts:264 `mapper(issue)` | identifier / call | src/v3/ZodError.ts:220 <anonymous> | no call site |
| zod | base | stops at import binding | src/v3/helpers/parseUtil.ts:73 `getErrorMap()` | identifier / call | src/v3/errors.ts:11 getErrorMap | import binding, no unresolved marker |
| commander | base | callee not a polint function | lib/help.js:331 `option.argChoices.map((choice) => JSON.stringify(choice))` | member / callback | lib/help.js:331 <anonymous> | no target, marked unresolved |
| commander | base | found | lib/command.js:70 `useColor()` | identifier / call | lib/command.js:2752 useColor | targets lib/command.js:2752 useColor, no unresolved marker |
| commander | base | no target, marked unresolved | index.js:7 `new Command()` | new / call | lib/command.js:20 constructor | no target, marked unresolved |
| commander | refined | callee not a polint function | lib/help.js:331 `option.argChoices.map((choice) => JSON.stringify(choice))` | member / callback | lib/help.js:331 <anonymous> | no target, marked unresolved |
| commander | refined | found | index.js:7 `new Command()` | new / call | lib/command.js:20 constructor | targets lib/command.js:13 Command, no unresolved marker |
| commander | refined | no target, marked unresolved | lib/argument.js:144 `arg.name()` | member / call | lib/argument.js:48 name | no target, marked unresolved |
| async | base | callee not a polint function | memoize.js:67 `hasher(...args)` | identifier / call | memoize.js:62 <anonymous> | no target, marked unresolved |
| async | base | found | applyEach.js:9 `_interopRequireDefault(_applyEach)` | identifier / call | applyEach.js:15 _interopRequireDefault | targets applyEach.js:15 _interopRequireDefault, no unresolved marker |
| async | base | no target, marked unresolved | applyEach.js:56 `(0, _applyEach2.default)(_map2.default)` | expression / call | internal/applyEach.js:7 default | no target, marked unresolved |
| async | refined | callee not a polint function | memoize.js:67 `hasher(...args)` | identifier / call | memoize.js:62 <anonymous> | no target, marked unresolved |
| async | refined | found | applyEach.js:9 `_interopRequireDefault(_applyEach)` | identifier / call | applyEach.js:15 _interopRequireDefault | targets applyEach.js:15 _interopRequireDefault, no unresolved marker |
| async | refined | no target, marked unresolved | applyEach.js:56 `(0, _applyEach2.default)(_map2.default)` | expression / call | internal/applyEach.js:7 default | no target, marked unresolved |
| chalk | base | found | source/index.js:240 `createChalk()` | identifier / call | source/index.js:61 createChalk | targets source/index.js:61 createChalk, no unresolved marker |
| chalk | base | no target, marked unresolved | source/index.js:37 `assertValidLevel(options.level)` | identifier / call | source/index.js:17 assertValidLevel | no target, marked unresolved |
| chalk | base | site not a polint call site | source/index.js:241 `createChalk({level: stderrColor ? stderrColor.level : 0})` | identifier / call | source/index.js:61 createChalk | no call site |
| chalk | base | site or callee in a file polint does not analyse | source/index.js:91 `style.ansi(ansiStyles.rgbToAnsi(red, green, blue))` | member / call | source/vendor/ansi-styles/index.js:4 <anonymous> | no target, marked unresolved |
| chalk | base | stops at import binding | source/index.js:219 `stringReplaceAll(string, styler.close, styler.open)` | identifier / call | source/utilities.js:2 stringReplaceAll | import binding, no unresolved marker |
| chalk | refined | found | source/index.js:118 `createModelConverters(model, type)` | identifier / call | source/index.js:87 createModelConverters | targets source/index.js:87 createModelConverters, no unresolved marker |
| chalk | refined | no target, marked unresolved | source/index.js:37 `assertValidLevel(options.level)` | identifier / call | source/index.js:17 assertValidLevel | no target, marked unresolved |
| chalk | refined | site not a polint call site | source/index.js:241 `createChalk({level: stderrColor ? stderrColor.level : 0})` | identifier / call | source/index.js:61 createChalk | no call site |
| chalk | refined | site or callee in a file polint does not analyse | source/index.js:91 `style.ansi(ansiStyles.rgbToAnsi(red, green, blue))` | member / call | source/vendor/ansi-styles/index.js:4 <anonymous> | no target, marked unresolved |
| dayjs | base | callee not a polint function | dayjs.min.js:1 `function(t,e){"object"==typeof exports&&"undefined"!=typeof module?module.exports=e():"fu…` | expression / call | dayjs.min.js:1 <anonymous> | no target, marked unresolved |
| dayjs | base | site not a polint call site | dayjs.min.js:1 `m(r,2,"0")` | identifier / call | dayjs.min.js:1 m | no call site |
| dayjs | refined | callee not a polint function | dayjs.min.js:1 `function(t,e){"object"==typeof exports&&"undefined"!=typeof module?module.exports=e():"fu…` | expression / call | dayjs.min.js:1 <anonymous> | no target, marked unresolved |
| dayjs | refined | site not a polint call site | dayjs.min.js:1 `m(r,2,"0")` | identifier / call | dayjs.min.js:1 m | no call site |
| underscore | base | found | underscore-umd.js:1 `function (global, factory) { typeof exports === 'object' && typeof module !== 'undefined'…` | expression / call | underscore-umd.js:1 <anonymous> | targets underscore-umd.js:1 <polint:anonymous:1:451>, no unresolved marker |
| underscore | base | no target, marked unresolved | underscore-umd.js:304 `getLength(obj)` | identifier / call | underscore-umd.js:223 <anonymous> | no target, marked unresolved |
| underscore | refined | found | underscore-umd.js:1 `function (global, factory) { typeof exports === 'object' && typeof module !== 'undefined'…` | expression / call | underscore-umd.js:1 <anonymous> | targets underscore-umd.js:1 <polint:anonymous:1:451>, no unresolved marker |
| underscore | refined | no target, marked unresolved | underscore-umd.js:304 `getLength(obj)` | identifier / call | underscore-umd.js:223 <anonymous> | no target, marked unresolved |
| underscore | refined | other targets, no unresolved marker | underscore-umd.js:1371 `iteratee(obj[currentKey], currentKey, obj)` | identifier / call | underscore-umd.js:727 <anonymous> | targets underscore-umd.js:710 identity, no unresolved marker |

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

The only part of this report that changes between runs. Machine: Linux-6.8.0-147-generic-x86_64-with-glibc2.39, 32 CPUs, commit 55bf3fde8da0.

`polint check` (release) with the census rule pack, which asks for `Functions`, `CallGraph`, `Symbols`, `References`; median of 3 runs. `analysis` is the in-process cost record (`POLINT_GOLDEN_COST_PATH`), `process` is the whole CLI as `bench_matrix.measure` sees it. Cold has no `.polint/cache`; warm reruns over the cache the cold run left. Stored bytes are `.polint/cache` after a cold run, raw and gzip-compressed. Per MB means per 1,000,000 source bytes. Process peak RSS comes from `wait4`, which on Linux counts the launching Python process's resident size at fork, so no process figure here is below the launcher's peak of 29 MB; the analysis column has no such floor.

| tree | source KB | cold analysis ms (per MB) | cold process ms | warm analysis ms (per MB) | warm process ms | cold peak RSS MB (analysis / process) | warm peak RSS MB | stored bytes per MB (gzip) |
|---|---|---|---|---|---|---|---|---|
| constructs (all cases) | 8 | 67 (8102) | 189 | 31 (3748) | 147 | 73 / 73 | 73 / 73 | 108277630 (11093349) |
| axios | 282 | 590 (2092) | 711 | 364 (1291) | 480 | 89 / 89 | 92 / 92 | 75350157 (5339729) |
| zod | 3088 | 10157 (3289) | 10348 | 10157 (3289) | 10349 | 1011 / 1011 | 993 / 993 | 95877087 (6780763) |
| commander | 127 | 243 (1920) | 360 | 172 (1359) | 282 | 72 / 72 | 73 / 73 | 44222052 (3192777) |
| async | 321 | 427 (1332) | 547 | 291 (908) | 405 | 78 / 78 | 77 / 77 | 44203329 (3457642) |
| chalk | 10 | 45 (4726) | 154 | 23 (2416) | 137 | 73 / 73 | 73 / 73 | 86943598 (6904002) |
| dayjs | 7 | 39 (5446) | 154 | 18 (2514) | 127 | 73 / 73 | 73 / 73 | 221683284 (12812456) |
| underscore | 69 | 195 (2830) | 309 | 144 (2090) | 260 | 72 / 72 | 72 / 72 | 91000174 (5822567) |

Fact dump (no cache, one tree per process; peak RSS is the process figure, with the same floor): this is
the only way to time the refined plan, because no public view asks for it.

| tree | base ms | base peak RSS MB | refined ms | refined peak RSS MB |
|---|---|---|---|---|
| axios | 1226 | 106 | 3218 | 237 |
| zod | 15787 | 1268 | 600195 (timed out after 600 s) | 2325 |
| commander | 248 | 61 | 1895 | 145 |
| async | 745 | 94 | 2243 | 264 |
| chalk | 54 | 32 | 94 | 40 |
| dayjs | 39 | 32 | 110 | 36 |
| underscore | 212 | 53 | 697 | 120 |

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
- An observed callee maps to the polint function with the same span before the innermost one ending at the same byte: a concise arrow returning an arrow, `(a) => (b) => ...`, ends at the same byte as the arrow it returns.
- Every missed observed edge is attributed to one gap from a syntax census: Oxc's own function and call spans with their syntactic context, parsed again apart from the analysis. A site or callee polint does not extract is charged to the outermost function around it that polint does not extract, else to the outermost enclosing function or the call in a gap; edges polint maps go to constructors, callbacks to built-ins, computed keys and prototype or class methods, in that order. A computed-key miss is counted whether or not the key set is known, so that row is an upper bound for computed keys over a known set.
- A census function counts as extracted when polint has a function in the same file ending at the same byte, or, for a class constructor, when polint has the class.
- Every package edge with other targets and no unresolved marker is kept as an example; other outcomes and gaps keep a bounded sample, in observed-edge order.
- Process peak RSS is measured with `wait4` like the matrix runner does. Linux carries the parent's resident size into the child's peak at fork, so every process is launched before the report loads any fact dump, and the launcher's own peak is recorded with the cost section. The in-process analysis peak has no such floor.
- Each fact dump has a wall-clock limit (`--dump-timeout`, 600 s by default); a tree whose dump exceeds it is reported as timed out for that plan and has no scores in that column.
