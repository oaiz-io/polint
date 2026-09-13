use std::collections::BTreeMap;

use crate::analysis_api::FactDatabase;
use crate::analysis_api::FactFamily;
use crate::go::semantic::facts::{
    GoSemanticAddressTakenFact, GoSemanticAddressTakenId, GoSemanticCallStatus,
    GoSemanticCallsiteFact, GoSemanticCallsiteId, GoSemanticDynamicDispatchFact,
    GoSemanticDynamicDispatchId, GoSemanticFunctionFact, GoSemanticFunctionId,
    GoSemanticFunctionKind, GoSemanticInstantiatedTypeFact, GoSemanticInstantiatedTypeId,
    GoSemanticMethodSetFact, GoSemanticMethodSetId, GoSemanticPackageErrorFact,
    GoSemanticPackageErrorId, GoSemanticPackageFact, GoSemanticPackageId, GoSemanticRtaEdgeFact,
    GoSemanticRtaEdgeId,
};
use crate::go::semantic::protocol::{GoSemanticOutput, GoSemanticRawFrame, GoSemanticSpan};
use crate::go::semantic::store::GoSemanticFactsOutput;
use crate::go::semantic::validate::validate_relative_path;
use crate::go::stable_key::semantic_stable_key;
use crate::internal_core::{FileId, Language, Span};

