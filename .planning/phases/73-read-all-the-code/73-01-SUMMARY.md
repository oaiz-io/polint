---
phase: 73-read-all-the-code
plan: 01
subsystem: call-graph
tags: [extraction, ts-frontend, mir, vendor, js-bench]

requires: [72-01, 72-02]
provides:
  - Every function and call site Oxc parses in the analysed source is a FunctionFact or CallSiteFact with Oxc's span
  - A call whose callee is a sequence expression is a call to its last operand
  - A package's own vendor directory is analysed when the package is the unit of analysis
  - A committed key list that every later extraction change must keep (the superset test)
affects: [phase-74, phase-75, phase-76]

key-files:
  created:
    - tests/js-analysis-bench/function-keys.txt
    - tests/js-analysis-bench/constructs/block-function/
    - tests/js-analysis-bench/constructs/exported-initializer/
    - tests/js-analysis-bench/constructs/package-vendor/
    - tests/js-analysis-bench/constructs/sequence-callee/
    - tests/js-analysis-bench/constructs/unary-iife/
  modified:
    - crates/polint/src/ts/adapter.rs
    - crates/polint/src/ts/mir/lower.rs
    - crates/polint-eval/src/harness/js_bench.rs
    - docs/facts/call-graph.md
    - docs/facts/functions.md
    - tests/js-analysis-bench/README.md
    - tests/js-analysis-bench/report.py
    - tests/js-analysis-bench/selftest.py
    - tests/js-analysis-bench/baseline/

key-decisions:
  - "Function rows come from the frontend (`ts/adapter.rs`) and the call fact takes its callee from the MIR call's callee text, so the sequence form is handled in `ts/mir/lower.rs`; `analysis_neutral/calls/` needed no change."
  - "A package's vendor directory needs no engine or config change: `[workspace] include` and `exclude` exist, and the bench writes them into each staged tree."
  - "A site whose callee is `(0, m.f)` is attributed to the gap of `m.f`; `sequence_interop` is kept for sites polint cannot read."
  - "Reproducibility is checked on `report.json` with wall time, peak memory and per-dump cost removed; a report is never byte-identical across runs because it carries those fields."

requirements-completed: [PG-04]
---

# Plan 73-01 summary: read all the code

Pull request: #146, stacked on #145.

## What changed

The TS/JS frontend and MIR lowering now walk every place Oxc puts a function
or a call:

- operands of unary operators (`!function () {}()`, UMD prologues);
- functions declared in `try`, `catch`, `finally` and `if` blocks;
- calls in parameter defaults, destructuring defaults and computed keys;
- methods with a computed key, named by their span.

A call through a sequence expression (`(0, m.x)(a)`) takes the callee of its
last operand. The bench analyses a package's own `vendor/` directory and keeps
the repository default for repositories.

## Numbers (untyped; base / refined)

| measure | before | after |
|---|---|---|
| tune recall, every tier, base | 42.6% | 44.4% |
| tune recall, every tier, refined | 42.6% | 44.4% |
| held-out recall, every tier, base | 23.1% | 27.1% |
| held-out recall, every tier, refined | 23.5% | 27.5% |
| wrong exact targets, tune and held-out, every plan | 0 | 0 |
| false-complete, constructs (call lines) | 0 of 87 | 0 of 121 |
| `not_extracted` edges, tune | zod 9, axios 20, dayjs 117, async 1, commander 1 | 0 |
| `not_analysed` edges, tune | chalk 47 | 0 |
| `sequence_interop` edges, tune | async 157 | 0 |

Per tune package, base (refined where it differs):

| package | before | after |
|---|---|---|
| async | 50.9% (51.5%) | 50.9% (51.5%) |
| axios | 40.0% | 40.4% |
| chalk | 23.6% | 59.7% |
| commander | 6.6% (37.4%) | 6.6% (37.4%) |
| dayjs | 0% | 30.8% (31.6%) |
| underscore | 59.9% (60.3%) | 59.9% (60.3%) |
| zod | 55.0% | 55.1% |

