use std::collections::{BTreeMap, BTreeSet};

use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use serde::Serialize;

use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::CallTargetStatus;
use crate::internal_core::{FunctionId, StableKeyId, StableKeyInterner};

// ---------------------------------------------------------------------------
// SCC types for summary scheduling from direct call targets
// ---------------------------------------------------------------------------

/// A strongly connected component in the summary call graph.
///
/// Members are sorted by their callable stable key for determinism (D-17).
/// When `is_recursive` is true, the SCC requires fixpoint iteration;
/// otherwise a single pass suffices (D-05).
/// In-memory SCC. `member_stable_keys` owns interned identities sorted by resolved
/// text at construction; do not serde this type (numeric ids). Use
/// [`SccSchedule::debug_info`] for resolved-text payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scc {
    /// Function IDs in this SCC, sorted by resolved stable-key text.
    pub members: Vec<FunctionId>,
    /// Stable keys corresponding to each member, sorted by resolved text.
    pub member_stable_keys: Vec<StableKeyId>,
    /// True if the SCC has mutual recursion or a self-call.
    pub is_recursive: bool,
    /// Number of members in this SCC.
    pub size: usize,
}

/// The complete SCC schedule for summary computation.
///
/// SCCs are stored in reverse topological order (D-04): leaf callees come
/// first so their summaries are available when callers are processed.
/// Independent SCCs at the same topological level are ordered by the
/// sorted stable keys of their members for determinism (D-17).
/// In-memory SCC schedule. Not a wire type — use [`SccSchedule::debug_info`] for
/// resolved-text serialization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SccSchedule {
    /// SCCs in reverse topological order (leaf callees first).
    pub sccs: Vec<Scc>,
    /// Total number of functions in the schedule.
    pub total_functions: usize,
    /// Total number of SCCs.
    pub total_sccs: usize,
    /// Number of recursive SCCs (multi-member or self-calling).
    pub recursive_scc_count: usize,
    /// Size of the largest SCC.
    pub max_scc_size: usize,
}

/// Debug/trace information for SCC scheduling (D-14).
#[derive(Clone, Debug, Serialize)]
pub struct SccScheduleDebug {
    /// Total number of SCCs discovered.
    pub total_sccs: usize,
    /// Total number of functions processed.
    pub total_functions: usize,
    /// Number of recursive SCCs.
    pub recursive_scc_count: usize,
    /// Size of the largest SCC.
    pub max_scc_size: usize,
    /// Sizes of each SCC in processing order.
    pub scc_sizes: Vec<usize>,
    /// Resolved stable-key text of members in each SCC, in processing order.
    pub processing_order: Vec<Vec<String>>,
}

impl SccSchedule {
    /// Extracts debug/trace summary statistics per D-14.
    pub fn debug_info(&self, interner: &StableKeyInterner) -> SccScheduleDebug {
        SccScheduleDebug {
            total_sccs: self.total_sccs,
            total_functions: self.total_functions,
            recursive_scc_count: self.recursive_scc_count,
            max_scc_size: self.max_scc_size,
            scc_sizes: self.sccs.iter().map(|scc| scc.size).collect(),
            processing_order: self
                .sccs
                .iter()
                .map(|scc| resolved_member_keys(interner, &scc.member_stable_keys))
                .collect(),
        }
    }
}

pub fn resolved_member_keys(interner: &StableKeyInterner, keys: &[StableKeyId]) -> Vec<String> {
    keys.iter()
        .map(|key| interner.resolve(*key).to_string())
        .collect()
}

