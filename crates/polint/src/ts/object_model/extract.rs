#![allow(dead_code, reason = "kept for private internal consumers")]

use std::collections::BTreeMap;

use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast::ast::{
    Argument, AssignmentTarget, BinaryOperator, BindingPattern, Class, Expression, Function,
    FunctionType, MethodDefinition, MethodDefinitionKind, ObjectPropertyKind, Program, PropertyKey,
    VariableDeclarator,
};
use oxc_semantic::{AstNodes, NodeId, Scoping, SemanticBuilder, SymbolId as OxcSymbolId};
use oxc_span::GetSpan;

use crate::analysis_api::SourceFile;
use crate::internal_core::{Span, StableKeyInterner};
use crate::ts::inventory::extract::extract_ts_inventory_from_program;
use crate::ts::inventory::store::TsInventoryOutput;
use crate::ts::object_model::facts::{
    TsObjectAllocationFact, TsObjectAllocationId, TsObjectAllocationKind, TsObjectModelStatus,
    TsPropertyKey, TsPropertyKeyKind, TsPropertyReadFact, TsPropertyReadId, TsPropertyWriteFact,
    TsPropertyWriteId, TsPrototypeLinkFact, TsPrototypeLinkId, TsPrototypeLinkKind,
    TsReceiverBindingFact, TsReceiverBindingId, TsReceiverBindingKind,
};
use crate::ts::object_model::store::TsObjectModelOutput;
use crate::ts::parse::{PARTIAL_AST_REASON, parse_ts_file};

pub fn extract_ts_object_model(
    interner: &StableKeyInterner,
    file: &SourceFile,
) -> TsObjectModelOutput {
    let source = file.source.as_ref();
    let allocator = Allocator::default();
    let parsed = parse_ts_file(&allocator, file);

    if parsed.is_catastrophic() {
        return TsObjectModelOutput::default();
    }

    let semantic = SemanticBuilder::new().build(parsed.program()).semantic;
    let inventory = extract_ts_inventory_from_program(
        interner,
        file,
        source,
        parsed.program(),
        semantic.nodes(),
    )
    .normalized(interner);
    let mut output = extract_ts_object_model_from_program(
        interner,
        file,
        source,
        parsed.program(),
        semantic.scoping(),
        semantic.nodes(),
        &inventory,
    );
    if !parsed.fully_parsed {
        mark_object_model_partial_ast(&mut output);
    }
    output
}

pub fn mark_object_model_partial_ast(output: &mut TsObjectModelOutput) {
    let unsupported = TsObjectModelStatus::unsupported(PARTIAL_AST_REASON);
    for row in &mut output.allocations {
        if matches!(row.status, TsObjectModelStatus::Resolved) {
            row.status = unsupported.clone();
        }
    }
    for row in &mut output.property_writes {
        if matches!(row.status, TsObjectModelStatus::Resolved) {
            row.status = unsupported.clone();
        }
    }
    for row in &mut output.property_reads {
        if matches!(row.status, TsObjectModelStatus::Resolved) {
            row.status = unsupported.clone();
        }
    }
    for row in &mut output.receiver_bindings {
        if matches!(row.status, TsObjectModelStatus::Resolved) {
            row.status = unsupported.clone();
        }
    }
    for row in &mut output.prototype_links {
        if matches!(row.status, TsObjectModelStatus::Resolved) {
            row.status = unsupported.clone();
        }
    }
}

pub fn extract_ts_object_model_from_program(
    interner: &StableKeyInterner,
    file: &SourceFile,
    source: &str,
    _program: &Program<'_>,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    inventory: &TsInventoryOutput,
) -> TsObjectModelOutput {
    crate::ts::with_frontend_stable_keys(interner, || {
        let mut extractor = ObjectModelExtractor::new(file, source, scoping, nodes, inventory);
        extractor.build()
    })
}

struct ObjectModelExtractor<'a> {
    file: &'a SourceFile,
    source: &'a str,
    scoping: &'a Scoping,
    nodes: &'a AstNodes<'a>,
    inventory: ObjectModelInventoryIndex,
    objects_by_symbol: BTreeMap<OxcSymbolId, String>,
    objects_by_name: BTreeMap<String, Option<String>>,
    allocation_keys_by_span: BTreeMap<(u32, u32, TsObjectAllocationKind), String>,
    class_prototypes_by_symbol: BTreeMap<OxcSymbolId, String>,
    class_prototypes_by_name: BTreeMap<String, Option<String>>,
    symbols_by_declaration: BTreeMap<(NodeId, String), OxcSymbolId>,
    reference_symbols_by_span: BTreeMap<(u32, u32), OxcSymbolId>,
}

impl<'a> ObjectModelExtractor<'a> {
    fn new(
        file: &'a SourceFile,
        source: &'a str,
        scoping: &'a Scoping,
        nodes: &'a AstNodes<'a>,
        inventory: &TsInventoryOutput,
    ) -> Self {
        Self {
            file,
            source,
            scoping,
            nodes,
            inventory: ObjectModelInventoryIndex::from_output(inventory),
            objects_by_symbol: BTreeMap::new(),
            objects_by_name: BTreeMap::new(),
            allocation_keys_by_span: BTreeMap::new(),
            class_prototypes_by_symbol: BTreeMap::new(),
            class_prototypes_by_name: BTreeMap::new(),
            symbols_by_declaration: symbols_by_declaration(scoping),
            reference_symbols_by_span: reference_symbols_by_span(scoping, nodes),
        }
    }

