#![allow(dead_code, reason = "kept for private internal consumers")]

use std::collections::{BTreeMap, BTreeSet};

use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast::ast::{
    Argument, AssignmentTarget, BindingPattern, Declaration, ExportDefaultDeclarationKind,
    Expression, FunctionType, ImportDeclarationSpecifier, ImportOrExportKind, ModuleExportName,
    ObjectPropertyKind, Program, PropertyKey, Statement, VariableDeclarationKind,
};
use oxc_semantic::{
    AstNodes, ScopeId as OxcScopeId, Scoping, SemanticBuilder, SymbolFlags, SymbolId as OxcSymbolId,
};
use oxc_span::GetSpan;

use crate::analysis_api::SourceFile;
use crate::internal_core::{FileId, Span, StableKeyInterner};
use crate::ts::ids::{TsBindingId, TsScopeId};
use crate::ts::parse::{PARTIAL_AST_REASON, parse_ts_file};
use crate::ts::scope::facts::{
    TsBindingFact, TsBindingKind, TsBindingStatus, TsDeclarationKind, TsImportExportKind,
    TsScopeFact, TsScopeKind,
};
use crate::ts::scope::store::TsScopeOutput;
use crate::ts::sealed_bindings::{bindings_written_by_block_functions, dynamic_scope};

pub fn extract_ts_scope(interner: &StableKeyInterner, file: &SourceFile) -> TsScopeOutput {
    let source = file.source.as_ref();
    let allocator = Allocator::default();
    let parsed = parse_ts_file(&allocator, file);

    if parsed.is_catastrophic() {
        return TsScopeOutput::default();
    }

    let semantic = SemanticBuilder::new().build(parsed.program()).semantic;
    let mut output = extract_ts_scope_from_program(
        interner,
        file,
        source,
        parsed.program(),
        semantic.scoping(),
        semantic.nodes(),
    )
    .normalized(interner);
    if !parsed.fully_parsed {
        mark_scope_partial_ast(&mut output);
    }
    output
}

pub fn mark_scope_partial_ast(output: &mut TsScopeOutput) {
    for binding in &mut output.bindings {
        if matches!(binding.status, TsBindingStatus::Present) {
            mark_binding_dynamic(binding, PARTIAL_AST_REASON);
        }
    }
}

/// The reason a binding carries when a `with` statement or a direct `eval`
/// can reach it: what its name holds there is not decided by the binding.
const DYNAMIC_SCOPE_REASON: &str = "dynamic-scope: with statement or eval";

/// The reason a binding carries when a block-level function declaration of
/// non-strict code also writes it.
const BLOCK_FUNCTION_WRITE_REASON: &str = "dynamic-scope: written by a block function declaration";

fn mark_binding_dynamic(binding: &mut TsBindingFact, reason: &str) {
    binding.status = TsBindingStatus::unsupported_dynamic(reason);
    binding.binding_kind = TsBindingKind::UnsupportedDynamic;
    binding.declaration_kind = TsDeclarationKind::UnsupportedDynamic;
}

pub fn extract_ts_scope_from_program(
    interner: &StableKeyInterner,
    file: &SourceFile,
    source: &str,
    program: &Program<'_>,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
) -> TsScopeOutput {
    crate::ts::with_frontend_stable_keys(interner, || {
        let (scopes, scope_keys) = extract_scope_rows(file, source, scoping, nodes);
        let written = bindings_written_by_block_functions(scoping, nodes);
        let mut bindings =
            extract_semantic_binding_rows(file, source, scoping, nodes, &scope_keys, &written);
        // Oxc semantic symbols record import bindings, but not each import/export
        // specifier's module/source alias shape as a standalone row.
        bindings.extend(extract_import_export_rows(
            file,
            source,
            program,
            &scope_keys,
            scoping,
        ));
        // Oxc semantic symbols do not model local alias assignments as alias rows;
        // this AST fallback records only direct identifier/require aliases.
        bindings.extend(extract_alias_rows(
            file,
            source,
            nodes,
            &scope_keys,
            scoping,
            &written,
        ));
        // Oxc semantic symbols point destructured names at the declaration owner in
        // some cases; this AST fallback preserves an explicit destructuring row.
        bindings.extend(extract_destructuring_rows(
            file,
            source,
            nodes,
            &scope_keys,
            scoping,
            &written,
        ));
        bindings.extend(extract_boundary_rows(
            file,
            source,
            nodes,
            &scope_keys,
            scoping,
            &bindings,
        ));

        // A `with` body can make any name of the file read another value than
        // its binding holds, and a direct `eval` any name its call can see.
        let dynamic = dynamic_scope(program, scoping, nodes);
        let reached = dynamic
            .bindings()
            .iter()
            .map(|symbol| {
                (
                    crate::ts::intern_frontend_stable_key(scope_key_for_symbol(
                        file,
                        &scope_keys,
                        scoping,
                        *symbol,
                    )),
                    scoping.symbol_name(*symbol),
                )
            })
            .collect::<BTreeSet<_>>();
        // An eval that may name the module wrapper may also rewrite what the
        // file exports and what its `require` calls load.
        for binding in &mut bindings {
            let through_wrapper = dynamic.opens_wrapper()
                && (binding.module_source.is_some() || binding.exported_name.is_some());
            if dynamic.whole_file()
                || through_wrapper
                || reached.contains(&(binding.scope_key, binding.name.as_str()))
            {
                mark_binding_dynamic(binding, DYNAMIC_SCOPE_REASON);
            }
        }

        TsScopeOutput { scopes, bindings }
    })
}

fn extract_scope_rows(
    file: &SourceFile,
    _source: &str,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
) -> (Vec<TsScopeFact>, BTreeMap<OxcScopeId, String>) {
    let mut scope_keys = BTreeMap::new();
    let mut rows = Vec::new();

    for scope_id in scoping.scope_descendants_from_root() {
        let node_id = scoping.get_node_id(scope_id);
        let kind = ts_scope_kind(nodes.kind(node_id));
        let span = span_from_oxc(file, nodes.kind(node_id).span());
        let stable_key = scope_stable_key(file, &span, kind);
        scope_keys.insert(scope_id, stable_key.clone());
        let parent_scope_key = scoping
            .scope_parent_id(scope_id)
            .and_then(|parent| scope_keys.get(&parent).cloned());
        rows.push(TsScopeFact {
            id: TsScopeId(0),
            file: file.id,
            span,
            stable_key: crate::ts::intern_frontend_stable_key(stable_key),
            parent_scope_key: parent_scope_key.map(crate::ts::intern_frontend_stable_key),
            kind,
        });
    }

    (rows, scope_keys)
}

