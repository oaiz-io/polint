use super::ProviderManifest;
use super::incremental::{Digest, PrecisionTier};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderOutcomeStatus {
    Succeeded,
    Failed,
    DependencyBlocked,
    Unsupported,
    SetupMissing,
    PlannedAbsent,
    /// The run's resource envelope was exhausted before this provider ran.
    BudgetExceeded,
}
impl ProviderOutcomeStatus {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::DependencyBlocked => "dependency_blocked",
            Self::Unsupported => "unsupported",
            Self::SetupMissing => "setup_missing",
            Self::PlannedAbsent => "planned_absent",
            Self::BudgetExceeded => "budget_exceeded",
        }
    }
    pub(crate) fn decode(label: &str) -> Option<Self> {
        match label {
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "dependency_blocked" => Some(Self::DependencyBlocked),
            "unsupported" => Some(Self::Unsupported),
            "setup_missing" => Some(Self::SetupMissing),
            "planned_absent" => Some(Self::PlannedAbsent),
            "budget_exceeded" => Some(Self::BudgetExceeded),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderFailureStage {
    Planning,
    Dependency,
    Setup,
    Execution,
    Validation,
}
impl ProviderFailureStage {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Dependency => "dependency",
            Self::Setup => "setup",
            Self::Execution => "execution",
            Self::Validation => "validation",
        }
    }
    pub(crate) fn decode(label: &str) -> Option<Self> {
        match label {
            "planning" => Some(Self::Planning),
            "dependency" => Some(Self::Dependency),
            "setup" => Some(Self::Setup),
            "execution" => Some(Self::Execution),
            "validation" => Some(Self::Validation),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderFailureReason {
    NotSelected,
    DependencyUnavailable,
    Unsupported,
    SetupMissing,
    ExecutionFailed,
    ValidationRejected,
    /// Live RSS crossed the run's memory ceiling (see `analysis_kernel::resource`).
    MemoryCeiling,
}
impl ProviderFailureReason {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::NotSelected => "not_selected",
            Self::DependencyUnavailable => "dependency_unavailable",
            Self::Unsupported => "unsupported",
            Self::SetupMissing => "setup_missing",
            Self::ExecutionFailed => "execution_failed",
            Self::ValidationRejected => "validation_rejected",
            Self::MemoryCeiling => "memory_ceiling",
        }
    }
    pub(crate) fn decode(label: &str) -> Option<Self> {
        match label {
            "not_selected" => Some(Self::NotSelected),
            "dependency_unavailable" => Some(Self::DependencyUnavailable),
            "unsupported" => Some(Self::Unsupported),
            "setup_missing" => Some(Self::SetupMissing),
            "execution_failed" => Some(Self::ExecutionFailed),
            "validation_rejected" => Some(Self::ValidationRejected),
            "memory_ceiling" => Some(Self::MemoryCeiling),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderOutputIdentity {
    pub(crate) provider_id: String,
    pub(crate) provider_version: String,
    pub(crate) schema_version: String,
    pub(crate) output_digest: Digest,
    pub(crate) precision: PrecisionTier,
}
impl ProviderOutputIdentity {
    pub(crate) fn from_manifest(
        manifest: &ProviderManifest,
        output_digest: Digest,
        precision: PrecisionTier,
    ) -> Self {
        Self {
            provider_id: manifest.id.to_string(),
            provider_version: manifest.provider_version().to_string(),
            schema_version: manifest.primary_schema_label(),
            output_digest,
            precision,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderOutcome {
    pub(crate) provider_id: String,
    pub(crate) status: ProviderOutcomeStatus,
    pub(crate) output_identity: Option<ProviderOutputIdentity>,
    pub(crate) failure_stage: Option<ProviderFailureStage>,
    pub(crate) failure_reason: Option<ProviderFailureReason>,
    pub(crate) blockers: Vec<String>,
}
impl ProviderOutcome {
    fn succeeded(provider_id: String, output_identity: ProviderOutputIdentity) -> Self {
        Self {
            provider_id,
            status: ProviderOutcomeStatus::Succeeded,
            output_identity: Some(output_identity),
            failure_stage: None,
            failure_reason: None,
            blockers: Vec::new(),
        }
    }
    fn non_success(
        provider_id: String,
        status: ProviderOutcomeStatus,
        failure_stage: ProviderFailureStage,
        failure_reason: ProviderFailureReason,
        blockers: Vec<String>,
    ) -> Result<Self, ProviderOutcomeError> {
        let blockers = sorted_unique(blockers);
        let valid = match status {
            ProviderOutcomeStatus::Succeeded => false,
            // A resource stop is recorded before the provider runs, so it never
            // carries dependency blockers.
            ProviderOutcomeStatus::BudgetExceeded => {
                failure_stage == ProviderFailureStage::Execution
                    && failure_reason == ProviderFailureReason::MemoryCeiling
                    && blockers.is_empty()
            }
            ProviderOutcomeStatus::Failed => {
                matches!(
                    (failure_stage, failure_reason),
                    (
                        ProviderFailureStage::Execution,
                        ProviderFailureReason::ExecutionFailed
                    ) | (
                        ProviderFailureStage::Validation,
                        ProviderFailureReason::ValidationRejected
                    )
                ) && blockers.is_empty()
            }
            ProviderOutcomeStatus::DependencyBlocked => {
                failure_stage == ProviderFailureStage::Dependency
                    && failure_reason == ProviderFailureReason::DependencyUnavailable
                    && !blockers.is_empty()
            }
            ProviderOutcomeStatus::Unsupported => {
                failure_stage == ProviderFailureStage::Setup
                    && failure_reason == ProviderFailureReason::Unsupported
                    && blockers.is_empty()
            }
            ProviderOutcomeStatus::SetupMissing => {
                failure_stage == ProviderFailureStage::Setup
                    && failure_reason == ProviderFailureReason::SetupMissing
                    && blockers.is_empty()
            }
            ProviderOutcomeStatus::PlannedAbsent => {
                failure_stage == ProviderFailureStage::Planning
                    && failure_reason == ProviderFailureReason::NotSelected
                    && blockers.is_empty()
            }
        };
        if !valid {
            return Err(ProviderOutcomeError::InvalidTransition {
                provider_id,
                detail: "status, stage, reason, and blocker shape are inconsistent",
            });
        }
        Ok(Self {
            provider_id,
            status,
            output_identity: None,
            failure_stage: Some(failure_stage),
            failure_reason: Some(failure_reason),
            blockers,
        })
    }

    pub(crate) fn from_closed_parts(
        provider_id: String,
        status: ProviderOutcomeStatus,
        output_identity: Option<ProviderOutputIdentity>,
        failure_stage: Option<ProviderFailureStage>,
        failure_reason: Option<ProviderFailureReason>,
        blockers: Vec<String>,
    ) -> Result<Self, ProviderOutcomeError> {
        let invalid = |provider_id, detail| ProviderOutcomeError::InvalidTransition {
            provider_id,
            detail,
        };
        match (status, output_identity, failure_stage, failure_reason) {
            (ProviderOutcomeStatus::Succeeded, Some(identity), None, None)
                if blockers.is_empty() && identity.provider_id == provider_id =>
            {
                Ok(Self::succeeded(provider_id, identity))
            }
            (ProviderOutcomeStatus::Succeeded, _, _, _) => Err(invalid(
                provider_id,
                "successful outcome has inconsistent fields",
            )),
            (_, None, Some(stage), Some(reason)) => {
                let original_id = provider_id.clone();
                let canonical =
                    Self::non_success(provider_id, status, stage, reason, blockers.clone())?;
                (canonical.blockers == blockers)
                    .then_some(canonical)
                    .ok_or_else(|| invalid(original_id, "blockers must be sorted and unique"))
            }
            _ => Err(invalid(
                provider_id,
                "non-success outcome has inconsistent fields",
            )),
        }
    }
    #[cfg(test)]
    pub(crate) fn reject_validation_for_test(&mut self) {
        self.status = ProviderOutcomeStatus::Failed;
        self.output_identity = None;
        self.failure_stage = Some(ProviderFailureStage::Validation);
        self.failure_reason = Some(ProviderFailureReason::ValidationRejected);
    }
    #[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
    pub(crate) fn validation_display(&self) -> String {
        match (self.failure_stage, self.failure_reason) {
            (None, None) => self.status.label().to_string(),
            (Some(stage), Some(reason)) => format!(
                "{}:{}:{}",
                self.status.label(),
                stage.label(),
                reason.label()
            ),
            _ => unreachable!("provider outcome construction enforces paired failure details"),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ValidationDowngrades {
    global: bool,
    provider_ids: BTreeSet<String>,
}
impl ValidationDowngrades {
    #[cfg(test)]
    pub(crate) fn for_providers(provider_ids: impl IntoIterator<Item = String>) -> Self {
        Self {
            global: false,
            provider_ids: provider_ids.into_iter().collect(),
        }
    }
    pub(crate) fn extend_provider_ids(&mut self, provider_ids: impl IntoIterator<Item = String>) {
        self.provider_ids.extend(provider_ids);
    }
    pub(crate) fn mark_global(&mut self) {
        self.global = true;
    }
    pub(super) fn contains(&self, provider_id: &str) -> bool {
        self.global || self.provider_ids.contains(provider_id)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum AttemptState {
    PlannedAbsent,
    Pending,
    ProvisionalSuccess(ProviderOutputIdentity),
    Final(ProviderOutcome),
}
#[derive(Debug)]
pub(crate) struct ProviderOutcomeTracker {
    order: Vec<String>,
    dependencies: BTreeMap<String, Vec<String>>,
    states: BTreeMap<String, AttemptState>,
}
impl ProviderOutcomeTracker {
    pub(crate) fn from_manifests(
        manifests: &[ProviderManifest],
        selected: &BTreeSet<&str>,
    ) -> Result<Self, ProviderOutcomeError> {
        let order = manifests
            .iter()
            .map(|manifest| manifest.id.to_string())
            .collect::<Vec<_>>();
        let inventory = order.iter().map(String::as_str).collect::<BTreeSet<_>>();
        if let Some(unknown) = selected.iter().find(|id| !inventory.contains(**id)) {
            return Err(ProviderOutcomeError::UnknownProvider(
                (*unknown).to_string(),
            ));
        }
        let dependencies = manifests
            .iter()
            .map(|manifest| {
                let dependencies = hard_dependencies(manifest.id)
                    .iter()
                    .map(|id| (*id).to_string())
                    .collect::<Vec<_>>();
                if let Some(unknown) = dependencies
                    .iter()
                    .find(|id| !inventory.contains(id.as_str()))
                {
                    return Err(ProviderOutcomeError::UnknownDependency {
                        provider_id: manifest.id.to_string(),
                        dependency_id: unknown.clone(),
                    });
                }
                Ok((manifest.id.to_string(), sorted_unique(dependencies)))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let states = order
            .iter()
            .map(|provider_id| {
                let state = if selected.contains(provider_id.as_str()) {
                    AttemptState::Pending
                } else {
                    AttemptState::PlannedAbsent
                };
                (provider_id.clone(), state)
            })
            .collect();
        Ok(Self {
            order,
            dependencies,
            states,
        })
    }
    pub(crate) fn can_run(&self, provider_id: &str) -> Result<Vec<String>, ProviderOutcomeError> {
        let state = self.state(provider_id)?;
        if !matches!(state, AttemptState::Pending) {
            return Err(ProviderOutcomeError::InvalidTransition {
                provider_id: provider_id.to_string(),
                detail: "provider is not pending",
            });
        }
        Ok(self
            .dependencies
            .get(provider_id)
            .into_iter()
            .flatten()
            .filter(|dependency| !self.is_usable(dependency))
            .cloned()
            .collect())
    }
    pub(crate) fn record_success(
        &mut self,
        provider_id: &str,
        output_identity: ProviderOutputIdentity,
    ) -> Result<(), ProviderOutcomeError> {
        self.replace_pending(
            provider_id,
            AttemptState::ProvisionalSuccess(output_identity),
        )
    }
    /// Record a provider execution failure before later providers are considered.
    ///
    /// Keeping this transition in the tracker makes failed output identities
    /// unavailable to dependents in the same scheduling pass; sealing remains a
    /// final consistency check rather than the first point at which failure is
    /// observed.
    pub(crate) fn record_failure(
        &mut self,
        provider_id: &str,
        status: ProviderOutcomeStatus,
        failure_stage: ProviderFailureStage,
        failure_reason: ProviderFailureReason,
    ) -> Result<(), ProviderOutcomeError> {
        let outcome = ProviderOutcome::non_success(
            provider_id.to_string(),
            status,
            failure_stage,
            failure_reason,
            Vec::new(),
        )?;
        self.replace_pending(provider_id, AttemptState::Final(outcome))
    }

    #[cfg(test)]
    pub(crate) fn record_non_success(
        &mut self,
        provider_id: &str,
        status: ProviderOutcomeStatus,
        failure_stage: ProviderFailureStage,
        failure_reason: ProviderFailureReason,
    ) -> Result<(), ProviderOutcomeError> {
        let outcome = ProviderOutcome::non_success(
            provider_id.to_string(),
            status,
            failure_stage,
            failure_reason,
            Vec::new(),
        )?;
        self.replace_pending(provider_id, AttemptState::Final(outcome))
    }
    pub(crate) fn record_dependency_blocked(
        &mut self,
        provider_id: &str,
        blockers: Vec<String>,
    ) -> Result<(), ProviderOutcomeError> {
        let outcome = dependency_blocked_outcome(provider_id, blockers)?;
        self.replace_pending(provider_id, AttemptState::Final(outcome))
    }
    pub(crate) fn seal(
        mut self,
        validation: &ValidationDowngrades,
    ) -> Result<Vec<ProviderOutcome>, ProviderOutcomeError> {
        for provider_id in &self.order {
            if validation.contains(provider_id)
                && matches!(
                    self.states.get(provider_id),
                    Some(AttemptState::ProvisionalSuccess(_))
                )
            {
                let outcome = ProviderOutcome::non_success(
                    provider_id.clone(),
                    ProviderOutcomeStatus::Failed,
                    ProviderFailureStage::Validation,
                    ProviderFailureReason::ValidationRejected,
                    Vec::new(),
                )?;
                self.states
                    .insert(provider_id.clone(), AttemptState::Final(outcome));
            }
        }
        loop {
            let mut changed = false;
            for provider_id in &self.order {
                if !matches!(
                    self.states.get(provider_id),
                    Some(AttemptState::ProvisionalSuccess(_))
                ) {
                    continue;
                }
                let blockers = self
                    .dependencies
                    .get(provider_id)
                    .into_iter()
                    .flatten()
                    .filter(|dependency| !self.is_usable(dependency))
                    .cloned()
                    .collect::<Vec<_>>();
                if blockers.is_empty() {
                    continue;
                }
                let outcome = dependency_blocked_outcome(provider_id, blockers)?;
                self.states
                    .insert(provider_id.clone(), AttemptState::Final(outcome));
                changed = true;
            }
            if !changed {
                break;
            }
        }
        let mut outcomes = Vec::with_capacity(self.order.len());
        for provider_id in self.order {
            let state = self
                .states
                .remove(&provider_id)
                .expect("tracker inventory and state map stay aligned");
            let outcome = match state {
                AttemptState::PlannedAbsent => ProviderOutcome::non_success(
                    provider_id,
                    ProviderOutcomeStatus::PlannedAbsent,
                    ProviderFailureStage::Planning,
                    ProviderFailureReason::NotSelected,
                    Vec::new(),
                )?,
                AttemptState::Pending => {
                    return Err(ProviderOutcomeError::IncompleteProvider(provider_id));
                }
                AttemptState::ProvisionalSuccess(identity) => {
                    ProviderOutcome::succeeded(provider_id, identity)
                }
                AttemptState::Final(outcome) => outcome,
            };
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }
    fn state(&self, provider_id: &str) -> Result<&AttemptState, ProviderOutcomeError> {
        self.states
            .get(provider_id)
            .ok_or_else(|| ProviderOutcomeError::UnknownProvider(provider_id.to_string()))
    }
    fn replace_pending(
        &mut self,
        provider_id: &str,
        replacement: AttemptState,
    ) -> Result<(), ProviderOutcomeError> {
        let state = self
            .states
            .get_mut(provider_id)
            .ok_or_else(|| ProviderOutcomeError::UnknownProvider(provider_id.to_string()))?;
        if !matches!(state, AttemptState::Pending) {
            return Err(ProviderOutcomeError::InvalidTransition {
                provider_id: provider_id.to_string(),
                detail: "provider completion was recorded more than once or while absent",
            });
        }
        *state = replacement;
        Ok(())
    }
    fn is_usable(&self, provider_id: &str) -> bool {
        matches!(
            self.states.get(provider_id),
            Some(AttemptState::ProvisionalSuccess(_))
                | Some(AttemptState::Final(ProviderOutcome {
                    status: ProviderOutcomeStatus::Succeeded,
                    ..
                }))
        )
    }
    #[cfg(test)]
    fn for_test(order: &[&str], selected: &[&str], dependencies: &[(&str, &[&str])]) -> Self {
        let selected = selected.iter().copied().collect::<BTreeSet<_>>();
        let states = order
            .iter()
            .map(|provider_id| {
                (
                    (*provider_id).to_string(),
                    if selected.contains(provider_id) {
                        AttemptState::Pending
                    } else {
                        AttemptState::PlannedAbsent
                    },
                )
            })
            .collect();
        let mut dependency_map = order
            .iter()
            .map(|provider_id| ((*provider_id).to_string(), Vec::new()))
            .collect::<BTreeMap<_, _>>();
        for (provider_id, inputs) in dependencies {
            dependency_map.insert(
                (*provider_id).to_string(),
                sorted_unique(inputs.iter().map(|input| (*input).to_string()).collect()),
            );
        }
        Self {
            order: order.iter().map(|id| (*id).to_string()).collect(),
            dependencies: dependency_map,
            states,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProviderOutcomeError {
    UnknownProvider(String),
    UnknownDependency {
        provider_id: String,
        dependency_id: String,
    },
    InvalidTransition {
        provider_id: String,
        detail: &'static str,
    },
    IncompleteProvider(String),
}
impl fmt::Display for ProviderOutcomeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownProvider(provider_id) => {
                write!(formatter, "unknown provider `{provider_id}`")
            }
            Self::UnknownDependency {
                provider_id,
                dependency_id,
            } => write!(
                formatter,
                "provider `{provider_id}` depends on unknown provider `{dependency_id}`"
            ),
            Self::InvalidTransition {
                provider_id,
                detail,
            } => write!(
                formatter,
                "invalid provider outcome transition for `{provider_id}`: {detail}"
            ),
            Self::IncompleteProvider(provider_id) => {
                write!(
                    formatter,
                    "provider `{provider_id}` was not completed before sealing"
                )
            }
        }
    }
}
impl std::error::Error for ProviderOutcomeError {}
fn dependency_blocked_outcome(
    provider_id: &str,
    blockers: Vec<String>,
) -> Result<ProviderOutcome, ProviderOutcomeError> {
    ProviderOutcome::non_success(
        provider_id.to_string(),
        ProviderOutcomeStatus::DependencyBlocked,
        ProviderFailureStage::Dependency,
        ProviderFailureReason::DependencyUnavailable,
        blockers,
    )
}
fn sorted_unique(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}
pub(crate) fn hard_dependencies(provider_id: &str) -> &'static [&'static str] {
    const SRC: &str = "polint.source";
    const GO: &str = "polint.go.syntax";
    const TS: &str = "polint.ts.syntax";
    const MOD: &str = "polint.module_graph";
    const SYM: &str = "polint.symbol_graph";
    const TOP: &str = "polint.module_topology";
    const MIR: &str = "polint.semantic_mir";
    const CFG: &str = "polint.cfg";
    const CALLS: &str = "polint.calls";
    const GO_SEM: &str = "polint.go.semantic";
    const ID: &str = "polint.identity";
    const DOM: &str = "polint.abstract_domains";
    const SUM: &str = "polint.direct_summaries";
    const ENTRY: &str = "polint.entrypoints";
    const REACH: &str = "polint.reachability";
    const EXT: &str = "polint.extensions";
    const TVA: &str = "polint.type_value_alias";
    const GRAPH: &str = "polint.semantic_graph";
    const SOLVER: &str = "polint.solver";
    const REFINED: &str = "polint.refined_calls";
    const FLOW: &str = "polint.data_flow";
    match provider_id {
        "polint.source" => &[],
        "polint.go.syntax" | "polint.ts.syntax" => &[SRC],
        "polint.module_graph" => &[GO, TS],
        "polint.symbol_graph" => &[GO, MOD, TS],
        "polint.module_topology" => &[MOD, SYM],
        "polint.semantic_mir" => &[GO, TOP, SYM, TS],
        "polint.cfg" => &[GO, MIR, TS],
        "polint.calls" => &[CFG, GO, TOP, MIR, SYM, TS],
        "polint.go.semantic" => &[GO],
        "polint.identity" => &[CALLS, GO_SEM],
        "polint.abstract_domains" => &[CALLS, CFG, GO, TOP, MIR, SYM, TS],
        "polint.direct_summaries" => &[DOM, CALLS, CFG, GO, TOP, MIR, SYM, TS],
        "polint.entrypoints" => &[CALLS, CFG, GO, TOP, MIR, SYM, TS],
        "polint.reachability" => &[CALLS, ENTRY, ID, TOP, SYM],
        "polint.extensions" => &[],
        "polint.type_value_alias" => &[DOM, CALLS, CFG, SUM, ENTRY, EXT, GO, TOP, MIR, SYM, TS],
        "polint.semantic_graph" => &[
            DOM, CALLS, ENTRY, GO_SEM, GO, ID, TOP, REACH, MIR, SYM, TS, TVA,
        ],
        "polint.solver" => &[GO_SEM, GRAPH, TVA],
        "polint.refined_calls" => &[CALLS, SUM, ENTRY, EXT, SOLVER, TVA],
        "polint.data_flow" => &[CALLS, CFG, SUM, ENTRY, EXT, REFINED, MIR, TVA],
        "polint.evidence" => &[CALLS, CFG, FLOW, SUM, ENTRY, EXT, REFINED, MIR, TVA],
        "polint.metrics" => &[GO, TS],
        _ => &[],
    }
}
/// Projects sealed provider outcomes and their telemetry into report rows.
///
/// The kernel already decides every field; this is the projection that stops
/// the decision from being dropped after the run.
///
/// Providers whose ids the public output already names.
///
/// These back a public capability or already appear in `polint/capability`
/// blockers and `polint/go-semantic` diagnostics, so reporting their cost adds
/// no new vocabulary. Providers outside this set are internal fact families and
/// are named only when they failed, which is the one case a consumer must be
/// able to trace.
const PUBLICLY_NAMED_PROVIDERS: [&str; 9] = [
    "polint.evidence",
    "polint.go.semantic",
    "polint.go.syntax",
    "polint.identity",
    "polint.metrics",
    "polint.module_graph",
    "polint.refined_calls",
    "polint.symbol_graph",
    "polint.ts.syntax",
];

/// Only providers a consumer can act on are named: one that failed or was
/// blocked (so a blocked rule can be traced to it), one that reported counters
/// (so a measured stage can be read), and one whose id the public output
/// already uses (so its cost is attributable). A provider that quietly
/// succeeded outside that vocabulary, or that this run never selected, stays an
/// implementation detail.
pub(crate) fn provider_outcome_rows(
    outcomes: &[ProviderOutcome],
    telemetry: &[crate::analysis_kernel::incremental::ProviderTelemetry],
) -> Vec<crate::diagnostics::ProviderOutcomeRow> {
    outcomes
        .iter()
        .zip(telemetry)
        .filter(|(outcome, telemetry)| {
            if outcome.status == ProviderOutcomeStatus::PlannedAbsent {
                return false;
            }
            outcome.status != ProviderOutcomeStatus::Succeeded
                || !telemetry.counts.is_empty()
                || PUBLICLY_NAMED_PROVIDERS.contains(&outcome.provider_id.as_str())
        })
        .map(
            |(outcome, telemetry)| crate::diagnostics::ProviderOutcomeRow {
                provider_id: outcome.provider_id.clone(),
                status: outcome.status.label().to_string(),
                stage: outcome.failure_stage.map(|stage| stage.label().to_string()),
                reason: outcome
                    .failure_reason
                    .map(|reason| reason.label().to_string()),
                elapsed_ms: telemetry.elapsed_ms,
                blockers: outcome.blockers.clone(),
                cache: crate::diagnostics::ProviderCacheRow {
                    hits: telemetry.cache_stats.hits,
                    misses: telemetry.cache_stats.misses,
                    recomputes: telemetry.cache_stats.recomputes,
                    writes: telemetry.cache_stats.writes,
                },
                counts: telemetry.counts.clone(),
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_kernel::AnalysisKernel;
    use crate::analysis_kernel::incremental::DigestKind;
    fn identity(label: &str) -> ProviderOutputIdentity {
        ProviderOutputIdentity {
            provider_id: label.to_string(),
            provider_version: "test".to_string(),
            schema_version: "test:1".to_string(),
            output_digest: Digest::from_parts(DigestKind::ProviderOutput, "test", &[label]),
            precision: PrecisionTier::Exact,
        }
    }
    #[test]
    fn status_codec_accepts_exactly_the_closed_six_labels() {
        let statuses = [
            ProviderOutcomeStatus::Succeeded,
            ProviderOutcomeStatus::Failed,
            ProviderOutcomeStatus::DependencyBlocked,
            ProviderOutcomeStatus::Unsupported,
            ProviderOutcomeStatus::SetupMissing,
            ProviderOutcomeStatus::PlannedAbsent,
        ];
        for status in statuses {
            assert_eq!(ProviderOutcomeStatus::decode(status.label()), Some(status));
        }
        for rejected in [
            "",
            "success",
            "native_trusted",
            "provider_failed",
            "Succeeded",
            "planned-absent",
        ] {
            assert_eq!(ProviderOutcomeStatus::decode(rejected), None);
        }
        assert!(
            ProviderOutcome::from_closed_parts(
                "A".into(),
                ProviderOutcomeStatus::DependencyBlocked,
                None,
                Some(ProviderFailureStage::Dependency),
                Some(ProviderFailureReason::DependencyUnavailable),
                vec!["B".into(), "B".into()],
            )
            .is_err()
        );
    }
    #[test]
    fn hard_dependency_audit_references_only_static_manifest_providers() {
        let inventory = AnalysisKernel::provider_manifests()
            .iter()
            .map(|manifest| manifest.id)
            .collect::<BTreeSet<_>>();
        for manifest in AnalysisKernel::provider_manifests() {
            for dependency in hard_dependencies(manifest.id) {
                assert!(
                    inventory.contains(dependency),
                    "{} consumes missing provider {dependency}",
                    manifest.id
                );
                assert_ne!(
                    manifest.id, *dependency,
                    "{} cannot consume its own output",
                    manifest.id
                );
            }
        }
    }
    /// Every provider in the sealed inventory needs an explicit
    /// `hard_dependencies` arm. The `_ => &[]` fallback would otherwise give a
    /// newly added provider zero dependencies, so a failed upstream would never
    /// block it and its outcome would claim success on missing inputs.
    #[test]
    fn every_manifest_provider_has_an_explicit_hard_dependency_arm() {
        let declared = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/analysis_kernel/outcome.rs"),
        )
        .expect("this source file is readable");
        let arms = declared
            .split("match provider_id {")
            .nth(1)
            .expect("hard_dependencies has a match on provider_id");
        for manifest in AnalysisKernel::provider_manifests() {
            assert!(
                arms.contains(&format!("\"{}\"", manifest.id)),
                "{} falls through to the `_ => &[]` hard-dependency default",
                manifest.id
            );
        }
    }

    #[test]
    fn duplicate_unknown_and_absent_transitions_are_rejected() {
        let mut tracker = ProviderOutcomeTracker::for_test(&["A", "B"], &["A"], &[]);
        tracker.record_success("A", identity("A")).unwrap();
        assert!(matches!(
            tracker.record_success("A", identity("again")),
            Err(ProviderOutcomeError::InvalidTransition { .. })
        ));
        assert!(matches!(
            tracker.record_success("unknown", identity("unknown")),
            Err(ProviderOutcomeError::UnknownProvider(_))
        ));
        assert!(matches!(
            tracker.record_success("B", identity("B")),
            Err(ProviderOutcomeError::InvalidTransition { .. })
        ));
    }
    #[test]
    fn can_run_returns_sorted_exact_blockers_and_skips_unrelated_branches() {
        let mut tracker = ProviderOutcomeTracker::for_test(
            &["A", "B", "C", "D"],
            &["A", "B", "C", "D"],
            &[("C", &["B", "A", "B"])],
        );
        tracker
            .record_non_success(
                "A",
                ProviderOutcomeStatus::Failed,
                ProviderFailureStage::Execution,
                ProviderFailureReason::ExecutionFailed,
            )
            .unwrap();
        tracker
            .record_non_success(
                "B",
                ProviderOutcomeStatus::SetupMissing,
                ProviderFailureStage::Setup,
                ProviderFailureReason::SetupMissing,
            )
            .unwrap();
        assert_eq!(tracker.can_run("C").unwrap(), ["A", "B"]);
        assert!(tracker.can_run("D").unwrap().is_empty());
        tracker
            .record_dependency_blocked("C", vec!["B".to_string(), "A".to_string()])
            .unwrap();
    }
    #[test]
    fn execution_failure_is_recorded_immediately_and_blocks_hard_dependents() {
        let mut tracker = ProviderOutcomeTracker::for_test(
            &["upstream", "dependent"],
            &["upstream", "dependent"],
            &[("dependent", &["upstream"])],
        );
        tracker
            .record_failure(
                "upstream",
                ProviderOutcomeStatus::Failed,
                ProviderFailureStage::Execution,
                ProviderFailureReason::ExecutionFailed,
            )
            .unwrap();

        assert_eq!(tracker.can_run("dependent").unwrap(), ["upstream"]);
        tracker
            .record_dependency_blocked("dependent", vec!["upstream".to_string()])
            .unwrap();
        let outcomes = tracker.seal(&ValidationDowngrades::default()).unwrap();
        assert_eq!(outcomes[0].status, ProviderOutcomeStatus::Failed);
        assert!(outcomes[0].output_identity.is_none());
        assert_eq!(outcomes[1].status, ProviderOutcomeStatus::DependencyBlocked);
        assert!(outcomes[1].output_identity.is_none());
    }

    #[test]
    fn validation_failure_closes_transitively_and_preserves_independent_success() {
        let mut tracker = ProviderOutcomeTracker::for_test(
            &["A", "B", "C", "D", "E"],
            &["A", "B", "C", "D"],
            &[("B", &["A"]), ("C", &["B"])],
        );
        for provider_id in ["A", "B", "C", "D"] {
            assert!(tracker.can_run(provider_id).unwrap().is_empty());
            tracker
                .record_success(provider_id, identity(provider_id))
                .unwrap();
        }
        let outcomes = tracker
            .seal(&ValidationDowngrades::for_providers(["A".to_string()]))
            .unwrap();
        assert_eq!(
            outcomes
                .iter()
                .map(|outcome| (outcome.provider_id.as_str(), outcome.status))
                .collect::<Vec<_>>(),
            [
                ("A", ProviderOutcomeStatus::Failed),
                ("B", ProviderOutcomeStatus::DependencyBlocked),
                ("C", ProviderOutcomeStatus::DependencyBlocked),
                ("D", ProviderOutcomeStatus::Succeeded),
                ("E", ProviderOutcomeStatus::PlannedAbsent),
            ]
        );
        assert_eq!(outcomes[1].blockers, ["A"]);
        assert_eq!(outcomes[2].blockers, ["B"]);
        assert!(
            outcomes[..3]
                .iter()
                .all(|outcome| outcome.output_identity.is_none())
        );
        assert!(outcomes[3].output_identity.is_some());
        assert!(outcomes[4].output_identity.is_none());
    }
    #[test]
    fn global_validation_failure_downgrades_every_provisional_success() {
        let mut tracker =
            ProviderOutcomeTracker::for_test(&["A", "B"], &["A", "B"], &[("B", &["A"])]);
        tracker.record_success("A", identity("A")).unwrap();
        tracker.record_success("B", identity("B")).unwrap();
        let mut validation = ValidationDowngrades::default();
        validation.extend_provider_ids(["unused".to_string()]);
        validation.mark_global();
        let outcomes = tracker.seal(&validation).unwrap();
        assert!(
            outcomes
                .iter()
                .all(|outcome| outcome.status == ProviderOutcomeStatus::Failed)
        );
        assert!(
            outcomes
                .iter()
                .all(|outcome| outcome.failure_stage == Some(ProviderFailureStage::Validation))
        );
    }

    #[cfg(test)]
    mod resource_outcome_tests {
        use super::*;

        #[test]
        fn budget_exceeded_round_trips_through_its_label() {
            assert_eq!(
                ProviderOutcomeStatus::BudgetExceeded.label(),
                "budget_exceeded"
            );
            assert_eq!(
                ProviderOutcomeStatus::decode("budget_exceeded"),
                Some(ProviderOutcomeStatus::BudgetExceeded)
            );
            assert_eq!(
                ProviderFailureReason::MemoryCeiling.label(),
                "memory_ceiling"
            );
            assert_eq!(
                ProviderFailureReason::decode("memory_ceiling"),
                Some(ProviderFailureReason::MemoryCeiling)
            );
        }

        #[test]
        fn a_memory_ceiling_stop_is_a_valid_non_success_outcome() {
            let outcome = ProviderOutcome::non_success(
                "polint.data_flow".to_string(),
                ProviderOutcomeStatus::BudgetExceeded,
                ProviderFailureStage::Execution,
                ProviderFailureReason::MemoryCeiling,
                Vec::new(),
            )
            .expect("a resource stop is a recordable outcome");
            assert_eq!(outcome.status, ProviderOutcomeStatus::BudgetExceeded);
        }

        #[test]
        fn a_memory_ceiling_stop_rejects_a_mismatched_stage() {
            assert!(
                ProviderOutcome::non_success(
                    "polint.data_flow".to_string(),
                    ProviderOutcomeStatus::BudgetExceeded,
                    ProviderFailureStage::Validation,
                    ProviderFailureReason::MemoryCeiling,
                    Vec::new(),
                )
                .is_err()
            );
        }
    }
}
