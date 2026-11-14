//! TypeScript token-source flow extraction used by semantic-graph projection.

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

use crate::analysis_api::{FactFamily, SourceFile};
use crate::analysis_neutral::stable_key::semantic_stable_key;
use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast::ast::{Argument, BindingPattern, Expression, Function, Statement, VariableDeclarator};
use oxc_semantic::{AstNodes, NodeId, SemanticBuilder};
use oxc_span::{GetSpan, Span};

use crate::ts::callable_flow::cannot_be_callable;
use crate::ts::inventory::extract::extract_ts_inventory;
use crate::ts::inventory::store::TsInventoryOutput;
use crate::ts::parse::parse_ts_file;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TsTokenSourceFlow {
    pub source_function_stable_key_text: String,
    pub callsite_stable_key_text: String,
    pub stable_key_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TsTokenSourceFlowKind {
    ParameterArgument,
    ReturnValue,
}

impl TsTokenSourceFlowKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ParameterArgument => "parameter_argument",
            Self::ReturnValue => "return_value",
        }
    }
}

#[derive(Debug, Clone)]
struct TsFunctionInventoryRef {
    stable_key_text: String,
    display_name: Option<String>,
}

#[derive(Debug, Default)]
struct TsTokenSourceFlowIndex {
    function_by_span: BTreeMap<(u32, u32), TsFunctionInventoryRef>,
    callsite_key_by_span: BTreeMap<(u32, u32), String>,
    unique_function_key_by_name: BTreeMap<String, Option<String>>,
    unique_function_node_by_name: BTreeMap<String, Option<NodeId>>,
    parameter_name_by_function: BTreeMap<NodeId, Vec<String>>,
    parameter_calls_by_function: BTreeMap<NodeId, BTreeMap<usize, Vec<String>>>,
    local_alias_target_by_function_var: BTreeMap<(NodeId, String), Option<String>>,
    returned_function_key_by_function: BTreeMap<NodeId, Option<String>>,
    local_return_token_by_function_var: BTreeMap<(NodeId, String), Option<String>>,
    /// Names written somewhere inside a function body, nested closures included.
    /// A written name may hold any value, so no flow is keyed on it there.
    written_names_by_function: BTreeSet<(NodeId, String)>,
}

pub fn collect_ts_token_source_flows(
    interner: &crate::internal_core::StableKeyInterner,
    file: &SourceFile,
    inventory: Option<&TsInventoryOutput>,
) -> Vec<TsTokenSourceFlow> {
    match inventory {
        Some(inventory) => collect_ts_token_source_flows_with_inventory(interner, file, inventory),
        None => {
            let inventory = extract_ts_inventory(interner, file);
            collect_ts_token_source_flows_with_inventory(interner, file, &inventory)
        }
    }
}

fn collect_ts_token_source_flows_with_inventory(
    interner: &crate::internal_core::StableKeyInterner,
    file: &SourceFile,
    inventory: &TsInventoryOutput,
) -> Vec<TsTokenSourceFlow> {
    let allocator = Allocator::default();
    let parsed = parse_ts_file(&allocator, file);

    if parsed.is_catastrophic() {
        return Vec::new();
    }

    let semantic = SemanticBuilder::new().build(parsed.program()).semantic;
    collect_ts_token_source_flows_from_nodes(interner, file, inventory, semantic.nodes())
}

pub fn collect_ts_token_source_flows_from_nodes(
    interner: &crate::internal_core::StableKeyInterner,
    file: &SourceFile,
    inventory: &TsInventoryOutput,
    nodes: &AstNodes<'_>,
) -> Vec<TsTokenSourceFlow> {
    let source = file.source.as_ref();
    let mut index = TsTokenSourceFlowIndex::from_inventory(interner, inventory, nodes);
    index.collect_written_names(nodes);
    index.collect_parameter_calls(source, nodes);
    index.collect_local_alias_assignments(nodes);
    index.collect_returned_functions(nodes);
    index.collect_local_return_assignments(nodes);
    index.collect_flows(source, nodes)
}