fn extract_semantic_binding_rows(
    file: &SourceFile,
    source: &str,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    scope_keys: &BTreeMap<OxcScopeId, String>,
    written: &BTreeSet<OxcSymbolId>,
) -> Vec<TsBindingFact> {
    let mut symbols = scoping.symbol_ids().collect::<Vec<_>>();
    symbols.sort_by(|left, right| {
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

    symbols
        .into_iter()
        .map(|symbol| {
            let mut row = semantic_binding_row(file, source, scoping, nodes, scope_keys, symbol);
            if written.contains(&symbol) {
                mark_binding_dynamic(&mut row, BLOCK_FUNCTION_WRITE_REASON);
            }
            row
        })
        .collect()
}

fn semantic_binding_row(
    file: &SourceFile,
    _source: &str,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    scope_keys: &BTreeMap<OxcScopeId, String>,
    symbol: OxcSymbolId,
) -> TsBindingFact {
    let flags = scoping.symbol_flags(symbol);
    let scope_id = scoping.symbol_scope_id(symbol);
    let scope_key = scope_keys
        .get(&scope_id)
        .cloned()
        .unwrap_or_else(|| module_scope_key(file));
    let parent_scope_key = scoping
        .scope_parent_id(scope_id)
        .and_then(|parent| scope_keys.get(&parent).cloned());
    let declaration_node = scoping.symbol_declaration(symbol);
    let declaration_kind = declaration_kind_for_symbol(flags, nodes, declaration_node);
    let binding_kind = binding_kind_for_symbol(flags, declaration_kind);
    let span = span_from_oxc(file, scoping.symbol_span(symbol));
    let name = scoping.symbol_name(symbol).to_string();
    let status = if flags.is_import() {
        TsBindingStatus::external("<semantic-import>")
    } else {
        TsBindingStatus::present()
    };
    let import_export_kind = if flags.is_import() {
        TsImportExportKind::ImportNamed
    } else {
        TsImportExportKind::None
    };
    let stable_key = binding_stable_key(
        file,
        &span,
        &scope_key,
        &name,
        binding_kind,
        import_export_kind,
        &status,
    );

    TsBindingFact {
        id: TsBindingId(0),
        file: file.id,
        span,
        stable_key: crate::ts::intern_frontend_stable_key(stable_key),
        scope_key: crate::ts::intern_frontend_stable_key(scope_key),
        parent_scope_key: parent_scope_key.map(crate::ts::intern_frontend_stable_key),
        name,
        declaration_kind,
        binding_kind,
        import_export_kind,
        module_source: None,
        imported_name: None,
        exported_name: None,
        inventory_function_key: None,
        inventory_callsite_key: None,
        status,
    }
}

fn extract_import_export_rows(
    file: &SourceFile,
    source: &str,
    program: &Program<'_>,
    scope_keys: &BTreeMap<OxcScopeId, String>,
    scoping: &Scoping,
) -> Vec<TsBindingFact> {
    let module_scope = scope_keys
        .get(&scoping.root_scope_id())
        .cloned()
        .unwrap_or_else(|| module_scope_key(file));
    let mut rows = Vec::new();

    for statement in &program.body {
        match statement {
            Statement::ImportDeclaration(import) => {
                let import_path = import.source.value.to_string();
                let Some(specifiers) = &import.specifiers else {
                    rows.push(binding_row(BindingRowDraft {
                        file,
                        source,
                        span: span_from_oxc(file, import.span),
                        scope_key: module_scope.clone(),
                        name: import_path.clone(),
                        declaration_kind: TsDeclarationKind::Import,
                        binding_kind: TsBindingKind::Import,
                        import_export_kind: TsImportExportKind::SideEffectImport,
                        module_source: Some(import_path.clone()),
                        imported_name: None,
                        exported_name: None,
                        status: TsBindingStatus::external(import_path),
                    }));
                    continue;
                };

                for specifier in specifiers {
                    match specifier {
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(default) => {
                            let local = default.local.name.to_string();
                            rows.push(import_binding_row(
                                file,
                                source,
                                default.span,
                                module_scope.clone(),
                                local,
                                "default".to_string(),
                                import_path.clone(),
                                TsBindingKind::DefaultImport,
                                TsImportExportKind::ImportDefault,
                            ));
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(namespace) => {
                            let local = namespace.local.name.to_string();
                            rows.push(import_binding_row(
                                file,
                                source,
                                namespace.span,
                                module_scope.clone(),
                                local,
                                "*".to_string(),
                                import_path.clone(),
                                TsBindingKind::NamespaceImport,
                                TsImportExportKind::ImportNamespace,
                            ));
                        }
                        ImportDeclarationSpecifier::ImportSpecifier(named) => {
                            let local = named.local.name.to_string();
                            let imported = module_export_name_text(&named.imported);
                            let is_type_import =
                                matches!(import.import_kind, ImportOrExportKind::Type)
                                    || matches!(named.import_kind, ImportOrExportKind::Type);
                            rows.push(import_binding_row(
                                file,
                                source,
                                named.span,
                                module_scope.clone(),
                                local.clone(),
                                imported.clone(),
                                import_path.clone(),
                                TsBindingKind::NamedImport,
                                TsImportExportKind::ImportNamed,
                            ));
                            if local != imported || is_type_import {
                                rows.push(binding_row(BindingRowDraft {
                                    file,
                                    source,
                                    span: span_from_oxc(file, named.span),
                                    scope_key: module_scope.clone(),
                                    name: local,
                                    declaration_kind: TsDeclarationKind::Alias,
                                    binding_kind: TsBindingKind::Alias,
                                    import_export_kind: TsImportExportKind::Alias,
                                    module_source: Some(import_path.clone()),
                                    imported_name: Some(imported),
                                    exported_name: None,
                                    status: TsBindingStatus::external(import_path.clone()),
                                }));
                            }
                        }
                    }
                }
            }
            Statement::ExportNamedDeclaration(export) => {
                if let Some(source_module) = &export.source {
                    for specifier in &export.specifiers {
                        let exported = module_export_name_text(&specifier.exported);
                        rows.push(binding_row(BindingRowDraft {
                            file,
                            source,
                            span: span_from_oxc(file, specifier.span),
                            scope_key: module_scope.clone(),
                            name: exported.clone(),
                            declaration_kind: TsDeclarationKind::ReExport,
                            binding_kind: TsBindingKind::ReExport,
                            import_export_kind: TsImportExportKind::ReExportNamed,
                            module_source: Some(source_module.value.to_string()),
                            imported_name: Some(module_export_name_text(&specifier.local)),
                            exported_name: Some(exported),
                            status: TsBindingStatus::external(source_module.value.to_string()),
                        }));
                    }
                } else if let Some(declaration) = &export.declaration {
                    rows.extend(local_export_declaration_rows(
                        file,
                        source,
                        module_scope.clone(),
                        declaration,
                    ));
                } else {
                    for specifier in &export.specifiers {
                        rows.push(local_export_row(
                            file,
                            source,
                            span_from_oxc(file, specifier.span),
                            module_scope.clone(),
                            module_export_name_text(&specifier.local),
                            module_export_name_text(&specifier.exported),
                        ));
                    }
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                rows.push(binding_row(BindingRowDraft {
                    file,
                    source,
                    span: span_from_oxc(file, export.span),
                    scope_key: module_scope.clone(),
                    name: "default".to_string(),
                    declaration_kind: TsDeclarationKind::ReExport,
                    binding_kind: TsBindingKind::ReExport,
                    import_export_kind: TsImportExportKind::ExportDefault,
                    module_source: None,
                    imported_name: default_export_target_name(&export.declaration),
                    exported_name: Some("default".to_string()),
                    status: TsBindingStatus::present(),
                }));
            }
            Statement::ExpressionStatement(statement) => {
                if let Some((exported, imported, span)) =
                    commonjs_export_assignment(&statement.expression)
                {
                    rows.push(local_export_row(
                        file,
                        source,
                        span_from_oxc(file, span),
                        module_scope.clone(),
                        imported,
                        exported,
                    ));
                }
            }
            Statement::ExportAllDeclaration(export) => {
                rows.push(binding_row(BindingRowDraft {
                    file,
                    source,
                    span: span_from_oxc(file, export.span),
                    scope_key: module_scope.clone(),
                    name: "*".to_string(),
                    declaration_kind: TsDeclarationKind::ReExport,
                    binding_kind: TsBindingKind::ReExport,
                    import_export_kind: TsImportExportKind::ReExportAll,
                    module_source: Some(export.source.value.to_string()),
                    imported_name: Some("*".to_string()),
                    exported_name: Some("*".to_string()),
                    status: TsBindingStatus::external(export.source.value.to_string()),
                }));
            }
            _ => {}
        }
    }

    rows
}

fn default_export_target_name(declaration: &ExportDefaultDeclarationKind<'_>) -> Option<String> {
    match declaration {
        ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
            function.id.as_ref().map(|id| id.name.to_string())
        }
        ExportDefaultDeclarationKind::ClassDeclaration(class) => {
            class.id.as_ref().map(|id| id.name.to_string())
        }
        ExportDefaultDeclarationKind::Identifier(identifier) => Some(identifier.name.to_string()),
        _ => None,
    }
}

fn local_export_declaration_rows(
    file: &SourceFile,
    source: &str,
    module_scope: String,
    declaration: &Declaration<'_>,
) -> Vec<TsBindingFact> {
    match declaration {
        Declaration::FunctionDeclaration(function) => function
            .id
            .as_ref()
            .map(|id| {
                vec![local_export_row(
                    file,
                    source,
                    span_from_oxc(file, function.span),
                    module_scope,
                    id.name.to_string(),
                    id.name.to_string(),
                )]
            })
            .unwrap_or_default(),
        Declaration::VariableDeclaration(variable) => variable
            .declarations
            .iter()
            .filter_map(|declarator| {
                let name = binding_identifier_name(&declarator.id)?;
                Some(local_export_row(
                    file,
                    source,
                    span_from_oxc(file, declarator.span),
                    module_scope.clone(),
                    name.clone(),
                    name,
                ))
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn local_export_row(
    file: &SourceFile,
    source: &str,
    span: Span,
    scope_key: String,
    imported_name: String,
    exported_name: String,
) -> TsBindingFact {
    binding_row(BindingRowDraft {
        file,
        source,
        span,
        scope_key,
        name: imported_name.clone(),
        declaration_kind: TsDeclarationKind::ReExport,
        binding_kind: TsBindingKind::ReExport,
        import_export_kind: TsImportExportKind::ReExportNamed,
        module_source: None,
        imported_name: Some(imported_name),
        exported_name: Some(exported_name),
        status: TsBindingStatus::present(),
    })
}

fn commonjs_export_assignment(
    expression: &Expression<'_>,
) -> Option<(String, String, oxc_span::Span)> {
    let Expression::AssignmentExpression(assignment) = expression else {
        return None;
    };
    let exported = commonjs_export_assignment_name(&assignment.left)?;
    let imported = match &assignment.right {
        Expression::FunctionExpression(function) => function
            .id
            .as_ref()
            .map(|id| id.name.to_string())
            .unwrap_or_else(|| exported.clone()),
        _ => exported.clone(),
    };
    Some((exported, imported, assignment.span))
}

fn commonjs_export_assignment_name(target: &AssignmentTarget<'_>) -> Option<String> {
    match target {
        AssignmentTarget::StaticMemberExpression(member)
            if matches!(
                expression_text(&member.object).as_deref(),
                Some("exports" | "module.exports")
            ) =>
        {
            Some(member.property.name.to_string())
        }
        _ => None,
    }
}

fn extract_alias_rows(
    file: &SourceFile,
    source: &str,
    nodes: &AstNodes<'_>,
    scope_keys: &BTreeMap<OxcScopeId, String>,
    scoping: &Scoping,
    written: &BTreeSet<OxcSymbolId>,
) -> Vec<TsBindingFact> {
    let module_scope = scope_keys
        .get(&scoping.root_scope_id())
        .cloned()
        .unwrap_or_else(|| module_scope_key(file));
    let mut rows = Vec::new();

    for (_, node) in nodes.iter_enumerated() {
        let AstKind::VariableDeclarator(declarator) = node.kind() else {
            continue;
        };
        let Some(identifier) = binding_identifier(&declarator.id) else {
            continue;
        };
        let Some(init) = &declarator.init else {
            continue;
        };
        let symbol = symbol_for_binding(scoping, identifier.name.as_str(), identifier.span);
        if !keeps_declared_value(scoping, written, symbol) {
            continue;
        }
        let scope_key = symbol.map_or_else(
            || module_scope.clone(),
            |symbol| scope_key_for_symbol(file, scope_keys, scoping, symbol),
        );
        match init {
            Expression::Identifier(target) if reads_declared_value(scoping, written, init) => rows
                .push(binding_row(BindingRowDraft {
                    file,
                    source,
                    span: span_from_oxc(file, declarator.span),
                    scope_key: scope_key.clone(),
                    name: identifier.name.clone(),
                    declaration_kind: TsDeclarationKind::Alias,
                    binding_kind: TsBindingKind::Alias,
                    import_export_kind: TsImportExportKind::Alias,
                    module_source: None,
                    imported_name: Some(target.name.to_string()),
                    exported_name: None,
                    status: TsBindingStatus::present(),
                })),
            Expression::ObjectExpression(object) => {
                rows.extend(object.properties.iter().filter_map(|property| {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        return None;
                    };
                    if property.computed || !reads_declared_value(scoping, written, &property.value)
                    {
                        return None;
                    }
                    let property_name = property_key_name(&property.key)?;
                    let target_name = expression_identifier_name(&property.value)?;
                    Some(binding_row(BindingRowDraft {
                        file,
                        source,
                        span: span_from_oxc(file, declarator.span),
                        scope_key: scope_key.clone(),
                        name: format!("{}.{}", identifier.name, property_name),
                        declaration_kind: TsDeclarationKind::Alias,
                        binding_kind: TsBindingKind::Alias,
                        import_export_kind: TsImportExportKind::Alias,
                        module_source: None,
                        imported_name: Some(target_name),
                        exported_name: None,
                        status: TsBindingStatus::present(),
                    }))
                }));
            }
            Expression::StaticMemberExpression(member)
                if let Some(module) = require_module_path(&member.object) =>
            {
                rows.push(binding_row(BindingRowDraft {
                    file,
                    source,
                    span: span_from_oxc(file, declarator.span),
                    scope_key: scope_key.clone(),
                    name: identifier.name.clone(),
                    declaration_kind: TsDeclarationKind::Import,
                    binding_kind: TsBindingKind::Import,
                    import_export_kind: TsImportExportKind::CommonJsRequire,
                    module_source: Some(module.clone()),
                    imported_name: Some(member.property.name.to_string()),
                    exported_name: None,
                    status: TsBindingStatus::external(module),
                }));
            }
            Expression::CallExpression(call)
                if expression_text(&call.callee).as_deref() == Some("require") =>
            {
                let module = match call.arguments.first() {
                    Some(Argument::StringLiteral(path)) => path.value.to_string(),
                    _ => "<dynamic-require>".to_string(),
                };
                rows.push(binding_row(BindingRowDraft {
                    file,
                    source,
                    span: span_from_oxc(file, declarator.span),
                    scope_key,
                    name: identifier.name,
                    declaration_kind: TsDeclarationKind::Import,
                    binding_kind: TsBindingKind::Import,
                    import_export_kind: TsImportExportKind::CommonJsRequire,
                    module_source: Some(module.clone()),
                    imported_name: Some("module.exports".to_string()),
                    exported_name: None,
                    status: if module == "<dynamic-require>" {
                        TsBindingStatus::unsupported_dynamic("dynamic require")
                    } else {
                        TsBindingStatus::external(module)
                    },
                }));
            }
            _ => {}
        }
    }

    rows
}

fn require_module_path(expression: &Expression<'_>) -> Option<String> {
    let Expression::CallExpression(call) = expression else {
        return None;
    };
    if expression_text(&call.callee).as_deref() != Some("require") {
        return None;
    }
    match call.arguments.first() {
        Some(Argument::StringLiteral(path)) => Some(path.value.to_string()),
        _ => None,
    }
}

fn extract_destructuring_rows(
    file: &SourceFile,
    source: &str,
    nodes: &AstNodes<'_>,
    scope_keys: &BTreeMap<OxcScopeId, String>,
    scoping: &Scoping,
    written: &BTreeSet<OxcSymbolId>,
) -> Vec<TsBindingFact> {
    let module_scope = scope_keys
        .get(&scoping.root_scope_id())
        .cloned()
        .unwrap_or_else(|| module_scope_key(file));
    let mut rows = Vec::new();

    for (_, node) in nodes.iter_enumerated() {
        let AstKind::VariableDeclarator(declarator) = node.kind() else {
            continue;
        };
        if matches!(declarator.id, BindingPattern::BindingIdentifier(_)) {
            continue;
        }
        let destructuring_aliases =
            object_literal_destructuring_aliases(&declarator.id, declarator.init.as_ref());
        let mut names = Vec::new();
        collect_pattern_bindings(&declarator.id, &mut names);
        for binding in names {
            let symbol = symbol_for_binding(scoping, binding.name.as_str(), binding.span);
            let imported_name = destructuring_aliases
                .get(&binding.name)
                .filter(|_| keeps_declared_value(scoping, written, symbol))
                .cloned();
            let import_export_kind = if imported_name.is_some() {
                TsImportExportKind::Alias
            } else {
                TsImportExportKind::None
            };
            let scope_key = symbol.map_or_else(
                || module_scope.clone(),
                |symbol| scope_key_for_symbol(file, scope_keys, scoping, symbol),
            );
            rows.push(binding_row(BindingRowDraft {
                file,
                source,
                span: span_from_oxc(file, declarator.span),
                scope_key,
                name: binding.name,
                declaration_kind: TsDeclarationKind::Destructuring,
                binding_kind: TsBindingKind::Destructuring,
                import_export_kind,
                module_source: None,
                imported_name,
                exported_name: None,
                status: TsBindingStatus::present(),
            }));
        }
    }

    rows
}

fn object_literal_destructuring_aliases(
    pattern: &BindingPattern<'_>,
    init: Option<&Expression<'_>>,
) -> BTreeMap<String, String> {
    let BindingPattern::ObjectPattern(pattern) = pattern else {
        return BTreeMap::new();
    };
    let Some(Expression::ObjectExpression(object)) = init else {
        return BTreeMap::new();
    };

    let targets_by_key = object
        .properties
        .iter()
        .filter_map(|property| {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                return None;
            };
            if property.computed {
                return None;
            }
            Some((
                property_key_name(&property.key)?,
                expression_identifier_name(&property.value)?,
            ))
        })
        .collect::<BTreeMap<_, _>>();

    pattern
        .properties
        .iter()
        .filter_map(|property| {
            if property.computed {
                return None;
            }
            let target = targets_by_key.get(&property_key_name(&property.key)?)?;
            Some((
                destructured_local_name(&property.value)?,
                target.to_string(),
            ))
        })
        .collect()
}

fn extract_boundary_rows(
    file: &SourceFile,
    source: &str,
    nodes: &AstNodes<'_>,
    scope_keys: &BTreeMap<OxcScopeId, String>,
    scoping: &Scoping,
    bindings: &[TsBindingFact],
) -> Vec<TsBindingFact> {
    let module_scope = scope_keys
        .get(&scoping.root_scope_id())
        .cloned()
        .unwrap_or_else(|| module_scope_key(file));
    let scope_parents = scope_parent_keys(scoping, scope_keys);
    let mut rows = Vec::new();

    for (_, node) in nodes.iter_enumerated() {
        let AstKind::CallExpression(call) = node.kind() else {
            continue;
        };
        let call_scope = scope_key_for_span(scope_keys, scoping, nodes, call.span)
            .unwrap_or_else(|| module_scope.clone());
        let Some((name, reason)) =
            boundary_reason(&call.callee, bindings, &scope_parents, call_scope.as_str())
        else {
            continue;
        };
        rows.push(binding_row(BindingRowDraft {
            file,
            source,
            span: span_from_oxc(file, call.span),
            scope_key: call_scope,
            name,
            declaration_kind: TsDeclarationKind::UnsupportedDynamic,
            binding_kind: TsBindingKind::UnsupportedDynamic,
            import_export_kind: TsImportExportKind::None,
            module_source: None,
            imported_name: None,
            exported_name: None,
            status: TsBindingStatus::unsupported_dynamic(reason),
        }));
    }

    rows
}

fn boundary_reason(
    callee: &Expression<'_>,
    bindings: &[TsBindingFact],
    scope_parents: &BTreeMap<String, Option<String>>,
    call_scope: &str,
) -> Option<(String, &'static str)> {
    match callee {
        Expression::Identifier(identifier)
            if visible_binding_in_rows(
                bindings,
                scope_parents,
                call_scope,
                identifier.name.as_str(),
            )
            .is_some_and(|binding| binding.binding_kind == TsBindingKind::Parameter) =>
        {
            Some((
                identifier.name.to_string(),
                "parameter callback requires function-token analysis",
            ))
        }
        Expression::ComputedMemberExpression(_) => Some((
            "<computed-member>".to_string(),
            "computed property requires property-flow analysis",
        )),
        Expression::StaticMemberExpression(_)
            if expression_text(callee)
                .as_deref()
                .is_some_and(|text| text.contains(".prototype.")) =>
        {
            Some((
                expression_text(callee).unwrap_or_else(|| "<prototype>".to_string()),
                "prototype dispatch requires prototype analysis",
            ))
        }
        Expression::StaticMemberExpression(member)
            if matches!(&member.object, Expression::ThisExpression(_)) =>
        {
            Some((
                expression_text(callee).unwrap_or_else(|| "this.<member>".to_string()),
                "this-dependent call requires receiver modeling",
            ))
        }
        _ => None,
    }
}

fn scope_parent_keys(
    scoping: &Scoping,
    scope_keys: &BTreeMap<OxcScopeId, String>,
) -> BTreeMap<String, Option<String>> {
    scoping
        .scope_descendants_from_root()
        .filter_map(|scope| {
            let key = scope_keys.get(&scope)?.clone();
            let parent = scoping
                .scope_parent_id(scope)
                .and_then(|parent| scope_keys.get(&parent).cloned());
            Some((key, parent))
        })
        .collect()
}

fn visible_binding_in_rows<'a>(
    bindings: &'a [TsBindingFact],
    scope_parents: &BTreeMap<String, Option<String>>,
    scope_key: &str,
    name: &str,
) -> Option<&'a TsBindingFact> {
    let mut current = Some(scope_key);
    while let Some(scope) = current {
        if let Some(binding) = bindings.iter().find(|binding| {
            crate::ts::resolve_frontend_stable_key(binding.scope_key).as_ref() == scope
                && binding.name == name
        }) {
            return Some(binding);
        }
        current = scope_parents
            .get(scope)
            .and_then(|parent| parent.as_deref());
    }
    None
}

fn scope_key_for_span(
    scope_keys: &BTreeMap<OxcScopeId, String>,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    span: oxc_span::Span,
) -> Option<String> {
    scoping
        .scope_descendants_from_root()
        .filter_map(|scope| {
            let node_id = scoping.get_node_id(scope);
            let kind = ts_scope_kind(nodes.kind(node_id));
            if kind == TsScopeKind::Module {
                return None;
            }
            let scope_span = nodes.kind(node_id).span();
            if scope_span.start > span.start || scope_span.end < span.end {
                return None;
            }
            Some((
                scope_span.end - scope_span.start,
                scope_keys.get(&scope)?.clone(),
            ))
        })
        .min_by_key(|(width, _)| *width)
        .map(|(_, key)| key)
        .or_else(|| scope_keys.get(&scoping.root_scope_id()).cloned())
}

#[derive(Clone)]
struct PatternBinding {
    name: String,
    span: oxc_span::Span,
}

fn collect_pattern_bindings(pattern: &BindingPattern<'_>, names: &mut Vec<PatternBinding>) {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => names.push(PatternBinding {
            name: identifier.name.to_string(),
            span: identifier.span,
        }),
        BindingPattern::ObjectPattern(pattern) => {
            for property in &pattern.properties {
                collect_pattern_bindings(&property.value, names);
            }
            if let Some(rest) = &pattern.rest {
                collect_pattern_bindings(&rest.argument, names);
            }
        }
        BindingPattern::ArrayPattern(pattern) => {
            for element in pattern.elements.iter().flatten() {
                collect_pattern_bindings(element, names);
            }
            if let Some(rest) = &pattern.rest {
                collect_pattern_bindings(&rest.argument, names);
            }
        }
        BindingPattern::AssignmentPattern(pattern) => {
            collect_pattern_bindings(&pattern.left, names);
        }
    }
}

