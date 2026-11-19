# JS/TS analysis benchmark

polint 0.5.3. Written by `python3 tests/js-analysis-bench/bench.py report`;
see `tests/js-analysis-bench/README.md` for the measures, the decisions and their limits.
There is no overall accuracy number on purpose: read the rows.

- **base** is what a rule gets from `CallGraph` (capabilities `syntax, call_graph, symbols, references`).
- **refined** is the raw refined call-edge rows, every status included (adds `calls, control_flow,
  dataflow`). `CallGraph` serves the rows of this layer that claim a target, so the two columns differ only
  where a row's status withholds or weakens its claim.
- Precision is measured against hand labels only. **Observed calls give recall, never precision**:
  a run sees the paths it exercised, so an edge polint reports that the run did not take is not wrong.

## Constructs (hand labels)

Counts are summed over each construct's cases. `held out` cases must not be used to tune analysis;
compare their rows with the other cases to see whether a change generalises.

| construct | cases (held out) | call recall base | call recall refined | callback recall base | callback recall refined | precision base | precision refined | hidden paths base: reported / false-complete / silent | hidden paths refined | false-complete call lines base / refined | stops at import binding (base) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| accessor | 2 (1) | 3/6 (50%) | 3/6 (50%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0 / 0 | 0 |
| argument-flow | 2 (1) | 14/14 (100%) | 14/14 (100%) | 1/1 (100%) | 1/1 (100%) | 15/15 (100%) | 15/15 (100%) | – | – | 0 / 0 | 0 |
| await | 2 (1) | 6/6 (100%) | 6/6 (100%) | – | – | 6/6 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| block-function | 2 (1) | 9/9 (100%) | 9/9 (100%) | – | – | 9/9 (100%) | 9/9 (100%) | – | – | 0 / 0 | 0 |
| call-apply-bind | 2 (1) | 5/5 (100%) | 5/5 (100%) | – | – | 5/5 (100%) | 5/5 (100%) | – | – | 0 / 0 | 0 |
| callback | 3 (1) | 9/9 (100%) | 9/9 (100%) | 4/4 (100%) | 4/4 (100%) | 12/12 (100%) | 12/12 (100%) | – | – | 0 / 0 | 0 |
| cjs-exports-property | 3 (1) | 6/6 (100%) | 6/6 (100%) | – | – | 6/6 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| cjs-interop | 2 (1) | 3/4 (75%) | 3/4 (75%) | – | – | 3/4 (75%) | 3/4 (75%) | – | – | 0 / 0 | 0 |
| cjs-module-exports | 2 (1) | 6/6 (100%) | 6/6 (100%) | – | – | 6/6 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| class-features | 2 (1) | 12/13 (92%) | 12/13 (92%) | – | – | 12/14 (86%) | 12/14 (86%) | – | – | 0 / 0 | 0 |
| class-method | 2 (1) | 9/9 (100%) | 9/9 (100%) | – | – | 9/9 (100%) | 9/9 (100%) | – | – | 0 / 0 | 0 |
| closure | 3 (1) | 9/11 (82%) | 9/11 (82%) | – | – | 9/9 (100%) | 9/9 (100%) | – | – | 0 / 0 | 0 |
| computed-key-foldable | 2 (1) | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 0 / 0 | 0 |
| computed-key-unknown | 2 (1) | 4/6 (67%) | 4/6 (67%) | – | – | 4/4 (100%) | 4/4 (100%) | 1 / 0 / 0 | 1 / 0 / 0 | 0 / 0 | 0 |
| constant-key-field | 2 (1) | 6/6 (100%) | 6/6 (100%) | – | – | 6/6 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| destructuring | 2 (1) | 7/7 (100%) | 7/7 (100%) | – | – | 7/7 (100%) | 7/7 (100%) | – | – | 0 / 0 | 0 |
| dynamic-import | 2 (0) | 1/2 (50%) | 1/2 (50%) | – | – | 1/1 (100%) | 1/1 (100%) | 1 / 0 / 0 | 1 / 0 / 0 | 0 / 0 | 0 |
| dynamic-property | 2 (1) | 0/2 (0%) | 0/2 (0%) | – | – | – | – | 1 / 0 / 0 | 1 / 0 / 0 | 0 / 0 | 0 |
| esm-namespace | 2 (1) | 7/7 (100%) | 7/7 (100%) | – | – | 7/9 (78%) | 7/9 (78%) | – | – | 0 / 0 | 0 |
| esm-reexport | 2 (1) | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 0 / 0 | 0 |
| eval | 2 (1) | – | – | – | – | – | – | 2 / 0 / 0 | 2 / 0 / 0 | 0 / 0 | 0 |
| exported-initializer | 2 (1) | 6/6 (100%) | 6/6 (100%) | – | – | 6/6 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| function-object | 2 (1) | 5/5 (100%) | 5/5 (100%) | – | – | 5/5 (100%) | 5/5 (100%) | – | – | 0 / 0 | 0 |
| iteration | 2 (1) | 8/8 (100%) | 8/8 (100%) | – | – | 8/9 (89%) | 8/9 (89%) | – | – | 0 / 0 | 0 |
| literal-spread | 2 (1) | 5/7 (71%) | 5/7 (71%) | – | – | 5/5 (100%) | 5/5 (100%) | – | – | 0 / 0 | 0 |
| logical-join | 2 (1) | 21/21 (100%) | 21/21 (100%) | – | – | 21/21 (100%) | 21/21 (100%) | – | – | 0 / 0 | 0 |
| module-initializer | 6 (2) | 23/23 (100%) | 23/23 (100%) | – | – | 23/23 (100%) | 23/23 (100%) | – | – | 0 / 0 | 0 |
| new-and-prototype | 2 (1) | 8/8 (100%) | 8/8 (100%) | – | – | 8/8 (100%) | 8/8 (100%) | – | – | 0 / 0 | 0 |
| object-of-functions | 3 (1) | 8/8 (100%) | 8/8 (100%) | – | – | 8/8 (100%) | 8/8 (100%) | – | – | 0 / 0 | 0 |
| optional-chaining | 2 (1) | 5/5 (100%) | 5/5 (100%) | – | – | 5/5 (100%) | 5/5 (100%) | – | – | 0 / 0 | 0 |
| package-vendor | 2 (1) | 6/6 (100%) | 6/6 (100%) | – | – | 6/6 (100%) | 6/6 (100%) | – | – | 0 / 0 | 0 |
| promise | 2 (1) | 4/4 (100%) | 4/4 (100%) | 1/1 (100%) | 1/1 (100%) | 5/6 (83%) | 5/6 (83%) | – | – | 0 / 0 | 0 |
| prototype | 3 (1) | 11/11 (100%) | 11/11 (100%) | – | – | 11/11 (100%) | 11/11 (100%) | – | – | 0 / 0 | 0 |
| prototype-chain | 3 (2) | 11/11 (100%) | 11/11 (100%) | – | – | 11/11 (100%) | 11/11 (100%) | – | – | 0 / 0 | 0 |
| returned-function | 4 (2) | 23/23 (100%) | 23/23 (100%) | – | – | 23/23 (100%) | 23/23 (100%) | – | – | 0 / 0 | 0 |
| same-name-member | 2 (1) | 4/4 (100%) | 4/4 (100%) | – | – | 4/6 (67%) | 4/6 (67%) | – | – | 0 / 0 | 0 |
| sequence-callee | 2 (1) | 8/8 (100%) | 8/8 (100%) | – | – | 8/8 (100%) | 8/8 (100%) | – | – | 0 / 0 | 0 |
| this-forms | 2 (1) | 11/11 (100%) | 11/11 (100%) | – | – | 11/11 (100%) | 11/11 (100%) | – | – | 0 / 0 | 0 |
| typed-join | 10 (5) | 32/32 (100%) | 32/32 (100%) | 1/2 (50%) | 1/2 (50%) | 33/37 (89%) | 33/37 (89%) | – | – | 0 / 0 | 0 |
| unary-iife | 2 (1) | 13/13 (100%) | 13/13 (100%) | 1/1 (100%) | 1/1 (100%) | 13/13 (100%) | 13/13 (100%) | – | – | 0 / 0 | 0 |

**False-complete count** (a labelled hidden path where polint gives a resolved answer with no unresolved marker): base 0, refined 0, over 5 labelled hidden paths.

**False-complete call lines** (a labelled call line where polint gives resolved targets that miss a labelled callee, with no unresolved marker on the line): base 0, refined 0, over 297 labelled call lines.

| case | plan | line | resolved to | missed |
|---|---|---|---|---|
| none | | | | |

### Per case

| case | held out | calls base | calls refined | callbacks base | callbacks refined | precision base | precision refined | hidden paths base | hidden paths refined | unresolved call share base | labels = observed |
|---|---|---|---|---|---|---|---|---|---|---|---|
| accessor/class-accessor-pair | yes | 2/4 (50%) | 2/4 (50%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 67% | yes |
| accessor/getter-returns-function | no | 1/2 (50%) | 1/2 (50%) | – | – | 1/1 (100%) | 1/1 (100%) | – | – | 100% | yes |
| argument-flow/parameter-and-return | no | 8/8 (100%) | 8/8 (100%) | – | – | 8/8 (100%) | 8/8 (100%) | – | – | 0% | yes |
| argument-flow/rest-and-spread | yes | 6/6 (100%) | 6/6 (100%) | 1/1 (100%) | 1/1 (100%) | 7/7 (100%) | 7/7 (100%) | – | – | 33% | yes |
| await/awaited-function | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 33% | yes |
| await/promise-resolve-value | yes | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 25% | yes |
| block-function/catch-and-finally | yes | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 17% | yes |
| block-function/try-and-if | no | 5/5 (100%) | 5/5 (100%) | – | – | 5/5 (100%) | 5/5 (100%) | – | – | 25% | yes |
| call-apply-bind/bind | yes | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 100% | yes |
| call-apply-bind/call-and-apply | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 100% | yes |
| callback/builtin-invoked | no | 1/1 (100%) | 1/1 (100%) | 3/3 (100%) | 3/3 (100%) | 3/3 (100%) | 3/3 (100%) | – | – | 75% | yes |
| callback/higher-order-parameter | no | 6/6 (100%) | 6/6 (100%) | – | – | 6/6 (100%) | 6/6 (100%) | – | – | 0% | yes |
| callback/nested-higher-order | yes | 2/2 (100%) | 2/2 (100%) | 1/1 (100%) | 1/1 (100%) | 3/3 (100%) | 3/3 (100%) | – | – | 25% | yes |
| cjs-exports-property/exports-of-factory-result | yes | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 50% | yes |
| cjs-exports-property/function-expressions | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 50% | yes |
| cjs-exports-property/renamed-destructuring | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 33% | yes |
| cjs-interop/compiled-default | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 50% | yes |
| cjs-interop/define-property-export | yes | 1/2 (50%) | 1/2 (50%) | – | – | 1/2 (50%) | 1/2 (50%) | – | – | 100% | yes |
| cjs-module-exports/function-with-property | yes | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 25% | yes |
| cjs-module-exports/object-literal | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 40% | yes |
| class-features/class-expression | yes | 3/4 (75%) | 3/4 (75%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| class-features/fields-statics-super | no | 9/9 (100%) | 9/9 (100%) | – | – | 9/11 (82%) | 9/11 (82%) | – | – | 0% | yes |
| class-method/instance-and-static | no | 5/5 (100%) | 5/5 (100%) | – | – | 5/5 (100%) | 5/5 (100%) | – | – | 17% | yes |
| class-method/subclass-override | yes | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 60% | yes |
| closure/captured-callback | no | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 0% | yes |
| closure/captured-parameter | yes | 2/4 (50%) | 2/4 (50%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 25% | yes |
| closure/factory-returned-function | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| computed-key-foldable/const-key-literal | yes | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 50% | yes |
| computed-key-foldable/constant-concatenation | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 0% | yes |
| computed-key-unknown/escaped-object | yes | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | reported-unknown | reported-unknown | 33% | yes |
| computed-key-unknown/known-literal-table | no | 2/4 (50%) | 2/4 (50%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 60% | yes |
| constant-key-field/private-field | yes | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 0% | yes |
| constant-key-field/string-and-numeric-keys | no | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 0% | yes |
| destructuring/array-and-nested | yes | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 67% | yes |
| destructuring/object-defaults-and-rest | no | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 75% | yes |
| dynamic-import/computed-specifier | no | – | – | – | – | – | – | reported-unknown | reported-unknown | 100% | yes |
| dynamic-import/literal-specifier | no | 1/2 (50%) | 1/2 (50%) | – | – | 1/1 (100%) | 1/1 (100%) | – | – | 50% | yes |
| dynamic-property/computed-key-known-set | no | 0/2 (0%) | 0/2 (0%) | – | – | – | – | – | – | 100% | yes |
| dynamic-property/key-from-input | yes | – | – | – | – | – | – | reported-unknown | reported-unknown | 100% | yes |
| esm-namespace/namespace-object | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 40% | yes |
| esm-namespace/star-and-live-binding | yes | 4/4 (100%) | 4/4 (100%) | – | – | 4/6 (67%) | 4/6 (67%) | – | – | 0% | yes |
| esm-reexport/named-barrel | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 50% | yes |
| esm-reexport/star-and-alias | yes | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 0% | yes |
| eval/direct-eval | no | – | – | – | – | – | – | reported-unknown | reported-unknown | 100% | yes |
| eval/function-constructor | yes | – | – | – | – | – | – | reported-unknown | reported-unknown | 100% | yes |
| exported-initializer/cjs-exports | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| exported-initializer/esm-declarators | yes | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| function-object/factory-made-constructor | yes | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 40% | yes |
| function-object/own-properties | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 0% | yes |
| iteration/for-of-array | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 67% | yes |
| iteration/generator | yes | 5/5 (100%) | 5/5 (100%) | – | – | 5/6 (83%) | 5/6 (83%) | – | – | 25% | yes |
| literal-spread/object-spread | no | 3/4 (75%) | 3/4 (75%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 100% | yes |
| literal-spread/override-after-spread | yes | 2/3 (67%) | 2/3 (67%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 100% | yes |
| logical-join/conditional-callee | yes | 10/10 (100%) | 10/10 (100%) | – | – | 10/10 (100%) | 10/10 (100%) | – | – | 0% | yes |
| logical-join/default-operand | no | 11/11 (100%) | 11/11 (100%) | – | – | 11/11 (100%) | 11/11 (100%) | – | – | 0% | yes |
| module-initializer/arrow-in-initializer | no | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 20% | yes |
| module-initializer/chained-call | yes | 5/5 (100%) | 5/5 (100%) | – | – | 5/5 (100%) | 5/5 (100%) | – | – | 50% | yes |
| module-initializer/class-expression | yes | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 60% | yes |
| module-initializer/exported-call | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 0% | yes |
| module-initializer/local-initializer | no | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 0% | yes |
| module-initializer/object-literal-methods | no | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 0% | yes |
| new-and-prototype/constructor-function | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| new-and-prototype/return-override | yes | 5/5 (100%) | 5/5 (100%) | – | – | 5/5 (100%) | 5/5 (100%) | – | – | 20% | yes |
| object-of-functions/literal-methods | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 0% | yes |
| object-of-functions/registry-built-later | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| object-of-functions/registry-passed-around | yes | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| optional-chaining/optional-call | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/2 (100%) | 2/2 (100%) | – | – | 0% | yes |
| optional-chaining/optional-member-chain | yes | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 33% | yes |
| package-vendor/cjs-require | yes | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 50% | yes |
| package-vendor/esm-import | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 25% | yes |
| promise/async-await | yes | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| promise/then-chain | no | 1/1 (100%) | 1/1 (100%) | 1/1 (100%) | 1/1 (100%) | 2/3 (67%) | 2/3 (67%) | – | – | 60% | no |
| prototype-chain/escaped-subclass-instance | yes | 5/5 (100%) | 5/5 (100%) | – | – | 5/5 (100%) | 5/5 (100%) | – | – | 60% | yes |
| prototype-chain/object-create | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| prototype-chain/set-prototype-of | yes | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 25% | yes |
| prototype/constructor-function | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| prototype/escaped-instance | no | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 50% | yes |
| prototype/inherited-method | yes | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 60% | yes |
| returned-function/branch-returns | no | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 0% | yes |
| returned-function/conditional-default | yes | 9/9 (100%) | 9/9 (100%) | – | – | 9/9 (100%) | 9/9 (100%) | – | – | 0% | yes |
| returned-function/stored-in-property | yes | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 0% | yes |
| returned-function/two-levels | no | 6/6 (100%) | 6/6 (100%) | – | – | 6/6 (100%) | 6/6 (100%) | – | – | 0% | yes |
| same-name-member/cjs-static-method | yes | 2/2 (100%) | 2/2 (100%) | – | – | 2/3 (67%) | 2/3 (67%) | – | – | 33% | yes |
| same-name-member/wrapper-of-static-method | no | 2/2 (100%) | 2/2 (100%) | – | – | 2/3 (67%) | 2/3 (67%) | – | – | 0% | yes |
| sequence-callee/compiled-interop | no | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 33% | yes |
| sequence-callee/local-sequence | yes | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 0% | yes |
| this-forms/literal-method-this | no | 5/5 (100%) | 5/5 (100%) | – | – | 5/5 (100%) | 5/5 (100%) | – | – | 80% | yes |
| this-forms/union-receiver | yes | 6/6 (100%) | 6/6 (100%) | – | – | 6/6 (100%) | 6/6 (100%) | – | – | 0% | yes |
| typed-join/closure-field-assigned | yes | 4/4 (100%) | 4/4 (100%) | – | – | 4/5 (80%) | 4/5 (80%) | – | – | 75% | yes |
| typed-join/closure-field-typed | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/4 (75%) | 3/4 (75%) | – | – | 0% | yes |
| typed-join/external-base-emit | no | 2/2 (100%) | 2/2 (100%) | 1/1 (100%) | 1/1 (100%) | 3/3 (100%) | 3/3 (100%) | – | – | 67% | yes |
| typed-join/external-base-super-emit | yes | 2/2 (100%) | 2/2 (100%) | 0/1 (0%) | 0/1 (0%) | 2/2 (100%) | 2/2 (100%) | – | – | 67% | yes |
| typed-join/factory-constructor-function | yes | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| typed-join/factory-mixin-class | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/3 (100%) | 3/3 (100%) | – | – | 0% | yes |
| typed-join/reassigned-call-initializer | no | 3/3 (100%) | 3/3 (100%) | – | – | 3/4 (75%) | 3/4 (75%) | – | – | 25% | yes |
| typed-join/reassigned-function-initializer | yes | 2/2 (100%) | 2/2 (100%) | – | – | 2/3 (67%) | 2/3 (67%) | – | – | 0% | yes |
| typed-join/union-parameter-jsdoc | yes | 6/6 (100%) | 6/6 (100%) | – | – | 6/6 (100%) | 6/6 (100%) | – | – | 20% | yes |
| typed-join/union-parameter-typed | no | 4/4 (100%) | 4/4 (100%) | – | – | 4/4 (100%) | 4/4 (100%) | – | – | 0% | yes |
| unary-iife/umd-prologue | no | 5/5 (100%) | 5/5 (100%) | 1/1 (100%) | 1/1 (100%) | 5/5 (100%) | 5/5 (100%) | – | – | 58% | yes |
| unary-iife/unary-operators | yes | 8/8 (100%) | 8/8 (100%) | – | – | 8/8 (100%) | 8/8 (100%) | – | – | 0% | no |

## Packages (observed calls: recall only)

Recall is over observed edges at call expressions (`call` and `callback`); `implicit` calls (getters,
coercions, iterator and thenable protocol) have no call expression and are listed by kind only.

Coverage is the share of the analysed files' functions (syntax census, classes and static blocks left out)
that the recording entered.

| package | format | files analysed / selected | source KB | coverage | observed edges | recall base | recall refined | stops at import binding (base) | not mappable to a polint site or function | unresolved call share base | call sites without target base / refined | unresolved reference share |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| axios 1.21.1 | ESM source, Node and browser adapters | 72 / 72 | 282 | 300/534 (56%) | 675 | 604/675 (89%) | 604/675 (89%) | 0 | 0 | 62% | 47% / 47% | 9% |
| zod 4.6.7 | TypeScript ESM source | 125 / 125 | 1354 | 566/2469 (23%) (flagged) | 1308 | 1181/1308 (90%) | 1181/1308 (90%) | 0 | 0 | 67% | 34% / 34% | 5% |
| commander 14.0.3 | CommonJS only | 7 / 7 | 127 | 218/296 (74%) | 318 | 183/318 (58%) | 183/318 (58%) | 0 | 0 | 89% | 66% / 66% | 10% |
| async 3.2.6 | CommonJS transpiled from ESM | 129 / 129 | 321 | 308/504 (61%) | 637 | 631/637 (99%) | 631/637 (99%) | 0 | 0 | 62% | 40% / 40% | 27% |
| chalk 6.0.1 | ESM only, with subpath imports | 5 / 5 | 22 | 41/53 (77%) | 72 | 56/72 (78%) | 56/72 (78%) | 0 | 0 | 44% | 55% / 55% | 11% |
| dayjs 1.11.23 | minified UMD | 1 / 1 | 7 | 55/57 (96%) | 112 | 88/112 (79%) | 88/112 (79%) | 0 | 0 | 72% | 48% / 48% | 7% |
| underscore 1.13.8 | bundled UMD (Rollup) | 1 / 1 | 69 | 159/186 (85%) | 260 | 250/260 (96%) | 250/260 (96%) | 0 | 0 | 38% | 29% / 29% | 6% |

### Claimed targets the run never called

A claimed target is resolved and not heuristic. It counts here when the run executed the site, the target is
in an instrumented file, and the run never called it from that site. On a **single-target** answer that is a
false precise edge: every execution of the site went elsewhere. On a **multi-target** answer it is an upper
bound only, since the run may not have taken the paths to the other targets. A site matched to a polint site
by its end byte can also inflate both counts.

| package | base: single / multi | refined: single / multi | typed: single / multi |
|---|---|---|---|
| axios | 1 / 30 | 1 / 30 | 1 / 30 |
| zod | 0 / 40 | 0 / 40 | 0 / 40 |
| commander | 0 / 0 | 0 / 0 | 1 / 0 |
| async | 0 / 764 | 0 / 764 | 0 / 764 |
| chalk | 0 / 0 | 0 / 0 | 0 / 0 |
| dayjs | 0 / 0 | 0 / 0 | 0 / 0 |
| underscore | 0 / 249 | 0 / 249 | 0 / 249 |
| **total** | **1 / 1083** | **1 / 1083** | **2 / 1083** |

| package | plan | site | site text | single-target answer | the run called |
|---|---|---|---|---|---|
| axios | base | lib/core/mergeConfig.js:220 | `merge(a, b, prop, config1, config2)` | lib/core/mergeConfig.js:79 mergeDeepProperties | lib/core/mergeConfig.js:130 mergeDirectKeys |
| axios | refined | lib/core/mergeConfig.js:220 | `merge(a, b, prop, config1, config2)` | lib/core/mergeConfig.js:79 mergeDeepProperties | lib/core/mergeConfig.js:130 mergeDirectKeys |
| axios | typed | lib/core/mergeConfig.js:220 | `merge(a, b, prop, config1, config2)` | lib/core/mergeConfig.js:79 mergeDeepProperties | lib/core/mergeConfig.js:130 mergeDirectKeys |
| commander | typed | lib/command.js:601 | `target.parseArg(value, previous)` | lib/argument.js:100 <polint:anonymous:2195:2491> | lib/option.js:183 <polint:anonymous:4994:5290> |

### What types add (refined vs typed, TypeScript 5.9.3)

**typed** is the refined plan with `[languages.ts] type_sidecar = true` over a copy of each tree with a
tsconfig (`allowJs`, `nodenext`). The type tier only feeds the refined layer, so base is unchanged by it.
Dump time and peak RSS are one fact dump each, sidecar included.

| package | recall refined | recall typed | false-complete refined / typed | type callsite rows | dump s refined / typed | dump peak MB refined / typed |
|---|---|---|---|---|---|---|
| axios | 604/675 (89%) | 606/675 (90%) | 0 / 0 | 2098 | 2.7 / 3.7 | 378.3 / 399.5 |
| zod | 1181/1308 (90%) | 1189/1308 (91%) | 0 / 0 | 6125 | 15.7 / 18.6 | 1381.5 / 1397.8 |
| commander | 183/318 (58%) | 306/318 (96%) | 0 / 0 | 814 | 1.8 / 2.5 | 180.0 / 233.4 |
| async | 631/637 (99%) | 631/637 (99%) | 0 / 0 | 1605 | 11.4 / 12.4 | 1798.7 / 1819.6 |
| chalk | 56/72 (78%) | 56/72 (78%) | 0 / 0 | 144 | 0.2 / 0.7 | 65.8 / 217.6 |
| dayjs | 88/112 (79%) | 88/112 (79%) | 0 / 0 | 181 | 0.3 / 0.8 | 68.0 / 223.7 |
| underscore | 250/260 (96%) | 250/260 (96%) | 0 / 0 | 455 | 0.8 / 1.4 | 168.9 / 225.2 |

How the type rows joined the syntax facts. A callsite row joins a call site by span and adds nothing when it does not. A deferred callsite joined but sits in a file dense with `any` and is not resolved, so it adds nothing either. A callee row joins a function by span, then by name for the kinds a call runs (an accessor joins by span only); a callee outside the tree counts as matched. An unmatched callee is a declaration in the tree that names no function polint extracted, and adds no edge.

| package | callsites matched / unmatched / deferred | callees matched / unmatched |
|---|---|---|
| axios | 2,098 / 0 / 7 | 931 / 0 |
| zod | 6,108 / 17 / 0 | 3,112 / 80 |
| commander | 814 / 0 / 0 | 395 / 0 |
| async | 1,605 / 0 / 145 | 467 / 0 |
| chalk | 144 / 0 / 0 | 47 / 0 |
| dayjs | 181 / 0 / 0 | 60 / 0 |
| underscore | 455 / 0 / 0 | 273 / 0 |

What the type step changed in the refined plan's answer: `type_mismatch` is an exact edge to a function whose declared kind or arity the checker says the call cannot run, moved to the conservative tier and kept; `type_seed` is a method of the receiver's declared class listed as a conservative candidate at an open site.

| package | type_mismatch | type_seed |
|---|---|---|
| axios | 0 | 0 |
| zod | 0 | 0 |
| commander | 0 | 0 |
| async | 0 | 0 |
| chalk | 0 | 0 |
| dayjs | 0 | 0 |
| underscore | 0 | 0 |

Typed against refined, every package and the held-out set, in exact_complete, exact, exact_conservative: no package loses an observed edge.

Constructs, typed: false-complete hidden paths 0, false-complete call lines 0.

### Package recall by observed call kind and syntax

| package | kind / syntax | observed | base | refined |
|---|---|---|---|---|
| axios | kind: call | 632 | 575/632 (91%) | 575/632 (91%) |
| axios | kind: callback | 43 | 29/43 (67%) | 29/43 (67%) |
| axios | kind: implicit | 6 | 0/6 (0%) | 0/6 (0%) |
| axios | syntax: computed-member | 2 | 1/2 (50%) | 1/2 (50%) |
| axios | syntax: expression | 8 | 8/8 (100%) | 8/8 (100%) |
| axios | syntax: identifier | 317 | 306/317 (97%) | 306/317 (97%) |
| axios | syntax: member | 326 | 268/326 (82%) | 268/326 (82%) |
| axios | syntax: new | 21 | 20/21 (95%) | 20/21 (95%) |
| axios | syntax: super | 1 | 1/1 (100%) | 1/1 (100%) |
| zod | kind: call | 1265 | 1150/1265 (91%) | 1150/1265 (91%) |
| zod | kind: callback | 43 | 31/43 (72%) | 31/43 (72%) |
| zod | kind: implicit | 18 | 0/18 (0%) | 0/18 (0%) |
| zod | syntax: computed-member | 5 | 0/5 (0%) | 0/5 (0%) |
| zod | syntax: expression | 1 | 1/1 (100%) | 1/1 (100%) |
| zod | syntax: identifier | 408 | 378/408 (93%) | 378/408 (93%) |
| zod | syntax: member | 829 | 738/829 (89%) | 738/829 (89%) |
| zod | syntax: new | 65 | 64/65 (98%) | 64/65 (98%) |
| commander | kind: call | 264 | 134/264 (51%) | 134/264 (51%) |
| commander | kind: callback | 54 | 49/54 (91%) | 49/54 (91%) |
| commander | syntax: identifier | 35 | 34/35 (97%) | 34/35 (97%) |
| commander | syntax: member | 274 | 140/274 (51%) | 140/274 (51%) |
| commander | syntax: new | 8 | 8/8 (100%) | 8/8 (100%) |
| commander | syntax: super | 1 | 1/1 (100%) | 1/1 (100%) |
| async | kind: call | 622 | 616/622 (99%) | 616/622 (99%) |
| async | kind: callback | 15 | 15/15 (100%) | 15/15 (100%) |
| async | kind: implicit | 4 | 0/4 (0%) | 0/4 (0%) |
| async | syntax: expression | 157 | 157/157 (100%) | 157/157 (100%) |
| async | syntax: identifier | 403 | 398/403 (99%) | 398/403 (99%) |
| async | syntax: member | 73 | 72/73 (99%) | 72/73 (99%) |
| async | syntax: new | 4 | 4/4 (100%) | 4/4 (100%) |
| chalk | kind: call | 71 | 55/71 (77%) | 55/71 (77%) |
| chalk | kind: callback | 1 | 1/1 (100%) | 1/1 (100%) |
| chalk | kind: implicit | 1 | 0/1 (0%) | 0/1 (0%) |
| chalk | syntax: computed-member | 10 | 0/10 (0%) | 0/10 (0%) |
| chalk | syntax: identifier | 43 | 43/43 (100%) | 43/43 (100%) |
| chalk | syntax: member | 19 | 13/19 (68%) | 13/19 (68%) |
| dayjs | kind: call | 110 | 86/110 (78%) | 86/110 (78%) |
| dayjs | kind: callback | 2 | 2/2 (100%) | 2/2 (100%) |
| dayjs | kind: implicit | 12 | 0/12 (0%) | 0/12 (0%) |
| dayjs | syntax: computed-member | 1 | 0/1 (0%) | 0/1 (0%) |
| dayjs | syntax: identifier | 35 | 35/35 (100%) | 35/35 (100%) |
| dayjs | syntax: member | 75 | 52/75 (69%) | 52/75 (69%) |
| dayjs | syntax: new | 1 | 1/1 (100%) | 1/1 (100%) |
| underscore | kind: call | 255 | 247/255 (97%) | 247/255 (97%) |
| underscore | kind: callback | 5 | 3/5 (60%) | 3/5 (60%) |
| underscore | kind: implicit | 2 | 0/2 (0%) | 0/2 (0%) |
| underscore | syntax: identifier | 234 | 232/234 (99%) | 232/234 (99%) |
| underscore | syntax: member | 25 | 17/25 (68%) | 17/25 (68%) |
| underscore | syntax: new | 1 | 1/1 (100%) | 1/1 (100%) |

### Why observed edges were missed (base / refined)

| package | found | no target, marked unresolved | other targets, marked unresolved |
|---|---|---|---|
| axios | 604 / 604 / 606 | 49 / 49 / 47 | 22 / 22 / 22 |
| zod | 1181 / 1181 / 1189 | 120 / 120 / 112 | 7 / 7 / 7 |
| commander | 183 / 183 / 306 | 134 / 134 / 10 | 1 / 1 / 2 |
| async | 631 / 631 / 631 | 4 / 4 / 4 | 2 / 2 / 2 |
| chalk | 56 / 56 / 56 | 16 / 16 / 16 | 0 / 0 / 0 |
| dayjs | 88 / 88 / 88 | 24 / 24 / 24 | 0 / 0 / 0 |
| underscore | 250 / 250 / 250 | 8 / 8 / 8 | 2 / 2 / 2 |

### Missed edges ranked by gap

Every observed call or callback edge polint did not find is attributed to one gap: the first in the order
unary IIFEs, try / catch / if blocks, exported initializers, constructors, callbacks to built-ins, computed
keys, prototype and class methods, then the remainders. Extraction gaps come first because no resolution
change finds an edge whose call site or callee polint does not extract; a site or callee polint misses is
charged to the outermost function around it that polint does not extract. **attributed** counts each edge
once; **applying** counts every gap whose condition holds. The recall gain is an upper bound: it assumes
every attributed edge is found once the gap is fixed. The syntax census behind this is described in the README.

Base: 389 missed of 3382 observed edges. Refined: 389 missed of 3382.

| rank | gap | attributed base | applying base | recall gain base (upper bound) | attributed refined | recall gain refined (upper bound) | base by package |
|---|---|---|---|---|---|---|---|
| 1 | _other member calls (object-literal functions, namespaces)_ | 163 | 0 | 4.8% | 163 | 4.8% | zod 81, axios 38, dayjs 23, commander 8, chalk 6, underscore 6, async 1 |
| 2 | prototype and class methods, incl. inheritance | 139 | 140 | 4.1% | 139 | 4.1% | commander 121, zod 10, axios 7, underscore 1 |
| 3 | _other resolution misses_ | 36 | 0 | 1.1% | 36 | 1.1% | zod 18, axios 11, async 5, commander 1, underscore 1 |
| 4 | callbacks invoked by built-ins | 32 | 33 | 0.9% | 32 | 0.9% | axios 13, zod 12, commander 5, underscore 2 |
| 5 | computed-key member calls | 17 | 17 | 0.5% | 17 | 0.5% | chalk 10, zod 5, axios 1, dayjs 1 |
| 6 | class constructors and `new` | 2 | 2 | 0.1% | 2 | 0.1% | axios 1, zod 1 |
| 7 | functions declared in try / catch / if blocks | 0 | 0 | 0.0% | 0 | 0.0% | – |
| 8 | calls in exported initializers | 0 | 0 | 0.0% | 0 | 0.0% | – |
| 9 | _identifier call stops at import binding_ | 0 | 0 | 0.0% | 0 | 0.0% | – |
| 10 | _file not analysed (discovery excludes)_ | 0 | 0 | 0.0% | 0 | 0.0% | – |
| 11 | _other call sites or functions not extracted_ | 0 | 0 | 0.0% | 0 | 0.0% | – |
| 12 | functions under a unary operator (unary IIFEs) | 0 | 0 | 0.0% | 0 | 0.0% | – |

Rows in italics are remainders, not one of the named extraction or resolution gaps. Of the base
prototype and class method misses, 2 go through `super` or a `this`
call into another class; that split reads the receiver syntactically and is approximate.
'Other resolution misses' by call syntax and callee kind (base): identifier call to arrow 24, identifier call to declaration 7, identifier call to expression 4, identifier call to method 1. An `expression` call is one whose callee is neither a name nor a member, such as the `(0, f)(x)`
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
| axios | base | found | lib/adapters/adapters.js:53 `utils.isFunction(adapter)` | member / call | lib/utils.js:194 <anonymous> | targets lib/utils.js:194 <polint:anonymous:5832:5864>, marked unresolved |
| axios | base | no target, marked unresolved | lib/adapters/adapters.js:88 `adapter.get(config)` | member / call | lib/adapters/fetch.js:718 getFetch | no target, marked unresolved |
| axios | base | other targets, marked unresolved | lib/adapters/fetch.js:480 `headers.set('User-Agent', 'axios/' + VERSION, false)` | member / call | lib/core/AxiosHeaders.js:478 get | targets lib/core/AxiosHeaders.js:216 AxiosHeaders.set, marked unresolved |
| axios | refined | found | lib/adapters/adapters.js:53 `utils.isFunction(adapter)` | member / call | lib/utils.js:194 <anonymous> | targets lib/utils.js:194 <polint:anonymous:5832:5864>, marked unresolved |
| axios | refined | no target, marked unresolved | lib/adapters/adapters.js:88 `adapter.get(config)` | member / call | lib/adapters/fetch.js:718 getFetch | no target, marked unresolved |
| axios | refined | other targets, marked unresolved | lib/adapters/fetch.js:480 `headers.set('User-Agent', 'axios/' + VERSION, false)` | member / call | lib/core/AxiosHeaders.js:478 get | targets lib/core/AxiosHeaders.js:216 AxiosHeaders.set, marked unresolved |
| axios | typed | found | lib/adapters/adapters.js:53 `utils.isFunction(adapter)` | member / call | lib/utils.js:194 <anonymous> | targets lib/utils.js:194 <polint:anonymous:5832:5864>, marked unresolved |
| axios | typed | no target, marked unresolved | lib/adapters/adapters.js:88 `adapter.get(config)` | member / call | lib/adapters/fetch.js:718 getFetch | no target, marked unresolved |
| axios | typed | other targets, marked unresolved | lib/adapters/fetch.js:480 `headers.set('User-Agent', 'axios/' + VERSION, false)` | member / call | lib/core/AxiosHeaders.js:478 get | targets lib/core/AxiosHeaders.js:216 AxiosHeaders.set, marked unresolved |
| zod | base | found | src/v3/ZodError.ts:264 `mapper(issue)` | identifier / call | src/v3/ZodError.ts:220 <anonymous> | targets src/v3/ZodError.ts:220 <polint:anonymous:5731:5797>, marked unresolved |
| zod | base | no target, marked unresolved | src/v4/classic/from-json-schema.ts:505 `z.string()` | member / call | src/v4/classic/schemas.ts:619 string | no target, marked unresolved |
| zod | base | other targets, marked unresolved | src/v4/core/core.ts:124 `fn()` | identifier / call | src/v4/core/memoizer.ts:228 <anonymous> | targets src/v4/core/schemas.ts:229 <polint:anonymous:7915:7971>, marked unresolved |
| zod | refined | found | src/v3/ZodError.ts:264 `mapper(issue)` | identifier / call | src/v3/ZodError.ts:220 <anonymous> | targets src/v3/ZodError.ts:220 <polint:anonymous:5731:5797>, marked unresolved |
| zod | refined | no target, marked unresolved | src/v4/classic/from-json-schema.ts:505 `z.string()` | member / call | src/v4/classic/schemas.ts:619 string | no target, marked unresolved |
| zod | refined | other targets, marked unresolved | src/v4/core/core.ts:124 `fn()` | identifier / call | src/v4/core/memoizer.ts:228 <anonymous> | targets src/v4/core/schemas.ts:229 <polint:anonymous:7915:7971>, marked unresolved |
| zod | typed | found | src/v3/ZodError.ts:264 `mapper(issue)` | identifier / call | src/v3/ZodError.ts:220 <anonymous> | targets src/v3/ZodError.ts:220 <polint:anonymous:5731:5797>, marked unresolved |
| zod | typed | no target, marked unresolved | src/v4/classic/from-json-schema.ts:505 `z.string()` | member / call | src/v4/classic/schemas.ts:619 string | no target, marked unresolved |
| zod | typed | other targets, marked unresolved | src/v4/core/core.ts:124 `fn()` | identifier / call | src/v4/core/memoizer.ts:228 <anonymous> | targets src/v4/core/schemas.ts:229 <polint:anonymous:7915:7971>, marked unresolved |
| commander | base | found | index.js:7 `new Command()` | new / call | lib/command.js:20 constructor | targets lib/command.js:13 Command, no unresolved marker |
| commander | base | no target, marked unresolved | lib/argument.js:144 `arg.name()` | member / call | lib/argument.js:48 name | no target, marked unresolved |
| commander | base | other targets, marked unresolved | lib/command.js:1488 `fn()` | identifier / call | lib/command.js:1515 <anonymous> | targets lib/command.js:1363 <polint:anonymous:45378:45588>; lib/command.js:1603 <polint:anonymous:52130:52183>; lib/command.js:1607 <polint:anonymous:52274:52362>, marked unresolved |
| commander | refined | found | index.js:7 `new Command()` | new / call | lib/command.js:20 constructor | targets lib/command.js:13 Command, no unresolved marker |
| commander | refined | no target, marked unresolved | lib/argument.js:144 `arg.name()` | member / call | lib/argument.js:48 name | no target, marked unresolved |
| commander | refined | other targets, marked unresolved | lib/command.js:1488 `fn()` | identifier / call | lib/command.js:1515 <anonymous> | targets lib/command.js:1363 <polint:anonymous:45378:45588>; lib/command.js:1603 <polint:anonymous:52130:52183>; lib/command.js:1607 <polint:anonymous:52274:52362>, marked unresolved |
| commander | typed | found | index.js:7 `new Command()` | new / call | lib/command.js:20 constructor | targets lib/command.js:13 Command, no unresolved marker |
| commander | typed | no target, marked unresolved | lib/command.js:646 `cmd.name()` | member / call | lib/command.js:2331 name | no target, marked unresolved |
| commander | typed | other targets, marked unresolved | lib/command.js:601 `target.parseArg(value, previous)` | member / call | lib/option.js:183 parseArg | targets lib/argument.js:100 <polint:anonymous:2195:2491>, marked unresolved |
| async | base | found | applyEach.js:9 `_interopRequireDefault(_applyEach)` | identifier / call | applyEach.js:15 _interopRequireDefault | targets applyEach.js:15 _interopRequireDefault, no unresolved marker |
| async | base | no target, marked unresolved | asyncify.js:111 `callback(error, value)` | identifier / call | internal/map.js:22 <anonymous> | no target, marked unresolved |
| async | base | other targets, marked unresolved | auto.js:293 `taskFn(results, taskCallback)` | identifier / call | autoInject.js:173 newTask | targets all.js:115 every; all.js:116 <polint:anonymous:3398:3411>; all.js:116 <polint:anonymous:3413:3424>; allSeries.js:41 everySeries; allSeries.js:42 <polint:anonymous:1472:1485>; allSeries.js:42 <polint:anonymous:1487:1498>; any.js:118 some; any.js:119 <polint:anonymous:3659:3669>; anySeries.js:42 someSeries; anySeries.js:43 <polint:anonymous:1524:1534>; asyncify.js:78 <polint:anonymous:2637:2828>; auto.js:265 <polint:anonymous:9708:10464>; concat.js:111 concat; concatLimit.js:41 concatLimit; concatLimit.js:43 <polint:anonymous:1575:1733>; concatLimit.js:44 <polint:anonymous:1617:1725>; concatLimit.js:48 <polint:anonymous:1735:1996>; concatSeries.js:37 concatSeries; detect.js:92 detect; detect.js:93 <polint:anonymous:3080:3092>; detect.js:93 <polint:anonymous:3094:3113>; doDuring.js:44 doWhilst; doDuring.js:50 next; doUntil.js:41 <polint:anonymous:1547:1655>; doWhilst.js:44 doWhilst; doWhilst.js:50 next; during.js:55 whilst; during.js:61 next; during.js:68 check; each.js:124 eachLimit; eachLimit.js:46 eachLimit; eachOf.js:48 iteratorCallback; eachOf.js:179 eachOf; eachOfLimit.js:42 eachOfLimit; eachOfSeries.js:35 eachOfSeries; eachSeries.js:40 eachSeries; ensureAsync.js:53 <polint:anonymous:1897:2274>; every.js:115 every; every.js:116 <polint:anonymous:3398:3411>; every.js:116 <polint:anonymous:3413:3424>; everySeries.js:41 everySeries; everySeries.js:42 <polint:anonymous:1472:1485>; everySeries.js:42 <polint:anonymous:1487:1498>; filter.js:89 filter; filterSeries.js:39 filterSeries; find.js:92 detect; find.js:93 <polint:anonymous:3080:3092>; find.js:93 <polint:anonymous:3094:3113>; flatMap.js:111 concat; flatMapLimit.js:41 concatLimit; flatMapLimit.js:43 <polint:anonymous:1575:1733>; flatMapLimit.js:44 <polint:anonymous:1617:1725>; flatMapLimit.js:48 <polint:anonymous:1735:1996>; flatMapSeries.js:37 concatSeries; foldl.js:142 reduce; foldl.js:145 <polint:anonymous:4633:4758>; foldl.js:146 <polint:anonymous:4680:4750>; foldl.js:150 <polint:anonymous:4760:4786>; forEach.js:124 eachLimit; forEachLimit.js:46 eachLimit; forEachOf.js:48 iteratorCallback; forEachOf.js:179 eachOf; forEachOfLimit.js:42 eachOfLimit; forEachOfSeries.js:35 eachOfSeries; forEachSeries.js:40 eachSeries; forever.js:56 forever; forever.js:60 next; groupByLimit.js:41 groupByLimit; groupByLimit.js:43 <polint:anonymous:1587:1749>; groupByLimit.js:44 <polint:anonymous:1629:1741>; groupByLimit.js:48 <polint:anonymous:1751:2347>; inject.js:142 reduce; inject.js:145 <polint:anonymous:4633:4758>; inject.js:146 <polint:anonymous:4680:4750>; inject.js:150 <polint:anonymous:4760:4786>; internal/applyEach.js:7 default; internal/applyEach.js:8 <polint:anonymous:132:451>; internal/applyEach.js:9 <polint:anonymous:214:424>; internal/applyEach.js:11 <polint:anonymous:296:402>; internal/asyncEachOfLimit.js:46 iterateeCallback; internal/awaitify.js:9 awaitify; internal/awaitify.js:12 awaitable; internal/createTester.js:34 <polint:anonymous:1062:1183>; internal/createTester.js:18 _createTester; internal/createTester.js:19 <polint:anonymous:467:1191>; internal/createTester.js:23 <polint:anonymous:639:1060>; internal/createTester.js:24 <polint:anonymous:693:1048>; internal/eachOfLimit.js:32 <polint:anonymous:765:2599>; internal/filter.js:38 <polint:anonymous:1043:1218>; internal/filter.js:45 <polint:anonymous:1228:1366>; internal/filter.js:51 _filter; internal/filter.js:20 <polint:anonymous:532:670>; internal/filter.js:21 <polint:anonymous:576:662>; internal/filter.js:25 <polint:anonymous:672:895>; internal/filter.js:37 <polint:anonymous:999:1226>; internal/initialParams.js:8 <polint:anonymous:128:250>; internal/iterator.js:21 <polint:anonymous:521:606>; internal/iterator.js:28 <polint:anonymous:680:834>; internal/iterator.js:40 <polint:anonymous:976:1159>; internal/map.js:14 _asyncMap; internal/map.js:20 <polint:anonymous:507:675>; internal/map.js:22 <polint:anonymous:587:667>; internal/map.js:26 <polint:anonymous:677:723>; internal/once.js:8 wrapper; internal/onlyOnce.js:8 <polint:anonymous:145:323>; internal/parallel.js:21 <polint:anonymous:526:923>; internal/parallel.js:24 <polint:anonymous:639:888>; internal/parallel.js:25 <polint:anonymous:703:880>; internal/parallel.js:32 <polint:anonymous:890:919>; internal/promiseCallback.js:10 callback; internal/queue.js:92 <polint:anonymous:2672:2767>; internal/queue.js:107 <polint:anonymous:3018:3803>; internal/queue.js:141 <polint:anonymous:3990:4012>; internal/queue.js:212 <polint:anonymous:6162:7313>; internal/reject.js:18 reject; internal/reject.js:20 <polint:anonymous:550:647>; internal/reject.js:21 <polint:anonymous:591:639>; internal/withoutIndex.js:8 <polint:anonymous:161:214>; internal/wrapAsync.js:26 wrapAsync; map.js:138 map; mapSeries.js:40 mapSeries; mapValuesLimit.js:47 mapValuesLimit; mapValuesLimit.js:51 <polint:anonymous:1804:1986>; mapValuesLimit.js:52 <polint:anonymous:1854:1978>; mapValuesLimit.js:57 <polint:anonymous:1988:2016>; memoize.js:66 <polint:anonymous:2234:2955>; memoize.js:69 <polint:anonymous:2358:2392>; queue.js:9 <polint:anonymous:218:271>; race.js:57 race; reduce.js:142 reduce; reduce.js:145 <polint:anonymous:4633:4758>; reduce.js:146 <polint:anonymous:4680:4750>; reduce.js:150 <polint:anonymous:4760:4786>; reflect.js:59 <polint:anonymous:1714:2228>; reject.js:83 reject; rejectLimit.js:41 rejectLimit; rejectSeries.js:39 rejectSeries; retry.js:132 <polint:anonymous:4598:4944>; retryable.js:63 <polint:anonymous:2064:2497>; retryable.js:68 taskFn; select.js:89 filter; selectSeries.js:39 filterSeries; seq.js:70 <polint:anonymous:2358:2516>; seq.js:74 <polint:anonymous:2518:2555>; some.js:118 some; some.js:119 <polint:anonymous:3659:3669>; someSeries.js:42 someSeries; someSeries.js:43 <polint:anonymous:1524:1534>; sortBy.js:171 sortBy; sortBy.js:173 <polint:anonymous:5666:5832>; sortBy.js:174 <polint:anonymous:5704:5824>; sortBy.js:178 <polint:anonymous:5834:5966>; timeout.js:62 <polint:anonymous:1994:2717>; transform.js:168 <polint:anonymous:5373:5436>; transform.js:170 <polint:anonymous:5438:5471>; tryEach.js:59 tryEach; tryEach.js:62 <polint:anonymous:2074:2418>; tryEach.js:63 <polint:anonymous:2133:2410>; tryEach.js:74 <polint:anonymous:2420:2449>; until.js:59 <polint:anonymous:1978:2022>; until.js:59 <polint:anonymous:1990:2021>; waterfall.js:82 waterfall; waterfall.js:93 next; whilst.js:55 whilst; whilst.js:61 next; whilst.js:68 check, marked unresolved |
| async | refined | found | applyEach.js:9 `_interopRequireDefault(_applyEach)` | identifier / call | applyEach.js:15 _interopRequireDefault | targets applyEach.js:15 _interopRequireDefault, no unresolved marker |
| async | refined | no target, marked unresolved | asyncify.js:111 `callback(error, value)` | identifier / call | internal/map.js:22 <anonymous> | no target, marked unresolved |
| async | refined | other targets, marked unresolved | auto.js:293 `taskFn(results, taskCallback)` | identifier / call | autoInject.js:173 newTask | targets all.js:115 every; all.js:116 <polint:anonymous:3398:3411>; all.js:116 <polint:anonymous:3413:3424>; allSeries.js:41 everySeries; allSeries.js:42 <polint:anonymous:1472:1485>; allSeries.js:42 <polint:anonymous:1487:1498>; any.js:118 some; any.js:119 <polint:anonymous:3659:3669>; anySeries.js:42 someSeries; anySeries.js:43 <polint:anonymous:1524:1534>; asyncify.js:78 <polint:anonymous:2637:2828>; auto.js:265 <polint:anonymous:9708:10464>; concat.js:111 concat; concatLimit.js:41 concatLimit; concatLimit.js:43 <polint:anonymous:1575:1733>; concatLimit.js:44 <polint:anonymous:1617:1725>; concatLimit.js:48 <polint:anonymous:1735:1996>; concatSeries.js:37 concatSeries; detect.js:92 detect; detect.js:93 <polint:anonymous:3080:3092>; detect.js:93 <polint:anonymous:3094:3113>; doDuring.js:44 doWhilst; doDuring.js:50 next; doUntil.js:41 <polint:anonymous:1547:1655>; doWhilst.js:44 doWhilst; doWhilst.js:50 next; during.js:55 whilst; during.js:61 next; during.js:68 check; each.js:124 eachLimit; eachLimit.js:46 eachLimit; eachOf.js:48 iteratorCallback; eachOf.js:179 eachOf; eachOfLimit.js:42 eachOfLimit; eachOfSeries.js:35 eachOfSeries; eachSeries.js:40 eachSeries; ensureAsync.js:53 <polint:anonymous:1897:2274>; every.js:115 every; every.js:116 <polint:anonymous:3398:3411>; every.js:116 <polint:anonymous:3413:3424>; everySeries.js:41 everySeries; everySeries.js:42 <polint:anonymous:1472:1485>; everySeries.js:42 <polint:anonymous:1487:1498>; filter.js:89 filter; filterSeries.js:39 filterSeries; find.js:92 detect; find.js:93 <polint:anonymous:3080:3092>; find.js:93 <polint:anonymous:3094:3113>; flatMap.js:111 concat; flatMapLimit.js:41 concatLimit; flatMapLimit.js:43 <polint:anonymous:1575:1733>; flatMapLimit.js:44 <polint:anonymous:1617:1725>; flatMapLimit.js:48 <polint:anonymous:1735:1996>; flatMapSeries.js:37 concatSeries; foldl.js:142 reduce; foldl.js:145 <polint:anonymous:4633:4758>; foldl.js:146 <polint:anonymous:4680:4750>; foldl.js:150 <polint:anonymous:4760:4786>; forEach.js:124 eachLimit; forEachLimit.js:46 eachLimit; forEachOf.js:48 iteratorCallback; forEachOf.js:179 eachOf; forEachOfLimit.js:42 eachOfLimit; forEachOfSeries.js:35 eachOfSeries; forEachSeries.js:40 eachSeries; forever.js:56 forever; forever.js:60 next; groupByLimit.js:41 groupByLimit; groupByLimit.js:43 <polint:anonymous:1587:1749>; groupByLimit.js:44 <polint:anonymous:1629:1741>; groupByLimit.js:48 <polint:anonymous:1751:2347>; inject.js:142 reduce; inject.js:145 <polint:anonymous:4633:4758>; inject.js:146 <polint:anonymous:4680:4750>; inject.js:150 <polint:anonymous:4760:4786>; internal/applyEach.js:7 default; internal/applyEach.js:8 <polint:anonymous:132:451>; internal/applyEach.js:9 <polint:anonymous:214:424>; internal/applyEach.js:11 <polint:anonymous:296:402>; internal/asyncEachOfLimit.js:46 iterateeCallback; internal/awaitify.js:9 awaitify; internal/awaitify.js:12 awaitable; internal/createTester.js:34 <polint:anonymous:1062:1183>; internal/createTester.js:18 _createTester; internal/createTester.js:19 <polint:anonymous:467:1191>; internal/createTester.js:23 <polint:anonymous:639:1060>; internal/createTester.js:24 <polint:anonymous:693:1048>; internal/eachOfLimit.js:32 <polint:anonymous:765:2599>; internal/filter.js:38 <polint:anonymous:1043:1218>; internal/filter.js:45 <polint:anonymous:1228:1366>; internal/filter.js:51 _filter; internal/filter.js:20 <polint:anonymous:532:670>; internal/filter.js:21 <polint:anonymous:576:662>; internal/filter.js:25 <polint:anonymous:672:895>; internal/filter.js:37 <polint:anonymous:999:1226>; internal/initialParams.js:8 <polint:anonymous:128:250>; internal/iterator.js:21 <polint:anonymous:521:606>; internal/iterator.js:28 <polint:anonymous:680:834>; internal/iterator.js:40 <polint:anonymous:976:1159>; internal/map.js:14 _asyncMap; internal/map.js:20 <polint:anonymous:507:675>; internal/map.js:22 <polint:anonymous:587:667>; internal/map.js:26 <polint:anonymous:677:723>; internal/once.js:8 wrapper; internal/onlyOnce.js:8 <polint:anonymous:145:323>; internal/parallel.js:21 <polint:anonymous:526:923>; internal/parallel.js:24 <polint:anonymous:639:888>; internal/parallel.js:25 <polint:anonymous:703:880>; internal/parallel.js:32 <polint:anonymous:890:919>; internal/promiseCallback.js:10 callback; internal/queue.js:92 <polint:anonymous:2672:2767>; internal/queue.js:107 <polint:anonymous:3018:3803>; internal/queue.js:141 <polint:anonymous:3990:4012>; internal/queue.js:212 <polint:anonymous:6162:7313>; internal/reject.js:18 reject; internal/reject.js:20 <polint:anonymous:550:647>; internal/reject.js:21 <polint:anonymous:591:639>; internal/withoutIndex.js:8 <polint:anonymous:161:214>; internal/wrapAsync.js:26 wrapAsync; map.js:138 map; mapSeries.js:40 mapSeries; mapValuesLimit.js:47 mapValuesLimit; mapValuesLimit.js:51 <polint:anonymous:1804:1986>; mapValuesLimit.js:52 <polint:anonymous:1854:1978>; mapValuesLimit.js:57 <polint:anonymous:1988:2016>; memoize.js:66 <polint:anonymous:2234:2955>; memoize.js:69 <polint:anonymous:2358:2392>; queue.js:9 <polint:anonymous:218:271>; race.js:57 race; reduce.js:142 reduce; reduce.js:145 <polint:anonymous:4633:4758>; reduce.js:146 <polint:anonymous:4680:4750>; reduce.js:150 <polint:anonymous:4760:4786>; reflect.js:59 <polint:anonymous:1714:2228>; reject.js:83 reject; rejectLimit.js:41 rejectLimit; rejectSeries.js:39 rejectSeries; retry.js:132 <polint:anonymous:4598:4944>; retryable.js:63 <polint:anonymous:2064:2497>; retryable.js:68 taskFn; select.js:89 filter; selectSeries.js:39 filterSeries; seq.js:70 <polint:anonymous:2358:2516>; seq.js:74 <polint:anonymous:2518:2555>; some.js:118 some; some.js:119 <polint:anonymous:3659:3669>; someSeries.js:42 someSeries; someSeries.js:43 <polint:anonymous:1524:1534>; sortBy.js:171 sortBy; sortBy.js:173 <polint:anonymous:5666:5832>; sortBy.js:174 <polint:anonymous:5704:5824>; sortBy.js:178 <polint:anonymous:5834:5966>; timeout.js:62 <polint:anonymous:1994:2717>; transform.js:168 <polint:anonymous:5373:5436>; transform.js:170 <polint:anonymous:5438:5471>; tryEach.js:59 tryEach; tryEach.js:62 <polint:anonymous:2074:2418>; tryEach.js:63 <polint:anonymous:2133:2410>; tryEach.js:74 <polint:anonymous:2420:2449>; until.js:59 <polint:anonymous:1978:2022>; until.js:59 <polint:anonymous:1990:2021>; waterfall.js:82 waterfall; waterfall.js:93 next; whilst.js:55 whilst; whilst.js:61 next; whilst.js:68 check, marked unresolved |
| async | typed | found | applyEach.js:9 `_interopRequireDefault(_applyEach)` | identifier / call | applyEach.js:15 _interopRequireDefault | targets applyEach.js:15 _interopRequireDefault, no unresolved marker |
| async | typed | no target, marked unresolved | asyncify.js:111 `callback(error, value)` | identifier / call | internal/map.js:22 <anonymous> | no target, marked unresolved |
| async | typed | other targets, marked unresolved | auto.js:293 `taskFn(results, taskCallback)` | identifier / call | autoInject.js:173 newTask | targets all.js:115 every; all.js:116 <polint:anonymous:3398:3411>; all.js:116 <polint:anonymous:3413:3424>; allSeries.js:41 everySeries; allSeries.js:42 <polint:anonymous:1472:1485>; allSeries.js:42 <polint:anonymous:1487:1498>; any.js:118 some; any.js:119 <polint:anonymous:3659:3669>; anySeries.js:42 someSeries; anySeries.js:43 <polint:anonymous:1524:1534>; asyncify.js:78 <polint:anonymous:2637:2828>; auto.js:265 <polint:anonymous:9708:10464>; concat.js:111 concat; concatLimit.js:41 concatLimit; concatLimit.js:43 <polint:anonymous:1575:1733>; concatLimit.js:44 <polint:anonymous:1617:1725>; concatLimit.js:48 <polint:anonymous:1735:1996>; concatSeries.js:37 concatSeries; detect.js:92 detect; detect.js:93 <polint:anonymous:3080:3092>; detect.js:93 <polint:anonymous:3094:3113>; doDuring.js:44 doWhilst; doDuring.js:50 next; doUntil.js:41 <polint:anonymous:1547:1655>; doWhilst.js:44 doWhilst; doWhilst.js:50 next; during.js:55 whilst; during.js:61 next; during.js:68 check; each.js:124 eachLimit; eachLimit.js:46 eachLimit; eachOf.js:48 iteratorCallback; eachOf.js:179 eachOf; eachOfLimit.js:42 eachOfLimit; eachOfSeries.js:35 eachOfSeries; eachSeries.js:40 eachSeries; ensureAsync.js:53 <polint:anonymous:1897:2274>; every.js:115 every; every.js:116 <polint:anonymous:3398:3411>; every.js:116 <polint:anonymous:3413:3424>; everySeries.js:41 everySeries; everySeries.js:42 <polint:anonymous:1472:1485>; everySeries.js:42 <polint:anonymous:1487:1498>; filter.js:89 filter; filterSeries.js:39 filterSeries; find.js:92 detect; find.js:93 <polint:anonymous:3080:3092>; find.js:93 <polint:anonymous:3094:3113>; flatMap.js:111 concat; flatMapLimit.js:41 concatLimit; flatMapLimit.js:43 <polint:anonymous:1575:1733>; flatMapLimit.js:44 <polint:anonymous:1617:1725>; flatMapLimit.js:48 <polint:anonymous:1735:1996>; flatMapSeries.js:37 concatSeries; foldl.js:142 reduce; foldl.js:145 <polint:anonymous:4633:4758>; foldl.js:146 <polint:anonymous:4680:4750>; foldl.js:150 <polint:anonymous:4760:4786>; forEach.js:124 eachLimit; forEachLimit.js:46 eachLimit; forEachOf.js:48 iteratorCallback; forEachOf.js:179 eachOf; forEachOfLimit.js:42 eachOfLimit; forEachOfSeries.js:35 eachOfSeries; forEachSeries.js:40 eachSeries; forever.js:56 forever; forever.js:60 next; groupByLimit.js:41 groupByLimit; groupByLimit.js:43 <polint:anonymous:1587:1749>; groupByLimit.js:44 <polint:anonymous:1629:1741>; groupByLimit.js:48 <polint:anonymous:1751:2347>; inject.js:142 reduce; inject.js:145 <polint:anonymous:4633:4758>; inject.js:146 <polint:anonymous:4680:4750>; inject.js:150 <polint:anonymous:4760:4786>; internal/applyEach.js:7 default; internal/applyEach.js:8 <polint:anonymous:132:451>; internal/applyEach.js:9 <polint:anonymous:214:424>; internal/applyEach.js:11 <polint:anonymous:296:402>; internal/asyncEachOfLimit.js:46 iterateeCallback; internal/awaitify.js:9 awaitify; internal/awaitify.js:12 awaitable; internal/createTester.js:34 <polint:anonymous:1062:1183>; internal/createTester.js:18 _createTester; internal/createTester.js:19 <polint:anonymous:467:1191>; internal/createTester.js:23 <polint:anonymous:639:1060>; internal/createTester.js:24 <polint:anonymous:693:1048>; internal/eachOfLimit.js:32 <polint:anonymous:765:2599>; internal/filter.js:38 <polint:anonymous:1043:1218>; internal/filter.js:45 <polint:anonymous:1228:1366>; internal/filter.js:51 _filter; internal/filter.js:20 <polint:anonymous:532:670>; internal/filter.js:21 <polint:anonymous:576:662>; internal/filter.js:25 <polint:anonymous:672:895>; internal/filter.js:37 <polint:anonymous:999:1226>; internal/initialParams.js:8 <polint:anonymous:128:250>; internal/iterator.js:21 <polint:anonymous:521:606>; internal/iterator.js:28 <polint:anonymous:680:834>; internal/iterator.js:40 <polint:anonymous:976:1159>; internal/map.js:14 _asyncMap; internal/map.js:20 <polint:anonymous:507:675>; internal/map.js:22 <polint:anonymous:587:667>; internal/map.js:26 <polint:anonymous:677:723>; internal/once.js:8 wrapper; internal/onlyOnce.js:8 <polint:anonymous:145:323>; internal/parallel.js:21 <polint:anonymous:526:923>; internal/parallel.js:24 <polint:anonymous:639:888>; internal/parallel.js:25 <polint:anonymous:703:880>; internal/parallel.js:32 <polint:anonymous:890:919>; internal/promiseCallback.js:10 callback; internal/queue.js:92 <polint:anonymous:2672:2767>; internal/queue.js:107 <polint:anonymous:3018:3803>; internal/queue.js:141 <polint:anonymous:3990:4012>; internal/queue.js:212 <polint:anonymous:6162:7313>; internal/reject.js:18 reject; internal/reject.js:20 <polint:anonymous:550:647>; internal/reject.js:21 <polint:anonymous:591:639>; internal/withoutIndex.js:8 <polint:anonymous:161:214>; internal/wrapAsync.js:26 wrapAsync; map.js:138 map; mapSeries.js:40 mapSeries; mapValuesLimit.js:47 mapValuesLimit; mapValuesLimit.js:51 <polint:anonymous:1804:1986>; mapValuesLimit.js:52 <polint:anonymous:1854:1978>; mapValuesLimit.js:57 <polint:anonymous:1988:2016>; memoize.js:66 <polint:anonymous:2234:2955>; memoize.js:69 <polint:anonymous:2358:2392>; queue.js:9 <polint:anonymous:218:271>; race.js:57 race; reduce.js:142 reduce; reduce.js:145 <polint:anonymous:4633:4758>; reduce.js:146 <polint:anonymous:4680:4750>; reduce.js:150 <polint:anonymous:4760:4786>; reflect.js:59 <polint:anonymous:1714:2228>; reject.js:83 reject; rejectLimit.js:41 rejectLimit; rejectSeries.js:39 rejectSeries; retry.js:132 <polint:anonymous:4598:4944>; retryable.js:63 <polint:anonymous:2064:2497>; retryable.js:68 taskFn; select.js:89 filter; selectSeries.js:39 filterSeries; seq.js:70 <polint:anonymous:2358:2516>; seq.js:74 <polint:anonymous:2518:2555>; some.js:118 some; some.js:119 <polint:anonymous:3659:3669>; someSeries.js:42 someSeries; someSeries.js:43 <polint:anonymous:1524:1534>; sortBy.js:171 sortBy; sortBy.js:173 <polint:anonymous:5666:5832>; sortBy.js:174 <polint:anonymous:5704:5824>; sortBy.js:178 <polint:anonymous:5834:5966>; timeout.js:62 <polint:anonymous:1994:2717>; transform.js:168 <polint:anonymous:5373:5436>; transform.js:170 <polint:anonymous:5438:5471>; tryEach.js:59 tryEach; tryEach.js:62 <polint:anonymous:2074:2418>; tryEach.js:63 <polint:anonymous:2133:2410>; tryEach.js:74 <polint:anonymous:2420:2449>; until.js:59 <polint:anonymous:1978:2022>; until.js:59 <polint:anonymous:1990:2021>; waterfall.js:82 waterfall; waterfall.js:93 next; whilst.js:55 whilst; whilst.js:61 next; whilst.js:68 check, marked unresolved |
| chalk | base | found | source/index.js:37 `assertValidLevel(options.level)` | identifier / call | source/index.js:17 assertValidLevel | targets source/index.js:17 assertValidLevel, no unresolved marker |
| chalk | base | no target, marked unresolved | source/index.js:91 `style.ansi(ansiStyles.rgbToAnsi(red, green, blue))` | member / call | source/vendor/ansi-styles/index.js:4 <anonymous> | no target, marked unresolved |
| chalk | refined | found | source/index.js:37 `assertValidLevel(options.level)` | identifier / call | source/index.js:17 assertValidLevel | targets source/index.js:17 assertValidLevel, no unresolved marker |
| chalk | refined | no target, marked unresolved | source/index.js:91 `style.ansi(ansiStyles.rgbToAnsi(red, green, blue))` | member / call | source/vendor/ansi-styles/index.js:4 <anonymous> | no target, marked unresolved |
| chalk | typed | found | source/index.js:37 `assertValidLevel(options.level)` | identifier / call | source/index.js:17 assertValidLevel | targets source/index.js:17 assertValidLevel, no unresolved marker |
| chalk | typed | no target, marked unresolved | source/index.js:91 `style.ansi(ansiStyles.rgbToAnsi(red, green, blue))` | member / call | source/vendor/ansi-styles/index.js:4 <anonymous> | no target, marked unresolved |
| dayjs | base | found | dayjs.min.js:1 `m(r,2,"0")` | identifier / call | dayjs.min.js:1 m | targets dayjs.min.js:1 <polint:anonymous:859:948>, no unresolved marker |
| dayjs | base | no target, marked unresolved | dayjs.min.js:1 `e.date()` | member / call | dayjs.min.js:1 <anonymous> | no target, marked unresolved |
| dayjs | refined | found | dayjs.min.js:1 `m(r,2,"0")` | identifier / call | dayjs.min.js:1 m | targets dayjs.min.js:1 <polint:anonymous:859:948>, no unresolved marker |
| dayjs | refined | no target, marked unresolved | dayjs.min.js:1 `e.date()` | member / call | dayjs.min.js:1 <anonymous> | no target, marked unresolved |
| dayjs | typed | found | dayjs.min.js:1 `m(r,2,"0")` | identifier / call | dayjs.min.js:1 m | targets dayjs.min.js:1 <polint:anonymous:859:948>, no unresolved marker |
| dayjs | typed | no target, marked unresolved | dayjs.min.js:1 `e.date()` | member / call | dayjs.min.js:1 <anonymous> | no target, marked unresolved |
| underscore | base | found | underscore-umd.js:289 `isObject(obj)` | identifier / call | underscore-umd.js:88 isObject | targets underscore-umd.js:88 isObject, no unresolved marker |
| underscore | base | no target, marked unresolved | underscore-umd.js:1006 `sourceFunc.apply(context, args)` | member / call | underscore-umd.js:1238 before | no target, marked unresolved |
| underscore | base | other targets, marked unresolved | underscore-umd.js:1664 `iteratee(value, key, obj)` | identifier / call | underscore-umd.js:1677 iteratee | targets underscore-umd.js:710 identity; underscore-umd.js:718 <polint:anonymous:25165:25220>; underscore-umd.js:727 <polint:anonymous:25455:25509>; underscore-umd.js:738 <polint:anonymous:25846:25913>; underscore-umd.js:742 <polint:anonymous:26006:26111>; underscore-umd.js:745 <polint:anonymous:26134:26265>; underscore-umd.js:749 <polint:anonymous:26284:26347>; underscore-umd.js:1209 <polint:anonymous:42056:42122>; underscore-umd.js:1412 <polint:anonymous:49081:49179>; underscore-umd.js:1464 <polint:anonymous:50883:51216>; underscore-umd.js:1502 <polint:anonymous:52164:52401>; underscore-umd.js:1527 <polint:anonymous:53017:53252>; underscore-umd.js:1582 <polint:anonymous:54836:54985>; underscore-umd.js:1604 <polint:anonymous:55513:55630>; underscore-umd.js:1645 keyInObj; underscore-umd.js:1729 <polint:anonymous:60125:60184>; underscore-umd.js:1866 <polint:anonymous:64437:64679>; underscore-umd.js:1878 <polint:anonymous:64831:65181>; underscore-umd.js:1893 <polint:anonymous:65277:65505>; underscore-umd.js:202 isNaN$1; underscore-umd.js:223 <polint:anonymous:7915:7982>, marked unresolved |
| underscore | refined | found | underscore-umd.js:289 `isObject(obj)` | identifier / call | underscore-umd.js:88 isObject | targets underscore-umd.js:88 isObject, no unresolved marker |
| underscore | refined | no target, marked unresolved | underscore-umd.js:1006 `sourceFunc.apply(context, args)` | member / call | underscore-umd.js:1238 before | no target, marked unresolved |
| underscore | refined | other targets, marked unresolved | underscore-umd.js:1664 `iteratee(value, key, obj)` | identifier / call | underscore-umd.js:1677 iteratee | targets underscore-umd.js:710 identity; underscore-umd.js:718 <polint:anonymous:25165:25220>; underscore-umd.js:727 <polint:anonymous:25455:25509>; underscore-umd.js:738 <polint:anonymous:25846:25913>; underscore-umd.js:742 <polint:anonymous:26006:26111>; underscore-umd.js:745 <polint:anonymous:26134:26265>; underscore-umd.js:749 <polint:anonymous:26284:26347>; underscore-umd.js:1209 <polint:anonymous:42056:42122>; underscore-umd.js:1412 <polint:anonymous:49081:49179>; underscore-umd.js:1464 <polint:anonymous:50883:51216>; underscore-umd.js:1502 <polint:anonymous:52164:52401>; underscore-umd.js:1527 <polint:anonymous:53017:53252>; underscore-umd.js:1582 <polint:anonymous:54836:54985>; underscore-umd.js:1604 <polint:anonymous:55513:55630>; underscore-umd.js:1645 keyInObj; underscore-umd.js:1729 <polint:anonymous:60125:60184>; underscore-umd.js:1866 <polint:anonymous:64437:64679>; underscore-umd.js:1878 <polint:anonymous:64831:65181>; underscore-umd.js:1893 <polint:anonymous:65277:65505>; underscore-umd.js:202 isNaN$1; underscore-umd.js:223 <polint:anonymous:7915:7982>, marked unresolved |
| underscore | typed | found | underscore-umd.js:289 `isObject(obj)` | identifier / call | underscore-umd.js:88 isObject | targets underscore-umd.js:88 isObject, no unresolved marker |
| underscore | typed | no target, marked unresolved | underscore-umd.js:1006 `sourceFunc.apply(context, args)` | member / call | underscore-umd.js:1238 before | no target, marked unresolved |
| underscore | typed | other targets, marked unresolved | underscore-umd.js:1664 `iteratee(value, key, obj)` | identifier / call | underscore-umd.js:1677 iteratee | targets underscore-umd.js:710 identity; underscore-umd.js:718 <polint:anonymous:25165:25220>; underscore-umd.js:727 <polint:anonymous:25455:25509>; underscore-umd.js:738 <polint:anonymous:25846:25913>; underscore-umd.js:742 <polint:anonymous:26006:26111>; underscore-umd.js:745 <polint:anonymous:26134:26265>; underscore-umd.js:749 <polint:anonymous:26284:26347>; underscore-umd.js:1209 <polint:anonymous:42056:42122>; underscore-umd.js:1412 <polint:anonymous:49081:49179>; underscore-umd.js:1464 <polint:anonymous:50883:51216>; underscore-umd.js:1502 <polint:anonymous:52164:52401>; underscore-umd.js:1527 <polint:anonymous:53017:53252>; underscore-umd.js:1582 <polint:anonymous:54836:54985>; underscore-umd.js:1604 <polint:anonymous:55513:55630>; underscore-umd.js:1645 keyInObj; underscore-umd.js:1729 <polint:anonymous:60125:60184>; underscore-umd.js:1866 <polint:anonymous:64437:64679>; underscore-umd.js:1878 <polint:anonymous:64831:65181>; underscore-umd.js:1893 <polint:anonymous:65277:65505>; underscore-umd.js:202 isNaN$1; underscore-umd.js:223 <polint:anonymous:7915:7982>, marked unresolved |

## Tiers

Recall over the same observed edges as above, split by the tier of the edge that found the callee
(`research/package-graphs/TIERS.md`): `Exact` and `SetupAware` are exact, except typed-hierarchy edges
that dispatch to an `implementation` or `union_member`, which are candidates and count as conservative;
`Conservative` and `Ambiguous` are conservative; `Heuristic` is apart and never counts in the first three
columns. **exact-complete** also needs the site to be complete: at least one exact or conservative target and
no unresolved row.

| package | plan | observed edges | exact-complete | exact | exact + conservative | apart only | apart edges |
|---|---|---|---|---|---|---|---|
| axios | base | 675 | 302/675 (45%) | 302/675 (45%) | 569/675 (84%) | 35/675 (5%) | 55 |
| axios | refined | 675 | 302/675 (45%) | 302/675 (45%) | 569/675 (84%) | 35/675 (5%) | 186 |
| axios | typed | 675 | 302/675 (45%) | 302/675 (45%) | 603/675 (89%) | 3/675 (0%) | 186 |
| zod | base | 1308 | 502/1308 (38%) | 674/1308 (52%) | 763/1308 (58%) | 418/1308 (32%) | 3015 |
| zod | refined | 1308 | 502/1308 (38%) | 674/1308 (52%) | 763/1308 (58%) | 418/1308 (32%) | 3588 |
| zod | typed | 1308 | 502/1308 (38%) | 674/1308 (52%) | 771/1308 (59%) | 418/1308 (32%) | 3588 |
| commander | base | 318 | 34/318 (11%) | 34/318 (11%) | 85/318 (27%) | 98/318 (31%) | 146 |
| commander | refined | 318 | 34/318 (11%) | 34/318 (11%) | 85/318 (27%) | 98/318 (31%) | 146 |
| commander | typed | 318 | 34/318 (11%) | 34/318 (11%) | 298/318 (94%) | 8/318 (3%) | 146 |
| async | base | 637 | 325/637 (51%) | 325/637 (51%) | 485/637 (76%) | 146/637 (23%) | 45951 |
| async | refined | 637 | 325/637 (51%) | 325/637 (51%) | 485/637 (76%) | 146/637 (23%) | 45951 |
| async | typed | 637 | 325/637 (51%) | 325/637 (51%) | 490/637 (77%) | 141/637 (22%) | 45951 |
| chalk | base | 72 | 43/72 (60%) | 43/72 (60%) | 56/72 (78%) | 0/72 (0%) | 0 |
| chalk | refined | 72 | 43/72 (60%) | 43/72 (60%) | 56/72 (78%) | 0/72 (0%) | 2 |
| chalk | typed | 72 | 43/72 (60%) | 43/72 (60%) | 56/72 (78%) | 0/72 (0%) | 2 |
| dayjs | base | 112 | 34/112 (30%) | 34/112 (30%) | 88/112 (79%) | 0/112 (0%) | 0 |
| dayjs | refined | 112 | 34/112 (30%) | 34/112 (30%) | 88/112 (79%) | 0/112 (0%) | 8 |
| dayjs | typed | 112 | 34/112 (30%) | 34/112 (30%) | 88/112 (79%) | 0/112 (0%) | 8 |
| underscore | base | 260 | 196/260 (75%) | 196/260 (75%) | 250/260 (96%) | 0/260 (0%) | 0 |
| underscore | refined | 260 | 196/260 (75%) | 196/260 (75%) | 250/260 (96%) | 0/260 (0%) | 8 |
| underscore | typed | 260 | 196/260 (75%) | 196/260 (75%) | 250/260 (96%) | 0/260 (0%) | 8 |
| **total** | base | 3382 | 1436/3382 (42%) | 1608/3382 (48%) | 2296/3382 (68%) | 697/3382 (21%) | 49167 |
| **total** | refined | 3382 | 1436/3382 (42%) | 1608/3382 (48%) | 2296/3382 (68%) | 697/3382 (21%) | 49889 |
| **total** | typed | 3382 | 1436/3382 (42%) | 1608/3382 (48%) | 2556/3382 (76%) | 570/3382 (17%) | 49889 |

Targets per site that claims any in the tier: mean / p95 (nearest rank) / max.

| package | plan | exact | conservative | exact + conservative | apart |
|---|---|---|---|---|---|
| axios | base | 1.0 / 1 / 1 (615 sites) | 1.17 / 2 / 19 (440 sites) | 1.07 / 1 / 19 (1055 sites) | 1.0 / 1 / 1 (55 sites) |
| axios | refined | 1.0 / 1 / 1 (615 sites) | 1.17 / 2 / 19 (440 sites) | 1.07 / 1 / 19 (1055 sites) | 1.0 / 1 / 1 (118 sites) |
| axios | typed | 1.0 / 1 / 1 (615 sites) | 1.15 / 2 / 19 (492 sites) | 1.07 / 1 / 19 (1107 sites) | 1.0 / 1 / 1 (118 sites) |
| zod | base | 1.0 / 1 / 1 (2547 sites) | 1.18 / 1 / 39 (815 sites) | 1.04 / 1 / 39 (3362 sites) | 4.63 / 47 / 252 (651 sites) |
| zod | refined | 1.0 / 1 / 1 (2547 sites) | 1.09 / 1 / 39 (1663 sites) | 1.04 / 1 / 39 (3362 sites) | 3.57 / 36 / 252 (921 sites) |
| zod | typed | 1.0 / 1 / 1 (2547 sites) | 1.12 / 1 / 39 (1851 sites) | 1.06 / 1 / 39 (3550 sites) | 3.57 / 36 / 252 (921 sites) |
| commander | base | 1.0 / 1 / 1 (62 sites) | 1.01 / 1 / 2 (76 sites) | 1.01 / 1 / 2 (138 sites) | 1.04 / 1 / 3 (140 sites) |
| commander | refined | 1.0 / 1 / 1 (62 sites) | 1.01 / 1 / 2 (76 sites) | 1.01 / 1 / 2 (138 sites) | 1.04 / 1 / 3 (140 sites) |
| commander | typed | 1.0 / 1 / 1 (62 sites) | 1.0 / 1 / 2 (408 sites) | 1.0 / 1 / 2 (470 sites) | 1.04 / 1 / 3 (140 sites) |
| async | base | 1.0 / 1 / 1 (448 sites) | 10.95 / 166 / 183 (217 sites) | 4.25 / 4 / 183 (665 sites) | 154.2 / 169 / 180 (298 sites) |
| async | refined | 1.0 / 1 / 1 (448 sites) | 10.95 / 166 / 183 (217 sites) | 4.25 / 4 / 183 (665 sites) | 154.2 / 169 / 180 (298 sites) |
| async | typed | 1.0 / 1 / 1 (448 sites) | 10.51 / 12 / 183 (227 sites) | 4.2 / 4 / 183 (675 sites) | 154.2 / 169 / 180 (298 sites) |
| chalk | base | 1.0 / 1 / 1 (47 sites) | 1.0 / 1 / 1 (18 sites) | 1.0 / 1 / 1 (65 sites) | – |
| chalk | refined | 1.0 / 1 / 1 (47 sites) | 1.0 / 1 / 1 (18 sites) | 1.0 / 1 / 1 (65 sites) | 1.0 / 1 / 1 (1 sites) |
| chalk | typed | 1.0 / 1 / 1 (47 sites) | 1.0 / 1 / 1 (18 sites) | 1.0 / 1 / 1 (65 sites) | 1.0 / 1 / 1 (1 sites) |
| dayjs | base | 1.0 / 1 / 1 (41 sites) | 1.0 / 1 / 1 (54 sites) | 1.0 / 1 / 1 (95 sites) | – |
| dayjs | refined | 1.0 / 1 / 1 (41 sites) | 1.0 / 1 / 1 (54 sites) | 1.0 / 1 / 1 (95 sites) | 1.0 / 1 / 1 (4 sites) |
| dayjs | typed | 1.0 / 1 / 1 (41 sites) | 1.0 / 1 / 1 (54 sites) | 1.0 / 1 / 1 (95 sites) | 1.0 / 1 / 1 (4 sites) |
| underscore | base | 1.0 / 1 / 1 (246 sites) | 8.37 / 21 / 22 (78 sites) | 2.77 / 21 / 22 (324 sites) | – |
| underscore | refined | 1.0 / 1 / 1 (246 sites) | 8.37 / 21 / 22 (78 sites) | 2.77 / 21 / 22 (324 sites) | 1.0 / 1 / 1 (4 sites) |
| underscore | typed | 1.0 / 1 / 1 (246 sites) | 8.37 / 21 / 22 (78 sites) | 2.77 / 21 / 22 (324 sites) | 1.0 / 1 / 1 (4 sites) |

## Precision by tier

At executed sites only: a site counts once the run was seen calling a polint function from it, and a
claimed target counts when it is a function in an instrumented file. **precision** is the share of those
targets the run called there; it is a lower bound, since a run takes only some paths. A target the run never
called on a single-target answer in the exact tier is a **wrong exact target**; the gate is 0.

| package | plan | exact: precision, single / multi not called | conservative: precision, single / multi not called | apart: precision, single / multi not called |
|---|---|---|---|---|
| axios | base | 302/302 (100%), 0 / 0 | 267/298 (90%), 1 / 30 | 35/35 (100%), 0 / 0 |
| axios | refined | 302/302 (100%), 0 / 0 | 267/298 (90%), 1 / 30 | 45/45 (100%), 0 / 0 |
| axios | typed | 302/302 (100%), 0 / 0 | 301/332 (91%), 1 / 30 | 45/45 (100%), 0 / 0 |
| zod | base | 674/674 (100%), 0 / 0 | 89/129 (69%), 0 / 40 | 418/1645 (25%), 0 / 1227 |
| zod | refined | 674/674 (100%), 0 / 0 | 261/301 (87%), 0 / 40 | 679/1906 (36%), 0 / 1227 |
| zod | typed | 674/674 (100%), 0 / 0 | 269/309 (87%), 0 / 40 | 679/1906 (36%), 0 / 1227 |
| commander | base | 34/34 (100%), 0 / 0 | 51/51 (100%), 0 / 0 | 98/98 (100%), 0 / 0 |
| commander | refined | 34/34 (100%), 0 / 0 | 51/51 (100%), 0 / 0 | 98/98 (100%), 0 / 0 |
| commander | typed | 34/34 (100%), 0 / 0 | 264/265 (100%), 1 / 0 | 98/98 (100%), 0 / 0 |
| async | base | 325/325 (100%), 0 / 0 | 160/924 (17%), 0 / 764 | 146/14812 (1%), 0 / 14666 |
| async | refined | 325/325 (100%), 0 / 0 | 160/924 (17%), 0 / 764 | 146/14812 (1%), 0 / 14666 |
| async | typed | 325/325 (100%), 0 / 0 | 165/929 (18%), 0 / 764 | 146/14812 (1%), 0 / 14666 |
| chalk | base | 43/43 (100%), 0 / 0 | 13/13 (100%), 0 / 0 | – |
| chalk | refined | 43/43 (100%), 0 / 0 | 13/13 (100%), 0 / 0 | – |
| chalk | typed | 43/43 (100%), 0 / 0 | 13/13 (100%), 0 / 0 | – |
| dayjs | base | 34/34 (100%), 0 / 0 | 54/54 (100%), 0 / 0 | – |
| dayjs | refined | 34/34 (100%), 0 / 0 | 54/54 (100%), 0 / 0 | – |
| dayjs | typed | 34/34 (100%), 0 / 0 | 54/54 (100%), 0 / 0 | – |
| underscore | base | 196/196 (100%), 0 / 0 | 54/303 (18%), 0 / 249 | – |
| underscore | refined | 196/196 (100%), 0 / 0 | 54/303 (18%), 0 / 249 | 1/1 (100%), 0 / 0 |
| underscore | typed | 196/196 (100%), 0 / 0 | 54/303 (18%), 0 / 249 | 1/1 (100%), 0 / 0 |
| **total** | base | **1608/1608 (100%), 0 / 0** | **688/1772 (39%), 1 / 1083** | **697/16590 (4%), 0 / 15893** |
| **total** | refined | **1608/1608 (100%), 0 / 0** | **860/1944 (44%), 1 / 1083** | **969/16862 (6%), 0 / 15893** |
| **total** | typed | **1608/1608 (100%), 0 / 0** | **1120/2205 (51%), 2 / 1083** | **969/16862 (6%), 0 / 15893** |

| package | plan | wrong exact target at | site text | target | the run called |
|---|---|---|---|---|---|
| none | | | | | |

## Open sites

A site is open when it has no exact or conservative target, or the answer marks it unresolved. Each open
site counts under every reason its answer gives; `none recorded` is an open site whose answer gives none,
which the engine must not produce.

| package | plan | call sites | complete | open | open with a claimed target | open by reason |
|---|---|---|---|---|---|---|
| axios | base | 2098 | 621/2098 (30%) | 1477 | 434 | DynamicProperty 853, MissingSemanticReference 396, Inflow(EscapedObject) 269, none recorded 170, Unknown 113, Inflow(InwardParameter) 40, CallApplyBind 35, Inflow(OutsideValue) 15, Inflow(InwardThis) 3, Inflow(OpenResult) 2 |
| axios | refined | 2098 | 621/2098 (30%) | 1477 | 434 | DynamicProperty 853, MissingSemanticReference 396, Inflow(EscapedObject) 269, none recorded 170, Unknown 113, Inflow(InwardParameter) 40, CallApplyBind 35, Inflow(OutsideValue) 15, Inflow(InwardThis) 3, Inflow(OpenResult) 2 |
| axios | typed | 2098 | 621/2098 (30%) | 1477 | 486 | DynamicProperty 853, MissingSemanticReference 396, UnknownCallee 359, Inflow(EscapedObject) 269, none recorded 157, Unknown 113, Inflow(InwardParameter) 40, CallApplyBind 35, Inflow(OutsideValue) 15, Inflow(InwardThis) 3, Inflow(OpenResult) 2 |
| zod | base | 6109 | 1706/6109 (28%) | 4403 | 1656 | DynamicProperty 2269, Inflow(EscapedObject) 963, MissingSemanticReference 915, Inflow(InwardParameter) 335, none recorded 295, Unknown 239, Inflow(InwardThis) 117, CallApplyBind 45, Inflow(OutsideValue) 2, Reflection 1 |
| zod | refined | 6109 | 1706/6109 (28%) | 4403 | 1656 | DynamicProperty 2269, Inflow(EscapedObject) 963, MissingSemanticReference 915, Inflow(InwardParameter) 335, none recorded 295, Unknown 239, Inflow(InwardThis) 117, CallApplyBind 45, Inflow(OutsideValue) 2, Reflection 1 |
| zod | typed | 6109 | 1706/6109 (28%) | 4403 | 1844 | DynamicProperty 2269, Inflow(EscapedObject) 963, MissingSemanticReference 915, Inflow(InwardParameter) 335, none recorded 295, Unknown 239, UnknownCallee 122, Inflow(InwardThis) 117, CallApplyBind 45, Inflow(OutsideValue) 2, Reflection 1 |
| commander | base | 814 | 63/814 (8%) | 751 | 75 | DynamicProperty 588, MissingSemanticReference 135, Unknown 75, none recorded 27, CallApplyBind 1 |
| commander | refined | 814 | 63/814 (8%) | 751 | 75 | DynamicProperty 588, MissingSemanticReference 135, Unknown 75, none recorded 27, CallApplyBind 1 |
| commander | typed | 814 | 63/814 (8%) | 751 | 407 | DynamicProperty 588, MissingSemanticReference 135, UnknownCallee 110, Unknown 75, none recorded 26, CallApplyBind 1 |
| async | base | 1605 | 451/1605 (28%) | 1154 | 214 | MissingSemanticReference 622, DynamicProperty 362, Inflow(EscapedObject) 159, none recorded 155, Unknown 48, CallApplyBind 14, Inflow(InwardParameter) 4, Inflow(OutsideValue) 3, UnsupportedSyntax 1 |
| async | refined | 1605 | 451/1605 (28%) | 1154 | 214 | MissingSemanticReference 622, DynamicProperty 362, Inflow(EscapedObject) 159, none recorded 155, Unknown 48, CallApplyBind 14, Inflow(InwardParameter) 4, Inflow(OutsideValue) 3, UnsupportedSyntax 1 |
| async | typed | 1605 | 451/1605 (28%) | 1154 | 224 | UnknownCallee 666, MissingSemanticReference 622, DynamicProperty 362, Inflow(EscapedObject) 159, none recorded 155, Unknown 48, CallApplyBind 14, Inflow(InwardParameter) 4, Inflow(OutsideValue) 3, UnsupportedSyntax 1 |
| chalk | base | 144 | 47/144 (33%) | 97 | 18 | DynamicProperty 39, none recorded 33, MissingSemanticReference 23, Inflow(EscapedObject) 12, Unknown 6 |
| chalk | refined | 144 | 47/144 (33%) | 97 | 18 | DynamicProperty 39, none recorded 33, MissingSemanticReference 23, Inflow(EscapedObject) 12, Unknown 6 |
| chalk | typed | 144 | 47/144 (33%) | 97 | 18 | DynamicProperty 39, none recorded 33, UnknownCallee 24, MissingSemanticReference 23, Inflow(EscapedObject) 12, Unknown 6 |
| dayjs | base | 181 | 41/181 (23%) | 140 | 54 | DynamicProperty 89, MissingSemanticReference 41, Inflow(InwardParameter) 28, Inflow(EscapedObject) 20, none recorded 10, Inflow(InwardThis) 4, Unknown 2 |
| dayjs | refined | 181 | 41/181 (23%) | 140 | 54 | DynamicProperty 89, MissingSemanticReference 41, Inflow(InwardParameter) 28, Inflow(EscapedObject) 20, none recorded 10, Inflow(InwardThis) 4, Unknown 2 |
| dayjs | typed | 181 | 41/181 (23%) | 140 | 54 | DynamicProperty 89, UnknownCallee 44, MissingSemanticReference 41, Inflow(InwardParameter) 28, Inflow(EscapedObject) 20, none recorded 10, Inflow(InwardThis) 4, Unknown 2 |
| underscore | base | 455 | 270/455 (59%) | 185 | 54 | MissingSemanticReference 86, DynamicProperty 44, CallApplyBind 41, Inflow(InwardParameter) 19, Unknown 18, none recorded 14, Inflow(OutsideValue) 10, Inflow(EscapedObject) 7 |
| underscore | refined | 455 | 270/455 (59%) | 185 | 54 | MissingSemanticReference 86, DynamicProperty 44, CallApplyBind 41, Inflow(InwardParameter) 19, Unknown 18, none recorded 14, Inflow(OutsideValue) 10, Inflow(EscapedObject) 7 |
| underscore | typed | 455 | 270/455 (59%) | 185 | 54 | MissingSemanticReference 86, DynamicProperty 44, CallApplyBind 41, UnknownCallee 30, Inflow(InwardParameter) 19, Unknown 18, none recorded 14, Inflow(OutsideValue) 10, Inflow(EscapedObject) 7 |

## Budget events

The solver's budget status for each dump and every budget it reported as reached. The target is none on
the tune set, and a budget event must never discard an edge already derived.

| package | plan | status | reasons |
|---|---|---|---|
| axios | base | WithinBudget | – |
| axios | refined | WithinBudget | – |
| axios | typed | WithinBudget | – |
| zod | base | WithinBudget | – |
| zod | refined | WithinBudget | – |
| zod | typed | WithinBudget | – |
| commander | base | WithinBudget | – |
| commander | refined | WithinBudget | – |
| commander | typed | WithinBudget | – |
| async | base | WithinBudget | – |
| async | refined | WithinBudget | – |
| async | typed | WithinBudget | – |
| chalk | base | WithinBudget | – |
| chalk | refined | WithinBudget | – |
| chalk | typed | WithinBudget | – |
| dayjs | base | WithinBudget | – |
| dayjs | refined | WithinBudget | – |
| dayjs | typed | WithinBudget | – |
| underscore | base | WithinBudget | – |
| underscore | refined | WithinBudget | – |
| underscore | typed | WithinBudget | – |

## Refined edges

Every refined row by the tier the bench gives it, its algorithm, status and precision, and whether it
names a function. Rows with a status other than `Resolved` or `Ambiguous` claim nothing.

| package | plan | tier | algorithm | status | precision | names a function | rows |
|---|---|---|---|---|---|---|---|
| axios | refined | none | PointsTo | Unresolved | Unknown | no | 1726 |
| axios | refined | exact | PointsTo | Resolved | Exact | yes | 615 |
| axios | refined | conservative | PointsTo | Resolved | Conservative | yes | 521 |
| axios | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 488 |
| axios | refined | exact | DirectReference | Resolved | SetupAware | yes | 388 |
| axios | refined | exact | ImportBinding | Resolved | SetupAware | yes | 94 |
| axios | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 67 |
| axios | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 67 |
| axios | refined | apart | PointsTo | Resolved | Heuristic | yes | 52 |
| axios | refined | exact | ImportBinding | Resolved | SetupAware | no | 8 |
| axios | refined | exact | SummaryAssisted | Resolved | SetupAware | no | 8 |
| axios | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 6 |
| axios | typed | none | PointsTo | Unresolved | Unknown | no | 1726 |
| axios | typed | exact | PointsTo | Resolved | Exact | yes | 615 |
| axios | typed | conservative | PointsTo | Resolved | Conservative | yes | 521 |
| axios | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 488 |
| axios | typed | exact | DirectReference | Resolved | SetupAware | yes | 388 |
| axios | typed | none | TypeHierarchy | Unresolved | Unknown | no | 359 |
| axios | typed | exact | ImportBinding | Resolved | SetupAware | yes | 94 |
| axios | typed | apart | SummaryAssisted | Resolved | Heuristic | yes | 67 |
| axios | typed | apart | SyntaxOnly | Resolved | Heuristic | yes | 67 |
| axios | typed | apart | PointsTo | Resolved | Heuristic | yes | 52 |
| axios | typed | conservative | TypeHierarchy | Resolved | Conservative | yes | 52 |
| axios | typed | exact | ImportBinding | Resolved | SetupAware | no | 8 |
| axios | typed | exact | SummaryAssisted | Resolved | SetupAware | no | 8 |
| axios | typed | exact | ConstructorBinding | Resolved | SetupAware | yes | 6 |
| zod | refined | none | PointsTo | Unresolved | Unknown | no | 4885 |
| zod | refined | apart | PointsTo | Resolved | Heuristic | yes | 3268 |
| zod | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 2506 |
| zod | refined | conservative | PointsTo | Resolved | Conservative | yes | 1811 |
| zod | refined | exact | PointsTo | Resolved | Exact | yes | 1439 |
| zod | refined | exact | ImportBinding | Resolved | SetupAware | yes | 1300 |
| zod | refined | exact | DirectReference | Resolved | SetupAware | yes | 1081 |
| zod | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 160 |
| zod | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 160 |
| zod | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 125 |
| zod | refined | exact | ImportBinding | Resolved | SetupAware | no | 3 |
| zod | refined | exact | SummaryAssisted | Resolved | SetupAware | no | 3 |
| zod | refined | none | PointsTo | Unresolved | Unsupported | no | 1 |
| zod | typed | none | PointsTo | Unresolved | Unknown | no | 4885 |
| zod | typed | apart | PointsTo | Resolved | Heuristic | yes | 3268 |
| zod | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 2506 |
| zod | typed | conservative | PointsTo | Resolved | Conservative | yes | 1811 |
| zod | typed | exact | PointsTo | Resolved | Exact | yes | 1439 |
| zod | typed | exact | ImportBinding | Resolved | SetupAware | yes | 1300 |
| zod | typed | exact | DirectReference | Resolved | SetupAware | yes | 1081 |
| zod | typed | conservative | TypeHierarchy | Resolved | Conservative | yes | 258 |
| zod | typed | apart | SummaryAssisted | Resolved | Heuristic | yes | 160 |
| zod | typed | apart | SyntaxOnly | Resolved | Heuristic | yes | 160 |
| zod | typed | exact | ConstructorBinding | Resolved | SetupAware | yes | 125 |
| zod | typed | none | TypeHierarchy | Unresolved | Unknown | no | 122 |
| zod | typed | exact | ImportBinding | Resolved | SetupAware | no | 3 |
| zod | typed | exact | SummaryAssisted | Resolved | SetupAware | no | 3 |
| zod | typed | none | PointsTo | Unresolved | Unsupported | no | 1 |
| commander | refined | none | PointsTo | Unresolved | Unknown | no | 799 |
| commander | refined | apart | PointsTo | Resolved | Heuristic | yes | 146 |
| commander | refined | conservative | PointsTo | Resolved | Conservative | yes | 77 |
| commander | refined | exact | PointsTo | Resolved | Exact | yes | 62 |
| commander | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 42 |
| commander | refined | exact | DirectReference | Resolved | SetupAware | yes | 41 |
| commander | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 1 |
| commander | typed | none | PointsTo | Unresolved | Unknown | no | 799 |
| commander | typed | conservative | TypeHierarchy | Resolved | Conservative | yes | 332 |
| commander | typed | apart | PointsTo | Resolved | Heuristic | yes | 146 |
| commander | typed | none | TypeHierarchy | Unresolved | Unknown | no | 110 |
| commander | typed | conservative | PointsTo | Resolved | Conservative | yes | 77 |
| commander | typed | exact | PointsTo | Resolved | Exact | yes | 62 |
| commander | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 42 |
| commander | typed | exact | DirectReference | Resolved | SetupAware | yes | 41 |
| commander | typed | exact | ConstructorBinding | Resolved | SetupAware | yes | 1 |
| async | refined | apart | PointsTo | Resolved | Heuristic | yes | 45951 |
| async | refined | conservative | PointsTo | Resolved | Conservative | yes | 2376 |
| async | refined | none | PointsTo | Unresolved | Unknown | no | 1212 |
| async | refined | exact | PointsTo | Resolved | Exact | yes | 448 |
| async | refined | exact | DirectReference | Resolved | SetupAware | yes | 440 |
| async | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 440 |
| async | refined | none | FrameworkModel | Unsupported | Conservative | no | 1 |
| async | typed | apart | PointsTo | Resolved | Heuristic | yes | 45951 |
| async | typed | conservative | PointsTo | Resolved | Conservative | yes | 2376 |
| async | typed | none | PointsTo | Unresolved | Unknown | no | 1212 |
| async | typed | none | TypeHierarchy | Unresolved | Unknown | no | 666 |
| async | typed | exact | PointsTo | Resolved | Exact | yes | 448 |
| async | typed | exact | DirectReference | Resolved | SetupAware | yes | 440 |
| async | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 440 |
| async | typed | conservative | TypeHierarchy | Resolved | Conservative | yes | 10 |
| async | typed | none | FrameworkModel | Unsupported | Conservative | no | 1 |
| chalk | refined | none | PointsTo | Unresolved | Unknown | no | 80 |
| chalk | refined | exact | PointsTo | Resolved | Exact | yes | 47 |
| chalk | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 46 |
| chalk | refined | exact | DirectReference | Resolved | SetupAware | yes | 44 |
| chalk | refined | conservative | PointsTo | Resolved | Conservative | yes | 18 |
| chalk | refined | exact | ImportBinding | Resolved | SetupAware | yes | 2 |
| chalk | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 1 |
| chalk | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 1 |
| chalk | typed | none | PointsTo | Unresolved | Unknown | no | 80 |
| chalk | typed | exact | PointsTo | Resolved | Exact | yes | 47 |
| chalk | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 46 |
| chalk | typed | exact | DirectReference | Resolved | SetupAware | yes | 44 |
| chalk | typed | none | TypeHierarchy | Unresolved | Unknown | no | 24 |
| chalk | typed | conservative | PointsTo | Resolved | Conservative | yes | 18 |
| chalk | typed | exact | ImportBinding | Resolved | SetupAware | yes | 2 |
| chalk | typed | apart | SummaryAssisted | Resolved | Heuristic | yes | 1 |
| chalk | typed | apart | SyntaxOnly | Resolved | Heuristic | yes | 1 |
| dayjs | refined | none | PointsTo | Unresolved | Unknown | no | 184 |
| dayjs | refined | conservative | PointsTo | Resolved | Conservative | yes | 54 |
| dayjs | refined | exact | PointsTo | Resolved | Exact | yes | 41 |
| dayjs | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 34 |
| dayjs | refined | exact | DirectReference | Resolved | SetupAware | yes | 23 |
| dayjs | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 11 |
| dayjs | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 4 |
| dayjs | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 4 |
| dayjs | typed | none | PointsTo | Unresolved | Unknown | no | 184 |
| dayjs | typed | conservative | PointsTo | Resolved | Conservative | yes | 54 |
| dayjs | typed | none | TypeHierarchy | Unresolved | Unknown | no | 44 |
| dayjs | typed | exact | PointsTo | Resolved | Exact | yes | 41 |
| dayjs | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 34 |
| dayjs | typed | exact | DirectReference | Resolved | SetupAware | yes | 23 |
| dayjs | typed | exact | ConstructorBinding | Resolved | SetupAware | yes | 11 |
| dayjs | typed | apart | SummaryAssisted | Resolved | Heuristic | yes | 4 |
| dayjs | typed | apart | SyntaxOnly | Resolved | Heuristic | yes | 4 |
| underscore | refined | conservative | PointsTo | Resolved | Conservative | yes | 653 |
| underscore | refined | exact | PointsTo | Resolved | Exact | yes | 246 |
| underscore | refined | none | PointsTo | Unresolved | Unknown | no | 225 |
| underscore | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 187 |
| underscore | refined | exact | DirectReference | Resolved | SetupAware | yes | 186 |
| underscore | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 4 |
| underscore | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 4 |
| underscore | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 1 |
| underscore | typed | conservative | PointsTo | Resolved | Conservative | yes | 653 |
| underscore | typed | exact | PointsTo | Resolved | Exact | yes | 246 |
| underscore | typed | none | PointsTo | Unresolved | Unknown | no | 225 |
| underscore | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 187 |
| underscore | typed | exact | DirectReference | Resolved | SetupAware | yes | 186 |
| underscore | typed | none | TypeHierarchy | Unresolved | Unknown | no | 30 |
| underscore | typed | apart | SummaryAssisted | Resolved | Heuristic | yes | 4 |
| underscore | typed | apart | SyntaxOnly | Resolved | Heuristic | yes | 4 |
| underscore | typed | exact | ConstructorBinding | Resolved | SetupAware | yes | 1 |

## Sites gained by the refined plan

Sites where a refined plan claims an exact or conservative target and the base plan claims none, by file.
**test path** marks a file in a test or benchmark folder or named `*.test.*` / `*.spec.*`; **executed** counts
the gained sites the run called a polint function from, the only ones recall can credit.

| package | plan | sites gained | in test paths | executed | top files |
|---|---|---|---|---|---|
| axios | refined | 0 | 0 | 0 | – |
| axios | typed | 52 | 0 | 34 | lib/core/Axios.js 15, lib/adapters/http.js 12, lib/core/AxiosHeaders.js 10 |
| zod | refined | 0 | 0 | 0 | – |
| zod | typed | 188 | 0 | 8 | src/v3/types.ts 107, src/v4/classic/from-json-schema.ts 64, src/v4/core/to-json-schema.ts 7 |
| commander | refined | 0 | 0 | 0 | – |
| commander | typed | 332 | 0 | 214 | lib/command.js 235, lib/help.js 89, lib/option.js 6 |
| async | refined | 0 | 0 | 0 | – |
| async | typed | 10 | 0 | 5 | internal/DoublyLinkedList.js 6, internal/Heap.js 3, internal/queue.js 1 |
| chalk | refined | 0 | 0 | 0 | – |
| chalk | typed | 0 | 0 | 0 | – |
| dayjs | refined | 0 | 0 | 0 | – |
| dayjs | typed | 0 | 0 | 0 | – |
| underscore | refined | 0 | 0 | 0 | – |
| underscore | typed | 0 | 0 | 0 | – |

## Analysed set: globs alone against shipped source

The analysed set of every package is its shipped source: the `analysed` globs without test and benchmark
paths. Reports written before this rule analysed what the globs alone select; each package listed here is
scored both ways once, so this report reads against them. Every other table uses the shipped set.

| package | set | files | source KB | plan | recall | exact / exact + conservative | wrong targets single / multi | wrong exact targets | call sites | dump s | dump peak MB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| zod | globs alone | 332 | 3088 | base | 1190/1308 (91%) | 674 / 763 | 0 / 40 | 0 | 49016 | 44.0 | 3473 |
| zod | globs alone | 332 | 3088 | refined | 1190/1308 (91%) | 674 / 763 | 0 / 40 | 0 | 49016 | 57.8 | 4873 |
| zod | globs alone | 332 | 3088 | typed | 1197/1308 (92%) | 674 / 771 | 0 / 40 | 0 | 49016 | 66.4 | 5264 |
| zod | shipped | 125 | 1354 | base | 1181/1308 (90%) | 674 / 763 | 0 / 40 | 0 | 6109 | 12.2 | 925 |
| zod | shipped | 125 | 1354 | refined | 1181/1308 (90%) | 674 / 763 | 0 / 40 | 0 | 6109 | 15.7 | 1381 |
| zod | shipped | 125 | 1354 | typed | 1189/1308 (91%) | 674 / 771 | 0 / 40 | 0 | 6109 | 18.6 | 1398 |

## Outside tools

Jelly at `62e60eb4fd5b991c5bf4cfc992f018984f560ea4`, run by `bench.py outside` in the bench sandbox and scored
from the committed outputs under `baseline/outside/` with the bench's site and callee matching.
Recall is over observed `call` and `callback` edges. Wrong targets are claimed targets at executed
sites that the run never called there, split by answers with one target and with several.

| package | polint base | polint refined | polint typed | jelly-static | jelly-approx |
|---|---|---|---|---|---|
| async | 99.1% | 99.1% | 99.1% | 55.7% | 55.7% |
| axios | 89.5% | 89.5% | 89.8% | 89.2% | 89.3% |
| chalk | 77.8% | 77.8% | 77.8% | 86.1% | 86.1% |
| commander | 57.6% | 57.6% | 96.2% | 96.5% | 96.5% |
| dayjs | 78.6% | 78.6% | 78.6% | 77.7% | 87.5% |
| underscore | 96.2% | 96.2% | 96.2% | 96.2% | 98.9% |
| zod | 90.3% | 90.3% | 90.9% | 91.6% | 92.0% |
| **total** | **88.5%** | **88.5%** | **92.4%** | **84.6%** | **85.3%** |

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

| package | format | files analysed / selected | source KB | census functions | entered | coverage | observed edges | recall base | recall refined | recall typed |
|---|---|---|---|---|---|---|---|---|---|---|
| co 4.6.0 | CommonJS, generator and promise control flow | 1 / 1 | 5 | 20 | 20 | 20/20 (100%) | 22 | 22/22 (100%) | 22/22 (100%) | 22/22 (100%) |
| pako 2.1.0 | minified UMD (Rollup and terser) | 1 / 1 | 47 | 99 | 94 | 94/99 (95%) | 257 | 255/257 (99%) | 255/257 (99%) | 255/257 (99%) |
| nanostores 1.5.5 | ESM source | 14 / 14 | 18 | 97 | 91 | 91/97 (94%) | 103 | 71/103 (69%) | 71/103 (69%) | 71/103 (69%) |
| express 5.3.0 | CommonJS framework with dependencies | 7 / 7 | 62 | 109 | 109 | 109/109 (100%) | 166 | 67/166 (40%) | 67/166 (40%) | 73/166 (44%) |
| immer 11.2.10 | TypeScript source | 17 / 17 | 96 | 181 | 142 | 142/181 (78%) | 295 | 253/295 (86%) | 253/295 (86%) | 260/295 (88%) |
| js-yaml 4.1.0 | CommonJS source, table-driven schema compilation | 25 / 25 | 110 | 146 | 143 | 143/146 (98%) | 338 | 276/338 (82%) | 276/338 (82%) | 276/338 (82%) |
| acorn 8.19.0 | bundled ESM output | 1 / 1 | 235 | 358 | 333 | 333/358 (93%) | 1141 | 1119/1141 (98%) | 1119/1141 (98%) | 1119/1141 (98%) |
| node-forge 1.4.3 | CommonJS, large | 49 / 49 | 972 | 960 | 649 | 649/960 (68%) | 3131 | 2762/3131 (88%) | 2762/3131 (88%) | 2762/3131 (88%) |

### Held-out gate

Recall per tier pooled over the tune set and over the unflagged held-out packages (acorn, co, express, immer, js-yaml, nanostores, node-forge, pako; flagged: none). The gate holds when held-out recall is at most 5 points below the tune set's in every tier,
and no held-out package, flagged or not, has a wrong exact target or an answer with other targets and no
unresolved marker.

| plan | tier | tune | held-out | points below tune | within |
|---|---|---|---|---|---|
| base | exact-complete | 42.5% | 32.3% | 10.2 | no |
| base | exact | 47.5% | 32.3% | 15.3 | no |
| base | exact + conservative | 67.9% | 83.5% | -15.7 | yes |
| refined | exact-complete | 42.5% | 32.3% | 10.2 | no |
| refined | exact | 47.5% | 32.3% | 15.3 | no |
| refined | exact + conservative | 67.9% | 83.5% | -15.7 | yes |
| typed | exact-complete | 42.5% | 32.3% | 10.2 | no |
| typed | exact | 47.5% | 32.3% | 15.3 | no |
| typed | exact + conservative | 75.6% | 84.3% | -8.8 | yes |

| plan | wrong exact targets | other targets, no unresolved marker |
|---|---|---|
| base | 0 | 0 |
| refined | 0 | 0 |
| typed | 0 | 0 |

### Claimed targets the run never called

A claimed target is resolved and not heuristic. It counts here when the run executed the site, the target is
in an instrumented file, and the run never called it from that site. On a **single-target** answer that is a
false precise edge: every execution of the site went elsewhere. On a **multi-target** answer it is an upper
bound only, since the run may not have taken the paths to the other targets. A site matched to a polint site
by its end byte can also inflate both counts.

| package | base: single / multi | refined: single / multi | typed: single / multi |
|---|---|---|---|
| co | 0 / 0 | 0 / 0 | 0 / 0 |
| pako | 0 / 0 | 0 / 0 | 0 / 0 |
| nanostores | 0 / 4 | 0 / 4 | 0 / 4 |
| express | 0 / 0 | 0 / 0 | 0 / 0 |
| immer | 0 / 3 | 0 / 3 | 0 / 7 |
| js-yaml | 0 / 0 | 0 / 0 | 0 / 0 |
| acorn | 0 / 0 | 0 / 0 | 0 / 0 |
| node-forge | 0 / 64 | 0 / 64 | 0 / 64 |
| **total** | **0 / 71** | **0 / 71** | **0 / 75** |

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
| co | base | 22 | 12/22 (55%) | 12/22 (55%) | 22/22 (100%) | 0/22 (0%) | 0 |
| co | refined | 22 | 12/22 (55%) | 12/22 (55%) | 22/22 (100%) | 0/22 (0%) | 0 |
| co | typed | 22 | 12/22 (55%) | 12/22 (55%) | 22/22 (100%) | 0/22 (0%) | 0 |
| pako | base | 257 | 235/257 (91%) | 235/257 (91%) | 255/257 (99%) | 0/257 (0%) | 0 |
| pako | refined | 257 | 235/257 (91%) | 235/257 (91%) | 255/257 (99%) | 0/257 (0%) | 22 |
| pako | typed | 257 | 235/257 (91%) | 235/257 (91%) | 255/257 (99%) | 0/257 (0%) | 22 |
| nanostores | base | 103 | 32/103 (31%) | 32/103 (31%) | 71/103 (69%) | 0/103 (0%) | 0 |
| nanostores | refined | 103 | 32/103 (31%) | 32/103 (31%) | 71/103 (69%) | 0/103 (0%) | 0 |
| nanostores | typed | 103 | 32/103 (31%) | 32/103 (31%) | 71/103 (69%) | 0/103 (0%) | 0 |
| express | base | 166 | 33/166 (20%) | 33/166 (20%) | 43/166 (26%) | 24/166 (14%) | 25 |
| express | refined | 166 | 33/166 (20%) | 33/166 (20%) | 43/166 (26%) | 24/166 (14%) | 25 |
| express | typed | 166 | 33/166 (20%) | 33/166 (20%) | 58/166 (35%) | 15/166 (9%) | 25 |
| immer | base | 295 | 226/295 (77%) | 226/295 (77%) | 253/295 (86%) | 0/295 (0%) | 1 |
| immer | refined | 295 | 226/295 (77%) | 226/295 (77%) | 253/295 (86%) | 0/295 (0%) | 2 |
| immer | typed | 295 | 226/295 (77%) | 226/295 (77%) | 260/295 (88%) | 0/295 (0%) | 2 |
| js-yaml | base | 338 | 265/338 (78%) | 265/338 (78%) | 276/338 (82%) | 0/338 (0%) | 0 |
| js-yaml | refined | 338 | 265/338 (78%) | 265/338 (78%) | 276/338 (82%) | 0/338 (0%) | 4 |
| js-yaml | typed | 338 | 265/338 (78%) | 265/338 (78%) | 276/338 (82%) | 0/338 (0%) | 4 |
| acorn | base | 1141 | 179/1141 (16%) | 179/1141 (16%) | 1113/1141 (98%) | 6/1141 (1%) | 9 |
| acorn | refined | 1141 | 179/1141 (16%) | 179/1141 (16%) | 1113/1141 (98%) | 6/1141 (1%) | 12 |
| acorn | typed | 1141 | 179/1141 (16%) | 179/1141 (16%) | 1119/1141 (98%) | 0/1141 (0%) | 12 |
| node-forge | base | 3131 | 777/3131 (25%) | 777/3131 (25%) | 2523/3131 (81%) | 239/3131 (8%) | 389 |
| node-forge | refined | 3131 | 777/3131 (25%) | 777/3131 (25%) | 2523/3131 (81%) | 239/3131 (8%) | 407 |
| node-forge | typed | 3131 | 777/3131 (25%) | 777/3131 (25%) | 2538/3131 (81%) | 224/3131 (7%) | 407 |
| **total** | base | 5453 | 1759/5453 (32%) | 1759/5453 (32%) | 4556/5453 (84%) | 269/5453 (5%) | 424 |
| **total** | refined | 5453 | 1759/5453 (32%) | 1759/5453 (32%) | 4556/5453 (84%) | 269/5453 (5%) | 472 |
| **total** | typed | 5453 | 1759/5453 (32%) | 1759/5453 (32%) | 4599/5453 (84%) | 239/5453 (4%) | 472 |

Targets per site that claims any in the tier: mean / p95 (nearest rank) / max.

| package | plan | exact | conservative | exact + conservative | apart |
|---|---|---|---|---|---|
| co | base | 1.0 / 1 / 1 (12 sites) | 1.08 / 2 / 2 (13 sites) | 1.04 / 1 / 2 (25 sites) | – |
| co | refined | 1.0 / 1 / 1 (12 sites) | 1.08 / 2 / 2 (13 sites) | 1.04 / 1 / 2 (25 sites) | – |
| co | typed | 1.0 / 1 / 1 (12 sites) | 1.08 / 2 / 2 (13 sites) | 1.04 / 1 / 2 (25 sites) | – |
| pako | base | 1.0 / 1 / 1 (276 sites) | 1.0 / 1 / 1 (25 sites) | 1.0 / 1 / 1 (301 sites) | – |
| pako | refined | 1.0 / 1 / 1 (276 sites) | 1.0 / 1 / 1 (25 sites) | 1.0 / 1 / 1 (301 sites) | 1.0 / 1 / 1 (11 sites) |
| pako | typed | 1.0 / 1 / 1 (276 sites) | 1.0 / 1 / 1 (25 sites) | 1.0 / 1 / 1 (301 sites) | 1.0 / 1 / 1 (11 sites) |
| nanostores | base | 1.0 / 1 / 1 (37 sites) | 1.32 / 2 / 5 (38 sites) | 1.16 / 2 / 5 (75 sites) | – |
| nanostores | refined | 1.0 / 1 / 1 (37 sites) | 1.32 / 2 / 5 (38 sites) | 1.16 / 2 / 5 (75 sites) | – |
| nanostores | typed | 1.0 / 1 / 1 (37 sites) | 1.32 / 2 / 5 (38 sites) | 1.16 / 2 / 5 (75 sites) | – |
| express | base | 1.0 / 1 / 1 (35 sites) | 1.0 / 1 / 1 (13 sites) | 1.0 / 1 / 1 (48 sites) | 1.0 / 1 / 1 (25 sites) |
| express | refined | 1.0 / 1 / 1 (35 sites) | 1.0 / 1 / 1 (13 sites) | 1.0 / 1 / 1 (48 sites) | 1.0 / 1 / 1 (25 sites) |
| express | typed | 1.0 / 1 / 1 (35 sites) | 1.0 / 1 / 1 (28 sites) | 1.0 / 1 / 1 (63 sites) | 1.0 / 1 / 1 (25 sites) |
| immer | base | 1.0 / 1 / 1 (303 sites) | 1.48 / 6 / 6 (27 sites) | 1.04 / 1 / 6 (330 sites) | 1.0 / 1 / 1 (1 sites) |
| immer | refined | 1.0 / 1 / 1 (303 sites) | 1.48 / 6 / 6 (27 sites) | 1.04 / 1 / 6 (330 sites) | 1.0 / 1 / 1 (1 sites) |
| immer | typed | 1.0 / 1 / 1 (303 sites) | 1.17 / 2 / 6 (98 sites) | 1.04 / 1 / 6 (401 sites) | 1.0 / 1 / 1 (1 sites) |
| js-yaml | base | 1.0 / 1 / 1 (305 sites) | 1.0 / 1 / 1 (11 sites) | 1.0 / 1 / 1 (316 sites) | – |
| js-yaml | refined | 1.0 / 1 / 1 (305 sites) | 1.0 / 1 / 1 (11 sites) | 1.0 / 1 / 1 (316 sites) | 1.0 / 1 / 1 (2 sites) |
| js-yaml | typed | 1.0 / 1 / 1 (305 sites) | 1.0 / 1 / 1 (11 sites) | 1.0 / 1 / 1 (316 sites) | 1.0 / 1 / 1 (2 sites) |
| acorn | base | 1.0 / 1 / 1 (200 sites) | 1.01 / 1 / 8 (1308 sites) | 1.01 / 1 / 8 (1508 sites) | 1.0 / 1 / 1 (9 sites) |
| acorn | refined | 1.0 / 1 / 1 (200 sites) | 1.01 / 1 / 8 (1308 sites) | 1.01 / 1 / 8 (1508 sites) | 1.0 / 1 / 1 (9 sites) |
| acorn | typed | 1.0 / 1 / 1 (200 sites) | 1.01 / 1 / 8 (1314 sites) | 1.01 / 1 / 8 (1514 sites) | 1.0 / 1 / 1 (9 sites) |
| node-forge | base | 1.0 / 1 / 1 (941 sites) | 1.11 / 2 / 6 (2086 sites) | 1.08 / 1 / 6 (3027 sites) | 1.22 / 3 / 6 (319 sites) |
| node-forge | refined | 1.0 / 1 / 1 (941 sites) | 1.11 / 2 / 6 (2086 sites) | 1.08 / 1 / 6 (3027 sites) | 1.21 / 3 / 6 (328 sites) |
| node-forge | typed | 1.0 / 1 / 1 (941 sites) | 1.11 / 2 / 6 (2114 sites) | 1.08 / 1 / 6 (3055 sites) | 1.21 / 3 / 6 (328 sites) |

## Precision by tier (held-out)

At executed sites only: a site counts once the run was seen calling a polint function from it, and a
claimed target counts when it is a function in an instrumented file. **precision** is the share of those
targets the run called there; it is a lower bound, since a run takes only some paths. A target the run never
called on a single-target answer in the exact tier is a **wrong exact target**; the gate is 0.

| package | plan | exact: precision, single / multi not called | conservative: precision, single / multi not called | apart: precision, single / multi not called |
|---|---|---|---|---|
| co | base | 12/12 (100%), 0 / 0 | 10/10 (100%), 0 / 0 | – |
| co | refined | 12/12 (100%), 0 / 0 | 10/10 (100%), 0 / 0 | – |
| co | typed | 12/12 (100%), 0 / 0 | 10/10 (100%), 0 / 0 | – |
| pako | base | 235/235 (100%), 0 / 0 | 20/20 (100%), 0 / 0 | – |
| pako | refined | 235/235 (100%), 0 / 0 | 20/20 (100%), 0 / 0 | 8/8 (100%), 0 / 0 |
| pako | typed | 235/235 (100%), 0 / 0 | 20/20 (100%), 0 / 0 | 8/8 (100%), 0 / 0 |
| nanostores | base | 32/32 (100%), 0 / 0 | 39/43 (91%), 0 / 4 | – |
| nanostores | refined | 32/32 (100%), 0 / 0 | 39/43 (91%), 0 / 4 | – |
| nanostores | typed | 32/32 (100%), 0 / 0 | 39/43 (91%), 0 / 4 | – |
| express | base | 33/33 (100%), 0 / 0 | 10/10 (100%), 0 / 0 | 24/24 (100%), 0 / 0 |
| express | refined | 33/33 (100%), 0 / 0 | 10/10 (100%), 0 / 0 | 24/24 (100%), 0 / 0 |
| express | typed | 33/33 (100%), 0 / 0 | 25/25 (100%), 0 / 0 | 24/24 (100%), 0 / 0 |
| immer | base | 226/226 (100%), 0 / 0 | 27/30 (90%), 0 / 3 | – |
| immer | refined | 226/226 (100%), 0 / 0 | 27/30 (90%), 0 / 3 | – |
| immer | typed | 226/226 (100%), 0 / 0 | 34/41 (83%), 0 / 7 | – |
| js-yaml | base | 265/265 (100%), 0 / 0 | 11/11 (100%), 0 / 0 | – |
| js-yaml | refined | 265/265 (100%), 0 / 0 | 11/11 (100%), 0 / 0 | – |
| js-yaml | typed | 265/265 (100%), 0 / 0 | 11/11 (100%), 0 / 0 | – |
| acorn | base | 179/179 (100%), 0 / 0 | 934/934 (100%), 0 / 0 | 6/9 (67%), 3 / 0 |
| acorn | refined | 179/179 (100%), 0 / 0 | 934/934 (100%), 0 / 0 | 6/9 (67%), 3 / 0 |
| acorn | typed | 179/179 (100%), 0 / 0 | 940/940 (100%), 0 / 0 | 6/9 (67%), 3 / 0 |
| node-forge | base | 777/777 (100%), 0 / 0 | 1746/1810 (96%), 0 / 64 | 239/275 (87%), 0 / 36 |
| node-forge | refined | 777/777 (100%), 0 / 0 | 1746/1810 (96%), 0 / 64 | 246/282 (87%), 0 / 36 |
| node-forge | typed | 777/777 (100%), 0 / 0 | 1761/1825 (96%), 0 / 64 | 246/282 (87%), 0 / 36 |
| **total** | base | **1759/1759 (100%), 0 / 0** | **2797/2868 (98%), 0 / 71** | **269/308 (87%), 3 / 36** |
| **total** | refined | **1759/1759 (100%), 0 / 0** | **2797/2868 (98%), 0 / 71** | **284/323 (88%), 3 / 36** |
| **total** | typed | **1759/1759 (100%), 0 / 0** | **2840/2915 (97%), 0 / 75** | **284/323 (88%), 3 / 36** |

| package | plan | wrong exact target at | site text | target | the run called |
|---|---|---|---|---|---|
| none | | | | | |

## Open sites (held-out)

A site is open when it has no exact or conservative target, or the answer marks it unresolved. Each open
site counts under every reason its answer gives; `none recorded` is an open site whose answer gives none,
which the engine must not produce.

| package | plan | call sites | complete | open | open with a claimed target | open by reason |
|---|---|---|---|---|---|---|
| co | base | 46 | 12/46 (26%) | 34 | 13 | Unknown 13, CallApplyBind 12, MissingSemanticReference 11, DynamicProperty 7, none recorded 4 |
| co | refined | 46 | 12/46 (26%) | 34 | 13 | Unknown 13, CallApplyBind 12, MissingSemanticReference 11, DynamicProperty 7, none recorded 4 |
| co | typed | 46 | 12/46 (26%) | 34 | 13 | Unknown 13, CallApplyBind 12, MissingSemanticReference 11, DynamicProperty 7, UnknownCallee 5, none recorded 4 |
| pako | base | 430 | 276/430 (64%) | 154 | 25 | DynamicProperty 73, MissingSemanticReference 64, Inflow(EscapedObject) 12, Inflow(InwardThis) 9, CallApplyBind 8, Inflow(InwardParameter) 4, none recorded 4 |
| pako | refined | 430 | 276/430 (64%) | 154 | 25 | DynamicProperty 73, MissingSemanticReference 64, Inflow(EscapedObject) 12, Inflow(InwardThis) 9, CallApplyBind 8, Inflow(InwardParameter) 4, none recorded 4 |
| pako | typed | 430 | 276/430 (64%) | 154 | 25 | DynamicProperty 73, MissingSemanticReference 64, UnknownCallee 47, Inflow(EscapedObject) 12, Inflow(InwardThis) 9, CallApplyBind 8, Inflow(InwardParameter) 4, none recorded 4 |
| nanostores | base | 164 | 37/164 (23%) | 127 | 38 | MissingSemanticReference 61, DynamicProperty 56, Inflow(EscapedObject) 15, Unknown 15, none recorded 9, Inflow(OutsideValue) 8 |
| nanostores | refined | 164 | 37/164 (23%) | 127 | 38 | MissingSemanticReference 61, DynamicProperty 56, Inflow(EscapedObject) 15, Unknown 15, none recorded 9, Inflow(OutsideValue) 8 |
| nanostores | typed | 164 | 37/164 (23%) | 127 | 38 | MissingSemanticReference 61, DynamicProperty 56, UnknownCallee 54, Inflow(EscapedObject) 15, Unknown 15, none recorded 9, Inflow(OutsideValue) 8 |
| express | base | 466 | 35/466 (8%) | 431 | 13 | DynamicProperty 192, MissingSemanticReference 175, none recorded 51, Unknown 13, CallApplyBind 11, UnknownCallee 1 |
| express | refined | 466 | 35/466 (8%) | 431 | 13 | DynamicProperty 192, MissingSemanticReference 175, none recorded 51, Unknown 13, CallApplyBind 11, UnknownCallee 1 |
| express | typed | 466 | 35/466 (8%) | 431 | 28 | UnknownCallee 273, DynamicProperty 192, MissingSemanticReference 175, none recorded 45, Unknown 13, CallApplyBind 11 |
| immer | base | 575 | 303/575 (53%) | 272 | 27 | MissingSemanticReference 111, DynamicProperty 110, none recorded 33, CallApplyBind 18, Unknown 15, Inflow(EscapedObject) 6, Inflow(InwardParameter) 4, Inflow(OutsideValue) 2 |
| immer | refined | 575 | 303/575 (53%) | 272 | 27 | MissingSemanticReference 111, DynamicProperty 110, none recorded 33, CallApplyBind 18, Unknown 15, Inflow(EscapedObject) 6, Inflow(InwardParameter) 4, Inflow(OutsideValue) 2 |
| immer | typed | 575 | 303/575 (53%) | 272 | 98 | MissingSemanticReference 111, DynamicProperty 110, UnknownCallee 32, none recorded 26, CallApplyBind 18, Unknown 15, Inflow(EscapedObject) 6, Inflow(InwardParameter) 4, Inflow(OutsideValue) 2 |
| js-yaml | base | 709 | 305/709 (43%) | 404 | 11 | DynamicProperty 193, MissingSemanticReference 127, none recorded 55, CallApplyBind 29, Unknown 8, Inflow(EscapedObject) 3 |
| js-yaml | refined | 709 | 305/709 (43%) | 404 | 11 | DynamicProperty 193, MissingSemanticReference 127, none recorded 55, CallApplyBind 29, Unknown 8, Inflow(EscapedObject) 3 |
| js-yaml | typed | 709 | 305/709 (43%) | 404 | 11 | UnknownCallee 237, DynamicProperty 193, MissingSemanticReference 127, none recorded 55, CallApplyBind 29, Unknown 8, Inflow(EscapedObject) 3 |
| acorn | base | 1809 | 201/1809 (11%) | 1608 | 1307 | DynamicProperty 1539, Inflow(InwardThis) 1099, Inflow(InwardParameter) 185, MissingSemanticReference 43, none recorded 19, Inflow(OutsideValue) 11, Inflow(EscapedObject) 8, CallApplyBind 6, Unknown 4, DynamicImport 1 |
| acorn | refined | 1809 | 201/1809 (11%) | 1608 | 1307 | DynamicProperty 1539, Inflow(InwardThis) 1099, Inflow(InwardParameter) 185, MissingSemanticReference 43, none recorded 19, Inflow(OutsideValue) 11, Inflow(EscapedObject) 8, CallApplyBind 6, Unknown 4, DynamicImport 1 |
| acorn | typed | 1809 | 201/1809 (11%) | 1608 | 1313 | DynamicProperty 1539, Inflow(InwardThis) 1099, Inflow(InwardParameter) 185, UnknownCallee 183, MissingSemanticReference 43, none recorded 19, Inflow(OutsideValue) 11, Inflow(EscapedObject) 8, CallApplyBind 6, Unknown 4, DynamicImport 1 |
| node-forge | base | 5283 | 947/5283 (18%) | 4336 | 2080 | DynamicProperty 3152, Inflow(EscapedObject) 1097, MissingSemanticReference 938, Inflow(InwardThis) 551, Inflow(InwardParameter) 411, none recorded 196, Unknown 19, CallApplyBind 15, Inflow(OutsideValue) 2 |
| node-forge | refined | 5283 | 947/5283 (18%) | 4336 | 2080 | DynamicProperty 3152, Inflow(EscapedObject) 1097, MissingSemanticReference 938, Inflow(InwardThis) 551, Inflow(InwardParameter) 411, none recorded 196, Unknown 19, CallApplyBind 15, Inflow(OutsideValue) 2 |
| node-forge | typed | 5283 | 947/5283 (18%) | 4336 | 2108 | DynamicProperty 3152, UnknownCallee 1290, Inflow(EscapedObject) 1097, MissingSemanticReference 938, Inflow(InwardThis) 551, Inflow(InwardParameter) 411, none recorded 191, Unknown 19, CallApplyBind 15, Inflow(OutsideValue) 2 |

## Budget events (held-out)

The solver's budget status for each dump and every budget it reported as reached. The target is none on
the tune set, and a budget event must never discard an edge already derived.

| package | plan | status | reasons |
|---|---|---|---|
| co | base | WithinBudget | – |
| co | refined | WithinBudget | – |
| co | typed | WithinBudget | – |
| pako | base | WithinBudget | – |
| pako | refined | WithinBudget | – |
| pako | typed | WithinBudget | – |
| nanostores | base | WithinBudget | – |
| nanostores | refined | WithinBudget | – |
| nanostores | typed | WithinBudget | – |
| express | base | WithinBudget | – |
| express | refined | WithinBudget | – |
| express | typed | WithinBudget | – |
| immer | base | WithinBudget | – |
| immer | refined | WithinBudget | – |
| immer | typed | WithinBudget | – |
| js-yaml | base | WithinBudget | – |
| js-yaml | refined | WithinBudget | – |
| js-yaml | typed | WithinBudget | – |
| acorn | base | WithinBudget | – |
| acorn | refined | WithinBudget | – |
| acorn | typed | WithinBudget | – |
| node-forge | base | WithinBudget | – |
| node-forge | refined | WithinBudget | – |
| node-forge | typed | WithinBudget | – |

## Refined edges (held-out)

Every refined row by the tier the bench gives it, its algorithm, status and precision, and whether it
names a function. Rows with a status other than `Resolved` or `Ambiguous` claim nothing.

| package | plan | tier | algorithm | status | precision | names a function | rows |
|---|---|---|---|---|---|---|---|
| co | refined | none | PointsTo | Unresolved | Unknown | no | 43 |
| co | refined | conservative | PointsTo | Resolved | Conservative | yes | 14 |
| co | refined | exact | DirectReference | Resolved | SetupAware | yes | 12 |
| co | refined | exact | PointsTo | Resolved | Exact | yes | 12 |
| co | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 12 |
| co | typed | none | PointsTo | Unresolved | Unknown | no | 43 |
| co | typed | conservative | PointsTo | Resolved | Conservative | yes | 14 |
| co | typed | exact | DirectReference | Resolved | SetupAware | yes | 12 |
| co | typed | exact | PointsTo | Resolved | Exact | yes | 12 |
| co | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 12 |
| co | typed | none | TypeHierarchy | Unresolved | Unknown | no | 5 |
| pako | refined | exact | PointsTo | Resolved | Exact | yes | 276 |
| pako | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 249 |
| pako | refined | none | PointsTo | Unresolved | Unknown | no | 170 |
| pako | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 148 |
| pako | refined | exact | DirectReference | Resolved | SetupAware | yes | 101 |
| pako | refined | conservative | PointsTo | Resolved | Conservative | yes | 25 |
| pako | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 11 |
| pako | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 11 |
| pako | typed | exact | PointsTo | Resolved | Exact | yes | 276 |
| pako | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 249 |
| pako | typed | none | PointsTo | Unresolved | Unknown | no | 170 |
| pako | typed | exact | ConstructorBinding | Resolved | SetupAware | yes | 148 |
| pako | typed | exact | DirectReference | Resolved | SetupAware | yes | 101 |
| pako | typed | none | TypeHierarchy | Unresolved | Unknown | no | 47 |
| pako | typed | conservative | PointsTo | Resolved | Conservative | yes | 25 |
| pako | typed | apart | SummaryAssisted | Resolved | Heuristic | yes | 11 |
| pako | typed | apart | SyntaxOnly | Resolved | Heuristic | yes | 11 |
| nanostores | refined | none | PointsTo | Unresolved | Unknown | no | 155 |
| nanostores | refined | conservative | PointsTo | Resolved | Conservative | yes | 50 |
| nanostores | refined | exact | PointsTo | Resolved | Exact | yes | 37 |
| nanostores | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 36 |
| nanostores | refined | exact | DirectReference | Resolved | SetupAware | yes | 22 |
| nanostores | refined | exact | ImportBinding | Resolved | SetupAware | yes | 14 |
| nanostores | typed | none | PointsTo | Unresolved | Unknown | no | 155 |
| nanostores | typed | none | TypeHierarchy | Unresolved | Unknown | no | 54 |
| nanostores | typed | conservative | PointsTo | Resolved | Conservative | yes | 50 |
| nanostores | typed | exact | PointsTo | Resolved | Exact | yes | 37 |
| nanostores | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 36 |
| nanostores | typed | exact | DirectReference | Resolved | SetupAware | yes | 22 |
| nanostores | typed | exact | ImportBinding | Resolved | SetupAware | yes | 14 |
| express | refined | none | PointsTo | Unresolved | Unknown | no | 392 |
| express | refined | exact | PointsTo | Resolved | Exact | yes | 35 |
| express | refined | apart | PointsTo | Resolved | Heuristic | yes | 25 |
| express | refined | exact | DirectReference | Resolved | SetupAware | yes | 24 |
| express | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 24 |
| express | refined | conservative | PointsTo | Resolved | Conservative | yes | 13 |
| express | typed | none | PointsTo | Unresolved | Unknown | no | 392 |
| express | typed | none | TypeHierarchy | Unresolved | Unknown | no | 273 |
| express | typed | exact | PointsTo | Resolved | Exact | yes | 35 |
| express | typed | apart | PointsTo | Resolved | Heuristic | yes | 25 |
| express | typed | exact | DirectReference | Resolved | SetupAware | yes | 24 |
| express | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 24 |
| express | typed | conservative | TypeHierarchy | Resolved | Conservative | yes | 15 |
| express | typed | conservative | PointsTo | Resolved | Conservative | yes | 13 |
| immer | refined | exact | PointsTo | Resolved | Exact | yes | 303 |
| immer | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 302 |
| immer | refined | none | PointsTo | Unresolved | Unknown | no | 266 |
| immer | refined | exact | ImportBinding | Resolved | SetupAware | yes | 173 |
| immer | refined | exact | DirectReference | Resolved | SetupAware | yes | 127 |
| immer | refined | conservative | PointsTo | Resolved | Conservative | yes | 40 |
| immer | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 2 |
| immer | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 1 |
| immer | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 1 |
| immer | typed | exact | PointsTo | Resolved | Exact | yes | 303 |
| immer | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 302 |
| immer | typed | none | PointsTo | Unresolved | Unknown | no | 266 |
| immer | typed | exact | ImportBinding | Resolved | SetupAware | yes | 173 |
| immer | typed | exact | DirectReference | Resolved | SetupAware | yes | 127 |
| immer | typed | conservative | TypeHierarchy | Resolved | Conservative | yes | 75 |
| immer | typed | conservative | PointsTo | Resolved | Conservative | yes | 40 |
| immer | typed | none | TypeHierarchy | Unresolved | Unknown | no | 32 |
| immer | typed | exact | ConstructorBinding | Resolved | SetupAware | yes | 2 |
| immer | typed | apart | SummaryAssisted | Resolved | Heuristic | yes | 1 |
| immer | typed | apart | SyntaxOnly | Resolved | Heuristic | yes | 1 |
| js-yaml | refined | none | PointsTo | Unresolved | Unknown | no | 360 |
| js-yaml | refined | exact | PointsTo | Resolved | Exact | yes | 305 |
| js-yaml | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 254 |
| js-yaml | refined | exact | DirectReference | Resolved | SetupAware | yes | 252 |
| js-yaml | refined | conservative | PointsTo | Resolved | Conservative | yes | 11 |
| js-yaml | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 2 |
| js-yaml | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 2 |
| js-yaml | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 2 |
| js-yaml | typed | none | PointsTo | Unresolved | Unknown | no | 360 |
| js-yaml | typed | exact | PointsTo | Resolved | Exact | yes | 305 |
| js-yaml | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 254 |
| js-yaml | typed | exact | DirectReference | Resolved | SetupAware | yes | 252 |
| js-yaml | typed | none | TypeHierarchy | Unresolved | Unknown | no | 237 |
| js-yaml | typed | conservative | PointsTo | Resolved | Conservative | yes | 11 |
| js-yaml | typed | apart | SummaryAssisted | Resolved | Heuristic | yes | 2 |
| js-yaml | typed | apart | SyntaxOnly | Resolved | Heuristic | yes | 2 |
| js-yaml | typed | exact | ConstructorBinding | Resolved | SetupAware | yes | 2 |
| acorn | refined | none | PointsTo | Unresolved | Unknown | no | 2896 |
| acorn | refined | conservative | PointsTo | Resolved | Conservative | yes | 1317 |
| acorn | refined | exact | PointsTo | Resolved | Exact | yes | 200 |
| acorn | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 197 |
| acorn | refined | exact | DirectReference | Resolved | SetupAware | yes | 136 |
| acorn | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 61 |
| acorn | refined | apart | PointsTo | Resolved | Heuristic | yes | 6 |
| acorn | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 3 |
| acorn | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 3 |
| acorn | typed | none | PointsTo | Unresolved | Unknown | no | 2896 |
| acorn | typed | conservative | PointsTo | Resolved | Conservative | yes | 1317 |
| acorn | typed | exact | PointsTo | Resolved | Exact | yes | 200 |
| acorn | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 197 |
| acorn | typed | none | TypeHierarchy | Unresolved | Unknown | no | 183 |
| acorn | typed | exact | DirectReference | Resolved | SetupAware | yes | 136 |
| acorn | typed | exact | ConstructorBinding | Resolved | SetupAware | yes | 61 |
| acorn | typed | apart | PointsTo | Resolved | Heuristic | yes | 6 |
| acorn | typed | conservative | TypeHierarchy | Resolved | Conservative | yes | 6 |
| acorn | typed | apart | SummaryAssisted | Resolved | Heuristic | yes | 3 |
| acorn | typed | apart | SyntaxOnly | Resolved | Heuristic | yes | 3 |
| node-forge | refined | none | PointsTo | Unresolved | Unknown | no | 6185 |
| node-forge | refined | conservative | PointsTo | Resolved | Conservative | yes | 2319 |
| node-forge | refined | exact | PointsTo | Resolved | Exact | yes | 941 |
| node-forge | refined | exact | SummaryAssisted | Resolved | SetupAware | yes | 664 |
| node-forge | refined | exact | DirectReference | Resolved | SetupAware | yes | 610 |
| node-forge | refined | apart | PointsTo | Resolved | Heuristic | yes | 389 |
| node-forge | refined | exact | ConstructorBinding | Resolved | SetupAware | yes | 54 |
| node-forge | refined | apart | SummaryAssisted | Resolved | Heuristic | yes | 9 |
| node-forge | refined | apart | SyntaxOnly | Resolved | Heuristic | yes | 9 |
| node-forge | typed | none | PointsTo | Unresolved | Unknown | no | 6185 |
| node-forge | typed | conservative | PointsTo | Resolved | Conservative | yes | 2319 |
| node-forge | typed | none | TypeHierarchy | Unresolved | Unknown | no | 1290 |
| node-forge | typed | exact | PointsTo | Resolved | Exact | yes | 941 |
| node-forge | typed | exact | SummaryAssisted | Resolved | SetupAware | yes | 664 |
| node-forge | typed | exact | DirectReference | Resolved | SetupAware | yes | 610 |
| node-forge | typed | apart | PointsTo | Resolved | Heuristic | yes | 389 |
| node-forge | typed | exact | ConstructorBinding | Resolved | SetupAware | yes | 54 |
| node-forge | typed | conservative | TypeHierarchy | Resolved | Conservative | yes | 28 |
| node-forge | typed | apart | SummaryAssisted | Resolved | Heuristic | yes | 9 |
| node-forge | typed | apart | SyntaxOnly | Resolved | Heuristic | yes | 9 |

## Sites gained by the refined plan (held-out)

Sites where a refined plan claims an exact or conservative target and the base plan claims none, by file.
**test path** marks a file in a test or benchmark folder or named `*.test.*` / `*.spec.*`; **executed** counts
the gained sites the run called a polint function from, the only ones recall can credit.

| package | plan | sites gained | in test paths | executed | top files |
|---|---|---|---|---|---|
| co | refined | 0 | 0 | 0 | – |
| co | typed | 0 | 0 | 0 | – |
| pako | refined | 0 | 0 | 0 | – |
| pako | typed | 0 | 0 | 0 | – |
| nanostores | refined | 0 | 0 | 0 | – |
| nanostores | typed | 0 | 0 | 0 | – |
| express | refined | 0 | 0 | 0 | – |
| express | typed | 15 | 0 | 15 | lib/application.js 13, lib/view.js 2 |
| immer | refined | 0 | 0 | 0 | – |
| immer | typed | 71 | 0 | 7 | src/plugins/mapset.ts 41, src/plugins/arrayMethods.ts 9, src/plugins/patches.ts 7 |
| js-yaml | refined | 0 | 0 | 0 | – |
| js-yaml | typed | 0 | 0 | 0 | – |
| acorn | refined | 0 | 0 | 0 | – |
| acorn | typed | 6 | 0 | 6 | dist/acorn.mjs 6 |
| node-forge | refined | 0 | 0 | 0 | – |
| node-forge | typed | 28 | 0 | 15 | lib/util.js 22, lib/jsbn.js 6 |

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
| case prototype-chain/escaped-subclass-instance | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case prototype/escaped-instance | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case same-name-member/cjs-static-method | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case same-name-member/wrapper-of-static-method | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case sequence-callee/compiled-interop | 0/3 | 0/3 (0%) | – | 3/3 | 3/3 (100%) | 3/3 (100%) | CommonJsExportsProperty/Unsupported 3 | 0 |
| case sequence-callee/local-sequence | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |
| case typed-join/closure-field-assigned | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case typed-join/external-base-emit | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case typed-join/external-base-super-emit | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case typed-join/factory-constructor-function | 0/1 | 0/2 (0%) | – | 0/1 | 0/2 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case typed-join/reassigned-call-initializer | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case typed-join/reassigned-function-initializer | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case typed-join/union-parameter-jsdoc | 0/1 | 0/1 (0%) | – | 0/1 | 0/1 (0%) | – | CommonJsModuleExports/Unsupported 1 | 0 |
| case unary-iife/umd-prologue | 0/1 | 0/2 (0%) | – | 0/1 | 0/2 (0%) | – | – | 0 |
| case unary-iife/unary-operators | 1/1 | 1/1 (100%) | 1/1 (100%) | 1/1 | 1/1 (100%) | 1/1 (100%) | Named/Resolved 1 | 0 |

## Cost

The only part of this report that changes between runs. Machine: Linux-6.8.0-147-generic-x86_64-with-glibc2.39, 32 CPUs, commit 0fd00910e253, load average 10.83 / 7.89 / 13.62 at the start and 6.26 / 7.70 / 10.83 at the end (1, 5, 15 minutes).

`polint check` (release) with the census rule pack, which asks for `Functions`, `CallGraph`, `Symbols`, `References`; median of 3 runs. `analysis` is the in-process cost record (`POLINT_GOLDEN_COST_PATH`), `process` is the whole CLI as `bench_matrix.measure` sees it. Cold has no `.polint/cache`; warm reruns over the cache the cold run left. Stored bytes are `.polint/cache` after a cold run, raw and gzip-compressed. Per MB means per 1,000,000 source bytes. Process peak RSS comes from `wait4`, which on Linux counts the launching Python process's resident size at fork, so no process figure here is below the launcher's peak of 30 MB; the analysis column has no such floor.

| tree | source KB | cold analysis ms (per MB) | cold process ms | warm analysis ms (per MB) | warm process ms | cold peak RSS MB (analysis / process) | warm peak RSS MB | stored bytes per MB (gzip) |
|---|---|---|---|---|---|---|---|---|
| constructs (all cases) | 41 | 378 (9286) | 491 | 280 (6878) | 398 | 73 / 73 | 74 / 74 | 103501351 (9812568) |
| axios | 282 | 1693 (6003) | 1827 | 1499 (5315) | 1643 | 234 / 234 | 228 / 228 | 75884599 (5415395) |
| zod | 1354 | 11840 (8743) | 12024 | 10219 (7546) | 10396 | 848 / 848 | 860 / 860 | 127822334 (10464689) |
| commander | 127 | 1386 (10949) | 1511 | 1325 (10467) | 1446 | 109 / 109 | 109 / 109 | 44296772 (3204199) |
| async | 321 | 3012 (9396) | 3145 | 2850 (8891) | 2991 | 309 / 309 | 312 / 312 | 44447690 (3482287) |
| chalk | 22 | 145 (6723) | 262 | 118 (5471) | 235 | 73 / 73 | 73 / 73 | 78773646 (5811805) |
| dayjs | 7 | 175 (24438) | 300 | 157 (21924) | 283 | 73 / 73 | 73 / 73 | 228136294 (13651585) |
| underscore | 69 | 420 (6094) | 536 | 373 (5412) | 487 | 77 / 77 | 79 / 79 | 91434297 (5881595) |

Fact dump (no cache, one tree per process; peak RSS is the process figure, with the same floor): the
base plan is what a `CallGraph` rule pays; the refined plan adds the capabilities that read the raw rows.

| tree | base ms | base peak RSS MB | refined ms | refined peak RSS MB | budget s |
|---|---|---|---|---|---|
| axios | 1775 | 268 | 2742 | 378 | none |
| zod | 12176 | 925 | 15732 | 1381 | 180 |
| commander | 1409 | 125 | 1807 | 180 | none |
| async | 4507 | 1005 | 11364 | 1799 | none |
| chalk | 153 | 53 | 237 | 66 | none |
| dayjs | 190 | 55 | 275 | 68 | none |
| underscore | 456 | 104 | 822 | 169 | none |

### Stage times

Every fact dump runs with `RUST_LOG=polint::kernel::stage=info,polint::points_to=info`; each provider's `stage done` lines are summed. Stage times are as noisy as the dump times above. `semantic_graph share` is the `polint.semantic_graph` stage's part of the refined plan's summed stage time. Peak MB is the process peak when the stage ended.

| tree | plan | stage total s | largest stages: s (peak MB) | semantic_graph share |
|---|---|---|---|---|
| axios | base | 1.6 | `semantic_graph` 0.4 (212), `solver` 0.3 (240), `type_value_alias` 0.3 (201) | n/a |
| axios | refined | 2.5 | `evidence` 0.5 (321), `semantic_graph` 0.4 (234), `symbol_graph` 0.3 (89) | 16% |
| axios | typed | 3.5 | `ts.types` 0.9 (136), `evidence` 0.5 (339), `semantic_graph` 0.4 (243) | n/a |
| zod | base | 11.4 | `semantic_graph` 4.5 (815), `symbol_graph` 2.0 (509), `solver` 1.7 (830) | n/a |
| zod | refined | 14.7 | `semantic_graph` 4.4 (868), `evidence` 2.1 (1156), `symbol_graph` 2.0 (509) | 30% |
| zod | typed | 17.5 | `semantic_graph` 4.5 (903), `ts.types` 2.6 (510), `evidence` 2.1 (1167) | n/a |
| commander | base | 1.3 | `semantic_graph` 0.9 (110), `type_value_alias` 0.1 (100), `solver` 0.1 (115) | n/a |
| commander | refined | 1.7 | `semantic_graph` 0.9 (147), `cfg` 0.1 (82), `abstract_domains` 0.1 (119) | 53% |
| commander | typed | 2.4 | `semantic_graph` 0.9 (149), `ts.types` 0.7 (85), `cfg` 0.1 (81) | n/a |
| async | base | 3.0 | `solver` 1.2 (304), `semantic_graph` 0.9 (201), `refined_calls` 0.3 (310) | n/a |
| async | refined | 9.3 | `evidence` 3.7 (1254), `data_flow` 2.2 (1254), `solver` 1.2 (354) | 9% |
| async | typed | 10.2 | `evidence` 3.7 (1269), `data_flow` 2.2 (1269), `solver` 1.2 (363) | n/a |
| chalk | base | 0.1 | `semantic_graph` 0.0 (46), `solver` 0.0 (50), `type_value_alias` 0.0 (43) | n/a |
| chalk | refined | 0.2 | `cfg` 0.0 (39), `semantic_graph` 0.0 (54), `evidence` 0.0 (60) | 14% |
| chalk | typed | 0.7 | `ts.types` 0.5 (42), `cfg` 0.0 (40), `semantic_graph` 0.0 (56) | n/a |
| dayjs | base | 0.2 | `semantic_graph` 0.1 (47), `symbol_graph` 0.0 (31), `type_value_alias` 0.0 (46) | n/a |
| dayjs | refined | 0.2 | `semantic_graph` 0.1 (53), `cfg` 0.0 (39), `symbol_graph` 0.0 (32) | 27% |
| dayjs | typed | 0.7 | `ts.types` 0.5 (41), `semantic_graph` 0.1 (55), `cfg` 0.0 (39) | n/a |
| underscore | base | 0.4 | `symbol_graph` 0.1 (44), `semantic_graph` 0.1 (81), `type_value_alias` 0.1 (81) | n/a |
| underscore | refined | 0.7 | `cfg` 0.1 (75), `evidence` 0.1 (145), `symbol_graph` 0.1 (45) | 9% |
| underscore | typed | 1.3 | `ts.types` 0.5 (76), `cfg` 0.1 (74), `evidence` 0.1 (147) | n/a |
| co | base | 0.0 | `semantic_graph` 0.0 (35), `solver` 0.0 (36), `type_value_alias` 0.0 (33) | n/a |
| co | refined | 0.0 | `cfg` 0.0 (31), `semantic_graph` 0.0 (37), `evidence` 0.0 (40) | 15% |
| co | typed | 0.5 | `ts.types` 0.4 (32), `cfg` 0.0 (31), `semantic_graph` 0.0 (37) | n/a |
| pako | base | 4.0 | `semantic_graph` 2.9 (184), `symbol_graph` 0.5 (61), `type_value_alias` 0.3 (184) | n/a |
| pako | refined | 4.6 | `semantic_graph` 2.9 (229), `symbol_graph` 0.5 (61), `type_value_alias` 0.3 (229) | 63% |
| pako | typed | 5.2 | `semantic_graph` 2.9 (235), `ts.types` 0.6 (77), `symbol_graph` 0.5 (60) | n/a |
| nanostores | base | 0.1 | `symbol_graph` 0.0 (32), `semantic_graph` 0.0 (49), `type_value_alias` 0.0 (47) | n/a |
| nanostores | refined | 0.2 | `cfg` 0.0 (40), `semantic_graph` 0.0 (55), `evidence` 0.0 (64) | 14% |
| nanostores | typed | 0.7 | `ts.types` 0.5 (42), `semantic_graph` 0.0 (56), `cfg` 0.0 (41) | n/a |
| express | base | 0.3 | `semantic_graph` 0.1 (68), `type_value_alias` 0.1 (67), `solver` 0.0 (73) | n/a |
| express | refined | 0.5 | `cfg` 0.1 (55), `abstract_domains` 0.1 (75), `type_value_alias` 0.1 (92) | 12% |
| express | typed | 1.0 | `ts.types` 0.5 (56), `cfg` 0.1 (54), `abstract_domains` 0.1 (76) | n/a |
| immer | base | 0.4 | `symbol_graph` 0.1 (54), `semantic_graph` 0.1 (91), `type_value_alias` 0.1 (91) | n/a |
| immer | refined | 0.7 | `symbol_graph` 0.1 (55), `cfg` 0.1 (71), `evidence` 0.1 (140) | 11% |
| immer | typed | 1.4 | `ts.types` 0.7 (74), `symbol_graph` 0.1 (55), `cfg` 0.1 (73) | n/a |
| js-yaml | base | 0.6 | `type_value_alias` 0.1 (126), `symbol_graph` 0.1 (53), `semantic_graph` 0.1 (126) | n/a |
| js-yaml | refined | 1.0 | `abstract_domains` 0.2 (127), `evidence` 0.1 (165), `type_value_alias` 0.1 (165) | 11% |
| js-yaml | typed | 1.6 | `ts.types` 0.6 (74), `abstract_domains` 0.2 (130), `evidence` 0.2 (169) | n/a |
| acorn | base | 1.8 | `semantic_graph` 0.5 (230), `type_value_alias` 0.4 (230), `semantic_mir` 0.3 (76) | n/a |
| acorn | refined | 2.7 | `semantic_graph` 0.5 (264), `evidence` 0.5 (340), `type_value_alias` 0.4 (264) | 19% |
| acorn | typed | 3.4 | `ts.types` 0.7 (116), `semantic_graph` 0.5 (258), `evidence` 0.5 (334) | n/a |
| node-forge | base | 5.4 | `type_value_alias` 1.2 (504), `semantic_graph` 1.2 (509), `solver` 1.1 (622) | n/a |
| node-forge | refined | 7.4 | `semantic_graph` 1.2 (564), `solver` 1.2 (653), `type_value_alias` 1.1 (561) | 16% |
| node-forge | typed | 8.9 | `semantic_graph` 1.2 (576), `ts.types` 1.2 (288), `type_value_alias` 1.2 (575) | n/a |
| zod, globs alone | base | 39.7 | `semantic_graph` 14.1 (2670), `solver` 7.4 (2952), `type_value_alias` 6.3 (2160) | n/a |
| zod, globs alone | refined | 52.5 | `semantic_graph` 14.2 (2874), `solver` 7.5 (3140), `evidence` 6.5 (3721) | 27% |
| zod, globs alone | typed | 59.7 | `semantic_graph` 14.1 (2918), `solver` 7.4 (3188), `evidence` 7.4 (3896) | n/a |

## Solver

Every fact dump logs one `points_to solve` line per solve under `polint::points_to`. `alias` solves the constraints alone for the type, value and alias facts; `call graph` solves them with the call rule. Constraints are the input count (by kind in report.json); variables include heap slots; pops are worklist pops; collapsed are variables merged into another on a copy cycle. Time is per step: reading the input, the worklist, and building the result. Memory is the resident growth across the solve and the process peak after it. Counts are the same in every run; time and memory are as noisy as the stage times above.

| tree | plan | solve | constraints | calls | variables | objects | slots | copy edges | pops | collapsed | largest set | sets > 64 | sets > 1,024 | ms (input / worklist / result) | RSS growth MB | peak MB | budget |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| axios | base | alias | 7,676 | 0 | 7,273 | 2,735 | 227 | 1,247 | 2,728 | 0 | 5 | 0 | 0 | 3 / 0 / 4 | 3 | 127 | WithinBudget |
| axios | base | call graph | 40,548 | 3,804 | 26,188 | 2,229 | 3,797 | 35,467 | 12,624 | 124 | 137 | 3 | 0 | 20 / 6 / 41 | 14 | 240 | WithinBudget |
| axios | refined | alias | 7,676 | 0 | 7,273 | 2,735 | 227 | 1,247 | 2,728 | 0 | 5 | 0 | 0 | 2 / 0 / 4 | 0 | 168 | WithinBudget |
| axios | refined | call graph | 40,548 | 3,804 | 26,188 | 2,229 | 3,797 | 35,467 | 12,624 | 124 | 137 | 3 | 0 | 20 / 5 / 43 | 10 | 249 | WithinBudget |
| axios | typed | alias | 7,676 | 0 | 7,273 | 2,735 | 227 | 1,247 | 2,728 | 0 | 5 | 0 | 0 | 2 / 0 / 4 | 0 | 174 | WithinBudget |
| axios | typed | call graph | 40,548 | 3,804 | 26,188 | 2,229 | 3,797 | 35,467 | 12,624 | 124 | 137 | 3 | 0 | 21 / 6 / 49 | 10 | 265 | WithinBudget |
| zod | base | alias | 32,608 | 0 | 26,996 | 8,514 | 465 | 3,004 | 8,495 | 0 | 23 | 0 | 0 | 19 / 0 / 18 | 9 | 509 | WithinBudget |
| zod | base | call graph | 162,839 | 11,265 | 126,934 | 10,123 | 41,007 | 358,325 | 80,330 | 2,862 | 920 | 6,090 | 0 | 141 / 113 / 366 | 34 | 830 | WithinBudget |
| zod | refined | alias | 32,608 | 0 | 26,996 | 8,514 | 465 | 3,004 | 8,495 | 0 | 23 | 0 | 0 | 16 / 0 / 15 | 7 | 521 | WithinBudget |
| zod | refined | call graph | 162,839 | 11,265 | 126,934 | 10,123 | 41,007 | 358,325 | 80,330 | 2,862 | 920 | 6,090 | 0 | 117 / 115 / 347 | 20 | 902 | WithinBudget |
| zod | typed | alias | 32,608 | 0 | 26,996 | 8,514 | 465 | 3,004 | 8,495 | 0 | 23 | 0 | 0 | 16 / 0 / 15 | 9 | 535 | WithinBudget |
| zod | typed | call graph | 162,839 | 11,265 | 126,934 | 10,123 | 41,007 | 358,325 | 80,330 | 2,862 | 920 | 6,090 | 0 | 136 / 131 / 373 | 47 | 931 | WithinBudget |
| commander | base | alias | 4,037 | 0 | 3,462 | 1,073 | 100 | 514 | 1,062 | 0 | 3 | 0 | 0 | 1 / 0 / 1 | 0 | 66 | WithinBudget |
| commander | base | call graph | 17,626 | 1,665 | 10,255 | 828 | 1,308 | 14,549 | 3,390 | 53 | 309 | 2 | 0 | 4 / 0 / 10 | 3 | 114 | WithinBudget |
| commander | refined | alias | 4,037 | 0 | 3,462 | 1,073 | 100 | 514 | 1,062 | 0 | 3 | 0 | 0 | 1 / 0 / 1 | 0 | 120 | WithinBudget |
| commander | refined | call graph | 17,626 | 1,665 | 10,255 | 828 | 1,308 | 14,549 | 3,390 | 53 | 309 | 2 | 0 | 6 / 1 / 10 | 4 | 147 | WithinBudget |
| commander | typed | alias | 4,037 | 0 | 3,462 | 1,073 | 100 | 514 | 1,062 | 0 | 3 | 0 | 0 | 1 / 0 / 17 | 0 | 121 | WithinBudget |
| commander | typed | call graph | 17,626 | 1,665 | 10,255 | 828 | 1,308 | 14,549 | 3,390 | 53 | 309 | 2 | 0 | 6 / 0 / 9 | 6 | 149 | WithinBudget |
| async | base | alias | 6,082 | 0 | 6,602 | 1,857 | 33 | 1,444 | 1,863 | 0 | 3 | 0 | 0 | 2 / 0 / 25 | 7 | 100 | WithinBudget |
| async | base | call graph | 25,935 | 2,937 | 26,494 | 1,933 | 9,206 | 298,872 | 65,114 | 1,460 | 253 | 5,090 | 0 | 10 / 171 / 210 | 8 | 204 | WithinBudget |
| async | refined | alias | 6,082 | 0 | 6,602 | 1,857 | 33 | 1,444 | 1,863 | 0 | 3 | 0 | 0 | 1 / 0 / 2 | 0 | 258 | WithinBudget |
| async | refined | call graph | 25,935 | 2,937 | 26,494 | 1,933 | 9,206 | 298,872 | 65,114 | 1,460 | 253 | 5,090 | 0 | 8 / 173 / 197 | 4 | 262 | WithinBudget |
| async | typed | alias | 6,082 | 0 | 6,602 | 1,857 | 33 | 1,444 | 1,863 | 0 | 3 | 0 | 0 | 1 / 0 / 2 | 0 | 265 | WithinBudget |
| async | typed | call graph | 25,935 | 2,937 | 26,494 | 1,933 | 9,206 | 298,872 | 65,114 | 1,460 | 253 | 5,090 | 0 | 10 / 192 / 205 | 4 | 270 | WithinBudget |
| chalk | base | alias | 627 | 0 | 575 | 203 | 13 | 72 | 199 | 0 | 3 | 0 | 0 | 0 / 0 / 0 | 0 | 36 | WithinBudget |
| chalk | base | call graph | 4,467 | 249 | 2,864 | 374 | 773 | 2,972 | 1,420 | 0 | 90 | 1 | 0 | 1 / 0 / 8 | 2 | 50 | WithinBudget |
| chalk | refined | alias | 627 | 0 | 575 | 203 | 13 | 72 | 199 | 0 | 3 | 0 | 0 | 0 / 0 / 0 | 0 | 47 | WithinBudget |
| chalk | refined | call graph | 4,467 | 249 | 2,864 | 374 | 773 | 2,972 | 1,420 | 0 | 90 | 1 | 0 | 1 / 0 / 3 | 0 | 56 | WithinBudget |
| chalk | typed | alias | 627 | 0 | 575 | 203 | 13 | 72 | 199 | 0 | 3 | 0 | 0 | 0 / 0 / 0 | 0 | 49 | WithinBudget |
| chalk | typed | call graph | 4,467 | 249 | 2,864 | 374 | 773 | 2,972 | 1,420 | 0 | 90 | 1 | 0 | 1 / 0 / 3 | 0 | 57 | WithinBudget |
| dayjs | base | alias | 872 | 0 | 788 | 257 | 31 | 138 | 263 | 0 | 1 | 0 | 0 | 0 / 0 / 0 | 0 | 38 | WithinBudget |
| dayjs | base | call graph | 3,569 | 327 | 2,256 | 184 | 390 | 3,099 | 924 | 23 | 82 | 1 | 0 | 0 / 0 / 6 | 3 | 51 | WithinBudget |
| dayjs | refined | alias | 872 | 0 | 788 | 257 | 31 | 138 | 263 | 0 | 1 | 0 | 0 | 0 / 0 / 0 | 0 | 47 | WithinBudget |
| dayjs | refined | call graph | 3,569 | 327 | 2,256 | 184 | 390 | 3,099 | 924 | 23 | 82 | 1 | 0 | 0 / 0 / 2 | 1 | 53 | WithinBudget |
| dayjs | typed | alias | 872 | 0 | 788 | 257 | 31 | 138 | 263 | 0 | 1 | 0 | 0 | 0 / 0 / 0 | 0 | 49 | WithinBudget |
| dayjs | typed | call graph | 3,569 | 327 | 2,256 | 184 | 390 | 3,099 | 924 | 23 | 82 | 1 | 0 | 0 / 0 / 2 | 1 | 56 | WithinBudget |
| underscore | base | alias | 1,661 | 0 | 1,721 | 626 | 49 | 391 | 630 | 0 | 2 | 0 | 0 | 0 / 0 / 1 | 0 | 55 | WithinBudget |
| underscore | base | call graph | 9,178 | 689 | 5,750 | 419 | 836 | 10,262 | 3,319 | 177 | 227 | 1 | 0 | 3 / 1 / 8 | 3 | 85 | WithinBudget |
| underscore | refined | alias | 1,661 | 0 | 1,721 | 626 | 49 | 391 | 630 | 0 | 2 | 0 | 0 | 0 / 0 / 0 | 0 | 101 | WithinBudget |
| underscore | refined | call graph | 9,178 | 689 | 5,750 | 419 | 836 | 10,262 | 3,319 | 177 | 227 | 1 | 0 | 2 / 1 / 9 | 0 | 121 | WithinBudget |
| underscore | typed | alias | 1,661 | 0 | 1,721 | 626 | 49 | 391 | 630 | 0 | 2 | 0 | 0 | 0 / 0 / 0 | 0 | 101 | WithinBudget |
| underscore | typed | call graph | 9,178 | 689 | 5,750 | 419 | 836 | 10,262 | 3,319 | 177 | 227 | 1 | 0 | 2 / 1 / 8 | 1 | 121 | WithinBudget |
| co | base | alias | 174 | 0 | 172 | 60 | 0 | 14 | 60 | 0 | 1 | 0 | 0 | 0 / 0 / 0 | 0 | 32 | WithinBudget |
| co | base | call graph | 1,099 | 100 | 634 | 50 | 54 | 954 | 286 | 0 | 24 | 0 | 0 | 0 / 0 / 0 | 0 | 36 | WithinBudget |
| co | refined | alias | 174 | 0 | 172 | 60 | 0 | 14 | 60 | 0 | 1 | 0 | 0 | 0 / 0 / 0 | 0 | 34 | WithinBudget |
| co | refined | call graph | 1,099 | 100 | 634 | 50 | 54 | 954 | 286 | 0 | 24 | 0 | 0 | 0 / 0 / 0 | 0 | 38 | WithinBudget |
| co | typed | alias | 174 | 0 | 172 | 60 | 0 | 14 | 60 | 0 | 1 | 0 | 0 | 0 / 0 / 0 | 0 | 35 | WithinBudget |
| co | typed | call graph | 1,099 | 100 | 634 | 50 | 54 | 954 | 286 | 0 | 24 | 0 | 0 | 0 / 0 / 0 | 0 | 38 | WithinBudget |
| pako | base | alias | 3,618 | 0 | 2,668 | 688 | 21 | 578 | 642 | 1 | 14 | 0 | 0 | 1 / 0 / 1 | 0 | 91 | WithinBudget |
| pako | base | call graph | 16,113 | 575 | 8,670 | 443 | 1,179 | 12,260 | 2,947 | 5 | 136 | 1 | 0 | 6 / 2 / 13 | 6 | 184 | WithinBudget |
| pako | refined | alias | 3,618 | 0 | 2,668 | 688 | 21 | 578 | 642 | 1 | 14 | 0 | 0 | 0 / 0 / 1 | 0 | 154 | WithinBudget |
| pako | refined | call graph | 16,113 | 575 | 8,670 | 443 | 1,179 | 12,260 | 2,947 | 5 | 136 | 1 | 0 | 4 / 1 / 13 | 0 | 229 | WithinBudget |
| pako | typed | alias | 3,618 | 0 | 2,668 | 688 | 21 | 578 | 642 | 1 | 14 | 0 | 0 | 1 / 0 / 1 | 0 | 154 | WithinBudget |
| pako | typed | call graph | 16,113 | 575 | 8,670 | 443 | 1,179 | 12,260 | 2,947 | 5 | 136 | 1 | 0 | 4 / 1 / 10 | 0 | 235 | WithinBudget |
| nanostores | base | alias | 761 | 0 | 698 | 270 | 14 | 90 | 277 | 0 | 2 | 0 | 0 | 0 / 0 / 0 | 0 | 39 | WithinBudget |
| nanostores | base | call graph | 3,945 | 312 | 2,589 | 325 | 247 | 3,158 | 1,361 | 12 | 31 | 0 | 0 | 1 / 0 / 3 | 1 | 52 | WithinBudget |
| nanostores | refined | alias | 761 | 0 | 698 | 270 | 14 | 90 | 277 | 0 | 2 | 0 | 0 | 0 / 0 / 0 | 0 | 47 | WithinBudget |
| nanostores | refined | call graph | 3,945 | 312 | 2,589 | 325 | 247 | 3,158 | 1,361 | 12 | 31 | 0 | 0 | 1 / 0 / 3 | 1 | 57 | WithinBudget |
| nanostores | typed | alias | 761 | 0 | 698 | 270 | 14 | 90 | 277 | 0 | 2 | 0 | 0 | 0 / 0 / 0 | 0 | 49 | WithinBudget |
| nanostores | typed | call graph | 3,945 | 312 | 2,589 | 325 | 247 | 3,158 | 1,361 | 12 | 31 | 0 | 0 | 1 / 0 / 3 | 1 | 58 | WithinBudget |
| express | base | alias | 2,022 | 0 | 1,892 | 614 | 61 | 381 | 611 | 0 | 3 | 0 | 0 | 0 / 0 / 0 | 0 | 48 | WithinBudget |
| express | base | call graph | 8,042 | 931 | 4,789 | 390 | 344 | 6,027 | 1,312 | 1 | 72 | 1 | 0 | 2 / 0 / 3 | 2 | 73 | WithinBudget |
| express | refined | alias | 2,022 | 0 | 1,892 | 614 | 61 | 381 | 611 | 0 | 3 | 0 | 0 | 0 / 0 / 9 | 0 | 76 | WithinBudget |
| express | refined | call graph | 8,042 | 931 | 4,789 | 390 | 344 | 6,027 | 1,312 | 1 | 72 | 1 | 0 | 1 / 0 / 3 | 0 | 92 | WithinBudget |
| express | typed | alias | 2,022 | 0 | 1,892 | 614 | 61 | 381 | 611 | 0 | 3 | 0 | 0 | 0 / 0 / 0 | 0 | 77 | WithinBudget |
| express | typed | call graph | 8,042 | 931 | 4,789 | 390 | 344 | 6,027 | 1,312 | 1 | 72 | 1 | 0 | 2 / 0 / 3 | 0 | 93 | WithinBudget |
| immer | base | alias | 2,267 | 0 | 2,006 | 727 | 61 | 336 | 721 | 0 | 3 | 0 | 0 | 0 / 0 / 1 | 0 | 58 | WithinBudget |
| immer | base | call graph | 11,476 | 890 | 7,256 | 571 | 888 | 9,502 | 3,056 | 77 | 59 | 0 | 0 | 3 / 0 / 8 | 2 | 92 | WithinBudget |
| immer | refined | alias | 2,267 | 0 | 2,006 | 727 | 61 | 336 | 721 | 0 | 3 | 0 | 0 | 0 / 0 / 1 | 0 | 91 | WithinBudget |
| immer | refined | call graph | 11,476 | 890 | 7,256 | 571 | 888 | 9,502 | 3,056 | 77 | 59 | 0 | 0 | 3 / 0 / 10 | 0 | 122 | WithinBudget |
| immer | typed | alias | 2,267 | 0 | 2,006 | 727 | 61 | 336 | 721 | 0 | 3 | 0 | 0 | 0 / 0 / 1 | 0 | 93 | WithinBudget |
| immer | typed | call graph | 11,476 | 890 | 7,256 | 571 | 888 | 9,502 | 3,056 | 77 | 59 | 0 | 0 | 2 / 0 / 10 | 3 | 125 | WithinBudget |
| js-yaml | base | alias | 3,230 | 0 | 2,822 | 933 | 92 | 612 | 914 | 0 | 16 | 0 | 0 | 0 / 0 / 1 | 0 | 76 | WithinBudget |
| js-yaml | base | call graph | 14,203 | 1,178 | 9,201 | 593 | 1,199 | 12,211 | 3,267 | 39 | 41 | 0 | 0 | 3 / 2 / 13 | 2 | 126 | WithinBudget |
| js-yaml | refined | alias | 3,230 | 0 | 2,822 | 933 | 92 | 612 | 914 | 0 | 16 | 0 | 0 | 0 / 0 / 1 | 0 | 129 | WithinBudget |
| js-yaml | refined | call graph | 14,203 | 1,178 | 9,201 | 593 | 1,199 | 12,211 | 3,267 | 39 | 41 | 0 | 0 | 3 / 1 / 11 | 0 | 165 | WithinBudget |
| js-yaml | typed | alias | 3,230 | 0 | 2,822 | 933 | 92 | 612 | 914 | 0 | 16 | 0 | 0 | 0 / 0 / 1 | 0 | 132 | WithinBudget |
| js-yaml | typed | call graph | 14,203 | 1,178 | 9,201 | 593 | 1,199 | 12,211 | 3,267 | 39 | 41 | 0 | 0 | 4 / 1 / 10 | 0 | 169 | WithinBudget |
| acorn | base | alias | 10,718 | 0 | 8,202 | 2,263 | 111 | 1,191 | 2,264 | 2 | 3 | 0 | 0 | 3 / 0 / 3 | 0 | 124 | WithinBudget |
| acorn | base | call graph | 38,280 | 3,430 | 21,755 | 1,175 | 3,880 | 37,196 | 13,135 | 337 | 489 | 1 | 0 | 17 / 7 / 81 | 13 | 230 | WithinBudget |
| acorn | refined | alias | 10,718 | 0 | 8,202 | 2,263 | 111 | 1,191 | 2,264 | 2 | 3 | 0 | 0 | 3 / 0 / 3 | 0 | 166 | WithinBudget |
| acorn | refined | call graph | 38,280 | 3,430 | 21,755 | 1,175 | 3,880 | 37,196 | 13,135 | 337 | 489 | 1 | 0 | 26 / 11 / 95 | 13 | 264 | WithinBudget |
| acorn | typed | alias | 10,718 | 0 | 8,202 | 2,263 | 111 | 1,191 | 2,264 | 2 | 3 | 0 | 0 | 3 / 0 / 3 | 0 | 170 | WithinBudget |
| acorn | typed | call graph | 38,280 | 3,430 | 21,755 | 1,175 | 3,880 | 37,196 | 13,135 | 337 | 489 | 1 | 0 | 24 / 10 / 89 | 13 | 258 | WithinBudget |
| node-forge | base | alias | 34,331 | 0 | 26,963 | 7,229 | 1,371 | 5,976 | 7,194 | 4 | 16 | 0 | 0 | 15 / 1 / 19 | 9 | 308 | WithinBudget |
| node-forge | base | call graph | 121,348 | 9,159 | 81,081 | 5,385 | 20,326 | 169,797 | 43,245 | 514 | 412 | 1,715 | 0 | 83 / 50 / 399 | 59 | 622 | WithinBudget |
| node-forge | refined | alias | 34,331 | 0 | 26,963 | 7,229 | 1,371 | 5,976 | 7,194 | 4 | 16 | 0 | 0 | 21 / 1 / 17 | 10 | 362 | WithinBudget |
| node-forge | refined | call graph | 121,348 | 9,159 | 81,081 | 5,385 | 20,326 | 169,797 | 43,245 | 514 | 412 | 1,715 | 0 | 88 / 58 / 194 | 34 | 653 | WithinBudget |
| node-forge | typed | alias | 34,331 | 0 | 26,963 | 7,229 | 1,371 | 5,976 | 7,194 | 4 | 16 | 0 | 0 | 22 / 1 / 19 | 10 | 371 | WithinBudget |
| node-forge | typed | call graph | 121,348 | 9,159 | 81,081 | 5,385 | 20,326 | 169,797 | 43,245 | 514 | 412 | 1,715 | 0 | 82 / 55 / 213 | 21 | 657 | WithinBudget |
| zod, globs alone | base | alias | 157,403 | 0 | 139,918 | 60,145 | 2,430 | 10,863 | 60,170 | 1 | 23 | 0 | 0 | 141 / 4 / 169 | 56 | 1342 | WithinBudget |
| zod, globs alone | base | call graph | 693,639 | 96,148 | 469,440 | 43,112 | 95,800 | 1,057,363 | 237,788 | 3,701 | 1,417 | 14,092 | 2 | 670 / 561 / 1232 | 162 | 2952 | WithinBudget |
| zod, globs alone | refined | alias | 157,403 | 0 | 139,918 | 60,145 | 2,430 | 10,863 | 60,170 | 1 | 23 | 0 | 0 | 142 / 6 / 160 | 39 | 1542 | WithinBudget |
| zod, globs alone | refined | call graph | 693,639 | 96,148 | 469,440 | 43,112 | 95,800 | 1,057,363 | 237,788 | 3,701 | 1,417 | 14,092 | 2 | 683 / 534 / 1279 | 162 | 3140 | WithinBudget |
| zod, globs alone | typed | alias | 157,403 | 0 | 139,918 | 60,145 | 2,430 | 10,863 | 60,170 | 1 | 23 | 0 | 0 | 148 / 4 / 151 | 39 | 1619 | WithinBudget |
| zod, globs alone | typed | call graph | 693,639 | 96,148 | 469,440 | 43,112 | 95,800 | 1,057,363 | 237,788 | 3,701 | 1,417 | 14,092 | 2 | 665 / 567 / 1268 | 168 | 3188 | WithinBudget |

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
