use super::facts::{
    RefinedCallConfidence, RefinedCallEdgeFact, RefinedCallTier, RefinedCallValidation,
};
use super::store::RefinedCallOutput;
use crate::analysis_api::{FactFamily, stable_key_from_key_parts, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallCallee, CallEdgeKind, CallPrecision, CallProvenance, CallTargetStatus,
    UnresolvedCallReason,
};
use crate::analysis_neutral::extensions::sinks::{
    ExtensionFactConfidence, ExtensionFactPrecision, REFINED_CALL_EDGE_FAMILY,
};
use crate::analysis_neutral::extensions::store::AcceptedExtensionFact;
use crate::analysis_neutral::ids::{CallSiteId, RefinedCallEdgeId};
use crate::internal_core::{FunctionId, KeyPart, SymbolId};

pub fn derive_extension_refinements(db: &impl AnalysisHost) -> RefinedCallOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut edges = Vec::new();
    for fact in db
        .extension_facts()
        .iter()
        .filter(|fact| fact.fact_family == REFINED_CALL_EDGE_FAMILY)
    {
        if let Some(edge) = edge_from_extension_fact(db, fact, edges.len()) {
            edges.push(edge);
        }
    }
    RefinedCallOutput { edges }.normalized(interner)
}

fn edge_from_extension_fact(
    db: &impl AnalysisHost,
    fact: &AcceptedExtensionFact,
    index: usize,
) -> Option<RefinedCallEdgeFact> {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let site = resolve_site(db, payload(fact, "site=")?)?;
    let target_function = payload(fact, "target_function=").and_then(|value| {
        parse_function_ref(value)
            .filter(|function| db.functions().iter().any(|fact| fact.id == *function))
    });
    let target_symbol = payload(fact, "target_symbol=").and_then(|value| {
        parse_symbol_ref(value).filter(|symbol| db.symbols().iter().any(|fact| fact.id == *symbol))
    });
    let synthetic_target = payload(fact, "synthetic_target=").map(str::to_string);
    if target_function.is_none() && target_symbol.is_none() && synthetic_target.is_none() {
        return None;
    }
    let call_site = db
        .call_sites()
        .iter()
        .find(|call_site| call_site.id == site)?;
    let algorithm = payload(fact, "algorithm=")
        .map(extension_algorithm)
        .unwrap_or(CallAlgorithm::RepoModel);
    let status = payload(fact, "status=")
        .map(extension_status)
        .unwrap_or(CallTargetStatus::Ambiguous);
    Some(RefinedCallEdgeFact {
        id: RefinedCallEdgeId(index as u64),
        site,
        base_target: None,
        caller: call_site.caller,
        target_function,
        target_symbol,
        synthetic_target,
        language: call_site.language,
        edge_kind: CallEdgeKind::Synthetic,
        algorithm,
        tier: RefinedCallTier::ExtensionModel,
        status,
        reason: reason_for_status(status),
        provenance: extension_provenance(algorithm),
        precision: extension_precision(fact.precision),
        validation: extension_validation(status),
        confidence: extension_confidence(fact.confidence),
        evidence: extension_evidence(fact),
        input_stable_keys: vec![interner.resolve(fact.stable_key).to_string()],
        stable_key: stable_key_from_key_parts(
            interner,
            FactFamily::RefinedCallEdge,
            [
                ("tier", KeyPart::Text("extension_model")),
                ("extension", KeyPart::Text(&fact.extension_id.clone())),
                ("provider", KeyPart::Text(&fact.provider_id.clone())),
                ("candidate", KeyPart::Key(fact.stable_key)),
            ],
        ),
    })
}

fn resolve_site(db: &impl AnalysisHost, value: &str) -> Option<CallSiteId> {
    if let Some(id) = value.strip_prefix("call_site:") {
        if let Ok(id) = id.parse::<u64>() {
            return Some(CallSiteId(id));
        }
        return db
            .call_sites()
            .iter()
            .find(|site| db.resolve_stable_key(site.stable_key).as_ref() == id)
            .map(|site| site.id);
    }
    value
        .strip_prefix("stable:")
        .and_then(|stable| {
            db.call_sites()
                .iter()
                .find(|site| db.resolve_stable_key(site.stable_key).as_ref() == stable)
                .map(|site| site.id)
        })
        .or_else(|| {
            value
                .strip_prefix("file_span:")
                .and_then(|file_span| resolve_file_span_site(db, file_span))
        })
        .or_else(|| {
            value
                .strip_prefix("file_callee:")
                .and_then(|file_callee| resolve_file_callee_site(db, file_callee))
        })
}

