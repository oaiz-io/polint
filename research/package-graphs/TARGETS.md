# Targets, checked

Date: 2026-11-17. Each target of [ITERATION-1-CLASSICAL.md](ITERATION-1-CLASSICAL.md)
against what [DIAGNOSIS.md](DIAGNOSIS.md) and [BASELINES.md](BASELINES.md)
found. "Now" is the committed v0.5.3 baseline unless a cell says otherwise.

## Accuracy

| measure | now | ITERATION-1 target | decision | reason |
|---|---|---|---|---|
| recall, exact plus conservative, no types | 47.5% (commander's walker rows; the points-to solve contributes 0 on the other six) | 80%, gate 75% | **keep** | Jelly static reaches 84.5% on these packages with an inclusion-based analysis and no types, at 9 targets per site on async. 80% with polint's precision rules is the right first ask; parity with Jelly is the stretch, not the gate. |
| recall per package, no types | 0% to 60% | each ≥ 65% | **keep** | Jelly's worst is async at 55.7% because it models the interop helper natively; polint analyses the helper as code and should pass async. dayjs at 77.8% for Jelly says 65% is not optimistic. |
| recall, exact tier alone, no types | 44.5% (base; every base edge is `SetupAware`, which the tier list counts as exact) | 60% | **keep** | The exact tier grows by the call rule on closed sites (a `const` callback passed to a local function is exact). 60% is below Jelly's static total and above today's base. |
| recall with types | 61.4% | 85% | **keep, with a condition** | Types gave zod 3 points and async 2; the typed plan's gain is axios and commander. 85% is reachable only once the solver carries the value flow, so this target is measured after WP3, not before. |
| against Jelly static on the same packages | now measured: Jelly 84.5% | at or above | **replace with numbers** | No-types recall ≥ 80% and typed recall ≥ 85% against Jelly static 84.5% and approx 85.4%; plus the precision rows below, which Jelly does not meet (13 wrong single, 5,811 wrong multi). |
| complete answers the run contradicts | 0 | 0 | keep | |
| wrong exact targets at executed sites | 0, 0, 15 single and 2 multi (base, refined, typed) | 0 on every plan | keep | DIAGNOSIS §4 names the join rule that removes the 15. |
| executed-site precision, conservative tier | not measured | ≥ 70%, per package | **keep, add a width row** | Add "targets per resolved site, mean and p95, per tier" as a reported row; no gate on it this iteration. Jelly's 9 per site on async is what the conservative tier must not become. |
| precision on hand-labelled constructs, exact tier | 100% | 100% | keep | |
| edges lost to code the engine does not read | 349 | 0 apart from written exclusions | keep | |
| held-out recall | no set | within 5 points of the tune set | **keep, with coverage** | The set is in [HELD-OUT.md](HELD-OUT.md); a held-out package under 40% driver coverage is reported and left out of the gate's average. |
| **new:** `Heuristic` edges | 161 on commander, 0 elsewhere | — | **add: 0 at the end of the iteration** | The walker is the only producer; its rows must be replaced by lowered constraints or dropped. |
| **new:** budget events that discard derived edges | every package above ~1,000 call sites | — | **add: none, by construction** | A budget event may stop a solve and mark the package graph incomplete; it may never remove an edge already derived or blank the result. |

## Cost

Same machine as the committed baseline, quiet. The figures measured for
DIAGNOSIS §5 were taken on a loaded host and are not the new baseline; WP0
re-measures first.

| measure | now | ITERATION-1 target | decision | reason |
|---|---|---|---|---|
| zod, full graph, no types, time | 48.8 s committed; 85.7 s here under load | 20 s | **keep** | Jelly static does zod in 22.9 s inside docker. 20 s is at the same scale with a native engine. Note that 1.7 of zod's 3.1 MB are test files the driver never runs; WP0 decides whether `analysed` keeps them (it changes cost, not recall). |
| zod, peak memory, no types | 3.8 GB | 2 GB | **keep** | The base plan is 1.3 GB; the solver should add less than it. |
| axios, full graph, no types, time | 2.1 s committed; 105 to 112 s here | 1.5 s | **keep, and add a gate** | The regression on this checkout (DIAGNOSIS §5) must be confirmed or refuted in WP0. New gate: on every tune-set package the refined plan takes at most 3× the base plan. |
| budget events on the tune set | not reported; present on axios (`points_to.max_dynamic_vars`, `solver.max_steps`) | 0, and reported | **keep** | Now a measured fact rather than an estimate. |
| stitch time on a real tree against analysing it whole | 9% on axios (27 packages) and express (65), one run each, load average 20.8 / 12.4 / 8.6 on 32 CPUs; 10% and 7% on an earlier run at load 5.9 / 5.8 / 6.3 | ≤ 20% | **keep, now measured** | Reading the stored graphs back and solving the union from nothing costs a tenth of the whole analysis. Building every package graph costs more than the whole analysis (5.0 s against 4.1 s on axios), but it is paid once per package version, not per program. |
| stored package graph, gzip, per MB of source | 3.4 MB per MB on the axios tree, 3.6 on the express tree; 2.2 to 4.0 on the tune-set packages alone (zod 2.6), more on sources under 10 KB | ≤ 2 MB | **keep as a ceiling, not met** | Reference: Jelly's zod call graph is 0.18 MB gzip per MB of source. The constraints are about three quarters of the stored JSON, each written with its field names and its stable key in full; the encoding is the first thing to shrink, before the ceiling is reconsidered. [STORAGE-A.md](STORAGE-A.md) gives the reasoning behind 2 MB. |
| **new:** walker time | `semantic_graph` provider 91 to 95 s on axios here | — | **add: the walker is removed or its provider is ≤ 10% of the refined plan** | The retirement criterion in WP2 needs a number. |

## Structure

Unchanged: stitched equals whole on the fixture and three real trees (measured so far on the fixture and two: axios, and express from the held-out set, compared only);
byte-identical package graphs across two scans and any thread count; every
edge carries stage, algorithm and tier; every open site a reason; every
guard a failing test and listed mutation checks.

Added:

- The package graph refuses to store a budget-exceeded solve, and the
  stitcher refuses to read one.
- Every exact-complete site stores its assumption ids ([TIERS.md](TIERS.md) (b)).
- The bench reports recall per tier (exact-complete, exact, exact plus
  conservative), executed-site precision per tier, targets per site, open
  sites by reason, and driver coverage per package, for the tune set and
  the held-out set apart.
