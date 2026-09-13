//! Typed fact views used by macro-derived rules.
//!
//! A rule requests facts by accepting these view types as parameters. The
//! `#[polint::rule]` macro derives capabilities from the view types and builds
//! the matching views at runtime.

use crate::core::{
    AnalysisDb, BranchObligation, ChangeStatus, ChangedFile, ComplexityMetricFact, CoverageFact,
    DefinitionFact, FileId, FileMetricFact, FunctionFact, FunctionId, FunctionMetricFact,
    GoTypeDeclFact, ImportFact, ImportId, JsxAttributeFact, Language, ModuleEdge, ModuleNode,
    ModuleNodeId, PackageFact, ReferenceFact, ResolutionStatus, ResolvedImportFact,
    ReviewChangeset, SourceFile, StringLiteralFact, SymbolFact, SymbolId, SymbolKind,
    SymbolResolutionStatus, TestFact, TsClassFact, TsComponentFact,
};
use crate::sdk::policy::{
    EventPattern, FlowQuery, GuardQuery, LifecycleQuery, PolicyResult, PolicyViolation, ReachQuery,
};
use crate::symbol_graph::query;

/// Public source-file view. Requesting this view maps to the `syntax` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct SourceFiles<'a> {
    db: &'a AnalysisDb,
}

impl<'a> SourceFiles<'a> {
    /// Returns all analyzed source files in deterministic database order.
    pub fn all(self) -> &'a [SourceFile] {
        self.db.files()
    }

    /// Iterates all analyzed source files in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, SourceFile> {
        self.db.files().iter()
    }

    /// Returns the source file for a stable file ID.
    pub fn get(self, file: FileId) -> Option<&'a SourceFile> {
        self.db.file(file)
    }

    /// Returns the language for a stable file ID.
    pub fn language(self, file: FileId) -> Option<Language> {
        self.get(file).map(|source| source.language)
    }

    /// Returns a display path for a file ID, or `<unknown>` if missing.
    pub fn path_for(self, file: FileId) -> String {
        self.db.path_for(file)
    }
}

/// Package fact view. Requesting this view maps to the `syntax` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct Packages<'a> {
    db: &'a AnalysisDb,
}

impl<'a> Packages<'a> {
    /// Returns all package facts in deterministic database order.
    pub fn all(self) -> &'a [PackageFact] {
        self.db.packages()
    }

    /// Iterates all package facts in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, PackageFact> {
        self.db.packages().iter()
    }
}

/// Function fact view. Requesting this view maps to the `syntax` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct Functions<'a> {
    db: &'a AnalysisDb,
}

impl<'a> Functions<'a> {
    /// Returns all function facts in deterministic database order.
    pub fn all(self) -> &'a [FunctionFact] {
        self.db.functions()
    }

    /// Iterates all function facts in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, FunctionFact> {
        self.db.functions().iter()
    }

    /// Returns function facts for a file without cloning facts.
    pub fn for_file(self, file: FileId) -> impl Iterator<Item = &'a FunctionFact> {
        self.db.functions_for_file(file)
    }
}

/// Source-file metric view. Requesting this view maps to the `file_metrics` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct FileMetrics<'a> {
    db: &'a AnalysisDb,
}

impl<'a> FileMetrics<'a> {
    /// Returns all derived file metrics in deterministic database order.
    pub fn all(self) -> &'a [FileMetricFact] {
        self.db.file_metrics()
    }

    /// Iterates all derived file metrics in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, FileMetricFact> {
        self.db.file_metrics().iter()
    }

    /// Returns derived metrics for one source file.
    pub fn get(self, file: FileId) -> Option<&'a FileMetricFact> {
        self.db.file_metric_for_file(file)
    }

    /// Returns file metrics for one language.
    pub fn for_language(self, language: Language) -> impl Iterator<Item = &'a FileMetricFact> {
        self.db
            .file_metrics()
            .iter()
            .filter(move |metric| metric.language == language)
    }

    /// Iterates files whose total line count is greater than `max`.
    pub fn over_line_count(self, max: u32) -> impl Iterator<Item = &'a FileMetricFact> {
        self.db
            .file_metrics()
            .iter()
            .filter(move |metric| metric.line_count > max)
    }

    /// Iterates files whose byte count is greater than `max`.
    pub fn over_byte_count(self, max: u32) -> impl Iterator<Item = &'a FileMetricFact> {
        self.db
            .file_metrics()
            .iter()
            .filter(move |metric| metric.byte_count > max)
    }

    /// Iterates files whose derived function count is greater than `max`.
    pub fn over_function_count(self, max: u32) -> impl Iterator<Item = &'a FileMetricFact> {
        self.db
            .file_metrics()
            .iter()
            .filter(move |metric| metric.function_count > max)
    }
}

/// Function-size metric view. Requesting this view maps to the `function_metrics` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct FunctionMetrics<'a> {
    db: &'a AnalysisDb,
}

impl<'a> FunctionMetrics<'a> {
    /// Returns all derived function-size metrics in deterministic database order.
    pub fn all(self) -> &'a [FunctionMetricFact] {
        self.db.function_metrics()
    }

    /// Iterates all derived function-size metrics in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, FunctionMetricFact> {
        self.db.function_metrics().iter()
    }

    /// Returns function-size metrics for a file without cloning facts.
    pub fn for_file(self, file: FileId) -> impl Iterator<Item = &'a FunctionMetricFact> {
        self.db.function_metrics_for_file(file)
    }

    /// Returns function-size metrics for one function.
    pub fn get(self, function: FunctionId) -> Option<&'a FunctionMetricFact> {
        self.db.function_metric_for_function(function)
    }

    /// Iterates functions whose total line count is greater than `max`.
    pub fn over_line_count(self, max: u32) -> impl Iterator<Item = &'a FunctionMetricFact> {
        self.db
            .function_metrics()
            .iter()
            .filter(move |metric| metric.line_count > max)
    }

    /// Iterates functions whose byte count is greater than `max`.
    pub fn over_byte_count(self, max: u32) -> impl Iterator<Item = &'a FunctionMetricFact> {
        self.db
            .function_metrics()
            .iter()
            .filter(move |metric| metric.byte_count > max)
    }
}

/// Complexity metric view. Requesting this view maps to the `complexity_metrics` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct ComplexityMetrics<'a> {
    db: &'a AnalysisDb,
}

impl<'a> ComplexityMetrics<'a> {
    /// Returns all derived complexity metrics in deterministic database order.
    pub fn all(self) -> &'a [ComplexityMetricFact] {
        self.db.complexity_metrics()
    }

    /// Iterates all derived complexity metrics in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, ComplexityMetricFact> {
        self.db.complexity_metrics().iter()
    }

    /// Returns complexity metrics for a file without cloning facts.
    pub fn for_file(self, file: FileId) -> impl Iterator<Item = &'a ComplexityMetricFact> {
        self.db.complexity_metrics_for_file(file)
    }

    /// Returns complexity metrics for one function.
    pub fn get(self, function: FunctionId) -> Option<&'a ComplexityMetricFact> {
        self.db.complexity_metric_for_function(function)
    }

    /// Iterates functions whose cyclomatic complexity is greater than `max`.
    pub fn over(self, max: u32) -> impl Iterator<Item = &'a ComplexityMetricFact> {
        self.db
            .complexity_metrics()
            .iter()
            .filter(move |metric| metric.cyclomatic_complexity > max)
    }

    /// Alias for [`ComplexityMetrics::over`] that names the threshold explicitly.
    pub fn over_complexity(self, max: u32) -> impl Iterator<Item = &'a ComplexityMetricFact> {
        self.over(max)
    }
}

/// Import fact view. Requesting this view maps to the `imports` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct Imports<'a> {
    db: &'a AnalysisDb,
}

impl<'a> Imports<'a> {
    /// Returns all import facts in deterministic database order.
    pub fn all(self) -> &'a [ImportFact] {
        self.db.imports()
    }

    /// Iterates all import facts in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, ImportFact> {
        self.db.imports().iter()
    }

    /// Returns import facts for a file without cloning facts.
    pub fn for_file(self, file: FileId) -> impl Iterator<Item = &'a ImportFact> {
        self.db.imports_for_file(file)
    }

    /// Returns syntactic import graph edges as `(source file, import)` pairs.
    pub fn edges(self) -> impl Iterator<Item = (&'a SourceFile, &'a ImportFact)> {
        self.db
            .imports()
            .iter()
            .filter_map(move |import| self.db.file(import.file).map(|file| (file, import)))
    }
}

/// Resolved import fact view. Requesting this view maps to the `resolved_imports` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct ResolvedImports<'a> {
    db: &'a AnalysisDb,
}

