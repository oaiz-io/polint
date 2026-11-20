//! The walker that lowers one file's syntax and scopes into flow rows.
//!
//! Every expression is lowered: a construct the rows model gets its rows, and
//! any other operation gives an open value and passes its operands to the
//! sink. The rows are flow-insensitive, so a binding holds every value any
//! assignment gives it. Constructs whose bindings the scopes cannot resolve
//! (`with`, a direct `eval`) and syntax the walker does not know (JSX, V8
//! intrinsics, auto-accessors) stop the walk, and the file is not lowered.

mod platform;

use std::collections::{BTreeMap, BTreeSet};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrayExpression, ArrayExpressionElement, ArrowFunctionExpression,
    AssignmentExpression, AssignmentOperator, AssignmentTarget, AssignmentTargetMaybeDefault,
    AssignmentTargetPattern, AssignmentTargetProperty, BinaryExpression, BinaryOperator,
    BindingPattern, CallExpression, ChainElement, Class, ClassElement, Declaration,
    ExportDefaultDeclarationKind, Expression, ForStatementInit, ForStatementLeft, FormalParameters,
    Function, FunctionType, IdentifierReference, ImportDeclarationSpecifier, LogicalOperator,
    MethodDefinitionKind, ModuleDeclaration, ModuleExportName, NewExpression, ObjectExpression,
    ObjectProperty, ObjectPropertyKind, Program, PropertyKey, PropertyKind, SimpleAssignmentTarget,
    Statement, TSModuleDeclaration, TSModuleDeclarationBody, TSModuleDeclarationName,
    TSModuleReference, TaggedTemplateExpression, UnaryOperator, UpdateExpression,
    VariableDeclaration, VariableDeclarationKind,
};
use oxc_semantic::{AstNodes, Scoping, SemanticBuilder};
use oxc_span::{GetSpan, Span};

use super::{
    FlowCall, FlowObject, FlowRequest, FlowRow, FlowSpan, FlowVar, Invocable, LoweredFunction,
    TsValueFlows,
};
use crate::analysis_api::{ImportSyntax, SourceFile};
use crate::ts::parse::parse_ts_file;
use crate::ts::sealed_bindings::{
    DynamicScope, bindings_written_by_block_functions, dynamic_scope, is_direct_eval,
};
use crate::ts::spans::{
    normalized_call_expression_span, normalized_new_expression_span,
    normalized_tagged_template_span,
};
use crate::ts::{class_method_function_span, object_method_function_span};

/// Parses and lowers one file. A file that does not parse completely is not
/// lowered.
pub(crate) fn lower_ts_source_value_flows(file: &SourceFile) -> TsValueFlows {
    let allocator = Allocator::default();
    let parsed = parse_ts_file(&allocator, file);
    if parsed.is_catastrophic() || !parsed.fully_parsed {
        return TsValueFlows::default();
    }
    let semantic = SemanticBuilder::new().build(parsed.program()).semantic;
    lower_ts_program_value_flows(
        file.source.as_ref(),
        parsed.program(),
        semantic.scoping(),
        semantic.nodes(),
    )
}

/// Lowers a fully parsed program with its scopes.
pub(crate) fn lower_ts_program_value_flows(
    source: &str,
    program: &Program<'_>,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
) -> TsValueFlows {
    let mut lowerer = Lowerer {
        source,
        scoping,
        flows: TsValueFlows::default(),
        next_temp: 0,
        frames: Vec::new(),
        aborted: false,
        bodied: BTreeSet::new(),
        bodiless: BTreeSet::new(),
        constants: BTreeMap::new(),
        copy_sources: BTreeMap::new(),
        field_sources: BTreeMap::new(),
        bound_values: BTreeMap::new(),
        classes: Vec::new(),
        dynamic: dynamic_scope(program, scoping, nodes),
    };
    lowerer.flows.esm = program.body.iter().any(|statement| {
        matches!(
            statement.as_module_declaration(),
            Some(
                ModuleDeclaration::ImportDeclaration(_)
                    | ModuleDeclaration::ExportAllDeclaration(_)
                    | ModuleDeclaration::ExportDefaultDeclaration(_)
                    | ModuleDeclaration::ExportNamedDeclaration(_)
            )
        )
    });
    // A block function also writes the same-named binding of its function
    // scope when its block runs, which no reference shows: the binding may
    // hold a value the rows do not describe.
    for symbol in bindings_written_by_block_functions(scoping, nodes) {
        let local = lowerer.local(symbol);
        if !lowerer.aborted {
            lowerer.copy(local, FlowVar::Open);
        }
    }
    // A `with` body may read an object property for any name of the file. A
    // direct eval may write the bindings its code names, and exposes what
    // they hold to its code.
    if lowerer.dynamic.whole_file() {
        lowerer.abort();
    }
    let written = lowerer.dynamic.bindings().clone();
    let exposed = lowerer.dynamic.exposed().clone();
    for symbol in written.union(&exposed) {
        let local = lowerer.local(*symbol);
        if !lowerer.aborted {
            if written.contains(symbol) {
                lowerer.copy(local, FlowVar::Open);
            }
            lowerer.sink(local);
        }
    }
    lowerer.statements(&program.body);
    lowerer.finish()
}

/// A function whose body is being lowered.
struct Frame {
    span: FlowSpan,
    /// Arrows take `this` and `arguments` from the enclosing function.
    arrow: bool,
    params: u8,
    /// Whether `arguments` was read in this function.
    arguments_opened: bool,
    returned_field: Option<&'static str>,
}

/// Where an assignment writes.
enum Place {
    Var(FlowVar),
    Field {
        base: FlowVar,
        field: Box<str>,
    },
    /// A field whose key the rows do not know, of `base`.
    UnknownField {
        base: FlowVar,
    },
    /// A global binding: code the rows do not describe reads it.
    Global,
}

/// The home object of the code being lowered: a class, or an object literal
/// for its methods. `super` reads the current
/// prototype of the constructor or of its prototype object, not the class the
/// `extends` clause named: `Object.setPrototypeOf` and `__proto__` replace it.
struct ClassFrame {
    constructor: FlowVar,
    prototype: FlowVar,
    static_member: bool,
}

struct Lowerer<'s> {
    source: &'s str,
    scoping: &'s Scoping,
    flows: TsValueFlows,
    next_temp: u32,
    constants: BTreeMap<u32, String>,
    copy_sources: BTreeMap<FlowVar, Vec<FlowVar>>,
    field_sources: BTreeMap<FlowVar, BTreeMap<Box<str>, Vec<FlowVar>>>,
    bound_values: BTreeMap<FlowVar, platform::BoundValue>,
    classes: Vec<ClassFrame>,
    frames: Vec<Frame>,
    /// What the file's `with` statements and direct `eval` calls reach.
    dynamic: DynamicScope,
    aborted: bool,
    /// Function declarations with a body, by symbol.
    bodied: BTreeSet<u32>,
    /// Function declarations without one (`declare`, overload signatures).
    bodiless: BTreeSet<u32>,
}

fn flow_span(span: Span) -> FlowSpan {
    (span.start, span.end)
}