    fn build(&mut self) -> TsObjectModelOutput {
        self.index_allocations();
        self.index_class_prototypes();

        let mut output = TsObjectModelOutput::default();
        let mut entries = self
            .nodes
            .iter_enumerated()
            .map(|(node_id, node)| {
                let span = node.kind().span();
                (span.start, span.end, node_id, node.kind())
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|(start, end, node_id, _)| (*start, *end, node_id.index()));

        for (_, _, node_id, kind) in entries {
            match kind {
                AstKind::ObjectExpression(object) => {
                    self.record_object_expression(node_id, object, &mut output);
                }
                AstKind::ArrayExpression(array) => {
                    output.allocations.push(self.allocation_row(
                        node_id,
                        array.span,
                        TsObjectAllocationKind::ArrayLiteral,
                        self.allocation_display_name(node_id, kind),
                    ));
                }
                AstKind::Class(class) => self.record_class(node_id, class, &mut output),
                AstKind::NewExpression(expression) => {
                    self.record_new_expression(node_id, expression, &mut output);
                }
                AstKind::Function(function)
                    if allocates_function_object(self.nodes, node_id, function) =>
                {
                    output.allocations.push(
                        self.allocation_row(
                            node_id,
                            function.span,
                            TsObjectAllocationKind::FunctionObject,
                            function
                                .id
                                .as_ref()
                                .map(|identifier| identifier.name.to_string())
                                .or_else(|| variable_declarator_name(self.nodes, node_id)),
                        ),
                    );
                }
                AstKind::ArrowFunctionExpression(function) => {
                    output.allocations.push(self.allocation_row(
                        node_id,
                        function.span,
                        TsObjectAllocationKind::FunctionObject,
                        variable_declarator_name(self.nodes, node_id),
                    ));
                    output.receiver_bindings.push(self.receiver_row(
                        function.span,
                        TsReceiverBindingKind::LexicalThis,
                        None,
                        None,
                        None,
                        Some("lexical-this".to_string()),
                    ));
                }
                AstKind::StaticMemberExpression(member) => {
                    output
                        .property_reads
                        .push(self.static_member_read(node_id, member));
                }
                AstKind::ComputedMemberExpression(member) => {
                    output
                        .property_reads
                        .push(self.computed_member_read(node_id, member));
                }
                AstKind::PrivateFieldExpression(member) => {
                    output
                        .property_reads
                        .push(self.private_member_read(node_id, member));
                }
                AstKind::AssignmentExpression(assignment) => {
                    if let Some(write) =
                        self.assignment_member_write(&assignment.left, &assignment.right)
                    {
                        output.property_writes.push(write);
                    }
                }
                AstKind::CallExpression(call) => {
                    if let Some(receiver) = self.receiver_binding_for_call(node_id, call) {
                        output.receiver_bindings.push(receiver);
                    }
                }
                _ => {}
            }
        }

        output
    }

    fn index_allocations(&mut self) {
        let entries = self
            .nodes
            .iter_enumerated()
            .filter_map(|(node_id, node)| {
                let ast_kind = node.kind();
                let kind = allocation_kind_for_node(self.nodes, node_id, ast_kind)?;
                let span = ast_kind.span();
                let lexical_parent_key = self.lexical_parent_key(node_id);
                let display_name = self.allocation_display_name(node_id, ast_kind);
                let stable_key = self.allocation_stable_key(
                    span,
                    kind,
                    lexical_parent_key.as_deref(),
                    display_name.as_deref(),
                );
                let binding_name = allocation_binding_name(self.nodes, node_id, ast_kind);
                let binding_symbol = binding_name
                    .as_deref()
                    .and_then(|name| self.declaration_symbol_for_node_name(node_id, name));
                Some((
                    span.start,
                    span.end,
                    kind,
                    stable_key,
                    binding_name,
                    binding_symbol,
                ))
            })
            .collect::<Vec<_>>();

        for (start, end, kind, stable_key, binding_name, binding_symbol) in entries {
            self.allocation_keys_by_span
                .insert((start, end, kind), stable_key.clone());
            if let Some(symbol) = binding_symbol {
                self.objects_by_symbol.insert(symbol, stable_key.clone());
            }
            if let Some(name) = binding_name {
                insert_unique(&mut self.objects_by_name, name, stable_key);
            }
        }
    }

    fn index_class_prototypes(&mut self) {
        let entries = self
            .nodes
            .iter_enumerated()
            .filter_map(|(node_id, node)| {
                let AstKind::Class(class) = node.kind() else {
                    return None;
                };
                let name = class_name(self.nodes, node_id, class)?;
                let class_key = self.allocation_stable_key(
                    class.span,
                    TsObjectAllocationKind::ClassObject,
                    self.lexical_parent_key(node_id).as_deref(),
                    Some(&name),
                );
                let prototype_key = format!("{class_key}|prototype");
                let symbol = self.declaration_symbol_for_node_name(node_id, &name);
                Some((name, prototype_key, symbol))
            })
            .collect::<Vec<_>>();

        for (name, prototype_key, symbol) in entries {
            if let Some(symbol) = symbol {
                self.class_prototypes_by_symbol
                    .insert(symbol, prototype_key.clone());
            }
            insert_unique(&mut self.class_prototypes_by_name, name, prototype_key);
        }
    }

    fn record_object_expression(
        &self,
        node_id: NodeId,
        object: &oxc_ast::ast::ObjectExpression<'_>,
        output: &mut TsObjectModelOutput,
    ) {
        let base_stable_key = self.allocation_stable_key(
            object.span,
            TsObjectAllocationKind::ObjectLiteral,
            self.lexical_parent_key(node_id).as_deref(),
            variable_declarator_name(self.nodes, node_id).as_deref(),
        );
        output.allocations.push(self.allocation_row(
            node_id,
            object.span,
            TsObjectAllocationKind::ObjectLiteral,
            variable_declarator_name(self.nodes, node_id),
        ));

        for property in &object.properties {
            match property {
                ObjectPropertyKind::ObjectProperty(property) => {
                    let property_key = property_key(&property.key, property.computed);
                    output.property_writes.push(TsPropertyWriteFact {
                        id: TsPropertyWriteId(0),
                        file: self.file.id,
                        span: span_from_oxc(self.file, property.span),
                        stable_key: crate::ts::intern_frontend_stable_key(
                            self.operation_stable_key(
                                "ts_object_property_write",
                                property.span,
                                &base_stable_key,
                                &property_key.stable_label(),
                            ),
                        ),
                        base_object_stable_key: crate::ts::intern_frontend_stable_key(
                            base_stable_key.clone(),
                        ),
                        property_key,
                        value_function: None,
                        value_function_stable_key: self
                            .object_property_function_key(property)
                            .map(crate::ts::intern_frontend_stable_key),
                        value_object_stable_key: self
                            .object_key_for_expression(&property.value)
                            .map(crate::ts::intern_frontend_stable_key),
                        status: TsObjectModelStatus::resolved(),
                    });
                }
                ObjectPropertyKind::SpreadProperty(spread) => {
                    let property_key = TsPropertyKey {
                        kind: TsPropertyKeyKind::ComputedBucket,
                        value: None,
                    };
                    output.property_writes.push(TsPropertyWriteFact {
                        id: TsPropertyWriteId(0),
                        file: self.file.id,
                        span: span_from_oxc(self.file, spread.span),
                        stable_key: crate::ts::intern_frontend_stable_key(
                            self.operation_stable_key(
                                "ts_object_property_write",
                                spread.span,
                                &base_stable_key,
                                "computed_bucket",
                            ),
                        ),
                        base_object_stable_key: crate::ts::intern_frontend_stable_key(
                            base_stable_key.clone(),
                        ),
                        property_key,
                        value_function: None,
                        value_function_stable_key: None,
                        value_object_stable_key: self
                            .object_key_for_expression(&spread.argument)
                            .map(crate::ts::intern_frontend_stable_key),
                        status: TsObjectModelStatus::unknown("spread property"),
                    });
                }
            }
        }
    }

    fn record_class(
        &mut self,
        node_id: NodeId,
        class: &Class<'_>,
        output: &mut TsObjectModelOutput,
    ) {
        let class_name = class_name(self.nodes, node_id, class);
        let class_key = self.allocation_stable_key(
            class.span,
            TsObjectAllocationKind::ClassObject,
            self.lexical_parent_key(node_id).as_deref(),
            class_name.as_deref(),
        );
        let prototype_key = format!("{class_key}|prototype");

        output.allocations.push(self.allocation_row(
            node_id,
            class.span,
            TsObjectAllocationKind::ClassObject,
            class_name,
        ));
        output.allocations.push(TsObjectAllocationFact {
            id: TsObjectAllocationId(0),
            file: self.file.id,
            span: span_from_oxc(self.file, class.body.span),
            stable_key: crate::ts::intern_frontend_stable_key(prototype_key.clone()),
            lexical_parent_key: self
                .lexical_parent_key(node_id)
                .map(crate::ts::intern_frontend_stable_key),
            inventory_function: None,
            inventory_function_stable_key: None,
            inventory_callsite: None,
            inventory_callsite_stable_key: None,
            kind: TsObjectAllocationKind::PrototypeObject,
            status: TsObjectModelStatus::resolved(),
        });
        output.prototype_links.push(TsPrototypeLinkFact {
            id: TsPrototypeLinkId(0),
            file: self.file.id,
            span: span_from_oxc(self.file, class.span),
            stable_key: crate::ts::intern_frontend_stable_key(self.operation_stable_key(
                "ts_object_prototype_link",
                class.span,
                &class_key,
                &prototype_key,
            )),
            kind: TsPrototypeLinkKind::ClassPrototype,
            object_stable_key: crate::ts::intern_frontend_stable_key(class_key.clone()),
            prototype_stable_key: crate::ts::intern_frontend_stable_key(prototype_key.clone()),
            property_key: None,
            status: TsObjectModelStatus::resolved(),
        });

        if let Some(super_class) = &class.super_class
            && let Some(super_name) = expression_text(super_class)
        {
            let super_prototype = self
                .class_prototype_key_for_expression(super_class)
                .unwrap_or_else(|| format!("ts_class_prototype:{super_name}"));
            output.prototype_links.push(TsPrototypeLinkFact {
                id: TsPrototypeLinkId(0),
                file: self.file.id,
                span: span_from_oxc(self.file, super_class.span()),
                stable_key: crate::ts::intern_frontend_stable_key(self.operation_stable_key(
                    "ts_object_prototype_link",
                    super_class.span(),
                    &prototype_key,
                    &super_prototype,
                )),
                kind: TsPrototypeLinkKind::ClassExtends,
                object_stable_key: crate::ts::intern_frontend_stable_key(prototype_key.clone()),
                prototype_stable_key: crate::ts::intern_frontend_stable_key(super_prototype),
                property_key: None,
                status: TsObjectModelStatus::resolved(),
            });
        }

        for element in &class.body.body {
            let oxc_ast::ast::ClassElement::MethodDefinition(method) = element else {
                continue;
            };
            self.record_method_property_write(method, &class_key, &prototype_key, output);
        }
    }

    fn record_method_property_write(
        &self,
        method: &MethodDefinition<'_>,
        class_key: &str,
        prototype_key: &str,
        output: &mut TsObjectModelOutput,
    ) {
        let property_key = property_key(&method.key, method.computed);
        let base_object_stable_key = if method.r#static {
            class_key.to_string()
        } else {
            prototype_key.to_string()
        };
        output.property_writes.push(TsPropertyWriteFact {
            id: TsPropertyWriteId(0),
            file: self.file.id,
            span: span_from_oxc(self.file, method.span),
            stable_key: crate::ts::intern_frontend_stable_key(self.operation_stable_key(
                "ts_object_property_write",
                method.span,
                &base_object_stable_key,
                &property_key.stable_label(),
            )),
            base_object_stable_key: crate::ts::intern_frontend_stable_key(base_object_stable_key),
            property_key,
            value_function: None,
            value_function_stable_key: self
                .method_function_key(method)
                .map(crate::ts::intern_frontend_stable_key),
            value_object_stable_key: None,
            status: TsObjectModelStatus::resolved(),
        });
    }

