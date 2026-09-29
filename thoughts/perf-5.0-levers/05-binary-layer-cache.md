# L5 — Binary layer-cache encoding + a word-at-a-time integrity hash (2026-09-28)

Situation: **warm**. Status: **owner decision — storage format + protocol bump.**

## What it is

Two changes to the on-disk syntax layer cache, bundled because they attack the same warm-path cost:

1. **Binary encoding** of the per-provider syntax layer blobs, replacing the current JSON encoding.
2. A **word-at-a-time integrity hash** (consuming 8+ bytes per step) replacing the current byte-wise
   FNV-1a digest used to verify the cache on load.

## Measured evidence

Current cache blob sizes on the Go+TS monorepo workload: **35 MB** (Go layer) + **12.6 MB** (TS layer)
of JSON, plus another **52 MB** across per-file entries. Every warm run decodes these and verifies an
FNV-1a digest byte by byte.

Main-thread CPU profiles (warm, base engine):

| Workload | Layer read (total) | JSON decode | FNV-1a verification |
|---|---|---|---|
| Go+TS monorepo | 25% of main-thread samples | 17% | 6.7% |
| OAIZ core | — | 12.9% | 6.2% |

After the campaign's shipped changes (deferred syntax metadata, memoized Go digest), **this decode +
verify pair is the largest polint-side cost left in a warm run** on both measured workloads — the
remaining warm-tier time on OAIZ core in particular is dominated by consumer rule code (see the
floor analysis in the [README](README.md)), but on the polint side, layer read is now the top line
item.

## Expected return

Projections from the profile breakdown above — **not yet built or measured directly**:

- Warm `go.syntax` stage: ~235 ms → **80–100 ms**
- Warm `ts.syntax` stage: ~85 ms → **~40 ms**
- Go+TS monorepo warm wall time: **−0.15…−0.2 s** (**−10…−14%** of warm)
- OAIZ core warm wall time: **−0.15…−0.25 s**
- OAIZ frontend warm wall time: **~−0.1 s**
- Cold tiers would also write less, since the same encoding applies to the write path.

## Cost / risk

- **Cache-protocol version bump**: every existing on-disk cache entry would miss once after the
  change, since the blob format changes. This is a one-time invalidation cost across every machine
  with a warm `.polint/cache`, not a correctness risk, but it needs to be communicated.
- **A new dependency**: a binary serialization format (the specific crate has not been selected in
  this campaign — that selection is part of the owner decision, not a settled detail here).
- **A second deterministic encoding path to maintain**: the JSON encoding (or its binary replacement)
  must stay byte-for-byte deterministic for cache-key and digest purposes, so introducing a second
  format means keeping two encodings' determinism guarantees in sync, or fully retiring the JSON path.
