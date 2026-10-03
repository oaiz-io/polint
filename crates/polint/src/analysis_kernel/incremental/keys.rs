#![cfg_attr(
    not(test),
    expect(dead_code, reason = "kept for private internal consumers")
)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::digest::{Digest, DigestKind};
use crate::analysis_api::TS_MODULE_RESOLVER;
use crate::analysis_kernel::ProviderManifest;
use crate::cache::{CACHE_VERSION, CacheKey};
use crate::core::AnalysisDb;
use crate::repo_fs::{
    TOPOLOGY_LOCKFILE_MAX_BYTES, TOPOLOGY_MANIFEST_MAX_BYTES, normalize_repo_relative,
    normalize_repo_relative_input, read_repo_file_to_string_with_limit, read_repo_file_with_limit,
    repo_dir_path, repo_file_exists, repo_file_path, repo_relative_existing_path,
};
use crate::ts::module_graph::parse_pnpm_workspace_packages;

pub(crate) const MODULE_GRAPH_TOPOLOGY_INPUT_FILE_NAMES: &[&str] = &[
    "go.mod",
    "go.work",
    "go.sum",
    "package.json",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "pnpm-lock.yaml",
    "pnpm-workspace.yaml",
    "yarn.lock",
    "bun.lock",
    "tsconfig.json",
];

fn module_graph_toolchain_digest(resolver: &str) -> Digest {
    Digest::from_parts(
        DigestKind::ToolInvocation,
        "module_graph_toolchain",
        &[resolver],
    )
}

pub(crate) use crate::analysis_api::LayerKind;

pub(crate) use crate::analysis_api::PrecisionTier;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LayerKey {
    pub(crate) layer_kind: LayerKind,
    pub(crate) provider_id: String,
    pub(crate) provider_version: String,
    pub(crate) schema_version: String,
    pub(crate) parameter_digest: Digest,
    pub(crate) lifecycle_digest: Digest,
    pub(crate) config_digest: Digest,
    pub(crate) toolchain_digest: Digest,
    pub(crate) input_digests: Arc<Vec<Digest>>,
    pub(crate) dependency_layer_digests: Arc<Vec<Digest>>,
    pub(crate) extension_digests: Arc<Vec<Digest>>,
}

// The order is field by field, as a derived order would be. A key's digest lists
// can hold one digest per input file, and every dependency edge of a layer
// carries a clone of its key that shares those lists. Comparing two shared lists
// answers without walking them, so sorting a layer's edges stays O(n log n)
// instead of walking tens of thousands of digests per comparison. (Equality
// already short-circuits on shared lists: `Arc` equality checks the pointer
// first when the contents implement `Eq`.)
impl Ord for LayerKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.layer_kind
            .cmp(&other.layer_kind)
            .then_with(|| self.provider_id.cmp(&other.provider_id))
            .then_with(|| self.provider_version.cmp(&other.provider_version))
            .then_with(|| self.schema_version.cmp(&other.schema_version))
            .then_with(|| self.parameter_digest.cmp(&other.parameter_digest))
            .then_with(|| self.lifecycle_digest.cmp(&other.lifecycle_digest))
            .then_with(|| self.config_digest.cmp(&other.config_digest))
            .then_with(|| self.toolchain_digest.cmp(&other.toolchain_digest))
            .then_with(|| shared_digests_cmp(&self.input_digests, &other.input_digests))
            .then_with(|| {
                shared_digests_cmp(
                    &self.dependency_layer_digests,
                    &other.dependency_layer_digests,
                )
            })
            .then_with(|| shared_digests_cmp(&self.extension_digests, &other.extension_digests))
    }
}

impl PartialOrd for LayerKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn shared_digests_cmp(left: &Arc<Vec<Digest>>, right: &Arc<Vec<Digest>>) -> std::cmp::Ordering {
    if Arc::ptr_eq(left, right) {
        std::cmp::Ordering::Equal
    } else {
        left.cmp(right)
    }
}

