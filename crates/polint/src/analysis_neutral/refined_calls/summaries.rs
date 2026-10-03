use std::collections::HashMap;

use super::facts::{
    RefinedCallConfidence, RefinedCallEdgeFact, RefinedCallTier, RefinedCallValidation,
};
use super::store::RefinedCallOutput;
use crate::analysis_api::{FactFamily, FactRef, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallPrecision, CallProvenance, CallTargetFact, CallTargetStatus,
};
use crate::analysis_neutral::ids::RefinedCallEdgeId;
use crate::analysis_neutral::summaries::facts::{
    SummaryDomainKind, SummaryFact, SummaryPrecision, SummaryStatus,
};

pub fn derive_summary_assisted_refinements(db: &impl AnalysisHost) -> RefinedCallOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    // Each caller's targets in storage order, and each site's language, built once
    // instead of scanning every target per summary and every site per edge.
    let mut targets_by_caller = HashMap::<_, Vec<&CallTargetFact>>::new();
    for target in db.call_targets() {
        targets_by_caller
            .entry(target.caller)
            .or_default()
            .push(target);
    }
    let mut site_languages = HashMap::new();
    for site in db.call_sites() {
        site_languages.entry(site.id).or_insert(site.language);
    }
    let mut edges = Vec::new();
    for summary in db.summary_facts() {
        if summary.domain != SummaryDomainKind::CallEffects
            || summary.status != SummaryStatus::Present
        {
            continue;
        }
        for target in targets_by_caller
            .get(&summary.function)
            .map_or(&[][..], Vec::as_slice)
        {
            let language = site_languages
                .get(&target.site)
                .copied()
                .unwrap_or(crate::internal_core::Language::Unknown);
            edges.push(edge_from_summary(
                db,
                summary,
                target,
                language,
                edges.len(),
            ));
        }
    }
    RefinedCallOutput { edges }.normalized(interner)
}

fn edge_from_summary(
    db: &impl AnalysisHost,
    summary: &SummaryFact,
    target: &CallTargetFact,
    language: crate::internal_core::Language,
    index: usize,
) -> RefinedCallEdgeFact {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let summary_key = metadata_key(
        db,
        FactFamily::SummaryCall,
        summary.id.0,
        &db.resolve_stable_key(summary.stable_key),
    );
    let target_key = metadata_key(
        db,
        FactFamily::CallTarget,
        target.id.0,
        &db.resolve_stable_key(target.stable_key),
    );
    RefinedCallEdgeFact {
        id: RefinedCallEdgeId(index as u64),
        site: target.site,
        base_target: Some(target.id),
        caller: target.caller,
        target_function: target.target_function,
        target_symbol: target.target_symbol,
        synthetic_target: target.synthetic_target.clone(),
        language,
        edge_kind: target.edge_kind,
        algorithm: CallAlgorithm::SummaryAssisted,
        tier: RefinedCallTier::SummaryAssisted,
        status: target.status,
        reason: target.reason,
        provenance: CallProvenance::Native,
        precision: summary_precision(summary.precision),
        validation: RefinedCallValidation::ReferentiallyValidated,
        confidence: summary_confidence(summary.precision, target.status),
        evidence: vec![
            "summary_call_effect".to_string(),
            format!("summary={summary_key}"),
        ],
        input_stable_keys: vec![summary_key.clone(), target_key.clone()],
        stable_key: stable_key_from_parts(
            interner,
            FactFamily::RefinedCallEdge,
            &[
                ("tier", "summary_assisted".to_string()),
                ("summary", summary_key),
                ("base_target", target_key),
            ],
        ),
    }
}

fn summary_precision(precision: SummaryPrecision) -> CallPrecision {
    match precision {
        SummaryPrecision::Local | SummaryPrecision::SetupAware => CallPrecision::SetupAware,
        SummaryPrecision::Heuristic => CallPrecision::Heuristic,
        SummaryPrecision::UnknownTop => CallPrecision::Unknown,
    }
}

fn summary_confidence(
    precision: SummaryPrecision,
    target_status: CallTargetStatus,
) -> RefinedCallConfidence {
    match target_status {
        CallTargetStatus::Resolved => match precision {
            SummaryPrecision::Local | SummaryPrecision::SetupAware => RefinedCallConfidence::High,
            SummaryPrecision::Heuristic => RefinedCallConfidence::Medium,
            SummaryPrecision::UnknownTop => RefinedCallConfidence::Low,
        },
        CallTargetStatus::Ambiguous => RefinedCallConfidence::Medium,
        CallTargetStatus::Unresolved
        | CallTargetStatus::Unsupported
        | CallTargetStatus::SetupMissing
        | CallTargetStatus::BudgetExceeded
        | CallTargetStatus::Rejected
        | CallTargetStatus::Unreachable => RefinedCallConfidence::Low,
    }
}

