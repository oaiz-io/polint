# Closed lists: tiers, assumptions, constructs

Date: 2026-11-17. Three lists the plan needs, derived from the code at
`7aa3b3cf`. Each is closed: adding an entry is a change to this document and
to the test that pins the list.

## (a) `CallPrecision` to the two tiers

Producers today (`crates/polint/src/`):

| `CallPrecision` | produced by | tier |
|---|---|---|
| `Exact` | Go static calls (`go/typed_calls.rs:64`); the typed tier's `Declared` dispatch to a function, method or constructor in the analysed tree (a class only where the call constructs it), at a site that is not a union, on a receiver named by one declaration (not an intersection or an object literal), in a file under 25% `any` receivers (`refined_calls/ts_types.rs`, `TypedAnswer::names_the_target`); direct summaries (`analysis/summaries/provider.rs:398,416`). `refined_calls/validate.rs:104-128` refuses it on `Model` provenance, on extension-validated edges, and on the `PointsTo`, `FrameworkModel` and `RepoModel` algorithms. | **exact** |
| `SetupAware` | the direct tier (`calls/direct.rs:116`: `DirectReference`, `ImportBinding`, `ConstructorBinding`, `StaticMember`, `DirectMember`), summary-assisted edges, solver edges from `FlowInsensitive`/`LocalFlowSensitive` sets (`refined_calls/provider.rs:664`) | **exact** |
| `Conservative` | every other typed edge: `Declared` in a degraded file, at a union site, on an intersection or object-literal receiver, or to an accessor, a class called without `new`, a declaration outside the tree or one whose kind the sidecar did not send; and every `DeclaredSignature`, `Implementation`, `UnionMember` and `Inferred` candidate (`ts_types.rs`, `precision_for`); framework and extension rows | conservative |
| `Ambiguous` | a reference that resolves to several symbols (`calls/extract.rs`, `unresolved.rs`) | conservative |
| `Heuristic` | the walker's rows (`ts/semantic_graph_build.rs:663`, surfacing as solver edges with `Heuristic` sets); the lexical fallback `SyntaxOnly` (`direct.rs:67`) | **apart**: reported as its own column until the producer is gone |
| `Unknown`, `Unsupported` | unresolved rows | no claim |

Note on `SetupAware`: it means "exact given the setup" only. The typed
tier no longer produces it: a typed answer is `Exact` when the checker named
the function the call runs, and `Conservative` otherwise, so precision alone
decides the tier. A typed declaration whose native function cannot be found
by span or by name gets no edge (never the function around it), and is
counted as an unmatched callee in the `ts type tier join` log line.

The site status is the second axis and is independent of precision: a site
is **complete** when it has at least one resolved target and no unresolved
row, else **open**. Recall is reported for exact-complete, exact (complete
or open), and exact plus conservative; precision (executed-site) per tier.

The list is pinned by a test in WP0 that matches every `CallPrecision`
variant to one of `exact | conservative | apart | none`.

## (b) Assumptions an exact claim may carry

From the resolvers that produce the exact tier today (`calls/direct.rs`,
the value-function rows of `ts/symbol_graph.rs:465-560`, `ts_types.rs`), and
from the assumptions the solver's exact tier will need once it exists (WP3).
Each assumption has a scope: *package-internal* assumptions cannot be
broken by stitching and are recorded for audit only; *boundary* assumptions
are re-checked at stitch and drop the claim to conservative when they fail.

