//! Go's slice of the shared sidecar source cache.
//!
//! The cache mechanics live in [`crate::subprocess::embedded_cache`]; what
//! belongs to Go is the cache directory name and the set of files the Go
//! sidecars create inside their own source directories at run time.

use std::path::PathBuf;

use crate::subprocess::SidecarCacheFamily;
#[cfg(unix)]
pub(crate) use crate::subprocess::write_private_file;
pub(crate) use crate::subprocess::{read_verified_private_file, verify_private_file};

const GO_SIDECAR_CACHE: SidecarCacheFamily = SidecarCacheFamily {
    directory: "go-sidecars",
    runtime_artifacts: is_go_sidecar_runtime_artifact,
};

pub(crate) fn materialize_embedded_sources(
    cache_name: &str,
    version: &str,
    content_hash: &str,
    files: &[(&str, &str)],
) -> Result<PathBuf, String> {
    crate::subprocess::materialize_embedded_sources(
        &GO_SIDECAR_CACHE,
        cache_name,
        version,
        content_hash,
        files,
    )
}

/// Files the Go sidecars write into their own materialized source directory:
/// the completion marker, `go build` staging outputs, the published binary,
/// and the receipt and lock that guard it. They are outputs of using the
/// cache, not inputs to it, so they must not invalidate the source manifest.
fn is_go_sidecar_runtime_artifact(relative: &str) -> bool {
    !relative.contains('/')
        && (matches!(
            relative,
            ".complete" | "polint-go-frontend" | "polint-go-frontend.exe"
        ) || relative.starts_with(".build-")
            || relative.starts_with(".polint-go-frontend-")
            || relative.starts_with(".binary-")
            || relative.starts_with(".binary-lock-")
            || relative.starts_with(".binary-receipt-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_build_outputs_are_runtime_artifacts_not_sources() {
        for artifact in [
            ".complete",
            ".build-1-2",
            ".binary-lock",
            ".binary-receipt",
            ".binary-receipt.receipt-1-2",
            ".polint-go-frontend-abcdef",
            "polint-go-frontend",
            "polint-go-frontend.exe",
        ] {
            assert!(
                is_go_sidecar_runtime_artifact(artifact),
                "{artifact} should not invalidate the materialized sources"
            );
        }
    }

    #[test]
    fn embedded_sources_and_nested_paths_are_not_runtime_artifacts() {
        for source in ["go.mod", "go.sum", "main.go", "internal/semantic/emit.go"] {
            assert!(
                !is_go_sidecar_runtime_artifact(source),
                "{source} is embedded source and must be verified"
            );
        }
    }
}