pub(crate) use crate::analysis_api::QueryKey;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct SummaryKey {
    #[serde(rename = "callable_stable_key")]
    pub(crate) callable_stable_key_text: String,
    pub(crate) summary_domain: String,
    pub(crate) summary_version: String,
    pub(crate) body_shape_digest: Digest,
    pub(crate) dependency_summary_digests: Vec<Digest>,
    pub(crate) extension_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct DiagnosticKey {
    pub(crate) rule_id: String,
    pub(crate) rule_version: String,
    pub(crate) rule_code_digest: Digest,
    pub(crate) options_digest: Digest,
    pub(crate) requested_view_digests: Vec<Digest>,
    pub(crate) evidence_digest: Digest,
}

impl LayerKey {
    #[expect(
        clippy::too_many_arguments,
        reason = "Layer cache identity is intentionally explicit so every digest input remains visible at construction."
    )]
    pub(crate) fn new(
        layer_kind: LayerKind,
        provider_id: impl Into<String>,
        provider_version: impl Into<String>,
        schema_version: impl Into<String>,
        parameter_digest: Digest,
        lifecycle_digest: Digest,
        config_digest: Digest,
        toolchain_digest: Digest,
        input_digests: Vec<Digest>,
        dependency_layer_digests: Vec<Digest>,
        extension_digests: Vec<Digest>,
    ) -> Self {
        Self {
            layer_kind,
            provider_id: provider_id.into(),
            provider_version: provider_version.into(),
            schema_version: schema_version.into(),
            parameter_digest,
            lifecycle_digest,
            config_digest,
            toolchain_digest,
            input_digests: Arc::new(sorted_digests(input_digests)),
            dependency_layer_digests: Arc::new(sorted_digests(dependency_layer_digests)),
            extension_digests: Arc::new(sorted_digests(extension_digests)),
        }
    }

    pub(crate) fn from_existing_file_cache(
        layer_kind: LayerKind,
        provider_id: impl Into<String>,
        provider_version: impl Into<String>,
        key: &CacheKey,
    ) -> Self {
        let version = if key.version.is_empty() {
            CACHE_VERSION
        } else {
            key.version.as_str()
        };
        let compatibility_input_digests = vec![
            Digest::from_parts(DigestKind::SourceText, "file_hash", &[&key.file_hash]),
            Digest::from_parts(DigestKind::Config, "config_hash", &[&key.config_hash]),
            Digest::from_parts(DigestKind::RuleCode, "rule_hash", &[&key.rule_hash]),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "plan_hash",
                &[&key.plan_hash],
            ),
            Digest::from_parts(DigestKind::ToolInvocation, "version", &[version]),
            Digest::from_parts(
                DigestKind::ToolInvocation,
                "parser_identity",
                &[&key.parser_identity],
            ),
            Digest::from_parts(DigestKind::ProviderOutput, "schema", &[&key.schema]),
        ];

        Self::new(
            layer_kind,
            provider_id,
            provider_version,
            key.schema.clone(),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "existing_file_cache_parameters",
                &[&key.rule_hash, &key.plan_hash],
            ),
            Digest::absent(DigestKind::DependencyLayer, "existing_file_cache_lifecycle"),
            Digest::from_parts(DigestKind::Config, "config_hash", &[&key.config_hash]),
            Digest::from_parts(
                DigestKind::ToolInvocation,
                "version",
                &[version, &key.parser_identity],
            ),
            compatibility_input_digests,
            Vec::new(),
            Vec::new(),
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Syntax layer identity must keep parser inputs explicit and separate from downstream rule inputs."
    )]
    pub(crate) fn syntax_layer_key(
        layer_kind: LayerKind,
        provider_id: impl Into<String>,
        provider_version: impl Into<String>,
        schema_version: impl Into<String>,
        source_text_digests: Vec<Digest>,
        config_digest: Digest,
        lifecycle_digest: Digest,
        toolchain_digest: Digest,
        parser_parameter_digest: Digest,
    ) -> Self {
        let provider_id = provider_id.into();
        debug_assert!(
            matches!(
                (layer_kind, provider_id.as_str()),
                (LayerKind::GoSyntax, "polint.go.syntax")
                    | (LayerKind::TsSyntax, "polint.ts.syntax")
            ),
            "syntax layer keys are only defined for Go and TS/JS syntax providers"
        );

        Self::new(
            layer_kind,
            provider_id,
            provider_version,
            schema_version,
            parser_parameter_digest,
            lifecycle_digest,
            config_digest,
            toolchain_digest,
            source_text_digests,
            Vec::new(),
            vec![Digest::absent(
                DigestKind::ExtensionCode,
                "extension_digest_absent",
            )],
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Module graph layer identity must keep import, lifecycle, config, upstream, and provider parameters explicit."
    )]
    pub(crate) fn module_graph_layer_key(
        manifest: &ProviderManifest,
        import_shape_digests: Vec<Digest>,
        source_package_digests: Vec<Digest>,
        config_digest: Digest,
        go_lifecycle_digest: Digest,
        ts_js_lifecycle_digest: Digest,
        upstream_syntax_output_digests: Vec<Digest>,
        module_graph_parameter_digest: Digest,
    ) -> Self {
        debug_assert_eq!(
            manifest.id, "polint.module_graph",
            "module graph layer keys require the module graph provider manifest"
        );

        let lifecycle_digest = Digest::from_unordered(
            DigestKind::ProviderParameters,
            "module_graph_lifecycle_inputs",
            vec![go_lifecycle_digest.clone(), ts_js_lifecycle_digest.clone()],
        );
        let mut input_digests =
            Vec::with_capacity(2 + import_shape_digests.len() + source_package_digests.len());
        input_digests.push(go_lifecycle_digest);
        input_digests.push(ts_js_lifecycle_digest);
        input_digests.extend(import_shape_digests);
        input_digests.extend(source_package_digests);

        Self::new(
            LayerKind::ModuleGraph,
            manifest.id,
            manifest.provider_version(),
            manifest.primary_schema_label(),
            module_graph_parameter_digest,
            lifecycle_digest,
            config_digest,
            module_graph_toolchain_digest(TS_MODULE_RESOLVER),
            input_digests,
            upstream_syntax_output_digests
                .into_iter()
                .map(dependency_layer_digest)
                .collect(),
            vec![Digest::absent(
                DigestKind::ExtensionCode,
                "extension_digest_absent",
            )],
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Symbol graph layer identity must keep source, import, lifecycle, upstream, and provider inputs explicit."
    )]
    pub(crate) fn symbol_graph_layer_key(
        manifest: &ProviderManifest,
        source_function_digests: Vec<Digest>,
        package_context_digests: Vec<Digest>,
        import_shape_digests: Vec<Digest>,
        config_digest: Digest,
        go_lifecycle_digest: Digest,
        ts_js_lifecycle_digest: Digest,
        module_graph_output_digest: Digest,
        upstream_syntax_output_digests: Vec<Digest>,
        symbol_graph_parameter_digest: Digest,
    ) -> Self {
        debug_assert_eq!(
            manifest.id, "polint.symbol_graph",
            "symbol graph layer keys require the symbol graph provider manifest"
        );

        let lifecycle_digest = Digest::from_unordered(
            DigestKind::ProviderParameters,
            "symbol_graph_lifecycle_inputs",
            vec![go_lifecycle_digest.clone(), ts_js_lifecycle_digest.clone()],
        );
        let mut input_digests = Vec::with_capacity(
            2 + source_function_digests.len()
                + package_context_digests.len()
                + import_shape_digests.len(),
        );
        input_digests.push(go_lifecycle_digest);
        input_digests.push(ts_js_lifecycle_digest);
        input_digests.extend(source_function_digests);
        input_digests.extend(package_context_digests);
        input_digests.extend(import_shape_digests);

        let mut dependency_layer_digests =
            Vec::with_capacity(1 + upstream_syntax_output_digests.len());
        dependency_layer_digests.push(dependency_layer_digest(module_graph_output_digest));
        dependency_layer_digests.extend(
            upstream_syntax_output_digests
                .into_iter()
                .map(dependency_layer_digest),
        );

        Self::new(
            LayerKind::SymbolGraph,
            manifest.id,
            manifest.provider_version(),
            manifest.primary_schema_label(),
            symbol_graph_parameter_digest,
            lifecycle_digest,
            config_digest,
            Digest::absent(DigestKind::ToolInvocation, "symbol_graph_toolchain"),
            input_digests,
            dependency_layer_digests,
            vec![Digest::absent(
                DigestKind::ExtensionCode,
                "extension_digest_absent",
            )],
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Module topology identity must keep base topology, semantic, and upstream provider inputs explicit."
    )]
    pub(crate) fn module_topology_layer_key(
        manifest: &ProviderManifest,
        import_shape_digests: Vec<Digest>,
        base_topology_digests: Vec<Digest>,
        config_digest: Digest,
        go_lifecycle_digest: Digest,
        ts_js_lifecycle_digest: Digest,
        module_graph_output_digest: Digest,
        symbol_graph_output_digest: Digest,
        semantic_provider_parameter_digest: Digest,
    ) -> Self {
        debug_assert_eq!(
            manifest.id, "polint.module_topology",
            "module topology layer keys require the module topology provider manifest"
        );

        let lifecycle_digest = Digest::from_unordered(
            DigestKind::ProviderParameters,
            "module_topology_lifecycle_inputs",
            vec![go_lifecycle_digest.clone(), ts_js_lifecycle_digest.clone()],
        );
        let parameter_digest = Digest::from_unordered(
            DigestKind::ProviderParameters,
            "module_topology_parameters",
            vec![
                semantic_provider_parameter_digest.clone(),
                Digest::from_parts(
                    DigestKind::ProviderParameters,
                    "module_topology_outputs",
                    &["import_to_package_edges"],
                ),
            ],
        );
        let mut input_digests =
            Vec::with_capacity(3 + import_shape_digests.len() + base_topology_digests.len());
        input_digests.push(go_lifecycle_digest);
        input_digests.push(ts_js_lifecycle_digest);
        input_digests.push(semantic_provider_parameter_digest);
        input_digests.extend(import_shape_digests);
        input_digests.extend(base_topology_digests);

        Self::new(
            LayerKind::ModuleTopology,
            manifest.id,
            manifest.provider_version(),
            manifest.primary_schema_label(),
            parameter_digest,
            lifecycle_digest,
            config_digest,
            Digest::absent(DigestKind::ToolInvocation, "module_topology_toolchain"),
            input_digests,
            vec![
                dependency_layer_digest(module_graph_output_digest),
                dependency_layer_digest(symbol_graph_output_digest),
            ],
            vec![Digest::absent(
                DigestKind::ExtensionCode,
                "extension_digest_absent",
            )],
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Semantic MIR layer identity must keep source, lifecycle, upstream, and provider inputs explicit."
    )]
    pub(crate) fn semantic_mir_layer_key(
        manifest: &ProviderManifest,
        source_function_digests: Vec<Digest>,
        config_digest: Digest,
        go_lifecycle_digest: Digest,
        ts_js_lifecycle_digest: Digest,
        upstream_syntax_output_digests: Vec<Digest>,
        symbol_graph_output_digest: Digest,
        module_topology_output_digest: Digest,
        semantic_mir_parameter_digest: Digest,
    ) -> Self {
        debug_assert_eq!(
            manifest.id, "polint.semantic_mir",
            "semantic MIR layer keys require the semantic MIR provider manifest"
        );

        let lifecycle_digest = Digest::from_unordered(
            DigestKind::ProviderParameters,
            "semantic_mir_lifecycle_inputs",
            vec![go_lifecycle_digest.clone(), ts_js_lifecycle_digest.clone()],
        );
        let mut input_digests = Vec::with_capacity(2 + source_function_digests.len());
        input_digests.push(go_lifecycle_digest);
        input_digests.push(ts_js_lifecycle_digest);
        input_digests.extend(source_function_digests);

        let mut dependency_layer_digests =
            Vec::with_capacity(2 + upstream_syntax_output_digests.len());
        dependency_layer_digests.extend(
            upstream_syntax_output_digests
                .into_iter()
                .map(dependency_layer_digest),
        );
        dependency_layer_digests.push(dependency_layer_digest(symbol_graph_output_digest));
        dependency_layer_digests.push(dependency_layer_digest(module_topology_output_digest));

        Self::new(
            LayerKind::SemanticMir,
            manifest.id,
            manifest.provider_version(),
            manifest.primary_schema_label(),
            semantic_mir_parameter_digest,
            lifecycle_digest,
            config_digest,
            Digest::absent(DigestKind::ToolInvocation, "semantic_mir_toolchain"),
            input_digests,
            dependency_layer_digests,
            vec![
                Digest::absent(DigestKind::ExtensionCode, "extension_digest_absent"),
                Digest::absent(DigestKind::ModelFile, "model_digest_absent"),
            ],
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "CFG layer identity must keep semantic MIR, syntax, lifecycle, view, and provider inputs explicit."
    )]
    pub(crate) fn cfg_layer_key(
        manifest: &ProviderManifest,
        source_function_digests: Vec<Digest>,
        config_digest: Digest,
        go_lifecycle_digest: Digest,
        ts_js_lifecycle_digest: Digest,
        upstream_syntax_output_digests: Vec<Digest>,
        semantic_mir_output_digest: Digest,
        cfg_parameter_digest: Digest,
    ) -> Self {
        debug_assert_eq!(
            manifest.id, "polint.cfg",
            "CFG layer keys require the CFG provider manifest"
        );

        let lifecycle_digest = Digest::from_unordered(
            DigestKind::ProviderParameters,
            "cfg_lifecycle_inputs",
            vec![go_lifecycle_digest.clone(), ts_js_lifecycle_digest.clone()],
        );
        let view_digest = Digest::from_parts(
            DigestKind::ProviderParameters,
            "cfg_graph_views",
            &["normal_control_view", "abrupt_aware_view"],
        );
        let parameter_digest = Digest::from_unordered(
            DigestKind::ProviderParameters,
            "cfg_parameters",
            vec![cfg_parameter_digest, view_digest.clone()],
        );
        let mut input_digests = Vec::with_capacity(3 + source_function_digests.len());
        input_digests.push(go_lifecycle_digest);
        input_digests.push(ts_js_lifecycle_digest);
        input_digests.push(view_digest);
        input_digests.extend(source_function_digests);

        let mut dependency_layer_digests =
            Vec::with_capacity(1 + upstream_syntax_output_digests.len());
        dependency_layer_digests.push(dependency_layer_digest(semantic_mir_output_digest));
        dependency_layer_digests.extend(
            upstream_syntax_output_digests
                .into_iter()
                .map(dependency_layer_digest),
        );

        Self::new(
            LayerKind::Cfg,
            manifest.id,
            manifest.provider_version(),
            manifest.primary_schema_label(),
            parameter_digest,
            lifecycle_digest,
            config_digest,
            Digest::absent(DigestKind::ToolInvocation, "cfg_toolchain"),
            input_digests,
            dependency_layer_digests,
            vec![
                Digest::absent(DigestKind::ExtensionCode, "extension_digest_absent"),
                Digest::absent(DigestKind::ModelFile, "model_digest_absent"),
            ],
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Calls layer identity must keep direct-call source, lifecycle, upstream, and provider inputs explicit."
    )]
    pub(crate) fn calls_layer_key(
        manifest: &ProviderManifest,
        source_function_digests: Vec<Digest>,
        config_digest: Digest,
        go_lifecycle_digest: Digest,
        ts_js_lifecycle_digest: Digest,
        upstream_syntax_output_digests: Vec<Digest>,
        semantic_mir_output_digest: Digest,
        cfg_output_digest: Digest,
        symbol_graph_output_digest: Digest,
        module_topology_output_digest: Digest,
        calls_parameter_digest: Digest,
    ) -> Self {
        debug_assert_eq!(
            manifest.id, "polint.calls",
            "calls layer keys require the calls provider manifest"
        );

        let lifecycle_digest = Digest::from_unordered(
            DigestKind::ProviderParameters,
            "calls_lifecycle_inputs",
            vec![go_lifecycle_digest.clone(), ts_js_lifecycle_digest.clone()],
        );
        let parameter_digest = Digest::from_unordered(
            DigestKind::ProviderParameters,
            "calls_parameters",
            vec![calls_parameter_digest.clone()],
        );
        let mut input_digests = Vec::with_capacity(3 + source_function_digests.len());
        input_digests.push(go_lifecycle_digest);
        input_digests.push(ts_js_lifecycle_digest);
        input_digests.push(calls_parameter_digest);
        input_digests.extend(source_function_digests);

        let mut dependency_layer_digests =
            Vec::with_capacity(4 + upstream_syntax_output_digests.len());
        dependency_layer_digests.extend(
            upstream_syntax_output_digests
                .into_iter()
                .map(dependency_layer_digest),
        );
        dependency_layer_digests.push(dependency_layer_digest(semantic_mir_output_digest));
        dependency_layer_digests.push(dependency_layer_digest(cfg_output_digest));
        dependency_layer_digests.push(dependency_layer_digest(symbol_graph_output_digest));
        dependency_layer_digests.push(dependency_layer_digest(module_topology_output_digest));

        Self::new(
            LayerKind::Calls,
            manifest.id,
            manifest.provider_version(),
            manifest.primary_schema_label(),
            parameter_digest,
            lifecycle_digest,
            config_digest,
            Digest::absent(DigestKind::ToolInvocation, "calls_toolchain"),
            input_digests,
            dependency_layer_digests,
            vec![
                Digest::absent(DigestKind::ExtensionCode, "extension_digest_absent"),
                Digest::absent(DigestKind::ModelFile, "model_digest_absent"),
            ],
        )
    }

    // Layer cache reuse for direct summaries supports demand queries and SCC caching.
    #[expect(dead_code, reason = "kept for private internal consumers")]
    #[expect(
        clippy::too_many_arguments,
        reason = "Direct summaries layer cache identity is intentionally explicit so every upstream digest input remains visible."
    )]
    pub(crate) fn direct_summaries_layer_key(
        manifest: &ProviderManifest,
        source_function_digests: Vec<Digest>,
        config_digest: Digest,
        go_lifecycle_digest: Digest,
        ts_js_lifecycle_digest: Digest,
        upstream_syntax_output_digests: Vec<Digest>,
        semantic_mir_output_digest: Digest,
        cfg_output_digest: Digest,
        calls_output_digest: Digest,
        abstract_domains_output_digest: Digest,
        symbol_graph_output_digest: Digest,
        module_topology_output_digest: Digest,
        direct_summaries_parameter_digest: Digest,
    ) -> Self {
        debug_assert_eq!(
            manifest.id, "polint.direct_summaries",
            "direct-summaries layer keys require the direct summaries provider manifest"
        );

        let lifecycle_digest = Digest::from_unordered(
            DigestKind::ProviderParameters,
            "direct_summaries_lifecycle_inputs",
            vec![go_lifecycle_digest.clone(), ts_js_lifecycle_digest.clone()],
        );
        let parameter_digest = Digest::from_unordered(
            DigestKind::ProviderParameters,
            "direct_summaries_parameters",
            vec![direct_summaries_parameter_digest.clone()],
        );
        let mut input_digests = Vec::with_capacity(3 + source_function_digests.len());
        input_digests.push(go_lifecycle_digest);
        input_digests.push(ts_js_lifecycle_digest);
        input_digests.push(direct_summaries_parameter_digest);
        input_digests.extend(source_function_digests);

        let mut dependency_layer_digests =
            Vec::with_capacity(6 + upstream_syntax_output_digests.len());
        dependency_layer_digests.extend(
            upstream_syntax_output_digests
                .into_iter()
                .map(dependency_layer_digest),
        );
        dependency_layer_digests.push(dependency_layer_digest(semantic_mir_output_digest));
        dependency_layer_digests.push(dependency_layer_digest(cfg_output_digest));
        dependency_layer_digests.push(dependency_layer_digest(calls_output_digest));
        dependency_layer_digests.push(dependency_layer_digest(abstract_domains_output_digest));
        dependency_layer_digests.push(dependency_layer_digest(symbol_graph_output_digest));
        dependency_layer_digests.push(dependency_layer_digest(module_topology_output_digest));

        Self::new(
            LayerKind::DirectSummaries,
            manifest.id,
            manifest.provider_version(),
            manifest.primary_schema_label(),
            parameter_digest,
            lifecycle_digest,
            config_digest,
            Digest::absent(DigestKind::ToolInvocation, "direct_summaries_toolchain"),
            input_digests,
            dependency_layer_digests,
            vec![
                Digest::absent(DigestKind::ExtensionCode, "extension_digest_absent"),
                Digest::absent(DigestKind::ModelFile, "model_digest_absent"),
            ],
        )
    }

    /// Combines many digests into one stable digest (a hash of their values).
    /// Used to keep layer keys compact when a provider depends on a large,
    /// per-file/per-function input set whose fine-grained invalidation is already
    /// expressed as dependency edges.
    fn combine_digests_into(kind: DigestKind, label: &str, digests: &[Digest]) -> Digest {
        let parts = digests
            .iter()
            .map(|digest| digest.value.as_str())
            .collect::<Vec<_>>();
        Digest::from_parts(kind, label, &parts)
    }

    pub(crate) fn metrics_layer_key(
        manifest: &ProviderManifest,
        source_text_digests: Vec<Digest>,
        function_fact_digests: Vec<Digest>,
        metrics_parameter_digest: Digest,
    ) -> Self {
        debug_assert_eq!(
            manifest.id, "polint.metrics",
            "metrics layer keys require the metrics provider manifest"
        );

        // Fold the per-file and per-function digests into two combined digests
        // instead of storing all of them (one per file + one per function) in the
        // key. Fine-grained, per-input incremental invalidation is driven by the
        // dependency EDGES emitted in `metrics::metrics_layer_dependency_edges`
        // (one `to` node per file/function), not by the key's `input_digests`.
        // Embedding every digest in the key as well made the key O(files+functions)
        // in size, and because `CacheNode::Layer(key)` is cloned into the `from`
        // field of every dependency edge, dependency-edge construction became
        // O(files * (files+functions)) in time and memory — tens of GB and minutes
        // on large repos. The combined digests still change whenever any source or
        // function digest changes, so the key identity and the coarse
        // `input_digests != input_digests` change check stay correct.
        let input_digests = vec![
            Self::combine_digests_into(
                DigestKind::SourceText,
                "metrics_source_text_combined",
                &source_text_digests,
            ),
            Self::combine_digests_into(
                DigestKind::ProviderParameters,
                "metrics_function_fact_combined",
                &function_fact_digests,
            ),
        ];

        Self::new(
            LayerKind::Metrics,
            manifest.id,
            manifest.provider_version(),
            manifest.primary_schema_label(),
            metrics_parameter_digest,
            Digest::absent(DigestKind::ProviderParameters, "metrics_lifecycle"),
            Digest::absent(DigestKind::Config, "metrics_config"),
            Digest::absent(DigestKind::ToolInvocation, "metrics_toolchain"),
            input_digests,
            Vec::new(),
            vec![
                Digest::absent(DigestKind::ExtensionCode, "extension_digest_absent"),
                Digest::absent(DigestKind::ModelFile, "model_digest_absent"),
            ],
        )
    }
}

