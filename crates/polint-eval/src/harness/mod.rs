pub(crate) mod adaptation;
pub(crate) mod adapter;
pub(crate) mod baseline;
pub(crate) mod bench;
#[cfg(test)]
pub(crate) mod capability_probes;
pub(crate) mod competitors;
pub(crate) mod delta;
pub(crate) mod determinism_gate;
pub(crate) mod external;
pub(crate) mod fact_rows_dump;
pub(crate) mod fixtures;
pub(crate) mod gates;
#[cfg(test)]
pub(crate) mod go_rta;
pub(crate) mod markdown;
pub(crate) mod matcher;
pub(crate) mod metrics;
pub(crate) mod model;
pub(crate) mod observed;
pub(crate) mod performance;
pub(crate) mod report;
pub(crate) mod runner;
#[cfg(all(test, feature = "lang-typescript"))]
pub(crate) mod semantic_graph_snapshot;
pub(crate) mod suite;
pub(crate) mod tiers;
#[cfg(test)]
pub(crate) mod ts_object_model;
#[cfg(test)]
pub(crate) mod ts_tokens;

#[cfg(test)]
mod direct_call_rows {
    use crate::eval::matcher::{MatchOutcome, MatcherConfig, match_case};
    use crate::eval::metrics::compute_metrics;
    use crate::eval::model::{
        AssertionMode, ExpectedFact, ExpectedItem, FixtureArea, ObservedFact, ObservedItem,
        ObservedStatus,
    };
    use serde_json::json;

    #[test]
    fn eval_expected_facts_accept_call_families_and_direct_call_area() {
        let families = crate::eval::model::CALL_FACT_FAMILIES.join(",");
        assert!(families.contains("CallSite"));
        assert!(families.contains("CallTarget"));
        assert!(families.contains("UnresolvedCall"));

        let manifest = r#"
schema_version = "polint-eval-fixture-1"
case_id = "direct-call-row-model"
area = "direct-calls"

[repo]
path = "repo"

[[expected]]
fact = { family = "CallSite", stable_key = "call-site:src/app.ts:handler", mode = "partial", producer_id = "polint.calls", precision = "setup_aware", status = "resolved" }

[[expected]]
fact = { family = "CallTarget", stable_key = "call-target:src/app.ts:handler", mode = "partial", producer_id = "polint.calls", precision = "setup_aware", status = "resolved" }

[[expected]]
fact = { family = "UnresolvedCall", stable_key = "call-unresolved:src/app.ts:dynamic", mode = "partial", producer_id = "polint.calls", precision = "unknown", status = "unresolved" }
"#;

        let parsed: crate::eval::fixtures::NativeFixtureManifest =
            toml::from_str(manifest).expect("call expected facts should parse");

        assert_eq!(parsed.area, FixtureArea::DirectCalls);
        assert_eq!(parsed.expected.len(), 3);
    }

