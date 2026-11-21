use crate::analysis_api::{
    FactDatabase, ModuleEdgeKind, ResolutionPrecision, ResolutionStatus, SourceFile,
    UnresolvedReason,
};
use crate::internal_core::{FileId, Language, ModuleNodeId, StableKeyInterner};
mod formats;

use crate::analysis_neutral::module_graph::model::{
    ModuleGraphBuilder, ModuleNodeDraft, ResolvedImportDraft, ResolverInput,
};
use crate::analysis_neutral::module_graph::topology::{
    DependencyRequirementFact, DependencyRequirementId, RequirementKind,
    ResolvedDependencyEdgeFact, ResolvedDependencyEdgeId, ResolvedDependencyKind, SourceSetFact,
    SourceSetId, SourceSetKind, TopologyOutput, TopologyPackageFact, TopologyPackageId,
    TopologyPackageKind, TopologyPrecision, TopologyStatus, WorkspaceRootFact, WorkspaceRootId,
    WorkspaceRootKind,
};
use crate::go::lifecycle::{self, GoAnalysisConfig};
use crate::go::process_runner::{GO_SUBPROCESS_TIMEOUT, run_bounded};
use crate::go::repo_fs as paths;
use crate::go::repo_fs::{
    TOPOLOGY_LOCKFILE_MAX_BYTES, TOPOLOGY_MANIFEST_MAX_BYTES, read_repo_file_to_string_with_limit,
    repo_file_exists,
};
use formats::go_mod::parse_go_mod;
use formats::go_work::parse_go_work;
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::Duration;

const GO_TOPOLOGY_PROVIDER_ID: &str = "polint.module_graph";

thread_local! {
    static MODULE_GRAPH_STABLE_KEYS: RefCell<Option<StableKeyInterner>> = const { RefCell::new(None) };
}

fn with_module_graph_stable_keys<T>(
    interner: &StableKeyInterner,
    operation: impl FnOnce() -> T,
) -> T {
    MODULE_GRAPH_STABLE_KEYS.with(|slot| {
        let previous = slot.replace(Some(interner.clone()));
        let result = operation();
        slot.replace(previous);
        result
    })
}

fn intern_module_graph_stable_key(
    key: impl AsRef<str> + Into<String>,
) -> crate::internal_core::StableKeyId {
    MODULE_GRAPH_STABLE_KEYS.with(|slot| {
        slot.borrow()
            .as_ref()
            .expect("module graph stable-key interner is installed during topology derivation")
            .intern(key)
    })
}

#[derive(Debug, Clone)]
pub struct GoPackageIndex {
    by_import_path: BTreeMap<String, GoPackageMetadata>,
    file_to_import_path: BTreeMap<FileId, String>,
    module_paths: BTreeSet<String>,
    setup_missing_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct GoModuleOwnership {
    file_owner_modules: BTreeMap<FileId, ModuleNodeId>,
    package_nodes_by_file: BTreeMap<FileId, ModuleNodeId>,
}

impl GoModuleOwnership {
    pub fn package_node_for_file(&self, file: FileId) -> Option<ModuleNodeId> {
        self.package_nodes_by_file.get(&file).copied()
    }

    pub fn file_owner_modules(&self) -> impl Iterator<Item = (FileId, ModuleNodeId)> + '_ {
        self.file_owner_modules
            .iter()
            .map(|(file, node)| (*file, *node))
    }

    pub fn package_nodes_by_file(&self) -> impl Iterator<Item = (FileId, ModuleNodeId)> + '_ {
        self.package_nodes_by_file
            .iter()
            .map(|(file, node)| (*file, *node))
    }
}

impl Default for GoPackageIndex {
    fn default() -> Self {
        Self::setup_missing("Go package metadata was not loaded.")
    }
}

impl GoPackageIndex {
    pub fn load(
        root: &Path,
        settings: &BTreeMap<String, toml::Value>,
        db: &dyn FactDatabase,
    ) -> Self {
        let config = match GoAnalysisConfig::from_settings(root, settings, db) {
            Ok(config) => config,
            Err(error) => return Self::setup_missing(error.reason()),
        };
        Self::load_with_runner(root, db, &config, run_go_list)
    }