/// Computes the SCC schedule from direct call target facts in the LocalAnalysisDb.
///
/// Steps (per D-04):
/// 1. Collect functions that have at least one SummaryFact.
/// 2. Build a DiGraph from CallStore outgoing edges between those functions.
/// 3. Run Tarjan's SCC on the graph.
/// 4. Sort members within each SCC by resolved stable-key text (D-17).
/// 5. Classify recursive vs non-recursive SCCs.
pub fn compute_scc_schedule(db: &impl AnalysisHost) -> SccSchedule {
    let interner = db.stable_key_interner();
    // Step 1: Collect functions with direct summaries.
    let summary_store = db.summary_store();

    // Collect unique FunctionIds that have summaries, along with their
    // callable_stable_key for deterministic ordering.
    let mut function_stable_keys: BTreeMap<FunctionId, StableKeyId> = BTreeMap::new();
    for fact in summary_store.all_summaries() {
        function_stable_keys
            .entry(fact.function)
            .or_insert(fact.callable_stable_key);
    }

    if function_stable_keys.is_empty() {
        return SccSchedule {
            sccs: Vec::new(),
            total_functions: 0,
            total_sccs: 0,
            recursive_scc_count: 0,
            max_scc_size: 0,
        };
    }

    let summary_function_set: BTreeSet<FunctionId> = function_stable_keys.keys().copied().collect();

    // Step 2: Build DiGraph<FunctionId, ()>.
    let mut graph = DiGraph::<FunctionId, ()>::new();
    let mut node_map: BTreeMap<FunctionId, NodeIndex> = BTreeMap::new();

    // Add nodes for all functions with summaries.
    for &func_id in &summary_function_set {
        let idx = graph.add_node(func_id);
        node_map.insert(func_id, idx);
    }

    // Track self-edges for recursive SCC detection.
    let mut has_self_edge: BTreeSet<FunctionId> = BTreeSet::new();

    // Add edges from CallStore outgoing targets.
    {
        let call_store = db.calls_store();
        for &caller_id in &summary_function_set {
            let targets = call_store.outgoing_by_function(caller_id);
            for target in targets {
                if target.status != CallTargetStatus::Resolved {
                    continue;
                }
                if let Some(target_func) = target.target_function
                    && summary_function_set.contains(&target_func)
                {
                    let caller_idx = node_map[&caller_id];
                    let callee_idx = node_map[&target_func];
                    graph.add_edge(caller_idx, callee_idx, ());

                    if caller_id == target_func {
                        has_self_edge.insert(caller_id);
                    }
                }
            }
        }
    }

    // Step 3: Compute SCCs via petgraph Tarjan's algorithm.
    // petgraph::algo::tarjan_scc returns SCCs in reverse topological order
    // (leaf/sink SCCs first), which is exactly the bottom-up order we need.
    let raw_sccs = tarjan_scc(&graph);

    // Step 4 & 5: Build Scc structs with sorted members and recursive classification.
    let mut sccs = Vec::with_capacity(raw_sccs.len());
    let mut total_functions = 0_usize;
    let mut recursive_scc_count = 0_usize;
    let mut max_scc_size = 0_usize;

    for raw_scc in &raw_sccs {
        // Collect (resolved text, StableKeyId, FunctionId) for sorting.
        let mut members_with_keys: Vec<(String, StableKeyId, FunctionId)> = raw_scc
            .iter()
            .map(|&node_idx| {
                let func_id = graph[node_idx];
                let stable_key = function_stable_keys
                    .get(&func_id)
                    .copied()
                    .unwrap_or_else(|| interner.intern(format!("<unknown:{}>", func_id.0)));
                (
                    interner.resolve(stable_key).to_string(),
                    stable_key,
                    func_id,
                )
            })
            .collect();

        // Sort by resolved stable-key text for determinism (D-17).
        members_with_keys.sort_by(|a, b| a.0.cmp(&b.0));

        let member_stable_keys: Vec<StableKeyId> =
            members_with_keys.iter().map(|(_, key, _)| *key).collect();
        let members: Vec<FunctionId> = members_with_keys.iter().map(|(_, _, id)| *id).collect();

        let size = members.len();

        // Determine if recursive: multi-member SCC or single member with self-edge.
        let is_recursive = size > 1 || (size == 1 && has_self_edge.contains(&members[0]));

        if is_recursive {
            recursive_scc_count += 1;
        }
        total_functions += size;
        if size > max_scc_size {
            max_scc_size = size;
        }

        sccs.push(Scc {
            members,
            member_stable_keys,
            is_recursive,
            size,
        });
    }

    let sccs = sorted_sccs_by_rank_and_stable_key(&interner, &graph, &sccs);

    SccSchedule {
        total_sccs: sccs.len(),
        sccs,
        total_functions,
        recursive_scc_count,
        max_scc_size,
    }
}

