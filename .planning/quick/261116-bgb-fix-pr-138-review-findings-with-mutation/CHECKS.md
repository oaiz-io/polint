# CI check evidence

The four changed Rust files match `SOURCE-SHA256.txt`. These final checks ran before the interrupted session ended; their green results remain applicable because the source checksums are unchanged. Full workspace tests and the JS benchmark are rerun during this resume and recorded in SUMMARY.md.

Environment: Rust 1.95, four build jobs, debug info and sccache disabled; Go 1.25.11 and TypeScript 5 are required, with temporary writable dependency caches. Commands and environment are preserved in `/tmp/pr138-env.sh`, `/tmp/pr138-ci.sh`, `/tmp/pr138-final-checks.sh`, and `/tmp/pr138-gates-rerun.sh`.

## cargo fmt --all -- --check

PASS; source log: `/tmp/pr138-checks/fmt-frozen.log`.

```text
```

## cargo check --workspace --all-targets --all-features --locked (Rust 1.95)

PASS; source log: `/tmp/pr138-checks/msrv-frozen.log`.

```text
    Checking polint-example-basic-rule v0.5.1 (/Users/emilwareus/Development/polint-review/pr138-speed/examples/basic/.polint/rules)
    Checking polint-example-go-import-boundaries-rule v0.5.1 (/Users/emilwareus/Development/polint-review/pr138-speed/examples/go-import-boundaries/.polint/rules)
    Checking polint-example-config-denied-literal-rule v0.5.1 (/Users/emilwareus/Development/polint-review/pr138-speed/examples/config-denied-literal/.polint/rules)
    Finished `dev` profile [unoptimized] target(s) in 28.33s
```

## cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

PASS; source log: `/tmp/pr138-checks/clippy-all-frozen.log`.

```text
    Checking polint-example-go-sensitive-writes-rule v0.5.1 (/Users/emilwareus/Development/polint-review/pr138-speed/examples/go-sensitive-writes/.polint/rules)
    Checking polint-example-ts-design-tokens-rule v0.5.1 (/Users/emilwareus/Development/polint-review/pr138-speed/examples/ts-design-tokens/.polint/rules)
    Checking polint-js-analysis-bench-rules v0.5.1 (/Users/emilwareus/Development/polint-review/pr138-speed/tests/js-analysis-bench/rule-pack)
    Finished `dev` profile [unoptimized] target(s) in 33.94s
```

## cargo clippy -p polint --all-targets --locked -- -D warnings

PASS; source log: `/tmp/pr138-checks/clippy-default-frozen.log`.

```text
    Checking polint v0.5.1 (/Users/emilwareus/Development/polint-review/pr138-speed/crates/polint)
    Finished `dev` profile [unoptimized] target(s) in 34.11s
```

## cargo clippy -p polint --all-targets --no-default-features --locked -- -D warnings

PASS; source log: `/tmp/pr138-checks/clippy-none-frozen.log`.

```text
    Checking polint v0.5.1 (/Users/emilwareus/Development/polint-review/pr138-speed/crates/polint)
    Finished `dev` profile [unoptimized] target(s) in 24.87s
```

## cargo test -p polint --lib --no-default-features --locked

PASS; source log: `/tmp/pr138-checks/lib-none-frozen.log`.

```text
test sdk::scope::tests::every_shipped_pattern_matches_every_corpus_path_as_the_reference_does ... ok

test result: ok. 2134 passed; 0 failed; 15 ignored; 0 measured; 0 filtered out; finished in 11.02s

```

## cargo test -p polint --test cli --no-default-features --locked disabled_

PASS; source log: `/tmp/pr138-checks/cli-disabled-none.log`.

```text
test disabled_typescript_feature_reports_capability_diagnostic ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 191 filtered out; finished in 1.47s

```

## cargo test -p polint --test cli --no-default-features --locked new_rule_creates_skeleton

PASS; source log: `/tmp/pr138-checks/cli-new-rule-none.log`.

```text
test new_rule_creates_skeleton ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 192 filtered out; finished in 1.60s

```

## cargo clippy -p polint --all-targets --no-default-features --features lang-go --locked -- -D warnings

PASS; source log: `/tmp/pr138-checks/clippy-go-frozen.log`.

```text
    Checking polint v0.5.1 (/Users/emilwareus/Development/polint-review/pr138-speed/crates/polint)
    Finished `dev` profile [unoptimized] target(s) in 29.06s
```

## cargo test -p polint --lib --no-default-features --features lang-go --locked

PASS; source log: `/tmp/pr138-checks/lib-go-frozen.log`.

```text
test sdk::scope::tests::every_shipped_pattern_matches_every_corpus_path_as_the_reference_does ... ok

test result: ok. 2228 passed; 0 failed; 15 ignored; 0 measured; 0 filtered out; finished in 10.71s

```

## cargo test -p polint --test cli --no-default-features --features lang-go --locked disabled_

PASS; source log: `/tmp/pr138-checks/cli-disabled-go.log`.