Held-out packages that moved: pako 0% → 86.8% (87.2% refined) and immer
74.6% → 76.3%. No other held-out package moved. The held-out set was not
read; only its report rows were.

No recall number on any cause bucket dropped. async's 157 `sequence_interop`
edges are now `module_member` (150) and `other` (7): they fail for the reason
the same call without the sequence fails.

## Typed plan (TypeScript 5.9.3)

| measure | before | after |
|---|---|---|
| tune recall: exact-complete / exact / exact + conservative | 45.6% / 54.2% / 59.9% | 47.5% / 56.9% / 62.7% |
| held-out recall: exact-complete / exact / exact + conservative | 23.7% / 28.6% / 29.6% | 27.8% / 33.2% / 34.4% |
| wrong exact targets, every set | 0 | 0 |
| false-complete, constructs | 0 / 0 | 0 / 0 |

The 8 typed single-target answers the run never called (commander 1,
underscore 7) are conservative candidates and unchanged from the plan
before this one; none is in the exact tier.

## Cost (zod, `make js-bench` with cost)

| measure | before | after |
|---|---|---|
| cold check | 3982 ms / 525 MB | 4291 ms / 524 MB |
| warm check | | 2734 ms |
| fact dump, base | | 3762 ms / 534 MB |
| fact dump, refined | | 11383 ms / 1023 MB |

The 1-minute / 5-minute / 15-minute load average was 3.21 / 7.16 / 18.06 at
the start of the cost run and 15.47 / 11.62 / 17.26 at its end. The server
is shared; the times are upper bounds.

## Construct cases

Ten new cases, each committed with its recording before any extraction
change. Every labelled function and call site in them is extracted.

| construct | base calls | refined calls |
|---|---|---|
| block-function | 9/9 | 9/9 |
| exported-initializer | 6/6 | 6/6 |
| package-vendor | 4/6 | 4/6 |
| sequence-callee | 5/8 | 5/8 |
| unary-iife | 11/13 | 12/13 |

Every remaining miss has a polint call site and is a resolution gap: a member
of a `require` binding, an object-literal member reached only through the
walker, a call of a parameter, and `module.exports` assigned through a
factory's return. These belong to the value-flow and resolution plans.

## Reproducibility

Two `bench.py report --no-cost` runs give equal `report.json` once wall time,
peak memory and dump cost are removed (`EQUAL`). The superset test
(`construct_function_keys`) checks every construct function's key against
`function-keys.txt`.

## Stitched equals whole

There is no composition test yet; it is built with the package graph export
and stitch. This plan does not change what a package graph contains at its
boundary.

## Mutation checks

| mutation | result |
|---|---|
| unary-operand walk removed (frontend and MIR) | caught: both `unary-iife` cases 0, dayjs recall 0 with 117 edges `not_extracted` |
| sequence callee takes the first operand | caught: `sequence-callee/local-sequence` 3/4 → 1/4 (`compiled-interop` stays 2/4; its misses are the require-member gap) |
| `catch` block not walked | caught: `block-function/catch-and-finally` 4/4 → 3/4, 1 call `not_extracted` |
| anonymous callable name format changed | caught: `construct_function_keys` lists every anonymous key as missing |

## Deviations from the plan

Recorded in the plan's `<plan_corrections>`: the files changed, the two
exported-initializer cases that were already at 100%, extraction read as
"every site extracted", sequence attribution, defaults and computed keys,
TypeScript spans, the vendor residual, the meaning of byte-identical, and the
mutation expectations.

## Still open

- `package-vendor` and `sequence-callee` stay below 100% on require-member
  resolution. This is a value-flow gap, not an extraction gap.
- The held-out exact recall is 17.3 points below tune (base). The gap
  narrowed from 19.5 and is still outside the margin.
