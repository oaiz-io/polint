use crate::analysis_api::{FactFamily, stable_key_from_key_parts, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::entrypoints::facts::{
    DispatchEdgeKind, EntrypointFact, EntrypointKind, FrameworkDispatchEdgeFact,
};
use crate::analysis_neutral::entrypoints::provider::ENTRYPOINTS_PROVIDER_ID;
use crate::analysis_neutral::ids::DispatchEdgeId;
use crate::internal_core::{KeyPart, StableKeyId};

/// Derive framework dispatch edge facts from recognized entrypoints.
///
/// Per D-04: dispatch edges represent synthetic call edges from framework/protocol
/// dispatch roots to handler functions. For each entrypoint with a resolved
/// target_function, emit a FrameworkDispatchEdgeFact connecting a synthetic
/// framework dispatch root to the handler.
pub fn derive_dispatch_edges(
    _db: &impl AnalysisHost,
    entrypoints: &[EntrypointFact],
) -> Vec<FrameworkDispatchEdgeFact> {
    let interner_handle = _db.stable_key_interner();
    let interner = &interner_handle;
    let mut edges = Vec::new();

    for entrypoint in entrypoints {
        let edge_kind = edge_kind_for_entrypoint(entrypoint.kind);

        let stable_key = dispatch_edge_stable_key(
            interner,
            entrypoint.stable_key,
            &format!("{}", entrypoint.target_function.0),
            edge_kind,
            entrypoint.language,
        );

        edges.push(FrameworkDispatchEdgeFact {
            id: DispatchEdgeId(0), // Reassigned during normalization
            from_source: interner.resolve(entrypoint.stable_key).to_string(),
            to_target: entrypoint.target_function,
            to_symbol: entrypoint.target_symbol,
            edge_kind,
            guard_metadata: None,
            ordering: None,
            language: entrypoint.language,
            file: entrypoint.registration_file,
            span: entrypoint.registration_span.clone(),
            precision: entrypoint.precision,
            provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
            stable_key,
        });
    }

    edges.sort_by(|edge, other| interner.compare_canonical(edge.stable_key, other.stable_key));
    edges
}

/// Map EntrypointKind to DispatchEdgeKind.
///
/// Per D-04:
/// - HttpRoute -> RouteDispatch
/// - HttpMiddleware -> MiddlewareChain
/// - McpTool/McpResource/McpPrompt -> McpDispatch
/// - Test -> TestRunner
/// - CliCommand -> RouteDispatch
/// - Job/QueueConsumer -> JobScheduler
/// - LifecycleCallback -> LifecycleHook
/// - EventListener -> EventDispatch
/// - ServerlessHandler/GeneratedDispatch -> RouteDispatch
fn edge_kind_for_entrypoint(kind: EntrypointKind) -> DispatchEdgeKind {
    match kind {
        EntrypointKind::HttpRoute => DispatchEdgeKind::RouteDispatch,
        EntrypointKind::HttpMiddleware => DispatchEdgeKind::MiddlewareChain,
        EntrypointKind::McpTool | EntrypointKind::McpResource | EntrypointKind::McpPrompt => {
            DispatchEdgeKind::McpDispatch
        }
        EntrypointKind::Test => DispatchEdgeKind::TestRunner,
        EntrypointKind::CliCommand => DispatchEdgeKind::RouteDispatch,
        EntrypointKind::Job | EntrypointKind::QueueConsumer => DispatchEdgeKind::JobScheduler,
        EntrypointKind::LifecycleCallback => DispatchEdgeKind::LifecycleHook,
        EntrypointKind::EventListener => DispatchEdgeKind::EventDispatch,
        EntrypointKind::ServerlessHandler | EntrypointKind::GeneratedDispatch => {
            DispatchEdgeKind::RouteDispatch
        }
    }
}

/// Generate a stable key for a dispatch edge fact.
fn dispatch_edge_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    from_source: StableKeyId,
    to_function_key: &str,
    edge_kind: DispatchEdgeKind,
    language: crate::internal_core::Language,
) -> StableKeyId {
    stable_key_from_key_parts(
        interner,
        FactFamily::DispatchEdge,
        [
            ("from", KeyPart::Key(from_source)),
            ("to_function_key", KeyPart::Text(to_function_key)),
            ("edge_kind", KeyPart::Text(&format!("{edge_kind:?}"))),
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
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::entrypoints::facts::{
        EntrypointConfidence, EntrypointPrecision, EntrypointProvenance, EntrypointStatus,
        TriggerMetadata,
    };
    use crate::analysis_neutral::ids::EntrypointId;
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    fn make_entrypoint(kind: EntrypointKind, framework_id: &str) -> EntrypointFact {
        EntrypointFact {
            id: EntrypointId(0),
            language: Language::Go,
            framework_id: framework_id.to_string(),
            kind,
            target_function: FunctionId::from_raw(42),
            target_symbol: None,
            registration_span: Span::point(FileId::from_raw(1), 1, 1),
            registration_file: FileId::from_raw(1),
            trigger_metadata: TriggerMetadata::empty(),
            trust_boundary_link: None,
            precision: EntrypointPrecision::Heuristic,
            provenance: EntrypointProvenance::NativeRecognizer,
            confidence: EntrypointConfidence::High,
            status: EntrypointStatus::Resolved,
            provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
            stable_key: crate::internal_core::stable_key_for_test(&format!("ep-{kind:?}")),
        }
    }

    #[test]
    fn http_route_produces_route_dispatch_edge() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(EntrypointKind::HttpRoute, "go.net_http");

        let edges = derive_dispatch_edges(&db, &[ep]);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_kind, DispatchEdgeKind::RouteDispatch);
        assert_eq!(edges[0].to_target, FunctionId::from_raw(42));
        assert_eq!(edges[0].from_source, "ep-HttpRoute");
    }

    #[test]
    fn http_middleware_produces_middleware_chain_edge() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(EntrypointKind::HttpMiddleware, "go.chi");

        let edges = derive_dispatch_edges(&db, &[ep]);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_kind, DispatchEdgeKind::MiddlewareChain);
    }

    #[test]
    fn mcp_tool_produces_mcp_dispatch_edge() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(EntrypointKind::McpTool, "ts.mcp_sdk");

        let edges = derive_dispatch_edges(&db, &[ep]);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_kind, DispatchEdgeKind::McpDispatch);
    }

    #[test]
    fn test_entrypoint_produces_test_runner_edge() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(EntrypointKind::Test, "go.testing");

        let edges = derive_dispatch_edges(&db, &[ep]);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_kind, DispatchEdgeKind::TestRunner);
    }

    #[test]
    fn cli_command_produces_route_dispatch_edge() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(EntrypointKind::CliCommand, "go.cobra");

        let edges = derive_dispatch_edges(&db, &[ep]);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_kind, DispatchEdgeKind::RouteDispatch);
    }

    #[test]
    fn job_produces_job_scheduler_edge() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(EntrypointKind::Job, "test.jobs");

        let edges = derive_dispatch_edges(&db, &[ep]);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_kind, DispatchEdgeKind::JobScheduler);
    }

    #[test]
    fn dispatch_edge_precision_follows_entrypoint_precision() {
        let db = LocalAnalysisDb::new();
        let mut ep = make_entrypoint(EntrypointKind::HttpRoute, "go.net_http");
        ep.precision = EntrypointPrecision::Conservative;

        let edges = derive_dispatch_edges(&db, &[ep]);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].precision, EntrypointPrecision::Conservative);
    }

    #[test]
    fn dispatch_edge_stable_keys_include_fact_family() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(EntrypointKind::HttpRoute, "go.net_http");

        let edges = derive_dispatch_edges(&db, &[ep]);

        assert!(!edges.is_empty());
        let key = db.resolve_stable_key(edges[0].stable_key);
        assert!(
            key.contains("12:DispatchEdge"),
            "stable key should contain FactFamily::DispatchEdge; got: {key}"
        );
    }

    #[test]
    fn multiple_entrypoints_produce_multiple_edges() {
        let db = LocalAnalysisDb::new();
        let ep1 = make_entrypoint(EntrypointKind::HttpRoute, "go.net_http");
        let ep2 = make_entrypoint(EntrypointKind::McpTool, "ts.mcp_sdk");

        let edges = derive_dispatch_edges(&db, &[ep1, ep2]);

        assert_eq!(edges.len(), 2);
    }

    #[test]
    fn output_is_sorted_by_stable_key() {
        let db = LocalAnalysisDb::new();
        let mut ep1 = make_entrypoint(EntrypointKind::HttpRoute, "z.framework");
        ep1.target_function = FunctionId::from_raw(1);
        let mut ep2 = make_entrypoint(EntrypointKind::HttpRoute, "a.framework");
        ep2.target_function = FunctionId::from_raw(2);

        let edges = derive_dispatch_edges(&db, &[ep1, ep2]);

        let keys: Vec<_> = edges
            .iter()
            .map(|edge| db.resolve_stable_key(edge.stable_key))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(
            keys, sorted,
            "dispatch edges should be sorted by stable key"
        );
    }

    #[test]
    fn dispatch_edge_from_source_references_entrypoint_stable_key() {
        let db = LocalAnalysisDb::new();
        let ep = make_entrypoint(EntrypointKind::Test, "go.testing");
        let entrypoint_key = ep.stable_key;

        let edges = derive_dispatch_edges(&db, &[ep]);

        assert_eq!(edges.len(), 1);
        assert_eq!(
            edges[0].from_source,
            db.resolve_stable_key(entrypoint_key).as_ref()
        );
    }
}
