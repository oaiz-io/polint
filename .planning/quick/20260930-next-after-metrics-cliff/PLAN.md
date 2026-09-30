---
task: next-after-metrics-cliff
---

After the metrics-trigger cliff fix merged (PR #131, released as v0.4.3), synthesize every piece of
perf research so far and pick the next performance improvement for polint. Update the state
table with the metrics-cliff numbers. Re-rank the candidates: the cold-path metrics miss, L1 thin
SDK, L5 binary layers, the edit-loop tier, and anything the fresh stage data newly surfaces. Pick
one, argue it both ways, and give it a spike plan with gates and kill criteria. Record the result in
`thoughts/perf-5.0-levers/11-next-after-metrics-cliff.md` with a README pointer. Mine the existing
raw evidence at `/opt/data/polint-perf5/`; take no new measurements, change no code. Docs only;
no consumer source or diagnostic text; never name the second measured consumer repository.
