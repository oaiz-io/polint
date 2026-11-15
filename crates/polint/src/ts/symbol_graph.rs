use crate::analysis_api::{
    DefinitionKind, FactDatabase, FactFamily, ModuleNode, ReferenceKind, ResolutionStatus,
    ResolvedImportFact, SourceFile, SymbolKind, SymbolNamespace, SymbolPrecision,
    stable_key_from_parts,
};
use crate::analysis_neutral::symbol_graph::{
    LanguageCapabilitySupport, LanguageSymbolOutput, SymbolCapabilityStatus, SymbolGraphRequest,
    model::{DefinitionDraft, ReferenceDraft, SymbolDraft, SymbolGraphBuilder},
    semantic::{
        AliasFact, AliasId, AliasKind, ExportFact, ExportId, ExportKind, ResolutionFact,
        ResolutionId, ResolutionStepKind, ScopeFact, ScopeId, ScopeKind, SemanticImportFact,
        SemanticImportId, SemanticImportKind, SemanticIndexBuilder, SemanticIndexOutput,
        SemanticStatus, StableExportId, StableExportIdentity, ValueFunctionFact,
    },
    stable_id::{StableReferenceKey, StableSymbolKey},
};
use crate::internal_core::{
    Diagnostic, DiagnosticRange as TextRange, FileId, Language, ModuleNodeId, Span, StableKeyId,
    SymbolId as PolintSymbolId,
};
use crate::ts::parse::parse_ts_file;
use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast::ast::{
    Argument, AssignmentTarget, BindingIdentifier, BindingPattern, Declaration,
    ExportDefaultDeclarationKind, Expression, ImportDeclarationSpecifier, ImportOrExportKind,
    ModuleExportName, Program, Statement, VariableDeclarationKind,
};
use oxc_semantic::{
    AstNodes, Reference, ReferenceFlags, ReferenceId as OxcReferenceId, Scoping, SemanticBuilder,
    SymbolFlags, SymbolId as OxcSymbolId,
};
use oxc_span::GetSpan;
use std::collections::{BTreeMap, BTreeSet};

/// Request passed by the composition root to the TypeScript/JavaScript symbol adapter.
#[derive(Debug, Clone)]
pub struct TsSymbolOptions {
    pub request: SymbolGraphRequest,
    pub resolved_imports: Vec<ResolvedImportFact>,
    pub module_nodes: Vec<ModuleNode>,
}

const TS_SYMBOL_GRAPH_CAPABILITIES: &[&str] = &["symbols", "references"];
#[derive(Debug, Clone)]
struct TsFileSymbolSummary {
    file: FileId,
    import_aliases: Vec<ImportAlias>,
    exports: BTreeMap<String, Vec<PolintSymbolId>>,
}

