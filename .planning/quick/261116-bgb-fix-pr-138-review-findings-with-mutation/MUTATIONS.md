# Mutation evidence

Commands used `cargo test -p polint --lib --all-features --locked <test filter>` with Rust 1.95, debug info disabled, and a writable temporary Cargo cache. Every mutation below compiled and ran its regression; none is a compilation-failure proxy. Production changes were restored after each mutation round.

| Finding | Regression | Removed behavior / mutation | Observed failure | Restored result |
| --- | --- | --- | --- | --- |
| 1 | `indexed_family_streams_large_payloads` | Restore 4096-row packed-payload retention after finalizing the regression fixture | Expanded payloads retained 571,408,384 bytes (544.94 MiB), exceeding the 64 MiB limit | Pass |
| 2 | `completeness_skips_call_site_index_without_unknown_edges` | Original eager completeness collectors | Five index builds; expected zero | Pass |
| 2 | `aggregate_unknowns_include_policy_capabilities` | Original per-collector index construction | Five index builds; expected one shared index | Pass |
| 2 | `empty_and_go_only_refinements_skip_expensive_indexes` | Original eager TS refinement implementation | One expensive index build; expected zero | Pass |
| 3 | `packed_parts_report_the_first_failing_row` | Reverse chunk results before choosing the first error | `Err(2999)`; expected `Err(127)` | Pass |

The strengthened first-error test puts failures at 127, 191, 2999, 5999 and 8999, and runs with one, two and four workers. The first three failures occur in different chunks of the first batch. Error selection is still ordered in production.

The evidence-emission equivalence regression exercises the production emitter with 8200 rows, repeated IDs, distinct tie payloads, Unicode/escaped text and headers before and after the family. Its output digest equals a reference that sorts all full payloads and headers together.

The completeness tests use populated call-site tables for both empty refined output and fully resolved target/edge facts. The TS tests cover empty input, a populated TS input with no unresolved rows, Go-only unresolved calls, and a real unresolved TS row that still returns its callable-place result. Two ignored release microbenchmarks cover 20,000-site empty and resolved workloads.

The failing mutation logs are preserved in `mutation-logs/` beside this document. Additional local logs: `/tmp/pr138-f1-final-mutant.log`, `/tmp/pr138-f1-fixed-final.log`, `/tmp/pr138-f2-{empty,sharing,ts}-mutant.log`, `/tmp/pr138-f2-fixed-2.log`, `/tmp/pr138-f2-sharing-fixed.log`, `/tmp/pr138-f2-ts-fixed-2.log`, `/tmp/pr138-f3-{mutant,fixed}.log`. Final full-suite results are recorded in SUMMARY.md.

The architecture gate also caught incidental debug formatting in the new test fixture and timing logs. These now use a named primitive row id and explicit nanosecond timing labels; every assertion and the dense-id baseline are unchanged. The memory mutation was repeated after that formatting cleanup and failed again at 571,408,384 bytes.
