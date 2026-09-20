use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::facts::{
    SummaryDomainKind, SummaryEventFact, SummaryFact, SummaryPrecision, SummaryProvenance,
    SummaryStatus,
};
use super::scc::{Scc, SccSchedule};
use crate::analysis_api::{Digest, DigestKind, PrecisionTier, QueryKey, stable_key_from_key_parts};
use crate::analysis_api::{FactFamily, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::CallTargetStatus;
use crate::analysis_neutral::demand::{DemandQueryEngine, DemandQueryResult};
use crate::analysis_neutral::ids::{SummaryEventId, SummaryId};
use crate::analysis_neutral::summaries::scc::resolved_member_keys;
use crate::internal_core::{FunctionId, KeyPart, StableKeyId};

// ---------------------------------------------------------------------------
// SccClosureConfig
// ---------------------------------------------------------------------------

/// Configuration for interprocedural SCC closure.
#[derive(Clone, Debug)]
pub struct SccClosureConfig {
    /// Maximum iterations for recursive SCC fixpoint (default 100).
    pub max_iterations: u32,
    /// Whether to compare output digests against previous run (default true).
    pub enable_backdating: bool,
}

impl Default for SccClosureConfig {
    fn default() -> Self {
        Self {
            max_iterations: 100,
            enable_backdating: true,
        }
    }
}

// ---------------------------------------------------------------------------
// SccClosureResult
// ---------------------------------------------------------------------------

/// Result of interprocedural SCC closure across all SCCs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SccClosureResult {
    pub total_sccs_processed: usize,
    pub non_recursive_sccs: usize,
    pub recursive_sccs: usize,
    pub budget_exceeded_sccs: usize,
    pub backdated_sccs: usize,
    pub total_iterations: usize,
    pub updated_summaries: usize,
    pub scc_iteration_counts: Vec<(Vec<String>, u32)>,
    pub scc_output_digests: BTreeMap<Vec<String>, String>,
}

// ---------------------------------------------------------------------------
// Internal: per-function summary state during closure
// ---------------------------------------------------------------------------

/// Tracks the interprocedural summary payload digest for a single function
/// during SCC closure. We work at the payload_digest level since the actual
/// domain value is already encoded into the digest string by the builder.
#[derive(Clone, Debug)]
struct FunctionSummaryState {
    /// Current payload digests by domain kind.
    digests: BTreeMap<SummaryDomainKind, String>,
    /// Callable stable key for this function.
    callable_stable_key: StableKeyId,
}

// ---------------------------------------------------------------------------
// close_summaries_by_scc
// ---------------------------------------------------------------------------

/// Runs interprocedural summary closure over the given SCC schedule.
///
/// For each SCC in reverse topological order (leaf callees first):
/// - Non-recursive SCCs: single pass applying callee summaries.
/// - Recursive SCCs: iterate with join (widening in finite summary domain)
///   until convergence or budget exhaustion.
/// - After each SCC, compare output digests against `previous_scc_digests`
///   for backdating.
/// - Record each SCC computation as a demand query entry in `demand_engine`.
pub fn close_summaries_by_scc(
    db: &mut impl AnalysisHost,
    schedule: &SccSchedule,
    config: &SccClosureConfig,
    demand_engine: &mut DemandQueryEngine,
    previous_scc_digests: &BTreeMap<Vec<String>, String>,
) -> SccClosureResult {
    let mut result = SccClosureResult {
        total_sccs_processed: 0,
        non_recursive_sccs: 0,
        recursive_sccs: 0,
        budget_exceeded_sccs: 0,
        backdated_sccs: 0,
        total_iterations: 0,
        updated_summaries: 0,
        scc_iteration_counts: Vec::new(),
        scc_output_digests: BTreeMap::new(),
    };
    let mut summary_metadata_dirty = false;

    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;

    for scc in &schedule.sccs {
        result.total_sccs_processed += 1;
        let member_key_texts = resolved_member_keys(interner, &scc.member_stable_keys);

        if scc.is_recursive {
            result.recursive_sccs += 1;
            let (scc_summaries, scc_events, iterations, budget_exceeded) =
                process_recursive_scc(db, scc, config);

            result.total_iterations += iterations as usize;
            result
                .scc_iteration_counts
                .push((member_key_texts.clone(), iterations));

            if budget_exceeded {
                result.budget_exceeded_sccs += 1;
            }

            result.updated_summaries += scc_summaries.len();

            if !scc_summaries.is_empty() || !scc_events.is_empty() {
                merge_updated_summaries(db, &scc_summaries, &scc_events);
                summary_metadata_dirty = true;
            }

            // Compute the post-merge SCC output digest for backdating.
            let scc_digest = compute_current_scc_digest(interner, db, scc);
            let was_backdated = check_backdating(
                config,
                &member_key_texts,
                &scc_digest,
                previous_scc_digests,
                &mut result,
            );
            result
                .scc_output_digests
                .insert(member_key_texts.clone(), scc_digest.clone());

            // Record demand query entry
            record_scc_demand_query(
                demand_engine,
                &member_key_texts,
                &scc_digest,
                iterations,
                was_backdated,
            );
        } else {
            result.non_recursive_sccs += 1;
            let (scc_summaries, scc_events) = process_non_recursive_scc(db, scc);

            result.updated_summaries += scc_summaries.len();

            if !scc_summaries.is_empty() || !scc_events.is_empty() {
                merge_updated_summaries(db, &scc_summaries, &scc_events);
                summary_metadata_dirty = true;
            }

            // Compute the post-merge SCC output digest for backdating.
            let scc_digest = compute_current_scc_digest(interner, db, scc);
            let was_backdated = check_backdating(
                config,
                &member_key_texts,
                &scc_digest,
                previous_scc_digests,
                &mut result,
            );
            result
                .scc_output_digests
                .insert(member_key_texts.clone(), scc_digest.clone());

            // Record demand query entry
            record_scc_demand_query(
                demand_engine,
                &member_key_texts,
                &scc_digest,
                1,
                was_backdated,
            );
        }
    }

    if summary_metadata_dirty {
        db.refresh_summary_metadata_after_bulk_update();
    }

    result
}

