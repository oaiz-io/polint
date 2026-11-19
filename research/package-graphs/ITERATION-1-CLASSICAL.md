# Iteration 1: The Classical Baseline for JS/TS

Date: 2026-11-17

Baseline: polint v0.5.3 (`7aa3b3cf`). Numbers are from the committed
`tests/js-analysis-bench/baseline/report.json` unless a line says otherwise.

Goal: make stages 0 and 1 of the [accuracy ladder](README.md#the-accuracy-ladder)
as accurate as classical analysis can make them on JavaScript and TypeScript,
with measurements that say so, and leave the seams the later stages need:
stable identities, tiers, a first package graph and a first stitch.

Later stages are cheaper per recovered edge the better this one is. Every edge
that classical analysis finds is one that no run, model or agent has to pay
for.

## Where the engine stands

Recall against calls observed while each package runs. `base` is what a rule
gets today. `refined` adds the points-to layer. `typed` adds the TypeScript
checker.

| package | observed edges | base | refined | typed |
|---|---|---|---|---|
| async | 637 | 324 (51%) | 328 (51%) | 338 (53%) |
| axios | 675 | 270 (40%) | 270 (40%) | 511 (76%) |
| chalk | 72 | 17 (24%) | 17 (24%) | 17 (24%) |
| commander | 318 | 21 (7%) | 119 (37%) | 248 (78%) |
| dayjs | 117 | 0 (0%) | 0 (0%) | 0 (0%) |
| underscore | 262 | 157 (60%) | 158 (60%) | 202 (77%) |
| zod | 1308 | 719 (55%) | 719 (55%) | 764 (58%) |
| **total** | **3389** | **1508 (44.5%)** | **1611 (47.5%)** | **2080 (61.4%)** |

Wrong answers: no call site reports a complete answer that the run
contradicts, on any plan. `base` and `refined` claim no target that an
executed site never called. `typed` claims 15 such single targets and 2
multi-target ones.

Cost on zod (3.1 MB of source), one fact dump:

| plan | time | peak memory |
|---|---|---|
| base | 13.2 s | 1.3 GB |
| refined | 48.8 s | 3.8 GB |
| typed | 58.7 s | 4.4 GB |

### What the misses are

The 1,309 edges that even `typed` misses, grouped by what would find them.
The grouping is a reading of the bench's cause buckets, which are a triage of
the call site and the observed callee, not a proof of cause.

| Group | Buckets | Edges | Share |
|---|---|---|---|
| Function values that move | `parameter_call` 224, `callback` 155, `call_apply_bind` 96, `local_call` 70 | 545 | 42% |
| Objects built at run time | `local_member` 126, `parameter_member` 101, `module_member` 67, `constructor` 65, `this_member` 34, `object_literal_member` 6, `call_result_member` 1 | 400 | 31% |
| Code the engine does not read | `sequence_interop` 154, `not_extracted` 148, `not_analysed` 47 | 349 | 27% |
| Computed keys | `computed_key` 15 | 15 | 1% |

More type information does not find these. The checker knows the signature of
a parameter, not which function was passed. For an object built at run time it
points at an interface declaration, not at the function with the body. The
missing part is value flow.

### What the engine has for value flow

- A constraint vocabulary (`CopyEdge`, `Alloc`, `FieldLoad`, `FieldStore`,
  `CallConstraint`, `ModelEdge`, `TypeConstraint`) in
  `crates/polint/src/analysis_neutral/semantic_graph/constraints.rs`.
- An inclusion-based points-to solver with budgets in
  `crates/polint/src/analysis_neutral/points_to/` (about 1,600 lines).
- For JS/TS, the constraints come from direct bindings and from
  `crates/polint/src/ts/callable_flow/extract.rs` (about 11,000 lines): a
  walk per call site over the syntax, with a node budget, a depth limit of 64
  and four rounds across modules. Its own description is "bounded, heuristic
  callable flow".

The first reading of this document (medium confidence, before the code was
read in full) was that the solver is not the gap and lowering is. That reading
was checked and corrected in [DIAGNOSIS.md](DIAGNOSIS.md); the verdict, in
order of weight:

1. The points-to solve is discarded wholesale on every package above about a
   thousand call sites: three global budgets sized for fixtures (10,000 steps,
   64 objects per set, 512 field slots) mark every set `BudgetExceeded` when
   one trips, and no edge is derived from such a set. axios, async and zod
   have zero resolved points-to edges; `polint inspect unknowns` on axios
   reports the latch. The walker's rows ride through the same solve and are
   lost with it. Only commander stays inside the budget, which is the whole
   of the `refined` column's gain.
2. The solver has no call rule (arguments to parameters, return to result,
   receiver to `this`). The walker does that job with recognisers under a
   depth cap, a visit budget, four module rounds and a single return pass.
   It is not a fixed point and cannot be made one.
3. Lowering is incomplete in the specific ways the twenty traces in
   DIAGNOSIS §3 list: closures that capture parameters, function objects with
   properties, rest and spread, `call`/`apply`/`bind`, `(0, f)(x)`, `this` in
   object-literal methods, computed keys with a known field set, and the
   platform.
4. The typed plan's 15 wrong exact targets come from the join between the
   checker's declaration and polint's functions falling back to the nearest
   enclosing function, not from the checker.

The "274 call sites more than base on zod" were not the points-to layer:
they are test-framework recogniser rows in `src/**/tests/**`, files the
driver never runs, which the refined plan's capabilities enable. The
outside baselines are now measured, not quoted: Jelly static reaches 84.5%
on these packages with 13 wrong single-target claims and 5,811 wrong
multi-target claims ([BASELINES.md](BASELINES.md)).

## Work packages

In order. Each is its own pull request. Each lands with tests that fail
without it, and a bench report that shows its effect. The executable plan,
one GSD phase per pull request with files, steps, fixtures-first tests,
exit commands, mutation checks and the lines not to cross, is in
[implementation/README.md](implementation/README.md) and
`.planning/phases/72-…` to `79-…`. The diagnosis moved two things: WP2 lands
as two pull requests, the solver's call rule and budget semantics first
(nothing lowered can be measured while the solve is discarded), then the
lowering; and the walker's retirement is a measured criterion of WP2, not a
hope.

### WP0. Measure first

1. **Tiers in the bench.** Report recall for exact edges alone and for exact
   plus conservative edges. Map the existing `CallPrecision` values to the two
   tiers as a closed list (proposal: `Exact` and `SetupAware` are exact,
   `Conservative` and `Ambiguous` are conservative, `Heuristic` is neither and
   is reported apart until it is gone).
2. **Executed-site precision.** For call sites the run executed: claimed
   targets that the run called from that site, over all claimed targets
   there. A target the run did not call may still be real, so this is a lower
   bound on precision. Also report targets per resolved site (mean and 95th
   percentile).
3. **Open-site report.** Share of call sites that are open, by reason.
4. **Outside baselines on the same packages, scored by the same scorer.**
   Measured in [BASELINES.md](BASELINES.md) with a throwaway scorer: Jelly
   static 84.5%, `--approx` 85.4%, ACG unusable on modern syntax. WP0 commits
   the scorer as a bench measure so the columns are reproducible from
   tracked files.
5. **A held-out set.** Decided in [HELD-OUT.md](HELD-OUT.md): eight pinned
   packages, their observed calls recorded once under the authorship rule,
   driver coverage reported with every number. The Jelly PLDI 2024 artifact
   is a 43 GB VM image with nothing separately downloadable, so its traces
   are a cross-check at most, not the baseline Q1 hoped for.
6. **Cost, re-measured on a quiet machine**, with the refined plan's
   `semantic_graph` provider timed apart; confirm or refute the 50× axios
   regression DIAGNOSIS §5 observed, and bisect if it holds.
7. **Budget events**, reported per package from the solver's status, so the
   latch is visible in the report before WP2 removes it.
8. **Two measures the diagnosis needed and the report lacks**: refined edges
   by tier, algorithm, status and precision; and sites gained by the refined
   plan grouped by file and intersected with executed sites.

Exit: one report with all of the above for v0.5.3, committed as the baseline
of this iteration.

### WP1. Read all the code (stage 0)

The bench lists these as misses that are not worked around:

- code inside UMD and IIFE wrappers, and functions under a unary operator
  (dayjs: 117 edges, 0% recall);
- functions declared inside `try`, `catch` and `if` blocks;
- calls in exported variable initialisers;
- Babel's CommonJS interop: `(0, x.default)(...)` (the call extractor
  reports a sequence-expression callee as `Unknown`; it is a call to the
  last operand), `_interopRequireDefault(require(p))`, `exports.default = f`
  (async: 154 edges);
