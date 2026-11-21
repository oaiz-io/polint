# The Package Graph

Date: 2026-11-17

Research question: what must polint store for one package version so that
(a) graphs of many packages stitch into the graph of a whole program with no
loss against analysing them together, (b) the stored graph can be corrected
later without a new scan, and (c) a better engine can replace the engine's
part while the corrections stay.

Status: design direction. Nothing here is built. Open questions are listed at
the end with the experiment that settles each.

## Prior art

| Source | What it does | What polint takes from it |
|---|---|---|
| JAM, modular call graphs for Node.js (ISSTA 2021) | Builds a call graph per module of a Node.js application and its dependencies, and composes them. | The unit of analysis follows the module structure, and composition is designed in from the start. |
| FASTEN | Builds and stores one partial call graph per package version, **including its unresolved calls**, and stitches them against a resolved dependency tree. | The stored graph keeps its open ends by name. Dependency resolution is an input to stitching, not to the scan. |
| Infer (Meta) | Per-procedure summaries, computed bottom-up, reused across runs. | A summary lets the analysis use a callee without loading its body. |
| CodeQL model rows | Library behaviour (sources, sinks, flow summaries) written as data beside the analysis. | Package models are data with a fixed schema, validated before use. |
| Jelly with approximate interpretation (PLDI 2024) | Executes module code to learn dynamic property writes, then feeds them to the static analysis. Average recall 75.9% to 88.1% on 36 projects. | Recorded execution is a stage of its own and its result is an input to the graph, with evidence. |

The first four are already cited in
[`static-analysis-2.0/03-summary-store.md`](../static-analysis-2.0/03-summary-store.md)
and [`OPEN-QUESTIONS.md`](../static-analysis-2.0/OPEN-QUESTIONS.md). Source
notes are in [VALIDATION.md](VALIDATION.md).

## Identity

A package graph is identified by:

- package name and version;
- a digest of the package content (the registry integrity value where there
  is one; a content hash of the analysed files otherwise);
- the entry configuration: which export conditions and which module format
  were analysed, because `exports` in `package.json` can select different
  files for `import`, `require` and `browser`;
- engine version, graph schema version, and the digest of the model set.

The application's own code is a package with the same format. Its identity is
its content hash.

Inside a graph, a function is identified by its path in the package, its span
and a hash of its body. A call site is identified by its enclosing function
and its span inside that function. These identities must be the same in two
scans of the same content, on any machine. They are the anchors of
corrections.

## Contents

1. **Functions and call sites.** Every one, including those the engine cannot
   resolve.
2. **Internal edges.** Call site to function, each with its evidence: stage,
   algorithm, and tier (exact or conservative).
3. **Site status.** Each call site is *complete* (every target is known) or
   *open*. An open site names what is missing: a value that enters through
   the boundary, a computed key, dynamic code, an exhausted budget.
4. **The boundary.** This is what makes the graph stitchable.
   - *Exports.* For each entry point, each access path a user can reach
     (`default`, `parse`, `z.string`) and what it is: a function, a class, an
     object with members, or a re-export of an access path in another
     package.
   - *Imports.* Each access path this package reads from another package
     (`("lodash", "map")`, `require("x").a.b`), by package name. Which version
     that name means is decided at stitch time.
   - *Inward values.* What the package does with a value it receives from
     outside: a parameter of an exported function, `this`, the result of an
     imported call. It may call the value (with which arguments), store it,
     return it, or pass it on to an import.
   - *Outward values.* Package functions that leave: returned from an export,
     or passed as a callback to an import. Outside code may call them.
5. **Assumptions.** An exact claim can depend on a closed world, for example
   "nothing outside writes this property". Each such condition is recorded
   with the claim and checked again when graphs are stitched.
6. **Unknowns.** Regions the stage could not analyse, each with a reason.
7. **Stage reached**, counts, and budget events.

## Stitching

Input: a set of package graphs and a resolved dependency tree, which says for
each package which version each of its imports gets. The tree comes from the
package manager's lock file; polint's module graph already resolves it for a
checked-out repository.

Steps:

1. Link each import to the export it names.
2. Let values flow across each link in both directions: callbacks go from the
   application into a library and are called there, and library functions
   come back as results.
3. Solve to a fixed point. This is the only fixed point that crosses package
   boundaries.
4. Re-check the recorded assumptions. An exact claim whose assumption no
   longer holds drops to conservative.
5. Close the open sites that now have every target.

### Why stitching can equal whole-program analysis

Confidence: medium-high for the argument, unproven for the implementation.

