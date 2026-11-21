//! Where the bindings Oxc reports do not decide what a name holds.
//!
//! Oxc resolves every reference lexically. Three constructs of non-strict
//! JavaScript change a binding, or what a name reads, without a reference Oxc
//! sees: a `with` body reads and writes the properties of its object first, a
//! direct `eval` can write a binding its call can see or declare a `var` that
//! shadows one, and a function declaration in a nested block also writes the
//! same-named binding of the enclosing function scope. Every producer that
//! claims a binding holds one value asks these questions here.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast::ast::{Argument, BindingIdentifier, CallExpression, Expression, Program, Statement};
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
/// read `this`, `arguments` and the function or class around the call. In a
/// CommonJS file the call can also see the parameters of the module wrapper,
/// `require`, `module` and `exports`, and `this` at the top of the file is
/// `module.exports`: code that may name them may rewrite what the file
/// exports and what any of its `require` calls return, while the bindings of
/// scopes the call is not inside stay sealed. Any
/// call through an identifier named `eval` counts, since a parameter or local
/// of that name may hold the global; a call of `eval` under any other name is
/// indirect and runs as global code, as another script would.
#[derive(Debug, Default)]
pub(crate) struct DynamicScope {
    /// A `with` statement.
    whole_file: bool,
    /// An eval that can name the module wrapper's `require`, `module` or
    /// `exports`, or `this` at the top of a CommonJS file.
    wrapper: bool,
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

    /// Whether an eval of a CommonJS file may rewrite `module`, `exports` or
    /// `require`, so neither what the file exports nor what one of its
    /// `require` calls returns is decided by the syntax.
    pub(crate) fn opens_wrapper(&self) -> bool {
        self.whole_file || self.wrapper
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
    let mut calls = Vec::new();
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
        scope.wrapper |= wrapper_named && !module;
        if matches!(reach, Reach::Everything) {
            scope.open_contexts.insert(call.span);
        }
        calls.push(EvalCall {
            scope: node.scope_id(),
            reach,
        });
    }
    if calls.is_empty() {
        return scope;
    }
    let order = ScopeOrder::new(scoping);
    reached_bindings(scoping, &order, &calls, &mut scope);
    shadow_globals(scoping, nodes, &order, &calls, &mut scope.shadowed);
    scope
}

/// A call of an identifier named `eval`, behind any parentheses and any type
/// wrapper: `as`, `satisfies`, `<T>`, `!` and type arguments are erased before
/// the code runs, so the call stays a direct `eval`.
pub(crate) fn is_direct_eval(call: &CallExpression<'_>) -> bool {
    matches!(call.callee.get_inner_expression(), Expression::Identifier(identifier) if identifier.name == "eval")
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

/// A direct eval call: the scope it runs in and the names its code mentions.
struct EvalCall {
    scope: ScopeId,
    reach: Reach,
}

/// The scopes of a file in preorder, so that whether one scope lies inside
/// another is two comparisons rather than a walk up the scopes between them.
struct ScopeOrder {
    /// The scopes, each before the scopes inside it.
    preorder: Vec<ScopeId>,
    /// The position of each scope in `preorder`.
    position: Vec<usize>,
    /// One past the position of the last scope inside each scope.
    end: Vec<usize>,
    /// The nearest function, class static block, namespace or top-level
    /// scope at or around each scope: the one its `var` declarations join.
    var_scope: Vec<Option<ScopeId>>,
}

impl ScopeOrder {
    fn new(scoping: &Scoping) -> Self {
        let len = scoping.scopes_len();
        let mut children = vec![Vec::new(); len];
        let mut roots = Vec::new();
        for scope in scoping.scope_descendants_from_root() {
            match scoping.scope_parent_id(scope) {
                Some(parent) => children[parent.index()].push(scope),
                None => roots.push(scope),
            }
        }
        let mut order = Self {
            preorder: Vec::with_capacity(len),
            position: vec![0; len],
            end: vec![0; len],
            var_scope: vec![None; len],
        };
        let mut stack: Vec<(ScopeId, bool)> =
            roots.iter().rev().map(|root| (*root, false)).collect();
        while let Some((scope, left)) = stack.pop() {
            if left {
                order.end[scope.index()] = order.preorder.len();
                continue;
            }
            order.position[scope.index()] = order.preorder.len();
            order.preorder.push(scope);
            order.var_scope[scope.index()] = if scoping.scope_flags(scope).is_var() {
                Some(scope)
            } else {
                scoping
                    .scope_parent_id(scope)
                    .and_then(|parent| order.var_scope[parent.index()])
            };
            stack.push((scope, true));
            stack.extend(
                children[scope.index()]
                    .iter()
                    .rev()
                    .map(|child| (*child, false)),
            );
        }
        order
    }

    fn position(&self, scope: ScopeId) -> usize {
        self.position[scope.index()]
    }

    /// The positions of `scope` and of every scope inside it.
    fn subtree(&self, scope: ScopeId) -> Range<usize> {
        self.position[scope.index()]..self.end[scope.index()]
    }

    fn var_scope(&self, scope: ScopeId) -> Option<ScopeId> {
        self.var_scope[scope.index()]
    }
}

/// Whether one of the sorted `positions` lies in `range`.
fn any_within(positions: &[usize], range: &Range<usize>) -> bool {
    let first = positions.partition_point(|position| *position < range.start);
    positions
        .get(first)
        .is_some_and(|position| *position < range.end)
}

/// The union of some subtrees of the scope tree, as disjoint sorted ranges.
/// Two subtrees are nested or apart, so a range that starts inside the last
/// one kept lies inside it.
struct Subtrees(Vec<Range<usize>>);

impl Subtrees {
    fn new(mut ranges: Vec<Range<usize>>) -> Self {
        ranges.sort_by_key(|range| range.start);
        let mut union: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            if union.last().is_none_or(|last| range.start >= last.end) {
                union.push(range);
            }
        }
        Self(union)
    }

    fn contains(&self, position: usize) -> bool {
        let after = self.0.partition_point(|range| range.start <= position);
        after > 0 && position < self.0[after - 1].end
    }
}