    fn load_with_runner(
        root: &Path,
        db: &dyn FactDatabase,
        config: &GoAnalysisConfig,
        run: impl FnOnce(&Path, &GoAnalysisConfig) -> GoCommandOutput,
    ) -> Self {
        if !config.files_without_module_root.is_empty() {
            return Self::setup_missing("some Go files are not under a go.mod module root.");
        }
        let missing_roots = config.missing_module_roots(root);
        if !missing_roots.is_empty() {
            return Self::setup_missing(format!(
                "configured Go module roots are missing go.mod: {}.",
                missing_roots.join(", ")
            ));
        }

        let output = run(root, config);
        if !output.status.success() {
            return Self::setup_missing(go_list_failure_reason(&output));
        }

        Self::from_go_list_stdout(root, db, &output.stdout)
    }

    fn from_go_list_stdout(root: &Path, db: &dyn FactDatabase, stdout: &[u8]) -> Self {
        let mut packages = Vec::new();
        let stream = serde_json::Deserializer::from_slice(stdout).into_iter::<GoListPackage>();
        for parsed in stream {
            match parsed {
                Ok(package) => packages.push(package),
                Err(error) => {
                    return Self::setup_missing(format!(
                        "go list -json ./... failed to parse output: {error}"
                    ));
                }
            }
        }
        packages.sort_by(|left, right| left.import_path.cmp(&right.import_path));

        let file_ids = db
            .files()
            .iter()
            .filter_map(|file| {
                paths::normalize_repo_relative(&file.relative_path)
                    .map(|relative_path| (relative_path, file.id))
            })
            .collect::<BTreeMap<_, _>>();

        let mut index = Self {
            by_import_path: BTreeMap::new(),
            file_to_import_path: BTreeMap::new(),
            module_paths: BTreeSet::new(),
            setup_missing_reason: None,
        };
        for package in packages {
            if package.import_path.is_empty() {
                continue;
            }
            let metadata = GoPackageMetadata::from_go_list_package(root, &file_ids, package);
            if let Some(module_path) = &metadata.module_path {
                index.module_paths.insert(module_path.clone());
            }
            for file in &metadata.files {
                index
                    .file_to_import_path
                    .insert(*file, metadata.import_path.clone());
            }
            index
                .by_import_path
                .insert(metadata.import_path.clone(), metadata);
        }
        index
    }

    fn setup_missing(reason: impl Into<String>) -> Self {
        Self {
            by_import_path: BTreeMap::new(),
            file_to_import_path: BTreeMap::new(),
            module_paths: BTreeSet::new(),
            setup_missing_reason: Some(reason.into()),
        }
    }

    pub fn setup_missing_reason(&self) -> Option<&str> {
        self.setup_missing_reason.as_deref()
    }

    pub fn is_setup_missing(&self) -> bool {
        self.setup_missing_reason.is_some()
    }

    pub(crate) fn package(&self, import_path: &str) -> Option<&GoPackageMetadata> {
        self.by_import_path.get(import_path)
    }

    fn local_packages(&self) -> impl Iterator<Item = &GoPackageMetadata> {
        self.by_import_path
            .values()
            .filter(|package| self.is_local_package(package))
    }

    fn is_local_package(&self, package: &GoPackageMetadata) -> bool {
        !package.standard
            && !package.files.is_empty()
            && package.module_path.as_deref().is_some_and(|module_path| {
                self.module_paths.contains(module_path)
                    && import_is_within_module(&package.import_path, module_path)
            })
    }

    fn import_is_external_dependency(&self, import_path: &str) -> bool {
        if !self.module_paths.is_empty() {
            return !self
                .module_paths
                .iter()
                .any(|module_path| import_is_within_module(import_path, module_path));
        }
        is_go_stdlib_import_path(import_path)
    }
}

pub fn seed_go_module_nodes(
    builder: &mut ModuleGraphBuilder,
    metadata: &GoPackageIndex,
) -> GoModuleOwnership {
    let mut ownership = GoModuleOwnership::default();

    for package in metadata.local_packages() {
        let Some(module_path) = &package.module_path else {
            continue;
        };
        let module = builder.ensure_module_node(module_path.clone());
        let package_node =
            builder.ensure_package_node_with_label(package.import_path(), None, Some(Language::Go));
        builder.link_module_contains(module, package_node);
        for file in package.files() {
            let file_node = builder.ensure_file_node(file);
            builder.link_contains(package_node, file_node);
            ownership.file_owner_modules.insert(file, module);
            ownership.package_nodes_by_file.insert(file, package_node);
        }
    }

    ownership
}

