# Round 2: cold + warm optimization — canonical identity sharing

Base `4906af0e` (v0.3.7 on `main`, after PR #113's indexed joins and PR #114's
JS/TS callable-flow restoration). Branch `perf/coldwarm-opt`.

`NEXT-LEVER.md` named canonical identity sharing as the big memory lever: 2.5 M
interned keys holding ~3 GiB of recursively expanded identity text for 2.6 MB of
Excalidraw source. This round implements it and measures what it moves.

## What the baseline profile shows

Deep analysis at the base revision, warm medians of three fresh processes
(`.perf/results/baseline/`). Interned key text dominates peak RSS:

| Suite | Facts | Interned keys | Key text MiB | Peak RSS MiB | Key text share |
|---|---:|---:|---:|---:|---:|
| excalidraw-excalidraw-scale | 1,921,319 | 2,524,665 | 3,115 | 4,483 | 69% |
| gohugoio-hugo-scale | 2,032,661 | 2,344,145 | 2,107 | 4,987 | 42% |
| jelly-callgraph-micro | 1,092,676 | 1,371,759 | 1,789 | 2,576 | 69% |

Cold cost is a Go-only phenomenon at this revision. Cold minus warm-median, per
provider:

| Suite | Cold − warm total | Where |
|---|---:|---|
| gohugoio-hugo-scale | +20.7 s | `polint.symbol_graph` +9.6 s, `polint.module_graph` +7.0 s, `polint.go.semantic` +3.5 s |
| jelly-callgraph-micro | +4.3 s | `polint.semantic_graph` +4.0 s |
| excalidraw-excalidraw-scale | −2.0 s | no cold penalty; within run-to-run spread |

The Go providers restore from the warm layer cache almost completely
(`polint.symbol_graph` 9,693 ms cold → 58 ms warm). The JS/TS side does not:
`polint.semantic_graph` costs 117.3 s cold and 113.3 s warm on jelly, so the warm
cache saves ~3% of the dominant stage. **The cold/warm gap on TS corpora is not
where the time is — the dominant stage is recomputed on both.**

## Results

AMD EPYC-Rome, 8 visible cores, six-CPU cgroup quota and a 12 GiB memory limit,
Rust/Cargo 1.95.0, Go 1.26.5. `POLINT_CACHE_STORE` unset; no
`polint-store-stamp.json` before clearing, before analysis or afterwards. Each
cold sample clears only that checkout's `.polint/cache`; each warm sample is a
fresh process preserving it. Wall is whole-child-process elapsed; peak RSS is the
analysis process's own `getrusage(RUSAGE_SELF)` high-water mark and excludes
child processes. Warm figures are medians of three samples; cold samples are
single observations.

Binary identity: the "before" executable is
`cargo test -p polint --lib --all-features --locked --release --no-run` at
`4906af0e` (SHA-256 `2fc8e58fd6c7f7d8d67f5e6c01f7a7b4baaa67c73efeab8657b35eb80202ccd1`);
the "after" executable is the same command at this branch's code head (SHA-256
`5a1d2605150baf74fd18ef7962ad84b369b207efaa53213ff75769b8194a2d95`), rebuilt
after the fact and byte-identical to the one every "after" sample was taken with.
Release profile, dependencies and compiler flags are unchanged.

### Sampling protocol: interleaved, because block sampling lied

This host is shared with other active workloads; its load average ranged from 2
to 349 over the measurement window. Measuring all "before" samples and then all
"after" samples — the obvious protocol, and the one the first pass used — puts
each binary under whatever the host happened to be doing at the time, and the
difference between those two periods is larger than the difference between the
binaries. It produced, for instance, a 19% syntactic regression on Excalidraw and
a 135% one on golang/tools that **do not exist**: re-sampling the same two
binaries alternately gives ±5% and parity respectively.