    #[test]
    fn observed_call_debug_rows_normalize_to_fact_rows_with_compact_payloads() {
        let debug = json!({
            "calls": {
                "sites": [
                    call_site_row("call-site:resolved", "Resolved"),
                    call_site_row("call-site:ambiguous", "Ambiguous"),
                    call_site_row("call-site:unresolved", "Unresolved"),
                    call_site_row("call-site:unsupported", "Unsupported"),
                    call_site_row("call-site:setup-missing", "SetupMissing")
                ],
                "targets": [{
                    "family": "CallTarget",
                    "stable_key": "call-target:resolved",
                    "producer_id": "polint.calls",
                    "status": "Resolved",
                    "precision": "SetupAware",
                    "path": "src/app.ts",
                    "span": {"start_line": 1, "start_col": 1, "end_line": 1, "end_col": 9},
                    "site_stable_key": "call-site:resolved",
                    "caller_stable_key": "function:handler",
                    "target_function_stable_key": "function:target",
                    "target_symbol_stable_key": "symbol:target",
                    "edge_kind": "Direct",
                    "algorithm": "DirectReference",
                    "reason": null,
                    "provenance": "NativeDirect"
                }],
                "unresolved": [{
                    "family": "UnresolvedCall",
                    "stable_key": "call-unresolved:setup",
                    "producer_id": "polint.calls",
                    "status": "SetupMissing",
                    "precision": "Unsupported",
                    "path": "src/app.ts",
                    "span": {"start_line": 2, "start_col": 1, "end_line": 2, "end_col": 12},
                    "site_stable_key": "call-site:setup-missing",
                    "caller_stable_key": "function:handler",
                    "algorithm": "Unsupported",
                    "reason": "SetupMissing",
                    "provenance": "MirShape"
                }]
            }
        });

        let observed = crate::eval::observed::call_facts_for_test(&debug);
        let call_statuses = observed
            .iter()
            .filter_map(|item| match item {
                ObservedItem::Fact(fact) if fact.family == "CallSite" => fact.status,
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(
            call_statuses,
            [
                ObservedStatus::Ambiguous,
                ObservedStatus::Resolved,
                ObservedStatus::SetupMissing,
                ObservedStatus::Unresolved,
                ObservedStatus::Unsupported,
            ]
            .into_iter()
            .collect()
        );
        assert!(observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.family == "CallTarget"
                    && fact.producer_id.as_deref() == Some("polint.calls")
                    && fact.precision.as_deref() == Some("setup_aware")
                    && fact.status == Some(ObservedStatus::Resolved)
                    && fact.payload.as_deref().is_some_and(|payload| {
                        payload.contains("path=src/app.ts")
                            && payload.contains("span=1:1-1:9")
                            && payload.contains("algorithm=DirectReference")
                            && payload.contains("target_symbol=symbol:target")
                    })
            }
            _ => false,
        }));
    }

    #[test]
    fn observed_call_debug_rows_emit_count_and_index_invariants() {
        let debug = json!({
            "calls": {
                "counts": {
                    "by_language": {"Go": 2, "TypeScript": 3},
                    "by_call_kind": {"Function": 3, "Member": 1, "Constructor": 1},
                    "by_algorithm": {
                        "DirectReference": 2,
                        "ImportBinding": 1,
                        "StaticMember": 1
                    },
                    "by_status": {
                        "Resolved": 4,
                        "Unresolved": 3,
                        "Unsupported": 2,
                        "SetupMissing": 1
                    },
                    "by_unresolved_reason": {
                        "FunctionValue": 1,
                        "DynamicProperty": 1,
                        "Reflection": 1,
                        "GoroutineBoundary": 1,
                        "Eval": 1,
                        "DynamicImport": 1,
                        "CallApplyBind": 1,
                        "SetupMissing": 1
                    },
                    "by_provider": {"polint.calls": 10}
                },
                "index_counts": {
                    "outgoing_by_function": 2,
                    "outgoing_by_symbol": 2,
                    "incoming_by_symbol": 2,
                    "incoming_by_function": 2,
                    "unresolved_by_reason": 4,
                    "unresolved_by_status": 3
                }
            }
        });

        let observed = crate::eval::observed::call_facts_for_test(&debug);
        for required in [
            "direct_calls.counts.by_language.Go.nonzero",
            "direct_calls.counts.by_call_kind.Function.nonzero",
            "direct_calls.counts.by_algorithm.DirectReference.nonzero",
            "direct_calls.counts.by_status.Resolved.nonzero",
            "direct_calls.counts.by_unresolved_reason.Reflection.nonzero",
            "direct_calls.counts.by_provider.polint.calls.nonzero",
            "direct_calls.index_counts.outgoing_by_function.nonzero",
            "direct_calls.index_counts.outgoing_by_symbol.nonzero",
            "direct_calls.index_counts.incoming_by_symbol.nonzero",
            "direct_calls.index_counts.incoming_by_function.nonzero",
            "direct_calls.index_counts.unresolved_by_reason.nonzero",
            "direct_calls.index_counts.unresolved_by_status.nonzero",
        ] {
            assert!(
                observed.iter().any(|item| match item {
                    ObservedItem::Invariant(invariant) => {
                        invariant.name == required && invariant.value == "true"
                    }
                    _ => false,
                }),
                "missing observed call invariant {required}: {observed:#?}"
            );
        }
    }

    #[test]
    fn call_unresolved_unsupported_and_setup_missing_statuses_count_as_unknown_metrics() {
        let expected = [
            expected_call_fact(
                "CallSite",
                "call-site:unresolved",
                ObservedStatus::Unresolved,
            ),
            expected_call_fact(
                "CallSite",
                "call-site:unsupported",
                ObservedStatus::Unsupported,
            ),
            expected_call_fact(
                "UnresolvedCall",
                "call-unresolved:setup",
                ObservedStatus::SetupMissing,
            ),
        ];
        let observed = [
            observed_call_fact(
                "CallSite",
                "call-site:unresolved",
                ObservedStatus::Unresolved,
            ),
            observed_call_fact(
                "CallSite",
                "call-site:unsupported",
                ObservedStatus::Unsupported,
            ),
            observed_call_fact(
                "UnresolvedCall",
                "call-unresolved:setup",
                ObservedStatus::SetupMissing,
            ),
        ];

        let summaries = match_case(&expected, &observed, MatcherConfig::default());
        let metrics = compute_metrics(&summaries);

        assert_eq!(
            summaries
                .iter()
                .map(|summary| summary.outcome)
                .collect::<Vec<_>>(),
            vec![
                MatchOutcome::Unknown,
                MatchOutcome::Unknown,
                MatchOutcome::Unknown,
            ]
        );
        assert_eq!(metrics.unknown_count, 3);
    }

    fn call_site_row(stable_key: &str, status: &str) -> serde_json::Value {
        json!({
            "family": "CallSite",
            "stable_key": stable_key,
            "producer_id": "polint.calls",
            "status": status,
            "precision": "SetupAware",
            "path": "src/app.ts",
            "span": {"start_line": 1, "start_col": 1, "end_line": 1, "end_col": 9},
            "language": "TypeScript",
            "kind": "Function",
            "callee": "identifier:target"
        })
    }

    fn expected_call_fact(family: &str, stable_key: &str, status: ObservedStatus) -> ExpectedItem {
        ExpectedItem::Fact(ExpectedFact {
            family: family.to_string(),
            stable_key: stable_key.to_string(),
            mode: AssertionMode::Exact,
            producer_id: Some("polint.calls".to_string()),
            precision: None,
            status: Some(status),
            false_positive_trap: false,
        })
    }

    fn observed_call_fact(family: &str, stable_key: &str, status: ObservedStatus) -> ObservedItem {
        ObservedItem::Fact(ObservedFact {
            family: family.to_string(),
            stable_key: stable_key.to_string(),
            mode: AssertionMode::Exact,
            producer_id: Some("polint.calls".to_string()),
            provenance: Some("kernel.metadata_debug_json.calls".to_string()),
            precision: None,
            status: Some(status),
            payload: None,
        })
    }
}

#[cfg(test)]
mod refined_call_rows {
    use crate::eval::model::{
        FixtureArea, ObservedItem, ObservedStatus, REFINED_CALL_FACT_FAMILIES,
    };
    use serde_json::json;

    #[test]
    fn eval_expected_facts_accept_refined_call_family_and_area() {
        assert_eq!(REFINED_CALL_FACT_FAMILIES, ["RefinedCallEdge"]);

        let manifest = r#"
schema_version = "polint-eval-fixture-1"
case_id = "refined-call-row-model"
area = "refined-calls"

[repo]
path = "repo"

[[expected]]
fact = { family = "RefinedCallEdge", stable_key = "family=RefinedCallEdge", mode = "partial", producer_id = "polint.refined_calls", precision = "setup_aware", status = "resolved" }
"#;

        let parsed: crate::eval::fixtures::NativeFixtureManifest =
            toml::from_str(manifest).expect("refined call expected facts should parse");

        assert_eq!(parsed.area, FixtureArea::RefinedCalls);
        assert_eq!(parsed.expected.len(), 1);
    }

