use crate::cache::keys::deterministic_rule_options;
use crate::cache::stable_hash;
use crate::config::{LoadedConfig, RuleConfig};
use crate::core::{
    Capabilities, CapabilitySupport, CapabilitySupportStatus, CapabilitySupportView, Language,
    Rule, RuleMeta, RuleOptions, SourceFile, rule_id_matches,
};
use crate::diagnostics::{Diagnostic, Severity, TextRange};
use crate::sdk::policy::POLICY_QUERY_VERSION;
#[cfg(test)]
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

pub(crate) const ANALYSIS_PLAN_SCHEMA: &str = "analysis-plan-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnalysisPlan {
    digest: String,
    rules: Vec<PlannedRule>,
    capabilities: Vec<PlannedCapability>,
    setup_checks: Vec<SetupCheck>,
    support_view: CapabilitySupportView,
}

#[derive(Debug, Clone)]
pub(crate) struct RulePlanInputs {
    rules: Vec<RulePlanInput>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
struct RulePlanInput {
    meta: RuleMeta,
    capabilities: Capabilities,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedRule {
    pub(crate) id: String,
    pub(crate) description: String,
    pub(crate) severity: Severity,
    pub(crate) requested_capabilities: Vec<String>,
    pub(crate) files: Vec<String>,
    pub(crate) allow_files: Vec<String>,
    pub(crate) options_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedCapability {
    pub(crate) capability: String,
    pub(crate) language: Option<Language>,
    pub(crate) status: CapabilitySupportStatus,
    pub(crate) rules: Vec<String>,
    pub(crate) reason: Option<String>,
    pub(crate) hint: Option<String>,
    pub(crate) docs_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SetupCheck {
    pub(crate) id: String,
    pub(crate) capability: String,
    pub(crate) language: Option<Language>,
    pub(crate) status: String,
    pub(crate) reason: Option<String>,
    pub(crate) hint: Option<String>,
    pub(crate) docs_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg(test)]
pub(crate) struct ExplainPlanReport {
    pub(crate) schema: String,
    pub(crate) digest: String,
    pub(crate) rules: Vec<ExplainPlanRule>,
    pub(crate) capabilities: Vec<ExplainPlanCapability>,
    pub(crate) setup_checks: Vec<ExplainPlanSetupCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg(test)]
pub(crate) struct ExplainPlanRule {
    pub(crate) id: String,
    pub(crate) description: String,
    pub(crate) severity: Severity,
    pub(crate) capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg(test)]
pub(crate) struct ExplainPlanCapability {
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) rules: Vec<String>,
    pub(crate) reason: Option<String>,
    pub(crate) hint: Option<String>,
    pub(crate) docs_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg(test)]
pub(crate) struct ExplainPlanSetupCheck {
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) docs_path: Option<String>,
}

impl AnalysisPlan {
    pub(crate) fn empty() -> Self {
        Self::finish(Vec::new(), Vec::new(), Vec::new())
    }

    #[allow(dead_code)]
    pub(crate) fn from_rules(
        rules: &[Rule],
        enabled: Option<&BTreeSet<String>>,
        options: &BTreeMap<String, RuleOptions>,
    ) -> Self {
        let inputs = RulePlanInputs::collect(rules, enabled);
        Self::from_inputs(&inputs, options)
    }

    pub(crate) fn from_inputs(
        inputs: &RulePlanInputs,
        options: &BTreeMap<String, RuleOptions>,
    ) -> Self {
        let mut planned_rules = inputs
            .rules
            .iter()
            .map(|input| {
                let capabilities = requested_capabilities(input.capabilities);
                let default_options = RuleOptions::default();
                let rule_options = options.get(&input.meta.id).unwrap_or(&default_options);
                let options_digest = deterministic_rule_options(rule_options);

                PlannedRule {
                    id: input.meta.id.clone(),
                    description: input.meta.description.clone(),
                    severity: rule_options.severity.unwrap_or(input.meta.severity),
                    requested_capabilities: capabilities,
                    files: rule_options.files.clone(),
                    allow_files: rule_options.allow_files.clone(),
                    options_digest,
                }
            })
            .collect::<Vec<_>>();
        planned_rules.sort_by(|left, right| {
            (
                left.id.as_str(),
                left.description.as_str(),
                left.severity.to_string(),
            )
                .cmp(&(
                    right.id.as_str(),
                    right.description.as_str(),
                    right.severity.to_string(),
                ))
        });

        let capabilities = plan_capabilities(&planned_rules);

        Self::finish(planned_rules, capabilities, Vec::new())
    }

    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }

    #[allow(dead_code)]
    pub(crate) fn rules(&self) -> &[PlannedRule] {
        &self.rules
    }

    #[allow(dead_code)]
    pub(crate) fn capabilities(&self) -> &[PlannedCapability] {
        &self.capabilities
    }

    pub(crate) fn requests_capability(&self, capability: &str) -> bool {
        self.capabilities
            .iter()
            .any(|planned| planned.capability == capability)
    }

    pub(crate) fn requests_any_capability(&self, capabilities: &[&str]) -> bool {
        capabilities
            .iter()
            .any(|capability| self.requests_capability(capability))
    }

    pub(crate) fn rules_for_capability_matching_files(
        &self,
        capability: &str,
        files: &[&SourceFile],
    ) -> Vec<String> {
        self.rules
            .iter()
            .filter(|rule| rule_requests_capability(rule, capability))
            .filter(|rule| rule_matches_any_file(rule, files))
            .map(|rule| rule.id.clone())
            .collect()
    }

    #[allow(dead_code)]
    pub(crate) fn setup_checks(&self) -> &[SetupCheck] {
        &self.setup_checks
    }

    pub(crate) fn support_view(&self) -> &CapabilitySupportView {
        &self.support_view
    }

    /// Build per-rule execution rows for silent-rule diagnosis.
    ///
    /// One row per planned rule. `files_in_scope` counts analyzed files that pass
    /// this rule's `files` / `allow_files` filters (post-discovery). Capability
    /// skips set `planned = false` and copy the existing `polint/capability`
    /// diagnostic message into `skipped_reason`.
    pub(crate) fn rule_execution_rows(
        &self,
        options: &BTreeMap<String, RuleOptions>,
        file_paths: &[&str],
        diagnostics: &[Diagnostic],
        observed_events: &BTreeMap<String, u64>,
    ) -> Vec<crate::diagnostics::RuleExecutionRow> {
        self.rules
            .iter()
            .map(|rule| {
                let capabilities_ok = !self.support_view.entries().iter().any(|entry| {
                    entry.rules.iter().any(|id| id == &rule.id)
                        && entry.status != CapabilitySupportStatus::Supported
                });
                let planned = capabilities_ok;
                let default_options = RuleOptions::default();
                let rule_options = options.get(&rule.id).unwrap_or(&default_options);
                let files_in_scope = file_paths
                    .iter()
                    .filter(|path| crate::sdk::scope::file_matches_globs(rule_options, path))
                    .count();
                let diagnostics_emitted = diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.rule_id == rule.id)
                    .count();
                let skipped_reason = if planned && capabilities_ok {
                    None
                } else {
                    capability_skip_reason_from_diagnostics(&rule.id, diagnostics)
                };
                let blocking_providers = blocking_providers_from_diagnostics(&rule.id, diagnostics);
                // A rule blocked at runtime survives capability *planning*, so
                // `planned` alone cannot tell a blocked rule from one that ran
                // and found nothing. The capability diagnostic can.
                let outcome = if !blocking_providers.is_empty()
                    || capability_diagnostic_for(&rule.id, diagnostics).is_some()
                {
                    crate::diagnostics::RULE_OUTCOME_CAPABILITY_BLOCKED
                } else if !planned {
                    crate::diagnostics::RULE_OUTCOME_NOT_PLANNED
                } else {
                    crate::diagnostics::RULE_OUTCOME_ANALYZED
                };
                crate::diagnostics::RuleExecutionRow {
                    rule_id: rule.id.clone(),
                    planned,
                    capabilities_ok,
                    files_in_scope,
                    diagnostics_emitted,
                    skipped_reason,
                    outcome: outcome.to_string(),
                    blocking_providers,
                    observed_events: observed_events.get(&rule.id).copied().unwrap_or_default(),
                }
            })
            .collect()
    }

