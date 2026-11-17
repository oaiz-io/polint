use crate::analysis_neutral::AnalysisHost;
use std::cell::LazyCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::facts::{
    RefinedCallConfidence, RefinedCallEdgeFact, RefinedCallTier, RefinedCallValidation,
};
use super::store::RefinedCallOutput;
use crate::analysis_api::{FactFamily, stable_key_from_parts};
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallCallee, CallEdgeKind, CallProvenance, CallSiteFact, CallTargetStatus,
    UnresolvedCallFact, UnresolvedCallReason,
};
use crate::analysis_neutral::ids::{CallSiteId, PlaceId, RefinedCallEdgeId, SemanticNodeId};
use crate::analysis_neutral::points_to::facts::PointsToStatus;
use crate::analysis_neutral::semantic_graph::constraints::ConstraintKind;
use crate::analysis_neutral::semantic_graph::facts::NodeKind;
use crate::internal_core::StableKeyId;

pub fn derive_ts_js_refinements(db: &impl AnalysisHost) -> RefinedCallOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let sites = LazyCell::new(|| {
        let mut sites = HashMap::new();
        for site in db.call_sites() {
            if site.language.is_ts_family() {
                sites.entry(site.id).or_insert(site);
            }
        }
        sites
    });
    let index = LazyCell::new(|| SiteIndex::new(db));
    let edges = db
        .unresolved_calls()
        .iter()
        .filter_map(|unresolved| {
            let site = *sites.get(&unresolved.site)?;
            unresolved_edge(db, &index, site, unresolved)
        })
        .collect();
    RefinedCallOutput { edges }.normalized(interner)
}

/// The facts each unresolved call is matched against, indexed once per run so
/// matching stays linear in the number of facts.
struct SiteIndex {
    /// Call sites whose first callsite node carries a call constraint that a
    /// present solver edge was derived from.
    solver_resolved: HashSet<CallSiteId>,
    /// The stable key of the first place with each id.
    places: HashMap<PlaceId, StableKeyId>,
}

impl SiteIndex {
    fn new(db: &impl AnalysisHost) -> Self {
        #[cfg(test)]
        tests::REFINEMENT_INDEX_BUILDS.with(|count| count.set(count.get() + 1));
        let mut places = HashMap::new();
        for place in db.mir_places() {
            places.entry(place.id).or_insert(place.stable_key);
        }
        Self {
            solver_resolved: solver_resolved_sites(db),
            places,
        }
    }
}

/// Call sites the points-to solver resolved: a present solver edge derived
/// from a call constraint lists, among its contributing facts, a call
/// constraint on the site's callsite node. Keys are compared as text.
fn solver_resolved_sites(db: &impl AnalysisHost) -> HashSet<CallSiteId> {
    let contributing: HashSet<Arc<str>> = db
        .solver_derived_edges()
        .iter()
        .filter(|edge| {
            edge.status == PointsToStatus::Present
                && edge.provenance.constraint_kind == "call_constraint"
        })
        .flat_map(|edge| edge.provenance.contributing_facts.iter())
        .map(|fact| db.resolve_stable_key(fact.stable_key))
        .collect();
    if contributing.is_empty() {
        return HashSet::new();
    }
    let resolved_nodes: HashSet<SemanticNodeId> = db
        .semantic_constraints()
        .iter()
        .filter_map(|constraint| match constraint.kind {
            ConstraintKind::CallConstraint { callsite, .. }
                if contributing.contains(db.resolve_stable_key(constraint.stable_key).as_ref()) =>
            {
                Some(callsite)
            }
            _ => None,
        })
        .collect();
    let mut callsite_nodes: HashMap<CallSiteId, SemanticNodeId> = HashMap::new();
    for node in db.semantic_nodes() {
        if let NodeKind::Callsite(site) = node.kind {
            callsite_nodes.entry(site).or_insert(node.id);
        }
    }
    callsite_nodes
        .into_iter()
        .filter(|(_, node)| resolved_nodes.contains(node))
        .map(|(site, _)| site)
        .collect()
}

