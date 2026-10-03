use super::facts::{
    RefinedCallConfidence, RefinedCallEdgeFact, RefinedCallTier, RefinedCallValidation,
};
use super::store::RefinedCallOutput;
use crate::analysis_api::{FactFamily, FactRef, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallEdgeKind, CallPrecision, CallProvenance, CallTargetFact, CallTargetStatus,
    UnresolvedCallFact, UnresolvedCallReason,
};
use std::collections::HashMap;

use crate::analysis_neutral::calls::facts::CallSiteFact;
use crate::analysis_neutral::ids::{CallSiteId, PlaceId, PtVarId, RefinedCallEdgeId};
use crate::analysis_neutral::points_to::facts::{
    PointsToBudgetStatus, PointsToSetFact, PointsToStatus,
};
use crate::analysis_neutral::points_to::vars::place_var;
use crate::analysis_neutral::types::facts::{TypeFact, TypePrecision, TypeStatus, TypeSubject};
use crate::internal_core::Language;

pub fn derive_go_refinements(db: &impl AnalysisHost) -> RefinedCallOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut edges = Vec::new();
    let index = GoRefinementIndex::build(db);

    for site in db
        .call_sites()
        .iter()
        .filter(|site| site.language == Language::Go)
    {
        if let Some(receiver) = site.receiver {
            let receiver_types = index.type_facts(receiver);
            let receiver_points_to = index.points_to_sets(receiver);

            for target in index.targets(site.id) {
                if receiver_types
                    .iter()
                    .any(|fact| fact.status == TypeStatus::Present)
                {
                    edges.push(type_edge_from_target(
                        db,
                        target,
                        receiver_types
                            .iter()
                            .find(|fact| fact.status == TypeStatus::Present)
                            .copied(),
                        edges.len(),
                    ));
                }

                if receiver_points_to.iter().any(|set| {
                    set.status == PointsToStatus::Present
                        && set.budget == PointsToBudgetStatus::WithinBudget
                }) {
                    edges.push(points_to_edge_from_target(
                        db,
                        target,
                        receiver_points_to
                            .iter()
                            .find(|set| {
                                set.status == PointsToStatus::Present
                                    && set.budget == PointsToBudgetStatus::WithinBudget
                            })
                            .copied(),
                        edges.len(),
                    ));
                }
            }
        }
    }

    for unresolved in db.unresolved_calls().iter().filter(|unresolved| {
        matches!(
            unresolved.reason,
            UnresolvedCallReason::InterfaceDispatch
                | UnresolvedCallReason::FunctionValue
                | UnresolvedCallReason::DynamicProperty
        )
    }) {
        if let Some(site) = index.go_site(unresolved.site) {
            let mut emitted_status_edge = false;
            if site.receiver.is_some_and(|receiver| {
                index
                    .type_facts(receiver)
                    .iter()
                    .any(|fact| fact.status == TypeStatus::SetupMissing)
            }) {
                edges.push(unresolved_go_edge(
                    db,
                    unresolved,
                    CallTargetStatus::SetupMissing,
                    UnresolvedCallReason::SetupMissing,
                    RefinedCallTier::TypeValueFunctionToken,
                    edges.len(),
                ));
                emitted_status_edge = true;
            }

            if site.receiver.is_some_and(|receiver| {
                index.points_to_sets(receiver).iter().any(|set| {
                    set.status == PointsToStatus::BudgetExceeded
                        || set.budget == PointsToBudgetStatus::BudgetExceeded
                })
            }) {
                edges.push(unresolved_go_edge(
                    db,
                    unresolved,
                    CallTargetStatus::BudgetExceeded,
                    UnresolvedCallReason::BudgetExceeded,
                    RefinedCallTier::PointsToAssisted,
                    edges.len(),
                ));
                emitted_status_edge = true;
            }

            if !emitted_status_edge {
                edges.push(unresolved_go_edge(
                    db,
                    unresolved,
                    CallTargetStatus::Unresolved,
                    unresolved.reason,
                    RefinedCallTier::TypeValueFunctionToken,
                    edges.len(),
                ));
            }
        }
    }

    RefinedCallOutput { edges }.normalized(interner)
}

