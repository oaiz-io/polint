# L7 — Split the engine crate for parallel compilation (2026-09-28)

Situation: **ice-cold**. Status: **owner decision — entirely unmeasured, needs a prototype.**

## What it is

A facade `polint` crate over 3–4 internal crates (candidate split: facts/db, Go, TS, deep analysis),
so that during an ice-cold build, rustc can compile the independent internal crates' frontends in
parallel across job slots instead of one rustc process working through the whole engine
single-threaded.

## Measured evidence (of the problem, not of the fix)

- One rustc process compiles the entire polint crate — **~261k non-test lines** — as a single
  compilation unit.
- That crate's frontend (type check, borrow check, metadata generation) is **~50 s and
  single-threaded**, and it only starts after every one of the crate's dependencies has finished
  building, since Cargo can't start compiling a crate before its dependency graph is ready.
- Across a **whole ice-cold build**, rustc utilization averages only **~2.7 of 4** available job slots
  — implying roughly 1.3 job-slots' worth of idle capacity across the build that a crate split with
  independent, parallel-buildable frontends could in principle use.

## Expected return

**Entirely unmeasured.** The claim is qualitative: overlapping independent frontends should shorten
the polint phase of an ice-cold build substantially, but no prototype exists, and no number in this
campaign estimates by how much. This is explicitly flagged as needing a prototype before any return
can be claimed, unlike L1 (which at least has a prior research estimate) or L5/L6 (which have profiled
cost breakdowns to project from).

## Cost / risk

- Breaks the **two-package architecture** documented in `ARCHITECTURE.md` — the current design is
  deliberately a two-package graph with private module layering, and this lever proposes replacing
  that with a facade over several internal crates.
- Widens **visibility** across what are today module boundaries into crate boundaries, working against
  this project's visibility discipline (narrowest-visibility-first, `unreachable_pub` enabled,
  `pub(crate)` preferred — see AGENTS.md's "Public API and visibility" conventions). Crate boundaries
  are a harder visibility line than module boundaries: anything crossing a crate boundary must be at
  least `pub`, which is a strictly wider commitment than `pub(crate)` within a single crate.
- Estimated at weeks of restructuring work, and the architectural cost would need to be weighed against
  an ice-cold win that has not yet been measured or even prototyped.