impl<'a> ResolvedImports<'a> {
    /// Returns all resolved import facts in deterministic database order.
    pub fn all(self) -> &'a [ResolvedImportFact] {
        self.db.resolved_imports()
    }

    /// Iterates all resolved import facts in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, ResolvedImportFact> {
        self.db.resolved_imports().iter()
    }

    /// Returns resolved import facts for a source file without cloning facts.
    pub fn for_file(self, file: FileId) -> impl Iterator<Item = &'a ResolvedImportFact> {
        self.db.resolved_imports_for_file(file)
    }

    /// Returns resolved imports for a source file.
    pub fn resolved_for_file(self, file: FileId) -> impl Iterator<Item = &'a ResolvedImportFact> {
        self.for_file(file)
            .filter(|resolved| resolved.status == ResolutionStatus::Resolved)
    }

    /// Returns resolved import facts whose syntactic import path exactly matches `specifier`.
    pub fn by_specifier(self, specifier: &str) -> impl Iterator<Item = &'a ResolvedImportFact> {
        self.db.resolved_imports().iter().filter(move |resolved| {
            self.db
                .imports()
                .iter()
                .find(|import| import.id == resolved.import)
                .is_some_and(|import| import.path == specifier)
        })
    }

    /// Returns imports that could not be resolved to a target.
    pub fn unresolved(self) -> impl Iterator<Item = &'a ResolvedImportFact> {
        self.db
            .resolved_imports()
            .iter()
            .filter(|resolved| resolved.status == ResolutionStatus::Unresolved)
    }

    /// Returns imports whose target is dynamic.
    pub fn dynamic(self) -> impl Iterator<Item = &'a ResolvedImportFact> {
        self.db
            .resolved_imports()
            .iter()
            .filter(|resolved| resolved.status == ResolutionStatus::Dynamic)
    }

    /// Returns imports whose language or import form is unsupported.
    pub fn unsupported(self) -> impl Iterator<Item = &'a ResolvedImportFact> {
        self.db
            .resolved_imports()
            .iter()
            .filter(|resolved| resolved.status == ResolutionStatus::Unsupported)
    }

    /// Returns setup-missing, dynamic, unsupported, or unresolved imports for a source file.
    pub fn unresolved_for_file(self, file: FileId) -> impl Iterator<Item = &'a ResolvedImportFact> {
        self.for_file(file).filter(|resolved| {
            matches!(
                resolved.status,
                ResolutionStatus::Unresolved
                    | ResolutionStatus::SetupMissing
                    | ResolutionStatus::Dynamic
                    | ResolutionStatus::Unsupported
            )
        })
    }

    /// Returns the resolved import record for a syntactic import ID.
    pub fn for_import(self, import: ImportId) -> Option<&'a ResolvedImportFact> {
        self.db
            .resolved_imports()
            .iter()
            .find(|resolved| resolved.import == import)
    }
}

/// Module relationship graph fact view. Requesting this view maps to the `module_graph` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct ModuleGraphFacts<'a> {
    db: &'a AnalysisDb,
}

impl<'a> ModuleGraphFacts<'a> {
    /// Returns all module graph nodes in deterministic database order.
    pub fn nodes(self) -> &'a [ModuleNode] {
        self.db.module_nodes()
    }

    /// Returns all module graph edges in deterministic database order.
    pub fn edges(self) -> &'a [ModuleEdge] {
        self.db.module_edges()
    }

    /// Returns the first file node for a source file ID, if one exists.
    pub fn node_for_file(self, file: FileId) -> Option<ModuleNodeId> {
        self.db.module_node_for_file(file).map(|node| node.id)
    }

    /// Returns graph nodes attached to a package name or label.
    pub fn nodes_for_package(self, package_name: &str) -> impl Iterator<Item = &'a ModuleNode> {
        self.db.module_nodes().iter().filter(move |node| {
            node.label == package_name
                || node
                    .package
                    .and_then(|package| {
                        self.db
                            .packages()
                            .iter()
                            .find(|candidate| candidate.id == package)
                    })
                    .is_some_and(|package| package.name == package_name)
        })
    }

    /// Returns outgoing graph edges from the first file node for a source file.
    pub fn edges_from_file(self, file: FileId) -> impl Iterator<Item = &'a ModuleEdge> {
        let node = self.node_for_file(file);
        self.db
            .module_edges()
            .iter()
            .filter(move |edge| node.is_some_and(|node| edge.from == node))
    }

    /// Returns graph edges from one file node to another, if both files are present in the graph.
    pub fn imports_between(self, from: FileId, to: FileId) -> impl Iterator<Item = &'a ModuleEdge> {
        let from_node = self.node_for_file(from);
        let to_node = self.node_for_file(to);
        self.db.module_edges().iter().filter(move |edge| {
            from_node.is_some_and(|from_node| edge.from == from_node)
                && to_node.is_some_and(|to_node| edge.to == to_node)
        })
    }

    /// Returns outgoing graph edges for a node without cloning facts.
    pub fn outgoing(self, node: ModuleNodeId) -> impl Iterator<Item = &'a ModuleEdge> {
        self.db
            .module_edges()
            .iter()
            .filter(move |edge| edge.from == node)
    }

    /// Returns incoming graph edges for a node without cloning facts.
    pub fn incoming(self, node: ModuleNodeId) -> impl Iterator<Item = &'a ModuleEdge> {
        self.db
            .module_edges()
            .iter()
            .filter(move |edge| edge.to == node)
    }

    /// Returns the resolution status attached to a dependency edge.
    pub fn dependency_status(self, edge: &ModuleEdge) -> ResolutionStatus {
        edge.status
    }

    /// Computes deterministic breadth-first reachability over resolved or external edges.
    pub fn reachable_from(self, node: ModuleNodeId) -> Vec<ModuleNodeId> {
        let mut seen = std::collections::BTreeSet::new();
        let mut queue = std::collections::VecDeque::new();
        let mut reachable = Vec::new();

        seen.insert(node);
        queue.push_back(node);

        while let Some(current) = queue.pop_front() {
            for edge in self.outgoing(current) {
                if !matches!(
                    edge.status,
                    ResolutionStatus::Resolved | ResolutionStatus::External
                ) {
                    continue;
                }

                if seen.insert(edge.to) {
                    reachable.push(edge.to);
                    queue.push_back(edge.to);
                }
            }
        }

        reachable
    }
}

/// Symbol and definition fact view. Requesting this view maps to the `symbols` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct Symbols<'a> {
    db: &'a AnalysisDb,
}

impl<'a> Symbols<'a> {
    /// Returns all symbol facts in deterministic database order.
    pub fn all(self) -> &'a [SymbolFact] {
        self.db.symbols()
    }

    /// Iterates all symbol facts in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, SymbolFact> {
        self.db.symbols().iter()
    }

    /// Returns a symbol fact for a stable symbol ID.
    pub fn get(self, symbol: SymbolId) -> Option<&'a SymbolFact> {
        self.db.symbol_by_id(symbol)
    }

    /// Resolves a symbol's stable identity text.
    pub fn stable_key(self, symbol: &SymbolFact) -> std::sync::Arc<str> {
        self.db.resolve_stable_key(symbol.stable_key)
    }

    /// Resolves a definition's stable identity text.
    pub fn definition_stable_key(self, definition: &DefinitionFact) -> std::sync::Arc<str> {
        self.db.resolve_stable_key(definition.stable_key)
    }

    /// Returns symbol facts for one source file without cloning facts.
    pub fn for_file(self, file: FileId) -> impl Iterator<Item = &'a SymbolFact> {
        self.db.symbols_for_file(file)
    }

    /// Returns symbol facts with the exact public name.
    pub fn by_name(self, name: &str) -> impl Iterator<Item = &'a SymbolFact> {
        query::symbols_by_name(self.db, name)
    }

    /// Returns symbol facts of a specific public symbol kind.
    pub fn by_kind(self, kind: SymbolKind) -> impl Iterator<Item = &'a SymbolFact> {
        self.db
            .symbols()
            .iter()
            .filter(move |symbol| symbol.kind == kind)
    }

    /// Returns exported symbols with the exact public name.
    pub fn exported_by_name(self, name: &str) -> impl Iterator<Item = &'a SymbolFact> {
        self.db
            .symbols()
            .iter()
            .filter(move |symbol| symbol.is_exported && symbol.name == name)
    }

    /// Returns definitions whose primary location is in a source file.
    pub fn definitions_in_file(self, file: FileId) -> impl Iterator<Item = &'a DefinitionFact> {
        self.db.definitions_for_file(file)
    }

    /// Returns the primary definition for a symbol, if one is known.
    pub fn definition(self, symbol: SymbolId) -> Option<&'a DefinitionFact> {
        self.db.definition_for_symbol(symbol)
    }

    /// Returns all definitions for a symbol, including declaration-merged definitions.
    pub fn definitions(self, symbol: SymbolId) -> impl Iterator<Item = &'a DefinitionFact> {
        self.db.definitions_for_symbol(symbol)
    }

    /// Returns exported symbols without cloning facts.
    pub fn exported(self) -> impl Iterator<Item = &'a SymbolFact> {
        self.db.symbols().iter().filter(|symbol| symbol.is_exported)
    }
}

