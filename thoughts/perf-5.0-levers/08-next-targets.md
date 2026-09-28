# In-contract next targets, measured rejections, and one consumer finding (2026-09-28)

These are smaller than the L1–L7 lever menu and, unlike L4's consumer-side finding, are all
**in-contract** (they don't change any build, cache, or rule-authoring contract) — they were found and
sized during the campaign but not taken, either because they're small next to the lever menu or because
they needed an owner decision this campaign could not make on its own.

## Sized but not taken

1. **Fuse the Go extractor's four whole-tree walks.** Imports, string literals, functions, and type
   declarations are each extracted with their own whole-tree walk today. Traversal cost in the base
   cold profile: ~136 samples (**~0.5 CPU-s** on the Go+TS monorepo workload) of `named_child`
   stepping across those four separate walks; fusing them into one visitor would save most of that. A
   cursor-based walk was tried as an alternative approach to the same cost and measured **slower**
   (+7% CPU — see Rejected, below) and was not repeated with a fused-walk shape.
2. **Index `GoTests::related_for_file`.** This SDK view method scans every test fact on every call
   today, so a rule that calls it once per source file is O(files × tests) — **~0.13 CPU-s** on the
   Go+TS monorepo workload today, and growing quadratically as either files or tests grow.
3. **Rule summary rows.** `files_in_scope` re-matches every rule's glob patterns against every file
   *after* the rules have already run (a lock, a hash, and up to two regex matches per rule/file
   pair). Precompiling per-rule matchers, or reusing the scope decisions rules already made while
   running, would save an estimated **~0.03–0.05 s per run**.
4. **Cold layer write.** ~0.4 s serial on the Go+TS monorepo's cold tier: serializing, FNV-hashing, and
   writing 47 MB of JSON layer data, after the per-file cache entries have already been written
   separately. Overlapping this with the next pipeline stage would help, but it requires a concurrent
   thread, which this campaign's no-more-parallelism constraint deliberately kept out of scope; **L5**
   (binary encoding) shrinks the cost instead of pipelining around it.
5. **Canonical Go projection cost.** After the shipped `ad82246a` change, only runs that request a
   trigger capability pay this cost at all, and only on a memo miss. What remains could be made
   cheaper by streaming: writing `u64` fields directly into the digest instead of building rows and
   strings first.
6. **Driver toolchain probes.** `rustc -vV` and `cargo -V`, both invoked through rustup shims, cost
   roughly **50–80 ms of every warm run**. Caching them needs a cache key that is sound against
   changes to the toolchain file and to rustup's own state — risky today because a floating `stable`
   channel means the same toolchain file can resolve to a different actual toolchain over time without
   the file itself changing.
7. **The edit-loop tier itself is unmeasured.** Warm cache with a small number of files changed since
   the last run was **not in this campaign's tier set at all** (the tiers measured were ice-cold, cold,
   and warm-no-change). It restores per-file cache entries one at a time — on the Go+TS monorepo
   workload, roughly 3,000 JSON files — and is very plausibly the single most common real-world
   invocation shape, since most local development is edit-then-check, not check-from-nothing. This is
   flagged here as a measurement gap, not a lever: nobody has a number for it yet.
8. **Not a performance issue — recorded for the owner.** The semantic call-site stable-key fallback
   uses a raw `token.Pos` when no better identity is available, and this makes **~12 of 5,092 rows**
   differ between two otherwise-identical Go semantic analysis runs. This only affects deep
   capabilities (call graph, control flow, data flow, symbols) — neither measured consumer workload in
   this campaign requests deep capabilities, so it did not affect any number in this report, but it is
   a determinism gap worth fixing independently of performance.

## Rejected (measured)

- **Cursor-based Go tree walk.** Cold CPU 9.31 → 10.02 s (**+7%**) on the Go+TS monorepo workload, with
  identical output digests. Cursor stepping over the AST's many anonymous tokens, plus its per-call
  allocation, costs more than indexed `named_child(i)` recursion on the narrow nodes that dominate this
  grammar. See the README's [Two measured rejections](README.md#the-two-measured-rejections) section
  and [`06-lto-build-trade.md`](06-lto-build-trade.md) for the second rejection (ThinLTO off, which
  became lever L6 rather than a quiet default).

## Consumer finding not covered by L4

L4 documents the OAIZ pack's quadratic span-helper fix, which is the single largest consumer-side win
measured. Separately: **one rule in the OAIZ core profile (measured at roughly 3.7–4.2 s per run) bounds
the entire rules phase for that profile**, and therefore bounds the profile's warm floor (see the
floor-analysis table in [README.md](README.md)) — even after L4's span-helper fix is applied, this one
rule remains the slowest single component of the core profile's rules phase. This is reported here, not
as a lever with an expected return, because no alternative implementation was measured for it — only
its bounding effect on the floor was observed. It is consumer code and out of scope for a polint change;
it is recorded so whoever looks at OAIZ core's warm floor next knows where the remaining time goes.

Also worth restating from [`02-rule-result-memoization.md`](02-rule-result-memoization.md): **both**
measured consumer packs read the filesystem directly from at least one rule (`std::fs`), which is
exactly what makes L2's fact-input-only cache key unsound today. This is the same finding, cross-
referenced here because it is a consumer-side observation as much as an engine-lever blocker.