fn destructured_local_name(pattern: &BindingPattern<'_>) -> Option<String> {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => Some(identifier.name.to_string()),
        BindingPattern::AssignmentPattern(pattern) => destructured_local_name(&pattern.left),
        _ => None,
    }
}

fn property_key_name(key: &PropertyKey<'_>) -> Option<String> {
    key.name().map(|name| name.into_owned())
}

fn expression_identifier_name(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier.name.to_string()),
        Expression::ParenthesizedExpression(expression) => {
            expression_identifier_name(&expression.expression)
        }
        Expression::TSAsExpression(expression) => {
            expression_identifier_name(&expression.expression)
        }
        Expression::TSSatisfiesExpression(expression) => {
            expression_identifier_name(&expression.expression)
        }
        Expression::TSNonNullExpression(expression) => {
            expression_identifier_name(&expression.expression)
        }
        Expression::TSTypeAssertion(expression) => {
            expression_identifier_name(&expression.expression)
        }
        _ => None,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "import rows mirror the normalized binding fact dimensions"
)]
fn import_binding_row(
    file: &SourceFile,
    source: &str,
    span: oxc_span::Span,
    scope_key: String,
    local: String,
    imported: String,
    import_path: String,
    binding_kind: TsBindingKind,
    import_export_kind: TsImportExportKind,
) -> TsBindingFact {
    binding_row(BindingRowDraft {
        file,
        source,
        span: span_from_oxc(file, span),
        scope_key,
        name: local,
        declaration_kind: TsDeclarationKind::Import,
        binding_kind,
        import_export_kind,
        module_source: Some(import_path.clone()),
        imported_name: Some(imported),
        exported_name: None,
        status: TsBindingStatus::external(import_path),
    })
}