    fn record_new_expression(
        &self,
        node_id: NodeId,
        expression: &oxc_ast::ast::NewExpression<'_>,
        output: &mut TsObjectModelOutput,
    ) {
        let class_name = expression_text(&expression.callee);
        let display_name =
            variable_declarator_name(self.nodes, node_id).or_else(|| class_name.clone());
        let instance_key = self.allocation_stable_key(
            expression.span,
            TsObjectAllocationKind::ClassInstance,
            self.lexical_parent_key(node_id).as_deref(),
            display_name.as_deref(),
        );
        output.allocations.push(self.allocation_row(
            node_id,
            expression.span,
            TsObjectAllocationKind::ClassInstance,
            display_name,
        ));
        if let Some(prototype_key) = class_name
            .as_ref()
            .and_then(|_| self.class_prototype_key_for_expression(&expression.callee))
        {
            output.prototype_links.push(TsPrototypeLinkFact {
                id: TsPrototypeLinkId(0),
                file: self.file.id,
                span: span_from_oxc(self.file, expression.span),
                stable_key: crate::ts::intern_frontend_stable_key(self.operation_stable_key(
                    "ts_object_prototype_link",
                    expression.span,
                    &instance_key,
                    &prototype_key,
                )),
                kind: TsPrototypeLinkKind::ConstructorPrototypeProperty,
                object_stable_key: crate::ts::intern_frontend_stable_key(instance_key),
                prototype_stable_key: crate::ts::intern_frontend_stable_key(prototype_key.clone()),
                property_key: None,
                status: TsObjectModelStatus::resolved(),
            });
        }
    }

    fn static_member_read(
        &self,
        node_id: NodeId,
        member: &oxc_ast::ast::StaticMemberExpression<'_>,
    ) -> TsPropertyReadFact {
        let property_key = TsPropertyKey {
            kind: TsPropertyKeyKind::Static,
            value: Some(member.property.name.to_string()),
        };
        self.member_read(node_id, member.span, &member.object, property_key)
    }

    fn computed_member_read(
        &self,
        node_id: NodeId,
        member: &oxc_ast::ast::ComputedMemberExpression<'_>,
    ) -> TsPropertyReadFact {
        let property_key = property_key_from_expression(&member.expression);
        self.member_read(node_id, member.span, &member.object, property_key)
    }