/// Reference fact view. Requesting this view maps to the `references` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct References<'a> {
    db: &'a AnalysisDb,
}

impl<'a> References<'a> {
    /// Returns all reference facts in deterministic database order.
    pub fn all(self) -> &'a [ReferenceFact] {
        self.db.references()
    }

    /// Iterates all reference facts in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, ReferenceFact> {
        self.db.references().iter()
    }

    /// Resolves a reference's stable identity text.
    pub fn stable_key(self, reference: &ReferenceFact) -> std::sync::Arc<str> {
        self.db.resolve_stable_key(reference.stable_key)
    }

    /// Returns resolved references to a symbol without cloning facts.
    pub fn to(self, symbol: SymbolId) -> impl Iterator<Item = &'a ReferenceFact> {
        query::references_to_symbol(self.db, symbol)
    }

    /// Returns references in one source file without cloning facts.
    pub fn for_file(self, file: FileId) -> impl Iterator<Item = &'a ReferenceFact> {
        self.db.references_for_file(file)
    }

    /// Returns references explicitly marked unresolved.
    pub fn unresolved(self) -> impl Iterator<Item = &'a ReferenceFact> {
        query::unresolved_references(self.db)
    }

    /// Returns references explicitly marked ambiguous.
    pub fn ambiguous(self) -> impl Iterator<Item = &'a ReferenceFact> {
        query::ambiguous_references(self.db)
    }

    /// Returns references explicitly resolved to one target.
    pub fn resolved(self) -> impl Iterator<Item = &'a ReferenceFact> {
        self.db
            .references()
            .iter()
            .filter(|reference| reference.status == SymbolResolutionStatus::Resolved)
    }

    /// Returns references with the exact public name.
    pub fn by_name(self, name: &str) -> impl Iterator<Item = &'a ReferenceFact> {
        self.db
            .references()
            .iter()
            .filter(move |reference| reference.name == name)
    }

    /// Returns references resolved to any symbol yielded by `symbols`.
    pub fn to_any<I>(self, symbols: I) -> impl Iterator<Item = &'a ReferenceFact>
    where
        I: IntoIterator<Item = &'a SymbolFact>,
    {
        let targets = symbols
            .into_iter()
            .map(|symbol| symbol.id)
            .collect::<std::collections::BTreeSet<_>>();
        self.db.references().iter().filter(move |reference| {
            reference
                .target
                .is_some_and(|target| targets.contains(&target))
        })
    }

    /// Returns unresolved references with the exact public name.
    pub fn unresolved_by_name(self, name: &str) -> impl Iterator<Item = &'a ReferenceFact> {
        self.unresolved()
            .filter(move |reference| reference.name == name)
    }
}

/// Branch obligation fact view. Requesting this view maps to the `branch_obligations` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct BranchObligations<'a> {
    db: &'a AnalysisDb,
}

impl<'a> BranchObligations<'a> {
    /// Returns all branch obligations in deterministic database order.
    pub fn all(self) -> &'a [BranchObligation] {
        self.db.branches()
    }

    /// Iterates all branch obligations in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, BranchObligation> {
        self.db.branches().iter()
    }

    /// Returns branch obligations for a file without cloning facts.
    pub fn for_file(self, file: FileId) -> impl Iterator<Item = &'a BranchObligation> {
        self.db.branches_for_file(file)
    }

    /// Returns branch obligations for a function without cloning facts.
    pub fn for_function(self, function: FunctionId) -> impl Iterator<Item = &'a BranchObligation> {
        self.db
            .branches()
            .iter()
            .filter(move |branch| branch.function == Some(function))
    }
}

/// Go test fact view. Requesting this view maps to the `go_tests` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct GoTests<'a> {
    db: &'a AnalysisDb,
}

impl<'a> GoTests<'a> {
    /// Returns all Go test facts in deterministic database order.
    pub fn all(self) -> &'a [TestFact] {
        self.db.tests()
    }

    /// Iterates all Go test facts in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, TestFact> {
        self.db.tests().iter()
    }

    /// Returns Go test facts for a file without cloning facts.
    pub fn for_file(self, file: FileId) -> impl Iterator<Item = &'a TestFact> {
        self.db.tests_for_file(file)
    }

    /// Returns Go tests related to a source file.
    pub fn related_for_file(self, file: FileId) -> Vec<&'a TestFact> {
        let Some(source_file) = self.db.file(file) else {
            return Vec::new();
        };
        if source_file.language != Language::Go {
            return Vec::new();
        }

        let source_path = std::path::Path::new(&source_file.relative_path);
        let source_name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();

        if source_name.ends_with("_test.go") {
            return self.for_file(file).collect();
        }

        let source_dir = source_path.parent();
        self.db
            .tests()
            .iter()
            .filter(|test| {
                if test.file == file {
                    return true;
                }
                let Some(test_file) = self.db.file(test.file) else {
                    return false;
                };
                if test_file.language != Language::Go {
                    return false;
                }
                let test_path = std::path::Path::new(&test_file.relative_path);
                let test_name = test_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default();
                test_name.ends_with("_test.go") && test_path.parent() == source_dir
            })
            .collect()
    }
}

/// Go structural type fact view. Requesting this view maps to the `syntax` capability.
///
/// Exposes typed Go declarations (`type X struct|interface|...`, aliases, grouped and
/// function-local specs) plus anonymous `struct { ... }` occurrences so rules can stop
/// re-parsing Go source with regexes and brace matching. See `docs/facts/go-types.md`.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct GoTypeDecls<'a> {
    db: &'a AnalysisDb,
}

impl<'a> GoTypeDecls<'a> {
    /// Returns all Go structural type facts in deterministic database order.
    pub fn all(self) -> &'a [GoTypeDeclFact] {
        self.db.go_types()
    }

    /// Iterates all Go structural type facts in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, GoTypeDeclFact> {
        self.db.go_types().iter()
    }

    /// Returns Go structural type facts for a file without cloning facts.
    pub fn for_file(self, file: FileId) -> impl Iterator<Item = &'a GoTypeDeclFact> {
        self.db.go_types_for_file(file)
    }
}

/// TS/JS component fact view. Requesting this view maps to the `ts_components` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct TsComponents<'a> {
    db: &'a AnalysisDb,
}

impl<'a> TsComponents<'a> {
    /// Returns all component facts in deterministic database order.
    pub fn all(self) -> &'a [TsComponentFact] {
        self.db.ts_components()
    }

    /// Iterates all component facts in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, TsComponentFact> {
        self.db.ts_components().iter()
    }

    /// Returns component facts for a file without cloning facts.
    pub fn for_file(self, file: FileId) -> impl Iterator<Item = &'a TsComponentFact> {
        self.db.ts_components_for_file(file)
    }
}

/// TS/JS class fact view. Requesting this view maps to the `ts_classes` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct TsClasses<'a> {
    db: &'a AnalysisDb,
}

impl<'a> TsClasses<'a> {
    /// Returns all class facts in deterministic database order.
    pub fn all(self) -> &'a [TsClassFact] {
        self.db.ts_classes()
    }

    /// Iterates all class facts in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, TsClassFact> {
        self.db.ts_classes().iter()
    }

    /// Returns class facts for a file without cloning facts.
    pub fn for_file(self, file: FileId) -> impl Iterator<Item = &'a TsClassFact> {
        self.db.ts_classes_for_file(file)
    }
}

/// String-literal fact view. Requesting this view maps to the `string_literals` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct StringLiterals<'a> {
    db: &'a AnalysisDb,
}

impl<'a> StringLiterals<'a> {
    /// Returns all string literal facts in deterministic database order.
    pub fn all(self) -> &'a [StringLiteralFact] {
        self.db.string_literals()
    }

    /// Iterates all string literal facts in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, StringLiteralFact> {
        self.db.string_literals().iter()
    }

    /// Returns string literal facts for a file without cloning facts.
    pub fn for_file(self, file: FileId) -> impl Iterator<Item = &'a StringLiteralFact> {
        self.db.string_literals_for_file(file)
    }
}

/// JSX attribute fact view. Requesting this view maps to the `jsx_attributes` capability.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct JsxAttributes<'a> {
    db: &'a AnalysisDb,
}

