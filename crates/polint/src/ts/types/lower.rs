use std::collections::BTreeMap;

use crate::analysis_api::FactDatabase;
use crate::internal_core::{FileId, Span, StableKeyInterner};
use crate::ts::types::facts::{
    TsTypeCallStatus, TsTypeCallableFact, TsTypeCallableId, TsTypeCallableKind, TsTypeCalleeFact,
    TsTypeCalleeId, TsTypeCallsiteFact, TsTypeCallsiteId, TsTypeDispatch, TsTypeFileDensityFact,
    TsTypeFileDensityId, TsTypeProjectErrorFact, TsTypeProjectErrorId, TsTypeProjectFact,
    TsTypeProjectId, TsTypeReceiverFact, TsTypeReceiverId,
};
use crate::ts::types::protocol::{TsTypesOutput, TsTypesRawFrame, TsTypesSpan};
use crate::ts::types::store::TsTypesFactsOutput;
use crate::ts::types::validate::validate_relative_path;

/// Rows the sidecar emitted that this scan does not own.
///
/// The sidecar builds a whole TypeScript program, so it can report declarations
/// in files a narrower scan never discovered — a path-argument scan of two
/// files still sees the callees they resolve to. Those rows are ordinary input,
/// not corruption: they are skipped and counted, never raised.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TsTypesLowerReport {
    pub(crate) out_of_scope_rows: usize,
}

struct LoweredLocation {
    relative_file: Option<String>,
    file: Option<FileId>,
    span: Option<Span>,
    name_span: Option<Span>,
}

pub(crate) fn lower_ts_types(
    db: &dyn FactDatabase,
    output: &TsTypesOutput,
) -> (TsTypesFactsOutput, TsTypesLowerReport) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let files = db
        .files()
        .iter()
        .filter(|file| file.language.is_ts_family())
        .map(|file| (file.relative_path.as_str(), file.id))
        .collect::<BTreeMap<_, _>>();

    let mut lowered = TsTypesFactsOutput::default();
    let mut report = TsTypesLowerReport::default();

    for row in &output.rows {
        match row.kind.as_str() {
            "project" => lowered.projects.push(lower_project(interner, row)),
            "callable" => push_in_scope(
                &mut lowered.callables,
                lower_callable(interner, row, &files),
                &mut report,
            ),
            "callsite" => push_in_scope(
                &mut lowered.callsites,
                lower_callsite(interner, row, &files),
                &mut report,
            ),
            // A callee row is keyed by its call site, not by its own file: a
            // declaration outside the scan is still the answer to an in-scope
            // call, so the row is kept with an absent file rather than dropped.
            "callee" => lowered.callees.push(lower_callee(interner, row, &files)),
            "receiver" => lowered.receivers.push(lower_receiver(interner, row)),
            "any_density" => push_in_scope(
                &mut lowered.file_densities,
                lower_file_density(interner, row, &files),
                &mut report,
            ),
            "diagnostic" => lowered
                .project_errors
                .push(lower_project_error(interner, row)),
            _ => {}
        }
    }

    if report.out_of_scope_rows > 0 {
        tracing::debug!(
            rows = report.out_of_scope_rows,
            "skipped TS type rows naming files outside the scan scope"
        );
    }

    (lowered.normalized(interner), report)
}

fn push_in_scope<T>(target: &mut Vec<T>, fact: Option<T>, report: &mut TsTypesLowerReport) {
    match fact {
        Some(fact) => target.push(fact),
        None => report.out_of_scope_rows += 1,
    }
}

fn lower_project(interner: &StableKeyInterner, row: &TsTypesRawFrame) -> TsTypeProjectFact {
    TsTypeProjectFact {
        id: TsTypeProjectId(0),
        stable_key: interner.intern(row.stable_key_text.as_str()),
        project: row.project.clone(),
        options_digest: row.options_digest.clone(),
        typescript_version: row.typescript_version.clone(),
        file_count: row.file_count,
    }
}

