# Prepared commit messages

Git writes are blocked in this session: the index is outside the writable worktree, under `/Users/emilwareus/Development/polint-review/main/.git/worktrees/pr138-speed`. These are prepared messages, not completed commits.

## fix(evidence): stream indexed payloads one row at a time

Stable-key expansion can make an individual evidence payload large. Hash each ordered row before expanding the next, preserving ID/full-payload tie ordering and header merging. Remove the unused packed-buffer clearing helper.

Regression and mutation proof: indexed_family_streams_large_payloads fails with the original 4096-row batching: 571,408,384 bytes of additional peak RSS exceeds its 64 MiB bound. Restoring streaming passes. indexed_family_emission_matches_full_sort_across_batches_and_headers checks 8200 production-emitted rows against a full-sort reference.

## fix(analysis): build refinement lookup indexes only when needed

Share one lazy call-site index across completeness collectors. Build TS call-site lookup only when unresolved calls are read, and MIR/solver indexes only after an unresolved row matches a TS-family call site. Preserve the first matching fact for duplicate IDs. Add release microbenchmarks for empty and resolved refinement sets.

Regression and mutation proof: restoring original collectors makes completeness_skips_call_site_index_without_unknown_edges fail (5 builds vs 0) and aggregate_unknowns_include_policy_capabilities fail (5 vs 1). Restoring eager TS indexing makes empty_and_go_only_refinements_skip_expensive_indexes fail (1 expensive build vs 0). Fixed versions pass, and the populated unresolved TS regression preserves its edge and callable-place key. Intern fixture keys explicitly so the tests run independently.

## test(digest): check earliest errors across chunks in one batch

Put multiple failures in separate chunks of the first batch and exercise one, two and four worker pools.

Regression and mutation proof: reversing chunk-result selection returns Err(2999) instead of Err(127), failing packed_parts_report_the_first_failing_row. Restoring ordered chunk-result selection passes. The production algorithm needs no semantic change.

## docs(bench): record review-fix validation and local benchmark costs

Record the mutation evidence, final CI command results, and the local macOS benchmark. Every non-cost, non-version report field is identical to the committed PR-head baseline. Zod refined dump: 35.254 s, 5,645,402,112 bytes peak RSS; complete make js-bench: 163.368 s. These native macOS samples are distinct from the historical AX102 samples.

No analysis accuracy fields, assertions, or golden costs were relaxed. See SUMMARY.md for final full-suite status and the filesystem restriction that prevented creating commits in this session.
