<!-- GSD:project-start source:PROJECT.md -->
## Project

**polint**

polint is a high-performance Rust framework for writing repo-local static-analysis rules across multiple languages. Adapters today cover **Go** (tree-sitter) and **TypeScript / JavaScript** (Oxc); more languages can be added through the same adapter contract. polint gives rule authors reusable infrastructure for file discovery, parsing, typed facts, diagnostics, local rule execution, and CI output.

The product is for engineering teams using AI-assisted development who need executable project-specific policies instead of repeating local conventions in prompts. It is not a replacement for ESLint, Ruff, Biome, golangci-lint, or formatters; it is a framework for checks that those generic tools cannot know.

**Core Value:** Make it easy to express a repo-specific engineering policy as a small rule and run it locally, in CI, and with AI coding agents.

### Constraints

- **Stack**: Rust workspace with Rust 2024 edition - required by the prompt and fits performance/static-analysis needs.
- **Language support**: multi-language framework. Today: Go (tree-sitter) and TypeScript/JavaScript (Oxc). New languages added through the adapter contract.
- **Parser choices**: tree-sitter-go for Go and Oxc for TS/JS - requested baseline and current crate ecosystem fit.
- **Performance**: Use deterministic parallelism and avoid cloning large source strings - large repo support is a core requirement.
- **Reliability**: Parser errors and rule panics should become diagnostics or controlled internal errors, not crashes.
- **Truthfulness**: Heuristic rules must say they are heuristic and must not claim exact coverage.
- **Repository layout**: Product code and GSD planning documents live together in the repository root on `main`.
<!-- GSD:project-end -->

<!-- GSD:stack-start source:research/STACK.md -->
## Technology Stack

## Recommended Stack
| Area | Choice | Version Checked | Rationale | Confidence |
|------|--------|-----------------|-----------|------------|
| CLI | `clap` with derive | 4.6.1 | Mature Rust CLI parser with strong help text support. | High |
| Serialization | `serde`, `serde_json`, `toml` | serde 1.0.228, serde_json 1.0.149, toml 1.1.2+spec-1.1.0 | Required for config, diagnostics, cache metadata, JSON output, and SARIF-like output. | High |
| Errors | `anyhow`, `thiserror` | anyhow 1.0.102, thiserror 2.0.18 | Application errors in CLI, typed errors in libraries. | High |
| Tracing | `tracing`, `tracing-subscriber` | tracing 0.1.44, subscriber 0.3.23 | Structured internal logging and future profiling integration. | High |
| Parallelism | `rayon` | 1.12.0 | Straightforward parallel parsing and rule execution. | High |
| File discovery | `ignore`, `globset` | ignore 0.4.25, globset 0.4.18 | Fast walking with `.gitignore` support and reliable glob matching. | High |
| Internal relations | `petgraph` | 0.8.3 | Internal representation for relationship facts behind the SDK; no public CLI/export contract. | High |
| Go parsing | `tree-sitter`, `tree-sitter-go` | tree-sitter 0.26.8, tree-sitter-go 0.25.0 | Practical syntax extraction without needing Go type checking in v1. | High |
| TS/JS parsing | Oxc crates | 0.129.0 | Rust-native high-performance JS/TS parser ecosystem. | Medium |
| Import resolution | `oxc_resolver` | 11.19.1 | Useful for future TS import-resolution precision. Initial v1 can start with syntactic imports. | Medium |
| Tests | `insta`, `assert_cmd`, `predicates`, `tempfile`, `pretty_assertions`, `proptest` | Current versions checked | Covers snapshots, CLI integration, fixtures, diffs, and invariants. | High |
## What Not To Use First
- Salsa as a hard dependency for v1 query infrastructure. Keep a cache abstraction and ship the hash-based cache first.
- Full Go semantic analysis in the first pass. Avoid depending on a sidecar until syntax/fact extraction is stable.
- A custom JS/TS parser. Oxc is the right default.
- Leaking full AST dumps through public rule APIs. Prefer stable IDs and incremental facts on the host.
## Version Notes
<!-- GSD:stack-end -->

