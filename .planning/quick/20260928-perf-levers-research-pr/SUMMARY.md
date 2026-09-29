---
status: complete
---

Authored `thoughts/perf-5.0-levers/` (README + 8 lever/next-target/methodology files) from the
perf-5 campaign's report of record and supporting artifacts at `/opt/data/polint-perf5/`. Every
number traced to source and marked `(unmeasured)` where projected. Verified a repo-wide
case-insensitive grep for the forbidden consumer name returned zero hits across the diff. Committed
per file (10 commits), pushed `research/perf-levers`, and opened non-draft PR
https://github.com/oaiz-io/polint/pull/130 against `main`. Confirmed via
`gh pr view --json url,isDraft,state`: `isDraft: false`, `state: OPEN`.
