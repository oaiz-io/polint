# Measurement and migration scripts

Retained exactly as run. The measurement entry is an internal libtest harness,
not a supported CLI or SDK feature.

## Measuring

Build the release test executable at the revision under test:

```sh
cargo test -p polint --lib --all-features --locked --release --no-run
```

Then, with `POLINT_CACHE_STORE` unset and Go 1.26.5 on `PATH`:

```sh
python3 measure.py --bin <test-executable> \
  --repo research/evaluation-harness/repos/<suite> \
  --label <label> --mode deep --warm-reps 3
```

`measure.py` clears only that checkout's `.polint/cache` for the cold sample,
checks for `polint-store-stamp.json` before and after, times the whole child
process, reads the child's own `getrusage(RUSAGE_SELF)` peak RSS out of the
emitted `CurvePoint`, parses the per-provider `stage done` rows (elapsed, RSS,
fact/key counts, output digest) and the run diagnostics digest, samples the
process table during the run, and **discards and retakes any sample that
overlapped foreign compilation** (rejected samples are kept as
`rejected-*.json`). `matrix.sh` drives deep and syntactic modes over the suites.

`report.py <before-label> <after-label>` emits the markdown tables;
`summarize.py` prints a terse form and compares digests.

## Migration

`migrate_keys.py`, `migrate_sorts.py` and `add_key_imports.py` performed the
mechanical call-site rewrites (composite key construction, sort comparators,
imports); every rewrite was reviewed in the diff and the residual hand fixes are
in the commits. `split_commits.py` split the working tree into the reviewable
slices by hunk content.
