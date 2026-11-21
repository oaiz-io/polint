# Resolved Imports And Module Graph

`ResolvedImports<'_>` and `ModuleGraphFacts<'_>` expose setup-aware import
resolution and module relationship facts to repo-local rules. Rules request
these views as typed parameters in a `#[polint::rule]` function, and polint
derives the `resolved_imports` and `module_graph` capabilities from those
parameters.

A site is exact or complete only when polint has checked every applicable way
the specifier can resolve. A limit, a refused input, or a selectable condition
whose outcome is not shared by all profiles leaves the site open, with an
unknown marker explaining why. A known candidate does not close the site, and
an unchecked input is not evidence that there is no target.

Start rules with:

```rust
use polint::sdk::prelude::*;
```

## ResolvedImports<'_>

`ResolvedImports<'_>` is import-centric. It returns one `ResolvedImportFact` for
each syntactic `ImportFact` that polint harvested, including imports that did
not resolve.

Query methods:

| Method | Meaning |
|--------|---------|
| `all()` | Returns all resolved import facts in deterministic database order. |
| `iter()` | Iterates all resolved import facts. |
| `for_file(file)` | Iterates resolved imports from one source file. |
| `resolved_for_file(file)` | Iterates only `Resolved` imports from one source file. |
| `by_specifier(specifier)` | Iterates resolved import facts whose syntactic import path exactly matches `specifier`. |
| `unresolved()` | Iterates imports with `Unresolved` status. |
| `dynamic()` | Iterates imports with `Dynamic` status. |
| `unsupported()` | Iterates imports with `Unsupported` status. |
| `unresolved_for_file(file)` | Iterates non-resolved imports from one source file. For running relationship rules this means `Unresolved`, `Dynamic`, or `Unsupported`; `SetupMissing` is reported as a capability diagnostic before rule execution. |
| `for_import(import)` | Returns the resolved import fact for a syntactic `ImportId`. |

`ResolvedImportFact` fields:

| Field | Meaning |
|-------|---------|
| `id` | Stable ID for this analysis run. |
| `import` | The syntactic `ImportId` this fact came from. |
| `from_file` | Source file containing the import. |
| `target_node` | Resolved module graph node, when one exists. |
| `status` | `ResolutionStatus` for the import. |
| `precision` | `ResolutionPrecision` for the result. |
| `reason` | Optional `UnresolvedReason` for uncertain results. |
| `alternatives` | For an import with no single target (`ModeDependent`), the module graph nodes of the analysed files that one way of loading it may name: its mode, its conditions, Node or a bundler. Each is a conservative candidate, and the list may be incomplete. Empty when `target_node` names the one target. |

## Example: Report Unresolved Imports

Use `ResolvedImports<'_>` when a policy cares about setup-aware resolution
status instead of just the literal import text:

```rust
use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/no-unresolved-imports",
    description = "Require imports to resolve through project setup.",
    severity = "warn"
)]
fn no_unresolved_imports(
    ctx: &mut RuleCtx<'_>,
    imports: ResolvedImports<'_>,
) -> RuleResult {
    for import in imports.iter() {
        if matches!(
            import.status,
            ResolutionStatus::Resolved | ResolutionStatus::External
        ) {
            continue;
        }

        ctx.report(
            Diagnostic::warning(
                ctx.rule_id(),
                ctx.file_path(import.from_file),
                DiagnosticRange::point(1, 1),
                "Import did not resolve through the configured project setup.",
            )
            .with_evidence("status", format!("{:?}", import.status))
            .with_evidence("precision", format!("{:?}", import.precision)),
        );
    }
    Ok(())
}
```

## ModuleGraphFacts<'_>

`ModuleGraphFacts<'_>` is graph-centric. It exposes normalized file, package,
module, and external dependency nodes plus relationship edges between them.

Query methods:

| Method | Meaning |
|--------|---------|
| `nodes()` | Returns all `ModuleNode` records in deterministic order. |
| `edges()` | Returns all `ModuleEdge` records in deterministic order. |
| `node_for_file(file)` | Returns the first file node for a source file. |
| `nodes_for_package(package_name)` | Iterates graph nodes whose label or source package name matches `package_name`. |
| `edges_from_file(file)` | Iterates outgoing edges from the first file node for a source file. |
| `imports_between(from, to)` | Iterates graph edges from one file node to another when both files have graph nodes. |
| `outgoing(node)` | Iterates outgoing edges for a node. |
| `incoming(node)` | Iterates incoming edges for a node. |
| `dependency_status(edge)` | Returns the `ResolutionStatus` attached to an edge. |
| `reachable_from(node)` | Computes deterministic breadth-first reachability over `Resolved` and `External` edges. |

`ModuleNode` fields:

| Field | Meaning |
|-------|---------|
| `id` | Stable node ID for this analysis run. |
| `kind` | `File`, `Package`, `Module`, or `External`. |
| `label` | Repo-relative path, package name, module label, or external package label. |
| `file` | Source `FileId` for file nodes. |
| `package` | Source `PackageId` when the node comes from a package fact. |
| `language` | Language family when the node is language-specific. |

`ModuleEdge` fields:

| Field | Meaning |
|-------|---------|
| `id` | Stable edge ID for this analysis run. |
| `from` | Source `ModuleNodeId`. |
| `to` | Target `ModuleNodeId`. |
| `import` | Syntactic `ImportId` when the edge came from an import. |
| `resolved_import` | `ResolvedImportId` when the edge came from a resolved import fact. |
| `kind` | `Contains`, `Imports`, or `DependsOn`. |
| `status` | Resolution status for dependency edges. |

