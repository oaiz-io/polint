# GitHub Action

The repository ships a composite GitHub Action that installs polint, restores
the documented `.polint/cache` directories, runs polint, and saves the caches
after the run.

```yaml
name: polint

on: [push, pull_request]

jobs:
  polint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: oaiz-io/polint@v1
        with:
          version: latest
          args: check --format github
```

`--format github` emits GitHub Actions annotations with file, line, column,
severity, rule id, and message. The command still exits according to polint's
normal `--fail-on` behavior. Parallel work defaults to 80% of available CPUs;
pass `--jobs` in `args` (`8`, `50%`, or `0` for every CPU) to change that.

## Inputs

| Input | Default | Notes |
|---|---:|---|
| `version` | `latest` | Installs a GitHub Release asset. Accepts `latest`, `vX.Y.Z`, or `X.Y.Z`. If an asset is missing, the action falls back to `cargo install polint --locked`. |
| `args` | `check --format github` | Arguments passed to `polint`. |
| `cache` | `true` | Master switch for both cache entries below. |
| `cache-rule-builds` | `true` | Restores and saves Cargo build intermediates for repo-local rule and extension packages. Ignored when `cache` is `false`. |
| `rule-paths` | empty | Override the rule package directories the build cache covers, newline- or comma-separated. Empty reads `[rules].paths` from `.polint.toml`. |
| `build-cache-max-size-mb` | empty | Refuse to save the build cache when the pruned directories exceed this many MB. Empty reports the size without a ceiling. |
| `cache-key-prefix` | `polint` | Prefix for the GitHub Actions cache keys. |
| `working-directory` | `.` | Directory where `polint` should run. |
| `fail-on` | empty | Optional convenience value appended as `--fail-on <value>`. Leave empty if `args` already contains `--fail-on`. |

## Outputs

| Output | Notes |
|---|---|
| `version` | Installed polint version. |
| `cache-hit` | Whether the exact analysis cache key was restored. |
| `rule-build-cache-hit` | Whether the exact build cache key was restored. Empty when build caching did not run. |
| `rule-build-cache-skipped` | Why build caching did not run, or empty when it ran. |
| `rule-build-cache-save-skipped` | Why the build cache was not saved after this run, or empty when it was saved or never ran. |
| `rule-build-cache-size-mb` | Size of the build cache directories after pruning, or empty when they were not measured. |
| `exit-code` | polint process exit code. |

## Caching

The action keeps two cache entries. They exist because the directories under
`.polint/cache` have two different roles, and a role decides both what may key
an entry and what may be restored from it. `polint cache status` prints the role
of each directory.

### 1. Analysis cache (source-validated artifacts)

```text
.polint/cache/analysis
.polint/cache/layers
.polint/cache/derived
.polint/cache/semantic-store
```

Key: `<prefix>-analysis-<runner os>-<polint version>-<analysis digest>`

The analysis digest covers `.polint.toml`, the repository `Cargo.lock`,
`rust-toolchain.toml`, and — for every rule package the action resolved — its
`Cargo.toml`, `Cargo.lock`, and `src/**/*.rs`. Resolving the packages first is
what lets a repository with custom `[rules].paths` be covered at all; when the
paths cannot be resolved, the digest records that fact and falls back to the
default `.polint/rules` layout.

Fallback: the same key without the digest, so an older entry for the same polint
version can warm-start a run. There is no fallback across polint versions,
because polint stamps its own artifact keys with the version and would reject
those entries anyway.

Restoring this entry never bypasses validation. polint keys every artifact by
source path and content, loaded config, rule/options digest, requested
capability plan, cache format, and polint version; anything that does not match
the current sources is a miss, and an artifact that cannot be decoded is
evicted. A restored entry can only save recomputation of facts about files that
are byte-identical to what is being analyzed now. That is also why this entry is
saved even when the run reported findings or failed: a partial entry costs a
miss, never a wrong answer.

