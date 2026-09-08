# API visibility hardening plan

**Goal:** Eliminate accidental `pub` inside `crates/polint`, align with [`AGENTS.md`](../AGENTS.md) (Conventions → Public API and visibility), and clear **`unreachable_pub`** warnings on normal `cargo build` / `cargo test` so CI logs and local builds stay quiet without hiding real issues.

**Non-goals:** Redesigning the rule-author SDK or changing semver policy; this is visibility and module-boundary hygiene.

## Current shape (baseline)

- [`crates/polint/src/lib.rs`](../crates/polint/src/lib.rs): **`pub`** only for `runner`, `sdk`, `run_main`, and **`#[cfg(feature = "bench")] pub mod _bench`** (required so `polint-bench` can import `polint::_bench::*`). Everything else is already **`pub(crate) mod`**.
- **`unreachable_pub`** fires because many items *inside* those `pub(crate)` modules are still declared **`pub`**, even though no downstream crate can name them through `lib.rs`. Rust correctly suggests **`pub(crate)`** (or tighter).

## Phase 40 eval boundary

Phase 40's evaluation harness, external benchmark adapters, adaptation records,
baseline comparison records, and promotion gates remain crate-private/internal.
They are allowed to support tests, research summaries, and hidden implementation
work, but they are not a supported SDK, runner API, stable CLI command, public
JSON schema, or docs/facts surface.

Public query-view promotion for call graph, data flow, evidence, benchmark
execution, or eval reports is deferred to Phase 41 and must go through the same
explicit visibility review as other SDK additions.

## Phase 41 promotion audit

Phase 41 promotion is evidence-gated, not roadmap-name-gated. A surface is
stable only when the code, docs, temp-repo tests, bounded behavior, setup
behavior, cache/input behavior, stable JSON contract where applicable, and
no-leak proof all exist. Broad raw graph/database APIs stay deferred.

| Surface | Disposition | Required gates and notes |
|---|---|---|
| `ResolvedImports<'_>` | stable | Documented in `docs/facts/resolved-imports.md`; temp-repo SDK tests; borrowed iterators; setup/dynamic/unsupported statuses; normal cache inputs; no-leak proof. |
| `ModuleGraphFacts<'_>` | stable | Documented with resolved imports; temp-repo SDK tests; file/package helpers only; no raw topology store; normal cache inputs; no-leak proof. |
| `Symbols<'_>` | stable | Documented in `docs/facts/symbols-and-references.md`; temp-repo SDK tests; precision remains visible; no internal semantic rows. |
| `References<'_>` | stable | Documented in `docs/facts/symbols-and-references.md`; temp-repo SDK tests; status/precision-aware helpers; no raw semantic graph. |
| Metric views | stable | `FileMetrics<'_>`, `FunctionMetrics<'_>`, and `ComplexityMetrics<'_>` are documented in `docs/facts/metrics.md`; threshold helpers are bounded over stored facts. |
| `Cfg<'_>` | defer | Reserved capability. Needs public fact design, docs, temp-repo tests, bounded queries, setup behavior, cache/input proof, and no-leak proof before support. |
| `CallGraph<'_>` | defer | Reserved capability. A future API must separate direct/refined/unresolved/dynamic/unsupported/budgeted results. |
| `DataFlow<'_>` | preview | Policy-level view documented in `docs/facts/data-flow.md`; v1.4 backs bounded source/sink/barrier queries while raw graph APIs stay private. |
| `Evidence<'_>` | internal | Evidence remains diagnostic rendering data, not a rule-author SDK view; see `docs/facts/evidence.md`. |
| Effects/Summaries | internal | Private analysis substrate; no public SDK, stable CLI JSON, or docs/facts contract yet. |
| Types/Values/Aliases | internal | Private precision substrate; no public SDK, stable CLI JSON, or docs/facts contract yet. |
| Model packs | preview | Agent-authoring concept only until a checked-in model-pack contract, fixtures, and cache inputs exist. |
| Provider extensions | preview | Existing extension sink remains validated but not a broad public provider API. |
| `polint inspect rule` | stable | Versioned JSON schema, deterministic rule ordering, rule metadata, derived fact views, support rows, docs paths. |
| `polint test` | stable | Versioned JSON schema, deterministic case ordering, positive/negative fixture semantics, no temp-root leakage. |
| `polint facts` | stable | `facts list` and bounded `facts sample` expose only supported public fact fields and reserved capability dispositions. |
| `polint inspect unknowns` | stable | Reports the consolidated public unknown taxonomy queue as versioned JSON without exposing solver or provider internals. |
| `polint unknowns` | stable | Compatibility alias for cap-filtered public setup/resolution gaps and unsupported rows for reserved capabilities. |
| `polint explain` | stable | Explains rule-derived fact views and capability support without provider execution graphs or layer-cache internals. |
| `polint diff` | defer | No stable public contract in Phase 41. |
| `polint eval` | internal | Evaluation and benchmark schemas remain internal; stable public `polint eval` is deferred. |