pub fn collect_go_topology(
    root: &Path,
    settings: &BTreeMap<String, toml::Value>,
    db: &dyn FactDatabase,
    metadata: &GoPackageIndex,
) -> TopologyOutput {
    let interner = db.stable_key_interner();
    with_module_graph_stable_keys(&interner, || {
        collect_go_topology_inner(root, settings, db, metadata, &interner)
    })
}

fn collect_go_topology_inner(
    root: &Path,
    settings: &BTreeMap<String, toml::Value>,
    db: &dyn FactDatabase,
    metadata: &GoPackageIndex,
    interner: &StableKeyInterner,
) -> TopologyOutput {
    let config = match GoAnalysisConfig::from_settings(root, settings, db) {
        Ok(config) => config,
        Err(_) => {
            return TopologyOutput {
                workspace_roots: vec![repository_root()],
                source_sets: go_files_setup_missing(db, None),
                ..TopologyOutput::default()
            }
            .normalized(interner);
        }
    };

    let mut output = TopologyOutput::default();
    output.workspace_roots.push(repository_root());

    let mut root_ids_by_path = BTreeMap::new();
    for module_root in &config.module_roots {
        let id = WorkspaceRootId(output.workspace_roots.len() as u64);
        let go_mod_path = module_root_manifest_path(module_root, "go.mod");
        let present = repo_file_exists(root, &go_mod_path);
        output.workspace_roots.push(WorkspaceRootFact {
            id,
            kind: WorkspaceRootKind::GoModule,
            root_path: module_root.clone(),
            manifest_path: present.then_some(go_mod_path),
            language: Some(Language::Go),
            stable_key: intern_module_graph_stable_key(format!("go-root:{module_root}")),
            producer_id: GO_TOPOLOGY_PROVIDER_ID,
            precision: TopologyPrecision::ExactStatic,
            status: if present {
                TopologyStatus::Present
            } else {
                TopologyStatus::SetupMissing
            },
        });
        root_ids_by_path.insert(module_root.clone(), id);
    }

    let go_work = root.join("go.work");
    if repo_file_exists(root, "go.work")
        && lifecycle::go_work_covers_module_roots(root, &go_work, &config.module_roots)
    {
        output.workspace_roots.push(WorkspaceRootFact {
            id: WorkspaceRootId(output.workspace_roots.len() as u64),
            kind: WorkspaceRootKind::GoWorkspace,
            root_path: ".".to_string(),
            manifest_path: Some("go.work".to_string()),
            language: Some(Language::Go),
            stable_key: intern_module_graph_stable_key("go-work:."),
            producer_id: GO_TOPOLOGY_PROVIDER_ID,
            precision: TopologyPrecision::ExactStatic,
            status: TopologyStatus::Present,
        });
        if let Ok(contents) =
            read_repo_file_to_string_with_limit(root, "go.work", TOPOLOGY_MANIFEST_MAX_BYTES)
        {
            let _ = parse_go_work("go.work", &contents);
        }
    }

    let mut package_ids_by_import_path = BTreeMap::new();
    let mut go_module_paths = Vec::new();
    let mut module_requirements = BTreeMap::new();
    let mut module_replacements = BTreeMap::new();
    let mut module_package_ids = BTreeMap::new();
    for module_root in &config.module_roots {
        let go_mod_path = module_root_manifest_path(module_root, "go.mod");
        let Ok(contents) =
            read_repo_file_to_string_with_limit(root, &go_mod_path, TOPOLOGY_MANIFEST_MAX_BYTES)
        else {
            continue;
        };
        let manifest = parse_go_mod(&go_mod_path, &contents);
        let Some(module_path) = manifest.module_path.clone() else {
            continue;
        };
        let id = TopologyPackageId(output.packages.len() as u64);
        output.packages.push(TopologyPackageFact {
            id,
            workspace_root: root_ids_by_path.get(module_root).copied(),
            package: None,
            module_node: None,
            kind: TopologyPackageKind::Workspace,
            name: module_path.clone(),
            version: None,
            path: module_root.clone(),
            language: Some(Language::Go),
            stable_key: intern_module_graph_stable_key(format!(
                "go-module:{module_root}:{module_path}"
            )),
            producer_id: GO_TOPOLOGY_PROVIDER_ID,
            precision: TopologyPrecision::ExactStatic,
            status: TopologyStatus::Present,
        });
        package_ids_by_import_path.insert(module_path.clone(), id);
        go_module_paths.push(module_path);
        module_package_ids.insert(module_root.clone(), id);
        emit_go_mod_requirements(
            &mut output,
            id,
            &go_mod_path,
            &manifest.requirements,
            &manifest.replacements,
            &manifest.excludes,
        );
        module_requirements.insert(module_root.clone(), manifest.requirements);
        module_replacements.insert(module_root.clone(), manifest.replacements);
    }

    for package in metadata.local_packages() {
        let id = TopologyPackageId(output.packages.len() as u64);
        let path = go_package_path(db, package);
        output.packages.push(TopologyPackageFact {
            id,
            workspace_root: package
                .module_path
                .as_deref()
                .and_then(|module_path| module_root_for_module_path(module_path, &config, &output))
                .and_then(|module_root| root_ids_by_path.get(module_root).copied()),
            package: None,
            module_node: None,
            kind: TopologyPackageKind::Package,
            name: package.import_path.clone(),
            version: package.module_version.clone(),
            path,
            language: Some(Language::Go),
            stable_key: intern_module_graph_stable_key(format!(
                "go-package:{}",
                package.import_path
            )),
            producer_id: GO_TOPOLOGY_PROVIDER_ID,
            precision: TopologyPrecision::ExactStatic,
            status: TopologyStatus::Present,
        });
        package_ids_by_import_path.insert(package.import_path.clone(), id);
    }

    let files_without_roots = config
        .files_without_module_root
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    for file in lifecycle::go_files(db) {
        let (root, status) = if files_without_roots.contains(&file.relative_path) {
            (None, TopologyStatus::SetupMissing)
        } else {
            (
                module_root_for_file(&file.relative_path, &config.module_roots)
                    .and_then(|root| root_ids_by_path.get(root).copied()),
                TopologyStatus::Present,
            )
        };
        let package = metadata
            .file_to_import_path
            .get(&file.id)
            .and_then(|import_path| package_ids_by_import_path.get(import_path))
            .copied();
        let kind = classify_go_source_set(file);
        output.source_sets.push(SourceSetFact {
            id: SourceSetId(output.source_sets.len() as u64),
            package,
            root,
            kind,
            path: file.relative_path.clone(),
            language: Some(Language::Go),
            files: vec![file.id],
            stable_key: intern_module_graph_stable_key(format!(
                "go-source-set:{}:{}",
                source_set_kind_label(kind),
                file.relative_path
            )),
            producer_id: GO_TOPOLOGY_PROVIDER_ID,
            precision: TopologyPrecision::ExactStatic,
            status,
        });
    }

    emit_go_sum_edges(
        &mut output,
        root,
        &config.module_roots,
        &module_requirements,
        &module_replacements,
        &module_package_ids,
        &go_module_paths,
    );

    output.normalized(interner)
}

