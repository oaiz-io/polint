# L6 — Rule-host builds without thin-local LTO (2026-09-28)

Situation: **ice-cold build time vs. every subsequent cold/warm analysis run** — a genuine trade, not
a strict win. Status: **owner decision — runtime-vs-build-time trade.**

## What it is

rustc's default release profile runs a ThinLTO pass across each crate's codegen units. This lever is
building the rule host with `CARGO_PROFILE_RELEASE_LTO=off` (or an equivalent documented
recommendation for pack authors), trading link-time optimization for a faster build.

## Measured evidence

`-Z time-passes` on the polint crate (fresh target, 4 jobs): **153.6 s total**; frontend ~50 s (type
check 10.3 s, borrowck 12.6 s, metadata 20.7 s); `finish_ongoing_codegen` **69.0 s**, of which
`LLVM_thinlto` is **63.3 s** — ThinLTO alone accounts for over 40% of the crate's total compile time.

Same-session fresh OAIZ pack builds:

| | Default (LTO on) | `LTO=off` | Change |
|---|---|---|---|
| Full pack build | 210.9 s | 164.5 s | **−46 s, −22%** |
| polint crate alone | 150.2 s | 114.8 s | — |
| Rule-host binary size | 26.9 MB | 30.3 MB | +3.4 MB |

Measured analysis cost of the LTO-off host (OAIZ core, 3 interleaved rounds, identical stdout
verified):

| Tier | LTO on | LTO off | Change |
|---|---|---|---|
| Cold | 6.20 s | 6.50 s | **+4.7%** |
| Warm | 4.69 s | 5.20 s | **+10.9%** |
| Warm `go.syntax` stage | 389 ms | 464 ms | +19% |

**Break-even is approximately 90 warm analysis runs per one host rebuild** before the LTO-on build's
extra 46 s of build time pays for itself against the LTO-off host's per-run overhead.

## Why this is a trade, not a win

Unlike every other lever in this menu, L6 is measured, real, and **not** presented as something to
ship as a default. It genuinely helps machines that rebuild the rule host often relative to how often
they run `check` (e.g., CI that rebuilds on every commit but runs few checks per build) and genuinely
hurts machines that build once and check many times (the common developer workflow, and every
measured workload in this campaign's warm tiers). It also changes how consumer rule code itself
should be optimized, since a slower-linked host changes the relative cost of rule-side hot paths.

## Cost / risk

No implementation cost beyond flipping a build-profile setting — the cost here is entirely the
runtime regression above, borne on every check after the build until the break-even point. This is why
it stays an owner decision: the right choice depends on a given team's actual build-vs-run ratio, which
this campaign's fixed workloads cannot determine on their behalf.
