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
(default 3) sets the number of measured cold and warm runs.

Recording observed calls executes third-party code and is a separate command:

```sh
make js-bench-record-observed  # needs docker; writes observed/
```

It runs every case and package inside the pinned `node` image with
`--network none`, no credentials or environment passed through, a read-only
root, the inputs mounted read-only and every capability dropped. It is never
part of `cargo test`; its output under `observed/` is committed so the report
never needs to run it.

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
  The set covers ESM source (axios, chalk), TypeScript source (zod), CommonJS
  only (commander), CommonJS transpiled from ESM (async), minified UMD (dayjs)
  and a bundled UMD (underscore).
- `observed/`: the recorder (`recorder.mjs`), one driver per package that
  exercises its public API, placeholder modules for axios's Node-adapter
  dependencies (the driver never sends a request), and the recorded
  `*.calls.json` and `*.exports.json`.

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
- **Export-set accuracy**: per file, exported names from `Symbols` and from
  the module export facts against the keys of the module as Node loads it.
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

## The internal hook

`crates/polint-eval/src/harness/js_bench.rs` (compiled into the `polint`
library as `crate::eval::js_bench`, test-only) holds one ignored test,
`dump_js_bench_facts`. It analyses each tree listed in
`POLINT_JS_BENCH_MANIFEST` with the capabilities in `POLINT_JS_BENCH_CAPS` and
writes the facts as JSON (`polint-js-bench-facts-2`). It only reads facts and is
not part of any public surface. `report.py` runs it through the test binary.

With `POLINT_JS_BENCH_CENSUS=1` the dump also carries a syntax census: every
function and call expression Oxc parses in each file, with its syntactic
context (under a unary operator, in a try / catch / finally / if block, in an
export, a callee or argument, a prototype assignment) and enclosing class. The
census parses each file again on its own, reads no analysis store and feeds
nothing back. The report runs it as a separate dump so the timed dumps are
unchanged.

## Limits

- Labels are per line, so two sites on one line calling the same callee count
  once. See the decisions in the report.
- Observed calls cover only what the drivers exercise, so package recall
  speaks for the exercised paths only; unexercised code adds nothing to the
  denominator.
- Implicit observed edges (accessors, coercion, iterator protocol) have no
  call expression. They are listed by kind but left out of headline recall.
- Four zod files need TypeScript transforms (not just type stripping) and run
  without instrumentation, so calls into them are not observed.
- Wall times come from whatever machine runs the report, and on a shared
  machine they move with its load: the committed baseline ran beside other
  jobs, and the small trees' cold times about doubled from one full run to
  the next. Compare times only between runs on the same quiet machine; peak
  RSS, stored bytes and every score are stable.
- Packages use polint's default discovery, so default excludes apply (chalk's
  `source/vendor/` is not analysed).

## Findings in the baseline

These are found by the benchmark and are not changed by it:

- Functions under a unary operator (`!function(){}()`, `void function(){}()`)
  are not extracted, so minified UMD bundles such as dayjs yield almost no
  facts.
- Functions declared inside `try`, `catch` or `if` blocks are not
  extracted, nor are the calls in their bodies. Calls written directly in try,
  finally and if blocks, and functions in bare blocks and loop bodies, are.
- Member calls on object-literal functions and namespace imports are not
  resolved. This is the largest single gap on the real packages: 1053 of 2582
  missed base edges, 724 of them on zod.
- Of the base misses that are not extraction or named resolution gaps, most
  are identifier calls to an arrow function (435) and `(0, f)(x)` calls to
  a declaration (134, async's compiled CommonJS interop).
- A class is one function spanning the class; there is no constructor function.
- On underscore both plans mark `iteratee(...)` inside `map` unresolved:
  `iteratee = cb(iteratee, context)` can return closures the refined walk does
  not follow, so it does not report `identity`, the one return it can name, as
  the complete answer.
- On axios the refined plan resolves no points-to edges, so refined recall
  equals base recall there.
- `.polint/cache` holds tens to hundreds of MB per MB of source after a
  cold run, mostly layer blobs.
- The refined plan's dump on zod runs for more than ten minutes
  single-threaded; it is reported as timed out.