struct BindingRowDraft<'a> {
    file: &'a SourceFile,
    source: &'a str,
    span: Span,
    scope_key: String,
    name: String,
    declaration_kind: TsDeclarationKind,
    binding_kind: TsBindingKind,
    import_export_kind: TsImportExportKind,
    module_source: Option<String>,
    imported_name: Option<String>,
    exported_name: Option<String>,
    status: TsBindingStatus,
}

fn binding_row(draft: BindingRowDraft<'_>) -> TsBindingFact {
    let stable_key = binding_stable_key(
        draft.file,
        &draft.span,
        &draft.scope_key,
        &draft.name,
        draft.binding_kind,
        draft.import_export_kind,
        &draft.status,
    );
    TsBindingFact {
        id: TsBindingId(0),
        file: draft.file.id,
        span: draft.span,
        stable_key: crate::ts::intern_frontend_stable_key(stable_key),
        scope_key: crate::ts::intern_frontend_stable_key(draft.scope_key),
        parent_scope_key: None,
        name: draft.name,
        declaration_kind: draft.declaration_kind,
        binding_kind: draft.binding_kind,
        import_export_kind: draft.import_export_kind,
        module_source: draft.module_source,
        imported_name: draft.imported_name,
        exported_name: draft.exported_name,
        inventory_function_key: None,
        inventory_callsite_key: None,
        status: draft.status,
    }
}

