use serde::Deserialize;

use super::{Digest, DigestKind};
use crate::analysis::extensions::discovery::{DiscoveredExtension, discover_local_extensions};
use crate::analysis_kernel::ProviderManifest;
use crate::config::LoadedConfig;
use crate::core::AnalysisDb;
use crate::repo_fs::{
    TOPOLOGY_LOCKFILE_MAX_BYTES, TOPOLOGY_MANIFEST_MAX_BYTES, normalize_repo_relative,
    normalize_repo_relative_input, read_repo_file_to_string_with_limit, read_repo_file_with_limit,
    repo_file_exists, repo_file_path, repo_relative_existing_path,
};
use std::collections::BTreeSet;
use std::path::{Path as FsPath, PathBuf};

pub(crate) use crate::analysis_api::{
    FileSnapshot, GoLifecycleSnapshot, INPUT_SNAPSHOT_SCHEMA_VERSION, InputComponent,
    InputComponentStatus, InputSnapshot, ProviderSchemaSnapshot, TsJsLifecycleSnapshot,
};

pub(crate) fn input_snapshot_from_run_inputs(
    loaded: &LoadedConfig,
    db: &AnalysisDb,
    config_digest: &str,
    rule_digest: &str,
    plan_digest: &str,
    provider_manifests: &[ProviderManifest],
) -> InputSnapshot {
    let mut files = db
        .files()
        .iter()
        .map(FileSnapshot::from_source_file)
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    let mut provider_schemas = provider_manifests
        .iter()
        .map(ProviderSchemaSnapshot::from_manifest)
        .collect::<Vec<_>>();
    provider_schemas.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
    let go_lifecycle = go_lifecycle_snapshot_from_loaded(loaded, db);
    let ts_js_lifecycle = ts_js_lifecycle_snapshot_from_loaded(loaded, db);
    let mut tool_invocations = vec![
        tool_invocation_component(&go_lifecycle.components, "go.tool_invocation").clone(),
        tool_invocation_component(&ts_js_lifecycle.components, "ts_js.tool_invocation").clone(),
    ];
    tool_invocations.sort_by(|left, right| left.name.cmp(&right.name));

    InputSnapshot {
        schema_version: INPUT_SNAPSHOT_SCHEMA_VERSION.to_string(),
        files,
        config: config_component(loaded, config_digest),
        go_lifecycle,
        ts_js_lifecycle,
        rules: rule_components(rule_digest, plan_digest),
        models: vec![InputComponent::absent(
            "model.files",
            DigestKind::ModelFile,
            "no model files configured",
        )],
        extensions: extension_components_from_repo(&loaded.root),
        tool_invocations,
        provider_schemas,
    }
}

fn go_lifecycle_snapshot_from_loaded(
    loaded: &LoadedConfig,
    db: &AnalysisDb,
) -> GoLifecycleSnapshot {
    let components = match crate::go::lifecycle::GoAnalysisConfig::from_settings(
        &loaded.root,
        &loaded.config.languages.go,
        db,
    ) {
        Ok(config) => go_lifecycle_components(loaded, &config),
        Err(error) => go_lifecycle_error_components(loaded, error.reason()),
    };

    GoLifecycleSnapshot {
        components: sorted_components(components),
    }
}

fn ts_js_lifecycle_snapshot_from_loaded(
    loaded: &LoadedConfig,
    db: &AnalysisDb,
) -> TsJsLifecycleSnapshot {
    let source_paths = ts_js_source_paths(db);
    let lifecycle_dirs = ts_js_lifecycle_dirs(&source_paths);
    let components = vec![
        file_digest_component(
            "ts_js.package_manifests",
            DigestKind::TsJsLifecycle,
            &loaded.root,
            lifecycle_candidates(&lifecycle_dirs, package_manifest_names()),
        ),
        file_digest_component(
            "ts_js.lockfiles",
            DigestKind::TsJsLifecycle,
            &loaded.root,
            lifecycle_candidates(&lifecycle_dirs, lock_file_names()),
        ),
        file_digest_component(
            "ts_js.config_files",
            DigestKind::TsJsLifecycle,
            &loaded.root,
            ts_js_config_candidates(&loaded.root, &lifecycle_dirs),
        ),
        settings_component(
            "ts_js.resolver_options",
            DigestKind::TsJsLifecycle,
            &loaded.config.languages.ts,
        ),
        values_component(
            "ts_js.source_set_membership",
            DigestKind::TsJsLifecycle,
            source_paths,
        ),
        InputComponent::unsupported(
            "ts_js.tool_invocation",
            DigestKind::ToolInvocation,
            "not invoked by input snapshot",
        ),
    ];

    TsJsLifecycleSnapshot {
        components: sorted_components(components),
    }
}

fn tool_invocation_component<'a>(
    components: &'a [InputComponent],
    name: &str,
) -> &'a InputComponent {
    component_by_name(components, name)
}

