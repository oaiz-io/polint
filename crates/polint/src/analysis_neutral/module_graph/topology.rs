use crate::internal_core::{
    FileId, ImportId, Language, ModuleNodeId, PackageId, ResolvedImportId, StableKeyId,
    StableKeyInterner,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WorkspaceRootId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TopologyPackageId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SourceSetId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DependencyRequirementId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ResolvedDependencyEdgeId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ImportToPackageId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RepoTopologyOverlayId(pub u64);

macro_rules! impl_topology_id_from_u64 {
    ($($id:ty),* $(,)?) => {
        $(
            impl From<u64> for $id {
                fn from(value: u64) -> Self {
                    Self(value)
                }
            }
        )*
    };
}

impl_topology_id_from_u64!(
    WorkspaceRootId,
    TopologyPackageId,
    SourceSetId,
    DependencyRequirementId,
    ResolvedDependencyEdgeId,
    ImportToPackageId,
    RepoTopologyOverlayId,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRootFact {
    pub id: WorkspaceRootId,
    pub kind: WorkspaceRootKind,
    pub root_path: String,
    pub manifest_path: Option<String>,
    pub language: Option<Language>,
    pub stable_key: StableKeyId,
    #[serde(skip_deserializing, default = "module_graph_producer_id")]
    pub producer_id: &'static str,
    pub precision: TopologyPrecision,
    pub status: TopologyStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyPackageFact {
    pub id: TopologyPackageId,
    pub workspace_root: Option<WorkspaceRootId>,
    pub package: Option<PackageId>,
    pub module_node: Option<ModuleNodeId>,
    pub kind: TopologyPackageKind,
    pub name: String,
    pub version: Option<String>,
    pub path: String,
    pub language: Option<Language>,
    pub stable_key: StableKeyId,
    #[serde(skip_deserializing, default = "module_graph_producer_id")]
    pub producer_id: &'static str,
    pub precision: TopologyPrecision,
    pub status: TopologyStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSetFact {
    pub id: SourceSetId,
    pub package: Option<TopologyPackageId>,
    pub root: Option<WorkspaceRootId>,
    pub kind: SourceSetKind,
    pub path: String,
    pub language: Option<Language>,
    pub files: Vec<FileId>,
    pub stable_key: StableKeyId,
    #[serde(skip_deserializing, default = "module_graph_producer_id")]
    pub producer_id: &'static str,
    pub precision: TopologyPrecision,
    pub status: TopologyStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyRequirementFact {
    pub id: DependencyRequirementId,
    pub from_package: Option<TopologyPackageId>,
    pub target_package: Option<TopologyPackageId>,
    pub target_name: String,
    pub version_requirement: Option<String>,
    pub kind: RequirementKind,
    pub manifest_path: Option<String>,
    pub stable_key: StableKeyId,
    #[serde(skip_deserializing, default = "module_graph_producer_id")]
    pub producer_id: &'static str,
    pub precision: TopologyPrecision,
    pub status: TopologyStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedDependencyEdgeFact {
    pub id: ResolvedDependencyEdgeId,
    pub requirement: Option<DependencyRequirementId>,
    pub from_package: Option<TopologyPackageId>,
    pub to_package: Option<TopologyPackageId>,
    pub package_name: String,
    pub resolved_version: Option<String>,
    pub kind: ResolvedDependencyKind,
    pub stable_key: StableKeyId,
    #[serde(skip_deserializing, default = "module_graph_producer_id")]
    pub producer_id: &'static str,
    pub precision: TopologyPrecision,
    pub status: TopologyStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportToPackageFact {
    pub id: ImportToPackageId,
    pub syntax_import: Option<ImportId>,
    pub resolved_import: Option<ResolvedImportId>,
    pub semantic_import_stable_key: Option<StableKeyId>,
    pub from_file: Option<FileId>,
    pub from_package: Option<TopologyPackageId>,
    pub to_package: Option<TopologyPackageId>,
    pub target_node: Option<ModuleNodeId>,
    pub from_package_stable_key: Option<StableKeyId>,
    pub to_package_stable_key: Option<StableKeyId>,
    pub source_set_stable_key: Option<StableKeyId>,
    pub import_path: String,
    pub context: ImportContextKind,
    pub stable_key: StableKeyId,
    #[serde(skip_deserializing, default = "module_topology_producer_id")]
    pub producer_id: &'static str,
    pub precision: TopologyPrecision,
    pub status: ImportToPackageStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoTopologyOverlayFact {
    pub id: RepoTopologyOverlayId,
    pub root: Option<WorkspaceRootId>,
    pub package: Option<TopologyPackageId>,
    pub source_set: Option<SourceSetId>,
    pub kind: RepoTopologyOverlayKind,
    pub label: String,
    pub path: Option<String>,
    pub stable_key: StableKeyId,
    #[serde(skip_deserializing, default = "module_graph_producer_id")]
    pub producer_id: &'static str,
    pub precision: TopologyPrecision,
    pub status: TopologyStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyOutput {
    pub workspace_roots: Vec<WorkspaceRootFact>,
    pub packages: Vec<TopologyPackageFact>,
    pub source_sets: Vec<SourceSetFact>,
    pub dependency_requirements: Vec<DependencyRequirementFact>,
    pub resolved_dependency_edges: Vec<ResolvedDependencyEdgeFact>,
    pub import_to_package_edges: Vec<ImportToPackageFact>,
    pub overlays: Vec<RepoTopologyOverlayFact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum WorkspaceRootKind {
    Repository,
    GoModule,
    GoWorkspace,
    JsWorkspace,
    PackageWorkspace,
    TsProject,
    Virtual,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TopologyPackageKind {
    Workspace,
    JsPackage,
    Project,
    Package,
    External,
    Virtual,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SourceSetKind {
    Source,
    Test,
    Generated,
    Vendor,
    External,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RequirementKind {
    Direct,
    Runtime,
    Dev,
    Development,
    Peer,
    Optional,
    Bundled,
    Workspace,
    Build,
    Test,
    Tool,
    Replace,
    Exclude,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ResolvedDependencyKind {
    Lockfile,
    LockfileSelected,
    ChecksumEvidence,
    ToolResolved,
    LocalReplacement,
    Workspace,
    External,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ImportToPackageStatus {
    Resolved,
    External,
    Unresolved,
    SetupMissing,
    Unsupported,
    Dynamic,
    Ambiguous,
    Undeclared,
    OutsideWorkspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ImportContextKind {
    Source,
    Test,
    Generated,
    Vendor,
    External,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RepoTopologyOverlayKind {
    OwnershipZone,
    ArchitectureLayer,
    DeployUnit,
    GeneratedZone,
    TestOnlyVisibility,
    InternalPublicApiBoundary,
    SourceOfTruthDirectory,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TopologyPrecision {
    ExactStatic,
    ExactLockfile,
    Heuristic,
    Unknown,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TopologyStatus {
    Present,
    Resolved,
    Ambiguous,
    Unresolved,
    Unknown,
    SetupMissing,
    MissingLockfile,
    Generated,
    External,
    Unsupported,
}

impl TopologyOutput {
    pub fn merge(&mut self, mut other: TopologyOutput) {
        let root_offset = self.workspace_roots.len() as u64;
        let package_offset = self.packages.len() as u64;
        let source_set_offset = self.source_sets.len() as u64;
        let requirement_offset = self.dependency_requirements.len() as u64;
        let resolved_dependency_offset = self.resolved_dependency_edges.len() as u64;
        let import_to_package_offset = self.import_to_package_edges.len() as u64;
        let overlay_offset = self.overlays.len() as u64;

        offset_output_ids(
            &mut other,
            root_offset,
            package_offset,
            source_set_offset,
            requirement_offset,
            resolved_dependency_offset,
            import_to_package_offset,
            overlay_offset,
        );
        self.workspace_roots.extend(other.workspace_roots);
        self.packages.extend(other.packages);
        self.source_sets.extend(other.source_sets);
        self.dependency_requirements
            .extend(other.dependency_requirements);
        self.resolved_dependency_edges
            .extend(other.resolved_dependency_edges);
        self.import_to_package_edges
            .extend(other.import_to_package_edges);
        self.overlays.extend(other.overlays);
    }

    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        let root_ids = normalize_rows(
            &mut self.workspace_roots,
            |row| row.id,
            |row| row.stable_key,
            interner,
            |row, id| row.id = WorkspaceRootId(id),
        );
        let package_ids = normalize_rows(
            &mut self.packages,
            |row| row.id,
            |row| row.stable_key,
            interner,
            |row, id| row.id = TopologyPackageId(id),
        );
        let source_set_ids = normalize_rows(
            &mut self.source_sets,
            |row| row.id,
            |row| row.stable_key,
            interner,
            |row, id| row.id = SourceSetId(id),
        );
        let requirement_ids = normalize_rows(
            &mut self.dependency_requirements,
            |row| row.id,
            |row| row.stable_key,
            interner,
            |row, id| row.id = DependencyRequirementId(id),
        );
        normalize_rows(
            &mut self.resolved_dependency_edges,
            |row| row.id,
            |row| row.stable_key,
            interner,
            |row, id| row.id = ResolvedDependencyEdgeId(id),
        );
        normalize_rows(
            &mut self.import_to_package_edges,
            |row| row.id,
            |row| row.stable_key,
            interner,
            |row, id| row.id = ImportToPackageId(id),
        );
        normalize_rows(
            &mut self.overlays,
            |row| row.id,
            |row| row.stable_key,
            interner,
            |row, id| row.id = RepoTopologyOverlayId(id),
        );
        for package in &mut self.packages {
            remap_option(&mut package.workspace_root, &root_ids);
        }
        for source_set in &mut self.source_sets {
            remap_option(&mut source_set.package, &package_ids);
            remap_option(&mut source_set.root, &root_ids);
        }
        for requirement in &mut self.dependency_requirements {
            remap_option(&mut requirement.from_package, &package_ids);
            remap_option(&mut requirement.target_package, &package_ids);
        }
        for edge in &mut self.resolved_dependency_edges {
            remap_option(&mut edge.requirement, &requirement_ids);
            remap_option(&mut edge.from_package, &package_ids);
            remap_option(&mut edge.to_package, &package_ids);
        }
        for edge in &mut self.import_to_package_edges {
            remap_option(&mut edge.from_package, &package_ids);
            remap_option(&mut edge.to_package, &package_ids);
        }
        for overlay in &mut self.overlays {
            remap_option(&mut overlay.root, &root_ids);
            remap_option(&mut overlay.package, &package_ids);
            remap_option(&mut overlay.source_set, &source_set_ids);
        }
        self
    }

    pub fn canonicalized_for_cache(mut self, interner: &StableKeyInterner) -> (Self, Vec<String>) {
        let mut texts = self
            .stable_key_ids()
            .into_iter()
            .map(|key| interner.resolve(key).to_string())
            .collect::<Vec<_>>();
        texts.sort();
        texts.dedup();
        let ids_by_text = texts
            .iter()
            .enumerate()
            .map(|(index, text)| (text.as_str(), StableKeyId(index as u32)))
            .collect::<BTreeMap<_, _>>();
        self.remap_stable_keys(|key| ids_by_text[interner.resolve(key).as_ref()]);
        (self, texts)
    }

    pub fn reintern_cached(
        mut self,
        stable_key_texts: &[String],
        interner: &StableKeyInterner,
    ) -> Option<Self> {
        let ids = stable_key_texts
            .iter()
            .map(|text| interner.intern(text.clone()))
            .collect::<Vec<_>>();
        let mut valid = true;
        self.remap_stable_keys(|key| {
            ids.get(key.0 as usize).copied().unwrap_or_else(|| {
                valid = false;
                StableKeyId(0)
            })
        });
        valid.then_some(self)
    }

    fn stable_key_ids(&self) -> Vec<StableKeyId> {
        let mut ids = Vec::new();
        ids.extend(self.workspace_roots.iter().map(|row| row.stable_key));
        ids.extend(self.packages.iter().map(|row| row.stable_key));
        ids.extend(self.source_sets.iter().map(|row| row.stable_key));
        ids.extend(
            self.dependency_requirements
                .iter()
                .map(|row| row.stable_key),
        );
        ids.extend(
            self.resolved_dependency_edges
                .iter()
                .map(|row| row.stable_key),
        );
        for row in &self.import_to_package_edges {
            ids.push(row.stable_key);
            ids.extend(row.semantic_import_stable_key);
            ids.extend(row.from_package_stable_key);
            ids.extend(row.to_package_stable_key);
            ids.extend(row.source_set_stable_key);
        }
        ids.extend(self.overlays.iter().map(|row| row.stable_key));
        ids
    }

    fn remap_stable_keys(&mut self, mut remap: impl FnMut(StableKeyId) -> StableKeyId) {
        for row in &mut self.workspace_roots {
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut self.packages {
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut self.source_sets {
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut self.dependency_requirements {
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut self.resolved_dependency_edges {
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut self.import_to_package_edges {
            row.stable_key = remap(row.stable_key);
            row.semantic_import_stable_key = row.semantic_import_stable_key.map(&mut remap);
            row.from_package_stable_key = row.from_package_stable_key.map(&mut remap);
            row.to_package_stable_key = row.to_package_stable_key.map(&mut remap);
            row.source_set_stable_key = row.source_set_stable_key.map(&mut remap);
        }
        for row in &mut self.overlays {
            row.stable_key = remap(row.stable_key);
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "each topology ID family has an independent run-local namespace that must be offset separately"
)]
fn offset_output_ids(
    output: &mut TopologyOutput,
    root_offset: u64,
    package_offset: u64,
    source_set_offset: u64,
    requirement_offset: u64,
    resolved_dependency_offset: u64,
    import_to_package_offset: u64,
    overlay_offset: u64,
) {
    for root in &mut output.workspace_roots {
        root.id.0 += root_offset;
    }
    for package in &mut output.packages {
        package.id.0 += package_offset;
        offset_option_id(&mut package.workspace_root, root_offset);
    }
    for source_set in &mut output.source_sets {
        source_set.id.0 += source_set_offset;
        offset_option_id(&mut source_set.package, package_offset);
        offset_option_id(&mut source_set.root, root_offset);
    }
    for requirement in &mut output.dependency_requirements {
        requirement.id.0 += requirement_offset;
        offset_option_id(&mut requirement.from_package, package_offset);
        offset_option_id(&mut requirement.target_package, package_offset);
    }
    for edge in &mut output.resolved_dependency_edges {
        edge.id.0 += resolved_dependency_offset;
        offset_option_id(&mut edge.requirement, requirement_offset);
        offset_option_id(&mut edge.from_package, package_offset);
        offset_option_id(&mut edge.to_package, package_offset);
    }
    for edge in &mut output.import_to_package_edges {
        edge.id.0 += import_to_package_offset;
        offset_option_id(&mut edge.from_package, package_offset);
        offset_option_id(&mut edge.to_package, package_offset);
    }
    for overlay in &mut output.overlays {
        overlay.id.0 += overlay_offset;
        offset_option_id(&mut overlay.root, root_offset);
        offset_option_id(&mut overlay.package, package_offset);
        offset_option_id(&mut overlay.source_set, source_set_offset);
    }
}

trait OffsetTopologyId {
    fn offset(&mut self, offset: u64);
}

macro_rules! impl_offset_topology_id {
    ($($id:ty),* $(,)?) => {
        $(
            impl OffsetTopologyId for $id {
                fn offset(&mut self, offset: u64) {
                    self.0 += offset;
                }
            }
        )*
    };
}

impl_offset_topology_id!(
    WorkspaceRootId,
    TopologyPackageId,
    SourceSetId,
    DependencyRequirementId,
);

fn offset_option_id<Id: OffsetTopologyId>(value: &mut Option<Id>, offset: u64) {
    if let Some(id) = value {
        id.offset(offset);
    }
}

fn normalize_rows<T, Id>(
    rows: &mut [T],
    id: impl Fn(&T) -> Id,
    stable_key: impl Fn(&T) -> StableKeyId,
    interner: &StableKeyInterner,
    mut assign_id: impl FnMut(&mut T, u64),
) -> BTreeMap<Id, Id>
where
    Id: Copy + Ord + From<u64>,
{
    rows.sort_by(|row, other| interner.compare_canonical(stable_key(row), stable_key(other)));
    let mut ids = BTreeMap::new();
    for (index, row) in rows.iter_mut().enumerate() {
        let old_id = id(row);
        let new_id = Id::from(index as u64);
        assign_id(row, index as u64);
        ids.insert(old_id, new_id);
    }
    ids
}

fn remap_option<Id: Copy + Ord>(value: &mut Option<Id>, ids: &BTreeMap<Id, Id>) {
    if let Some(id) = value
        && let Some(remapped) = ids.get(id)
    {
        *value = Some(*remapped);
    }
}

fn module_graph_producer_id() -> &'static str {
    "polint.module_graph"
}

fn module_topology_producer_id() -> &'static str {
    "polint.module_topology"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stable_key(key: &str) -> StableKeyId {
        crate::internal_core::stable_key_for_test(key)
    }

    fn root(key: &str, id: u64) -> WorkspaceRootFact {
        WorkspaceRootFact {
            id: WorkspaceRootId(id),
            kind: WorkspaceRootKind::Repository,
            root_path: ".".to_string(),
            manifest_path: None,
            language: None,
            stable_key: stable_key(key),
            producer_id: "test",
            precision: TopologyPrecision::ExactStatic,
            status: TopologyStatus::Present,
        }
    }

    fn package(key: &str, id: u64) -> TopologyPackageFact {
        TopologyPackageFact {
            id: TopologyPackageId(id),
            workspace_root: Some(WorkspaceRootId(0)),
            package: Some(PackageId::from_raw(0)),
            module_node: Some(ModuleNodeId::from_raw(0)),
            kind: TopologyPackageKind::Workspace,
            name: key.to_string(),
            version: None,
            path: ".".to_string(),
            language: Some(Language::TypeScript),
            stable_key: stable_key(key),
            producer_id: "test",
            precision: TopologyPrecision::ExactStatic,
            status: TopologyStatus::Present,
        }
    }

    fn source_set(key: &str, id: u64) -> SourceSetFact {
        SourceSetFact {
            id: SourceSetId(id),
            package: Some(TopologyPackageId(0)),
            root: Some(WorkspaceRootId(0)),
            kind: SourceSetKind::Source,
            path: "src".to_string(),
            language: Some(Language::TypeScript),
            files: vec![FileId::from_raw(0)],
            stable_key: stable_key(key),
            producer_id: "test",
            precision: TopologyPrecision::ExactStatic,
            status: TopologyStatus::Present,
        }
    }

    fn requirement(key: &str, id: u64) -> DependencyRequirementFact {
        DependencyRequirementFact {
            id: DependencyRequirementId(id),
            from_package: Some(TopologyPackageId(0)),
            target_package: None,
            target_name: key.to_string(),
            version_requirement: Some("^1.0.0".to_string()),
            kind: RequirementKind::Runtime,
            manifest_path: Some("package.json".to_string()),
            stable_key: stable_key(key),
            producer_id: "test",
            precision: TopologyPrecision::ExactStatic,
            status: TopologyStatus::Present,
        }
    }

    fn resolved_edge(key: &str, id: u64) -> ResolvedDependencyEdgeFact {
        ResolvedDependencyEdgeFact {
            id: ResolvedDependencyEdgeId(id),
            requirement: Some(DependencyRequirementId(0)),
            from_package: Some(TopologyPackageId(0)),
            to_package: None,
            package_name: key.to_string(),
            resolved_version: Some("1.0.0".to_string()),
            kind: ResolvedDependencyKind::Lockfile,
            stable_key: stable_key(key),
            producer_id: "test",
            precision: TopologyPrecision::ExactLockfile,
            status: TopologyStatus::Resolved,
        }
    }

    fn import_edge(key: &str, id: u64) -> ImportToPackageFact {
        ImportToPackageFact {
            id: ImportToPackageId(id),
            syntax_import: Some(ImportId::from_raw(0)),
            resolved_import: Some(ResolvedImportId::from_raw(0)),
            semantic_import_stable_key: None,
            from_file: Some(FileId::from_raw(0)),
            from_package: Some(TopologyPackageId(0)),
            to_package: None,
            target_node: Some(ModuleNodeId::from_raw(0)),
            from_package_stable_key: None,
            to_package_stable_key: None,
            source_set_stable_key: None,
            import_path: "example".to_string(),
            context: ImportContextKind::Source,
            stable_key: stable_key(key),
            producer_id: "test",
            precision: TopologyPrecision::ExactStatic,
            status: ImportToPackageStatus::Resolved,
        }
    }

    fn overlay(key: &str, id: u64) -> RepoTopologyOverlayFact {
        RepoTopologyOverlayFact {
            id: RepoTopologyOverlayId(id),
            root: Some(WorkspaceRootId(0)),
            package: Some(TopologyPackageId(0)),
            source_set: Some(SourceSetId(0)),
            kind: RepoTopologyOverlayKind::OwnershipZone,
            label: key.to_string(),
            path: Some("src".to_string()),
            stable_key: stable_key(key),
            producer_id: "test",
            precision: TopologyPrecision::Heuristic,
            status: TopologyStatus::Present,
        }
    }

    #[test]
    fn normalized_sorts_every_family_by_stable_key_and_reassigns_ids() {
        let output = TopologyOutput {
            workspace_roots: vec![root("root:z", 99), root("root:a", 42)],
            packages: vec![package("package:z", 99), package("package:a", 42)],
            source_sets: vec![
                source_set("source-set:z", 99),
                source_set("source-set:a", 42),
            ],
            dependency_requirements: vec![
                requirement("requirement:z", 99),
                requirement("requirement:a", 42),
            ],
            resolved_dependency_edges: vec![
                resolved_edge("resolved:z", 99),
                resolved_edge("resolved:a", 42),
            ],
            import_to_package_edges: vec![import_edge("import:z", 99), import_edge("import:a", 42)],
            overlays: vec![overlay("overlay:z", 99), overlay("overlay:a", 42)],
        }
        .normalized(&crate::internal_core::test_stable_key_interner());

        assert_eq!(output.workspace_roots[0].id, WorkspaceRootId(0));
        assert_eq!(output.workspace_roots[0].stable_key, stable_key("root:a"));
        assert_eq!(output.packages[0].id, TopologyPackageId(0));
        assert_eq!(output.packages[0].stable_key, stable_key("package:a"));
        assert_eq!(output.source_sets[0].id, SourceSetId(0));
        assert_eq!(output.source_sets[0].stable_key, stable_key("source-set:a"));
        assert_eq!(
            output.dependency_requirements[0].id,
            DependencyRequirementId(0)
        );
        assert_eq!(
            output.dependency_requirements[0].stable_key,
            stable_key("requirement:a")
        );
        assert_eq!(
            output.resolved_dependency_edges[0].id,
            ResolvedDependencyEdgeId(0)
        );
        assert_eq!(
            output.resolved_dependency_edges[0].stable_key,
            stable_key("resolved:a")
        );
        assert_eq!(output.import_to_package_edges[0].id, ImportToPackageId(0));
        assert_eq!(
            output.import_to_package_edges[0].stable_key,
            stable_key("import:a")
        );
        assert_eq!(output.overlays[0].id, RepoTopologyOverlayId(0));
        assert_eq!(output.overlays[0].stable_key, stable_key("overlay:a"));
    }

    #[test]
    fn merge_offsets_colliding_ids_before_final_normalization() {
        let mut left = TopologyOutput {
            workspace_roots: vec![root("root:b", 0)],
            packages: vec![package("package:b", 0)],
            source_sets: vec![source_set("source-set:b", 0)],
            dependency_requirements: vec![requirement("requirement:b", 0)],
            resolved_dependency_edges: vec![resolved_edge("resolved:b", 0)],
            import_to_package_edges: vec![import_edge("import:b", 0)],
            overlays: vec![overlay("overlay:b", 0)],
        };
        let right = TopologyOutput {
            workspace_roots: vec![root("root:a", 0)],
            packages: vec![package("package:a", 0)],
            source_sets: vec![source_set("source-set:a", 0)],
            dependency_requirements: vec![requirement("requirement:a", 0)],
            resolved_dependency_edges: vec![resolved_edge("resolved:a", 0)],
            import_to_package_edges: vec![import_edge("import:a", 0)],
            overlays: vec![overlay("overlay:a", 0)],
        };

        left.merge(right);
        let output = left.normalized(&crate::internal_core::test_stable_key_interner());

        let source_set_a = output
            .source_sets
            .iter()
            .find(|row| row.stable_key == stable_key("source-set:a"))
            .expect("right source set survives merge");
        assert_eq!(
            stable_key_for_root(&output, source_set_a.root),
            Some(stable_key("root:a"))
        );
        assert_eq!(
            stable_key_for_package(&output, source_set_a.package),
            Some(stable_key("package:a"))
        );

        let requirement_a = output
            .dependency_requirements
            .iter()
            .find(|row| row.stable_key == stable_key("requirement:a"))
            .expect("right requirement survives merge");
        assert_eq!(
            stable_key_for_package(&output, requirement_a.from_package),
            Some(stable_key("package:a"))
        );

        let resolved_a = output
            .resolved_dependency_edges
            .iter()
            .find(|row| row.stable_key == stable_key("resolved:a"))
            .expect("right resolved edge survives merge");
        assert_eq!(
            stable_key_for_requirement(&output, resolved_a.requirement),
            Some(stable_key("requirement:a"))
        );
        assert_eq!(
            stable_key_for_package(&output, resolved_a.from_package),
            Some(stable_key("package:a"))
        );

        let overlay_a = output
            .overlays
            .iter()
            .find(|row| row.stable_key == stable_key("overlay:a"))
            .expect("right overlay survives merge");
        assert_eq!(
            stable_key_for_root(&output, overlay_a.root),
            Some(stable_key("root:a"))
        );
        assert_eq!(
            stable_key_for_package(&output, overlay_a.package),
            Some(stable_key("package:a"))
        );
        assert_eq!(
            stable_key_for_source_set(&output, overlay_a.source_set),
            Some(stable_key("source-set:a"))
        );
    }

    #[test]
    fn cache_identity_table_reinterns_into_an_independent_interner() {
        let source = StableKeyInterner::default();
        let output = TopologyOutput {
            workspace_roots: vec![WorkspaceRootFact {
                stable_key: source.intern("root:z"),
                ..root("root:a", 0)
            }],
            import_to_package_edges: vec![ImportToPackageFact {
                stable_key: source.intern("import:a"),
                from_package_stable_key: Some(source.intern("package:a")),
                ..import_edge("import:unused", 0)
            }],
            ..TopologyOutput::default()
        };

        let (cached, texts) = output.canonicalized_for_cache(&source);
        let target = StableKeyInterner::default();
        target.intern("unrelated");
        let restored = cached
            .reintern_cached(&texts, &target)
            .expect("cache table covers every identity");

        assert_eq!(
            target
                .resolve(restored.workspace_roots[0].stable_key)
                .as_ref(),
            "root:z"
        );
        assert_eq!(
            target
                .resolve(
                    restored.import_to_package_edges[0]
                        .from_package_stable_key
                        .expect("related package identity"),
                )
                .as_ref(),
            "package:a"
        );
    }

    fn stable_key_for_root(
        output: &TopologyOutput,
        id: Option<WorkspaceRootId>,
    ) -> Option<StableKeyId> {
        output
            .workspace_roots
            .iter()
            .find(|row| Some(row.id) == id)
            .map(|row| row.stable_key)
    }

    fn stable_key_for_package(
        output: &TopologyOutput,
        id: Option<TopologyPackageId>,
    ) -> Option<StableKeyId> {
        output
            .packages
            .iter()
            .find(|row| Some(row.id) == id)
            .map(|row| row.stable_key)
    }

    fn stable_key_for_source_set(
        output: &TopologyOutput,
        id: Option<SourceSetId>,
    ) -> Option<StableKeyId> {
        output
            .source_sets
            .iter()
            .find(|row| Some(row.id) == id)
            .map(|row| row.stable_key)
    }

    fn stable_key_for_requirement(
        output: &TopologyOutput,
        id: Option<DependencyRequirementId>,
    ) -> Option<StableKeyId> {
        output
            .dependency_requirements
            .iter()
            .find(|row| Some(row.id) == id)
            .map(|row| row.stable_key)
    }

    #[test]
    fn import_to_package_status_contains_required_uncertainty_states() {
        let statuses = [
            ImportToPackageStatus::Resolved,
            ImportToPackageStatus::External,
            ImportToPackageStatus::Unresolved,
            ImportToPackageStatus::SetupMissing,
            ImportToPackageStatus::Unsupported,
            ImportToPackageStatus::Dynamic,
            ImportToPackageStatus::Ambiguous,
            ImportToPackageStatus::Undeclared,
            ImportToPackageStatus::OutsideWorkspace,
        ];

        assert_eq!(statuses.len(), 9);
    }

    #[test]
    fn source_set_kind_contains_required_contexts() {
        let kinds = [
            SourceSetKind::Source,
            SourceSetKind::Test,
            SourceSetKind::Generated,
            SourceSetKind::Vendor,
            SourceSetKind::External,
            SourceSetKind::Unknown,
        ];

        assert_eq!(kinds.len(), 6);
    }
}
