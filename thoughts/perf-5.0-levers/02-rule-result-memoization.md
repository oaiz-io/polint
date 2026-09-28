# L2 — Rule-result memoization for no-change reruns (2026-09-28)

Situation: **warm, full-repo no-change reruns**. Status: **owner decision — rule purity /
declared-inputs contract.**

## What it is

polint has no rule-level result reuse across runs: every `check` re-executes every rule against the
current facts, even when nothing in the repository has changed since the last run. Rule-result
memoization would cache each rule's diagnostics keyed on its declared fact inputs and options, and
skip re-execution when that key matches the previous run.

## Measured evidence

- OAIZ full-repo **warm 594 s ≈ cold 612 s** — a no-change rerun costs almost the same as a cache-miss
  run, because **rules are 97% of the wall time** on that workload.
- OAIZ core warm: rules are ≈4 s of the 4.73 s FINAL warm median.
- The Go+TS monorepo's own rules cost is much smaller (≈0.7–0.9 s), but on that workload rules are
  still one of the largest remaining warm-tier costs after the shipped campaign changes.
- polint has no existing rule-level reuse mechanism today — this was confirmed by reading the kernel,
  not inferred.

## The measured blocker

Reading both measured consumer packs directly showed that **rules in both call `std::fs` outside their
declared fact inputs today**. In the OAIZ pack, one rule reads a repo-local baseline config file from
disk directly rather than through a declared fact view; the same pattern appears in the second pack's
rule that checks project dependencies against a repo-local policy file. Because of this, a cache key
built only from a rule's declared fact-view inputs would be **unsound**: the rule's actual output can
depend on file-system state the cache key never captures, so a cache hit could silently serve a stale
result.

## Expected returns if the contract existed

From FINAL's stage times, assuming rules declared their extra filesystem inputs (or were proven
view-pure so no declaration is needed):

- OAIZ full-repo warm: 594 s → **~7 s**
- OAIZ core warm: 4.73 s → **~1.0 s**
- Go+TS monorepo warm: 1.44 s → **~0.9 s**

These are projections from the campaign's measured stage breakdown, not measurements of a working
cache — **no memoization implementation exists to measure**.

## The contract that would be needed

- Rules would need a way to **declare extra inputs** beyond their typed fact-view parameters — e.g. an
  explicit list of file paths or a content hash the rule reads outside the SDK's view system — so the
  cache key can account for them.
- Alternatively, rules could be required to be **view-pure**: no direct filesystem or environment
  access outside declared fact views, with all such access mediated through a fact view instead. This
  is the stronger, more valuable guarantee but requires migrating existing rules like the two found
  above.
- **Soundness enforcement**: a debug-only assertion on undeclared reads, analogous to the deferral
  guard already used elsewhere in the kernel (the mechanism that currently checks rule checks don't
  read syntax-fact metadata they never declared needing). In debug builds, any filesystem or
  environment access a rule performs outside its declared inputs would trip the assertion during
  development and CI, without adding a runtime cost to release builds.

## Why this doesn't help the edit loop

Any source-file edit changes the `SourceFiles` fact input that essentially every rule consumes
transitively, so a fact-input-keyed cache invalidates on every edit — L2's cache would never hit during
an interactive edit loop, only on a true no-change rerun (e.g., re-running `check` in CI without new
commits, or running it twice locally without touching any file). The edit-loop case is what **L3**
(per-file rule caching) addresses instead.
