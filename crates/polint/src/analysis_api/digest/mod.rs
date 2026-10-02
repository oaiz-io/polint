mod input_snapshot;
mod keys;
mod stats;

pub use input_snapshot::{
    FileSnapshot, GoLifecycleSnapshot, INPUT_SNAPSHOT_SCHEMA_VERSION, InputComponent,
    InputComponentStatus, InputSnapshot, ProviderSchemaSnapshot, TsJsLifecycleSnapshot,
};
pub use keys::{LayerKind, PrecisionTier, QueryKey};
pub use stats::CacheStats;

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Digest {
    pub kind: DigestKind,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestKind {
    SourceText,
    Config,
    Workspace,
    RunManifest,
    GoLifecycle,
    TsJsLifecycle,
    RuleCode,
    RuleOptions,
    ModelFile,
    ExtensionCode,
    ToolInvocation,
    ProviderParameters,
    ProviderOutput,
    LayerOutput,
    DependencyLayer,
    QueryParameters,
    Budget,
    Evidence,
    SummaryBody,
    SummaryDependency,
}

impl Digest {
    pub fn from_parts(kind: DigestKind, label: &str, parts: &[&str]) -> Self {
        let kind_label = kind.as_str();
        let mut hash = FNV_OFFSET_BASIS;
        fingerprint_length_prefixed_part(&mut hash, "kind", kind_label);
        for part in parts {
            fingerprint_length_prefixed_part(&mut hash, label, part);
        }

        Self {
            kind,
            value: format!("{hash:016x}"),
        }
    }

    pub fn builder(kind: DigestKind, label: &'static str) -> DigestBuilder {
        DigestBuilder::new(kind, label)
    }

    /// Digest of `rows` in their given order, hashed in parallel.
    ///
    /// The rows are cut into chunks of a fixed size, each chunk is hashed on
    /// its own, and the chunk digests are folded in chunk order behind the row
    /// count. The value is therefore a function of the rows, their order and
    /// what `write` feeds for each, and not of how many threads computed it.
    ///
    /// `task` is called once per chunk, on the thread hashing it, for state
    /// that chunk's rows are written with — such as a stable-key read view,
    /// which must not be held across the parallel section by the caller.
    pub fn of_rows<T: Sync, S>(
        kind: DigestKind,
        label: &'static str,
        rows: &[T],
        task: impl Fn() -> S + Sync,
        write: impl Fn(&mut DigestBuilder, &S, &T) + Sync,
    ) -> Self {
        use rayon::prelude::*;
        const ROWS_PER_CHUNK: usize = 4096;
        let chunks = rows
            .par_chunks(ROWS_PER_CHUNK)
            .map(|chunk| {
                let state = task();
                let mut digest = DigestBuilder::new(kind, label);
                for row in chunk {
                    write(&mut digest, &state, row);
                }
                digest.hash
            })
            .collect::<Vec<_>>();
        let mut folded = DigestBuilder::new(kind, label);
        folded.u64_field("rows", rows.len() as u64);
        for chunk in chunks {
            folded.bytes_field("chunk", &chunk.to_le_bytes());
        }
        folded.finish()
    }

    pub fn from_unordered(kind: DigestKind, label: &str, mut digests: Vec<Digest>) -> Self {
        digests.sort();
        let digest_parts = digests.iter().map(ToString::to_string).collect::<Vec<_>>();
        let hash_parts = digest_parts.iter().map(String::as_str).collect::<Vec<_>>();

        Self::from_parts(kind, label, &hash_parts)
    }

    pub fn absent(kind: DigestKind, label: &str) -> Self {
        Self::from_parts(kind, "absent", &[label])
    }

    pub fn unsupported(kind: DigestKind, label: &str, reason: &str) -> Self {
        Self::from_parts(kind, "unsupported", &[label, reason])
    }
}

#[derive(Debug)]
pub struct DigestBuilder {
    kind: DigestKind,
    label: &'static str,
    hash: u64,
}

impl DigestBuilder {
    fn new(kind: DigestKind, label: &'static str) -> Self {
        let mut hash = FNV_OFFSET_BASIS;
        fingerprint_length_prefixed_part(&mut hash, "kind", kind.as_str());
        Self { kind, label, hash }
    }

    pub fn part(&mut self, value: &str) {
        fingerprint_length_prefixed_part(&mut self.hash, self.label, value);
    }

    pub fn field(&mut self, label: &str, value: &str) {
        fingerprint_length_prefixed_part(&mut self.hash, label, value);
    }

    pub fn bytes_field(&mut self, label: &str, value: &[u8]) {
        fingerprint_length_prefixed_bytes(&mut self.hash, label, value);
    }

    pub fn u64_field(&mut self, label: &str, value: u64) {
        let value = value.to_string();
        self.field(label, &value);
    }

    pub fn debug_part(&mut self, value: impl fmt::Debug) {
        self.part(&format!("{value:?}"));
    }

    pub fn bool_part(&mut self, value: bool) {
        self.part(if value { "true" } else { "false" });
    }

    pub fn finish(self) -> Digest {
        Digest {
            kind: self.kind,
            value: format!("{:016x}", self.hash),
        }
    }
}

impl DigestKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceText => "source_text",
            Self::Config => "config",
            Self::Workspace => "workspace",
            Self::RunManifest => "run_manifest",
            Self::GoLifecycle => "go_lifecycle",
            Self::TsJsLifecycle => "ts_js_lifecycle",
            Self::RuleCode => "rule_code",
            Self::RuleOptions => "rule_options",
            Self::ModelFile => "model_file",
            Self::ExtensionCode => "extension_code",
            Self::ToolInvocation => "tool_invocation",
            Self::ProviderParameters => "provider_parameters",
            Self::ProviderOutput => "provider_output",
            Self::LayerOutput => "layer_output",
            Self::DependencyLayer => "dependency_layer",
            Self::QueryParameters => "query_parameters",
            Self::Budget => "budget",
            Self::Evidence => "evidence",
            Self::SummaryBody => "summary_body",
            Self::SummaryDependency => "summary_dependency",
        }
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.kind.as_str(), self.value)
    }
}

