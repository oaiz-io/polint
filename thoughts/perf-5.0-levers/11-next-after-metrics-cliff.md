# 11 — After the metrics-cliff fix: where things stand, and the next improvement (2026-09-30)

Situation: **every tier, after the metrics-trigger cliff fix merged** (PR #131, merged as
`a9c4e7a8`, released as v0.4.3 `5b4a9034`; its `crates/` tree is identical to the measured branch
tip `35411d4f`). Status: **research. A pick is made and its spike is designed. Nothing is built.**
Docs only.

[10](10-next-lever.md) picked the cliff and designed the §6.1 spike that closed it. This file takes
stock after that fix. It re-reads the fix's raw evidence and re-ranks what is left. One new item
surfaced, and it outranks the documented leftover. Files 01–10 are not rewritten.

## Summary

- **The cliff is closed.** On the Go+TS-heavy OAIZ full repo, a default `polint check` runs:
  - warm: 9.05 → **6.00 s**;
  - cold: 13.70 → **10.40 s**;
  - edit: 9.94 → **7.16 s**;
  - peak RSS: 963 → 524 MB.

  Code-health warm went 1.99 → **1.01 s**. Every report was byte-identical.
- **New finding: the full-repo rules phase has a scheduling tax.** On the full repo the rules phase
  is now the largest stage (3.05 of 6.00 s warm). One consumer rule, `local/backend-endpoint-authority`
  (2.43 s median), bounds it. rayon's split order decides when that rule starts, not its cost.
  Across 107 clean full-repo samples it started **0.77 s late at the median, and up to 1.36 s late**.
  That moves the rules phase from 2.41 s to 3.97 s at random.
  - When the bounding rule happened to start first (21 samples), the phase took 2.52 s (median).
  - When it started more than 0.5 s late (63 samples), the phase took 3.33 s.
  - The core and frontend profiles show no such tax: their bounding rule always starts within 0.1 s.
- **The documented leftover, the cold-path metrics miss, is real but smaller.** Its measured
  ceiling is providers −1.65 s cold, −1.32 s edit and −0.30 s warm. It needs an owner call on what
  the metrics layer is for.
- **The edit loop is not "≈ cold".** Full-repo edit is 7.16 s, against 6.00 warm and 10.40 cold.
  With rules-phase luck removed, edit costs **+1.67 s** of provider work over warm:
  - metrics miss: +1.10 s;
  - Go layer restore and rewrite: +0.58 s.

  The cold-path metrics fix and L5 cover it, so it is not a separate lever.
- **The compile is untouched and is now the largest cost by two orders of magnitude.** A fresh
  build takes 206.5 s, and adopting a release ~152 s, per machine and CI cache scope. polint shipped
  three releases in three days (v0.4.1 2026-09-27, v0.4.2 2026-09-28, v0.4.3 2026-09-30).
- **Pick: cost-ordered rule dispatch.** Start the rules that took longest last time first, and keep
  output in registration order. In-contract, 2–3 days including its spike. Projected −0.6 s median
  (up to −1.4 s) on every default full-repo run that has a timing hint (warm and edit, not a wiped
  cache), with a much narrower run-to-run spread. It also stops the rules phase from swamping the
  next levers' A/Bs.
- **Then:** L1's E1 closure spike (3 days) as the strategic step. In parallel, get an owner answer
  on the metrics-layer question, because the cold-path metrics fix waits on it.
- **Do first, no lever decision needed:**
  - OAIZ bumps its pack to 0.4.3; it is on 0.4.2 today. That brings the whole cliff fix to its
    default and code-health runs.
  - The release-binary install still is not the first option in `README.md` or
    `docs/AGENT-PLAYBOOK.md`.

## 1. Sources and what was re-derived here

- **Report of record:** `/opt/data/polint-perf5/REPORT.md`.
- **Lever files and the post-L4 re-analysis:** [README](README.md), 01–09 and [10](10-next-lever.md).
- **The fix's outcome and attribution:** `/opt/data/polint-perf5/metrics-cliff/outcome.md` and
  `attr.md`.
- **The fix's raw runs:** `metrics-cliff/runs/{ab-full,ab-core,ab-frontend,ab-health,nm-full,a1-full,b1-full,b2-full,c1-full}/`.
- **PR #131's body.**

No new measurement was taken for this file. Three things were derived from existing raw runs:

1. **Stage breakdowns per workload, FINAL arm.** Medians of clean samples, from each run's
   `timeline.json` and its kernel stage log.