impl TsTokenSourceFlowIndex {
    fn from_inventory(
        interner: &crate::internal_core::StableKeyInterner,
        inventory: &TsInventoryOutput,
        nodes: &AstNodes<'_>,
    ) -> Self {
        let mut index = Self::default();
        for function in &inventory.functions {
            let key = (function.span.start_byte, function.span.end_byte);
            let reference = TsFunctionInventoryRef {
                stable_key_text: interner.resolve(function.stable_key).to_string(),
                display_name: function.display_name.clone(),
            };
            if let Some(display_name) = &reference.display_name {
                insert_unique(
                    &mut index.unique_function_key_by_name,
                    display_name.clone(),
                    reference.stable_key_text.clone(),
                );
            }
            index.function_by_span.insert(key, reference);
        }
        for callsite in &inventory.callsites {
            index.callsite_key_by_span.insert(
                (callsite.span.start_byte, callsite.span.end_byte),
                interner.resolve(callsite.stable_key).to_string(),
            );
        }

        for (node_id, node) in nodes.iter_enumerated() {
            let Some(reference) = index.function_reference_for_kind(node.kind()).cloned() else {
                continue;
            };
            if let Some(display_name) = &reference.display_name {
                insert_unique(
                    &mut index.unique_function_node_by_name,
                    display_name.clone(),
                    node_id,
                );
            }
            if let Some(params) = function_parameter_names(node.kind()) {
                index.parameter_name_by_function.insert(node_id, params);
            }
        }

        index
    }

    fn collect_written_names(&mut self, nodes: &AstNodes<'_>) {
        for (node_id, node) in nodes.iter_enumerated() {
            let AstKind::IdentifierReference(identifier) = node.kind() else {
                continue;
            };
            if !is_written_reference(nodes, node_id, identifier.span) {
                continue;
            }
            for (ancestor_id, ancestor) in nodes.ancestors_enumerated(node_id) {
                if matches!(
                    ancestor.kind(),
                    AstKind::Function(_)
                        | AstKind::ArrowFunctionExpression(_)
                        | AstKind::MethodDefinition(_)
                ) {
                    self.written_names_by_function
                        .insert((ancestor_id, identifier.name.to_string()));
                }
            }
        }
    }

    fn is_written_in(&self, function: NodeId, name: &str) -> bool {
        self.written_names_by_function
            .contains(&(function, name.to_string()))
    }

    fn collect_parameter_calls(&mut self, source: &str, nodes: &AstNodes<'_>) {
        for (node_id, node) in nodes.iter_enumerated() {
            let AstKind::CallExpression(call) = node.kind() else {
                continue;
            };
            let Some(callee_name) = expression_identifier_name(&call.callee) else {
                continue;
            };
            let Some(enclosing_function) = enclosing_function_node(nodes, node_id) else {
                continue;
            };
            let Some(parameter_names) = self.parameter_name_by_function.get(&enclosing_function)
            else {
                continue;
            };
            let Some(position) = parameter_names.iter().position(|name| name == &callee_name)
            else {
                continue;
            };
            if self.is_written_in(enclosing_function, &callee_name) {
                continue;
            }
            let Some(callsite_key) = self.callsite_key_for_kind(source, node.kind()) else {
                continue;
            };

            self.parameter_calls_by_function
                .entry(enclosing_function)
                .or_default()
                .entry(position)
                .or_default()
                .push(callsite_key);
        }
    }

    fn collect_local_alias_assignments(&mut self, nodes: &AstNodes<'_>) {
        for (node_id, node) in nodes.iter_enumerated() {
            let AstKind::VariableDeclarator(declarator) = node.kind() else {
                continue;
            };
            let Some(variable_name) = variable_declarator_name(declarator) else {
                continue;
            };
            let Some(init) = declarator.init.as_ref() else {
                continue;
            };
            let Some(target_name) = expression_identifier_name(init) else {
                continue;
            };
            let Some(enclosing_function) = enclosing_function_node(nodes, node_id) else {
                continue;
            };
            let Some(target_function_key) =
                unique_lookup(&self.unique_function_key_by_name, &target_name).cloned()
            else {
                continue;
            };
            insert_unique(
                &mut self.local_alias_target_by_function_var,
                (enclosing_function, variable_name),
                target_function_key,
            );
        }
    }

    fn collect_returned_functions(&mut self, nodes: &AstNodes<'_>) {
        for (node_id, node) in nodes.iter_enumerated() {
            let AstKind::ReturnStatement(statement) = node.kind() else {
                continue;
            };
            let Some(argument) = statement.argument.as_ref() else {
                continue;
            };
            let Some(enclosing_function) = enclosing_function_node(nodes, node_id) else {
                continue;
            };
            let Some(returned_function_key) =
                self.function_key_from_return_argument(enclosing_function, argument)
            else {
                // A return this index cannot name may still yield a function, so
                // the function no longer has a single known returned value.
                if !cannot_be_callable(argument) {
                    self.returned_function_key_by_function
                        .insert(enclosing_function, None);
                }
                continue;
            };

            insert_unique(
                &mut self.returned_function_key_by_function,
                enclosing_function,
                returned_function_key,
            );
        }
    }