/// Lookups the Go tier makes once per call site, built once per run.
///
/// Every bucket keeps the facts in storage order, which is the order the
/// per-site scans of the whole fact families they replace returned them in, so
/// the tier picks the same facts and emits the same edges.
struct GoRefinementIndex<'db> {
    targets_by_site: HashMap<CallSiteId, Vec<&'db CallTargetFact>>,
    type_facts_by_place: HashMap<PlaceId, Vec<&'db TypeFact>>,
    points_to_by_variable: HashMap<PtVarId, Vec<&'db PointsToSetFact>>,
    go_sites: HashMap<CallSiteId, &'db CallSiteFact>,
}

impl<'db> GoRefinementIndex<'db> {
    fn build(db: &'db impl AnalysisHost) -> Self {
        let mut targets_by_site = HashMap::<_, Vec<_>>::new();
        for target in db.call_targets() {
            targets_by_site.entry(target.site).or_default().push(target);
        }
        let mut type_facts_by_place = HashMap::<_, Vec<_>>::new();
        for fact in db.type_facts() {
            if let TypeSubject::Place(place) = fact.subject {
                type_facts_by_place.entry(place).or_default().push(fact);
            }
        }
        let mut points_to_by_variable = HashMap::<_, Vec<_>>::new();
        for set in db.points_to_sets() {
            points_to_by_variable
                .entry(set.variable)
                .or_default()
                .push(set);
        }
        let mut go_sites = HashMap::new();
        for site in db
            .call_sites()
            .iter()
            .filter(|site| site.language == Language::Go)
        {
            // The first site with an id, as a scan for it would find.
            go_sites.entry(site.id).or_insert(site);
        }
        Self {
            targets_by_site,
            type_facts_by_place,
            points_to_by_variable,
            go_sites,
        }
    }

    fn targets(&self, site: CallSiteId) -> &[&'db CallTargetFact] {
        self.targets_by_site.get(&site).map_or(&[], Vec::as_slice)
    }

    fn type_facts(&self, place: PlaceId) -> &[&'db TypeFact] {
        self.type_facts_by_place
            .get(&place)
            .map_or(&[], Vec::as_slice)
    }

    fn points_to_sets(&self, place: PlaceId) -> &[&'db PointsToSetFact] {
        self.points_to_by_variable
            .get(&place_var(place))
            .map_or(&[], Vec::as_slice)
    }

    fn go_site(&self, site: CallSiteId) -> Option<&'db CallSiteFact> {
        self.go_sites.get(&site).copied()
    }
}

fn type_edge_from_target(
    db: &impl AnalysisHost,
    target: &CallTargetFact,
    type_fact: Option<&TypeFact>,
    index: usize,
) -> RefinedCallEdgeFact {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let type_key = type_fact
        .map(|fact| metadata_key(db, FactFamily::Type, fact.id.0, fact.stable_key))
        .unwrap_or_else(|| "type:none".to_string());
    let target_key = metadata_key(db, FactFamily::CallTarget, target.id.0, target.stable_key);
    edge_from_target(
        target,
        RefinedCallEdgeId(index as u64),
        TargetRefinement {
            tier: RefinedCallTier::TypeValueFunctionToken,
            algorithm: CallAlgorithm::TypeHierarchy,
            precision: type_fact.map_or(CallPrecision::Heuristic, |fact| {
                type_precision(fact.precision)
            }),
            evidence: vec!["go_receiver_type".to_string(), format!("type={type_key}")],
            input_stable_keys: vec![target_key.clone(), type_key.clone()],
            stable_key: stable_key_from_parts(
                interner,
                FactFamily::RefinedCallEdge,
                &[
                    ("tier", "go_receiver_type".to_string()),
                    ("base_target", target_key),
                    ("type", type_key),
                ],
            ),
        },
    )
}

