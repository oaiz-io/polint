use serde::{Deserialize, Serialize};

use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallEdgeKind, CallPrecision, CallProvenance, CallTargetStatus,
    UnresolvedCallReason,
};
use crate::analysis_neutral::ids::{CallSiteId, CallTargetId, RefinedCallEdgeId};
use crate::internal_core::{FunctionId, Language, StableKeyId, SymbolId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefinedCallEdgeFact {
    pub id: RefinedCallEdgeId,
    pub site: CallSiteId,
    pub base_target: Option<CallTargetId>,
    pub caller: FunctionId,
    pub target_function: Option<FunctionId>,
    pub target_symbol: Option<SymbolId>,
    pub synthetic_target: Option<String>,
    pub language: Language,
    pub edge_kind: CallEdgeKind,
    pub algorithm: CallAlgorithm,
    pub tier: RefinedCallTier,
    pub status: CallTargetStatus,
    pub reason: Option<UnresolvedCallReason>,
    pub provenance: CallProvenance,
    pub precision: CallPrecision,
    pub validation: RefinedCallValidation,
    pub confidence: RefinedCallConfidence,
    pub evidence: Vec<String>,
    pub input_stable_keys: Vec<String>,
    pub stable_key: StableKeyId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RefinedCallTier {
    DirectOnly,
    DirectPlusFramework,
    /// An edge a language type checker resolved, or a rapid-type candidate it
    /// narrowed. Ranked above the token and points-to tiers because it is
    /// derived from declared types rather than from value flow, and its
    /// ordering is what lets a consumer prefer it for the same call site.
    TypeDirected,
    TypeValueFunctionToken,
    SummaryAssisted,
    PointsToAssisted,
    ExtensionModel,
    AllAccepted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RefinedCallValidation {
    Native,
    ReferentiallyValidated,
    ExtensionValidated,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RefinedCallConfidence {
    High,
    Medium,
    Low,
}

impl RefinedCallEdgeFact {
    pub fn normalized(mut self) -> Self {
        self.evidence.sort();
        self.evidence.dedup();
        self.input_stable_keys.sort();
        self.input_stable_keys.dedup();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal_core::Language;

    #[test]
    fn refined_edge_preserves_base_call_site_and_stable_key() {
        let edge = RefinedCallEdgeFact {
            id: RefinedCallEdgeId(1),
            site: CallSiteId(2),
            base_target: Some(CallTargetId(3)),
            caller: FunctionId::from_raw(4),
            target_function: Some(FunctionId::from_raw(5)),
            target_symbol: None,
            synthetic_target: None,
            language: Language::TypeScript,
            edge_kind: CallEdgeKind::FunctionValue,
            algorithm: CallAlgorithm::PointsTo,
            tier: RefinedCallTier::TypeValueFunctionToken,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::Native,
            precision: CallPrecision::SetupAware,
            validation: RefinedCallValidation::Native,
            confidence: RefinedCallConfidence::High,
            evidence: vec!["value".to_string()],
            input_stable_keys: vec!["call-site".to_string()],
            stable_key: crate::internal_core::stable_key_for_test("refined:edge"),
        };

        assert_eq!(edge.site, CallSiteId(2));
        assert_eq!(edge.base_target, Some(CallTargetId(3)));
        assert_eq!(
            edge.stable_key,
            crate::internal_core::stable_key_for_test("refined:edge")
        );
    }

    #[test]
    fn normalized_edge_sorts_evidence_and_inputs() {
        let edge = RefinedCallEdgeFact {
            id: RefinedCallEdgeId(0),
            site: CallSiteId(0),
            base_target: None,
            caller: FunctionId::from_raw(0),
            target_function: None,
            target_symbol: None,
            synthetic_target: Some("synthetic".to_string()),
            language: Language::Unknown,
            edge_kind: CallEdgeKind::Synthetic,
            algorithm: CallAlgorithm::RepoModel,
            tier: RefinedCallTier::ExtensionModel,
            status: CallTargetStatus::Ambiguous,
            reason: Some(UnresolvedCallReason::Unknown),
            provenance: CallProvenance::Extension,
            precision: CallPrecision::Heuristic,
            validation: RefinedCallValidation::ExtensionValidated,
            confidence: RefinedCallConfidence::Medium,
            evidence: vec!["b".to_string(), "a".to_string(), "a".to_string()],
            input_stable_keys: vec!["z".to_string(), "a".to_string()],
            stable_key: crate::internal_core::stable_key_for_test("refined:synthetic"),
        }
        .normalized();

        assert_eq!(edge.evidence, vec!["a", "b"]);
        assert_eq!(edge.input_stable_keys, vec!["a", "z"]);
    }
}
