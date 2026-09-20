# Slot-1a fact-row dumps

The first I1b baseline: the `eval::fact_rows_dump::tests::dump_fact_rows` entry
run from the first tree that carries it, before W3 commit 0 lands. The section 2
baseline rule puts it here — a worktree at the branch base cannot run the entry,
so the branch base cannot be the I1b `before` side, and this dump is fact-row
identical to it by construction: the commit that produced the entry added a test
entry and scripts and changed no provider.

Tree: `6fc7ba3c` (`feat(gate): local probe and gate scripts with a
hygiene-checked report writer`), on `research/full-app-deep-capability`.
Dump schema: `polint-fact-rows-1`. Record format:
`scripts/deep-gate/factrows.py --summarize` (see `../../README.md`).

## Cells

Every cell scans a fixture repository committed in this repository, so its key
text is repository-owned and can be recorded here in full. The consumer scopes
the plan's slot-1 table also names (excalidraw, 45-file, 885-file, full backend)
are not on this host: `research/evaluation-harness/repos/` is empty, so those
cells could not be captured and are left to the slice that has the corpus.

| cell | scanned fixture | cap | files | rows | summary payload column |
|---|---|---|---:|---:|---|
| `fx-scc-calls` | `tests/eval-fixtures/direct-summaries/scc-closure/repo` | `calls` | 2 | 1347 | id-only (39 rows) |
| `fx-scc-control-flow` | `tests/eval-fixtures/direct-summaries/scc-closure/repo` | `control_flow` | 2 | 921 | id-only (39 rows) |
| `fx-scc-dataflow` | `tests/eval-fixtures/direct-summaries/scc-closure/repo` | `dataflow` | 2 | 1507 | id-only (39 rows) |
| `fx-summaries-calls` | `tests/eval-fixtures/direct-summaries/core/repo` | `calls` | 2 | 1482 | digest recipe (63 rows) |
| `fx-domains-dataflow` | `tests/eval-fixtures/abstract-domains/core/repo` | `dataflow` | 2 | 3863 | digest recipe absent; id-only (23 rows) |

The last column is what W3 commit 0 is about, and it is read off the dump rather
than assumed. On four of the five cells every summary row's payload column is
`summary:<SummaryId>` or `summary-event:<SummaryEventId>` text: the SCC closure
updated at least one summary there, set its dirty flag, and the
`AnalysisHost` trait default re-recorded all five families as id-only text,
overwriting what `core/db.rs` had written. On `fx-summaries-calls` it did not,
so those 63 rows still carry the FNV recipe's value. That cell is the control:
W3 commit 0 must leave it byte-identical while the other four move on every
summary row and nowhere else.

## How to reproduce

```sh
POLINT_FACT_ROWS_REPO=<repo> POLINT_FACT_ROWS_CAP=<cap> \
POLINT_FACT_ROWS_OUT=<dir> POLINT_CACHE_DIR=<fresh dir> \
  cargo test -p polint --lib --all-features --locked --release \
    eval::fact_rows_dump::tests::dump_fact_rows -- --exact --ignored --nocapture
python3 scripts/deep-gate/factrows.py <dir> --summarize
```

The raw dumps (93 files per cell) are not committed; the per-family SHA-256 in
each record is the identity claim, and the five summary families' rows are
inline because an I1b allowlist is read against them line by line.
