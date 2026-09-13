use crate::go::lifecycle::GoAnalysisConfig;

pub const GO_SEMANTIC_SCHEMA_LABEL: &str = "go-semantic-facts-2";
pub const GO_SEMANTIC_PROVIDER_ID: &str = "polint.go.semantic";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticCacheInputs {
    pub sidecar_digest: String,
    pub go_version: String,
    pub x_tools_version: String,
    pub upstream_digest: String,
    pub lifecycle: GoAnalysisConfig,
}

pub fn go_semantic_provider_parameter_digest() -> String {
    crate::go::hash::stable_hash(&[
        GO_SEMANTIC_SCHEMA_LABEL,
        GO_SEMANTIC_PROVIDER_ID,
        "sidecar_digest",
        "go_version",
        "x_tools_version",
        "lifecycle_v1",
        "upstream_digest",
        // GO-05: the RTA-signal fact families grew the row vocabulary; a
        // vocabulary change must invalidate the downstream solver cache key (D-12).
        "address_taken_v1",
        "instantiated_type_v1",
        "dynamic_dispatch_v1",
    ])
}

pub fn go_semantic_sidecar_cache_key(
    sidecar_digest: &str,
    go_version: &str,
    upstream_digest: &str,
    config: &GoAnalysisConfig,
) -> String {
    let lifecycle_digest = go_semantic_lifecycle_digest(config);
    crate::go::hash::stable_hash(&[
        "go-semantic-sidecar-cache-v1",
        go_semantic_provider_parameter_digest().as_str(),
        sidecar_digest,
        go_version,
        upstream_digest,
        lifecycle_digest.as_str(),
    ])
}

pub fn go_semantic_input_digest(inputs: &GoSemanticCacheInputs) -> String {
    let lifecycle_digest = go_semantic_lifecycle_digest(&inputs.lifecycle);
    crate::go::hash::stable_hash(&[
        go_semantic_provider_parameter_digest().as_str(),
        inputs.sidecar_digest.as_str(),
        inputs.go_version.as_str(),
        inputs.x_tools_version.as_str(),
        inputs.upstream_digest.as_str(),
        lifecycle_digest.as_str(),
    ])
}

