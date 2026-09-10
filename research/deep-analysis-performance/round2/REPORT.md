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
single observations. The host is shared: every sample records the load average
and the process table is watched during the run, with any sample that overlapped
foreign compilation discarded and retaken (rejects retained as `rejected-*.json`).

Binary identity: the "before" executable is
`cargo test -p polint --lib --all-features --locked --release --no-run` at
`4906af0e` (SHA-256 `2fc8e58fd6c7f7d8d67f5e6c01f7a7b4baaa67c73efeab8657b35eb80202ccd1`);
the "after" executable is the same command at this branch's code head (SHA-256
`5a1d2605150baf74fd18ef7962ad84b369b207efaa53213ff75769b8194a2d95`), rebuilt
after the fact and byte-identical to the one every "after" sample was taken with.
Release profile, dependencies and compiler flags are unchanged.

### Deep workloads

| Suite / SHA / license | Warm wall s, before → after | Speedup | Warm peak RSS GiB, before → after | RSS Δ | Cold wall s, before → after | Cold peak RSS GiB, before → after |
|---|---:|---:|---:|---:|---:|---:|
| jelly-callgraph-micro / `b799ed4f0d68c670fe398830aaa51dd5c628cf74` / BSD-3-Clause | 150.329 → 147.429 | 1.020× | 2.648 → 2.263 | -14.52% | 154.646 → 160.558 | 2.648 → 2.267 |
| excalidraw-excalidraw-scale / `0dbd2a39319d41fda37b2945dea0dcbd58d6a564` / MIT | 87.356 → 87.543 | 0.998× | 4.753 → 3.956 | -16.76% | 85.568 → 90.964 | 4.760 → 3.969 |
| gohugoio-hugo-scale / `3f35721fb2c75a1f7cc5a7a14400b66e73d4b06e` / Apache-2.0 | 61.590 → 65.493 | 0.940× | 4.871 → 4.027 | -17.32% | 82.310 → 68.228 | 4.893 → 4.047 |

### Syntactic workloads

| Suite / SHA / license | Warm wall s, before → after | Speedup | Warm peak RSS GiB, before → after | RSS Δ | Cold wall s, before → after | Cold peak RSS GiB, before → after |
|---|---:|---:|---:|---:|---:|---:|
| jelly-callgraph-micro / `b799ed4f0d68c670fe398830aaa51dd5c628cf74` / BSD-3-Clause | 0.098 → 0.096 | 1.021× | 0.034 → 0.034 | +0.20% | 0.123 → 0.149 | 0.036 → 0.036 |
| excalidraw-excalidraw-scale / `0dbd2a39319d41fda37b2945dea0dcbd58d6a564` / MIT | 0.167 → 0.199 | 0.839× | 0.049 → 0.050 | +1.24% | 0.243 → 0.300 | 0.052 → 0.053 |
| gohugoio-hugo-scale / `3f35721fb2c75a1f7cc5a7a14400b66e73d4b06e` / Apache-2.0 | 0.678 → 0.660 | 1.028× | 0.096 → 0.096 | -0.27% | 1.160 → 1.220 | 0.108 → 0.108 |

### deep warm repetitions

| Suite | Before seconds | After seconds |
|---|---|---|
| jelly-callgraph-micro | 152.159, 149.988, 150.329 | 147.429, 151.298, 147.19 |
| excalidraw-excalidraw-scale | 88.095, 87.356, 87.009 | 87.543, 88.316, 87.308 |
| gohugoio-hugo-scale | 61.112, 61.59, 62.158 | 64.979, 65.633, 65.493 |

### Interned identity retained

| Suite / mode | Keys | Key text MiB, before → after | Reduction |
|---|---:|---:|---:|
| jelly-callgraph-micro / deep | 1,371,759 | 1789 → 812 | -54.6% |
| jelly-callgraph-micro / syn | 13,495 | 2 → 2 | +0.0% |
| excalidraw-excalidraw-scale / deep | 2,524,665 | 3115 → 1477 | -52.6% |
| excalidraw-excalidraw-scale / syn | 28,479 | 4 → 4 | +0.0% |
| gohugoio-hugo-scale / deep | 2,344,145 | 2107 → 1266 | -39.9% |
| gohugoio-hugo-scale / syn | 90,933 | 12 → 12 | +0.0% |

### Provider and diagnostic digest equality

