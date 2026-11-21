//! Embedded model bytes and identity shared by cache keys and frontends.
//!
//! Schema validation and constraint lowering belong to the consuming frontend.
//! Keeping the bytes here lets neutral cache metadata hash the same input
//! without depending on language-specific analysis code.

use crate::analysis_api::{Digest, DigestKind};

pub(crate) const SOURCE: &str = include_str!("../../../models/platform/javascript.toml");

pub(crate) fn digest(source: &str) -> Digest {
    Digest::from_parts(
        DigestKind::ProviderParameters,
        "javascript_platform",
        &[source],
    )
}

/// First evidence entry of a model edge derived from the embedded platform
/// model, followed by the model's name.
pub(crate) const EVIDENCE_PREFIX: &str = "platform:";

/// Evidence entry naming the embedded platform model's digest.
pub(crate) fn digest_evidence() -> String {
    format!("platform_digest:{}", digest(SOURCE).value)
}

/// Whether a model edge was derived from the embedded platform model rather
/// than from a repo-local adaptation fact. Platform edges follow points-to
/// sets, so the solver budget bounds them; the adaptation caps on repo-local
/// facts do not apply.
pub(crate) fn is_platform_model_edge(evidence: &[String]) -> bool {
    matches!(
        evidence,
        [model, digest] if model.starts_with(EVIDENCE_PREFIX) && *digest == digest_evidence()
    )
}