fn points_to_edge_from_target(
    db: &impl AnalysisHost,
    target: &CallTargetFact,
    points_to: Option<&PointsToSetFact>,
    index: usize,
) -> RefinedCallEdgeFact {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let points_to_key = points_to
        .map(|fact| metadata_key(db, FactFamily::PointsToSet, fact.id.0, fact.stable_key))
        .unwrap_or_else(|| "points-to:none".to_string());
    let target_key = metadata_key(db, FactFamily::CallTarget, target.id.0, target.stable_key);
    edge_from_target(
        target,
        RefinedCallEdgeId(index as u64),
        TargetRefinement {
            tier: RefinedCallTier::PointsToAssisted,
            algorithm: CallAlgorithm::PointsTo,
            precision: CallPrecision::Conservative,
            evidence: vec![
                "go_receiver_points_to".to_string(),
                format!("points_to={points_to_key}"),
            ],
            input_stable_keys: vec![target_key.clone(), points_to_key.clone()],
            stable_key: stable_key_from_parts(
                interner,
                FactFamily::RefinedCallEdge,
                &[
                    ("tier", "go_points_to".to_string()),
                    ("base_target", target_key),
                    ("points_to", points_to_key),
                ],
            ),
        },
    )
}

struct TargetRefinement {
    tier: RefinedCallTier,
    algorithm: CallAlgorithm,
    precision: CallPrecision,
    evidence: Vec<String>,
    input_stable_keys: Vec<String>,
    stable_key: crate::internal_core::StableKeyId,
}

fn edge_from_target(
    target: &CallTargetFact,
    id: RefinedCallEdgeId,
    refinement: TargetRefinement,
) -> RefinedCallEdgeFact {
    RefinedCallEdgeFact {
        id,
        site: target.site,
        base_target: Some(target.id),
        caller: target.caller,
        target_function: target.target_function,
        target_symbol: target.target_symbol,
        synthetic_target: target.synthetic_target.clone(),
        language: Language::Go,
        edge_kind: target.edge_kind,
        algorithm: refinement.algorithm,
        tier: refinement.tier,
        status: target.status,
        reason: target.reason,
        provenance: CallProvenance::Native,
        precision: refinement.precision,
        validation: RefinedCallValidation::ReferentiallyValidated,
        confidence: confidence_for_status(target.status),
        evidence: refinement.evidence,
        input_stable_keys: refinement.input_stable_keys,
        stable_key: refinement.stable_key,
    }
}

fn unresolved_go_edge(
    db: &impl AnalysisHost,
    unresolved: &UnresolvedCallFact,
    status: CallTargetStatus,
    reason: UnresolvedCallReason,
    tier: RefinedCallTier,
    index: usize,
) -> RefinedCallEdgeFact {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let unresolved_stable_key = db.resolve_stable_key(unresolved.stable_key);
    let unresolved_key = metadata_key(
        db,
        FactFamily::UnresolvedCall,
        unresolved_stable_key
            .rsplit(':')
            .next()
            .and_then(|part| part.parse::<u64>().ok())
            .unwrap_or(index as u64),
        unresolved.stable_key,
    );
    RefinedCallEdgeFact {
        id: RefinedCallEdgeId(index as u64),
        site: unresolved.site,
        base_target: None,
        caller: unresolved.caller,
        target_function: None,
        target_symbol: None,
        synthetic_target: Some("go:interface-dispatch".to_string()),
        language: Language::Go,
        edge_kind: CallEdgeKind::Unknown,
        algorithm: match tier {
            RefinedCallTier::PointsToAssisted => CallAlgorithm::PointsTo,
            _ => CallAlgorithm::TypeHierarchy,
        },
        tier,
        status,
        reason: Some(reason),
        provenance: CallProvenance::Native,
        precision: match status {
            CallTargetStatus::SetupMissing => CallPrecision::Unknown,
            CallTargetStatus::BudgetExceeded => CallPrecision::Unknown,
            _ => unresolved.precision,
        },
        validation: RefinedCallValidation::ReferentiallyValidated,
        confidence: RefinedCallConfidence::Low,
        evidence: vec!["go_unresolved_dispatch".to_string()],
        input_stable_keys: vec![unresolved_key.clone()],
        stable_key: stable_key_from_parts(
            interner,
            FactFamily::RefinedCallEdge,
            &[
                ("tier", format!("{tier:?}")),
                ("unresolved", unresolved_key),
                ("status", format!("{status:?}")),
            ],
        ),
    }
}

