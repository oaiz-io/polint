# Open kernel correctness issues (#86–#91)

**Researched:** 2026-11-02 against `main` at `338b570b` (v0.5.0). All six issues were filed 2026-07-19 and are still present on main. Read-only code research; every claim carries a path.

No `polint graph` subcommand exists today (`Command` enum in `cli/mod.rs:179-201`: AddSkill, NewRule, Baseline, Cache, Inspect, Facts, Unknowns, Explain, Test, Check, Review, Ignores).

**#86, cross-file semantic ID collisions.** Still present. `SemanticIndexBuilder::extend` (`crates/polint/src/analysis_neutral/symbol_graph/semantic.rs:752-760`) appends seven row vectors with no rebase; per-file builders allocate `ScopeId(self.scopes.len())` etc. (`:638-723`), and `symbol_graph/mod.rs:1243` extends per file. Validation cannot catch it: `IdSets` are `BTreeSet`s (`validation.rs:3463-3487`). Schema is still `symbol-graph-facts-2` (`symbol_graph/model.rs:10`, `analysis_kernel/provider.rs:1496`). A complete fix exists off-main: `d2bb4f57 fix(symbols): rebase merged semantic identities` on `origin/find-next-gsd-task` (offset-rebases every id and parent/scope/export reference, bumps schema to `-3`) but targets the pre-refactor path `symbol_graph/semantic.rs`. Fix: port it to `analysis_neutral`, add the two-file regression, rotate schema. **1–2 days.** Blocker for a public graph CLI (wrong-file ownership) and for persistence (warm replay of collided rows).

**#87, inferred provider success.** Present. `provider_output_for_with_optional_digest` (`analysis_kernel/mod.rs:994-1024`) mints a `"native_trusted"` digest from whatever `fact_meta` rows exist (`:1193+`) whenever `output_digest` is `None` and execution was `Succeeded`. Cache-write failure is exactly that case: `ts_syntax_layer_write_failure_withholds_native_output_identity` (`ts/tests.rs:302-316`) asserts `output_digest.is_none()` plus an `internal/cache` diagnostic. Downstream, `upstream_digests` only records `Some` (`:402-404`), so failed/skipped/absent collapse to `Digest::absent` (e.g. `solver/provider.rs:342`). Fix: keep the digest computed before the write (adapters compute it from results, `go/adapter.rs:364-368`) and report write failure as telemetry; delete the fallback at `:1016`; carry `ProviderOutcomeStatus` (already closed, `outcome.rs:6-15`) through dependency inputs. **3–4 days** bounded per family. Blocker for persistence.

**#88, capability support not revoked.** Present but substantially mitigated. `capability_support` is cloned from the plan before providers run (`mod.rs:249`) and returned unchanged (`:948`). After `seal(&validation_downgrades)` (`:915-921`), `runtime_capability_blockers` (`:1057+`) blocks rules (`core/rule.rs:445`) and sets unavailable capabilities that `build_optional` honours (`sdk/facts.rs:1178`). Residual: the `CapabilitySupportView` handed to `RuleCtx`, `explain`, and inspect JSON still says Supported; `capability_providers` (`:1140-1171`) is a hand-maintained shallow map (`dataflow` → `polint.evidence` only) relying on `hard_dependencies` for propagation. Fix: derive an effective view from sealed outcomes and return it in `KernelOutput`. **1–2 days.** Not a blocker for either goal.

**#89, syntax-layer manifests under-authenticated.** Present. The adapter writes only `SourceText` edges from `input_digests` (`cache/analysis_cache_adapter.rs:117-125`); config/lifecycle/toolchain/parameter digests in the key (`go/adapter.rs:239-251`; note `lifecycle_digest` is `absent` for Go syntax) have no edges. Read-side `dependencies_match_layer_key` (`incremental/layer_cache.rs:696-717`) only checks each edge's `from`, and only requires non-empty edges for `GoSyntax|ModuleGraph|SymbolGraph|Metrics`, not `TsSyntax`. `config_digest` wraps a rendered `config_hash` string. Fix: one canonical edge builder from key + snapshot, exact-set equality on read, include TsSyntax. **1–2 days**, one cache-protocol bump. Matters for trusted reuse, secondary for the graph CLI.

