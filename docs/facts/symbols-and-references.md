# Symbols And References

`Symbols<'_>` and `References<'_>` expose stable symbol identities,
definitions, and references to repo-local rules. Rules request these views as
typed parameters in a `#[polint::rule]` function, and polint derives the
`symbols` and `references` capabilities from those parameter types.

Start rules with:

```rust
use polint::sdk::prelude::*;
```

`References<'_>` implies symbol identity internally. A rule can request only
`References<'_>` when it does not need to inspect symbols directly; polint still
derives the symbol identities needed to bind resolved references.

## Example

```rust
use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/no-forbidden-api",
    description = "Flag calls to a forbidden API.",
    severity = "warn"
)]
fn no_forbidden_api(
    ctx: &mut RuleCtx<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
) -> RuleResult {
    for symbol in symbols.by_name("forbiddenApi") {
        for reference in references.to(symbol.id) {
            let Some(file) = reference.file else {
                continue;
            };
            ctx.report(
                Diagnostic::warning(
                    ctx.rule_id(),
                    ctx.file_path(file),
                    reference
                        .primary_span
                        .as_ref()
                        .map(Span::diagnostic_range)
                        .unwrap_or_else(|| DiagnosticRange::point(1, 1)),
                    "Do not call forbiddenApi here.",
                )
                .with_evidence("symbol_id", symbol.id.0.to_string())
                .with_evidence("reference_id", reference.id.0.to_string())
                .with_evidence("precision", format!("{:?}", reference.precision))
                .with_evidence("status", format!("{:?}", reference.status)),
            );
        }
    }

    for reference in references.unresolved() {
        if reference.name == "forbiddenApi" {
            // Unresolved references are visible facts. Decide whether your
            // policy should warn, ignore, or produce a lower-severity finding.
        }
    }

    Ok(())
}
```

## Symbols<'_>

`Symbols<'_>` exposes symbol identities and their definitions. There is no
separate `Definitions<'_>` view; definitions are queried through `Symbols`.

Query methods:

| Method | Meaning |
|--------|---------|
| `all()` | Returns all symbol facts in deterministic database order. |
| `iter()` | Iterates all symbol facts. |
| `get(symbol)` | Returns one `SymbolFact` by stable `SymbolId`. |
| `stable_key(symbol)` | Resolves debug-safe stable identity text for a symbol. |
| `definition_stable_key(definition)` | Resolves debug-safe stable identity text for a definition. |
| `for_file(file)` | Iterates symbols owned by one source file. |
| `by_name(name)` | Iterates symbols with the exact public name. |
| `by_kind(kind)` | Iterates symbols with a specific `SymbolKind`. |
| `exported_by_name(name)` | Iterates exported symbols with the exact public name. |
| `definitions_in_file(file)` | Iterates definitions whose primary location is in one file. |
| `definition(symbol)` | Returns the primary definition for a symbol, if known. |
| `definitions(symbol)` | Iterates all definitions for a symbol. |
| `exported()` | Iterates symbols marked exported by the provider. |

`SymbolFact` fields:

| Field | Meaning |
|-------|---------|
| `id` | Stable polint-owned `SymbolId`. |
| `language` | Language family that produced the symbol. |
| `name` | Short symbol name. |
| `qualified_name` | Provider-normalized qualified name. |
| `kind` | Normalized `SymbolKind`. |
| `namespace` | `SymbolNamespace` for value, type, namespace, package, module, or unknown identities. |
| `file` | Source `FileId` when the symbol belongs to one file. |
| `package` | Source `PackageId` when package ownership is known. |
| `module` | Source `ModuleNodeId` when module ownership is known. |
| `owner` | Owning `SymbolId` when known. |
| `primary_span` | Primary source span when the symbol has one. |
| `is_exported` | Whether the provider considers the symbol exported. |
| `precision` | `SymbolPrecision` for the symbol identity. |

`DefinitionFact` records where a symbol is declared or defined. Definitions are
not references. Multiple definitions can exist, for example TypeScript
declaration merging.

`DefinitionFact` fields:

| Field | Meaning |
|-------|---------|
| `id` | Stable polint-owned `DefinitionId`. |
| `symbol` | `SymbolId` this definition belongs to. |
| `language` | Language family that produced the definition. |
| `name` | Short definition name. |
| `qualified_name` | Provider-normalized qualified name. |
| `kind` | Normalized `DefinitionKind`. |
| `namespace` | `SymbolNamespace` for this definition. |
| `file` | Source `FileId` when known. |
| `package` | Source `PackageId` when package ownership is known. |
| `module` | Source `ModuleNodeId` when module ownership is known. |
| `owner` | Owning `SymbolId` when known. |
| `primary_span` | Source span for this definition when available. |
| `is_primary` | Whether this is the provider's primary definition for the symbol. |
| `is_exported` | Whether the definition contributes to an exported symbol. |
| `precision` | `SymbolPrecision` for the definition. |

## References<'_>

`References<'_>` exposes uses of symbol-like names. It includes resolved,
unresolved, ambiguous, setup-missing, and unsupported reference states when a
provider can represent them honestly.

Query methods:

| Method | Meaning |
|--------|---------|
| `all()` | Returns all reference facts in deterministic database order. |
| `iter()` | Iterates all reference facts. |
| `stable_key(reference)` | Resolves debug-safe stable identity text for a reference. |
| `to(symbol)` | Iterates resolved references to one `SymbolId`. |
| `for_file(file)` | Iterates references in one source file. |
| `resolved()` | Iterates references with `Resolved` status. |
| `by_name(name)` | Iterates references with the exact public name. |
| `to_any(symbols)` | Iterates references resolved to any symbol yielded by a symbol iterator. |
| `unresolved()` | Iterates references with `Unresolved` status. |
| `unresolved_by_name(name)` | Iterates unresolved references with the exact public name. |
| `ambiguous()` | Iterates references with `Ambiguous` status. |

`ReferenceFact` fields:

| Field | Meaning |
|-------|---------|
| `id` | Stable polint-owned `ReferenceId`. |
| `language` | Language family that produced the reference. |
| `name` | Short referenced name. |
| `qualified_name` | Provider-normalized qualified name when available. |
| `kind` | Normalized `ReferenceKind`. |
| `namespace` | Namespace the reference was interpreted in. |
| `file` | Source `FileId` when known. |
| `package` | Source `PackageId` when package ownership is known. |
| `module` | Source `ModuleNodeId` when module ownership is known. |
| `owner` | Owning `SymbolId` when known. |
| `primary_span` | Source span for the reference when available. |
| `target` | Resolved target `SymbolId`, when exact enough. |
| `candidates` | Candidate `SymbolId` values for ambiguous references. |
| `status` | `SymbolResolutionStatus` for the reference. |
| `precision` | `SymbolPrecision` for the binding. |

## IDs, Kinds, And Namespaces

`SymbolId`, `DefinitionId`, and `ReferenceId` are polint-owned stable IDs. They
are derived from normalized semantic key material, not vector positions or raw
parser IDs. IDs are intended to stay stable across repeated cached checks for
unchanged source and setup inputs. They can still change when the provider's
semantic identity changes, such as moving a local-only symbol whose stable key
includes a file path or source span.

`SymbolKind` variants:

- `Package`
- `Module`
- `File`
- `Function`
- `Method`
- `Class`
- `Interface`
- `TypeAlias`
- `Enum`
- `EnumMember`
- `Variable`
- `Constant`
- `Parameter`
- `Field`
- `Property`
- `Namespace`
- `Import`
- `Export`
- `Unknown`

`DefinitionKind` variants:

- `Declaration`
- `Definition`
- `Import`
- `Export`
- `Implicit`
- `Unknown`

`ReferenceKind` variants:

- `Read`
- `Write`
- `ReadWrite`
- `Call`
- `TypeUse`
- `Import`
- `Export`
- `MemberAccess`
- `Assignment`
- `DeclarationUse`
- `Unknown`

`SymbolNamespace` variants:

- `Value`
- `Type`
- `Namespace`
- `Package`
- `Module`
- `Unknown`

## Status And Precision

`SymbolPrecision` describes how precise the identity or binding is:

| Precision | Meaning |
|-----------|---------|
| `ExactSemantic` | Derived from language semantic information, such as Go typed package data. |
| `ExactLocal` | Exact within one parsed file or lexical scope. |
| `ModuleLinked` | Linked across modules through resolved import/module graph facts. |
| `Heuristic` | Best-effort result from a heuristic provider. Rules must treat this as approximate. |
| `Unresolved` | The provider saw the reference but did not resolve a target. |
| `Ambiguous` | The provider found multiple possible target symbols. |
| `SetupMissing` | Required language or repository setup was missing. |
| `Unsupported` | The language or reference form is known but not implemented. |

`SymbolResolutionStatus` describes reference resolution:

| Status | Meaning |
|--------|---------|
| `Resolved` | The reference has a target symbol. |
| `Unresolved` | Resolver setup existed, but no target was found. |
| `Ambiguous` | More than one candidate target was found. |
| `SetupMissing` | Required setup was absent or failed. |
| `Unsupported` | The provider does not support this reference form yet. |

Uncertain states are data, not hidden failures. Rules should decide explicitly
how to handle unresolved, ambiguous, setup-missing, unsupported, and heuristic
facts.

The precision and status values may reflect lexical, module-linked, ambiguous,
unresolved, setup-missing, unsupported, generated or external semantic evidence.
These labels describe the existing `Symbols<'_>` and `References<'_>` facts
only; scopes/import closure/resolution-step rows remain internal and are not
separate rule-author fact views.

## Language Coverage

### TypeScript And JavaScript

TS/JS symbols and references are derived from Oxc semantic data plus polint's
module graph.

Current strengths:

- exact local lexical symbols and references
- function, class, interface, type alias, enum, variable, constant, parameter,
  import, and namespace-style symbol identities where Oxc exposes them
- local read, write, read-write, call, and type-use references
- unresolved root references
- module-linked import alias references when import resolution finds the target
- TypeScript declaration merging represented as one symbol with multiple
  definitions when Oxc reports redeclarations

Current limits:

- no TypeScript type-checker sidecar
- no cross-file member or property resolution beyond module-linked import/export
  aliases
- no declaration-file or project-reference precision claims
- a `qualified_name` names at most the 64 innermost scopes that enclose the
  symbol; for a symbol nested deeper, `...` stands for the outer ones. Every
  scope it names carries its source span, so the name still identifies one
  symbol
- no call graph, CFG, coverage, or dataflow facts
- no public exposure of Oxc IDs, raw Oxc semantic records, AST nodes, or resolver
  internals

### Go

Go symbols and references are derived from a small sidecar that uses typed Go
package information when repository setup is available.

Current strengths:

- package-level function and method symbols
- field, parameter, local variable, constant, type, and package symbols where
  typed package data exposes them
- call references, selector references, field references, reads, writes, and
  type-use references
- `ExactSemantic` precision for typed facts emitted by the sidecar
- setup-aware diagnostics when Go package loading cannot run
- monorepos with Go modules below the repository root, inferred automatically or
  declared through `[languages.go].module_roots`

Current limits:

- requires Go 1.25 or newer on `PATH` when using default embedded source
  sidecars; prebuilt `POLINT_GO_SYMBOLS` or `POLINT_GO_FRONTEND` binaries can
  avoid requiring Go for the corresponding sidecar
- each analyzed Go file must belong to a Go module with a `go.mod`
- package loading must succeed for the configured package patterns and build tags
- setup failures produce `polint/capability` diagnostics and block requesting
  Go symbol/reference rules instead of running them with placeholder facts
- package patterns are interpreted inside each configured module root; repo-level
  `go.work` is honored, otherwise polint can use a temporary internal workspace
  for package loading below the repository root
- no Go SSA, pointer analysis, call graph, CFG, coverage, or dataflow facts
- no public exposure of Go object values, object addresses, package loader JSON,
  sidecar DTOs, or sidecar internals

The private `polint-go-frontend` semantic sidecar used by graph analysis is
separate from the public symbol/reference fact surface documented here. Its
private rows are not SDK facts and should not be imported by repo-local rules.

## Cache And Determinism

Symbol and reference facts participate in deterministic planning and cache
inputs through the requested capabilities and provider setup. Repeated checks of
unchanged source should preserve stable `SymbolId` and `ReferenceId` evidence
across cold and warm cache runs.

Rules should prefer public IDs, status, precision, file paths, spans, and stable
keys for diagnostics. Do not depend on fact vector positions or any language
engine's internal IDs.
