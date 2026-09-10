// This is the whole policy for the go-guard-outcomes example repo. It registers
// one local rule, local/guard-outcomes, which reports one outcome per protected
// operation: covered, violation, or unknown. Reporting every outcome — including
// the proved ones — is the point of the query: an empty diagnostic list is not a
// proof, so this rule never lets a proved operation be silent.
use polint::sdk::prelude::*;

const GUARD: &str = "CheckAccess";
const OPERATION: &str = "SaveRecord";

#[polint::rule(
    id = "local/guard-outcomes",
    description = "Report the guard outcome of every protected operation.",
    severity = "error"
)]
pub(crate) fn guard_outcomes(ctx: &mut RuleCtx<'_>, control: ControlFlow<'_>) -> RuleResult {
    let rule_id = ctx.rule_id().to_string();
    let mut query = GuardQuery::new(
        EventPattern::call(OPERATION),
        GuardPattern::call_any([GUARD]),
    );
    query.require_checked_error = true;

    let mut diagnostics = Vec::new();
    for result in control.guard_outcomes(query) {
        if !file_in_scope(ctx.options(), result.file()) {
            continue;
        }
        diagnostics.push(outcome_diagnostic(&rule_id, &result));
    }

    for diagnostic in diagnostics {
        ctx.report(diagnostic);
    }
    Ok(())
}

fn outcome_diagnostic(rule_id: &str, result: &PolicyResult) -> Diagnostic {
    match result.outcome() {
        PolicyOutcome::Covered => covered_diagnostic(rule_id, result),
        PolicyOutcome::Violation => result
            .diagnostic(rule_id, "Guard outcome: violation.")
            .unwrap_or_else(|| covered_diagnostic(rule_id, result)),
        _ => unknown_diagnostic(rule_id, result),
    }
}

/// Covered results carry no engine diagnostic, so the rule builds its own
/// informational one to keep the proved cases visible in the report.
fn covered_diagnostic(rule_id: &str, result: &PolicyResult) -> Diagnostic {
    Diagnostic::info(
        rule_id.to_string(),
        result.file().to_string(),
        result.range(),
        "Guard outcome: covered.",
    )
    .with_evidence("policy_outcome", "covered")
    .with_help("Coverage is control-flow only; the guard's argument identity is not checked.")
}

/// An undecided operation is a warning, not an error: the engine examined it and
/// could not prove or refute the contract.
fn unknown_diagnostic(rule_id: &str, result: &PolicyResult) -> Diagnostic {
    let mut diagnostic = result
        .diagnostic(rule_id, "Guard outcome: unknown.")
        .unwrap_or_else(|| covered_diagnostic(rule_id, result));
    diagnostic.severity = Severity::Warn;
    diagnostic
}
