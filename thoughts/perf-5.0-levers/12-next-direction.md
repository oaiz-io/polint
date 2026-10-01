# 12 — The next direction: more runtime performance, or more capability (2026-10-01)

Situation: **v0.4.4** (`db8d7ea2`, released 2026-09-30) is the engine of record. It includes the
metrics-cliff fix (#131) and cost-ordered rule dispatch (#132). Status: **research. One direction is
recommended, with a first-three-steps plan. Nothing is built.** Docs only.

Files 01–11 asked one question: which performance lever comes next. This file asks a different one.
Should polint keep spending on runtime performance (**A**), or on what rules can express (**B**)? The
owner's constraint of 2026-09-30 applies throughout. Only two things count:

- the runtime of `polint check` on real repositories;
- capability.

Anything whose value is compile time, installs, updates or distribution is out of scope. That removes:

- L1, the thin SDK;
- L6 and L7;
- the install-docs action;
- the store-key-depth item.

Files 01–11 are not rewritten. The [README](README.md) carries the pointers.

## Summary

**The recommendation is B, capability. Start with the typed middle layer that real rules are missing,
not with deeper whole-program analysis.**

- **Middle layer, defined.** Exact, syntax-tier and type-tier facts about functions, calls, structs,
  tests, routes and non-code files.
- **Where it sits.** Above today's thin syntax facts, and below the call-graph, data-flow and taint
  stack that already exists but that no consumer can run.
- **Why first.** It is what both consumer rule packs re-derive from raw text today. It is also what the
  deep stack needs before its answers stop being mostly "unknown" on real code.

The top evidence (**M** = measured for this file, **R** = re-derived from existing evidence):

1. **(M) Runtime has stopped being polint's problem in the rules phase.** I profiled three warm OAIZ
   full-repo runs on v0.4.4 (9,922 samples). The rules phase is 65% of CPU, and within it:
   - consumer code 64.9%;
   - the `regex` crate the consumer calls 28.2%;
   - **polint's own SDK and engine code 4.3%.**

   At least half of the rules phase's CPU (50.7%) re-derives syntax that polint's parsers already built:
   - comment and literal masking, 32.1%;
   - re-parsing import blocks, 9.2%;
   - bracket matching to find bodies and arguments, 9.3%.

   The rule that bounds the phase, `local/backend-endpoint-authority`, spends 65.8% of its CPU on that
   re-derivation, 23.9% in regex, and 0% in polint. No runtime lever in direction A can touch this
   floor. Typed facts can.
2. **(R) Neither consumer uses the deep capabilities polint already has, and both work around missing
   facts with text scanning.** From the rule-pack audit (§4.2):
   - **OAIZ:** 29 rules, 8,907 lines, about 85% textual (estimate). It ships a 1,716-line shared Go
     scanner.
   - **The Go+TS monorepo:** 77 rules, 20,396 lines, about 70% textual (estimate). It ships a private
     3,709-line Go parser.
   - **Deep views:** zero rules request one. They have existed since v0.4.0.
   - **What OAIZ gave up:** on 2026-05-19 it replaced its `go/analysis`-based analyzers with polint
     rules (`fd0067f8ca`), and typed AST access went with them.
3. **(M) The deep stack still cannot serve OAIZ, even at package scope.** I ran a forced `control_flow`
   scan of one 41-file OAIZ package:
   - **Default config:** the process tree passed 12 GB and was killed, after 93 s cold and 30 s warm.
     The Go semantic sidecar held 11.8 GB of it.
   - **With `include_tests = false`:** it finished in 26 s cold at a 7.8 GB peak, and 3.8 s warm. It
     also left 1,112 unknown rows in 40 of the 41 files, 1,072 of them unresolved member calls
     (`x.f(...)`).

   Scaling the deep stack first would mostly produce fast "unknown" answers. Type facts and framework
   models are what resolve those calls.
4. **(R/M) Direction A's remaining in-contract pot is bounded.** On the default agent path (OAIZ full
   repo):
   - **Warm:** about −0.9 to −1.6 s of 5.4 s.
   - **Edit:** about −1.9 to −2.3 s of 7.1 s.
   - **Cold:** about −2 s of 10.5 s.
   - **Profile runs:** at most −0.3 s.

   After that, what's left is consumer code plus about 1.2 s of fixed cost. One new lever is measured
   here (finding 5).
5. **(M) A new bug-shaped runtime lever, found and measured here.** The layer cache refuses to read
   any blob over 64 MiB (`layer_cache.rs:32`). OAIZ's full-repo Go syntax layer is 96.2 MB, so every
   warm run rejects it and rebuilds it from 5,019 per-file entries, then re-serializes and rewrites
   96 MB of JSON.
   - Raising the limit saves **−0.30 s warm**: 5.56 → 5.26 s, median of 5 interleaved rounds, all five
     paired deltas negative, identical reports.
   - The edit tier does not change.
   - This is a small in-contract fix to do regardless of direction.

**First three steps** (§6):
1. **A one-week throwaway spike.** Add rich Go structure facts to a polint branch. Rewrite scratch
   copies of OAIZ's three heaviest scanner-based rules against them. Measure lines removed, report
   parity, and full-repo wall time.
