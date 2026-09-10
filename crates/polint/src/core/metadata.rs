//! Fact metadata construction, digests, fingerprints, and early status mappers.
//!
//! Free helpers used by `AnalysisDb` metadata methods; extracted from the core
//! monolith without behaviour changes.

use super::SOURCE_PROVIDER_ID;
use super::facts::{
    ModuleEdgeKind, ModuleNodeKind, ResolutionPrecision, ResolutionStatus, SymbolPrecision,
    SymbolResolutionStatus, UnresolvedReason,
};
use super::labels::{none_value, topology_precision_metadata};
use super::lang::Language;
use super::span::Span;
use crate::analysis::adaptation::facts::AcceptedModelFact;
use crate::analysis::calls::facts::{CallPrecision, CallTargetStatus};
use crate::analysis::cfg::facts::{CfgPrecision, CfgStatus};
use crate::analysis::entrypoints::facts::{EntrypointPrecision, EntrypointStatus};
use crate::analysis::extensions::sinks::{ExtensionFactConfidence, ExtensionFactPrecision};
use crate::analysis::extensions::store::AcceptedExtensionFact;
use crate::analysis::mir::body::MirStatus;
use crate::analysis::places::PlaceStatus;
use crate::analysis::summaries::facts::{SummaryEventFact, SummaryFact};
use crate::analysis_kernel::{
    FactConfidence, FactFamily, FactMeta, FactPrecision, ValidationStatus, write_stable_key_text,
};
use crate::analysis_neutral::domains::facts::{DomainPrecision, DomainStatus};
use crate::core::StableKeyId;
use crate::module_graph::topology::TopologyPrecision;
use crate::symbol_graph::semantic::{
    AliasFact, ExportFact, GeneratedSymbolFact, ResolutionFact, ScopeFact, SemanticImportFact,
    SemanticStatus, StableExportIdentity,
};
use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::Ordering;

pub(super) fn normalize_scope_facts(
    interner: &crate::core::StableKeyInterner,
    facts: &mut [ScopeFact],
) {
    for fact in facts.iter_mut() {
        if interner.resolve(fact.stable_key).is_empty() {
            fact.stable_key = fact.computed_stable_key(interner);
        }
    }
    facts.sort_by_key(|fact| interner.resolve(fact.stable_key));
}

pub(super) fn normalize_semantic_import_facts(
    interner: &crate::core::StableKeyInterner,
    facts: &mut [SemanticImportFact],
) {
    for fact in facts.iter_mut() {
        if interner.resolve(fact.stable_key).is_empty() {
            fact.stable_key =
                crate::symbol_graph::semantic::semantic_import_computed_stable_key(fact, interner);
        }
    }
    facts.sort_by_key(|fact| interner.resolve(fact.stable_key));
}

pub(super) fn normalize_export_facts(
    interner: &crate::core::StableKeyInterner,
    facts: &mut [ExportFact],
) {
    for fact in facts.iter_mut() {
        if interner.resolve(fact.stable_key).is_empty() {
            fact.stable_key = fact.computed_stable_key(interner);
        }
    }
    facts.sort_by_key(|fact| interner.resolve(fact.stable_key));
}

pub(super) fn normalize_alias_facts(
    interner: &crate::core::StableKeyInterner,
    facts: &mut [AliasFact],
) {
    for fact in facts.iter_mut() {
        if interner.resolve(fact.stable_key).is_empty() {
            fact.stable_key = fact.computed_stable_key(interner);
        }
    }
    facts.sort_by_key(|fact| interner.resolve(fact.stable_key));
}

pub(super) fn normalize_resolution_facts(
    interner: &crate::core::StableKeyInterner,
    facts: &mut [ResolutionFact],
) {
    for fact in facts.iter_mut() {
        if interner.resolve(fact.stable_key).is_empty() {
            fact.stable_key = fact.computed_stable_key(interner);
        }
    }
    facts.sort_by_key(|fact| interner.resolve(fact.stable_key));
}

