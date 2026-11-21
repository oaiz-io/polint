# Static Analysis 2.0

Research track for the next generation of polint's analysis engine. Started
2026-07-07 from a full-repo critical review plus a 2020–2026 literature survey
(algorithmic + ML). The review that seeded this track is
[00-critical-review.md](00-critical-review.md).

## Product vision

Static Analysis 2.0 does not replace polint's core offer; it makes that offer
work on larger, more complex codebases.

The two core products stay:

1. **Custom rules as Rust code** — `polint check` remains the repo-local policy
   engine: teams write small typed Rust rules against stable SDK views instead
   of encoding local conventions in prompts.
2. **Agentic review** — `polint review` is the diff-time workflow for AI coding
   agents and PR review: focus analysis on changed code, surface evidence and
   uncertainty, and help agents make higher-quality changes.

The long-term promise is broader: **polint becomes the semantic layer that helps
AI agents understand, explore, and improve large codebases.** The engine should
let an agent move from a question to executable evidence: "what uses this?",
"what changes if this API moves?", "does this SQL query receive user-controlled
input?", "which architectural boundary does this flow cross?", "show the
neighbors of this symbol/call/flow", or "find similar sinks/sanitizers across
the repo." Custom rules and review diagnostics are the enforcement side of that
same graph; interactive graph exploration is the understanding side.

This implies a local-first product surface beyond rule execution:

- queryable program graph commands for used-by, neighbors, callers/callees,
  definitions/references, module boundaries, paths, and impact;
- normal structured filters over facts, summaries, precision tiers, provenance,
  unknowns, and budgets;
- future vector/embedding search over code, symbols, evidence bundles, and
  summaries, as an index over the same local semantic corpus;
- output shaped for agents: stable IDs, provenance chains, precision labels,
  compact evidence paths, and machine-readable query results.

The first implementation remains an offline, embedded local store. A remote
package-summary registry is **not** part of the initial build. The local
summary/store design must still preserve the seams that make a registry possible
later: content-addressed package summaries, package/version identity, schema
versions, provenance, validation metadata, and trust hooks. That keeps the
future option open without paying the product, security, and operations cost now.

## Goals

North star: **the best static-analysis framework in the world.** The
bring-your-own-rules linter and agentic review are the first products on top of
it, not the ceiling.

Concrete, falsifiable goals:

1. **Scale**: build CG/CFG/DF facts for multi-million-LOC applications
   (app code + full dependency tree) on a laptop/CI runner without OOM —
   peak memory proportional to the *working set*, not the repository.
2. **Latency**: warm/incremental runs proportional to the *change*
   (`polint review` on a PR re-derives only changed functions + their
   summary-dependents); cold runs bounded by per-package summarization that
   is computed once per (package, version) and shared.
3. **Accuracy**: real-application callgraph F1 (measured against dynamic-trace
   oracles on real repos, not micro fixtures) at or above the best published
   static tools per language, with **no silent degradation at scale** —
   budget/precision loss must be measured and surfaced, never invisible.
4. **Honesty**: every fact carries precision tier, provenance, and unknowns —
   already a polint principle; 2.0 extends it to summaries and ML-assisted
   facts.
5. **Language scalability**: adding language N+1 must not require rebuilding
   the analysis stack — one MIR/summary/solver substrate, thin typed
   frontends, official language tooling as fact providers.

Non-goals: soundness guarantees (we are optimizing F1 under honesty labels,
like every production analyzer); replacing type checkers/compilers; GPU or
distributed-cluster execution (out-of-core single-machine wins for this
workload).

## TL;DR implementation

The 2.0 architecture in one paragraph:

> **Tiered resolution + compositional summaries + a queryable local semantic
> store, with verified ML at the edges.** Call targets resolve through the cheapest
> sufficient tier: types first (Go types, TS types via a type sidecar —
> XTA-grade, near-linear), field-based/value-flow next, Andersen points-to
> heap only for the untyped residue. Every function/package boils down to a
> **summary** (callable surface, callback-invocation behavior, value flow,
> taint transfer) persisted through an embedded SQLite/rusqlite semantic store
> with content-addressed payloads and typed graph indexes: dependencies keyed
> by (package, version) and application code keyed by content hash at
> function/SCC granularity. The store is offline and embedded first; it keeps
> package-summary boundaries and trust metadata so a remote registry can be
> added later, but the registry itself is deferred. A Salsa-style red-green
> query layer sits on top for sub-second re-analysis. Data-flow/taint runs
> demand-driven from rule queries over the refined callgraph instead of
> eagerly. The same store later backs local `polint graph` queries, structured
> filters, Tantivy lexical search, and explicitly locked vector-search
> experiments. ML enters only where the literature shows it works: neural
> type/callable-shape inference for unresolved sites (accepted only after
> symbolic verification), a learned callee ranker (verify-then-accept),
> LLM-synthesized summaries for dynamic dependencies (validated against
> `.d.ts`), and LLM triage on review findings. Detection stays symbolic.