**#90, unconsumed outputs in semantic-graph identity.** Present. `semantic_graph_output_digest` folds `identity`, `abstract_domains`, `entrypoints`, `reachability`, `module_topology` (`analysis/semantic_graph/provider.rs:222-240, 582-588`); the builders read only semantic nodes/edges/constraints/scopes, call sites, value facts, functions, packages, paths, TS object-model and Go semantic rows. `cache_key.rs:33-36` documents them as "ALSO-FOLDED, NOT-YET-READ". `hard_dependencies("polint.semantic_graph")` lists the same five (`outcome.rs:688-690`). Performance issue, not correctness, **except** that it currently masks #91.

**#91, Go RTA roots outside solver identity.** Present. `GoRtaInputs::from_db` reads `db.reachability_roots()` (`go/rta/inputs.rs:94-104`), but `solver_output_digest` (`analysis_neutral/solver/digest.rs:11-51`) has no reachability term and `hard_dependencies("polint.solver")` is `[GO_SEM, GRAPH, TVA]` (`outcome.rs:691`). Roots are covered only transitively via the semantic-graph digest that #90 wants to remove. Fix: add `reachability` to the solver's hard deps and digest (the reachability digest already binds `root=` rows and unresolved configured roots, `reachability/provider.rs:163-172`), plus a sorted digest of the consumed Go root set built from the same `GoRtaInputs` snapshot; then strip the five inputs from semantic_graph. **2–3 days for #90+#91 together.** Blocker for persisting summaries.

## PR grouping

- **"identity and invalidation"**: #86 (port `d2bb4f57`, schema → `-3`), #91 then #90 in that order with mutation regressions. ~4–5 days. Prerequisite for both a public graph CLI and cross-run persistence.
- **"outcome truthfulness"**: #87, #88, #89 (one shared outcome primitive, then per family). ~5–7 days. Prerequisite for persistence; #88 improves rule honesty only.

## Other open issues (non-kernel)

- **#52** `WorkspaceFiles` fact view for non-language files (Astro/MDX/lockfiles). SDK feature; independent of the engine push. Candidate for a later rule-authoring milestone.
- **#9** AI-agent ergonomics: rule templates, remediation hints in diagnostics, rule test workflow. Partly addressed (`polint test`, `--unit`, generated skill); the rest is a rule-authoring milestone item.
- **#49** graph accuracy engine for benchmark recall: superseded by v1.3/v1.4 and the typed tiers; should be closed with a pointer to the committed accuracy baseline.

## Facts verified in code (main @ 338b570b)

- `extend` without rebase `analysis_neutral/symbol_graph/semantic.rs:752-760`; schema `symbol_graph/model.rs:10`; fix commit `d2bb4f57` only on `origin/find-next-gsd-task`.
- Trusted-digest fallback `analysis_kernel/mod.rs:994-1024`, caller `:408-414`; write-failure test `ts/tests.rs:302-316`.
- Support view cloned pre-run `mod.rs:249`, returned `:948`; blockers `:1057+`, `:1140-1171`; `rule.rs:445`; `facts.rs:1178`.
- Syntax edges `cache/analysis_cache_adapter.rs:117-125`; read check `incremental/layer_cache.rs:696-717`; Go key `go/adapter.rs:239-251`.
- Semantic-graph digest `analysis/semantic_graph/provider.rs:222-240, 565-598`; rationale `analysis_neutral/semantic_graph/cache_key.rs:33-36`; hard deps `analysis_kernel/outcome.rs:642-697`.
- RTA roots `go/rta/inputs.rs:94-104`; solver digest `analysis_neutral/solver/digest.rs:11-51`; reachability digest rows `analysis_neutral/reachability/provider.rs:141-172`.