    #[test]
    fn observed_refined_call_debug_rows_normalize_to_fact_rows_and_invariants() {
        let debug = json!({
            "refined_calls": {
                "edges": [{
                    "family": "RefinedCallEdge",
                    "stable_key": "family=RefinedCallEdge/tier=direct_only",
                    "producer_id": "polint.refined_calls",
                    "status": "resolved",
                    "precision": "setup_aware",
                    "language": "TypeScript",
                    "tier": "direct_only",
                    "edge_kind": "Direct",
                    "algorithm": "direct_reference",
                    "reason": null,
                    "provenance": "native_direct",
                    "site_stable_key": "call-site:callee",
                    "base_target_stable_key": "call-target:callee",
                    "caller_stable_key": "function:caller",
                    "target_function_stable_key": "function:callee",
                    "target_symbol_stable_key": null,
                    "synthetic_target": null
                }],
                "counts": {
                    "total_edges": 1,
                    "direct_edges": 1,
                    "refined_non_direct_edges": 0,
                    "by_language": {"TypeScript": 1},
                    "by_algorithm": {"direct_reference": 1},
                    "by_tier": {"direct_only": 1},
                    "by_status": {"resolved": 1},
                    "by_precision": {"setup_aware": 1},
                    "by_provenance": {"native_direct": 1},
                    "by_reason": {}
                },
                "deltas": {
                    "direct_edges": 1,
                    "refined_edges": 1,
                    "changed_edges": 0,
                    "extension_model_edges": 0,
                    "unresolved_refined_edges": 0,
                    "budget_exceeded_refined_edges": 0
                }
            }
        });

        let observed = crate::eval::observed::refined_call_facts_for_test(&debug);

        assert!(observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.family == "RefinedCallEdge"
                    && fact.status == Some(ObservedStatus::Resolved)
                    && fact.payload.as_deref().is_some_and(|payload| {
                        payload.contains("tier=direct_only")
                            && payload.contains("algorithm=direct_reference")
                    })
            }
            _ => false,
        }));
        assert!(observed.iter().any(|item| match item {
            ObservedItem::Invariant(invariant) => {
                invariant.name == "refined_calls.counts.by_tier.direct_only.nonzero"
                    && invariant.value == "true"
            }
            _ => false,
        }));
    }
}

#[cfg(test)]
mod abstract_domain_rows {
    use crate::eval::matcher::{MatchOutcome, MatcherConfig, match_case};
    use crate::eval::metrics::compute_metrics;
    use crate::eval::model::{
        ABSTRACT_DOMAIN_FACT_FAMILIES, AssertionMode, ExpectedFact, ExpectedItem, FixtureArea,
        ObservedFact, ObservedItem, ObservedStatus,
    };
    use serde_json::json;

    #[test]
    fn eval_expected_facts_accept_abstract_domain_families_and_area() {
        assert_eq!(
            ABSTRACT_DOMAIN_FACT_FAMILIES,
            ["DomainObservation", "DomainEvent"]
        );

        let manifest = r#"
schema_version = "polint-eval-fixture-1"
case_id = "abstract-domain-row-model"
area = "abstract-domains"

[repo]
path = "repo"

[[expected]]
fact = { family = "DomainObservation", stable_key = "domain-observation:reachability", mode = "partial", producer_id = "polint.abstract_domains", precision = "exact_local", status = "present" }

[[expected]]
fact = { family = "DomainEvent", stable_key = "domain-event:budget", mode = "partial", producer_id = "polint.abstract_domains", precision = "conservative", status = "budget_exceeded" }
"#;

        let parsed: crate::eval::fixtures::NativeFixtureManifest =
            toml::from_str(manifest).expect("abstract-domain expected facts should parse");

        assert_eq!(parsed.area, FixtureArea::AbstractDomains);
        assert_eq!(parsed.expected.len(), 2);
    }

    #[test]
    fn observed_abstract_domain_debug_rows_normalize_to_compact_fact_rows() {
        let debug = json!({
            "abstract_domains": {
                "observations": [{
                    "family": "DomainObservation",
                    "stable_key": "domain-observation:src/app.ts:truthy",
                    "producer_id": "polint.abstract_domains",
                    "layer_id": "polint.abstract_domains",
                    "status": "present",
                    "precision": "exact_local",
                    "path": "src/app.ts",
                    "span": {"start_line": 3, "start_col": 5, "end_line": 3, "end_col": 13},
                    "body_stable_key": "mir-body:src/app.ts:handler",
                    "block_stable_key": "cfg-block:src/app.ts:handler:entry",
                    "operation_stable_key": "mir-op:src/app.ts:handler:branch",
                    "place_stable_key": "place:src/app.ts:handler:param:flag",
                    "slot": "truthiness",
                    "location": "after_operation",
                    "value": "truthiness=true",
                    "reason": null,
                    "raw_source": "if (flag) { secret(); }",
                    "absolute_path": "/tmp/private/src/app.ts"
                }],
                "events": [{
                    "family": "DomainEvent",
                    "stable_key": "domain-event:src/app.ts:budget",
                    "producer_id": "polint.abstract_domains",
                    "layer_id": "polint.abstract_domains",
                    "status": "budget_exceeded",
                    "precision": "conservative",
                    "path": "src/app.ts",
                    "span": {"start_line": 9, "start_col": 3, "end_line": 9, "end_col": 12},
                    "body_stable_key": "mir-body:src/app.ts:handler",
                    "block_stable_key": "cfg-block:src/app.ts:handler:loop",
                    "operation_stable_key": "mir-op:src/app.ts:handler:loop",
                    "slot": "reachability",
                    "reason": "solver_budget_exceeded"
                }],
                "counts": {
                    "by_slot": {"truthiness": 1, "reachability": 1},
                    "by_status": {"present": 1, "budget_exceeded": 1},
                    "by_precision": {"exact_local": 1, "conservative": 1},
                    "by_reason": {"solver_budget_exceeded": 1},
                    "by_provider": {"polint.abstract_domains": 2}
                },
                "index_counts": {
                    "observations_by_body": 1,
                    "observations_by_place": 1,
                    "events_by_status": 1
                }
            }
        });

        let observed = crate::eval::observed::abstract_domain_facts_for_test(&debug);
        let rendered = serde_json::to_string(&observed).unwrap();

        assert!(observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.family == "DomainObservation"
                    && fact.producer_id.as_deref() == Some("polint.abstract_domains")
                    && fact.precision.as_deref() == Some("exact_local")
                    && fact.status == Some(ObservedStatus::Present)
                    && fact.payload.as_deref().is_some_and(|payload| {
                        payload.contains("path=src/app.ts")
                            && payload.contains("span=3:5-3:13")
                            && payload.contains("body=mir-body:src/app.ts:handler")
                            && payload.contains("operation=mir-op:src/app.ts:handler:branch")
                            && payload.contains("place=place:src/app.ts:handler:param:flag")
                            && payload.contains("slot=truthiness")
                            && payload.contains("location=after_operation")
                            && payload.contains("value=truthiness=true")
                    })
            }
            _ => false,
        }));
        assert!(rendered.contains("abstract_domains.counts.by_status.budget_exceeded.nonzero"));
        assert!(rendered.contains("abstract_domains.index_counts.events_by_status.nonzero"));
        for forbidden in [
            "raw_source",
            "source_text",
            "absolute_path",
            "tree_sitter",
            "oxc::",
            "/tmp/private",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "unexpected abstract-domain payload leak: {forbidden}"
            );
        }
    }

    #[test]
    fn abstract_domain_top_unknown_and_budget_statuses_count_as_unknown_metrics() {
        let statuses = [
            ObservedStatus::Top,
            ObservedStatus::Unknown,
            ObservedStatus::Unsupported,
            ObservedStatus::SetupMissing,
            ObservedStatus::BudgetExceeded,
        ];
        let expected = statuses
            .into_iter()
            .enumerate()
            .map(|(index, status)| expected_domain_fact(format!("domain:{index}"), status))
            .collect::<Vec<_>>();
        let observed = [
            ObservedStatus::Top,
            ObservedStatus::Unknown,
            ObservedStatus::Unsupported,
            ObservedStatus::SetupMissing,
            ObservedStatus::BudgetExceeded,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, status)| observed_domain_fact(format!("domain:{index}:observed"), status))
        .collect::<Vec<_>>();

        let summaries = match_case(&expected, &observed, MatcherConfig::default());
        let metrics = compute_metrics(&summaries);

        assert_eq!(
            summaries
                .iter()
                .map(|summary| summary.outcome)
                .collect::<Vec<_>>(),
            vec![MatchOutcome::Unknown; 5]
        );
        assert_eq!(metrics.unknown_count, 5);
    }

    fn expected_domain_fact(stable_key: String, status: ObservedStatus) -> ExpectedItem {
        ExpectedItem::Fact(ExpectedFact {
            family: "DomainObservation".to_string(),
            stable_key,
            mode: AssertionMode::Partial,
            producer_id: Some("polint.abstract_domains".to_string()),
            precision: None,
            status: Some(status),
            false_positive_trap: false,
        })
    }

    fn observed_domain_fact(stable_key: String, status: ObservedStatus) -> ObservedItem {
        ObservedItem::Fact(ObservedFact {
            family: "DomainObservation".to_string(),
            stable_key,
            mode: AssertionMode::Exact,
            producer_id: Some("polint.abstract_domains".to_string()),
            provenance: Some("kernel.metadata_debug_json.abstract_domains".to_string()),
            precision: Some("conservative".to_string()),
            status: Some(status),
            payload: None,
        })
    }
}

