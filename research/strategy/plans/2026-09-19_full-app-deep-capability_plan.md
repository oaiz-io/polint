# Implementation Plan: Full-Application Deep Capability (W0 to W9)

Date: 2026-09-19
Planner: Claude Fable 5.1 (delegated)
Input contract: [../04-full-app-deep-capability.md](../04-full-app-deep-capability.md) at `07a338ce`. Every workstream, gate, and resolved question in that document is binding here; nothing is relitigated. Where this plan says "the research doc", it means that file at that commit.
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
| `push_body` package and module scans | `go/mir/lower.rs:645-656` | `:621` (fn), scans inside | unchanged |
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

Two things in this tree that the research doc did not record and that change the plan's shape:

1. `AbstractDomainsProvider::run` already selects a compact "summary inputs" materialisation when `control_flow` is requested without `calls` or `dataflow` (`analysis_kernel/provider.rs:450-458`). W3 therefore does not introduce the concept of a reduced domains run; it moves the decision from a capability string check inside one provider into the family closure and makes the full run per-function.
2. The policy-query guard checks (`policy_queries.rs:1383-1418`) consume `cfg_dominators` as a directed relation and answer by walking it. Tree edges are a valid relation for that walk, which is why the bound has been byte-identical on `polint check` (`.scale-envelope/EXPERIMENTS.md` X5b). W4 can therefore make "tree edges always" the only materialisation without a consumer change, and keep the closure emission solely for the `POLINT_CFG_MAX_DOMINANCE_PAIRS=0` digest-identity path.

## 1. Goal, invariants, and conventions

### 1.1 Goal

After W0 to W9: a forced `calls` scan of the full 4,752-file Go backend (the "full backend" below; the consumer repository is never named in any committed artifact) completes in under 300 s at 12 threads with peak tree RSS under 12 GB, with byte-identical `polint check` output across cold, warm and permuted provider order; the same for the 2,381-file TS frontend; a one-file change re-lowers only its unit and dependents; and a fresh-container rule-host store miss no longer costs a 193 s compile. The research doc's section 7 is the gate list; section 5 here restates it as a checklist.

### 1.2 Invariants carried into every workstream

- **I1, the digest oracle.** Every provider's `digest=` value in the `polint::kernel::stage` rows and the `polint check` diagnostics digest are byte-identical before and after a change, except where a workstream's "may move" list says otherwise. Tool: `.scale-envelope/digests.py <before.stderr> <after.stderr>` prints `N/N provider output digests identical`; any `DIFFER` or `MISSING` row is a failing build for the workstream unless listed. The "must not move" column of the research doc's stage-row table (section 8) is copied into each workstream's test plan below.
- **I2, frozen public surface.** `crates/polint/src/sdk/facts.rs`, `crates/polint/src/sdk/policy.rs`, the `evidence_v1` envelope, `docs/facts/`, every `docs/schemas/*.json`, and the ai-friendly report shape do not change. `crates/polint/tests/public_surface_leak.rs` and `consumer_api_compat.rs` gate this and must stay green in every commit. (Resolved Q2.)
- **I3, internal traits change freely.** `AnalysisHost`, `AnalysisDb`, `FactDatabase`, `FactMetaStore`, the provider manifests, and every `analysis_*` module are `pub(crate)` and change without a decision record (Resolved Q2). The store schema and the layer-cache manifest schema are one-way doors and carry a schema-label bump when they change (W7).
- **I4, no dual paths.** The PR that lands a replacement deletes the path it replaces (report 03 rule 2). This plan names the deletion in each workstream's commit shape.
- **I5, determinism.** `cargo test -p polint --lib eval::determinism_gate --locked` (the N=10 seeded-permutation gate, `.github/workflows/ci.yml:247-248`) stays green in every commit; W6 extends its fixture set.
- **I6, hygiene.** No consumer source, diagnostic text, or repository name enters this repository or its history; committed measurements carry counts, timings, rule ids and scanned-file basenames only.
- **I7, no CI for the gate.** The acceptance gate is local-only, forever (Resolved Q6). No hosted job, no self-hosted runner, no schedule, no `workflow_dispatch`. The existing CI jobs (fmt, clippy, tests, determinism gate, leak gate) continue to run on every PR as today; nothing here adds to them except unit tests.

### 1.3 Conventions