fn go_lifecycle_components(
    loaded: &LoadedConfig,
    config: &crate::go::lifecycle::GoAnalysisConfig,
) -> Vec<InputComponent> {
    vec![
        values_component(
            "go.module_roots",
            DigestKind::GoLifecycle,
            config.module_roots.clone(),
        ),
        if config.files_without_module_root.is_empty() {
            InputComponent::absent(
                "go.files_without_module_root",
                DigestKind::GoLifecycle,
                "all discovered Go files are under module roots",
            )
        } else {
            InputComponent::setup_missing(
                "go.files_without_module_root",
                DigestKind::GoLifecycle,
                config.files_without_module_root.clone(),
            )
        },
        file_digest_component(
            "go.mod",
            DigestKind::GoLifecycle,
            &loaded.root,
            module_lifecycle_candidates(&config.module_roots, "go.mod"),
        ),
        file_digest_component(
            "go.sum",
            DigestKind::GoLifecycle,
            &loaded.root,
            module_lifecycle_candidates(&config.module_roots, "go.sum"),
        ),
        file_digest_component(
            "go.work",
            DigestKind::GoLifecycle,
            &loaded.root,
            go_work_candidates(&config.module_roots),
        ),
        values_component(
            "go.build_tags",
            DigestKind::GoLifecycle,
            config.build_tags.clone(),
        ),
        values_component(
            "go.include_tests",
            DigestKind::GoLifecycle,
            vec![config.include_tests.to_string()],
        ),
        values_component(
            "go.offline",
            DigestKind::GoLifecycle,
            vec![config.offline.to_string()],
        ),
        values_component(
            "go.rta_edges",
            DigestKind::GoLifecycle,
            vec![config.emit_rta_edges.to_string()],
        ),
        values_component(
            "go.package_patterns",
            DigestKind::GoLifecycle,
            config.package_patterns.clone(),
        ),
        InputComponent::unsupported(
            "go.tool_invocation",
            DigestKind::ToolInvocation,
            "not invoked by input snapshot",
        ),
        values_component(
            "go.environment_policy",
            DigestKind::GoLifecycle,
            vec![go_environment_policy(loaded, &config.module_roots)],
        ),
    ]
}

fn go_lifecycle_error_components(loaded: &LoadedConfig, reason: &str) -> Vec<InputComponent> {
    let setup_detail = vec![format!("error={reason}")];
    let unavailable_detail = vec![format!("module roots unavailable: {reason}")];
    vec![
        InputComponent::setup_missing(
            "go.module_roots",
            DigestKind::GoLifecycle,
            setup_detail.clone(),
        ),
        InputComponent::setup_missing(
            "go.files_without_module_root",
            DigestKind::GoLifecycle,
            setup_detail,
        ),
        InputComponent::setup_missing(
            "go.mod",
            DigestKind::GoLifecycle,
            unavailable_detail.clone(),
        ),
        InputComponent::setup_missing(
            "go.sum",
            DigestKind::GoLifecycle,
            unavailable_detail.clone(),
        ),
        InputComponent::setup_missing(
            "go.work",
            DigestKind::GoLifecycle,
            unavailable_detail.clone(),
        ),
        values_component(
            "go.build_tags",
            DigestKind::GoLifecycle,
            string_or_array_snapshot_setting(&loaded.config.languages.go, "build_tags", &[]),
        ),
        values_component(
            "go.include_tests",
            DigestKind::GoLifecycle,
            vec![
                bool_snapshot_setting(&loaded.config.languages.go, "include_tests", true)
                    .to_string(),
            ],
        ),
        values_component(
            "go.offline",
            DigestKind::GoLifecycle,
            vec![bool_snapshot_setting(&loaded.config.languages.go, "offline", false).to_string()],
        ),
        values_component(
            "go.package_patterns",
            DigestKind::GoLifecycle,
            string_or_array_snapshot_setting(
                &loaded.config.languages.go,
                "package_patterns",
                &["./..."],
            ),
        ),
        InputComponent::unsupported(
            "go.tool_invocation",
            DigestKind::ToolInvocation,
            "not invoked by input snapshot",
        ),
        InputComponent::setup_missing(
            "go.environment_policy",
            DigestKind::GoLifecycle,
            unavailable_detail,
        ),
    ]
}

fn config_component(loaded: &LoadedConfig, config_digest: &str) -> InputComponent {
    let mut detail = vec![format!("missing={}", loaded.missing)];
    detail.extend(
        loaded
            .config
            .workspace
            .include
            .iter()
            .map(|pattern| format!("workspace.include={}", normalize_relative_path(pattern))),
    );
    detail.extend(
        loaded
            .config
            .workspace
            .exclude
            .iter()
            .map(|pattern| format!("workspace.exclude={}", normalize_relative_path(pattern))),
    );
    detail.extend(
        loaded
            .config
            .rules
            .paths
            .iter()
            .map(|path| format!("rules.path={}", normalize_relative_path(path))),
    );

    InputComponent::present(
        "config.loaded",
        DigestKind::Config,
        "config_digest",
        config_digest,
        detail,
    )
}

fn values_component(name: &str, digest_kind: DigestKind, values: Vec<String>) -> InputComponent {
    let details = sorted_normalized_details(values);
    if details.is_empty() {
        return InputComponent::absent(name, digest_kind, "no values");
    }

    let detail_refs = details.iter().map(String::as_str).collect::<Vec<_>>();
    InputComponent::new(
        name,
        InputComponentStatus::Present,
        Digest::from_parts(digest_kind, name, &detail_refs),
        details,
    )
}

fn settings_component(
    name: &str,
    digest_kind: DigestKind,
    settings: &std::collections::BTreeMap<String, toml::Value>,
) -> InputComponent {
    let values = settings
        .iter()
        .map(|(key, value)| format!("{key}={}", toml_value_label(value)))
        .collect::<Vec<_>>();
    values_component(name, digest_kind, values)
}