    pub(crate) fn diagnostics(&self) -> Vec<Diagnostic> {
        self.capabilities
            .iter()
            .filter(|capability| capability.status != CapabilitySupportStatus::Supported)
            .flat_map(|capability| {
                capability
                    .rules
                    .iter()
                    .map(|rule_id| capability_diagnostic(capability, rule_id))
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn explain_report(&self) -> ExplainPlanReport {
        ExplainPlanReport {
            schema: ANALYSIS_PLAN_SCHEMA.to_string(),
            digest: self.digest.clone(),
            rules: self
                .rules
                .iter()
                .map(|rule| ExplainPlanRule {
                    id: rule.id.clone(),
                    description: rule.description.clone(),
                    severity: rule.severity,
                    capabilities: rule.requested_capabilities.clone(),
                })
                .collect(),
            capabilities: self
                .capabilities
                .iter()
                .map(|capability| ExplainPlanCapability {
                    name: capability.capability.clone(),
                    status: capability_status_json(&capability.status).to_string(),
                    rules: capability.rules.clone(),
                    reason: capability.reason.clone(),
                    hint: capability.hint.clone(),
                    docs_path: capability.docs_path.clone(),
                })
                .collect(),
            setup_checks: self
                .setup_checks
                .iter()
                .map(|check| ExplainPlanSetupCheck {
                    id: check.id.clone(),
                    status: check.status.clone(),
                    message: setup_check_message(check),
                    docs_path: check.docs_path.clone(),
                })
                .collect(),
        }
    }

    pub(crate) fn from_capability_names(names: &[&str]) -> Self {
        let rules = vec![PlannedRule {
            id: "polint/requested-capabilities".to_string(),
            description: "Requested capability analysis".to_string(),
            severity: Severity::Warn,
            requested_capabilities: names.iter().map(|name| (*name).to_string()).collect(),
            files: Vec::new(),
            allow_files: Vec::new(),
            options_digest: deterministic_rule_options(&RuleOptions::default()),
        }];
        let capabilities = plan_capabilities(&rules);
        Self::finish(rules, capabilities, Vec::new())
    }

    /// A plan that requests enough capabilities to exercise the whole analysis
    /// pipeline (module/symbol graphs + the interprocedural/semantic stages + the
    /// metric stages). Use this in tests and eval fixtures that assert on deep
    /// facts (call graph, points-to, reachability, semantic graph, RTA, …) so the
    /// kernel's capability-closure schedule includes the deep providers. Mirrors
    /// what a rule requesting a graph capability would trigger in production.
    #[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
    pub(crate) fn full_pipeline_for_test() -> Self {
        Self::from_capability_names_for_test(&[
            "dataflow",
            "file_metrics",
            "function_metrics",
            "complexity_metrics",
        ])
    }

    #[cfg(test)]
    pub(crate) fn from_capability_names_for_test(names: &[&str]) -> Self {
        let rules = vec![PlannedRule {
            id: "test/requested-capabilities".to_string(),
            description: "Test rule".to_string(),
            severity: Severity::Warn,
            requested_capabilities: names.iter().map(|name| (*name).to_string()).collect(),
            files: Vec::new(),
            allow_files: Vec::new(),
            options_digest: deterministic_rule_options(&RuleOptions::default()),
        }];
        let capabilities = plan_capabilities(&rules);
        Self::finish(rules, capabilities, Vec::new())
    }

    fn finish(
        rules: Vec<PlannedRule>,
        capabilities: Vec<PlannedCapability>,
        setup_checks: Vec<SetupCheck>,
    ) -> Self {
        let support_view = CapabilitySupportView::new(
            capabilities
                .iter()
                .map(|capability| CapabilitySupport {
                    capability: capability.capability.clone(),
                    language: capability.language,
                    status: capability.status.clone(),
                    rules: capability.rules.clone(),
                    reason: capability.reason.clone(),
                    hint: capability.hint.clone(),
                    docs_path: capability.docs_path.clone(),
                })
                .collect(),
        );
        let digest = plan_digest(&rules, &capabilities, &setup_checks);

        Self {
            digest,
            rules,
            capabilities,
            setup_checks,
            support_view,
        }
    }
}

#[cfg(test)]
impl ExplainPlanReport {
    pub(crate) fn to_human(&self) -> String {
        let mut out = String::new();
        out.push_str("Analysis plan\n");
        out.push_str(&format!("Digest: {}\n\n", self.digest));

        out.push_str("Rules\n");
        if self.rules.is_empty() {
            out.push_str("- none\n");
        } else {
            for rule in &self.rules {
                let capabilities = if rule.capabilities.is_empty() {
                    "none".to_string()
                } else {
                    rule.capabilities.join(", ")
                };
                out.push_str(&format!(
                    "- {} [{}]: {} (capabilities: {})\n",
                    rule.id, rule.severity, rule.description, capabilities
                ));
            }
        }

        out.push_str("\nCapabilities\n");
        if self.capabilities.is_empty() {
            out.push_str("- none\n");
        } else {
            for capability in &self.capabilities {
                let rules = if capability.rules.is_empty() {
                    "none".to_string()
                } else {
                    capability.rules.join(", ")
                };
                out.push_str(&format!(
                    "- {}: {} (rules: {})",
                    capability.name, capability.status, rules
                ));
                if let Some(reason) = &capability.reason {
                    out.push_str(&format!(" - {reason}"));
                }
                if let Some(hint) = &capability.hint {
                    out.push_str(&format!(" Hint: {hint}"));
                }
                if let Some(docs_path) = &capability.docs_path {
                    out.push_str(&format!(" See: {docs_path}"));
                }
                out.push('\n');
            }
        }

        out.push_str("\nSetup checks\n");
        if self.setup_checks.is_empty() {
            out.push_str("- none\n");
        } else {
            for check in &self.setup_checks {
                out.push_str(&format!(
                    "- {}: {} - {}",
                    check.id, check.status, check.message
                ));
                if let Some(docs_path) = &check.docs_path {
                    out.push_str(&format!(" See: {docs_path}"));
                }
                out.push('\n');
            }
        }

        out
    }
}

impl RulePlanInputs {
    pub(crate) fn collect(rules: &[Rule], enabled: Option<&BTreeSet<String>>) -> Self {
        let mut inputs = Vec::new();
        let mut diagnostics = Vec::new();

        for rule in rules {
            let meta = match catch_unwind(AssertUnwindSafe(|| rule.meta())) {
                Ok(meta) => meta,
                Err(_) => {
                    diagnostics.push(internal_plan_error("unknown", "rule metadata panicked"));
                    continue;
                }
            };

            if let Some(enabled) = enabled
                && !enabled
                    .iter()
                    .any(|pattern| rule_id_matches(pattern, &meta.id))
            {
                continue;
            }

            let capabilities = match catch_unwind(AssertUnwindSafe(|| rule.capabilities())) {
                Ok(capabilities) => capabilities,
                Err(_) => {
                    diagnostics.push(capability_collection_error(&meta));
                    Capabilities::new()
                }
            };

            inputs.push(RulePlanInput { meta, capabilities });
        }

        Self {
            rules: inputs,
            diagnostics,
        }
    }

    pub(crate) fn retain_rule_pattern(&mut self, pattern: Option<&str>) {
        let Some(pattern) = pattern else {
            return;
        };
        self.rules
            .retain(|input| rule_id_matches(pattern, &input.meta.id));
    }

    pub(crate) fn exact_rule_ids(&self) -> BTreeSet<String> {
        self.rules
            .iter()
            .map(|input| input.meta.id.clone())
            .collect()
    }

    pub(crate) fn rule_options_from_config(
        &self,
        loaded: &LoadedConfig,
    ) -> BTreeMap<String, RuleOptions> {
        self.rules
            .iter()
            .map(|input| {
                (
                    input.meta.id.clone(),
                    rule_options_from_config(loaded.rule_config(&input.meta.id)),
                )
            })
            .collect()
    }

    pub(crate) fn rule_digest(&self, options: &BTreeMap<String, RuleOptions>) -> String {
        let mut parts = Vec::new();
        let mut rules = self.rules.iter().collect::<Vec<_>>();
        rules.sort_by(|left, right| {
            (
                left.meta.id.as_str(),
                left.meta.description.as_str(),
                left.meta.severity.to_string(),
            )
                .cmp(&(
                    right.meta.id.as_str(),
                    right.meta.description.as_str(),
                    right.meta.severity.to_string(),
                ))
        });

        for input in rules {
            parts.push(format!("rule:{}", input.meta.id));
            parts.push(format!("description:{}", input.meta.description));
            parts.push(format!("severity:{}", input.meta.severity));
            let options_digest = options
                .get(&input.meta.id)
                .map(deterministic_rule_options)
                .unwrap_or_else(|| deterministic_rule_options(&RuleOptions::default()));
            parts.push(format!("options:{options_digest}"));
        }

        let part_refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
        stable_hash(&part_refs)
    }

    pub(crate) fn diagnostics(&self) -> Vec<Diagnostic> {
        self.diagnostics.clone()
    }
}

fn internal_plan_error(rule_id: &str, message: &str) -> Diagnostic {
    Diagnostic::error(
        format!("internal/{rule_id}"),
        "<workspace>",
        TextRange::point(1, 1),
        format!("Rule `{rule_id}` failed: {message}"),
    )
}

fn capability_collection_error(meta: &RuleMeta) -> Diagnostic {
    Diagnostic::error(
        "polint/capability",
        "<workspace>",
        TextRange::point(1, 1),
        format!("Rule `{}` capability collection panicked.", meta.id),
    )
    .with_evidence("rule", meta.id.clone())
    .with_help("Capability declarations must not panic; polint ignored requested capabilities for this rule.")
}

fn capability_diagnostic(capability: &PlannedCapability, rule_id: &str) -> Diagnostic {
    let message = match capability.status {
        CapabilitySupportStatus::Supported => unreachable!("supported capabilities are filtered"),
        CapabilitySupportStatus::Unsupported => format!(
            "Rule `{rule_id}` requested unsupported capability `{}`.",
            capability.capability
        ),
        CapabilitySupportStatus::SetupMissing => format!(
            "Rule `{rule_id}` requested capability `{}`, but required setup is missing.",
            capability.capability
        ),
    };
    let docs_path = capability
        .docs_path
        .as_deref()
        .unwrap_or("docs/facts/capability-plans.md");
    let help = match capability.status {
        CapabilitySupportStatus::Supported => unreachable!("supported capabilities are filtered"),
        CapabilitySupportStatus::Unsupported => format!(
            "Capability `{}` is not currently supported; see {docs_path}.",
            capability.capability
        ),
        CapabilitySupportStatus::SetupMissing => format!(
            "Capability `{}` needs additional local setup before this rule can run; see {docs_path}.",
            capability.capability
        ),
    };

    Diagnostic::error(
        "polint/capability",
        "<workspace>",
        TextRange::point(1, 1),
        message,
    )
    .with_evidence("rule", rule_id.to_string())
    .with_evidence("capability", capability.capability.clone())
    .with_evidence(
        "status",
        capability_status_json(&capability.status).to_string(),
    )
    .with_help(help)
}

fn capability_skip_reason_from_diagnostics(
    rule_id: &str,
    diagnostics: &[Diagnostic],
) -> Option<String> {
    capability_diagnostic_for(rule_id, diagnostics).map(|diagnostic| diagnostic.message.clone())
}

fn capability_diagnostic_for<'a>(
    rule_id: &str,
    diagnostics: &'a [Diagnostic],
) -> Option<&'a Diagnostic> {
    diagnostics.iter().find(|diagnostic| {
        diagnostic.rule_id == "polint/capability"
            && diagnostic
                .evidence
                .iter()
                .any(|evidence| evidence.label == "rule" && evidence.value == rule_id)
    })
}

/// Providers whose failure blocked `rule_id`, read from the capability
/// diagnostics the kernel already emits.
fn blocking_providers_from_diagnostics(rule_id: &str, diagnostics: &[Diagnostic]) -> Vec<String> {
    let mut providers = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.rule_id == "polint/capability"
                && diagnostic
                    .evidence
                    .iter()
                    .any(|evidence| evidence.label == "rule" && evidence.value == rule_id)
        })
        .flat_map(|diagnostic| {
            diagnostic
                .evidence
                .iter()
                .filter(|evidence| evidence.label == "blockers")
                .flat_map(|evidence| evidence.value.split(','))
                .map(str::trim)
                .filter(|provider| !provider.is_empty())
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    providers.sort();
    providers.dedup();
    providers
}

#[cfg(test)]
fn setup_check_message(check: &SetupCheck) -> String {
    match (&check.reason, &check.hint) {
        (Some(reason), Some(hint)) => format!("{reason} {hint}"),
        (Some(reason), None) => reason.clone(),
        (None, Some(hint)) => hint.clone(),
        (None, None) => String::new(),
    }
}

fn rule_options_from_config(config: Option<&RuleConfig>) -> RuleOptions {
    let Some(config) = config else {
        return RuleOptions::default();
    };
    RuleOptions {
        severity: config.severity.as_deref().and_then(parse_severity),
        files: config.files.clone(),
        allow_files: config.allow_files.clone(),
        allow: config.allow.clone(),
        max: config.max,
        deny: config.deny.clone(),
        forbidden_imports: config.forbidden_imports.clone(),
        settings: config.settings.clone(),
    }
}

fn rule_matches_any_file(rule: &PlannedRule, files: &[&SourceFile]) -> bool {
    let options = RuleOptions {
        files: rule.files.clone(),
        allow_files: rule.allow_files.clone(),
        ..RuleOptions::default()
    };
    files
        .iter()
        .any(|file| crate::sdk::scope::file_matches_globs(&options, &file.relative_path))
}

fn parse_severity(value: &str) -> Option<Severity> {
    match value {
        "info" => Some(Severity::Info),
        "warn" | "warning" => Some(Severity::Warn),
        "error" => Some(Severity::Error),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct CapabilityAccumulator {
    status: CapabilitySupportStatus,
    rules: BTreeSet<String>,
    reason: Option<String>,
    hint: Option<String>,
    docs_path: Option<String>,
}

fn plan_capabilities(rules: &[PlannedRule]) -> Vec<PlannedCapability> {
    let mut capabilities = BTreeMap::<String, CapabilityAccumulator>::new();
    for rule in rules {
        for requested in &rule.requested_capabilities {
            insert_capability_request(&mut capabilities, requested, &rule.id);
            for dependency in capability_dependencies(requested) {
                insert_capability_request(&mut capabilities, dependency, &rule.id);
            }
        }
    }

    capabilities
        .into_iter()
        .map(|(capability, accumulator)| PlannedCapability {
            capability,
            language: None,
            status: accumulator.status,
            rules: accumulator.rules.into_iter().collect(),
            reason: accumulator.reason,
            hint: accumulator.hint,
            docs_path: accumulator.docs_path,
        })
        .collect()
}

fn insert_capability_request(
    capabilities: &mut BTreeMap<String, CapabilityAccumulator>,
    capability: &str,
    rule_id: &str,
) {
    let entry = capabilities
        .entry(capability.to_string())
        .or_insert_with(|| support_for(capability));
    entry.rules.insert(rule_id.to_string());
}

fn capability_dependencies(capability: &str) -> &'static [&'static str] {
    match capability {
        "calls" | "control_flow" | "dataflow" => {
            &["resolved_imports", "module_graph", "symbols", "references"]
        }
        "references" => &["symbols"],
        _ => &[],
    }
}

#[rustfmt::skip]
fn support_for(capability: &str) -> CapabilityAccumulator {
    let (status, reason, hint, docs_path) = match capability {
        "syntax" | "imports" | "go_tests" | "branch_obligations" | "file_metrics"
        | "function_metrics" | "complexity_metrics" | "ts_components" | "ts_classes"
        | "string_literals" | "jsx_attributes" | "changeset" => {
            (CapabilitySupportStatus::Supported, None, None, None)
        }
        "resolved_imports" | "module_graph" => (CapabilitySupportStatus::Supported, None, None, None),
        "symbols" | "references" => (CapabilitySupportStatus::Supported, None, None, None),
        "test_suite_metrics" => (
            CapabilitySupportStatus::Unsupported,
            Some("Normalized test suite metrics are not currently supported.".to_string()),
            Some("Use go_tests for current Go test evidence.".to_string()),
            Some("docs/facts/capability-plans.md".to_string()),
        ),
        "events" => (
            CapabilitySupportStatus::Supported,
            None,
            None,
            None,
        ),
        "calls" => (
            CapabilitySupportStatus::Supported,
            None,
            None,
            None,
        ),
        "control_flow" => (
            CapabilitySupportStatus::Supported,
            None,
            None,
            None,
        ),
        "dataflow" => (
            CapabilitySupportStatus::Supported,
            None,
            None,
            None,
        ),
        "cfg" | "call_graph" | "coverage_facts" => (
            CapabilitySupportStatus::Unsupported,
            Some("Capability is not currently supported.".to_string()),
            None,
            Some("docs/facts/capability-plans.md".to_string()),
        ),
        _ => (
            CapabilitySupportStatus::Unsupported,
            Some("Capability is not recognized by this analysis plan schema.".to_string()),
            None,
            Some("docs/facts/capability-plans.md".to_string()),
        ),
    };

    CapabilityAccumulator {
        status,
        rules: BTreeSet::new(),
        reason,
        hint,
        docs_path,
    }
}

fn requested_capabilities(capabilities: Capabilities) -> Vec<String> {
    capabilities.requested_names().map(str::to_string).collect()
}

fn rule_requests_capability(rule: &PlannedRule, capability: &str) -> bool {
    rule.requested_capabilities.iter().any(|requested| {
        requested == capability || capability_dependencies(requested).contains(&capability)
    })
}

fn plan_digest(
    rules: &[PlannedRule],
    capabilities: &[PlannedCapability],
    setup_checks: &[SetupCheck],
) -> String {
    let mut parts = Vec::new();
    parts.push(format!("schema={}", encode_str(ANALYSIS_PLAN_SCHEMA)));

    for rule in rules {
        parts.push(format!("rule.id={}", encode_str(&rule.id)));
        parts.push(format!(
            "rule.description={}",
            encode_str(&rule.description)
        ));
        parts.push(format!(
            "rule.severity={}",
            encode_str(&rule.severity.to_string())
        ));
        parts.push(format!("rule.options={}", encode_str(&rule.options_digest)));
        parts.push(format!(
            "rule.capabilities={}",
            encode_str_list(&rule.requested_capabilities)
        ));
    }

    for capability in capabilities {
        parts.push(format!(
            "capability.name={}",
            encode_str(&capability.capability)
        ));
        parts.push(format!(
            "capability.language={}",
            encode_optional_language(capability.language)
        ));
        parts.push(format!(
            "capability.status={}",
            encode_str(capability_status_name(&capability.status))
        ));
        parts.push(format!(
            "capability.rules={}",
            encode_str_list(&capability.rules)
        ));
        push_policy_query_digest_parts(&mut parts, capability);
    }

    for check in setup_checks {
        parts.push(format!("setup.id={}", encode_str(&check.id)));
        parts.push(format!("setup.status={}", encode_str(&check.status)));
        parts.push(format!(
            "setup.capability={}",
            encode_str(&check.capability)
        ));
        parts.push(format!(
            "setup.language={}",
            encode_optional_language(check.language)
        ));
    }

    let part_refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    stable_hash(&part_refs)
}

fn push_policy_query_digest_parts(parts: &mut Vec<String>, capability: &PlannedCapability) {
    if let Some(version) = policy_query_version_for_capability(&capability.capability) {
        parts.push(format!(
            "capability.policy_query_version.{}={}",
            encode_str(&capability.capability),
            encode_str(version)
        ));
    }
}

fn policy_query_version_for_capability(capability: &str) -> Option<&'static str> {
    match capability {
        "events" | "calls" | "control_flow" | "dataflow" => Some(POLICY_QUERY_VERSION),
        _ => None,
    }
}

fn encode_str(value: &str) -> String {
    format!("{}:{value}", value.len())
}

fn encode_str_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| encode_str(value))
        .collect::<Vec<_>>()
        .join("|")
}

