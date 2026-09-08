---
gsd_state_version: 1.0
milestone: v2.0
milestone_name: Static Analysis 2.0 Implementation
status: in_progress
last_updated: "2026-09-08T06:59:01Z"
last_activity: 2026-09-08
progress:
  total_phases: 9
  completed_phases: 2
  total_plans: 9
  completed_plans: 9
  percent: 22
---

# State: polint

## Project Reference

See: `.planning/PROJECT.md` (updated 2026-07-10)

**Core value:** Make it easy to express a repo-specific engineering policy as a small rule and run it locally, in CI, and with AI coding agents.

**Current focus:** Phase 65 — Generation Manifest and Metadata Mirroring

## Current Status

- **GitHub:** `emilwareus/polint` (public repository name).
- Active branch policy: do not push directly to `main` unless explicitly instructed; create a feature/fix branch before sharing remote work.
- v1.0 MVP was audited, archived, tagged, and closed on 2026-05-02.
- v1.1 Capability Fulfillment completed the capability plan, resolved imports/module graph, and symbols/references foundations.
- Static-analysis engine research completed on 2026-05-16 in `research/ROADMAP.md`.
- v1.2 Static Analysis Engine Implementation was audited, archived, and closed on 2026-05-27.
- v1.3 Graph Engine Precision completed on 2026-06-06 and its requirements/roadmap are archived to `.planning/milestones/`.
- v1.4 Policy Query Surface requirements and roadmap are complete in `.planning/REQUIREMENTS.md` and `.planning/ROADMAP.md`; v2.0 planning will replace those active files after requirements and roadmap generation.
- Static Analysis 2.0 research was locked on 2026-07-07 in `research/static-analysis-2.0/` and `research/local-semantic-store/`.
- Phase 22 has been shipped for review in PR #22: https://github.com/emilwareus/polint/pull/22.
- Phase 24 has been shipped for review in PR #25: https://github.com/emilwareus/polint/pull/25.
- Phase 29 has been shipped for review in PR #34: https://github.com/emilwareus/polint/pull/34.
- Each v1.2 research PR maps to one GSD phase, in order, from Phase 20 through Phase 41.
- New broad research is not needed by default. Use the relevant research documents referenced by each phase; do additional research only for a concrete implementation gap.

## Current Position

Phase: 65
Plan: 02 (R2 only)
Status: R1 and R2 complete and code-review clean; broader Phase 65 remains open for R3-R6
Last activity: 2026-08-29 - Completed quick task 260829-s1l: Refuse cross-checkout sharing for rule hosts that embed Cargo-provided checkout paths

### Active Milestone Phase Progress

9 phases planned (63-71), 2 complete. Phase 65 (Generation Manifest and Metadata Mirroring) is next; it adds complete-generation discipline and mirrors the kernel's existing identity/invalidation vocabulary behind Phase 64's private store boundary. Phase 70 (Lexical Search) remains the designated scope-cut. Locked decisions (regression budgets, benchmark repo set, search cut) are recorded in `.planning/REQUIREMENTS.md`.

### Open repo-admin action (T-42-04-10)

Add `public surface leak gate (ubuntu-latest)` AND `public surface leak gate (macos-latest)` to GitHub branch protection required checks on `main` and `release/*`. Only a repo admin can configure branch protection; until then a PR can merge with the v1.3 leak gate failing. Source: Phase 42 Plan 04 (`crates/polint/tests/public_surface_leak.rs`).

### Quick Tasks Completed

| # | Description | Date | Commit | Directory |
|---|-------------|------|--------|-----------|
| 260906-1ex | Index deep-analysis joins and stream evidence payloads; inherited Jelly gate remains red | 2026-09-08 | this commit | [260906-1ex-fundamental-deep-analysis-performance-re](./quick/260906-1ex-fundamental-deep-analysis-performance-re/) |
| 260830-nzd | Implement and A/B benchmark byte-identical Polint algorithmic scan speedups | 2026-08-31 | d196786d | [260830-nzd-implement-and-a-b-benchmark-the-polint-1](./quick/260830-nzd-implement-and-a-b-benchmark-the-polint-1/) |
| 260829-s1l | Refuse cross-checkout sharing for rule hosts that embed Cargo-provided checkout paths | 2026-08-29 | this commit | [260829-s1l-fix-cache-sharing-for-rule-hosts-that-em](./quick/260829-s1l-fix-cache-sharing-for-rule-hosts-that-em/) |
| 260829-qto | Final v0.3.2 rule-host-store release review and blocker fixes | 2026-08-29 | c56d69de | [260829-qto-final-release-readiness-review-and-block](./quick/260829-qto-final-release-readiness-review-and-block/) |
| 260829-q4d | Fix the Windows rules-store absolute override test | 2026-08-29 | working tree | [260829-q4d-fix-the-windows-rules-store-absolute-ove](./quick/260829-q4d-fix-the-windows-rules-store-absolute-ove/) |
| 260826-bc1 | Build-cost baseline harness for the repo-local rule-host path (`polint-bench build-cost`, `make build-cost`, measured baseline) | 2026-08-26 | 6a08477f | [260826-bc1-build-cost-baseline-harness](./quick/260826-bc1-build-cost-baseline-harness/) |
| 260811-t-intern-c-factmeta | PRE-SHIP T-INTERN-C: migrate FactMeta/stable_key_owners to StableKeyId | 2026-08-11 | 932cfd0b | [260811-t-intern-c-factmeta](./quick/260811-t-intern-c-factmeta/) |
| 260810-t-intern-b-solver-summaries-audit-fix | PRE-SHIP T-INTERN-B solver/summaries audit-fix: StableKeyId Ord/serde/normalize findings | 2026-08-10 | 39097eeb | [260810-t-intern-b-solver-summaries-audit-fix](./quick/260810-t-intern-b-solver-summaries-audit-fix/) |
| 260810-t-intern-b-rest-solver-summaries | PRE-SHIP T-INTERN-B rest-solver-summaries: intern solver/summaries/slicing identities | 2026-08-10 | 1a2d6238 | [260810-t-intern-b-rest-solver-summaries](./quick/260810-t-intern-b-rest-solver-summaries/) |
| 260810-t-intern-b | PRE-SHIP T-INTERN-B rest-small-facts: intern small fact family identities | 2026-08-10 | 15aaf817 | [260810-t-intern-b-rest-small-facts](./quick/260810-t-intern-b-rest-small-facts/) |
| 260811-inc | Configure a bounded sccache compiler cache shared across worktrees and disable per-worktree incremental compilation | 2026-08-11 | 3b1e3193 | [260811-inc-configure-a-bounded-sccache-compiler-cac](./quick/260811-inc-configure-a-bounded-sccache-compiler-cac/) |
| 260719-t2i | Parallelize eval fixture coverage tests and inspect remaining serial test bottlenecks without running tests locally | 2026-07-19 | 71ef10ae | [260719-t2i-parallelize-eval-fixture-coverage-tests-](./quick/260719-t2i-parallelize-eval-fixture-coverage-tests-/) |
| 260719-rld | Optimize polint integration test suite speed by reducing cli.rs subprocess invocations without building or running tests | 2026-07-19 | 4650d753 | [260719-rld-optimize-polint-integration-test-suite-s](./quick/260719-rld-optimize-polint-integration-test-suite-s/) |
| 260707-jpy | Fix cargo-deny RustSec advisory for crossbeam-epoch | 2026-07-07 | working tree | [260707-jpy-fix-cargo-deny-rustsec-advisory-for-cros](./quick/260707-jpy-fix-cargo-deny-rustsec-advisory-for-cros/) |
| 260707-static-analysis-20-vision | Clarify Static Analysis 2.0 product vision and defer remote registry while preserving registry-ready seams | 2026-07-07 | ec9872b5 | [260707-static-analysis-20-product-vision](./quick/260707-static-analysis-20-product-vision/) |
| 260707-static-analysis-20 | Validate and lock static-analysis-2.0 open-question decisions | 2026-07-07 | ec9872b5 | [260707-static-analysis-20-open-question-decisions](./quick/260707-static-analysis-20-open-question-decisions/) |
| 260623-oy3 | polint review (rules-as-code, diff-gated): kind=review, ChangedFiles fact-view, git changeset, Command::Review + diff gate | 2026-06-23 | a778b8df | [260623-oy3-implement-polint-review-review-rules-as-](./quick/260623-oy3-implement-polint-review-review-rules-as-/) |
| 260607-bzh | Improve Jelly JS recall through native object/array models and computed property key flow | 2026-06-07 | working tree | [260607-bzh-native-computed-js-recall](./quick/260607-bzh-native-computed-js-recall/) |
| 260606-qjp | Deeply research Jelly JS semantics gaps and add failing unit probes | 2026-06-06 | working tree | [260606-qjp-deeply-research-jelly-js-semantics-gaps-](./quick/260606-qjp-deeply-research-jelly-js-semantics-gaps-/) |
| 260606-fkk | Continue recall-focused Jelly JS/TS callgraph improvements from 16.16% F1 to 37.06% F1 | 2026-06-06 | working tree | [260606-fkk-continue-recall-focused-jelly-js-ts-call](./quick/260606-fkk-continue-recall-focused-jelly-js-ts-call/) |
| 260606-ef2 | Iteratively improve Jelly JS/TS callgraph F1 while preserving architecture and benchmark honesty | 2026-06-06 | working tree | [260606-ef2-iteratively-improve-jelly-js-ts-callgraph-f1](./quick/260606-ef2-iteratively-improve-jelly-js-ts-callgraph-f1/) |
| 260606-ea2 | Research and document how to close the Jelly JS/TS callgraph performance gap | 2026-06-06 | implemented | [260606-ea2-research-and-document-how-to-close-the-j](./quick/260606-ea2-research-and-document-how-to-close-the-j/) |
| 260606-c3l | Measure current static-analysis performance and write a dated report under performance/ | 2026-06-06 | implemented | [260606-c3l-measure-current-static-analysis-performa](./quick/260606-c3l-measure-current-static-analysis-performa/) |
| 260605-n0r | Critical review/fix loop for Phase 53 follow-up fixes until two consecutive clean review rounds | 2026-06-05 | implemented | [260605-n0r-loop-critical-review-and-fixes-for-phase](./quick/260605-n0r-loop-critical-review-and-fixes-for-phase/) |
| 260605-mea | Fix Phase 53 review findings: cache dependency guards, budget reasons, and RSS reporting | 2026-06-05 | 9d3337bc | [260605-mea-fix-phase-53-review-findings-wire-cache-](./quick/260605-mea-fix-phase-53-review-findings-wire-cache-/) |

## Deferred Items

Items acknowledged and deferred at v1.2 milestone close on 2026-05-27. These are non-blocking closeout artifacts: legacy quick-task bookkeeping plus UAT audit false positives whose source UAT files are passed with zero open scenarios.

