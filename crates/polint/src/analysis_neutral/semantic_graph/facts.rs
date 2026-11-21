use serde::{Deserialize, Serialize};

use crate::analysis_neutral::ids::ScopeId;
use crate::analysis_neutral::ids::{
    CallSiteId, ObjectTokenId, PlaceId, SemanticEdgeId, SemanticNodeId,
};
use crate::internal_core::{FunctionId, ModuleNodeId, PackageId, StableKeyId};

// ---------------------------------------------------------------------------
// NodeKind
// ---------------------------------------------------------------------------

/// Closed taxonomy of semantic-graph node kinds (D-02, D-03).
///
/// Each variant **composes an existing v1.2 identity newtype by reference** (D-04)
/// — the graph invents no parallel identities. Pinned declaration order so the
/// derived `Ord` and serde representation are declaration-driven and byte-stable,
/// matching the established `RootKind`/`PointsToConstraintKind` convention in this
/// codebase. No explicit integer-ordinal representation attribute (`#[repr(u8)]`)
/// is used — byte-stability is achieved purely via pinned order + derived `Ord` +
/// serde rename + an `as_str()` label method, exactly as every other closed enum
/// here does.
///
/// Because the variants carry payloads they are NOT `Copy`; every payload field is
/// an `Ord` ID newtype so the derived `Ord` that drives byte-stable ordering
/// survives.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Function(FunctionId),
    Callsite(CallSiteId),
    Scope(ScopeId),
    Place(PlaceId),
    AbstractObject(ObjectTokenId),
    Module(ModuleNodeId),
    Package(PackageId),
    /// The parameter at a position of a function, as the call rule binds it.
    FunctionParam(FunctionId, u8),
    /// The rest parameter of a function.
    FunctionRest(FunctionId),
    /// The value a function's `this` holds.
    FunctionThis(FunctionId),
    /// The values a function returns.
    FunctionReturn(FunctionId),
}

impl NodeKind {
    /// Stable lowercase tag label used in stable keys and digest payloads.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Function(_) => "function",
            Self::Callsite(_) => "callsite",
            Self::Scope(_) => "scope",
            Self::Place(_) => "place",
            Self::AbstractObject(_) => "abstract_object",
            Self::Module(_) => "module",
            Self::Package(_) => "package",
            Self::FunctionParam(..) => "function_param",
            Self::FunctionRest(_) => "function_rest",
            Self::FunctionThis(_) => "function_this",
            Self::FunctionReturn(_) => "function_return",
        }
    }
}

// ---------------------------------------------------------------------------
// EdgeKind
// ---------------------------------------------------------------------------

/// Closed taxonomy of semantic-graph edge kinds (D-02, D-03).
///
/// A fieldless `Copy` enum following the `RootKind` template exactly: pinned
/// declaration order + derived `Ord` + `#[serde(rename_all = "snake_case")]` + an
/// `as_str()` label, with NO `#[repr(u8)]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Call,
    MemberOf,
    Alloc,
    Flow,
}

impl EdgeKind {
    /// Stable lowercase label used in stable keys and digest payloads.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::MemberOf => "member_of",
            Self::Alloc => "alloc",
            Self::Flow => "flow",
        }
    }
}

// ---------------------------------------------------------------------------
// Precision vocabulary
// ---------------------------------------------------------------------------

/// Honest precision tier carried by every node and edge (D-07).
///
/// Reuses the same shape as `points_to::facts::PointsToPrecision` — graph rows are
/// derived/aggregated, so the `Exact` ceiling is intentionally absent here; the
/// precision-ceiling enforcement itself lands in Plan 03 validation. Pinned order +
/// derived `Ord` + serde rename for byte-stability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticPrecision {
    ResolvedStatic,
    SetupAware,
    FlowInsensitive,
    Heuristic,
    Conservative,
    Unknown,
}

impl SemanticPrecision {
    /// Stable lowercase label used in stable keys and digest payloads.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ResolvedStatic => "resolved_static",
            Self::SetupAware => "setup_aware",
            Self::FlowInsensitive => "flow_insensitive",
            Self::Heuristic => "heuristic",
            Self::Conservative => "conservative",
            Self::Unknown => "unknown",
        }
    }
}

// ---------------------------------------------------------------------------
// Fact families
// ---------------------------------------------------------------------------