fn repository_root() -> WorkspaceRootFact {
    WorkspaceRootFact {
        id: WorkspaceRootId(0),
        kind: WorkspaceRootKind::Repository,
        root_path: ".".to_string(),
        manifest_path: None,
        language: None,
        stable_key: intern_module_graph_stable_key("repository:."),
        producer_id: GO_TOPOLOGY_PROVIDER_ID,
        precision: TopologyPrecision::ExactStatic,
        status: TopologyStatus::Present,
    }
}

fn go_files_setup_missing(
    db: &dyn FactDatabase,
    root: Option<WorkspaceRootId>,
) -> Vec<SourceSetFact> {
    lifecycle::go_files(db)
        .into_iter()
        .enumerate()
        .map(|(index, file)| SourceSetFact {
            id: SourceSetId(index as u64),
            package: None,
            root,
            kind: classify_go_source_set(file),
            path: file.relative_path.clone(),
            language: Some(Language::Go),
            files: vec![file.id],
            stable_key: intern_module_graph_stable_key(format!(
                "go-source-set:{}:{}",
                source_set_kind_label(classify_go_source_set(file)),
                file.relative_path
            )),
            producer_id: GO_TOPOLOGY_PROVIDER_ID,
            precision: TopologyPrecision::Unknown,
            status: TopologyStatus::SetupMissing,
        })
        .collect()
}