#[cfg(test)]
mod direct_summary {
    use crate::eval::model::{
        AssertionMode, DIRECT_SUMMARY_FACT_FAMILIES, ExpectedFact, ExpectedItem, FixtureArea,
        ObservedFact, ObservedItem, ObservedStatus,
    };
    use serde_json::json;

    #[test]
    fn direct_summary_fixture_area_parses_from_toml() {
        let manifest = r#"
schema_version = "polint-eval-fixture-1"
case_id = "direct-summaries-core"
area = "direct-summaries"

[repo]
path = "repo"

[[expected]]
fact = { family = "summary_control", stable_key = "summary:control:panics", mode = "partial", producer_id = "polint.direct_summaries", precision = "local", status = "present" }

[[expected]]
fact = { family = "summary_call", stable_key = "summary:call:unresolved", mode = "partial", producer_id = "polint.direct_summaries", precision = "unknown_top", status = "unknown" }

[[expected]]
fact = { family = "summary_memory", stable_key = "summary:memory:write", mode = "partial", producer_id = "polint.direct_summaries", precision = "setup_aware", status = "present" }

[[expected]]
fact = { family = "summary_tito", stable_key = "summary:tito:param-return", mode = "partial", producer_id = "polint.direct_summaries", precision = "local", status = "present" }

[[expected]]
fact = { family = "summary_event", stable_key = "SummaryEvent", mode = "partial", producer_id = "polint.direct_summaries", precision = "unknown_top", status = "unknown" }
"#;

        let parsed: crate::eval::fixtures::NativeFixtureManifest =
            toml::from_str(manifest).expect("direct-summaries manifest should parse");

        assert_eq!(parsed.area, FixtureArea::DirectSummaries);
        assert_eq!(parsed.expected.len(), 5);
    }

    #[test]
    fn direct_summary_fact_families_are_recognized() {
        assert_eq!(
            DIRECT_SUMMARY_FACT_FAMILIES,
            [
                "summary_control",
                "summary_call",
                "summary_memory",
                "summary_tito",
                "summary_event",
            ]
        );

        for family in DIRECT_SUMMARY_FACT_FAMILIES {
            assert!(!family.is_empty(), "fact family must be non-empty");
        }
    }

