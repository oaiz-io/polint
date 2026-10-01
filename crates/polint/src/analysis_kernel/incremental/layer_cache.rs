#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Layer-cache persistence has a few test-only helpers and reserved validation paths."
    )
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::change_set::{ChangeKind, ChangeSet, ChangeSetRow};
use super::dependency_index::{
    CacheNode, DEPENDENCY_INDEX_SCHEMA, DependencyEdge, DependencyIndex, DependencyKind, ShapeKind,
};
use super::digest::{Digest, DigestKind};
use super::invalidation::{InvalidationAction, InvalidationPlan};
use super::keys::{LayerKey, LayerKind, PrecisionTier};
use crate::cache::stable_hash;

pub(crate) const LAYER_CACHE_MANIFEST_SCHEMA: &str = "polint-layer-cache-manifest-2";
// A manifest lists one dependency edge per tracked input, so on a large repo it
// is naturally large. The ceiling only bounds how much is read from disk; a
// manifest above it is treated as unreadable and evicted, which silently makes
// its layer miss on every subsequent run. It therefore tracks the payload
// ceiling rather than sitting below it.
const LAYER_CACHE_MANIFEST_MAX_BYTES: u64 = LAYER_CACHE_PAYLOAD_MAX_BYTES;
// The writer has no ceiling, so a payload above this one is written, rejected on
// the next read, evicted and rebuilt from its per-file entries on every warm run.
// A Go syntax layer for a repository of about five thousand Go files is close to
// 100 MB of JSON, so the ceiling sits well above that.
const LAYER_CACHE_PAYLOAD_MAX_BYTES: u64 = 256 * 1_048_576;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);
const MANIFEST_LAYER_DEPENDENCY_SOURCE: &str = "__manifest_layer__";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LayerCacheManifest {
    pub(crate) manifest_schema: String,
    pub(crate) dependency_index_schema: String,
    pub(crate) key: LayerKey,
    pub(crate) output_digest: Digest,
    pub(crate) payload_digest: Digest,
    pub(crate) created_by_polint: String,
    #[serde(with = "manifest_dependencies")]
    pub(crate) dependencies: Vec<DependencyEdge>,
    #[serde(skip)]
    pub(crate) dependency_index: DependencyIndex,
    pub(crate) precision: PrecisionTier,
    pub(crate) validation: String,
    pub(crate) warnings: Vec<String>,
}

impl LayerCacheManifest {
    pub(crate) fn new(
        key: LayerKey,
        output_digest: Digest,
        payload_digest: Digest,
        mut dependencies: Vec<DependencyEdge>,
        precision: PrecisionTier,
        validation: impl Into<String>,
        mut warnings: Vec<String>,
    ) -> Self {
        sort_and_dedup_manifest_dependencies(&mut dependencies);
        warnings.sort();
        warnings.dedup();

        let mut manifest = Self {
            manifest_schema: LAYER_CACHE_MANIFEST_SCHEMA.to_string(),
            dependency_index_schema: DEPENDENCY_INDEX_SCHEMA.to_string(),
            key,
            output_digest,
            payload_digest,
            created_by_polint: env!("CARGO_PKG_VERSION").to_string(),
            dependency_index: DependencyIndex::default(),
            dependencies,
            precision,
            validation: validation.into(),
            warnings,
        };
        manifest.canonicalize_dependency_sources();
        manifest
    }

    fn canonicalize_dependency_sources(&mut self) {
        let from = relative_manifest_dependency_source();
        for edge in &mut self.dependencies {
            edge.from = from.clone();
        }
        sort_and_dedup_manifest_dependencies(&mut self.dependencies);
    }
}

pub(crate) fn relative_manifest_dependency_source() -> CacheNode {
    CacheNode::Input(MANIFEST_LAYER_DEPENDENCY_SOURCE.to_string())
}

fn sort_and_dedup_manifest_dependencies(dependencies: &mut Vec<DependencyEdge>) {
    dependencies.sort_by(manifest_dependency_order);
    dependencies.dedup_by(manifest_dependencies_match);
}

fn manifest_dependency_order(left: &DependencyEdge, right: &DependencyEdge) -> std::cmp::Ordering {
    (&left.to, left.kind, left.required_shape, &left.from).cmp(&(
        &right.to,
        right.kind,
        right.required_shape,
        &right.from,
    ))
}

fn manifest_dependencies_match(left: &mut DependencyEdge, right: &mut DependencyEdge) -> bool {
    left.to == right.to
        && left.kind == right.kind
        && left.required_shape == right.required_shape
        && left.from == right.from
}

#[derive(Deserialize, Serialize)]
struct ManifestDependencyRow {
    to: CacheNode,
    kind: DependencyKind,
    required_shape: ShapeKind,
}

mod manifest_dependencies {
    use super::*;

    pub(super) fn serialize<S>(
        dependencies: &[DependencyEdge],
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let rows = dependencies
            .iter()
            .map(|edge| ManifestDependencyRow {
                to: edge.to.clone(),
                kind: edge.kind,
                required_shape: edge.required_shape,
            })
            .collect::<Vec<_>>();
        rows.serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Vec<DependencyEdge>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let rows = Vec::<ManifestDependencyRow>::deserialize(deserializer)?;
        Ok(rows
            .into_iter()
            .map(|row| DependencyEdge {
                from: relative_manifest_dependency_source(),
                to: row.to,
                kind: row.kind,
                required_shape: row.required_shape,
            })
            .collect())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LayerCacheStore {
    root: PathBuf,
    repo_root: Option<PathBuf>,
    enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayerCacheReadStatus {
    Hit,
    Miss,
    InvalidEvicted,
    BypassedDisabled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LayerCacheReadOutcome<T> {
    pub(crate) status: LayerCacheReadStatus,
    pub(crate) manifest: Option<LayerCacheManifest>,
    pub(crate) output_digest: Option<Digest>,
    pub(crate) payload_digest: Option<Digest>,
    pub(crate) value: Option<T>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayerCacheWriteStatus {
    Written,
    BypassedDisabled,
}

impl LayerCacheStore {
    pub(crate) fn new(root: impl AsRef<Path>, enabled: bool) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            repo_root: None,
            enabled,
        }
    }

    pub(crate) fn new_with_repo_root(
        root: impl AsRef<Path>,
        enabled: bool,
        repo_root: Option<PathBuf>,
    ) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            repo_root,
            enabled,
        }
    }

    pub(crate) fn payload_digest_for_json<T>(value: &T) -> Result<Digest>
    where
        T: Serialize,
    {
        let payload = serde_json::to_vec(value)?;
        payload_digest_for_bytes(&payload)
    }

    pub(crate) fn payload_digest_for_json_bytes(payload: &[u8]) -> Result<Digest> {
        payload_digest_for_bytes(payload)
    }

    pub(crate) fn read_json<T>(&self, key: &LayerKey) -> LayerCacheReadOutcome<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.read_json_validated(key, |_, _| true)
    }