    fn collect_local_return_assignments(&mut self, nodes: &AstNodes<'_>) {
        for (node_id, node) in nodes.iter_enumerated() {
            let AstKind::CallExpression(call) = node.kind() else {
                continue;
            };
            let Some(variable_name) = variable_declarator_name_for_call(nodes, node_id) else {
                continue;
            };
            let Some(callee_name) = expression_identifier_name(&call.callee) else {
                continue;
            };
            let Some(callee_function) =
                unique_lookup(&self.unique_function_node_by_name, &callee_name).copied()
            else {
                continue;
            };
            let Some(enclosing_function) = enclosing_function_node(nodes, node_id) else {
                continue;
            };
            let Some(returned_function_key) =
                unique_lookup(&self.returned_function_key_by_function, &callee_function).cloned()
            else {
                continue;
            };
            insert_unique(
                &mut self.local_return_token_by_function_var,
                (enclosing_function, variable_name),
                returned_function_key,
            );
        }
    }

    fn collect_flows(&self, source: &str, nodes: &AstNodes<'_>) -> Vec<TsTokenSourceFlow> {
        let mut flows = Vec::new();
        for (node_id, node) in nodes.iter_enumerated() {
            let AstKind::CallExpression(call) = node.kind() else {
                continue;
            };
            let Some(callsite_key) = self.callsite_key_for_kind(source, node.kind()) else {
                continue;
            };
            let Some(callee_name) = expression_identifier_name(&call.callee) else {
                continue;
            };

            self.collect_parameter_argument_flows(&mut flows, &callee_name, call, &callsite_key);
            self.collect_return_value_flow(&mut flows, nodes, node_id, &callee_name, &callsite_key);
        }
        flows.sort();
        flows.dedup();
        flows
    }

    fn collect_parameter_argument_flows(
        &self,
        flows: &mut Vec<TsTokenSourceFlow>,
        callee_name: &str,
        call: &oxc_ast::ast::CallExpression<'_>,
        source_callsite_key: &str,
    ) {
        let Some(callee_function) =
            unique_lookup(&self.unique_function_node_by_name, &callee_name.to_string()).copied()
        else {
            return;
        };
        let Some(parameter_calls) = self.parameter_calls_by_function.get(&callee_function) else {
            return;
        };
        for (position, argument) in call.arguments.iter().enumerate() {
            let Some(argument_name) = argument_identifier_name(argument) else {
                continue;
            };
            let Some(argument_function_key) =
                unique_lookup(&self.unique_function_key_by_name, &argument_name)
            else {
                continue;
            };
            let Some(destination_callsites) = parameter_calls.get(&position) else {
                continue;
            };
            for destination_callsite_key in destination_callsites {
                flows.push(ts_token_source_flow(
                    TsTokenSourceFlowKind::ParameterArgument,
                    argument_function_key,
                    destination_callsite_key,
                    source_callsite_key,
                ));
            }
        }
    }

    fn collect_return_value_flow(
        &self,
        flows: &mut Vec<TsTokenSourceFlow>,
        nodes: &AstNodes<'_>,
        node_id: NodeId,
        callee_name: &str,
        callsite_key: &str,
    ) {
        let Some(enclosing_function) = enclosing_function_node(nodes, node_id) else {
            return;
        };
        if self.is_written_in(enclosing_function, callee_name) {
            return;
        }
        let Some(returned_function_key) = unique_lookup(
            &self.local_return_token_by_function_var,
            &(enclosing_function, callee_name.to_string()),
        ) else {
            return;
        };
        flows.push(ts_token_source_flow(
            TsTokenSourceFlowKind::ReturnValue,
            returned_function_key,
            callsite_key,
            callee_name,
        ));
    }

    fn function_reference_for_kind(&self, kind: AstKind<'_>) -> Option<&TsFunctionInventoryRef> {
        if !matches!(
            kind,
            AstKind::Function(_)
                | AstKind::ArrowFunctionExpression(_)
                | AstKind::MethodDefinition(_)
        ) {
            return None;
        }
        let span = kind.span();
        self.function_by_span.get(&(span.start, span.end))
    }