impl SummaryKey {
    pub(crate) fn new(
        callable_stable_key_text: impl Into<String>,
        summary_domain: impl Into<String>,
        summary_version: impl Into<String>,
        body_shape_digest: Digest,
        dependency_summary_digests: Vec<Digest>,
        extension_digest: Digest,
    ) -> Self {
        Self {
            callable_stable_key_text: callable_stable_key_text.into(),
            summary_domain: summary_domain.into(),
            summary_version: summary_version.into(),
            body_shape_digest,
            dependency_summary_digests: sorted_digests(dependency_summary_digests),
            extension_digest,
        }
    }
}

impl DiagnosticKey {
    pub(crate) fn new(
        rule_id: impl Into<String>,
        rule_version: impl Into<String>,
        rule_code_digest: Digest,
        options_digest: Digest,
        requested_view_digests: Vec<Digest>,
        evidence_digest: Digest,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            rule_version: rule_version.into(),
            rule_code_digest,
            options_digest,
            requested_view_digests: sorted_digests(requested_view_digests),
            evidence_digest,
        }
    }
}

fn sorted_digests(mut digests: Vec<Digest>) -> Vec<Digest> {
    digests.sort();
    digests
}

pub(crate) fn dependency_layer_digest(output_digest: Digest) -> Digest {
    Digest::from_parts(
        DigestKind::DependencyLayer,
        "upstream_layer_output",
        &[&output_digest.to_string()],
    )
}

pub(crate) fn module_graph_topology_input_digests(root: &Path, db: &AnalysisDb) -> Vec<Digest> {
    module_graph_topology_input_digest_rows(root, db)
        .into_iter()
        .map(|(_, digest)| digest)
        .collect()
}

pub(crate) fn module_graph_topology_input_digest_rows(
    root: &Path,
    db: &AnalysisDb,
) -> Vec<(String, Digest)> {
    let dirs = topology_relevant_dirs(db);
    let mut candidate_paths = dirs
        .into_iter()
        .flat_map(|dir| {
            MODULE_GRAPH_TOPOLOGY_INPUT_FILE_NAMES
                .iter()
                .map(move |file_name| topology_input_relative_path(&dir, file_name))
        })
        .collect::<Vec<_>>();
    candidate_paths.extend(workspace_member_topology_input_candidates(
        root,
        candidate_paths.iter().map(String::as_str),
    ));
    candidate_paths.extend(extended_ts_config_candidates(
        root,
        candidate_paths.iter().map(String::as_str),
    ));

    let mut rows = candidate_paths
        .into_iter()
        .filter_map(|relative_path| {
            let bytes = match read_repo_file_with_limit(
                root,
                &relative_path,
                topology_input_max_bytes(&relative_path),
            ) {
                Ok(bytes) => bytes,
                Err(error) if error.is_not_found() => return None,
                Err(error) => {
                    let digest = Digest::from_parts(
                        DigestKind::ProviderParameters,
                        "module_graph_topology_input",
                        &[relative_path.as_str(), error.stable_reason()],
                    );
                    return Some((relative_path, digest));
                }
            };
            let content_hash = stable_hash_bytes(&bytes);
            let digest = Digest::from_parts(
                DigestKind::ProviderParameters,
                "module_graph_topology_input",
                &[relative_path.as_str(), content_hash.as_str()],
            );
            Some((relative_path, digest))
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows.dedup();
    rows
}

fn topology_input_max_bytes(relative_path: &str) -> u64 {
    match Path::new(relative_path)
        .file_name()
        .and_then(|name| name.to_str())
    {
        Some(
            "package-lock.json"
            | "npm-shrinkwrap.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "bun.lock",
        ) => TOPOLOGY_LOCKFILE_MAX_BYTES,
        _ => TOPOLOGY_MANIFEST_MAX_BYTES,
    }
}

fn workspace_member_topology_input_candidates<'a>(
    root: &Path,
    initial_candidates: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut package_paths = BTreeSet::new();
    for candidate in initial_candidates {
        if Path::new(candidate)
            .file_name()
            .and_then(|name| name.to_str())
            != Some("package.json")
        {
            continue;
        }
        let Some(package_path) = package_path_for_manifest_path(candidate) else {
            continue;
        };
        if !repo_file_exists(root, candidate) {
            continue;
        }
        let Ok(contents) =
            read_repo_file_to_string_with_limit(root, candidate, TOPOLOGY_MANIFEST_MAX_BYTES)
        else {
            continue;
        };
        for workspace in package_json_workspace_patterns(&contents) {
            package_paths.extend(expand_package_workspace_glob(
                root,
                &package_path,
                &workspace,
            ));
        }
    }
    if let Ok(contents) = read_repo_file_to_string_with_limit(
        root,
        "pnpm-workspace.yaml",
        TOPOLOGY_MANIFEST_MAX_BYTES,
    ) {
        for workspace in parse_pnpm_workspace_packages(&contents) {
            package_paths.extend(expand_package_workspace_glob(root, ".", &workspace));
        }
    }

    package_paths
        .into_iter()
        .flat_map(|package_path| {
            MODULE_GRAPH_TOPOLOGY_INPUT_FILE_NAMES
                .iter()
                .map(move |file_name| topology_input_relative_path(&package_path, file_name))
        })
        .collect()
}

fn package_path_for_manifest_path(relative_path: &str) -> Option<String> {
    if relative_path == "package.json" {
        Some(".".to_string())
    } else {
        relative_path
            .strip_suffix("/package.json")
            .map(str::to_string)
    }
}

fn package_json_workspace_patterns(contents: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(contents) else {
        return Vec::new();
    };
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    match object.get("workspaces") {
        Some(Value::Array(entries)) => entries
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::Object(object)) => object
            .get("packages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn expand_package_workspace_glob(root: &Path, package_path: &str, pattern: &str) -> Vec<String> {
    let Some(base) = pattern.strip_suffix("/*") else {
        return Vec::new();
    };
    let base_path = if package_path == "." {
        PathBuf::from(base)
    } else {
        Path::new(package_path).join(base)
    };
    let Some(base_path) = normalize_repo_relative_input(&base_path) else {
        return Vec::new();
    };
    let Ok(base_dir) = repo_dir_path(root, &base_path) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(base_dir) else {
        return Vec::new();
    };
    let mut members = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_type().ok()?.is_dir().then_some(entry.path()))
        .filter_map(|path| repo_relative_existing_path(root, &path))
        .filter(|path| repo_file_exists(root, topology_input_relative_path(path, "package.json")))
        .collect::<Vec<_>>();
    members.sort();
    members
}

fn topology_relevant_dirs(db: &AnalysisDb) -> BTreeSet<String> {
    let mut dirs = BTreeSet::from([".".to_string()]);
    for file in db.files() {
        let normalized = normalize_relative_path(&file.relative_path);
        let mut current = Path::new(&normalized).parent();
        while let Some(dir) = current {
            dirs.insert(path_to_repo_string(dir));
            current = dir.parent();
        }
    }
    dirs
}

fn extended_ts_config_candidates<'a>(
    root: &Path,
    initial_candidates: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut visited = BTreeSet::new();
    let mut discovered = BTreeSet::new();
    for candidate in initial_candidates {
        let file_name = Path::new(candidate)
            .file_name()
            .and_then(|name| name.to_str());
        if !matches!(file_name, Some("tsconfig.json" | "jsconfig.json")) {
            continue;
        }
        collect_extended_ts_configs(root, &root.join(candidate), &mut visited, &mut discovered);
    }
    discovered.into_iter().collect()
}

