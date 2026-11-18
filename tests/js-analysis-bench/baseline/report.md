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
| accessor | 2 (1) | 0/6 (0%) | 3/6 (50%) | – | – | – | 3/3 (100%) | – | – | 0 / 0 | 0 |
| argument-flow | 2 (1) | 5/14 (36%) | 14/14 (100%) | 0/1 (0%) | 1/1 (100%) | 5/5 (100%) | 15/15 (100%) | – | – | 0 / 0 | 0 |
| await | 2 (1) | 4/6 (67%) | 6/6 (100%) | – | – | 4/4 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| block-function | 2 (1) | 9/9 (100%) | 9/9 (100%) | – | – | 9/9 (100%) | 9/9 (100%) | – | – | 0 / 0 | 0 |
| call-apply-bind | 2 (1) | 0/5 (0%) | 5/5 (100%) | – | – | – | 5/5 (100%) | – | – | 0 / 0 | 0 |
| callback | 3 (1) | 5/9 (56%) | 9/9 (100%) | 0/4 (0%) | 4/4 (100%) | 5/5 (100%) | 12/12 (100%) | – | – | 0 / 0 | 0 |
| cjs-exports-property | 3 (1) | 1/6 (17%) | 6/6 (100%) | – | – | 1/1 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| cjs-interop | 2 (1) | 0/4 (0%) | 3/4 (75%) | – | – | – | 3/4 (75%) | – | – | 0 / 0 | 0 |
| cjs-module-exports | 2 (1) | 1/6 (17%) | 6/6 (100%) | – | – | 1/1 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| class-features | 2 (1) | 3/13 (23%) | 12/13 (92%) | – | – | 3/3 (100%) | 12/14 (86%) | – | – | 0 / 0 | 0 |
| class-method | 2 (1) | 3/9 (33%) | 8/9 (89%) | – | – | 3/3 (100%) | 8/8 (100%) | – | – | 0 / 0 | 0 |
| closure | 3 (1) | 4/11 (36%) | 9/11 (82%) | – | – | 4/4 (100%) | 9/9 (100%) | – | – | 0 / 0 | 0 |
| computed-key-foldable | 2 (1) | 0/4 (0%) | 4/4 (100%) | – | – | – | 4/4 (100%) | – | – | 0 / 0 | 0 |
| computed-key-unknown | 2 (1) | 4/6 (67%) | 4/6 (67%) | – | – | 4/4 (100%) | 4/4 (100%) | 1 / 0 / 0 | 1 / 0 / 0 | 0 / 0 | 0 |
| constant-key-field | 2 (1) | 1/6 (17%) | 6/6 (100%) | – | – | 1/1 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| destructuring | 2 (1) | 1/7 (14%) | 7/7 (100%) | – | – | 1/1 (100%) | 7/7 (100%) | – | – | 0 / 0 | 0 |
| dynamic-import | 2 (0) | 1/2 (50%) | 1/2 (50%) | – | – | 1/1 (100%) | 1/1 (100%) | 1 / 0 / 0 | 1 / 0 / 0 | 0 / 0 | 0 |
| dynamic-property | 2 (1) | 0/2 (0%) | 0/2 (0%) | – | – | – | – | 1 / 0 / 0 | 1 / 0 / 0 | 0 / 0 | 0 |
| esm-namespace | 2 (1) | 4/7 (57%) | 7/7 (100%) | – | – | 4/4 (100%) | 7/9 (78%) | – | – | 0 / 0 | 0 |
| esm-reexport | 2 (1) | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 0 / 0 | 0 |
| eval | 2 (1) | – | – | – | – | – | – | 2 / 0 / 0 | 2 / 0 / 0 | 0 / 0 | 0 |
| exported-initializer | 2 (1) | 6/6 (100%) | 6/6 (100%) | – | – | 6/6 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| function-object | 2 (1) | 2/5 (40%) | 5/5 (100%) | – | – | 2/2 (100%) | 5/5 (100%) | – | – | 0 / 0 | 0 |
| iteration | 2 (1) | 2/8 (25%) | 8/8 (100%) | – | – | 2/2 (100%) | 8/9 (89%) | – | – | 0 / 0 | 0 |
| literal-spread | 2 (1) | 0/7 (0%) | 5/7 (71%) | – | – | – | 5/5 (100%) | – | – | 0 / 0 | 0 |
| logical-join | 2 (1) | 10/21 (48%) | 21/21 (100%) | – | – | 10/10 (100%) | 21/21 (100%) | – | – | 0 / 0 | 0 |
| module-initializer | 6 (2) | 13/23 (57%) | 22/23 (96%) | – | – | 13/13 (100%) | 22/22 (100%) | – | – | 0 / 0 | 0 |
| new-and-prototype | 2 (1) | 3/8 (38%) | 8/8 (100%) | – | – | 3/3 (100%) | 8/8 (100%) | – | – | 0 / 0 | 0 |
| object-of-functions | 3 (1) | 2/8 (25%) | 8/8 (100%) | – | – | 2/2 (100%) | 8/8 (100%) | – | – | 0 / 0 | 0 |
| optional-chaining | 2 (1) | 2/5 (40%) | 5/5 (100%) | – | – | 2/2 (100%) | 5/5 (100%) | – | – | 0 / 0 | 0 |
| package-vendor | 2 (1) | 4/6 (67%) | 6/6 (100%) | – | – | 4/4 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| promise | 2 (1) | 4/4 (100%) | 4/4 (100%) | 0/1 (0%) | 1/1 (100%) | 4/4 (100%) | 5/6 (83%) | – | – | 0 / 0 | 0 |
| prototype | 2 (1) | 2/7 (29%) | 6/7 (86%) | – | – | 2/2 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| prototype-chain | 2 (1) | 1/6 (17%) | 6/6 (100%) | – | – | 1/1 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| returned-function | 4 (2) | 8/23 (35%) | 23/23 (100%) | – | – | 8/8 (100%) | 23/23 (100%) | – | – | 0 / 0 | 0 |
| same-name-member | 2 (1) | 2/4 (50%) | 4/4 (100%) | – | – | 2/4 (50%) | 4/6 (67%) | – | – | 0 / 0 | 0 |
| sequence-callee | 2 (1) | 5/8 (62%) | 8/8 (100%) | – | – | 5/5 (100%) | 8/8 (100%) | – | – | 0 / 0 | 0 |
| this-forms | 2 (1) | 3/11 (27%) | 11/11 (100%) | – | – | 3/3 (100%) | 11/11 (100%) | – | – | 0 / 0 | 0 |
| unary-iife | 2 (1) | 11/13 (85%) | 13/13 (100%) | 0/1 (0%) | 1/1 (100%) | 11/11 (100%) | 13/13 (100%) | – | – | 0 / 0 | 0 |

**False-complete count** (a labelled hidden path where polint gives a resolved answer with no unresolved marker): base 0, refined 0, over 5 labelled hidden paths.

**False-complete call lines** (a labelled call line where polint gives resolved targets that miss a labelled callee, with no unresolved marker on the line): base 0, refined 0, over 263 labelled call lines.

| case | plan | line | resolved to | missed |
|---|---|---|---|---|
| none | | | | |

### Per case

