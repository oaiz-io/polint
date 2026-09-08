# 05 — Moat Economics: Why Teams Choose polint, Which Moat Is Weakest, What Strengthens It Most

Date: 2026-09-01
Researcher: Claude Fable 5.1 (delegated via Hermes)
Revised 2026-09-02: time estimates removed, capability roadmap added.
Reads with: [02-gap-analysis.md](02-gap-analysis.md) section 6 (moat table), [03-build-plan.md](03-build-plan.md) (where the strengthening feature lands).

## TL;DR

- polint has five candidate moats: rules as compiler-verified typed Rust, AI-agent extensibility with an authoring loop, local-first execution with no cloud, a performance budget, and per-finding honesty with replayable evidence. Three are real today (typed rules, honesty and evidence, local-first). One is half-built (agent extensibility: rules yes, models and extensions not yet public). One is currently inverted (performance: cheap for syntactic rules, out of memory for deep analysis on a medium repository).
- At feature parity a team chooses polint over CodeQL, Semgrep or SonarQube because the policy it wants to enforce is about *its* repository, an agent can write the rule and the compiler proves the rule is well-formed, the result runs offline in seconds, and every finding says how sure it is. None of those four properties is available together anywhere else.
- The buying environment moved in polint's favor in 2025: GitHub unbundled Code Security at 30 dollars per active committer per month and Secret Protection at 19 dollars from 1 April 2025 ([GitHub changelog](https://github.blog/changelog/2025-04-01-github-secret-protection-and-github-code-security-for-github-enterprise/)); Semgrep moved community rules and engine features behind a proprietary license in December 2024 and more than ten vendors forked it as Opengrep on 23 January 2025 ([Socket](https://socket.dev/blog/opengrep-forks-semgrep), [Kodem press release](https://www.kodemsecurity.com/resources/press-release-security-rivals-unite-to-launch-opengrep-following-semgrep-clampdown)). Local-first typed rules with a permissive license are a buying criterion for teams burned twice.
- The weakest moat today is the **performance budget**. The engine's deep tiers cannot run on an 86k-LOC TypeScript repository without exhausting 12 GB, warm runs re-derive everything below the parser cache, and a first `polint check` compiles 225 crates for 187 seconds. A moat you cannot demonstrate on a real repository is not a moat.
- The single feature that strengthens it most is **summary-persisted, frontier-driven analysis under an enforced runtime envelope** (v2.0 Phase 67 plus the envelope from build-plan Stage 0): memory proportional to the working set, warm review proportional to the change, and degradation that is reported instead of fatal. It converts the honesty moat from an apology into a guarantee and makes every deep rule usable where teams actually work.
- Runner-up: the thin-SDK prebuilt rule host (0.3.0 code-preserving build), which takes time-to-first-rule from minutes to seconds and removes the strongest objection to rules-as-code. It strengthens the authoring moat, not the performance moat, and should follow rather than precede the keystone.
- The moat that competitors are most likely to erode is agent extensibility: every incumbent is adding MCP servers and assistants. The defense is verifiability: an agent-written polint rule compiles or it does not, and a model pack lands with a measured default-versus-extended delta. Incumbents can add agents on top of DSLs; they cannot make YAML type-check.
- Risks: a Rust toolchain requirement for rule authors, a Go toolchain for Go semantics and Node for TS types, and the small size of the current benchmark evidence. All three are addressed in the build plan; none is structural.

## 1. The five moats, scored

| Moat | What it is | Evidence it exists | Evidence against | Strength today (0 to 3) |
|---|---|---|---|---|
| Typed Rust rules, compiler-verified | `#[polint::rule]` derives capabilities from typed fact-view parameters; unsupported capabilities refuse to run; rules are ordinary code with fixtures and `polint test` | `crates/polint-macros/src/lib.rs`; `AGENTS.md` authoring contract; decision record `research/agent-rule-authoring/decisions/001-typed-rust-rules-not-dsl-first.md`; precedents at scale: Clippy lints, `go/analysis`, Roslyn analyzers, error-prone are all typed-code lint frameworks | cold rule-host build 187 s and 582.7 MB (`research/evaluation-harness/baselines/build-cost.json`); Rust required for authors | 2 (3 once the thin-SDK build lands) |
| Agent extensibility | scaffold, test, inspect, facts, unknowns, explain, `ai-friendly` output, generated skill text, ten policy templates | `crates/polint/src/cli/mod.rs`; `docs/AGENT-PLAYBOOK.md`; adaptation model layer and extension host exist privately | model packs and provider extensions have no public author surface; only rules are agent-authorable today | 2 |
| Local-first, no cloud | everything runs in the checkout; caches on disk; no telemetry or SaaS | `README.md`; Go sidecar embedded; store private and local | Go toolchain on `PATH` for Go semantics; Node for the planned TS type tier; rule host build needs Cargo and crates.io unless vendored | 3 |
| Performance budget | capability-gated discovery, layer caches, budgets, determinism, 5x warm speedups in v0.3.1 and algorithmic speedups in v0.3.3 (private benchmark repos at about 2 to 3 s) | PR #102, #104 descriptions; `.planning/REQUIREMENTS.md` scale gate | full pipeline OOM at 12 GB on excalidraw; single-threaded analysis; no summary reuse; no runtime envelope | 1 |
| Honesty and evidence | per-fact precision, status, provenance, budget reasons, unknown taxonomy; `evidence_v1` envelopes with paths, unknowns, omitted regions and replay keys on policy findings | `crates/polint/src/sdk/policy.rs`; `diagnostics/mod.rs:237-262`; `polint inspect unknowns` | no rule-level completeness accessor; heuristic labels on name-based sources and sinks are honest but frequent | 2 |