fn collect_extended_ts_configs(
    root: &Path,
    config_path: &Path,
    visited: &mut BTreeSet<String>,
    discovered: &mut BTreeSet<String>,
) {
    let Some(config_path) = repo_relative_existing_path(root, config_path) else {
        return;
    };
    if !visited.insert(config_path.clone()) {
        return;
    }
    let Some(config) = read_tsconfig_extends_wire(root, &config_path) else {
        return;
    };
    let config_dir = Path::new(&config_path).parent().unwrap_or(Path::new(""));
    for specifier in config
        .extends
        .into_iter()
        .flat_map(TsconfigExtendsWire::into_specifiers)
    {
        let Some(extended_path) = resolve_tsconfig_extends_path(root, config_dir, &specifier)
        else {
            continue;
        };
        discovered.insert(extended_path.clone());
        collect_extended_ts_configs(root, &root.join(&extended_path), visited, discovered);
    }
}

fn read_tsconfig_extends_wire(root: &Path, relative_path: &str) -> Option<TsconfigExtendsOnlyWire> {
    let Ok(mut source) =
        read_repo_file_to_string_with_limit(root, relative_path, TOPOLOGY_MANIFEST_MAX_BYTES)
    else {
        return None;
    };
    if let Some(stripped) = source.strip_prefix('\u{feff}') {
        source = stripped.to_string();
    }
    if json_strip_comments::strip(&mut source).is_err() {
        return None;
    }
    serde_json::from_str::<TsconfigExtendsOnlyWire>(&source).ok()
}

fn resolve_tsconfig_extends_path(
    root: &Path,
    config_dir: &Path,
    specifier: &str,
) -> Option<String> {
    let specifier_path = Path::new(specifier);
    if specifier_path.is_absolute() {
        return None;
    }
    if specifier.starts_with('.') {
        return resolve_tsconfig_file_candidate(root, &config_dir.join(specifier_path));
    }
    resolve_package_tsconfig_extends_path(root, config_dir, specifier)
}

fn resolve_package_tsconfig_extends_path(
    root: &Path,
    config_dir: &Path,
    specifier: &str,
) -> Option<String> {
    let mut current = normalize_repo_relative_input(config_dir)?;
    loop {
        let candidate = if current.as_os_str().is_empty() {
            PathBuf::from("node_modules").join(specifier)
        } else {
            current.join("node_modules").join(specifier)
        };
        if let Some(resolved) = resolve_tsconfig_file_candidate(root, &candidate) {
            return Some(resolved);
        }
        if current.as_os_str().is_empty() {
            return None;
        }
        current.pop();
    }
}

fn resolve_tsconfig_file_candidate(root: &Path, base: &Path) -> Option<String> {
    let mut candidates = vec![base.to_path_buf()];
    if base.extension().and_then(|extension| extension.to_str()) != Some("json") {
        let mut with_json = base.as_os_str().to_owned();
        with_json.push(".json");
        candidates.push(PathBuf::from(with_json));
    }
    candidates.push(base.join("tsconfig.json"));

    candidates
        .into_iter()
        .filter_map(|candidate| normalized_existing_repo_file(root, &candidate))
        .next()
}

