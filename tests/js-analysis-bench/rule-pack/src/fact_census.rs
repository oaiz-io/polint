// The one rule the JS/TS analysis benchmark runs when it measures cost.
//
// It requests the fact views a call-graph policy would read (functions, the
// call graph, symbols and references) so a measured check pays for building
// them, and reports their sizes as one note. The benchmark reads accuracy from
// a separate fact dump; this rule exists only to make the measured work real.
use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/js-bench-fact-census",
    description = "Count the JS/TS call facts a call-graph policy would read.",
    severity = "info"
)]
pub(crate) fn fact_census(
    ctx: &mut RuleCtx<'_>,
    functions: Functions<'_>,
    graph: CallGraph<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
) -> RuleResult {
    let Some(first) = functions.iter().min_by_key(|function| function.id) else {
        return Ok(());
    };
    let edges = graph.edges().count();
    let exported = symbols.iter().filter(|symbol| symbol.is_exported).count();
    let unresolved = references.unresolved().count();
    let diagnostic = Diagnostic::info(
        ctx.rule_id(),
        ctx.file_path(first.file),
        first.span.diagnostic_range(),
        format!(
            "{} functions, {edges} call graph edges",
            functions.iter().count()
        ),
    )
    .with_evidence("call_graph_edges", edges.to_string())
    .with_evidence("exported_symbols", exported.to_string())
    .with_evidence("references", references.iter().count().to_string())
    .with_evidence("unresolved_references", unresolved.to_string());
    ctx.report(diagnostic);
    Ok(())
}
