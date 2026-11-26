---
phase: 78-package-graph-v0-and-stitch
plan: 01
subsystem: package-graph
requirements-completed: [PG-09]
status: executed-with-deviations
---

# A package graph per package, stitched into the program's answers

A TS/JS package now exports a package graph: its identity, functions, call
sites, signatures, every points-to constraint with nodes named by stable text,
the answer at every call site with its tier, its budget, and a boundary (the
imports of other packages with the place that receives each, every file's
module and namespace places, and the widest function the value-flow guards
stand for). `stitch` reads the graphs of a program and a tree of where their
imports lead, links each import to the module place of the file it resolves
to, solves the union from nothing and answers every call site again.

On the composition fixture, the axios tree (27 packages) and the express tree
(65 packages, held out, compared only) the stitched answers equal the whole
program's at every call site: the same claim and the same targets, exact,
conservative and open alike.

## Commits

| commit | change |
|---|---|
| e275f502 | Three-package composition fixture |
| 312bbf09 | Composition test: stitched graphs must answer as the whole program |
| 81c4e901 | The export conditions are one named list, shared with the package identity |
| b5dd5110 | Stored form (`format.rs`) and identity (`identity.rs`) of a package graph |
| 54b76b6b | Export of a TS/JS package's points-to inputs and answers; determinism tests |
| e270e3e7 | The bench writes and sizes each tune package's graph |
| 766382b5 | A package's imports of other packages are described as outside code (the package cut) |
| 6a73bd86 | The boundary: imports, module places, guard width |
| bada7b0f | `stitch` and `Tree::on_disk`; the composition test passes |
| 66dfc463 | An ignored lib test stitches any npm-laid-out tree for the bench |
| de7d4920 | The axios and express trees pinned in `inputs.toml`, every module by integrity and sha256 |
| d4653093 | The bench compares stitched trees with the whole program |
| 0a2bfee0 | The bench's written graphs are ignored |
| a9f08fe8 | Baseline with stitched trees and graph sizes |
| 2c1d920e | TARGETS.md: stitch time and size measured |
| 71c0347c | Dense-id sweep re-baselined for the new code |

## Results

Baseline `tests/js-analysis-bench/baseline/` at a9f08fe8, TypeScript 5.9.3,
`POLINT_JS_BENCH_PACKAGE_GRAPH=1`. Counts are whole / stitched / only one side.

| tree | packages | sites | exact edges | conservative edges | open sites | unlinked imports |
|---|---|---|---|---|---|---|
| axios | 27 | 3130 | 774 / 774 / 0 | 575 / 575 / 0 | 2237 / 2237 / 0 | 50 |
| composition | 3 | 14 | 0 / 0 / 0 | 10 / 10 / 0 | 14 / 14 / 0 | 0 |
| express (held out) | 65 | 5041 | 888 / 888 / 0 | 734 / 734 / 0 | 4082 / 4082 / 0 | 55 |

An unlinked import resolves into no package of the tree (a Node built-in, or a
module the tree does not hold); the whole program sees it as outside code too,
so both answers agree. The report counts them by reason only.

Size, every package graph of the tree as JSON and gzip:

| tree | source KB | graph KB | gzip KB | gzip KB per MB of source | largest package |
|---|---|---|---|---|---|
| axios | 424 | 19345 | 1458 | 3439 | the root: 944 KB, 3347 per MB |
| composition | 2 | 100 | 11 | 7224 | the root: 5 KB |
| express | 663 | 31892 | 2408 | 3630 | qs: 530 KB, 4790 per MB |

The tune packages alone: 2232 (async) to 3991 (chalk) KB per MB, zod 2560,
dayjs 11724 on 7 KB of source. The constraints are 73% (axios) and 75% (zod)
of the stored JSON, nodes 20%.

Time, one run each:

| tree | whole ms | package graphs ms | stitch ms | stitch / whole | load average start | end |
|---|---|---|---|---|---|---|
| axios | 4088 | 4975 | 356 | 9% | 21.44 / 12.19 / 8.52 | 20.83 / 12.36 / 8.62 |
| composition | 40 | 36 | 1 | 2% | 21.44 / 12.19 / 8.52 | 21.44 / 12.19 / 8.52 |
| express | 8836 | 10748 | 775 | 9% | 20.83 / 12.36 / 8.62 | 17.35 / 12.13 / 8.62 |

An earlier full run at load 4.75 / 5.54 / 6.21 to 5.51 / 5.68 / 6.23 measured
10% (axios, 395 of 4131 ms) and 7% (express, 694 of 9394 ms).

TARGETS.md: stitch time ≤ 20% kept, now measured. The 2 MB ceiling is kept
and not met; the encoding of the constraints shrinks first.

## Deviations from the plan

- **The stitch solves the union from nothing.** The plan stored the solved
  sets of stitch-readable nodes, seeded the union solve with them, and then
  re-checked each exact-complete site's boundary assumptions. The graph stores
  no solved sets and no assumptions; the stitch derives every claim from the
  union, so no claim of a graph alone carries over and there is nothing to
  re-check. The mutation checks for seeding and for `prototype_closed` have
  nothing to act on. Seeding stays a cost lever for when stitch time matters;
  it is 9% today.