fn normalized_existing_repo_file(root: &Path, candidate: &Path) -> Option<String> {
    repo_file_path(root, candidate).ok()?;
    let normalized = normalize_repo_relative_input(candidate)?;
    let relative = normalized.to_string_lossy();
    if relative.is_empty() {
        None
    } else {
        normalize_repo_relative(relative)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TsconfigExtendsOnlyWire {
    #[serde(default)]
    extends: Option<TsconfigExtendsWire>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TsconfigExtendsWire {
    Single(String),
    Multiple(Vec<String>),
}

impl TsconfigExtendsWire {
    fn into_specifiers(self) -> Vec<String> {
        match self {
            Self::Single(specifier) => vec![specifier],
            Self::Multiple(specifiers) => specifiers,
        }
    }
}

fn topology_input_relative_path(dir: &str, file_name: &str) -> String {
    if dir == "." {
        file_name.to_string()
    } else {
        format!("{dir}/{file_name}")
    }
}

fn normalize_relative_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn path_to_repo_string(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    if path.is_empty() {
        ".".to_string()
    } else {
        path
    }
}

fn stable_hash_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(crate) fn semantic_provider_parameter_digest() -> Digest {
    Digest::from_parts(
        DigestKind::ProviderParameters,
        "semantic_provider_parameters",
        &[
            "scopes=enabled",
            "semantic_imports=enabled",
            "exports=enabled",
            "aliases=enabled",
            "resolution_facts=enabled",
            "generated_symbols=enabled",
            "stable_exports=enabled",
            "alias_closure=max_input_plus_one",
            "generated_hooks=native_rows_only",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::AnalysisDb;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn digest(label: &str, value: &str) -> Digest {
        Digest::from_parts(DigestKind::SourceText, label, &[value])
    }

    fn module_graph_manifest() -> &'static crate::analysis_kernel::ProviderManifest {
        crate::analysis_kernel::AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.module_graph")
            .expect("module graph provider manifest exists")
    }

    fn module_graph_key(
        import_shape_digest: Digest,
        go_lifecycle_digest: Digest,
        ts_js_lifecycle_digest: Digest,
        config_digest: Digest,
        upstream_syntax_output_digest: Digest,
    ) -> LayerKey {
        LayerKey::module_graph_layer_key(
            module_graph_manifest(),
            vec![import_shape_digest],
            vec![Digest::from_parts(
                DigestKind::SourceText,
                "source_package",
                &["src/app.ts", "hash", "pkg"],
            )],
            config_digest,
            go_lifecycle_digest,
            ts_js_lifecycle_digest,
            vec![upstream_syntax_output_digest],
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "module_graph_parameters",
                &["resolver=default"],
            ),
        )
    }

    fn symbol_graph_manifest() -> &'static crate::analysis_kernel::ProviderManifest {
        crate::analysis_kernel::AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.symbol_graph")
            .expect("symbol graph provider manifest exists")
    }

    fn module_topology_manifest() -> &'static crate::analysis_kernel::ProviderManifest {
        crate::analysis_kernel::AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.module_topology")
            .expect("module topology provider manifest exists")
    }

    fn symbol_graph_key(
        source_function_digest: Digest,
        import_shape_digest: Digest,
        config_digest: Digest,
        go_lifecycle_digest: Digest,
        ts_js_lifecycle_digest: Digest,
        module_graph_output_digest: Digest,
        syntax_output_digest: Digest,
    ) -> LayerKey {
        LayerKey::symbol_graph_layer_key(
            symbol_graph_manifest(),
            vec![source_function_digest],
            vec![Digest::from_parts(
                DigestKind::ProviderParameters,
                "package_context",
                &["src/app.ts", "pkg"],
            )],
            vec![import_shape_digest],
            config_digest,
            go_lifecycle_digest,
            ts_js_lifecycle_digest,
            module_graph_output_digest,
            vec![syntax_output_digest],
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "symbol_graph_parameters",
                &["symbols", "references"],
            ),
        )
    }

    fn metrics_manifest() -> &'static crate::analysis_kernel::ProviderManifest {
        crate::analysis_kernel::AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.metrics")
            .expect("metrics provider manifest exists")
    }

    fn metrics_key(
        source_digest: Digest,
        function_digest: Digest,
        _config_digest: Digest,
        _syntax_output_digest: Digest,
    ) -> LayerKey {
        LayerKey::metrics_layer_key(
            metrics_manifest(),
            vec![source_digest],
            vec![function_digest],
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "metrics_parameters",
                &["file_metrics", "function_metrics", "complexity_metrics"],
            ),
        )
    }

    fn module_topology_key(
        import_shape_digest: Digest,
        base_topology_digest: Digest,
        module_graph_output_digest: Digest,
        symbol_graph_output_digest: Digest,
        semantic_parameter_digest: Digest,
    ) -> LayerKey {
        LayerKey::module_topology_layer_key(
            module_topology_manifest(),
            vec![import_shape_digest],
            vec![base_topology_digest],
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            module_graph_output_digest,
            symbol_graph_output_digest,
            semantic_parameter_digest,
        )
    }

    fn syntax_key(source_text_digests: Vec<Digest>) -> LayerKey {
        LayerKey::syntax_layer_key(
            LayerKind::GoSyntax,
            "polint.go.syntax",
            "1",
            "go-facts-v2",
            source_text_digests,
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "lifecycle", &["base"]),
            Digest::from_parts(DigestKind::ToolInvocation, "toolchain", &["base"]),
            Digest::from_parts(DigestKind::ProviderParameters, "parser", &["base"]),
        )
    }

    #[test]
    fn module_graph_layer_key_topology_inputs_change_on_manifest_lock_workspace_and_tsconfig() {
        let temp = tempfile::tempdir().expect("tempdir");
        let topology_files = [
            "go.mod",
            "go.work",
            "go.sum",
            "package.json",
            "package-lock.json",
            "npm-shrinkwrap.json",
            "pnpm-lock.yaml",
            "pnpm-workspace.yaml",
            "yarn.lock",
            "bun.lock",
            "tsconfig.json",
        ];
        for name in topology_files {
            write_file(temp.path(), name, &format!("{name}: base\n"));
        }
        let mut db = AnalysisDb::new();
        add_file(&mut db, temp.path(), "src/app.ts", "export {};\n");

        let base = module_graph_topology_input_digests(temp.path(), &db);
        assert_eq!(base.len(), topology_files.len());

        for changed_name in topology_files {
            write_file(
                temp.path(),
                changed_name,
                &format!("{changed_name}: changed\n"),
            );
            let changed = module_graph_topology_input_digests(temp.path(), &db);
            assert_ne!(
                base, changed,
                "{changed_name} should affect topology inputs"
            );
            write_file(
                temp.path(),
                changed_name,
                &format!("{changed_name}: base\n"),
            );
        }
    }

    #[test]
    fn module_graph_layer_key_topology_inputs_follow_tsconfig_extends() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "tsconfig.json",
            r#"{"extends":"./tsconfig.base.json"}"#,
        );
        write_file(
            temp.path(),
            "tsconfig.base.json",
            r#"{"compilerOptions":{"baseUrl":"."}}"#,
        );
        let mut db = AnalysisDb::new();
        add_file(&mut db, temp.path(), "src/app.ts", "export {};\n");

        let base = module_graph_topology_input_digest_rows(temp.path(), &db);
        assert!(base.iter().any(|(path, _)| path == "tsconfig.base.json"));

        write_file(
            temp.path(),
            "tsconfig.base.json",
            r#"{"compilerOptions":{"baseUrl":"src"}}"#,
        );
        let changed = module_graph_topology_input_digest_rows(temp.path(), &db);

        assert_ne!(base, changed);
    }

    #[test]
    fn module_graph_layer_key_topology_inputs_reject_absolute_tsconfig_extends() {
        let repo = tempfile::tempdir().expect("repo tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        write_file(
            outside.path(),
            "tsconfig.base.json",
            r#"{"compilerOptions":{"baseUrl":"."}}"#,
        );
        let outside_config = outside.path().join("tsconfig.base.json");
        write_file(
            repo.path(),
            "tsconfig.json",
            &format!(
                r#"{{"extends":{}}}"#,
                serde_json::to_string(&outside_config).unwrap()
            ),
        );
        let mut db = AnalysisDb::new();
        add_file(&mut db, repo.path(), "src/app.ts", "export {};\n");

        let rows = module_graph_topology_input_digest_rows(repo.path(), &db);

        assert!(rows.iter().all(|(path, _)| path != "tsconfig.base.json"));
    }

    #[cfg(unix)]
    #[test]
    fn module_graph_layer_key_topology_inputs_do_not_hash_symlink_escape_contents() {
        let repo = tempfile::tempdir().expect("repo tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        write_file(outside.path(), "package.json", r#"{"name":"outside-one"}"#);
        std::os::unix::fs::symlink(
            outside.path().join("package.json"),
            repo.path().join("package.json"),
        )
        .expect("create symlink");
        let mut db = AnalysisDb::new();
        add_file(&mut db, repo.path(), "src/app.ts", "export {};\n");

        let first = module_graph_topology_input_digest_rows(repo.path(), &db);
        write_file(outside.path(), "package.json", r#"{"name":"outside-two"}"#);
        let second = module_graph_topology_input_digest_rows(repo.path(), &db);

        assert_eq!(first, second);
    }

    #[test]
    fn module_graph_layer_key_topology_inputs_follow_workspace_member_manifests() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "package.json",
            r#"{"name":"root","workspaces":["packages/*"]}"#,
        );
        let mut db = AnalysisDb::new();
        add_file(&mut db, temp.path(), "src/app.ts", "export {};\n");

        let base = module_graph_topology_input_digest_rows(temp.path(), &db);
        assert!(
            base.iter()
                .all(|(path, _)| path != "packages/ui/package.json")
        );

        write_file(
            temp.path(),
            "packages/ui/package.json",
            r#"{"name":"@acme/ui","version":"1.0.0"}"#,
        );
        let with_member = module_graph_topology_input_digest_rows(temp.path(), &db);
        assert!(
            with_member
                .iter()
                .any(|(path, _)| path == "packages/ui/package.json")
        );
        assert_ne!(base, with_member);

        write_file(
            temp.path(),
            "packages/ui/package.json",
            r#"{"name":"@acme/ui","version":"1.0.1"}"#,
        );
        let changed_member = module_graph_topology_input_digest_rows(temp.path(), &db);

        assert_ne!(with_member, changed_member);
    }

    #[test]
    fn module_graph_layer_key_workspace_globs_reject_absolute_and_escaping_patterns() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "packages/ui/package.json",
            r#"{"name":"@acme/ui"}"#,
        );

        assert_eq!(
            expand_package_workspace_glob(temp.path(), ".", "/tmp/*"),
            Vec::<String>::new()
        );
        assert_eq!(
            expand_package_workspace_glob(temp.path(), ".", "../outside/*"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn module_graph_layer_key_topology_inputs_follow_pnpm_workspace_member_manifests() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(temp.path(), "package.json", r#"{"name":"root"}"#);
        write_file(
            temp.path(),
            "pnpm-workspace.yaml",
            r#"packages: ["packages/*"]"#,
        );
        let mut db = AnalysisDb::new();
        add_file(&mut db, temp.path(), "src/app.ts", "export {};\n");

        let base = module_graph_topology_input_digest_rows(temp.path(), &db);
        assert!(
            base.iter()
                .all(|(path, _)| path != "packages/ui/package.json")
        );

        write_file(
            temp.path(),
            "packages/ui/package.json",
            r#"{"name":"@acme/ui","version":"1.0.0"}"#,
        );
        let with_member = module_graph_topology_input_digest_rows(temp.path(), &db);

        assert!(
            with_member
                .iter()
                .any(|(path, _)| path == "packages/ui/package.json")
        );
        assert_ne!(base, with_member);
    }

    #[test]
    fn module_graph_layer_key_topology_inputs_include_absent_extension_placeholder() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(temp.path(), "package.json", r#"{"name":"root"}"#);
        let mut db = AnalysisDb::new();
        add_file(&mut db, temp.path(), "src/app.ts", "export {};\n");
        let topology_inputs = module_graph_topology_input_digests(temp.path(), &db);

        let key = LayerKey::module_graph_layer_key(
            module_graph_manifest(),
            Vec::new(),
            topology_inputs,
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Vec::new(),
            Digest::from_parts(DigestKind::ProviderParameters, "module_graph", &["base"]),
        );

        assert!(key.extension_digests.contains(&Digest::absent(
            DigestKind::ExtensionCode,
            "extension_digest_absent"
        )));
    }

    fn write_file(root: &Path, relative_path: &str, source: &str) -> PathBuf {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().expect("test file has parent")).expect("mkdirs");
        fs::write(&path, source).expect("write file");
        path
    }

    fn add_file(
        db: &mut AnalysisDb,
        root: &Path,
        relative_path: &str,
        source: &str,
    ) -> crate::core::FileId {
        let path = write_file(root, relative_path, source);
        db.add_file(path, relative_path.to_string(), source.to_string())
    }

    #[test]
    fn module_graph_layer_key_changes_on_import_lifecycle_config_or_upstream_digest() {
        let base = module_graph_key(
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["react"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "go_syntax", &["base"]),
        );
        let changed_import = module_graph_key(
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["lodash"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "go_syntax", &["base"]),
        );
        let changed_go_lifecycle = module_graph_key(
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["react"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["changed"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "go_syntax", &["base"]),
        );
        let changed_ts_lifecycle = module_graph_key(
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["react"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["changed"]),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "go_syntax", &["base"]),
        );
        let changed_config = module_graph_key(
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["react"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::Config, "config", &["changed"]),
            Digest::from_parts(DigestKind::ProviderOutput, "go_syntax", &["base"]),
        );
        let changed_upstream = module_graph_key(
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["react"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "go_syntax", &["changed"]),
        );
        let mut changed_provider_version = base.clone();
        changed_provider_version.provider_version = "different-provider-version".to_string();
        let mut changed_schema = base.clone();
        changed_schema.schema_version = "module-graph-facts-3:3".to_string();

        for changed in [
            changed_import,
            changed_go_lifecycle,
            changed_ts_lifecycle,
            changed_config,
            changed_upstream,
            changed_provider_version,
            changed_schema,
        ] {
            assert_ne!(base, changed);
        }
    }

    #[test]
    fn module_graph_layer_key_tracks_the_resolver_toolchain() {
        let key = module_graph_key(
            digest("import_shape", "base"),
            digest("go_lifecycle", "base"),
            digest("ts_js_lifecycle", "base"),
            digest("config", "base"),
            digest("syntax", "base"),
        );

        assert_eq!(
            key.toolchain_digest,
            module_graph_toolchain_digest(TS_MODULE_RESOLVER)
        );
        assert_ne!(
            key.toolchain_digest,
            module_graph_toolchain_digest("oxc-resolver-0.0.0")
        );
    }

    #[test]
    fn module_topology_layer_key_changes_on_import_topology_module_symbol_and_semantic_inputs() {
        let base = module_topology_key(
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["react"]),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "base_topology",
                &["package:a"],
            ),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
            semantic_provider_parameter_digest(),
        );
        let changed_import = module_topology_key(
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["vue"]),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "base_topology",
                &["package:a"],
            ),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
            semantic_provider_parameter_digest(),
        );
        let changed_topology = module_topology_key(
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["react"]),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "base_topology",
                &["package:b"],
            ),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
            semantic_provider_parameter_digest(),
        );
        let changed_module = module_topology_key(
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["react"]),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "base_topology",
                &["package:a"],
            ),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["changed"]),
            Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
            semantic_provider_parameter_digest(),
        );
        let changed_symbol = module_topology_key(
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["react"]),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "base_topology",
                &["package:a"],
            ),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["changed"]),
            semantic_provider_parameter_digest(),
        );
        let changed_semantic = module_topology_key(
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["react"]),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "base_topology",
                &["package:a"],
            ),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
            Digest::from_parts(DigestKind::ProviderParameters, "semantic", &["changed"]),
        );

        assert_ne!(base, changed_import);
        assert_ne!(base, changed_topology);
        assert_ne!(base, changed_module);
        assert_ne!(base, changed_symbol);
        assert_ne!(base, changed_semantic);
    }

    #[test]
    fn module_graph_layer_key_ignores_rule_digest_changes() {
        let base = module_graph_key(
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["react"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "go_syntax", &["base"]),
        );
        let rule_a = Digest::from_parts(DigestKind::RuleCode, "rule", &["a"]);
        let rule_b = Digest::from_parts(DigestKind::RuleOptions, "rule", &["b"]);

        assert_ne!(rule_a, rule_b);
        assert_eq!(
            base,
            module_graph_key(
                Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["react"]),
                Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
                Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
                Digest::from_parts(DigestKind::Config, "config", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "go_syntax", &["base"]),
            )
        );
    }

    #[test]
    fn symbol_graph_layer_key_changes_on_source_import_lifecycle_config_or_upstream_digest() {
        let base = symbol_graph_key(
            Digest::from_parts(
                DigestKind::SourceText,
                "source_function",
                &["src/app.ts", "base"],
            ),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "import_shape",
                &["./target"],
            ),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
        );
        let changed_source = symbol_graph_key(
            Digest::from_parts(
                DigestKind::SourceText,
                "source_function",
                &["src/app.ts", "changed"],
            ),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "import_shape",
                &["./target"],
            ),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
        );
        let changed_import = symbol_graph_key(
            Digest::from_parts(
                DigestKind::SourceText,
                "source_function",
                &["src/app.ts", "base"],
            ),
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &["./other"]),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
        );
        let changed_config = symbol_graph_key(
            Digest::from_parts(
                DigestKind::SourceText,
                "source_function",
                &["src/app.ts", "base"],
            ),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "import_shape",
                &["./target"],
            ),
            Digest::from_parts(DigestKind::Config, "config", &["changed"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
        );
        let changed_go_lifecycle = symbol_graph_key(
            Digest::from_parts(
                DigestKind::SourceText,
                "source_function",
                &["src/app.ts", "base"],
            ),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "import_shape",
                &["./target"],
            ),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["changed"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
        );
        let changed_module_output = symbol_graph_key(
            Digest::from_parts(
                DigestKind::SourceText,
                "source_function",
                &["src/app.ts", "base"],
            ),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "import_shape",
                &["./target"],
            ),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["changed"]),
            Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
        );
        let changed_syntax_output = symbol_graph_key(
            Digest::from_parts(
                DigestKind::SourceText,
                "source_function",
                &["src/app.ts", "base"],
            ),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "import_shape",
                &["./target"],
            ),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["changed"]),
        );
        let mut changed_provider_version = base.clone();
        changed_provider_version.provider_version = "different-provider-version".to_string();
        let mut changed_schema = base.clone();
        changed_schema.schema_version = "symbol-graph-facts-2:changed".to_string();

        for changed in [
            changed_source,
            changed_import,
            changed_config,
            changed_go_lifecycle,
            changed_module_output,
            changed_syntax_output,
            changed_provider_version,
            changed_schema,
        ] {
            assert_ne!(base, changed);
        }
        assert_eq!(base.layer_kind, LayerKind::SymbolGraph);
        assert!(base.extension_digests.contains(&Digest::absent(
            DigestKind::ExtensionCode,
            "extension_digest_absent"
        )));
    }

    mod symbol_graph_semantic_layer_key {
        use super::*;

        #[test]
        fn includes_semantic_provider_parameters() {
            assert_eq!(
                semantic_provider_parameter_digest(),
                Digest::from_parts(
                    DigestKind::ProviderParameters,
                    "semantic_provider_parameters",
                    &[
                        "scopes=enabled",
                        "semantic_imports=enabled",
                        "exports=enabled",
                        "aliases=enabled",
                        "resolution_facts=enabled",
                        "generated_symbols=enabled",
                        "stable_exports=enabled",
                        "alias_closure=max_input_plus_one",
                        "generated_hooks=native_rows_only",
                    ],
                )
            );
        }

        #[test]
        fn key_changes_when_semantic_parameters_change() {
            let base = symbol_graph_key(
                Digest::from_parts(
                    DigestKind::SourceText,
                    "source_function",
                    &["src/app.ts", "base"],
                ),
                Digest::from_parts(
                    DigestKind::ProviderParameters,
                    "import_shape",
                    &["./target"],
                ),
                Digest::from_parts(DigestKind::Config, "config", &["base"]),
                Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
                Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
            );
            let changed = LayerKey::symbol_graph_layer_key(
                symbol_graph_manifest(),
                vec![Digest::from_parts(
                    DigestKind::SourceText,
                    "source_function",
                    &["src/app.ts", "base"],
                )],
                vec![Digest::from_parts(
                    DigestKind::ProviderParameters,
                    "package_context",
                    &["src/app.ts", "pkg"],
                )],
                vec![Digest::from_parts(
                    DigestKind::ProviderParameters,
                    "import_shape",
                    &["./target"],
                )],
                Digest::from_parts(DigestKind::Config, "config", &["base"]),
                Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
                Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
                vec![Digest::from_parts(
                    DigestKind::ProviderOutput,
                    "ts_syntax",
                    &["base"],
                )],
                Digest::from_parts(
                    DigestKind::ProviderParameters,
                    "symbol_graph_parameters",
                    &[
                        "output=symbols",
                        "output=definitions",
                        "output=references",
                        "scopes=disabled",
                    ],
                ),
            );

            assert_ne!(base.parameter_digest, changed.parameter_digest);
            assert_ne!(base, changed);
        }

        #[test]
        fn key_tracks_provider_schema_and_absent_extension_digest() {
            let base = symbol_graph_key(
                Digest::from_parts(
                    DigestKind::SourceText,
                    "source_function",
                    &["src/app.ts", "base"],
                ),
                Digest::from_parts(
                    DigestKind::ProviderParameters,
                    "import_shape",
                    &["./target"],
                ),
                Digest::from_parts(DigestKind::Config, "config", &["base"]),
                Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
                Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
            );
            let mut changed_schema = base.clone();
            changed_schema.schema_version = "symbol-graph-facts-3:3".to_string();

            assert_ne!(base, changed_schema);
            assert!(base.extension_digests.contains(&Digest::absent(
                DigestKind::ExtensionCode,
                "extension_digest_absent"
            )));
            assert!(
                base.dependency_layer_digests
                    .contains(&dependency_layer_digest(Digest::from_parts(
                        DigestKind::ProviderOutput,
                        "module_graph",
                        &["base"]
                    )))
            );
            assert!(
                base.dependency_layer_digests
                    .contains(&dependency_layer_digest(Digest::from_parts(
                        DigestKind::ProviderOutput,
                        "ts_syntax",
                        &["base"]
                    )))
            );
        }
    }

    mod semantic_mir_layer_key {
        use super::*;

        fn semantic_mir_manifest() -> &'static crate::analysis_kernel::ProviderManifest {
            crate::analysis_kernel::AnalysisKernel::provider_manifests()
                .iter()
                .find(|manifest| manifest.id == "polint.semantic_mir")
                .expect("semantic MIR provider manifest exists")
        }

        #[expect(
            clippy::too_many_arguments,
            reason = "test helper mirrors the semantic MIR layer key inputs"
        )]
        fn semantic_mir_key(
            source_function_digest: Digest,
            config_digest: Digest,
            go_lifecycle_digest: Digest,
            ts_js_lifecycle_digest: Digest,
            syntax_output_digest: Digest,
            symbol_graph_output_digest: Digest,
            module_topology_output_digest: Digest,
            parameter_digest: Digest,
        ) -> LayerKey {
            LayerKey::semantic_mir_layer_key(
                semantic_mir_manifest(),
                vec![source_function_digest],
                config_digest,
                go_lifecycle_digest,
                ts_js_lifecycle_digest,
                vec![syntax_output_digest],
                symbol_graph_output_digest,
                module_topology_output_digest,
                parameter_digest,
            )
        }

        #[test]
        fn key_changes_on_source_lifecycle_config_parameters_and_upstream_outputs() {
            let base = semantic_mir_key(
                Digest::from_parts(
                    DigestKind::SourceText,
                    "source_function",
                    &["src/app.ts", "base"],
                ),
                Digest::from_parts(DigestKind::Config, "config", &["base"]),
                Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
                Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "module_topology", &["base"]),
                crate::analysis_neutral::cache_key::semantic_mir_provider_parameter_digest(),
            );

            let changed_source = semantic_mir_key(
                Digest::from_parts(
                    DigestKind::SourceText,
                    "source_function",
                    &["src/app.ts", "changed"],
                ),
                Digest::from_parts(DigestKind::Config, "config", &["base"]),
                Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
                Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "module_topology", &["base"]),
                crate::analysis_neutral::cache_key::semantic_mir_provider_parameter_digest(),
            );
            let changed_config = semantic_mir_key(
                Digest::from_parts(
                    DigestKind::SourceText,
                    "source_function",
                    &["src/app.ts", "base"],
                ),
                Digest::from_parts(DigestKind::Config, "config", &["changed"]),
                Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
                Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "module_topology", &["base"]),
                crate::analysis_neutral::cache_key::semantic_mir_provider_parameter_digest(),
            );
            let changed_go_lifecycle = semantic_mir_key(
                Digest::from_parts(
                    DigestKind::SourceText,
                    "source_function",
                    &["src/app.ts", "base"],
                ),
                Digest::from_parts(DigestKind::Config, "config", &["base"]),
                Digest::from_parts(DigestKind::GoLifecycle, "go", &["changed"]),
                Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "module_topology", &["base"]),
                crate::analysis_neutral::cache_key::semantic_mir_provider_parameter_digest(),
            );
            let changed_ts_lifecycle = semantic_mir_key(
                Digest::from_parts(
                    DigestKind::SourceText,
                    "source_function",
                    &["src/app.ts", "base"],
                ),
                Digest::from_parts(DigestKind::Config, "config", &["base"]),
                Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
                Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["changed"]),
                Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "module_topology", &["base"]),
                crate::analysis_neutral::cache_key::semantic_mir_provider_parameter_digest(),
            );
            let changed_syntax = semantic_mir_key(
                Digest::from_parts(
                    DigestKind::SourceText,
                    "source_function",
                    &["src/app.ts", "base"],
                ),
                Digest::from_parts(DigestKind::Config, "config", &["base"]),
                Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
                Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["changed"]),
                Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "module_topology", &["base"]),
                crate::analysis_neutral::cache_key::semantic_mir_provider_parameter_digest(),
            );
            let changed_symbol = semantic_mir_key(
                Digest::from_parts(
                    DigestKind::SourceText,
                    "source_function",
                    &["src/app.ts", "base"],
                ),
                Digest::from_parts(DigestKind::Config, "config", &["base"]),
                Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
                Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["changed"]),
                Digest::from_parts(DigestKind::ProviderOutput, "module_topology", &["base"]),
                crate::analysis_neutral::cache_key::semantic_mir_provider_parameter_digest(),
            );
            let changed_topology = semantic_mir_key(
                Digest::from_parts(
                    DigestKind::SourceText,
                    "source_function",
                    &["src/app.ts", "base"],
                ),
                Digest::from_parts(DigestKind::Config, "config", &["base"]),
                Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
                Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "module_topology", &["changed"]),
                crate::analysis_neutral::cache_key::semantic_mir_provider_parameter_digest(),
            );
            let changed_parameters = semantic_mir_key(
                Digest::from_parts(
                    DigestKind::SourceText,
                    "source_function",
                    &["src/app.ts", "base"],
                ),
                Digest::from_parts(DigestKind::Config, "config", &["base"]),
                Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
                Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "module_topology", &["base"]),
                Digest::from_parts(
                    DigestKind::ProviderParameters,
                    "semantic_mir_provider_parameters",
                    &["changed"],
                ),
            );

            for changed in [
                changed_source,
                changed_config,
                changed_go_lifecycle,
                changed_ts_lifecycle,
                changed_syntax,
                changed_symbol,
                changed_topology,
                changed_parameters,
            ] {
                assert_ne!(base, changed);
            }
        }

        #[test]
        fn key_includes_absent_extension_model_and_toolchain_slots_and_excludes_rule_code() {
            let base = semantic_mir_key(
                Digest::from_parts(DigestKind::SourceText, "source_function", &["base"]),
                Digest::from_parts(DigestKind::Config, "config", &["base"]),
                Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
                Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "go_syntax", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
                Digest::from_parts(DigestKind::ProviderOutput, "module_topology", &["base"]),
                crate::analysis_neutral::cache_key::semantic_mir_provider_parameter_digest(),
            );
            let rule_code = Digest::from_parts(DigestKind::RuleCode, "rule", &["changed"]);

            assert_eq!(base.layer_kind, LayerKind::SemanticMir);
            assert!(
                base.toolchain_digest
                    .to_string()
                    .contains("tool_invocation")
            );
            assert!(base.extension_digests.contains(&Digest::absent(
                DigestKind::ExtensionCode,
                "extension_digest_absent"
            )));
            assert!(base.extension_digests.contains(&Digest::absent(
                DigestKind::ModelFile,
                "model_digest_absent"
            )));
            assert!(!base.input_digests.contains(&rule_code));
            assert!(!base.dependency_layer_digests.contains(&rule_code));
            assert!(!base.extension_digests.contains(&rule_code));
        }
    }

    #[test]
    fn metrics_layer_key_changes_only_on_consumed_source_and_function_inputs() {
        let base = metrics_key(
            Digest::from_parts(DigestKind::SourceText, "source", &["src/app.ts", "base"]),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "function_fact",
                &["handler", "base"],
            ),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
        );
        let changed_source = metrics_key(
            Digest::from_parts(DigestKind::SourceText, "source", &["src/app.ts", "changed"]),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "function_fact",
                &["handler", "base"],
            ),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
        );
        let changed_function = metrics_key(
            Digest::from_parts(DigestKind::SourceText, "source", &["src/app.ts", "base"]),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "function_fact",
                &["handler", "changed"],
            ),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
        );
        let changed_config = metrics_key(
            Digest::from_parts(DigestKind::SourceText, "source", &["src/app.ts", "base"]),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "function_fact",
                &["handler", "base"],
            ),
            Digest::from_parts(DigestKind::Config, "config", &["changed"]),
            Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]),
        );
        let changed_syntax = metrics_key(
            Digest::from_parts(DigestKind::SourceText, "source", &["src/app.ts", "base"]),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "function_fact",
                &["handler", "base"],
            ),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["changed"]),
        );
        let mut changed_provider_version = base.clone();
        changed_provider_version.provider_version = "different-provider-version".to_string();
        let mut changed_schema = base.clone();
        changed_schema.schema_version = "metrics-facts-2:2".to_string();

        for changed in [
            changed_source,
            changed_function,
            changed_provider_version,
            changed_schema,
        ] {
            assert_ne!(base, changed);
        }
        assert_eq!(base, changed_config);
        assert_eq!(base, changed_syntax);
        assert_eq!(base.layer_kind, LayerKind::Metrics);
        assert_eq!(
            base.config_digest,
            Digest::absent(DigestKind::Config, "metrics_config")
        );
        assert_eq!(
            base.toolchain_digest,
            Digest::absent(DigestKind::ToolInvocation, "metrics_toolchain")
        );
        assert!(base.dependency_layer_digests.is_empty());
        assert!(base.extension_digests.contains(&Digest::absent(
            DigestKind::ExtensionCode,
            "extension_digest_absent"
        )));
        assert!(base.extension_digests.contains(&Digest::absent(
            DigestKind::ModelFile,
            "model_digest_absent"
        )));
    }

    #[test]
    fn layer_key_constructor_sorts_variable_digest_lists() {
        let a = digest("file", "a");
        let b = digest("file", "b");

        let left = LayerKey::new(
            LayerKind::TsSyntax,
            "polint.ts.syntax",
            "1",
            "ts-facts-v1",
            Digest::absent(DigestKind::ProviderParameters, "none"),
            Digest::absent(DigestKind::TsJsLifecycle, "none"),
            Digest::absent(DigestKind::Config, "none"),
            Digest::absent(DigestKind::ToolInvocation, "none"),
            vec![b.clone(), a.clone()],
            vec![b.clone(), a.clone()],
            vec![b.clone(), a.clone()],
        );
        let right = LayerKey::new(
            LayerKind::TsSyntax,
            "polint.ts.syntax",
            "1",
            "ts-facts-v1",
            Digest::absent(DigestKind::ProviderParameters, "none"),
            Digest::absent(DigestKind::TsJsLifecycle, "none"),
            Digest::absent(DigestKind::Config, "none"),
            Digest::absent(DigestKind::ToolInvocation, "none"),
            vec![a.clone(), b.clone()],
            vec![a.clone(), b.clone()],
            vec![a, b],
        );

        assert_eq!(left, right);
    }

    #[test]
    fn syntax_layer_key_ignores_rule_digest_changes() {
        let source = digest("file", "src/main.go");
        let rule_a = Digest::from_parts(DigestKind::RuleCode, "rule", &["a"]);
        let rule_b = Digest::from_parts(DigestKind::RuleCode, "rule", &["b"]);

        assert_ne!(rule_a, rule_b);
        assert_eq!(syntax_key(vec![source.clone()]), syntax_key(vec![source]));
    }

    #[test]
    fn syntax_layer_key_changes_when_parser_inputs_change() {
        let source = digest("file", "src/main.go");
        let base = syntax_key(vec![source.clone()]);

        let changed_source = syntax_key(vec![digest("file", "src/other.go")]);
        let changed_config = LayerKey::syntax_layer_key(
            LayerKind::GoSyntax,
            "polint.go.syntax",
            "1",
            "go-facts-v2",
            vec![source.clone()],
            Digest::from_parts(DigestKind::Config, "config", &["changed"]),
            Digest::from_parts(DigestKind::GoLifecycle, "lifecycle", &["base"]),
            Digest::from_parts(DigestKind::ToolInvocation, "toolchain", &["base"]),
            Digest::from_parts(DigestKind::ProviderParameters, "parser", &["base"]),
        );
        let changed_lifecycle = LayerKey::syntax_layer_key(
            LayerKind::GoSyntax,
            "polint.go.syntax",
            "1",
            "go-facts-v2",
            vec![source.clone()],
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "lifecycle", &["changed"]),
            Digest::from_parts(DigestKind::ToolInvocation, "toolchain", &["base"]),
            Digest::from_parts(DigestKind::ProviderParameters, "parser", &["base"]),
        );
        let changed_toolchain = LayerKey::syntax_layer_key(
            LayerKind::GoSyntax,
            "polint.go.syntax",
            "1",
            "go-facts-v2",
            vec![source.clone()],
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "lifecycle", &["base"]),
            Digest::from_parts(DigestKind::ToolInvocation, "toolchain", &["changed"]),
            Digest::from_parts(DigestKind::ProviderParameters, "parser", &["base"]),
        );
        let changed_parser_params = LayerKey::syntax_layer_key(
            LayerKind::GoSyntax,
            "polint.go.syntax",
            "1",
            "go-facts-v2",
            vec![source],
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "lifecycle", &["base"]),
            Digest::from_parts(DigestKind::ToolInvocation, "toolchain", &["base"]),
            Digest::from_parts(DigestKind::ProviderParameters, "parser", &["changed"]),
        );
        let changed_provider_version = LayerKey::syntax_layer_key(
            LayerKind::GoSyntax,
            "polint.go.syntax",
            "2",
            "go-facts-v2",
            base.input_digests.as_ref().clone(),
            base.config_digest.clone(),
            base.lifecycle_digest.clone(),
            base.toolchain_digest.clone(),
            base.parameter_digest.clone(),
        );
        let changed_schema_version = LayerKey::syntax_layer_key(
            LayerKind::GoSyntax,
            "polint.go.syntax",
            "1",
            "go-facts-v3",
            base.input_digests.as_ref().clone(),
            base.config_digest.clone(),
            base.lifecycle_digest.clone(),
            base.toolchain_digest.clone(),
            base.parameter_digest.clone(),
        );

        for changed in [
            changed_source,
            changed_config,
            changed_lifecycle,
            changed_toolchain,
            changed_parser_params,
            changed_provider_version,
            changed_schema_version,
        ] {
            assert_ne!(base, changed);
        }
    }

    #[test]
    fn syntax_layer_key_sorts_source_digest_inputs() {
        let a = digest("file", "a");
        let b = digest("file", "b");

        let left = LayerKey::syntax_layer_key(
            LayerKind::TsSyntax,
            "polint.ts.syntax",
            "1",
            "ts-facts-v1",
            vec![b.clone(), a.clone()],
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "lifecycle", &["base"]),
            Digest::from_parts(DigestKind::ToolInvocation, "toolchain", &["base"]),
            Digest::from_parts(DigestKind::ProviderParameters, "parser", &["base"]),
        );
        let right = LayerKey::syntax_layer_key(
            LayerKind::TsSyntax,
            "polint.ts.syntax",
            "1",
            "ts-facts-v1",
            vec![a, b],
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "lifecycle", &["base"]),
            Digest::from_parts(DigestKind::ToolInvocation, "toolchain", &["base"]),
            Digest::from_parts(DigestKind::ProviderParameters, "parser", &["base"]),
        );

        assert_eq!(left, right);
        assert!(left.extension_digests.contains(&Digest::absent(
            DigestKind::ExtensionCode,
            "extension_digest_absent"
        )));
    }

    #[test]
    fn existing_file_cache_bridge_includes_all_cache_key_fields_as_digests() {
        let cache_key = CacheKey {
            file_hash: "file-hash".to_string(),
            config_hash: "config-hash".to_string(),
            rule_hash: "rule-hash".to_string(),
            plan_hash: "plan-hash".to_string(),
            version: CACHE_VERSION.to_string(),
            schema: "ts-facts-v1".to_string(),
            parser_identity: "oxc-0.0.0".to_string(),
        };
        let key = LayerKey::from_existing_file_cache(
            LayerKind::TsSyntax,
            "polint.ts.syntax",
            "1",
            &cache_key,
        );

        assert_eq!(
            key.config_digest,
            Digest::from_parts(DigestKind::Config, "config_hash", &["config-hash"])
        );
        assert!(key.input_digests.contains(&Digest::from_parts(
            DigestKind::SourceText,
            "file_hash",
            &["file-hash"]
        )));
        assert!(key.input_digests.contains(&Digest::from_parts(
            DigestKind::RuleCode,
            "rule_hash",
            &["rule-hash"]
        )));
        assert!(key.input_digests.contains(&Digest::from_parts(
            DigestKind::ProviderParameters,
            "plan_hash",
            &["plan-hash"]
        )));
        assert!(key.input_digests.contains(&Digest::from_parts(
            DigestKind::ToolInvocation,
            "version",
            &[CACHE_VERSION]
        )));
        assert!(key.input_digests.contains(&Digest::from_parts(
            DigestKind::ToolInvocation,
            "parser_identity",
            &["oxc-0.0.0"]
        )));
        assert!(key.input_digests.contains(&Digest::from_parts(
            DigestKind::ProviderOutput,
            "schema",
            &["ts-facts-v1"]
        )));
    }

    #[test]
    fn query_summary_and_diagnostic_keys_serialize_snake_case_with_sorted_digest_lists() {
        let a = digest("digest", "a");
        let b = digest("digest", "b");
        let query = QueryKey::new(
            "call_graph",
            "1",
            Digest::absent(DigestKind::QueryParameters, "none"),
            vec![b.clone(), a.clone()],
            Digest::absent(DigestKind::Budget, "none"),
            PrecisionTier::Syntax,
        );
        let summary = SummaryKey::new(
            "function:src/main.rs:main",
            "return_effects",
            "1",
            Digest::absent(DigestKind::SummaryBody, "none"),
            vec![b.clone(), a.clone()],
            Digest::absent(DigestKind::ExtensionCode, "none"),
        );
        let diagnostic = DiagnosticKey::new(
            "local/example",
            "1",
            Digest::absent(DigestKind::RuleCode, "none"),
            Digest::absent(DigestKind::RuleOptions, "none"),
            vec![b, a],
            Digest::absent(DigestKind::Evidence, "none"),
        );

        let query_json = serde_json::to_value(query).expect("query key should serialize");
        let summary_json = serde_json::to_value(summary).expect("summary key should serialize");
        let diagnostic_json =
            serde_json::to_value(diagnostic).expect("diagnostic key should serialize");

        assert!(query_json.get("query_kind").is_some());
        assert!(summary_json.get("callable_stable_key").is_some());
        assert!(diagnostic_json.get("requested_view_digests").is_some());
        assert!(
            digest_value(&query_json["layer_digests"][0])
                < digest_value(&query_json["layer_digests"][1])
        );
        assert!(
            digest_value(&summary_json["dependency_summary_digests"][0])
                < digest_value(&summary_json["dependency_summary_digests"][1])
        );
        assert!(
            digest_value(&diagnostic_json["requested_view_digests"][0])
                < digest_value(&diagnostic_json["requested_view_digests"][1])
        );
    }

    fn digest_value(value: &serde_json::Value) -> &str {
        value["value"]
            .as_str()
            .expect("serialized digest should have a string value")
    }
}

