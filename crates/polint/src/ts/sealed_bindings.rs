//! Where the bindings Oxc reports do not decide what a name holds.
//!
//! Oxc resolves every reference lexically. Three constructs of non-strict
//! JavaScript change a binding, or what a name reads, without a reference Oxc
//! sees: a `with` body reads and writes the properties of its object first, a
//! direct `eval` can write a binding its call can see or declare a `var` that
//! shadows one, and a function declaration in a nested block also writes the
//! same-named binding of the enclosing function scope. Every producer that
//! claims a binding holds one value asks these questions here.

use std::collections::BTreeSet;
use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast::ast::{Argument, CallExpression, Expression, Program, Statement};
use oxc_semantic::{
    AstNodes, ReferenceId, ScopeId, Scoping, SemanticBuilder, SymbolId as OxcSymbolId,
};
use oxc_span::Span;

use crate::ts::parse::parse_ts_source;

/// What the file's `with` statements and direct `eval` calls can reach.
///
/// A `with` body can read an object property for any name inside it, so a file
/// with one has no sealed binding. A direct eval runs its code in the scope of
/// its call: it can read and write the bindings of that scope and of every
/// scope around it, and in non-strict code a `var` or function it declares
/// joins the nearest enclosing function scope, where it shadows a global the
/// code there names. Bindings of scopes the call is not inside stay sealed.
///
/// Code that is a string literal is parsed: it writes only the names it
/// assigns or declares, and exposes the values of the names it reads. Any
/// other code may name every binding the call can see, and may
/// read `this`, `arguments` and the function or class around the call. Any
/// call through an identifier named `eval` counts, since a parameter or local
/// of that name may hold the global; a call of `eval` under any other name is
/// indirect and runs as global code, as another script would.
#[derive(Debug, Default)]
pub(crate) struct DynamicScope {
    /// A `with` statement, or an eval that can name the module wrapper's
    /// `require`, `module` or `exports`, or `this` at the top of a CommonJS file.
    whole_file: bool,
    /// Bindings an eval can write, or shadow for some of their references.
    bindings: BTreeSet<OxcSymbolId>,
    /// Bindings whose value an eval can read.
    exposed: BTreeSet<OxcSymbolId>,
    /// References to globals a non-strict eval may declare a `var` over.
    shadowed: BTreeSet<ReferenceId>,
    /// Eval calls whose code may read the context of the function around them.
    open_contexts: BTreeSet<Span>,
}

impl DynamicScope {
    /// Whether no binding of the file can be claimed to hold one value.
    pub(crate) fn whole_file(&self) -> bool {
        self.whole_file
    }

    /// Whether an eval or `with` statement may write or shadow `symbol`.
    pub(crate) fn reaches(&self, symbol: OxcSymbolId) -> bool {
        self.whole_file || self.bindings.contains(&symbol)
    }

    /// The bindings an eval may write or shadow, when not the whole file.
    pub(crate) fn bindings(&self) -> &BTreeSet<OxcSymbolId> {
        &self.bindings
    }

    /// The bindings whose value an eval may read, when not the whole file.
    pub(crate) fn exposed(&self) -> &BTreeSet<OxcSymbolId> {
        &self.exposed
    }

    /// Whether a non-strict eval may declare a `var` the unresolved
    /// `reference` reads instead of the global.
    pub(crate) fn shadows(&self, reference: ReferenceId) -> bool {
        self.whole_file || self.shadowed.contains(&reference)
    }

    /// Whether the eval call at `call` may read `this`, `arguments`, `super`
    /// or `new.target` of the function around it.
    pub(crate) fn opens_context(&self, call: Span) -> bool {
        self.open_contexts.contains(&call)
    }
}

/// The names eval code mentions, where its text is known.
enum Reach {
    Nothing,
    Names {
        named: BTreeSet<String>,
        written: BTreeSet<String>,
        declared: BTreeSet<String>,
    },
    Everything,
}

/// What the `with` statements and direct `eval` calls of a file reach.
pub(crate) fn dynamic_scope(
    program: &Program<'_>,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
) -> DynamicScope {
    let module = program.source_type.is_module()
        || program.body.iter().any(Statement::is_module_declaration);
    let mut scope = DynamicScope::default();
    for node in nodes.iter() {
        let call = match node.kind() {
            AstKind::WithStatement(_) => {
                return DynamicScope {
                    whole_file: true,
                    ..DynamicScope::default()
                };
            }
            AstKind::CallExpression(call) if is_direct_eval(call) => call,
            _ => continue,
        };
        let reach = eval_reach(call);
        let wrapper_named = match &reach {
            Reach::Nothing => false,
            Reach::Names { named, .. } => ["require", "module", "exports"]
                .iter()
                .any(|name| named.contains(*name)),
            Reach::Everything => true,
        };
        if wrapper_named && !module {
            return DynamicScope {
                whole_file: true,
                ..DynamicScope::default()
            };
        }
        let call_scope = node.scope_id();
        for ancestor in scoping.scope_ancestors(call_scope) {
            for symbol in scoping.iter_bindings_in(ancestor) {
                let name = scoping.symbol_name(symbol);
                let (written, read) = match &reach {
                    Reach::Nothing => (false, false),
                    Reach::Names {
                        named,
                        written,
                        declared,
                    } => (
                        written.contains(name) || declared.contains(name),
                        named.contains(name),
                    ),
                    Reach::Everything => (true, true),
                };
                if written {
                    scope.bindings.insert(symbol);
                }
                if read {
                    scope.exposed.insert(symbol);
                }
            }
        }
        if matches!(reach, Reach::Everything) {
            scope.open_contexts.insert(call.span);
        }
        if !scoping.scope_flags(call_scope).is_strict_mode() {
            shadow_globals(scoping, nodes, call_scope, &reach, &mut scope.shadowed);
        }
    }
    scope
}