const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
const PART_SEPARATOR: u8 = 0xfe;

fn fingerprint_length_prefixed_part(hash: &mut u64, label: &str, value: &str) {
    fingerprint_length_prefixed_bytes(hash, label, value.as_bytes());
}

fn fingerprint_length_prefixed_bytes(hash: &mut u64, label: &str, value: &[u8]) {
    fingerprint_usize_decimal(hash, label.len());
    fingerprint_byte(hash, b':');
    fingerprint_bytes(hash, label.as_bytes());
    fingerprint_byte(hash, b'=');
    fingerprint_usize_decimal(hash, value.len());
    fingerprint_byte(hash, b':');
    fingerprint_bytes(hash, value);
    fingerprint_byte(hash, PART_SEPARATOR);
}

fn fingerprint_usize_decimal(hash: &mut u64, mut value: usize) {
    if value == 0 {
        fingerprint_byte(hash, b'0');
        return;
    }

    let mut buffer = [0u8; 20];
    let mut index = buffer.len();
    while value > 0 {
        index -= 1;
        buffer[index] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    fingerprint_bytes(hash, &buffer[index..]);
}

fn fingerprint_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        fingerprint_byte(hash, *byte);
    }
}

fn fingerprint_byte(hash: &mut u64, byte: u8) {
    *hash ^= u64::from(byte);
    *hash = hash.wrapping_mul(FNV_PRIME);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_parts_is_deterministic_and_kind_separated() {
        let first = Digest::from_parts(DigestKind::SourceText, "source_text", &["path", "hash"]);
        let second = Digest::from_parts(DigestKind::SourceText, "source_text", &["path", "hash"]);
        let config = Digest::from_parts(DigestKind::Config, "source_text", &["path", "hash"]);

        assert_eq!(first, second);
        assert_ne!(first, config);
    }

    #[test]
    fn from_unordered_sorts_input_digests_canonically() {
        let a = Digest::from_parts(DigestKind::SourceText, "file", &["a"]);
        let b = Digest::from_parts(DigestKind::SourceText, "file", &["b"]);

        assert_eq!(
            Digest::from_unordered(DigestKind::LayerOutput, "layer", vec![b.clone(), a.clone()]),
            Digest::from_unordered(DigestKind::LayerOutput, "layer", vec![a, b])
        );
    }

    #[test]
    fn serde_and_display_include_kind_and_value() {
        let digest = Digest::from_parts(DigestKind::SourceText, "source_text", &["path"]);
        let json = serde_json::to_string(&digest).expect("digest should serialize");

        assert!(json.contains("\"kind\""));
        assert!(json.contains("\"value\""));
        assert_eq!(digest.to_string(), format!("source_text:{}", digest.value));
    }

    #[test]
    fn absent_and_unsupported_helpers_are_explicit() {
        assert_ne!(
            Digest::absent(DigestKind::ToolInvocation, "go"),
            Digest::unsupported(DigestKind::ToolInvocation, "go", "not on path")
        );
    }
}
