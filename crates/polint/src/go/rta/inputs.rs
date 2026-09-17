//! Go fact projection into the frontend-neutral RTA snapshot.
//!
//! This adapter reads Go semantic facts and the composed semantic graph once, then
//! returns a closed [`GoRtaInputs`] snapshot. It owns no dispatch or fixpoint logic.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_api::FactFamily;
use crate::analysis_neutral::ids::SemanticNodeId;
use crate::analysis_neutral::semantic_graph::facts::NodeKind;
use crate::analysis_neutral::stable_key::semantic_stable_key;
use crate::core::{AnalysisDb, FileId, FunctionFact, Language, Span, StableKeyId};
use crate::go::semantic::facts::{GoSemanticCallStatus, GoSemanticFunctionFact};

pub(crate) use crate::analysis_neutral::solver::go_rta::{
    RtaCallsite as GoRtaCallsite, RtaInputs as GoRtaInputs, RtaMethod as GoRtaMethod,
};

impl GoRtaInputs {
    /// Build the closed snapshot from the stored Go-frontend facts + the already-built
    /// `polint.semantic_graph` function nodes. Reads only the AnalysisDb
    /// accessors; builds `BTree`-keyed structures for determinism. Honest by
    /// construction: a function with no matching semantic node is simply absent from
    /// [`Self::function_node`] (its edges cannot be emitted), never fabricated.
    pub(crate) fn from_db(interner: &crate::core::StableKeyInterner, db: &AnalysisDb) -> Self {
        // qualified -> SemanticNodeId, reconstructed from the function-node stable key
        // recipe `polint.semantic_graph` used (composition over a private coupling):
        // a Go semantic function maps to a node iff a core FunctionFact matches it and
        // that function was interned as a graph node.
        let function_node_by_stable_key: BTreeMap<String, SemanticNodeId> = db
            .semantic_nodes()
            .iter()
            .filter(|node| matches!(node.kind, NodeKind::Function(_)))
            .map(|node| (db.resolve_stable_key(node.stable_key).to_string(), node.id))
            .collect();

        // Build the Go-only core-function, call-site, and callsite-node indexes once for
        // scale). Every per-Go-function / per-dispatch-row join below is a bucket lookup
        // into this, never a fresh `db.functions()` / `db.call_sites()` /
        // `db.semantic_nodes()` linear scan. This keeps the O(F_go · F_total) plus
        // O(rows · call_sites) joins near-linear while preserving byte identity (the buckets
        // preserve storage order; see `GoCoreIndex`).
        let index = GoCoreIndex::build(db);

        let mut function_node: BTreeMap<String, SemanticNodeId> = BTreeMap::new();
        let mut function_signature: BTreeMap<String, String> = BTreeMap::new();
        let mut methods_by_receiver: BTreeMap<String, Vec<GoRtaMethod>> = BTreeMap::new();
        // `qualified -> core FunctionId` for the caller-disambiguated callsite join
        // A Go callsite's `caller` is a `qualified` identity; resolve
        // it to the core `FunctionId` so `callsite_constraint_node` can bind the
        // provenance to the CALLER's own same-span call site, not a sibling's.
        let mut caller_function_id: BTreeMap<String, crate::core::FunctionId> = BTreeMap::new();

        for semantic_function in db.go_semantic_functions() {
            function_signature.insert(
                semantic_function.qualified.clone(),
                semantic_function.signature.clone(),
            );

            let Some(core_function) = matching_core_function_indexed(&index, semantic_function)
            else {
                continue;
            };
            caller_function_id.insert(semantic_function.qualified.clone(), core_function.id);

            let key = function_node_key(db, core_function);
            let Some(node) = function_node_by_stable_key.get(key.as_str()).copied() else {
                continue;
            };
            function_node.insert(semantic_function.qualified.clone(), node);

            // A method (receiver-bearing function) is an interface-invoke candidate
            // callee, indexed by its normalized receiver type. The core function names
            // a method `Receiver.Method` (e.g. "Dog.Speak"), but interface-invoke
            // resolution matches the BARE invoked method name (the dynamic-dispatch
            // discriminant, e.g. "Speak"), so index the bare method name here.
            if let Some(receiver) = semantic_function.receiver.as_deref() {
                methods_by_receiver
                    .entry(normalize_type(receiver))
                    .or_default()
                    .push(GoRtaMethod {
                        method_name: bare_method_name(&semantic_function.name),
                        qualified: semantic_function.qualified.clone(),
                        node,
                    });
            }
        }
        // Deterministic order within each receiver bucket (by qualified identity).
        for methods in methods_by_receiver.values_mut() {
            methods.sort_by(|left, right| left.qualified.cmp(&right.qualified));
            methods.dedup();
        }

        // Reachability roots that resolve to a Go function node, mapped to `qualified`.
        let mut roots: BTreeSet<String> = BTreeSet::new();
        for root in db.reachability_roots() {
            if root.language != Language::Go {
                continue;
            }
            if let Some(qualified) = qualified_for_function_id_indexed(&index, root.target_function)
            {
                roots.insert(qualified);
            }
        }

        // Method-sets: type_name -> methods (+ contributing fact key).
        let mut method_sets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut method_set_keys: BTreeMap<String, StableKeyId> = BTreeMap::new();
        for method_set in db.go_semantic_method_sets() {
            let type_name = normalize_type(&method_set.type_name);
            method_sets
                .entry(type_name.clone())
                .or_default()
                .extend(method_set.methods.iter().cloned());
            method_set_keys.insert(type_name, method_set.stable_key);
        }

        // Instantiated runtime types (the rapid-type set) + contributing fact key.
        let mut instantiated: BTreeSet<String> = BTreeSet::new();
        let mut instantiated_keys: BTreeMap<String, StableKeyId> = BTreeMap::new();
        for fact in db.go_semantic_instantiated_types() {
            let type_name = normalize_type(&fact.type_name);
            instantiated.insert(type_name.clone());
            instantiated_keys.insert(type_name, fact.stable_key);
        }

        // Address-taken functions (func-value candidates) + contributing fact key.
        let mut address_taken: BTreeSet<String> = BTreeSet::new();
        let mut address_taken_keys: BTreeMap<String, StableKeyId> = BTreeMap::new();
        for fact in db.go_semantic_address_taken() {
            address_taken.insert(fact.function.clone());
            address_taken_keys.insert(fact.function.clone(), fact.stable_key);
        }

        // Join dynamic-dispatch detail to its callsite by `callsite_stable_key`. Only
        // `UnresolvedDynamic` callsites that map to a `CallConstraint` node are RTA
        // obligations; the rest are statically resolved or unsupported (honest).
        let callsite_by_stable_key: BTreeMap<
            crate::core::StableKeyId,
            &crate::go::semantic::facts::GoSemanticCallsiteFact,
        > = db
            .go_semantic_callsites()
            .iter()
            .map(|callsite| (callsite.stable_key, callsite))
            .collect();

        let mut callsites: Vec<GoRtaCallsite> = Vec::new();
        for dispatch in db.go_semantic_dynamic_dispatch() {
            let Some(callsite) = callsite_by_stable_key.get(&dispatch.callsite_stable_key) else {
                continue;
            };
            if callsite.status != GoSemanticCallStatus::UnresolvedDynamic {
                continue;
            }
            // The callsite must map to a `CallConstraint` node the semantic graph
            // emitted; if it did not (e.g. no matching core call site), there is no
            // edge SOURCE/anchor — skip rather than fabricate. The join is disambiguated
            // by the caller's core `FunctionId` so a method-chain / mixed static+dynamic
            // shared span binds to the right sibling.
            let caller = caller_function_id.get(callsite.caller.as_str()).copied();
            let Some(callsite_node) =
                callsite_constraint_node_indexed(interner, &index, callsite, caller)
            else {
                continue;
            };
            callsites.push(GoRtaCallsite {
                caller: dispatch.caller.clone(),
                callsite_node,
                callsite_stable_key: callsite.stable_key,
                interface_method: dispatch.method.clone(),
                signature: dispatch.signature.clone(),
                dispatch_stable_key: dispatch.stable_key,
            });
        }
        // Deterministic callsite order (by resolved callsite stable text, then dispatch).
        callsites.sort_by(|left, right| {
            interner
                .compare_canonical(left.callsite_stable_key, right.callsite_stable_key)
                .then_with(|| {
                    interner.compare_canonical(left.dispatch_stable_key, right.dispatch_stable_key)
                })
        });

        // Static call graph: caller qualified -> statically-called callee
        // qualified, restricted to Go functions that map to a `function_node`. The Go
        // frontend emits `static_callee` as the callee's `ssa.Function.String()`, which
        // is the SAME identity format as a function's `qualified` (the `function_node`
        // key), so a `ResolvedStatic` callsite's `static_callee` is looked up directly.
        // An unresolvable static callee (no known function identity) is skipped — honest,
        // never fabricated. Reachability closes over these edges so dispatch in a
        // statically-reached (non-root) function is resolved; static edges GROW
        // reachability only and never emit a derived edge.
        let mut static_call_targets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for callsite in db.go_semantic_callsites() {
            if callsite.status != GoSemanticCallStatus::ResolvedStatic {
                continue;
            }
            let Some(callee) = callsite.static_callee.as_deref() else {
                continue;
            };
            // Both endpoints must be Go functions present in the function index, or there
            // is no reachable-graph node to grow (honest skip, not a fabricated edge).
            if !function_node.contains_key(callee) || !function_node.contains_key(&callsite.caller)
            {
                continue;
            }
            static_call_targets
                .entry(callsite.caller.clone())
                .or_default()
                .insert(callee.to_string());
        }

        Self {
            roots,
            callsites,
            method_sets,
            method_set_keys,
            instantiated,
            instantiated_keys,
            address_taken,
            address_taken_keys,
            function_node,
            function_signature,
            methods_by_receiver,
            // Built ONCE by `finalize_indexes` below from the now-populated primary fields
            // so per-callsite resolution is a map lookup rather than an
            // O(whole-instantiated-set) scan.
            interface_candidate_index: BTreeMap::new(),
            static_call_targets,
        }
        .finalize_indexes()
    }
}