    fn private_member_read(
        &self,
        node_id: NodeId,
        member: &oxc_ast::ast::PrivateFieldExpression<'_>,
    ) -> TsPropertyReadFact {
        let property_key = TsPropertyKey {
            kind: TsPropertyKeyKind::Private,
            value: Some(member.field.name.to_string()),
        };
        self.member_read(node_id, member.span, &member.object, property_key)
    }

    fn member_read(
        &self,
        node_id: NodeId,
        span: oxc_span::Span,
        object: &Expression<'_>,
        property_key: TsPropertyKey,
    ) -> TsPropertyReadFact {
        let base_object_stable_key = self
            .object_key_for_expression(object)
            .unwrap_or_else(|| expression_object_key(object));
        let callsite_stable_key = self.parent_callsite_key(node_id);
        TsPropertyReadFact {
            id: TsPropertyReadId(0),
            file: self.file.id,
            span: span_from_oxc(self.file, span),
            stable_key: crate::ts::intern_frontend_stable_key(self.operation_stable_key(
                "ts_object_property_read",
                span,
                &base_object_stable_key,
                &property_key.stable_label(),
            )),
            base_object_stable_key: crate::ts::intern_frontend_stable_key(base_object_stable_key),
            property_key,
            destination_stable_key: None,
            callsite: None,
            callsite_stable_key: callsite_stable_key.map(crate::ts::intern_frontend_stable_key),
            status: TsObjectModelStatus::resolved(),
        }
    }

    fn assignment_member_write(
        &self,
        left: &AssignmentTarget<'_>,
        right: &Expression<'_>,
    ) -> Option<TsPropertyWriteFact> {
        let (span, object, property_key) = match left {
            AssignmentTarget::StaticMemberExpression(member) => (
                member.span,
                &member.object,
                TsPropertyKey {
                    kind: TsPropertyKeyKind::Static,
                    value: Some(member.property.name.to_string()),
                },
            ),
            AssignmentTarget::ComputedMemberExpression(member) => (
                member.span,
                &member.object,
                property_key_from_expression(&member.expression),
            ),
            AssignmentTarget::PrivateFieldExpression(member) => (
                member.span,
                &member.object,
                TsPropertyKey {
                    kind: TsPropertyKeyKind::Private,
                    value: Some(member.field.name.to_string()),
                },
            ),
            _ => return None,
        };
        let base_object_stable_key = self
            .object_key_for_expression(object)
            .unwrap_or_else(|| expression_object_key(object));
        Some(TsPropertyWriteFact {
            id: TsPropertyWriteId(0),
            file: self.file.id,
            span: span_from_oxc(self.file, span),
            stable_key: crate::ts::intern_frontend_stable_key(self.operation_stable_key(
                "ts_object_property_write",
                span,
                &base_object_stable_key,
                &property_key.stable_label(),
            )),
            base_object_stable_key: crate::ts::intern_frontend_stable_key(base_object_stable_key),
            property_key,
            value_function: None,
            value_function_stable_key: self
                .value_function_key(right)
                .map(crate::ts::intern_frontend_stable_key),
            value_object_stable_key: self
                .object_key_for_expression(right)
                .map(crate::ts::intern_frontend_stable_key),
            status: TsObjectModelStatus::resolved(),
        })
    }

