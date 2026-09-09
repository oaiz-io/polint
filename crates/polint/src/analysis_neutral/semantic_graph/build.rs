use std::collections::BTreeMap;
use std::sync::Arc;

use crate::analysis_api::{FactFamily, FunctionFact, PackageFact};
use crate::internal_core::{StableKeyId, StableKeyInterner};

use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::adaptation::store::AdaptationModelStore;
use crate::analysis_neutral::ids::{PlaceId, ScopeId, SemanticNodeId};
use crate::analysis_neutral::points_to::facts::{PointsToPrecision, PointsToStatus};
use crate::analysis_neutral::semantic_graph::constraints::{ConstraintFact, ConstraintKind};
use crate::analysis_neutral::semantic_graph::facts::{
    EdgeKind, NodeKind, SemanticEdgeFact, SemanticNodeFact, SemanticPrecision,
};
use crate::analysis_neutral::semantic_graph::store::SemanticGraphOutput;
use crate::analysis_neutral::stable_key::semantic_stable_key;
use crate::analysis_neutral::values::facts::{ValueKind, ValueSubject};

#[doc(hidden)]
#[derive(Default)]
pub struct SemanticGraphBuilder {
    nodes: Vec<SemanticNodeFact>,
    edges: Vec<SemanticEdgeFact>,
    constraints: Vec<ConstraintFact>,
    next_node: u64,
    /// node stable key -> pre-sort SemanticNodeId, so a node identity is projected
    /// at most once and edges/constraints can resolve their endpoints.
    node_by_key: BTreeMap<StableKeyId, SemanticNodeId>,
    /// pre-sort SemanticNodeId (as index) -> node stable key, the O(1) reverse of
    /// `node_by_key` so `node_key_for` (called twice per edge) never linear-scans.
    key_by_node: Vec<StableKeyId>,
}

impl SemanticGraphBuilder {
    pub fn intern_node(
        &mut self,
        interner: &StableKeyInterner,
        kind: NodeKind,
        stable_key_text: String,
    ) -> SemanticNodeId {
        let stable_key = interner.intern(stable_key_text);
        if let Some(&id) = self.node_by_key.get(&stable_key) {
            return id;
        }
        let id = SemanticNodeId(self.next_node);
        self.next_node += 1;
        self.node_by_key.insert(stable_key, id);
        // key_by_node is index-aligned with the sequential pre-sort id, so id.0 ==
        // key_by_node.len() at the moment of the push.
        debug_assert_eq!(id.0 as usize, self.key_by_node.len());
        self.key_by_node.push(stable_key);
        self.nodes.push(SemanticNodeFact {
            id,
            kind,
            // Conservative: graph rows are derived/aggregated, never `ResolvedStatic`
            // (the Exact-equivalent ceiling). Plan 03 validates the ceiling.
            precision: SemanticPrecision::Conservative,
            stable_key,
        });
        id
    }

    pub fn push_edge(
        &mut self,
        interner: &StableKeyInterner,
        kind: EdgeKind,
        source: SemanticNodeId,
        target: SemanticNodeId,
    ) {
        // Edge stable key is composed from (edge-kind label, source node stable key,
        // target node stable key) — never dense IDs (D-06). Resolve the endpoint
        // stable keys from the interned node table.
        let source_key = self.node_key_for(source);
        let target_key = self.node_key_for(target);
        let stable_key = interner.intern(
            semantic_stable_key(
                FactFamily::Reference,
                &[
                    ("edge_kind", kind.as_str().to_string()),
                    ("source", interner.resolve(source_key).to_string()),
                    ("target", interner.resolve(target_key).to_string()),
                ],
            )
            .into_string(),
        );
        self.edges.push(SemanticEdgeFact {
            id: Default::default(),
            source,
            target,
            kind,
            precision: SemanticPrecision::Conservative,
            stable_key,
        });
    }

    pub fn push_constraint(
        &mut self,
        interner: &StableKeyInterner,
        kind: ConstraintKind,
        identity: &str,
    ) {
        self.push_constraint_with_precision(
            interner,
            kind,
            identity,
            PointsToPrecision::FlowInsensitive,
        );
    }

    pub(crate) fn push_constraint_with_precision(
        &mut self,
        interner: &StableKeyInterner,
        kind: ConstraintKind,
        identity: &str,
        precision: PointsToPrecision,
    ) {
        let stable_key = interner.intern(
            semantic_stable_key(
                FactFamily::PointsToConstraint,
                &[
                    ("constraint_kind", kind.as_str().to_string()),
                    ("identity", identity.to_string()),
                ],
            )
            .into_string(),
        );
        self.constraints.push(ConstraintFact {
            id: Default::default(),
            kind,
            status: PointsToStatus::Present,
            precision,
            stable_key,
        });
    }

