# `polint-ts-types-1` wire protocol

The TS type sidecar speaks newline-delimited JSON on stdout. One JSON object
per line; no partial lines; stderr is diagnostic text only and is captured into
the failure message when the process exits non-zero.

Decoder: `crates/polint/src/ts/types/protocol.rs`.
Emitter: `tools/polint-ts-types/index.js`, embedded at
`crates/polint/src/ts-sidecar/polint-ts-types/`.

## Framing

Every line carries `schema` and `kind`. `schema` must equal
`polint-ts-types-1` on every line, including the session frames.

```
{"schema":"polint-ts-types-1","kind":"session_begin","typescript_version":"5.9.3","node_version":"v22.22.3"}
... zero or more `phase` and row frames ...
{"schema":"polint-ts-types-1","kind":"session_end","elapsed_ms":812,"projects":1,"files":84,"rows_emitted":5120}
```

These are typed decode errors, not panics and not silently tolerated states:

| Condition | Error |
|---|---|
| line is not JSON | `InvalidJson` |
| `schema` is anything else | `UnsupportedSchema` |
| `kind` is outside the known set | `UnknownKind` |
| any frame before `session_begin` | `RowBeforeBegin` |
| any frame after `session_end` | `RowAfterEnd` |
| second `session_begin` | `DuplicateBegin` |
| second `session_end` | `DuplicateEnd` |
| no `session_begin` | `MissingBegin` |
| no `session_end` | `MissingEnd` |

A missing terminator is the important one: it is how a killed or crashed
sidecar is distinguished from one that legitimately found nothing.

## Session frames

`session_begin`

| Field | Type | Meaning |
|---|---|---|
| `typescript_version` | string | `ts.version` of the module actually loaded |
| `node_version` | string | `process.version` |
| `typescript_path` | string | resolved module directory, for diagnostics |

`session_end` carries the session totals with the same field names as `phase`.

`phase`

| Field | Type | Meaning |
|---|---|---|
| `phase` | string | stage name: `resolve_typescript`, `discover_projects`, `create_program`, `walk_callsites` |
| `elapsed_ms` | u64 | wall time closed at the stage boundary |
| `projects` | u64 | tsconfig units seen so far |
| `files` | u64 | program source files, excluding declaration files |
| `rows_emitted` | u64 | rows written so far |
| `peak_heap_bytes` | u64 | `process.memoryUsage().heapUsed` sampled at the boundary, not a continuously sampled high-water mark |

## Row frames

### `project`

One per tsconfig unit actually loaded.

| Field | Type | Meaning |
|---|---|---|
| `project` | string | repo-relative tsconfig path |
| `options_digest` | string | digest of the resolved compiler options that affect type semantics |
| `typescript_version` | string | the loaded compiler version |
| `file_count` | u64 | non-declaration program files |
| `stable_key` | string | length-prefixed from `project` |

### `callable`

A function-like declaration the type checker resolved. Emitted only for files
in scope.

| Field | Type | Meaning |
|---|---|---|
| `project` | string | owning tsconfig unit |
| `callable` | string | declaration identity: `<relative-file>:<start-byte>:<name>` |
| `name` | string | declared or inferred name; empty for anonymous |
| `file` | string | repo-relative path |
| `span` | span | declaration span |
| `kind` | string | `function`, `method`, `constructor`, `arrow`, `getter`, `setter`, `class` |
| `stable_key` | string | length-prefixed from `callable` |

### `callsite`

| Field | Type | Meaning |
|---|---|---|
| `project` | string | owning tsconfig unit |
| `callsite` | string | `<relative-file>:<start-byte>` |
| `enclosing` | string | `callable` identity containing this site; empty at module top level |
| `file` | string | repo-relative path |
| `span` | span | the whole call expression |
| `call_kind` | string | `call`, `method`, `new`, `tagged_template`, `decorator` |
| `status` | string | `resolved`, `unresolved`, `any_receiver`, `union`, `external` |
| `reason` | string | present when `status` is not `resolved` |
| `stable_key` | string | length-prefixed from `callsite` |