fn declaration_kind_for_symbol(
    flags: SymbolFlags,
    nodes: &AstNodes<'_>,
    declaration_node: oxc_semantic::NodeId,
) -> TsDeclarationKind {
    if flags.is_import() {
        return TsDeclarationKind::Import;
    }
    if flags.is_catch_variable() {
        return TsDeclarationKind::Catch;
    }
    if is_parameter_declaration(nodes, declaration_node) {
        return TsDeclarationKind::Parameter;
    }
    if is_destructuring_declaration(nodes, declaration_node) {
        return TsDeclarationKind::Destructuring;
    }
    match nodes.kind(declaration_node) {
        AstKind::VariableDeclaration(declaration) => match declaration.kind {
            VariableDeclarationKind::Var => TsDeclarationKind::Var,
            VariableDeclarationKind::Let => TsDeclarationKind::Let,
            VariableDeclarationKind::Const => TsDeclarationKind::Const,
            VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing => {
                TsDeclarationKind::UnsupportedDynamic
            }
        },
        AstKind::VariableDeclarator(declarator) => match declarator.kind {
            VariableDeclarationKind::Var => TsDeclarationKind::Var,
            VariableDeclarationKind::Let => TsDeclarationKind::Let,
            VariableDeclarationKind::Const => match declarator.init.as_ref() {
                Some(Expression::FunctionExpression(_)) => TsDeclarationKind::FunctionExpression,
                Some(Expression::ClassExpression(_)) => TsDeclarationKind::ClassExpression,
                _ => TsDeclarationKind::Const,
            },
            VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing => {
                TsDeclarationKind::UnsupportedDynamic
            }
        },
        AstKind::Function(function) => match function.r#type {
            FunctionType::FunctionDeclaration | FunctionType::TSDeclareFunction => {
                TsDeclarationKind::FunctionDeclaration
            }
            FunctionType::FunctionExpression | FunctionType::TSEmptyBodyFunctionExpression => {
                TsDeclarationKind::FunctionExpression
            }
        },
        AstKind::Class(_) => TsDeclarationKind::ClassDeclaration,
        AstKind::FormalParameter(_) | AstKind::FormalParameterRest(_) => {
            TsDeclarationKind::Parameter
        }
        AstKind::CatchParameter(_) => TsDeclarationKind::Catch,
        _ if flags.is_function() => TsDeclarationKind::FunctionDeclaration,
        _ if flags.is_class() => TsDeclarationKind::ClassDeclaration,
        _ if flags.is_const_variable() => TsDeclarationKind::Const,
        _ if flags.is_function_scoped_declaration() => TsDeclarationKind::Var,
        _ => TsDeclarationKind::Unknown,
    }
}