    fn callsite_key_for_kind(&self, source: &str, kind: AstKind<'_>) -> Option<String> {
        let span = crate::ts::spans::normalized_callsite_span(source, kind)?;
        self.callsite_key_by_span
            .get(&(span.start, span.end))
            .cloned()
    }

    fn function_key_from_return_argument(
        &self,
        enclosing_function: NodeId,
        argument: &Expression<'_>,
    ) -> Option<String> {
        match argument {
            Expression::Identifier(identifier) => {
                let name = identifier.name.to_string();
                self.lookup_function_or_local_alias(enclosing_function, &name)
            }
            Expression::FunctionExpression(function) => self
                .returned_function_expression_target(enclosing_function, function)
                .or_else(|| {
                    self.function_by_span
                        .get(&(function.span.start, function.span.end))
                        .map(|reference| reference.stable_key_text.clone())
                }),
            Expression::ParenthesizedExpression(expression) => {
                self.function_key_from_return_argument(enclosing_function, &expression.expression)
            }
            Expression::TSAsExpression(expression) => {
                self.function_key_from_return_argument(enclosing_function, &expression.expression)
            }
            Expression::TSSatisfiesExpression(expression) => {
                self.function_key_from_return_argument(enclosing_function, &expression.expression)
            }
            Expression::TSNonNullExpression(expression) => {
                self.function_key_from_return_argument(enclosing_function, &expression.expression)
            }
            Expression::TSTypeAssertion(expression) => {
                self.function_key_from_return_argument(enclosing_function, &expression.expression)
            }
            Expression::TSInstantiationExpression(expression) => {
                self.function_key_from_return_argument(enclosing_function, &expression.expression)
            }
            _ => None,
        }
    }

    fn returned_function_expression_target(
        &self,
        enclosing_function: NodeId,
        function: &Function<'_>,
    ) -> Option<String> {
        let body = function.body.as_ref()?;
        for statement in &body.statements {
            let Statement::ReturnStatement(statement) = statement else {
                continue;
            };
            let Some(Expression::CallExpression(call)) = statement.argument.as_ref() else {
                continue;
            };
            let Some(callee_name) = expression_identifier_name(&call.callee) else {
                continue;
            };
            if let Some(target) =
                self.lookup_function_or_local_alias(enclosing_function, &callee_name)
            {
                return Some(target);
            }
        }
        None
    }

    fn lookup_function_or_local_alias(
        &self,
        enclosing_function: NodeId,
        name: &str,
    ) -> Option<String> {
        if self.is_written_in(enclosing_function, name) {
            return None;
        }
        unique_lookup(
            &self.local_alias_target_by_function_var,
            &(enclosing_function, name.to_string()),
        )
        .cloned()
        .or_else(|| unique_lookup(&self.unique_function_key_by_name, &name.to_string()).cloned())
    }
}

fn ts_token_source_flow(
    kind: TsTokenSourceFlowKind,
    source_function_stable_key: &str,
    callsite_stable_key: &str,
    evidence: &str,
) -> TsTokenSourceFlow {
    TsTokenSourceFlow {
        source_function_stable_key_text: source_function_stable_key.to_string(),
        callsite_stable_key_text: callsite_stable_key.to_string(),
        stable_key_text: semantic_stable_key(
            FactFamily::PointsToConstraint,
            &[
                ("source", source_function_stable_key.to_string()),
                ("callsite", callsite_stable_key.to_string()),
                ("kind", kind.as_str().to_string()),
                ("evidence", evidence.to_string()),
            ],
        )
        .into_string(),
    }
}

fn insert_unique<K: Ord, V: PartialEq>(map: &mut BTreeMap<K, Option<V>>, key: K, value: V) {
    match map.entry(key) {
        Entry::Vacant(entry) => {
            entry.insert(Some(value));
        }
        Entry::Occupied(mut entry) => {
            if entry
                .get()
                .as_ref()
                .is_some_and(|existing| existing != &value)
            {
                entry.insert(None);
            }
        }
    }
}

fn unique_lookup<'a, K: Ord, V>(map: &'a BTreeMap<K, Option<V>>, key: &K) -> Option<&'a V> {
    map.get(key).and_then(Option::as_ref)
}