## Example: Disallow UI To Domain Edges

Use `ModuleGraphFacts<'_>` when a policy is about relationships between files,
packages, modules, or external dependencies:

```rust
use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/no-ui-to-domain",
    description = "Disallow UI files importing domain files directly.",
    severity = "error"
)]
fn no_ui_to_domain(
    ctx: &mut RuleCtx<'_>,
    graph: ModuleGraphFacts<'_>,
) -> RuleResult {
    for edge in graph.edges() {
        let Some(source) = graph.nodes().iter().find(|node| node.id == edge.from) else {
            continue;
        };
        let Some(target) = graph.nodes().iter().find(|node| node.id == edge.to) else {
            continue;
        };

        if !source.label.starts_with("src/ui/") || !target.label.starts_with("src/domain/") {
            continue;
        }

        let file = source
            .file
            .map(|file| ctx.file_path(file))
            .unwrap_or_else(|| source.label.clone());
        ctx.report(
            Diagnostic::error(
                ctx.rule_id(),
                file,
                DiagnosticRange::point(1, 1),
                "UI code must not import the domain layer directly.",
            )
            .with_evidence("source", source.label.clone())
            .with_evidence("target", target.label.clone())
            .with_evidence("status", format!("{:?}", graph.dependency_status(edge))),
        );
    }
    Ok(())
}
```

## Status, Precision, And Reasons

`ResolutionStatus` is the main control point for architecture rules:

| Status | Meaning |
|--------|---------|
| `Resolved` | Target is a repo file, package, or module node. |
| `External` | Target is outside the repo, including standard library and package-manager dependencies. |
| `Unresolved` | Resolver setup existed, but no target was found. |
| `SetupMissing` | Required resolver setup was absent or failed; requesting relationship rules receive `polint/capability` diagnostics and do not run. |
| `Dynamic` | Import shape is dynamic and cannot be resolved statically. |
| `Unsupported` | Language or import form is known but not implemented. |

`ResolutionPrecision` currently includes `ExactFile`, `Package`,
`ExternalPackage`, `Heuristic`, and `None`.

`UnresolvedReason` currently includes `NotFound`, `SetupMissing`,
`DynamicExpression`, `UnsupportedLanguage`, `UnsupportedImport`, `ResolverError`,
`OutsideWorkspace`, and `ModeDependent`.

TS/JS imports resolve in the mode their file loads in, so an `exports` map
with `import` and `require` conditions gives each its own file. A `require`
resolves in require mode. Import syntax resolves in import mode when the
importing file runs as an ES module: `.mjs`, `.mts`, or a `.js` or `.ts` file
of a package with `"type": "module"`. Elsewhere a compiler may turn the import
into a `require`, so the import resolves only when both modes name the same
file; otherwise it is `Unresolved` with reason `ModeDependent`.

A package without `exports` loads its `main` under Node, but its `module` or
`browser` field first under a bundler, so an import resolves only when Node
and a bundler name the same file as well. A package specifier whose files are
not analysed stays `External` either way, since its label is the specifier.

An `exports` or `imports` map is read for every condition set at once. A
condition the runtime always selects (`default`, and `import` or `require` for
the file's mode) ends the search for its map; every other condition may or may
not be set, so each file it reaches is a candidate. Arrays go on past `null`,
an invalid target and an entry that selects nothing, as Node's do; a map with
a numeric key, or an `exports` map that mixes subpaths and conditions, names
nothing. A target that is another specifier, such as a package name or a
self reference, is followed in turn, up to a fixed number of follows. An
import resolves to one file only when every candidate is that file; a
declaration file is not a candidate. Where a condition repeats under nested
maps the candidates may include a file no runtime loads, so the list is
conservative. The candidates are checked against Node on a generated corpus
of maps (`tests/js-analysis-bench/conditions/`).

A `ModeDependent` import lists in `alternatives` each analysed file one way of
loading it names. The call graph follows each of them as a conservative
target, so a call through the import may reach every candidate and none of
its answers is exact or complete.

## Setup-Sensitive Behavior

- TS/JS resolution uses `oxc_resolver` with project-aware settings such as
  package metadata, extension aliases, and `tsconfig` discovery.
- Go resolution uses the same `[languages.go]` lifecycle as symbol/reference
  facts: module roots are inferred from discovered Go files or declared with
  `module_roots`, package patterns are interpreted inside those roots, and a
  temporary internal `go.work` is used when needed.
- Standard library imports and package-manager dependencies are reported as
  `External`.
- `Unresolved`, `Dynamic`, and `Unsupported` imports remain visible as facts to
  running relationship rules instead of being dropped.
- `SetupMissing` is surfaced through `polint/capability` diagnostics for
  requesting rules, and those rules do not execute with placeholder relationship
  facts.

## Limits

These facts are useful for architecture policies, but they are not a full
semantic graph:

- no TypeScript type checking
- no Go type checking
- no symbols
- no call graph
- no CFG
- no coverage
- no dataflow or taint analysis
- no project-level graph cache
- no public exposure of raw `oxc_resolver` output, `go list` JSON, absolute
  resolver paths, AST nodes, or graph internals

More precise symbol, call, CFG, coverage, and dataflow facts are later fact
families. Rules should treat uncertain `ResolutionStatus` and
`UnresolvedReason` values as part of the contract rather than as errors.
