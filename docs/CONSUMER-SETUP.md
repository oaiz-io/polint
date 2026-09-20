# Consumer setup and troubleshooting

## Rust toolchain

polint and repo-local rule crates target the MSRV in the workspace `Cargo.toml`.
Rule packs use **Rust 2024**.

If `polint check` fails while compiling `.polint/rules` with an MSRV error, align
`rust-toolchain.toml` (or your CI image) with that MSRV, or pin the compiler for
the child `cargo` process (see below).

## Go symbol/reference setup

Rules that request Go `Symbols<'_>` or `References<'_>` use the embedded
`polint-go-symbols` sidecar. The current implementation requires:

- Go 1.25 or newer on `PATH` when using default embedded source sidecars
- each analyzed Go file to belong to a Go module with a `go.mod`
- package loading to succeed for the configured package patterns and build tags

If those requirements are missing, polint emits `polint/capability` diagnostics
and blocks the Go symbol/reference rules instead of running them with placeholder
facts.

For simple repositories, no Go-specific config is needed. polint walks from each
discovered Go file to the nearest `go.mod` and loads those module roots. For
monorepos, keep the setup in the single `.polint.toml` file when you want an
explicit lifecycle:

```toml
[languages.go]
module_roots = ["services/payments", "libs/money"]
package_patterns = ["./..."]
build_tags = ["enterprise"]
include_tests = true
semantic_timeout_ms = 120000
```

`package_patterns` are interpreted inside each configured module root. If the
repository has a root `go.work` that covers every selected module root, polint
uses it. Otherwise, when package loading needs workspace mode for module roots
below the repository root, polint creates a temporary internal `go.work`; it does
not write another setup file into the repository.

### Bounding a Go semantic scan

The Go semantic sidecar type-checks the full dependency graph, builds SSA over
every package, and runs reachability analysis. It is the heaviest Go subprocess,
and it is bounded by wall time:

| Lever | Default | Effect |
|---|---|---|
| `semantic_timeout_ms` | `120000` | Budget for one sidecar run. `POLINT_GO_SEMANTIC_TIMEOUT_MS` overrides it for one run. |
| `package_patterns` | `["./..."]` per module root | Which packages are loaded and analysed. |
| `include_tests` | `true` | Whether `_test.go` files and their synthesized test packages are loaded. |

Exhausting the budget is a *reported outcome*: the provider fails and the rules
that needed it are blocked with `polint/capability` diagnostics. It never
silently reduces scope. Raising the number is the wrong first move — read the
per-stage timings first:

```bash
RUST_LOG=polint::kernel::stage=debug polint check --format json
```

Each sidecar stage (`packages_load`, `ssa_build`, `emit_rows`, `rta_analyze`)
logs its wall time alongside the packages, compiled Go files, and
type-checked dependencies it saw, and the same counters appear in
`summary.providers` of `--format json`. A run dominated by `packages_load` with
a large `deps_with_types` is a scope problem that `package_patterns` can fix; a
run dominated by `rta_analyze` is not.

**A rule's `files` list does not bound analysis.** Requesting a cross-file
capability (`calls`, `control_flow`, `dataflow`, `module_graph`, `references`,
`resolved_imports`, `symbols`) loads every discovered file regardless, because
those analyses cross file boundaries by definition. `files` narrows which
findings are *reported*. polint emits a `polint/scope` note when a rule's
`files` list is strictly narrower than the analysed set, so the distinction is
visible rather than folklore. Narrowing `[workspace] include` to a handful of
files is not a performance fix either: it breaks discovery of the imported
in-repo files those files depend on, which is a correctness hazard.

**What the discovered-file set does bound.** Two things follow the files a scan
discovered rather than `package_patterns`:

- The symbol sidecar loads the packages that hold those files. Every fact the
  symbol graph produces is anchored to a file and a fact naming an undiscovered
  file is dropped, so a package holding no discovered file cannot contribute a
  kept row.
- The semantic sidecar still loads the whole program, because the types and
  interface implementations its call-graph answers depend on can come from
  anywhere. It skips *emitting* the file-anchored rows the kernel would drop.

Neither narrows what the analysis knows. Both stop work whose result was already
being discarded.