fn resolve_file_span_site(db: &impl AnalysisHost, value: &str) -> Option<CallSiteId> {
    let (relative_path, start_byte) = value.rsplit_once(':')?;
    let start_byte = start_byte.parse::<u32>().ok()?;
    db.call_sites()
        .iter()
        .find(|site| {
            site.span.start_byte == start_byte
                && db
                    .file(site.file)
                    .is_some_and(|file| file.relative_path == relative_path)
        })
        .map(|site| site.id)
}

fn resolve_file_callee_site(db: &impl AnalysisHost, value: &str) -> Option<CallSiteId> {
    let (relative_path, callee) = value.rsplit_once(':')?;
    let mut matches = db.call_sites().iter().filter(|site| {
        db.file(site.file)
            .is_some_and(|file| file.relative_path == relative_path)
            && call_site_callee_label(&site.callee) == Some(callee)
    });
    let site = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(site.id)
}

fn call_site_callee_label(callee: &CallCallee) -> Option<&str> {
    match callee {
        CallCallee::Identifier { name, .. } => Some(name.as_str()),
        CallCallee::Member { property, .. } => Some(property.as_str()),
        CallCallee::Constructor { name, .. } => name.as_deref(),
        _ => None,
    }
}

fn parse_function_ref(value: &str) -> Option<FunctionId> {
    value
        .strip_prefix("function:")
        .and_then(|id| id.parse::<u64>().ok())
        .map(FunctionId::from_raw)
}

fn parse_symbol_ref(value: &str) -> Option<SymbolId> {
    value
        .strip_prefix("symbol:")
        .and_then(|id| id.parse::<u64>().ok())
        .map(SymbolId::from_raw)
}

fn extension_algorithm(value: &str) -> CallAlgorithm {
    match value {
        "framework_model" => CallAlgorithm::FrameworkModel,
        "points_to" => CallAlgorithm::PointsTo,
        "repo_model" => CallAlgorithm::RepoModel,
        _ => CallAlgorithm::RepoModel,
    }
}

fn extension_status(value: &str) -> CallTargetStatus {
    match value {
        "resolved" => CallTargetStatus::Resolved,
        "ambiguous" => CallTargetStatus::Ambiguous,
        "unresolved" => CallTargetStatus::Unresolved,
        "budget_exceeded" => CallTargetStatus::BudgetExceeded,
        "rejected" => CallTargetStatus::Rejected,
        _ => CallTargetStatus::Ambiguous,
    }
}

fn extension_precision(precision: ExtensionFactPrecision) -> CallPrecision {
    match precision {
        ExtensionFactPrecision::Exact | ExtensionFactPrecision::SetupAware => {
            CallPrecision::SetupAware
        }
        ExtensionFactPrecision::Heuristic => CallPrecision::Heuristic,
        ExtensionFactPrecision::GeneratedUnvalidated => CallPrecision::Unknown,
    }
}

fn extension_confidence(confidence: ExtensionFactConfidence) -> RefinedCallConfidence {
    match confidence {
        ExtensionFactConfidence::High => RefinedCallConfidence::High,
        ExtensionFactConfidence::Medium => RefinedCallConfidence::Medium,
        ExtensionFactConfidence::Low => RefinedCallConfidence::Low,
    }
}

fn extension_validation(status: CallTargetStatus) -> RefinedCallValidation {
    if status == CallTargetStatus::Rejected {
        RefinedCallValidation::Rejected
    } else {
        RefinedCallValidation::ExtensionValidated
    }
}

fn extension_provenance(algorithm: CallAlgorithm) -> CallProvenance {
    match algorithm {
        CallAlgorithm::RepoModel | CallAlgorithm::FrameworkModel => CallProvenance::Model,
        _ => CallProvenance::Extension,
    }
}

fn reason_for_status(status: CallTargetStatus) -> Option<UnresolvedCallReason> {
    match status {
        CallTargetStatus::Unresolved => Some(UnresolvedCallReason::Unknown),
        CallTargetStatus::BudgetExceeded => Some(UnresolvedCallReason::BudgetExceeded),
        CallTargetStatus::Rejected => Some(UnresolvedCallReason::Unknown),
        _ => None,
    }
}

fn extension_evidence(fact: &AcceptedExtensionFact) -> Vec<String> {
    let mut evidence = vec![
        format!("extension_id={}", fact.extension_id),
        format!("provider_id={}", fact.provider_id),
    ];
    evidence.extend(fact.evidence.iter().cloned());
    evidence
}

