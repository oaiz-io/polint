# Releasing

polint ships from a single workflow on `main`.

## Workflows (`.github/workflows/`)

| Workflow | Trigger | Purpose |
|---|---|---|
| `ci.yml` | Push/PR to `main` | `rustfmt`, then `clippy -D warnings` + `cargo test --workspace` on Ubuntu, Windows, and macOS. Includes an ignored `cargo install` smoke test that mirrors the crates.io install path. |
| `release.yml` | Manual (`workflow_dispatch` on `main`) | Bump via `scripts/bump-workspace-version.py` at the `bump_level` input (`patch` default, `minor`, `major`), push the bump commit to `main`, create the annotated tag `vX.Y.Z`, then optionally publish all crates to crates.io, attach CLI archives to the matching GitHub Release, and move the stable `v1` action tag. |

## GitHub Action versioning

The checked-in `action.yml` is published as `oaiz-io/polint@v1`. Release tags
like `v0.1.12` identify the CLI release assets that the action installs; the
action's major tag identifies the workflow contract and input/output
compatibility.

The release workflow moves the lightweight `v1` tag to the reviewed release
commit when **Publish action** is enabled. Breaking action input or output
changes require a new major tag.

## Secrets

| Secret | Required for | Notes |
|---|---|---|
| _(none)_ | `ci.yml` | Uses the default `GITHUB_TOKEN`. |
| _(none)_ | `release.yml` (typical) | `GITHUB_TOKEN` can push tags and manage releases when branch protection allows it. |
| `WORKFLOW_PUSH_TOKEN` | `release.yml` when `main` is protected | PAT with `contents: write` and the right to push to protected `main`. |
| `CRATES_IO_TOKEN` | `release.yml` with **Publish crates** on | Publish-scoped token from <https://crates.io/settings/tokens>. |

## Ship a version

1. Open **Actions → Release → Run workflow** on `main`.
2. Leave **Publish crates**, **Build CLI**, and **Publish action** on (defaults) for a full release; turn any off for a partial release.
3. The workflow:
   - bumps the workspace patch version,
   - commits and pushes to `main`,
   - creates and pushes the annotated tag `vX.Y.Z`,
   - publishes the `polint` crate to crates.io (when enabled),
   - builds the cross-platform CLI matrix and uploads archives to the GitHub Release for that tag (when enabled).
   - moves the stable `v1` GitHub Action tag to the release commit (when enabled).

## Smoke-test crates publish locally

```bash
DRY_RUN=1 ./scripts/publish-crates.sh
```

This walks the ordered publish without uploading anything.

## Manual bump (only outside the workflow)

```bash
python3 scripts/bump-workspace-version.py
cargo build --workspace
git commit -am "chore(release): bump crate version to <new>"
```

**Release always bumps again.** It runs the same script on whatever `main`
holds, so dispatching it after a manual bump ships the *next* version, not the
one just committed: a manual `0.4.0` plus a `minor` run tags `v0.5.0`. Pick one
path per release.

- To ship `X.Y.Z` through the workflow, leave `main` at the previous version and
  dispatch **Release** with the bump level that reaches `X.Y.Z`.
- To ship a version that is already committed on `main`, tag it yourself
  (`git tag -a vX.Y.Z -m "polint X.Y.Z" && git push origin vX.Y.Z`). The
  workflow has no tag trigger, so that path publishes nothing on its own: run
  `./scripts/publish-crates.sh` and attach the CLI archives separately.

## Rule pack API changes (SDK)

When upgrading the `polint` dependency in `.polint/rules/`, align rule code with the current SDK:

- **`Rule::run` return type:** use `RuleResult` (from `polint::sdk::prelude::*`) instead of `anyhow::Result<()>`. Errors are still created with `anyhow!` / `bail!` and converted with `.into()` where needed. The host maps `RuleError` to internal diagnostics the same way as before.

## Choosing the bump level

`release.yml` takes a `bump_level` input. It defaults to `patch`.

Below `1.0`, a caret requirement on `0.1.x` resolves to `>=0.1.x, <0.2.0`. `polint init` generates
`polint = "0.1.x"` into every rule pack, so **a patch release reaches every existing consumer on
their next `cargo update`.** Choose `minor` whenever a release removes or narrows anything on the
public SDK surface — a removed item, a field made private, or a type gaining `#[non_exhaustive]` —
so existing pins stay where they are until their owner chooses to move.
