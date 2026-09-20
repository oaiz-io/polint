use crate::analysis_api::{FactFamily, stable_key_from_key_parts, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::entrypoints::facts::{
    EntrypointFact, EntrypointKind, TrustBoundaryFact, TrustBoundarySourceKind,
};
use crate::analysis_neutral::entrypoints::provider::ENTRYPOINTS_PROVIDER_ID;
use crate::analysis_neutral::ids::TrustBoundaryId;
use crate::analysis_neutral::places::{PlaceFact, PlaceRoot};
use crate::internal_core::{KeyPart, Language, StableKeyId};

/// Derive trust boundary facts from recognized entrypoints.
///
/// Per D-03, D-19, D-20, D-21, D-22: trust boundaries are per-entrypoint,
/// per-source-kind facts. Each identifies a concrete source of untrusted data
/// entering through an entrypoint. Trust boundary precision follows entrypoint
/// precision per D-21.
pub fn derive_trust_boundaries(
    db: &impl AnalysisHost,
    entrypoints: &[EntrypointFact],
) -> Vec<TrustBoundaryFact> {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut boundaries = Vec::new();

    for entrypoint in entrypoints {
        let source_kinds = source_kinds_for_entrypoint(entrypoint);

        for source_kind in source_kinds {
            let stable_key = trust_boundary_stable_key(
                interner,
                entrypoint.stable_key,
                source_kind,
                entrypoint.language,
            );

            boundaries.push(TrustBoundaryFact {
                id: TrustBoundaryId(0), // Reassigned during normalization
                entrypoint_stable_key: entrypoint.stable_key,
                source_kind,
                target_parameter: Some(entrypoint.target_function),
                target_parameter_index: target_parameter_index_for_entrypoint(db, entrypoint),
                access_path: None,
                protocol: protocol_for_kind(entrypoint.kind),
                language: entrypoint.language,
                file: entrypoint.registration_file,
                span: entrypoint.registration_span.clone(),
                precision: entrypoint.precision, // Per D-21
                provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
                stable_key,
            });
        }
    }

    boundaries.sort_by(|boundary, other| {
        interner.compare_canonical(boundary.stable_key, other.stable_key)
    });
    boundaries
}

fn target_parameter_index_for_entrypoint(
    db: &impl AnalysisHost,
    entrypoint: &EntrypointFact,
) -> Option<usize> {
    match (entrypoint.kind, entrypoint.language) {
        (EntrypointKind::HttpRoute, Language::Go) => Some(1),
        (EntrypointKind::HttpRoute, Language::TypeScript | Language::JavaScript) => {
            match ts_js_request_parameter_index_from_places(db, entrypoint) {
                Some(Some(index)) => Some(index),
                Some(None) | None => Some(0),
            }
        }
        (EntrypointKind::HttpMiddleware, Language::TypeScript | Language::JavaScript) => {
            match ts_js_request_parameter_index_from_places(db, entrypoint) {
                Some(index) => index,
                None => Some(0),
            }
        }
        _ => None,
    }
}

fn ts_js_request_parameter_index_from_places(
    db: &impl AnalysisHost,
    entrypoint: &EntrypointFact,
) -> Option<Option<usize>> {
    let params = function_parameters(db, entrypoint.target_function);
    if params.is_empty() {
        return None;
    }

    if let Some((index, _)) = params
        .iter()
        .find(|(_, name)| name.as_deref().is_some_and(is_request_parameter_name))
    {
        return Some(Some(*index as usize));
    }

    if entrypoint.kind == EntrypointKind::HttpMiddleware && params.len() >= 4 {
        let first_is_error = params
            .first()
            .and_then(|(_, name)| name.as_deref())
            .is_some_and(is_error_parameter_name);
        if first_is_error && params.iter().any(|(index, _)| *index == 1) {
            return Some(Some(1));
        }
        return Some(None);
    }

    None
}

fn function_parameters(
    db: &impl AnalysisHost,
    function: crate::internal_core::FunctionId,
) -> Vec<(u32, Option<String>)> {
    let mut params = db
        .mir_places()
        .iter()
        .filter_map(|place| parameter_index_and_name(place, function))
        .collect::<Vec<_>>();
    params.sort();
    params
}

fn parameter_index_and_name(
    place: &PlaceFact,
    function: crate::internal_core::FunctionId,
) -> Option<(u32, Option<String>)> {
    if !place.projections.is_empty() {
        return None;
    }
    match &place.root {
        PlaceRoot::Parameter {
            function: place_function,
            index,
            name,
        } if *place_function == function => Some((*index, name.clone())),
        _ => None,
    }
}

fn is_request_parameter_name(name: &str) -> bool {
    matches!(name, "req" | "request")
}

fn is_error_parameter_name(name: &str) -> bool {
    matches!(name, "err" | "error")
}

/// Determine which trust boundary source kinds apply to a given entrypoint kind.
///
/// Per D-19 and D-20:
/// - HTTP routes: PathParam (if path has params), QueryString, RequestBody (for
///   mutation methods), RequestHeader
/// - HTTP middleware: RequestBody, RequestHeader, QueryString
/// - MCP tools/prompts: McpArguments
/// - MCP resources: McpResourceUri
/// - CLI commands: CliArgs, CliFlags
/// - Tests: no trust boundaries (not external-facing)
/// - Job/QueueConsumer: QueuePayload
/// - Others: Unknown (per D-20, mandatory for unknown source kinds)
fn source_kinds_for_entrypoint(entrypoint: &EntrypointFact) -> Vec<TrustBoundarySourceKind> {
    match entrypoint.kind {
        EntrypointKind::HttpRoute => {
            let mut kinds = Vec::new();

            // PathParam if trigger_metadata.path contains parameter placeholders
            if let Some(path) = &entrypoint.trigger_metadata.path
                && path_has_parameters(path)
            {
                kinds.push(TrustBoundarySourceKind::PathParam);
            }

            // HTTP routes always accept query strings
            kinds.push(TrustBoundarySourceKind::QueryString);

            // RequestBody for POST, PUT, PATCH, DELETE methods
            if method_accepts_body(entrypoint.trigger_metadata.method.as_deref()) {
                kinds.push(TrustBoundarySourceKind::RequestBody);
            }

            // All HTTP routes receive headers
            kinds.push(TrustBoundarySourceKind::RequestHeader);

            kinds
        }

        EntrypointKind::HttpMiddleware => {
            vec![
                TrustBoundarySourceKind::RequestBody,
                TrustBoundarySourceKind::RequestHeader,
                TrustBoundarySourceKind::QueryString,
            ]
        }

        EntrypointKind::McpTool => {
            vec![TrustBoundarySourceKind::McpArguments]
        }

        EntrypointKind::McpResource => {
            vec![TrustBoundarySourceKind::McpResourceUri]
        }

        EntrypointKind::McpPrompt => {
            vec![TrustBoundarySourceKind::McpArguments]
        }

        EntrypointKind::CliCommand => {
            vec![
                TrustBoundarySourceKind::CliArgs,
                TrustBoundarySourceKind::CliFlags,
            ]
        }

        EntrypointKind::Test => {
            // Tests are not external-facing; no trust boundary facts
            Vec::new()
        }

        EntrypointKind::Job | EntrypointKind::QueueConsumer => {
            vec![TrustBoundarySourceKind::QueuePayload]
        }

        EntrypointKind::ServerlessHandler
        | EntrypointKind::LifecycleCallback
        | EntrypointKind::EventListener
        | EntrypointKind::GeneratedDispatch => {
            // Per D-20: Unknown is mandatory for entrypoints where source kind
            // cannot be determined
            vec![TrustBoundarySourceKind::Unknown]
        }
    }
}

/// Check if an HTTP path contains parameter placeholders.
/// Recognizes both /:id (Express/chi) and /{id} (OpenAPI) styles.
fn path_has_parameters(path: &str) -> bool {
    // Express/chi style: /:param
    if path.contains("/:") {
        return true;
    }
    // OpenAPI/Go style: /{param}
    if path.contains("/{") && path.contains('}') {
        return true;
    }
    false
}

/// Check if an HTTP method accepts a request body.
/// POST, PUT, PATCH, DELETE accept bodies. GET, HEAD, OPTIONS, CONNECT, TRACE
/// typically do not. When method is None (all methods), we assume body is possible.
fn method_accepts_body(method: Option<&str>) -> bool {
    match method {
        None => true, // All methods registered; body is possible
        Some(m) => matches!(
            m.to_uppercase().as_str(),
            "POST" | "PUT" | "PATCH" | "DELETE"
        ),
    }
}

/// Determine protocol for a given entrypoint kind.
fn protocol_for_kind(kind: EntrypointKind) -> Option<String> {
    match kind {
        EntrypointKind::HttpRoute | EntrypointKind::HttpMiddleware => Some("http".to_string()),
        EntrypointKind::McpTool | EntrypointKind::McpResource | EntrypointKind::McpPrompt => {
            Some("mcp".to_string())
        }
        _ => None,
    }
}

/// Generate a stable key for a trust boundary fact.
/// Per D-21: uses the entrypoint's provider_id. Generate stable keys using
/// semantic_stable_key(FactFamily::TrustBoundary, ...).
fn trust_boundary_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    entrypoint_key: StableKeyId,
    source_kind: TrustBoundarySourceKind,
    language: crate::internal_core::Language,
) -> StableKeyId {
    stable_key_from_key_parts(
        interner,
        FactFamily::TrustBoundary,
        [
            ("entrypoint_key", KeyPart::Key(entrypoint_key)),
            ("source_kind", KeyPart::Text(&format!("{source_kind:?}"))),
            ("language", KeyPart::Text(&format!("{language:?}"))),
        ],
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::entrypoints::facts::{
        EntrypointConfidence, EntrypointPrecision, EntrypointProvenance, EntrypointStatus,
        TriggerMetadata,
    };
    use crate::analysis_neutral::ids::{EntrypointId, PlaceId};
    use crate::analysis_neutral::mir_body::MirOutput;
    use crate::analysis_neutral::places::PlaceStatus;
    use crate::internal_core::{FileId, FunctionId, Language, Span};
    use std::path::PathBuf;

    fn make_entrypoint(
        kind: EntrypointKind,
        precision: EntrypointPrecision,
        method: Option<&str>,
        path: Option<&str>,
    ) -> EntrypointFact {
        EntrypointFact {
            id: EntrypointId(0),
            language: Language::TypeScript,
            framework_id: "test.framework".to_string(),
            kind,
            target_function: FunctionId::from_raw(1),
            target_symbol: None,
            registration_span: Span::point(FileId::from_raw(1), 1, 1),
            registration_file: FileId::from_raw(1),
            trigger_metadata: TriggerMetadata {
                method: method.map(String::from),
                path: path.map(String::from),
                tool_name: None,
                event_name: None,
                test_name: None,
            },
            trust_boundary_link: None,
            precision,
            provenance: EntrypointProvenance::NativeRecognizer,
            confidence: EntrypointConfidence::High,
            status: EntrypointStatus::Resolved,
            provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
            stable_key: crate::internal_core::stable_key_for_test(&format!(
                "ep-{kind:?}-{method:?}"
            )),
        }
    }

    #[test]
    fn http_route_get_produces_query_and_header_boundaries() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::HttpRoute,
            EntrypointPrecision::Heuristic,
            Some("GET"),
            Some("/api/users"),
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        let source_kinds: Vec<_> = boundaries.iter().map(|tb| tb.source_kind).collect();
        // GET on /api/users (no path params) -> QueryString, RequestHeader
        assert!(source_kinds.contains(&TrustBoundarySourceKind::QueryString));
        assert!(source_kinds.contains(&TrustBoundarySourceKind::RequestHeader));
        assert!(!source_kinds.contains(&TrustBoundarySourceKind::PathParam));
        assert!(!source_kinds.contains(&TrustBoundarySourceKind::RequestBody));
        assert!(
            boundaries
                .iter()
                .all(|boundary| boundary.target_parameter_index == Some(0))
        );
    }

    #[test]
    fn go_http_route_targets_request_parameter_index() {
        let db = LocalAnalysisDb::new();
        let mut ep = make_entrypoint(
            EntrypointKind::HttpRoute,
            EntrypointPrecision::Heuristic,
            Some("GET"),
            Some("/api/users"),
        );
        ep.language = Language::Go;

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        assert!(
            boundaries
                .iter()
                .all(|boundary| boundary.target_parameter_index == Some(1))
        );
    }

    #[test]
    fn http_route_post_with_params_produces_all_four_boundary_kinds() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::HttpRoute,
            EntrypointPrecision::Heuristic,
            Some("POST"),
            Some("/api/users/:id"),
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        let source_kinds: Vec<_> = boundaries.iter().map(|tb| tb.source_kind).collect();
        assert!(
            source_kinds.contains(&TrustBoundarySourceKind::PathParam),
            "POST with /:id should have PathParam"
        );
        assert!(source_kinds.contains(&TrustBoundarySourceKind::QueryString));
        assert!(source_kinds.contains(&TrustBoundarySourceKind::RequestBody));
        assert!(source_kinds.contains(&TrustBoundarySourceKind::RequestHeader));
        assert_eq!(source_kinds.len(), 4);
    }

    #[test]
    fn http_route_no_method_includes_request_body() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::HttpRoute,
            EntrypointPrecision::Heuristic,
            None, // All methods
            Some("/api/users"),
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        // No method means all methods -> body is possible
        assert!(
            boundaries
                .iter()
                .any(|tb| tb.source_kind == TrustBoundarySourceKind::RequestBody)
        );
    }

    #[test]
    fn http_middleware_produces_body_header_query_boundaries() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::HttpMiddleware,
            EntrypointPrecision::Heuristic,
            None,
            None,
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        let source_kinds: Vec<_> = boundaries.iter().map(|tb| tb.source_kind).collect();
        assert_eq!(source_kinds.len(), 3);
        assert!(source_kinds.contains(&TrustBoundarySourceKind::RequestBody));
        assert!(source_kinds.contains(&TrustBoundarySourceKind::RequestHeader));
        assert!(source_kinds.contains(&TrustBoundarySourceKind::QueryString));
    }

    #[test]
    fn go_http_middleware_keeps_parameter_index_unknown() {
        let db = LocalAnalysisDb::new();
        let mut ep = make_entrypoint(
            EntrypointKind::HttpMiddleware,
            EntrypointPrecision::Heuristic,
            None,
            None,
        );
        ep.language = Language::Go;

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        assert!(
            boundaries
                .iter()
                .all(|boundary| boundary.target_parameter_index.is_none())
        );
    }

    #[test]
    fn ts_http_error_middleware_targets_request_parameter_index() {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "function errorMiddleware(err, req, res, next) {}".to_string(),
        );
        let function = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "errorMiddleware".to_string(),
            Span::point(file, 1, 1),
            Language::TypeScript,
            false,
            false,
            1,
            Vec::new(),
        ));
        db.replace_semantic_mir(MirOutput {
            bodies: Vec::new(),
            places: vec![
                parameter_place(&interner, file, function, 0, "err"),
                parameter_place(&interner, file, function, 1, "req"),
                parameter_place(&interner, file, function, 2, "res"),
                parameter_place(&interner, file, function, 3, "next"),
            ],
            operations: Vec::new(),
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("valid MIR places");

        let mut ep = make_entrypoint(
            EntrypointKind::HttpMiddleware,
            EntrypointPrecision::Heuristic,
            None,
            None,
        );
        ep.target_function = function;
        ep.registration_file = file;
        ep.registration_span = Span::point(file, 1, 1);

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        assert!(
            boundaries
                .iter()
                .all(|boundary| boundary.target_parameter_index == Some(1))
        );
    }

    #[test]
    fn mcp_tool_produces_mcp_arguments_boundary() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::McpTool,
            EntrypointPrecision::Heuristic,
            None,
            None,
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        assert_eq!(boundaries.len(), 1);
        assert_eq!(
            boundaries[0].source_kind,
            TrustBoundarySourceKind::McpArguments
        );
    }

    #[test]
    fn mcp_resource_produces_mcp_resource_uri_boundary() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::McpResource,
            EntrypointPrecision::Heuristic,
            None,
            None,
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        assert_eq!(boundaries.len(), 1);
        assert_eq!(
            boundaries[0].source_kind,
            TrustBoundarySourceKind::McpResourceUri
        );
    }

    #[test]
    fn mcp_prompt_produces_mcp_arguments_boundary() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::McpPrompt,
            EntrypointPrecision::Heuristic,
            None,
            None,
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        assert_eq!(boundaries.len(), 1);
        assert_eq!(
            boundaries[0].source_kind,
            TrustBoundarySourceKind::McpArguments
        );
    }

    #[test]
    fn cli_command_produces_cli_args_and_flags_boundaries() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::CliCommand,
            EntrypointPrecision::Conservative,
            None,
            None,
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        let source_kinds: Vec<_> = boundaries.iter().map(|tb| tb.source_kind).collect();
        assert_eq!(source_kinds.len(), 2);
        assert!(source_kinds.contains(&TrustBoundarySourceKind::CliArgs));
        assert!(source_kinds.contains(&TrustBoundarySourceKind::CliFlags));
    }

    #[test]
    fn test_entrypoint_produces_no_boundaries() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::Test,
            EntrypointPrecision::ResolvedStatic,
            None,
            None,
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        assert!(
            boundaries.is_empty(),
            "test entrypoints should produce no trust boundaries"
        );
    }

    #[test]
    fn job_entrypoint_produces_queue_payload_boundary() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::Job,
            EntrypointPrecision::Heuristic,
            None,
            None,
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        assert_eq!(boundaries.len(), 1);
        assert_eq!(
            boundaries[0].source_kind,
            TrustBoundarySourceKind::QueuePayload
        );
    }

    #[test]
    fn serverless_handler_produces_unknown_boundary() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::ServerlessHandler,
            EntrypointPrecision::Conservative,
            None,
            None,
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        assert_eq!(boundaries.len(), 1);
        assert_eq!(boundaries[0].source_kind, TrustBoundarySourceKind::Unknown);
    }

    #[test]
    fn trust_boundary_precision_follows_entrypoint_precision() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::McpTool,
            EntrypointPrecision::Conservative,
            None,
            None,
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        assert_eq!(boundaries.len(), 1);
        assert_eq!(
            boundaries[0].precision,
            EntrypointPrecision::Conservative,
            "trust boundary precision should match entrypoint precision per D-21"
        );
    }

    #[test]
    fn trust_boundary_stable_keys_include_fact_family() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::McpTool,
            EntrypointPrecision::Heuristic,
            None,
            None,
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        assert!(!boundaries.is_empty());
        let key = db.resolve_stable_key(boundaries[0].stable_key);
        assert!(
            key.contains("13:TrustBoundary"),
            "stable key should contain FactFamily::TrustBoundary; got: {key}"
        );
    }

    #[test]
    fn path_param_detection_with_express_style() {
        assert!(path_has_parameters("/api/users/:id"));
        assert!(path_has_parameters("/api/:org/repos/:id"));
    }

    #[test]
    fn path_param_detection_with_openapi_style() {
        assert!(path_has_parameters("/api/users/{id}"));
        assert!(path_has_parameters("/api/{org}/repos/{id}"));
    }

    #[test]
    fn path_param_detection_plain_path() {
        assert!(!path_has_parameters("/api/users"));
        assert!(!path_has_parameters("/"));
        assert!(!path_has_parameters("/api/users/list"));
    }

    #[test]
    fn http_protocol_set_for_http_boundaries() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::HttpRoute,
            EntrypointPrecision::Heuristic,
            Some("GET"),
            Some("/api/users"),
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        for tb in &boundaries {
            assert_eq!(tb.protocol.as_deref(), Some("http"));
        }
    }

    fn parameter_place(
        interner: &crate::internal_core::StableKeyInterner,
        file: FileId,
        function: FunctionId,
        index: u32,
        name: &str,
    ) -> PlaceFact {
        PlaceFact {
            id: PlaceId(index as u64),
            language: Language::TypeScript,
            file: Some(file),
            function: Some(function),
            root: PlaceRoot::Parameter {
                function,
                index,
                name: Some(name.to_string()),
            },
            projections: Vec::new(),
            stable_key: interner.intern(format!("place:{name}")),
            status: PlaceStatus::Resolved,
        }
    }

    #[test]
    fn mcp_protocol_set_for_mcp_boundaries() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(
            EntrypointKind::McpTool,
            EntrypointPrecision::Heuristic,
            None,
            None,
        );

        let boundaries = derive_trust_boundaries(&db, &[ep]);

        assert_eq!(boundaries.len(), 1);
        assert_eq!(boundaries[0].protocol.as_deref(), Some("mcp"));
    }
}
