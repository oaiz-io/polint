use crate::analysis_api::{
    FactFamily, SymbolNamespace, stable_key_from_key_parts, stable_key_from_parts,
};
use crate::internal_core::{Diagnostic, DiagnosticRange as TextRange, KeyPart};
use crate::internal_core::{
    FileId, Language, ModuleNodeId, PackageId, Span, StableKeyId, SymbolId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub use crate::analysis_api::{
    ScopeId, SemanticImportFact, SemanticImportId, SemanticImportKind, SemanticStatus,
};

const SYMBOL_GRAPH_PRODUCER_ID: &str = "polint.symbol_graph";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ExportId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AliasId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ResolutionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GeneratedSymbolId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StableExportId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeFact {
    pub id: ScopeId,
    pub language: Language,
    pub file: Option<FileId>,
    pub package: Option<PackageId>,
    pub module: Option<ModuleNodeId>,
    pub parent: Option<ScopeId>,
    pub scope_path: Vec<String>,
    pub kind: ScopeKind,
    pub stable_key: StableKeyId,
    pub status: SemanticStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFact {
    pub id: ExportId,
    pub language: Language,
    pub file: Option<FileId>,
    pub package: Option<PackageId>,
    pub module: Option<ModuleNodeId>,
    pub scope: Option<ScopeId>,
    pub symbol: Option<SymbolId>,
    pub export_name: String,
    pub namespace: SymbolNamespace,
    pub kind: ExportKind,
    pub stable_key: StableKeyId,
    pub status: SemanticStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasFact {
    pub id: AliasId,
    pub language: Language,
    pub file: Option<FileId>,
    pub package: Option<PackageId>,
    pub module: Option<ModuleNodeId>,
    pub source_symbol_stable_key: StableKeyId,
    pub target_symbol_stable_keys: Vec<StableKeyId>,
    pub kind: AliasKind,
    pub stable_key: StableKeyId,
    pub status: SemanticStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionFact {
    pub id: ResolutionId,
    pub language: Language,
    pub file: Option<FileId>,
    pub package: Option<PackageId>,
    pub module: Option<ModuleNodeId>,
    pub source_stable_key: StableKeyId,
    pub target_stable_keys: Vec<StableKeyId>,
    pub step: ResolutionStepKind,
    pub stable_key: StableKeyId,
    pub status: SemanticStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedSymbolFact {
    pub id: GeneratedSymbolId,
    pub language: Language,
    pub file: Option<FileId>,
    pub package: Option<PackageId>,
    pub module: Option<ModuleNodeId>,
    pub symbol_stable_key: StableKeyId,
    pub source_stable_key: StableKeyId,
    pub producer_id: String,
    pub generator: String,
    pub generated_discriminator: String,
    pub kind: GeneratedSymbolKind,
    pub span: Option<Span>,
    pub stable_key: StableKeyId,
    pub status: SemanticStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableExportIdentity {
    pub id: StableExportId,
    pub export: ExportId,
    pub language: Language,
    pub package_key: Option<String>,
    pub module_key: Option<String>,
    pub export_name: String,
    pub namespace: SymbolNamespace,
    pub symbol_stable_key: StableKeyId,
    pub generated_discriminator: Option<String>,
    pub stable_key: StableKeyId,
    pub status: SemanticStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticIndexOutput {
    pub scopes: Vec<ScopeFact>,
    pub semantic_imports: Vec<SemanticImportFact>,
    pub exports: Vec<ExportFact>,
    pub aliases: Vec<AliasFact>,
    pub resolutions: Vec<ResolutionFact>,
    pub generated_symbols: Vec<GeneratedSymbolFact>,
    pub stable_exports: Vec<StableExportIdentity>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedSemanticIndexOutput {
    pub scopes: Vec<CachedScopeFact>,
    pub semantic_imports: Vec<CachedSemanticImportFact>,
    pub exports: Vec<CachedExportFact>,
    pub aliases: Vec<CachedAliasFact>,
    pub resolutions: Vec<CachedResolutionFact>,
    pub generated_symbols: Vec<CachedGeneratedSymbolFact>,
    pub stable_exports: Vec<CachedStableExportIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedScopeFact {
    id: ScopeId,
    language: Language,
    file: Option<FileId>,
    package: Option<PackageId>,
    module: Option<ModuleNodeId>,
    parent: Option<ScopeId>,
    scope_path: Vec<String>,
    kind: ScopeKind,
    pub stable_key_text: String,
    status: SemanticStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedSemanticImportFact {
    id: SemanticImportId,
    language: Language,
    file: Option<FileId>,
    package: Option<PackageId>,
    module: Option<ModuleNodeId>,
    scope: Option<ScopeId>,
    import_path: String,
    local_name: Option<String>,
    imported_name: Option<String>,
    namespace: SymbolNamespace,
    kind: SemanticImportKind,
    stable_key_text: String,
    status: SemanticStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedExportFact {
    id: ExportId,
    language: Language,
    file: Option<FileId>,
    package: Option<PackageId>,
    module: Option<ModuleNodeId>,
    scope: Option<ScopeId>,
    symbol: Option<SymbolId>,
    export_name: String,
    namespace: SymbolNamespace,
    kind: ExportKind,
    stable_key_text: String,
    status: SemanticStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedAliasFact {
    id: AliasId,
    language: Language,
    file: Option<FileId>,
    package: Option<PackageId>,
    module: Option<ModuleNodeId>,
    source_symbol_stable_key_text: String,
    target_symbol_stable_key_texts: Vec<String>,
    kind: AliasKind,
    stable_key_text: String,
    status: SemanticStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResolutionFact {
    id: ResolutionId,
    language: Language,
    file: Option<FileId>,
    package: Option<PackageId>,
    module: Option<ModuleNodeId>,
    source_stable_key_text: String,
    target_stable_key_texts: Vec<String>,
    step: ResolutionStepKind,
    stable_key_text: String,
    status: SemanticStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedGeneratedSymbolFact {
    id: GeneratedSymbolId,
    language: Language,
    file: Option<FileId>,
    package: Option<PackageId>,
    module: Option<ModuleNodeId>,
    symbol_stable_key_text: String,
    source_stable_key_text: String,
    producer_id: String,
    generator: String,
    generated_discriminator: String,
    kind: GeneratedSymbolKind,
    span: Option<Span>,
    stable_key_text: String,
    status: SemanticStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedStableExportIdentity {
    pub id: StableExportId,
    pub export: ExportId,
    language: Language,
    package_key: Option<String>,
    pub module_key: Option<String>,
    export_name: String,
    namespace: SymbolNamespace,
    pub symbol_stable_key_text: String,
    generated_discriminator: Option<String>,
    pub stable_key_text: String,
    status: SemanticStatus,
}

impl CachedSemanticIndexOutput {
    pub fn from_output(
        interner: &crate::internal_core::StableKeyInterner,
        output: &SemanticIndexOutput,
    ) -> Self {
        Self {
            scopes: output
                .scopes
                .iter()
                .map(|row| CachedScopeFact {
                    id: row.id,
                    language: row.language,
                    file: row.file,
                    package: row.package,
                    module: row.module,
                    parent: row.parent,
                    scope_path: row.scope_path.clone(),
                    kind: row.kind,
                    stable_key_text: interner.resolve(row.stable_key).to_string(),
                    status: row.status,
                })
                .collect(),
            semantic_imports: output
                .semantic_imports
                .iter()
                .map(|row| CachedSemanticImportFact {
                    id: row.id,
                    language: row.language,
                    file: row.file,
                    package: row.package,
                    module: row.module,
                    scope: row.scope,
                    import_path: row.import_path.clone(),
                    local_name: row.local_name.clone(),
                    imported_name: row.imported_name.clone(),
                    namespace: row.namespace,
                    kind: row.kind,
                    stable_key_text: interner.resolve(row.stable_key).to_string(),
                    status: row.status,
                })
                .collect(),
            exports: output
                .exports
                .iter()
                .map(|row| CachedExportFact {
                    id: row.id,
                    language: row.language,
                    file: row.file,
                    package: row.package,
                    module: row.module,
                    scope: row.scope,
                    symbol: row.symbol,
                    export_name: row.export_name.clone(),
                    namespace: row.namespace,
                    kind: row.kind,
                    stable_key_text: interner.resolve(row.stable_key).to_string(),
                    status: row.status,
                })
                .collect(),
            aliases: output
                .aliases
                .iter()
                .map(|row| CachedAliasFact {
                    id: row.id,
                    language: row.language,
                    file: row.file,
                    package: row.package,
                    module: row.module,
                    source_symbol_stable_key_text: interner
                        .resolve(row.source_symbol_stable_key)
                        .to_string(),
                    target_symbol_stable_key_texts: row
                        .target_symbol_stable_keys
                        .iter()
                        .map(|key| interner.resolve(*key).to_string())
                        .collect(),
                    kind: row.kind,
                    stable_key_text: interner.resolve(row.stable_key).to_string(),
                    status: row.status,
                })
                .collect(),
            resolutions: output
                .resolutions
                .iter()
                .map(|row| CachedResolutionFact {
                    id: row.id,
                    language: row.language,
                    file: row.file,
                    package: row.package,
                    module: row.module,
                    source_stable_key_text: interner.resolve(row.source_stable_key).to_string(),
                    target_stable_key_texts: row
                        .target_stable_keys
                        .iter()
                        .map(|key| interner.resolve(*key).to_string())
                        .collect(),
                    step: row.step,
                    stable_key_text: interner.resolve(row.stable_key).to_string(),
                    status: row.status,
                })
                .collect(),
            generated_symbols: output
                .generated_symbols
                .iter()
                .map(|row| CachedGeneratedSymbolFact {
                    id: row.id,
                    language: row.language,
                    file: row.file,
                    package: row.package,
                    module: row.module,
                    symbol_stable_key_text: interner.resolve(row.symbol_stable_key).to_string(),
                    source_stable_key_text: interner.resolve(row.source_stable_key).to_string(),
                    producer_id: row.producer_id.clone(),
                    generator: row.generator.clone(),
                    generated_discriminator: row.generated_discriminator.clone(),
                    kind: row.kind,
                    span: row.span.clone(),
                    stable_key_text: interner.resolve(row.stable_key).to_string(),
                    status: row.status,
                })
                .collect(),
            stable_exports: output
                .stable_exports
                .iter()
                .map(|row| CachedStableExportIdentity {
                    id: row.id,
                    export: row.export,
                    language: row.language,
                    package_key: row.package_key.clone(),
                    module_key: row.module_key.clone(),
                    export_name: row.export_name.clone(),
                    namespace: row.namespace,
                    symbol_stable_key_text: interner.resolve(row.symbol_stable_key).to_string(),
                    generated_discriminator: row.generated_discriminator.clone(),
                    stable_key_text: interner.resolve(row.stable_key).to_string(),
                    status: row.status,
                })
                .collect(),
        }
    }

    pub fn into_output(
        self,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> SemanticIndexOutput {
        SemanticIndexOutput {
            scopes: self
                .scopes
                .into_iter()
                .map(|row| ScopeFact {
                    id: row.id,
                    language: row.language,
                    file: row.file,
                    package: row.package,
                    module: row.module,
                    parent: row.parent,
                    scope_path: row.scope_path,
                    kind: row.kind,
                    stable_key: interner.intern(row.stable_key_text),
                    status: row.status,
                })
                .collect(),
            semantic_imports: self
                .semantic_imports
                .into_iter()
                .map(|row| SemanticImportFact {
                    id: row.id,
                    language: row.language,
                    file: row.file,
                    package: row.package,
                    module: row.module,
                    scope: row.scope,
                    import_path: row.import_path,
                    local_name: row.local_name,
                    imported_name: row.imported_name,
                    namespace: row.namespace,
                    kind: row.kind,
                    stable_key: interner.intern(row.stable_key_text),
                    status: row.status,
                })
                .collect(),
            exports: self
                .exports
                .into_iter()
                .map(|row| ExportFact {
                    id: row.id,
                    language: row.language,
                    file: row.file,
                    package: row.package,
                    module: row.module,
                    scope: row.scope,
                    symbol: row.symbol,
                    export_name: row.export_name,
                    namespace: row.namespace,
                    kind: row.kind,
                    stable_key: interner.intern(row.stable_key_text),
                    status: row.status,
                })
                .collect(),
            aliases: self
                .aliases
                .into_iter()
                .map(|row| AliasFact {
                    id: row.id,
                    language: row.language,
                    file: row.file,
                    package: row.package,
                    module: row.module,
                    source_symbol_stable_key: interner.intern(row.source_symbol_stable_key_text),
                    target_symbol_stable_keys: row
                        .target_symbol_stable_key_texts
                        .into_iter()
                        .map(|key| interner.intern(key))
                        .collect(),
                    kind: row.kind,
                    stable_key: interner.intern(row.stable_key_text),
                    status: row.status,
                })
                .collect(),
            resolutions: self
                .resolutions
                .into_iter()
                .map(|row| ResolutionFact {
                    id: row.id,
                    language: row.language,
                    file: row.file,
                    package: row.package,
                    module: row.module,
                    source_stable_key: interner.intern(row.source_stable_key_text),
                    target_stable_keys: row
                        .target_stable_key_texts
                        .into_iter()
                        .map(|key| interner.intern(key))
                        .collect(),
                    step: row.step,
                    stable_key: interner.intern(row.stable_key_text),
                    status: row.status,
                })
                .collect(),
            generated_symbols: self
                .generated_symbols
                .into_iter()
                .map(|row| GeneratedSymbolFact {
                    id: row.id,
                    language: row.language,
                    file: row.file,
                    package: row.package,
                    module: row.module,
                    symbol_stable_key: interner.intern(row.symbol_stable_key_text),
                    source_stable_key: interner.intern(row.source_stable_key_text),
                    producer_id: row.producer_id,
                    generator: row.generator,
                    generated_discriminator: row.generated_discriminator,
                    kind: row.kind,
                    span: row.span,
                    stable_key: interner.intern(row.stable_key_text),
                    status: row.status,
                })
                .collect(),
            stable_exports: self
                .stable_exports
                .into_iter()
                .map(|row| StableExportIdentity {
                    id: row.id,
                    export: row.export,
                    language: row.language,
                    package_key: row.package_key,
                    module_key: row.module_key,
                    export_name: row.export_name,
                    namespace: row.namespace,
                    symbol_stable_key: interner.intern(row.symbol_stable_key_text),
                    generated_discriminator: row.generated_discriminator,
                    stable_key: interner.intern(row.stable_key_text),
                    status: row.status,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AliasReexportClosureOutput {
    pub aliases: Vec<AliasFact>,
    pub resolutions: Vec<ResolutionFact>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeGeneratedHooksOutput {
    pub generated_symbols: Vec<GeneratedSymbolFact>,
    pub resolutions: Vec<ResolutionFact>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Default)]
pub struct SemanticIndexBuilder {
    scopes: Vec<ScopeFact>,
    semantic_imports: Vec<SemanticImportFact>,
    exports: Vec<ExportFact>,
    aliases: Vec<AliasFact>,
    resolutions: Vec<ResolutionFact>,
    generated_symbols: Vec<GeneratedSymbolFact>,
    stable_exports: Vec<StableExportIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ScopeKind {
    Workspace,
    Package,
    Module,
    File,
    Class,
    Function,
    Method,
    Block,
    Catch,
    Loop,
    Switch,
    Type,
    Namespace,
    Generated,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ExportKind {
    Named,
    Default,
    Namespace,
    StarReexport,
    CommonJsModuleExports,
    CommonJsExportsProperty,
    ReExport,
    Star,
    Generated,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AliasKind {
    Import,
    ImportAlias,
    Export,
    ReExport,
    Generated,
    TypeOnly,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ResolutionStepKind {
    LexicalLookup,
    ImportAliasLookup,
    ModuleLookup,
    MemberLookup,
    GeneratedHintLookup,
    UnknownFallback,
    Lexical,
    ImportAlias,
    ExportAlias,
    Package,
    Module,
    Member,
    Generated,
    External,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GeneratedSymbolKind {
    FrameworkEntrypoint,
    Route,
    TestHarness,
    BuildGenerated,
    ExtensionProvided,
    Unknown,
}

impl SemanticIndexBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_scope(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        mut fact: ScopeFact,
    ) -> ScopeId {
        let id = ScopeId(self.scopes.len() as u64);
        fact.id = id;
        if interner.resolve(fact.stable_key).is_empty() {
            fact.stable_key = fact.computed_stable_key(interner);
        }
        self.scopes.push(fact);
        id
    }

    pub fn add_semantic_import(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        mut fact: SemanticImportFact,
    ) -> SemanticImportId {
        let id = SemanticImportId(self.semantic_imports.len() as u64);
        fact.id = id;
        if interner.resolve(fact.stable_key).is_empty() {
            fact.stable_key = semantic_import_computed_stable_key(&fact, interner);
        }
        self.semantic_imports.push(fact);
        id
    }

    pub fn add_export_identity(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        mut fact: ExportFact,
    ) -> ExportId {
        let id = ExportId(self.exports.len() as u64);
        fact.id = id;
        if interner.resolve(fact.stable_key).is_empty() {
            fact.stable_key = fact.computed_stable_key(interner);
        }
        self.exports.push(fact);
        id
    }

    pub fn add_alias(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        mut fact: AliasFact,
    ) -> AliasId {
        let id = AliasId(self.aliases.len() as u64);
        fact.id = id;
        if interner.resolve(fact.stable_key).is_empty() {
            fact.stable_key = fact.computed_stable_key(interner);
        }
        self.aliases.push(fact);
        id
    }

    pub fn add_resolution(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        mut fact: ResolutionFact,
    ) -> ResolutionId {
        let id = ResolutionId(self.resolutions.len() as u64);
        fact.id = id;
        if interner.resolve(fact.stable_key).is_empty() {
            fact.stable_key = fact.computed_stable_key(interner);
        }
        self.resolutions.push(fact);
        id
    }

    #[cfg(test)]
    pub fn add_generated_symbol(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        mut fact: GeneratedSymbolFact,
    ) -> GeneratedSymbolId {
        let id = GeneratedSymbolId(self.generated_symbols.len() as u64);
        fact.id = id;
        if interner.resolve(fact.stable_key).is_empty() {
            fact.stable_key = fact.computed_stable_key(interner);
        }
        self.generated_symbols.push(fact);
        id
    }

    pub fn add_stable_export(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        mut fact: StableExportIdentity,
    ) -> StableExportId {
        let id = StableExportId(self.stable_exports.len() as u64);
        fact.id = id;
        if interner.resolve(fact.stable_key).is_empty() {
            fact.stable_key = fact.computed_stable_key(interner);
        }
        self.stable_exports.push(fact);
        id
    }

    pub fn finish(self, interner: &crate::internal_core::StableKeyInterner) -> SemanticIndexOutput {
        SemanticIndexOutput {
            scopes: sort_rows(self.scopes, |row| scope_sort_key(interner, row)),
            semantic_imports: sort_rows(self.semantic_imports, |row| {
                semantic_import_sort_key(interner, row)
            }),
            exports: sort_rows(self.exports, |row| export_sort_key(interner, row)),
            aliases: sort_rows(self.aliases, |row| alias_sort_key(interner, row)),
            resolutions: sort_rows(self.resolutions, |row| resolution_sort_key(interner, row)),
            generated_symbols: sort_rows(self.generated_symbols, |row| {
                generated_symbol_sort_key(interner, row)
            }),
            stable_exports: sort_rows(self.stable_exports, |row| {
                stable_export_sort_key(interner, row)
            }),
        }
    }
}

impl SemanticIndexOutput {
    pub fn extend(&mut self, other: Self) {
        self.scopes.extend(other.scopes);
        self.semantic_imports.extend(other.semantic_imports);
        self.exports.extend(other.exports);
        self.aliases.extend(other.aliases);
        self.resolutions.extend(other.resolutions);
        self.generated_symbols.extend(other.generated_symbols);
        self.stable_exports.extend(other.stable_exports);
    }
}

pub fn alias_reexport_closure(
    interner: &crate::internal_core::StableKeyInterner,
    aliases: &[AliasFact],
    exports: &[ExportFact],
    stable_exports: &[StableExportIdentity],
) -> AliasReexportClosureOutput {
    let max_iterations = aliases.len() + exports.len() + stable_exports.len() + 1;
    let alias_targets = aliases
        .iter()
        .map(|alias| {
            (
                alias.source_symbol_stable_key,
                alias.target_symbol_stable_keys.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut closure_aliases = BTreeMap::<StableKeyId, AliasFact>::new();
    let mut closure_resolutions = BTreeMap::<StableKeyId, ResolutionFact>::new();

    for alias in aliases {
        let (targets, status) =
            resolve_alias_targets(interner, alias, &alias_targets, max_iterations);
        let mut closure = alias.clone();
        closure.target_symbol_stable_keys = sorted_stable_key_ids(interner, targets);
        closure.status = closure_status(alias.status, status);
        closure.stable_key = closure_alias_stable_key(interner, alias, &closure);
        let resolution = closure_resolution_for_alias(interner, &closure);
        closure_resolutions.insert(resolution.stable_key, resolution);
        closure_aliases.insert(closure.stable_key, closure);
    }

    let stable_export_targets = stable_exports
        .iter()
        .map(|export| export.symbol_stable_key)
        .collect::<BTreeSet<_>>();
    let stable_export_targets = sorted_stable_key_ids(interner, stable_export_targets);
    for export in exports
        .iter()
        .filter(|export| matches!(export.kind, ExportKind::Star | ExportKind::StarReexport))
    {
        let mut closure = AliasFact {
            id: AliasId(0),
            language: export.language,
            file: export.file,
            package: export.package,
            module: export.module,
            source_symbol_stable_key: export.stable_key,
            target_symbol_stable_keys: stable_export_targets.clone(),
            kind: AliasKind::ReExport,
            stable_key: interner.intern(""),
            status: SemanticStatus::Ambiguous,
        };
        closure.stable_key = closure.computed_stable_key(interner);
        let resolution = closure_resolution_for_alias(interner, &closure);
        closure_resolutions.insert(resolution.stable_key, resolution);
        closure_aliases.insert(closure.stable_key, closure);
    }

    AliasReexportClosureOutput {
        aliases: sort_rows(closure_aliases.into_values().collect(), |row| {
            alias_sort_key(interner, row)
        }),
        resolutions: sort_rows(closure_resolutions.into_values().collect(), |row| {
            resolution_sort_key(interner, row)
        }),
    }
}

pub fn emit_native_generated_symbol_hooks(
    interner: &crate::internal_core::StableKeyInterner,
    semantic: &SemanticIndexOutput,
) -> NativeGeneratedHooksOutput {
    let mut generated = BTreeMap::<StableKeyId, GeneratedSymbolFact>::new();
    let mut resolutions = BTreeMap::<StableKeyId, ResolutionFact>::new();
    let mut diagnostics = BTreeMap::<StableKeyId, Diagnostic>::new();

    for stable_export in &semantic.stable_exports {
        let Some(discriminator) = stable_export.generated_discriminator.as_ref() else {
            continue;
        };
        let mut row = GeneratedSymbolFact {
            id: GeneratedSymbolId(0),
            language: stable_export.language,
            file: None,
            package: None,
            module: None,
            symbol_stable_key: stable_export.symbol_stable_key,
            source_stable_key: stable_export.stable_key,
            producer_id: SYMBOL_GRAPH_PRODUCER_ID.to_string(),
            generator: SYMBOL_GRAPH_PRODUCER_ID.to_string(),
            generated_discriminator: discriminator.clone(),
            kind: GeneratedSymbolKind::Unknown,
            span: None,
            stable_key: interner.intern(""),
            status: SemanticStatus::Generated,
        };
        row.stable_key = row.computed_stable_key(interner);

        if let Some(existing) = generated.get(&row.stable_key) {
            if existing != &row {
                diagnostics.entry(row.stable_key).or_insert_with(|| {
                    generated_symbol_conflict_diagnostic(&interner.resolve(row.stable_key))
                });
            }
            continue;
        }

        let resolution = generated_hint_resolution(interner, stable_export, &row);
        resolutions.insert(resolution.stable_key, resolution);
        generated.insert(row.stable_key, row);
    }

    NativeGeneratedHooksOutput {
        generated_symbols: sort_rows(generated.into_values().collect(), |row| {
            generated_symbol_sort_key(interner, row)
        }),
        resolutions: sort_rows(resolutions.into_values().collect(), |row| {
            resolution_sort_key(interner, row)
        }),
        diagnostics: diagnostics.into_values().collect(),
    }
}

fn generated_hint_resolution(
    interner: &crate::internal_core::StableKeyInterner,
    stable_export: &StableExportIdentity,
    generated: &GeneratedSymbolFact,
) -> ResolutionFact {
    let mut resolution = ResolutionFact {
        id: ResolutionId(0),
        language: stable_export.language,
        file: None,
        package: None,
        module: None,
        source_stable_key: stable_export.stable_key,
        target_stable_keys: vec![generated.stable_key],
        step: ResolutionStepKind::GeneratedHintLookup,
        stable_key: interner.intern(""),
        status: SemanticStatus::Generated,
    };
    resolution.stable_key = resolution.computed_stable_key(interner);
    resolution
}

fn generated_symbol_conflict_diagnostic(stable_key: &str) -> Diagnostic {
    Diagnostic::error(
        "polint/internal",
        "<workspace>",
        TextRange::point(1, 1),
        "Generated symbol hook stable key has incompatible native rows.",
    )
    .with_evidence("family", FactFamily::GeneratedSymbol.label().to_string())
    .with_evidence("stable_key", stable_key.to_string())
    .with_evidence("reason", "duplicate_generated_stable_key".to_string())
}

fn resolve_alias_targets(
    interner: &crate::internal_core::StableKeyInterner,
    alias: &AliasFact,
    alias_targets: &BTreeMap<StableKeyId, Vec<StableKeyId>>,
    max_iterations: usize,
) -> (BTreeSet<StableKeyId>, SemanticStatus) {
    if !matches!(
        alias.status,
        SemanticStatus::Resolved | SemanticStatus::Generated
    ) {
        return (
            alias.target_symbol_stable_keys.iter().cloned().collect(),
            alias.status,
        );
    }

    let mut terminals = BTreeSet::new();
    let mut stack = alias
        .target_symbol_stable_keys
        .iter()
        .map(|target| (*target, 0usize))
        .collect::<Vec<_>>();
    let mut seen_edges = BTreeSet::<(String, String)>::new();
    let mut status = SemanticStatus::Resolved;

    while let Some((target, depth)) = stack.pop() {
        if depth >= max_iterations || target == alias.source_symbol_stable_key {
            status = SemanticStatus::Cycle;
            continue;
        }
        if let Some(next_targets) = alias_targets.get(&target) {
            for next in next_targets {
                let edge = (
                    interner.resolve(target).to_string(),
                    interner.resolve(*next).to_string(),
                );
                if !seen_edges.insert(edge) {
                    status = SemanticStatus::Cycle;
                    continue;
                }
                stack.push((*next, depth + 1));
            }
        } else {
            terminals.insert(target);
        }
    }

    (terminals, status)
}

fn closure_status(original: SemanticStatus, resolved: SemanticStatus) -> SemanticStatus {
    if resolved == SemanticStatus::Cycle {
        SemanticStatus::Cycle
    } else {
        original
    }
}

fn closure_alias_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    original: &AliasFact,
    closure: &AliasFact,
) -> StableKeyId {
    stable_key_from_key_parts(
        interner,
        FactFamily::Alias,
        [
            ("base_alias", KeyPart::Key(original.stable_key)),
            (
                "targets",
                KeyPart::Text(&sorted_stable_key_value(
                    interner,
                    &closure.target_symbol_stable_keys,
                )),
            ),
            (
                "status",
                KeyPart::Text(semantic_status_label(closure.status)),
            ),
        ],
    )
}

fn closure_resolution_for_alias(
    interner: &crate::internal_core::StableKeyInterner,
    alias: &AliasFact,
) -> ResolutionFact {
    let mut resolution = ResolutionFact {
        id: ResolutionId(0),
        language: alias.language,
        file: alias.file,
        package: alias.package,
        module: alias.module,
        source_stable_key: alias.source_symbol_stable_key,
        target_stable_keys: alias.target_symbol_stable_keys.clone(),
        step: ResolutionStepKind::ImportAliasLookup,
        stable_key: interner.intern(""),
        status: alias.status,
    };
    resolution.stable_key = resolution.computed_stable_key(interner);
    resolution
}

impl ScopeFact {
    pub fn computed_stable_key(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> StableKeyId {
        Self::stable_key_for(
            interner,
            self.language,
            &self.scope_path,
            self.file.map(file_id_key),
            self.package.map(package_id_key),
            self.module.map(module_node_id_key),
            (self.kind, self.status),
        )
    }

    pub fn stable_key_for(
        interner: &crate::internal_core::StableKeyInterner,
        language: Language,
        scope_path: &[String],
        file_key: Option<String>,
        package_key: Option<String>,
        module_key: Option<String>,
        pair: (ScopeKind, SemanticStatus),
    ) -> StableKeyId {
        let (kind, status) = pair;
        stable_key_from_parts(
            interner,
            FactFamily::Scope,
            &[
                ("language", language_label(language).to_string()),
                ("scope_path", sorted_repeated_value(scope_path)),
                ("file", option_key(file_key)),
                ("package", option_key(package_key)),
                ("module", option_key(module_key)),
                ("kind", scope_kind_label(kind).to_string()),
                ("status", semantic_status_label(status).to_string()),
            ],
        )
    }
}

pub fn semantic_import_computed_stable_key(
    fact: &SemanticImportFact,
    interner: &crate::internal_core::StableKeyInterner,
) -> StableKeyId {
    stable_key_from_parts(
        interner,
        FactFamily::SemanticImport,
        &[
            ("language", language_label(fact.language).to_string()),
            (
                "file",
                fact.file.map(file_id_key).unwrap_or_else(none_value),
            ),
            (
                "package",
                fact.package.map(package_id_key).unwrap_or_else(none_value),
            ),
            (
                "module",
                fact.module
                    .map(module_node_id_key)
                    .unwrap_or_else(none_value),
            ),
            ("name", option_key(fact.local_name.clone())),
            ("import_path", fact.import_path.clone()),
            ("imported_name", option_key(fact.imported_name.clone())),
            ("namespace", namespace_label(fact.namespace).to_string()),
            ("kind", semantic_import_kind_label(fact.kind).to_string()),
            ("status", semantic_status_label(fact.status).to_string()),
        ],
    )
}

impl ExportFact {
    pub fn computed_stable_key(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> StableKeyId {
        stable_key_from_parts(
            interner,
            FactFamily::Export,
            &[
                ("language", language_label(self.language).to_string()),
                (
                    "file",
                    self.file.map(file_id_key).unwrap_or_else(none_value),
                ),
                (
                    "package",
                    self.package.map(package_id_key).unwrap_or_else(none_value),
                ),
                (
                    "module",
                    self.module
                        .map(module_node_id_key)
                        .unwrap_or_else(none_value),
                ),
                ("export_name", self.export_name.clone()),
                ("namespace", namespace_label(self.namespace).to_string()),
                ("kind", export_kind_label(self.kind).to_string()),
                ("status", semantic_status_label(self.status).to_string()),
            ],
        )
    }
}

impl AliasFact {
    pub fn computed_stable_key(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> StableKeyId {
        stable_key_from_key_parts(
            interner,
            FactFamily::Alias,
            [
                ("language", KeyPart::Text(language_label(self.language))),
                (
                    "file",
                    KeyPart::Text(&self.file.map(file_id_key).unwrap_or_else(none_value)),
                ),
                (
                    "package",
                    KeyPart::Text(&self.package.map(package_id_key).unwrap_or_else(none_value)),
                ),
                (
                    "module",
                    KeyPart::Text(
                        &self
                            .module
                            .map(module_node_id_key)
                            .unwrap_or_else(none_value),
                    ),
                ),
                (
                    "symbol_stable_key",
                    KeyPart::Key(self.source_symbol_stable_key),
                ),
                (
                    "target_symbol_stable_keys",
                    KeyPart::Text(&sorted_stable_key_value(
                        interner,
                        &self.target_symbol_stable_keys,
                    )),
                ),
                ("kind", KeyPart::Text(alias_kind_label(self.kind))),
                ("status", KeyPart::Text(semantic_status_label(self.status))),
            ],
        )
    }
}

impl ResolutionFact {
    pub fn computed_stable_key(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> StableKeyId {
        stable_key_from_key_parts(
            interner,
            FactFamily::Resolution,
            [
                ("language", KeyPart::Text(language_label(self.language))),
                (
                    "file",
                    KeyPart::Text(&self.file.map(file_id_key).unwrap_or_else(none_value)),
                ),
                (
                    "package",
                    KeyPart::Text(&self.package.map(package_id_key).unwrap_or_else(none_value)),
                ),
                (
                    "module",
                    KeyPart::Text(
                        &self
                            .module
                            .map(module_node_id_key)
                            .unwrap_or_else(none_value),
                    ),
                ),
                ("symbol_stable_key", KeyPart::Key(self.source_stable_key)),
                (
                    "target_stable_keys",
                    KeyPart::Text(&sorted_stable_key_value(interner, &self.target_stable_keys)),
                ),
                ("kind", KeyPart::Text(resolution_step_kind_label(self.step))),
                ("status", KeyPart::Text(semantic_status_label(self.status))),
            ],
        )
    }
}

impl GeneratedSymbolFact {
    pub fn computed_stable_key(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> StableKeyId {
        stable_key_from_key_parts(
            interner,
            FactFamily::GeneratedSymbol,
            [
                ("language", KeyPart::Text(language_label(self.language))),
                (
                    "file",
                    KeyPart::Text(&self.file.map(file_id_key).unwrap_or_else(none_value)),
                ),
                (
                    "package",
                    KeyPart::Text(&self.package.map(package_id_key).unwrap_or_else(none_value)),
                ),
                (
                    "module",
                    KeyPart::Text(
                        &self
                            .module
                            .map(module_node_id_key)
                            .unwrap_or_else(none_value),
                    ),
                ),
                ("symbol_stable_key", KeyPart::Key(self.symbol_stable_key)),
                ("source_stable_key", KeyPart::Key(self.source_stable_key)),
                ("producer_id", KeyPart::Text(&self.producer_id.clone())),
                (
                    "generated_discriminator",
                    KeyPart::Text(&self.generated_discriminator.clone()),
                ),
                (
                    "kind",
                    KeyPart::Text(generated_symbol_kind_label(self.kind)),
                ),
                ("status", KeyPart::Text(semantic_status_label(self.status))),
            ],
        )
    }
}

impl StableExportIdentity {
    pub fn computed_stable_key(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> StableKeyId {
        stable_key_from_key_parts(
            interner,
            FactFamily::StableExport,
            [
                ("language", KeyPart::Text(language_label(self.language))),
                (
                    "package",
                    KeyPart::Text(&option_key(self.package_key.clone())),
                ),
                (
                    "module",
                    KeyPart::Text(&option_key(self.module_key.clone())),
                ),
                ("export_name", KeyPart::Text(&self.export_name.clone())),
                ("namespace", KeyPart::Text(namespace_label(self.namespace))),
                ("symbol_stable_key", KeyPart::Key(self.symbol_stable_key)),
                (
                    "generated_discriminator",
                    KeyPart::Text(&option_key(self.generated_discriminator.clone())),
                ),
                ("status", KeyPart::Text(semantic_status_label(self.status))),
            ],
        )
    }
}

fn sorted_repeated_value(values: &[String]) -> String {
    let mut values = values
        .iter()
        .map(|value| value.replace('\\', "/"))
        .collect::<Vec<_>>();
    values.sort();
    values
        .into_iter()
        .map(|value| format!("{}:{value}", value.len()))
        .collect::<Vec<_>>()
        .join(",")
}

fn sorted_stable_key_value(
    interner: &crate::internal_core::StableKeyInterner,
    values: &[StableKeyId],
) -> String {
    let mut values = values
        .iter()
        .map(|value| interner.resolve(*value).replace('\\', "/"))
        .collect::<Vec<_>>();
    values.sort();
    values
        .into_iter()
        .map(|value| format!("{}:{value}", value.len()))
        .collect::<Vec<_>>()
        .join(",")
}

fn sorted_stable_key_ids(
    interner: &crate::internal_core::StableKeyInterner,
    values: BTreeSet<StableKeyId>,
) -> Vec<StableKeyId> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort_by_key(|value| interner.resolve(*value));
    values
}

fn option_key(value: Option<String>) -> String {
    value.unwrap_or_else(none_value)
}

fn file_id_key(id: FileId) -> String {
    id.0.to_string()
}

fn package_id_key(id: PackageId) -> String {
    id.0.to_string()
}

fn module_node_id_key(id: ModuleNodeId) -> String {
    id.0.to_string()
}

fn none_value() -> String {
    "<none>".to_string()
}

fn language_label(language: Language) -> &'static str {
    match language {
        Language::Go => "go",
        Language::TypeScript => "typescript",
        Language::Tsx => "tsx",
        Language::JavaScript => "javascript",
        Language::Jsx => "jsx",
        Language::Unknown => "unknown",
        _ => "unknown",
    }
}

fn namespace_label(namespace: SymbolNamespace) -> &'static str {
    match namespace {
        SymbolNamespace::Value => "value",
        SymbolNamespace::Type => "type",
        SymbolNamespace::Namespace => "namespace",
        SymbolNamespace::Package => "package",
        SymbolNamespace::Module => "module",
        SymbolNamespace::Unknown => "unknown",
        _ => "unknown",
    }
}

fn scope_kind_label(kind: ScopeKind) -> &'static str {
    match kind {
        ScopeKind::Workspace => "workspace",
        ScopeKind::Package => "package",
        ScopeKind::Module => "module",
        ScopeKind::File => "file",
        ScopeKind::Class => "class",
        ScopeKind::Function => "function",
        ScopeKind::Method => "method",
        ScopeKind::Block => "block",
        ScopeKind::Catch => "catch",
        ScopeKind::Loop => "loop",
        ScopeKind::Switch => "switch",
        ScopeKind::Type => "type",
        ScopeKind::Namespace => "namespace",
        ScopeKind::Generated => "generated",
        ScopeKind::Unknown => "unknown",
    }
}

fn sort_rows<T, K: Ord>(mut rows: Vec<T>, key: impl Fn(&T) -> K) -> Vec<T> {
    rows.sort_by_key(key);
    rows
}

fn scope_sort_key(
    interner: &crate::internal_core::StableKeyInterner,
    fact: &ScopeFact,
) -> (String, Vec<String>, Option<FileId>, ScopeKind) {
    (
        interner.resolve(fact.stable_key).to_string(),
        fact.scope_path.clone(),
        fact.file,
        fact.kind,
    )
}

fn semantic_import_sort_key(
    interner: &crate::internal_core::StableKeyInterner,
    fact: &SemanticImportFact,
) -> (
    String,
    Option<FileId>,
    String,
    Option<String>,
    SemanticImportKind,
    SemanticStatus,
) {
    (
        interner.resolve(fact.stable_key).to_string(),
        fact.file,
        fact.import_path.clone(),
        fact.local_name.clone(),
        fact.kind,
        fact.status,
    )
}

fn export_sort_key(
    interner: &crate::internal_core::StableKeyInterner,
    fact: &ExportFact,
) -> (
    String,
    Option<FileId>,
    String,
    String,
    ExportKind,
    SemanticStatus,
) {
    (
        interner.resolve(fact.stable_key).to_string(),
        fact.file,
        fact.export_name.clone(),
        namespace_label(fact.namespace).to_string(),
        fact.kind,
        fact.status,
    )
}

fn alias_sort_key(
    interner: &crate::internal_core::StableKeyInterner,
    fact: &AliasFact,
) -> (
    String,
    Option<FileId>,
    String,
    Vec<String>,
    AliasKind,
    SemanticStatus,
) {
    (
        interner.resolve(fact.stable_key).to_string(),
        fact.file,
        interner.resolve(fact.source_symbol_stable_key).to_string(),
        fact.target_symbol_stable_keys
            .iter()
            .map(|key| interner.resolve(*key).to_string())
            .collect(),
        fact.kind,
        fact.status,
    )
}

fn resolution_sort_key(
    interner: &crate::internal_core::StableKeyInterner,
    fact: &ResolutionFact,
) -> (
    String,
    Option<FileId>,
    String,
    Vec<String>,
    ResolutionStepKind,
    SemanticStatus,
) {
    (
        interner.resolve(fact.stable_key).to_string(),
        fact.file,
        interner.resolve(fact.source_stable_key).to_string(),
        fact.target_stable_keys
            .iter()
            .map(|key| interner.resolve(*key).to_string())
            .collect(),
        fact.step,
        fact.status,
    )
}

fn generated_symbol_sort_key(
    interner: &crate::internal_core::StableKeyInterner,
    fact: &GeneratedSymbolFact,
) -> (
    String,
    Option<FileId>,
    String,
    String,
    GeneratedSymbolKind,
    SemanticStatus,
) {
    (
        interner.resolve(fact.stable_key).to_string(),
        fact.file,
        interner.resolve(fact.symbol_stable_key).to_string(),
        fact.generated_discriminator.clone(),
        fact.kind,
        fact.status,
    )
}

type StableExportSortKey = (
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
    Option<String>,
    SemanticStatus,
);

fn stable_export_sort_key(
    interner: &crate::internal_core::StableKeyInterner,
    fact: &StableExportIdentity,
) -> StableExportSortKey {
    (
        interner.resolve(fact.stable_key).to_string(),
        fact.package_key.clone(),
        fact.module_key.clone(),
        fact.export_name.clone(),
        namespace_label(fact.namespace).to_string(),
        interner.resolve(fact.symbol_stable_key).to_string(),
        fact.generated_discriminator.clone(),
        fact.status,
    )
}

fn semantic_import_kind_label(kind: SemanticImportKind) -> &'static str {
    match kind {
        SemanticImportKind::StaticNamed => "static_named",
        SemanticImportKind::StaticDefault => "static_default",
        SemanticImportKind::StaticNamespace => "static_namespace",
        SemanticImportKind::SideEffect => "side_effect",
        SemanticImportKind::TypeOnly => "type_only",
        SemanticImportKind::CommonJsRequire => "commonjs_require",
        SemanticImportKind::DynamicImport => "dynamic_import",
        SemanticImportKind::GoDefault => "go_default",
        SemanticImportKind::GoNamed => "go_named",
        SemanticImportKind::GoDot => "go_dot",
        SemanticImportKind::GoBlank => "go_blank",
        SemanticImportKind::GoImplicit => "go_implicit",
        SemanticImportKind::EsNamed => "es_named",
        SemanticImportKind::EsDefault => "es_default",
        SemanticImportKind::EsNamespace => "es_namespace",
        SemanticImportKind::ReExport => "re_export",
        SemanticImportKind::CommonJs => "common_js",
        SemanticImportKind::Dynamic => "dynamic",
        SemanticImportKind::Unknown => "unknown",
    }
}

fn export_kind_label(kind: ExportKind) -> &'static str {
    match kind {
        ExportKind::Named => "named",
        ExportKind::Default => "default",
        ExportKind::Namespace => "namespace",
        ExportKind::StarReexport => "star_reexport",
        ExportKind::CommonJsModuleExports => "commonjs_module_exports",
        ExportKind::CommonJsExportsProperty => "commonjs_exports_property",
        ExportKind::ReExport => "re_export",
        ExportKind::Star => "star",
        ExportKind::Generated => "generated",
        ExportKind::Unknown => "unknown",
    }
}

fn alias_kind_label(kind: AliasKind) -> &'static str {
    match kind {
        AliasKind::Import => "import",
        AliasKind::ImportAlias => "import_alias",
        AliasKind::Export => "export",
        AliasKind::ReExport => "re_export",
        AliasKind::Generated => "generated",
        AliasKind::TypeOnly => "type_only",
        AliasKind::Unknown => "unknown",
    }
}

fn resolution_step_kind_label(kind: ResolutionStepKind) -> &'static str {
    match kind {
        ResolutionStepKind::LexicalLookup => "LexicalLookup",
        ResolutionStepKind::ImportAliasLookup => "ImportAliasLookup",
        ResolutionStepKind::ModuleLookup => "ModuleLookup",
        ResolutionStepKind::MemberLookup => "MemberLookup",
        ResolutionStepKind::GeneratedHintLookup => "GeneratedHintLookup",
        ResolutionStepKind::UnknownFallback => "UnknownFallback",
        ResolutionStepKind::Lexical => "lexical",
        ResolutionStepKind::ImportAlias => "import_alias",
        ResolutionStepKind::ExportAlias => "export_alias",
        ResolutionStepKind::Package => "package",
        ResolutionStepKind::Module => "module",
        ResolutionStepKind::Member => "member",
        ResolutionStepKind::Generated => "generated",
        ResolutionStepKind::External => "external",
        ResolutionStepKind::Unsupported => "unsupported",
    }
}

fn semantic_status_label(status: SemanticStatus) -> &'static str {
    match status {
        SemanticStatus::Resolved => "resolved",
        SemanticStatus::Ambiguous => "ambiguous",
        SemanticStatus::Unresolved => "unresolved",
        SemanticStatus::Cycle => "cycle",
        SemanticStatus::Generated => "generated",
        SemanticStatus::Dynamic => "dynamic",
        SemanticStatus::External => "external",
        SemanticStatus::SetupMissing => "setup_missing",
        SemanticStatus::Unsupported => "unsupported",
    }
}

fn generated_symbol_kind_label(kind: GeneratedSymbolKind) -> &'static str {
    match kind {
        GeneratedSymbolKind::FrameworkEntrypoint => "framework_entrypoint",
        GeneratedSymbolKind::Route => "route",
        GeneratedSymbolKind::TestHarness => "test_harness",
        GeneratedSymbolKind::BuildGenerated => "build_generated",
        GeneratedSymbolKind::ExtensionProvided => "extension_provided",
        GeneratedSymbolKind::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::SymbolNamespace;
    use crate::internal_core::{
        FileId, Language, ModuleNodeId, PackageId, stable_key_for_test, test_stable_key_interner,
    };

    #[test]
    fn scope_stable_keys_are_deterministic_across_input_order() {
        let interner = test_stable_key_interner();
        let first = ScopeFact::stable_key_for(
            &interner,
            Language::TypeScript,
            &["module".to_string(), "function:handler".to_string()],
            Some("src/api.ts".to_string()),
            Some("pkg:web".to_string()),
            Some("mod:web".to_string()),
            (ScopeKind::Function, SemanticStatus::Resolved),
        );
        let second = ScopeFact::stable_key_for(
            &interner,
            Language::TypeScript,
            &["function:handler".to_string(), "module".to_string()],
            Some("src/api.ts".to_string()),
            Some("pkg:web".to_string()),
            Some("mod:web".to_string()),
            (ScopeKind::Function, SemanticStatus::Resolved),
        );

        assert_eq!(first, second);
    }

    #[test]
    fn stable_export_identity_key_includes_required_context() {
        let interner = test_stable_key_interner();
        let identity = StableExportIdentity {
            id: StableExportId(17),
            export: ExportId(3),
            language: Language::Go,
            package_key: Some("pkg:service".to_string()),
            module_key: Some("mod:service".to_string()),
            export_name: "Handler".to_string(),
            namespace: SymbolNamespace::Value,
            symbol_stable_key: interner.intern("symbol:handler"),
            generated_discriminator: Some("route:/users".to_string()),
            stable_key: interner.intern(""),
            status: SemanticStatus::Generated,
        };

        let stable_key = interner.resolve(identity.computed_stable_key(&interner));

        assert!(stable_key.contains("language"));
        assert!(stable_key.contains("package"));
        assert!(stable_key.contains("module"));
        assert!(stable_key.contains("export_name"));
        assert!(stable_key.contains("namespace"));
        assert!(stable_key.contains("symbol_stable_key"));
        assert!(stable_key.contains("generated_discriminator"));
    }

    #[test]
    fn alias_status_supports_all_closure_outcomes() {
        let statuses = [
            SemanticStatus::Resolved,
            SemanticStatus::Ambiguous,
            SemanticStatus::Unresolved,
            SemanticStatus::Cycle,
            SemanticStatus::Generated,
            SemanticStatus::Dynamic,
            SemanticStatus::External,
            SemanticStatus::SetupMissing,
            SemanticStatus::Unsupported,
        ];

        let alias_count = statuses
            .into_iter()
            .enumerate()
            .map(|(index, status)| AliasFact {
                id: AliasId(index as u64),
                language: Language::JavaScript,
                file: Some(FileId::from_raw(0)),
                package: Some(PackageId::from_raw(1)),
                module: Some(ModuleNodeId::from_raw(2)),
                source_symbol_stable_key: stable_key_for_test(&format!("alias:{index}")),
                target_symbol_stable_keys: Vec::new(),
                kind: AliasKind::ReExport,
                stable_key: stable_key_for_test(&format!("alias-key:{index}")),
                status,
            })
            .count();

        assert_eq!(alias_count, 9);
    }

    #[test]
    fn semantic_index_builder_sorts_scope_rows_by_stable_key() {
        let interner = test_stable_key_interner();
        let mut builder = SemanticIndexBuilder::new();
        builder.add_scope(
            &interner,
            ScopeFact {
                id: ScopeId(0),
                language: Language::TypeScript,
                file: Some(FileId::from_raw(0)),
                package: None,
                module: None,
                parent: None,
                scope_path: vec!["src/app.ts".to_string(), "function@20-40".to_string()],
                kind: ScopeKind::Function,
                stable_key: interner.intern("z-scope"),
                status: SemanticStatus::Resolved,
            },
        );
        builder.add_scope(
            &interner,
            ScopeFact {
                id: ScopeId(0),
                language: Language::TypeScript,
                file: Some(FileId::from_raw(0)),
                package: None,
                module: None,
                parent: None,
                scope_path: vec!["src/app.ts".to_string(), "module@0-50".to_string()],
                kind: ScopeKind::Module,
                stable_key: interner.intern("a-scope"),
                status: SemanticStatus::Resolved,
            },
        );

        let output = builder.finish(&interner);

        assert_eq!(
            output
                .scopes
                .iter()
                .map(|scope| interner.resolve(scope.stable_key).to_string())
                .collect::<Vec<_>>(),
            vec!["a-scope".to_string(), "z-scope".to_string()]
        );
    }

    #[test]
    fn semantic_index_builder_collects_all_row_families() {
        let interner = test_stable_key_interner();
        let mut builder = SemanticIndexBuilder::new();
        builder.add_semantic_import(
            &interner,
            SemanticImportFact {
                id: SemanticImportId(0),
                language: Language::TypeScript,
                file: Some(FileId::from_raw(0)),
                package: None,
                module: None,
                scope: None,
                import_path: "./target".to_string(),
                local_name: Some("target".to_string()),
                imported_name: Some("default".to_string()),
                namespace: SymbolNamespace::Value,
                kind: SemanticImportKind::EsDefault,
                stable_key: interner.intern(""),
                status: SemanticStatus::Resolved,
            },
        );
        let export = builder.add_export_identity(
            &interner,
            ExportFact {
                id: ExportId(0),
                language: Language::TypeScript,
                file: Some(FileId::from_raw(0)),
                package: None,
                module: None,
                scope: None,
                symbol: None,
                export_name: "target".to_string(),
                namespace: SymbolNamespace::Value,
                kind: ExportKind::Default,
                stable_key: interner.intern(""),
                status: SemanticStatus::Resolved,
            },
        );
        builder.add_alias(
            &interner,
            AliasFact {
                id: AliasId(0),
                language: Language::TypeScript,
                file: Some(FileId::from_raw(0)),
                package: None,
                module: None,
                source_symbol_stable_key: interner.intern("alias:source"),
                target_symbol_stable_keys: vec![interner.intern("alias:target")],
                kind: AliasKind::Import,
                stable_key: interner.intern(""),
                status: SemanticStatus::Resolved,
            },
        );
        builder.add_resolution(
            &interner,
            ResolutionFact {
                id: ResolutionId(0),
                language: Language::TypeScript,
                file: Some(FileId::from_raw(0)),
                package: None,
                module: None,
                source_stable_key: interner.intern("ref:source"),
                target_stable_keys: vec![interner.intern("symbol:target")],
                step: ResolutionStepKind::Lexical,
                stable_key: interner.intern(""),
                status: SemanticStatus::Resolved,
            },
        );
        builder.add_generated_symbol(
            &interner,
            GeneratedSymbolFact {
                id: GeneratedSymbolId(0),
                language: Language::TypeScript,
                file: Some(FileId::from_raw(0)),
                package: None,
                module: None,
                symbol_stable_key: interner.intern("symbol:generated"),
                source_stable_key: interner.intern("stable-export:generated"),
                producer_id: "polint.symbol_graph".to_string(),
                generator: "test".to_string(),
                generated_discriminator: "native".to_string(),
                kind: GeneratedSymbolKind::Unknown,
                span: None,
                stable_key: interner.intern(""),
                status: SemanticStatus::Generated,
            },
        );
        builder.add_stable_export(
            &interner,
            StableExportIdentity {
                id: StableExportId(0),
                export,
                language: Language::TypeScript,
                package_key: None,
                module_key: Some("src/target.ts".to_string()),
                export_name: "target".to_string(),
                namespace: SymbolNamespace::Value,
                symbol_stable_key: interner.intern("symbol:target"),
                generated_discriminator: Some("native".to_string()),
                stable_key: interner.intern(""),
                status: SemanticStatus::Resolved,
            },
        );

        let output = builder.finish(&interner);

        assert_eq!(output.semantic_imports.len(), 1);
        assert_eq!(output.exports.len(), 1);
        assert_eq!(output.aliases.len(), 1);
        assert_eq!(output.resolutions.len(), 1);
        assert_eq!(output.generated_symbols.len(), 1);
        assert_eq!(output.stable_exports.len(), 1);
    }
}

#[cfg(test)]
mod alias_reexport_closure_tests {
    use super::*;
    use crate::analysis_api::SymbolNamespace;
    use crate::internal_core::{FileId, Language, test_stable_key_interner};

    fn alias(source: &str, targets: &[&str], status: SemanticStatus) -> AliasFact {
        let interner = test_stable_key_interner();
        let mut fact = AliasFact {
            id: AliasId(0),
            language: Language::TypeScript,
            file: Some(FileId::from_raw(0)),
            package: None,
            module: None,
            source_symbol_stable_key: interner.intern(source),
            target_symbol_stable_keys: targets
                .iter()
                .map(|target| interner.intern(*target))
                .collect(),
            kind: AliasKind::Import,
            stable_key: interner.intern(""),
            status,
        };
        fact.stable_key = fact.computed_stable_key(&interner);
        fact
    }

    fn star_export(stable_key: &str, status: SemanticStatus) -> ExportFact {
        let interner = test_stable_key_interner();
        ExportFact {
            id: ExportId(0),
            language: Language::TypeScript,
            file: Some(FileId::from_raw(0)),
            package: None,
            module: None,
            scope: None,
            symbol: None,
            export_name: "*".to_string(),
            namespace: SymbolNamespace::Module,
            kind: ExportKind::StarReexport,
            stable_key: interner.intern(stable_key),
            status,
        }
    }

    fn stable_export(export_name: &str, symbol_key: &str) -> StableExportIdentity {
        let interner = test_stable_key_interner();
        StableExportIdentity {
            id: StableExportId(0),
            export: ExportId(0),
            language: Language::TypeScript,
            package_key: None,
            module_key: Some("src/mod.ts".to_string()),
            export_name: export_name.to_string(),
            namespace: SymbolNamespace::Value,
            symbol_stable_key: interner.intern(symbol_key),
            generated_discriminator: Some("native".to_string()),
            stable_key: interner.intern(format!("stable-export:{export_name}")),
            status: SemanticStatus::Resolved,
        }
    }

    #[test]
    fn acyclic_import_alias_chains_resolve_to_deterministic_targets() {
        let interner = test_stable_key_interner();
        let output = super::alias_reexport_closure(
            &interner,
            &[
                alias("import:a", &["import:b"], SemanticStatus::Resolved),
                alias("import:b", &["symbol:c"], SemanticStatus::Resolved),
            ],
            &[],
            &[],
        );

        let closure_alias = output
            .aliases
            .iter()
            .find(|alias| interner.resolve(alias.source_symbol_stable_key).as_ref() == "import:a")
            .expect("closure contains source alias");

        assert_eq!(closure_alias.status, SemanticStatus::Resolved);
        assert_eq!(
            closure_alias
                .target_symbol_stable_keys
                .iter()
                .map(|key| interner.resolve(*key).to_string())
                .collect::<Vec<_>>(),
            vec!["symbol:c".to_string()]
        );
        assert!(output.resolutions.iter().any(|resolution| {
            interner.resolve(resolution.source_stable_key).as_ref() == "import:a"
                && resolution
                    .target_stable_keys
                    .iter()
                    .map(|key| interner.resolve(*key).to_string())
                    .collect::<Vec<_>>()
                    == vec!["symbol:c".to_string()]
                && resolution.step == ResolutionStepKind::ImportAliasLookup
                && resolution.status == SemanticStatus::Resolved
        }));
    }

    #[test]
    fn reexport_cycles_terminate_and_emit_cycle_rows() {
        let interner = test_stable_key_interner();
        let output = super::alias_reexport_closure(
            &interner,
            &[
                alias("reexport:a", &["reexport:b"], SemanticStatus::Resolved),
                alias("reexport:b", &["reexport:a"], SemanticStatus::Resolved),
            ],
            &[],
            &[],
        );

        assert!(output.aliases.iter().any(|alias| {
            interner.resolve(alias.source_symbol_stable_key).as_ref() == "reexport:a"
                && alias.status == SemanticStatus::Cycle
        }));
        assert!(output.resolutions.iter().any(|resolution| {
            interner.resolve(resolution.source_stable_key).as_ref() == "reexport:a"
                && resolution.step == ResolutionStepKind::ImportAliasLookup
                && resolution.status == SemanticStatus::Cycle
        }));
    }

    #[test]
    fn star_exports_with_incomplete_targets_emit_ambiguous_closure_rows() {
        let interner = test_stable_key_interner();
        let output = super::alias_reexport_closure(
            &interner,
            &[],
            &[star_export(
                "export:*:src/star.ts",
                SemanticStatus::Unresolved,
            )],
            &[stable_export("known", "symbol:known")],
        );

        let alias = output
            .aliases
            .iter()
            .find(|alias| {
                interner.resolve(alias.source_symbol_stable_key).as_ref() == "export:*:src/star.ts"
            })
            .expect("star export gets conservative closure alias");

        assert_eq!(alias.status, SemanticStatus::Ambiguous);
        assert_eq!(
            alias
                .target_symbol_stable_keys
                .iter()
                .map(|key| interner.resolve(*key).to_string())
                .collect::<Vec<_>>(),
            vec!["symbol:known".to_string()]
        );
        assert!(output.resolutions.iter().any(|resolution| {
            interner.resolve(resolution.source_stable_key).as_ref() == "export:*:src/star.ts"
                && resolution.status == SemanticStatus::Ambiguous
        }));
    }

    #[test]
    fn star_reexport_closure_keys_include_original_export_key() {
        let interner = test_stable_key_interner();
        let output = super::alias_reexport_closure(
            &interner,
            &[],
            &[
                star_export("export:*:src/a.ts", SemanticStatus::Unresolved),
                star_export("export:*:src/b.ts", SemanticStatus::Unresolved),
            ],
            &[stable_export("known", "symbol:known")],
        );
        let stable_keys = output
            .aliases
            .iter()
            .map(|alias| interner.resolve(alias.stable_key).to_string())
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(stable_keys.len(), 2);
    }
}

#[cfg(test)]
mod native_generated_hooks {
    use super::*;
    use crate::analysis_api::SymbolNamespace;
    use crate::internal_core::Language;

    fn stable_export(
        interner: &crate::internal_core::StableKeyInterner,
        language: Language,
        export_name: &str,
        generated: bool,
    ) -> StableExportIdentity {
        StableExportIdentity {
            id: StableExportId(0),
            export: ExportId(0),
            language,
            package_key: (language == Language::Go)
                .then(|| "go:package:example.com/app".to_string()),
            module_key: (language != Language::Go).then(|| "src/mod.ts".to_string()),
            export_name: export_name.to_string(),
            namespace: SymbolNamespace::Value,
            symbol_stable_key: interner.intern(format!("symbol:{export_name}")),
            generated_discriminator: generated.then(|| "native".to_string()),
            stable_key: interner.intern(format!("stable-export:{export_name}")),
            status: if generated {
                SemanticStatus::Generated
            } else {
                SemanticStatus::Resolved
            },
        }
    }

    #[test]
    fn ts_native_stable_exports_emit_generated_symbol_hooks_with_provenance() {
        let interner = crate::internal_core::test_stable_key_interner();
        let mut semantic = SemanticIndexOutput::default();
        semantic.stable_exports.push(stable_export(
            &interner,
            Language::TypeScript,
            "route",
            true,
        ));

        let output = super::emit_native_generated_symbol_hooks(&interner, &semantic);

        assert_eq!(output.generated_symbols.len(), 1);
        let generated = &output.generated_symbols[0];
        assert_eq!(generated.status, SemanticStatus::Generated);
        assert_eq!(generated.producer_id, "polint.symbol_graph");
        assert_eq!(
            interner.resolve(generated.source_stable_key).as_ref(),
            "stable-export:route"
        );
        assert_eq!(generated.generated_discriminator, "native");
        assert!(
            interner
                .resolve(generated.stable_key)
                .contains("polint.symbol_graph")
        );
        assert!(output.resolutions.iter().any(|resolution| {
            resolution.step == ResolutionStepKind::GeneratedHintLookup
                && resolution.status == SemanticStatus::Generated
                && interner.resolve(resolution.source_stable_key).as_ref() == "stable-export:route"
        }));
    }

    #[test]
    fn go_generated_stable_exports_emit_generated_symbol_hooks() {
        let interner = crate::internal_core::test_stable_key_interner();
        let mut semantic = SemanticIndexOutput::default();
        semantic
            .stable_exports
            .push(stable_export(&interner, Language::Go, "Handler", true));

        let output = super::emit_native_generated_symbol_hooks(&interner, &semantic);

        assert_eq!(output.generated_symbols.len(), 1);
        let generated = &output.generated_symbols[0];
        assert_eq!(generated.language, Language::Go);
        assert_eq!(generated.status, SemanticStatus::Generated);
        assert_eq!(generated.producer_id, "polint.symbol_graph");
        assert_eq!(
            interner.resolve(generated.source_stable_key).as_ref(),
            "stable-export:Handler"
        );
    }
}