impl<'a> JsxAttributes<'a> {
    /// Returns all JSX attribute facts in deterministic database order.
    pub fn all(self) -> &'a [JsxAttributeFact] {
        self.db.jsx_attributes()
    }

    /// Iterates all JSX attribute facts in deterministic database order.
    pub fn iter(self) -> std::slice::Iter<'a, JsxAttributeFact> {
        self.db.jsx_attributes().iter()
    }

    /// Returns JSX attribute facts for a file without cloning facts.
    pub fn for_file(self, file: FileId) -> impl Iterator<Item = &'a JsxAttributeFact> {
        self.db.jsx_attributes_for_file(file)
    }
}

/// Reserved CFG fact view. Requesting this view currently maps to unsupported `cfg`.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct Cfg<'a> {
    _db: &'a AnalysisDb,
}

/// Reserved call-graph fact view. Requesting this view currently maps to unsupported `call_graph`.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct CallGraph<'a> {
    _db: &'a AnalysisDb,
}

/// Preview event policy view. Requesting this view maps to lightweight `events`.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct Events<'a> {
    db: &'a AnalysisDb,
}

impl<'a> Events<'a> {
    /// Finds events matching `query`.
    ///
    /// Supports call-event matching over syntax-derived function calls, upgrading
    /// to call/refined-call facts when another requested capability already
    /// builds them. Other event kinds remain preview vocabulary until backed facts land.
    pub fn matching(self, query: EventPattern) -> Vec<PolicyViolation> {
        crate::policy_queries::matching_events(self.db, query)
    }
}

/// Preview calls policy view. Requesting this view maps to provider-backed `calls`.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct Calls<'a> {
    db: &'a AnalysisDb,
}

impl<'a> Calls<'a> {
    /// Finds forbidden reachable calls described by `query`.
    ///
    /// Answers bounded reachability over private refined-call and
    /// reachability facts without exposing raw call-graph internals.
    pub fn forbidden_reachable(self, query: ReachQuery) -> Vec<PolicyViolation> {
        crate::policy_queries::forbidden_reachable(self.db, query)
    }
}

/// Preview control-flow policy view. Requesting this view maps to `control_flow`.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct ControlFlow<'a> {
    db: &'a AnalysisDb,
}

impl<'a> ControlFlow<'a> {
    /// Finds events missing a required guard.
    ///
    /// Supports same-function call-event guard checks over private
    /// refined call facts and CFG-backed operation order, with MIR/source
    /// ordering as fallback when CFG facts are absent. Other event families
    /// remain preview vocabulary until backed facts land.
    pub fn missing_guard(self, query: GuardQuery) -> Vec<PolicyViolation> {
        crate::policy_queries::missing_guards(self.db, query)
    }

    /// Decides, per protected operation, whether a required guard covers it.
    ///
    /// Unlike [`ControlFlow::missing_guard`], every matched operation produces
    /// a [`PolicyResult`], so "proved covered", "refuted", and "could not
    /// decide" are separate answers instead of presence or absence in a list.
    ///
    /// The decision is same-function and covers one guard call and one
    /// protected call. With `require_checked_error` the guard's returned error
    /// must also be tested by a nil-comparison branch that dominates the
    /// operation, on an error edge that cannot reach it. See
    /// `docs/facts/control-flow.md` for the reason vocabulary and the
    /// documented limits.
    pub fn guard_outcomes(self, query: GuardQuery) -> Vec<PolicyResult> {
        crate::policy_queries::guard_outcomes(self.db, query)
    }

    /// Finds lifecycle starts missing required cleanup.
    ///
    /// Supports same-function call-event lifecycle checks over private
    /// refined call facts and CFG-backed operation order, with MIR/source
    /// ordering as fallback when CFG facts are absent. Exact path, error-exit,
    /// and interprocedural resource proof remains deferred.
    pub fn missing_cleanup(self, query: LifecycleQuery) -> Vec<PolicyViolation> {
        crate::policy_queries::missing_cleanup(self.db, query)
    }
}

/// Preview data-flow policy view. Requesting this view maps to provider-backed `dataflow`.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct DataFlow<'a> {
    db: &'a AnalysisDb,
}

impl<'a> DataFlow<'a> {
    /// Finds forbidden source-to-sink flows described by `query`.
    ///
    /// Answers bounded source-to-sink queries over private data-flow
    /// facts without exposing raw graph nodes, edges, or solver internals.
    pub fn forbidden(self, query: FlowQuery) -> Vec<PolicyViolation> {
        crate::policy_queries::forbidden_flows(self.db, query)
    }
}

/// Reserved coverage fact view. Requesting this view currently maps to unsupported `coverage_facts`.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct CoverageFacts<'a> {
    db: &'a AnalysisDb,
}

impl<'a> CoverageFacts<'a> {
    /// Returns all coverage facts currently stored in the database.
    pub fn all(self) -> &'a [CoverageFact] {
        self.db.coverage()
    }

    /// Iterates all coverage facts currently stored in the database.
    pub fn iter(self) -> std::slice::Iter<'a, CoverageFact> {
        self.db.coverage().iter()
    }
}

/// Reserved test-suite metric view. Requesting this view currently maps to unsupported `test_suite_metrics`.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct TestSuiteMetrics<'a> {
    _db: &'a AnalysisDb,
}

/// Diff-to-target-ref fact view for `polint review` rules.
///
/// Requesting this view maps to the `changeset` capability. The view is empty
/// under `polint check` (no diff is injected) and is populated only when
/// `polint review <ref>` runs, so a review rule can target changed paths and
/// changed line ranges as ordinary Rust code.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct ChangedFiles<'a> {
    db: &'a AnalysisDb,
}

impl<'a> ChangedFiles<'a> {
    fn facts(self) -> Option<&'a ReviewChangeset> {
        self.db.changeset()
    }

    /// Iterates the changed files in stored order.
    ///
    /// `polint review` populates the changeset path-sorted (the git module sorts
    /// before injection), so iteration is deterministic in practice.
    pub fn iter(self) -> impl Iterator<Item = ChangedFileRef<'a>> {
        self.facts()
            .map(|facts| facts.files.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(|file| ChangedFileRef { file })
    }

    /// Returns true when no files changed (always true under `polint check`).
    pub fn is_empty(self) -> bool {
        self.facts().is_none_or(|facts| facts.files.is_empty())
    }

    /// Returns true when `path` (exact repo-relative, `/`-normalized) changed.
    pub fn contains_path(self, path: &str) -> bool {
        self.iter().any(|file| file.path() == path)
    }

    /// Returns true when any changed path matches the glob pattern.
    pub fn matches_glob(self, glob: &str) -> bool {
        self.iter().any(|file| file.matches_glob(glob))
    }

    /// Returns the new-side changed line ranges for `path`.
    ///
    /// Ranges are inclusive and 1-based. Returns `&[]` when the path did not
    /// change or was deleted (deleted files carry no new-side lines).
    pub fn lines_for(self, path: &str) -> &'a [(u32, u32)] {
        self.facts()
            .and_then(|facts| facts.files.iter().find(|file| file.path == path))
            .map(|file| file.new_line_ranges.as_slice())
            .unwrap_or(&[])
    }
}

/// A single changed file in a `polint review` diff.
///
/// Yielded by [`ChangedFiles::iter`]; a thin borrow over one changed entry.
#[derive(Clone, Copy)]
pub struct ChangedFileRef<'a> {
    file: &'a ChangedFile,
}

impl<'a> ChangedFileRef<'a> {
    /// The repo-relative, `/`-normalized path, matching `Diagnostic.file`.
    pub fn path(self) -> &'a str {
        &self.file.path
    }

    /// How this file changed relative to the target ref.
    pub fn status(self) -> ChangeStatus {
        self.file.status
    }

    /// The new-side changed line ranges (inclusive, 1-based); empty if deleted.
    pub fn lines(self) -> &'a [(u32, u32)] {
        &self.file.new_line_ranges
    }

    /// Returns true when this file's path matches the glob pattern.
    pub fn matches_glob(self, glob: &str) -> bool {
        crate::sdk::scope::glob_matches(glob, &self.file.path)
    }

    /// Returns true when this file was added.
    pub fn is_added(self) -> bool {
        matches!(self.file.status, ChangeStatus::Added)
    }

    /// Returns true when this file was modified.
    pub fn is_modified(self) -> bool {
        matches!(self.file.status, ChangeStatus::Modified)
    }

    /// Returns true when this file was deleted.
    pub fn is_deleted(self) -> bool {
        matches!(self.file.status, ChangeStatus::Deleted)
    }

    /// Returns true when this file was renamed.
    pub fn is_renamed(self) -> bool {
        matches!(self.file.status, ChangeStatus::Renamed)
    }
}

/// Hidden trait used by the `#[polint::rule]` macro to construct fact views.
#[doc(hidden)]
pub trait FactView<'a>: Sized {
    /// Builds a view for the current analysis database.
    fn build(db: &'a AnalysisDb) -> Self;
}

