use std::collections::{BTreeMap, HashMap, HashSet};

use crate::internal_core::{StableKeyId, StableKeyInterner};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FactFamily {
    SourceFile,
    Package,
    Function,
    Import,
    BranchObligation,
    Test,
    Coverage,
    TsComponent,
    TsClass,
    StringLiteral,
    JsxAttribute,
    ResolvedImport,
    ModuleNode,
    ModuleEdge,
    WorkspaceRoot,
    TopologyPackage,
    SourceSet,
    DependencyRequirement,
    ResolvedDependencyEdge,
    ImportToPackage,
    RepoTopologyOverlay,
    Scope,
    SemanticImport,
    Export,
    Alias,
    Resolution,
    GeneratedSymbol,
    StableExport,
    Symbol,
    Definition,
    Reference,
    FileMetric,
    FunctionMetric,
    ComplexityMetric,
    Place,
    MirBody,
    MirOperation,
    CfgFunction,
    CfgNode,
    BasicBlock,
    CfgEdge,
    CfgReachability,
    CfgDominator,
    CfgPostDominator,
    CfgControlDependence,
    UnsupportedControlFlow,
    CallSite,
    CallTarget,
    UnresolvedCall,
    RefinedCallEdge,
    DataFlowNode,
    DataFlowEdge,
    DataFlowModel,
    DataFlowBudget,
    EvidenceNode,
    EvidenceEdge,
    EvidenceBundle,
    EvidencePath,
    EvidenceSlice,
    EvidenceUnknown,
    EvidenceOmittedRegion,
    EvidenceReplayKey,
    DomainObservation,
    DomainEvent,
    SummaryControl,
    SummaryCall,
    SummaryMemory,
    SummaryTito,
    SummaryEvent,
    ExtensionFact,
    Entrypoint,
    TrustBoundary,
    DispatchEdge,
    UnresolvedFramework,
    Type,
    NarrowedType,
    Value,
    AllocationToken,
    AccessPath,
    PointsToConstraint,
    PointsToSet,
    AliasAnswer,
    AdaptationModel,
    /// Unified-solver derived-edge facts (GRAPH-04). Each row tags a
    /// solver-derived edge whose `DerivedEdgeProvenance` records its contributing
    /// fact IDs (total-ordered by stable ID), producing `ConstraintKind`, and the
    /// monotonic solver step. The provider that emits these into the kernel lands in
    /// Plan 03; the fact family + stable-key tagging are produced here in Plan 02.
    SolverDerivedEdge,
    TypeValueAliasEvent,
    MirStatement,
    MirTerminator,
    UnsupportedSemantic,
    /// Registry key for the Go semantic store.
    GoSemantic,
    /// Registry key for the TS object-model store.
    TsObjectModel,
    /// Registry key for the TypeScript type-sidecar store.
    TsTypes,
    /// Registry key for the identity store.
    Identity,
    /// Registry key for the reachability store.
    Reachability,
    /// Registry key for the semantic-graph store.
    SemanticGraph,
}