/// Indexes built ONCE per [`GoRtaInputs::from_db`] so the core-function and call-site
/// joins are bucket lookups, not full linear scans of every core fact in the repo
/// to keep `from_db` near-linear rather than O(F_go · F_total). Each Go semantic
/// function lookup is limited to its `(file, name)` bucket, including the zero-width
/// point fallback, and each dynamic-dispatch row is limited to its `(file, span)` callsite
/// bucket. The indexes are restricted to Go facts and bucketed by `(file, name)` /
/// `(file, span)`, so a join touches only the handful of same-`(file, name)` candidates.
///
/// Determinism contract (byte identity, not just speed): each bucket's `Vec` preserves
/// `db.functions()` / `db.call_sites()` storage order. The reference scans use `.find()`
/// (first match in storage order for the exact-span case) and `Iterator::min_by_key`
/// (which returns the FIRST element achieving the minimum). Preserving storage order
/// within the bucket makes the indexed `.find()` / `.min_by_key()` pick the exact same
/// element the full-slice scan did — the result is identical in every case, ties
/// included.
struct GoCoreIndex<'a> {
    /// `(file, name)` -> the Go core `FunctionFact`s with that file+name, in
    /// `db.functions()` storage order. Restricted to `Language::Go`. Keyed by an owned
    /// `String` name (not a borrow of `db`) so a lookup by a shorter-lived `&str` query is
    /// borrow-clean; the few owned-key allocations per build are O(F_go), not O(F_go²).
    core_functions_by_file_name: BTreeMap<(FileId, String), Vec<&'a FunctionFact>>,
    /// `FunctionId` -> its core `FunctionFact` (for the reverse `qualified_for_function_id`
    /// lookup, avoiding a `db.functions().iter().find(id == ..)` per reachability root).
    core_function_by_id: BTreeMap<crate::core::FunctionId, &'a FunctionFact>,
    /// `(file, name)` -> the Go semantic functions with that file+name, in
    /// `db.go_semantic_functions()` storage order (for `qualified_for_function_id`'s
    /// reverse scan). Only semantic functions that carry a `file` are indexed.
    semantic_functions_by_file_name: BTreeMap<(FileId, String), Vec<&'a GoSemanticFunctionFact>>,
    /// `(file, start_byte, end_byte)` -> the Go core call sites at that exact span, in
    /// `db.call_sites()` storage order (for `callsite_constraint_node`). Restricted to
    /// `Language::Go`.
    call_sites_by_file_span:
        BTreeMap<(FileId, u32, u32), Vec<&'a crate::analysis_neutral::calls::facts::CallSiteFact>>,
    /// Callsite-kind semantic node stable key -> its `SemanticNodeId`, so the final
    /// callsite-node lookup in `callsite_constraint_node` is a map hit, not a
    /// `db.semantic_nodes().iter().find(...)` per dynamic-dispatch row. Keeps the FIRST
    /// occurrence to mirror reference `.find()` semantics (stable keys are unique in practice).
    callsite_node_by_stable_key: BTreeMap<String, SemanticNodeId>,
}

impl<'a> GoCoreIndex<'a> {
    /// Build all four indexes from the db in a single pass each, preserving storage order
    /// within every bucket (the determinism contract above).
    fn build(db: &'a AnalysisDb) -> Self {
        let mut core_functions_by_file_name: BTreeMap<(FileId, String), Vec<&'a FunctionFact>> =
            BTreeMap::new();
        let mut core_function_by_id: BTreeMap<crate::core::FunctionId, &'a FunctionFact> =
            BTreeMap::new();
        for function in db.functions() {
            core_function_by_id.insert(function.id, function);
            if function.language == Language::Go {
                core_functions_by_file_name
                    .entry((function.file, function.name.clone()))
                    .or_default()
                    .push(function);
            }
        }

        let mut semantic_functions_by_file_name: BTreeMap<
            (FileId, String),
            Vec<&'a GoSemanticFunctionFact>,
        > = BTreeMap::new();
        for semantic_function in db.go_semantic_functions() {
            if let Some(file) = semantic_function.file {
                semantic_functions_by_file_name
                    .entry((file, semantic_function.name.clone()))
                    .or_default()
                    .push(semantic_function);
            }
        }

        let mut call_sites_by_file_span: BTreeMap<
            (FileId, u32, u32),
            Vec<&'a crate::analysis_neutral::calls::facts::CallSiteFact>,
        > = BTreeMap::new();
        for site in db.call_sites() {
            if site.language == Language::Go {
                call_sites_by_file_span
                    .entry((site.file, site.span.start_byte, site.span.end_byte))
                    .or_default()
                    .push(site);
            }
        }

        let mut callsite_node_by_stable_key: BTreeMap<String, SemanticNodeId> = BTreeMap::new();
        for node in db.semantic_nodes() {
            if matches!(node.kind, NodeKind::Callsite(_)) {
                // First-occurrence wins, matching reference `.find()` semantics.
                callsite_node_by_stable_key
                    .entry(db.resolve_stable_key(node.stable_key).to_string())
                    .or_insert(node.id);
            }
        }

        Self {
            core_functions_by_file_name,
            core_function_by_id,
            semantic_functions_by_file_name,
            call_sites_by_file_span,
            callsite_node_by_stable_key,
        }
    }