fn binding_kind_for_symbol(
    flags: SymbolFlags,
    declaration_kind: TsDeclarationKind,
) -> TsBindingKind {
    if flags.is_import() {
        return TsBindingKind::Import;
    }
    match declaration_kind {
        TsDeclarationKind::Var => TsBindingKind::Var,
        TsDeclarationKind::Let => TsBindingKind::Let,
        TsDeclarationKind::Const => TsBindingKind::Const,
        TsDeclarationKind::FunctionDeclaration => TsBindingKind::FunctionDeclaration,
        TsDeclarationKind::FunctionExpression => TsBindingKind::FunctionExpression,
        TsDeclarationKind::ClassDeclaration => TsBindingKind::ClassDeclaration,
        TsDeclarationKind::ClassExpression => TsBindingKind::ClassExpression,
        TsDeclarationKind::Parameter => TsBindingKind::Parameter,
        TsDeclarationKind::Destructuring => TsBindingKind::Destructuring,
        TsDeclarationKind::Catch => TsBindingKind::Catch,
        TsDeclarationKind::Import => TsBindingKind::Import,
        TsDeclarationKind::ReExport => TsBindingKind::ReExport,
        TsDeclarationKind::Alias => TsBindingKind::Alias,
        TsDeclarationKind::UnsupportedDynamic => TsBindingKind::UnsupportedDynamic,
        TsDeclarationKind::Unknown => TsBindingKind::Unknown,
    }
}