    fn receiver_binding_for_call(
        &self,
        node_id: NodeId,
        call: &oxc_ast::ast::CallExpression<'_>,
    ) -> Option<TsReceiverBindingFact> {
        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };
        let name = member.property.name.as_str();
        let kind = match name {
            "bind" => TsReceiverBindingKind::BoundFunction,
            "call" => TsReceiverBindingKind::CallReceiver,
            "apply" => TsReceiverBindingKind::ApplyReceiver,
            _ => TsReceiverBindingKind::MethodCall,
        };
        let receiver = if matches!(
            kind,
            TsReceiverBindingKind::BoundFunction
                | TsReceiverBindingKind::CallReceiver
                | TsReceiverBindingKind::ApplyReceiver
        ) {
            call.arguments.first().and_then(argument_expression)
        } else {
            Some(&member.object)
        };
        Some(self.receiver_row(
            call.span,
            kind,
            Some(expression_object_key(&member.object)),
            receiver.and_then(|expression| self.object_key_for_expression(expression)),
            self.callsite_key_for_call(node_id),
            None,
        ))
    }

    fn receiver_row(
        &self,
        span: oxc_span::Span,
        kind: TsReceiverBindingKind,
        callee_function_stable_key: Option<String>,
        receiver_object_stable_key: Option<String>,
        callsite_stable_key: Option<String>,
        lexical_parent_key: Option<String>,
    ) -> TsReceiverBindingFact {
        TsReceiverBindingFact {
            id: TsReceiverBindingId(0),
            file: self.file.id,
            span: span_from_oxc(self.file, span),
            stable_key: crate::ts::intern_frontend_stable_key(
                self.operation_stable_key(
                    "ts_object_receiver_binding",
                    span,
                    kind.as_str(),
                    receiver_object_stable_key
                        .as_deref()
                        .unwrap_or("unknown_receiver"),
                ),
            ),
            kind,
            callsite: None,
            callsite_stable_key: callsite_stable_key.map(crate::ts::intern_frontend_stable_key),
            callee_function: None,
            callee_function_stable_key: callee_function_stable_key
                .map(crate::ts::intern_frontend_stable_key),
            receiver_object_stable_key: receiver_object_stable_key
                .map(crate::ts::intern_frontend_stable_key),
            receiver_place_stable_key: None,
            lexical_parent_key: lexical_parent_key.map(crate::ts::intern_frontend_stable_key),
            status: TsObjectModelStatus::resolved(),
        }
    }

    fn allocation_row(
        &self,
        node_id: NodeId,
        span: oxc_span::Span,
        kind: TsObjectAllocationKind,
        display_name: Option<String>,
    ) -> TsObjectAllocationFact {
        TsObjectAllocationFact {
            id: TsObjectAllocationId(0),
            file: self.file.id,
            span: span_from_oxc(self.file, span),
            stable_key: crate::ts::intern_frontend_stable_key(self.allocation_stable_key(
                span,
                kind,
                self.lexical_parent_key(node_id).as_deref(),
                display_name.as_deref(),
            )),
            lexical_parent_key: self
                .lexical_parent_key(node_id)
                .map(crate::ts::intern_frontend_stable_key),
            inventory_function: None,
            inventory_function_stable_key: None,
            inventory_callsite: None,
            inventory_callsite_stable_key: None,
            kind,
            status: TsObjectModelStatus::resolved(),
        }
    }

    fn object_key_for_expression(&self, expression: &Expression<'_>) -> Option<String> {
        match expression {
            Expression::Identifier(identifier) => {
                self.object_key_for_identifier(expression.span(), identifier.name.as_str())
            }
            Expression::ObjectExpression(object) => {
                self.allocation_key_for_span(object.span, TsObjectAllocationKind::ObjectLiteral)
            }
            Expression::ArrayExpression(array) => {
                self.allocation_key_for_span(array.span, TsObjectAllocationKind::ArrayLiteral)
            }
            Expression::NewExpression(expression) => {
                self.allocation_key_for_span(expression.span, TsObjectAllocationKind::ClassInstance)
            }
            Expression::ParenthesizedExpression(expression) => {
                self.object_key_for_expression(&expression.expression)
            }
            Expression::TSAsExpression(expression) => {
                self.object_key_for_expression(&expression.expression)
            }
            Expression::TSSatisfiesExpression(expression) => {
                self.object_key_for_expression(&expression.expression)
            }
            Expression::TSNonNullExpression(expression) => {
                self.object_key_for_expression(&expression.expression)
            }
            Expression::TSTypeAssertion(expression) => {
                self.object_key_for_expression(&expression.expression)
            }
            _ => None,
        }
    }

    fn object_key_for_identifier(&self, span: oxc_span::Span, name: &str) -> Option<String> {
        self.reference_symbols_by_span
            .get(&(span.start, span.end))
            .and_then(|symbol| self.objects_by_symbol.get(symbol))
            .cloned()
            .or_else(|| {
                self.objects_by_name
                    .get(name)
                    .and_then(Option::as_ref)
                    .cloned()
            })
    }

    fn class_prototype_key_for_expression(&self, expression: &Expression<'_>) -> Option<String> {
        match expression {
            Expression::Identifier(identifier) => {
                self.class_prototype_key_for_identifier(expression.span(), identifier.name.as_str())
            }
            Expression::ParenthesizedExpression(expression) => {
                self.class_prototype_key_for_expression(&expression.expression)
            }
            Expression::TSAsExpression(expression) => {
                self.class_prototype_key_for_expression(&expression.expression)
            }
            Expression::TSSatisfiesExpression(expression) => {
                self.class_prototype_key_for_expression(&expression.expression)
            }
            Expression::TSNonNullExpression(expression) => {
                self.class_prototype_key_for_expression(&expression.expression)
            }
            Expression::TSTypeAssertion(expression) => {
                self.class_prototype_key_for_expression(&expression.expression)
            }
            _ => expression_text(expression).and_then(|name| {
                self.class_prototypes_by_name
                    .get(&name)
                    .and_then(Option::as_ref)
                    .cloned()
            }),
        }
    }

    fn class_prototype_key_for_identifier(
        &self,
        span: oxc_span::Span,
        name: &str,
    ) -> Option<String> {
        self.reference_symbols_by_span
            .get(&(span.start, span.end))
            .and_then(|symbol| self.class_prototypes_by_symbol.get(symbol))
            .cloned()
            .or_else(|| {
                self.class_prototypes_by_name
                    .get(name)
                    .and_then(Option::as_ref)
                    .cloned()
            })
    }

    fn value_function_key(&self, expression: &Expression<'_>) -> Option<String> {
        match expression {
            Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => {
                self.function_key_for_span(expression.span())
            }
            Expression::ParenthesizedExpression(expression) => {
                self.value_function_key(&expression.expression)
            }
            Expression::TSAsExpression(expression) => {
                self.value_function_key(&expression.expression)
            }
            Expression::TSSatisfiesExpression(expression) => {
                self.value_function_key(&expression.expression)
            }
            Expression::TSNonNullExpression(expression) => {
                self.value_function_key(&expression.expression)
            }
            Expression::TSTypeAssertion(expression) => {
                self.value_function_key(&expression.expression)
            }
            _ => expression_text(expression)
                .and_then(|name| self.inventory.unique_function_key(&name).cloned()),
        }
    }

    fn object_property_function_key(
        &self,
        property: &oxc_ast::ast::ObjectProperty<'_>,
    ) -> Option<String> {
        self.value_function_key(&property.value)
            .or_else(|| self.function_key_for_span(property.span))
    }

    fn parent_callsite_key(&self, node_id: NodeId) -> Option<String> {
        match self.nodes.parent_kind(node_id) {
            AstKind::CallExpression(call) => self.callsite_key_for_span(call.span),
            _ => None,
        }
    }

    fn callsite_key_for_call(&self, node_id: NodeId) -> Option<String> {
        match self.nodes.get_node(node_id).kind() {
            AstKind::CallExpression(call) => self.callsite_key_for_span(call.span),
            _ => None,
        }
    }

    fn callsite_key_for_span(&self, span: oxc_span::Span) -> Option<String> {
        self.inventory
            .callsite_key_by_span
            .get(&(span.start, span.end))
            .cloned()
    }

    fn method_function_key(&self, method: &MethodDefinition<'_>) -> Option<String> {
        self.function_key_for_span(method.span)
    }

    fn function_key_for_span(&self, span: oxc_span::Span) -> Option<String> {
        self.inventory
            .function_key_by_span
            .get(&(span.start, span.end))
            .cloned()
    }

    fn allocation_key_for_span(
        &self,
        span: oxc_span::Span,
        kind: TsObjectAllocationKind,
    ) -> Option<String> {
        self.allocation_keys_by_span
            .get(&(span.start, span.end, kind))
            .cloned()
    }

    fn allocation_display_name(&self, node_id: NodeId, kind: AstKind<'_>) -> Option<String> {
        match kind {
            AstKind::Function(function) => function
                .id
                .as_ref()
                .map(|identifier| identifier.name.to_string())
                .or_else(|| variable_declarator_name(self.nodes, node_id)),
            AstKind::Class(class) => class_name(self.nodes, node_id, class),
            AstKind::NewExpression(expression) => variable_declarator_name(self.nodes, node_id)
                .or_else(|| expression_text(&expression.callee)),
            _ => variable_declarator_name(self.nodes, node_id),
        }
    }

    fn declaration_symbol_for_node_name(&self, node_id: NodeId, name: &str) -> Option<OxcSymbolId> {
        self.symbols_by_declaration
            .get(&(node_id, name.to_string()))
            .copied()
            .or_else(|| {
                self.symbols_by_declaration
                    .get(&(self.nodes.parent_id(node_id), name.to_string()))
                    .copied()
            })
    }

    fn lexical_parent_key(&self, node_id: NodeId) -> Option<String> {
        self.nodes
            .ancestors_enumerated(node_id)
            .find_map(|(_, node)| match node.kind() {
                AstKind::Function(function) => Some(parent_key(
                    self.file,
                    function.span,
                    "function",
                    function
                        .id
                        .as_ref()
                        .map(|identifier| identifier.name.to_string()),
                )),
                AstKind::ArrowFunctionExpression(function) => Some(parent_key(
                    self.file,
                    function.span,
                    "arrow",
                    variable_declarator_name(self.nodes, node_id),
                )),
                AstKind::MethodDefinition(method) => Some(parent_key(
                    self.file,
                    method.span,
                    method_kind_label(method.kind),
                    method_name(method),
                )),
                AstKind::Class(class) => Some(parent_key(
                    self.file,
                    class.span,
                    "class",
                    class_name(self.nodes, node_id, class),
                )),
                _ => None,
            })
            .or_else(|| {
                Some(format!(
                    "ts_object_parent:{}:module",
                    self.file.relative_path
                ))
            })
    }

    fn allocation_stable_key(
        &self,
        span: oxc_span::Span,
        kind: TsObjectAllocationKind,
        lexical_parent_key: Option<&str>,
        display_name: Option<&str>,
    ) -> String {
        stable_object_key(
            "ts_object_allocation",
            &[
                ("file", self.file.relative_path.clone()),
                ("kind", kind.as_str().to_string()),
                ("start", span.start.to_string()),
                ("end", span.end.to_string()),
                (
                    "parent",
                    lexical_parent_key.unwrap_or("<module>").to_string(),
                ),
                ("display", display_name.unwrap_or("<anonymous>").to_string()),
            ],
        )
    }

    fn operation_stable_key(
        &self,
        prefix: &str,
        span: oxc_span::Span,
        base: &str,
        label: &str,
    ) -> String {
        stable_object_key(
            prefix,
            &[
                ("file", self.file.relative_path.clone()),
                ("start", span.start.to_string()),
                ("end", span.end.to_string()),
                ("base", base.to_string()),
                ("label", label.to_string()),
            ],
        )
    }
}