2. **The rules-phase schedule.** For each clean full-repo sample (every arm of the six full-repo run
   sets, n = 107), the script took each rule's start (finish time minus `elapsed_ms`) relative to the
   last provider stage. From that it found the phase end, the bounding rule and its start offset.
3. **The provider-span deltas of the no-metrics-rule arm** (`nm-full`, the same FINAL engine with
   `local/code-health-metrics` unregistered). They are used as the ceiling of candidate A, because
   wall-clock deltas on the full repo carry the rules-phase noise from item 2.

"Clean" uses the campaign's rule: ≤ 1 s of cgroup throttling and ≤ 1.5 foreign cores. Everything is
at 4 jobs on the campaign host ([09](09-methodology.md)). OAIZ numbers are from the span-fix tree
`a06c8323`: 17,552 files in the full repo, 2,703 Go files in core and 2,502 TS files in frontend.

## 2. State of the union

Seconds. "Current best" is v0.4.3 code (`01ff2227` as measured; the merged and released code
differs from it only in comments and version strings), host-direct, medians of three clean
interleaved samples, unless marked. Floors are estimates built the README's way, not measurements.

| Workload / tier | Current best (source) | Sound floor, today's contracts (estimate) | Gap left | Where the gap is |
|---|---|---|---|---|
| OAIZ full repo / warm | **6.00** (#131 FINAL; driver path 6.38) | **~4.2** (re-derived below; [10](10-next-lever.md) said ~4.7) | **~1.8** | rules-phase scheduling ~0.6–0.7; JSON layer restores (Go 0.93, TS 0.35, metrics 0.34 → L5 and A); report tail 0.46 and pre-source gap 0.37, both unattributed |
| OAIZ full repo / edit | **7.16** (#131 FINAL; 2 of 3 samples had an early bounding rule) | ~4.3 (warm floor + one file's re-parse) | ~2.9 | metrics miss +1.10 over warm; Go layer restore and rewrite +0.58; scheduling |
| OAIZ full repo / cold | **10.40** (#131 FINAL; driver path 11.06) | (not re-derived; the same engine without the metrics rule measures 8.88) | ≥ 1.5 | metrics miss path (~1.5, candidate A); scheduling; parse and layer writes |
| OAIZ code-health / warm | **1.01** (#131 FINAL) | ~0.6 ([10](10-next-lever.md)) | ~0.4 | go restore 0.42, pre-source 0.22; the rule itself is 0.04 |
| OAIZ code-health / edit, cold | **1.52 / 2.56** (#131 FINAL) | (not re-derived) | (unmeasured) | metrics stage 0.29 in both (A takes most of it); Go parse |
| OAIZ core / warm | **2.97** (#131 FINAL; = v0.4.2's 2.94 within noise) | ~2.6 ([10](10-next-lever.md)) | ~0.4 | go restore 0.41 (L5); the rules phase is 2.09 = one consumer rule |
| OAIZ core / cold | **4.28** (#131 FINAL) | ~3.8 | ~0.5 | layer and per-file writes |
| OAIZ core / edit | **3.53** (0.4.2 host, [10](10-next-lever.md); #131 does not touch this path) | ~2.7 | ~0.8 | per-file restore + rewrite of the 61.5 MB layer |
| OAIZ frontend / warm | **1.22** (#131 FINAL) | ~1.0 | ~0.2 | ts restore 0.20 (L5) |
| OAIZ frontend / cold, edit | **1.49 / 1.54** (#131 FINAL / 0.4.2 host) | ~1.25 / ~1.05 | ~0.25 / ~0.5 | TS parse and layer write; per-file restore |
| Go+TS monorepo / warm, cold | 1.44 / 2.73 (#129's campaign FINAL) | ~1.0–1.1 / ~2.1 (README) | ~0.4 / ~0.6 | L5; layer write. #131's effect, its engine pin and its edit tier: (unmeasured) |
| Compile: fresh machine, agent sandbox, CI cache miss | **206.5**, 3,177 MB peak rustc RSS (0.4.2 pack, [10](10-next-lever.md) §3B; 0.4.3 (unmeasured)) | ≈ the build itself. Under L1: 15.9 dependency floor (measured) + thin SDK (unmeasured) + 5.7 pack | **~6–10× vs L1's projection** (0 under today's contracts) | the `polint` crate: 146.7 s, 137.3 s of it on a single rustc |
| Compile: adopting a release | ~152 (`polint` 146.7 + pack 5.7) | same | same | three polint releases in three days; OAIZ adopted two of them |
| `cargo install` driver | +201.1, 3,191 MB | a release binary: ~0 | 201.1 | docs order (action 0b, still open) |

**How the full-repo warm floor was re-derived (~4.2 s).** It is the sum of five terms:

| Term | s | Basis |
|---|---|---|
| load | 0.37 | measured: reading 61 MB of sources |
| pre-source gap | 0.37 | measured, unattributed; kept whole, so the floor is conservative |
| restores | ~0.53 | Go and TS restores at L5's projected one-third speed (0.31 + 0.12), plus ~0.1 for deriving metrics from restored facts (A: derivation 23–27 ms + line counts ~81 ms, from `attr.md`) |
| rules phase | 2.43 | `max(bounding rule 2.43, total rule CPU 7.96 ÷ 4 = 1.99)` |
| report tail | 0.46 | measured, kept whole |

[10](10-next-lever.md)'s ~4.7 s carried a 3.46 s rules phase. The rules term here is lower because
§3 shows ~0.7 s of that phase is scheduling, not rule work.

**What a run is made of now** (FINAL, medians; the rules phase runs from the last provider stage to
the last rule finishing, and the tail from there to exit):

| Run | Wall | Load | Pre-source | Providers (go / ts / metrics) | Rules phase | Tail |
|---|---|---|---|---|---|---|
| full repo warm | 6.00 | 0.37 | 0.37 | 1.74 (0.93 / 0.35 / 0.34) | **3.05 (51%)** | 0.46 |
| full repo edit | 7.16 | 0.37 | 0.42 | 3.41 (1.51 / 0.34 / 1.44) | 2.45 | 0.45 |
| full repo cold | 10.40 | 0.38 | 0.41 | 6.04 (3.42 / 1.29 / 1.31) | 3.14 | 0.48 |
| core warm | 2.97 | 0.14 | 0.18 | 0.42 (0.41 / — / —) | 2.09 (70%) | 0.12 |
| frontend warm | 1.22 | 0.13 | 0.08 | 0.21 (— / 0.20 / —) | 0.66 (54%) | 0.11 |
| code-health warm | 1.01 | 0.14 | 0.22 | 0.51 (0.42 / — / 0.07) | 0.04 | 0.10 |

**The new biggest warm-stage item is the full-repo rules phase, and a quarter of it is not rule
work.**

## 3. The finding: which rule starts first decides the full-repo rules phase

Rules run as `rules.par_iter().map(run_one).collect()` (`crates/polint/src/core/rule.rs`, line 437
on `origin/main`). rayon splits the rule slice in halves and idle workers steal the right halves.
Which rules start first therefore depends on each rule's position and on steal races, not on its
cost. `collect` keeps registration order, so this has never affected output. It does affect
wall time.

Every clean full-repo sample of the fix's campaign, across all arms (BASE, FINAL, the attribution,
per-change and no-metrics-rule hosts: n = 107, 22–23 rules each):

| Quantity | Median | Range |
|---|---|---|
| Rules phase | **3.13** | 2.41 – 3.97 |
| Bounding rule's own time (`local/backend-endpoint-authority`, every sample) | 2.43 | 2.28 – 2.90 |
| Bounding rule's start after the phase began | **0.77** | 0.01 – 1.36 |
| Phase minus bounding rule (scheduling slack) | **0.77** | 0.02 – 1.36 |
| Total rule CPU | 7.96 | — |
| All other rules ÷ the 3 other workers | 1.84 | — |

- Samples whose bounding rule started within 0.1 s: n = 21, phase median **2.52**.
- Samples whose bounding rule started more than 0.5 s late: n = 63, phase median **3.33**.
- The bounding rule's own time does not depend on when it starts (2.28–2.90 either way).
- The other rules fit behind it: 1.84 s on three workers, against its 2.43 s.

So a longest-first schedule would bound the phase at ~2.43–2.52 s. That is **−0.6 s at the median
and up to −1.4 s**, and the phase would stop varying by 1.5 s from run to run.

Other workloads:
- **Core:** the same rule bounds the phase, but in all 6 FINAL samples it started within 0.08 s.
  The phase is 2.04–2.13 s against the rule's 2.01–2.08 s, so there is no slack.
- **Frontend:** the bounding rule starts at 0.02–0.03 s, so there is no slack either.
- **Code-health:** one rule.

This tax only appears when a rule set has one dominant rule and enough other rules that rayon's
split can queue it behind them. Of the measured workloads, only the default full-repo run has that
shape. The Go+TS monorepo was not checked (unmeasured).

**Why this matters beyond its own 0.6 s:** the rules phase's run-to-run spread (1.56 s) is larger
than several levers' warm effect on the full repo, and 3-sample medians hide which way each sample
fell. The fix's own no-metrics-rule A/B shows it:
- the arm *without* the rule had the slower rules phase (3.13 vs 2.72 s warm);
- the "~0 at the wall" warm residual came out of that noise, whereas the provider span shows +0.30 s.

The FINAL edit median (7.16 s) is also flattered: 2 of its 3 samples had an early bounding rule.

## 4. The candidates after the cliff fix, re-ranked

### A. The cold-path metrics miss (the documented leftover)

- **What it is.** A syntax-level rule check that requests metrics still runs the metrics provider's
  full miss path whenever its inputs memo misses: every cold run and every edit. Per `attr.md` and
  `outcome.md`, that path is:

  | Step | Cost |
  |---|---|
  | canonical metrics context from the database | ~0.3 s |
  | layer key | ~0.09 s |
  | output projection and digest | ~0.24 s |
  | payload and dependency edges | ~0.12 s |
  | writing a 47.6 MB layer | ~0.32 s |
  | the derivation itself | 23–27 ms |

  Warm, the layer's own I/O (manifest, 47.6 MB blob read, byte-wise FNV, JSON parse) is ~0.35 s.

  The change: in a syntax-level check nothing reads the metrics output identity, so derive the
  metric facts straight from the restored syntax facts and skip the canonical projection, the output
  projection, and the layer write and read.
- **Measured ceiling.** From the same FINAL engine without the metrics rule, provider span:

  | Tier | With the rule | Without | Ceiling |
  |---|---|---|---|
  | cold | 6.87 | 5.22 | **−1.65** |
  | edit | 3.75 | 2.43 | **−1.32** |
  | warm | 2.14 | 1.84 | **−0.30** |

  A pays back the derivation (~0.1 s). Code-health cold and edit: ≤ ~0.25 s of its 0.29 s metrics
  stage. It also stops each edit from writing a new 47.6 MB metrics blob, which feeds the
  orphaned-blob growth [10](10-next-lever.md) ranked 4th.
- **Who pays today.**
  - Default full-repo runs on a fresh cache. They are mostly fresh environments that also pay a
    206.5 s compile, so 1.65 s is < 1% there.
  - Default full-repo edit runs: agents following the generated skill. This is where A matters:
    −1.3 s per edit run.
- **Effort and risk.** Days. The fix's own write-up classifies it as an owner decision because it
  changes what the metrics layer is for. The layer would stop being written for syntax-level checks,
  so any consumer of it outside those checks would need its own path. Which consumers exist is
  (unverified here). The derivation must stay identical by construction: it is the same function
  over the same facts. The metric facts' deferred metadata ordering from #131 must hold. The
  existing eager-versus-deferred and per-dimension memo tests are the gate pattern.
- **Cheapest de-risk.** Half a day of reading: list every reader of the metrics layer and of its
  output identity. If none sits on a syntax-level path, the owner decision is a one-line answer.

### B. L1 — thin SDK + prebuilt engine

- **What it is.** Unchanged from [01](01-thin-sdk-prebuilt-engine.md) and [10](10-next-lever.md) §6.2.
- **Measured.**
  - Fresh 0.4.2 pack build: 206.5 s, 659 CPU-s, 3,177 MB. The `polint` crate is 146.7 s.
  - A release adoption costs ~152 s.
  - The thin-SDK dependency floor: 15.9 s, 328 MB, 62 units.
  - 0.4.3's build: (unmeasured). #131 changed 4 files (+850/−54 lines), not the dependency set.
- **What changed since [10](10-next-lever.md).**
  - polint's release cadence: v0.4.1, v0.4.2 and v0.4.3 in three days. OAIZ adopted two of them
    (oaiz#4739, oaiz#4805), and each adoption cost ~152 s per machine and CI cache scope.
  - The per-run tiers the compile competes with shrank. One adopted release now equals ~25 warm
    default full-repo runs, ~51 warm core runs, or ~250 runs' worth of the pick's saving.
- **Return.** Per adopted release: ~146.7 s → a thin-SDK rebuild (unmeasured) or nothing. Per fresh
  environment: 206.5 s → 15.9 s + SDK + 5.7 s. Compile RSS 3.18 GB → ~0.33 GB floor. E3's snapshot
  tax (unmeasured) is paid back on every run.
- **Who benefits:** fresh machines and agent sandboxes, every release adoption, and CI cache misses.
  Release engineering: frequent releases stop being a tax on consumers.
- **Effort and risk.**
  - E1: 3 days, throwaway.
  - E3: 2 days.
  - The migration: weeks, breaking (packs depend on an SDK crate), with a versioned snapshot
    protocol.
- **Cheapest de-risk:** E1 exactly as [10](10-next-lever.md) §6.2 specifies it. Its kill/green
  lines stand: kill at > 52 s or > 1 GB, green at ≤ 35 s and ≤ 0.6 GB, 4 jobs. E3 now runs on the
  post-cliff kernel it was waiting for. Its core budget stands (≤ 5% ≈ 0.15 s of 2.97 s).
  Its full-repo budget should be judged against 6.00 s, not 9.5 s.

### C. L5 — binary layers and a word-at-a-time hash

- **What it is.** Unchanged from [05](05-binary-layer-cache.md).
- **Measured now (warm, FINAL):** go.syntax 0.93 s full repo and 0.41 s core; ts.syntax 0.35 s full
  repo and 0.20 s frontend; the metrics layer I/O ~0.35 s full repo. The layers are 94.0 MB Go,
  46.5 MB TS and 47.6 MB metrics of JSON.
- **Return (projection).** At the README's one-third-of-JSON-restore assumption:
  - full-repo warm: −0.5…−0.85 s if A lands first (upper bound: two thirds of the Go+TS restore
    stages, which also contain fact pushes);
  - core warm: −0.15…−0.27 s;
  - frontend: ~−0.1 s.

  It also shrinks the edit loop's Go layer rewrite (+0.58 s over warm on the full repo) and cold
  writes. All (unmeasured).
- **Who benefits:** every warm and edit run on every workload. It doubles as L1's snapshot codec.
- **Effort and risk:** ~1 week, a cache-protocol bump (every cache misses once), a new dependency and
  a second deterministic encoding.
- **Cheapest de-risk:** a throwaway profile of the warm full-repo go.syntax stage to learn the
  decode share now that metadata is deferred. Half a day. It tells whether −0.85 s is reachable or
  whether pushes dominate.

### D. The edit-loop tier

- **Measured.** Full-repo edit is 7.16 s, not ≈ cold (10.40 s). Take rules-phase luck out and look
  at providers, and edit costs **+1.67 s** over warm:

  | Stage | Warm | Edit | Delta |
  |---|---|---|---|
  | metrics | 0.34 | 1.44 | +1.10 |
  | go.syntax | 0.93 | 1.51 | +0.58 |
  | ts.syntax | 0.35 | 0.34 | ~0 |

  On the profiles, the penalty is +0.41 s on core and +0.31 s on frontend (0.4.2, [10](10-next-lever.md)).
  On code-health it is +0.51 s.
- **Verdict: not a separate lever.**
  - A removes ~1.1–1.3 s of the full-repo penalty.
  - L5 (or a chunked layer) is what shrinks the ~0.4–0.6 s rewrite-and-restore remainder.
  - L3 (per-file rule caching) stays parked: rules are not in the edit penalty. The bounding rule is
    cross-file, and the penalty's per-rule share is ~0.
  - Edit loops that touch many files (branch switches) are still (unmeasured).

### E. What the fresh numbers newly surface

1. **Cost-ordered rule dispatch (new: §3).**
   - **Return:** −0.6 s median and up to −1.4 s on default full-repo runs that have a timing hint.
     The run-to-run spread shrinks from 1.56 s to roughly the bounding rule's own jitter (~0.6 s,
     2.28–2.90).
   - **Who benefits:** anyone running the default `polint check` in a repo whose rule set has one
     dominant rule. That is the generated skill's path for agents. CI profile runs and the measured
     profiles gain nothing.
   - **Effort:** 2–3 days including its spike.
   - **Risk:** low and testable. Execution order is not output-visible today.
2. **Report tail and pre-source gap (new, unattributed).**
   - **Measured:** the tail is 0.46 s on the full repo vs 0.10–0.12 s on the profiles; the
     pre-source gap is 0.37 s vs 0.08–0.22 s. Both scale with files × rules or with file count, not
     with diagnostics alone.
   - **Candidates:** [08](08-next-targets.md) item 3's summary rows (`files_in_scope` re-matches
     23 rules × 17,552 files after the run), report serialization, teardown of a 524 MB database,
     and input digesting before the first provider. None is attributed.
   - **Return:** ≤ ~0.5 s on the full repo (unmeasured), ~0.1 s on profiles.
   - **Effort:** half a day of throwaway spans, then small in-contract fixes.
3. **Still open from [10](10-next-lever.md) §4, both small and in-contract:**
   - orphaned layer blobs: rank 4;
   - the store-key depth: rank 5. It forces a ~200 s compile when a checkout moves directory depth,
     so it belongs to the compile family.

### The ranking

| Rank | Candidate | Worth (measured, unless marked) | To whom | Effort | Decision needed | Verdict |
|---|---|---|---|---|---|---|
| 1 | **Cost-ordered rule dispatch** (E1) | Rules phase 3.13 → ~2.5 at the median (n = 107): **−0.6 s median, up to −1.4 s** per default full-repo run with a hint; spread 1.56 → ~0.6 s. Profiles: 0 | default `polint check`: agents, humans without a profile | 2–3 days | none: in-contract | **The pick** (§5) |
| 2 | L1 E1 → E3 (B) | Per adopted release ~146.7 s; per fresh environment 206.5 s → 15.9 s floor + SDK (unmeasured) + 5.7 s; 3.18 → ~0.33 GB | fresh machines and sandboxes, every release adoption, CI misses | E1 3 days + E3 2 days, then weeks | the migration is an owner decision; the spikes are not | the next spike after the pick |
| 3 | Cold-path metrics miss (A) | Providers ceiling −1.65 cold, **−1.32 edit**, −0.30 warm (full repo); ≤ ~0.25 code-health cold and edit | default full-repo edit loop; fresh caches | days | owner: what the metrics layer is for | ask now; build when answered |
| 4 | L5 binary layers (C) | −0.5…−0.85 full-repo warm, −0.15…−0.27 core (projection) | every warm and edit run | ~1 week + protocol bump | owner: format and protocol | after A; profile first |
| 5 | Tail and pre-source attribution (E2) | ≤ ~0.5 full repo (unmeasured) | full-repo runs | half a day + small fixes | none | cheap follow-on to the pick's spans |
| 6 | Orphaned blobs, store-key depth ([10](10-next-lever.md)) | disk: +61.5–141.6 MB per edit; a full compile per depth change | inner-loop disk; varied-path worktrees | small each | none | backlog, in-contract |
| — | Edit-loop tier as a lever (D) | its +1.67 s is A (+1.1) and the Go layer (+0.58) | — | — | — | absorbed by A and L5 |
| — | L3, L2, L6, L7 | unchanged from [10](10-next-lever.md) §4 | — | — | — | parked / killed / knob / downgraded |

**Do first, not levers:**
- **OAIZ bumps its pack to 0.4.3.** No 0.4.3 bump PR exists yet. Gains on its default and
  code-health runs: full-repo warm −3.05 s, code-health warm −0.98 s; profile runs are unchanged.
  It costs one ~152 s rule-host rebuild per machine, which is the L1 argument in miniature.
- **Swap the install docs so the release binary comes first** (action 0b in [10](10-next-lever.md)).
  `README.md` and `docs/AGENT-PLAYBOOK.md` still lead with `cargo install polint --locked`, which
  costs 201.1 s and 3.19 GB per release.

## 5. The pick: cost-ordered rule dispatch, argued both ways

**What would change.** The kernel runs rules in descending order of their previous run's time and
keeps results in registration order.
- **Hints:** per rule id, the last `elapsed_ms`, stored best-effort in the analysis cache directory.
  They are outside every cache digest and outside the report, and are never read by a rule.
- **Missing hints:** a missing or unreadable hint file falls back to registration order.
- **Dispatch:** a pull queue over the worker count (longest-processing-time list scheduling) instead
  of rayon's recursive split, with results written into registration-order slots before the
  existing dedupe.
- **Surface:** no CLI flag, no report field, no rule-API change.

### For it

1. **It is the largest polint-side item on the default warm path** now that the cliff is gone:
   0.77 s median slack inside the 3.05 s rules phase, against 0.35 s of metrics layer I/O, a 0.93 s
   Go restore that L5 might cut by up to two thirds, and a 0.46 s tail. It is measured on 107
   samples, not projected from a profile.
2. **It is in-contract and output-neutral by construction.** Execution order already varies from run
   to run and has never changed output. All 98 samples with the full rule set wrote the same report
   bytes (`9c4b4d061d94`). The 9 without the metrics rule also agreed with one another.
3. **It is the cheapest item on the list:** a scheduler and a small hint file. There is no protocol
   bump and no owner decision.
4. **It fixes the measurement floor for everything after it.** The full-repo rules phase spreads
   1.56 s between samples. That is larger than A's warm effect and L5's projected effect, and it
   already muddied the fix's own no-metrics-rule A/B (§3). A and L5 are the next full-repo levers,
   and both get honest wall-clock A/Bs only once the phase stops rolling dice.
5. **It pays on the path the product tells agents to run,** in warm and edit runs alike. It is the
   same audience the cliff fix served.

### The strongest case against

1. **The compile is ~250 of these savings per adopted release.** One release adoption is ~152 s per
   machine and CI scope. polint shipped three releases in three days, and a fresh sandbox pays
   206.5 s and 3.2 GB before its first check. By machine-seconds, L1 dwarfs this pick for any machine
   that runs fewer than ~250 default full-repo checks per adopted release.
2. **It rests on one consumer rule's shape.** The whole saving exists because
   `local/backend-endpoint-authority` (2.43 s) is longer than the other rules' balanced share (1.84 s),
   and because the default rule set has 23 rules. The consumer:
   - already has a reason to speed that rule up;
   - L4 halved it once;
   - if the rule drops below ~1.9 s, the pick is worth at most what the remaining imbalance allows.

   The Go+TS monorepo's shape is unmeasured. The pick is general in mechanism but measured in one
   repository.
3. **A is bigger where it applies.** In the default edit loop A is worth −1.3 s against the pick's
   −0.6 s median.
4. **The hint makes cold runs no better.** A wiped cache has no hints, so the cold tier keeps today's
   rayon split. That tier is mostly fresh environments, so this costs little, but the pick is a
   warm-and-edit improvement only.
5. **Shared lazy state could shrink it.** If some SDK view builds a shared index on first use,
   starting the bounding rule first might move that cost onto it and lengthen it. The 107 samples
   argue against this: its own time is 2.28–2.90 s whether it starts first or last. Step 1 settles
   it directly.

### Why it still wins

- **Against L1: they are not competing for the same days.**
  - The pick is 2–3 days with return on merge.
  - E1 is 3 days of throwaway whose output is a decision, and the migration it could green-light is
    weeks.
  - Doing the pick first delays E1 by under a week and changes none of E1's inputs.

  The machine-seconds argument decides *whether L1 happens*, not *what goes first this week*. It is
  why E1 is scheduled immediately after, not parked.
- **Against A: A is blocked on an owner answer; the pick is not.**
  - Ask the A question today. The half-day reader audit above makes it cheap to answer.
  - Build A right after the pick, and measure it on a rules phase that no longer adds a random
    0–1.4 s.
- **Against the single-rule dependency:** the pick costs 2–3 days, and its kill criterion (below)
  fires if the measured saving is under 0.3 s. If OAIZ later speeds up the bounding rule, the
  scheduler remains correct and costs nothing; it simply has less to win.

### Execution plan (spike shaped like [10](10-next-lever.md) §6.1: throwaway first, then an ordinary in-contract PR)

Measure on the OAIZ full repo, code-health, core and frontend. BASE is v0.4.3. The protocol is
§1 plus these changes:
- **five** interleaved rounds instead of three, because BASE's rules phase is bimodal;
- the rules-phase schedule (the §1 script) reported for every sample alongside wall and stages.

**Step 1 — confirm the mechanism (half a day, throwaway host).**
- Build three hosts from v0.4.3 source, differing only in dispatch:
  - **(a)** today's `par_iter`;
  - **(b)** a pull queue in registration order;
  - **(c)** a pull queue ordered by a hand-written hint file from one BASE run.
- Run full-repo warm and edit.
- Output: the bounding rule's start offset and own time, and the phase length, per arm.
- Arm (b) answers whether a stateless queue is enough for this consumer, which would be simpler.
  The answer is expected to depend on the rule's registration position, and so not to generalize.

- **Kill:** in arm (c) the bounding rule starts within 0.1 s in every sample, yet the rules phase
  median stays > 2.8 s. The slack then is not scheduling; shared-init or contention would be
  bounding it instead. Record that and stop.
- **Continue:** arm (c)'s rules phase median ≤ 2.6 s, and the bounding rule's own time is within
  0.2 s of BASE's.

**Step 2 — build it in-contract (1–1.5 days).** Build the hint store, the pull-queue dispatcher and
the registration-order slots described above. Keep today's invariant that a worker runs one rule at
a time: the thread-local observed-events counter depends on it. Whether rule bodies that call rayon
internally already interleave two rules on one worker today is (unverified). Check it, and do not
make it worse.

Gates:
- a permutation test: the same rule set dispatched in shuffled orders produces byte-identical
  reports, summary rows and observed-event counts;
- a missing, corrupt or stale-schema hint file falls back to registration order;
- hints change no cache digest and no report byte: assert the digests with and without a hint file;
- a panicking rule is still isolated at any queue position;
- the golden tests, fmt, clippy, the lib suite, the integration targets, the cli suite, and the
  no-language / Go-only / TypeScript-only feature matrix.

**Step 3 — measure (1 day).** BASE v0.4.3 vs FINAL, five rounds, cold, warm and edit, on all four
workloads, host-direct plus one driver-path confirmation on the full repo.

- **Green-light:**
  - the full-repo rules phase median ≤ 2.6 s over warm and edit samples;
  - its max − min ≤ 0.6 s (today 1.56 s);
  - full-repo warm and edit each improve by ≥ 0.4 s at the median;
  - core, frontend and code-health stay within noise;
  - byte-identical reports in every sample.

  Then ship it as an ordinary PR, measured the way #129 and #131 were.
- **Kill:** full-repo warm or edit improves by < 0.3 s at the median, any output difference, or a
  > 2% regression on any profile run.

**Then, in order:**
1. E2's spans, together with Step 3's hosts: half a day, attributes the tail and pre-source gaps.
2. L1's E1, the 3-day closure spike, with [10](10-next-lever.md) §6.2's gates unchanged; E3 after
   it, on this kernel.
3. A, once the owner has answered, measured on the de-noised rules phase.
4. L5, after its half-day profile.

## 6. Where there is honestly nothing left for polint

- **The profile runs are consumer-bound.**
  - Core warm (2.97 s) spends 2.09 s in one consumer rule. What polint still owns is ~0.4 s: go
    restore 0.41 s, of which L5 might take a third to two thirds, plus ~0.45 s of load, pre-source
    and tail that are near their fixed cost.
  - Frontend warm (1.22 s) spends 0.66 s in rules, bounded by one consumer rule (0.60–0.66 s).
  - Neither run has scheduling slack. No lever in this file moves them by more than ~0.3 s.
- **The full-repo rules phase floor is 2.43 s of consumer code.** After the pick, the only way below
  it is the consumer's own rule: a faster `local/backend-endpoint-authority`, or splitting it. polint
  can report it and cannot ship it.
- **The compile is at its floor under today's contracts.** Nothing in-contract moves the 146.7 s
  `polint` crate compile beyond L6's documented trade. Only L1 changes the contract that forces it.
  The one free item is the install-docs swap (0b).
- **Deep capabilities remain irrelevant to both measured consumers.** Neither requests any. OAIZ's
  full-repo run logs `requested_capabilities` = the three metric ones only.
- **Loose ends that are not work items:**
  - the unsound mtime-replay floor stays rejected ([README](README.md));
  - the cursor walk stays rejected (+7%);
  - L2 stays killed: a no-change rerun's rules phase is consumer time that only an unsound or
    contract-breaking cache could skip.

## 7. Still unmeasured

- The pick's effect on any repository other than OAIZ. The Go+TS monorepo's rule-set shape, engine
  pin and post-#131 numbers are all unknown.
- The 0.4.3 compile. The 0.4.2 numbers are used throughout.
- The split of the full-repo report tail (0.46 s) and pre-source gap (0.37 s).
- The decode share of the post-deferral warm Go restore, which decides L5's real return.
- Readers of the metrics layer outside syntax-level checks, which decides whether A needs an owner
  call at all.
- Run frequencies: default vs profile runs, edit vs no-change runs, and runs per adopted release per
  machine. Every break-even above (~25, ~51, ~250 runs per release) is a threshold for that reason.
- Everything [10](10-next-lever.md) §7 lists that this file did not touch: true ice-cold with
  downloads, 2-vCPU runners, many-file edits, and OAIZ's frontend CI cache shape.