    /// Resolves a pre-sort node id back to its stable key in O(1) via the
    /// index-aligned `key_by_node` sidecar. `push_edge` only ever receives ids
    /// returned by `intern_node`, so the index is always in range; an out-of-range
    /// id is a builder invariant violation, not an expected empty-key fallback.
    fn node_key_for(&self, id: SemanticNodeId) -> StableKeyId {
        self.key_by_node
            .get(id.0 as usize)
            .copied()
            .expect("edge endpoint must reference an interned node")
    }

    /// Look up a pre-sort node by its composed stable-key text.
    pub fn node_for_key(&self, interner: &StableKeyInterner, key: &str) -> Option<SemanticNodeId> {
        self.node_by_key.get(&interner.intern(key)).copied()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    // -- node projection ----------------------------------------------------

    pub fn project_nodes(&mut self, db: &impl AnalysisHost) {
        let interner_handle = db.stable_key_interner();
        let interner = &interner_handle;
        for function in db.functions() {
            let key = function_node_key(db, function);
            self.intern_node(interner, NodeKind::Function(function.id), key);
        }
        for package in db.packages() {
            let key = package_node_key(db, package);
            self.intern_node(interner, NodeKind::Package(package.id), key);
        }
        for site in db.call_sites() {
            let key = node_key_from_identity("callsite", &interner.resolve(site.stable_key));
            self.intern_node(interner, NodeKind::Callsite(site.id), key);
        }
        for scope in db.semantic_scopes() {
            let key = node_key_from_identity("scope", &interner.resolve(scope.stable_key));
            self.intern_node(interner, NodeKind::Scope(ScopeId(scope.id.0)), key);
        }
    }

    pub fn function_node(
        &self,
        interner: &StableKeyInterner,
        db: &impl AnalysisHost,
        function: &FunctionFact,
    ) -> Option<SemanticNodeId> {
        let key = function_node_key(db, function);
        self.node_by_key.get(&interner.intern(key)).copied()
    }

    // -- call edges + call constraints --------------------------------------

    pub fn project_call_edges_and_constraints(&mut self, db: &impl AnalysisHost) {
        let interner_handle = db.stable_key_interner();
        let interner = &interner_handle;
        // Map FunctionId -> its node id by walking functions once. Functions that
        // were not interned as nodes are simply absent (no sentinel).
        let func_node: BTreeMap<_, _> = db
            .functions()
            .iter()
            .filter_map(|function| {
                self.function_node(interner, db, function)
                    .map(|node| (function.id, node))
            })
            .collect();

        for site in db.call_sites() {
            let site_key = node_key_from_identity("callsite", &interner.resolve(site.stable_key));
            let Some(&callsite_node) = self.node_by_key.get(&interner.intern(site_key)) else {
                continue;
            };
            // Call edge: caller function node -> callsite node, only where the caller
            // resolves to a projected function node (honest: no fabricated endpoints).
            if let Some(&caller_node) = func_node.get(&site.caller) {
                self.push_edge(interner, EdgeKind::Call, caller_node, callsite_node);
            }
            // CallConstraint anchored at the callsite node — one per call site.
            self.push_constraint(
                interner,
                ConstraintKind::CallConstraint {
                    callsite: callsite_node,
                },
                &interner.resolve(site.stable_key),
            );
        }
    }

    // -- adaptation model constraints --------------------------------------

    pub fn project_adaptation_models(
        &mut self,
        interner: &StableKeyInterner,
        adaptation_models: &AdaptationModelStore,
    ) {
        for accepted in adaptation_models.accepted() {
            let fact = &accepted.fact;
            let (Some(&source), Some(&target)) = (
                self.node_by_key.get(&interner.intern(&fact.source_pattern)),
                self.node_by_key.get(&interner.intern(&fact.target_pattern)),
            ) else {
                continue;
            };
            self.push_constraint(
                interner,
                ConstraintKind::ModelEdge {
                    source,
                    target,
                    language: fact.language.as_str().to_string(),
                    scope: fact.scope.clone(),
                    confidence: fact.confidence.as_str().to_string(),
                    evidence: fact.evidence.clone(),
                },
                &interner.resolve(fact.stable_key),
            );
        }
    }

    // -- neutral containment edges ------------------------------------------

    pub fn project_member_of_edges(&mut self, db: &impl AnalysisHost) {
        let interner = db.stable_key_interner();
        let package_nodes: BTreeMap<_, _> = db
            .packages()
            .iter()
            .filter_map(|package| {
                self.node_for_key(&interner, &package_node_key(db, package))
                    .map(|node| (package.id, node))
            })
            .collect();

        for scope in db.semantic_scopes() {
            let Some(package) = scope.package else {
                continue;
            };
            let scope_key = node_key_from_identity("scope", &interner.resolve(scope.stable_key));
            let Some(scope_node) = self.node_for_key(&interner, &scope_key) else {
                continue;
            };
            if let Some(&package_node) = package_nodes.get(&package) {
                self.push_edge(&interner, EdgeKind::MemberOf, scope_node, package_node);
            }
        }
    }

    // -- value-derived constraints (CopyEdge) -------------------------------
    //
    // `Alloc` constraints are NOT emitted here: the abstract-object endpoint is a
    // `NodeKind::AbstractObject(ObjectTokenId)`, but `ValueFact` allocations carry an
    // `AllocationTokenId` (a distinct identity family with no honest 1:1 mapping at
    // this layer). Synthesizing an object node from an allocation token would
    // fabricate an endpoint, so real `Alloc` emission is deferred to a later plan
    // that wires the object-token bridge (D-07 honesty), rather than inflating recall.

    pub fn project_value_constraints(&mut self, db: &impl AnalysisHost) {
        let interner_handle = db.stable_key_interner();
        let interner = &interner_handle;
        // `PlaceId` -> the place's own content-stable key. Keying a Place node by the
        // *place* identity (not the owning value fact) means the SAME place referenced
        // by different value facts maps to ONE node, so the solver can unify
        // copy chains through a shared place. A place absent from the MIR place table
        // has no honest stable identity at this layer, so its copy is skipped rather
        // than fabricated (D-07), mirroring the Alloc/Field deferrals.
        let place_key_by_id: BTreeMap<PlaceId, Arc<str>> = db
            .mir_places()
            .iter()
            .map(|place| (place.id, interner.resolve(place.stable_key)))
            .collect();

        for value in db.value_facts() {
            // Only project where the value's subject is a concrete place — that is the
            // `dst` of the copy relationship.
            let ValueSubject::Place(dst_place) = value.subject else {
                continue;
            };

            // `dst <- src`: a value copy from another concrete place.
            let src_place = match &value.kind {
                ValueKind::PlaceRef(src) | ValueKind::CallReturn(src) => src,
                _ => continue,
            };

            // Both endpoints must resolve to a place with an honest, shared stable
            // identity; otherwise skip rather than fabricate a per-fact place node.
            let (Some(dst_stable), Some(src_stable)) = (
                place_key_by_id.get(&dst_place),
                place_key_by_id.get(src_place),
            ) else {
                continue;
            };

            let dst_node = self.intern_node(
                interner,
                NodeKind::Place(dst_place),
                place_node_key(dst_stable),
            );
            let src_node = self.intern_node(
                interner,
                NodeKind::Place(*src_place),
                place_node_key(src_stable),
            );
            self.push_constraint(
                interner,
                ConstraintKind::CopyEdge {
                    dst: dst_node,
                    src: src_node,
                },
                &interner.resolve(value.stable_key),
            );
        }
    }

    pub fn into_output(self) -> SemanticGraphOutput {
        SemanticGraphOutput {
            nodes: self.nodes,
            edges: self.edges,
            constraints: self.constraints,
        }
    }
}

/// Compose a function node key from its stable source identity.
pub fn function_node_key(db: &impl AnalysisHost, function: &FunctionFact) -> String {
    let path = db.path_for(function.file);
    semantic_stable_key(
        FactFamily::Function,
        &[
            ("node_kind", "function".to_string()),
            ("path", path),
            ("name", function.name.clone()),
            ("span", span_identity(&function.span)),
        ],
    )
    .into_string()
}

pub fn package_node_key(db: &impl AnalysisHost, package: &PackageFact) -> String {
    let path = db.path_for(package.file);
    semantic_stable_key(
        FactFamily::Package,
        &[
            ("node_kind", "package".to_string()),
            ("path", path),
            ("name", package.name.clone()),
        ],
    )
    .into_string()
}

pub fn node_key_from_identity(node_kind: &str, identity: &str) -> String {
    semantic_stable_key(
        FactFamily::Scope,
        &[
            ("node_kind", node_kind.to_string()),
            ("identity", identity.to_string()),
        ],
    )
    .into_string()
}

pub fn place_node_key(place_stable_key: &str) -> String {
    semantic_stable_key(
        FactFamily::Place,
        &[
            ("node_kind", "place".to_string()),
            ("identity", place_stable_key.to_string()),
        ],
    )
    .into_string()
}

fn span_identity(span: &crate::internal_core::Span) -> String {
    format!("{}:{}..{}", span.file.0, span.start_byte, span.end_byte)
}
