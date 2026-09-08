# Strategy Research: What polint Must Build Next to Become the Most Capable Static-Analysis Engine

Date: 2026-09-01
Researcher: Claude Fable 5.1 (delegated via Hermes)
Revised 2026-09-02: time estimates removed, capability roadmap added.
Scope: depth of analysis, correctness, performance, developer experience and defensibility for Go and TypeScript/JavaScript. More language support is explicitly out of scope, by the founder's instruction.

## TL;DR

- "Most capable" is defined here as a position on a capability ladder (eight conclusion levels, L0 lexical to L7 verification-grade) crossed with seven measured axes (honesty, scale, latency, framework modeling, evidence, authoring, measurement). polint today is L3 complete, L4 partial, L5 seeded, with best-in-class authoring, honesty and evidence and weak scale, latency, framework modeling and measurement.
- The plan is six dependency-ordered stages: instrument (Stage 0), certify L4 (Stage 1), persist summaries and make warm review real (Stage 2), fix the scale envelope and ship the graph surface (Stage 3), buy L5 selectively (Stage 4), buy L6 selectively and publish head-to-head results (Stage 5).
- The five-item L4 certification set is the highest-value work in the repository: a TypeScript type tier, IFDS tabulation, access-path summaries, framework models as data, and a taint benchmark with a CI gate.
- The weakest moat is the performance budget; the feature that strengthens it most is summary-persisted, frontier-driven analysis under an enforced runtime envelope (v2.0 Phase 67 plus an envelope), with the thin-SDK rule build following.
- Every claim about polint in these reports was checked against the code at v0.3.3 (`9b6ac59d`); the July 2026 architecture review is cited with a table of what the August refactor changed.
- External claims are cited to primary sources where verified in this session (TypeScript 7 API timing, the Opengrep fork, GitHub Code Security pricing) and marked as assumptions or unverified elsewhere. No benchmark number is invented.
- Report 06 restates the plan as plain-English capabilities gained per stage, for readers who are not analysis specialists.

## Reports