fn lower_callable(
    interner: &StableKeyInterner,
    row: &TsTypesRawFrame,
    files: &BTreeMap<&str, FileId>,
) -> Option<TsTypeCallableFact> {
    let location = lower_location(row, files)?;
    Some(TsTypeCallableFact {
        id: TsTypeCallableId(0),
        stable_key: interner.intern(row.stable_key_text.as_str()),
        project: row.project.clone(),
        callable: row.callable.clone(),
        name: row.name.clone(),
        kind: callable_kind(row.callable_kind.as_str()),
        relative_file: location.relative_file,
        file: location.file,
        span: location.span,
        name_span: location.name_span,
    })
}

fn lower_callsite(
    interner: &StableKeyInterner,
    row: &TsTypesRawFrame,
    files: &BTreeMap<&str, FileId>,
) -> Option<TsTypeCallsiteFact> {
    let location = lower_location(row, files)?;
    Some(TsTypeCallsiteFact {
        id: TsTypeCallsiteId(0),
        stable_key: interner.intern(row.stable_key_text.as_str()),
        project: row.project.clone(),
        callsite: row.callsite.clone(),
        enclosing: non_empty(row.enclosing.as_str()),
        call_kind: row.call_kind.clone(),
        status: call_status(row.status.as_str()),
        reason: non_empty(row.reason.as_str()),
        relative_file: location.relative_file,
        file: location.file,
        span: location.span,
    })
}

fn lower_callee(
    interner: &StableKeyInterner,
    row: &TsTypesRawFrame,
    files: &BTreeMap<&str, FileId>,
) -> TsTypeCalleeFact {
    let location = lower_location(row, files).unwrap_or(LoweredLocation {
        relative_file: None,
        file: None,
        span: None,
        name_span: None,
    });
    TsTypeCalleeFact {
        id: TsTypeCalleeId(0),
        stable_key: interner.intern(row.stable_key_text.as_str()),
        project: row.project.clone(),
        callsite_stable_key: interner.intern(row.callsite_stable_key_text.as_str()),
        callable: non_empty(row.callable.as_str()),
        external: non_empty(row.external.as_str()),
        dispatch: dispatch_kind(row.dispatch.as_str()),
        relative_file: location.relative_file,
        file: location.file,
        span: location.span,
    }
}

fn lower_receiver(interner: &StableKeyInterner, row: &TsTypesRawFrame) -> TsTypeReceiverFact {
    TsTypeReceiverFact {
        id: TsTypeReceiverId(0),
        stable_key: interner.intern(row.stable_key_text.as_str()),
        project: row.project.clone(),
        callsite_stable_key: interner.intern(row.callsite_stable_key_text.as_str()),
        printed: row.printed.clone(),
        is_any: row.is_any,
        is_unknown: row.is_unknown,
        union_size: row.union_size,
    }
}

fn lower_file_density(
    interner: &StableKeyInterner,
    row: &TsTypesRawFrame,
    files: &BTreeMap<&str, FileId>,
) -> Option<TsTypeFileDensityFact> {
    if row.file.is_empty() || validate_relative_path(Some(row.file.as_str())).is_err() {
        return None;
    }
    let &file = files.get(row.file.as_str())?;
    Some(TsTypeFileDensityFact {
        id: TsTypeFileDensityId(0),
        stable_key: interner.intern(row.stable_key_text.as_str()),
        project: row.project.clone(),
        relative_file: row.file.clone(),
        file: Some(file),
        callsites: row.callsites,
        any_receivers: row.any_receivers,
    })
}

fn lower_project_error(
    interner: &StableKeyInterner,
    row: &TsTypesRawFrame,
) -> TsTypeProjectErrorFact {
    let relative_file = non_empty(row.file.as_str())
        .filter(|path| validate_relative_path(Some(path.as_str())).is_ok());
    TsTypeProjectErrorFact {
        id: TsTypeProjectErrorId(0),
        // Diagnostic rows carry no stable key on the wire: their identity is
        // the category, path and message they report.
        stable_key: interner.intern(crate::analysis_api::stable_key_text_from_parts(
            crate::analysis_api::FactFamily::TsTypes,
            &[
                ("category", row.category.clone()),
                ("file", row.file.clone()),
                ("message", row.message.clone()),
            ],
        )),
        category: row.category.clone(),
        relative_file,
        message: row.message.clone(),
    }
}