## Phase 55 preview promotion audit

Phase 55 intentionally adds a preview policy-query vocabulary while preserving
the v1.4 API design contract: rule authors request typed views, construct one
plain query object with `Query::new(required...)`, set explicit option fields,
call one view method, and report `PolicyViolation` diagnostics. The preview
names compile and appear in rule manifests. Unsupported preview families fail
closed with `polint/capability` until query behavior lands. Phase 56 promotes
`Events<'_>` and `Calls<'_>` to preview behavior: Events is syntax-first for
direct call-event matching and Calls is provider-backed for reachability.
Phase 57 promotes `ControlFlow<'_>` for same-function call-event guard and
cleanup checks. Phase 58 promotes `DataFlow<'_>` for bounded
source/sink/barrier policies. Raw graph internals remain private.

| Surface | Disposition | Required gates and notes |
|---|---|---|
| `Events<'_>` | preview | Public policy view under `polint::sdk::facts`; derives supported `events`; documented in `docs/facts/events.md`; syntax-first call-event matching upgrades when provider facts are already present. |
| `Calls<'_>` | preview | Public policy view under `polint::sdk::facts`; derives supported `calls`; documented in `docs/facts/calls.md`; Phase 56 backs bounded reachable-call queries with provider facts. |
| `ControlFlow<'_>` | preview | Public policy view under `polint::sdk::facts`; derives supported `control_flow`; documented in `docs/facts/control-flow.md`; Phase 57 backs same-function call-event guard and cleanup queries with refined call facts and CFG-backed operation order, with MIR/source ordering as fallback. |
| `DataFlow<'_>` | preview | Promoted as a policy-level view, not a raw data-flow graph; derives supported `dataflow`; documented in `docs/facts/data-flow.md`; Phase 58 backs bounded source/sink/barrier behavior. |
| Query structs | preview | `ReachQuery`, `GuardQuery`, `LifecycleQuery`, and `FlowQuery` live under `polint::sdk::policy` and are re-exported by the prelude. Required inputs use `new(...)`; optional knobs are named fields. |
| Pattern structs | preview | `EventPattern`, `SourcePattern`, `SinkPattern`, `GuardPattern`, and `BarrierPattern` live under `polint::sdk::policy` and are re-exported by the prelude. Phase 55 starts with exact strings and explicit lists. |
| `PolicyViolation` and status enums | preview | `PolicyViolation`, `PolicyStatus`, `PolicyPrecision`, and `PolicyConfidence` are public result vocabulary; full evidence semantics are deferred to Phase 59. |
| `Cfg<'_>` | defer | Reserved raw CFG capability `cfg`; not an alias for `ControlFlow<'_>` and still unsupported. |
| `CallGraph<'_>` | defer | Reserved raw call-graph capability `call_graph`; not an alias for `Calls<'_>` and still unsupported. |
| Raw graph, solver, provider, parser, and `AnalysisDb` internals | internal | Must remain unreachable from `polint::sdk::prelude::*`, CLI public JSON, README examples, generated skill text, and `docs/facts/`. |

## Principles (execution checklist)