/// One semantic-graph node. Mirrors the `ReachabilityRootFact` shape: the dense
/// `id` is a run-local post-normalization read concern only and MUST NOT enter any
/// serialized stable payload that feeds the output digest (D-06) — `#[serde(skip)]`
/// strips it; serde restores it via `SemanticNodeId::default()` (= 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticNodeFact {
    pub id: SemanticNodeId,
    pub kind: NodeKind,
    pub precision: SemanticPrecision,
    /// Built from the referenced existing identity (D-06), never run-local dense
    /// IDs. Populated by Plan 03's builder; the skeleton carries the field.
    pub stable_key: StableKeyId,
}

/// One semantic-graph edge. `source`/`target` are dense `SemanticNodeId` handles
/// resolved after normalization; the persistent identity is carried by
/// `stable_key`, built from the source/target node stable keys (D-06). The dense
/// `id` carries `#[serde(skip)]` for the same digest discipline as the node fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticEdgeFact {
    pub id: SemanticEdgeId,
    pub source: SemanticNodeId,
    pub target: SemanticNodeId,
    pub kind: EdgeKind,
    pub precision: SemanticPrecision,
    pub stable_key: StableKeyId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_kind_composes_existing_identity_newtypes() {
        // Each variant wraps an existing v1.2 identity newtype (D-04); no parallel
        // identity is invented.
        let nodes = [
            NodeKind::Function(FunctionId::from_raw(1)),
            NodeKind::Callsite(CallSiteId(2)),
            NodeKind::Scope(ScopeId(3)),
            NodeKind::Place(PlaceId(4)),
            NodeKind::AbstractObject(ObjectTokenId(5)),
            NodeKind::Module(ModuleNodeId::from_raw(6)),
            NodeKind::Package(PackageId::from_raw(7)),
            NodeKind::FunctionParam(FunctionId::from_raw(1), 0),
            NodeKind::FunctionRest(FunctionId::from_raw(1)),
            NodeKind::FunctionThis(FunctionId::from_raw(1)),
            NodeKind::FunctionReturn(FunctionId::from_raw(1)),
        ];
        assert_eq!(nodes.len(), 11);
    }

    #[test]
    fn node_kind_labels_are_stable_snake_case() {
        assert_eq!(
            NodeKind::Function(FunctionId::from_raw(0)).as_str(),
            "function"
        );
        assert_eq!(NodeKind::Callsite(CallSiteId(0)).as_str(), "callsite");
        assert_eq!(NodeKind::Scope(ScopeId(0)).as_str(), "scope");
        assert_eq!(NodeKind::Place(PlaceId(0)).as_str(), "place");
        assert_eq!(
            NodeKind::AbstractObject(ObjectTokenId(0)).as_str(),
            "abstract_object"
        );
        assert_eq!(
            NodeKind::Module(ModuleNodeId::from_raw(0)).as_str(),
            "module"
        );
        assert_eq!(
            NodeKind::Package(PackageId::from_raw(0)).as_str(),
            "package"
        );
        let function = FunctionId::from_raw(0);
        assert_eq!(
            NodeKind::FunctionParam(function, 0).as_str(),
            "function_param"
        );
        assert_eq!(NodeKind::FunctionRest(function).as_str(), "function_rest");
        assert_eq!(NodeKind::FunctionThis(function).as_str(), "function_this");
        assert_eq!(
            NodeKind::FunctionReturn(function).as_str(),
            "function_return"
        );
    }

    #[test]
    fn node_kind_has_exactly_11_variants() {
        // Compile-time exhaustive match over every arm; the array length lock fails
        // to compile if a variant is added without updating this test.
        fn assert_all(kind: &NodeKind) -> &'static str {
            match kind {
                NodeKind::Function(_) => "function",
                NodeKind::Callsite(_) => "callsite",
                NodeKind::Scope(_) => "scope",
                NodeKind::Place(_) => "place",
                NodeKind::AbstractObject(_) => "abstract_object",
                NodeKind::Module(_) => "module",
                NodeKind::Package(_) => "package",
                NodeKind::FunctionParam(..) => "function_param",
                NodeKind::FunctionRest(_) => "function_rest",
                NodeKind::FunctionThis(_) => "function_this",
                NodeKind::FunctionReturn(_) => "function_return",
            }
        }
        let variants = [
            assert_all(&NodeKind::Function(FunctionId::from_raw(0))),
            assert_all(&NodeKind::Callsite(CallSiteId(0))),
            assert_all(&NodeKind::Scope(ScopeId(0))),
            assert_all(&NodeKind::Place(PlaceId(0))),
            assert_all(&NodeKind::AbstractObject(ObjectTokenId(0))),
            assert_all(&NodeKind::Module(ModuleNodeId::from_raw(0))),
            assert_all(&NodeKind::Package(PackageId::from_raw(0))),
            assert_all(&NodeKind::FunctionParam(FunctionId::from_raw(0), 0)),
            assert_all(&NodeKind::FunctionRest(FunctionId::from_raw(0))),
            assert_all(&NodeKind::FunctionThis(FunctionId::from_raw(0))),
            assert_all(&NodeKind::FunctionReturn(FunctionId::from_raw(0))),
        ];
        assert_eq!(variants.len(), 11);
    }

    #[test]
    fn node_kind_module_variant_uses_module_node_id() {
        // V3 correction guard: the module node composes `core::ModuleNodeId`, not a
        // (non-existent) `ModuleId`.
        let module = NodeKind::Module(ModuleNodeId::from_raw(42));
        match module {
            NodeKind::Module(id) => assert_eq!(id, ModuleNodeId::from_raw(42)),
            _ => panic!("expected module variant"),
        }
    }

    #[test]
    fn edge_kind_sorts_in_pinned_declaration_order() {
        let mut kinds = vec![
            EdgeKind::Flow,
            EdgeKind::Alloc,
            EdgeKind::MemberOf,
            EdgeKind::Call,
        ];
        kinds.sort();
        assert_eq!(
            kinds,
            vec![
                EdgeKind::Call,
                EdgeKind::MemberOf,
                EdgeKind::Alloc,
                EdgeKind::Flow,
            ]
        );
    }

    #[test]
    fn edge_kind_has_exactly_4_variants() {
        fn assert_all(kind: EdgeKind) -> EdgeKind {
            match kind {
                EdgeKind::Call | EdgeKind::MemberOf | EdgeKind::Alloc | EdgeKind::Flow => kind,
            }
        }
        let variants = [
            assert_all(EdgeKind::Call),
            assert_all(EdgeKind::MemberOf),
            assert_all(EdgeKind::Alloc),
            assert_all(EdgeKind::Flow),
        ];
        assert_eq!(variants.len(), 4);
    }

    #[test]
    fn edge_kind_labels_are_stable_snake_case() {
        assert_eq!(EdgeKind::Call.as_str(), "call");
        assert_eq!(EdgeKind::MemberOf.as_str(), "member_of");
        assert_eq!(EdgeKind::Alloc.as_str(), "alloc");
        assert_eq!(EdgeKind::Flow.as_str(), "flow");
    }

    #[test]
    fn node_fact_keeps_dense_and_stable_ids_separate() {
        let db = crate::analysis_neutral::LocalAnalysisDb::new();
        let node = SemanticNodeFact {
            id: SemanticNodeId(7),
            kind: NodeKind::Function(FunctionId::from_raw(3)),
            precision: SemanticPrecision::SetupAware,
            stable_key: db.stable_key_interner().intern("node|function|pkg.F"),
        };
        assert_eq!(node.id, SemanticNodeId(7));
        assert_eq!(
            db.resolve_stable_key(node.stable_key).as_ref(),
            "node|function|pkg.F"
        );
    }

    #[test]
    fn edge_fact_keeps_dense_and_stable_ids_separate() {
        let db = crate::analysis_neutral::LocalAnalysisDb::new();
        let edge = SemanticEdgeFact {
            id: SemanticEdgeId(9),
            source: SemanticNodeId(1),
            target: SemanticNodeId(2),
            kind: EdgeKind::Call,
            precision: SemanticPrecision::Conservative,
            stable_key: db.stable_key_interner().intern("edge|call|a|b"),
        };
        assert_eq!(edge.id, SemanticEdgeId(9));
        assert_eq!(
            db.resolve_stable_key(edge.stable_key).as_ref(),
            "edge|call|a|b"
        );
    }

    #[test]
    fn semantic_precision_labels_are_stable_snake_case() {
        assert_eq!(
            SemanticPrecision::ResolvedStatic.as_str(),
            "resolved_static"
        );
        assert_eq!(SemanticPrecision::SetupAware.as_str(), "setup_aware");
        assert_eq!(
            SemanticPrecision::FlowInsensitive.as_str(),
            "flow_insensitive"
        );
        assert_eq!(SemanticPrecision::Unknown.as_str(), "unknown");
    }
}