| Category | Item | Status |
|----------|------|--------|
| quick_task | `260502-dql-remove-readme-note-that-the-repository-i` | missing |
| quick_task | `260502-dto-improve-examples-with-real-minimal-linte` | missing |
| quick_task | `260502-ehi-remove-built-in-rules-and-move-example-r` | missing |
| quick_task | `260502-qsd-make-examples-self-contained-with-one-lo` | missing |
| quick_task | `260503-a9n-add-clear-explanatory-comments-to-self-c` | missing |
| quick_task | `260503-adu-rewrite-example-readmes-to-remove-meta-c` | missing |
| quick_task | `260503-ba9-add-multi-rule-example-with-one-local-ru` | missing |
| quick_task | `260503-l2p-publish-main-branch-cli-release-assets-a` | missing |
| quick_task | `260503-l7c-update-publish-workflow-actions-to-node-` | missing |
| quick_task | `260503-leg-build-macos-release-targets-from-the-ava` | missing |
| quick_task | `260503-lht-fix-release-checksum-paths-for-installer` | missing |
| quick_task | `260503-lwv-add-interactive-cli-skill-installer-for-` | missing |
| quick_task | `260503-p7f-add-make-install-command-for-source-inst` | missing |
| quick_task | `260505-e2y-add-readme-try-it-workflow-and-verify-it` | missing |
| quick_task | `260505-ffu-make-polint-check-run-repo-local-rule-ho` | missing |
| quick_task | `260506-iuu-fix-staged-review-findings-for-agent-qua` | missing |
| quick_task | `260507-rap-rule-authoring-platform-hardening` | missing |
| quick_task | `260509-h5x-fix-capability-roadmap-docs-and-add-real` | missing |
| quick_task | `260509-ibk-implement-static-capability-derivation-r` | missing |
| quick_task | `260509-ignores-feature` | missing |
| quick_task | `260509-macro-rule-boundary-hardening` | missing |
| quick_task | `260509-rul-remove-manual-rule-escape-hatch` | missing |
| quick_task | `260509-typed-future-capability-contract` | unknown |
| quick_task | `260510-check-stats` | missing |
| quick_task | `260510-dbv-tighten-public-cli-surface-and-remove-in` | missing |
| quick_task | `260510-dzr-implement-reusable-derived-metric-signal` | missing |
| quick_task | `260510-eur-prompt-before-overwriting-existing-insta` | missing |
| quick_task | `260510-f1n-review-and-harden-reusable-metric-signal` | missing |
| quick_task | `260510-ffh-document-rust-skill-usage-in-agents` | missing |
| quick_task | `260511-gyu-add-compact-yaml-baseline-and-central-ig` | missing |
| quick_task | `260511-i7m-make-the-baseline-file-live-only-at-poli` | missing |
| quick_task | `260512-aop-fix-review-findings-for-baseline-and-mod` | missing |
| quick_task | `260512-h4g-fix-publish-script-to-be-idempotent-afte` | missing |
| quick_task | `260512-tga-research-lifecycle-extensibility-archite` | unknown |
| quick_task | `260512-yml-replace-unsound-serde-yml-dependency` | missing |
| quick_task | `260513-fga-add-customer-facing-symbol-reference-exa` | completed |
| quick_task | `260513-gld-fix-symbol-reference-pr-review-findings-` | unknown |
| quick_task | `260513-hkw-fix-macos-ci-go-symbol-sidecar-test-fail` | unknown |
| quick_task | `260513-jdo-support-go-1-24-for-the-go-symbols-sidec` | missing |
| quick_task | `260513-oy0-research-and-design-monorepo-friendly-go` | missing |
| quick_task | `260513-v1j-fix-final-pr-review-issues-for-monorepo-` | unknown |
| quick_task | `260514-ci-fix-windows-sidecar-null-json` | unknown |
| quick_task | `260514-jjl-speed-up-ci-integration-tests-by-reducin` | missing |
| quick_task | `260515-awz-analyze-and-complete-github-issue-15-shi` | missing |
| quick_task | `260518-lky-fix-phase-24-critical-review-findings-fo` | unknown |
| quick_task | `260518-m6j-optimize-local-polint-rule-host-scan-spe` | missing |
| quick_task | `260518-m7h-fix-follow-up-cache-review-findings-for-` | unknown |
| quick_task | `260518-pu7-fix-ci-native-eval-layer-cache-runtime-b` | unknown |
| quick_task | `260518-qzd-research-and-plan-ai-friendly-polint-che` | missing |
| quick_task | `260519-ci-fix-phase-26-ci-failures` | unknown |
| quick_task | `260519-fqg-fix-pr-review-findings-for-semantic-inde` | unknown |
| quick_task | `260519-naj-fix-phase-27-topology-review-issues` | missing |
| quick_task | `260519-qdf-fix-second-phase-27-topology-review-find` | missing |
| quick_task | `260519-vl1-full-lockfile-based-package-manager-supp` | missing |
| quick_task | `260520-9jr-fix-package-manager-topology-review-find` | missing |
| quick_task | `260520-a6t-fix-pnpm-workspace-package-manager-revie` | missing |
| quick_task | `260520-ai8-fix-package-manager-topology-review-find` | missing |
| quick_task | `260520-c7k-fix-security-findings-around-repo-escape` | missing |
| quick_task | `260520-da2-harden-core-trust-boundaries-and-run-sec` | missing |
| quick_task | `260520-fpj-fix-remaining-go-work-repo-boundary-secu` | missing |
| quick_task | `260520-h6j-fix-phase-28-local-mir-correctness-issue` | missing |
| quick_task | `260520-iba-resolve-pr-33-merge-conflict-against-lat` | missing |
| quick_task | `260520-ii6-merge-latest-main-security-fixes-into-pr` | missing |
| quick_task | `260520-jho-speed-up-ci-with-rust-caching-and-lighte` | missing |
| quick_task | `260521-a5k-fix-cfg-pr-review-findings` | missing |
| quick_task | `260521-af1-fix-cfg-stored-reachability-for-syntheti` | missing |
| quick_task | `260521-b38-fix-cfg-digest-payload-and-stable-unsupp` | missing |
| quick_task | `260521-m9k-fix-critical-pr-review-findings-for-dire` | unknown |
| quick_task | `260521-nem-add-realistic-structured-coverage-for-di` | unknown |
| quick_task | `260522-no8-fix-phase-33-scc-closure-review-findings` | unknown |
| quick_task | `260524-fix-phase35-review-findings` | unknown |
| quick_task | `260524-fix-phase36-closeout-review-proof` | unknown |
| quick_task | `260525-c1a-fix-final-review-findings-for-phase-37-r` | missing |
| quick_task | `260525-d15-fix-ci-failures-from-pr-45-attached-logs` | missing |
| quick_task | `260525-dtr-fix-pr-45-windows-platform-library-test-` | missing |
| quick_task | `260525-fix-phase38-39-review-findings` | missing |
| quick_task | `260525-otb-implement-tdd-data-flow-fixes-for-summar` | missing |
| quick_task | `260525-refined-call-review-fixes` | unknown |
| quick_task | `260526-eq9-remove-unsupported-language-benchmark-ar` | missing |
| quick_task | `260526-fix-graph-review-findings` | unknown |
| quick_task | `260526-fix-windows-platform-library-ci` | unknown |
| quick_task | `260526-graph-engine-benchmark-research` | unknown |
| uat_gap | `Phase 33 33-UAT.md` | passed, open_scenario_count=0 |
| uat_gap | `Phase 34 34-UAT.md` | passed, open_scenario_count=0 |

## Phase Progress

| Phase | Status | Notes |
|-------|--------|-------|
| 20 | Complete | 2/2 plans complete; private kernel facade/delegation plus internal provider manifests/order inspection done |
| 21 | Complete | 4/4 plans complete; provenance, precision, validation metadata, deterministic debug JSON, and public compatibility proof done; requirement SAE-FND-02 |
| 22 | Complete | 6/6 plans complete; evaluation model/report hashing, generic matchers/metrics, native fixture runner, provenance/cache/extension fixtures, fixture category coverage, and public-boundary proof done; requirement SAE-FND-03 |
| 23 | Complete | 5/5 plans complete; input snapshots, typed cache keys, provider output metadata, cache stats, lifecycle/toolchain/rule/model digest inputs, and cache invalidation proof done; requirement SAE-FND-04 |
| 24 | Complete | 5/5 plans complete; persistent layer cache proof, stale-safety, public-boundary coverage, and full verification done; requirement SAE-FND-05 |
| 25 | Complete | 4/4 plans complete; rule manifests, inspect JSON, test fixture runner, and public rule behavior proof done; requirement SAE-FND-06 |
| 26 | Complete | 6/6 plans complete; semantic index contracts, TS/JS and Go semantic rows, validation/debug output, cache persistence, eval fixtures, and public-boundary proof done; requirement SAE-SEM-01 |
| 27 | Complete | 7/7 plans complete; topology contracts, Go/TS topology collectors, provider/cache wiring, module topology provider, eval fixtures, public-boundary proof, and docs alignment done; requirement SAE-SEM-02 |
| 28 | Complete | 7/7 plans complete; private MIR/place contracts, semantic store, Go and TS/JS lowering, provider/cache/debug wiring, semantic-MIR eval snapshots, and public-boundary proof done; requirement SAE-SEM-03 |
| 29 | Complete | 6/6 plans complete; private CFG contracts/storage, shared builder/derived analyses, provider/cache/validation/debug wiring, Go CFG lowering, TS/JS CFG lowering, eval fixtures, and public-boundary proof done; requirement SAE-SEM-04 |
| 30 | Complete | 8/8 plans complete; direct call contracts, provider/cache identity, validation/debug snapshots, MIR call-site extraction, direct targets, unresolved evidence, eval observation/fixtures, and public-boundary proof done; requirement SAE-SEM-05 |
| 31 | Complete | 5/5 plans complete; private domain contracts, deterministic local solver, stored domain facts, provider/cache identity, validation, debug JSON, abstract-domain eval fixtures, public-boundary proof, review fixes, and final verification done; requirement SAE-INT-01 |
| 32 | Complete | 7/7 plans complete; summary kernel contracts, store, builder, provider, cache identity, validation, debug, eval fixtures, and public-boundary proof done; requirement SAE-INT-02 |
| 33 | Complete | 7/7 plans complete; demand queries, summary SCC cache, extension-aware quarantine, eval fixtures, public-boundary proof, review fixes, and final verification done; requirement SAE-INT-03 |
| 34 | Complete | 6/6 plans complete; Rust extension discovery/host/protocol, sink validation, kernel integration, cache identity/quarantine, real extension eval, review fixes, and final verification done; requirement SAE-INT-04 |
| 35 | Complete | 8/8 plans complete; framework fact contracts, provider wiring, Go/TS recognizers, trust boundaries, dispatch, validation, eval fixtures, public no-leak proof, and clippy cleanup done; requirement SAE-INT-05 |
| 36 | Complete | 7/7 plans complete; private type/value/place/alias substrate, validation/debug/eval fixtures, extension precision, public no-leak proof, and final verification done; requirement SAE-PREC-01 |
| 37 | Complete | 6/6 plans complete; refined-call providers, validation, real eval fixtures, public no-leak proof, review fixes, and final verification done; requirement SAE-PREC-02 |
| 38 | Complete | 10/10 plans complete; local value-flow edges, summary projection, stored budget/unknown facts, data-flow eval fixtures, debug rows, public no-leak proof, and final verification done; requirement SAE-PREC-03 |
| 39 | Complete | 7/7 plans complete; private evidence substrate, local slices, bounded/ranked paths, summary context expansion, diagnostic rendering, extension evidence validation, eval fixtures, public no-leak proof, and final verification done; requirement SAE-PREC-04 |
| 40 | Complete | 8/8 plans complete; Go and TS/JS benchmark adapters, comparison rows, adaptation prompt/deltas, baselines, promotion gates, and public-boundary proof done; unsupported-language benchmark scope removed; requirement SAE-PROM-01 |
| 41 | Complete | 5/5 plans complete; public SDK query helpers, agent JSON commands, generated fixture ergonomics, public docs/skills, review fixes, and final verification done; requirement SAE-PROM-02 |
| 42 | Complete | 5/5 plans complete; identity substrate + dedup, Go RelString/Jelly span renderers + CRLF fixture + jelly_oracle_coverage, closed IdentityCategory taxonomy + categorized_failures counter map, public-surface-leak CI gate, and Plan 05 gap closure (Go package-NAME qualification via PackageFact + go_relstring_v2 cache bump + dedup literal total order) done; requirements IDENT-01/02/03 |
| 48 | Complete | 3/3 plans complete; Plan 01 Go-frontend RTA-signal emission + Plan 02 go_rta RTA driver (analysis::solver::go_rta fixpoint via SolverEngine::run_to_solver_output, points-to byte-identical; GoRtaSubBudget + [solver].go config + go_rta_fixpoint_v1 cache key; BudgetExceeded latching) + Plan 03 verification (iteration-cap BudgetExceeded + interface-dispatch instantiated-type filter + address-taken func-value + polyglot Go+TS canary + go_rta determinism fixtures, all green; determinism + leak + provider-order snapshots unchanged). Plan 03 surfaced + auto-fixed 3 Rule-1 Go-frontend bugs (set-fact dedup, bare method-set names, method node-mapping by span-containment) without which RTA resolved zero real interface edges. Requirement GO-05 COMPLETE |
| 49 | Complete | 3/3 plans complete; Plan 01 JS token budgets/config/cache handoff complete; Plan 02 private ts_tokens closed inputs + deterministic token fixpoint + too-many-tokens sentinel + token DerivedEdgeFact dispatch + real TsTokensPolicy complete; Plan 03 native TS token fixtures + token-explosion BudgetExceeded proof + polyglot/determinism/Jelly evidence + full suite/leak/clippy green. Requirement JS-04 COMPLETE. Caveat: current frontend producers cover direct aliases; parameter/return/closure coverage is solver-level over CopyEdge inputs and awaits frontend producer expansion. |
| 50 | Complete | 5/5 plans complete; private TS object-model facts/storage/graph lowering, opt-in flag, object budgets, property-bucket fixpoint, prototype/class/accessor lookup, receiver binding, native fixtures, determinism/polyglot evidence, public leak gate, full regression, and final verification done. Requirement JS-05 COMPLETE. Caveat: external Jelly corpus floors remain Phase 54-owned. |
| 51 | Complete | 4/4 plans complete; private adaptation model schema/loader/store/validator, accepted-only ModelEdge lowering, solver provenance/cache participation, adapted reporting with sandbox/model/held-out evidence, public leak gate, full regression, and final verification done. Requirements ADAPT-01/ADAPT-02 COMPLETE. Caveats: corpus floors remain Phase 54-owned; final refined-call projection and unknown taxonomy remain Phase 52-owned. |
| 52 | Complete | 4/4 plans complete; solver/direct refined-call projection, consolidated unknown taxonomy, canonical `polint inspect unknowns --format json`, legacy unknowns compatibility, schema/docs/skill updates, eval fixture alignment, and final verification done. Requirements GRAPH-05/TAX-01 COMPLETE. |
| 53 | Complete | 4/4 plans complete; V13 cache dependency ledger and provider metadata, solver budget consolidation and evidence, deterministic budget diagnostics, RSS evaluation summary/reporting, review fixes, and final verification done. Requirements CACHE-01/CACHE-02 COMPLETE. |

## Accumulated Context

