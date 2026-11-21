//! Holds every exact edge to the way its call site invokes.
//!
//! A call site either calls its callee or constructs with it (`new`,
//! `super()`), and a function admits a call, a construction, both or neither:
//! calling a class or constructing an arrow, a method, an async function or a
//! generator throws a `TypeError` before the callee runs. The frontend states
//! both sides as invocation constraints of the semantic graph. An exact edge
//! whose site and target do not admit each other, or where either side is not
//! stated, moves to the conservative tier with `invocation_unproven`: it is a
//! weaker claim, never a deleted one.

use std::collections::BTreeMap;

use super::facts::{RefinedCallConfidence, RefinedCallEdgeFact};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::{CallPrecision, CallTargetStatus, ClaimTier};
use crate::analysis_neutral::semantic_graph::constraints::invocations;
use crate::analysis_neutral::semantic_graph::facts::NodeKind;
use crate::internal_core::Language;

/// Evidence on an exact edge whose invocation the frontend does not show to
/// run the target.
pub(crate) const INVOCATION_UNPROVEN: &str = "invocation_unproven";

/// Moves each exact TypeScript or JavaScript edge of `edges` whose site is not
/// shown to invoke its target in a way the target admits to the conservative
/// tier, and returns how many it moved.
pub(crate) fn hold_exact_edges_to_invocation(
    db: &impl AnalysisHost,
    edges: &mut [RefinedCallEdgeFact],
) -> usize {
    let exact = |edge: &RefinedCallEdgeFact| {
        matches!(edge.language, Language::TypeScript | Language::JavaScript)
            && edge.status == CallTargetStatus::Resolved
            && edge.precision.tier() == ClaimTier::Exact
            && edge.target_function.is_some()
    };
    if !edges.iter().any(exact) {
        return 0;
    }
    let invocations = invocations(db.semantic_constraints());
    let mut sites = BTreeMap::new();
    let mut functions = BTreeMap::new();
    for node in db.semantic_nodes() {
        match node.kind {
            NodeKind::Callsite(site) => {
                sites.insert(site, node.id);
            }
            NodeKind::Function(function) => {
                functions.insert(function, node.id);
            }
            _ => {}
        }
    }
    let mut moved = 0;
    for edge in edges.iter_mut().filter(|edge| exact(edge)) {
        let site = sites.get(&edge.site).and_then(|node| invocations.get(node));
        let target = edge
            .target_function
            .and_then(|function| functions.get(&function))
            .and_then(|node| invocations.get(node));
        if let (Some(&site), Some(target)) = (site, target)
            && target.admits(site)
        {
            continue;
        }
        edge.precision = CallPrecision::Conservative;
        edge.assumptions.clear();
        edge.confidence = edge.confidence.weaker(RefinedCallConfidence::Medium);
        edge.evidence.push(INVOCATION_UNPROVEN.to_string());
        moved += 1;
    }
    moved
}