| id | assumption | where it comes from today | scope |
|---|---|---|---|
| `sealed_bindings(file)` | the file has no reference named `eval` and no `with` statement, so no binding is written outside the references Oxc sees | `symbol_graph.rs:500-507` | package-internal |
| `single_literal(binding)` | the binding is `const`, or `let`/`var` declared once, never written, not overwritten by a block-level function declaration, and (at module top level) the file is a module, so the binding holds exactly its initialiser literal | `symbol_graph.rs:512-535`, `direct.rs:488-526` (`callees_by_binding`) | package-internal |
| `declaration_only(symbol)` | every declaration of the symbol is a function, interface or namespace and the symbol is never written; an overload set has one implementation | `symbol_graph.rs:537-555` | package-internal |
| `unique_reference(site)` | the call's identifier resolves to exactly one symbol (no `Ambiguous` row) | `direct.rs:261-285` | package-internal |
| `export_link(import, package, name)` | the import binding follows module links, at most 8 hops, to one exported declaration; namespace imports and multi-symbol links stop | `direct.rs:305-327` | **boundary**: the link is re-resolved against the stitched dependency tree (which version answers the name) |
| `export_not_rewritten(package, path)` | nothing writes the exported access path after the module initialises; trivially true for an ESM named export written only by its module, not for `module.exports.x`, `exports.x` or a namespace object another package can patch | implied today, never recorded | **boundary**: a stitched package that writes the path breaks it |
| `prototype_closed(class)` | no code outside the package writes the class's prototype or the prototype chain its instances inherit | implied by every method resolution; never recorded | **boundary** |
| `declared_target(site)` | the type checker resolved the call to one declaration, the declaration is a function, method or constructor in the analysed tree (a class only where the call constructs it; not an accessor, a variable or an external declaration), the receiver type is named by one declaration (not a union, an intersection or an object literal), and the file is under 25% `any` receivers | `ts_types.rs`, `TypedAnswer::names_the_target` | package-internal (types are compiled per package) |
| `no_inward_flow(site)` | no value from the boundary reaches the site's callee expression: not a parameter of an exported function or method, not `this` of an exported method, not an imported value, not a property of an object that enters or leaves | the solver's exact tier, WP3 | **boundary**: stitching that adds inflow drops the site to open |
| `no_outside_field_write(object, field)` | no stitched package stores into field `f` of an object that leaves this package | the solver's exact tier, WP3 | **boundary** |
| `within_budget(package)` | the solve reached a fixed point with no budget event | `points_to/solver.rs`, `solver/engine.rs` | package-internal; a package graph with a budget event has no exact claims at all |

Abstentions are not assumptions and are listed so they are not mistaken for
them: sites inside a `throw` argument are skipped (`direct.rs:36`); member
calls on built-in globals are never bound to a same-named user function
(`direct.rs:46-52, 414-458`); a class symbol is a target only at `new`
(`direct.rs:87-91`); a `SyntaxOnly` lexical match stays `Heuristic`.

The package graph stores, per exact-complete site, the list of assumption
ids it depends on. WP5's stitcher re-checks the boundary ones; WP3's tier
assignment writes them.

## (c) Constructs WP2 must lower, against what MIR carries

What MIR carries today (`ts/mir/lower.rs`, `ir/op.rs`): every statement kind
is visited; every expression kind is visited; values are `Literal`, `Place`,
`Temporary`, `CallReturn`, `BinOp`, `Aggregate{Array|Object|Composite}`,
`Closure{body, captures}`, `Unknown`; operations are `Bind`, `Assign`,
`Read`, `Write`, `Branch`, `Call{site, callee, arguments, return_place}`,
`Return`, `Unsupported`; places are roots with `Property`, `IndexKnown`,
`IndexUnknown`, `Deref` projections. The constraint projection reads only
`PlaceRef`/`CallReturn` value facts, the object-model rows (object, array,
function, class allocations; static, private, literal-key reads and writes;
receiver bindings; prototype links) and the direct bindings. The `Call`
operation's callee is `MirValue::Unknown { evidence: <text> }`
(`lower.rs:3734`), so no constraint today connects a call to its callee
place, its arguments or its result.