fn emit_go_mod_requirements(
    output: &mut TopologyOutput,
    from_package: TopologyPackageId,
    manifest_path: &str,
    requirements: &[formats::go_mod::GoRequirementDirective],
    replacements: &[formats::go_mod::GoReplacementDirective],
    excludes: &[formats::go_mod::GoRequirementDirective],
) {
    for requirement in requirements {
        output
            .dependency_requirements
            .push(DependencyRequirementFact {
                id: DependencyRequirementId(output.dependency_requirements.len() as u64),
                from_package: Some(from_package),
                target_package: None,
                target_name: requirement.target_name.clone(),
                version_requirement: requirement.version_requirement.clone(),
                kind: RequirementKind::Direct,
                manifest_path: Some(manifest_path.to_string()),
                stable_key: intern_module_graph_stable_key(format!(
                    "go-require:{manifest_path}:{}:{}",
                    requirement.target_name,
                    requirement.version_requirement.as_deref().unwrap_or("")
                )),
                producer_id: GO_TOPOLOGY_PROVIDER_ID,
                precision: requirement.precision,
                status: requirement.status,
            });
    }
    for replacement in replacements {
        output
            .dependency_requirements
            .push(DependencyRequirementFact {
                id: DependencyRequirementId(output.dependency_requirements.len() as u64),
                from_package: Some(from_package),
                target_package: None,
                target_name: replacement.target_name.clone(),
                version_requirement: replacement.version_requirement.clone(),
                kind: RequirementKind::Replace,
                manifest_path: Some(manifest_path.to_string()),
                stable_key: intern_module_graph_stable_key(format!(
                    "go-replace:{manifest_path}:{}:{}=>{}:{}",
                    replacement.target_name,
                    replacement.version_requirement.as_deref().unwrap_or(""),
                    replacement.replacement_target,
                    replacement.replacement_version.as_deref().unwrap_or("")
                )),
                producer_id: GO_TOPOLOGY_PROVIDER_ID,
                precision: replacement.precision,
                status: replacement.status,
            });
    }
    for exclude in excludes {
        output
            .dependency_requirements
            .push(DependencyRequirementFact {
                id: DependencyRequirementId(output.dependency_requirements.len() as u64),
                from_package: Some(from_package),
                target_package: None,
                target_name: exclude.target_name.clone(),
                version_requirement: exclude.version_requirement.clone(),
                kind: RequirementKind::Exclude,
                manifest_path: Some(manifest_path.to_string()),
                stable_key: intern_module_graph_stable_key(format!(
                    "go-exclude:{manifest_path}:{}:{}",
                    exclude.target_name,
                    exclude.version_requirement.as_deref().unwrap_or("")
                )),
                producer_id: GO_TOPOLOGY_PROVIDER_ID,
                precision: exclude.precision,
                status: exclude.status,
            });
    }
}

fn emit_go_sum_edges(
    output: &mut TopologyOutput,
    root: &Path,
    module_roots: &[String],
    module_requirements: &BTreeMap<String, Vec<formats::go_mod::GoRequirementDirective>>,
    module_replacements: &BTreeMap<String, Vec<formats::go_mod::GoReplacementDirective>>,
    module_package_ids: &BTreeMap<String, TopologyPackageId>,
    go_module_paths: &[String],
) {
    for module_root in module_roots {
        let go_sum_path = module_root_manifest_path(module_root, "go.sum");
        let source_label = "go.sum";
        let module_package = module_package_ids.get(module_root).copied();
        let requirements = module_requirements
            .get(module_root)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let replacements = module_replacements
            .get(module_root)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if let Ok(contents) =
            read_repo_file_to_string_with_limit(root, &go_sum_path, TOPOLOGY_LOCKFILE_MAX_BYTES)
        {
            for (line_index, line) in contents.lines().enumerate() {
                if let Some((package_name, resolved_version)) = parse_go_sum_line(line) {
                    output.resolved_dependency_edges.push(ResolvedDependencyEdgeFact {
                        id: ResolvedDependencyEdgeId(output.resolved_dependency_edges.len() as u64),
                        requirement: requirement_id_for(
                            output,
                            module_package,
                            &package_name,
                            Some(&resolved_version),
                        ),
                        from_package: module_package,
                        to_package: None,
                        package_name: package_name.clone(),
                        resolved_version: Some(resolved_version.clone()),
                        kind: ResolvedDependencyKind::ChecksumEvidence,
                        stable_key: intern_module_graph_stable_key(format!(
                            "go-sum-line:{go_sum_path}:{line_index}:{package_name}:{resolved_version}:source_label={source_label}"
                        )),
                        producer_id: GO_TOPOLOGY_PROVIDER_ID,
                        precision: TopologyPrecision::ExactLockfile,
                        status: TopologyStatus::Resolved,
                    });
                }
            }
            continue;
        }

        for requirement in requirements.iter().filter(|requirement| {
            is_external_requirement(&requirement.target_name, go_module_paths)
                && !requirement_has_local_replacement(requirement, replacements)
        }) {
            let resolved_version = requirement.version_requirement.clone();
            output
                .resolved_dependency_edges
                .push(ResolvedDependencyEdgeFact {
                    id: ResolvedDependencyEdgeId(output.resolved_dependency_edges.len() as u64),
                    requirement: requirement_id_for(
                        output,
                        module_package,
                        &requirement.target_name,
                        resolved_version.as_deref(),
                    ),
                    from_package: module_package,
                    to_package: None,
                    package_name: requirement.target_name.clone(),
                    resolved_version,
                    kind: ResolvedDependencyKind::ChecksumEvidence,
                    stable_key: intern_module_graph_stable_key(format!(
                        "go.sum:absent:{go_sum_path}:{}:{}:source_label={source_label}",
                        requirement.target_name,
                        requirement.version_requirement.as_deref().unwrap_or("")
                    )),
                    producer_id: GO_TOPOLOGY_PROVIDER_ID,
                    precision: TopologyPrecision::Unknown,
                    status: TopologyStatus::MissingLockfile,
                });
        }
    }
}

