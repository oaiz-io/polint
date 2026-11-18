//! RTA dispatch resolver: interface-invoke-by-method-set + func-value-by-signature.
//!
//! [`resolve_callsite`] resolves one `UnresolvedDynamic` Go callsite to candidate
//! callees filtered by the current instantiated runtime-type set. Each result is a
//! [`DerivedEdgeFact`] from caller function node to callee function node in the unified
//! vocabulary. Edges reuse the shared `points_to` status and precision vocabulary.
//! [`DerivedEdgeProvenance`] records every contributing callsite, dispatch, method-set,
//! and instantiated-type fact so deleting a contributor invalidates the edge. Dynamic
//! dispatch is an over-approximation, so RTA edges are never marked exact and remain
//! subject to the store's `derived_edge_precision_ceiling`.

use std::collections::BTreeSet;

use crate::analysis_api::{FactFamily, stable_key_from_parts};
use crate::analysis_neutral::ids::DerivedEdgeId;
use crate::analysis_neutral::points_to::facts::{PointsToPrecision, PointsToStatus};
use crate::analysis_neutral::semantic_graph::constraints::ConstraintKind;
use crate::analysis_neutral::solver::engine::{weakest_precision, weakest_status};
use crate::analysis_neutral::solver::facts::DerivedEdgeFact;
use crate::analysis_neutral::solver::provenance::{ContributingFact, DerivedEdgeProvenance};
use crate::internal_core::StableKeyId;

use super::snapshot::{RtaCallsite, RtaInputs};

/// Outcome of resolving one callsite: the (possibly empty) resolved edges and whether
/// the per-callsite candidate cap was exceeded (a run-level exhaustion signal).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CallsiteResolution {
    pub(crate) edges: Vec<DerivedEdgeFact>,
    pub(crate) candidate_cap_exceeded: bool,
}

