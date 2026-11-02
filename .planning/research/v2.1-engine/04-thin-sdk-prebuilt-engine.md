# L1: prebuilt engine + thin SDK

**Researched:** 2026-11-02 against `main` at `338b570b` (v0.5.0) and `origin/research/perf-levers` (PR #130). Read-only code research; every claim carries a path.

Today a repo-local rule pack (`.polint/rules`, a Cargo crate depending on the full `polint` crate) is compiled into a "rule host" binary; the engine compile is 97–99% of ice-cold time (~200 s, 3.2 GB RSS) while a thin-SDK dependency floor builds in 15.9 s / 328 MB.

## 1. The design as researched

Source of record: `research/code-preserving-rule-build/{FINAL-REPORT,IMPLEMENTATION-PLAN}.md` on main (never released; only the Phase A harness `polint-bench build-cost` landed), re-ranked in `thoughts/perf-5.0-levers/{01,10,11}.md` on `origin/research/perf-levers`.

**Shape.** The installed `polint` binary runs the kernel and writes an owned fact snapshot; the pack compiles against a renamed `polint-sdk` crate (`polint = { package = "polint-sdk", ... }` so `polint::sdk::prelude::*`, `#[polint::rule]` and `polint::runner::run_cli` keep their paths; rule `.rs` files unchanged). `polint::sdk::__private::AnalysisDb` becomes a snapshot-backed read-only type; views keep `&'a AnalysisDb` borrows and `&'a [Fact]` returns (invariant I3).

**Snapshot protocol (plan §4.4, §5).** Container `PSNAP1`: magic, `container_version: u16`, JSON header (`schema = "polint-fact-snapshot-v1"`, engine version, `sdk_abi`, `content_digest`, per-family sections each with its own codec, JSON in v1, binary as a section-level switch). Written to `<cache>/snapshots/<digest>.polintsnap` (file, not pipe; one write shared by every pack; `Scratch` cache role). The host spawns one rule process per pack per invocation and drives it over length-prefixed (`u32` LE + JSON) stdio frames: Hello/manifest handshake, `RunRequest` (snapshot path + digest, `enabled_rule_ids`, `RuleOptionsWire`, `CapabilitySupportView`, inline changeset, kind), then `RunResponse` (`Vec<Diagnostic>`; host computes `RuleExecutionRow`s itself). `SnapshotRequest` is derived from the union of rule manifests, so only requested families are serialized.

**What crosses.** Source files (with text when `syntax` is requested), the syntax/import/go_tests/metrics/symbol/module families, the `stable_keys` table (ids must resolve to identical text; ordering depends on it), capability support, options, changeset. Policy queries do not cross: `Events`/`Calls`/`ControlFlow`/`DataFlow` call `policy_queries::*` and `flow_queries::flows` with runtime-constructed patterns (`crates/polint/src/sdk/facts.rs:886-986`), so they become synchronous RPC frames (`PolicyQueryFrame`/`PolicyResultFrame`) answered by the host between request and response.

**Versioning.** `sdk_abi: u32` bumped on any fact-row change; host declares an accepted range; mismatch, schema mismatch, or `content_digest` mismatch are hard exits (codes 3/4/5), never best-effort decode. The host embeds the SDK sources and `[patch]`es packs onto them (§6.6) so the SDK version always matches the host.

## 2. Alternatives and what breaks

| | What breaks / cost |
|---|---|
| **a. Thin SDK + snapshot (researched pick)** | `core/db.rs` (6,599 lines) must split into read model vs engine (E2); `OnceLock` indexes (`call_graph_index`, `go_types_index`, `db.rs:260-262`) must be rebuildable SDK-side from shipped edges. Four policy views become RPC. The three-crate graph fails `workspace_has_only_two_publishable_product_packages` (`tests/internal_architecture.rs:16`) and rewrites ARCHITECTURE.md's two-package contract. `tests/public_surface_leak.rs` demands the `ALLOWED_PRELUDE` set match exactly; names survive, but `RuleOptions` holds `toml::Value` and is not serde, so a hidden wire type is needed, and `polint_sdk::{protocol,snapshot}` must stay `#[doc(hidden)]`. Byte-identity: today the parent merges the child's JSON report (`cli/mod.rs:4780+`); under L1 diagnostics arrive as frames and rows are computed host-side, so E4 (goldens + 20× reversed-order determinism) is the gate. `polint test` fixtures spawn `cargo run` per case (`rule_test.rs:331-381`) and need one snapshot per case driven through the same process (§6.4). `polint test --unit` (`cli/mod.rs:2852-2875`) keeps working and gets cheaper. The research also moves the rule process cwd to a scratch dir (§7.5); that would break the OAIZ pack's four `std::fs` readers, so native mode should keep cwd = repo root. New family audit needed: the macro now maps 26 views (`polint-macros/src/lib.rs:596-624`) including `Routes`, `GoTypes`, `CallGraph`, `Cfg`, `TsComponents` that post-date the research. |
| **b. cdylib / abi_stable / stabby** | Rejected (FINAL-REPORT §6.4): no stable Rust ABI; every fact struct becomes `repr(C)` with `RVec`/`RStr`, so rules stop being plain typed Rust; couples pack builds to the exact host rustc; in-process code with no boundary; conflicts with the `unsafe_code` policy. It also saves nothing unless the SDK split happens first (the cdylib would otherwise still link the engine). Its one real advantage, zero snapshot tax, only matters if E3 fails. |
| **c. Crate split (L7)** | Measured ceiling ~41 s of 206.5 s (659 CPU-s / 4 jobs = 165 s floor); `polint`'s frontend is 46.1 s single-threaded. Breaks the two-package test and widens `pub(crate)` across crates. Downgraded. |
| **d. WASM** | Does not remove scenario-A compilation (still needs rustup target + SDK compiled for wasm), adds a runtime to the single static binary, 4 GiB guest memory and a copy, `wasip2` Tier 2; needs the same SDK split first. Kept as a later gate for shared packs / untrusted repos. |

Also rejected: prebuilt SDK rlibs (rustc-pinned, ≥70% hit-rate gate) and lazy RPC views (would force owned `Vec` returns).

## 3. The E1/E3 prototype plan and gates

- **Step 0 (measured):** thin-SDK dependency floor builds fresh in **15.9 s, 328 MB, 62 units** (serde, serde_json, toml, globset, anyhow, thiserror, rayon, tracing, polint-macros; clap dropped). Critical path is the regex chain.
- **E1, closure (3 days, throwaway):** build a `polint-sdk` spike from `sdk/`, `core/{rule,db,capability,labels,metadata}.rs`, `policy_queries.rs`, `rule_manifest.rs`, `rule_error.rs`, `runner/`, `internal_core/` + JSON report, stub engine paths, point a copy of the OAIZ pack at it via `package = "polint-sdk"`, `cargo --timings` at 4 and 2 jobs. **Kill:** > 52 s at 4 jobs, or > 1 GB peak RSS, or rule sources need changes. **Green:** ≤ 35 s and ≤ 0.6 GB. Judge time, not units.
- **E3, snapshot tax (2 days):** at the point rules start, serialize what OAIZ's rules read with serde_json and one binary codec; also measure "no snapshot" (rule process reads validated layer blobs). **Kill:** > 15% of warm wall on OAIZ core (> ~0.45 s of 2.97 s) or > 25% full repo. **Green:** ≤ 5% core (≤ ~0.15 s); full repo judged against 6.00 s.
- **Decision rule:** both green → E2 (`core/db.rs` split spike) and the phased plan; E1 green/E3 5–15% → do L5 (binary layer cache) first and re-run with its codec; E1 red → shelve, rely on release-binary installs + store-key depth fix + L6 knob.
- Global kill (§9.4): E2 needs > 20% of `analysis_kernel` in the SDK; any non-determinism not fixable by canonical ordering.

## 4. Recommended PR sequence

| PR | Outcome | Gate | Effort |
|---|---|---|---|
| 0. Spike branch, E1+E3 (+E2 probe) | numbers for the decision | E1/E3 thresholds above; no merge | 1 week |
| 1. SDK extraction, unreleased integration branch | `polint-sdk` + `polint-engine` + `polint` facade; packs compile unchanged via rename; three-package test updated; leak-probe green | closure ≤ 35 s; goldens, determinism, `public_surface_leak` unchanged | 1–2 weeks |
| 2. `FactSnapshot` + protocol | host writes snapshot; rule process reads; policy RPC; `RuleOptionsWire`; family audit for the 26 views | byte-identical to a pre-L1 oracle across examples/temp-repo tests; E3 within budget | 2–3 weeks |
| 3. Runner/CLI integration | `run_local_rule_host_kind` drives the protocol; fingerprint keyed on `sdk_abi`; `polint test` snapshot-per-case; `--unit` unchanged; cwd stays repo root | zero `cargo` spawns on warm runs; fixture suite green; store round-trip tests | 1–2 weeks |
| 4. Breaking cutover + docs/action | old path deleted; CONSUMER-SETUP/CACHE/GITHUB-ACTION updated; migration test for unmigrated manifests | cold ≤ 20 s on the 2-vCPU rig; offline | 1 week |

Roughly 6–9 engineer-weeks after a green spike. Do PR 0 after cost-ordered dispatch (shipped in #132), as `11-next-after-metrics-cliff.md` ranks it.

## Facts verified in code (main @ 338b570b)

- Rule host spawn, args, store restore/build: `crates/polint/src/cli/mod.rs:4400-4457, 4587-4630, 4733-4780`; `--unit`: `:2852-2875`; fixture runner: `crates/polint/src/rule_test.rs:331-381`.
- Macro builds views via `FactView::build(db)` from `polint::sdk::__private` (`crates/polint-macros/src/lib.rs:69-78`); `FactView` trait `sdk/facts.rs:1124-1133`; policy views delegate to `policy_queries`/`flow_queries` `:886-986`; lazy indexes `core/db.rs:260-262`.
- Two-package gate `tests/internal_architecture.rs:16`; prelude allowlist `tests/public_surface_leak.rs`.