1. **Default private** in new code; widen only on demand.
2. **`pub(crate)`** for anything shared across `crates/polint` but not part of `sdk` / `runner` / `_bench` contracts.
3. **Do not** widen `sdk` or `runner` exports without an explicit design note and docs update.
4. **`_bench`:** keep stable enough for `polint-bench`; prefer `pub(crate)` on the underlying modules and thin `pub` re-exports only where the bench crate must see symbols.

## Phased rollout

Work in small PRs or commits per area so rebases stay tractable.

### Phase 1 — Leaf internal modules

**Scope:** `cache/` (including `keys.rs`), `config/`, `fs/`, `graph/`, `diagnostics/` (non-SDK parts), `rule_error.rs` if any stray `pub`.

**Actions:**

- Replace top-level **`pub`** on types, fns, and consts with **`pub(crate)`** where every use site is inside `crates/polint`.
- Keep **`pub`** only if something is re-exported through a path that must stay `pub` (unlikely here).

**Verify:** `cargo build -p polint --locked`, `cargo test -p polint --lib --locked`, `make lint`.

### Phase 2 — `core`

**Scope:** Large module; facts, `AnalysisDb`, `Rule`, `run_rules`, etc.

**Actions:**

- Same rule: **`pub`** only for items that `sdk` / `runner` / `_bench` genuinely need as **`pub`** through their module boundaries. Most internals should become **`pub(crate)`** or private with `pub(crate)` accessors.
- Pay attention to **`pub use`** or re-exports from `sdk::prelude` — prelude items come from `core`; those paths stay **`pub`** via `sdk`, not by leaving every `core` helper `pub`.

**Verify:** Full `cargo test --workspace --all-features --locked` (examples embed `polint`).

### Phase 3 — Adapters `go/` and `ts/`

**Scope:** `go/adapter.rs`, `go/mod.rs`, `ts/adapter.rs`, `ts/mod.rs`, tests under `go/tests.rs`, `ts/tests.rs`.

**Actions:**

- In **`adapter.rs`**, most `pub fn analyze_*` and similar are only called from runner/fs paths inside the crate → prefer **`pub(crate) fn`** unless a test-only need forces a different split (then **`pub(super)`** + `#[cfg(test)]` helpers in the parent module).
- **`pub use adapter::*`** in `mod.rs`: after adapter items are `pub(crate)`, the glob re-export still exposes them to sibling modules inside the crate; confirm no accidental **`pub`** leakage at the `go` / `ts` module boundary.

**Verify:** Adapter unit tests + integration tests touching Go/TS fixtures.

### Phase 4 — `runner`, `cli`, `main`

**Scope:** Only symbols that must stay public for binary vs lib split.

**Actions:**

- Tighten **`pub`** on CLI helpers and runner internals to **`pub(crate)`** where the binary only needs `run_main` + crate internals.
- Ensure **`run_cli`** and any stable rule-pack entry points in `runner` stay documented and correctly `pub`.

**Verify:** `cargo test -p polint --test cli --locked`, `polint` binary smoke.

### Phase 5 — `sdk` audit

**Scope:** `sdk/mod.rs`, `sdk/scope.rs`, prelude re-exports.

**Actions:**

- Confirm every **`pub`** item in `sdk` is intentional for rule authors and covered by `#![deny(missing_docs)]` where required.
- Avoid adding new **`pub use`** of internal types; prefer explicit re-exports in `prelude` only.

**Verify:** Doc build `RUSTDOCFLAGS=-D warnings cargo doc -p polint --all-features --no-deps --locked`.

### Phase 6 — Lint hardening ✅

- **`[workspace.lints.rust] unreachable_pub = "deny"`** in root [`Cargo.toml`](../Cargo.toml).
- Bench-only and `_bench` facade items use **`#[allow(unreachable_pub)]`** with a one-line rationale where rustc cannot see reachability through `polint::_bench::*`.

## Acceptance criteria (done = plan complete)