### `callee`

Zero or more per call site. A site the checker resolved to several signatures
(a union receiver, an overload set) emits one row per distinct declaration.

| Field | Type | Meaning |
|---|---|---|
| `project` | string | owning tsconfig unit |
| `callsite_stable_key` | string | joins to the `callsite` row |
| `callable` | string | in-scope `callable` identity, or empty when external |
| `external` | string | moniker for a declaration outside scope: `node_modules:<package>/<subpath>#<name>` or `lib:<lib-file>#<name>` |
| `file` | string | repo-relative path of the declaration, empty when external |
| `span` | span | declaration span, absent when external |
| `stable_key` | string | length-prefixed from callsite key + callee identity |

The external moniker is why declarations outside scope never cross the wire as
full rows: a call into `node_modules` or `lib.dom.d.ts` is reported as a named
external target, and no `callable` row is emitted for it.

### `receiver`

At most one per call site, describing the type of the receiver (for method and
property calls) or of the callee expression (for value calls).

| Field | Type | Meaning |
|---|---|---|
| `project` | string | owning tsconfig unit |
| `callsite_stable_key` | string | joins to the `callsite` row |
| `printed` | string | `checker.typeToString` output, capped at 512 chars with a trailing ellipsis when truncated |
| `is_any` | bool | the type is `any` |
| `is_unknown` | bool | the type is `unknown` |
| `union_size` | u64 | 0 when not a union, otherwise the constituent count |
| `stable_key` | string | length-prefixed from the callsite key |

### `any_density`

One per in-scope file with at least one call site.

| Field | Type | Meaning |
|---|---|---|
| `project` | string | owning tsconfig unit |
| `file` | string | repo-relative path |
| `callsites` | u64 | call sites in the file |
| `any_receivers` | u64 | sites whose receiver is `any` or `unknown` |
| `stable_key` | string | length-prefixed from `file` |

`any_receivers / callsites` is the Q22 ratio. The Rust side owns the
thresholds; the sidecar reports only the counts.

### `diagnostic`

| Field | Type | Meaning |
|---|---|---|
| `category` | string | `setup_missing`, `project_error`, `unsupported` |
| `message` | string | human-readable, no absolute paths outside the repo root |
| `file` | string | optional repo-relative path |

## Spans

```
"span":{"start_byte":120,"end_byte":137,"start_line":8,"start_column":3,"end_line":8,"end_column":20}
```

Byte offsets are UTF-8 byte offsets from the start of the file, which is what
the join to `CallSiteFact` / `FunctionFact` compares. TypeScript reports
positions as UTF-16 code-unit offsets into the file text, so the sidecar
converts them; a file containing astral-plane characters would otherwise join
against nothing after the first such character.

Lines and columns are 1-based, matching `internal_core::Span`.

## Stable keys

Same recipe as the Go tier: length-prefixed concatenation,
`len(part) ":" part ";"` per part, so no separator can be forged from content.
Keys are derived from repo-relative paths and in-file byte offsets only, never
from a program-wide position counter — the Go tier learned that the hard way
(`emit.go` `positionKey`), where a FileSet-global offset made 1.5% of callsite
keys differ between two runs of the same binary over the same tree.

## Invocation

```
node <sidecar>/index.js \
  --root <repo-root> \
  --projects <comma-separated tsconfig paths, or empty to discover> \
  --scope-files <path to newline-delimited repo-relative file list> \
  --typescript <resolved typescript module dir> \
  --ndjson
```

`--ndjson` is required; without it the sidecar exits 2 with usage. `--root` is
the analysis root and every emitted path is relative to it. `--scope-files`
bounds emission only: the program is still built over the full import closure,
because a type checker that cannot see an import cannot type the call.