Go semantic facts are derived in memory, but the sidecar's raw output is cached
on disk, keyed by the sidecar binary, the Go toolchain, the upstream syntax
digest, the lifecycle settings and the discovered-file set. A second run of an
unchanged repository reuses it and skips the sidecar round trip. `--no-cache`
disables that, so a `--no-cache` timing is a cold timing and is not what a normal
run costs.

## TypeScript type-directed call resolution

polint resolves TypeScript and JavaScript calls with a points-to solver by
default. When a TypeScript compiler is available it also runs a type sidecar
under Node and adds a *type-directed* tier above that solver: calls the checker
resolves exactly, plus the instantiated implementations an interface-typed call
can reach.

This tier is optional precision, not a capability. If it cannot run, nothing is
blocked and nothing fails — the points-to tier answers those calls as before.

### What it needs

- **Node** on `PATH`. The sidecar is plain JavaScript and needs no build step.
- **A TypeScript compiler**, resolved in this order:
  1. `POLINT_TS_TYPESCRIPT`, or `[languages.ts] typescript_path`
  2. the analyzed repository's own `node_modules/typescript`, searched from each
     project directory upward
  3. a global `npm` install
- **A `tsconfig.json`** at or above the analyzed files. polint walks from each
  discovered TS/JS file to the nearest one, the same walk import resolution
  uses; there is no separate project flag. A solution-style config — no inputs
  of its own, only `references` — is followed to the projects it references.

Preferring the repository's own compiler is deliberate: it is the compiler the
repository type-checks with, so the types polint sees are the types the
repository has. TypeScript 7 is refused rather than guessed at — it exposes no
stable programmatic API before 7.1 — and so are versions older than 4.x.

### Configuration

No configuration is needed for a repository that has a `tsconfig.json` and a
TypeScript install. Everything below is optional:

```toml
[languages.ts]
type_sidecar = true
type_projects = ["packages/web/tsconfig.json", "packages/api/tsconfig.json"]
typescript_path = "node_modules/typescript"
type_timeout_ms = 300000
```

Naming any of these keys tells polint the tier was asked for, which changes how
a setup gap is reported: a repository that never mentions the tier and has no
compiler stays silent, while one that asked for it gets a `polint/ts-types`
diagnostic explaining why it did not run. The provider row in
`summary.providers` of `--format json` is present either way, so a skip is never
invisible.

### What the tier will not claim

- **`any` is never a typed edge.** A call whose receiver the checker types as
  `any` or `unknown` produces an explicitly unresolved, low-confidence row
  rather than a target.
- **Density gates.** A file where at least 25% of call receivers are untyped has
  its typed answers reported as degraded; at 50% its inexact calls are left to
  the points-to tier entirely, while its exactly resolved calls still produce
  edges.
- **Expanded candidates are candidates.** An interface-typed call expands to the
  methods of classes this scan instantiates, which is a rapid-type
  over-approximation, and those edges carry medium confidence rather than the
  high confidence an exactly resolved call gets.
- **Files with no project get no typed edges.** A TS/JS file with no
  `tsconfig.json` above it is analysed by the other tiers only.

### Bounding a type-directed scan

| Lever | Default | Effect |
|---|---|---|
| `type_timeout_ms` | `300000` | Budget for one sidecar run. `POLINT_TS_TYPES_TIMEOUT_MS` overrides it for one run. |
| `type_projects` | discovered | Which tsconfig units are loaded. |
| `type_sidecar` | `true` | Whether the tier runs at all. |

The sidecar builds a full TypeScript program per project because a checker that
cannot see an import cannot type the call, but it emits rows only for the files
this scan discovered. As with the Go sidecar, narrowing the scan reduces
emission, not what the analysis knows.

Per-stage timings appear under the same tracing target as every other kernel
stage:

```bash
RUST_LOG=polint::kernel::stage=debug polint check --format json
```

Stages are `resolve_typescript`, `discover_projects`, `create_program` and
`walk_callsites`, and the same counters appear in `summary.providers` of
`--format json`, alongside `ts_types.dropped_rows` and
`ts_types.dangling_callees` — rows the store could not key or join, which are a
regression signal rather than a repository property.