fn requirement_has_local_replacement(
    requirement: &formats::go_mod::GoRequirementDirective,
    replacements: &[formats::go_mod::GoReplacementDirective],
) -> bool {
    replacements.iter().any(|replacement| {
        replacement.target_name == requirement.target_name
            && (replacement.version_requirement.is_none()
                || replacement.version_requirement == requirement.version_requirement)
            && replacement.replacement_version.is_none()
            && is_local_go_replacement_target(&replacement.replacement_target)
    })
}

fn is_local_go_replacement_target(target: &str) -> bool {
    matches!(target, "." | "..")
        || target.starts_with("./")
        || target.starts_with("../")
        || Path::new(target).is_absolute()
}

fn parse_go_sum_line(line: &str) -> Option<(String, String)> {
    let mut parts = line.split_whitespace();
    let package_name = parts.next()?.to_string();
    let version = parts.next()?.trim_end_matches("/go.mod").to_string();
    let _checksum = parts.next()?;
    Some((package_name, version))
}

fn requirement_id_for(
    output: &TopologyOutput,
    from_package: Option<TopologyPackageId>,
    target_name: &str,
    version_requirement: Option<&str>,
) -> Option<DependencyRequirementId> {
    output
        .dependency_requirements
        .iter()
        .find(|requirement| {
            requirement.from_package == from_package
                && requirement.target_name == target_name
                && requirement.version_requirement.as_deref() == version_requirement
        })
        .map(|requirement| requirement.id)
}

fn is_external_requirement(target_name: &str, go_module_paths: &[String]) -> bool {
    !go_module_paths
        .iter()
        .any(|module_path| import_is_within_module(target_name, module_path))
}

fn module_root_manifest_path(module_root: &str, manifest_name: &str) -> String {
    if module_root == "." {
        manifest_name.to_string()
    } else {
        format!("{module_root}/{manifest_name}")
    }
}

fn module_root_for_file<'a>(relative_path: &str, module_roots: &'a [String]) -> Option<&'a str> {
    module_roots
        .iter()
        .filter(|module_root| file_is_under_module_root(relative_path, module_root))
        .max_by_key(|module_root| module_root.len())
        .map(String::as_str)
}

fn file_is_under_module_root(relative_path: &str, module_root: &str) -> bool {
    module_root == "."
        || relative_path == module_root
        || relative_path
            .strip_prefix(module_root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn module_root_for_module_path<'a>(
    module_path: &str,
    config: &'a GoAnalysisConfig,
    output: &TopologyOutput,
) -> Option<&'a str> {
    output
        .packages
        .iter()
        .find(|package| {
            package.kind == TopologyPackageKind::Workspace && package.name == module_path
        })
        .and_then(|package| {
            config
                .module_roots
                .iter()
                .find(|module_root| package.path == module_root.as_str())
        })
        .map(String::as_str)
}