fn sorted_sccs_by_rank_and_stable_key(
    interner: &StableKeyInterner,
    graph: &DiGraph<FunctionId, ()>,
    sccs: &[Scc],
) -> Vec<Scc> {
    let mut scc_by_function = BTreeMap::new();
    for (index, scc) in sccs.iter().enumerate() {
        for function in &scc.members {
            scc_by_function.insert(*function, index);
        }
    }

    let mut ranks = vec![0_usize; sccs.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for edge in graph.edge_references() {
            let caller = graph[edge.source()];
            let callee = graph[edge.target()];
            let Some(&caller_scc) = scc_by_function.get(&caller) else {
                continue;
            };
            let Some(&callee_scc) = scc_by_function.get(&callee) else {
                continue;
            };
            if caller_scc == callee_scc {
                continue;
            }

            let required_rank = ranks[callee_scc] + 1;
            if ranks[caller_scc] < required_rank {
                ranks[caller_scc] = required_rank;
                changed = true;
            }
        }
    }

    let mut ordered = sccs
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, scc)| {
            (
                ranks[index],
                resolved_member_keys(interner, &scc.member_stable_keys),
                scc,
            )
        })
        .collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    ordered.into_iter().map(|(_, _, scc)| scc).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallAlgorithm, CallCallee, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact,
        CallSyntaxKind, CallTargetFact, CallTargetStatus,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::ids::SummaryId;
    use crate::analysis_neutral::ids::{CallSiteId, CallTargetId, MirBodyId, MirOpId};
    use crate::analysis_neutral::summaries::facts::{
        SummaryDomainKind, SummaryPrecision, SummaryProvenance, SummaryStatus,
    };
    use crate::analysis_neutral::summaries::store::SummaryOutput;
    use crate::internal_core::{FileId, Language, Span, stable_key_for_test};

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn span() -> Span {
        Span::point(FileId::from_raw(1), 1, 1)
    }

    /// Create a minimal SummaryFact for a function.
    fn summary_fact(
        function_id: u64,
        callable_key: &str,
    ) -> crate::analysis_neutral::summaries::facts::SummaryFact {
        crate::analysis_neutral::summaries::facts::SummaryFact {
            id: SummaryId(0),
            callable_stable_key: stable_key_for_test(callable_key),
            function: FunctionId::from_raw(function_id),
            domain: SummaryDomainKind::ControlEffects,
            status: SummaryStatus::Present,
            precision: SummaryPrecision::Local,
            provenance: SummaryProvenance::NativeLocal,
            payload_digest: format!("digest:{callable_key}"),
            tito_flows: Vec::new(),
            stable_key: stable_key_for_test(&format!("summary:control_effects:{callable_key}")),
        }
    }

    /// Create a call site for a caller.
    fn call_site(id: u64, caller: u64) -> CallSiteFact {
        CallSiteFact {
            in_throw: false,
            id: CallSiteId(id),
            language: Language::TypeScript,
            file: FileId::from_raw(1),
            caller: FunctionId::from_raw(caller),
            owner_symbol: None,
            body: MirBodyId(caller),
            operation: MirOpId(id),
            span: span(),
            kind: CallSyntaxKind::Function,
            callee: CallCallee::Identifier {
                reference: None,
                name: format!("call_{id}"),
            },
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Resolved,
            precision: CallPrecision::Exact,
            stable_key: crate::internal_core::StableKeyId(id as u32),
        }
    }

    /// Create a resolved call target from caller to callee.
    fn call_target(id: u64, site_id: u64, caller: u64, target_func: u64) -> CallTargetFact {
        CallTargetFact {
            id: CallTargetId(id),
            site: CallSiteId(site_id),
            caller: FunctionId::from_raw(caller),
            target_function: Some(FunctionId::from_raw(target_func)),
            target_symbol: None,
            synthetic_target: None,
            edge_kind: CallEdgeKind::Direct,
            algorithm: CallAlgorithm::DirectReference,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::Native,
            precision: CallPrecision::Exact,
            stable_key: crate::internal_core::StableKeyId(id as u32),
        }
    }

    /// Build an LocalAnalysisDb with the given summaries and call edges.
    fn build_db(
        summaries: Vec<crate::analysis_neutral::summaries::facts::SummaryFact>,
        sites: Vec<CallSiteFact>,
        targets: Vec<CallTargetFact>,
    ) -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();

        db.replace_summary_facts(SummaryOutput {
            summaries,
            events: Vec::new(),
        });

        db.replace_call_facts(CallOutput {
            sites,
            targets,
            unresolved: Vec::new(),
        })
        .expect("call output should be valid");

        db
    }

    fn member_texts(interner: &StableKeyInterner, scc: &Scc) -> Vec<String> {
        resolved_member_keys(interner, &scc.member_stable_keys)
    }

    // -----------------------------------------------------------------------
    // Test (a): Empty LocalAnalysisDb produces SccSchedule with 0 SCCs
    // -----------------------------------------------------------------------

    #[test]
    fn empty_db_produces_empty_schedule() {
        let db = LocalAnalysisDb::new();
        let schedule = compute_scc_schedule(&db);

        assert_eq!(schedule.total_sccs, 0);
        assert_eq!(schedule.total_functions, 0);
        assert_eq!(schedule.recursive_scc_count, 0);
        assert_eq!(schedule.max_scc_size, 0);
        assert!(schedule.sccs.is_empty());
    }

    // -----------------------------------------------------------------------
    // Test (b): Chain A->B->C produces SCCs [C], [B], [A] in that order
    // -----------------------------------------------------------------------

    #[test]
    fn chain_call_graph_produces_reverse_topological_sccs() {
        // Functions: A(1) -> B(2) -> C(3)
        let summaries = vec![
            summary_fact(1, "func::a"),
            summary_fact(2, "func::b"),
            summary_fact(3, "func::c"),
        ];

        let sites = vec![
            call_site(1, 1), // A calls something
            call_site(2, 2), // B calls something
        ];

        let targets = vec![
            call_target(1, 1, 1, 2), // A -> B
            call_target(2, 2, 2, 3), // B -> C
        ];

        let db = build_db(summaries, sites, targets);
        let interner = db.stable_key_interner();
        let schedule = compute_scc_schedule(&db);

        assert_eq!(schedule.total_sccs, 3);
        assert_eq!(schedule.total_functions, 3);
        assert_eq!(schedule.recursive_scc_count, 0);
        assert_eq!(schedule.max_scc_size, 1);

        // Verify reverse topological order: C first, then B, then A
        assert_eq!(member_texts(&interner, &schedule.sccs[0]), vec!["func::c"]);
        assert_eq!(member_texts(&interner, &schedule.sccs[1]), vec!["func::b"]);
        assert_eq!(member_texts(&interner, &schedule.sccs[2]), vec!["func::a"]);

        // None are recursive
        for scc in &schedule.sccs {
            assert!(!scc.is_recursive);
            assert_eq!(scc.size, 1);
        }
    }

    // -----------------------------------------------------------------------
    // Test (c): Self-calling function produces a recursive SCC of size 1
    // -----------------------------------------------------------------------

    #[test]
    fn self_call_produces_recursive_scc() {
        let summaries = vec![summary_fact(1, "func::recursive")];

        let sites = vec![call_site(1, 1)];
        let targets = vec![call_target(1, 1, 1, 1)]; // self-call

        let db = build_db(summaries, sites, targets);
        let interner = db.stable_key_interner();
        let schedule = compute_scc_schedule(&db);

        assert_eq!(schedule.total_sccs, 1);
        assert_eq!(schedule.total_functions, 1);
        assert_eq!(schedule.recursive_scc_count, 1);
        assert_eq!(schedule.max_scc_size, 1);

        let scc = &schedule.sccs[0];
        assert!(scc.is_recursive);
        assert_eq!(scc.size, 1);
        assert_eq!(member_texts(&interner, scc), vec!["func::recursive"]);
    }

    // -----------------------------------------------------------------------
    // Test (d): Independent functions are scheduled in sorted stable-key order
    // -----------------------------------------------------------------------

    #[test]
    fn independent_functions_sorted_by_stable_key() {
        // Three functions with no call edges between them.
        // Stable keys are intentionally out of alphabetical order in input.
        let summaries = vec![
            summary_fact(3, "func::zebra"),
            summary_fact(1, "func::alpha"),
            summary_fact(2, "func::middle"),
        ];

        let db = build_db(summaries, Vec::new(), Vec::new());
        let interner = db.stable_key_interner();
        let schedule = compute_scc_schedule(&db);

        assert_eq!(schedule.total_sccs, 3);
        assert_eq!(schedule.total_functions, 3);
        assert_eq!(schedule.recursive_scc_count, 0);

        let keys: Vec<String> = schedule
            .sccs
            .iter()
            .map(|scc| member_texts(&interner, scc)[0].clone())
            .collect();

        assert_eq!(
            keys,
            vec![
                "func::alpha".to_string(),
                "func::middle".to_string(),
                "func::zebra".to_string()
            ],
            "independent SCCs must be sorted by stable key"
        );
    }

    // -----------------------------------------------------------------------
    // Additional: debug_info returns correct summary statistics
    // -----------------------------------------------------------------------

    #[test]
    fn debug_info_extracts_correct_statistics() {
        // A->B (chain) + C (self-recursive) + D (independent)
        let summaries = vec![
            summary_fact(1, "func::a"),
            summary_fact(2, "func::b"),
            summary_fact(3, "func::c"),
            summary_fact(4, "func::d"),
        ];

        let sites = vec![
            call_site(1, 1), // A calls B
            call_site(2, 3), // C calls C
        ];

        let targets = vec![
            call_target(1, 1, 1, 2), // A -> B
            call_target(2, 2, 3, 3), // C -> C (self)
        ];

        let db = build_db(summaries, sites, targets);
        let interner = db.stable_key_interner();
        let schedule = compute_scc_schedule(&db);
        let debug = schedule.debug_info(&interner);

        assert_eq!(debug.total_sccs, schedule.total_sccs);
        assert_eq!(debug.total_functions, 4);
        assert_eq!(debug.recursive_scc_count, 1);
        assert_eq!(debug.max_scc_size, 1);
        assert_eq!(debug.scc_sizes.len(), schedule.total_sccs);
        assert_eq!(debug.processing_order.len(), schedule.total_sccs);

        // Each processing_order entry should have the stable keys
        for (i, keys) in debug.processing_order.iter().enumerate() {
            assert_eq!(keys, &member_texts(&interner, &schedule.sccs[i]));
        }
    }

    // -----------------------------------------------------------------------
    // Additional: Mutual recursion forms single SCC
    // -----------------------------------------------------------------------

    #[test]
    fn mutual_recursion_forms_single_scc() {
        // A -> B, B -> A
        let summaries = vec![summary_fact(1, "func::a"), summary_fact(2, "func::b")];

        let sites = vec![call_site(1, 1), call_site(2, 2)];

        let targets = vec![
            call_target(1, 1, 1, 2), // A -> B
            call_target(2, 2, 2, 1), // B -> A
        ];

        let db = build_db(summaries, sites, targets);
        let interner = db.stable_key_interner();
        let schedule = compute_scc_schedule(&db);

        assert_eq!(schedule.total_sccs, 1);
        assert_eq!(schedule.total_functions, 2);
        assert_eq!(schedule.recursive_scc_count, 1);
        assert_eq!(schedule.max_scc_size, 2);

        let scc = &schedule.sccs[0];
        assert!(scc.is_recursive);
        assert_eq!(scc.size, 2);
        // Members sorted by stable key
        assert_eq!(member_texts(&interner, scc), vec!["func::a", "func::b"]);
    }

    // -----------------------------------------------------------------------
    // Additional: Unresolved targets are excluded from edges
    // -----------------------------------------------------------------------

    #[test]
    fn unresolved_targets_are_excluded() {
        let summaries = vec![summary_fact(1, "func::a"), summary_fact(2, "func::b")];

        let sites = vec![call_site(1, 1)];

        let mut target = call_target(1, 1, 1, 2);
        target.status = CallTargetStatus::Unresolved; // Should be excluded

        let db = build_db(summaries, sites, vec![target]);
        let schedule = compute_scc_schedule(&db);

        // No edges => 2 independent SCCs, neither recursive
        assert_eq!(schedule.total_sccs, 2);
        assert_eq!(schedule.recursive_scc_count, 0);
    }

    // -----------------------------------------------------------------------
    // Additional: Targets to functions outside the summary set are excluded
    // -----------------------------------------------------------------------

    #[test]
    fn targets_outside_summary_set_excluded() {
        // Only func::a has a summary; func::b does not.
        let summaries = vec![summary_fact(1, "func::a")];

        let sites = vec![call_site(1, 1)];
        let targets = vec![call_target(1, 1, 1, 2)]; // A -> B, but B has no summary

        let db = build_db(summaries, sites, targets);
        let interner = db.stable_key_interner();
        let schedule = compute_scc_schedule(&db);

        assert_eq!(schedule.total_sccs, 1);
        assert_eq!(schedule.total_functions, 1);
        assert_eq!(schedule.recursive_scc_count, 0);

        let scc = &schedule.sccs[0];
        assert!(!scc.is_recursive);
        assert_eq!(member_texts(&interner, scc), vec!["func::a"]);
    }

    // -----------------------------------------------------------------------
    // Additional: SccSchedule serializes cleanly
    // -----------------------------------------------------------------------

    #[test]
    fn schedule_debug_info_serializes_resolved_text() {
        let summaries = vec![summary_fact(1, "func::a"), summary_fact(2, "func::b")];
        let db = build_db(summaries, Vec::new(), Vec::new());
        let interner = db.stable_key_interner();
        let schedule = compute_scc_schedule(&db);

        let debug_json =
            serde_json::to_string(&schedule.debug_info(&interner)).expect("debug should serialize");
        assert!(debug_json.contains("func::a"));
        assert!(debug_json.contains("func::b"));
        assert!(debug_json.contains("total_sccs"));
        assert!(debug_json.contains("processing_order"));
    }

    #[test]
    fn reverse_intern_order_preserves_member_text_order() {
        let forward = crate::internal_core::StableKeyInterner::default();
        let reverse = crate::internal_core::StableKeyInterner::default();
        let fa = forward.intern("func::a");
        let fb = forward.intern("func::b");
        let rb = reverse.intern("func::b");
        let ra = reverse.intern("func::a");
        assert_ne!(fa.0, ra.0);

        let mut forward_keys = vec![fb, fa];
        let mut reverse_keys = vec![ra, rb];
        forward_keys.sort_by_key(|key| forward.resolve(*key));
        reverse_keys.sort_by_key(|key| reverse.resolve(*key));
        assert_eq!(
            resolved_member_keys(&forward, &forward_keys),
            resolved_member_keys(&reverse, &reverse_keys)
        );
        assert_eq!(
            resolved_member_keys(&forward, &forward_keys),
            vec!["func::a".to_string(), "func::b".to_string()]
        );
    }
}
