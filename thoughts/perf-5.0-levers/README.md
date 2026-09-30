# perf-5.0 levers — unshipped performance work (2026-09-28)

Source: the perf-5 campaign, branch `performance-improvements-5.0` (cut from `origin/main` @
`bbd1b785`, v0.4.1; final code tip `e7bf1e52` + `45b9fbf0`). The in-contract wins from that campaign
already shipped as `#129` ("perf: 1.2–1.6x faster checks — deferred syntax metadata, memoized Go
digest, SQLite test-only build") — 1.15–1.56× across ice-cold/cold/warm on every measured workload,
byte-identical output, no rule-visible behavior change. This directory does not re-document that PR;
it is the extended record of everything the campaign measured or sized but did **not** ship: the
big-lever menu (L1–L7), the small in-contract targets that were found but not taken, one consumer-side
finding, and the methodology behind every number here.

Raw evidence, harness scripts, and per-run logs are preserved at `/opt/data/polint-perf5/` on the
measurement host (not in this repository): `baseline/`, `ab/`, `dev/runs/`, `prof/out/`, `timings/`,
`logs/`, `decisions.md`. See [`09-methodology.md`](09-methodology.md) for how to read them.

All numbers below are copied from the campaign's report of record. Anything not directly measured is
marked **(unmeasured)**.

## Post-L4 update (2026-09-29)

The consumer-side L4 fix shipped in OAIZ (oaiz-io/oaiz#4800, released v1.0.3358), which made parts of
the tables below stale. [`10-next-lever.md`](10-next-lever.md) re-measures the affected tiers on the
fixed tree (both engines, three clean interleaved rounds, 4 jobs), re-ranks every lever, and picks the
next one. Rows below that the change affected carry a **Post-L4 update (2026-09-29)** marker; their
original text is unchanged. Numbers under that marker come from 10's measurements, not from the
campaign's report of record. Current OAIZ states, in seconds:

| OAIZ workload: as deployed (pack pins 0.4.1) → after oaiz#4805 (pin 0.4.2) | cold | warm | edit (first measurement; host-direct) |
|---|---|---|---|
| core profile | 5.89 → 4.60 | 4.33 → 3.12 | 4.78 → 3.53 |
| frontend profile | 2.09 → 1.56 | 1.76 → 1.23 | 1.98 → 1.54 |
| full repo (default `polint check`) | 14.45 → 14.00 | 10.12 → 9.20 | 10.84 → 10.29 |

- **L4: SHIPPED 2026-09-29.**
- **The pick:** the new metrics-trigger cliff. One metrics rule in the default rule set costs every
  default full-repo run 3.6 s warm and 5.4 s in the edit loop; the fix is in-contract.
- **L1** stays the next strategic lever. Its thin-SDK dependency floor is now measured: 15.9 s and
  328 MB.
- **The rule-host compile is unchanged:** 206.5 s fresh on 0.4.2, with 3.18 GB peak RSS.
- **Also:** L2 is killed as a performance lever, L3 is parked, and L7 is downgraded.

## Post-metrics-cliff update (2026-09-30)

The metrics-trigger cliff fix merged as #131 and shipped in v0.4.3. On the OAIZ full repo, warm
went 9.05 → 6.00 s, cold 13.70 → 10.40 s and edit 9.94 → 7.16 s; code-health warm went
1.99 → 1.01 s. [`11-next-after-metrics-cliff.md`](11-next-after-metrics-cliff.md) re-derives the
state from that fix's raw runs and re-ranks what is left:

- **New finding:** on the full repo, rayon's split order decides when the one bounding consumer rule
  starts, which costs a median 0.77 s of the rules phase (n = 107 samples).
- **The pick:** cost-ordered rule dispatch, which is in-contract and 2–3 days of work.
- **Then:** L1's E1 spike, and the cold-path metrics miss once its owner question is answered.

## Files

| File | Lever | One line |
|---|---|---|
| [01-thin-sdk-prebuilt-engine.md](01-thin-sdk-prebuilt-engine.md) | L1 | Prebuilt engine + thin-SDK rule binary over a fact snapshot — attacks the ice-cold rule-host compile. |
| [02-rule-result-memoization.md](02-rule-result-memoization.md) | L2 | Whole-run rule-result cache for no-change reruns — blocked today by rules reading the filesystem directly. |
| [03-per-file-rule-caching.md](03-per-file-rule-caching.md) | L3 | Per-file rule result cache keyed on file digest — the edit-loop complement to L2. |
| [04-consumer-span-helper.md](04-consumer-span-helper.md) | L4 | Consumer-side quadratic span helper in the OAIZ pack — the single biggest measured win in the whole campaign, not shippable by polint. |
| [05-binary-layer-cache.md](05-binary-layer-cache.md) | L5 | Binary-encoded syntax-layer cache blobs + a faster integrity hash — the largest polint-side cost left in a warm run. |
| [06-lto-build-trade.md](06-lto-build-trade.md) | L6 | ThinLTO off for rule-host builds — trades build time for analysis time. |
| [07-engine-crate-split.md](07-engine-crate-split.md) | L7 | Facade crate over 3–4 internal crates so rustc frontends overlap during an ice-cold build. |
| [08-next-targets.md](08-next-targets.md) | — | Small in-contract targets sized but not taken, the two measured rejections, and the one consumer finding L4 doesn't cover. |
| [09-methodology.md](09-methodology.md) | — | How every number in this directory was measured, so the evidence is auditable. |
| [10-next-lever.md](10-next-lever.md) | — | Post-L4 re-analysis: re-measured gaps (the edit tier included), re-ranked levers, the pick (the metrics-trigger cliff), and de-risk plans for it and for L1. |
| [11-next-after-metrics-cliff.md](11-next-after-metrics-cliff.md) | — | After the metrics-cliff fix (#131, v0.4.3): state-of-the-union table, a new rules-dispatch scheduling finding, re-ranked candidates, the pick (cost-ordered rule dispatch) and its spike plan. |

## What each lever is

**L1 — Prebuilt engine + thin-SDK rule binary.** Today `polint check` compiles the entire engine
(parsers, kernel, solvers, SQLite) into the repo-local rule pack on every machine that has never run
it. L1 inverts that: the installed `polint` binary does the analysis and writes a fact snapshot; the
pack compiles only a thin SDK (views, `RuleCtx`, diagnostics) plus its own rules against that snapshot.
This is the only lever that touches the ice-cold rule-host compile itself, which is 97–99% of every
ice-cold run today.

**L2 — Rule-result memoization.** No rule's output is reused across runs, even when nothing changed.
On a full-repo warm rerun, rules are 97% of the wall time. The blocker is that rules in both measured
consumer packs read the filesystem directly outside their declared fact inputs, so keying a cache on
fact inputs alone would be unsound until rules declare (or are proven pure of) those extra reads.

**L3 — Per-file rule caching.** L2's complement for the edit loop: cache each rule's per-file result
keyed on `(rule, file, file digest, options)`, so editing one file re-runs rules only for that file
instead of the whole rule set. L2 helps the no-change rerun; L3 helps the common case where a few
files changed.

**L4 — Consumer-side span-helper fix.** Not a polint change — a finding about the OAIZ rule pack. Its
shared Go scanner recomputes line/column spans by rescanning the file from byte 0 on every call, and
five rules do this over every Go file. This is the single largest measured win in the whole campaign
(49× on a full-repo run) and it is entirely consumer-side; polint's role is exposing the fix shape
(spans that are already line-indexed on typed fact views) and reporting the number to the OAIZ team.

**L5 — Binary layer-cache encoding.** Every warm run today decodes the syntax layer cache from JSON
and verifies it with a byte-wise FNV-1a hash. After the campaign's shipped changes, this decode+verify
is the largest polint-side cost left in a warm run. A binary encoding plus a word-at-a-time hash would
shrink both.

**L6 — ThinLTO off for rule-host builds.** rustc's default release profile runs ThinLTO across the
polint crate's codegen units on every ice-cold build. Turning it off cuts build time meaningfully but
measurably slows every subsequent `check` — a genuine runtime-vs-build-time trade, not a clear win.

**L7 — Split the engine crate.** One rustc single-threaded frontend pass compiles the whole polint
crate (~261k non-test lines) before any of its codegen can start, and idles part of the job pool while
doing it. Splitting the engine into a facade over several internal crates could let independent
frontends overlap — entirely unmeasured, needs a prototype, and breaks the current two-package
architecture and visibility rules.

## Expected impact by situation

| Lever | What it is (short) | Where it pays | Expected impact (measured, unless noted) | Cost | Status |
|---|---|---|---|---|---|
| L1 | Prebuilt engine, thin SDK | ice-cold | Rule-host compile is 97–99% of ice-cold (polint crate ~150 s of ~203 s OAIZ pack build; compile RSS 3.2–3.3 GB vs 0.13–0.35 GB analysis RSS). Projected ice-cold ~225 s → order of 30–60 s **(unmeasured — needs the E1 closure-size prototype)** | Weeks; breaks build/manifest contract, needs a snapshot protocol + versioning | Owner decision — unmeasured prototype needed **Post-L4 update (2026-09-29):** still the only lever on the compile, re-measured at 206.5 s fresh (0.4.2, 4 jobs), ~152 s per adopted release and 3.18 GB peak RSS. Now ranked the next strategic lever, behind the metrics-trigger cliff. The E1/E3 plan, with a measured 15.9 s / 328 MB thin-SDK dependency floor, is in [10](10-next-lever.md) §6.2. |
| L2 | Whole-run rule-result cache | warm, full-repo no-change rerun | OAIZ full-repo warm 594 s ≈ cold 612 s (rules 97%); projected OAIZ full warm 594→~7 s, OAIZ core warm 4.73→~1.0 s, Go+TS monorepo warm 1.44→~0.9 s **if rules declared their extra inputs (unmeasured contract, not yet built)**. **Post-L4 update (2026-09-29):** superseded. L4 already took OAIZ full-repo warm to 10.12 s as deployed (9.20 s on 0.4.2). What memoization could still remove on a no-change rerun is the rules phase: ~2.2 s core, ~0.7 s frontend, ~3.5 s full repo (0.4.2 stage logs; unbuilt). The blocker grew: four files of the OAIZ pack call `std::fs`. Killed as a performance lever; see [10](10-next-lever.md) §4. | Rule purity / declared-extra-inputs contract; a debug-only soundness assertion | Owner decision |
| L3 | Per-file rule result cache | edit-loop | Same underlying per-rule costs as L2; heavy rules are per-file scans in practice, so per-file caching maps directly onto edit-loop reruns **(impact unmeasured as a standalone change)** | SDK/API addition for a keyed per-file cache | Owner decision **Post-L4 update (2026-09-29):** the edit tier is now measured: +0.41 s over warm on OAIZ core and +0.31 s on frontend, all of it kernel cache work, and the bounding core rule is cross-file. Parked; see [10](10-next-lever.md) §4. |
| L4 | Consumer linear span fix | consumer, full-repo | Measured on a scratch copy of the OAIZ pack, same engine, identical output: full repo 598.7→12.1 s (49×), rules CPU 1,777→7.5 s, core cold 7.32→5.38 s, core warm 5.79→3.86 s (−33%) | None to polint; a consumer-side rewrite of the shared Go scanner | Consumer change — reported to OAIZ, not polint's to ship **Post-L4 update (2026-09-29): SHIPPED 2026-09-29** in oaiz-io/oaiz#4800 (released v1.0.3358); re-measured on the fixed tree in [10](10-next-lever.md) §3. |
| L5 | Binary layer-cache + faster hash | warm | Layer read = 25% of Go+TS-monorepo warm main-thread samples (decode 17%, FNV 6.7%); OAIZ core decode 12.9%/FNV 6.2%. Projected warm go.syntax ~235→80–100 ms, ts.syntax ~85→40 ms; −10…−14% warm on the Go+TS monorepo, −0.15…−0.25 s OAIZ core warm **(projection, not yet built)** | One-time cache-protocol bump (existing caches miss once), a new dependency, a second deterministic encoding | Owner decision **Post-L4 update (2026-09-29):** OAIZ warm restores now decode 61.5 MB of JSON on core (0.44 s on 0.4.2), 29.7 MB on frontend (0.21 s) and 188 MB on the full repo. Ranked third, after the metrics-trigger cliff; doubles as L1's snapshot codec. See [10](10-next-lever.md) §4. |
| L6 | ThinLTO off, rule-host build | build time vs cold/warm analysis | Build 210.9→164.5 s (−46 s, −22%); binary 26.9→30.3 MB; measured analysis cost cold +4.7%, warm +10.9% (interleaved, identical output); break-even ≈ 90 warm runs per host rebuild | None to build (env var); ongoing analysis-time cost across every run after | Owner decision — runtime-vs-build trade **Post-L4 update (2026-09-29):** at +10.9% of a 3.12 s post-L4 warm core run, break-even rises to ~135 warm runs per rebuild (projection). Document as a knob for ephemeral environments. |
| L7 | Parallel crate split | ice-cold | Single rustc frontend for ~261k non-test lines takes ~50 s single-threaded; rustc averages only ~2.7 of 4 job slots across a whole ice-cold build, implying idle capacity a split could use **(entirely unmeasured, needs a prototype)** | Weeks; breaks the two-package architecture and widens visibility across crate boundaries | Owner decision — unmeasured prototype needed **Post-L4 update (2026-09-29):** measured ceiling ~41 s. The 0.4.2 build is 659 CPU-s over 206.5 s at 4 jobs, so at the same total work it cannot beat ~165 s; `polint`'s frontend is 46.1 s single-threaded. Downgraded. |
| Next targets (08) | Small sized-not-taken fixes | mostly cold, some warm | Go-walk fusion ~0.5 CPU-s; `GoTests::related_for_file` index ~0.13 CPU-s (growing quadratically); rule summary-row matchers ~0.03–0.05 s/run; cold layer write ~0.4 s serial; toolchain-probe caching ~50–80 ms/warm run; edit-loop tier itself is unmeasured | Small each; several need a design decision (streaming, caching key soundness, no-more-parallelism constraint) | Sized, not taken **Post-L4 update (2026-09-29):** the edit-loop tier is now measured. Three new in-contract targets (the metrics-trigger cliff, orphaned layer blobs, store-key depth) are in [10](10-next-lever.md) §4. |
| Consumer finding (08) | Core-profile rule-bound warm floor | consumer, OAIZ core | One rule (~3.7–4.2 s) bounds the OAIZ core rules phase and therefore the profile's warm floor — not covered by L4's span-helper fix | None to polint | Consumer change — reported only **Post-L4 update (2026-09-29):** after L4 that rule takes ~2.1–2.2 s and still bounds the core rules phase (2.17–2.32 s). |
| Metrics trigger (10) | Keep deferral + memo metrics | full repo / default `polint check`, code-health profile | **Post-L4 update (2026-09-29):** measured by unregistering one rule. One metrics rule costs a default full-repo OAIZ run 3.56 s warm, 5.40 s in the edit loop, 4.29 s cold and ~450 MB RSS on 0.4.2 (≤ 0.18 s of that is the rule), and ~1.0 s of a 1.99 s code-health run | Days; in-contract (deferred stable-key ids must stay identical) | **The pick** — see [10](10-next-lever.md) §5–6 |

## Floor analysis

Sound floors under today's contracts (rules compiled with the engine; rules may read any view and the
filesystem; output must stay byte-identical; caches are an optimization only). These are estimates
built from measured stage times and CPU profiles, **not measurements of an implementation**:

| Workload / tier | FINAL (measured) | Sound floor (estimate) | What the gap is |
|---|---|---|---|
| any / ice-cold | 224–229 s | ≈ the rule-host build, ~221 s at 4 jobs: dependencies (~45–50 s critical path), the polint crate (~150 s: ~50 s single-threaded frontend, then ~63 s of ThinLTO), the pack (5–7 s), ~3 s analysis | Nothing left in-contract short of shrinking the polint crate's own code; L6 trades −46 s of that for slower analysis, L1 removes the engine compile entirely, L7 overlaps it |
| Go+TS monorepo / cold | 2.73 s | ~2.1 s: Go parse+extract ~4.6 CPU-s over 4 workers (1.15 s) + TS 0.1 s + rules 0.5 s (consumer) + load 0.13 s + fixed ~0.2 s | ~0.4 s serial layer write (L5 or pipelining), ~0.1 s Go traversal (next target 1), merge overhead |
| Go+TS monorepo / warm | 1.44 s | ~1.0–1.1 s: read+hash 21 MB of inputs (0.13 s) + decode needed facts (~0.15 s with binary encoding) + rules 0.54 s + fixed ~0.2 s | JSON decode + FNV verification of 47 MB of layer blobs (L5), driver toolchain probes (next target 6) |
| OAIZ core / cold | 6.15 s | ~5.2 s: rules 3.7–3.9 s (bounded by one consumer rule) + Go parse ~1.2 s + fixed ~0.3 s | Serial layer write and merge; beyond that the profile is consumer-bound (L4 measured −1.9 s on the base engine) |
| OAIZ core / warm | 4.73 s | ~4.3 s: rules 3.78 s + restore ~0.25 s + load 0.13 s + fixed ~0.15 s | ~0.4 s of polint-side restore/fixed cost (L5); the rest is consumer code (L4 measured 5.79→3.86 s on the base engine) |
| OAIZ frontend / cold | 1.53 s | ~1.2 s: TS parse ~0.3 s + rules 0.66 s + load 0.13 s + fixed ~0.15 s | Layer write, TS merge |
| OAIZ frontend / warm | 1.37 s | ~1.1 s: rules 0.68 s + restore ~0.15 s + load 0.13 s + fixed ~0.15 s | JSON decode (L5) |

**Post-L4 update (2026-09-29):** the OAIZ rows above predate L4. [10](10-next-lever.md) §3 re-derives them on the fixed tree:

- **core warm:** floor ~2.6 s against 3.12 s measured on 0.4.2 (the rules term fell from 3.78 s to 2.17 s);
- **core cold:** ~3.8 s against 4.60 s;
- **frontend:** ~1.0 s warm and ~1.25 s cold, against 1.23 s and 1.56 s;
- **new full-repo rows:** a ~4.7 s warm floor against 9.20 s measured, most of that gap the metrics-trigger cliff;
- **new edit-tier rows.**

**Unsound floor, for contrast:** replaying the previous report when no input file's mtime changed
(no reading, no parsing, no rules) lands around 0.05–0.15 s for any repository. It is unsound because
both measured consumers' rules read the filesystem outside their declared inputs, mtime has coarse
granularity and is subject to clock skew, and toolchain or environment changes can alter results
without touching the repository at all. Nobody should benchmark against this number or ship it as a
default.

## The two measured rejections

Keep these from being re-proposed without re-measuring:

1. **Cursor-based Go tree walk.** Hypothesis: replacing indexed `named_child(i)` recursion with a
   single `TreeCursor` pre-order walk would remove quadratic-looking `named_child`/`parent` costs.
   Measured (dev A/B, Go+TS monorepo, 3 interleaved rounds, host-direct, 4 jobs): cold wall
   3.76→4.16 s, cold CPU 9.31→10.02 s (**+7%**), warm unchanged, go.syntax digest identical. Cursor
   stepping and its per-call allocation cost more than `named_child(i)` on narrow AST nodes; the
   widest nodes are ~3k children, so the quadratic term is only ~0.1 s. Not landed.
2. **Rule-host builds without thin-local LTO.** Measured and not shipped as a quiet default — it is
   documented as owner-decision lever **L6** instead, because it is a genuine trade (−22% build time,
   but +4.7% cold / +10.9% warm analysis time), not a strict win.