fn function_parameter_names(kind: AstKind<'_>) -> Option<Vec<String>> {
    let params = match kind {
        AstKind::Function(function) => function.params.as_ref(),
        AstKind::ArrowFunctionExpression(function) => &function.params,
        _ => return None,
    };
    Some(
        params
            .items
            .iter()
            .filter_map(|parameter| binding_pattern_identifier_name(&parameter.pattern))
            .collect(),
    )
}

fn enclosing_function_node(nodes: &AstNodes<'_>, node_id: NodeId) -> Option<NodeId> {
    nodes
        .ancestors_enumerated(node_id)
        .find_map(|(ancestor_id, node)| {
            if matches!(
                node.kind(),
                AstKind::Function(_)
                    | AstKind::ArrowFunctionExpression(_)
                    | AstKind::MethodDefinition(_)
            ) {
                Some(ancestor_id)
            } else {
                None
            }
        })
}

/// Whether the identifier reference at `node_id` is written: the target of an
/// assignment, an element of a destructuring assignment pattern, or the head of
/// a `for…in` / `for…of` loop. Member targets such as `name.field = value` do not
/// write `name`.
fn is_written_reference(nodes: &AstNodes<'_>, node_id: NodeId, span: Span) -> bool {
    let contains = |outer: Span| outer.start <= span.start && span.end <= outer.end;
    for (_, ancestor) in nodes.ancestors_enumerated(node_id) {
        match ancestor.kind() {
            AstKind::ArrayAssignmentTarget(_)
            | AstKind::ObjectAssignmentTarget(_)
            | AstKind::AssignmentTargetRest(_)
            | AstKind::ParenthesizedExpression(_)
            | AstKind::TSAsExpression(_)
            | AstKind::TSSatisfiesExpression(_)
            | AstKind::TSTypeAssertion(_)
            | AstKind::TSNonNullExpression(_) => {}
            AstKind::AssignmentTargetWithDefault(target) => {
                if !contains(target.binding.span()) {
                    return false;
                }
            }
            AstKind::AssignmentTargetPropertyIdentifier(property) => {
                if property.binding.span != span {
                    return false;
                }
            }
            AstKind::AssignmentTargetPropertyProperty(property) => {
                if !contains(property.binding.span()) {
                    return false;
                }
            }
            AstKind::AssignmentExpression(assignment) => return contains(assignment.left.span()),
            AstKind::ForInStatement(statement) => return contains(statement.left.span()),
            AstKind::ForOfStatement(statement) => return contains(statement.left.span()),
            _ => return false,
        }
    }
    false
}

fn variable_declarator_name_for_call(nodes: &AstNodes<'_>, node_id: NodeId) -> Option<String> {
    let AstKind::VariableDeclarator(declarator) = nodes.parent_kind(node_id) else {
        return None;
    };
    if !declarator.init.as_ref().is_some_and(|init| {
        let AstKind::CallExpression(call) = nodes.kind(node_id) else {
            return false;
        };
        init.span() == call.span
    }) {
        return None;
    }
    variable_declarator_name(declarator)
}

fn variable_declarator_name(declarator: &VariableDeclarator<'_>) -> Option<String> {
    binding_pattern_identifier_name(&declarator.id)
}

fn binding_pattern_identifier_name(pattern: &BindingPattern<'_>) -> Option<String> {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => Some(identifier.name.to_string()),
        BindingPattern::AssignmentPattern(pattern) => {
            binding_pattern_identifier_name(&pattern.left)
        }
        _ => None,
    }
}

fn argument_identifier_name(argument: &Argument<'_>) -> Option<String> {
    match argument {
        Argument::Identifier(identifier) => Some(identifier.name.to_string()),
        Argument::ParenthesizedExpression(expression) => {
            expression_identifier_name(&expression.expression)
        }
        Argument::TSAsExpression(expression) => expression_identifier_name(&expression.expression),
        Argument::TSSatisfiesExpression(expression) => {
            expression_identifier_name(&expression.expression)
        }
        Argument::TSNonNullExpression(expression) => {
            expression_identifier_name(&expression.expression)
        }
        Argument::TSTypeAssertion(expression) => expression_identifier_name(&expression.expression),
        Argument::TSInstantiationExpression(expression) => {
            expression_identifier_name(&expression.expression)
        }
        _ => None,
    }
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
        Expression::TSInstantiationExpression(expression) => {
            expression_identifier_name(&expression.expression)
        }
        _ => None,
    }
}
