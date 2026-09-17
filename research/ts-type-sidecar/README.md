# TS type sidecar

Question: what is the smallest architecture that gives polint a type-directed
call-graph tier for TypeScript and JavaScript, and how much recall does it buy
over the Andersen points-to tier that answers those call sites today?

Why it matters: [`research/strategy/02-gap-analysis.md`](../strategy/02-gap-analysis.md)
ranks "TS type sidecar" as gap item 2 of twelve and the largest real-world
recall lever for TS/JS, and
[`03-build-plan.md`](../strategy/03-build-plan.md) schedules it as the first
Stage 1 item. Go already consumes compiler types through a sidecar
(`go/packages` + `go/ssa`); TypeScript reaches the refined-call provider only
through low-confidence points-to rows.

Status: implemented. This folder holds the design that was built, the wire
protocol as shipped, and the measurement record.

- [plan.md](plan.md) — architecture, module layout, plumbing, acquisition
  policy, risks, and the decisions taken.
- [wire-protocol.md](wire-protocol.md) — `polint-ts-types-1` NDJSON contract.
- [measurement.md](measurement.md) — what was measured, how, and the numbers.

Scope: resolution only. This work adds a typed tier to the call graph. It does
not change IFDS, summaries, or models-as-data, which are the other Stage 1
items.

Non-goals, recorded so they are not re-litigated:

- No TypeScript 7 / `typescript-go` dependency. TypeScript 7 exposes no stable
  programmatic API before 7.1, which is Microsoft's timing and not polint's
  (`research/static-analysis-2.0/OPEN-QUESTIONS.md` Q20).
- No bundled TypeScript. The compiler comes from the analyzed repository, an
  explicit environment override, or a global install; absent all three, the
  tier is skipped with a capability diagnostic.
- No replacement of the Andersen tier. The typed tier ranks above it and the
  heap tier stays as the fallback for everything types cannot answer.