- **The boundary is imports and module places, not exports with inward and
  outward values.** A package offers its files' module and namespace places;
  an import names the place that receives it and the open value it holds
  alone. The stitch copies the target's module place into the receiving place
  and keeps the open value, because each package was described with the other
  as outside code, as the whole program now is.
- **The package cut.** The first composition run showed the whole program
  deciding openness, escape and summaries from every file it could see, so it
  made claims no package graph could derive. A file now belongs to the package
  of its innermost `node_modules` directory, and value-flow requests,
  callable-flow summaries and direct bindings into another package treat the
  target as outside code. The direct-binding cut is defensive: no fixture or
  tree needed it. Runs without `node_modules`, every tune package among them,
  are unchanged: the bench's numbers did not move.
- **Where the code is.** `format.rs`, `identity.rs` and `layout.rs` are under
  `analysis_neutral/package_graph/`; the export and the stitch read TS points-to
  inputs and live in `ts/points_to/package_graph.rs` and
  `ts/points_to/package_graph/stitch.rs`. The composition tests are a lib test
  module (`ts/package_graph_composition_tests.rs`), not
  `tests/package_graph_composition.rs`: they need crate-private items, and an
  integration test would widen visibility. Run them with
  `cargo test -p polint --lib package_graph_composition`.
- **sha256, not blake3,** for the body hash of a function identity: the
  workspace already depends on sha2; blake3 would be a new dependency.
- **Schema 1.** The stored form started at 0; adding the boundary bumped it.
- **Type constraints are left out of the graph.** They name a type fact by a
  run-local id and the solve does not read them.
- **The copy closure stays in `derive_edges`.** Removing it is deferred: the
  graph never stores it, so it does not affect size, and removing it changes
  the solver output digest and cache keys outside this plan's scope.
- **The boundary re-lowers each file's value flow** to read its import
  requests; the facts keep only places. A second lowering per file, paid at
  export only.
- **`require` resolves through the export conditions of `import`**, as the
  module graph does. Whole and stitched agree because both use the same
  resolver; a package whose `require` and `import` entries differ is resolved
  to the `import` entry.
- **Trees.** The plan asked for three real trees; two are measured, axios and
  express, plus the fixture. Their closures were resolved with
  `npm install --package-lock-only` (with a separate `--cache` directory, as
  the shared npm cache was not writable) and pinned by integrity and sha256.
  Only each module's shipped `.js`, `.cjs` and `.mjs` files are analysed, as
  for the tune packages.
- **Express is held out.** It is compared only: its row reports counts and
  differences, never example sites, and its source was not read. Its
  recordings and drivers are untouched.
- **The stitch runs only with `POLINT_JS_BENCH_PACKAGE_GRAPH=1`**, like the
  graph writing; a plain `make js-bench` is unchanged in cost.
- **The written graphs are not committed.** Their sizes are in the report; the
  `.gz` files are ignored.
- **Size attribution.** STORAGE-A.md assumed the solved sets would dominate.
  The graph stores none; the constraints, each with its field names and its
  stable key in full, are three quarters of the JSON.

## Verification

At 71c0347c, `GOTOOLCHAIN=go1.25.12`:

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: clean.
- `cargo test --workspace --locked --no-fail-fast`: all pass (polint lib 3057
  passed, 23 ignored). The first run failed `dense_id_sweep_matches_baseline`
  on nine new hits, each classified in 71c0347c; none admits a dense id into
  stored output.
- `cargo test -p polint --test public_surface_leak --locked`: 8 passed.
- `cargo test -p polint --lib package_graph_composition`: 6 passed, the bench
  tree test ignored by default.
- Two `POLINT_JS_BENCH_PACKAGE_GRAPH=1 bench.py report --no-cost` runs with
  TypeScript 5.9.3: every written package graph byte-identical; `report.json`
  identical except `dump_cost` and `stitch_cost`, which hold wall times and
  memory. Load average 7.05 / 5.56 / 6.76 at the start, 6.25 / 6.90 / 7.03 at
  the end.
- Wrong exact claims: 0 on every tune package, as before.

## Mutation checks

| mutation | caught by |
|---|---|
| The stitch accepts a graph with a budget event | `the_stitch_refuses_a_graph_its_budget_cut_short` fails |
| The stitch drops the import links | `stitched_package_graphs_answer_as_the_whole_program` and `the_stitch_keeps_every_edge_a_graph_found_alone` fail |
| Run-local ids in the stored constraints | `the_graph_names_no_run_local_id` and `the_graph_does_not_depend_on_the_thread_count` (not mutated; they compare the graph's text and bytes) |
| Skip re-checking `prototype_closed`; seed from stored sets | not applicable: the stitch carries no claim and stores no set |
| Store the copy closure | not applicable: the graph does not hold it |
