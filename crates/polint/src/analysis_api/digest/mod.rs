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
        Self::from_part_iter(kind, label, parts.iter().copied())
    }

    fn from_part_iter<'a>(
        kind: DigestKind,
        label: &str,
        parts: impl IntoIterator<Item = &'a str>,
    ) -> Self {
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

/// Digest parts packed end to end in one buffer, hashed in sorted order.
///
/// Large outputs have millions of parts. Owning each as its own string costs an
/// allocation per part and leaves the freed parts scattered over the heap; one
/// buffer is a few large allocations that go back to the operating system when
/// it is dropped. [`DigestParts::digest`] sorts the parts as strings and hashes
/// them in fixed-size chunks of that order, in parallel.
#[derive(Debug, Default)]
pub(crate) struct DigestParts {
    text: String,
    spans: Vec<(usize, usize)>,
}

impl DigestParts {
    pub(crate) fn push(&mut self, part: &str) {
        let start = self.text.len();
        self.text.push_str(part);
        self.spans.push((start, self.text.len()));
    }

    pub(crate) fn extend<S: AsRef<str>>(&mut self, parts: impl IntoIterator<Item = S>) {
        for part in parts {
            self.push(part.as_ref());
        }
    }

    /// Appends one part per row, written by `write` in parallel, in row order.
    ///
    /// Rows are written a bounded batch at a time, each worker into a buffer
    /// of its own that is copied into this one before the next batch starts,
    /// so workers only ever hold one batch. A failure reports the first
    /// failing row, as writing them one by one would.
    pub(crate) fn try_extend_in_order<T: Sync, E: Send>(
        &mut self,
        rows: &[T],
        write: impl Fn(&mut String, &T) -> Result<(), E> + Sync,
    ) -> Result<(), E> {
        use rayon::prelude::*;
        const ROWS_PER_CHUNK: usize = 64;
        const ROWS_PER_BATCH: usize = 64 * ROWS_PER_CHUNK;
        self.spans.reserve(rows.len());
        for batch in rows.chunks(ROWS_PER_BATCH) {
            let chunks = batch
                .par_chunks(ROWS_PER_CHUNK)
                .map(|chunk| {
                    let mut text = String::new();
                    let mut ends = Vec::with_capacity(chunk.len());
                    for row in chunk {
                        write(&mut text, row)?;
                        ends.push(text.len());
                    }
                    Ok((text, ends))
                })
                .collect::<Vec<Result<_, E>>>();
            for chunk in chunks {
                let (text, ends) = chunk?;
                let base = self.text.len();
                self.text.push_str(&text);
                let mut start = base;
                for end in ends {
                    self.spans.push((start, base + end));
                    start = base + end;
                }
            }
        }
        Ok(())
    }

    /// [`DigestParts::try_extend_in_order`] for a writer that cannot fail.
    pub(crate) fn extend_in_order<T: Sync>(
        &mut self,
        rows: &[T],
        write: impl Fn(&mut String, &T) + Sync,
    ) {
        let Ok(()) = self.try_extend_in_order(rows, |text, row| {
            write(text, row);
            Ok::<_, std::convert::Infallible>(())
        });
    }

    /// Sorts the parts as strings.
    pub(crate) fn sort(&mut self) {
        use rayon::slice::ParallelSliceMut as _;
        let text = self.text.as_str();
        // Parts that compare equal are identical bytes, so an unstable sort
        // orders them exactly as a stable one would.
        self.spans
            .par_sort_unstable_by(|left, right| text[left.0..left.1].cmp(&text[right.0..right.1]));
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &str> {
        self.spans
            .iter()
            .map(|&(start, end)| &self.text[start..end])
    }

    /// Digest of the parts, sorted.
    ///
    /// The sorted parts are cut into chunks of a fixed size, each chunk is
    /// hashed on its own and the chunk hashes are folded in order behind the
    /// part count, so the value depends on the parts and not on how many
    /// threads hashed them.
    pub(crate) fn digest(mut self, kind: DigestKind, label: &str) -> Digest {
        use rayon::prelude::*;
        const PARTS_PER_CHUNK: usize = 4096;
        self.sort();
        let text = self.text.as_str();
        let chunks = self
            .spans
            .par_chunks(PARTS_PER_CHUNK)
            .map(|chunk| {
                Digest::from_part_iter(
                    kind,
                    label,
                    chunk.iter().map(|&(start, end)| &text[start..end]),
                )
                .value
            })
            .collect::<Vec<_>>();
        let count = self.spans.len().to_string();
        Digest::from_part_iter(
            kind,
            label,
            std::iter::once(count.as_str()).chain(chunks.iter().map(String::as_str)),
        )
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
    fn packed_parts_digest_depends_on_the_parts_not_their_order_or_threads() {
        // Enough rows to span several parallel batches, with duplicates and
        // parts that are prefixes of each other.
        let rows = (0..20_000_u32).rev().collect::<Vec<_>>();
        let part = |row: &u32| format!("row={}{}", row % 997, "x".repeat((row % 3) as usize));

        let digest = |rows: &[u32], threads: usize| {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .expect("thread pool");
            pool.install(|| {
                let mut packed = DigestParts::default();
                packed.push("header");
                packed.extend_in_order(rows, |text, row| text.push_str(&part(row)));
                packed.digest(DigestKind::ProviderOutput, "rows")
            })
        };
        let mut sorted = rows.clone();
        sorted.sort();
        let expected = digest(&rows, 1);
        assert_eq!(digest(&rows, 4), expected);
        assert_eq!(digest(&sorted, 3), expected);

        let mut changed = rows.clone();
        changed[17] = 20_001;
        assert_ne!(digest(&changed, 4), expected);
        let mut fewer = rows;
        fewer.pop();
        assert_ne!(digest(&fewer, 4), expected);
    }

    #[test]
    fn packed_parts_report_the_first_failing_row() {
        let rows = (0..10_000_u32).collect::<Vec<_>>();
        // The first two failures are in different 64-row chunks of the first
        // 4096-row batch; later batches must not change error precedence either.
        for threads in [1, 2, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .expect("thread pool");
            let failed = pool.install(|| {
                let mut packed = DigestParts::default();
                packed.try_extend_in_order(&rows, |text, row| {
                    if [127, 191, 2_999, 5_999, 8_999].contains(row) {
                        return Err(*row);
                    }
                    text.push_str(&row.to_string());
                    Ok(())
                })
            });
            assert_eq!(
                failed,
                Err(127),
                "lowest row must win with {threads} workers"
            );
        }
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
