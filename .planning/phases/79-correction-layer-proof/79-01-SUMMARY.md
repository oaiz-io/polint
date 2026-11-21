---
phase: 79-correction-layer-proof
plan: 01
subsystem: package-graph
requirements-completed: [PG-10]
status: executed-with-deviations
---

# The correction layer, proven with recorded calls

A package graph now has a correction layer beside it. A correction names the
package graph it was written against, an anchor (a function, or a call site in
a function, each with the hash of the function's text), an operation (add an
edge, reject an edge, mark a site complete, a boundary fact), its evidence
(observed with the run and driver, a test, or a review), the stage that wrote
it and the engine version. `observed_corrections` turns a recorder's
`*.calls.json` into corrections; `apply` lays them beside a finished graph and
reports each one as applied, derived (the engine's answer already holds the
edge) or stale (its function, site or target is no longer in the graph with
the same text). The engine never reads the layer, and every edge it adds is
conservative with evidence `observed`.

Loaded with each tune package's own recording, the layer reaches every placed
call and callback edge. That is 100% by construction and is reported apart
from the accuracy numbers, which did not move.

This closes iteration 1.

## Commits

| commit | change |
|---|---|
| 77149ac9 | The proof as tests, failing: recorded edges as corrections beside a package graph |
| bb28285a | The correction layer: format, loader, apply, stale rule |
| 420a07e9 | The bench dump applies a package's recorded calls as corrections and writes the result beside the graph |
| 676de2f4 | `report.py`: a Corrections section apart from the accuracy numbers |
| d5a38243 | The report matches a correction's target by span |
| 5fa6e3a6 | README: the package graph and corrections flags |
| 353ddae0 | Baseline with the corrections section |
| 23286990 | Dense-id sweep re-baselined for the new code |

## Results

Baseline `tests/js-analysis-bench/baseline/` at 353ddae0, TypeScript 5.9.3,
`POLINT_JS_BENCH_PACKAGE_GRAPH=1 POLINT_JS_BENCH_CORRECTIONS=1`, refined plan.

| package | corrections | applied | derived | stale | unanchored (no call expression) | recall with corrections |
|---|---|---|---|---|---|---|
| async | 637 | 212 | 425 | 0 | 4 | 637/637 (100%) |
| axios | 675 | 155 | 520 | 0 | 6 | 675/675 (100%) |
| chalk | 72 | 17 | 55 | 0 | 1 | 72/72 (100%) |
| commander | 318 | 282 | 36 | 0 | 0 | 318/318 (100%) |
| dayjs | 112 | 26 | 86 | 0 | 12 | 112/112 (100%) |
| underscore | 260 | 26 | 234 | 0 | 2 | 260/260 (100%) |
| zod | 1308 | 836 | 472 | 0 | 18 | 1308/1308 (100%) |

An unanchored edge was recorded where the graph has no call expression (a
call the recorder saw without one at the site). Wrong exact claims stay 0 on
every tune package: the layer adds conservative edges only and the accuracy
sections are unchanged.

Commander's derived count is 36: `derived` compares with the points-to answer
in the package graph only, while the refined recall of 183/318 in the
accuracy section scores the dump's full call answers.

Cost of the baseline run: load average 7.51 / 8.55 / 8.24 at the start and
12.98 / 14.61 / 11.16 at the end. Stitch, whole / package graphs / stitch ms:
axios 4059 / 4978 / 365 (9%), express 8831 / 10527 / 715 (8%). The corrections
add no solve; the dump reads the recording and applies it after export.

## Deviations from the plan

- **Where the tests are.** The proof is a lib test module
  (`ts/package_graph_corrections_tests.rs`), not
  `crates/polint/tests/package_graph_corrections.rs`: it needs crate-private
  items, and an integration test would widen visibility. The fixture tests run
  by default; the commander proof reads fetched bench source and is an ignored
  test run explicitly with
  `cargo test -p polint --lib package_graph_corrections -- --include-ignored`.
- **`format.rs` and `bench.py` are untouched.** The identities in `format.rs`
  were enough to anchor; `report.py` runs the dump with the flag.
- **The corrected graph is the added edges, not the graph again.** The dump
  writes `<out>.corrections.json` with the apply report and the added edges
  beside the package graph, which stays byte-identical with and without the
  layer.
- **An edit stales the enclosing functions too.** A function's body hash covers
  its nested functions, so editing one function stales the corrections of
  every function around it. Functions with duplicate text that move also go
  stale. A hash of a function's own text, without its nested functions, would
  localise this; that is a question for the next iteration.
- **Only edges are carried out.** `RejectEdge`, `MarkComplete` and
  `AddBoundaryFact` are recorded and counted as not applied; no stage writes
  them yet.
- **The stitch does not carry corrected edges yet.** Corrections apply to one
  package graph before stitching; carrying them through the stitch waits for
  a stage that writes cross-package corrections.
- **Determinism** was checked with one no-cost run against the full run with
  cost (`report.json` equal apart from `cost`, `dump_cost` and
  `stitch_cost`; every package graph `.gz` byte-identical across runs),
  instead of two no-cost runs. The accuracy sections were compared with the
  committed baseline, generated without the flag: equal apart from the new
  section and timing columns.

## Verification

At 353ddae0 and 23286990, `GOTOOLCHAIN=go1.25.12`. Load average 10.70 /
13.96 / 11.04 at the start, 6.84 / 10.11 / 10.50 at the end.

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: clean.
- `cargo test --workspace --locked --no-fail-fast`: all pass but
  `dense_id_sweep_matches_baseline`, which failed on twelve new hits; each is
  classified in 23286990 and none admits a dense id into stored output. After
  the re-baseline, `internal_architecture` passes (10). polint lib: 3064
  passed, 24 ignored.
- `cargo test -p polint --test public_surface_leak --locked`: 8 passed.
- `cargo test -p polint --lib package_graph_corrections -- --include-ignored`:
  8 passed. The commander proof: 318 corrections, none unanchored; applied
  282, derived 36, stale 0. After one statement is appended to the function
  at `lib/command.js@22383..23407`: applied 268, derived 33, stale 17, the
  corrections of that function and of the functions around it.
- The bench, `POLINT_JS_BENCH_CORRECTIONS=1 POLINT_JS_BENCH_PACKAGE_GRAPH=1`:
  a no-cost run and the baseline run with cost give the same `report.json`
  apart from cost keys; against the committed baseline, generated without the
  flag, every accuracy number is equal.
- Wrong exact claims: 0 on every tune package, as before.

## Mutation checks

| mutation | caught by |
|---|---|
| Apply a stale correction | 3 tests fail |
| Give an added edge tier exact | `recorded_edges_complete_the_graph_beside_the_engine_answers` fails |
| The engine reads corrections | `the_engine_never_reads_the_correction_layer` fails |
| Anchor by span only, without the body hash | `an_edit_of_the_same_length_stales_the_corrections_aimed_at_it` fails |

## The next question

Which cause buckets remain, as the input to stage 2. The edges the layer
applied are those the package graph's answers do not hold; the report's
"Missed edges ranked by gap" names their causes. Under the refined plan:
other member calls on object-literal functions and namespaces (163), prototype
and class methods including inheritance (139), other resolution misses (36),
callbacks invoked by built-ins (32), computed-key member calls (17), class
constructors (2). Stage 2 records these edges; the buckets that keep
needing corrections are where stage 1 improves first.
