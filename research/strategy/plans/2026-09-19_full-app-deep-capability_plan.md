# Implementation Plan: Full-Application Deep Capability (W0 to W9)

Date: 2026-09-19
Planner: Claude Fable 5.1 (delegated)
Input contract: [../04-full-app-deep-capability.md](../04-full-app-deep-capability.md) as revised after the adversarial review of round 1 (the commit that carries this plan revision). Every workstream, gate, and resolved question in that document is binding here; nothing is relitigated. Where this plan says "the research doc", it means that file at that commit.
Worktree: `/workspace/polint-research-fullapp`, branch `research/full-app-deep-capability`. Every `file:line` anchor below was re-read in this worktree at `07a338ce` before being written down; section 0 records the ones that drifted from the research doc.
Document type: implementation plan. Plan only. No code was changed for this document.

## 0. Provenance and anchor verification

The research doc was written against `686461be`; the two commits since then touched only research files, so code anchors were expected to be stable. Every load-bearing anchor was re-verified by grep in this worktree. Result:

| Anchor | Research doc | This tree | Status |
|---|---|---|---|
| `build_fingerprint` `cargo_home` line | `cache/rules_store.rs:797-806` | `:797-806` (`"cargo_home"` at `:798`) | unchanged |
| cargo config content hashing | `rules_store.rs:827-834` | `:827-834` | unchanged |
| `path_digest` | `rules_store.rs:1098-1110` | `:1098` | unchanged |
| `lower_go_mir` sequential loop, trailing `normalized` | `go/mir/lower.rs:39-41`, `:99` | `:39`, `:99` | unchanged |
| `lower_control_flow` per-body filters (Go) | `go/mir/lower.rs:115-124` | `:115`, `:118`, `:123` | unchanged |
| `matching_function` (Go) | `go/mir/lower.rs:2317-2329` | `:2317` | unchanged |
| `push_body` package and module scans | `go/mir/lower.rs:649-660` | `:621` (fn), scans inside; the package scan is `:649-653` and the module scan `:654-660` (round 4: `:645-656` truncated the module scan) | anchor widened |
| `go_closure_capture_names` trait-default reference scan | `go/mir/lower.rs:732-767`, `:742` | `:732`, `:742` | unchanged |
| `matching_function` (TS) | `ts/mir/lower.rs:4540-4553` | **`:4541`** | drifted by one line; corrected below |
| `matching_module_function`, `enclosing_function` (TS) | `:4568-4581` | `:4556`, `:4568` | unchanged |
| `lower_control_flow` per-body filters (TS) | `ts/mir/lower.rs:132-141` | `:132`, `:135` | unchanged |
| TS closure capture reference scan | `ts/mir/lower.rs:764` | `:764` | unchanged |
| `AnalysisHost` trait defaults | `analysis_neutral/host.rs:129-152` | `:129`, `:141` | unchanged |
| indexed `references_for_file`, `definition_for_symbol`, `definitions_for_symbol` | `core/db.rs:3746-3753`, `:3730` | `:3746`, `:3730`, `:3739` | unchanged |
| `impl AnalysisHost for AnalysisDb` | `core/db.rs:6043` | `:6043` | unchanged |
| `Icfg::from_facts` node scan | `ifds/mod.rs:93-98` | `:84` (fn), `:96` (find) | unchanged |
| refined-calls Go semantic join | `refined_calls/provider.rs:333-340`, `:361-454` | `:333`, `:361`, `:405`, `:426` | unchanged |
| `owner_symbol` | `calls/extract.rs:554-567` | `:554` | unchanged |
| sorted digest sites | `analysis/provider.rs:190`; `domains/provider.rs:224`; `types/provider.rs:300`; `calls/provider.rs:186`; `refined_calls/provider.rs:657`; `data_flow/provider.rs:466` | all six at the same lines | unchanged |
| streamed evidence digest and helpers | `evidence/provider.rs:570-649` | `:576` (fn), `:723` `family_parts`, `:736` `emit_family`, `:754` `indexed_family_parts`, `:913` property test | unchanged |
| domain key side tables | `domains/provider.rs:248-297` | `:245`, `:252`, `:266`, `:278` | unchanged |
| capability closure and seeds | `analysis_kernel/provider.rs:1013-1028`, `:1070-1090`, `:1094-1130` | `:1013`, `:1020`, `:1070`, `:1094`, `:1133` | unchanged |
| solver budget and worklist | `domains/solver.rs:84`, `:119`, `:134`, `:662`, `:700` | same | unchanged |
| summaries builder observation use | `summaries/builder.rs:75-81`, `:133-147`, `:347`, `:420-431` | `:75`, `:133`, `:147`, `:344` (fn), `:347`, `:422`, `:431` | unchanged |
| dominator relation | `cfg/derived.rs:339-400`, `:66-118`, `:203-257`, `:438-462` | `:339`, `:66`, `:203`, `:438`, `:464` | unchanged |
| dominance budget application | `cfg/provider.rs:109-116` | `:109`, `:112` | unchanged |
| interner | `internal_core/stable_key.rs:24-41`, `:62-73`, `:78-88`, `:106-112` | `:33` (state), `:62`, `:80`, `:104` | unchanged (two-line offset in fn starts, ranges still correct) |
| `FactMeta`, `FactMetaStore` | `analysis_api/metadata.rs:231-239`, `:353-357` | `:231`, `:238`, `:353` | unchanged |
| `write_stable_key_text` | `analysis_api/metadata.rs:512-523` | `:512`, `:525`, `:533` | unchanged |
| `semantic_stable_key` | `analysis_neutral/stable_key.rs` | `:16`; 34 non-definition call sites | research doc said 35; this tree counts 34 (`grep -c`), difference is one test site |
| layer cache API and ceilings | `layer_cache.rs:218`, `:385`, `:31-32` | `write_json` `:374`, `write_json_bytes` `:396`, `read_json_bytes_validated` `:233`, ceilings `:31-32` | unchanged |
| `LayerKey` | `incremental/keys.rs:51-62` | `:51` | unchanged |
| `ResourceEnvelope` | `resource.rs` | `from_env` `:69`, `observe` `:97`, `exhausted` `:121`, rule id `:130` | unchanged |
| kernel sequential loop and gauge | `analysis_kernel/mod.rs:260`, `:327-341` | `:260`, `:327-341` | unchanged |
| provider dispatch and `Provider::run` | `analysis_kernel/provider.rs:991-1005` | `:991-1005`; `SemanticMirProvider` `:281`; `AbstractDomainsProvider` `:441`, run gate `:450-458` | new anchors for this plan |
| determinism gate | `polint-eval/src/harness/determinism_gate.rs` | fixtures at `tests/eval-fixtures/determinism/{go_reachable,go_rta,ts_object_model,ts_reachable,ts_tokens}` | new anchor |
| policy-query dominance consumer | not in research doc | `policy_queries.rs:1383-1418` builds an edge relation and answers by reachability over it | new anchor; confirms W4's tree-only emission is already consumed as a graph |
| `ProviderCtx::dependency_digest` | research doc section 6 (revised) | `analysis_kernel/provider.rs:69-74`; returns `Digest::absent(ProviderOutput, id)` for a provider with no recorded output | the reason provider digests are cache keys, not fact digests (round 1 finding 1) |
| digest recipes that fold upstream digests | research doc Appendix A "digest recipes" | `summaries/provider.rs:39-42`; `types/provider.rs:196`, `:218-222`; `refined_calls/provider.rs:611-616`, `:627-632`; `entrypoints/provider.rs:92-94`; `domains/provider.rs:158-160`; `calls/provider.rs:115-116`; `data_flow/provider.rs:417-422`; `evidence/provider.rs:611-616`; call sites `analysis_kernel/provider.rs:428`, `:463-470`, `:513-521`, `:550-555`, `:585-592`, `:620-624`, `:683-694`, `:724-735`, `:761-763`, `:789-794`, `:819-826`, `:851-859` | verified per provider; drives every "expected digest move" list below |
| closure parity assert and test | research doc section 8 W3 (revised) | `analysis_kernel/mod.rs:603-607` (`debug_assert_eq!` against `providers_enabled_by_boolean_gates`); `provider.rs:1031-1068` (the boolean gates); `provider.rs:1999-2025` (`capability_closure_matches_boolean_pipeline_gates`, 128 subsets); `provider.rs:2027-2052` (`v13_cache_dependency_ledger_matches_provider_manifest_inputs`) | W3 must name and replace these |
| `domain_events` input rows, and the summary rows that re-pull them | research doc section 8 W3 (revised) | `provider.rs:1583-1584` (`direct_summaries`), `:1688-1689` (`type_value_alias` domain rows), `:1690-1693` (`type_value_alias` summary rows, all unread under `analysis_neutral/types/`); sole domain producer `:1561` | all six rows leave `type_value_alias` (round 1 finding 2; round 2 finding 1) |
| post-dominance universe | research doc section 8 W4 (revised) | `cfg/derived.rs:82-83` (forward, entry-reachable), `:133-154` (reverse, all blocks plus virtual exit), `:309-321` (`reachable_blocks`), `:346-356` (universe-seeded fixpoint), `:404-425` (`collect_reversed_predecessors`), `:438-462` (`immediate_relation`), `:493-509` (`selected_exit_blocks`) | W4's tree must take the universe as a parameter |
| layering rule for `analysis_neutral` | not in research doc | `crates/polint/tests/module_layering.rs:71-75` forbids `go`, `ts`, `frontend`, `cli`; no rule constrains `analysis_kernel`; `frontend_api` forbids `analysis_kernel`, `go`, `ts`, `cli` (`:66-70`) | W6's unit discovery must enter through `frontend_api` |
| TS project discovery | research doc section 8 W6 | `ts/module_graph/mod.rs:1591` `nearest_tsconfig_path` (private; `tsconfig.json` only); `:281-292` `find_ts_project_root` (matches `tsconfig.json` or `package.json`; feeds module nodes at `:264`); TS `TopologyPackageFact` rows are `JsPackage` per `package.json` (`:352-372`); TS `SourceSetFact` rows are one per file (`:414-432`) | the plan's earlier anchor `:286` named the wrong function |
| tree-RSS sampler | research doc section 7 (revised) | `.scale-envelope/rssrun.py:22-58` (tree walk via `/proc/<pid>/task/<pid>/children`, `VmRSS` sum), `:75-107` (JSON summary with `peak_rss_bytes`, `wall_s`, `RLIMIT_AS` guard) | replaces `/usr/bin/time -v` for memory |
| dependency blocking | not in research doc | `analysis_kernel/outcome.rs:311-357` (`from_manifests`, `hard_dependencies` at `:325-341`, `PlannedAbsent` at `:344-354`), `:361-377` (`can_run`), `:536-545` (`is_usable`), `:639-687` (the hand-written table), `:806-846` (its two guard tests); `mod.rs:179-210` (`skipped_direct_summaries_result`), `:777-840` (`runtime_capability_blockers`), `:859` (`calls` maps to `polint.refined_calls`) | round 3: W3 commit 2 must filter hard dependencies to the selected set or it blocks the whole `calls` stack |
| `ControlFlow` dominance adapter regimes | not in research doc | `policy_queries.rs:1370-1381` (`BlockRelation::new`), `:1406-1417` (`answer_for_blocks`), `:1420-1433` (`reaches`, breadth-first over emitted rows); vacuous rows under `Full` `cfg/derived.rs:156-175`, under `ImmediateOnly` the `immediate_relation` pick `:438-462` | round 3: W8's adapter must reproduce the materialisation-dependent answer for vacuous event blocks |
| plaintext summary payload | not in research doc | `summaries/facts.rs:164` (`SummaryFact::payload_digest: String`), `builder.rs:156` (`control.stable_digest_parts().join(";")`), `core.rs:110-124` (parts), `core/db.rs:2138-2141` (metadata run id is `SummaryId`), `core/metadata.rs:392-409` and `:430-438` (the metadata digest is a 16-hex FNV) | round 3: the G1c parts column that makes the `summary_control` allowlist evaluable |
| polint-process peak | research doc section 7 (revised) | `measure.rs:27-32` (`getrusage(RUSAGE_SELF).ru_maxrss`), reported as `peak_rss_mb` on stage rows (`analysis_kernel/mod.rs:334`) | the figure G2 binds |
| test-only fact dump | research doc section 7 G1c | `analysis_kernel/debug.rs:29-68` `metadata_debug_json_for_test` (per-family rows with resolved stable keys) | the seed of the fact-row oracle |
| `CallSiteOrderKey` | not in research doc | `mir_body_compose.rs:180-193` sorts resolved key text to assign `CallSiteId` | must be in W5's conversion list (round 1 note 15) |
| `merge_language_outputs` normalisation | not in research doc | `mir_body_compose.rs:28` re-normalises (`:27` is `remap_call_site_ids`); per-language offsets are table lengths (`:32-36`, `:43`) except `operation_offset`, which is `max(existing id) + 1` (`:37-42`); all are order-independent | the actual reason W1 may drop the lowerer's trailing `normalized` (round 1 note 16) |

Three things in this tree that the research doc did not record and that change the plan's shape:

1. `AbstractDomainsProvider::run` already selects a compact "summary inputs" materialisation when `control_flow` is requested without `calls` or `dataflow` (`analysis_kernel/provider.rs:450-458`). W3 therefore does not introduce the concept of a reduced domains run; it moves the decision from a capability string check inside one provider into the family closure and makes the full run per-function.
2. The policy-query guard checks (`policy_queries.rs:1383-1418`) consume `cfg_dominators` as a directed relation and answer by walking it. Tree edges are a valid relation for that walk, which is why the bound has been byte-identical on `polint check` (`.scale-envelope/EXPERIMENTS.md` X5b). W4 can therefore make "tree edges always" the only materialisation without a consumer change, and keep the closure emission solely for the `POLINT_CFG_MAX_DOMINANCE_PAIRS=0` digest-identity path.
3. `polint unknowns` never reads `abstract_domain_events`: the collector (`analysis/unknown_taxonomy/collect.rs`) builds budget rows from the kernel's resource diagnostic (`:242-258`), the points-to and RTA solver's `solver_budget_status` (`:295-306`), call targets, data-flow budgets and evidence unknowns, and nothing in it names a domain event (`grep -n 'DomainEvent\|abstract_domain_events' collect.rs` is empty). A domain-solver budget row therefore does not surface today, and W3's "reports which functions were cut" needs a collector row of its own (W3, `analysis/unknown_taxonomy/collect.rs` bullet) before any probe can grep for it.

## 1. Goal, invariants, and conventions

### 1.1 Goal

After W0 to W9: a forced `calls` scan of the full 4,752-file Go backend (the "full backend" below; the consumer repository is never named in any committed artifact) completes in under 300 s at 12 threads with peak tree RSS under 12 GB, with the diagnostics oracle and ai-friendly stdout (section 5.1) byte-identical across cold, warm and permuted provider order; the same for the 2,381-file TS frontend; a one-file change re-lowers only its unit and dependents; and a fresh-container rule-host store miss no longer costs a 193 s compile. The research doc's section 7 is the gate list; section 5 here restates it as a checklist.

### 1.2 Invariants carried into every workstream

- **I1, the identity oracle, in two tiers.** The protected quantity is the fact rows: for every fact family, the sorted list of (canonical stable-key text, payload digest), plus the `polint check` diagnostics digest and the ai-friendly stdout. A provider's `digest=` is a cache key: it is computed from the provider's rows and from the output digests of the upstream providers its recipe names, fetched through `ProviderCtx::dependency_digest` (`analysis_kernel/provider.rs:69-74`), which returns a fixed `absent` value for a provider that did not run (section 0, "digest recipes"). So:
  - **I1a, provider digests.** For a workstream that changes neither the set of registered providers nor the set that runs for a given request (W0, W1, W2, W4, W5, W7 on a warm run, W8 on a cold run), every `digest=` in the stage rows is byte-identical before and after. Tool: `.scale-envelope/digests.py <before.stderr> <after.stderr>` prints `N/N provider output digests identical` with N the count of providers that had a row in the before capture, per the provider-count rule in section 5.1. Any `DIFFER` or `MISSING` row is a failing build. `digests.py` reports only providers present in the before capture and treats a failed provider's `digest="-"` as a comparable value, so the gate additionally checks the after capture's provider set and rejects any `digest="-"` (W9, `gate.sh`). I1a is sufficient for these workstreams because a moved row moves its own provider's digest.
  - **I1b, fact rows.** For a workstream that changes which providers run (W3) or which providers exist (W6), the digests of every provider whose recipe names a changed provider move by construction, and I1a cannot be the oracle. The oracle is the fact-row dump: `polint-eval`'s harness gains `fact_rows_dump` (slot 1, with the W9 scripts), which writes one file per fact family containing the sorted (canonical key text, `payload_digest` hex) pairs of every metadata row, built from the same per-family walk `metadata_debug_json_for_test` does today (`analysis_kernel/debug.rs:29-68`) but over `FactMetaStore::family_rows` for every family; it is run with the before and after binaries on the same checkout and diffed byte for byte. The workstream's test plan lists exactly which families may differ (W3 on a `calls` request: `domain_observations` and `domain_events` absent, and `summary_control` rows whose payload digest moves because their `DoesNotReturn` exit kind is derived from the absent observations, with the same key set and a counted, reported delta; W6: none) and exactly which provider digests are expected to move; any other family or digest moving is a failing build. The `FactMeta::payload_digest` column is a sixteen-character FNV hex over the fact's parts (`core/metadata.rs:392-409`, `:430-438`), so it can show that a row moved but not why; for the four `SummaryFact` families the dump therefore adds a third column, the plaintext `SummaryFact::payload_digest` (`summaries/facts.rs:164`; for `summary_control` it is `control.stable_digest_parts().join(";")`, `builder.rs:156`, i.e. the sorted `exit:*` parts followed by `async:*` and `cleanup:*`, `core.rs:110-124`), joined to the metadata row through its run id, which is the `SummaryId` (`core/db.rs:2138-2141`) into `db.summary_facts()`. With that column the W3 allowlist is evaluable line by line. The I1b line rule, stated here once and referred to by name everywhere else: a differing `summary_control` line is permitted iff its before parts contain `exit:DoesNotReturn` and its after parts take one of three shapes, each matching a branch of `build_control_effects`: (1) simple removal, the before parts with `exit:DoesNotReturn` removed and nothing else changed (`summaries/builder.rs:430-432` no longer inserts it); (2) set-emptied replacement, the before parts with `exit:DoesNotReturn` replaced by `exit:Returns`, which the builder inserts when the removal would leave no exit kind and the body has operations (`:435-437`); (3) bottom, the after parts are the single literal `control=bottom` (`core.rs:112`), which the builder returns when the exit set is empty, the body has no operations and no unresolved call adds `exit:Unknown` (`:440-442`, `:445-446`). The `async:*` and `cleanup:*` parts never change, because neither reads observations. Any other difference in the parts column is a failing build. (Round 3: the earlier wording asked the operator to read the exit kind from the hex digest, which is not possible. Round 4: the earlier rule permitted only shape (1), which rejects the set-emptied replacement the builder actually performs and which test (e) constructs; shapes (2) and (3) were added and the five other sites that restated the rule now point here.) The dump covers every `FactFamily` through a new `FactFamily::ALL` constant with a test that its length equals the variant count (`analysis_api/metadata.rs:6-108` has no iterator today); families that are aggregated into one metadata row (`SemanticGraph`, one row over the sorted join of every node, edge and constraint, `core/db.rs:5892-6039`) or that record no metadata row at all (`call_reachability`, `solver_budget_status`, `solver_budget_reasons`) are change-detected only, and their per-row identity is covered by the provider digests of tier I1a on the workstreams where those providers are not expected to move. The dump is produced by an ignored test entry in the `polint-eval` harness (W9), because that harness is compiled only under `cfg(test)` (`crates/polint/src/lib.rs:32-34`); "before" and "after" are two runs of that entry from two checkouts, not two binaries handed to a script. The diagnostics digest and stdout must be identical in both tiers.
  - The "must not move" column of the research doc's stage-row table (section 8) is copied into each workstream's test plan below, and each workstream states its tier.
- **I2, frozen public surface.** `crates/polint/src/sdk/facts.rs`, `crates/polint/src/sdk/policy.rs`, the `evidence_v1` envelope, `docs/facts/`, every `docs/schemas/*.json`, and the ai-friendly report shape do not change. `crates/polint/tests/public_surface_leak.rs` and `consumer_api_compat.rs` gate this and must stay green in every commit. (Resolved Q2.)
- **I3, internal traits change freely.** `AnalysisHost`, `AnalysisDb`, `FactDatabase`, `FactMetaStore`, the provider manifests, and every `analysis_*` module are `pub(crate)` and change without a decision record (Resolved Q2). The store schema and the layer-cache manifest schema are one-way doors and carry a schema-label bump when they change (W7).
- **I4, no dual paths.** The PR that lands a replacement deletes the path it replaces (report 03 rule 2). This plan names the deletion in each workstream's commit shape.
- **I5, determinism.** `cargo test -p polint --lib eval::determinism_gate --locked` (the N=10 seeded-permutation gate, `.github/workflows/ci.yml:247-248`) stays green in every commit; W6 extends its fixture set.
- **I6, hygiene.** No consumer source, diagnostic text, or repository name enters this repository or its history; committed measurements carry counts, timings, rule ids and scanned-file basenames only.
- **I7, no CI for the gate.** The acceptance gate is local-only, forever (Resolved Q6). No hosted job, no self-hosted runner, no schedule, no `workflow_dispatch`. The existing CI jobs (fmt, clippy, tests, determinism gate, leak gate) continue to run on every PR as today; nothing here adds to them except unit tests.

### 1.3 Conventions

- Delivery rules are report 03 section 3: a PR changes structure or behaviour, never both; at most 1,500 changed lines and 25 files; one storage invariant per PR; no expectation edits to pass a test.
- "Probe" means the shell function in section 5.1; every probe run stores `stdout`, `stderr` (which carries the stage rows and the `rssrun.py` JSON summary line) and the sampler's `timeline.json` under a local, uncommitted directory. No probe invokes `/usr/bin/time`. The committed artifact is the report format in section 6.
- Hash maps are lookup-only; output order never follows hash iteration (`research/deep-analysis-performance/FINAL-REPORT.md`, retained mechanisms).
- "885-file scope" and "45-file scope" are the two consumer sub-scopes the benchmark report measured; they are identified in the local report by file count only.