#[derive(Debug, Clone, Default)]
struct ObjectModelInventoryIndex {
    unique_function_key_by_name: BTreeMap<String, Option<String>>,
    function_key_by_span: BTreeMap<(u32, u32), String>,
    callsite_key_by_span: BTreeMap<(u32, u32), String>,
}

impl ObjectModelInventoryIndex {
    fn from_output(output: &TsInventoryOutput) -> Self {
        let mut index = Self::default();
        for function in &output.functions {
            index.function_key_by_span.insert(
                (function.span.start_byte, function.span.end_byte),
                crate::ts::resolve_frontend_stable_key(function.stable_key).to_string(),
            );
            if let Some(name) = &function.display_name {
                insert_unique(
                    &mut index.unique_function_key_by_name,
                    name.clone(),
                    crate::ts::resolve_frontend_stable_key(function.stable_key).to_string(),
                );
            }
        }
        for callsite in &output.callsites {
            index.callsite_key_by_span.insert(
                (callsite.span.start_byte, callsite.span.end_byte),
                crate::ts::resolve_frontend_stable_key(callsite.stable_key).to_string(),
            );
        }
        index
    }

    fn unique_function_key(&self, name: &str) -> Option<&String> {
        self.unique_function_key_by_name
            .get(name)
            .and_then(Option::as_ref)
    }
}

fn insert_unique<K: Ord, V: PartialEq>(map: &mut BTreeMap<K, Option<V>>, key: K, value: V) {
    match map.get_mut(&key) {
        Some(existing) if existing.as_ref() == Some(&value) => {}
        Some(existing) => {
            *existing = None;
        }
        None => {
            map.insert(key, Some(value));
        }
    }
}

/// Whether `function` is a function expression this model allocates an object
/// for. Declarations and methods get theirs elsewhere, and a function without a
/// body (an overload signature, an ambient `declare function` or an abstract
/// member) is a type, not a value: no call reaches it.
fn allocates_function_object(
    nodes: &AstNodes<'_>,
    node_id: NodeId,
    function: &Function<'_>,
) -> bool {
    function.body.is_some()
        && !matches!(function.r#type, FunctionType::FunctionDeclaration)
        && !matches!(nodes.parent_kind(node_id), AstKind::MethodDefinition(_))
}

fn allocation_kind_for_node(
    nodes: &AstNodes<'_>,
    node_id: NodeId,
    kind: AstKind<'_>,
) -> Option<TsObjectAllocationKind> {
    match kind {
        AstKind::ObjectExpression(_) => Some(TsObjectAllocationKind::ObjectLiteral),
        AstKind::ArrayExpression(_) => Some(TsObjectAllocationKind::ArrayLiteral),
        AstKind::Function(function) if allocates_function_object(nodes, node_id, function) => {
            Some(TsObjectAllocationKind::FunctionObject)
        }
        AstKind::ArrowFunctionExpression(_) => Some(TsObjectAllocationKind::FunctionObject),
        AstKind::Class(_) => Some(TsObjectAllocationKind::ClassObject),
        AstKind::NewExpression(_) => Some(TsObjectAllocationKind::ClassInstance),
        _ => None,
    }
}

fn binding_identifier_name(declarator: &VariableDeclarator<'_>) -> Option<String> {
    match &declarator.id {
        BindingPattern::BindingIdentifier(identifier) => Some(identifier.name.to_string()),
        _ => None,
    }
}

fn variable_declarator_name(nodes: &AstNodes<'_>, node_id: NodeId) -> Option<String> {
    match nodes.parent_kind(node_id) {
        AstKind::VariableDeclarator(declarator) => binding_identifier_name(declarator),
        _ => None,
    }
}

fn allocation_binding_name(
    nodes: &AstNodes<'_>,
    node_id: NodeId,
    kind: AstKind<'_>,
) -> Option<String> {
    match kind {
        AstKind::Class(class) => class_name(nodes, node_id, class),
        _ => variable_declarator_name(nodes, node_id),
    }
}

fn class_name(nodes: &AstNodes<'_>, node_id: NodeId, class: &Class<'_>) -> Option<String> {
    class
        .id
        .as_ref()
        .map(|identifier| identifier.name.to_string())
        .or_else(|| variable_declarator_name(nodes, node_id))
}

fn symbols_by_declaration(scoping: &Scoping) -> BTreeMap<(NodeId, String), OxcSymbolId> {
    scoping
        .symbol_ids()
        .map(|symbol| {
            (
                (
                    scoping.symbol_declaration(symbol),
                    scoping.symbol_name(symbol).to_string(),
                ),
                symbol,
            )
        })
        .collect()
}

fn reference_symbols_by_span(
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
) -> BTreeMap<(u32, u32), OxcSymbolId> {
    let mut symbols_by_span = BTreeMap::new();
    for symbol in scoping.symbol_ids() {
        for reference in scoping.get_resolved_references(symbol) {
            let span = nodes.kind(reference.node_id()).span();
            symbols_by_span.insert((span.start, span.end), symbol);
        }
    }
    symbols_by_span
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

fn method_kind_label(kind: MethodDefinitionKind) -> &'static str {
    match kind {
        MethodDefinitionKind::Constructor => "constructor",
        MethodDefinitionKind::Method => "method",
        MethodDefinitionKind::Get => "get",
        MethodDefinitionKind::Set => "set",
    }
}

fn property_key(key: &PropertyKey<'_>, computed: bool) -> TsPropertyKey {
    if computed {
        return property_key_from_property_expression(key);
    }
    match key {
        PropertyKey::StaticIdentifier(identifier) => TsPropertyKey {
            kind: TsPropertyKeyKind::Static,
            value: Some(identifier.name.to_string()),
        },
        PropertyKey::PrivateIdentifier(identifier) => TsPropertyKey {
            kind: TsPropertyKeyKind::Private,
            value: Some(identifier.name.to_string()),
        },
        PropertyKey::StringLiteral(literal) => TsPropertyKey {
            kind: TsPropertyKeyKind::StringLiteral,
            value: Some(literal.value.to_string()),
        },
        PropertyKey::NumericLiteral(literal) => TsPropertyKey {
            kind: TsPropertyKeyKind::NumericLiteral,
            value: Some(literal.value.to_string()),
        },
        _ => TsPropertyKey {
            kind: TsPropertyKeyKind::UnknownBucket,
            value: None,
        },
    }
}

fn property_key_from_property_expression(key: &PropertyKey<'_>) -> TsPropertyKey {
    match key {
        PropertyKey::StringLiteral(literal) => TsPropertyKey {
            kind: TsPropertyKeyKind::StringLiteral,
            value: Some(literal.value.to_string()),
        },
        PropertyKey::NumericLiteral(literal) => TsPropertyKey {
            kind: TsPropertyKeyKind::NumericLiteral,
            value: Some(literal.value.to_string()),
        },
        PropertyKey::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
            if let (Some(left), Some(right)) = (
                constant_property_key_expression(&binary.left),
                constant_property_key_expression(&binary.right),
            ) {
                TsPropertyKey {
                    kind: TsPropertyKeyKind::StringLiteral,
                    value: Some(format!("{left}{right}")),
                }
            } else {
                TsPropertyKey {
                    kind: TsPropertyKeyKind::ComputedBucket,
                    value: None,
                }
            }
        }
        _ => TsPropertyKey {
            kind: TsPropertyKeyKind::ComputedBucket,
            value: None,
        },
    }
}

