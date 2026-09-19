# Measurement record

Every number here was measured on the host described below, with the command
that produced it named next to it. Anything not run is written as
`unmeasured` with the reason.

## Host

| | |
|---|---|
| CPU | 16 × AMD EPYC-Rome |
| Memory | 30 GB |
| Kernel | Linux 6.8.0-139-generic |
| rustc | 1.95.0 (2026-04-14) |
| Node | v22.22.3 |
| TypeScript | 5.9.3 (the analyzed repository's own install) |
| Go | 1.26.5 |
| Build profile | `--release` for every timing; `--all-features --locked` |

The host is shared. Every timing comparison alternates the two configurations
inside one process, sample by sample, rather than running one arm to completion
and then the other: block sampling on this machine has previously produced
20–135% deltas that interleaving erased.

## Subject repository

[`cs-au-dk/jelly`](https://github.com/cs-au-dk/jelly) at
`b799ed4f0d68c670fe398830aaa51dd5c628cf74`, which is both the call-graph
oracle already used by `research/evaluation-harness` and, in its own `src/`, a
real TypeScript codebase: 84 files, 23,715 lines, two `tsconfig.json` units,
`typescript@5.9.3` in its own `node_modules`. `npm install` was run so the
compiler sees the real import closure rather than typing every dependency as
`any`.

polint discovered 265 TS/JS files and 8,689 call sites in that repository.

## 1. What the tier adds

> Sections 1 and 2 were measured before the call-site identity fix and are kept
> as the record of that run. Section 5 re-measures the same repository with the
> shipped sidecar and says what moved.

Command:

```sh
# An absolute path: `cargo test` runs with the package directory as its
# working directory, not the workspace root.
POLINT_TS_TYPES_MEASURE_REPO=$PWD/research/evaluation-harness/repos/jelly \
POLINT_TS_TYPES_MEASURE_SAMPLES=3 \
cargo test -p polint --lib --all-features --locked --release \
  analysis_kernel::ts_types_tests::measure_type_directed_tier \
  -- --exact --ignored --nocapture
```

The harness alternates `[languages.ts] type_sidecar = true` and `= false` in one
process and reports both arms.

### Call sites resolved to a target

A call site counts as resolved when at least one refined edge for it has
status `Resolved` **and** a non-`None` target function.

| | tier off | tier on | delta |
|---|---|---|---|
| Call sites with a resolved target | 828 | 3,338 | **+2,510 (+303%)** |
| Call sites that lost a target | — | — | **0** |
| Call sites discovered | 8,689 | 8,689 | 0 |

This is a **recall proxy, not an oracle score.** It says the typed tier names a
target for 2,510 call sites that no other tier named, and that no site lost one.
It says nothing about whether those targets are correct; that needs an oracle,
and section 3 explains why the available one cannot answer it.

### Edges by tier

| Tier | tier off | tier on |
|---|---|---|
| `DirectOnly` | 2,396 | 2,396 |
| `DirectPlusFramework` | 23 | 23 |
| `SummaryAssisted` | 2,396 | 2,396 |
| `PointsToAssisted` | 5,995 | 5,995 |
| `TypeDirected` | 0 | 3,449 |
| total | 10,810 | 14,259 |

Every existing tier is byte-for-byte unchanged in count. The typed tier is
purely additive, which is the fallback contract holding: turning it off returns
the previous call graph exactly.

### What the sidecar saw

From the provider counters on the same run:

| Counter | Value |
|---|---|
| `ts_types.projects` | 2 |
| `ts_types.files` | 97 |
| `ts_types.rows_emitted` | 25,607 |
| `ts_types.out_of_scope_rows` | 0 |
| `ts_types.peak_heap_bytes` | 770,742,464 (735 MB) |

`out_of_scope_rows = 0` means the `--scope-files` list did its job: the sidecar
emitted no row the kernel would have dropped.

Row-level detail from the same sidecar run, invoked directly:

| Row kind | Count |
|---|---|
| `callsite` | 7,903 |
| `callee` | 8,207 |
| `receiver` | 7,903 |
| `callable` | 1,503 |
| `any_density` | 85 |

| Call-site status | Count | Share |
|---|---|---|
| `external` (target outside the scan) | 4,205 | 53.2% |
| `resolved` | 3,489 | 44.1% |
| `unresolved` | 132 | 1.7% |
| `union` | 60 | 0.8% |
| `any_receiver` | 17 | **0.2%** |

| Callee dispatch | Count |
|---|---|
| `declared_signature` (describes the call, cannot run) | 4,429 |
| `declared` (exact target) | 3,457 |
| `implementation` (rapid-type candidate) | 307 |
| `union_member` | 14 |

The `any_receiver` share is the Q22 gate's input. On this repository it is 0.2%,
so the density gates never fired — which is the expected shape for a codebase
compiled with `strict: true`, and is exactly the case where a typed tier should
pay off. A repository with a high `any` share would see the gates do their work
instead, and that case is covered by unit tests rather than by this measurement.

## 2. What the tier costs

### Sidecar, standalone

Direct invocation on the same repository, scope list of 106 files:

```sh
node crates/polint/src/ts-sidecar/polint-ts-types/index.js \
  --root . --projects tsconfig.json --typescript ./node_modules/typescript \
  --scope-files /tmp/jelly-scope.txt --ndjson
```

| Stage | Wall time |
|---|---|
| `resolve_typescript` | 205 ms |
| `discover_projects` | 1 ms |
| `create_program` | 1,034 ms |
| `walk_callsites` | 12,264 ms |
| **session total** | **13,504 ms** |

Peak sidecar heap at the last stage boundary: 541 MB for one project, 735 MB
across both.

`walk_callsites` is 91% of the run and is the type checker doing the work the
tier exists for: `getResolvedSignature` and `getTypeAtLocation` per call site,
7,903 of them, about 1.55 ms each.

Two optimizations were measured and **rejected**:

| Change | Result |
|---|---|
| Drop the printed receiver type entirely | 13.6 s → 11.2 s (−18%), at the cost of the evidence string on every typed edge |
| Memoize `typeToString` by type identity | 13.6 s → 13.5 s (within noise); receiver types are mostly distinct objects |

The printed type was kept: it is the evidence a reader needs to see why the tier
answered as it did, and 18% of a cached-after-first-run stage is not worth
removing it for.

### Whole pipeline

Three interleaved samples, `calls` capability, release build:

| Sample | tier on | tier off |
|---|---|---|
| 0 | 148,708 ms | 128,368 ms |
| 1 | 144,429 ms | 134,900 ms |
| 2 | 146,455 ms | 129,127 ms |
| **median** | **146,455 ms** | **129,127 ms** |

Delta: **+17,328 ms, +13.4%**, of which the sidecar itself accounts for
14,920 ms. The remaining ~2.4 s is lowering, validation, the join, and the extra
3,449 edges flowing through the refined-call store.

An earlier run of the same harness, before the span join was indexed by file,
measured median 147,533 ms on / 128,947 ms off (+14.4%). The two runs differ by
less than the spread between samples in either of them, so **indexing the join
did not produce a measurable wall-clock win** on this repository — it removed a
scan of every native call site per sidecar row, which is quadratic in call-site
count and would matter on a larger one, but this measurement does not show it
paying off and does not claim it does. Tier attribution was identical across
both runs.

The pipeline harness constructs the kernel with a disabled cache so both arms
run cold and stay comparable, which leaves the cached path out of those numbers.
It is measured separately below.

### Sidecar, cold and warm

`TsTypesClient::run_cached` driven directly against one cache directory, so the
first pass is the sidecar and the rest are the stored NDJSON being replayed:

```sh
POLINT_TS_TYPES_MEASURE_REPO=$PWD/research/evaluation-harness/repos/jelly \
cargo test -p polint --lib --all-features --locked --release \
  analysis_kernel::ts_types_tests::measure_sidecar_cold_and_warm \
  -- --exact --ignored --nocapture
```

264 discovered TS/JS files across 2 projects:

| Pass | Wall time | Rows |
|---|---|---|
| 0 (cold) | 14,291 ms | 25,607 |
| 1 (warm) | **42 ms** | 25,607 |
| 2 (warm) | **30 ms** | 25,607 |

**340× on the invocation**, with identical row counts. The cache key is
`sidecar_digest + typescript_version + upstream_digest + lifecycle`, and the
lifecycle folds the discovered-file set, so a scan whose scope changed is a
different key and pays the cold cost again.

One caveat this measurement exposes: the `ts_types.*.elapsed_ms` counters a warm
run reports are the **cold** run's timings, because the stored artifact is the
raw NDJSON including its phase rows. `sidecar_self_reported_ms` stayed 14,077 on
both warm passes while the wall clock was 42 ms and 30 ms. The Go sidecar's
cached path has the same property; it is recorded here rather than changed,
because the phase rows describe the sidecar's work and not this run's.

## 3. Accuracy against the Jelly oracle

**Result: no change, and the reason is structural.**

The Jelly micro suite is 149 `.js`, 23 `.ts`, 21 `.mjs` and 1 `.jsx` standalone
snippets under `tests/micro/`, scored against per-file oracle JSON. Jelly's own
`tsconfig.json` includes `src/**/*` and `tests/**/*.test.ts` — it does not
include those snippets. polint walks from each analyzed file to its nearest
`tsconfig.json`, finds that one, and the snippet is not in the program it
describes, so the sidecar emits no rows for it and the typed tier contributes
nothing.

That is the correct behavior — inventing a project for a file the repository
does not compile would be guessing — but it means this oracle cannot score the
tier as it stands. The measured lane numbers are recorded below for the record.

Command, run identically on both trees against the same clone (the baseline
tree reaches it through a symlink), release build, release tier:

```sh
POLINT_GRAPH_BENCH_TIER=release \
cargo test -p polint --lib --all-features --locked --release \
  eval::external::tests::measure_jelly_callgraph_lane \
  -- --exact --ignored --nocapture
```

| | main (82a3c129) | this branch |
|---|---|---|
| cases | 76 | 76 |
| edges expected | 1,479 | 1,479 |
| edges observed | 1,009 | 1,009 |
| unknown count | 899 | 899 |
| recall | 0.6619337390128465 | 0.6619337390128465 |
| precision | 0.9702675916749256 | 0.9702675916749256 |
| F1 | 0.7869774919614148 | 0.7869774919614148 |

Byte-identical, to the last digit. The typed tier neither helps nor harms this
lane, for the structural reason above.

Two notes on these numbers:

- They differ slightly from the committed baseline in
  `research/evaluation-harness/baselines/persisted-graph-accuracy.json`
  (recall 0.6646, precision 0.9742, unknown 529). That drift is **pre-existing**:
  both trees measured identically here, so it comes from the host or the
  toolchain this run used, not from this change. It is not investigated in this
  PR.
- The first branch run of this lane took 142 s against the baseline's 16 s,
  because the sidecar built Jelly's whole program once per case — 76 times — to
  emit nothing. That is what motivated the project-ownership skip described
  below. After it the same lane takes **36 s**, with byte-identical accuracy.
  The residual 20 s over the baseline is 76 Node process spawns at ~0.27 s each,
  which is a property of running 76 separate scans of one repository rather than
  of scanning a repository once.

### The project-ownership skip

Measured directly, with a scope list naming one file that no project lists as an
input:

| | before the skip | after the skip |
|---|---|---|
| Sidecar wall time, one unclaimed file | ~1.5 s (program built, no rows) | **0.27 s** (no program) |
| Whole Jelly lane, 76 cases | 142.06 s | **36.40 s** |
| Jelly lane recall / precision / F1 | 0.6619 / 0.9703 / 0.7870 | identical |
| Rows emitted for an unclaimed file | 0 | 0, plus one diagnostic explaining the skip |

A scope list naming the project's real files is unaffected: byte-identical rows,
same wall time.

## 4. Capability probes

Command, run identically on both trees:

```sh
cargo test -p polint --lib --all-features --locked \
  eval::capability_probes::capability_probe_certification_rollup \
  -- --exact --test-threads=1 --nocapture
```

| Level / language | main (82a3c129) | this branch |
|---|---|---|
| L1 Go | positives 4/4, twins 4/4 | positives 4/4, twins 4/4 |
| L1 TypeScript | positives 4/4, twins 4/4 | positives 4/4, twins 4/4 |
| L2 Go | positives 5/5, twins 5/5 | positives 5/5, twins 5/5 |
| L2 TypeScript | positives 5/5, twins 5/5 | positives 5/5, twins 5/5 |
| L3 Go | positives 6/6, twins 7/7 | positives 6/6, twins 7/7 |
| L3 TypeScript | positives 6/6, twins 7/7 | positives 6/6, twins 7/7 |
| L4 Go (seed) | positives 4/10, twins 15/20 | positives 4/10, twins 15/20 |
| L4 TypeScript (seed) | positives 4/10, twins 18/20 | positives 4/10, twins 18/20 |

Identical, which is both the no-regression result and a statement about the
suite: `tests/capability-probes/repo/` has no `tsconfig.json`, so the typed tier
does not run there at all.

**The probe suite was deliberately not extended.** Adding a `tsconfig.json` to
the probe repository would make the L4 `refined_must` probes answer differently
depending on whether the host running the certification gate happens to have
Node and TypeScript installed, turning a CI gate into a host-dependent one. The
typed tier is covered instead by dedicated end-to-end tests
(`analysis_kernel::ts_types_tests`) that skip when no compiler is present and
fail loudly when `POLINT_REQUIRE_TS_TYPESCRIPT=1`, which is how the Linux CI job
runs them. Making the probe suite able to express a host-dependent capability is
recorded as a follow-up.

## 5. Re-measured after the call-site identity fix

Sections 1 and 2 were measured against a sidecar whose call-site identity was
`<file>:<start-byte>`. Nested calls share a start offset — `a.b().c()` and its
inner `a.b()` both begin at `a`, and so do `f()()` and `f()` — so that identity
collapsed them into one row, and the store dropped the loser as a duplicate
while the survivor kept both calls' callee rows. The identity now carries both
ends. Everything below re-measures the same repository with the fix in.

### Row integrity, sidecar invoked directly

Same invocation on both sidecars, scope list of 264 files, root project only:

| | `<file>:<start>` | `<file>:<start>:<end>` |
|---|---|---|
| Rows emitted | 25,602 | 25,602 |
| `callsite` / `callee` / `receiver` rows | 7,903 / 8,207 / 7,903 | 7,903 / 8,207 / 7,903 |
| **Rows carrying a duplicate stable key** | **801** (359 `callsite`, 359 `receiver`, 83 `callee`) | **0** |
| Call-site status distribution | unchanged | unchanged |
| Callee dispatch distribution | unchanged | unchanged |
| Sidecar wall time | 15,959 ms | 15,724 / 16,120 / 15,782 ms |

The 801 duplicates were dropped by the store on every scan of this repository
and reported as `"801 TS type row(s) dropped (missing or duplicate identity)"`.
They are gone, at no measurable cost.

Determinism, same command three times: the row payload is byte-identical across
runs (`md5` over every non-timing row), as it is on the small fixtures.

### Whole pipeline, re-run

Three interleaved samples, `calls` capability, release build, same host:

| Sample | tier on | tier off |
|---|---|---|
| 0 | 145,535 ms | 132,575 ms |
| 1 | 156,712 ms | 145,759 ms |
| 2 | 157,097 ms | 140,914 ms |
| **median** | **156,712 ms** | **140,914 ms** |

Delta **+15,798 ms, +11.2%**. The host was under different load than during the
run in section 2, so this is not comparable to that run's absolute numbers; both
arms of *this* run are.

| | tier off | tier on |
|---|---|---|
| Call sites with a resolved target (any tier) | 828 | **3,338** |
| Call sites the typed tier alone resolved | 0 | **3,158** |
| Call sites that lost a target | — | **0** |
| `TypeDirected` edges | 0 | **3,450** |
| `DirectOnly` / `Framework` / `Summary` / `PointsTo` | 2,396 / 23 / 2,396 / 5,995 | unchanged |
| `ts_types.dropped_rows` | — | **0** |
| `ts_types.dangling_callees` | — | **0** |
| `ts_types.out_of_scope_rows` | — | 0 |
| `ts_types.rows_emitted` | — | 25,607 |

Two site counts, because they answer different questions. 3,338 is the recall
proxy every tier contributes to; 3,158 is what the typed tier named a runnable
target for on its own, of the 7,904 call sites the sidecar reported. The 2,510
it *gained* are the sites no other tier named.

**The recall proxy did not move**, and that is the expected shape: a collapsed
pair produced two edges off one site, and the fix produces the same two edges
off the two sites they belong to. What changed is which call each edge hangs
off — a precision property this repository has no oracle to score — plus one
extra edge, from a pair whose two calls resolved to the same declaration and
whose callee rows therefore also collided.

### Cached path, re-run

| Pass | Wall time | Rows |
|---|---|---|
| 0 (cold) | 19,208 ms | 25,607 |
| 1 (warm) | **56 ms** | 25,607 |
| 2 (warm) | **33 ms** | 25,607 |

The cache key now also folds the text of each project's `tsconfig.json` and of
everything it extends or references, so an edit to `strict`, `paths` or
`include` invalidates the entry. Before that it did not: those files are not
TypeScript sources, so no source digest covered them and the stored NDJSON was
replayed against changed compiler options.

## 6. What was not measured

| Claim | Status |
|---|---|
| Precision of typed edges against an oracle | **unmeasured** — the Jelly micro oracle cannot reach the tier (section 3), and no other TS call-graph oracle is wired into this repository |
| Warm-cache pipeline cost | **unmeasured** — the harness runs both arms cold by construction |
| Cost on a repository with a high `any` density | **unmeasured** — jelly is `strict: true` and measured 0.2% |
| Behaviour on a monorepo with many `tsconfig.json` units | **partially measured** — jelly has 2 projects and they do not overlap; overlapping projects and solution-style `references` are covered by fixtures and end-to-end tests only |
| macOS and Windows | **unmeasured** — the sidecar is platform-neutral JavaScript and the process runner is the one the Go tier already uses on all three, but no timing or accuracy run was made off Linux |