fn file_digest_component(
    name: &str,
    digest_kind: DigestKind,
    root: &FsPath,
    candidate_paths: Vec<String>,
) -> InputComponent {
    let mut paths = candidate_paths;
    paths.sort();
    paths.dedup();

    let mut present_paths = Vec::new();
    let mut digest_parts = Vec::new();
    let mut unreadable = Vec::new();
    let mut unsupported = Vec::new();
    for relative_path in paths {
        let normalized = normalize_relative_path(&relative_path);
        let contents = match read_repo_file_with_limit(
            root,
            &normalized,
            topology_input_max_bytes(&normalized),
        ) {
            Ok(contents) => contents,
            Err(error) if error.is_not_found() => continue,
            Err(error) => {
                let detail = format!("unsupported={normalized}: {}", error.stable_reason());
                if matches!(
                    error,
                    crate::repo_fs::RepoFileReadError::TooLarge { .. }
                        | crate::repo_fs::RepoFileReadError::AbsolutePath
                        | crate::repo_fs::RepoFileReadError::EscapesRepo
                ) {
                    unsupported.push(detail);
                } else {
                    unreadable.push(format!(
                        "unreadable={normalized}: {}",
                        error.stable_reason()
                    ));
                }
                continue;
            }
        };
        present_paths.push(normalized.clone());
        digest_parts.push(format!("file={normalized}"));
        digest_parts.push(format!("content_hash={}", stable_hash_bytes(&contents)));
    }

    if !unreadable.is_empty() {
        let mut detail = present_paths;
        detail.extend(unreadable.clone());
        detail.extend(unsupported.clone());
        let mut setup_missing_digest_parts = digest_parts;
        setup_missing_digest_parts.extend(unreadable);
        setup_missing_digest_parts.extend(unsupported);
        return InputComponent::setup_missing_with_digest_parts(
            name,
            digest_kind,
            detail,
            setup_missing_digest_parts,
        );
    }

    if !unsupported.is_empty() {
        let mut detail = present_paths;
        detail.extend(unsupported.clone());
        let mut unsupported_digest_parts = digest_parts;
        unsupported_digest_parts.extend(unsupported);
        let digest_refs = unsupported_digest_parts
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        return InputComponent::new(
            name,
            InputComponentStatus::Unsupported,
            Digest::from_parts(digest_kind, name, &digest_refs),
            detail,
        );
    }

    if present_paths.is_empty() {
        return InputComponent::absent(name, digest_kind, "no lifecycle files present");
    }

    let digest_refs = digest_parts.iter().map(String::as_str).collect::<Vec<_>>();
    InputComponent::new(
        name,
        InputComponentStatus::Present,
        Digest::from_parts(digest_kind, name, &digest_refs),
        present_paths,
    )
}