fn property_key_from_expression(expression: &Expression<'_>) -> TsPropertyKey {
    if let Some(value) = constant_property_key_expression(expression) {
        return TsPropertyKey {
            kind: TsPropertyKeyKind::StringLiteral,
            value: Some(value),
        };
    }

    match expression {
        Expression::NumericLiteral(literal) => TsPropertyKey {
            kind: TsPropertyKeyKind::NumericLiteral,
            value: Some(literal.value.to_string()),
        },
        _ => TsPropertyKey {
            kind: TsPropertyKeyKind::ComputedBucket,
            value: None,
        },
    }
}

fn argument_expression<'a>(argument: &'a Argument<'a>) -> Option<&'a Expression<'a>> {
    match argument {
        Argument::SpreadElement(_) => None,
        _ => argument.as_expression(),
    }
}

fn expression_text(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier.name.to_string()),
        Expression::ThisExpression(_) => Some("this".to_string()),
        Expression::StaticMemberExpression(member) => expression_text(&member.object)
            .map(|object| format!("{object}.{}", member.property.name)),
        Expression::ComputedMemberExpression(member) => {
            let object = expression_text(&member.object)?;
            let property =
                expression_text(&member.expression).unwrap_or_else(|| "<computed>".to_string());
            Some(format!("{object}[{property}]"))
        }
        Expression::PrivateFieldExpression(member) => {
            expression_text(&member.object).map(|object| format!("{object}.#{}", member.field.name))
        }
        Expression::StringLiteral(literal) => Some(literal.value.to_string()),
        Expression::NumericLiteral(literal) => Some(literal.value.to_string()),
        Expression::ParenthesizedExpression(expression) => expression_text(&expression.expression),
        Expression::TSAsExpression(expression) => expression_text(&expression.expression),
        Expression::TSSatisfiesExpression(expression) => expression_text(&expression.expression),
        Expression::TSNonNullExpression(expression) => expression_text(&expression.expression),
        Expression::TSTypeAssertion(expression) => expression_text(&expression.expression),
        _ => None,
    }
}

fn expression_object_key(expression: &Expression<'_>) -> String {
    stable_object_key(
        "ts_object_expression",
        &[(
            "display",
            expression_text(expression).unwrap_or_else(|| "<unknown>".to_string()),
        )],
    )
}

fn parent_key(
    file: &SourceFile,
    span: oxc_span::Span,
    kind: &str,
    display_name: Option<String>,
) -> String {
    stable_object_key(
        "ts_object_parent",
        &[
            ("file", file.relative_path.clone()),
            ("kind", kind.to_string()),
            ("start", span.start.to_string()),
            ("end", span.end.to_string()),
            (
                "display",
                display_name.as_deref().unwrap_or("<anonymous>").to_string(),
            ),
        ],
    )
}

