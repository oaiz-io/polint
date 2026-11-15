# Milestones

## v2.0 Static Analysis 2.0 Implementation (Re-scoped: 2026-11-02)

**Phases completed:** 3 of 9, Phase 63 through Phase 65 (benchmarks and regression gates, SQLite store foundation, generation manifests with sealed outcomes and cold/warm parity).

**Artifacts:** [roadmap archive](milestones/v2.0-ROADMAP.md), [requirements archive](milestones/v2.0-REQUIREMENTS.md), phase records under `phases/63-*`, `64-*`, `65-*`.

**Disposition:** the surviving requirements (summary persistence and invalidation frontier, query engine, graph CLI, validation gates) are carried into v2.1 as file-based persistence and in-memory query services; provider metadata mirroring, whole-fact SQLite ingest, lexical search and SQLite scale benchmarks are cut. Research: `research/v2.1-engine/03-semantic-store-rescoping.md`.

**Shipped outside the phase sequence while v2.0 was open:** the perf campaign (#129–#132) and Go deep analysis (#133), recorded as quick tasks in `STATE.md`.


## v1.3 Graph Engine Precision (Completed: 2026-06-06)

**Phases completed:** 13 phases, Phase 42 through Phase 54

**Artifacts:** [roadmap archive](milestones/v1.3-ROADMAP.md), [requirements archive](milestones/v1.3-REQUIREMENTS.md)

**Key accomplishments:**

- Stable benchmark identity records, renderers, deduplication, and identity-vs-unsupported taxonomy for Go and Jelly scoring.
- Explicit reachability roots, per-suite scoring modes, and deterministic provider-order gates inherited by solver phases.
- Private shared semantic graph, constraint vocabulary, unified deterministic solver core, and derived-edge provenance.
- Go semantic sidecar/frontend and Go RTA driver over the unified solver.
- JS/TS function/callsite inventory, lexical scope/binding model, function-token propagation, and object/property/prototype/`this` model.
- Private adaptation model layer with accepted/rejected facts, sandboxed benchmark adaptation reporting, and model provenance.
- Refined-call projection over solver output plus consolidated unknown taxonomy and `polint inspect unknowns --format json`.
- Cache and solver-budget consolidation across v1.3 fact families.
- Benchmark promotion gate extension with precision floors, F0.5 tracking, polyglot canary, and public-surface leak proof.

## v1.2 Static Analysis Engine Implementation (Shipped: 2026-05-27)

**Phases completed:** 22 phases, 136 plans, 267 tasks

**Artifacts:** [roadmap archive](milestones/v1.2-ROADMAP.md), [requirements archive](milestones/v1.2-REQUIREMENTS.md), [milestone audit](milestones/v1.2-MILESTONE-AUDIT.md), [phase archive](milestones/v1.2-phases/)

**Tag:** `v1.2`

**Known deferred items at close:** 84 acknowledged non-blocking artifact-audit items (see `.planning/STATE.md` Deferred Items).

**Key accomplishments:**

- Private AnalysisKernel facade now owns the existing source, syntax, module, symbol, and metrics provider sequence without changing rule-facing behavior
- Crate-private provider manifests and test-only provider order inspection for the existing AnalysisKernel sequence
- Crate-private FactMeta sidecar with deterministic stable keys for source, Go syntax, and TS/JS syntax facts
- Module graph, symbol graph, and metrics facts now receive internal metadata, with deterministic gap detection across all current fact families
- Deterministic metadata stable-key conflict tracking and pre-rule kernel validation diagnostics
- Deterministic internal provenance debug JSON for core fact families with public CLI/API compatibility proof
- Crate-private evaluation schema with deterministic report JSON and semantic output hashing for Phase 22 fixtures
- Deterministic eval matching and unified accuracy/cost metrics over normalized internal harness rows
- Native fixture execution against real AnalysisKernel output with deterministic provider-order evidence
- Native provenance and cache determinism fixtures over real kernel output with strict producer metadata matching
- Extension-style accepted, rejected, and changed facts proven through a synthetic native fixture without activating extension execution
- Native eval fixture coverage and public-boundary proof for the internal evaluation harness MVP
- Crate-private incremental cache identity vocabulary with deterministic digest helpers, typed keys, cache counters, and provider output metadata.
- Crate-private deterministic input snapshots for source, config, lifecycle, rule, model, extension, provider schema, and tool identity inputs.
- Current Go and TS/JS file-fact cache access now reports deterministic internal CacheStats without changing existing cache reuse behavior.
- Crate-private kernel run reports now carry deterministic input snapshots, provider output metadata, and aggregate cache stats.
- Native eval fixture coverage for Phase 23 input snapshots, provider metadata, exact syntax cache counters, and public no-leak behavior.
- Crate-private layer cache foundation with deterministic dependency indexes, fail-closed invalidation, and safe manifest/blob persistence
- Persistent Go and TS/JS syntax layers with rule-independent keys, verified reuse stats, and public-output compatibility coverage
- Persistent module graph layer cache with conservative import/lifecycle/config invalidation and internal run-report stats
- Persistent symbol/reference and metrics layers with conservative upstream invalidation and internal run-report reuse stats
- Real-provider layer-cache proof with stale-entry hardening and public no-leak regression coverage
- Crate-private semantic index substrate with deterministic stable keys, AnalysisDb storage, metadata rows, and provider manifest outputs
- Oxc-backed TS/JS semantic rows for scopes, imports, exports, aliases, resolution steps, and native stable export identities
- Go sidecar semantic schema and crate-private normalization for scopes, imports, aliases, stable exports, setup gaps, and unknown resolution states
- Bounded alias/reexport closure, native generated-symbol hooks, fail-closed semantic validation, and test-only semantic debug JSON
- Semantic index rows now participate in symbol graph layer cache identity, payload persistence, validation, restore, and stable export warm-reuse proof
- Internal semantic eval rows, native semantic-index fixture coverage, and public no-leak proof for existing symbol/reference surfaces
- Crate-private topology facts with AnalysisDb storage, sidecar metadata, and module graph provider schema v2 outputs
- Go monorepo module roots, static manifests, source sets, declared requirements, and go.sum evidence as internal topology rows
- Deterministic TS/JS workspace, package, source-set, declared dependency, and lockfile-evidence topology from static manifests
- Base Go and TS/JS topology rows now flow through module graph derivation, cache payloads, and topology-aware invalidation
- Semantic-aware import-to-package topology with a post-symbol provider, cache-backed payloads, and fail-closed validation
- Private topology facts are now verified through native eval rows, uncertainty scoring, and cold/warm/edit cache invariants.
- Module topology internals remain private while public relationship fact views stay compatible for external rule packs.
- Crate-private semantic analysis contracts with deterministic MIR/place stable keys and explicit unsupported-semantics rows
- Crate-private semantic MIR store with deterministic replacement, metadata coverage, and public-boundary guards
- Go function and method bodies lower into deterministic private MIR, place, call-shape, and unsupported-semantics rows
- TypeScript and JavaScript function bodies lower into deterministic private MIR, place, call-shape, and unsupported-semantics rows
- Private semantic MIR provider with deterministic cache identity, validation diagnostics, run-report rows, and test-only debug JSON.
- Internal eval snapshots for Go and TS/JS semantic MIR bodies, operations, places, and unsupported semantics
- Semantic MIR/place internals remain private while public rule-author SDK workflows still run through check, inspect, and test JSON.
- Private CFG fact/storage contracts with AnalysisDb metadata-backed storage
- Shared CFG construction plus deterministic reachability, dominance, postdominance, and control-dependence derivation
- Private `polint.cfg` provider wiring with deterministic identity, validation, and test-only visibility
- Private Go CFG lowering over semantic MIR
- Private TS/JS CFG lowering over semantic MIR
- CFG validation fixtures and public-boundary proof
- Crate-private direct-call fact contracts with deterministic storage indexes and polint.calls metadata sidecars
- Private polint.calls provider shell with deterministic output digest and future-fit calls layer keys
- Crate-private call fact validation plus test-only safe call debug rows, counts, and D-10 index evidence
- MIR-driven call-site extraction with explicit unresolved-call evidence and populated provider/debug proof
- Semantic-reference-backed direct, import-binding, and static/member call targets with honest unresolved rows for deferred call graph tiers
- Direct-call debug rows now normalize into deterministic eval facts with compact call evidence and unknown-like status proof
- 1. [Rule 1 - Bug] Classified MIR unknown direct-call evidence
- Public-boundary proof that private direct-call facts do not leak and call_graph remains unsupported
- Crate-private P0 abstract-domain vocabulary with law-tested lattice contracts, finite core domains, and deterministic product state
- Deterministic local abstract interpreter with conservative MIR/call transfer and stable result cursors
- Private abstract-domain facts persisted in AnalysisDb with provider metadata and deterministic cache identity
- Fail-closed abstract-domain validation with safe debug snapshots and eval proof of provider order
- Internal abstract-domain eval fixtures with deterministic top/unknown/budget evidence and public CLI/SDK no-leak proof
- SummaryDomain trait, four core direct-summary domain types with lattice law tests, fact vocabulary enums, and summary ID newtypes
- SummaryOutput normalization with SummaryStore indexed accessors, FactFamily summary variants, and AnalysisDb storage with metadata refresh under polint.direct_summaries
- DirectSummaryBuilder producing four-domain SummaryOutput from MIR/CFG/calls/domain facts with explicit unknown/top for unresolved calls
- Summary provider wired into kernel with parameter digest, output digest, LayerKind::DirectSummaries, and provider order between abstract_domains and metrics
- Summary validation catches dangling function references, duplicate stable keys, and precision ceiling violations; debug JSON provides compact deterministic summary snapshots with status and domain counts
- Eval observation normalizes summary debug JSON into compact eval rows with four fact families, domain count invariants, and a native mixed Go/TS fixture proving control/call/memory/TITO/unknown summaries and determinism
- Integration test proving 21 direct-summary internal markers stay private across public CLI JSON, inspect, test, help, SDK, runner, README, docs, and external rule consumers
- Private demand query layer with iterative Tarjan SCC decomposition, dependency-tracking query context, extension-aware quarantine, and typed query trace for expensive analysis views
- DemandQueryEngine with BTreeMap-based in-run memoization, kernel-level trace recording, and KernelRunReport demand_query_trace integration
- SCC discovery builds a petgraph call graph from CallStore/SummaryStore facts and computes Tarjan SCCs in reverse topological order with deterministic stable-key tie-breaking for summary scheduling
- SCC-ordered interprocedural summary closure is implemented and wired into the kernel after direct summaries, with recursive fixpoint budgeting, backdating, and demand-query trace recording.
- QuarantineStore with CacheNode-keyed quarantine, reinstate, cleanup, native-only rejection, and invalidation-to-quarantine integration proven through 18 synthetic extension digest tests
- SCC closure summaries are now validated for provenance, budget evidence, and precision, and metadata debug JSON exposes SCC schedule, closure iteration/backdating stats, and demand query trace rows.
- Eval observation now covers SCC schedule and demand query debug rows, a mixed Go/TS SCC fixture exists, and public surfaces are guarded against Phase 33 internal leakage.
- 1. [Rule 1 - Bug] Fixed direct_summaries provider output digest divergence after SCC closure
- 1. [Rule 3 - Blocking] Entrypoint fact accessors were #[cfg(test)] only
- 1. [Rule 2 - Missing critical functionality] Fixture requires .polint.toml configuration
- Private TS/JS MIR-derived type/value/access-path facts with conservative dynamic unknowns and TS/JS lifecycle cache identity coverage
- Bounded points-to constraints and evidence-backed alias answers stored by the private Phase 36 provider
- Validated extension facts can now add bounded type/value/alias precision to the private Phase 36 provider output.
- Phase 36 is closed with validation, deterministic debug/eval coverage, full regression, and no public API leak for the private type/value/place/alias substrate.
- Private refined call edge facts with deterministic storage, metadata, and internal tier indexes
- Deterministic private refined-call provider wired into the kernel after type/value/alias analysis
- Framework dispatch and bind-only summary hints now produce private refined call edges
- Go receiver type and bounded points-to evidence now create explicit refined call candidates
- TS/JS and validated extension/model evidence now participate in refined call candidates
- Phase 37 now has closure coverage for private refined-call providers
- Crate-private data-flow fact contracts with stable-key identity and AnalysisDb metadata
- Data-flow provider wired into the kernel with deterministic cache identity
- Local MIR places projected into private data-flow nodes
- Resolved refined-call edges projected as interprocedural data-flow edges
- Trust-boundary source models and extension-provided data-flow models
- Budgeted private path search for data-flow graph queries
- Validation hook, eval order proof, and documented private data-flow boundary
- Local value-flow edges and stored uncertainty
- Summary-projected and interprocedural data-flow closure
- Data-flow eval fixtures, debug, and public boundary proof
- Private evidence substrate wired into the analysis kernel after data flow with deterministic empty output
- Local data-flow and control evidence can now be sliced with thin/full traversal modes
- Evidence paths can now be bounded, chopped, and ranked deterministically
- Evidence paths now preserve summary expansion handles and call-site context
- Diagnostics can now carry versioned structured evidence and project it to JSON/SARIF
- Extension evidence is now validation-gated and visible as deterministic deltas
- Phase 39 is now closed with deterministic fixtures and a public-boundary proof
- Crate-private benchmark schema for suite manifests, three-way comparison rows, and auditable adaptation records
- Provider/cache performance evidence and deterministic Markdown summaries over canonical eval JSON
- Native promotion gate verdicts and fixture coverage for graph, fact, path, unknown, budget, and cache evidence
- Internal tier runner plus supported-language SecBench.js and gosec smoke adapter shapes
- Recorded adaptation prompt, validation gates, and default-vs-adapted delta reporting
- Competitor result records plus normalized baseline regression comparison gates
- Internal eval execution helper, native promotion determinism, public boundary proof, and Phase 40 verification

---

## v1.0 MVP (Shipped: 2026-05-02)

**Phases completed:** 10 phases, 35 plans, 90 tasks

**Artifacts:** [roadmap archive](milestones/v1.0-ROADMAP.md), [requirements archive](milestones/v1.0-REQUIREMENTS.md), [milestone audit](milestones/v1.0-MILESTONE-AUDIT.md)

**Tag:** `v1.0`

**Key accomplishments:**

- Core fact storage, span conversion, and rule execution are now covered by deterministic unit/property tests with ordered parallel runner output.
- Diagnostics now carry the full Phase 3 contract, use stable full-range fingerprints, and have deterministic unit, property, and inline snapshot coverage.
- Deterministic discovery now has focused filesystem and CLI evidence, and Phase 3 status records reflect only the verified core/diagnostics scope.
- Tree-sitter-backed Go parser diagnostics and package-name facts now feed the core AnalysisDb without broad core refactors.
- Tree-sitter-backed Go imports, declarations, calls, complexity, and heuristic test evidence now feed core facts and graph/rule consumers.
- Parser-backed Go branch obligations with stable fingerprints and conservative syntax-only error-path flags.
- Expanded Go fixtures and JSON CLI integration tests now prove Phase 4 parser diagnostics, Go facts, branch obligations, and import-boundary diagnostics through `polint check`.
- Oxc-backed TS/JS parsing now emits parser/ts diagnostics, preserves recoverable import facts, and parses borrowed SourceFile text without full-source cloning.
- Oxc AST-backed TS/JS imports, functions, classes, methods, component heuristics, and call facts with a narrow core TsClassFact contract.
- Completed the remaining parser-backed Phase 5 TS/JS fact extraction and unit proof.
- Proved Phase 5 end to end through fixtures, CLI JSON tests, and full workspace gates.
- SDK-first RuleCtx helpers, prelude exports, and new-rule scaffolds for borrowed fact queries
- Exact literal allow-list config plus Go string literal and TS/JS regex literal facts for SDK rules
- SDK-facing Go/TS complexity, import-boundary, raw-color, and denied-literal rules with configured thresholds, allow-lists, and deterministic evidence
- SDK-facing Go heuristic rules with companion test evidence, weighted suite scores, and assertion evidence labels
- Parsed JSON CLI integration coverage for all eight SDK example rules with clean and failing fixture proof
- Representative human and JSON diagnostic snapshots for all Phase 6 example rule families, with full workspace verification passing
- Schema-aware cache keys and disabled-cache proof for the Phase 7 parser/fact cache.
- Source-free Go and TS/JS parser facts cached under `.polint/cache` with deterministic restoration.
- Rayon-backed file and adapter analysis with deterministic `AnalysisDb` merge and repeated CLI output proof.
- Closed Phase 7 with deterministic per-rule profiling output and end-to-end cache/performance verification.
- CLI command surface and exit-code contracts are covered by integration tests, and `test-rules` no longer corrupts machine-readable stdout.
- SARIF-like output now has renderer snapshot coverage and CLI integration proof for CI fields and fail thresholds.
- Import and function graph DOT exports now have unit and CLI coverage for deterministic, valid output.
- Phase 8 targeted and full workspace verification passed, with SARIF-like output hardened against feature-dependent JSON field ordering.
- README now gives a complete v1 user path for installation, quickstart, config, SDK authoring, rule testing, CI, examples, release checks, and roadmap.
- Top-level examples now document the quickstart, custom rule authoring helpers, and runnable Go/TS example-rule fixtures with honest v1 limitations.
- CLI integration coverage now proves mixed Go/TS fixtures and the configured Go/TS example directories run through their local native rule hosts.
- Phase 10 closes with README/examples verified, targeted fixture/example CLI tests passing, and the full Rust workspace release matrix clean.

---
