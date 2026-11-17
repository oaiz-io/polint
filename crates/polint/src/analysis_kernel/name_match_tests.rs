//! A call answered only by a name match has no complete answer, and every
//! layer says so: the call layer keeps the site unresolved, the refined layer
//! carries the unresolved row, and no edge derived from the name match claims
//! more than the name match does.

use crate::analysis_kernel::{AnalysisKernel, KernelInput, KernelOutput};
use crate::analysis_neutral::calls::facts::{
    CallCallee, CallSiteFact, CallTargetStatus, ClaimTier,
};
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::load_config;

/// `Parser.parse` is the property assigned a function expression; the only
/// function declared with that name is the caller itself.
const MEMBER_MATCHED_BY_NAME: &str = "\
var Parser = function Parser() {};
Parser.parse = function parse(input) { return input; };
function parse(input) {
  return Parser.parse(input);
}
export { parse };
";

/// `this.name()` is answered by points-to with a heuristic edge, which claims
/// no tier, while the base layer marks the site unresolved.
const METHOD_THROUGH_THIS: &str = "\
class Command {
  name() {
    return \"command\";
  }
  describe() {
    return this.name();
  }
}
module.exports = { Command };
";

const BASE: &[&str] = &["syntax", "call_graph", "symbols", "references"];
const REFINED: &[&str] = &[
    "syntax",
    "calls",
    "control_flow",
    "dataflow",
    "call_graph",
    "symbols",
    "references",
];

fn run(source: &str, capabilities: &[&str]) -> KernelOutput {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(temp.path().join("src")).expect("create src");
    std::fs::write(temp.path().join("src/index.js"), source).expect("write source");
    let loaded = load_config(temp.path()).expect("default config loads");
    let plan = AnalysisPlan::from_capability_names_for_test(capabilities);
    AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &Cache::new("", false),
        config_digest: "config",
        rule_digest: "rules",
        plan: &plan,
        parallel: false,
    })
    .expect("kernel should run")
}

fn member_site<'db>(output: &'db KernelOutput, property: &str) -> &'db CallSiteFact {
    output
        .db
        .call_sites()
        .iter()
        .find(|site| {
            matches!(&site.callee, CallCallee::Member { property: name, .. } if name == property)
        })
        .expect("the member call is a call site")
}

fn claims(tier: ClaimTier) -> bool {
    matches!(tier, ClaimTier::Exact | ClaimTier::Conservative)
}

#[test]
fn a_member_call_matched_only_by_property_name_stays_unresolved() {
    for capabilities in [BASE, REFINED] {
        let output = run(MEMBER_MATCHED_BY_NAME, capabilities);
        let site = member_site(&output, "parse");

        for target in output
            .db
            .call_targets()
            .iter()
            .filter(|t| t.site == site.id)
        {
            assert!(
                !claims(target.precision.tier()),
                "{capabilities:?}: a name match claims {:?}",
                target.precision
            );
        }
        assert_ne!(
            site.status,
            CallTargetStatus::Resolved,
            "{capabilities:?}: the site claims a complete answer"
        );
        assert!(
            output
                .db
                .unresolved_calls()
                .iter()
                .any(|row| row.site == site.id),
            "{capabilities:?}: the site has no unresolved row"
        );
    }
}

#[test]
fn the_refined_answer_for_a_name_matched_call_keeps_its_unresolved_row() {
    let output = run(MEMBER_MATCHED_BY_NAME, REFINED);
    let site = member_site(&output, "parse");
    let refined = output
        .db
        .refined_call_edges()
        .iter()
        .filter(|edge| edge.site == site.id)
        .collect::<Vec<_>>();

    assert!(!refined.is_empty(), "the refined layer answers the site");
    for edge in &refined {
        assert!(
            !(edge.status == CallTargetStatus::Resolved && claims(edge.precision.tier())),
            "refined {:?} edge claims {:?}",
            edge.tier,
            edge.precision
        );
    }
    assert!(
        refined
            .iter()
            .any(|edge| edge.status != CallTargetStatus::Resolved),
        "the refined answer has no unresolved row"
    );
}

#[test]
fn a_site_whose_refined_edges_claim_nothing_keeps_its_unresolved_row() {
    let output = run(METHOD_THROUGH_THIS, REFINED);
    let site = member_site(&output, "name");
    assert!(
        output
            .db
            .unresolved_calls()
            .iter()
            .any(|row| row.site == site.id),
        "the base layer marks the site unresolved"
    );
    let refined = output
        .db
        .refined_call_edges()
        .iter()
        .filter(|edge| edge.site == site.id)
        .collect::<Vec<_>>();

    assert!(
        refined
            .iter()
            .any(|edge| edge.status == CallTargetStatus::Resolved),
        "the refined layer resolves an edge at the site"
    );
    assert!(
        refined.iter().all(|edge| !claims(edge.precision.tier())),
        "a refined edge claims a tier"
    );
    assert!(
        refined
            .iter()
            .any(|edge| edge.status != CallTargetStatus::Resolved),
        "the refined answer has no unresolved row"
    );
}