fn is_parameter_declaration(nodes: &AstNodes<'_>, node_id: oxc_semantic::NodeId) -> bool {
    matches!(
        nodes.kind(node_id),
        AstKind::FormalParameter(_) | AstKind::FormalParameterRest(_)
    ) || nodes.ancestor_kinds(node_id).any(|kind| {
        matches!(
            kind,
            AstKind::FormalParameter(_)
                | AstKind::FormalParameterRest(_)
                | AstKind::TSThisParameter(_)
        )
    })
}

fn is_destructuring_declaration(nodes: &AstNodes<'_>, node_id: oxc_semantic::NodeId) -> bool {
    nodes.ancestor_kinds(node_id).any(|kind| {
        matches!(
            kind,
            AstKind::ObjectPattern(_) | AstKind::ArrayPattern(_) | AstKind::AssignmentPattern(_)
        )
    }) || matches!(
        nodes.kind(node_id),
        AstKind::ObjectPattern(_) | AstKind::ArrayPattern(_) | AstKind::AssignmentPattern(_)
    )
}

fn binding_identifier_name(pattern: &BindingPattern<'_>) -> Option<String> {
    binding_identifier(pattern).map(|identifier| identifier.name)
}

fn binding_identifier(pattern: &BindingPattern<'_>) -> Option<PatternBinding> {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => Some(PatternBinding {
            name: identifier.name.to_string(),
            span: identifier.span,
        }),
        _ => None,
    }
}

fn scope_key_for_symbol(
    file: &SourceFile,
    scope_keys: &BTreeMap<OxcScopeId, String>,
    scoping: &Scoping,
    symbol: OxcSymbolId,
) -> String {
    scope_keys
        .get(&scoping.symbol_scope_id(symbol))
        .cloned()
        .unwrap_or_else(|| module_scope_key(file))
}

fn symbol_for_binding(scoping: &Scoping, name: &str, span: oxc_span::Span) -> Option<OxcSymbolId> {
    scoping.symbol_ids().find(|&symbol| {
        let symbol_span = scoping.symbol_span(symbol);
        scoping.symbol_name(symbol) == name
            && symbol_span.start == span.start
            && symbol_span.end == span.end
    })
}

/// Whether a declared name keeps the value it is declared with: no reference
/// writes it, no second declaration redeclares it and no block function
/// declaration writes it. A name that may hold another value is no alias of
/// its initializer.
fn keeps_declared_value(
    scoping: &Scoping,
    written: &BTreeSet<OxcSymbolId>,
    symbol: Option<OxcSymbolId>,
) -> bool {
    symbol.is_some_and(|symbol| {
        !scoping.symbol_is_mutated(symbol)
            && scoping.symbol_redeclarations(symbol).is_empty()
            && !written.contains(&symbol)
    })
}

/// Whether the value an alias reads stays the declared one: the name it reads
/// is not written anywhere. An unresolved name is left to the binding lookup.
fn reads_declared_value(
    scoping: &Scoping,
    written: &BTreeSet<OxcSymbolId>,
    expression: &Expression<'_>,
) -> bool {
    let Expression::Identifier(identifier) = expression else {
        return true;
    };
    identifier
        .reference_id
        .get()
        .and_then(|reference| scoping.get_reference(reference).symbol_id())
        .is_none_or(|symbol| !scoping.symbol_is_mutated(symbol) && !written.contains(&symbol))
}

