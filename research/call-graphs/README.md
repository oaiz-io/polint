# Call Graph Research

This folder is a research package for adding call graph facts to polint without using GSD.

Date: 2026-05-15

Revision: 2026-05-16 bootstrap integration update after
`research/implementation-bootstrap/`.

## Main Finding

There is no single state-of-the-art call graph algorithm that works well across Go, JavaScript/TypeScript, Java, Python, and other languages. The practical state of the art is a layered architecture:

1. Emit direct syntactic call sites for every language.
2. Bind names/imports/symbols when the adapter can do so cheaply.
3. Add language-specific dispatch algorithms such as CHA, RTA, VTA, points-to analysis, type tracking, MRO lookup, or JavaScript value-flow.
4. Preserve unresolved and uncertain calls as first-class facts instead of pretending they do not exist.
5. Make precision explicit in every fact and cache key.

For polint, the right next step is not a monolithic "build a call graph"
feature. After the bootstrap research, it is an internal `analysis::calls` fact
family that consumes MIR, places, symbols, direct summaries, and extension sinks.
Public `Calls<'_>` / `CallGraph<'_>` views come later, after validation gates.

The product-specific refinement is that polint should support repo-local call graph models authored by AI agents or rule authors. These models should bind back to native facts, carry provenance, and reduce unresolved calls for the specific codebase instead of forcing the native engine to auto-discover every framework convention.

## Contents

- [FINAL-REPORT.md](FINAL-REPORT.md): final synthesis and recommended implementation path.
- [AGENT-EXTENSIBLE-CALL-GRAPHS.md](AGENT-EXTENSIBLE-CALL-GRAPHS.md): how repo-local call models change the product architecture.
- [RESEARCH-ANALYSIS.md](RESEARCH-ANALYSIS.md): deeper accuracy, complexity, benchmark, and tradeoff analysis.
- [RECOMMENDED_IMPLEMENTATION.md](RECOMMENDED_IMPLEMENTATION.md): concrete recommendation for a native Rust implementation.
- [REPO-INDEX.md](REPO-INDEX.md): OSS repositories cloned and inspected.
- [PAPER-INDEX.md](PAPER-INDEX.md): papers and docs downloaded locally.
- [VALIDATION.md](VALIDATION.md): validation pass for references, source paths, and core claims.
- [JS-RECALL-BUCKETS.md](JS-RECALL-BUCKETS.md): JS/TS bench recall by package and plan, missed edges by cause, and the plan for the remaining buckets.
- [STANDARD.md](STANDARD.md): standardized vocabulary and comparison template.
- [algorithms/core-algorithms.md](algorithms/core-algorithms.md): Python-ish pseudocode for the main algorithms.
- [languages/go.md](languages/go.md): Go-specific findings.
- [languages/typescript-javascript.md](languages/typescript-javascript.md): JS/TS-specific findings.
- [languages/java.md](languages/java.md): Java/JVM-specific findings.
- [languages/python.md](languages/python.md): Python-specific findings.
- [implementation/polint-call-graph-path.md](implementation/polint-call-graph-path.md): concrete polint implementation path.
- [implementation/BOOTSTRAP-INTEGRATION.md](implementation/BOOTSTRAP-INTEGRATION.md): revised implementation path aligned with the semantic bootstrap, `SemanticStore`, `PlaceId`, direct summaries, cache keys, and extension sinks.

## Local Clone Policy

All third-party repositories are under `research/call-graphs/repos/`, which is gitignored. The research notes reference exact local paths and commit hashes, but the cloned source is not intended to be committed.
