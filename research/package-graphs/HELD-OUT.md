# The held-out set

Date: 2026-11-17. Decision for WP0; nothing here is recorded yet.

Research question: which packages form the held-out set of the JS/TS
call-graph bench, pinned, and how are their observed calls recorded so that
no change is tuned on them?

## The set

Eight packages, none in the tune set or in its dependency closure (axios's
`follow-redirects`, `form-data` and its five dependencies, `proxy-from-env`;
the other six tune-set packages have no dependencies). Six are in the Jelly
PLDI 2024 benchmark (so a published dynamic call graph exists for a nearby
version); the ESM-source and TypeScript-source slots are not, because nothing
in that benchmark is ESM-first or ships `.ts` in its tarball.

| # | package | format slot | analysed | size | deps | licence | Jelly 36 | observed through |
|---|---|---|---|---|---|---|---|---|
| 1 | `co@4.6.0` | CommonJS, generator and promise control flow | `index.js` | 5 KB, 1 file | 0 | MIT | yes | driver |
| 2 | `pako@2.1.0` | minified UMD (Rollup + terser) | `dist/pako.min.js` | 47 KB, 1 file | 0 | MIT AND Zlib | yes | driver, small inputs |
| 3 | `nanostores@1.5.5` | ESM source, `"type": "module"` | `*/index.js`, `deep-map/path.js` | 55 KB, 14 files | 0 | MIT | no | driver |
| 4 | `express@5.3.0` | CommonJS framework, 28 dependencies | `index.js`, `lib/*.js` | 62 KB, 7 files | 28 | MIT | yes | own mocha/supertest suite over loopback, or `app.handle(req, res)` in process |
| 5 | `immer@11.2.10` | TypeScript source in the tarball | `src/**/*.ts` | 95 KB, 16 files | 0 | MIT | no | driver |
| 6 | `js-yaml@4.1.0` | CommonJS source, table-driven schema compilation | `index.js`, `lib/**/*.js` | 108 KB, 25 files | 1 (CLI only, not analysed) | MIT | yes | own mocha suite, or driver |
| 7 | `acorn@8.19.0` | bundled ESM output | `dist/acorn.mjs` | 235 KB, 1 file | 0 | MIT | yes | corpus driver (parse a fixed corpus) |
| 8 | `node-forge@1.4.3` | CommonJS, large | `lib/*.js` | 972 KB, 49 files | 0 | BSD-3-Clause (OR GPL-2.0; take BSD) | yes | own mocha suite with fixed keys, or driver |

Pins (registry metadata read 2026-11-17; `sha256` is computed by
`bench.py fetch` on first download, as for the tune set):

| package | tarball | `dist.integrity` |
|---|---|---|
| co 4.6.0 | https://registry.npmjs.org/co/-/co-4.6.0.tgz | `sha512-QVb0dM5HvG+uaxitm8wONl7jltx8dqhfU33DcqtOZcLSVIKSDDLDi7+0LbAKiyI8hD9u42m2YxXSkMGWThaecQ==` |
| pako 2.1.0 | https://registry.npmjs.org/pako/-/pako-2.1.0.tgz | `sha512-w+eufiZ1WuJYgPXbV/PO3NCMEc3xqylkKHzp8bxp1uW4qaSNQUkwmLLEc3kKsfz8lpV1F8Ht3U1Cm+9Srog2ug==` |
| nanostores 1.5.5 | https://registry.npmjs.org/nanostores/-/nanostores-1.5.5.tgz | `sha512-FixtE239Gl6Gz0ZGCK0WzvNwObNCIwVT+L7zQcf1QgcA6AzBxblzqe9OT88qiucNkhsjSOqiogSaF5jVbJiqSA==` |
| express 5.3.0 | https://registry.npmjs.org/express/-/express-5.3.0.tgz | `sha512-qCqy1xSoaugxOD4IVHb1ZWdkvCF2W3aE2okKouWj8VwJ+ztskSiXoBnCSJqfKT+cVPJhuJqe9YLbY425+Zh00g==` |
| immer 11.2.10 | https://registry.npmjs.org/immer/-/immer-11.2.10.tgz | `sha512-pzANBj+0A9u+prdhBJ14DVi44vCvVUQzBa8DKt6I0AMCokPHtt722BK9IX5bFC70xUSU9TnzLRIJLTGqIzjEFw==` |
| js-yaml 4.1.0 | https://registry.npmjs.org/js-yaml/-/js-yaml-4.1.0.tgz | `sha512-wpxZs9NoxZaJESJGIZTyDEaYpl0FKSA+FB9aJiyemKhMwkxQg63h4T1KJgUGHpTqPDNRcmmYLugrRjJlBtWvRA==` |
| acorn 8.19.0 | https://registry.npmjs.org/acorn/-/acorn-8.19.0.tgz | `sha512-oJlA3XiRm7Cyk6qFD2Jn8ak9B7jSy0qy00ADO3+8dpT0LSjFihQYv4C02LFCSYJV3Q37xYwwRy5m+IpIiUzqWw==` |
| node-forge 1.4.3 | https://registry.npmjs.org/node-forge/-/node-forge-1.4.3.tgz | `sha512-P6wzfh6t70avmV8FrVEAd/36sOPJQ+hkaJVX1Laap9J/77m7M0eyozDvc8SwnDf4P7MP94gwbAAo0rg3YVXT+g==` |