/// Resolve one dynamic callsite to its candidate-callee edges under the current
/// instantiated-type / address-taken sets.
///
/// - Interface invoke (`interface_method` set): candidate callees are the concrete
///   methods whose receiver type is instantiated AND whose method-set contains the
///   invoked method. The instantiated-type filter is the RTA discriminant — it is baked
///   into [`RtaInputs::interface_candidate_index`] at build time, so this hot
///   path is a single index lookup rather than a per-callsite instantiated-set scan.
/// - Func-value call (`signature` set): candidate callees are address-taken functions
///   whose signature matches.
///
/// A callsite whose interface type or method has no
/// method-set match — or whose caller/callee has no semantic node — contributes NO
/// edge; it stays an unresolved obligation rather than a fabricated edge. When the
/// candidate set would exceed `max_candidates_per_callsite`, resolution stops and
/// `candidate_cap_exceeded` latches (edges resolved before the cap keep their honest
/// status).
///
/// `resolve_func_values` gates ONLY the func-value (signature) path: when the
/// address-taken surface exceeds `address_taken_threshold` the caller disables func-value
/// resolution, but interface-invoke resolution (which never consults the address-taken
/// set) still proceeds — a func-value callsite then resolves to nothing (honest
/// unresolved), an interface callsite is unaffected.
pub(crate) fn resolve_callsite(
    interner: &crate::internal_core::StableKeyInterner,
    callsite: &RtaCallsite,
    inputs: &RtaInputs,
    address_taken: &BTreeSet<String>,
    max_candidates_per_callsite: usize,
    resolve_func_values: bool,
    solver_step: u64,
) -> CallsiteResolution {
    // The edge SOURCE is the caller FUNCTION node, not the callsite node; the
    // callsite node is recorded via the producing CallConstraint in provenance. No
    // caller node => no honest edge anchor.
    let Some(&caller_node) = inputs.function_node.get(&callsite.caller) else {
        return CallsiteResolution::default();
    };

    // Gather candidate callees as (callee_node, callee_qualified, extra contributing
    // fact keys) — deterministic, deduplicated by callee qualified identity.
    let mut candidates: Vec<DispatchCandidate> = Vec::new();
    let mut candidate_cap_exceeded = false;

    if let Some(method) = callsite.interface_method.as_deref() {
        collect_interface_candidates(
            method,
            inputs,
            max_candidates_per_callsite,
            &mut candidates,
            &mut candidate_cap_exceeded,
        );
    } else if let Some(signature) = callsite.signature.as_deref() {
        // Func-value resolution is suppressed when the address-taken surface is too large
        // while the interface path above is never gated.
        if resolve_func_values {
            collect_func_value_candidates(
                signature,
                inputs,
                address_taken,
                max_candidates_per_callsite,
                &mut candidates,
                &mut candidate_cap_exceeded,
            );
        }
    }

    let mut edges = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        // Worst-trust status/precision across the adopted derivation. The
        // contributing frontend facts are all "present", so status is Present; an
        // RTA-resolved dynamic edge is an over-approximation, so precision is floored
        // at Heuristic via the worst-of combine (never exact).
        let status = weakest_status(PointsToStatus::Present, PointsToStatus::Present);
        let precision = weakest_precision(
            PointsToPrecision::Heuristic,
            PointsToPrecision::FlowInsensitive,
        );

        // Contributing facts: the callsite + dispatch detail (always) + whatever
        // method-set / instantiated-type / address-taken facts justified THIS callee
        // (so deleting any of them does not reproduce the edge).
        let mut contributing: Vec<ContributingFact> = vec![
            ContributingFact {
                stable_key: callsite.callsite_stable_key,
            },
            ContributingFact {
                stable_key: callsite.dispatch_stable_key,
            },
        ];
        contributing.extend(
            candidate
                .contributing_keys
                .into_iter()
                .map(|stable_key| ContributingFact { stable_key }),
        );

        let provenance = DerivedEdgeProvenance::new(
            interner,
            contributing,
            &ConstraintKind::call_at(callsite.callsite_node),
            solver_step,
        );
        let stable_key = stable_key_from_parts(
            interner,
            FactFamily::SolverDerivedEdge,
            &[
                ("source", caller_node.0.to_string()),
                ("target", candidate.node.0.to_string()),
                ("provenance", provenance.stable_key_fragment(interner)),
            ],
        );
        edges.push(DerivedEdgeFact {
            id: DerivedEdgeId(0),
            source: caller_node,
            target: candidate.node,
            status,
            precision,
            stable_key,
            provenance,
        });
    }

    CallsiteResolution {
        edges,
        candidate_cap_exceeded,
    }
}

/// One resolved candidate callee with the contributing-fact keys that justified it.
struct DispatchCandidate {
    node: crate::analysis_neutral::ids::SemanticNodeId,
    contributing_keys: Vec<StableKeyId>,
}

/// Interface-invoke candidates: concrete methods named `method` whose receiver type
/// is instantiated AND whose method-set contains `method`. The instantiated
/// filter is what makes it RTA. Deterministic by `(type_name, callee qualified)`.
///
/// Resolved via the pre-built inverted index
/// [`RtaInputs::interface_candidate_index`] — a single `BTreeMap` lookup — instead of
/// re-scanning the WHOLE instantiated set per callsite. The index was built by iterating
/// `instantiated` (sorted) then `methods_by_receiver[type]` (in order), so `index[method]`
/// is byte-identical to the reference whole-set scan for this `method`: the same
/// candidate set, same order, same per-candidate `contributing_keys`. The per-callsite
/// cap is applied to the SAME prefix, so the cap-exceeded signal and the emitted edges
/// (and their stable keys) are unchanged. The whole reachable program's total interface
/// resolution drops from O(C_iface · T) to O(C_iface · resolved-candidates).
fn collect_interface_candidates(
    method: &str,
    inputs: &RtaInputs,
    max_candidates_per_callsite: usize,
    candidates: &mut Vec<DispatchCandidate>,
    candidate_cap_exceeded: &mut bool,
) {
    let Some(indexed) = inputs.interface_candidate_index.get(method) else {
        return;
    };
    // The index Vec is already in the scan's `(instantiated-sorted, then within-type)`
    // order, so applying the cap to this prefix preserves reference behavior exactly.
    for candidate in indexed {
        if candidates.len() >= max_candidates_per_callsite {
            *candidate_cap_exceeded = true;
            return;
        }
        candidates.push(DispatchCandidate {
            node: candidate.node,
            contributing_keys: candidate.contributing_keys.clone(),
        });
    }
}

