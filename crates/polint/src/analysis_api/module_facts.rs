//! Module-graph fact rows shared by frontends and analyses.
//!
//! Owned here so language crates (for example `polint-ts` direct-binding) can name
//! resolved-import / module-node rows without depending on the facade.

use crate::internal_core::{
    FileId, ImportId, Language, ModuleEdgeId, ModuleNodeId, PackageId, ResolvedImportId,
};
use serde::{Deserialize, Serialize};

/// File, package, module, or external target participating in the module graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ModuleNode {
    pub id: ModuleNodeId,
    pub kind: ModuleNodeKind,
    pub label: String,
    pub file: Option<FileId>,
    pub package: Option<PackageId>,
    pub language: Option<Language>,
}

/// Relationship edge between two module graph nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ModuleEdge {
    pub id: ModuleEdgeId,
    pub from: ModuleNodeId,
    pub to: ModuleNodeId,
    pub import: Option<ImportId>,
    pub resolved_import: Option<ResolvedImportId>,
    pub kind: ModuleEdgeKind,
    pub status: ResolutionStatus,
}

/// Setup-aware resolution result for one syntactic import fact.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ResolvedImportFact {
    pub id: ResolvedImportId,
    pub import: ImportId,
    pub from_file: FileId,
    pub target_node: Option<ModuleNodeId>,
    pub status: ResolutionStatus,
    pub precision: ResolutionPrecision,
    pub reason: Option<UnresolvedReason>,
    /// The files an import with no single target may load, where the ways a
    /// loader may run it (its mode, its conditions, Node or a bundler) name
    /// different files. Each is a conservative candidate, and the list may
    /// be incomplete; it is empty where `target_node` names the one target.
    #[serde(default)]
    pub alternatives: Vec<ModuleNodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ModuleNodeKind {
    File,
    Package,
    Module,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ModuleEdgeKind {
    Contains,
    Imports,
    DependsOn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ResolutionStatus {
    Resolved,
    External,
    Unresolved,
    SetupMissing,
    Dynamic,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ResolutionPrecision {
    ExactFile,
    Package,
    ExternalPackage,
    Heuristic,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum UnresolvedReason {
    NotFound,
    SetupMissing,
    DynamicExpression,
    UnsupportedLanguage,
    UnsupportedImport,
    ResolverError,
    OutsideWorkspace,
    /// The import may load in import mode or in require mode, and the two
    /// modes resolve it to different files.
    ModeDependent,
}
impl ModuleNode {
    /// Constructs a module-graph node from its complete fact fields.
    pub fn new(
        id: ModuleNodeId,
        kind: ModuleNodeKind,
        label: String,
        file: Option<FileId>,
        package: Option<PackageId>,
        language: Option<Language>,
    ) -> Self {
        Self {
            id,
            kind,
            label,
            file,
            package,
            language,
        }
    }
}

impl ModuleEdge {
    /// Constructs a module-graph edge from its complete fact fields.
    pub fn new(
        id: ModuleEdgeId,
        from: ModuleNodeId,
        to: ModuleNodeId,
        import: Option<ImportId>,
        resolved_import: Option<ResolvedImportId>,
        kind: ModuleEdgeKind,
        status: ResolutionStatus,
    ) -> Self {
        Self {
            id,
            from,
            to,
            import,
            resolved_import,
            kind,
            status,
        }
    }
}

impl ResolvedImportFact {
    /// Constructs a resolved-import fact from its complete fields.
    pub fn new(
        id: ResolvedImportId,
        import: ImportId,
        from_file: FileId,
        target_node: Option<ModuleNodeId>,
        status: ResolutionStatus,
        precision: ResolutionPrecision,
        reason: Option<UnresolvedReason>,
    ) -> Self {
        Self {
            id,
            import,
            from_file,
            target_node,
            status,
            precision,
            reason,
            alternatives: Vec::new(),
        }
    }

    /// The fact with `alternatives` as the files the import may load.
    #[must_use]
    pub fn with_alternatives(mut self, alternatives: Vec<ModuleNodeId>) -> Self {
        self.alternatives = alternatives;
        self
    }
}