Why each, in one line: co is the smallest member of the Jelly benchmark and
all of it is generators, promises and thunk dispatch; pako is a second
minifier's output (terser over Rollup) beside dayjs, the "engine for two"
rule for the unary-IIFE extraction path; nanostores is idiomatic modern ESM
with closures returned from factories and listener arrays, the "identifier
call to an arrow" miss family; express is the dependency-volume and
framework stratum, whose call structure is built by `methods.forEach(m =>
app[m] = …)`, router delegation and `setPrototypeOf`; immer ships `src/*.ts`
like zod, so the TypeScript slot keeps the registry-tarball pinning model,
with Proxy traps and plugin tables; js-yaml compiles schema and type tables
at load time, the approximate-interpretation paper's own theme; acorn is a
class-heavy bundled ESM file (`pp.method = function`) with near-complete
dynamic coverage; node-forge is the one-megabyte stratum, 49 modules wiring
themselves onto a shared `forge` object.

Caveats recorded with the choice: immer's `src/types/types-internal.ts` has a
`const enum` that type stripping cannot handle (the recorder already leaves
such files uninstrumented for zod; this one is type-only); express's own
suite needs loopback HTTP, which docker `--network none` keeps, to be
verified once in the sandbox; node-forge drivers must use fixed keys and
leave `http.js`, `socket.js`, `xhr.js`, `prime.worker.js` unexercised;
express 5.3's router is a separate package, so edges into it are external.

Alternates if a slot is rejected: `express-validator@7.3.2` (TypeScript
source, Jelly benchmark, but only as a GitHub tag archive, not a registry
tarball), `big.js@7.0.1` (unminified UMD, dependency-free test runner),
`deep-diff@1.0.2` and `pubsub-js@1.9.5` (UMD, tests ship in the tarball),
`passport@0.7.0`.

Rejected from the Jelly benchmark and why: `xss` (depends on commander),
`js-git` (on pako), `tern` (on acorn), `nedb` (on underscore and async),
`kraken-js` (Apache-2.0), `is-thirteen` (licence), `typedi` (decorators need
transforms), `xml2js` (compiled CoffeeScript), the HTTP-server-centred ones
(`cors`, `body-parser`, `multer`, `compression`, `connect`, `supertest`) as
lower value than express for the same stratum, and the rest for size or
fit.

## Recording observed calls so nobody tunes on them

1. One recording, committed, reused. `make js-bench-record-observed` in the
   existing sandbox (pinned Node image, `--network none`, read-only root, no
   environment) produces `*.calls.json` and `*.exports.json` once; reports
   never re-run it. A re-recording needs a changed pin, recorder or image,
   and is its own commit touching no analysis code.
2. Separation of authorship. The driver, or the decision to run the package's
   own suite, and the first recording are committed before the analysis
   change starts, in their own pull request. A CI check refuses a pull
   request that touches both `crates/polint/src/{ts,analysis*,graph}` and
   held-out files under `tests/js-analysis-bench/observed/` that already
   exist on the base branch: changing, renaming or deleting a held-out file
   goes in its own pull request. A first add of a held-out file is not
   refused, so a pull request that lands a stack can carry held-out files
   committed apart from the analysis change, together with that change. The
   check cannot tell whether a first add was written apart; the authors still
   must. (Set by the owner on 2026-11-19.)
3. Drivers come from the package's documentation and public API, never from
   polint's miss buckets. A driver may be extended only to raise coverage of
   the package, as a separate observed-file commit with the coverage number
   before and after.
