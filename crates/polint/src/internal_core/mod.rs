//! Foundational types for the polint workspace.
//!
//! This crate must not name concrete analysis facts or depend on other polint crates.

mod diagnostic;
mod ids;
mod lang;
mod language_id;
mod span;
mod stable_key;

pub use diagnostic::{
    Diagnostic, Evidence, EvidenceBundleId, EvidenceBundleRef, Fix, Label, Severity,
    StructuredEvidenceV1, Suggestion, TextRange as DiagnosticRange, diagnostic_fingerprint,
    fingerprint,
};
pub use ids::{
    BranchId, DefinitionId, FileId, FunctionId, ImportId, ModuleEdgeId, ModuleNodeId, NodeId,
    PackageId, ReferenceId, ResolvedImportId, RuleId, SymbolId,
};
pub use lang::Language;
pub use language_id::{
    LANGUAGE_IDS_GO, LANGUAGE_IDS_GO_AND_TS, LANGUAGE_IDS_NONE, LANGUAGE_IDS_TS, LanguageId,
};
pub(crate) use span::{SourceTextIndex, span_from_byte_range};
pub use span::{Span, TextRange};
pub(crate) use stable_key::{
    KeyPart, push_decimal, push_length_prefixed, push_length_prefixed_path,
};
pub use stable_key::{StableKeyId, StableKeyInterner};

#[doc(hidden)]
pub use stable_key::{stable_key_for_test, test_stable_key_interner};
