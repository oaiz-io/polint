# L1 — Prebuilt engine + thin-SDK rule binary over a fact snapshot (2026-09-28)

Situation: **ice-cold** (`.polint/cache`, rule-host target, `CARGO_HOME`, XDG caches all absent).
Status: **owner decision — unmeasured prototype needed.**

## What it is

Today `polint check` shells out to build the repo-local rule pack, and that pack depends on the whole
`polint` library — parsers, kernel, solvers, and a bundled SQLite. The first thing any machine that has
never run a check waits for is a full compile of the analysis engine; that compile repeats on any
machine without a warm `CARGO_HOME`/target directory, and re-triggers whenever the pinned polint
version changes.

L1 inverts the roles: the **installed, prebuilt `polint` binary does the analysis** and writes an
owned fact snapshot. The repo-local rule pack compiles only a **thin SDK** — typed fact views,
`RuleCtx`, diagnostics, and the `#[polint::rule]` macro surface — with no parsers, no kernel, no
solvers, no SQLite. The rule process deserializes the snapshot once and the existing typed views
borrow from it, so rule `.rs` source does not change by a byte; what changes is the package/build
contract a pack depends on.

## Measured evidence

- The rule-host compile is **97–99% of every ice-cold run** measured in the campaign (Go+TS monorepo
  278 of 283 s Phase-1 baseline; OAIZ 249–253 s).
- In the OAIZ pack build (`cargo --timings`, fresh target, 4 jobs), after the campaign's shipped
  `373e1ed4` (SQLite test-only), the **polint crate alone is ~150 s of a ~203 s makespan** — the pack's
  own crate is only 5–7 s of that.
- Compile peak RSS is **3.2–3.3 GB** (rustc compiling the polint crate) against **0.13–0.35 GB** for
  the analysis process itself — roughly an order of magnitude difference in memory, not just time.

## Expected return

Projected ice-cold: ~225 s (current FINAL) → dependency fetch + thin-SDK compile + pack compile +
analysis, order of **30–60 s**. **This is unmeasured** — it is a projection from the prior
`research/code-preserving-rule-build` research's estimated SDK closure size (~25–35 compiled units
versus today's ~223), not a measurement of a working prototype.

## Prior research

This is not a new idea: `research/code-preserving-rule-build/` in this repository already contains a
completed research pass — `FINAL-REPORT.md` (current-state evidence, seven code-preserving
alternatives with explicit rejection reasons, a decision matrix, the recommended architecture,
security/trust boundaries, and an experiment plan with budgets and kill criteria) and
`IMPLEMENTATION-PLAN.md` (package graph, boundary design, snapshot format, host/rule protocol, an
eleven-phase build plan, and a direct breaking `0.3.0` migration plan — no legacy backend, no
compatibility shim). That research's own conclusion: **prebuilt engine host + thin-SDK rule binary
over a fact-snapshot protocol**, matching L1 exactly. It was scoped as an intentionally breaking
0.3.0 migration and was never released; only its Phase A measurement harness landed
(`polint-bench build-cost`, `make build-cost`).

## What a prototype would measure first

The prior research already specifies the first experiment, **E1**: measure the SDK dependency
closure's size before and after, via `cargo tree -e normal --target <triple>` and
`cargo build --timings` for (a) today's pack and (b) a prototype SDK-closure pack, across the
released target triples. E1's own numbers to report: compiled-unit count, wall time, target
directory bytes, and `CARGO_HOME` delta bytes — the same instrumentation `polint-bench build-cost`
already implements. The research's own risk note: if the SDK closure cannot be brought under roughly
60 units on any released target, the cold-start improvement drops under ~4× and would not justify the
migration churn — so E1 is a go/no-go gate, not just a benchmark.

## Cost / risk

- Breaks the build/manifest contract: packs move from depending on `polint` to a renamed `polint-sdk`
  (or equivalent), which is a hard version boundary, not a compatible upgrade.
- Requires designing and versioning a snapshot protocol between the engine binary and the rule
  process.
- Estimated at weeks of implementation once E1 confirms the closure-size assumption; the prior
  research's own plan lists eleven phases (B through K) beyond the measurement harness, none of which
  have started.