```text
test disabled_typescript_feature_reports_capability_diagnostic ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 192 filtered out; finished in 1.62s

```

## cargo test -p polint --test cli --no-default-features --features lang-go --locked new_rule_creates_skeleton

PASS; source log: `/tmp/pr138-checks/cli-new-rule-go.log`.

```text
test new_rule_creates_skeleton ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 192 filtered out; finished in 1.61s

```

## cargo clippy -p polint --all-targets --no-default-features --features lang-typescript --locked -- -D warnings

PASS; source log: `/tmp/pr138-checks/clippy-typescript-frozen.log`.

```text
    Checking polint v0.5.1 (/Users/emilwareus/Development/polint-review/pr138-speed/crates/polint)
    Finished `dev` profile [unoptimized] target(s) in 29.33s
```

## cargo test -p polint --lib --no-default-features --features lang-typescript --locked

PASS; source log: `/tmp/pr138-checks/lib-typescript-frozen.log`.

```text
test sdk::scope::tests::every_shipped_pattern_matches_every_corpus_path_as_the_reference_does ... ok

test result: ok. 2330 passed; 0 failed; 17 ignored; 0 measured; 0 filtered out; finished in 10.01s

```

## cargo test -p polint --test cli --no-default-features --features lang-typescript --locked disabled_

PASS; source log: `/tmp/pr138-checks/cli-disabled-typescript.log`.

```text
test disabled_go_feature_reports_capability_diagnostic ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 192 filtered out; finished in 2.01s

```

## cargo test -p polint --test cli --no-default-features --features lang-typescript --locked new_rule_creates_skeleton

PASS; source log: `/tmp/pr138-checks/cli-new-rule-typescript.log`.

```text
test new_rule_creates_skeleton ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 192 filtered out; finished in 2.09s

```

## cargo test -p polint --lib --all-features --locked

PASS; source log: `/tmp/pr138-checks/lib-all-frozen.log`.

```text
test ts::callable_flow::tests::many_self_calls_keep_the_walk_count_bounded ... ok

test result: ok. 2854 passed; 0 failed; 21 ignored; 0 measured; 0 filtered out; finished in 61.77s

```

## cargo test -p polint --test internal_architecture --all-features --locked

PASS; source log: `/tmp/pr138-checks/architecture-frozen.log`.

```text
test dense_id_sweep_matches_baseline ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.51s

```

## RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked

PASS; source log: `/tmp/pr138-checks/doc-frozen.log`.

```text
 Documenting polint-example-basic-rule v0.5.1 (/Users/emilwareus/Development/polint-review/pr138-speed/examples/basic/.polint/rules)
 Documenting polint-example-ts-design-tokens-rule v0.5.1 (/Users/emilwareus/Development/polint-review/pr138-speed/examples/ts-design-tokens/.polint/rules)
    Finished `dev` profile [unoptimized] target(s) in 17.02s
   Generated /Users/emilwareus/Development/polint-review/pr138-speed/target/doc/polint/index.html and 22 other files
```

## cargo deny check all

PASS; source log: `/tmp/pr138-checks/deny-frozen.log`.

```text
advisories ok, bans ok, licenses ok, sources ok
```

## CI polyglot Go/TS canary

PASS; source log: `/tmp/pr138-checks/canary-rerun.log`.

```text
test eval::go_rta::polyglot_go_ts_canary_resolves_go_edges_without_ts_interference ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2874 filtered out; finished in 0.71s

```

## CI capability certification rollup

PASS; source log: `/tmp/pr138-checks/certification-rerun.log`.

```text
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2874 filtered out; finished in 3.44s

```

## CI public surface leak check

PASS; source log: `/tmp/pr138-checks/public-surface.log`.

```text
test probe_crate_compiles_against_prelude_only ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 101.89s

```

## CI determinism gate

PASS; source log: `/tmp/pr138-checks/determinism-rerun.log`.

```text
test eval::determinism_gate::ts_reachable_fixture_is_byte_identical_under_ten_seeded_permutations ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 2863 filtered out; finished in 2.34s

```

## CI ignored semantic store boundary gate

PASS; source log: `/tmp/pr138-checks/semantic-store-rerun.log`.

```text
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2874 filtered out; finished in 52.46s

```

## cargo test -p polint --test cargo_install_smoke --locked -- --ignored

PASS; source log: `/tmp/pr138-checks/install-smoke.log`.

```text
test cargo_install_to_custom_prefix_runs_version ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 260.53s

```

## CI SARIF CLI run; schema validation logged separately

PASS; source log: `/tmp/pr138-checks/sarif-run.log`.

```text
```

## Dependency isolation

PASS for no languages, Go only and TypeScript only. Checked `cargo tree` against the CI parser-dependency patterns.

## Previous native full-suite failure

The macOS workspace run failed the pre-existing golden cost gate for `examples/go-guard-outcomes/json`: committed RSS delta was zero, measured RSS delta was 45,563,904 bytes. No golden files or assertions were weakened. The full suite is rerun in the prepared Linux environment with the same source hashes and required TypeScript tests enabled.