Everything below therefore alternates the two binaries sample by sample —
baseline cold, final cold, then baseline/final warm pairs — so ambient load lands
on both. Cold samples each clear the cache first, so they remain genuinely cold.
Every sample records its load average and watches the process table, discarding
and retaking any sample that overlapped foreign compilation (rejects retained as
`rejected-*.json`). The earlier block-sampled matrix is kept under
`results/baseline/` and `results/final/` for comparison, but **the paired numbers
below supersede it.**

### Deep workloads (paired)

| Suite / SHA / license | Warm wall s, before → after | Speedup | Warm peak RSS GiB, before → after | RSS Δ | Cold wall s, before → after | Cold peak RSS GiB, before → after |
|---|---:|---:|---:|---:|---:|---:|
| jelly-callgraph-micro / `b799ed4f0d68c670fe398830aaa51dd5c628cf74` / BSD-3-Clause | 140.872 → 143.490 | 0.982× | 2.648 → 2.263 | -14.52% | 143.152 → 141.143 | 2.647 → 2.264 |
| excalidraw-excalidraw-scale / `0dbd2a39319d41fda37b2945dea0dcbd58d6a564` / MIT | 80.082 → 86.330 | 0.928× | 4.753 → 3.955 | -16.78% | 82.241 → 86.093 | 4.758 → 3.961 |
| gohugoio-hugo-scale / `3f35721fb2c75a1f7cc5a7a14400b66e73d4b06e` / Apache-2.0 | 57.047 → 61.986 | 0.920× | 4.871 → 4.030 | -17.25% | 58.697 → 64.097 | 4.875 → 4.049 |

### Deep warm repetitions (paired)

| Suite | Before seconds | After seconds |
|---|---|---|
| jelly-callgraph-micro | 141.717, 139.514, 140.872 | 143.49, 144.193, 142.984 |
| excalidraw-excalidraw-scale | 80.002, 80.082, 80.406 | 86.185, 86.33, 87.418 |
| gohugoio-hugo-scale | 59.429, 56.065, 57.047 | 61.355, 62.724, 61.986 |

### Syntactic workloads (paired)

Wall medians of three interleaved samples per binary, with the summed
per-provider stage time beside them, since these runs are short enough for
process startup to matter:

| Suite | Warm wall s | Warm stage ms | Cold wall s | Cold stage ms |
|---|---:|---:|---:|---:|
| jelly-callgraph-micro | 0.093 → 0.101 | 61 → 68 | 0.120 → 0.125 | 91 → 94 |
| excalidraw-excalidraw-scale | 0.172 → 0.179 | 125 → 134 | 0.227 → 0.232 | 172 → 185 |
| gohugoio-hugo-scale | 0.625 → 0.627 | 397 → 404 | 1.101 → 1.229 | 867 → 991 |
| go-x-tools-rta-callgraph | 1.418 → 1.501 | 871 → 942 | 2.971 → 2.913 | 2,310 → 2,349 |

### Interned identity retained

| Suite / mode | Keys | Key text MiB, before → after | Reduction |
|---|---:|---:|---:|
| jelly-callgraph-micro / deep | 1,371,759 | 1789 → 812 | -54.6% |
| excalidraw-excalidraw-scale / deep | 2,524,665 | 3115 → 1477 | -52.6% |
| gohugoio-hugo-scale / deep | 2,344,145 | 2107 → 1266 | -39.9% |

### Provider and diagnostic digest equality (paired)

| Suite / mode / sample | Providers compared | Verdict |
|---|---:|---|
| jelly-callgraph-micro / deep / cold | 23 | identical |
| jelly-callgraph-micro / deep / warm1 | 23 | identical |
| jelly-callgraph-micro / deep / warm2 | 23 | identical |
| jelly-callgraph-micro / deep / warm3 | 23 | identical |
| excalidraw-excalidraw-scale / deep / cold | 23 | identical |
| excalidraw-excalidraw-scale / deep / warm1 | 23 | identical |
| excalidraw-excalidraw-scale / deep / warm2 | 23 | identical |
| excalidraw-excalidraw-scale / deep / warm3 | 23 | identical |
| gohugoio-hugo-scale / deep / cold | 16 | identical |
| gohugoio-hugo-scale / deep / warm1 | 16 | identical |
| gohugoio-hugo-scale / deep / warm2 | 16 | identical |
| gohugoio-hugo-scale / deep / warm3 | 16 | identical |


