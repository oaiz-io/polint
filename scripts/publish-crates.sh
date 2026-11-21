#!/usr/bin/env bash
# Publish the public polint crates to crates.io.
# Prerequisites: clean git tree (CI) or pass --allow-dirty only locally.
#
# Usage:
#   CRATES_IO_TOKEN=... ./scripts/publish-crates.sh
#   DRY_RUN=1 ./scripts/publish-crates.sh   # cargo publish --dry-run/package smoke
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# In dependency order. polint-oxc-parser is outside the workspace (it keeps
# upstream Oxc's lints and formatting), so it is published by manifest path and
# resolves its own dependencies.
PACKAGES=(
  polint-oxc-parser
  polint-macros
  polint
)
OXC_PARSER_MANIFEST=crates/polint-oxc-parser/Cargo.toml

crate_version() {
  cargo pkgid -p "$1" | sed 's/.*#//'
}

publish_package() {
  local name="$1"
  shift
  if [[ "$name" == polint-oxc-parser ]]; then
    cargo publish --manifest-path "$OXC_PARSER_MANIFEST" "$@"
  else
    cargo publish -p "$name" --locked "$@"
  fi
}

crate_version_exists() {
  local name="$1"
  local version="$2"
  local tmp_dir
  local status

  tmp_dir="$(mktemp -d)"

  (
    cd "$tmp_dir"
    cargo info "${name}@${version}" --registry crates-io >/dev/null 2>&1
  )
  status=$?

  rmdir "$tmp_dir"
  return "$status"
}

wait_for_crate_version() {
  local name="$1"
  local version="$2"

  for _ in {1..30}; do
    if crate_version_exists "$name" "$version"; then
      return 0
    fi
    sleep 10
  done

  echo "error: ${name} ${version} was not visible in Cargo registry metadata after publishing" >&2
  return 1
}

publish_crate() {
  local name="$1"
  local version

  version="$(crate_version "$name")"
  if crate_version_exists "$name" "$version"; then
    echo "Skipping ${name} ${version}; already published."
    return 0
  fi

  echo "Publishing ${name}..."
  publish_package "$name"

  wait_for_crate_version "$name" "$version"
}

if [[ "${DRY_RUN:-}" == "1" || "${DRY_RUN:-}" == "true" ]]; then
  echo "DRY_RUN: smoke-check packaging for ${PACKAGES[*]}."
  publish_package polint-oxc-parser --dry-run --allow-dirty
  publish_package polint-macros --dry-run --allow-dirty

  unpublished=()
  for dependency in polint-oxc-parser polint-macros; do
    if ! crate_version_exists "$dependency" "$(crate_version "$dependency")"; then
      unpublished+=("$dependency $(crate_version "$dependency")")
    fi
  done
  if [[ ${#unpublished[@]} -eq 0 ]]; then
    publish_package polint --dry-run --allow-dirty
  else
    echo "DRY_RUN: polint depends on ${unpublished[*]}, not on crates.io yet."
    echo "DRY_RUN: checking polint package contents; full polint publish dry-run is possible after they are published."
    cargo package -p polint --list --locked --allow-dirty >/dev/null
  fi

  printf '\nPublish: %s\n' "${PACKAGES[*]}"
  exit 0
fi

if [[ -z "${CRATES_IO_TOKEN:-}" && -z "${CARGO_REGISTRY_TOKEN:-}" ]]; then
  echo "error: set CRATES_IO_TOKEN or CARGO_REGISTRY_TOKEN (create at https://crates.io/settings/tokens)" >&2
  exit 1
fi

if [[ -n "${CRATES_IO_TOKEN:-}" && -z "${CARGO_REGISTRY_TOKEN:-}" ]]; then
  export CARGO_REGISTRY_TOKEN="$CRATES_IO_TOKEN"
fi

for p in "${PACKAGES[@]}"; do
  publish_crate "$p"
done

echo "Done. Verify: https://crates.io/crates/polint"