## 2. Why a team chooses polint at parity

Three buyers, and the reason each one picks polint even when CodeQL or Semgrep could express the check:

| Buyer | Policy they want | Why polint at parity |
|---|---|---|
| Platform team running AI coding agents | "our service layer must not call the raw database client; request data must not reach the shell without our validator; GORM model changes need index review" | the rule lives next to the code, an agent writes it from `AGENTS.md` conventions, the compiler and `polint test` verify it, and the diagnostic injects the missing context back into the agent at fix time (`README.md`, Quick Example) |
| Security engineer at a company with source-confidentiality constraints | taint policies over internal frameworks and wrappers | no cloud, no license restricting non-open-source use, models for internal frameworks stay in the repository, findings carry precision and unknown reasons so triage is bounded |
| Engineering lead adopting a monorepo | architectural boundaries, layering, ownership, test-quality gates | L2 facts with explicit unresolved states, byte-deterministic output for CI, baselines for existing debt, review-time gating on the diff |

What the incumbents offer instead: CodeQL offers depth in a query language whose license and pricing are tied to GitHub; Semgrep offers speed and ergonomics in YAML with the deep engine and community rules behind a commercial license; SonarQube offers breadth and dashboards with custom rules gated by edition. All three sell coverage of the world's bugs; polint sells enforcement of one repository's rules.

## 3. The economics of the environment

| Fact | Effect on polint's position |
|---|---|
| GitHub Code Security: 30 dollars per active committer per month; Secret Protection 19 dollars; available to Team plans since April 2025 (GitHub changelog, above) | for a 200-committer organization, CodeQL costs about 72,000 dollars per year before any custom query work; a repo-local Rust engine with a permissive license competes on price with zero marginal cost per committer |
| Semgrep's December 2024 relicensing moved community rules and engine features such as ignore tracking and fingerprinting behind commercial terms; Opengrep forked under LGPL-2.1 with a vendor consortium governing it (Socket, The New Stack coverage above) | buyers now discount "open core" promises; a stable license and a local runtime are worth more than they were in 2024 |
| AI coding agents raise pull-request volume and shift review load to policy enforcement (industry reports on AI-driven code churn; specific figures are **unverified** in this session) | executable, repo-specific policies are the scarce good; generic CWE coverage is not |
| AI reviewers (Cursor Bugbot, Copilot code review, CodeRabbit) sell natural-language rules with no soundness posture and vendor-run accuracy claims | polint's counter is not a better LLM but a verified artifact: a compiled rule with fixtures and a provenance-bearing finding the reviewer can trust |

## 4. The weakest moat: performance budget

The claim in the product thesis is a fast local engine. The measurements say:

- Deep tiers: excalidraw v0.17.6, 86,527 LOC, full pipeline killed at 12 GB or more after about 1,026 s (`scale-corpus-run.json`, 2026-08-08). hugo and grafana were never attempted.
- Latency: no summary persistence, so every warm run below the parser and layer caches re-derives the semantic pipeline; `polint review` gates the diff but does not analyze only the diff.
- First run: a cold rule-host build compiles 225 units in 187 s and retains 582.7 MB; a no-op re-scan still started Cargo before v0.3.2's host store.
- Envelope: budgets bound solver steps, not memory or time; the 30 GB OOM of early 2026 was fixed by capability gating, which one unscoped rule can disengage (`docs/architecture-review/05`, B5).

Syntactic and L2 rules are fast today (about 2 to 3 s warm on private benchmark repositories after v0.3.3). The moat inverts exactly where the ladder gets interesting: a taint policy a team wants to run on its 100k-LOC service cannot run at all. Every other moat is weakened by this one, because typed rules over facts the engine cannot compute are typed rules over nothing.