pub(super) fn normalize_generated_symbol_facts(
    interner: &crate::core::StableKeyInterner,
    facts: &mut [GeneratedSymbolFact],
) {
    for fact in facts.iter_mut() {
        if interner.resolve(fact.stable_key).is_empty() {
            fact.stable_key = fact.computed_stable_key(interner);
        }
    }
    facts.sort_by_key(|fact| interner.resolve(fact.stable_key));
}

pub(super) fn normalize_stable_export_identities(
    interner: &crate::core::StableKeyInterner,
    facts: &mut [StableExportIdentity],
) {
    for fact in facts.iter_mut() {
        if interner.resolve(fact.stable_key).is_empty() {
            fact.stable_key = fact.computed_stable_key(interner);
        }
    }
    facts.sort_by_key(|fact| interner.resolve(fact.stable_key));
}

pub(super) fn source_file_metadata(
    interner: &crate::core::StableKeyInterner,
    relative_path: &str,
    language: Language,
    content_hash: &str,
) -> FactMeta {
    fact_meta_from_parts(
        interner,
        FactFamily::SourceFile,
        SOURCE_PROVIDER_ID,
        FactPrecision::Exact,
        FactConfidence::High,
        stable_parts([
            ("path", relative_path.to_string()),
            ("content_hash", content_hash.to_string()),
        ]),
        stable_parts([("language", language_label(language).to_string())]),
    )
}

#[allow(
    dead_code,
    reason = "Extension fact metadata is reached through extension provider wiring in the next plan."
)]
pub(super) fn extension_fact_metadata(
    interner: &crate::core::StableKeyInterner,
    fact: &AcceptedExtensionFact,
) -> FactMeta {
    let producer_id = leaked_extension_producer_id(&fact.extension_id, &fact.provider_id);
    let precision = extension_precision_metadata(fact.precision);
    let confidence = extension_confidence_metadata(fact.confidence);
    let stable_key = fact.stable_key;
    let payload_extra_parts = [
        ("family", fact.fact_family.clone()),
        ("bindings", fact.binding_refs.join(",")),
        ("evidence", fact.evidence.join(",")),
        ("payload", fact.payload_labels.join(",")),
        ("status", format!("{:?}", fact.status)),
    ];
    let payload_digest =
        metadata_payload_digest_for_key(interner, stable_key, &payload_extra_parts);

    FactMeta {
        stable_key,
        producer_id,
        layer_id: producer_id,
        precision,
        confidence,
        validation: ValidationStatus::SchemaValidated,
        payload_digest,
    }
}

pub(super) fn adaptation_model_fact_metadata(
    interner: &crate::core::StableKeyInterner,
    fact: &AcceptedModelFact,
) -> FactMeta {
    let stable_key = fact.fact.stable_key;
    FactMeta {
        stable_key,
        producer_id: "polint.adaptation.model",
        layer_id: "polint.adaptation.model",
        precision: FactPrecision::Heuristic,
        confidence: FactConfidence::Medium,
        validation: ValidationStatus::SchemaValidated,
        payload_digest: metadata_payload_digest(
            interner.resolve(stable_key).as_ref(),
            &[
                ("model_path", fact.fact.model_path.clone()),
                ("source_pattern", fact.fact.source_pattern.clone()),
                ("target_pattern", fact.fact.target_pattern.clone()),
                ("confidence", fact.fact.confidence.as_str().to_string()),
                ("language", fact.fact.language.as_str().to_string()),
                ("scope", fact.fact.scope.clone()),
                ("evidence", fact.fact.evidence.join(",")),
            ],
        ),
    }
}

#[allow(
    dead_code,
    reason = "Extension producer ids are reached through extension provider wiring in the next plan."
)]
pub(super) fn leaked_extension_producer_id(extension_id: &str, provider_id: &str) -> &'static str {
    Box::leak(format!("polint.extension.{extension_id}.{provider_id}").into_boxed_str())
}