impl TsFileSymbolSummary {
    fn new(file: FileId) -> Self {
        Self {
            file,
            import_aliases: Vec::new(),
            exports: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct ImportAlias {
    file: FileId,
    language: Language,
    file_key: String,
    import_path: String,
    import_source_span: Span,
    local_name: String,
    local_span: Span,
    target: ImportAliasTarget,
    namespace: SymbolNamespace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ImportAliasTarget {
    Named(String),
    Default,
    Namespace,
}

pub fn derive_ts_symbols(
    builder: &mut SymbolGraphBuilder,
    db: &dyn FactDatabase,
    options: TsSymbolOptions,
) -> LanguageSymbolOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let files = ts_files(db);
    if files.is_empty() {
        return LanguageSymbolOutput::default();
    }

    let mut output = LanguageSymbolOutput::default();
    let requests_symbols = options.request.symbols;
    let requests_references = options.request.references;
    let languages = files
        .iter()
        .map(|file| file.language)
        .collect::<BTreeSet<_>>();
    for language in languages {
        output
            .capability_support
            .extend(supported_language_support(options.request, language));
    }

    if !requests_symbols && !requests_references {
        return output;
    }

    let mut summaries = Vec::new();
    for file in files {
        summaries.push(derive_ts_file_symbols(
            interner,
            builder,
            &mut output,
            file,
            requests_references,
        ));
    }
    if requests_references {
        add_import_alias_references(
            builder,
            db,
            &options.resolved_imports,
            &options.module_nodes,
            &summaries,
        );
    }

    output
}

fn ts_files(db: &dyn FactDatabase) -> Vec<&SourceFile> {
    let mut files = db
        .files()
        .iter()
        .filter(|file| file.language.is_ts_family())
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    files
}

fn supported_language_support(
    request: SymbolGraphRequest,
    language: Language,
) -> Vec<LanguageCapabilitySupport> {
    TS_SYMBOL_GRAPH_CAPABILITIES
        .iter()
        .filter(|capability| {
            (**capability == "symbols" && request.symbols)
                || (**capability == "references" && request.references)
        })
        .map(|capability| LanguageCapabilitySupport {
            capability: (*capability).to_string(),
            language,
            status: SymbolCapabilityStatus::Supported,
            reason: None,
            hint: None,
        })
        .collect()
}

fn derive_ts_file_symbols(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SymbolGraphBuilder,
    output: &mut LanguageSymbolOutput,
    file: &SourceFile,
    requests_references: bool,
) -> TsFileSymbolSummary {
    let source = file.source.as_ref();
    let allocator = Allocator::default();
    let parsed = parse_ts_file(&allocator, file);
    // Parser diagnostics are emitted only by the primary ts adapter.

    if parsed.is_catastrophic() {
        return TsFileSymbolSummary::new(file.id);
    }
    let symbol_precision = if parsed.fully_parsed {
        SymbolPrecision::ExactLocal
    } else {
        SymbolPrecision::Unsupported
    };

    let import_aliases = collect_import_aliases(file, source, parsed.program());
    let semantic_return = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(parsed.program());
    output.diagnostics.extend(oxc_diagnostics(
        file,
        &semantic_return.errors,
        "semantic/ts",
    ));

    let semantic = semantic_return.semantic;
    let scoping = semantic.scoping();
    let nodes = semantic.nodes();
    output.semantic.extend(derive_ts_semantic_index(
        interner,
        file,
        source,
        parsed.program(),
        scoping,
        nodes,
        requests_references,
    ));
    let export_names = collect_export_names(parsed.program(), scoping);
    let parameter_symbols = collect_parameter_symbols(nodes);
    let mut summary = TsFileSymbolSummary {
        file: file.id,
        import_aliases,
        exports: BTreeMap::new(),
    };
    let mut symbol_ids = BTreeMap::<OxcSymbolId, PolintSymbolId>::new();
    let mut oxc_symbols = scoping.symbol_ids().collect::<Vec<_>>();
    oxc_symbols.sort_by(|left, right| {
        let left_span = scoping.symbol_span(*left);
        let right_span = scoping.symbol_span(*right);
        (
            left_span.start,
            left_span.end,
            scoping.symbol_name(*left),
            left.index(),
        )
            .cmp(&(
                right_span.start,
                right_span.end,
                scoping.symbol_name(*right),
                right.index(),
            ))
    });

    for oxc_symbol in oxc_symbols {
        let name = scoping.symbol_name(oxc_symbol).to_string();
        let flags = scoping.symbol_flags(oxc_symbol);
        let kind = symbol_kind(flags, parameter_symbols.contains(&oxc_symbol));
        let namespace = symbol_namespace(flags);
        let owner_chain = owner_chain(scoping, nodes, scoping.symbol_scope_id(oxc_symbol));
        let qualified_name = qualified_name(&file.relative_path, &owner_chain, &name);
        let primary_span = span_from_oxc(file, scoping.symbol_span(oxc_symbol));
        let is_exported = export_names.contains_key(&oxc_symbol);

        let symbol = builder.add_symbol(SymbolDraft {
            language: file.language,
            name: name.clone(),
            qualified_name: qualified_name.clone(),
            kind,
            namespace,
            file: Some(file.id),
            package: None,
            module: None,
            owner: None,
            module_key: None,
            package_key: None,
            file_key: Some(file.relative_path.clone()),
            owner_chain,
            primary_span: Some(primary_span),
            is_exported,
            precision: symbol_precision,
        });
        symbol_ids.insert(oxc_symbol, symbol);
        if let Some(names) = export_names.get(&oxc_symbol) {
            for export_name in names {
                summary
                    .exports
                    .entry(export_name.clone())
                    .or_default()
                    .push(symbol);
            }
        }

        add_symbol_definitions(
            builder,
            file,
            source,
            scoping,
            oxc_symbol,
            symbol,
            name,
            qualified_name,
            namespace,
            is_exported,
        );
    }

    if requests_references {
        add_ts_references(builder, file, source, scoping, nodes, &symbol_ids);
    }

    summary
}

fn derive_ts_semantic_index(
    interner: &crate::internal_core::StableKeyInterner,
    file: &SourceFile,
    source: &str,
    program: &Program<'_>,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    requests_references: bool,
) -> SemanticIndexOutput {
    let mut builder = SemanticIndexBuilder::new();
    let mut ids_by_stable_key = BTreeMap::<StableKeyId, ScopeId>::new();
    let export_symbol_stable_keys =
        export_symbol_stable_keys(file, source, program, scoping, nodes);

    for scope_id in scoping.scope_descendants_from_root() {
        let node_id = scoping.get_node_id(scope_id);
        let kind = scope_kind_for_ast(nodes.kind(node_id));
        let stable_key = scope_stable_key(interner, file, scoping, nodes, scope_id);
        if ids_by_stable_key.contains_key(&stable_key) {
            continue;
        }
        let parent = scoping.scope_parent_id(scope_id).and_then(|parent| {
            let parent_key = scope_stable_key(interner, file, scoping, nodes, parent);
            ids_by_stable_key.get(&parent_key).copied()
        });
        let id = builder.add_scope(
            interner,
            ScopeFact {
                id: ScopeId(0),
                language: file.language,
                file: Some(file.id),
                package: None,
                module: None,
                parent,
                scope_path: scope_path_for_scope(file, scoping, nodes, scope_id),
                kind,
                stable_key,
                status: SemanticStatus::Resolved,
            },
        );
        ids_by_stable_key.insert(stable_key, id);
    }

    let module_scope_id = ids_by_stable_key
        .get(&scope_stable_key(
            interner,
            file,
            scoping,
            nodes,
            scoping.root_scope_id(),
        ))
        .copied();
    let mut semantic_nodes = nodes
        .iter_enumerated()
        .filter_map(|(node_id, node)| {
            let kind = semantic_scope_kind_for_ast(node.kind())?;
            let span = node.kind().span();
            Some((span.start, span.end, node_id, kind))
        })
        .collect::<Vec<_>>();
    semantic_nodes.sort_by_key(|(start, end, _, kind)| (*start, *end, *kind));

    for (_, _, node_id, kind) in semantic_nodes {
        let scope_path = scope_path_for_node(file, nodes, node_id);
        let stable_key = ScopeFact::stable_key_for(
            interner,
            file.language,
            &scope_path,
            Some(file.relative_path.clone()),
            None,
            None,
            (kind, SemanticStatus::Resolved),
        );
        if ids_by_stable_key.contains_key(&stable_key) {
            continue;
        }
        let parent =
            nearest_semantic_scope_parent(interner, file, nodes, node_id, &ids_by_stable_key)
                .or(module_scope_id);
        let id = builder.add_scope(
            interner,
            ScopeFact {
                id: ScopeId(0),
                language: file.language,
                file: Some(file.id),
                package: None,
                module: None,
                parent,
                scope_path,
                kind,
                stable_key,
                status: SemanticStatus::Resolved,
            },
        );
        ids_by_stable_key.insert(stable_key, id);
    }

    add_ts_semantic_import_export_rows(
        interner,
        &mut builder,
        file,
        source,
        program,
        module_scope_id,
        &export_symbol_stable_keys,
    );
    if requests_references {
        add_ts_semantic_resolution_rows(interner, &mut builder, file, source, scoping, nodes);
    }
    add_ts_value_function_rows(&mut builder, file, program, scoping, nodes);

    builder.finish(interner)
}

/// Records bindings that can only ever hold the function literal they are
/// declared with, so a call through the binding has exactly one callee.
///
/// The initializer must be the function or arrow expression itself; calls,
/// conditionals, wrappers and type assertions around it are not followed.
/// `const` bindings always qualify. `let`/`var` bindings qualify only when they
/// are declared once, never written, and the file has no direct `eval` that
/// could write them. A top-level `let`/`var` also needs import or export syntax
/// in the file: without it the file may run as a classic script, whose global
/// bindings other scripts can reassign.
fn add_ts_value_function_rows(
    builder: &mut SemanticIndexBuilder,
    file: &SourceFile,
    program: &Program<'_>,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
) {
    let mutable_bindings_are_sealed = !scoping.root_unresolved_references().contains_key("eval");
    let root_is_module_scope = program.body.iter().any(Statement::is_module_declaration);
    for node in nodes.iter() {
        let AstKind::VariableDeclarator(declarator) = node.kind() else {
            continue;
        };
        let BindingPattern::BindingIdentifier(binding) = &declarator.id else {
            continue;
        };
        let Some(symbol) = binding.symbol_id.get() else {
            continue;
        };
        let function_span = match &declarator.init {
            Some(Expression::ArrowFunctionExpression(function)) => function.span,
            Some(Expression::FunctionExpression(function)) => function.span,
            _ => continue,
        };
        let single_value = match declarator.kind {
            VariableDeclarationKind::Const => true,
            VariableDeclarationKind::Let | VariableDeclarationKind::Var => {
                mutable_bindings_are_sealed
                    && scoping.symbol_redeclarations(symbol).is_empty()
                    && !scoping.symbol_is_mutated(symbol)
                    && (root_is_module_scope
                        || scoping.symbol_scope_id(symbol) != scoping.root_scope_id())
            }
            VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing => false,
        };
        if !single_value {
            continue;
        }
        builder.add_value_function(ValueFunctionFact {
            binding: span_from_oxc(file, scoping.symbol_span(symbol)),
            function: span_from_oxc(file, function_span),
        });
    }
}

fn export_symbol_stable_keys(
    file: &SourceFile,
    source: &str,
    program: &Program<'_>,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
) -> BTreeMap<String, String> {
    let export_names = collect_export_names(program, scoping);
    let parameter_symbols = collect_parameter_symbols(nodes);
    let mut keys = BTreeMap::new();

    for (oxc_symbol, export_names) in export_names {
        let key =
            ts_symbol_stable_key(file, source, scoping, nodes, oxc_symbol, &parameter_symbols);
        for export_name in export_names {
            keys.insert(export_name, key.clone());
        }
    }

    keys
}

fn ts_symbol_stable_key(
    file: &SourceFile,
    source: &str,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    oxc_symbol: OxcSymbolId,
    parameter_symbols: &BTreeSet<OxcSymbolId>,
) -> String {
    ts_symbol_stable_key_input(file, source, scoping, nodes, oxc_symbol, parameter_symbols)
        .stable_key()
}

fn ts_symbol_stable_key_input(
    file: &SourceFile,
    _source: &str,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    oxc_symbol: OxcSymbolId,
    parameter_symbols: &BTreeSet<OxcSymbolId>,
) -> StableSymbolKey {
    let name = scoping.symbol_name(oxc_symbol).to_string();
    let flags = scoping.symbol_flags(oxc_symbol);
    let kind = symbol_kind(flags, parameter_symbols.contains(&oxc_symbol));
    let namespace = symbol_namespace(flags);
    let owner_chain = owner_chain(scoping, nodes, scoping.symbol_scope_id(oxc_symbol));
    let primary_span = span_from_oxc(file, scoping.symbol_span(oxc_symbol));

    StableSymbolKey::new(
        file.language,
        None,
        None,
        Some(file.relative_path.clone()),
        owner_chain,
        namespace,
        kind,
        name,
        Some(primary_span),
    )
}

fn add_ts_semantic_import_export_rows(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SemanticIndexBuilder,
    file: &SourceFile,
    source: &str,
    program: &Program<'_>,
    module_scope: Option<ScopeId>,
    export_symbol_stable_keys: &BTreeMap<String, String>,
) {
    for statement in &program.body {
        match statement {
            Statement::ImportDeclaration(import) => {
                add_import_declaration_rows(interner, builder, file, module_scope, import);
            }
            Statement::ExportNamedDeclaration(export) => {
                add_export_named_rows(
                    interner,
                    builder,
                    file,
                    module_scope,
                    export_symbol_stable_keys,
                    export,
                );
            }
            Statement::ExportDefaultDeclaration(export) => {
                add_export_row(
                    interner,
                    builder,
                    file,
                    module_scope,
                    "default".to_string(),
                    SymbolNamespace::Value,
                    ExportKind::Default,
                    export_symbol_stable_keys.get("default").cloned(),
                    SemanticStatus::Resolved,
                );
                collect_commonjs_from_export_default(
                    interner,
                    builder,
                    file,
                    source,
                    module_scope,
                    export,
                );
            }
            Statement::ExportAllDeclaration(export) => {
                add_export_row(
                    interner,
                    builder,
                    file,
                    module_scope,
                    module_export_source_name(export.source.value.as_str()),
                    SymbolNamespace::Module,
                    ExportKind::StarReexport,
                    None,
                    SemanticStatus::Unresolved,
                );
                add_alias_row(
                    interner,
                    builder,
                    file,
                    "export:*".to_string(),
                    vec![format!("module:{}", export.source.value.as_str())],
                    AliasKind::ReExport,
                    SemanticStatus::Unresolved,
                );
            }
            Statement::ExpressionStatement(statement) => {
                collect_commonjs_from_expression(
                    interner,
                    builder,
                    file,
                    source,
                    module_scope,
                    &statement.expression,
                );
            }
            Statement::VariableDeclaration(variable) => {
                for declarator in &variable.declarations {
                    if let Some(init) = &declarator.init {
                        collect_commonjs_from_expression(
                            interner,
                            builder,
                            file,
                            source,
                            module_scope,
                            init,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

fn add_import_declaration_rows(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SemanticIndexBuilder,
    file: &SourceFile,
    module_scope: Option<ScopeId>,
    import: &oxc_ast::ast::ImportDeclaration<'_>,
) {
    let import_path = import.source.value.to_string();
    let status = import_status_for_path(&import_path);
    let Some(specifiers) = &import.specifiers else {
        add_import_row(
            interner,
            builder,
            file,
            module_scope,
            import_path,
            None,
            None,
            SymbolNamespace::Module,
            SemanticImportKind::SideEffect,
            status,
        );
        return;
    };

    for specifier in specifiers {
        match specifier {
            ImportDeclarationSpecifier::ImportDefaultSpecifier(default) => {
                let local_name = default.local.name.to_string();
                add_import_row(
                    interner,
                    builder,
                    file,
                    module_scope,
                    import_path.clone(),
                    Some(local_name.clone()),
                    Some("default".to_string()),
                    SymbolNamespace::Value,
                    SemanticImportKind::StaticDefault,
                    status,
                );
                add_import_lookup_rows(interner, builder, file, &local_name, &import_path, status);
            }
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(namespace) => {
                let local_name = namespace.local.name.to_string();
                add_import_row(
                    interner,
                    builder,
                    file,
                    module_scope,
                    import_path.clone(),
                    Some(local_name.clone()),
                    Some("*".to_string()),
                    SymbolNamespace::Namespace,
                    SemanticImportKind::StaticNamespace,
                    status,
                );
                add_import_lookup_rows(interner, builder, file, &local_name, &import_path, status);
            }
            ImportDeclarationSpecifier::ImportSpecifier(named) => {
                let is_type_import = matches!(import.import_kind, ImportOrExportKind::Type)
                    || matches!(named.import_kind, ImportOrExportKind::Type);
                let local_name = named.local.name.to_string();
                add_import_row(
                    interner,
                    builder,
                    file,
                    module_scope,
                    import_path.clone(),
                    Some(local_name.clone()),
                    Some(module_export_name_text(&named.imported)),
                    if is_type_import {
                        SymbolNamespace::Type
                    } else {
                        SymbolNamespace::Value
                    },
                    if is_type_import {
                        SemanticImportKind::TypeOnly
                    } else {
                        SemanticImportKind::StaticNamed
                    },
                    status,
                );
                add_alias_row(
                    interner,
                    builder,
                    file,
                    format!("import:{local_name}"),
                    Vec::new(),
                    if is_type_import {
                        AliasKind::TypeOnly
                    } else {
                        AliasKind::Import
                    },
                    status,
                );
                add_import_lookup_rows(interner, builder, file, &local_name, &import_path, status);
            }
        }
    }
}

fn add_export_named_rows(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SemanticIndexBuilder,
    file: &SourceFile,
    module_scope: Option<ScopeId>,
    export_symbol_stable_keys: &BTreeMap<String, String>,
    export: &oxc_ast::ast::ExportNamedDeclaration<'_>,
) {
    if let Some(source) = &export.source {
        for specifier in &export.specifiers {
            add_export_row(
                interner,
                builder,
                file,
                module_scope,
                module_export_name_text(&specifier.exported),
                SymbolNamespace::Module,
                ExportKind::Namespace,
                None,
                SemanticStatus::Unresolved,
            );
            add_alias_row(
                interner,
                builder,
                file,
                format!("reexport:{}", module_export_name_text(&specifier.exported)),
                vec![format!(
                    "{}:{}",
                    source.value.as_str(),
                    module_export_name_text(&specifier.local)
                )],
                AliasKind::ReExport,
                SemanticStatus::Unresolved,
            );
        }
        return;
    }

    if let Some(declaration) = &export.declaration {
        let mut declarations = Vec::new();
        collect_declaration_symbols(declaration, &mut declarations);
        for (_, name) in declarations {
            add_export_row(
                interner,
                builder,
                file,
                module_scope,
                name.clone(),
                SymbolNamespace::Value,
                ExportKind::Named,
                export_symbol_stable_keys.get(&name).cloned(),
                SemanticStatus::Resolved,
            );
        }
    }
    for specifier in &export.specifiers {
        let export_name = module_export_name_text(&specifier.exported);
        add_export_row(
            interner,
            builder,
            file,
            module_scope,
            export_name.clone(),
            SymbolNamespace::Value,
            ExportKind::Named,
            export_symbol_stable_keys.get(&export_name).cloned(),
            SemanticStatus::Resolved,
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "semantic import rows mirror the normalized internal fact fields"
)]
fn add_import_row(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SemanticIndexBuilder,
    file: &SourceFile,
    scope: Option<ScopeId>,
    import_path: String,
    local_name: Option<String>,
    imported_name: Option<String>,
    namespace: SymbolNamespace,
    kind: SemanticImportKind,
    status: SemanticStatus,
) -> SemanticImportId {
    let stable_key = ts_semantic_import_stable_key(
        interner,
        file,
        &import_path,
        local_name.as_deref(),
        imported_name.as_deref(),
        namespace,
        (kind, status),
    );
    builder.add_semantic_import(
        interner,
        SemanticImportFact {
            id: SemanticImportId(0),
            language: file.language,
            file: Some(file.id),
            package: None,
            module: None,
            scope,
            import_path,
            local_name,
            imported_name,
            namespace,
            kind,
            stable_key,
            status,
        },
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "semantic export rows mirror the normalized internal fact fields"
)]
fn add_export_row(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SemanticIndexBuilder,
    file: &SourceFile,
    scope: Option<ScopeId>,
    export_name: String,
    namespace: SymbolNamespace,
    kind: ExportKind,
    symbol_stable_key: Option<String>,
    status: SemanticStatus,
) -> ExportId {
    let stable_key = ts_export_stable_key(interner, file, &export_name, namespace, kind, status);
    let export = builder.add_export_identity(
        interner,
        ExportFact {
            id: ExportId(0),
            language: file.language,
            file: Some(file.id),
            package: None,
            module: None,
            scope,
            symbol: None,
            export_name: export_name.clone(),
            namespace,
            kind,
            stable_key,
            status,
        },
    );
    if status == SemanticStatus::Resolved
        && matches!(
            kind,
            ExportKind::Named | ExportKind::Default | ExportKind::Namespace
        )
        && let Some(symbol_stable_key) = symbol_stable_key
    {
        add_stable_export_identity(
            interner,
            builder,
            file,
            export,
            &export_name,
            namespace,
            (symbol_stable_key, status),
        );
    }
    export
}

fn add_alias_row(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SemanticIndexBuilder,
    file: &SourceFile,
    source_symbol_stable_key_text: String,
    target_symbol_stable_keys: Vec<String>,
    kind: AliasKind,
    status: SemanticStatus,
) -> AliasId {
    let stable_key = ts_alias_stable_key(
        interner,
        file,
        &source_symbol_stable_key_text,
        &target_symbol_stable_keys,
        kind,
        status,
    );
    builder.add_alias(
        interner,
        AliasFact {
            id: AliasId(0),
            language: file.language,
            file: Some(file.id),
            package: None,
            module: None,
            source_symbol_stable_key: interner.intern(source_symbol_stable_key_text),
            target_symbol_stable_keys: target_symbol_stable_keys
                .into_iter()
                .map(|key| interner.intern(key))
                .collect(),
            kind,
            stable_key,
            status,
        },
    )
}

fn add_resolution_row(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SemanticIndexBuilder,
    file: &SourceFile,
    source_stable_key_text: String,
    target_stable_keys: Vec<String>,
    step: ResolutionStepKind,
    status: SemanticStatus,
) -> ResolutionId {
    let stable_key = ts_resolution_stable_key(
        interner,
        file,
        &source_stable_key_text,
        &target_stable_keys,
        step,
        status,
    );
    builder.add_resolution(
        interner,
        ResolutionFact {
            id: ResolutionId(0),
            language: file.language,
            file: Some(file.id),
            package: None,
            module: None,
            source_stable_key: interner.intern(source_stable_key_text),
            target_stable_keys: target_stable_keys
                .into_iter()
                .map(|key| interner.intern(key))
                .collect(),
            step,
            stable_key,
            status,
        },
    )
}

fn add_import_lookup_rows(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SemanticIndexBuilder,
    file: &SourceFile,
    local_name: &str,
    import_path: &str,
    status: SemanticStatus,
) {
    let source = format!("{}::import::{local_name}", file.relative_path);
    add_resolution_row(
        interner,
        builder,
        file,
        source.clone(),
        Vec::new(),
        ResolutionStepKind::ImportAliasLookup,
        status,
    );
    add_resolution_row(
        interner,
        builder,
        file,
        source,
        vec![format!("module:{import_path}")],
        ResolutionStepKind::ModuleLookup,
        status,
    );
}

fn add_stable_export_identity(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SemanticIndexBuilder,
    file: &SourceFile,
    export: ExportId,
    export_name: &str,
    namespace: SymbolNamespace,
    pair: (String, SemanticStatus),
) -> StableExportId {
    let (symbol_stable_key, status) = pair;
    builder.add_stable_export(
        interner,
        StableExportIdentity {
            id: StableExportId(0),
            export,
            language: file.language,
            package_key: None,
            module_key: Some(file.relative_path.clone()),
            export_name: export_name.to_string(),
            namespace,
            symbol_stable_key: interner.intern(symbol_stable_key),
            generated_discriminator: Some("native".to_string()),
            stable_key: interner.intern(""),
            status,
        },
    )
}

fn ts_semantic_import_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    file: &SourceFile,
    import_path: &str,
    local_name: Option<&str>,
    imported_name: Option<&str>,
    namespace: SymbolNamespace,
    pair: (SemanticImportKind, SemanticStatus),
) -> StableKeyId {
    let (kind, status) = pair;
    stable_key_from_parts(
        interner,
        FactFamily::SemanticImport,
        &[
            ("language", format!("{:?}", file.language)),
            ("file", file.relative_path.clone()),
            ("import_path", import_path.to_string()),
            ("local_name", local_name.unwrap_or("<none>").to_string()),
            (
                "imported_name",
                imported_name.unwrap_or("<none>").to_string(),
            ),
            ("namespace", format!("{namespace:?}")),
            ("kind", format!("{kind:?}")),
            ("status", format!("{status:?}")),
        ],
    )
}

fn ts_export_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    file: &SourceFile,
    export_name: &str,
    namespace: SymbolNamespace,
    kind: ExportKind,
    status: SemanticStatus,
) -> StableKeyId {
    stable_key_from_parts(
        interner,
        FactFamily::Export,
        &[
            ("language", format!("{:?}", file.language)),
            ("file", file.relative_path.clone()),
            ("export_name", export_name.to_string()),
            ("namespace", format!("{namespace:?}")),
            ("kind", format!("{kind:?}")),
            ("status", format!("{status:?}")),
        ],
    )
}

fn ts_alias_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    file: &SourceFile,
    source_symbol_stable_key: &str,
    target_symbol_stable_keys: &[String],
    kind: AliasKind,
    status: SemanticStatus,
) -> StableKeyId {
    let mut targets = target_symbol_stable_keys.to_vec();
    targets.sort();
    targets.dedup();
    stable_key_from_parts(
        interner,
        FactFamily::Alias,
        &[
            ("language", format!("{:?}", file.language)),
            ("file", file.relative_path.clone()),
            ("source", source_symbol_stable_key.to_string()),
            ("targets", targets.join("\n")),
            ("kind", format!("{kind:?}")),
            ("status", format!("{status:?}")),
        ],
    )
}

fn ts_resolution_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    file: &SourceFile,
    source_stable_key: &str,
    target_stable_keys: &[String],
    step: ResolutionStepKind,
    status: SemanticStatus,
) -> StableKeyId {
    let mut targets = target_stable_keys.to_vec();
    targets.sort();
    targets.dedup();
    stable_key_from_parts(
        interner,
        FactFamily::Resolution,
        &[
            ("language", format!("{:?}", file.language)),
            ("file", file.relative_path.clone()),
            ("source", source_stable_key.to_string()),
            ("targets", targets.join("\n")),
            ("step", format!("{step:?}")),
            ("status", format!("{status:?}")),
        ],
    )
}

fn collect_commonjs_from_export_default(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SemanticIndexBuilder,
    file: &SourceFile,
    source: &str,
    module_scope: Option<ScopeId>,
    export: &oxc_ast::ast::ExportDefaultDeclaration<'_>,
) {
    if let ExportDefaultDeclarationKind::CallExpression(call) = &export.declaration {
        collect_commonjs_from_call(interner, builder, file, source, module_scope, call);
    }
}

fn collect_commonjs_from_expression(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SemanticIndexBuilder,
    file: &SourceFile,
    source: &str,
    module_scope: Option<ScopeId>,
    expression: &Expression<'_>,
) {
    match expression {
        Expression::CallExpression(call) => {
            collect_commonjs_from_call(interner, builder, file, source, module_scope, call);
            for argument in &call.arguments {
                if let Some(expression) = argument.as_expression() {
                    collect_commonjs_from_expression(
                        interner,
                        builder,
                        file,
                        source,
                        module_scope,
                        expression,
                    );
                }
            }
        }
        Expression::ImportExpression(import) => {
            let (import_path, status) = match &import.source {
                Expression::StringLiteral(path) => (
                    path.value.to_string(),
                    import_status_for_path(path.value.as_str()),
                ),
                _ => ("<dynamic>".to_string(), SemanticStatus::Dynamic),
            };
            add_import_row(
                interner,
                builder,
                file,
                module_scope,
                import_path,
                None,
                None,
                SymbolNamespace::Module,
                SemanticImportKind::DynamicImport,
                status,
            );
        }
        Expression::AssignmentExpression(assignment) => {
            if let Some(kind) = commonjs_export_kind(&assignment.left) {
                add_export_row(
                    interner,
                    builder,
                    file,
                    module_scope,
                    commonjs_export_name(&assignment.left),
                    SymbolNamespace::Value,
                    kind,
                    None,
                    SemanticStatus::Unsupported,
                );
            }
            collect_commonjs_from_expression(
                interner,
                builder,
                file,
                source,
                module_scope,
                &assignment.right,
            );
        }
        Expression::AwaitExpression(expression) => {
            collect_commonjs_from_expression(
                interner,
                builder,
                file,
                source,
                module_scope,
                &expression.argument,
            );
        }
        Expression::ParenthesizedExpression(expression) => {
            collect_commonjs_from_expression(
                interner,
                builder,
                file,
                source,
                module_scope,
                &expression.expression,
            );
        }
        Expression::TSAsExpression(expression) => {
            collect_commonjs_from_expression(
                interner,
                builder,
                file,
                source,
                module_scope,
                &expression.expression,
            );
        }
        Expression::TSSatisfiesExpression(expression) => {
            collect_commonjs_from_expression(
                interner,
                builder,
                file,
                source,
                module_scope,
                &expression.expression,
            );
        }
        Expression::TSNonNullExpression(expression) => {
            collect_commonjs_from_expression(
                interner,
                builder,
                file,
                source,
                module_scope,
                &expression.expression,
            );
        }
        Expression::TSTypeAssertion(expression) => {
            collect_commonjs_from_expression(
                interner,
                builder,
                file,
                source,
                module_scope,
                &expression.expression,
            );
        }
        _ => {
            let _ = source;
        }
    }
}

fn collect_commonjs_from_call(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SemanticIndexBuilder,
    file: &SourceFile,
    _source: &str,
    module_scope: Option<ScopeId>,
    call: &oxc_ast::ast::CallExpression<'_>,
) {
    if callee_text(&call.callee).as_deref() != Some("require") {
        return;
    }
    let (import_path, status) = match call.arguments.first() {
        Some(Argument::StringLiteral(path)) => (
            path.value.to_string(),
            import_status_for_path(path.value.as_str()),
        ),
        _ => ("<dynamic>".to_string(), SemanticStatus::Dynamic),
    };
    add_import_row(
        interner,
        builder,
        file,
        module_scope,
        import_path,
        None,
        None,
        SymbolNamespace::Module,
        SemanticImportKind::CommonJsRequire,
        status,
    );
}

fn commonjs_export_kind(target: &AssignmentTarget<'_>) -> Option<ExportKind> {
    match target {
        AssignmentTarget::StaticMemberExpression(member)
            if expression_text(&member.object).as_deref() == Some("module")
                && member.property.name == "exports" =>
        {
            Some(ExportKind::CommonJsModuleExports)
        }
        AssignmentTarget::StaticMemberExpression(member)
            if expression_text(&member.object).as_deref() == Some("exports") =>
        {
            Some(ExportKind::CommonJsExportsProperty)
        }
        _ => None,
    }
}

fn commonjs_export_name(target: &AssignmentTarget<'_>) -> String {
    match target {
        AssignmentTarget::StaticMemberExpression(member)
            if expression_text(&member.object).as_deref() == Some("exports") =>
        {
            member.property.name.to_string()
        }
        _ => "module.exports".to_string(),
    }
}

fn import_status_for_path(path: &str) -> SemanticStatus {
    if path == "<dynamic>" {
        SemanticStatus::Dynamic
    } else if path.starts_with('.') || path.starts_with('/') || path.starts_with('#') {
        SemanticStatus::Unresolved
    } else {
        SemanticStatus::External
    }
}

fn module_export_source_name(path: &str) -> String {
    format!("* from {path}")
}

fn callee_text(expression: &Expression<'_>) -> Option<String> {
    expression_text(expression)
}

fn expression_text(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier.name.to_string()),
        Expression::StaticMemberExpression(member) => expression_text(&member.object)
            .map(|object| format!("{object}.{}", member.property.name)),
        Expression::ParenthesizedExpression(expression) => expression_text(&expression.expression),
        Expression::TSAsExpression(expression) => expression_text(&expression.expression),
        Expression::TSSatisfiesExpression(expression) => expression_text(&expression.expression),
        Expression::TSNonNullExpression(expression) => expression_text(&expression.expression),
        Expression::TSTypeAssertion(expression) => expression_text(&expression.expression),
        _ => None,
    }
}

fn scope_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    file: &SourceFile,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    scope_id: oxc_semantic::ScopeId,
) -> StableKeyId {
    let kind = scope_kind_for_ast(nodes.kind(scoping.get_node_id(scope_id)));
    ScopeFact::stable_key_for(
        interner,
        file.language,
        &scope_path_for_scope(file, scoping, nodes, scope_id),
        Some(file.relative_path.clone()),
        None,
        None,
        (kind, SemanticStatus::Resolved),
    )
}

fn add_ts_semantic_resolution_rows(
    interner: &crate::internal_core::StableKeyInterner,
    builder: &mut SemanticIndexBuilder,
    file: &SourceFile,
    source: &str,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
) {
    let parameter_symbols = collect_parameter_symbols(nodes);
    let mut resolved = scoping
        .symbol_ids()
        .flat_map(|symbol| {
            scoping
                .get_resolved_reference_ids(symbol)
                .iter()
                .map(move |reference| (symbol, *reference))
        })
        .collect::<Vec<_>>();
    resolved.sort_by(|left, right| {
        reference_order_key(scoping, nodes, left.1)
            .cmp(&reference_order_key(scoping, nodes, right.1))
    });

    for (symbol, reference_id) in resolved {
        let reference = scoping.get_reference(reference_id);
        let target_key =
            ts_symbol_stable_key_input(file, source, scoping, nodes, symbol, &parameter_symbols);
        add_resolution_row(
            interner,
            builder,
            file,
            semantic_resolved_reference_key(file, source, nodes, reference, target_key.clone()),
            vec![target_key.stable_key()],
            ResolutionStepKind::LexicalLookup,
            SemanticStatus::Resolved,
        );
    }

    let mut unresolved = scoping
        .root_unresolved_references_ids()
        .flat_map(|references| references.collect::<Vec<_>>())
        .collect::<Vec<_>>();
    unresolved.sort_by(|left, right| {
        reference_order_key(scoping, nodes, *left).cmp(&reference_order_key(scoping, nodes, *right))
    });
    unresolved.dedup();

    for reference_id in unresolved {
        let reference = scoping.get_reference(reference_id);
        let name = reference_name(nodes, reference);
        add_resolution_row(
            interner,
            builder,
            file,
            semantic_unresolved_reference_key(file, source, nodes, reference, &name),
            Vec::new(),
            ResolutionStepKind::UnknownFallback,
            SemanticStatus::Unresolved,
        );
    }
}

fn semantic_resolved_reference_key(
    file: &SourceFile,
    _source: &str,
    nodes: &AstNodes<'_>,
    reference: &Reference,
    target: StableSymbolKey,
) -> String {
    StableReferenceKey::resolved(
        target,
        file.relative_path.clone(),
        span_from_oxc(file, nodes.kind(reference.node_id()).span()),
    )
    .stable_key()
}

fn semantic_unresolved_reference_key(
    file: &SourceFile,
    _source: &str,
    nodes: &AstNodes<'_>,
    reference: &Reference,
    name: &str,
) -> String {
    StableReferenceKey::unresolved(
        file.language,
        file.relative_path.clone(),
        name.to_string(),
        span_from_oxc(file, nodes.kind(reference.node_id()).span()),
    )
    .stable_key()
}

fn scope_path_for_scope(
    file: &SourceFile,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    scope_id: oxc_semantic::ScopeId,
) -> Vec<String> {
    let mut ancestors = scoping.scope_ancestors(scope_id).collect::<Vec<_>>();
    ancestors.reverse();
    let mut path = vec![file.relative_path.clone()];
    path.extend(ancestors.into_iter().map(|scope| {
        let kind = nodes.kind(scoping.get_node_id(scope));
        scope_path_segment(kind)
    }));
    path
}

fn scope_path_for_node(
    file: &SourceFile,
    nodes: &AstNodes<'_>,
    node_id: oxc_semantic::NodeId,
) -> Vec<String> {
    let mut ancestors = nodes
        .ancestors_enumerated(node_id)
        .filter_map(|(_, node)| semantic_scope_kind_for_ast(node.kind()).map(|_| node.kind()))
        .collect::<Vec<_>>();
    ancestors.reverse();
    let mut path = vec![file.relative_path.clone()];
    path.extend(ancestors.into_iter().map(scope_path_segment));
    path.push(scope_path_segment(nodes.kind(node_id)));
    path
}

fn nearest_semantic_scope_parent(
    interner: &crate::internal_core::StableKeyInterner,
    file: &SourceFile,
    nodes: &AstNodes<'_>,
    node_id: oxc_semantic::NodeId,
    ids_by_stable_key: &BTreeMap<StableKeyId, ScopeId>,
) -> Option<ScopeId> {
    nodes.ancestors_enumerated(node_id).find_map(|(_, node)| {
        let kind = semantic_scope_kind_for_ast(node.kind())?;
        let scope_path = scope_path_for_node(file, nodes, node.id());
        let stable_key = ScopeFact::stable_key_for(
            interner,
            file.language,
            &scope_path,
            Some(file.relative_path.clone()),
            None,
            None,
            (kind, SemanticStatus::Resolved),
        );
        ids_by_stable_key.get(&stable_key).copied()
    })
}

fn semantic_scope_kind_for_ast(kind: AstKind<'_>) -> Option<ScopeKind> {
    match kind {
        AstKind::Program(_) => Some(ScopeKind::Module),
        AstKind::Function(_) | AstKind::ArrowFunctionExpression(_) => Some(ScopeKind::Function),
        AstKind::Class(_) => Some(ScopeKind::Class),
        AstKind::BlockStatement(_) => Some(ScopeKind::Block),
        AstKind::CatchClause(_) => Some(ScopeKind::Catch),
        AstKind::ForStatement(_) | AstKind::ForInStatement(_) | AstKind::ForOfStatement(_) => {
            Some(ScopeKind::Loop)
        }
        AstKind::SwitchStatement(_) => Some(ScopeKind::Switch),
        AstKind::TSInterfaceDeclaration(_)
        | AstKind::TSTypeAliasDeclaration(_)
        | AstKind::TSEnumDeclaration(_) => Some(ScopeKind::Type),
        AstKind::TSModuleDeclaration(_) => Some(ScopeKind::Namespace),
        _ => None,
    }
}

fn scope_kind_for_ast(kind: AstKind<'_>) -> ScopeKind {
    semantic_scope_kind_for_ast(kind).unwrap_or(ScopeKind::Block)
}

fn scope_path_segment(kind: AstKind<'_>) -> String {
    let span = kind.span();
    format!("{}@{}-{}", ast_kind_label(kind), span.start, span.end)
}

fn add_ts_references(
    builder: &mut SymbolGraphBuilder,
    file: &SourceFile,
    source: &str,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    symbol_ids: &BTreeMap<OxcSymbolId, PolintSymbolId>,
) {
    let mut resolved = symbol_ids
        .iter()
        .flat_map(|(oxc_symbol, target)| {
            scoping
                .get_resolved_reference_ids(*oxc_symbol)
                .iter()
                .map(|reference| (*reference, *target))
        })
        .collect::<Vec<_>>();
    resolved.sort_by(|left, right| {
        reference_order_key(scoping, nodes, left.0)
            .cmp(&reference_order_key(scoping, nodes, right.0))
    });

    for (reference_id, target) in resolved {
        let reference = scoping.get_reference(reference_id);
        let draft = reference_draft(
            file,
            source,
            nodes,
            reference,
            reference_name(nodes, reference),
            SymbolPrecision::ExactLocal,
        );
        builder.add_reference(target, draft);
    }

    let mut unresolved = scoping
        .root_unresolved_references_ids()
        .flat_map(|references| references.collect::<Vec<_>>())
        .collect::<Vec<_>>();
    unresolved.sort_by(|left, right| {
        reference_order_key(scoping, nodes, *left).cmp(&reference_order_key(scoping, nodes, *right))
    });
    unresolved.dedup();

    for reference_id in unresolved {
        let reference = scoping.get_reference(reference_id);
        let draft = reference_draft(
            file,
            source,
            nodes,
            reference,
            reference_name(nodes, reference),
            SymbolPrecision::Unresolved,
        );
        builder.add_unresolved_reference(draft);
    }
}

fn add_import_alias_references(
    builder: &mut SymbolGraphBuilder,
    db: &dyn FactDatabase,
    resolved_imports: &[ResolvedImportFact],
    module_nodes: &[ModuleNode],
    summaries: &[TsFileSymbolSummary],
) {
    let exports_by_file = summaries
        .iter()
        .map(|summary| (summary.file, summary.exports.clone()))
        .collect::<BTreeMap<_, _>>();
    let imports_by_key = db
        .imports()
        .iter()
        .map(|import| {
            (
                (import.file, import.path.clone(), import.span.start_byte),
                import,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let resolved_by_import = resolved_imports
        .iter()
        .map(|resolved| (resolved.import, resolved))
        .collect::<BTreeMap<_, _>>();
    let file_by_node = module_nodes
        .iter()
        .filter_map(|node| node.file.map(|file| (node.id, file)))
        .collect::<BTreeMap<ModuleNodeId, FileId>>();

    for alias in summaries
        .iter()
        .flat_map(|summary| summary.import_aliases.iter())
    {
        let key = (
            alias.file,
            alias.import_path.clone(),
            alias.import_source_span.start_byte,
        );
        let Some(import) = imports_by_key.get(&key) else {
            let draft = import_alias_reference_draft(alias, SymbolPrecision::SetupMissing);
            builder.add_setup_missing_reference_draft(draft);
            continue;
        };
        let Some(resolved) = resolved_by_import.get(&import.id) else {
            let draft = import_alias_reference_draft(alias, SymbolPrecision::SetupMissing);
            builder.add_setup_missing_reference_draft(draft);
            continue;
        };

        add_import_alias_reference(builder, alias, resolved, &file_by_node, &exports_by_file);
    }
}

fn add_import_alias_reference(
    builder: &mut SymbolGraphBuilder,
    alias: &ImportAlias,
    resolved: &ResolvedImportFact,
    file_by_node: &BTreeMap<ModuleNodeId, FileId>,
    exports_by_file: &BTreeMap<FileId, BTreeMap<String, Vec<PolintSymbolId>>>,
) {
    match resolved.status {
        ResolutionStatus::Resolved => {
            let Some(target_file) = resolved
                .target_node
                .and_then(|target_node| file_by_node.get(&target_node).copied())
            else {
                let draft = import_alias_reference_draft(alias, SymbolPrecision::Unresolved);
                builder.add_unresolved_reference(draft);
                return;
            };
            let candidates = import_alias_candidates(alias, target_file, exports_by_file);
            match candidates.as_slice() {
                [target] => {
                    let draft = import_alias_reference_draft(alias, SymbolPrecision::ModuleLinked);
                    builder.add_reference(*target, draft);
                }
                [] => {
                    let draft = import_alias_reference_draft(alias, SymbolPrecision::Unresolved);
                    builder.add_unresolved_reference(draft);
                }
                _ => {
                    let draft = import_alias_reference_draft(alias, SymbolPrecision::Ambiguous);
                    builder.add_ambiguous_reference(candidates, draft);
                }
            }
        }
        ResolutionStatus::SetupMissing => {
            let draft = import_alias_reference_draft(alias, SymbolPrecision::SetupMissing);
            builder.add_setup_missing_reference_draft(draft);
        }
        ResolutionStatus::Unsupported => {
            let draft = import_alias_reference_draft(alias, SymbolPrecision::Unsupported);
            builder.add_unsupported_reference_draft(draft);
        }
        ResolutionStatus::External | ResolutionStatus::Unresolved | ResolutionStatus::Dynamic => {
            let draft = import_alias_reference_draft(alias, SymbolPrecision::Unresolved);
            builder.add_unresolved_reference(draft);
        }
        _ => {
            let draft = import_alias_reference_draft(alias, SymbolPrecision::Unresolved);
            builder.add_unresolved_reference(draft);
        }
    }
}

fn import_alias_candidates(
    alias: &ImportAlias,
    target_file: FileId,
    exports_by_file: &BTreeMap<FileId, BTreeMap<String, Vec<PolintSymbolId>>>,
) -> Vec<PolintSymbolId> {
    let Some(exports) = exports_by_file.get(&target_file) else {
        return Vec::new();
    };
    let mut candidates = match &alias.target {
        ImportAliasTarget::Named(name) => exports.get(name).cloned().unwrap_or_default(),
        ImportAliasTarget::Default => exports.get("default").cloned().unwrap_or_default(),
        ImportAliasTarget::Namespace => exports
            .values()
            .flat_map(|symbols| symbols.iter().copied())
            .collect::<Vec<_>>(),
    };
    candidates.sort_unstable();
    candidates.dedup();
    candidates
}

fn import_alias_reference_draft(alias: &ImportAlias, precision: SymbolPrecision) -> ReferenceDraft {
    ReferenceDraft {
        language: alias.language,
        name: alias.local_name.clone(),
        qualified_name: format!("{}::{}", alias.file_key, alias.local_name),
        kind: ReferenceKind::Import,
        namespace: alias.namespace,
        file: Some(alias.file),
        package: None,
        module: None,
        owner: None,
        file_key: alias.file_key.clone(),
        primary_span: Some(alias.local_span.clone()),
        precision,
    }
}

fn reference_order_key(
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    reference_id: OxcReferenceId,
) -> (u32, u32, String) {
    let reference = scoping.get_reference(reference_id);
    let span = nodes.kind(reference.node_id()).span();
    (span.start, span.end, reference_name(nodes, reference))
}

fn reference_draft(
    file: &SourceFile,
    _source: &str,
    nodes: &AstNodes<'_>,
    reference: &Reference,
    name: String,
    precision: SymbolPrecision,
) -> ReferenceDraft {
    let flags = reference.flags();
    ReferenceDraft {
        language: file.language,
        name: name.clone(),
        qualified_name: format!("{}::{name}", file.relative_path),
        kind: reference_kind(flags, nodes, reference),
        namespace: reference_namespace(flags),
        file: Some(file.id),
        package: None,
        module: None,
        owner: None,
        file_key: file.relative_path.clone(),
        primary_span: Some(span_from_oxc(file, nodes.kind(reference.node_id()).span())),
        precision,
    }
}

fn reference_name(nodes: &AstNodes<'_>, reference: &Reference) -> String {
    match nodes.kind(reference.node_id()) {
        AstKind::IdentifierReference(identifier) => identifier.name.to_string(),
        AstKind::BindingIdentifier(identifier) => identifier.name.to_string(),
        AstKind::IdentifierName(identifier) => identifier.name.to_string(),
        kind => {
            let span = kind.span();
            format!("<reference:{}-{}>", span.start, span.end)
        }
    }
}

fn reference_kind(
    flags: ReferenceFlags,
    nodes: &AstNodes<'_>,
    reference: &Reference,
) -> ReferenceKind {
    if flags.is_type() || flags.is_value_as_type() {
        ReferenceKind::TypeUse
    } else if is_call_reference(nodes, reference.node_id()) {
        ReferenceKind::Call
    } else if flags.is_read_write() {
        ReferenceKind::ReadWrite
    } else if flags.is_write() {
        ReferenceKind::Write
    } else if flags.is_read() {
        ReferenceKind::Read
    } else {
        ReferenceKind::Unknown
    }
}

fn is_call_reference(nodes: &AstNodes<'_>, node_id: oxc_semantic::NodeId) -> bool {
    matches!(
        nodes.parent_kind(node_id),
        AstKind::CallExpression(_) | AstKind::NewExpression(_)
    )
}

fn reference_namespace(flags: ReferenceFlags) -> SymbolNamespace {
    if flags.is_namespace() {
        SymbolNamespace::Namespace
    } else if flags.is_type() || flags.is_value_as_type() {
        SymbolNamespace::Type
    } else if flags.is_value() {
        SymbolNamespace::Value
    } else {
        SymbolNamespace::Unknown
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "definition rows need the Oxc symbol context and normalized polint identity"
)]
fn add_symbol_definitions(
    builder: &mut SymbolGraphBuilder,
    file: &SourceFile,
    _source: &str,
    scoping: &Scoping,
    oxc_symbol: OxcSymbolId,
    symbol: PolintSymbolId,
    name: String,
    qualified_name: String,
    namespace: SymbolNamespace,
    is_exported: bool,
) {
    let redeclarations = scoping.symbol_redeclarations(oxc_symbol);
    if redeclarations.is_empty() {
        let flags = scoping.symbol_flags(oxc_symbol);
        let span = span_from_oxc(file, scoping.symbol_span(oxc_symbol));
        builder.add_definition(
            symbol,
            definition_draft(
                file,
                name,
                qualified_name,
                namespace,
                flags,
                Some(span),
                true,
                is_exported,
            ),
        );
        return;
    }

    for (index, redeclaration) in redeclarations.iter().enumerate() {
        builder.add_definition(
            symbol,
            definition_draft(
                file,
                name.clone(),
                qualified_name.clone(),
                namespace,
                redeclaration.flags,
                Some(span_from_oxc(file, redeclaration.span)),
                index == 0,
                is_exported,
            ),
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "definition drafts mirror the public fact identity and span fields"
)]
fn definition_draft(
    file: &SourceFile,
    name: String,
    qualified_name: String,
    namespace: SymbolNamespace,
    flags: SymbolFlags,
    primary_span: Option<Span>,
    is_primary: bool,
    is_exported: bool,
) -> DefinitionDraft {
    DefinitionDraft {
        language: file.language,
        name,
        qualified_name,
        kind: definition_kind(flags, is_exported),
        namespace,
        file: Some(file.id),
        package: None,
        module: None,
        owner: None,
        file_key: file.relative_path.clone(),
        primary_span,
        is_primary,
        is_exported,
        precision: SymbolPrecision::ExactLocal,
    }
}

fn definition_kind(flags: SymbolFlags, is_exported: bool) -> DefinitionKind {
    if flags.is_import() {
        DefinitionKind::Import
    } else if is_exported {
        DefinitionKind::Export
    } else {
        DefinitionKind::Declaration
    }
}

fn symbol_kind(flags: SymbolFlags, is_parameter: bool) -> SymbolKind {
    if flags.is_import() {
        SymbolKind::Import
    } else if is_parameter {
        SymbolKind::Parameter
    } else if flags.is_interface() {
        SymbolKind::Interface
    } else if flags.is_type_alias() {
        SymbolKind::TypeAlias
    } else if flags.is_enum_member() {
        SymbolKind::EnumMember
    } else if flags.is_enum() {
        SymbolKind::Enum
    } else if flags.is_class() {
        SymbolKind::Class
    } else if flags.is_function() {
        SymbolKind::Function
    } else if flags.is_const_variable() {
        SymbolKind::Constant
    } else if flags.is_namespace_module() || flags.is_value_module() {
        SymbolKind::Namespace
    } else if flags.is_variable() {
        SymbolKind::Variable
    } else {
        SymbolKind::Unknown
    }
}

fn symbol_namespace(flags: SymbolFlags) -> SymbolNamespace {
    if flags.is_namespace_module() || flags.is_value_module() {
        SymbolNamespace::Namespace
    } else if flags.is_type() && !flags.is_value() {
        SymbolNamespace::Type
    } else if flags.is_value() {
        SymbolNamespace::Value
    } else if flags.is_type() {
        SymbolNamespace::Type
    } else {
        SymbolNamespace::Unknown
    }
}

fn collect_parameter_symbols(nodes: &AstNodes<'_>) -> BTreeSet<OxcSymbolId> {
    nodes
        .iter_enumerated()
        .filter_map(|(node_id, node)| {
            let AstKind::BindingIdentifier(identifier) = node.kind() else {
                return None;
            };
            let is_parameter = nodes.ancestor_kinds(node_id).any(|kind| {
                matches!(
                    kind,
                    AstKind::FormalParameter(_)
                        | AstKind::FormalParameterRest(_)
                        | AstKind::CatchParameter(_)
                        | AstKind::TSThisParameter(_)
                )
            });
            is_parameter.then(|| identifier.symbol_id.get()).flatten()
        })
        .collect()
}

fn owner_chain(
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    scope_id: oxc_semantic::ScopeId,
) -> Vec<String> {
    let mut scopes = scoping.scope_ancestors(scope_id).collect::<Vec<_>>();
    scopes.reverse();
    scopes
        .into_iter()
        .filter_map(|scope| {
            let node_id = scoping.get_node_id(scope);
            let kind = nodes.kind(node_id);
            if matches!(kind, AstKind::Program(_)) {
                return None;
            }
            let span = kind.span();
            Some(format!(
                "{}@{}-{}",
                ast_kind_label(kind),
                span.start,
                span.end
            ))
        })
        .collect()
}

fn ast_kind_label(kind: AstKind<'_>) -> &'static str {
    match kind {
        AstKind::Function(_) => "function",
        AstKind::ArrowFunctionExpression(_) => "arrow",
        AstKind::Class(_) => "class",
        AstKind::BlockStatement(_) => "block",
        AstKind::CatchClause(_) => "catch",
        AstKind::ForStatement(_) => "for",
        AstKind::ForInStatement(_) => "for-in",
        AstKind::ForOfStatement(_) => "for-of",
        AstKind::SwitchStatement(_) => "switch",
        AstKind::TSInterfaceDeclaration(_) => "interface",
        AstKind::TSTypeAliasDeclaration(_) => "type",
        AstKind::TSEnumDeclaration(_) => "enum",
        _ => "scope",
    }
}

fn qualified_name(file_key: &str, owner_chain: &[String], name: &str) -> String {
    if owner_chain.is_empty() {
        format!("{file_key}::{name}")
    } else {
        format!("{file_key}::{}::{name}", owner_chain.join("::"))
    }
}

fn collect_import_aliases(
    file: &SourceFile,
    _source: &str,
    program: &Program<'_>,
) -> Vec<ImportAlias> {
    let mut aliases = Vec::new();

    for statement in &program.body {
        let Statement::ImportDeclaration(import) = statement else {
            continue;
        };
        let Some(specifiers) = &import.specifiers else {
            continue;
        };
        let import_path = import.source.value.to_string();
        let import_source_span = span_from_oxc(file, import.source.span);

        for specifier in specifiers {
            match specifier {
                // default import alias
                ImportDeclarationSpecifier::ImportDefaultSpecifier(default) => {
                    aliases.push(ImportAlias {
                        file: file.id,
                        language: file.language,
                        file_key: file.relative_path.clone(),
                        import_path: import_path.clone(),
                        import_source_span: import_source_span.clone(),
                        local_name: default.local.name.to_string(),
                        local_span: span_from_oxc(file, default.local.span),
                        target: ImportAliasTarget::Default,
                        namespace: SymbolNamespace::Value,
                    });
                }
                // namespace import alias
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(namespace) => {
                    aliases.push(ImportAlias {
                        file: file.id,
                        language: file.language,
                        file_key: file.relative_path.clone(),
                        import_path: import_path.clone(),
                        import_source_span: import_source_span.clone(),
                        local_name: namespace.local.name.to_string(),
                        local_span: span_from_oxc(file, namespace.local.span),
                        target: ImportAliasTarget::Namespace,
                        namespace: SymbolNamespace::Namespace,
                    });
                }
                ImportDeclarationSpecifier::ImportSpecifier(named) => {
                    let is_type_import = matches!(import.import_kind, ImportOrExportKind::Type)
                        || matches!(named.import_kind, ImportOrExportKind::Type);
                    aliases.push(ImportAlias {
                        file: file.id,
                        language: file.language,
                        file_key: file.relative_path.clone(),
                        import_path: import_path.clone(),
                        import_source_span: import_source_span.clone(),
                        local_name: named.local.name.to_string(),
                        local_span: span_from_oxc(file, named.local.span),
                        target: ImportAliasTarget::Named(module_export_name_text(&named.imported)),
                        namespace: if is_type_import {
                            SymbolNamespace::Type
                        } else {
                            SymbolNamespace::Value
                        },
                    });
                }
            }
        }
    }

    aliases
}

fn collect_export_names(
    program: &Program<'_>,
    scoping: &Scoping,
) -> BTreeMap<OxcSymbolId, BTreeSet<String>> {
    let mut exports = BTreeMap::<OxcSymbolId, BTreeSet<String>>::new();

    for statement in &program.body {
        match statement {
            Statement::ExportNamedDeclaration(export) => {
                if let Some(declaration) = &export.declaration {
                    let mut declarations = Vec::new();
                    collect_declaration_symbols(declaration, &mut declarations);
                    for (symbol, name) in declarations {
                        exports.entry(symbol).or_default().insert(name);
                    }
                }

                if export.source.is_none() {
                    for specifier in &export.specifiers {
                        if let Some(symbol) = module_export_name_symbol(&specifier.local, scoping) {
                            exports
                                .entry(symbol)
                                .or_default()
                                .insert(module_export_name_text(&specifier.exported));
                        }
                    }
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                if let Some(symbol) = default_export_symbol(&export.declaration, scoping) {
                    exports
                        .entry(symbol)
                        .or_default()
                        .insert("default".to_string());
                }
            }
            _ => {}
        }
    }

    exports
}

fn collect_declaration_symbols(
    declaration: &Declaration<'_>,
    symbols: &mut Vec<(OxcSymbolId, String)>,
) {
    match declaration {
        Declaration::VariableDeclaration(declaration) => {
            for declarator in &declaration.declarations {
                collect_binding_pattern_symbols(&declarator.id, symbols);
            }
        }
        Declaration::FunctionDeclaration(function) => {
            if let Some(id) = &function.id {
                collect_binding_identifier_symbol(id, symbols);
            }
        }
        Declaration::ClassDeclaration(class) => {
            if let Some(id) = &class.id {
                collect_binding_identifier_symbol(id, symbols);
            }
        }
        Declaration::TSTypeAliasDeclaration(type_alias) => {
            collect_binding_identifier_symbol(&type_alias.id, symbols);
        }
        Declaration::TSInterfaceDeclaration(interface) => {
            collect_binding_identifier_symbol(&interface.id, symbols);
        }
        Declaration::TSEnumDeclaration(enumeration) => {
            collect_binding_identifier_symbol(&enumeration.id, symbols);
        }
        Declaration::TSModuleDeclaration(_)
        | Declaration::TSGlobalDeclaration(_)
        | Declaration::TSImportEqualsDeclaration(_) => {}
    }
}

fn collect_binding_pattern_symbols(
    pattern: &BindingPattern<'_>,
    symbols: &mut Vec<(OxcSymbolId, String)>,
) {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => {
            collect_binding_identifier_symbol(identifier, symbols);
        }
        BindingPattern::ObjectPattern(pattern) => {
            for property in &pattern.properties {
                collect_binding_pattern_symbols(&property.value, symbols);
            }
            if let Some(rest) = &pattern.rest {
                collect_binding_pattern_symbols(&rest.argument, symbols);
            }
        }
        BindingPattern::ArrayPattern(pattern) => {
            for element in pattern.elements.iter().flatten() {
                collect_binding_pattern_symbols(element, symbols);
            }
            if let Some(rest) = &pattern.rest {
                collect_binding_pattern_symbols(&rest.argument, symbols);
            }
        }
        BindingPattern::AssignmentPattern(pattern) => {
            collect_binding_pattern_symbols(&pattern.left, symbols);
        }
    }
}

fn collect_binding_identifier_symbol(
    identifier: &BindingIdentifier<'_>,
    symbols: &mut Vec<(OxcSymbolId, String)>,
) {
    if let Some(symbol) = identifier.symbol_id.get() {
        symbols.push((symbol, identifier.name.to_string()));
    }
}

fn default_export_symbol(
    declaration: &ExportDefaultDeclarationKind<'_>,
    scoping: &Scoping,
) -> Option<OxcSymbolId> {
    match declaration {
        ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
            function.id.as_ref()?.symbol_id.get()
        }
        ExportDefaultDeclarationKind::ClassDeclaration(class) => class.id.as_ref()?.symbol_id.get(),
        ExportDefaultDeclarationKind::TSInterfaceDeclaration(interface) => {
            interface.id.symbol_id.get()
        }
        _ => declaration
            .as_expression()
            .and_then(|expression| expression_symbol(expression, scoping)),
    }
}

fn expression_symbol(expression: &Expression<'_>, scoping: &Scoping) -> Option<OxcSymbolId> {
    match expression {
        Expression::Identifier(identifier) => identifier
            .reference_id
            .get()
            .and_then(|reference| scoping.get_reference(reference).symbol_id()),
        Expression::ParenthesizedExpression(expression) => {
            expression_symbol(&expression.expression, scoping)
        }
        Expression::TSAsExpression(expression) => {
            expression_symbol(&expression.expression, scoping)
        }
        Expression::TSSatisfiesExpression(expression) => {
            expression_symbol(&expression.expression, scoping)
        }
        Expression::TSNonNullExpression(expression) => {
            expression_symbol(&expression.expression, scoping)
        }
        Expression::TSTypeAssertion(expression) => {
            expression_symbol(&expression.expression, scoping)
        }
        _ => None,
    }
}

fn module_export_name_symbol(
    name: &ModuleExportName<'_>,
    scoping: &Scoping,
) -> Option<OxcSymbolId> {
    match name {
        ModuleExportName::IdentifierReference(identifier) => identifier
            .reference_id
            .get()
            .and_then(|reference| scoping.get_reference(reference).symbol_id()),
        ModuleExportName::IdentifierName(_) | ModuleExportName::StringLiteral(_) => None,
    }
}

fn module_export_name_text(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(identifier) => identifier.name.to_string(),
        ModuleExportName::IdentifierReference(identifier) => identifier.name.to_string(),
        ModuleExportName::StringLiteral(literal) => literal.value.to_string(),
    }
}

fn span_from_oxc(file: &SourceFile, span: oxc_span::Span) -> Span {
    file.span_from_byte_range(span.start as usize, span.end as usize)
}

fn oxc_diagnostics(
    file: &SourceFile,
    diagnostics: &[impl std::fmt::Display],
    rule_id: &str,
) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .map(|error| {
            Diagnostic::error(
                rule_id,
                file.relative_path.clone(),
                TextRange::point(1, 1),
                format!("TS/JS semantic provider reported an error: {error}"),
            )
        })
        .collect()
}

#[cfg(test)]
include!("symbol_graph_frontend_tests.rs");
#[cfg(test)]
include!("symbol_graph_tests.rs");