4. Prefer the package's own test suite where it runs offline (express,
   js-yaml, node-forge; deep-diff and pubsub-js among the alternates). The
   test tree comes from the repository at the release tag with a vendored
   `node_modules` from its lockfile, pinned by commit and content hash in
   `inputs.toml` beside the tarball pin, mounted read-only. The recorder's
   rule that a call whose stack never reaches an instrumented file is an
   entry, not an edge, keeps mocha and supertest frames out, as Jelly's
   `TEST_PACKAGES` filter does. Hand drivers for co, pako, nanostores, immer
   and acorn, whose suites need vitest, a DOM or fixtures that do not fit the
   sandbox.
5. Driver coverage with every number: functions in the analysed files (from
   the syntax census), functions the run entered, the share, and the observed
   edge count, per package. A held-out package under 40% entered functions is
   reported, flagged, and left out of the held-out gate's average.
6. Determinism: no time, randomness, network, `process.env` or key generation
   in a driver; two runs on the same pin must give a byte-identical observed
   file before it is accepted.
7. The gate as ITERATION-1 states it: held-out recall within 5 points of the
   tune set, per tier, and zero wrong complete answers on the held-out set
   too. A held-out miss that is a general construct becomes a construct
   fixture with its own held-out case, never a recogniser; the package stays
   untouched.
8. Rotation: a held-out package that keeps revealing general gaps may be
   promoted to the tune set and replaced from the alternates, recorded in the
   report's decisions; never tune on it while it still counts as held out.
9. Cross-check only: Jelly's own dynamic call graphs for the six overlapping
   packages, if extracted from the artifact, are a second observed source
   for diagnosing the recorder (edges in one and not the other), never the
   denominator, because the engine (GraalJS/NodeProf vs. Node 24 with source
   rewriting) and the versions differ.

## The Jelly PLDI 2024 artifact

What it is and is not (accessed 2026-11-17):

- Paper: Laursen, Xu, Møller, "Reducing Static Analysis Unsoundness with
  Approximate Interpretation", PACMPL 8 (PLDI), article 194, 2024,
  DOI 10.1145/3656424, https://cs.au.dk/~amoeller/papers/approx/paper.pdf.
- Artifact: one VirtualBox OVA per version on Zenodo, concept DOI
  10.5281/zenodo.10796242; v1 https://zenodo.org/records/10796243 (43.0 GB),
  v3 https://zenodo.org/records/10930752 (48.3 GB, 2024-04-05). Nothing else
  is downloadable: no separate traces, tarballs, benchmark list or scripts.
  No licence field on the record; Jelly itself is BSD-3-Clause. The OVA was
  not downloaded, so the 36 projects' versions and the stored dynamic graphs
  are unconfirmed (confidence low on contents, high on what the record shows).
- The 36 projects with dynamic call graphs (paper, Table 1): co, deep-diff,
  xss, xml2js, js-git, passport, jsinspect, typedi, connect, nedb,
  compression, body-parser, express-validator, is-thirteen, cors, hubot,
  posthtml, kraken-js, supertest, tern, doctoc, gulp, pubsub-js,
  node-schedule, multer, js-yaml, express, node-forge, scrape-it, acorn,
  nexe, json2csv, pako, sharedb, big.js, bootlint. All GitHub repositories,
  nearly all CommonJS; versions are not in the paper. The dynamic graphs
  were produced by running each project's own test suite under Jelly's
  NodeProf instrumentation (`jelly --dynamic <out> --npm-test <dir>`,
  `src/dynamic/dyn.ts`).
- The current Jelly repository (`62e60eb4`, 2026-08-11) ships dynamic graphs
  only for `tests/micro/*.json` (92 small cases) and `tests/mochatest`; none
  for the 36. Its dynamic mode needs GraalVM with NodeProf (Node 16 era) and
  is not what polint's recorder uses.
- A smaller licensed download from the same group exists for the ECOOP 2024
  indirection-bounding paper: https://zenodo.org/records/12720724
  (`benchmarks.tar.gz`, 1.6 GB, CC-BY-4.0, "all benchmarks used for
  experiments"); whether it holds dynamic graphs is not stated.

What this means for OPEN-QUESTIONS Q1: "trust artifact traces for the
baseline" is not available without opening a 43 GB VM image. The held-out
set therefore records its own observed calls with the bench's recorder, and
uses Jelly's graphs, if ever extracted, as the cross-check in rule 9.

## What was not done

No package code was executed, no driver written, no OVA downloaded. The next
step is WP0's bench change: eight `[[package]]` entries with `held_out =
true`, a held-out driver directory under the authorship rule, the coverage
column in `report.py`, and the CI check of rule 2. The working notes behind
this document are in the gitignored
`research/package-graphs/repos/heldout-research/NOTES.md`.