fn go_package_path(db: &dyn FactDatabase, package: &GoPackageMetadata) -> String {
    package
        .files
        .iter()
        .filter_map(|file| db.files().iter().find(|source| source.id == *file))
        .filter_map(|file| parent_path(&file.relative_path))
        .next()
        .unwrap_or_else(|| ".".to_string())
}

fn parent_path(relative_path: &str) -> Option<String> {
    relative_path
        .rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .or_else(|| Some(".".to_string()))
}

fn classify_go_source_set(file: &SourceFile) -> SourceSetKind {
    if path_contains_vendor(&file.relative_path) {
        return SourceSetKind::Vendor;
    }
    if file.relative_path.ends_with("_test.go") {
        return SourceSetKind::Test;
    }
    if file.relative_path.ends_with(".pb.go") || first_comment_is_generated(&file.source) {
        return SourceSetKind::Generated;
    }
    SourceSetKind::Source
}

fn path_contains_vendor(relative_path: &str) -> bool {
    relative_path == "vendor"
        || relative_path.starts_with("vendor/")
        || relative_path.contains("/vendor/")
}

fn first_comment_is_generated(source: &str) -> bool {
    source
        .lines()
        .map(str::trim_start)
        .find(|line| !line.is_empty())
        .is_some_and(|line| {
            line.starts_with("// Code generated") || line.starts_with("/* Code generated")
        })
}

fn source_set_kind_label(kind: SourceSetKind) -> &'static str {
    match kind {
        SourceSetKind::Source => "source",
        SourceSetKind::Test => "test",
        SourceSetKind::Generated => "generated",
        SourceSetKind::Vendor => "vendor",
        SourceSetKind::External => "external",
        SourceSetKind::Unknown => "unknown",
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GoPackageMetadata {
    import_path: String,
    #[allow(dead_code)]
    name: Option<String>,
    files: Vec<FileId>,
    standard: bool,
    module_path: Option<String>,
    #[allow(dead_code)]
    module_version: Option<String>,
}

impl GoPackageMetadata {
    fn from_go_list_package(
        root: &Path,
        file_ids: &BTreeMap<String, FileId>,
        package: GoListPackage,
    ) -> Self {
        let files = mapped_package_files(root, file_ids, &package)
            .into_iter()
            .collect();
        let module_path = package
            .module
            .as_ref()
            .and_then(|module| module.path.clone());
        let module_version = package
            .module
            .as_ref()
            .and_then(|module| module.version.clone());
        Self {
            import_path: package.import_path,
            name: package.name,
            files,
            standard: package.standard,
            module_path,
            module_version,
        }
    }

    pub(crate) fn import_path(&self) -> &str {
        &self.import_path
    }

    pub(crate) fn files(&self) -> impl Iterator<Item = FileId> + '_ {
        self.files.iter().copied()
    }
}

#[derive(Debug, Deserialize)]
struct GoListPackage {
    #[serde(rename = "ImportPath", default)]
    import_path: String,
    #[serde(rename = "Name", default)]
    name: Option<String>,
    #[serde(rename = "Dir", default)]
    dir: Option<PathBuf>,
    #[serde(rename = "GoFiles", default)]
    go_files: Vec<PathBuf>,
    #[serde(rename = "TestGoFiles", default)]
    test_go_files: Vec<PathBuf>,
    #[serde(rename = "CompiledGoFiles", default)]
    compiled_go_files: Vec<PathBuf>,
    #[serde(rename = "Standard", default)]
    standard: bool,
    #[serde(rename = "Module", default)]
    module: Option<GoListModule>,
}

#[derive(Debug, Deserialize)]
struct GoListModule {
    #[serde(rename = "Path", default)]
    path: Option<String>,
    #[serde(rename = "Version", default)]
    version: Option<String>,
}

