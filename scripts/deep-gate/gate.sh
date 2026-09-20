#!/usr/bin/env bash
# gate.sh — the local acceptance gate (plan sections 5.1, 5.2 and 6).
#
# Runs the G-matrix cells that apply to the checkout it is pointed at, compares
# each against a `before` capture, and prints one Markdown table per cell. No CI
# runs this, ever (invariant I7): it is a script the owner runs on a machine of
# his choice, and the committed artifact is the report scripts/deep-gate/report.py
# writes from this run's directory.
#
# Steps, in order:
#   0. the dense-id sweep (section 2.1 standing rule); a failure stops the gate
#   1. one probe per cell, through scripts/deep-gate/probe.sh
#   2. the provider-set check and the digest oracle (I1a) against the before capture
#   3. the fact-row oracle (I1b) against the before dump, when one is given
#
# Environment:
#   POLINT_GATE_REPO     repository to scan (required)
#   POLINT_GATE_SCOPES   cells as `label=path=cap` triples, whitespace separated
#   POLINT_GATE_OUT      output directory (default /tmp/polint-gate)
#   POLINT_GATE_BEFORE   directory holding <label>.stderr captures to compare against
#   POLINT_GATE_ROWS_BEFORE  directory holding <label>/ fact-row dumps to compare against
#   POLINT_GATE_ROWS=1   also produce this checkout's fact-row dumps, by running the
#                        `eval::fact_rows_dump::tests::dump_fact_rows` ignored entry
#                        per cell into $POLINT_GATE_OUT/<label>.rows
#   POLINT_GATE_ALLOW    the expected-move allowlist handed to factrows.py
#   POLINT_GATE_EXPECTED directory holding <label>.providers, one provider id per line
#   POLINT_GATE_SKIP_SWEEP=1  skip step 0 (for a tree that has no cargo available)
#   POLINT_BIN, POLINT_GATE_TIMEOUT, POLINT_GATE_AS_GB  passed to probe.sh
#
# `digest="-"` is a value, not a failure marker: polint.source, polint.ts.syntax
# and polint.metrics return `output_digest: None` on a successful run
# (analysis_kernel/provider.rs), and the kernel prints the dash for them. The gate
# compares it for equality across runs like any other digest and never rejects its
# presence. What `digests.py` cannot see is a provider that gained or lost a row,
# because it iterates the before capture's keys only; that is what the
# provider-set check in step 2 closes.
set -uo pipefail

here=$(cd -- "$(dirname -- "$0")" && pwd)
root=$(dirname -- "$(dirname -- "$here")")
out=${POLINT_GATE_OUT:-/tmp/polint-gate}
mkdir -p "$out" || exit 2
# probe.sh is a separate process and resolves its own paths from this; export it
# so a caller that set it as a shell variable does not get two directories.
export POLINT_GATE_OUT="$out"

fail=0
note() { printf '%s\n' "$*"; }
# `tracing`'s pretty format writes SGR escapes between a field name and its `=`,
# so a stage row is only greppable after they are stripped. digests.py and
# stages.py do the same thing in Python.
strip_ansi() { sed -E $'s/\x1b\\[[0-9;]*[a-zA-Z]//g' "$@"; }

# --- step 0: the dense-id sweep --------------------------------------------
if [ -z "${POLINT_GATE_SKIP_SWEEP:-}" ]; then
  note "## Step 0: dense-id sweep"
  if (cd "$root" && cargo test -p polint --test internal_architecture dense_id_sweep --locked) \
      > "$out/dense_id_sweep.log" 2>&1; then
    note "dense-id sweep: pass"
  else
    note "dense-id sweep: FAIL (see $out/dense_id_sweep.log)"
    tail -30 "$out/dense_id_sweep.log"
    note ""
    note "A new dense id in persisted key text or a payload part is a scope-dependent"
    note "fact row. Classify it against section 2.1 before measuring anything else."
    exit 1
  fi
  note ""
fi

if [ -z "${POLINT_GATE_REPO:-}" ]; then
  note "POLINT_GATE_REPO is required"
  exit 2
