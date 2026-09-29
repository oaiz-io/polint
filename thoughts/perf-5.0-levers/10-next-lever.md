# 10 — After L4 shipped: re-derived gaps, the lever ranking, and the next lever (2026-09-29)

Situation: **every tier, after the consumer-side span-helper fix (L4) shipped in OAIZ.** Status:
**research — the pick is made and its de-risking spike is designed; nothing is built.** Docs only.

Files 01–09 describe the world before L4 shipped. This file measures that world again where the
old numbers went stale, re-ranks every unshipped lever for what is left, picks the next one, and
designs the experiment that decides whether to build it. The [README](README.md) carries
"Post-L4 update (2026-09-29)" markers that point here; nothing in 01–09 is rewritten.

## Summary

- **L4 shipped** (oaiz-io/oaiz#4800, released v1.0.3358). The bounding core rule,
  `local/backend-endpoint-authority`, now takes ~2.2 s (3.7–4.2 s before), and it still bounds the
  core rules phase (~2.2 s, was 3.8 s).
- **OAIZ still runs the 0.4.1 engine.** Its pack pins `polint = "0.4.1"`. The pin bump that is
  open as oaiz-io/oaiz#4805 is worth **−1.39 s on every warm core run (4.51 → 3.12 s, −31%)**,
  −1.27 s cold, −1.25 s in the edit loop, and about −0.5 s on frontend runs, with identical output
  (measured today, §3).
- **The profile runs are near their floors after that bump.** A warm core run is 70% consumer rules;
  what polint can still remove is about 0.5 s against a sound floor of about 2.6 s.
- **The edit loop is cheap.** This is its first measurement: +0.41 s over warm on core, +0.31 s on
  frontend, almost all of it kernel cache work. L3 has nothing to win on these rules; parked.
- **The default `polint check` is not near its floor.** A full-repo OAIZ run is the shape polint's
  generated agent skill tells agents to run. It is 9.2–9.5 s warm on 0.4.2, and about 60% of that is
  polint. **One metrics rule in the default rule set costs every such run 3.6 s warm (38%), 5.4 s
  in the edit loop, 4.3 s cold and about 450 MB of peak RSS.** That was measured by unregistering
  that one rule and changing nothing else. The rule itself runs for ≤ 0.18 s; the rest is the
  kernel losing its deferred syntax metadata and restoring metrics for 17,552 files.
- **The compile is unchanged.** It measured 206.5 s fresh (0.4.2, 4 jobs). The `polint` crate
  alone takes 146.7 s, 137.3 s of which it is the only unit compiling. Peak rustc RSS is 3.18 GB.
  The compile is paid again on every release a consumer adopts; OAIZ pinned six engine versions
  between 2026-08-25 and 2026-09-27. A consumer that installs the driver with `cargo install` —
  the README's first install option — compiles the engine twice per release (+201.1 s).
- **Pick: close the metrics-trigger cliff.** Keep syntax metadata deferred when a plan's only
  trigger capabilities are metrics, and let a warm metrics hit skip its canonical re-projection.
  It is in-contract, days of work, with a measured ceiling of about 3.4 s per default full-repo run.
- **L1 stays the next strategic lever.** Its E1/E3 prototype is designed in §6, starting from a
  first measured number: the thin-SDK dependency floor is 15.9 s and 328 MB.
- **Do first, no lever decision needed:**
  - merge oaiz#4805;
  - install the driver from the release binary rather than with `cargo install` (swap the docs'
    order; switch OAIZ's CI and dev setup).
- **Killed or downgraded:**
  - L2: L4 absorbed its headline.
  - L3: the edit penalty is 0.3–0.4 s and the bounding rules are cross-file.
  - L7: measured ceiling ~41 s.

## 1. What changed, and five corrections

1. **L4 shipped** (oaiz-io/oaiz#4800, v1.0.3358). The pack now computes spans through a per-file
   line index. On a copy of the fixed tree, the core rules phase is 2.17–2.32 s. The single rule
   above bounds it at 2.09–2.20 s, in every tier and on both engines.
2. **The OAIZ pack still pins `polint = "0.4.1"`.** The post-L4 numbers first quoted for this
   re-analysis (core warm ~4.2 s, full-repo warm ~9 s) were therefore produced by the 0.4.1 engine,
   without anything #129 shipped. The analysis runs inside the rule host, and the rule host links
   the pinned engine, not the driver. §3 measures both engines on the same post-L4 rules.
3. **"Core warm ~4.2 s is at the sound floor" does not hold.** The README's ~4.3 s floor was built
   from the pre-L4 rules term (3.78 s). L4 cut that term to 2.17 s, and the 4.2 s run is on the older
   engine. Both halves of the comparison moved; the floor is now about 2.6 s (§3).
4. **The driver named for these measurements** (`/workspace/polint/target/release/polint`) reports
   `polint 0.4.0` (built from `e47258c4`), not v0.4.2. That does not change analysis numbers,
   because the pinned engine inside the host does the analysis. But the driver's version is part of
   the machine-global rule-host store key, so it decides whether a run restores a host or compiles
   one (§3, table B).
5. **Scope drift.** OAIZ core grew from 2,598 to 2,703 Go files (+4%) and the full repo from 16,456
   to 17,552 files (+7%) between the campaign's baseline commit and the span-fix commit.
   Cross-session comparisons also carry host drift of up to ~10%; every before/after claim in this
   file comes from one interleaved session.

## 2. What was measured for this file

Campaign host and envelope (6-core cgroup quota, 4 jobs; see [09](09-methodology.md)), 2026-09-29,
between about 17:07 and 18:15 UTC. Raw evidence lives at `/opt/data/polint-perf5/post-l4/` on the
measurement host; its `README.md` indexes every script, run and build.

- **Tree:** a `git archive` copy of the span-fix worktree (OAIZ `a06c8323`). The kernel's own file
  counts match the real worktree: core 2,703 Go files (20 MB), frontend 2,502 TS files (17 MB), full
  repo 17,552 files (61 MB: 4,888 Go, 12,664 TS).
- **Arms:**
  - *0.4.1 host:* the rule host OAIZ runs today — crates.io polint 0.4.1 plus the fixed pack, the
    exact bytes taken from the machine-global store.
  - *0.4.2 host:* the same pack pinned to crates.io 0.4.2. Its lockfile delta is `polint` and
    `polint-macros`, plus the SQLite chain #129 dropped; nothing else.
  - *Driver path:* the 0.4.0 driver named above, run in the real worktree, where it restores that
    same 0.4.1 host from the store.

  The driver adds ~0.05 s before the host starts and ~0.03 s after it ends, which is within noise
  for a whole run.
- **Tiers:**
  - cold: the arm's cache wiped, rules target kept;
  - warm: nothing removed;
  - **edit:** one never-seen line comment appended, before each attempt, to one p75-size file in
    scope (a Go file for core, full repo and code-health; a TSX file for frontend), restored after
    the round. This is the tier [08](08-next-targets.md) item 7 flagged as unmeasured.
- **Protocol:** three rounds per cell, with arm order rotated per round. Each sample waits for a
  quiet host and is retaken if it ran throttled (> 1 s of cgroup throttling) or under > 1.5 foreign
  cores. A test suite in the same container made the first batch unusable (15 of 24 samples ran
  throttled or under foreign load); the guarded re-run retook 6 attempts. Every attempt is kept on
  disk.
- **Output identity:** the 0.4.1 and 0.4.2 hosts write identical reports once the engine version
  string is normalized, in every tier including edit. That holds for 74 diagnostics on core, 16 on
  frontend, 677 on the full repo and 181 on code-health.
- **Builds:** a fresh-target build of the 0.4.2-pinned pack with `cargo --timings`, the E1
  dependency floor (§6.2), and `cargo install polint --version 0.4.2 --locked`, all at 4 jobs with a
  warm registry.

## 3. The gap table, re-derived

**A. Analysis tiers** (seconds, medians of three clean interleaved samples)

| Workload / tier | Campaign, pre-L4 (0.4.1 → FINAL) | Now: as deployed → after oaiz#4805 | Sound floor, post-L4 (estimate) | Gap left after #4805 | Who pays |
|---|---|---|---|---|---|
| OAIZ core / cold | 7.51 → 6.15 | **5.89 → 4.60** | ~3.8 | ~0.8: layer and per-file cache writes, merge | OAIZ's backend CI job: it restores a cached rule-host build but deliberately no analysis cache, so every CI core run is this tier |
| OAIZ core / warm | 5.97 → 4.73 | **4.33 → 3.12** | ~2.6 | ~0.5: JSON restore of a 61.5 MB layer (L5) | no-change reruns |
| OAIZ core / edit | (unmeasured) | **4.78 → 3.53** (host) | ~2.7 | ~0.9: per-file restore plus a rewrite of the whole 61.5 MB layer | the inner loop |
| OAIZ frontend / cold | 2.10 → 1.53 | **2.09 → 1.56** | ~1.25 | ~0.3 | fresh caches |
| OAIZ frontend / warm | 1.74 → 1.37 | **1.76 → 1.23** | ~1.0 | ~0.2 | no-change reruns |
| OAIZ frontend / edit | (unmeasured) | **1.98 → 1.54** (host) | ~1.05 | ~0.5 | the inner loop |
| OAIZ code-health (core scope) / warm | (unmeasured) | **2.44 → 1.99** (host) | ~0.6 | ~1.4, of which ~1.0 is the metrics cliff: providers 1.64 s here vs 0.65 s for the same files in the core profile; the rule itself is 34 ms | the owner's code-health target |
| OAIZ full repo / cold | 612 (0.4.1, n=1) | **14.45 → 14.00** | (not re-derived) | ≥ 4.3: the metrics cliff alone | default `polint check` on a fresh cache |
| OAIZ full repo / warm | 594 (0.4.1, n=1) | **10.12 → 9.20** | ~4.7 | ~4.5: metrics cliff 3.4, JSON restores, report tail | agents following polint's generated skill; anyone running `polint check` without a profile |
| OAIZ full repo / edit | (unmeasured) | **10.84 → 10.29** (host) | ~4.8 | ~5.5 | the same, in their inner loop |
| Go+TS monorepo / all | FINAL 224.35 / 2.73 / 1.44 | not re-measured: L4 does not touch it | README floors stand (~2.1 cold, ~1.0–1.1 warm) | README gaps stand | — |

"(host)" marks rows measured host-direct (0.4.1 → 0.4.2 host), since the real worktree is not
edited; add ~0.05–0.1 s for the driver.

What the post-L4 runs are made of:

| 0.4.2 run | Rules phase (consumer) | Syntax restore | Metrics | Load + report tail |
|---|---|---|---|---|
| core warm (3.12 s) | 2.17 (70%) | go 0.44 | — | 0.27 |
| frontend warm (1.23 s) | 0.69 (56%) | ts 0.21 | — | 0.25 |
| full repo warm (9.20 s) | 3.46 (38%) | go 1.93 + ts 1.00 | 1.56 | 0.84 |

The floors are estimates built the README's way, not measurements. Each is the measured rules
phase, plus a restore at L5's projected binary-decode speed (one third of today's JSON restore),
plus the measured load and report tail, plus the driver. The cold floors keep the parse spread over
4 workers. The full-repo floor also assumes the deferral and the metrics memo of §6.1.

**B. The compile tier**

| Event | Measured now | Under today's contracts | Under L1 (projection) | Who pays |
|---|---|---|---|---|
| Fresh machine, agent sandbox, CI cache miss: 0.4.2 pack, fresh target | **206.5 s**, 659 CPU-s, **3,177 MB** peak rustc RSS | ≈ the build (the README floor stands) | 15.9 s dependency floor (measured, §6.2) + the thin SDK (unmeasured) + the pack (5.7 s measured) | new machines, ephemeral agents, CI misses |
| Adopting a polint release, dependencies already built | ~152 s: `polint` 146.7 s + pack 5.7 s, from the same build's unit timings | same | thin SDK + pack (unmeasured), or nothing if the SDK did not change | every machine and CI cache scope; OAIZ pinned 0.2.1, 0.3.0, 0.3.2, 0.3.7, 0.3.9 and 0.4.1 between 2026-08-25 and 2026-09-27 |
| Driver installed with `cargo install polint --locked` | **201.1 s**, 3,191 MB | same | same: L1 moves the engine *into* the driver, so only a prebuilt driver escapes it | README and AGENT-PLAYBOOK followers; OAIZ CI and dev setup |
| Checkout at a new directory depth | full compile: the store key hashes one numbered "absent" line per candidate `.cargo/config` path at every ancestor of the repo root. The same tree restored in a 5.8 s run at depth 2 and started a compile at depth 5 | fixable in-contract (§4, rank 5) | — | worktree and agent tools that place checkouts at varied paths |
| Rules-only edit, dependencies built | 6.5 s (cargo's own report) | — | similar | rule authors |

Build shape (fresh 0.4.2 target, 252 units):
- Dependencies end at 63.5 s. `polint` starts at 54.1 s, then runs 46.1 s of single-threaded
  frontend and 100.5 s of codegen with ThinLTO.
- Mean active units: 3.97 before `polint` starts, 1.09 while it compiles, 1.84 over the whole
  build. The whole build averages 3.19 busy cores of 4 (659 CPU-s over 206.5 s).
- The compile is now **66× a warm core run** and **22× a warm full-repo run** on 0.4.2.

## 4. The levers, re-ranked for the post-L4 world

**Do first.** These two are not levers: they need no design and no contract decision, and each beats
everything below on return per hour.

| | Action | Worth now (measured) | To whom | Effort |
|---|---|---|---|---|
| 0a | Merge oaiz#4805 (pin 0.4.2) | core warm −1.39 s (−31%), cold −1.27 s, edit −1.25 s; frontend warm −0.49 s; full-repo warm −0.91 s; identical output | every OAIZ run, CI included | one merge, then one ~152 s rule-host rebuild per machine and CI scope |
| 0b | Install the driver from the release binary: list it first in `README.md` and `docs/AGENT-PLAYBOOK.md` (both lead with `cargo install`); OAIZ's CI and dev setup switch to the release asset | −201.1 s and one 3.19 GB compile per release, per machine and CI scope | README and playbook followers, agents in fresh sandboxes (their first-run compile halves), OAIZ CI | hours |

**Levers**, ranked by expected return per effort:

| Rank | Lever | Worth now | To whom | Effort | Verdict |
|---|---|---|---|---|---|
| 1 | **Metrics-trigger cliff** (new) | **Measured:** one metrics rule costs a default full-repo OAIZ run 3.56 s warm, 5.40 s edit, 4.29 s cold and ~450 MB RSS; ≤ 0.18 s of that is the rule | everyone who runs `polint check` without a profile in a repo with any metrics rule; code-health runs (~1.0 s of a 1.99 s warm run) | days, in-contract | **The pick** (§5) |
| 2 | L1 thin SDK + prebuilt engine | Per adopted release: the 146.7 s engine compile is replaced by a thin SDK. Per fresh environment: 206.5 s → a measured 15.9 s floor + SDK + 5.7 s pack. Compile RSS: 3.18 GB → a 0.33 GB floor. Adds a per-run snapshot (unmeasured) | fresh machines and agents, every release adoption, CI misses | a one-week prototype, then weeks; breaking | the next strategic lever; prototype in §6.2 |
| 3 | L5 binary layers + word hash | Warm restores decode 61.5 MB of JSON on core (0.44 s), 29.7 MB on frontend (0.21 s) and 188 MB on the full repo (Go 94.0, TS 46.5, metrics 47.6). Projected −0.15…−0.3 s on profile runs, ~−0.9 s on the full repo once rank 1 lands. Also shrinks the edit loop's layer rewrite | every warm and edit run | ~1 week + cache-protocol bump | keep; after rank 1; doubles as L1's snapshot codec |
| 4 | Orphaned layer blobs (new) | A layer miss evicts the stale manifest but never its content-addressed blob, which stays until `polint cache prune`. One edit leaves +61.5 MB (core), +29.7 MB (frontend), +141.6 MB (full repo) | inner-loop disk; CI that saves `.polint/cache` | small | in-contract fix |
| 5 | Store-key depth (new) | Byte-identical checkouts at different depths miss the machine-global store and compile (table B) | worktree and agent tools with varied paths | small + soundness review | in-contract fix |
| 6 | [08](08-next-targets.md) small targets | Summary rows: the report tail is 0.48 s on the full repo vs 0.13 s on core. Toolchain probes: part of the driver's ~0.05 s start. `GoTests::related_for_file`: three heavy OAIZ core rules take `GoTests` (unmeasured there). Fused Go walks: cold only | per run | small each | backlog |
| 7 | L6 LTO off | −46 s per rebuild vs +4.7% cold / +10.9% warm (measured on pre-L4 rules). At +10.9% of a 3.12 s run, break-even is ~135 warm runs per rebuild (projection) | ephemeral CI and agent containers | none (an env var) | document as a knob, not a default |
| 8 | L7 crate split | Measured ceiling: at the same total work the build cannot beat 659 CPU-s ÷ 4 jobs = 165 s, so a split can save at most ~41 s of 206.5 s. It would have to overlap the 46.1 s single-threaded frontend | fresh builds | weeks; breaks the two-package architecture | downgrade: L6 buys as much for free, L1 removes it |
| 9 | L3 per-file rule caching | The edit penalty is +0.41 s (core) and +0.31 s (frontend), and it is kernel work, not rules. The bounding core rule collects endpoints across files before checking them; three heavy core rules pair files with tests | the inner loop | an SDK addition | park until some consumer's rules are per-file-bound |
| 10 | L2 rule memoization | L4 absorbed the 594 → ~7 s headline. What is left is the rules phase of a no-change rerun: 2.2 s core, 0.7 s frontend, 3.5 s full repo. Its blocker grew: four files of the OAIZ pack call `std::fs` | no-change reruns | a contract | kill as a performance lever |

Notes on the changes:

- **L5 moves up** because the default full-repo run decodes 188 MB of JSON layers per warm run.
- **L3 moves down** because the edit loop turned out to be kernel-bound and small. Its premise in 03
  ("heavy rules are per-file scans") holds for where the time went inside the pre-L4 rules. It does
  not hold for their outputs: those depend on collections across files, so a per-file cache could not
  cut the bound without a map/reduce rule API.
- **L2's value collapsed with L4.** Its blocker grew at the same time.

## 5. The pick: close the metrics-trigger cliff, argued both ways

### For it

1. **It is the largest per-run polint-side cost left anywhere in the measured matrix.** The A/B below
   removes one rule and changes nothing else (full repo, 0.4.2 host, same session, interleaved):

   | | cold | warm | edit | peak RSS (warm) |
   |---|---|---|---|---|
   | all 23 rules | 13.27 | 9.49 | 11.14 | 955 MB |
   | without `local/code-health-metrics` | 8.98 | 5.92 | 5.74 | 502 MB |

   Without the rule the kernel logs `requested_capabilities={}`, so deferral is back on:
   - go.syntax warm: 1,959 → 1,015 ms;
   - ts.syntax warm: 1,011 → 366 ms;
   - metrics: 1,578 → 3 ms.

   The rule itself ran for 108–182 ms.
2. **It sits on the default path.** `polint check` without a profile runs every discovered rule, and
   polint has no way to keep a rule out of default runs. polint's generated agent skill, which OAIZ
   ships in its repository, tells agents to run exactly that. The metrics views (`FileMetrics<'_>`,
   `FunctionMetrics<'_>`, `ComplexityMetrics<'_>`) are a showcased SDK pattern, so every consumer
   that adopts them meets the same cliff.
3. **The mechanism is known.** The kernel defers syntax metadata only when a plan requests no
   trigger capability (`syntax_only_rule_check` in `analysis_kernel/mod.rs`), and the three metric
   capabilities are trigger capabilities. A warm metrics hit still rebuilds
   `CanonicalMetricsContext` before it can read its layer: it re-fingerprints every file's full source
   with byte-wise FNV and walks every function.
4. **It is in-contract.** No manifest, cache-protocol or rule-API change. The deferral already has an
   eager-versus-deferred identity test pattern to extend.
5. **It is days of work, and the return starts on merge.**

### The strongest case against

1. **None of OAIZ's measured day-to-day commands hits it.** `make polint-core`, `make polint-frontend`
   and both CI jobs are profile runs with `requested_capabilities={}`. Its value rests on how often
   agents and humans run the default check, which nobody has measured.
2. **The fix is not a flag.** The deferral's byte-identity argument rests on nothing interning a
   stable key between the deferred restores and the point the metadata is recorded
   (`record_deferred_syntax_metadata`). The metrics provider interns keys for its own facts right
   there. Keeping every id, and therefore every digest, identical may need a redesign of when metric
   keys are interned — and could shrink the win to the metrics memo alone.
3. **The compile dwarfs it per event.** One adopted release costs ~152 s, or ~353 s with a
   `cargo install` driver. That is 40–100 default full-repo runs' worth of the cliff. A fresh agent
   sandbox pays 206.5 s (plus 201.1 s) and 3.2 GB before its first check.
4. **A consumer-side workaround would exist** if polint could keep a rule out of default runs: OAIZ
   would run its code-health rule only through its own profile. It cannot today, which is itself a
   product gap.

### Why it still wins

- **Return per effort.** Days, for up to 3.4 s of kernel work on every default run of the measured
  consumer (36% of the run) and 5.2 s in its edit loop. L1 is a week of prototype, then weeks of a
  breaking migration. The compile's cheapest half, the driver, is taken by action 0b for hours of
  work.
- **Who pays.** The default path is the path the product tells agents to run. The warm profile runs
  are already within ~0.5 s of their floors after #4805; the default full-repo path is ~4.5 s from its
  floor, and this cliff is most of that distance.
- **Risk.** The determinism concern is testable in a day with the existing deferral tests. If ids
  cannot be kept identical, the metrics half still stands. A warm metrics hit costs nearly what a
  cold derivation does: 1,559–1,578 ms vs 1,748–1,807 ms on the full repo, and 353 vs 369–373 ms on
  code-health. That points at work both paths share — the canonical re-projection, which a memo keyed
  on the syntax providers' native digests can skip — rather than at the derivation a hit already
  skips. Step 1 of §6.1 confirms or refutes that.
- **It does not block L1, and it makes L1's test honest.** E3 budgets the snapshot tax against the
  warm analysis wall. Measured against a 9.5 s run that carries a 3.4 s cliff, the snapshot would
  get a budget it will not have once the cliff is gone.

Against the "ice-cold economics" framing, per machine and per adopted release:
- **Compile, today:** ~152 s, plus 201.1 s with a `cargo install` driver.
- **This cliff:** 3.4–5.2 s per default run.

Below ~40 default runs per release, L1 matters more for that machine. Above it, the cliff does.
Both are worth doing; the cliff is days and L1 is weeks, so the cliff goes first.

## 6. The de-risking plans

### 6.1 The pick: metrics-cliff spike (2–3 days, throwaway branch)

Measure on the OAIZ full repo and the code-health profile, 0.4.2 base, with the §2 protocol.

1. **Attribute (half a day).** Put throwaway tracing spans around:
   - `CanonicalMetricsContext::from_db`, `metrics_layer_key`, the layer read and validation,
     `restore_metrics_layer_payload` and `refresh_metric_metadata`;
   - the metadata rows of the syntax restores.

   Output: how the 1.58 s warm metrics stage and the ~1.6 s of lost deferral split.
2. **Deferral with metrics (one day).** Let `run_with` defer syntax metadata when the only trigger
   capabilities are the three metric ones. Then settle how the metrics provider interns its keys, so
   that the database keeps the stable-key ids an eager run would produce — or show that no output
   reads those ids. Gates:
   - the eager-versus-deferred identity tests, extended to a metrics plan: identical
     `functions()`, diagnostics, report and stable-key ids;
   - golden tests;
   - a debug-build run with validation on the OAIZ full repo.
3. **Warm metrics memo (half a day to a day).** Key the canonical metrics inputs on the syntax
   providers' native output digests, the shape `27bd045c` used for the Go projection, so that a
   warm hit skips the re-projection.

- **Kill:** step 2 cannot keep ids identical without interning every syntax key eagerly, *and*
  steps 2 + 3 together recover < 1.0 s of OAIZ full-repo warm. Fold what remains into L5 and stop.
- **Green-light:** ≥ 2.0 s off OAIZ full-repo warm (9.2–9.5 s today), with normalized-identical
  output. Profile runs (core and frontend warm) stay within noise, and code-health warm (1.99 s)
  improves by ≥ 0.5 s. Then ship it as an ordinary in-contract PR, measured the way #129 was.

### 6.2 The next lever: L1's E1 + E3 (one week, after 6.1)

**Step 0 — measured for this file: the thin-SDK dependency floor.** A crate that depends on exactly
the external crates the SDK-side modules name today builds fresh in **15.9 s at 4 jobs**, with 62.7
CPU-s, a **328 MB** peak rustc RSS and 62 units (40 crates).
- The SDK-side modules are `sdk/`, `core/{rule,db,capability,labels,metadata}.rs`,
  `policy_queries.rs`, `rule_manifest.rs`, `rule_error.rs`, `runner/` and `internal_core/`. Of
  those, `sdk/`, `core/rule.rs`, `core/db.rs`, `policy_queries.rs`, `rule_manifest.rs` and
  `rule_error.rs` alone are ~17.6k lines before any split; the prior research estimated 14–18k for
  the whole SDK.
- Their external crates are serde, serde_json, `toml` (because `RuleConfigValue` is `toml::Value`),
  globset, anyhow, thiserror, rayon, tracing and polint-macros, plus the OAIZ pack's own `regex`.
  clap is excluded: the rule-process wire needs no CLI parser.
- The critical path is the regex chain (`regex-syntax` 7.0 s → `regex-automata` 10.8 s), which both
  `globset` and the pack need.
- cargo counts 62 units for this floor alone, which is already around the prior research's 60-unit
  kill line (its §9.4). By time it is 13× faster than today's build, so E1 should judge time, not
  units.

**E1 — closure (three days).** Build a throwaway `polint-sdk` spike crate from the modules above plus
the JSON-report part of `diagnostics/`, and stub engine-side paths. The build is compile-cost only,
with no behaviour. Point a copy of the OAIZ pack at it through
`polint = { package = "polint-sdk", path = … }`, rule sources unchanged. Build fresh with
`cargo --timings` at 4 jobs and at 2 jobs (the size of OAIZ's frontend CI runner). Record units,
wall time, peak rustc RSS and target bytes, plus the rebuild after touching the SDK (the per-release
cost under L1).
- **Kill:**
  - the fresh thin-SDK pack build takes > 52 s at 4 jobs (less than the prior research's 4×
    against today's 206.5 s);
  - or peak compile RSS exceeds 1 GB;
  - or the OAIZ pack needs rule-source changes the SDK cannot absorb.
- **Green:** ≤ 35 s at 4 jobs (≥ 5.9×) and ≤ 0.6 GB.

**E3 — snapshot tax (two days, on the post-6.1 kernel).**
- At the point rules start, serialize what OAIZ's planned rules read (`SourceFiles` including text,
  imports, `GoTests`, string literals, metrics) with serde_json and with one binary codec. Record
  bytes, serialize and deserialize time and the RSS delta, on OAIZ core, frontend and full repo.
- Also measure the design that adds no snapshot at all: the rule process reads the
  already-validated layer blobs and the source files itself, so nothing is restored twice.
- **Kill:** the best design's round-trip is > 15% of warm analysis wall on OAIZ core (> ~0.47 s of
  3.12 s), or > 25% on the full repo.
- **Green:** ≤ 5% on core (≤ ~0.15 s). At that tax, L1 is net-positive for anyone who runs fewer
  than ~1,000 checks per adopted release (~146.7 s saved ÷ 0.15 s). This is stricter than the prior
  research's 15%/25% budgets, which were set when warm runs were several times longer. At 15% of
  today's 3.12 s core run, the tax repays the compile within ~300 runs, a count an agent-heavy
  machine can pass between two of OAIZ's pin bumps.

**Decision rule:**
- E1 and E3 both green: start E2 (the `core/db.rs` read/write split spike, 6,358 lines now) and the
  phased plan in `research/code-preserving-rule-build/`.
- E1 green, E3 between 5% and 15%: do L5 first and re-run E3 with its codec.
- E1 red: shelve L1. The compile answer is then prebuilt driver installs, the store-key depth fix,
  and L6 as a documented knob.

## 7. Still unmeasured

- **The Go+TS monorepo.** Not re-run: L4 does not apply to it, and this pass had no mandate to
  measure it. Its current engine pin, its edit tier and whether it runs any metrics rule are all
  unknown here.
- **True ice-cold today** (fresh `CARGO_HOME`, with crate downloads), and anything at 2 vCPUs (OAIZ's
  frontend CI runner size).
- **Edit loops touching many files** (branch switches), and OAIZ's frontend CI job. That job restores
  a whole `.polint/cache` saved under a lockfile-and-toolchain key, so it runs edit-like against an
  older tree.
- **Run frequencies:** no-change reruns, default versus profile runs, and runs per adopted release
  per machine. Every break-even in this file is stated as a threshold for that reason.
- **Lever-specific gaps:**
  - L6's penalty on post-L4 rules;
  - L7, which remains unprototyped;
  - L1's thin-SDK compile (E1) and snapshot tax (E3);
  - the split of the metrics cliff (§6.1 step 1).
- **The floors** in §3, which are estimates.