    #[test]
    fn observed_direct_summary_debug_rows_normalize_to_fact_rows_with_compact_payloads() {
        let debug = json!({
            "summaries": {
                "summaries": [
                    {
                        "callable_stable_key": "func:main",
                        "domain": "control_effects",
                        "status": "present",
                        "precision": "local",
                        "provenance": "native_local",
                        "payload_digest": "abc12345deadbeef",
                        "stable_key": "summary:control_effects:func:main"
                    },
                    {
                        "callable_stable_key": "func:dynamic",
                        "domain": "call_effects",
                        "status": "unknown",
                        "precision": "unknown_top",
                        "provenance": "native_local",
                        "payload_digest": "ffffffff00000000",
                        "stable_key": "summary:call_effects:func:dynamic"
                    },
                    {
                        "callable_stable_key": "func:mutate",
                        "domain": "memory_effects",
                        "status": "present",
                        "precision": "setup_aware",
                        "provenance": "native_local",
                        "payload_digest": "deadbeef12345678",
                        "stable_key": "summary:memory_effects:func:mutate"
                    },
                    {
                        "callable_stable_key": "func:identity",
                        "domain": "data_flow_tito",
                        "status": "present",
                        "precision": "local",
                        "provenance": "lifted_from_domain",
                        "payload_digest": "1234567890abcdef",
                        "stable_key": "summary:data_flow_tito:func:identity"
                    }
                ],
                "events": [
                    {
                        "callable_stable_key": "func:dynamic",
                        "domain": "call_effects",
                        "event_kind": "unresolved_callee",
                        "reason": "dynamic_dispatch",
                        "status": "unknown",
                        "precision": "unknown_top",
                        "stable_key": "summary_event:call_effects:func:dynamic:0"
                    }
                ],
                "counts": {
                    "total": 4,
                    "present": 3,
                    "unknown": 1,
                    "unsupported": 0,
                    "setup_missing": 0,
                    "budget_exceeded": 0,
                    "events": 1
                },
                "domain_counts": {
                    "control_effects": 1,
                    "call_effects": 1,
                    "memory_effects": 1,
                    "data_flow_tito": 1
                }
            }
        });

        let observed = crate::eval::observed::direct_summary_facts_for_test(&debug);

        // Check that we get facts for all four summary domains
        assert!(observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.family == "summary_control"
                    && fact.producer_id.as_deref() == Some("polint.direct_summaries")
                    && fact.precision.as_deref() == Some("local")
                    && fact.status == Some(ObservedStatus::Present)
                    && fact.payload.as_deref().is_some_and(|p| {
                        p.contains("domain=control_effects")
                            && p.contains("status=present")
                            && p.contains("precision=local")
                            && p.contains("provenance=native_local")
                            && p.contains("payload_digest_prefix=abc12345")
                    })
            }
            _ => false,
        }));

        assert!(observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.family == "summary_call"
                    && fact.status == Some(ObservedStatus::Unknown)
                    && fact.precision.as_deref() == Some("unknown_top")
            }
            _ => false,
        }));

        assert!(observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.family == "summary_memory" && fact.status == Some(ObservedStatus::Present)
            }
            _ => false,
        }));

        assert!(observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.family == "summary_tito"
                    && fact.status == Some(ObservedStatus::Present)
                    && fact
                        .payload
                        .as_deref()
                        .is_some_and(|p| p.contains("provenance=lifted_from_domain"))
            }
            _ => false,
        }));

        // Check event rows
        assert!(observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.family == "summary_event"
                    && fact.status == Some(ObservedStatus::Unknown)
                    && fact.payload.as_deref().is_some_and(|p| {
                        p.contains("event_kind=unresolved_callee")
                            && p.contains("reason=dynamic_dispatch")
                    })
            }
            _ => false,
        }));

        // Check count invariants
        assert!(observed.iter().any(|item| match item {
            ObservedItem::Invariant(inv) => {
                inv.name == "direct_summaries.counts.total" && inv.value == "4"
            }
            _ => false,
        }));
        assert!(observed.iter().any(|item| match item {
            ObservedItem::Invariant(inv) => {
                inv.name == "direct_summaries.counts.present" && inv.value == "3"
            }
            _ => false,
        }));

        // Check domain count invariants
        for domain in [
            "control_effects",
            "call_effects",
            "memory_effects",
            "data_flow_tito",
        ] {
            assert!(
                observed.iter().any(|item| match item {
                    ObservedItem::Invariant(inv) => {
                        inv.name == format!("direct_summaries.domain_counts.{domain}.nonzero")
                            && inv.value == "true"
                    }
                    _ => false,
                }),
                "missing domain count invariant for {domain}"
            );
        }
    }

    #[test]
    fn direct_summary_unknown_and_budget_statuses_count_as_unknown_metrics() {
        use crate::eval::matcher::{MatchOutcome, MatcherConfig, match_case};
        use crate::eval::metrics::compute_metrics;

        let expected = [
            ExpectedItem::Fact(ExpectedFact {
                family: "summary_control".to_string(),
                stable_key: "unknown".to_string(),
                mode: AssertionMode::Partial,
                producer_id: Some("polint.direct_summaries".to_string()),
                precision: None,
                status: Some(ObservedStatus::Unknown),
                false_positive_trap: false,
            }),
            ExpectedItem::Fact(ExpectedFact {
                family: "summary_call".to_string(),
                stable_key: "unsupported".to_string(),
                mode: AssertionMode::Partial,
                producer_id: Some("polint.direct_summaries".to_string()),
                precision: None,
                status: Some(ObservedStatus::Unsupported),
                false_positive_trap: false,
            }),
            ExpectedItem::Fact(ExpectedFact {
                family: "summary_memory".to_string(),
                stable_key: "setup".to_string(),
                mode: AssertionMode::Partial,
                producer_id: Some("polint.direct_summaries".to_string()),
                precision: None,
                status: Some(ObservedStatus::SetupMissing),
                false_positive_trap: false,
            }),
        ];

        let observed = [
            ObservedItem::Fact(ObservedFact {
                family: "summary_control".to_string(),
                stable_key: "summary:control:unknown:observed".to_string(),
                mode: AssertionMode::Exact,
                producer_id: Some("polint.direct_summaries".to_string()),
                provenance: Some("kernel.metadata_debug_json.summaries".to_string()),
                precision: Some("unknown_top".to_string()),
                status: Some(ObservedStatus::Unknown),
                payload: None,
            }),
            ObservedItem::Fact(ObservedFact {
                family: "summary_call".to_string(),
                stable_key: "summary:call:unsupported:observed".to_string(),
                mode: AssertionMode::Exact,
                producer_id: Some("polint.direct_summaries".to_string()),
                provenance: Some("kernel.metadata_debug_json.summaries".to_string()),
                precision: Some("unknown_top".to_string()),
                status: Some(ObservedStatus::Unsupported),
                payload: None,
            }),
            ObservedItem::Fact(ObservedFact {
                family: "summary_memory".to_string(),
                stable_key: "summary:memory:setup:observed".to_string(),
                mode: AssertionMode::Exact,
                producer_id: Some("polint.direct_summaries".to_string()),
                provenance: Some("kernel.metadata_debug_json.summaries".to_string()),
                precision: Some("unknown_top".to_string()),
                status: Some(ObservedStatus::SetupMissing),
                payload: None,
            }),
        ];

        let summaries = match_case(&expected, &observed, MatcherConfig::default());
        let metrics = compute_metrics(&summaries);

        assert_eq!(
            summaries.iter().map(|s| s.outcome).collect::<Vec<_>>(),
            vec![
                MatchOutcome::Unknown,
                MatchOutcome::Unknown,
                MatchOutcome::Unknown
            ]
        );
        assert_eq!(metrics.unknown_count, 3);
    }
}

#[cfg(test)]
mod semantic_rows {
    use crate::eval::matcher::{MatchOutcome, MatcherConfig, match_case};
    use crate::eval::model::{
        AssertionMode, ExpectedFact, ExpectedItem, ObservedFact, ObservedItem, ObservedStatus,
    };
    use serde_json::json;

