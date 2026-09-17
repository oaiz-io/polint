use crate::ts::types::lifecycle::{
    ANY_DENSITY_DEFER_PERCENT, ANY_DENSITY_DEGRADED_PERCENT, TsTypesConfig,
};

pub(crate) const TS_TYPES_SCHEMA_LABEL: &str = "ts-type-facts-1";
pub(crate) const TS_TYPES_PROVIDER_ID: &str = "polint.ts.types";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TsTypesCacheInputs {
    pub(crate) sidecar_digest: String,
    pub(crate) typescript_version: String,
    pub(crate) node_version: String,
    pub(crate) upstream_digest: String,
    pub(crate) lifecycle: TsTypesConfig,
}

/// Digest of everything about the provider itself that can change its answers.
///
/// The row vocabulary is listed explicitly: adding a row kind changes what the
/// refinement can conclude, so it has to invalidate a stored output rather than
/// quietly reuse one produced before the kind existed. The density thresholds
/// are here for the same reason — they are constants, but they decide which
/// typed edges survive.
pub(crate) fn ts_types_provider_parameter_digest() -> String {
    crate::ts::hash::stable_hash(&[
        TS_TYPES_SCHEMA_LABEL,
        TS_TYPES_PROVIDER_ID,
        "sidecar_digest",
        "typescript_version",
        "node_version",
        "lifecycle_v1",
        "upstream_digest",
        "project_v1",
        "callable_v1",
        "callsite_v1",
        "callee_v1",
        "receiver_v1",
        "any_density_v1",
        &format!("any_density_degraded_percent={ANY_DENSITY_DEGRADED_PERCENT}"),
        &format!("any_density_defer_percent={ANY_DENSITY_DEFER_PERCENT}"),
    ])
}

/// Key for the stored raw sidecar NDJSON.
pub(crate) fn ts_types_sidecar_cache_key(
    sidecar_digest: &str,
    typescript_version: &str,
    upstream_digest: &str,
    config: &TsTypesConfig,
) -> String {
    let lifecycle_digest = ts_types_lifecycle_digest(config);
    crate::ts::hash::stable_hash(&[
        "ts-types-sidecar-cache-v1",
        ts_types_provider_parameter_digest().as_str(),
        sidecar_digest,
        typescript_version,
        upstream_digest,
        lifecycle_digest.as_str(),
    ])
}

pub(crate) fn ts_types_input_digest(inputs: &TsTypesCacheInputs) -> String {
    let lifecycle_digest = ts_types_lifecycle_digest(&inputs.lifecycle);
    crate::ts::hash::stable_hash(&[
        ts_types_provider_parameter_digest().as_str(),
        inputs.sidecar_digest.as_str(),
        inputs.typescript_version.as_str(),
        inputs.node_version.as_str(),
        inputs.upstream_digest.as_str(),
        lifecycle_digest.as_str(),
    ])
}

pub(crate) fn ts_types_lifecycle_digest(config: &TsTypesConfig) -> String {
    let mut parts = vec![
        format!("enabled={}", config.enabled),
        format!(
            "typescript_path={}",
            config.typescript_path.as_deref().unwrap_or("")
        ),
        format!(
            "timeout_ms={}",
            config
                .timeout_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "default".to_string())
        ),
        // The scan scope decides which rows the sidecar emits, so two scopes
        // are two different outputs and must not share a cache entry.
        format!(
            "scope_files={}",
            crate::ts::hash::stable_hash(
                &config
                    .scope_files
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            )
        ),
    ];
    parts.extend(
        config
            .projects
            .iter()
            .map(|project| format!("project={project}")),
    );
    parts.sort();
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    crate::ts::hash::stable_hash(&refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> TsTypesConfig {
        TsTypesConfig {
            enabled: true,
            projects: vec!["tsconfig.json".to_string()],
            typescript_path: None,
            timeout_ms: None,
            scope_files: vec!["src/app.ts".to_string()],
            files_without_project: Vec::new(),
            explicitly_requested: false,
        }
    }

    #[test]
    fn lifecycle_digest_changes_when_the_project_set_changes() {
        let first = config();
        let mut second = config();
        second
            .projects
            .push("packages/web/tsconfig.json".to_string());

        assert_ne!(
            ts_types_lifecycle_digest(&first),
            ts_types_lifecycle_digest(&second)
        );
    }

    #[test]
    fn lifecycle_digest_changes_when_the_scan_scope_changes() {
        let first = config();
        let mut second = config();
        second.scope_files.push("src/other.ts".to_string());

        assert_ne!(
            ts_types_lifecycle_digest(&first),
            ts_types_lifecycle_digest(&second)
        );
    }

    #[test]
    fn lifecycle_digest_changes_when_the_tier_is_disabled() {
        let first = config();
        let mut second = config();
        second.enabled = false;

        assert_ne!(
            ts_types_lifecycle_digest(&first),
            ts_types_lifecycle_digest(&second)
        );
    }

    #[test]
    fn lifecycle_digest_changes_when_the_compiler_path_is_pinned() {
        let first = config();
        let mut second = config();
        second.typescript_path = Some("vendor/typescript".to_string());

        assert_ne!(
            ts_types_lifecycle_digest(&first),
            ts_types_lifecycle_digest(&second)
        );
    }

    #[test]
    fn lifecycle_digest_changes_when_the_budget_changes() {
        let first = config();
        let mut second = config();
        second.timeout_ms = Some(30_000);

        assert_ne!(
            ts_types_lifecycle_digest(&first),
            ts_types_lifecycle_digest(&second)
        );
    }

    #[test]
    fn lifecycle_digest_is_independent_of_project_order() {
        let mut first = config();
        first.projects = vec!["a/tsconfig.json".to_string(), "b/tsconfig.json".to_string()];
        let mut second = config();
        second.projects = vec!["b/tsconfig.json".to_string(), "a/tsconfig.json".to_string()];

        assert_eq!(
            ts_types_lifecycle_digest(&first),
            ts_types_lifecycle_digest(&second)
        );
    }

    #[test]
    fn sidecar_cache_key_changes_with_the_compiler_version() {
        let config = config();
        assert_ne!(
            ts_types_sidecar_cache_key("digest", "5.9.3", "upstream", &config),
            ts_types_sidecar_cache_key("digest", "6.0.0", "upstream", &config)
        );
    }

    #[test]
    fn sidecar_cache_key_changes_with_the_sidecar_digest() {
        let config = config();
        assert_ne!(
            ts_types_sidecar_cache_key("one", "5.9.3", "upstream", &config),
            ts_types_sidecar_cache_key("two", "5.9.3", "upstream", &config)
        );
    }

    #[test]
    fn input_digest_changes_with_the_node_version() {
        let base = TsTypesCacheInputs {
            sidecar_digest: "digest".to_string(),
            typescript_version: "5.9.3".to_string(),
            node_version: "v22.0.0".to_string(),
            upstream_digest: "upstream".to_string(),
            lifecycle: config(),
        };
        let mut changed = base.clone();
        changed.node_version = "v24.0.0".to_string();

        assert_ne!(
            ts_types_input_digest(&base),
            ts_types_input_digest(&changed)
        );
    }
}
