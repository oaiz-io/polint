#![allow(dead_code, reason = "kept for private internal consumers")]

use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast::ast::{
    Argument, BinaryOperator, BindingPattern, Expression, FunctionType, MethodDefinition,
    MethodDefinitionKind, Program, PropertyKey, VariableDeclarator,
};
use oxc_semantic::{AstNodes, NodeId, SemanticBuilder};
use oxc_span::GetSpan;

use crate::analysis_api::SourceFile;
use crate::internal_core::{Span, StableKeyInterner};
use crate::ts::ids::{TsInventoryCallsiteId, TsInventoryFunctionId};
use crate::ts::inventory::facts::{
    TsCallsiteInventoryKind, TsFunctionInventoryKind, TsInventoryCallsiteFact,
    TsInventoryFunctionFact, TsInventoryStatus,
};
use crate::ts::inventory::store::TsInventoryOutput;
use crate::ts::parse::{PARTIAL_AST_REASON, parse_ts_file};

pub fn extract_ts_inventory(interner: &StableKeyInterner, file: &SourceFile) -> TsInventoryOutput {
    let source = file.source.as_ref();
    let allocator = Allocator::default();
    let parsed = parse_ts_file(&allocator, file);

    if parsed.is_catastrophic() {
        return TsInventoryOutput::default();
    }

    let semantic = SemanticBuilder::new().build(parsed.program()).semantic;
    let mut output = extract_ts_inventory_from_program(
        interner,
        file,
        source,
        parsed.program(),
        semantic.nodes(),
    )
    .normalized(interner);
    if !parsed.fully_parsed {
        mark_inventory_partial_ast(&mut output);
    }
    output
}

pub fn mark_inventory_partial_ast(output: &mut TsInventoryOutput) {
    for function in &mut output.functions {
        if matches!(function.status, TsInventoryStatus::Resolved) {
            function.status = TsInventoryStatus::unsupported(PARTIAL_AST_REASON);
        }
    }
    for callsite in &mut output.callsites {
        if matches!(callsite.status, TsInventoryStatus::Resolved) {
            callsite.status = TsInventoryStatus::unsupported(PARTIAL_AST_REASON);
        }
    }
}

pub fn extract_ts_inventory_from_program(
    interner: &StableKeyInterner,
    file: &SourceFile,
    source: &str,
    _program: &Program<'_>,
    nodes: &AstNodes<'_>,
) -> TsInventoryOutput {
    let mut node_entries = nodes
        .iter_enumerated()
        .filter_map(|(node_id, node)| {
            let kind = function_inventory_kind(nodes, node_id, node.kind())?;
            let span = node.kind().span();
            Some((span.start, span.end, kind, node_id, node.kind()))
        })
        .collect::<Vec<_>>();
    node_entries.sort_by_key(|(start, end, kind, _, _)| (*start, *end, kind.as_str()));

    let mut function_rows = Vec::new();
    for (_, _, kind, node_id, ast_kind) in node_entries {
        let span = span_from_oxc(file, ast_kind.span());
        let display_name = function_display_name(nodes, node_id, ast_kind);
        let lexical_parent_key =
            lexical_parent_key(file, nodes, node_id).map(|key| interner.intern(key));
        let stable_key = interner.intern(function_stable_key(
            file,
            &span,
            kind,
            lexical_parent_key
                .map(|key| interner.resolve(key))
                .as_deref(),
            &display_name,
        ));
        function_rows.push(TsInventoryFunctionFact {
            id: TsInventoryFunctionId(0),
            file: file.id,
            span,
            stable_key,
            lexical_parent_key,
            display_name,
            kind,
            has_body: function_has_body(ast_kind),
            status: TsInventoryStatus::resolved(),
        });
    }

    let mut callsite_entries = nodes
        .iter_enumerated()
        .filter_map(|(node_id, node)| {
            let kind = callsite_inventory_kind(node.kind())?;
            let span = crate::ts::spans::normalized_callsite_span(source, node.kind())?;
            Some((span.start, span.end, kind, node_id, node.kind()))
        })
        .collect::<Vec<_>>();
    callsite_entries.sort_by_key(|(start, end, kind, _, _)| (*start, *end, kind.as_str()));

    let mut callsite_rows = Vec::new();
    for (_, _, kind, node_id, ast_kind) in callsite_entries {
        let span = span_from_oxc(
            file,
            crate::ts::spans::normalized_callsite_span(source, ast_kind)
                .expect("callsite entries have normalized callsite spans"),
        );
        let display_name = callsite_display_name(ast_kind);
        let lexical_parent_key =
            lexical_parent_key(file, nodes, node_id).map(|key| interner.intern(key));
        let status = callsite_status(ast_kind, display_name.as_deref());
        let stable_key = interner.intern(callsite_stable_key(
            file,
            &span,
            kind,
            lexical_parent_key
                .map(|key| interner.resolve(key))
                .as_deref(),
            &display_name,
            &status,
        ));
        callsite_rows.push(TsInventoryCallsiteFact {
            id: TsInventoryCallsiteId(0),
            file: file.id,
            span,
            stable_key,
            lexical_parent_key,
            display_name,
            kind,
            status,
        });
    }

    TsInventoryOutput {
        functions: function_rows,
        callsites: callsite_rows,
    }
}

