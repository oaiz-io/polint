---
quick_id: 261001-n6a
status: complete
date: 2026-10-02
branch: perf/deep-analysis
---

# Quick Task 261001-n6a: deep-analysis step 0 repairs — Summary

**Date:** 2026-10-02
**Status:** Implemented, measured, gated (gate 0 PASS on all four lines); branch pushed, no PR.

## Completed

- **Symbols sidecar loads modules newer than the host Go.** The synthetic `go.work` takes the
  highest `go` directive of the module roots and lives in a private temporary directory removed with
  its `go.work.sum`. The embedded sidecar is built once with the local toolchain into the shared
  private binary cache and executed without a `GOTOOLCHAIN=local` pin, so its own `go list` honours
  the module's `go` line. The built binary is classified as a runtime artifact of the materialized
  source cache (it was being rebuilt in a fresh fallback copy on every run, ~6 s each).
- **Symbols sidecar failures are visible and never cached.** A failed load logs once at warn level
  with the sidecar's reason; a symbol graph carrying setup-missing support is recomputed on every
  run instead of being written to the layer cache.
- **Semantic sidecar load mode.** Dependencies from export data (no `NeedDeps`), SSA for the root
  packages only (`ssautil.Packages`; the whole-program load stays behind the test-only RTA oracle
  switch). Soft `GOMEMLIMIT` of a quarter of the memory available to polint unless the environment
  sets one. Test variants are opt-in for the semantic sidecar (an unset `include_tests` keeps tests
  in the symbols sidecar only); the effective value is in the sidecar cache key. The sidecar reports
  its peak RSS, printed in the stage log next to its heap figure. Every frontend source file is
  embedded, with a test that a new `.go` file cannot be left out.
- **Subprocess drain.** The bounded runner waited a fixed 10 ms after every empty non-blocking read,
  capping a row-by-row writer at 6.4 MB/s (the 35 MB semantic NDJSON took ~5.5 s to drain after the
  sidecar finished). It now waits on `poll(2)` readiness.
- **`polint unknowns` fails loudly.** Every stage in the requested capabilities' own closure that
  failed, is setup-missing or unsupported, or was skipped by the resource budget, and every language
  support reported setup-missing at run time, becomes a `<workspace>` row in the `provider_failed`
  category; the command exits 1.
- **Compact abstract domains by default.** Every plan gets the summary-input materialization; the
  per-point states only when a plan asks for them (no public capability can).
- **Layer read limit and grammar.** Layer payloads read up to 256 MiB; the vendored tree-sitter-go
  grammar accepts Go 1.26 `new(expr)` with unchanged trees for `new(T)`/`make(T, …)`.

## Gate 0 (OAIZ bench, 4 jobs, interleaved BASE = v0.4.4 vs FINAL, medians of clean samples)

| Line | Gate | BASE | FINAL | Verdict |
|---|---|---|---|---|
| catalog `control_flow` cold | ≤ 15 s, ≤ 2.5 GB tree | 27.0 s, ≤ 8.07 GB | 11.8 s, ≤ 2.21 GB | PASS |
| catalog `calls` warm | ≤ 8 s, ≤ 1 GB | 26.3 s, 6.10 GB | 5.1 s, ≤ 0.54 GB | PASS |
| `--cap references`, host Go older than the module | 0 `setup_missing` | 41 rows | 0 rows | PASS |
| reports for rules requesting no deep capability | byte-identical | 691 diagnostics | 649 | PASS: every difference is the `new(expr)` grammar fix (43 parse errors gone, 1 metrics diagnostic on a file that now parses) |

The semantic sidecar's rows (excluding phase/session rows) are identical and in the same order under
export-data loading on the catalog scope (64,996 rows). Raw samples and contamination accounting:
`/opt/data/polint-deep/gates.md` (outside the repository).

## Gate 1 kill-criterion probe

The whole `core` module (2,846 files) under the step-0 driver: `semantic_mir` + `cfg` alone hold
8.3 GB (6.6 GB of it stable-key text, because deep keys embed their parent key's full text) before
the call layer runs. Per the design doc's kill rule, step 1 ships only the sidecar change, its storage
and the `(file, span)` call-target join, and the step-2 unit split precedes any rule work.

## Commits

| Commit | Change |
|---|---|
| `1938bfc9` | fix(go): accept Go 1.26 new(expr) — vendor the grammar with the special-argument-list widening |
| `78173e2e` | perf(cache): read layer payloads up to 256 MiB |
| `8f6aac1c` | fix(go): load modules newer than the host toolchain in the symbol sidecar |
| `811faf40` | fix(symbols): never layer-cache a setup-missing symbol graph |
| `bfe582e3` | perf(subprocess): drain child output on readiness instead of a fixed poll |
| `35405d2e` | perf(go): load the semantic sidecar from export data, roots only, tests opt-in |
| `2cda6f10` | fix(cli): name what did not run in `polint unknowns` and exit 1 |
| `f4e00cc9` | perf(domains): give every deep capability the compact domain materialization |
| `73dc7e98` | test(go): neutral new(expr) fixtures, no debug formatting in their assertions |
| `6c594c86` | test: gate the unknowns and domain-materialization tests on the languages they run |

## Gates

Run sequentially on `73dc7e98`, then the language-gated steps again on `6c594c86`:

- fmt, clippy `-D warnings` (default, none, go, ts), rustdoc `-D warnings`: green (the first run of
  the none and ts clippy steps and of the none and go lib steps failed on three ungated tests and one
  ungated test constructor; `6c594c86` gates them and the re-run is green).
- Lib suite 2,772 passed (default), 2,075 (none), 2,157 (go), 2,273 (ts); every integration target run
  alone green (capability_matrix 4, consumer_api_compat 6, github_action_cache 26, golden 11,
  golden_corpus 3, internal_architecture 10, module_layering 1, public_surface_leak 8, rule_host_store
  2); doctests green; the cli suite in 48 batches of 4 green (one batch collided with a second runner
  and was re-run alone green); the other workspace crates and MSRV 1.95.0 green.
- Mutation checks: flipping the compact-domains default fails
  `deep_capabilities_get_compact_domain_facts_unless_per_point_states_are_requested`; disabling the
  setup-missing no-cache guard fails `setup_missing_symbol_graph_is_recomputed_instead_of_cached`.