Points-to analysis of the inclusion-based (Andersen) kind computes the least
solution of a set of monotone constraints: adding a constraint can only add
members to points-to sets and edges to the call graph. The constraints of a
whole program are the union of the constraints of its packages plus the link
constraints. The solution of one package alone is therefore below the
whole-program solution, and solving the union starting from the stored
per-package solutions reaches the same least solution as solving from
nothing.

Two things do not follow from this argument and need their own handling:

- **Claims that depend on absence.** "This binding holds exactly one
  function" is true because nothing else writes it. More code can make it
  false. These are the assumptions of item 5 above.
- **Budgets.** A solve that was cut short is not a fixed point. A package
  graph produced under an exhausted budget must say so, and stitching must
  not treat it as complete.

The invariant "graphs compose" is enforced by a test: analyse a multi-package
fixture as one program, analyse its packages apart and stitch, and require
equal edge sets at both tiers.

### Two ways to store what stitching needs

| Option | Stored | Stitch cost | Risk |
|---|---|---|---|
| A. Constraint graph plus solved state | Every constraint of the package and its internal solution | The remaining solve over the union; no parsing or lowering | Simple and equal by construction, but the stored graph is large and stitching touches all of it |
| B. Boundary summary | Only the boundary and the effect of the package on values that cross it, with internal nodes removed | Proportional to the boundary | Small and fast, but removing internal nodes correctly is the hard part |

Recommendation: build option A first, with the composition test, and measure.
Then reduce toward option B where the measurement shows the cost, keeping the
same test. Option A is the reference that option B must match.

## The correction layer

A correction is a record beside the engine output:

- the package graph identity it applies to;
- its anchor: the stable identity of a function or call site, with the body
  hash;
- the operation: add an edge, reject an edge, mark a site complete, add a
  boundary fact;
- the evidence: a recorded execution (which run, which input), a test (where
  it is, how to run it), or a review (who, what was read);
- the stage that wrote it and the engine version at that time.

Rules:

1. The engine never writes into this layer and never reads it during
   analysis. It is applied after the engine output and before stitching.
2. A re-scan with a newer engine replaces the engine output and keeps the
   layer. Each correction is then one of: still needed, now derived by the
   engine (retire it and keep it as a fixture), or stale (its anchor is gone).
3. An edge that came from a correction shows that in its evidence, so a
   consumer can tell an observed call from a derived one.
4. A recorded execution proves that an edge exists. It never proves that an
   edge is absent: a run sees only the paths it took.

The bench recorder under `tests/js-analysis-bench/observed/` already produces
edges of exactly this kind. Loading them as corrections is the smallest
end-to-end proof of the layer.

## Package models

A model describes a pattern the engine cannot derive from the code: "a
function passed to `router.use` is called with a request and a response",
"`$constructor(name, init)` returns a class whose constructor runs `init`".
The engine already accepts validated model edges as constraints
(`ConstraintKind::ModelEdge` in
`crates/polint/src/analysis_neutral/semantic_graph/constraints.rs`), and
rejected model facts stay report-only.

A package model is the same mechanism with a different owner: it is written
once for a library and is part of that package graph's identity through the
model-set digest. Prefer a model over a change to the engine when the pattern
belongs to one library. Move it into the engine when a second library needs
the same thing.

The platform is a package too. `Array.prototype.forEach`, `Promise.then`,
timers and `EventEmitter` call the functions they are given; a platform model
states that, so "callbacks called by builtins" is ordinary stitching.

## Open questions

| # | Question | Settled by |
|---|---|---|
| 1 | Option A or B for storage, and at what package size does A stop being acceptable? | Size and stitch-time measurements in iteration 1 on three real dependency trees. |
| 2 | Is a body hash enough to carry a correction from one version of a library to the next when the function did not change? | Count stable anchors across ten consecutive versions of two libraries. |
| 3 | Which unit is stored: the package, or each module in it? | Start with the package. Revisit if packages with many entry points are mostly unused code. |
| 4 | How are conflicting corrections ordered? | Not needed until stage 4 exists. Recorded execution outranks review; a rejection needs a reason that can be re-checked. |
| 5 | Which assumptions can exact claims carry, as a closed list? | Enumerate them from the exact-tier resolvers that exist today (`calls/direct.rs`, the value-function rows in `ts/symbol_graph.rs`). |
| 6 | How is a package graph trusted when it comes from somewhere else? | Out of scope here; `static-analysis-2.0/03-summary-store.md` already names signing or spot verification. |
