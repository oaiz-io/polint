# Outside baselines on the same packages, scored by the same oracle

Date: 2026-11-17. Measured, not quoted. Every number here is on the seven
packages of `tests/js-analysis-bench/inputs.toml`, scored against
`tests/js-analysis-bench/observed/packages/<name>.calls.json` (kinds `call`
and `callback`, the bench's headline recall), with the bench's site and
callee matching rules reproduced.

Research question: where do Jelly (static and with approximate
interpretation) and a field-based analysis (ACG) stand on this bench, so the
targets in [ITERATION-1-CLASSICAL.md](ITERATION-1-CLASSICAL.md) compare with
numbers rather than with averages from papers?

## Result

| package | polint base / refined / typed | Jelly static | Jelly `--approx` | ACG ONESHOT |
|---|---|---|---|---|
| async | 51 / 51 / 53 | 55.7 | 55.7 | 74.4 |
| axios | 40 / 40 / 76 | 89.2 | 89.3 | 44.4 |
| chalk | 24 / 24 / 24 | 86.1 | 86.1 | 12.5 |
| commander | 7 / 37 / 78 | 96.5 | 96.5 | 3.1 |
| dayjs | 0 / 0 / 0 | 77.8 | 87.2 | 89.7 |
| underscore | 60 / 60 / 77 | 96.2 | 98.9 | 64.1 |
| zod | 55 / 55 / 58 | 91.3 | 92.3 | fail |
| **total** | **44.5 / 47.5 / 61.4** | **84.5** | **85.4** | 31.5 |

Recall alone flatters the outside tools. The same wrong-target measure the
bench applies to polint (targets claimed at a site the run executed that the
run never called from there; single-target claims are false exact edges,
multi-target claims an upper bound):

| tool | wrong single | wrong multi | note |
|---|---|---|---|
| polint base | 0 | 0 | |
| polint refined | 0 | 0 | |
| polint typed | 15 | 2 | see DIAGNOSIS §4 |
| Jelly static | 13 | 5,811 | async: 14,352 edges for 1,610 sites |
| Jelly `--no-interops` | 13 | 22,916 | async: 66,747 edges |
| ACG ONESHOT | 1 | 30,273 | async: 57,848 edges for 992 sites; zod excluded |

Jelly's 13 wrong single targets are 12 on axios and 1 on underscore. Jelly
and ACG have no notion of a complete answer or of a heuristic edge, so every
edge counts as claimed; that is a deviation listed below, and it makes the
tools' numbers an upper bound.

Wall time (docker wall, container start included): every Jelly run 0.6 to
3 s except zod (static 22.9 s, approx 35.3 s); ACG 0.3 to 10 s. Nothing timed
out; the host limit was 900 s.

## What it means for the targets

- Jelly static at 84.5% is the reference point for the no-types target. It
  reaches it with an inclusion-based analysis that has the call rule, field
  sensitivity, and models for the platform and for Babel interop; it does not
  use types. polint's refined layer, with its points-to solve discarded on
  three of these packages ([DIAGNOSIS.md](DIAGNOSIS.md) §1), is not a
  comparable configuration; the 47.5% is the walker's effect on commander.
- Jelly pays for recall with width: 9 targets per site on async, 5,811
  multi-target claims the run never confirmed. polint's precision measures
  (wrong single 0, executed-site precision, targets per site) are the
  differentiator and must be reported beside recall in every bench run.
- Approximate interpretation adds little on this set (85.4 vs 84.5; dayjs and
  underscore gain, the rest within one edge). Stage 2 should not be expected
  to recover more than a few points on packages like these; its value is on
  the dynamic patterns the paper names.
- ACG (the 2013 field-based analysis as implemented in js-callgraph) cannot
  parse ES2020 and drops files silently; it is not a usable baseline on
  modern packages. Its field-based idea stays relevant as polint's
  conservative fallback; its tool does not.
- Jelly's zod call graph is 2.3 MB raw, 0.57 MB gzip for 3.1 MB of source
  (8,439 functions, 50,062 calls, 18,997 edges): 0.18 MB gzip per MB of
  source for a plain call graph. A reference for the size target in
  [TARGETS.md](TARGETS.md).

## Per-run tables

Columns: `observed` oracle edges; `found`; `functions`/`calls`/`edges` the
tool's own counts (Jelly: functions without the per-file module entries,
`call2fun` edges to functions); `wrong single`/`multi` as above;
`executed` sites the oracle matched a site and callee for; `wall s`.