1. **`cargo build -p polint --locked`** emits **zero** `unreachable_pub` warnings (and no new `dead_code` / privacy errors).
2. **`cargo test --workspace --all-features --locked`** and **`make lint`** pass.
3. **Rule-pack compile check:** at least one example under `examples/**/.polint/rules` still builds against the published-style path dependency.
4. **`polint-bench`** still builds with **`--features bench`** on `polint` as today (or with documented import path updates only).

## Tracking

- Execute phases in order; merge Phase 1–2 before large adapter edits to reduce conflict surface.
- Link PRs to this file in the PR description until all phases are checked off.

## Review-rules promotion (sanctioned prelude addition)

`polint review` (see [`facts/changed-files.md`](facts/changed-files.md)) is `polint check`
with rules authored in Rust and gated to a diff against a target ref. Review rules read the
diff as an ordinary typed fact view, exactly like `Imports<'_>` or `Symbols<'_>`. This
requires two new names on `polint::sdk::prelude`, promoted deliberately and recorded here so
the v1.3 leak gate (`crates/polint/tests/public_surface_leak.rs`) stays honest rather than
bypassed.

| Surface | Disposition | Required gates and notes |
|---|---|---|
| `ChangedFiles<'_>` | stable | Documented in `docs/facts/changed-files.md`; maps to the `changeset` capability; empty under `polint check`; populated only by `polint review`; new public fact-view authoring surface. Probe witness `_assert_changedfiles`. |
| `ChangeStatus` | stable | Returned by `ChangedFileRef::status()`; review-rule authors match on it (`Added`/`Modified`/`Deleted`/`Renamed`). Probe witness `_assert_changestatus`. |

`RuleKind` (the `#[polint::rule(kind = "review")]` designation) is intentionally **not**
prelude-exported — it rides `polint::sdk::__private` only, like `RuleMeta`, so it does not
touch `ALLOWED_PRELUDE`. `ChangedFile`, `ChangedFileRef`, and `ReviewChangeset` (the injected
diff store) are reachable through `ChangedFiles` methods and are not prelude names
(`ChangedFile`/`ReviewChangeset` stay `pub(crate)`; `ChangedFileRef` stays `pub` but
unexported). The `ALLOWED_PRELUDE` count moved `97 -> 99` for these two additions, with two
probe witnesses added in the same change.

## Structured evidence promotion (sanctioned prelude addition)

Policy-query diagnostics ship a validated `evidence_v1` envelope. The wrapper type is
promoted so rule authors and host tooling can name it through the prelude without
exposing `EvidenceStore` or other `analysis/evidence/` internals.

| Surface | Disposition | Required gates and notes |
|---|---|---|
| `StructuredEvidenceV1` | stable | Validated JSON wrapper only (`try_from_value`); field private; appears on `Diagnostic::evidence_v1` for policy-query findings; probe witness `_assert_structuredevidencev1`. |

The `ALLOWED_PRELUDE` count moved `+1` for this addition. Internal evidence store /
node / edge / bundle facts remain crate-private.

## Completeness accessor promotion (sanctioned prelude addition)

Rules need to distinguish a clean result from analysis that stopped early or
could not prove coverage. The accessor stays on `RuleCtx`, while three small
typed values are promoted so rule authors can inspect its result without
depending on provider, run-report, solver, or unknown-taxonomy internals.

| Surface | Disposition | Required gates and notes |
|---|---|---|
| `CompletenessView` | stable | Returned by `RuleCtx::completeness()`; limited to capabilities directly requested by the current rule; missing host information reports `unknown`. Probe witness `_assert_completenessview`. |
| `CapabilityCompleteness` | stable | Read-only per-capability row with capability, status, and reason accessors. Provider identities and internal outcome rows remain private. Probe witness `_assert_capabilitycompleteness`. |
| `CapabilityCompletenessStatus` | stable | Closed rule-facing vocabulary for `complete`, `budget_exceeded`, `provider_failed`, `degraded`, and `unknown`; non-exhaustive for compatibility. Probe witness `_assert_capabilitycompletenessstatus`. |

The `ALLOWED_PRELUDE` count moved `119 -> 122`. No provider graph, solver row,
run-report type, or unknown-taxonomy type is promoted.