| case | held out | calls base | calls refined | callbacks base | callbacks refined | precision base | precision refined | hidden paths base | hidden paths refined | unresolved call share base | labels = observed |
|---|---|---|---|---|---|---|---|---|---|---|---|
| accessor/class-accessor-pair | yes | 0/4 (0%) | 2/4 (50%) | – | – | – | 2/2 (100%) | – | – | 67% | yes |
| accessor/getter-returns-function | no | 0/2 (0%) | 1/2 (50%) | – | – | – | 1/1 (100%) | – | – | 100% | yes |
| argument-flow/parameter-and-return | no | 3/8 (38%) | 8/8 (100%) | – | – | 3/3 (100%) | 8/8 (100%) | – | – | 57% | yes |
| argument-flow/rest-and-spread | yes | 2/6 (33%) | 6/6 (100%) | 0/1 (0%) | 1/1 (100%) | 2/2 (100%) | 7/7 (100%) | – | – | 67% | yes |
| await/awaited-function | no | 2/3 (67%) | 3/3 (100%) | – | – | 2/2 (100%) | 3/3 (100%) | – | – | 33% | yes |
| await/promise-resolve-value | yes | 2/3 (67%) | 3/3 (100%) | – | – | 2/2 (100%) | 3/3 (100%) | – | – | 25% | yes |
| block-function/catch-and-finally | yes | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 17% | yes |
| block-function/try-and-if | no | 5/5 (100%) | 5/5 (100%) | – | – | 5/5 (100%) | 5/5 (100%) | – | – | 25% | yes |
| call-apply-bind/bind | yes | 0/2 (0%) | 2/2 (100%) | – | – | – | 2/2 (100%) | – | – | 100% | yes |
| call-apply-bind/call-and-apply | no | 0/3 (0%) | 3/3 (100%) | – | – | – | 3/3 (100%) | – | – | 100% | yes |
| callback/builtin-invoked | no | 1/1 (100%) | 1/1 (100%) | 0/3 (0%) | 3/3 (100%) | 1/1 (100%) | 3/3 (100%) | – | – | 75% | yes |
| callback/higher-order-parameter | no | 2/6 (33%) | 6/6 (100%) | – | – | 2/2 (100%) | 6/6 (100%) | – | – | 71% | yes |
| callback/nested-higher-order | yes | 2/2 (100%) | 2/2 (100%) | 0/1 (0%) | 1/1 (100%) | 2/2 (100%) | 3/3 (100%) | – | – | 50% | yes |
| cjs-exports-property/exports-of-factory-result | yes | 1/2 (50%) | 2/2 (100%) | – | – | 1/1 (100%) | 2/2 (100%) | – | – | 75% | yes |
| cjs-exports-property/function-expressions | no | 0/2 (0%) | 2/2 (100%) | – | – | – | 2/2 (100%) | – | – | 100% | yes |
| cjs-exports-property/renamed-destructuring | no | 0/2 (0%) | 2/2 (100%) | – | – | – | 2/2 (100%) | – | – | 100% | yes |
| cjs-interop/compiled-default | no | 0/2 (0%) | 2/2 (100%) | – | – | – | 2/2 (100%) | – | – | 75% | yes |
| cjs-interop/define-property-export | yes | 0/2 (0%) | 1/2 (50%) | – | – | – | 1/2 (50%) | – | – | 67% | yes |
| cjs-module-exports/function-with-property | yes | 0/3 (0%) | 3/3 (100%) | – | – | – | 3/3 (100%) | – | – | 100% | yes |
| cjs-module-exports/object-literal | no | 1/3 (33%) | 3/3 (100%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 80% | yes |
| class-features/class-expression | yes | 1/4 (25%) | 3/4 (75%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 50% | yes |
| class-features/fields-statics-super | no | 2/9 (22%) | 9/9 (100%) | – | – | 2/2 (100%) | 9/11 (82%) | – | – | 100% | yes |
| class-method/instance-and-static | no | 2/5 (40%) | 5/5 (100%) | – | – | 2/2 (100%) | 5/5 (100%) | – | – | 83% | yes |
| class-method/subclass-override | yes | 1/4 (25%) | 3/4 (75%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 80% | yes |
| closure/captured-callback | no | 1/4 (25%) | 4/4 (100%) | – | – | 1/1 (100%) | 4/4 (100%) | – | – | 75% | yes |
| closure/captured-parameter | yes | 1/4 (25%) | 2/4 (50%) | – | – | 1/1 (100%) | 2/2 (100%) | – | – | 50% | yes |
| closure/factory-returned-function | no | 2/3 (67%) | 3/3 (100%) | – | – | 2/2 (100%) | 3/3 (100%) | – | – | 25% | yes |
| computed-key-foldable/const-key-literal | yes | 0/2 (0%) | 2/2 (100%) | – | – | – | 2/2 (100%) | – | – | 100% | yes |
| computed-key-foldable/constant-concatenation | no | 0/2 (0%) | 2/2 (100%) | – | – | – | 2/2 (100%) | – | – | 100% | yes |
| computed-key-unknown/escaped-object | yes | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | reported-unknown | reported-unknown | 33% | yes |
| computed-key-unknown/known-literal-table | no | 2/4 (50%) | 2/4 (50%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 60% | yes |
| constant-key-field/private-field | yes | 0/2 (0%) | 2/2 (100%) | – | – | – | 2/2 (100%) | – | – | 67% | yes |
| constant-key-field/string-and-numeric-keys | no | 1/4 (25%) | 4/4 (100%) | – | – | 1/1 (100%) | 4/4 (100%) | – | – | 75% | yes |
| destructuring/array-and-nested | yes | 0/3 (0%) | 3/3 (100%) | – | – | – | 3/3 (100%) | – | – | 100% | yes |
| destructuring/object-defaults-and-rest | no | 1/4 (25%) | 4/4 (100%) | – | – | 1/1 (100%) | 4/4 (100%) | – | – | 75% | yes |
| dynamic-import/computed-specifier | no | – | – | – | – | – | – | reported-unknown | reported-unknown | 100% | yes |
| dynamic-import/literal-specifier | no | 1/2 (50%) | 1/2 (50%) | – | – | 1/1 (100%) | 1/1 (100%) | – | – | 50% | yes |
| dynamic-property/computed-key-known-set | no | 0/2 (0%) | 0/2 (0%) | – | – | – | – | – | – | 100% | yes |
| dynamic-property/key-from-input | yes | – | – | – | – | – | – | reported-unknown | reported-unknown | 100% | yes |
| esm-namespace/namespace-object | no | 2/3 (67%) | 3/3 (100%) | – | – | 2/2 (100%) | 3/3 (100%) | – | – | 60% | yes |
| esm-namespace/star-and-live-binding | yes | 2/4 (50%) | 4/4 (100%) | – | – | 2/2 (100%) | 4/6 (67%) | – | – | 50% | yes |
| esm-reexport/named-barrel | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 50% | yes |
| esm-reexport/star-and-alias | yes | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 0% | yes |
| eval/direct-eval | no | – | – | – | – | – | – | reported-unknown | reported-unknown | 100% | yes |
| eval/function-constructor | yes | – | – | – | – | – | – | reported-unknown | reported-unknown | 100% | yes |
| exported-initializer/cjs-exports | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| exported-initializer/esm-declarators | yes | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| function-object/factory-made-constructor | yes | 0/3 (0%) | 3/3 (100%) | – | – | – | 3/3 (100%) | – | – | 60% | yes |
| function-object/own-properties | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 67% | yes |
| iteration/for-of-array | no | 0/3 (0%) | 3/3 (100%) | – | – | – | 3/3 (100%) | – | – | 100% | yes |
| iteration/generator | yes | 2/5 (40%) | 5/5 (100%) | – | – | 2/2 (100%) | 5/6 (83%) | – | – | 50% | yes |
| literal-spread/object-spread | no | 0/4 (0%) | 3/4 (75%) | – | – | – | 3/3 (100%) | – | – | 100% | yes |
| literal-spread/override-after-spread | yes | 0/3 (0%) | 2/3 (67%) | – | – | – | 2/2 (100%) | – | – | 100% | yes |
| logical-join/conditional-callee | yes | 6/10 (60%) | 10/10 (100%) | – | – | 6/6 (100%) | 10/10 (100%) | – | – | 33% | yes |
| logical-join/default-operand | no | 4/11 (36%) | 11/11 (100%) | – | – | 4/4 (100%) | 11/11 (100%) | – | – | 56% | yes |
| module-initializer/arrow-in-initializer | no | 2/4 (50%) | 4/4 (100%) | – | – | 2/2 (100%) | 4/4 (100%) | – | – | 60% | yes |
| module-initializer/chained-call | yes | 3/5 (60%) | 5/5 (100%) | – | – | 3/3 (100%) | 5/5 (100%) | – | – | 50% | yes |
| module-initializer/class-expression | yes | 1/4 (25%) | 3/4 (75%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 80% | yes |
| module-initializer/exported-call | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 0% | yes |
| module-initializer/local-initializer | no | 3/4 (75%) | 4/4 (100%) | – | – | 3/3 (100%) | 4/4 (100%) | – | – | 25% | yes |
| module-initializer/object-literal-methods | no | 2/4 (50%) | 4/4 (100%) | – | – | 2/2 (100%) | 4/4 (100%) | – | – | 50% | yes |
| new-and-prototype/constructor-function | no | 1/3 (33%) | 3/3 (100%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 67% | yes |
| new-and-prototype/return-override | yes | 2/5 (40%) | 5/5 (100%) | – | – | 2/2 (100%) | 5/5 (100%) | – | – | 60% | yes |
| object-of-functions/literal-methods | no | 0/2 (0%) | 2/2 (100%) | – | – | – | 2/2 (100%) | – | – | 100% | yes |
| object-of-functions/registry-built-later | no | 1/3 (33%) | 3/3 (100%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 67% | yes |
| object-of-functions/registry-passed-around | yes | 1/3 (33%) | 3/3 (100%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 67% | yes |
| optional-chaining/optional-call | no | 0/2 (0%) | 2/2 (100%) | – | – | – | 2/2 (100%) | – | – | 100% | yes |
| optional-chaining/optional-member-chain | yes | 2/3 (67%) | 3/3 (100%) | – | – | 2/2 (100%) | 3/3 (100%) | – | – | 33% | yes |
| package-vendor/cjs-require | yes | 1/3 (33%) | 3/3 (100%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 83% | yes |
| package-vendor/esm-import | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 25% | yes |
| promise/async-await | yes | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| promise/then-chain | no | 1/1 (100%) | 1/1 (100%) | 0/1 (0%) | 1/1 (100%) | 1/1 (100%) | 2/3 (67%) | – | – | 60% | no |
| prototype-chain/object-create | no | 0/3 (0%) | 3/3 (100%) | – | – | – | 3/3 (100%) | – | – | 60% | yes |
| prototype-chain/set-prototype-of | yes | 1/3 (33%) | 3/3 (100%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 50% | yes |
| prototype/constructor-function | no | 1/3 (33%) | 3/3 (100%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 67% | yes |
| prototype/inherited-method | yes | 1/4 (25%) | 3/4 (75%) | – | – | 1/1 (100%) | 3/3 (100%) | – | – | 60% | yes |
| returned-function/branch-returns | no | 1/4 (25%) | 4/4 (100%) | – | – | 1/1 (100%) | 4/4 (100%) | – | – | 50% | yes |
| returned-function/conditional-default | yes | 3/9 (33%) | 9/9 (100%) | – | – | 3/3 (100%) | 9/9 (100%) | – | – | 50% | yes |
| returned-function/stored-in-property | yes | 1/4 (25%) | 4/4 (100%) | – | – | 1/1 (100%) | 4/4 (100%) | – | – | 50% | yes |
| returned-function/two-levels | no | 3/6 (50%) | 6/6 (100%) | – | – | 3/3 (100%) | 6/6 (100%) | – | – | 25% | yes |
| same-name-member/cjs-static-method | yes | 1/2 (50%) | 2/2 (100%) | – | – | 1/2 (50%) | 2/3 (67%) | – | – | 67% | yes |
| same-name-member/wrapper-of-static-method | no | 1/2 (50%) | 2/2 (100%) | – | – | 1/2 (50%) | 2/3 (67%) | – | – | 50% | yes |
| sequence-callee/compiled-interop | no | 2/4 (50%) | 4/4 (100%) | – | – | 2/2 (100%) | 4/4 (100%) | – | – | 44% | yes |
| sequence-callee/local-sequence | yes | 3/4 (75%) | 4/4 (100%) | – | – | 3/3 (100%) | 4/4 (100%) | – | – | 25% | yes |
| this-forms/literal-method-this | no | 1/5 (20%) | 5/5 (100%) | – | – | 1/1 (100%) | 5/5 (100%) | – | – | 80% | yes |
| this-forms/union-receiver | yes | 2/6 (33%) | 6/6 (100%) | – | – | 2/2 (100%) | 6/6 (100%) | – | – | 43% | yes |
| unary-iife/umd-prologue | no | 3/5 (60%) | 5/5 (100%) | 0/1 (0%) | 1/1 (100%) | 3/3 (100%) | 5/5 (100%) | – | – | 83% | yes |
| unary-iife/unary-operators | yes | 8/8 (100%) | 8/8 (100%) | – | – | 8/8 (100%) | 8/8 (100%) | – | – | 50% | no |

## Packages (observed calls: recall only)

Recall is over observed edges at call expressions (`call` and `callback`); `implicit` calls (getters,
coercions, iterator and thenable protocol) have no call expression and are listed by kind only.

Coverage is the share of the analysed files' functions (syntax census, classes and static blocks left out)
that the recording entered.

| package | format | files analysed / selected | source KB | coverage | observed edges | recall base | recall refined | stops at import binding (base) | not mappable to a polint site or function | unresolved call share base | call sites without target base / refined | unresolved reference share |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| axios 1.21.1 | ESM source, Node and browser adapters | 72 / 72 | 282 | 300/534 (56%) | 675 | 273/675 (40%) | 582/675 (86%) | 1 | 0 | 67% | 74% / 49% | 9% |
| zod 4.6.7 | TypeScript ESM source | 125 / 125 | 1354 | 566/2469 (23%) (flagged) | 1308 | 721/1308 (55%) | 1181/1308 (90%) | 1 | 0 | 53% | 56% / 38% | 5% |
| commander 14.0.3 | CommonJS only | 7 / 7 | 127 | 218/296 (74%) | 318 | 21/318 (7%) | 183/318 (58%) | 0 | 0 | 92% | 95% / 66% | 10% |
| async 3.2.6 | CommonJS transpiled from ESM | 129 / 129 | 321 | 308/504 (61%) | 637 | 324/637 (51%) | 587/637 (92%) | 0 | 0 | 63% | 73% / 40% | 27% |
| chalk 6.0.1 | ESM only, with subpath imports | 5 / 5 | 22 | 41/53 (77%) | 72 | 43/72 (60%) | 56/72 (78%) | 0 | 0 | 44% | 67% / 55% | 11% |
| dayjs 1.11.23 | minified UMD | 1 / 1 | 7 | 55/57 (96%) | 112 | 32/112 (29%) | 62/112 (55%) | 0 | 0 | 76% | 79% / 62% | 7% |
| underscore 1.13.8 | bundled UMD (Rollup) | 1 / 1 | 69 | 159/186 (85%) | 260 | 155/260 (60%) | 250/260 (96%) | 0 | 0 | 56% | 58% / 29% | 6% |

### Claimed targets the run never called

A claimed target is resolved and not heuristic. It counts here when the run executed the site, the target is
in an instrumented file, and the run never called it from that site. On a **single-target** answer that is a
false precise edge: every execution of the site went elsewhere. On a **multi-target** answer it is an upper
bound only, since the run may not have taken the paths to the other targets. A site matched to a polint site
by its end byte can also inflate both counts.

| package | base: single / multi | refined: single / multi |
|---|---|---|
| axios | 0 / 0 | 2 / 22 |
| zod | 0 / 0 | 0 / 5 |
| commander | 0 / 0 | 0 / 0 |
| async | 0 / 0 | 0 / 27 |
| chalk | 0 / 0 | 0 / 0 |
| dayjs | 0 / 0 | 0 / 0 |
| underscore | 0 / 0 | 0 / 53 |
| **total** | **0 / 0** | **2 / 107** |

| package | plan | site | site text | single-target answer | the run called |
|---|---|---|---|---|---|
| axios | refined | lib/core/Axios.js:222 | `this.interceptors.request.forEach(function unshiftRequestInterceptors(interceptor) { if (…` | lib/core/Axios.js:222 <polint:anonymous:7284:7986> | lib/core/InterceptorManager.js:159 InterceptorManager.forEach |
| axios | refined | lib/core/Axios.js:241 | `this.interceptors.response.forEach(function pushResponseInterceptors(interceptor) { respo…` | lib/core/Axios.js:241 <polint:anonymous:8070:8206> | lib/core/InterceptorManager.js:159 InterceptorManager.forEach |

The typed plan (refined with the TypeScript type sidecar) was not run: no TypeScript compiler was given.

### Package recall by observed call kind and syntax

| package | kind / syntax | observed | base | refined |
|---|---|---|---|---|
| axios | kind: call | 632 | 272/632 (43%) | 554/632 (88%) |
| axios | kind: callback | 43 | 1/43 (2%) | 28/43 (65%) |
| axios | kind: implicit | 6 | 0/6 (0%) | 0/6 (0%) |
| axios | syntax: computed-member | 2 | 0/2 (0%) | 1/2 (50%) |
| axios | syntax: expression | 8 | 8/8 (100%) | 8/8 (100%) |
| axios | syntax: identifier | 317 | 249/317 (79%) | 304/317 (96%) |
| axios | syntax: member | 326 | 1/326 (0%) | 248/326 (76%) |
| axios | syntax: new | 21 | 15/21 (71%) | 20/21 (95%) |
| axios | syntax: super | 1 | 0/1 (0%) | 1/1 (100%) |
| zod | kind: call | 1265 | 721/1265 (57%) | 1150/1265 (91%) |
| zod | kind: callback | 43 | 0/43 (0%) | 31/43 (72%) |
| zod | kind: implicit | 18 | 0/18 (0%) | 0/18 (0%) |
| zod | syntax: computed-member | 5 | 0/5 (0%) | 0/5 (0%) |
| zod | syntax: expression | 1 | 1/1 (100%) | 1/1 (100%) |
| zod | syntax: identifier | 408 | 246/408 (60%) | 378/408 (93%) |
| zod | syntax: member | 829 | 472/829 (57%) | 738/829 (89%) |
| zod | syntax: new | 65 | 2/65 (3%) | 64/65 (98%) |
| commander | kind: call | 264 | 21/264 (8%) | 134/264 (51%) |
| commander | kind: callback | 54 | 0/54 (0%) | 49/54 (91%) |
| commander | syntax: identifier | 35 | 20/35 (57%) | 34/35 (97%) |
| commander | syntax: member | 274 | 0/274 (0%) | 140/274 (51%) |
| commander | syntax: new | 8 | 1/8 (12%) | 8/8 (100%) |
| commander | syntax: super | 1 | 0/1 (0%) | 1/1 (100%) |
| async | kind: call | 622 | 324/622 (52%) | 573/622 (92%) |
| async | kind: callback | 15 | 0/15 (0%) | 14/15 (93%) |
| async | kind: implicit | 4 | 0/4 (0%) | 0/4 (0%) |
| async | syntax: expression | 157 | 0/157 (0%) | 157/157 (100%) |
| async | syntax: identifier | 403 | 324/403 (80%) | 398/403 (99%) |
| async | syntax: member | 73 | 0/73 (0%) | 28/73 (38%) |
| async | syntax: new | 4 | 0/4 (0%) | 4/4 (100%) |
| chalk | kind: call | 71 | 43/71 (61%) | 55/71 (77%) |
| chalk | kind: callback | 1 | 0/1 (0%) | 1/1 (100%) |
| chalk | kind: implicit | 1 | 0/1 (0%) | 0/1 (0%) |
| chalk | syntax: computed-member | 10 | 0/10 (0%) | 0/10 (0%) |
| chalk | syntax: identifier | 43 | 43/43 (100%) | 43/43 (100%) |
| chalk | syntax: member | 19 | 0/19 (0%) | 13/19 (68%) |
| dayjs | kind: call | 110 | 32/110 (29%) | 60/110 (55%) |
| dayjs | kind: callback | 2 | 0/2 (0%) | 2/2 (100%) |
| dayjs | kind: implicit | 12 | 0/12 (0%) | 0/12 (0%) |
| dayjs | syntax: computed-member | 1 | 0/1 (0%) | 0/1 (0%) |
| dayjs | syntax: identifier | 35 | 32/35 (91%) | 35/35 (100%) |
| dayjs | syntax: member | 75 | 0/75 (0%) | 26/75 (35%) |
| dayjs | syntax: new | 1 | 0/1 (0%) | 1/1 (100%) |
| underscore | kind: call | 255 | 155/255 (61%) | 247/255 (97%) |
| underscore | kind: callback | 5 | 0/5 (0%) | 3/5 (60%) |
| underscore | kind: implicit | 2 | 0/2 (0%) | 0/2 (0%) |
| underscore | syntax: identifier | 234 | 154/234 (66%) | 232/234 (99%) |
| underscore | syntax: member | 25 | 0/25 (0%) | 17/25 (68%) |
| underscore | syntax: new | 1 | 1/1 (100%) | 1/1 (100%) |

### Why observed edges were missed (base / refined)

| package | found | no target, marked unresolved | no target, no unresolved marker | other targets, marked unresolved | stops at import binding |
|---|---|---|---|---|---|
| axios | 273 / 582 | 392 / 78 | 9 / 0 | 0 / 15 | 1 / 0 |
| zod | 721 / 1181 | 585 / 120 | 1 / 0 | 0 / 7 | 1 / 0 |
| commander | 21 / 183 | 297 / 134 | 0 / 0 | 0 / 1 | 0 / 0 |
| async | 324 / 587 | 313 / 6 | 0 / 0 | 0 / 44 | 0 / 0 |
| chalk | 43 / 56 | 29 / 16 | 0 / 0 | 0 / 0 | 0 / 0 |
| dayjs | 32 / 62 | 80 / 50 | 0 / 0 | 0 / 0 | 0 / 0 |
| underscore | 155 / 250 | 105 / 8 | 0 / 0 | 0 / 2 | 0 / 0 |

### Missed edges ranked by gap

Every observed call or callback edge polint did not find is attributed to one gap: the first in the order
unary IIFEs, try / catch / if blocks, exported initializers, constructors, callbacks to built-ins, computed
keys, prototype and class methods, then the remainders. Extraction gaps come first because no resolution
change finds an edge whose call site or callee polint does not extract; a site or callee polint misses is
charged to the outermost function around it that polint does not extract. **attributed** counts each edge
once; **applying** counts every gap whose condition holds. The recall gain is an upper bound: it assumes
every attributed edge is found once the gap is fixed. The syntax census behind this is described in the README.

Base: 1813 missed of 3382 observed edges. Refined: 481 missed of 3382.

| rank | gap | attributed base | applying base | recall gain base (upper bound) | attributed refined | recall gain refined (upper bound) | base by package |
|---|---|---|---|---|---|---|---|
| 1 | _other member calls (object-literal functions, namespaces)_ | 730 | 0 | 21.6% | 231 | 6.8% | zod 306, axios 251, dayjs 73, async 52, underscore 20, chalk 18, commander 10 |
| 2 | _other resolution misses_ | 549 | 0 | 16.2% | 38 | 1.1% | async 236, zod 149, underscore 79, axios 67, commander 15, dayjs 3 |
| 3 | prototype and class methods, incl. inheritance | 275 | 277 | 8.1% | 159 | 4.7% | commander 210, axios 36, zod 20, async 8, underscore 1 |
| 4 | callbacks invoked by built-ins | 156 | 162 | 4.6% | 34 | 1.0% | commander 54, zod 43, axios 38, async 13, underscore 5, dayjs 2, chalk 1 |
| 5 | class constructors and `new` | 83 | 83 | 2.5% | 2 | 0.1% | zod 63, commander 8, axios 7, async 4, dayjs 1 |
| 6 | computed-key member calls | 18 | 18 | 0.5% | 17 | 0.5% | chalk 10, zod 5, axios 2, dayjs 1 |
| 7 | _identifier call stops at import binding_ | 2 | 0 | 0.1% | 0 | 0.0% | axios 1, zod 1 |
| 8 | functions declared in try / catch / if blocks | 0 | 0 | 0.0% | 0 | 0.0% | – |
| 9 | calls in exported initializers | 0 | 0 | 0.0% | 0 | 0.0% | – |
| 10 | _file not analysed (discovery excludes)_ | 0 | 0 | 0.0% | 0 | 0.0% | – |
| 11 | _other call sites or functions not extracted_ | 0 | 0 | 0.0% | 0 | 0.0% | – |
| 12 | functions under a unary operator (unary IIFEs) | 0 | 0 | 0.0% | 0 | 0.0% | – |

Rows in italics are remainders, not one of the named extraction or resolution gaps. Of the base
prototype and class method misses, 2 go through `super` or a `this`
call into another class; that split reads the receiver syntactically and is approximate.
'Other resolution misses' by call syntax and callee kind (base): identifier call to arrow 247, expression call to declaration 134, identifier call to expression 101, identifier call to declaration 39, expression call to arrow 14, expression call to expression 9, identifier call to unknown 4, identifier call to method 1. An `expression` call is one whose callee is neither a name nor a member, such as the `(0, f)(x)`
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
| axios | base | found | lib/adapters/adapters.js:80 `isResolvedHandle(nameOrAdapter)` | identifier / call | lib/adapters/adapters.js:52 isResolvedHandle | targets lib/adapters/adapters.js:52 isResolvedHandle, no unresolved marker |
| axios | base | no target, marked unresolved | lib/adapters/adapters.js:53 `utils.isFunction(adapter)` | member / call | lib/utils.js:194 <anonymous> | no target, marked unresolved |
| axios | base | no target, no unresolved marker | lib/adapters/fetch.js:535 `AxiosHeaders.from(response.headers)` | member / call | lib/core/AxiosHeaders.js:423 from | no target, no unresolved marker |
| axios | base | stops at import binding | lib/core/AxiosHeaders.js:244 `parseHeaders(header)` | identifier / call | lib/helpers/parseHeaders.js:41 <anonymous> | import binding, no unresolved marker |
| axios | refined | found | lib/adapters/adapters.js:53 `utils.isFunction(adapter)` | member / call | lib/utils.js:194 <anonymous> | targets lib/utils.js:194 <polint:anonymous:5832:5864>, marked unresolved |
| axios | refined | no target, marked unresolved | lib/adapters/adapters.js:88 `adapter.get(config)` | member / call | lib/adapters/fetch.js:718 getFetch | no target, marked unresolved |
| axios | refined | other targets, marked unresolved | lib/core/Axios.js:222 `this.interceptors.request.forEach(function unshiftRequestInterceptors(interceptor) { if (…` | member / call | lib/core/InterceptorManager.js:159 forEach | targets lib/core/Axios.js:222 <polint:anonymous:7284:7986>, marked unresolved |
| zod | base | found | src/v3/ZodError.ts:272 `processError(this)` | identifier / call | src/v3/ZodError.ts:224 processError | targets src/v3/ZodError.ts:224 <polint:anonymous:5894:7736>, no unresolved marker |
| zod | base | no target, marked unresolved | src/v3/ZodError.ts:264 `mapper(issue)` | identifier / call | src/v3/ZodError.ts:220 <anonymous> | no target, marked unresolved |
| zod | base | no target, no unresolved marker | src/v4/core/json-schema-processors.ts:750 `JSON.stringify(value, (_, val) => { if (typeof val !== "bigint") return val; unrepresenta…` | member / callback | src/v4/core/json-schema-processors.ts:750 <anonymous> | no target, no unresolved marker |
| zod | base | stops at import binding | src/v4/classic/schemas.ts:15 `en()` | identifier / call | src/v4/locales/en.ts:132 <anonymous> | import binding, no unresolved marker |
| zod | refined | found | src/v3/ZodError.ts:264 `mapper(issue)` | identifier / call | src/v3/ZodError.ts:220 <anonymous> | targets src/v3/ZodError.ts:220 <polint:anonymous:5731:5797>, marked unresolved |
| zod | refined | no target, marked unresolved | src/v4/classic/from-json-schema.ts:505 `z.string()` | member / call | src/v4/classic/schemas.ts:619 string | no target, marked unresolved |
| zod | refined | other targets, marked unresolved | src/v4/core/core.ts:124 `fn()` | identifier / call | src/v4/core/memoizer.ts:228 <anonymous> | targets src/v4/core/schemas.ts:229 <polint:anonymous:7915:7971>, marked unresolved |
| commander | base | found | lib/command.js:649 `knownBy(command)` | identifier / call | lib/command.js:645 knownBy | targets lib/command.js:645 <polint:anonymous:20982:21047>, no unresolved marker |
| commander | base | no target, marked unresolved | index.js:7 `new Command()` | new / call | lib/command.js:20 constructor | no target, marked unresolved |
| commander | refined | found | index.js:7 `new Command()` | new / call | lib/command.js:20 constructor | targets lib/command.js:13 Command, no unresolved marker |
| commander | refined | no target, marked unresolved | lib/argument.js:144 `arg.name()` | member / call | lib/argument.js:48 name | no target, marked unresolved |
| commander | refined | other targets, marked unresolved | lib/command.js:1488 `fn()` | identifier / call | lib/command.js:1515 <anonymous> | targets lib/command.js:1363 <polint:anonymous:45378:45588>; lib/command.js:1603 <polint:anonymous:52130:52183>; lib/command.js:1607 <polint:anonymous:52274:52362>, marked unresolved |
| async | base | found | applyEach.js:9 `_interopRequireDefault(_applyEach)` | identifier / call | applyEach.js:15 _interopRequireDefault | targets applyEach.js:15 _interopRequireDefault, no unresolved marker |
| async | base | no target, marked unresolved | applyEach.js:56 `(0, _applyEach2.default)(_map2.default)` | expression / call | internal/applyEach.js:7 default | no target, marked unresolved |
| async | refined | found | applyEach.js:9 `_interopRequireDefault(_applyEach)` | identifier / call | applyEach.js:15 _interopRequireDefault | targets applyEach.js:15 _interopRequireDefault, no unresolved marker |
| async | refined | no target, marked unresolved | asyncify.js:111 `callback(error, value)` | identifier / call | internal/map.js:22 <anonymous> | no target, marked unresolved |
| async | refined | other targets, marked unresolved | auto.js:293 `taskFn(results, taskCallback)` | identifier / call | autoInject.js:173 newTask | targets all.js:115 every; all.js:116 <polint:anonymous:3398:3411>; all.js:116 <polint:anonymous:3413:3424>; allSeries.js:41 everySeries; allSeries.js:42 <polint:anonymous:1472:1485>; allSeries.js:42 <polint:anonymous:1487:1498>; any.js:118 some; any.js:119 <polint:anonymous:3659:3669>; anySeries.js:42 someSeries; anySeries.js:43 <polint:anonymous:1524:1534>; asyncify.js:78 <polint:anonymous:2637:2828>; auto.js:265 <polint:anonymous:9708:10464>; concat.js:111 concat; concatLimit.js:41 concatLimit; concatLimit.js:43 <polint:anonymous:1575:1733>; concatLimit.js:44 <polint:anonymous:1617:1725>; concatLimit.js:48 <polint:anonymous:1735:1996>; concatSeries.js:37 concatSeries; detect.js:92 detect; detect.js:93 <polint:anonymous:3080:3092>; detect.js:93 <polint:anonymous:3094:3113>; doDuring.js:44 doWhilst; doDuring.js:50 next; doUntil.js:41 <polint:anonymous:1547:1655>; doWhilst.js:44 doWhilst; doWhilst.js:50 next; during.js:55 whilst; during.js:61 next; during.js:68 check; each.js:124 eachLimit; eachLimit.js:46 eachLimit; eachOf.js:48 iteratorCallback; eachOf.js:179 eachOf; eachOfLimit.js:42 eachOfLimit; eachOfSeries.js:35 eachOfSeries; eachSeries.js:40 eachSeries; ensureAsync.js:53 <polint:anonymous:1897:2274>; every.js:115 every; every.js:116 <polint:anonymous:3398:3411>; every.js:116 <polint:anonymous:3413:3424>; everySeries.js:41 everySeries; everySeries.js:42 <polint:anonymous:1472:1485>; everySeries.js:42 <polint:anonymous:1487:1498>; filter.js:89 filter; filterSeries.js:39 filterSeries; find.js:92 detect; find.js:93 <polint:anonymous:3080:3092>; find.js:93 <polint:anonymous:3094:3113>; flatMap.js:111 concat; flatMapLimit.js:41 concatLimit; flatMapLimit.js:43 <polint:anonymous:1575:1733>; flatMapLimit.js:44 <polint:anonymous:1617:1725>; flatMapLimit.js:48 <polint:anonymous:1735:1996>; flatMapSeries.js:37 concatSeries; foldl.js:142 reduce; foldl.js:145 <polint:anonymous:4633:4758>; foldl.js:146 <polint:anonymous:4680:4750>; foldl.js:150 <polint:anonymous:4760:4786>; forEach.js:124 eachLimit; forEachLimit.js:46 eachLimit; forEachOf.js:48 iteratorCallback; forEachOf.js:179 eachOf; forEachOfLimit.js:42 eachOfLimit; forEachOfSeries.js:35 eachOfSeries; forEachSeries.js:40 eachSeries; forever.js:56 forever; forever.js:60 next; groupByLimit.js:41 groupByLimit; groupByLimit.js:43 <polint:anonymous:1587:1749>; groupByLimit.js:44 <polint:anonymous:1629:1741>; groupByLimit.js:48 <polint:anonymous:1751:2347>; inject.js:142 reduce; inject.js:145 <polint:anonymous:4633:4758>; inject.js:146 <polint:anonymous:4680:4750>; inject.js:150 <polint:anonymous:4760:4786>; internal/applyEach.js:7 default; internal/applyEach.js:8 <polint:anonymous:132:451>; internal/applyEach.js:9 <polint:anonymous:214:424>; internal/applyEach.js:11 <polint:anonymous:296:402>; internal/asyncEachOfLimit.js:46 iterateeCallback; internal/awaitify.js:9 awaitify; internal/awaitify.js:12 awaitable; internal/createTester.js:34 <polint:anonymous:1062:1183>; internal/createTester.js:18 _createTester; internal/createTester.js:19 <polint:anonymous:467:1191>; internal/createTester.js:23 <polint:anonymous:639:1060>; internal/createTester.js:24 <polint:anonymous:693:1048>; internal/eachOfLimit.js:32 <polint:anonymous:765:2599>; internal/filter.js:38 <polint:anonymous:1043:1218>; internal/filter.js:45 <polint:anonymous:1228:1366>; internal/filter.js:51 _filter; internal/filter.js:20 <polint:anonymous:532:670>; internal/filter.js:21 <polint:anonymous:576:662>; internal/filter.js:25 <polint:anonymous:672:895>; internal/filter.js:37 <polint:anonymous:999:1226>; internal/initialParams.js:8 <polint:anonymous:128:250>; internal/iterator.js:21 <polint:anonymous:521:606>; internal/iterator.js:28 <polint:anonymous:680:834>; internal/iterator.js:40 <polint:anonymous:976:1159>; internal/map.js:14 _asyncMap; internal/map.js:20 <polint:anonymous:507:675>; internal/map.js:22 <polint:anonymous:587:667>; internal/map.js:26 <polint:anonymous:677:723>; internal/once.js:8 wrapper; internal/onlyOnce.js:8 <polint:anonymous:145:323>; internal/parallel.js:21 <polint:anonymous:526:923>; internal/parallel.js:24 <polint:anonymous:639:888>; internal/parallel.js:25 <polint:anonymous:703:880>; internal/parallel.js:32 <polint:anonymous:890:919>; internal/promiseCallback.js:10 callback; internal/queue.js:92 <polint:anonymous:2672:2767>; internal/queue.js:107 <polint:anonymous:3018:3803>; internal/queue.js:141 <polint:anonymous:3990:4012>; internal/queue.js:212 <polint:anonymous:6162:7313>; internal/reject.js:18 reject; internal/reject.js:20 <polint:anonymous:550:647>; internal/reject.js:21 <polint:anonymous:591:639>; internal/withoutIndex.js:8 <polint:anonymous:161:214>; internal/wrapAsync.js:26 wrapAsync; map.js:138 map; mapSeries.js:40 mapSeries; mapValuesLimit.js:47 mapValuesLimit; mapValuesLimit.js:51 <polint:anonymous:1804:1986>; mapValuesLimit.js:52 <polint:anonymous:1854:1978>; mapValuesLimit.js:57 <polint:anonymous:1988:2016>; memoize.js:66 <polint:anonymous:2234:2955>; memoize.js:69 <polint:anonymous:2358:2392>; queue.js:9 <polint:anonymous:218:271>; race.js:57 race; reduce.js:142 reduce; reduce.js:145 <polint:anonymous:4633:4758>; reduce.js:146 <polint:anonymous:4680:4750>; reduce.js:150 <polint:anonymous:4760:4786>; reflect.js:59 <polint:anonymous:1714:2228>; reject.js:83 reject; rejectLimit.js:41 rejectLimit; rejectSeries.js:39 rejectSeries; retry.js:132 <polint:anonymous:4598:4944>; retryable.js:63 <polint:anonymous:2064:2497>; retryable.js:68 taskFn; select.js:89 filter; selectSeries.js:39 filterSeries; seq.js:70 <polint:anonymous:2358:2516>; seq.js:74 <polint:anonymous:2518:2555>; some.js:118 some; some.js:119 <polint:anonymous:3659:3669>; someSeries.js:42 someSeries; someSeries.js:43 <polint:anonymous:1524:1534>; sortBy.js:171 sortBy; sortBy.js:173 <polint:anonymous:5666:5832>; sortBy.js:174 <polint:anonymous:5704:5824>; sortBy.js:178 <polint:anonymous:5834:5966>; timeout.js:62 <polint:anonymous:1994:2717>; transform.js:168 <polint:anonymous:5373:5436>; transform.js:170 <polint:anonymous:5438:5471>; tryEach.js:59 tryEach; tryEach.js:62 <polint:anonymous:2074:2418>; tryEach.js:63 <polint:anonymous:2133:2410>; tryEach.js:74 <polint:anonymous:2420:2449>; until.js:59 <polint:anonymous:1978:2022>; until.js:59 <polint:anonymous:1990:2021>; waterfall.js:82 waterfall; waterfall.js:93 next; whilst.js:55 whilst; whilst.js:61 next; whilst.js:68 check, marked unresolved |
| chalk | base | found | source/index.js:37 `assertValidLevel(options.level)` | identifier / call | source/index.js:17 assertValidLevel | targets source/index.js:17 assertValidLevel, no unresolved marker |
| chalk | base | no target, marked unresolved | source/index.js:91 `style.ansi(ansiStyles.rgbToAnsi(red, green, blue))` | member / call | source/vendor/ansi-styles/index.js:4 <anonymous> | no target, marked unresolved |
| chalk | refined | found | source/index.js:37 `assertValidLevel(options.level)` | identifier / call | source/index.js:17 assertValidLevel | targets source/index.js:17 assertValidLevel, no unresolved marker |
| chalk | refined | no target, marked unresolved | source/index.js:91 `style.ansi(ansiStyles.rgbToAnsi(red, green, blue))` | member / call | source/vendor/ansi-styles/index.js:4 <anonymous> | no target, marked unresolved |
| dayjs | base | found | dayjs.min.js:1 `m(r,2,"0")` | identifier / call | dayjs.min.js:1 m | targets dayjs.min.js:1 <polint:anonymous:859:948>, no unresolved marker |
| dayjs | base | no target, marked unresolved | dayjs.min.js:1 `e.date()` | member / call | dayjs.min.js:1 <anonymous> | no target, marked unresolved |
| dayjs | refined | found | dayjs.min.js:1 `m(r,2,"0")` | identifier / call | dayjs.min.js:1 m | targets dayjs.min.js:1 <polint:anonymous:859:948>, no unresolved marker |
| dayjs | refined | no target, marked unresolved | dayjs.min.js:1 `e.date()` | member / call | dayjs.min.js:1 <anonymous> | no target, marked unresolved |
| underscore | base | found | underscore-umd.js:289 `isObject(obj)` | identifier / call | underscore-umd.js:88 isObject | targets underscore-umd.js:88 isObject, no unresolved marker |
| underscore | base | no target, marked unresolved | underscore-umd.js:304 `getLength(obj)` | identifier / call | underscore-umd.js:223 <anonymous> | no target, marked unresolved |
| underscore | refined | found | underscore-umd.js:289 `isObject(obj)` | identifier / call | underscore-umd.js:88 isObject | targets underscore-umd.js:88 isObject, no unresolved marker |
| underscore | refined | no target, marked unresolved | underscore-umd.js:1006 `sourceFunc.apply(context, args)` | member / call | underscore-umd.js:1238 before | no target, marked unresolved |
| underscore | refined | other targets, marked unresolved | underscore-umd.js:1664 `iteratee(value, key, obj)` | identifier / call | underscore-umd.js:1677 iteratee | targets underscore-umd.js:710 identity; underscore-umd.js:718 <polint:anonymous:25165:25220>; underscore-umd.js:727 <polint:anonymous:25455:25509>; underscore-umd.js:738 <polint:anonymous:25846:25913>; underscore-umd.js:742 <polint:anonymous:26006:26111>; underscore-umd.js:745 <polint:anonymous:26134:26265>; underscore-umd.js:749 <polint:anonymous:26284:26347>; underscore-umd.js:1209 <polint:anonymous:42056:42122>; underscore-umd.js:1412 <polint:anonymous:49081:49179>; underscore-umd.js:1464 <polint:anonymous:50883:51216>; underscore-umd.js:1502 <polint:anonymous:52164:52401>; underscore-umd.js:1527 <polint:anonymous:53017:53252>; underscore-umd.js:1582 <polint:anonymous:54836:54985>; underscore-umd.js:1604 <polint:anonymous:55513:55630>; underscore-umd.js:1645 keyInObj; underscore-umd.js:1729 <polint:anonymous:60125:60184>; underscore-umd.js:1866 <polint:anonymous:64437:64679>; underscore-umd.js:1878 <polint:anonymous:64831:65181>; underscore-umd.js:1893 <polint:anonymous:65277:65505>; underscore-umd.js:202 isNaN$1; underscore-umd.js:223 <polint:anonymous:7915:7982>, marked unresolved |

## Tiers

Recall over the same observed edges as above, split by the tier of the edge that found the callee
(`research/package-graphs/TIERS.md`): `Exact` and `SetupAware` are exact, except typed-hierarchy edges
that dispatch to an `implementation` or `union_member`, which are candidates and count as conservative;
`Conservative` and `Ambiguous` are conservative; `Heuristic` is apart and never counts in the first three
columns. **exact-complete** also needs the site to be complete: at least one exact or conservative target and
no unresolved row.

| package | plan | observed edges | exact-complete | exact | exact + conservative | apart only | apart edges |
|---|---|---|---|---|---|---|---|
| axios | base | 675 | 262/675 (39%) | 262/675 (39%) | 262/675 (39%) | 11/675 (2%) | 67 |
| axios | refined | 675 | 308/675 (46%) | 308/675 (46%) | 358/675 (53%) | 224/675 (33%) | 505 |
| zod | base | 1308 | 670/1308 (51%) | 670/1308 (51%) | 670/1308 (51%) | 51/1308 (4%) | 160 |
| zod | refined | 1308 | 678/1308 (52%) | 678/1308 (52%) | 709/1308 (54%) | 472/1308 (36%) | 4851 |
| commander | base | 318 | 21/318 (7%) | 21/318 (7%) | 21/318 (7%) | 0/318 (0%) | 0 |
| commander | refined | 318 | 36/318 (11%) | 36/318 (11%) | 85/318 (27%) | 98/318 (31%) | 146 |
| async | base | 637 | 324/637 (51%) | 324/637 (51%) | 324/637 (51%) | 0/637 (0%) | 0 |
| async | refined | 637 | 328/637 (51%) | 328/637 (51%) | 348/637 (55%) | 239/637 (38%) | 46397 |
| chalk | base | 72 | 43/72 (60%) | 43/72 (60%) | 43/72 (60%) | 0/72 (0%) | 1 |
| chalk | refined | 72 | 43/72 (60%) | 43/72 (60%) | 44/72 (61%) | 12/72 (17%) | 14 |
| dayjs | base | 112 | 32/112 (29%) | 32/112 (29%) | 32/112 (29%) | 0/112 (0%) | 4 |
| dayjs | refined | 112 | 34/112 (30%) | 34/112 (30%) | 36/112 (32%) | 26/112 (23%) | 34 |
| underscore | base | 260 | 154/260 (59%) | 154/260 (59%) | 154/260 (59%) | 1/260 (0%) | 4 |
| underscore | refined | 260 | 213/260 (82%) | 213/260 (82%) | 229/260 (88%) | 21/260 (8%) | 424 |
| **total** | base | 3382 | 1506/3382 (45%) | 1506/3382 (45%) | 1506/3382 (45%) | 63/3382 (2%) | 236 |
| **total** | refined | 3382 | 1640/3382 (48%) | 1640/3382 (48%) | 1809/3382 (53%) | 1092/3382 (32%) | 52371 |

Targets per site that claims any in the tier: mean / p95 (nearest rank) / max.

| package | plan | exact | conservative | exact + conservative | apart |
|---|---|---|---|---|---|
| axios | base | 1.0 / 1 / 1 (488 sites) | – | 1.0 / 1 / 1 (488 sites) | 1.0 / 1 / 1 (67 sites) |
| axios | refined | 1.01 / 1 / 4 (621 sites) | 1.32 / 2 / 16 (112 sites) | 1.06 / 1 / 16 (733 sites) | 1.05 / 1 / 3 (415 sites) |
| zod | base | 1.0 / 1 / 1 (2506 sites) | – | 1.0 / 1 / 1 (2506 sites) | 1.0 / 1 / 1 (160 sites) |
| zod | refined | 1.0 / 1 / 4 (2554 sites) | 1.0 / 1 / 1 (239 sites) | 1.0 / 1 / 4 (2793 sites) | 2.14 / 1 / 252 (2121 sites) |
| commander | base | 1.0 / 1 / 1 (42 sites) | – | 1.0 / 1 / 1 (42 sites) | – |
| commander | refined | 1.02 / 1 / 2 (63 sites) | 1.0 / 1 / 1 (75 sites) | 1.01 / 1 / 2 (138 sites) | 1.04 / 1 / 3 (140 sites) |
| async | base | 1.0 / 1 / 1 (440 sites) | – | 1.0 / 1 / 1 (440 sites) | – |
| async | refined | 1.01 / 1 / 2 (451 sites) | 4.67 / 16 / 16 (48 sites) | 1.36 / 1 / 16 (499 sites) | 100.86 / 167 / 180 (460 sites) |
| chalk | base | 1.0 / 1 / 1 (46 sites) | – | 1.0 / 1 / 1 (46 sites) | 1.0 / 1 / 1 (1 sites) |
| chalk | refined | 1.0 / 1 / 1 (47 sites) | 1.0 / 1 / 1 (6 sites) | 1.0 / 1 / 1 (53 sites) | 1.0 / 1 / 1 (13 sites) |
| dayjs | base | 1.0 / 1 / 1 (34 sites) | – | 1.0 / 1 / 1 (34 sites) | 1.0 / 1 / 1 (4 sites) |
| dayjs | refined | 1.0 / 1 / 1 (41 sites) | 1.0 / 1 / 1 (2 sites) | 1.0 / 1 / 1 (43 sites) | 1.0 / 1 / 1 (30 sites) |
| underscore | base | 1.0 / 1 / 1 (187 sites) | – | 1.0 / 1 / 1 (187 sites) | 1.0 / 1 / 1 (4 sites) |
| underscore | refined | 1.1 / 2 / 4 (270 sites) | 8.67 / 16 / 16 (18 sites) | 1.57 / 2 / 16 (288 sites) | 10.5 / 21 / 21 (40 sites) |

## Precision by tier

At executed sites only: a site counts once the run was seen calling a polint function from it, and a
claimed target counts when it is a function in an instrumented file. **precision** is the share of those
targets the run called there; it is a lower bound, since a run takes only some paths. A target the run never
called on a single-target answer in the exact tier is a **wrong exact target**; the gate is 0.

| package | plan | exact: precision, single / multi not called | conservative: precision, single / multi not called | apart: precision, single / multi not called |
|---|---|---|---|---|
| axios | base | 262/262 (100%), 0 / 0 | – | 11/11 (100%), 0 / 0 |
| axios | refined | 308/312 (99%), 0 / 4 | 50/70 (71%), 2 / 18 | 238/241 (99%), 1 / 2 |
| zod | base | 670/670 (100%), 0 / 0 | – | 51/51 (100%), 0 / 0 |
| zod | refined | 678/683 (99%), 0 / 5 | 31/31 (100%), 0 / 0 | 905/2167 (42%), 0 / 1262 |
| commander | base | 21/21 (100%), 0 / 0 | – | – |
| commander | refined | 36/36 (100%), 0 / 0 | 49/49 (100%), 0 / 0 | 98/98 (100%), 0 / 0 |
| async | base | 324/324 (100%), 0 / 0 | – | – |
| async | refined | 328/329 (100%), 0 / 1 | 20/46 (43%), 0 / 26 | 239/15061 (2%), 0 / 14822 |
| chalk | base | 43/43 (100%), 0 / 0 | – | – |
| chalk | refined | 43/43 (100%), 0 / 0 | 1/1 (100%), 0 / 0 | 12/12 (100%), 0 / 0 |
| dayjs | base | 32/32 (100%), 0 / 0 | – | – |
| dayjs | refined | 34/34 (100%), 0 / 0 | 2/2 (100%), 0 / 0 | 26/26 (100%), 0 / 0 |
| underscore | base | 154/154 (100%), 0 / 0 | – | 1/1 (100%), 0 / 0 |
| underscore | refined | 213/224 (95%), 0 / 11 | 16/58 (28%), 0 / 42 | 22/212 (10%), 0 / 190 |
| **total** | base | **1506/1506 (100%), 0 / 0** | – | **63/63 (100%), 0 / 0** |
| **total** | refined | **1640/1661 (99%), 0 / 21** | **169/257 (66%), 2 / 86** | **1540/17817 (9%), 1 / 16276** |

| package | plan | wrong exact target at | site text | target | the run called |
|---|---|---|---|---|---|
| none | | | | | |

## Open sites

A site is open when it has no exact or conservative target, or the answer marks it unresolved. Each open
site counts under every reason its answer gives; `none recorded` is an open site whose answer gives none,
which the engine must not produce.

| package | plan | call sites | complete | open | open with a claimed target | open by reason |
|---|---|---|---|---|---|---|
| axios | base | 2098 | 488/2098 (23%) | 1610 | 0 | DynamicProperty 857, MissingSemanticReference 521, none recorded 197, CallApplyBind 35 |
| axios | refined | 2098 | 621/2098 (30%) | 1477 | 112 | DynamicProperty 853, MissingSemanticReference 396, none recorded 191, Unknown 112, CallApplyBind 35 |
| zod | base | 6109 | 2506/6109 (41%) | 3603 | 0 | DynamicProperty 2269, MissingSemanticReference 949, none recorded 340, CallApplyBind 45, Reflection 1 |
| zod | refined | 6109 | 2554/6109 (42%) | 3555 | 239 | DynamicProperty 2269, MissingSemanticReference 915, none recorded 314, Unknown 239, CallApplyBind 45, Reflection 1 |
| commander | base | 814 | 42/814 (5%) | 772 | 0 | DynamicProperty 589, MissingSemanticReference 155, none recorded 27, CallApplyBind 1 |
| commander | refined | 814 | 63/814 (8%) | 751 | 75 | DynamicProperty 588, MissingSemanticReference 135, Unknown 75, none recorded 27, CallApplyBind 1 |
| async | base | 1605 | 440/1605 (27%) | 1165 | 0 | MissingSemanticReference 626, DynamicProperty 369, none recorded 156, CallApplyBind 14 |
| async | refined | 1605 | 451/1605 (28%) | 1154 | 48 | MissingSemanticReference 622, DynamicProperty 362, none recorded 155, Unknown 48, CallApplyBind 14, UnsupportedSyntax 1 |
| chalk | base | 144 | 46/144 (32%) | 98 | 0 | DynamicProperty 39, none recorded 35, MissingSemanticReference 24 |
| chalk | refined | 144 | 47/144 (33%) | 97 | 6 | DynamicProperty 39, none recorded 33, MissingSemanticReference 23, Unknown 6 |
| dayjs | base | 181 | 34/181 (19%) | 147 | 0 | DynamicProperty 89, MissingSemanticReference 48, none recorded 10 |
| dayjs | refined | 181 | 41/181 (23%) | 140 | 2 | DynamicProperty 89, MissingSemanticReference 41, none recorded 10, Unknown 2 |
| underscore | base | 455 | 187/455 (41%) | 268 | 0 | MissingSemanticReference 169, DynamicProperty 44, CallApplyBind 41, none recorded 14 |
| underscore | refined | 455 | 270/455 (59%) | 185 | 18 | MissingSemanticReference 86, DynamicProperty 44, CallApplyBind 41, Unknown 18, none recorded 14 |

## Budget events

The solver's budget status for each dump and every budget it reported as reached. The target is none on
the tune set, and a budget event must never discard an edge already derived.

| package | plan | status | reasons |
|---|---|---|---|
| axios | base | NotRun | – |
| axios | refined | BudgetExceeded | adaptation.max_targets_per_source |
| zod | base | NotRun | – |
| zod | refined | WithinBudget | – |
| commander | base | NotRun | – |
| commander | refined | WithinBudget | – |
| async | base | NotRun | – |
| async | refined | BudgetExceeded | adaptation.max_targets_per_source |
| chalk | base | NotRun | – |
| chalk | refined | WithinBudget | – |
| dayjs | base | NotRun | – |
| dayjs | refined | WithinBudget | – |
| underscore | base | NotRun | – |
| underscore | refined | BudgetExceeded | adaptation.max_targets_per_source |

## Refined edges

Every refined row by the tier the bench gives it, its algorithm, status and precision, and whether it
names a function. Rows with a status other than `Resolved` or `Ambiguous` claim nothing.

| package | plan | tier | algorithm | status | precision | names a function | rows |
|---|---|---|---|---|---|---|---|
| axios | refined | none | PointsTo | Unresolved | Unknown | no | 1396 |
| axios | refined | exact | PointsTo | Resolved | SetupAware | yes | 629 |
| axios | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 488 |
| axios | refined | exact | DirectReference | Resolved | SetupAware | yes | 388 |
| axios | refined | apart | PointsTo | Resolved | Heuristic | yes | 371 |
| axios | refined | conservative | PointsTo | Resolved | Conservative | yes | 148 |
| axios | refined | exact | ImportBinding | Resolved | SetupAware | yes | 94 |
| axios | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 67 |
| axios | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 67 |
| axios | refined | exact | ImportBinding | Resolved | SetupAware | no | 8 |
| axios | refined | exact | SummaryAssisted | Resolved | SetupAware | no | 8 |
| axios | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 6 |
| zod | refined | apart | PointsTo | Resolved | Heuristic | yes | 4531 |
| zod | refined | none | PointsTo | Unresolved | Unknown | no | 3468 |
| zod | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 2506 |
| zod | refined | exact | PointsTo | Resolved | SetupAware | yes | 1456 |
| zod | refined | exact | ImportBinding | Resolved | SetupAware | yes | 1300 |
| zod | refined | exact | DirectReference | Resolved | SetupAware | yes | 1081 |
| zod | refined | conservative | PointsTo | Resolved | Conservative | yes | 239 |
| zod | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 160 |
| zod | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 160 |
| zod | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 125 |
| zod | refined | exact | ImportBinding | Resolved | SetupAware | no | 3 |
| zod | refined | exact | SummaryAssisted | Resolved | SetupAware | no | 3 |
| zod | refined | none | PointsTo | Unresolved | Unsupported | no | 1 |
| commander | refined | none | PointsTo | Unresolved | Unknown | no | 799 |
| commander | refined | apart | PointsTo | Resolved | Heuristic | yes | 146 |
| commander | refined | conservative | PointsTo | Resolved | Conservative | yes | 75 |
| commander | refined | exact | PointsTo | Resolved | SetupAware | yes | 64 |
| commander | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 42 |
| commander | refined | exact | DirectReference | Resolved | SetupAware | yes | 41 |
| commander | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 1 |
| async | refined | apart | PointsTo | Resolved | Heuristic | yes | 46397 |
| async | refined | none | PointsTo | Unresolved | Unknown | no | 1046 |
| async | refined | exact | PointsTo | Resolved | SetupAware | yes | 454 |
| async | refined | exact | DirectReference | Resolved | SetupAware | yes | 440 |
| async | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 440 |
| async | refined | conservative | PointsTo | Resolved | Conservative | yes | 224 |
| async | refined | none | FrameworkModel | Unsupported | Conservative | no | 1 |
| chalk | refined | none | PointsTo | Unresolved | Unknown | no | 68 |
| chalk | refined | exact | PointsTo | Resolved | SetupAware | yes | 47 |
| chalk | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 46 |
| chalk | refined | exact | DirectReference | Resolved | SetupAware | yes | 44 |
| chalk | refined | apart | PointsTo | Resolved | Heuristic | yes | 12 |
| chalk | refined | conservative | PointsTo | Resolved | Conservative | yes | 6 |
| chalk | refined | exact | ImportBinding | Resolved | SetupAware | yes | 2 |
| chalk | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 1 |
| chalk | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 1 |
| dayjs | refined | none | PointsTo | Unresolved | Unknown | no | 132 |
| dayjs | refined | exact | PointsTo | Resolved | SetupAware | yes | 41 |
| dayjs | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 34 |
| dayjs | refined | apart | PointsTo | Resolved | Heuristic | yes | 26 |
| dayjs | refined | exact | DirectReference | Resolved | SetupAware | yes | 23 |
| dayjs | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 11 |
| dayjs | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 4 |
| dayjs | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 4 |
| dayjs | refined | conservative | PointsTo | Resolved | Conservative | yes | 2 |
| underscore | refined | apart | PointsTo | Resolved | Heuristic | yes | 416 |
| underscore | refined | exact | PointsTo | Resolved | SetupAware | yes | 296 |
| underscore | refined | none | PointsTo | Unresolved | Unknown | no | 189 |
| underscore | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 187 |
| underscore | refined | exact | DirectReference | Resolved | SetupAware | yes | 186 |
| underscore | refined | conservative | PointsTo | Resolved | Conservative | yes | 156 |
| underscore | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 4 |
| underscore | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 4 |
| underscore | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 1 |

## Sites gained by the refined plan

Sites where a refined plan claims an exact or conservative target and the base plan claims none, by file.
**test path** marks a file in a test or benchmark folder or named `*.test.*` / `*.spec.*`; **executed** counts
the gained sites the run called a polint function from, the only ones recall can credit.

| package | plan | sites gained | in test paths | executed | top files |
|---|---|---|---|---|---|
| axios | refined | 245 | 0 | 84 | lib/adapters/http.js 50, lib/utils.js 45, lib/adapters/fetch.js 21 |
| zod | refined | 287 | 0 | 38 | src/v3/types.ts 77, src/v4/core/schemas.ts 76, src/v4/core/json-schema-processors.ts 20 |
| commander | refined | 96 | 0 | 63 | lib/command.js 56, lib/help.js 26, index.js 4 |
| async | refined | 59 | 0 | 23 | internal/queue.js 10, auto.js 7, asyncify.js 3 |
| chalk | refined | 7 | 0 | 1 | source/index.js 2, source/vendor/supports-color/browser.js 2, source/vendor/supports-color/index.js 2 |
| dayjs | refined | 9 | 0 | 4 | dayjs.min.js 9 |
| underscore | refined | 101 | 0 | 62 | underscore-umd.js 101 |

## Analysed set: globs alone against shipped source

The analysed set of every package is its shipped source: the `analysed` globs without test and benchmark
paths. Reports written before this rule analysed what the globs alone select; each package listed here is
scored both ways once, so this report reads against them. Every other table uses the shipped set.

| package | set | files | source KB | plan | recall | exact / exact + conservative | wrong targets single / multi | wrong exact targets | call sites | dump s | dump peak MB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| zod | globs alone | 332 | 3088 | base | 721/1308 (55%) | 670 / 670 | 0 / 0 | 0 | 49016 | 35.5 | 1365 |
| zod | globs alone | 332 | 3088 | refined | 1117/1308 (85%) | 678 / 709 | 0 / 5 | 0 | 49016 | 96.4 | 4715 |
| zod | shipped | 125 | 1354 | base | 721/1308 (55%) | 670 / 670 | 0 / 0 | 0 | 6109 | 23.6 | 537 |
| zod | shipped | 125 | 1354 | refined | 1181/1308 (90%) | 678 / 709 | 0 / 5 | 0 | 6109 | 64.8 | 1318 |

## Outside tools

Jelly at `62e60eb4fd5b991c5bf4cfc992f018984f560ea4`, run by `bench.py outside` in the bench sandbox and scored
from the committed outputs under `baseline/outside/` with the bench's site and callee matching.
Recall is over observed `call` and `callback` edges. Wrong targets are claimed targets at executed
sites that the run never called there, split by answers with one target and with several.

| package | polint base | polint refined | jelly-static | jelly-approx |
|---|---|---|---|---|
| async | 50.9% | 92.2% | 55.7% | 55.7% |
| axios | 40.4% | 86.2% | 89.2% | 89.3% |
| chalk | 59.7% | 77.8% | 86.1% | 86.1% |
| commander | 6.6% | 57.6% | 96.5% | 96.5% |
| dayjs | 28.6% | 55.4% | 77.7% | 87.5% |
| underscore | 59.6% | 96.2% | 96.2% | 98.9% |
| zod | 55.1% | 90.3% | 91.6% | 92.0% |
| **total** | **46.4%** | **85.8%** | **84.6%** | **85.3%** |

### jelly-static

| package | observed | found | recall | functions | calls | edges | targets per site, mean / p95 / max | wrong single | wrong multi | executed sites | wall s | exit |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| async | 637 | 355 | 55.7% | 504 | 1610 | 14352 | 14.53 / 76 / 82 | 0 | 3679 | 571 | 1.3 | 0 |
| axios | 675 | 602 | 89.2% | 535 | 2281 | 1448 | 1.09 / 2 / 20 | 12 | 78 | 620 | 1.3 | 0 |
| chalk | 72 | 62 | 86.1% | 53 | 148 | 72 | 1.07 / 1 / 6 | 0 | 0 | 63 | 0.7 | 0 |
| commander | 318 | 307 | 96.5% | 296 | 814 | 517 | 1.03 / 1 / 5 | 0 | 4 | 311 | 1.8 | 0 |
| dayjs | 112 | 87 | 77.7% | 57 | 181 | 97 | 1.01 / 1 / 2 | 0 | 0 | 112 | 0.6 | 0 |
| underscore | 260 | 250 | 96.2% | 186 | 455 | 1120 | 3.37 / 26 / 27 | 1 | 330 | 231 | 0.8 | 0 |
| zod | 1308 | 1198 | 91.6% | 2421 | 6502 | 8375 | 1.83 / 1 / 252 | 0 | 1709 | 1079 | 4.2 | 0 |

### jelly-approx

| package | observed | found | recall | functions | calls | edges | targets per site, mean / p95 / max | wrong single | wrong multi | executed sites | wall s | exit |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| async | 637 | 355 | 55.7% | 504 | 1610 | 14352 | 14.53 / 76 / 82 | 0 | 3679 | 571 | 3.1 | 0 |
| axios | 675 | 603 | 89.3% | 535 | 2304 | 1474 | 1.09 / 2 / 20 | 12 | 78 | 620 | 3.2 | 0 |
| chalk | 72 | 62 | 86.1% | 53 | 148 | 72 | 1.07 / 1 / 6 | 0 | 0 | 63 | 1.3 | 0 |
| commander | 318 | 307 | 96.5% | 296 | 814 | 517 | 1.03 / 1 / 5 | 0 | 4 | 311 | 2.9 | 0 |
| dayjs | 112 | 98 | 87.5% | 57 | 181 | 108 | 1.01 / 1 / 2 | 0 | 0 | 112 | 1.2 | 0 |
| underscore | 260 | 257 | 98.9% | 186 | 455 | 2232 | 6.38 / 34 / 116 | 0 | 638 | 231 | 1.6 | 0 |
| zod | 1308 | 1204 | 92.0% | 2421 | 6651 | 9079 | 1.91 / 1 / 252 | 0 | 1867 | 1079 | 9.6 | 0 |

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
| co 4.6.0 | CommonJS, generator and promise control flow | 1 / 1 | 5 | 20 | 20 | 20/20 (100%) | 22 | 12/22 (55%) | 22/22 (100%) |
| pako 2.1.0 | minified UMD (Rollup and terser) | 1 / 1 | 47 | 99 | 94 | 94/99 (95%) | 257 | 223/257 (87%) | 246/257 (96%) |
| nanostores 1.5.5 | ESM source | 14 / 14 | 18 | 97 | 91 | 91/97 (94%) | 103 | 32/103 (31%) | 71/103 (69%) |
| express 5.3.0 | CommonJS framework with dependencies | 7 / 7 | 62 | 109 | 109 | 109/109 (100%) | 166 | 22/166 (13%) | 67/166 (40%) |
| immer 11.2.10 | TypeScript source | 17 / 17 | 96 | 181 | 142 | 142/181 (78%) | 295 | 225/295 (76%) | 251/295 (85%) |
| js-yaml 4.1.0 | CommonJS source, table-driven schema compilation | 25 / 25 | 110 | 146 | 143 | 143/146 (98%) | 338 | 227/338 (67%) | 273/338 (81%) |
| acorn 8.19.0 | bundled ESM output | 1 / 1 | 235 | 358 | 333 | 333/358 (93%) | 1141 | 176/1141 (15%) | 190/1141 (17%) |
| node-forge 1.4.3 | CommonJS, large | 49 / 49 | 972 | 960 | 649 | 649/960 (68%) | 3131 | 577/3131 (18%) | 1900/3131 (61%) |

### Held-out gate

Recall per tier pooled over the tune set and over the unflagged held-out packages (acorn, co, express, immer, js-yaml, nanostores, node-forge, pako; flagged: none). The gate holds when held-out recall is at most 5 points below the tune set's in every tier,
and no held-out package, flagged or not, has a wrong exact target or an answer with other targets and no
unresolved marker.

| plan | tier | tune | held-out | points below tune | within |
|---|---|---|---|---|---|
| base | exact-complete | 44.5% | 27.1% | 17.4 | no |
| base | exact | 44.5% | 27.1% | 17.4 | no |
| base | exact + conservative | 44.5% | 27.1% | 17.4 | no |
| refined | exact-complete | 48.5% | 32.5% | 16.0 | no |
| refined | exact | 48.5% | 32.5% | 16.0 | no |
| refined | exact + conservative | 53.5% | 33.5% | 20.0 | no |

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
| immer | 0 / 0 | 0 / 0 |
| js-yaml | 0 / 0 | 0 / 0 |
| acorn | 0 / 0 | 0 / 0 |
| node-forge | 0 / 0 | 0 / 7 |
| **total** | **0 / 0** | **0 / 7** |

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
| co | refined | 22 | 12/22 (55%) | 12/22 (55%) | 22/22 (100%) | 0/22 (0%) | 0 |
| pako | base | 257 | 215/257 (84%) | 215/257 (84%) | 215/257 (84%) | 8/257 (3%) | 11 |
| pako | refined | 257 | 235/257 (91%) | 235/257 (91%) | 235/257 (91%) | 11/257 (4%) | 36 |
| nanostores | base | 103 | 32/103 (31%) | 32/103 (31%) | 32/103 (31%) | 0/103 (0%) | 0 |
| nanostores | refined | 103 | 32/103 (31%) | 32/103 (31%) | 41/103 (40%) | 30/103 (29%) | 35 |
| express | base | 166 | 22/166 (13%) | 22/166 (13%) | 22/166 (13%) | 0/166 (0%) | 0 |
| express | refined | 166 | 33/166 (20%) | 33/166 (20%) | 43/166 (26%) | 24/166 (14%) | 25 |
| immer | base | 295 | 225/295 (76%) | 225/295 (76%) | 225/295 (76%) | 0/295 (0%) | 1 |
| immer | refined | 295 | 226/295 (77%) | 226/295 (77%) | 237/295 (80%) | 14/295 (5%) | 23 |
| js-yaml | base | 338 | 227/338 (67%) | 227/338 (67%) | 227/338 (67%) | 0/338 (0%) | 2 |
| js-yaml | refined | 338 | 265/338 (78%) | 265/338 (78%) | 273/338 (81%) | 0/338 (0%) | 4 |
| acorn | base | 1141 | 176/1141 (15%) | 176/1141 (15%) | 176/1141 (15%) | 0/1141 (0%) | 3 |
| acorn | refined | 1141 | 179/1141 (16%) | 179/1141 (16%) | 181/1141 (16%) | 9/1141 (1%) | 23 |
| node-forge | base | 3131 | 570/3131 (18%) | 570/3131 (18%) | 570/3131 (18%) | 7/3131 (0%) | 9 |
| node-forge | refined | 3131 | 789/3131 (25%) | 789/3131 (25%) | 792/3131 (25%) | 1108/3131 (35%) | 1631 |
| **total** | base | 5453 | 1479/5453 (27%) | 1479/5453 (27%) | 1479/5453 (27%) | 15/5453 (0%) | 26 |
| **total** | refined | 5453 | 1771/5453 (32%) | 1771/5453 (32%) | 1824/5453 (33%) | 1196/5453 (22%) | 1777 |

Targets per site that claims any in the tier: mean / p95 (nearest rank) / max.

| package | plan | exact | conservative | exact + conservative | apart |
|---|---|---|---|---|---|
| co | base | 1.0 / 1 / 1 (12 sites) | – | 1.0 / 1 / 1 (12 sites) | – |
| co | refined | 1.0 / 1 / 1 (12 sites) | 1.08 / 2 / 2 (13 sites) | 1.04 / 1 / 2 (25 sites) | – |
| pako | base | 1.0 / 1 / 1 (249 sites) | – | 1.0 / 1 / 1 (249 sites) | 1.0 / 1 / 1 (11 sites) |
| pako | refined | 1.0 / 1 / 1 (276 sites) | – | 1.0 / 1 / 1 (276 sites) | 1.0 / 1 / 1 (25 sites) |
| nanostores | base | 1.0 / 1 / 1 (36 sites) | – | 1.0 / 1 / 1 (36 sites) | – |
| nanostores | refined | 1.0 / 1 / 1 (37 sites) | 1.0 / 1 / 1 (15 sites) | 1.0 / 1 / 1 (52 sites) | 1.52 / 2 / 5 (23 sites) |
| express | base | 1.0 / 1 / 1 (24 sites) | – | 1.0 / 1 / 1 (24 sites) | – |
| express | refined | 1.0 / 1 / 1 (35 sites) | 1.0 / 1 / 1 (13 sites) | 1.0 / 1 / 1 (48 sites) | 1.0 / 1 / 1 (25 sites) |
| immer | base | 1.0 / 1 / 1 (302 sites) | – | 1.0 / 1 / 1 (302 sites) | 1.0 / 1 / 1 (1 sites) |
| immer | refined | 1.0 / 1 / 1 (303 sites) | 1.07 / 2 / 2 (15 sites) | 1.0 / 1 / 2 (318 sites) | 2.2 / 6 / 6 (10 sites) |
| js-yaml | base | 1.0 / 1 / 1 (254 sites) | – | 1.0 / 1 / 1 (254 sites) | 1.0 / 1 / 1 (2 sites) |
| js-yaml | refined | 1.0 / 1 / 1 (305 sites) | 1.0 / 1 / 1 (8 sites) | 1.0 / 1 / 1 (313 sites) | 1.0 / 1 / 1 (2 sites) |
| acorn | base | 1.0 / 1 / 1 (197 sites) | – | 1.0 / 1 / 1 (197 sites) | 1.0 / 1 / 1 (3 sites) |
| acorn | refined | 1.0 / 1 / 2 (201 sites) | 1.0 / 1 / 1 (2 sites) | 1.0 / 1 / 2 (203 sites) | 1.0 / 1 / 1 (20 sites) |
| node-forge | base | 1.0 / 1 / 1 (664 sites) | – | 1.0 / 1 / 1 (664 sites) | 1.0 / 1 / 1 (9 sites) |
| node-forge | refined | 1.01 / 1 / 5 (947 sites) | 1.74 / 4 / 4 (19 sites) | 1.02 / 1 / 5 (966 sites) | 1.15 / 2 / 6 (1416 sites) |

## Precision by tier (held-out)

At executed sites only: a site counts once the run was seen calling a polint function from it, and a
claimed target counts when it is a function in an instrumented file. **precision** is the share of those
targets the run called there; it is a lower bound, since a run takes only some paths. A target the run never
called on a single-target answer in the exact tier is a **wrong exact target**; the gate is 0.

| package | plan | exact: precision, single / multi not called | conservative: precision, single / multi not called | apart: precision, single / multi not called |
|---|---|---|---|---|
| co | base | 12/12 (100%), 0 / 0 | – | – |
| co | refined | 12/12 (100%), 0 / 0 | 10/10 (100%), 0 / 0 | – |
| pako | base | 215/215 (100%), 0 / 0 | – | 8/8 (100%), 0 / 0 |
| pako | refined | 235/235 (100%), 0 / 0 | – | 19/19 (100%), 0 / 0 |
| nanostores | base | 32/32 (100%), 0 / 0 | – | – |
| nanostores | refined | 32/32 (100%), 0 / 0 | 9/9 (100%), 0 / 0 | 30/34 (88%), 0 / 4 |
| express | base | 22/22 (100%), 0 / 0 | – | – |
| express | refined | 33/33 (100%), 0 / 0 | 10/10 (100%), 0 / 0 | 24/24 (100%), 0 / 0 |
| immer | base | 225/225 (100%), 0 / 0 | – | – |
| immer | refined | 226/226 (100%), 0 / 0 | 11/11 (100%), 0 / 0 | 14/17 (82%), 0 / 3 |
| js-yaml | base | 227/227 (100%), 0 / 0 | – | – |
| js-yaml | refined | 265/265 (100%), 0 / 0 | 8/8 (100%), 0 / 0 | – |
| acorn | base | 176/176 (100%), 0 / 0 | – | 0/3 (0%), 3 / 0 |
| acorn | refined | 179/179 (100%), 0 / 0 | 2/2 (100%), 0 / 0 | 9/12 (75%), 3 / 0 |
| node-forge | base | 570/570 (100%), 0 / 0 | – | 7/7 (100%), 0 / 0 |
| node-forge | refined | 789/792 (100%), 0 / 3 | 3/7 (43%), 0 / 4 | 1115/1169 (95%), 4 / 50 |
| **total** | base | **1479/1479 (100%), 0 / 0** | – | **15/18 (83%), 3 / 0** |
| **total** | refined | **1771/1774 (100%), 0 / 3** | **53/57 (93%), 0 / 4** | **1211/1275 (95%), 7 / 57** |

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
| co | refined | 46 | 12/46 (26%) | 34 | 13 | Unknown 13, CallApplyBind 12, MissingSemanticReference 11, DynamicProperty 7, none recorded 4 |
| pako | base | 430 | 249/430 (58%) | 181 | 0 | MissingSemanticReference 91, DynamicProperty 73, none recorded 9, CallApplyBind 8 |
| pako | refined | 430 | 276/430 (64%) | 154 | 0 | DynamicProperty 73, MissingSemanticReference 64, none recorded 9, CallApplyBind 8 |
| nanostores | base | 164 | 36/164 (22%) | 128 | 0 | MissingSemanticReference 62, DynamicProperty 56, none recorded 10 |
| nanostores | refined | 164 | 37/164 (23%) | 127 | 15 | MissingSemanticReference 61, DynamicProperty 56, Unknown 15, none recorded 10 |
| express | base | 466 | 24/466 (5%) | 442 | 0 | DynamicProperty 192, MissingSemanticReference 186, none recorded 53, CallApplyBind 11, UnknownCallee 1 |
| express | refined | 466 | 35/466 (8%) | 431 | 13 | DynamicProperty 192, MissingSemanticReference 175, none recorded 51, Unknown 13, CallApplyBind 11, UnknownCallee 1 |
| immer | base | 575 | 302/575 (53%) | 273 | 0 | MissingSemanticReference 112, DynamicProperty 110, none recorded 33, CallApplyBind 18 |
| immer | refined | 575 | 303/575 (53%) | 272 | 15 | MissingSemanticReference 111, DynamicProperty 110, none recorded 33, CallApplyBind 18, Unknown 15 |
| js-yaml | base | 709 | 254/709 (36%) | 455 | 0 | DynamicProperty 212, MissingSemanticReference 159, none recorded 55, CallApplyBind 29 |
| js-yaml | refined | 709 | 305/709 (43%) | 404 | 8 | DynamicProperty 193, MissingSemanticReference 127, none recorded 55, CallApplyBind 29, Unknown 8 |
| acorn | base | 1809 | 197/1809 (11%) | 1612 | 0 | DynamicProperty 1539, MissingSemanticReference 47, none recorded 19, CallApplyBind 6, DynamicImport 1 |
| acorn | refined | 1809 | 201/1809 (11%) | 1608 | 2 | DynamicProperty 1539, MissingSemanticReference 43, none recorded 19, CallApplyBind 6, Unknown 2, DynamicImport 1 |
| node-forge | base | 5283 | 664/5283 (13%) | 4619 | 0 | DynamicProperty 3293, MissingSemanticReference 1080, none recorded 231, CallApplyBind 15 |
| node-forge | refined | 5283 | 947/5283 (18%) | 4336 | 19 | DynamicProperty 3152, MissingSemanticReference 938, none recorded 231, Unknown 19, CallApplyBind 15 |

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
| node-forge | refined | WithinBudget | – |

## Refined edges (held-out)

Every refined row by the tier the bench gives it, its algorithm, status and precision, and whether it
names a function. Rows with a status other than `Resolved` or `Ambiguous` claim nothing.

| package | plan | tier | algorithm | status | precision | names a function | rows |
|---|---|---|---|---|---|---|---|
| co | refined | none | PointsTo | Unresolved | Unknown | no | 43 |
| co | refined | conservative | PointsTo | Resolved | Conservative | yes | 14 |
| co | refined | exact | DirectReference | Resolved | SetupAware | yes | 12 |
| co | refined | exact | PointsTo | Resolved | SetupAware | yes | 12 |
| co | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 12 |
| pako | refined | exact | PointsTo | Resolved | SetupAware | yes | 276 |
| pako | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 249 |
| pako | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 148 |
| pako | refined | none | PointsTo | Unresolved | Unknown | no | 145 |
| pako | refined | exact | DirectReference | Resolved | SetupAware | yes | 101 |
| pako | refined | apart | PointsTo | Resolved | Heuristic | yes | 14 |
| pako | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 11 |
| pako | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 11 |
| nanostores | refined | none | PointsTo | Unresolved | Unknown | no | 132 |
| nanostores | refined | exact | PointsTo | Resolved | SetupAware | yes | 37 |
| nanostores | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 36 |
| nanostores | refined | apart | PointsTo | Resolved | Heuristic | yes | 35 |
| nanostores | refined | exact | DirectReference | Resolved | SetupAware | yes | 22 |
| nanostores | refined | conservative | PointsTo | Resolved | Conservative | yes | 15 |
| nanostores | refined | exact | ImportBinding | Resolved | SetupAware | yes | 14 |
| express | refined | none | PointsTo | Unresolved | Unknown | no | 392 |
| express | refined | exact | PointsTo | Resolved | SetupAware | yes | 35 |
| express | refined | apart | PointsTo | Resolved | Heuristic | yes | 25 |
| express | refined | exact | DirectReference | Resolved | SetupAware | yes | 24 |
| express | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 24 |
| express | refined | conservative | PointsTo | Resolved | Conservative | yes | 13 |
| immer | refined | exact | PointsTo | Resolved | SetupAware | yes | 303 |
| immer | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 302 |
| immer | refined | none | PointsTo | Unresolved | Unknown | no | 254 |
| immer | refined | exact | ImportBinding | Resolved | SetupAware | yes | 173 |
| immer | refined | exact | DirectReference | Resolved | SetupAware | yes | 127 |
| immer | refined | apart | PointsTo | Resolved | Heuristic | yes | 21 |
| immer | refined | conservative | PointsTo | Resolved | Conservative | yes | 16 |
| immer | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 2 |
| immer | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 1 |
| immer | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 1 |
| js-yaml | refined | none | PointsTo | Unresolved | Unknown | no | 357 |
| js-yaml | refined | exact | PointsTo | Resolved | SetupAware | yes | 305 |
| js-yaml | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 254 |
| js-yaml | refined | exact | DirectReference | Resolved | SetupAware | yes | 252 |
| js-yaml | refined | conservative | PointsTo | Resolved | Conservative | yes | 8 |
| js-yaml | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 2 |
| js-yaml | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 2 |
| js-yaml | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 2 |
| acorn | refined | none | PointsTo | Unresolved | Unknown | no | 1591 |
| acorn | refined | exact | PointsTo | Resolved | SetupAware | yes | 202 |
| acorn | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 197 |
| acorn | refined | exact | DirectReference | Resolved | SetupAware | yes | 136 |
| acorn | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 61 |
| acorn | refined | apart | PointsTo | Resolved | Heuristic | yes | 17 |
| acorn | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 3 |
| acorn | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 3 |
| acorn | refined | conservative | PointsTo | Resolved | Conservative | yes | 2 |
| node-forge | refined | none | PointsTo | Unresolved | Unknown | no | 4124 |
| node-forge | refined | apart | PointsTo | Resolved | Heuristic | yes | 1613 |
| node-forge | refined | exact | PointsTo | Resolved | SetupAware | yes | 956 |
| node-forge | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 664 |
| node-forge | refined | exact | DirectReference | Resolved | SetupAware | yes | 610 |
| node-forge | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 54 |
| node-forge | refined | conservative | PointsTo | Resolved | Conservative | yes | 33 |
| node-forge | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 9 |
| node-forge | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 9 |

## Sites gained by the refined plan (held-out)

Sites where a refined plan claims an exact or conservative target and the base plan claims none, by file.
**test path** marks a file in a test or benchmark folder or named `*.test.*` / `*.spec.*`; **executed** counts
the gained sites the run called a polint function from, the only ones recall can credit.

| package | plan | sites gained | in test paths | executed | top files |
|---|---|---|---|---|---|
| co | refined | 13 | 0 | 10 | index.js 13 |
| pako | refined | 27 | 0 | 20 | dist/pako.min.js 27 |
| nanostores | refined | 16 | 0 | 9 | computed/index.js 5, effect/index.js 3, task/index.js 3 |
| express | refined | 24 | 0 | 21 | lib/response.js 14, lib/application.js 6, lib/utils.js 3 |
| immer | refined | 16 | 0 | 12 | src/plugins/patches.ts 6, src/plugins/mapset.ts 3, src/core/proxy.ts 2 |
| js-yaml | refined | 59 | 0 | 46 | lib/dumper.js 12, lib/loader.js 10, lib/schema.js 10 |
| acorn | refined | 6 | 0 | 5 | dist/acorn.mjs 6 |
| node-forge | refined | 302 | 0 | 218 | lib/x509.js 53, lib/rsa.js 41, lib/tls.js 41 |

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
| package chalk | 1/5 | 10/29 (34%) | 10/16 (62%) | 1/5 | 25/29 (86%) | 25/25 (100%) | Default/Resolved 4, Named/Resolved 12, Namespace/Unresolved 9 | 0 |
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
| case block-function/catch-and-finally | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | – | 0 |
| case block-function/try-and-if | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | – | 0 |
| case cjs-exports-property/exports-of-factory-result | 0/1 | 0/1 (0%) | – | 1/1 | 1/1 (100%) | 1/1 (100%) | CommonJsExportsProperty/Unsupported 1 | 0 |
| case cjs-exports-property/function-expressions | 0/1 | 0/2 (0%) | – | 1/1 | 2/2 (100%) | 2/2 (100%) | CommonJsExportsProperty/Unsupported 2 | 0 |
| case cjs-exports-property/renamed-destructuring | 0/1 | 0/2 (0%) | – | 1/1 | 2/2 (100%) | 2/2 (100%) | CommonJsExportsProperty/Unsupported 2 | 0 |
| case cjs-interop/compiled-default | 0/1 | 0/1 (0%) | – | 1/1 | 1/1 (100%) | 1/1 (100%) | CommonJsExportsProperty/Unsupported 1 | 0 |
| case cjs-interop/define-property-export | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | – | 0 |
| case cjs-module-exports/function-with-property | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case cjs-module-exports/object-literal | 0/1 | 0/2 (0%) | – | 0/1 | 0/2 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case class-method/subclass-override | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case computed-key-unknown/escaped-object | 0/1 | 0/2 (0%) | – | 0/1 | 0/2 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case dynamic-import/computed-specifier | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case dynamic-import/literal-specifier | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case esm-namespace/namespace-object | 1/1 | 2/2 (100%) | 2/2 (100%) | 1/1 | 2/2 (100%) | 2/2 (100%) | Named/Resolved 2 | 0 |
| case esm-namespace/star-and-live-binding | 2/3 | 3/6 (50%) | 3/3 (100%) | 2/3 | 3/6 (50%) | 3/3 (100%) | Named/Resolved 3, StarReexport/Unresolved 2 | 0 |
| case esm-reexport/named-barrel | 2/3 | 2/4 (50%) | 2/2 (100%) | 3/3 | 4/4 (100%) | 4/4 (100%) | Named/Resolved 2, Namespace/Unresolved 2 | 0 |
| case esm-reexport/star-and-alias | 2/3 | 2/4 (50%) | 2/2 (100%) | 2/3 | 3/4 (75%) | 3/3 (100%) | Named/Resolved 2, Namespace/Unresolved 1, StarReexport/Unresolved 1 | 0 |
| case exported-initializer/cjs-exports | 0/1 | 0/2 (0%) | – | 0/1 | 1/2 (50%) | 1/1 (100%) | CommonJsExportsProperty/Unsupported 1 | 0 |
| case exported-initializer/esm-declarators | 0/1 | 2/4 (50%) | 2/2 (100%) | 0/1 | 3/4 (75%) | 3/3 (100%) | Default/Resolved 1, Named/Resolved 2 | 0 |
| case function-object/factory-made-constructor | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case module-initializer/arrow-in-initializer | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case module-initializer/chained-call | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case module-initializer/class-expression | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case module-initializer/exported-call | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case module-initializer/object-literal-methods | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case package-vendor/cjs-require | 0/2 | 0/3 (0%) | – | 0/2 | 0/3 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case package-vendor/esm-import | 2/2 | 2/2 (100%) | 2/2 (100%) | 2/2 | 2/2 (100%) | 2/2 (100%) | Named/Resolved 2 | 0 |
| case same-name-member/cjs-static-method | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case same-name-member/wrapper-of-static-method | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case sequence-callee/compiled-interop | 0/3 | 0/3 (0%) | – | 3/3 | 3/3 (100%) | 3/3 (100%) | CommonJsExportsProperty/Unsupported 3 | 0 |
| case sequence-callee/local-sequence | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case unary-iife/umd-prologue | 0/1 | 0/2 (0%) | – | 0/1 | 0/2 (0%) | – | – | 0 |
| case unary-iife/unary-operators | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |

## Cost

The only part of this report that changes between runs. Machine: Linux-6.8.0-147-generic-x86_64-with-glibc2.39, 32 CPUs, commit 3d1c021dcd78, load average 52.61 / 73.89 / 87.72 at the start and 16.51 / 61.13 / 82.92 at the end (1, 5, 15 minutes).

`polint check` (release) with the census rule pack, which asks for `Functions`, `CallGraph`, `Symbols`, `References`; median of 3 runs. `analysis` is the in-process cost record (`POLINT_GOLDEN_COST_PATH`), `process` is the whole CLI as `bench_matrix.measure` sees it. Cold has no `.polint/cache`; warm reruns over the cache the cold run left. Stored bytes are `.polint/cache` after a cold run, raw and gzip-compressed. Per MB means per 1,000,000 source bytes. Process peak RSS comes from `wait4`, which on Linux counts the launching Python process's resident size at fork, so no process figure here is below the launcher's peak of 36 MB; the analysis column has no such floor.

| tree | source KB | cold analysis ms (per MB) | cold process ms | warm analysis ms (per MB) | warm process ms | cold peak RSS MB (analysis / process) | warm peak RSS MB | stored bytes per MB (gzip) |
|---|---|---|---|---|---|---|---|---|
| constructs (all cases) | 34 | 201 (5840) | 333 | 110 (3196) | 229 | 72 / 72 | 73 / 73 | 106780561 (10072701) |
| axios | 282 | 694 (2461) | 1154 | 476 (1688) | 602 | 102 / 102 | 103 / 103 | 75884595 (5415395) |
| zod | 1354 | 4451 (3287) | 5111 | 2858 (2110) | 3064 | 524 / 524 | 668 / 668 | 127822333 (10464689) |
| commander | 127 | 259 (2046) | 458 | 203 (1604) | 332 | 73 / 73 | 72 / 72 | 44296772 (3204191) |
| async | 321 | 538 (1678) | 672 | 344 (1073) | 496 | 80 / 80 | 78 / 78 | 44447687 (3482287) |
| chalk | 22 | 71 (3292) | 241 | 45 (2086) | 196 | 73 / 73 | 73 / 73 | 78773646 (5811805) |
| dayjs | 7 | 72 (10054) | 213 | 51 (7122) | 196 | 73 / 73 | 72 / 72 | 228136294 (13651445) |
| underscore | 69 | 223 (3236) | 357 | 161 (2336) | 296 | 73 / 73 | 73 / 73 | 91434282 (5881609) |

Fact dump (no cache, one tree per process; peak RSS is the process figure, with the same floor): this is
the only way to time the refined plan, because no public view asks for it.

| tree | base ms | base peak RSS MB | refined ms | refined peak RSS MB | budget s |
|---|---|---|---|---|---|
| axios | 3823 | 122 | 11417 | 391 | none |
| zod | 23606 | 537 | 64810 | 1318 | 180 |
| commander | 1474 | 64 | 10571 | 189 | none |
| async | 2516 | 98 | 47483 | 1552 | none |
| chalk | 780 | 39 | 1062 | 67 | none |
| dayjs | 309 | 39 | 1242 | 71 | none |
| underscore | 808 | 56 | 4026 | 174 | none |

### Stage times

Every fact dump runs with `RUST_LOG=polint::kernel::stage=info`; each provider's `stage done` lines are summed. Stage times are as noisy as the dump times above. `semantic_graph share` is the `polint.semantic_graph` stage's part of the refined plan's summed stage time. Peak MB is the process peak when the stage ended.

| tree | plan | stage total s | largest stages: s (peak MB) | semantic_graph share |
|---|---|---|---|---|
| axios | base | 3.5 | `cfg` 1.3 (107), `symbol_graph` 0.9 (89), `semantic_mir` 0.8 (89) | n/a |
| axios | refined | 10.3 | `type_value_alias` 1.7 (238), `semantic_graph` 1.6 (251), `evidence` 1.6 (340) | 16% |
| zod | base | 21.8 | `symbol_graph` 9.3 (511), `semantic_mir` 9.1 (511), `cfg` 2.5 (511) | n/a |
| zod | refined | 60.6 | `semantic_graph` 13.9 (920), `solver` 9.7 (1015), `type_value_alias` 7.3 (825) | 23% |
| commander | base | 1.2 | `semantic_mir` 0.5 (47), `symbol_graph` 0.4 (43), `cfg` 0.2 (57) | n/a |
| commander | refined | 10.1 | `semantic_graph` 4.2 (155), `type_value_alias` 1.2 (155), `cfg` 1.1 (90) | 42% |
| async | base | 1.9 | `semantic_mir` 0.6 (69), `symbol_graph` 0.6 (68), `cfg` 0.5 (86) | n/a |
| async | refined | 42.1 | `evidence` 13.0 (1253), `data_flow` 8.7 (1253), `solver` 7.9 (358) | 11% |
| chalk | base | 0.6 | `semantic_mir` 0.2 (34), `symbol_graph` 0.2 (33), `cfg` 0.2 (35) | n/a |
| chalk | refined | 0.9 | `cfg` 0.2 (40), `type_value_alias` 0.2 (54), `semantic_graph` 0.1 (56) | 12% |
| dayjs | base | 0.2 | `cfg` 0.1 (35), `semantic_mir` 0.1 (33), `symbol_graph` 0.1 (33) | n/a |
| dayjs | refined | 1.1 | `semantic_graph` 0.3 (57), `cfg` 0.1 (40), `type_value_alias` 0.1 (56) | 28% |
| underscore | base | 0.7 | `symbol_graph` 0.3 (44), `semantic_mir` 0.2 (44), `cfg` 0.2 (50) | n/a |
| underscore | refined | 3.6 | `evidence` 0.7 (154), `cfg` 0.6 (83), `solver` 0.5 (129) | 6% |
| co | base | 0.1 | `semantic_mir` 0.0 (33), `cfg` 0.0 (33), `symbol_graph` 0.0 (33) | n/a |
| co | refined | 0.2 | `cfg` 0.1 (33), `evidence` 0.0 (42), `semantic_graph` 0.0 (39) | 10% |
| pako | base | 3.2 | `symbol_graph` 1.9 (59), `semantic_mir` 0.7 (59), `cfg` 0.5 (68) | n/a |
| pako | refined | 21.7 | `semantic_graph` 13.6 (248), `type_value_alias` 2.0 (248), `symbol_graph` 1.5 (61) | 63% |
| nanostores | base | 0.2 | `symbol_graph` 0.1 (33), `semantic_mir` 0.1 (33), `cfg` 0.0 (36) | n/a |
| nanostores | refined | 1.1 | `semantic_graph` 0.2 (61), `symbol_graph` 0.2 (33), `data_flow` 0.2 (68) | 16% |
| express | base | 0.6 | `semantic_mir` 0.2 (40), `symbol_graph` 0.2 (37), `cfg` 0.1 (46) | n/a |
| express | refined | 2.6 | `cfg` 0.7 (60), `solver` 0.3 (95), `symbol_graph` 0.3 (37) | 9% |
| immer | base | 0.6 | `symbol_graph` 0.3 (56), `semantic_mir` 0.1 (56), `cfg` 0.1 (56) | n/a |
| immer | refined | 3.1 | `cfg` 0.6 (77), `solver` 0.4 (132), `symbol_graph` 0.4 (57) | 8% |
| js-yaml | base | 1.1 | `symbol_graph` 0.4 (55), `semantic_mir` 0.3 (55), `cfg` 0.3 (65) | n/a |
| js-yaml | refined | 4.8 | `abstract_domains` 0.9 (131), `type_value_alias` 0.7 (175), `cfg` 0.6 (75) | 9% |
| acorn | base | 2.1 | `semantic_mir` 1.0 (76), `symbol_graph` 0.6 (64), `cfg` 0.4 (98) | n/a |
| acorn | refined | 7.3 | `type_value_alias` 1.5 (276), `semantic_graph` 1.2 (276), `solver` 1.0 (276) | 16% |
| node-forge | base | 4.1 | `symbol_graph` 1.8 (193), `semantic_mir` 1.3 (193), `cfg` 0.7 (251) | n/a |
| node-forge | refined | 19.3 | `solver` 4.0 (677), `type_value_alias` 3.5 (566), `evidence` 2.9 (811) | 14% |
| zod, globs alone | base | 31.6 | `symbol_graph` 13.1 (881), `semantic_mir` 11.9 (881), `calls` 3.5 (1017) | n/a |
| zod, globs alone | refined | 87.5 | `semantic_graph` 16.3 (2830), `solver` 14.7 (3269), `type_value_alias` 11.9 (2377) | 19% |

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
- A package is analysed as the package it is: its `analysed` globs are the include set (a construct case includes all its files), and of polint's default excludes only `**/node_modules/**` and `**/.git/**` apply, so a package's own `vendor/` is analysed; observed edges touching an excluded file count as 'site or callee in a file polint does not analyse'.
- Missing call sites and functions are not worked around: a function or call polint does not extract counts as a miss.
- An observed callee maps to the polint function with the same span before the innermost one ending at the same byte: a concise arrow returning an arrow, `(a) => (b) => ...`, ends at the same byte as the arrow it returns.
- Every missed observed edge is attributed to one gap from a syntax census: Oxc's own function and call spans with their syntactic context, parsed again apart from the analysis. A site or callee polint does not extract is charged to the outermost function around it that polint does not extract, else to the outermost enclosing function or the call in a gap; edges polint maps go to constructors, callbacks to built-ins, computed keys and prototype or class methods, in that order. A computed-key miss is counted whether or not the key set is known, so that row is an upper bound for computed keys over a known set.
- A census function counts as extracted when polint has a function in the same file ending at the same byte, or, for a class constructor, when polint has the class.
- Every package edge with other targets and no unresolved marker is kept as an example; other outcomes and gaps keep a bounded sample, in observed-edge order.
- Process peak RSS is measured with `wait4` like the matrix runner does. Linux carries the parent's resident size into the child's peak at fork, so every process is launched before the report loads any fact dump, and the launcher's own peak is recorded with the cost section. The in-process analysis peak has no such floor.
- Each fact dump has a wall-clock limit (`--dump-timeout`, 600 s by default); a tree whose dump exceeds it is reported as timed out for that plan and has no scores in that column.
- A package with `dump_budget_s` in `inputs.toml` has a time budget for each of its fact dumps; the report is written either way, and the run exits with status 1 naming every dump that ran over its budget or timed out.