    #[test]
    fn eval_expected_facts_accept_semantic_families_and_statuses() {
        let cases = crate::eval::model::SEMANTIC_FACT_FAMILIES
            .iter()
            .copied()
            .zip([
                "resolved",
                "dynamic",
                "resolved",
                "ambiguous",
                "unresolved",
                "generated",
                "external",
            ]);

        for (family, status) in cases {
            let manifest = format!(
                r#"
schema_version = "polint-eval-fixture-1"
case_id = "semantic-row-model"
area = "facts"

[repo]
path = "repo"

[[expected]]
fact = {{ family = "{family}", stable_key = "semantic:{family}", mode = "partial", producer_id = "polint.symbol_graph", precision = "setup_aware", status = "{status}" }}
"#
            );
            let parsed: crate::eval::fixtures::NativeFixtureManifest =
                toml::from_str(&manifest).expect("semantic expected fact should parse");
            assert_eq!(parsed.expected.len(), 1);
        }
    }

    #[test]
    fn observed_semantic_debug_rows_normalize_to_fact_rows_with_evidence() {
        let debug = json!({
            "semantic": {
                "scopes": [{
                    "family": "Scope",
                    "stable_key": "scope:src/app.ts:module",
                    "producer_id": "polint.symbol_graph",
                    "layer_id": "polint.symbol_graph",
                    "status": "resolved",
                    "metadata": { "precision": "setup_aware" },
                    "path": "src/app.ts",
                    "span": {
                        "start_byte": 0,
                        "end_byte": 12,
                        "start_line": 1,
                        "start_col": 1,
                        "end_line": 1,
                        "end_col": 13
                    }
                }],
                "imports": [],
                "exports": [],
                "aliases": [],
                "resolutions": [],
                "generated_symbols": [],
                "stable_exports": []
            }
        });

        let observed = crate::eval::observed::metadata_debug_facts_for_test(&debug);

        assert!(observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.family == "Scope"
                    && fact.stable_key == "scope:src/app.ts:module"
                    && fact.producer_id.as_deref() == Some("polint.symbol_graph")
                    && fact.provenance.as_deref() == Some("polint.symbol_graph")
                    && fact.precision.as_deref() == Some("setup_aware")
                    && fact.status == Some(ObservedStatus::Resolved)
                    && fact.payload.as_deref().is_some_and(|payload| {
                        payload.contains("\"path\":\"src/app.ts\"")
                            && payload.contains("\"start_byte\":0")
                    })
            }
            _ => false,
        }));
    }

    #[test]
    fn semantic_partial_and_unknown_status_matching_works() {
        let summaries = match_case(
            &[ExpectedItem::Fact(ExpectedFact {
                family: "Resolution".to_string(),
                stable_key: "handler".to_string(),
                mode: AssertionMode::Partial,
                producer_id: Some("polint.symbol_graph".to_string()),
                precision: Some("ambiguous".to_string()),
                status: Some(ObservedStatus::Ambiguous),
                false_positive_trap: false,
            })],
            &[ObservedItem::Fact(ObservedFact {
                family: "Resolution".to_string(),
                stable_key: "resolution:src/app.ts:handler:candidates".to_string(),
                mode: AssertionMode::Exact,
                producer_id: Some("polint.symbol_graph".to_string()),
                provenance: Some("polint.symbol_graph".to_string()),
                precision: Some("ambiguous".to_string()),
                status: Some(ObservedStatus::Ambiguous),
                payload: None,
            })],
            MatcherConfig::default(),
        );

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].outcome, MatchOutcome::Unknown);
    }
}

#[cfg(test)]
mod topology_rows {
    use crate::core::{AnalysisDb, FileId};
    use crate::eval::matcher::{MatchOutcome, MatcherConfig, match_case};
    use crate::eval::model::{
        AssertionMode, ExpectedFact, ExpectedItem, FixtureArea, ObservedFact, ObservedItem,
        ObservedStatus, TOPOLOGY_FACT_FAMILIES,
    };
    use crate::module_graph::topology::{
        DependencyRequirementFact, DependencyRequirementId, ImportContextKind, ImportToPackageFact,
        ImportToPackageId, ImportToPackageStatus, RepoTopologyOverlayFact, RepoTopologyOverlayId,
        RepoTopologyOverlayKind, RequirementKind, ResolvedDependencyEdgeFact,
        ResolvedDependencyEdgeId, ResolvedDependencyKind, SourceSetFact, SourceSetId,
        SourceSetKind, TopologyOutput, TopologyPackageFact, TopologyPackageId, TopologyPackageKind,
        TopologyPrecision, TopologyStatus, WorkspaceRootFact, WorkspaceRootId, WorkspaceRootKind,
    };

    #[test]
    fn eval_expected_facts_accept_topology_families_and_statuses() {
        let families = TOPOLOGY_FACT_FAMILIES.join(",");
        assert!(families.contains("WorkspaceRoot"));
        assert!(families.contains("TopologyPackage"));
        assert!(families.contains("SourceSet"));
        assert!(families.contains("DependencyRequirement"));
        assert!(families.contains("ResolvedDependencyEdge"));
        assert!(families.contains("ImportToPackage"));
        assert!(families.contains("RepoTopologyOverlay"));

        let manifest = r#"
schema_version = "polint-eval-fixture-1"
case_id = "topology-row-model"
area = "module-topology"

[repo]
path = "repo"

[[expected]]
fact = { family = "ImportToPackage", stable_key = "import:outside", mode = "partial", producer_id = "polint.module_topology", precision = "heuristic", status = "outside_workspace" }

[[expected]]
fact = { family = "DependencyRequirement", stable_key = "requirement:undeclared", mode = "partial", producer_id = "polint.module_graph", precision = "heuristic", status = "undeclared" }

[[expected]]
fact = { family = "ResolvedDependencyEdge", stable_key = "edge:missing-lockfile", mode = "partial", producer_id = "polint.module_graph", precision = "unknown", status = "missing_lockfile" }
"#;

        let parsed: crate::eval::fixtures::NativeFixtureManifest =
            toml::from_str(manifest).expect("topology expected facts should parse");

        assert_eq!(parsed.area, FixtureArea::ModuleTopology);
        assert_eq!(parsed.expected.len(), 3);
    }