- Delivery rules are report 03 section 3: a PR changes structure or behaviour, never both; at most 1,500 changed lines and 25 files; one storage invariant per PR; no expectation edits to pass a test.
- "Probe" means the shell function in section 5.1; every probe run stores `stdout`, `stderr` and the `/usr/bin/time -v` file under a local, uncommitted directory. The committed artifact is the report format in section 6.
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
- W1, W2 and W4 can be developed in parallel worktrees because they touch disjoint files (W1: lowerers, `ifds`, `refined_calls`, `calls`, `core/db.rs` overrides; W2: six digest functions and `domains/provider.rs` side tables; W4: `cfg/derived.rs` and `cfg/provider.rs`). Their probes must not overlap on the 885-file or full-backend scopes; the 45-file scope and excalidraw can run concurrently with a build.
- The G1 oracle needs a "before" stderr per scope captured once from the branch base and reused by every workstream; capture it before starting W1.
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
CARGO_HOME=$B /usr/bin/time -v polint check --profile core --fail-on none <paths> 2> /tmp/g0.time
grep -E "Elapsed|Maximum resident" /tmp/g0.time
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
    functions_by_file_name: HashMap<(FileId, &'db str), Vec<&'db FunctionFact>>, // insertion order preserved
    package_by_file: HashMap<FileId, PackageId>,
    module_node_by_file: HashMap<FileId, ModuleNodeId>,
}
```
  - `matching_function` (`:2317`) becomes a lookup in `functions_by_file_name` followed by the same `span_contains` filter over the bucket, returning the first match in bucket order (which is `db.functions()` order, so first-match semantics are preserved).
  - `push_body` (`:621`): replace the two `.iter().find(...)` scans with `package_by_file.get(&file.id)` and `module_node_by_file.get(&file.id)`. The old scans picked the first package and first module node for the file in database order; the index is built by iterating in database order and inserting only when absent (`entry().or_insert`), which reproduces "first".
  - `go_closure_capture_names` (`:732`): unchanged code, now reaching the indexed `references_for_file` through the override; `definition_for_symbol` likewise.
  - `lower_control_flow` (`:102`): replace the two per-body `filter` scans (`:118`, `:123`) with one pre-pass that groups `operations` and `control_effects` by `MirBodyId` into `BTreeMap<MirBodyId, Vec<&T>>` (operations are pushed in body order, so a `Vec<Range<usize>>` is also possible; the map is simpler and the order within a body is the push order either way). The `body_operations` and `body_effects` locals keep their types so the rest of the function is untouched.
  - Delete the trailing `.normalized(interner)` at `:99`; `SemanticStore::from_output` normalises at `analysis_neutral/store.rs:38`. Digest identity: `semantic_mir_output_digest` sorts its parts (`analysis/provider.rs:190`), so the row order fed to it does not matter, and the store's own normalisation is what every downstream consumer sees. Verify with G1 before merging; if any digest moves, keep the call and record why.

`crates/polint/src/ts/mir/lower.rs`
- Same `LoweringIndex` (shared as `analysis_neutral::lowering_index::LoweringIndex` so both lowerers use one definition), applied to `matching_function` (`:4541`), `matching_module_function` (`:4556`, becomes a per-file `Option<&FunctionFact>` computed once), `enclosing_function` (`:4568`, per-file `Vec<&FunctionFact>` sorted by span start, then linear scan of that file's functions for the smallest containing; a binary search is not needed for correctness and per-file counts are small), the per-body filters at `:132-141`, the closure capture path at `:764`, and the trailing `normalized` at `:116`.

`crates/polint/src/analysis_neutral/ifds/mod.rs`
- `Icfg::from_facts` (`:84`): build `HashMap<MirOpId, CfgNodeId>` from `cfg_nodes` once, replace the per-call-site `find` at `:96`. Both callers (`domains/solver.rs:119`, `ifds/mod.rs:221`) benefit.

`crates/polint/src/analysis_neutral/refined_calls/provider.rs`
- In the index-building block that ends with the loop at `:333`, build once:
  - `call_sites_by_file_span: HashMap<(FileId, u32, u32), Vec<&CallSiteFact>>` from `db.call_sites()` in database order;
  - `functions_by_file_name: HashMap<(FileId, &str), Vec<&FunctionFact>>` from `db.functions()` in database order;
  - `go_functions_by_qualified: HashMap<&str, Vec<&GoSemanticFunctionInput>>` from `go_semantic_functions` in input order.
  - `core_callsite_for_go_semantic_callsite` (`:361`) takes the indexes and keeps its candidate-narrowing steps (caller match, dynamic-status preference, `min_by_key` on resolved key) exactly; `core_function_for_go_semantic_function` (`:405`) and `matching_core_function_for_go_semantic_span` (`:426`) likewise. The final `min_by_key(|site| db.resolve_stable_key(site.stable_key))` stays so ties resolve identically.

`crates/polint/src/analysis_neutral/calls/extract.rs`
- `owner_symbol` (`:554`): a `HashMap<(FileId, &str, Span), SymbolId>` built once per `extract_call_sites` (`:16`) call from `db.symbols()`, first-wins insertion.

**Interfaces.** `LoweringIndex<'db>` is `pub(crate)` in `analysis_neutral`, constructed by `LoweringIndex::build(db: &'db impl AnalysisHost) -> Self`. No trait signature changes except the added `definitions_for_symbol`.

**Test strategy.**
- Gating unit tests: `go::mir::lower` and `ts::mir::lower` test modules (existing lowering fixtures at `go/mir/lower.rs:2612+`, `:2868+`); `analysis_neutral::calls::extract` tests including `extract_call_sites_is_deterministic_for_different_operation_orders` (`:966`); `analysis_neutral::refined_calls` tests; `analysis_neutral::ifds` tests; `analysis_neutral::domains::solver::deterministic_shuffled_rows_produce_byte_identical_result_digests` (`:794`).
- New unit tests: for each replaced scan, a fixture where the bucket has two candidates and the old first-match rule and the new lookup agree (two functions with the same name in one file at different spans; two symbols with the same name in one file; two call sites with the same span from different callers).
- Invariant I1: `digests.py` reports 23/23 identical on excalidraw, the 45-file scope, and the 885-file scope; the `polint check --format json` diagnostics digest is identical on `examples/*` (golden corpus, `crates/polint/tests/golden.rs`).
- Must not move: every `digest=`; `facts`; `keys`; `key_mb` on every stage row.
- Full suite: `cargo test -p polint --lib --all-features --locked` with the scale corpus moved aside (`.scale-envelope/EXPERIMENTS.md`, "Note on running the suite locally").

**Verification probe (G1, G1b, G2).**

```sh
probe s885-before calls <885-file scope>     # captured once from the branch base
probe s885-after  calls <885-file scope>
python3 .scale-envelope/digests.py /tmp/s885-before.stderr /tmp/s885-after.stderr   # 23/23 identical
python3 .scale-envelope/stages.py /tmp/s885-after.stderr                              # semantic_mir, refined_calls, abstract_domains rows drop
```

G1b (cost split): temporary `tracing::debug!(target: "polint::probe", step = ...)` rows around `lower_file`, `finish_with_types`, `lower_control_flow` and `normalized` inside `lower_go_mir`, on the 885 and 1,588-file scopes, removed before merge. Expected: `lower_control_flow` and `matching_function` are the two largest shares before W1 and are no longer visible after.

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
- Invariant I1: 23/23 on excalidraw, 45-file, 885-file. Must not move: `digest=`; `rss_mb`. Expected: `peak_rss_mb - rss_mb` on the six stage rows shrinks toward zero.
- Note the research doc's warning (section 3.3): the semantic MIR digest is a header plus sorted rows; the header parts (`provider_id=`, `config=`, `upstream_syntax=`) must sort before or after each family exactly as `parts.sort()` placed them; the property test is what proves it.

**Verification probe (G1, stage-row gap).**

```sh
probe s45-after calls <45-file scope>
python3 .scale-envelope/digests.py /tmp/s45-before.stderr /tmp/s45-after.stderr
python3 .scale-envelope/stages.py /tmp/s45-after.stderr | awk '$1 ~ /abstract_domains|semantic_mir/'   # peak column approaches rss column
```

Expected on the 45-file scope: `polint.abstract_domains` peak falls from 5,024 MB toward its retained 1,736 MB (benchmark report A.3).

**Dependency and risk.** Depends on nothing; parallel with W1 and W4. Top failure mode: a header label that is a prefix of a family label, or a family label that is a prefix of another (the property test rejects both); or a provider whose rows do not start with the family prefix in the same place the old `format!` put it. Detection: the property test at compile time of the test suite, and the digest oracle.

**Commit shape.** Two commits: (1) `refactor(digest): extract the streamed family digest from the evidence provider` (pure move, evidence digest unchanged); (2) `perf(digest): stream the six remaining sorted provider digests and drop the domains key side tables`.

### W3. Demand at fact-family granularity, and an honest domain solver

**Goal.** After W3, a rule that requests only `calls` never runs `polint.abstract_domains`, because the closure seeds fact families rather than providers and no `calls` consumer reads the one summary row domains influence (Resolved Q4). When domains do run, the solver is intraprocedural by default with a per-function iteration cap and a per-run total (Resolved Q5), and it reports which functions were cut.

**Concrete changes.**

`crates/polint/src/analysis_api/provider/mod.rs`
- `ProviderManifest` (`:180`) gains `pub output_inputs: &'static [(&'static str, &'static [&'static str])]`: for each output family, the subset of `inputs` that family actually reads. Providers with one output or uniform needs list every output against every input, so the field is a refinement, not a rewrite. `ProviderManifest` is `pub(crate)`-reachable only (Resolved Q2), so this is not a public change; `provider_manifests_are_not_public_sdk_runner_or_cli_contract` (`analysis_kernel/provider.rs:2779`) stays green.

`crates/polint/src/analysis_kernel/provider.rs`
- `polint.direct_summaries` manifest (`:1568-1598`): `output_inputs` lists `summary_control` against the full input set including `domain_observations` and `domain_events`, and `summary_call`, `summary_memory`, `summary_tito`, `summary_events` against the input set minus those two.
- `polint.type_value_alias` manifest (`:1669-1716`): remove `domain_observations` from `inputs` (declared, never read: no reference under `analysis_neutral/types/`, research doc section 3.4). This is a manifest-only change with a digest that must not move.
- `seed_providers_for_capability` (`:1070`) becomes `seed_families_for_capability(capability) -> &'static [&'static str]`: `calls` and `control_flow` seed `refined_call_edges`, `call_reachability`, `summary_call`, `summary_events`, `solver_derived_edges`; `control_flow` additionally seeds `domain_observations`, `domain_events`, `cfg_dominators`, `cfg_postdominators` (the guard policies read them, `policy_queries.rs:1383-1418`); `dataflow` adds `data_flow_*` and `evidence_*`; the rest as today.
- `providers_enabled_by_capability_closure` (`:1094`) closes over families: a demanded family enables its producer; the producer's `output_inputs` row for that family demands those input families; repeat to fixpoint. A provider is enabled when at least one of its outputs is demanded. `BASELINE_PROVIDER_SEEDS` (`:1013`) stays as provider seeds.
- `AbstractDomainsProvider::run` (`:448-497`): the `compact_domain_materialization` decision (`:450-458`) moves out of the provider: the closure records, per enabled provider, which of its outputs were demanded, and `ProviderCtx` exposes `demanded_outputs(&self) -> &BTreeSet<&'static str>`; the provider picks `SummaryInputs` materialisation when only `domain_events` plus what `summary_control` needs are demanded. The two `derive_*` entry points in `domains/provider.rs` stay.
- `scheduled_order_for` (`:1133`) is unchanged (topological order over the enabled set).

`crates/polint/src/analysis_neutral/domains/solver.rs`
- `SolverBudget` (`:33`) becomes `{ max_iterations_per_function: u32, max_iterations_total: u32, widening_fuel: u32 }` with `deterministic()` (`:81`) setting per-function 10_000 and total 1_000_000 (the total is the safety net; both are constants and can be retuned from probe data).
- `solve_with_output_mode` (`:112`): default mode is intraprocedural: `Icfg::build` is replaced by `Icfg::build_intra(db)` that emits `Intra` and `CallToReturn` edges only, so `Call`/`Return` edges and call-stack growth never occur; the exploded point's `call_stack` is then always empty and the `ExplodedPoint` map is keyed by node alone. The interprocedural mode (`Icfg::build`, call strings) remains behind `SolverPolicy { interprocedural: bool }` and is selected only when `dataflow` demanded `domain_observations` (kept for parity with today's behaviour on the `dataflow` path; W8 revisits it on the unit ICFG).
- The worklist loop (`:146-157`): per-function counter keyed by `function.body`; when a function's counter exceeds the per-function cap, only that function's states and status are marked `BudgetExceeded` (a per-function variant of `mark_ide_budget_exceeded`, `:662`) and its remaining queue entries are skipped; the total cap keeps today's whole-run behaviour.
- `materialize_results` (`:700-704`): replace the nested loop over all states per function with a lookup of the function's entry-node state (one `states.get(&ExplodedPoint { node: entry, call_stack: vec![] })`, or in interprocedural mode a `BTreeMap<CfgNodeId, Vec<&ProductState>>` grouped once).
- `DomainOutput` gains one `DomainEventFact` per cut function with reason `solver_budget_exceeded_function` and a run-level event `solver_budget_exceeded_total` when the total trips; `polint unknowns` surfaces both through the existing `budget_exceeded` row path.

`crates/polint/src/analysis_neutral/ifds/mod.rs`
- Add `Icfg::build_intra(db)`; `from_facts` gains an `include_calls: bool` parameter.

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
}
pub(crate) fn demand_plan_for(requested: &BTreeSet<&str>) -> DemandPlan;

// domains/solver.rs
pub struct SolverBudget { pub max_iterations_per_function: u32, pub max_iterations_total: u32, pub widening_fuel: u32 }
pub struct SolverPolicy { pub budget: SolverBudget, pub reduction_rounds: u32, pub interprocedural: bool }
```

**Test strategy.**
- Gating: `analysis_kernel::provider` tests (`provider_manifests_have_required_metadata` `:1974` extended to assert every output appears in `output_inputs` and every listed input is in `inputs`); `analysis_kernel` capability-closure tests; `analysis_neutral::domains::solver` tests including the shuffled-rows digest test (`:794`); `analysis_neutral::summaries` tests; `crates/polint/tests/capability_matrix.rs`.
- New tests: (a) closure over a synthetic manifest set where one provider's second output needs an extra input, asserting the extra input's producer is enabled only when that family is demanded; (b) `calls`-only plan does not enable `polint.abstract_domains`; `control_flow` plan does; `dataflow` plan does; (c) solver fixture with one function that never converges and one that does, asserting only the first is `BudgetExceeded` under the per-function cap; (d) summary-family identity test: `summary_call`, `summary_memory`, `summary_tito`, `summary_events` digests equal with and without `domain_observations` present (the research doc's section 10 mitigation).
- Invariant I1: on a `calls` request the `polint.refined_calls` `digest=` must not move (Resolved Q4); `polint.abstract_domains` and `polint.direct_summaries` rows are absent, so their digests are not compared; on a `control_flow` request all digests are compared and the domains digest may move only because of the per-function budget events (list the exact reason in the PR); on a `dataflow` request, interprocedural mode is kept so the domains digest must not move.
- The determinism gate fixtures (`tests/eval-fixtures/determinism/*`) request capabilities that pull domains; they stay green.

**Verification probe (G5).**

```sh
probe s885-calls calls <885-file scope>
grep -c 'provider="polint.abstract_domains"' /tmp/s885-calls.stderr      # 0
grep 'provider="polint.refined_calls"' /tmp/s885-calls.stderr | grep -o 'digest=[^ ]*'   # equal to before
probe s885-cf control_flow <885-file scope>
grep -c 'provider="polint.abstract_domains"' /tmp/s885-cf.stderr         # 1
polint unknowns --cap control_flow <885-file scope> | grep -c budget      # reports the per-function cuts
```

Expected: on `calls`, total `facts` falls by the domain family size (280,617 at 885 files, benchmark report A.4); `polint.refined_calls` digest unchanged.

**Dependency and risk.** Closure half depends on nothing and can land with W1; solver half after W1 so the per-function solver's cost is measured against an indexed `Icfg::build`. Top failure mode: a family-level input declared too narrowly, so a provider reads a store the closure did not populate (a silent empty read rather than a crash, because the stores default to empty). Detection: test (d) above generalised: for every provider, run the fixture with each non-declared input family's store cleared and assert the output digest is unchanged; and the `dependency_blocked` outcome path in `ProviderOutcomeTracker` must stay consistent with `output_inputs`.

**Commit shape.** Three commits: (1) `feat(kernel): declare per-output input families in provider manifests` (structure only; closure unchanged; digests unchanged); (2) `feat(kernel): seed and close the provider set over fact families` (behaviour: domains leave the `calls` path); (3) `feat(domains): intraprocedural default with per-function and per-run iteration caps, reported` (behaviour on the `control_flow` path only).

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
}
fn dom_tree(graph: &CfgGraph<'_>, root: BasicBlockId, direction: Direction, selected_exits: &BTreeSet<BasicBlockId>) -> DomTree;
impl DomTree {
    fn immediate(&self, block: BasicBlockId) -> Option<BasicBlockId>;
    fn ancestors(&self, block: BasicBlockId) -> impl Iterator<Item = BasicBlockId> + '_; // walk to root, reflexive
    fn dominates(&self, a: BasicBlockId, b: BasicBlockId) -> bool;
}
```
  - `dom_tree` computes reverse postorder over the reachable set (forward: from the entry over `successor_blocks`; reverse: from the virtual exit over reversed edges as `collect_reversed_predecessors` (`:404`) does today, including `selected_exits`), then iterates `for b in order[1..]: new_idom = intersect over processed predecessors` until no change, with the two-finger `intersect` on rpo positions (https://www.cs.tufts.edu/~nr/cs257/archive/keith-cooper/dom14.pdf, the `doms` array formulation).
  - `derive_dominators` (`:66`) and `derive_postdominators` (`:120`): emit `immediate == true` facts from `idom` always; when `materialization == Full`, additionally emit every `(dominated, ancestor)` pair from `ancestors()`, sorted as today (`facts.sort_by_cached_key(...)` at the end of each function stays). The reflexive pair (a block dominates itself) is emitted today because the relation initialises with the start in its own set and every block ends up containing itself; keep it so the pair set is identical under `POLINT_CFG_MAX_DOMINANCE_PAIRS=0`.
  - `derive_control_dependence` (`:203`): use `DomTree::immediate` for the `immediate` map and `DomTree::dominates` for the membership test at `:223-225`; the emitted facts are unchanged.
  - Keep the fact stable-key recipe (`:96-104`) byte for byte.
- `reachable_blocks` (`:309`) stays for `derive_reachability`.

`crates/polint/src/analysis_neutral/cfg/provider.rs`
- `append_derived_rows` (`:96`): the budget estimate and `materialization` selection (`:109-116`) stay; the relation is no longer computed when bounded, which is the wall-clock win.

**Interfaces.** Internal to `cfg::derived`; `derive_dominators`, `derive_postdominators`, `derive_control_dependence` keep their signatures.

**Test strategy.**
- Gating: `analysis_neutral::cfg::derived` tests (existing fixtures for immediate dominators, the `first_return` post-dominance case at `:723`); `cfg::validate` (`validate_cfg` checks dominator rows at `:232-260`); `policy_queries` guard tests (`guard_dominates_operation`, `guard_does_not_dominate`, `docs/facts/control-flow.md:94-98`); `analysis_kernel` tests asserting non-empty dominator families (`analysis_kernel/mod.rs:2214-2264`); the determinism gate.
- New tests: (a) a differential test that computes the old set-intersection relation (kept under `#[cfg(test)]` as `legacy_dominator_relation`) and the new tree closure on every CFG fixture and on randomly generated reducible and irreducible graphs (seeded, small), asserting identical `(dominated, dominator)` sets and identical immediate maps for both directions; (b) an unreachable-block fixture asserting unreachable blocks get no facts, as today; (c) the `selected_exits` virtual-exit case.
- Invariant I1: with `POLINT_CFG_MAX_DOMINANCE_PAIRS=0` every digest must not move on excalidraw, 45-file and 885-file (full closure emitted, identical pair set). With the default bound, the `polint.cfg` digest and downstream digests must not move either, because tree-only emission is what the bound already produced; the emitted tree-edge set must be identical. `polint check` diagnostics digest unchanged in both modes.
- Must not move: everything; the only permitted movement is `elapsed_ms` on the `cfg` step rows.

**Verification probe (G4).**

```sh
probe full-cfg calls <core>                                  # after W1 so semantic_mir completes
grep 'provider="polint.cfg"' /tmp/full-cfg.stderr | grep -E 'step|stage done'
# expected: dominators and postdominators steps each under 5 s; stage under 30 s (research doc G4)
POLINT_CFG_MAX_DOMINANCE_PAIRS=0 probe s885-full-relation calls <885-file scope>
python3 .scale-envelope/digests.py /tmp/s885-before-full-relation.stderr /tmp/s885-full-relation.stderr   # 23/23
```

**Dependency and risk.** Depends on nothing; parallel with W1 and W2. Top failure mode: a difference between the legacy relation and the tree closure on irreducible graphs or graphs with the virtual exit and selected exits (post-dominance with multiple returns). Detection: the differential test with seeded random graphs and the `POLINT_CFG_MAX_DOMINANCE_PAIRS=0` oracle on excalidraw, which has 4,193 functions.

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
- `PlaceStableContext` (`places.rs:27`) carries `file_key: StableKeyId, function_key: StableKeyId, body_key: StableKeyId` instead of `String`s; `PlaceTableBuilder::places: BTreeMap<String, PlaceDraft>` (`:13`) becomes `BTreeMap<StableKeyId, PlaceDraft>` ordered by `canonical_cmp` (a `BTreeMap` with a comparator wrapper, or a `Vec` sorted once at `finish_with_types`), and the `place_ids: BTreeMap<String, PlaceId>` side table in `lower_go_mir` (`:44-47`) becomes `HashMap<StableKeyId, PlaceId>`.
- Every `sort_by_cached_key(|row| interner.resolve(row.stable_key))` (`ir/body.rs:123-160`, `analysis_neutral/store.rs:192,207,225,263,285,308,420`, `cfg/store.rs`, `cfg/derived.rs`, `cfg/graph.rs:59,165`) becomes `sort_by(|a, b| interner.canonical_cmp(a.stable_key, b.stable_key))`. Same total order, no materialisation.

`crates/polint/src/analysis_kernel/mod.rs`
- The gauge (`:327-341`) keeps `keys = interner.len()` and `key_mb = interner.text_bytes()`; the semantics change is documented in the row's doc comment (atom bytes rather than composed text).

`crates/polint/src/analysis_kernel/store/*` and the layer cache
- Any persisted `payload_digest` string (provider mirrors, `witness_value`) is written through `lower_hex_u64`, so on-disk bytes are unchanged; no schema bump.

**Interfaces.** Above. `resolve` keeps returning `Arc<str>` for the SDK (`sdk/facts.rs:490`, `:565`) and for `resolve_stable_key` (`core/db.rs:417`); the boundary cache bounds the retained materialisations (default 64 k entries, `POLINT_KEY_CACHE_ENTRIES` override for measurement), and the gauge reports `resolved_texts` and `resolved_bytes` so the "memory improved" claim is measured (obligation 5).

**Test strategy.**
- Gating: every existing test that constructs or resolves keys (`internal_core::stable_key` tests, `analysis_api::metadata` tests including the conflict tests at `:676-720`, `analysis_neutral::stable_key` tests at `:27-49`, `places` tests at `:239-371`, every provider's `*_stable_key_*` test, the determinism gate, the golden corpus).
- New tests, one per proof obligation (research doc, W5 list):
  1. Canonical stream equality: for every fact family, on every fixture, on excalidraw, and on the 45 and 885-file scopes, `write_canonical(id)` equals the text the pre-W5 binary produced. Mechanism: a `#[cfg(test)]` dump of `(family, resolved text)` per fact family, run on the branch base and on W5, diffed byte for byte (a test-only CLI flag or the eval harness's fixture observation). The dump file is uncommitted for the consumer scopes and committed for fixtures under `tests/eval-fixtures/`.
  2. Digest oracle 23/23 plus diagnostics digest on the same corpora.
  3. Conflict-set identity: `FactMetaStore::stable_key_conflicts()` count and members identical per corpus (`analysis_kernel/validation.rs:6015` covers the reporting path).
  4. Backslash folding and length prefixes: the `semantic_stable_key_sorts_parts_normalizes_backslashes_and_includes_family` test (`stable_key.rs:27`) ported to `intern_parts`, plus a nested case (child with a backslash inside a parent).
  5. Boundary materialisation count: a gauge assertion on the 45-file scope that `resolved_texts` after a `calls` run is below the number of facts.
  6. Iterative walk: a synthetic 100,000-deep chain interns and resolves without stack growth; `canonical_len` overflow is a checked-arithmetic error, not a wrap.
- Must not move: every `digest=`; `facts`; `keys`. Must move: `key_mb` down by an order of magnitude.

**Verification probe (G3, part; G1).**

```sh
probe full-mir-w5 calls <core>
grep 'provider="polint.semantic_mir"' /tmp/full-mir-w5.stderr | grep -oE 'keys=[0-9]+|key_mb=[0-9]+|digest=[^ ]+'
# expected: keys unchanged versus the post-W1 run; key_mb under 350 (from 3,448); digest unchanged
```

**Dependency and risk.** Depends on W1 and W2 landing on main first (measurement fairness; and W2 removes the last `String`-keyed side tables that would otherwise need conversion). Serial with everything: touches `internal_core`, `analysis_api`, every provider. Top failure mode: a canonical text that differs by one byte in one family (a part label order, a missing backslash fold in a nested child, a decimal formatted differently). Detection: obligation 1's byte-level dump diff on every family; obligation 2 is downstream of it and catches what the dump misses only if the family reaches a digest.

**Commit shape.** Five commits, each green on the fixture suite and the excalidraw oracle:
1. `refactor(identity): add the structural interner behind the existing resolve contract; raw-text intern unchanged` (new state, `intern_parts`, `write_canonical`, `canonical_cmp`; no caller converted; `resolve` byte-identical by construction).
2. `refactor(identity): payload digests as u64, printed as the same hex` (metadata store and constructors).
3. `refactor(identity): convert the MIR, place and CFG key recipes to structural parts` (lowerers, places, cfg; oracle on excalidraw).
4. `refactor(identity): convert the remaining 34 semantic_stable_key call sites and the canonical-order sorts`.
5. `chore(identity): delete intern_and_resolve and the text-keyed side tables; gauge reports atom bytes and boundary materialisations`.

### W6. Per-unit lowering, arenas, and parallel units

**Goal.** After W6, MIR lowering, CFG construction, per-function domains and direct call extraction run per unit (Go package, or TS project with a per-file fallback, Resolved Q7) into arena-backed unit graphs with dense local ids, in parallel across units with a deterministic merge order; the whole-program `SemanticStore`, `CfgFactStore` and the per-run `normalized` sorts are deleted; `polint.semantic_mir` and `polint.cfg` are linear in the unit and parallel across units. G3, G4, G6 (first attempt), G7 and G8 are measured on this.

**Concrete changes.**

New module `crates/polint/src/analysis_neutral/unit/` (`mod.rs`, `graph.rs`, `schedule.rs`, `merge.rs`):

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
    pub(crate) input_digest: Digest,          // over the unit's file content digests plus its imports' export digests (W7 key)
}

pub(crate) struct UnitSet {
    pub(crate) units: Vec<Unit>,              // index == UnitId
    pub(crate) unit_of_file: HashMap<FileId, UnitId>,
    pub(crate) order: Vec<UnitId>,            // topological over `imports`; unordered remainder appended as its own SCC, sorted
    pub(crate) sccs: Vec<Vec<UnitId>>,        // for TS projects with reference cycles; Go SCCs are singletons
}
pub(crate) fn build_unit_set(db: &impl AnalysisHost) -> UnitSet;
```
  - Go: one unit per `TopologyPackageFact` (`analysis_neutral/module_graph/topology.rs:66`) with `language == Some(Go)`, files from `PackageFact.file` grouped by package path; `imports` from `ImportToPackageFact.{from_package,to_package}` (`:132-144`). A file with no package fact is its own unit of kind `GoPackage` with a synthetic path.
  - TS/JS: one unit per `tsconfig.json` discovered by `ts::module_graph::nearest_tsconfig_path` (`ts/module_graph/mod.rs:286`, `:1360`); files with no config above them are `TsFile` units. Project references, when parseable from the config, become `imports`; otherwise the projects are ordered by path. (PR #121's `ts/types/lifecycle.rs` does the same walk for the sidecar; when it merges, `build_unit_set` reuses its project list so the two agree.)
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
}
/// Typed dense index: a u32 newtype per arena, `Copy`, with `Arena<I, T> = TiVec<I, T>` semantics.
pub(crate) struct Arena<I, T>(Vec<T>, PhantomData<I>);
```
  - Row types keep today's payload enums (`MirTerminatorKind` `ir/body.rs:51`, `MirOperationKind` `ir/op.rs:20`, `PlaceRoot`/`PlaceProjection` `ir/places.rs:25-60`), with `Vec<PlaceId>` argument lists and `Vec<(MirValue, MirBlockId)>` cases moved into side arenas referenced by `Range<u32>` so `OpRow` and `TermRow` are `Copy`. `MirValue::BinOp`'s boxed children become indices into a `values: Arena<ValueIdx, MirValue>` side arena.
  - Whole-program ids (`MirBodyId`, `MirOpId`, `PlaceId`, `BasicBlockId`, `CfgNodeId`, `CallSiteId`) become `(UnitId, local index)` pairs packed in the existing `u64` newtypes: high 32 bits unit, low 32 bits local. Every consumer that compares ids keeps working; every consumer that indexes a whole-program `Vec` by id is rewritten to go through the unit accessor.

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
- `go/mir/lower.rs`: `lower_go_mir(db)` becomes `lower_go_unit(db, unit: &Unit, index: &LoweringIndex) -> UnitGraph`; `GoMirLowering` state becomes per unit; `lower_control_flow` runs per body inside the unit; the CFG builder (`cfg/lower.rs`, `cfg/builder.rs`) is invoked per body producing `FunctionCfg` with W4's `DomTree` stored as `idom`/`ipdom`; per-function domains (W3's intraprocedural solver) run per body inside the unit when demanded; direct call extraction (`calls/extract.rs`) runs per unit. `ts/mir/lower.rs` likewise.
- `analysis/provider.rs` (`derive_semantic_mir_with_cache_stats`) and `cfg/provider.rs` (`derive_cfg_with_cache_stats`) are replaced by one `analysis_neutral/unit/provider.rs::derive_unit_graphs_with_cache_stats(db, snapshot, manifest, demanded_outputs, upstream_digests)` behind a new provider id `polint.unit_graphs` whose manifest outputs the union of today's `semantic_mir`, `cfg`, `abstract_domains` (intraprocedural families) and `calls` outputs; the four old provider ids are removed from `provider_manifests()`, the dispatch table (`analysis_kernel/provider.rs:991-1005`), the layer kinds (`analysis_api/digest/keys.rs:9-21`, add `UnitGraphs`, remove `SemanticMir`, `Cfg`, `AbstractDomains`, `Calls`), and the run-manifest and store-mirror vocabularies. The determinism gate auto-enrolls the new provider (`determinism_gate.rs`, D-22). The interprocedural domains mode (dataflow path) becomes a separate provider `polint.interprocedural_domains` running after the merge, so `dataflow` digests can be compared.
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
- New tests: (a) `build_unit_set` on the fixture repositories asserts unit membership, order, and SCC grouping; a Go fixture with a package not reachable by any import lands in the appended tail; (b) lowering a unit twice yields byte-identical `UnitGraph::digest`; (c) lowering the fixture set with `parallel = true` under the N=10 seeded permutation of unit processing order yields byte-identical merged output (extend `determinism_gate.rs` with a unit-order permutation alongside the provider-order permutation); (d) the L2 and L3 capability probes (`crates/polint-eval/src/harness/capability_probes.rs`) pass at the same counts as before (research doc section 10, "per-unit lowering loses a cross-file fact"); (e) closure captures across files within a unit and across units resolve to the same capture names as the whole-program lowerer did (fixture with a closure in package B capturing a symbol defined in package A).
- Invariant I1 is redefined for W6, and this is the one workstream where provider digests may move: the four old provider ids disappear and `polint.unit_graphs` appears, so the per-provider oracle compares the downstream providers only (`identity`, `direct_summaries`, `entrypoints`, `reachability`, `extensions`, `type_value_alias`, `semantic_graph`, `solver`, `refined_calls`, `data_flow`, `evidence`, `metrics`) and the `polint check` diagnostics digest, all of which must not move. The stable-key text of every MIR, place, CFG and call-site fact must still be byte-identical (W5's obligation 1 dump, re-run), because the key recipes are unchanged and only the storage changed.
- Must not move: ai-friendly stdout bytes across permutations; downstream digests; diagnostics digest. Must move: `polint.semantic_mir` and `polint.cfg` rows are replaced by one `polint.unit_graphs` row whose `elapsed_ms` is a fraction of their sum and whose `rss_delta_mb` is per unit.

**Verification probe (G3, G4, G6 first attempt, G7, G8).**

```sh
probe full-w6 calls <core>
grep 'provider="polint.unit_graphs"' /tmp/full-w6.stderr        # elapsed_ms under 60 s, rss_delta_mb under 3,000, key_mb under 1,500
grep -E "Elapsed|Maximum resident" /tmp/full-w6.time             # first G6 attempt: exit 0 is the requirement; 300 s / 12 GB is the target
probe full-ts-w6 calls <frontend paths>                          # G8 first attempt
# G7: the determinism gate plus two full 885-file runs with RAYON_NUM_THREADS=1 and =12, stdout diffed
```

**Dependency and risk.** Depends on W5 (structural ids before unit-local ids), W1, W3. Serial with everything. Top failure mode: a cross-unit fact the whole-program lowerer produced from database order (closure capture names via `references_for_file`, module-level TS functions, `enclosing_function` across a unit boundary) is lost or reordered. Detection: test (e), the L2/L3 probes, and the W5 dump diff re-run on the 885-file scope; any missing key text is a stop. Second failure mode: parallel units change output bytes. Detection: the extended determinism gate and the two-thread-count stdout diff.

**Commit shape.** Seven commits, in this order, each green:
1. `feat(unit): unit set construction from the package topology and tsconfig discovery, with tests` (structure only; nothing consumes it).
2. `feat(unit): arena-backed UnitGraph and typed dense indexes; lowering into a unit graph for one unit behind a test entry point` (no provider change).
3. `feat(unit): per-body CFG with stored idom/ipdom inside the unit graph` (uses W4's tree).
4. `feat(kernel): polint.unit_graphs provider replacing semantic_mir, cfg, calls and intraprocedural abstract_domains; whole-program stores deleted` (the behaviour commit; largest; oracle on downstream digests and diagnostics).
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
- Add `write_bytes(&self, manifest: &LayerCacheManifest, payload: Vec<u8>)` and `read_bytes_validated(&self, key, validator)` that do not go through `serde_json` (the existing `write_json_bytes` at `:396` already takes bytes but computes a JSON-labelled digest; the shard path uses `payload_digest_for_bytes` directly and a `payload_encoding: "unit-shard-1"` field added to `LayerCacheManifest`, which bumps `LAYER_CACHE_MANIFEST_SCHEMA` to `polint-layer-cache-manifest-3`).
- Ceilings (`:31-32`): shards get their own `UNIT_SHARD_PAYLOAD_MAX_BYTES = 256 MiB`; the manifest ceiling stays.
- `LayerKind` (`analysis_api/digest/keys.rs:9`): add `UnitGraph`.

`crates/polint/src/analysis_kernel/incremental/keys.rs`
- `LayerKey` (`:51`) is reused per unit: `provider_id = "polint.unit_graphs"`, `input_digests = [unit.input_digest]`, `dependency_layer_digests = [import units' export digests]` (the gopls key shape, research doc 4.5). The unit's export digest is the digest of its exported symbol rows (names, kinds, spans), computed from the symbol graph per unit, so a dependency's internal edit does not invalidate dependents.

`crates/polint/src/analysis_kernel/store/`
- New migration adding `unit_shards (generation_id, unit_path, unit_kind, language, input_digest, export_digest, payload_digest, layer_key_digest, file_count, body_count)` and `unit_imports (generation_id, unit_path, import_path)`; schema version v6. The store remains manifest-and-index only; payloads stay in the layer cache (Resolved Q3).
- The SUM-03 benchmark decides blob-in-cache versus adjacent content-addressed file before this layout is locked: a `polint-bench` case that writes and reads the 885-file scope's shards both ways and reports DB size, WAL growth, and read latency. The plan assumes blob-in-cache and switches only if the benchmark shows a read-latency regression above 20 percent.

`crates/polint/src/analysis_neutral/unit/provider.rs`
- Before lowering, for each unit in order: compute `LayerKey`, `read_bytes_validated`; on hit, decode and skip lowering; on miss, lower, encode, `write_bytes`. Hits and misses are counted in `cache_stats` and reported as `unit_shards.hit` and `unit_shards.miss` counters on the stage row.
- Stale-reuse safety: the manifest's `dependencies` list the unit's files (content digests) and its imports' export digests; the existing invalidation machinery (`incremental/invalidation.rs`) applies.

**Interfaces.** Above.

**Test strategy.**
- Gating: `analysis_kernel::incremental::layer_cache` tests; store migration tests (`analysis_kernel/store/migrations.rs` tests, sentinel and version checks); `analysis_kernel::incremental::invalidation` tests.
- New tests: (a) encode/decode round trip is byte-identical (`encode(decode(encode(g))) == encode(g)`) on every fixture unit; (b) a decoded unit graph produces the same per-unit digest and the same canonical key texts as a freshly lowered one; (c) corrupted shard (truncated, bad trailer, out-of-range offset) is rejected and recomputed, never panics; (d) the stale-reuse mutation matrix report 03 names (VAL-04): edit a file in unit A, assert A and its dependents miss and every other unit hits; edit a non-exported function body in A, assert dependents still hit (export digest unchanged); change `tsconfig.json`, assert the whole project misses; change the polint version, assert every shard misses; (e) warm output byte-identical to cold on the fixture set and on the 885-file scope (cold, then warm, then `diff` of ai-friendly stdout).
- Invariant I1: every downstream digest and the diagnostics digest identical between cold and warm runs; the `polint.unit_graphs` provider digest identical between cold and warm (it is computed from per-unit digests, which are identical by test (b)).
- Must not move: first-run rows. Must move: second-run `polint.unit_graphs` `elapsed_ms` near zero for unchanged units.

**Verification probe (G9, first half).**

```sh
probe warm-1 calls <885-file scope>                              # cold: writes shards
cp -r .polint/cache /tmp/cache-after-cold
probe warm-2 calls <885-file scope>                              # do not wipe the cache for this one: edit the probe() helper or run polint directly
grep 'provider="polint.unit_graphs"' /tmp/warm-2.stderr | grep -oE 'elapsed_ms=[0-9]+|unit_shards\.[a-z]+=[0-9]+'
diff /tmp/warm-1.stdout /tmp/warm-2.stdout && echo warm-identical
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

- `polint.direct_summaries` and the SCC closure (`summaries/closure.rs:87` `close_summaries_by_scc`): the SCC schedule becomes two-level: units in `UnitSet::order`, and within a unit the existing function-level Tarjan SCCs; callee summaries from already-processed units are read from `UnitSummaries` (loaded from the cache on a warm run) rather than recomputed. Cross-unit recursion cannot occur for Go (import DAG); for TS project cycles the whole project SCC is one closure step as today.
- `polint.solver`: the Go RTA policy (`analysis/solver/policy.rs`) seeds from `UnitSummaries::rta_seed` accumulated in unit order; propagation is the existing fixpoint over the whole seed set (RTA is a single global set, research doc 4.11; it is cheap). The TS points-to policy is unchanged in algorithm and reads allocations and property facts through the unit accessors.
- `polint.refined_calls`: the Go semantic join (W1's indexes) reads `CrossUnitIndex::functions_by_file_span`; the sidecar rows are partitioned by unit via `unit_of_file` so a warm run joins only the units that re-lowered plus their dependents.
- `polint.reachability` and `polint.data_flow`: the ICFG (`ifds/mod.rs`) is built per demanded root set from unit graphs lazily: `Icfg::demand(db, index, roots)` adds a unit's function CFGs when a call edge reaches into it; the IFDS search (`find_taint_paths`, `ifds/mod.rs:191`) is unchanged in algorithm; `data_flow/local.rs` and `summary_edges.rs` read through the unit accessors. The whole-program `cfg_reachability`, `cfg_dominators`, `cfg_postdominators`, `cfg_control_dependence` and `call_sites` `Vec`s kept by W6 are replaced by unit-scoped accessors plus a `ControlFlow` view adapter in `policy_queries.rs:1383-1418` that walks `FunctionCfg::idom` directly (the tree walk `reaches` does today, without the edge relation).
- Recompute set on warm runs: a unit is "touched" when its shard missed or any import's export digest changed; `direct_summaries`, `solver`, `refined_calls`, `reachability`, `data_flow` and `evidence` run over touched units plus every unit whose summaries depend on a touched unit (transitive dependents in the import graph), and merge with persisted results for the rest. The stage rows gain `units.touched` and `units.total` counters.

**Interfaces.** Above. The `ControlFlow` SDK view's behaviour is unchanged (I2); its implementation reads the tree.

**Test strategy.**
- Gating: `analysis_neutral::summaries` (closure and builder tests), `analysis::solver` policy tests, `analysis_neutral::refined_calls`, `reachability`, `data_flow`, `evidence`, `ifds` tests; `policy_queries` guard and reach tests; the golden corpus; the capability matrix; the L4 seed probes at their current counts (4/10 Go, 4/10 TS, twins 15/20 and 18/20, `research/ts-type-sidecar/measurement.md` section 4 numbers as measured on `82a3c129`; they must not regress); the determinism gate.
- New tests: (a) a three-unit diamond fixture where a call from C reaches A through B, asserting `refined_call_edges`, `call_reachability` and a taint path are identical to the pre-W8 whole-program result (the pre-W8 expected output is committed as the fixture's golden); (b) warm-run recompute set: edit unit B, assert `units.touched == {B, C}` and that A's summaries were loaded, with output byte-identical to a cold run; (c) a TS project cycle fixture asserting the closure handles it as one SCC step.
- Invariant I1: every digest identical between cold and warm; diagnostics digest identical; the ai-friendly report's precision and status distributions unchanged (research doc section 8, "must not move" for W8).
- Must move: `polint.refined_calls`, `polint.solver`, `polint.reachability`, `polint.data_flow` rows bounded by touched units on warm runs; on cold runs their `elapsed_ms` must not regress against the post-W6 run.

**Verification probe (G6 final, G9 second half).**

```sh
probe full-w8 calls <core>                                       # G6 at the final threshold: exit 0, wall under 300 s, tree peak under 12 GB
python3 .scale-envelope/stages.py /tmp/full-w8.stderr             # all selected providers have a row
# G9: cold, then edit one Go file in a leaf package, then warm
probe g9-cold calls <core>; <edit one file>; polint unknowns --cap calls <core> ... (cache kept)
grep -oE 'units\.touched=[0-9]+' /tmp/g9-warm.stderr            # equals the edited unit plus its dependents
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
- `probe.sh`: the section 5.1 function as a script: `probe.sh <tag> <cap> <paths...>`, writes `stdout`, `stderr`, `time` under `$POLINT_GATE_OUT` (default `/tmp/polint-gate`), never under the repository.
- `gate.sh`: runs the G-matrix cells of section 5.2 that apply to the checkout it is pointed at (`POLINT_GATE_REPO`, `POLINT_GATE_SCOPES` as a list of `label=path` pairs), compares digests with `.scale-envelope/digests.py` against a `before` directory when given, and prints one Markdown table per cell.
- `report.py`: folds the run directory into the report format of section 6 and writes `research/strategy/plans/gate-reports/<date>_<host-label>.md`; it refuses to include any line from stdout or stderr other than the stage rows, the resource-budget diagnostic count, and the `/usr/bin/time` summary, so no consumer text can leak.
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
grep -c "polint/resource-budget" /tmp/polint-gate/g10.stdout      # 1; the run exited 0 rather than being killed
POLINT_WALL_BUDGET_MS=60000 scripts/deep-gate/probe.sh g10-wall calls <core>
```

**Dependency and risk.** The envelope half depends on W6 (units to check); the scripts and report format depend on nothing and should land early so W1's probes already use them. Top failure mode: a report that leaks consumer text. Detection: `report.py`'s allowlist and its unit test; review of the first committed report line by line.

**Commit shape.** Two commits: (1) `feat(gate): local probe and gate scripts with a hygiene-checked report writer` (early, after W0); (2) `feat(kernel): wall-clock budget and per-unit memory check in the resource envelope` (after W6).

## 4. Sequencing detail and parallel work on this host

| Slot | Work | Parallel with | Serialised behind |
|---|---|---|---|
| 1 | W0; W9 scripts and report format; capture `before` stderr for excalidraw, 45-file, 885-file (and the timed-out full backend) from the branch base | each other | nothing |
| 2 | W1 commits 1 to 4 | W2, W4 (separate worktrees, `CARGO_BUILD_JOBS=4` each, one build at a time) | 885-file and full-backend probes run one at a time |
| 2 | W2 commits 1 to 2 | W1, W4 | as above |
| 2 | W4 commits 1 to 2 | W1, W2 | as above |
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

```sh
export GOROOT=/opt/data/home/.local/share/go
export PATH=$GOROOT/bin:$HOME/.local/bin:$HOME/.cargo/bin:$PATH
export RAYON_NUM_THREADS=12 POLINT_JOBS=12 GOMAXPROCS=12 GOFLAGS=-p=12
export RUST_LOG=polint=debug
export POLINT_GATE_OUT=${POLINT_GATE_OUT:-/tmp/polint-gate}; mkdir -p "$POLINT_GATE_OUT"
probe() { # $1 tag, $2 cap, $3... paths; set KEEP_CACHE=1 for warm cells
  tag=$1; cap=$2; shift 2
  [ -z "${KEEP_CACHE:-}" ] && rm -rf .polint/cache/analysis .polint/cache/layers
  /usr/bin/time -v -o "$POLINT_GATE_OUT/$tag.time" timeout 300 \
    polint unknowns --cap "$cap" "$@" > "$POLINT_GATE_OUT/$tag.stdout" 2> "$POLINT_GATE_OUT/$tag.stderr"
  echo "exit=$?"; grep -E "Maximum resident|Elapsed" "$POLINT_GATE_OUT/$tag.time"
  python3 .scale-envelope/stages.py "$POLINT_GATE_OUT/$tag.stderr"
}
```

### 5.2 Gate checklist

| Gate | Closes | Command | Pass condition | Rows that must not move |
|---|---|---|---|---|
| G0 | W0 | `CARGO_HOME=$(mktemp -d) polint check --profile core --fail-on none <paths>` after one publish from a different cargo home | under 10 s; no `cargo`/`rustc` child | all |
| G1 | W1, W2, W4, W5 | `python3 .scale-envelope/digests.py before/<scope>.stderr after/<scope>.stderr` on excalidraw, 45-file, 885-file | `23/23 provider output digests identical`; `polint check --format json` diagnostics digest identical on `examples/*` | every `digest=` |
| G1b | W1 | temporary `polint::probe` step rows in `lower_go_mir` on 885 and 1,588-file scopes | `lower_control_flow` and `matching_function` shares as section 3.8 of the research doc orders them, else the section is corrected before W1 is designed | n/a |
| G2 | W1, W2 | `probe s885 calls <885-file scope>` | wall under 60 s; peak under 8 GB | `digest=`, `facts`, `keys` |
| G3 | W1, W2, W5, W6 | `probe full-mir calls <core>`; read the `polint.unit_graphs` row (pre-W6: `polint.semantic_mir`) | stage under 60 s; `rss_delta_mb` under 3,000; `key_mb` growth under 1,500 | downstream `digest=` |
| G4 | W4 | same run, `polint.cfg` step rows (pre-W6) or the unit-graphs CFG sub-rows (post-W6) | stage under 30 s; dominators step under 5 s | `digest=` with `POLINT_CFG_MAX_DOMINANCE_PAIRS=0`; tree-edge set otherwise |
| G5 | W3 | `probe s885-calls calls <885-file scope>`; `probe s885-cf control_flow <885-file scope>` | `polint.abstract_domains` row absent on `calls`, present on `control_flow`; `polint unknowns` shows the budget rows when it runs | `polint.refined_calls` `digest=` |
| G6 | W1 to W8 | `probe full-calls calls <core>` | exit 0; wall under 300 s; tree peak under 12 GB; every selected provider has a stage row | precision and status distributions in the ai-friendly report |
| G7 | every workstream | `cargo test -p polint --lib eval::determinism_gate --locked`; two 885-file runs at `RAYON_NUM_THREADS=1` and `=12`, cold and warm, stdout diffed | byte-identical ai-friendly stdout | all |
| G8 | W6 | `probe full-ts calls <frontend paths>` | exit 0; wall under 300 s; peak under 12 GB | downstream `digest=` |
| G9 | W7, W8 | cold probe; edit one Go file in a leaf unit; `KEEP_CACHE=1 probe g9-warm calls <core>` | warm under 30 s; `units.touched` equals the edited unit plus dependents; stdout identical to a cold run on the edited tree | first-run rows |
| G10 | W9 | `POLINT_MEMORY_CEILING_MB=8192 probe g10 calls <core>` | run finishes with one `polint/resource-budget` diagnostic naming the degraded capabilities | rows under a normal ceiling |

### 5.3 Probe matrix

| Scope | cold | warm | one-file change |
|---|---|---|---|
| 45 Go files | G1 | G7 | |
| 885 Go files | G1, G1b, G2, G5 | G7 | G9 |
| 1,588 Go files | G1b, curve point | | |
| 4,752 Go files | G3, G4, G6, G10 | G6 warm | G9 |
| 2,381 TS files | G8 | | |
| excalidraw (public, 385 TS files) | G1 against `.scale-envelope` X6 | | |

### 5.4 Suite and lint

Every commit: `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`; `cargo test -p polint --lib --all-features --locked` with the scale corpus moved aside; `cargo test -p polint --tests --locked` for the integration targets (`public_surface_leak`, `consumer_api_compat`, `capability_matrix`, `golden`, `rule_host_store`, `internal_architecture`, `module_layering`); `cargo test -p polint --lib eval::determinism_gate --locked`. The workspace-wide `cargo test --workspace` is not a per-commit gate (it takes hours and fail-fasts before the integration targets); run it once before the final gate matrix.

## 6. Report format for committed gate runs

`research/strategy/plans/gate-reports/<YYYY-MM-DD>_<host-label>.md`, written by `scripts/deep-gate/report.py` only:

```markdown
# Deep-capability gate run: <date>, <host-label>

polint: <version> at <sha>; host: <cores> cores, <GB> RAM; threads: 12; cache: <cold|warm>

| Scope (file count) | cap | exit | wall s | tree peak MB | providers with rows | digest oracle |
|---|---|---:|---:|---:|---:|---|
| 45 | calls | 0 | ... | ... | 21 | 23/23 |

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