    pub(crate) fn read_json_bytes_validated<F>(
        &self,
        key: &LayerKey,
        validator: F,
    ) -> LayerCacheReadOutcome<Vec<u8>>
    where
        F: FnOnce(&[u8], &LayerCacheManifest) -> bool,
    {
        self.read_payload_bytes_validated(key, validator)
    }

    pub(crate) fn read_json_validated<T, F>(
        &self,
        key: &LayerKey,
        validator: F,
    ) -> LayerCacheReadOutcome<T>
    where
        T: for<'de> Deserialize<'de>,
        F: FnOnce(&T, &LayerCacheManifest) -> bool,
    {
        // Validating requires deserializing the payload; keep that value so a hit
        // does not deserialize the same bytes a second time.
        let mut validated = None;
        let read = self.read_payload_bytes_validated(key, |payload_bytes, manifest| {
            let Ok(value) = serde_json::from_slice::<T>(payload_bytes) else {
                return false;
            };
            if !validator(&value, manifest) {
                return false;
            }
            validated = Some(value);
            true
        });
        match read.status {
            LayerCacheReadStatus::Hit => {
                let value = validated
                    .expect("layer cache hit ran the validator, which keeps the parsed payload");
                LayerCacheReadOutcome {
                    status: LayerCacheReadStatus::Hit,
                    output_digest: read.output_digest,
                    payload_digest: read.payload_digest,
                    manifest: read.manifest,
                    value: Some(value),
                }
            }
            status => LayerCacheReadOutcome {
                status,
                output_digest: read.output_digest,
                payload_digest: read.payload_digest,
                manifest: read.manifest,
                value: None,
            },
        }
    }

    fn read_payload_bytes_validated<F>(
        &self,
        key: &LayerKey,
        validator: F,
    ) -> LayerCacheReadOutcome<Vec<u8>>
    where
        F: FnOnce(&[u8], &LayerCacheManifest) -> bool,
    {
        if !self.enabled {
            return LayerCacheReadOutcome::without_value(LayerCacheReadStatus::BypassedDisabled);
        }

        let Ok(manifest_path) = self.manifest_path(key) else {
            return LayerCacheReadOutcome::without_value(LayerCacheReadStatus::Miss);
        };
        let manifests_dir = self.manifests_dir();
        if self.path_is_missing(&manifest_path) {
            self.evict_stale_manifests_for_key(key);
            return LayerCacheReadOutcome::without_value(LayerCacheReadStatus::Miss);
        }

        let Some(manifest_path) = self.managed_existing_file(&manifest_path, &manifests_dir) else {
            return LayerCacheReadOutcome::without_value(LayerCacheReadStatus::InvalidEvicted);
        };
        let Ok(raw_manifest) = crate::repo_fs::read_file_to_string_with_limit(
            &manifest_path,
            LAYER_CACHE_MANIFEST_MAX_BYTES,
        ) else {
            evict_file(&manifest_path);
            return LayerCacheReadOutcome::without_value(LayerCacheReadStatus::Miss);
        };
        let Ok(mut manifest) = serde_json::from_str::<LayerCacheManifest>(&raw_manifest) else {
            evict_file(&manifest_path);
            return LayerCacheReadOutcome::without_value(LayerCacheReadStatus::InvalidEvicted);
        };
        let mut canonical_dependencies = manifest.dependencies.clone();
        sort_and_dedup_manifest_dependencies(&mut canonical_dependencies);
        if canonical_dependencies != manifest.dependencies {
            evict_file(&manifest_path);
            return LayerCacheReadOutcome::without_value(LayerCacheReadStatus::InvalidEvicted);
        }
        manifest.canonicalize_dependency_sources();
        if !self.manifest_metadata_is_supported(&manifest, key) {
            evict_file(&manifest_path);
            return LayerCacheReadOutcome::without_value(LayerCacheReadStatus::InvalidEvicted);
        }

        let Some(blob_path) = self.blob_path(&manifest.payload_digest) else {
            evict_file(&manifest_path);
            return LayerCacheReadOutcome::without_value(LayerCacheReadStatus::InvalidEvicted);
        };
        let Some(blob_path) = self.managed_existing_file(&blob_path, &self.blobs_dir()) else {
            evict_file(&manifest_path);
            evict_file(&blob_path);
            return LayerCacheReadOutcome::without_value(LayerCacheReadStatus::InvalidEvicted);
        };
        let Ok(payload_bytes) =
            crate::repo_fs::read_file_with_limit(&blob_path, LAYER_CACHE_PAYLOAD_MAX_BYTES)
        else {
            evict_file(&manifest_path);
            return LayerCacheReadOutcome::without_value(LayerCacheReadStatus::InvalidEvicted);
        };
        let Ok(payload_digest) = payload_digest_for_bytes(&payload_bytes) else {
            evict_file(&manifest_path);
            evict_file(&blob_path);
            return LayerCacheReadOutcome::without_value(LayerCacheReadStatus::InvalidEvicted);
        };
        if payload_digest != manifest.payload_digest {
            evict_file(&manifest_path);
            evict_file(&blob_path);
            return LayerCacheReadOutcome::without_value(LayerCacheReadStatus::InvalidEvicted);
        }
        if !validator(&payload_bytes, &manifest) {
            evict_file(&manifest_path);
            return LayerCacheReadOutcome::without_value(LayerCacheReadStatus::InvalidEvicted);
        }

        LayerCacheReadOutcome {
            status: LayerCacheReadStatus::Hit,
            output_digest: Some(manifest.output_digest.clone()),
            payload_digest: Some(payload_digest),
            manifest: Some(manifest),
            value: Some(payload_bytes),
        }
    }

    pub(crate) fn write_json<T>(
        &self,
        manifest: &LayerCacheManifest,
        value: &T,
    ) -> Result<LayerCacheWriteStatus>
    where
        T: Serialize,
    {
        if manifest.manifest_schema != LAYER_CACHE_MANIFEST_SCHEMA {
            return Err(anyhow!("unsupported layer cache manifest schema"));
        }
        let payload = serde_json::to_vec(value)?;
        let payload_digest = payload_digest_for_bytes(&payload)?;
        if payload_digest != manifest.payload_digest {
            return Err(anyhow!("layer cache payload digest mismatch"));
        }
        if !self.manifest_metadata_is_supported(manifest, &manifest.key) {
            return Err(anyhow!("unsupported layer cache manifest metadata"));
        }
        self.write_json_bytes_inner(&manifest.key, manifest, payload)
    }