fn function_inventory_kind(
    nodes: &AstNodes<'_>,
    node_id: NodeId,
    kind: AstKind<'_>,
) -> Option<TsFunctionInventoryKind> {
    match kind {
        AstKind::Function(function) => {
            if matches!(nodes.parent_kind(node_id), AstKind::MethodDefinition(_)) {
                return None;
            }
            match function.r#type {
                FunctionType::FunctionDeclaration | FunctionType::TSDeclareFunction => {
                    Some(TsFunctionInventoryKind::Declaration)
                }
                FunctionType::FunctionExpression | FunctionType::TSEmptyBodyFunctionExpression => {
                    Some(TsFunctionInventoryKind::FunctionExpression)
                }
            }
        }
        AstKind::ArrowFunctionExpression(_) => Some(TsFunctionInventoryKind::Arrow),
        AstKind::MethodDefinition(method) => Some(method_inventory_kind(method)),
        AstKind::StaticBlock(_) => Some(TsFunctionInventoryKind::ClassStaticBlock),
        _ => None,
    }
}

/// Whether an inventoried function node carries a body. Arrows and static
/// blocks always do; a function or method can be a body-less signature.
fn function_has_body(kind: AstKind<'_>) -> bool {
    match kind {
        AstKind::Function(function) => function.body.is_some(),
        AstKind::MethodDefinition(method) => method.value.body.is_some(),
        _ => true,
    }
}

fn method_inventory_kind(method: &MethodDefinition<'_>) -> TsFunctionInventoryKind {
    match method.kind {
        MethodDefinitionKind::Constructor => TsFunctionInventoryKind::Constructor,
        MethodDefinitionKind::Get | MethodDefinitionKind::Set => TsFunctionInventoryKind::Accessor,
        MethodDefinitionKind::Method => TsFunctionInventoryKind::Method,
    }
}

fn callsite_inventory_kind(kind: AstKind<'_>) -> Option<TsCallsiteInventoryKind> {
    match kind {
        AstKind::CallExpression(call) if call.optional => {
            Some(TsCallsiteInventoryKind::OptionalCall)
        }
        AstKind::CallExpression(call)
            if expression_text(&call.callee).as_deref() == Some("require") =>
        {
            Some(TsCallsiteInventoryKind::Require)
        }
        AstKind::CallExpression(_) => Some(TsCallsiteInventoryKind::Call),
        AstKind::NewExpression(_) => Some(TsCallsiteInventoryKind::New),
        AstKind::TaggedTemplateExpression(_) => Some(TsCallsiteInventoryKind::TaggedTemplate),
        AstKind::ImportExpression(_) => Some(TsCallsiteInventoryKind::DynamicImport),
        _ => None,
    }
}

fn function_display_name(
    nodes: &AstNodes<'_>,
    node_id: NodeId,
    kind: AstKind<'_>,
) -> Option<String> {
    match kind {
        AstKind::Function(function) => function
            .id
            .as_ref()
            .map(|id| id.name.to_string())
            .or_else(|| variable_declarator_name(nodes, node_id)),
        AstKind::ArrowFunctionExpression(_) => variable_declarator_name(nodes, node_id),
        AstKind::MethodDefinition(method) => method_name(method),
        AstKind::StaticBlock(_) => Some("static".to_string()),
        _ => None,
    }
}

fn variable_declarator_name(nodes: &AstNodes<'_>, node_id: NodeId) -> Option<String> {
    match nodes.parent_kind(node_id) {
        AstKind::VariableDeclarator(declarator) => binding_identifier_name(declarator),
        _ => None,
    }
}

fn binding_identifier_name(declarator: &VariableDeclarator<'_>) -> Option<String> {
    match &declarator.id {
        BindingPattern::BindingIdentifier(identifier) => Some(identifier.name.to_string()),
        _ => None,
    }
}

fn method_name(method: &MethodDefinition<'_>) -> Option<String> {
    match &method.key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::PrivateIdentifier(identifier) => Some(format!("#{}", identifier.name)),
        PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
        PropertyKey::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
            Some(format!(
                "{}{}",
                constant_property_key_expression(&binary.left)?,
                constant_property_key_expression(&binary.right)?
            ))
        }
        _ => None,
    }
}

fn constant_property_key_expression(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::StringLiteral(literal) => Some(literal.value.to_string()),
        Expression::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
            Some(format!(
                "{}{}",
                constant_property_key_expression(&binary.left)?,
                constant_property_key_expression(&binary.right)?
            ))
        }
        Expression::ParenthesizedExpression(expression) => {
            constant_property_key_expression(&expression.expression)
        }
        _ => None,
    }
}