    /// The Go core `FunctionFact`s with this `(file, name)`, in storage order (empty slice
    /// if none). The disambiguation predicates already require `file`/`name`/`Go`, so
    /// scanning this bucket is equivalent to scanning the full `db.functions()` slice.
    fn core_functions(&self, file: FileId, name: &str) -> &[&'a FunctionFact] {
        self.core_functions_by_file_name
            .get(&(file, name.to_string()))
            .map_or(&[], Vec::as_slice)
    }

    /// The Go semantic functions with this `(file, name)`, in storage order (empty slice
    /// if none).
    fn semantic_functions(&self, file: FileId, name: &str) -> &[&'a GoSemanticFunctionFact] {
        self.semantic_functions_by_file_name
            .get(&(file, name.to_string()))
            .map_or(&[], Vec::as_slice)
    }

    /// The Go core call sites at this exact `(file, span)`, in storage order (empty slice
    /// if none).
    fn call_sites_at(
        &self,
        file: FileId,
        span: &Span,
    ) -> &[&'a crate::analysis_neutral::calls::facts::CallSiteFact] {
        self.call_sites_by_file_span
            .get(&(file, span.start_byte, span.end_byte))
            .map_or(&[], Vec::as_slice)
    }

    /// The `SemanticNodeId` of the callsite-kind node with this stable key, if interned.
    fn callsite_node(&self, stable_key: &str) -> Option<SemanticNodeId> {
        self.callsite_node_by_stable_key.get(stable_key).copied()
    }
}

/// Normalize a Go type identity for matching: strip a single leading `*` so a
/// pointer receiver (`*pkg.T`) matches the value type-name (`pkg.T`) used by the
/// instantiated-type / method-set facts. Idempotent and honest (no other rewriting).
pub(crate) fn normalize_type(type_name: &str) -> String {
    type_name.strip_prefix('*').unwrap_or(type_name).to_string()
}

/// The bare method identifier from a core function name. The core (tree-sitter) facts
/// name a method `Receiver.Method` (e.g. "Dog.Speak"); interface-invoke resolution
/// matches the BARE invoked method name (the dynamic-dispatch discriminant, e.g.
/// "Speak"). A plain function name (no `.`) is already bare and returned unchanged.
fn bare_method_name(name: &str) -> String {
    name.rsplit_once('.')
        .map_or(name, |(_, method)| method)
        .to_string()
}

/// The `qualified` identity of the Go semantic function matching a core `FunctionId`,
/// if any. This is the exact INVERSE of [`matching_core_function_indexed`]: it returns
/// the semantic function `S` such that `matching_core_function_indexed(S) == function_id`,
/// so the two directions are in genuine lockstep rather than merely
/// "similar". Defining the reverse via the forward is what keeps the point-containment
/// fallback symmetric: the forward maps a zero-width SSA point to the INNERMOST
/// containing core declaration (`min_by_key`), so the reverse must NOT map an OUTER
/// same-named declaration to a point that actually belongs to a tighter inner one.
/// Otherwise both declarations could match the same contained point and break symmetry.
#[cfg(test)]
fn qualified_for_function_id(
    db: &AnalysisDb,
    function_id: crate::core::FunctionId,
) -> Option<String> {
    qualified_for_function_id_indexed(&GoCoreIndex::build(db), function_id)
}

/// Indexed [`qualified_for_function_id`]: the hot-path variant `from_db` calls per
/// reachability root, reusing the prebuilt [`GoCoreIndex`] (the core function is found by
/// id, and the reverse semantic-function scan is restricted to the `(file, name)` bucket
/// in storage order). Identical resolution order to the `db`-based version, so the
/// forward/reverse lockstep is preserved.
fn qualified_for_function_id_indexed(
    index: &GoCoreIndex<'_>,
    function_id: crate::core::FunctionId,
) -> Option<String> {
    let function = *index.core_function_by_id.get(&function_id)?;
    let bucket = index.semantic_functions(function.file, &function.name);

    // 1. Prefer a semantic function with an EXACT-equal span (unambiguous, and the forward
    //    direction's step 1 — so this side agrees). `.find()` over the storage-ordered
    //    bucket preserves first-match-in-storage-order semantics; the bucket already
    //    enforces the `file == Some(function.file) && name == function.name` identity.
    if let Some(exact) = bucket.iter().copied().find(|semantic_function| {
        semantic_function.span.as_ref().is_some_and(|span| {
            span.start_byte == function.span.start_byte && span.end_byte == function.span.end_byte
        })
    }) {
        return Some(exact.qualified.clone());
    }

    // 2. Fall back to a zero-width semantic POINT (the SSA-method case), but ONLY when that
    //    point maps FORWARD to exactly THIS core declaration — i.e. this function is the
    //    INNERMOST core declaration containing the point (mirroring the forward
    //    `min_by_key`). This is what makes the two directions symmetric: an outer same-named
    //    declaration cannot claim a point that resolves to a tighter inner one. Deterministic
    //    minimum by `qualified` if several semantic points legitimately resolve here.
    bucket
        .iter()
        .copied()
        .filter(|semantic_function| {
            let Some(span) = semantic_function.span.as_ref() else {
                return false;
            };
            if span.start_byte != span.end_byte {
                return false;
            }
            // The forward map of this point must land on THIS core function (innermost).
            matching_core_function_for_indexed(index, function.file, &function.name, span)
                .is_some_and(|resolved| resolved.id == function_id)
        })
        .map(|semantic_function| semantic_function.qualified.clone())
        .min()
}

/// The core `FunctionFact` matching a Go semantic function (file + language + name +
/// span), resolved through the prebuilt [`GoCoreIndex`] (the hot-path join `from_db`
/// runs once per Go semantic function instead of a fresh `db.functions()`
/// scan).
///
/// The Go SSA frontend and the core (tree-sitter) facts disagree on the span of a
/// METHOD: tree-sitter reports the FULL declaration span (`func (r R) M() {...}`),
/// while the SSA frontend reports a zero-width POINT at the `func` token. To bridge
/// that gap WITHOUT over-matching, prefer an EXACT span match (regular functions and
/// any frontend that already reports the declaration span) and fall back to
/// point-in-declaration CONTAINMENT only when the semantic span is a zero-width point
/// (the documented SSA-method case). Containment is not a unique relation — two
/// same-named declarations whose byte ranges nest could otherwise mis-map under a
/// first-match-wins scan, so the fallback both requires a zero-width
/// point and is the narrowest match available. Honest by construction: `name` + `file`
/// disambiguate and the exact match is tried first.
fn matching_core_function_indexed<'a>(
    index: &GoCoreIndex<'a>,
    semantic_function: &GoSemanticFunctionFact,
) -> Option<&'a FunctionFact> {
    let file = semantic_function.file?;
    let span = semantic_function.span.as_ref()?;
    matching_core_function_for_indexed(index, file, &semantic_function.name, span)
}

/// Shared file+language+name+span disambiguation for mapping a Go semantic span to its
/// core `FunctionFact` (used by both [`matching_core_function_indexed`] and
/// [`qualified_for_function_id`] so the two directions stay in lockstep).
///
/// Resolution order:
/// 1. EXACT span equality (`start == start && end == end`) — the precise, unambiguous
///    match for regular functions / declaration-span frontends.
/// 2. Only when the semantic span is a zero-width POINT (`start == end`, the SSA-method
///    case), the innermost core declaration CONTAINING that point. "Innermost" (the
///    narrowest containing span) is chosen deterministically so nested same-named
///    ranges resolve to the tightest enclosing declaration rather than a first-match.
#[cfg(test)]
fn matching_core_function_for<'a>(
    db: &'a AnalysisDb,
    file: FileId,
    name: &str,
    span: &Span,
) -> Option<&'a FunctionFact> {
    matching_core_function_for_indexed(&GoCoreIndex::build(db), file, name, span)
}

