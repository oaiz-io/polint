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