fn stable_object_key(prefix: &str, parts: &[(&str, String)]) -> String {
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
    use std::path::PathBuf;

    use crate::ts::local_db::LocalFactDb;
    use crate::ts::object_model::facts::{
        TsObjectAllocationKind, TsPropertyKeyKind, TsReceiverBindingKind,
    };

    use super::*;

    #[test]
    fn functions_without_a_body_allocate_no_function_object() {
        let output = extract(
            r#"
function pick(x: string): string;
function pick(x: any) { return x; }
declare function ambient(x: string): void;
const run = function () {};
"#,
        );

        let function_objects = output
            .allocations
            .iter()
            .filter(|allocation| allocation.kind == TsObjectAllocationKind::FunctionObject)
            .count();
        assert_eq!(function_objects, 1, "{:#?}", output.allocations);
    }

    #[test]
    fn object_literal_shorthand_yields_allocation_and_static_property_write() {
        let output = extract(
            r#"
function target() {}
const holder = { target };
"#,
        );

        assert!(
            output
                .allocations
                .iter()
                .any(|allocation| allocation.kind == TsObjectAllocationKind::ObjectLiteral)
        );
        assert!(output.property_writes.iter().any(|write| {
            write.property_key.kind == TsPropertyKeyKind::Static
                && write.property_key.value.as_deref() == Some("target")
        }));
    }

    #[test]
    fn exact_string_element_access_yields_string_literal_property_read() {
        let output = extract(
            r#"
function target() {}
const holder = { target };
holder["target"]();
"#,
        );

        assert!(output.property_reads.iter().any(|read| {
            read.property_key.kind == TsPropertyKeyKind::StringLiteral
                && read.property_key.value.as_deref() == Some("target")
        }));
    }

    #[test]
    fn dynamic_element_access_yields_computed_bucket_property_read() {
        let output = extract(
            r#"
const key = "target";
const holder = {};
holder[key]();
"#,
        );

        assert!(
            output
                .property_reads
                .iter()
                .any(|read| read.property_key.kind == TsPropertyKeyKind::ComputedBucket)
        );
    }

    #[test]
    fn class_method_yields_class_prototype_and_method_property_facts() {
        let output = extract(
            r#"
class C {
  m() {}
}
"#,
        );

        assert!(
            output
                .allocations
                .iter()
                .any(|allocation| allocation.kind == TsObjectAllocationKind::ClassObject)
        );
        assert!(
            output
                .allocations
                .iter()
                .any(|allocation| allocation.kind == TsObjectAllocationKind::PrototypeObject)
        );
        assert!(output.property_writes.iter().any(|write| {
            write.property_key.kind == TsPropertyKeyKind::Static
                && write.property_key.value.as_deref() == Some("m")
                && write.value_function_stable_key.is_some()
        }));
        assert!(!output.prototype_links.is_empty());
    }

    #[test]
    fn computed_string_concatenation_class_methods_yield_named_writes() {
        let output = extract(
            r#"
class C {
  ["na" + "me"]() {}
  ["a" + "b" + "c"]() {}
  static [("static") + "G"]() {}
}
"#,
        );

        for expected in ["name", "abc", "staticG"] {
            assert!(
                output.property_writes.iter().any(|write| {
                    write.property_key.kind == TsPropertyKeyKind::StringLiteral
                        && write.property_key.value.as_deref() == Some(expected)
                        && write.value_function_stable_key.is_some()
                }),
                "missing computed method property write for {expected}: {:?}",
                output.property_writes
            );
        }
    }

    #[test]
    fn inline_object_member_read_uses_recorded_allocation_key() {
        let output = extract(
            r#"
function target() {}
export function run() {
  return ({ target }).target();
}
"#,
        );
        let read = output
            .property_reads
            .iter()
            .find(|read| read.property_key.value.as_deref() == Some("target"))
            .expect("inline object property read should be extracted");

        assert!(
            output
                .allocations
                .iter()
                .any(|allocation| allocation.stable_key == read.base_object_stable_key),
            "inline object read base should match a recorded allocation"
        );
    }

    #[test]
    fn inline_new_member_read_uses_recorded_allocation_key() {
        let output = extract(
            r#"
class C {
  m() {}
}
export function run() {
  return new C().m();
}
"#,
        );
        let read = output
            .property_reads
            .iter()
            .find(|read| read.property_key.value.as_deref() == Some("m"))
            .expect("inline new property read should be extracted");

        assert!(
            output
                .allocations
                .iter()
                .any(|allocation| allocation.stable_key == read.base_object_stable_key),
            "inline new read base should match a recorded allocation"
        );
    }

    #[test]
    fn same_name_object_bindings_resolve_to_their_own_scope() {
        let output = extract(
            r#"
function firstTarget() {}
function secondTarget() {}

export function first() {
  const holder = { firstTarget };
  return holder.firstTarget();
}

export function second() {
  const holder = { secondTarget };
  return holder.secondTarget();
}
"#,
        );
        let first_write = output
            .property_writes
            .iter()
            .find(|write| write.property_key.value.as_deref() == Some("firstTarget"))
            .expect("first holder write should be extracted");
        let first_read = output
            .property_reads
            .iter()
            .find(|read| read.property_key.value.as_deref() == Some("firstTarget"))
            .expect("first holder read should be extracted");
        let second_write = output
            .property_writes
            .iter()
            .find(|write| write.property_key.value.as_deref() == Some("secondTarget"))
            .expect("second holder write should be extracted");
        let second_read = output
            .property_reads
            .iter()
            .find(|read| read.property_key.value.as_deref() == Some("secondTarget"))
            .expect("second holder read should be extracted");

        assert_eq!(
            first_read.base_object_stable_key, first_write.base_object_stable_key,
            "first scoped holder read must use the first scoped allocation"
        );
        assert_eq!(
            second_read.base_object_stable_key, second_write.base_object_stable_key,
            "second scoped holder read must use the second scoped allocation"
        );
        assert_ne!(
            first_read.base_object_stable_key, second_read.base_object_stable_key,
            "same-name scoped holders must not collapse to one allocation"
        );
    }

    #[test]
    fn inline_new_before_class_declaration_links_to_later_prototype() {
        let output = extract(
            r#"
export function run() {
  return new C().m();
}

class C {
  m() {}
}
"#,
        );
        let read = output
            .property_reads
            .iter()
            .find(|read| read.property_key.value.as_deref() == Some("m"))
            .expect("inline new property read should be extracted");

        assert!(
            output.prototype_links.iter().any(|link| {
                link.object_stable_key == read.base_object_stable_key
                    && output.allocations.iter().any(|allocation| {
                        allocation.stable_key == link.prototype_stable_key
                            && allocation.kind == TsObjectAllocationKind::PrototypeObject
                    })
            }),
            "inline new before class declaration should link its instance to the class prototype"
        );
    }

    #[test]
    fn object_literal_method_yields_callable_property_write() {
        let output = extract(
            r#"
const holder = {
  methodTarget() {
    return "method";
  },
};
holder.methodTarget();
"#,
        );

        assert!(output.property_writes.iter().any(|write| {
            write.property_key.kind == TsPropertyKeyKind::Static
                && write.property_key.value.as_deref() == Some("methodTarget")
                && write.value_function_stable_key.is_some()
        }));
    }

    #[test]
    fn arrow_function_yields_lexical_receiver_marker() {
        let (output, interner) = extract_with_interner("const f = () => this.target;\n");

        assert!(output.receiver_bindings.iter().any(|binding| {
            binding.kind == TsReceiverBindingKind::LexicalThis
                && binding
                    .lexical_parent_key
                    .is_some_and(|key| interner.resolve(key).as_ref() == "lexical-this")
        }));
    }

    fn extract(source: &str) -> TsObjectModelOutput {
        extract_with_interner(source).0
    }

    fn extract_with_interner(source: &str) -> (TsObjectModelOutput, StableKeyInterner) {
        let mut db = LocalFactDb::new();
        let file_id = db.add_file(
            PathBuf::from("fixture.ts"),
            "fixture.ts".to_string(),
            source.to_string(),
        );
        let file = db.file(file_id).expect("source file exists");
        let interner = db.stable_key_interner();
        (extract_ts_object_model(&interner, file), interner)
    }
}
