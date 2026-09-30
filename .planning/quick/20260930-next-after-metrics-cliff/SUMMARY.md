---
status: complete
---

This task took no new measurements. It re-derived the post-fix state from the metrics-cliff raw runs
at `/opt/data/polint-perf5/metrics-cliff/runs/`:
- FINAL-arm stage breakdowns for the full repo, core, frontend and code-health;
- the rules-phase schedule of every clean full-repo sample (n = 107, all arms);
- the provider-span ceiling of the cold-path metrics miss, from the no-metrics-rule arm.

It wrote `thoughts/perf-5.0-levers/11-next-after-metrics-cliff.md` and added a post-metrics-cliff
update to the README. The new finding is a full-repo rules-dispatch scheduling tax: a median of
0.77 s of the rules phase goes to rayon's split order. The pick is cost-ordered rule dispatch
(in-contract, 2–3 days, with a spike plan). L1's E1 spike comes next. The cold-path metrics miss
waits on an owner answer.

Content commit a7714a19, pushed to `research/perf-levers`. PR #130's body gained a "Next-lever
research (2026-09-30)" section, appended over REST with the original text preserved. A
case-insensitive grep for the forbidden consumer name returned zero hits across the diff and the PR
body.