The sidecar's raw output is cached on disk, keyed by the sidecar script, the
TypeScript version, the upstream syntax digest, the lifecycle settings, the
discovered-file set and the text of each project's `tsconfig.json` together with
everything it extends or references. Editing `strict`, `paths` or `include`
therefore invalidates the entry, even though it moves no TypeScript source.

## Inspect and test local rules

Use `polint inspect rule` to inspect registered repo-local rules before running
analysis:

```bash
polint inspect rule --format json
polint inspect rule --rule custom/no-raw-colors --format json
```

The JSON output is a stable public surface for rule manifests. It includes rule
ids, descriptions, severities, macro-derived fact views, capabilities, resolved
option metadata, and capability support rows. The schema is
[`polint-rule-inspect-v1.json`](schemas/polint-rule-inspect-v1.json).

Use `polint test` for fixture-based rule development:

```bash
polint test
polint test --format json
```

Fixtures live under:

```text
.polint/tests/rules/<rule>/<case>/polint-test.toml
```

Initial fixture manifests support:

```toml
rule = "custom/no-raw-colors"
paths = ["src/**"]

[[expect.diagnostic]]
rule_id = "custom/no-raw-colors"
file = "src/example.ts"
severity = "warn"
message_contains = "Project-specific policy"
range_start_line = 1
range_start_column = 21
```

`message_contains`, `range_start_line`, and `range_start_column` are optional.
The JSON report schema is
[`polint-test-report-v1.json`](schemas/polint-test-report-v1.json).

Agents can also use bounded public inspection JSON:

```bash
polint facts list --format json
polint facts sample --cap resolved_imports --limit 20 --format json
polint inspect unknowns --format json
polint unknowns --cap references --format json
polint explain --rule custom/no-raw-colors --format json
```

`polint facts` lists stable and reserved public fact-view dispositions and
samples only bounded public fields. `polint inspect unknowns` reports the
consolidated setup, unsupported, budget, model, and resolution queue.
`polint unknowns --cap ...` remains supported for cap-filtered compatibility and
returns an unsupported row for reserved capabilities. `polint explain` reports macro-derived fact views and
capability support without exposing provider execution graphs, layer-cache
internals, or eval/debug schemas.

## Environment variables

| Variable | Effect |
|----------|--------|
| `POLINT_CARGO` | Executable used to spawn repo-local rule hosts (default: `cargo` or `CARGO`). |
| `POLINT_JOBS` | Cap parallel work. Same values as `--jobs`: a core count, a percentage such as `80%`, or `0` to use every available CPU. Unset defaults to 80% of available CPUs. `--jobs` wins when both are set. |
| `POLINT_CACHE_DIR` | Optional cache root. Defaults to `.polint/cache` relative to the checked repository. |
| `POLINT_CACHE_STORE` | Absolute path to the machine-global store of compiled rule-host binaries, or `off` / `disabled` / `none` to share nothing. Defaults to the platform user cache directory (`$XDG_CACHE_HOME/polint/store`, `~/Library/Caches/polint/store`, `%LOCALAPPDATA%\polint\store`). |
| `POLINT_GO_SYMBOLS` | Optional path to a `polint-go-symbols` binary or sidecar source directory. A binary can avoid requiring Go for that sidecar; a source directory still needs Go. |
| `POLINT_GO_FRONTEND` | Internal/private override for the Go semantic frontend used by graph analysis experiments. A binary can avoid requiring Go for that sidecar; a source directory still needs Go 1.25+. This is not a rule-authoring SDK surface. |
| `POLINT_TS_TYPESCRIPT` | Path to the `typescript` package directory (or its entry script) the TS type sidecar should load. Overrides `[languages.ts] typescript_path` and the repository's own install. |
| `POLINT_TS_TYPES_TIMEOUT_MS` | Budget for one TS type sidecar run, overriding `[languages.ts] type_timeout_ms` for that run. |
| `POLINT_TS_TYPES_NODE` | Node executable used for the TS type sidecar. Defaults to `node` on `PATH`. |
| `POLINT_TS_TYPES_SIDECAR` | Internal/private override pointing at a TS type sidecar script. This is not a rule-authoring SDK surface. |
| `POLINT_RULES_PROFILE` | Cargo profile used for repo-local rule hosts. Defaults to `release`; set `dev` or `debug` for unoptimized rule-pack development, or any custom Cargo profile name. |
| `POLINT_RULES_TARGET_DIR` | Optional Cargo target directory for repo-local rule hosts. Defaults to `$POLINT_CACHE_DIR/rules-target`. |
| `POLINT_RULES_TOOLCHAIN` | When set to a non-empty value, forwarded as `RUSTUP_TOOLCHAIN` to every subprocess polint starts for a repo-local rule host (parent `polint check` only). |
| `NO_COLOR` | Disables ANSI colors when `--color auto`. |

