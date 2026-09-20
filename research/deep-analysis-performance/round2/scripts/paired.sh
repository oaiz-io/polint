#!/usr/bin/env bash
# Interleaved before/after sampling for one suite, so ambient load on a shared
# box lands on both binaries alike.
#   paired.sh <suite> <mode> <warm-reps>
set -u
export PATH="/opt/data/home/.local/bin:$PATH"
unset POLINT_CACHE_STORE
SUITE="$1"; MODE="${2:-deep}"; REPS="${3:-3}"
REPO="research/evaluation-harness/repos/$SUITE"
BASE=.perf/bin/baseline-polint-tests
NEW=.perf/bin/final-polint-tests

# Cold samples: each needs its own cleared cache, so take them back to back.
python3 .perf/measure.py --bin "$BASE" --repo "$REPO" --label paired-baseline --mode "$MODE" --warm-reps 0
python3 .perf/measure.py --bin "$NEW"  --repo "$REPO" --label paired-final    --mode "$MODE" --warm-reps 0
# Warm samples alternate against the cache both binaries share (identical
# provider schema digests), which also demonstrates cache compatibility.
for rep in $(seq 1 "$REPS"); do
  python3 .perf/measure.py --bin "$BASE" --repo "$REPO" --label paired-baseline --mode "$MODE" --warm-reps 1 --no-cold
  [ "$rep" = 1 ] || mv ".perf/results/paired-baseline/$SUITE-$MODE-warm1.json" ".perf/results/paired-baseline/$SUITE-$MODE-warm$rep.json"
  python3 .perf/measure.py --bin "$NEW" --repo "$REPO" --label paired-final --mode "$MODE" --warm-reps 1 --no-cold
  [ "$rep" = 1 ] || mv ".perf/results/paired-final/$SUITE-$MODE-warm1.json" ".perf/results/paired-final/$SUITE-$MODE-warm$rep.json"
done
echo "PAIRED_DONE=$SUITE/$MODE"