#[cfg(test)]
mod symbol_graph_semantic_layer_key {
    use super::*;

    fn manifest() -> &'static crate::analysis_kernel::ProviderManifest {
        crate::analysis_kernel::AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.symbol_graph")
            .expect("symbol graph provider manifest exists")
    }

    fn key(parameter_digest: Digest) -> LayerKey {
        LayerKey::symbol_graph_layer_key(
            manifest(),
            vec![Digest::from_parts(
                DigestKind::SourceText,
                "source_function",
                &["src/app.ts", "base"],
            )],
            vec![Digest::from_parts(
                DigestKind::ProviderParameters,
                "package_context",
                &["src/app.ts", "pkg"],
            )],
            vec![Digest::from_parts(
                DigestKind::ProviderParameters,
                "import_shape",
                &["./target"],
            )],
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            vec![Digest::from_parts(
                DigestKind::ProviderOutput,
                "ts_syntax",
                &["base"],
            )],
            parameter_digest,
        )
    }

    #[test]
    fn includes_semantic_provider_parameters() {
        assert_eq!(
            semantic_provider_parameter_digest(),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "semantic_provider_parameters",
                &[
                    "scopes=enabled",
                    "semantic_imports=enabled",
                    "exports=enabled",
                    "aliases=enabled",
                    "resolution_facts=enabled",
                    "generated_symbols=enabled",
                    "stable_exports=enabled",
                    "alias_closure=max_input_plus_one",
                    "generated_hooks=native_rows_only",
                ],
            )
        );
    }

    #[test]
    fn changes_when_semantic_provider_parameters_change() {
        let enabled = key(semantic_provider_parameter_digest());
        let disabled = key(Digest::from_parts(
            DigestKind::ProviderParameters,
            "semantic_provider_parameters",
            &["scopes=disabled"],
        ));

        assert_ne!(enabled.parameter_digest, disabled.parameter_digest);
        assert_ne!(enabled, disabled);
    }

    #[test]
    fn tracks_schema_upstream_outputs_and_absent_extension_digest() {
        let base = key(semantic_provider_parameter_digest());
        let mut changed_schema = base.clone();
        changed_schema.schema_version = "symbol-graph-facts-3:3".to_string();

        assert_ne!(base, changed_schema);
        assert!(base.extension_digests.contains(&Digest::absent(
            DigestKind::ExtensionCode,
            "extension_digest_absent"
        )));
        assert!(
            base.dependency_layer_digests
                .contains(&dependency_layer_digest(Digest::from_parts(
                    DigestKind::ProviderOutput,
                    "module_graph",
                    &["base"]
                )))
        );
        assert!(
            base.dependency_layer_digests
                .contains(&dependency_layer_digest(Digest::from_parts(
                    DigestKind::ProviderOutput,
                    "ts_syntax",
                    &["base"]
                )))
        );
    }
}

