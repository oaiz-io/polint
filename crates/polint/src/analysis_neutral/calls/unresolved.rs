use std::collections::BTreeMap;

use crate::analysis_api::FactFamily;
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallCallee, CallPrecision, CallProvenance, CallSiteFact, CallSyntaxKind,
    CallTargetStatus, UnresolvedCallFact, UnresolvedCallReason,
};
use crate::analysis_neutral::ids::MirOpId;
use crate::analysis_neutral::mir_op::{UnsupportedDomain, UnsupportedSemanticFact};
use crate::analysis_neutral::stable_key::semantic_stable_key;

pub fn derive_unresolved_calls(
    db: &impl AnalysisHost,
    sites: &[CallSiteFact],
) -> Vec<UnresolvedCallFact> {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut rows = BTreeMap::new();

    for site in sites {
        if let Some(reason) = reason_for_site(site) {
            insert_unresolved(interner, &mut rows, site, reason, "call-site-shape");
        }
    }

    let sites_by_operation = sites
        .iter()
        .map(|site| (site.operation, site))
        .collect::<BTreeMap<_, _>>();
    for unsupported in db
        .unsupported_semantics()
        .iter()
        .filter(|row| row.affected_domains.contains(&UnsupportedDomain::Calls))
    {
        let Some(site) = site_for_unsupported(unsupported, sites, &sites_by_operation) else {
            continue;
        };
        let reason = reason_for_unsupported(unsupported);
        let unsupported_stable_key = interner.resolve(unsupported.stable_key);
        insert_unresolved(interner, &mut rows, site, reason, &unsupported_stable_key);
    }

    rows.into_values().collect()
}

/// The unresolved row for a site whose only targets claim no tier. A target
/// found by a name match is not a semantic reference, so the site still lacks
/// one, whatever its shape.
pub(crate) fn unclaimed_site_row(
    db: &impl AnalysisHost,
    site: &CallSiteFact,
) -> UnresolvedCallFact {
    let interner_handle = db.stable_key_interner();
    let mut rows = BTreeMap::new();
    insert_unresolved(
        &interner_handle,
        &mut rows,
        site,
        UnresolvedCallReason::MissingSemanticReference,
        "unclaimed-targets",
    );
    rows.into_values()
        .next()
        .expect("inserting into an empty map leaves one row")
}

fn insert_unresolved(
    interner: &crate::internal_core::StableKeyInterner,
    rows: &mut BTreeMap<String, UnresolvedCallFact>,
    site: &CallSiteFact,
    reason: UnresolvedCallReason,
    evidence: &str,
) {
    let status = status_for_reason(reason);
    let stable_key = unresolved_stable_key(interner, site, reason, status, evidence);
    rows.entry(interner.resolve(stable_key).to_string())
        .or_insert(UnresolvedCallFact {
            site: site.id,
            caller: site.caller,
            status,
            reason,
            algorithm: algorithm_for_reason(reason),
            provenance: CallProvenance::MirShape,
            precision: precision_for_reason(reason),
            stable_key,
        });
}

fn reason_for_site(site: &CallSiteFact) -> Option<UnresolvedCallReason> {
    match &site.callee {
        CallCallee::FunctionValue { .. } => Some(UnresolvedCallReason::FunctionValue),
        CallCallee::Unknown { reason } => Some(normalize_unknown_reason(*reason)),
        CallCallee::Identifier {
            reference: None, ..
        }
        | CallCallee::Constructor {
            reference: None, ..
        } => Some(UnresolvedCallReason::MissingSemanticReference),
        CallCallee::Index { .. } => Some(UnresolvedCallReason::DynamicProperty),
        CallCallee::Member { .. } if matches!(site.kind, CallSyntaxKind::Member) => {
            Some(UnresolvedCallReason::DynamicProperty)
        }
        CallCallee::Import | CallCallee::Constructor { .. }
            if matches!(site.kind, CallSyntaxKind::DynamicImport) =>
        {
            Some(UnresolvedCallReason::DynamicImport)
        }
        _ => None,
    }
}