// ---------------------------------------------------------------------------
// Non-recursive SCC: single pass
// ---------------------------------------------------------------------------

fn process_non_recursive_scc(
    db: &impl AnalysisHost,
    scc: &Scc,
) -> (Vec<SummaryFact>, Vec<SummaryEventFact>) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    debug_assert_eq!(scc.members.len(), 1);
    let function = scc.members[0];
    let callable_key = scc.member_stable_keys[0];

    // Get the function's existing direct summaries
    let existing_summaries: Vec<SummaryFact> = db
        .summary_store()
        .summaries_by_function(function)
        .into_iter()
        .cloned()
        .collect();

    if existing_summaries.is_empty() {
        return (Vec::new(), Vec::new());
    }

    // For each callee of this function, look up their current summaries
    let callee_info = collect_callee_info(db, function);
    if callee_info.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut events = Vec::new();

    // Apply callee effects to improve caller summaries
    let updated = apply_callee_effects(
        interner,
        &existing_summaries,
        &callee_info,
        callable_key,
        function,
        &mut events,
    );

    (updated, events)
}

// ---------------------------------------------------------------------------
// Recursive SCC: fixpoint iteration
// ---------------------------------------------------------------------------

fn process_recursive_scc(
    db: &impl AnalysisHost,
    scc: &Scc,
    config: &SccClosureConfig,
) -> (Vec<SummaryFact>, Vec<SummaryEventFact>, u32, bool) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    // Initialize per-function summary state from direct summaries
    let mut states: BTreeMap<FunctionId, FunctionSummaryState> = BTreeMap::new();
    let summary_store = db.summary_store();

    for (i, &func_id) in scc.members.iter().enumerate() {
        let summaries = summary_store.summaries_by_function(func_id);
        let mut digests = BTreeMap::new();
        for fact in &summaries {
            digests.insert(fact.domain, fact.payload_digest.clone());
        }
        states.insert(
            func_id,
            FunctionSummaryState {
                digests,
                callable_stable_key: scc.member_stable_keys[i],
            },
        );
    }

    let member_set: std::collections::BTreeSet<FunctionId> = scc.members.iter().copied().collect();

    let mut iteration = 0_u32;
    let mut converged = false;
    let mut budget_exceeded = false;

    while iteration < config.max_iterations {
        iteration += 1;
        let mut any_changed = false;

        // Process members in deterministic order (sorted by stable_key per D-17)
        for &func_id in &scc.members {
            let callee_info = collect_callee_info(db, func_id);

            // Build the current callee digests including SCC-internal members
            let mut effective_callee_info = callee_info;
            // For SCC-internal callees, use the latest iterative state
            for entry in &mut effective_callee_info {
                if member_set.contains(&entry.callee_function)
                    && let Some(state) = states.get(&entry.callee_function)
                {
                    entry.callee_digests = state.digests.clone();
                }
            }

            // Compute new digests by joining callee effects
            let old_state = states
                .get(&func_id)
                .cloned()
                .unwrap_or_else(|| FunctionSummaryState {
                    digests: BTreeMap::new(),
                    callable_stable_key: StableKeyId(0),
                });

            let new_digests = join_callee_digests_into(&old_state.digests, &effective_callee_info);

            // Check convergence via digest equality (leq in digest space)
            if new_digests != old_state.digests {
                any_changed = true;
                if let Some(state) = states.get_mut(&func_id) {
                    state.digests = new_digests;
                }
            }
        }

        if !any_changed {
            converged = true;
            break;
        }
    }

    if !converged {
        budget_exceeded = true;
    }

    // Build the final summary facts from the iterated states
    let mut result_summaries = Vec::new();
    let mut result_events = Vec::new();

    for &func_id in &scc.members {
        let state = match states.get(&func_id) {
            Some(s) => s,
            None => continue,
        };

        let existing = summary_store.summaries_by_function(func_id);
        for fact in existing {
            let new_digest = state
                .digests
                .get(&fact.domain)
                .cloned()
                .unwrap_or_else(|| fact.payload_digest.clone());

            let (status, precision, provenance) = if budget_exceeded {
                (
                    SummaryStatus::BudgetExceeded,
                    SummaryPrecision::UnknownTop,
                    SummaryProvenance::InterproceduralClosure,
                )
            } else {
                (
                    fact.status,
                    SummaryPrecision::SetupAware,
                    SummaryProvenance::InterproceduralClosure,
                )
            };

            let tito_flows = if !budget_exceeded && fact.domain == SummaryDomainKind::DataFlowTito {
                fact.tito_flows.clone()
            } else {
                Vec::new()
            };

            result_summaries.push(SummaryFact {
                id: SummaryId(0),
                callable_stable_key: state.callable_stable_key,
                function: func_id,
                domain: fact.domain,
                status,
                precision,
                provenance,
                payload_digest: new_digest,
                tito_flows,
                stable_key: fact.stable_key,
            });
        }

        if budget_exceeded {
            result_events.push(SummaryEventFact {
                id: SummaryEventId(0),
                callable_stable_key: state.callable_stable_key,
                function: func_id,
                domain: SummaryDomainKind::ControlEffects,
                event_kind: "budget_exceeded".to_string(),
                reason: format!(
                    "SCC fixpoint did not converge within {} iterations",
                    config.max_iterations
                ),
                status: SummaryStatus::BudgetExceeded,
                precision: SummaryPrecision::UnknownTop,
                stable_key: stable_key_from_key_parts(
                    interner,
                    FactFamily::SummaryEvent,
                    [
                        ("callable", KeyPart::Key(state.callable_stable_key)),
                        ("domain", KeyPart::Text("scc_closure")),
                        ("event", KeyPart::Text("budget_exceeded")),
                    ],
                ),
            });
        }
    }

    (result_summaries, result_events, iteration, budget_exceeded)
}