fn ts_scope_kind(kind: AstKind<'_>) -> TsScopeKind {
    match kind {
        AstKind::Program(_) => TsScopeKind::Module,
        AstKind::Function(_) | AstKind::ArrowFunctionExpression(_) => TsScopeKind::Function,
        AstKind::Class(_) => TsScopeKind::Class,
        AstKind::BlockStatement(_) => TsScopeKind::Block,
        AstKind::CatchClause(_) => TsScopeKind::Catch,
        AstKind::ForStatement(_) | AstKind::ForInStatement(_) | AstKind::ForOfStatement(_) => {
            TsScopeKind::Loop
        }
        AstKind::SwitchStatement(_) => TsScopeKind::Switch,
        AstKind::TSInterfaceDeclaration(_)
        | AstKind::TSTypeAliasDeclaration(_)
        | AstKind::TSEnumDeclaration(_) => TsScopeKind::Type,
        AstKind::TSModuleDeclaration(_) => TsScopeKind::Namespace,
        _ => TsScopeKind::Unknown,
    }
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

fn module_export_name_text(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(identifier) => identifier.name.to_string(),
        ModuleExportName::IdentifierReference(identifier) => identifier.name.to_string(),
        ModuleExportName::StringLiteral(literal) => literal.value.to_string(),
    }
}

fn scope_stable_key(file: &SourceFile, span: &Span, kind: TsScopeKind) -> String {
    stable_key(
        "ts_scope",
        &[
            ("file", file.relative_path.clone()),
            ("kind", kind.as_str().to_string()),
            ("start", span.start_byte.to_string()),
            ("end", span.end_byte.to_string()),
        ],
    )
}

fn module_scope_key(file: &SourceFile) -> String {
    stable_key(
        "ts_scope",
        &[
            ("file", file.relative_path.clone()),
            ("kind", "module".to_string()),
            ("start", "0".to_string()),
            ("end", "0".to_string()),
        ],
    )
}

fn binding_stable_key(
    file: &SourceFile,
    span: &Span,
    scope_key: &str,
    name: &str,
    binding_kind: TsBindingKind,
    import_export_kind: TsImportExportKind,
    status: &TsBindingStatus,
) -> String {
    stable_key(
        "ts_binding",
        &[
            ("file", file.relative_path.clone()),
            ("scope", scope_key.to_string()),
            ("name", name.to_string()),
            ("kind", binding_kind.as_str().to_string()),
            ("io", format!("{import_export_kind:?}")),
            ("status", status.as_str().to_string()),
            ("start", span.start_byte.to_string()),
            ("end", span.end_byte.to_string()),
        ],
    )
}

fn stable_key(prefix: &str, parts: &[(&str, String)]) -> String {
    let mut normalized = parts
        .iter()
        .map(|(label, value)| (*label, value.replace('\\', "/")))
        .collect::<Vec<_>>();
    normalized.sort_by(|left, right| left.0.cmp(right.0));

    let mut key = length_prefixed(prefix);
    for (label, value) in normalized {
        key.push('|');
        key.push_str(&length_prefixed(label));
        key.push('=');
        key.push_str(&length_prefixed(&value));
    }
    key
}

fn length_prefixed(value: &str) -> String {
    format!("{}:{}", value.len(), value)
}

fn span_from_oxc(file: &SourceFile, span: oxc_span::Span) -> Span {
    file.span_from_byte_range(span.start as usize, span.end as usize)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use crate::ts::local_db::LocalFactDb;

    use super::*;

    #[test]
    fn extracts_required_scope_binding_forms_from_oxc_semantics() {
        let file = fixture_file(
            r#"
import defaultThing, { named as alias, other } from "./dep";
import * as ns from "./ns";
export { named as renamed } from "./dep";
export * from "./star";

var legacy = 1;
let local = 2;
const fixed = 3;
const { item: destructured } = source;
function declared(param, { nested }) {
  try {
    return param;
  } catch (err) {
    const inside = err;
  }
}
class Klass {}
const expr = function namedExpr() {};
const KlassExpr = class {};
const aliasLocal = local;
"#,
        );

        let interner = StableKeyInterner::default();
        let output = extract_ts_scope(&interner, file);
        let kinds = output
            .bindings
            .iter()
            .map(|binding| binding.binding_kind)
            .collect::<BTreeSet<_>>();

        for expected in [
            TsBindingKind::Var,
            TsBindingKind::Let,
            TsBindingKind::Const,
            TsBindingKind::FunctionDeclaration,
            TsBindingKind::FunctionExpression,
            TsBindingKind::ClassDeclaration,
            TsBindingKind::ClassExpression,
            TsBindingKind::Parameter,
            TsBindingKind::Destructuring,
            TsBindingKind::Catch,
            TsBindingKind::Import,
            TsBindingKind::ReExport,
            TsBindingKind::NamespaceImport,
            TsBindingKind::DefaultImport,
            TsBindingKind::NamedImport,
            TsBindingKind::Alias,
        ] {
            assert!(kinds.contains(&expected), "missing {expected:?}");
        }
        assert!(!output.scopes.is_empty());
    }

    #[test]
    fn import_rows_carry_external_status_and_module_source() {
        let file = fixture_file(r#"import { named as alias } from "./dep";"#);
        let interner = StableKeyInterner::default();
        let output = extract_ts_scope(&interner, file);
        let import = output
            .bindings
            .iter()
            .find(|binding| binding.binding_kind == TsBindingKind::NamedImport)
            .expect("named import row");

        assert_eq!(import.module_source.as_deref(), Some("./dep"));
        assert_eq!(import.imported_name.as_deref(), Some("named"));
        assert!(matches!(import.status, TsBindingStatus::External { .. }));
    }

    #[test]
    fn object_literal_destructuring_rows_carry_alias_target() {
        let file = fixture_file(r#"const { destructured } = { destructured: localTarget };"#);
        let interner = StableKeyInterner::default();
        let output = extract_ts_scope(&interner, file);
        let binding = output
            .bindings
            .iter()
            .find(|binding| {
                binding.name == "destructured"
                    && binding.binding_kind == TsBindingKind::Destructuring
                    && binding.import_export_kind == TsImportExportKind::Alias
            })
            .expect("destructuring alias binding");

        assert_eq!(binding.imported_name.as_deref(), Some("localTarget"));
    }

    fn fixture_file(source: &str) -> &'static SourceFile {
        let mut db = Box::new(LocalFactDb::new());
        let file_id = db.add_file(
            PathBuf::from("src/scope.ts"),
            "src/scope.ts".to_string(),
            source.to_string(),
        );
        let leaked = Box::leak(db);
        leaked.file(file_id).expect("fixture file")
    }
}