### Jelly static (`-j out.json -b <pkg> -i 840 --ignore-unresolved`)

| package | observed | found | recall % | functions | calls | edges | wrong single | wrong multi | executed | wall s |
|---|---|---|---|---|---|---|---|---|---|---|
| axios | 675 | 602 | 89.2 | 535 | 2281 | 1448 | 12 | 78 | 620 | 2.0 |
| zod | 1308 | 1194 | 91.3 | 8439 | 50062 | 18997 | 0 | 1720 | 1075 | 22.9 |
| commander | 318 | 307 | 96.5 | 296 | 814 | 517 | 0 | 4 | 311 | 1.2 |
| async | 637 | 355 | 55.7 | 504 | 1610 | 14352 | 0 | 3679 | 571 | 1.6 |
| chalk | 72 | 62 | 86.1 | 53 | 148 | 72 | 0 | 0 | 63 | 0.7 |
| dayjs | 117 | 91 | 77.8 | 57 | 181 | 97 | 0 | 0 | 116 | 0.6 |
| underscore | 262 | 252 | 96.2 | 186 | 455 | 1120 | 1 | 330 | 233 | 0.8 |

async: 278 of 281 misses are `_interopRequireDefault(x)` calls, which Jelly
models natively instead of analysing the helper's body. With `--no-interops`
async reaches 99.5% (634/637) and 66,747 edges; nothing else changes.

### Jelly `--approx` (approximate interpretation; executes the package in the sandbox)

| package | observed | found | recall % | functions | calls | edges | wrong single | wrong multi | executed | wall s |
|---|---|---|---|---|---|---|---|---|---|---|
| axios | 675 | 603 | 89.3 | 535 | 2304 | 1474 | 12 | 78 | 620 | 2.6 |
| zod | 1308 | 1207 | 92.3 | 8899 | 52142 | 53295 | 0 | 1929 | 1075 | 35.3 |
| commander | 318 | 307 | 96.5 | 296 | 814 | 517 | 0 | 4 | 311 | 1.6 |
| async | 637 | 355 | 55.7 | 504 | 1610 | 14352 | 0 | 3679 | 571 | 2.9 |
| chalk | 72 | 62 | 86.1 | 53 | 148 | 72 | 0 | 0 | 63 | 1.8 |
| dayjs | 117 | 102 | 87.2 | 57 | 181 | 108 | 0 | 0 | 116 | 1.9 |
| underscore | 262 | 259 | 98.9 | 186 | 455 | 2232 | 0 | 638 | 233 | 2.8 |

`--approx --library` and `--library` drop `.ts` entry files through Jelly's
node_modules file filter, so zod is empty under them (recorded, not used).

### ACG, `@persper/js-callgraph` 1.3.2, ONESHOT

| package | observed | found | recall % | functions | calls | edges | wrong single | wrong multi | executed | wall s |
|---|---|---|---|---|---|---|---|---|---|---|
| axios | 675 | 300 | 44.4 | 261 | 1169 | 832 | 1 | 54 | 311 | 0.6 |
| zod | 1308 | 4 (line-level) | 0.3 | 226 | 5600 | 9924 | 0 | 0 | 4 | 3.7 |
| commander | 318 | 10 | 3.1 | 11 | 70 | 21 | 0 | 3 | 10 | 0.3 |
| async | 637 | 474 | 74.4 | 248 | 992 | 57848 | 0 | 30216 | 470 | 0.9 |
| chalk | 72 | 9 | 12.5 | 4 | 40 | 9 | 0 | 0 | 9 | 0.3 |
| dayjs | 117 | 105 | 89.7 | 40 | 149 | 109 | 0 | 0 | 105 | 0.3 |
| underscore | 262 | 168 | 64.1 | 73 | 340 | 263 | 0 | 0 | 170 | 0.3 |

