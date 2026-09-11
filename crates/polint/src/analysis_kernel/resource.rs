//! Runtime resource envelope: a memory ceiling checked at provider boundaries.
//!
//! polint's existing budgets (`solver::budget`, `PathBudget`, the demand-query
//! caps) bound *precision work*: how many steps a fixpoint may take, how many
//! objects a variable may point at. None of them bounds the thing that actually
//! kills a run on a large repository — the resident set. A repo can exhaust
//! host memory without any solver exceeding its step count, simply by being big.
//!
//! This module adds the missing dimension. Live RSS is sampled once per provider
//! boundary (23 samples per run — free relative to the providers themselves).
//! When it crosses the ceiling, the remaining providers are not executed: they
//! are recorded as `budget_exceeded`, the capabilities that depend on them
//! degrade through the existing capability-support path, and the run emits a
//! `polint/resource-budget` diagnostic that `polint unknowns` surfaces as a
//! `budget_exceeded` row. The run finishes and reports what it could not do,
//! instead of being killed by the OOM reaper with no output at all.
//!
//! The default ceiling is a *safety net*, not a policy: 80 % of host RAM. A run
//! that fits in memory today behaves exactly as it did before, byte for byte.
//! `POLINT_MEMORY_CEILING_MB` pins an explicit ceiling for CI, and
//! `POLINT_MEMORY_CEILING_MB=0` disables the net entirely.

/// Environment override for the ceiling, in mebibytes. `0` disables it.
const CEILING_ENV: &str = "POLINT_MEMORY_CEILING_MB";

/// Fraction of host RAM used when no explicit ceiling is configured.
const HOST_FRACTION_PERCENT: u64 = 80;

/// Where the active ceiling came from (reported in the degradation message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CeilingSource {
    /// `POLINT_MEMORY_CEILING_MB`.
    Configured,
    /// [`HOST_FRACTION_PERCENT`] of what the host reports as total memory.
    HostFraction,
}

impl CeilingSource {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::HostFraction => "host_fraction",
        }
    }
}

/// The provider boundary at which the ceiling was first crossed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResourceTrip {
    /// Provider that had just finished when the ceiling was observed crossed.
    pub(crate) after_provider: &'static str,
    pub(crate) observed_bytes: u64,
    pub(crate) ceiling_bytes: u64,
    pub(crate) source: CeilingSource,
}

/// Memory envelope for one kernel run.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ResourceEnvelope {
    ceiling_bytes: Option<u64>,
    source: CeilingSource,
    trip: Option<ResourceTrip>,
}

impl ResourceEnvelope {
    /// Reads the ceiling from the environment, falling back to a fraction of
    /// host RAM. An unparseable or absent host reading disables the net rather
    /// than guessing a number that could degrade a healthy run.
    pub(crate) fn from_env() -> Self {
        if let Ok(raw) = std::env::var(CEILING_ENV) {
            if let Ok(megabytes) = raw.trim().parse::<u64>() {
                return Self {
                    ceiling_bytes: (megabytes > 0).then(|| megabytes * 1024 * 1024),
                    source: CeilingSource::Configured,
                    trip: None,
                };
            }
            tracing::warn!(
                target: "polint::kernel",
                value = raw,
                "ignoring unparseable {CEILING_ENV}; falling back to the host-memory ceiling"
            );
        }
        Self {
            ceiling_bytes: host_memory_bytes()
                .map(|total| total.saturating_mul(HOST_FRACTION_PERCENT) / 100),
            source: CeilingSource::HostFraction,
            trip: None,
        }
    }

    /// Samples live RSS after `provider_id` and latches a trip if it is over.
    ///
    /// Latching (rather than re-testing) keeps the schedule deterministic for a
    /// given trip point: once the envelope is exhausted every later provider
    /// sees the same answer.
    pub(crate) fn observe(&mut self, provider_id: &'static str) {
        let Some(ceiling) = self.ceiling_bytes else {
            return;
        };
        if self.trip.is_some() {
            return;
        }
        let observed = crate::measure::current_rss_bytes();
        if observed > ceiling {
            self.trip = Some(ResourceTrip {
                after_provider: provider_id,
                observed_bytes: observed,
                ceiling_bytes: ceiling,
                source: self.source,
            });
        }
    }