impl FactFamily {
    /// Every variant, in declaration order.
    ///
    /// The enum has no iterator; `fact_rows_dump` (the I1b oracle) walks this to
    /// dump one file per family, so a variant added without a row here would be
    /// invisible to the oracle. `fact_family_all_covers_every_variant` fails when
    /// the two drift apart.
    pub const ALL: &'static [FactFamily] = &[
        Self::SourceFile,
        Self::Package,
        Self::Function,
        Self::Import,
        Self::BranchObligation,
        Self::Test,
        Self::Coverage,
        Self::TsComponent,
        Self::TsClass,
        Self::StringLiteral,
        Self::JsxAttribute,
        Self::ResolvedImport,
        Self::ModuleNode,
        Self::ModuleEdge,
        Self::WorkspaceRoot,
        Self::TopologyPackage,
        Self::SourceSet,
        Self::DependencyRequirement,
        Self::ResolvedDependencyEdge,
        Self::ImportToPackage,
        Self::RepoTopologyOverlay,
        Self::Scope,
        Self::SemanticImport,
        Self::Export,
        Self::Alias,
        Self::Resolution,
        Self::GeneratedSymbol,
        Self::StableExport,
        Self::Symbol,
        Self::Definition,
        Self::Reference,
        Self::FileMetric,
        Self::FunctionMetric,
        Self::ComplexityMetric,
        Self::Place,
        Self::MirBody,
        Self::MirOperation,
        Self::CfgFunction,
        Self::CfgNode,
        Self::BasicBlock,
        Self::CfgEdge,
        Self::CfgReachability,
        Self::CfgDominator,
        Self::CfgPostDominator,
        Self::CfgControlDependence,
        Self::UnsupportedControlFlow,
        Self::CallSite,
        Self::CallTarget,
        Self::UnresolvedCall,
        Self::RefinedCallEdge,
        Self::DataFlowNode,
        Self::DataFlowEdge,
        Self::DataFlowModel,
        Self::DataFlowBudget,
        Self::EvidenceNode,
        Self::EvidenceEdge,
        Self::EvidenceBundle,
        Self::EvidencePath,
        Self::EvidenceSlice,
        Self::EvidenceUnknown,
        Self::EvidenceOmittedRegion,
        Self::EvidenceReplayKey,
        Self::DomainObservation,
        Self::DomainEvent,
        Self::SummaryControl,
        Self::SummaryCall,
        Self::SummaryMemory,
        Self::SummaryTito,
        Self::SummaryEvent,
        Self::ExtensionFact,
        Self::Entrypoint,
        Self::TrustBoundary,
        Self::DispatchEdge,
        Self::UnresolvedFramework,
        Self::Type,
        Self::NarrowedType,
        Self::Value,
        Self::AllocationToken,
        Self::AccessPath,
        Self::PointsToConstraint,
        Self::PointsToSet,
        Self::AliasAnswer,
        Self::AdaptationModel,
        Self::SolverDerivedEdge,
        Self::TypeValueAliasEvent,
        Self::MirStatement,
        Self::MirTerminator,
        Self::UnsupportedSemantic,
        Self::GoSemantic,
        Self::TsObjectModel,
        Self::TsTypes,
        Self::Identity,
        Self::Reachability,
        Self::SemanticGraph,
    ];

    /// Whether this family's key texts embed long part values by digest.
    ///
    /// The deep-analysis families compose their keys from other keys: an
    /// operation key carries its body key, which carries its function key; a CFG
    /// edge carries a body, two blocks and two nodes. Embedding each parent's full
    /// text made key text grow with nesting depth (kilobytes per key, gigabytes per
    /// large module). These families embed a part value longer than
    /// [`COMPACT_PART_VALUE_MAX_BYTES`] by its digest instead; see
    /// [`compact_part_value`]. The source, syntax, module-graph, symbol and metrics
    /// families, and the unsupported-semantic family that also orders `polint
    /// unknowns` rows, keep their values verbatim.
    pub fn compacts_long_part_values(self) -> bool {
        matches!(
            self,
            Self::Place
                | Self::MirBody
                | Self::MirOperation
                | Self::MirStatement
                | Self::MirTerminator
                | Self::CfgFunction
                | Self::CfgNode
                | Self::BasicBlock
                | Self::CfgEdge
                | Self::CfgReachability
                | Self::CfgDominator
                | Self::CfgPostDominator
                | Self::CfgControlDependence
                | Self::UnsupportedControlFlow
                | Self::CallSite
                | Self::CallTarget
                | Self::UnresolvedCall
                | Self::RefinedCallEdge
                | Self::DataFlowNode
                | Self::DataFlowEdge
                | Self::DataFlowModel
                | Self::DataFlowBudget
                | Self::EvidenceNode
                | Self::EvidenceEdge
                | Self::EvidenceBundle
                | Self::EvidencePath
                | Self::EvidenceSlice
                | Self::EvidenceUnknown
                | Self::EvidenceOmittedRegion
                | Self::EvidenceReplayKey
                | Self::DomainObservation
                | Self::DomainEvent
                | Self::SummaryControl
                | Self::SummaryCall
                | Self::SummaryMemory
                | Self::SummaryTito
                | Self::SummaryEvent
                | Self::ExtensionFact
                | Self::Entrypoint
                | Self::TrustBoundary
                | Self::DispatchEdge
                | Self::UnresolvedFramework
                | Self::Type
                | Self::NarrowedType
                | Self::Value
                | Self::AllocationToken
                | Self::AccessPath
                | Self::PointsToConstraint
                | Self::PointsToSet
                | Self::AliasAnswer
                | Self::AdaptationModel
                | Self::SolverDerivedEdge
                | Self::TypeValueAliasEvent
                | Self::GoSemantic
                | Self::TsObjectModel
                | Self::TsTypes
                | Self::Identity
                | Self::Reachability
                | Self::SemanticGraph
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::SourceFile => "SourceFile",
            Self::Package => "Package",
            Self::Function => "Function",
            Self::Import => "Import",
            Self::BranchObligation => "BranchObligation",
            Self::Test => "Test",
            Self::Coverage => "Coverage",
            Self::TsComponent => "TsComponent",
            Self::TsClass => "TsClass",
            Self::StringLiteral => "StringLiteral",
            Self::JsxAttribute => "JsxAttribute",
            Self::ResolvedImport => "ResolvedImport",
            Self::ModuleNode => "ModuleNode",
            Self::ModuleEdge => "ModuleEdge",
            Self::WorkspaceRoot => "WorkspaceRoot",
            Self::TopologyPackage => "TopologyPackage",
            Self::SourceSet => "SourceSet",
            Self::DependencyRequirement => "DependencyRequirement",
            Self::ResolvedDependencyEdge => "ResolvedDependencyEdge",
            Self::ImportToPackage => "ImportToPackage",
            Self::RepoTopologyOverlay => "RepoTopologyOverlay",
            Self::Scope => "Scope",
            Self::SemanticImport => "SemanticImport",
            Self::Export => "Export",
            Self::Alias => "Alias",
            Self::Resolution => "Resolution",
            Self::GeneratedSymbol => "GeneratedSymbol",
            Self::StableExport => "StableExport",
            Self::Symbol => "Symbol",
            Self::Definition => "Definition",
            Self::Reference => "Reference",
            Self::FileMetric => "FileMetric",
            Self::FunctionMetric => "FunctionMetric",
            Self::ComplexityMetric => "ComplexityMetric",
            Self::Place => "Place",
            Self::MirBody => "MirBody",
            Self::MirOperation => "MirOperation",
            Self::CfgFunction => "CfgFunction",
            Self::CfgNode => "CfgNode",
            Self::BasicBlock => "BasicBlock",
            Self::CfgEdge => "CfgEdge",
            Self::CfgReachability => "CfgReachability",
            Self::CfgDominator => "CfgDominator",
            Self::CfgPostDominator => "CfgPostDominator",
            Self::CfgControlDependence => "CfgControlDependence",
            Self::UnsupportedControlFlow => "UnsupportedControlFlow",
            Self::CallSite => "CallSite",
            Self::CallTarget => "CallTarget",
            Self::UnresolvedCall => "UnresolvedCall",
            Self::RefinedCallEdge => "RefinedCallEdge",
            Self::DataFlowNode => "DataFlowNode",
            Self::DataFlowEdge => "DataFlowEdge",
            Self::DataFlowModel => "DataFlowModel",
            Self::DataFlowBudget => "DataFlowBudget",
            Self::EvidenceNode => "EvidenceNode",
            Self::EvidenceEdge => "EvidenceEdge",
            Self::EvidenceBundle => "EvidenceBundle",
            Self::EvidencePath => "EvidencePath",
            Self::EvidenceSlice => "EvidenceSlice",
            Self::EvidenceUnknown => "EvidenceUnknown",
            Self::EvidenceOmittedRegion => "EvidenceOmittedRegion",
            Self::EvidenceReplayKey => "EvidenceReplayKey",
            Self::DomainObservation => "DomainObservation",
            Self::DomainEvent => "DomainEvent",
            Self::SummaryControl => "SummaryControl",
            Self::SummaryCall => "SummaryCall",
            Self::SummaryMemory => "SummaryMemory",
            Self::SummaryTito => "SummaryTito",
            Self::SummaryEvent => "SummaryEvent",
            Self::ExtensionFact => "ExtensionFact",
            Self::Entrypoint => "Entrypoint",
            Self::TrustBoundary => "TrustBoundary",
            Self::DispatchEdge => "DispatchEdge",
            Self::UnresolvedFramework => "UnresolvedFramework",
            Self::Type => "Type",
            Self::NarrowedType => "NarrowedType",
            Self::Value => "Value",
            Self::AllocationToken => "AllocationToken",
            Self::AccessPath => "AccessPath",
            Self::PointsToConstraint => "PointsToConstraint",
            Self::PointsToSet => "PointsToSet",
            Self::AliasAnswer => "AliasAnswer",
            Self::AdaptationModel => "AdaptationModel",
            Self::SolverDerivedEdge => "SolverDerivedEdge",
            Self::TypeValueAliasEvent => "TypeValueAliasEvent",
            Self::MirStatement => "MirStatement",
            Self::MirTerminator => "MirTerminator",
            Self::UnsupportedSemantic => "UnsupportedSemantic",
            Self::GoSemantic => "GoSemantic",
            Self::TsObjectModel => "TsObjectModel",
            Self::TsTypes => "TsTypes",
            Self::Identity => "Identity",
            Self::Reachability => "Reachability",
            Self::SemanticGraph => "SemanticGraph",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactRef {
    pub family: FactFamily,
    pub run_id: u64,
}

impl FactRef {
    pub fn new(family: FactFamily, run_id: u64) -> Self {
        Self { family, run_id }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingFactMeta {
    pub family: FactFamily,
    pub run_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactMeta {
    pub stable_key: StableKeyId,
    pub producer_id: &'static str,
    pub layer_id: &'static str,
    pub precision: FactPrecision,
    pub confidence: FactConfidence,
    pub validation: ValidationStatus,
    pub payload_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactPrecision {
    Exact,
    Syntax,
    SetupAware,
    Heuristic,
    Unresolved,
    Ambiguous,
    SetupMissing,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationStatus {
    NativeTrusted,
    SchemaValidated,
    ReferentiallyValidated,
    SpanValidated,
    StableKeyValidated,
    ConflictRejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableKeyOwner {
    pub reference: FactRef,
    pub payload_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StableKeyConflict {
    pub family: FactFamily,
    pub stable_key: StableKeyId,
    pub existing: FactRef,
    pub incoming: FactRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactMetaInsert {
    Inserted,
    Idempotent,
    Conflict {
        family: FactFamily,
        stable_key: StableKeyId,
        existing: FactRef,
        incoming: FactRef,
    },
}

#[derive(Debug, Default, Clone)]
struct FactFamilyRows {
    dense: Vec<Option<FactMeta>>,
    sparse: BTreeMap<u64, FactMeta>,
}

impl FactFamilyRows {
    const DENSE_GAP_LIMIT: usize = 1024;

    fn insert(&mut self, run_id: u64, meta: FactMeta) {
        if let Ok(index) = usize::try_from(run_id)
            && index <= self.dense.len().saturating_add(Self::DENSE_GAP_LIMIT)
        {
            if self.dense.len() <= index {
                self.dense.resize_with(index + 1, || None);
            }
            self.dense[index] = Some(meta);
            self.sparse.remove(&run_id);
        } else {
            self.sparse.insert(run_id, meta);
        }
    }

    fn remove(&mut self, run_id: u64) -> Option<FactMeta> {
        if let Ok(index) = usize::try_from(run_id)
            && let Some(row) = self.dense.get_mut(index)
        {
            return row.take();
        }
        self.sparse.remove(&run_id)
    }

    fn get(&self, run_id: u64) -> Option<&FactMeta> {
        if let Ok(index) = usize::try_from(run_id)
            && let Some(row) = self.dense.get(index)
        {
            return row.as_ref();
        }
        self.sparse.get(&run_id)
    }

    fn rows(&self) -> impl Iterator<Item = (u64, &FactMeta)> {
        self.dense
            .iter()
            .enumerate()
            .filter_map(|(run_id, metadata)| {
                metadata.as_ref().map(|metadata| (run_id as u64, metadata))
            })
            .chain(
                self.sparse
                    .iter()
                    .map(|(run_id, metadata)| (*run_id, metadata)),
            )
    }
}

#[derive(Debug, Default, Clone)]
pub struct FactMetaStore {
    rows: BTreeMap<FactFamily, FactFamilyRows>,
    stable_key_owners: BTreeMap<FactFamily, HashMap<StableKeyId, StableKeyOwner>>,
    stable_key_conflicts: HashSet<StableKeyConflict>,
}

impl FactMetaStore {
    pub fn insert(&mut self, reference: FactRef, meta: FactMeta) -> FactMetaInsert {
        let family_owners = self.stable_key_owners.entry(reference.family).or_default();
        let result = if let Some(owner) = family_owners.get(&meta.stable_key) {
            if owner.reference == reference && owner.payload_digest == meta.payload_digest {
                FactMetaInsert::Idempotent
            } else {
                let conflict = StableKeyConflict {
                    family: reference.family,
                    stable_key: meta.stable_key,
                    existing: owner.reference,
                    incoming: reference,
                };
                self.stable_key_conflicts.insert(conflict.clone());
                FactMetaInsert::Conflict {
                    family: conflict.family,
                    stable_key: conflict.stable_key,
                    existing: conflict.existing,
                    incoming: conflict.incoming,
                }
            }
        } else {
            family_owners.insert(
                meta.stable_key,
                StableKeyOwner {
                    reference,
                    payload_digest: meta.payload_digest.clone(),
                },
            );
            FactMetaInsert::Inserted
        };

        self.rows
            .entry(reference.family)
            .or_default()
            .insert(reference.run_id, meta);
        result
    }

    pub fn remove_family(&mut self, family: FactFamily) {
        self.rows.remove(&family);
        self.stable_key_owners.remove(&family);
        self.stable_key_conflicts
            .retain(|conflict| conflict.family != family);
    }

    pub fn finish_family_insertions(&mut self, family: FactFamily) {
        self.stable_key_owners.remove(&family);
    }

    pub fn finish_all_insertions(&mut self) {
        self.stable_key_owners.clear();
    }

    #[doc(hidden)]
    pub fn remove_for_test(&mut self, reference: FactRef) -> Option<FactMeta> {
        self.rows
            .get_mut(&reference.family)
            .and_then(|rows| rows.remove(reference.run_id))
    }

    pub fn get(&self, reference: FactRef) -> Option<&FactMeta> {
        self.rows
            .get(&reference.family)
            .and_then(|rows| rows.get(reference.run_id))
    }

    pub fn stable_key_conflicts(&self) -> impl Iterator<Item = &StableKeyConflict> {
        self.stable_key_conflicts.iter()
    }

    #[cfg(test)]
    pub fn stable_key_owner(
        &self,
        family: FactFamily,
        stable_key: StableKeyId,
    ) -> Option<&StableKeyOwner> {
        self.stable_key_owners
            .get(&family)
            .and_then(|owners| owners.get(&stable_key))
    }

    /// Number of metadata rows retained across every family.
    ///
    /// Counted from the per-family row vectors (O(families)), so the resource
    /// gauge can report fact volume at a stage boundary without walking rows.
    pub fn row_count(&self) -> usize {
        self.rows
            .values()
            .map(|rows| rows.dense.iter().filter(|row| row.is_some()).count() + rows.sparse.len())
            .sum()
    }

    pub fn rows(&self) -> impl Iterator<Item = (FactRef, &FactMeta)> {
        self.rows.iter().flat_map(|(family, rows)| {
            rows.rows()
                .map(move |(run_id, metadata)| (FactRef::new(*family, run_id), metadata))
        })
    }

    /// Iterates metadata rows for one fact family without scanning unrelated families.
    pub fn family_rows(&self, family: FactFamily) -> impl Iterator<Item = &FactMeta> {
        self.family_rows_with_run_id(family)
            .map(|(_, metadata)| metadata)
    }

    /// [`family_rows`](Self::family_rows) with each row's run id.
    ///
    /// The run id is the producing fact's own dense id (a `SummaryId` for the
    /// summary families, `core/db.rs`'s `refresh_summary_metadata`), which is the
    /// only exact join back to the fact behind a metadata row. `fact_rows_dump`
    /// needs it to render the plaintext parts and attributes columns of I1b.
    pub fn family_rows_with_run_id(
        &self,
        family: FactFamily,
    ) -> impl Iterator<Item = (u64, &FactMeta)> {
        self.rows
            .get(&family)
            .into_iter()
            .flat_map(|rows| rows.rows())
    }
}

pub fn stable_key_from_parts<V: AsRef<str>>(
    interner: &StableKeyInterner,
    family: FactFamily,
    parts: &[(&str, V)],
) -> StableKeyId {
    let mut borrowed = parts
        .iter()
        .map(|(label, value)| (*label, value.as_ref()))
        .collect::<Vec<_>>();
    let mut text = String::new();
    write_stable_key_text(&mut text, family, &mut borrowed);
    interner.intern(text)
}

/// Canonical stable-key text for `family` and `parts`, **without interning it**.
///
/// Most callers want the text: to embed in a larger composed key, to feed a
/// payload digest, or to key a local map. Routing them through the interner
/// retained one key per call for the whole run, because interning is permanent
/// and these keys are never a fact's identity. The text produced here is
/// byte-identical to what [`stable_key_from_parts`] interns.
pub fn stable_key_text_from_parts<V: AsRef<str>>(
    family: FactFamily,
    parts: &[(&str, V)],
) -> String {
    let mut borrowed = parts
        .iter()
        .map(|(label, value)| (*label, value.as_ref()))
        .collect::<Vec<_>>();
    let mut text = String::new();
    write_stable_key_text(&mut text, family, &mut borrowed);
    text
}

/// Writes the canonical stable-key text for `family` and `parts` into `buffer`,
/// replacing whatever it held.
///
/// `parts` is sorted in place because the key must not depend on the order a
/// caller listed its parts in. Every label and value is length-prefixed so no
/// value can forge a delimiter, and `\` is folded to `/` in values so a key does
/// not depend on the host's path separator.
///
/// Callers reuse one buffer across many facts; the text is only copied when a
/// key turns out to be new.
pub fn write_stable_key_text(buffer: &mut String, family: FactFamily, parts: &mut [(&str, &str)]) {
    parts.sort_by(|left, right| left.0.cmp(right.0));

    buffer.clear();
    push_length_prefixed(buffer, family.label());
    let compact = family.compacts_long_part_values();
    for (label, value) in parts.iter() {
        buffer.push('|');
        push_length_prefixed(buffer, label);
        buffer.push('=');
        if compact && value.len() > COMPACT_PART_VALUE_MAX_BYTES {
            push_length_prefixed(buffer, &compact_part_value(value));
        } else {
            push_length_prefixed_path(buffer, value);
        }
    }
}

/// The longest part value a compacting family embeds verbatim; see
/// [`FactFamily::compacts_long_part_values`].
pub const COMPACT_PART_VALUE_MAX_BYTES: usize = 64;

/// The digest form of a long key part value: `#` followed by the first 128 bits
/// of the SHA-256 of the value (with `\` folded to `/`, as every key value is),
/// as lowercase hex.
///
/// Distinct values keep distinct keys with overwhelming probability (a
/// collision among `n` values has probability about n²/2¹²⁹), and the form is a
/// pure function of the value, so every builder of a key derives the same text.
/// No value is ever parsed back out of a key, so nothing reads the digest but
/// equality and ordering.
pub fn compact_part_value(value: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    let mut rest = value;
    while let Some(index) = rest.find('\\') {
        hasher.update(rest[..index].as_bytes());
        hasher.update(b"/");
        rest = &rest[index + 1..];
    }
    hasher.update(rest.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(33);
    out.push('#');
    for byte in &digest[..16] {
        out.push(char::from_digit(u32::from(byte >> 4), 16).expect("a nibble is a hex digit"));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).expect("a nibble is a hex digit"));
    }
    out
}

/// A key's text as another composed key embeds it: verbatim when short, by
/// [`compact_part_value`] when longer than [`COMPACT_PART_VALUE_MAX_BYTES`].
/// For keys composed outside [`write_stable_key_text`], such as a MIR block key
/// built from its body key.
pub fn compact_key_reference(key: &str) -> std::borrow::Cow<'_, str> {
    if key.len() > COMPACT_PART_VALUE_MAX_BYTES {
        std::borrow::Cow::Owned(compact_part_value(key))
    } else {
        std::borrow::Cow::Borrowed(key)
    }
}

fn push_length_prefixed(buffer: &mut String, value: &str) {
    push_decimal(buffer, value.len());
    buffer.push(':');
    buffer.push_str(value);
}

/// Like [`push_length_prefixed`], folding `\` to `/`. The fold is byte-for-byte,
/// so the length prefix is the same either way.
fn push_length_prefixed_path(buffer: &mut String, value: &str) {
    push_decimal(buffer, value.len());
    buffer.push(':');
    let mut rest = value;
    while let Some(index) = rest.find('\\') {
        buffer.push_str(&rest[..index]);
        buffer.push('/');
        rest = &rest[index + 1..];
    }
    buffer.push_str(rest);
}

fn push_decimal(buffer: &mut String, value: usize) {
    let mut digits = [0_u8; 20];
    let mut index = digits.len();
    let mut value = value;
    loop {
        index -= 1;
        digits[index] = b'0' + u8::try_from(value % 10).expect("a decimal digit fits in a byte");
        value /= 10;
        if value == 0 {
            break;
        }
    }
    buffer.push_str(std::str::from_utf8(&digits[index..]).expect("decimal digits are ASCII"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal_core::stable_key_for_test;

    #[test]
    fn deep_families_embed_long_part_values_by_digest_and_others_verbatim() {
        let long = format!("{}/handlers.go", "core/internal/catalog/service".repeat(3));
        assert!(long.len() > COMPACT_PART_VALUE_MAX_BYTES);
        let deep = stable_key_text_from_parts(
            FactFamily::MirOperation,
            &[("body", long.as_str()), ("ordinal", "7")],
        );
        let digest = compact_part_value(&long);
        assert_eq!(
            deep,
            format!("12:MirOperation|4:body=33:{digest}|7:ordinal=1:7")
        );
        assert!(!deep.contains("handlers.go"));

        let syntax = stable_key_text_from_parts(FactFamily::Function, &[("path", long.as_str())]);
        assert!(
            syntax.contains(&long),
            "syntax-family keys keep every value verbatim"
        );

        let at_limit = "x".repeat(COMPACT_PART_VALUE_MAX_BYTES);
        let kept = stable_key_text_from_parts(FactFamily::CfgNode, &[("block", at_limit.as_str())]);
        assert!(
            kept.contains(&at_limit),
            "a value at the limit stays verbatim"
        );
    }

    #[test]
    fn compact_part_values_are_fixed_width_folded_and_distinct() {
        let value = "a".repeat(100);
        let digest = compact_part_value(&value);
        assert_eq!(digest.len(), 33);
        assert!(digest.starts_with('#'));
        assert!(
            digest[1..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        assert_eq!(
            digest,
            compact_part_value(&value),
            "a pure function of the value"
        );
        assert_eq!(
            compact_part_value(&format!("dir\\{value}")),
            compact_part_value(&format!("dir/{value}")),
            "backslashes fold before hashing, as they do in verbatim values"
        );
        assert_ne!(digest, compact_part_value(&format!("{value}b")));
    }

    #[test]
    fn a_key_reference_is_verbatim_when_short_and_a_digest_when_long() {
        assert_eq!(compact_key_reference("short-key"), "short-key");
        let long = "k".repeat(COMPACT_PART_VALUE_MAX_BYTES + 1);
        assert_eq!(compact_key_reference(&long), compact_part_value(&long));
    }

    /// `FactFamily::ALL` must list every variant exactly once.
    ///
    /// The enum has no derived iterator and no stable `variant_count`, so the
    /// count comes from the declaration itself: the test reads this file and
    /// counts the bare variant lines of the `enum FactFamily` body. A variant
    /// added without a row in `ALL` would otherwise be dumped by no oracle at
    /// all, since `fact_rows_dump` walks `ALL` and nothing else.
    #[test]
    fn fact_family_all_covers_every_variant() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/analysis_api/metadata.rs"),
        )
        .expect("read this source file");
        let body = source
            .split_once("pub enum FactFamily {")
            .expect("the enum declaration")
            .1
            .split_once("\n}")
            .expect("the enum body ends at a column-zero brace")
            .0;
        let declared: Vec<&str> = body
            .lines()
            .map(str::trim)
            .filter(|line| {
                line.ends_with(',')
                    && line[..line.len() - 1]
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric())
                    && line.starts_with(|character: char| character.is_ascii_uppercase())
            })
            .collect();

        assert_eq!(
            FactFamily::ALL.len(),
            declared.len(),
            "FactFamily::ALL has {} entries for {} declared variants",
            FactFamily::ALL.len(),
            declared.len()
        );
        let labels: BTreeMap<&str, usize> =
            FactFamily::ALL
                .iter()
                .enumerate()
                .fold(BTreeMap::new(), |mut acc, (index, family)| {
                    acc.insert(family.label(), index);
                    acc
                });
        assert_eq!(
            labels.len(),
            FactFamily::ALL.len(),
            "FactFamily::ALL lists a variant twice"
        );
        for (index, name) in declared.iter().enumerate() {
            let variant = &name[..name.len() - 1];
            assert_eq!(
                FactFamily::ALL[index].label(),
                variant,
                "FactFamily::ALL is not in declaration order at index {index}"
            );
        }
    }

    #[test]
    fn fact_ref_keeps_run_local_id_separate_from_stable_key() {
        let reference = FactRef::new(FactFamily::Import, 7);
        let metadata = FactMeta {
            stable_key: stable_key_for_test("import:stable"),
            producer_id: "polint.go.syntax",
            layer_id: "polint.go.syntax",
            precision: FactPrecision::Syntax,
            confidence: FactConfidence::High,
            validation: ValidationStatus::NativeTrusted,
            payload_digest: "payload".to_string(),
        };

        assert_eq!(reference.run_id, 7);
        assert_eq!(metadata.stable_key, stable_key_for_test("import:stable"));
    }

    /// The exact bytes matter: this text is interned as a fact's stable key and
    /// hashed into its payload digest, both of which reach reports.
    #[test]
    fn stable_key_text_is_length_prefixed_sorted_and_path_normalized() {
        let mut text = String::from("left over from an earlier fact");

        write_stable_key_text(
            &mut text,
            FactFamily::Import,
            &mut [
                ("path", "src\\main.go"),
                ("import_path", "fmt"),
                ("path", "b"),
            ],
        );

        assert_eq!(
            text,
            "6:Import|11:import_path=3:fmt|4:path=11:src/main.go|4:path=1:b"
        );
    }

    #[test]
    fn stable_key_from_parts_sorts_and_normalizes_length_prefixed_parts() {
        let interner = StableKeyInterner::default();
        let first = stable_key_from_parts(
            &interner,
            FactFamily::Import,
            &[
                ("path", "src\\main.go".to_string()),
                ("import_path", "fmt".to_string()),
            ],
        );
        let second = stable_key_from_parts(
            &interner,
            FactFamily::Import,
            &[
                ("import_path", "fmt".to_string()),
                ("path", "src/main.go".to_string()),
            ],
        );

        assert_eq!(first, second);
        let first = interner.resolve(first);
        assert!(first.contains("6:Import"));
        assert!(first.contains("4:path=11:src/main.go"));
    }

    #[test]
    fn fact_meta_store_rows_are_deterministically_ordered() {
        let mut store = FactMetaStore::default();
        let later = FactRef::new(FactFamily::Import, 10);
        let earlier = FactRef::new(FactFamily::Import, 2);

        store.insert(later, test_meta("function", "function"));
        store.insert(earlier, test_meta("import", "import"));

        let rows = store.rows().collect::<Vec<_>>();

        assert_eq!(rows[0].0, earlier);
        assert_eq!(rows[1].0, later);
    }

    #[test]
    fn stable_key_conflict_insert_is_idempotent_when_same_reference_payload_repeats() {
        let mut store = FactMetaStore::default();
        let reference = FactRef::new(FactFamily::Import, 7);

        let first = store.insert(reference, test_meta("import:key", "payload:a"));
        let second = store.insert(reference, test_meta("import:key", "payload:a"));

        assert_eq!(first, FactMetaInsert::Inserted);
        assert_eq!(second, FactMetaInsert::Idempotent);
    }

    #[test]
    fn stable_key_conflict_insert_records_duplicate_reference_for_same_payload() {
        let mut store = FactMetaStore::default();
        let first_ref = FactRef::new(FactFamily::Import, 1);
        let second_ref = FactRef::new(FactFamily::Import, 2);
        let key = stable_key_for_test("import:key");

        store.insert(first_ref, test_meta("import:key", "payload:a"));
        let result = store.insert(second_ref, test_meta("import:key", "payload:a"));

        assert_eq!(
            result,
            FactMetaInsert::Conflict {
                family: FactFamily::Import,
                stable_key: key,
                existing: first_ref,
                incoming: second_ref,
            }
        );
        assert_eq!(store.stable_key_conflicts().count(), 1);
        assert_eq!(
            store
                .stable_key_owner(FactFamily::Import, key)
                .map(|owner| owner.reference),
            Some(first_ref)
        );
    }

    #[test]
    fn finish_family_insertions_keeps_rows_and_conflicts_but_drops_owner_index() {
        let mut store = FactMetaStore::default();
        let first_ref = FactRef::new(FactFamily::Import, 1);
        let second_ref = FactRef::new(FactFamily::Import, 2);
        let key = stable_key_for_test("import:key");

        store.insert(first_ref, test_meta("import:key", "payload:a"));
        store.insert(second_ref, test_meta("import:key", "payload:b"));
        store.finish_family_insertions(FactFamily::Import);

        assert!(store.get(first_ref).is_some());
        assert!(store.get(second_ref).is_some());
        assert_eq!(store.stable_key_conflicts().count(), 1);
        assert!(store.stable_key_owner(FactFamily::Import, key).is_none());
    }

    #[test]
    fn remove_family_drops_rows_owners_and_conflicts_for_that_family_only() {
        let mut store = FactMetaStore::default();
        let import_ref = FactRef::new(FactFamily::Import, 1);
        let duplicate_import_ref = FactRef::new(FactFamily::Import, 2);
        let function_ref = FactRef::new(FactFamily::Function, 1);
        let shared = stable_key_for_test("shared:key");
        let function_key = stable_key_for_test("function:key");

        store.insert(import_ref, test_meta("shared:key", "payload:a"));
        store.insert(duplicate_import_ref, test_meta("shared:key", "payload:b"));
        store.insert(function_ref, test_meta("function:key", "payload:c"));

        store.remove_family(FactFamily::Import);

        assert!(store.get(import_ref).is_none());
        assert!(store.get(duplicate_import_ref).is_none());
        assert!(store.stable_key_owner(FactFamily::Import, shared).is_none());
        assert_eq!(store.stable_key_conflicts().count(), 0);
        assert!(store.get(function_ref).is_some());
        assert!(
            store
                .stable_key_owner(FactFamily::Function, function_key)
                .is_some()
        );
    }

    #[test]
    fn fact_meta_store_accepts_sparse_high_run_ids_without_dense_resize() {
        let mut store = FactMetaStore::default();
        let high_ref = FactRef::new(FactFamily::Import, u64::MAX);
        let low_ref = FactRef::new(FactFamily::Import, 0);
        let high_key = stable_key_for_test("import:high");
        let low_key = stable_key_for_test("import:low");

        store.insert(high_ref, test_meta("import:high", "payload:high"));
        store.insert(low_ref, test_meta("import:low", "payload:low"));

        assert_eq!(store.get(high_ref).unwrap().stable_key, high_key);
        assert_eq!(store.get(low_ref).unwrap().stable_key, low_key);
        assert_eq!(
            store
                .rows()
                .map(|(reference, metadata)| (reference, metadata.stable_key))
                .collect::<Vec<_>>(),
            vec![(low_ref, low_key), (high_ref, high_key)]
        );
    }

    #[test]
    fn stable_key_conflict_insert_records_conflicts_deterministically() {
        let mut store = FactMetaStore::default();
        let import_owner_ref = FactRef::new(FactFamily::Import, 8);
        let import_conflict_ref = FactRef::new(FactFamily::Import, 3);
        let key = stable_key_for_test("import:key");

        store.insert(import_owner_ref, test_meta("import:key", "payload:a"));
        let result = store.insert(import_conflict_ref, test_meta("import:key", "payload:b"));
        store.insert(import_conflict_ref, test_meta("import:key", "payload:b"));

        let conflicts = store.stable_key_conflicts().collect::<Vec<_>>();

        assert_eq!(
            result,
            FactMetaInsert::Conflict {
                family: FactFamily::Import,
                stable_key: key,
                existing: import_owner_ref,
                incoming: import_conflict_ref,
            }
        );
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].family, FactFamily::Import);
        assert_eq!(conflicts[0].stable_key, key);
        assert_eq!(conflicts[0].existing, import_owner_ref);
        assert_eq!(conflicts[0].incoming, import_conflict_ref);
    }

    fn test_meta(stable_key: &str, payload_digest: &str) -> FactMeta {
        FactMeta {
            stable_key: stable_key_for_test(stable_key),
            producer_id: "polint.go.syntax",
            layer_id: "polint.go.syntax",
            precision: FactPrecision::Syntax,
            confidence: FactConfidence::High,
            validation: ValidationStatus::NativeTrusted,
            payload_digest: payload_digest.to_string(),
        }
    }
}