#[allow(
    dead_code,
    reason = "Extension precision mapping is reached through extension provider wiring in the next plan."
)]
pub(super) fn extension_precision_metadata(precision: ExtensionFactPrecision) -> FactPrecision {
    match precision {
        ExtensionFactPrecision::Exact => FactPrecision::Exact,
        ExtensionFactPrecision::SetupAware => FactPrecision::SetupAware,
        ExtensionFactPrecision::Heuristic => FactPrecision::Heuristic,
        ExtensionFactPrecision::GeneratedUnvalidated => FactPrecision::Heuristic,
    }
}

#[allow(
    dead_code,
    reason = "Extension confidence mapping is reached through extension provider wiring in the next plan."
)]
pub(super) fn extension_confidence_metadata(confidence: ExtensionFactConfidence) -> FactConfidence {
    match confidence {
        ExtensionFactConfidence::High => FactConfidence::High,
        ExtensionFactConfidence::Medium => FactConfidence::Medium,
        ExtensionFactConfidence::Low => FactConfidence::Low,
    }
}

pub(super) fn fact_meta_from_parts<const STABLE: usize, const EXTRA: usize>(
    interner: &crate::core::StableKeyInterner,
    family: FactFamily,
    producer_id: &'static str,
    precision: FactPrecision,
    confidence: FactConfidence,
    stable_parts: [(&'static str, String); STABLE],
    payload_extra_parts: [(&'static str, String); EXTRA],
) -> FactMeta {
    fact_meta_from_borrowed_parts(
        interner,
        family,
        producer_id,
        precision,
        confidence,
        stable_parts
            .each_ref()
            .map(|(label, value)| (*label, value.as_str())),
        payload_extra_parts
            .each_ref()
            .map(|(label, value)| (*label, value.as_str())),
    )
}

thread_local! {
    /// Reused across fact-metadata construction: the stable-key text is built
    /// once per fact and thrown away as soon as it has been interned, so it does
    /// not deserve a fresh allocation each time.
    static STABLE_KEY_TEXT: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Builds fact metadata from parts the caller already owns.
///
/// Preferred over [`fact_meta_from_parts`] on paths that restore many facts:
/// values such as a file path, a language label, or a string literal's text are
/// borrowed straight from the fact rather than cloned into the parts array.
pub(super) fn fact_meta_from_borrowed_parts<const STABLE: usize, const EXTRA: usize>(
    interner: &crate::core::StableKeyInterner,
    family: FactFamily,
    producer_id: &'static str,
    precision: FactPrecision,
    confidence: FactConfidence,
    stable_parts: [(&'static str, &str); STABLE],
    payload_extra_parts: [(&'static str, &str); EXTRA],
) -> FactMeta {
    let (stable_key, payload_digest) = STABLE_KEY_TEXT.with(|buffer| {
        let mut buffer = buffer.borrow_mut();
        let mut sorted = stable_parts;
        write_stable_key_text(&mut buffer, family, &mut sorted);
        // The buffer already holds exactly the text the digest hashes, so the
        // interner is asked for the id only — never to hand the text back.
        let stable_key = interner.intern(buffer.as_str());
        let mut payload_parts = stable_parts.to_vec();
        payload_parts.extend(payload_extra_parts);
        (stable_key, metadata_payload_digest(&buffer, &payload_parts))
    });

    FactMeta {
        stable_key,
        producer_id,
        layer_id: producer_id,
        precision,
        confidence,
        validation: ValidationStatus::NativeTrusted,
        payload_digest,
    }
}

pub(super) fn fact_meta_from_stable_key<const EXTRA: usize>(
    interner: &crate::core::StableKeyInterner,
    producer_id: &'static str,
    precision: FactPrecision,
    confidence: FactConfidence,
    stable_key: StableKeyId,
    payload_extra_parts: [(&'static str, String); EXTRA],
) -> FactMeta {
    let payload_digest =
        metadata_payload_digest_for_key(interner, stable_key, &payload_extra_parts);

    FactMeta {
        stable_key,
        producer_id,
        layer_id: producer_id,
        precision,
        confidence,
        validation: ValidationStatus::NativeTrusted,
        payload_digest,
    }
}

pub(super) fn fact_meta_from_stable_key_with_validation<const EXTRA: usize>(
    interner: &crate::core::StableKeyInterner,
    producer_id: &'static str,
    precision: FactPrecision,
    confidence: FactConfidence,
    validation: ValidationStatus,
    stable_key: StableKeyId,
    payload_extra_parts: [(&'static str, String); EXTRA],
) -> FactMeta {
    let payload_digest =
        metadata_payload_digest_for_key(interner, stable_key, &payload_extra_parts);

    FactMeta {
        stable_key,
        producer_id,
        layer_id: producer_id,
        precision,
        confidence,
        validation,
        payload_digest,
    }
}

pub(super) fn topology_fact_metadata(
    interner: &crate::core::StableKeyInterner,
    producer_id: &'static str,
    precision: TopologyPrecision,
    stable_key: StableKeyId,
) -> FactMeta {
    let (precision, confidence) = topology_precision_metadata(precision);
    fact_meta_from_stable_key(
        interner,
        producer_id,
        precision,
        confidence,
        stable_key,
        stable_parts([]),
    )
}

pub(super) fn stable_parts<T, const N: usize>(
    parts: [(&'static str, T); N],
) -> [(&'static str, T); N] {
    parts
}

/// `true` / `false` without the allocation `bool::to_string` would make.
pub(super) fn bool_label(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

pub(super) fn metadata_payload_digest<V: AsRef<str>>(
    stable_key: &str,
    parts: &[(&'static str, V)],
) -> String {
    let mut normalized = parts
        .iter()
        .map(|(label, value)| (*label, metadata_value(value.as_ref())))
        .collect::<Vec<_>>();
    normalized.sort_by(compare_metadata_parts);

    let mut hash = 0xcbf29ce484222325_u64;
    fingerprint_part(&mut hash, stable_key.as_bytes());
    for (label, value) in &normalized {
        fingerprint_metadata_part(&mut hash, label, value.as_ref());
    }
    lower_hex_u64(hash)
}

/// Payload digest for a fact whose stable key is already interned.
///
/// Streams the key's canonical bytes into the fingerprint rather than expanding
/// them into a string first. This runs once per fact, and a composite key's text
/// is wanted here one byte at a time — materializing it would reintroduce the
/// per-fact expansion that structural sharing removes.
pub(super) fn metadata_payload_digest_for_key<V: AsRef<str>>(
    interner: &crate::core::StableKeyInterner,
    stable_key: StableKeyId,
    parts: &[(&'static str, V)],
) -> String {
    let mut normalized = parts
        .iter()
        .map(|(label, value)| (*label, metadata_value(value.as_ref())))
        .collect::<Vec<_>>();
    normalized.sort_by(compare_metadata_parts);

    let mut hash = 0xcbf29ce484222325_u64;
    interner.stream_canonical(stable_key, |chunk| {
        for byte in chunk {
            fingerprint_byte(&mut hash, *byte);
        }
    });
    fingerprint_separator(&mut hash);
    for (label, value) in &normalized {
        fingerprint_metadata_part(&mut hash, label, value.as_ref());
    }
    lower_hex_u64(hash)
}

pub(super) fn summary_fact_payload_metadata_digest(
    interner: &crate::core::StableKeyInterner,
    fact: &SummaryFact,
) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    interner.stream_canonical(fact.stable_key, |chunk| {
        for byte in chunk {
            fingerprint_byte(&mut hash, *byte);
        }
    });
    fingerprint_separator(&mut hash);
    fingerprint_normalized_metadata_part(
        &mut hash,
        "callable",
        interner.resolve(fact.callable_stable_key).as_ref(),
    );
    fingerprint_normalized_metadata_part(&mut hash, "domain", fact.domain.as_str());
    fingerprint_normalized_metadata_part(&mut hash, "payload_digest", &fact.payload_digest);
    fingerprint_normalized_metadata_part(&mut hash, "precision", fact.precision.as_str());
    fingerprint_normalized_metadata_part(&mut hash, "provenance", fact.provenance.as_str());
    fingerprint_normalized_metadata_part(&mut hash, "status", fact.status.as_str());
    lower_hex_u64(hash)
}

pub(super) fn summary_event_payload_metadata_digest(
    interner: &crate::core::StableKeyInterner,
    fact: &SummaryEventFact,
) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    interner.stream_canonical(fact.stable_key, |chunk| {
        for byte in chunk {
            fingerprint_byte(&mut hash, *byte);
        }
    });
    fingerprint_separator(&mut hash);
    fingerprint_normalized_metadata_part(
        &mut hash,
        "callable",
        interner.resolve(fact.callable_stable_key).as_ref(),
    );
    fingerprint_normalized_metadata_part(&mut hash, "domain", fact.domain.as_str());
    fingerprint_normalized_metadata_part(&mut hash, "event_kind", &fact.event_kind);
    fingerprint_normalized_metadata_part(&mut hash, "precision", fact.precision.as_str());
    fingerprint_normalized_metadata_part(&mut hash, "reason", &fact.reason);
    fingerprint_normalized_metadata_part(&mut hash, "status", fact.status.as_str());
    lower_hex_u64(hash)
}

pub(super) fn lower_hex_u64(value: u64) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(16);
    for shift in (0..64).step_by(4).rev() {
        let nibble = ((value >> shift) & 0x0f) as usize;
        output.push(char::from(HEX[nibble]));
    }
    output
}

pub(super) fn metadata_value(value: &str) -> Cow<'_, str> {
    if value.contains('\\') {
        Cow::Owned(value.replace('\\', "/"))
    } else {
        Cow::Borrowed(value)
    }
}

pub(super) fn compare_metadata_parts(
    left: &(&'static str, Cow<'_, str>),
    right: &(&'static str, Cow<'_, str>),
) -> Ordering {
    let mut index = 0;
    loop {
        match (
            metadata_part_byte(left.0, left.1.as_ref(), index),
            metadata_part_byte(right.0, right.1.as_ref(), index),
        ) {
            (Some(left), Some(right)) => match left.cmp(&right) {
                Ordering::Equal => index += 1,
                ordering => return ordering,
            },
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (None, None) => return Ordering::Equal,
        }
    }
}

pub(super) fn metadata_part_byte(label: &str, value: &str, index: usize) -> Option<u8> {
    if index < label.len() {
        return Some(label.as_bytes()[index]);
    }
    if index == label.len() {
        return Some(b'=');
    }
    value.as_bytes().get(index - label.len() - 1).copied()
}

pub(super) fn fingerprint_metadata_part(hash: &mut u64, label: &str, value: &str) {
    for byte in label.as_bytes() {
        fingerprint_byte(hash, *byte);
    }
    fingerprint_byte(hash, b'=');
    for byte in value.as_bytes() {
        fingerprint_byte(hash, *byte);
    }
    fingerprint_separator(hash);
}

pub(super) fn fingerprint_normalized_metadata_part(hash: &mut u64, label: &str, value: &str) {
    for byte in label.as_bytes() {
        fingerprint_byte(hash, *byte);
    }
    fingerprint_byte(hash, b'=');
    for byte in value.bytes() {
        fingerprint_byte(hash, if byte == b'\\' { b'/' } else { byte });
    }
    fingerprint_separator(hash);
}

pub(super) fn fingerprint_part(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        fingerprint_byte(hash, *byte);
    }
    fingerprint_separator(hash);
}

pub(super) fn fingerprint_byte(hash: &mut u64, byte: u8) {
    *hash ^= u64::from(byte);
    *hash = hash.wrapping_mul(0x100000001b3);
}

pub(super) fn fingerprint_separator(hash: &mut u64) {
    fingerprint_byte(hash, 0xff);
}

pub(super) fn span_metadata_value(span: &Span) -> String {
    format!(
        "{}-{}:{}:{}-{}:{}",
        span.start_byte,
        span.end_byte,
        span.start_line,
        span.start_col,
        span.end_line,
        span.end_col
    )
}

pub(super) fn option_span_metadata_value(span: Option<&Span>) -> String {
    span.map(span_metadata_value).unwrap_or_else(none_value)
}

pub(super) fn language_label(language: Language) -> &'static str {
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

pub(super) fn module_node_kind_label(kind: ModuleNodeKind) -> &'static str {
    match kind {
        ModuleNodeKind::File => "file",
        ModuleNodeKind::Package => "package",
        ModuleNodeKind::Module => "module",
        ModuleNodeKind::External => "external",
        _ => "unknown",
    }
}

pub(super) fn module_edge_kind_label(kind: ModuleEdgeKind) -> &'static str {
    match kind {
        ModuleEdgeKind::Contains => "contains",
        ModuleEdgeKind::Imports => "imports",
        ModuleEdgeKind::DependsOn => "depends_on",
        _ => "unknown",
    }
}

pub(super) fn resolution_status_label(status: ResolutionStatus) -> &'static str {
    match status {
        ResolutionStatus::Resolved => "resolved",
        ResolutionStatus::External => "external",
        ResolutionStatus::Unresolved => "unresolved",
        ResolutionStatus::SetupMissing => "setup_missing",
        ResolutionStatus::Dynamic => "dynamic",
        ResolutionStatus::Unsupported => "unsupported",
        _ => "unknown",
    }
}

pub(super) fn resolution_precision_label(precision: ResolutionPrecision) -> &'static str {
    match precision {
        ResolutionPrecision::ExactFile => "exact_file",
        ResolutionPrecision::Package => "package",
        ResolutionPrecision::ExternalPackage => "external_package",
        ResolutionPrecision::Heuristic => "heuristic",
        ResolutionPrecision::None => "none",
        _ => "unknown",
    }
}

pub(super) fn unresolved_reason_label(reason: UnresolvedReason) -> &'static str {
    match reason {
        UnresolvedReason::NotFound => "not_found",
        UnresolvedReason::SetupMissing => "setup_missing",
        UnresolvedReason::DynamicExpression => "dynamic_expression",
        UnresolvedReason::UnsupportedLanguage => "unsupported_language",
        UnresolvedReason::UnsupportedImport => "unsupported_import",
        UnresolvedReason::ResolverError => "resolver_error",
        UnresolvedReason::OutsideWorkspace => "outside_workspace",
        _ => "unknown",
    }
}

pub(super) fn symbol_precision_label(precision: SymbolPrecision) -> &'static str {
    match precision {
        SymbolPrecision::ExactSemantic => "exact_semantic",
        SymbolPrecision::ExactLocal => "exact_local",
        SymbolPrecision::ModuleLinked => "module_linked",
        SymbolPrecision::Heuristic => "heuristic",
        SymbolPrecision::Unresolved => "unresolved",
        SymbolPrecision::Ambiguous => "ambiguous",
        SymbolPrecision::SetupMissing => "setup_missing",
        SymbolPrecision::Unsupported => "unsupported",
        _ => "unknown",
    }
}

pub(super) fn symbol_resolution_status_label(status: SymbolResolutionStatus) -> &'static str {
    match status {
        SymbolResolutionStatus::Resolved => "resolved",
        SymbolResolutionStatus::Unresolved => "unresolved",
        SymbolResolutionStatus::Ambiguous => "ambiguous",
        SymbolResolutionStatus::SetupMissing => "setup_missing",
        SymbolResolutionStatus::Unsupported => "unsupported",
        _ => "unknown",
    }
}

pub(super) fn semantic_status_metadata(status: SemanticStatus) -> (FactPrecision, FactConfidence) {
    match status {
        SemanticStatus::Resolved | SemanticStatus::Generated => {
            (FactPrecision::SetupAware, FactConfidence::High)
        }
        SemanticStatus::Ambiguous => (FactPrecision::Ambiguous, FactConfidence::Medium),
        SemanticStatus::Unresolved => (FactPrecision::Unresolved, FactConfidence::Medium),
        SemanticStatus::Cycle | SemanticStatus::Unsupported => {
            (FactPrecision::Unsupported, FactConfidence::Low)
        }
        SemanticStatus::Dynamic => (FactPrecision::Heuristic, FactConfidence::Low),
        SemanticStatus::External => (FactPrecision::SetupAware, FactConfidence::Medium),
        SemanticStatus::SetupMissing => (FactPrecision::SetupMissing, FactConfidence::High),
    }
}

pub(super) fn mir_status_metadata(status: MirStatus) -> (FactPrecision, FactConfidence) {
    match status {
        MirStatus::Resolved => (FactPrecision::SetupAware, FactConfidence::High),
        MirStatus::Partial => (FactPrecision::Heuristic, FactConfidence::Medium),
        MirStatus::Unknown => (FactPrecision::Unresolved, FactConfidence::Low),
        MirStatus::Unsupported => (FactPrecision::Unsupported, FactConfidence::Low),
    }
}

pub(super) fn place_status_metadata(status: PlaceStatus) -> (FactPrecision, FactConfidence) {
    match status {
        PlaceStatus::Resolved => (FactPrecision::SetupAware, FactConfidence::High),
        PlaceStatus::Partial => (FactPrecision::Heuristic, FactConfidence::Medium),
        PlaceStatus::Unknown => (FactPrecision::Unresolved, FactConfidence::Low),
        PlaceStatus::Unsupported => (FactPrecision::Unsupported, FactConfidence::Low),
    }
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb CFG metadata until dual accessors are removed."
)]
pub(super) fn cfg_status_metadata(
    status: CfgStatus,
    precision: CfgPrecision,
) -> (FactPrecision, FactConfidence) {
    let fact_precision = match (status, precision) {
        (CfgStatus::Resolved, CfgPrecision::ExactSyntax) => FactPrecision::Syntax,
        (CfgStatus::Resolved, CfgPrecision::ExactLowered | CfgPrecision::SetupAware) => {
            FactPrecision::SetupAware
        }
        (_, CfgPrecision::Conservative | CfgPrecision::Heuristic) => FactPrecision::Heuristic,
        (CfgStatus::Partial, _) => FactPrecision::Heuristic,
        (CfgStatus::Unknown, _) | (_, CfgPrecision::Unknown) => FactPrecision::Unresolved,
        (CfgStatus::Unsupported, _) | (_, CfgPrecision::Unsupported) => FactPrecision::Unsupported,
    };
    let confidence = match status {
        CfgStatus::Resolved => FactConfidence::High,
        CfgStatus::Partial => FactConfidence::Medium,
        CfgStatus::Unknown | CfgStatus::Unsupported => FactConfidence::Low,
    };
    (fact_precision, confidence)
}

pub(super) fn call_status_metadata(
    status: CallTargetStatus,
    precision: CallPrecision,
) -> (FactPrecision, FactConfidence) {
    let fact_precision = match status {
        CallTargetStatus::Resolved => match precision {
            CallPrecision::Exact | CallPrecision::SetupAware => FactPrecision::SetupAware,
            CallPrecision::Conservative | CallPrecision::Heuristic => FactPrecision::Heuristic,
            CallPrecision::Ambiguous => FactPrecision::Ambiguous,
            CallPrecision::Unknown => FactPrecision::Unresolved,
            CallPrecision::Unsupported => FactPrecision::Unsupported,
        },
        CallTargetStatus::Ambiguous => FactPrecision::Ambiguous,
        CallTargetStatus::Unresolved | CallTargetStatus::BudgetExceeded => {
            FactPrecision::Unresolved
        }
        CallTargetStatus::Unsupported | CallTargetStatus::Rejected => FactPrecision::Unsupported,
        CallTargetStatus::SetupMissing => FactPrecision::SetupMissing,
    };
    let confidence = match status {
        CallTargetStatus::Resolved => FactConfidence::High,
        CallTargetStatus::Ambiguous => FactConfidence::Medium,
        CallTargetStatus::Unresolved
        | CallTargetStatus::Unsupported
        | CallTargetStatus::SetupMissing
        | CallTargetStatus::BudgetExceeded
        | CallTargetStatus::Rejected => FactConfidence::Low,
    };
    (fact_precision, confidence)
}

pub(super) fn domain_status_metadata(
    status: DomainStatus,
    precision: DomainPrecision,
) -> (FactPrecision, FactConfidence) {
    let fact_precision = match status {
        DomainStatus::Present => match precision {
            DomainPrecision::ExactLocal => FactPrecision::SetupAware,
            DomainPrecision::SetupAware => FactPrecision::SetupAware,
            DomainPrecision::Conservative | DomainPrecision::Heuristic => FactPrecision::Heuristic,
            DomainPrecision::Unknown => FactPrecision::Unresolved,
            DomainPrecision::Unsupported => FactPrecision::Unsupported,
        },
        DomainStatus::Top => FactPrecision::Ambiguous,
        DomainStatus::Unknown | DomainStatus::BudgetExceeded => FactPrecision::Unresolved,
        DomainStatus::Unsupported => FactPrecision::Unsupported,
        DomainStatus::SetupMissing => FactPrecision::SetupMissing,
    };
    let confidence = match status {
        DomainStatus::Present => match precision {
            DomainPrecision::ExactLocal | DomainPrecision::SetupAware => FactConfidence::High,
            DomainPrecision::Conservative | DomainPrecision::Heuristic => FactConfidence::Medium,
            DomainPrecision::Unknown | DomainPrecision::Unsupported => FactConfidence::Low,
        },
        DomainStatus::Top => FactConfidence::Medium,
        DomainStatus::Unknown
        | DomainStatus::Unsupported
        | DomainStatus::SetupMissing
        | DomainStatus::BudgetExceeded => FactConfidence::Low,
    };
    (fact_precision, confidence)
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb metadata until dual accessors are removed."
)]
pub(super) fn entrypoint_precision_metadata(
    status: EntrypointStatus,
    precision: EntrypointPrecision,
) -> (FactPrecision, FactConfidence) {
    let fact_precision = match status {
        EntrypointStatus::Resolved => match precision {
            EntrypointPrecision::ResolvedStatic | EntrypointPrecision::SetupAware => {
                FactPrecision::SetupAware
            }
            EntrypointPrecision::Heuristic | EntrypointPrecision::Conservative => {
                FactPrecision::Heuristic
            }
            EntrypointPrecision::Unknown => FactPrecision::Unresolved,
        },
        EntrypointStatus::Partial => FactPrecision::Ambiguous,
        EntrypointStatus::Unresolved => FactPrecision::Unresolved,
        EntrypointStatus::SetupMissing => FactPrecision::SetupMissing,
        EntrypointStatus::Unsupported => FactPrecision::Unsupported,
    };
    let confidence = match status {
        EntrypointStatus::Resolved => match precision {
            EntrypointPrecision::ResolvedStatic | EntrypointPrecision::SetupAware => {
                FactConfidence::High
            }
            EntrypointPrecision::Heuristic | EntrypointPrecision::Conservative => {
                FactConfidence::Medium
            }
            EntrypointPrecision::Unknown => FactConfidence::Low,
        },
        EntrypointStatus::Partial => FactConfidence::Medium,
        EntrypointStatus::Unresolved
        | EntrypointStatus::SetupMissing
        | EntrypointStatus::Unsupported => FactConfidence::Low,
    };
    (fact_precision, confidence)
}
