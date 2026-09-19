# Slot-1b fact-row dumps

The same five cells as `../slot-1a/`, re-run from the tree that carries W3
commit 0. The pair is that commit's I1b oracle, and from here on it is the
`before` side every other workstream in slots 2 and 3 compares against (section 2
baseline rule).

Tree: `1955298b` (`fix(summaries): route the SCC closure's metadata refresh to the
digest recipe on AnalysisDb; delete the dead inherent twin`), on
`research/full-app-deep-capability`.
Dump schema: `polint-fact-rows-1`. Record format:
`scripts/deep-gate/factrows.py --summarize`.

## What moved

`scripts/deep-gate/factrows.py <slot-1a> <slot-1b> --allow
scripts/deep-gate/allow/w3-commit-0.txt`, per cell:

| cell | families identical | summary rows whose payload column moved | rows moved outside column 2 |
|---|---:|---:|---:|
| `fx-scc-calls` | 88/93 | 39 | 0 |
| `fx-scc-control-flow` | 88/93 | 39 | 0 |
| `fx-scc-dataflow` | 88/93 | 39 | 0 |
| `fx-summaries-calls` | 93/93 | 0 | 0 |
| `fx-domains-dataflow` | 88/93 | 23 | 0 |

140 payload columns in total, every one of them from `summary:<SummaryId>` or
`summary-event:<SummaryEventId>` text to a sixteen-character FNV hex digest. The
five families that moved are exactly `SummaryControl`, `SummaryCall`,
`SummaryMemory`, `SummaryTito` and `SummaryEvent`; the other 88 families'
per-family SHA-256 is unchanged in every cell, which the committed records carry
and anyone can re-check without the raw dumps.

`fx-summaries-calls` is the control. Its SCC closure updates no summary, so the
trait default never ran there and its 63 summary rows already carried the digest
recipe's value; after the commit they are byte-identical. A change that had
touched the recipe rather than the route would have moved them too.

Columns 1, 3 and 4 — the canonical key text, the plaintext parts and the
attributes — are identical on every row of every cell. That is what makes the
pair the oracle: the commit is a route change, and a route change may move the
column that records which route ran and nothing else.

## I1a, for the same commit

`polint unknowns` under `scripts/deep-gate/probe.sh` before and after, three
cells, compared with `.scale-envelope/digests.py`:

| cell | cap | providers | verdict |
|---|---|---:|---|
| `direct-summaries/scc-closure` | `calls` | 21 | 21/21 provider output digests identical |
| `direct-summaries/scc-closure` | `control_flow` | 21 | 21/21 provider output digests identical |
| `abstract-domains/core` | `dataflow` | 23 | 23/23 provider output digests identical |

`polint unknowns` stdout is byte-identical on all three. No provider output
digest folds `FactMeta::payload_digest`, so I1a could not have moved; this
measures it rather than asserting it.