### Go setup variability, and why it also broke block sampling

Under the paired protocol Hugo's sixteen provider digests and its diagnostics
digest are identical before and after. Under block sampling they were not: nine
providers and the diagnostics digest differed. That difference is **not** caused
by this branch. Re-running the *retained baseline binary* in the same session as
the "after" samples reproduces the "after" digests exactly:

| Run | Diagnostics digest | Provider digests vs after |
|---|---|---|
| baseline binary, first session | `a34bc7740096991d` | 9 of 16 differ |
| baseline binary, second session | `37da3ce75ed0d715` | all 16 identical |
| final binary, second session | `37da3ce75ed0d715` | — |

Fact and interned-key counts are identical across all three (2,032,661 facts /
2,344,145 keys), so the Go setup resolved something differently between sessions
rather than the analysis changing. This independently confirms
`NEXT-LEVER.md`'s instruction to require identical provider and diagnostic
outcomes before accepting any paired Go comparison — and it is a second reason,
beyond load, that samples of the two binaries must be interleaved.

No `GoSubprocessTimeout` occurred in any Hugo run in this round.

### What this is: memory for time, not both

**This branch does not make analysis faster.** Warm deep wall time is 1.9% worse
on jelly, 7.8% on Excalidraw and 8.7% on Hugo; cold is 1.4% *better* on jelly and
4.7% / 9.2% worse on Excalidraw and Hugo. Syntactic mode is 4–9% worse on wall
and 7–8% on stage time except golang/tools cold, which is flat. What it does buy:

- Retained identity text falls **52.6%** on Excalidraw, **54.6%** on jelly and
  **39.9%** on Hugo.
- Warm peak RSS falls **16.8%**, **14.5%** and **17.3%**; cold peak RSS falls
  **16.8%**, **14.5%** and **16.9%**.
- Every provider output digest and the diagnostics digest is byte-identical on
  every suite, mode and sample, and the interned key count is identical, so the
  analysis is unchanged.

The time is going where the memory came from. Two costs are identified and both
are addressable:

1. `resolve` now expands a composite instead of cloning a stored `Arc<str>`, so
   the leaf hash and the traversal show up wherever a key is touched — which is
   the whole syntactic regression, since syntactic requests intern a few thousand
   leaf keys and have no substructure to share.
2. Streaming comparison replaced `Arc<str>` comparison in the store sorts. That
   removed the `O(n log n)` *materializations* the first prototype introduced
   (jelly `polint.evidence` 14,080 → 9,460 ms), but a chunked DAG walk is still
   slower than `memcmp` over contiguous bytes, and sibling keys share long
   prefixes. On Excalidraw `polint.semantic_graph`'s three normalization sorts
   account for about +2.0 s of the +6.2 s.

Both have concrete fixes, listed first among the next levers. Neither was
attempted here: the shared host offered no window in which a re-measured full
matrix would have meant anything, and shipping an unmeasured change would defeat
the point.

## Retained mechanisms

### 1. Composite stable keys are stored as shared structure

`internal_core/stable_key.rs` previously interned a complete `Arc<str>` per key.
A composite key embeds its parent's whole canonical text as a part, so interning
identical complete strings shared nothing between distinct keys and every level
re-expanded every level below it.

A key is now either complete text (what `intern` of a string produces, where
there is nothing to share) or a segment list: literal runs in one shared arena
plus references to the child keys it embeds. Canonical bytes are streamed from
that structure on demand.

