---
status: complete
date: 2026-11-16
---

# Bound polint rule-host disk usage

## Results

| Measure | Before | After |
|---|---|---|
| Dev rule host, one TS rule pack (stock cargo home) | 554,262,080 B | 107,437,312 B |
| Cold `polint check` including the host build | 62.5 s | 39.5 s |
| Rules target dir after that build | 3.8 GB | 854 MB |
| `target/polint-cli-test-rules` after full run 1 | 61.3 GB (103 hosts) | 4.21 GB (13 hosts) |
| `target/polint-cli-test-rules` after full run 2 | 108.9 GB (190 hosts) | 4.20 GB (11 hosts) |
| Full `cargo test --workspace --all-features --no-fail-fast`, run 1 / run 2 | 441 s / 210 s | 431 s / 183 s |

Both full runs fail the same 26 Go-loading tests before and after: the
machine's go1.27 writes export data version 5, which the pinned Go loader
cannot read. Environment only; CI uses its own Go.

## Changes

1. Dev rule-host profile defaults (`rules_store::dev_profile_settings`):
   line-tables-only for the rule package, `debug = 0` and
   `incremental = false` for `"*"` dependencies, each dropped when the user
   set that key; part of the build fingerprint.
2. Test rule packs are named after test binary, thread and fixture layout, so
   re-runs rebuild in place; the shared test target dir is bounded by (3).
3. `cache::rules_budget`: shared per-host leases (`polint-hosts/<pkg>.lock`),
   LRU eviction of unleased hosts after a check down to
   `POLINT_RULES_TARGET_MAX_MB` (4096), store LRU eviction after publish down
   to `POLINT_CACHE_STORE_MAX_MB` (2048); publishers skip while an eviction
   runs.

## Verification

- Mutation checks: item 1 4/4 killed; item 2 2/2 killed; item 3 8/8 killed
  after removing a redundant current-host check and adding tests.
- fmt, clippy (five feature sets), rustdoc clean; `make js-bench` results
  identical apart from timings.
