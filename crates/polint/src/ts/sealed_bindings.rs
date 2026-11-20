//! Where the bindings Oxc reports do not decide what a name holds.
//!
//! Oxc resolves every reference lexically. Three constructs of non-strict
//! JavaScript change a binding, or what a name reads, without a reference Oxc
//! sees: a `with` body reads and writes the properties of its object first, a
//! direct `eval` can write any visible binding or declare a `var` that shadows
//! one, and a function declaration in a nested block also writes the
//! same-named binding of the enclosing function scope. Every producer that
//! claims a binding holds one value asks these questions here.

use std::collections::BTreeSet;

use oxc_ast::AstKind;
use oxc_semantic::{AstNodes, Scoping, SymbolId as OxcSymbolId};

/// Whether the file has no `with` statement and no reference named `eval`.
///
/// Any call through an identifier named `eval` counts, since a parameter or
/// local of that name may hold the global.
pub(crate) fn file_bindings_are_sealed(nodes: &AstNodes<'_>) -> bool {
    !nodes.iter().any(|node| match node.kind() {
        AstKind::IdentifierReference(identifier) => identifier.name == "eval",
        AstKind::WithStatement(_) => true,
        _ => false,
    })
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