struct LoweredLocation {
    relative_file: Option<String>,
    file: Option<FileId>,
    span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GoSemanticLowerError {
    InvalidPath(String),
}

impl std::fmt::Display for GoSemanticLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath(reason) => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for GoSemanticLowerError {}

pub(crate) fn lower_go_semantic(
    db: &dyn FactDatabase,
    output: &GoSemanticOutput,
) -> Result<GoSemanticFactsOutput, GoSemanticLowerError> {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let files = db
        .files()
        .iter()
        .filter(|file| file.language == Language::Go)
        .map(|file| (file.relative_path.as_str(), file.id))
        .collect::<BTreeMap<_, _>>();
    let mut lowered = GoSemanticFactsOutput::default();
    // The sidecar loads WHOLE packages (`package_patterns`), so it routinely reports files a
    // narrower scan never discovered — a PATH-argument scan of two files still gets rows for
    // every file of every loaded package. Those rows are ordinary input, not corruption: they
    // are skipped and counted, never raised. The scope reduction is already visible through
    // the `polint/scope` diagnostic and the run summary.
    let mut out_of_scope_rows = 0usize;

    for row in &output.rows {
        match row.kind.as_str() {
            "package" => push_in_scope(
                &mut lowered.packages,
                lower_package(interner, row, &files)?,
                &mut out_of_scope_rows,
            ),
            "function" | "method" | "init_function" => push_in_scope(
                &mut lowered.functions,
                lower_function(interner, row, &files)?,
                &mut out_of_scope_rows,
            ),
            "callsite" => push_in_scope(
                &mut lowered.callsites,
                lower_callsite(interner, row, &files)?,
                &mut out_of_scope_rows,
            ),
            "method_set" => lowered.method_sets.push(lower_method_set(interner, row)),
            "address_taken" => lowered
                .address_taken
                .push(lower_address_taken(interner, row)),
            "instantiated_type" => {
                lowered
                    .instantiated_types
                    .push(lower_instantiated_type(interner, row));
            }
            "dynamic_dispatch" => lowered
                .dynamic_dispatch
                .push(lower_dynamic_dispatch(interner, row)),
            "rta_edge" => lowered.rta_edges.push(lower_rta_edge(interner, row)),
            "package_error" => lowered
                .package_errors
                .push(lower_package_error(interner, row)),
            "receiver_type" | "unsupported" | "type_fact" => {}
            _ => {}
        }
    }

    if out_of_scope_rows > 0 {
        tracing::debug!(
            rows = out_of_scope_rows,
            "skipped Go semantic rows naming files outside the scan scope"
        );
    }

    Ok(lowered.normalized(interner))
}

/// Collect a lowered fact, or count the row as out of scope when lowering returned `None`.
fn push_in_scope<T>(target: &mut Vec<T>, fact: Option<T>, out_of_scope_rows: &mut usize) {
    match fact {
        Some(fact) => target.push(fact),
        None => *out_of_scope_rows += 1,
    }
}

/// Lower a `package` row, keeping only the files this scan discovered.
///
/// The discovered-file map is the only trust anchor: every path it does not contain is out of
/// scope and is dropped, whatever shape the path has. That includes paths escaping the
/// repository — `--tests` makes x/tools report a synthesized `<pkg>.test` main package whose
/// sole "file" is a GOCACHE build artifact, reached by climbing out of the root — so those are
/// dropped like any other undiscovered path rather than raised. A row that named files but
/// kept none has no in-scope content left to contribute and is skipped entirely (`Ok(None)`);
/// a row that named no files at all is unaffected, as a location-less function row is.
fn lower_package(
    interner: &crate::internal_core::StableKeyInterner,
    row: &GoSemanticRawFrame,
    files: &BTreeMap<&str, FileId>,
) -> Result<Option<GoSemanticPackageFact>, GoSemanticLowerError> {
    let mut in_scope = Vec::with_capacity(row.files.len());
    for file in &row.files {
        if validate_relative_path(file).is_ok() && files.contains_key(file.as_str()) {
            in_scope.push(file.clone());
        }
    }
    if in_scope.is_empty() && !row.files.is_empty() {
        return Ok(None);
    }
    Ok(Some(GoSemanticPackageFact {
        id: GoSemanticPackageId(0),
        stable_key: row_stable_key(interner, row, "package"),
        package_id: row.package_id.clone(),
        package_path: row.package_path.clone(),
        package_name: row.package_name.clone(),
        module_path: row.module_path.clone(),
        files: in_scope,
    }))
}

fn lower_function(
    interner: &crate::internal_core::StableKeyInterner,
    row: &GoSemanticRawFrame,
    files: &BTreeMap<&str, FileId>,
) -> Result<Option<GoSemanticFunctionFact>, GoSemanticLowerError> {
    let Some(location) = lower_optional_file_span(row, files)? else {
        return Ok(None);
    };
    Ok(Some(GoSemanticFunctionFact {
        id: GoSemanticFunctionId(0),
        stable_key: row_stable_key(interner, row, row.kind.as_str()),
        package_id: row.package_id.clone(),
        package_path: row.package_path.clone(),
        name: row.name.clone(),
        qualified: row.qualified.clone(),
        signature: row.signature.clone(),
        kind: match row.kind.as_str() {
            "method" => GoSemanticFunctionKind::Method,
            "init_function" => GoSemanticFunctionKind::Init,
            _ => GoSemanticFunctionKind::Function,
        },
        receiver: non_empty(row.receiver.as_str()),
        relative_file: location.relative_file,
        file: location.file,
        span: location.span,
    }))
}

fn lower_callsite(
    interner: &crate::internal_core::StableKeyInterner,
    row: &GoSemanticRawFrame,
    files: &BTreeMap<&str, FileId>,
) -> Result<Option<GoSemanticCallsiteFact>, GoSemanticLowerError> {
    let Some(location) = lower_optional_file_span(row, files)? else {
        return Ok(None);
    };
    Ok(Some(GoSemanticCallsiteFact {
        id: GoSemanticCallsiteId(0),
        stable_key: row_stable_key(interner, row, "callsite"),
        package_id: row.package_id.clone(),
        package_path: row.package_path.clone(),
        caller: row.caller.clone(),
        static_callee: non_empty(row.static_callee.as_str()),
        status: match row.status.as_str() {
            "resolved_static" => GoSemanticCallStatus::ResolvedStatic,
            "unsupported" => GoSemanticCallStatus::Unsupported,
            _ => GoSemanticCallStatus::UnresolvedDynamic,
        },
        reason: non_empty(row.reason.as_str()),
        relative_file: location.relative_file,
        file: location.file,
        span: location.span,
    }))
}

fn lower_method_set(
    interner: &crate::internal_core::StableKeyInterner,
    row: &GoSemanticRawFrame,
) -> GoSemanticMethodSetFact {
    GoSemanticMethodSetFact {
        id: GoSemanticMethodSetId(0),
        // FINDING C: a `method_set` row's identity is its `type_name`, which is ABSENT from
        // the `row_stable_key` fallback recipe — so the fallback would collapse all of a
        // package's types to one key (WR-03). Use the sidecar-provided stable_key VERBATIM
        // (no fabricated fallback); a stable-key-less harvest row is dropped at the store
        // boundary (`drop_invalid_harvest_rows`), not fatal (FINDING B).
        stable_key: harvest_stable_key(interner, row),
        package_id: row.package_id.clone(),
        package_path: row.package_path.clone(),
        type_name: row.type_name.clone(),
        methods: row.methods.clone(),
    }
}

fn lower_address_taken(
    interner: &crate::internal_core::StableKeyInterner,
    row: &GoSemanticRawFrame,
) -> GoSemanticAddressTakenFact {
    GoSemanticAddressTakenFact {
        id: GoSemanticAddressTakenId(0),
        stable_key: harvest_stable_key(interner, row),
        package_id: row.package_id.clone(),
        package_path: row.package_path.clone(),
        function: row.function.clone(),
    }
}

fn lower_instantiated_type(
    interner: &crate::internal_core::StableKeyInterner,
    row: &GoSemanticRawFrame,
) -> GoSemanticInstantiatedTypeFact {
    GoSemanticInstantiatedTypeFact {
        id: GoSemanticInstantiatedTypeId(0),
        stable_key: harvest_stable_key(interner, row),
        package_id: row.package_id.clone(),
        package_path: row.package_path.clone(),
        type_name: row.type_name.clone(),
    }
}

fn lower_dynamic_dispatch(
    interner: &crate::internal_core::StableKeyInterner,
    row: &GoSemanticRawFrame,
) -> GoSemanticDynamicDispatchFact {
    GoSemanticDynamicDispatchFact {
        id: GoSemanticDynamicDispatchId(0),
        stable_key: harvest_stable_key(interner, row),
        package_id: row.package_id.clone(),
        package_path: row.package_path.clone(),
        caller: row.caller.clone(),
        callsite_stable_key: interner.intern(row.callsite_stable_key_text.clone()),
        interface_type: non_empty(row.interface_type.as_str()),
        method: non_empty(row.method.as_str()),
        signature: non_empty(row.signature.as_str()),
    }
}

fn lower_rta_edge(
    interner: &crate::internal_core::StableKeyInterner,
    row: &GoSemanticRawFrame,
) -> GoSemanticRtaEdgeFact {
    GoSemanticRtaEdgeFact {
        id: GoSemanticRtaEdgeId(0),
        stable_key: harvest_stable_key(interner, row),
        package_id: row.package_id.clone(),
        package_path: row.package_path.clone(),
        caller: row.caller.clone(),
        callee: row.callee.clone(),
        edge_kind: row.edge_kind.clone(),
    }
}

fn lower_package_error(
    interner: &crate::internal_core::StableKeyInterner,
    row: &GoSemanticRawFrame,
) -> GoSemanticPackageErrorFact {
    GoSemanticPackageErrorFact {
        id: GoSemanticPackageErrorId(0),
        stable_key: row_stable_key(interner, row, "package_error"),
        package_id: row.package_id.clone(),
        package_path: row.package_path.clone(),
        message: row.message.clone(),
    }
}

/// Resolve a row's optional `file` against the discovered files.
///
/// `Ok(None)` means the row names a path this scan did not discover — it belongs outside the
/// scan scope and its caller skips it. A path that escapes the repository takes the same
/// route: it can never be a discovered file, so it is undiscoverable by definition rather
/// than an error. A row with no file at all is unaffected (it lowers to a location-less
/// fact).
fn lower_optional_file_span(
    row: &GoSemanticRawFrame,
    files: &BTreeMap<&str, FileId>,
) -> Result<Option<LoweredLocation>, GoSemanticLowerError> {
    if row.file.is_empty() {
        return Ok(Some(LoweredLocation {
            relative_file: None,
            file: None,
            span: None,
        }));
    }
    if validate_relative_path(&row.file).is_err() {
        return Ok(None);
    }
    let Some(&file) = files.get(row.file.as_str()) else {
        return Ok(None);
    };
    let span = row.span.as_ref().map(|span| to_span(file, span));
    Ok(Some(LoweredLocation {
        relative_file: Some(row.file.clone()),
        file: Some(file),
        span,
    }))
}

fn to_span(file: FileId, span: &GoSemanticSpan) -> Span {
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

/// The sidecar-provided `stable_key` for an RTA-signal harvest row (`method_set` /
/// `address_taken` / `instantiated_type` / `dynamic_dispatch`), used VERBATIM with no
/// fabricated fallback (FINDING C / WR-03). These rows are machine-generated and `emit.go`
/// emits a stable_key for each; their discriminating identity lives in fields (`function` /
/// `type_name`) the `row_stable_key` fallback recipe does NOT include, so a fabricated
/// fallback would collide distinct facts and the set-dedup in `store` would silently drop a
/// real member. A row that nonetheless lacks a stable_key carries no resolvable identity and
/// is DROPPED at the store boundary ([`super::store::GoSemanticFactsOutput`]
/// `drop_invalid_harvest_rows`) — a single malformed harvest row must not nuke the whole Go
/// fact set (FINDING B), so this returns the (possibly empty) key rather than failing the
/// entire lowering.
fn harvest_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    row: &GoSemanticRawFrame,
) -> crate::internal_core::StableKeyId {
    interner.intern(row.stable_key_text.clone())
}

fn row_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    row: &GoSemanticRawFrame,
    kind: &str,
) -> crate::internal_core::StableKeyId {
    if !row.stable_key_text.is_empty() {
        return interner.intern(row.stable_key_text.clone());
    }
    interner.intern(
        semantic_stable_key(
            FactFamily::SemanticImport,
            &[
                ("go_kind", kind.to_string()),
                ("package", row.package_path.clone()),
                ("name", row.qualified.clone()),
                ("file", row.file.clone()),
                ("caller", row.caller.clone()),
                ("message", row.message.clone()),
            ],
        )
        .into_string(),
    )
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::go::semantic::protocol::{GO_SEMANTIC_SCHEMA, decode_ndjson_str};
    use std::path::PathBuf;

    #[test]
    fn lower_accepts_in_repository_path() {
        let db = db_with_go_file("main.go");
        let output = decode_ndjson_str(
            r#"{"schema":"polint-go-semantic-3","kind":"session_begin"}
{"schema":"polint-go-semantic-3","kind":"function","package_id":"example.com/p","package_path":"example.com/p","name":"F","qualified":"example.com/p.F","stable_key":"fn","file":"main.go","span":{"start_byte":1,"end_byte":2,"start_line":1,"start_column":1,"end_line":1,"end_column":2}}
{"schema":"polint-go-semantic-3","kind":"session_end"}
"#,
        )
        .expect("valid protocol");
        let lowered = lower_go_semantic(&db, &output).expect("lowered");
        assert_eq!(lowered.functions[0].file, Some(FileId::from_raw(0)));
    }

    #[test]
    fn lower_harvests_rta_signal_rows() {
        let db = db_with_go_file("main.go");
        let output = decode_ndjson_str(
            r#"{"schema":"polint-go-semantic-3","kind":"session_begin"}
{"schema":"polint-go-semantic-3","kind":"address_taken","package_id":"example.com/p","package_path":"example.com/p","function":"example.com/p.F","stable_key":"at"}
{"schema":"polint-go-semantic-3","kind":"instantiated_type","package_id":"example.com/p","package_path":"example.com/p","type":"example.com/p.T","stable_key":"it"}
{"schema":"polint-go-semantic-3","kind":"dynamic_dispatch","package_id":"example.com/p","package_path":"example.com/p","caller":"example.com/p.call","callsite_stable_key":"cs","interface_type":"example.com/p.I","method":"M","stable_key":"dd"}
{"schema":"polint-go-semantic-3","kind":"rta_edge","package_id":"example.com/p","package_path":"example.com/p","caller":"main","callee":"init$1","edge_kind":"dynamic function call","stable_key":"rta"}
{"schema":"polint-go-semantic-3","kind":"session_end"}
"#,
        )
        .expect("valid protocol");
        let lowered = lower_go_semantic(&db, &output).expect("lowered");
        assert_eq!(lowered.address_taken[0].function, "example.com/p.F");
        assert_eq!(lowered.instantiated_types[0].type_name, "example.com/p.T");
        assert_eq!(
            lowered.dynamic_dispatch[0].interface_type.as_deref(),
            Some("example.com/p.I")
        );
        assert_eq!(lowered.dynamic_dispatch[0].method.as_deref(), Some("M"));
        assert_eq!(lowered.dynamic_dispatch[0].signature, None);
        assert_eq!(
            db.stable_key_interner()
                .resolve(lowered.dynamic_dispatch[0].callsite_stable_key)
                .as_ref(),
            "cs"
        );
        assert_eq!(lowered.rta_edges[0].caller, "main");
        assert_eq!(lowered.rta_edges[0].callee, "init$1");
        assert_eq!(lowered.rta_edges[0].edge_kind, "dynamic function call");
    }

    #[test]
    fn lower_harvest_row_missing_stable_key_passes_through_for_store_to_drop() {
        // FINDING B/C: a machine-generated RTA-signal row should carry a stable_key, but a
        // missing one must NOT fail the WHOLE lowering (that would zero every function /
        // callsite / root from the valid rows). Lowering uses the sidecar key VERBATIM (no
        // fabricated colliding fallback, WR-03); the stable-key-less row is produced with an
        // empty key and DROPPED at the store boundary (`drop_invalid_harvest_rows`), not
        // fatal here.
        let db = db_with_go_file("main.go");
        let output = decode_ndjson_str(
            r#"{"schema":"polint-go-semantic-3","kind":"session_begin"}
{"schema":"polint-go-semantic-3","kind":"address_taken","package_id":"example.com/p","package_path":"example.com/p","function":"example.com/p.F"}
{"schema":"polint-go-semantic-3","kind":"session_end"}
"#,
        )
        .expect("valid protocol");
        let lowered = lower_go_semantic(&db, &output).expect("lowering does not fail");
        // The row is produced with an empty stable_key (no fabricated fallback).
        assert_eq!(lowered.address_taken.len(), 1);
        assert!(
            db.stable_key_interner()
                .resolve(lowered.address_taken[0].stable_key)
                .is_empty()
        );
        // The store drops it (the bad row) without touching valid facts.
        let store = crate::go::semantic::store::GoSemanticStore::from_output(
            lowered,
            &db.stable_key_interner(),
        )
        .expect("store drops the bad harvest row");
        assert!(store.output().address_taken.is_empty());
    }

    #[test]
    fn lower_instantiated_type_missing_stable_key_passes_through_for_store_to_drop() {
        // FINDING B/C: same row-resilient contract for the instantiated_type harvest row.
        let db = db_with_go_file("main.go");
        let output = decode_ndjson_str(
            r#"{"schema":"polint-go-semantic-3","kind":"session_begin"}
{"schema":"polint-go-semantic-3","kind":"instantiated_type","package_id":"example.com/p","package_path":"example.com/p","type":"example.com/p.T"}
{"schema":"polint-go-semantic-3","kind":"session_end"}
"#,
        )
        .expect("valid protocol");
        let lowered = lower_go_semantic(&db, &output).expect("lowering does not fail");
        assert_eq!(lowered.instantiated_types.len(), 1);
        assert!(
            db.stable_key_interner()
                .resolve(lowered.instantiated_types[0].stable_key)
                .is_empty()
        );
        let store = crate::go::semantic::store::GoSemanticStore::from_output(
            lowered,
            &db.stable_key_interner(),
        )
        .expect("store drops the bad harvest row");
        assert!(store.output().instantiated_types.is_empty());
    }

    #[test]
    fn lower_method_set_uses_sidecar_key_verbatim_without_fabricated_fallback() {
        // FINDING C: method_set must route through the sidecar key VERBATIM, NOT the
        // `row_stable_key` fallback (whose recipe omits `type_name`, collapsing all of a
        // package's types onto one key). With a stable_key present, it is used as-is.
        let db = db_with_go_file("main.go");
        let output = decode_ndjson_str(
            r#"{"schema":"polint-go-semantic-3","kind":"session_begin"}
{"schema":"polint-go-semantic-3","kind":"method_set","package_id":"example.com/p","package_path":"example.com/p","type":"example.com/p.T","methods":["M"],"stable_key":"ms|example.com/p.T"}
{"schema":"polint-go-semantic-3","kind":"session_end"}
"#,
        )
        .expect("valid protocol");
        let lowered = lower_go_semantic(&db, &output).expect("lowered");
        assert_eq!(lowered.method_sets.len(), 1);
        assert_eq!(
            db.stable_key_interner()
                .resolve(lowered.method_sets[0].stable_key)
                .as_ref(),
            "ms|example.com/p.T"
        );
    }

    #[test]
    fn lower_dynamic_dispatch_func_value_carries_signature() {
        let db = db_with_go_file("main.go");
        let output = decode_ndjson_str(
            r#"{"schema":"polint-go-semantic-3","kind":"session_begin"}
{"schema":"polint-go-semantic-3","kind":"dynamic_dispatch","package_id":"example.com/p","package_path":"example.com/p","caller":"example.com/p.apply","callsite_stable_key":"cs2","signature":"func()","stable_key":"dd2"}
{"schema":"polint-go-semantic-3","kind":"session_end"}
"#,
        )
        .expect("valid protocol");
        let lowered = lower_go_semantic(&db, &output).expect("lowered");
        assert_eq!(
            lowered.dynamic_dispatch[0].signature.as_deref(),
            Some("func()")
        );
        assert_eq!(lowered.dynamic_dispatch[0].interface_type, None);
        assert_eq!(lowered.dynamic_dispatch[0].method, None);
    }

    #[test]
    fn lower_skips_absolute_repo_escaping_path() {
        let db = db_with_go_file("main.go");
        let outside = if cfg!(windows) {
            r"C:\tmp\outside.go"
        } else {
            "/tmp/outside.go"
        };
        let outside_json = serde_json::to_string(outside).expect("path serializes as JSON");
        let output = decode_ndjson_str(&format!(
            r#"{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_begin"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"function","package_id":"example.com/p","package_path":"example.com/p","name":"F","qualified":"example.com/p.F","stable_key":"fn","file":{outside_json}}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_end"}}
"#,
        ))
        .expect("valid protocol");
        let lowered =
            lower_go_semantic(&db, &output).expect("a repo-escaping path is skipped, not fatal");
        assert!(lowered.functions.is_empty());
    }

    #[test]
    fn lower_skips_relative_repo_escaping_path() {
        // No path shape is fatal during lowering: lowering cannot tell a corrupted path from a
        // legitimate build artifact, and does not need to — the discovered-file map decides.
        // A path that climbs out of the repository can never be in that map, so the row is
        // skipped exactly like any other out-of-scope row, for both row shapes.
        let db = db_with_go_file("main.go");
        let output = decode_ndjson_str(&format!(
            r#"{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_begin"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"function","package_id":"example.com/p","package_path":"example.com/p","name":"F","qualified":"example.com/p.F","stable_key":"fn","file":"../secrets.go"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"callsite","package_id":"example.com/p","package_path":"example.com/p","caller":"example.com/p.F","static_callee":"example.com/p.G","status":"resolved_static","stable_key":"cs","file":"../secrets.go"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_end"}}
"#,
        ))
        .expect("valid protocol");
        let lowered =
            lower_go_semantic(&db, &output).expect("a repo-escaping path is skipped, not fatal");
        assert!(lowered.functions.is_empty());
        assert!(lowered.callsites.is_empty());

        let package_output = decode_ndjson_str(&format!(
            r#"{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_begin"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"package","package_id":"example.com/p","package_path":"example.com/p","stable_key":"pkg","files":["main.go","../secrets.go"]}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_end"}}
"#,
        ))
        .expect("valid protocol");
        let lowered = lower_go_semantic(&db, &package_output)
            .expect("a repo-escaping path is dropped from the file list, not fatal");
        assert_eq!(lowered.packages.len(), 1);
        assert_eq!(lowered.packages[0].files, vec!["main.go".to_string()]);
    }

    #[test]
    fn lower_skips_synthesized_test_package_naming_only_a_build_cache_artifact() {
        // With `--tests`, x/tools reports a SYNTHESIZED `<pkg>.test` main package whose files
        // list holds exactly one entry: a GOCACHE artifact, relative but climbing out of the
        // repository root. That is legitimate loader output, not corruption, and it must not
        // fail the run: the row names no discovered file, so it is skipped like any other
        // out-of-scope row and the real rows lower untouched.
        let db = db_with_go_file("main.go");
        let output = decode_ndjson_str(&format!(
            r#"{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_begin"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"package","package_id":"example.com/p","package_path":"example.com/p","stable_key":"pkg","files":["main.go"]}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"package","package_id":"example.com/p.test","package_path":"example.com/p.test","stable_key":"pkg_testmain","files":["../../home/.cache/go-build/b0/b0ddf933bee58efe09b1d2a18dfe72a91678e47e6e91bb9040231a8727b0573b-d"]}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"function","package_id":"example.com/p","package_path":"example.com/p","name":"F","qualified":"example.com/p.F","stable_key":"fn","file":"main.go"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_end"}}
"#,
        ))
        .expect("valid protocol");
        let lowered = lower_go_semantic(&db, &output)
            .expect("a synthesized testmain build-cache path is skipped, not fatal");
        assert_eq!(lowered.packages.len(), 1);
        assert_eq!(lowered.packages[0].package_path, "example.com/p");
        assert_eq!(lowered.packages[0].files, vec!["main.go".to_string()]);
        assert_eq!(lowered.functions.len(), 1);
        assert_eq!(lowered.functions[0].qualified, "example.com/p.F");
    }

    #[test]
    fn lower_skips_rows_naming_files_outside_the_scan_scope() {
        // A PATH-argument scan discovers only the named files, while the sidecar loads whole
        // packages — so rows for undiscovered files are normal input. They are skipped, not
        // raised: the whole Go fact set must survive a narrowed scope.
        let db = db_with_go_file("main.go");
        let output = decode_ndjson_str(&format!(
            r#"{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_begin"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"function","package_id":"example.com/p","package_path":"example.com/p","name":"F","qualified":"example.com/p.F","stable_key":"fn","file":"main.go"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"function","package_id":"example.com/p","package_path":"example.com/p","name":"G","qualified":"example.com/p.G","stable_key":"fn_out","file":"out_of_scope.go"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"callsite","package_id":"example.com/p","package_path":"example.com/p","caller":"example.com/p.G","static_callee":"example.com/p.F","status":"resolved_static","stable_key":"cs_out","file":"out_of_scope.go"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_end"}}
"#,
        ))
        .expect("valid protocol");
        let lowered = lower_go_semantic(&db, &output).expect("out-of-scope rows are not fatal");
        assert_eq!(lowered.functions.len(), 1);
        assert_eq!(lowered.functions[0].qualified, "example.com/p.F");
        assert!(lowered.callsites.is_empty());
    }

    #[test]
    fn lower_package_keeps_only_the_files_this_scan_discovered() {
        let db = db_with_go_file("main.go");
        let output = decode_ndjson_str(&format!(
            r#"{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_begin"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"package","package_id":"example.com/p","package_path":"example.com/p","stable_key":"pkg","files":["main.go","other.go"]}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_end"}}
"#,
        ))
        .expect("valid protocol");
        let lowered = lower_go_semantic(&db, &output).expect("out-of-scope files are not fatal");
        assert_eq!(lowered.packages.len(), 1);
        assert_eq!(lowered.packages[0].files, vec!["main.go".to_string()]);
    }

    #[test]
    fn lower_skips_package_with_no_in_scope_files() {
        let db = db_with_go_file("main.go");
        let output = decode_ndjson_str(&format!(
            r#"{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_begin"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"package","package_id":"example.com/dep","package_path":"example.com/dep","stable_key":"pkg_dep","files":["dep/a.go","dep/b.go"]}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_end"}}
"#,
        ))
        .expect("valid protocol");
        let lowered = lower_go_semantic(&db, &output).expect("out-of-scope package is not fatal");
        assert!(lowered.packages.is_empty());
    }

    #[test]
    fn lower_keeps_rows_without_a_file() {
        // Scope filtering keys on the row's file; a row that names none is untouched.
        let db = db_with_go_file("main.go");
        let output = decode_ndjson_str(&format!(
            r#"{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_begin"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"package","package_id":"example.com/p","package_path":"example.com/p","stable_key":"pkg"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"function","package_id":"example.com/p","package_path":"example.com/p","name":"F","qualified":"example.com/p.F","stable_key":"fn"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"callsite","package_id":"example.com/p","package_path":"example.com/p","caller":"example.com/p.F","static_callee":"example.com/p.G","status":"resolved_static","stable_key":"cs"}}
{{"schema":"{GO_SEMANTIC_SCHEMA}","kind":"session_end"}}
"#,
        ))
        .expect("valid protocol");
        let lowered = lower_go_semantic(&db, &output).expect("lowered");
        assert_eq!(lowered.packages.len(), 1);
        assert!(lowered.packages[0].files.is_empty());
        assert_eq!(lowered.functions.len(), 1);
        assert_eq!(lowered.functions[0].relative_file, None);
        assert_eq!(lowered.functions[0].file, None);
        assert_eq!(lowered.callsites.len(), 1);
        assert_eq!(lowered.callsites[0].relative_file, None);
    }

    #[test]
    fn lower_preserves_package_load_errors() {
        let db = db_with_go_file("main.go");
        let output = decode_ndjson_str(
            r#"{"schema":"polint-go-semantic-3","kind":"session_begin"}
{"schema":"polint-go-semantic-3","kind":"package_error","package_id":"example.com/p","package_path":"example.com/p","message":"load failed"}
{"schema":"polint-go-semantic-3","kind":"session_end"}
"#,
        )
        .expect("valid protocol");
        let lowered = lower_go_semantic(&db, &output).expect("lowered");
        assert_eq!(lowered.package_errors[0].message, "load failed");
    }

    #[test]
    fn lower_package_error_fallback_stable_key_includes_message() {
        let db = db_with_go_file("main.go");
        let output = decode_ndjson_str(
            r#"{"schema":"polint-go-semantic-3","kind":"session_begin"}
{"schema":"polint-go-semantic-3","kind":"package_error","package_id":"example.com/p","package_path":"example.com/p","message":"first"}
{"schema":"polint-go-semantic-3","kind":"package_error","package_id":"example.com/p","package_path":"example.com/p","message":"second"}
{"schema":"polint-go-semantic-3","kind":"session_end"}
"#,
        )
        .expect("valid protocol");
        let lowered = lower_go_semantic(&db, &output).expect("lowered");
        assert_ne!(
            lowered.package_errors[0].stable_key,
            lowered.package_errors[1].stable_key
        );
    }

    fn db_with_go_file(path: &str) -> crate::go::local_db::LocalFactDb {
        let mut db = crate::go::local_db::LocalFactDb::new();
        db.add_file(
            PathBuf::from(path),
            path.to_string(),
            "package main\n".to_string(),
        );
        db
    }
}