## 5. The single feature: summary-persisted, frontier-driven analysis under an envelope

Definition: persist per-function and per-SCC summaries with Merkle-shaped keys and dependency package summaries keyed by (package, version); on a warm run recompute only the invalidation frontier; run every provider under an enforced memory and wall-clock envelope that degrades with reported budget facts. This is v2.0 Phase 67 (SUM-01 to SUM-07, REV-01 to REV-03, PERF-04) plus the envelope scheduled in build-plan Stage 0.

Why this feature and not another:

| Alternative | Why it ranks lower |
|---|---|
| Thin-SDK prebuilt rule host | fixes time-to-first-rule (187 s to under 20 s) but not analysis cost; a cheap build of an engine that OOMs on the target repository does not create a demonstrable moat |
| Parallelism alone | wall-clock help, no memory help; the OOM is a working-set problem, and summaries are the only known way to make memory proportional to the working set (Infer's model, `research/static-analysis-2.0/03-summary-store.md`) |
| TS type tier | the largest accuracy lever, but accuracy on a run that cannot finish is invisible; it is Stage 1, right after the envelope |
| Demand-driven queries | depends on persisted summaries to be worth anything; it is the second customer of the keystone |

What it changes for buyers: `polint review` becomes proportional to the pull request, `polint check` becomes proportional to what changed since the last generation, dependencies are summarized once per version, and a run that hits the envelope says so per finding instead of dying. The honesty moat then reads "bounded and reported" rather than "sometimes fatal".

## 6. Erosion risks and defenses

| Risk | Likelihood | Defense |
|---|---|---|
| Incumbents ship agent integrations (MCP servers, assistants that write rules) | already happening | verifiability: compiled rules, fixtures, measured model deltas; publish head-to-head numbers (report 04) |
| Opengrep or Semgrep Community Edition become "good enough" for policy at L1 and L2 | medium | be certified L4 with honest evidence; L2 policy alone is not a moat |
| AI reviewers commoditize natural-language policy | medium | provenance-bearing findings the reviewer consumes; the agent becomes polint's user, not its competitor |
| Rust requirement deters authors | medium | thin-SDK build, templates, agent authoring; the compile step is the verifier and should be defended, not apologized for |
| Toolchain dependencies (Go, Node) undermine local-first | low | embedded sidecars, version-pinned, digested into cache keys; capability diagnostics instead of silent degradation |
| Small evidence base undermines credibility | high today | report 04's program; on-demand gates run and archived before any public claim |

## 7. Decision

Weakest moat: performance budget. Strengthening feature: summary-persisted, frontier-driven analysis under an enforced envelope (Phase 67 plus envelope), delivered in build-plan Stage 0 to Stage 2, with the thin-SDK rule host following in Stage 3. Measured proof: peak RSS on hugo and excalidraw proportional to the working set, warm review recomputing only the frontier, byte-identical warm and cold output, and a cold `polint check` under 20 s.

## Assumptions and open questions

- Per-committer pricing arithmetic assumes list price and active committers; enterprise discounts are unknown. **Assumption.**
- Claims about AI-driven pull-request volume growth are widely reported but not verified here. **Unverified.**
- Whether buyers weight "local-first" above "breadth" is inferred from the licensing backlash, not from a survey. **Assumption.**
- The thin-SDK build's budgets (cold under 20 s, zero Cargo spawns on unchanged rules) are targets from `research/code-preserving-rule-build/FINAL-REPORT.md` section 9.3, not results.

## References

- `README.md`; `research/ROADMAP.md` (product thesis); `research/agent-rule-authoring/decisions/001-typed-rust-rules-not-dsl-first.md`; `docs/architecture-review/10-sota-landscape-and-bar.md` (three wedges)
- `research/evaluation-harness/baselines/build-cost.json`, `scale-corpus-run.json`; PR #102, #103, #104 descriptions in `git log`
- `research/static-analysis-2.0/03-summary-store.md`, `04-incrementality.md`; `.planning/REQUIREMENTS.md` (SUM, REV, PERF requirements)
- `research/code-preserving-rule-build/FINAL-REPORT.md`
- GitHub Code Security and Secret Protection pricing, https://github.blog/changelog/2025-04-01-github-secret-protection-and-github-code-security-for-github-enterprise/ and https://resources.github.com/evolving-github-advanced-security/
- Opengrep launch, https://socket.dev/blog/opengrep-forks-semgrep, https://thenewstack.io/opengrep-launches-as-free-fork-after-semgrep-license-shift/, https://www.kodemsecurity.com/resources/press-release-security-rivals-unite-to-launch-opengrep-following-semgrep-clampdown
- Distefano et al., "Scaling static analyses at Facebook", CACM 2019 (diff-time deployment and compositional summaries)
