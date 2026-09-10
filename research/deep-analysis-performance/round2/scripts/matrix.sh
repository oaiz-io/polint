#!/usr/bin/env bash
# Full cold+warm matrix for one binary.  usage: matrix.sh <label> <binary> [suites...]
set -u
export PATH="/opt/data/home/.local/bin:$PATH"
unset POLINT_CACHE_STORE
LABEL="$1"; BIN="$2"; shift 2
SUITES=("$@")
if [ ${#SUITES[@]} -eq 0 ]; then
  SUITES=(excalidraw-excalidraw gohugoio-hugo jelly)
fi
R=research/evaluation-harness/repos
for suite in "${SUITES[@]}"; do
  for mode in deep syn; do
    echo "=== $LABEL $suite $mode ==="
    python3 .perf/measure.py --bin "$BIN" --repo "$R/$suite" --label "$LABEL" --mode "$mode" --warm-reps 3
  done
done
echo "MATRIX_DONE=$LABEL"
