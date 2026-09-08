//! Rule-visible analysis completeness status.

/// Completeness status for one capability requested by a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CapabilityCompletenessStatus {
    /// Every tracked provider completed and no relevant unknown row was recorded.
    Complete,
    /// A provider or analysis query exhausted a configured resource budget.
    BudgetExceeded,
    /// A provider required for the capability failed or was dependency-blocked.
    ProviderFailed,
    /// Analysis ran, but an explicit unknown means its result is not complete.
    Degraded,
    /// The host has no completeness information for this capability.
    Unknown,
}

impl CapabilityCompletenessStatus {
    /// Returns the stable snake-case status label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::BudgetExceeded => "budget_exceeded",
            Self::ProviderFailed => "provider_failed",
            Self::Degraded => "degraded",
            Self::Unknown => "unknown",
        }
    }
}

/// Completeness information for one requested capability.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CapabilityCompleteness {
    capability: String,
    status: CapabilityCompletenessStatus,
    reason: Option<String>,
    rules: Vec<String>,
}

impl CapabilityCompleteness {
    pub(crate) fn new(
        capability: String,
        status: CapabilityCompletenessStatus,
        reason: Option<String>,
        rules: Vec<String>,
    ) -> Self {
        Self {
            capability,
            status,
            reason,
            rules,
        }
    }

    /// Returns the stable capability name.
    pub fn capability(&self) -> &str {
        &self.capability
    }

    /// Returns the completeness status.
    pub fn status(&self) -> CapabilityCompletenessStatus {
        self.status
    }

    /// Returns the deterministic explanation for a non-complete status.
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

/// Read-only completeness information for the current rule's requested capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CompletenessView {
    entries: Vec<CapabilityCompleteness>,
    available: bool,
}

impl CompletenessView {
    pub(crate) fn new(entries: Vec<CapabilityCompleteness>) -> Self {
        Self {
            entries,
            available: true,
        }
    }

    pub(crate) fn unknown() -> Self {
        Self {
            entries: Vec::new(),
            available: false,
        }
    }

    pub(crate) fn for_rule(&self, rule_id: &str) -> Self {
        if !self.available {
            return Self::unknown();
        }
        Self::new(
            self.entries
                .iter()
                .filter(|entry| entry.rules.iter().any(|rule| rule == rule_id))
                .cloned()
                .collect(),
        )
    }

    /// Returns completeness rows in deterministic capability order.
    pub fn entries(&self) -> &[CapabilityCompleteness] {
        &self.entries
    }

    /// Returns the status for `capability`, or `Unknown` when no row is available.
    pub fn status_for(&self, capability: &str) -> CapabilityCompletenessStatus {
        self.entries
            .iter()
            .find(|entry| entry.capability == capability)
            .map_or(CapabilityCompletenessStatus::Unknown, |entry| entry.status)
    }

    /// Returns the explanation recorded for `capability`, when available.
    pub fn reason_for(&self, capability: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|entry| entry.capability == capability)
            .and_then(CapabilityCompleteness::reason)
    }

    /// Returns whether completeness telemetry is available and every requested capability is complete.
    pub fn is_complete(&self) -> bool {
        self.available
            && self
                .entries
                .iter()
                .all(|entry| entry.status == CapabilityCompletenessStatus::Complete)
    }

    /// Returns whether any requested capability or its queries exceeded a budget.
    pub fn budget_exceeded(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.status == CapabilityCompletenessStatus::BudgetExceeded)
    }
}