`push_unsupported` sites in the lowering, by label (non-test code):
`spread` (7), `optional chaining` (4), `complex destructuring` (2), and one
each of `with`, `v8 intrinsic`, `try`, `switch`, `Proxy`, `private in`,
`private field`, `labeled statement`, `JSX callback scheduling`, `for-of`,
`for left binding`, `for initializer`, `for-in`, `eval`, `dynamic property
key`, `dynamic import`, `dynamic CommonJS require`, `do while`, `debugger`,
`continue`, `break`, `class expression`, `catch destructuring`. The
control-flow ones (`try`, `switch`, loops, `break`, `continue`, `labeled`)
do not matter to a flow-insensitive value analysis as long as the
assignments inside them are still lowered; WP2 must confirm they are
(axios: 214 `try`, 115 `break` unsupported rows).

The closed list. "Have" means the constraint exists or is a one-line
projection of a MIR fact; "partial" means MIR has the fact and the
constraint is missing or incomplete; "missing" means neither.

| # | construct | constraint rule | status |
|---|---|---|---|
| 1 | variable binding, assignment, compound assignment | `Copy(lhs ← rhs)`; `a = b = c` chains | have (`PlaceRef` copies) |
| 2 | `\|\|`, `&&`, `??`, `?:`, comma | `Copy(result ← each operand that can be the value)`; comma: last operand | missing (`BinOp` is opaque) |
| 3 | object and array literals, incl. methods, getters/setters, computed keys, spread | `Alloc`; `FieldStore(obj, k, v)` per property; getters/setters as accessor fields; spread as a copy of every field of the source | partial (allocations and constant-key writes; spread and accessors missing) |
| 4 | property load/store with a constant key, incl. string and numeric literals, private `#x` | `FieldLoad`/`FieldStore` | partial (object-model rows only when the base resolves to an allocation; places with `Property` projections are not projected) |
| 5 | computed key, unknown | load: `Copy(dst ← every field of base)` and `Copy(dst ← base.*)` as conservative; store: `FieldStore(base, *, v)`; site open with reason `computed_key` | missing (`dynamic property key` is unsupported/havoc) |
| 6 | computed key, constant-foldable (`"a" + "b"`, a `const` string, an enum member) | as #4 | missing |
| 7 | destructuring: object, array, nested, defaults, rest | object: `FieldLoad` per name; array: element load per index, rest as remaining elements; default: `Copy` from the default expression too | partial (`complex destructuring` unsupported) |
| 8 | call: callee value, arguments to parameters, return to result, `this` | `CallConstraint(site, callee_place, args, result, this_place)`; the solver's call rule adds `Copy(param_i ← arg_i)`, `Copy(result ← callee.return)`, `Copy(callee.this ← receiver)` per resolved callee | missing (the central gap) |
| 9 | rest parameters, `arguments`, spread arguments | rest: an array allocation whose elements are the remaining arguments; spread argument: every element flows to every remaining parameter | missing |
| 10 | `new`: fresh object, prototype from `callee.prototype`, `this` bound, return override when the constructor returns an object | `Alloc(instance)`, `FieldStore(instance, $proto, callee.prototype)`, call rule with `this = instance`, `Copy(result ← instance ∪ returned objects)` | partial (`ClassInstance` allocation; no prototype store from a function value; no `this`) |
| 11 | function declarations and expressions as objects: `.prototype`, own properties, `Object.defineProperty(f, k, …)` | `Alloc(fn)`, `FieldStore(fn, "prototype", proto_obj)` created lazily; own-property stores as #4 | partial (`FunctionObject` allocation; `prototype` object missing) |
| 12 | closures and captured variables | a captured variable is one place shared by the defining body and the closure body; `Closure{captures}` names them | partial (MIR records captures; constraints do not share the place) |
| 13 | `this`: method call receiver, arrow lexical `this`, free-standing `this` (module/undefined), `call`/`apply`/`bind` receivers | `Copy(callee.this ← receiver)` from the call rule; arrow: `Copy(arrow.this ← enclosing.this)`; object-literal method with no known receiver: conservative `Copy(this ← literal)` | partial (`LexicalThis`, `MethodCall` receiver rows exist; no `this` place per function) |
| 14 | classes: constructor, methods on the prototype object, static members, instance fields, `extends`, `super(...)`, `super.m()`, class expressions, accessors | `Alloc(class)`, `Alloc(class.prototype)`, `FieldStore(prototype, m, method)`, `FieldStore(class, s, static)`, `FieldStore(prototype, $proto, parent.prototype)`, `super()` as a call of the parent with the same `this`, `super.m()` as a load on the parent prototype | partial (object-model `ClassPrototype`/`ClassExtends` links; `class expression` unsupported in MIR; `super` and fields missing) |
| 15 | prototypes without `class`: `F.prototype.m = …`, `F.prototype = {…}`, `Object.setPrototypeOf`, `Object.create(p)`, `__proto__` | stores on the prototype object; `$proto` links; method lookup walks `$proto` in the solver | partial (`ConstructorPrototypeProperty`, `SetPrototypeOf`, `ProtoProperty` links; `Object.create` and lookup along `$proto` in the solver missing) |
| 16 | getters and setters | a load through an accessor is a call of the getter with `this = base`; a store a call of the setter | missing |
| 17 | `Function.prototype.call`, `apply`, `bind` | `call`: call rule with `this = arg0`, args shifted; `apply`: `this = arg0`, args from the array's elements; `bind`: `Alloc(bound)` whose call forwards with the bound `this` and prefix arguments | missing (`CallApplyBind` is an unresolved reason) |
| 18 | ESM: `import`, `export`, `export default`, `export *`, re-exports, namespace imports, live bindings | an import binding is a `Copy` of the exporting module's place; `export *` copies every export; namespace import is an object whose fields are the exports | partial (direct bindings resolve named/default/namespace-member calls; `export *` and namespace objects as heap objects missing) |
| 19 | CommonJS: `module.exports = …`, `exports.x = …`, `require(literal)`, `Object.defineProperty(exports, …)`, the `__esModule` marker and the Babel/TypeScript interop helpers, `(0, m.x)(…)` | `module.exports` and `exports` as one object per module; `require` as a load of that object; the interop helpers analysed as ordinary code once #8 and #11 exist (no special case) | partial (walker recognisers; bench: `CommonJsExportsProperty/Unsupported` on every commander export; sequence callee unextracted) |
| 20 | platform, as a package model: `Array.prototype` iteration and transformation methods, `Array.from`/`of`, `Promise` (`then`/`catch`/`finally`/`all`/`allSettled`/`race`/`resolve`, executor), `async`/`await`, generators and iterators, timers, `queueMicrotask`, `EventEmitter.on`/`emit`, `Object.assign`/`defineProperty`/`defineProperties`/`keys`/`entries`/`values`/`freeze`/`create`, `Reflect`, `Map`/`Set`/`WeakMap` | model rows: which parameter is called with which arguments and `this`, and what is returned; `ModelEdge` exists as a constraint kind | missing as data (the walker has hand-written cases for most) |
| 21 | optional chaining `a?.b()`, `a?.()` | as the non-optional form | missing (unsupported) |
| 22 | `await`, promise results | `await p` copies the fulfilled value: `Copy(dst ← p.$fulfilled)`; an `async` function's return stores into its promise's `$fulfilled` | missing |
| 23 | generators, `yield`, `for…of`, spread of iterables | elements as one `$elements` field; `for…of` loads it; `yield v` stores it | partial (walker sequence model; MIR marks `for-of` unsupported) |
| 24 | `eval`, `new Function`, `with`, `import(expr)`, `require(expr)`, `Proxy`, `Reflect.construct`, `arguments.callee` | no lowering; the site or file is open with the matching reason | have (unsupported facts) and must stay honest |
| 25 | tagged templates | a call with the strings array first | have |
| 26 | JSX | element creation as a call of the component; attributes as arguments | out of scope for this iteration; keep unsupported with reason |

Each construct lands with two construct fixtures under
`tests/js-analysis-bench/constructs/`, one held out, as ITERATION-1 already
requires. The walker is retired construct by construct: a recogniser goes
when the construct's fixtures pass without it and the tune-set recall for
that construct's cause bucket does not drop.
