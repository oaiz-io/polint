# Quick task 261126-spd: JS/TS analysis speed since #156

Owner-approved scope: fixes 1–6 of the investigation of the slowdown after
#156, as one pull request. No change in precision, recall or any fact; no
package slower in wall time.

## Tasks

1. Columns in O(log n): `internal_core/span.rs` `SourceTextIndex::line_col`
   counts characters from the line start for every span. Keep the offsets of
   UTF-8 continuation bytes; the column is the bytes since the line start
   less the continuation bytes among them. Tests: single-line and multi-byte
   sources, every offset, against the character count.
2. MIR lowering: share each file's closure tables between the functions
   lowered in it (`ts/mir/lower.rs` `TsMirLowering::lower_file`,
   `FunctionLowering`), and find a branch operation by its position in
   `set_branch_predicate`, keeping the scan as a fallback.
3. Entrypoint recognizers: look a MIR place up through the store's index
   (`AnalysisHost::mir_place`) instead of scanning every place per call site.
4. Callable-flow walk: copy-on-write tables in `FlowEnv` and `ObjectTargets`
   (`ts/callable_flow/extract.rs`).
5. Callable-flow passes per file in parallel, each worker parsing its own
   file, without the serial second parse of every file.

## Verification

- `bench.py report --no-cost --no-types --keep-dumps` on main and on the
  branch: every fact dump byte-identical (`cmp`).
- `make check`; tests that fail on unmodified main on this machine are shown
  to fail the same way there.
- Interleaved main/branch runs on commander, async, dayjs, underscore, pako,
  axios and zod, base and refined plans: `perf stat` instructions and wall
  time, with the load recorded.