## 2. Sequencing

Execution order, honouring the research doc's dependency graph:

```
W0                                    independent; ship first
W1 || W2 || W4                        three parallel branches, identity-preserving
W3                                    after W1 (per-function solver measured fairly); closure half can start with W1
W5                                    after W1 and W2 land on main
W6                                    after W5 and W3
W7                                    after W6
W8                                    after W7
W9                                    envelope half after W6; probe-script and report-format half any time after W0
```

Box-capacity rules on the 16-core, 30 GB host:

- At most one release build at a time (`CARGO_BUILD_JOBS=4` while a probe or another build runs). A cold `cargo build --release -p polint` and a full-backend probe together exceed the box's comfortable memory when the probe is on the pre-W5 pipeline (14 to 18 GB tree peak); probes on the full backend are serialised behind every build.
- W1 and W4 can be developed in parallel worktrees because they touch disjoint files (W1: lowerers, `ifds`, `refined_calls/provider.rs:300-454`, `calls/extract.rs`, `core/db.rs` overrides; W4: `cfg/derived.rs` and `cfg/provider.rs`). W2 is not file-disjoint from W1: one of its six digest functions is `refined_calls_output_digest` (`refined_calls/provider.rs:596-665`, `parts.sort()` at `:657`), in the same file W1 edits at `:333-454`. The two regions do not overlap, so W2's commit 1 (the pure extraction from the evidence provider) may proceed in parallel, but W2's commit 2 (the six sites) is written on top of W1's merged commit 4 and its oracle run is taken against that base. Probes must not overlap on the 885-file or full-backend scopes; the 45-file scope and excalidraw can run concurrently with a build.
- Baseline ownership, stated once here and referred to everywhere else as "the section 2 baseline rule": every oracle comparison (G1 stderr, G1c fact-row dump, probe stdout) is against a capture taken from the tree state the workstream's slot starts from. W0, W1, W2 and W4 compare against the branch base (captured in slot 1; they are identity-preserving, so one capture serves all four). W3 compares against the branch base and is the workstream that permanently moves the `calls`-cell digest set. After W3 merges, slot 4 re-captures every cell (stderr, stdout and fact-row dumps) from post-W3 main, and W5 compares against that capture, never against the branch base (round 5: W5 had no baseline sentence and would have failed G1 with W3's one `MISSING` and five `DIFFER`). W6 compares against post-W5 main under I1b; after its commit 4 the new digests are the baseline for its commits 5 to 7 and for W7. W7 and W8 compare against the main they start from. A workstream that finds its baseline stale re-captures before it measures and says so in its PR.
- W5 and W6 are serial with everything else: both touch `internal_core`, `analysis_api/metadata.rs` and every provider, so no other branch can be merged underneath them without a rebase.

## 3. Workstreams

### W0. Rule-host store key independent of the cargo home path

**Goal.** After W0, two machines (or two containers) whose cargo home paths differ but whose cargo config contents match restore the same compiled rule host from the machine-global store, so a fresh-container run pays the 5 s restore instead of the 193 s compile (benchmark report section 4.2, experiment E1). Nothing else changes.

**Concrete changes.**

- `crates/polint/src/cache/rules_store.rs`
  - Delete the `digest.line("cargo_home", ...)` block at `:797-806`. The cargo config files under the cargo home are already discovered by `cargo_config_files(rule_pkg_dir, repo_root, environment.cargo_home.clone())` and hashed by content at `:827-834`, so a cargo home whose `config.toml` differs already produces a different key.
  - Keep `path_digest` (`:1098`) if any other caller remains; if `cargo_home` was its only caller, delete it and its `#[cfg(windows)]` branch.
  - Keep `shareable_with_cargo_home` (`:1190`) unchanged: it decides shareability from config contents, not the path.
- Tests in the same file:
  - Rewrite `cargo_configs_and_cargo_home_are_part_of_the_key` (`:1733-1767`): keep the two `assert_ne!` for the ancestor config and the cargo-home config contents; replace the final assertion (`:1762-1766`, "Cargo home location affects relative config and registry paths") with an `assert_eq!` that a second, empty cargo home at a different path yields the same fingerprint as the first empty cargo home. Rename to `cargo_configs_are_part_of_the_key_and_cargo_home_path_is_not`.
  - Add: same rule package, cargo home A with `config.toml` X, cargo home B at another path with identical `config.toml` X, fingerprints equal.
  - `a_fingerprint_is_stable_and_changes_with_every_input_it_names` (`:1575`) must still pass; it does not name `cargo_home`.
- `crates/polint/tests/rule_host_store.rs`: extend `a_rule_host_compiled_once_is_shared_with_every_other_checkout` (`:160`) or add a sibling that publishes from one temporary `CARGO_HOME` and restores from a fresh, empty `CARGO_HOME` at a different path, asserting no `cargo` child was spawned (the test already has `cargo_that_refuses_to_compile` at `:118` for exactly this assertion).
- `docs/CACHE.md`: one sentence stating that the store key hashes cargo configuration by content and not by location.

**Interfaces.** None; `build_fingerprint(root, package, environment) -> Option<String>` keeps its signature.

**Test strategy.**
- Gating: `cargo test -p polint --lib cache::rules_store --locked`; `cargo test -p polint --test rule_host_store --locked`.
- Invariant I1 is not applicable (no analysis provider runs); the rule-host binary produced is byte-identical because the compile inputs are unchanged.
- Invariant: every other line of the fingerprint (driver version, profile, os, arch, rustc, cargo, toolchain, flag env vars, manifests, lockfiles, rule sources, cargo configs) is still present; the existing `a_fingerprint_is_stable_and_changes_with_every_input_it_names` test asserts this.

**Verification probe (G0).**

```sh
# publish once from cargo home A, then restore from a brand-new empty cargo home B
A=$(mktemp -d); B=$(mktemp -d)
CARGO_HOME=$A polint check --profile core --fail-on none <paths>      # may compile (store miss) or restore
CARGO_HOME=$B python3 .scale-envelope/rssrun.py --label g0 -- polint check --profile core --fail-on none <paths> 2> "$POLINT_GATE_OUT/g0.stderr"
grep -oE '"wall_s": *[0-9.]+' "$POLINT_GATE_OUT/g0.stderr"      # wall clock only; G0 has no memory threshold
# expected: elapsed under 10 s; ps tree during the run shows polint and the rule host only
```

Stage rows: none must move (analysis is unchanged).

**Dependency and risk.** Depends on nothing. Top failure mode: a cargo config discovered through the cargo home path but outside the files `cargo_config_files` enumerates (for example a registry mirror configured only via `CARGO_HOME/config` in a legacy location). Detection: the rewritten test writes the config under both `config.toml` and `config` names and asserts the key changes for each.

**Commit shape.** One commit: the deletion, the test rewrite, the integration test, the doc sentence. Title `fix(cache): stop hashing the cargo home path into the rule-host store key`.

### W1. Index every join (identity-preserving)