- Product code and GSD planning documents live together in the repository root on `main`.
- Public API discipline is strict: use `pub(crate)` for internals, promote only curated SDK/runner surfaces, and fix `unreachable_pub` by tightening visibility.
- Rule-author examples and temp-repo tests must consume `polint::sdk::prelude::*` and `polint::runner::run_cli`, not internal modules.
- Capability names must stay honest: unsupported or setup-missing hard capabilities produce capability diagnostics rather than placeholder facts.
- Comment ignores are an engine/reporting concern; individual rules should report the diagnostics they find.
- Go semantic lifecycle must support monorepos without requiring a root `go.mod`; module roots are inferred or configured in `.polint.toml`.
- New analysis modules for v1.2 should stay private until validation and promotion gates justify public SDK or CLI exposure.
- Every new fact family should carry stable IDs, precision/status/provenance, deterministic ordering, cache inputs, validation fixtures, and explicit unknown states.
- Phase 20 Plan 01 added a crate-private `AnalysisKernel` facade that owns the existing source, Go syntax, TS/JS syntax, module graph, symbol graph, and metrics execution order.
- Runner and parent CLI analysis paths delegate provider execution through `AnalysisKernel::run`; rule selection, rule options, ignores, report filtering/rendering, exit behavior, and rule execution remain outside the kernel.
- Phase 20 Plan 02 added deterministic crate-private provider manifests for the six current providers and test-only provider order/report helpers.
- Provider manifests are consumed by production kernel code only for metadata consistency; they do not drive scheduling, diagnostics, or cache identity.

## Decisions