<!-- GSD:conventions-start source:CONVENTIONS.md -->
## Conventions

### Public API and visibility

Everything **public** is a **liability**: semver, documentation, stability expectations, and review surface all attach to names users and tools can import. Default to the **narrowest** visibility that still works (`private` → `pub(super)` / `pub(in path)` → `pub(crate)` → `pub` only when crossing an intentional boundary).

- **Supported rule-author surface:** `polint::sdk` (including `prelude` and `scope`) and `polint::runner`, plus what those modules deliberately document and re-export. Treat `core`, `cache`, `config`, `fs`, `go`, `ts`, `graph`, `cli`, and other crate-root modules as **implementation detail** unless a change explicitly promotes something to the SDK.
- **Public CLI discipline:** treat every visible CLI command, flag, output format, generated skill instruction, README workflow, and example command as a public product contract. Expose only features that are complete, genuinely valuable to users today, documented honestly, and intended to be supportable. Keep internal diagnostics, debug helpers, partial graph/export surfaces, placeholders, and "soon" / TBD functionality private or hidden until they are finished and promoted intentionally.
- **Inside `crates/polint`:** use **`pub(crate)`** for anything shared across internal modules but not meant for downstream crates. Use bare **`pub`** only when a name must be visible outside its defining module *and* that visibility is intentional (e.g. items in `sdk` / `runner`, or the unstable **`polint::_bench`** tree behind **`feature = "bench"`** for `polint-bench` only).
- **`pub use` re-exports:** treat each one as widening the API; prefer small, curated surfaces over large barrel re-exports.
- **Linting:** the workspace enables **`unreachable_pub`**. If it fires, **fix visibility** (usually `pub` → `pub(crate)` or tightening module `pub`) rather than weakening the lint, unless there is a documented false positive.

Bench-only and internal hooks should stay namespaced (**`_bench`**, **`#[doc(hidden)]`**) and must not be presented as a supported extension API for rule packs.
<!-- GSD:conventions-end -->

<!-- GSD:architecture-start source:ARCHITECTURE.md -->
## Architecture

The landed two-package graph, private module layering, composition root, provider/fact/frontend contracts,
identity model, lifecycle adapters, determinism rules, and extension guidance
are documented in [`ARCHITECTURE.md`](ARCHITECTURE.md). Keep public visibility
narrow and follow [`docs/API-VISIBILITY-PLAN.md`](docs/API-VISIBILITY-PLAN.md)
when tightening `pub` / `pub(crate)` boundaries; the supported rule-author
surface remains `polint::sdk` and `polint::runner`.
<!-- GSD:architecture-end -->

<!-- GSD:skills-start source:skills/ -->
## Project Skills

No project skills found. Add skills to any of: `.claude/skills/`, `.agents/skills/`, `.cursor/skills/`, or `.github/skills/` with a `SKILL.md` index file.
<!-- GSD:skills-end -->

<!-- GSD:workflow-start source:GSD defaults -->
## GSD Workflow Enforcement

Before using Edit, Write, or other file-changing tools, start work through a GSD command so planning artifacts and execution context stay in sync.

Use these entry points:
- `/gsd-quick` for small fixes, doc updates, and ad-hoc tasks
- `/gsd-debug` for investigation and bug fixing
- `/gsd-execute-phase` for planned phase work

Do not make direct repo edits outside a GSD workflow unless the user explicitly asks to bypass it.
<!-- GSD:workflow-end -->

## Engine Direction: Package Graphs

Read [`research/package-graphs/README.md`](research/package-graphs/README.md)
before changing call-graph, points-to, summary, model or benchmark code. It is
the source of truth for this section; the current iteration is
[`research/package-graphs/ITERATION-1-CLASSICAL.md`](research/package-graphs/ITERATION-1-CLASSICAL.md).