// ---------------------------------------------------------------------------
// Callee information collection
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct CalleeInfo {
    callee_function: FunctionId,
    callee_digests: BTreeMap<SummaryDomainKind, String>,
    resolved: bool,
}

fn collect_callee_info(db: &impl AnalysisHost, caller: FunctionId) -> Vec<CalleeInfo> {
    let mut result = Vec::new();

    let call_store = db.calls_store();

    let summary_store = db.summary_store();

    let targets = call_store.outgoing_by_function(caller);
    let mut seen = std::collections::BTreeSet::new();

    for target in targets {
        if target.status != CallTargetStatus::Resolved {
            continue;
        }

        if let Some(callee_func) = target.target_function {
            if !seen.insert(callee_func) {
                continue; // already processed this callee
            }

            let callee_summaries = summary_store.summaries_by_function(callee_func);
            let mut digests = BTreeMap::new();
            for fact in &callee_summaries {
                digests.insert(fact.domain, fact.payload_digest.clone());
            }

            result.push(CalleeInfo {
                callee_function: callee_func,
                callee_digests: digests,
                resolved: true,
            });
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Callee effect application
// ---------------------------------------------------------------------------

/// Apply callee summary effects to improve caller summaries.
///
/// For the initial implementation (per plan):
/// (a) Join callee ControlEffects into the caller's CallEffects
///     (callee may throw/panic propagates to caller)
/// (b) Join callee MemoryEffects for pass-through arguments
///     (if caller passes its param[i] to callee param[j] and callee writes
///      param[j], then caller has transitive memory effect on param[i])
/// (c) Mark unresolved-callee entries with unknown top reasons
fn apply_callee_effects(
    interner: &crate::internal_core::StableKeyInterner,
    existing_summaries: &[SummaryFact],
    callee_info: &[CalleeInfo],
    callable_key: StableKeyId,
    function: FunctionId,
    events: &mut Vec<SummaryEventFact>,
) -> Vec<SummaryFact> {
    let mut result = Vec::new();

    // Build a combined callee effect summary:
    // If any callee has control effects indicating throws/panics, propagate that
    // to the caller's call effects.
    let has_unresolved_callee = callee_info.iter().any(|c| !c.resolved);
    let has_callee_with_no_summary = callee_info.iter().any(|c| c.callee_digests.is_empty());

    // Collect callee control effect digests that indicate throwing/panicking
    let mut callee_control_digests: Vec<String> = Vec::new();
    let mut callee_memory_digests: Vec<String> = Vec::new();

    for info in callee_info {
        if let Some(control_digest) = info.callee_digests.get(&SummaryDomainKind::ControlEffects) {
            callee_control_digests.push(control_digest.clone());
        }
        if let Some(memory_digest) = info.callee_digests.get(&SummaryDomainKind::MemoryEffects) {
            callee_memory_digests.push(memory_digest.clone());
        }
    }

    for fact in existing_summaries {
        let mut updated_fact = fact.clone();

        match fact.domain {
            SummaryDomainKind::CallEffects => {
                // (a) Join callee control effects into caller's call effects.
                // If any callee throws, that propagates to the caller's call-effect.
                if !callee_control_digests.is_empty() {
                    let mut parts: Vec<String> = vec![fact.payload_digest.clone()];
                    parts.extend(
                        callee_control_digests
                            .iter()
                            .map(|d| format!("callee_control:{d}")),
                    );
                    parts.sort();
                    updated_fact.payload_digest = parts.join(";");
                }

                // (c) Mark unresolved callee
                if has_unresolved_callee || has_callee_with_no_summary {
                    updated_fact.payload_digest =
                        format!("{};unresolved_callee:true", updated_fact.payload_digest);
                    if fact.status != SummaryStatus::Unknown {
                        // Keep original status but note the unknown callee
                    }
                }

                updated_fact.precision = SummaryPrecision::SetupAware;
            }
            SummaryDomainKind::MemoryEffects => {
                // (b) Join callee memory effects for transitive param effects
                if !callee_memory_digests.is_empty() {
                    let mut parts: Vec<String> = vec![fact.payload_digest.clone()];
                    parts.extend(
                        callee_memory_digests
                            .iter()
                            .map(|d| format!("callee_memory:{d}")),
                    );
                    parts.sort();
                    updated_fact.payload_digest = parts.join(";");
                }

                if has_unresolved_callee || has_callee_with_no_summary {
                    updated_fact.payload_digest = format!(
                        "{};unresolved_callee_memory:true",
                        updated_fact.payload_digest
                    );
                }

                updated_fact.precision = SummaryPrecision::SetupAware;
            }
            SummaryDomainKind::ControlEffects => {
                // Control effects: if any callee throws, caller may also throw
                if !callee_control_digests.is_empty() {
                    let mut parts: Vec<String> = vec![fact.payload_digest.clone()];
                    parts.extend(
                        callee_control_digests
                            .iter()
                            .map(|d| format!("callee_propagated:{d}")),
                    );
                    parts.sort();
                    updated_fact.payload_digest = parts.join(";");
                }

                updated_fact.precision = SummaryPrecision::SetupAware;
            }
            SummaryDomainKind::DataFlowTito => {
                // TITO: unchanged for now, callee TITO composition requires
                // argument-to-parameter mapping from the summary builder
                updated_fact.precision = SummaryPrecision::SetupAware;
            }
        }

        result.push(updated_fact);
    }

    if !callee_info.is_empty() {
        for fact in &mut result {
            fact.provenance = SummaryProvenance::InterproceduralClosure;
        }
    }

    // Add events for unresolved callees
    if has_unresolved_callee || has_callee_with_no_summary {
        events.push(SummaryEventFact {
            id: SummaryEventId(0),
            callable_stable_key: callable_key,
            function,
            domain: SummaryDomainKind::CallEffects,
            event_kind: "unresolved_callee_in_closure".to_string(),
            reason: "callee has no summary or is unresolved".to_string(),
            status: SummaryStatus::Unknown,
            precision: SummaryPrecision::UnknownTop,
            stable_key: stable_key_from_key_parts(
                interner,
                FactFamily::SummaryEvent,
                [
                    ("callable", KeyPart::Key(callable_key)),
                    ("domain", KeyPart::Text("scc_closure")),
                    ("event", KeyPart::Text("unresolved_callee_in_closure")),
                ],
            ),
        });
    }

    result
}

// ---------------------------------------------------------------------------
// Digest-level join for fixpoint iteration
// ---------------------------------------------------------------------------

fn join_callee_digests_into(
    current_digests: &BTreeMap<SummaryDomainKind, String>,
    callee_info: &[CalleeInfo],
) -> BTreeMap<SummaryDomainKind, String> {
    let mut new_digests = current_digests.clone();

    for info in callee_info {
        for (domain, callee_digest) in &info.callee_digests {
            if *domain == SummaryDomainKind::DataFlowTito {
                continue;
            }
            let entry = new_digests.entry(*domain).or_default();
            let mut parts = entry
                .split(';')
                .filter(|part| !part.is_empty())
                .map(str::to_string)
                .collect::<BTreeSet<_>>();
            let before = parts.len();
            parts.extend(
                callee_digest
                    .split(';')
                    .filter(|part| !part.is_empty())
                    .map(str::to_string),
            );
            if parts.len() != before {
                *entry = parts.into_iter().collect::<Vec<_>>().join(";");
            }
        }
    }

    new_digests
}

// ---------------------------------------------------------------------------
// SCC output digest computation
// ---------------------------------------------------------------------------

fn compute_scc_digest(
    interner: &crate::internal_core::StableKeyInterner,
    member_keys: &[String],
    summaries: &[SummaryFact],
) -> String {
    let mut digest_parts: Vec<String> = summaries
        .iter()
        .map(|s| {
            format!(
                "{}:{}:{}:{:?}",
                interner.resolve(s.callable_stable_key),
                s.domain.as_str(),
                s.payload_digest,
                s.tito_flows
            )
        })
        .collect();
    digest_parts.sort();
    let combined = digest_parts.join("|");
    // Use a stable hash-like representation
    format!("scc_digest:{}:{}", member_keys.join(","), combined)
}

fn compute_current_scc_digest(
    interner: &crate::internal_core::StableKeyInterner,
    db: &impl AnalysisHost,
    scc: &Scc,
) -> String {
    let store = db.summary_store();
    let summaries = scc
        .members
        .iter()
        .flat_map(|function| store.summaries_by_function(*function).into_iter().cloned())
        .collect::<Vec<_>>();

    let member_keys = resolved_member_keys(interner, &scc.member_stable_keys);
    compute_scc_digest(interner, &member_keys, &summaries)
}

// ---------------------------------------------------------------------------
// Backdating check
// ---------------------------------------------------------------------------

fn check_backdating(
    config: &SccClosureConfig,
    member_keys: &[String],
    current_digest: &str,
    previous_digests: &BTreeMap<Vec<String>, String>,
    result: &mut SccClosureResult,
) -> bool {
    if !config.enable_backdating {
        return false;
    }

    if let Some(previous) = previous_digests.get(member_keys)
        && previous == current_digest
    {
        result.backdated_sccs += 1;
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Demand query recording
// ---------------------------------------------------------------------------

fn record_scc_demand_query(
    demand_engine: &mut DemandQueryEngine,
    member_keys: &[String],
    scc_digest: &str,
    iterations: u32,
    was_backdated: bool,
) {
    let param_parts: Vec<String> = member_keys.iter().map(|k| format!("member:{k}")).collect();
    let param_refs: Vec<&str> = param_parts.iter().map(|s| s.as_str()).collect();

    let parameter_digest =
        Digest::from_parts(DigestKind::QueryParameters, "scc_closure", &param_refs);

    let query_key = QueryKey {
        query_kind: "scc_closure".to_string(),
        query_version: "1".to_string(),
        parameter_digest,
        layer_digests: Vec::new(),
        budget_digest: Digest::from_parts(
            DigestKind::Budget,
            "scc_closure",
            &[&format!("iterations:{iterations}")],
        ),
        precision_tier: PrecisionTier::SetupAware,
    };

    let output_digest = Digest::from_parts(
        DigestKind::ProviderOutput,
        "scc_closure_result",
        &[scc_digest],
    );

    let query_result = DemandQueryResult {
        query_key,
        output_digest,
        precision_tier: PrecisionTier::SetupAware,
        provenance: "native_scc_closure".to_string(),
        was_cached: was_backdated,
    };

    if was_backdated {
        let key = query_result.query_key.clone();
        demand_engine.record_cache_hit(&key, &query_result, 0);
    } else {
        demand_engine.insert(query_result);
    }
}

// ---------------------------------------------------------------------------
// Merge updated summaries into LocalAnalysisDb
// ---------------------------------------------------------------------------

fn merge_updated_summaries(
    db: &mut impl AnalysisHost,
    updated: &[SummaryFact],
    events: &[SummaryEventFact],
) {
    db.merge_summary_facts_without_metadata(updated, events);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallAlgorithm, CallCallee, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact,
        CallSyntaxKind, CallTargetFact, CallTargetStatus,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::ids::{CallSiteId, CallTargetId, MirBodyId, MirOpId};
    use crate::analysis_neutral::summaries::scc::compute_scc_schedule;
    use crate::analysis_neutral::summaries::store::SummaryOutput;
    use crate::internal_core::{FileId, Language, Span, stable_key_for_test};

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn span() -> Span {
        Span::point(FileId::from_raw(1), 1, 1)
    }

    fn summary_fact(
        function_id: u64,
        callable_key: &str,
        domain: SummaryDomainKind,
    ) -> SummaryFact {
        SummaryFact {
            id: SummaryId(0),
            callable_stable_key: stable_key_for_test(callable_key),
            function: FunctionId::from_raw(function_id),
            domain,
            status: SummaryStatus::Present,
            precision: SummaryPrecision::Local,
            provenance: SummaryProvenance::NativeLocal,
            payload_digest: format!("digest:{callable_key}:{}", domain.as_str()),
            tito_flows: Vec::new(),
            stable_key: stable_key_for_test(&format!("summary:{}:{callable_key}", domain.as_str())),
        }
    }

    fn control_summary_with_throw(function_id: u64, callable_key: &str) -> SummaryFact {
        SummaryFact {
            id: SummaryId(0),
            callable_stable_key: stable_key_for_test(callable_key),
            function: FunctionId::from_raw(function_id),
            domain: SummaryDomainKind::ControlEffects,
            status: SummaryStatus::Present,
            precision: SummaryPrecision::Local,
            provenance: SummaryProvenance::NativeLocal,
            payload_digest: "exit:Throws;async:Sync;cleanup:false".to_string(),
            tito_flows: Vec::new(),
            stable_key: stable_key_for_test(&format!("summary:control_effects:{callable_key}")),
        }
    }

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

    fn call_target(id: u64, site_id: u64, caller: u64, target_func: u64) -> CallTargetFact {
        CallTargetFact {
            id: CallTargetId(id),
            site: CallSiteId(site_id),
            caller: FunctionId::from_raw(caller),
            target_function: Some(FunctionId::from_raw(target_func)),
            target_symbol: None,
            edge_kind: CallEdgeKind::Direct,
            algorithm: CallAlgorithm::DirectReference,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::Native,
            precision: CallPrecision::Exact,
            stable_key: crate::internal_core::StableKeyId(id as u32),
        }
    }

    fn build_db(
        summaries: Vec<SummaryFact>,
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

    // -----------------------------------------------------------------------
    // Test (a): Non-recursive SCC with a callee that throws
    // -----------------------------------------------------------------------

    #[test]
    fn closure_non_recursive_scc_callee_throw_propagates() {
        // A calls B. B throws. After closure, A's call-effects should
        // record the throw propagation from B.
        let summaries = vec![
            summary_fact(1, "func::a", SummaryDomainKind::ControlEffects),
            summary_fact(1, "func::a", SummaryDomainKind::CallEffects),
            summary_fact(1, "func::a", SummaryDomainKind::MemoryEffects),
            summary_fact(1, "func::a", SummaryDomainKind::DataFlowTito),
            control_summary_with_throw(2, "func::b"),
            summary_fact(2, "func::b", SummaryDomainKind::CallEffects),
            summary_fact(2, "func::b", SummaryDomainKind::MemoryEffects),
            summary_fact(2, "func::b", SummaryDomainKind::DataFlowTito),
        ];

        let sites = vec![call_site(1, 1)]; // A has a call site
        let targets = vec![call_target(1, 1, 1, 2)]; // A -> B

        let mut db = build_db(summaries, sites, targets);
        let schedule = compute_scc_schedule(&db);

        let config = SccClosureConfig::default();
        let mut demand_engine = DemandQueryEngine::default();
        let previous_digests = BTreeMap::new();

        let result = close_summaries_by_scc(
            &mut db,
            &schedule,
            &config,
            &mut demand_engine,
            &previous_digests,
        );

        // Both B and A should be processed as non-recursive SCCs
        assert_eq!(result.total_sccs_processed, 2);
        assert_eq!(result.non_recursive_sccs, 2);
        assert_eq!(result.recursive_sccs, 0);
        assert!(result.updated_summaries > 0);

        // A's call effects should have callee control digest joined in
        let store = db.summary_store().expect("summary store should exist");
        let a_summaries = store.summaries_by_function(FunctionId::from_raw(1));
        let a_call_effects = a_summaries
            .iter()
            .find(|s| s.domain == SummaryDomainKind::CallEffects)
            .expect("A should have call effects");

        // The call effects digest should contain the callee's control digest
        assert!(
            a_call_effects.payload_digest.contains("callee_control:"),
            "A's call effects should record callee control propagation, got: {}",
            a_call_effects.payload_digest,
        );

        // Demand query trace should have entries
        assert!(!demand_engine.trace().is_empty());
    }

    #[test]
    fn closure_non_recursive_chain_uses_already_closed_callee_summary() {
        let summaries = vec![
            summary_fact(1, "func::a", SummaryDomainKind::CallEffects),
            summary_fact(1, "func::a", SummaryDomainKind::ControlEffects),
            summary_fact(2, "func::b", SummaryDomainKind::CallEffects),
            summary_fact(2, "func::b", SummaryDomainKind::ControlEffects),
            control_summary_with_throw(3, "func::c"),
            summary_fact(3, "func::c", SummaryDomainKind::CallEffects),
        ];
        let sites = vec![call_site(1, 1), call_site(2, 2)];
        let targets = vec![call_target(1, 1, 1, 2), call_target(2, 2, 2, 3)];
        let mut db = build_db(summaries, sites, targets);
        let schedule = compute_scc_schedule(&db);
        let mut demand_engine = DemandQueryEngine::default();

        let result = close_summaries_by_scc(
            &mut db,
            &schedule,
            &SccClosureConfig::default(),
            &mut demand_engine,
            &BTreeMap::new(),
        );

        assert_eq!(result.non_recursive_sccs, 3);
        let store = db.summary_store().expect("summary store should exist");
        let a_call_effects = store
            .summaries_by_function(FunctionId::from_raw(1))
            .into_iter()
            .find(|summary| summary.domain == SummaryDomainKind::CallEffects)
            .expect("A should have call effects");
        assert!(
            a_call_effects.payload_digest.contains("callee_propagated:"),
            "A should observe B's closed control summary, got {}",
            a_call_effects.payload_digest
        );
    }

    #[test]
    fn closure_leaf_scc_without_callees_preserves_direct_summary() {
        let summaries = vec![
            summary_fact(1, "func::leaf", SummaryDomainKind::ControlEffects),
            summary_fact(1, "func::leaf", SummaryDomainKind::CallEffects),
        ];
        let mut db = build_db(summaries, Vec::new(), Vec::new());
        let schedule = compute_scc_schedule(&db);
        let mut demand_engine = DemandQueryEngine::default();

        let result = close_summaries_by_scc(
            &mut db,
            &schedule,
            &SccClosureConfig::default(),
            &mut demand_engine,
            &BTreeMap::new(),
        );

        assert_eq!(
            result.updated_summaries, 0,
            "leaf SCCs with no callees should not be rewritten"
        );
        let store = db.summary_store().expect("summary store should exist");
        let leaf_summaries = store.summaries_by_function(FunctionId::from_raw(1));
        assert!(
            leaf_summaries
                .iter()
                .all(|summary| summary.precision == SummaryPrecision::Local
                    && summary.provenance == SummaryProvenance::NativeLocal),
            "leaf summaries should stay local: {leaf_summaries:#?}"
        );
    }

    #[test]
    fn closure_leaf_scc_digest_tracks_preserved_direct_summary() {
        let first_summaries = vec![
            summary_fact(1, "func::leaf", SummaryDomainKind::ControlEffects),
            summary_fact(1, "func::leaf", SummaryDomainKind::CallEffects),
        ];
        let mut db = build_db(first_summaries, Vec::new(), Vec::new());
        let schedule = compute_scc_schedule(&db);
        let mut demand_engine = DemandQueryEngine::default();

        let first = close_summaries_by_scc(
            &mut db,
            &schedule,
            &SccClosureConfig::default(),
            &mut demand_engine,
            &BTreeMap::new(),
        );

        let mut changed_call_summary =
            summary_fact(1, "func::leaf", SummaryDomainKind::CallEffects);
        changed_call_summary.payload_digest = "digest:func::leaf:call_effects:changed".to_string();
        let changed_summaries = vec![
            summary_fact(1, "func::leaf", SummaryDomainKind::ControlEffects),
            changed_call_summary,
        ];
        let mut changed_db = build_db(changed_summaries, Vec::new(), Vec::new());
        let changed_schedule = compute_scc_schedule(&changed_db);
        let mut changed_demand_engine = DemandQueryEngine::default();

        let changed = close_summaries_by_scc(
            &mut changed_db,
            &changed_schedule,
            &SccClosureConfig::default(),
            &mut changed_demand_engine,
            &first.scc_output_digests,
        );

        assert_eq!(
            changed.backdated_sccs, 0,
            "preserved leaf summaries must not backdate when their direct output changed"
        );
        assert_ne!(first.scc_output_digests, changed.scc_output_digests);
    }

    // -----------------------------------------------------------------------
    // Test (b): Recursive SCC with two mutually-calling functions
    // -----------------------------------------------------------------------

    #[test]
    fn closure_recursive_scc_converges_or_budget_exceeded() {
        // A <-> B (mutual recursion). Should converge or produce BudgetExceeded.
        let summaries = vec![
            summary_fact(1, "func::a", SummaryDomainKind::ControlEffects),
            summary_fact(1, "func::a", SummaryDomainKind::CallEffects),
            summary_fact(2, "func::b", SummaryDomainKind::ControlEffects),
            summary_fact(2, "func::b", SummaryDomainKind::CallEffects),
        ];

        let sites = vec![call_site(1, 1), call_site(2, 2)];
        let targets = vec![
            call_target(1, 1, 1, 2), // A -> B
            call_target(2, 2, 2, 1), // B -> A
        ];

        let mut db = build_db(summaries, sites, targets);
        let schedule = compute_scc_schedule(&db);

        // Use a small budget to test convergence
        let config = SccClosureConfig {
            max_iterations: 10,
            enable_backdating: true,
        };
        let mut demand_engine = DemandQueryEngine::default();
        let previous_digests = BTreeMap::new();

        let result = close_summaries_by_scc(
            &mut db,
            &schedule,
            &config,
            &mut demand_engine,
            &previous_digests,
        );

        assert_eq!(result.total_sccs_processed, 1);
        assert_eq!(result.recursive_sccs, 1);
        assert!(!result.scc_iteration_counts.is_empty());

        // The SCC should have converged (digests stabilize after joining)
        // OR exceeded budget. Either is valid behavior.
        let (ref members, iterations) = result.scc_iteration_counts[0];
        assert!(!members.is_empty());
        assert!(iterations >= 1);

        // If converged, budget_exceeded_sccs == 0
        // If not converged, budget_exceeded_sccs == 1
        assert!(
            result.budget_exceeded_sccs == 0 || result.budget_exceeded_sccs == 1,
            "should be 0 (converged) or 1 (exceeded): {}",
            result.budget_exceeded_sccs
        );

        // Verify that if budget exceeded, summaries have BudgetExceeded status
        if result.budget_exceeded_sccs > 0 {
            let store = db.summary_store().expect("store should exist");
            let a_summaries = store.summaries_by_function(FunctionId::from_raw(1));
            assert!(
                a_summaries
                    .iter()
                    .any(|s| s.status == SummaryStatus::BudgetExceeded),
                "BudgetExceeded SCCs should produce BudgetExceeded summaries"
            );
        }
    }

    #[test]
    fn closure_recursive_scc_reaches_fixpoint_without_digest_growth() {
        let summaries = vec![
            summary_fact(1, "func::a", SummaryDomainKind::ControlEffects),
            summary_fact(1, "func::a", SummaryDomainKind::CallEffects),
            summary_fact(2, "func::b", SummaryDomainKind::ControlEffects),
            summary_fact(2, "func::b", SummaryDomainKind::CallEffects),
        ];
        let sites = vec![call_site(1, 1), call_site(2, 2)];
        let targets = vec![call_target(1, 1, 1, 2), call_target(2, 2, 2, 1)];
        let mut db = build_db(summaries, sites, targets);
        let schedule = compute_scc_schedule(&db);
        let config = SccClosureConfig {
            max_iterations: 6,
            enable_backdating: true,
        };
        let mut demand_engine = DemandQueryEngine::default();

        let result = close_summaries_by_scc(
            &mut db,
            &schedule,
            &config,
            &mut demand_engine,
            &BTreeMap::new(),
        );

        assert_eq!(result.recursive_sccs, 1);
        assert_eq!(
            result.budget_exceeded_sccs, 0,
            "recursive SCC should converge before budget: {result:#?}"
        );
        assert!(
            result.scc_iteration_counts[0].1 < config.max_iterations,
            "recursive SCC should not consume the whole budget: {result:#?}"
        );
        let max_digest_len = db
            .summary_facts()
            .iter()
            .map(|fact| fact.payload_digest.len())
            .max()
            .unwrap_or_default();
        assert!(
            max_digest_len < 256,
            "fixpoint digests should stay bounded, got max len {max_digest_len}"
        );
    }

    // -----------------------------------------------------------------------
    // Test (c): Backdating — same inputs twice, second run is backdated
    // -----------------------------------------------------------------------

    #[test]
    fn closure_backdating_detects_unchanged_digests() {
        // Single function, no calls — SCC closure produces same digest both times.
        let summaries = vec![summary_fact(
            1,
            "func::a",
            SummaryDomainKind::ControlEffects,
        )];

        let mut db = build_db(summaries.clone(), Vec::new(), Vec::new());
        let schedule = compute_scc_schedule(&db);

        let config = SccClosureConfig::default();
        let mut demand_engine1 = DemandQueryEngine::default();

        // First run: no previous digests
        let result1 = close_summaries_by_scc(
            &mut db,
            &schedule,
            &config,
            &mut demand_engine1,
            &BTreeMap::new(),
        );

        assert_eq!(
            result1.backdated_sccs, 0,
            "first run has no previous digests"
        );

        // Second run: with previous digests, same inputs
        let mut db2 = build_db(summaries, Vec::new(), Vec::new());
        let schedule2 = compute_scc_schedule(&db2);
        let mut demand_engine2 = DemandQueryEngine::default();

        let result2 = close_summaries_by_scc(
            &mut db2,
            &schedule2,
            &config,
            &mut demand_engine2,
            &result1.scc_output_digests,
        );

        assert_eq!(
            result2.backdated_sccs, 1,
            "second run with same inputs should backdate"
        );
    }

    // -----------------------------------------------------------------------
    // Test: SccClosureConfig defaults
    // -----------------------------------------------------------------------

    #[test]
    fn closure_config_defaults() {
        let config = SccClosureConfig::default();
        assert_eq!(config.max_iterations, 100);
        assert!(config.enable_backdating);
    }

    #[test]
    fn recursive_digest_join_does_not_propagate_unmapped_tito_summaries() {
        let mut current = BTreeMap::new();
        current.insert(SummaryDomainKind::DataFlowTito, "caller_tito".to_string());
        current.insert(SummaryDomainKind::CallEffects, "caller_call".to_string());

        let mut callee_digests = BTreeMap::new();
        callee_digests.insert(SummaryDomainKind::DataFlowTito, "callee_tito".to_string());
        callee_digests.insert(SummaryDomainKind::CallEffects, "callee_call".to_string());

        let joined = join_callee_digests_into(
            &current,
            &[CalleeInfo {
                callee_function: FunctionId::from_raw(2),
                callee_digests,
                resolved: true,
            }],
        );

        assert_eq!(
            joined.get(&SummaryDomainKind::DataFlowTito),
            Some(&"caller_tito".to_string())
        );
        assert!(
            joined
                .get(&SummaryDomainKind::CallEffects)
                .is_some_and(|digest| digest.contains("callee_call"))
        );
    }

    // -----------------------------------------------------------------------
    // Test: empty schedule produces zero-result
    // -----------------------------------------------------------------------

    #[test]
    fn closure_empty_schedule_produces_zero_result() {
        let mut db = LocalAnalysisDb::new();
        let schedule = SccSchedule {
            sccs: Vec::new(),
            total_functions: 0,
            total_sccs: 0,
            recursive_scc_count: 0,
            max_scc_size: 0,
        };
        let config = SccClosureConfig::default();
        let mut demand_engine = DemandQueryEngine::default();

        let result = close_summaries_by_scc(
            &mut db,
            &schedule,
            &config,
            &mut demand_engine,
            &BTreeMap::new(),
        );

        assert_eq!(result.total_sccs_processed, 0);
        assert_eq!(result.non_recursive_sccs, 0);
        assert_eq!(result.recursive_sccs, 0);
        assert_eq!(result.updated_summaries, 0);
    }

    // -----------------------------------------------------------------------
    // Test: SccClosureResult serializes
    // -----------------------------------------------------------------------

    #[test]
    fn closure_result_serializes() {
        let result = SccClosureResult {
            total_sccs_processed: 3,
            non_recursive_sccs: 2,
            recursive_sccs: 1,
            budget_exceeded_sccs: 0,
            backdated_sccs: 1,
            total_iterations: 5,
            updated_summaries: 10,
            scc_iteration_counts: vec![(vec!["func::a".to_string(), "func::b".to_string()], 5)],
            scc_output_digests: BTreeMap::new(),
        };

        let json = serde_json::to_string(&result).expect("should serialize");
        assert!(json.contains("total_sccs_processed"));
        assert!(json.contains("backdated_sccs"));
        assert!(json.contains("scc_iteration_counts"));
    }
}