- a package's own `vendor/` directory. The default exclusion is right for a
  repository and wrong when the package itself is the unit (chalk: 47 of 72
  edges).

Exit: `not_extracted`, `not_analysed` and `sequence_interop` are 0 on the
tune set, apart from exclusions that are written down with a reason.

### WP2. The call rule, then lower all JS/TS value flow into constraints (stage 1)

Two pull requests. The first makes the solver able to carry what the second
lowers:

- **2a. Solver semantics.** Remove the global latches: a budget event ends
  the solve, keeps every set derived so far, marks the package incomplete
  with the reason, and never blanks the result. Add the call rule: a
  `CallConstraint` names the callee place, the argument places, the result
  place and the receiver place; when the callee set gains a function, copy
  arguments to parameters, return to result, receiver to `this`. Give every
  function a parameter place per position, a rest place, a `this` place and
  a return place. Key field slots by `(object identity, field)`. Exit: the
  composition fixtures of [STORAGE-A.md](STORAGE-A.md) solve to the same
  edges in any constraint order and at any budget that does not trip, and
  the three large tune-set packages report a solve with no budget event.
- **2b. Lowering**, by the closed list in [TIERS.md](TIERS.md) (c), in the
  order of edges it wins on the tune set: calls and closures, function
  objects and prototypes, `this` and classes, modules, platform models,
  computed keys and destructuring, accessors, `call`/`apply`/`bind`,
  `await` and generators.

