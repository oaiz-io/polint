# 261115-p8s Findings: JS/TS analysis speed

Baseline: `e0d4199f` (v0.5.1). Host: AX102 (16 cores / 32 threads), release build.

The workload is the JS bench fact dump. It runs
`eval::js_bench::tests::dump_js_bench_facts` once per package and plan:

- **refined:** `syntax,calls,control_flow,dataflow,call_graph,symbols,references`
- **base:** `syntax,call_graph,symbols,references`

## Tools

- **Provider stage timings:** `RUST_LOG=polint::kernel::stage=info` for each provider (and `debug` for each
  provider step). Each line carries wall time, RSS, peak RSS, fact count, interned key count, and the provider
  output digest. This was added in commit `347e7c32` and is test-only (the eval harness).
- **CPU profile:** `perf record -F 499 -g` on a frame-pointer release build (`-C force-frame-pointers=yes`). The
  stacks were folded and aggregated by inclusive and exclusive time per frame.
- **Heap profile:** heaptrack on base zod.
- **Equivalence check, after every change:**
  - All 82 dumps, i.e. 41 cases (construct cases plus the seven packages) × the 2 plans, must be byte-identical
    to the `e0d4199f` dumps.
  - The refined zod stage digests must be identical.

## Baseline profile: refined zod (93.2 s wall, 3.6 GB peak RSS)

| Provider | Wall (ms) | Dominant cost |
| --- | ---: | --- |
| `polint.refined_calls` | 38 980 | Quadratic. Each unresolved call scanned every call site, semantic node, constraint and solver edge. |
| `polint.type_value_alias` | 9 366 | Output digest is 4.4 s: one heap `String` per fact row, sorted, on one thread. The other 1.4 s is TS facts. |
| `polint.semantic_graph` | 7 562 | `ts_direct_bindings` 4.2 s, `build_normalize` 1.8 s, digest 0.6 s. |
| `polint.evidence` | 6 935 | Metadata 3.4 s (sequential, cloning every row), digest 1.9 s, normalize sorts. |
| `polint.data_flow` | 5 779 | Metadata 2.0 s (sequential), digest 1.8 s. Narrowing scanned every place for each operation. |
| `polint.symbol_graph` | 3 983 | `derive_requested_symbols_with_cache_stats`. Shared with base. |
| `polint.semantic_mir` | 3 873 | `lower_ts`. Shared with base. |
| completeness view (post-run) | ~6 000 | Unknown taxonomy. Each data-flow and refined-call unknown looked up its node or call site by linear scan. |
| `calls`, `cfg`, solver, domains, summaries, others | ~7 500 | Spread out; no single hotspot. |

Memory:

- **Interned stable keys:**
  - base: 1.32 M keys, 297 MB
  - refined: 4.9 M keys, 751 MB
- **Dump serialization:** about 550 MB of the measured peak. This is part of the bench harness, not of
  `polint check`.
- **Everything else:** the rest of the peak is fact rows that live in `AnalysisDb` for the whole run, accumulated
  across providers. No single transient buffer dominates.

## Baseline profile: base zod (11.0 s, 1.3 GB)

| Provider | Wall (ms) |
| --- | ---: |
| `polint.symbol_graph` | 3 895 |
| `polint.semantic_mir` | 3 674 |
| `polint.calls` | 1 609 |
| `polint.cfg` | 896 |

Base zod is dominated by two providers that refined zod also runs. Neither has an algorithmic hotspot; the time is
spread over per-node work.

## Ranked causes and their outcomes

| Rank | Cause | Refined zod won | Commit |
| ---: | --- | ---: | --- |
| 1 | Quadratic call-site, place and solver-edge scans in JS/TS call refinement | ~37.6 s | `abe8a601` |
| 2 | Linear node and call-site lookup per unknown when collecting unknowns | ~6 s | `6837a20b` |
| 3 | Per-row heap strings and single-threaded sort in four provider output digests | 8.7 → 4.1 s | `222c9e96` |
| 4 | Sequential, row-cloning evidence and data-flow fact metadata | 5.4 → 2.4 s | `9680264f` |
| 5 | Narrowing scanned every place for each operation | ~0.5 s | `c43a0d70` |
| 6 | One interner lock and `Arc` clone per sort comparison and per key in reference validation | within noise | `f5559d1b` |

## Not addressed, with reasons

- **`ts_direct_bindings` (4.2 s), in `collect_ts_file_analyses`:**
  - Of this, 2.4 s is CPU in `collect_callable_flows` and 1.2 s is in `analyze_parsed_ts_file`; parsing is
    negligible.
  - Both read the oxc `Program`, which is `!Sync`, so per-file work cannot be shared across rayon workers without
    re-parsing per thread or unsafe code.
  - `analyze_parsed_ts_file` also interns through the frontend interner. Interning order affects the digest.
- **`semantic_mir` and `symbol_graph` (about 3.7 s and 3.9 s, in both plans):**
  - The work is spread over per-node work, with no quadratic term.
  - Speeding them up needs structural parallelization: deterministic interner overlays per file, then absorb and
    remap.
  - That is a larger change than this task's budget allows.
- **Memory:**
  - Refined peak RSS is set by long-lived fact rows and 4.9 M interned keys, not by transient buffers.
  - Reducing it means changing what facts are retained (for example, dropping intermediate rows after downstream
    providers consume them). That changes the database contract, not just its speed.
  - The digest change avoided a regression by keeping buffers below glibc's dynamic mmap threshold. Before that
    fix, an early version raised peak RSS.
- **Remaining sequential metadata refreshes:**
  - These are summaries, extensions, adaptation, entrypoints, module, scopes, imports and exports, symbol graph,
    metrics, and reachability.
  - Each one is small on zod.
  - They can only be moved to parallel `record_family_metadata` after checking that their builders never intern
    new keys. Builders that use `fact_meta_from_borrowed_parts` do intern.