2. **Ship those facts as preview SDK views.** Go first, then TS import and call facts, plus a
   non-code-files view (#52) and the Go grammar fix (#126). Gate it on full-repo runtime not regressing.
3. **Framework model v1.** HTTP routes with their effective middleware chain, defined as data, and
   proven by rewriting the endpoint-authority rule on it.

Deep work (type-resolved member calls, a package-scoped Go sidecar, then real IFDS taint) comes after
step 3, entered through a measured gate.

**Do regardless of direction** (no design needed):
- raise or chunk the 64 MiB layer limit (−0.30 s warm, measured);
- merge #126, the Go 1.26 `new(expr)` grammar fix. 43 OAIZ files currently fail to parse, and those
  are 43 of the 691 diagnostics in every full-repo OAIZ report.

## 1. Sources, method, and what was measured

**Read:**
- the [README](README.md) and files 01–11 of this directory;
- the cost-dispatch outcome and final review, the campaign report, `report-levers.md`,
  `report-next.md` and `decisions.md` (`/opt/data/polint-perf5/`);
- `ARCHITECTURE.md`, `AGENTS.md`, `docs/` (facts, playbook, roadmaps, `docs/architecture-review/`);
- `research/` (strategy, full-app deep-capability plan, landscape indexes);
- `.planning/`;
- GitHub issues #9, #49, #52, #86–#91, #116 and #125;
- both consumer rule packs, read-only.

Five read-only research agents swept these areas, and I spot-checked their load-bearing claims against
code and git:
- the SDK surface;
- the deep engine;
- the prior capability research;
- the architecture-review line;
- the rule packs.

A sixth agent researched the external landscape on the web. Its sources are cited by URL where used.

**Measured for this file.** All runs were on the campaign host at 4 jobs, host-direct, against a
`--shared` clone of OAIZ at `2956a791a7`: the cost-dispatch bench commit, 18,039 files in scope,
23 check rules, 691 diagnostics. Raw evidence and scripts are at `/opt/data/polint-next-direction/`
(its `README.md` indexes them). None of it is committed.

- **M1, a CPU profile of the warm full repo.**
  - Host: OAIZ's pack pinned to crates.io `polint = "=0.4.4"`, wrapped in a pprof guard that writes
    folded stacks tagged with each frame's source file. Line-tables debug info only.
  - Runs: three warm runs, 9,922 samples at 499 Hz.
  - Load: the profiles ran under 2–3 foreign cores of load, so absolute times are inflated (the
    bounding rule took 2.8–3.0 s against 2.49 s in #132's quiet A/B). Shares, not seconds, are used
    from M1.
  - Sampling loss: pprof's SIGPROF timer recorded about half the samples the runs' CPU time implies,
    because pending signals coalesce. The loss does not visibly bias the split: the sampled rules share
    is 64.6%, and the rules' own logged elapsed times sum to 61–63% of each run's user+sys CPU. Shares
    *within* the rules phase, where all four workers run alike, are the ones this file leans on.
- **M2, an A/B of the layer limit.** The same host against a copy of v0.4.4 with
  `LAYER_CACHE_PAYLOAD_MAX_BYTES` raised from 64 MiB to 256 MiB. Five interleaved rounds, warm and
  one-file edit tiers, separate caches. The edit appends a comment to a p75-size Go file in `core` and
  restores it after each round.
- **M3, a deep scan at package scope.** `polint unknowns --cap control_flow` on one 41-file
  bounded-context package under `core/internal`, cold and warm.
  - Driver: the cost-dispatch driver (0.4.3 plus #132; its deep code is identical to v0.4.4, since
    nothing under `analysis_neutral`, `go/mir`, `ts/mir` or `provider.rs` changed after 2026-09-21).
  - Safety: process-tree RSS was sampled and killed at a 12 GB ceiling, because this 22 GiB container
    is shared.
  - Arms: the repository's default config, and `[languages.go] include_tests = false`. That setting
    was written into the clone only and restored afterwards.

**Environment notes:**
- The brief described `/workspace/polint` as v0.4.4. It is checked out at `e47258c4` (v0.4.0+1). Every
  source citation below is from a `git archive` of `db8d7ea2`.
- The 2026-09-17 deep-capability report that memory points at no longer exists. Its folder at
  `/opt/data/polint-bench-20260917/` holds only briefs and result JSON. Its numbers are quoted here
  through `research/strategy/04-full-app-deep-capability.md` (Appendix B) and are marked as such.
- `docs/ANALYSIS-ROADMAP.md` still lists call graphs, CFGs, data flow, taint and points-to as
  "Planned", although all exist as preview views. Several statements in `docs/architecture-review/`
  predate the August refactor.

## 2. State of the union after v0.4.4

### 2.1 Runtime

Seconds. Rows marked #132 come from its quiet campaign: n = 15 warm/edit and 25 cold, medians. "Today"
is M2's base arm, n = 5, about 3% slower from host drift.

| OAIZ workload / tier | v0.4.4 (#132) | Today, M2 base | What a run is made of (M1 shares for warm) |
|---|---|---|---|
| full repo / warm | **5.41** (review spot check 5.45) | 5.56 (5.51–5.94) | load ~0.37, pre-source ~0.37; go.syntax 0.95–1.07 (includes the 64 MiB-limit rebuild, §3.2); ts 0.36–0.50; metrics 0.33; rules 2.5 (quiet) to 2.9 (loaded); tail ~0.46 |
| full repo / edit | **7.14** | 7.32 (7.27–7.48) | warm, plus the metrics miss (+1.1) and the Go layer rewrite (+0.6) ([11](11-next-after-metrics-cliff.md) §4D) |
| full repo / cold | **10.54–10.74** (lottery: no timings to dispatch on) | 11.85 (n = 1, profiling build) | parse, layer writes, and the metrics miss path |
| core / warm | 2.99 | — | the rules phase (2.09) is one consumer rule |
| frontend / warm | 1.21 | — | the rules phase (0.66) is one consumer rule |
| code-health / warm | 0.90 | — | providers ~0.5; the rule itself 0.04 |

Every report since #129 has been byte-identical across arms; that held for M2 too (691 diagnostics,
one digest in all samples).

### 2.2 What a warm full-repo run spends its CPU on (M1)

| Bucket | Share | What it is |
|---|---|---|
| **Rules phase** | **64.6%** of all samples | — |
| ↳ consumer code | 64.9% of rules | dominated by the pack's shared Go scanner (§4.2) |
| ↳ `regex` crate | 28.2% of rules | called by consumer rules on masked source text |
| ↳ polint SDK + engine | **4.3%** of rules | span helpers, fact iteration, ID equality |
| ↳ other crates | 2.6% of rules | globset, bstr |
| **Engine (outside rules)** | **35.4%** | — |
| ↳ FNV-1a byte loop | 23.0% of engine | layer payload verification and per-file output digests (L5's target) |
| ↳ `serde_json` decode/encode | ≥ 20.8% of engine | layer restores (L5's target), plus the Go layer re-serialization of §3.2 |
| ↳ Go layer fallback (by caller; overlaps the two rows above) | 25.8% of engine | 5,019 per-file cache reads plus a 96 MB re-serialization, on every warm run (§3.2) |
| ↳ comment-ignore application | 9.7% of engine | `apply_ignores` → `scan_comments`, on the main thread after the rules; about 0.45–0.5 CPU-s per run (§3.2) |
| ↳ source loading | 8.2% of engine | reading 62 MB of inputs |

### 2.3 Capability: what exists, and who uses it

| Layer | What exists at v0.4.4 | Status | Used by OAIZ / the Go+TS monorepo |
|---|---|---|---|
| Raw text | `SourceFiles` (full source text) | stable | 28 of 29 rules / 64 of 77 rules |
| Thin syntax facts | `Imports` (path, alias when written), `Functions` (name, span, test/exported flags, complexity, *names* of calls), `GoTypeDecls` (kind, name, body range), `GoTests`, literals, JSX attributes, branches, TS components/classes, metrics, `ChangedFiles` | stable | Imports 5, Metrics 1, StringLiterals 1, ChangedFiles 6; Functions 0; GoTests 3, **never read** / Imports 20, Functions 7, GoTests 5 |
| Resolution | `ResolvedImports`, `ModuleGraphFacts`, `Symbols`, `References` (Go through the symbols sidecar; TS through `oxc_semantic`) | stable, production-grade (with open identity bugs) | 0 / 0 |
| Deep policy views | `Calls::forbidden_reachable`; `ControlFlow::{missing_guard, guard_outcomes, missing_cleanup}`; `DataFlow::forbidden`; `Events` | **preview**, with fixed query shapes | 0 / 0 |
| Internal deep machinery (about 87k production lines) | MIR (not SSA); CFG with dominators and control dependence; Go RTA over semantic-sidecar rows; TS Andersen points-to and callable flow; TS type-directed tier; abstract domains; SCC summaries; bounded BFS taint paths; evidence; framework entrypoints for net/http, chi, cobra, express, commander, yargs and MCP | experimental at consumer scale; slicing dormant | — |
| Reserved, not provided | `Cfg`, `CallGraph`, `CoverageFacts`, `TestSuiteMetrics` | unsupported | — |

Sources:
- the views: `crates/polint/src/sdk/facts.rs`, `polint facts list`;
- the fact shapes: `crates/polint/src/analysis_api/syntax_facts.rs:13-23,60-83,136-143`;
- the machinery and its maturity: §4.1;
- usage counts: the rule-pack audit (§4.2).

## 3. Direction A: what runtime performance is left

### 3.1 Out of scope by the owner's constraint

| Item | Why it is out |
|---|---|
| L1, the thin SDK and prebuilt engine | Its return is the compile (206.5 s fresh, ~152 s per adopted release). Its one runtime effect, a per-run snapshot, would be a cost, not a saving. |
| L6, ThinLTO off | A build-time saving paid for with slower runs (+4.7% cold, +10.9% warm). |
| L7, crate split | Compile only. |
| Install docs (action 0b) and store-key depth | Install and compile only. |

### 3.2 The runtime levers that remain

The edit and cold figures in the table below come from the "metrics miss", the path the metrics
provider takes whenever its inputs memo misses. That happens on every cold run and every edit; see
[11](11-next-after-metrics-cliff.md) §4A.

| # | Lever | Pays on | Worth (evidence) | Effort / contract | Notes |
|---|---|---|---|---|---|
| A1 | **Go layer over the 64 MiB read limit** (new) | full-repo warm | **−0.30 s measured** (M2): 5.562 → 5.261 s; paired deltas −0.18, −0.39, −0.60, −0.32, −0.31; go.syntax 971 → 612 ms. Edit tier unchanged (7.32 → 7.21 s, within noise) | hours for the limit; days to chunk the layer | `read_file_with_limit` rejects the 96.2 MB blob as `TooLarge` (`analysis_kernel/incremental/layer_cache.rs:32,341-348`; `repo_fs.rs:213-220`). The manifest is evicted, and the provider rebuilds the payload from 5,019 per-file entries and rewrites it (`go/adapter.rs:179-212`). The writer has no limit. Cache counters on every warm run: hits 0, recomputes 1, writes 1. Core's 61.5 MB layer stays under the limit, which is why only the full repo pays. A repository with more Go than OAIZ would also lose its TS or metrics layer. Raising the limit trades the rebuild for a serial 96 MB JSON decode, which is why the gain is 0.30 s and not more. Chunking the layer would also help the edit tier (unmeasured). |
| A2 | Cold-path metrics miss ([11](11-next-after-metrics-cliff.md) §4A) | edit, cold | provider-span ceiling **−1.32 s edit, −1.65 s cold**, −0.30 s warm (measured in #131's no-metrics arm) | days; needs an owner answer on what the metrics layer is for | still the largest single runtime item on the agent edit loop |
| A3 | L5: binary layers and a word-at-a-time hash ([05](05-binary-layer-cache.md)) | every warm and edit run | projection −0.5…−0.85 s full-repo warm, −0.15…−0.27 s core. M1 confirms the premise: FNV plus JSON is at least 44% of warm engine CPU | about a week; a cache-protocol bump | overlaps A1, because once the Go layer is read its decode joins L5's target. Also the codec any richer fact family will need (§4.8) |
| A4 | **Cache comment-ignore directives per file** (new) | every run | about 0.45–0.5 CPU-s per warm run, serial after the rules (M1: 9.7% of engine samples, against 4.7–5.0 CPU-s of engine work per run). Estimate −0.3…−0.45 s wall (unmeasured) | days, in-contract; the key is the file content hash | partly explains the "report tail" (0.46 s) that [11](11-next-after-metrics-cliff.md) left unattributed |
| A5 | The rest of the tail and pre-source gap | full repo | ≤ ~0.3 s once A4 is out (unmeasured) | half a day of spans | diagnostic fingerprinting (3.6% of engine CPU) is one candidate |
| A6 | A resident process or incremental rule execution | edit, warm | an edit could approach "rules phase plus one re-parse", about 3 s instead of 7.1 s (projection, unmeasured) | weeks to months; a new process contract and CLI surface | listed as the last phase of `research/incremental-query-engine`; never designed in this series |
| A7 | L2/L3 rule-result caching ([02](02-rule-result-memoization.md), [03](03-per-file-rule-caching.md)) | no-change reruns, edits | the rules phase (2.5 s) on a no-change rerun (projection) | a rule-purity contract | still blocked: 4 OAIZ files and 1 Go+TS rule call `std::fs`. Note that B2 (§4.8) removes the main reason they do |
| — | SDK query indexes (e.g. `GoTests::related_for_file`) | rules CPU | ~0: polint is 0.7% of rule CPU in its SDK, and OAIZ's `GoTests` parameters are never read | — | dropped |

**The pot, summed** (estimates, with overlaps removed by hand):

| Tier | Today (v0.4.4) | After A1–A5 | Change | Then the floor is |
|---|---|---|---|---|
| full repo, warm | 5.41 | ~3.8–4.5 | −0.9…−1.6 (−17…−30%) | the bounding consumer rule (2.5 s) plus ~1.2 s of load, restore and tail |
| full repo, edit | 7.14 | ~4.8–5.2 | −1.9…−2.3 (−27…−32%) | the same, plus one re-parse and the layer rewrite |
| full repo, cold | 10.5–10.7 | ~8.5 | ~−2 (~−20%) | parse plus rules |
| core / frontend / code-health, warm | 2.99 / 1.21 / 0.90 | ~2.7 / ~1.1 / ~0.65 | −0.1…−0.3 | one consumer rule each |

Only A6, a new process model, goes below those floors without touching what rules do.

### 3.3 The case for A, as strong as I can make it

1. **It pays on the path agents actually run.** The generated skill tells agents to run
   `polint check --format ai-friendly --fail-on none` with no profile (`.claude/skills/polint/SKILL.md:21`),
   which means a full-repo run on every edit. Taking the edit loop from 7.1 s to ~5 s is felt every
   time.
2. **It is cheap, measured, and safe.**
   - A1 is hours. A4 is days. A2 is days once its owner question is answered.
   - All three are in-contract, with byte-identical output as the gate. The perf series has shipped
     four such changes without one regression (#129, #131, #132, and L4 consumer-side).
3. **It carries no product risk.**
   - It adds no public surface. AGENTS.md treats every public name as a liability, and A adds none.
   - B adds SDK views that become semver obligations.
4. **Capability has a record of not being adopted.** About 87k production lines of deep machinery
   shipped between May and September, and neither consumer requests any of it (§2.3). More capability
   risks more of the same.
5. **The remaining A items do not expire.** They can be done at any time, by anyone, with the
   methodology in [09](09-methodology.md).

### 3.4 Why A is not the recommendation

- **Its pot is bounded and its floor is consumer code.** After A1–A5, the polint-side warm cost left on
  the full repo is ~1.2 s of fixed work. M1 shows the rules phase is 93% consumer code and the regex
  engine it calls, with polint at 4.3%. No polint runtime change lowers that. Only better facts, or
  consumer rewrites, can.
- **The largest remaining items need decisions or new contracts, not engineering:**
  - A2 waits on an owner answer;
  - A3 is a protocol bump;
  - A6 is a new process model;
  - A7 is a purity contract.
- **It does not change what polint can say.** The product's stated wedge is "checks that generic tools
  cannot know" (`AGENTS.md`). Today about 85% of OAIZ's rule code (estimate) re-derives syntax from
  text: the layer that generic structural matchers already provide on parse trees (§4.2). polint's
  difference has to come from facts above that layer, and runtime work does not add any.
- **Points 4 and 5 of the case for A argue for how B should be done, not against doing it.** B should
  be pulled by measured consumer walls, not pushed. A's cheap items stay available, and the two that
  cost almost nothing (A1, and #126 under B) should be done now.

## 4. Direction B: capability

### 4.1 What the engine can actually do today

| Family | What it really is (v0.4.4) | Public? | Maturity at consumer scale |
|---|---|---|---|
| Module graph, resolved imports | go.mod/go.work topology; `oxc_resolver` with tsconfig | stable views | **production** |
| Symbols and references | Go: `polint-go-symbols` sidecar (`packages.Load` without dependencies, 120 s fixed timeout). TS: `oxc_semantic` | stable views | **production**, with an open TS ID collision (#86) and a Go Export key collision on test variants |
| Guards and cleanup | same-function dominance and post-dominance over CFG rows. Checked-error verdict and argument binding (#117) | `ControlFlow` (preview) | the most mature deep query, but intraprocedural only, and it pulls in 22 of 24 providers (all-or-nothing selection) |
| Reachability | BFS over refined call edges from labelled roots | `Calls` (preview) | bounded scopes only |
| Taint and data flow | one graph node per MIR place; a bounded breadth-first path enumeration per (source, sink) pair, with a call stack in the state. **Not IFDS tabulation; taint never enters callee bodies.** Sources: `http_request` and `secret_like` only | `DataFlow` (preview) | bounded scopes only; the capability-ladder level-4 probes for taint through helper functions found 0 of 10 (#111); no taint corpus exists |
| Go call graph | semantic sidecar (`packages.Load` with dependencies, then SSA), plus Rust-side RTA (budgets: 256 address-taken functions, 32 rounds). The sidecar's own x/tools RTA is compiled out of release builds; CHA and VTA exist as labels only | through `Calls` | experimental; **production RTA has never been scored against an oracle**, because the x/tools harness scores rows only test builds emit |
| TS call graph | field-sensitive Andersen points-to (global 10k-step budget), heuristic callable flow, and the type-directed tier from the repository's own `tsc` (#121) | through `Calls` | Jelly F1 0.79; the type tier is unscored |
| Abstract interpretation | a call-string worklist over a whole-program ICFG: nilness, truthiness, constants, strings, initializedness. One run-global 10k-iteration budget | internal | the budget trips at 45 files; the output feeds only one flag |
| Framework models | hard-coded recognizers: net/http, chi, cobra, express, commander, yargs, MCP. **gin, echo, fiber, fastify, koa, nest and next are "unrecognized"**. No dependency injection | internal | — |
| Evidence | `evidence_v1` survives to the report, but its `unknowns` and `omitted_regions` are hard-coded empty. **Engine findings never emit SARIF `codeFlows`.** The real evidence store is read only by `polint unknowns` | partly | thin |

Sources: the deep-engine and architecture-review audits, with these spot checks:
- `analysis_neutral/ifds/mod.rs:200-400`;
- `analysis_neutral/data_flow/direct_calls.rs:108-190`;
- `analysis_neutral/domains/solver.rs:84,148`;
- `analysis_neutral/entrypoints/recognizers_go.rs:38-65`;
- `sdk/policy.rs:639-779,1069-1075`;
- `analysis_kernel/provider.rs:1056-1127`;
- `go/lifecycle.rs:150-166`.

**Scale.** M3 is new; the rest is quoted from the full-app research.

| Scope | Cost |
|---|---|
| 2026-09-17, polint 0.3.10, forced `calls` (quoted) | 885 Go files: 152 s, 13.7 GB tree peak. 1,588 files: timeout. The 4,752-file backend: killed at 18.3 GB, with `semantic_mir` at 565 s and +10.4 GB. The 2,381-file TS frontend: every deep capability timed out. |
| #124, W0/W1/W4 | `semantic_mir` 23.1 → 10.8 s at 885 files on a **synthetic** corpus, still superlinear. Never re-measured at consumer scale. |
| **M3, v0.4.4 deep code, one 41-file OAIZ package, `control_flow`, default config** | **Killed at 12 GB after 93 s cold and after 30 s warm.** The Go semantic sidecar held 11.79 GB, polint 0.27 GB. |
| **M3, same package, `include_tests = false`** | **26.0 s and a 7.82 GB peak cold** (sidecar 7.49 GB); **3.8 s and 0.45 GB warm**. **1,112 unknown rows in 40 of 41 files:** 1,072 `dynamic_property` (member calls the call layer could not resolve), 21 `framework_dispatch` (gin), 16 unsupported syntax, 3 function values. |

The full-app plan's own advice to consumers stands: do not request `calls`, `control_flow` or
`dataflow` repo-wide until its gate G6 passes (`research/strategy/04-full-app-deep-capability.md`
§6). G6 means the full backend in under 300 s and 12 GB. W2, W3 (beyond its first commit) and W5–W9
of that plan are not started.

### 4.2 What real rules need, and the walls they hit

The two packs are the only usage evidence there is: 106 rules, about 29k lines. Agents co-author much
of them: 24 of the 31 commits that touch OAIZ's rule sources carry an agent co-author trailer, and 41
of 98 in the Go+TS monorepo. From the read-only audit:

| | OAIZ | Go+TS monorepo |
|---|---|---|
| Rules | 29 (23 check, 6 review) | 77 |
| Lines, non-test / test | 6,353 / 2,554 | 18,730 / 1,666, plus 331 fixtures |
| Shared scanner code | 1,889 lines (21%) | 4,258 lines (21%), including a private Go lexer and parser whose header says `GoTypeDecls` lacks method signatures and value declarations |
| Share that is textual scanning (estimate) | ≈ 85% | ≈ 70% |
| Rules using only `SourceFiles` | 13 | 49 |
| Deep views requested | 0 | 0 |
| `std::fs` | 4 files (tests outside the workspace, non-Go directories, `go.mod`) | 1 rule (CI and supply-chain files) |

**Representative walls** (abridged from the audit's 23):

| Wall | What the rule needs | What it does today | What would answer it exactly |
|---|---|---|---|
| Tests | tests, subtests and table rows for each source function | the workspace excludes `_test.go`, so three rules take a `GoTests<'_>` they ignore and parse `_test.go` from disk with regex | test facts that don't depend on workspace excludes; `t.Run` names and table rows |
| Signatures | method receivers, parameters, struct fields, tags, embedding | regex over raw text; diagnostics anchored on the struct, not the field | typed declaration facts with spans |
| Routes and middleware | the effective middleware on each route | `.Use`/`.Group` tracked by variable name inside one function body. Groups passed into helper registrars lose protection, and real code does this | a route model with gin's copy-at-`Group` semantics across functions |
| Handler → gate | does a handler reach an admin gate? | a naming convention plus a substring fixpoint within one package; the rule's own comments disclaim being a call graph | type-resolved calls plus reachability; dominance for "the gate runs before the write" |
| Error flows | does a publish error reach a return? | the `if … != nil` header and the return line; misses a separate `err :=` statement | typed AST plus intraprocedural def-use |
| Transaction handles | is the appender built from the active transaction handle? | the argument name (`tx` vs `db`) plus brace counting | def-use plus type info |
| Non-code files | SQL, HCL, YAML, Dockerfile, Makefile content | the walker drops non-Go/TS files, so configured globs never reach the rule; `std::fs` and an awk script stand in | a non-code files view (#52) |
| Interface roles | interface implementers, method sets, a dependency's role | signature text and name suffixes | Go type info |

**Counts by capability** (agent judgement; one rule can need several):

| Capability | OAIZ / 29 | Go+TS / 77 |
|---|---|---|
| Typed Go AST queries | 16 | 57 |
| Framework models (routes, ORM, message bus, DI) | 9 | 19 |
| Go type info | 10 | 21 |
| Call graph or reachability | 5 | 11 |
| Data flow or taint | 3 | 9 |
| CFG or dominance | 2 | 5 |
| Non-code files | 3 | 2 |
| Facts already sufficient | 7 | 15 |

**The heaviest rule, where its time goes.** `local/backend-endpoint-authority` is about 1,200 lines.
It joins route registrations, handler functions, command handlers, gate functions, `_test.go` files
and the policy table in config.
- **Its CPU (M1):** 65.8% re-derivation, 23.9% regex, 0% polint.
- **Its code (per the audit):** it parses function declarations twice, discovers routes four times,
  re-masks every function body for each gate on each fixpoint pass, and reads 2.9k test files twice.
- **What native facts would replace:**
  - **A route fact** with a middleware chain: about 250 lines, and the routes-and-middleware wall
    above disappears.
  - **Type-resolved reachability:** about 140 lines, and the one-package limit goes.
  - **Test facts:** about 130 lines.
- **What would remain:** about 300 lines of actual policy.

**Not every wall is missing capability.** `ImportFact.package` already carries an import's alias when
the source writes one (`go/adapter.rs:780-797`), yet OAIZ re-parses import blocks. That is about 9% of
rule CPU, and partly a discoverability problem. Masking and bracket matching (32.1% + 9.3%) are not:
polint exposes no comment, literal, body or argument spans to rules.

Two more points from the record:
- The Go+TS pack's own skill tells it to prefer parser facts, yet its rule comments describe line
  heuristics as the pack's established style. Agents write the scanner when the fact they need is
  absent.
- Issue #9's second round (a rule author's feedback) asks for exactly this middle layer: receivers,
  struct fields and tags, interface method sets, call expressions with arguments, enclosing functions,
  body ranges, and route calls with middleware arguments.

### 4.3 The expressiveness ceiling with today's facts

Rules **can** express:
- import and layer boundaries;
- path and shape conventions;
- literal and JSX policies;
- metrics thresholds;
- test-name pairing, if test files are inside the workspace;
- same-function guard dominance with checked errors and argument binding;
- reachability between labelled roots and named calls;
- bounded source→sink flows from HTTP inputs or secret-like names into named calls.

Rules **cannot** express, without raw-text scanning:
- **Structure:**
  - anything about expressions: call arguments, receivers, composite literals, struct fields and
    tags, `go`/`defer`, constant values;
  - wildcard or regex call names.
- **Types and frameworks:**
  - Go or TS types, method sets, interface satisfaction;
  - routes, middleware chains, ORM query shapes, DI wiring.
- **Flow:**
  - taint that flows through a callee's body;
  - custom taint sources;
  - field-sensitive flows beyond name heuristics;
  - path-sensitive conditions beyond literal `nil` checks;
  - concurrency.
- **Scope and output:**
  - cross-language contracts;
  - non-code files;
  - optional capabilities (a rule with one unavailable capability never runs);
  - cross-file related locations;
  - multi-range fixes.

Exposing raw ASTs or a pattern language is a recorded non-goal (`ARCHITECTURE.md` and
`docs/facts/policy-queries.md:17-18`). Every capability added must be a typed projection.

### 4.4 What the strongest engines do, and which parts fit polint

From the landscape research (sources are URLs in the agent report; selected ones inline):

| Capability | Who has it | Fits polint? | Why |
|---|---|---|---|
| Typed structural facts / patterns | everyone (ast-grep, Semgrep, CodeQL, go/analysis) | **yes, cheapest** | tree-sitter and Oxc trees already exist; it only needs exposing as typed projections |
| Type-aware rules | CodeQL, Semgrep Pro (one file in CE), go/analysis, typescript-eslint, tsgolint | **yes for Go** (the symbols sidecar already type-checks). **TS: risky** | TS 7 ships with no programmatic API until 7.1, and tsgolint reaches internals through shims ([TS 7 announcement](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/)). polint's type tier runs the repository's own `tsc` and refuses TS 7 |
| Framework models as data | CodeQL models-as-data, Semgrep Pro (JS/TS, not Go), Sonar | **yes** | polint has recognizers and a private adaptation TOML. Models-as-data is already planned (deferred August 2026) |
| Call graphs: CHA, RTA, VTA | x/tools, CodeQL | **yes for Go, by reuse** | the x/tools VTA package is about 43 KB of Go, and polint already loads SSA in its sidecar. Go's Andersen analysis was deprecated as slow and unsound ([golang/go#59676](https://github.com/golang/go/issues/59676)) |
| Summary-based interprocedural taint | CodeQL, Semgrep Pro, Infer, Pysa | **yes, per package, after scoping** | polint has SCC summaries and per-call-site TITO. It lacks tabulation and access paths |
| Nil-safety | NilAway, staticcheck `nilness`, Infer | **partly** | `nilness` is intraprocedural on SSA (17.7 KB); NilAway is a separate whole-program constraint solver |
| Explicit unknowns | CodeQL (extraction coverage), Pulse (latent vs manifest), Jelly | **already a strength** | capability diagnostics, `summary.rules[].outcome`, `polint unknowns` |
| Evidence traces (SARIF `codeFlows`) and suggested edits | CodeQL, Semgrep, go/analysis `SuggestedFix` | **yes** | the evidence store exists and is not rendered; fixes are text-only |
| Whole-program points-to, unbounded path sensitivity, symbolic execution, a home-grown TS checker, a Datalog engine | Doop, Infer, Sonar (Java/Python only) | **no: rewrite-class** | recorded exclusions (`docs/architecture-review/10-sota-landscape-and-bar.md` §(f)); x/tools and Biome history argues the same |

Three findings from the research bear directly on the choice:

1. **Rules and models beat depth for detection.** In a study of seven SAST tools, more than 76.9% of
   misses came from insufficient tool support, not analysis limits. Adding Semgrep rules raised its
   detection rate by 181% ([FSE'23](https://sen-chen.github.io/pdf/C38-FSE2023-Comparison%20and%20Evaluation%20on%20Static%20Application%20Security%20Test%20(SAST)%20Tools%20for%20Java.pdf);
   [Brunel](https://bura.brunel.ac.uk/handle/2438/30374)).
2. **Developers act on findings that arrive at diff time and are rarely wrong.** At Meta the same
   analyzer saw a 70% fix rate at diff time and 0% offline. Google requires under 10% "effective false
   positives" ([CACM](https://cacm.acm.org/research/scaling-static-analyses-at-facebook/);
   [SWE book ch. 20](https://abseil.io/resources/swe-book/html/ch20.html)).
3. **Agents use analyzers through hooks and want determinism, explicit unknowns, low noise,
   machine-readable evidence and edits.** Semgrep's argument is that hooks beat optional tool calls
   ([Semgrep, 2025](https://semgrep.dev/blog/2025/cursor-hooks-mcp-server/)).

### 4.5 Does capability change the semantic Go sidecar's equation?

- **Under A, no.** It returns nothing to today's consumers, because they request nothing that needs it.
- **Under B, it matters twice, in two different ways:**
  - **Types come from the cheap sidecar.** The symbols sidecar (`packages.Load` without dependencies,
    about 2.5 s on the OAIZ scan measured in September) can serve the type facts that 10 OAIZ rules
    and 21 Go+TS rules need. Its Export stable-key collision on test variants must be fixed first,
    because it emits a severity-error `polint/internal` whenever validation runs.
  - **Deep queries need the expensive sidecar, and it is not ready.** It loads with dependencies and
    builds SSA, and M3 shows it is the wall even for one 41-file package:
    - 11.8 GB, killed, with the default `include_tests = true`;
    - 7.5 GB with `include_tests = false`.

    Before any OAIZ rule can use a deep query in an agent loop, the sidecar has to be scoped:
    - the `include_tests` default;
    - SSA only for the selected packages and their callees;
    - export data for everything else.

  So B turns the semantic sidecar from irrelevant into a known blocker with a measured size.

### 4.6 The agent wedge: what would make polint the policy engine agents trust

polint already has several things agents need:
- deterministic output;
- explicit capability blocking;
- run summaries that separate "analyzed, nothing found" from "never ran";
- compiled, type-checked rules;
- `review` for diff-gated runs;
- an ai-friendly format.

The gaps that matter, in order:

1. **Exact facts for the rules agents write.** Agents co-author most of OAIZ's rule changes and many of
   the Go+TS monorepo's, and issue #9's author was an agent. They fall back to regex when the SDK lacks
   a fact (§4.2). Every typed fact turns a class of heuristic, false-positive-prone agent rules into
   exact ones. Combined with the compiler as verifier, this is wedge 1 of
   `docs/architecture-review/10`.
2. **Per-finding provenance with honest unknowns.** Render the evidence store's unknowns, omitted
   regions and budgets into diagnostics and SARIF `codeFlows`. This is wedge 2. Its value grows with
   the depth of the facts behind a finding, so it comes after the first fact layers.
3. **Structured edits.** Ranged and multi-range fixes, with an apply path. `Fix` today is a message
   plus a replacement for the diagnostic's own range, and nothing applies it.
4. **Speed in a hook.** The full-repo run is 5.4–7.1 s. Direction A's bounded pot and B's runtime
   dividend (§4.8, B1) both feed this.

### 4.7 Breadth or depth

**Depth, with the evidence:**

- **Demand.** Both consumers are Go plus TS only. No issue asks for another language; #9, #52, #116
  and #49 all ask for depth.
- **Decisions on record.** "More language support is explicitly out of scope, by the founder's
  instruction" (`research/strategy/README.md`). New languages are recorded exclusions
  (`research/strategy/03-build-plan.md` §3).
- **Cost.**
  - The frontend contract is only partly open. The closed `Language` enum is referenced more than
    1,100 times across 153 files, and no third frontend or index adapter has ever been built
    (architecture-review audit).
  - A new language today costs a syntax tier (M per `docs/roadmap/08`) plus edits across that enum.
  - Its value would be the same thin facts §4.2 shows rule authors already work around.
- **The landscape.** "Breadth as moat" belongs to SAST vendors selling compliance coverage
  (`docs/architecture-review/10` §(a)). That is not polint's buyer.

**Trigger to revisit:** a consumer arrives with a third language, or the frontend contract is freed of
the closed enum as a by-product of depth work.

### 4.8 Workstreams

Effort classes:
- **S:** under a week.
- **M:** 2–5 weeks.
- **L:** 1–3 months.
- **XL:** more.

All are estimates.

| # | Workstream | What it unlocks | Effort | Risk | Depends on | Expected value |
|---|---|---|---|---|---|---|
| B0 | **Frontend fidelity:** merge #126 (Go 1.26 `new(expr)`), and track grammar against the toolchain | correct facts for 43 OAIZ files that parse with errors today; 43 fewer `parser/go` errors in every OAIZ report (6% of its 691 diagnostics) | S (PR open) | low | — | every rule, every run |
| B1 | **Typed structure facts, syntax tier, exact.** Go: functions with receiver, parameters, results and body range; call sites with callee selector chain, argument spans, literal values, `go`/`defer` and the enclosing function; struct fields with type text, parsed tags and embedding; syntactic interface method sets; const/var specs; composite-literal keys; test facts for `_test.go` independent of workspace excludes (`t.Run` names, table rows best-effort). TS: import specifiers and bindings, call and member facts | the shared scanners in both packs (6,147 lines); the ≥ 50% of OAIZ rule CPU that is re-derivation (M1); typed-AST rules 16/29 and 57/77 | M (Go 2–4 weeks, TS 1–2 more) | low to medium: more public surface, and layer size grows. The Go layer is already 96 MB of JSON, so A1/A3 become its runtime guard | — | **runtime dividend (estimate, unmeasured):** if the re-derivation and half the regex go, the bounding rule drops from ~2.5 s to well under 1 s. The full-repo rules phase then approaches total rule CPU ÷ 4 ≈ 1.0–1.3 s, so every full-repo tier gets −1.2…−1.5 s and core warm about −1 s. That is a profile tier direction A cannot touch. Step 1 measures it |
| B2 | **Non-code workspace files (#52):** typed text facts for included YAML, SQL, HCL, Dockerfile, Makefile and Markdown, under the same include/exclude rules and cache digests | 3 + 2 rules' walls. Removes most `std::fs` use from rule packs, which reopens L2/L3 rule caching (A7) | S | low | — | correctness, plus a runtime contract later |
| B3 | **Framework models as data:** HTTP routes (verb, path, handler, effective middleware with gin `Group`/`Use` semantics across registrar helpers); ORM query chains (GORM model, predicates, context); message-bus subscriptions; DI wiring. Recognizers declared in `.polint.toml` and shipped as defaults for gin, chi, net/http and express | endpoint-authority-class and route-security rules, tenant scoping, ORM rules (9/29, 19/77); later the Go↔TS API contract, the "clearest open problem" in `docs/architecture-review/10` §(b) | M to L | medium: heuristic precision has to be carried honestly | B1 (B4 for cross-package handlers) | the first capability no generic linter has, applied to the heaviest rule in the measured matrix |
| B4 | **Type facts at rule level:** expose the Go symbols sidecar's `go/types` results (field, parameter and receiver types resolved to packages; method sets; interface satisfaction), and the TS type tier's declaration types where available | type-aware rules (10/29, 21/77). It is also the input that resolves the 1,072 member-call unknowns M3 found | M | medium: setup-dependent (Go ≥ 1.25; Node with TS < 7); fix the Export collision first | B1 | precision for B3 and B5 |
| B5 | **Make deep analysis usable at package scope:** scope the semantic sidecar (`include_tests` default; SSA only where needed); select providers per query family (a guard query should not pay for whole-program RTA and domains); persist deep facts; type-aware member-call resolution; then real IFDS tabulation into callees and taint models as data | authorization guards (#116), publish-error flows, transaction def-use, audit-on-error (CFG 2/5, data flow 3/9, call graph 5/11) | L to XL | high: the full-app plan's W2–W9 are unbuilt; precision on real code is unmeasured beyond M3 | B4 (and B3 for entrypoints and sources) | "the world's most powerful" claims live here, but only once B1–B4 make its answers resolvable |
| B6 | **Provenance and accuracy as product:** render real evidence (per-finding unknowns, omitted regions, budgets, SARIF `codeFlows` with locations); score the production Go RTA; build a taint corpus; run accuracy gates per PR | agent trust; falsifiable capability claims | M | low to medium (additive report schema) | grows with B3–B5 | wedge 2 |
| B7 | **Structured edits:** ranged, multi-range fixes and an apply command | agents close the loop | M | edits must be correct | B1 | agent throughput |
| B8 | **New languages** | a third language | L each (syntax), XL (semantics) | high | an open frontend contract | none for the measured consumers; deferred (§4.7) |

### 4.9 The case for B, and its risks

**For it:**
- It attacks the measured floor, where A cannot. 93% of the rules phase is consumer code, and half of
  that re-derives syntax polint already parsed.
- It is pulled by measured walls, not pushed:
  - 106 rules;
  - issue #9's explicit list;
  - #52;
  - #116;
  - and one consumer that gave up `go/analysis` to adopt polint.
- It compounds. Every fact family serves every future rule, and B1–B4 are prerequisites for the deep
  stack's answers to be anything but "unknown" (M3).
- It is the only direction that moves "most powerful". Depth that real rules cannot run is not power.

**Risks, and how the plan contains them:**

| Risk | How the plan contains it |
|---|---|
| **More unused capability.** The deep stack is the precedent. | Each step is gated on a consumer rule rewritten against the new facts and measured; nothing ships on a fixture alone. |
| **API surface.** | Every view ships as preview with docs under `docs/facts/`, a temp-repo test, digest participation and honest capability names (`AGENTS.md`). |
| **Layer growth slows runs.** | Every step carries direction A's runtime gate (full-repo warm and edit must not regress). L5 (A3) becomes B's codec when the gate trips. |
| **Adoption.** OAIZ and the Go+TS monorepo must rewrite rules. | Step 1 proves the rewrite on scratch copies first, the way L4 was proven. |

## 5. Head to head

| | A: runtime | B: capability, middle layer first |
|---|---|---|
| Remaining return | warm −0.9…−1.6 s, edit −1.9…−2.3 s, cold ~−2 s on the full repo; ≤ −0.3 s on profiles (estimates built on measured ceilings; A1 measured at −0.30 s) | new rule classes; consumer rule code down an estimated 40–70%; a runtime dividend estimated at −1.2…−1.5 s on every full-repo tier and about −1 s on core warm (unmeasured: step 1's job) |
| Evidence of demand | the agent path runs it on every edit; no issue asks for speed | issues #9, #52, #116, #49; 106 rules, 70–85% text scanning; a consumer that lost typed AST access in migration |
| Risk | low; in-contract except A3, A6 and A7 | medium; contained by the gates in §4.9 |
| Time to first value | hours (A1) to days (A2 once answered) | about a week to a measured spike; 3–5 weeks to shipped preview facts |
| Floor | consumer code (polint is 4.3% of rule CPU) | lowers the consumer-code floor itself |
| Compounds | no | yes; a prerequisite for deep precision |
| Moves "world's most powerful" | no | yes |
| Expires if not done now | no | no, but every month of rules written as scanners adds code to migrate later |

The decisive rows are the floor and the compounding. A's pot is real but closes at consumer code. B's
first steps reach below that floor and also build the foundation every deeper capability needs.

## 6. Recommendation: B, starting with the middle layer

**One direction: B.**
- **Build first:** typed structure facts (B1), non-code files (B2) and framework models (B3), pulled and
  measured by the two real rule packs.
- **Then:** type facts (B4), and only then the deep work (B5) behind a measured entry gate.
- **Alongside:** provenance (B6), once there is depth behind findings to show.
- **Direction A carries over as a constraint, not a workstream.** Every step keeps the full-repo
  runtime gate, and A3 (L5) is pulled in as the codec when B's facts trip it.
- **Do now, regardless:** A1 (the 64 MiB layer limit) and B0 (#126).

### Step 1: prove the middle layer pays on OAIZ (about a week, throwaway branch)

**Build:**
- In a throwaway polint branch, extract the Go part of B1:
  - receivers, parameters, results, body ranges;
  - call sites with selector chains, argument spans and literal values, and the enclosing function;
  - struct fields with tags;
  - `_test.go` test facts, including `t.Run` names.
- Expose them through a spike-only view.
- On scratch copies of the OAIZ pack (as L4 was done), rewrite three rules against those facts:
  - `local/backend-endpoint-authority` (2.5 s);
  - `local/backend-gorm-adapter-tests` (1.4 s);
  - `local/backend-json-tags` (0.4 s).

**Measure:**
- non-test lines removed;
- diagnostic diffs against today's report, each one explained as a fixed false positive, a fixed false
  negative, or a regression;
- rule CPU;
- full-repo warm, edit and cold wall time (5 interleaved rounds, the [09](09-methodology.md)
  protocol);
- go.syntax layer size, and restore time.

**Green (all of):**
- ≥ 40% of those rules' non-test lines removed;
- zero unexplained diagnostic diffs;
- full-repo warm and edit each improve by ≥ 0.5 s at the median;
- the layer grows by no more than its restore cost allows: net wall time, not size, is the gate.

**Kill:** the rewrite needs facts the spike cannot provide exactly, or the layer's restore growth eats
the rules saving. In that case record why and fall back to direction A's pot (§3.2), starting with A2.

### Step 2: ship the Go structure facts, non-code files and the grammar fix (2–4 weeks)

**Build:**
- B1 (Go) as **preview** SDK views. Each one needs:
  - a `docs/facts/` page that states precision and limits;
  - a temp-repo test of the kind `AGENTS.md` requires (generated `.polint/rules`, public imports only,
    a diagnostic asserted through `polint check --format json`);
  - stable-key recipes;
  - participation in cache digests.
- Test facts must stop depending on workspace excludes.
- B2 (#52).
- B0 (#126), if not already merged.
- TS import specifiers and call/member facts, as the second half of B1, once the Go half is green.

**Measure:**
- on OAIZ and the Go+TS monorepo, every tier, the [09](09-methodology.md) protocol;
- report bytes identical for rules that don't use the new views;
- layer sizes.

**Gate:**
- no full-repo warm or edit regression above 2%;
- if the layer growth trips that gate, A3 (L5) moves into this step as its codec.

**Offer:** OAIZ a migration PR for its shared scanner.

### Step 3: framework model v1, HTTP routes and their middleware, as data (3–4 weeks)

**Build:**
- A typed routes view: verb, literal path, handler, and the effective middleware chain.
  - It follows gin's `Group`/`Use` copy semantics through helper registrars within a package, using B1
    call facts.
  - Recognizers for gin, chi and net/http are declared as data in `.polint.toml`, with built-in
    defaults.
  - Unrecognized forms become explicit unknown rows, not silence.
- Rewrite `local/backend-endpoint-authority` (scratch copy, then a consumer PR) and the Go+TS
  monorepo's route-security rules on it.

**Green (all of):**
- the route inventory is a superset of the current rule's, with zero unexplained diffs;
- the helper-registrar wall (§4.2) is closed, shown by a fixture;
- endpoint-authority runs in under 1 s;
- the full-repo rules phase is ≤ 1.5 s.

**Then:** B4 (type facts) and the B5 entry gate. The entry gate re-runs M3 and requires, for one OAIZ
package:
- a cold `control_flow` scan ≤ 10 s and ≤ 2 GB;
- unknown member calls under one per file.

Only after that does taint and IFDS work begin.

## 7. Still unmeasured

- B1's runtime dividend and its layer-growth cost. This is step 1's purpose; every B runtime number
  above is an estimate.
- Whether OAIZ and the Go+TS monorepo will adopt new facts. They are separate owners; OAIZ is in-house.
- A4's wall-clock effect (only its CPU is measured), and A1's effect once the Go layer is chunked rather
  than read whole.
- The deep stack on the full OAIZ backend at v0.4.4: last measured at 0.3.10. M3 covers one package.
- How often agents run full-repo checks versus profiles, and edit runs versus no-change runs. Every "who
  pays" claim in §3 rests on the generated skill's instruction, not on telemetry.
- Precision of any deep tier on real code beyond M3's unknown counts. There is no taint corpus, and the
  production Go RTA is unscored.
- The Go+TS monorepo's runtime profile. M1 is OAIZ only.