Generate constraints from MIR for every construct, not from a search that
starts at a call site:

- assignment, destructuring, default values, spread;
- property store and load. Field-based where the base object is known; by
  property name alone where it is not, as a conservative edge;
- calls: arguments to parameters, return to result, `this`, rest parameters;
- closures and captured variables;
- classes and prototypes: `new`, method lookup along the prototype chain,
  `super`, `Object.assign(F.prototype, {...})`;
- `call`, `apply`, `bind`;
- modules: ES imports and exports, `module.exports`, `require`;
- promises and `await`; getters and setters;
- factories that return classes or functions (zod's `$constructor`).

The platform is modelled as a package: array iteration methods,
`Promise.prototype.then`/`catch`/`finally`, timers, `queueMicrotask`,
`EventEmitter`, `Object.assign` and `Object.defineProperty` with constant
keys.

Each construct lands with two construct fixtures under
`tests/js-analysis-bench/constructs/`, one of them held out.

Exit: the walker is no longer the source of flow constraints. It is removed,
or kept only for the edges that a measurement shows nothing else finds, with
that measurement in the pull request. Measured: `Heuristic` edges on the tune
set are 0, and the `semantic_graph` provider is at most 10% of the refined
plan's time on every package.

### WP3. Solver: fixed point, scale, tiers

- The solve reaches a fixed point on every tune-set package with no budget
  event. Where a budget is still hit, the affected sites say so, and the
  edges derived before it stay (WP2a made that the contract).
- The standard scaling work: cycle collapse, difference propagation, shared
  points-to sets, and the indirection bounding that
  `static-analysis-2.0/02-memory-and-representation.md` already names.
- Tier assignment from the solution: a site with one target and no open
  inflow is exact, with its assumptions recorded from the closed list in
  [TIERS.md](TIERS.md) (b); anything else is conservative, and open when a
  value can still enter from outside.

Exit: the cost targets below.

### WP4. Types feed the same solver

The checker's facts become constraints and filters in the one solver, not a
separate tier beside it: declared types filter targets that cannot fit, and
they seed receivers where flow is open (as conservative edges). The 15 single
wrong targets the typed plan claims today are fixed here; DIAGNOSIS §4 traces
them to the join rule in `refined_calls/ts_types.rs` (a declaration that is
not a function, or a union receiver, must not give an exact edge), with the
fifteen as fixtures.

Exit: the typed plan finds every edge the refined plan finds, and claims no
wrong exact target.

As built: types are a step after the solve rather than constraints inside
it (`refined_calls/type_assignment.rs`; [TIERS.md](TIERS.md), note on
types). Declared types say what a call may run, not which value flows to it,
so the step claims nothing exact: it moves an exact edge the checker's
declared kind or arity says cannot run to conservative (`type_mismatch`),
adds methods of the receiver's declared class at an open site
(`type_seed`), and lists the checker's targets at a site no tier answered,
conservative and open. The fifteen wrong exact targets are gone because the
typed tier no longer claims exact; the fixtures stay. The bench fails when
the typed plan finds fewer observed edges than the refined plan in any tier
(`report.py`, `types_never_lose`). That check found the native call kind
guessing construction from a capitalised name; the step reads it from the
checker's syntax instead. On the bench the typed plan equals the refined
plan at exact (tune 47.5%, held-out 32.3%) and raises exact + conservative
from 67.9% to 75.6% on the tune set and from 83.5% to 84.3% held out, with no
wrong exact target. No mismatch or seed fires on the tune set; the gain is the
checker's targets at sites no other tier answered.

### WP5. Package graph v0: export, stitch, composition test

Follow [PACKAGE-GRAPH.md](PACKAGE-GRAPH.md), storage option A: the package's
constraints and its solved state, under the preconditions
[STORAGE-A.md](STORAGE-A.md) derives from the solver (no latches, stable slot
identities, no walker rows, no stored copy closure, solution stored for
stitch-readable nodes only).

- Export a package graph for one package. Its edges reach rules through the
  `CallGraph` view; a public surface for the export itself (and for the
  stitcher and the correction layer) is an open question for the owner, not
  a scope limit.
- Stitch package graphs against a resolved dependency tree.
- The composition test: a fixture of one application and two libraries with
  callbacks that cross both ways, analysed together and analysed apart then
  stitched. Edge sets are equal at both tiers.
- The same comparison on three real trees (for example axios with its
  dependencies).
- Function and call-site identities are equal across two scans and across
  machines.

Exit: the composition test passes on the fixture and the three trees, and the
size and stitch time of each are in the report.

### WP6. Correction layer, smallest proof

Load the bench's recorded edges for one package as corrections with evidence
kind `observed`, apply them after the engine output, and show: recall over
observed edges is 100% for that package, every added edge is marked as
observed, the engine output without the layer is byte-identical to before,
and after a change to one function its corrections are reported stale.

Exit: that test. Nothing more of stages 2 to 5 is built in this iteration.

## Success criteria and targets

Tune set: the seven packages above. The held-out set is the eight packages
of [HELD-OUT.md](HELD-OUT.md). "No types" is today's `refined` plan. Every
target below was checked against the diagnosis and the measured baselines
in [TARGETS.md](TARGETS.md), which records what was kept, replaced or added
and why; the tables here are the result.

### Accuracy

| Measure | v0.5.3 | Target |
|---|---|---|
| Recall, exact plus conservative, no types | 47.5% | 80% overall; hard gate 75% |
| Recall per package, no types | 0% to 60% | every package at least 65% |
| Recall, exact tier alone, no types | 44.5% (base) | 60% |
| Recall with types | 61.4% | 85% |
| Against Jelly on the same packages (measured: static 84.5%, `--approx` 85.4%) | 47.5% / 61.4% | no types at least 80%, with types at least 85%, and the precision rows below, which Jelly does not meet |
| Call sites with a complete answer the run contradicts | 0 | 0 |
| Wrong exact targets at executed sites | 0, 0, 15 single and 2 multi (base, refined, typed) | 0 on every plan |
| Executed-site precision, conservative tier | not measured | at least 70%, reported per package |
| Targets per resolved site, mean and 95th percentile, per tier | not measured | reported; no gate this iteration |
| Precision on hand-labelled constructs, exact tier | 100% | 100% |
| Edges lost to code the engine does not read | 349 | 0, apart from written exclusions |
| `Heuristic` edges on the tune set | 161 (all on commander) | 0 |
| Budget events that discard derived edges | on every package above about 1,000 call sites | none, by construction |
| Held-out recall | no held-out set | within 5 points of the tune set; packages under 40% driver coverage reported and left out of the average |

The measured reference points on this bench are Jelly static at 84.5% with
13 wrong single-target and 5,811 wrong multi-target claims, and Jelly with
approximate interpretation at 85.4%; the field-based tool of Feldthaus et al.
does not parse these packages ([BASELINES.md](BASELINES.md)).

What remains missed at the end is reported by cause with counts. That list is
the input to stage 2: it should consist of what no static analysis sees
(property names computed at run time, code built from strings).

### Cost

Same machine as the committed baseline.

| Measure | v0.5.3 | Target |
|---|---|---|
| zod, full graph, no types | 48.8 s | 20 s (Jelly static: 22.9 s in docker) |
| zod, peak memory, no types | 3.8 GB | 2 GB |
| axios, full graph, no types | 2.1 s committed; 105 to 112 s measured on this checkout under load (DIAGNOSIS §5, to be confirmed in WP0) | 1.5 s |
| Refined plan against base plan, every tune-set package | up to 50× on axios if §5 holds | at most 3× |
| `semantic_graph` provider (the walker) share of the refined plan | 91 to 95 s of 105 to 112 s on axios here | removed, or at most 10% |
| Budget events on the tune set | present on axios: `points_to.max_dynamic_vars`, `solver.max_steps` | 0, and reported |
| Stitch time on a real tree, against analysing it whole | no stitcher | at most 20% |
| Stored package graph, gzip, per MB of source | no package graph (the fact store is 6.9 MB per MB for zod; Jelly's zod call graph is 0.18 MB per MB) | at most 2 MB, as a ceiling |

The stitch and size targets are first estimates. WP5 measures them, and the
measurement sets the budget for the next iteration.

### Structure

- Stitched equals whole: equal edge sets on the fixture and on three real
  trees.
- Two scans of the same package give a byte-identical package graph, for any
  thread count.
- Every edge carries stage, algorithm and tier. Every open site carries a
  reason. Every exact-complete site carries its assumption ids.
- The package graph refuses to store a budget-exceeded solve and the
  stitcher refuses to read one.
- Every new guard has a test that fails without it; mutation checks are
  listed in each pull request, as in #139 and #140.

## Out of scope

- Building stages 2 to 5. Only WP6 touches them.
- A registry, a scanning service, or agents.
- An unfinished public CLI command, flag or debug helper. Every analysis
  result this iteration builds reaches rules through the SDK view that serves
  its question, by the "No Side Engines" section of `AGENTS.md`; a result that
  needs a public surface not yet designed is an open question for the owner.
- Go. The same design applies; this iteration proves it on JS/TS.
- Replacing the Node type sidecar with TypeScript 7. Its stable API comes in
  7.1.
- Taint and data-flow changes.

## Risks

| Risk | Sign | Response |
|---|---|---|
| Field-based flow makes the conservative tier too wide to use | Targets per site grows; executed-site precision falls under 70% | Selective precision where it pays (`static-analysis-2.0/06-selective-precision-and-demand.md`); keep the exact tier untouched |
| The solve does not fit the cost targets on zod | Budget events, or time above the refined plan today | The scaling work in WP3 before more constructs in WP2 |
| Tuning on seven packages | Tune-set recall rises and held-out recall does not | The held-out gate; construct fixtures with a held-out case each |
| Recall over thin runs | A package's driver exercises little of it | Driver coverage in the report; extend the drivers |
| Exact claims that stop being true after stitching | A composition-test failure at the exact tier | Assumptions recorded with each claim and re-checked at stitch |