fn normalize_unknown_reason(reason: UnresolvedCallReason) -> UnresolvedCallReason {
    match reason {
        UnresolvedCallReason::Unknown => UnresolvedCallReason::UnknownCallee,
        other => other,
    }
}

fn site_for_unsupported<'site>(
    unsupported: &UnsupportedSemanticFact,
    sites: &'site [CallSiteFact],
    sites_by_operation: &BTreeMap<MirOpId, &'site CallSiteFact>,
) -> Option<&'site CallSiteFact> {
    unsupported
        .operation
        .and_then(|operation| sites_by_operation.get(&operation).copied())
        .or_else(|| {
            sites.iter().find(|site| {
                site.file == unsupported.file
                    && site.language == unsupported.language
                    && spans_overlap_or_touch(&site.span, &unsupported.span)
            })
        })
}

fn reason_for_unsupported(row: &UnsupportedSemanticFact) -> UnresolvedCallReason {
    let construct = row.construct.to_ascii_lowercase();
    let evidence = row.source_evidence.to_ascii_lowercase();
    let labels = [construct.as_str(), evidence.as_str()];
    if any_contains(
        &labels,
        &["setup missing", "setup-missing", "package missing"],
    ) {
        UnresolvedCallReason::SetupMissing
    } else if any_contains(&labels, &["interface"]) {
        UnresolvedCallReason::InterfaceDispatch
    } else if any_contains(&labels, &["reflect"]) {
        UnresolvedCallReason::Reflection
    } else if any_contains(&labels, &["go_statement", "goroutine", "go statement"]) {
        UnresolvedCallReason::GoroutineBoundary
    } else if any_contains(&labels, &["eval"]) {
        UnresolvedCallReason::Eval
    } else if any_contains(&labels, &["dynamic import", "import("]) {
        UnresolvedCallReason::DynamicImport
    } else if any_contains(&labels, &["call/apply/bind", ".call", ".apply", ".bind"]) {
        UnresolvedCallReason::CallApplyBind
    } else if any_contains(&labels, &["proxy", "getter", "setter", "accessor"]) {
        UnresolvedCallReason::ProxyOrAccessor
    } else if any_contains(&labels, &["framework", "decorator"]) {
        UnresolvedCallReason::FrameworkDispatch
    } else if any_contains(&labels, &["dynamic property", "index", "computed"]) {
        UnresolvedCallReason::DynamicProperty
    } else if any_contains(&labels, &["unsupported", "parser", "syntax"]) {
        UnresolvedCallReason::UnsupportedSyntax
    } else {
        UnresolvedCallReason::UnknownCallee
    }
}

fn any_contains(values: &[&str], needles: &[&str]) -> bool {
    values
        .iter()
        .any(|value| needles.iter().any(|needle| value.contains(needle)))
}

fn spans_overlap_or_touch(
    left: &crate::internal_core::Span,
    right: &crate::internal_core::Span,
) -> bool {
    left.file == right.file
        && left.start_byte <= right.end_byte
        && right.start_byte <= left.end_byte
}

fn unresolved_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    site: &CallSiteFact,
    reason: UnresolvedCallReason,
    status: CallTargetStatus,
    evidence: &str,
) -> crate::internal_core::StableKeyId {
    interner.intern(
        semantic_stable_key(
            FactFamily::UnresolvedCall,
            &[
                ("site", interner.resolve(site.stable_key).to_string()),
                ("reason", format!("{reason:?}")),
                ("status", format!("{status:?}")),
                ("algorithm", format!("{:?}", algorithm_for_reason(reason))),
                ("evidence", evidence.to_string()),
            ],
        )
        .into_string(),
    )
}

fn status_for_reason(reason: UnresolvedCallReason) -> CallTargetStatus {
    match reason {
        UnresolvedCallReason::SetupMissing => CallTargetStatus::SetupMissing,
        UnresolvedCallReason::UnsupportedSyntax
        | UnresolvedCallReason::Reflection
        | UnresolvedCallReason::GoroutineBoundary
        | UnresolvedCallReason::FrameworkDispatch
        | UnresolvedCallReason::ProxyOrAccessor => CallTargetStatus::Unsupported,
        _ => CallTargetStatus::Unresolved,
    }
}