| Report | Question answered | One paragraph |
|---|---|---|
| [01-capability-ladder.md](01-capability-ladder.md) | What does "world's most capable" mean, measured, and where is polint on it? | Defines levels L0 to L7 by the machinery each adds and the bug classes it newly concludes, grounded in the dataflow, points-to, path-sensitivity and verification literature. Adds seven orthogonal axes graded 0 to 3. Places about fourteen engines, including CodeQL, Semgrep Pro and Community Edition, Opengrep, ast-grep, Infer, Doop and WALA, Joern, Jelly, SonarQube, commercial SAST, Clang Static Analyzer, the Rust verifiers, and AI reviewers. Places polint from the code: L3 solid, L4 partial, L5 seeds, L6 and L7 absent; axes A 2, B 1, C 1, D 1, E 2, F 3, G 1. Names the four instruments that make any placement falsifiable and sets a placement target after each stage of the build plan. |
| [02-gap-analysis.md](02-gap-analysis.md) | For each rung, what does the strongest engine do that polint does not, with evidence, and what can polint do that they cannot? | Opens with a table of July-review findings versus the September state so the gaps are current. Tabulates gaps per rung: L2 near parity, L3 thin on domains, L4 five concrete items, L5 selective sensitivity and dependency summaries, L6 blocked on branch predicates. Tabulates axis gaps and the hygiene polint must match. Documents the moat: compiler-verified typed rules, repo-local policy, per-finding honesty with replayable evidence, local-first execution against a licensing backdrop that moved in polint's favor, and an agent loop. Ends with a twelve-item prioritized gap list. |
| [03-build-plan.md](03-build-plan.md) | What to build, in what order, with what risks and exit criteria? | Reconciles the three existing plans: the 22-PR research backlog (mapped one-to-one to shipped v1.2 phases with a residue per PR), the v2.0 phases 63 to 71, and the architecture-review milestones. Sequences six dependency-ordered stages, Stage 0 to Stage 5, with items, tracks, sources, and exit criteria tied to probe pass rates, oracle lanes and cost curves. Names the hardest risks (fixpoint termination under tabulation, summary precision collapse, cache invalidation correctness, cross-language fact identity, the TS sidecar dependency, scope collapse, benchmark overfitting, memory regressions) with mitigations and kill criteria. Lists what is explicitly not built and the decisions the founder must make. |
| [04-evaluation-as-a-weapon.md](04-evaluation-as-a-weapon.md) | How does polint become the most rigorously measured engine and publish numbers competitors cannot refute? | Inventories the existing harness honestly (strong machinery, no enforcement: the F1 gate never runs in CI). Specifies four instruments: capability probes per level with must-not-report twins, two real-application oracle lanes never blended, cost curves, and mutation-based soundness checks. Names ground-truth corpora for Go and TS/JS, sets false-positive and false-negative budgets per rule family, defines a differential-testing protocol against CodeQL, Semgrep and Opengrep with adjudication, a publication checklist, and CI tiers. |
| [05-moat-economics.md](05-moat-economics.md) | Why would a team choose polint at parity, which moat is weakest, and what single feature strengthens it most? | Scores five moats with evidence for and against. Explains the choice at parity for three buyers. Sets out the 2025 pricing and licensing environment (GitHub Code Security at 30 dollars per committer per month, Semgrep's relicensing and the Opengrep fork). Identifies the performance budget as the weakest moat, with the measurements that show it, and argues for summary-persisted, frontier-driven analysis under an envelope as the single strengthening feature, with the thin-SDK rule build as runner-up. Lists erosion risks and defenses. |
| [06-capability-roadmap.md](06-capability-roadmap.md) | In plain English, what do we gain at each stage of the build plan? | The entry point for non-specialists: the whole journey in one paragraph, a summary table of what each stage gains and what it unlocks next, and per-stage "you will be able to" bullets that each cite the build-plan item or gap-analysis row delivering them. The five-item L4 certification set is called out as its own section. Technical terms are glossed once on first use. No time estimates. |

## How to read this

Non-specialists should start with [06-capability-roadmap.md](06-capability-roadmap.md), the plain-English capability roadmap; every capability there links back to the evidence in reports 02 and 03.

1. Start with report 01 section 5 (where polint stands) and report 02 section 2 (what changed since July). Those two sections are the factual base; everything else is argument on top of them.
2. If you are planning the next milestone, read report 03 sections 2 and 4, then the risk table in section 5. Section 2 reconciles the PR backlog, the v2.0 phases and the architecture-review milestones so nothing is scheduled twice.
3. If you are deciding what to measure first, read report 04 sections 3 and 8. The Stage 0 items in report 03 are the first three actions there.
4. If you are deciding positioning or pricing, read report 05 sections 2 to 5.
5. Treat the placements of closed commercial engines as assumptions until the differential runs in report 04 replace them with measurements.

## Conventions

- Levels are written L0 to L7; axes A to G with grades 0 to 3 as defined in report 01.
- Internal citations are repository paths, with line numbers where a specific claim is made, checked at commit `9b6ac59d`.
- External citations are inline URLs. Claims that could not be verified in this session are marked **unverified** or **assumption**.
- Numbers are only quoted when they come from a committed artifact, a cited primary source, or a repository research document that itself cites one; where a repository document is the source, the report says so.

## Relationship to existing research

- `research/static-analysis-2.0/` supplies the locked technical direction (tiered resolution, compositional summaries, local store, verified ML at the edges); these reports adopt it and sequence it.
- `docs/architecture-review/` supplied the July 2026 critique; report 02 records which findings the August refactor resolved.
- `.planning/ROADMAP.md` and `.planning/REQUIREMENTS.md` define v2.0; report 03 keeps the phases and moves them into the six-stage plan.
- `research/code-preserving-rule-build/` and `research/evaluation-harness/` are consumed by reports 05 and 04 respectively.

## Known limitations of this research pass

- The web-research fan-out planned for this pass failed under a session rate limit; the reports rely on primary sources already indexed in the repository, three verified web checks, and the researcher's knowledge of the literature. Competitor numbers not present in the repository's own research are marked as assumptions rather than reported.
- No engine was run in this session; all polint measurements are quoted from committed artifacts and pull-request descriptions.