Memory drops because dependency bodies are never parsed after first
summarization and ASTs are dropped per-file after lowering. Latency drops
because warm runs are summary lookups plus a small recomputation frontier.
F1 rises because typed tiers resolve the majority of real-world (TS/Go) call
sites precisely, and the heap + ML handle the residue instead of everything.
Exploration improves because the same persisted facts and summaries can back
local CLI graph queries and agent-facing evidence retrieval.

## Rough plan

Ordered workstreams; each compounds the next. No time estimates by design.

| # | Workstream | Doc | Outcome |
|---|---|---|---|
| 0 | Ground truth first: real-app benchmarks, F1-vs-size + RSS-vs-size curves, budget-exhaustion telemetry | [01-benchmarking-and-measurement.md](01-benchmarking-and-measurement.md) | The problem becomes visible and gateable |
| 1 | Representation: drop-AST, arenas/interning, shared points-to sets, indirection bounding, fixed-cost profiling | [02-memory-and-representation.md](02-memory-and-representation.md) | Immediate memory/speed wins, zero semantic change |
| 2 | Local semantic store: SQLite/rusqlite facts, graph indexes, summary manifests, search boundary | [../local-semantic-store/](../local-semantic-store/) | Durable offline store for summaries, graph queries, filters, and future search |
| 3 | The keystone: compositional summary store (deps tier + app tier) | [03-summary-store.md](03-summary-store.md) | O(repo) -> O(working set); unlocks 4, 5, 7 |
| 4 | Incrementality: Salsa red-green over the summary layer | [04-incrementality.md](04-incrementality.md) | O(change) warm runs, sub-second review |
| 5 | Accuracy: type-directed CG tier (TS sidecar, Go types) + tiered fallbacks | [05-type-directed-callgraph.md](05-type-directed-callgraph.md) | Largest real-world F1 lever |
| 6 | Precision where it pays: selective context sensitivity, demand-driven queries | [06-selective-precision-and-demand.md](06-selective-precision-and-demand.md) | Precision without global cost |
| 7 | ML integration: verified type inference, callee ranking, LLM summaries, triage | [07-ml-integration.md](07-ml-integration.md) | Recall on the residue; long-tail summaries |
| 8 | Data-flow/taint on the refined callgraph, demand-driven | [08-dataflow-and-taint.md](08-dataflow-and-taint.md) | CG investment becomes rule-visible |
| — | Competitive landscape (standing reference) | [09-competitive-landscape.md](09-competitive-landscape.md) | Positioning + what to steal |
| — | Open questions to resolve before/while building | [OPEN-QUESTIONS.md](OPEN-QUESTIONS.md) | Decision log seed |

[`../package-graphs/README.md`](../package-graphs/README.md) (2026-11-17) extends
this track: package graphs per (package, version) become a primary output,
and graph construction gains stages after classical analysis. It lists which
of the decisions below it moves.

Standing decisions already taken (see 00-critical-review.md for rationale):

- Stop iterating recognizers against the Jelly micro suite (~89% F1 is the
  transfer plateau; further gains are oracle-modeling).
- Do not build: ML callgraph pruning, GNN/transformer replacement of
  symbolic dataflow, whole-repo LLM callgraph extraction, GPU/distributed
  solving, resident differential-dataflow incrementality as a first move.
- CFG stays intraprocedural per function; it is not a bottleneck.

## How this folder works

One doc per problem space: problem → current state in polint (with code
anchors) → what the research/industry says (with links) → direction for
polint → references. [OPEN-QUESTIONS.md](OPEN-QUESTIONS.md) enumerates the
decisions each doc leaves open (Q-IDs, with status); we resolve them one by
one and record outcomes there.
