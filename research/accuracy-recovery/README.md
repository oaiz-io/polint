# Jelly accuracy recovery research

**Question:** which missing JS/TS models explain the recall collapse, and what can tonight's implementation crew realistically recover without changing the accuracy contract?

**Recommendation:** continue the archived private collector port, correcting its integration with the canonical graph/shared solver. Do not start another 8,519-line restoration: 8,261 lines already match the deleted production collector exactly. Budget **10–16 engineer-hours / 8–12 elapsed hours with two experienced implementers**, plus **2–4 elapsed hours contingency**; passing tonight is plausible, not established.

Research only, 2026-09-08, branch `accuracy/astra-f1`, starting/ending product revision `d713dbd2e1c16ee5d89c3341db29fc585ee94c1b`. No product, baseline, tolerance, Go, or gate edits; no PR. No new collector implementation or gate execution. Saved measurements were independently reconciled against all 76 cases and their edge sets.

- [Final report and FN taxonomy](FINAL-REPORT.md)
- [Implementation sequence, files, and fixtures](RECOMMENDED_IMPLEMENTATION.md)
- [Gate procedure, evidence inventory, and limits](VALIDATION.md)
- [Collector comparison and architecture evidence](INSIGHTS-collector-architecture.md)
- [Per-case and edge analysis](INSIGHTS-fn-taxonomy.md)
- [Independent skeptical review](INSIGHTS-review.md)

Raw reports, checksums, full edge ledger, source snippets, and reproducible research scripts are under `.context/accuracy-research/`. The original reports were found in the sibling `/workspace/polint-perf-research/` worktree and copied locally without changing them. The durable reports record exact provenance so conclusions remain interpretable if ignored artifacts are unavailable.