/// Indexed [`matching_core_function_for`]: identical resolution order, but scanning the
/// `(file, name)` bucket (storage-ordered) rather than the full `db.functions()` slice.
/// Because the bucket already restricts to same-`(file, name)` Go functions — exactly the
/// `same_identity` predicate — and preserves storage order, the indexed `.find()` /
/// `.min_by_key()` choose the SAME `FunctionFact` the full-slice scan did.
fn matching_core_function_for_indexed<'a>(
    index: &GoCoreIndex<'a>,
    file: FileId,
    name: &str,
    span: &Span,
) -> Option<&'a FunctionFact> {
    let bucket = index.core_functions(file, name);

    // 1. Prefer an exact-equal span (deterministic, unambiguous). `.find()` over the
    //    storage-ordered bucket mirrors the old first-match-in-storage-order semantics.
    if let Some(exact) = bucket.iter().copied().find(|function| {
        function.span.start_byte == span.start_byte && function.span.end_byte == span.end_byte
    }) {
        return Some(exact);
    }

    // 2. Fall back to point-in-declaration ONLY for a zero-width semantic point (the
    //    documented SSA-method case), choosing the INNERMOST containing declaration so
    //    a nested same-named range cannot mis-map to an outer one.
    if span.start_byte != span.end_byte {
        return None;
    }
    bucket
        .iter()
        .copied()
        .filter(|function| {
            function.span.start_byte <= span.start_byte && span.start_byte <= function.span.end_byte
        })
        .min_by_key(|function| {
            function
                .span
                .end_byte
                .saturating_sub(function.span.start_byte)
        })
}

