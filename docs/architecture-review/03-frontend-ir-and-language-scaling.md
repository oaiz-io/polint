# 03 — Frontends, IR, and the Cost of Language N+1

Audit date: 2026-07-28. Commit base: `1263208a` (branch `biarritz`).
Scope: `crates/polint/src/{go,ts,core,analysis,analysis_kernel,symbol_graph,module_graph}`,
`crates/polint/go-sidecar/`, `tools/polint-go-symbols/`.

All paths absolute. All LOC are `wc -l` on the file as committed (the repo inlines
`#[cfg(test)]` modules, so file totals run 30–60% test code; where the distinction
matters it is called out).

---

## Executive verdict

Three findings dominate everything else in this document.

1. **There is no adapter contract.** Not a trait, not a registry, not a vtable.
   `grep` for `trait .*Language`, `trait .*Frontend`, `trait .*Adapter` across the
   crate returns nothing. The seventeen traits that exist are all unrelated
   internals. What `AGENTS.md:6` and `docs/CAPABILITY-FULFILLMENT-RESEARCH.md:579`
   call "the adapter contract" is **prose in a markdown file** plus a
   **duck-typed naming convention**: `go/adapter.rs` and `ts/adapter.rs` each
   export a free function named `analyze_with_plan_options_and_cache_stats` with
   byte-identical signatures, and the kernel calls both by hand, back to back, at
   `/Users/emilwareus/conductor/workspaces/exlint/biarritz/crates/polint/src/analysis_kernel/mod.rs:191`
   and `:209`.

2. **The MIR is not a real IR.** It is a 883-LOC schema fed by 5,865 LOC of
   per-language lowering — a 1:6.6 ratio. It has no basic blocks, no terminators,
   no SSA, no types, no operators, no exceptions, no concurrency, and no closures.
   `MirStatement` and `MirTerminator` are declared at
   `analysis/mir/body.rs:32` and `:49` and **constructed nowhere in the crate**.
   The single largest consequence: because MIR discards control-flow structure,
   the CFG layer reconstructs loops by **substring-matching source-text evidence
   strings** — `contains_token(&evidence, &["for ", "while", "do while", "for-of",
   "for-in", "for await"])` at `analysis/cfg/lower_ts.rs:249-254`, backed by
   `fn contains_token` at `:482`. An IR whose consumers must grep the original
   source text to recover a `for` loop is a fact-annotation format, not an IR.

3. **The TS pipeline already forked around the MIR.** `analysis/calls/ts_value_flows.rs`
   is 11,898 LOC (the largest file in the crate), re-parses source with
   `oxc_parser::Parser` at `:64`, `:324`, `:396`, builds its own
   `oxc_resolver::Resolver` at `:243`, and walks the Oxc AST directly for the
   call graph. It is 2.6× the size of the MIR contract + both lowerers combined.
   When the team needed real JS precision, they did not extend the MIR — they
   built a parallel frontend beside it. That is the most honest available signal
   about whether the MIR is load-bearing.

The engine is not architecturally ready for Python. It is, however, closer than
those three findings suggest, because three genuinely language-neutral layers do
exist and are well built: the solver engine (`SolverPolicy`), the interprocedural
summary/SCC closure, and the abstract points-to solver. Section (e) is written
around preserving those and replacing the rest.

---

## (a) The real adapter contract, as it exists in code

### The claim

> "Adapters today cover Go (tree-sitter) and TypeScript / JavaScript (Oxc); more
> languages can be added through the same adapter contract."
> — `/Users/emilwareus/conductor/workspaces/exlint/biarritz/AGENTS.md:6`

