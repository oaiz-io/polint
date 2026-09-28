# L4 — Consumer-side: OAIZ's quadratic span helper (2026-09-28)

Situation: **consumer code, full-repo and core-profile runs**. Status: **consumer change — reported to
the OAIZ team, not polint's to ship.** This is the single largest measured win in the entire campaign.

This file documents a finding about the OAIZ rule pack for the OAIZ team's benefit. **No OAIZ source
code or diagnostic text appears below** — only the measured numbers, the shape of the problem, and the
shape of the fix, consistent with the campaign's hygiene rule (only basenames, rule ids, timings, and
counts leave the bench copy).

## What was found

The OAIZ pack's shared Go scanner computes each diagnostic's line/column span with a helper that
**rescans the file from byte 0 on every call** to find the line and column for a byte offset, rather
than indexing newlines once per file. Five rules in the pack use this shared scanner over every Go
file, and one additional rule has its own similar per-call line counter. Because span computation runs
once per finding location and Go files can produce many candidate spans, this is effectively quadratic
in file size for any rule that emits more than a handful of diagnostics per file.

## Measured evidence

Measured on a **scratch copy of the OAIZ pack** with the shared span helper and the one rule's line
counter both replaced by an equivalent linear-scan, per-source newline index — identical output
verified, same polint engine (`bbd1b785`), host-direct, 4 jobs:

| Metric | Before | After | Change |
|---|---|---|---|
| Full repo wall time | 598.7 s | 12.1 s | **49×** |
| Rules CPU time (full repo) | 1,777 s | 7.5 s | — |
| OAIZ core cold | 7.32 s | 5.38 s | −26.5% |
| OAIZ core warm | 5.79 s | 3.86 s | −33% |

stdout was byte-identical before and after in every case (full-repo: `9869ebc831e5d67f`; core:
`16af3691f174`) — this is purely a performance fix, not a behavior change.

## The SDK-shaped fix

The underlying problem is that the pack hand-rolls span computation instead of using spans that are
already indexed. polint's typed fact views — e.g. `Functions<'_>` and `GoTypeDecls<'_>` — expose spans
that are already line-indexed by the engine, computed once per file during parsing rather than
recomputed per call. Switching the affected rules to read spans from those views instead of computing
them with the shared scanner would eliminate the quadratic behavior without changing what the rules
check or report.

## Status

This is entirely a change to OAIZ's own rule pack source, not to polint. polint's role here was
measurement (proving the win exists and quantifying it on a scratch copy without touching the real
pack) and pointing at the SDK-native alternative that avoids the pattern. Nothing about this finding is
shippable in a polint release; it is reported here so the size of the opportunity is documented and the
fix direction is concrete for whoever picks it up on the OAIZ side.