impl Lowerer<'_> {
    fn finish(mut self) -> TsValueFlows {
        let mut preamble = vec![
            FlowRow::Alloc {
                dst: FlowVar::Namespace,
                object: FlowObject::Namespace,
            },
            FlowRow::Alloc {
                dst: FlowVar::Module,
                object: FlowObject::Module,
            },
            FlowRow::Alloc {
                dst: FlowVar::Exports,
                object: FlowObject::Exports,
            },
        ];
        let replaces_exports = self.flows.rows.iter().any(|row| matches!(row, FlowRow::Store {base: FlowVar::Module, field, ..} if field.as_ref() == "exact:exports"));
        if self.flows.esm || !replaces_exports {
            preamble.push(FlowRow::Store {
                base: FlowVar::Module,
                field: "exact:exports".into(),
                src: if self.flows.esm {
                    FlowVar::Namespace
                } else {
                    FlowVar::Exports
                },
            });
        }
        if !self.flows.esm {
            let exported = self.load(FlowVar::Module, "exact:exports".into());
            self.store(FlowVar::Namespace, "exact:default".into(), exported);
            self.store(FlowVar::Namespace, "$proto".into(), exported);
        }
        preamble.append(&mut self.flows.rows);
        self.flows.rows = preamble;
        let bodiless = std::mem::take(&mut self.bodiless);
        let bodied = std::mem::take(&mut self.bodied);
        for &symbol in bodiless.difference(&bodied) {
            self.copy(FlowVar::Local(symbol), FlowVar::Open);
        }
        if self.aborted {
            return TsValueFlows::default();
        }
        self.flows.lowered = true;
        self.flows
    }

    fn abort(&mut self) -> FlowVar {
        self.aborted = true;
        FlowVar::Open
    }

    fn request(&mut self, specifier: &str, syntax: ImportSyntax, namespace: bool) -> FlowVar {
        let index = self
            .flows
            .requests
            .iter()
            .position(|request| request.specifier.as_ref() == specifier && request.syntax == syntax)
            .unwrap_or_else(|| {
                self.flows.requests.push(FlowRequest {
                    specifier: specifier.into(),
                    syntax,
                });
                self.flows.requests.len() - 1
            }) as u32;
        if namespace {
            FlowVar::RequestNamespace(index)
        } else {
            FlowVar::RequestModule(index)
        }
    }

    fn temp(&mut self) -> FlowVar {
        let temp = FlowVar::Temp(self.next_temp);
        self.next_temp = self.next_temp.saturating_add(1);
        temp
    }

    fn copy(&mut self, dst: FlowVar, src: FlowVar) {
        debug_assert!(!matches!(dst, FlowVar::Open | FlowVar::Prim));
        if dst != src {
            self.copy_sources.entry(dst).or_default().push(src);
            self.flows.rows.push(FlowRow::Copy { dst, src });
        }
    }

    fn sink(&mut self, value: FlowVar) {
        if !matches!(value, FlowVar::Prim | FlowVar::Open | FlowVar::Sink) {
            self.flows.rows.push(FlowRow::Copy {
                dst: FlowVar::Sink,
                src: value,
            });
        }
    }

    fn join(&mut self, left: FlowVar, right: FlowVar) -> FlowVar {
        if left == right {
            return left;
        }
        let joined = self.temp();
        self.copy(joined, left);
        self.copy(joined, right);
        joined
    }

    fn load(&mut self, base: FlowVar, field: Box<str>) -> FlowVar {
        let dst = self.temp();
        self.flows.rows.push(FlowRow::Load { dst, base, field });
        dst
    }

    fn store(&mut self, base: FlowVar, field: Box<str>, src: FlowVar) {
        self.field_sources
            .entry(base)
            .or_default()
            .entry(field.clone())
            .or_default()
            .push(src);
        let numeric = field
            .strip_prefix("exact:")
            .is_some_and(|key| key.parse::<usize>().is_ok());
        self.flows.rows.push(FlowRow::Store { base, field, src });
        if numeric {
            self.store(base, "$elements".into(), src);
        }
    }

    fn local(&mut self, symbol: oxc_semantic::SymbolId) -> FlowVar {
        match u32::try_from(symbol.index()) {
            Ok(index) => FlowVar::Local(index),
            Err(_) => self.abort(),
        }
    }

    fn binding_local(&mut self, symbol: Option<oxc_semantic::SymbolId>) -> Option<FlowVar> {
        let symbol = symbol?;
        Some(self.local(symbol))
    }

    /// The binding a reference resolves to, if the scopes resolve it.
    ///
    /// A global a non-strict eval may declare a `var` over is open.
    fn reference_local(&mut self, reference: &IdentifierReference<'_>) -> Option<FlowVar> {
        let id = reference.reference_id.get()?;
        let symbol = self.scoping.get_reference(id).symbol_id();
        if symbol.is_none() && self.dynamic.shadows(id) {
            return Some(FlowVar::Open);
        }
        self.binding_local(symbol)
    }

    /// `this` of the nearest function that binds one; the top level's is
    /// open.
    fn this(&self) -> FlowVar {
        self.frames
            .iter()
            .rev()
            .find(|frame| !frame.arrow)
            .map_or(FlowVar::Open, |frame| FlowVar::This(frame.span))
    }

    /// `arguments` aliases every parameter of the nearest function that binds
    /// it, and exposes them to whatever reads it.
    fn arguments(&mut self) -> FlowVar {
        let Some(frame) = self.frames.iter_mut().rev().find(|frame| !frame.arrow) else {
            return FlowVar::Open;
        };
        if frame.arguments_opened {
            return FlowVar::Open;
        }
        frame.arguments_opened = true;
        let (span, params) = (frame.span, frame.params);
        for position in 0..params {
            let param = FlowVar::Param(span, position);
            self.copy(param, FlowVar::Open);
            self.sink(param);
        }
        FlowVar::Open
    }

    // Functions.

    fn param_count(&mut self, params: &FormalParameters<'_>) -> Option<u8> {
        match u8::try_from(params.items.len()) {
            Ok(count) => Some(count),
            Err(_) => {
                self.abort();
                None
            }
        }
    }

    /// Binds the formal parameters of the frame on top.
    fn bind_params(&mut self, span: FlowSpan, params: &FormalParameters<'_>) {
        for (position, param) in params.items.iter().enumerate() {
            let Ok(position) = u8::try_from(position) else {
                self.abort();
                return;
            };
            let mut value = FlowVar::Param(span, position);
            if let Some(initializer) = &param.initializer {
                let default = self.expression(initializer);
                value = self.join(value, default);
            }
            self.bind(&param.pattern, value);
            if (param.accessibility.is_some() || param.readonly || param.r#override)
                && let BindingPattern::BindingIdentifier(identifier) = &param.pattern
            {
                let this = FlowVar::This(span);
                self.store(this, format!("exact:{}", identifier.name).into(), value);
            }
        }
        if let Some(rest) = &params.rest {
            self.bind(&rest.rest.argument, FlowVar::Open);
        }
    }

    /// Lowers a function with a body at `span`, the span the frontend gives
    /// its function, and returns its function object. `invocable` says how
    /// its syntactic position lets it be invoked: an async function or a
    /// generator can never be constructed.
    fn function(&mut self, span: Span, function: &Function<'_>, invocable: Invocable) -> FlowVar {
        let Some(body) = &function.body else {
            return FlowVar::Open;
        };
        let Some(params) = self.param_count(&function.params) else {
            return FlowVar::Open;
        };
        let span = flow_span(span);
        if function.r#type == FunctionType::FunctionExpression
            && let Some(local) = self.binding_local(
                function
                    .id
                    .as_ref()
                    .and_then(|identifier| identifier.symbol_id.get()),
            )
        {
            self.copy(local, FlowVar::Function(span));
        }
        self.frames.push(Frame {
            span,
            arrow: false,
            params,
            arguments_opened: false,
            returned_field: if function.generator {
                Some("$elements")
            } else if function.r#async {
                Some("$fulfilled")
            } else {
                None
            },
        });
        self.suspended_return(span, function.r#async || function.generator);
        self.bind_params(span, &function.params);
        self.statements(&body.statements);
        self.frames.pop();
        let invocable = Invocable {
            construct: invocable.construct && !function.r#async && !function.generator,
            ..invocable
        };
        self.flows.functions.push(LoweredFunction {
            span,
            params,
            invocable,
        });
        FlowVar::Function(span)
    }

    fn arrow(&mut self, arrow: &ArrowFunctionExpression<'_>) -> FlowVar {
        let Some(params) = self.param_count(&arrow.params) else {
            return FlowVar::Open;
        };
        let span = flow_span(arrow.span);
        self.frames.push(Frame {
            span,
            arrow: true,
            params,
            arguments_opened: false,
            returned_field: arrow.r#async.then_some("$fulfilled"),
        });
        self.suspended_return(span, arrow.r#async);
        self.bind_params(span, &arrow.params);
        if arrow.expression {
            if let Some(Statement::ExpressionStatement(statement)) = arrow.body.statements.first() {
                let value = self.expression(&statement.expression);
                self.return_value(value);
            }
        } else {
            self.statements(&arrow.body.statements);
        }
        self.frames.pop();
        self.flows.functions.push(LoweredFunction {
            span,
            params,
            invocable: Invocable::CALL,
        });
        FlowVar::Function(span)
    }

    /// An async function or a generator hands its returned values to a
    /// promise or an iterator, and the caller gets that instead.
    fn suspended_return(&mut self, span: FlowSpan, suspends: bool) {
        if suspends {
            let ret = FlowVar::Return(span);
            self.flows.rows.push(FlowRow::Alloc {
                dst: ret,
                object: FlowObject::Site(span),
            });
        }
    }

    fn return_value(&mut self, value: FlowVar) {
        if let Some(frame) = self.frames.last() {
            let ret = FlowVar::Return(frame.span);
            if let Some(field) = frame.returned_field {
                if field == "$fulfilled" {
                    self.store(ret, field.into(), value);
                    let fulfilled = self.load(value, field.into());
                    self.store(ret, field.into(), fulfilled);
                }
            } else {
                self.copy(ret, value);
            }
        } else {
            self.sink(value);
        }
    }

    /// A function the frontend makes of a class member's code: a static
    /// block, or a field initializer.
    fn member_body(&mut self, span: Span, lower: impl FnOnce(&mut Self)) {
        let span = flow_span(span);
        self.frames.push(Frame {
            span,
            arrow: false,
            params: 0,
            arguments_opened: false,
            returned_field: None,
        });
        lower(self);
        self.frames.pop();
        self.flows.functions.push(LoweredFunction {
            span,
            params: 0,
            invocable: Invocable::NEVER,
        });
    }

    // Classes.

    fn class(&mut self, class: &Class<'_>) -> FlowVar {
        if class.declare {
            return FlowVar::Open;
        }
        let span = flow_span(class.span);
        let value = FlowVar::Function(span);
        if let Some(local) = self.binding_local(class.id.as_ref().and_then(|id| id.symbol_id.get()))
        {
            self.copy(local, value);
        }
        let prototype = self.load(value, "exact:prototype".into());
        let parent = class
            .super_class
            .as_ref()
            .map_or(FlowVar::Prim, |parent| self.expression(parent));
        if class.super_class.is_some() {
            let parent_prototype = self.load(parent, "exact:prototype".into());
            self.store(value, "$proto".into(), parent);
            self.store(prototype, "$proto".into(), parent_prototype);
        }
        for decorator in &class.decorators {
            let decorator = self.expression(&decorator.expression);
            self.sink(decorator);
            self.sink(value);
        }
        self.classes.push(ClassFrame {
            constructor: value,
            prototype,
            static_member: false,
        });
        let mut explicit = false;
        for element in &class.body.body {
            match element {
                ClassElement::MethodDefinition(method) => {
                    if let Some(frame) = self.classes.last_mut() {
                        frame.static_member = method.r#static;
                    }
                    let field = self.key_field(&method.key, method.computed);
                    let constructor = method.kind == MethodDefinitionKind::Constructor;
                    explicit |= constructor;
                    let function = self.function(
                        if constructor {
                            class.span
                        } else {
                            class_method_function_span(method, self.source)
                        },
                        &method.value,
                        if constructor {
                            Invocable::CONSTRUCT
                        } else {
                            Invocable::CALL
                        },
                    );
                    let base = if method.r#static { value } else { prototype };
                    if !constructor {
                        if method.kind == MethodDefinitionKind::Method
                            && let Some(field) = field
                        {
                            self.store(base, field, function);
                        } else {
                            self.sink(function);
                            self.sink(base);
                        }
                    }
                    for decorator in &method.decorators {
                        let decorator = self.expression(&decorator.expression);
                        self.sink(decorator);
                        self.sink(function);
                        self.sink(base);
                    }
                }
                ClassElement::PropertyDefinition(property) => {
                    if property.declare {
                        continue;
                    }
                    if let Some(frame) = self.classes.last_mut() {
                        frame.static_member = property.r#static;
                    }
                    let field = self.key_field(&property.key, property.computed);
                    let this = if property.r#static {
                        value
                    } else {
                        FlowVar::This(span)
                    };
                    if let Some(init) = &property.value {
                        let init_span = Span::new(property.key.span().start, property.span.end);
                        self.copy(FlowVar::This(flow_span(init_span)), this);
                        self.member_body(init_span, |lowerer| {
                            let init = lowerer.expression(init);
                            if let Some(field) = field {
                                lowerer.store(this, field, init);
                            } else {
                                lowerer.sink(this);
                                lowerer.sink(init);
                            }
                        });
                    } else if let Some(field) = field {
                        self.store(this, field, FlowVar::Prim);
                    }
                    for decorator in &property.decorators {
                        let decorator = self.expression(&decorator.expression);
                        self.sink(decorator);
                        self.sink(this);
                    }
                }
                ClassElement::StaticBlock(block) => {
                    if let Some(frame) = self.classes.last_mut() {
                        frame.static_member = true;
                    }
                    self.copy(FlowVar::This(flow_span(block.span)), value);
                    self.member_body(block.span, |lowerer| lowerer.statements(&block.body));
                }
                ClassElement::AccessorProperty(_) => {
                    self.abort();
                }
                ClassElement::TSIndexSignature(_) => {}
            }
        }
        if !explicit {
            // The frontend emits a class callable even for an implicit constructor.
            self.flows.functions.push(LoweredFunction {
                span,
                params: 0,
                invocable: Invocable::CONSTRUCT,
            });
            if class.super_class.is_some() {
                let callee = self.load(value, "$proto".into());
                let result = self.temp();
                self.flows.calls.push(FlowCall {
                    site: span,
                    callee,
                    receiver: Some(FlowVar::This(span)),
                    arguments: Vec::new(),
                    // The implicit constructor forwards its argument list;
                    // element flow through that list is not described yet.
                    spread_from: Some(0),
                    result,
                    constructs: None,
                    construct: true,
                    model: None,
                });
                self.copy(FlowVar::Return(span), result);
            }
        }
        self.classes.pop();
        value
    }

    // Statements.

    fn statements(&mut self, statements: &[Statement<'_>]) {
        for statement in statements {
            if self.aborted {
                return;
            }
            self.statement(statement);
        }
    }

    fn statement(&mut self, statement: &Statement<'_>) {
        match statement {
            Statement::BlockStatement(block) => self.statements(&block.body),
            Statement::BreakStatement(_)
            | Statement::ContinueStatement(_)
            | Statement::DebuggerStatement(_)
            | Statement::EmptyStatement(_) => {}
            Statement::DoWhileStatement(statement) => {
                self.statement(&statement.body);
                self.expression(&statement.test);
            }
            Statement::WhileStatement(statement) => {
                self.expression(&statement.test);
                self.statement(&statement.body);
            }
            Statement::ExpressionStatement(statement) => {
                self.expression(&statement.expression);
            }
            Statement::ForInStatement(statement) => {
                // The loop reads the keys of the object, which are strings.
                self.expression(&statement.right);
                self.for_left(&statement.left, FlowVar::Prim);
                self.statement(&statement.body);
            }
            Statement::ForOfStatement(statement) => {
                let iterated = self.expression(&statement.right);
                let mut elements = self.load(iterated, "$elements".into());
                if !self.known_fields(iterated).contains_key("$array") {
                    elements = self.join(elements, FlowVar::Open);
                }
                self.for_left(&statement.left, elements);
                self.statement(&statement.body);
            }
            Statement::ForStatement(statement) => {
                match &statement.init {
                    Some(ForStatementInit::VariableDeclaration(declaration)) => {
                        self.variable_declaration(declaration);
                    }
                    Some(init) => {
                        if let Some(expression) = init.as_expression() {
                            self.expression(expression);
                        }
                    }
                    None => {}
                }
                if let Some(test) = &statement.test {
                    self.expression(test);
                }
                if let Some(update) = &statement.update {
                    self.expression(update);
                }
                self.statement(&statement.body);
            }
            Statement::IfStatement(statement) => {
                self.expression(&statement.test);
                self.statement(&statement.consequent);
                if let Some(alternate) = &statement.alternate {
                    self.statement(alternate);
                }
            }
            Statement::LabeledStatement(statement) => self.statement(&statement.body),
            Statement::ReturnStatement(statement) => {
                let value = statement
                    .argument
                    .as_ref()
                    .map_or(FlowVar::Prim, |argument| self.expression(argument));
                self.return_value(value);
            }
            Statement::SwitchStatement(statement) => {
                self.expression(&statement.discriminant);
                for case in &statement.cases {
                    if let Some(test) = &case.test {
                        self.expression(test);
                    }
                    self.statements(&case.consequent);
                }
            }
            Statement::ThrowStatement(statement) => {
                let value = self.expression(&statement.argument);
                self.sink(value);
            }
            Statement::TryStatement(statement) => {
                self.statements(&statement.block.body);
                if let Some(handler) = &statement.handler {
                    if let Some(param) = &handler.param {
                        self.bind(&param.pattern, FlowVar::Open);
                    }
                    self.statements(&handler.body.body);
                }
                if let Some(finalizer) = &statement.finalizer {
                    self.statements(&finalizer.body);
                }
            }
            Statement::WithStatement(_) => {
                self.abort();
            }
            _ => {
                if let Some(declaration) = statement.as_declaration() {
                    self.declaration(declaration);
                } else if let Some(declaration) = statement.as_module_declaration() {
                    self.module_declaration(declaration);
                }
            }
        }
    }

    fn for_left(&mut self, left: &ForStatementLeft<'_>, value: FlowVar) {
        match left {
            ForStatementLeft::VariableDeclaration(declaration) => {
                for declarator in &declaration.declarations {
                    let value = match &declarator.init {
                        Some(init) => {
                            let init = self.expression(init);
                            self.join(value, init)
                        }
                        None => value,
                    };
                    self.bind(&declarator.id, value);
                }
            }
            _ => {
                if let Some(target) = left.as_assignment_target() {
                    self.assign(target, value);
                }
            }
        }
    }

    fn variable_declaration(&mut self, declaration: &VariableDeclaration<'_>) {
        if declaration.declare {
            for declarator in &declaration.declarations {
                self.bind(&declarator.id, FlowVar::Open);
            }
            return;
        }
        let disposed = matches!(
            declaration.kind,
            VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
        );
        for declarator in &declaration.declarations {
            if declaration.kind == VariableDeclarationKind::Const
                && let BindingPattern::BindingIdentifier(id) = &declarator.id
                && let Some(symbol) = id.symbol_id.get()
                && !self.dynamic.reaches(symbol)
                && let Some(init) = &declarator.init
                && let Some(value) = self.constant_string(init, 0)
            {
                self.constants.insert(symbol.index() as u32, value);
            }
            let value = declarator
                .init
                .as_ref()
                .map_or(FlowVar::Prim, |init| self.expression(init));
            if disposed {
                self.sink(value);
            }
            self.bind(&declarator.id, value);
        }
    }

    fn declaration(&mut self, declaration: &Declaration<'_>) {
        match declaration {
            Declaration::VariableDeclaration(declaration) => {
                self.variable_declaration(declaration);
            }
            Declaration::FunctionDeclaration(function) => {
                self.function_declaration(function);
            }
            Declaration::ClassDeclaration(class) => {
                self.class(class);
            }
            Declaration::TSTypeAliasDeclaration(_)
            | Declaration::TSInterfaceDeclaration(_)
            | Declaration::TSGlobalDeclaration(_) => {}
            Declaration::TSEnumDeclaration(declaration) => {
                if !declaration.declare {
                    for member in &declaration.body.members {
                        if let Some(initializer) = &member.initializer {
                            let value = self.expression(initializer);
                            self.sink(value);
                        }
                    }
                }
                if let Some(local) = self.binding_local(declaration.id.symbol_id.get()) {
                    self.copy(local, FlowVar::Open);
                }
            }
            Declaration::TSModuleDeclaration(declaration) => {
                self.namespace(declaration);
            }
            Declaration::TSImportEqualsDeclaration(declaration) => {
                if let Some(local) = self.binding_local(declaration.id.symbol_id.get()) {
                    let value = match &declaration.module_reference {
                        TSModuleReference::ExternalModuleReference(reference) => {
                            let module = self.request(
                                reference.expression.value.as_str(),
                                ImportSyntax::Require,
                                false,
                            );
                            self.load(module, "exact:exports".into())
                        }
                        _ => FlowVar::Open,
                    };
                    self.copy(local, value);
                }
            }
        }
    }

    fn function_declaration(&mut self, function: &Function<'_>) -> FlowVar {
        let symbol = function
            .id
            .as_ref()
            .and_then(|identifier| identifier.symbol_id.get());
        let local = self.binding_local(symbol);
        if function.declare || function.body.is_none() {
            if let Some(FlowVar::Local(symbol)) = local {
                self.bodiless.insert(symbol);
            }
            return local.unwrap_or(FlowVar::Open);
        }
        let value = self.function(function.span, function, Invocable::PLAIN);
        if let Some(local) = local {
            if let FlowVar::Local(symbol) = local {
                self.bodied.insert(symbol);
            }
            self.copy(local, value);
        }
        value
    }

    /// A namespace is an object the rows do not model; its exported members
    /// escape.
    fn namespace(&mut self, declaration: &TSModuleDeclaration<'_>) {
        if let TSModuleDeclarationName::Identifier(identifier) = &declaration.id
            && let Some(local) = self.binding_local(identifier.symbol_id.get())
        {
            self.copy(local, FlowVar::Open);
        }
        if declaration.declare {
            return;
        }
        match &declaration.body {
            Some(TSModuleDeclarationBody::TSModuleBlock(block)) => self.statements(&block.body),
            Some(TSModuleDeclarationBody::TSModuleDeclaration(inner)) => self.namespace(inner),
            None => {}
        }
    }

    fn module_declaration(&mut self, declaration: &ModuleDeclaration<'_>) {
        match declaration {
            ModuleDeclaration::ImportDeclaration(declaration) => {
                let request = self.request(
                    declaration.source.value.as_str(),
                    ImportSyntax::Import,
                    true,
                );
                for specifier in declaration.specifiers.iter().flatten() {
                    let (id, value) = match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(specifier) => (
                            &specifier.local,
                            self.load(
                                request,
                                format!("exact:{}", specifier.imported.name()).into(),
                            ),
                        ),
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                            (&specifier.local, self.load(request, "exact:default".into()))
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                            (&specifier.local, request)
                        }
                    };
                    if let Some(local) = self.binding_local(id.symbol_id.get()) {
                        self.copy(local, value);
                    }
                }
            }
            ModuleDeclaration::ExportAllDeclaration(declaration) => {
                let request = self.request(
                    declaration.source.value.as_str(),
                    ImportSyntax::Import,
                    true,
                );
                let field = declaration.exported.as_ref().map_or_else(
                    || "$proto".into(),
                    |name| format!("exact:{}", name.name()).into(),
                );
                self.store(FlowVar::Namespace, field, request);
            }
            ModuleDeclaration::TSNamespaceExportDeclaration(_) => {}
            ModuleDeclaration::ExportDefaultDeclaration(declaration) => {
                let value = match &declaration.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                        self.function_declaration(function)
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => self.class(class),
                    ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => FlowVar::Prim,
                    kind => kind
                        .as_expression()
                        .map_or(FlowVar::Prim, |expression| self.expression(expression)),
                };
                self.store(FlowVar::Namespace, "exact:default".into(), value);
            }
            ModuleDeclaration::ExportNamedDeclaration(declaration) => {
                if let Some(inner) = &declaration.declaration {
                    self.declaration(inner);
                    let mut bound = Vec::new();
                    declared_symbols(inner, &mut bound);
                    for symbol in bound {
                        let field = format!("exact:{}", self.scoping.symbol_name(symbol)).into();
                        let local = self.local(symbol);
                        self.store(FlowVar::Namespace, field, local);
                    }
                }
                let request = declaration
                    .source
                    .as_ref()
                    .map(|source| self.request(source.value.as_str(), ImportSyntax::Import, true));
                for specifier in &declaration.specifiers {
                    let value = match request {
                        Some(request) => {
                            self.load(request, format!("exact:{}", specifier.local.name()).into())
                        }
                        None => match &specifier.local {
                            ModuleExportName::IdentifierReference(reference) => {
                                self.reference_local(reference).unwrap_or(FlowVar::Open)
                            }
                            _ => FlowVar::Open,
                        },
                    };
                    self.store(
                        FlowVar::Namespace,
                        format!("exact:{}", specifier.exported.name()).into(),
                        value,
                    );
                }
            }
            ModuleDeclaration::TSExportAssignment(assignment) => {
                let value = self.expression(&assignment.expression);
                self.store(FlowVar::Module, "exact:exports".into(), value);
            }
        }
    }

    // Bindings and assignments.

    fn bind(&mut self, pattern: &BindingPattern<'_>, value: FlowVar) {
        match pattern {
            BindingPattern::BindingIdentifier(identifier) => {
                match self.binding_local(identifier.symbol_id.get()) {
                    Some(local) => self.copy(local, value),
                    None => self.sink(value),
                }
            }
            BindingPattern::AssignmentPattern(pattern) => {
                let default = self.expression(&pattern.right);
                let value = self.join(value, default);
                self.bind(&pattern.left, value);
            }
            BindingPattern::ObjectPattern(pattern) => {
                for property in &pattern.properties {
                    match self.key_field(&property.key, property.computed) {
                        Some(field) => {
                            let loaded = self.load(value, field);
                            self.bind(&property.value, loaded);
                        }
                        None => {
                            self.sink(value);
                            self.bind(&property.value, FlowVar::Open);
                        }
                    }
                }
                if let Some(rest) = &pattern.rest {
                    self.sink(value);
                    self.bind(&rest.argument, FlowVar::Open);
                }
            }
            BindingPattern::ArrayPattern(pattern) => {
                for (index, element) in pattern.elements.iter().enumerate() {
                    if let Some(element) = element {
                        let element_value = self.array_element(value, index);
                        self.bind(element, element_value);
                    }
                }
                if let Some(rest) = &pattern.rest {
                    self.bind(&rest.argument, FlowVar::Open);
                }
            }
        }
    }

    fn array_element(&mut self, value: FlowVar, index: usize) -> FlowVar {
        let indexed = self.load(value, format!("exact:{index}").into());
        let fields = self.known_fields(value);
        if fields.contains_key("$array") && !fields.contains_key("$spread") {
            return indexed;
        }
        let elements = self.load(value, "$elements".into());
        let joined = self.join(indexed, elements);
        self.join(joined, FlowVar::Open)
    }

    fn assign(&mut self, target: &AssignmentTarget<'_>, value: FlowVar) {
        if let Some(target) = target.as_simple_assignment_target() {
            let place = self.place(target);
            self.write(&place, value);
        } else if let Some(pattern) = target.as_assignment_target_pattern() {
            self.assign_pattern(pattern, value);
        }
    }

    fn assign_pattern(&mut self, pattern: &AssignmentTargetPattern<'_>, value: FlowVar) {
        match pattern {
            AssignmentTargetPattern::ArrayAssignmentTarget(pattern) => {
                for (index, element) in pattern.elements.iter().enumerate() {
                    if let Some(element) = element {
                        let element_value = self.array_element(value, index);
                        self.assign_with_default(element, element_value);
                    }
                }
                if let Some(rest) = &pattern.rest {
                    self.assign(&rest.target, FlowVar::Open);
                }
            }
            AssignmentTargetPattern::ObjectAssignmentTarget(pattern) => {
                for property in &pattern.properties {
                    match property {
                        AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(property) => {
                            let mut loaded =
                                self.load(value, format!("exact:{}", property.binding.name).into());
                            if let Some(init) = &property.init {
                                let default = self.expression(init);
                                loaded = self.join(loaded, default);
                            }
                            let place = self.identifier_place(&property.binding);
                            self.write(&place, loaded);
                        }
                        AssignmentTargetProperty::AssignmentTargetPropertyProperty(property) => {
                            match self.key_field(&property.name, property.computed) {
                                Some(field) => {
                                    let loaded = self.load(value, field);
                                    self.assign_with_default(&property.binding, loaded);
                                }
                                None => {
                                    self.sink(value);
                                    self.assign_with_default(&property.binding, FlowVar::Open);
                                }
                            }
                        }
                    }
                }
                if let Some(rest) = &pattern.rest {
                    self.sink(value);
                    self.assign(&rest.target, FlowVar::Open);
                }
            }
        }
    }

    fn assign_with_default(&mut self, target: &AssignmentTargetMaybeDefault<'_>, value: FlowVar) {
        match target {
            AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(target) => {
                let default = self.expression(&target.init);
                let value = self.join(value, default);
                self.assign(&target.binding, value);
            }
            _ => {
                if let Some(target) = target.as_assignment_target() {
                    self.assign(target, value);
                }
            }
        }
    }

    fn identifier_place(&mut self, reference: &IdentifierReference<'_>) -> Place {
        match self.reference_local(reference) {
            Some(FlowVar::Open) => Place::Global,
            Some(local) => Place::Var(local),
            None => match reference.name.as_str() {
                "module" => Place::Var(FlowVar::Module),
                "exports" => Place::Var(FlowVar::Exports),
                _ => Place::Global,
            },
        }
    }

    fn place(&mut self, target: &SimpleAssignmentTarget<'_>) -> Place {
        match target {
            SimpleAssignmentTarget::AssignmentTargetIdentifier(reference) => {
                self.identifier_place(reference)
            }
            SimpleAssignmentTarget::TSAsExpression(expression) => {
                self.expression_place(&expression.expression)
            }
            SimpleAssignmentTarget::TSSatisfiesExpression(expression) => {
                self.expression_place(&expression.expression)
            }
            SimpleAssignmentTarget::TSNonNullExpression(expression) => {
                self.expression_place(&expression.expression)
            }
            SimpleAssignmentTarget::TSTypeAssertion(expression) => {
                self.expression_place(&expression.expression)
            }
            SimpleAssignmentTarget::ComputedMemberExpression(member) => {
                self.member_place(&member.object, MemberKey::Computed(&member.expression))
            }
            SimpleAssignmentTarget::StaticMemberExpression(member) => {
                self.member_place(&member.object, MemberKey::Static(&member.property.name))
            }
            SimpleAssignmentTarget::PrivateFieldExpression(member) => {
                self.member_place(&member.object, MemberKey::Private(&member.field.name))
            }
        }
    }

    /// The place of an assignment target written inside a type wrapper.
    fn expression_place(&mut self, expression: &Expression<'_>) -> Place {
        match expression.get_inner_expression() {
            Expression::Identifier(reference) => self.identifier_place(reference),
            Expression::ComputedMemberExpression(member) => {
                self.member_place(&member.object, MemberKey::Computed(&member.expression))
            }
            Expression::StaticMemberExpression(member) => {
                self.member_place(&member.object, MemberKey::Static(&member.property.name))
            }
            Expression::PrivateFieldExpression(member) => {
                self.member_place(&member.object, MemberKey::Private(&member.field.name))
            }
            other => {
                let value = self.expression(other);
                self.sink(value);
                Place::Global
            }
        }
    }

    fn member_place(&mut self, object: &Expression<'_>, key: MemberKey<'_, '_>) -> Place {
        // `super.x = v` writes `this.x`, through setters of the prototype.
        let base = if matches!(object, Expression::Super(_)) {
            self.this()
        } else {
            self.expression(object)
        };
        match self.member_field(key) {
            // Writing `__proto__` replaces the prototype.
            Some(field) if &*field == "exact:__proto__" => Place::UnknownField { base },
            Some(field) if !matches!(object, Expression::Super(_)) => Place::Field { base, field },
            _ => Place::UnknownField { base },
        }
    }

    fn read(&mut self, place: &Place) -> FlowVar {
        match place {
            Place::Var(var) => *var,
            Place::Field { base, field } => self.load(*base, field.clone()),
            Place::UnknownField { base } => {
                self.sink(*base);
                FlowVar::Open
            }
            Place::Global => FlowVar::Open,
        }
    }

    fn write(&mut self, place: &Place, value: FlowVar) {
        match place {
            Place::Var(var) => self.copy(*var, value),
            Place::Field { base, field } => self.store(*base, field.clone(), value),
            Place::UnknownField { base } => {
                self.sink(*base);
                self.sink(value);
            }
            Place::Global => self.sink(value),
        }
    }

    // Property keys.

    /// The field a property key names, or `None` when the rows cannot know
    /// it; a key expression the rows cannot fold is lowered and escapes.
    fn key_field(&mut self, key: &PropertyKey<'_>, computed: bool) -> Option<Box<str>> {
        match key {
            PropertyKey::StaticIdentifier(identifier) if !computed => {
                Some(format!("exact:{}", identifier.name).into())
            }
            PropertyKey::StaticIdentifier(_) => None,
            PropertyKey::PrivateIdentifier(identifier) => {
                Some(format!("private:{}", identifier.name).into())
            }
            _ => key
                .as_expression()
                .and_then(|expression| self.expression_field(expression)),
        }
    }

    fn member_field(&mut self, key: MemberKey<'_, '_>) -> Option<Box<str>> {
        match key {
            MemberKey::Static(name) => Some(format!("exact:{name}").into()),
            MemberKey::Private(name) => Some(format!("private:{name}").into()),
            MemberKey::Computed(expression) => self.expression_field(expression),
        }
    }

    /// The field a computed key names when it is a literal.
    fn expression_field(&mut self, expression: &Expression<'_>) -> Option<Box<str>> {
        let folded = self.constant_string(expression, 0).or_else(|| {
            match expression.get_inner_expression() {
                Expression::StringLiteral(literal) => Some(literal.value.to_string()),
                Expression::BooleanLiteral(literal) => Some(literal.value.to_string()),
                Expression::NullLiteral(_) => Some("null".to_string()),
                Expression::NumericLiteral(literal) => integral_key(literal.value),
                Expression::TemplateLiteral(literal) => {
                    literal.single_quasi().map(|quasi| quasi.to_string())
                }
                _ => None,
            }
        });
        if let Some(folded) = folded {
            return Some(format!("exact:{folded}").into());
        }
        let value = self.expression(expression);
        self.sink(value);
        None
    }

    fn constant_string(&self, expression: &Expression<'_>, depth: u8) -> Option<String> {
        if depth >= 16 {
            return None;
        }
        match expression.get_inner_expression() {
            Expression::StringLiteral(literal) => Some(literal.value.to_string()),
            Expression::TemplateLiteral(literal) => {
                literal.single_quasi().map(|quasi| quasi.to_string())
            }
            Expression::Identifier(reference) => {
                let symbol = reference
                    .reference_id
                    .get()
                    .and_then(|id| self.scoping.get_reference(id).symbol_id())?;
                self.constants.get(&(symbol.index() as u32)).cloned()
            }
            Expression::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
                let left = self.constant_string(&binary.left, depth + 1)?;
                let right = self.constant_string(&binary.right, depth + 1)?;
                Some(left + &right)
            }
            _ => None,
        }
    }

    // Expressions.

    fn expression(&mut self, expression: &Expression<'_>) -> FlowVar {
        if self.aborted {
            return FlowVar::Open;
        }
        match expression {
            Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BigIntLiteral(_)
            | Expression::RegExpLiteral(_)
            | Expression::StringLiteral(_) => FlowVar::Prim,
            Expression::TemplateLiteral(literal) => {
                for expression in &literal.expressions {
                    let value = self.expression(expression);
                    self.sink(value);
                }
                FlowVar::Prim
            }
            Expression::Identifier(reference) => match self.reference_local(reference) {
                Some(local) => local,
                None => match reference.name.as_str() {
                    "undefined" => FlowVar::Prim,
                    "arguments" => self.arguments(),
                    "module" => FlowVar::Module,
                    "exports" => FlowVar::Exports,
                    _ => FlowVar::Open,
                },
            },
            Expression::MetaProperty(_) | Expression::Super(_) => FlowVar::Open,
            Expression::ArrayExpression(array) => self.array(array),
            Expression::ArrowFunctionExpression(arrow) => self.arrow(arrow),
            Expression::AssignmentExpression(assignment) => self.assignment(assignment),
            Expression::AwaitExpression(expression) => {
                let value = self.expression(&expression.argument);
                let fulfilled = self.load(value, "$fulfilled".into());
                self.join(value, fulfilled)
            }
            Expression::YieldExpression(expression) => {
                if let Some(argument) = &expression.argument {
                    let mut value = self.expression(argument);
                    if expression.delegate {
                        value = self.load(value, "$elements".into());
                    }
                    if let Some(frame) = self.frames.last() {
                        let ret = FlowVar::Return(frame.span);
                        self.store(ret, "$elements".into(), value);
                    }
                }
                FlowVar::Open
            }
            Expression::BinaryExpression(binary) => self.binary(binary),
            Expression::CallExpression(call) => self.call(call),
            Expression::ChainExpression(chain) => match &chain.expression {
                ChainElement::CallExpression(call) => self.call(call),
                ChainElement::TSNonNullExpression(expression) => {
                    self.expression(&expression.expression)
                }
                ChainElement::ComputedMemberExpression(member) => {
                    self.member_read(&member.object, MemberKey::Computed(&member.expression))
                }
                ChainElement::StaticMemberExpression(member) => {
                    self.member_read(&member.object, MemberKey::Static(&member.property.name))
                }
                ChainElement::PrivateFieldExpression(member) => {
                    self.member_read(&member.object, MemberKey::Private(&member.field.name))
                }
            },
            Expression::ClassExpression(class) => self.class(class),
            Expression::ConditionalExpression(conditional) => {
                self.expression(&conditional.test);
                let consequent = self.expression(&conditional.consequent);
                let alternate = self.expression(&conditional.alternate);
                self.join(consequent, alternate)
            }
            Expression::FunctionExpression(function) => {
                self.function(function.span, function, Invocable::PLAIN)
            }
            Expression::ImportExpression(import) => {
                let mut arguments = vec![self.expression(&import.source)];
                if let Some(options) = &import.options {
                    arguments.push(self.expression(options));
                }
                let result = self.temp();
                self.flows.calls.push(FlowCall {
                    site: flow_span(import.span),
                    callee: FlowVar::Open,
                    receiver: None,
                    arguments,
                    spread_from: None,
                    result,
                    constructs: None,
                    construct: false,
                    model: None,
                });
                result
            }
            Expression::LogicalExpression(logical) => {
                let left = self.expression(&logical.left);
                let right = self.expression(&logical.right);
                // `a && b` is `a` only when `a` is falsy, and no object or
                // function is falsy.
                if logical.operator == LogicalOperator::And {
                    return right;
                }
                self.join(left, right)
            }
            Expression::NewExpression(new) => self.construct(new),
            Expression::ObjectExpression(object) => self.object(object),
            Expression::ParenthesizedExpression(expression) => {
                self.expression(&expression.expression)
            }
            Expression::SequenceExpression(sequence) => {
                let mut value = FlowVar::Prim;
                for expression in &sequence.expressions {
                    value = self.expression(expression);
                }
                value
            }
            Expression::TaggedTemplateExpression(tagged) => self.tagged_template(tagged),
            Expression::ThisExpression(_) => self.this(),
            Expression::UnaryExpression(unary) => match unary.operator {
                // These read the operand's type or truth only.
                UnaryOperator::Typeof | UnaryOperator::LogicalNot | UnaryOperator::Void => {
                    self.expression(&unary.argument);
                    FlowVar::Prim
                }
                UnaryOperator::Delete => {
                    match unary.argument.get_inner_expression() {
                        Expression::ComputedMemberExpression(member) => {
                            let base = self.expression(&member.object);
                            self.sink(base);
                            let key = self.expression(&member.expression);
                            self.sink(key);
                        }
                        Expression::StaticMemberExpression(member) => {
                            let base = self.expression(&member.object);
                            self.sink(base);
                        }
                        Expression::PrivateFieldExpression(member) => {
                            let base = self.expression(&member.object);
                            self.sink(base);
                        }
                        other => {
                            self.expression(other);
                        }
                    }
                    FlowVar::Prim
                }
                UnaryOperator::UnaryPlus
                | UnaryOperator::UnaryNegation
                | UnaryOperator::BitwiseNot => {
                    let value = self.expression(&unary.argument);
                    self.sink(value);
                    FlowVar::Prim
                }
            },
            Expression::UpdateExpression(update) => self.update(update),
            Expression::PrivateInExpression(expression) => {
                self.expression(&expression.right);
                FlowVar::Prim
            }
            Expression::JSXElement(_)
            | Expression::JSXFragment(_)
            | Expression::V8IntrinsicExpression(_) => self.abort(),
            Expression::TSAsExpression(expression) => self.expression(&expression.expression),
            Expression::TSSatisfiesExpression(expression) => {
                self.expression(&expression.expression)
            }
            Expression::TSTypeAssertion(expression) => self.expression(&expression.expression),
            Expression::TSNonNullExpression(expression) => self.expression(&expression.expression),
            Expression::TSInstantiationExpression(expression) => {
                self.expression(&expression.expression)
            }
            Expression::ComputedMemberExpression(member) => {
                self.member_read(&member.object, MemberKey::Computed(&member.expression))
            }
            Expression::StaticMemberExpression(member) => {
                self.member_read(&member.object, MemberKey::Static(&member.property.name))
            }
            Expression::PrivateFieldExpression(member) => {
                self.member_read(&member.object, MemberKey::Private(&member.field.name))
            }
        }
    }

    fn member_base(&mut self, object: &Expression<'_>) -> FlowVar {
        if matches!(object, Expression::Super(_)) {
            // A method's home object is the class for a static member, and
            // its prototype object otherwise.
            match self.classes.last() {
                Some(frame) if frame.static_member => {
                    let home = frame.constructor;
                    self.load(home, "$proto".into())
                }
                Some(frame) => {
                    let home = frame.prototype;
                    self.load(home, "$proto".into())
                }
                None => FlowVar::Open,
            }
        } else {
            self.expression(object)
        }
    }

    fn member_read(&mut self, object: &Expression<'_>, key: MemberKey<'_, '_>) -> FlowVar {
        let base = self.member_base(object);
        match self.member_field(key) {
            Some(field) => self.load(base, field),
            None => {
                self.sink(base);
                FlowVar::Open
            }
        }
    }

    fn binary(&mut self, binary: &BinaryExpression<'_>) -> FlowVar {
        let left = self.expression(&binary.left);
        let right = self.expression(&binary.right);
        let compares_identity = match binary.operator {
            BinaryOperator::StrictEquality | BinaryOperator::StrictInequality => true,
            BinaryOperator::Equality | BinaryOperator::Inequality => {
                is_nullish(&binary.left) || is_nullish(&binary.right)
            }
            _ => false,
        };
        // Other operators convert their operands, which runs their methods.
        if !compares_identity {
            self.sink(left);
            self.sink(right);
        }
        FlowVar::Prim
    }

    fn assignment(&mut self, assignment: &AssignmentExpression<'_>) -> FlowVar {
        if assignment.operator.is_assign() {
            let value = self.expression(&assignment.right);
            self.assign(&assignment.left, value);
            return value;
        }
        let Some(target) = assignment.left.as_simple_assignment_target() else {
            let value = self.expression(&assignment.right);
            self.sink(value);
            return FlowVar::Open;
        };
        let place = self.place(target);
        let current = self.read(&place);
        let value = self.expression(&assignment.right);
        if assignment.operator == AssignmentOperator::LogicalAnd {
            // The place keeps its value only when that value is falsy.
            self.write(&place, value);
            return value;
        }
        if assignment.operator.is_logical() {
            let joined = self.join(current, value);
            self.write(&place, joined);
            return joined;
        }
        self.sink(current);
        self.sink(value);
        self.write(&place, FlowVar::Prim);
        debug_assert!(assignment.operator != AssignmentOperator::Assign);
        FlowVar::Prim
    }

    fn update(&mut self, update: &UpdateExpression<'_>) -> FlowVar {
        let place = self.place(&update.argument);
        let current = self.read(&place);
        self.sink(current);
        self.write(&place, FlowVar::Prim);
        FlowVar::Prim
    }

    // Calls.

    /// The callee of a call and the receiver it passes.
    fn callee(&mut self, callee: &Expression<'_>) -> (FlowVar, Option<FlowVar>) {
        let callee = callee.get_inner_expression();
        let member = match callee {
            Expression::Super(_) => {
                let parent = match self.classes.last() {
                    Some(frame) => {
                        let constructor = frame.constructor;
                        self.load(constructor, "$proto".into())
                    }
                    None => FlowVar::Open,
                };
                return (parent, Some(self.this()));
            }
            Expression::ComputedMemberExpression(member) => {
                Some((&member.object, MemberKey::Computed(&member.expression)))
            }
            Expression::StaticMemberExpression(member) => {
                Some((&member.object, MemberKey::Static(&member.property.name)))
            }
            Expression::PrivateFieldExpression(member) => {
                Some((&member.object, MemberKey::Private(&member.field.name)))
            }
            _ => None,
        };
        let Some((object, key)) = member else {
            return (self.expression(callee), None);
        };
        let base = self.member_base(object);
        let receiver = if matches!(object, Expression::Super(_)) {
            self.this()
        } else {
            base
        };
        let function = match self.member_field(key) {
            Some(field) => self.load(base, field),
            None => {
                self.sink(base);
                FlowVar::Open
            }
        };
        (function, Some(receiver))
    }

    fn arguments_of(&mut self, arguments: &[Argument<'_>]) -> (Vec<FlowVar>, Option<usize>) {
        let mut values = Vec::with_capacity(arguments.len());
        let mut spread_from = None;
        for argument in arguments {
            let value = match argument {
                Argument::SpreadElement(spread) => {
                    let value = self.expression(&spread.argument);
                    if spread_from.is_none()
                        && let Some(elements) = self.array_arguments(value)
                    {
                        values.extend(elements);
                        continue;
                    }
                    self.sink(value);
                    spread_from.get_or_insert(values.len());
                    self.load(value, "$elements".into())
                }
                _ => argument
                    .as_expression()
                    .map_or(FlowVar::Open, |expression| self.expression(expression)),
            };
            values.push(value);
        }
        (values, spread_from)
    }

    fn call(&mut self, call: &CallExpression<'_>) -> FlowVar {
        if is_direct_eval(call) {
            return self.direct_eval(call);
        }
        if let Some(value) = self.platform_call(call) {
            return value;
        }
        if let Some(value) = self.builtin_call(call) {
            return value;
        }
        let (callee, receiver) = self.callee(&call.callee);
        let (arguments, spread_from) = self.arguments_of(&call.arguments);
        let site = flow_span(normalized_call_expression_span(self.source, call));
        self.forward_bound_calls(callee, site, &arguments, spread_from);
        let result = self.temp();
        self.flows.calls.push(FlowCall {
            site: flow_span(normalized_call_expression_span(self.source, call)),
            callee,
            receiver,
            arguments,
            spread_from,
            result,
            constructs: None,
            construct: matches!(call.callee.get_inner_expression(), Expression::Super(_)),
            model: None,
        });
        result
    }

    /// A direct eval: the bindings its code can name are open from the start
    /// of the walk, see [`DynamicScope`]. Its arguments and, where its code is
    /// not known, `this` and `arguments` of the function around it escape to
    /// that code, and what it returns is open.
    fn direct_eval(&mut self, call: &CallExpression<'_>) -> FlowVar {
        let (arguments, _) = self.arguments_of(&call.arguments);
        for argument in arguments {
            self.sink(argument);
        }
        if self.dynamic.opens_context(call.span) {
            let this = self.this();
            self.sink(this);
            self.arguments();
        }
        FlowVar::Open
    }

    fn builtin_call(&mut self, call: &CallExpression<'_>) -> Option<FlowVar> {
        if let Expression::Identifier(reference) = call.callee.get_inner_expression()
            && reference.name == "require"
            && self.reference_local(reference).is_none()
            && let [argument] = call.arguments.as_slice()
            && let Some(Expression::StringLiteral(specifier)) = argument.as_expression()
        {
            let module = self.request(specifier.value.as_str(), ImportSyntax::Require, false);
            return Some(self.load(module, "exact:exports".into()));
        }
        None
    }

    fn construct(&mut self, new: &NewExpression<'_>) -> FlowVar {
        if let Some(value) = self.platform_construct(new) {
            return value;
        }
        let object = flow_span(new.span);
        let instance = self.temp();
        self.flows.rows.push(FlowRow::Alloc {
            dst: instance,
            object: FlowObject::Site(object),
        });
        let callee = self.expression(&new.callee);
        let prototype = self.load(callee, "exact:prototype".into());
        self.store(instance, "$proto".into(), prototype);
        let (arguments, spread_from) = self.arguments_of(&new.arguments);
        let result = self.temp();
        self.flows.calls.push(FlowCall {
            site: flow_span(normalized_new_expression_span(self.source, new)),
            callee,
            receiver: Some(instance),
            arguments,
            spread_from,
            result,
            constructs: Some(object),
            construct: true,
            model: None,
        });
        self.join(instance, result)
    }

    fn tagged_template(&mut self, tagged: &TaggedTemplateExpression<'_>) -> FlowVar {
        let (callee, receiver) = self.callee(&tagged.tag);
        let mut arguments = vec![FlowVar::Prim];
        for expression in &tagged.quasi.expressions {
            arguments.push(self.expression(expression));
        }
        let result = self.temp();
        self.flows.calls.push(FlowCall {
            site: flow_span(normalized_tagged_template_span(tagged)),
            callee,
            receiver,
            arguments,
            spread_from: None,
            result,
            constructs: None,
            construct: false,
            model: None,
        });
        result
    }

    // Literals.

    fn object(&mut self, object: &ObjectExpression<'_>) -> FlowVar {
        let instance = self.temp();
        self.flows.rows.push(FlowRow::Alloc {
            dst: instance,
            object: FlowObject::Site(flow_span(object.span)),
        });
        for property in &object.properties {
            match property {
                ObjectPropertyKind::SpreadProperty(spread) => {
                    let value = self.expression(&spread.argument);
                    self.sink(value);
                    self.sink(instance);
                }
                ObjectPropertyKind::ObjectProperty(property) => {
                    self.object_property(instance, property);
                }
            }
        }
        instance
    }

    fn object_property(&mut self, instance: FlowVar, property: &ObjectProperty<'_>) {
        let field = self.key_field(&property.key, property.computed);
        if property.kind != PropertyKind::Init {
            // An accessor runs when the field is read or written.
            let value = match &property.value {
                Expression::FunctionExpression(function) => self.with_home(instance, |lowerer| {
                    lowerer.function(function.span, function, Invocable::CALL)
                }),
                value => self.expression(value),
            };
            self.sink(value);
            self.sink(instance);
            return;
        }
        let value = match &property.value {
            Expression::FunctionExpression(function) if property.method => {
                self.with_home(instance, |lowerer| {
                    lowerer.function(
                        object_method_function_span(property),
                        function,
                        Invocable::CALL,
                    )
                })
            }
            value => self.expression(value),
        };
        let sets_prototype = !property.computed
            && !property.shorthand
            && field.as_deref() == Some("exact:__proto__");
        match field {
            Some(field) if !sets_prototype => self.store(instance, field, value),
            _ => {
                self.sink(instance);
                self.sink(value);
            }
        }
    }

    /// Lowers a method of an object literal, whose `super` reads the current
    /// prototype of the literal: the class around it is not its home object.
    fn with_home(&mut self, home: FlowVar, lower: impl FnOnce(&mut Self) -> FlowVar) -> FlowVar {
        self.classes.push(ClassFrame {
            constructor: home,
            prototype: home,
            static_member: false,
        });
        let value = lower(self);
        self.classes.pop();
        value
    }

    fn array(&mut self, array: &ArrayExpression<'_>) -> FlowVar {
        let instance = self.temp();
        self.flows.rows.push(FlowRow::Alloc {
            dst: instance,
            object: FlowObject::Site(flow_span(array.span)),
        });
        self.store(instance, "$array".into(), FlowVar::Prim);
        let mut spread = false;
        for (index, element) in array.elements.iter().enumerate() {
            match element {
                ArrayExpressionElement::Elision(_) => {}
                ArrayExpressionElement::SpreadElement(element) => {
                    let value = self.expression(&element.argument);
                    let elements = self.load(value, "$elements".into());
                    self.store(instance, "$elements".into(), elements);
                    self.store(instance, "$spread".into(), FlowVar::Prim);
                    spread = true;
                }
                _ => {
                    let Some(expression) = element.as_expression() else {
                        continue;
                    };
                    let value = self.expression(expression);
                    self.store(instance, "$elements".into(), value);
                    self.store(instance, "$keys".into(), FlowVar::Prim);
                    if !spread {
                        self.store(instance, format!("exact:{index}").into(), value);
                    }
                }
            }
        }
        instance
    }
}

