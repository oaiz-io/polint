# JS/TS analysis benchmark

Measures how much of a JavaScript/TypeScript program's call structure polint's
facts recover: per construct against hand labels, and per real package against
calls observed while the package runs. It sits beside the golden corpus
(`tests/golden-corpus/`) and reuses its cost records and the matrix runner's
process measurement (`scripts/bench-runner/bench_matrix.py`).

The committed baseline is `baseline/report.json` (complete, deterministic
except for the `cost` section) and `baseline/report.md` (short tables).

There is no single overall score. Every number is per construct, per case or
per package, and each one says what it is measured against.

## Running it

```sh
make js-bench-fetch            # once: download and verify the pinned tarballs
make js-bench                  # analyse, score, measure cost, write baseline/
make js-bench JS_BENCH_ARGS="--no-cost --out /tmp/js-bench"   # scores only
```

`make js-bench` is offline after the fetch. It builds the release `polint`
binary and the polint library test binary, so expect a release build the first
time. `--dump-timeout` (default 600 s) bounds each fact dump; `--cost-runs`
(default 3) sets the number of measured cold and warm runs. A package's
`dump_budget_s` in `inputs.toml` is a time budget for each of its fact dumps:
the report shows it. `bench.py report` still writes the report but exits with
status 1 when any fact dump fails, times out, discovers no files, or runs over
its package's budget.

Recording observed calls executes third-party code and is a separate command:

```sh
make js-bench-record-observed  # needs docker; writes observed/
```

It runs every case and package inside the pinned `node` image with
`--network none`, no credentials or environment passed through, a read-only
root, the inputs mounted read-only and every capability dropped. It is never
part of `cargo test`; its output under `observed/` is committed so the report
never needs to run it.

Each package is recorded twice, and nothing is written unless the two
recordings are byte-identical. The recorder lists as failures a driver that
threw and a selected file it could not instrument; a recording with any
failure is refused, not written, so a partial run cannot pass for a complete
one. Drivers seed every source of randomness they use for the same reason.

A package recorded through its own test suite (`tests_from`) needs the
suite's archive and its modules. `bench.py fetch` downloads the archive, checks
its `sha256`, unpacks the listed paths, and installs the modules with `npm ci
--ignore-scripts` from the committed manifest and lockfile, in the pinned image
with network: npm only downloads and unpacks there, and no package code runs
until the recorder runs it without network. The lockfile is written with
`python3 tests/js-analysis-bench/bench.py lock-test-modules --only <package>`
when the manifest changes.

## Inputs

- `constructs/<construct>/<case>/`: small hand-labelled programs, two per
  construct, with `case.toml` naming the construct, whether the case is held
  out, and a description. Held-out cases (`held_out = true`, at least a quarter)
  are marked in every per-case row and counted in each construct row, so
  a change tuned on the others can be checked against cases it was not tuned
  on. The report also checks that each case's labels agree with the calls the
  recorder observed when the case runs.
- `inputs.toml`: the real packages, each pinned by exact tarball URL, the
  tarball's `sha512` integrity and `sha256`, with format, licence and origin.
  `analysed` selects the files polint analyses and the recorder instruments.
  **The analysed set of a package is its shipped source, not its test files**:
  one rule for every package, tune set and held-out set alike. Paths under a
  `test`, `tests`, `__tests__`, `benchmark` or `benchmarks` folder and files
  named `*.test.*` or `*.spec.*` are left out even when the tarball carries
  them (zod's does). The library never imports them and no driver runs them,
  so the rule changes what analysis costs, not what the run observed: zod's
  recording has no edge in those paths. One report, the first written with the
  rule, also scores zod on its globs alone (`compare_previous_set`), apart from
  every other table, so it reads against earlier reports.
  The set covers ESM source (axios, chalk), TypeScript source (zod), CommonJS
  only (commander), CommonJS transpiled from ESM (async), minified UMD (dayjs)
  and a bundled UMD (underscore).
- The held-out packages (`held_out = true` in `inputs.toml`,
  `research/package-graphs/HELD-OUT.md`): acorn, co, express, immer, js-yaml,
  nanostores, node-forge and pako. No analysis change is tuned on them. Their
  drivers under `observed/drivers/heldout/` are written from each package's
  documentation, or run the package's own test suite (`tests_from`: express,
  js-yaml, node-forge), and are committed apart from analysis code; the
  `held-out authorship` CI job runs `held_out_guard.py` on a pull request's
  changed paths and refuses one that changes both
  `crates/polint/src/{ts,analysis*,graph}` and a held-out file (anything under
  `observed/drivers/heldout/`, or an `observed/` file named after a held-out
  package). `corpus` gives a parser's driver its input: the shipped JavaScript
  of the named packages, copied into the tree in sorted order.
