# Methodology: how these numbers were measured (2026-09-28)

Every number in this directory traces back to the perf-5 campaign's raw evidence, preserved outside
this repository at `/opt/data/polint-perf5/` on the measurement host (`baseline/`, `ab/`, `dev/runs/`,
`prof/out/`, `timings/`, `logs/`, `decisions.md`). This file explains how to read that evidence and the
rules that keep it honest.

## Environment

AMD EPYC-Rome, 16 visible CPUs, but a cgroup `cpu.max` of `600000 100000` — a **6-core quota** — and
22 GiB of cgroup memory, on Linux 6.8. polint's default job count is derived from a quota-aware
`available_parallelism()` at 80% of the quota, which **resolves to 4 jobs** in this container, not 16
and not the 12 recorded in an earlier, stale `env.txt`. The harness exports that same value — 4 — to
the rule host, rayon, cargo, and Go (`POLINT_JOBS` / `RAYON_NUM_THREADS` / `CARGO_BUILD_JOBS` /
`GOMAXPROCS`), so every number in this directory is **at 4 jobs**, matching a real 6-core-quota
container rather than the host's full 16 visible CPUs.

The host is **shared**: foreign load of 1–14 cores during runs, disk free swinging 0.6–91 GB, and the
consumer-repo bench copies were deleted externally mid-campaign and recreated read-only via
`git archive` at the exact baseline commits (file counts re-verified after recreation).

## Tier definitions

- **ice-cold**: fresh `CARGO_HOME`, rule-host build target, XDG caches (the machine-global rules store
  and the Go sidecar cache), `GOCACHE`, `GOMODCACHE`, and `.polint/cache` all removed. Every run pays
  dependency download, rule-pack compile, and analysis.
- **cold**: only `.polint/cache` removed; toolchains and dependency caches stay warm.
- **warm**: nothing removed; this is a repeat run against an already-populated cache.
- **edit-loop**: warm cache with a small number of source files changed since the last run. **Not
  measured in this campaign** — see [`08-next-targets.md`](08-next-targets.md) item 7.
- **full-repo**: the consumer's entire repository in one `check` invocation, as opposed to a
  profile-scoped subset. For OAIZ this is a separate, much larger, rules-dominated workload from the
  profile-scoped `core`/`frontend` commands the owner actually runs day to day.
- **consumer**: findings measured on a scratch copy of a consumer's rule pack rather than on the
  polint engine itself (e.g. L4).

## Interleaved before/after sampling

Every A/B comparison in this campaign alternates arm order round to round (BASE, FINAL, BASE, FINAL,
...) rather than running all of one arm's samples and then all of the other's. This matters because
the host is shared and under variable foreign load: block-sampling (all of arm A, then all of arm B)
lets host-load drift masquerade as a real difference between the two arms, in either direction.
Interleaving cancels drift in expectation because both arms see the same load distribution across the
measurement window. Every headline number in this directory that carries a percentage or a multiplier
is a **median of interleaved samples**, never a single run and never a block-sampled comparison.

Formal A/B runs (the headline table) additionally gave each arm its own full copy of the consumer
repository, with its rule pack pointed at a local git copy of that arm's engine revision, and its own
release driver binary — so the two arms differ only in engine code, not in any shared mutable state.

## Determinism gates: stdout identity

Every measured run's stdout was hashed (sha256, `--format json`). A change is only reported as a
performance change, not silently also a behavior change, if every arm's runs in a comparison produced
byte-identical stdout to each other and to the Phase-1 baseline. This held for all 54 formal A/B runs
(3 workloads × 3 tiers × 3 rounds × 2 arms) plus every dev A/B and consumer-side scratch-copy
measurement cited in this directory — each file above states the relevant digest prefix or an
"identical output" claim where it applies, rather than asserting it blind.

## Wall-clock measurement and quantization

Until approximately 2026-09-27 19:50 UTC (covering the baseline measurements and the first three dev
A/Bs), the harness detected process exit by polling every 0.1 s, so those wall-time samples carry up
to ~0.1 s of **positive quantization** — equal in expectation across both arms of any A/B run in that
window, so it does not bias a comparison, but it does mean individual absolute numbers from that window
carry more noise than later ones. From that point on (the fourth and fifth dev A/Bs, the L6/LTO
measurement, and the formal headline A/B), the harness waits on a `pidfd` instead, and wall time is
exact.

Peak RSS is `ru_maxrss` — exact, the largest single process (rustc during an ice-cold build, the rule
host otherwise) — except where a file explicitly marks a number as sampled. The 0.1 s tree-RSS sampler
used for host-level bookkeeping elsewhere in the raw evidence can miss short peaks (one observed case:
a sampled 103 MB vs. an exact 129.7 MB for the same run), which is why every RSS number in these
lever files is the exact `ru_maxrss` figure, not the sampled tree figure.

## Honesty rules

- Every number in this directory that is not directly present in the source material
  (`/opt/data/polint-perf5/REPORT.md` and its supporting files) is explicitly marked
  **"(unmeasured)"** or "projected" in the file where it appears, including every cell of the
  README's expected-impact table.
- No lever's "expected return" is presented as a measurement unless a prototype was actually built and
  timed (L4 and L6 are; L1, L3, L5, and L7's returns are explicitly projections or, for L7, entirely
  unmeasured).
- Both measured rejections (cursor-based Go walk; ThinLTO off as a quiet default) are recorded with
  their actual measured regressions so neither gets re-proposed without someone re-measuring first.

## Raw artifact pointers

For anyone auditing a specific number: `/opt/data/polint-perf5/REPORT.md` is the report of record this
directory is built from; `baseline/baseline-oaiz.md` and the equivalent OAIZ-only baseline file hold
the full per-run phase-map tables (files, stage timings, RSS, stdout hashes) behind the summarized
numbers here; `decisions.md` holds the rejected-experiment and consumer-finding write-ups verbatim;
`report-next.md` holds the in-contract next-targets list; and `ab/`, `dev/runs/`, `prof/out/`, and
`timings/` hold the raw per-round sample data and CPU profiles behind every median reported above.