#[cfg(test)]
mod cfg_layer_key {
    use super::*;
    use crate::analysis_neutral::cfg::cache_key::cfg_provider_parameter_digest;

    fn manifest() -> &'static crate::analysis_kernel::ProviderManifest {
        crate::analysis_kernel::AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.cfg")
            .expect("CFG provider manifest exists")
    }

    #[allow(clippy::too_many_arguments)]
    fn key(
        source_suffix: &str,
        config_suffix: &str,
        go_suffix: &str,
        ts_suffix: &str,
        syntax_suffix: &str,
        semantic_suffix: &str,
        parameter_digest: Digest,
    ) -> LayerKey {
        LayerKey::cfg_layer_key(
            manifest(),
            vec![Digest::from_parts(
                DigestKind::SourceText,
                "source_function",
                &["src/app.ts", source_suffix],
            )],
            Digest::from_parts(DigestKind::Config, "config", &[config_suffix]),
            Digest::from_parts(DigestKind::GoLifecycle, "go_lifecycle", &[go_suffix]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts_js_lifecycle", &[ts_suffix]),
            vec![Digest::from_parts(
                DigestKind::ProviderOutput,
                "ts_syntax",
                &[syntax_suffix],
            )],
            Digest::from_parts(
                DigestKind::ProviderOutput,
                "semantic_mir",
                &[semantic_suffix],
            ),
            parameter_digest,
        )
    }

    #[test]
    fn cfg_provider_parameters_include_views_outputs_and_schema() {
        assert_eq!(
            cfg_provider_parameter_digest(),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "cfg_provider_parameters",
                &[
                    "cfg-facts-1",
                    "cfg_functions",
                    "cfg_nodes",
                    "basic_blocks",
                    "cfg_edges",
                    "cfg_reachability",
                    "cfg_dominators",
                    "cfg_postdominators",
                    "cfg_control_dependence",
                    "unsupported_control_flow",
                    "normal_control_view",
                    "abrupt_aware_view",
                ],
            )
        );
    }

    #[test]
    fn cfg_layer_key_changes_for_every_cfg_input_family() {
        let base = key(
            "base",
            "base",
            "base",
            "base",
            "base",
            "base",
            cfg_provider_parameter_digest(),
        );

        assert_ne!(
            base,
            key(
                "changed",
                "base",
                "base",
                "base",
                "base",
                "base",
                cfg_provider_parameter_digest()
            )
        );
        assert_ne!(
            base,
            key(
                "base",
                "changed",
                "base",
                "base",
                "base",
                "base",
                cfg_provider_parameter_digest()
            )
        );
        assert_ne!(
            base,
            key(
                "base",
                "base",
                "changed",
                "base",
                "base",
                "base",
                cfg_provider_parameter_digest()
            )
        );
        assert_ne!(
            base,
            key(
                "base",
                "base",
                "base",
                "changed",
                "base",
                "base",
                cfg_provider_parameter_digest()
            )
        );
        assert_ne!(
            base,
            key(
                "base",
                "base",
                "base",
                "base",
                "changed",
                "base",
                cfg_provider_parameter_digest()
            )
        );
        assert_ne!(
            base,
            key(
                "base",
                "base",
                "base",
                "base",
                "base",
                "changed",
                cfg_provider_parameter_digest()
            )
        );
        assert_ne!(
            base,
            key(
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                Digest::from_parts(
                    DigestKind::ProviderParameters,
                    "cfg_provider_parameters",
                    &["normal_control_view"]
                )
            )
        );
    }

    #[test]
    fn cfg_layer_key_tracks_absent_extension_model_toolchain_and_graph_views() {
        let base = key(
            "base",
            "base",
            "base",
            "base",
            "base",
            "base",
            cfg_provider_parameter_digest(),
        );

        assert_eq!(base.layer_kind, LayerKind::Cfg);
        assert!(base.extension_digests.contains(&Digest::absent(
            DigestKind::ExtensionCode,
            "extension_digest_absent"
        )));
        assert!(base.extension_digests.contains(&Digest::absent(
            DigestKind::ModelFile,
            "model_digest_absent"
        )));
        assert_eq!(
            base.toolchain_digest,
            Digest::absent(DigestKind::ToolInvocation, "cfg_toolchain")
        );
        assert!(base.input_digests.contains(&Digest::from_parts(
            DigestKind::ProviderParameters,
            "cfg_graph_views",
            &["normal_control_view", "abrupt_aware_view"]
        )));
    }
}