    /// The trip, if the ceiling has been crossed.
    pub(crate) const fn trip(&self) -> Option<ResourceTrip> {
        self.trip
    }

    /// Whether providers scheduled after the trip should be skipped.
    pub(crate) const fn exhausted(&self) -> bool {
        self.trip.is_some()
    }
}

/// Rule id for the single diagnostic a resource-budget stop emits.
///
/// `polint unknowns` translates it into a `budget_exceeded` row, the same way
/// `polint/go-semantic` diagnostics become `go_*` rows.
pub(crate) const RESOURCE_BUDGET_RULE_ID: &str = "polint/resource-budget";

/// The one diagnostic a resource-budget stop emits.
pub(crate) fn budget_diagnostic(trip: &ResourceTrip) -> crate::diagnostics::Diagnostic {
    crate::diagnostics::Diagnostic::warning(
        RESOURCE_BUDGET_RULE_ID,
        "<workspace>",
        crate::internal_core::DiagnosticRange::point(1, 1),
        format!(
            "memory ceiling reached after `{}`: {} MiB resident against a {} MiB ceiling ({}). \
             Providers scheduled after it were skipped and their capabilities degraded; \
             set POLINT_MEMORY_CEILING_MB to raise or disable the ceiling.",
            trip.after_provider,
            trip.observed_bytes / (1024 * 1024),
            trip.ceiling_bytes / (1024 * 1024),
            trip.source.label(),
        ),
    )
    .with_evidence(crate::diagnostics::BUDGET_EVIDENCE_LABEL, "memory_ceiling")
    .with_evidence(crate::diagnostics::BUDGET_STATUS_EVIDENCE_LABEL, "exceeded")
    .with_evidence("after_provider", trip.after_provider)
    .with_evidence("observed_bytes", trip.observed_bytes.to_string())
    .with_evidence("ceiling_bytes", trip.ceiling_bytes.to_string())
}

/// Total host memory in bytes, read from `/proc/meminfo` on Linux.
///
/// Returns `None` on every other target and whenever the reading is not
/// available, which disables the safety net rather than inventing a ceiling.
fn host_memory_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        for line in meminfo.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:")
                && let Some(kilobytes) = rest.split_whitespace().next()
                && let Ok(kilobytes) = kilobytes.parse::<u64>()
            {
                return Some(kilobytes.saturating_mul(1024));
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_ceiling_never_trips() {
        let mut envelope = ResourceEnvelope {
            ceiling_bytes: None,
            source: CeilingSource::Configured,
            trip: None,
        };
        envelope.observe("polint.source");
        assert!(!envelope.exhausted());
        assert_eq!(envelope.trip(), None);
    }

    #[test]
    fn a_zero_ceiling_trips_immediately_and_latches_the_first_provider() {
        let mut envelope = ResourceEnvelope {
            ceiling_bytes: Some(1),
            source: CeilingSource::Configured,
            trip: None,
        };
        envelope.observe("polint.cfg");
        envelope.observe("polint.calls");
        let trip = envelope.trip().expect("ceiling of 1 byte must trip");
        assert_eq!(trip.after_provider, "polint.cfg");
        assert_eq!(trip.ceiling_bytes, 1);
        assert!(trip.observed_bytes > 0);
        assert!(envelope.exhausted());
    }

    #[test]
    fn host_fraction_is_below_host_total_when_available() {
        if let Some(total) = host_memory_bytes() {
            let envelope = ResourceEnvelope::from_env();
            if envelope.source == CeilingSource::HostFraction {
                let ceiling = envelope
                    .ceiling_bytes
                    .expect("host reading yields a ceiling");
                assert!(
                    ceiling < total,
                    "ceiling {ceiling} must be under host {total}"
                );
                assert!(ceiling > 0);
            }
        }
    }
}