    pub(crate) fn write_json_bytes(
        &self,
        manifest: &LayerCacheManifest,
        payload: Vec<u8>,
    ) -> Result<LayerCacheWriteStatus> {
        if manifest.manifest_schema != LAYER_CACHE_MANIFEST_SCHEMA {
            return Err(anyhow!("unsupported layer cache manifest schema"));
        }
        let payload_digest = payload_digest_for_bytes(&payload)?;
        if payload_digest != manifest.payload_digest {
            return Err(anyhow!("layer cache payload digest mismatch"));
        }
        if !self.manifest_metadata_is_supported(manifest, &manifest.key) {
            return Err(anyhow!("unsupported layer cache manifest metadata"));
        }
        self.write_json_bytes_inner(&manifest.key, manifest, payload)
    }

    fn write_json_bytes_inner(
        &self,
        key: &LayerKey,
        manifest: &LayerCacheManifest,
        payload: Vec<u8>,
    ) -> Result<LayerCacheWriteStatus> {
        if !self.enabled {
            return Ok(LayerCacheWriteStatus::BypassedDisabled);
        }

        let payload_path = self
            .blob_path(&manifest.payload_digest)
            .ok_or_else(|| anyhow!("invalid layer cache payload digest path"))?;
        let manifest_path = self.manifest_path(key)?;
        self.create_managed_dir(&self.blobs_dir())
            .with_context(|| {
                format!(
                    "failed to create layer cache dir {}",
                    self.blobs_dir().display()
                )
            })?;
        self.create_managed_dir(&self.manifests_dir())
            .with_context(|| {
                format!(
                    "failed to create layer cache dir {}",
                    self.manifests_dir().display()
                )
            })?;
        self.ensure_managed_dir(&self.blobs_dir())?;
        self.ensure_managed_dir(&self.manifests_dir())?;

        self.write_layer_file_atomic(&payload_path, payload)
            .with_context(|| format!("failed to write layer payload {}", payload_path.display()))?;

        self.write_manifest_last(&manifest_path, manifest)?;
        Ok(LayerCacheWriteStatus::Written)
    }

    fn write_manifest_last(
        &self,
        manifest_path: &Path,
        manifest: &LayerCacheManifest,
    ) -> Result<()> {
        self.write_layer_file_atomic(manifest_path, serde_json::to_vec(manifest)?)
            .with_context(|| {
                format!("failed to write layer manifest {}", manifest_path.display())
            })?;
        Ok(())
    }

    fn blobs_dir(&self) -> PathBuf {
        self.root.join("blobs")
    }

    fn manifests_dir(&self) -> PathBuf {
        self.root.join("manifests")
    }

    fn blob_path(&self, digest: &Digest) -> Option<PathBuf> {
        if is_safe_digest_file_component(&digest.value) {
            Some(self.blobs_dir().join(format!("{}.json", digest.value)))
        } else {
            None
        }
    }

    fn manifest_path(&self, key: &LayerKey) -> Result<PathBuf> {
        let key_json = serde_json::to_string(key)?;
        Ok(self
            .manifests_dir()
            .join(format!("{}.json", stable_hash(&[&key_json]))))
    }

    fn manifest_metadata_is_supported(
        &self,
        manifest: &LayerCacheManifest,
        key: &LayerKey,
    ) -> bool {
        manifest.manifest_schema == LAYER_CACHE_MANIFEST_SCHEMA
            && manifest.dependency_index_schema == DEPENDENCY_INDEX_SCHEMA
            && manifest.output_digest.kind == DigestKind::ProviderOutput
            && is_safe_digest_file_component(&manifest.output_digest.value)
            && manifest.payload_digest.kind == DigestKind::LayerOutput
            && is_safe_digest_file_component(&manifest.payload_digest.value)
            && is_supported_validation_label(&manifest.validation)
            && dependencies_match_layer_key(manifest)
            && invalidation_allows_manifest_reuse(manifest, key)
    }

    fn managed_existing_file(&self, path: &Path, managed_dir: &Path) -> Option<PathBuf> {
        if let Some(repo_root) = &self.repo_root {
            let path_relative = self.repo_relative_path(path)?;
            let managed_relative = self.repo_relative_path(managed_dir)?;
            let path = crate::repo_fs::repo_file_path(repo_root, path_relative).ok()?;
            let managed_dir = crate::repo_fs::repo_dir_path(repo_root, managed_relative).ok()?;
            return path.starts_with(&managed_dir).then_some(path);
        }
        crate::repo_fs::managed_existing_file(&self.root, managed_dir, path)
    }