pub(crate) struct GoCommandOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl From<std::process::Output> for GoCommandOutput {
    fn from(output: std::process::Output) -> Self {
        Self {
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }
}

fn run_go_list(root: &Path, config: &GoAnalysisConfig) -> GoCommandOutput {
    let (mut command, _workspace) = match lifecycle::command_with_go_env(root, &config.module_roots)
    {
        Ok(command) => command,
        Err(error) => {
            return GoCommandOutput {
                status: failed_exit_status(),
                stdout: Vec::new(),
                stderr: error.reason().as_bytes().to_vec(),
            };
        }
    };
    command.args(["list", "-mod=readonly", "-json"]);
    if !config.build_tags.is_empty() {
        command.arg(format!("-tags={}", config.build_tags.join(",")));
    }
    lifecycle::apply_go_offline_env(&mut command, config.offline);
    command.args(config.rooted_package_patterns());
    run_go_list_command(command, GO_SUBPROCESS_TIMEOUT)
}

fn run_go_list_command(command: Command, timeout: Duration) -> GoCommandOutput {
    run_bounded(command, timeout, "go list")
        .map(|output| GoCommandOutput {
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        })
        .unwrap_or_else(|error| GoCommandOutput {
            status: failed_exit_status(),
            stdout: Vec::new(),
            stderr: error.reason().as_bytes().to_vec(),
        })
}

fn mapped_package_files(
    root: &Path,
    file_ids: &BTreeMap<String, FileId>,
    package: &GoListPackage,
) -> BTreeSet<FileId> {
    let Some(dir) = &package.dir else {
        return BTreeSet::new();
    };
    package
        .go_files
        .iter()
        .chain(package.test_go_files.iter())
        .chain(package.compiled_go_files.iter())
        .filter_map(|entry| map_package_file(root, dir, entry, file_ids))
        .collect()
}

fn map_package_file(
    root: &Path,
    dir: &Path,
    entry: &Path,
    file_ids: &BTreeMap<String, FileId>,
) -> Option<FileId> {
    let absolute_dir = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        root.join(dir)
    };
    let absolute_path = if entry.is_absolute() {
        entry.to_path_buf()
    } else {
        absolute_dir.join(entry)
    };
    let relative_path = paths::normalize_repo_relative_path(root, &absolute_path)?;
    file_ids.get(&relative_path).copied()
}

fn go_list_failure_reason(output: &GoCommandOutput) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        format!("go list -json ./... failed: {}", output.status)
    } else {
        format!("go list -json ./... failed: {stderr}")
    }
}

fn failed_exit_status() -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(1 << 8)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(1)
    }
}

pub fn resolve_go_import(
    input: ResolverInput<'_>,
    metadata: &GoPackageIndex,
) -> ResolvedImportDraft {
    let _ = (input.root, input.db.files().len(), input.owner_module);
    if input.import.language != Language::Go {
        return ResolvedImportDraft::unsupported_language();
    }
    if metadata.is_setup_missing() {
        return ResolvedImportDraft::setup_missing();
    }

    if let Some(package) = metadata.package(&input.import.path) {
        if metadata.is_local_package(package) {
            return ResolvedImportDraft {
                target: Some(ModuleNodeDraft::package(
                    package.import_path(),
                    None,
                    Some(Language::Go),
                )),
                status: ResolutionStatus::Resolved,
                precision: ResolutionPrecision::Package,
                reason: None,
                edge_kind: Some(ModuleEdgeKind::DependsOn),
                alternatives: Vec::new(),
                resolution_issue: None,
            };
        }
        return external_draft(input.import.path.clone());
    }

    if metadata.import_is_external_dependency(&input.import.path) {
        return external_draft(input.import.path.clone());
    }

    ResolvedImportDraft::unresolved(UnresolvedReason::NotFound)
}

fn external_draft(label: String) -> ResolvedImportDraft {
    ResolvedImportDraft {
        target: Some(ModuleNodeDraft::external(label, Some(Language::Go))),
        status: ResolutionStatus::External,
        precision: ResolutionPrecision::ExternalPackage,
        reason: None,
        edge_kind: Some(ModuleEdgeKind::DependsOn),
        alternatives: Vec::new(),
        resolution_issue: None,
    }
}

fn import_is_within_module(import_path: &str, module_path: &str) -> bool {
    import_path == module_path
        || import_path
            .strip_prefix(module_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn is_go_stdlib_import_path(import_path: &str) -> bool {
    import_path
        .split('/')
        .next()
        .is_some_and(|first| !first.contains('.') && !first.is_empty())
}

#[cfg(test)]
mod subprocess_timeout_tests {
    use super::*;

    #[test]
    fn go_list_command_times_out_instead_of_hanging() {
        let command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/C", "ping -n 3 127.0.0.1 > nul"]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 2"]);
            command
        };
        let output = run_go_list_command(command, Duration::from_millis(1));
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("timeout"));
    }
}
