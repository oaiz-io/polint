# Research

This folder contains static-analysis research for polint.

Start with [ROADMAP.md](ROADMAP.md).

Existing research:

- [abstract-interpretation/](abstract-interpretation/): reduced-product abstract-domain kernel, lattice/transfer interfaces, widening/narrowing, nilness/nullish, constants, strings, ranges, initializedness, shapes, typestate/resource, extension validation, and benchmark strategy.
- [cfg-control-flow/](cfg-control-flow/): native operation/basic-block CFG facts, exceptional and cleanup edges, dominance/postdominance, control dependence, path evidence, and extension overlays.
- [module-graph/](module-graph/): native workspace roots, package managers, lockfiles, dependency edges, import-to-package resolution, source sets, build targets, and repo topology.
- [type-alias-points-to/](type-alias-points-to/): native type, value, place, points-to, and alias fact layers; bounded Andersen-style solver; alias provider stack; and agent extension hooks.
- [effects-summaries/](effects-summaries/): function effects and summaries as the scaling boundary for call graphs, data flow, alias queries, framework models, and agent-authored analysis extensions.
- [semantic-index/](semantic-index/): native scopes, symbols, imports, references, aliases, resolution facts, xref indexes, and export identity.
- [call-graphs/](call-graphs/): native call-site/call-edge facts, algorithm tiers, unresolved facts, and agent-extensible repo-local call models.
- [package-graphs/](package-graphs/): the engine direction. One stored call graph per package version with an explicit boundary, stitched into the whole-program graph; the accuracy ladder from classical analysis to recorded execution, confidence scoring, agent review and package models; the current iteration with its targets; and the research behind it: the corrected diagnosis with twenty traced misses, Jelly and ACG measured on the bench, the held-out set, the closed lists of tiers, assumptions and constructs, storage option A against the solver, and the pull-request plan (GSD phases 72 to 79).
- [data-flow/](data-flow/): native data-flow substrate, summaries, source/sink/sanitizer models, call-graph dependency, and agent-extensible repo-local data-flow models.
- [agent-extension-surface/](agent-extension-surface/): Rust-code extension lifecycle, validation, provenance, capability planning, and default-vs-extended deltas.
- [analysis-kernel/](analysis-kernel/): fact layers, scheduling, provenance, precision, validation, extension merges, cache keys, and invalidation.
- [evaluation-harness/](evaluation-harness/): external-benchmark-first evaluation, ground truth, fixture schema, metrics, and regression gates.
- [framework-entrypoints/](framework-entrypoints/): framework/protocol boundaries, routes, jobs, callbacks, generated dispatch, MCP, and repo-local providers.
- [implementation-bootstrap/](implementation-bootstrap/): implementation-ready Rust bootstrap design for semantic MIR, place identity, direct call facts, P0 domains, direct summaries, minimal cache/invalidation, and extension sinks.
- [program-slicing-evidence/](program-slicing-evidence/): native slicing and evidence layer for diagnostics: PDG/SDG lessons, thin slices, chops, path ranking, JSON/SARIF evidence, provenance, unknowns, and extension merges.
- [incremental-query-engine/](incremental-query-engine/): native incremental query engine and cache design: input snapshots, layer/query/summary/diagnostic keys, shape digests, dependency indexes, invalidation planning, extension-aware quarantine, and future red-green/relation backends.
- [agent-rule-authoring/](agent-rule-authoring/): rule SDK, query ergonomics, model packs, provider extension boundaries, `polint test`, rule manifests, and AI-agent inspect/explain/diff authoring workflow.
- [local-semantic-store/](local-semantic-store/): SQLite/rusqlite-backed embedded semantic store, graph query indexes, registry-ready summary manifests, Tantivy lexical search path, vector-search boundary, and validation plan for local graph/agent exploration.
- [code-preserving-rule-build/](code-preserving-rule-build/): build, distribution, and execution architecture that removes the per-repository engine compile while keeping rules as real, typed Rust: thin SDK, prebuilt engine host, fact-snapshot protocol, artifact fingerprinting, trust modes, and a measured build-cost baseline. Proposed as an intentionally breaking 0.3.0 rule-pack manifest/build migration with no legacy backend; rule `.rs` source/API stays stable.