/// Records the bindings each eval call can see whose names its code writes,
/// declares or reads: those of the scope of the call and of every scope
/// around it. A binding is reached when an eval call lies in its scope.
fn reached_bindings(
    scoping: &Scoping,
    order: &ScopeOrder,
    calls: &[EvalCall],
    scope: &mut DynamicScope,
) {
    let mut everything = Vec::new();
    let mut writes: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut reads: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for call in calls {
        let at = order.position(call.scope);
        match &call.reach {
            Reach::Nothing => {}
            Reach::Names {
                named,
                written,
                declared,
            } => {
                for name in written.union(declared) {
                    writes.entry(name.as_str()).or_default().push(at);
                }
                for name in named {
                    reads.entry(name.as_str()).or_default().push(at);
                }
            }
            Reach::Everything => everything.push(at),
        }
    }
    everything.sort_unstable();
    for positions in writes.values_mut().chain(reads.values_mut()) {
        positions.sort_unstable();
    }
    let reached = |positions: Option<&Vec<usize>>, range: &Range<usize>| {
        positions.is_some_and(|positions| any_within(positions, range))
    };
    for binding_scope in scoping.scope_descendants_from_root() {
        let range = order.subtree(binding_scope);
        let everything = any_within(&everything, &range);
        for symbol in scoping.iter_bindings_in(binding_scope) {
            let name = scoping.symbol_name(symbol);
            if everything || reached(writes.get(name), &range) {
                scope.bindings.insert(symbol);
            }
            if everything || reached(reads.get(name), &range) {
                scope.exposed.insert(symbol);
            }
        }
    }
}