#[cfg(test)]
mod calls_layer_key {
    use super::*;
    use crate::analysis_neutral::calls::cache_key::calls_provider_parameter_digest;

    fn manifest() -> &'static crate::analysis_kernel::ProviderManifest {
        crate::analysis_kernel::AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.calls")
            .expect("calls provider manifest exists")
    }

    #[allow(clippy::too_many_arguments)]
    fn key(
        source_suffix: &str,
        config_suffix: &str,
        go_suffix: &str,
        ts_suffix: &str,
        syntax_suffix: &str,
        semantic_suffix: &str,
        cfg_suffix: &str,
        symbol_suffix: &str,
        topology_suffix: &str,
        parameter_digest: Digest,
    ) -> LayerKey {
        LayerKey::calls_layer_key(
            manifest(),
            vec![Digest::from_parts(
                DigestKind::SourceText,
                "source_function",
                &["src/app.ts", source_suffix],
            )],
            Digest::from_parts(DigestKind::Config, "config", &[config_suffix]),
            Digest::from_parts(DigestKind::GoLifecycle, "go_lifecycle", &[go_suffix]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts_js_lifecycle", &[ts_suffix]),
            vec![Digest::from_parts(
                DigestKind::ProviderOutput,
                "ts_syntax",
                &[syntax_suffix],
            )],
            Digest::from_parts(
                DigestKind::ProviderOutput,
                "semantic_mir",
                &[semantic_suffix],
            ),
            Digest::from_parts(DigestKind::ProviderOutput, "cfg", &[cfg_suffix]),
            Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &[symbol_suffix]),
            Digest::from_parts(
                DigestKind::ProviderOutput,
                "module_topology",
                &[topology_suffix],
            ),
            parameter_digest,
        )
    }

    #[test]
    fn calls_layer_key_changes_for_every_calls_input_family() {
        let base = key(
            "base",
            "base",
            "base",
            "base",
            "base",
            "base",
            "base",
            "base",
            "base",
            calls_provider_parameter_digest(),
        );

        let cases = [
            key(
                "changed",
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                calls_provider_parameter_digest(),
            ),
            key(
                "base",
                "changed",
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                calls_provider_parameter_digest(),
            ),
            key(
                "base",
                "base",
                "changed",
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                calls_provider_parameter_digest(),
            ),
            key(
                "base",
                "base",
                "base",
                "changed",
                "base",
                "base",
                "base",
                "base",
                "base",
                calls_provider_parameter_digest(),
            ),
            key(
                "base",
                "base",
                "base",
                "base",
                "changed",
                "base",
                "base",
                "base",
                "base",
                calls_provider_parameter_digest(),
            ),
            key(
                "base",
                "base",
                "base",
                "base",
                "base",
                "changed",
                "base",
                "base",
                "base",
                calls_provider_parameter_digest(),
            ),
            key(
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                "changed",
                "base",
                "base",
                calls_provider_parameter_digest(),
            ),
            key(
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                "changed",
                "base",
                calls_provider_parameter_digest(),
            ),
            key(
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                "changed",
                calls_provider_parameter_digest(),
            ),
            key(
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                "base",
                Digest::from_parts(
                    DigestKind::ProviderParameters,
                    "calls_provider_parameters",
                    &["direct_binding=disabled"],
                ),
            ),
        ];

        for changed in cases {
            assert_ne!(base, changed);
        }
    }

    #[test]
    fn calls_layer_key_tracks_absent_extension_model_toolchain() {
        let base = key(
            "base",
            "base",
            "base",
            "base",
            "base",
            "base",
            "base",
            "base",
            "base",
            calls_provider_parameter_digest(),
        );

        assert_eq!(base.layer_kind, LayerKind::Calls);
        assert!(base.extension_digests.contains(&Digest::absent(
            DigestKind::ExtensionCode,
            "extension_digest_absent"
        )));
        assert!(base.extension_digests.contains(&Digest::absent(
            DigestKind::ModelFile,
            "model_digest_absent"
        )));
        assert_eq!(
            base.toolchain_digest,
            Digest::absent(DigestKind::ToolInvocation, "calls_toolchain")
        );
        assert_eq!(
            base.lifecycle_digest,
            Digest::from_unordered(
                DigestKind::ProviderParameters,
                "calls_lifecycle_inputs",
                vec![
                    Digest::from_parts(DigestKind::GoLifecycle, "go_lifecycle", &["base"]),
                    Digest::from_parts(DigestKind::TsJsLifecycle, "ts_js_lifecycle", &["base"]),
                ],
            )
        );
    }
}