fn lower_location(
    row: &TsTypesRawFrame,
    files: &BTreeMap<&str, FileId>,
) -> Option<LoweredLocation> {
    if row.file.is_empty() {
        return None;
    }
    if validate_relative_path(Some(row.file.as_str())).is_err() {
        return None;
    }
    let &file = files.get(row.file.as_str())?;
    Some(LoweredLocation {
        relative_file: Some(row.file.clone()),
        file: Some(file),
        span: row.span.as_ref().map(|span| to_span(file, span)),
        name_span: row.name_span.as_ref().map(|span| to_span(file, span)),
    })
}

fn to_span(file: FileId, span: &TsTypesSpan) -> Span {
    Span::new(
        file,
        span.start_byte,
        span.end_byte,
        span.start_line,
        span.start_col,
        span.end_line,
        span.end_col,
    )
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn callable_kind(kind: &str) -> TsTypeCallableKind {
    match kind {
        "method" => TsTypeCallableKind::Method,
        "constructor" => TsTypeCallableKind::Constructor,
        "arrow" => TsTypeCallableKind::Arrow,
        "getter" => TsTypeCallableKind::Getter,
        "setter" => TsTypeCallableKind::Setter,
        "class" => TsTypeCallableKind::Class,
        _ => TsTypeCallableKind::Function,
    }
}

fn call_status(status: &str) -> TsTypeCallStatus {
    match status {
        "resolved" => TsTypeCallStatus::Resolved,
        "union" => TsTypeCallStatus::Union,
        "external" => TsTypeCallStatus::External,
        "any_receiver" => TsTypeCallStatus::AnyReceiver,
        _ => TsTypeCallStatus::Unresolved,
    }
}

/// An unrecognized dispatch label is treated as a declaration-only answer.
///
/// That is the conservative direction: a label this build does not know about
/// contributes no typed edge instead of contributing an unranked one.
fn dispatch_kind(dispatch: &str) -> TsTypeDispatch {
    match dispatch {
        "declared" => TsTypeDispatch::Declared,
        "implementation" => TsTypeDispatch::Implementation,
        "union_member" => TsTypeDispatch::UnionMember,
        _ => TsTypeDispatch::DeclaredSignature,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::ts::types::protocol::decode_ndjson_str;

    fn db_with_app_file() -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        db.add_file(
            "src/app.ts".into(),
            "src/app.ts".to_string(),
            "export function run() {}\n".to_string(),
        );
        db
    }

    fn framed(rows: &[&str]) -> String {
        let mut text = String::from(
            "{\"schema\":\"polint-ts-types-1\",\"kind\":\"session_begin\",\
             \"typescript_version\":\"5.9.3\"}\n",
        );
        for row in rows {
            text.push_str(row);
            text.push('\n');
        }
        text.push_str("{\"schema\":\"polint-ts-types-1\",\"kind\":\"session_end\"}\n");
        text
    }

    #[test]
    fn a_callsite_in_a_discovered_file_lowers_with_its_span() {
        let db = db_with_app_file();
        let output = decode_ndjson_str(&framed(&[
            "{\"schema\":\"polint-ts-types-1\",\"kind\":\"callsite\",\"project\":\"tsconfig.json\",\
             \"callsite\":\"src/app.ts:10\",\"file\":\"src/app.ts\",\"call_kind\":\"call\",\
             \"status\":\"resolved\",\"stable_key\":\"site\",\
             \"span\":{\"start_byte\":10,\"end_byte\":15,\"start_line\":1,\"start_column\":11,\
             \"end_line\":1,\"end_column\":16}}",
        ]))
        .expect("decodes");

        let (lowered, report) = lower_ts_types(&db, &output);

        assert_eq!(lowered.callsites.len(), 1);
        assert_eq!(lowered.callsites[0].status, TsTypeCallStatus::Resolved);
        let span = lowered.callsites[0].span.as_ref().expect("span");
        assert_eq!(span.start_byte, 10);
        assert_eq!(span.end_byte, 15);
        assert_eq!(report.out_of_scope_rows, 0);
    }

    #[test]
    fn a_row_naming_an_undiscovered_file_is_skipped_and_counted() {
        let db = db_with_app_file();
        let output = decode_ndjson_str(&framed(&[
            "{\"schema\":\"polint-ts-types-1\",\"kind\":\"callsite\",\"project\":\"tsconfig.json\",\
             \"callsite\":\"src/other.ts:10\",\"file\":\"src/other.ts\",\"status\":\"resolved\",\
             \"stable_key\":\"site\"}",
        ]))
        .expect("decodes");

        let (lowered, report) = lower_ts_types(&db, &output);

        assert!(lowered.callsites.is_empty());
        assert_eq!(report.out_of_scope_rows, 1);
    }

    #[test]
    fn a_row_whose_path_escapes_the_repository_is_skipped() {
        let db = db_with_app_file();
        let output = decode_ndjson_str(&framed(&[
            "{\"schema\":\"polint-ts-types-1\",\"kind\":\"callsite\",\"project\":\"tsconfig.json\",\
             \"file\":\"../outside/app.ts\",\"status\":\"resolved\",\"stable_key\":\"site\"}",
        ]))
        .expect("decodes");

        let (lowered, report) = lower_ts_types(&db, &output);

        assert!(lowered.callsites.is_empty());
        assert_eq!(report.out_of_scope_rows, 1);
    }

    #[test]
    fn an_external_callee_keeps_its_moniker_with_no_file() {
        let db = db_with_app_file();
        let output = decode_ndjson_str(&framed(&[
            "{\"schema\":\"polint-ts-types-1\",\"kind\":\"callee\",\"project\":\"tsconfig.json\",\
             \"callsite_stable_key\":\"site\",\"external\":\"node_modules:express/index.d.ts#Router\",\
             \"dispatch\":\"declared\",\"stable_key\":\"callee\"}",
        ]))
        .expect("decodes");

        let (lowered, _) = lower_ts_types(&db, &output);

        assert_eq!(lowered.callees.len(), 1);
        assert_eq!(
            lowered.callees[0].external.as_deref(),
            Some("node_modules:express/index.d.ts#Router")
        );
        assert!(lowered.callees[0].file.is_none());
        assert_eq!(lowered.callees[0].dispatch, TsTypeDispatch::Declared);
    }

    #[test]
    fn an_unknown_dispatch_label_lowers_to_a_declaration_only_answer() {
        assert_eq!(
            dispatch_kind("future_tier"),
            TsTypeDispatch::DeclaredSignature
        );
        assert_eq!(dispatch_kind(""), TsTypeDispatch::DeclaredSignature);
    }

    #[test]
    fn an_unknown_status_lowers_to_unresolved() {
        assert_eq!(call_status("something_new"), TsTypeCallStatus::Unresolved);
    }

    #[test]
    fn density_rows_carry_the_counts_the_gates_read() {
        let db = db_with_app_file();
        let output = decode_ndjson_str(&framed(&[
            "{\"schema\":\"polint-ts-types-1\",\"kind\":\"any_density\",\
             \"project\":\"tsconfig.json\",\"file\":\"src/app.ts\",\"callsites\":8,\
             \"any_receivers\":4,\"stable_key\":\"density\"}",
        ]))
        .expect("decodes");

        let (lowered, _) = lower_ts_types(&db, &output);

        assert_eq!(lowered.file_densities.len(), 1);
        assert_eq!(lowered.file_densities[0].any_percent(), 50);
    }

    #[test]
    fn a_diagnostic_row_becomes_a_project_error_with_a_derived_key() {
        let db = db_with_app_file();
        let output = decode_ndjson_str(&framed(&[
            "{\"schema\":\"polint-ts-types-1\",\"kind\":\"diagnostic\",\
             \"category\":\"project_error\",\"file\":\"tsconfig.json\",\
             \"message\":\"no inputs were found\"}",
        ]))
        .expect("decodes");

        let (lowered, _) = lower_ts_types(&db, &output);

        assert_eq!(lowered.project_errors.len(), 1);
        assert_eq!(lowered.project_errors[0].category, "project_error");
        assert!(!lowered.project_errors[0].message.is_empty());
    }
}
