# L3 — Per-file rule execution and caching (2026-09-28)

Situation: **edit loop** (warm cache, a small number of files changed since the last run). Status:
**owner decision — SDK/API addition.**

## What it is

A per-file rule result cache keyed on `(rule, file, file digest, options)`. Instead of a rule always
running over every file in scope, its per-file shape would be cached, and an edit to one file would
only invalidate — and re-run — that rule's result for that one file. Every other file's cached
per-file result would be reused unchanged.

## Measured evidence

- Same underlying per-rule costs as **L2**: rules dominate warm wall time on the workloads that have
  expensive rules (OAIZ core rules ≈4 s of 4.73 s FINAL warm).
- OAIZ's heaviest rules are, in practice, **per-file scans** — they iterate Go files independently and
  accumulate diagnostics per file, which is exactly the shape a per-file cache can exploit without
  changing rule semantics.
- No per-file rule caching implementation exists today to measure; this lever's return has not been
  measured directly and would need its own prototype.

## Why L3 complements L2

L2 (whole-run rule-result memoization) only pays off on a **no-change rerun** — any source edit
invalidates it entirely, because the fact-input key changes globally through `SourceFiles`. L3 targets
the opposite and far more common case: a small edit loop where most files are untouched. By keying
the cache per file instead of per whole run, L3 lets an edit to one file invalidate only that file's
contribution to each per-file rule, while L2-style whole-run caching would have to treat the entire run
as a cache miss. The two levers are not redundant: L2 covers "nothing changed," L3 covers "a little
changed," and a rule pack could in principle benefit from both — a whole-run cache check first, falling
through to per-file reuse when it misses.

## Cost

Requires an SDK/API addition: a stable way to identify a rule's per-file granularity (not every rule is
naturally per-file — some aggregate across files, e.g. module-graph or cross-file dependency rules,
and would not benefit), a file-digest-keyed cache store, and a contract for what counts as "the same
rule, same file, same options" so cache correctness doesn't depend on incidental rule internals. This
has not been scoped to the same depth as L1's or L2's contracts in this campaign; it is named and sized
qualitatively, not designed.