fn payload<'a>(fact: &'a AcceptedExtensionFact, prefix: &str) -> Option<&'a str> {
    fact.payload_labels
        .iter()
        .find_map(|label| label.strip_prefix(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallCallee, CallPrecision, CallSiteFact, CallSyntaxKind,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::extensions::store::ExtensionOutput;
    use crate::analysis_neutral::ids::{MirBodyId, MirOpId};
    use crate::internal_core::{FileId, Language, Span};

    #[test]
    fn accepted_extension_refined_edge_carries_extension_evidence() {
        let mut db = db_with_call_site();
        db.replace_extension_facts(ExtensionOutput {
            accepted: vec![accepted_extension_fact(vec![
                "site=file_callee:src/app.ts:model",
                "target_function=function:0",
                "algorithm=repo_model",
                "status=resolved",
            ])],
            ..ExtensionOutput::default()
        });

        let output = derive_extension_refinements(&db);

        assert_eq!(output.edges.len(), 1);
        assert_eq!(output.edges[0].tier, RefinedCallTier::ExtensionModel);
        assert_eq!(output.edges[0].provenance, CallProvenance::Model);
        assert!(
            output.edges[0]
                .evidence
                .iter()
                .any(|item| item == "extension_id=demo")
        );
    }

    #[test]
    fn generated_unvalidated_extension_edge_is_not_exact() {
        let mut db = db_with_call_site();
        let mut fact = accepted_extension_fact(vec![
            "site=call_site:0",
            "synthetic_target=synthetic:callable",
            "algorithm=repo_model",
            "status=ambiguous",
        ]);
        fact.precision = ExtensionFactPrecision::GeneratedUnvalidated;
        db.replace_extension_facts(ExtensionOutput {
            accepted: vec![fact],
            ..ExtensionOutput::default()
        });

        let output = derive_extension_refinements(&db);

        assert_eq!(output.edges[0].precision, CallPrecision::Unknown);
    }

    #[test]
    fn dangling_native_target_id_is_ignored() {
        let mut db = db_with_call_site();
        db.replace_extension_facts(ExtensionOutput {
            accepted: vec![accepted_extension_fact(vec![
                "site=call_site:call-site:model",
                "target_function=function:42",
                "algorithm=repo_model",
                "status=resolved",
            ])],
            ..ExtensionOutput::default()
        });

        let output = derive_extension_refinements(&db);

        assert!(output.edges.is_empty());
    }

    fn db_with_call_site() -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        let file = db.add_file(
            "src/app.ts".into(),
            "src/app.ts".to_string(),
            "function caller() { model(); }\n".to_string(),
        );
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "caller".to_string(),
            span(),
            Language::TypeScript,
            false,
            true,
            1,
            vec!["model".to_string()],
        ));
        db.replace_call_facts(CallOutput {
            sites: vec![CallSiteFact {
                in_throw: false,
                id: CallSiteId(0),
                language: Language::TypeScript,
                file,
                caller: FunctionId::from_raw(0),
                owner_symbol: None,
                body: MirBodyId(0),
                operation: MirOpId(0),
                span: span(),
                kind: CallSyntaxKind::Function,
                callee: CallCallee::Identifier {
                    reference: None,
                    name: "model".to_string(),
                },
                receiver: None,
                arguments: Vec::new(),
                result: None,
                status: CallTargetStatus::Ambiguous,
                precision: CallPrecision::Heuristic,
                stable_key: crate::internal_core::StableKeyId(0),
            }],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db
    }

    fn accepted_extension_fact(labels: Vec<&str>) -> AcceptedExtensionFact {
        AcceptedExtensionFact {
            extension_id: "demo".to_string(),
            provider_id: "model".to_string(),
            fact_family: REFINED_CALL_EDGE_FAMILY.to_string(),
            stable_key: crate::internal_core::stable_key_for_test("extension:refined"),
            binding_refs: vec!["file:src/app.ts".to_string()],
            precision: ExtensionFactPrecision::Heuristic,
            confidence: ExtensionFactConfidence::Medium,
            status: crate::analysis_neutral::extensions::sinks::ExtensionFactStatus::Accepted,
            evidence: vec!["fixture".to_string()],
            payload_labels: labels.into_iter().map(str::to_string).collect(),
            payload_digest: "digest".to_string(),
        }
    }

    fn span() -> Span {
        Span::point(FileId::from_raw(0), 1, 1)
    }
}