/// Records the unresolved references a `var` of non-strict eval code may
/// shadow: those inside the nearest enclosing function scope of the call,
/// which the eval's declarations join, whose name the code declares.
fn shadow_globals(
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    order: &ScopeOrder,
    calls: &[EvalCall],
    shadowed: &mut BTreeSet<ReferenceId>,
) {
    let mut everything = Vec::new();
    let mut declaring: BTreeMap<&str, Vec<Range<usize>>> = BTreeMap::new();
    for call in calls {
        if scoping.scope_flags(call.scope).is_strict_mode() {
            continue;
        }
        let Some(var_scope) = order.var_scope(call.scope) else {
            continue;
        };
        match &call.reach {
            Reach::Nothing => {}
            Reach::Names { declared, .. } => {
                for name in declared {
                    declaring
                        .entry(name.as_str())
                        .or_default()
                        .push(order.subtree(var_scope));
                }
            }
            Reach::Everything => everything.push(order.subtree(var_scope)),
        }
    }
    if everything.is_empty() && declaring.is_empty() {
        return;
    }
    let everything = Subtrees::new(everything);
    let declaring: BTreeMap<&str, Subtrees> = declaring
        .into_iter()
        .map(|(name, ranges)| (name, Subtrees::new(ranges)))
        .collect();
    for node in nodes.iter() {
        let AstKind::IdentifierReference(identifier) = node.kind() else {
            continue;
        };
        let Some(reference) = identifier.reference_id.get() else {
            continue;
        };
        if scoping.get_reference(reference).symbol_id().is_some() {
            continue;
        }
        let at = order.position(node.scope_id());
        if everything.contains(at)
            || declaring
                .get(identifier.name.as_str())
                .is_some_and(|subtrees| subtrees.contains(at))
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
    let mut functions = Vec::new();
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
        functions.push(BlockFunction {
            id,
            symbol,
            scope,
            declared_in: node.scope_id(),
            declaration: function.is_declaration(),
        });
    }
    if functions.is_empty() {
        return BTreeSet::new();
    }
    let order = ScopeOrder::new(scoping);
    let mut written = BTreeSet::new();
    for function in &functions {
        let outer = order
            .var_scope(function.scope)
            .and_then(|function_scope| scoping.get_binding(function_scope, function.id.name));
        if let Some(outer) = outer.filter(|outer| *outer != function.symbol) {
            written.insert(outer);
        }
    }
    written.extend(declarations_bound_between(scoping, &order, &functions));
    written
}

/// A named function of non-strict code.
struct BlockFunction<'n, 'a> {
    id: &'n BindingIdentifier<'a>,
    symbol: OxcSymbolId,
    /// The scope Oxc binds the function's name in.
    scope: ScopeId,
    /// The scope the function is written in.
    declared_in: ScopeId,
    declaration: bool,
}