pub fn go_semantic_lifecycle_digest(config: &GoAnalysisConfig) -> String {
    // The budget is a lifecycle input, not a tuning knob: a run that exhausts
    // it stores an empty output, so a different budget can produce a different
    // outcome from identical sources.
    let mut parts = vec![
        format!("include_tests={}", config.include_tests),
        format!("offline={}", config.offline),
        format!(
            "semantic_timeout_ms={}",
            crate::go::semantic::budget::semantic_timeout(config.semantic_timeout_ms).as_millis()
        ),
    ];
    parts.extend(
        config
            .module_roots
            .iter()
            .map(|root| format!("module_root={root}")),
    );
    parts.extend(
        config
            .package_patterns
            .iter()
            .map(|pattern| format!("package_pattern={pattern}")),
    );
    parts.extend(
        config
            .build_tags
            .iter()
            .map(|tag| format!("build_tag={tag}")),
    );
    parts.sort();
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    crate::go::hash::stable_hash(&refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_digest_changes_when_build_tags_change() {
        let mut first = config();
        let mut second = config();
        second.build_tags.push("integration".to_string());

        assert_ne!(
            go_semantic_lifecycle_digest(&first),
            go_semantic_lifecycle_digest(&second)
        );
        first.build_tags.push("integration".to_string());
        assert_eq!(
            go_semantic_lifecycle_digest(&first),
            go_semantic_lifecycle_digest(&second)
        );
    }

    #[test]
    fn provider_parameter_digest_is_stable_and_non_empty() {
        assert!(!go_semantic_provider_parameter_digest().is_empty());
        assert_eq!(
            go_semantic_provider_parameter_digest(),
            go_semantic_provider_parameter_digest()
        );
    }

    #[test]
    fn provider_parameter_digest_locks_schema_label_and_rta_vocabulary() {
        // Trip-wire (GO-05 D-12): the schema label is bumped to -2 and the
        // RTA-signal fact families are folded into the parameter digest. If this fails,
        // the row vocabulary changed without a schema-label bump — that is a regression,
        // not a snapshot to bless. Reconstruct the exact locked parts list.
        assert_eq!(GO_SEMANTIC_SCHEMA_LABEL, "go-semantic-facts-2");
        let expected = crate::go::hash::stable_hash(&[
            "go-semantic-facts-2",
            GO_SEMANTIC_PROVIDER_ID,
            "sidecar_digest",
            "go_version",
            "x_tools_version",
            "lifecycle_v1",
            "upstream_digest",
            "address_taken_v1",
            "instantiated_type_v1",
            "dynamic_dispatch_v1",
        ]);
        assert_eq!(go_semantic_provider_parameter_digest(), expected);
    }

    #[test]
    fn provider_parameter_digest_differs_from_pre_phase48_recipe() {
        // The previous recipe (schema -1, no RTA vocabulary) must not collide with
        // the bumped recipe, so a cache built before this stage is invalidated.
        let pre_phase48 = crate::go::hash::stable_hash(&[
            "go-semantic-facts-1",
            GO_SEMANTIC_PROVIDER_ID,
            "sidecar_digest",
            "go_version",
            "x_tools_version",
            "lifecycle_v1",
            "upstream_digest",
        ]);
        assert_ne!(go_semantic_provider_parameter_digest(), pre_phase48);
    }

    #[test]
    fn input_digest_invalidates_on_sidecar_go_xtools_and_lifecycle() {
        let base = inputs();
        for changed in [
            GoSemanticCacheInputs {
                sidecar_digest: "sidecar-b".to_string(),
                ..base.clone()
            },
            GoSemanticCacheInputs {
                go_version: "go1.26.0".to_string(),
                ..base.clone()
            },
            GoSemanticCacheInputs {
                x_tools_version: "v0.46.0".to_string(),
                ..base.clone()
            },
            GoSemanticCacheInputs {
                lifecycle: GoAnalysisConfig {
                    build_tags: vec!["integration".to_string()],
                    ..base.lifecycle.clone()
                },
                ..base.clone()
            },
            GoSemanticCacheInputs {
                lifecycle: GoAnalysisConfig {
                    include_tests: false,
                    ..base.lifecycle.clone()
                },
                ..base.clone()
            },
        ] {
            assert_ne!(
                go_semantic_input_digest(&base),
                go_semantic_input_digest(&changed)
            );
        }
    }

    #[test]
    fn input_digest_preserves_hit_for_unrelated_config() {
        let base = inputs();
        let same_relevant_inputs = GoSemanticCacheInputs {
            lifecycle: GoAnalysisConfig {
                files_without_module_root: vec!["ignored.go".to_string()],
                ..base.lifecycle.clone()
            },
            ..base.clone()
        };

        assert_eq!(
            go_semantic_input_digest(&base),
            go_semantic_input_digest(&same_relevant_inputs)
        );
    }

    fn inputs() -> GoSemanticCacheInputs {
        GoSemanticCacheInputs {
            sidecar_digest: "sidecar-a".to_string(),
            go_version: "go1.25.0".to_string(),
            x_tools_version: "v0.45.0".to_string(),
            upstream_digest: "go-syntax-a".to_string(),
            lifecycle: config(),
        }
    }

    fn config() -> GoAnalysisConfig {
        GoAnalysisConfig {
            module_roots: vec![".".to_string()],
            package_patterns: vec!["./...".to_string()],
            build_tags: Vec::new(),
            include_tests: true,
            offline: false,
            semantic_timeout_ms: None,
            files_without_module_root: Vec::new(),
        }
    }
}