- `observed/`: the recorder (`recorder.mjs`), one driver per package that
  exercises its public API, and the recorded `*.calls.json` (schema
  `polint-js-bench-observed-calls-2`, with the functions the run entered) and
  `*.exports.json`. `observed/stubs/node_modules/` holds placeholder modules
  for axios's Node-adapter dependencies (the driver never sends a request):
  `form-data` is Node's own `FormData` and `follow-redirects` Node's `http` and
  `https`. They are committed through an exception to the root ignore of
  `node_modules/`. The recorder strips TypeScript types in place; it blanks
  value-import names that only a type declares, and a file that needs code
  generation (enums, namespaces, parameter properties) runs uninstrumented
  and is listed under `transformed_not_instrumented`.

### Label grammar

Labels are trailing line comments in case sources:

| label | meaning |
|---|---|
| `// @fn NAME` | a function starts on this line; `NAME` is how other labels refer to it |
| `// @calls NAME ...` | a call expression starting on this line invokes `NAME` |
| `// @callback NAME ...` | a function passed on this line is invoked by the callee (a runtime builtin or another function) |
| `// @unknown` | the path from this line is hidden from static analysis (computed key from input, eval, computed import specifier); a complete answer here is wrong |
| `// @external` | the line calls code outside the case (builtins); not scored |
| `// @exports NAME ...` | first line only: the names the module exports |

Several directives may share a line (`@callback isEven arrow @fn arrow @calls isEven`).

## Measures

- **Edge recall** per case and construct: labelled `@calls` and `@callback`
  edges polint finds, keyed by (file, line, callee). For packages, recall is
  over observed calls and callbacks.
- **Precision** against hand labels only. Observed calls show what one run
  did; they give recall, never precision, so no package precision is reported.
- **False-complete count** (required): `@unknown` lines where polint gives
  resolved targets and no unresolved marker. `reported-unknown` and `silent`
  lines are counted next to it. Labelled `@calls` lines count apart as
  false-complete call lines when polint resolves them to targets that miss a
  labelled callee and marks no call site on the line unresolved.
- **Wrong targets** (packages): claimed targets (resolved, not heuristic) at
  a call site the run was seen calling from, in an instrumented file, that the
  run never called from that site. On a single-target answer this is a false
  precise edge: polint names one callee and every observed execution of the
  site went elsewhere. On a multi-target answer it is only an upper bound,
  because the run may not have taken the path to the other targets. A site
  counts as executed only when one of its observed callees maps to a polint
  function, so sites that only reached code the run does not record are left
  out. Every single-target case is listed in `report.md`.
- **Export-set accuracy**: per file, exported names from `Symbols` and from
  the module export facts against the keys of the module as Node loads it.
  A file that fails to load on its own (node-forge's jQuery form helper and
  its web-worker script, for example) has no keys to compare; it is left out
  and counted under `load errors`.
- **Unresolved share**: share of call sites with an unresolved marker, and of
  references with no resolved target.