The referenced contract is a seven-bullet prose list at
`/Users/emilwareus/conductor/workspaces/exlint/biarritz/docs/CAPABILITY-FULFILLMENT-RESEARCH.md:579-591`
("Declare supported capabilities…", "Harvest requested facts…", "Preserve stable
IDs and spans…"). Nothing in the compiler enforces any of it.

### What actually exists

**One duck-typed pair of free functions.** Both adapters expose the same four
names with identical signatures:

```rust
// go/adapter.rs:59  and  ts/adapter.rs:91 — byte-identical
pub(crate) fn analyze_with_plan_options(
    db: &mut AnalysisDb, cache: &crate::cache::Cache,
    config_hash: &str, rule_hash: &str,
    plan: &AnalysisPlan, parallel: bool,
) -> Vec<Diagnostic>
```

The production entry point is the `_and_cache_stats` variant
(`go/adapter.rs:78`, `ts/adapter.rs:110`). Each adapter declares its **own private**
`ProviderAnalysisResult` struct with identical fields — `go/adapter.rs:72` and
`ts/adapter.rs:104` are two distinct types, not one shared type. The only
substantive difference between the two adapters is one line each:

- `go/adapter.rs:87` — `.filter(|file| file.language == Language::Go)`
- `ts/adapter.rs:119` — `.filter(|file| file.language.is_ts_family())`

Nine private helper shapes are duplicated verbatim under mirrored names in both
files (`{go,ts}_syntax_layer_key`, `parse_{go,ts}_syntax_layer_payload`,
`restore_syntax_layer_payload`, `write_syntax_layer_payload`,
`validate_syntax_layer_payload`, `source_text_digest`, `parser_parameter_digest`,
`cache_write_diagnostic`).

**Dispatch is hand-written duplication at every fan-out site.** There are twelve
of them, each a hardcoded two-call sequence:

| Layer | Dispatch site | Shape |
|---|---|---|
| Kernel syntax | `analysis_kernel/mod.rs:191`, `:209` | `crate::go::…` then `crate::ts::…`, unconditional |
| Go deep semantics | `analysis_kernel/mod.rs:424` | Go-only; **no TS counterpart exists** |
| Module graph | `module_graph/mod.rs:1252-1258` | `if is_ts_family { ts::resolve_ts_import } else if Go { go::resolve_go_import } else { unsupported_language() }` |
| Module topology | `module_graph/mod.rs:1628-1629` | `go::collect_go_topology(..).merge(ts::collect_ts_topology(..))` |
| Symbol graph | `symbol_graph/mod.rs:229`, `:234` | `ts::derive_ts_symbols` + `go::derive_go_symbols` → `merge_language_output` |
| MIR | `analysis/provider.rs:29` | `merge_language_outputs([lower_go_mir(db), lower_ts_mir(db)])` |
| CFG | `analysis/cfg/provider.rs:64-65` | `lower_go_cfg(db)` then `merge_base_output(.., lower_ts_cfg(db))` |
| Types | `analysis/types/provider.rs:42-43` | `go::derive_go_type_value_alias` + `ts_js::derive_ts_js_type_value_alias` |
| Refined calls | `analysis/refined_calls/provider.rs:76`, `:79` | `go::derive_go_refinements` + `ts_js::derive_ts_js_refinements` |
| Entrypoints | `analysis/entrypoints/extract.rs:13-14` | `recognize_go_entrypoints(db)` + `recognize_ts_entrypoints(db)` |
| Solver | `analysis/solver/provider.rs:143-148` | hardcoded `vec![GoRtaPolicy, TsTokensPolicy, (TsObjectModelPolicy)]` |
| Call algorithm | `analysis/refined_calls/provider.rs:392-393` | `Language::Go => GoRta`, `is_ts_family() => PointsTo` |

**The provider registry is metadata, not execution.**
`analysis_kernel/provider.rs:255` declares `const PROVIDER_MANIFESTS: &[ProviderManifest]`
with 23 entries. `ProviderManifest` (`:2-11`) carries `id, kind, inputs, outputs,
language_scope, cache_policy, schema_versions, precision_ceiling` and **no function
pointer**. Execution order is a hand-written straight-line body in
`AnalysisKernel::run` (`analysis_kernel/mod.rs:92`–~960) with output digests
threaded manually between stages; the manifest array is kept in sync by an
assertion test at `analysis_kernel/provider.rs:938-961`.

Of the 23 providers, only three declare a language scope (`polint.go.syntax` `:267`,
`polint.ts.syntax` `:285`, `polint.go.semantic` `:449`). The other twenty are
labelled `MultiLanguage` — but as the dispatch table above shows, that label is
aspirational: they are Go-arm + TS-arm merges internally.

**Config is two hardcoded struct fields.**

```rust
// config/mod.rs:275
pub(crate) struct LanguageConfig {
    #[serde(default)] pub(crate) go: BTreeMap<String, toml::Value>,
    #[serde(default)] pub(crate) ts: BTreeMap<String, toml::Value>,
}
```

**The `Language` enum is closed and matched exhaustively in 19 files.**
`core/mod.rs:184` declares `Go | TypeScript | Tsx | JavaScript | Jsx | Unknown`.
`rg 'Language::Go\s*=>'` finds 23 arms across 19 files, every one of which is a
compile error when a variant is added — which is the one genuinely good property
here, since the compiler will at least enumerate the work.

**The extension protocol is not a frontend seam.** `analysis/extensions/` (4,128
LOC) is a real out-of-process provider protocol with an NDJSON handshake
(`protocol.rs:5-6`, schemas `polint-extension-handshake-v1` /
`polint-extension-provider-run-v1`), budgets, and trust gating. But extensions may
only emit a fixed family whitelist — the six `type_value_alias.*` families and
`refined_calls.edge` (`analysis/extensions/sinks.rs:3-19`) — and their output
lands in a segregated `extension_facts` vector (`core/mod.rs:744`), not the core
fact tables. An extension cannot contribute functions, symbols, module edges, or
MIR bodies. It is a *refinement* channel, not a frontend channel.

### Honest summary of (a)

The contract is: *write a module that looks like the other two, then edit twelve
call sites, two config fields, one const array, and every exhaustive `match` on
`Language`.* Convention-by-copy. The compiler helps only via enum exhaustiveness.

---

## (b) To add Python you must write X

Estimates are **production LOC** (excluding inline tests, which the repo convention
roughly doubles). Anchored on the measured Go and TS/JS equivalents.

### New code

| # | Item | Model to copy | Existing Go / TS LOC | Python est. (prod) | Notes |
|---|---|---|---|---|---|
| 1 | `python/adapter.rs` — parse, syntax facts, per-file cache layer | `go/adapter.rs` 1,673 / `ts/adapter.rs` 4,348 | 6,021 | **1,500–2,500** | Includes the 9 duplicated cache-layer helpers, again |
| 2 | `analysis/mir/lower_python.rs` | `lower_go.rs` 1,913 / `lower_ts.rs` 3,952 | 5,865 | **2,000–3,000** | Python has both Go's and JS's hard cases |
| 3 | `analysis/cfg/lower_python.rs` | `lower_go.rs` 802 / `lower_ts.rs` 827 | 1,629 | **600–900** | Only exists because MIR has no blocks (see (c)) |
| 4 | `symbol_graph/python.rs` | `go.rs` 2,587 / `ts.rs` 3,067 | 5,654 | **1,500–2,500** | Incl. ~13 duplicated stable-key fns, 5 enum mappers, 3 draft builders, `derive_python_semantic_index` |
| 5 | `module_graph/python.rs` | `go.rs` 2,261 / `ts.rs` 3,209 | 5,470 | **1,000–2,500** | `resolve_python_import` + `collect_python_topology` + a `PythonPackageIndex` |
| 6 | `module_graph/formats/` — `pyproject.toml`, `requirements.txt`, `poetry.lock`, `uv.lock` | `js_lockfile.rs` 778, `go_mod.rs` 400, `package_json.rs` 330 | 1,982 (6 files) | **600–1,200** | |
| 7 | `analysis/types/python.rs` | `go.rs` 848 / `ts_js.rs` 1,264 | 2,112 | **700–1,000** | ~60% is a verbatim clone of the sibling (see (e)) |
| 8 | `analysis/entrypoints/recognizers_py.rs` | `recognizers_go.rs` 1,498 / `recognizers_ts.rs` 2,397 | 3,895 | **1,000–1,600** | Flask/FastAPI/Django/Click/Celery/pytest, all hand-coded; ~15 helpers copied a third time |
| 9 | `analysis/solver/py_*/` — `inputs.rs`, `fixpoint.rs`, `dispatch.rs` + a `PyPolicy` | `ts_object_model/` 2,014 / `go_rta/` 3,739 | 5,753 | **1,300–1,800** | Engine itself is free (real trait) |
| 10 | `py_value_flows.rs` — the AST recognizer bank | `ts_value_flows.rs` 11,898 (8,536 prod, **69 recognizers**) | 11,898 | **3,000–8,500** | **Dominant risk.** Decorators, `functools.partial`, bound methods, `__call__`, `getattr`, monkey-patching, metaclasses |
| 11 | `py_points_to/harvest.rs` (field-sensitive heap) | `js_points_to/harvest.rs` 1,343 | 1,343 | **1,000–1,400** | Solver core reusable; harvest is a pure AST walk |
| 12 | `python/object_model/` | `ts/object_model/` 2,491 | 2,491 | **1,300–1,600** | Classes, MRO, `self` binding, `__dict__`, descriptors |
| 13 | `analysis/refined_calls/python.rs` | `go.rs` 636 / `ts_js.rs` 696 | 1,332 | **350–450** | 55–61% verbatim overlap between the existing two |
| 14 | Optional: Python sidecar (`ast`/`symtable`/`mypy`) | `go-sidecar/polint-go-frontend` 1,352 Go prod | 1,352 | **800–1,500 (Python)** | Only if you want real type info; see (e)/(f) |

**New-code subtotal: ~16,600–30,450 production LOC** (~28,000–52,000 with the
repo's inline-test convention).

### Edits to existing files

| Item | Sites | Effort |
|---|---|---|
| `core/mod.rs:184` add `Language::Python` + `from_path` | 1 | trivial |
| Fix every exhaustive `match` on `Language` | **23 arms across 19 files** | mechanical, compiler-guided |
| Twelve hardcoded dispatch fan-outs (table in (a)) | 12 | mechanical |
| `analysis_kernel/mod.rs` — insert `python_output_digest` and thread it into the module-graph and symbol-graph dependency vectors | `:191`–`:275` | fiddly, digest plumbing |
| `analysis_kernel/provider.rs` — add `polint.python.syntax` to `PROVIDER_MANIFESTS` + the two duplicated order lists at `:940` and `:972` | 3 | plus **~7 provider-order snapshot assertions** across the test suite |
| `config/mod.rs:275` add `python:` field | 1 | trivial |
| `analysis_plan.rs:854` `language_label`; `cli/mod.rs:3442`; `symbol_graph/stable_id.rs:280`; `symbol_graph/semantic.rs:872`; `module_graph/model.rs:522`; `analysis/cfg/lower_ts.rs:486` — six independent label maps | 6 | duplicated |
| `analysis/identity/facts.rs:30` `LanguageTag` (closed at Go/TS/JS) | 1 | trivial |
| `analysis/calls/extract.rs:305`, `:507` — two TS heuristics | 2 | small |

### What you get for free (genuinely)

| Layer | LOC | Evidence |
|---|---|---|
| `analysis/solver/engine.rs` + `SolverPolicy` trait | 1,485 | `analysis/solver/policy.rs:77`; `Vec<Box<dyn SolverPolicy>>` at `engine.rs:64`; 4 implementors |
| `analysis/summaries/` (interprocedural summaries, closures, SCC) | 7,321 | **verified zero `Language::` in `builder.rs` prod lines 1–947**; zero `oxc` imports directory-wide |
| `analysis/points_to/` (abstract Andersen solver) | 1,306 | `points_to/solver.rs` — no `Language::`, no AST |
| `analysis/calls/direct.rs` prod | 458 | zero `Language::` outside test fixtures |
| `analysis/domains/`, `analysis/data_flow/`, `analysis/reachability/` cores | — | consume neutral facts |
| Solver budget / provenance / store / validate / cache-key | ~2,200 | |

### Bottom line

**~17k–30k production LOC of new per-language code, plus edits to ~25 existing
files and ~7 snapshot assertions, to bring Python to Go/TS parity.**

A *shallow* Python tier (syntax facts, functions/classes, imports, literals,
pytest facts — the tier `docs/roadmap/08_ENTRY_8_PYTHON_ADAPTER.md` actually
proposes) is items 1, 5, 6, 8 partial ≈ **4,000–7,000 LOC**. That is achievable.
Parity — call graph, points-to, refined calls — is items 9–13, and that is where
the ~10k–14k additional LOC live with no reusable seam whatsoever.

---

## (c) MIR capability matrix

### What the contract actually is

`analysis/mir/mod.rs` is **four lines** (`pub(crate) mod {body, lower_go, lower_ts, op};`).
The entire language-neutral schema is:

| File | LOC |
|---|---|
| `analysis/mir/mod.rs` | 4 |
| `analysis/mir/op.rs` | 129 |
| `analysis/mir/body.rs` | 295 (incl. ~195 test) |
| `analysis/places.rs` | 455 |
| **Neutral contract total** | **883** |
| `analysis/mir/lower_ts.rs` | 3,952 |
| `analysis/mir/lower_go.rs` | 1,913 |
| **Per-language lowering total** | **5,865** |

**Ratio: 1 line of shared IR per 6.6 lines of per-language lowering.** For
comparison, LLVM IR, Rust MIR, and Soot Jimple all sit on the other side of 1:1 —
the whole point of an IR is that the shared part is the expensive part.

The complete operation vocabulary — `MIR-operation kind` at `analysis/mir/op.rs:21`,
**9 variants**:

```rust
StorageLive { place }
Bind        { place, value }
Assign      { place, value, mode: AssignMode }
Read        { place }
Write       { place, value }
Branch      { predicate: MirPredicateId, predicate_place: Option<PlaceId> }
Call        { site: CallSiteId, callee: MirValue, arguments: Vec<PlaceId>, return_place: PlaceId }
Return      { value: Option<MirValue> }
Unsupported { unsupported: UnsupportedId }
```

The complete value vocabulary — `MirValue` at `op.rs:70`, **5 variants**:
`Literal { value: String }`, `Place(PlaceId)`, `Temporary(MirValueId)`,
`CallReturn(CallSiteId)`, `Unknown { evidence: String }`.

`MirOutput` (`body.rs:66`) contains exactly `{ bodies, places, operations, unsupported }`.

### Capability matrix

| Construct | Modelled? | Evidence |
|---|---|---|
| Local variable assignment | **Yes** | `MIR-operation kind::Assign`, `op.rs:29` |
| Field / property read & write | **Yes** | `PlaceProjection::{Field, Property}`, `places.rs:51-52` |
| Direct & indirect call, args, return place | **Yes** | `MIR-operation kind::Call`, `op.rs:45` |
| `if` / two-way branch | **Partial** | `Branch { predicate }` — the predicate itself lives outside MIR as an opaque `MirPredicateId` |
| Return | **Yes** | `op.rs:51` |
| Deref | **Yes** | `PlaceProjection::Deref`, `places.rs:55` |
| Array/map index (known key) | **Yes** | `PlaceProjection::IndexKnown(String)`, `places.rs:53` |
| Assignment *mode* (declaration / overwrite / partial / simultaneous / projection-mutation) | **Yes — unusually good** | `AssignMode`, `op.rs:60`. This is a genuine design strength: Go's `a, b = b, a` and JS destructuring are distinguishable |
| Explicit unknown-with-evidence + conservative action | **Yes — genuinely good** | `unsupported-semantic fact record` `op.rs:79`; `ConservativeAction::{SkipOperation, HavocAffectedPlaces, PreserveWithUnknownValue, StopLowering}` `op.rs:117`; per-domain blast radius `UnsupportedDomain` `op.rs:106` |
| **Basic blocks** | **NO** | `MirOutput` `body.rs:66` has no blocks field. Operations carry only `ordinal: u32` |
| **Terminators** | **NO** | `MirTerminator` declared `body.rs:49`, `MirTerminatorKind` `body.rs:59` — **constructed nowhere in the crate** (verified by grep) |
| **`MirStatement`** | **NO** | declared `body.rs:32`, constructed nowhere; the `#[cfg(test)] expect(dead_code)` reason at `body.rs:27-30` says "before lowering populates them" — it never did |
| **SSA / phi nodes** | **NO** | no versioning anywhere; `PlaceRoot::Local { function, name }` `places.rs:24` is name-keyed |
| **Types on places or operations** | **NO** | `place-fact record` `places.rs:11` has no type field; `MirValue::Literal` is a bare `String` |
| **Binary / unary operators** | **NO** | no `BinOp` in `op.rs`. `a + b` lowers to two operand places and a temporary. Only special case: string-concat for property keys, `lower_ts.rs:866`, `:880` |
| **Aggregate construction** (struct/object/array/tuple literal) | **NO** | no `Aggregate`/`New`/`Alloc` operation |
| **Exceptions** — `try`/`catch`/`finally`/`throw`, Go `panic`/`recover` | **NO** | `"try"`, `"throw"`, `"catch destructuring"` → `Unsupported` (`lower_ts.rs:2894-2942`); `"panic"`, `"recover"` → `Unsupported` (`lower_go.rs:1275-1295`). No unwind edges, no exception terminator |
| **Loops** — `while`, `do-while`, `for-of`, `for-in`, `for await` | **NO** | all in the `Unsupported` list at `lower_ts.rs:2906-2916` |
| **`switch` / `select` / `fallthrough` / `goto`** | **NO** | `lower_ts.rs:2908`; `lower_go.rs:1283`, `cfg/lower_go.rs:301`, `:306` |
| **`break` / `continue` / labeled statements** | **NO** | `lower_ts.rs:2913-2915` |
| **Closures / lambdas / function expressions** | **NO** | `ArrowFunctionExpression`, `FunctionExpression`, `ClassExpression` all → `push_unsupported(.., "function expression"/"class expression", .., HavocAffectedPlaces)` at `lower_ts.rs:1696`, `:1707`, `:1718`. No capture list, no environment |
| **`async` / `await` / generators / `yield`** | **NO** | `await` at `lower_ts.rs:1613`, `yield` at `:1651` → `Unsupported` + havoc. The only trace of async in the whole IR is `PlaceProjection::AwaitResult` (`places.rs:56`) — a projection, not an effect |
| **Concurrency** — goroutines, channels, `defer` | **NO** | `"go_statement"`, `"defer_statement"`, `"select_statement"`, `"send_statement"`, `"channel_receive"` → `Unsupported` (`lower_go.rs:1276-1284`). Go's three signature constructs are all outside the IR |
| **Destructuring / spread / rest** | **NO** | `lower_ts.rs:2918-2920` |
| **Getters / setters / private fields** | **NO** | `lower_ts.rs:2921-2925` |
| **Generics** | **NO** | nothing in the IR; nothing reaches the type model either (see (e)) |
| **Optional chaining, dynamic property keys, `eval`, `with`, `Proxy`, dynamic `require`** | **NO** | `lower_ts.rs:2896-2900` (max blast radius: 5 domains) |

### The load-bearing consequence

Because MIR has neither blocks nor terminators, the CFG cannot be built from it
structurally. `analysis/cfg/lower_{go,ts}.rs` (1,629 LOC) therefore reconstruct
control-flow shape by **string-matching the `construct` label and
`source_evidence` text of the `Unsupported` rows**:

```rust
// analysis/cfg/lower_ts.rs:249-254
"for ", "while", "do while", "for-of", "for-in", "for await",
// :261
} else if contains_token(&evidence, &["&&", "||", "ShortCircuit", "logical"]) {
// :482
fn contains_token(evidence: &str, tokens: &[&str]) -> bool {
    tokens.iter().any(|token| evidence.contains(token))
}
```

`analysis/cfg/lower_go.rs:247` does the same with `["for ", "range ", "for_statement"]`.

This is the strongest single piece of evidence in the audit. A CFG builder that
greps source text to find a loop is not consuming an IR; it is doing a second,
lossier parse. And it means **item 3 in table (b) — a per-language CFG lowerer —
is pure IR-design debt**: with real terminators, CFG construction would be one
shared 300-LOC function forever.

### Would Python / Java / C++ fit?

| Language | Verdict |
|---|---|
| **Python** | Frontend fits (it is dynamic like JS, and the `Unknown { evidence }` discipline suits it). But `try/except/finally` is idiomatic and pervasive, decorators are function-valued (needs closures), `with` blocks need unwind edges, generators need suspension — **all four are `Unsupported` today**. A Python MIR would be ~60% `Unsupported` rows. |
| **Java** | **Breaks.** Checked exceptions are the type system; `try`/`catch`/`finally` cannot be `Unsupported` and produce a useful analysis. Generics erasure, interfaces, and virtual dispatch all need a type model the IR does not have. `switch` unsupported is fatal. |
| **C/C++** | **Breaks hard.** No pointer arithmetic (`PlaceProjection` has `Deref` but no offset arithmetic and no operators at all), no unions, no stack-vs-heap distinction, no lifetime, no `goto` (already `Unsupported` for Go), no preprocessor model. |
| **Rust** | Would want to consume `rustc` MIR directly, which is far richer than this one — the impedance mismatch runs the wrong direction. |
| **Kotlin / Swift / C#** | Same failures as Java: exceptions, generics, `async`, and closures are all first-class in the language and all `Unsupported` in the IR. |

**The IR models the intersection of "straight-line Go" and "straight-line JS." Every
language on the target list needs strictly more than that intersection.**

### To be fair to the design

Two things in this IR are better than most:

1. **`unsupported-semantic fact record`** (`op.rs:79`) is an excellent idea, well executed.
   Recording `construct`, `source_evidence`, `affected_places`, `affected_domains`,
   `conservative_action`, and `precision` for every unmodelled construct — and
   requiring completeness (`is_complete`, `op.rs:97`) — means the engine's
   uncertainty is auditable rather than silent. Very few analysis engines do this.
   **Keep it verbatim in any redesign.**
2. **`analysis/mir/body.rs:244` `mir_contract_source_does_not_store_parser_ast_objects`**
   is a compile-time-adjacent guardrail asserting no `tree_sitter::Node`, `oxc_ast`,
   or `oxc_span::Span` type ever appears in the IR contract. That discipline is
   why the IR *could* become real. Keep it and extend it.

The problem is not that the design is wrong. It is that the vocabulary was frozen
at 9 operations and never grew, so every construct beyond straight-line code
became an `Unsupported` row, and the honesty machinery became the main output.

---

## (d) Language leakage, quantified

### Headline numbers

| Measure | Value |
|---|---|
| `crates/polint/src` total | **253,559 LOC** |
| …excluding `eval/` (29,344) | **224,215 LOC** |
| **TS/JS-specific** (`ts/`, `*_ts.rs`, `ts.rs`, `ts_js.rs`, `ts_value_flows.rs`, `ts_object_model/`, `ts_tokens/`, `js_points_to/`, JS formats) | **48,374 LOC** |
| **Go-specific** (`go/`, `*_go.rs`, `go.rs`, `go_rta/`, `go_mod.rs`, `go_work.rs`, `go_relstring.rs`) | **23,500 LOC** |
| Go sidecars (Go source, non-test) | **+2,632 LOC** (`polint-go-frontend` 1,352 · `polint-go-symbols` 1,597 prod, one a dead duplicate) |
| **Per-language total** | **~71,900 LOC = 32% of the non-eval crate** |

### Files naming a language

| Directory | Files naming a language / total |
|---|---|
| `analysis/` | 133 / 218 (**61%**) |
| `analysis_kernel/` | 15 / 21 (**71%**) |
| `symbol_graph/` | 7 / 7 (**100%**) |
| `module_graph/` | 8 / 14 (**57%**) |
| `sdk/` | 2 / 4 |
| `core/` | 1 / 1 |
| `runner/` | **0 / 1** — the one clean layer |

Twenty-three files under the "generic" layers import a frontend module directly
(`rg -l 'crate::ts::|crate::go::' analysis/ analysis_kernel/ symbol_graph/ module_graph/`).
Worst offender: `analysis/semantic_graph/build.rs` (2,955 LOC, nominally the
language-neutral semantic graph builder) with **21 direct `crate::ts::` /
`crate::go::` imports** at `:39-54`.

### The `_ts` / `_go` duplication tax, file by file

| Concern | Go file (LOC) | TS file (LOC) | Measured overlap |
|---|---|---|---|
| MIR lowering | `analysis/mir/lower_go.rs` 1,913 | `analysis/mir/lower_ts.rs` 3,952 | structural mirror |
| CFG lowering | `analysis/cfg/lower_go.rs` 802 | `analysis/cfg/lower_ts.rs` 827 | structural mirror; both grep evidence strings |
| Type derivation | `analysis/types/go.rs` 848 | `analysis/types/ts_js.rs` 1,264 | `type_fact_for_place` (`go.rs:115` / `ts_js.rs:162`) and `type_shape_for_place` (`go.rs:183` / `ts_js.rs:226`) differ only by a `reason:` string prefix and one `Global` arm — **near-total clone** |
| Refined calls | `analysis/refined_calls/go.rs` 636 | `analysis/refined_calls/ts_js.rs` 696 | **135 of go.rs's 220 unique prod lines (61%) appear verbatim in ts_js.rs** |
| Entrypoints | `analysis/entrypoints/recognizers_go.rs` 1,498 | `analysis/entrypoints/recognizers_ts.rs` 2,397 | **313 of 468 unique Go prod lines (67%) verbatim in the TS file**; 15 helper fns identical under identical names |
| Symbol graph | `symbol_graph/go.rs` 2,587 | `symbol_graph/ts.rs` 3,067 | **13 near-parallel stable-key fns**, 5 enum mappers, 3 draft builders; ~700–900 LOC structurally parallel |
| Module graph | `module_graph/go.rs` 2,261 | `module_graph/ts.rs` 3,209 | separate resolvers; ~65–70% of the subsystem is per-language |
| Adapter | `go/adapter.rs` 1,673 | `ts/adapter.rs` 4,348 | 9 private helpers duplicated verbatim |
| Solver inputs | `solver/go_rta/` 3,739 | `solver/ts_object_model/` 2,014 + `ts_tokens/` 1,332 | legitimately different algorithms |

**Conservative estimate of pure copy-paste already in the tree: ~2,000–2,500 LOC.**
That is the amount a third language would triplicate rather than double.

### Parsing is duplicated too

There is **no shared AST cache**. Each provider re-parses from source:

- **Oxc `Parser::new` — 11 production sites**: `ts/inventory/extract.rs:25`,
  `ts/object_model/extract.rs:31`, `ts/scope/extract.rs:30`, `ts/adapter.rs:486`,
  `symbol_graph/ts.rs:162`, `analysis/semantic_graph/build.rs:235`, `:284`, `:952`,
  `analysis/calls/ts_value_flows.rs:64`, `:324`, `:396`, `analysis/mir/lower_ts.rs:80`,
  `analysis/calls/js_points_to/provider.rs:61`.
- **`oxc_semantic::SemanticBuilder` — 5+ sites**, each rebuilding scope/binding
  resolution from scratch after its own re-parse.
- **tree-sitter — 2 sites**: `go/adapter.rs:452`, `analysis/mir/lower_go.rs:63`.
- **`oxc_resolver` — 2 independent `Resolver` instances**: `module_graph/ts.rs:28`
  and `analysis/calls/ts_value_flows.rs:243`, duplicating module resolution.

A typical `.ts` file is parsed on the order of **ten times per run**. This is both
a per-language tax (every new frontend re-implements its own parse-and-cache
plumbing) and a straightforward multiple-x performance cost, and it is the most
directly actionable finding in this document.

### Two dead/duplicated sidecars

`tools/polint-go-symbols/internal/symbols/emit.go` (1,527 LOC) is byte-identical
to `crates/polint/go-sidecar/polint-go-symbols/internal/symbols/emit.go`. Only the
latter is `include_str!`-embedded (`symbol_graph/go.rs:32-44`). The `tools/` copy
is referenced by nothing outside itself. Delete it.

---

## (e) Target architecture: making language N+1 cost O(small)

The goal is to move from **~17k–30k LOC per language** to **~2k–4k LOC per
language**. Five changes, in dependency order. Each is independently shippable.

### E1. Make the IR real — blocks, terminators, operators, effects

**This is the keystone. Nothing else compounds without it.**

Grow `MIR-operation kind` and add the missing structural layer:

```rust
// New: the structural layer that is currently absent entirely.
struct MirBlock { id: BlockId, body: MIR-body id, ordinal: u32,
                  statements: Vec<MirOpId>, terminator: MirTerminatorId }

enum MirTerminatorKind {
    Goto      { target: BlockId },
    Branch    { predicate: MirPredicateId, then_: BlockId, else_: BlockId },
    Switch    { scrutinee: PlaceId, arms: Vec<(MirValue, BlockId)>, default: BlockId },
    Return    { value: Option<MirValue> },
    // The exception/effect layer that today does not exist at all:
    Throw     { value: MirValue, unwind: Option<BlockId> },
    Call      { site: CallSiteId, normal: BlockId, unwind: Option<BlockId> },
    Suspend   { kind: SuspendKind /* Await | Yield | ChannelRecv | ChannelSend */,
                value: Option<MirValue>, resume: BlockId },
    Unreachable,
    Unsupported { unsupported: UnsupportedId, successors: Vec<BlockId> },
}

// New operations:
BinOp    { place: PlaceId, op: BinOpKind, lhs: MirValue, rhs: MirValue }
UnOp     { place: PlaceId, op: UnOpKind, operand: MirValue }
Aggregate{ place: PlaceId, kind: AggregateKind /* Struct|Object|Array|Tuple|Map */,
           fields: Vec<(FieldKey, MirValue)>, alloc: AllocationTokenId }
Closure  { place: PlaceId, body: MIR-body id, captures: Vec<(PlaceId, CaptureMode)> }
```

Why each earns its place:

- **Blocks + terminators** delete the two per-language CFG lowerers (1,629 LOC)
  and the substring-matching in `cfg/lower_ts.rs:249`, permanently. Item 3 in
  table (b) drops to zero for every future language.
- **`Throw` / `Call { unwind }`** is the single highest-value addition: it is the
  difference between "Java/Python/C#/Kotlin/Swift are analysable" and "they are not."
- **`Suspend`** unifies `await`, `yield`, and Go channel operations under one
  effect concept, replacing three separate `Unsupported` families.
- **`Closure` with an explicit capture list** is what would let `ts_value_flows.rs`
  (11,898 LOC) fold back into the shared pipeline. It is why that file forked.
- **`BinOp` / `Aggregate`** unlock constant propagation, string-concat taint, and
  object-literal points-to without per-language hacks.

Keep `unsupported-semantic fact record` exactly as it is. Keep the
`mir_contract_source_does_not_store_parser_ast_objects` guard
(`body.rs:244`) and extend it to every new IR file.

Add types to places: `place-fact record.ty: Option<TypeSetId>`, referencing the existing
`analysis/types` lattice. Today `place-fact record` (`places.rs:11`) has no type field at
all, which is why the type layer is a separate, near-duplicated pair of files.

Target ratio after this work: **~3,000 LOC of shared IR to ~1,500 LOC per
frontend** — i.e. invert the current 1:6.6 to roughly 2:1.

### E2. Introduce a real `LanguageFrontend` trait and a registry

Replace the twelve hand-written fan-outs with one:

```rust
pub(crate) trait LanguageFrontend: Send + Sync {
    fn id(&self) -> &'static str;                    // "go", "ts", "python"
    fn languages(&self) -> &'static [Language];
    fn schema_version(&self) -> &'static str;

    /// Parse once. The returned handle is cached and shared by every consumer.
    fn parse(&self, file: &SourceFile) -> Result<ParsedUnit, ParseError>;

    fn lower_mir(&self, unit: &ParsedUnit, cx: &mut LoweringCx) -> MirOutput;
    fn extract_symbols(&self, unit: &ParsedUnit, cx: &mut SymbolCx) -> SymbolOutput;
    fn resolve_import(&self, input: ResolverInput<'_>) -> ResolvedImportDraft;
    fn collect_topology(&self, cx: &TopologyCx) -> TopologyOutput;

    // Optional, with neutral defaults:
    fn entrypoint_specs(&self) -> &[FrameworkSpec] { &[] }   // see E4
    fn solver_policy(&self, db: &AnalysisDb) -> Option<Box<dyn SolverPolicy>> { None }
    fn type_provider(&self) -> Option<&dyn TypeProvider> { None }
}
```

Then `analysis_kernel/mod.rs:191-225` becomes a loop over
`Vec<Box<dyn LanguageFrontend>>`, and `PROVIDER_MANIFESTS` gains one
programmatically-generated `polint.<id>.syntax` entry per registered frontend
instead of hand-maintained duplicate order lists at `:940` and `:972`.

`ResolverInput` / `ResolvedImportDraft` (`module_graph/model.rs:62-78`) and the
`SymbolDraft` family (`symbol_graph/model.rs:37-100`) already have the right
shape — they are the existing proof that this factoring works. Promote them into
the trait.

### E3. Parse once, share the AST

Add a per-run `ParsedUnitCache: FileId -> Arc<ParsedUnit>` owned by the kernel.
This alone:

- collapses **11 Oxc parse sites + 5 `SemanticBuilder` builds + 2 tree-sitter
  parses** to one each per file;
- removes the second `oxc_resolver::Resolver` at `ts_value_flows.rs:243`;
- deletes the nine duplicated cache-layer helpers from each adapter (item 1 in
  table (b) drops by roughly 40%);
- is a large, unambiguous performance win independent of any language work.

Lifetime note: Oxc requires an `oxc_allocator::Allocator` to outlive the AST, so
`ParsedUnit` must own its arena (`self_cell` or an owning-arena wrapper). This is
the one non-trivial engineering problem in E3 and should be spiked first.

### E4. Make framework recognition data-driven

Today: `enum TsFramework { Express, McpSdk, Jest, Vitest, Mocha, Commander, Yargs }`
(`recognizers_ts.rs:32`) and `enum GoFramework { NetHttp, Chi, Cobra, Testing }`
(`recognizers_go.rs:30`) — **11 frameworks, all hand-coded**, with a *second*
hardcoded substring list of frameworks that are recognised only well enough to
emit "unsupported" (`recognizers_go.rs:50-64` for gin/echo/fiber/gorilla,
`recognizers_ts.rs:60-87` for fastify/koa/hapi/nest/next/nuxt/remix/sveltekit/astro).

Replace with a declarative spec the frontend returns from `entrypoint_specs()`:

```rust
struct FrameworkSpec {
    id: &'static str,               // "flask"
    import_match: ImportMatch,      // Exact("flask") | Prefix("@nestjs/") | ModulePath(..)
    patterns: &'static [EntrypointPattern],
}
struct EntrypointPattern {
    receiver: ReceiverMatch,        // decorator | method-on-import | free call
    method: MethodMatch,            // one of ["get","post",..] | any
    handler_arg: ArgSelector,       // positional(1) | last | decorated-fn
    kind: EntrypointKind,           // HttpRoute | CliCommand | Test | Job | Serverless
}
```

The `EntrypointKind` / `DispatchEdgeKind` enums (`entrypoints/dispatch.rs:64`)
are already generic enough. This turns "add a framework" from ~150 LOC of Rust
into ~10 lines of data, in any language, and eliminates the ~313 verbatim-copied
lines between the two recognizer files.

### E5. Extract the shared per-language boilerplate before writing a third copy

Cheap, do-now items that stop triplication:

| Extraction | Saves per new language |
|---|---|
| `entrypoints/recognizers_common.rs` — the 15 identical helpers (`resolve_handler_function`, `extract_first_arg_literal`, `handler_argument_names`, `find_function_by_name`, `single_call_argument`, `split_top_level_arguments`, `unquote_literal`, …) | ~300 LOC |
| `refined_calls/common.rs` — `TargetRefinement`, `edge_from_target`, `points_to_sets_for_place`, `type_precision`, `confidence_for_status`, `metadata_key` (also fixes a real inconsistency: `ts_js.rs` has an overflow guard in `points_to_sets_for_place` that `go.rs` lacks) | ~135 LOC |
| `symbol_graph/keys.rs` — collapse the 13 near-parallel `{go,ts}_*_stable_key` fns into one parameterised builder; likewise the 5 enum mappers and 3 draft builders | ~700–900 LOC |
| `types/derive_common.rs` — `type_fact_for_place` / `type_shape_for_place` are ~clones differing by a string prefix | ~600 LOC |
| Adapter cache-layer helpers (the 9 duplicated private fns) — subsumed by E3 | ~400 LOC |
| Consolidate 6 independent `language_label` maps and 3 FNV-1a implementations (`cache/mod.rs:773`, `symbol_graph/stable_id.rs:219`, `analysis/identity/facts.rs:226`) and 3 length-prefixed key encoders | ~150 LOC + correctness risk |
| Delete the dead duplicate sidecar `tools/polint-go-symbols/` | 2,225 LOC removed |

**Total: ~2,300–2,500 LOC saved per new language, available before any IR work.**

### Also fix: identity, so the answer is cross-repo and cross-language

Not strictly a frontend concern, but it caps what any number of frontends can
deliver, so it belongs in the target architecture.

Today (`analysis/identity/facts.rs`, `symbol_graph/stable_id.rs`):

- **Not stable across edits.** Every `stable_key` embeds byte offsets —
  `compute_identity_stable_key` emits `identity|{kind}|{lang}|{pkg}|{container}|{file_id}|{start}..{end}`
  (`facts.rs:164-182`); `StableSymbolKey` embeds `span_part` = `start-end:line:col-line:col`
  (`stable_id.rs:86`, `:266`). Tests *lock this in as intended*
  (`stable_id.rs:391`, `:403`). Insert a line at the top of a file and every
  symbol below it gets a new ID — which also defeats incremental caching.
- **Not cross-repo.** No repo ID, commit SHA, or package version in any key.
- **Not cross-language, by explicit design.** `LanguageTag` is the first field of
  every key (`stable_id.rs:78`, `facts.rs:152`), and the eval suite *asserts* the
  absence of cross-language edges: `assert_ne!(node_language(other), Some(Language::Go),
  "no derived edge may cross the Go<->TS boundary")` at `eval/go_rta.rs:704-718`,
  duplicated at `eval/ts_object_model.rs:466` and `eval/ts_tokens.rs:239`.
  Zero protobuf/gRPC/OpenAPI support anywhere in the crate.
- **No SCIP/LSIF/moniker concept.** Zero hits crate-wide. `IdentityKind` is closed
  at `Function | Callsite` (`facts.rs:18`). The two "renderers"
  (`render/go_relstring.rs`, `render/jelly_span.rs`) exist only to match external
  benchmark output formats and reverse-engineer Go shape by string-sniffing
  `container_path` (`go_relstring.rs:80-127`).

What good looks like: adopt **SCIP symbol syntax** as the canonical identity —
`scheme manager package-name version descriptor…`. It is language-neutral by
construction, cross-repo by construction (package + version are in the string),
edit-stable (descriptors are names and scopes, never offsets), and it is already
what the wider tooling ecosystem speaks. Keep byte spans as a *separate*
`location` field on the fact, never inside the key. The existing
`SignatureDigest` (`facts.rs:143`) is the right *idea* — location-free semantic
identity — but it is fed `None` for both `parameter_shape` and `return_shape` by
the live provider (`identity/provider.rs:150-151`), so overloads are invisible to
it. Fix that as part of the same change.

Once identities are SCIP-shaped, cross-language edges (TS `fetch('/api/users')` →
Go `mux.HandleFunc("/api/users")`, or either side of a protobuf service) become
expressible for the first time. That is a category of rule no competitor ships,
and it is currently blocked by a 20-line key-format decision.

### Sequencing

| Order | Work | Unblocks |
|---|---|---|
| 0 | **E5** extractions + delete dead sidecar | Immediate; stops triplication |
| 1 | **E3** parse-once cache | Large perf win; shrinks every frontend |
| 2 | **E1** real IR (blocks, terminators, `Throw`, `Suspend`, `Closure`, `BinOp`, `Aggregate`, types on places) | Deletes per-language CFG; makes Java/Python/C# *possible* |
| 3 | **E2** `LanguageFrontend` trait + registry | Collapses 12 fan-outs to 1 |
| 4 | Identity → SCIP-shaped | Cross-repo, cross-language, edit-stable, better incremental caching |
| 5 | **E4** declarative framework specs | Frameworks become data |
| 6 | *Then* Python, as the proof that N+1 is cheap | — |

Doing Python **before** steps 0–3 would be the expensive mistake: it would bake a
third copy of every duplicated concern into the tree and roughly double the cost
of the refactor that follows.

---

## (f) tree-sitter vs native frontends vs external indexers

### What the code does today, and why it is not a principle

| Language | Parser | Rationale in code |
|---|---|---|
| Go | `tree-sitter` 0.26.8 + `tree-sitter-go` 0.25.0, 2 sites | historical; first language |
| TS/JS | Oxc 0.129.0 (`parser`, `ast`, `semantic`, `span`) + `oxc_resolver` 11.19.1, 13 sites | speed, and `oxc_semantic` gives real scope/binding resolution |
| Go *deep semantics* | **out-of-process Go sidecar** — `crates/polint/go-sidecar/polint-go-frontend` (1,352 prod LOC of Go), NDJSON over stdout, schema `polint-go-semantic-3` (`go/semantic/protocol.rs:4`), driven by `go/semantic/{client,process}.rs` | `go/packages` + `go/ssa` with `ssa.InstantiateGenerics` — real types, real SSA |

There is no stated rule. The pattern is: *use whatever was convenient, then bolt
on a sidecar when precision demanded it.* And note the asymmetry — Go got a
sidecar and reached `x/tools`-grade call-graph precision; **TS never did**, and
that is the single largest capability gap in the engine.

### The TS type-information gap, stated plainly

There is **no tsc, no tsserver, no typescript-go, no `.d.ts` consumption**
anywhere in the crate (verified: zero hits for `tsserver`, `"tsc"`,
`TypeChecker`, `type_checker`). `tsconfig.json` appears exactly once, at
`analysis_kernel/incremental/keys.rs:35`, used as a **cache-invalidation key** and
never parsed for `paths` or `compilerOptions`.

Consequently the TS "type inference" is substring matching on source text.
`analysis/types/ts_js.rs:819` `narrowing_shape`:

```rust
if trimmed.contains("typeof")     { TypeShape::Primitive("typeof-refinement") }
if trimmed.contains("instanceof") { TypeShape::Nominal { type_id: "instanceof-refinement" } }
if trimmed.contains(" in ") || trimmed.contains(".hasOwnProperty") { … Object … }
```

paired with `evidence_mentions_place` (`ts_js.rs:803-817`), which attributes a
narrowing to a variable **if the evidence string contains the variable's name** —
so a comment mentioning `user`, or a different in-scope `user`, matches.

To the team's credit these are labelled `TypePrecision::Heuristic`
(`ts_js.rs:770`, `:788`), and they are the *only* two sites in the entire crate
that ever write `TypeStatus::Present`. Everything else is explicitly `Unknown`.
The engine does not lie about what it knows. But the honest summary is:
**`TypeShape` (`analysis/types/facts.rs:57`, 15 variants) is a schema, not a
lattice.** `GenericPlaceholder` is constructed **nowhere**. `Union`/`Intersection`
are constructed **only in tests**. `Structural` is reachable only via the
extension string-parser at `types/validate.rs:508`. There is no join, no meet, no
subtype check. A rule author cannot ask "is this a `string`?" in either language.

And even Go's genuine type information never reaches the type model: `analysis/types/go.rs`
contains **zero references to `go_semantic_*`** — the sidecar's `go/types` data
flows only to `refined_calls/provider.rs:255`, `solver/go_rta/inputs.rs:153-296`,
and `semantic_graph/provider.rs:571-576`. Type-directed precision exists, but
only inside the call graph.

### Recommendation

**A three-tier frontend strategy, with an explicit promotion rule.**

#### Tier 1 — tree-sitter baseline. *Mandatory for every language.*

Every language gets a tree-sitter grammar and a lowering to the (post-E1) MIR.
Delivers: functions, classes, imports, literals, CFG, syntactic call graph,
metrics, and the `Unsupported` ledger. Error-tolerant, so it works on broken and
partial code — which matters for `polint review` on an in-progress diff.

Cost per language after E1–E5: **~2,000–3,500 LOC.** This is the tier that makes
"polint supports 12 languages" true, and it is the right default for Ruby, PHP,
Kotlin, Swift, and C/C++ headers.

Precision ceiling: `ResolutionPrecision::Syntactic`. Declare it and mean it.

#### Tier 2 — external indexer ingestion. *The highest-leverage unbuilt capability.*

**Consume SCIP.** `scip-typescript`, `scip-java`, `scip-python`, `scip-go`,
`scip-ruby`, `rust-analyzer` (LSIF/SCIP), and `clangd` (with `scip-clang`) already
exist, are maintained by others, and are built on each language's real compiler
front end. A single SCIP ingester — **one implementation, ~1,500–2,500 LOC total,
not per language** — would give polint:

- compiler-grade symbol identity, definitions, references, and hover types
- for **every language with a SCIP indexer**, at once
- cross-repo identity for free, because SCIP symbols carry package + version

This is strictly better leverage than writing a fourth, fifth, and sixth native
frontend. It is also the natural forcing function for the identity rework in (e):
adopting SCIP symbol syntax as polint's canonical identity makes ingestion nearly
mechanical.

The trade-off is real and should be stated: SCIP requires a *build*. It is slow,
needs the toolchain and dependencies present, and does not work on
partially-broken code. So it must be **optional and additive** — Tier 1 always
runs; Tier 2 overlays higher-precision facts when an index is available. The
existing `TypePhase::SetupMissing` (`types/facts.rs:76`) and the
`CapabilitySupportStatus::SetupMissing` machinery are already the correct plumbing
for exactly this.

Prefer SCIP over LSIF (LSIF is deprecated by Sourcegraph) and over live LSP (LSP
is stateful, per-file, latency-bound, and a poor fit for whole-repo batch
analysis).

#### Tier 3 — native high-fidelity frontends. *Rare. Requires a written business case.*

Reserve for languages where (i) polint's differentiating rules demand semantics no
indexer exposes, and (ii) volume justifies the cost. Oxc for TS/JS is a defensible
Tier 3 — polint needs JS object-model and prototype semantics that no indexer
surfaces. The Go sidecar is another: `go/ssa` gives RTA-quality dispatch that SCIP
does not carry.

**Explicit promotion rule** (write this into `AGENTS.md`, so it is a decision and
not a drift):

> A language is promoted from Tier 1 to Tier 3 only when a *named, shipped* rule
> requires semantics that neither tree-sitter nor the language's SCIP indexer can
> supply, and that requirement is documented with the specific facts needed. Type
> information alone is never sufficient justification — that is Tier 2's job.

#### Two specific near-term actions

1. **Close the TS type gap via Tier 2, not Tier 3.** Ship a `scip-typescript`
   ingester before writing any more of `analysis/types/ts_js.rs`. It replaces
   ~1,264 LOC of substring heuristics with real checker output, and the same
   ingester immediately serves Python (`scip-python`), Java (`scip-java`), and
   Ruby (`scip-ruby`). This is the highest ratio of capability gained to code
   written available anywhere in this audit.
2. **Feed the Go sidecar's type strings into `analysis/types/go.rs`.** The data is
   already crossing the process boundary and being thrown away by the type layer.
   Small change, immediate precision gain, and it validates the ingestion shape
   that Tier 2 will generalise.

---

## Appendix: the six most load-bearing citations

| Claim | Citation |
|---|---|
| No language adapter trait exists | `rg '^\s*(pub.*)?trait \w+' crates/polint/src` → 17 traits, none language-related; `analysis/solver/policy.rs:77`, `sdk/facts.rs:1057`, `analysis/domains/lattice.rs:71`, `analysis/summaries/domain.rs:58`, `eval/adapter.rs:10` are the notable ones |
| Kernel dispatch is hand-written | `analysis_kernel/mod.rs:191`, `:209`, `:424` |
| MIR has no blocks or terminators | `analysis/mir/body.rs:66` (`MirOutput` fields); `MirStatement` `:32` and `MirTerminator` `:49` constructed nowhere |
| CFG recovers loops by grepping source evidence | `analysis/cfg/lower_ts.rs:249-254`, `:482`; `analysis/cfg/lower_go.rs:247` |
| TS call graph forked around the MIR | `analysis/calls/ts_value_flows.rs` 11,898 LOC, `oxc_parser::Parser` at `:64`, `:324`, `:396`; own `oxc_resolver::Resolver` at `:243` |
| TS "type inference" is substring matching | `analysis/types/ts_js.rs:819` `narrowing_shape`, `:803` `evidence_mentions_place` |