fn metadata_key(db: &impl AnalysisHost, family: FactFamily, run_id: u64, fallback: &str) -> String {
    db.metadata_for(FactRef::new(family, run_id))
        .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
        .unwrap_or_else(|| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallCallee, CallEdgeKind, CallSiteFact, CallSyntaxKind, CallTargetFact,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::ids::{CallSiteId, CallTargetId, MirBodyId, MirOpId, SummaryId};
    use crate::analysis_neutral::summaries::facts::SummaryProvenance;
    use crate::analysis_neutral::summaries::store::SummaryOutput;
    use crate::internal_core::{FileId, FunctionId, Language, Span, SymbolId};

    #[test]
    fn bindable_call_summary_creates_summary_assisted_edge() {
        let mut db = db_with_call_target();
        db.replace_summary_facts(SummaryOutput {
            summaries: vec![summary(
                SummaryStatus::Present,
                SummaryDomainKind::CallEffects,
            )],
            events: Vec::new(),
        });

        let output = derive_summary_assisted_refinements(&db);

        assert_eq!(output.edges.len(), 1);
        assert_eq!(output.edges[0].tier, RefinedCallTier::SummaryAssisted);
        assert_eq!(output.edges[0].algorithm, CallAlgorithm::SummaryAssisted);
    }

    #[test]
    fn missing_or_non_call_summary_does_not_create_guessed_edges() {
        let mut db = db_with_call_target();
        db.replace_summary_facts(SummaryOutput {
            summaries: vec![summary(
                SummaryStatus::Present,
                SummaryDomainKind::ControlEffects,
            )],
            events: Vec::new(),
        });

        let output = derive_summary_assisted_refinements(&db);

        assert!(output.edges.is_empty());
    }

    fn db_with_call_target() -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        let file = db.add_file(
            "src/app.ts".into(),
            "src/app.ts".to_string(),
            "function caller() { callee(); } function callee() {}\n".to_string(),
        );
        let caller = db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            file,
            "caller".to_string(),
            span(),
            Language::TypeScript,
            false,
            true,
            1,
            vec!["callee".to_string()],
        ));
        let callee = db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            file,
            "callee".to_string(),
            span(),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        db.replace_call_facts(CallOutput {
            sites: vec![CallSiteFact {
                in_throw: false,
                id: CallSiteId(0),
                language: Language::TypeScript,
                file,
                caller,
                owner_symbol: None,
                body: MirBodyId(0),
                operation: MirOpId(0),
                span: span(),
                kind: CallSyntaxKind::Function,
                callee: CallCallee::Identifier {
                    reference: None,
                    name: "callee".to_string(),
                },
                receiver: None,
                arguments: Vec::new(),
                result: None,
                status: CallTargetStatus::Resolved,
                precision: CallPrecision::SetupAware,
                stable_key: crate::internal_core::StableKeyId(0),
            }],
            targets: vec![CallTargetFact {
                id: CallTargetId(0),
                site: CallSiteId(0),
                caller,
                target_function: Some(callee),
                target_symbol: Some(SymbolId::from_raw(0)),
                synthetic_target: None,
                edge_kind: CallEdgeKind::Direct,
                algorithm: CallAlgorithm::DirectReference,
                status: CallTargetStatus::Resolved,
                reason: None,
                provenance: CallProvenance::NativeDirect,
                precision: CallPrecision::SetupAware,
                stable_key: crate::internal_core::StableKeyId(1),
            }],
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db
    }

    fn summary(status: SummaryStatus, domain: SummaryDomainKind) -> SummaryFact {
        SummaryFact {
            id: SummaryId(0),
            callable_stable_key: crate::internal_core::stable_key_for_test("function:caller"),
            function: FunctionId::from_raw(0),
            domain,
            status,
            precision: SummaryPrecision::SetupAware,
            provenance: SummaryProvenance::NativeLocal,
            payload_digest: "digest".to_string(),
            tito_flows: Vec::new(),
            stable_key: crate::internal_core::stable_key_for_test("summary:caller:call"),
        }
    }

    fn span() -> Span {
        Span::point(FileId::from_raw(0), 1, 1)
    }
}