**Goal.** After W1, no lowerer or consumer stage scans a whole-program `Vec` inside a loop over another whole-program collection. `polint.semantic_mir` on the full backend completes in bounded time (the research doc's cause 3.2 removed); every provider digest is byte-identical to before. This is the first change that lets G3 be measured at all.

**Concrete changes.**

`crates/polint/src/core/db.rs`
- In `impl crate::analysis_neutral::AnalysisHost for AnalysisDb` (`:6043`) add overrides that route the trait's `references_for_file` and `definition_for_symbol` to the inherent indexed methods (`:3746`, `:3730`), and add a new trait method `definitions_for_symbol` routed to `:3739`. The trait defaults in `analysis_neutral/host.rs:129-152` stay for `LocalAnalysisDb` and the two `LocalFactDb` test databases.

`crates/polint/src/analysis_neutral/host.rs`
- Add to `AnalysisHost`: `fn definitions_for_symbol(&self, symbol: SymbolId) -> Box<dyn Iterator<Item = &DefinitionFact> + '_>` with the filtering default; keep `references_for_file` returning `Vec<&ReferenceFact>` (call sites collect anyway).

`crates/polint/src/go/mir/lower.rs`
- New private struct built once at the top of `lower_go_mir` (`:28`) and passed by reference into `lower_file` (`:494`) and `push_body` (`:621`):

```rust
struct LoweringIndex<'db> {
    functions_by_file_name: HashMap<(FileId, Language, &'db str), Vec<&'db FunctionFact>>, // insertion order preserved; Language is part of the key because both matching_function predicates test it (go/mir/lower.rs:2323-2327, ts/mir/lower.rs:4548-4552)
    package_by_file: HashMap<FileId, PackageId>,
    module_node_by_file: HashMap<FileId, ModuleNodeId>,
}
```
  - `matching_function` (`:2317`) becomes a lookup in `functions_by_file_name` keyed by `(file, Language::Go, name)` followed by the same `span_contains` filter over the bucket, returning the first match in bucket order (which is `db.functions()` order, so first-match semantics are preserved); the TS lookup keys by its `language` parameter. Keeping the language in the key is what makes the index equivalent to the scan even if one file ever carries `FunctionFact`s of two languages.
  - `push_body` (`:621`): replace the two `.iter().find(...)` scans with `package_by_file.get(&file.id)` and `module_node_by_file.get(&file.id)`. The old scans picked the first package and first module node for the file in database order; the index is built by iterating in database order and inserting only when absent (`entry().or_insert`), which reproduces "first".
  - `go_closure_capture_names` (`:732`): unchanged code, now reaching the indexed `references_for_file` through the override; `definition_for_symbol` likewise.
  - `lower_control_flow` (`:102`): replace the two per-body `filter` scans (`:118`, `:123`) with one pre-pass that groups `operations` and `control_effects` by `MirBodyId` into `BTreeMap<MirBodyId, Vec<&T>>` (operations are pushed in body order, so a `Vec<Range<usize>>` is also possible; the map is simpler and the order within a body is the push order either way). The `body_operations` and `body_effects` locals keep their types so the rest of the function is untouched.
  - Delete the trailing `.normalized(interner)` at `:99`; `SemanticStore::from_output` normalises at `analysis_neutral/store.rs:38`. Digest identity does not follow from the digest sorting its parts (`analysis/provider.rs:190`), because the parts embed raw dense ids (`file={:?} function={:?}` at `:116`, `statements={:?} terminator={:?}` at `:128`, and the operation-kind fragments at `:246-286`), and id assignment is order-sensitive. It follows from the merge: `merge_language_outputs` re-normalises the merged output (`mir_body_compose.rs:28`), its per-language id offsets are order-independent (table lengths at `:32-36` and `:43`; `operation_offset` is `max(existing id) + 1` at `:37-42`, which is also independent of input order), and `remap_call_site_ids` sorts by an explicit `CallSiteOrderKey` before assigning ids (`:193`). Verify with I1a before merging; if any digest moves, keep the call and record why.

`crates/polint/src/ts/mir/lower.rs`
- Same `LoweringIndex` (shared as `analysis_neutral::lowering_index::LoweringIndex` so both lowerers use one definition), applied to `matching_function` (`:4541`), `matching_module_function` (`:4556`, becomes a per-file `Option<&FunctionFact>` computed once), `enclosing_function` (`:4568`, per-file `Vec<&FunctionFact>` sorted by span start, then linear scan of that file's functions for the smallest containing; a binary search is not needed for correctness and per-file counts are small), the per-body filters at `:132-141`, the closure capture path at `:764`, and the trailing `normalized` at `:116`.

`crates/polint/src/analysis_neutral/ifds/mod.rs`
- `Icfg::from_facts` (`:84`): build `HashMap<MirOpId, CfgNodeId>` from `cfg_nodes` once, replace the per-call-site `find` at `:96`. Both callers (`domains/solver.rs:119`, `ifds/mod.rs:221`) benefit.

`crates/polint/src/analysis_neutral/refined_calls/provider.rs`
- In the index-building block that ends with the loop at `:333`, build once:
  - `call_sites_by_file_span: HashMap<(FileId, Language, u32, u32), Vec<&CallSiteFact>>` from `db.call_sites()` in database order (the legacy filter tests `site.language == Language::Go`, `:372`);
  - `functions_by_file_name: HashMap<(FileId, Language, &str), Vec<&FunctionFact>>` from `db.functions()` in database order (the legacy filter tests `core.language == Language::Go`, `:435`);
  - `go_functions_by_qualified: HashMap<&str, Vec<&GoSemanticFunctionInput>>` from `go_semantic_functions` in input order.
  - `core_callsite_for_go_semantic_callsite` (`:361`) takes the indexes and keeps its candidate-narrowing steps (caller match, dynamic-status preference, `min_by_key` on resolved key) exactly; `core_function_for_go_semantic_function` (`:405`) and `matching_core_function_for_go_semantic_span` (`:426`) likewise. The final `min_by_key(|site| db.resolve_stable_key(site.stable_key))` stays so ties resolve identically.

`crates/polint/src/analysis_neutral/calls/extract.rs`
- `owner_symbol` (`:554`): a `HashMap<(FileId, &str, Span), SymbolId>` built once per `extract_call_sites` (`:16`) call from `db.symbols()`, first-wins insertion.

**Interfaces.** `LoweringIndex<'db>` is `pub(crate)` in `analysis_neutral`, constructed by `LoweringIndex::build(db: &'db impl AnalysisHost) -> Self`. No trait signature changes except the added `definitions_for_symbol`.

**Test strategy.**
- Gating unit tests: `go::mir::lower` and `ts::mir::lower` test modules (existing lowering fixtures at `go/mir/lower.rs:2612+`, `:2868+`); `analysis_neutral::calls::extract` tests including `extract_call_sites_is_deterministic_for_different_operation_orders` (`:966`); `analysis_neutral::refined_calls` tests; `analysis_neutral::ifds` tests; `analysis_neutral::domains::solver::deterministic_shuffled_rows_produce_byte_identical_result_digests` (`:794`).
- New unit tests: for each replaced scan, a fixture where the bucket has two candidates and the old first-match rule and the new lookup agree (two functions with the same name in one file at different spans; two symbols with the same name in one file; two call sites with the same span from different callers).
- Invariant I1a: `digests.py` reports N/N identical (N per the provider-count rule, section 5.1); the diagnostics oracle (section 5.1) passes.
- Must not move: every `digest=`; `facts`; `keys`; `key_mb` on every stage row.
- Full suite: `cargo test -p polint --lib --all-features --locked` with the scale corpus moved aside (`.scale-envelope/EXPERIMENTS.md`, "Note on running the suite locally").

**Verification probe (G1, G1b, G2).**

```sh
probe s885-before calls <885-file scope>     # W1's baseline is the branch base (section 2 baseline rule)
probe s885-after  calls <885-file scope>
python3 .scale-envelope/digests.py "$POLINT_GATE_OUT/s885-before.stderr" "$POLINT_GATE_OUT/s885-after.stderr"   # N/N identical (provider-count rule, section 5.1)
python3 .scale-envelope/stages.py "$POLINT_GATE_OUT/s885-after.stderr"                              # semantic_mir, refined_calls, abstract_domains rows drop
```

G1b (cost split): temporary `tracing::debug!(target: "polint::probe", step = ...)` rows around `lower_file`, `finish_with_types`, `lower_control_flow` and `normalized` inside `lower_go_mir`, on the 885 and 1,588-file scopes (the latter with `POLINT_GATE_TIMEOUT=900`, since it measures 300.6 s today), removed before merge. Expected: the four superlinear terms of the research doc's section 3.2 table (`lower_control_flow`, `matching_function`, `push_body`, closure captures) together account for most of the stage before W1 and are not visible after; their relative order is recorded and section 3.8 of the research doc is corrected if it disagrees.

Expected movement (research doc section 8): `polint.semantic_mir` `elapsed_ms` large drop; `polint.refined_calls` `elapsed_ms` large drop; `polint.abstract_domains` `elapsed_ms` drop via `Icfg::build`. On the full backend `semantic_mir` completes (G3's first attempt; the 60 s threshold is not expected until W6).

**Dependency and risk.** Depends on nothing; parallel with W2 and W4. Top failure mode: a replaced scan had first-match semantics that the index does not reproduce (the X6 lesson: a fast run that dropped work). Detection: the digest oracle on all three scopes plus the two-candidate unit tests; any `DIFFER` row is a stop.

**Commit shape.** Four commits, each independently green and oracle-checked:
1. `perf(db): route AnalysisHost reference and definition lookups to the indexed AnalysisDb methods` (host.rs, core/db.rs).
2. `perf(mir): index function, package and module lookups in the Go and TS lowerers; group control-flow inputs by body` (both lowerers, `LoweringIndex`, the trailing `normalized` deletions).
3. `perf(ifds): index CFG nodes by operation when building the ICFG`.
4. `perf(calls): index the Go semantic call-site join and the owner-symbol lookup` (refined_calls, calls/extract).

### W2. Stream every provider digest

**Goal.** After W2, no provider builds a `Vec<String>` of one formatted row per fact to sort before hashing; the transient peak above retained RSS on `semantic_mir`, `abstract_domains`, `type_value_alias`, `calls`, `refined_calls` and `data_flow` shrinks to the retained size; every digest is byte-identical.

**Concrete changes.**

- Extract the evidence provider's streaming machinery into `crates/polint/src/analysis_neutral/digest_stream.rs` (new, `pub(crate)`): `family_parts` (`evidence/provider.rs:723`), `emit_family` (`:736`), `indexed_family_parts` (`:754`), and a generic `emit_sorted_digest(kind, label, header: Vec<String>, families: &[(prefix, iterator)])` that reproduces `parts.sort()` order by the family-prefix argument documented at `evidence/provider.rs:560-575`. Move `family_prefixes_partition_the_sorted_order` (`:913`) into the new module as a generic property test parameterised by a provider's prefix list and header labels.
- Apply it to the six sites, each with its own `FAMILY_PREFIXES` constant asserted by the property test:
  - `analysis/provider.rs:190` (`semantic_mir_output_digest`): families `body=`, `block=`, `statement=`, `terminator=`, `place=`, `operation=`, `unsupported=` (confirm the exact labels from `:112-185` when editing; the prefix list must match the format strings byte for byte).
  - `analysis_neutral/domains/provider.rs:224`: families `observation=`, `event=`; and delete the four side tables `body_stable_key_map`, `block_stable_key_map`, `operation_stable_key_map`, `place_stable_key_map` (`:245-297`) in favour of resolving through the interner at emission (`interner.resolve(id)` per row; `Arc<str>` clone, no owned copy).
  - `analysis_neutral/types/provider.rs:300`; `calls/provider.rs:186`; `refined_calls/provider.rs:657`; `data_flow/provider.rs:466`: same pattern; each file's family labels are read from its own `format!` strings.
- For families whose rows start with a decimal id, use `indexed_family_parts` so only one row is materialised at a time; for the others, `family_parts` sorts the family alone, which is still one family live at a time instead of all.

**Interfaces.** `pub(crate) fn emit_sorted_digest(kind: DigestKind, label: &'static str, header: Vec<String>, families: Vec<(&'static str, Box<dyn Iterator<Item = String> + '_>)>) -> Digest` plus `pub(crate) fn assert_family_prefixes_partition(prefixes: &[&str], header_labels: &[&str])` for tests.

**Test strategy.**
- Gating: `analysis_neutral::evidence` tests (must still pass after the move); the new generic property test instantiated once per provider; the per-provider `*_output_digest` unit tests where they exist.
- Invariant I1a: N/N on excalidraw (`dataflow`), the 45-file scope (`calls` and `dataflow`) and the 885-file scope (`calls`). The `dataflow` cells are required: two of the six sites belong to `data_flow` (`data_flow/provider.rs:466`) and the streaming helper is shared with `evidence`, and neither provider runs on a `calls` request (`provider.rs:1065-1067`), so a `calls`-only matrix would leave W2's riskiest edits unverified. Must not move: `digest=`. `rss_mb` is a sampled measurement (`analysis_kernel/mod.rs:332`) and is not reproducible to the megabyte, so it is reported as a direction, retained does not grow, and not gated (round 3). Expected: `peak_rss_mb - rss_mb` on the six stage rows shrinks toward zero.
- Note the research doc's warning (section 3.3): the semantic MIR digest is a header plus sorted rows; the header parts (`provider_id=`, `config=`, `upstream_syntax=`) must sort before or after each family exactly as `parts.sort()` placed them; the property test is what proves it.

**Verification probe (G1, stage-row gap).**

```sh
probe s45-after calls <45-file scope>
probe s45-df-after dataflow <45-file scope>                      # data_flow and evidence rows exist only here
python3 .scale-envelope/digests.py "$POLINT_GATE_OUT/s45-before.stderr" "$POLINT_GATE_OUT/s45-after.stderr"          # N/N (provider-count rule, section 5.1)
python3 .scale-envelope/digests.py "$POLINT_GATE_OUT/s45-df-before.stderr" "$POLINT_GATE_OUT/s45-df-after.stderr"    # N/N, the dataflow cell has two more rows
python3 .scale-envelope/stages.py "$POLINT_GATE_OUT/s45-after.stderr" | awk '$1 ~ /abstract_domains|semantic_mir/'   # peak column approaches rss column
```

Expected on the 45-file scope: `polint.abstract_domains` peak falls from 5,024 MB toward its retained 1,736 MB (benchmark report A.3).

**Dependency and risk.** Commit 1 depends on nothing; commit 2 is written on W1's merged commit 4 because both edit `refined_calls/provider.rs` (section 2). Top failure mode: a header label that is a prefix of a family label, or a family label that is a prefix of another (the property test rejects both); or a provider whose rows do not start with the family prefix in the same place the old `format!` put it. Detection: the property test at compile time of the test suite, and the digest oracle.

**Commit shape.** Two commits: (1) `refactor(digest): extract the streamed family digest from the evidence provider` (pure move, evidence digest unchanged; may land before W1); (2) `perf(digest): stream the six remaining sorted provider digests and drop the domains key side tables` (rebased on W1's commit 4; oracle captured against that base).

### W3. Demand at fact-family granularity, and an honest domain solver

**Goal.** After W3, a rule that requests only `calls` never runs `polint.abstract_domains`, because the closure seeds fact families rather than providers and no `calls` consumer reads the one summary row domains influence (Resolved Q4). When domains do run, the solver is intraprocedural by default with a per-function iteration cap and a per-run total (Resolved Q5), and it reports which functions were cut.

**Concrete changes.**

`crates/polint/src/analysis_api/provider/mod.rs`
- `ProviderManifest` (`:180`) gains `pub output_inputs: &'static [(&'static str, &'static [&'static str])]`: for each output family, the subset of `inputs` that family actually reads. Providers with one output or uniform needs list every output against every input, so the field is a refinement, not a rewrite. `ProviderManifest` is `pub(crate)`-reachable only (Resolved Q2), so this is not a public change; `provider_manifests_are_not_public_sdk_runner_or_cli_contract` (`analysis_kernel/provider.rs:2779`) stays green.

`crates/polint/src/analysis_kernel/provider.rs`
- `polint.direct_summaries` manifest (`:1568-1598`): `output_inputs` lists `summary_control` against the full input set including `domain_observations` and `domain_events`, and `summary_call`, `summary_memory`, `summary_tito`, `summary_events` against the input set minus those two.
- `polint.type_value_alias` manifest (`:1669-1716`): remove `domain_observations` and `domain_events` (`:1688-1689`) and all four summary families, `summary_control`, `summary_call`, `summary_memory`, `summary_tito` (`:1690-1693`), from `inputs`. All six are declared and never read: `grep -rn 'abstract_domain_observations\|abstract_domain_events\|summary_facts\|SummaryDomainKind\|ControlEffects\|summary_control\|summary_call\|summary_memory\|summary_tito' crates/polint/src/analysis_neutral/types/` returns nothing; the broader pattern `abstract_domain` returns exactly five hits, all `abstract_domains_output_digest` in `types/provider.rs` (`:55`, `:135`, `:182`, `:196`, `:221`), which is the digest-recipe plumbing that folds the `abstract_domains=` upstream digest, not a fact read (round 4: the earlier sentence said the broader grep returned nothing). Removing the domain rows alone is not enough: `summary_control`'s `output_inputs` row in `direct_summaries` demands both domain families, so a `type_value_alias` that still declared `summary_control` would re-demand them, and `refined_calls` requires `type_value_alias` on every `calls` path for `type_facts`, `value_facts`, `allocation_tokens`, `points_to_sets` and `alias_answers` (`:1825-1829`, produced only at `:1701-1710`). With all six rows gone, the closure on a `calls` request is: `refined_call_edges` demands `type_facts` (from `type_value_alias`, whose remaining inputs name no summary or domain family), `summary_call` and `summary_events` (from `direct_summaries`, whose `output_inputs` rows for those two families exclude the domain families), and `solver_derived_edges`; no demanded family's `output_inputs` row names `domain_observations` or `domain_events`, so `polint.abstract_domains` is not enabled. The three summary rows other than `summary_control` change no closure result (`direct_summaries` is on the `calls` path through `refined_calls` regardless) and are removed for the same reason, declared-but-unread. `polint.abstract_domains` is the only producer of the domain families (`:1561`); no other manifest declares them except `direct_summaries` (`:1583-1584`), which keeps both against `summary_control` only. Manifest inputs have no behavioural surface outside the closure: `ProviderOutcomeTracker::can_run` does not read `manifest.inputs`, it reads the hand-written `hard_dependencies()` table (`analysis_kernel/outcome.rs:325-341`, `:639-687`), so this edit changes nothing about which failures block which provider; the tracker change that W3 does need is in the `outcome.rs` block below and lands in commit 2. (Supersedes the round-2 sentence that claimed a `direct_summaries` failure would stop blocking `type_value_alias` after this edit; that was backwards, `hard_dependencies("polint.type_value_alias")` still lists `SUM` at `:677` and the manifest edit does not touch it.) In the same commit, `analysis_neutral/cache_key.rs` is checked for a v13 ledger entry naming `polint.type_value_alias` as `provider_id` (today the ledger lists `semantic_graph`, `go.semantic`, `solver` and `refined_calls` at `:13`, `:53`, `:67`, `:97`, and names `type_value_alias` only as an upstream digest), because `v13_cache_dependency_ledger_matches_provider_manifest_inputs` (`provider.rs:2027-2052`) asserts ledger inputs equal manifest inputs for every listed provider. This is a manifest-only change: on a `calls` request the closure result is unchanged by this commit alone (domains are still pulled by `direct_summaries` until commit 2), so every digest must not move.
- `seed_providers_for_capability` (`:1070`) becomes `seed_families_for_capability(capability) -> &'static [&'static str]`: `calls` and `control_flow` seed `refined_call_edges`, `call_reachability`, `summary_call`, `summary_events`, `solver_derived_edges`; `control_flow` additionally seeds `cfg_dominators`, `cfg_postdominators` (the guard policies read them, `policy_queries.rs:1383-1418`) and `summary_control`, which is what keeps `polint.abstract_domains` enabled on `control_flow` exactly as today, through `summary_control`'s `output_inputs` row, and not by seeding a domain family directly (nothing on the `control_flow` path reads `domain_observations` or `domain_events`: `policy_queries.rs` and `sdk/` contain no reference to either, and the only production reader of observations is `summaries/builder.rs:75`); `dataflow` seeds `domain_observations` and `domain_events` directly, plus `data_flow_*` and `evidence_*`; the rest as today. The `DemandPlan` records which families were demanded by a seed as opposed to by a consumer's `output_inputs` row (`seed_demanded`), because the materialisation decision below is a function of that distinction.
- `providers_enabled_by_capability_closure` (`:1094`) closes over families: a demanded family enables its producer; the producer's `output_inputs` row for that family demands those input families; repeat to fixpoint. A provider is enabled when at least one of its outputs is demanded. `BASELINE_PROVIDER_SEEDS` (`:1013`) stays as provider seeds. Resulting `calls`-only set: today's 21 minus `polint.abstract_domains`, that is 20; `control_flow` and `dataflow` sets unchanged. For a request that names both `calls` and `control_flow` (no `dataflow`), the enabled set is unchanged (21, domains through `summary_control`) but the materialisation changes from full to compact (see the `AbstractDomainsProvider::run` bullet); that is a stated behaviour change of commit 2, consistent with W3's goal that `calls` never pays for the full domain run, and it moves the `polint.abstract_domains` digest and its downstream digests on such requests while every diagnostic stays identical (no diagnostic reads observations, Q4). The golden corpus (`examples/*`) is checked for such requests and the change is recorded in the commit message.
- `providers_enabled_by_boolean_gates` (`:1031-1068`) is deleted together with the `debug_assert_eq!` that compares the closure against it on every run (`analysis_kernel/mod.rs:603-607`), and the exhaustive parity test `capability_closure_matches_boolean_pipeline_gates` (`provider.rs:1999-2025`) is replaced by `demand_plan_matches_expected_provider_sets`, which enumerates the same 128 capability subsets against an explicit table written by hand from the family rule (baseline 6; any of `calls | control_flow | dataflow` adds the 14 deep providers other than domains; `control_flow` or `dataflow` adds `polint.abstract_domains`; `dataflow` adds `data_flow` and `evidence`) and keeps the existing `scheduled_order_for` filter assertion. This is a stated behaviour change carried by the behaviour commit (commit 2 below), not an expectation edit made to pass a test: the boolean gates are the old expectation and are deleted, not edited.
- `AbstractDomainsProvider::run` (`:448-497`): the `compact_domain_materialization` decision (`:450-458`) moves out of the provider: the closure records, per enabled provider, which of its outputs were demanded and by whom, and `ProviderCtx` exposes `demanded_outputs(&self) -> &BTreeSet<&'static str>` and `seed_demanded(&self, family) -> bool`. The exact gating condition the new code must preserve: today the provider runs the compact `SummaryInputs` materialisation iff some rule requests `control_flow` and none requests `calls` or `dataflow` (`:450-458`, calling `derive_summary_input_abstract_domains_with_cache_stats`, `domains/provider.rs:45-68`), and the full materialisation otherwise; `polint unknowns --cap control_flow` is the compact case (`cli/mod.rs:2623-2629` requests the one named capability). After W3 the rule is: `Full` iff `domain_observations` is seed-demanded (only the `dataflow` seed does that); `SummaryInputs` iff the family is demanded only through `summary_control`'s `output_inputs` row. On the three probe cells this reproduces today's choice exactly: `calls`, not enabled; `control_flow`, enabled through `summary_control`, compact; `dataflow`, seed-demanded, full. The interprocedural solver mode (below) is selected by the same predicate, so "full" and "interprocedural" stay paired as they are today. The two `derive_*` entry points in `domains/provider.rs` stay.
- `scheduled_order_for` (`:1133`) is unchanged, and so is the order it yields, which is what commit 1's "every digest must not move" depends on: `scheduled_order` (`:923-955`) sorts topologically by manifest `inputs` with a min-heap tie-break on declaration index; the declaration order (`:1306` to `:1925`) is itself a topological order of the input graph, and a min-heap over a graph whose declaration order is topological emits ascending declaration index (the smallest unemitted node always has every predecessor, all of smaller index, already emitted), so removing `type_value_alias`'s edges from `abstract_domains` (index 11) and `direct_summaries` (index 12) cannot move any node. That matters because `ProviderCtx::dependency_digest` (`:69-74`) returns `absent` for an upstream whose output is not yet recorded; since no provider runs earlier than before, every recipe folds the same upstream digests and commit 1 moves nothing.

`crates/polint/src/analysis_neutral/domains/solver.rs`
- `SolverBudget` (`:33`) becomes `{ max_iterations_per_function: u32, max_iterations_total: u32, widening_fuel: u32 }` with `deterministic()` (`:81`) setting per-function 10_000 and total 1_000_000 (the total is the safety net; both are constants and can be retuned from probe data).
- `solve_with_output_mode` (`:112`): default mode is intraprocedural: `Icfg::build` is replaced by `Icfg::build_intra(db)` that emits `Intra` and `CallToReturn` edges only, so `Call`/`Return` edges and call-stack growth never occur; the exploded point's `call_stack` is then always empty and the `ExplodedPoint` map is keyed by node alone. The interprocedural mode (`Icfg::build`, call strings) remains behind `SolverPolicy { interprocedural: bool }` and is selected only when `dataflow` demanded `domain_observations` (kept for parity with today's behaviour on the `dataflow` path; W8 revisits it on the unit ICFG).
- The worklist loop (`:146-157`): per-function counter keyed by `function.body`; when a function's counter exceeds the per-function cap, only that function's states and status are marked `BudgetExceeded` (a per-function variant of `mark_ide_budget_exceeded`, `:662`) and its remaining queue entries are skipped; the total cap keeps today's whole-run behaviour.
- `materialize_results` (`:700-704`): replace the nested loop over all states per function with a lookup of the function's entry-node state (one `states.get(&ExplodedPoint { node: entry, call_stack: vec![] })`, or in interprocedural mode a `BTreeMap<CfgNodeId, Vec<&ProductState>>` grouped once).
- `DomainOutput` gains one `DomainEventFact` per cut function with reason `solver_budget_exceeded_function` and a run-level event `solver_budget_exceeded_total` when the total trips; `polint unknowns` surfaces both through the existing `budget_exceeded` row path.

`crates/polint/src/analysis/unknown_taxonomy/collect.rs`
- Add `domain_budget_unknowns(db)` that reads `db.abstract_domain_events()` and emits one `UnknownRow` per event with reason `solver_budget_exceeded_function` or `solver_budget_exceeded_total`, category `BudgetExceeded`, family `DomainBudget`, provider `polint.abstract_domains`, capability `control_flow` (or `dataflow` when that seed demanded the family), in the same shape as `resource_budget_unknowns` (`:242-258`) and `data_flow_budget_unknown` (`:496-506`). Today no collector path reads domain events, so without this row the "reports which functions were cut" claim would be unobservable from `polint unknowns`; the row is what the commit-3 probe greps for. Pre-existing `solver_budget_exceeded` events (the whole-run trip today) are surfaced by the same row so the `dataflow` cell shows its budget state too.

`crates/polint/src/analysis_neutral/ifds/mod.rs`
- Add `Icfg::build_intra(db)`; `from_facts` gains an `include_calls: bool` parameter.

`crates/polint/src/analysis_kernel/outcome.rs` (added after the adversarial review, round 3; this is the change without which commit 2 blocks the whole `calls` stack)
- Why it is needed: `ProviderOutcomeTracker::from_manifests` (`:311-357`) builds each provider's blocker list from `hard_dependencies(manifest.id)` (`:325-341`), a hand-written `match` (`:639-687`), not from `manifest.inputs`; every provider outside the enabled set is `AttemptState::PlannedAbsent` (`:344-354`); `is_usable` accepts only `ProvisionalSuccess` and `Final(Succeeded)` (`:536-545`); and `can_run` returns every non-usable hard dependency as a blocker (`:361-377`). Today that never bites because the boolean gates enable `polint.abstract_domains` on every deep request, and the only providers ever absent on a `calls` request (`data_flow`, `evidence`) are nobody's hard dependency. After the family closure of commit 2, `polint.abstract_domains` is absent on a `calls` request while `hard_dependencies` still names it under `polint.direct_summaries` (`:673`), `polint.type_value_alias` (`:677`) and `polint.semantic_graph` (`:678`): the kernel loop (`mod.rs:260-287`) would record `direct_summaries` as `dependency_blocked` with blocker `["polint.abstract_domains"]`, take the `skipped_direct_summaries_result` branch (`mod.rs:179-210`, an empty summary set), then block `type_value_alias` (on `DOM` and `SUM`), `semantic_graph` (`DOM`, `TVA`), `solver` (`GRAPH`, `TVA`) and `refined_calls` (`SUM`, `SOLVER`, `TVA`); `runtime_capability_blockers` (`mod.rs:777-840`) would then mark every rule requesting `calls` as blocked, because `capability_providers("calls")` is `polint.refined_calls` (`:859`), and emit a `polint/capability` error per rule. G5 would show five `MISSING` rows instead of five `DIFFER` rows, the ai-friendly `providers` array (`ProviderOutcomeRow`, `diagnostics/mod.rs:223-241`, with `status`, `stage`, `reason`, `blockers`) would carry five `dependency_blocked` rows, and `polint unknowns --cap calls` would report an empty call graph.
- The change: in `from_manifests`, a hard dependency that the demand plan did not select is not a dependency of this run. Concretely, the `dependencies` map is built as today and then each provider's list is filtered to `selected` (the `enabled_providers` set the kernel passes at `mod.rs:251-254`), so that a `PlannedAbsent` provider can never appear as a blocker. `hard_dependencies()` itself is not edited: its arms stay the full static dependency graph, which `hard_dependency_audit_references_only_static_manifest_providers` (`:806-823`) checks against the inventory and `every_manifest_provider_has_an_explicit_hard_dependency_arm` (`:826-846`, a source-text guard) requires to stay hand-written with one arm per manifest id; both tests are unchanged and stay green. The alternative of deleting `DOM` from the three arms was rejected: on a `control_flow` or `dataflow` request `polint.abstract_domains` is enabled and `direct_summaries` does read its observations for `summary_control` (`summaries/builder.rs:133-147`), so a failed domain run must still block `direct_summaries` there, which only the selected-set filter preserves. The semantics after the change are exactly the family closure's: a provider is blocked by a dependency the plan enabled and that did not succeed; a dependency the plan proved unneeded (its families are not demanded by any enabled provider's `output_inputs` row) is not a dependency.
- Behaviour today is unchanged by the filter: no enabled provider has a hard dependency outside the enabled set on any of the 128 request shapes under the boolean gates (the deep gates enable all fifteen deep providers together, `provider.rs:1046-1064`, and the baseline six have baseline-only dependencies), so the filtered lists equal the unfiltered lists on every request that exists before commit 2. The `polint.metrics` mirror validation `dependency_blockers_must_be_actual_metrics_dependencies` (`metrics_projection.rs:293`, test at `:596`) checks that recorded blockers are a subset of `hard_dependencies(id)`, which the filtered list still satisfies.
- Test impact of the behaviour commit, stated exactly: `hard_dependencies` is referenced at `analysis_kernel/mod.rs:30` (re-export), `metrics_projection.rs:293` and inside `outcome.rs` only (`grep -rn hard_dependencies crates/polint/src`); none of `providers_enabled_by_boolean_gates` (`provider.rs:1031`), `capability_closure_matches_boolean_pipeline_gates` (the 128-subset parity test, `provider.rs:1999-2025`) or `v13_cache_dependency_ledger_matches_provider_manifest_inputs` (`provider.rs:2028`) reads it or constructs a tracker, so the tracker change and the closure change do not interact through any test. `can_run_returns_sorted_exact_blockers_and_skips_unrelated_branches` (`outcome.rs:867`) constructs its tracker with every provider selected and is unaffected.

**Interfaces.**

```rust
// analysis_api/provider/mod.rs
pub struct ProviderManifest {
    pub id: &'static str,
    pub kind: ProviderKind,
    pub inputs: &'static [&'static str],
    pub outputs: &'static [&'static str],
    /// Per output family, the subset of `inputs` that family reads. Every output must appear.
    pub output_inputs: &'static [(&'static str, &'static [&'static str])],
    pub language_ids: &'static [LanguageId],
    pub cache_policy: CachePolicy,
    pub schema_versions: &'static [SchemaVersion],
    pub precision_ceiling: PrecisionCeiling,
}

// analysis_kernel/provider.rs
pub(crate) struct DemandPlan {
    pub(crate) enabled: BTreeSet<&'static str>,                          // providers
    pub(crate) demanded_outputs: BTreeMap<&'static str, BTreeSet<&'static str>>, // provider -> families
    pub(crate) seed_demanded: BTreeSet<&'static str>,                    // families demanded by a capability seed, not by a consumer row
}
pub(crate) fn demand_plan_for(requested: &BTreeSet<&str>) -> DemandPlan;

// analysis_kernel/outcome.rs (round 3)
impl ProviderOutcomeTracker {
    /// `dependencies[p]` = hard_dependencies(p) ∩ selected: a provider the plan did not select cannot block.
    pub(crate) fn from_manifests(manifests: &[ProviderManifest], selected: &BTreeSet<&'static str>) -> Result<Self, ProviderOutcomeError>;
}

// domains/solver.rs
pub struct SolverBudget { pub max_iterations_per_function: u32, pub max_iterations_total: u32, pub widening_fuel: u32 }
pub struct SolverPolicy { pub budget: SolverBudget, pub reduction_rounds: u32, pub interprocedural: bool }
```

**Test strategy.**
- Gating: `analysis_kernel::provider` tests (`provider_manifests_have_required_metadata` `:1974` extended to assert every output appears in `output_inputs` and every listed input is in `inputs`); `analysis_kernel` capability-closure tests; `analysis_neutral::domains::solver` tests including the shuffled-rows digest test (`:794`); `analysis_neutral::summaries` tests; `crates/polint/tests/capability_matrix.rs`.
- New tests: (a) closure over a synthetic manifest set where one provider's second output needs an extra input, asserting the extra input's producer is enabled only when that family is demanded; (b) `calls`-only plan does not enable `polint.abstract_domains`; `control_flow` plan does with `seed_demanded` lacking `domain_observations` (compact); `dataflow` plan does with it present (full); `{calls, control_flow}` enables it compact; the 128-subset table asserts the materialisation column as well as the enabled set; (c) solver fixture with one function that never converges and one that does, asserting only the first is `BudgetExceeded` under the per-function cap; (d) summary-family identity test: `summary_call`, `summary_memory`, `summary_tito`, `summary_events` digests equal with and without `domain_observations` present (the research doc's section 10 mitigation); (e) `summary_control` delta test: on a fixture with one function whose every exit block is observed `unreachable` and one that returns, the `summary_control` key set is identical with and without observations, exactly one row's `payload_digest` differs, and that row is a permitted shape (2) of the I1b line rule: its before parts contain `exit:DoesNotReturn` and its after parts are the same with `exit:Returns` in its place (`summaries/builder.rs:419-438`, `core.rs:110-124`); a companion unit test of the line-rule predicate itself feeds it constructed before/after parts strings for shapes (1), (2) and (3) and for three forbidden differences (a changed `async:*` part, a changed `cleanup:*` part, an added `exit:Unknown`) and asserts permit or fail accordingly; and `summary_events` is identical because the control-effects event is emitted only for `is_top()`, which is the `Top` variant and is unaffected by the exit set (`builder.rs:176-177`, `core.rs:53-55`); (f) `domain_budget_unknowns` emits one row per cut function on the (c) fixture and none when no event exists; (g) tracker test in `outcome.rs`: order `[A, B, C]`, selected `{A, C}`, dependencies `C -> [A, B]`: after `A` succeeds, `can_run("C")` returns no blocker (`B` is `PlannedAbsent` and filtered out at construction); with `B` selected and recorded as failed, `can_run("C")` returns `["B"]`; (h) kernel test on the `calls`-only fixture asserting every enabled provider's outcome is `succeeded`, no `dependency_blocked` row exists, and `polint.abstract_domains` is `planned_absent` in the sealed outcomes.
- Invariant I1, by commit. Commit 1 (manifest structure only): I1a, every digest identical on every cell. Commit 2 (domains leave the `calls` path): I1b on the `calls` cells: the fact-row dump is byte-identical for every family except `domain_observations` and `domain_events`, which are absent, and `summary_control`, whose key set is identical and whose rows differ only as the I1b line rule permits, on rows whose before parts carry `exit:DoesNotReturn` (the exit kind `build_control_effects` derives from observations, `builder.rs:419-431`; the row's `payload_digest` folds `control.stable_digest_parts()`, `builder.rs:156-166`, through `summary_fact_payload_metadata_digest`, `core/metadata.rs:392-409`), a delta the gate counts and the PR reports as the precision change Resolved Q4 (a) names; the diagnostics digest and stdout are identical (no diagnostic or unknown row reads `summary_control` or the observations); and the provider digests that move are exactly `polint.direct_summaries` (folds `abstract_domains=`, `summaries/provider.rs:42`, now `absent`), `polint.type_value_alias` (`types/provider.rs:221-222`), `polint.semantic_graph` (`dependency_digest("polint.abstract_domains")` at `provider.rs:726`), `polint.solver` (folds `semantic_graph` and `type_value_alias`, `:761-762`) and `polint.refined_calls` (`refined_calls/provider.rs:629-632`); `digests.py` must report `DIFFER` for those five and identical for the other fifteen rows, and `polint.direct_summaries` does have a row on a `calls` request (its `summary_call` and `summary_events` are `refined_calls` inputs, `:1823-1824`). Re-derived against `hard_dependencies()` as well as the manifests (round 3): with the tracker filter in this commit, the blocker lists of the 20 enabled providers are their `hard_dependencies` arms minus `polint.abstract_domains`, every member of which is enabled and succeeds in schedule order, so no provider is `dependency_blocked`, all five movers have a `stage done` row with a real digest (`DIFFER`, not `MISSING`), and `polint.abstract_domains` is the only `MISSING`. The `providers` array of `polint check --format json` (`ProviderOutcomeRow`, `diagnostics/mod.rs:223-241`; only the json path strips measurements through `without_measurements`, `:1022`, `:254-261`, while the ai-friendly JSON file clones the rows with `elapsed_ms` and cache counters, `:581`, so no byte-identity claim is made for it, round 5) is byte-identical on `calls`-only requests before and after commit 2: `provider_outcome_rows` (`analysis_kernel/outcome.rs:719-753`) drops every `PlannedAbsent` outcome (`:726-729`) and emits a `succeeded` row only for a provider with counters or one of the seven `PUBLICLY_NAMED_PROVIDERS` (`:698-706`, `:730-732`), and `polint.abstract_domains` reports `counts: Default::default()` (`analysis_kernel/provider.rs:490`) and is not publicly named, so it has no row today and no row after; no golden under `examples/*` or `tests/` can therefore change, and I2 is untouched. (Round 4: the round-3 sentence here claimed a `succeeded` to `planned_absent` row change and that `data_flow` and `evidence` already carry `planned_absent` on such requests; both claims were false, those two providers have no row at all on a `calls` request, and the sentence is superseded.) The observable proof that domains were not selected is therefore not the report but the `polint::kernel::stage` rows: the kernel emits a `stage done` row only for a provider that was selected and ready (`analysis_kernel/mod.rs:324-341`), and the rule host forwards them for `polint check` runs (`cli/mod.rs:4736`), so on `calls` the row for `polint.abstract_domains` is absent and on `control_flow` it is present. A missing stage row alone would also fit a `dependency_blocked` or `budget_exceeded` provider. The decisive clause, and the only one a G5 command produces (the probe runs `polint unknowns`, whose report is `{version, schema, tool, capability, rows}`, `cli/mod.rs:2654-2660`, with no providers field; no `polint check` cell exists in G5 and on the consumer scope it would be vacuous because no rule there requests `calls`), is the `digests.py` pattern: `MISSING` for `polint.abstract_domains` and `DIFFER`, not `MISSING`, for exactly `direct_summaries`, `type_value_alias`, `semantic_graph`, `solver` and `refined_calls`. A blocked or ceiling-tripped `abstract_domains` takes every one of those five with it (each is `dependency_blocked` in turn through its `DOM` hard dependency, `outcome.rs:673`, `:677`, `:678`, and emits no stage row), so all five would be `MISSING`; five `DIFFER` rows with real digests prove the five ran to completion without their dependency, which only not-selected permits. Test (h) asserts the sealed outcome is `planned_absent` in process. The unchanged `providers` array is a consequence, not a gate clause (round 5: the round-4 wording made the array the discriminator although no G5 command produces it). `polint unknowns` stdout is unaffected because its report carries `rows` only (`cli/mod.rs:2654-2660`), and the rows are computed from fact families that do not move. On the `control_flow` and `dataflow` cells every digest is identical in commit 2 because the enabled set and the materialisation are unchanged (the gating condition above reproduces compact on `control_flow` and full on `dataflow`); the only request shape whose digests move in commit 2 besides `calls` alone is `calls` together with `control_flow`, which goes from full to compact, as stated above. Commit 3 (per-function solver, `control_flow` path): I1a on the `calls` cells (domains do not run); on the `control_flow` cell the `polint.abstract_domains` digest may move only through the new per-function budget events, and every digest downstream of it moves by construction (the same five providers plus `entrypoints`, `reachability`, `data_flow` and `evidence` where they fold it, section 0 "digest recipes"); I1b on that cell must show every family identical except `domain_events` (new per-function rows), `domain_observations` (the intraprocedural default drops the call-string refinement of branch reachability, and the per-function cap cuts different functions) and `summary_control` (its `DoesNotReturn` rows follow the block-entry reachability observations); the PR lists the observed row deltas for all three and the count of `summary_control` rows that moved. On the `dataflow` cell the interprocedural mode is kept, so I1a applies.
- The determinism gate fixtures (`tests/eval-fixtures/determinism/*`) request capabilities that pull domains; they stay green.

**Verification probe (G5).**

```sh
probe s885-calls calls <885-file scope>
grep -c 'provider="polint.abstract_domains"' "$POLINT_GATE_OUT/s885-calls.stderr"      # 0
python3 .scale-envelope/digests.py "$POLINT_GATE_OUT/s885-before.stderr" "$POLINT_GATE_OUT/s885-calls.stderr"
#   expected: (N-6)/N identical, 15/21 on the branch base (provider-count rule, section 5.1); MISSING polint.abstract_domains; DIFFER exactly direct_summaries,
#   type_value_alias, semantic_graph, solver, refined_calls; any other DIFFER is a failure
# fact-row oracle: the same ignored test entry run from W3's baseline worktree (the branch base, section 2 baseline rule) and this checkout (after)
POLINT_FACT_ROWS_REPO=<scratch checkout> POLINT_FACT_ROWS_PATHS=<885-file scope> POLINT_FACT_ROWS_CAP=calls \
  POLINT_FACT_ROWS_OUT="$POLINT_GATE_OUT/rows-after-calls" \
  cargo test -p polint --lib --all-features --locked --release eval::fact_rows_dump::tests::dump_fact_rows -- --exact --ignored --nocapture
diff -r "$POLINT_GATE_OUT/rows-before-calls" "$POLINT_GATE_OUT/rows-after-calls"
#   expected: domain_observations and domain_events files absent after; summary_control differs only on lines whose
#   before parts column contains exit:DoesNotReturn and whose after parts column is one of the three shapes the I1b line
#   rule permits (removal; exit:Returns substituted; control=bottom), count reported; the parts column is the plaintext
#   SummaryFact::payload_digest, see W9; every other family identical
grep -c 'provider="polint.abstract_domains"' "$POLINT_GATE_OUT/s885-calls.stderr"      # 0: no stage row, so not selected and ready (mod.rs:324)
#   decisive clause: digests.py above shows DIFFER (real digests), not MISSING, for the five downstream providers; a
#   blocked or ceiling-tripped abstract_domains would leave all five dependency_blocked and MISSING (outcome.rs:673-678),
#   so DIFFER-not-MISSING plus the absent stage row proves not-selected; test (h) asserts planned_absent in process
diff "$POLINT_GATE_OUT/s885-before.stdout" "$POLINT_GATE_OUT/s885-calls.stdout"           # identical
probe s885-cf control_flow <885-file scope>
grep -c 'provider="polint.abstract_domains"' "$POLINT_GATE_OUT/s885-cf.stderr"         # 1
python3 .scale-envelope/digests.py "$POLINT_GATE_OUT/s885-cf-before.stderr" "$POLINT_GATE_OUT/s885-cf.stderr"   # N/N after commit 2 (provider-count rule, section 5.1)
polint unknowns --cap control_flow <885-file scope> | grep -c '"family": "DomainBudget"'   # the per-function cuts after commit 3; the row's family field, collect.rs shape as resource_budget_unknowns (:242-262)
```

Expected: on `calls`, total `facts` falls by the domain family size (280,617 at 885 files, benchmark report A.4); every family other than the two domain families and `summary_control` byte-identical at the row level, `summary_control` identical in key set with a counted payload-digest delta; exactly the five named provider digests move.

**Dependency and risk.** Closure half depends on nothing and can land with W1; solver half after W1 so the per-function solver's cost is measured against an indexed `Icfg::build`. Top failure mode: a family-level input declared too narrowly, so a provider reads a store the closure did not populate (a silent empty read rather than a crash, because the stores default to empty). Detection: test (d) above generalised: for every provider, run the fixture with each non-declared input family's store cleared and assert the output digest is unchanged; and test (h), which asserts no `dependency_blocked` outcome on the `calls` fixture, is the guard for the tracker: any future family-level narrowing that leaves a hard dependency unselected is covered by the selected-set filter, and any hard dependency that is selected and fails still blocks as today.

**Commit shape.** Three commits: (1) `feat(kernel): declare per-output input families in provider manifests; drop the unread domain inputs from type_value_alias` (structure only; closure result unchanged; every digest unchanged; ledger mirrored); (2) `feat(kernel): seed and close the provider set over fact families; hard dependencies filtered to the selected set; delete the boolean pipeline gates and their parity assert` (behaviour: domains leave the `calls` path; `outcome.rs` tracker filter with tests (g) and (h); the five downstream digests move as listed; the `providers` array is unchanged, since `polint.abstract_domains` has no row before or after, see I1 commit 2 above; I1b oracle); (3) `feat(domains): intraprocedural default with per-function and per-run iteration caps, reported through polint unknowns` (behaviour on the `control_flow` path only; includes the `DomainBudget` collector row).

### W4. Dominators from the reverse-postorder algorithm, tree only

**Goal.** After W4, dominance and post-dominance are computed per function with the Cooper-Harvey-Kennedy iteration over an immediate-dominator array in reverse postorder, never materialising the relation; tree edges are always emitted; the full closure is emitted only when the pair budget allows, derived from the tree; control dependence reads the post-dominator tree directly. The `polint.cfg` `dominators` step drops from 11 s to near-linear on the full backend.

**Concrete changes.**

`crates/polint/src/analysis_neutral/cfg/derived.rs`
- Replace `dominator_relation` (`:330`), `dominator_relation_with_extra_exit` (`:339`), `intersect_sets` (`:427`), `immediate_relation` (`:438`) and `postdominator_relation_for_graph` (`:464`) with:

```rust
/// Immediate dominators for one function graph in the given direction, CHK-style.
struct DomTree {
    /// Blocks in reverse postorder of the walked direction; index = rpo position.
    order: Vec<BasicBlockId>,
    position: BTreeMap<BasicBlockId, u32>,
    /// idom[i] for order[i]; None for the root and for unreachable blocks.
    idom: Vec<Option<u32>>,
    /// Universe members the walk from `root` never reached (exit-unreachable blocks in the reverse
    /// direction). Today's fixpoint leaves their set at the whole universe; every query on the tree
    /// answers for them by the legacy rule so that all three consumers (dominator emission,
    /// post-dominator emission, control dependence) see the relation they see today.
    vacuous: BTreeSet<BasicBlockId>,
    root: BasicBlockId,
    universe_without_root: BTreeSet<BasicBlockId>,
}
/// `universe` is the block set the relation is defined over; blocks outside it get no facts.
/// Forward: the entry-reachable set. Reverse: every block of the function plus the virtual exit.
fn dom_tree(graph: &CfgGraph<'_>, root: BasicBlockId, direction: Direction, universe: &BTreeSet<BasicBlockId>, selected_exits: &BTreeSet<BasicBlockId>) -> DomTree;
impl DomTree {
    /// Legacy answers for every block, vacuous or not:
    ///   dominators(b) == universe                      if b is vacuous   (derived.rs:346-356)
    ///   dominators(b) == ancestors of b in the tree     otherwise
    ///   immediate(b)  == smallest vacuous block != b    if b is vacuous   (immediate_relation, :438-462: the first strict
    ///                    candidate in BTreeSet order whose own set contains every other candidate; only another
    ///                    lattice-top block qualifies, so the pick is the smallest other vacuous id, or None)
    ///   immediate(b)  == idom(b)                        otherwise, with the root mapped to None when the caller
    ///                    strips the virtual exit (control dependence, :464-489)
    fn immediate(&self, block: BasicBlockId, strip_root: bool) -> Option<BasicBlockId>;
    fn dominators(&self, block: BasicBlockId) -> impl Iterator<Item = BasicBlockId> + '_;  // reflexive; universe for vacuous
    fn dominates(&self, a: BasicBlockId, b: BasicBlockId) -> bool;                      // a in dominators(b)
    fn is_vacuous(&self, block: BasicBlockId) -> bool;
}
```
  - Universe semantics, matching today's two directions exactly. Forward (`derive_dominators`, `:66-118`): `universe = reachable_blocks(&graph)` (`:82-83`, `:309-321`), root = entry; a block outside the universe (forward-unreachable) gets no dominator facts today and none after. Reverse (`derive_postdominators`, `:120-200`): `universe = every block of the function ∪ {virtual_exit}` (`:133-146`), root = `virtual_exit_for(function)` (`:511`), edges reversed with the selected exits feeding the virtual exit (`collect_reversed_predecessors`, `:404-425`; `selected_exit_blocks`, `:493-509`); forward-unreachable blocks are in this universe and do get post-dominator facts today. `dom_tree` computes reverse postorder over the blocks of `universe` reachable from `root` in the walked direction, then iterates `for b in order[1..]: new_idom = intersect over processed predecessors` until no change, with the two-finger `intersect` on rpo positions (https://www.cs.tufts.edu/~nr/cs257/archive/keith-cooper/dom14.pdf, the `doms` array formulation). Universe members the walk never reaches are the exit-unreachable blocks (an infinite loop with no return, a block whose only successors cycle back): today's set-intersection fixpoint never updates them (their reverse-reachable neighbour set is empty or all-vacuous), so their relation stays at the seeded `universe`, and under `Full` materialisation every `(block, member)` pair with `member != virtual_exit` is emitted (`:156-175`), with `immediate` set by `immediate_relation` (`:438-462`), which for such a block picks the first strict candidate in `BTreeSet` order whose own relation contains every other candidate. These rows are not a tree and cannot be read from `idom`; the `vacuous` set inside `DomTree` names them and every query (`dominators`, `dominates`, `immediate`) answers for them by the legacy rule, so the emission reproduces them literally (the pairs from `universe`, the immediate flag from the ported `immediate_relation` rule restricted to those blocks). They are also what today's `ImmediateOnly` emission produces for those blocks, so the default-bound output is reproduced too. Whether the vacuous rows should exist is a separate, reported semantic change and is out of W4's scope. One consequence of the legacy rule is recorded here because the port must not inherit it silently: with two or more vacuous blocks, `immediate` maps the smallest to the second-smallest and every other one to the smallest, so the legacy runner loop in `derive_control_dependence` (`:229-251`, which stops only on `stop`, on `None`, or on a self-loop) can alternate between two vacuous blocks without terminating when a reachable block has an edge into such a region. The differential test (e) below includes that shape; if the legacy code hangs on it, the port terminates (a `seen` guard on the runner) and emits the facts the loop had produced up to the repeat, the fixture's expected output is defined by the port, and the change is recorded as a bounded semantic difference in the commit message, since no byte-identity claim can be made against a computation that does not finish.
  - `derive_dominators` (`:66`) and `derive_postdominators` (`:120`): emit `immediate == true` facts from `idom` always; when `materialization == Full`, additionally emit every `(dominated, ancestor)` pair from `ancestors()`, sorted as today (`facts.sort_by_cached_key(...)` at the end of each function stays). The reflexive pair (a block dominates itself) is emitted today because the relation initialises with the start in its own set and every block ends up containing itself; keep it so the pair set is identical under `POLINT_CFG_MAX_DOMINANCE_PAIRS=0`.
  - `derive_control_dependence` (`:203`): today it recomputes the unbounded post-dominance relation itself (`postdominator_relation_for_graph`, `:214` and `:464-489`, which strips the virtual exit from every set and from the map) regardless of `DominanceMaterialization`, and uses it twice: the skip test `postdominators[from].contains(to)` (`:223-225`) and the runner stop `immediate.get(from)` (`:229`, `:245`). Its output is therefore the same in both materialisation modes today, and the port keeps that: it builds the reverse `DomTree` over the same universe and calls `tree.dominates(to, from)` for the skip test and `tree.immediate(b, strip_root = true)` for the stop and the runner steps. For a vacuous `from`, `dominates` is true for every `to` (the legacy set is the universe), so every outgoing edge is skipped exactly as today; for a vacuous runner, `immediate` returns the legacy pick. The virtual exit is stripped exactly where `postdominator_relation_for_graph` strips it: an exit block's immediate post-dominator is `None` for control dependence and the virtual exit for post-dominator emission (which then skips the pair at `:156-161`). The emitted `cfg_control_dependence` facts are unchanged in both modes, subject only to the runner-termination note above.
  - Keep the fact stable-key recipe (`:96-104`) byte for byte.
- `reachable_blocks` (`:309`) stays for `derive_reachability`.

`crates/polint/src/analysis_neutral/cfg/provider.rs`
- `append_derived_rows` (`:96`): the budget estimate and `materialization` selection (`:109-116`) stay; the relation is no longer computed when bounded, which is the wall-clock win.

**Interfaces.** Internal to `cfg::derived`; `derive_dominators`, `derive_postdominators`, `derive_control_dependence` keep their signatures.

**Test strategy.**
- Gating: `analysis_neutral::cfg::derived` tests (existing fixtures for immediate dominators, the `first_return` post-dominance case at `:723`); `cfg::validate` (`validate_cfg` checks dominator rows at `:232-260`); `policy_queries` guard tests (`guard_dominates_operation`, `guard_does_not_dominate`, `docs/facts/control-flow.md:94-98`); `analysis_kernel` tests asserting non-empty dominator families (`analysis_kernel/mod.rs:2214-2264`); the determinism gate.
- New tests: (a) a differential test that computes the old set-intersection relation (kept under `#[cfg(test)]` as `legacy_dominator_relation`) and the new tree closure plus vacuous rows on every CFG fixture and on randomly generated reducible and irreducible graphs (seeded, small, including graphs with exit-unreachable blocks), asserting identical `(dominated, dominator)` sets, identical immediate maps for both directions and both materialisations, and identical `cfg_control_dependence` fact sets (the legacy `derive_control_dependence` is kept beside the legacy relation under `#[cfg(test)]` for the comparison); (b) a forward-unreachable-block fixture asserting those blocks get no dominator facts (as today) and do get post-dominator facts (as today, because the reverse universe is every block); (c) an exit-unreachable fixture (`for {}` with no return) asserting the vacuous post-dominator rows and their immediate flags equal the legacy relation's; (d) the `selected_exits` virtual-exit case with multiple returns; (e) a fixture with a reachable block that branches into a region of two or more exit-unreachable blocks (`if c { return }; for { if x { a() } else { b() } }`), asserting the control-dependence facts equal the legacy output when the legacy loop terminates, and recording the bounded difference when it does not.
- Invariant I1a: with `POLINT_CFG_MAX_DOMINANCE_PAIRS=0` every digest must not move on excalidraw (`dataflow`, the cell the X5 A/B ran with the bound off at 6,400 MB peak and 242.5 s, `.scale-envelope/EXPERIMENTS.md`; `POLINT_GATE_TIMEOUT=600`) and on the 45-file scope, where the default bound does not trip and the default run already is the full relation (the bound first trips at 314 files, benchmark report 5.5.1); the full closure is emitted, the pair set is identical, vacuous rows included. The 885-file scope is not run with the bound off: its worst case is 4,111,312 pairs at roughly 1.4 KB of key text each (`cfg/budget.rs:1-19`), on the order of 5.8 GB of additional retained text on a run that already retains 7,113 MB beside a 6,846 MB sidecar, which does not fit the probe's 28 GB address-space guard with margin and has no branch-base capture. With the default bound, the `polint.cfg` digest and downstream digests must not move on any cell, because tree-only emission is what the bound already produced and control dependence never depended on the bound; the emitted tree-edge set must be identical. `polint check` diagnostics digest unchanged in both modes.
- Must not move: every digest in both modes, every pair set, every immediate flag, every `cfg_control_dependence` row (identical in both modes by construction, since its input is the unbounded relation in both); the only permitted movement is `elapsed_ms` on the `cfg` step rows.

**Verification probe (G4).**

```sh
probe full-cfg calls <core>                                  # after W1 so semantic_mir completes
grep 'provider="polint.cfg"' "$POLINT_GATE_OUT/full-cfg.stderr" | grep -E 'step|stage done'
# expected: dominators and postdominators steps each under 5 s; stage under 30 s (research doc G4)
POLINT_CFG_MAX_DOMINANCE_PAIRS=0 POLINT_GATE_TIMEOUT=600 probe exc-full-relation dataflow <excalidraw checkout>
python3 .scale-envelope/digests.py "$POLINT_GATE_OUT/exc-before-full-relation.stderr" "$POLINT_GATE_OUT/exc-full-relation.stderr"   # N/N (provider-count rule, section 5.1); the before capture is in slot 1
probe s45-w4 calls <45-file scope>; python3 .scale-envelope/digests.py "$POLINT_GATE_OUT/s45-before.stderr" "$POLINT_GATE_OUT/s45-w4.stderr"   # N/N; default bound == full relation at this size
```

**Dependency and risk.** Depends on nothing; parallel with W1. Top failure mode: a difference between the legacy relation and the tree closure on irreducible graphs, on graphs with the virtual exit and selected exits (post-dominance with multiple returns), or on the exit-unreachable blocks whose vacuous rows must be reproduced rather than derived. Detection: the differential test with seeded random graphs (dominators, post-dominators and control dependence), the fixture (e) for the runner-termination shape, and the `POLINT_CFG_MAX_DOMINANCE_PAIRS=0` oracle on excalidraw, which has 4,193 functions.

**Commit shape.** Two commits: (1) `perf(cfg): compute dominator trees with the reverse-postorder idom iteration; keep the legacy relation under cfg(test) for the differential` (behaviour-preserving, includes the differential test); (2) `perf(cfg): derive the bounded closure from the tree and skip the relation when the budget trips` (deletes the legacy relation's production use; the test copy stays until W6 deletes `CfgFactStore`).

### W5. Structural identity behind the resolver contract (the pivot)

**Goal.** After W5, a `StableKeyId` indexes a structural identity node `(family, parent, discriminant, atom)` rather than a retained text; canonical key text is produced on demand by walking parents into the existing `write_stable_key_text` buffer and is byte-identical to today's; `FactMeta::payload_digest` is a `u64`; the interner no longer retains composed text for the run. `key_mb` on every stage row drops by an order of magnitude; `keys` and every digest are unchanged.

**Concrete changes.**

`crates/polint/src/internal_core/stable_key.rs` (rewrite of the state, same public surface)

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct StableKeyId(pub u32);              // unchanged: dense, insertion-ordered

/// One canonical part of a key: a label and either a literal atom or a child key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum PartValue { Atom(AtomId), Key(StableKeyId) }

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AtomId(u32);                       // interned literal text (paths, names, decimal spans)

struct KeyNode {
    family: FactFamily,
    /// Sorted by label byte order, as write_stable_key_text sorts parts today.
    parts: Range<u32>,                        // into `part_labels` / `part_values`
    canonical_len: u32,                       // cached encoded length, checked arithmetic at build
    canonical_hash: u64,                      // FNV-1a of the canonical bytes, for map lookup
}

struct StableKeyInternerState {
    nodes: Vec<KeyNode>,
    part_labels: Vec<AtomId>,
    part_values: Vec<PartValue>,
    atoms: Vec<Box<str>>,                     // text stored once
    atom_ids: HashMap<Box<str>, AtomId>,
    /// canonical_hash -> candidate ids; collisions resolved by streamed byte comparison.
    by_hash: HashMap<u64, SmallVec<[StableKeyId; 1]>>,
    /// Boundary cache for `resolve`: bounded, LRU-evicted materialised texts.
    resolved: BoundaryCache,
}

impl StableKeyInterner {
    /// Structural intern: the only constructor providers use after W5.
    pub(crate) fn intern_parts(&self, family: FactFamily, parts: &[(&'static str, KeyPart<'_>)]) -> StableKeyId;
    /// Text intern kept for the boundary: parses nothing, interns as a single literal node.
    pub fn intern(&self, key: impl AsRef<str> + Into<String>) -> StableKeyId;   // unchanged signature
    pub fn resolve(&self, id: StableKeyId) -> Arc<str>;                          // unchanged signature; materialises via the walk, cached
    pub(crate) fn write_canonical(&self, id: StableKeyId, out: &mut String);    // streams the canonical text
    pub(crate) fn stream_canonical(&self, id: StableKeyId, sink: &mut impl FnMut(&[u8]));
    pub(crate) fn canonical_cmp(&self, a: StableKeyId, b: StableKeyId) -> Ordering; // streamed prefix comparison
    pub(crate) fn canonical_len(&self, id: StableKeyId) -> usize;
    pub(crate) fn len(&self) -> usize;                                           // node count, reported as `keys`
    pub(crate) fn text_bytes(&self) -> usize;                                    // atom bytes, reported as `key_mb`
}
pub(crate) enum KeyPart<'a> { Text(&'a str), Key(StableKeyId), Decimal(u64) }
```

  - `write_canonical` reproduces `write_stable_key_text` (`analysis_api/metadata.rs:512-523`) exactly: family label length-prefixed, then for each part in label order `|` + length-prefixed label + `=` + length-prefixed value with backslash folding (`:533-542`), where a `Key` part's value is the child's canonical text (recursively, iteratively walked with an explicit stack) and its length is the child's cached `canonical_len`.
  - `intern` of raw text (the boundary path and every test helper) creates a one-atom node with `family = Literal` whose canonical text is the atom itself; `write_canonical` emits it verbatim, so `intern("x")` still resolves to `"x"`.
  - Lookup: hash the would-be canonical stream without allocating (`fingerprint_part` semantics, `core/metadata.rs:501`), probe `by_hash`, confirm by streamed comparison. A raw-text node and a structural node with identical canonical bytes must intern to the same id (the `NEXT-LEVER.md` obligation); the comparison is on canonical bytes, so they do.
  - `detached_clone` (`:124`) deep-copies the state as today.
  - `intern_and_resolve` (`:80`) is deleted; its only callers are the metadata constructors, which W5 changes to stream.

`crates/polint/src/analysis_api/metadata.rs`
- `FactMeta::payload_digest: String` (`:238`, `:273`) becomes `payload_digest: u64`; `StableKeyOwner` likewise; `lower_hex_u64` (`core/metadata.rs:430`) is applied only where the digest is printed (debug output, store mirrors).
- `stable_key_from_parts` (`:468`) and `stable_key_text_from_parts` (`:489`) become thin wrappers over `intern_parts` and `write_canonical`; `write_stable_key_text` (`:512`) stays as the single source of the encoding and is what `write_canonical` calls per level.
- `FactMetaStore` (`:353`): unchanged structure; `payload_digest` comparisons become integer comparisons.

`crates/polint/src/core/metadata.rs`
- `fact_meta_from_borrowed_parts` and `fact_meta_from_stable_key` (`:301-321`): build the key with `intern_parts`; compute `payload_digest` by streaming `stream_canonical` into the FNV state followed by the extra parts through `fingerprint_metadata_part` (`:479`) in the same order `metadata_payload_digest` (`:374`) uses today, so the `u64` equals the value today's hex string encodes. Test: `lower_hex_u64(new) == old_string` on every fixture.

`crates/polint/src/analysis_neutral/stable_key.rs`
- `semantic_stable_key(family, parts) -> StableFactKey` (`:16`) is replaced by `semantic_key_parts(family, parts) -> Vec<(&'static str, KeyPart)>` plus `intern_semantic_key(interner, family, parts) -> StableKeyId`; all 34 call sites are converted. Call sites that embed a parent key by text (`("body", body_stable_key.to_string())` at `go/mir/lower.rs:2246`, `("owner", owner_stable_key_text.clone())` at `:636`, `("function", context.function_key.clone())` at `places.rs:144`, the dominator recipe at `cfg/derived.rs:96-104`, the observation recipe at `domains/store.rs:499-509`) pass `KeyPart::Key(parent_id)` instead. Call sites with decimal parts pass `KeyPart::Decimal` so no `to_string` allocation occurs.
- `PlaceStableContext` (`places.rs:27`) carries `file_key: StableKeyId, function_key: StableKeyId, body_key: StableKeyId` instead of `String`s; `PlaceTableBuilder::places: BTreeMap<String, PlaceDraft>` (`:13`) becomes `drafts: Vec<(StableKeyId, PlaceDraft)>` plus `index: HashMap<StableKeyId, usize>`; `insert_typed_with_context` keeps its dedup-and-fill semantics exactly (`places.rs:76-90`: first insert wins for every field, a later insert only fills a `None` type) by probing `index` and either pushing or filling; `finish_with_types` sorts `drafts` once by `canonical_cmp` before assigning `PlaceId(index)` (`:99-119`), which reproduces today's `BTreeMap<String, _>` iteration order (lexicographic key text) without a comparator (Rust's `BTreeMap` has none, and `StableKeyId`'s derived `Ord` is insertion order). The `place_ids: BTreeMap<String, PlaceId>` side table in `lower_go_mir` (`:44-47`) becomes `HashMap<StableKeyId, PlaceId>`.
- Every `sort_by_cached_key(|row| interner.resolve(row.stable_key))` (`ir/body.rs:123-160`, `analysis_neutral/store.rs:192,207,225,263,285,308,420`, `cfg/store.rs`, `cfg/derived.rs`, `cfg/graph.rs:59,165`) becomes `sort_by(|a, b| interner.canonical_cmp(a.stable_key, b.stable_key))`. Same total order, no materialisation. `mir_body_compose.rs:180-193` is in the same list: `CallSiteOrderKey` materialises `body_stable_key` and `operation_stable_key` as `String`s and sorts descriptors by them to assign every `CallSiteId`, so its two `String` fields become `StableKeyId`s compared through `canonical_cmp` (the derived `Ord` on the struct is replaced by an explicit comparator that consults the interner); W6 deletes the module later, but W5 lands first and the id assignment must stay byte-identical in between.

`crates/polint/src/analysis_kernel/mod.rs`
- The gauge (`:327-341`) keeps `keys = interner.len()` and `key_mb = interner.text_bytes()`; the semantics change is documented in the row's doc comment (atom bytes rather than composed text).

`crates/polint/src/analysis_kernel/store/*` and the layer cache
- Any persisted `payload_digest` string (provider mirrors, `witness_value`) is written through `lower_hex_u64`, so on-disk bytes are unchanged; no schema bump.

**Interfaces.** Above. `resolve` keeps returning `Arc<str>` for the SDK (`sdk/facts.rs:490`, `:565`) and for `resolve_stable_key` (`core/db.rs:417`); the boundary cache bounds the retained materialisations (default 64 k entries, `POLINT_KEY_CACHE_ENTRIES` override for measurement), and the gauge reports `resolved_texts` and `resolved_bytes` so the "memory improved" claim is measured (obligation 5).

**Test strategy.**
- Gating: every existing test that constructs or resolves keys (`internal_core::stable_key` tests, `analysis_api::metadata` tests including the conflict tests at `:676-720`, `analysis_neutral::stable_key` tests at `:27-49`, `places` tests at `:239-371`, every provider's `*_stable_key_*` test, the determinism gate, the golden corpus).
- New tests, one per proof obligation (research doc, W5 list):
  1. Canonical stream equality: for every fact family, on every fixture, on excalidraw, and on the 45 and 885-file scopes, `write_canonical(id)` equals the text the pre-W5 binary produced. Mechanism: a `#[cfg(test)]` dump of `(family, resolved text)` per fact family, run on post-W3 main (W5's baseline, section 2 baseline rule) and on W5, diffed byte for byte (a test-only CLI flag or the eval harness's fixture observation). The dump file is uncommitted for the consumer scopes and committed for fixtures under `tests/eval-fixtures/`.
  2. Digest oracle N/N (I1a) plus diagnostics digest on the same corpora.
  3. Conflict-set identity: `FactMetaStore::stable_key_conflicts()` count and members identical per corpus (`analysis_kernel/validation.rs:6015` covers the reporting path).
  4. Backslash folding and length prefixes: the `semantic_stable_key_sorts_parts_normalizes_backslashes_and_includes_family` test (`stable_key.rs:27`) ported to `intern_parts`, plus a nested case (child with a backslash inside a parent).
  5. Boundary materialisation count: a gauge assertion on the 45-file scope that `resolved_texts` after a `calls` run is below the number of facts.
  6. Iterative walk: a synthetic 100,000-deep chain interns and resolves without stack growth; `canonical_len` overflow is a checked-arithmetic error, not a wrap.
- Must not move: every `digest=`; `facts`; G1c canonical key text. Must move: `key_mb` down by an order of magnitude. Reported, not gated: `keys`, whose meaning changes from "distinct interned texts" (`internal_core/stable_key.rs:95-97`) to "identity nodes"; the two counts coincide only if every text that is embedded as a parent value is also interned standalone and no standalone text becomes a leaf atom, which holds for the recipes read here (`go/mir/lower.rs:630-643` interns the owner text at `:642` and embeds it at `:636`) but is not asserted, so the gauge documents the new meaning and the PR records the before and after counts.

**Verification probe (G3, part; G1).**

```sh
# W5's baseline is the post-W3 capture of slot 4, not the branch base (section 2 baseline rule): W3 removed
# polint.abstract_domains from every calls cell and moved five digests, so a branch-base comparison would fail by construction
probe full-mir-w5 calls <core>
grep 'provider="polint.semantic_mir"' "$POLINT_GATE_OUT/full-mir-w5.stderr" | grep -oE 'keys=[0-9]+|key_mb=[0-9]+|digest=[^ ]+'
# expected: key_mb under 350 (from 3,448); digest unchanged against the post-W3 capture; keys recorded (new meaning: identity nodes)
python3 .scale-envelope/digests.py "$POLINT_GATE_OUT/s885-postw3.stderr" "$POLINT_GATE_OUT/s885-w5.stderr"   # N/N (provider-count rule, section 5.1)
```

**Dependency and risk.** Depends on W1 and W2 landing on main first (measurement fairness; and W2 removes the last `String`-keyed side tables that would otherwise need conversion). Serial with everything: touches `internal_core`, `analysis_api`, every provider. Top failure mode: a canonical text that differs by one byte in one family (a part label order, a missing backslash fold in a nested child, a decimal formatted differently). Detection: obligation 1's byte-level dump diff on every family; obligation 2 is downstream of it and catches what the dump misses only if the family reaches a digest.

**Commit shape.** Five commits, each green on the fixture suite and the excalidraw oracle:
1. `refactor(identity): add the structural interner behind the existing resolve contract; raw-text intern unchanged` (new state, `intern_parts`, `write_canonical`, `canonical_cmp`; no caller converted; `resolve` byte-identical by construction).
2. `refactor(identity): payload digests as u64, printed as the same hex` (metadata store and constructors).
3. `refactor(identity): convert the MIR, place and CFG key recipes to structural parts` (lowerers, places, cfg; oracle on excalidraw).
4. `refactor(identity): convert the remaining 34 semantic_stable_key call sites and the canonical-order sorts`.
5. `chore(identity): delete intern_and_resolve and the text-keyed side tables; gauge reports atom bytes and boundary materialisations`.

### W6. Per-unit lowering, arenas, and parallel units

**Goal.** After W6, MIR lowering, CFG construction, per-function domains and call-site extraction run per unit (direct call target resolution stays a whole-program pass after the merge until W8, see below) (Go package, or TS project with a per-file fallback, Resolved Q7) into arena-backed unit graphs with dense local ids, in parallel across units with a deterministic merge order; the whole-program `SemanticStore`, `CfgFactStore` and the per-run `normalized` sorts are deleted; `polint.semantic_mir` and `polint.cfg` are linear in the unit and parallel across units. G3, G4, G6 (first attempt), G7 and G8 are measured on this.

**Concrete changes.**

New module `crates/polint/src/analysis_neutral/unit/` (`mod.rs`, `graph.rs`, `schedule.rs`, `merge.rs`) for the unit set, the unit graph and the merge, all language-neutral. Unit discovery is not neutral for TS (it needs the `tsconfig` walk in `ts/module_graph`), and `analysis_neutral` may not name `ts` (`crates/polint/tests/module_layering.rs:71-75`, gating in section 5.4), so the two languages discover units differently, and the difference is stated because the interface must be writable as declared. Go units need no frontend call: they are read from `TopologyPackageFact` and `ImportToPackageFact`, which live in `analysis_neutral/module_graph/topology.rs` and are reachable through the host, so `build_unit_set` builds them directly. TS units need the `tsconfig.json` walk, which is a filesystem walk over paths (`nearest_tsconfig_path`, `ts/module_graph/mod.rs:1591`, made `pub(crate)` inside `ts`) and needs no fact database, so it fits the trait's existing input: `frontend_api::LanguageFrontend` gains `fn unit_roots(&self, unit: &AnalysisUnit<'_>) -> Vec<UnitRoot>` with a default body returning an empty vector (`AnalysisUnit<'a>` is `{ files: &'a [&'a SourceFile], root: &'a Path }`, `frontend_api/mod.rs:21-25`; `SourceFile` carries its `FileId` and relative path, which is all the walk needs). The TS frontend overrides it; the Go frontend keeps the default. The kernel's provider dispatch, which is not constrained by any layering rule and already dispatches per-language providers (`analysis_kernel/provider.rs:991-1005`), collects the TS roots from the frontend registry and hands them to `analysis_neutral::unit::build_unit_set(db, ts_roots: &[UnitRoot])` as data. `UnitRoot { language, kind: UnitKind, path: String, files: Vec<FileId>, imports: Vec<String> }` lives in `frontend_api`; for TS `imports` are the project-reference paths parsed from the config (filesystem again), empty when unparseable.

```rust
// unit/mod.rs
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) struct UnitId(pub u32);            // index into UnitSet::units, assigned in sorted unit-path order

#[derive(Clone, Debug)]
pub(crate) struct Unit {
    pub(crate) id: UnitId,
    pub(crate) kind: UnitKind,                // GoPackage | TsProject | TsFile
    pub(crate) path: String,                  // package directory or tsconfig directory or file path, repo-relative, '/'-separated
    pub(crate) language: Language,
    pub(crate) files: Vec<FileId>,            // sorted by relative path
    pub(crate) imports: Vec<UnitId>,          // direct dependencies among units, sorted
    pub(crate) topology_package: Option<TopologyPackageId>,
    pub(crate) input_digest: Digest,          // over the unit's own file content digests only; imports' export digests travel separately as W7's dependency_layer_digests (round 5: the earlier comment double-counted them)
}

pub(crate) struct UnitSet {
    pub(crate) units: Vec<Unit>,              // index == UnitId
    pub(crate) unit_of_file: HashMap<FileId, UnitId>,
    pub(crate) order: Vec<UnitId>,            // topological over `imports`; unordered remainder appended as its own SCC, sorted
    pub(crate) sccs: Vec<Vec<UnitId>>,        // TS projects with reference cycles, and Go packages whose _test.go files close an import cycle
}
pub(crate) fn build_unit_set(db: &impl AnalysisHost, ts_roots: &[UnitRoot]) -> UnitSet;  // Go units from topology facts in db; TS units from ts_roots
```
  - Go: one unit per `TopologyPackageFact` (`analysis_neutral/module_graph/topology.rs:66`) with `language == Some(Go)`, files from `PackageFact.file` grouped by package path; `imports` from `ImportToPackageFact.{from_package,to_package}` (`:132-144`). A file with no package fact is its own unit of kind `GoPackage` with a synthetic path. The Go import graph over these units is not guaranteed acyclic: `include_tests` defaults to `true` (`go/lifecycle.rs:121-124`), `_test.go` files are grouped into the unit of the package they sit in, and Go allows a package's test files to import a package that imports the package under test (the toolchain builds a separate test variant; `ImportToPackageFact` carries no variant distinction). Such a cycle becomes one SCC in `sccs`, closed as one step exactly as a TS project cycle is; the scheduler needs no special case. The only singleton statement the plan makes is W8's qualified one, that non-test Go units form a DAG of singleton SCCs; units that include `_test.go` files carry no such guarantee (round 3 wording).
  - TS/JS: the TS frontend's `unit_roots` yields one root per `tsconfig.json` discovered by `nearest_tsconfig_path` (`ts/module_graph/mod.rs:1591`, the private function that walks up to the nearest `tsconfig.json` only; not `find_ts_project_root` at `:281-292`, which also stops at `package.json` and defines module nodes, a different partition) with the files it claims (the same walk `:1355-1362` performs for path aliases); files with no config above them are `TsFile` roots. Project references, when parseable from the config, become `imports`; otherwise the projects are ordered by path. (PR #121's `ts/types/lifecycle.rs` does the same walk for the sidecar; when it merges, the TS `unit_roots` reuses its project list so the two agree.) The existing TS `TopologyPackageFact` rows are `JsPackage` per `package.json` (`:352-372`) and are not the unit.
  - Ordering: Tarjan SCC over `imports` (reuse `analysis_neutral/summaries/scc.rs`), SCCs in topological order, members sorted by `UnitId`; units not reachable through any import edge are appended in `UnitId` order (research doc section 10, "synthetic go.work" risk).

```rust
// unit/graph.rs
pub(crate) struct UnitGraph {
    pub(crate) unit: UnitId,
    pub(crate) bodies: Arena<BodyIdx, BodyRow>,
    pub(crate) blocks: Arena<BlockIdx, BlockRow>,
    pub(crate) statements: Arena<StmtIdx, StmtRow>,
    pub(crate) terminators: Arena<TermIdx, TermRow>,
    pub(crate) operations: Arena<OpIdx, OpRow>,
    pub(crate) places: Arena<PlaceIdx, PlaceRow>,
    pub(crate) projections: Vec<PlaceProjection>,      // ranges referenced by PlaceRow
    pub(crate) place_types: Vec<(PlaceIdx, TypeShape)>,
    pub(crate) unsupported: Arena<UnsupportedIdx, UnsupportedRow>,
    pub(crate) call_sites: Arena<SiteIdx, CallSiteRow>,
    pub(crate) cfg: Vec<FunctionCfg>,                  // one per body
    pub(crate) keys: UnitKeys,                         // StableKeyId per row, built structurally (W5)
    pub(crate) digest: Digest,                         // canonical byte stream of the arenas, streamed
}
pub(crate) struct FunctionCfg {
    pub(crate) body: BodyIdx,
    pub(crate) entry: BlockIdx,
    pub(crate) normal_exit: BlockIdx,
    pub(crate) exceptional_exit: Option<BlockIdx>,
    pub(crate) edges: Csr<BlockIdx, CfgEdgeKind>,      // sorted, deduplicated; petgraph::csr or an in-crate CSR
    pub(crate) rpo: Vec<BlockIdx>,
    pub(crate) idom: Vec<Option<BlockIdx>>,            // W4's DomTree, stored
    pub(crate) ipdom: Vec<Option<BlockIdx>>,
    pub(crate) pdom_vacuous: Vec<BlockIdx>,            // W4's vacuous set for the reverse tree; needed by every consumer that answers post-dominance
}
/// The virtual exit of the reverse walk is a per-function sentinel, never a packed id:
/// `BlockIdx::VIRTUAL_EXIT == BlockIdx(u32::MAX)`; `Arena` indexing rejects it and the CFG
/// consumers test for it explicitly. W4's `virtual_exit_for(function)` (`cfg/derived.rs:511-513`,
/// `BasicBlockId(u64::MAX - function.0)`) is replaced by this sentinel in W6, because under the
/// packed encoding below it would decode as unit `0xFFFF_FFFF`, which is reserved (see the id bullet).
/// Typed dense index: a u32 newtype per arena, `Copy`, with `Arena<I, T> = TiVec<I, T>` semantics.
pub(crate) struct Arena<I, T>(Vec<T>, PhantomData<I>);
```
  - Row types keep today's payload enums (`MirTerminatorKind` `ir/body.rs:51`, `MirOperationKind` `ir/op.rs:20`, `PlaceRoot`/`PlaceProjection` `ir/places.rs:25-60`), with `Vec<PlaceId>` argument lists and `Vec<(MirValue, MirBlockId)>` cases moved into side arenas referenced by `Range<u32>` so `OpRow` and `TermRow` are `Copy`. `MirValue::BinOp`'s boxed children become indices into a `values: Arena<ValueIdx, MirValue>` side arena.
  - Whole-program ids (`MirBodyId`, `MirOpId`, `PlaceId`, `BasicBlockId`, `CfgNodeId`, `CallSiteId`) become `(UnitId, local index)` pairs packed in the existing `u64` newtypes: high 32 bits unit, low 32 bits local. Unit id `0xFFFF_FFFF` is reserved and never assigned to a unit, so any synthetic id that today lives in the top of the `u64` range (the virtual exit, `cfg/derived.rs:511-513`) is either replaced by a sentinel (above) or, if one survives, decodes to "no unit" and is rejected by `unit_of`. Every consumer that compares ids keeps working; every consumer that indexes a whole-program `Vec` by id is rewritten to go through the unit accessor.

```rust
// analysis_neutral/host.rs additions; the &[Fact] accessors for MIR and CFG families are deleted
pub trait AnalysisHost: FactDatabase {
    fn units(&self) -> &UnitSet;
    fn unit_graph(&self, unit: UnitId) -> &UnitGraph;
    fn unit_graphs(&self) -> impl Iterator<Item = &UnitGraph> + '_;       // in UnitId order
    fn body(&self, id: MirBodyId) -> &BodyRow;                              // (unit, local) unpacked
    fn operation(&self, id: MirOpId) -> &OpRow;
    fn place(&self, id: PlaceId) -> &PlaceRow;
    fn block(&self, id: BasicBlockId) -> &BlockRow;
    fn function_cfg(&self, body: MirBodyId) -> &FunctionCfg;
    fn call_site(&self, id: CallSiteId) -> &CallSiteRow;
    fn replace_unit_graphs(&mut self, graphs: Vec<UnitGraph>) -> Result<(), AnalysisError>;
    // deleted: mir_bodies(), mir_operations(), mir_blocks(), mir_statements(), mir_terminators(), mir_places(),
    //          mir_place_types(), cfg_functions(), cfg_nodes(), cfg_blocks(), cfg_edges(), replace_semantic_mir(), replace_cfg_facts()
    // kept for now (W8 revisits): cfg_reachability(), cfg_dominators(), cfg_postdominators(), cfg_control_dependence(),
    //          call_sites(), call_targets(), and every non-MIR family
}
```
  - The four derived CFG families and the call-site facts are still materialised as whole-program `Vec`s after the merge so that `policy_queries`, `validate_cfg`, `data_flow`, `evidence` and the SDK's `ControlFlow` view keep their inputs unchanged in this workstream; they are produced from the unit graphs in `UnitId` order. W8 moves the interprocedural consumers onto the unit graphs.

Lowering and providers:
- `go/mir/lower.rs`: `lower_go_mir(db)` becomes `lower_go_unit(db, unit: &Unit, index: &LoweringIndex) -> UnitGraph`; `GoMirLowering` state becomes per unit; `lower_control_flow` runs per body inside the unit; the CFG builder (`cfg/lower.rs`, `cfg/builder.rs`) is invoked per body producing `FunctionCfg` with W4's `DomTree` stored as `idom`/`ipdom`; per-function domains (W3's intraprocedural solver) run per body inside the unit when demanded; call-site extraction (`calls/extract.rs::extract_call_sites`, `:16`, body-local apart from the `owner_symbol` lookup W1 indexes) runs per unit. Direct call target resolution does not: `resolve_direct_call_targets` (`calls/direct.rs:18`) builds `functions_by_name` over every `FunctionFact` in the run (`:153`, `:315-331`) and resolves each site's lexical callee against that whole-program index, so a per-unit run with a unit-local function table would lose every cross-package direct target and move `call_targets`, `unresolved_calls` and everything downstream. In W6 it therefore stays a whole-program pass inside the `polint.unit_graphs` provider, run after the unit merge over the merged call sites and the unchanged `db.functions()`, producing `call_targets` and `unresolved_calls` exactly as today; W8 moves it onto the `CrossUnitIndex`. `ts/mir/lower.rs` likewise.
- `analysis/provider.rs` (`derive_semantic_mir_with_cache_stats`) and `cfg/provider.rs` (`derive_cfg_with_cache_stats`) are replaced by one `analysis_neutral/unit/provider.rs::derive_unit_graphs_with_cache_stats(db, snapshot, manifest, demanded_outputs, upstream_digests)` behind a new provider id `polint.unit_graphs` whose manifest outputs the union of today's `semantic_mir`, `cfg`, `abstract_domains` (intraprocedural families) and `calls` outputs; the four old provider ids are removed from every place that names them, which at `026407b7` is 34 files (`grep -rlE 'polint\.(semantic_mir|cfg|abstract_domains|calls)\b'` over `crates/polint/src`, `crates/polint-eval/src`, `crates/polint/tests`; 32 with the ids quoted as string literals, the two extra being doc comments; round 3 corrected the earlier count of 33, which had omitted `analysis_neutral/fact_store.rs`): `analysis/{identity,refined_calls,semantic_graph,solver,summaries}/provider.rs` and `analysis/provider.rs`, `analysis/unknown_taxonomy/collect.rs`, `analysis_kernel/{debug,mod,outcome,provider,resource,validation}.rs` (in `outcome.rs` the `hard_dependencies` arms: the four deleted ids' arms go, a `"polint.unit_graphs"` arm is added, and every consumer arm's `MIR`, `CFG`, `CALLS` and `DOM` entries collapse to the new id, which the source-text guard `every_manifest_provider_has_an_explicit_hard_dependency_arm` and the inventory audit both require), `analysis_kernel/incremental/{keys,run_report}.rs`, `analysis_neutral/{cache_key,error,fact_store,mod}.rs`, `analysis_neutral/calls/{direct,store,validate}.rs`, `analysis_neutral/cfg/provider.rs`, `analysis_neutral/demand/{context,trace}.rs`, `analysis_neutral/domains/validate.rs`, `analysis_neutral/semantic_graph/cache_key.rs`, `core/mod.rs`, `core/tests/batch{1,2}.rs`, `polint-eval/src/harness/{fixtures,mod,observed}.rs`, `tests/cli.rs` (a hard-coded provider-id list at `:3313`), plus `analysis_api/digest/keys.rs:9-25` (add `UnitGraphs`, remove `SemanticMir`, `Cfg`, `AbstractDomains`, `Calls` at `:16-19`; `LayerKind` is `Serialize`/`Deserialize` with `#[serde(rename_all = "snake_case")]` and is a field of the persisted `LayerKey`, `incremental/keys.rs:51-64`, so this commit bumps `LAYER_CACHE_MANIFEST_SCHEMA` to `polint-layer-cache-manifest-3` as I3 requires, even though only the `GoSyntax` and `TsSyntax` layers are wired today, `cache/analysis_cache_adapter.rs:152-156`; W7's later bump is then to `-4`). `analysis_neutral/cache_key.rs` is load-bearing: its v13 ledger entries name the deleted ids among their upstream digests (`semantic_graph` at `:36-49` lists `polint.calls`, `polint.abstract_domains` and `polint.semantic_mir`; `refined_calls` at `:121` lists `polint.calls`) and `v13_cache_dependency_ledger_matches_provider_manifest_inputs` (`provider.rs:2027-2052`) asserts ledger inputs equal manifest inputs, so every manifest edit is mirrored there in the same commit. The determinism gate auto-enrolls the new provider (`determinism_gate.rs`, D-22). The interprocedural domains mode (dataflow path) becomes a separate provider `polint.interprocedural_domains` running after the merge, so `dataflow` digests can be compared.
- Parallelism: `derive_unit_graphs_with_cache_stats` lowers units with `rayon::par_iter` over `UnitSet::order` chunks when `KernelInput.parallel` is true (`analysis_kernel/mod.rs:531`; job count from `jobs.rs`), collecting `Vec<UnitGraph>` and sorting by `UnitId` before the merge. Each unit's lowering gets a `LoweringIndex` slice for its own files plus the whole-program symbol graph (read-only). The interner (W5) is shared and its writes are the only cross-unit synchronisation; `intern_parts` takes the write lock only on a miss.
- Deleted in the same PR series: `analysis_neutral/store.rs` (`SemanticStore` and every `normalize_*`), `cfg/store.rs::CfgFactStore` and `CfgOutput::normalized`, `mir_body_compose.rs::merge_language_outputs`, `ir/body.rs::MirOutput::normalized`, the `refresh_semantic_mir_metadata` and `refresh_cfg_metadata` walks in `core/db.rs` (metadata is recorded per unit at build time), and the legacy dominator relation kept under `cfg(test)` by W4.

**Interfaces.** Above; plus the provider-side contract:

```rust
pub(crate) fn derive_unit_graphs_with_cache_stats(
    db: &mut AnalysisDb,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    demanded_outputs: &BTreeSet<&'static str>,
    upstream_digests: UnitGraphUpstreamDigests,   // module_topology, symbol_graph, go.syntax, ts.syntax
    parallel: bool,
) -> UnitGraphsProviderOutput;                     // diagnostics, cache_stats, output_digest, execution, per-unit digests
```

The provider output digest is the digest over `(UnitId, unit.input_digest, unit_graph.digest)` in `UnitId` order; per-unit digests are what W7 persists.

**Test strategy.**
- Gating: every test that touched the deleted accessors (the research doc's count: `cfg/validate.rs` 12 sites, `data_flow/local.rs` 10, `analysis_kernel/debug.rs` 10, `analysis_kernel/validation.rs` 8, `mir_validation.rs` 7, `domains/solver.rs` 7, `policy_queries.rs` 6, `types/*` 17, `summaries/builder.rs` 5, `calls/extract.rs` 5, and the rest) is rewritten against the unit accessors; the golden corpus; the capability matrix; `public_surface_leak.rs`; `consumer_api_compat.rs`; the determinism gate with a new fixture `tests/eval-fixtures/determinism/go_multi_package` (three packages with a diamond import graph and a closure capturing a cross-package symbol) and `ts_multi_project` (two `tsconfig` projects with a reference and one orphan file).
- New tests: (a) `build_unit_set` on the fixture repositories asserts unit membership, order, and SCC grouping; a Go fixture with a package not reachable by any import lands in the appended tail; (b) lowering a unit twice yields byte-identical `UnitGraph::digest`; (c) lowering the fixture set with `parallel = true` under the N=10 seeded permutation of unit processing order yields byte-identical merged output (extend `determinism_gate.rs` with a unit-order permutation alongside the provider-order permutation); (d) the L2 and L3 capability probes (`crates/polint-eval/src/harness/capability_probes.rs`) pass at the same counts as before (research doc section 10, "per-unit lowering loses a cross-file fact"); (e) closure captures across files within a unit and across units resolve to the same capture names as the whole-program lowerer did (fixture with a closure in package B capturing a symbol defined in package A); (f) a Go fixture whose `_test.go` file imports a package that imports the package under test lands the two packages in one SCC and lowers deterministically under permutation; (g) a direct call from package A into package B resolves to the same `call_targets` row as before W6 (the whole-program target pass), asserted on the three-package diamond fixture.
- Invariant I1 for W6 is tier I1b, and the provider-digest tier is expected to move almost everywhere: every downstream recipe names the four deleted providers (`entrypoints/provider.rs:92-94`; `summaries/provider.rs:39-42`; `types/provider.rs:218-222`; `data_flow/provider.rs:417-422`; `evidence/provider.rs:611-616`; and the `dependency_digest("polint.semantic_mir" | "polint.cfg" | "polint.calls" | "polint.abstract_domains")` sites at `analysis_kernel/provider.rs:428`, `:513-516`, `:585-587`, `:620`, `:683-687`, `:724-734`, `:819-821`, `:851-853`, with `solver` and `refined_calls` folding the movers transitively at `:761-763` and `:789-794`), so `identity`, `direct_summaries`, `entrypoints`, `reachability`, `type_value_alias`, `semantic_graph`, `solver`, `refined_calls`, `data_flow` and `evidence` all move on the first W6 commit that removes the ids. Commit 4 therefore rewrites those recipes to fold `polint.unit_graphs=` (one line per recipe in place of the four), which is a stated cache-key change, and the oracle is: (a) the fact-row dump (I1b) byte-identical for every family, including the MIR, place, CFG and call-site families whose key recipes are unchanged and whose storage moved into unit graphs; (b) the `polint check` diagnostics digest identical; (c) ai-friendly stdout identical; (d) the provider digests of `source`, `go.syntax`, `ts.syntax`, `module_graph`, `symbol_graph`, `module_topology`, `go.semantic`, `extensions` and `metrics`, which fold none of the four, identical (I1a on that subset). After commit 4 the new digests are the baseline for commits 5 to 7, which must hold I1a in full.
- Must not move: ai-friendly stdout bytes across permutations; the nine unaffected provider digests; diagnostics digest; every fact family at the row level. Must move: `polint.semantic_mir` and `polint.cfg` rows are replaced by one `polint.unit_graphs` row whose `elapsed_ms` is a fraction of their sum and whose `rss_delta_mb` is per unit; the ten downstream digests move once, in commit 4, for the reason stated.

**Verification probe (G3, G4, G6 first attempt, G7, G8).**

```sh
probe full-w6 calls <core>
grep 'provider="polint.unit_graphs"' "$POLINT_GATE_OUT/full-w6.stderr"        # elapsed_ms under 60 s, rss_delta_mb under 3,000, key_mb under 1,500
grep '"peak_rss_gb"' "$POLINT_GATE_OUT/full-w6.stderr" | tail -1                # first G6 attempt: exit 0 is the requirement; 300 s / 12 GB tree is the target
probe full-ts-w6 calls <frontend paths>                          # G8 first attempt
# G7: the determinism gate plus two full 885-file runs with RAYON_NUM_THREADS=1 and =12, probe stdout diffed
#     (polint unknowns emits agent JSON only, cli/mod.rs:314-316); ai-friendly bytes are diffed from polint check on examples/*
```

**Dependency and risk.** Depends on W5 (structural ids before unit-local ids), W1, W3. Serial with everything. Top failure mode: a cross-unit fact the whole-program lowerer produced from database order (closure capture names via `references_for_file`, module-level TS functions, `enclosing_function` across a unit boundary) is lost or reordered. Detection: test (e), the L2/L3 probes, and the W5 dump diff re-run on the 885-file scope; any missing key text is a stop. Second failure mode: parallel units change output bytes. Detection: the extended determinism gate and the two-thread-count stdout diff.

**Commit shape.** Seven commits, in this order, each green:
1. `feat(unit): TS unit roots from the frontend contract, Go units from topology facts, unit set construction with SCC order, with tests` (structure only; nothing consumes it; `module_layering` stays green because `analysis_neutral` reads its own topology facts and receives TS roots as data).
2. `feat(unit): arena-backed UnitGraph and typed dense indexes; lowering into a unit graph for one unit behind a test entry point` (no provider change).
3. `feat(unit): per-body CFG with stored idom/ipdom inside the unit graph` (uses W4's tree).
4. `feat(kernel): polint.unit_graphs provider replacing semantic_mir, cfg, calls and intraprocedural abstract_domains; whole-program stores deleted; downstream digest recipes fold unit_graphs` (the behaviour commit; oracle I1b plus the nine unaffected digests plus diagnostics and stdout). This commit touches the 34 files listed above before any accessor rewrite and so exceeds the 25-file delivery rule by construction; it cannot be halved without a dual registration. The research doc's Q10 asks the owner for a recorded exception (default) or a transitional facade; the plan proceeds under the default and records the file list in the PR description.
5. `refactor: consumers read MIR and CFG through unit accessors` (the accessor rewrite; may be split by module if it exceeds the size rule).
6. `perf(unit): lower units in parallel under rayon with UnitId merge order; determinism gate extended with unit-order permutation`.
7. `chore(unit): delete the legacy dominator relation, mir_body_compose, MirOutput::normalized and the metadata refresh walks`.

### W7. Persisted unit shards

**Goal.** After W7, a cold run writes one binary columnar shard per unit into the layer cache keyed by the unit's input digest; a warm run loads unchanged shards and re-lowers only units whose input digest changed; the layer cache's JSON payload path is not used for shards; manifests and the cross-unit index live in the SQLite store (Resolved Q3, Q1). G9's "second run under 30 s" for a one-file change is measured on this.

**Concrete changes.**

`crates/polint/src/analysis_neutral/unit/shard.rs` (new)

```rust
pub(crate) const UNIT_SHARD_SCHEMA: &str = "polint-unit-shard-1";

/// Wire format, little-endian, one file per unit. All sections are length-prefixed columns.
/// header: magic b"PLUS" | u32 schema_version | u32 unit_kind | u32 language
///         | u32 file_count | [u32 path_atom]*file_count
///         | u32 import_count | [Digest]*import_count        (imports' export digests, W8)
/// atoms:  u32 count | u32 total_bytes | [u32 offset]*count | bytes   (this shard's atom table; ids are shard-local)
/// keys:   u32 count | [u8 family, u32 parent, u32 part_range_start, u32 part_range_len]*count
///         | u32 part_count | [u32 label_atom, u8 tag, u32 value]*part_count
/// bodies, blocks, statements, terminators, operations, places, projections, place_types, unsupported, call_sites:
///         struct-of-arrays, one column per field, fixed-width where the field is fixed, else an offsets column plus a value column
/// cfg:    per body: u32 block_count | rpo column | idom column | ipdom column | CSR (offsets, targets, kinds)
/// domains (when present): observation columns
/// trailer: u64 fnv of everything before it | Digest content_digest
pub(crate) fn encode_unit_shard(graph: &UnitGraph, interner: &StableKeyInterner) -> Vec<u8>;
pub(crate) fn decode_unit_shard(bytes: &[u8], unit: UnitId, interner: &StableKeyInterner) -> Result<UnitGraph, ShardError>;
```
  - Atoms and keys are shard-local on disk; on load they are re-interned into the run's interner (W5's `intern_parts` guarantees the same `StableKeyId` for the same canonical bytes only within a run, so the shard stores its own atom table and the loader remaps). The canonical text is therefore reproducible from the shard without the run's interner.
  - Encoding is plain `Vec<u8>` writes; no serde for the payload. Decoding validates every offset against the length before use and rejects the shard (evict, recompute) on any inconsistency.

`crates/polint/src/analysis_kernel/incremental/layer_cache.rs`
- Add `write_bytes(&self, manifest: &LayerCacheManifest, payload: Vec<u8>)` and `read_bytes_validated(&self, key, validator)` that do not go through `serde_json` (the existing `write_json_bytes` at `:396` already takes bytes but computes a JSON-labelled digest; the shard path uses `payload_digest_for_bytes` directly and a `payload_encoding: "unit-shard-1"` field added to `LayerCacheManifest`, which bumps `LAYER_CACHE_MANIFEST_SCHEMA` to `polint-layer-cache-manifest-4`; `-3` is taken by W6's `LayerKind` change).
- Ceilings (`:31-32`): shards get their own `UNIT_SHARD_PAYLOAD_MAX_BYTES = 256 MiB`; the manifest ceiling stays.
- `LayerKind` (`analysis_api/digest/keys.rs:9`): add `UnitGraph`.

`crates/polint/src/analysis_kernel/incremental/keys.rs`
- `LayerKey` (`:51-64`) is reused per unit: `provider_id = "polint.unit_graphs"`, `input_digests = [unit.input_digest]`, `dependency_layer_digests = [import units' export digests]` (the gopls key shape, research doc 4.5), and `parameter_digest` = digest over everything else that changes the shard's bytes and maps to no other field, enumerated against every run-time input this plan itself says changes lowering output (round 5): the sorted `demanded_outputs` set the provider was run with (a shard lowered for a `calls` request carries no domain families, one for `control_flow` does; W3 made demand a run-time input); the resolved domain materialisation mode (`Full`, `SummaryInputs`, or absent) and the solver mode (intraprocedural or interprocedural), both of which W3 derives from `seed_demanded` and not from `demanded_outputs` (plan W3, `PlanDemand`): a `control_flow` and a `dataflow` request demand the same families and write different observation columns, so `demanded_outputs` alone would give them the same key (round 5); the per-function and per-run solver iteration caps as resolved for the run; the active `max_dominance_pairs()` value (`cfg/budget.rs:37-56` reads `POLINT_CFG_MAX_DOMINANCE_PAIRS` at run time and decides whether the closure or only tree edges exist; today it never entered a key because `polint.cfg` is `InMemoryDerived`, `provider.rs:1470`), the solver budget constants, and the shard schema label. `provider_version`, `schema_version`, `lifecycle_digest`, `config_digest` and `toolchain_digest` are filled as for every other layer. The unit's export digest is the digest of its exported symbol rows (names, kinds, spans, and every other `DefinitionFact` field the lowering reads, see the invariant below), computed from the symbol graph per unit, so a dependency's internal edit does not invalidate dependents.

  The soundness invariant this key rests on, stated explicitly (round 4): a unit's shard bytes are a function of exactly the fields in its `LayerKey`, its own files' contents (`input_digests`), its imports' export digests (`dependency_layer_digests`), the parameter digest (demanded outputs, materialisation and solver mode, iteration caps, dominance ceiling, schema label), and the version, schema, lifecycle, config and toolchain digests, and of nothing else in the database. W6 hands each unit's lowering the whole-program symbol graph read-only, so the invariant is a claim about what the lowering reads from it: only definitions of symbols the unit's own files reference, each of which resolves either inside the unit (covered by `input_digests`) or to an exported symbol of an import (covered by that import's export digest). Any read outside that set is a stale-reuse defect. Two reads must be checked against the export digest's field list before the shard schema is locked: `go_closure_capture_names` calls `definition_for_symbol` for every reference inside a closure and compares the definition's `primary_span` against the closure span (`go/mir/lower.rs:732-767`), so the export digest must cover `primary_span` and `is_primary` as well as name and kind; and `matching_function` and `enclosing_function` read `FunctionFact` rows, which are per-file and therefore inside `input_digests`. The W7 mutation matrix (test (d)) samples this invariant; it does not establish it, so the W7 PR states it in the shard module's doc comment and adds a test that lowers a unit twice with an unrelated unit's non-exported body changed in between and asserts byte-identical shard bytes, and a second that changes an import's exported definition span and asserts a miss.

`crates/polint/src/analysis_kernel/store/`
- New migration adding `unit_shards (generation_id, unit_path, unit_kind, language, input_digest, export_digest, payload_digest, layer_key_digest, file_count, body_count)` and `unit_imports (generation_id, unit_path, import_path)`; schema version v6. The store remains manifest-and-index only; payloads stay in the layer cache (Resolved Q3).
- The SUM-03 benchmark decides blob-in-cache versus adjacent content-addressed file before this layout is locked: a `polint-bench` case that writes and reads the 885-file scope's shards both ways and reports DB size, WAL growth, and read latency. The plan assumes blob-in-cache and switches only if the benchmark shows a read-latency regression above 20 percent.

`crates/polint/src/analysis_neutral/unit/provider.rs`
- Before lowering, for each unit in order: compute `LayerKey`, `read_bytes_validated`; on hit, decode and skip lowering; on miss, lower, encode, `write_bytes`. Hits and misses are counted in `cache_stats` and reported as `unit_shards.hit` and `unit_shards.miss` counters on the stage row.
- Stale-reuse safety: the manifest's `dependencies` list the unit's files (content digests) and its imports' export digests; the existing invalidation machinery (`incremental/invalidation.rs`) applies.

**Interfaces.** Above.

**Test strategy.**
- Gating: `analysis_kernel::incremental::layer_cache` tests; store migration tests (`analysis_kernel/store/migrations.rs` tests, sentinel and version checks); `analysis_kernel::incremental::invalidation` tests.
- New tests: (a) encode/decode round trip is byte-identical (`encode(decode(encode(g))) == encode(g)`) on every fixture unit; (b) a decoded unit graph produces the same per-unit digest and the same canonical key texts as a freshly lowered one; (c) corrupted shard (truncated, bad trailer, out-of-range offset) is rejected and recomputed, never panics; (d) the stale-reuse mutation matrix report 03 names (VAL-04): edit a file in unit A, assert A and its dependents miss and every other unit hits; edit a non-exported function body in A, assert dependents still hit (export digest unchanged); change `tsconfig.json`, assert the whole project misses; change the polint version, assert every shard misses; run `--cap calls` then `--cap control_flow` on the same cache, assert every unit misses on the second run and its shard carries the domain families; run `--cap control_flow` then `--cap dataflow` on the same cache and unchanged tree, assert every unit misses (materialisation and solver mode differ while `demanded_outputs` does not; round 5); run with `POLINT_CFG_MAX_DOMINANCE_PAIRS=0` after a default run, assert every unit misses; and, as the standing cache-key rule, one forcing test per `parameter_digest` dimension (demanded outputs, materialisation mode, solver mode, iteration caps, dominance ceiling, shard schema label) that changes only that dimension and asserts a miss, so that a dimension added later without a test fails review by construction; (e) warm output byte-identical to cold on the fixture set and on the 885-file scope (cold, then warm, then `diff` of ai-friendly stdout).
- Invariant I1: every downstream digest and the diagnostics digest identical between cold and warm runs; the `polint.unit_graphs` provider digest identical between cold and warm (it is computed from per-unit digests, which are identical by test (b)).
- Must not move: first-run rows. Must move: second-run `polint.unit_graphs` `elapsed_ms` near zero for unchanged units.

**Verification probe (G9, first half).**

```sh
probe warm-1 calls <885-file scope>                              # cold: writes shards
cp -r .polint/cache "$POLINT_GATE_OUT/cache-after-cold"
KEEP_CACHE=1 probe warm-2 calls <885-file scope>                 # warm: the helper keeps the cache when KEEP_CACHE is set
grep 'provider="polint.unit_graphs"' "$POLINT_GATE_OUT/warm-2.stderr" | grep -oE 'elapsed_ms=[0-9]+|unit_shards\.[a-z]+=[0-9]+'
diff "$POLINT_GATE_OUT/warm-1.stdout" "$POLINT_GATE_OUT/warm-2.stdout" && echo warm-identical
```

**Dependency and risk.** Depends on W6. Top failure mode: stale reuse (a shard served for inputs that changed) or write cost dominating cold runs. Detection: the mutation matrix (d) for the first; the stage row's `unit_shards.write_ms` counter against the research doc's kill criterion (shard writing above 20 percent of cold wall) for the second.

**Commit shape.** Four commits: (1) `feat(shard): unit shard encoding and decoding with round-trip and corruption tests` (no I/O); (2) `feat(cache): raw-bytes layer payloads with an encoding label; manifest schema 3; UnitGraph layer kind`; (3) `feat(store): unit_shards and unit_imports tables, schema v6, with the SUM-03 benchmark case`; (4) `feat(unit): read and write unit shards around lowering; hit and miss counters; mutation matrix fixtures`.

### W8. Cross-unit demand joins for calls, RTA, reachability and data flow

**Goal.** After W8, `polint.solver` (Go RTA, TS points-to), `polint.refined_calls`, `polint.reachability` and `polint.data_flow` read unit graphs through a cross-unit index instead of whole-program `Vec`s, process Go units in import order and TS projects in reference order with intra-project SCCs handled by the existing closure, and use persisted per-unit summaries at unit boundaries; the four whole-program CFG derived families and the call-site `Vec` that W6 kept are deleted; the final G6 threshold is met and G9 asserts that only changed units and their dependents recompute.

**Concrete changes.**

`crates/polint/src/analysis_neutral/unit/index.rs` (new)

```rust
/// Cross-unit lookup built once per run from the unit graphs and the symbol graph.
pub(crate) struct CrossUnitIndex {
    functions_by_qualified: HashMap<Arc<str>, Vec<(UnitId, BodyIdx)>>,     // Go: import path + name (+ receiver); TS: module path + name
    functions_by_file_span: HashMap<(FileId, u32, u32), (UnitId, BodyIdx)>,
    call_sites_by_file_span: HashMap<(FileId, u32, u32), Vec<CallSiteId>>,
    exported_symbols: HashMap<SymbolId, (UnitId, BodyIdx)>,
    unit_export_digest: Vec<Digest>,                                        // by UnitId
}
impl CrossUnitIndex {
    pub(crate) fn build(db: &impl AnalysisHost) -> Self;
    pub(crate) fn function(&self, id: FunctionId) -> Option<(UnitId, BodyIdx)>;
    pub(crate) fn callee_candidates(&self, site: CallSiteId) -> &[(UnitId, BodyIdx)];  // after refined_calls, resolved targets
}

/// Per-unit interprocedural summary, persisted next to the shard (W7 manifest gains `summary_digest`).
pub(crate) struct UnitSummaries {
    pub(crate) unit: UnitId,
    pub(crate) call_effects: Vec<SummaryFact>,        // the existing summary_* families, unit-scoped
    pub(crate) memory: Vec<SummaryFact>,
    pub(crate) tito: Vec<SummaryFact>,
    pub(crate) events: Vec<SummaryEventFact>,
    pub(crate) rta_seed: RtaSeed,                     // address-taken functions, instantiated types, dynamic dispatch sites, unit-local
    pub(crate) digest: Digest,
}
```

- `polint.direct_summaries` and the SCC closure (`summaries/closure.rs:87` `close_summaries_by_scc`): the SCC schedule becomes two-level: units in `UnitSet::order`, and within a unit the existing function-level Tarjan SCCs; callee summaries from already-processed units are read from `UnitSummaries` (loaded from the cache on a warm run) rather than recomputed. Cross-unit recursion is closed the same way for both languages: a TS project cycle or a Go test-variant import cycle (W6, Go bullet) is one SCC and one closure step, as today; non-test Go units form a DAG and are singletons.
- `polint.solver`: the Go RTA policy (`analysis/solver/policy.rs`) seeds from `UnitSummaries::rta_seed` accumulated in unit order; propagation is the existing fixpoint over the whole seed set (RTA is a single global set, research doc 4.11; it is cheap). The TS points-to policy is unchanged in algorithm and reads allocations and property facts through the unit accessors.
- `polint.refined_calls`: the Go semantic join (W1's indexes) reads `CrossUnitIndex::functions_by_file_span`; the sidecar rows are partitioned by unit via `unit_of_file` so a warm run joins only the units that re-lowered plus their dependents.
- `polint.reachability` and `polint.data_flow`: the ICFG (`ifds/mod.rs`) is built per demanded root set from unit graphs lazily: `Icfg::demand(db, index, roots)` adds a unit's function CFGs when a call edge reaches into it; the IFDS search (`find_taint_paths`, `ifds/mod.rs:191`) is unchanged in algorithm; `data_flow/local.rs` and `summary_edges.rs` read through the unit accessors. The whole-program `cfg_reachability`, `cfg_dominators`, `cfg_postdominators`, `cfg_control_dependence` and `call_sites` `Vec`s kept by W6 are replaced by unit-scoped accessors plus a `ControlFlow` view adapter in `policy_queries.rs:1383-1418` that answers `answer_for_blocks` (`:1406-1417`) through W4's `DomTree` API over `FunctionCfg::idom`, `ipdom` and `pdom_vacuous`. What the adapter must reproduce, stated per materialisation regime (round 3; the earlier sentence claimed the vacuous rule alone reproduces today's answers, which is false at 314 files and above): today `answer_for_blocks` builds `by_source` from the emitted `cfg_dominators` or `cfg_postdominators` rows (`BlockRelation::new`, `:1370-1381`; `dominators`, `postdominators`, `:1383-1397`) and answers `reaches(event, candidate)`, a breadth-first walk over those rows (`:1420-1433`). For a non-vacuous event block the walk reaches the same set in both regimes, the transitive set under `Full` and the closure of the `idom` chain under `ImmediateOnly`, so tree ancestry (`DomTree::dominates`) reproduces it in both. For a vacuous event block (exit-unreachable, post-dominance only) the two regimes differ: under `Full` its bucket is the whole universe (`cfg/derived.rs:346-356`, `:156-175`), so the answer is `Holds` for every candidate; under `ImmediateOnly` its bucket is the single `immediate_relation` pick (`:438-462`, the smallest other vacuous block), so the walk reaches only the smallest two vacuous blocks of the function and the answer is `Holds` for those two (minus the event itself) and `DoesNotHold` for every other candidate. W4 keeps the budget selection (`cfg/provider.rs:109-116`), the bound first trips at 314 files (benchmark report 5.5.1), and W8's probe cells (885 files, the full backend) are therefore `ImmediateOnly` cells while the 45-file and fixture cells are `Full`. The adapter therefore takes the run's `DominanceMaterialization` and answers a vacuous event block by the regime's rule: `DomTree::dominated_by_answer(candidate, event, materialization)` returns `Holds` for all candidates under `Full`, and under `ImmediateOnly` `Holds` iff the candidate is one of the two smallest vacuous blocks of the function other than the event; non-vacuous event blocks use `dominates` in both regimes; and in both regimes the `candidate == event` early return at the top of `answer_for_blocks` (`policy_queries.rs:1407-1409`) is retained ahead of the regime rule, so a guard and its guarded call in the same basic block answer `Holds` whether or not that block is vacuous or among the two smallest (round 4). That reproduces today's answers byte for byte on every cell, which is W8's invariant. The regime dependence itself is a pre-existing quirk (post-dominance of a block that cannot reach an exit is vacuous, and a guard answered `Holds` from it is not a proof), and the honest answer is `DominanceAnswer::Unknown` with a new reason for vacuous event blocks in both regimes; that is a behaviour change on every scope with an exit-unreachable guarded block, is not made in W8, and is recorded as a follow-up decision with its own diagnostic-delta measurement. Tests: the guard tests in `docs/facts/control-flow.md:94-98`, plus a vacuous-block guard fixture (`for {}` with no return, guarded by a post-dominance policy) asserted under both `POLINT_CFG_MAX_DOMINANCE_PAIRS=0` and a bound low enough to trip on the fixture, each against the pre-W8 answer captured as a golden.
- Persisted `UnitSummaries` key, enumerated the same way as W7's shard key (round 5): the unit's shard `LayerKey` digest, the sorted summary digests of its imports (transitively, through the import order), the demanded summary families, the resolved domain materialisation and solver mode (the `dataflow` path's interprocedural observations reach `summary_control`), the summaries closure budgets (`summaries/closure.rs:51`, `:116-125`), and the summary schema label; one forcing test per dimension asserts a miss, and the manifest's `summary_digest` is recomputed whenever any of them changes.
- Recompute set on warm runs: a unit is "touched" when its shard missed, its summaries key missed, or any import's export digest changed; `direct_summaries`, `solver`, `refined_calls`, `reachability`, `data_flow` and `evidence` run over touched units plus every unit whose summaries depend on a touched unit (transitive dependents in the import graph), and merge with persisted results for the rest. The stage rows gain `units.touched` and `units.total` counters.

**Interfaces.** Above, plus `DomTree::dominated_by_answer(&self, candidate: BasicBlockId, event: BasicBlockId, materialization: DominanceMaterialization) -> bool` on W4's tree. The `ControlFlow` SDK view's behaviour is unchanged (I2) in both materialisation regimes; its implementation reads the dominator trees through the `DomTree` queries with the regime-dependent vacuous rule above, after the reflexive `candidate == event` short-circuit.

**Test strategy.**
- Gating: `analysis_neutral::summaries` (closure and builder tests), `analysis::solver` policy tests, `analysis_neutral::refined_calls`, `reachability`, `data_flow`, `evidence`, `ifds` tests; `policy_queries` guard and reach tests; the golden corpus; the capability matrix; the L4 seed probes at their current counts (4/10 Go, 4/10 TS, twins 15/20 and 18/20, `research/ts-type-sidecar/measurement.md` section 4 numbers as measured on `82a3c129`; they must not regress); the determinism gate.
- New tests: (a) a three-unit diamond fixture where a call from C reaches A through B, asserting `refined_call_edges`, `call_reachability` and a taint path are identical to the pre-W8 whole-program result (the pre-W8 expected output is committed as the fixture's golden); (b) warm-run recompute set: edit unit B, assert `units.touched == {B, C}` and that A's summaries were loaded, with output byte-identical to a cold run; (c) a TS project cycle fixture asserting the closure handles it as one SCC step.
- Invariant I1: every digest identical between cold and warm; the diagnostics oracle and ai-friendly stdout (section 5.1) identical; the guard-policy answers of `docs/facts/control-flow.md:94-98` identical in both dominance regimes (research doc section 8, "must not move" for W8).
- Must move: `polint.refined_calls`, `polint.solver`, `polint.reachability`, `polint.data_flow` rows bounded by touched units on warm runs; on cold runs their `elapsed_ms` must not regress against the post-W6 run.

**Verification probe (G6 final, G9 second half).**

```sh
probe full-w8 calls <core>                                       # G6 at the final threshold: exit 0, wall under 300 s, tree peak under 12 GB
python3 .scale-envelope/stages.py "$POLINT_GATE_OUT/full-w8.stderr"             # all selected providers have a row
# G9: cold, then edit one Go file in a leaf package, then warm
probe g9-cold calls <core>; <edit one file>; polint unknowns --cap calls <core> ... (cache kept)
grep -oE 'units\.touched=[0-9]+' "$POLINT_GATE_OUT/g9-warm.stderr"            # equals the edited unit plus its dependents
```

**Dependency and risk.** Depends on W7 (summaries persisted per unit) and on the Phase 67 manifest fields (`summary_digest` on the shard manifest, added here). Top failure mode: a cross-unit edge that the whole-program analysis found and the unit-ordered analysis misses (a callee summary read before it was computed because the unit order was wrong, or a TS project reference not captured). Detection: test (a) on the diamond fixture, the L4 probes, and a cold-run digest comparison against the post-W6 run on the 885-file scope, where every downstream digest must be identical.

**Commit shape.** Five commits: (1) `feat(unit): cross-unit index` (no consumer); (2) `feat(summaries): unit-ordered SCC closure with persisted per-unit summaries`; (3) `feat(solver): RTA and points-to over unit graphs and unit summaries`; (4) `feat(dataflow): demand-built ICFG over unit graphs; refined-calls join through the cross-unit index`; (5) `chore(unit): delete the whole-program derived CFG and call-site vectors; ControlFlow view reads the dominator tree`.

### W9. Envelope enforcement and the local acceptance gate

**Goal.** After W9, the resource envelope bounds wall clock as well as memory and checks memory per unit, degrading with a reported diagnostic instead of a kill; the G6 and G8 probes exist as committed shell scripts the owner runs locally on a machine of his choice; each run's results are recorded in a committed report of counts and timings only. No CI of any kind runs the gate (Resolved Q6).

**Concrete changes.**

`crates/polint/src/analysis_kernel/resource.rs`
- `ResourceEnvelope` gains `wall_budget: Option<Duration>` from `POLINT_WALL_BUDGET_MS` (unset means none), checked in `observe` (`:97`) alongside the memory sample; a trip records `ResourceTripKind::{Memory, WallClock}` and the diagnostic (`budget_diagnostic`, `:133`) names the kind.
- Per-unit check: `derive_unit_graphs_with_cache_stats` (W6) calls `envelope.observe_unit(unit_id)` after each unit; when the ceiling is crossed mid-provider, remaining units are skipped and recorded as `budget_exceeded` with their paths in the diagnostic's evidence (repo-relative paths are fine in a local report; they are not consumer source).
- `POLINT_MEMORY_CEILING_MB` semantics unchanged (`:24`).

`scripts/deep-gate/` (new, committed)
- `probe.sh`: the section 5.1 function as a script: `probe.sh <tag> <cap> <paths...>`, wraps the run in `.scale-envelope/rssrun.py`, writes `stdout`, `stderr` and the sampler's `timeline.json` under `$POLINT_GATE_OUT` (default `/tmp/polint-gate`), never under the repository.
- `fact_rows_dump`: an ignored test entry in the `polint-eval` harness (`crates/polint-eval/src/harness/fact_rows_dump.rs`, alongside `observed.rs`; the file holds the dump function and a `#[cfg(test)] mod tests` with `#[test] #[ignore] fn dump_fact_rows()`, the same layout as `bench/runner.rs:186`, so the filter path is `eval::fact_rows_dump::tests::dump_fact_rows`), run as `cargo test -p polint --lib --all-features --locked --release eval::fact_rows_dump::tests::dump_fact_rows -- --exact --ignored --nocapture` with `POLINT_FACT_ROWS_REPO`, `POLINT_FACT_ROWS_PATHS`, `POLINT_FACT_ROWS_CAP` and `POLINT_FACT_ROWS_OUT` in the environment, the same shape as the perf harness's `POLINT_PERF_CHILD_REPO` entry (`polint-eval/src/harness/bench/runner.rs:189`). It has to be a test entry: the harness is compiled into `polint` only under `cfg(test)` (`crates/polint/src/lib.rs:32-34`), `polint-eval` declares no dependency on `polint` and cannot be a binary, and the building blocks are crate-private (`FactMetaStore::family_rows`, `analysis_api/metadata.rs:460`; `metadata_debug_json_for_test`, `analysis_kernel/debug.rs:29`). It runs the kernel in process on the checkout with the given capability and writes one file per fact family (iterating a new `FactFamily::ALL`) with the sorted (canonical stable-key text, `payload_digest` hex) pairs from `FactMetaStore::family_rows`, plus, for the four `SummaryFact` families, a third column holding the plaintext `SummaryFact::payload_digest` joined by `SummaryId` (I1b, round 3); "before" and "after" are two runs of the entry, one from a worktree at the branch base and one from the checkout under test, and `gate.sh` diffs the two output directories per family. The dump runs on a `--release` test-profile build rather than the release binary the probes run; the fact rows are the same in both because they are a function of the inputs and the code, not of the build profile, and the determinism gate already relies on that. It is the I1b oracle and W5's obligation 1 dump; it lands in slot 1 so W3's first commit can use it.
- `gate.sh`: runs the G-matrix cells of section 5.2 that apply to the checkout it is pointed at (`POLINT_GATE_REPO`, `POLINT_GATE_SCOPES` as a list of `label=path=cap` triples), compares digests with `.scale-envelope/digests.py` and fact rows by diffing a `fact_rows_dump` output directory against a `before` directory when given, checks the per-workstream expected-move list (a small allowlist file naming the provider ids and families that may differ, and for `summary_control` the parts-column rule of I1b), and prints one Markdown table per cell. Two blind spots of `digests.py` (`:18-24` iterates the before capture's keys only; the stage row prints `digest="-"` for a provider that ran and failed, `analysis_kernel/mod.rs:338`, which the regex accepts as a value) are closed here: `gate.sh` also asserts that the set of providers with a `stage done` row in the after capture equals the cell's expected set (so a new provider such as W6's `polint.unit_graphs` is checked for presence) and that no `digest="-"` appears in either capture.
- `report.py`: folds the run directory into the report format of section 6 and writes `research/strategy/plans/gate-reports/<date>_<host-label>.md`; it refuses to include any line from stdout or stderr other than the stage rows, the resource-budget diagnostic count, and the `rssrun.py` JSON summary line, so no consumer text can leak.
- `.scale-envelope/digests.py` and `stages.py` are reused, not copied.

`research/strategy/plans/gate-reports/README.md`: the report format (section 6) and the rule that a report is written by `report.py` only.

No workflow file is added or changed.

**Interfaces.** `ResourceEnvelope::observe_unit(&mut self, unit: UnitId)`; `ResourceTrip { kind: ResourceTripKind, after_provider, after_unit: Option<UnitId>, observed, ceiling, source }`.

**Test strategy.**
- Gating: `analysis_kernel::resource` tests; the kernel tests that assert the `polint/resource-budget` diagnostic path; `polint unknowns` budget-row tests.
- New tests: (a) a wall budget of 1 ms on a fixture trips after the first provider and the run finishes with the diagnostic; (b) a memory ceiling below the fixture's unit-graph size trips mid-provider and the skipped units are listed; (c) `report.py` unit test: a synthetic stderr containing a non-stage line is rejected.
- Invariant I1: rows under a normal ceiling must not move; the diagnostic appears only under a low ceiling.

**Verification probe (G10).**

```sh
POLINT_MEMORY_CEILING_MB=8192 scripts/deep-gate/probe.sh g10 calls <core>
grep -c '"family": "ResourceBudget"' /tmp/polint-gate/g10.stdout   # 1; polint unknowns surfaces the polint/resource-budget diagnostic as a budget_exceeded row with family ResourceBudget (collect.rs:242-262); the run exited 0 rather than being killed
grep -oE 'after `polint\.[a-z_.]+`' /tmp/polint-gate/g10.stdout               # the provider the ceiling tripped after (resource.rs:133-147); the degraded providers are those without a stage done row in g10.stderr after it
POLINT_WALL_BUDGET_MS=60000 scripts/deep-gate/probe.sh g10-wall calls <core>
```

**Dependency and risk.** The envelope half depends on W6 (units to check); the scripts and report format depend on nothing and should land early so W1's probes already use them. Top failure mode: a report that leaks consumer text. Detection: `report.py`'s allowlist and its unit test; review of the first committed report line by line.

**Commit shape.** Two commits: (1) `feat(gate): local probe and gate scripts with a hygiene-checked report writer` (early, after W0); (2) `feat(kernel): wall-clock budget and per-unit memory check in the resource envelope` (after W6).

## 4. Sequencing detail and parallel work on this host

| Slot | Work | Parallel with | Serialised behind |
|---|---|---|---|
| 1 | W0; W9 scripts, `fact_rows_dump` harness entry and report format; capture the branch-base `before` stderr, stdout and fact-row dumps (section 2 baseline rule) for excalidraw (`dataflow`, once with the default bound and once with `POLINT_CFG_MAX_DOMINANCE_PAIRS=0` at `POLINT_GATE_TIMEOUT=600`), 45-file (`calls`, `dataflow`), 885-file (`calls`, `control_flow`) and the timed-out full backend from a worktree at the branch base; every capture is a cold cell with a fresh cache root (section 5.1) | each other | nothing |
| 2 | W1 commits 1 to 4 | W4; W2 commit 1 (separate worktrees, `CARGO_BUILD_JOBS=4` each, one build at a time) | 885-file and full-backend probes run one at a time |
| 2 | W2 commit 1 (extraction) | W1, W4 | as above |
| 2 | W4 commits 1 to 2 | W1, W2 commit 1 | as above |
| 2b | W2 commit 2 (six sites) | W4 | W1 commit 4 merged (shared file `refined_calls/provider.rs`) |
| 3 | W3 closure commits 1 to 2 | W2, W4 | W1 commit 1 (indexed references) |
| 3 | W3 solver commit 3 | nothing | W1 complete |
| 4 | W5 commits 1 to 5 | nothing | W1, W2, W3, W4 merged |
| 5 | W6 commits 1 to 7 | nothing | W5 merged |
| 6 | W7 commits 1 to 4 | W9 envelope commit | W6 merged |
| 7 | W8 commits 1 to 5 | nothing | W7 merged |
| 8 | Final gate matrix (section 5.2) with `report.py` | nothing | everything merged; no build running |

Capacity notes: a pre-W5 full-backend `calls` probe reaches 14 to 18 GB tree peak (benchmark report); do not run it while any `cargo build --release` is in flight. After W5 the probe should fit alongside a build, but the rule stays until G6 has been measured. The 45-file scope and excalidraw are safe to run concurrently with a build.

## 5. Definition of done

### 5.1 Probe helper

Memory is measured two ways and every gate names which one it binds. The tree peak comes from `.scale-envelope/rssrun.py`, the committed sampler that walks `/proc/<pid>/task/<pid>/children` from the probe's pid every 200 ms, sums `VmRSS`, prints `{"peak_rss_bytes", "peak_rss_gb", "wall_s", "exit_code"}` as JSON on stderr, and sets `RLIMIT_AS` so a runaway probe fails with an allocation error instead of an OOM kill (`rssrun.py:22-58`, `:75-107`). The polint-process peak is the `peak_rss_mb` field of the last stage row (`getrusage(RUSAGE_SELF).ru_maxrss`, `measure.rs:27-32`). `/usr/bin/time -v` is not used for memory because its "Maximum resident set size" is `ru_maxrss` of the timed process or of its largest single waited-for child, never a sum, and the Go sidecar is a grandchild of `polint`. The benchmark report's tree peaks were sampled the same way at 150 ms and are comparable.

The `--as-limit-gb` guard is an `RLIMIT_AS` ceiling set in `preexec_fn` (`rssrun.py:72-76`), so it bounds virtual address space, not resident memory, and it is inherited by the Go sidecar. A cell that hits it does not look like an out-of-memory kill: the `polint` process prints `memory allocation of N bytes failed` and aborts (SIGABRT, exit status 134 from the shell, `-6` in the sampler summary), and a Go child prints `fatal error: runtime: out of memory` and exits 2, whereas the kernel's OOM reaper delivers SIGKILL (137, `-9`). The report records the exit status and the last stderr line so the two are told apart. Pre-W5 full-backend cells, which the benchmark measured at 14 to 18 GB tree RSS with a Go child that reserves address space liberally, run with `POLINT_GATE_AS_GB=40`; the default of 28 applies to every other cell (round 3).

Cache state is part of the cell definition. The cache root has more children than `analysis` and `layers`: `sidecar` (`cache/mod.rs:177-187`), `semantic-store`, `derived`, `extensions-target` and `review` (`:470-493`), and the Go semantic sidecar persists its rows under `sidecar` keyed by `go_semantic_sidecar_cache_key` (`go/semantic/cache_key.rs:32`, used at `go/semantic/client.rs:104`), so wiping only two directories leaves a run that may or may not pay the sidecar's 25 to 27 s and roughly 7 GB depending on what an earlier run left behind. A cold cell therefore runs with `POLINT_CACHE_DIR` (`cache/mod.rs:14`, `:426`) pointing at a fresh directory, which moves the whole root including `sidecar`, and every threshold below that names a cold cell (G2, G3, G4, G5, G6, G8) binds with the sidecar running and its heap in the tree; a warm cell (`KEEP_CACHE=1`) reuses the previous cell's directory. A before/after pair is always captured in the same state.

Three rules every gate row refers to, stated once here (round 5, single-sourcing):

- Provider-count rule. `digests.py` prints `N/N provider output digests identical` with N the number of providers that had a `stage done` row in the before capture; N is a function of the request and of the tree state, never a constant of the gate. On the branch base a Go `calls` cell has 21 rows (`providers_enabled_by_boolean_gates`, `provider.rs:1031-1068`) and a `dataflow` cell 23 (`data_flow` and `evidence`, `:1065-1067`); after W3 a `calls` cell has 20 (`polint.abstract_domains` is not selected); after W6 four ids collapse into `polint.unit_graphs` and the `dataflow` cell gains `polint.interprocedural_domains`, so N changes again. A row is printed only for a provider that ran (`analysis_kernel/mod.rs:324-341`). The committed report records the observed N per cell; "every selected provider has a stage row" means the after capture's row set equals the enabled set the closure computed for that request on that tree (`scheduled_order_for`, `provider.rs:1133`), not a number.
- Diagnostics oracle. "Diagnostics digest identical" means: `polint check --format json --fail-on none` on every `examples/*` case, normalised as `crates/polint/tests/golden.rs::normalize_report` does (`:299-324`: volatile path strings stripped, diagnostics sorted by fingerprint, keys sorted, compact serialisation), is byte-identical; the executable form is the golden characterisation test, `cargo test -p polint --test golden --locked`, plus `gate.sh`'s SHA-256 over the same normalised text for the report. The JSON path is the only report path that strips measurements (`ProviderOutcomeRow::without_measurements`, `diagnostics/mod.rs:254-261`, applied at `:1022`). The ai-friendly stdout oracle is `render_ai_friendly_stdout` (`diagnostics/mod.rs:703-766`), which is timestamp-free and prints the constant path `.polint/output/latest.json` (`cli/mod.rs:45`, `:3509`); it is diffed byte for byte. The ai-friendly JSON file under `.polint/output` is never compared byte for byte: `ai_friendly_summary` clones the provider rows with `elapsed_ms` and cache counters (`:581`), so two runs always differ there.
- Counters on the stage row. Provider counters (`ProviderRunResult::counts`) are printed today only in the `providers` array of `polint check` (`provider_outcome_rows`, `outcome.rs:730-732`), never on the `stage done` tracing row (`analysis_kernel/mod.rs:327-341`) and never by `polint unknowns`. W9 commit 1 therefore adds a `counts=` field to the `stage done` row (sorted `key:value` pairs), which is what every `grep -oE '...=[0-9]+'` over probe stderr in this document reads (`unit_shards.hit`, `unit_shards.miss`, `unit_shards.write_ms`, `units.touched`, `units.total`). A provider that gains counters also gains a row in the `polint check` providers array from then on (`polint.unit_graphs` from W7); the array's shape is frozen by I2, its row set is not.

```sh
export GOROOT=/opt/data/home/.local/share/go
export PATH=$GOROOT/bin:$HOME/.local/bin:$HOME/.cargo/bin:$PATH
export RAYON_NUM_THREADS=12 POLINT_JOBS=12 GOMAXPROCS=12 GOFLAGS=-p=12
export RUST_LOG=polint=debug
export POLINT_GATE_OUT=${POLINT_GATE_OUT:-/tmp/polint-gate}; mkdir -p "$POLINT_GATE_OUT"
probe() { # $1 tag, $2 cap, $3... paths; POLINT_GATE_TIMEOUT (s, default 300); KEEP_CACHE=1 for warm cells
  tag=$1; cap=$2; shift 2
  [ -z "${KEEP_CACHE:-}" ] && POLINT_GATE_CACHE=$(mktemp -d /tmp/polint-gate-cache.XXXXXX)   # cold: fresh root, sidecar included
  export POLINT_GATE_CACHE
  POLINT_CACHE_DIR="$POLINT_GATE_CACHE" python3 .scale-envelope/rssrun.py --label "$tag" --as-limit-gb "${POLINT_GATE_AS_GB:-28}" \
    --timeout "${POLINT_GATE_TIMEOUT:-300}" --timeline "$POLINT_GATE_OUT/$tag.timeline.json" -- \
    polint unknowns --cap "$cap" "$@" > "$POLINT_GATE_OUT/$tag.stdout" 2> "$POLINT_GATE_OUT/$tag.stderr"
  echo "exit=$?"
  grep '"peak_rss_gb"' "$POLINT_GATE_OUT/$tag.stderr" | tail -1          # tree peak (sum of VmRSS over the process tree, 200 ms samples) and wall
  grep "stage done" "$POLINT_GATE_OUT/$tag.stderr" | tail -1 | grep -oE 'peak_rss_mb=[0-9]+'   # polint-process peak (getrusage RUSAGE_SELF)
  python3 .scale-envelope/stages.py "$POLINT_GATE_OUT/$tag.stderr"
}
```

### 5.2 Gate checklist

| Gate | Closes | Command | Pass condition | Rows that must not move |
|---|---|---|---|---|
| G0 | W0 | `CARGO_HOME=$(mktemp -d) polint check --profile core --fail-on none <paths>` after one publish from a different cargo home | under 10 s; no `cargo`/`rustc` child | all |
| G1 | W1, W2, W4, W5 (I1a) | `python3 .scale-envelope/digests.py before/<cell>.stderr after/<cell>.stderr` on excalidraw (`dataflow`), 45-file (`calls`, `dataflow`), 885-file (`calls`) | `N/N provider output digests identical` with N per the provider-count rule (section 5.1); no `MISSING`; the diagnostics oracle (section 5.1) passes | every `digest=` |
| G1b | W1 | temporary `polint::probe` step rows in `lower_go_mir` on the 885 and 1,588-file scopes (`POLINT_GATE_TIMEOUT=900` for the latter) | the four superlinear terms of research doc section 3.2 dominate the stage before W1 and are not visible after; their order is recorded | n/a |
| G1c | W3, W6 (I1b); W5 obligation 1 | the `eval::fact_rows_dump::tests::dump_fact_rows` ignored test entry run from the baseline worktree the section 2 baseline rule assigns and from the checkout under test on the G1 cells; `diff -r` per family; `diff` of stdout | every dumped family byte-identical except those the workstream lists (W3 on `calls`: `domain_observations`, `domain_events` absent, `summary_control` key set identical and each differing line's parts column changes only as the I1b line rule permits, count reported (round 5: the earlier cell restated the round-3 single-shape rule); W6: none); diagnostics digest and stdout identical | all dumped fact rows; aggregated and row-less families through I1a |
| G2 | W1, W2 | `probe s885 calls <885-file scope>` | wall under 60 s; polint-process `peak_rss_mb` under 7,500 (today 8,143 peak, 7,113 retained, benchmark report A.4; W1 and W2 remove the transient, W5 the floor); tree peak reported, expected near today's 13.7 GB: that peak is set while the sidecar (6,846 MB) is alive, during the early stages, and not at the polint-process high-water mark of row 17 (8,143 + 6,846 would be 14,989, above the measured tree peak, so the two do not coincide); W1 and W2 change neither component | `digest=`, `facts`, `keys` |
| G2b | W5, W6 | `probe s885 calls <885-file scope>` with `rssrun.py --timeline "$POLINT_GATE_OUT/s885.timeline.json"` (`rssrun.py:62`, `:112-114`); polint-process and sidecar-process RSS read per 200 ms sample | polint-process RSS under 5,265 MB at every sample at which the Go sidecar process is present (today about 6,843 MB by subtraction, 13,689 tree minus 6,846 sidecar); this is the necessary condition for G6's 12 GB tree ceiling, stated in the research doc section 7, measured here before the full backend is attempted | the tree peak on this cell is reported, not bound |
| G3 | W1, W2, W5, W6 | `probe full-mir calls <core>` (first post-W1 run with `POLINT_GATE_TIMEOUT=1800` to match the benchmark's over-budget baseline); read the `polint.unit_graphs` row (pre-W6: `polint.semantic_mir`) | stage under 60 s; `rss_delta_mb` under 3,000; `key_mb` growth under 1,500 | the nine `digest=` values W6 leaves unaffected; all `digest=` pre-W6 |
| G4 | W4 | same run, `polint.cfg` step rows (pre-W6) or the unit-graphs CFG sub-rows (post-W6); `POLINT_CFG_MAX_DOMINANCE_PAIRS=0` on excalidraw (`dataflow`, `POLINT_GATE_TIMEOUT=600`, slot-1 capture) and the 45-file scope, where the default bound already yields the full relation; the fixture differential for the 885-file shape | stage under 30 s; dominators step under 5 s; with the bound disabled the dominator and post-dominator pair sets are identical to the legacy relation's, vacuous exit-unreachable rows included; with the default bound the tree-edge sets are identical; `cfg_control_dependence` rows identical in both modes | every `digest=` in both modes |
| G5 | W3 | `probe s885-calls calls <885-file scope>`; `probe s885-cf control_flow <885-file scope>`; `digests.py` and `fact_rows_dump` against the branch-base captures (W3's baseline, section 2) | `polint.abstract_domains` row absent on `calls` (0 rows), present on `control_flow`; on `calls`, `digests.py` reports `MISSING polint.abstract_domains`, `DIFFER` for exactly `direct_summaries`, `type_value_alias`, `semantic_graph`, `solver`, `refined_calls`, identical for the other 15; the fact-row dump identical for every family except the two absent domain families and `summary_control` (same key set; every differing line's parts column changes only as the I1b line rule permits; count reported); no `dependency_blocked` row in the sealed outcomes (the tracker filter, asserted in process by test (h)); `polint unknowns` stdout identical; `grep -c 'provider="polint.abstract_domains"'` on the probe stderr is 0 on `calls` and 1 on `control_flow`; decisive clause: the `digests.py` pattern is `DIFFER`, not `MISSING`, for those five providers, because a blocked or ceiling-tripped `abstract_domains` would leave all five `dependency_blocked` and `MISSING` (W3 test strategy, commit 2; round 5: the `polint check --format json` providers-array clause was dropped because no G5 command produces that array); on `control_flow` after commit 2, N/N identical (provider-count rule, section 5.1) because the enabled set and the compact materialisation are both unchanged; `polint unknowns --cap control_flow` shows `DomainBudget` rows after commit 3, through the collector row W3 adds | every fact row outside `domain_*` and the counted `summary_control` payloads; the 15 unaffected digests; stdout |
| G6 | W1 to W8 | `probe full-calls calls <core>` with `--timeline` | exit 0; wall under 300 s; tree peak (`rssrun.py`) under 12 GB with the polint-process `peak_rss_mb` and the overlap peak (polint RSS while the sidecar is resident, expected under 5,265 MB) reported beside it; G2b passed first; every selected provider has a stage row (provider-count rule, section 5.1; the arithmetic is in the research doc section 7, G6 defence, round 4) | the diagnostics oracle and ai-friendly stdout (section 5.1); the fact-row dump on the 885-file `calls` cell against the post-W6 baseline (round 5: the earlier cell named an ai-friendly distribution the probe does not produce) |
| G7 | every workstream | `cargo test -p polint --lib eval::determinism_gate --locked`; two 885-file runs at `RAYON_NUM_THREADS=1` and `=12`, cold and warm, probe stdout diffed; `polint check --format ai-friendly` on `examples/*` at both thread counts, stdout diffed | byte-identical probe stdout (`polint unknowns` agent JSON; the command has no ai-friendly form, `cli/mod.rs:314-316`) and byte-identical ai-friendly stdout on the golden corpus (round 4: the earlier cell named ai-friendly output for a probe that cannot produce it) | all |
| G8 | W6 | `probe full-ts calls <frontend paths>`; `fact_rows_dump` on the same cell against the post-W5 baseline (section 2 baseline rule) | exit 0; wall under 300 s; tree peak under 12 GB (no Go sidecar on this cell, so tree and process peaks are close) | the nine unaffected `digest=` values; every dumped fact family (round 5: the cell named fact rows without a dump command) |
| G9 | W7, W8 | cold probe; edit one Go file in a leaf unit; `KEEP_CACHE=1 probe g9-warm calls <core>` | warm under 30 s; `units.touched` equals the edited unit plus dependents; stdout identical to a cold run on the edited tree | first-run rows |
| G10 | W9 | `POLINT_MEMORY_CEILING_MB=8192 probe g10 calls <core>` | exit 0; `polint unknowns` stdout carries exactly one row with family `ResourceBudget` and status `budget_exceeded` (`collect.rs:242-262`) whose reason names the provider the ceiling tripped after (`resource.rs:133-147`); the providers without a `stage done` row after that provider are the degraded set, and W9's diagnostic evidence lists their capabilities | rows under a normal ceiling |

### 5.3 Probe matrix

| Scope and request | cold | warm | one-file change |
|---|---|---|---|
| 45 Go files, `calls` | G1, G1c | G7 | |
| 45 Go files, `dataflow` | G1, G1c (the only Go cell where `data_flow` and `evidence` run; required for W2) | | |
| 885 Go files, `calls` | G1, G1c, G1b, G2, G5 | G7 | G9 |
| 885 Go files, `control_flow` | G5 | | |
| 1,588 Go files, `calls` | G1b, curve point (`POLINT_GATE_TIMEOUT=900`) | | |
| 4,752 Go files, `calls` | G3, G4, G6, G10 | G6 warm | G9 |
| 2,381 TS files, `calls` | G8 | | |
| excalidraw (public, 385 TS files), `dataflow` | G1, G1c against `.scale-envelope` X6 (the X-series ran the full `dataflow` plan) | | |
| excalidraw, `dataflow`, `POLINT_CFG_MAX_DOMINANCE_PAIRS=0` | G4 full-relation identity (`POLINT_GATE_TIMEOUT=600`; X5 measured 6,400 MB and 242.5 s for this cell) | | |

### 5.4 Suite and lint

Every commit: `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`; `cargo test -p polint --lib --all-features --locked` with the scale corpus moved aside; `cargo test -p polint --tests --locked` for the integration targets (`public_surface_leak`, `consumer_api_compat`, `capability_matrix`, `golden`, `rule_host_store`, `internal_architecture`, `module_layering`); `cargo test -p polint --lib eval::determinism_gate --locked`. The workspace-wide `cargo test --workspace` is not a per-commit gate (it takes hours and fail-fasts before the integration targets); run it once before the final gate matrix.

## 6. Report format for committed gate runs

`research/strategy/plans/gate-reports/<YYYY-MM-DD>_<host-label>.md`, written by `scripts/deep-gate/report.py` only:

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

Permitted content: file counts, timings, sizes, provider ids, rule ids of built-in diagnostics (for example `polint/resource-budget`), gate names. Forbidden: any scanned file path beyond a basename, any diagnostic message text, any consumer identifier. `report.py` enforces the allowlist and the README states it.

## 7. Out of scope

- No CI for the acceptance gate: no GitHub-hosted job, no self-hosted runner, no schedule, no `workflow_dispatch` (Resolved Q6). Existing PR checks continue unchanged.
- No change to `sdk/facts.rs`, `sdk/policy.rs`, `evidence_v1`, `docs/facts/`, `docs/schemas/*.json`, or the ai-friendly report shape (Resolved Q2). No new public raw-graph API; Phase 69's `polint graph` is separate.
- No consumer source, diagnostic text, or repository name in this repository or its history; local probe output stays outside the repository; committed reports carry counts and timings only.
- No rewrite of either sidecar; no MIR emission from the Go sidecar; no TypeScript 7 dependency.
- No query language, no ML, no distributed or GPU solving, no daemon.
- No change to which capabilities a rule can request or to the capability derivation from typed views; W3 changes what a capability costs, not what it means.
- No re-research of the TS type sidecar (PR #121); when it merges, W6's TS unit discovery reuses its project list.
- No time estimates or calendar framing; sequencing is by dependency only.
- `polint.type_value_alias` and `polint.semantic_graph` are not root-caused or redesigned here; they inherit W1's indexes and W5's identity, are re-profiled after W6, and get their own plan if they still exceed 60 s on the full backend (research doc section 10).