esprima 4.0.1 rejects ES2020 and the tool drops the file silently: axios 7 of
72 files, commander 2 of 7 (`lib/command.js` among them), chalk 3 of 5, zod
135 of 332 (all of `src/v4/core`). DEMAND crashes on any package with a parse
failure; FULL is unimplemented. zod's `.ts` is transpiled by Babel first, so
ACG's positions are in transpiled coordinates and only a labelled line-level
score (4 found) was possible.

## How it was measured

Tools: Jelly 0.13.0, https://github.com/cs-au-dk/jelly at
`62e60eb4fd5b991c5bf4cfc992f018984f560ea4` (2026-08-11), BSD-3-Clause;
`@persper/js-callgraph` 1.3.2 from npm. Both run in an image built on the
bench's pinned `node@sha256:dc26ae92…` (Node v24.22.3) and executed with the
bench's sandbox flags: `--user 1000:1000 --network none --read-only --tmpfs
/tmp --cap-drop ALL --security-opt no-new-privileges --pids-limit 256
--memory 4g -e HOME=/tmp`, the package tree mounted read-only, no host
environment. Jelly got `NODE_OPTIONS=--max-old-space-size=3584` and a 256 MB
`/tmp`; ACG ran with `-w /acg` so Babel finds its plugins. The file list is
the bench's `analysed` globs, passed as entry files; `-b <pkg>` makes Jelly's
paths relative to the package root.

Dockerfiles, the runner, the scorer, raw outputs and logs are in the
gitignored `research/package-graphs/repos/jelly-baseline/` (`Dockerfile`,
`Dockerfile.js-callgraph`, `run.py`, `score.py`, `raw/`, `results/`,
`RESULTS.md`); the Jelly checkout is `research/package-graphs/repos/jelly`.
The Dockerfile is eleven lines: `FROM` the pinned image, `npm ci`, copy
`resources bin src`, `npm run build && npm prune --omit=dev`, `chmod -R a+rX`
(the sandbox uid must read the install), entrypoint `node /jelly/lib/main.js`.
WP0 commits the scorer as a bench measure (`tests/js-analysis-bench/`) so the
comparison is reproducible from tracked files; until then this document and
the gitignored folder are the record.

Position conversion: Jelly writes `file:line:col:line:col` with 1-based
lines and 1-based, end-exclusive columns (Babel's 0-based UTF-16 column plus
one; `src/output/analysisstatereporter.ts`, `makeLocStr`); js-callgraph
writes esprima `range` in UTF-16 code units. Both were converted to UTF-8 byte
offsets of the original file. Every observed site matched a Jelly site by
exact span and 97 to 100% of callees by exact span (the rest by same end
byte: observed method spans start at the key, Jelly's at the modifier).

## Deviations from the bench's scorer

1. "Claimed" targets: the bench counts only resolved, non-heuristic targets;
   the tools have no such distinction, so every tool edge counts, Jelly's
   external-callback edges included. The tools' wrong-target numbers are an
   upper bound.
2. ACG's analysed-file set is inferred from its edges (it writes no file
   list), so its "not analysed" outcome is overstated; recall is unaffected.
3. Jelly's per-file module entries are excluded from callee matching as polint
   excludes its `module` functions.
4. The line-level match for ACG on zod is not the bench's rule and is labelled.
5. polint's outcome taxonomy ("stops at import binding", "marked unresolved")
   has no counterpart.
6. Wall times are docker wall times on a host that ran the other tool chain
   in parallel for part of the time.

## Sources

- Jelly repository and `README.md` for the flags, accessed 2026-11-17.
- Laursen, Xu, Møller, "Reducing Static Analysis Unsoundness with Approximate
  Interpretation", PLDI 2024, https://cs.au.dk/~amoeller/papers/approx/ — for
  what `--approx` does; its published averages are not used here.
- Feldthaus, Schäfer, Sridharan, Dolby, Tip, ICSE 2013 — the algorithm
  js-callgraph implements; https://github.com/Persper/js-callgraph.
