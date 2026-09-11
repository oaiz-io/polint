use serde::{Deserialize, Serialize};

pub(crate) use crate::analysis_api::CacheStats;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProviderOutputMeta {
    pub(crate) provider_id: String,
    pub(crate) provider_version: String,
    pub(crate) schema_version: String,
    pub(crate) output_digest: super::digest::Digest,
    pub(crate) precision: super::keys::PrecisionTier,
    pub(crate) validation: String,
    pub(crate) dependency_inputs: Vec<super::digest::Digest>,
    pub(crate) cache_stats: CacheStats,
}

impl ProviderOutputMeta {
    #[expect(
        clippy::too_many_arguments,
        reason = "Provider output metadata construction keeps identity and status fields explicit."
    )]
    pub(crate) fn new(
        provider_id: impl Into<String>,
        provider_version: impl Into<String>,
        schema_version: impl Into<String>,
        output_digest: super::digest::Digest,
        precision: super::keys::PrecisionTier,
        validation: impl Into<String>,
        dependency_inputs: Vec<super::digest::Digest>,
        cache_stats: CacheStats,
    ) -> Self {
        let mut dependency_inputs = dependency_inputs;
        dependency_inputs.sort();
        Self {
            provider_id: provider_id.into(),
            provider_version: provider_version.into(),
            schema_version: schema_version.into(),
            output_digest,
            precision,
            validation: validation.into(),
            dependency_inputs,
            cache_stats,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderTelemetry {
    pub(crate) provider_id: String,
    pub(crate) cache_stats: CacheStats,
    /// Wall time the provider stage took, or `None` when it did not run.
    pub(crate) elapsed_ms: Option<u64>,
    /// Stage-specific counters the provider reported, such as a sidecar's
    /// per-phase timings and workload sizes.
    pub(crate) counts: std::collections::BTreeMap<String, u64>,
}

impl ProviderTelemetry {
    pub(crate) fn new(provider_id: impl Into<String>, cache_stats: CacheStats) -> Self {
        Self {
            provider_id: provider_id.into(),
            cache_stats,
            elapsed_ms: None,
            counts: std::collections::BTreeMap::new(),
        }
    }

    pub(crate) fn with_stage(
        mut self,
        elapsed_ms: Option<u64>,
        counts: std::collections::BTreeMap<String, u64>,
    ) -> Self {
        self.elapsed_ms = elapsed_ms;
        self.counts = counts;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cache_stats_serialize_zero_counters() {
        let stats = CacheStats::default();
        let json = serde_json::to_value(stats).expect("cache stats should serialize");

        assert_eq!(json["hits"], 0);
        assert_eq!(json["misses"], 0);
        assert_eq!(json["recomputes"], 0);
        assert_eq!(json["writes"], 0);
        assert_eq!(json["bypasses_disabled"], 0);
        assert_eq!(json["invalid_evicted_reads"], 0);
        assert_eq!(json["verified_reuse"], 0);
        assert_eq!(json["quarantines"], 0);
    }

    #[test]
    fn record_methods_increment_each_counter() {
        let mut stats = CacheStats::default();

        stats.record_hit();
        stats.record_miss();
        stats.record_recompute();
        stats.record_write();
        stats.record_disabled_bypass();
        stats.record_invalid_evicted_read();
        stats.record_verified_reuse();
        stats.record_quarantine();

        assert_eq!(
            stats,
            CacheStats {
                hits: 1,
                misses: 1,
                recomputes: 1,
                writes: 1,
                bypasses_disabled: 1,
                invalid_evicted_reads: 1,
                verified_reuse: 1,
                quarantines: 1,
            }
        );
    }

    #[test]
    fn provider_telemetry_defaults_to_provider_key_and_cache_stats() {
        let telemetry = ProviderTelemetry::new("polint.ts.syntax", CacheStats::default());
        assert!(telemetry.elapsed_ms.is_none());
        assert!(telemetry.counts.is_empty());

        assert_eq!(telemetry.provider_id, "polint.ts.syntax");
        assert_eq!(telemetry.cache_stats, CacheStats::default());
    }
}