fn callsite_display_name(kind: AstKind<'_>) -> Option<String> {
    match kind {
        AstKind::CallExpression(call) => expression_text(&call.callee),
        AstKind::NewExpression(expression) => expression_text(&expression.callee),
        AstKind::TaggedTemplateExpression(expression) => expression_text(&expression.tag),
        AstKind::ImportExpression(expression) => match &expression.source {
            Expression::StringLiteral(literal) => Some(format!("import:{}", literal.value)),
            _ => Some("import:<dynamic>".to_string()),
        },
        _ => None,
    }
}

fn callsite_status(kind: AstKind<'_>, display_name: Option<&str>) -> TsInventoryStatus {
    match kind {
        AstKind::ImportExpression(expression) => match &expression.source {
            Expression::StringLiteral(_) => TsInventoryStatus::resolved(),
            _ => TsInventoryStatus::unresolved("non-string dynamic import"),
        },
        AstKind::CallExpression(call)
            if expression_text(&call.callee).as_deref() == Some("require") =>
        {
            if matches!(call.arguments.first(), Some(Argument::StringLiteral(_))) {
                TsInventoryStatus::resolved()
            } else {
                TsInventoryStatus::unresolved("non-string require")
            }
        }
        AstKind::CallExpression(_)
        | AstKind::NewExpression(_)
        | AstKind::TaggedTemplateExpression(_) => {
            if display_name.is_some() {
                TsInventoryStatus::resolved()
            } else {
                TsInventoryStatus::unresolved("dynamic callee")
            }
        }
        _ => TsInventoryStatus::unsupported("unsupported callsite syntax"),
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

fn lexical_parent_key(file: &SourceFile, nodes: &AstNodes<'_>, node_id: NodeId) -> Option<String> {
    nodes
        .ancestors_enumerated(node_id)
        .find_map(|(ancestor_id, node)| {
            let kind = node.kind();
            let inventory_kind = match kind {
                AstKind::Function(function) => match function.r#type {
                    FunctionType::FunctionDeclaration | FunctionType::TSDeclareFunction => {
                        TsFunctionInventoryKind::Declaration
                    }
                    FunctionType::FunctionExpression
                    | FunctionType::TSEmptyBodyFunctionExpression => {
                        TsFunctionInventoryKind::FunctionExpression
                    }
                },
                AstKind::ArrowFunctionExpression(_) => TsFunctionInventoryKind::Arrow,
                AstKind::MethodDefinition(method) => method_inventory_kind(method),
                AstKind::StaticBlock(_) => TsFunctionInventoryKind::ClassStaticBlock,
                _ => return None,
            };
            let display_name = function_display_name(nodes, ancestor_id, kind);
            Some(parent_key(file, kind.span(), inventory_kind, &display_name))
        })
        .or_else(|| Some(format!("ts_inventory_parent:{}:module", file.relative_path)))
}

fn parent_key(
    file: &SourceFile,
    span: oxc_span::Span,
    kind: TsFunctionInventoryKind,
    display_name: &Option<String>,
) -> String {
    stable_inventory_key(
        "ts_inventory_parent",
        &[
            ("file", file.relative_path.clone()),
            ("kind", kind.as_str().to_string()),
            ("start", span.start.to_string()),
            ("end", span.end.to_string()),
            (
                "display",
                display_name.as_deref().unwrap_or("<anonymous>").to_string(),
            ),
        ],
    )
}

fn function_stable_key(
    file: &SourceFile,
    span: &Span,
    kind: TsFunctionInventoryKind,
    lexical_parent_key: Option<&str>,
    display_name: &Option<String>,
) -> String {
    stable_inventory_key(
        "ts_inventory_function",
        &[
            ("file", file.relative_path.clone()),
            ("kind", kind.as_str().to_string()),
            ("start", span.start_byte.to_string()),
            ("end", span.end_byte.to_string()),
            (
                "parent",
                lexical_parent_key.unwrap_or("<module>").to_string(),
            ),
            (
                "display",
                display_name.as_deref().unwrap_or("<anonymous>").to_string(),
            ),
        ],
    )
}

fn callsite_stable_key(
    file: &SourceFile,
    span: &Span,
    kind: TsCallsiteInventoryKind,
    lexical_parent_key: Option<&str>,
    display_name: &Option<String>,
    status: &TsInventoryStatus,
) -> String {
    stable_inventory_key(
        "ts_inventory_callsite",
        &[
            ("file", file.relative_path.clone()),
            ("kind", kind.as_str().to_string()),
            ("start", span.start_byte.to_string()),
            ("end", span.end_byte.to_string()),
            (
                "parent",
                lexical_parent_key.unwrap_or("<module>").to_string(),
            ),
            (
                "display",
                display_name.as_deref().unwrap_or("<dynamic>").to_string(),
            ),
            ("status", inventory_status_label(status).to_string()),
        ],
    )
}

fn inventory_status_label(status: &TsInventoryStatus) -> &'static str {
    match status {
        TsInventoryStatus::Resolved => "resolved",
        TsInventoryStatus::Unresolved { .. } => "unresolved",
        TsInventoryStatus::Unsupported { .. } => "unsupported",
    }
}

fn stable_inventory_key(prefix: &str, parts: &[(&str, String)]) -> String {
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