- **Why missed** (packages): each observed edge polint does not find is put in
  one bucket, such as no matching call site, no matching function, a target
  that stops at an import binding, or other targets with or without an
  unresolved marker. Each bucket keeps a bounded sample of examples (site,
  observed callee, what polint answered); every edge with other targets and no
  unresolved marker is kept and listed in `report.md`.
- **Missed edges by gap** (packages): each missed edge is also attributed to
  one extraction or resolution gap (unary IIFEs, functions in try / catch / if
  blocks, exported initializers, constructors, callbacks to built-ins,
  computed keys, prototype and class methods) or a named remainder, using a
  syntax census of the same files. The ranking gives each gap's count and an
  upper bound on the recall it could add. Gap conditions are read from syntax
  alone, so the attribution says where a miss sits, not that fixing that gap
  alone would find it.
- **Tiers**: what each edge claims, from its precision
  (`research/package-graphs/TIERS.md`). `Exact` and `SetupAware` are
  **exact**; `Conservative` and `Ambiguous` are **conservative**; `Heuristic`
  is **apart**; `Unknown` and `Unsupported` claim nothing. A typed-hierarchy
  edge whose evidence says it dispatches to an `implementation`, a
  `union_member` or an `inferred` target names a candidate, not the target;
  the engine tiers it conservative, and the report stops on one that claims
  exact. A refined row
  claims only when its status is `Resolved` or `Ambiguous`, and an `Ambiguous`
  row is never exact. The dump carries the engine's tier for every edge
  (`claim_tier`) and the report stops when its own list disagrees, so a new
  precision cannot be scored without a decision on both sides.
  Recall per tier is over the same observed edges as recall: **exact-complete**
  (an exact edge found the callee and the site is complete: at least one exact
  or conservative target and no unresolved row), **exact**, **exact plus
  conservative**, and **apart only** (found by a heuristic edge alone, which
  no other view credits). The older recall column counts every edge with a
  target, heuristic ones included, and is kept for comparison with earlier
  reports.
- **Precision by tier**: at executed sites, as for wrong targets, the share of
  each tier's claimed targets the run called there. It is a lower bound: a run
  takes some paths only. A single exact target the run never called there is
  a **wrong exact target**; the target is 0 on every plan, and every case is
  listed.
- **Targets per site**: mean, nearest-rank p95 and maximum over the sites that
  claim any target in the tier; how wide a tier's answers are.
- **Open sites by reason**: every site that is not complete, counted under each
  reason its answer gives (the refined row's reason or status, else the
  unresolved row's). `none recorded` is an open site whose answer gives no
  reason.
- **Budget events**: the solver's budget status per dump and every budget it
  reported as reached, such as `points_to.max_dynamic_vars`.
- **Refined edges**: refined rows counted by bench tier, algorithm, status,
  precision and whether they name a function.
- **Sites gained by the refined plan**: sites where a refined plan claims an
  exact or conservative target and the base plan claims none, by file, with
  whether the file is a test or benchmark path and how many of the gained sites
  the run executed.
- `bench.py selftest` checks these measures on a synthetic package without
  fetching, building or sandboxing anything: a site only a heuristic edge
  answers counts as apart only, a typed candidate counts as conservative, a
  single exact target the run did not call is a wrong exact target, a budget
  reason is reported, and every tier section is rendered. It also checks the
  outside-tool scorer: position conversion on a line with non-ASCII and astral
  characters before a call, and that it matches an observed edge to the same
  site and callee as polint's scorer. It checks driver coverage, that a
  held-out package's misses lose their causes and examples, that the gate
  leaves a flagged package out of recall but counts its wrong exact targets,
  that recall under a recall target is reported as not met, and the
  authorship guard's classification of changed paths.
- **Driver coverage** (packages): the functions the recording entered, over
  the census's functions in the analysed files (classes and static blocks
  left out, since no call enters them). An entered function counts once, as
  the innermost census function ending at the same byte. It says how much of
  a package the recall speaks for.