/// The function declarations whose name a scope binds from the one they are
/// written in up to, and not including, the one Oxc binds them in.
///
/// One sweep over the scopes in preorder keeps, for each such name, the
/// positions of the open scopes that bind it, so the innermost binder around
/// a declaration is the last one kept.
fn declarations_bound_between(
    scoping: &Scoping,
    order: &ScopeOrder,
    functions: &[BlockFunction<'_, '_>],
) -> Vec<OxcSymbolId> {
    let mut declarations: Vec<&BlockFunction<'_, '_>> = functions
        .iter()
        .filter(|function| function.declaration)
        .collect();
    declarations.sort_by_key(|function| order.position(function.declared_in));
    let mut binders: BTreeMap<&str, Vec<usize>> = declarations
        .iter()
        .map(|function| (scoping.symbol_name(function.symbol), Vec::new()))
        .collect();
    let mut bound = Vec::new();
    let mut open: Vec<ScopeId> = Vec::new();
    let mut next = declarations.iter().peekable();
    for &scope in &order.preorder {
        let at = order.position(scope);
        while let Some(&last) = open.last() {
            if order.subtree(last).end > at {
                break;
            }
            open.pop();
            for symbol in scoping.iter_bindings_in(last) {
                if let Some(positions) = binders.get_mut(scoping.symbol_name(symbol)) {
                    positions.pop();
                }
            }
        }
        open.push(scope);
        for symbol in scoping.iter_bindings_in(scope) {
            if let Some(positions) = binders.get_mut(scoping.symbol_name(symbol)) {
                positions.push(at);
            }
        }
        while let Some(function) = next.next_if(|function| function.declared_in == scope) {
            let innermost = binders
                .get(scoping.symbol_name(function.symbol))
                .and_then(|positions| positions.last());
            let between = if order.subtree(function.scope).contains(&at) {
                innermost.is_some_and(|binder| *binder > order.position(function.scope))
            } else {
                innermost.is_some()
            };
            if between {
                bound.push(function.symbol);
            }
        }
    }
    bound
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// Room for the parser and the semantic builder to nest a few thousand
    /// blocks in an unoptimised build.
    const STACK_BYTES: usize = 256 * 1024 * 1024;

    /// What a file's eval calls reach, how many block functions write a
    /// binding, the names of the shadowed references and how long the two
    /// questions took, without the parse.
    struct Answers {
        shadowed: Vec<String>,
        exposed: usize,
        written_by_block_functions: usize,
        elapsed: Duration,
    }

    fn answers(source: String) -> Answers {
        std::thread::Builder::new()
            .stack_size(STACK_BYTES)
            .spawn(move || {
                let allocator = Allocator::default();
                let parsed = parse_ts_source(&allocator, Path::new("deep.cjs"), &source);
                assert!(parsed.fully_parsed);
                let semantic = SemanticBuilder::new().build(parsed.program()).semantic;
                let started = Instant::now();
                let dynamic = dynamic_scope(parsed.program(), semantic.scoping(), semantic.nodes());
                let written =
                    bindings_written_by_block_functions(semantic.scoping(), semantic.nodes());
                let elapsed = started.elapsed();
                let shadowed = semantic
                    .nodes()
                    .iter()
                    .filter_map(|node| match node.kind() {
                        AstKind::IdentifierReference(identifier) => identifier
                            .reference_id
                            .get()
                            .filter(|reference| dynamic.shadows(*reference))
                            .map(|_| identifier.name.to_string()),
                        _ => None,
                    })
                    .collect();
                Answers {
                    shadowed,
                    exposed: dynamic.exposed().len(),
                    written_by_block_functions: written.len(),
                    elapsed,
                }
            })
            .expect("spawn a thread with room for deep nesting")
            .join()
            .expect("the deep file is analysed")
    }

    fn nested_blocks(depth: usize, inner: &str) -> String {
        let mut source = String::with_capacity(depth * 2 + inner.len());
        source.push_str(&"{".repeat(depth));
        source.push_str(inner);
        source.push_str(&"}".repeat(depth));
        source
    }

    /// The questions take time linear in the file: a walk from every reference
    /// or binding up to the scopes around it would take minutes here.
    const LINEAR_IN_THE_FILE: Duration = Duration::from_secs(5);

    #[test]
    fn an_eval_above_deeply_nested_references_shadows_them_in_time_linear_in_the_file() {
        let references = "u;\n".repeat(50_000);
        let mut source = String::from("eval(s);\n");
        source.push_str(&nested_blocks(8_000, &references));
        let answers = answers(source);
        let shadowed_u = answers.shadowed.iter().filter(|name| *name == "u").count();
        assert_eq!(
            shadowed_u, 50_000,
            "every reference to `u` may read a `var` of the eval"
        );
        assert!(
            answers.elapsed < LINEAR_IN_THE_FILE,
            "took {:?}",
            answers.elapsed
        );
    }

    #[test]
    fn many_evals_below_many_bindings_expose_them_in_time_linear_in_the_file() {
        let mut source = String::new();
        for depth in 0..4_000 {
            source.push_str(&format!("{{ let a{depth} = 0;\n"));
        }
        source.push_str(&"eval(s);\n".repeat(4_000));
        source.push_str(&"}".repeat(4_000));
        let answers = answers(source);
        assert_eq!(
            answers.exposed, 4_000,
            "the evals may read every binding around them"
        );
        assert!(
            answers.elapsed < LINEAR_IN_THE_FILE,
            "took {:?}",
            answers.elapsed
        );
    }

    #[test]
    fn functions_declared_in_deeply_nested_blocks_are_checked_in_time_linear_in_the_file() {
        let mut inner = String::new();
        for depth in 0..8_000 {
            inner.push_str(&format!("{{ function f{depth}() {{}} let g{depth} = 0;\n"));
        }
        inner.push_str("function g0() {}\n");
        inner.push_str(&"}".repeat(8_000));
        let source = format!("function outer() {{ var f0;\n{inner}}}\n");
        let answers = answers(source);
        // `f0` is written in the function scope by its block function, and
        // `g0` is bound by a block between the innermost `function g0` and
        // the function scope, so that function stays in its block.
        assert_eq!(answers.written_by_block_functions, 2);
        assert!(
            answers.elapsed < LINEAR_IN_THE_FILE,
            "took {:?}",
            answers.elapsed
        );
    }
}
