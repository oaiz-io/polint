use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionRejectionReason {
    UndeclaredOutput,
    MissingBinding,
    InvalidSpan,
    MissingPrecision,
    MissingProvenance,
    SyntheticIdMissingEvidence,
    DuplicateStableKey,
    NativeConflict,
    FrameworkPrecisionCeiling,
    TypeValueAliasPrecisionCeiling,
    MalformedPayload,
}

use super::manifest::ExtensionActivationStatus;
use super::sinks::{
    ExtensionFactCandidate, ExtensionFactConfidence, ExtensionFactPrecision, ExtensionFactStatus,
};
use crate::internal_core::{StableKeyId, StableKeyInterner};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionOutput {
    pub activations: Vec<ExtensionActivationRow>,
    pub accepted: Vec<AcceptedExtensionFact>,
    pub rejected: Vec<RejectedExtensionFact>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionActivationRow {
    pub extension_id: String,
    pub provider_id: Option<String>,
    pub status: ExtensionActivationStatus,
    pub diagnostic_count: usize,
    pub output_digest_inputs: Vec<String>,
    pub diagnostic_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedExtensionFact {
    pub extension_id: String,
    pub provider_id: String,
    pub fact_family: String,
    pub stable_key: StableKeyId,
    pub binding_refs: Vec<String>,
    pub precision: ExtensionFactPrecision,
    pub confidence: ExtensionFactConfidence,
    pub status: ExtensionFactStatus,
    pub evidence: Vec<String>,
    pub payload_labels: Vec<String>,
    pub payload_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedExtensionFact {
    pub extension_id: String,
    pub provider_id: String,
    pub fact_family: String,
    pub stable_key: StableKeyId,
    pub reason: ExtensionRejectionReason,
    pub evidence: Vec<String>,
}

impl ExtensionOutput {
    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        for activation in &mut self.activations {
            activation.output_digest_inputs.sort();
            activation.output_digest_inputs.dedup();
        }
        self.activations.sort_by(|left, right| {
            (
                left.extension_id.as_str(),
                left.provider_id.as_deref().unwrap_or(""),
                left.status,
            )
                .cmp(&(
                    right.extension_id.as_str(),
                    right.provider_id.as_deref().unwrap_or(""),
                    right.status,
                ))
        });
        self.accepted.sort_by(|left, right| {
            left.extension_id
                .as_str()
                .cmp(right.extension_id.as_str())
                .then_with(|| left.provider_id.as_str().cmp(right.provider_id.as_str()))
                .then_with(|| left.fact_family.as_str().cmp(right.fact_family.as_str()))
                .then_with(|| interner.compare_canonical(left.stable_key, right.stable_key))
        });
        self.rejected.sort_by(|left, right| {
            left.extension_id
                .as_str()
                .cmp(right.extension_id.as_str())
                .then_with(|| left.provider_id.as_str().cmp(right.provider_id.as_str()))
                .then_with(|| left.fact_family.as_str().cmp(right.fact_family.as_str()))
                .then_with(|| interner.compare_canonical(left.stable_key, right.stable_key))
                .then_with(|| left.reason.cmp(&right.reason))
        });
        self
    }
}

impl AcceptedExtensionFact {
    pub fn from_candidate(candidate: ExtensionFactCandidate, interner: &StableKeyInterner) -> Self {
        let candidate = candidate.normalized();
        let payload_digest = extension_payload_digest(&candidate, interner);
        Self {
            extension_id: candidate.extension_id,
            provider_id: candidate.provider_id,
            fact_family: candidate.fact_family,
            stable_key: candidate.stable_key,
            binding_refs: candidate.binding_refs,
            precision: candidate.precision.expect("accepted fact has precision"),
            confidence: candidate.confidence,
            status: ExtensionFactStatus::Accepted,
            evidence: candidate.evidence,
            payload_labels: candidate.payload_labels,
            payload_digest,
        }
    }
}

impl RejectedExtensionFact {
    pub fn from_candidate(
        candidate: &ExtensionFactCandidate,
        reason: ExtensionRejectionReason,
        _interner: &StableKeyInterner,
    ) -> Self {
        let candidate = candidate.clone().normalized();
        Self {
            extension_id: candidate.extension_id,
            provider_id: candidate.provider_id,
            fact_family: candidate.fact_family,
            stable_key: candidate.stable_key,
            reason,
            evidence: candidate.evidence,
        }
    }
}

fn extension_payload_digest(
    candidate: &ExtensionFactCandidate,
    interner: &StableKeyInterner,
) -> String {
    let mut parts = vec![
        format!("extension_id={}", candidate.extension_id),
        format!("provider_id={}", candidate.provider_id),
        format!("family={}", candidate.fact_family),
        format!("stable_key={}", interner.resolve(candidate.stable_key)),
        format!("precision={:?}", candidate.precision),
        format!("confidence={:?}", candidate.confidence),
    ];
    parts.extend(
        candidate
            .binding_refs
            .iter()
            .map(|binding| format!("binding={binding}")),
    );
    parts.extend(
        candidate
            .evidence
            .iter()
            .map(|evidence| format!("evidence={evidence}")),
    );
    parts.extend(
        candidate
            .payload_labels
            .iter()
            .map(|payload| format!("payload={payload}")),
    );
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    crate::analysis_neutral::hash::stable_hash(&refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_sorts_accepted_and_rejected_rows_deterministically() {
        let interner = crate::internal_core::test_stable_key_interner();
        let output = ExtensionOutput {
            activations: Vec::new(),
            accepted: vec![
                accepted("demo", "z", "family", "b"),
                accepted("demo", "a", "family", "a"),
            ],
            rejected: vec![
                rejected(
                    "demo",
                    "z",
                    "family",
                    "b",
                    ExtensionRejectionReason::MissingPrecision,
                ),
                rejected(
                    "demo",
                    "a",
                    "family",
                    "a",
                    ExtensionRejectionReason::UndeclaredOutput,
                ),
            ],
        }
        .normalized(&interner);

        assert_eq!(output.accepted[0].provider_id, "a");
        assert_eq!(
            output.rejected[0].reason,
            ExtensionRejectionReason::UndeclaredOutput
        );
    }

    fn accepted(
        extension_id: &str,
        provider_id: &str,
        fact_family: &str,
        stable_key: &str,
    ) -> AcceptedExtensionFact {
        AcceptedExtensionFact {
            extension_id: extension_id.to_string(),
            provider_id: provider_id.to_string(),
            fact_family: fact_family.to_string(),
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
            binding_refs: Vec::new(),
            precision: ExtensionFactPrecision::Heuristic,
            confidence: ExtensionFactConfidence::Medium,
            status: ExtensionFactStatus::Accepted,
            evidence: Vec::new(),
            payload_labels: Vec::new(),
            payload_digest: stable_key.to_string(),
        }
    }

    fn rejected(
        extension_id: &str,
        provider_id: &str,
        fact_family: &str,
        stable_key: &str,
        reason: ExtensionRejectionReason,
    ) -> RejectedExtensionFact {
        RejectedExtensionFact {
            extension_id: extension_id.to_string(),
            provider_id: provider_id.to_string(),
            fact_family: fact_family.to_string(),
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
            reason,
            evidence: Vec::new(),
        }
    }
}
