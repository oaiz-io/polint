---
status: complete
---

Measured on a copy of the OAIZ span-fix tree, with raw evidence at `/opt/data/polint-perf5/post-l4/`:
- core, frontend and full-repo cold, warm and a first edit tier, on the pinned 0.4.1 engine and on
  0.4.2, host-direct and through the driver, three clean interleaved rounds each, with identical
  normalized output;
- a fresh 0.4.2 rule-host build with cargo timings, `cargo install polint 0.4.2`, the thin-SDK
  dependency floor and a rule-host store depth check;
- a one-rule A/B that measured the metrics-trigger cliff.

Wrote `thoughts/perf-5.0-levers/10-next-lever.md` and amended the README; its original text is
unchanged. The pick is the metrics-trigger cliff, with L1 (via E1/E3) as the next strategic lever.
Content commit 2893a9fd, pushed to `research/perf-levers`. PR #130's body gained a "Post-L4 update
(2026-09-29)" section, appended over REST with the original text preserved. A case-insensitive grep
for the forbidden consumer name returned zero hits across the diff and the PR body.