fn unresolved_edge(
    db: &impl AnalysisHost,
    index: &SiteIndex,
    site: &CallSiteFact,
    unresolved: &UnresolvedCallFact,
) -> Option<RefinedCallEdgeFact> {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    if index.solver_resolved.contains(&site.id) {
        return None;
    }
    let status = match unresolved.reason {
        UnresolvedCallReason::SetupMissing => CallTargetStatus::SetupMissing,
        UnresolvedCallReason::BudgetExceeded => CallTargetStatus::BudgetExceeded,
        UnresolvedCallReason::UnsupportedSyntax => CallTargetStatus::Unsupported,
        _ => CallTargetStatus::Unresolved,
    };
    let unresolved_key = db.resolve_stable_key(unresolved.stable_key).to_string();
    let callable_place_key = callable_place_stable_key(db, index, site);

    Some(RefinedCallEdgeFact {
        id: RefinedCallEdgeId(0),
        site: unresolved.site,
        base_target: None,
        caller: unresolved.caller,
        target_function: None,
        target_symbol: None,
        synthetic_target: callable_place_key
            .map(|place_key| format!("ts-js:callable-place:{place_key}")),
        language: site.language,
        edge_kind: CallEdgeKind::Unknown,
        algorithm: CallAlgorithm::PointsTo,
        tier: RefinedCallTier::PointsToAssisted,
        status,
        reason: Some(unresolved.reason),
        provenance: CallProvenance::Native,
        precision: unresolved.precision,
        validation: RefinedCallValidation::ReferentiallyValidated,
        confidence: RefinedCallConfidence::Low,
        evidence: vec!["ts_js_points_to_unresolved".to_string()],
        input_stable_keys: vec![unresolved_key.clone()],
        stable_key: stable_key_from_parts(
            interner,
            FactFamily::RefinedCallEdge,
            &[
                ("tier", "ts_js_points_to_unresolved".to_string()),
                ("unresolved", unresolved_key),
                ("status", format!("{status:?}")),
            ],
        ),
    })
}