fn encode_optional_language(language: Option<Language>) -> String {
    language.map(language_name).unwrap_or("none").to_string()
}

fn language_name(language: Language) -> &'static str {
    match language {
        Language::Go => "go",
        Language::TypeScript => "typescript",
        Language::Tsx => "tsx",
        Language::JavaScript => "javascript",
        Language::Jsx => "jsx",
        Language::Unknown => "unknown",
        _ => "unknown",
    }
}

fn capability_status_name(status: &CapabilitySupportStatus) -> &'static str {
    match status {
        CapabilitySupportStatus::Supported => "Supported",
        CapabilitySupportStatus::Unsupported => "Unsupported",
        CapabilitySupportStatus::SetupMissing => "SetupMissing",
    }
}

fn capability_status_json(status: &CapabilitySupportStatus) -> &'static str {
    match status {
        CapabilitySupportStatus::Supported => "supported",
        CapabilitySupportStatus::Unsupported => "unsupported",
        CapabilitySupportStatus::SetupMissing => "setup_missing",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        Capabilities, Rule, RuleKind, RuleMeta, RuleOptions, run_rules_with_capability_support,
    };
    use crate::diagnostics::{Severity, TextRange as DiagnosticRange};
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Clone, Copy)]
    struct PlanRule {
        id: &'static str,
        description: &'static str,
        severity: Severity,
        capabilities: Capabilities,
        behavior: PlanRuleBehavior,
    }

    #[derive(Clone, Copy)]
    enum PlanRuleBehavior {
        Normal,
        MetaPanic,
        CapabilitiesPanic,
    }

    impl PlanRule {
        fn into_rule(self) -> Rule {
            let meta_rule = self;
            let capabilities_rule = self;
            Rule::from_parts(
                move || meta_rule.meta(),
                move || capabilities_rule.capabilities(),
                |_db, _ctx| Ok(()),
            )
        }

        fn meta(self) -> RuleMeta {
            if matches!(self.behavior, PlanRuleBehavior::MetaPanic) {
                panic!("intentional plan-time metadata panic");
            }

            RuleMeta {
                id: self.id.to_string(),
                description: self.description.to_string(),
                severity: self.severity,
                kind: RuleKind::Check,
            }
        }

        fn capabilities(self) -> Capabilities {
            if matches!(self.behavior, PlanRuleBehavior::CapabilitiesPanic) {
                panic!("intentional plan-time capability panic");
            }

            self.capabilities
        }
    }

    fn rule(
        id: &'static str,
        description: &'static str,
        severity: Severity,
        capabilities: Capabilities,
    ) -> Rule {
        PlanRule {
            id,
            description,
            severity,
            capabilities,
            behavior: PlanRuleBehavior::Normal,
        }
        .into_rule()
    }

    fn rule_with_behavior(
        id: &'static str,
        description: &'static str,
        severity: Severity,
        capabilities: Capabilities,
        behavior: PlanRuleBehavior,
    ) -> Rule {
        PlanRule {
            id,
            description,
            severity,
            capabilities,
            behavior,
        }
        .into_rule()
    }

    #[test]
    fn analysis_plan_schema_is_versioned() {
        assert_eq!(ANALYSIS_PLAN_SCHEMA, "analysis-plan-v1");

        let plan = AnalysisPlan::empty();
        assert!(plan.rules().is_empty());
        assert!(plan.capabilities().is_empty());
        assert!(plan.setup_checks().is_empty());
    }

    #[test]
    fn analysis_plan_merges_enabled_rule_capabilities_deterministically() {
        let first_rules = vec![
            rule(
                "local/zeta",
                "Zeta rule",
                Severity::Warn,
                Capabilities::new().imports().cfg(),
            ),
            rule(
                "local/alpha",
                "Alpha rule",
                Severity::Error,
                Capabilities::new().syntax().imports(),
            ),
        ];
        let second_rules = vec![first_rules[1].clone(), first_rules[0].clone()];
        let enabled = BTreeSet::from(["local/*".to_string()]);
        let mut options = BTreeMap::new();
        options.insert(
            "local/alpha".to_string(),
            RuleOptions {
                max: Some(3),
                ..RuleOptions::default()
            },
        );

        let first = AnalysisPlan::from_rules(&first_rules, Some(&enabled), &options);
        let second = AnalysisPlan::from_rules(&second_rules, Some(&enabled), &options);

        assert_eq!(first.digest(), second.digest());
        assert_eq!(
            first
                .rules()
                .iter()
                .map(|rule| rule.id.as_str())
                .collect::<Vec<_>>(),
            vec!["local/alpha", "local/zeta"]
        );
        assert_eq!(
            first
                .capabilities()
                .iter()
                .map(|capability| capability.capability.as_str())
                .collect::<Vec<_>>(),
            vec!["cfg", "imports", "syntax"]
        );
    }

    #[test]
    fn analysis_plan_uses_run_rules_enabled_filter_semantics() {
        let rules = vec![
            rule(
                "local/selected",
                "Selected rule",
                Severity::Warn,
                Capabilities::new().imports(),
            ),
            rule(
                "other/skipped",
                "Skipped rule",
                Severity::Warn,
                Capabilities::new().cfg(),
            ),
        ];
        let enabled = BTreeSet::from(["local/*".to_string()]);

        let plan = AnalysisPlan::from_rules(&rules, Some(&enabled), &BTreeMap::new());

        assert_eq!(plan.rules().len(), 1);
        assert_eq!(plan.rules()[0].id, "local/selected");
        assert!(plan.diagnostics().is_empty());
    }

    #[test]
    fn rule_plan_inputs_can_be_narrowed_by_only_rule_pattern() {
        let rules = vec![
            rule(
                "local/selected",
                "Selected rule",
                Severity::Warn,
                Capabilities::new().imports(),
            ),
            rule(
                "local/skipped",
                "Skipped rule",
                Severity::Warn,
                Capabilities::new().cfg(),
            ),
        ];
        let enabled = BTreeSet::from(["local/*".to_string()]);
        let mut inputs = RulePlanInputs::collect(&rules, Some(&enabled));

        inputs.retain_rule_pattern(Some("local/selected"));

        assert_eq!(
            inputs.exact_rule_ids(),
            BTreeSet::from(["local/selected".to_string()])
        );
        let plan = AnalysisPlan::from_inputs(&inputs, &BTreeMap::new());
        assert_eq!(plan.rules().len(), 1);
        assert_eq!(plan.rules()[0].id, "local/selected");
        assert_eq!(
            plan.capabilities()
                .iter()
                .map(|capability| capability.capability.as_str())
                .collect::<Vec<_>>(),
            vec!["imports"]
        );
    }

    #[test]
    fn analysis_plan_reports_reserved_capabilities_as_diagnostics() {
        let rules = vec![rule(
            "local/needs-metrics",
            "Needs metrics",
            Severity::Warn,
            Capabilities::new().test_suite_metrics().cfg().dataflow(),
        )];

        let plan = AnalysisPlan::from_rules(&rules, None, &BTreeMap::new());
        let diagnostics = plan.diagnostics();

        assert_eq!(
            plan.support_view().status_for("test_suite_metrics"),
            Some(crate::core::CapabilitySupportStatus::Unsupported)
        );
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.rule_id == "polint/capability"
                && diagnostic.file == "<workspace>"
                && diagnostic.range == DiagnosticRange::point(1, 1)
                && diagnostic
                    .message
                    .contains("Rule `local/needs-metrics` requested unsupported capability `cfg`.")
                && diagnostic
                    .help
                    .as_deref()
                    .is_some_and(|help| help.contains("docs/facts/capability-plans.md"))
                && diagnostic.evidence.iter().any(|evidence| {
                    evidence.label == "rule" && evidence.value == "local/needs-metrics"
                })
                && diagnostic
                    .evidence
                    .iter()
                    .any(|evidence| evidence.label == "capability" && evidence.value == "cfg")
        }));
        assert!(plan.support_view().entries().iter().any(|entry| {
            entry.capability == "test_suite_metrics"
                && entry
                    .hint
                    .as_deref()
                    .is_some_and(|hint| hint.contains("Use go_tests for current Go test evidence"))
        }));
        assert_eq!(
            plan.support_view().status_for("dataflow"),
            Some(crate::core::CapabilitySupportStatus::Supported)
        );
    }

    #[test]
    fn reserved_capabilities_remain_unsupported() {
        let accumulator = support_for("call_graph");

        assert_eq!(accumulator.status, CapabilitySupportStatus::Unsupported);
        assert_eq!(
            accumulator.reason.as_deref(),
            Some("Capability is not currently supported.")
        );
        assert_eq!(
            accumulator.docs_path.as_deref(),
            Some("docs/facts/capability-plans.md")
        );

        let plan = AnalysisPlan::from_capability_names_for_test(&["call_graph"]);
        let capability = plan
            .capabilities()
            .iter()
            .find(|capability| capability.capability == "call_graph")
            .unwrap_or_else(|| panic!("expected call_graph capability row: {plan:#?}"));
        assert_eq!(capability.status, CapabilitySupportStatus::Unsupported);
        assert_eq!(
            capability.reason.as_deref(),
            Some("Capability is not currently supported.")
        );
        assert_eq!(
            capability.docs_path.as_deref(),
            Some("docs/facts/capability-plans.md")
        );
    }

    #[test]
    fn policy_capabilities_report_phase_support_boundaries() {
        let plan = AnalysisPlan::from_capability_names_for_test(&[
            "events",
            "calls",
            "control_flow",
            "dataflow",
        ]);

        for capability in [
            "calls",
            "control_flow",
            "dataflow",
            "events",
            "module_graph",
            "references",
            "resolved_imports",
            "symbols",
        ] {
            let planned = plan
                .capabilities()
                .iter()
                .find(|planned| planned.capability == capability)
                .unwrap_or_else(|| panic!("missing planned capability {capability}: {plan:#?}"));
            assert_eq!(planned.status, CapabilitySupportStatus::Supported);
            assert_eq!(planned.docs_path.as_deref(), None);
            assert_eq!(
                planned.rules,
                vec!["test/requested-capabilities".to_string()]
            );
        }

        let events_only = AnalysisPlan::from_capability_names_for_test(&["events"]);
        assert_eq!(
            events_only
                .capabilities()
                .iter()
                .map(|planned| planned.capability.as_str())
                .collect::<Vec<_>>(),
            vec!["events"],
            "events-only rules should stay on the lightweight syntax path"
        );

        let control_flow_only = AnalysisPlan::from_capability_names_for_test(&["control_flow"]);
        assert_eq!(
            control_flow_only
                .capabilities()
                .iter()
                .map(|planned| planned.capability.as_str())
                .collect::<Vec<_>>(),
            vec![
                "control_flow",
                "module_graph",
                "references",
                "resolved_imports",
                "symbols",
            ],
            "control-flow rules need the same graph prerequisites as call rules"
        );

        let diagnostics = plan.diagnostics();
        for capability in ["events", "calls", "control_flow", "dataflow"] {
            assert!(
                diagnostics.iter().all(|diagnostic| {
                    !diagnostic.evidence.iter().any(|evidence| {
                        evidence.label == "capability" && evidence.value == capability
                    })
                }),
                "supported capability {capability} should not emit diagnostics: {diagnostics:#?}"
            );
        }
    }

    #[test]
    fn policy_capability_dependencies_match_rules_for_setup_gates() {
        let rules = vec![rule(
            "local/no-secret-flow",
            "No secret flow",
            Severity::Error,
            Capabilities::new().dataflow(),
        )];
        let plan = AnalysisPlan::from_rules(&rules, None, &BTreeMap::new());
        let file = SourceFile::new(
            crate::core::FileId::from_raw(0),
            "src/app.go".into(),
            "src/app.go".to_string(),
            Language::Go,
            "".into(),
            "hash".to_string(),
        );
        let files = [&file];

        assert_eq!(
            plan.rules_for_capability_matching_files("references", &files),
            vec!["local/no-secret-flow".to_string()]
        );
    }

    #[test]
    fn policy_query_version_digest_parts_apply_only_to_policy_capabilities() {
        for capability in ["events", "calls", "control_flow", "dataflow"] {
            assert_eq!(
                policy_query_version_for_capability(capability),
                Some(POLICY_QUERY_VERSION)
            );
        }
        for capability in ["resolved_imports", "symbols", "cfg", "call_graph"] {
            assert_eq!(policy_query_version_for_capability(capability), None);
        }

        let mut parts = Vec::new();
        push_policy_query_digest_parts(
            &mut parts,
            &PlannedCapability {
                capability: "dataflow".to_string(),
                language: None,
                status: CapabilitySupportStatus::Supported,
                rules: vec!["local/rule".to_string()],
                reason: None,
                hint: None,
                docs_path: None,
            },
        );

        assert_eq!(parts.len(), 1);
        assert!(parts[0].contains("capability.policy_query_version."));
        assert!(parts[0].contains(POLICY_QUERY_VERSION));
    }

    #[test]
    fn analysis_plan_supports_derived_metric_capabilities() {
        let rules = vec![rule(
            "local/quality-score",
            "Quality score",
            Severity::Warn,
            Capabilities::new()
                .file_metrics()
                .function_metrics()
                .complexity_metrics(),
        )];

        let plan = AnalysisPlan::from_rules(&rules, None, &BTreeMap::new());

        assert!(plan.diagnostics().is_empty());
        assert_eq!(
            plan.capabilities()
                .iter()
                .map(|capability| (capability.capability.as_str(), capability.status.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("complexity_metrics", CapabilitySupportStatus::Supported),
                ("file_metrics", CapabilitySupportStatus::Supported),
                ("function_metrics", CapabilitySupportStatus::Supported),
            ]
        );
        assert!(plan.requests_any_capability(&["function_metrics"]));
    }

    #[test]
    fn analysis_plan_supports_module_relationship_capabilities() {
        let plan =
            AnalysisPlan::from_capability_names_for_test(&["resolved_imports", "module_graph"]);
        let capabilities = plan.capabilities();

        assert_eq!(
            capabilities
                .iter()
                .map(|capability| capability.capability.as_str())
                .collect::<Vec<_>>(),
            vec!["module_graph", "resolved_imports"]
        );

        for capability in capabilities {
            assert_eq!(
                capability.status,
                crate::core::CapabilitySupportStatus::Supported
            );
            assert_eq!(capability.reason, None);
            assert_eq!(capability.hint, None);
            assert_eq!(capability.docs_path, None);
        }

        assert_eq!(
            plan.support_view().status_for("resolved_imports"),
            Some(crate::core::CapabilitySupportStatus::Supported)
        );
        assert_eq!(
            plan.support_view().status_for("module_graph"),
            Some(crate::core::CapabilitySupportStatus::Supported)
        );
        assert!(plan.diagnostics().is_empty());
    }

    #[test]
    fn analysis_plan_supports_symbol_capabilities() {
        let plan = AnalysisPlan::from_capability_names_for_test(&["symbols", "references"]);
        let capabilities = plan.capabilities();

        assert_eq!(
            capabilities
                .iter()
                .map(|capability| capability.capability.as_str())
                .collect::<Vec<_>>(),
            vec!["references", "symbols"]
        );

        for capability in capabilities {
            assert_eq!(
                capability.status,
                crate::core::CapabilitySupportStatus::Supported
            );
            assert_eq!(capability.reason, None);
            assert_eq!(capability.hint, None);
            assert_eq!(capability.docs_path, None);
        }

        assert_eq!(
            plan.support_view().status_for("symbols"),
            Some(crate::core::CapabilitySupportStatus::Supported)
        );
        assert_eq!(
            plan.support_view().status_for("references"),
            Some(crate::core::CapabilitySupportStatus::Supported)
        );
        assert!(plan.diagnostics().is_empty());

        let report = plan.explain_report();
        assert!(report.capabilities.iter().any(|capability| {
            capability.name == "symbols" && capability.status == "supported"
        }));
        assert!(report.capabilities.iter().any(|capability| {
            capability.name == "references" && capability.status == "supported"
        }));
    }

    #[test]
    fn references_capability_requires_symbol_identity() {
        let capabilities = Capabilities::new().references();
        assert!(capabilities.references);
        assert!(capabilities.symbols);
        assert_eq!(
            capabilities.requested_names().collect::<Vec<_>>(),
            vec!["symbols", "references"]
        );

        let rules = vec![rule(
            "local/needs-references",
            "Needs references",
            Severity::Warn,
            capabilities,
        )];
        let plan = AnalysisPlan::from_rules(&rules, None, &BTreeMap::new());

        assert_eq!(
            plan.rules()[0].requested_capabilities,
            vec!["symbols".to_string(), "references".to_string()]
        );
        assert_eq!(
            plan.capabilities()
                .iter()
                .map(|capability| (capability.capability.as_str(), capability.status.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("references", CapabilitySupportStatus::Supported),
                ("symbols", CapabilitySupportStatus::Supported),
            ]
        );

        let symbols_only = AnalysisPlan::from_rules(
            &[rule(
                "local/needs-references",
                "Needs references",
                Severity::Warn,
                Capabilities::new().symbols(),
            )],
            None,
            &BTreeMap::new(),
        );
        assert_ne!(
            plan.digest(),
            symbols_only.digest(),
            "references must add symbol identity plus reference facts to the plan digest"
        );
    }

    #[test]
    fn analysis_plan_reports_setup_missing_capabilities_as_diagnostics() {
        let plan = AnalysisPlan::finish(
            vec![PlannedRule {
                id: "local/needs-coverage".to_string(),
                description: "Needs coverage".to_string(),
                severity: Severity::Warn,
                requested_capabilities: vec!["coverage_facts".to_string()],
                files: Vec::new(),
                allow_files: Vec::new(),
                options_digest: "options".to_string(),
            }],
            vec![PlannedCapability {
                capability: "coverage_facts".to_string(),
                language: None,
                status: crate::core::CapabilitySupportStatus::SetupMissing,
                rules: vec!["local/needs-coverage".to_string()],
                reason: Some("Coverage report was not configured.".to_string()),
                hint: Some("Set coverage report paths in .polint.toml.".to_string()),
                docs_path: Some("docs/facts/capability-plans.md".to_string()),
            }],
            Vec::new(),
        );

        let diagnostics = plan.diagnostics();

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule_id, "polint/capability");
        assert!(diagnostics[0].message.contains("required setup is missing"));
        assert!(
            diagnostics[0]
                .evidence
                .iter()
                .any(|evidence| evidence.label == "status" && evidence.value == "setup_missing")
        );
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .is_some_and(|help| help.contains("docs/facts/capability-plans.md"))
        );
    }

    #[test]
    fn analysis_plan_explain_report_json_fields() {
        let rules = vec![
            rule(
                "local/needs-cfg",
                "Needs CFG",
                Severity::Warn,
                Capabilities::new().cfg(),
            ),
            rule(
                "local/needs-imports",
                "Needs imports",
                Severity::Error,
                Capabilities::new().imports(),
            ),
        ];
        let plan = AnalysisPlan::from_rules(&rules, None, &BTreeMap::new());

        let value = serde_json::to_value(plan.explain_report()).unwrap();

        assert_eq!(value["schema"], "analysis-plan-v1");
        assert_eq!(value["digest"], plan.digest());
        assert_eq!(value["rules"][0]["id"], "local/needs-cfg");
        assert_eq!(value["rules"][0]["description"], "Needs CFG");
        assert_eq!(value["rules"][0]["severity"], "warn");
        assert_eq!(
            value["rules"][0]["capabilities"],
            serde_json::json!(["cfg"])
        );
        assert_eq!(value["capabilities"][0]["name"], "cfg");
        assert_eq!(value["capabilities"][0]["status"], "unsupported");
        assert_eq!(
            value["capabilities"][0]["rules"],
            serde_json::json!(["local/needs-cfg"])
        );
        assert_eq!(value["setup_checks"], serde_json::json!([]));
    }

    #[test]
    fn analysis_plan_explain_report_human_sections() {
        let rules = vec![rule(
            "local/needs-imports",
            "Needs imports",
            Severity::Warn,
            Capabilities::new().imports(),
        )];
        let plan = AnalysisPlan::from_rules(&rules, None, &BTreeMap::new());

        let human = plan.explain_report().to_human();

        assert!(human.contains("Analysis plan"));
        assert!(human.contains("Digest: "));
        assert!(human.contains("Rules"));
        assert!(human.contains("local/needs-imports"));
        assert!(human.contains("Capabilities"));
        assert!(human.contains("imports"));
        assert!(human.contains("Setup checks"));
    }

    #[test]
    fn analysis_plan_contains_rule_metadata_and_capability_panics() {
        let rules = vec![
            rule_with_behavior(
                "local/meta-panic",
                "Metadata panic",
                Severity::Warn,
                Capabilities::new().syntax(),
                PlanRuleBehavior::MetaPanic,
            ),
            rule_with_behavior(
                "local/capability-panic",
                "Capability panic",
                Severity::Error,
                Capabilities::new().imports(),
                PlanRuleBehavior::CapabilitiesPanic,
            ),
        ];

        let inputs = RulePlanInputs::collect(&rules, None);
        let plan = AnalysisPlan::from_inputs(&inputs, &BTreeMap::new());
        let mut diagnostics = inputs.diagnostics();
        diagnostics.extend(plan.diagnostics());

        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.rule_id == "internal/unknown"
                    && diagnostic.file == "<workspace>"
                    && diagnostic.message.contains("rule metadata panicked")
            }),
            "{diagnostics:#?}"
        );
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.rule_id == "polint/capability"
                    && diagnostic.file == "<workspace>"
                    && diagnostic
                        .message
                        .contains("Rule `local/capability-panic` capability collection panicked")
            }),
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn analysis_plan_support_view_is_passed_to_rules() {
        let db = crate::core::AnalysisDb::new();
        let rules = vec![rule(
            "local/reader",
            "Reader",
            Severity::Warn,
            Capabilities::new().imports(),
        )];
        let plan = AnalysisPlan::from_rules(&rules, None, &BTreeMap::new());

        let diagnostics = run_rules_with_capability_support(
            &db,
            &rules,
            &BTreeMap::new(),
            None,
            false,
            plan.support_view(),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn a_rule_that_matches_nothing_appears_in_the_summary() {
        let rules = vec![rule(
            "local/silent",
            "Never matches",
            Severity::Warn,
            Capabilities::new().imports(),
        )];
        let plan = AnalysisPlan::from_rules(&rules, None, &BTreeMap::new());
        let diagnostics = plan.diagnostics();
        let rows = plan.rule_execution_rows(
            &BTreeMap::new(),
            &["src/a.ts", "src/b.ts"],
            &diagnostics,
            &BTreeMap::new(),
        );
        let row = rows
            .iter()
            .find(|row| row.rule_id == "local/silent")
            .expect("silent rule row");
        assert!(row.planned);
        assert!(row.capabilities_ok);
        assert!(row.files_in_scope > 0);
        assert_eq!(row.diagnostics_emitted, 0);
        assert_eq!(row.skipped_reason, None);
    }

    #[test]
    fn a_rule_that_matched_nothing_reports_zero_observed_events_and_an_analyzed_outcome() {
        let rules = vec![rule(
            "local/silent",
            "Never matches",
            Severity::Warn,
            Capabilities::new().imports(),
        )];
        let plan = AnalysisPlan::from_rules(&rules, None, &BTreeMap::new());
        let observed = BTreeMap::from([("local/silent".to_string(), 0)]);

        let rows = plan.rule_execution_rows(&BTreeMap::new(), &["src/a.ts"], &[], &observed);
        let row = rows
            .iter()
            .find(|row| row.rule_id == "local/silent")
            .expect("silent rule row");

        assert_eq!(row.outcome, crate::diagnostics::RULE_OUTCOME_ANALYZED);
        assert_eq!(row.observed_events, 0);
        assert!(row.blocking_providers.is_empty());
    }

    #[test]
    fn a_rule_that_examined_operations_reports_them() {
        let rules = vec![rule(
            "local/busy",
            "Examines operations",
            Severity::Warn,
            Capabilities::new().imports(),
        )];
        let plan = AnalysisPlan::from_rules(&rules, None, &BTreeMap::new());
        let observed = BTreeMap::from([("local/busy".to_string(), 13)]);

        let rows = plan.rule_execution_rows(&BTreeMap::new(), &["src/a.ts"], &[], &observed);

        assert_eq!(rows[0].observed_events, 13);
    }

    #[test]
    fn a_runtime_blocked_rule_names_the_providers_that_blocked_it() {
        let rules = vec![rule(
            "local/blocked",
            "Blocked at runtime",
            Severity::Warn,
            Capabilities::new().imports(),
        )];
        let plan = AnalysisPlan::from_rules(&rules, None, &BTreeMap::new());
        let diagnostics = vec![
            Diagnostic::error(
                "polint/capability",
                "<workspace>",
                crate::diagnostics::TextRange::point(1, 1),
                "Rule `local/blocked` requested capability `control_flow`, but provider closure did not succeed.",
            )
            .with_evidence("rule", "local/blocked")
            .with_evidence("blockers", "polint.go.semantic,polint.refined_calls"),
        ];

        let rows = plan.rule_execution_rows(
            &BTreeMap::new(),
            &["src/a.ts"],
            &diagnostics,
            &BTreeMap::new(),
        );

        assert_eq!(
            rows[0].outcome,
            crate::diagnostics::RULE_OUTCOME_CAPABILITY_BLOCKED
        );
        assert_eq!(
            rows[0].blocking_providers,
            vec![
                "polint.go.semantic".to_string(),
                "polint.refined_calls".to_string()
            ]
        );
    }

    #[test]
    fn budget_rows_come_from_the_diagnostics_that_reported_them() {
        let diagnostics = vec![
            Diagnostic::warning(
                "polint/resource-budget",
                "<workspace>",
                crate::diagnostics::TextRange::point(1, 1),
                "control-flow dominance materialisation bounded",
            )
            .with_evidence(
                crate::diagnostics::BUDGET_EVIDENCE_LABEL,
                "cfg_dominance_pairs",
            )
            .with_evidence(crate::diagnostics::BUDGET_STATUS_EVIDENCE_LABEL, "exceeded"),
            Diagnostic::warning(
                "polint/internal",
                "<workspace>",
                crate::diagnostics::TextRange::point(1, 1),
                "not a budget",
            ),
        ];

        let rows = crate::diagnostics::RunSummary::budget_rows(&diagnostics);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].budget, "cfg_dominance_pairs");
        assert_eq!(rows[0].status, "exceeded");
        assert_eq!(rows[0].reported_by, "polint/resource-budget");
    }

    #[test]
    fn a_run_without_budget_trips_reports_no_budget_rows() {
        assert!(crate::diagnostics::RunSummary::budget_rows(&[]).is_empty());
    }

    #[test]
    fn a_rule_dropped_by_capability_planning_reports_why() {
        let rules = vec![rule(
            "local/needs-cfg",
            "Needs cfg",
            Severity::Warn,
            Capabilities::new().cfg(),
        )];
        let plan = AnalysisPlan::from_rules(&rules, None, &BTreeMap::new());
        let diagnostics = plan.diagnostics();
        let rows = plan.rule_execution_rows(
            &BTreeMap::new(),
            &["src/a.ts"],
            &diagnostics,
            &BTreeMap::new(),
        );
        let row = rows
            .iter()
            .find(|row| row.rule_id == "local/needs-cfg")
            .expect("capability-dropped row");
        assert!(!row.planned);
        assert!(!row.capabilities_ok);
        assert_eq!(row.diagnostics_emitted, 0);
        let reason = row.skipped_reason.as_deref().expect("skipped_reason");
        assert!(!reason.is_empty());
        assert!(reason.contains("unsupported capability `cfg`"), "{reason}");
    }

    #[test]
    fn a_rule_whose_scope_excludes_everything_reports_zero_files() {
        let rules = vec![rule(
            "local/scoped",
            "Scoped away",
            Severity::Warn,
            Capabilities::new().imports(),
        )];
        let mut options = BTreeMap::new();
        options.insert(
            "local/scoped".to_string(),
            RuleOptions {
                files: vec!["does-not-exist/**".to_string()],
                ..RuleOptions::default()
            },
        );
        let plan = AnalysisPlan::from_rules(&rules, None, &options);
        let diagnostics = plan.diagnostics();
        let rows = plan.rule_execution_rows(
            &options,
            &["src/a.ts", "src/b.ts"],
            &diagnostics,
            &BTreeMap::new(),
        );
        let row = rows
            .iter()
            .find(|row| row.rule_id == "local/scoped")
            .expect("scoped rule row");
        assert_eq!(row.files_in_scope, 0);
        assert!(row.planned);
        assert!(row.capabilities_ok);
        assert_eq!(row.diagnostics_emitted, 0);
    }
}
