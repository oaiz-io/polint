#[cfg(feature = "lang-typescript")]
mod bounded_fs;
#[cfg(feature = "lang-typescript")]
mod conditions;
#[cfg(all(test, feature = "lang-typescript"))]
mod conditions_differential_tests;
mod formats;
use crate::analysis_api::{
    FactDatabase, ImportSyntax, ModuleEdgeKind, ResolutionPrecision, ResolutionStatus, SourceFile,
    UnresolvedReason,
};
use crate::analysis_neutral::module_graph::model::{
    ModuleGraphBuilder, ModuleNodeDraft, ResolvedImportDraft, ResolverInput,
};
use crate::analysis_neutral::module_graph::topology::{
    DependencyRequirementFact, DependencyRequirementId, RepoTopologyOverlayFact,
    RepoTopologyOverlayId, RepoTopologyOverlayKind, RequirementKind, ResolvedDependencyEdgeFact,
    ResolvedDependencyEdgeId, ResolvedDependencyKind, SourceSetFact, SourceSetId, SourceSetKind,
    TopologyOutput, TopologyPackageFact, TopologyPackageId, TopologyPackageKind, TopologyPrecision,
    TopologyStatus, WorkspaceRootFact, WorkspaceRootId, WorkspaceRootKind,
};
#[cfg(feature = "lang-typescript")]
use crate::bounded_json;
use crate::internal_core::{FileId, Language, ModuleNodeId, StableKeyInterner};
#[cfg(feature = "lang-typescript")]
use crate::ts::DYNAMIC_IMPORT_SPECIFIER;
use crate::ts::repo_fs::{
    RepoFileReadError, TOPOLOGY_LOCKFILE_MAX_BYTES, TOPOLOGY_MANIFEST_MAX_BYTES, normalize_path,
    normalize_repo_relative, normalize_repo_relative_input, normalize_repo_relative_path,
    read_repo_file_to_string_with_limit, repo_dir_path, repo_file_exists, repo_file_path,
    repo_relative_existing_path,
};
use crate::ts::tsconfig_chain::{
    TsconfigExtendsWire, extended_configs, parse_tsconfig_json, read_tsconfig_json,
    resolve_tsconfig_file_candidate, walk_tsconfig_chain,
};
#[cfg(feature = "lang-typescript")]
use bounded_fs::{BoundedFs, RefusedInput};
use formats::js_lockfile::{
    JsLockfileKind, JsLockfileManifest, JsLockfilePackage, JsPackageManager, parse_js_lockfile,
    unsupported_js_lockfile,
};
use formats::package_json::{PackageJsonManifest, parse_package_json, unsupported_package_json};
pub use formats::pnpm_workspace::parse_pnpm_workspace_packages;
#[cfg(feature = "lang-typescript")]
use oxc_resolver::{
    FileSystem as _, ResolveError, ResolveOptions, ResolverGeneric, TsconfigDiscovery,
};
#[cfg(feature = "lang-typescript")]
type Resolver = ResolverGeneric<BoundedFs>;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(feature = "lang-typescript")]
use std::sync::{Arc, Mutex, PoisonError};

#[cfg(test)]
use std::cell::Cell;
use std::cell::RefCell;

