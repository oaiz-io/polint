---
status: complete
---

This task weighed more runtime performance (A) against capability expansion (B) after v0.4.4, under
the owner's constraint that compile, install and distribution savings do not count. It wrote
`thoughts/perf-5.0-levers/12-next-direction.md` and added a post-v0.4.4 update to the README index
and expected-impact table.

The pick is B, starting with the typed middle layer. Go and TS structure facts, test facts and
non-code files come first, then framework models, then type facts. Deep analysis comes last, behind a
measured entry gate. The first step is a one-week spike that rewrites scratch copies of OAIZ's three
heaviest scanner-based rules against spike facts.

Three throwaway measurements back it. They ran on a private `--shared` clone of the OAIZ bench commit
`2956a791a7`; raw evidence is in `/opt/data/polint-next-direction/`, and nothing was committed.
- **A warm full-repo CPU profile on v0.4.4.** polint code is 4.3% of the rules phase. Consumer code
  and regex make up 93%, and at least half of it re-derives syntax polint already parsed.
- **An A/B of the 64 MiB layer read limit.** OAIZ's 96 MB Go layer is rejected and rebuilt on every
  warm run. Raising the limit gives −0.30 s warm (5 interleaved rounds, identical reports).
- **A package-scope `control_flow` scan.** With the default `include_tests` setting, the Go semantic
  sidecar passed 12 GB on one 41-file package. With it off, the scan ran 26 s cold and 3.8 s warm, and
  left 1,112 unknowns.

A case-insensitive grep for the forbidden consumer name returned zero hits across the diff.
