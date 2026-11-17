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

Reading (confidence: medium; the module documentation and the constraint
projection in `ts/semantic_graph_build.rs` were read, not all of the walker):
the solver is not the gap. The gap is that JS/TS value flow is not lowered
into constraints completely and uniformly. A walk that starts at a call site
and looks for its callee finds what its recognisers cover; a constraint for
every assignment, property access, argument and return finds what the program
does.

Evidence that fits this reading: on zod the refined layer resolves 274 call
sites more than base (43,508 to 43,234 without a target), none of them on an
executed edge, at 3.7 times the time and 2.9 times the memory.

## Work packages

In order. Each is its own pull request. Each lands with tests that fail
without it, and a bench report that shows its effect.

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
   Jelly (static, and with `--approx`) and the field-based analysis of
   Feldthaus et al. This replaces averages borrowed from papers with numbers
   on this bench. The micro suite's manifest
   (`research/evaluation-harness/suites/jelly-callgraph-micro.toml`) already
   pins a Jelly checkout.
5. **A held-out set.** Add packages that no change is tuned on.
   `static-analysis-2.0/OPEN-QUESTIONS.md` Q1 already decided the source:
   projects from the Jelly PLDI 2024 artifact that have dynamic call graphs.
   Record driver coverage for every package, so a recall number over a thin
   run is visible as such.

Exit: one report with all of the above for v0.5.3, committed as the baseline
of this iteration.

### WP1. Read all the code (stage 0)

The bench lists these as misses that are not worked around:

- code inside UMD and IIFE wrappers, and functions under a unary operator
  (dayjs: 117 edges, 0% recall);
- functions declared inside `try`, `catch` and `if` blocks;
- calls in exported variable initialisers;
- Babel's CommonJS interop: `(0, x.default)(...)`,
  `_interopRequireDefault(require(p))`, `exports.default = f` (async: 154
  edges);
- a package's own `vendor/` directory. The default exclusion is right for a
  repository and wrong when the package itself is the unit (chalk: 47 of 72
  edges).

Exit: `not_extracted`, `not_analysed` and `sequence_interop` are 0 on the
tune set, apart from exclusions that are written down with a reason.

### WP2. Lower all JS/TS value flow into constraints (stage 1)

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
that measurement in the pull request.

### WP3. Solver: fixed point, scale, tiers

- The solve reaches a fixed point on every tune-set package with no budget
  event. Where a budget is still hit, the affected sites say so.
- The standard scaling work: cycle collapse, difference propagation, shared
  points-to sets, and the indirection bounding that
  `static-analysis-2.0/02-memory-and-representation.md` already names.
- Tier assignment from the solution: a site with one target and no open
  inflow is exact, with its assumptions recorded; anything else is
  conservative, and open when a value can still enter from outside.

Exit: the cost targets below.

### WP4. Types feed the same solver

The checker's facts become constraints and filters in the one solver, not a
separate tier beside it: declared types filter targets that cannot fit, and
they seed receivers where flow is open (as conservative edges). The 15 single
wrong targets the typed plan claims today are fixed here.

Exit: the typed plan finds every edge the refined plan finds, and claims no
wrong exact target.

### WP5. Package graph v0: export, stitch, composition test

Follow [PACKAGE-GRAPH.md](PACKAGE-GRAPH.md), storage option A: the package's
constraints and its solved state.

- Export a package graph for one package, crate-private, through the bench
  dump only. No public command and no SDK surface in this iteration.
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

Tune set: the seven packages above. The held-out set comes from WP0. "No
types" is today's `refined` plan.

### Accuracy

| Measure | v0.5.3 | Target |
|---|---|---|
| Recall, exact plus conservative, no types | 47.5% | 80% overall; hard gate 75% |
| Recall per package, no types | 0% to 60% | every package at least 65% |
| Recall, exact tier alone, no types | 44.5% (base) | 60% |
| Recall with types | 61.4% | 85% |
| Against Jelly (static) on the same packages | not measured | at or above Jelly overall |
| Call sites with a complete answer the run contradicts | 0 | 0 |
| Wrong exact targets at executed sites | 0, 0, 17 (base, refined, typed) | 0 on every plan |
| Executed-site precision, conservative tier | not measured | at least 70%, reported per package |
| Precision on hand-labelled constructs, exact tier | 100% | 100% |
| Edges lost to code the engine does not read | 349 | 0, apart from written exclusions |
| Held-out recall | no held-out set | within 5 points of the tune set |

The published reference points are Jelly's 75.9% average static recall on 36
projects, and at least 85% recall with at least 66% precision for the
field-based analysis of Feldthaus et al. on its own benchmarks. The targets
sit at or above them; WP0 replaces them with measurements on this bench.

What remains missed at the end is reported by cause with counts. That list is
the input to stage 2: it should consist of what no static analysis sees
(property names computed at run time, code built from strings).

### Cost

Same machine as the committed baseline.

| Measure | v0.5.3 | Target |
|---|---|---|
| zod, full graph, no types | 48.8 s | 20 s |
| zod, peak memory, no types | 3.8 GB | 2 GB |
| axios, full graph, no types | 2.1 s | 1.5 s |
| Budget events on the tune set | not reported | 0, and reported |
| Stitch time on a real tree, against analysing it whole | no stitcher | at most 20% |
| Stored package graph, gzip, per MB of source | no package graph (the fact store is 6.9 MB per MB for zod) | at most 2 MB |

The stitch and size targets are first estimates. WP5 measures them, and the
measurement sets the budget for the next iteration.

### Structure

- Stitched equals whole: equal edge sets on the fixture and on three real
  trees.
- Two scans of the same package give a byte-identical package graph, for any
  thread count.
- Every edge carries stage, algorithm and tier. Every open site carries a
  reason.
- Every new guard has a test that fails without it; mutation checks are
  listed in each pull request, as in #139 and #140.

## Out of scope

- Building stages 2 to 5. Only WP6 touches them.
- A registry, a scanning service, or agents.
- Any new public CLI command, flag or SDK view. Everything stays crate-private
  and is read through the bench dump, by the public-surface rules in
  `AGENTS.md`.
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