fi
if [ -z "${POLINT_GATE_SCOPES:-}" ]; then
  note 'POLINT_GATE_SCOPES is required: label=path=cap triples'
  exit 2
fi

# W3 commit 4 produces this; it is absent before slot 3b, which means "no
# normalisation" rather than an error (round 10, S-3).
sed_script="$here/normalize_r9_keys.sed"
sed_flag=()
[ -f "$sed_script" ] && sed_flag=(--sed "$sed_script")

for cell in $POLINT_GATE_SCOPES; do
  label=${cell%%=*}
  rest=${cell#*=}
  path=${rest%%=*}
  cap=${rest#*=}
  if [ "$label" = "$cell" ] || [ "$path" = "$rest" ]; then
    note "malformed cell '$cell': expected label=path=cap"
    fail=1
    continue
  fi

  note "## Cell $label ($cap, $path)"
  # report.py folds this file; gate.sh's own stdout is a transcript, not an input.
  verdict="$out/$label.verdict"
  : > "$verdict"
  POLINT_GATE_REPO="$POLINT_GATE_REPO" "$here/probe.sh" "$label" "$cap" "$path" \
    > "$out/$label.probe" 2>&1
  status=$?
  exit_line=$(grep -m1 '^exit=' "$out/$label.probe")
  summary=$(grep -m1 '"peak_rss_gb"' "$out/$label.stderr" 2>/dev/null)
  note ""
  note "| field | value |"
  note "|---|---|"
  note "| probe | ${exit_line:-exit=?} |"
  note "| sampler | ${summary:-none} |"
  if [ "$status" -ne 0 ]; then
    note "| verdict | FAIL (the probe did not exit 0) |"
    fail=1
    note ""
    continue
  fi

  # --- step 2a: the provider set ------------------------------------------
  observed="$out/$label.providers"
  strip_ansi "$out/$label.stderr" 2>/dev/null | grep 'stage done' \
    | grep -oE 'provider="?[a-z][a-z0-9_.]*"?' \
    | sed -E 's/provider="?([a-z0-9_.]*)"?/\1/' | sort -u > "$observed"
  note "| providers with a stage row | $(wc -l < "$observed" | tr -d ' ') |"
  echo "providers: $(wc -l < "$observed" | tr -d ' ')" >> "$verdict"
  expected="${POLINT_GATE_EXPECTED:-}/$label.providers"
  if [ -n "${POLINT_GATE_EXPECTED:-}" ] && [ -f "$expected" ]; then
    if diff -q <(sort -u "$expected") "$observed" >/dev/null; then
      note "| provider set | matches $label.providers |"
    else
      note "| provider set | FAIL (differs from $label.providers) |"
      diff <(sort -u "$expected") "$observed" | sed 's/^/    /'
      fail=1
    fi
  fi

  # Providers whose stage row carries `digest="-"`. Three healthy providers do
  # (polint.source, polint.ts.syntax, polint.metrics: they succeed with no output
  # digest), and so does a provider that ran and failed, so the dash is reported
  # and compared, never rejected. digests.py compares it for equality like any
  # other value, so a provider moving into or out of the dash fails the cell.
  dashes=$(strip_ansi "$out/$label.stderr" 2>/dev/null | grep 'stage done' \
    | grep -F 'digest="-"' \
    | grep -oE 'provider="?[a-z][a-z0-9_.]*"?' \
    | sed -E 's/provider="?([a-z0-9_.]*)"?/\1/' | sort -u | tr '\n' ' ')
  note "| providers with digest=\"-\" | ${dashes:-none} |"
  echo "digest_dash: ${dashes:-none}" >> "$verdict"

  # --- step 2b: the digest oracle (I1a) -----------------------------------
  before_stderr="${POLINT_GATE_BEFORE:-}/$label.stderr"
  if [ -n "${POLINT_GATE_BEFORE:-}" ] && [ -f "$before_stderr" ]; then
    digest_report=$(python3 "$root/.scale-envelope/digests.py" "$before_stderr" "$out/$label.stderr" 2>&1)
    digest_status=$?
    note "| digest oracle | $(printf '%s' "$digest_report" | tail -1) |"
    echo "digest: $(printf '%s' "$digest_report" | tail -1)" >> "$verdict"
    if [ "$digest_status" -ne 0 ]; then
      printf '%s\n' "$digest_report" | sed 's/^/    /'
      # Every moved digest must be on the allowlist, by provider id. A workstream
      # that moves a digest declares which ones; anything else fails the cell.
      unexpected=$(printf '%s\n' "$digest_report" \
        | grep -E '^(DIFFER|MISSING)' \
        | sed -E 's/^(DIFFER|MISSING) +([a-z0-9_.]+).*/\2/' \
        | while read -r id; do
            if [ -n "${POLINT_GATE_ALLOW:-}" ] \
               && grep -qE "^[[:space:]]*digest[[:space:]]+$id[[:space:]]*$" "$POLINT_GATE_ALLOW"; then
              continue
            fi
            printf '%s\n' "$id"
          done)
      if [ -z "$unexpected" ]; then
        note "| digest allowlist | every moved digest is declared |"
        digest_status=0
      else
        note "| digest allowlist | FAIL: $(printf '%s' "$unexpected" | tr '\n' ' ') |"
      fi
    fi
  else
    note "| digest oracle | no before capture |"
    digest_status=0
  fi

  # --- step 3: the fact-row oracle (I1b) ----------------------------------
  rows_before="${POLINT_GATE_ROWS_BEFORE:-}/$label"
  rows_after="$out/$label.rows"
  if [ -n "${POLINT_GATE_ROWS:-}" ]; then
    rm -rf "$rows_after"
    if (cd "$root" && POLINT_FACT_ROWS_REPO="$POLINT_GATE_REPO" \
          POLINT_FACT_ROWS_PATHS="$path" POLINT_FACT_ROWS_CAP="$cap" \
          POLINT_FACT_ROWS_OUT="$rows_after" POLINT_CACHE_DIR="$out/$label.rows.cache" \
          cargo test -p polint --lib --all-features --locked --release \
            eval::fact_rows_dump::tests::dump_fact_rows \
            -- --exact --ignored --nocapture) > "$out/$label.rows.log" 2>&1; then
      note "| fact-row dump | $(grep -m1 '^fact_rows_dump:' "$out/$label.rows.log") |"
    else
      note "| fact-row dump | FAIL (see $label.rows.log) |"
      fail=1
    fi
  fi
  if [ -n "${POLINT_GATE_ROWS_BEFORE:-}" ] && [ -d "$rows_before" ] && [ -d "$rows_after" ]; then
    allow_flag=()
    [ -n "${POLINT_GATE_ALLOW:-}" ] && allow_flag=(--allow "$POLINT_GATE_ALLOW")
    rows_report=$(python3 "$here/factrows.py" "$rows_before" "$rows_after" \
      ${allow_flag[@]+"${allow_flag[@]}"} ${sed_flag[@]+"${sed_flag[@]}"} 2>&1)
    rows_status=$?
    note "| fact-row oracle | $(printf '%s' "$rows_report" | grep -m1 'fact families identical') |"
    echo "factrows: $(printf '%s' "$rows_report" | grep -m1 'fact families identical')" >> "$verdict"
    if [ "$rows_status" -ne 0 ]; then
      printf '%s\n' "$rows_report" | sed 's/^/    /'
    fi
  else
    note "| fact-row oracle | no before dump |"
    rows_status=0
  fi

  if [ "$digest_status" -eq 0 ] && [ "$rows_status" -eq 0 ]; then
    note "| verdict | pass |"
    echo "verdict: pass" >> "$verdict"
  else
    note "| verdict | FAIL |"
    echo "verdict: fail" >> "$verdict"
    fail=1
  fi
  note ""

  if [ -s "$out/$label.stderr" ]; then
    note "### Stage rows ($label)"
    note ""
    python3 "$root/.scale-envelope/stages.py" "$out/$label.stderr" | sed 's/^/    /'
    note ""
  fi
done

if [ "$fail" -ne 0 ]; then
  note "gate: FAIL"
  exit 1
fi
note "gate: pass"