fn callable_place_stable_key(
    db: &impl AnalysisHost,
    index: &SiteIndex,
    site: &CallSiteFact,
) -> Option<String> {
    let place = match site.callee {
        CallCallee::FunctionValue { place } => Some(place),
        _ => site.receiver,
    }?;
    index
        .places
        .get(&place)
        .map(|stable_key| db.resolve_stable_key(*stable_key).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallPrecision, CallSiteFact, CallSyntaxKind, UnresolvedCallFact,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::ids::{CallSiteId, MirBodyId, MirOpId, PlaceId};
    use crate::analysis_neutral::mir_body::MirOutput;
    use crate::analysis_neutral::places::{PlaceFact, PlaceRoot, PlaceStatus};
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    std::thread_local! {
        pub(super) static REFINEMENT_INDEX_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    #[test]
    fn empty_and_go_only_refinements_skip_expensive_indexes() {
        let mut resolved = refinement_db(Language::TypeScript);
        let sites = resolved.call_sites().to_vec();
        resolved
            .replace_call_facts(CallOutput {
                sites,
                ..CallOutput::default()
            })
            .expect("resolved calls");
        for db in [
            LocalAnalysisDb::new(),
            resolved,
            refinement_db(Language::Go),
        ] {
            REFINEMENT_INDEX_BUILDS.set(0);
            assert!(derive_ts_js_refinements(&db).edges.is_empty());
            assert_eq!(REFINEMENT_INDEX_BUILDS.get(), 0);
        }
    }

    #[test]
    #[ignore = "release microbenchmark: empty unresolved set and Go-only unresolved calls"]
    fn benchmark_empty_and_go_only_refinements() {
        for language in [Language::TypeScript, Language::Go] {
            let mut db = refinement_db(language);
            let template = db.call_sites()[0].clone();
            let sites = (0..20_000)
                .map(|id| CallSiteFact {
                    id: CallSiteId(id),
                    stable_key: db.stable_key_interner().intern(format!("site:{id}")),
                    ..template.clone()
                })
                .collect();
            let unresolved = if language == Language::Go {
                db.unresolved_calls().to_vec()
            } else {
                Vec::new()
            };
            db.replace_call_facts(CallOutput {
                sites,
                unresolved,
                ..CallOutput::default()
            })
            .expect("call facts");
            REFINEMENT_INDEX_BUILDS.set(0);
            let start = std::time::Instant::now();
            for _ in 0..100 {
                assert!(
                    std::hint::black_box(derive_ts_js_refinements(&db))
                        .edges
                        .is_empty()
                );
            }
            let language_name = if language.is_ts_family() {
                "TypeScript"
            } else {
                "Go"
            };
            eprintln!(
                "TS refinement language={language_name}: {} ns/100 runs; expensive index builds={}",
                start.elapsed().as_nanos(),
                REFINEMENT_INDEX_BUILDS.get()
            );
            assert_eq!(REFINEMENT_INDEX_BUILDS.get(), 0);
        }
    }

    fn refinement_db(language: Language) -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        let file = db.add_file(
            "src/app.ts".into(),
            "src/app.ts".to_string(),
            "function caller(fn) { fn(); }\n".to_string(),
        );
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "caller".to_string(),
            Span::point(file, 1, 1),
            language,
            false,
            true,
            1,
            vec!["fn".to_string()],
        ));
        let site = CallSiteFact {
            in_throw: false,
            id: CallSiteId(0),
            language,
            file: FileId::from_raw(0),
            caller: FunctionId::from_raw(0),
            owner_symbol: None,
            body: MirBodyId(0),
            operation: MirOpId(0),
            span: Span::point(file, 1, 1),
            kind: CallSyntaxKind::FunctionValue,
            callee: CallCallee::FunctionValue { place: PlaceId(0) },
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Unresolved,
            precision: CallPrecision::Unknown,
            stable_key: db.stable_key_interner().intern("callsite:callable"),
        };
        db.replace_semantic_mir(MirOutput {
            places: vec![PlaceFact {
                id: PlaceId(0),
                language,
                file: Some(file),
                function: Some(FunctionId::from_raw(0)),
                root: PlaceRoot::Local {
                    function: FunctionId::from_raw(0),
                    name: "fn".to_string(),
                },
                projections: Vec::new(),
                stable_key: db.stable_key_interner().intern("place:callable:stable"),
                status: PlaceStatus::Resolved,
            }],
            ..MirOutput::default()
        })
        .expect("valid MIR places");
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: vec![UnresolvedCallFact {
                site: CallSiteId(0),
                caller: FunctionId::from_raw(0),
                status: CallTargetStatus::Unresolved,
                reason: UnresolvedCallReason::FunctionValue,
                algorithm: CallAlgorithm::Unsupported,
                provenance: CallProvenance::MirShape,
                precision: CallPrecision::Unknown,
                stable_key: db.stable_key_interner().intern("unresolved:callable"),
            }],
        })
        .expect("valid call facts");

        db
    }

    #[test]
    fn unresolved_ts_call_is_reported_as_points_to_unresolved() {
        let db = refinement_db(Language::TypeScript);
        REFINEMENT_INDEX_BUILDS.set(0);
        let output = derive_ts_js_refinements(&db);
        assert_eq!(REFINEMENT_INDEX_BUILDS.get(), 1);

        assert_eq!(output.edges.len(), 1);
        assert_eq!(output.edges[0].algorithm, CallAlgorithm::PointsTo);
        assert_eq!(output.edges[0].tier, RefinedCallTier::PointsToAssisted);
        assert_eq!(
            output.edges[0].synthetic_target.as_deref(),
            Some("ts-js:callable-place:place:callable:stable")
        );
    }
}