- **Held-out packages**: `## Held-out packages` scores the held-out set with
  the tune set's measures, apart from it, and with no gap ranking, no cause
  and no example of a missed edge, so nothing in the report invites tuning on
  them. Wrong exact targets and answers with other targets and no unresolved
  marker keep their examples: either is a defect to fix. A package whose
  recording entered under 40% of its functions is flagged and left out of
  held-out recall. The **held-out gate** pools recall per tier over the tune
  set and over the unflagged held-out packages; it passes on a plan when
  every held-out tier is within 5 points of the tune set and no held-out
  package, flagged or not, has a wrong exact target or a false-complete
  answer. The tune-set tables, cost and outside tools cover the tune set only.
- **Recall targets**: `## Recall targets` compares the pooled tune-set recall of
  each plan and tier against the targets of
  `research/package-graphs/ITERATION-1-CLASSICAL.md`, and each tune package's
  no-types recall against the per-package target. It is a report: a target
  not met is a row marked `no` and does not fail the run.
- **Cost**: `polint check` with the census rule pack in `rule-pack/` (asks for
  `Functions`, `CallGraph`, `Symbols`, `References`), cold and warm, wall time
  and peak RSS from the in-process cost record (`POLINT_GOLDEN_COST_PATH`) and
  the whole process, stored cache bytes raw and gzip, all also per MB of
  source. The fact dumps' time and peak memory are reported as well.
  Process peak RSS comes from `wait4`, which on Linux starts a child's peak at
  the launcher's resident size, so the report starts every measured process
  before it loads any fact dump and records the launcher's own peak beside the
  numbers. The in-process analysis peak has no such floor.

Two columns are reported throughout:

- **base**: call-graph targets (`syntax,call_graph,symbols,references`).
- **refined**: the refined call facts that the deep analysis plan
  (`syntax,calls,control_flow,dataflow,call_graph,symbols,references`)
  produces. Rules have no public view of these facts. The benchmark reads them
  through an internal hook and adds no public view.

## Outside tools