/// Func-value candidates: address-taken functions whose signature matches the call
/// signature. Deterministic by address-taken qualified identity.
fn collect_func_value_candidates(
    signature: &str,
    inputs: &RtaInputs,
    address_taken: &BTreeSet<String>,
    max_candidates_per_callsite: usize,
    candidates: &mut Vec<DispatchCandidate>,
    candidate_cap_exceeded: &mut bool,
) {
    for function in address_taken {
        let Some(&node) = inputs.function_node.get(function) else {
            continue;
        };
        // Signature match: the address-taken function's signature must equal the call
        // signature. Functions with no recorded signature cannot honestly match.
        let Some(function_signature) = inputs.function_signature.get(function) else {
            continue;
        };
        if function_signature != signature {
            continue;
        }
        if candidates.len() >= max_candidates_per_callsite {
            *candidate_cap_exceeded = true;
            return;
        }
        let mut contributing_keys = Vec::new();
        if let Some(key) = inputs.address_taken_keys.get(function) {
            contributing_keys.push(*key);
        }
        candidates.push(DispatchCandidate {
            node,
            contributing_keys,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    use super::super::snapshot::{RtaCallsite, RtaInputs, RtaMethod};
    use crate::analysis_neutral::ids::SemanticNodeId;

    /// Two instantiated types both declare `Read` → two candidate callees. A
    /// per-callsite cap of 1 latches `candidate_cap_exceeded` and emits only the
    /// pre-cap edge (edges resolved before the cap are honest).
    #[test]
    fn per_callsite_candidate_cap_latches_and_keeps_precap_edges() {
        let interner = crate::internal_core::test_stable_key_interner();
        let mut function_node = BTreeMap::new();
        function_node.insert("main.main".to_string(), SemanticNodeId(1));
        function_node.insert("(pkg.A).Read".to_string(), SemanticNodeId(3));
        function_node.insert("(pkg.B).Read".to_string(), SemanticNodeId(4));

        let mut method_sets = BTreeMap::new();
        method_sets.insert("pkg.A".to_string(), BTreeSet::from(["Read".to_string()]));
        method_sets.insert("pkg.B".to_string(), BTreeSet::from(["Read".to_string()]));
        let mut method_set_keys = BTreeMap::new();
        method_set_keys.insert("pkg.A".to_string(), interner.intern("ms|A"));
        method_set_keys.insert("pkg.B".to_string(), interner.intern("ms|B"));

        let mut methods_by_receiver = BTreeMap::new();
        methods_by_receiver.insert(
            "pkg.A".to_string(),
            vec![RtaMethod {
                method_name: "Read".to_string(),
                qualified: "(pkg.A).Read".to_string(),
                node: SemanticNodeId(3),
            }],
        );
        methods_by_receiver.insert(
            "pkg.B".to_string(),
            vec![RtaMethod {
                method_name: "Read".to_string(),
                qualified: "(pkg.B).Read".to_string(),
                node: SemanticNodeId(4),
            }],
        );

        let mut instantiated_keys = BTreeMap::new();
        instantiated_keys.insert("pkg.A".to_string(), interner.intern("inst|A"));
        instantiated_keys.insert("pkg.B".to_string(), interner.intern("inst|B"));

        let inputs = RtaInputs {
            method_sets,
            method_set_keys,
            instantiated: BTreeSet::from(["pkg.A".to_string(), "pkg.B".to_string()]),
            instantiated_keys,
            methods_by_receiver,
            function_node,
            ..RtaInputs::default()
        }
        .finalize_indexes();
        let callsite = RtaCallsite {
            caller: "main.main".to_string(),
            callsite_node: SemanticNodeId(2),
            callsite_stable_key: interner.intern("cs|main"),
            interface_method: Some("Read".to_string()),
            signature: None,
            dispatch_stable_key: interner.intern("dd|main"),
        };

        let resolution = resolve_callsite(
            &crate::internal_core::test_stable_key_interner(),
            &callsite,
            &inputs,
            &BTreeSet::new(),
            1,
            true,
            7,
        );
        assert!(resolution.candidate_cap_exceeded, "cap of 1 must latch");
        assert_eq!(
            resolution.edges.len(),
            1,
            "only the pre-cap edge is emitted; it keeps its honest status"
        );
        assert!(resolution.edges[0].honors_precision_ceiling());
    }

    /// No caller node => no honest edge anchor (the source endpoint is missing).
    #[test]
    fn missing_caller_node_resolves_nothing() {
        let interner = crate::internal_core::test_stable_key_interner();
        let inputs = RtaInputs::default();
        let callsite = RtaCallsite {
            caller: "main.main".to_string(),
            callsite_node: SemanticNodeId(2),
            callsite_stable_key: interner.intern("cs|main"),
            interface_method: Some("Read".to_string()),
            signature: None,
            dispatch_stable_key: interner.intern("dd|main"),
        };
        let resolution = resolve_callsite(
            &crate::internal_core::test_stable_key_interner(),
            &callsite,
            &inputs,
            &BTreeSet::new(),
            128,
            true,
            1,
        );
        assert!(resolution.edges.is_empty());
        assert!(!resolution.candidate_cap_exceeded);
    }

    /// With `resolve_func_values = false`, a func-value (signature) callsite
    /// resolves to NOTHING even though a signature-matching address-taken function exists
    /// — the func-value path is gated. An interface callsite (separate test) is unaffected.
    #[test]
    fn func_value_resolution_is_suppressed_when_disabled() {
        let interner = crate::internal_core::test_stable_key_interner();
        let mut function_node = BTreeMap::new();
        function_node.insert("main.main".to_string(), SemanticNodeId(1));
        function_node.insert("pkg.handler".to_string(), SemanticNodeId(3));
        let mut function_signature = BTreeMap::new();
        function_signature.insert("pkg.handler".to_string(), "func()".to_string());
        let address_taken = BTreeSet::from(["pkg.handler".to_string()]);
        let mut address_taken_keys = BTreeMap::new();
        address_taken_keys.insert("pkg.handler".to_string(), interner.intern("at|handler"));

        let inputs = RtaInputs {
            function_node,
            function_signature,
            address_taken: address_taken.clone(),
            address_taken_keys,
            ..RtaInputs::default()
        }
        .finalize_indexes();
        let callsite = RtaCallsite {
            caller: "main.main".to_string(),
            callsite_node: SemanticNodeId(2),
            callsite_stable_key: interner.intern("cs|main:fv"),
            interface_method: None,
            signature: Some("func()".to_string()),
            dispatch_stable_key: interner.intern("dd|main:fv"),
        };

        // Disabled: no edge (the signature-matching handler is NOT resolved).
        let disabled = resolve_callsite(
            &crate::internal_core::test_stable_key_interner(),
            &callsite,
            &inputs,
            &address_taken,
            128,
            false,
            1,
        );
        assert!(
            disabled.edges.is_empty(),
            "func-value resolution must yield nothing when disabled: {:#?}",
            disabled.edges
        );
        // Enabled: the same callsite resolves to the address-taken handler (control).
        let enabled = resolve_callsite(
            &crate::internal_core::test_stable_key_interner(),
            &callsite,
            &inputs,
            &address_taken,
            128,
            true,
            1,
        );
        assert!(
            enabled.edges.iter().any(|e| e.target == SemanticNodeId(3)),
            "func-value resolution must work when enabled (control)"
        );
    }

    /// Reference oracle: the whole-instantiated-set scan, reproduced verbatim so
    /// the test can prove the indexed path is byte-identical to it. Iterates `instantiated`
    /// (sorted BTreeSet) then `methods_by_receiver[type]` (in order), pushing every concrete
    /// method named `method` whose type's method-set contains `method`, capping the same way.
    fn collect_interface_candidates_via_whole_scan(
        method: &str,
        inputs: &RtaInputs,
        max_candidates_per_callsite: usize,
    ) -> (
        Vec<(SemanticNodeId, Vec<crate::internal_core::StableKeyId>)>,
        bool,
    ) {
        let mut candidates = Vec::new();
        let mut capped = false;
        for type_name in &inputs.instantiated {
            let Some(methods) = inputs.method_sets.get(type_name) else {
                continue;
            };
            if !methods.contains(method) {
                continue;
            }
            let Some(concrete_methods) = inputs.methods_by_receiver.get(type_name) else {
                continue;
            };
            for concrete in concrete_methods {
                if concrete.method_name != method {
                    continue;
                }
                if candidates.len() >= max_candidates_per_callsite {
                    capped = true;
                    return (candidates, capped);
                }
                let mut contributing_keys = Vec::new();
                if let Some(key) = inputs.method_set_keys.get(type_name) {
                    contributing_keys.push(*key);
                }
                if let Some(key) = inputs.instantiated_keys.get(type_name) {
                    contributing_keys.push(*key);
                }
                candidates.push((concrete.node, contributing_keys));
            }
        }
        (candidates, capped)
    }

    /// The indexed `collect_interface_candidates` produces the SAME candidate set,
    /// ORDER, and per-candidate contributing keys as the whole-instantiated-set scan it
    /// models, and the per-callsite cap selects the same prefix. Three instantiated types
    /// share the method `Speak`; a fourth type declares `Speak` in its method-set but is NOT
    /// instantiated (it must be excluded); a fifth instantiated type lacks `Speak` from its
    /// set (also excluded). Byte-identity is proven by comparing against the reference oracle.
    #[test]
    fn indexed_interface_candidates_match_whole_set_scan_and_cap_prefix() {
        let interner = crate::internal_core::test_stable_key_interner();
        let node = |id: u64| SemanticNodeId(id);

        // Three instantiated implementers of `Speak`, one declared-but-not-instantiated
        // implementer (Zebra), and one instantiated type whose method-set lacks `Speak`
        // (Mute, which only has `Hush`). Method-set / instantiated keys are distinct per
        // type so the contributing-key comparison is meaningful.
        let method_sets = BTreeMap::from([
            ("pkg.Cat".to_string(), BTreeSet::from(["Speak".to_string()])),
            ("pkg.Dog".to_string(), BTreeSet::from(["Speak".to_string()])),
            ("pkg.Fox".to_string(), BTreeSet::from(["Speak".to_string()])),
            (
                "pkg.Zebra".to_string(),
                BTreeSet::from(["Speak".to_string()]),
            ),
            ("pkg.Mute".to_string(), BTreeSet::from(["Hush".to_string()])),
        ]);
        let method_set_keys = BTreeMap::from([
            ("pkg.Cat".to_string(), interner.intern("ms|Cat")),
            ("pkg.Dog".to_string(), interner.intern("ms|Dog")),
            ("pkg.Fox".to_string(), interner.intern("ms|Fox")),
            ("pkg.Zebra".to_string(), interner.intern("ms|Zebra")),
            ("pkg.Mute".to_string(), interner.intern("ms|Mute")),
        ]);
        let method = |receiver: &str, n: &str, id: u64| RtaMethod {
            method_name: n.to_string(),
            qualified: format!("({receiver}).{n}"),
            node: node(id),
        };
        let methods_by_receiver = BTreeMap::from([
            ("pkg.Cat".to_string(), vec![method("pkg.Cat", "Speak", 11)]),
            ("pkg.Dog".to_string(), vec![method("pkg.Dog", "Speak", 12)]),
            ("pkg.Fox".to_string(), vec![method("pkg.Fox", "Speak", 13)]),
            (
                "pkg.Zebra".to_string(),
                vec![method("pkg.Zebra", "Speak", 14)],
            ),
            ("pkg.Mute".to_string(), vec![method("pkg.Mute", "Hush", 15)]),
        ]);
        // Zebra is NOT instantiated; Mute IS but lacks `Speak`.
        let instantiated = BTreeSet::from([
            "pkg.Cat".to_string(),
            "pkg.Dog".to_string(),
            "pkg.Fox".to_string(),
            "pkg.Mute".to_string(),
        ]);
        let instantiated_keys = BTreeMap::from([
            ("pkg.Cat".to_string(), interner.intern("inst|Cat")),
            ("pkg.Dog".to_string(), interner.intern("inst|Dog")),
            ("pkg.Fox".to_string(), interner.intern("inst|Fox")),
            ("pkg.Mute".to_string(), interner.intern("inst|Mute")),
        ]);

        let inputs = RtaInputs {
            method_sets,
            method_set_keys,
            instantiated,
            instantiated_keys,
            methods_by_receiver,
            ..RtaInputs::default()
        }
        .finalize_indexes();

        // For every relevant cap (uncapped, and each prefix length), the indexed path equals
        // the whole-set scan element-for-element (nodes, contributing keys, cap flag).
        for cap in [usize::MAX, 0, 1, 2, 3, 4] {
            let (expected, expected_capped) =
                collect_interface_candidates_via_whole_scan("Speak", &inputs, cap);

            let mut indexed = Vec::new();
            let mut indexed_capped = false;
            collect_interface_candidates("Speak", &inputs, cap, &mut indexed, &mut indexed_capped);
            let indexed_pairs: Vec<(SemanticNodeId, Vec<crate::internal_core::StableKeyId>)> =
                indexed
                    .into_iter()
                    .map(|candidate| (candidate.node, candidate.contributing_keys))
                    .collect();

            assert_eq!(
                indexed_pairs, expected,
                "indexed candidates must equal the whole-set scan (cap {cap})"
            );
            assert_eq!(
                indexed_capped, expected_capped,
                "cap-exceeded flag must match the whole-set scan (cap {cap})"
            );
        }

        // Concretely: uncapped resolves exactly Cat(11), Dog(12), Fox(13) in sorted-type
        // order — Zebra (not instantiated) and Mute's Hush (wrong method) are excluded.
        let mut all = Vec::new();
        let mut capped = false;
        collect_interface_candidates("Speak", &inputs, usize::MAX, &mut all, &mut capped);
        assert_eq!(
            all.iter().map(|c| c.node).collect::<Vec<_>>(),
            vec![node(11), node(12), node(13)],
            "exactly the three instantiated `Speak` implementers, in sorted-type order"
        );
        assert!(!capped);
        // The not-instantiated Zebra method node (14) is never a candidate (RTA filter).
        assert!(
            all.iter().all(|c| c.node != node(14)),
            "a declared-but-not-instantiated implementer must be excluded"
        );
        // A cap of 2 selects the SAME first-two prefix (Cat, Dog) and latches.
        let mut prefix = Vec::new();
        let mut prefix_capped = false;
        collect_interface_candidates("Speak", &inputs, 2, &mut prefix, &mut prefix_capped);
        assert_eq!(
            prefix.iter().map(|c| c.node).collect::<Vec<_>>(),
            vec![node(11), node(12)],
            "the cap must select the same prefix the scan would"
        );
        assert!(prefix_capped, "a cap below the candidate count must latch");
    }
}