- [Phase 63-01]: polint owns its lint table (unsafe_code forbid->deny, all other workspace lints mirrored) so one audited getrusage FFI opts in via `#[allow(unsafe_code)]`; unsafe stays denied crate-wide otherwise, and the workspace-wide forbid still applies to every other crate.
- [Phase 63-01]: Benchmark repos pinned to real release tags (grafana v11.4.0 `b587018...`, hugo v0.140.0 `3f35721...`, excalidraw v0.17.6 `f164071...`); devloupe is a local-only/non-CI reference via a research-only tier + `local_clone_policy = "allow_absolute"` so CI fast/nightly/release never resolve it.
- [Phase 63-01]: getrusage(RUSAGE_SELF).ru_maxrss is the single source for the previously-never-populated `peak_rss_bytes`, normalized per-OS (macOS bytes / Linux kilobytes); curve-point telemetry (`CurvePoint`/`CurveSeries`) is keyed by repo size + diff size with cache/store size and budget-exhaustion counters, `deny_unknown_fields` + derived `Ord` for deterministic serialization.
- [Phase 48-03]: The `eval::go_rta` acceptance gate sources RTA edges from the kernel-built db by rebuilding `GoRtaInputs::from_db` + driving `SolverEngine::run_to_solver_output`, NOT through `graph_edges_from_kernel_output` (which reads `refined_call_edges`/`call_targets`, not `solver_derived_edges`; Phase 52/GRAPH-05 wires solver edges into the observable refined-call projection). Self-contained fixtures are the always-runnable, x/tools-clone-free proof. Iteration-cap BudgetExceeded is driven by `max_candidates_per_callsite = 1` (one interface invoke, three instantiated implementers), not `max_rta_rounds`, because all exported Go functions are reachability roots so a multi-round chain cannot be built.
- [Phase 48-03]: Verification surfaced 3 Rule-1 Go-frontend bugs masked by Plan 02's synthetic unit tests (which used already-bare method names + exact spans): whole-program SET facts (callsite/address_taken/instantiated_type/dynamic_dispatch) must dedup by stable key in `normalized()`; the method-set must carry bare method names (`Obj().Name()`), not signatures; and `GoRtaInputs` must map Go methods to nodes by span-CONTAINMENT (the SSA point-span lies within the tree-sitter declaration span) + index the bare method name. Without all three, RTA derived ZERO real Go interface edges. The go-rta/polyglot manifests carry no `[[expected]]` rows (the solver signal is crate-private, not an observable manifest fact); the gate is the proof.
- [Phase 48-01]: Go sidecar harvests the RTA rapid-type set from `*ssa.MakeInterface` ONLY — the `*ssa.Alloc`/`MakeMap`/`MakeSlice`/`MakeChan` families are deliberately excluded because allocation alone does not make a type dynamically dispatchable under x/tools RTA (only interface conversion does), so adding them would over-approximate and flood precision. `address_taken` from `*ssa.MakeClosure`/func-value operands; `dynamic_dispatch` detail joins its callsite via `callsite_stable_key`.
- [Phase 48-01]: Schema-pin lockstep — `decode_ndjson_str` strictly pins `GO_SEMANTIC_SCHEMA`, so the Go `SchemaVersion` bump to `polint-go-semantic-2` forced bumping the Rust constant, adding the three new `allowed_kinds`, and updating every NDJSON test fixture (protocol/lower/tests/provider/client) to `-2`. `GO_SEMANTIC_SCHEMA_LABEL` → `go-semantic-facts-2`; the provider parameter digest folds `address_taken_v1`/`instantiated_type_v1`/`dynamic_dispatch_v1` (D-12). New `GoSemantic*Id` newtypes stay in `go/semantic/facts.rs` (not `analysis/ids.rs`); `assert_small_id_contract` unperturbed; public-surface-leak + determinism gates green; `polint.solver` provider-order slot unchanged.
- [Phase 47-03]: `polint.solver` registered in the reserved slot (after `polint.semantic_graph`, before `polint.refined_calls`, D-13); cache key digests upstream output digests (semantic_graph + type_value_alias points-to families) + the `SolverBudget` (D-15); validation enforces the precision ceiling + a bounded D-12 solver↔summary cycle-detection check; determinism gate (10-shuffle byte-identical) and leak gate (`ALLOWED_PRELUDE` unchanged) stay green. Adding the provider touched 11 provider-order snapshot sites (memory floor of ~7 confirmed conservative).
- Keep `AnalysisKernel`, `KernelInput`, and `KernelOutput` crate-private with no new SDK, crate-root public, or CLI surface.
- Preserve the existing eager provider order inside the kernel until provider manifests and order inspection land in Plan 20-02.
- Merge module graph support over the static plan support view, then symbol graph support over module support, before rules run.
- [Phase 20-private-analysis-kernel-facade]: Keep provider manifests crate-private and consume them only for behavior-preserving metadata consistency in this phase.
- [Phase 20-private-analysis-kernel-facade]: Keep provider execution order as explicit AnalysisKernel::run calls; manifest dependency data remains deterministic test metadata only.
- [Phase 20-private-analysis-kernel-facade]: Expose provider order inspection only through #[cfg(test)] crate-private helpers, with no SDK, runner, or CLI contract.
- [Phase 21-provenance-precision-and-validation-metadata]: Metadata stays in an AnalysisDb sidecar rather than widening public fact structs.
- [Phase 21-provenance-precision-and-validation-metadata]: Provider IDs polint.source, polint.go.syntax, and polint.ts.syntax are reused as producer and layer IDs for current source/syntax facts.
- [Phase 21-provenance-precision-and-validation-metadata]: Stable keys are deterministic strings built from sorted, normalized, length-prefixed labeled parts while run-local FactRef IDs remain separate.
- [Phase 21-provenance-precision-and-validation-metadata]: Derived provider metadata uses hard-coded manifest IDs polint.module_graph, polint.symbol_graph, and polint.metrics.
- [Phase 21-provenance-precision-and-validation-metadata]: Symbol, definition, and reference metadata stable keys reuse the existing symbol graph stable_key fields exactly.
- [Phase 21-provenance-precision-and-validation-metadata]: The missing metadata report stays crate-private and test-facing, with a debug assertion keeping the invariant live inside the kernel.
- [Phase 21-provenance-precision-and-validation-metadata]: Stable-key ownership is keyed by (FactFamily, stable_key); conflicting payloads keep existing fact rows but become deterministic validation diagnostics.
- [Phase 21-provenance-precision-and-validation-metadata]: Metadata validation runs after metrics derivation and before KernelOutput is returned to rule execution.
- [Phase 21-provenance-precision-and-validation-metadata]: Provider precision ceilings allow lower-confidence precision labels while flagging syntax providers that claim Exact or SetupAware output.
- [Phase 21-provenance-precision-and-validation-metadata]: Metadata debug JSON remains behind cfg(test) and crate-private AnalysisKernel helpers, with no SDK, runner, or public CLI surface.
- [Phase 21-provenance-precision-and-validation-metadata]: Debug rows use SourceFile.relative_path and explicit row sorting by path/span/name/stable key/run id to avoid machine-local or transient details.
- [Phase 21-provenance-precision-and-validation-metadata]: Public compatibility is proven through a temp-repo external rule importing only polint::sdk::prelude::* and checking metadata-only keys stay out of public JSON.
- [Phase 22-internal-evaluation-harness-mvp]: Keep eval crate-private and internal; no public SDK, runner, crate-root public, or CLI contract was introduced.
- [Phase 22-internal-evaluation-harness-mvp]: Normalize reports by sorting cases, expected items, observed items, and matches before serialization and hashing.
- [Phase 22-internal-evaluation-harness-mvp]: Compute output hashes from canonical JSON with output_hash cleared and runtime durations removed, while preserving runtime pass/fail semantics.
- [Phase 22-internal-evaluation-harness-mvp]: Use a scoped dead_code lint expectation on the eval module until later Phase 22 plans consume the foundation types.
- [Phase 22-internal-evaluation-harness-mvp]: Keep matcher and metric logic crate-private and pure over normalized in-memory eval rows.
- [Phase 22-internal-evaluation-harness-mvp]: Represent matcher outcomes as typed report data instead of outcome strings so metrics can aggregate deterministically.
- [Phase 22-internal-evaluation-harness-mvp]: Clear observed runtime durations from match summaries before deterministic output hashing, preserving pass/fail semantics without wall-clock hash input.
- [Phase 22-internal-evaluation-harness-mvp]: Extend the existing MetricSummary report type from Plan 22-01 instead of adding a duplicate metric report shape.
- [Phase 22-internal-evaluation-harness-mvp]: 22-03 kept native fixture loading, observation, and execution crate-private/test-facing under eval with no public CLI or SDK surface.
- [Phase 22-internal-evaluation-harness-mvp]: 22-03 copies fixture repos into temporary directories before AnalysisKernel::run and rejects symlink escape during fixture copy.
- [Phase 22-internal-evaluation-harness-mvp]: 22-03 sources provider-order observations from AnalysisKernel::provider_manifests() and keeps exact runtime durations out of deterministic output hashes.
- [Phase 22-internal-evaluation-harness-mvp]: 22-04 keeps provenance and cache fixtures crate-private/test-facing with no public CLI, SDK, runner, or crate-root surface.
- [Phase 22-internal-evaluation-harness-mvp]: 22-04 expected fact matching honors producer_id, precision, and status when manifests specify them, with partial stable-key matching for content-hash-bearing metadata rows.
- [Phase 22-internal-evaluation-harness-mvp]: 22-04 derives cache.current_determinism only after cold, warm, and no-cache fixture runs have matching normalized JSON and output_hash values.
- [Phase 22-internal-evaluation-harness-mvp]: 22-05 keeps synthetic observed rows manifest-owned, test-facing, and rejected outside extension fixtures.
- [Phase 22-internal-evaluation-harness-mvp]: 22-05 counts present, accepted, and rejected observed fact statuses separately in eval metrics.
- [Phase 22-internal-evaluation-harness-mvp]: 22-05 represents extension delta evidence with normalized invariant rows and extension.real_sink_active = false.
- [Phase 22-internal-evaluation-harness-mvp]: 22-05 adds no real extension provider activation, merge surface, CLI, SDK, or runner contract.
- [Phase 22-internal-evaluation-harness-mvp]: 22-06 keeps Phase 22 eval proof entirely test-facing with no public eval CLI, SDK export, runner entrypoint, or documented schema.
- [Phase 22-internal-evaluation-harness-mvp]: 22-06 proves suite category coverage by executing every native fixture manifest and requiring passing kernel, provenance, cache, and extension areas.
- [Phase 22-internal-evaluation-harness-mvp]: 22-06 uses repeated minimal public check JSON output as the no-leak and determinism guard.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Layer-cache persistence remains crate-private under analysis_kernel::incremental with no SDK, runner, CLI, or public JSON surface.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Layer payloads use digest-derived blob paths and manifests are published last under .polint/cache/layers.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Invalidation planning fails closed for unknown, schema, provider, lifecycle, toolchain, model, extension, and missing dependency cases.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Existing key structs derive ordering so CacheNode can support deterministic BTreeMap indexes.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Syntax layer identity excludes rule code, rule options, and downstream diagnostic identity; parser reuse is keyed by parser/source/config/lifecycle/provider inputs.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Go and TS/JS syntax layer payloads store normalized facts and parser diagnostics, not raw source bodies or absolute temp roots.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Adapter provider-output metadata reuses validated layer read output digests on hits and computes output digests after recompute misses.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Cache hit/miss/reuse counters remain internal; CLI compatibility is guarded by public PolintReport parsing and no-leak assertions.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Module graph cache identity includes provider/schema, import shape, source/package, config, Go lifecycle, TS/JS lifecycle, absent toolchain/extension slots, and upstream Go/TS syntax output digests.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Module graph cache hits restore normalized facts through AnalysisDb::replace_module_graph_facts instead of bypassing metadata normalization.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Disabled module graph caching records bypasses_disabled and recomputes without reading or writing layer-cache files.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Module graph cache stats remain internal to KernelRunReport and do not change public check JSON.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Symbol graph cache identity includes source/function/package/import inputs, lifecycle/config digests, module graph output digest, syntax output digests, provider/schema identity, and absent extension/toolchain slots.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Symbol graph and metrics cache hits restore normalized facts through existing AnalysisDb::replace_* paths so metadata and public SDK behavior stay compatible.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Metrics cache identity includes source/function inputs, upstream syntax output digests, provider/schema identity, config digest, and absent extension/toolchain slots.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Derived provider cache stats remain internal to KernelRunReport; public check JSON and SDK surfaces are unchanged.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Layer-cache eval uses an explicit capability-requesting AnalysisPlan so all Phase 24 providers run through real cache paths.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: LayerCacheStore rejects invalid manifests before payload reads, including dependency-index schema drift and derived-layer manifests without dependency rows.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: Layer-cache internals remain test/eval-facing only; public JSON, CLI help, SDK, runner, and crate-root surfaces are guarded by integration tests.
- [Phase 24-persistent-layer-cache-for-existing-cheap-facts]: The public cache status contract includes the managed layers category but still does not expose layer-cache internals or provider stats.
- [Phase 26]: Phase 26 context gathered at .planning/phases/26-semantic-index-deepening/26-CONTEXT.md
- [Phase 26-semantic-index-deepening]: Keep semantic index rows crate-private under symbol_graph::semantic with no SDK, runner, CLI, or crate-root public surface.
- [Phase 26-semantic-index-deepening]: Use polint.symbol_graph as producer/layer id for semantic metadata rows.
- [Phase 26-semantic-index-deepening]: Assign semantic run-local IDs by sorted stable keys while keeping stable keys separate from IDs.
- [Phase 26-semantic-index-deepening]: Keep TS/JS semantic rows crate-private under symbol_graph::semantic with no SDK, runner, CLI, or crate-root public surface.
- [Phase 26-semantic-index-deepening]: Use Oxc scopes and references as the TS/JS semantic source, with conservative rows for unresolved, dynamic, external, and unsupported forms.
- [Phase 26-semantic-index-deepening]: Represent TS/JS stable export identities with a native generated discriminator while future plans decide DB/cache publication.
- [Phase 26-semantic-index-deepening]: Use the existing Go lifecycle and sidecar path, adding semantic rows without writing repository lifecycle files.
- [Phase 26-semantic-index-deepening]: Keep Go semantic rows crate-private under symbol_graph::semantic with no SDK, runner, CLI, or crate-root public surface.
- [Phase 26-semantic-index-deepening]: Represent Go setup gaps and unresolved sidecar references as UnknownFallback semantic rows while preserving polint/capability diagnostics.
- [Phase 26-semantic-index-deepening]: Keep semantic closure, generated hooks, validation, and debug JSON crate-private/test-only for plan 26-04.
- [Phase 26-semantic-index-deepening]: Semantic metadata from polint.symbol_graph must not claim FactPrecision::Exact; setup-aware precision is enforced by validation.
- [Phase 26-semantic-index-deepening]: Native generated hooks are polint.symbol_graph rows with source_stable_key, generated_discriminator, and GeneratedHintLookup provenance.
- [Phase 26-semantic-index-deepening]: Keep semantic cache identity and payload restore crate-private under the existing symbol graph provider.
- [Phase 26-semantic-index-deepening]: Use schema symbol-graph-facts-2 for symbol graph layer payloads that include semantic_index rows.
- [Phase 26-semantic-index-deepening]: Reject malformed semantic cache payloads before reuse instead of restoring partial or placeholder semantic facts.
- [Phase 26-semantic-index-deepening]: Keep semantic eval support crate-private/test-facing; no public eval CLI, SDK view, or generic semantic graph API was added.
- [Phase 26-semantic-index-deepening]: Represent semantic unknown statuses explicitly in eval reports so ambiguous, unresolved, dynamic, external, cycle, generated, setup-missing, and unsupported rows count as unknown evidence.
- [Phase 26-semantic-index-deepening]: Document only existing Symbols<'_> and References<'_> behavior; scopes/import closure/resolution-step rows remain internal.
- [Phase 27-layered-module-package-topology-graph]: Keep topology contracts crate-private under module_graph::topology with no SDK, runner, CLI, crate-root, or public docs promotion.
- [Phase 27-layered-module-package-topology-graph]: Use polint.module_graph for base topology metadata and polint.module_topology for import-to-package metadata.
- [Phase 27-layered-module-package-topology-graph]: Advertise only base topology outputs on the existing polint.module_graph provider; import_to_package_edges remains deferred to the later semantic-aware module topology pass.
- [Phase 27-layered-module-package-topology-graph]: Go module topology reuses GoAnalysisConfig::from_loaded so configured module_roots take precedence and nearest go.mod discovery remains centralized.
- [Phase 27-layered-module-package-topology-graph]: go.mod requirements, replace/exclude directives, and go.sum checksum rows remain separate topology facts rather than import or DependsOn edges.
- [Phase 27-layered-module-package-topology-graph]: Missing go.sum evidence for external requirements is represented as explicit MissingLockfile topology uncertainty.
- [Phase 27-layered-module-package-topology-graph]: Represent package-manager and tsconfig evidence as internal repo topology overlay rows until a dedicated manager-evidence fact family is introduced.
- [Phase 27-layered-module-package-topology-graph]: Treat package-lock.json packages as exact lockfile-selected rows while marking pnpm, Yarn, and Bun lockfile presence as unsupported evidence.
- [Phase 27-layered-module-package-topology-graph]: Use workspace: dependency ranges to override the dependency-section kind with RequirementKind::Workspace.
- [Phase 27-layered-module-package-topology-graph]: Base topology is stored by the existing polint.module_graph provider immediately after resolved imports, module nodes, and module edges are replaced.
- [Phase 27-layered-module-package-topology-graph]: Module graph layer payload schema v2 persists base topology rows but keeps import_to_package_edges out for the later semantic-aware topology pass.
- [Phase 27-layered-module-package-topology-graph]: Topology cache identity hashes checked-in manifest, lockfile, workspace, and tsconfig files under topology-relevant roots while preserving absent-only extension handling.
- [Phase 27-layered-module-package-topology-graph]: Add semantic-aware import-to-package facts in crate-private polint.module_topology instead of widening public module graph contracts.
- [Phase 27-layered-module-package-topology-graph]: Run module topology after polint.symbol_graph so semantic import rows are available without creating a provider cycle.
- [Phase 27-layered-module-package-topology-graph]: Reject duplicate cached import-to-package stable keys before restore so stale or conflicting topology payloads are recomputed.
- [Phase 27-layered-module-package-topology-graph]: Kept topology eval observation crate-private and test-facing, with no SDK, runner, CLI, or public crate-root topology API.
- [Phase 27-layered-module-package-topology-graph]: Represented topology expected rows through stable keys, status labels, precision labels, and compact payload fragments instead of raw source or absolute paths.
- [Phase 27-layered-module-package-topology-graph]: Updated existing layer-cache expectations so polint.module_topology is part of the managed provider cache proof.
- [Phase 27-layered-module-package-topology-graph]: Keep Phase 27 topology internals private and prove the boundary with public CLI JSON, help text, and source-surface assertions rather than adding any SDK topology view.
- [Phase 27-layered-module-package-topology-graph]: Document ResolvedImports<'_> and ModuleGraphFacts<'_> as the supported relationship surfaces while explicitly leaving richer package/workspace topology internals outside SDK facts.
- [Phase 28-private-semantic-mir-and-place-identity]: Keep the new analysis module crate-private and expose no SDK, runner, CLI, or public docs surface.
- [Phase 28-private-semantic-mir-and-place-identity]: Use run-local dense IDs only as handles; persistent place and MIR identity is carried by stable keys.
- [Phase 28-private-semantic-mir-and-place-identity]: Represent unsupported semantics as structured rows with source evidence and conservative action labels.
- [Phase 28-private-semantic-mir-and-place-identity]: Keep stored semantic MIR artifacts behind AnalysisDb crate-private accessors and SemanticStore rather than adding SDK or RuleCtx views.
- [Phase 28-private-semantic-mir-and-place-identity]: Use polint.semantic_mir as the internal producer/layer id and map stored MIR precision conservatively, never Exact.
- [Phase 28-private-semantic-mir-and-place-identity]: Treat public-boundary proof as source-surface tests over SDK, runner, docs, README, and _bench.
- [Phase 28-private-semantic-mir-and-place-identity]: Keep Go MIR lowering crate-private under analysis::mir::lower_go with no SDK, runner, CLI, docs, or public JSON surface.
- [Phase 28-private-semantic-mir-and-place-identity]: Draft MIR operations against stable place keys, then resolve to run-local PlaceId values only after PlaceTableBuilder assigns deterministic dense IDs.
- [Phase 28-private-semantic-mir-and-place-identity]: Represent Go calls only as MirOperationKind::Call shape evidence and emit UnsupportedSemanticFact rows for dynamic/control constructs instead of direct-call facts.
- [Phase 28-private-semantic-mir-and-place-identity]: Keep TS/JS MIR lowering crate-private under analysis::mir::lower_ts with no SDK, runner, CLI, docs, or public JSON surface.
- [Phase 28-private-semantic-mir-and-place-identity]: Use Oxc AST nodes only inside the lowering pass; emitted MIR/place rows contain polint-owned IDs, spans, stable keys, roots, projections, operations, and unsupported facts.
- [Phase 28-private-semantic-mir-and-place-identity]: Represent TS/JS calls only as MirOperationKind::Call shape evidence with call-return places; no direct target facts or call graph surface was added.
- [Phase 28-private-semantic-mir-and-place-identity]: Semantic MIR remains private and crate-internal; no SDK, runner, CLI, or public JSON surface was promoted.
- [Phase 28-private-semantic-mir-and-place-identity]: Malformed unsupported semantic rows are stored and rejected by validation so diagnostics carry stable family/stable_key/field/reason evidence.
- [Phase 28-private-semantic-mir-and-place-identity]: Semantic MIR cache identity includes absent extension, model, and toolchain slots even before those inputs exist.
- [Phase 28-private-semantic-mir-and-place-identity]: Keep semantic-MIR eval observation crate-private and test-facing, sourced only from metadata_debug_json_for_test.
- [Phase 28-private-semantic-mir-and-place-identity]: Use compact semicolon payload fragments for MIR eval evidence instead of raw source, AST dumps, absolute paths, or dense IDs as identity.
- [Phase 28-private-semantic-mir-and-place-identity]: Treat Partial semantic-MIR rows as unknown-like evidence in matcher outcomes and metrics.
- [Phase 28-private-semantic-mir-and-place-identity]: Keep semantic MIR/place internals out of public check JSON, inspect JSON, polint test JSON, CLI help, SDK, runner, crate-root public exports, README, and docs.
- [Phase 28-private-semantic-mir-and-place-identity]: Use an external temp-repo rule that requests only supported public fact views to prove existing rule-author workflows remain compatible.
- [Phase 28-private-semantic-mir-and-place-identity]: Offset private MIR/place/unsupported IDs per language output before merge so validation does not cross-wire Go and TS/JS run-local IDs.
- [Phase 29-local-cfg-and-control-dependence]: Keep CFG contracts crate-private with no SDK, runner, CLI, or docs promotion.
- [Phase 29-local-cfg-and-control-dependence]: Use run-local dense IDs only as handles; persistent CFG identity is carried by stable keys.
- [Phase 29-local-cfg-and-control-dependence]: Preserve duplicate CFG rows during normalization so later validation can report conflicts deterministically.
- [Phase 29-local-cfg-and-control-dependence]: Drive language CFG lowering through one shared builder rather than duplicating graph construction per language.
- [Phase 29-local-cfg-and-control-dependence]: Derive reachability, dominators, postdominators, and control dependence from selected graph views instead of storing language-authored derived rows.
- [Phase 29-local-cfg-and-control-dependence]: Use a synthetic unified exit for postdominance and preserve controlling edge evidence on control-dependence facts.
- [Phase 29-local-cfg-and-control-dependence]: Run polint.cfg after polint.semantic_mir and before polint.metrics.
- [Phase 29-local-cfg-and-control-dependence]: Accept an empty CFG provider output until language lowering plans populate real graph rows.
- [Phase 29-local-cfg-and-control-dependence]: Keep CFG validation and debug output crate-private/test-facing with no SDK, runner, CLI, or public JSON surface.
- [Phase 29-local-cfg-and-control-dependence]: Lower Go CFG from private semantic MIR rows and keep raw tree-sitter AST objects out of CFG facts.
- [Phase 29-local-cfg-and-control-dependence]: Keep language CFG lowerers responsible for base nodes/edges only; shared provider code derives reachability, dominance, postdominance, and control dependence.
- [Phase 29-local-cfg-and-control-dependence]: Represent Go spawn, defer, panic, select, goto, fallthrough, and unsupported semantics with typed CFG edges or unsupported control-flow rows instead of exact claims.
- [Phase 29-local-cfg-and-control-dependence]: Lower TS/JS CFG from private semantic MIR rows and keep Oxc AST/span objects out of CFG facts.
- [Phase 29-local-cfg-and-control-dependence]: Merge language base CFG outputs with deterministic run-local ID offsets before deriving shared CFG analyses.
- [Phase 29-local-cfg-and-control-dependence]: Represent TS/JS dynamic, async, cleanup, optional/nullish, throw, and unsupported semantics with typed CFG edges or unsupported control-flow rows instead of exact scheduler/runtime claims.
- [Phase 29-local-cfg-and-control-dependence]: Keep CFG eval support crate-private and test-facing, sourced only from metadata_debug_json_for_test.
- [Phase 29-local-cfg-and-control-dependence]: Use the existing TOML eval fixture manifest format instead of adding JSON fixture files.
- [Phase 29-local-cfg-and-control-dependence]: CFG stable keys must use MIR/body stable identity, not run-local CFG IDs, to avoid cross-language and cross-function collisions.
- [Phase 29-local-cfg-and-control-dependence]: Keep reserved public cfg capability unsupported until a later intentional promotion phase.
- [Phase 30-direct-call-facts]: Call facts remain crate-private under analysis::calls with no SDK, runner, CLI, or docs promotion.
- [Phase 30-direct-call-facts]: CallStore validates target and unresolved site references before publishing indexes.
- [Phase 30-direct-call-facts]: CALLS_PROVIDER_ID is polint.calls and call metadata uses compact status/kind/algorithm/reason/stable-key payload fragments.
- [Phase 30-direct-call-facts]: polint.calls remains crate-private and manifest-owned, with no SDK, runner, CLI, or public call graph promotion.
- [Phase 30-direct-call-facts]: The calls provider runs after polint.cfg and before polint.metrics so direct calls can consume CFG/MIR context before metrics remain unchanged.
- [Phase 30-direct-call-facts]: Calls cache identity includes semantic MIR, CFG, symbol graph, module topology, syntax, lifecycle, config, parameters, and absent extension/model/toolchain slots.
- [Phase 30-direct-call-facts]: Call validation remains crate-private under analysis::calls and is invoked from metadata validation after CFG validation.
- [Phase 30-direct-call-facts]: Calls debug snapshots stay behind cfg(test) and expose relative paths, stable keys, spans, statuses, precision, compact payload labels, counts, and index evidence only.
- [Phase 30-direct-call-facts]: Exact metadata precision from polint.calls is rejected because call facts are setup-aware/conservative internal rows, not public exact facts.
- [Phase 30-direct-call-facts]: Call-site extraction consumes semantic MIR and place rows only; no parser AST or source reparsing dependency was added.
- [Phase 30-direct-call-facts]: Direct targets remain empty in this plan; function-value, dynamic, unknown, setup-missing, and unsupported call evidence is published as unresolved rows.
- [Phase 30-direct-call-facts]: Call output digest proof now covers provider-derived populated sites and unresolved rows, while direct target coverage remains in the later direct-target plan.
- [Phase 30-direct-call-facts]: Direct targets are emitted only from precise resolved ReferenceFact evidence; dynamic/interface/function-token/framework/value-flow cases remain unresolved or unsupported.
- [Phase 30-direct-call-facts]: Native direct target rows use NativeDirect provenance and SetupAware precision under the private polint.calls provider.
- [Phase 30-direct-call-facts]: Provider-derived unresolved rows are filtered off call sites that have a resolved direct target, so precise evidence wins over dynamic-shape uncertainty.
- [Phase 30-direct-call-facts]: Eval call observation stays crate-private/test-facing; no public SDK, runner, CLI, docs, or call graph API was promoted.
- [Phase 30-direct-call-facts]: Call eval payloads use relative path, source span, status/kind/algorithm/reason/provider, and stable-key target identity only.
- [Phase 30-direct-call-facts]: Existing matcher/metrics/report unknown-like status accounting already covered unresolved, unsupported, and setup_missing; plan-specific tests now prove it for call rows.
- [Phase 30-direct-call-facts]: Plan 30-07 kept direct-call fixture coverage internal and test-facing; no public CallGraph API was exposed.
- [Phase 30-direct-call-facts]: Plan 30-07 uses nonzero eval invariants for direct-call debug count and D-10 index coverage instead of fragile exact counts.
- [Phase 30-direct-call-facts]: Plan 30-07 derives missing call-site owner symbols from existing function/symbol facts before call-store indexing.
- [Phase 30-direct-call-facts]: Plan 30-08 kept direct-call internals private and test-facing; no SDK, runner, CLI, README, or docs/facts call surface was promoted.
- [Phase 30-direct-call-facts]: Plan 30-08 kept CallGraph as an inert reserved SDK view whose call_graph capability remains unsupported.
- [Phase 30-direct-call-facts]: Plan 30-08 recorded the verification-only regression task as an empty test commit to preserve the per-task commit contract.
- [Phase 31-p0-abstract-domain-kernel]: Keep abstract-domain contracts and P0 slots crate-private under analysis::domains with no public SDK, runner, CLI, README, or docs/facts promotion.
- [Phase 31-p0-abstract-domain-kernel]: Represent top and unknown causes as private TopReason labels that participate in stable digest parts.
- [Phase 31-p0-abstract-domain-kernel]: Use BTreeMap and BTreeSet ordering for deterministic product state and literal-set digest behavior.
- [Phase 31-p0-abstract-domain-kernel]: Keep solver, transfer, and result cursor APIs crate-private under analysis::domains with no SDK, runner, CLI, README, or docs/facts promotion.
- [Phase 31-p0-abstract-domain-kernel]: Materialize result identity and iteration through stable keys while using run-local IDs only for cursor lookup within a run.
- [Phase 31-p0-abstract-domain-kernel]: Treat calls, unsupported operations, dynamic writes, widening, and iteration budgets as explicit top/unknown events or states rather than silent certainty.
- [Phase 31-p0-abstract-domain-kernel]: Keep domain facts, provider, store, and cache identity crate-private with no SDK, runner, CLI, README, or docs/facts promotion.
- [Phase 31-p0-abstract-domain-kernel]: Normalize domain facts into observation rows and event rows with explicit status and precision labels, including top, unknown, setup, and budget cases.
- [Phase 31-p0-abstract-domain-kernel]: Make abstract-domain cache identity include provider policy, MIR, CFG, calls, symbol graph, module topology, syntax, lifecycle/config, and absent extension/model/toolchain slots.
- [Phase 31-p0-abstract-domain-kernel]: Represent domain bottom/no-info rows as explicit unknown top reasons before validation so malformed unknown rows fail closed.
- [Phase 31-p0-abstract-domain-kernel]: Record compact eval provider-output schema evidence for polint.abstract_domains without exposing a public provider surface.
- [Phase 31-p0-abstract-domain-kernel]: Abstract-domain facts remain internal eval/debug evidence, not SDK or CLI contract.
- [Phase 31-p0-abstract-domain-kernel]: Deterministic top and budget fixture rows use private test-only solver policies rather than changing production solver defaults.
- [Phase 31-p0-abstract-domain-kernel]: Transient domain place IDs are retained in stable keys but not exposed as invalid indexed references.
- [Phase 32-summary-kernel-and-direct-summaries]: Use max instead of saturating_add for CallEffects unresolved_count join to preserve lattice idempotence.
- [Phase 32-summary-kernel-and-direct-summaries]: Re-declare Changed enum locally in summaries::domain rather than importing from domains::lattice to keep module boundaries clean.
- [Phase 32-summary-kernel-and-direct-summaries]: Place AccessKind::join impl in core.rs since it is specific to summary domain join behavior.
- [Phase 32-summary-kernel-and-direct-summaries]: SummaryOutput normalized() sorts by (stable_key, id) then reassigns IDs sequentially, matching CallOutput pattern.
- [Phase 32-summary-kernel-and-direct-summaries]: Each SummaryDomainKind maps to a separate FactFamily variant for independent metadata tracking and removal.
- [Phase 32-summary-kernel-and-direct-summaries]: SummaryPrecision::Local and SetupAware both map to FactPrecision::SetupAware since summary facts are never Exact.
- [Phase 32-summary-kernel-and-direct-summaries]: Use polint.direct_summaries as the producer_id and layer_id for all summary metadata.
- [Phase 32-summary-kernel-and-direct-summaries]: Implement all four domain builders in a single DirectSummaryBuilder::build pass for deterministic output.
- [Phase 32-summary-kernel-and-direct-summaries]: TITO uses simple copy-chain tracing without field-level access paths per D-07/D-10.
- [Phase 32-summary-kernel-and-direct-summaries]: Memory effects treat all PlaceRoot::Parameter variants uniformly as Param(index) since the place model has no separate Receiver root.
- [Phase 32-summary-kernel-and-direct-summaries]: Output digest includes abstract_domains_output_digest as upstream input for cache invalidation when domain results change.
- [Phase 32-summary-kernel-and-direct-summaries]: Provider parameter digest includes all four summary domain IDs and versions for cache identity.
- [Phase 32-summary-kernel-and-direct-summaries]: LayerKind::DirectSummaries and direct_summaries_layer_key include absent extension/model/toolchain slots per D-14.
- [Phase 32-summary-kernel-and-direct-summaries]: Summary validation runs after validate_abstract_domains in the kernel validation sequence.
- [Phase 32-summary-kernel-and-direct-summaries]: Precision ceiling check rejects FactPrecision::Exact from polint.direct_summaries metadata rows.
- [Phase 32-summary-kernel-and-direct-summaries]: Summary debug rows use as_str labels for domain, status, precision, and provenance instead of dense IDs.
- [Phase 32-summary-kernel-and-direct-summaries]: Eval observation maps summary domain names to fact families: control_effects -> summary_control, call_effects -> summary_call, memory_effects -> summary_memory, data_flow_tito -> summary_tito.
- [Phase 32-summary-kernel-and-direct-summaries]: Summary event facts use a single summary_event family rather than per-domain event families.
- [Phase 32-summary-kernel-and-direct-summaries]: Direct-summary eval payload uses semicolon-delimited compact fragments: domain;status;precision;provenance;payload_digest_prefix.
- [Phase 32-summary-kernel-and-direct-summaries]: Direct-summary determinism comparison uses cold/warm/no-cache three-way equality matching the established direct-calls and abstract-domains patterns.
- [Phase 32-summary-kernel-and-direct-summaries]: Direct-summary public-boundary proof uses 21 specific internal markers (provider IDs, domain names, type names, fact families) rather than generic substring markers that would match test naming.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: EntrypointOutput normalized() sorts by stable_key then reassigns sequential IDs from 0, matching the CallOutput pattern.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: EntrypointStore validates referential integrity: trust boundaries and dispatch edges must reference existing entrypoint stable keys via from_output.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: Four new FactFamily variants placed after ExtensionFact: Entrypoint, TrustBoundary, DispatchEdge, UnresolvedFramework.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: TriggerMetadata is a struct with optional fields (method, path, tool_name, event_name, test_name) rather than an enum.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: polint.entrypoints runs after polint.direct_summaries and SCC closure, before polint.extensions in the kernel run sequence.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: Direct summaries provider output uses provider-computed digest via provider_output_for_with_optional_digest, not metadata fallback.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: Upstream dependency digests are cloned before direct_summaries consumes them so entrypoints can reuse them.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: TS/JS test entrypoints use SetupAware precision (not ResolvedStatic) because they depend on test runner configuration being present.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: MCP SDK detection uses @modelcontextprotocol/ prefix matching to cover all possible subpath imports.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: Trust boundaries are per-entrypoint per-source-kind facts derived from EntrypointKind rules per D-19/D-20/D-21.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: HTTP routes produce PathParam (if path has /:id or /{id}), QueryString, RequestBody (POST/PUT/PATCH/DELETE), RequestHeader boundaries.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: Dispatch edges map EntrypointKind to DispatchEdgeKind following D-04 specification.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: Unresolved merge uses BTreeMap by stable key for dedup (first occurrence wins) and deterministic sort.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: Entrypoint fact accessors promoted from #[cfg(test)] to production visibility for validation pipeline access.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: Extension framework facts use FrameworkPrecisionCeiling rejection reason separate from MissingProvenance for Exact precision violations.
- [Phase 35-framework-entrypoints-and-trust-boundaries]: Conflicting entrypoint registrations detected by same target_function with different framework_ids produce warning diagnostics.
- [Phase 42-benchmark-identity-renderers-dedup-identity-taxonomy]: SignatureDigest uses a deterministic length-prefixed two-pass FNV-1a 16-byte digest with a local hex codec instead of sha2/hex (no new deps per T-42-SC; cross-platform byte-identical per D-25; length-prefixed per T-42-01).
- [Phase 42-benchmark-identity-renderers-dedup-identity-taxonomy]: Arc<str> serde uses a field-level adapter because the serde rc feature is not enabled.
- [Phase 42-benchmark-identity-renderers-dedup-identity-taxonomy]: Dedup collapse is order-independent — the canonical retained record is the smallest by sort key (D-11).
- [Phase 42-benchmark-identity-renderers-dedup-identity-taxonomy]: polint.identity manifest slots between polint.calls and polint.abstract_domains (D-23); IDENTITY_SCHEMA_LABEL = identity-facts-1.
- [Phase 42-benchmark-identity-renderers-dedup-identity-taxonomy]: Identity renderers are pure pub(crate) functions over &IdentityRecord (+ &SourceFile for Jelly); renderer shape is driven by container_path encoding (D-06, D-07).
- [Phase 42-benchmark-identity-renderers-dedup-identity-taxonomy]: CRLF->LF normalization happens at render time only; a multi-line CRLF/LF fixture proves byte-identical Jelly output (D-12, D-13, D-25).
- [Phase 42-benchmark-identity-renderers-dedup-identity-taxonomy]: Both eval adapters consume the renderers as the single source of truth; the inline jelly_span_identity formatter is deleted (D-05).
- [Phase 42-benchmark-identity-renderers-dedup-identity-taxonomy]: MetricSections gains jelly_oracle_coverage (#[serde(default)]); MetricSummary shape is frozen and locked by a destructure test; coverage is a deterministic matched/total count >=0.99 (D-15, D-20, D-22).
- [Phase ?]: [Phase 42-04] v1.3 public-surface-leak gate installed: excluded no_implicit_prelude probe + locked ALLOWED_PRELUDE (97 entries) snapshot-checked vs sdk/mod.rs; Approach B (no trybuild); leak-gate CI on ubuntu+macos fail-fast:false (D-18)
- [Phase ?]: [Phase 42-04] Leak-gate test relocated to crates/polint/tests/public_surface_leak.rs (workspace-root tests/ is not a crate; --package polint --test only resolves there); probe import is use ::polint::sdk::prelude::*; (leading :: required under no_implicit_prelude); probe carries its own committed Cargo.lock for --locked CI
- [Phase ?]: [Phase 42-04] Negative control proved both layers: Rust E0365 forbids pub use of a pub(crate) type into the public prelude; allowlist_matches_prelude_source catches a genuinely-pub addition with an UNSANCTIONED diff. Phases 43-54 must extend ALLOWED_PRELUDE + bump count(97) + add a probe witness to ship any new public type
- [Phase 42-benchmark-identity-renderers-dedup-identity-taxonomy]: [42-03] IdentityCategory is a closed five-variant enum (WrongIdentity, UnsupportedEdge, UnresolvedEdge, PackageLoadLimitation, ModelMissing) in pinned source order with #[repr(u8)] explicit ordinals; declaration order defines serde + Ord byte-stability (D-14, D-25); no Other/Unknown, no #[non_exhaustive].
- [Phase 42-benchmark-identity-renderers-dedup-identity-taxonomy]: [42-03] categorize maps every UnresolvedCallReason/CallTargetStatus variant explicitly (exhaustive match, no wildcard) so a new upstream variant is a compile error (Pattern H); it is a tag on existing facts (CategorizeReason) with zero new fact families (D-16).
- [Phase 42-benchmark-identity-renderers-dedup-identity-taxonomy]: [42-03] MetricSections gains categorized_failures: CategorizedFailureSection (#[serde(default)]) sibling AFTER jelly_oracle_coverage; MetricSummary shape frozen (destructure layout-lock test green); five u32 snake_case counters with deny_unknown_fields; record_category uses saturating_add (T-42-03-05).
- [Phase 42-benchmark-identity-renderers-dedup-identity-taxonomy]: [42-03] categorized_failures threads from the live AnalysisDb (O(n) per-fact projection) through per-category observed invariants into the report section across all eval build paths; the fixture asserts byte-stable .nonzero booleans for determinism-gate safety.
- [Phase 42-benchmark-identity-renderers-dedup-identity-taxonomy]: [42-03] Native syntactic Go/TS emits only unsupported_edge/unresolved_edge; the fixture proves those two from real source and eval::metrics unit tests drive categorized_failures_from_db for wrong_identity/package_load_limitation/model_missing (BLOCKER #4 fallback) so all FIVE counters are non-zero across the corpus (D-15, no scope reduction).
- [Phase 42-05]: Go identity records resolve the PackageFact package-clause NAME (foo.Bar) via package_or_module_for_record; non-Go keeps db.path_for byte-identical. Full module import path deferred to Phase 46.
- [Phase 42-05]: Dedup canonical selection + final sort use record_total_order_key (record_sort_key extended with originating_call_site_id, originating_call_target_id, signature_digest) for a literal total order; byte-stable across input order (CR-03 closed for Phase 43).
- [Phase 42-05]: Go RTA oracle key stays on display_name with an inline Phase 46 deferral note; cache trip-wire bumped go_relstring_v1 -> go_relstring_v2.
- [Phase ?]: [Phase 43-01]: reachability enums use pinned order + serde rename (no repr(u8)); polint.reachability runs after polint.entrypoints with SetupAware ceiling; configured-unresolvable roots become Unresolved.
- [Phase ?]: [Phase 43-01]: [reachability] roots config lives in crates/polint/src/config/mod.rs; configured roots passed to discovery as &[String] from LoadedConfig since InputSnapshot carries only digests.
- [Phase 43-02]: ScoringMode uses per-variant serde rename for kebab wire strings (oracle-rta/oracle-jelly/whole-repo), not rename_all; the required non-Option scoring_mode field gates structurally (deny_unknown_fields) + via an explicit validate() guard.
- [Phase 43-02]: Reachable-set BFS extends the frontier only on Resolved direct-call targets; the CallReachabilityFact marking is composed by call-site stable key and analysis::calls is never mutated; the provider seeds the traversal with explicit real-function roots before storing.
- [Phase 43-02]: oracle-rta filters scored edges to the reachable-from-roots set (unmarked edges fail closed); oracle-jelly/whole-repo score the full set; the backwards-mode footgun is guarded by an oracle-rta-subset-of-oracle-jelly regression test.
- [Phase 43-03]: SolverMetricSection (solver_step_count/budget_exceeded_reasons) reserved on a #[serde(default)] MetricSections section, NOT the frozen MetricSummary, defaulted 0/empty for Phase 47+ so the byte-identity determinism gate stays stable across the milestone (D-23).
- [Phase 43-03]: The determinism gate runs N=10 seeded permutations of provider-enumeration order + observed row-insertion order through the live normalize_run path, driven by provider_manifests() so phases 44-54 auto-enroll (D-22), with per-fixture >=1 root / >=1 call site / >=1 in_reachable_graph=false invariants (D-24).
- [Phase 43-03]: determinism-gate CI job mirrors leak-gate (ubuntu+macos, fail-fast false, independent passes); the Go fixture's unreachable mark needs only tree-sitter call sites so the gate passes without a Go toolchain, matching the no-Go leak-gate analog (D-24/D-25).
- [Phase ?]: [Phase 44-semantic-graph-skeleton]: Module node composes core::ModuleNodeId (PATTERNS V3); there is no ModuleId type.
- [Phase ?]: [Phase 44-semantic-graph-skeleton]: Closed NodeKind/EdgeKind enums use pinned-order + serde-rename + as_str() + lock tests, never #[repr(u8)] (PATTERNS V2).
- [Phase ?]: [Phase 44-semantic-graph-skeleton]: normalized() assigns dense SemanticNodeId/SemanticEdgeId only after the stable-key sort and remaps edge endpoints to the post-sort node numbering (D-05).
- [Phase ?]: [Phase 44-semantic-graph-skeleton]: SemanticGraphStore builds both outgoing (source) and incoming (target) adjacency in one post-normalization pass; the incoming index feeds the Phase 47 solver fixpoint (D-14).
- [Phase ?]: [Phase 44-semantic-graph-skeleton]: SemanticGraphOutput carries nodes+edges only; the constraints field is deferred to Plan 02.
- [Phase 44-02]: ConstraintFact mirrors PointsToConstraintFact and reuses points_to PointsToStatus/PointsToPrecision field types (D-10); ConstraintKind stays separate from PointsToConstraintKind with no import/merge (D-09; folding deferred to Phase 47).
- [Phase 44-02]: ModelEdge is a fieldless reserved variant emitting zero constraints (no producer until Phase 49); build_semantic_graph also emits zero Alloc/Field/Type constraints honestly (no endpoint bridge) rather than fabricating nodes to inflate recall (D-07).
- [Phase 44-02]: normalized() remaps constraint payload node references to the post-sort dense node numbering (mirroring the edge-endpoint remap); the store builds a constraints-by-ConstraintKind index and rejects dangling constraint node refs.
- [Phase 44-02]: build_semantic_graph is a read-only projection over functions/packages/scopes/call_sites/values, composing stable keys from referenced identity (D-06) and mutating no upstream family (D-13).
- [Phase 44-03]: polint.semantic_graph provider folds every consumed upstream provider output digest + schema/parameter into its output digest with an empty-output sentinel (D-17); deferred SC3 inputs (MIR/CFG/summaries/adaptation-models/solver-budgets) are self-documented and digested as zero until Phases 47/49/51/53 land producers.
- [Phase 44-03]: semantic-graph precision ceiling rejects the exact-equivalent tier (SemanticPrecision::ResolvedStatic) since the graph precision enums carry no literal Exact variant; replace_semantic_graph_facts routes through SemanticGraphStore::from_output for normalize + referential validation.
- [Phase 44-03]: the provider auto-enrolls into the Phase 43 determinism gate via provider_manifests() (no gate edit); a dedicated eval::semantic_graph_snapshot gate proves byte-stable Go + TS/JS constraint emission, and the Phase 42 public-surface-leak gate stays green unmodified.
- [Phase 47-01]: New private analysis::solver module registered between slicing and stable_key; all types pub(crate); solver/mod.rs carries the D-04 naming-collision guard (unified core vs points_to sub-domain) and the D-11 dependency contract (closed input set / single fixpoint per run / bounded outer iterations).
- [Phase 47-01]: Folded points_to::solver in BY COMPOSITION (D-03) — PointsToPolicy::solve invokes the existing solve_points_to fixpoint in place; equivalence test proves points-to-via-engine == solve_points_to; points-to snapshot/determinism fixtures byte-identical.
- [Phase 47-01]: SolverBudget WRAPS (not aliases) the points-to budget (D-05): cross-domain max_steps + max_outer_iterations + a PointsToSubBudget channel, projected onto PointsToBudget via points_to_budget(); PointsToBudget::default (10_000/64/512) unchanged.
- [Phase 47-01]: BudgetStatus closed enum (WithinBudget/BudgetExceeded/NotRun) is pinned-order byte-stable with no repr(u8); budget exhaustion surfaces honestly (D-06), never a silent drop.
- [Phase 47-01]: SolverPolicy trait ships exactly ONE real impl (points_to) + two honest stubs (GoRtaPolicy reserved for Phase 48 GO-05, TsTokensPolicy reserved for Phase 49 JS-04) that derive nothing (D-07).
- [Phase 47-01]: SolverEngine owns a deterministic policy-index VecDeque worklist + SolverBudget enforcement + monotonic u64 step counter (for Plan 02 provenance solver-step), driving policies to a single fixpoint per run.
- [Phase 47-02]: DerivedEdgeProvenance (D-08) carries contributing facts total-ordered by stable ID (sorted + de-duplicated in ::new), the producing ConstraintKind::as_str() label (owned String so the fact derives Deserialize), and the engine's monotonic u64 solver step; ContributingFact stores only the stable_key (the FactFamily label is folded into it via stable_key_from_parts).
- [Phase 47-02]: FactFamily::SolverDerivedEdge + DerivedEdgeFact (serde-skip dense DerivedEdgeId, reuses PointsToStatus/PointsToPrecision); derived edges reject FactPrecision::Exact via derived_edge_precision_ceiling (no arm maps to Exact, D-06), locked by an exhaustive unit test.
- [Phase 47-02]: SolverOutput/SolverStore mirror semantic_graph store — normalized() sorts by (stable_key, id) then assigns dense IDs (shuffle-stable), from_output validates duplicate stable keys + the precision ceiling, SOLVER_PROVIDER_ID = "polint.solver" (provider registration deferred to Plan 03).
- [Phase 47-02]: engine::derive_edges computes the transitive CopyEdge closure over a deterministic BTree worklist, accumulating the contributing-constraint set per derived edge so provenance is genuinely load-bearing.
- [Phase 47-02]: D-09 deletion property test proves deleting ANY single contributing fact does not reproduce the transitive derived edge; D-10 wires polint explain via a pub(crate) cfg(test)-facing seam (explain_derived_edge_provenance) with NO new public ExplainReport/ExplainRuleRow field and ALLOWED_PRELUDE unchanged.
- [Phase ?]: [Phase 48-02]: GoRtaPolicy stub replaced with a real RTA driver (analysis::solver::go_rta). RTA = CHA filtered by the instantiated runtime-type set seeded from Phase 43 roots; interface-invoke resolved by intersecting the invoked method with the method-sets of instantiated types, func-value by address-taken signature. Resolved edges are DerivedEdgeFacts (caller-node -> callee-node) with DerivedEdgeProvenance (callsite+dispatch+method-set+instantiated-type facts); never exact (Heuristic ceiling), worst-trust status, honest-unresolved preserved (D-04/D-06/D-08/D-09).
- [Phase ?]: [Phase 48-02]: Production routes through SolverEngine::run_to_solver_output (D-02) — the UNCHANGED derive_edges points-to closure + the Go RTA policy edges merge into one normalized SolverOutput under one SolverBudget; points-to output stays byte-identical (points_to_via_engine_equals_solve_points_to + derive_edges_is_shuffle_stable green). Provider drives PointsToPolicy+GoRtaPolicy+TsTokensPolicy; the polint.solver slot snapshot is unchanged.
- [Phase ?]: [Phase 48-02]: GoRtaSubBudget { address_taken_threshold:256, max_candidates_per_callsite:128, max_rta_rounds:32 } mirrors PointsToSubBudget; [solver].go config keys overlay via SolverConfig::to_go_sub_budget; SolverBudget::default existing fields stay 10_000/64. Go knobs + go_rta_fixpoint_v1 algo-version join the polint.solver cache key (all 3 locked trip-wire tests updated). Runaway dispatch latches the existing BudgetStatus::BudgetExceeded (D-10/D-12/D-13). Instantiated/address-taken sets seeded whole-reachable (Plan 1 facts carry no per-function attribution); RTA discriminant preserved at dispatch resolution.
- [Phase 50]: Keep TS object-model facts private and lower them through the existing semantic graph constraint vocabulary. — This preserves the v1.3 private-engine boundary, avoids a parallel object graph surface, and gives later solver plans stable Alloc, FieldStore, FieldLoad, CopyEdge, and CallConstraint inputs.
- [Phase 50]: Phase 50-02 keeps the JS/TS object model disabled by default behind `[solver.js] object_model = true` and distinct object-model budget caps. — The object model can add expensive property/prototype/receiver exploration. Keeping it opt-in while folding the flag and every cap into solver parameter/output digests prevents stale cache reuse and preserves existing Go RTA and TS token behavior until benchmark gates approve promotion.
- [Phase 50]: Phase 50-03 derives JS/TS object-model call edges only from callable tokens stored in property buckets, not from property names alone. — This keeps the object model precision-first while still improving recall for justified property-flow cases. Exact and computed buckets stay separate, budget exhaustion is explicit, and prototype/receiver semantics remain deferred to the next plan.
- [Phase 50]: Phase 50-04 resolves prototype and receiver-sensitive object-model edges only through stable prototype/receiver facts, with dynamic mutation left unsupported. — Prototype chains and `this` binding are high-risk precision surfaces. Stable fact-gated lookup with visited-set/depth termination improves justified recall while preventing name/type guesses, unbounded traversal, and broad native/framework modeling from entering JS-05.
- [Phase 50]: Phase 50-05 closes JS-05 with crate-private native object-model gates, closed-input determinism, budget evidence, and polyglot non-interference. — Local Jelly-oriented evidence is self-contained and explicitly scoped to `oracle-jelly` and `whole-repo` fixture modes; no external Jelly corpus floor is claimed before the Phase 54 benchmark promotion gate.
- [Phase ?]: [Phase 63-02] run_repo_perf_point times one capability-gated kernel run (the analysis cost check and review share); the review measurement adds diff-size fields via changeset_for_ref, not a second pipeline. Budget counters folded from live AnalysisDb *::BudgetExceeded: budget_exceeded<-SummaryStatus, tokens_exhausted<-CallTargetStatus, iteration_capped<-DomainStatus. store_bytes explicit 0 until Phase 64. Benchmark sweep skips absent large clones; emission determinism tested via an injected deterministic measurer since real cold/warm timing and peak RSS are volatile.
- [Phase ?]: Store-disabled reference baselines use a distinct StoreDisabledBaseline type (own schema constant); shared BASELINE_SCHEMA_VERSION untouched per Plan 04 contract
- [Phase ?]: Pre-store graph accuracy baseline records recall/precision as null when the gated Jelly/Go x-tools clones are absent; regenerated via POLINT_WRITE_GRAPH_BENCH
- [Phase 63-04]: Store-phase regression gate enforces locked +20% peak-RSS / +25% cold-wall-clock budgets vs the committed StoreDisabledBaseline; is_blocking exposes the fail-not-silent signal a Phase 64+ store phase wires to a non-zero exit (BENCH-03)
- [Phase 64]: Production cache constructors keep semantic-store activation off; only cfg(test) code can enable it during Phase 64. — Preserves the zero-cost disabled path while integration proof is developed.
- [Phase 64]: Schema v1 contains only migration bookkeeping and strictly refuses malformed or future versions. — Keeps Phase 65 ownership intact and prevents destructive downgrade behavior.
- [Phase 64]: Store writers use WAL, foreign keys, a 250 ms busy timeout, and BEGIN IMMEDIATE; contention becomes BusySkipped. — Serializes writers without indefinite blocking or changing policy answers.
- [Phase 64]: Normal maintenance preserves corrupt, invalid, future, and unsafe stores; only an exact verified cache-owned path can be explicitly rebuilt. — Prevents destructive fallback and symlink/path escape.
- [Phase 64]: Semantic store maintenance runs after validation/finalization and records only private StoreStatus telemetry. — Store availability cannot influence policy facts, diagnostics, capability support, or exit semantics.
- [Phase 64]: The cfg(test) isolated benchmark selects store mode through a private child environment key and excludes store bytes from cache_bytes. — Measures real overhead without a public activation contract or double-counting.
- [Phase 64]: Phase boundary measurements compute the diagnostics digest before the isolated point, matching the committed baseline generator's cache-priming order. — Prevents unlike toolchain/cache states from masquerading as store regressions without changing locked budgets.
- [Phase 64]: Public leak scanning bans curated store-specific namespaces, types, crate/schema/SQL identifiers while leaving generic store/row/connection prose legal. — Protects supported contracts with negative-controlled precision.

## Execution Metrics

| Phase | Plan | Duration | Tasks | Files |
|-------|------|----------|-------|-------|
| 47-unified-solver-core-derived-edge-provenance | 02 | 21 min | 3 | 8 |
| 44-semantic-graph-skeleton-constraint-vocabulary | 03 | 15 min | 3 | 13 |
| 44-semantic-graph-skeleton-constraint-vocabulary | 02 | 13 min | 3 | 5 |
| 20-private-analysis-kernel-facade | 01 | 9 min | 2 | 5 |
| 20-private-analysis-kernel-facade | 02 | 9 min | 2 | 2 |
| 21-provenance-precision-and-validation-metadata | 01 | 9h 8m | 2 | 3 |
| 21-provenance-precision-and-validation-metadata | 02 | 14m | 3 | 6 |
| 21-provenance-precision-and-validation-metadata | 03 | 14m | 2 | 4 |
| 21-provenance-precision-and-validation-metadata | 04 | 11m | 2 | 3 |
| 22-internal-evaluation-harness-mvp | 02 | 15 min | 2 | 5 |
| 22-internal-evaluation-harness-mvp | 03 | 12 min | 3 | 7 |
| 22-internal-evaluation-harness-mvp | 04 | 11 min | 2 | 12 |
| 22-internal-evaluation-harness-mvp | 05 | 8 min | 1 | 9 |
| 22-internal-evaluation-harness-mvp | 06 | 9 min | 1 | 4 |
| 24-persistent-layer-cache-for-existing-cheap-facts | 01 | 13 min | 2 | 7 |
| 24-persistent-layer-cache-for-existing-cheap-facts | 02 | 20 min | 3 | 10 |
| 24-persistent-layer-cache-for-existing-cheap-facts | 03 | 16 min | 2 | 6 |
| 24-persistent-layer-cache-for-existing-cheap-facts | 04 | 19 min | 2 | 7 |
| 24-persistent-layer-cache-for-existing-cheap-facts | 05 | 28 min | 3 | 17 |
| 26-semantic-index-deepening | 01 | 12 min | 3 | 5 |
| 26-semantic-index-deepening | 02 | 19 min | 3 | 2 |
| 26-semantic-index-deepening | 03 | 70 min | 3 | 5 |
| 26-semantic-index-deepening | 04 | 23min | 3 | 6 |
| 26-semantic-index-deepening | 05 | 13 min | 3 | 4 |
| 26-semantic-index-deepening | 06 | 17 min | 3 | 14 |
| 27-layered-module-package-topology-graph | 01 | 12 min | 3 | 8 |
| 27-layered-module-package-topology-graph | 02 | 14 min | 3 | 5 |
| 27-layered-module-package-topology-graph | 03 | 16 min | 3 | 5 |
| 27-layered-module-package-topology-graph | 04 | 14 min | 3 | 6 |
| 27-layered-module-package-topology-graph | 05 | 23 min | 3 | 12 |
| 27-layered-module-package-topology-graph | 06 | 17 min | 2 | 21 |
| 27-layered-module-package-topology-graph | 07 | 5 min | 1 | 2 |
| 28-private-semantic-mir-and-place-identity | 01 | 19 min | 3 | 12 |
| 28-private-semantic-mir-and-place-identity | 02 | 12 min | 3 | 4 |
| 28-private-semantic-mir-and-place-identity | 03 | 14 min | 2 | 2 |
| 28-private-semantic-mir-and-place-identity | 04 | 17 min | 2 | 2 |
| 28-private-semantic-mir-and-place-identity | 05 | 26 min | 3 | 12 |
| 28-private-semantic-mir-and-place-identity | 06 | 12 min | 2 | 13 |
| 28-private-semantic-mir-and-place-identity | 07 | 11 min | 1 | 6 |
| 29-local-cfg-and-control-dependence | 01 | 18 min | 3 | 7 |
| 29-local-cfg-and-control-dependence | 02 | 24 min | 3 | 4 |
| 29-local-cfg-and-control-dependence | 03 | 34 min | 3 | 12 |
| 29-local-cfg-and-control-dependence | 04 | 28 min | 2 | 4 |
| 29-local-cfg-and-control-dependence | 05 | 31 min | 2 | 3 |
| 29-local-cfg-and-control-dependence | 06 | 68 min | 3 | 19 |
| 30-direct-call-facts | 02 | 8 min | 2 | 9 |
| 30-direct-call-facts | 03 | 12 min | 2 | 4 |
| 30-direct-call-facts | 04 | 17 min | 3 | 7 |
| 30-direct-call-facts | 05 | 14 min | 3 | 9 |
| 30-direct-call-facts | 06 | 5min | 1 | 3 |
| 30-direct-call-facts | 08 | 10 min | 3 | 2 |
| 31-p0-abstract-domain-kernel | 01 | 8 min | 3 | 5 |
| 31-p0-abstract-domain-kernel | 02 | 14 min | 3 | 5 |
| 31-p0-abstract-domain-kernel | 03 | 16 min | 2 | 13 |
| 31-p0-abstract-domain-kernel | 04 | 14 min | 2 | 9 |
| 31-p0-abstract-domain-kernel | 05 | 43 min | 3 | 19 |
| 32-summary-kernel-and-direct-summaries | 01 | 8 min | 2 | 6 |
| 32-summary-kernel-and-direct-summaries | 02 | 5 min | 2 | 4 |
| 32-summary-kernel-and-direct-summaries | 03 | 6 min | 2 | 2 |
| 32-summary-kernel-and-direct-summaries | 04 | 12 min | 2 | 10 |
| 32-summary-kernel-and-direct-summaries | 05 | 9 min | 2 | 4 |
| 32-summary-kernel-and-direct-summaries | 06 | 10 min | 2 | 11 |
| 32-summary-kernel-and-direct-summaries | 07 | 10 min | 3 | 1 |
| 35-framework-entrypoints-and-trust-boundaries | 01 | 5 min | 2 | 6 |
| 35-framework-entrypoints-and-trust-boundaries | 02 | 7 min | 2 | 8 |
| 35-framework-entrypoints-and-trust-boundaries | 03 | 5 min | 1 | 2 |
| 35-framework-entrypoints-and-trust-boundaries | 04 | 4 min | 1 | 2 |
| 35-framework-entrypoints-and-trust-boundaries | 05 | 6 min | 2 | 6 |
| 35-framework-entrypoints-and-trust-boundaries | 06 | 8 min | 2 | 6 |
| 35-framework-entrypoints-and-trust-boundaries | 07 | recorded | 2 | recorded |
| 35-framework-entrypoints-and-trust-boundaries | 08 | recorded | 1 | 2 |
| 42-benchmark-identity-renderers-dedup-identity-taxonomy | 01 | 8h 9m | 2 | 22 |
| 42-benchmark-identity-renderers-dedup-identity-taxonomy | 02 | 1h 5m | 3 | 20 |
| 42-benchmark-identity-renderers-dedup-identity-taxonomy | 03 | 18m | 2 | 11 |
| 43-reachability-roots-per-suite-scoring-mode | 03 | 19m | 4 | 14 |
| 47-unified-solver-core-derived-edge-provenance | 01 | 9 min | 3 | 5 |

## Session

- Last session: 2026-08-31
- Last activity: 2026-08-31 - Completed quick task 260830-nzd after retained H1/H2/H4/H8/H12 waves and the final byte-identity benchmark matrix.
- Stopped at: Quick task complete; PR #104 is pushed and ready for review but remains unmerged.
- Resume file: None.

### Quick Tasks Completed

| # | Description | Date | Commit | Directory |
|---|-------------|------|--------|-----------|
| 260623-oy3 | polint review (rules-as-code, diff-gated): kind=review, ChangedFiles fact-view, git changeset, Command::Review + diff gate | 2026-06-23 | a778b8df | [260623-oy3-implement-polint-review-review-rules-as-](./quick/260623-oy3-implement-polint-review-review-rules-as-/) |
| 260607-e7j | Improve Jelly computed-property recall with bounded key evaluation and accessor flow | 2026-06-07 | implemented | [260607-e7j-computed-property-recall](./quick/260607-e7j-computed-property-recall/) |
| 260605-h96 | Review and fix Phase 52 PR findings until two consecutive clean review rounds | 2026-06-05 | implemented | [260605-h96-review-and-fix-phase-52-pr-findings-unti](./quick/260605-h96-review-and-fix-phase-52-pr-findings-unti/) |
| 260605-gwr | Fix PR review findings: include provider diagnostics in inspect unknowns taxonomy and enforce extension-model invariant value | 2026-06-05 | implemented | [260605-gwr-fix-pr-review-findings-include-provider-](./quick/260605-gwr-fix-pr-review-findings-include-provider-/) |
| 260605-ajl | Fix adaptation delta duplicate model counts and held-out separation | 2026-06-05 | implemented | [260605-ajl-fix-adaptation-delta-duplicate-model-cou](./quick/260605-ajl-fix-adaptation-delta-duplicate-model-cou/) |
| 260605-9zj | Fix adaptation delta model fact and held-out reporting review findings | 2026-06-05 | implemented | [260605-9zj-fix-adaptation-delta-model-fact-and-held](./quick/260605-9zj-fix-adaptation-delta-model-fact-and-held/) |
| 260604-l8k | Finish Phase 50 closeout by adding verification artifact and reconciling roadmap status | 2026-06-04 | artifact-only | [260604-l8k-finish-phase-50-closeout-by-adding-verif](./quick/260604-l8k-finish-phase-50-closeout-by-adding-verif/) |
| 260604-ik2 | Fix final TS object-model review findings | 2026-06-04 | 3f777d47 | [260604-ik2-fix-pr-review-findings-for-ts-object-mod](./quick/260604-ik2-fix-pr-review-findings-for-ts-object-mod/) |
| 260604-g7q | Fix PR review findings for TS object-model eval gates | 2026-06-04 | f6a8a956 | [260604-g7q-fix-pr-review-findings-for-ts-object-mod](./quick/260604-g7q-fix-pr-review-findings-for-ts-object-mod/) |
| 260601-e11 | Fix deep PR review findings | 2026-06-01 | implemented | [260601-e11-fix-deep-pr-review-findings](./quick/260601-e11-fix-deep-pr-review-findings/) |
| 260601-baq | Fix final PR review findings | 2026-06-01 | implemented | [260601-baq-fix-final-pr-review-findings](./quick/260601-baq-fix-final-pr-review-findings/) |
| 260527-d9f | Reconcile v1.2 milestone closeout artifacts before archival | 2026-05-27 | artifact-only | [260527-d9f-reconcile-v1-2-milestone-closeout-artifa](./quick/260527-d9f-reconcile-v1-2-milestone-closeout-artifa/) |
| 260527-auc | Document public ResolvedImports and ModuleGraphFacts examples | 2026-05-27 | implemented | [260527-auc-document-public-resolvedimports-and-modu](./quick/260527-auc-document-public-resolvedimports-and-modu/) |
| 260527-a8t | Fix generated Go rule scaffold heuristic diagnostic disclosure | 2026-05-27 | implemented | [260527-a8t-fix-generated-go-rule-scaffold-heuristic](./quick/260527-a8t-fix-generated-go-rule-scaffold-heuristic/) |
| 260526-uq2 | Fix Phase 41 review findings for generated fixtures and agent JSON contracts | 2026-05-26 | implemented | [260526-uq2-fix-phase-41-review-findings-for-generat](./quick/260526-uq2-fix-phase-41-review-findings-for-generat/) |
| 260526-eq9 | Remove unsupported-language benchmark artifacts and update roadmap to benchmark only Go and TS/JS | 2026-05-26 | implemented | [260526-eq9-remove-unsupported-language-benchmark-ar](./quick/260526-eq9-remove-unsupported-language-benchmark-ar/) |
| 260526-c36 | Capture Phase 40 benchmark comparison and agent-adaptation prompt requirements | 2026-05-26 | implemented | [260526-c36-capture-phase-40-benchmark-comparison-an](./quick/260526-c36-capture-phase-40-benchmark-comparison-an/) |
| 260525-dtr | Fix PR 45 Windows platform library test failure | 2026-05-25 | implemented | [260525-dtr-fix-pr-45-windows-platform-library-test-](./quick/260525-dtr-fix-pr-45-windows-platform-library-test-/) |
| 260525-d15 | Fix CI failures from PR 45 attached logs | 2026-05-25 | implemented | [260525-d15-fix-ci-failures-from-pr-45-attached-logs](./quick/260525-d15-fix-ci-failures-from-pr-45-attached-logs/) |
| 260525-c1a | Fix final review findings for Phase 37 refined calls | 2026-05-25 | implemented | [260525-c1a-fix-final-review-findings-for-phase-37-r](./quick/260525-c1a-fix-final-review-findings-for-phase-37-r/) |
| 260524 | Fix Phase 36 closeout review proof gaps | 2026-05-24 | implemented | [260524-fix-phase36-closeout-review-proof](./quick/260524-fix-phase36-closeout-review-proof/) |
| 260524-jtj | Fix Phase 36 review findings and add regressions | 2026-05-24 | implemented | [260524-jtj-fix-phase-36-review-findings-add-regress](./quick/260524-jtj-fix-phase-36-review-findings-add-regress/) |
| 260524 | Fix PR 41 Ubuntu clippy failures | 2026-05-24 | implemented | [260524-fix-pr41-ubuntu-clippy](./quick/260524-fix-pr41-ubuntu-clippy/) |
| 260524 | Fix deep review entrypoint issues | 2026-05-24 | implemented | [260524-fix-deep-review-entrypoint-issues](./quick/260524-fix-deep-review-entrypoint-issues/) |
| 260522-n3q | Fix Phase 33 review findings with TDD tests | 2026-05-22 | implemented | [260522-n3q-fix-phase-33-review-findings-with-tdd-te](./quick/260522-n3q-fix-phase-33-review-findings-with-tdd-te/) |
| 260521-nem | Add realistic structured coverage for direct calls and abstract domains | 2026-05-21 | implemented | [260521-nem-add-realistic-structured-coverage-for-di](./quick/260521-nem-add-realistic-structured-coverage-for-di/) |
| 260521-m9k | Fix critical PR review findings for direct calls and abstract domains | 2026-05-21 | implemented | [260521-m9k-fix-critical-pr-review-findings-for-dire](./quick/260521-m9k-fix-critical-pr-review-findings-for-dire/) |
| 260521-b38 | Fix CFG digest payload and stable unsupported control-flow keys | 2026-05-21 | implemented | [260521-b38-fix-cfg-digest-payload-and-stable-unsupp](./quick/260521-b38-fix-cfg-digest-payload-and-stable-unsupp/) |
| 260521-af1 | Fix CFG stored reachability for synthetic exits | 2026-05-21 | implemented | [260521-af1-fix-cfg-stored-reachability-for-syntheti](./quick/260521-af1-fix-cfg-stored-reachability-for-syntheti/) |
| 260521-a5k | Fix CFG PR review findings | 2026-05-21 | implemented | [260521-a5k-fix-cfg-pr-review-findings](./quick/260521-a5k-fix-cfg-pr-review-findings/) |
| 260520-jho | Speed up CI with Rust caching and lighter PR platform checks, then measure Actions runtime | 2026-05-20 | implemented | [260520-jho-speed-up-ci-with-rust-caching-and-lighte](./quick/260520-jho-speed-up-ci-with-rust-caching-and-lighte/) |
| 260520-ii6 | Merge latest main security fixes into PR 33 branch and rerun all local checks | 2026-05-20 | implemented | [260520-ii6-merge-latest-main-security-fixes-into-pr](./quick/260520-ii6-merge-latest-main-security-fixes-into-pr/) |
| 260520-iba | Resolve PR 33 merge conflict against latest main and re-review merge readiness | 2026-05-20 | implemented | [260520-iba-resolve-pr-33-merge-conflict-against-lat](./quick/260520-iba-resolve-pr-33-merge-conflict-against-lat/) |
| 260520-h6j | Fix Phase 28 local MIR correctness issues and add edge-case tests | 2026-05-20 | implemented | [260520-h6j-fix-phase-28-local-mir-correctness-issue](./quick/260520-h6j-fix-phase-28-local-mir-correctness-issue/) |
| 260520-fpj | Fix remaining go.work repo-boundary issues and run another security review | 2026-05-20 | implemented | [260520-fpj-fix-remaining-go-work-repo-boundary-secu](./quick/260520-fpj-fix-remaining-go-work-repo-boundary-secu/) |
| 260520-da2 | Harden core trust boundaries, add regression tests, and run a secondary deep security review | 2026-05-20 | implemented | [260520-da2-harden-core-trust-boundaries-and-run-sec](./quick/260520-da2-harden-core-trust-boundaries-and-run-sec/) |
| 260520-c7k | Fix security findings around repo escape reads, workspace glob validation, Go package pattern validation, topology input size limits, and synthetic go.work creation | 2026-05-20 | implemented | [260520-c7k-fix-security-findings-around-repo-escape](./quick/260520-c7k-fix-security-findings-around-repo-escape/) |
| 260520-ai8 | Fix package-manager topology review findings with TDD tests and deep review | 2026-05-20 | implemented | [260520-ai8-fix-package-manager-topology-review-find](./quick/260520-ai8-fix-package-manager-topology-review-find/) |
| 260520-a6t | Fix pnpm workspace package-manager review findings | 2026-05-20 | implemented | [260520-a6t-fix-pnpm-workspace-package-manager-revie](./quick/260520-a6t-fix-pnpm-workspace-package-manager-revie/) |
| 260520-9jr | Fix package-manager topology review findings | 2026-05-20 | implemented | [260520-9jr-fix-package-manager-topology-review-find](./quick/260520-9jr-fix-package-manager-topology-review-find/) |
| 260519-vl1 | Full lockfile-based package manager support for TS/JS topology | 2026-05-19 | implemented | [260519-vl1-full-lockfile-based-package-manager-supp](./quick/260519-vl1-full-lockfile-based-package-manager-supp/) |
| 260519-qdf | Fix second Phase 27 topology review findings | 2026-05-19 | cbb635e | [260519-qdf-fix-second-phase-27-topology-review-find](./quick/260519-qdf-fix-second-phase-27-topology-review-find/) |
| 260519-ci | Fix attached Phase 26 CI failures for manifest version, cross-platform path validation, and layer-cache eval budget | 2026-05-19 | implemented | [260519-ci-fix-phase-26-ci-failures](./quick/260519-ci-fix-phase-26-ci-failures/) |
| 260519-fqg | Fix PR review findings for semantic index keys, validation, lint failures, and rerun deep review | 2026-05-19 | implemented | [260519-fqg-fix-pr-review-findings-for-semantic-inde](./quick/260519-fqg-fix-pr-review-findings-for-semantic-inde/) |
| 260518-qzd | Research and plan ai-friendly polint check output format | 2026-05-18 | implemented | [260518-qzd-research-and-plan-ai-friendly-polint-che](./quick/260518-qzd-research-and-plan-ai-friendly-polint-che/) |

## Next Action

Discuss and plan R3 as a new bounded slice: closed provider execution outcomes,
dependency blocking, planned absence and failure states, plus post-validation
capability revocation without SQLite changes. Phase 65 and
STORE-04/STORE-05/META-01/META-04 remain open. The sub-five-minute CI target
also remains a separate non-blocking follow-up.

## Operator Next Steps

- Re-run `/gsd-discuss-phase 65` specifically for the R3 provider-outcome and capability-revocation slice before creating Plan 03.
- Preserve the restart sequence: R3 provider outcomes/capability revocation → one audited provider family at a time.

## Performance Metrics

| Phase | Plan | Duration | Notes |
|-------|------|----------|-------|
| Phase 64 P01 | 35min | 3 tasks | 7 files |
| Phase 64 P02 | 13min | 3 tasks | 4 files |
| Phase 64 P03 | 16min | 3 tasks | 7 files |
| Phase 64 P04 | 31min | 3 tasks | 4 files |
| Phase 65 P01 (R1) | 1h 35min | 3 tasks + review remediation | 6 implementation/test files |
| Phase 65 P02 (R2) | 40min | 3 tasks + review remediation | 8 implementation/test files |