    fn ensure_managed_dir(&self, dir: &Path) -> Result<()> {
        if let Some(repo_root) = &self.repo_root {
            let relative_path = self
                .repo_relative_path(dir)
                .ok_or_else(|| anyhow!("layer cache dir escapes managed cache root"))?;
            crate::repo_fs::repo_dir_path(repo_root, relative_path)
                .map_err(|_| anyhow!("layer cache dir escapes managed cache root"))?;
            return Ok(());
        }
        crate::repo_fs::ensure_no_symlink_ancestors(dir)
            .map_err(|_| anyhow!("layer cache dir escapes managed cache root"))?;
        let metadata = fs::symlink_metadata(dir)
            .with_context(|| format!("failed to inspect layer cache dir {}", dir.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(anyhow!("layer cache dir escapes managed cache root"));
        }
        let canonical_root = self.root.canonicalize().with_context(|| {
            format!(
                "failed to canonicalize layer cache root {}",
                self.root.display()
            )
        })?;
        let canonical_dir = dir
            .canonicalize()
            .with_context(|| format!("failed to canonicalize layer cache dir {}", dir.display()))?;
        if !canonical_dir.starts_with(canonical_root) {
            return Err(anyhow!("layer cache dir escapes managed cache root"));
        }
        Ok(())
    }

    fn create_managed_dir(&self, dir: &Path) -> Result<(), crate::repo_fs::RepoFileReadError> {
        if let Some(repo_root) = &self.repo_root {
            let relative_path = self
                .repo_relative_path(dir)
                .ok_or(crate::repo_fs::RepoFileReadError::EscapesRepo)?;
            crate::repo_fs::ensure_repo_dir(repo_root, relative_path).map(|_| ())
        } else {
            crate::repo_fs::create_dir_all_no_symlink(dir)
        }
    }

    fn write_layer_file_atomic(
        &self,
        path: &Path,
        contents: impl AsRef<[u8]>,
    ) -> Result<(), crate::repo_fs::RepoFileReadError> {
        if let Some(repo_root) = &self.repo_root {
            let relative_path = self
                .repo_relative_path(path)
                .ok_or(crate::repo_fs::RepoFileReadError::EscapesRepo)?;
            crate::repo_fs::write_repo_file_atomic(repo_root, relative_path, contents)
        } else {
            crate::repo_fs::write_file_atomic_no_symlink(path, contents)
        }
    }

    fn path_is_missing(&self, path: &Path) -> bool {
        if let Some(repo_root) = &self.repo_root {
            let Some(relative_path) = self.repo_relative_path(path) else {
                return false;
            };
            return matches!(
                crate::repo_fs::repo_file_path(repo_root, relative_path),
                Err(error) if error.is_not_found()
            );
        }
        if crate::repo_fs::ensure_no_symlink_ancestors(path).is_err() {
            return false;
        }
        matches!(
            fs::symlink_metadata(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        )
    }

    fn repo_relative_path(&self, path: &Path) -> Option<PathBuf> {
        let repo_root = self.repo_root.as_ref()?;
        let relative_path = path.strip_prefix(repo_root).ok()?;
        crate::repo_fs::normalize_repo_relative_input(relative_path)
    }

    fn evict_stale_manifests_for_key(&self, key: &LayerKey) {
        let manifests_dir = self.manifests_dir();
        if self.ensure_managed_dir(&manifests_dir).is_err() {
            return;
        }
        let Ok(entries) = fs::read_dir(&manifests_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(managed_path) = self.managed_existing_file(&path, &manifests_dir) else {
                continue;
            };
            let Ok(raw_manifest) = crate::repo_fs::read_file_to_string_with_limit(
                &managed_path,
                LAYER_CACHE_MANIFEST_MAX_BYTES,
            ) else {
                continue;
            };
            let Ok(mut manifest) = serde_json::from_str::<LayerCacheManifest>(&raw_manifest) else {
                continue;
            };
            manifest.canonicalize_dependency_sources();
            if layer_keys_share_identity(&manifest.key, key)
                && !invalidation_allows_manifest_reuse(&manifest, key)
            {
                evict_file(&managed_path);
            }
        }
    }

    #[cfg(test)]
    fn blobs_dir_for_test(&self) -> PathBuf {
        self.blobs_dir()
    }

    #[cfg(test)]
    fn manifest_path_for_test(&self, key: &LayerKey) -> PathBuf {
        self.manifest_path(key).expect("manifest path")
    }

    #[cfg(test)]
    fn write_json_for_key_without_repair_for_test<T>(
        &self,
        key: &LayerKey,
        manifest: &LayerCacheManifest,
        value: &T,
    ) -> Result<LayerCacheWriteStatus>
    where
        T: Serialize,
    {
        self.write_json_bytes_inner(key, manifest, serde_json::to_vec(value)?)
    }

    #[cfg(test)]
    fn write_json_without_repair_for_test<T>(
        &self,
        manifest: &LayerCacheManifest,
        value: &T,
    ) -> Result<LayerCacheWriteStatus>
    where
        T: Serialize,
    {
        self.write_json_bytes_inner(&manifest.key, manifest, serde_json::to_vec(value)?)
    }
}

impl<T> LayerCacheReadOutcome<T> {
    fn without_value(status: LayerCacheReadStatus) -> Self {
        Self {
            status,
            manifest: None,
            output_digest: None,
            payload_digest: None,
            value: None,
        }
    }
}

fn payload_digest_for_bytes(payload: &[u8]) -> Result<Digest> {
    let payload = std::str::from_utf8(payload).context("layer cache payload was not UTF-8")?;
    Ok(Digest::from_parts(
        DigestKind::LayerOutput,
        "layer_cache_payload",
        &[payload],
    ))
}

fn is_safe_digest_file_component(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_supported_validation_label(value: &str) -> bool {
    value == "native_trusted"
}

fn dependencies_match_layer_key(manifest: &LayerCacheManifest) -> bool {
    let requires_dependencies = matches!(
        manifest.key.layer_kind,
        LayerKind::GoSyntax | LayerKind::ModuleGraph | LayerKind::SymbolGraph | LayerKind::Metrics
    );
    if requires_dependencies && manifest.dependencies.is_empty() {
        return false;
    }
    manifest
        .dependencies
        .iter()
        .all(|edge| dependency_source_matches_manifest(edge, manifest))
}

fn dependency_source_matches_manifest(
    edge: &DependencyEdge,
    manifest: &LayerCacheManifest,
) -> bool {
    match &edge.from {
        CacheNode::Input(value) => value == MANIFEST_LAYER_DEPENDENCY_SOURCE,
        CacheNode::Layer(layer_key) => layer_key == &manifest.key,
        _ => false,
    }
}

fn invalidation_allows_manifest_reuse(manifest: &LayerCacheManifest, key: &LayerKey) -> bool {
    if manifest.key == *key {
        return true;
    }

    let plan = invalidation_plan_for_manifest(manifest, key);
    plan.affected_nodes.is_empty()
        && plan
            .actions
            .iter()
            .all(|action| matches!(action, InvalidationAction::Reuse(_)))
}

fn invalidation_plan_for_manifest(
    manifest: &LayerCacheManifest,
    key: &LayerKey,
) -> InvalidationPlan {
    let change_set = manifest_change_set(manifest, key);
    let dependency_index = DependencyIndex::from_edges(expanded_manifest_dependencies(manifest));
    InvalidationPlan::from_change_set(&dependency_index, &change_set)
}

fn expanded_manifest_dependencies(manifest: &LayerCacheManifest) -> Vec<DependencyEdge> {
    let from = CacheNode::Layer(manifest.key.clone());
    manifest
        .dependencies
        .iter()
        .cloned()
        .map(|mut edge| {
            edge.from = from.clone();
            edge
        })
        .collect()
}

fn manifest_change_set(manifest: &LayerCacheManifest, key: &LayerKey) -> ChangeSet {
    if manifest.key == *key {
        return ChangeSet::from_rows(Vec::new());
    }

    let mut rows = Vec::new();
    if manifest.key.layer_kind != key.layer_kind
        || manifest.key.provider_id != key.provider_id
        || manifest.key.provider_version != key.provider_version
        || manifest.key.schema_version != key.schema_version
    {
        rows.push(layer_key_change_row(
            manifest,
            key,
            ChangeKind::ProviderVersion,
        ));
        return ChangeSet::from_rows(rows);
    }

    if manifest.key.parameter_digest != key.parameter_digest {
        rows.push(layer_key_change_row(manifest, key, ChangeKind::RuleOptions));
    }
    if manifest.key.config_digest != key.config_digest {
        push_dependency_change_rows(
            &mut rows,
            manifest,
            &[DependencyKind::Config],
            ChangeKind::Unknown,
            manifest.key.config_digest.clone(),
        );
    }
    if manifest.key.lifecycle_digest != key.lifecycle_digest {
        push_dependency_change_rows(
            &mut rows,
            manifest,
            &[DependencyKind::Lifecycle],
            ChangeKind::Lifecycle,
            manifest.key.lifecycle_digest.clone(),
        );
    }
    if manifest.key.toolchain_digest != key.toolchain_digest {
        push_dependency_change_rows(
            &mut rows,
            manifest,
            &[DependencyKind::Toolchain, DependencyKind::ToolInvocation],
            ChangeKind::Toolchain,
            manifest.key.toolchain_digest.clone(),
        );
    }
    if manifest.key.input_digests != key.input_digests {
        push_dependency_change_rows(
            &mut rows,
            manifest,
            &[
                DependencyKind::Input,
                DependencyKind::SourceText,
                DependencyKind::ImportShape,
            ],
            ChangeKind::Unknown,
            changed_digest_or_fallback(&manifest.key.input_digests, &key.input_digests, manifest),
        );
    }
    if manifest.key.dependency_layer_digests != key.dependency_layer_digests {
        push_dependency_change_rows(
            &mut rows,
            manifest,
            &[DependencyKind::Layer, DependencyKind::UpstreamLayer],
            ChangeKind::Unknown,
            changed_digest_or_fallback(
                &manifest.key.dependency_layer_digests,
                &key.dependency_layer_digests,
                manifest,
            ),
        );
    }
    if manifest.key.extension_digests != key.extension_digests {
        push_dependency_change_rows(
            &mut rows,
            manifest,
            &[DependencyKind::Extension],
            ChangeKind::ExtensionDeclaredInput,
            changed_digest_or_fallback(
                &manifest.key.extension_digests,
                &key.extension_digests,
                manifest,
            ),
        );
    }
    if rows.is_empty() {
        rows.push(layer_key_change_row(manifest, key, ChangeKind::Unknown));
    }

    ChangeSet::from_rows(rows)
}

fn push_dependency_change_rows(
    rows: &mut Vec<ChangeSetRow>,
    manifest: &LayerCacheManifest,
    kinds: &[DependencyKind],
    fallback_kind: ChangeKind,
    digest: Digest,
) {
    let before = rows.len();
    rows.extend(
        manifest
            .dependencies
            .iter()
            .filter(|edge| kinds.contains(&edge.kind))
            .map(|edge| ChangeSetRow {
                node: edge.to.clone(),
                kind: change_kind_for_edge(edge, fallback_kind),
                digest: digest.clone(),
            }),
    );
    if rows.len() == before {
        rows.push(ChangeSetRow {
            node: CacheNode::Layer(manifest.key.clone()),
            kind: fallback_kind,
            digest,
        });
    }
}

fn change_kind_for_edge(edge: &DependencyEdge, fallback_kind: ChangeKind) -> ChangeKind {
    match edge.required_shape {
        ShapeKind::Content => ChangeKind::ContentOnly,
        ShapeKind::Syntax => ChangeKind::SyntaxShape,
        ShapeKind::Import => ChangeKind::ImportShape,
        ShapeKind::PublicApi => ChangeKind::PublicApiShape,
        ShapeKind::ModuleTopology => ChangeKind::ModuleTopology,
        ShapeKind::Lifecycle => ChangeKind::Lifecycle,
        ShapeKind::Toolchain => ChangeKind::Toolchain,
        ShapeKind::RuleCode => ChangeKind::RuleCode,
        ShapeKind::RuleOptions => ChangeKind::RuleOptions,
        ShapeKind::ExtensionCode => ChangeKind::ExtensionCode,
        ShapeKind::ExtensionDeclaredInput => ChangeKind::ExtensionDeclaredInput,
        ShapeKind::Model => ChangeKind::ModelFile,
        ShapeKind::ProviderVersion => ChangeKind::ProviderVersion,
        ShapeKind::Output | ShapeKind::Unknown => fallback_kind,
    }
}

fn layer_key_change_row(
    manifest: &LayerCacheManifest,
    requested: &LayerKey,
    kind: ChangeKind,
) -> ChangeSetRow {
    ChangeSetRow {
        node: CacheNode::Layer(manifest.key.clone()),
        kind,
        digest: layer_key_change_digest(&manifest.key, requested),
    }
}

fn changed_digest_or_fallback(
    cached: &[Digest],
    requested: &[Digest],
    manifest: &LayerCacheManifest,
) -> Digest {
    if let Some(digest) = cached.iter().find(|digest| !requested.contains(digest)) {
        digest.clone()
    } else {
        layer_key_change_digest(&manifest.key, &manifest.key)
    }
}

fn layer_key_change_digest(cached: &LayerKey, requested: &LayerKey) -> Digest {
    if cached.parameter_digest != requested.parameter_digest {
        cached.parameter_digest.clone()
    } else if cached.config_digest != requested.config_digest {
        cached.config_digest.clone()
    } else if cached.lifecycle_digest != requested.lifecycle_digest {
        cached.lifecycle_digest.clone()
    } else if cached.toolchain_digest != requested.toolchain_digest {
        cached.toolchain_digest.clone()
    } else if let Some(digest) = cached
        .input_digests
        .iter()
        .find(|digest| !requested.input_digests.contains(digest))
    {
        digest.clone()
    } else if let Some(digest) = cached
        .dependency_layer_digests
        .iter()
        .find(|digest| !requested.dependency_layer_digests.contains(digest))
    {
        digest.clone()
    } else if let Some(digest) = cached
        .extension_digests
        .iter()
        .find(|digest| !requested.extension_digests.contains(digest))
    {
        digest.clone()
    } else {
        Digest::from_parts(
            DigestKind::ProviderParameters,
            "layer_key_identity",
            &[
                cached.provider_id.as_str(),
                cached.provider_version.as_str(),
                cached.schema_version.as_str(),
                requested.provider_id.as_str(),
                requested.provider_version.as_str(),
                requested.schema_version.as_str(),
            ],
        )
    }
}

fn layer_keys_share_identity(cached: &LayerKey, requested: &LayerKey) -> bool {
    cached.layer_kind == requested.layer_kind && cached.provider_id == requested.provider_id
}

fn temp_path_for(final_path: &Path) -> PathBuf {
    let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    final_path.with_extension(format!("json.tmp.{}.{}", std::process::id(), sequence))
}

fn evict_file(path: &Path) {
    let _ = fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_kernel::incremental::{
        CacheNode, DependencyEdge, DependencyKind, Digest, DigestKind, LayerKey, PrecisionTier,
        ShapeKind,
    };

    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct Payload {
        items: Vec<String>,
    }

    fn digest(kind: DigestKind, label: &str) -> Digest {
        Digest::from_parts(kind, label, &[label])
    }

    fn key() -> LayerKey {
        LayerKey::new(
            crate::analysis_kernel::incremental::keys::LayerKind::TsSyntax,
            "polint.ts.syntax",
            "1",
            "ts-facts-v1",
            Digest::absent(DigestKind::ProviderParameters, "none"),
            Digest::absent(DigestKind::TsJsLifecycle, "none"),
            Digest::absent(DigestKind::Config, "none"),
            Digest::absent(DigestKind::ToolInvocation, "none"),
            vec![digest(DigestKind::SourceText, "src/main.ts")],
            Vec::new(),
            Vec::new(),
        )
    }

    fn derived_key() -> LayerKey {
        LayerKey::new(
            crate::analysis_kernel::incremental::keys::LayerKind::ModuleGraph,
            "polint.module_graph",
            "1",
            "module-graph-v1",
            Digest::absent(DigestKind::ProviderParameters, "none"),
            Digest::absent(DigestKind::ProviderParameters, "none"),
            Digest::absent(DigestKind::Config, "none"),
            Digest::absent(DigestKind::ToolInvocation, "none"),
            vec![digest(DigestKind::SourceText, "src/main.ts")],
            vec![digest(DigestKind::DependencyLayer, "polint.ts.syntax")],
            Vec::new(),
        )
    }

    fn changed_derived_key() -> LayerKey {
        LayerKey::new(
            crate::analysis_kernel::incremental::keys::LayerKind::ModuleGraph,
            "polint.module_graph",
            "1",
            "module-graph-v1",
            Digest::absent(DigestKind::ProviderParameters, "none"),
            Digest::absent(DigestKind::ProviderParameters, "none"),
            Digest::absent(DigestKind::Config, "none"),
            Digest::absent(DigestKind::ToolInvocation, "none"),
            vec![digest(DigestKind::SourceText, "src/changed.ts")],
            vec![digest(DigestKind::DependencyLayer, "polint.ts.syntax")],
            Vec::new(),
        )
    }

    fn dependency(layer_key: &LayerKey) -> DependencyEdge {
        DependencyEdge {
            from: CacheNode::Layer(layer_key.clone()),
            to: CacheNode::Input("src/main.ts".to_string()),
            kind: DependencyKind::Input,
            required_shape: ShapeKind::Content,
        }
    }

    fn scratch_dir() -> tempfile::TempDir {
        tempfile::TempDir::new().expect("scratch directory")
    }

    fn manifest_for_payload(layer_key: LayerKey, payload: &Payload) -> LayerCacheManifest {
        LayerCacheManifest::new(
            layer_key.clone(),
            digest(DigestKind::ProviderOutput, "output"),
            LayerCacheStore::payload_digest_for_json(payload).expect("payload digest"),
            vec![dependency(&layer_key)],
            PrecisionTier::Syntax,
            "native_trusted",
            Vec::new(),
        )
    }

    fn write_manifest_only(
        store: &LayerCacheStore,
        layer_key: &LayerKey,
        manifest: &LayerCacheManifest,
    ) {
        std::fs::create_dir_all(store.manifests_dir()).unwrap();
        std::fs::write(
            store.manifest_path_for_test(layer_key),
            serde_json::to_vec(manifest).expect("manifest serializes"),
        )
        .unwrap();
    }

    #[test]
    fn write_json_publishes_payload_before_manifest_and_read_returns_output_digest() {
        let scratch = scratch_dir();
        let store = LayerCacheStore::new(scratch.path().join("layers"), true);
        let payload = Payload {
            items: vec!["a".to_string(), "b".to_string()],
        };
        let layer_key = key();
        let manifest = manifest_for_payload(layer_key.clone(), &payload);

        let status = store.write_json(&manifest, &payload).unwrap();
        let outcome: LayerCacheReadOutcome<Payload> = store.read_json(&layer_key);

        assert_eq!(status, LayerCacheWriteStatus::Written);
        assert_eq!(outcome.status, LayerCacheReadStatus::Hit);
        assert_eq!(outcome.value, Some(payload));
        assert_eq!(outcome.output_digest, Some(manifest.output_digest));
        assert_eq!(
            outcome.manifest.as_ref().and_then(|manifest| {
                manifest.dependencies.first().map(|edge| edge.from.clone())
            }),
            Some(relative_manifest_dependency_source())
        );
        assert!(
            store
                .blobs_dir_for_test()
                .join(format!("{}.json", manifest.payload_digest.value))
                .exists()
        );
        assert!(store.manifest_path_for_test(&layer_key).exists());
    }

    /// A manifest lists one edge per tracked input, so on a large repo it grows
    /// past a few megabytes. If the read ceiling rejects it, the manifest is
    /// evicted and its layer misses on every run from then on, forever.
    #[test]
    fn a_manifest_larger_than_four_mebibytes_still_round_trips_to_a_hit() {
        let scratch = scratch_dir();
        let store = LayerCacheStore::new(scratch.path().join("layers"), true);
        let payload = Payload {
            items: vec!["a".to_string()],
        };
        let layer_key = key();
        let dependencies = (0..8_192)
            .map(|ordinal| DependencyEdge {
                from: CacheNode::Layer(layer_key.clone()),
                to: CacheNode::Input(format!("src/{ordinal:0>1024}.ts")),
                kind: DependencyKind::Input,
                required_shape: ShapeKind::Content,
            })
            .collect::<Vec<_>>();
        let manifest = LayerCacheManifest::new(
            layer_key.clone(),
            digest(DigestKind::ProviderOutput, "output"),
            LayerCacheStore::payload_digest_for_json(&payload).expect("payload digest"),
            dependencies,
            PrecisionTier::Syntax,
            "native_trusted",
            Vec::new(),
        );

        store.write_json(&manifest, &payload).unwrap();
        let manifest_bytes = std::fs::metadata(store.manifest_path_for_test(&layer_key))
            .expect("manifest metadata")
            .len();
        let outcome: LayerCacheReadOutcome<Payload> = store.read_json(&layer_key);

        assert!(
            manifest_bytes > 4 * 1_048_576,
            "fixture manifest is {manifest_bytes} bytes, too small to exercise the ceiling"
        );
        assert_eq!(outcome.status, LayerCacheReadStatus::Hit);
        assert_eq!(outcome.value, Some(payload));
    }

    /// The writer accepts any payload, so a payload the reader refuses is
    /// evicted and rebuilt on every warm run. A large repository's Go syntax
    /// layer passes 64 MiB of JSON; it must still restore.
    #[test]
    fn a_payload_larger_than_sixty_four_mebibytes_still_round_trips_to_a_hit() {
        let scratch = scratch_dir();
        let store = LayerCacheStore::new(scratch.path().join("layers"), true);
        let payload = Payload {
            items: (0..66)
                .map(|ordinal| format!("{ordinal}{}", "x".repeat(1_048_576)))
                .collect(),
        };
        let layer_key = key();
        let manifest = manifest_for_payload(layer_key.clone(), &payload);

        store.write_json(&manifest, &payload).unwrap();
        let payload_bytes = std::fs::metadata(
            store
                .blobs_dir_for_test()
                .join(format!("{}.json", manifest.payload_digest.value)),
        )
        .expect("payload metadata")
        .len();
        let outcome: LayerCacheReadOutcome<Payload> = store.read_json(&layer_key);

        assert!(
            payload_bytes > 64 * 1_048_576,
            "fixture payload is {payload_bytes} bytes, too small to exercise the ceiling"
        );
        assert_eq!(outcome.status, LayerCacheReadStatus::Hit);
        assert_eq!(outcome.value, Some(payload));
    }

    #[test]
    fn go_syntax_raw_duplicate_dependencies_are_rejected_before_repair() {
        let scratch = scratch_dir();
        let store = LayerCacheStore::new(scratch.path().join("layers"), true);
        let payload = Payload {
            items: vec!["go".into()],
        };
        let key = derived_key();
        let manifest = manifest_for_payload(key.clone(), &payload);
        store.write_json(&manifest, &payload).unwrap();
        let path = store.manifest_path_for_test(&key);
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let rows = json["dependencies"].as_array_mut().unwrap();
        rows.push(rows[0].clone());
        std::fs::write(path, serde_json::to_vec(&json).unwrap()).unwrap();
        let read: LayerCacheReadOutcome<Payload> = store.read_json(&key);
        assert_eq!(read.status, LayerCacheReadStatus::InvalidEvicted);
    }

    #[test]
    fn corrupt_payload_manifest_and_mismatches_return_controlled_invalid_reads() {
        let scratch = scratch_dir();
        let store = LayerCacheStore::new(scratch.path().join("layers"), true);
        let payload = Payload {
            items: vec!["a".to_string()],
        };
        let layer_key = key();
        let manifest = manifest_for_payload(layer_key.clone(), &payload);
        store.write_json(&manifest, &payload).unwrap();
        std::fs::write(
            store
                .blobs_dir_for_test()
                .join(format!("{}.json", manifest.payload_digest.value)),
            "{broken",
        )
        .unwrap();

        let corrupt_payload: LayerCacheReadOutcome<Payload> = store.read_json(&layer_key);

        assert_eq!(corrupt_payload.status, LayerCacheReadStatus::InvalidEvicted);

        let manifest = manifest_for_payload(layer_key.clone(), &payload);
        store.write_json(&manifest, &payload).unwrap();
        std::fs::write(store.manifest_path_for_test(&layer_key), "{broken").unwrap();

        let corrupt_manifest: LayerCacheReadOutcome<Payload> = store.read_json(&layer_key);

        assert_eq!(
            corrupt_manifest.status,
            LayerCacheReadStatus::InvalidEvicted
        );
    }

    #[test]
    fn mismatched_manifest_key_schema_payload_digest_and_validator_do_not_hit() {
        let scratch = scratch_dir();
        let store = LayerCacheStore::new(scratch.path().join("layers"), true);
        let payload = Payload {
            items: vec!["a".to_string()],
        };
        let layer_key = key();
        let mut manifest = manifest_for_payload(layer_key.clone(), &payload);
        manifest.manifest_schema = "old-schema".to_string();
        store
            .write_json_without_repair_for_test(&manifest, &payload)
            .unwrap();

        let schema_mismatch: LayerCacheReadOutcome<Payload> = store.read_json(&layer_key);

        assert_eq!(schema_mismatch.status, LayerCacheReadStatus::InvalidEvicted);

        let mut manifest = manifest_for_payload(layer_key.clone(), &payload);
        manifest.key.provider_id = "other.provider".to_string();
        store
            .write_json_for_key_without_repair_for_test(&layer_key, &manifest, &payload)
            .unwrap();

        let key_mismatch: LayerCacheReadOutcome<Payload> = store.read_json(&layer_key);

        assert_eq!(key_mismatch.status, LayerCacheReadStatus::InvalidEvicted);

        let mut manifest = manifest_for_payload(layer_key.clone(), &payload);
        manifest.payload_digest = digest(DigestKind::LayerOutput, "wrong-payload");
        store
            .write_json_without_repair_for_test(&manifest, &payload)
            .unwrap();

        let payload_mismatch: LayerCacheReadOutcome<Payload> = store.read_json(&layer_key);

        assert_eq!(
            payload_mismatch.status,
            LayerCacheReadStatus::InvalidEvicted
        );

        let manifest = manifest_for_payload(layer_key.clone(), &payload);
        store.write_json(&manifest, &payload).unwrap();
        let validator_mismatch: LayerCacheReadOutcome<Payload> =
            store.read_json_validated(&layer_key, |_, _| false);

        assert_eq!(
            validator_mismatch.status,
            LayerCacheReadStatus::InvalidEvicted
        );
    }

    #[test]
    fn manifest_json_contains_schema_and_no_forbidden_identity_fields() {
        let payload = Payload {
            items: vec!["a".to_string()],
        };
        let manifest = manifest_for_payload(key(), &payload);
        let json = serde_json::to_value(manifest).expect("manifest should serialize");

        assert_eq!(json["manifest_schema"], "polint-layer-cache-manifest-2");
        assert_eq!(json["dependency_index_schema"], "polint-dependency-index-1");
        let dependencies = json["dependencies"]
            .as_array()
            .expect("manifest dependencies should serialize as rows");
        assert_eq!(dependencies.len(), 1);
        assert!(
            dependencies[0].get("from").is_none(),
            "manifest dependencies should be stored relative to manifest.key"
        );
        assert!(dependencies[0].get("to").is_some());
        assert!(dependencies[0].get("kind").is_some());
        assert!(dependencies[0].get("required_shape").is_some());
        for forbidden in [
            "dependency_index",
            concat!("raw_", "source"),
            concat!("source", "_text"),
            concat!("created", "_at"),
            concat!("m", "time"),
            concat!("run", "_id"),
            concat!("temp", "dir"),
            concat!("abs", "olute"),
        ] {
            assert!(
                json.get(forbidden).is_none(),
                "unexpected field {forbidden}"
            );
        }
    }

    #[test]
    fn disabled_store_bypasses_reads_and_writes_without_filesystem_access() {
        let scratch = scratch_dir();
        let root = scratch.path().join("layers");
        let store = LayerCacheStore::new(&root, false);
        let payload = Payload {
            items: vec!["a".to_string()],
        };
        let layer_key = key();
        let manifest = manifest_for_payload(layer_key.clone(), &payload);

        let write = store.write_json(&manifest, &payload).unwrap();
        let read: LayerCacheReadOutcome<Payload> = store.read_json(&layer_key);

        assert_eq!(write, LayerCacheWriteStatus::BypassedDisabled);
        assert_eq!(read.status, LayerCacheReadStatus::BypassedDisabled);
        assert!(!root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn write_json_rejects_symlink_cache_parent() {
        let scratch = scratch_dir();
        let outside = scratch.path().join("outside");
        std::fs::create_dir_all(&outside).expect("outside dir");
        let root = scratch.path().join("cache/layers");
        std::fs::create_dir_all(root.parent().expect("cache parent")).expect("cache parent");
        std::os::unix::fs::symlink(&outside, &root).expect("symlink layer cache root");
        let store = LayerCacheStore::new(&root, true);
        let payload = Payload {
            items: vec!["a".to_string()],
        };
        let layer_key = key();
        let manifest = manifest_for_payload(layer_key, &payload);

        let error = store
            .write_json(&manifest, &payload)
            .expect_err("symlink layer root should fail");

        assert!(format!("{error:#}").contains("path escapes repository root"));
        assert!(!outside.join("blobs").exists());
    }

    #[test]
    fn temp_paths_are_unique_for_repeated_writes() {
        let scratch = scratch_dir();
        let final_path = scratch.path().join("payload.json");

        let first = temp_path_for(&final_path);
        let second = temp_path_for(&final_path);

        assert_ne!(first, second);
        assert_eq!(first.parent(), final_path.parent());
        assert_eq!(second.parent(), final_path.parent());
    }

    #[test]
    fn stale_same_layer_manifest_is_evicted_on_miss_through_lazy_dependency_index() {
        let scratch = scratch_dir();
        let store = LayerCacheStore::new(scratch.path().join("layers"), true);
        let payload = Payload {
            items: vec!["a".to_string()],
        };
        let stale_key = derived_key();
        let requested_key = changed_derived_key();
        let manifest = manifest_for_payload(stale_key.clone(), &payload);
        store.write_json(&manifest, &payload).unwrap();
        let stale_manifest_path = store.manifest_path_for_test(&stale_key);

        let outcome: LayerCacheReadOutcome<Payload> = store.read_json(&requested_key);

        assert_eq!(outcome.status, LayerCacheReadStatus::Miss);
        assert!(
            !stale_manifest_path.exists(),
            "stale manifest should be evicted after dependency-index invalidation"
        );
    }

    #[test]
    fn write_json_rejects_manifest_metadata_that_read_path_would_reject() {
        let scratch = scratch_dir();
        let store = LayerCacheStore::new(scratch.path().join("layers"), true);
        let payload = Payload {
            items: vec!["a".to_string()],
        };
        let layer_key = derived_key();
        let mut manifest = manifest_for_payload(layer_key, &payload);
        manifest.dependency_index_schema = "old-dependency-index".to_string();

        let error = store.write_json(&manifest, &payload).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unsupported layer cache manifest metadata")
        );
    }

    #[test]
    fn rejects_path_traversal_payload_digest() {
        let scratch = scratch_dir();
        let store = LayerCacheStore::new(scratch.path().join("layers"), true);
        let payload = Payload {
            items: vec!["a".to_string()],
        };
        let layer_key = key();
        let mut manifest = manifest_for_payload(layer_key.clone(), &payload);
        manifest.payload_digest.value = "../outside".to_string();
        write_manifest_only(&store, &layer_key, &manifest);

        let outcome: LayerCacheReadOutcome<Payload> = store.read_json(&layer_key);

        assert_eq!(outcome.status, LayerCacheReadStatus::InvalidEvicted);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape_payload() {
        let scratch = scratch_dir();
        let store = LayerCacheStore::new(scratch.path().join("layers"), true);
        let payload = Payload {
            items: vec!["a".to_string()],
        };
        let layer_key = key();
        let manifest = manifest_for_payload(layer_key.clone(), &payload);
        store.write_json(&manifest, &payload).unwrap();
        let blob_path = store
            .blobs_dir_for_test()
            .join(format!("{}.json", manifest.payload_digest.value));
        let outside_payload = scratch.path().join("outside-payload.json");
        std::fs::write(
            &outside_payload,
            serde_json::to_vec(&payload).expect("payload serializes"),
        )
        .unwrap();
        std::fs::remove_file(&blob_path).unwrap();
        std::os::unix::fs::symlink(&outside_payload, &blob_path).unwrap();

        let outcome: LayerCacheReadOutcome<Payload> = store.read_json(&layer_key);

        assert_eq!(outcome.status, LayerCacheReadStatus::InvalidEvicted);
    }

    #[test]
    fn schema_mismatch_output_digest_mismatch_dependency_index_schema_mismatch_and_missing_dependency_return_invalid()
     {
        let scratch = scratch_dir();
        let store = LayerCacheStore::new(scratch.path().join("layers"), true);
        let payload = Payload {
            items: vec!["a".to_string()],
        };
        let layer_key = derived_key();
        let mut manifest = manifest_for_payload(layer_key.clone(), &payload);
        manifest.dependency_index_schema = "old-dependency-index".to_string();
        store
            .write_json_without_repair_for_test(&manifest, &payload)
            .unwrap();

        let dependency_index_schema_mismatch: LayerCacheReadOutcome<Payload> =
            store.read_json(&layer_key);

        assert_eq!(
            dependency_index_schema_mismatch.status,
            LayerCacheReadStatus::InvalidEvicted
        );

        let mut manifest = manifest_for_payload(layer_key.clone(), &payload);
        manifest.output_digest = digest(DigestKind::ProviderOutput, "wrong-output");
        store
            .write_json_without_repair_for_test(&manifest, &payload)
            .unwrap();

        let output_digest_mismatch: LayerCacheReadOutcome<Payload> = store
            .read_json_validated(&layer_key, |_, manifest| {
                manifest.output_digest == digest(DigestKind::ProviderOutput, "output")
            });

        assert_eq!(
            output_digest_mismatch.status,
            LayerCacheReadStatus::InvalidEvicted
        );

        let mut manifest = manifest_for_payload(layer_key.clone(), &payload);
        manifest.dependencies.clear();
        store
            .write_json_without_repair_for_test(&manifest, &payload)
            .unwrap();

        let missing_dependency: LayerCacheReadOutcome<Payload> = store.read_json(&layer_key);

        assert_eq!(
            missing_dependency.status,
            LayerCacheReadStatus::InvalidEvicted
        );
    }

    #[test]
    fn deserialization_wrong_shape_and_corrupt_payload_return_invalid() {
        let scratch = scratch_dir();
        let store = LayerCacheStore::new(scratch.path().join("layers"), true);
        let payload = Payload {
            items: vec!["a".to_string()],
        };
        let layer_key = key();
        let manifest = manifest_for_payload(layer_key.clone(), &payload);
        store.write_json(&manifest, &payload).unwrap();
        std::fs::write(
            store
                .blobs_dir_for_test()
                .join(format!("{}.json", manifest.payload_digest.value)),
            r#"{"items":"not-a-list"}"#,
        )
        .unwrap();

        let wrong_shape: LayerCacheReadOutcome<Payload> = store.read_json(&layer_key);

        assert_eq!(wrong_shape.status, LayerCacheReadStatus::InvalidEvicted);

        let manifest = manifest_for_payload(layer_key.clone(), &payload);
        store.write_json(&manifest, &payload).unwrap();
        std::fs::write(
            store
                .blobs_dir_for_test()
                .join(format!("{}.json", manifest.payload_digest.value)),
            b"\xff\xfe\xfd",
        )
        .unwrap();

        let corrupt_payload: LayerCacheReadOutcome<Payload> = store.read_json(&layer_key);

        assert_eq!(corrupt_payload.status, LayerCacheReadStatus::InvalidEvicted);
    }
}