The compatibility boundary is **canonical bytes, not structure**: the lookup
index is keyed on a hash of the canonical byte stream and every accepted
candidate is confirmed by streaming comparison, so a raw string and a composite
that encode the same bytes intern to the same id, in either order. Ids stay
dense and in insertion order, so every `StableKeyId` a run produces is unchanged.

`analysis_api::stable_key_from_key_parts` is the private construction API; a part
is either text or `KeyPart::Key(id)`. `KeyPart::Key` encodes exactly the bytes
`KeyPart::Text(&interner.resolve(id))` would, including the `\` → `/` fold —
a child whose own bytes contain a backslash is written as a folded copy rather
than shared, because folding would change them.

Hashing is composable so that interning costs a key's *own* size rather than its
expansion: a polynomial hash modulo 2^61 − 1 with `H(a·b) = H(a)·base^|b| + H(b)`,
letting a parent fold each child in with one multiply from the child's stored
`(hash, pow)`. `base` is drawn per interner from the process hash seed, so bucket
distribution is not a fixed function of the input, and a collision costs one
byte comparison rather than a wrong identity. This is *not* the FNV digest
recurrence `NEXT-LEVER.md` warned against substituting for randomized map
hashing — the digest bytes themselves are untouched.

### 2. Stable keys are compared and digested without being materialized

Stores sort their rows by stable-key text. Doing that through `resolve` inside a
comparator is `O(n log n)` materializations of keys that are only being compared,
and `sort_by_cached_key` is `O(n)` of them. `StableKeyInterner::compare_canonical`
streams both sides, stops at the first differing byte and allocates nothing,
producing exactly the ordering `resolve(left).cmp(&resolve(right))` does. It
replaces 80 resolve-based comparators and cached-key sorts across 44 files,
expanding to 128 comparison terms because a tuple comparison becomes one term per
element.

Fact-metadata payload digests stream the key's canonical bytes into the
fingerprint instead of expanding them into a string first
(`core::metadata::metadata_payload_digest_for_key`), and
`fact_meta_from_borrowed_parts` hashes the buffer it already built rather than
asking the interner to hand the text back.

## Rejected

- **A `Weak<str>` materialization cache.** Intended to let repeated `resolve`
  calls share one allocation. Measured on jelly deep: peak RSS 2.25 → 2.83 GiB,
  worse than no cache and worse than the baseline's 2.65 GiB. `Arc<str>` keeps its
  bytes in the same allocation as its reference counts, so a surviving weak
  reference pins the text and the cache retains every key it ever handed out.
  Removed; the reason is recorded at `resolve`.
- **Structural (tuple) identity for the lookup index.** Cheaper to hash, but a
  raw string and a composite encoding the same bytes would then get different
  ids, and `go/rta/inputs.rs` interns strings that other code produces from the
  same canonical encoder. Canonical-byte identity is the compatibility boundary.
- **The exact composable FNV summary from `NEXT-LEVER.md` as the interner's map
  hash.** `NEXT-LEVER.md` explicitly rules this out (predictable digest in place
  of randomized map hashing); the composable hash used here is per-process
  randomized instead.
- **Parallelizing the TS file analyses.** They are the dominant stage and are
  embarrassingly parallel, but they intern, and dense ids are assigned in
  insertion order — parallel interning would make every `StableKeyId` in a run
  depend on thread scheduling. Not attempted.
- **Restructuring the `polint.semantic_graph` output digest.** Its parts are
  globally sorted complete strings; streaming them in row order would change the
  hashed sequence unless the sort is reproduced exactly, and the measured cost
  (334 ms of a 110 s stage) does not justify the digest-byte risk.

## Next levers, with measured evidence

0. **Drop the per-byte power accumulation in the canonical hash.**
   `CanonicalHasher::bytes` accumulates `base^len` alongside the hash, so every
   byte costs two `u128` multiplies. `base^len` is only needed when a key is
   embedded as a child and can be computed by fast exponentiation in `O(log len)`
   at that point instead, halving the leaf hash loop and letting `KeyNode` drop
   its `pow` field (8 bytes × 2.5 M keys ≈ 20 MiB on Excalidraw). This is the
   identified cause of the syntactic-mode regression above; it was left out of
   this branch because the shared host did not offer a quiet window to re-measure
   the full matrix after changing it, and an unmeasured change is not shippable
   here.

0b. **Skip a shared child in `compare_canonical` in `O(1)`.** Sibling keys embed
   the same parent, so two keys being compared usually reach a point where both
   cursors are positioned at the identical `Segment::Key(child)`. Recognising that
   and stepping over the child without reading its bytes would turn the common
   comparison from `O(shared prefix)` into `O(number of segments)`. This is the
   identified cost behind Excalidraw's `polint.semantic_graph` normalization sorts
   (about +2.0 s of that suite's +6.2 s) and behind the deep warm regression
   generally. `NEXT-LEVER.md` anticipated it as "chunked comparison with reusable
   lexical ranks"; the ranks are the weaker form, since they still need one
   streaming comparison per pair.

1. **`ts_direct_bindings` is the whole ballgame on JS/TS.** Private substep
   timing shows `polint.semantic_graph` spends >98% of its wall in
   `collect_ts_direct_binding_collection` (144.9 s of 147.4 s in an attribution
   run; `build` 992 ms, `normalize` 143 ms, `output_digest` 334 ms, `store`
   935 ms). On jelly that single call is ~73% of the whole run. It is sequential,
   it re-runs on warm, and it is where PR #114's restored callable-flow model
   lives. Splitting it into `parse` / `analyze` / `callable_flows` substeps is
   instrumented in this branch; the next round should attribute inside it before
   touching it, and must not regress the restored accuracy.
2. **Facts still retain expanded key text.** `EvidenceNodeFact` and
   `EvidenceEdgeFact` hold `source_fact_stable_keys: Vec<Arc<str>>` and
   `summary_stable_key: Option<Arc<str>>`; `DataFlowEdgeFact` holds
   `input_stable_keys: Vec<Arc<str>>`. Those used to be free `Arc` clones of
   interner text; with shared structure they are owned expansions, worth ~450 MiB
   on jelly (the `polint.evidence` stage's live RSS rises 1,707 → 2,152 MiB).
   Storing `StableKeyId` and resolving at the boundary that needs bytes would
   recover it, but `refined_calls` mixes derived strings (`format!("summary={}",
   …)`) into the same vectors, so it needs a small key-or-text representation
   rather than a straight id swap.
3. **`type_value_alias` digest key materialization.** `DigestKeys::new` in
   `analysis_neutral/types/provider.rs` builds a `BTreeMap<Id, String>` of the
   complete key text of every value, allocation and access path, then its
   accessors `.cloned()` each one again — two full copies of every key per digest.
   Measured 5.1 s of the `polint.type_value_alias` stage on jelly. Storing
   `Arc<str>` in the map removes one copy for free; streaming the digest removes
   both.
4. **Go cold setup.** `polint.symbol_graph` 9.7 s, `polint.module_graph` 7.0 s and
   `polint.go.semantic` 3.5 s on Hugo cold, all ~0 warm. No sidecar timeout
   reproduced in this round: the 120 s `GoSubprocessTimeout` from September 7 did
   not recur in any Hugo run here (`GO_SUBPROCESS_TIMEOUT` in
   `go/process_runner.rs` is unchanged), and both cold and warm Hugo runs
   produced identical diagnostics digests. The duplicated `packages.Load` between
   the symbol and semantic sidecars remains a source-evidence hypothesis, not a
   measured gain; their pinned x/tools versions still differ (0.42.0 vs 0.45.0)
   and `NeedDeps` must stay.


## Validation

- `cargo clippy -p polint --all-targets --all-features --locked -- -D warnings`:
  passed. The same command under each CI language-feature configuration
  (`--no-default-features`, `--features lang-go`, `--features lang-typescript`):
  passed.
- `cargo fmt --all -- --check` and `cargo check -p polint --all-targets
  --all-features --locked`: passed. Each of the two mechanism commits was checked
  out into a scratch worktree and `cargo check`ed on its own, so the slices build
  independently and not just as a set.
- `cargo test -p polint --lib --all-features --locked`: **2,505 passed, 14
  ignored, 0 failed** of the 2,519 tests in the suite. The single test not
  reached, `eval::bench::sweep::tests::sweep_entry_point_skips_absent_checkouts_
  without_failing`, re-analyses the whole scale corpora through the debug build;
  it was still running after an hour on a host whose load average had spent much
  of that time above 100, and was stopped rather than left to hold the session.
  It passed in the earlier full run of the same mechanism.
- An earlier full run of the same code, taken while the shared host was at load
  20–35 with `rustc` processes being OOM-killed, failed two fixture determinism
  tests (`eval_cfg_core_observes_required_families_and_determinism` and the
  direct-calls equivalent). Both pass when run individually against that same
  binary, and both pass in the quiet full run above. Their failure text names a
  pre-existing duplicate-identity condition — "Fact metadata stable key conflict
  detected for DomainObservation stable key" — produced by
  `analysis_neutral/domains/store.rs::observation`, whose key is
  `(source, slot, location, place)` with `place` falling back to the literal
  `"none"` when a place is missing from `place_stable_keys`. That construction
  site is on the unmigrated text path and is untouched by this branch.
- **Accuracy is unchanged, proven by running the fixed gate at both revisions.**
  `POLINT_REQUIRE_BENCH_CORPUS=1 POLINT_WRITE_GRAPH_BENCH=1
  POLINT_GRAPH_BENCH_TIER=release cargo test -p polint --lib --all-features
  --locked eval::external::tests::external_graph_baseline_reports_can_be_generated`
  passes on this branch. Because `POLINT_WRITE_GRAPH_BENCH` regenerates
  `research/evaluation-harness/baselines/persisted-graph-accuracy.json` in place,
  the same command was also run in a scratch worktree at `4906af0e`, and the two
  regenerate byte-identical accuracy columns:

  | Suite | Recall | Precision | Edges observed | Unknowns |
  |---|---:|---:|---:|---:|
  | jelly-callgraph-micro, `4906af0e` | 0.6666666666666666 | 0.970472440944882 | 1,016 | 913 |
  | jelly-callgraph-micro, this branch | 0.6666666666666666 | 0.970472440944882 | 1,016 | 913 |
  | go-x-tools-rta-callgraph, `4906af0e` | 1.0 | 0.04993252361673414 | 10,016 | 0 |
  | go-x-tools-rta-callgraph, this branch | 1.0 | 0.04993252361673414 | 10,016 | 0 |

  Jelly F1 is 0.790430 on both, against the committed baseline's 0.790227 and a
  0.005 tolerance. Only the cost columns differ, and in this branch's favour
  (jelly `peak_rss_bytes` 1,422,745,600 → 1,360,035,840, Go 113,487,872 →
  109,395,968). The regenerated file was reverted in both worktrees: **no
  baseline, tolerance or gate is modified by this branch.** The committed
  baseline's own accuracy columns already differ slightly from what either
  revision regenerates, and its `reference` string still describes cost columns
  the current writer no longer emits, so that drift predates this work.
- Determinism: the interned key count is identical before and after on every
  suite, and every provider output digest and the diagnostics digest is
  byte-identical on all three suites in the paired protocol — 23 providers on
  jelly and Excalidraw, 16 on Hugo, across the cold sample and all three warm
  samples.
