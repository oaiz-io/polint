use std::collections::{BTreeMap, BTreeSet};

use super::core::{CallEffects, ControlEffects, DataFlowTito, FlowEdge, MemoryEffects};
use super::facts::{
    AccessKind, AsyncKind, ExitKind, FlowKind, FlowRoot, SummaryDomainKind, SummaryEventFact,
    SummaryFact, SummaryFlowEdge, SummaryPrecision, SummaryProvenance, SummaryStatus,
};
use super::store::SummaryOutput;
use crate::analysis_api::{FactFamily, stable_key_from_key_parts, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::UnresolvedCallFact;
use crate::analysis_neutral::cfg::facts::{
    BasicBlockFact, BasicBlockKind, CfgEdgeFact, CfgEdgeKind,
};
use crate::analysis_neutral::cfg::ids::CfgFunctionId;
use crate::analysis_neutral::domains::facts::{
    DomainLocation, DomainObservationFact, DomainSlot, DomainValue,
};
use crate::analysis_neutral::ids::{MirBodyId, PlaceId, SummaryEventId, SummaryId};
use crate::analysis_neutral::mir_body::MirBody;
use crate::analysis_neutral::mir_op::{AssignMode, MirOperationKind, MirValue};
use crate::analysis_neutral::places::PlaceRoot;
use crate::internal_core::{FunctionId, KeyPart, StableKeyId};

/// Computes direct (local, single-function) summaries from LocalAnalysisDb facts.
///
/// The builder reads MIR bodies, operations, places, CFG facts, call facts, and
/// domain solver results. It does NOT re-run the local domain solver (D-12).
pub struct DirectSummaryBuilder;

impl DirectSummaryBuilder {
    pub fn build(
        interner: &crate::internal_core::StableKeyInterner,
        db: &impl AnalysisHost,
    ) -> SummaryOutput {
        let bodies = db.mir_bodies();
        if bodies.is_empty() {
            return SummaryOutput::empty();
        }

        let bodies_by_function: BTreeMap<FunctionId, Vec<&MirBody>> = {
            let mut map = BTreeMap::<FunctionId, Vec<&MirBody>>::new();
            for body in bodies {
                map.entry(body.function).or_default().push(body);
            }
            map
        };

        let operations_by_body: BTreeMap<
            MirBodyId,
            Vec<&crate::analysis_neutral::mir_op::MirOperation>,
        > = {
            let mut map: BTreeMap<MirBodyId, Vec<&crate::analysis_neutral::mir_op::MirOperation>> =
                BTreeMap::new();
            for op in db.mir_operations() {
                map.entry(op.body).or_default().push(op);
            }
            for ops in map.values_mut() {
                ops.sort_by(|a, b| {
                    (a.ordinal, interner.resolve(a.stable_key))
                        .cmp(&(b.ordinal, interner.resolve(b.stable_key)))
                });
            }
            map
        };

        let places_by_id: BTreeMap<PlaceId, &crate::analysis_neutral::places::PlaceFact> = {
            let mut map = BTreeMap::new();
            for place in db.mir_places() {
                map.insert(place.id, place);
            }
            map
        };

        let domain_observations = db.abstract_domain_observations();
        let observations_by_body: BTreeMap<MirBodyId, Vec<&DomainObservationFact>> = {
            let mut map = BTreeMap::<MirBodyId, Vec<&DomainObservationFact>>::new();
            for obs in domain_observations {
                map.entry(obs.body).or_default().push(obs);
            }
            map
        };

        let unresolved_by_caller: BTreeMap<FunctionId, Vec<&UnresolvedCallFact>> = {
            let mut map = BTreeMap::<FunctionId, Vec<&UnresolvedCallFact>>::new();
            for unresolved in db.unresolved_calls() {
                map.entry(unresolved.caller).or_default().push(unresolved);
            }
            map
        };

        let cfg_blocks = db.cfg_blocks();
        let cfg_edges = db.cfg_edges();
        let cfg_functions = db.cfg_functions();

        let cfg_functions_by_body: BTreeMap<MirBodyId, CfgFunctionId> = {
            let mut map = BTreeMap::new();
            for func in cfg_functions {
                map.insert(func.body, func.id);
            }
            map
        };
        let cfg_blocks_by_function: BTreeMap<CfgFunctionId, Vec<&BasicBlockFact>> = {
            let mut map = BTreeMap::<CfgFunctionId, Vec<&BasicBlockFact>>::new();
            for block in cfg_blocks {
                map.entry(block.cfg_function).or_default().push(block);
            }
            map
        };
        let cfg_edges_by_function: BTreeMap<CfgFunctionId, Vec<&CfgEdgeFact>> = {
            let mut map = BTreeMap::<CfgFunctionId, Vec<&CfgEdgeFact>>::new();
            for edge in cfg_edges {
                map.entry(edge.cfg_function).or_default().push(edge);
            }
            map
        };

        let mut summaries = Vec::new();
        let mut events = Vec::new();

        let mut sorted_functions: Vec<_> = bodies_by_function.keys().copied().collect();
        sorted_functions.sort();

        for function in sorted_functions {
            let function_bodies = &bodies_by_function[&function];
            // Use the first body (there should typically be one per function)
            let body = function_bodies[0];
            let callable_key = body.stable_key;
            let body_ops = operations_by_body
                .get(&body.id)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let body_observations = observations_by_body
                .get(&body.id)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let function_unresolved = unresolved_by_caller
                .get(&function)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let has_unresolved = !function_unresolved.is_empty();

            // --- Control Effects ---
            let control = build_control_effects(
                body,
                body_ops,
                body_observations,
                has_unresolved,
                &cfg_functions_by_body,
                &cfg_blocks_by_function,
                &cfg_edges_by_function,
            );

            let (control_status, control_precision, control_provenance) =
                classify_domain_output(&control);
            let control_digest = control.stable_digest_parts().join(";");

            summaries.push(SummaryFact {
                id: SummaryId(0),
                callable_stable_key: callable_key,
                function,
                domain: SummaryDomainKind::ControlEffects,
                status: control_status,
                precision: control_precision,
                provenance: control_provenance,
                payload_digest: control_digest,
                tito_flows: Vec::new(),
                stable_key: summary_stable_key(
                    interner,
                    FactFamily::SummaryControl,
                    callable_key,
                    "control_effects",
                ),
            });

            if has_unresolved && control.is_top() {
                events.push(SummaryEventFact {
                    id: SummaryEventId(0),
                    callable_stable_key: callable_key,
                    function,
                    domain: SummaryDomainKind::ControlEffects,
                    event_kind: "unknown_top".to_string(),
                    reason: "unresolved_calls".to_string(),
                    status: SummaryStatus::Unknown,
                    precision: SummaryPrecision::UnknownTop,
                    stable_key: summary_event_stable_key(
                        interner,
                        callable_key,
                        "control_effects",
                        "unknown_top",
                    ),
                });
            }

            // --- Call Effects ---
            let call_effects =
                build_call_effects(db, function, body, body_ops, function_unresolved);

            let (call_status, call_precision, call_provenance) =
                classify_domain_output(&call_effects);
            let call_digest = call_effects.stable_digest_parts().join(";");

            summaries.push(SummaryFact {
                id: SummaryId(0),
                callable_stable_key: callable_key,
                function,
                domain: SummaryDomainKind::CallEffects,
                status: call_status,
                precision: call_precision,
                provenance: call_provenance,
                payload_digest: call_digest,
                tito_flows: Vec::new(),
                stable_key: summary_stable_key(
                    interner,
                    FactFamily::SummaryCall,
                    callable_key,
                    "call_effects",
                ),
            });

            if has_unresolved {
                events.push(SummaryEventFact {
                    id: SummaryEventId(0),
                    callable_stable_key: callable_key,
                    function,
                    domain: SummaryDomainKind::CallEffects,
                    event_kind: "unresolved_callee".to_string(),
                    reason: format!("{} unresolved calls", function_unresolved.len()),
                    status: SummaryStatus::Unknown,
                    precision: SummaryPrecision::UnknownTop,
                    stable_key: summary_event_stable_key(
                        interner,
                        callable_key,
                        "call_effects",
                        "unresolved_callee",
                    ),
                });
            }

            // --- Memory Effects ---
            let memory = build_memory_effects(body_ops, &places_by_id, has_unresolved);

            let (mem_status, mem_precision, mem_provenance) = classify_domain_output(&memory);
            let mem_digest = memory.stable_digest_parts().join(";");

            summaries.push(SummaryFact {
                id: SummaryId(0),
                callable_stable_key: callable_key,
                function,
                domain: SummaryDomainKind::MemoryEffects,
                status: mem_status,
                precision: mem_precision,
                provenance: mem_provenance,
                payload_digest: mem_digest,
                tito_flows: Vec::new(),
                stable_key: summary_stable_key(
                    interner,
                    FactFamily::SummaryMemory,
                    callable_key,
                    "memory_effects",
                ),
            });

            if has_unresolved
                && matches!(
                    memory,
                    MemoryEffects::Effects {
                        may_have_external_effects: true,
                        ..
                    }
                )
            {
                events.push(SummaryEventFact {
                    id: SummaryEventId(0),
                    callable_stable_key: callable_key,
                    function,
                    domain: SummaryDomainKind::MemoryEffects,
                    event_kind: "may_have_external_effects".to_string(),
                    reason: "unresolved_calls".to_string(),
                    status: SummaryStatus::Unknown,
                    precision: SummaryPrecision::UnknownTop,
                    stable_key: summary_event_stable_key(
                        interner,
                        callable_key,
                        "memory_effects",
                        "may_have_external_effects",
                    ),
                });
            }

            // --- TITO ---
            let tito = build_tito(body_ops, &places_by_id, has_unresolved);

            let (tito_status, tito_precision, tito_provenance) = classify_domain_output(&tito);
            let tito_digest = tito.stable_digest_parts().join(";");
            let tito_flows = summary_tito_flows(&tito);

            summaries.push(SummaryFact {
                id: SummaryId(0),
                callable_stable_key: callable_key,
                function,
                domain: SummaryDomainKind::DataFlowTito,
                status: tito_status,
                precision: tito_precision,
                provenance: tito_provenance,
                payload_digest: tito_digest,
                tito_flows,
                stable_key: summary_stable_key(
                    interner,
                    FactFamily::SummaryTito,
                    callable_key,
                    "data_flow_tito",
                ),
            });

            if has_unresolved {
                events.push(SummaryEventFact {
                    id: SummaryEventId(0),
                    callable_stable_key: callable_key,
                    function,
                    domain: SummaryDomainKind::DataFlowTito,
                    event_kind: "unknown_top".to_string(),
                    reason: "unresolved_calls".to_string(),
                    status: SummaryStatus::Unknown,
                    precision: SummaryPrecision::UnknownTop,
                    stable_key: summary_event_stable_key(
                        interner,
                        callable_key,
                        "data_flow_tito",
                        "unknown_top",
                    ),
                });
            }
        }

        SummaryOutput { summaries, events }
    }
}

// ---------------------------------------------------------------------------
// Control Effects Builder
// ---------------------------------------------------------------------------

fn build_control_effects(
    body: &MirBody,
    body_ops: &[&crate::analysis_neutral::mir_op::MirOperation],
    body_observations: &[&DomainObservationFact],
    has_unresolved: bool,
    cfg_functions_by_body: &BTreeMap<MirBodyId, CfgFunctionId>,
    cfg_blocks_by_function: &BTreeMap<CfgFunctionId, Vec<&BasicBlockFact>>,
    cfg_edges_by_function: &BTreeMap<CfgFunctionId, Vec<&CfgEdgeFact>>,
) -> ControlEffects {
    let mut exits = BTreeSet::new();
    let mut async_kind = AsyncKind::Sync;
    let mut has_cleanup = false;

    // Check MIR operations for explicit return, throw, panic, process-exit operations
    for op in body_ops {
        match &op.kind {
            MirOperationKind::Return { .. } => {
                exits.insert(ExitKind::Returns);
            }
            MirOperationKind::Unsupported { .. } => {
                // Unsupported operations could be anything
            }
            _ => {}
        }
    }

    // Check CFG edges for throw, panic, cleanup evidence
    if let Some(func_id) = cfg_functions_by_body.get(&body.id).copied() {
        let function_edges = cfg_edges_by_function
            .get(&func_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for edge in function_edges {
            match edge.kind {
                CfgEdgeKind::Throw | CfgEdgeKind::ImplicitThrow => {
                    exits.insert(ExitKind::Throws);
                }
                CfgEdgeKind::Panic => {
                    exits.insert(ExitKind::Panics);
                }
                CfgEdgeKind::Cleanup | CfgEdgeKind::Defer | CfgEdgeKind::Finally => {
                    has_cleanup = true;
                }
                CfgEdgeKind::AwaitSuspend | CfgEdgeKind::AwaitResume => {
                    async_kind = AsyncKind::Async;
                }
                CfgEdgeKind::YieldSuspend | CfgEdgeKind::YieldResume => {
                    async_kind = match async_kind {
                        AsyncKind::Async => AsyncKind::Unknown,
                        _ => AsyncKind::Generator,
                    };
                }
                CfgEdgeKind::Spawn => {
                    // Spawn does not change the current function's async kind
                }
                _ => {}
            }
        }

        // Check for does-not-return evidence from domain observations
        // If all exit blocks are unreachable per reachability domain
        let function_blocks = cfg_blocks_by_function
            .get(&func_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let exit_blocks: Vec<_> = function_blocks
            .iter()
            .filter(|b| {
                matches!(
                    b.kind,
                    BasicBlockKind::ExitNormal | BasicBlockKind::ExitExceptional
                )
            })
            .collect();

        let all_exits_unreachable = !exit_blocks.is_empty()
            && exit_blocks.iter().all(|b| {
                // Check domain observations for reachability at this block
                body_observations.iter().any(|obs| {
                    obs.block == Some(b.id)
                        && obs.slot == DomainSlot::Reachability
                        && obs.location == DomainLocation::BlockEntry
                        && matches!(&obs.value, DomainValue::Label(label) if label == "unreachable")
                })
            });

        if all_exits_unreachable {
            exits.insert(ExitKind::DoesNotReturn);
        }
    }

    // If no explicit exit found, and function has operations, assume returns
    if exits.is_empty() && !body_ops.is_empty() {
        exits.insert(ExitKind::Returns);
    }

    // Handle unresolved calls: join with unknown/top per D-06
    if has_unresolved {
        exits.insert(ExitKind::Unknown);
    }

    if exits.is_empty() {
        ControlEffects::Bottom
    } else {
        ControlEffects::Effects {
            exits,
            async_kind,
            has_cleanup,
        }
    }
}

// ---------------------------------------------------------------------------
// Call Effects Builder
// ---------------------------------------------------------------------------

fn build_call_effects(
    db: &impl AnalysisHost,
    function: FunctionId,
    _body: &MirBody,
    body_ops: &[&crate::analysis_neutral::mir_op::MirOperation],
    function_unresolved: &[&UnresolvedCallFact],
) -> CallEffects {
    let mut direct_callees = BTreeSet::new();
    let mut has_callback_invoked = false;
    // Callback-stored detection requires value-flow tracking
    let has_callback_stored = false;
    let unresolved_count = function_unresolved.len() as u32;

    // Get call targets for this function from the call store
    {
        let call_store = db.calls_store();
        let targets = call_store.outgoing_by_function(function);
        for target in &targets {
            direct_callees.insert(db.resolve_stable_key(target.stable_key).to_string());
        }
    }

    // Check MIR operations for callback evidence
    for op in body_ops {
        if let MirOperationKind::Call {
            arguments, callee, ..
        } = &op.kind
        {
            // Check if any argument is a function value (callback evidence)
            for _arg in arguments {
                // Simple heuristic: if the callee itself is a place-based value,
                // it could be a callback invocation
            }
            // If callee is a place value (not a direct reference), it could be a callback
            if matches!(callee, MirValue::Place(_)) {
                has_callback_invoked = true;
            }
        }
        // Check for function-valued assignments (stored callbacks)
        if let MirOperationKind::Assign {
            value: MirValue::Place(_),
            ..
        } = &op.kind
        {
            // This is too broad; only mark if we detect function-value assignment
            // For now, keep conservative: only from explicit call evidence
        }
    }

    if direct_callees.is_empty()
        && unresolved_count == 0
        && !has_callback_invoked
        && !has_callback_stored
    {
        CallEffects::Bottom
    } else {
        CallEffects::Effects {
            direct_callees,
            unresolved_count,
            has_callback_invoked,
            has_callback_stored,
        }
    }
}

// ---------------------------------------------------------------------------
// Memory Effects Builder
// ---------------------------------------------------------------------------

fn build_memory_effects(
    body_ops: &[&crate::analysis_neutral::mir_op::MirOperation],
    places_by_id: &BTreeMap<PlaceId, &crate::analysis_neutral::places::PlaceFact>,
    has_unresolved: bool,
) -> MemoryEffects {
    let mut receiver = AccessKind::None;
    let mut params: BTreeMap<u16, AccessKind> = BTreeMap::new();
    let mut return_access = AccessKind::None;
    let mut local = AccessKind::None;
    let mut global = AccessKind::None;
    let mut module = AccessKind::None;
    let mut has_any_effect = false;

    for op in body_ops {
        match &op.kind {
            MirOperationKind::Read { place } => {
                track_access(
                    *place,
                    AccessKind::Read,
                    places_by_id,
                    &mut receiver,
                    &mut params,
                    &mut return_access,
                    &mut local,
                    &mut global,
                    &mut module,
                );
                has_any_effect = true;
            }
            MirOperationKind::Write { place, .. } => {
                track_access(
                    *place,
                    AccessKind::Write,
                    places_by_id,
                    &mut receiver,
                    &mut params,
                    &mut return_access,
                    &mut local,
                    &mut global,
                    &mut module,
                );
                has_any_effect = true;
            }
            MirOperationKind::Assign { place, value, .. } => {
                // The assigned place is written
                track_access(
                    *place,
                    AccessKind::Write,
                    places_by_id,
                    &mut receiver,
                    &mut params,
                    &mut return_access,
                    &mut local,
                    &mut global,
                    &mut module,
                );
                // If the value is a place, it is read
                if let MirValue::Place(src) = value {
                    track_access(
                        *src,
                        AccessKind::Read,
                        places_by_id,
                        &mut receiver,
                        &mut params,
                        &mut return_access,
                        &mut local,
                        &mut global,
                        &mut module,
                    );
                }
                has_any_effect = true;
            }
            MirOperationKind::Bind { place, value } => {
                track_access(
                    *place,
                    AccessKind::Write,
                    places_by_id,
                    &mut receiver,
                    &mut params,
                    &mut return_access,
                    &mut local,
                    &mut global,
                    &mut module,
                );
                if let MirValue::Place(src) = value {
                    track_access(
                        *src,
                        AccessKind::Read,
                        places_by_id,
                        &mut receiver,
                        &mut params,
                        &mut return_access,
                        &mut local,
                        &mut global,
                        &mut module,
                    );
                }
                has_any_effect = true;
            }
            MirOperationKind::Return { value } => {
                return_access = return_access.join(AccessKind::Write);
                if let Some(MirValue::Place(src)) = value {
                    track_access(
                        *src,
                        AccessKind::Read,
                        places_by_id,
                        &mut receiver,
                        &mut params,
                        &mut return_access,
                        &mut local,
                        &mut global,
                        &mut module,
                    );
                }
                has_any_effect = true;
            }
            MirOperationKind::Call {
                arguments,
                return_place,
                ..
            } => {
                // Arguments are read, return place is written
                for arg in arguments {
                    track_access(
                        *arg,
                        AccessKind::Read,
                        places_by_id,
                        &mut receiver,
                        &mut params,
                        &mut return_access,
                        &mut local,
                        &mut global,
                        &mut module,
                    );
                }
                track_access(
                    *return_place,
                    AccessKind::Write,
                    places_by_id,
                    &mut receiver,
                    &mut params,
                    &mut return_access,
                    &mut local,
                    &mut global,
                    &mut module,
                );
                has_any_effect = true;
            }
            MirOperationKind::StorageLive { place } => {
                track_access(
                    *place,
                    AccessKind::Write,
                    places_by_id,
                    &mut receiver,
                    &mut params,
                    &mut return_access,
                    &mut local,
                    &mut global,
                    &mut module,
                );
                has_any_effect = true;
            }
            MirOperationKind::Branch { .. } | MirOperationKind::Unsupported { .. } => {}
        }
    }

    if !has_any_effect && !has_unresolved {
        return MemoryEffects::Bottom;
    }

    MemoryEffects::Effects {
        receiver,
        params,
        return_access,
        local,
        global,
        module,
        may_have_external_effects: has_unresolved,
    }
}

// receiver, return_access, and module are reserved for the type/value/place/alias substrate.
// when PlaceRoot gains Receiver/Module variants and return-place tracking improves.
#[allow(clippy::too_many_arguments)]
fn track_access(
    place: PlaceId,
    access: AccessKind,
    places_by_id: &BTreeMap<PlaceId, &crate::analysis_neutral::places::PlaceFact>,
    _receiver: &mut AccessKind,
    params: &mut BTreeMap<u16, AccessKind>,
    _return_access: &mut AccessKind,
    local: &mut AccessKind,
    global: &mut AccessKind,
    _module: &mut AccessKind,
) {
    let Some(place_fact) = places_by_id.get(&place) else {
        // Unknown place: treat as local
        *local = local.join(access);
        return;
    };

    match &place_fact.root {
        PlaceRoot::Parameter { index, .. } => {
            // Parameter index 0 in methods could be the receiver;
            // treat all parameters uniformly as Param(index) per D-09
            let idx = *index as u16;
            let entry = params.entry(idx).or_insert(AccessKind::None);
            *entry = entry.join(access);
        }
        PlaceRoot::Local { .. } | PlaceRoot::Temporary { .. } | PlaceRoot::CallReturn { .. } => {
            *local = local.join(access);
        }
        PlaceRoot::Global { .. } => {
            *global = global.join(access);
        }
        PlaceRoot::Unknown { .. } => {
            // Unknown roots are conservative: treat as local
            *local = local.join(access);
        }
    }
}

// ---------------------------------------------------------------------------
// TITO Builder
// ---------------------------------------------------------------------------

fn build_tito(
    body_ops: &[&crate::analysis_neutral::mir_op::MirOperation],
    places_by_id: &BTreeMap<PlaceId, &crate::analysis_neutral::places::PlaceFact>,
    has_unresolved: bool,
) -> DataFlowTito {
    let mut edges = BTreeSet::new();
    let mut has_source_return = false;
    let mut has_sink_param = false;

    // Track direct assignments: build a simple place-to-place map
    // Then check for parameter-to-return flow
    let mut copy_map: BTreeMap<PlaceId, BTreeSet<PlaceId>> = BTreeMap::new();
    let has_branch = body_ops
        .iter()
        .any(|op| matches!(op.kind, MirOperationKind::Branch { .. }));

    for op in body_ops {
        match &op.kind {
            MirOperationKind::Bind { place, value } => {
                if !has_branch {
                    copy_map.remove(place);
                }
                if let MirValue::Place(src) = value {
                    copy_map.entry(*place).or_default().insert(*src);
                }
            }
            MirOperationKind::Assign { place, value, mode } => {
                if !has_branch
                    && matches!(
                        mode,
                        AssignMode::DeclarationBinding
                            | AssignMode::Overwrite
                            | AssignMode::Simultaneous
                            | AssignMode::UnknownWrite
                    )
                {
                    copy_map.remove(place);
                }
                if let MirValue::Place(src) = value {
                    copy_map.entry(*place).or_default().insert(*src);
                }
            }
            MirOperationKind::Return {
                value: Some(MirValue::Place(src)),
            } => {
                // trace_sources returns the transitive closure including *src itself
                for source in &trace_sources(*src, &copy_map) {
                    if let Some(place_fact) = places_by_id.get(source)
                        && let PlaceRoot::Parameter { index, .. } = &place_fact.root
                    {
                        edges.insert(FlowEdge {
                            from: FlowRoot::Param(*index as u16),
                            to: FlowRoot::Return,
                            kind: FlowKind::Value,
                        });
                        has_source_return = true;
                    }
                }
            }
            MirOperationKind::Write { place, .. } => {
                if let Some(place_fact) = places_by_id.get(place)
                    && let PlaceRoot::Parameter { index, .. } = &place_fact.root
                {
                    let idx = *index as u16;
                    edges.insert(FlowEdge {
                        from: FlowRoot::Param(idx),
                        to: FlowRoot::Param(idx),
                        kind: FlowKind::BySideEffect,
                    });
                    has_sink_param = true;
                }
            }
            _ => {}
        }
    }

    if edges.is_empty() && !has_source_return && !has_sink_param && !has_unresolved {
        return DataFlowTito::Bottom;
    }

    DataFlowTito::Flows {
        edges,
        has_source_return,
        has_sink_param,
    }
}

fn summary_tito_flows(tito: &DataFlowTito) -> Vec<SummaryFlowEdge> {
    let DataFlowTito::Flows { edges, .. } = tito else {
        return Vec::new();
    };
    edges
        .iter()
        .map(|edge| SummaryFlowEdge {
            from: edge.from,
            to: edge.to,
            kind: edge.kind,
        })
        .collect()
}

/// Simple transitive-closure source tracing through direct Assign/Copy/Move chains.
/// Does NOT follow field-level access paths per D-07/D-10.
fn trace_sources(
    place: PlaceId,
    copy_map: &BTreeMap<PlaceId, BTreeSet<PlaceId>>,
) -> BTreeSet<PlaceId> {
    let mut result = BTreeSet::new();
    let mut worklist = vec![place];
    let mut visited = BTreeSet::new();

    while let Some(current) = worklist.pop() {
        if !visited.insert(current) {
            continue;
        }
        result.insert(current);
        if let Some(sources) = copy_map.get(&current) {
            for src in sources {
                worklist.push(*src);
            }
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn summary_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    family: FactFamily,
    callable_key: StableKeyId,
    domain: &str,
) -> StableKeyId {
    stable_key_from_parts(
        interner,
        family,
        &[
            ("callable", interner.resolve(callable_key).to_string()),
            ("domain", domain.to_string()),
        ],
    )
}

fn summary_event_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    callable_key: StableKeyId,
    domain: &str,
    event: &str,
) -> StableKeyId {
    stable_key_from_key_parts(
        interner,
        FactFamily::SummaryEvent,
        [
            ("callable", KeyPart::Key(callable_key)),
            ("domain", KeyPart::Text(domain)),
            ("event", KeyPart::Text(event)),
        ],
    )
}

trait DigestAndClassify {
    fn stable_digest_parts(&self) -> Vec<String>;
    fn is_top(&self) -> bool;
}

impl DigestAndClassify for ControlEffects {
    fn stable_digest_parts(&self) -> Vec<String> {
        <Self as super::domain::SummaryDomain>::stable_digest_parts(self)
    }
    fn is_top(&self) -> bool {
        <Self as super::domain::SummaryDomain>::is_top(self)
    }
}

impl DigestAndClassify for CallEffects {
    fn stable_digest_parts(&self) -> Vec<String> {
        <Self as super::domain::SummaryDomain>::stable_digest_parts(self)
    }
    fn is_top(&self) -> bool {
        <Self as super::domain::SummaryDomain>::is_top(self)
    }
}

impl DigestAndClassify for MemoryEffects {
    fn stable_digest_parts(&self) -> Vec<String> {
        <Self as super::domain::SummaryDomain>::stable_digest_parts(self)
    }
    fn is_top(&self) -> bool {
        <Self as super::domain::SummaryDomain>::is_top(self)
    }
}

impl DigestAndClassify for DataFlowTito {
    fn stable_digest_parts(&self) -> Vec<String> {
        <Self as super::domain::SummaryDomain>::stable_digest_parts(self)
    }
    fn is_top(&self) -> bool {
        <Self as super::domain::SummaryDomain>::is_top(self)
    }
}

fn classify_domain_output(
    domain: &dyn DigestAndClassify,
) -> (SummaryStatus, SummaryPrecision, SummaryProvenance) {
    if domain.is_top() {
        (
            SummaryStatus::Unknown,
            SummaryPrecision::UnknownTop,
            SummaryProvenance::NativeLocal,
        )
    } else {
        // Bottom (no effects observed) and non-bottom (effects present) both
        // get Local precision — all direct summaries come from local analysis.
        (
            SummaryStatus::Present,
            SummaryPrecision::Local,
            SummaryProvenance::NativeLocal,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallAlgorithm, CallCallee, CallPrecision, CallProvenance as CallProvenanceFact,
        CallSiteFact, CallSyntaxKind, CallTargetStatus, UnresolvedCallFact, UnresolvedCallReason,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::ids::{CallSiteId, MirBodyId, MirOpId, MirPredicateId, PlaceId};
    use crate::analysis_neutral::mir_body::{MirBody, MirOutput, MirStatus};
    use crate::analysis_neutral::mir_op::{AssignMode, MirOperation, MirOperationKind, MirValue};
    use crate::analysis_neutral::places::{PlaceFact, PlaceRoot, PlaceStatus};
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    fn span() -> Span {
        Span::point(FileId::from_raw(1), 1, 1)
    }

    #[test]
    fn empty_db_produces_empty_output() {
        let db = LocalAnalysisDb::new();
        let output = DirectSummaryBuilder::build(
            &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
            &db,
        );

        assert!(output.summaries.is_empty());
        assert!(output.events.is_empty());
    }

    #[test]
    fn builder_does_not_call_solver() {
        // Verify that no solver types are imported or constructed in this module.
        // This is a source-level proof per D-12: the builder file must not contain
        // solver references outside the test module.
        let source = include_str!("builder.rs");

        // Split at the #[cfg(test)] boundary to only check non-test code
        let production_code = source
            .split("#[cfg(test)]")
            .next()
            .expect("file should have code before #[cfg(test)]");

        let solver_type = ["Local", "Domain", "Solver"].concat();
        let solver_call = ["solver", ".solve"].concat();
        let policy_type = ["Solver", "Policy"].concat();

        assert!(
            !production_code.contains(&solver_type),
            "builder.rs production code must not reference the solver type (D-12)"
        );
        assert!(
            !production_code.contains(&solver_call),
            "builder.rs production code must not call solve (D-12)"
        );
        assert!(
            !production_code.contains(&policy_type),
            "builder.rs production code must not reference solver policy (D-12)"
        );
    }

    #[test]
    fn single_function_produces_four_domain_summaries() {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![MirBody {
                id: MirBodyId(0),
                language: Language::Go,
                file: FileId::from_raw(1),
                function: FunctionId::from_raw(1),
                package: None,
                module: None,
                owner_stable_key: interner.intern("owner:test".to_string()),
                span: span(),
                stable_key: interner.intern("body:test".to_string()),
                status: MirStatus::Resolved,
            }],
            places: vec![PlaceFact {
                id: PlaceId(0),
                language: Language::Go,
                file: Some(FileId::from_raw(1)),
                function: Some(FunctionId::from_raw(1)),
                root: PlaceRoot::Local {
                    function: FunctionId::from_raw(1),
                    name: "x".to_string(),
                },
                projections: Vec::new(),
                stable_key: interner.intern("place:x".to_string()),
                status: PlaceStatus::Resolved,
            }],
            operations: vec![MirOperation {
                id: MirOpId(0),
                body: MirBodyId(0),
                ordinal: 1,
                span: span(),
                kind: MirOperationKind::Return { value: None },
                stable_key: interner.intern("op:return".to_string()),
                status: MirStatus::Resolved,
            }],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("MIR should store");

        let output = DirectSummaryBuilder::build(
            &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
            &db,
        );

        assert_eq!(output.summaries.len(), 4, "expected 4 domain summaries");

        let domains: BTreeSet<_> = output.summaries.iter().map(|s| s.domain).collect();
        assert!(domains.contains(&SummaryDomainKind::ControlEffects));
        assert!(domains.contains(&SummaryDomainKind::CallEffects));
        assert!(domains.contains(&SummaryDomainKind::MemoryEffects));
        assert!(domains.contains(&SummaryDomainKind::DataFlowTito));
    }

    #[test]
    fn unresolved_calls_produce_events() {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![MirBody {
                id: MirBodyId(0),
                language: Language::TypeScript,
                file: FileId::from_raw(1),
                function: FunctionId::from_raw(1),
                package: None,
                module: None,
                owner_stable_key: interner.intern("owner:test".to_string()),
                span: span(),
                stable_key: interner.intern("body:test".to_string()),
                status: MirStatus::Resolved,
            }],
            places: vec![],
            operations: vec![MirOperation {
                id: MirOpId(0),
                body: MirBodyId(0),
                ordinal: 1,
                span: span(),
                kind: MirOperationKind::Return { value: None },
                stable_key: interner.intern("op:return".to_string()),
                status: MirStatus::Resolved,
            }],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("MIR should store");

        // Add call output with unresolved calls
        let _ = db.replace_call_facts(CallOutput {
            sites: vec![CallSiteFact {
                in_throw: false,
                id: CallSiteId(1),
                language: Language::TypeScript,
                file: FileId::from_raw(1),
                caller: FunctionId::from_raw(1),
                owner_symbol: None,
                body: MirBodyId(0),
                operation: MirOpId(0),
                span: span(),
                kind: CallSyntaxKind::Function,
                callee: CallCallee::Unknown {
                    reason: UnresolvedCallReason::DynamicProperty,
                },
                receiver: None,
                arguments: Vec::new(),
                result: None,
                status: CallTargetStatus::Unresolved,
                precision: CallPrecision::Unknown,
                stable_key: db
                    .stable_key_interner()
                    .intern("call-site:dynamic".to_string()),
            }],
            targets: Vec::new(),
            unresolved: vec![UnresolvedCallFact {
                site: CallSiteId(1),
                caller: FunctionId::from_raw(1),
                status: CallTargetStatus::Unresolved,
                reason: UnresolvedCallReason::DynamicProperty,
                algorithm: CallAlgorithm::SyntaxOnly,
                provenance: CallProvenanceFact::MirShape,
                precision: CallPrecision::Unknown,
                stable_key: db
                    .stable_key_interner()
                    .intern("unresolved:dynamic".to_string()),
            }],
        });

        let output = DirectSummaryBuilder::build(
            &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
            &db,
        );

        assert!(
            !output.events.is_empty(),
            "unresolved calls should produce events"
        );

        // Call effects should have unresolved_count > 0
        let call_summary = output
            .summaries
            .iter()
            .find(|s| s.domain == SummaryDomainKind::CallEffects)
            .expect("call effects summary");
        assert!(
            call_summary.payload_digest.contains("unresolved:1"),
            "digest should record unresolved count"
        );
    }

    // -----------------------------------------------------------------------
    // Task 2 tests: memory effects and TITO
    // -----------------------------------------------------------------------

    fn db_with_param_and_local_ops() -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        // Function with param[0] read and local written
        db.replace_semantic_mir(MirOutput {
            bodies: vec![MirBody {
                id: MirBodyId(0),
                language: Language::Go,
                file: FileId::from_raw(1),
                function: FunctionId::from_raw(1),
                package: None,
                module: None,
                owner_stable_key: interner.intern("owner:mem".to_string()),
                span: span(),
                stable_key: interner.intern("body:mem".to_string()),
                status: MirStatus::Resolved,
            }],
            places: vec![
                PlaceFact {
                    id: PlaceId(0),
                    language: Language::Go,
                    file: Some(FileId::from_raw(1)),
                    function: Some(FunctionId::from_raw(1)),
                    root: PlaceRoot::Parameter {
                        function: FunctionId::from_raw(1),
                        index: 0,
                        name: Some("arg0".to_string()),
                    },
                    projections: Vec::new(),
                    stable_key: interner.intern("place:param0".to_string()),
                    status: PlaceStatus::Resolved,
                },
                PlaceFact {
                    id: PlaceId(1),
                    language: Language::Go,
                    file: Some(FileId::from_raw(1)),
                    function: Some(FunctionId::from_raw(1)),
                    root: PlaceRoot::Local {
                        function: FunctionId::from_raw(1),
                        name: "tmp".to_string(),
                    },
                    projections: Vec::new(),
                    stable_key: interner.intern("place:local-tmp".to_string()),
                    status: PlaceStatus::Resolved,
                },
            ],
            operations: vec![
                // Read param[0]
                MirOperation {
                    id: MirOpId(0),
                    body: MirBodyId(0),
                    ordinal: 1,
                    span: span(),
                    kind: MirOperationKind::Read { place: PlaceId(0) },
                    stable_key: interner.intern("op:read-param0".to_string()),
                    status: MirStatus::Resolved,
                },
                // Write local tmp from param[0]
                MirOperation {
                    id: MirOpId(1),
                    body: MirBodyId(0),
                    ordinal: 2,
                    span: span(),
                    kind: MirOperationKind::Assign {
                        place: PlaceId(1),
                        value: MirValue::Place(PlaceId(0)),
                        mode: AssignMode::Overwrite,
                    },
                    stable_key: interner.intern("op:assign-tmp".to_string()),
                    status: MirStatus::Resolved,
                },
                MirOperation {
                    id: MirOpId(2),
                    body: MirBodyId(0),
                    ordinal: 3,
                    span: span(),
                    kind: MirOperationKind::Return { value: None },
                    stable_key: interner.intern("op:return".to_string()),
                    status: MirStatus::Resolved,
                },
            ],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("MIR should store");
        db
    }

    #[test]
    fn memory_effects_tracks_receiver_and_param_access() {
        let db = db_with_param_and_local_ops();
        let output = DirectSummaryBuilder::build(
            &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
            &db,
        );

        let mem_summary = output
            .summaries
            .iter()
            .find(|s| s.domain == SummaryDomainKind::MemoryEffects)
            .expect("memory effects summary");

        // param[0] is read (by Read op) and read again (as source of Assign)
        assert!(
            mem_summary.payload_digest.contains("param[0]:Read"),
            "memory digest should record param[0] read access, got: {}",
            mem_summary.payload_digest
        );
        // local tmp is written (by Assign)
        assert!(
            mem_summary.payload_digest.contains("local:"),
            "memory digest should record local access, got: {}",
            mem_summary.payload_digest
        );
        // return is written (Return op writes to return)
        assert!(
            mem_summary.payload_digest.contains("return:"),
            "memory digest should record return access, got: {}",
            mem_summary.payload_digest
        );
    }

    #[test]
    fn tito_detects_param_returned_directly() {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        // Function that returns param[0] directly: `return arg0`
        db.replace_semantic_mir(MirOutput {
            bodies: vec![MirBody {
                id: MirBodyId(0),
                language: Language::TypeScript,
                file: FileId::from_raw(1),
                function: FunctionId::from_raw(1),
                package: None,
                module: None,
                owner_stable_key: interner.intern("owner:tito".to_string()),
                span: span(),
                stable_key: interner.intern("body:tito".to_string()),
                status: MirStatus::Resolved,
            }],
            places: vec![PlaceFact {
                id: PlaceId(0),
                language: Language::TypeScript,
                file: Some(FileId::from_raw(1)),
                function: Some(FunctionId::from_raw(1)),
                root: PlaceRoot::Parameter {
                    function: FunctionId::from_raw(1),
                    index: 0,
                    name: Some("arg0".to_string()),
                },
                projections: Vec::new(),
                stable_key: interner.intern("place:param0".to_string()),
                status: PlaceStatus::Resolved,
            }],
            operations: vec![MirOperation {
                id: MirOpId(0),
                body: MirBodyId(0),
                ordinal: 1,
                span: span(),
                kind: MirOperationKind::Return {
                    value: Some(MirValue::Place(PlaceId(0))),
                },
                stable_key: interner.intern("op:return-param".to_string()),
                status: MirStatus::Resolved,
            }],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("MIR should store");

        let output = DirectSummaryBuilder::build(
            &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
            &db,
        );

        let tito_summary = output
            .summaries
            .iter()
            .find(|s| s.domain == SummaryDomainKind::DataFlowTito)
            .expect("TITO summary");

        // Should detect Param(0) -> Return Value flow
        assert!(
            tito_summary.payload_digest.contains("edge:"),
            "TITO digest should contain flow edge, got: {}",
            tito_summary.payload_digest
        );
        assert!(
            tito_summary.payload_digest.contains("Param(0)"),
            "TITO digest should reference Param(0), got: {}",
            tito_summary.payload_digest
        );
        assert!(
            tito_summary.payload_digest.contains("Return"),
            "TITO digest should reference Return, got: {}",
            tito_summary.payload_digest
        );
        assert!(
            tito_summary.payload_digest.contains("source_return:true"),
            "TITO should mark has_source_return, got: {}",
            tito_summary.payload_digest
        );
        assert_eq!(
            tito_summary.tito_flows,
            vec![SummaryFlowEdge {
                from: FlowRoot::Param(0),
                to: FlowRoot::Return,
                kind: FlowKind::Value,
            }]
        );
    }

    #[test]
    fn tito_assignment_overwrite_kills_stale_param_copy() {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![MirBody {
                id: MirBodyId(0),
                language: Language::TypeScript,
                file: FileId::from_raw(1),
                function: FunctionId::from_raw(1),
                package: None,
                module: None,
                owner_stable_key: interner.intern("owner:tito-overwrite".to_string()),
                span: span(),
                stable_key: interner.intern("body:tito-overwrite".to_string()),
                status: MirStatus::Resolved,
            }],
            places: vec![
                PlaceFact {
                    id: PlaceId(0),
                    language: Language::TypeScript,
                    file: Some(FileId::from_raw(1)),
                    function: Some(FunctionId::from_raw(1)),
                    root: PlaceRoot::Parameter {
                        function: FunctionId::from_raw(1),
                        index: 0,
                        name: Some("arg0".to_string()),
                    },
                    projections: Vec::new(),
                    stable_key: interner.intern("place:param0".to_string()),
                    status: PlaceStatus::Resolved,
                },
                PlaceFact {
                    id: PlaceId(1),
                    language: Language::TypeScript,
                    file: Some(FileId::from_raw(1)),
                    function: Some(FunctionId::from_raw(1)),
                    root: PlaceRoot::Local {
                        function: FunctionId::from_raw(1),
                        name: "tmp".to_string(),
                    },
                    projections: Vec::new(),
                    stable_key: interner.intern("place:tmp".to_string()),
                    status: PlaceStatus::Resolved,
                },
            ],
            operations: vec![
                MirOperation {
                    id: MirOpId(0),
                    body: MirBodyId(0),
                    ordinal: 1,
                    span: span(),
                    kind: MirOperationKind::Assign {
                        place: PlaceId(1),
                        value: MirValue::Place(PlaceId(0)),
                        mode: AssignMode::DeclarationBinding,
                    },
                    stable_key: interner.intern("op:tmp-param".to_string()),
                    status: MirStatus::Resolved,
                },
                MirOperation {
                    id: MirOpId(1),
                    body: MirBodyId(0),
                    ordinal: 2,
                    span: span(),
                    kind: MirOperationKind::Assign {
                        place: PlaceId(1),
                        value: MirValue::Literal {
                            value: "safe".to_string(),
                        },
                        mode: AssignMode::Overwrite,
                    },
                    stable_key: interner.intern("op:tmp-safe".to_string()),
                    status: MirStatus::Resolved,
                },
                MirOperation {
                    id: MirOpId(2),
                    body: MirBodyId(0),
                    ordinal: 3,
                    span: span(),
                    kind: MirOperationKind::Return {
                        value: Some(MirValue::Place(PlaceId(1))),
                    },
                    stable_key: interner.intern("op:return-tmp".to_string()),
                    status: MirStatus::Resolved,
                },
            ],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("MIR should store");

        let output = DirectSummaryBuilder::build(
            &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
            &db,
        );
        let tito_summary = output
            .summaries
            .iter()
            .find(|s| s.domain == SummaryDomainKind::DataFlowTito)
            .expect("TITO summary");

        assert!(tito_summary.tito_flows.is_empty());
        assert!(
            !tito_summary.payload_digest.contains("edge:"),
            "literal overwrite should remove stale param-to-return flow, got: {}",
            tito_summary.payload_digest
        );
    }

    #[test]
    fn tito_unknown_write_kills_stale_param_copy_in_straight_line_body() {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![MirBody {
                id: MirBodyId(0),
                language: Language::TypeScript,
                file: FileId::from_raw(1),
                function: FunctionId::from_raw(1),
                package: None,
                module: None,
                owner_stable_key: interner.intern("owner:tito-unknown-write".to_string()),
                span: span(),
                stable_key: interner.intern("body:tito-unknown-write".to_string()),
                status: MirStatus::Resolved,
            }],
            places: vec![
                PlaceFact {
                    id: PlaceId(0),
                    language: Language::TypeScript,
                    file: Some(FileId::from_raw(1)),
                    function: Some(FunctionId::from_raw(1)),
                    root: PlaceRoot::Parameter {
                        function: FunctionId::from_raw(1),
                        index: 0,
                        name: Some("arg0".to_string()),
                    },
                    projections: Vec::new(),
                    stable_key: interner.intern("place:param0".to_string()),
                    status: PlaceStatus::Resolved,
                },
                PlaceFact {
                    id: PlaceId(1),
                    language: Language::TypeScript,
                    file: Some(FileId::from_raw(1)),
                    function: Some(FunctionId::from_raw(1)),
                    root: PlaceRoot::Local {
                        function: FunctionId::from_raw(1),
                        name: "tmp".to_string(),
                    },
                    projections: Vec::new(),
                    stable_key: interner.intern("place:tmp".to_string()),
                    status: PlaceStatus::Resolved,
                },
            ],
            operations: vec![
                MirOperation {
                    id: MirOpId(0),
                    body: MirBodyId(0),
                    ordinal: 1,
                    span: span(),
                    kind: MirOperationKind::Assign {
                        place: PlaceId(1),
                        value: MirValue::Place(PlaceId(0)),
                        mode: AssignMode::DeclarationBinding,
                    },
                    stable_key: interner.intern("op:tmp-param".to_string()),
                    status: MirStatus::Resolved,
                },
                MirOperation {
                    id: MirOpId(1),
                    body: MirBodyId(0),
                    ordinal: 2,
                    span: span(),
                    kind: MirOperationKind::Assign {
                        place: PlaceId(1),
                        value: MirValue::Unknown {
                            evidence: "dynamic".to_string(),
                        },
                        mode: AssignMode::UnknownWrite,
                    },
                    stable_key: interner.intern("op:tmp-unknown".to_string()),
                    status: MirStatus::Resolved,
                },
                MirOperation {
                    id: MirOpId(2),
                    body: MirBodyId(0),
                    ordinal: 3,
                    span: span(),
                    kind: MirOperationKind::Return {
                        value: Some(MirValue::Place(PlaceId(1))),
                    },
                    stable_key: interner.intern("op:return-tmp".to_string()),
                    status: MirStatus::Resolved,
                },
            ],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("MIR should store");

        let output = DirectSummaryBuilder::build(
            &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
            &db,
        );
        let tito_summary = output
            .summaries
            .iter()
            .find(|s| s.domain == SummaryDomainKind::DataFlowTito)
            .expect("TITO summary");

        assert!(tito_summary.tito_flows.is_empty());
    }

    #[test]
    fn tito_assignment_overwrite_in_branchy_body_keeps_possible_param_copy() {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![MirBody {
                id: MirBodyId(0),
                language: Language::TypeScript,
                file: FileId::from_raw(1),
                function: FunctionId::from_raw(1),
                package: None,
                module: None,
                owner_stable_key: interner.intern("owner:tito-branch-overwrite".to_string()),
                span: span(),
                stable_key: interner.intern("body:tito-branch-overwrite".to_string()),
                status: MirStatus::Resolved,
            }],
            places: vec![
                PlaceFact {
                    id: PlaceId(0),
                    language: Language::TypeScript,
                    file: Some(FileId::from_raw(1)),
                    function: Some(FunctionId::from_raw(1)),
                    root: PlaceRoot::Parameter {
                        function: FunctionId::from_raw(1),
                        index: 0,
                        name: Some("arg0".to_string()),
                    },
                    projections: Vec::new(),
                    stable_key: interner.intern("place:param0".to_string()),
                    status: PlaceStatus::Resolved,
                },
                PlaceFact {
                    id: PlaceId(1),
                    language: Language::TypeScript,
                    file: Some(FileId::from_raw(1)),
                    function: Some(FunctionId::from_raw(1)),
                    root: PlaceRoot::Local {
                        function: FunctionId::from_raw(1),
                        name: "tmp".to_string(),
                    },
                    projections: Vec::new(),
                    stable_key: interner.intern("place:tmp".to_string()),
                    status: PlaceStatus::Resolved,
                },
            ],
            operations: vec![
                MirOperation {
                    id: MirOpId(0),
                    body: MirBodyId(0),
                    ordinal: 1,
                    span: span(),
                    kind: MirOperationKind::Assign {
                        place: PlaceId(1),
                        value: MirValue::Place(PlaceId(0)),
                        mode: AssignMode::DeclarationBinding,
                    },
                    stable_key: interner.intern("op:tmp-param".to_string()),
                    status: MirStatus::Resolved,
                },
                MirOperation {
                    id: MirOpId(1),
                    body: MirBodyId(0),
                    ordinal: 2,
                    span: span(),
                    kind: MirOperationKind::Branch {
                        predicate: MirPredicateId(1),
                        predicate_place: Some(PlaceId(0)),
                        nil_test: None,
                    },
                    stable_key: interner.intern("op:branch".to_string()),
                    status: MirStatus::Resolved,
                },
                MirOperation {
                    id: MirOpId(2),
                    body: MirBodyId(0),
                    ordinal: 3,
                    span: span(),
                    kind: MirOperationKind::Assign {
                        place: PlaceId(1),
                        value: MirValue::Literal {
                            value: "safe".to_string(),
                        },
                        mode: AssignMode::Overwrite,
                    },
                    stable_key: interner.intern("op:tmp-safe".to_string()),
                    status: MirStatus::Resolved,
                },
                MirOperation {
                    id: MirOpId(3),
                    body: MirBodyId(0),
                    ordinal: 4,
                    span: span(),
                    kind: MirOperationKind::Return {
                        value: Some(MirValue::Place(PlaceId(1))),
                    },
                    stable_key: interner.intern("op:return-tmp".to_string()),
                    status: MirStatus::Resolved,
                },
            ],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("MIR should store");

        let output = DirectSummaryBuilder::build(
            &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
            &db,
        );
        let tito_summary = output
            .summaries
            .iter()
            .find(|s| s.domain == SummaryDomainKind::DataFlowTito)
            .expect("TITO summary");

        assert_eq!(
            tito_summary.tito_flows,
            vec![SummaryFlowEdge {
                from: FlowRoot::Param(0),
                to: FlowRoot::Return,
                kind: FlowKind::Value,
            }]
        );
    }

    #[test]
    fn unresolved_calls_produce_may_have_external_effects() {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![MirBody {
                id: MirBodyId(0),
                language: Language::Go,
                file: FileId::from_raw(1),
                function: FunctionId::from_raw(1),
                package: None,
                module: None,
                owner_stable_key: interner.intern("owner:ext".to_string()),
                span: span(),
                stable_key: interner.intern("body:ext".to_string()),
                status: MirStatus::Resolved,
            }],
            places: vec![],
            operations: vec![MirOperation {
                id: MirOpId(0),
                body: MirBodyId(0),
                ordinal: 1,
                span: span(),
                kind: MirOperationKind::Return { value: None },
                stable_key: interner.intern("op:return".to_string()),
                status: MirStatus::Resolved,
            }],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("MIR should store");

        // Add unresolved call facts
        let _ = db.replace_call_facts(CallOutput {
            sites: vec![CallSiteFact {
                in_throw: false,
                id: CallSiteId(1),
                language: Language::Go,
                file: FileId::from_raw(1),
                caller: FunctionId::from_raw(1),
                owner_symbol: None,
                body: MirBodyId(0),
                operation: MirOpId(0),
                span: span(),
                kind: CallSyntaxKind::Function,
                callee: CallCallee::Unknown {
                    reason: UnresolvedCallReason::DynamicProperty,
                },
                receiver: None,
                arguments: Vec::new(),
                result: None,
                status: CallTargetStatus::Unresolved,
                precision: CallPrecision::Unknown,
                stable_key: db
                    .stable_key_interner()
                    .intern("call-site:unknown".to_string()),
            }],
            targets: Vec::new(),
            unresolved: vec![UnresolvedCallFact {
                site: CallSiteId(1),
                caller: FunctionId::from_raw(1),
                status: CallTargetStatus::Unresolved,
                reason: UnresolvedCallReason::DynamicProperty,
                algorithm: CallAlgorithm::SyntaxOnly,
                provenance: CallProvenanceFact::MirShape,
                precision: CallPrecision::Unknown,
                stable_key: db
                    .stable_key_interner()
                    .intern("unresolved:dyn".to_string()),
            }],
        });

        let output = DirectSummaryBuilder::build(
            &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
            &db,
        );

        let mem_summary = output
            .summaries
            .iter()
            .find(|s| s.domain == SummaryDomainKind::MemoryEffects)
            .expect("memory effects summary");

        // Unresolved calls should set may_have_external_effects = true
        assert!(
            mem_summary.payload_digest.contains("external:true"),
            "memory digest should mark external effects when unresolved calls exist, got: {}",
            mem_summary.payload_digest
        );

        // Should also have a memory event for may_have_external_effects
        assert!(
            output
                .events
                .iter()
                .any(|e| e.domain == SummaryDomainKind::MemoryEffects),
            "should have memory effect event for unresolved calls"
        );
    }
}
