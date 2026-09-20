# Cache


polint keeps local, untracked cache data under `.polint/cache` by default:

```text
.polint/cache/
  analysis/           compact parser/fact JSON artifacts     [source-validated]
  layers/             per-layer fact manifests and blobs     [source-validated]
  derived/            reserved for project-level derived facts [source-validated]
  semantic-store/     durable semantic store, when enabled   [source-validated]
  rules-target/       Cargo target dir for repo-local rule hosts [compiler-output]
  extensions-target/  Cargo target dir for repo-local extensions [compiler-output]
  review/             serialized `polint review` changesets  [scratch]
```

Source-validated data is re-validated against current sources on every read;
compiler output is Cargo's to judge; scratch is rebuilt from the current inputs.
CI has to key those roles differently: see the
[GitHub Action guide](GITHUB-ACTION.md).

`--no-cache` disables analysis/fact cache reads and writes for that run. It does
not disable the `rules-target` Cargo cache, because rebuilding the local rule
host on every run is usually wasted work.
Analysis cache keys include the source path and content, rule/options digest,
loaded config, requested capability plan, cache format, and polint version.
If a cache artifact cannot be decoded, polint treats it as a miss and removes
that artifact.

Useful commands:

```bash
polint cache status
polint cache status --format json
polint cache prune --max-size-mb 512
polint cache prune --max-age-days 14 --dry-run
polint cache clean --category analysis
polint cache clean
```

Set `POLINT_CACHE_DIR` to move the whole cache root, or
`POLINT_RULES_TARGET_DIR` to move only the repo-local rule-host Cargo target
directory. Repo-local rule hosts run with Cargo's `release` profile by default;
set `POLINT_RULES_PROFILE=dev` for faster unoptimized local rule development, or
to another Cargo profile name to build with `--profile <name>`.

### Shared rule-host store

`.polint/cache` is per checkout, so every git worktree of a repository would
otherwise recompile the same rule host from the same bytes. polint shares the
compiled **rule-host binaries**, nothing else, through one machine-global
store:

```bash
POLINT_CACHE_STORE=/path/to/store polint check   # a store you name
POLINT_CACHE_STORE=off polint check              # no sharing this run
```

Unset, the store lives in the platform user cache directory:
`$XDG_CACHE_HOME/polint/store` (else `~/.cache/polint/store`) on Linux,
`~/Library/Caches/polint/store` on macOS, and `%LOCALAPPDATA%\polint\store` on
Windows. `off`, `disabled`, or `none` turns sharing off; so does any value that
is not an absolute path, because a relative one would name a different store
from every directory.

For packages eligible for sharing, each entry is keyed by the build's
**complete input surface**: the resolved `rustc` and `cargo`, the toolchain
override, the compiler-flag environment variables, the cargo profile, the OS
and architecture, the repository's
`Cargo.toml`/`Cargo.lock`/`rust-toolchain*`, every applicable ancestor and Cargo
home `.cargo/config*`, the rule package's manifests and lockfile, and the content
of every file in the package, Rust sources, `build.rs`, and data consumed by
`include_bytes!` or a build script. The fingerprint also records whether the
package tripped the checkout-path gate described below. Everything is hashed
from bytes, never modification times. A restored binary is re-hashed against
the length and digest the entry recorded before it is moved into place and run,
and any failure, a missing entry, an unreadable one, a corrupt blob, is a miss
that compiles locally instead. Committing the rule package's `Cargo.lock` is
what lets a fresh checkout compute the same key the build in another one
published under. Cargo configuration is hashed by content and never by
location, so two machines or containers whose cargo homes sit at different
paths but hold the same `config*` files compute the same key and share one
compiled host.

Three constructs make polint build locally without restoring or publishing a
machine-global host: any `path` dependency that leaves the rule package
(including an absolute path, whose contents have no lockfile checksum); a cargo
config with `patch`, `replace`, `paths`, source replacement, `include`, `[env]`,
or a target runner; and Rust source that can embed checkout-specific values
Cargo injects. The last check scans `*.rs` under `src`, `benches`, `examples`,
and `tests`, plus a root `build.rs`, for the byte tokens
`CARGO_MANIFEST_DIR`, `OUT_DIR`, and the executable environment names `CARGO`
and `RUSTC`. It is intentionally broad: even a token in a comment opts that
package out, because a false positive costs one local build while a false
negative could restore a host compiled for another checkout. `CARGO_PKG_*`
values are not gated because they come from checkout-independent package
metadata. A config or source tree polint cannot read or prove is local-only too.

The store is one user's state on one machine, at the trust level of
`~/.cargo/registry`: polint creates it `0700` on Unix and never shares it between
users or over a network. Point `POLINT_CACHE_STORE` only at local storage you
alone can write. Deleting the directory is always safe.

### Contributor build cache

The repository disables Cargo incremental compilation because its artifacts are
stored separately in every worktree and can grow much larger than the final
build outputs. For shared compile reuse across worktrees, install
[`sccache`](https://github.com/mozilla/sccache) and configure it once in
`~/.cargo/config.toml`:

```toml
[build]
rustc-wrapper = "sccache"
```

The repository caps sccache's shared local cache at 10 GiB. Each worktree keeps
its own `target` directory, avoiding Cargo lock contention and collisions
between concurrently developed branches.