/// Reproduces `semantic_graph::build::function_node_key` so a Go function can be
/// looked up in the already-built `polint.semantic_graph` function nodes by stable
/// key. Keep in lockstep with the builder recipe (node-kind label + path + name +
/// span identity, `FactFamily::Function`).
fn function_node_key(db: &AnalysisDb, function: &FunctionFact) -> String {
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

/// The unified `CallConstraint` callsite node for a Go callsite, reconstructed from
/// the `callsite` node-key recipe `semantic_graph::build` used. Returns `None` when
/// the callsite was not interned as a node (honest — no edge anchor). Resolved through
/// the prebuilt [`GoCoreIndex`]: the span-matched candidate set comes from the
/// `(file, span)` bucket (storage-ordered, restricted to Go) instead of a
/// `db.call_sites().iter().filter(...)` scan, and the final callsite-node resolution is a
/// stable-key map hit instead of a `db.semantic_nodes().iter().find(...)` scan. This keeps
/// the per-dynamic-dispatch-row join near-linear over all rows. `select_constraint_callsite` runs over the same candidate set, so the
/// disambiguation and resulting node remain byte-identical to the reference scan.
///
/// The core-call-site match must be disambiguated rather than first-match.
/// Several core call sites can share one byte span — a method chain `a().b()` reports
/// nested call expressions at the same outer span, and a single span can host both a
/// resolved-static and a dynamic call. Matching by `(file, language, span)` alone could
/// bind a dynamic Go callsite's provenance to the wrong sibling (e.g. the static
/// receiver call rather than the dynamic invoke), or coalesce two dynamic sites. So the
/// match is constrained to the call site whose `caller` maps back to the semantic
/// callsite's caller (`caller`, resolved to a core `FunctionId`) and, among those,
/// prefers a dynamic-dispatch-status site, choosing the deterministic minimum by stable
/// key rather than the first scanned.
fn callsite_constraint_node_indexed(
    interner: &crate::core::StableKeyInterner,
    index: &GoCoreIndex<'_>,
    callsite: &crate::go::semantic::facts::GoSemanticCallsiteFact,
    caller: Option<crate::core::FunctionId>,
) -> Option<SemanticNodeId> {
    let file = callsite.file?;
    let span = callsite.span.as_ref()?;
    let candidates = index.call_sites_at(file, span);
    let core_callsite = select_constraint_callsite(interner, candidates, caller)?;
    let node_key = node_key_from_identity("callsite", &interner.resolve(core_callsite.stable_key));
    index.callsite_node(&node_key)
}

/// Disambiguate a set of span-matched core call sites to the one that anchors a dynamic
/// Go callsite's provenance. Pure (no db) so the disambiguation is
/// unit-testable in isolation.
///
/// Resolution order (each step narrows; a step that would empty the set is skipped so
/// the join degrades gracefully rather than dropping a real anchor):
/// 1. restrict to sites whose `caller` equals the semantic callsite's caller (when that
///    caller resolved to a core `FunctionId`) — never bind to a different function's
///    sibling call at the same span;
/// 2. among those, prefer DYNAMIC-dispatch-status sites (`Unresolved` / `Ambiguous`) —
///    an `UnresolvedDynamic` Go callsite anchors on the dynamic site, not a co-located
///    static one;
/// 3. choose the deterministic minimum by `stable_key` (totally ordered, byte-stable),
///    never a first-match that depends on storage order.
fn select_constraint_callsite<'a>(
    interner: &crate::core::StableKeyInterner,
    candidates: &[&'a crate::analysis_neutral::calls::facts::CallSiteFact],
    caller: Option<crate::core::FunctionId>,
) -> Option<&'a crate::analysis_neutral::calls::facts::CallSiteFact> {
    use crate::analysis_neutral::calls::facts::CallTargetStatus;

    // 1. Prefer the caller's own call sites (skip if that would drop every candidate).
    let caller_matched: Vec<&'a crate::analysis_neutral::calls::facts::CallSiteFact> = match caller
    {
        Some(caller) => candidates
            .iter()
            .copied()
            .filter(|site| site.caller == caller)
            .collect(),
        None => Vec::new(),
    };
    let narrowed: &[&'a crate::analysis_neutral::calls::facts::CallSiteFact] =
        if caller_matched.is_empty() {
            candidates
        } else {
            &caller_matched
        };

    // 2. Among those, prefer dynamic-dispatch-status sites (skip if none).
    let is_dynamic = |site: &&crate::analysis_neutral::calls::facts::CallSiteFact| {
        matches!(
            site.status,
            CallTargetStatus::Unresolved | CallTargetStatus::Ambiguous
        )
    };
    let dynamic: Vec<&'a crate::analysis_neutral::calls::facts::CallSiteFact> =
        narrowed.iter().copied().filter(is_dynamic).collect();
    let final_set: &[&'a crate::analysis_neutral::calls::facts::CallSiteFact] =
        if dynamic.is_empty() {
            narrowed
        } else {
            &dynamic
        };

    // 3. Deterministic minimum by stable key (never first-match on storage order).
    final_set
        .iter()
        .copied()
        .min_by(|left, right| interner.compare_canonical(left.stable_key, right.stable_key))
}

/// Reproduces `semantic_graph::build::node_key_from_identity` (node-kind label +
/// identity, `FactFamily::Scope`). Keep in lockstep with the builder recipe.
fn node_key_from_identity(node_kind: &str, identity: &str) -> String {
    semantic_stable_key(
        FactFamily::Scope,
        &[
            ("node_kind", node_kind.to_string()),
            ("identity", identity.to_string()),
        ],
    )
    .into_string()
}

/// Reproduces `semantic_graph::build::span_identity`.
fn span_identity(span: &crate::core::Span) -> String {
    format!("{}:{}..{}", span.file.0, span.start_byte, span.end_byte)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::analysis_neutral::semantic_graph::facts::{SemanticNodeFact, SemanticPrecision};
    use crate::analysis_neutral::semantic_graph::store::SemanticGraphOutput;
    use crate::core::{FunctionId, Span};
    use crate::go::semantic::facts::{
        GoSemanticFunctionFact, GoSemanticFunctionKind, GoSemanticInstantiatedTypeFact,
        GoSemanticMethodSetFact,
    };
    use crate::go::semantic::store::GoSemanticFactsOutput;

    fn span(file: crate::core::FileId, start: u32, end: u32) -> Span {
        Span::new(file, start, end, 1, 1, 1, 1)
    }

    /// `from_db` joins the stored Go-frontend facts to the already-built semantic-graph
    /// function nodes: a Go function maps to its node, its receiver indexes it as a
    /// method, and the instantiated-type / method-set facts populate the RTA sets.
    #[test]
    fn from_db_maps_go_function_to_its_semantic_node_and_indexes_methods() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("pkg/file.go"),
            "pkg/file.go".to_string(),
            "package pkg\n".to_string(),
        );
        let method_span = span(file, 10, 40);

        // Core function for the concrete method (pkg.File).Read.
        let function_id = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "Read".to_string(),
            method_span.clone(),
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));

        // The semantic-graph function node, keyed exactly like the builder would key it.
        let node_stable_key = function_node_key(
            &db,
            db.functions().iter().find(|f| f.id == function_id).unwrap(),
        );
        db.replace_semantic_graph_facts(SemanticGraphOutput {
            nodes: vec![SemanticNodeFact {
                id: SemanticNodeId(0),
                kind: NodeKind::Function(function_id),
                precision: SemanticPrecision::Conservative,
                stable_key: db.stable_key_interner().intern(node_stable_key.clone()),
            }],
            edges: Vec::new(),
            constraints: Vec::new(),
        })
        .expect("semantic graph stores");
        let expected_node = db
            .semantic_nodes()
            .iter()
            .find(|node| db.resolve_stable_key(node.stable_key).as_ref() == node_stable_key)
            .expect("function node stored")
            .id;

        // Go-frontend facts: the function (matching the core function), an instantiated
        // type, and its method-set.
        let interner = db.stable_key_interner();
        db.replace_go_semantic_facts(GoSemanticFactsOutput {
            functions: vec![GoSemanticFunctionFact {
                id: crate::go::semantic::facts::GoSemanticFunctionId(0),
                stable_key: interner.intern("gofn|(pkg.File).Read"),
                package_id: "pkg".to_string(),
                package_path: "pkg".to_string(),
                name: "Read".to_string(),
                qualified: "(pkg.File).Read".to_string(),
                signature: "func() (int, error)".to_string(),
                kind: GoSemanticFunctionKind::Method,
                receiver: Some("*pkg.File".to_string()),
                relative_file: Some("pkg/file.go".to_string()),
                file: Some(file),
                span: Some(method_span),
            }],
            instantiated_types: vec![GoSemanticInstantiatedTypeFact {
                id: crate::go::semantic::facts::GoSemanticInstantiatedTypeId(0),
                stable_key: interner.intern("inst|pkg.File"),
                package_id: "pkg".to_string(),
                package_path: "pkg".to_string(),
                type_name: "pkg.File".to_string(),
            }],
            method_sets: vec![GoSemanticMethodSetFact {
                id: crate::go::semantic::facts::GoSemanticMethodSetId(0),
                stable_key: interner.intern("ms|pkg.File"),
                package_id: "pkg".to_string(),
                package_path: "pkg".to_string(),
                type_name: "pkg.File".to_string(),
                methods: vec!["Read".to_string()],
            }],
            ..GoSemanticFactsOutput::default()
        })
        .expect("go semantic facts store");

        let inputs = GoRtaInputs::from_db(&db.stable_key_interner(), &db);

        // The Go function maps to its semantic node.
        assert_eq!(
            inputs.function_node.get("(pkg.File).Read"),
            Some(&expected_node)
        );
        // The pointer receiver is normalized to the value type-name and indexes the
        // concrete method for interface-invoke resolution.
        let methods = inputs
            .methods_by_receiver
            .get("pkg.File")
            .expect("methods indexed by normalized receiver");
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].method_name, "Read");
        assert_eq!(methods[0].node, expected_node);
        // The instantiated-type + method-set sets are populated (normalized type-name).
        assert!(inputs.instantiated.contains("pkg.File"));
        assert!(
            inputs
                .method_sets
                .get("pkg.File")
                .is_some_and(|methods| methods.contains("Read"))
        );
        // The function signature is recorded for func-value matching.
        assert_eq!(
            inputs
                .function_signature
                .get("(pkg.File).Read")
                .map(String::as_str),
            Some("func() (int, error)")
        );
    }

    #[test]
    fn normalize_type_strips_single_leading_pointer() {
        assert_eq!(normalize_type("*pkg.T"), "pkg.T");
        assert_eq!(normalize_type("pkg.T"), "pkg.T");
        // Only a single leading `*` is stripped (honest, idempotent on the result).
        assert_eq!(normalize_type("**pkg.T"), "*pkg.T");
    }

    #[test]
    fn bare_method_name_strips_receiver_qualifier() {
        // The core (tree-sitter) facts name a method `Receiver.Method`; interface-invoke
        // resolution matches the bare invoked method name.
        assert_eq!(bare_method_name("Dog.Speak"), "Speak");
        // A plain function name (no `.`) is already bare.
        assert_eq!(bare_method_name("handler"), "handler");
        // Only the receiver qualifier is stripped (last `.` segment is the method).
        assert_eq!(bare_method_name("Outer.Inner.Method"), "Method");
    }

    #[test]
    fn from_db_maps_method_when_ssa_point_span_lies_within_core_declaration_span() {
        // The SSA frontend reports a
        // zero-width POINT span for a method (`func` token), while tree-sitter reports
        // the FULL declaration span. `matching_core_function` must map the method by
        // file + name + span-CONTAINMENT, or interface dispatch resolves nothing.
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("pkg/file.go"),
            "pkg/file.go".to_string(),
            "package pkg\n".to_string(),
        );
        // Core method declaration spans bytes 10..40 (the full `func (D) Speak() {...}`).
        let function_id = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "Dog.Speak".to_string(),
            span(file, 10, 40),
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));
        let node_stable_key = function_node_key(
            &db,
            db.functions().iter().find(|f| f.id == function_id).unwrap(),
        );
        db.replace_semantic_graph_facts(SemanticGraphOutput {
            nodes: vec![SemanticNodeFact {
                id: SemanticNodeId(0),
                kind: NodeKind::Function(function_id),
                precision: SemanticPrecision::Conservative,
                stable_key: db.stable_key_interner().intern(node_stable_key),
            }],
            edges: Vec::new(),
            constraints: Vec::new(),
        })
        .expect("semantic graph stores");

        let interner = db.stable_key_interner();
        db.replace_go_semantic_facts(GoSemanticFactsOutput {
            functions: vec![GoSemanticFunctionFact {
                id: crate::go::semantic::facts::GoSemanticFunctionId(0),
                stable_key: interner.intern("gofn|(pkg.Dog).Speak"),
                package_id: "pkg".to_string(),
                package_path: "pkg".to_string(),
                // The frontend names the method `Receiver.Method` and reports a point
                // span (25..25) WITHIN the core declaration span (10..40).
                name: "Dog.Speak".to_string(),
                qualified: "(*pkg.Dog).Speak".to_string(),
                signature: "func() string".to_string(),
                kind: GoSemanticFunctionKind::Method,
                receiver: Some("*pkg.Dog".to_string()),
                relative_file: Some("pkg/file.go".to_string()),
                file: Some(file),
                span: Some(span(file, 25, 25)),
            }],
            method_sets: vec![GoSemanticMethodSetFact {
                id: crate::go::semantic::facts::GoSemanticMethodSetId(0),
                stable_key: interner.intern("ms|pkg.Dog"),
                package_id: "pkg".to_string(),
                package_path: "pkg".to_string(),
                type_name: "pkg.Dog".to_string(),
                methods: vec!["Speak".to_string()],
            }],
            ..GoSemanticFactsOutput::default()
        })
        .expect("go semantic facts store");

        let inputs = GoRtaInputs::from_db(&db.stable_key_interner(), &db);

        // The method maps to its semantic node despite the point-vs-declaration span gap.
        assert_eq!(
            inputs.function_node.get("(*pkg.Dog).Speak"),
            Some(&SemanticNodeId(0))
        );
        // It is indexed by its normalized receiver with the BARE method name "Speak", so
        // an interface invoke of "Speak" can resolve to it.
        let methods = inputs
            .methods_by_receiver
            .get("pkg.Dog")
            .expect("method indexed by normalized receiver");
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].method_name, "Speak");
        assert_eq!(methods[0].node, SemanticNodeId(0));
    }

    /// An exact span match must win even when a same-named core function's range
    /// CONTAINS the semantic span. Two same-named declarations are pushed — an outer one
    /// spanning 0..100 and an inner one spanning 40..60. A semantic span of exactly
    /// 40..60 must map to the INNER function, never the containing outer one (which a
    /// first-match-wins containment scan could wrongly pick).
    #[test]
    fn matching_core_function_prefers_exact_span_over_containing_same_name() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("pkg/file.go"),
            "pkg/file.go".to_string(),
            "package pkg\n".to_string(),
        );
        let push = |db: &mut AnalysisDb, id: u64, start: u32, end: u32| {
            db.push_function(FunctionFact::new(
                FunctionId::from_raw(id),
                file,
                "F".to_string(),
                span(file, start, end),
                Language::Go,
                false,
                true,
                1,
                Vec::new(),
            ))
        };
        let outer = push(&mut db, 0, 0, 100);
        let inner = push(&mut db, 1, 40, 60);

        // An exact 40..60 span resolves to the inner function, not the containing outer.
        let matched = matching_core_function_for(&db, file, "F", &span(file, 40, 60))
            .expect("exact span must match");
        assert_eq!(matched.id, inner);
        assert_ne!(matched.id, outer);
    }

    /// A zero-width SSA point that lands inside TWO same-named containing
    /// declarations resolves to the INNERMOST (narrowest) one, deterministically, rather
    /// than the first scanned. Point 45 lies inside both 0..100 and 40..60; the narrower
    /// 40..60 wins.
    #[test]
    fn matching_core_function_point_fallback_picks_innermost_containing() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("pkg/file.go"),
            "pkg/file.go".to_string(),
            "package pkg\n".to_string(),
        );
        let push = |db: &mut AnalysisDb, id: u64, start: u32, end: u32| {
            db.push_function(FunctionFact::new(
                FunctionId::from_raw(id),
                file,
                "F".to_string(),
                span(file, start, end),
                Language::Go,
                false,
                true,
                1,
                Vec::new(),
            ))
        };
        let _outer = push(&mut db, 0, 0, 100);
        let inner = push(&mut db, 1, 40, 60);

        // A zero-width point (45..45) inside both ranges resolves to the innermost.
        let matched = matching_core_function_for(&db, file, "F", &span(file, 45, 45))
            .expect("point inside a declaration must match");
        assert_eq!(matched.id, inner);
    }

    fn call_site(
        interner: &crate::core::StableKeyInterner,
        id: u64,
        caller: FunctionId,
        start: u32,
        end: u32,
        status: crate::analysis_neutral::calls::facts::CallTargetStatus,
        stable_key: &str,
    ) -> crate::analysis_neutral::calls::facts::CallSiteFact {
        use crate::analysis_neutral::calls::facts::{CallCallee, CallPrecision, CallSyntaxKind};
        use crate::analysis_neutral::ids::{CallSiteId, MirBodyId, MirOpId};
        crate::analysis_neutral::calls::facts::CallSiteFact {
            in_throw: false,
            id: CallSiteId(id),
            language: Language::Go,
            file: FileId::from_raw(1),
            caller,
            owner_symbol: None,
            body: MirBodyId(0),
            operation: MirOpId(0),
            span: span(FileId::from_raw(1), start, end),
            kind: CallSyntaxKind::Method,
            callee: CallCallee::Unknown {
                reason:
                    crate::analysis_neutral::calls::facts::UnresolvedCallReason::InterfaceDispatch,
            },
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status,
            precision: CallPrecision::Conservative,
            stable_key: interner.intern(stable_key.to_string()),
        }
    }

    /// When several core call sites share one byte span, the
    /// disambiguator binds to the site whose `caller` matches the semantic callsite's
    /// caller AND that is a dynamic-dispatch-status site — never a sibling's call or a
    /// co-located static call at the same span.
    #[test]
    fn select_constraint_callsite_disambiguates_by_caller_and_dynamic_status() {
        use crate::analysis_neutral::calls::facts::CallTargetStatus;
        let caller_a = FunctionId::from_raw(10);
        let caller_b = FunctionId::from_raw(20);
        let interner = crate::core::StableKeyInterner::default();

        // All four share the SAME span (a method chain at one outer span). Among caller_a's
        // sites: one resolved-static receiver call + one dynamic invoke. caller_b also has a
        // dynamic site at the same span (a different function's sibling call).
        let static_a = call_site(
            &interner,
            1,
            caller_a,
            100,
            110,
            CallTargetStatus::Resolved,
            "cs|a-static",
        );
        let dynamic_a = call_site(
            &interner,
            2,
            caller_a,
            100,
            110,
            CallTargetStatus::Unresolved,
            "cs|a-dynamic",
        );
        let dynamic_b = call_site(
            &interner,
            3,
            caller_b,
            100,
            110,
            CallTargetStatus::Unresolved,
            "cs|b-dynamic",
        );
        let candidates = [&static_a, &dynamic_a, &dynamic_b];

        // The dynamic callsite belongs to caller_a → bind to caller_a's DYNAMIC site, not
        // its static sibling and not caller_b's same-span site.
        let chosen = select_constraint_callsite(&interner, &candidates, Some(caller_a))
            .expect("a caller-matched dynamic site must be selected");
        assert_eq!(chosen.id.0, 2, "must select caller_a's dynamic site");
        assert_eq!(interner.resolve(chosen.stable_key).as_ref(), "cs|a-dynamic");

        // For caller_b, only its own same-span dynamic site is eligible.
        let chosen_b = select_constraint_callsite(&interner, &candidates, Some(caller_b))
            .expect("caller_b's site must be selected");
        assert_eq!(chosen_b.id.0, 3);
    }

    /// With no dynamic site for the matched caller, fall back to that
    /// caller's site (a static one) rather than coalescing onto another caller's. And with
    /// an unresolved caller (None), the disambiguator still prefers a dynamic site and is
    /// deterministic (minimum stable key), never first-match on storage order.
    #[test]
    fn select_constraint_callsite_degrades_gracefully_and_is_deterministic() {
        use crate::analysis_neutral::calls::facts::CallTargetStatus;
        let caller_a = FunctionId::from_raw(10);
        let caller_b = FunctionId::from_raw(20);
        let interner = crate::core::StableKeyInterner::default();

        // Two dynamic sites at one span for DIFFERENT callers, plus a static one. Caller
        // unknown: prefer dynamic, deterministic minimum by stable key ("cs|x" < "cs|y").
        let dyn_y = call_site(
            &interner,
            3,
            caller_b,
            100,
            110,
            CallTargetStatus::Unresolved,
            "cs|y",
        );
        let dyn_x = call_site(
            &interner,
            2,
            caller_a,
            100,
            110,
            CallTargetStatus::Unresolved,
            "cs|x",
        );
        let static_z = call_site(
            &interner,
            1,
            caller_a,
            100,
            110,
            CallTargetStatus::Resolved,
            "cs|z-static",
        );
        let candidates = [&dyn_y, &dyn_x, &static_z];
        let chosen = select_constraint_callsite(&interner, &candidates, None)
            .expect("a dynamic site is selected when caller is unknown");
        assert_eq!(
            interner.resolve(chosen.stable_key).as_ref(),
            "cs|x",
            "deterministic minimum dynamic site, not first-scanned"
        );

        // Caller matches a site that has NO dynamic variant: fall back to that caller's
        // (static) site rather than another caller's dynamic one.
        let only_static_a = call_site(
            &interner,
            5,
            caller_a,
            100,
            110,
            CallTargetStatus::Resolved,
            "cs|a-only-static",
        );
        let other_dynamic = call_site(
            &interner,
            6,
            caller_b,
            100,
            110,
            CallTargetStatus::Unresolved,
            "cs|b-dyn",
        );
        let candidates2 = [&only_static_a, &other_dynamic];
        let chosen2 = select_constraint_callsite(&interner, &candidates2, Some(caller_a))
            .expect("caller_a's only site is selected");
        assert_eq!(
            chosen2.id.0, 5,
            "must bind to caller_a's own site, never caller_b's same-span dynamic one"
        );
    }

    /// The reverse map (`qualified_for_function_id`) must remain in lockstep as the
    /// exact INVERSE of the forward map's innermost (`min_by_key`) point-containment rule —
    /// not a first-match that lets an OUTER same-named declaration claim a point belonging to
    /// a tighter inner one. Two same-named core declarations (outer 0..100, inner 40..60) and
    /// ONE semantic function with a zero-width point at 45 (which the forward map resolves to
    /// the INNER declaration). The reverse must map the INNER core to that semantic
    /// `qualified` and the OUTER core to NOTHING (the point is not "innermost" for the
    /// outer) — proving forward/reverse symmetry.
    #[test]
    fn qualified_for_function_id_reverse_matches_forward_innermost_point() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("pkg/file.go"),
            "pkg/file.go".to_string(),
            "package pkg\n".to_string(),
        );
        let push = |db: &mut AnalysisDb, id: u64, start: u32, end: u32| {
            db.push_function(FunctionFact::new(
                FunctionId::from_raw(id),
                file,
                "F".to_string(),
                span(file, start, end),
                Language::Go,
                false,
                true,
                1,
                Vec::new(),
            ))
        };
        let outer = push(&mut db, 0, 0, 100);
        let inner = push(&mut db, 1, 40, 60);

        // One semantic function: a zero-width point at 45 (inside BOTH ranges). The forward
        // map resolves 45 to the INNER declaration (innermost), so the reverse must too.
        let interner = db.stable_key_interner();
        db.replace_go_semantic_facts(GoSemanticFactsOutput {
            functions: vec![GoSemanticFunctionFact {
                id: crate::go::semantic::facts::GoSemanticFunctionId(0),
                stable_key: interner.intern("gofn|(*pkg.T).F"),
                package_id: "pkg".to_string(),
                package_path: "pkg".to_string(),
                name: "F".to_string(),
                qualified: "(*pkg.T).F".to_string(),
                signature: "func()".to_string(),
                kind: GoSemanticFunctionKind::Method,
                receiver: Some("*pkg.T".to_string()),
                relative_file: Some("pkg/file.go".to_string()),
                file: Some(file),
                span: Some(span(file, 45, 45)),
            }],
            ..GoSemanticFactsOutput::default()
        })
        .expect("go semantic facts store");

        // Sanity: the FORWARD map sends point 45 to the inner declaration.
        let forward = matching_core_function_for(&db, file, "F", &span(file, 45, 45))
            .expect("forward point maps");
        assert_eq!(forward.id, inner);

        // The reverse map agrees: inner core -> the semantic qualified; outer core -> None.
        assert_eq!(
            qualified_for_function_id(&db, inner).as_deref(),
            Some("(*pkg.T).F"),
            "the innermost core must map back to the semantic point"
        );
        assert_eq!(
            qualified_for_function_id(&db, outer),
            None,
            "an OUTER same-named declaration must NOT claim a point that resolves to a tighter inner one (forward/reverse lockstep)"
        );
    }

    /// A non-zero-width semantic span that does not exactly equal any core span
    /// must NOT fall back to containment — the fallback is reserved for zero-width SSA
    /// points. A 45..50 span inside 0..100 (but not exactly equal) matches nothing.
    #[test]
    fn matching_core_function_rejects_inexact_nonpoint_span() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("pkg/file.go"),
            "pkg/file.go".to_string(),
            "package pkg\n".to_string(),
        );
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "F".to_string(),
            span(file, 0, 100),
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));
        // 45..50 is neither exact nor a zero-width point, so no match (no loose
        // containment for ranged spans).
        assert!(matching_core_function_for(&db, file, "F", &span(file, 45, 50)).is_none());
    }

    /// Indexing invariant: `matching_core_function_for` is keyed on `(file, name)` — two Go
    /// functions of the SAME NAME in DIFFERENT files must each resolve to the core
    /// function in their OWN file, never cross-matched/collapsed. This pins the
    /// per-file disambiguation the indexed lookup must preserve (a `(file, name)` bucket
    /// must never leak a different file's same-named function).
    #[test]
    fn matching_core_function_keys_on_file_so_same_name_in_other_file_is_excluded() {
        let mut db = AnalysisDb::new();
        let file_a = db.add_file(
            PathBuf::from("a/file.go"),
            "a/file.go".to_string(),
            "package a\n".to_string(),
        );
        let file_b = db.add_file(
            PathBuf::from("b/file.go"),
            "b/file.go".to_string(),
            "package b\n".to_string(),
        );
        // Same name "Handler", same byte span, but DIFFERENT files.
        let in_a = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file_a,
            "Handler".to_string(),
            span(file_a, 10, 40),
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));
        let in_b = db.push_function(FunctionFact::new(
            FunctionId::from_raw(1),
            file_b,
            "Handler".to_string(),
            span(file_b, 10, 40),
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));

        // Each file's span resolves to its OWN file's function — never the other file's.
        let matched_a = matching_core_function_for(&db, file_a, "Handler", &span(file_a, 10, 40))
            .expect("file_a's Handler must match");
        assert_eq!(matched_a.id, in_a);
        assert_eq!(matched_a.file, file_a);
        let matched_b = matching_core_function_for(&db, file_b, "Handler", &span(file_b, 10, 40))
            .expect("file_b's Handler must match");
        assert_eq!(matched_b.id, in_b);
        assert_eq!(matched_b.file, file_b);
    }

    /// Indexing invariant: TWO Go semantic functions of the SAME NAME in DIFFERENT
    /// files, each mapping to its own semantic node, must each resolve to the node in its
    /// OWN file — the `(file, name)` join must not collapse them. Asserts `function_node`
    /// carries both distinct `qualified` identities mapped to their own nodes.
    #[test]
    fn from_db_resolves_same_name_functions_in_different_files_to_their_own_nodes() {
        let mut db = AnalysisDb::new();
        let file_a = db.add_file(
            PathBuf::from("a/file.go"),
            "a/file.go".to_string(),
            "package a\n".to_string(),
        );
        let file_b = db.add_file(
            PathBuf::from("b/file.go"),
            "b/file.go".to_string(),
            "package b\n".to_string(),
        );
        let fn_a = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file_a,
            "Run".to_string(),
            span(file_a, 10, 40),
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));
        let fn_b = db.push_function(FunctionFact::new(
            FunctionId::from_raw(1),
            file_b,
            "Run".to_string(),
            span(file_b, 10, 40),
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));
        let key_a = function_node_key(&db, db.functions().iter().find(|f| f.id == fn_a).unwrap());
        let key_b = function_node_key(&db, db.functions().iter().find(|f| f.id == fn_b).unwrap());
        db.replace_semantic_graph_facts(SemanticGraphOutput {
            nodes: vec![
                SemanticNodeFact {
                    id: SemanticNodeId(0),
                    kind: NodeKind::Function(fn_a),
                    precision: SemanticPrecision::Conservative,
                    stable_key: db.stable_key_interner().intern(key_a.clone()),
                },
                SemanticNodeFact {
                    id: SemanticNodeId(1),
                    kind: NodeKind::Function(fn_b),
                    precision: SemanticPrecision::Conservative,
                    stable_key: db.stable_key_interner().intern(key_b.clone()),
                },
            ],
            edges: Vec::new(),
            constraints: Vec::new(),
        })
        .expect("semantic graph stores");
        let node_a = db
            .semantic_nodes()
            .iter()
            .find(|n| db.resolve_stable_key(n.stable_key).as_ref() == key_a)
            .unwrap()
            .id;
        let node_b = db
            .semantic_nodes()
            .iter()
            .find(|n| db.resolve_stable_key(n.stable_key).as_ref() == key_b)
            .unwrap()
            .id;

        let interner = db.stable_key_interner();
        db.replace_go_semantic_facts(GoSemanticFactsOutput {
            functions: vec![
                GoSemanticFunctionFact {
                    id: crate::go::semantic::facts::GoSemanticFunctionId(0),
                    stable_key: interner.intern("gofn|a.Run"),
                    package_id: "a".to_string(),
                    package_path: "a".to_string(),
                    name: "Run".to_string(),
                    qualified: "a.Run".to_string(),
                    signature: "func()".to_string(),
                    kind: GoSemanticFunctionKind::Function,
                    receiver: None,
                    relative_file: Some("a/file.go".to_string()),
                    file: Some(file_a),
                    span: Some(span(file_a, 10, 40)),
                },
                GoSemanticFunctionFact {
                    id: crate::go::semantic::facts::GoSemanticFunctionId(1),
                    stable_key: interner.intern("gofn|b.Run"),
                    package_id: "b".to_string(),
                    package_path: "b".to_string(),
                    name: "Run".to_string(),
                    qualified: "b.Run".to_string(),
                    signature: "func()".to_string(),
                    kind: GoSemanticFunctionKind::Function,
                    receiver: None,
                    relative_file: Some("b/file.go".to_string()),
                    file: Some(file_b),
                    span: Some(span(file_b, 10, 40)),
                },
            ],
            ..GoSemanticFactsOutput::default()
        })
        .expect("go semantic facts store");

        let inputs = GoRtaInputs::from_db(&db.stable_key_interner(), &db);
        // Each same-named function maps to the node in its OWN file (not collapsed).
        assert_eq!(inputs.function_node.get("a.Run"), Some(&node_a));
        assert_eq!(inputs.function_node.get("b.Run"), Some(&node_b));
        assert_ne!(node_a, node_b);
    }

    /// Indexing invariant: a METHOD and a free FUNCTION sharing the BARE name `Speak` (the method
    /// is `Dog.Speak`, the free function is `Speak`) must each resolve to their own core
    /// function/node, and only the method is indexed by receiver. Pins that the
    /// name-keyed join distinguishes `Dog.Speak` from `Speak` (the core `name` field
    /// differs, so the `(file, name)` buckets are distinct).
    #[test]
    fn from_db_distinguishes_method_and_free_function_sharing_a_bare_name() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("pkg/file.go"),
            "pkg/file.go".to_string(),
            "package pkg\n".to_string(),
        );
        // Core method `Dog.Speak` (10..40) and core free function `Speak` (50..80).
        let method_id = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "Dog.Speak".to_string(),
            span(file, 10, 40),
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));
        let free_id = db.push_function(FunctionFact::new(
            FunctionId::from_raw(1),
            file,
            "Speak".to_string(),
            span(file, 50, 80),
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));
        let method_key = function_node_key(
            &db,
            db.functions().iter().find(|f| f.id == method_id).unwrap(),
        );
        let free_key = function_node_key(
            &db,
            db.functions().iter().find(|f| f.id == free_id).unwrap(),
        );
        db.replace_semantic_graph_facts(SemanticGraphOutput {
            nodes: vec![
                SemanticNodeFact {
                    id: SemanticNodeId(0),
                    kind: NodeKind::Function(method_id),
                    precision: SemanticPrecision::Conservative,
                    stable_key: db.stable_key_interner().intern(method_key.clone()),
                },
                SemanticNodeFact {
                    id: SemanticNodeId(1),
                    kind: NodeKind::Function(free_id),
                    precision: SemanticPrecision::Conservative,
                    stable_key: db.stable_key_interner().intern(free_key.clone()),
                },
            ],
            edges: Vec::new(),
            constraints: Vec::new(),
        })
        .expect("semantic graph stores");
        let method_node = db
            .semantic_nodes()
            .iter()
            .find(|n| db.resolve_stable_key(n.stable_key).as_ref() == method_key)
            .unwrap()
            .id;
        let free_node = db
            .semantic_nodes()
            .iter()
            .find(|n| db.resolve_stable_key(n.stable_key).as_ref() == free_key)
            .unwrap()
            .id;

        let interner = db.stable_key_interner();
        db.replace_go_semantic_facts(GoSemanticFactsOutput {
            functions: vec![
                GoSemanticFunctionFact {
                    id: crate::go::semantic::facts::GoSemanticFunctionId(0),
                    stable_key: interner.intern("gofn|(pkg.Dog).Speak"),
                    package_id: "pkg".to_string(),
                    package_path: "pkg".to_string(),
                    name: "Dog.Speak".to_string(),
                    qualified: "(pkg.Dog).Speak".to_string(),
                    signature: "func() string".to_string(),
                    kind: GoSemanticFunctionKind::Method,
                    receiver: Some("*pkg.Dog".to_string()),
                    relative_file: Some("pkg/file.go".to_string()),
                    file: Some(file),
                    span: Some(span(file, 10, 40)),
                },
                GoSemanticFunctionFact {
                    id: crate::go::semantic::facts::GoSemanticFunctionId(1),
                    stable_key: interner.intern("gofn|pkg.Speak"),
                    package_id: "pkg".to_string(),
                    package_path: "pkg".to_string(),
                    name: "Speak".to_string(),
                    qualified: "pkg.Speak".to_string(),
                    signature: "func()".to_string(),
                    kind: GoSemanticFunctionKind::Function,
                    receiver: None,
                    relative_file: Some("pkg/file.go".to_string()),
                    file: Some(file),
                    span: Some(span(file, 50, 80)),
                },
            ],
            ..GoSemanticFactsOutput::default()
        })
        .expect("go semantic facts store");

        let inputs = GoRtaInputs::from_db(&db.stable_key_interner(), &db);
        // The method and the free function each map to their OWN node (the name-keyed
        // join distinguishes `Dog.Speak` from `Speak`).
        assert_eq!(
            inputs.function_node.get("(pkg.Dog).Speak"),
            Some(&method_node)
        );
        assert_eq!(inputs.function_node.get("pkg.Speak"), Some(&free_node));
        assert_ne!(method_node, free_node);
        // Only the METHOD is indexed by receiver (the free function has no receiver), and
        // it carries the bare method name `Speak`.
        let methods = inputs
            .methods_by_receiver
            .get("pkg.Dog")
            .expect("the method is indexed by its receiver");
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].method_name, "Speak");
        assert_eq!(methods[0].node, method_node);
        // The free function is NOT a receiver-indexed method.
        assert!(
            inputs
                .methods_by_receiver
                .values()
                .flatten()
                .all(|m| m.qualified != "pkg.Speak"),
            "the free function must not be indexed as a receiver method"
        );
    }
}