    #[test]
    fn observed_topology_rows_normalize_to_fact_rows_with_compact_payloads() {
        let mut db = AnalysisDb::new();
        db.replace_topology_facts(topology_output());

        let observed = crate::eval::observed::topology_facts_for_test(&db);

        for family in TOPOLOGY_FACT_FAMILIES {
            assert!(
                observed.iter().any(|item| matches!(
                    item,
                    ObservedItem::Fact(fact) if fact.family == *family
                )),
                "missing observed topology family {family}"
            );
        }
        assert!(observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.family == "ImportToPackage"
                    && fact.stable_key == "import:outside-workspace"
                    && fact.producer_id.as_deref() == Some("polint.module_topology")
                    && fact.precision.as_deref() == Some("heuristic")
                    && fact.status == Some(ObservedStatus::OutsideWorkspace)
                    && fact.payload.as_deref().is_some_and(|payload| {
                        payload.contains("source_set:set:source")
                            && payload.contains("from:pkg:web")
                            && !payload.contains(env!("CARGO_MANIFEST_DIR"))
                    })
            }
            _ => false,
        }));
    }

    #[test]
    fn topology_unknown_like_statuses_match_as_unknown_metrics() {
        let statuses = [
            ObservedStatus::MissingLockfile,
            ObservedStatus::Unsupported,
            ObservedStatus::Dynamic,
            ObservedStatus::Ambiguous,
            ObservedStatus::Undeclared,
            ObservedStatus::OutsideWorkspace,
        ];

        for status in statuses {
            let summaries = match_case(
                &[ExpectedItem::Fact(ExpectedFact {
                    family: "ImportToPackage".to_string(),
                    stable_key: "import".to_string(),
                    mode: AssertionMode::Partial,
                    producer_id: Some("polint.module_topology".to_string()),
                    precision: None,
                    status: Some(status),
                    false_positive_trap: false,
                })],
                &[ObservedItem::Fact(ObservedFact {
                    family: "ImportToPackage".to_string(),
                    stable_key: "import:row".to_string(),
                    mode: AssertionMode::Exact,
                    producer_id: Some("polint.module_topology".to_string()),
                    provenance: Some("polint.module_topology".to_string()),
                    precision: Some("heuristic".to_string()),
                    status: Some(status),
                    payload: None,
                })],
                MatcherConfig::default(),
            );

            assert_eq!(summaries[0].outcome, MatchOutcome::Unknown);
        }
    }

    fn topology_output() -> TopologyOutput {
        TopologyOutput {
            workspace_roots: vec![WorkspaceRootFact {
                id: WorkspaceRootId(0),
                kind: WorkspaceRootKind::JsWorkspace,
                root_path: "web".to_string(),
                manifest_path: Some("web/package.json".to_string()),
                language: None,
                stable_key: crate::core::stable_key_for_test("root:web"),
                producer_id: "polint.module_graph",
                precision: TopologyPrecision::ExactStatic,
                status: TopologyStatus::Resolved,
            }],
            packages: vec![TopologyPackageFact {
                id: TopologyPackageId(0),
                workspace_root: Some(WorkspaceRootId(0)),
                package: None,
                module_node: None,
                kind: TopologyPackageKind::JsPackage,
                name: "web".to_string(),
                version: Some("1.0.0".to_string()),
                path: "web".to_string(),
                language: None,
                stable_key: crate::core::stable_key_for_test("pkg:web"),
                producer_id: "polint.module_graph",
                precision: TopologyPrecision::ExactStatic,
                status: TopologyStatus::Resolved,
            }],
            source_sets: vec![SourceSetFact {
                id: SourceSetId(0),
                package: Some(TopologyPackageId(0)),
                root: Some(WorkspaceRootId(0)),
                kind: SourceSetKind::Source,
                path: "web/src/app.ts".to_string(),
                language: None,
                files: vec![FileId::from_raw(0)],
                stable_key: crate::core::stable_key_for_test("set:source"),
                producer_id: "polint.module_graph",
                precision: TopologyPrecision::ExactStatic,
                status: TopologyStatus::Present,
            }],
            dependency_requirements: vec![DependencyRequirementFact {
                id: DependencyRequirementId(0),
                from_package: Some(TopologyPackageId(0)),
                target_package: None,
                target_name: "@scope/lib".to_string(),
                version_requirement: Some("^1.0.0".to_string()),
                kind: RequirementKind::Runtime,
                manifest_path: Some("web/package.json".to_string()),
                stable_key: crate::core::stable_key_for_test("requirement:@scope/lib"),
                producer_id: "polint.module_graph",
                precision: TopologyPrecision::ExactStatic,
                status: TopologyStatus::Present,
            }],
            resolved_dependency_edges: vec![ResolvedDependencyEdgeFact {
                id: ResolvedDependencyEdgeId(0),
                requirement: Some(DependencyRequirementId(0)),
                from_package: Some(TopologyPackageId(0)),
                to_package: None,
                package_name: "@scope/lib".to_string(),
                resolved_version: None,
                kind: ResolvedDependencyKind::External,
                stable_key: crate::core::stable_key_for_test("edge:@scope/lib:missing-lockfile"),
                producer_id: "polint.module_graph",
                precision: TopologyPrecision::Unknown,
                status: TopologyStatus::MissingLockfile,
            }],
            import_to_package_edges: vec![ImportToPackageFact {
                id: ImportToPackageId(0),
                syntax_import: None,
                resolved_import: None,
                semantic_import_stable_key: Some(crate::core::stable_key_for_test(
                    "semantic:import:@scope/lib",
                )),
                from_file: Some(FileId::from_raw(0)),
                from_package: Some(TopologyPackageId(0)),
                to_package: None,
                target_node: None,
                from_package_stable_key: Some(crate::core::stable_key_for_test("pkg:web")),
                to_package_stable_key: None,
                source_set_stable_key: Some(crate::core::stable_key_for_test("set:source")),
                import_path: "@scope/lib".to_string(),
                context: ImportContextKind::Source,
                stable_key: crate::core::stable_key_for_test("import:outside-workspace"),
                producer_id: "polint.module_topology",
                precision: TopologyPrecision::Heuristic,
                status: ImportToPackageStatus::OutsideWorkspace,
            }],
            overlays: vec![RepoTopologyOverlayFact {
                id: RepoTopologyOverlayId(0),
                root: Some(WorkspaceRootId(0)),
                package: Some(TopologyPackageId(0)),
                source_set: Some(SourceSetId(0)),
                kind: RepoTopologyOverlayKind::GeneratedZone,
                label: "generated".to_string(),
                path: Some("web/generated".to_string()),
                stable_key: crate::core::stable_key_for_test("overlay:generated"),
                producer_id: "polint.module_graph",
                precision: TopologyPrecision::Heuristic,
                status: TopologyStatus::Present,
            }],
        }
    }
}

