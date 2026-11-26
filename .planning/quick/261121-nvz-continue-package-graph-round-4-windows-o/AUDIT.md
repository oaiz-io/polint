# Package-graph input audit

Scope: the inputs the analyzer reads from a repository, installed package, or
stored graph. Reviewed the file boundary and its downstream parse/walk for size,
depth, and cycles. The environments that execute user-selected tools and rule
packs are separate trust boundaries; this is not a claim that those tools are
sandboxed. A source-count or dependency-count budget is not a process-wide
memory bound.

| Input or walk checked | Entry points and outcome |
|---|---|
| Discovered Go and JS/TS source, including dependency source | `fs/mod.rs`, `repo_fs.rs`, `ts/repo_fs.rs`, and TS module discovery: bounded source reads, repository-path checks and regular-file checks where applicable. The file scanner reports oversized sources rather than inventing facts for them. |
| JS, TS, JSX and TSX syntax | `ts/parse.rs`, `ts/adapter.rs`, `polint-oxc-parser/src/{js,ts,jsx}`, Oxc semantic/visitor consumers: bound recursive productions **and** loop-built deep AST chains before handing off to the visitors. Serious parse errors report `parser/ts`; the later stages continue with no source declarations from that AST. |
| Go syntax | `go/{adapter,grammar,mir/lower}.rs`: a tree-sitter cursor checks depth iteratively before the recursive walks. An oversized tree reports `parser/go`; MIR does not lower it. |
| Scope identities and dynamic binding queries | `ts/symbol_graph`, `ts/sealed_bindings.rs`: bounded scope-key ancestry and one preorder index for scope containment; the eval and block-function queries no longer walk from every reference or binding through its parent chain. Eval string literals use the same bounded JS parser. |
| `tsconfig.json`, `jsconfig.json`, extends, and aliases | `ts/tsconfig_chain.rs`, `ts/module_graph/bounded_fs.rs`, `ts/module_graph/mod.rs`, `analysis_kernel/incremental/{input_snapshot,keys}.rs`: one iterative, visited-set walk, a 256-config ceiling, and 1 MiB and JSON/JSONC depth limits before parsing. Refused configs do not make an alias resolution exact. |
| Root and dependency `package.json` | `ts/module_graph/{mod,bounded_fs,formats/package_json}.rs`, `analysis_kernel/incremental/keys.rs`, `analysis_neutral/package_graph/identity.rs`, `ts/types/process.rs`: bounded reads and parses for indexing, resolution, workspace discovery and package identity. Oxc resolver's synthetic manifest view has the same input checks before its SIMD JSON parser. |
| `exports`, `imports`, package aliases and re-exports | `ts/module_graph/{mod,conditions}.rs` and `ts/points_to/package_graph/stitch.rs`: bounded JSON nesting, resolver link/cycle handling, and conservative alternatives. An unresolved branch cannot silently turn another branch into a complete call-site answer. |
| Package-manager workspace and lock metadata | `ts/module_graph/formats/{package_lock,js_lockfile,pnpm_workspace}.rs`: npm/shrinkwrap and Bun JSON/JSONC use the bounded parser. pnpm workspace/lock and Yarn Berry use bounded reads plus `serde_norway`'s nesting and alias repetition limits. Yarn Classic uses a line-oriented parser. |
| Go module/workspace metadata and user models | `go/lifecycle.rs`, `go/module_graph/{mod,formats}`, `go/{repo_fs,model_files,flow_models,route_models}.rs`: bounded repo-local `go.mod`, `go.work`, `go.sum`, and TOML model reads; model paths escaping the repository or exceeding the cap are refused. Go package loading itself is delegated to the Go toolchain. |
| polint config, discovery and ignore rules | `config/mod.rs`, `fs/mod.rs`, `ignores.rs`, `repo_fs.rs`: `.polint.toml` has a 1 MiB read limit, TOML decoding, and validated globs; ignore and directory traversal go through `ignore`. No separate aggregate bound for ignore-pattern count was established. |
| Stored graphs, corrections, observations and bench input | `analysis_neutral/package_graph/{identity,corrections}.rs`, `analysis_neutral/points_to/stored.rs`, `ts/points_to/package_graph/stitch.rs`, `polint-eval/src/harness/js_bench.rs`: checked package identity; finite byte/depth limits on JSON observations and solver inputs; stitch rejects incomplete graphs, duplicate identities and unlinked boundaries as appropriate. Corrections do not feed the engine. |
| Analysis caches, layer manifests/blobs, call facts and semantic store | `cache/mod.rs`, `analysis_kernel/{call_cache,incremental/layer_cache,store}.rs`: checked per-file and layer byte caps, parser-depth handling, payload digest validation for layer replay, and typed store validation. The call-cache reader lacked a byte cap; now a 256 MiB cap and bounded JSON check apply before restoration. Raw per-file JSON now also checks depth before `IgnoredAny` can accept it. |
| Sidecar and extension I/O | `go/semantic/{client,process,protocol}.rs`, `ts/types/{client,process,protocol}.rs`, `analysis/extensions/host.rs`, `subprocess/runner.rs`: checked timeouts and typed frames; the extension host caps output. The Go/TS sidecar artifact readers and generic command stdout/stderr collector do not enforce an aggregate byte ceiling. These are separate from manifest/package-graph JSON; no claim is made that sidecar memory or output disk usage is bounded by the analyzer's file limits. |
| Rule-host build/cache and tool inputs | `cache/{rules_store,rules_budget}.rs`, CLI Cargo/build mediation and `subprocess`: checked the managed store and disk budget. Rule-pack build scripts, configured executables and their output remain an execution/trust boundary, not input to the package-graph solver. |

The adversarial walk also examined copy closure, carried domain facts, and
access-path recording after source ingestion. Copy closure storage was removed
(`3a4b4061`), access-path projection no longer duplicates its prefix
(`1e503173`), and the domain solve has a carried-fact budget (`1b19298a`).
Exceeding a work budget does not justify marking a site complete.
