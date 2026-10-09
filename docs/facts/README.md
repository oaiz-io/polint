# Rule Fact Reference

These pages describe the fact families exposed through typed views exported by
`polint::sdk::prelude::*`, plus the `RuleCtx` helpers rules use to report
diagnostics.

Rule authors should treat these facts as the supported building blocks for
repo-local policies. The examples are only consumers of these facts, not special
internal entry points.

Promotion is explicit. Stable public rule-author views and preview policy,
call-graph and Go type views are listed below. Raw CFG, semantic graph,
solver/provider internals, evidence stores, effects/summaries, type/value/alias
facts, benchmarks, and eval reports stay private or reserved until they have
public SDK views, docs, temp-repo tests, bounded behavior, setup behavior,
cache/input proof, and no-leak proof. See
[`docs/API-VISIBILITY-PLAN.md`](../API-VISIBILITY-PLAN.md#phase-55-preview-promotion-audit)
for the current disposition table.

- [Functions](functions.md)
- [Metrics](metrics.md)
- [Imports](imports.md)
- [Resolved imports and module graph](resolved-imports.md)
- [Symbols and references](symbols-and-references.md)
- [Branches](branches.md)
- [Go tests](go-tests.md)
- [Go structural types](go-types.md)
- [Capability support](capability-plans.md)
- [TypeScript / JavaScript](ts-js.md)
- [Literals and JSX attributes](literals.md)
- [Changed files (review rules)](changed-files.md)

Preview policy-query references:

- [Policy query preview](policy-queries.md)
- [Events](events.md)
- [Calls](calls.md)
- [Control-flow policies](control-flow.md)
- [Data-flow policies](data-flow.md)

Preview graph and type references:

- [Call graph](call-graph.md)
- [Go types](go-semantic-types.md)
- [Routes](routes.md)

Reserved/internal references:

- [Reserved capability plans](capability-plans.md)
- [Diagnostic evidence limits](evidence.md)
