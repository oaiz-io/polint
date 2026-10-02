use serde::{Deserialize, Serialize};

use crate::analysis_neutral::ids::{CallSiteId, CallTargetId, MirBodyId, MirOpId, PlaceId};
use crate::internal_core::{
    FileId, FunctionId, Language, ReferenceId, Span, StableKeyId, SymbolId,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallSiteFact {
    pub id: CallSiteId,
    pub language: Language,
    pub file: FileId,
    pub caller: FunctionId,
    pub owner_symbol: Option<SymbolId>,
    pub body: MirBodyId,
    pub operation: MirOpId,
    pub span: Span,
    pub kind: CallSyntaxKind,
    pub callee: CallCallee,
    pub receiver: Option<PlaceId>,
    pub arguments: Vec<PlaceId>,
    pub result: Option<PlaceId>,
    pub status: CallTargetStatus,
    pub precision: CallPrecision,
    /// True when this call site is lexically inside a `throw` argument
    /// (`throw new E(... f() ...)`). Such calls sit on error paths that the
    /// demand-driven oracle does not exercise, so resolvers skip them to avoid
    /// false edges (e.g. express's `gettype(fn)` in a middleware-type-check throw).
    pub in_throw: bool,
    pub stable_key: StableKeyId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallTargetFact {
    pub id: CallTargetId,
    pub site: CallSiteId,
    pub caller: FunctionId,
    pub target_function: Option<FunctionId>,
    pub target_symbol: Option<SymbolId>,
    /// The callee's identity when it has neither an in-repository function nor a
    /// symbol: a dependency's function or method named by the typed frontend
    /// (`fmt.Println`), or a `go:`-prefixed label for a call expression that is a
    /// builtin or a conversion.
    pub synthetic_target: Option<String>,
    pub edge_kind: CallEdgeKind,
    pub algorithm: CallAlgorithm,
    pub status: CallTargetStatus,
    pub reason: Option<UnresolvedCallReason>,
    pub provenance: CallProvenance,
    pub precision: CallPrecision,
    pub stable_key: StableKeyId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedCallFact {
    pub site: CallSiteId,
    pub caller: FunctionId,
    pub status: CallTargetStatus,
    pub reason: UnresolvedCallReason,
    pub algorithm: CallAlgorithm,
    pub provenance: CallProvenance,
    pub precision: CallPrecision,
    pub stable_key: StableKeyId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CallSyntaxKind {
    Function,
    Method,
    Constructor,
    StaticMember,
    Member,
    Index,
    Super,
    Import,
    New,
    TaggedTemplate,
    GoRoutine,
    Deferred,
    DynamicImport,
    Require,
    FunctionValue,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CallCallee {
    Identifier {
        reference: Option<ReferenceId>,
        name: String,
    },
    Member {
        base: PlaceId,
        property: String,
    },
    Index {
        base: PlaceId,
        index: Option<PlaceId>,
    },
    Super,
    Import,
    FunctionValue {
        place: PlaceId,
    },
    Constructor {
        reference: Option<ReferenceId>,
        name: Option<String>,
    },
    Unknown {
        reason: UnresolvedCallReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CallEdgeKind {
    Direct,
    Constructor,
    StaticMember,
    MethodDirect,
    Method,
    FunctionValue,
    Synthetic,
    Spawn,
    Deferred,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CallAlgorithm {
    SyntaxOnly,
    DirectReference,
    ImportBinding,
    ConstructorBinding,
    StaticMember,
    DirectMember,
    GoStatic,
    GoCha,
    GoRta,
    GoVta,
    TypeHierarchy,
    PointsTo,
    SummaryAssisted,
    FrameworkModel,
    RepoModel,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CallTargetStatus {
    Resolved,
    Ambiguous,
    Unresolved,
    Unsupported,
    SetupMissing,
    BudgetExceeded,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum UnresolvedCallReason {
    FunctionValue,
    DynamicProperty,
    InterfaceDispatch,
    Eval,
    CallApplyBind,
    FrameworkDispatch,
    Reflection,
    GoroutineBoundary,
    DynamicImport,
    ProxyOrAccessor,
    MissingSemanticReference,
    MissingImportResolution,
    SetupMissing,
    UnsupportedSyntax,
    BudgetExceeded,
    UnknownCallee,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CallPrecision {
    Exact,
    SetupAware,
    Conservative,
    Heuristic,
    Ambiguous,
    Unknown,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CallProvenance {
    NativeDirect,
    Native,
    SemanticReference,
    ImportBinding,
    MirShape,
    Topology,
    Extension,
    Model,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::internal_core::{FileId, FunctionId, Language, SymbolId};
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
    fn call_target_id_is_copy_ordered_hashable_serializable_and_distinct_from_call_site_id() {
        assert_small_id_contract::<CallTargetId>();

        let site = CallSiteId(7);
        let target = CallTargetId(7);

        assert_eq!(site.0, target.0);
    }

    #[test]
    fn call_facts_keep_dense_ids_and_stable_keys_separate() {
        let db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        let site = CallSiteFact {
            in_throw: false,
            id: CallSiteId(1),
            language: Language::TypeScript,
            file: FileId::from_raw(2),
            caller: FunctionId::from_raw(3),
            owner_symbol: Some(SymbolId::from_raw(4)),
            body: MirBodyId(5),
            operation: MirOpId(6),
            span: Span::point(FileId::from_raw(2), 10, 20),
            kind: CallSyntaxKind::Function,
            callee: CallCallee::Identifier {
                reference: None,
                name: "run".to_string(),
            },
            receiver: None,
            arguments: vec![PlaceId(7)],
            result: Some(PlaceId(8)),
            status: CallTargetStatus::Resolved,
            precision: CallPrecision::Exact,
            stable_key: interner.intern("call-site:test".to_string()),
        };

        let target = CallTargetFact {
            id: CallTargetId(9),
            site: site.id,
            caller: site.caller,
            target_function: Some(FunctionId::from_raw(10)),
            target_symbol: Some(SymbolId::from_raw(11)),
            synthetic_target: None,
            edge_kind: CallEdgeKind::Direct,
            algorithm: CallAlgorithm::DirectReference,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::Native,
            precision: CallPrecision::Exact,
            stable_key: interner.intern("call-target:test".to_string()),
        };

        assert_ne!(site.stable_key, target.stable_key);
        assert_eq!(site.id, target.site);
    }

    #[test]
    fn call_status_and_reason_vocabulary_covers_direct_and_unknown_forms() {
        let statuses = [
            CallTargetStatus::Resolved,
            CallTargetStatus::Ambiguous,
            CallTargetStatus::Unresolved,
            CallTargetStatus::Unsupported,
            CallTargetStatus::SetupMissing,
        ];
        let reasons = [
            UnresolvedCallReason::FunctionValue,
            UnresolvedCallReason::DynamicProperty,
            UnresolvedCallReason::InterfaceDispatch,
            UnresolvedCallReason::Eval,
            UnresolvedCallReason::CallApplyBind,
            UnresolvedCallReason::FrameworkDispatch,
        ];

        assert_eq!(statuses.len(), 5);
        assert_eq!(reasons.len(), 6);
    }
}
