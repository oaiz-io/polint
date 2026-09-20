#!/usr/bin/env bash
# probe.sh <tag> <cap> <paths...>
#
# One gate cell, as section 5.1 of
# research/strategy/plans/2026-09-19_full-app-deep-capability_plan.md defines it:
# a `polint unknowns --cap <cap> <paths>` run under the RSS sampler, with its
# stdout, stderr, cache root and per-process timeline kept side by side under
# $POLINT_GATE_OUT so a later cell can be compared against it.
#
# Nothing is written under the repository. Probe output is local and
# uncommitted; the committed artifact is the report scripts/deep-gate/report.py
# writes (plan section 6).
#
# Environment:
#   POLINT_GATE_OUT       output directory (default /tmp/polint-gate)
#   POLINT_GATE_REPO      repository to scan (default the current directory)
#   POLINT_GATE_TIMEOUT   sampler timeout in seconds (default 300)
#   POLINT_GATE_AS_GB     RLIMIT_AS ceiling in GB (default 28; 40 for a pre-W5
#                         full-backend cell, which reserves address space hard)
#   POLINT_BIN            the polint binary (default `polint` from PATH)
#   KEEP_CACHE=<cold tag> run this cell warm against that cold cell's cache root
#
# The cache root is a function of the tag, never of an inherited variable, so a
# warm cell run as a separate process resolves the same root the cold cell
# created: $POLINT_GATE_OUT/<tag>.cache cold, $POLINT_GATE_OUT/$KEEP_CACHE.cache
# warm. A cold cell wipes its root first, which moves the whole cache root
# including `sidecar`, so the Go semantic sidecar's 25-27 s and its heap are
# inside every cold measurement.
set -uo pipefail

# `tracing`'s pretty format writes SGR escapes between a field name and its `=`.
strip_ansi() { sed -E $'s/\x1b\\[[0-9;]*[a-zA-Z]//g' "$@"; }

if [ "$#" -lt 2 ]; then
  echo "usage: probe.sh <tag> <cap> <paths...>" >&2
  exit 2
fi

tag=$1
cap=$2
shift 2

# An override points the run at a binary neither of overlap.py's two sidecar
# rules can recognise, which would leave G2b's window empty and its bound
# vacuously true. Refuse rather than measure something unreadable.
if [ -n "${POLINT_GO_FRONTEND:-}" ]; then
  echo "probe.sh: POLINT_GO_FRONTEND is set; overlap.py cannot classify an overridden sidecar" >&2
  exit 2
fi

here=$(cd -- "$(dirname -- "$0")" && pwd)
repo_scripts=$(dirname -- "$here")
repo=$(cd -- "${POLINT_GATE_REPO:-.}" 2>/dev/null && pwd)
if [ -z "$repo" ]; then
  echo "probe.sh: no repository at ${POLINT_GATE_REPO:-.}" >&2
  exit 2
fi
# The run happens inside the repo, so a relative binary path has to be resolved
# from here, not from there.
binary=${POLINT_BIN:-polint}
case "$binary" in
  */*) binary=$(cd -- "$(dirname -- "$binary")" 2>/dev/null && pwd)/$(basename -- "$binary") ;;
esac
sampler="$(dirname -- "$repo_scripts")/.scale-envelope/rssrun.py"
stages="$(dirname -- "$repo_scripts")/.scale-envelope/stages.py"
if [ ! -f "$sampler" ]; then
  echo "probe.sh: no sampler at $sampler" >&2
  exit 2
fi

out=${POLINT_GATE_OUT:-/tmp/polint-gate}
mkdir -p "$out" || exit 2

if [ -z "${KEEP_CACHE:-}" ]; then
  cache="$out/$tag.cache"
  rm -rf "$cache"
  mkdir -p "$cache"
else
  cache="$out/$KEEP_CACHE.cache"
  if [ ! -d "$cache" ]; then
    echo "probe.sh: no cache root for cold cell $KEEP_CACHE" >&2
    exit 3
  fi
fi
echo "$cache" > "$out/$tag.cachedir"

# Twelve threads everywhere, so the Rust side, the rayon pool and the Go
# toolchain agree and the measurement is of one configuration.
export RAYON_NUM_THREADS=${RAYON_NUM_THREADS:-12}
export POLINT_JOBS=${POLINT_JOBS:-12}
export GOMAXPROCS=${GOMAXPROCS:-12}
export GOFLAGS=${GOFLAGS:--p=12}
export RUST_LOG=${RUST_LOG:-polint=debug}
export POLINT_CACHE_DIR="$cache"

(
  cd "$repo" || exit 2
  python3 "$sampler" --label "$tag" \
    --as-limit-gb "${POLINT_GATE_AS_GB:-28}" \
    --timeout "${POLINT_GATE_TIMEOUT:-300}" \
    --timeline "$out/$tag.timeline.json" \
    -- "$binary" unknowns --cap "$cap" "$@"
) > "$out/$tag.stdout" 2> "$out/$tag.stderr"
status=$?

echo "exit=$status"
# Tree peak (sum of VmRSS over the tree-plus-scan union, 200 ms samples) and wall.
grep '"peak_rss_gb"' "$out/$tag.stderr" | tail -1
# polint-process peak, getrusage(RUSAGE_SELF).ru_maxrss on the last stage row.
# `tracing` colours its field names, so `peak_rss_mb` and `=` are not adjacent in
# the capture; every reader of a stage row strips the escapes first, as
# stages.py and digests.py do.
strip_ansi "$out/$tag.stderr" | grep "stage done" | tail -1 | grep -oE 'peak_rss_mb=[0-9]+'
if [ -f "$stages" ]; then
  python3 "$stages" "$out/$tag.stderr"
fi
exit "$status"