| Suite / mode / sample | Providers compared | Verdict |
|---|---:|---|
| jelly-callgraph-micro / deep / cold | 23 | identical |
| jelly-callgraph-micro / deep / warm1 | 23 | identical |
| jelly-callgraph-micro / deep / warm2 | 23 | identical |
| jelly-callgraph-micro / deep / warm3 | 23 | identical |
| jelly-callgraph-micro / syn / cold | 6 | identical |
| jelly-callgraph-micro / syn / warm1 | 6 | identical |
| jelly-callgraph-micro / syn / warm2 | 6 | identical |
| jelly-callgraph-micro / syn / warm3 | 6 | identical |
| excalidraw-excalidraw-scale / deep / cold | 23 | identical |
| excalidraw-excalidraw-scale / deep / warm1 | 23 | identical |
| excalidraw-excalidraw-scale / deep / warm2 | 23 | identical |
| excalidraw-excalidraw-scale / deep / warm3 | 23 | identical |
| excalidraw-excalidraw-scale / syn / cold | 6 | identical |
| excalidraw-excalidraw-scale / syn / warm1 | 6 | identical |
| excalidraw-excalidraw-scale / syn / warm2 | 6 | identical |
| excalidraw-excalidraw-scale / syn / warm3 | 6 | identical |
| gohugoio-hugo-scale / deep / cold | 16 | DIFFERS: ['polint.abstract_domains', 'polint.calls', 'polint.cfg', 'polint.direct_summaries', 'polint.entrypoints', 'polint.module_topology', 'polint.semantic_mir', 'polint.symbol_graph', 'polint.type_value_alias'] {'count': 5, 'digest': 'a34bc7740096991d'} vs {'count': 5, 'digest': '37da3ce75ed0d715'} |
| gohugoio-hugo-scale / deep / warm1 | 16 | DIFFERS: ['polint.abstract_domains', 'polint.calls', 'polint.cfg', 'polint.direct_summaries', 'polint.entrypoints', 'polint.module_topology', 'polint.semantic_mir', 'polint.symbol_graph', 'polint.type_value_alias'] {'count': 5, 'digest': 'a34bc7740096991d'} vs {'count': 5, 'digest': '37da3ce75ed0d715'} |
| gohugoio-hugo-scale / deep / warm2 | 16 | DIFFERS: ['polint.abstract_domains', 'polint.calls', 'polint.cfg', 'polint.direct_summaries', 'polint.entrypoints', 'polint.module_topology', 'polint.semantic_mir', 'polint.symbol_graph', 'polint.type_value_alias'] {'count': 5, 'digest': 'a34bc7740096991d'} vs {'count': 5, 'digest': '37da3ce75ed0d715'} |
| gohugoio-hugo-scale / deep / warm3 | 16 | DIFFERS: ['polint.abstract_domains', 'polint.calls', 'polint.cfg', 'polint.direct_summaries', 'polint.entrypoints', 'polint.module_topology', 'polint.semantic_mir', 'polint.symbol_graph', 'polint.type_value_alias'] {'count': 5, 'digest': 'a34bc7740096991d'} vs {'count': 5, 'digest': '37da3ce75ed0d715'} |
| gohugoio-hugo-scale / syn / cold | 6 | identical |
| gohugoio-hugo-scale / syn / warm1 | 6 | identical |
| gohugoio-hugo-scale / syn / warm2 | 6 | identical |
| gohugoio-hugo-scale / syn / warm3 | 6 | identical |


### The Hugo digest difference is Go setup variability, not this branch

The cross-session Hugo comparison above shows nine provider digests and the
diagnostics digest differing. That is **not** caused by this branch. Re-running
the *retained baseline binary* in the same session as the final measurements
reproduces the final binary's digests exactly:

| Run | Diagnostics digest | Provider digests vs final |
|---|---|---|
| baseline binary, session 1 | `a34bc7740096991d` | 9 of 16 differ |
| baseline binary, session 2 | `37da3ce75ed0d715` | all 16 identical |
| final binary, session 2 | `37da3ce75ed0d715` | — |

Fact and interned-key counts are identical across all three (2,032,661 facts /
2,344,145 keys), so the Go setup resolved something differently between sessions
rather than the analysis changing. This independently confirms
`NEXT-LEVER.md`'s instruction to require identical provider and diagnostic
outcomes before accepting any paired Go comparison. **Hugo's cross-session wall
comparison is therefore not a paired comparison and is not claimed as a
speedup.** The same-session paired figures, with all digests equal, are:

| Hugo, same session, digests identical | Baseline | Final | Δ |
|---|---:|---:|---:|
| warm peak RSS GiB | 4.877 | 4.027 | −17.43% |
| cold peak RSS GiB | 4.871 | 4.047 | −16.92% |

Peak RSS is far less load-sensitive than wall time, and the two runs did the same
work; the wall figures from that pairing (baseline warm 70.913 s, cold 62.392 s)
were taken as the host load was climbing past 100 and are not usable.

No `GoSubprocessTimeout` occurred in any Hugo run in this round.

### What moved, and what did not

- **Memory is the result.** Retained key text falls 52.6% on Excalidraw, 54.6% on
  jelly and 39.9% on Hugo, taking warm peak RSS down 16.8%, 14.5% and 17.4%
  respectively, with byte-identical provider and diagnostic digests and an
  identical interned key count on every suite.
- **Warm wall is roughly flat**: jelly 1.020×, Excalidraw 0.998×. The
  representation trades a cheap `Arc` clone in `resolve` for a traversal, and
  streaming comparison and digesting buy most of that back. On jelly
  `polint.refined_calls`, `polint.solver` and `polint.identity` get faster while
  `polint.semantic_graph`'s normalization sorts get slower.
- **Cold wall is noisier and slightly worse on the TS suites** (jelly 154.6 →
  160.6 s, Excalidraw 85.6 → 91.0 s, single observations each). The warm
  repetitions bracket the change much more tightly; treat the cold column as an
  observation, not a claim.
- **Syntactic mode pays a small constant.** Excalidraw's syntactic stage total
  rises 118 → 154 ms (jelly 62 → 65 ms, Hugo 417 → 435 ms). Syntactic requests
  intern a few thousand leaf keys with no substructure to share, so they get none
  of the benefit and pay the new leaf hash: the composable polynomial does two
  `u128` multiplies per byte where the old `RandomState` did SipHash. The fix is
  mechanical and is the first item below.

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