## Cache management

polint stores local cache data under `.polint/cache` unless `POLINT_CACHE_DIR`
is set:

| Path | Role | Contents |
|------|------|----------|
| `.polint/cache/analysis` | source-validated | Compact JSON parser/fact artifacts. |
| `.polint/cache/layers` | source-validated | Persistent per-layer fact manifests and blobs. |
| `.polint/cache/derived` | source-validated | Reserved for future project-level derived facts. |
| `.polint/cache/semantic-store` | source-validated | Durable semantic store, when enabled. |
| `.polint/cache/rules-target` | compiler-output | Cargo target directory used when `polint check` builds repo-local rule packages. |
| `.polint/cache/extensions-target` | compiler-output | Cargo target directory used when the extension host builds `.polint/extensions/*`. |
| `.polint/cache/review` | scratch | Serialized `polint review` changesets, rewritten from the diff being reviewed. |

Source-validated directories hold analysis data that polint re-validates against
current sources on every read. Compiler-output directories hold Cargo builds,
whose freshness only the compiler can judge. Scratch is rebuilt from the current
inputs on every run and is worth neither caching nor restoring.

`polint cache status` reports the role next to each directory, and every
directory in this table is a `--category` value for `polint cache clean` and
`polint cache prune`.

Analysis cache keys include source path and content, loaded config, rule/options
digest, requested capability plan, cache format, and polint version. Changing
those inputs produces fresh cache entries instead of reusing stale facts.
If an individual cache artifact cannot be decoded, polint treats it as a miss
and removes that artifact.

Use `polint cache status` to inspect size and file counts. The JSON form is
stable enough for scripts and follows
[`polint-cache-status-v1.json`](schemas/polint-cache-status-v1.json):

```bash
polint cache status --format json
```

Use explicit cleanup when the cache grows too large:

```bash
polint cache prune --max-size-mb 512
polint cache prune --max-age-days 14 --dry-run
polint cache clean --category analysis
polint cache clean --category rules-target
polint cache clean --category extensions-target
polint cache clean
```

`--no-cache` on `polint check`, `polint baseline`, and `polint ignores` disables
analysis/fact cache reads and writes for that run. It does not disable the
repo-local rule-host Cargo target cache; use `polint cache clean --category
rules-target` when you need a fresh rule-host build.
Repo-local rule hosts run optimized by default (`POLINT_RULES_PROFILE=release`)
because rule execution can dominate large-repo scans. Use
`POLINT_RULES_PROFILE=dev` when iterating on rule-pack code and compile latency
matters more than scan latency.

In GitHub Actions, prefer the official action, which installs polint and
restores/saves the cache by default:

```yaml
- uses: oaiz-io/polint@v1
  with:
    version: latest
    args: check --format github
```

The action keeps two entries, because the roles in `.polint/cache` need different
keys: the source-validated directories under a key scoped to the polint version
and the resolved config/rule inputs, and the compiler-output directories under a
key built from compiler inputs. See
[the GitHub Action guide](GITHUB-ACTION.md) for the full key and invalidation
contract.

If you wire the steps manually, keep those roles apart. Caching `rules-target`
under an analysis-shaped key (no compiler version, no architecture, no compiler
flags) can restore compiler output that does not match the toolchain in use:

```yaml
- uses: actions/cache@v4
  with:
    path: |
      .polint/cache/analysis
      .polint/cache/layers
      .polint/cache/derived
      .polint/cache/semantic-store
    key: polint-analysis-${{ runner.os }}-${{ hashFiles('.polint.toml', '.polint/rules/Cargo.lock', '.polint/rules/**/*.rs') }}

# The compiler that will build the rule host, not just the file that pins it.
- id: rustc
  run: echo "id=$(rustc -vV | sha256sum | cut -d ' ' -f 1)" >> "$GITHUB_OUTPUT"

- uses: actions/cache/restore@v4
  id: rules-build
  with:
    path: |
      .polint/cache/rules-target
      .polint/cache/extensions-target
    key: polint-rules-build-${{ runner.os }}-${{ runner.arch }}-${{ steps.rustc.outputs.id }}-${{ hashFiles('.polint/rules/Cargo.toml', '.polint/rules/Cargo.lock', 'rust-toolchain.toml') }}

# … run polint here …

# A saved target directory must not carry a rule host: `actions/cache` never
# overwrites a key, so anything stored under it is what every later run gets.
# Remove the rule package's own output first, and save only after a run that
# finished — Cargo then has to rebuild the rule host from the sources in the
# next checkout, while the dependency builds stay reusable.
- if: ${{ steps.rules-build.outputs.cache-hit != 'true' && (steps.polint.outputs.exit-code == '0' || steps.polint.outputs.exit-code == '1') }}
  run: |
    spec="$(cargo pkgid --manifest-path .polint/rules/Cargo.toml)"
    CARGO_TARGET_DIR=.polint/cache/rules-target \
      cargo clean --release --manifest-path .polint/rules/Cargo.toml -p "${spec}"
    find .polint/cache/rules-target -maxdepth 3 -type d -name incremental -prune -exec rm -rf {} +

- if: ${{ steps.rules-build.outputs.cache-hit != 'true' && (steps.polint.outputs.exit-code == '0' || steps.polint.outputs.exit-code == '1') }}
  uses: actions/cache/save@v4
  with:
    path: |
      .polint/cache/rules-target
      .polint/cache/extensions-target
    key: ${{ steps.rules-build.outputs.cache-primary-key }}
```

The first run for a new cache key still compiles the rule packages and populates
analysis artifacts. The cache primarily improves repeat CI runs.

## Rules host failures

When the parent CLI compiles or runs `…/.polint/rules/Cargo.toml`,
failures are reported on stderr with the prefix:

`polint: rules host:`

Follow-up hints may mention:

- **MSRV** — polint library requires the workspace MSRV; see stderr and `rustc -V`.
- **Network / registry** — dependency fetch failures (VPN, offline, crates.io).
- **Manifest** — invalid `Cargo.toml` or workspace layout under `.polint/rules`.
- **Missing rustc** — install Rust or set `POLINT_RULES_TOOLCHAIN`.

See also the [README Versions](../README.md#versions) table.

## SARIF rule metadata

Optional map in `.polint.toml`:

```toml
[sarif.rule_help_uri]
"local/my-rule" = "https://example.com/docs/my-rule"
```

Values become SARIF `reportingDescriptor.helpUri` for matching `rule_id`s.

## Rule-specific settings

Each `[[rules.config]]` table supports common shortcuts (`severity`, `files`,
`allow_files`, `allow`, `max`, `deny`, `forbidden_imports`) plus arbitrary
rule-owned fields. Unknown fields are preserved in `ctx.options().settings`.

```toml
[[rules.config]]
id = "local/no-placeholder-literals"
files = ["src/**/*.ts"]
literal = "TODO"
message = "Replace placeholder literals before merging."
```

```rust
let literal = ctx
    .options()
    .settings
    .get("literal")
    .and_then(|value| value.as_str())
    .unwrap_or("TODO");
```

## Comment ignores

polint supports source comments for suppressing policy diagnostics:

```ts
// polint-ignore-next-line local/no-placeholder-literals -- generated fixture
const status = "TODO";
```

Selectors are required and use the same exact / `prefix/*` / `*` matching as
profiles. Repositories can require reasons:

```toml
[ignores]
require_reason = true
```

Use `polint ignores --stat` or `polint ignores --format json` to inspect active,
unused, malformed, and missing-reason ignores. See
[IGNORE-COMMENTS.md](IGNORE-COMMENTS.md).

## Monorepo path pairing

Optional section pairs left/right path shapes that share a context segment (same
string between configured prefix/suffix markers). See `[path_contexts]` in
`.polint.toml` and `RuleCtx::path_context_related` in the SDK after analysis.