fn topology_input_max_bytes(relative_path: &str) -> u64 {
    match FsPath::new(relative_path)
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

fn rule_components(rule_digest: &str, plan_digest: &str) -> Vec<InputComponent> {
    vec![
        InputComponent::present(
            "rule.code",
            DigestKind::RuleCode,
            "rule_digest",
            rule_digest,
            Vec::new(),
        ),
        InputComponent::present(
            "rule.options",
            DigestKind::RuleOptions,
            "plan_digest",
            plan_digest,
            Vec::new(),
        ),
    ]
}

fn extension_components_from_repo(root: &FsPath) -> Vec<InputComponent> {
    let discovered = discover_local_extensions(root);
    if discovered.is_empty() {
        return vec![InputComponent::absent(
            "extension.providers",
            DigestKind::ExtensionCode,
            "no extension providers configured",
        )];
    }

    discovered
        .iter()
        .map(extension_component)
        .collect::<Vec<_>>()
}

fn extension_component(extension: &DiscoveredExtension) -> InputComponent {
    let source_digest = extension.source_digest.to_string();
    let dependency_digest = extension.dependency_digest.to_string();
    let status = extension_activation_status_label(extension.activation_status);
    let mut detail = vec![
        format!("extension_id={}", extension.extension_id),
        format!("manifest={}", extension.manifest_path),
        format!("activation_status={status}"),
        format!("source_digest={source_digest}"),
        format!("dependency_digest={dependency_digest}"),
    ];
    detail.extend(
        extension
            .digest_input_paths
            .iter()
            .map(|path| format!("digest_input={path}")),
    );
    let digest_refs = detail.iter().map(String::as_str).collect::<Vec<_>>();
    InputComponent::new(
        format!("extension.providers.{}", extension.extension_id),
        InputComponentStatus::Present,
        Digest::from_parts(
            DigestKind::ExtensionCode,
            "extension_provider",
            &digest_refs,
        ),
        detail,
    )
}

fn module_lifecycle_candidates(module_roots: &[String], file_name: &str) -> Vec<String> {
    module_roots
        .iter()
        .map(|module_root| {
            if module_root == "." {
                file_name.to_string()
            } else {
                format!("{module_root}/{file_name}")
            }
        })
        .collect()
}

fn go_work_candidates(module_roots: &[String]) -> Vec<String> {
    let mut candidates = vec!["go.work".to_string()];
    candidates.extend(module_lifecycle_candidates(module_roots, "go.work"));
    candidates
}

fn go_environment_policy(loaded: &LoadedConfig, module_roots: &[String]) -> String {
    let checked_in_go_work = loaded.root.join("go.work");
    if repo_file_exists(&loaded.root, "go.work")
        && crate::go::lifecycle::go_work_covers_module_roots(
            &loaded.root,
            &checked_in_go_work,
            module_roots,
        )
    {
        "checked_in_workspace_file".to_string()
    } else if module_roots.len() == 1
        && module_roots[0] == "."
        && repo_file_exists(&loaded.root, "go.mod")
    {
        "single_root_module".to_string()
    } else if module_roots.is_empty() {
        "no_module_roots".to_string()
    } else {
        "internal_temporary_workspace_if_needed".to_string()
    }
}

fn ts_js_source_paths(db: &AnalysisDb) -> Vec<String> {
    let mut paths = db
        .files()
        .iter()
        .filter(|file| file.language.is_ts_family())
        .map(|file| normalize_relative_path(&file.relative_path))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn ts_js_lifecycle_dirs(source_paths: &[String]) -> BTreeSet<String> {
    let mut dirs = BTreeSet::new();
    dirs.insert(String::new());
    for source_path in source_paths {
        let mut current = FsPath::new(source_path).parent();
        while let Some(parent) = current {
            let normalized = normalize_relative_path(&parent.to_string_lossy());
            if normalized.is_empty() {
                break;
            }
            dirs.insert(normalized);
            current = parent.parent();
        }
    }
    dirs
}

fn lifecycle_candidates(dirs: &BTreeSet<String>, file_names: Vec<String>) -> Vec<String> {
    let mut candidates = Vec::new();
    for dir in dirs {
        for file_name in &file_names {
            if dir.is_empty() {
                candidates.push(file_name.clone());
            } else {
                candidates.push(format!("{dir}/{file_name}"));
            }
        }
    }
    candidates
}

fn package_manifest_names() -> Vec<String> {
    vec!["package.json".to_string()]
}

fn lock_file_names() -> Vec<String> {
    vec![
        "package-lock.json".to_string(),
        "npm-shrinkwrap.json".to_string(),
        ["p", "n", "p", "m", "-lock.yaml"].concat(),
        ["ya", "rn.lock"].concat(),
        ["b", "un.lock"].concat(),
    ]
}

fn config_file_names() -> Vec<String> {
    vec![ts_config_name(), "jsconfig.json".to_string()]
}

fn ts_config_name() -> String {
    ["t", "sconfig.json"].concat()
}

fn ts_js_config_candidates(root: &FsPath, lifecycle_dirs: &BTreeSet<String>) -> Vec<String> {
    let mut candidates = lifecycle_candidates(lifecycle_dirs, config_file_names());
    candidates.extend(extended_ts_config_candidates(
        root,
        candidates.iter().map(String::as_str),
    ));
    candidates
}

fn extended_ts_config_candidates<'a>(
    root: &FsPath,
    initial_candidates: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut visited = BTreeSet::new();
    let mut discovered = BTreeSet::new();
    for candidate in initial_candidates {
        let file_name = FsPath::new(candidate)
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
    root: &FsPath,
    config_path: &FsPath,
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
    let config_dir = FsPath::new(&config_path)
        .parent()
        .unwrap_or(FsPath::new(""));
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

fn read_tsconfig_extends_wire(
    root: &FsPath,
    relative_path: &str,
) -> Option<TsconfigExtendsOnlyWire> {
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
    root: &FsPath,
    config_dir: &FsPath,
    specifier: &str,
) -> Option<String> {
    let specifier_path = FsPath::new(specifier);
    if specifier_path.is_absolute() {
        return None;
    }
    if specifier.starts_with('.') {
        return resolve_tsconfig_file_candidate(root, &config_dir.join(specifier_path));
    }
    resolve_package_tsconfig_extends_path(root, config_dir, specifier)
}

fn resolve_package_tsconfig_extends_path(
    root: &FsPath,
    config_dir: &FsPath,
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

fn resolve_tsconfig_file_candidate(root: &FsPath, base: &FsPath) -> Option<String> {
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

fn normalized_existing_repo_file(root: &FsPath, candidate: &FsPath) -> Option<String> {
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

fn string_or_array_snapshot_setting(
    settings: &std::collections::BTreeMap<String, toml::Value>,
    key: &str,
    default: &[&str],
) -> Vec<String> {
    let values = settings.get(key).map_or_else(
        || default.iter().map(|value| (*value).to_string()).collect(),
        string_or_array_snapshot_value,
    );
    sorted_normalized_details(values)
}

fn string_or_array_snapshot_value(value: &toml::Value) -> Vec<String> {
    match value {
        toml::Value::String(value) => vec![value.clone()],
        toml::Value::Array(values) => values
            .iter()
            .filter_map(toml::Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn bool_snapshot_setting(
    settings: &std::collections::BTreeMap<String, toml::Value>,
    key: &str,
    default: bool,
) -> bool {
    settings
        .get(key)
        .and_then(toml::Value::as_bool)
        .unwrap_or(default)
}

fn sorted_components(mut components: Vec<InputComponent>) -> Vec<InputComponent> {
    components.sort_by(|left, right| left.name.cmp(&right.name));
    components
}

fn component_by_name<'a>(components: &'a [InputComponent], name: &str) -> &'a InputComponent {
    components
        .iter()
        .find(|component| component.name == name)
        .unwrap_or_else(|| panic!("snapshot is missing component {name}"))
}

fn sorted_normalized_details(details: Vec<String>) -> Vec<String> {
    let mut details = details
        .into_iter()
        .map(|detail| normalize_detail(&detail))
        .collect::<Vec<_>>();
    details.sort();
    details.dedup();
    details
}

fn normalize_detail(value: &str) -> String {
    value.replace('\\', "/")
}

fn normalize_relative_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_string()
}

fn toml_value_label(value: &toml::Value) -> String {
    match value {
        toml::Value::String(value) => format!("{value:?}"),
        toml::Value::Integer(value) => value.to_string(),
        toml::Value::Float(value) => value.to_string(),
        toml::Value::Boolean(value) => value.to_string(),
        toml::Value::Datetime(value) => value.to_string(),
        toml::Value::Array(values) => {
            let labels = values.iter().map(toml_value_label).collect::<Vec<_>>();
            format!("[{}]", labels.join(","))
        }
        toml::Value::Table(values) => {
            let mut labels = values
                .iter()
                .map(|(key, value)| format!("{key}:{}", toml_value_label(value)))
                .collect::<Vec<_>>();
            labels.sort();
            format!("{{{}}}", labels.join(","))
        }
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

fn extension_activation_status_label(
    status: crate::analysis::extensions::manifest::ExtensionActivationStatus,
) -> &'static str {
    match status {
        crate::analysis::extensions::manifest::ExtensionActivationStatus::Configured => {
            "configured"
        }
        crate::analysis::extensions::manifest::ExtensionActivationStatus::Discovered => {
            "discovered"
        }
        crate::analysis::extensions::manifest::ExtensionActivationStatus::HandshakeOk => {
            "handshake_ok"
        }
        crate::analysis::extensions::manifest::ExtensionActivationStatus::Active => "active",
        crate::analysis::extensions::manifest::ExtensionActivationStatus::Failed => "failed",
        crate::analysis::extensions::manifest::ExtensionActivationStatus::ValidationFailed => {
            "validation_failed"
        }
        crate::analysis::extensions::manifest::ExtensionActivationStatus::Disabled => "disabled",
    }
}

#[cfg(test)]
mod source_config_rule_model_extension {
    use super::*;
    use crate::analysis_kernel::{
        CachePolicy, PrecisionCeiling, ProviderKind, ProviderManifest, SchemaVersion,
    };
    use crate::config::{LoadedConfig, PolintConfig};
    use crate::core::AnalysisDb;
    use std::path::Path;
    use tempfile::TempDir;

    fn loaded_config(root: &Path) -> LoadedConfig {
        LoadedConfig {
            root: root.to_path_buf(),
            config: PolintConfig::default(),
            missing: false,
            respect_gitignore: true,
        }
    }

    fn db_with_files(root: &Path, files: &[(&str, &str)]) -> AnalysisDb {
        let mut db = AnalysisDb::new();
        for (relative_path, source) in files {
            let path = root.join(relative_path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create fixture parent");
            }
            std::fs::write(&path, source).expect("write fixture source");
            db.add_file(path, (*relative_path).to_string(), (*source).to_string());
        }
        db
    }

    fn snapshot_for(
        loaded: &LoadedConfig,
        db: &AnalysisDb,
        provider_manifests: &[ProviderManifest],
    ) -> InputSnapshot {
        crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            loaded,
            db,
            "config-digest",
            "rule-digest",
            "plan-digest",
            provider_manifests,
        )
    }

    #[test]
    fn snapshots_from_same_inputs_serialize_to_identical_pretty_json() {
        let temp = TempDir::new().expect("create temp repo");
        let loaded = loaded_config(temp.path());
        let db = db_with_files(
            temp.path(),
            &[
                ("src/z.ts", "export const z = 1;\n"),
                ("cmd/main.go", "package main\n"),
            ],
        );

        let first = snapshot_for(
            &loaded,
            &db,
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );
        let second = snapshot_for(
            &loaded,
            &db,
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );

        assert_eq!(
            serde_json::to_string_pretty(&first).expect("serialize first snapshot"),
            serde_json::to_string_pretty(&second).expect("serialize second snapshot")
        );
    }

    #[test]
    fn file_rows_expose_safe_identity_without_source_or_machine_paths() {
        let temp = TempDir::new().expect("create temp repo");
        let loaded = loaded_config(temp.path());
        let db = db_with_files(
            temp.path(),
            &[("src/app.ts", "const secret = 'raw text';\n")],
        );

        let snapshot = snapshot_for(
            &loaded,
            &db,
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );
        let file = &snapshot.files[0];
        let rendered = serde_json::to_string_pretty(&snapshot).expect("serialize snapshot");

        assert_eq!(file.relative_path, "src/app.ts");
        assert_eq!(file.language, crate::core::Language::TypeScript);
        assert_eq!(file.source_text_digest.kind, DigestKind::SourceText);
        assert_eq!(file.size_bytes, "const secret = 'raw text';\n".len());
        assert!(file.mtime_hint_present);
        assert!(!rendered.contains("const secret"));
        assert!(!rendered.contains(temp.path().to_string_lossy().as_ref()));
        assert!(!rendered.contains(db.files()[0].path.to_string_lossy().as_ref()));
    }

    #[test]
    fn config_rule_plan_model_extension_and_provider_components_are_typed() {
        let temp = TempDir::new().expect("create temp repo");
        let loaded = loaded_config(temp.path());
        let db = db_with_files(temp.path(), &[("src/app.ts", "export const app = 1;\n")]);

        let snapshot = snapshot_for(
            &loaded,
            &db,
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );

        assert_eq!(snapshot.config.status, InputComponentStatus::Present);
        assert_eq!(snapshot.config.digest.kind, DigestKind::Config);
        assert!(
            snapshot
                .rules
                .iter()
                .any(|component| component.digest.kind == DigestKind::RuleCode)
        );
        assert!(
            snapshot
                .rules
                .iter()
                .any(|component| component.digest.kind == DigestKind::RuleOptions)
        );
        assert_eq!(snapshot.models[0].name, "model.files");
        assert_eq!(snapshot.models[0].status, InputComponentStatus::Absent);
        assert_eq!(snapshot.models[0].digest.kind, DigestKind::ModelFile);
        assert_eq!(snapshot.extensions[0].name, "extension.providers");
        assert_eq!(snapshot.extensions[0].status, InputComponentStatus::Absent);
        assert_eq!(
            snapshot.extensions[0].digest.kind,
            DigestKind::ExtensionCode
        );
        assert!(snapshot.provider_schemas.iter().all(|provider| {
            provider.provider_manifest_digest.kind == DigestKind::ProviderParameters
        }));
    }

    #[test]
    fn configured_local_extension_records_present_extension_code_component() {
        let temp = TempDir::new().expect("create temp repo");
        let loaded = loaded_config(temp.path());
        let db = db_with_files(temp.path(), &[("src/app.ts", "export const app = 1;\n")]);
        std::fs::create_dir_all(temp.path().join(".polint/extensions/demo/src"))
            .expect("create extension src");
        std::fs::write(
            temp.path().join(".polint/extensions/demo/Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write extension manifest");
        std::fs::write(
            temp.path().join(".polint/extensions/demo/src/main.rs"),
            "fn main() {}\n",
        )
        .expect("write extension source");

        let snapshot = snapshot_for(
            &loaded,
            &db,
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );
        let rendered = serde_json::to_string_pretty(&snapshot).expect("serialize snapshot");

        assert_eq!(snapshot.extensions.len(), 1);
        assert_eq!(snapshot.extensions[0].status, InputComponentStatus::Present);
        assert_eq!(
            snapshot.extensions[0].digest.kind,
            DigestKind::ExtensionCode
        );
        assert!(
            snapshot.extensions[0]
                .detail
                .contains(&"manifest=.polint/extensions/demo/Cargo.toml".to_string())
        );
        assert!(rendered.contains("activation_status=discovered"));
        assert!(!rendered.contains(temp.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn source_file_rows_are_sorted_by_normalized_relative_path() {
        let temp = TempDir::new().expect("create temp repo");
        let loaded = loaded_config(temp.path());
        let db = db_with_files(
            temp.path(),
            &[
                ("src/z.ts", "export const z = 1;\n"),
                ("cmd/main.go", "package main\n"),
                ("src/a.tsx", "export function A() { return null; }\n"),
            ],
        );

        let snapshot = snapshot_for(
            &loaded,
            &db,
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );

        assert_eq!(
            snapshot
                .files
                .iter()
                .map(|file| file.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["cmd/main.go", "src/a.tsx", "src/z.ts"]
        );
    }

    #[test]
    fn provider_schema_rows_include_manifest_identity_and_digest_scope_policy() {
        const SCHEMAS: &[SchemaVersion] = &[SchemaVersion {
            name: "example-facts",
            version: 1,
        }];
        let temp = TempDir::new().expect("create temp repo");
        let loaded = loaded_config(temp.path());
        let db = db_with_files(temp.path(), &[("src/app.ts", "export const app = 1;\n")]);
        let base = ProviderManifest {
            id: "polint.example",
            kind: ProviderKind::LanguageSyntax,
            inputs: &["source_files", "config"],
            outputs: &["facts"],
            language_ids: crate::frontend::LANGUAGE_IDS_GO,
            cache_policy: CachePolicy::NoCache,
            schema_versions: SCHEMAS,
            precision_ceiling: PrecisionCeiling::Syntax,
        };
        let scope_changed = ProviderManifest {
            language_ids: crate::frontend::LANGUAGE_IDS_TS,
            ..base
        };
        let policy_changed = ProviderManifest {
            cache_policy: CachePolicy::InMemoryDerived,
            ..base
        };

        let base_snapshot = snapshot_for(&loaded, &db, &[base]);
        let scope_snapshot = snapshot_for(&loaded, &db, &[scope_changed]);
        let policy_snapshot = snapshot_for(&loaded, &db, &[policy_changed]);
        let row = &base_snapshot.provider_schemas[0];

        assert_eq!(row.provider_id, "polint.example");
        assert_eq!(row.schema_versions, vec!["example-facts:1"]);
        assert_eq!(row.language_scope, "go");
        assert_eq!(row.cache_policy, "no_cache");
        assert_eq!(row.precision_ceiling, "syntax");
        assert_ne!(
            row.provider_manifest_digest,
            scope_snapshot.provider_schemas[0].provider_manifest_digest
        );
        assert_ne!(
            row.provider_manifest_digest,
            policy_snapshot.provider_schemas[0].provider_manifest_digest
        );
    }
}

#[cfg(test)]
mod lifecycle {
    use super::*;
    use crate::analysis_kernel::ProviderManifest;
    use crate::config::{LoadedConfig, PolintConfig};
    use crate::core::AnalysisDb;
    use std::path::Path;
    use tempfile::TempDir;

    fn loaded_config(root: &Path, config: PolintConfig) -> LoadedConfig {
        LoadedConfig {
            root: root.to_path_buf(),
            config,
            missing: false,
            respect_gitignore: true,
        }
    }

    fn default_loaded_config(root: &Path) -> LoadedConfig {
        loaded_config(root, PolintConfig::default())
    }

    fn db_with_files(root: &Path, files: &[(&str, &str)]) -> AnalysisDb {
        let mut db = AnalysisDb::new();
        for (relative_path, source) in files {
            write_file(root, relative_path, source);
            db.add_file(
                root.join(relative_path),
                (*relative_path).to_string(),
                (*source).to_string(),
            );
        }
        db
    }

    fn write_file(root: &Path, relative_path: &str, contents: &str) {
        let path = root.join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture parent");
        }
        std::fs::write(path, contents).expect("write fixture file");
    }

    fn snapshot_for(
        loaded: &LoadedConfig,
        db: &AnalysisDb,
        provider_manifests: &[ProviderManifest],
    ) -> InputSnapshot {
        crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            loaded,
            db,
            "config-digest",
            "rule-digest",
            "plan-digest",
            provider_manifests,
        )
    }

    fn component<'a>(components: &'a [InputComponent], name: &str) -> &'a InputComponent {
        components
            .iter()
            .find(|component| component.name == name)
            .unwrap_or_else(|| panic!("missing component {name}"))
    }

    fn component_names(components: &[InputComponent]) -> Vec<&str> {
        components
            .iter()
            .map(|component| component.name.as_str())
            .collect()
    }

    #[test]
    fn go_lifecycle_records_module_files_and_configured_options() {
        let temp = TempDir::new().expect("create temp repo");
        write_file(
            temp.path(),
            "services/app/go.mod",
            "module example.com/app\n\ngo 1.24\n",
        );
        write_file(
            temp.path(),
            "services/app/go.sum",
            "example.com/dep v1.0.0 h1:abc\n",
        );
        write_file(temp.path(), "go.work", "go 1.24\n\nuse ./services/app\n");

        let mut config = PolintConfig::default();
        config.languages.go.insert(
            "module_roots".to_string(),
            toml::Value::Array(vec![toml::Value::String("services/app".to_string())]),
        );
        config.languages.go.insert(
            "build_tags".to_string(),
            toml::Value::Array(vec![
                toml::Value::String("prod".to_string()),
                toml::Value::String("linux".to_string()),
            ]),
        );
        config
            .languages
            .go
            .insert("include_tests".to_string(), toml::Value::Boolean(false));
        config
            .languages
            .go
            .insert("offline".to_string(), toml::Value::Boolean(true));
        config.languages.go.insert(
            "package_patterns".to_string(),
            toml::Value::Array(vec![
                toml::Value::String("./internal/...".to_string()),
                toml::Value::String("./cmd/...".to_string()),
            ]),
        );

        let loaded = loaded_config(temp.path(), config);
        let db = db_with_files(temp.path(), &[("services/app/main.go", "package main\n")]);
        let snapshot = snapshot_for(
            &loaded,
            &db,
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );
        let components = &snapshot.go_lifecycle.components;

        assert_eq!(
            component(components, "go.module_roots").detail,
            vec!["services/app"]
        );
        assert_eq!(
            component(components, "go.mod").status,
            InputComponentStatus::Present
        );
        assert_eq!(
            component(components, "go.sum").status,
            InputComponentStatus::Present
        );
        assert_eq!(
            component(components, "go.work").status,
            InputComponentStatus::Present
        );
        assert_eq!(
            component(components, "go.build_tags").detail,
            vec!["linux", "prod"]
        );
        assert_eq!(
            component(components, "go.include_tests").detail,
            vec!["false"]
        );
        assert_eq!(component(components, "go.offline").detail, vec!["true"]);
        assert_eq!(
            component(components, "go.package_patterns").detail,
            vec!["./cmd/...", "./internal/..."]
        );
        assert_eq!(
            component(components, "go.environment_policy").status,
            InputComponentStatus::Present
        );
    }

    #[test]
    fn go_lifecycle_records_temporary_workspace_policy_when_root_go_work_misses_roots() {
        let temp = TempDir::new().expect("create temp repo");
        write_file(
            temp.path(),
            "services/app/go.mod",
            "module example.com/app\n\ngo 1.24\n",
        );
        write_file(temp.path(), "services/app/main.go", "package main\n");
        write_file(temp.path(), "go.work", "go 1.24\n\nuse ./other\n");

        let mut config = PolintConfig::default();
        config.languages.go.insert(
            "module_roots".to_string(),
            toml::Value::Array(vec![toml::Value::String("services/app".to_string())]),
        );
        let loaded = loaded_config(temp.path(), config);
        let db = db_with_files(temp.path(), &[("services/app/main.go", "package main\n")]);
        let snapshot = snapshot_for(
            &loaded,
            &db,
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );

        assert_eq!(
            component(&snapshot.go_lifecycle.components, "go.environment_policy").detail,
            vec!["internal_temporary_workspace_if_needed"]
        );
    }

    #[test]
    fn go_lifecycle_keeps_full_component_vocabulary_when_config_is_invalid() {
        let temp = TempDir::new().expect("create temp repo");
        let mut config = PolintConfig::default();
        config.languages.go.insert(
            "module_roots".to_string(),
            toml::Value::Array(vec![toml::Value::String("../outside".to_string())]),
        );
        let loaded = loaded_config(temp.path(), config);
        let db = db_with_files(temp.path(), &[("main.go", "package main\n")]);
        let snapshot = snapshot_for(
            &loaded,
            &db,
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );

        assert_eq!(
            component_names(&snapshot.go_lifecycle.components),
            vec![
                "go.build_tags",
                "go.environment_policy",
                "go.files_without_module_root",
                "go.include_tests",
                "go.mod",
                "go.module_roots",
                "go.offline",
                "go.package_patterns",
                "go.sum",
                "go.tool_invocation",
                "go.work",
            ]
        );
        assert_eq!(
            component(&snapshot.go_lifecycle.components, "go.module_roots").status,
            InputComponentStatus::SetupMissing
        );
        assert_eq!(
            component(&snapshot.go_lifecycle.components, "go.tool_invocation").status,
            InputComponentStatus::Unsupported
        );
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_lifecycle_file_is_setup_missing_not_absent() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().expect("create temp repo");
        write_file(temp.path(), "package.json", "{\"type\":\"module\"}\n");
        write_file(temp.path(), "src/app.ts", "export const app = 1;\n");
        let unreadable = temp.path().join("package.json");
        let original_permissions = std::fs::metadata(&unreadable)
            .expect("package metadata")
            .permissions();
        let mut locked_permissions = original_permissions.clone();
        locked_permissions.set_mode(0o0);
        std::fs::set_permissions(&unreadable, locked_permissions).expect("lock package.json");

        let loaded = default_loaded_config(temp.path());
        let db = db_with_files(temp.path(), &[("src/app.ts", "export const app = 1;\n")]);
        let snapshot = snapshot_for(
            &loaded,
            &db,
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );

        std::fs::set_permissions(&unreadable, original_permissions).expect("restore package.json");
        let package_manifests = component(
            &snapshot.ts_js_lifecycle.components,
            "ts_js.package_manifests",
        );
        assert_eq!(package_manifests.status, InputComponentStatus::SetupMissing);
        assert!(
            package_manifests
                .detail
                .iter()
                .any(|detail| detail.starts_with("unreadable=package.json:")),
            "{package_manifests:#?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn setup_missing_lifecycle_digest_changes_when_readable_file_content_changes() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().expect("create temp repo");
        write_file(temp.path(), "package.json", "{\"version\":1}\n");
        write_file(temp.path(), "web/package.json", "{\"version\":1}\n");
        let unreadable = temp.path().join("web/package.json");
        let original_permissions = std::fs::metadata(&unreadable)
            .expect("package metadata")
            .permissions();
        let mut locked_permissions = original_permissions.clone();
        locked_permissions.set_mode(0o0);
        std::fs::set_permissions(&unreadable, locked_permissions).expect("lock package.json");

        let first = file_digest_component(
            "ts_js.package_manifests",
            DigestKind::TsJsLifecycle,
            temp.path(),
            vec!["package.json".to_string(), "web/package.json".to_string()],
        );
        write_file(temp.path(), "package.json", "{\"version\":2}\n");
        let second = file_digest_component(
            "ts_js.package_manifests",
            DigestKind::TsJsLifecycle,
            temp.path(),
            vec!["package.json".to_string(), "web/package.json".to_string()],
        );

        std::fs::set_permissions(&unreadable, original_permissions).expect("restore package.json");
        assert_eq!(first.status, InputComponentStatus::SetupMissing);
        assert_eq!(second.status, InputComponentStatus::SetupMissing);
        assert_ne!(first.digest, second.digest);
    }

    #[test]
    fn ts_js_lifecycle_records_manifests_config_resolver_options_and_sources() {
        let temp = TempDir::new().expect("create temp repo");
        write_file(temp.path(), "package.json", "{\"type\":\"module\"}\n");
        write_file(
            temp.path(),
            "package-lock.json",
            "{\"lockfileVersion\":3}\n",
        );
        let ts_config = ts_config_name();
        write_file(
            temp.path(),
            "tsconfig.base.json",
            "{\"compilerOptions\":{\"baseUrl\":\".\"}}\n",
        );
        write_file(
            temp.path(),
            &ts_config,
            "{\"extends\":\"./tsconfig.base.json\",\"compilerOptions\":{}}\n",
        );

        let mut config = PolintConfig::default();
        config.languages.ts.insert(
            "module_resolution".to_string(),
            toml::Value::String("node".to_string()),
        );
        config.languages.ts.insert(
            "conditions".to_string(),
            toml::Value::Array(vec![
                toml::Value::String("import".to_string()),
                toml::Value::String("browser".to_string()),
            ]),
        );

        let loaded = loaded_config(temp.path(), config);
        let db = db_with_files(
            temp.path(),
            &[
                ("src/app.ts", "export const app = 1;\n"),
                ("src/view.tsx", "export function View() { return null; }\n"),
            ],
        );
        let snapshot = snapshot_for(
            &loaded,
            &db,
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );
        let components = &snapshot.ts_js_lifecycle.components;

        assert_eq!(
            component(components, "ts_js.package_manifests").detail,
            vec!["package.json"]
        );
        assert_eq!(
            component(components, "ts_js.lockfiles").detail,
            vec!["package-lock.json"]
        );
        assert_eq!(
            component(components, "ts_js.config_files").detail,
            vec!["tsconfig.base.json".to_string(), ts_config]
        );
        assert!(
            component(components, "ts_js.resolver_options")
                .detail
                .iter()
                .any(|detail| detail.contains("module_resolution"))
        );
        assert_eq!(
            component(components, "ts_js.source_set_membership").detail,
            vec!["src/app.ts", "src/view.tsx"]
        );
    }

    #[test]
    fn ts_js_lifecycle_rejects_absolute_tsconfig_extends_outside_repo() {
        let repo = TempDir::new().expect("create temp repo");
        let outside = TempDir::new().expect("create outside tempdir");
        write_file(
            outside.path(),
            "tsconfig.base.json",
            "{\"compilerOptions\":{\"baseUrl\":\".\"}}\n",
        );
        let outside_config = outside.path().join("tsconfig.base.json");
        write_file(
            repo.path(),
            &ts_config_name(),
            &format!(
                "{{\"extends\":{}}}\n",
                serde_json::to_string(&outside_config).unwrap()
            ),
        );
        let loaded = default_loaded_config(repo.path());
        let db = db_with_files(repo.path(), &[("src/app.ts", "export const app = 1;\n")]);

        let snapshot = snapshot_for(
            &loaded,
            &db,
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );
        let config_files = component(&snapshot.ts_js_lifecycle.components, "ts_js.config_files");

        assert_eq!(config_files.detail, vec![ts_config_name()]);
    }

    #[cfg(unix)]
    #[test]
    fn ts_js_lifecycle_does_not_hash_symlink_escape_contents() {
        let repo = TempDir::new().expect("create temp repo");
        let outside = TempDir::new().expect("create outside tempdir");
        write_file(
            outside.path(),
            "package.json",
            "{\"name\":\"outside-one\"}\n",
        );
        std::os::unix::fs::symlink(
            outside.path().join("package.json"),
            repo.path().join("package.json"),
        )
        .expect("create symlink");

        let first = file_digest_component(
            "ts_js.package_manifests",
            DigestKind::TsJsLifecycle,
            repo.path(),
            vec!["package.json".to_string()],
        );
        write_file(
            outside.path(),
            "package.json",
            "{\"name\":\"outside-two\"}\n",
        );
        let second = file_digest_component(
            "ts_js.package_manifests",
            DigestKind::TsJsLifecycle,
            repo.path(),
            vec!["package.json".to_string()],
        );

        assert_eq!(first.digest, second.digest);
        assert_eq!(first.status, InputComponentStatus::Unsupported);
    }

    #[test]
    fn official_tool_identity_components_are_unsupported_when_not_invoked() {
        let temp = TempDir::new().expect("create temp repo");
        let loaded = default_loaded_config(temp.path());
        let db = db_with_files(
            temp.path(),
            &[
                ("main.go", "package main\n"),
                ("src/app.ts", "export const app = 1;\n"),
            ],
        );

        let snapshot = snapshot_for(
            &loaded,
            &db,
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );

        assert_eq!(
            component(&snapshot.go_lifecycle.components, "go.tool_invocation").status,
            InputComponentStatus::Unsupported
        );
        assert_eq!(
            component(
                &snapshot.ts_js_lifecycle.components,
                "ts_js.tool_invocation"
            )
            .status,
            InputComponentStatus::Unsupported
        );
        assert!(snapshot.tool_invocations.iter().all(|component| {
            component.status == InputComponentStatus::Unsupported
                && component.digest.kind == DigestKind::ToolInvocation
        }));
    }

    #[test]
    fn go_files_outside_module_roots_are_setup_missing_root_relative_components() {
        let temp = TempDir::new().expect("create temp repo");
        let loaded = default_loaded_config(temp.path());
        let db = db_with_files(temp.path(), &[("cmd/tool/main.go", "package main\n")]);

        let snapshot = snapshot_for(
            &loaded,
            &db,
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );
        let gap = component(
            &snapshot.go_lifecycle.components,
            "go.files_without_module_root",
        );

        assert_eq!(gap.status, InputComponentStatus::SetupMissing);
        assert_eq!(gap.detail, vec!["cmd/tool/main.go"]);
        assert!(
            !serde_json::to_string_pretty(&snapshot)
                .expect("serialize snapshot")
                .contains(temp.path().to_string_lossy().as_ref())
        );
    }
}