macro_rules! impl_fact_view {
    ($ty:ident) => {
        impl<'a> FactView<'a> for $ty<'a> {
            fn build(db: &'a AnalysisDb) -> Self {
                Self { db }
            }
        }
    };
    ($ty:ident, $field:ident) => {
        impl<'a> FactView<'a> for $ty<'a> {
            fn build(db: &'a AnalysisDb) -> Self {
                Self { $field: db }
            }
        }
    };
}

impl_fact_view!(SourceFiles);
impl_fact_view!(Packages);
impl_fact_view!(Functions);
impl_fact_view!(FileMetrics);
impl_fact_view!(FunctionMetrics);
impl_fact_view!(ComplexityMetrics);
impl_fact_view!(Imports);
impl_fact_view!(ResolvedImports);
impl_fact_view!(ModuleGraphFacts);
impl_fact_view!(Symbols);
impl_fact_view!(References);
impl_fact_view!(BranchObligations);
impl_fact_view!(GoTests);
impl_fact_view!(GoTypeDecls);
impl_fact_view!(TsComponents);
impl_fact_view!(TsClasses);
impl_fact_view!(StringLiterals);
impl_fact_view!(JsxAttributes);
impl_fact_view!(CoverageFacts);
impl_fact_view!(ChangedFiles);
impl_fact_view!(Cfg, _db);
impl_fact_view!(CallGraph, _db);
impl_fact_view!(Events);
impl_fact_view!(Calls);
impl_fact_view!(ControlFlow);
impl_fact_view!(DataFlow);
impl_fact_view!(TestSuiteMetrics, _db);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        AnalysisDb, ComplexityMetricFact, DefinitionFact, DefinitionId, DefinitionKind,
        FileMetricFact, FunctionFact, FunctionId, FunctionMetricFact, ImportFact, ModuleEdge,
        ModuleEdgeId, ModuleEdgeKind, ModuleNode, ModuleNodeId, ModuleNodeKind, ReferenceFact,
        ReferenceId, ReferenceKind, ResolutionPrecision, ResolutionStatus, ResolvedImportFact,
        ResolvedImportId, Span, SymbolFact, SymbolId, SymbolKind, SymbolNamespace, SymbolPrecision,
        SymbolResolutionStatus, UnresolvedReason,
    };
    use std::path::PathBuf;

    #[test]
    fn metric_views_query_by_file_function_language_and_threshold() {
        let mut db = AnalysisDb::new();
        let go_file = db.add_file(
            PathBuf::from("src/router.go"),
            "src/router.go".to_string(),
            "package app\nfunc route() {}\n".to_string(),
        );
        let ts_file = db.add_file(
            PathBuf::from("src/panel.ts"),
            "src/panel.ts".to_string(),
            "export function Panel() {}\n".to_string(),
        );
        let route_function = FunctionId::from_raw(0);
        let panel_function = FunctionId::from_raw(1);
        let route_span = Span::new(go_file, 12, 27, 2, 1, 2, 16);
        let panel_span = Span::new(ts_file, 0, 26, 1, 1, 1, 27);

        db.replace_metric_facts(
            vec![
                FileMetricFact::new(go_file, Language::Go, 2, 2, 28, 1),
                FileMetricFact::new(ts_file, Language::TypeScript, 1, 1, 27, 1),
            ],
            vec![
                FunctionMetricFact::new(
                    route_function,
                    go_file,
                    "route".to_string(),
                    route_span.clone(),
                    Language::Go,
                    1,
                    15,
                ),
                FunctionMetricFact::new(
                    panel_function,
                    ts_file,
                    "Panel".to_string(),
                    panel_span.clone(),
                    Language::TypeScript,
                    1,
                    26,
                ),
            ],
            vec![
                ComplexityMetricFact::new(
                    route_function,
                    go_file,
                    "route".to_string(),
                    route_span,
                    Language::Go,
                    1,
                ),
                ComplexityMetricFact::new(
                    panel_function,
                    ts_file,
                    "Panel".to_string(),
                    panel_span,
                    Language::TypeScript,
                    3,
                ),
            ],
        );

        let files = FileMetrics::build(&db);
        let functions = FunctionMetrics::build(&db);
        let complexity = ComplexityMetrics::build(&db);

        assert_eq!(
            files.get(go_file).map(|metric| metric.function_count),
            Some(1)
        );
        assert_eq!(
            files
                .for_language(Language::TypeScript)
                .map(|metric| metric.file)
                .collect::<Vec<_>>(),
            vec![ts_file]
        );
        assert_eq!(
            files
                .over_line_count(1)
                .map(|metric| metric.file)
                .collect::<Vec<_>>(),
            vec![go_file]
        );
        assert_eq!(
            files
                .over_byte_count(27)
                .map(|metric| metric.file)
                .collect::<Vec<_>>(),
            vec![go_file]
        );
        assert!(files.over_function_count(1).next().is_none());
        assert_eq!(
            functions
                .for_file(go_file)
                .map(|metric| metric.name.as_str())
                .collect::<Vec<_>>(),
            vec!["route"]
        );
        assert_eq!(
            functions
                .get(panel_function)
                .map(|metric| metric.name.as_str()),
            Some("Panel")
        );
        assert_eq!(
            functions
                .over_line_count(0)
                .map(|metric| metric.function)
                .collect::<Vec<_>>(),
            vec![route_function, panel_function]
        );
        assert_eq!(
            functions
                .over_byte_count(20)
                .map(|metric| metric.function)
                .collect::<Vec<_>>(),
            vec![panel_function]
        );
        assert_eq!(
            complexity
                .for_file(ts_file)
                .map(|metric| metric.cyclomatic_complexity)
                .collect::<Vec<_>>(),
            vec![3]
        );
        assert_eq!(
            complexity
                .over(1)
                .map(|metric| metric.function)
                .collect::<Vec<_>>(),
            vec![panel_function]
        );
        assert_eq!(
            complexity
                .over_complexity(1)
                .map(|metric| metric.function)
                .collect::<Vec<_>>(),
            vec![panel_function]
        );
    }

    #[test]
    fn function_file_index_preserves_sparse_order_and_refreshes_after_mutation() {
        let mut db = AnalysisDb::new();
        let first_file = db.add_file(
            PathBuf::from("src/first.go"),
            "src/first.go".to_string(),
            "package first\n".to_string(),
        );
        let second_file = db.add_file(
            PathBuf::from("src/second.go"),
            "src/second.go".to_string(),
            "package second\n".to_string(),
        );
        let first_span = Span::point(first_file, 1, 1);
        let second_span = Span::point(second_file, 1, 1);

        db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            first_file,
            "first".to_string(),
            first_span.clone(),
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            second_file,
            "second".to_string(),
            second_span,
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            first_file,
            "third".to_string(),
            first_span.clone(),
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));

        let first_query = Functions::build(&db)
            .for_file(first_file)
            .map(|function| function.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(first_query, ["first", "third"]);

        db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            first_file,
            "fourth".to_string(),
            first_span,
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));

        let functions = Functions::build(&db);
        let refreshed = functions.for_file(first_file).collect::<Vec<_>>();
        assert_eq!(
            refreshed
                .iter()
                .map(|function| function.name.as_str())
                .collect::<Vec<_>>(),
            ["first", "third", "fourth"]
        );
        assert!(std::ptr::eq(refreshed[1], &functions.all()[2]));
    }

    #[test]
    fn go_syntax_file_indexes_preserve_sparse_storage_order_after_finalization() {
        let mut db = AnalysisDb::new();
        let first_file = db.add_file(
            PathBuf::from("src/first.go"),
            "src/first.go".to_string(),
            "package first\n".to_string(),
        );
        let second_file = db.add_file(
            PathBuf::from("src/second.go"),
            "src/second.go".to_string(),
            "package second\n".to_string(),
        );
        let first_span = Span::point(first_file, 1, 1);
        let second_span = Span::point(second_file, 1, 1);

        for (file, path, span) in [
            (first_file, "first-a", first_span.clone()),
            (second_file, "second", second_span.clone()),
            (first_file, "first-b", first_span.clone()),
        ] {
            db.push_import(ImportFact::new(
                ImportId::from_raw(99),
                file,
                None,
                path.to_string(),
                span,
                Language::Go,
            ));
        }
        for (file, condition, span) in [
            (first_file, "first-a", first_span.clone()),
            (second_file, "second", second_span.clone()),
            (first_file, "first-b", first_span.clone()),
        ] {
            db.push_branch(BranchObligation::new(
                crate::core::BranchId::from_raw(99),
                None,
                file,
                span,
                condition.to_string(),
                "true".to_string(),
                false,
                condition.to_string(),
            ));
        }
        for (file, name, span) in [
            (first_file, "first-a", first_span.clone()),
            (second_file, "second", second_span),
            (first_file, "first-b", first_span),
        ] {
            db.push_test(TestFact::new(
                file,
                None,
                name.to_string(),
                span,
                Vec::new(),
                0,
                0,
                Vec::new(),
                0,
            ));
        }
        db.finish_all_fact_meta_insertions();

        assert_eq!(
            (
                Imports::build(&db)
                    .for_file(first_file)
                    .map(|fact| fact.path.as_str())
                    .collect::<Vec<_>>(),
                BranchObligations::build(&db)
                    .for_file(first_file)
                    .map(|fact| fact.condition_text.as_str())
                    .collect::<Vec<_>>(),
                GoTests::build(&db)
                    .for_file(first_file)
                    .map(|fact| fact.name.as_str())
                    .collect::<Vec<_>>(),
            ),
            (
                vec!["first-a", "first-b"],
                vec!["first-a", "first-b"],
                vec!["first-a", "first-b"],
            )
        );
    }

    #[test]
    fn ts_syntax_file_indexes_preserve_sparse_storage_order_after_finalization() {
        let mut db = AnalysisDb::new();
        let first_file = db.add_file(
            PathBuf::from("src/first.tsx"),
            "src/first.tsx".to_string(),
            "export const First = () => null;\n".to_string(),
        );
        let second_file = db.add_file(
            PathBuf::from("src/second.tsx"),
            "src/second.tsx".to_string(),
            "export const Second = () => null;\n".to_string(),
        );
        let first_span = Span::point(first_file, 1, 1);
        let second_span = Span::point(second_file, 1, 1);

        for (file, name, span) in [
            (first_file, "first-a", first_span.clone()),
            (second_file, "second", second_span.clone()),
            (first_file, "first-b", first_span.clone()),
        ] {
            db.push_ts_component(TsComponentFact::new(file, None, name.to_string(), span));
        }
        for (file, name, span) in [
            (first_file, "first-a", first_span.clone()),
            (second_file, "second", second_span.clone()),
            (first_file, "first-b", first_span.clone()),
        ] {
            db.push_ts_class(TsClassFact::new(file, name.to_string(), span, true, true));
        }
        for (file, value, span) in [
            (first_file, "first-a", first_span.clone()),
            (second_file, "second", second_span.clone()),
            (first_file, "first-b", first_span.clone()),
        ] {
            db.push_string_literal(StringLiteralFact::new(
                file,
                value.to_string(),
                span,
                Language::Tsx,
            ));
        }
        for (file, name, span) in [
            (first_file, "first-a", first_span.clone()),
            (second_file, "second", second_span),
            (first_file, "first-b", first_span),
        ] {
            db.push_jsx_attribute(JsxAttributeFact::new(file, name.to_string(), None, span));
        }
        db.finish_all_fact_meta_insertions();

        assert_eq!(
            (
                TsComponents::build(&db)
                    .for_file(first_file)
                    .map(|fact| fact.name.as_str())
                    .collect::<Vec<_>>(),
                TsClasses::build(&db)
                    .for_file(first_file)
                    .map(|fact| fact.name.as_str())
                    .collect::<Vec<_>>(),
                StringLiterals::build(&db)
                    .for_file(first_file)
                    .map(|fact| fact.value.as_str())
                    .collect::<Vec<_>>(),
                JsxAttributes::build(&db)
                    .for_file(first_file)
                    .map(|fact| fact.name.as_str())
                    .collect::<Vec<_>>(),
            ),
            (
                vec!["first-a", "first-b"],
                vec!["first-a", "first-b"],
                vec!["first-a", "first-b"],
                vec!["first-a", "first-b"],
            )
        );
    }

    #[test]
    fn metric_id_indexes_follow_ids_instead_of_fact_positions() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "function first() {}\nfunction second() {}\n".to_string(),
        );
        let first_span = Span::new(file, 0, 19, 1, 1, 1, 20);
        let second_span = Span::new(file, 20, 40, 2, 1, 2, 21);
        let first = db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            file,
            "first".to_string(),
            first_span.clone(),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        let second = db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            file,
            "second".to_string(),
            second_span.clone(),
            Language::TypeScript,
            false,
            true,
            4,
            Vec::new(),
        ));

        db.replace_metric_facts(
            Vec::new(),
            vec![
                FunctionMetricFact::new(
                    second,
                    file,
                    "second".to_string(),
                    second_span.clone(),
                    Language::TypeScript,
                    1,
                    20,
                ),
                FunctionMetricFact::new(
                    first,
                    file,
                    "first".to_string(),
                    first_span.clone(),
                    Language::TypeScript,
                    1,
                    19,
                ),
            ],
            vec![
                ComplexityMetricFact::new(
                    second,
                    file,
                    "second".to_string(),
                    second_span,
                    Language::TypeScript,
                    4,
                ),
                ComplexityMetricFact::new(
                    first,
                    file,
                    "first".to_string(),
                    first_span,
                    Language::TypeScript,
                    1,
                ),
            ],
        );
        db.finish_all_fact_meta_insertions();

        let function_metrics = FunctionMetrics::build(&db);
        let complexity_metrics = ComplexityMetrics::build(&db);
        assert_eq!(
            function_metrics
                .get(first)
                .map(|metric| metric.name.as_str()),
            Some("first")
        );
        assert_eq!(
            complexity_metrics
                .get(second)
                .map(|metric| metric.cyclomatic_complexity),
            Some(4)
        );
        assert!(std::ptr::eq(
            function_metrics.get(first).unwrap(),
            &function_metrics.all()[1]
        ));
    }

    #[test]
    fn module_graph_sdk_views_resolved_imports_queries() {
        let mut db = AnalysisDb::new();
        let source_file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "import { Button } from './button';\nimport missing from './missing';\n".to_string(),
        );
        let target_file = db.add_file(
            PathBuf::from("src/button.ts"),
            "src/button.ts".to_string(),
            "export function Button() {}\n".to_string(),
        );
        let span = Span::point(source_file, 1, 1);
        let local_import = db.push_import(ImportFact::new(
            crate::core::ImportId::from_raw(99),
            source_file,
            None,
            "./button".to_string(),
            span.clone(),
            crate::core::Language::TypeScript,
        ));
        let missing_import = db.push_import(ImportFact::new(
            crate::core::ImportId::from_raw(99),
            source_file,
            None,
            "./missing".to_string(),
            span,
            crate::core::Language::TypeScript,
        ));

        db.replace_module_graph_facts(
            vec![
                ResolvedImportFact::new(
                    ResolvedImportId::from_raw(99),
                    local_import,
                    source_file,
                    Some(ModuleNodeId::from_raw(1)),
                    ResolutionStatus::Resolved,
                    ResolutionPrecision::ExactFile,
                    None,
                ),
                ResolvedImportFact::new(
                    ResolvedImportId::from_raw(99),
                    missing_import,
                    source_file,
                    None,
                    ResolutionStatus::Unresolved,
                    ResolutionPrecision::None,
                    Some(UnresolvedReason::NotFound),
                ),
            ],
            vec![
                ModuleNode::new(
                    ModuleNodeId::from_raw(99),
                    ModuleNodeKind::File,
                    "src/app.ts".to_string(),
                    Some(source_file),
                    None,
                    Some(crate::core::Language::TypeScript),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(99),
                    ModuleNodeKind::File,
                    "src/button.ts".to_string(),
                    Some(target_file),
                    None,
                    Some(crate::core::Language::TypeScript),
                ),
            ],
            Vec::new(),
        );

        let resolved = ResolvedImports::build(&db);
        assert_eq!(resolved.all().len(), 2);
        assert_eq!(
            resolved.iter().map(|fact| fact.id).collect::<Vec<_>>(),
            vec![ResolvedImportId::from_raw(0), ResolvedImportId::from_raw(1)]
        );
        assert_eq!(
            resolved
                .for_file(source_file)
                .map(|fact| fact.import)
                .collect::<Vec<_>>(),
            vec![local_import, missing_import]
        );
        assert_eq!(
            resolved
                .resolved_for_file(source_file)
                .map(|fact| fact.import)
                .collect::<Vec<_>>(),
            vec![local_import]
        );
        assert_eq!(
            resolved
                .by_specifier("./missing")
                .map(|fact| fact.import)
                .collect::<Vec<_>>(),
            vec![missing_import]
        );
        assert_eq!(
            resolved
                .unresolved()
                .map(|fact| fact.import)
                .collect::<Vec<_>>(),
            vec![missing_import]
        );
        assert!(resolved.dynamic().next().is_none());
        assert!(resolved.unsupported().next().is_none());
        assert_eq!(
            resolved
                .unresolved_for_file(source_file)
                .map(|fact| fact.import)
                .collect::<Vec<_>>(),
            vec![missing_import]
        );
        assert_eq!(
            resolved
                .for_import(local_import)
                .map(|fact| fact.target_node),
            Some(Some(ModuleNodeId::from_raw(1)))
        );
    }

    #[test]
    fn module_graph_sdk_views_graph_queries_are_deterministic() {
        let mut db = AnalysisDb::new();
        let app_file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "import { Button } from './button';\nimport React from 'react';\n".to_string(),
        );
        let button_file = db.add_file(
            PathBuf::from("src/button.ts"),
            "src/button.ts".to_string(),
            "import { token } from './tokens';\nexport function Button() {}\n".to_string(),
        );
        let token_file = db.add_file(
            PathBuf::from("src/tokens.ts"),
            "src/tokens.ts".to_string(),
            "export const token = 'primary';\n".to_string(),
        );
        let app_import = db.push_import(ImportFact::new(
            crate::core::ImportId::from_raw(99),
            app_file,
            None,
            "./button".to_string(),
            Span::point(app_file, 1, 1),
            crate::core::Language::TypeScript,
        ));
        let react_import = db.push_import(ImportFact::new(
            crate::core::ImportId::from_raw(99),
            app_file,
            None,
            "react".to_string(),
            Span::point(app_file, 2, 1),
            crate::core::Language::TypeScript,
        ));
        let token_import = db.push_import(ImportFact::new(
            crate::core::ImportId::from_raw(99),
            button_file,
            None,
            "./tokens".to_string(),
            Span::point(button_file, 1, 1),
            crate::core::Language::TypeScript,
        ));

        db.replace_module_graph_facts(
            vec![
                ResolvedImportFact::new(
                    ResolvedImportId::from_raw(99),
                    app_import,
                    app_file,
                    Some(ModuleNodeId::from_raw(1)),
                    ResolutionStatus::Resolved,
                    ResolutionPrecision::ExactFile,
                    None,
                ),
                ResolvedImportFact::new(
                    ResolvedImportId::from_raw(99),
                    react_import,
                    app_file,
                    Some(ModuleNodeId::from_raw(3)),
                    ResolutionStatus::External,
                    ResolutionPrecision::ExternalPackage,
                    None,
                ),
                ResolvedImportFact::new(
                    ResolvedImportId::from_raw(99),
                    token_import,
                    button_file,
                    Some(ModuleNodeId::from_raw(2)),
                    ResolutionStatus::Resolved,
                    ResolutionPrecision::ExactFile,
                    None,
                ),
            ],
            vec![
                ModuleNode::new(
                    ModuleNodeId::from_raw(99),
                    ModuleNodeKind::File,
                    "src/app.ts".to_string(),
                    Some(app_file),
                    None,
                    Some(crate::core::Language::TypeScript),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(99),
                    ModuleNodeKind::File,
                    "src/button.ts".to_string(),
                    Some(button_file),
                    None,
                    Some(crate::core::Language::TypeScript),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(99),
                    ModuleNodeKind::File,
                    "src/tokens.ts".to_string(),
                    Some(token_file),
                    None,
                    Some(crate::core::Language::TypeScript),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(99),
                    ModuleNodeKind::External,
                    "react".to_string(),
                    None,
                    None,
                    Some(crate::core::Language::TypeScript),
                ),
            ],
            vec![
                ModuleEdge::new(
                    ModuleEdgeId::from_raw(99),
                    ModuleNodeId::from_raw(0),
                    ModuleNodeId::from_raw(1),
                    Some(app_import),
                    Some(ResolvedImportId::from_raw(0)),
                    ModuleEdgeKind::Imports,
                    ResolutionStatus::Resolved,
                ),
                ModuleEdge::new(
                    ModuleEdgeId::from_raw(99),
                    ModuleNodeId::from_raw(0),
                    ModuleNodeId::from_raw(3),
                    Some(react_import),
                    Some(ResolvedImportId::from_raw(1)),
                    ModuleEdgeKind::DependsOn,
                    ResolutionStatus::External,
                ),
                ModuleEdge::new(
                    ModuleEdgeId::from_raw(99),
                    ModuleNodeId::from_raw(1),
                    ModuleNodeId::from_raw(2),
                    Some(token_import),
                    Some(ResolvedImportId::from_raw(2)),
                    ModuleEdgeKind::Imports,
                    ResolutionStatus::Resolved,
                ),
            ],
        );

        let graph = ModuleGraphFacts::build(&db);
        assert_eq!(graph.nodes().len(), 4);
        assert_eq!(graph.edges().len(), 3);
        assert_eq!(
            graph.node_for_file(app_file),
            Some(ModuleNodeId::from_raw(0))
        );
        assert_eq!(
            graph
                .nodes_for_package("src/button.ts")
                .map(|node| node.id)
                .collect::<Vec<_>>(),
            vec![ModuleNodeId::from_raw(1)]
        );
        assert_eq!(
            graph
                .edges_from_file(app_file)
                .map(|edge| edge.id)
                .collect::<Vec<_>>(),
            vec![ModuleEdgeId::from_raw(0), ModuleEdgeId::from_raw(1)]
        );
        assert_eq!(
            graph
                .imports_between(app_file, button_file)
                .map(|edge| edge.id)
                .collect::<Vec<_>>(),
            vec![ModuleEdgeId::from_raw(0)]
        );
        assert_eq!(
            graph
                .outgoing(ModuleNodeId::from_raw(0))
                .map(|edge| edge.to)
                .collect::<Vec<_>>(),
            vec![ModuleNodeId::from_raw(1), ModuleNodeId::from_raw(3)]
        );
        assert_eq!(
            graph
                .incoming(ModuleNodeId::from_raw(2))
                .map(|edge| edge.from)
                .collect::<Vec<_>>(),
            vec![ModuleNodeId::from_raw(1)]
        );
        assert_eq!(
            graph.dependency_status(&graph.edges()[1]),
            ResolutionStatus::External
        );
        assert_eq!(
            graph.reachable_from(ModuleNodeId::from_raw(0)),
            vec![
                ModuleNodeId::from_raw(1),
                ModuleNodeId::from_raw(3),
                ModuleNodeId::from_raw(2)
            ]
        );
    }

    #[test]
    fn symbol_sdk_views_query_borrowed_facts_deterministically() {
        let mut db = AnalysisDb::new();
        let app_file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "export function Button() { return theme; }\n".to_string(),
        );
        let theme_file = db.add_file(
            PathBuf::from("src/theme.ts"),
            "src/theme.ts".to_string(),
            "export const theme = {};\n".to_string(),
        );
        let button = SymbolId::from_raw(10);
        let theme = SymbolId::from_raw(20);
        let interner = db.stable_key_interner();

        db.replace_symbol_graph_facts(
            vec![
                SymbolFact::new(
                    button,
                    Language::TypeScript,
                    "Button".to_string(),
                    "src/app.ts::Button".to_string(),
                    SymbolKind::Function,
                    SymbolNamespace::Value,
                    Some(app_file),
                    None,
                    Some(ModuleNodeId::from_raw(0)),
                    None,
                    Some(Span::point(app_file, 1, 1)),
                    true,
                    interner.intern("ts|src/app.ts|Button".to_string()),
                    SymbolPrecision::ExactLocal,
                ),
                SymbolFact::new(
                    theme,
                    Language::TypeScript,
                    "theme".to_string(),
                    "src/theme.ts::theme".to_string(),
                    SymbolKind::Constant,
                    SymbolNamespace::Value,
                    Some(theme_file),
                    None,
                    Some(ModuleNodeId::from_raw(1)),
                    None,
                    Some(Span::point(theme_file, 1, 1)),
                    true,
                    interner.intern("ts|src/theme.ts|theme".to_string()),
                    SymbolPrecision::ModuleLinked,
                ),
            ],
            vec![DefinitionFact::new(
                DefinitionId::from_raw(30),
                button,
                Language::TypeScript,
                "Button".to_string(),
                "src/app.ts::Button".to_string(),
                DefinitionKind::Declaration,
                SymbolNamespace::Value,
                Some(app_file),
                None,
                Some(ModuleNodeId::from_raw(0)),
                None,
                Some(Span::point(app_file, 1, 1)),
                true,
                true,
                interner.intern("ts|src/app.ts|definition|Button".to_string()),
                SymbolPrecision::ExactLocal,
            )],
            {
                let mut references = vec![
                    ReferenceFact::new(
                        ReferenceId::from_raw(40),
                        Language::TypeScript,
                        "theme".to_string(),
                        "src/theme.ts::theme".to_string(),
                        ReferenceKind::Read,
                        SymbolNamespace::Value,
                        Some(app_file),
                        None,
                        Some(ModuleNodeId::from_raw(0)),
                        Some(button),
                        Some(Span::point(app_file, 1, 28)),
                        Some(theme),
                        Vec::new(),
                        interner.intern("ts|src/app.ts|reference|theme".to_string()),
                        SymbolResolutionStatus::Resolved,
                        SymbolPrecision::ModuleLinked,
                    ),
                    ReferenceFact::new(
                        ReferenceId::from_raw(50),
                        Language::TypeScript,
                        "missing".to_string(),
                        "missing".to_string(),
                        ReferenceKind::Read,
                        SymbolNamespace::Value,
                        Some(app_file),
                        None,
                        Some(ModuleNodeId::from_raw(0)),
                        Some(button),
                        Some(Span::point(app_file, 1, 35)),
                        None,
                        Vec::new(),
                        interner.intern("ts|src/app.ts|reference|missing".to_string()),
                        SymbolResolutionStatus::Unresolved,
                        SymbolPrecision::Unresolved,
                    ),
                    ReferenceFact::new(
                        ReferenceId::from_raw(60),
                        Language::TypeScript,
                        "ambiguous".to_string(),
                        "ambiguous".to_string(),
                        ReferenceKind::Read,
                        SymbolNamespace::Value,
                        Some(app_file),
                        None,
                        Some(ModuleNodeId::from_raw(0)),
                        Some(button),
                        Some(Span::point(app_file, 1, 44)),
                        None,
                        vec![button, theme],
                        interner.intern("ts|src/app.ts|reference|ambiguous".to_string()),
                        SymbolResolutionStatus::Ambiguous,
                        SymbolPrecision::Ambiguous,
                    ),
                ];
                references.rotate_right(1);
                references
            },
        );

        let symbols = Symbols::build(&db);
        let references = References::build(&db);

        assert_eq!(symbols.all().len(), 2);
        assert_eq!(
            symbols.iter().map(|symbol| symbol.id).collect::<Vec<_>>(),
            vec![button, theme]
        );
        assert!(std::ptr::eq(
            symbols.get(button).unwrap(),
            &symbols.all()[0]
        ));
        assert_eq!(
            symbols
                .for_file(app_file)
                .map(|symbol| symbol.id)
                .collect::<Vec<_>>(),
            vec![button]
        );
        assert_eq!(
            symbols
                .by_name("theme")
                .map(|symbol| symbol.id)
                .collect::<Vec<_>>(),
            vec![theme]
        );
        assert_eq!(
            symbols
                .by_kind(SymbolKind::Function)
                .map(|symbol| symbol.id)
                .collect::<Vec<_>>(),
            vec![button]
        );
        assert_eq!(
            symbols
                .exported_by_name("Button")
                .map(|symbol| symbol.id)
                .collect::<Vec<_>>(),
            vec![button]
        );
        assert_eq!(
            symbols
                .definitions_in_file(app_file)
                .map(|definition| definition.id)
                .collect::<Vec<_>>(),
            vec![DefinitionId::from_raw(30)]
        );
        assert_eq!(
            symbols.definition(button).map(|definition| definition.id),
            Some(DefinitionId::from_raw(30))
        );
        assert_eq!(
            symbols
                .definitions(button)
                .map(|definition| definition.id)
                .collect::<Vec<_>>(),
            vec![DefinitionId::from_raw(30)]
        );
        assert_eq!(
            symbols
                .exported()
                .map(|symbol| symbol.id)
                .collect::<Vec<_>>(),
            vec![button, theme]
        );

        assert_eq!(references.all().len(), 3);
        assert_eq!(
            references
                .iter()
                .map(|reference| reference.id)
                .collect::<Vec<_>>(),
            vec![
                ReferenceId::from_raw(60),
                ReferenceId::from_raw(40),
                ReferenceId::from_raw(50)
            ]
        );
        assert_eq!(
            references
                .to(theme)
                .map(|reference| reference.id)
                .collect::<Vec<_>>(),
            vec![ReferenceId::from_raw(40)]
        );
        assert_eq!(
            references
                .resolved()
                .map(|reference| reference.id)
                .collect::<Vec<_>>(),
            vec![ReferenceId::from_raw(40)]
        );
        assert_eq!(
            references
                .by_name("theme")
                .map(|reference| reference.id)
                .collect::<Vec<_>>(),
            vec![ReferenceId::from_raw(40)]
        );
        assert_eq!(
            references
                .to_any(symbols.exported_by_name("theme"))
                .map(|reference| reference.id)
                .collect::<Vec<_>>(),
            vec![ReferenceId::from_raw(40)]
        );
        assert_eq!(
            references
                .for_file(app_file)
                .map(|reference| reference.id)
                .collect::<Vec<_>>(),
            vec![
                ReferenceId::from_raw(40),
                ReferenceId::from_raw(50),
                ReferenceId::from_raw(60)
            ]
        );
        assert_eq!(
            references
                .unresolved()
                .map(|reference| reference.id)
                .collect::<Vec<_>>(),
            vec![ReferenceId::from_raw(50)]
        );
        assert_eq!(
            references
                .unresolved_by_name("missing")
                .map(|reference| reference.id)
                .collect::<Vec<_>>(),
            vec![ReferenceId::from_raw(50)]
        );
        assert_eq!(
            references
                .ambiguous()
                .map(|reference| reference.id)
                .collect::<Vec<_>>(),
            vec![ReferenceId::from_raw(60)]
        );
    }

    fn changeset_fixture() -> ReviewChangeset {
        // `polint review` injects a path-sorted changeset (the git module sorts
        // before injection), so the fixture mirrors that stored order.
        ReviewChangeset {
            files: vec![
                ChangedFile {
                    path: "db/migrations/0001_init.sql".to_string(),
                    status: ChangeStatus::Added,
                    new_line_ranges: vec![(1, 8)],
                },
                ChangedFile {
                    path: "src/legacy.go".to_string(),
                    status: ChangeStatus::Deleted,
                    new_line_ranges: vec![],
                },
                ChangedFile {
                    path: "src/router.go".to_string(),
                    status: ChangeStatus::Modified,
                    new_line_ranges: vec![(10, 12), (40, 40)],
                },
            ],
        }
    }

    #[test]
    fn changed_files_view_is_empty_on_a_default_db() {
        let db = AnalysisDb::new();
        let changes = ChangedFiles::build(&db);
        assert!(changes.is_empty());
        assert_eq!(changes.iter().count(), 0);
        assert!(!changes.contains_path("src/router.go"));
        assert!(!changes.matches_glob("**/*.go"));
        assert!(changes.lines_for("src/router.go").is_empty());
    }

    #[test]
    fn changed_files_view_reads_injected_changeset() {
        let mut db = AnalysisDb::new();
        db.set_changeset(changeset_fixture());
        let changes = ChangedFiles::build(&db);

        assert!(!changes.is_empty());

        // iter() preserves the stored (path-sorted) order the producer injects.
        let paths: Vec<&str> = changes.iter().map(|file| file.path()).collect();
        assert_eq!(
            paths,
            vec![
                "db/migrations/0001_init.sql",
                "src/legacy.go",
                "src/router.go",
            ]
        );

        // contains_path is exact repo-relative.
        assert!(changes.contains_path("src/router.go"));
        assert!(!changes.contains_path("src/router"));
        assert!(!changes.contains_path("router.go"));

        // matches_glob over any changed path.
        assert!(changes.matches_glob("db/migrations/**"));
        assert!(changes.matches_glob("src/*.go"));
        assert!(!changes.matches_glob("web/**"));

        // lines_for returns new-side ranges; deleted/absent => empty.
        assert_eq!(changes.lines_for("src/router.go"), &[(10, 12), (40, 40)]);
        assert_eq!(changes.lines_for("db/migrations/0001_init.sql"), &[(1, 8)]);
        assert!(changes.lines_for("src/legacy.go").is_empty());
        assert!(changes.lines_for("does/not/exist.go").is_empty());
    }

    #[test]
    fn changed_file_ref_exposes_status_and_predicates() {
        let mut db = AnalysisDb::new();
        db.set_changeset(changeset_fixture());
        let changes = ChangedFiles::build(&db);
        let by_path: std::collections::HashMap<&str, ChangedFileRef<'_>> =
            changes.iter().map(|file| (file.path(), file)).collect();

        let migration = by_path["db/migrations/0001_init.sql"];
        assert_eq!(migration.status(), ChangeStatus::Added);
        assert!(migration.is_added());
        assert!(!migration.is_modified() && !migration.is_deleted() && !migration.is_renamed());
        assert!(migration.matches_glob("db/migrations/**"));
        assert_eq!(migration.lines(), &[(1, 8)]);

        let router = by_path["src/router.go"];
        assert_eq!(router.status(), ChangeStatus::Modified);
        assert!(router.is_modified());
        assert!(!router.is_added());

        let legacy = by_path["src/legacy.go"];
        assert_eq!(legacy.status(), ChangeStatus::Deleted);
        assert!(legacy.is_deleted());
        assert!(legacy.lines().is_empty());
    }
}
