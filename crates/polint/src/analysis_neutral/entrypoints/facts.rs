use serde::{Deserialize, Serialize};

use crate::analysis_neutral::ids::{
    DispatchEdgeId, EntrypointId, TrustBoundaryId, UnresolvedFrameworkId,
};
use crate::internal_core::{FileId, FunctionId, Language, Span, StableKeyId, SymbolId};

// ---------------------------------------------------------------------------
// EntrypointFact
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntrypointFact {
    pub id: EntrypointId,
    pub language: Language,
    pub framework_id: String,
    pub kind: EntrypointKind,
    pub target_function: FunctionId,
    pub target_symbol: Option<SymbolId>,
    pub registration_span: Span,
    pub registration_file: FileId,
    pub trigger_metadata: TriggerMetadata,
    pub trust_boundary_link: Option<String>,
    pub precision: EntrypointPrecision,
    pub provenance: EntrypointProvenance,
    pub confidence: EntrypointConfidence,
    pub status: EntrypointStatus,
    pub provider_id: String,
    pub stable_key: StableKeyId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerMetadata {
    pub method: Option<String>,
    pub path: Option<String>,
    pub tool_name: Option<String>,
    pub event_name: Option<String>,
    pub test_name: Option<String>,
}

impl TriggerMetadata {
    pub fn empty() -> Self {
        Self {
            method: None,
            path: None,
            tool_name: None,
            event_name: None,
            test_name: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EntrypointKind {
    HttpRoute,
    HttpMiddleware,
    McpTool,
    McpResource,
    McpPrompt,
    CliCommand,
    Test,
    Job,
    QueueConsumer,
    ServerlessHandler,
    LifecycleCallback,
    EventListener,
    GeneratedDispatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EntrypointPrecision {
    ResolvedStatic,
    SetupAware,
    Heuristic,
    Conservative,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EntrypointProvenance {
    NativeRecognizer,
    Extension,
    Generated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EntrypointConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EntrypointStatus {
    Resolved,
    Partial,
    Unresolved,
    SetupMissing,
    Unsupported,
}

// ---------------------------------------------------------------------------
// TrustBoundaryFact
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustBoundaryFact {
    pub id: TrustBoundaryId,
    pub entrypoint_stable_key: StableKeyId,
    pub source_kind: TrustBoundarySourceKind,
    pub target_parameter: Option<FunctionId>,
    pub target_parameter_index: Option<usize>,
    pub access_path: Option<String>,
    pub protocol: Option<String>,
    pub language: Language,
    pub file: FileId,
    pub span: Span,
    pub precision: EntrypointPrecision,
    pub provider_id: String,
    pub stable_key: StableKeyId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TrustBoundarySourceKind {
    PathParam,
    QueryString,
    RequestBody,
    RequestHeader,
    Cookie,
    McpArguments,
    McpResourceUri,
    CliArgs,
    CliFlags,
    EnvVar,
    Stdin,
    QueuePayload,
    ExternalReturn,
    Unknown,
}

// ---------------------------------------------------------------------------
// FrameworkDispatchEdgeFact
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkDispatchEdgeFact {
    pub id: DispatchEdgeId,
    pub from_source: String,
    pub to_target: FunctionId,
    pub to_symbol: Option<SymbolId>,
    pub edge_kind: DispatchEdgeKind,
    pub guard_metadata: Option<String>,
    pub ordering: Option<u32>,
    pub language: Language,
    pub file: FileId,
    pub span: Span,
    pub precision: EntrypointPrecision,
    pub provider_id: String,
    pub stable_key: StableKeyId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DispatchEdgeKind {
    RouteDispatch,
    MiddlewareChain,
    LifecycleHook,
    EventDispatch,
    McpDispatch,
    TestRunner,
    JobScheduler,
}

// ---------------------------------------------------------------------------
// UnresolvedFrameworkFact
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedFrameworkFact {
    pub id: UnresolvedFrameworkId,
    pub language: Language,
    pub file: FileId,
    pub span: Span,
    pub framework_id: String,
    pub reason: UnresolvedFrameworkReason,
    pub evidence: String,
    pub scope_description: String,
    pub precision: EntrypointPrecision,
    pub provider_id: String,
    pub stable_key: StableKeyId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum UnresolvedFrameworkReason {
    DynamicRoute,
    UnknownWrapper,
    UnresolvedHandler,
    MissingSetup,
    UnsupportedFrameworkVersion,
    BudgetExceeded,
    DynamicRegistration,
    UnrecognizedPattern,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal_core::{FileId, FunctionId, Language, Span, SymbolId};
    use serde::Serialize;
    use serde::de::DeserializeOwned;
    use std::fmt::Debug;
    use std::hash::Hash;

    fn assert_small_id_contract<T>()
    where
        T: Debug
            + Clone
            + Copy
            + PartialEq
            + Eq
            + PartialOrd
            + Ord
            + Hash
            + Serialize
            + DeserializeOwned,
    {
    }

    #[test]
    fn entrypoint_vocabulary_enums_are_copy_ordered_hashable_serializable() {
        assert_small_id_contract::<EntrypointKind>();
        assert_small_id_contract::<EntrypointPrecision>();
        assert_small_id_contract::<EntrypointProvenance>();
        assert_small_id_contract::<EntrypointConfidence>();
        assert_small_id_contract::<EntrypointStatus>();
        assert_small_id_contract::<TrustBoundarySourceKind>();
        assert_small_id_contract::<DispatchEdgeKind>();
        assert_small_id_contract::<UnresolvedFrameworkReason>();
    }

    #[test]
    fn entrypoint_kind_has_13_variants() {
        let variants = [
            EntrypointKind::HttpRoute,
            EntrypointKind::HttpMiddleware,
            EntrypointKind::McpTool,
            EntrypointKind::McpResource,
            EntrypointKind::McpPrompt,
            EntrypointKind::CliCommand,
            EntrypointKind::Test,
            EntrypointKind::Job,
            EntrypointKind::QueueConsumer,
            EntrypointKind::ServerlessHandler,
            EntrypointKind::LifecycleCallback,
            EntrypointKind::EventListener,
            EntrypointKind::GeneratedDispatch,
        ];
        assert_eq!(variants.len(), 13);
    }

    #[test]
    fn trust_boundary_source_kind_has_14_variants() {
        let variants = [
            TrustBoundarySourceKind::PathParam,
            TrustBoundarySourceKind::QueryString,
            TrustBoundarySourceKind::RequestBody,
            TrustBoundarySourceKind::RequestHeader,
            TrustBoundarySourceKind::Cookie,
            TrustBoundarySourceKind::McpArguments,
            TrustBoundarySourceKind::McpResourceUri,
            TrustBoundarySourceKind::CliArgs,
            TrustBoundarySourceKind::CliFlags,
            TrustBoundarySourceKind::EnvVar,
            TrustBoundarySourceKind::Stdin,
            TrustBoundarySourceKind::QueuePayload,
            TrustBoundarySourceKind::ExternalReturn,
            TrustBoundarySourceKind::Unknown,
        ];
        assert_eq!(variants.len(), 14);
    }

    #[test]
    fn unresolved_framework_reason_has_8_variants() {
        let variants = [
            UnresolvedFrameworkReason::DynamicRoute,
            UnresolvedFrameworkReason::UnknownWrapper,
            UnresolvedFrameworkReason::UnresolvedHandler,
            UnresolvedFrameworkReason::MissingSetup,
            UnresolvedFrameworkReason::UnsupportedFrameworkVersion,
            UnresolvedFrameworkReason::BudgetExceeded,
            UnresolvedFrameworkReason::DynamicRegistration,
            UnresolvedFrameworkReason::UnrecognizedPattern,
        ];
        assert_eq!(variants.len(), 8);
    }

    #[test]
    fn entrypoint_facts_keep_dense_ids_and_stable_keys_separate() {
        let entrypoint = EntrypointFact {
            id: EntrypointId(1),
            language: Language::TypeScript,
            framework_id: "express".to_string(),
            kind: EntrypointKind::HttpRoute,
            target_function: FunctionId::from_raw(10),
            target_symbol: Some(SymbolId::from_raw(20)),
            registration_span: Span::point(FileId::from_raw(1), 1, 1),
            registration_file: FileId::from_raw(1),
            trigger_metadata: TriggerMetadata {
                method: Some("GET".to_string()),
                path: Some("/api/users".to_string()),
                tool_name: None,
                event_name: None,
                test_name: None,
            },
            trust_boundary_link: None,
            precision: EntrypointPrecision::ResolvedStatic,
            provenance: EntrypointProvenance::NativeRecognizer,
            confidence: EntrypointConfidence::High,
            status: EntrypointStatus::Resolved,
            provider_id: "polint.entrypoints".to_string(),
            stable_key: crate::internal_core::stable_key_for_test(
                "entrypoint:express:get:/api/users",
            ),
        };

        let boundary = TrustBoundaryFact {
            id: TrustBoundaryId(1),
            entrypoint_stable_key: entrypoint.stable_key,
            source_kind: TrustBoundarySourceKind::PathParam,
            target_parameter: Some(FunctionId::from_raw(10)),
            target_parameter_index: Some(0),
            access_path: Some("req.params.id".to_string()),
            protocol: Some("http".to_string()),
            language: Language::TypeScript,
            file: FileId::from_raw(1),
            span: Span::point(FileId::from_raw(1), 1, 1),
            precision: EntrypointPrecision::ResolvedStatic,
            provider_id: "polint.entrypoints".to_string(),
            stable_key: crate::internal_core::stable_key_for_test(
                "trust-boundary:express:get:/api/users:PathParam",
            ),
        };

        let dispatch = FrameworkDispatchEdgeFact {
            id: DispatchEdgeId(1),
            from_source: crate::internal_core::test_stable_key_interner()
                .resolve(entrypoint.stable_key)
                .to_string(),
            to_target: FunctionId::from_raw(10),
            to_symbol: Some(SymbolId::from_raw(20)),
            edge_kind: DispatchEdgeKind::RouteDispatch,
            guard_metadata: None,
            ordering: None,
            language: Language::TypeScript,
            file: FileId::from_raw(1),
            span: Span::point(FileId::from_raw(1), 1, 1),
            precision: EntrypointPrecision::ResolvedStatic,
            provider_id: "polint.entrypoints".to_string(),
            stable_key: crate::internal_core::stable_key_for_test(
                "dispatch:express:get:/api/users",
            ),
        };

        let unresolved = UnresolvedFrameworkFact {
            id: UnresolvedFrameworkId(1),
            language: Language::TypeScript,
            file: FileId::from_raw(2),
            span: Span::point(FileId::from_raw(2), 5, 10),
            framework_id: "fastify".to_string(),
            reason: UnresolvedFrameworkReason::UnrecognizedPattern,
            evidence: "import fastify detected".to_string(),
            scope_description: "fastify server registration".to_string(),
            precision: EntrypointPrecision::Conservative,
            provider_id: "polint.entrypoints".to_string(),
            stable_key: crate::internal_core::stable_key_for_test("unresolved:fastify:file2"),
        };

        // Stable keys are distinct across fact families
        assert_ne!(entrypoint.stable_key, boundary.stable_key);
        assert_ne!(entrypoint.stable_key, dispatch.stable_key);
        assert_ne!(entrypoint.stable_key, unresolved.stable_key);
        assert_ne!(boundary.stable_key, dispatch.stable_key);

        // Trust boundary references the entrypoint
        assert_eq!(boundary.entrypoint_stable_key, entrypoint.stable_key);

        // Dispatch edge references the entrypoint
        assert_eq!(
            dispatch.from_source,
            crate::internal_core::test_stable_key_interner()
                .resolve(entrypoint.stable_key)
                .as_ref()
        );
    }
}
