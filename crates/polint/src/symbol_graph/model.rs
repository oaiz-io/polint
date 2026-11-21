use crate::core::{
    DefinitionFact, DefinitionId, DefinitionKind, FileId, Language, ModuleNodeId, PackageId,
    ReferenceFact, ReferenceId, ReferenceKind, Span, StableKeyInterner, SymbolFact, SymbolId,
    SymbolKind, SymbolNamespace, SymbolPrecision, SymbolResolutionStatus,
};
use crate::diagnostics::Diagnostic;
use crate::symbol_graph::semantic::CachedSemanticIndexOutput;
use serde::{Deserialize, Serialize};

pub(crate) const SYMBOL_GRAPH_LAYER_SCHEMA: &str = "symbol-graph-facts-4";

pub(crate) use crate::analysis_neutral::symbol_graph::model::SymbolGraphBuilder;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SymbolGraphLayerPayload {
    pub(crate) schema: String,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) capability_support: Vec<crate::core::CapabilitySupport>,
    pub(crate) symbols: Vec<CachedSymbolFact>,
    pub(crate) definitions: Vec<CachedDefinitionFact>,
    pub(crate) references: Vec<CachedReferenceFact>,
    pub(crate) semantic_index: CachedSemanticIndexOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedSymbolFact {
    id: SymbolId,
    language: Language,
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    namespace: SymbolNamespace,
    file: Option<FileId>,
    package: Option<PackageId>,
    module: Option<ModuleNodeId>,
    owner: Option<SymbolId>,
    primary_span: Option<Span>,
    is_exported: bool,
    #[serde(rename = "stable_key")]
    stable_key_text: String,
    precision: SymbolPrecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedDefinitionFact {
    id: DefinitionId,
    symbol: SymbolId,
    language: Language,
    name: String,
    qualified_name: String,
    kind: DefinitionKind,
    namespace: SymbolNamespace,
    file: Option<FileId>,
    package: Option<PackageId>,
    module: Option<ModuleNodeId>,
    owner: Option<SymbolId>,
    primary_span: Option<Span>,
    is_primary: bool,
    is_exported: bool,
    #[serde(rename = "stable_key")]
    stable_key_text: String,
    precision: SymbolPrecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedReferenceFact {
    id: ReferenceId,
    language: Language,
    name: String,
    qualified_name: String,
    kind: ReferenceKind,
    namespace: SymbolNamespace,
    file: Option<FileId>,
    package: Option<PackageId>,
    module: Option<ModuleNodeId>,
    owner: Option<SymbolId>,
    primary_span: Option<Span>,
    target: Option<SymbolId>,
    candidates: Vec<SymbolId>,
    #[serde(rename = "stable_key")]
    stable_key_text: String,
    status: SymbolResolutionStatus,
    precision: SymbolPrecision,
}

impl CachedSymbolFact {
    pub(crate) fn from_fact(interner: &StableKeyInterner, fact: &SymbolFact) -> Self {
        Self {
            id: fact.id,
            language: fact.language,
            name: fact.name.clone(),
            qualified_name: fact.qualified_name.clone(),
            kind: fact.kind,
            namespace: fact.namespace,
            file: fact.file,
            package: fact.package,
            module: fact.module,
            owner: fact.owner,
            primary_span: fact.primary_span.clone(),
            is_exported: fact.is_exported,
            stable_key_text: interner.resolve(fact.stable_key).to_string(),
            precision: fact.precision,
        }
    }

    pub(crate) fn into_fact(self, interner: &StableKeyInterner) -> SymbolFact {
        SymbolFact::new(
            self.id,
            self.language,
            self.name,
            self.qualified_name,
            self.kind,
            self.namespace,
            self.file,
            self.package,
            self.module,
            self.owner,
            self.primary_span,
            self.is_exported,
            interner.intern(self.stable_key_text),
            self.precision,
        )
    }
}

impl CachedDefinitionFact {
    pub(crate) fn from_fact(interner: &StableKeyInterner, fact: &DefinitionFact) -> Self {
        Self {
            id: fact.id,
            symbol: fact.symbol,
            language: fact.language,
            name: fact.name.clone(),
            qualified_name: fact.qualified_name.clone(),
            kind: fact.kind,
            namespace: fact.namespace,
            file: fact.file,
            package: fact.package,
            module: fact.module,
            owner: fact.owner,
            primary_span: fact.primary_span.clone(),
            is_primary: fact.is_primary,
            is_exported: fact.is_exported,
            stable_key_text: interner.resolve(fact.stable_key).to_string(),
            precision: fact.precision,
        }
    }

    pub(crate) fn into_fact(self, interner: &StableKeyInterner) -> DefinitionFact {
        DefinitionFact::new(
            self.id,
            self.symbol,
            self.language,
            self.name,
            self.qualified_name,
            self.kind,
            self.namespace,
            self.file,
            self.package,
            self.module,
            self.owner,
            self.primary_span,
            self.is_primary,
            self.is_exported,
            interner.intern(self.stable_key_text),
            self.precision,
        )
    }
}

impl CachedReferenceFact {
    pub(crate) fn from_fact(interner: &StableKeyInterner, fact: &ReferenceFact) -> Self {
        Self {
            id: fact.id,
            language: fact.language,
            name: fact.name.clone(),
            qualified_name: fact.qualified_name.clone(),
            kind: fact.kind,
            namespace: fact.namespace,
            file: fact.file,
            package: fact.package,
            module: fact.module,
            owner: fact.owner,
            primary_span: fact.primary_span.clone(),
            target: fact.target,
            candidates: fact.candidates.clone(),
            stable_key_text: interner.resolve(fact.stable_key).to_string(),
            status: fact.status,
            precision: fact.precision,
        }
    }

    pub(crate) fn into_fact(self, interner: &StableKeyInterner) -> ReferenceFact {
        ReferenceFact::new(
            self.id,
            self.language,
            self.name,
            self.qualified_name,
            self.kind,
            self.namespace,
            self.file,
            self.package,
            self.module,
            self.owner,
            self.primary_span,
            self.target,
            self.candidates,
            interner.intern(self.stable_key_text),
            self.status,
            self.precision,
        )
    }
}

// Neutral builder tests live beside their implementation in polint-analysis.