Repositories without repo-local rules still work; missing files simply do not
contribute to the digest.

### 2. Build cache (compiler output)

```text
.polint/cache/rules-target
.polint/cache/extensions-target
```

These are the Cargo target directories `polint check` uses when it builds
repo-local rule packages (`--manifest-path .polint/rules/Cargo.toml`)
and when the extension host builds `.polint/extensions/*`. Compiling those
packages means compiling the `polint` library and its dependencies, which
dominates the check phase on a cold runner.

Key: `<prefix>-rules-build-v2-<runner os>-<runner arch>-env-<build env digest>-deps-<dependency digest>`

The build env digest covers:

- runner OS and architecture
- the resolved Cargo profile for rule hosts (`POLINT_RULES_PROFILE`)
- `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_BUILD_RUSTFLAGS`
- `CARGO_BUILD_TARGET`, `CARGO_INCREMENTAL`
- `RUSTC_WRAPPER`, `RUSTC_WORKSPACE_WRAPPER`, `RUSTUP_TOOLCHAIN`
- the resolved `rustc -vV` and `cargo -V`, read from the working directory so a
  checked-in `rust-toolchain.toml`, a pinned action toolchain, or a floating
  `stable` that has moved are all reflected

The dependency digest covers the files that decide which dependency units Cargo
resolves and builds:

- repository `Cargo.toml`, `Cargo.lock`
- `rust-toolchain.toml`, `rust-toolchain`
- `.cargo/config.toml`, `.cargo/config`
- the resolved list of covered packages, and the `Cargo.toml` + `Cargo.lock` of
  each one

`.polint.toml` contributes only through that resolved package list. Nothing else
in it reaches the compiler, so hashing the whole file would throw away reusable
dependency builds on every unrelated config edit.

The **installed polint version is deliberately not in this key.** The CLI does
not compile the rule host; the rule package's own manifest and lockfile pin the
`polint` library it links against, and those are in the dependency digest. A
polint release that does not change the library the rule packs depend on should
not discard a valid dependency build.

Fallback: exactly one key, identical up to and including the build env digest,
relaxing only the dependency digest. Every restored unit still comes from the
same compiler, flags, profile, and architecture, and Cargo re-fingerprints each
unit before reusing it. There is no broader fallback.

### Why a restored build cache can never carry a stale rule host

Rule sources are not part of the build key, and they do not need to be: **the
action removes every covered package's own output from the target directories
before saving them.** After `polint` runs, and before `actions/cache/save`, the
action runs `cargo clean --package <pkgid>` for each rule and extension package
against the same target directory and profile the build used, then verifies that
nothing named after that package survives anywhere under it. Incremental state
is dropped too, since it only accelerates a recompile that now always happens.

So every entry this action writes is an entry with no rule-host binary, no
rule-host rlib, and no rule-host fingerprint in it. On the next run — exact hit
or fallback hit — Cargo has nothing to reuse for the rule package and must
rebuild it from the sources in that checkout, while the dependency units, the
expensive part, are reused. That is a property of the cache contents, not of
file timestamps.

Measured locally on a rule pack with a path dependency on the `polint` library
(x86_64 Linux, release profile, 223 compiled units): a cold build takes 185.4 s;
after pruning, the rebuild recompiles exactly one unit — the rule package — in
0.7 s. The target directory is 562 MB before pruning and 537 MB after, the
difference being the rule package's own output plus incremental state.

Pruning happens in the working directory, so a later step in the same job that
runs polint again rebuilds the rule package once more — the same 0.7 s the next
job would have paid anyway. Nothing else is recompiled.

### When a build cache entry is saved

Two conditions, both necessary:

- **The exact key missed.** `actions/cache` never overwrites an existing key.
- **The run finished.** polint exits `0` when clean and `1` when it reports
  findings at or above the fail-on threshold; both mean the rule hosts built and
  ran. Any other exit code, or no exit code at all because the job was cancelled
  before polint finished, means the target directory describes an interrupted
  build — and one such entry saved under a key would be what every later run
  restores from it. The reason lands in `rule-build-cache-save-skipped`.

Pruning failures are treated the same way: if the action cannot prove it removed
the rule package's output, it does not save. Skipping a save only costs the next
run its speedup.

### When build caching is skipped entirely

The action reports the reason in `rule-build-cache-skipped` and in the job log:

| Condition | Reason |
|---|---|
| `cache-rule-builds` is not `true` | opted out |
| `POLINT_CACHE_DIR` resolves outside `<working-directory>/.polint/cache` | the cache root moved; the action also skips the analysis cache |
| `POLINT_RULES_TARGET_DIR` resolves outside `<cache root>/rules-target` | the target directory left the layout the action caches |
| `[rules].paths` cannot be decoded from `.polint.toml` and `rule-paths` is unset | the key would not cover every rule package |
| a configured rule path is absolute, contains `..`, resolves outside the working directory, or has no `Cargo.toml` | the path is not a repo-local rule package this action can cover |
| no rule or extension package found | nothing to build |
| no Rust toolchain, or no `sha256sum`/`shasum` | the key cannot be computed |
| `build-cache-max-size-mb` is exceeded | reported as a save skip, not a restore skip |

Both environment overrides are resolved the way polint resolves them —
`POLINT_CACHE_DIR` against the repository, `POLINT_RULES_TARGET_DIR` against the
cache root — so setting either to the location the action already caches is not
treated as a move.

Skipping only forgoes the speedup: `polint check` still builds and runs the rule
hosts, and reports build failures itself. The same holds if cache resolution
fails outright — the action warns, runs with no cache, and never fails the job
for a failed optimization.

### Reading `[rules].paths`

The action has to know which packages the build cache covers before polint runs,
so it reads `[rules].paths` from `.polint.toml` itself. It masks string bodies
first and then reads structure, so `[rules]` tables, dotted `rules.paths` keys,
`rules = { paths = [...] }` inline tables, multi-line arrays, literal strings,
and comments containing brackets all resolve to the same answer polint's own
TOML parser gives. Any shape it cannot decode byte for byte — multi-line
strings, escapes inside a path, a duplicate `[rules]` table, or a config past a
size bound far above any real one — is reported as undecodable and the build
cache is skipped, rather than guessed at. Set `rule-paths` to cover such a
repository explicitly.

### What is not cached

`.polint/cache/review` holds serialized `polint review` changesets. Each file is
named after a hash of the changeset it contains and is rewritten from the diff
being reviewed, so restoring one would save no work and only grow the entry. It
is deliberately in neither entry; `polint cache clean --category review` removes
it locally.

### Cache growth

The action reports the size of the pruned build cache directories in the job
summary and in `rule-build-cache-size-mb` on every run that saves. There is no
default ceiling, because the right one depends on the repository: GitHub evicts
caches least-recently-used within a 10 GB per-repository budget, so a large
polint entry is not an error — it is a trade against every other cache in that
repository. Set `build-cache-max-size-mb` when you want a specific bound; the
action then reports the size and refuses the save above it.

Cargo does not remove superseded units from a target directory, so the entry
grows as dependency sets change. The dev profile (`POLINT_RULES_PROFILE=dev`)
grows fastest. To retire entries deliberately, bump `cache-key-prefix`.

Use `polint cache status` locally or in CI to inspect the restored cache, and
`polint cache prune` / `polint cache clean` when you need explicit cleanup.

## Versioning

The action is published from this repository as `oaiz-io/polint@v1`. The
release workflow moves the `v1` tag to the reviewed release commit when
`publish_action` is enabled. Patch and minor polint releases can continue to use
`version: latest`; pin `version: vX.Y.Z` when a workflow must run an exact CLI
release.