fn type_precision(precision: TypePrecision) -> CallPrecision {
    match precision {
        TypePrecision::ExactLocal | TypePrecision::SetupAware => CallPrecision::SetupAware,
        TypePrecision::Conservative => CallPrecision::Conservative,
        TypePrecision::Heuristic => CallPrecision::Heuristic,
        TypePrecision::Unknown => CallPrecision::Unknown,
        TypePrecision::Unsupported => CallPrecision::Unsupported,
    }
}

fn confidence_for_status(status: CallTargetStatus) -> RefinedCallConfidence {
    match status {
        CallTargetStatus::Resolved => RefinedCallConfidence::High,
        CallTargetStatus::Ambiguous => RefinedCallConfidence::Medium,
        CallTargetStatus::Unresolved
        | CallTargetStatus::Unsupported
        | CallTargetStatus::SetupMissing
        | CallTargetStatus::BudgetExceeded
        | CallTargetStatus::Rejected
        | CallTargetStatus::Unreachable => RefinedCallConfidence::Low,
    }
}

fn metadata_key(
    db: &impl AnalysisHost,
    family: FactFamily,
    run_id: u64,
    fallback: crate::internal_core::StableKeyId,
) -> String {
    db.metadata_for(FactRef::new(family, run_id))
        .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
        .unwrap_or_else(|| db.resolve_stable_key(fallback).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallCallee, CallSiteFact, CallSyntaxKind, CallTargetFact,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::ids::CallSiteId;
    use crate::analysis_neutral::ids::{
        CallTargetId, MirBodyId, MirOpId, PointsToSetId, TypeFactId, TypeSetId,
    };
    use crate::analysis_neutral::places::{PlaceFact, PlaceRoot, PlaceStatus};
    use crate::analysis_neutral::points_to::facts::{PointsToPrecision, PointsToSetFact};
    use crate::analysis_neutral::points_to::store::PointsToOutput;
    use crate::analysis_neutral::types::facts::{
        TypeConfidence, TypeFact, TypePhase, TypeProvenance, TypeShape,
    };
    use crate::analysis_neutral::types::store::{TypeOutput, TypeValueAliasOutput};
    use crate::internal_core::{FileId, FunctionId, Language, Span, SymbolId};

    #[test]
    fn go_receiver_with_concrete_type_creates_type_refined_edge() {
        let mut db = go_db_with_target();
        db.replace_type_value_alias_facts(TypeValueAliasOutput {
            types: TypeOutput {
                types: vec![receiver_type(TypeStatus::Present)],
                narrowed: Vec::new(),
            },
            ..TypeValueAliasOutput::default()
        });

        let output = derive_go_refinements(&db);

        assert_eq!(output.edges.len(), 1);
        assert_eq!(
            output.edges[0].tier,
            RefinedCallTier::TypeValueFunctionToken
        );
        assert_eq!(output.edges[0].algorithm, CallAlgorithm::TypeHierarchy);
    }

    #[test]
    fn setup_missing_go_interface_call_stays_setup_missing() {
        let mut db = go_db_with_unresolved_receiver();
        db.replace_type_value_alias_facts(TypeValueAliasOutput {
            types: TypeOutput {
                types: vec![receiver_type(TypeStatus::SetupMissing)],
                narrowed: Vec::new(),
            },
            ..TypeValueAliasOutput::default()
        });

        let output = derive_go_refinements(&db);

        assert_eq!(output.edges.len(), 1);
        assert_eq!(output.edges[0].status, CallTargetStatus::SetupMissing);
        assert_eq!(
            output.edges[0].reason,
            Some(UnresolvedCallReason::SetupMissing)
        );
    }

    #[test]
    fn unresolved_go_interface_call_remains_explicit_unknown() {
        let db = go_db_with_unresolved_receiver();

        let output = derive_go_refinements(&db);

        assert_eq!(output.edges.len(), 1);
        assert_eq!(output.edges[0].status, CallTargetStatus::Unresolved);
        assert_eq!(
            output.edges[0].reason,
            Some(UnresolvedCallReason::InterfaceDispatch)
        );
    }

    #[test]
    fn unrelated_go_missing_semantic_call_is_not_recast_as_dispatch() {
        let db = go_db_with_unresolved_reason(UnresolvedCallReason::MissingSemanticReference);

        let output = derive_go_refinements(&db);

        assert!(output.edges.is_empty());
    }

    #[test]
    fn within_budget_points_to_set_creates_points_to_assisted_edge() {
        let mut db = go_db_with_target();
        db.replace_type_value_alias_facts(TypeValueAliasOutput {
            points_to: PointsToOutput {
                constraints: Vec::new(),
                sets: vec![points_to_set(
                    PointsToStatus::Present,
                    PointsToBudgetStatus::WithinBudget,
                )],
            },
            ..TypeValueAliasOutput::default()
        });

        let output = derive_go_refinements(&db);

        assert_eq!(output.edges.len(), 1);
        assert_eq!(output.edges[0].tier, RefinedCallTier::PointsToAssisted);
        assert_eq!(output.edges[0].algorithm, CallAlgorithm::PointsTo);
    }

    #[test]
    fn budget_exceeded_points_to_set_creates_budget_row() {
        let mut db = go_db_with_unresolved_receiver();
        db.replace_type_value_alias_facts(TypeValueAliasOutput {
            points_to: PointsToOutput {
                constraints: Vec::new(),
                sets: vec![points_to_set(
                    PointsToStatus::BudgetExceeded,
                    PointsToBudgetStatus::BudgetExceeded,
                )],
            },
            ..TypeValueAliasOutput::default()
        });

        let output = derive_go_refinements(&db);

        assert_eq!(output.edges.len(), 1);
        assert_eq!(output.edges[0].status, CallTargetStatus::BudgetExceeded);
    }

    fn go_db_with_target() -> LocalAnalysisDb {
        let mut db = go_db_base();
        db.replace_call_facts(CallOutput {
            sites: vec![go_call_site()],
            targets: vec![go_target()],
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db
    }

    fn go_db_with_unresolved_receiver() -> LocalAnalysisDb {
        go_db_with_unresolved_reason(UnresolvedCallReason::InterfaceDispatch)
    }

    fn go_db_with_unresolved_reason(reason: UnresolvedCallReason) -> LocalAnalysisDb {
        let mut db = go_db_base();
        db.replace_call_facts(CallOutput {
            sites: vec![go_call_site()],
            targets: Vec::new(),
            unresolved: vec![UnresolvedCallFact {
                site: CallSiteId(0),
                caller: FunctionId::from_raw(0),
                status: CallTargetStatus::Unresolved,
                reason,
                algorithm: CallAlgorithm::Unsupported,
                provenance: CallProvenance::Native,
                precision: CallPrecision::Unknown,
                stable_key: crate::internal_core::StableKeyId(0),
            }],
        })
        .expect("valid call facts");
        db
    }

    fn go_db_base() -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        let file = db.add_file(
            "handler.go".into(),
            "handler.go".to_string(),
            "package p\nfunc caller(r Receiver) { r.Handle() }\nfunc Handle() {}\n".to_string(),
        );
        let caller = db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            file,
            "caller".to_string(),
            span(),
            Language::Go,
            false,
            false,
            1,
            vec!["Handle".to_string()],
        ));
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            file,
            "Handle".to_string(),
            span(),
            Language::Go,
            false,
            false,
            1,
            Vec::new(),
        ));
        db.replace_semantic_mir(crate::analysis_neutral::mir_body::MirOutput {
            bodies: Vec::new(),
            operations: Vec::new(),
            places: vec![PlaceFact {
                id: PlaceId(0),
                language: Language::Go,
                file: Some(file),
                function: Some(caller),
                root: PlaceRoot::Parameter {
                    function: caller,
                    index: 0,
                    name: Some("r".to_string()),
                },
                projections: Vec::new(),
                stable_key: interner.intern("place:receiver".to_string()),
                status: PlaceStatus::Resolved,
            }],
            unsupported: Vec::new(),
            ..crate::analysis_neutral::mir_body::MirOutput::default()
        })
        .expect("valid MIR");
        db
    }

    fn go_call_site() -> CallSiteFact {
        CallSiteFact {
            in_throw: false,
            id: CallSiteId(0),
            language: Language::Go,
            file: FileId::from_raw(0),
            caller: FunctionId::from_raw(0),
            owner_symbol: None,
            body: MirBodyId(0),
            operation: MirOpId(0),
            span: span(),
            kind: CallSyntaxKind::Method,
            callee: CallCallee::Unknown {
                reason: UnresolvedCallReason::InterfaceDispatch,
            },
            receiver: Some(PlaceId(0)),
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Ambiguous,
            precision: CallPrecision::Unknown,
            stable_key: crate::internal_core::StableKeyId(0),
        }
    }

    fn go_target() -> CallTargetFact {
        CallTargetFact {
            id: CallTargetId(0),
            site: CallSiteId(0),
            caller: FunctionId::from_raw(0),
            target_function: Some(FunctionId::from_raw(1)),
            target_symbol: Some(SymbolId::from_raw(0)),
            synthetic_target: None,
            edge_kind: CallEdgeKind::Method,
            algorithm: CallAlgorithm::GoStatic,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::Native,
            precision: CallPrecision::SetupAware,
            stable_key: crate::internal_core::StableKeyId(1),
        }
    }

    fn receiver_type(status: TypeStatus) -> TypeFact {
        TypeFact {
            id: TypeFactId(0),
            subject: TypeSubject::Place(PlaceId(0)),
            type_set: TypeSetId(0),
            shape: TypeShape::Nominal {
                type_id: "Receiver".to_string(),
            },
            phase: TypePhase::Resolved,
            language: Language::Go,
            file: Some(FileId::from_raw(0)),
            function: Some(FunctionId::from_raw(0)),
            body: None,
            place: Some(PlaceId(0)),
            cfg_block: None,
            operation: None,
            precision: TypePrecision::SetupAware,
            confidence: TypeConfidence::High,
            status,
            provenance: TypeProvenance::Native,
            stable_key: crate::internal_core::stable_key_for_test("type:receiver"),
        }
    }

    fn points_to_set(status: PointsToStatus, budget: PointsToBudgetStatus) -> PointsToSetFact {
        PointsToSetFact {
            id: PointsToSetId(0),
            variable: place_var(PlaceId(0)),
            objects: Vec::new(),
            status,
            precision: PointsToPrecision::FlowInsensitive,
            budget,
            stable_key: crate::internal_core::stable_key_for_test("points-to:receiver"),
        }
    }

    fn span() -> Span {
        Span::point(FileId::from_raw(0), 1, 1)
    }
}