/// A call of an identifier named `eval`, parenthesized or not.
pub(crate) fn is_direct_eval(call: &CallExpression<'_>) -> bool {
    matches!(call.callee.without_parentheses(), Expression::Identifier(identifier) if identifier.name == "eval")
}

/// The names the code of an eval call mentions. A call without an argument,
/// or whose first argument is a literal other than a string, runs no code.
fn eval_reach(call: &CallExpression<'_>) -> Reach {
    let text = match call.arguments.first() {
        None => return Reach::Nothing,
        Some(Argument::StringLiteral(literal)) => literal.value.as_str(),
        Some(Argument::TemplateLiteral(template)) if template.expressions.is_empty() => {
            match template
                .quasis
                .first()
                .and_then(|quasi| quasi.value.cooked.as_ref())
            {
                Some(cooked) => cooked.as_str(),
                None => return Reach::Everything,
            }
        }
        Some(
            Argument::NumericLiteral(_)
            | Argument::BooleanLiteral(_)
            | Argument::NullLiteral(_)
            | Argument::BigIntLiteral(_)
            | Argument::RegExpLiteral(_),
        ) => return Reach::Nothing,
        Some(_) => return Reach::Everything,
    };
    let allocator = Allocator::default();
    let parsed = parse_ts_source(&allocator, Path::new("eval.cjs"), text);
    if !parsed.fully_parsed {
        return Reach::Everything;
    }
    let semantic = SemanticBuilder::new().build(parsed.program()).semantic;
    let mut named = BTreeSet::new();
    let mut written = BTreeSet::new();
    let mut declared = BTreeSet::new();
    for node in semantic.nodes().iter() {
        match node.kind() {
            AstKind::IdentifierReference(identifier) => {
                if matches!(identifier.name.as_str(), "eval" | "arguments") {
                    return Reach::Everything;
                }
                let write = identifier
                    .reference_id
                    .get()
                    .is_none_or(|id| semantic.scoping().get_reference(id).is_write());
                if write {
                    written.insert(identifier.name.to_string());
                }
                named.insert(identifier.name.to_string());
            }
            AstKind::BindingIdentifier(identifier) => {
                named.insert(identifier.name.to_string());
                declared.insert(identifier.name.to_string());
            }
            AstKind::ThisExpression(_)
            | AstKind::Super(_)
            | AstKind::MetaProperty(_)
            | AstKind::WithStatement(_) => return Reach::Everything,
            _ => {}
        }
    }
    Reach::Names {
        named,
        written,
        declared,
    }
}

/// Records the unresolved references a `var` of non-strict eval code at
/// `call_scope` may shadow: those inside the nearest enclosing function
/// scope, which the eval's declarations join, whose name the code declares.
fn shadow_globals(
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    call_scope: ScopeId,
    reach: &Reach,
    shadowed: &mut BTreeSet<ReferenceId>,
) {
    let declared = match reach {
        Reach::Nothing => return,
        Reach::Names { declared, .. } if declared.is_empty() => return,
        Reach::Names { declared, .. } => Some(declared),
        Reach::Everything => None,
    };
    let Some(var_scope) = scoping
        .scope_ancestors(call_scope)
        .find(|ancestor| scoping.scope_flags(*ancestor).is_var())
    else {
        return;
    };
    for node in nodes.iter() {
        let AstKind::IdentifierReference(identifier) = node.kind() else {
            continue;
        };
        let Some(reference) = identifier.reference_id.get() else {
            continue;
        };
        if scoping.get_reference(reference).symbol_id().is_some()
            || declared.is_some_and(|declared| !declared.contains(identifier.name.as_str()))
        {
            continue;
        }
        if scoping
            .scope_ancestors(node.scope_id())
            .any(|ancestor| ancestor == var_scope)
        {
            shadowed.insert(reference);
        }
    }
}

/// Bindings whose value a function declaration in a nested block of non-strict
/// code decides in a way no Oxc reference shows.
///
/// When such a block runs, its function is also assigned to the same-named
/// binding of the enclosing function scope. Oxc keeps the block's function a
/// separate symbol when that scope already binds the name, so the outer symbol
/// is written without a write reference. Otherwise Oxc moves the block's
/// function into the function scope, which is wrong when a scope between the
/// two binds the name: the function then stays in its block.
pub(crate) fn bindings_written_by_block_functions(
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
) -> BTreeSet<OxcSymbolId> {
    let mut written = BTreeSet::new();
    for node in nodes.iter() {
        let AstKind::Function(function) = node.kind() else {
            continue;
        };
        let Some(id) = &function.id else {
            continue;
        };
        let Some(symbol) = id.symbol_id.get() else {
            continue;
        };
        let scope = scoping.symbol_scope_id(symbol);
        if scoping.scope_flags(scope).is_strict_mode() {
            continue;
        }
        let outer = scoping
            .scope_ancestors(scope)
            .find(|ancestor| scoping.scope_flags(*ancestor).is_var())
            .and_then(|function_scope| scoping.get_binding(function_scope, id.name));
        if let Some(outer) = outer.filter(|outer| *outer != symbol) {
            written.insert(outer);
        }
        let declared_in = node.scope_id();
        if function.is_declaration()
            && scoping
                .scope_ancestors(declared_in)
                .take_while(|ancestor| *ancestor != scope)
                .any(|between| scoping.get_binding(between, id.name).is_some())
        {
            written.insert(symbol);
        }
    }
    written
}
