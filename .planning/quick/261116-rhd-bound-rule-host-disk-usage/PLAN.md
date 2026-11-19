---
slug: 261116-rhd-bound-rule-host-disk-usage
type: quick
---

# Bound rule-host disk usage

One worktree reached a 270 GB `target/`, 247 GB of it in
`target/polint-cli-test-rules/debug` (about 2,605 hosts and rlibs of ~500 MB).

## Measured cause

Stock dev profile builds the rule host with `debug = 2` across polint and every
dependency, plus incremental state for the `polint` path dependency. One
generated TS rule pack, cold: host 554 MB, libpolint rlib 1.0 GB,
incremental 1.6 GB, target 3.8 GB, 62.5 s. Tests give every rule pack a package
name hashed from its random tempdir path, so each run adds a new host per test.

## Tasks

1. Dev rule-host builds default to `debug = "line-tables-only"` for the rule
   package and `debug = 0`, `incremental = false` for dependencies, unless the
   user already configures those keys (env, cargo config, workspace manifest).
   Measured: host 107 MB, target 854 MB, cold 39.5 s.
2. Tests name rule packs from a stable test identity instead of the tempdir
   path, and run with a bounded rules target dir (task 3 budget).
3. Automatic LRU budget for the machine-global rule-host store and for the
   rules target dir, env override, eviction only under the target lock after the
   host finished; the target lock now also covers the `cargo run` path so a
   running host is never evicted or replaced.

Each change gets a test that fails without it (mutation checked).