thread_local! {
    static MODULE_GRAPH_STABLE_KEYS: RefCell<Option<StableKeyInterner>> = const { RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    static RESOLVER_CONTEXT_CONSTRUCTIONS: Cell<usize> = const { Cell::new(0) };
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

#[cfg(not(feature = "lang-typescript"))]
pub struct TsResolverContext;

#[cfg(not(feature = "lang-typescript"))]
impl TsResolverContext {
    pub fn new(_root: &Path, _db: &dyn FactDatabase, _owner_module: Option<ModuleNodeId>) -> Self {
        Self
    }
}

#[cfg(not(feature = "lang-typescript"))]
pub fn resolve_ts_import(
    input: ResolverInput<'_>,
    _context: Option<&TsResolverContext>,
) -> ResolvedImportDraft {
    let _ = input;
    ResolvedImportDraft::unsupported_language()
}

#[cfg(feature = "lang-typescript")]
pub struct TsResolverContext {
    resolver: ModeResolvers,
    root: PathBuf,
    file_by_absolute_normalized_path: BTreeMap<PathBuf, FileId>,
    path_aliases_by_config_dir: BTreeMap<PathBuf, Vec<String>>,
    pub(crate) owner_module: Option<ModuleNodeId>,
}

#[cfg(feature = "lang-typescript")]
impl TsResolverContext {
    pub fn new(root: &Path, db: &dyn FactDatabase, owner_module: Option<ModuleNodeId>) -> Self {
        #[cfg(test)]
        RESOLVER_CONTEXT_CONSTRUCTIONS.with(|count| count.set(count.get() + 1));

        let root = normalize_path(root).unwrap_or_else(|| root.to_path_buf());
        let file_by_absolute_normalized_path = db
            .files()
            .iter()
            .filter_map(|file| {
                let absolute = if file.path.is_absolute() {
                    file.path.clone()
                } else {
                    root.join(&file.relative_path)
                };
                normalize_path(&absolute).map(|path| (path, file.id))
            })
            .collect();

        Self {
            resolver: ModeResolvers::new(),
            path_aliases_by_config_dir: collect_ts_path_aliases(&root, db),
            root,
            file_by_absolute_normalized_path,
            owner_module,
        }
    }
}

#[cfg(feature = "lang-typescript")]
pub fn resolve_ts_import(
    input: ResolverInput<'_>,
    context: Option<&TsResolverContext>,
) -> ResolvedImportDraft {
    let _ = (input.root, input.owner_module, input.owner_package);
    if !input.import.language.is_ts_family() {
        return ResolvedImportDraft::unsupported_language();
    }
    if input.import.path == DYNAMIC_IMPORT_SPECIFIER {
        return ResolvedImportDraft {
            target: None,
            status: ResolutionStatus::Dynamic,
            precision: ResolutionPrecision::None,
            reason: Some(UnresolvedReason::DynamicExpression),
            edge_kind: None,
            alternatives: Vec::new(),
            resolution_issue: None,
        };
    }

    let Some(context) = context else {
        return ResolvedImportDraft::setup_missing();
    };
    let _owner_module = input.owner_module.or(context.owner_module);
    let Some(importer) = input.db.file(input.import.file) else {
        return ResolvedImportDraft::unresolved(UnresolvedReason::NotFound);
    };
    let importer_path = if importer.path.is_absolute() {
        importer.path.clone()
    } else {
        context.root.join(&importer.relative_path)
    };
    let Some(importer_path) = normalize_path(&importer_path) else {
        return ResolvedImportDraft::unresolved(UnresolvedReason::NotFound);
    };

    let (resolution, refusal) = context.resolver.resolve(
        &importer_path,
        input.import.path.as_str(),
        input.import.syntax,
    );
    let open = |paths: Vec<PathBuf>, reason, resolution_issue| ResolvedImportDraft {
        alternatives: paths
            .iter()
            .filter_map(|path| {
                let file = context
                    .file_by_absolute_normalized_path
                    .get(&normalize_path(path)?)
                    .copied()?;
                Some(ModuleNodeDraft::file(
                    file,
                    input.db.path_for(file),
                    input.import.language,
                ))
            })
            .collect(),
        resolution_issue,
        ..ResolvedImportDraft::unresolved(reason)
    };
    if let Some(refusal) = refusal {
        let file = refusal
            .path
            .strip_prefix(&context.root)
            .unwrap_or(&refusal.path);
        let details = Some(format!("{}: {}", file.display(), refusal.reason));
        if refusal.malformed_config {
            return ResolvedImportDraft {
                resolution_issue: details,
                ..ResolvedImportDraft::setup_missing()
            };
        }
        return open(
            resolution.into_paths(),
            UnresolvedReason::ResolverError,
            details,
        );
    }
    let resolution = match resolution {
        ModeResolution::One(resolution) => resolution,
        // No one file is the target; each analysed file a way of loading
        // the import names is a candidate.
        ModeResolution::Split(paths) => return open(paths, UnresolvedReason::ModeDependent, None),
    };
    match resolution {
        Ok(resolution) => resolved_path_draft(context, input, resolution.path()),
        Err(ResolveError::Builtin { resolved, .. }) => {
            external_draft(resolved, input.import.language)
        }
        Err(ResolveError::MatchedAliasNotFound(_, _)) => {
            ResolvedImportDraft::unresolved(UnresolvedReason::NotFound)
        }
        Err(ResolveError::NotFound(_)) => {
            if tsconfig_path_alias_matches(context, &importer_path, &input.import.path) {
                ResolvedImportDraft::unresolved(UnresolvedReason::NotFound)
            } else if is_external_package_specifier(&input.import.path) {
                external_draft(input.import.path.clone(), input.import.language)
            } else {
                ResolvedImportDraft::unresolved(UnresolvedReason::NotFound)
            }
        }
        Err(
            ResolveError::TsconfigNotFound(_)
            | ResolveError::TsconfigSelfReference(_)
            | ResolveError::TsconfigCircularExtend(_)
            | ResolveError::Json(_)
            | ResolveError::IOError(_),
        ) => ResolvedImportDraft::setup_missing(),
        Err(_) => ResolvedImportDraft::unresolved(UnresolvedReason::ResolverError),
    }
}

#[cfg(feature = "lang-typescript")]
fn resolved_path_draft(
    context: &TsResolverContext,
    input: ResolverInput<'_>,
    resolved_path: &Path,
) -> ResolvedImportDraft {
    let Some(normalized_path) = normalize_path(resolved_path) else {
        return ResolvedImportDraft::unresolved(UnresolvedReason::NotFound);
    };
    if let Some(file) = context
        .file_by_absolute_normalized_path
        .get(&normalized_path)
        .copied()
    {
        return ResolvedImportDraft {
            target: Some(ModuleNodeDraft::file(
                file,
                input.db.path_for(file),
                input.import.language,
            )),
            status: ResolutionStatus::Resolved,
            precision: ResolutionPrecision::ExactFile,
            reason: None,
            edge_kind: Some(ModuleEdgeKind::Imports),
            alternatives: Vec::new(),
            resolution_issue: None,
        };
    }

    if !normalized_path.starts_with(&context.root)
        || is_external_package_specifier(&input.import.path)
    {
        external_draft(input.import.path.clone(), input.import.language)
    } else {
        ResolvedImportDraft::unresolved(UnresolvedReason::NotFound)
    }
}

fn external_draft(label: String, language: Language) -> ResolvedImportDraft {
    ResolvedImportDraft {
        target: Some(ModuleNodeDraft::external(label, Some(language))),
        status: ResolutionStatus::External,
        precision: ResolutionPrecision::ExternalPackage,
        reason: None,
        edge_kind: Some(ModuleEdgeKind::DependsOn),
        alternatives: Vec::new(),
        resolution_issue: None,
    }
}

pub fn seed_ts_project_module_nodes(
    builder: &mut ModuleGraphBuilder,
    root: &Path,
    files: &[&SourceFile],
) -> BTreeMap<FileId, ModuleNodeId> {
    let mut module_by_root = BTreeMap::<PathBuf, ModuleNodeId>::new();
    let mut owner_by_file = BTreeMap::new();

    for file in files.iter().filter(|file| file.language.is_ts_family()) {
        let absolute_file = if file.path.is_absolute() {
            file.path.clone()
        } else {
            root.join(&file.relative_path)
        };
        let Some(module_root) = find_ts_project_root(root, &absolute_file) else {
            continue;
        };
        let module = if let Some(module) = module_by_root.get(&module_root).copied() {
            module
        } else {
            let label = ts_project_module_label(root, &module_root);
            let module = builder.ensure_module_node(label);
            module_by_root.insert(module_root, module);
            module
        };
        owner_by_file.insert(file.id, module);
    }

    owner_by_file
}

fn find_ts_project_root(root: &Path, file_path: &Path) -> Option<PathBuf> {
    let root = normalize_path(root)?;
    let mut current = normalize_path(file_path.parent()?)?;

    loop {
        if current.join("tsconfig.json").is_file() || current.join("package.json").is_file() {
            return Some(current);
        }
        if current == root || !current.starts_with(&root) || !current.pop() {
            return None;
        }
    }
}

fn ts_project_module_label(root: &Path, module_root: &Path) -> String {
    package_json_name(root, module_root).unwrap_or_else(|| {
        normalize_repo_relative_path(root, module_root).unwrap_or_else(|| ".".to_string())
    })
}

fn package_json_name(root: &Path, module_root: &Path) -> Option<String> {
    let relative_path = normalize_repo_relative_path(root, &module_root.join("package.json"))?;
    let source =
        read_repo_file_to_string_with_limit(root, relative_path, TOPOLOGY_MANIFEST_MAX_BYTES)
            .ok()?;
    let value =
        crate::bounded_json::from_str::<serde_json::Value>(&source, TOPOLOGY_MANIFEST_MAX_BYTES)
            .ok()?;
    value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

pub fn collect_ts_topology(root: &Path, db: &dyn FactDatabase) -> TopologyOutput {
    let interner = db.stable_key_interner();
    with_module_graph_stable_keys(&interner, || collect_ts_topology_inner(root, db, &interner))
}

fn collect_ts_topology_inner(
    root: &Path,
    db: &dyn FactDatabase,
    interner: &StableKeyInterner,
) -> TopologyOutput {
    let mut output = TopologyOutput::default();
    let ts_files = db
        .files()
        .iter()
        .filter(|file| file.language.is_ts_family())
        .collect::<Vec<_>>();
    let package_manifests = collect_package_manifests(root, &ts_files);
    let workspace_members_by_root = js_workspace_members_by_root(root, &package_manifests);
    let workspace_roots = js_workspace_roots(root, &package_manifests, &workspace_members_by_root);

    let mut root_ids_by_path = BTreeMap::new();
    for package_path in &workspace_roots {
        let id = WorkspaceRootId(output.workspace_roots.len() as u64);
        output.workspace_roots.push(WorkspaceRootFact {
            id,
            kind: WorkspaceRootKind::JsWorkspace,
            root_path: package_path.clone(),
            manifest_path: Some(package_manifest_path(package_path)),
            language: Some(Language::TypeScript),
            stable_key: intern_module_graph_stable_key(format!("js-workspace:{package_path}")),
            producer_id: TS_TOPOLOGY_PROVIDER_ID,
            precision: TopologyPrecision::ExactStatic,
            status: TopologyStatus::Present,
        });
        root_ids_by_path.insert(package_path.clone(), id);
    }

    let mut package_ids_by_path = BTreeMap::new();
    for (package_path, manifest) in &package_manifests {
        let (precision, status) = package_manifest_topology_state(manifest);
        let id = TopologyPackageId(output.packages.len() as u64);
        let workspace_root =
            workspace_root_for_package(package_path, &workspace_roots, &workspace_members_by_root)
                .and_then(|root| root_ids_by_path.get(root).copied());
        output.packages.push(TopologyPackageFact {
            id,
            workspace_root,
            package: None,
            module_node: None,
            kind: TopologyPackageKind::JsPackage,
            name: manifest
                .name
                .clone()
                .unwrap_or_else(|| package_path.clone()),
            version: manifest.version.clone(),
            path: package_path.clone(),
            language: Some(Language::TypeScript),
            stable_key: intern_module_graph_stable_key(format!(
                "js-package:{package_path}:{}",
                manifest.name.as_deref().unwrap_or("")
            )),
            producer_id: TS_TOPOLOGY_PROVIDER_ID,
            precision,
            status,
        });
        package_ids_by_path.insert(package_path.clone(), id);
    }

    let lockfile_selections = select_js_lockfiles(
        root,
        &package_manifests,
        &workspace_roots,
        &workspace_members_by_root,
    );

    for (package_path, manifest) in &package_manifests {
        emit_package_manager_overlays(&mut output, package_path, manifest);
        emit_package_manifest_unsupported_overlays(&mut output, package_path, manifest);
        emit_lockfile_overlays(&mut output, root, package_path);
        if let Some(selection) = lockfile_selections.get(package_path) {
            emit_lockfile_selection_overlay(&mut output, package_path, selection);
        }
        if let Some(package_id) = package_ids_by_path.get(package_path).copied() {
            emit_package_requirements(&mut output, package_id, package_path, manifest);
        }
    }
    for (package_path, package_id) in &package_ids_by_path {
        if let Some(selection) = lockfile_selections.get(package_path) {
            emit_js_lockfile_edges(&mut output, root, package_path, *package_id, selection);
        }
    }
    emit_pnpm_workspace_overlays(&mut output, root);
    emit_tsconfig_overlays(&mut output, root, &ts_files);

    for file in ts_files {
        let package_path = nearest_package_root_for_relative_path(root, &file.relative_path);
        let package = package_path
            .as_ref()
            .and_then(|path| package_ids_by_path.get(path).copied());
        let root = package_path
            .as_ref()
            .and_then(|path| {
                workspace_root_for_package(path, &workspace_roots, &workspace_members_by_root)
            })
            .and_then(|path| root_ids_by_path.get(path).copied());
        let kind = classify_ts_source_set(file);
        output.source_sets.push(SourceSetFact {
            id: SourceSetId(output.source_sets.len() as u64),
            package,
            root,
            kind,
            path: file.relative_path.clone(),
            language: Some(file.language),
            files: vec![file.id],
            stable_key: intern_module_graph_stable_key(format!(
                "ts-source-set:{}:{}",
                source_set_kind_label(kind),
                file.relative_path
            )),
            producer_id: TS_TOPOLOGY_PROVIDER_ID,
            precision: TopologyPrecision::ExactStatic,
            status: TopologyStatus::Present,
        });
    }

    output.normalized(interner)
}

const TS_TOPOLOGY_PROVIDER_ID: &str = "polint.module_graph";

#[derive(Debug, Clone, PartialEq, Eq)]
struct JsLockfileSelection {
    manager: Option<JsPackageManager>,
    root_path: String,
    lockfile: Option<DetectedJsLockfile>,
    status: JsLockfileSelectionStatus,
    reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DetectedJsLockfile {
    kind: JsLockfileKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsLockfileSelectionStatus {
    Selected,
    MissingLockfile,
    Ambiguous,
    Unsupported,
}

fn collect_package_manifests(
    root: &Path,
    ts_files: &[&SourceFile],
) -> BTreeMap<String, PackageJsonManifest> {
    let mut package_paths = BTreeSet::new();
    if repo_file_exists(root, "package.json") {
        package_paths.insert(".".to_string());
    }
    for file in ts_files {
        package_paths.extend(package_roots_for_relative_path(root, &file.relative_path));
    }

    let mut manifests = BTreeMap::new();
    for package_path in package_paths {
        if let Some(manifest) = read_package_manifest(root, &package_path) {
            for workspace in &manifest.workspaces {
                for member in expand_workspace_glob(root, &package_path, workspace) {
                    if let Some(member_manifest) = read_package_manifest(root, &member) {
                        manifests.insert(member, member_manifest);
                    }
                }
            }
            manifests.insert(package_path, manifest);
        }
    }
    for workspace in root_pnpm_workspace_patterns(root) {
        for member in expand_workspace_glob(root, ".", &workspace) {
            if let Some(member_manifest) = read_package_manifest(root, &member) {
                manifests.insert(member, member_manifest);
            }
        }
    }
    manifests
}

fn js_workspace_roots(
    root: &Path,
    package_manifests: &BTreeMap<String, PackageJsonManifest>,
    members_by_root: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<String> {
    let mut roots = package_manifests
        .iter()
        .filter(|(_, manifest)| !manifest.workspaces.is_empty())
        .map(|(path, _)| path.clone())
        .collect::<BTreeSet<_>>();
    roots.extend(members_by_root.keys().cloned());
    if package_manifests.contains_key(".") && !root_pnpm_workspace_patterns(root).is_empty() {
        roots.insert(".".to_string());
    }
    roots
}

fn js_workspace_members_by_root(
    root: &Path,
    package_manifests: &BTreeMap<String, PackageJsonManifest>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut members_by_root = BTreeMap::<String, BTreeSet<String>>::new();
    for (package_path, manifest) in package_manifests {
        for workspace in &manifest.workspaces {
            let members = expand_workspace_glob(root, package_path, workspace);
            members_by_root
                .entry(package_path.clone())
                .or_default()
                .extend(members);
        }
    }
    for workspace in root_pnpm_workspace_patterns(root) {
        let members = expand_workspace_glob(root, ".", &workspace);
        members_by_root
            .entry(".".to_string())
            .or_default()
            .extend(members);
    }
    members_by_root
}

fn package_roots_for_relative_path(root: &Path, relative_path: &str) -> Vec<String> {
    let Some(mut path) = Path::new(relative_path).parent().map(Path::to_path_buf) else {
        return Vec::new();
    };
    let mut package_roots = Vec::new();
    loop {
        if let Some(package_path) = normalize_repo_relative(path.to_string_lossy()) {
            let manifest_path = package_manifest_path(&package_path);
            if repo_file_exists(root, &manifest_path) {
                package_roots.push(package_path);
            }
        }
        if !path.pop() {
            break;
        }
    }
    package_roots
}

fn read_package_manifest(root: &Path, package_path: &str) -> Option<PackageJsonManifest> {
    let manifest_path = package_manifest_path(package_path);
    match read_repo_file_to_string_with_limit(root, &manifest_path, TOPOLOGY_MANIFEST_MAX_BYTES) {
        Ok(contents) => Some(parse_package_json(&manifest_path, &contents)),
        Err(error) if error.is_not_found() => None,
        Err(error) => Some(unsupported_package_json(
            &manifest_path,
            error.stable_reason(),
        )),
    }
}

fn package_manifest_topology_state(
    manifest: &PackageJsonManifest,
) -> (TopologyPrecision, TopologyStatus) {
    if manifest.unsupported.is_empty() {
        (TopologyPrecision::ExactStatic, TopologyStatus::Present)
    } else {
        (TopologyPrecision::Unknown, TopologyStatus::Unsupported)
    }
}

fn package_manifest_path(package_path: &str) -> String {
    if package_path == "." {
        "package.json".to_string()
    } else {
        format!("{package_path}/package.json")
    }
}

fn nearest_package_root_for_relative_path(root: &Path, relative_path: &str) -> Option<String> {
    let mut path = Path::new(relative_path).parent()?.to_path_buf();
    loop {
        let package_path = normalize_repo_relative(path.to_string_lossy())?;
        let manifest_path = package_manifest_path(&package_path);
        if repo_file_exists(root, &manifest_path) {
            return Some(package_path);
        }
        if !path.pop() {
            return None;
        }
    }
}

fn expand_workspace_glob(root: &Path, package_path: &str, pattern: &str) -> Vec<String> {
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
        .filter(|path| repo_file_exists(root, package_manifest_path(path)))
        .collect::<Vec<_>>();
    members.sort();
    members
}

fn workspace_root_for_package<'a>(
    package_path: &str,
    workspace_roots: &'a BTreeSet<String>,
    members_by_root: &BTreeMap<String, BTreeSet<String>>,
) -> Option<&'a String> {
    workspace_roots
        .iter()
        .filter(|root| {
            package_path == root.as_str()
                || members_by_root
                    .get(root.as_str())
                    .is_some_and(|members| members.contains(package_path))
        })
        .max_by_key(|root| root.len())
}

fn select_js_lockfiles(
    root: &Path,
    package_manifests: &BTreeMap<String, PackageJsonManifest>,
    workspace_roots: &BTreeSet<String>,
    members_by_root: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<String, JsLockfileSelection> {
    package_manifests
        .iter()
        .map(|(package_path, manifest)| {
            let root_path = lockfile_selection_root(
                root,
                package_path,
                manifest,
                package_manifests,
                workspace_roots,
                members_by_root,
            );
            let root_manifest = package_manifests.get(&root_path).unwrap_or(manifest);
            let selection = select_js_lockfile_at_root(root, &root_path, root_manifest);
            (package_path.clone(), selection)
        })
        .collect()
}

fn lockfile_selection_root(
    root: &Path,
    package_path: &str,
    manifest: &PackageJsonManifest,
    package_manifests: &BTreeMap<String, PackageJsonManifest>,
    workspace_roots: &BTreeSet<String>,
    members_by_root: &BTreeMap<String, BTreeSet<String>>,
) -> String {
    if manifest.package_manager.is_some() {
        return package_path.to_string();
    }
    let Some(workspace_root) =
        workspace_root_for_package(package_path, workspace_roots, members_by_root)
    else {
        return package_path.to_string();
    };
    if workspace_root == package_path {
        return package_path.to_string();
    }
    let Some(root_manifest) = package_manifests.get(workspace_root) else {
        return package_path.to_string();
    };
    if root_manifest.package_manager.is_some() {
        return workspace_root.clone();
    }
    if !detect_js_lockfiles(root, package_path).is_empty() {
        return package_path.to_string();
    }
    if !detect_js_lockfiles(root, workspace_root).is_empty() {
        workspace_root.clone()
    } else {
        package_path.to_string()
    }
}

fn select_js_lockfile_at_root(
    root: &Path,
    root_path: &str,
    manifest: &PackageJsonManifest,
) -> JsLockfileSelection {
    let lockfiles = detect_js_lockfiles(root, root_path);
    if let Some(package_manager) = manifest.package_manager.as_deref() {
        return match parse_package_manager(package_manager) {
            Ok(manager) => match select_lockfile_for_manager(manager, &lockfiles) {
                Some(lockfile) => JsLockfileSelection {
                    manager: Some(manager),
                    root_path: root_path.to_string(),
                    lockfile: Some(lockfile),
                    status: JsLockfileSelectionStatus::Selected,
                    reason: None,
                },
                None => JsLockfileSelection {
                    manager: Some(manager),
                    root_path: root_path.to_string(),
                    lockfile: None,
                    status: JsLockfileSelectionStatus::MissingLockfile,
                    reason: Some(format!("missing {} lockfile", manager.label())),
                },
            },
            Err(reason) => JsLockfileSelection {
                manager: None,
                root_path: root_path.to_string(),
                lockfile: None,
                status: JsLockfileSelectionStatus::Unsupported,
                reason: Some(reason),
            },
        };
    }

    let managers = lockfiles
        .iter()
        .map(|lockfile| lockfile.kind.manager())
        .collect::<BTreeSet<_>>();
    match managers.len() {
        0 => JsLockfileSelection {
            manager: None,
            root_path: root_path.to_string(),
            lockfile: None,
            status: JsLockfileSelectionStatus::MissingLockfile,
            reason: Some("missing js lockfile".to_string()),
        },
        1 => {
            let manager = *managers.iter().next().expect("manager exists");
            JsLockfileSelection {
                manager: Some(manager),
                root_path: root_path.to_string(),
                lockfile: select_lockfile_for_manager(manager, &lockfiles),
                status: JsLockfileSelectionStatus::Selected,
                reason: None,
            }
        }
        _ => JsLockfileSelection {
            manager: None,
            root_path: root_path.to_string(),
            lockfile: None,
            status: JsLockfileSelectionStatus::Ambiguous,
            reason: Some(format!(
                "multiple lockfile managers without packageManager: {}",
                managers
                    .iter()
                    .map(|manager| manager.label())
                    .collect::<Vec<_>>()
                    .join(",")
            )),
        },
    }
}

fn parse_package_manager(value: &str) -> Result<JsPackageManager, String> {
    let name = value
        .split('@')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match name.as_str() {
        "npm" => Ok(JsPackageManager::Npm),
        "pnpm" => Ok(JsPackageManager::Pnpm),
        "yarn" => Ok(JsPackageManager::Yarn),
        "bun" => Ok(JsPackageManager::Bun),
        "" => Err("empty packageManager".to_string()),
        other => Err(format!("unsupported packageManager {other}")),
    }
}

fn detect_js_lockfiles(root: &Path, package_path: &str) -> Vec<DetectedJsLockfile> {
    [
        JsLockfileKind::NpmPackageLock,
        JsLockfileKind::NpmShrinkwrap,
        JsLockfileKind::Pnpm,
        JsLockfileKind::Yarn,
        JsLockfileKind::Bun,
    ]
    .into_iter()
    .filter(|kind| repo_file_exists(root, lockfile_relative_path(package_path, *kind)))
    .map(|kind| DetectedJsLockfile { kind })
    .collect()
}

fn select_lockfile_for_manager(
    manager: JsPackageManager,
    lockfiles: &[DetectedJsLockfile],
) -> Option<DetectedJsLockfile> {
    if manager == JsPackageManager::Npm {
        return lockfiles
            .iter()
            .find(|lockfile| lockfile.kind == JsLockfileKind::NpmShrinkwrap)
            .copied()
            .or_else(|| {
                lockfiles
                    .iter()
                    .find(|lockfile| lockfile.kind == JsLockfileKind::NpmPackageLock)
                    .copied()
            });
    }
    lockfiles
        .iter()
        .find(|lockfile| lockfile.kind.manager() == manager)
        .copied()
}

fn lockfile_relative_path(package_path: &str, kind: JsLockfileKind) -> String {
    package_relative_path(package_path, kind.file_name())
}

fn default_lockfile_name_for_manager(manager: Option<JsPackageManager>) -> &'static str {
    match manager {
        Some(JsPackageManager::Npm) => "package-lock.json",
        Some(JsPackageManager::Pnpm) => "pnpm-lock.yaml",
        Some(JsPackageManager::Yarn) => "yarn.lock",
        Some(JsPackageManager::Bun) => "bun.lock",
        None => "js-lockfile",
    }
}

fn importer_path_for_package(lockfile_root: &str, package_path: &str) -> String {
    if lockfile_root == package_path {
        ".".to_string()
    } else if lockfile_root == "." {
        package_path.to_string()
    } else {
        package_path
            .strip_prefix(&format!("{lockfile_root}/"))
            .unwrap_or(package_path)
            .to_string()
    }
}

fn stable_label_fragment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn emit_package_manager_overlays(
    output: &mut TopologyOutput,
    package_path: &str,
    manifest: &PackageJsonManifest,
) {
    if let Some(manager) = &manifest.package_manager {
        push_overlay(
            output,
            package_path,
            format!("packageManager:{manager}"),
            Some(package_manifest_path(package_path)),
            TopologyPrecision::ExactStatic,
            TopologyStatus::Present,
        );
    }
}

fn emit_package_manifest_unsupported_overlays(
    output: &mut TopologyOutput,
    package_path: &str,
    manifest: &PackageJsonManifest,
) {
    for unsupported in &manifest.unsupported {
        let reason = unsupported.reason.replace([':', ' '], "-");
        push_overlay(
            output,
            package_path,
            format!("package-json-unsupported:{reason}"),
            Some(unsupported.source_path.clone()),
            unsupported.precision,
            unsupported.status,
        );
    }
}

fn emit_lockfile_overlays(output: &mut TopologyOutput, root: &Path, package_path: &str) {
    for lockfile in detect_js_lockfiles(root, package_path) {
        let relative_path = lockfile_relative_path(package_path, lockfile.kind);
        let manifest = read_js_lockfile_manifest(root, lockfile.kind, &relative_path);
        let schema = manifest.schema_label.clone();
        let (precision, status) = if manifest.unsupported.is_empty() {
            (TopologyPrecision::ExactStatic, TopologyStatus::Present)
        } else {
            (TopologyPrecision::Unsupported, TopologyStatus::Unsupported)
        };
        push_overlay(
            output,
            package_path,
            format!("lockfile:{}:{schema}", lockfile.kind.file_name()),
            Some(relative_path),
            precision,
            status,
        );
    }
}

fn read_js_lockfile_manifest(
    root: &Path,
    kind: JsLockfileKind,
    relative_path: &str,
) -> JsLockfileManifest {
    match read_repo_file_to_string_with_limit(root, relative_path, TOPOLOGY_LOCKFILE_MAX_BYTES) {
        Ok(contents) => parse_js_lockfile(kind, relative_path, &contents),
        Err(error) => unsupported_js_lockfile(kind, relative_path, error.stable_reason()),
    }
}

fn emit_lockfile_selection_overlay(
    output: &mut TopologyOutput,
    package_path: &str,
    selection: &JsLockfileSelection,
) {
    let manager = selection
        .manager
        .map(JsPackageManager::label)
        .unwrap_or("unknown");
    let source = selection
        .lockfile
        .map(|lockfile| lockfile.kind.file_name())
        .unwrap_or_else(|| default_lockfile_name_for_manager(selection.manager));
    let status = match selection.status {
        JsLockfileSelectionStatus::Selected => "selected",
        JsLockfileSelectionStatus::MissingLockfile => "missing",
        JsLockfileSelectionStatus::Ambiguous => "ambiguous",
        JsLockfileSelectionStatus::Unsupported => "unsupported",
    };
    let precision = match selection.status {
        JsLockfileSelectionStatus::Selected | JsLockfileSelectionStatus::MissingLockfile => {
            TopologyPrecision::ExactStatic
        }
        JsLockfileSelectionStatus::Ambiguous => TopologyPrecision::Unknown,
        JsLockfileSelectionStatus::Unsupported => TopologyPrecision::Unsupported,
    };
    let topology_status = match selection.status {
        JsLockfileSelectionStatus::Selected => TopologyStatus::Present,
        JsLockfileSelectionStatus::MissingLockfile => TopologyStatus::MissingLockfile,
        JsLockfileSelectionStatus::Ambiguous => TopologyStatus::Ambiguous,
        JsLockfileSelectionStatus::Unsupported => TopologyStatus::Unsupported,
    };
    push_overlay(
        output,
        package_path,
        format!("package-manager:{status}:{manager}:lockfile:{source}"),
        selection
            .lockfile
            .map(|lockfile| lockfile_relative_path(&selection.root_path, lockfile.kind)),
        precision,
        topology_status,
    );
}

fn emit_package_requirements(
    output: &mut TopologyOutput,
    package_id: TopologyPackageId,
    package_path: &str,
    manifest: &PackageJsonManifest,
) {
    for dependency in &manifest.dependencies {
        let kind = if dependency
            .version_requirement
            .as_deref()
            .is_some_and(|requirement| requirement.starts_with("workspace:"))
        {
            RequirementKind::Workspace
        } else {
            dependency.kind
        };
        output
            .dependency_requirements
            .push(DependencyRequirementFact {
                id: DependencyRequirementId(output.dependency_requirements.len() as u64),
                from_package: Some(package_id),
                target_package: None,
                target_name: dependency.target_name.clone(),
                version_requirement: dependency.version_requirement.clone(),
                kind,
                manifest_path: Some(package_manifest_path(package_path)),
                stable_key: intern_module_graph_stable_key(format!(
                    "js-require:{package_path}:{}:{}:{}",
                    dependency.section,
                    dependency.target_name,
                    dependency.version_requirement.as_deref().unwrap_or("")
                )),
                producer_id: TS_TOPOLOGY_PROVIDER_ID,
                precision: dependency.precision,
                status: dependency.status,
            });
    }
}

fn emit_js_lockfile_edges(
    output: &mut TopologyOutput,
    root: &Path,
    package_path: &str,
    package_id: TopologyPackageId,
    selection: &JsLockfileSelection,
) {
    match selection.status {
        JsLockfileSelectionStatus::Selected => {
            let Some(lockfile) = selection.lockfile else {
                return;
            };
            let relative_path = lockfile_relative_path(&selection.root_path, lockfile.kind);
            let manifest = read_js_lockfile_manifest(root, lockfile.kind, &relative_path);
            let unsupported_count =
                emit_lockfile_unsupported_edges(output, package_path, package_id, &manifest);
            let selected_count = emit_selected_lockfile_package_edges(
                output,
                package_path,
                package_id,
                selection,
                &manifest,
            );
            if unsupported_count == 0
                && selected_count == 0
                && package_has_lockfile_requirements(output, package_id)
            {
                emit_lockfile_problem_edge(
                    output,
                    package_path,
                    package_id,
                    lockfile.kind.file_name(),
                    "no parseable selected lockfile entries",
                    TopologyPrecision::Unsupported,
                    TopologyStatus::Unsupported,
                );
            }
        }
        JsLockfileSelectionStatus::MissingLockfile => emit_missing_lockfile_edges(
            output,
            package_path,
            package_id,
            default_lockfile_name_for_manager(selection.manager),
        ),
        JsLockfileSelectionStatus::Ambiguous => emit_lockfile_problem_edge(
            output,
            package_path,
            package_id,
            "js-lockfile",
            selection.reason.as_deref().unwrap_or("ambiguous lockfiles"),
            TopologyPrecision::Unknown,
            TopologyStatus::Ambiguous,
        ),
        JsLockfileSelectionStatus::Unsupported => emit_lockfile_problem_edge(
            output,
            package_path,
            package_id,
            "packageManager",
            selection
                .reason
                .as_deref()
                .unwrap_or("unsupported package manager"),
            TopologyPrecision::Unsupported,
            TopologyStatus::Unsupported,
        ),
    }
}

fn emit_lockfile_unsupported_edges(
    output: &mut TopologyOutput,
    package_path: &str,
    package_id: TopologyPackageId,
    manifest: &JsLockfileManifest,
) -> usize {
    let mut count = 0;
    for unsupported in &manifest.unsupported {
        let reason = stable_label_fragment(&unsupported.reason);
        output
            .resolved_dependency_edges
            .push(ResolvedDependencyEdgeFact {
                id: ResolvedDependencyEdgeId(output.resolved_dependency_edges.len() as u64),
                requirement: None,
                from_package: Some(package_id),
                to_package: None,
                package_name: String::new(),
                resolved_version: None,
                kind: ResolvedDependencyKind::LockfileSelected,
                stable_key: intern_module_graph_stable_key(format!(
                    "js-lock-unsupported:{package_path}:{}:source={}:schema={}:reason={reason}",
                    unsupported.source_path, unsupported.source_label, manifest.schema_label
                )),
                producer_id: TS_TOPOLOGY_PROVIDER_ID,
                precision: unsupported.precision,
                status: unsupported.status,
            });
        count += 1;
    }
    count
}

fn emit_selected_lockfile_package_edges(
    output: &mut TopologyOutput,
    package_path: &str,
    package_id: TopologyPackageId,
    selection: &JsLockfileSelection,
    manifest: &JsLockfileManifest,
) -> usize {
    let mut stable_keys = BTreeSet::new();
    let mut count = 0;
    for package in &manifest.packages {
        if !lockfile_package_applies_to_package(
            output,
            package_id,
            selection,
            package_path,
            package,
        ) {
            continue;
        }
        let stable_key = format!(
            "js-lock-selected:{package_path}:{}:{}:{}:source={}:schema={}",
            package.path,
            package.name,
            package.version,
            package.source_label,
            manifest.schema_label
        );
        if !stable_keys.insert(stable_key.clone()) {
            continue;
        }
        output
            .resolved_dependency_edges
            .push(ResolvedDependencyEdgeFact {
                id: ResolvedDependencyEdgeId(output.resolved_dependency_edges.len() as u64),
                requirement: requirement_id_for(output, package_id, &package.name),
                from_package: Some(package_id),
                to_package: None,
                package_name: package.name.clone(),
                resolved_version: Some(package.version.clone()),
                kind: ResolvedDependencyKind::LockfileSelected,
                stable_key: intern_module_graph_stable_key(stable_key),
                producer_id: TS_TOPOLOGY_PROVIDER_ID,
                precision: package.precision,
                status: package.status,
            });
        count += 1;
    }
    count
}

fn package_has_lockfile_requirements(
    output: &TopologyOutput,
    package_id: TopologyPackageId,
) -> bool {
    output.dependency_requirements.iter().any(|requirement| {
        requirement.from_package == Some(package_id)
            && requirement.kind != RequirementKind::Workspace
            && requirement.status == TopologyStatus::Present
    })
}

fn lockfile_package_applies_to_package(
    output: &TopologyOutput,
    package_id: TopologyPackageId,
    selection: &JsLockfileSelection,
    package_path: &str,
    package: &JsLockfilePackage,
) -> bool {
    let importer_path = importer_path_for_package(&selection.root_path, package_path);
    if let Some(package_importer_path) = package.importer_path.as_deref() {
        return package_importer_path == importer_path;
    }
    if selection.root_path == package_path {
        return true;
    }
    package_lock_path_matches_importer(&package.path, &importer_path, &package.name)
        && requirement_id_for(output, package_id, &package.name).is_some()
}

fn package_lock_path_matches_importer(
    package_path: &str,
    importer_path: &str,
    package_name: &str,
) -> bool {
    let package_entry = if importer_path == "." {
        package_path.strip_prefix("node_modules/")
    } else {
        let prefix = format!("{importer_path}/node_modules/");
        package_path.strip_prefix(&prefix)
    };
    package_entry.is_some_and(|entry| entry == package_name)
}

fn emit_lockfile_problem_edge(
    output: &mut TopologyOutput,
    package_path: &str,
    package_id: TopologyPackageId,
    source_label: &str,
    reason: &str,
    precision: TopologyPrecision,
    status: TopologyStatus,
) {
    let reason = stable_label_fragment(reason);
    output
        .resolved_dependency_edges
        .push(ResolvedDependencyEdgeFact {
            id: ResolvedDependencyEdgeId(output.resolved_dependency_edges.len() as u64),
            requirement: None,
            from_package: Some(package_id),
            to_package: None,
            package_name: String::new(),
            resolved_version: None,
            kind: ResolvedDependencyKind::LockfileSelected,
            stable_key: intern_module_graph_stable_key(format!(
                "js-lock-problem:{package_path}:source={source_label}:reason={reason}"
            )),
            producer_id: TS_TOPOLOGY_PROVIDER_ID,
            precision,
            status,
        });
}

fn emit_missing_lockfile_edges(
    output: &mut TopologyOutput,
    package_path: &str,
    package_id: TopologyPackageId,
    source_label: &str,
) {
    let requirements = output
        .dependency_requirements
        .iter()
        .filter(|requirement| {
            requirement.from_package == Some(package_id)
                && requirement.kind != RequirementKind::Workspace
                && requirement.status == TopologyStatus::Present
        })
        .map(|requirement| {
            (
                requirement.id,
                requirement.target_name.clone(),
                requirement.version_requirement.clone(),
            )
        })
        .collect::<Vec<_>>();
    for (requirement_id, target_name, version_requirement) in requirements {
        output
            .resolved_dependency_edges
            .push(ResolvedDependencyEdgeFact {
                id: ResolvedDependencyEdgeId(output.resolved_dependency_edges.len() as u64),
                requirement: Some(requirement_id),
                from_package: Some(package_id),
                to_package: None,
                package_name: target_name.clone(),
                resolved_version: version_requirement.clone(),
                kind: ResolvedDependencyKind::LockfileSelected,
                stable_key: intern_module_graph_stable_key(format!(
                    "js-lock-missing:{package_path}:{target_name}:{}:source={source_label}",
                    version_requirement.as_deref().unwrap_or("")
                )),
                producer_id: TS_TOPOLOGY_PROVIDER_ID,
                precision: TopologyPrecision::Unknown,
                status: TopologyStatus::MissingLockfile,
            });
    }
}

fn requirement_id_for(
    output: &TopologyOutput,
    package_id: TopologyPackageId,
    target_name: &str,
) -> Option<DependencyRequirementId> {
    output
        .dependency_requirements
        .iter()
        .find(|requirement| {
            requirement.from_package == Some(package_id) && requirement.target_name == target_name
        })
        .map(|requirement| requirement.id)
}

fn package_relative_path(package_path: &str, file_name: &str) -> String {
    if package_path == "." {
        file_name.to_string()
    } else {
        format!("{package_path}/{file_name}")
    }
}

fn emit_pnpm_workspace_overlays(output: &mut TopologyOutput, root: &Path) {
    let relative_path = "pnpm-workspace.yaml";
    match root_pnpm_workspace_patterns_result(root) {
        Ok(workspaces) => {
            for workspace in workspaces {
                push_overlay(
                    output,
                    ".",
                    format!("pnpm-workspace.yaml:{workspace}"),
                    Some(relative_path.to_string()),
                    TopologyPrecision::Heuristic,
                    TopologyStatus::Present,
                );
            }
        }
        Err(error) if !error.is_not_found() => push_overlay(
            output,
            ".",
            format!(
                "pnpm-workspace-unsupported:{}",
                stable_label_fragment(error.stable_reason())
            ),
            Some(relative_path.to_string()),
            TopologyPrecision::Unsupported,
            TopologyStatus::Unsupported,
        ),
        Err(_) => {}
    }
}

fn root_pnpm_workspace_patterns(root: &Path) -> Vec<String> {
    root_pnpm_workspace_patterns_result(root).unwrap_or_default()
}

fn root_pnpm_workspace_patterns_result(root: &Path) -> Result<Vec<String>, RepoFileReadError> {
    let relative_path = "pnpm-workspace.yaml";
    read_repo_file_to_string_with_limit(root, relative_path, TOPOLOGY_MANIFEST_MAX_BYTES)
        .map(|contents| parse_pnpm_workspace_packages(&contents))
}

fn emit_tsconfig_overlays(output: &mut TopologyOutput, root: &Path, ts_files: &[&SourceFile]) {
    let mut configs = BTreeSet::new();
    for file in ts_files {
        let absolute = if file.path.is_absolute() {
            file.path.clone()
        } else {
            root.join(&file.relative_path)
        };
        if let Some(config) = nearest_tsconfig_path(root, &absolute)
            && let Some(relative) = crate::ts::repo_fs::normalize_repo_relative_path(root, &config)
        {
            configs.insert(relative);
        }
    }
    for config in configs {
        let value = match read_json_with_comments(root, &config) {
            Ok(Some(value)) => value,
            Ok(None) => continue,
            Err(error) => {
                push_overlay(
                    output,
                    ".",
                    format!(
                        "tsconfig-unsupported:{}",
                        stable_label_fragment(error.stable_reason())
                    ),
                    Some(config.clone()),
                    TopologyPrecision::Unsupported,
                    TopologyStatus::Unsupported,
                );
                continue;
            }
        };
        let Some(object) = value.as_object() else {
            continue;
        };
        if let Some(options) = object
            .get("compilerOptions")
            .and_then(serde_json::Value::as_object)
        {
            if let Some(base_url) = options.get("baseUrl").and_then(serde_json::Value::as_str) {
                push_overlay(
                    output,
                    ".",
                    format!("tsconfig:baseUrl:{base_url}"),
                    Some(config.clone()),
                    TopologyPrecision::ExactStatic,
                    TopologyStatus::Present,
                );
            }
            if let Some(paths) = options.get("paths").and_then(serde_json::Value::as_object) {
                for pattern in paths.keys() {
                    push_overlay(
                        output,
                        ".",
                        format!("tsconfig:paths:{pattern}"),
                        Some(config.clone()),
                        TopologyPrecision::ExactStatic,
                        TopologyStatus::Present,
                    );
                }
            }
            if let Some(root_dirs) = options
                .get("rootDirs")
                .and_then(serde_json::Value::as_array)
            {
                for root_dir in root_dirs.iter().filter_map(serde_json::Value::as_str) {
                    push_overlay(
                        output,
                        ".",
                        format!("tsconfig:rootDirs:{root_dir}"),
                        Some(config.clone()),
                        TopologyPrecision::ExactStatic,
                        TopologyStatus::Present,
                    );
                }
            }
        }
        if let Some(references) = object
            .get("references")
            .and_then(serde_json::Value::as_array)
        {
            for reference in references
                .iter()
                .filter_map(serde_json::Value::as_object)
                .filter_map(|reference| reference.get("path").and_then(serde_json::Value::as_str))
            {
                push_overlay(
                    output,
                    ".",
                    format!("tsconfig:reference:{reference}"),
                    Some(config.clone()),
                    TopologyPrecision::ExactStatic,
                    TopologyStatus::Present,
                );
            }
        }
    }
}

fn read_json_with_comments(
    root: &Path,
    relative_path: &str,
) -> Result<Option<serde_json::Value>, RepoFileReadError> {
    let source =
        read_repo_file_to_string_with_limit(root, relative_path, TOPOLOGY_MANIFEST_MAX_BYTES)?;
    Ok(crate::bounded_json::from_jsonc_str(&source, TOPOLOGY_MANIFEST_MAX_BYTES).ok())
}

fn classify_ts_source_set(file: &SourceFile) -> SourceSetKind {
    let path = file.relative_path.replace('\\', "/");
    if path.contains("/node_modules/") || path.starts_with("node_modules/") {
        return SourceSetKind::Vendor;
    }
    if path.contains("/generated/")
        || path.starts_with("generated/")
        || path.contains("/gen/")
        || path.starts_with("gen/")
        || path.contains(".generated.")
    {
        return SourceSetKind::Generated;
    }
    if path.contains("/__tests__/")
        || path.starts_with("__tests__/")
        || path.contains("/test/")
        || path.starts_with("test/")
        || path.contains("/tests/")
        || path.starts_with("tests/")
        || path.contains(".test.")
        || path.contains(".spec.")
    {
        return SourceSetKind::Test;
    }
    SourceSetKind::Source
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

fn push_overlay(
    output: &mut TopologyOutput,
    package_path: &str,
    label: String,
    path: Option<String>,
    precision: TopologyPrecision,
    status: TopologyStatus,
) {
    output.overlays.push(RepoTopologyOverlayFact {
        id: RepoTopologyOverlayId(output.overlays.len() as u64),
        root: None,
        package: None,
        source_set: None,
        kind: RepoTopologyOverlayKind::SourceOfTruthDirectory,
        stable_key: intern_module_graph_stable_key(format!(
            "ts-overlay:{package_path}:{label}:{}",
            path.as_deref().unwrap_or("")
        )),
        label,
        path,
        producer_id: TS_TOPOLOGY_PROVIDER_ID,
        precision,
        status,
    });
}

fn is_external_package_specifier(specifier: &str) -> bool {
    !specifier.starts_with('.')
        && !specifier.starts_with('/')
        && !specifier.starts_with('#')
        && !specifier.starts_with("@/")
}

#[cfg(feature = "lang-typescript")]
fn tsconfig_path_alias_matches(
    context: &TsResolverContext,
    importer_path: &Path,
    specifier: &str,
) -> bool {
    let Some(mut current) = importer_path.parent().and_then(normalize_path) else {
        return false;
    };
    loop {
        if let Some(patterns) = context.path_aliases_by_config_dir.get(&current) {
            return patterns
                .iter()
                .any(|pattern| ts_path_pattern_matches(pattern, specifier));
        }
        if current == context.root || !current.starts_with(&context.root) || !current.pop() {
            return false;
        }
    }
}

fn ts_path_pattern_matches(pattern: &str, specifier: &str) -> bool {
    let Some((prefix, suffix)) = pattern.split_once('*') else {
        return pattern == specifier;
    };
    specifier.starts_with(prefix) && specifier.ends_with(suffix)
}

fn collect_ts_path_aliases(root: &Path, db: &dyn FactDatabase) -> BTreeMap<PathBuf, Vec<String>> {
    let mut aliases = BTreeMap::new();
    for file in db
        .files()
        .iter()
        .filter(|file| file.language.is_ts_family())
    {
        let absolute = if file.path.is_absolute() {
            file.path.clone()
        } else {
            root.join(&file.relative_path)
        };
        let Some(config_path) = nearest_tsconfig_path(root, &absolute) else {
            continue;
        };
        let Some(config_dir) = config_path.parent().and_then(normalize_path) else {
            continue;
        };
        aliases
            .entry(config_dir)
            .or_insert_with(|| read_tsconfig_path_aliases(root, &config_path));
    }
    aliases
}

/// First `tsconfig.json` at or above `file_path`, bounded by `root`.
///
/// Import resolution and the type sidecar must agree on where a TypeScript
/// project starts, so both ask this one walk rather than each keeping its own
/// notion of a project boundary.
pub(crate) fn nearest_tsconfig_path(root: &Path, file_path: &Path) -> Option<PathBuf> {
    let root = normalize_path(root)?;
    let mut current = normalize_path(file_path.parent()?)?;
    loop {
        let candidate = current.join("tsconfig.json");
        if candidate.is_file() {
            return Some(candidate);
        }
        if current == root || !current.starts_with(&root) || !current.pop() {
            return None;
        }
    }
}

/// Digest of a `tsconfig.json`, every config it extends, and every project it
/// references.
///
/// What a TypeScript program contains and how it is type-checked is decided by
/// these files, and no source-file digest covers them: editing `strict`,
/// `paths`, `lib` or `include` changes every type answer while leaving every
/// `.ts` file untouched. Anything keyed on the scan's sources alone would
/// replay the previous run's typed facts after such an edit.
///
/// A named config that does not exist contributes its absence, so creating it
/// later is also a change.
pub(crate) fn tsconfig_chain_digest(root: &Path, relative_config: &str) -> String {
    let mut parts = Vec::new();
    collect_tsconfig_chain_digest(root, relative_config, &mut parts);
    parts.sort();
    parts.dedup();
    crate::ts::hash::stable_hash(&parts.iter().map(String::as_str).collect::<Vec<_>>())
}

fn collect_tsconfig_chain_digest(root: &Path, relative_config: &str, parts: &mut Vec<String>) {
    let Some(start) = normalize_repo_relative(relative_config) else {
        return;
    };
    let chain = walk_tsconfig_chain(start, |relative_path| {
        let Ok(source) =
            read_repo_file_to_string_with_limit(root, relative_path, TOPOLOGY_MANIFEST_MAX_BYTES)
        else {
            parts.push(format!("{relative_path}=absent"));
            return Vec::new();
        };
        parts.push(format!(
            "{relative_path}={}",
            crate::ts::hash::stable_hash(&[source.as_str()])
        ));
        let Some(config) = parse_tsconfig_json::<TsconfigChainWire>(&source) else {
            return Vec::new();
        };
        let config_dir = Path::new(relative_path).parent().unwrap_or(Path::new(""));
        // A referenced project is a config whose own options decide the types
        // of the files the referencing config delegates to it, so the chain
        // follows references exactly as it follows `extends`.
        let referenced = config
            .references
            .into_iter()
            .flatten()
            .filter_map(|reference| reference.path)
            .filter_map(|path| {
                resolve_tsconfig_file_candidate(root, &config_dir.join(Path::new(&path)))
            });
        let mut next = extended_configs(root, relative_path, config.extends);
        next.extend(referenced);
        next
    });
    if chain.exceeded {
        parts.push(format!("{relative_config}=exceeds-config-bound"));
    }
}

/// The `paths` aliases the config at `path` declares or inherits.
///
/// A config that reaches more configs than the walk follows gives the aliases
/// of the configs it read: import resolution loads no such config, so its
/// imports are unknown whatever the aliases.
fn read_tsconfig_path_aliases(root: &Path, path: &Path) -> Vec<String> {
    let Some(start) = repo_relative_existing_path(root, path) else {
        return Vec::new();
    };
    let mut aliases = Vec::new();
    walk_tsconfig_chain(start, |relative_path| {
        let Some(config) = read_tsconfig_json::<TsconfigAliasWire>(root, relative_path) else {
            return Vec::new();
        };
        if let Some(paths) = config
            .compiler_options
            .as_ref()
            .and_then(|options| options.paths.as_ref())
        {
            aliases.extend(paths.keys().cloned());
            return Vec::new();
        }
        extended_configs(root, relative_path, config.extends)
            .into_iter()
            .filter_map(|extended| {
                let path = repo_file_path(root, &extended).ok()?;
                repo_relative_existing_path(root, &path)
            })
            .collect()
    });
    sorted_ts_path_aliases(aliases.into_iter())
}

fn sorted_ts_path_aliases(paths: impl Iterator<Item = String>) -> Vec<String> {
    let mut aliases = paths.collect::<Vec<_>>();
    aliases.sort();
    aliases.dedup();
    aliases
}

#[derive(Debug, Deserialize)]
struct TsconfigChainWire {
    #[serde(default)]
    extends: Option<TsconfigExtendsWire>,
    #[serde(default)]
    references: Option<Vec<TsconfigReferenceWire>>,
}

#[derive(Debug, Deserialize)]
struct TsconfigReferenceWire {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TsconfigAliasWire {
    #[serde(default)]
    extends: Option<TsconfigExtendsWire>,
    compiler_options: Option<TsconfigCompilerOptionsWire>,
}

#[derive(Debug, Deserialize)]
struct TsconfigCompilerOptionsWire {
    paths: Option<BTreeMap<String, Vec<String>>>,
}

/// The `exports` conditions import resolution matches: the condition of the
/// mode an import loads in, then the rest in this order.
pub(crate) const EXPORT_CONDITIONS: &[&str] = &["import", "require", "node", "default"];

/// The package fields a bundler for Node reads, in its order, where a package
/// has no `exports`.
#[cfg(feature = "lang-typescript")]
const BUNDLER_MAIN_FIELDS: &[&str] = &["module", "main"];

/// The package fields a bundler for the browser reads, in its order, where a
/// package has no `exports`.
#[cfg(feature = "lang-typescript")]
const BROWSER_MAIN_FIELDS: &[&str] = &["browser", "module", "main"];

/// The most importer and specifier pairs one resolution follows through the
/// targets of condition maps.
#[cfg(feature = "lang-typescript")]
const MAX_FOLLOWED_ALTERNATIVES: usize = 64;

/// The `exports` conditions of one mode: its own condition and the shared
/// ones, so an `exports` map that names both modes gives each its file.
#[cfg(feature = "lang-typescript")]
fn mode_conditions(own: &str) -> Vec<String> {
    EXPORT_CONDITIONS
        .iter()
        .filter(|name| **name == own || !matches!(**name, "import" | "require"))
        .map(|name| (*name).to_string())
        .collect()
}

/// The two modes a file loads a specifier in.
#[cfg(feature = "lang-typescript")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum LoadMode {
    Import,
    Require,
}

#[cfg(feature = "lang-typescript")]
impl LoadMode {
    fn condition(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Require => "require",
        }
    }
}

/// The resolvers of one mode: Node's, and those of a bundler for Node and
/// for the browser, which read a package's `module` and `browser` fields
/// before its `main`.
#[cfg(feature = "lang-typescript")]
struct LoaderResolvers {
    node: Resolver,
    bundled: Resolver,
    browser: Resolver,
}

/// The resolvers of the two modes a file loads in, under Node and under a
/// bundler, sharing one cache.
#[cfg(feature = "lang-typescript")]
pub(crate) struct ModeResolvers {
    import: LoaderResolvers,
    require: LoaderResolvers,
    /// Reads the module format of an importing file.
    format: Resolver,
    /// The `package.json` files read for their `exports` and `imports` maps,
    /// by path. An absent manifest and a refused manifest are distinct.
    manifests: Mutex<BTreeMap<PathBuf, ManifestRead>>,
    configs: BoundedFs,
    config_issues: Mutex<BTreeMap<PathBuf, Option<RefusedInput>>>,
}

#[cfg(feature = "lang-typescript")]
type ManifestRead = Result<Option<Arc<serde_json::Value>>, RefusedInput>;

/// What resolving an import gives: the answer of every mode the import may
/// load in, or `Split`, with the files any of them names, where those modes
/// give different answers.
#[cfg(feature = "lang-typescript")]
pub(crate) enum ModeResolution {
    One(Result<oxc_resolver::Resolution, ResolveError>),
    Split(Vec<PathBuf>),
}

#[cfg(feature = "lang-typescript")]
impl ModeResolution {
    fn into_paths(self) -> Vec<PathBuf> {
        match self {
            Self::One(Ok(resolution)) => vec![resolution.into_path_buf()],
            Self::Split(paths) => paths,
            Self::One(Err(_)) => Vec::new(),
        }
    }
}

/// The answer two ways of loading an import agree on, or `Split`.
#[cfg(feature = "lang-typescript")]
fn agree(
    left: Result<oxc_resolver::Resolution, ResolveError>,
    right: Result<oxc_resolver::Resolution, ResolveError>,
) -> ModeResolution {
    match (&left, &right) {
        (Ok(one), Ok(other)) if one.path() == other.path() => ModeResolution::One(left),
        (Err(_), Err(_)) => ModeResolution::One(left),
        _ => ModeResolution::Split(
            [left, right]
                .into_iter()
                .flatten()
                .map(|resolution| resolution.into_path_buf())
                .collect(),
        ),
    }
}

/// The answers of two modes joined: `One` where both are `One` and agree.
#[cfg(feature = "lang-typescript")]
fn join(left: ModeResolution, right: ModeResolution) -> ModeResolution {
    match (left, right) {
        (ModeResolution::One(left), ModeResolution::One(right)) => agree(left, right),
        (ModeResolution::Split(mut paths), ModeResolution::Split(more)) => {
            paths.extend(more);
            ModeResolution::Split(paths)
        }
        (ModeResolution::Split(mut paths), ModeResolution::One(one))
        | (ModeResolution::One(one), ModeResolution::Split(mut paths)) => {
            paths.extend(one.map(|resolution| resolution.into_path_buf()));
            ModeResolution::Split(paths)
        }
    }
}

/// The package a bare specifier names, and the subpath of its `exports` it
/// asks for: `@scope/name/sub` is `@scope/name` and `./sub`.
#[cfg(feature = "lang-typescript")]
fn package_and_subpath(specifier: &str) -> Option<(&str, String)> {
    let segments = if specifier.starts_with('@') { 2 } else { 1 };
    let end = specifier
        .match_indices('/')
        .nth(segments - 1)
        .map_or(specifier.len(), |(index, _)| index);
    let (name, rest) = specifier.split_at(end);
    (!name.is_empty() && !name.ends_with('/')).then(|| (name, format!(".{rest}")))
}

/// The files the ways of loading one specifier name, gathered: the first
/// file found, every file, and whether the list may be incomplete.
#[cfg(feature = "lang-typescript")]
#[derive(Default)]
struct Found {
    first: Option<oxc_resolver::Resolution>,
    paths: Vec<PathBuf>,
    error: Option<ResolveError>,
    open: bool,
}

#[cfg(feature = "lang-typescript")]
impl Found {
    fn add(&mut self, resolution: oxc_resolver::Resolution) {
        self.paths.push(resolution.path().to_path_buf());
        self.first.get_or_insert(resolution);
    }

    /// The one file found where every way names it and the list is
    /// complete, the error of a loader where none names a file, and `Split`
    /// otherwise.
    fn answer(self) -> ModeResolution {
        match self.first {
            Some(first) if !self.open && self.paths.iter().all(|path| path == first.path()) => {
                ModeResolution::One(Ok(first))
            }
            None if !self.open && self.paths.is_empty() => ModeResolution::One(Err(self
                .error
                .unwrap_or(ResolveError::NotFound(String::new())))),
            _ => ModeResolution::Split(self.paths),
        }
    }
}

#[cfg(feature = "lang-typescript")]
impl ModeResolvers {
    pub(crate) fn new() -> Self {
        let node = Resolver::new_with_file_system(
            BoundedFs::new(),
            resolve_options(mode_conditions("import")),
        );
        let loaders = |mode: LoadMode| {
            let conditions = || mode_conditions(mode.condition());
            let fields =
                |fields: &[&str]| fields.iter().map(|field| (*field).to_string()).collect();
            LoaderResolvers {
                node: node.clone_with_options(resolve_options(conditions())),
                bundled: node.clone_with_options(ResolveOptions {
                    main_fields: fields(BUNDLER_MAIN_FIELDS),
                    ..resolve_options(conditions())
                }),
                browser: node.clone_with_options(ResolveOptions {
                    main_fields: fields(BROWSER_MAIN_FIELDS),
                    // The object form of the field replaces files of the package.
                    alias_fields: vec![vec!["browser".into()]],
                    ..resolve_options(conditions())
                }),
            }
        };
        let format = node.clone_with_options(ResolveOptions {
            module_type: true,
            ..resolve_options(mode_conditions("import"))
        });
        Self {
            import: loaders(LoadMode::Import),
            require: loaders(LoadMode::Require),
            format,
            manifests: Mutex::new(BTreeMap::new()),
            configs: BoundedFs::new(),
            config_issues: Mutex::new(BTreeMap::new()),
        }
    }

    /// Resolves `specifier`, written with `syntax` in the file `importer`. A
    /// `require` loads in require mode. Import syntax loads in import mode
    /// where the importer runs as an ES module (`.mjs`, `.mts`, or a `.js`
    /// or `.ts` file of a `"type": "module"` package); elsewhere a compiler
    /// may turn it into a `require`, so both modes must give one answer.
    /// Node and a bundler must give one answer too: Node loads a package's
    /// `main` where it has no `exports`, a bundler its `module` or `browser`,
    /// and every condition of an `exports` or `imports` map a loader may
    /// select must name one file.
    fn resolve(
        &self,
        importer: &Path,
        specifier: &str,
        syntax: ImportSyntax,
    ) -> (ModeResolution, Option<RefusedInput>) {
        RefusedInput::capture(|| {
            if let Some(refusal) = self.config_issue(importer) {
                refusal.record();
            }
            self.resolve_modes(importer, specifier, syntax)
        })
    }

    fn resolve_modes(
        &self,
        importer: &Path,
        specifier: &str,
        syntax: ImportSyntax,
    ) -> ModeResolution {
        if syntax == ImportSyntax::Require {
            return self.resolve_in(importer, specifier, LoadMode::Require);
        }
        let import = self.resolve_in(importer, specifier, LoadMode::Import);
        if self.runs_as_module(importer) {
            return import;
        }
        join(
            import,
            self.resolve_in(importer, specifier, LoadMode::Require),
        )
    }

    fn config_issue(&self, importer: &Path) -> Option<RefusedInput> {
        let directory = importer.parent()?;
        let mut issues = self
            .config_issues
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        issues
            .entry(directory.to_path_buf())
            .or_insert_with(|| {
                for directory in directory.ancestors() {
                    for name in ["tsconfig.json", "jsconfig.json"] {
                        let config = directory.join(name);
                        match fs::symlink_metadata(&config) {
                            Ok(_) => return self.configs.check_config(&config).err(),
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(error) => return Some(RefusedInput::new(&config, error)),
                        }
                    }
                }
                None
            })
            .clone()
    }

    /// The answer every loader gives `specifier` in `mode`, or `Split`.
    fn resolve_in(&self, importer: &Path, specifier: &str, mode: LoadMode) -> ModeResolution {
        self.resolve_alternatives(importer, specifier, mode, &mut BTreeSet::new())
    }

    /// [`Self::resolve_in`], where `seen` holds the importers and specifiers
    /// already followed. Where an `exports` or `imports` map governs the
    /// specifier, its answer is every file a target of the map may give
    /// under any conditions, with the files the loaders named; a loader that
    /// rejects the map, where Node goes on past what it rejects, does not
    /// hide them. A target may name another specifier, even the one it came
    /// from; each pair is followed once, as its files are already among the
    /// answers where it was first followed. Past [`MAX_FOLLOWED_ALTERNATIVES`]
    /// pairs the answer is `Split`, an open answer, so a crafted manifest
    /// cannot make the walk deep or long.
    fn resolve_alternatives(
        &self,
        importer: &Path,
        specifier: &str,
        mode: LoadMode,
        seen: &mut BTreeSet<(PathBuf, String)>,
    ) -> ModeResolution {
        seen.insert((importer.to_path_buf(), specifier.to_string()));
        let loaders = match mode {
            LoadMode::Import => &self.import,
            LoadMode::Require => &self.require,
        };
        let resolve = |resolver: &Resolver| {
            let outcome = resolver.resolve_file(importer, specifier);
            if let Err(error) = &outcome {
                match error {
                    ResolveError::Json(error) => {
                        RefusedInput::new(&error.path, &error.message).record()
                    }
                    ResolveError::TsconfigNotFound(path)
                    | ResolveError::TsconfigSelfReference(path) => {
                        RefusedInput::new(path, error).record()
                    }
                    ResolveError::TsconfigCircularExtend(_) | ResolveError::Recursion => {
                        RefusedInput::new(importer, error).record()
                    }
                    _ => {}
                }
            }
            outcome
        };
        let answer = join(
            agree(resolve(&loaders.node), resolve(&loaders.bundled)),
            ModeResolution::One(resolve(&loaders.browser)),
        );
        let resolved = match &answer {
            ModeResolution::One(Ok(resolution)) => Some(resolution.path().to_path_buf()),
            _ => None,
        };
        let Some(targets) =
            self.conditional_targets(importer, specifier, resolved.as_deref(), mode)
        else {
            return answer;
        };
        let mut found = Found::default();
        match answer {
            ModeResolution::One(Ok(resolution)) => found.add(resolution),
            ModeResolution::One(Err(error)) => found.error = Some(error),
            ModeResolution::Split(paths) => found.paths.extend(paths),
        }
        for (root, target) in targets {
            let importer = root.join("package.json");
            if seen.contains(&(importer.clone(), target.clone())) {
                continue;
            }
            if seen.len() >= MAX_FOLLOWED_ALTERNATIVES {
                RefusedInput::new(&importer, "condition target limit exceeded").record();
                found.open = true;
                break;
            }
            match self.resolve_alternatives(&importer, &target, mode, seen) {
                ModeResolution::One(Ok(resolution)) => found.add(resolution),
                ModeResolution::One(Err(_)) => {}
                ModeResolution::Split(paths) => {
                    found.open = true;
                    found.paths.extend(paths);
                }
            }
        }
        found.answer()
    }

    /// The targets, each with the package directory it is written in, that
    /// the `exports` map of the package `specifier` names (or, for a `#`
    /// specifier, the `imports` map of the importer's package) may give it in
    /// `mode` under any conditions. `None` where no such map applies;
    /// `resolved` is the file a loader names, if one does.
    fn conditional_targets(
        &self,
        importer: &Path,
        specifier: &str,
        resolved: Option<&Path>,
        mode: LoadMode,
    ) -> Option<Vec<(PathBuf, String)>> {
        let (root, manifest, key, exports) = if specifier.starts_with('#') {
            let (root, manifest) = importer
                .ancestors()
                .skip(1)
                .find_map(|directory| Some((directory.to_path_buf(), self.manifest(directory)?)))?;
            (root, manifest, specifier.to_string(), false)
        } else if is_external_package_specifier(specifier) {
            let (name, subpath) = package_and_subpath(specifier)?;
            let (root, manifest) = resolved
                .and_then(|resolved| self.package_of_file(resolved, name))
                .or_else(|| self.package_of_importer(importer, name))?;
            (root, manifest, subpath, true)
        } else {
            return None;
        };
        let map = manifest
            .get(if exports { "exports" } else { "imports" })
            .filter(|map| !map.is_null())?;
        Some(
            conditions::reachable_targets(map, &key, exports, mode.condition())
                .into_iter()
                .map(|target| match target {
                    conditions::Reach::File(target) | conditions::Reach::Package(target) => {
                        (root.clone(), target)
                    }
                })
                .collect(),
        )
    }

    /// The package named `name` that holds the file `resolved`.
    fn package_of_file(
        &self,
        resolved: &Path,
        name: &str,
    ) -> Option<(PathBuf, Arc<serde_json::Value>)> {
        resolved.ancestors().skip(1).find_map(|directory| {
            let manifest = self.manifest(directory)?;
            let named = directory.ends_with(Path::new("node_modules").join(name))
                || manifest.get("name").and_then(serde_json::Value::as_str) == Some(name);
            named.then(|| (directory.to_path_buf(), manifest))
        })
    }

    /// The package named `name` that Node loads for a file `importer`: the
    /// importer's own package where it has that name and an `exports` map,
    /// and otherwise the first `node_modules/name` directory of its
    /// ancestors.
    fn package_of_importer(
        &self,
        importer: &Path,
        name: &str,
    ) -> Option<(PathBuf, Arc<serde_json::Value>)> {
        let own = importer
            .ancestors()
            .skip(1)
            .find_map(|directory| Some((directory.to_path_buf(), self.manifest(directory)?)));
        if let Some((root, manifest)) = own
            && manifest.get("name").and_then(serde_json::Value::as_str) == Some(name)
            && manifest
                .get("exports")
                .is_some_and(|exports| !exports.is_null())
        {
            return Some((root, manifest));
        }
        let directory = importer
            .ancestors()
            .skip(1)
            .map(|directory| directory.join("node_modules").join(name))
            .find(|directory| directory.is_dir())?;
        let manifest = self.manifest(&directory)?;
        Some((directory, manifest))
    }

    /// The `package.json` of `directory`, read once. `None` where it is
    /// missing, or larger or deeper than the resolvers load.
    fn manifest(&self, directory: &Path) -> Option<Arc<serde_json::Value>> {
        let path = directory.join("package.json");
        let mut manifests = self
            .manifests
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match manifests
            .entry(path)
            .or_insert_with_key(|path| {
                let Some(bytes) = BoundedFs::read_limited(path, TOPOLOGY_MANIFEST_MAX_BYTES)?
                else {
                    return Ok(None);
                };
                bounded_json::from_slice(&bytes, TOPOLOGY_MANIFEST_MAX_BYTES)
                    .map(|manifest| Some(Arc::new(manifest)))
                    .map_err(|error| RefusedInput::new(path, error.reason()))
            })
            .clone()
        {
            Ok(manifest) => manifest,
            Err(refusal) => {
                refusal.record();
                None
            }
        }
    }

    fn runs_as_module(&self, importer: &Path) -> bool {
        let (Some(directory), Some(name)) = (importer.parent(), importer.file_name()) else {
            return false;
        };
        self.format
            .resolve(directory, &format!("./{}", name.to_string_lossy()))
            .is_ok_and(|resolution| {
                resolution.module_type() == Some(oxc_resolver::ModuleType::Module)
            })
    }
}

/// Resolves import specifiers against the files on disk with the options of
/// import resolution, for callers that hold no fact database.
#[cfg(feature = "lang-typescript")]
pub(crate) struct DiskResolver(ModeResolvers);

#[cfg(feature = "lang-typescript")]
impl DiskResolver {
    pub(crate) fn new() -> Self {
        Self(ModeResolvers::new())
    }

    /// The files `specifier` may name when the file `importer` imports it
    /// with `syntax`: the one file every way of loading it names, or each
    /// file one of them names. `None` where it names no file.
    pub(crate) fn resolve(
        &self,
        importer: &Path,
        specifier: &str,
        syntax: ImportSyntax,
    ) -> Option<DiskTarget> {
        let (resolution, refusal) = self.0.resolve(importer, specifier, syntax);
        let resolution = if refusal.is_some() {
            ModeResolution::Split(resolution.into_paths())
        } else {
            resolution
        };
        match resolution {
            ModeResolution::One(Ok(resolution)) => {
                normalize_path(resolution.path()).map(DiskTarget::File)
            }
            ModeResolution::One(Err(_)) => None,
            ModeResolution::Split(paths) => Some(DiskTarget::Alternatives(
                paths
                    .iter()
                    .filter_map(|path| normalize_path(path))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            )),
        }
    }
}

/// Where an import resolved on disk leads.
#[cfg(feature = "lang-typescript")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiskTarget {
    /// The one file every way of loading the import names.
    File(PathBuf),
    /// The files the ways of loading the import name, where they differ.
    /// The list may be incomplete.
    Alternatives(Vec<PathBuf>),
}

#[cfg(feature = "lang-typescript")]
fn resolve_options(condition_names: Vec<String>) -> ResolveOptions {
    ResolveOptions {
        tsconfig: Some(TsconfigDiscovery::Auto),
        extensions: vec![
            ".ts".into(),
            ".tsx".into(),
            ".js".into(),
            ".jsx".into(),
            ".json".into(),
            ".node".into(),
        ],
        extension_alias: vec![
            (
                ".js".into(),
                vec![".js".into(), ".ts".into(), ".tsx".into()],
            ),
            (".jsx".into(), vec![".jsx".into(), ".tsx".into()]),
            (".mjs".into(), vec![".mjs".into(), ".mts".into()]),
            (".cjs".into(), vec![".cjs".into(), ".cts".into()]),
        ],
        condition_names,
        main_fields: vec!["main".into()],
        exports_fields: vec![vec!["exports".into()]],
        imports_fields: vec![vec!["imports".into()]],
        builtin_modules: true,
        symlinks: false,
        ..ResolveOptions::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[cfg(unix)]
    #[test]
    fn project_module_label_does_not_read_package_json_symlink_escape() {
        let repo = tempfile::tempdir().expect("repo");
        let outside = tempfile::tempdir().expect("outside");
        fs::create_dir_all(repo.path().join("packages/app")).expect("package dir");
        fs::write(outside.path().join("package.json"), r#"{"name":"outside"}"#)
            .expect("outside package");
        std::os::unix::fs::symlink(
            outside.path().join("package.json"),
            repo.path().join("packages/app/package.json"),
        )
        .expect("symlink package manifest");

        let label = ts_project_module_label(repo.path(), &repo.path().join("packages/app"));

        assert_eq!(label, "packages/app");
    }
}