/// The key of a member expression.
#[derive(Clone, Copy)]
enum MemberKey<'k, 'a> {
    Static(&'k str),
    Private(&'k str),
    Computed(&'k Expression<'a>),
}

fn is_nullish(expression: &Expression<'_>) -> bool {
    let expression = expression.get_inner_expression();
    expression.is_null() || expression.is_undefined() || expression.is_void_0()
}

/// The property key an integral number converts to.
fn integral_key(value: f64) -> Option<String> {
    if value == 0.0 {
        return Some("0".to_string());
    }
    (value.fract() == 0.0 && value.abs() < 1e21).then(|| format!("{value}"))
}

/// The bindings a declaration introduces.
fn declared_symbols(declaration: &Declaration<'_>, symbols: &mut Vec<oxc_semantic::SymbolId>) {
    match declaration {
        Declaration::VariableDeclaration(declaration) => {
            for declarator in &declaration.declarations {
                pattern_symbols(&declarator.id, symbols);
            }
        }
        Declaration::FunctionDeclaration(function) => {
            symbols.extend(function.id.as_ref().and_then(|id| id.symbol_id.get()));
        }
        Declaration::ClassDeclaration(class) => {
            symbols.extend(class.id.as_ref().and_then(|id| id.symbol_id.get()));
        }
        Declaration::TSEnumDeclaration(declaration) => {
            symbols.extend(declaration.id.symbol_id.get());
        }
        Declaration::TSModuleDeclaration(declaration) => {
            if let TSModuleDeclarationName::Identifier(identifier) = &declaration.id {
                symbols.extend(identifier.symbol_id.get());
            }
        }
        Declaration::TSImportEqualsDeclaration(declaration) => {
            symbols.extend(declaration.id.symbol_id.get());
        }
        Declaration::TSTypeAliasDeclaration(_)
        | Declaration::TSInterfaceDeclaration(_)
        | Declaration::TSGlobalDeclaration(_) => {}
    }
}

fn pattern_symbols(pattern: &BindingPattern<'_>, symbols: &mut Vec<oxc_semantic::SymbolId>) {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => {
            symbols.extend(identifier.symbol_id.get());
        }
        BindingPattern::AssignmentPattern(pattern) => pattern_symbols(&pattern.left, symbols),
        BindingPattern::ObjectPattern(pattern) => {
            for property in &pattern.properties {
                pattern_symbols(&property.value, symbols);
            }
            if let Some(rest) = &pattern.rest {
                pattern_symbols(&rest.argument, symbols);
            }
        }
        BindingPattern::ArrayPattern(pattern) => {
            for element in pattern.elements.iter().flatten() {
                pattern_symbols(element, symbols);
            }
            if let Some(rest) = &pattern.rest {
                pattern_symbols(&rest.argument, symbols);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::internal_core::{FileId, Language};
    use crate::ts::constraints::described::{describe, describe_program};

    fn lower(source: &str) -> TsValueFlows {
        let file = SourceFile::new(
            FileId::from_raw(0),
            PathBuf::from("src/x.js"),
            "src/x.js".to_string(),
            Language::TypeScript,
            source.to_string().into(),
            "test-hash".to_string(),
        );
        lower_ts_source_value_flows(&file)
    }

    /// The span of the call whose text starts at the `nth` occurrence of
    /// `needle`.
    fn site_of(flows: &TsValueFlows, source: &str, needle: &str) -> FlowSpan {
        let start = u32::try_from(source.find(needle).expect("needle in source")).expect("offset");
        flows
            .calls
            .iter()
            .map(|call| call.site)
            .find(|site| site.0 == start)
            .expect("a call at the needle")
    }

    #[test]
    fn a_callback_parameter_call_is_named() {
        let source = "function apply(fn) { return fn(); }\nfunction cb() {}\napply(cb);\n";
        let flows = lower(source);
        assert!(flows.lowered);
        let described = describe(&flows, |_| true);
        assert_eq!(
            described.named.get(&site_of(&flows, source, "fn()")),
            Some(&0)
        );
        assert_eq!(
            described.named.get(&site_of(&flows, source, "apply(cb)")),
            Some(&1)
        );
    }

    fn assert_named(source: &str, needles: &[&str]) {
        let flows = lower(source);
        assert!(flows.lowered);
        let described = describe(&flows, |_| true);
        for needle in needles {
            assert!(
                described
                    .named
                    .contains_key(&site_of(&flows, source, needle)),
                "{needle}: {described:?}"
            );
        }
    }

    #[test]
    fn object_fields_flow_through_parameters() {
        assert_named(
            "function f() {} function use(o) { o.run(); } const o = {}; o.run = f; use(o);",
            &["o.run()"],
        );
    }

    #[test]
    fn functions_hold_assigned_and_defined_properties() {
        assert_named(
            "function F() {} function g() {} F.run = g; Object.defineProperty(F, 'init', {value: g}); F.run(); F.init();",
            &["F.run()", "F.init()"],
        );
    }

    #[test]
    fn prototype_chains_and_own_shadowing_are_described() {
        assert_named(
            "function C() {} C.prototype.run = function run() {}; const o = new C(); o.run(); const p = Object.create(o); p.run();",
            &["new C()", "o.run()", "p.run()"],
        );
    }

    #[test]
    fn class_fields_inheritance_and_super_are_described() {
        assert_named(
            "function f() {} class A { constructor() {} run() {} } class B extends A { handler = f; constructor() { super(); } run() { super.run(); this.handler(); } } const b = new B(); b.run();",
            &[
                "super()",
                "super.run()",
                "this.handler()",
                "new B()",
                "b.run()",
            ],
        );
    }

    #[test]
    fn a_factory_constructor_keeps_its_initializer_and_prototype() {
        assert_named(
            "function make(initializer, proto) { function Made(def) { init(this, def); return this; } function init(inst, def) { initializer(inst, def); } Made.prototype.check = proto.check; Object.defineProperty(Made, 'init', {value: init}); return Made; } function initialize(inst, def) {} const C = make(initialize, {check() {}}); const inst = new C({}); inst.check(); C.init(inst, {});",
            &[
                "initializer(inst, def)",
                "new C({})",
                "inst.check()",
                "C.init(inst, {})",
            ],
        );
    }

    #[test]
    fn implicit_derived_constructors_keep_forwarded_arguments_open() {
        let source = "function fallback() {} function supplied() {} class Base { constructor(callback = fallback) { callback(); } } class Derived extends Base {} new Derived(supplied);";
        let flows = lower(source);
        let described = describe(&flows, |_| true);
        assert!(
            !described
                .named
                .contains_key(&site_of(&flows, source, "callback()"))
        );
    }

    #[test]
    fn constant_string_keys_are_folded() {
        assert_named(
            "function f() {} const PREFIX = 'on'; const o = {}; o[PREFIX + 'Open'] = f; o.onOpen();",
            &["o.onOpen()"],
        );
    }

    #[test]
    fn namespace_reexports_and_commonjs_requests_link_across_files() {
        for (library, barrel, main, needles) in [
            (
                "export function run() {}",
                "export * from './lib';",
                "import * as ns from './barrel'; ns.run();",
                vec!["ns.run()"],
            ),
            (
                "function run() {} module.exports = {run};",
                "module.exports = require('./lib');",
                "const ns = require('./barrel'); ns.run();",
                vec!["ns.run()"],
            ),
            (
                "exports.run = function run() {};",
                "export {run as task} from './lib';",
                "import {task} from './barrel'; task();",
                vec!["task()"],
            ),
        ] {
            let files = [lower(library), lower(barrel), lower(main)];
            assert!(files.iter().all(|flows| flows.lowered));
            let refs = files.iter().collect::<Vec<_>>();
            let described = describe_program(
                &refs,
                |_, _| true,
                |file, request| match files[file].requests[request as usize].specifier.as_ref() {
                    "./lib" => Some(0),
                    "./barrel" => Some(1),
                    _ => None,
                },
            );
            for needle in needles {
                assert!(
                    described[2]
                        .named
                        .contains_key(&site_of(&files[2], main, needle)),
                    "{described:?}"
                );
            }
        }
    }

    #[test]
    fn an_exported_object_keeps_its_fields_but_an_escaped_object_opens_them() {
        let source = "export const api = {run() {}}; api.run();";
        let flows = lower(source);
        let described = describe(&flows, |_| true);
        assert!(described.escaped_objects.is_empty());
        assert!(
            described
                .named
                .contains_key(&site_of(&flows, source, "api.run()"))
        );
        let source = "const api = {run() {}}; unknown(api); api.run();";
        let flows = lower(source);
        let described = describe(&flows, |_| true);
        assert!(
            !described
                .named
                .contains_key(&site_of(&flows, source, "api.run()"))
        );
    }

    #[test]
    fn prototype_lookup_respects_shadowing_and_opens_unknown_chains_and_cycles() {
        let source =
            "const p = {run() {}}; const o = Object.create(p); o.run = function own() {}; o.run();";
        let flows = lower(source);
        let described = describe(&flows, |_| true);
        assert!(
            described
                .named
                .contains_key(&site_of(&flows, source, "o.run()"))
        );
        assert!(described.lookups.is_empty());
        for source in [
            "const o = Object.create(unknown); o.run();",
            "const o = {}; Object.setPrototypeOf(o, o); o.run();",
        ] {
            let flows = lower(source);
            let described = describe(&flows, |_| true);
            assert!(
                !described
                    .named
                    .contains_key(&site_of(&flows, source, "o.run()"))
            );
        }
    }

    /// The functions whose objects reach `var` through copies.
    fn functions_reaching(flows: &TsValueFlows, var: FlowVar) -> BTreeSet<FlowSpan> {
        let mut seen = BTreeSet::from([var]);
        let mut work = vec![var];
        let mut functions = BTreeSet::new();
        while let Some(var) = work.pop() {
            if let FlowVar::Function(span) = var {
                functions.insert(span);
            }
            for row in &flows.rows {
                if let FlowRow::Copy { dst, src } = row
                    && *dst == var
                    && seen.insert(*src)
                {
                    work.push(*src);
                }
            }
        }
        functions
    }

    #[test]
    fn the_left_operand_of_and_passes_no_function() {
        let source = "function a() {}\nfunction b() {}\nconst v = a && b;\nv();\n";
        let flows = lower(source);
        let site = site_of(&flows, source, "v()");
        let call = flows
            .calls
            .iter()
            .find(|call| call.site == site)
            .expect("call");
        let reaching = functions_reaching(&flows, call.callee);
        let start =
            |needle: &str| u32::try_from(source.find(needle).expect("needle")).expect("offset");
        assert!(reaching.iter().any(|span| span.0 == start("function b")));
        assert!(!reaching.iter().any(|span| span.0 == start("function a")));
    }

    #[test]
    fn a_local_closure_call_is_named() {
        let source =
            "function outer() {\n  const inner = () => 1;\n  return inner();\n}\nouter();\n";
        let flows = lower(source);
        let described = describe(&flows, |_| true);
        assert!(
            described
                .named
                .contains_key(&site_of(&flows, source, "inner()"))
        );
    }

    #[test]
    fn with_and_an_eval_that_may_name_the_module_wrapper_stop_the_walk() {
        assert!(!lower("with (o) { f(); }\n").lowered);
        assert!(!lower("function g() { eval('exports.f = 1'); }\n").lowered);
        assert!(!lower("function g(code) { eval(code); }\n").lowered);
        assert!(lower("function f() {}\neval('f()');\n").lowered);
        assert!(lower("const e = eval;\ne('1');\n").lowered);
    }

    #[test]
    fn a_direct_eval_opens_only_the_bindings_its_code_names() {
        let source = "function a() {}\nfunction b() {}\nlet f = a;\nlet g = b;\neval('f = 1');\nf();\ng();\n";
        let flows = lower(source);
        assert!(flows.lowered);
        let described = describe(&flows, |_| true);
        assert!(
            !described
                .named
                .contains_key(&site_of(&flows, source, "f()"))
        );
        assert!(
            described
                .named
                .contains_key(&site_of(&flows, source, "g()"))
        );
    }

    #[test]
    fn a_function_iterated_or_formatted_escapes() {
        let source = "function f(a) { a(); }\nfor (const x of [f]) {}\n`${f}`;\n";
        let flows = lower(source);
        let described = describe(&flows, |_| true);
        assert!(described.escaped_functions.contains(&(0, 22)));
        assert!(
            !described
                .named
                .contains_key(&site_of(&flows, source, "a()"))
        );
    }

    #[test]
    fn unknown_computed_reads_open_callbacks_reachable_from_the_base() {
        let source = "function a() {} function invoke(fn) { fn(); } const table = {run: invoke}; const value = table[unknownKey]; invoke(a); value();";
        let flows = lower(source);
        let described = describe(&flows, |_| true);
        let invoke = flows
            .functions
            .iter()
            .find(|function| source[function.span.0 as usize..].starts_with("function invoke"))
            .expect("callback")
            .span;
        assert!(described.open.contains(&FlowVar::Param(invoke, 0)));
        assert!(
            !described
                .named
                .contains_key(&site_of(&flows, source, "value()"))
        );
    }

    #[test]
    fn an_exported_function_has_open_parameters() {
        let source = "export function run(task) { task(); }\n";
        let flows = lower(source);
        let described = describe(&flows, |_| true);
        let run = flows.functions[0].span;
        // Exported functions accept outside arguments without making their
        // own properties writable by unknown code.
        assert!(!described.escaped_functions.contains(&run));
        assert!(described.open.contains(&FlowVar::Param(run, 0)));
        assert!(
            !described
                .named
                .contains_key(&site_of(&flows, source, "task()"))
        );
    }

    #[test]
    fn a_method_called_on_its_literal_reads_this() {
        let source = "const o = {\n  run() { return this.clone(); },\n  clone() { return 1; },\n};\no.run();\n";
        let flows = lower(source);
        let described = describe(&flows, |_| true);
        assert!(
            described
                .named
                .contains_key(&site_of(&flows, source, "this.clone()"))
        );
        assert!(
            described
                .named
                .contains_key(&site_of(&flows, source, "o.run()"))
        );
    }

    #[test]
    fn a_missing_argument_leaves_the_parameter_undefined() {
        let source = "function f(cb) { if (cb) cb(); }\nfunction g() {}\nf();\nf(g);\n";
        let flows = lower(source);
        let described = describe(&flows, |_| true);
        assert!(
            described
                .named
                .contains_key(&site_of(&flows, source, "cb()"))
        );
    }

    #[test]
    fn reading_arguments_opens_the_parameters() {
        let source = "function f(cb) { g(arguments); cb(); }\nfunction h() {}\nf(h);\n";
        let flows = lower(source);
        let described = describe(&flows, |_| true);
        assert!(
            !described
                .named
                .contains_key(&site_of(&flows, source, "cb()"))
        );
    }

    #[test]
    fn a_loop_overwriting_a_parameter_leaves_its_call_open() {
        let source = "function a() {} function apply(fn, fns) { for (var fn of fns) {} return fn(); } apply(a, [lookup()]);";
        let flows = lower(source);
        let described = describe(&flows, |_| true);
        assert!(
            !described
                .named
                .contains_key(&site_of(&flows, source, "fn()")),
            "{flows:?}\n{described:?}"
        );
    }

    #[test]
    fn a_prototype_assigned_through_proto_escapes_its_methods() {
        let source = "function m() {}\nconst p = { m };\nconst o = {};\no.__proto__ = p;\no.m();\n";
        let flows = lower(source);
        let described = describe(&flows, |_| true);
        assert!(
            !described
                .named
                .contains_key(&site_of(&flows, source, "o.m()"))
        );
    }
}