fn precision_for_reason(reason: UnresolvedCallReason) -> CallPrecision {
    match reason {
        UnresolvedCallReason::SetupMissing => CallPrecision::Unsupported,
        UnresolvedCallReason::UnsupportedSyntax
        | UnresolvedCallReason::Reflection
        | UnresolvedCallReason::GoroutineBoundary
        | UnresolvedCallReason::FrameworkDispatch
        | UnresolvedCallReason::ProxyOrAccessor => CallPrecision::Unsupported,
        _ => CallPrecision::Unknown,
    }
}

fn algorithm_for_reason(reason: UnresolvedCallReason) -> CallAlgorithm {
    match reason {
        UnresolvedCallReason::FrameworkDispatch => CallAlgorithm::FrameworkModel,
        UnresolvedCallReason::UnsupportedSyntax
        | UnresolvedCallReason::Reflection
        | UnresolvedCallReason::GoroutineBoundary
        | UnresolvedCallReason::ProxyOrAccessor => CallAlgorithm::Unsupported,
        _ => CallAlgorithm::SyntaxOnly,
    }
}

#[cfg(test)]
mod tests {
    use crate::analysis_neutral::AnalysisHost;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallCallee, CallPrecision, CallSiteFact, CallSyntaxKind, CallTargetStatus,
        UnresolvedCallReason,
    };
    use crate::analysis_neutral::ids::{CallSiteId, MirBodyId, MirOpId, PlaceId, UnsupportedId};
    use crate::analysis_neutral::mir_body::{MirBody, MirOutput, MirStatus};
    use crate::analysis_neutral::mir_op::{
        ConservativeAction, MirOperation, MirOperationKind, MirValue, UnsupportedDomain,
        UnsupportedPrecision, UnsupportedSemanticFact,
    };
    use crate::analysis_neutral::places::{PlaceFact, PlaceRoot, PlaceStatus};
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    fn span(file: FileId, line: u32) -> Span {
        Span::new(file, line * 10, line * 10 + 4, line, 1, line, 5)
    }

    fn db_with_function(language: Language, path: &str) -> (LocalAnalysisDb, FileId, FunctionId) {
        let mut db = LocalAnalysisDb::new();
        let file = db.add_file(PathBuf::from(path), path.to_string(), String::new());
        let function = db.push_function(FunctionFact::new(
            FunctionId::from_raw(999),
            file,
            "caller".to_string(),
            span(file, 1),
            language,
            false,
            true,
            1,
            Vec::new(),
        ));
        (db, file, function)
    }

    fn site(
        db: &impl AnalysisHost,
        language: Language,
        file: FileId,
        caller: FunctionId,
        site: u64,
        callee: CallCallee,
        kind: CallSyntaxKind,
    ) -> CallSiteFact {
        CallSiteFact {
            in_throw: false,
            id: CallSiteId(site),
            language,
            file,
            caller,
            owner_symbol: None,
            body: MirBodyId(0),
            operation: MirOpId(site),
            span: span(file, 2),
            kind,
            callee,
            receiver: None,
            arguments: Vec::new(),
            result: Some(PlaceId(2)),
            status: CallTargetStatus::Unresolved,
            precision: CallPrecision::Conservative,
            stable_key: db.stable_key_interner().intern(format!("call-site:{site}")),
        }
    }

    fn replace_unsupported(
        db: &mut impl AnalysisHost,
        language: Language,
        file: FileId,
        function: FunctionId,
        rows: Vec<UnsupportedSemanticFact>,
    ) {
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![MirBody {
                id: MirBodyId(0),
                language,
                file,
                function,
                package: None,
                module: None,
                owner_stable_key: interner.intern("function:caller".to_string()),
                span: span(file, 1),
                stable_key: interner.intern("mir-body:caller".to_string()),
                status: MirStatus::Partial,
            }],
            places: vec![PlaceFact {
                id: PlaceId(1),
                language,
                file: Some(file),
                function: Some(function),
                root: PlaceRoot::Local {
                    function,
                    name: "callee".to_string(),
                },
                projections: Vec::new(),
                stable_key: interner.intern("place:callee".to_string()),
                status: PlaceStatus::Partial,
            }],
            operations: vec![MirOperation {
                id: MirOpId(0),
                body: MirBodyId(0),
                ordinal: 0,
                span: span(file, 2),
                kind: MirOperationKind::Call {
                    site: CallSiteId(10),
                    callee: MirValue::Place(PlaceId(1)),
                    arguments: Vec::new(),
                    return_place: PlaceId(1),
                },
                stable_key: interner.intern("mir-op:call".to_string()),
                status: MirStatus::Partial,
            }],
            unsupported: rows,
            ..MirOutput::default()
        })
        .expect("semantic MIR should store");
    }

    fn unsupported(
        db: &impl AnalysisHost,
        id: u64,
        language: Language,
        file: FileId,
        construct: &str,
    ) -> UnsupportedSemanticFact {
        UnsupportedSemanticFact {
            id: UnsupportedId(id),
            body: Some(MirBodyId(0)),
            operation: Some(MirOpId(0)),
            language,
            file,
            span: span(file, 2 + id as u32),
            construct: construct.to_string(),
            source_evidence: construct.to_string(),
            affected_places: Vec::new(),
            affected_domains: vec![UnsupportedDomain::Calls],
            conservative_action: ConservativeAction::HavocAffectedPlaces,
            precision: UnsupportedPrecision::Unsupported,
            status: MirStatus::Unsupported,
            stable_key: db
                .stable_key_interner()
                .intern(format!("unsupported:{construct}")),
        }
    }

    #[test]
    fn unsupported_call_evidence_without_operation_or_span_does_not_attach_to_arbitrary_file_call()
    {
        let (mut db, file, function) = db_with_function(Language::TypeScript, "src/app.ts");
        let mut row = unsupported(&db, 1, Language::TypeScript, file, "dynamic property");
        row.operation = None;
        row.span = span(file, 99);
        replace_unsupported(&mut db, Language::TypeScript, file, function, vec![row]);
        let sites = vec![site(
            &db,
            Language::TypeScript,
            file,
            function,
            10,
            CallCallee::Identifier {
                reference: Some(crate::internal_core::ReferenceId::from_raw(1)),
                name: "run".to_string(),
            },
            CallSyntaxKind::Function,
        )];

        let unresolved = super::derive_unresolved_calls(&db, &sites);

        assert!(unresolved.is_empty());
    }

    #[test]
    fn derive_unresolved_calls_emits_function_value_and_unknown_callee_rows() {
        let (db, file, caller) = db_with_function(Language::TypeScript, "src/app.ts");
        let sites = vec![
            site(
                &db,
                Language::TypeScript,
                file,
                caller,
                10,
                CallCallee::FunctionValue { place: PlaceId(1) },
                CallSyntaxKind::FunctionValue,
            ),
            site(
                &db,
                Language::TypeScript,
                file,
                caller,
                20,
                CallCallee::Unknown {
                    reason: UnresolvedCallReason::UnknownCallee,
                },
                CallSyntaxKind::Unknown,
            ),
        ];

        let unresolved = super::derive_unresolved_calls(&db, &sites);
        let reasons = unresolved
            .iter()
            .map(|row| (row.site, row.reason, row.status, row.precision))
            .collect::<Vec<_>>();

        assert_eq!(
            reasons,
            vec![
                (
                    CallSiteId(10),
                    UnresolvedCallReason::FunctionValue,
                    CallTargetStatus::Unresolved,
                    CallPrecision::Unknown,
                ),
                (
                    CallSiteId(20),
                    UnresolvedCallReason::UnknownCallee,
                    CallTargetStatus::Unresolved,
                    CallPrecision::Unknown,
                ),
            ]
        );
    }

    #[test]
    fn derive_unresolved_calls_emits_missing_semantic_reference_for_unbound_direct_shapes() {
        let (db, file, caller) = db_with_function(Language::TypeScript, "src/app.ts");
        let sites = vec![
            site(
                &db,
                Language::TypeScript,
                file,
                caller,
                30,
                CallCallee::Identifier {
                    reference: None,
                    name: "missing".to_string(),
                },
                CallSyntaxKind::Function,
            ),
            site(
                &db,
                Language::TypeScript,
                file,
                caller,
                40,
                CallCallee::Constructor {
                    reference: None,
                    name: Some("Missing".to_string()),
                },
                CallSyntaxKind::Constructor,
            ),
        ];

        let reasons = super::derive_unresolved_calls(&db, &sites)
            .into_iter()
            .map(|row| (row.site, row.reason, row.status))
            .collect::<Vec<_>>();

        assert_eq!(
            reasons,
            vec![
                (
                    CallSiteId(30),
                    UnresolvedCallReason::MissingSemanticReference,
                    CallTargetStatus::Unresolved,
                ),
                (
                    CallSiteId(40),
                    UnresolvedCallReason::MissingSemanticReference,
                    CallTargetStatus::Unresolved,
                ),
            ]
        );
    }

    #[test]
    fn derive_unresolved_calls_preserves_specific_ts_unsupported_reasons() {
        let (mut db, file, caller) = db_with_function(Language::TypeScript, "src/app.ts");
        let unsupported = vec![
            unsupported(&db, 1, Language::TypeScript, file, "eval"),
            unsupported(&db, 2, Language::TypeScript, file, "dynamic property key"),
            unsupported(&db, 3, Language::TypeScript, file, "dynamic import"),
            unsupported(&db, 4, Language::TypeScript, file, "call/apply/bind"),
            unsupported(&db, 5, Language::TypeScript, file, "Proxy"),
            unsupported(&db, 6, Language::TypeScript, file, "decorator dispatch"),
            unsupported(&db, 7, Language::TypeScript, file, "framework dispatch"),
        ];
        replace_unsupported(&mut db, Language::TypeScript, file, caller, unsupported);
        let sites = vec![site(
            &db,
            Language::TypeScript,
            file,
            caller,
            0,
            CallCallee::Unknown {
                reason: UnresolvedCallReason::UnknownCallee,
            },
            CallSyntaxKind::Unknown,
        )];

        let reasons = super::derive_unresolved_calls(&db, &sites)
            .into_iter()
            .map(|row| row.reason)
            .collect::<BTreeSet<_>>();

        assert!(reasons.contains(&UnresolvedCallReason::Eval));
        assert!(reasons.contains(&UnresolvedCallReason::DynamicProperty));
        assert!(reasons.contains(&UnresolvedCallReason::DynamicImport));
        assert!(reasons.contains(&UnresolvedCallReason::CallApplyBind));
        assert!(reasons.contains(&UnresolvedCallReason::ProxyOrAccessor));
        assert!(reasons.contains(&UnresolvedCallReason::FrameworkDispatch));
    }

    #[test]
    fn derive_unresolved_calls_preserves_specific_go_unsupported_reasons() {
        let (mut db, file, caller) = db_with_function(Language::Go, "flow.go");
        let unsupported = vec![
            unsupported(&db, 1, Language::Go, file, "interface dispatch"),
            unsupported(&db, 2, Language::Go, file, "reflect"),
            unsupported(&db, 3, Language::Go, file, "go_statement"),
            unsupported(&db, 4, Language::Go, file, "setup missing package"),
            unsupported(&db, 5, Language::Go, file, "unsupported syntax"),
        ];
        replace_unsupported(&mut db, Language::Go, file, caller, unsupported);
        let sites = vec![site(
            &db,
            Language::Go,
            file,
            caller,
            0,
            CallCallee::FunctionValue { place: PlaceId(1) },
            CallSyntaxKind::FunctionValue,
        )];

        let reasons = super::derive_unresolved_calls(&db, &sites)
            .into_iter()
            .map(|row| row.reason)
            .collect::<BTreeSet<_>>();

        assert!(reasons.contains(&UnresolvedCallReason::InterfaceDispatch));
        assert!(reasons.contains(&UnresolvedCallReason::Reflection));
        assert!(reasons.contains(&UnresolvedCallReason::GoroutineBoundary));
        assert!(reasons.contains(&UnresolvedCallReason::SetupMissing));
        assert!(reasons.contains(&UnresolvedCallReason::UnsupportedSyntax));
    }
}
