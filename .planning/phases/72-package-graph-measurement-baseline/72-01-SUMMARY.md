---
phase: 72-package-graph-measurement-baseline
plan: 01
subsystem: js-analysis-bench
tags: [bench, tiers, held-out, jelly, recorder, cost]

requires: []
provides:
  - Recall per tier (exact-complete, exact, exact + conservative, apart) per package and plan
  - Executed-site precision per tier, open sites by reason, budget events, refined-edge census, gained sites
  - Jelly static and --approx columns from tracked files, scored by the bench's own matching
  - Eight held-out packages, recorded twice, reported apart with driver coverage, and a CI guard
  - Provider stage times and the machine's load average per fact dump
affects: [phase-72-02, phase-73, phase-74, phase-75, phase-76, phase-77]

key-files:
  modified:
    - crates/polint/src/analysis_neutral/calls/facts.rs
    - crates/polint-eval/src/harness/js_bench.rs
    - tests/js-analysis-bench/report.py
    - tests/js-analysis-bench/bench.py
    - tests/js-analysis-bench/selftest.py
    - tests/js-analysis-bench/inputs.toml
    - tests/js-analysis-bench/README.md
    - tests/js-analysis-bench/observed/recorder.mjs
    - tests/js-analysis-bench/held_out_guard.py
    - .github/workflows/ci.yml
    - research/package-graphs/DIAGNOSIS.md
  created:
    - tests/js-analysis-bench/outside/Dockerfile.jelly
    - tests/js-analysis-bench/outside/score_outside.py
    - tests/js-analysis-bench/observed/drivers/heldout/

key-decisions:
  - "The analysed set of every package is its shipped source: the analysed globs with test and benchmark paths removed. zod is reported once with each set."
  - "CallPrecision::tier() is a closed match with no wildcard: Exact and SetupAware are exact, Conservative and Ambiguous conservative, Heuristic apart, Unknown and Unsupported no claim."
  - "The recorder records the functions each run enters; `bench.py record-observed` records every package twice and refuses to write when the runs differ."
  - "Driver coverage is computed in the report from the entered functions and the census, not in the recorder."
  - "The axios cost regression of DIAGNOSIS section 5 does not reproduce; there is no commit to bisect."

requirements-completed: [PG-01, PG-02]
---

# Plan 72-01 summary: the measurement baseline

Pull requests: #142 (part A: tiers, shipped source, Jelly) and #143 (part B:
held-out set, recorder, held-out gate). #144 adds the construct fixture for the
defect the held-out gate found, and plan 72-02 fixes that defect.

## What the bench reports now

Every number TARGETS.md gates on is in `report.md`:

- `## Tiers`
- `## Precision by tier`
- `## Open sites`
- `## Budget events`
- `## Refined edges`
- `## Sites gained by the refined plan`
- `## Outside tools`
- the held-out tables and the held-out gate

`bench.py selftest` fails when any of these sections is removed.

## Baseline numbers, before plan 72-02

**Tune set, no types, pooled.** There are 3389 observed edges.

| plan | exact-complete | exact | exact + conservative | apart only |
|---|---|---|---|---|
| base | 1443 (43%) | 1443 (43%) | 1443 (43%) | 65 (2%) |
| refined | 1508 (44%) | 1508 (44%) | 1508 (44%) | 103 (3%) |

There are 0 wrong exact targets on the tune set and on the construct fixtures.

**zod, under the analysed-set rule:**
- With the globs alone: 332 files, 48838 call sites.
- With shipped source: 125 files, 5967 call sites.
- Recall is 719/1308 under both sets.
- The refined dump goes from 52.3 s and 4001 MB to 13.4 s and 1011 MB.

**Held-out gate: not met.**
- The tune set reaches 42.6% (base) and 44.5% (refined).
- The held-out set reaches 23.1% (base) and 23.6% (refined), about 20 points below the tune set.
- The refined plan had 3 wrong exact targets on the held-out set (acorn `Parser.parse(input)`). This was a stop-and-fix, done in plan 72-02.

## DIAGNOSIS section 5: the axios cost

It does not hold. Across five runs:
- Axios refined is 2.0 to 2.6 s, against the committed 2.1 s.
- `semantic_graph` takes 0.21 to 0.28 s of it.
- Base is 0.63 to 0.74 s.

The refined plan is 3.2 to 3.5 times the base plan, from the providers it adds
(`evidence`, `type_value_alias`, `abstract_domains`), not from the walker. The
walker code is unchanged since `7aa3b3cf`, so there is no commit to bisect.
The measurement is written into DIAGNOSIS.md section 5.

## Mutation checks

| mutation | result |
|---|---|
| Heuristic mapped to exact in `tier()` | caught: `call_precision_tier_is_a_closed_list` fails |
| Heuristic mapped to exact in the report's tier table | caught: `selftest` fails |
| `solver_budget` dropped from the dump | caught: the report stops with `KeyError: 'solver_budget'` and writes nothing |
| Jelly spans read end-inclusive | caught: `selftest` fails on the outside-tool site match |
| A PR touching analysis code and a held-out file together | the guard exits 1 |
| Stage times summed into a parent stage | caught |
| `Math.random()` in a driver | caught on 4 of 5 runs, see below |

The `Math.random()` mutation made each step of the co driver skip with
probability one half. On 4 of 5 runs the two recordings differed and nothing
was written. On the first run both recordings agreed, so the recording was
written, and it was restored by path. The recording is a set of edges, and
most steps reach edges that other steps also reach. Two runs that skip
different steps can therefore record the same set. Recording twice catches a
driver whose randomness changes the recorded edges; it does not prove a driver
deterministic. HELD-OUT.md rule 6 (no time, randomness, network or environment
in a driver) is still kept by review, and a static check of the bench's own
drivers is open below.

A stale bytecode cache can hide a mutation's restore. The tier-table mutation
and its restore are the same size and were written in the same second, so
Python kept the mutated `report.pyc`. The next bench run stopped on it
(`the engine says 'apart' for Heuristic, the bench says 'exact'`). The cache
file was removed by path, and the remaining mutation runs set
`PYTHONDONTWRITEBYTECODE=1`.

## Deviations from the plan

- **The plan was split into two pull requests**, A and B. HELD-OUT.md rule 2
  refuses analysis code and held-out bench files in one pull request.
- **The recorder schema is now 2.** It records the entered functions, and
  the report computes driver coverage from them and the census, rather than
  the recorder computing it.
- **The stub modules axios loads are committed.** A missing stub directory is
  refused.
- **The gate runs with `GOTOOLCHAIN=go1.25.5`.** The host's Go 1.27 writes
  export data that the Go sidecar fixtures do not read.

## Still open

The cost baseline (`make js-bench` with cost) is committed on the 72-02
branch. The server stayed shared and loaded during the run: the 1-minute load
average ranged from 6.5 to 14.7 and the 5-minute average from 12 to 15. The
report's `## Cost` section records the readings at the start and the end. Read
the absolute figures as upper bounds for a quiet machine. A quiet-machine
rerun is still open. The `--no-cost` reproducibility diff is recorded in the
72-02 pull request.

A static check of the bench's own drivers for unseeded randomness and clock
reads is not written. It needs a rule for the drivers that already seed
`Math.random` and `crypto` (node-forge) or construct a `Date` whose value they
never read (immer). It also has to land in a pull request that touches no
analysis code (HELD-OUT.md rule 2).
