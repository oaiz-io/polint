# Package graph review: resolution coverage

- Opposite-mode conditions: an ESM import can select `require`, and a CJS
  require can select `import`, when explicitly enabled. Both overrides were
  reproduced with the bench's pinned, network-disabled Node sandbox.
- Config limits: the byte, JSON depth and `extends` limits leave a named
  resolution issue and an open call. The exact byte-boundary control still
  links the TypeScript alias; Node does not implement tsconfig aliases.
- Differential oracle: replayed seed `0x5EED_C0DE`, 2,401 maps, 11,346
  requests and 14 profiles. The stricter oracle found 876 exact answers that
  disagreed with at least one Node profile. No disagreements remained after
  preserving outcomes that select no runtime file.
- Resolver files: checked byte reads, config text reads, the `extends`
  preflight, manual manifest reads, and attach-to-call projection. A Unix
  FIFO without a writer timed out before the change; the reader now opens
  without blocking and checks the opened handle before reading.

## Resolution refusal audit

| Boundary | How refusal remains open |
|---|---|
| Importer's `tsconfig.json` or `jsconfig.json`; inherited config reads | Nearest-config preflight and the resolver's config reads preserve limits and read failures by path. Extends lookups retain the refusal instead of caching an empty child list. |
| Resolver's own and dependency `package.json` reads (`exports`, `imports`, `main`, `module`, `browser`) | Bounded filesystem distinguishes a missing document from a refused one. The manifest cache retains this distinction when the condition walker rechecks scope or package candidates. |
| Loader and mode joins, nested condition targets, on-disk stitch | Refusal is collected across the resolution, not inferred from the final loader result. Known paths can remain conservative, but the join or stitch cannot promote them to a single exact link. The condition walk reports profiles that may select no runtime file. |
| Direct calls to an imported binding | A syntactically bare specifier is not evidence of verified external resolution. The direct path uses the actual module-resolution result before closing a call; the named issue is available in `ResolvedImports`. |

The wider input inventory remains in the preceding review's audit. The
source, lockfile, stored graph, correction and cache readers are not alternative
specifier resolvers. This round does not change their input budgets.
