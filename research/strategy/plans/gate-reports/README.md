# Deep-capability gate reports

Committed results of the local acceptance gate for the full-application
deep-capability track ([the plan](../2026-09-19_full-app-deep-capability_plan.md),
sections 5 and 6). One file per run:

```
<YYYY-MM-DD>_<host-label>.md
```

## Who writes these

`scripts/deep-gate/report.py`, and nothing else. A report is never edited by
hand and never pasted from a terminal. The gate itself is
`scripts/deep-gate/gate.sh`; a run leaves its raw output under
`$POLINT_GATE_OUT` (default `/tmp/polint-gate`), outside the repository, and
`report.py` folds that directory into the file committed here.

No CI runs the gate. There is no hosted job, no self-hosted runner, no schedule
and no `workflow_dispatch` for it, now or later (invariant I7). It is a script
the owner runs on a machine of his choice.

## Format

```markdown
# Deep-capability gate run: <date>, <host-label>

polint: <version> at <sha>; host: <cores> cores, <GB> RAM; threads: 12; cache: <cold|warm>

| Scope (file count) | cap | exit | wall s | tree peak MB (rssrun.py) | polint peak MB (stage row) | providers with rows | digest oracle | fact-row oracle |
|---|---|---:|---:|---:|---:|---:|---|---|
| 45 | calls | 0 | ... | ... | ... | 21 | 21/21 | identical |

## Stage rows (<scope>)
| provider | ms | rss MB | delta MB | peak MB | facts | keys | key MB |

## Gate verdicts
| Gate | pass/fail | measured | threshold |
```

Two memory figures, never mixed: the **tree peak** is the sampler's sum of
`VmRSS` over the process union (`.scale-envelope/rssrun.py`), and the **polint
peak** is `getrusage(RUSAGE_SELF).ru_maxrss` off the last `stage done` row. On a
Go cell the two differ by the sidecar's resident set; `scripts/deep-gate/overlap.py`
splits them.

`digest="-"` in a stage row is a value, not a failure: `polint.source`,
`polint.ts.syntax` and `polint.metrics` succeed with no output digest, and the
kernel prints the dash for them. The gate compares it for equality across runs
like any other digest.

The number of providers with a stage row is not a constant of the gate: it is
whatever closure the request computed on that tree (21 on a branch-base Go
`calls` cell, 23 on `dataflow`, 20 after W3). The report records what was
observed; `gate.sh` compares it against the cell's expected set.

## Hygiene

Permitted: file counts, timings, sizes, provider ids, rule ids of built-in
diagnostics (for example `polint/resource-budget`), gate names, scanned-file
basenames.

Forbidden: any scanned file path beyond a basename, any diagnostic message text,
any consumer identifier, any repository name. Scopes are identified by file
count only.

`report.py` enforces this with a positive allowlist, not a blocklist: a line
reaches the report only when it parses as a `stage done` row, the sampler's JSON
summary line, or a `polint/resource-budget` mention (counted, never quoted).
Everything else in the run's stderr is dropped. `report.py --strict` turns a
dropped line into an error, and `scripts/deep-gate/test_report.py` asserts both
behaviours against a stderr carrying a planted consumer path.

## Fact-row dumps

`fact-rows/` holds the committed I1b baselines, one file per cell per slot:

```
fact-rows/<slot>/MANIFEST.md     the cells, the tree they were taken from, the schema
fact-rows/<slot>/<cell>.txt      the condensed record of that cell's dump
```

A raw dump is one file per fact family — 93 per cell, most of them empty — which
is neither reviewable nor inside the delivery rule's file budget. What is
committed is what `scripts/deep-gate/factrows.py --summarize` renders from it:

* the dump schema, the capability and the scanned file count;
* per family, the row count and the SHA-256 of the family file, which proves
  byte-identity across two dumps exactly as a `diff` would;
* the five summary families' rows in full, because those are the ones an I1b
  allowlist is read against line by line (columns: key, payload digest, plaintext
  parts, attributes).

The raw dumps stay under `$POLINT_GATE_OUT`, outside the repository. A cell whose
scanned tree is a consumer repository never has its rows committed in any form:
its key text is consumer paths. The cells recorded here scan this repository's
own committed fixtures, so their key text is already in the repository.
