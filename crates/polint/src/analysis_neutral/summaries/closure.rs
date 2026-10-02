use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::facts::{
    SummaryDomainKind, SummaryEventFact, SummaryFact, SummaryPrecision, SummaryProvenance,
    SummaryStatus,
};
use super::scc::{Scc, SccSchedule};
use crate::analysis_api::{Digest, DigestKind, PrecisionTier, QueryKey};
use crate::analysis_api::{FactFamily, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::CallTargetStatus;
use crate::analysis_neutral::demand::{DemandQueryEngine, DemandQueryResult};
use crate::analysis_neutral::ids::{SummaryEventId, SummaryId};
use crate::analysis_neutral::summaries::scc::resolved_member_keys;
use crate::internal_core::{FunctionId, StableKeyId};

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
#[cfg(test)]
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
    // Events are output only: no SCC reads another's events and no SCC digest
    // covers them. Merging them once, after the last SCC, sorts the store's
    // events once instead of once per SCC that emits any.
    let mut pending_events = Vec::new();

    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut profile = ClosureProfile::default();

    for scc in &schedule.sccs {
        result.total_sccs_processed += 1;
        let member_key_texts = resolved_member_keys(interner, &scc.member_stable_keys);

        if scc.is_recursive {
            result.recursive_sccs += 1;
            let started = std::time::Instant::now();
            let (scc_summaries, scc_events, iterations, budget_exceeded) =
                process_recursive_scc(db, scc, config);
            profile.record_recursive(scc.size, iterations, started.elapsed());

            result.total_iterations += iterations as usize;
            result
                .scc_iteration_counts
                .push((member_key_texts.clone(), iterations));

            if budget_exceeded {
                result.budget_exceeded_sccs += 1;
            }

            result.updated_summaries += scc_summaries.len();

            if !scc_summaries.is_empty() || !scc_events.is_empty() {
                merge_updated_summaries(db, &scc_summaries, &[]);
                pending_events.extend(scc_events);
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
            let started = std::time::Instant::now();
            let (scc_summaries, scc_events) = process_non_recursive_scc(db, scc);
            profile.non_recursive += started.elapsed();

            result.updated_summaries += scc_summaries.len();

            if !scc_summaries.is_empty() || !scc_events.is_empty() {
                merge_updated_summaries(db, &scc_summaries, &[]);
                pending_events.extend(scc_events);
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

    if !pending_events.is_empty() {
        merge_updated_summaries(db, &[], &pending_events);
    }
    if summary_metadata_dirty {
        db.refresh_summary_metadata_after_bulk_update();
    }
    profile.log(&result);

    result
}

/// Where the closure spends its time, logged once per run at debug level.
#[derive(Default)]
struct ClosureProfile {
    non_recursive: std::time::Duration,
    recursive: std::time::Duration,
    largest_recursive: Vec<(usize, u32, u128)>,
}

impl ClosureProfile {
    const LARGEST_KEPT: usize = 5;

    fn record_recursive(&mut self, size: usize, iterations: u32, elapsed: std::time::Duration) {
        self.recursive += elapsed;
        self.largest_recursive
            .push((size, iterations, elapsed.as_millis()));
        self.largest_recursive
            .sort_by(|left, right| right.0.cmp(&left.0).then(right.2.cmp(&left.2)));
        self.largest_recursive.truncate(Self::LARGEST_KEPT);
    }

    fn log(&self, result: &SccClosureResult) {
        let largest_recursive = self
            .largest_recursive
            .iter()
            .map(|(size, iterations, millis)| format!("{size}/{iterations}/{millis}ms"))
            .collect::<Vec<_>>()
            .join(",");
        tracing::debug!(
            target: "polint::kernel::stage",
            provider = "polint.direct_summaries",
            sccs = result.total_sccs_processed,
            recursive_sccs = result.recursive_sccs,
            budget_exceeded_sccs = result.budget_exceeded_sccs,
            total_iterations = result.total_iterations,
            non_recursive_ms = self.non_recursive.as_millis() as u64,
            recursive_ms = self.recursive.as_millis() as u64,
            largest_recursive = largest_recursive.as_str(),
            "scc closure profile"
        );
    }
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
    let summary_store = db.summary_store();
    let call_store = db.calls_store();
    let index_of = scc
        .members
        .iter()
        .enumerate()
        .map(|(index, function)| (*function, index))
        .collect::<std::collections::HashMap<_, _>>();

    // Every summary text this SCC can see, split into its `;` parts once and
    // numbered: the members' own payloads and those of the callees outside it.
    let mut parts = PartTable::default();
    let mut states = scc
        .members
        .iter()
        .map(|function| {
            MemberState::from_digests(&mut parts, &summary_digests(summary_store, *function))
        })
        .collect::<Vec<_>>();
    let callees = scc
        .members
        .iter()
        .map(|function| {
            let mut callees = MemberCallees::default();
            let mut seen = BTreeSet::new();
            for target in call_store.outgoing_by_function(*function) {
                if target.status != CallTargetStatus::Resolved {
                    continue;
                }
                let Some(callee) = target.target_function else {
                    continue;
                };
                if !seen.insert(callee) {
                    continue;
                }
                match index_of.get(&callee) {
                    Some(index) => callees.internal.push(*index),
                    None => callees
                        .external
                        .push(summary_digests(summary_store, callee)),
                }
            }
            callees
        })
        .collect::<Vec<_>>();
    // Number every part before sizing a set: the universe is only known once
    // the callees outside the SCC have been read too.
    let external_parts = callees
        .iter()
        .map(|callees| ExternalParts::new(&mut parts, &callees.external))
        .collect::<Vec<_>>();
    let words = parts.len().div_ceil(64);
    for state in &mut states {
        state.size_sets(words);
    }
    let external = external_parts
        .into_iter()
        .map(|parts| parts.join(words))
        .collect::<Vec<_>>();

    let mut iteration = 0_u32;
    let mut converged = false;
    let mut scratch = JoinedDomain::empty(words);
    while iteration < config.max_iterations {
        iteration += 1;
        let mut any_changed = false;
        // Members in their stable-key order, each reading the states the members
        // before it in this round already moved to: the round structure of the
        // string join this replaces, so a round count means the same thing.
        for member in 0..states.len() {
            let mut changed = false;
            for domain in JOINED_DOMAINS {
                scratch.load(&states[member].domains[domain]);
                scratch.join(&external[member].domains[domain]);
                for callee in &callees[member].internal {
                    scratch.join_state(&states[*callee].domains[domain]);
                }
                changed |= states[member].domains[domain].take(&scratch);
            }
            any_changed |= changed;
        }
        if !any_changed {
            converged = true;
            break;
        }
    }
    let budget_exceeded = !converged;

    let mut result_summaries = Vec::new();
    let mut result_events = Vec::new();
    for (member, function) in scc.members.iter().enumerate() {
        let state = &states[member];
        for fact in summary_store.summaries_by_function(*function) {
            let new_digest = state.text(&parts, fact.domain);
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
                callable_stable_key: scc.member_stable_keys[member],
                function: *function,
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
            let callable_stable_key = scc.member_stable_keys[member];
            result_events.push(SummaryEventFact {
                id: SummaryEventId(0),
                callable_stable_key,
                function: *function,
                domain: SummaryDomainKind::ControlEffects,
                event_kind: "budget_exceeded".to_string(),
                reason: format!(
                    "SCC fixpoint did not converge within {} iterations",
                    config.max_iterations
                ),
                status: SummaryStatus::BudgetExceeded,
                precision: SummaryPrecision::UnknownTop,
                stable_key: stable_key_from_parts(
                    interner,
                    FactFamily::SummaryEvent,
                    &[
                        (
                            "callable",
                            interner.resolve(callable_stable_key).to_string(),
                        ),
                        ("domain", "scc_closure".to_string()),
                        ("event", "budget_exceeded".to_string()),
                    ],
                ),
            });
        }
    }

    (result_summaries, result_events, iteration, budget_exceeded)
}

// ---------------------------------------------------------------------------
// Recursive SCC fixpoint over numbered parts
// ---------------------------------------------------------------------------
//
// The recursive join treats a summary text as the set of its non-empty `;`
// parts. A member's text for a domain stays exactly as stored until a callee
// contributes a part it lacks; from then on it is the sorted, de-duplicated
// parts joined by `;`. A domain the member has no summary for enters its map,
// with empty text, as soon as a callee has that domain. `DataFlowTito` is
// never joined. A round changes a member when a domain enters its map or a
// domain's part set grows, which is exactly when its map of texts changes.
// Holding the part sets as bitsets over numbered parts computes the same
// rounds without re-splitting and re-joining every callee's text per member
// per round.

/// The three domains the recursive join moves, by index into the state arrays.
const JOINED_DOMAINS: [usize; 3] = [0, 1, 2];

fn joined_domain_index(domain: SummaryDomainKind) -> Option<usize> {
    match domain {
        SummaryDomainKind::ControlEffects => Some(0),
        SummaryDomainKind::CallEffects => Some(1),
        SummaryDomainKind::MemoryEffects => Some(2),
        SummaryDomainKind::DataFlowTito => None,
    }
}

/// A function's summary texts by domain, the last summary of a domain winning,
/// as the string join's map building did.
fn summary_digests(
    store: &crate::analysis_neutral::summaries::store::SummaryStore,
    function: FunctionId,
) -> BTreeMap<SummaryDomainKind, String> {
    let mut digests = BTreeMap::new();
    for fact in store.summaries_by_function(function) {
        digests.insert(fact.domain, fact.payload_digest.clone());
    }
    digests
}

#[derive(Default)]
struct PartTable {
    ids: std::collections::HashMap<String, u32>,
    texts: Vec<String>,
}

impl PartTable {
    fn len(&self) -> usize {
        self.texts.len()
    }

    fn id(&mut self, part: &str) -> u32 {
        if let Some(id) = self.ids.get(part) {
            return *id;
        }
        let id = u32::try_from(self.texts.len()).expect("fewer than 2^32 summary parts");
        self.ids.insert(part.to_string(), id);
        self.texts.push(part.to_string());
        id
    }

    fn ids_of(&mut self, text: &str) -> Vec<u32> {
        text.split(';')
            .filter(|part| !part.is_empty())
            .map(|part| self.id(part))
            .collect()
    }
}

fn set_bit(bits: &mut [u64], id: u32) {
    bits[id as usize / 64] |= 1 << (id % 64);
}

#[derive(Clone, Default)]
struct DomainState {
    present: bool,
    /// The stored text, kept until the first part it lacks arrives.
    original: String,
    /// Whether a part was added: the text is then the canonical join.
    canonical: bool,
    /// Part ids, collected before the universe is known.
    initial: Vec<u32>,
    bits: Vec<u64>,
}

impl DomainState {
    /// Moves to the joined state in `joined`; true when that changes the map
    /// of texts this domain is part of.
    fn take(&mut self, joined: &JoinedDomain) -> bool {
        let entered = joined.present && !self.present;
        let grew = joined.bits != self.bits;
        if entered {
            self.present = true;
        }
        if grew {
            self.bits.copy_from_slice(&joined.bits);
            self.canonical = true;
        }
        entered || grew
    }
}

struct MemberState {
    domains: [DomainState; 3],
    /// Texts of the domains the join never touches, by domain.
    untouched: BTreeMap<SummaryDomainKind, String>,
}

impl MemberState {
    fn from_digests(parts: &mut PartTable, digests: &BTreeMap<SummaryDomainKind, String>) -> Self {
        let mut domains: [DomainState; 3] = Default::default();
        let mut untouched = BTreeMap::new();
        for (domain, text) in digests {
            match joined_domain_index(*domain) {
                Some(index) => {
                    domains[index] = DomainState {
                        present: true,
                        original: text.clone(),
                        canonical: false,
                        initial: parts.ids_of(text),
                        bits: Vec::new(),
                    };
                }
                None => {
                    untouched.insert(*domain, text.clone());
                }
            }
        }
        Self { domains, untouched }
    }

    fn size_sets(&mut self, words: usize) {
        for domain in &mut self.domains {
            domain.bits = vec![0; words];
            for id in std::mem::take(&mut domain.initial) {
                set_bit(&mut domain.bits, id);
            }
        }
    }

    /// The member's text for `domain` once the join is done.
    fn text(&self, parts: &PartTable, domain: SummaryDomainKind) -> String {
        let Some(index) = joined_domain_index(domain) else {
            return self.untouched.get(&domain).cloned().unwrap_or_default();
        };
        let state = &self.domains[index];
        if !state.canonical {
            return state.original.clone();
        }
        let mut texts = Vec::new();
        for (word_index, word) in state.bits.iter().enumerate() {
            let mut word = *word;
            while word != 0 {
                let bit = word.trailing_zeros() as usize;
                texts.push(parts.texts[word_index * 64 + bit].as_str());
                word &= word - 1;
            }
        }
        texts.sort_unstable();
        texts.join(";")
    }
}

#[derive(Default)]
struct MemberCallees {
    /// Members of the same SCC, by index.
    internal: Vec<usize>,
    /// Summary texts of callees outside the SCC, already closed.
    external: Vec<BTreeMap<SummaryDomainKind, String>>,
}

/// The parts a member's callees outside the SCC contribute, numbered.
#[derive(Default)]
struct ExternalParts {
    present: [bool; 3],
    ids: [Vec<u32>; 3],
}

impl ExternalParts {
    fn new(parts: &mut PartTable, callees: &[BTreeMap<SummaryDomainKind, String>]) -> Self {
        let mut external = Self::default();
        for digests in callees {
            for (domain, text) in digests {
                let Some(index) = joined_domain_index(*domain) else {
                    continue;
                };
                external.present[index] = true;
                external.ids[index].extend(parts.ids_of(text));
            }
        }
        external
    }

    fn join(self, words: usize) -> ExternalJoin {
        let mut domains = [
            JoinedDomain::empty(words),
            JoinedDomain::empty(words),
            JoinedDomain::empty(words),
        ];
        for (index, ids) in self.ids.into_iter().enumerate() {
            domains[index].present = self.present[index];
            for id in ids {
                set_bit(&mut domains[index].bits, id);
            }
        }
        ExternalJoin { domains }
    }
}

/// What a member's callees outside the SCC contribute every round.
struct ExternalJoin {
    domains: [JoinedDomain; 3],
}

/// A domain's presence and part set while a member's join is computed.
#[derive(Clone)]
struct JoinedDomain {
    present: bool,
    bits: Vec<u64>,
}

impl JoinedDomain {
    fn empty(words: usize) -> Self {
        Self {
            present: false,
            bits: vec![0; words],
        }
    }

    fn load(&mut self, state: &DomainState) {
        self.present = state.present;
        self.bits.copy_from_slice(&state.bits);
    }

    fn join(&mut self, other: &JoinedDomain) {
        self.present |= other.present;
        for (word, other) in self.bits.iter_mut().zip(&other.bits) {
            *word |= other;
        }
    }

    fn join_state(&mut self, other: &DomainState) {
        self.present |= other.present;
        for (word, other) in self.bits.iter_mut().zip(&other.bits) {
            *word |= other;
        }
    }
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

    // A caller's payload names each callee's payload by reference: verbatim when
    // short, by digest when long. Embedding the full text nested every callee's
    // payload inside each of its callers, so payloads grew with the number of
    // call paths below a function; the reference is a pure function of the text,
    // so equal callee payloads still compose to equal caller payloads.
    for info in callee_info {
        if let Some(control_digest) = info.callee_digests.get(&SummaryDomainKind::ControlEffects) {
            callee_control_digests
                .push(crate::analysis_api::compact_key_reference(control_digest).into_owned());
        }
        if let Some(memory_digest) = info.callee_digests.get(&SummaryDomainKind::MemoryEffects) {
            callee_memory_digests
                .push(crate::analysis_api::compact_key_reference(memory_digest).into_owned());
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
            stable_key: stable_key_from_parts(
                interner,
                FactFamily::SummaryEvent,
                &[
                    ("callable", interner.resolve(callable_key).to_string()),
                    ("domain", "scc_closure".to_string()),
                    ("event", "unresolved_callee_in_closure".to_string()),
                ],
            ),
        });
    }

    result
}

// ---------------------------------------------------------------------------
// Digest-level join for fixpoint iteration
// ---------------------------------------------------------------------------

#[cfg(test)]
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
    // The digest is kept per SCC and stored for backdating, so the summaries'
    // text is embedded by reference: equal text, equal digest.
    format!(
        "scc_digest:{}:{}",
        member_keys.join(","),
        crate::analysis_api::compact_key_reference(&combined)
    )
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

/// The string join `process_recursive_scc` replaced, kept as the oracle the
/// differential test compares it with.
#[cfg(test)]
fn legacy_process_recursive_scc(
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
                stable_key: stable_key_from_parts(
                    interner,
                    FactFamily::SummaryEvent,
                    &[
                        (
                            "callable",
                            interner.resolve(state.callable_stable_key).to_string(),
                        ),
                        ("domain", "scc_closure".to_string()),
                        ("event", "budget_exceeded".to_string()),
                    ],
                ),
            });
        }
    }

    (result_summaries, result_events, iteration, budget_exceeded)
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
    // Differential: the numbered-part fixpoint against the string join
    // -----------------------------------------------------------------------

    /// A small deterministic generator, so a failing seed reproduces.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }

        fn below(&mut self, bound: u64) -> u64 {
            self.next() % bound
        }
    }

    fn random_payload(rng: &mut Lcg) -> String {
        const PARTS: [&str; 7] = ["p0", "p1", "p2", "p3", "", "callee_control:#c0", "p1"];
        (0..rng.below(4))
            .map(|_| PARTS[rng.below(PARTS.len() as u64) as usize])
            .collect::<Vec<_>>()
            .join(";")
    }

    #[test]
    fn numbered_part_fixpoint_matches_the_string_join_on_random_graphs() {
        const DOMAINS: [SummaryDomainKind; 4] = [
            SummaryDomainKind::ControlEffects,
            SummaryDomainKind::CallEffects,
            SummaryDomainKind::MemoryEffects,
            SummaryDomainKind::DataFlowTito,
        ];
        let mut recursive_sccs_compared = 0;
        let mut budget_trips = 0;
        for seed in 0..300_u64 {
            let mut rng = Lcg(seed + 1);
            let functions = 2 + rng.below(9);
            let mut summaries = Vec::new();
            for function in 1..=functions {
                let key = format!("func::diff{seed}::{function}");
                for domain in DOMAINS {
                    if rng.below(5) == 0 {
                        continue;
                    }
                    let mut fact = summary_fact(function, &key, domain);
                    fact.payload_digest = random_payload(&mut rng);
                    summaries.push(fact);
                }
            }
            let mut sites = Vec::new();
            let mut targets = Vec::new();
            for caller in 1..=functions {
                for _ in 0..rng.below(4) {
                    let id = sites.len() as u64 + 1;
                    let callee = 1 + rng.below(functions + 1);
                    let mut site = call_site(id, caller);
                    site.stable_key = stable_key_for_test(&format!("site:diff{seed}:{id}"));
                    sites.push(site);
                    let mut target = call_target(id, id, caller, callee);
                    target.stable_key = stable_key_for_test(&format!("target:diff{seed}:{id}"));
                    if rng.below(8) == 0 {
                        target.status = CallTargetStatus::Unresolved;
                    }
                    targets.push(target);
                }
            }
            let db = build_db(summaries, sites, targets);
            let schedule = compute_scc_schedule(&db);
            let config = SccClosureConfig {
                max_iterations: 1 + rng.below(4) as u32,
                enable_backdating: true,
            };
            for scc in schedule.sccs.iter().filter(|scc| scc.is_recursive) {
                let numbered = process_recursive_scc(&db, scc, &config);
                let strings = legacy_process_recursive_scc(&db, scc, &config);
                assert_eq!(numbered.0, strings.0, "summaries differ for seed {seed}");
                assert_eq!(numbered.1, strings.1, "events differ for seed {seed}");
                assert_eq!(numbered.2, strings.2, "rounds differ for seed {seed}");
                assert_eq!(numbered.3, strings.3, "budget differs for seed {seed}");
                recursive_sccs_compared += 1;
                budget_trips += usize::from(numbered.3);
            }
        }
        assert!(
            recursive_sccs_compared > 100,
            "the generator must exercise recursive SCCs, got {recursive_sccs_compared}"
        );
        assert!(
            budget_trips > 0 && budget_trips < recursive_sccs_compared,
            "the generator must exercise both convergence and budget trips, got \
             {budget_trips} of {recursive_sccs_compared}"
        );
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
        let b_control_effects = store
            .summaries_by_function(FunctionId::from_raw(2))
            .into_iter()
            .find(|summary| summary.domain == SummaryDomainKind::ControlEffects)
            .expect("B should have control effects");
        assert!(
            b_control_effects
                .payload_digest
                .contains("callee_propagated:"),
            "B's control summary should be closed over C, got {}",
            b_control_effects.payload_digest
        );
        // A names B's control summary by reference; the reference must be the one
        // of B's closed summary, not of B's direct one.
        let closed_reference =
            crate::analysis_api::compact_key_reference(&b_control_effects.payload_digest);
        assert!(
            a_call_effects
                .payload_digest
                .contains(&format!("callee_control:{closed_reference}")),
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