What the engine builds: one **package graph** per package version (the
application's own code is a package too): its call graph plus an explicit
boundary of exports, imports and the values that cross. A package graph is
built once, stored, and **stitched** with others into the graph of the whole
program. Accuracy is paid for ahead of time, cheapest method first:

0. Extraction: every function and call site is in the graph.
1. Classical analysis: symbols, types, points-to. Deterministic.
2. Recorded execution: edges seen while the package's tests run.
3. Confidence scoring: orders what gets reviewed. It never removes an edge.
4. Agent review: corrections with evidence.
5. Package models and engine changes: a pattern that keeps needing
   corrections is modelled, so its edges are derived.

How the parts of this repository fit:

| Part | Role |
|---|---|
| Language frontends, MIR, symbol graph (`ts/`, `go/`) | Stage 0, and the constraints for stage 1 |
| Constraints and solver (`analysis_neutral/semantic_graph`, `points_to`, `solver`) | Stage 1 |
| Type sidecar (`ts/types`) | An input to stage 1, not a separate answer |
| Summaries and the store (`analysis_neutral/summaries`, `analysis_kernel/store`) | Where package graphs are kept and stitched |
| Model edges and extensions (`ConstraintKind::ModelEdge`, `analysis/extensions`) | Stage 5 |
| Call recorder and bench (`tests/js-analysis-bench`) | Ground truth, and the source of stage 2 evidence |

Rules for every change:

- **Evidence on everything.** An edge states its stage, algorithm and tier. A
  call site without a complete answer says so and says why.
- **Two tiers, never mixed.** *Exact*: the call reaches this function, and no
  other where the site is marked complete. *Conservative*: the call may reach
  it. A wrong exact claim is the most serious defect the engine can have.
- **Engine output is reproducible** from the package content, the engine
  version and the model set. Recorded runs, models' corrections and agent
  corrections live in a separate layer keyed by stable identity; analysis
  never reads it.
- **Graphs compose.** Stitching the graphs of A and B must give the edges of
  analysing A and B together. Keep the composition test green.
- **Cheapest first.** When a later stage keeps fixing the same thing, improve
  the earlier stage, and keep the fixed cases as fixtures.
- **Do not tune on the held-out set**, and do not add a recogniser for one
  benchmark package where a general construct is missing.
- **A model for one library, the engine for two.** A pattern of a single
  library is a package model. Move it into the engine when a second library
  needs it.

polint is the engine and the formats. A service that scans packages at scale,
stores their graphs or runs review agents is a consumer of polint and is not
built in this repository.

## Rust Skill Usage

Before writing, reviewing, or refactoring Rust code, consult and apply
`.agents/skills/rust-best-practices/SKILL.md`. When the Rust work touches polint
rule authoring, SDK contracts, generated agent skills, examples, or rule-pack
ergonomics, also consult `.claude/skills/polint/SKILL.md`.

## Shipped-Code Comment Policy

Keep delivery history in `.planning/`, commit messages, and pull requests. Do not
put GSD phase numbers, milestone numbers, plan/task identifiers, roadmap chronology,
or similar project-management references in source comments, doc comments, tests,
fixtures, lint-suppression reasons, build files, CI names/comments, generated text,
or product-facing output. Comments must explain enduring behavior, constraints, and
invariants in domain terms. Describe algorithm sequencing as steps or stages, never
as delivery phases.

## Git Branch Policy

Do not push directly to `main` unless the user explicitly instructs you to push
to `main` in that turn. For any code, docs, workflow, release, or planning
change that should be shared remotely, create a named feature/fix branch first
and push that branch for review.

If work starts on `main`, create the branch before pushing. If `main` is pushed
by mistake, stop and ask the user before attempting any remote repair, revert,
or force-push.

## Rule Authoring Platform Contract

Repo-local rules must be treated as external consumers of polint, even when they
live in `examples/` inside this repository.

- Rule code should import `polint::sdk::prelude::*` and register through
  `polint::runner::run_cli`; do not make examples depend on `polint::core`,
  `go`, `ts`, `config`, parser adapters, test helpers, or other internal modules.
- Examples should demonstrate composition of public typed fact views requested
  from `#[polint::rule]` signatures. `RuleCtx` is for diagnostics, options,
  source paths, and capability/setup metadata; do not reintroduce broad fact
  access on `RuleCtx` as the normal rule-authoring path.
- Future analysis families such as CFG, call graph, dataflow, coverage, module
  graph, symbols, references, and test metrics should be added as typed SDK
  views with query methods on those views, not as `RuleCtx` fact accessors.
- Capabilities for normal rules must be derived from typed fact-view parameters,
  not handwritten `Capabilities::new()` declarations. Do not add manual `impl
  Rule` examples, compatibility shims, or public rule constructors as a beta
  escape hatch; if the old API shape fights the product model, break it and move
  examples/tests to the typed macro path.
- Higher-level policy rules should compose reusable typed signal views such as
  `FileMetrics<'_>`, `FunctionMetrics<'_>`, and `ComplexityMetrics<'_>` instead
  of calling other rules or treating diagnostics as inputs.
- `#[polint::rule]` functions should stay plain and analyzable: no generics,
  no async/const/unsafe/extern forms, `&mut RuleCtx<'_>` first, `RuleResult`
  or `RuleResult<()>` return.
- Macro-derived fact parameters must resolve to SDK fact views exported by the
  prelude or written as canonical `polint::sdk::facts::*` paths, with the
  placeholder lifetime form such as `Imports<'_>`. Do not make locally defined
  lookalike types, aliases, or user-provided `FactView` implementations count
  as capability sources.
- When adding a rule-authoring feature, add at least one temp-repo style test
  that behaves like an outside user: generated `.polint/rules`, public SDK
  imports only, real facts consumed, and a diagnostic asserted through
  `polint check --format json`.
- Keep capability names honest. Do not expose or advertise a capability as a
  provided fact family until a rule can read the underlying facts through the
  public SDK. Rules that request unsupported or setup-missing hard capabilities
  should produce capability diagnostics and not execute with placeholder facts.
- If a rule needs custom config, preserve it through `RuleOptions::settings`
  rather than overloading unrelated fields like `allow`, `deny`, or `max`.
- Comment ignores are an engine/reporting concern. Rules should always report
  the real diagnostics they find; do not add per-rule ignore-comment parsing or
  suppression logic. Keep `polint ignores`, `docs/IGNORE-COMMENTS.md`, and the
  generated skill text aligned when ignore behavior changes.
- Config and resolved rule options that can affect rule behavior must
  participate in deterministic cache digests, with regression tests for new
  fields.
- Document new public facts under `docs/facts/`, including limits and heuristic
  behavior.

## Go Analysis Lifecycle Contract

Go semantic analysis must support monorepos from day one without requiring a
repository-root `go.mod` or extra checked-in setup files. Keep the user-facing
configuration in the single `.polint.toml` file.

- Go analyzers should infer module roots by walking from discovered Go files to
  the nearest `go.mod`, and should also honor explicit `[languages.go]`
  `module_roots = [...]` when users want deterministic lifecycle control.
- If Go module roots live below the repository root, or multiple roots are
  analyzed, use a checked-in root `go.work` only when it covers every selected
  module root. Otherwise, use an internal temporary workspace for package loading
  rather than writing generated lifecycle files into the repository.
- `package_patterns`, `build_tags`, and `include_tests` are analysis lifecycle
  inputs and must remain in `[languages.go]`; they must participate in cache or
  deterministic analysis digests when the affected analyzer is cached.
- Future Go analysis families such as module graph, call graph, CFG, dataflow,
  coverage, symbols, and references should share the same root-selection model.
  Do not introduce one-off root flags, hidden side files, or per-analyzer config
  files for these lifecycle concerns.
- Setup gaps should be explicit `polint/capability` diagnostics. Do not run rules
  with placeholder semantic facts when the configured Go lifecycle cannot load.



<!-- GSD:profile-start -->
## Developer Profile

> Profile not yet configured. Run `/gsd-profile-user` to generate your developer profile.
> This section is managed by `generate-claude-profile` -- do not edit manually.
<!-- GSD:profile-end -->