#[cfg(test)]
mod semantic_mir_rows {
    use crate::eval::matcher::{MatchOutcome, MatcherConfig, match_case};
    use crate::eval::model::{
        AssertionMode, ExpectedFact, ExpectedItem, FixtureArea, ObservedFact, ObservedItem,
        ObservedStatus, SEMANTIC_MIR_FACT_FAMILIES,
    };
    use serde_json::json;

    #[test]
    fn eval_expected_facts_accept_semantic_mir_families_and_partial_status() {
        assert_eq!(
            SEMANTIC_MIR_FACT_FAMILIES,
            ["MirBody", "MirOperation", "Place", "UnsupportedSemantic"]
        );

        let manifest = r#"
schema_version = "polint-eval-fixture-1"
case_id = "semantic-mir-row-model"
area = "semantic-mir"

[repo]
path = "repo"

[[expected]]
fact = { family = "MirBody", stable_key = "body:handler", mode = "partial", producer_id = "polint.semantic_mir", precision = "setup_aware", status = "resolved" }

[[expected]]
fact = { family = "MirOperation", stable_key = "op:branch", mode = "partial", producer_id = "polint.semantic_mir", precision = "heuristic", status = "partial" }

[[expected]]
fact = { family = "Place", stable_key = "root_kind:parameter", mode = "partial", producer_id = "polint.semantic_mir", precision = "setup_aware", status = "unknown" }

[[expected]]
fact = { family = "UnsupportedSemantic", stable_key = "unsupported:eval", mode = "partial", producer_id = "polint.semantic_mir", precision = "unsupported", status = "unsupported" }
"#;

        let parsed: crate::eval::fixtures::NativeFixtureManifest =
            toml::from_str(manifest).expect("semantic MIR expected facts should parse");

        assert_eq!(parsed.area, FixtureArea::SemanticMir);
        assert_eq!(parsed.expected.len(), 4);
    }

    #[test]
    fn observed_semantic_mir_debug_rows_normalize_to_fact_rows_with_compact_payloads() {
        let debug = json!({
            "mir": {
                "bodies": [{
                    "family": "MirBody",
                    "stable_key": "mir-body:src/app.ts:handler",
                    "producer_id": "polint.semantic_mir",
                    "layer_id": "polint.semantic_mir",
                    "status": "resolved",
                    "precision": "setup_aware",
                    "path": "src/app.ts",
                    "span": {
                        "start_byte": 0,
                        "end_byte": 16,
                        "start_line": 1,
                        "start_col": 1,
                        "end_line": 1,
                        "end_col": 17
                    },
                    "owner_function": 7
                }],
                "operations": [{
                    "family": "MirOperation",
                    "stable_key": "mir-op:src/app.ts:handler:branch",
                    "producer_id": "polint.semantic_mir",
                    "layer_id": "polint.semantic_mir",
                    "status": "partial",
                    "precision": "heuristic",
                    "path": "src/app.ts",
                    "span": {
                        "start_byte": 20,
                        "end_byte": 32,
                        "start_line": 2,
                        "start_col": 3,
                        "end_line": 2,
                        "end_col": 15
                    },
                    "owner_function": 7,
                    "operation_kind": "branch"
                }],
                "places": [{
                    "family": "Place",
                    "stable_key": "place:src/app.ts:handler:parameter:0:user",
                    "producer_id": "polint.semantic_mir",
                    "layer_id": "polint.semantic_mir",
                    "status": "unknown",
                    "precision": "setup_aware",
                    "path": "src/app.ts",
                    "owner_function": 7,
                    "place_root": "parameter:0:user",
                    "place_projections": ["property:name", "index_known:0"]
                }],
                "unsupported": [{
                    "family": "UnsupportedSemantic",
                    "stable_key": "unsupported:src/app.ts:eval",
                    "producer_id": "polint.semantic_mir",
                    "layer_id": "polint.semantic_mir",
                    "status": "unsupported",
                    "precision": "unsupported",
                    "path": "src/app.ts",
                    "span": {
                        "start_byte": 44,
                        "end_byte": 55,
                        "start_line": 4,
                        "start_col": 5,
                        "end_line": 4,
                        "end_col": 16
                    },
                    "owner_function": 7,
                    "unsupported_construct": "eval",
                    "conservative_action": "havoc_affected_places"
                }]
            }
        });

        let observed = crate::eval::observed::semantic_mir_facts_for_test(&debug);

        for family in SEMANTIC_MIR_FACT_FAMILIES {
            assert!(
                observed.iter().any(|item| matches!(
                    item,
                    ObservedItem::Fact(fact) if fact.family == *family
                )),
                "missing semantic MIR family {family}: {observed:#?}"
            );
        }
        let rendered = serde_json::to_string(&observed).unwrap();
        assert!(rendered.contains("path=src/app.ts"));
        assert!(rendered.contains("span=1:1-1:17"));
        assert!(rendered.contains("owner=7"));
        assert!(rendered.contains("kind=branch"));
        assert!(rendered.contains("root=parameter:0:user"));
        assert!(rendered.contains("projections=property:name>index_known:0"));
        assert!(rendered.contains("construct=eval"));
        assert!(rendered.contains("conservative_action=havoc_affected_places"));
        for forbidden in [
            "raw_source",
            "source_text",
            "tree_sitter",
            "oxc_ast",
            "/tmp/",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "unexpected payload leak: {forbidden}"
            );
        }
    }

    #[test]
    fn semantic_mir_unknown_like_statuses_match_as_unknown_metrics() {
        for status in [
            ObservedStatus::Partial,
            ObservedStatus::Unknown,
            ObservedStatus::Unsupported,
        ] {
            let summaries = match_case(
                &[ExpectedItem::Fact(ExpectedFact {
                    family: "MirOperation".to_string(),
                    stable_key: "handler".to_string(),
                    mode: AssertionMode::Partial,
                    producer_id: Some("polint.semantic_mir".to_string()),
                    precision: None,
                    status: Some(status),
                    false_positive_trap: false,
                })],
                &[ObservedItem::Fact(ObservedFact {
                    family: "MirOperation".to_string(),
                    stable_key: "mir-op:handler:branch".to_string(),
                    mode: AssertionMode::Exact,
                    producer_id: Some("polint.semantic_mir".to_string()),
                    provenance: Some("kernel.metadata_debug_json.mir".to_string()),
                    precision: Some("heuristic".to_string()),
                    status: Some(status),
                    payload: None,
                })],
                MatcherConfig::default(),
            );

            assert_eq!(summaries[0].outcome, MatchOutcome::Unknown);
        }
    }
}