`## Outside tools` puts [Jelly](https://github.com/cs-au-dk/jelly) beside
polint, scored on the same observed calls with the same site and callee
matching (`report.Spans`, `report.unobserved_targets`; the scorer imports them
rather than copying them). Jelly runs twice per tune-set package, static and
with `--approx`, on the same shipped source files polint analyses. Held-out
packages are not run.

```sh
python3 tests/js-analysis-bench/bench.py outside [--only <package>]  # needs docker and network for the build
```

`bench.py outside` builds Jelly at a pinned commit from a hash-checked archive
into an image on the pinned `node` base (`outside/Dockerfile.jelly`; the network
is used only while building the image), then runs it in the same sandbox as the
recorder, with `--network none`. Its raw call graphs are committed gzip-compressed
under `baseline/outside/<run>/<package>.json.gz`, with a `<package>.meta.json`
recording the image, arguments, exit code and wall time, so `bench.py report`
scores them without docker. `outside/score_outside.py` converts Jelly's
positions (1-based lines, 1-based end-exclusive UTF-16 columns) to the byte
offsets every bench key uses.

Jelly has no tiers, so every Jelly edge counts as claimed and its wrong-target
counts are an upper bound; the report lists this and the other deviations from
polint's scoring under the table. ACG (js-callgraph) is not run: its parser
rejects ES2020 and drops such files silently.

## The internal hook

`crates/polint-eval/src/harness/js_bench.rs` (compiled into the `polint`
library as `crate::eval::js_bench`, test-only) holds one ignored test,
`dump_js_bench_facts`. It analyses each tree listed in
`POLINT_JS_BENCH_MANIFEST` with the capabilities in `POLINT_JS_BENCH_CAPS` and
writes the facts as JSON (`polint-js-bench-facts-3`: every edge row carries
its `claim_tier`, refined rows their `reason` and `dispatch` evidence, and the
document the solver's `solver_budget`). It only reads facts and is
not part of any public surface. `report.py` runs it through the test binary.

With `POLINT_JS_BENCH_CENSUS=1` the dump also carries a syntax census: every
function and call expression Oxc parses in each file, with its syntactic
context (under a unary operator, in a try / catch / finally / if block, in an
export, a callee or argument, a prototype assignment) and enclosing class. The
census parses each file again on its own, reads no analysis store and feeds
nothing back. The report runs it as a separate dump so the timed dumps are
unchanged.

With `POLINT_JS_BENCH_PACKAGE_GRAPH=1` the refined dump of each tune-set
package also writes its package graph; the report stores it compressed and
states its size, and stitches the trees in `inputs.toml`.

With `POLINT_JS_BENCH_CORRECTIONS=1` the refined dump of each tune-set package
loads the package's own recorded calls as corrections to its package graph,
applies them after the export and writes what became of each. The report's
`## Corrections` section counts them: applied, derived, stale and unanchored.
Its recall with corrections is 100% by construction, because the corrections
come from the recording it is checked against; it proves the correction layer
and is not an accuracy number. Neither flag changes any other section.

## Limits

- Labels are per line, so two sites on one line calling the same callee count
  once. See the decisions in the report.
- Observed calls cover only what the drivers exercise, so package recall
  speaks for the exercised paths only; unexercised code adds nothing to the
  denominator.
- Implicit observed edges (accessors, coercion, iterator protocol) have no
  call expression. They are listed by kind but left out of headline recall.
- Four zod files and one immer file need TypeScript transforms (not just type
  stripping) and run without instrumentation, so calls into them are not
  observed.
- Wall times come from whatever machine runs the report, and on a shared
  machine they move with its load: the committed baseline ran beside other
  jobs, and the small trees' cold times about doubled from one full run to
  the next. Compare times only between runs on the same quiet machine; peak
  RSS, stored bytes and every score are stable.
- A package is analysed as the package it is: the report writes a
  `.polint.toml` into each staged tree whose `[workspace] include` is the
  package's `analysed` globs and whose `exclude` keeps only
  `**/node_modules/**` and `**/.git/**`, so a package's own `vendor/` (chalk's
  `source/vendor/`) is analysed. A repository keeps polint's default excludes.

## Findings in the baseline

These are found by the benchmark and are not changed by it:

- Every function and call site Oxc parses in the tune set is extracted:
  the report's extraction gaps (unary IIFEs, functions in try / catch / if
  blocks, calls in exported initialisers, files not analysed, other
  extraction) are 0 on every tune package.
- Member calls on object-literal functions and namespace imports are not
  resolved. This is the largest single gap on the real packages: 730 of 1814
  missed base edges, 306 of them on zod.
- Of the base misses that are not named resolution gaps, most are identifier
  calls to an arrow function (247) and `(0, f)(x)` calls to a declaration
  (134). All but at most one of the latter are async's compiled CommonJS
  interop: the sites are extracted with the last operand as callee and miss
  for the reason the operand's own call would.
- A class is one function spanning the class; there is no constructor function.
- On underscore both plans mark `iteratee(...)` inside `map` unresolved:
  `iteratee = cb(iteratee, context)` can return closures the refined walk does
  not follow, so it does not report `identity`, the one return it can name, as
  the complete answer.
- On axios the refined plan resolves no points-to edges, so refined recall
  equals base recall there.
- `.polint/cache` holds tens to hundreds of MB per MB of source after a
  cold run, mostly layer blobs.
- On the shipped set the refined plan's dump on zod takes about 11 s and
  1.0 GB, against about 4 s and 0.5 GB for the base plan, within its 180 s
  budget; `semantic_graph` is about 30% of its summed stage time. Refined
  recall on zod equals base recall (721 of 1308 observed edges).
- Sites the base plan stops at an import binding (1 on axios, 1 on zod)
  have no target and no unresolved marker on the refined plan.
