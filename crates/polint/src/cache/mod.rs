use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub(crate) mod analysis_cache_adapter;
pub(crate) mod keys;
pub(crate) mod rules_store;

pub(crate) use analysis_cache_adapter::CacheAnalysisCache;

pub(crate) const CACHE_VERSION: &str = concat!("polint-cache-v1:", env!("CARGO_PKG_VERSION"));
pub(crate) const POLINT_CACHE_DIR_ENV: &str = "POLINT_CACHE_DIR";
pub(crate) const POLINT_RULES_TARGET_DIR_ENV: &str = "POLINT_RULES_TARGET_DIR";
const ANALYSIS_CACHE_MAX_BYTES: u64 = 16 * 1_048_576;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CacheKey {
    pub(crate) file_hash: String,
    pub(crate) config_hash: String,
    pub(crate) rule_hash: String,
    pub(crate) plan_hash: String,
    pub(crate) version: String,
    pub(crate) schema: String,
    /// Identity of the parser whose facts this entry holds. Without it the entry
    /// is keyed only by engine version, so a parser or grammar bump reuses facts
    /// produced by the previous parser.
    pub(crate) parser_identity: String,
}

impl CacheKey {
    /// Test-only constructor — production code uses [`CacheKey::for_file`].
    #[cfg(test)]
    pub(crate) fn new(
        file_hash: impl Into<String>,
        config_hash: impl Into<String>,
        rule_hash: impl Into<String>,
    ) -> Self {
        Self {
            file_hash: file_hash.into(),
            config_hash: config_hash.into(),
            rule_hash: rule_hash.into(),
            plan_hash: String::new(),
            version: CACHE_VERSION.to_string(),
            schema: "analysis-facts-v1".to_string(),
            parser_identity: "test-parser".to_string(),
        }
    }

    pub(crate) fn for_file(
        relative_path: &str,
        content_hash: &str,
        config_hash: &str,
        rule_hash: &str,
        plan_hash: &str,
        schema: &str,
        parser_identity: &str,
    ) -> Self {
        Self {
            file_hash: stable_hash(&[relative_path, content_hash]),
            config_hash: config_hash.to_string(),
            rule_hash: rule_hash.to_string(),
            plan_hash: plan_hash.to_string(),
            version: CACHE_VERSION.to_string(),
            schema: schema.to_string(),
            parser_identity: parser_identity.to_string(),
        }
    }

    pub(crate) fn stable_id(&self) -> String {
        stable_hash(&[
            &self.file_hash,
            &self.config_hash,
            &self.rule_hash,
            &self.plan_hash,
            &self.version,
            &self.schema,
            &self.parser_identity,
        ])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheReadStatus {
    Disabled,
    Miss,
    Hit,
    InvalidEvicted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CacheReadOutcome<T> {
    pub(crate) value: Option<T>,
    pub(crate) status: CacheReadStatus,
}

/// Outcome of reading an entry off disk, before it is interpreted as JSON.
enum CacheEntryRead {
    Found { raw: String, path: PathBuf },
    Absent(CacheReadStatus),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheWriteStatus {
    Disabled,
    Written,
}

/// Disk-backed JSON cache (used from adapters in-tree; `polint::_bench::cache` for `polint-bench`).
#[allow(unreachable_pub)]
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
    repo_root: Option<PathBuf>,
    enabled: bool,
    semantic_store_enabled: bool,
}

impl Cache {
    pub(crate) fn new(root: impl AsRef<Path>, enabled: bool) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            repo_root: None,
            enabled,
            semantic_store_enabled: false,
        }
    }

    /// Used by `polint-bench` (`feature = "bench"`) and in-crate callers.
    #[allow(unreachable_pub)]
    pub fn default_for_repo(repo: impl AsRef<Path>, enabled: bool) -> Self {
        let layout = CacheLayout::for_repo(repo.as_ref());
        let mut cache = Self::new(layout.analysis_dir(), enabled);
        if std::env::var_os(POLINT_CACHE_DIR_ENV)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            cache.repo_root = Some(repo.as_ref().to_path_buf());
        }
        cache
    }

    #[cfg(test)]
    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[cfg(test)]
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn semantic_store_path(&self) -> PathBuf {
        self.semantic_store_dir().join("store.sqlite3")
    }

    pub(crate) fn semantic_store_enabled(&self) -> bool {
        self.semantic_store_enabled
    }

    #[cfg(test)]
    pub(crate) fn with_semantic_store_enabled_for_test(mut self) -> Self {
        self.semantic_store_enabled = true;
        self
    }

    fn semantic_store_dir(&self) -> PathBuf {
        if self.root.file_name().and_then(|name| name.to_str()) == Some("analysis")
            && let Some(parent) = self.root.parent()
        {
            return parent.join("semantic-store");
        }
        self.root.join("semantic-store")
    }

    pub(crate) fn sidecar_cache_dir(&self) -> Option<PathBuf> {
        if self.root.as_os_str().is_empty() || !self.enabled {
            return None;
        }
        if self.root.file_name().and_then(|name| name.to_str()) == Some("analysis")
            && let Some(parent) = self.root.parent()
        {
            return Some(parent.join("sidecar"));
        }
        Some(self.root.join("sidecar"))
    }

    pub(crate) fn layer_cache_dir(&self) -> PathBuf {
        if self.root.file_name().and_then(|name| name.to_str()) == Some("analysis")
            && let Some(parent) = self.root.parent()
        {
            return parent.join("layers");
        }
        self.root.join("layers")
    }

    pub(crate) fn layer_cache_store(&self) -> crate::analysis_kernel::incremental::LayerCacheStore {
        crate::analysis_kernel::incremental::LayerCacheStore::new_with_repo_root(
            self.layer_cache_dir(),
            self.enabled,
            self.repo_root.clone(),
        )
    }

    #[cfg(test)]
    pub(crate) fn read_json<T>(&self, key: &CacheKey) -> Result<Option<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        if !self.enabled {
            return Ok(None);
        }
        let path = self.path_for(key);
        let raw = match self.read_cache_file_to_string(&path) {
            Ok(Some(raw)) => raw,
            Ok(None) | Err(CacheFileAccess::Unsafe) => return Ok(None),
            Err(CacheFileAccess::ReadFailed(_)) => {
                return Err(anyhow::anyhow!("failed to read cache {}", path.display()));
            }
        };
        Ok(Some(serde_json::from_str(&raw)?))
    }

    #[allow(dead_code)]
    pub(crate) fn read_json_or_miss<T>(&self, key: &CacheKey) -> Option<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.read_json_with_status(key).value
    }

    pub(crate) fn read_json_with_status<T>(&self, key: &CacheKey) -> CacheReadOutcome<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let (raw, path) = match self.read_entry(key) {
            CacheEntryRead::Found { raw, path } => (raw, path),
            CacheEntryRead::Absent(status) => {
                return CacheReadOutcome {
                    value: None,
                    status,
                };
            }
        };
        match serde_json::from_str(&raw) {
            Ok(value) => CacheReadOutcome {
                value: Some(value),
                status: CacheReadStatus::Hit,
            },
            Err(_) => {
                let _ = fs::remove_file(path);
                CacheReadOutcome {
                    value: None,
                    status: CacheReadStatus::InvalidEvicted,
                }
            }
        }
    }

    /// Hands the stored entry back as raw JSON bytes, so a caller that knows the
    /// concrete type parses it exactly once instead of going through an
    /// intermediate [`serde_json::Value`]. The entry is still checked for
    /// well-formed JSON, and an unparsable one is evicted, so the hit, miss and
    /// eviction outcomes match [`Cache::read_json_with_status`].
    pub(crate) fn read_json_bytes_with_status(&self, key: &CacheKey) -> CacheReadOutcome<Vec<u8>> {
        let (raw, path) = match self.read_entry(key) {
            CacheEntryRead::Found { raw, path } => (raw, path),
            CacheEntryRead::Absent(status) => {
                return CacheReadOutcome {
                    value: None,
                    status,
                };
            }
        };
        match serde_json::from_str::<serde::de::IgnoredAny>(&raw) {
            Ok(_) => CacheReadOutcome {
                value: Some(raw.into_bytes()),
                status: CacheReadStatus::Hit,
            },
            Err(_) => {
                let _ = fs::remove_file(path);
                CacheReadOutcome {
                    value: None,
                    status: CacheReadStatus::InvalidEvicted,
                }
            }
        }
    }

    fn read_entry(&self, key: &CacheKey) -> CacheEntryRead {
        if !self.enabled {
            return CacheEntryRead::Absent(CacheReadStatus::Disabled);
        }
        let path = self.path_for(key);
        match self.read_cache_file_to_string(&path) {
            Ok(Some(raw)) => CacheEntryRead::Found { raw, path },
            Ok(None) => CacheEntryRead::Absent(CacheReadStatus::Miss),
            Err(CacheFileAccess::Unsafe) => CacheEntryRead::Absent(CacheReadStatus::InvalidEvicted),
            Err(CacheFileAccess::ReadFailed(managed_path)) => {
                evict_file(&managed_path);
                CacheEntryRead::Absent(CacheReadStatus::InvalidEvicted)
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn write_json<T>(&self, key: &CacheKey, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        self.write_json_with_status(key, value)?;
        Ok(())
    }

    pub(crate) fn write_json_with_status<T>(
        &self,
        key: &CacheKey,
        value: &T,
    ) -> Result<CacheWriteStatus>
    where
        T: Serialize,
    {
        if !self.enabled {
            return Ok(CacheWriteStatus::Disabled);
        }
        let path = self.path_for(key);
        self.write_cache_file_atomic(&path, serde_json::to_vec(value)?)
            .with_context(|| format!("failed to write cache {}", path.display()))?;
        Ok(CacheWriteStatus::Written)
    }

    /// Validates and stores JSON bytes without materializing an intermediate
    /// [`serde_json::Value`] or serializing the payload a second time.
    ///
    /// Frontends use this after serializing their concrete cache payload. A
    /// disabled cache returns before validation so cache bypass never parses a
    /// payload that will not be persisted.
    pub(crate) fn write_json_bytes_with_status(
        &self,
        key: &CacheKey,
        bytes: &[u8],
    ) -> Result<CacheWriteStatus> {
        if !self.enabled {
            return Ok(CacheWriteStatus::Disabled);
        }
        serde_json::from_slice::<serde::de::IgnoredAny>(bytes)
            .context("cache bytes must contain valid JSON")?;
        let path = self.path_for(key);
        self.write_cache_file_atomic(&path, bytes)
            .with_context(|| format!("failed to write cache {}", path.display()))?;
        Ok(CacheWriteStatus::Written)
    }

    fn path_for(&self, key: &CacheKey) -> PathBuf {
        self.root.join(format!("{}.json", key.stable_id()))
    }

    fn read_cache_file_to_string(&self, path: &Path) -> Result<Option<String>, CacheFileAccess> {
        if let Some(repo_root) = &self.repo_root {
            let Some(relative_path) = self.repo_relative_path(path) else {
                return Err(CacheFileAccess::Unsafe);
            };
            return match crate::repo_fs::read_repo_file_to_string_with_limit(
                repo_root,
                relative_path,
                ANALYSIS_CACHE_MAX_BYTES,
            ) {
                Ok(raw) => Ok(Some(raw)),
                Err(error) if error.is_not_found() => Ok(None),
                Err(_) => Err(CacheFileAccess::Unsafe),
            };
        }
        if crate::repo_fs::ensure_no_symlink_ancestors(path).is_err() {
            return Err(CacheFileAccess::Unsafe);
        }
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return Ok(None);
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(CacheFileAccess::Unsafe);
        }
        let Some(path) = crate::repo_fs::managed_existing_file(&self.root, &self.root, path) else {
            return Err(CacheFileAccess::Unsafe);
        };
        crate::repo_fs::read_file_to_string_with_limit(&path, ANALYSIS_CACHE_MAX_BYTES)
            .map(Some)
            .map_err(|_| CacheFileAccess::ReadFailed(path))
    }

    fn write_cache_file_atomic(
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

    fn repo_relative_path(&self, path: &Path) -> Option<PathBuf> {
        let repo_root = self.repo_root.as_ref()?;
        let relative_path = path.strip_prefix(repo_root).ok()?;
        crate::repo_fs::normalize_repo_relative_input(relative_path)
    }
}

enum CacheFileAccess {
    Unsafe,
    ReadFailed(PathBuf),
}

#[derive(Debug, Clone)]
pub(crate) struct CacheLayout {
    root: PathBuf,
}

impl CacheLayout {
    pub(crate) fn for_repo(repo: impl AsRef<Path>) -> Self {
        Self {
            root: env_path_or_repo_default(repo.as_ref(), POLINT_CACHE_DIR_ENV, ".polint/cache"),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_root(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn analysis_dir(&self) -> PathBuf {
        self.root.join("analysis")
    }

    /// Cargo target directory for the repo-local rule host.
    ///
    /// This holds compiler output rather than source-validated analysis
    /// artifacts, and the `rules-target` name is part of the published CI
    /// contract: `action.yml` restores and saves it under a key built from
    /// build inputs (runner OS/arch, resolved toolchain, compiler flags,
    /// manifests and lockfiles), separate from the analysis cache, and removes
    /// every rule package's own output from it before saving so a cached host
    /// binary never outlives its sources. `POLINT_RULES_TARGET_DIR` moves the
    /// directory for callers that manage it themselves, and the action skips
    /// its build cache when that variable moves it out of the cache root. See
    /// `cache_layout_matches_the_github_action_contract`.
    pub(crate) fn rules_target_dir(&self) -> PathBuf {
        env_path_or_cache_default(
            &self.root,
            POLINT_RULES_TARGET_DIR_ENV,
            Path::new("rules-target"),
        )
    }

    /// Cargo target directory for repo-local extension crates.
    ///
    /// Compiler output like [`CacheLayout::rules_target_dir`], built by the
    /// extension host from `.polint/extensions/*`, and cached under the same
    /// build-input key with the same prune-before-save rule.
    pub(crate) fn extensions_target_dir(&self) -> PathBuf {
        self.root.join("extensions-target")
    }

    pub(crate) fn derived_dir(&self) -> PathBuf {
        self.root.join("derived")
    }

    pub(crate) fn layer_cache_dir(&self) -> PathBuf {
        self.root.join("layers")
    }

    pub(crate) fn semantic_store_dir(&self) -> PathBuf {
        self.root.join("semantic-store")
    }

    /// Serialized `polint review` changesets.
    ///
    /// Per-run scratch: each file is named after a hash of the changeset it
    /// holds and is rewritten from the diff being reviewed, so restoring one
    /// from CI would save no work and only grow the entry. `action.yml`
    /// deliberately does not cache it.
    pub(crate) fn review_dir(&self) -> PathBuf {
        self.root.join("review")
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "kept for private internal consumers")
    )]
    pub(crate) fn semantic_store_path(&self) -> PathBuf {
        self.semantic_store_dir().join("store.sqlite3")
    }

    pub(crate) fn status(&self) -> Result<CacheStatus> {
        let categories = CacheManagedCategory::ALL
            .iter()
            .map(|category| {
                self.status_for(
                    category.name(),
                    category.role(),
                    self.category_path(*category),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let total_bytes = categories.iter().map(|category| category.bytes).sum();
        let total_files = categories.iter().map(|category| category.files).sum();
        Ok(CacheStatus {
            root: self.root.display().to_string(),
            categories,
            total_bytes,
            total_files,
        })
    }

    fn status_for(
        &self,
        name: &'static str,
        role: CacheDirRole,
        path: PathBuf,
    ) -> Result<CacheCategoryStatus> {
        let stats = directory_stats(&path)?;
        Ok(CacheCategoryStatus {
            name,
            role: role.name(),
            path: path.display().to_string(),
            exists: stats.exists,
            bytes: stats.bytes,
            files: stats.files,
        })
    }

    pub(crate) fn clean(&self, selection: CacheCleanSelection) -> Result<CacheCleanReport> {
        let mut report = CacheCleanReport::default();
        let categories = match selection {
            CacheCleanSelection::All => CacheManagedCategory::ALL.to_vec(),
            CacheCleanSelection::Category(category) => vec![category],
        };

        for category in categories {
            let path = self.category_path(category);
            let before = directory_stats(&path)?;
            if fs::symlink_metadata(&path).is_ok() {
                remove_cache_path(&path)
                    .with_context(|| format!("failed to remove cache {}", path.display()))?;
            }
            report.removed_bytes += before.bytes;
            report.removed_files += before.files;
            report.categories.push(CacheCleanCategoryReport {
                name: category.name(),
                path: path.display().to_string(),
                removed_bytes: before.bytes,
                removed_files: before.files,
            });
        }

        Ok(report)
    }

    pub(crate) fn prune(&self, options: &CachePruneOptions) -> Result<CachePruneReport> {
        let categories = if options.categories.is_empty() {
            CacheManagedCategory::ALL.to_vec()
        } else {
            options.categories.clone()
        };

        let mut report = CachePruneReport {
            dry_run: options.dry_run,
            ..CachePruneReport::default()
        };
        for category in categories {
            let category_report = self.prune_category(category, options)?;
            report.removed_bytes += category_report.removed_bytes;
            report.removed_files += category_report.removed_files;
            report.categories.push(category_report);
        }
        Ok(report)
    }

    fn prune_category(
        &self,
        category: CacheManagedCategory,
        options: &CachePruneOptions,
    ) -> Result<CachePruneCategoryReport> {
        let path = self.category_path(category);
        let before = directory_stats(&path)?;
        let mut candidates = Vec::new();
        collect_cache_files(&path, &mut candidates)?;
        candidates.sort_by(|left, right| {
            left.modified
                .cmp(&right.modified)
                .then_with(|| left.path.cmp(&right.path))
        });

        let now = SystemTime::now();
        let mut selected = std::collections::BTreeSet::new();
        if let Some(max_age) = options.max_age {
            for file in &candidates {
                if now.duration_since(file.modified).unwrap_or(Duration::ZERO) > max_age {
                    selected.insert(file.path.clone());
                }
            }
        }

        if let Some(max_bytes) = options.max_bytes
            && before.bytes > max_bytes
        {
            let mut projected = before.bytes;
            for file in &candidates {
                if projected <= max_bytes {
                    break;
                }
                if selected.insert(file.path.clone()) {
                    projected = projected.saturating_sub(file.bytes);
                }
            }
        }

        let mut removed_bytes = 0;
        let mut removed_files = 0;
        for file in candidates
            .iter()
            .filter(|file| selected.contains(&file.path))
        {
            removed_bytes += file.bytes;
            removed_files += 1;
            if !options.dry_run && file.path.is_file() {
                fs::remove_file(&file.path).with_context(|| {
                    format!("failed to remove cache file {}", file.path.display())
                })?;
            }
        }
        if !options.dry_run {
            remove_empty_dirs(&path)?;
        }

        Ok(CachePruneCategoryReport {
            name: category.name(),
            path: path.display().to_string(),
            before_bytes: before.bytes,
            before_files: before.files,
            removed_bytes,
            removed_files,
        })
    }

    fn category_path(&self, category: CacheManagedCategory) -> PathBuf {
        match category {
            CacheManagedCategory::Analysis => self.analysis_dir(),
            CacheManagedCategory::Layers => self.layer_cache_dir(),
            CacheManagedCategory::Derived => self.derived_dir(),
            CacheManagedCategory::Semantic => self.semantic_store_dir(),
            CacheManagedCategory::RulesTarget => self.rules_target_dir(),
            CacheManagedCategory::ExtensionsTarget => self.extensions_target_dir(),
            CacheManagedCategory::Review => self.review_dir(),
        }
    }
}

fn env_path_or_repo_default(repo: &Path, env_key: &str, default: &str) -> PathBuf {
    match std::env::var_os(env_key).filter(|value| !value.is_empty()) {
        Some(path) => absolutize_env_path(repo, PathBuf::from(path)),
        None => repo.join(default),
    }
}

fn env_path_or_cache_default(cache_root: &Path, env_key: &str, default: &Path) -> PathBuf {
    match std::env::var_os(env_key).filter(|value| !value.is_empty()) {
        Some(path) => absolutize_env_path(cache_root, PathBuf::from(path)),
        None => cache_root.join(default),
    }
}

fn absolutize_env_path(base: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

/// What a cache directory holds, and therefore how CI has to treat it.
///
/// `action.yml` keys the two cached roles differently and caches neither
/// scratch data nor anything it cannot classify; see
/// `cache_layout_matches_the_github_action_contract`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheDirRole {
    /// Artifacts polint re-validates against current sources on every read, so
    /// a restored entry can only save recomputation, never answer for a source
    /// it does not match.
    SourceValidated,
    /// Cargo output. Only the compiler can judge its freshness, so it is keyed
    /// on build inputs and pruned of repo-local package output before saving.
    CompilerOutput,
    /// Per-run scratch, rewritten from the current inputs. Caching it would
    /// save no work and only grow the entry.
    Scratch,
}

impl CacheDirRole {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::SourceValidated => "source-validated",
            Self::CompilerOutput => "compiler-output",
            Self::Scratch => "scratch",
        }
    }
}

/// Every directory polint writes under the cache root.
///
/// This is the single list `polint cache status`, `polint cache clean`,
/// `polint cache prune`, and the GitHub Action contract test all read, so a new
/// cache directory cannot quietly appear in one of them and be missing from the
/// others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheManagedCategory {
    Analysis,
    Layers,
    Derived,
    /// The `semantic-store` directory, named for the directory rather than the
    /// store type that fills it: that type stays private to the kernel.
    Semantic,
    RulesTarget,
    ExtensionsTarget,
    Review,
}

impl CacheManagedCategory {
    pub(crate) const ALL: &'static [Self] = &[
        Self::Analysis,
        Self::Layers,
        Self::Derived,
        Self::Semantic,
        Self::RulesTarget,
        Self::ExtensionsTarget,
        Self::Review,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Analysis => "analysis",
            Self::Layers => "layers",
            Self::Derived => "derived",
            Self::Semantic => "semantic-store",
            Self::RulesTarget => "rules-target",
            Self::ExtensionsTarget => "extensions-target",
            Self::Review => "review",
        }
    }

    pub(crate) fn role(self) -> CacheDirRole {
        match self {
            Self::Analysis | Self::Layers | Self::Derived | Self::Semantic => {
                CacheDirRole::SourceValidated
            }
            Self::RulesTarget | Self::ExtensionsTarget => CacheDirRole::CompilerOutput,
            Self::Review => CacheDirRole::Scratch,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CacheCleanSelection {
    All,
    Category(CacheManagedCategory),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CachePruneOptions {
    pub(crate) categories: Vec<CacheManagedCategory>,
    pub(crate) max_age: Option<Duration>,
    pub(crate) max_bytes: Option<u64>,
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CacheStatus {
    pub(crate) root: String,
    pub(crate) categories: Vec<CacheCategoryStatus>,
    pub(crate) total_bytes: u64,
    pub(crate) total_files: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CacheCategoryStatus {
    pub(crate) name: &'static str,
    pub(crate) role: &'static str,
    pub(crate) path: String,
    pub(crate) exists: bool,
    pub(crate) bytes: u64,
    pub(crate) files: u64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CacheCleanReport {
    pub(crate) removed_bytes: u64,
    pub(crate) removed_files: u64,
    pub(crate) categories: Vec<CacheCleanCategoryReport>,
}

#[derive(Debug, Clone)]
pub(crate) struct CacheCleanCategoryReport {
    pub(crate) name: &'static str,
    pub(crate) path: String,
    pub(crate) removed_bytes: u64,
    pub(crate) removed_files: u64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CachePruneReport {
    pub(crate) dry_run: bool,
    pub(crate) removed_bytes: u64,
    pub(crate) removed_files: u64,
    pub(crate) categories: Vec<CachePruneCategoryReport>,
}

#[derive(Debug, Clone)]
pub(crate) struct CachePruneCategoryReport {
    pub(crate) name: &'static str,
    pub(crate) path: String,
    pub(crate) before_bytes: u64,
    pub(crate) before_files: u64,
    pub(crate) removed_bytes: u64,
    pub(crate) removed_files: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct CacheDirStats {
    exists: bool,
    bytes: u64,
    files: u64,
}

#[derive(Debug, Clone)]
struct CacheFile {
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}

fn directory_stats(path: &Path) -> Result<CacheDirStats> {
    let mut stats = CacheDirStats {
        exists: true,
        ..CacheDirStats::default()
    };
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(CacheDirStats::default());
    };
    if metadata.is_dir() {
        add_directory_stats(path, &mut stats)?;
    } else if metadata.is_file() {
        stats.files = 1;
        stats.bytes = metadata.len();
    }
    Ok(stats)
}

fn add_directory_stats(path: &Path, stats: &mut CacheDirStats) -> Result<()> {
    for entry in fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() {
            add_directory_stats(&entry.path(), stats)?;
        } else if metadata.is_file() {
            stats.files += 1;
            stats.bytes += metadata.len();
        }
    }
    Ok(())
}

fn collect_cache_files(path: &Path, files: &mut Vec<CacheFile>) -> Result<()> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if !metadata.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        let path = entry.path();
        if metadata.is_dir() {
            collect_cache_files(&path, files)?;
        } else if metadata.is_file() {
            files.push(CacheFile {
                path,
                bytes: metadata.len(),
                modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }
    }
    Ok(())
}

fn remove_cache_path(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn evict_file(path: &Path) {
    let _ = fs::remove_file(path);
}

fn remove_empty_dirs(path: &Path) -> Result<bool> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(false);
    };
    if !metadata.is_dir() {
        return Ok(false);
    }
    let mut is_empty = true;
    for entry in fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry.file_type()?.is_dir() {
            if !remove_empty_dirs(&entry_path)? {
                is_empty = false;
            }
        } else {
            is_empty = false;
        }
    }
    if is_empty {
        fs::remove_dir(path)
            .with_context(|| format!("failed to remove empty cache dir {}", path.display()))?;
    }
    Ok(is_empty)
}

/// Deterministic cache key component hash (also used by `polint-bench` via `_bench::cache`).
#[allow(unreachable_pub)]
pub fn stable_hash(parts: &[&str]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xfe;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn cache_key_changes_with_config() {
        let a = CacheKey::new("file", "config-a", "rule");
        let b = CacheKey::new("file", "config-b", "rule");
        assert_ne!(a.stable_id(), b.stable_id());
    }

    #[test]
    fn cache_key_changes_with_rule_hash() {
        let a = CacheKey::for_file(
            "src/main.go",
            "content",
            "config",
            "rule-a",
            "plan",
            "go-facts-v1",
            "tree-sitter-0.26.8+tree-sitter-go-0.25.0",
        );
        let b = CacheKey::for_file(
            "src/main.go",
            "content",
            "config",
            "rule-b",
            "plan",
            "go-facts-v1",
            "tree-sitter-0.26.8+tree-sitter-go-0.25.0",
        );

        assert_ne!(a.stable_id(), b.stable_id());
    }

    #[test]
    fn cache_key_changes_with_plan_hash() {
        let a = CacheKey::for_file(
            "src/main.go",
            "content",
            "config",
            "rule",
            "plan-a",
            "go-facts-v1",
            "tree-sitter-0.26.8+tree-sitter-go-0.25.0",
        );
        let b = CacheKey::for_file(
            "src/main.go",
            "content",
            "config",
            "rule",
            "plan-b",
            "go-facts-v1",
            "tree-sitter-0.26.8+tree-sitter-go-0.25.0",
        );

        assert_ne!(a.stable_id(), b.stable_id());
    }

    #[test]
    fn cache_key_changes_with_schema() {
        let a = CacheKey::for_file(
            "src/main.go",
            "content",
            "config",
            "rule",
            "plan",
            "go-facts-v1",
            "tree-sitter-0.26.8+tree-sitter-go-0.25.0",
        );
        let b = CacheKey::for_file(
            "src/main.go",
            "content",
            "config",
            "rule",
            "plan",
            "ts-facts-v1",
            "tree-sitter-0.26.8+tree-sitter-go-0.25.0",
        );

        assert_ne!(a.stable_id(), b.stable_id());
    }

    /// Cached facts are only interchangeable with freshly parsed ones when the
    /// parser matches. Without this component a parser or grammar bump reuses
    /// facts produced by the previous parser at a key that claims they are current.
    #[test]
    fn cache_key_changes_with_parser_identity() {
        let a = CacheKey::for_file(
            "src/main.go",
            "content",
            "config",
            "rule",
            "plan",
            "go-facts-v1",
            "tree-sitter-0.26.8+tree-sitter-go-0.25.0",
        );
        let b = CacheKey::for_file(
            "src/main.go",
            "content",
            "config",
            "rule",
            "plan",
            "go-facts-v1",
            "tree-sitter-0.26.9+tree-sitter-go-0.25.0",
        );

        assert_ne!(a.stable_id(), b.stable_id());
    }

    #[test]
    fn cache_key_changes_with_relative_path() {
        let a = CacheKey::for_file(
            "src/main.go",
            "content",
            "config",
            "rule",
            "plan",
            "go-facts-v1",
            "tree-sitter-0.26.8+tree-sitter-go-0.25.0",
        );
        let b = CacheKey::for_file(
            "src/other.go",
            "content",
            "config",
            "rule",
            "plan",
            "go-facts-v1",
            "tree-sitter-0.26.8+tree-sitter-go-0.25.0",
        );

        assert_ne!(a.stable_id(), b.stable_id());
    }

    #[test]
    fn disabled_cache_does_not_create_cache_directory() {
        let temp = tempfile::tempdir().unwrap();
        let cache = Cache::default_for_repo(temp.path(), false);
        let key = CacheKey::for_file(
            "src/main.go",
            "content",
            "config",
            "rule",
            "plan",
            "go-facts-v1",
            "tree-sitter-0.26.8+tree-sitter-go-0.25.0",
        );

        cache.write_json(&key, &json!({ "ok": true })).unwrap();

        assert!(!cache.root().exists());
        assert!(!temp.path().join(".polint/cache").exists());
        assert!(!cache.is_enabled());
    }

    #[test]
    fn semantic_store_is_disabled_and_filesystem_free_by_default() {
        let temp = tempfile::tempdir().unwrap();
        let cache = Cache::default_for_repo(temp.path(), false);

        assert!(!cache.semantic_store_enabled());
        assert_eq!(
            cache.semantic_store_path(),
            temp.path()
                .join(".polint/cache/semantic-store/store.sqlite3")
        );
        assert!(!temp.path().join(".polint/cache").exists());
    }

    #[test]
    fn semantic_store_test_enablement_changes_only_activation_state() {
        let temp = tempfile::tempdir().unwrap();
        let cache =
            Cache::default_for_repo(temp.path(), false).with_semantic_store_enabled_for_test();

        assert!(cache.semantic_store_enabled());
        assert_eq!(
            cache.semantic_store_path(),
            temp.path()
                .join(".polint/cache/semantic-store/store.sqlite3")
        );
        assert!(!temp.path().join(".polint/cache").exists());
    }

    #[test]
    fn cache_layout_semantic_store_path_stays_under_configured_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("custom-cache");
        let layout = CacheLayout::from_root(&root);

        assert_eq!(
            layout.semantic_store_path(),
            root.join("semantic-store/store.sqlite3")
        );
        assert!(!root.exists());
    }

    #[test]
    fn cache_json_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let cache = Cache::default_for_repo(temp.path(), true);
        let key = CacheKey::for_file(
            "src/main.go",
            "content",
            "config",
            "rule",
            "plan",
            "go-facts-v1",
            "tree-sitter-0.26.8+tree-sitter-go-0.25.0",
        );
        let value = json!({ "diagnostics": [], "schema": "go-facts-v2" });

        cache.write_json(&key, &value).unwrap();
        let restored: serde_json::Value = cache.read_json(&key).unwrap().unwrap();

        assert_eq!(restored, value);
        assert!(cache.root().exists());
        assert_eq!(cache.root(), temp.path().join(".polint/cache/analysis"));
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn cache_json_is_written_compactly() {
        let temp = tempfile::tempdir().unwrap();
        let cache = Cache::default_for_repo(temp.path(), true);
        let key = CacheKey::for_file(
            "src/main.go",
            "content",
            "config",
            "rule",
            "plan",
            "go-facts-v1",
            "tree-sitter-0.26.8+tree-sitter-go-0.25.0",
        );

        cache
            .write_json(&key, &json!({ "schema": "go-facts-v1", "items": [1, 2] }))
            .unwrap();

        let raw = fs::read_to_string(cache.path_for(&key)).unwrap();
        assert_eq!(raw, r#"{"schema":"go-facts-v1","items":[1,2]}"#);
    }

    #[test]
    fn invalid_json_cache_entry_is_deleted_on_miss() {
        let temp = tempfile::tempdir().unwrap();
        let cache = Cache::default_for_repo(temp.path(), true);
        let key = CacheKey::for_file(
            "src/main.go",
            "content",
            "config",
            "rule",
            "plan",
            "go-facts-v1",
            "tree-sitter-0.26.8+tree-sitter-go-0.25.0",
        );
        fs::create_dir_all(cache.root()).unwrap();
        fs::write(cache.path_for(&key), "{not-json").unwrap();

        let restored: Option<serde_json::Value> = cache.read_json_or_miss(&key);

        assert!(restored.is_none());
        assert!(!cache.path_for(&key).exists());
    }

    mod cache_read_status {
        use super::*;

        fn key() -> CacheKey {
            CacheKey::for_file(
                "src/main.go",
                "content",
                "config",
                "rule",
                "plan",
                "go-facts-v1",
                "tree-sitter-0.26.8+tree-sitter-go-0.25.0",
            )
        }

        #[test]
        fn read_json_with_status_returns_disabled_when_cache_is_disabled() {
            let temp = tempfile::tempdir().unwrap();
            let cache = Cache::default_for_repo(temp.path(), false);
            let key = key();

            let outcome: CacheReadOutcome<serde_json::Value> = cache.read_json_with_status(&key);

            assert_eq!(outcome.status, CacheReadStatus::Disabled);
            assert!(outcome.value.is_none());
            assert!(!cache.root().exists());
        }

        #[test]
        fn read_json_with_status_returns_miss_when_entry_is_absent() {
            let temp = tempfile::tempdir().unwrap();
            let cache = Cache::default_for_repo(temp.path(), true);
            let key = key();

            let outcome: CacheReadOutcome<serde_json::Value> = cache.read_json_with_status(&key);

            assert_eq!(outcome.status, CacheReadStatus::Miss);
            assert!(outcome.value.is_none());
        }

        #[test]
        fn read_json_with_status_returns_hit_when_entry_is_valid_json() {
            let temp = tempfile::tempdir().unwrap();
            let cache = Cache::default_for_repo(temp.path(), true);
            let key = key();
            let value = json!({ "schema": "go-facts-v1", "items": [1, 2] });

            cache.write_json(&key, &value).unwrap();
            let outcome: CacheReadOutcome<serde_json::Value> = cache.read_json_with_status(&key);

            assert_eq!(outcome.status, CacheReadStatus::Hit);
            assert_eq!(outcome.value, Some(value));
        }

        #[test]
        fn read_json_with_status_evicts_invalid_json() {
            let temp = tempfile::tempdir().unwrap();
            let cache = Cache::default_for_repo(temp.path(), true);
            let key = key();
            fs::create_dir_all(cache.root()).unwrap();
            fs::write(cache.path_for(&key), "{not-json").unwrap();

            let outcome: CacheReadOutcome<serde_json::Value> = cache.read_json_with_status(&key);

            assert_eq!(outcome.status, CacheReadStatus::InvalidEvicted);
            assert!(outcome.value.is_none());
            assert!(!cache.path_for(&key).exists());
        }

        #[test]
        fn read_json_bytes_with_status_returns_the_stored_bytes_verbatim() {
            let temp = tempfile::tempdir().unwrap();
            let cache = Cache::default_for_repo(temp.path(), true);
            let key = key();
            let value = json!({ "schema": "go-facts-v1", "items": [1, 2] });

            cache.write_json(&key, &value).unwrap();
            let outcome = cache.read_json_bytes_with_status(&key);

            assert_eq!(outcome.status, CacheReadStatus::Hit);
            assert_eq!(outcome.value, Some(fs::read(cache.path_for(&key)).unwrap()));
        }

        #[test]
        fn read_json_bytes_with_status_reports_disabled_and_miss_like_the_typed_read() {
            let temp = tempfile::tempdir().unwrap();
            let key = key();

            let disabled =
                Cache::default_for_repo(temp.path(), false).read_json_bytes_with_status(&key);
            let miss = Cache::default_for_repo(temp.path(), true).read_json_bytes_with_status(&key);

            assert_eq!(disabled.status, CacheReadStatus::Disabled);
            assert!(disabled.value.is_none());
            assert_eq!(miss.status, CacheReadStatus::Miss);
            assert!(miss.value.is_none());
        }

        #[test]
        fn read_json_bytes_with_status_evicts_invalid_json() {
            let temp = tempfile::tempdir().unwrap();
            let cache = Cache::default_for_repo(temp.path(), true);
            let key = key();
            fs::create_dir_all(cache.root()).unwrap();
            fs::write(cache.path_for(&key), "{not-json").unwrap();

            let outcome = cache.read_json_bytes_with_status(&key);

            assert_eq!(outcome.status, CacheReadStatus::InvalidEvicted);
            assert!(outcome.value.is_none());
            assert!(!cache.path_for(&key).exists());
        }

        #[cfg(unix)]
        #[test]
        fn read_json_with_status_rejects_symlink_entry_without_following_it() {
            let temp = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let cache = Cache::default_for_repo(temp.path(), true);
            let key = key();
            fs::create_dir_all(cache.root()).unwrap();
            fs::write(outside.path().join("entry.json"), r#"{"secret":true}"#).unwrap();
            std::os::unix::fs::symlink(outside.path().join("entry.json"), cache.path_for(&key))
                .unwrap();

            let outcome: CacheReadOutcome<serde_json::Value> = cache.read_json_with_status(&key);

            assert_eq!(outcome.status, CacheReadStatus::InvalidEvicted);
            assert!(outcome.value.is_none());
            assert!(cache.path_for(&key).exists());
            assert!(outside.path().join("entry.json").exists());
        }

        #[test]
        fn write_json_preserves_existing_success_contract() {
            let temp = tempfile::tempdir().unwrap();
            let cache = Cache::default_for_repo(temp.path(), true);
            let key = key();
            let value = json!({ "schema": "go-facts-v1" });

            let status = cache.write_json_with_status(&key, &value).unwrap();
            cache.write_json(&key, &value).unwrap();
            let restored: serde_json::Value = cache.read_json(&key).unwrap().unwrap();

            assert_eq!(status, CacheWriteStatus::Written);
            assert_eq!(restored, value);
        }

        #[test]
        fn write_json_bytes_preserves_the_validated_representation() {
            let temp = tempfile::tempdir().unwrap();
            let cache = Cache::default_for_repo(temp.path(), true);
            let key = key();
            let bytes = br#"{"z":1,"a":[true,false]}"#;

            let status = cache.write_json_bytes_with_status(&key, bytes).unwrap();
            let outcome = cache.read_json_bytes_with_status(&key);

            assert_eq!(status, CacheWriteStatus::Written);
            assert_eq!(outcome.status, CacheReadStatus::Hit);
            assert_eq!(outcome.value.as_deref(), Some(bytes.as_slice()));
            assert_eq!(fs::read(cache.path_for(&key)).unwrap(), bytes);
        }

        #[test]
        fn write_json_bytes_rejects_invalid_json_when_enabled() {
            let temp = tempfile::tempdir().unwrap();
            let cache = Cache::default_for_repo(temp.path(), true);
            let key = key();

            let error = cache
                .write_json_bytes_with_status(&key, b"{not-json")
                .expect_err("invalid JSON should fail before the write");

            assert!(
                format!("{error:#}").contains("cache bytes must contain valid JSON"),
                "unexpected error: {error:#}"
            );
            assert!(!cache.path_for(&key).exists());
        }

        #[test]
        fn disabled_json_byte_write_bypasses_validation_and_io() {
            let temp = tempfile::tempdir().unwrap();
            let cache = Cache::default_for_repo(temp.path(), false);
            let key = key();

            let status = cache
                .write_json_bytes_with_status(&key, b"{not-json")
                .unwrap();

            assert_eq!(status, CacheWriteStatus::Disabled);
            assert!(!cache.root().exists());
            assert!(!temp.path().join(".polint/cache").exists());
        }

        #[cfg(unix)]
        #[test]
        fn write_json_rejects_symlink_cache_parent() {
            let temp = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), temp.path().join(".polint")).unwrap();
            let cache = Cache::default_for_repo(temp.path(), true);
            let key = key();

            let error = cache
                .write_json_with_status(&key, &json!({ "schema": "go-facts-v1" }))
                .expect_err("symlink cache parent should fail");

            assert!(format!("{error:#}").contains("path escapes repository root"));
            assert!(!outside.path().join("cache/analysis").exists());
        }
    }

    #[test]
    fn cache_status_counts_managed_categories() {
        let temp = tempfile::tempdir().unwrap();
        let layout = CacheLayout::from_root(temp.path().join("cache"));
        fs::create_dir_all(layout.analysis_dir()).unwrap();
        fs::write(layout.analysis_dir().join("a.json"), "{}").unwrap();
        fs::write(layout.root().join("incompatible.json"), "{}").unwrap();
        fs::create_dir_all(layout.rules_target_dir().join("debug")).unwrap();
        fs::write(layout.rules_target_dir().join("debug/rules"), "binary").unwrap();

        let status = layout.status().unwrap();

        assert_eq!(status.total_files, 2);
        assert_eq!(status.total_bytes, 8);
        assert!(
            status
                .categories
                .iter()
                .any(|category| category.name == "layers")
        );
    }

    #[test]
    fn cache_clean_analysis_removes_only_analysis_directory() {
        let temp = tempfile::tempdir().unwrap();
        let layout = CacheLayout::from_root(temp.path().join("cache"));
        fs::create_dir_all(layout.analysis_dir()).unwrap();
        fs::write(layout.analysis_dir().join("a.json"), "{}").unwrap();
        fs::write(layout.root().join("unmanaged.json"), "{}").unwrap();

        let report = layout
            .clean(CacheCleanSelection::Category(
                CacheManagedCategory::Analysis,
            ))
            .unwrap();

        assert_eq!(report.removed_files, 1);
        assert!(!layout.analysis_dir().exists());
        assert!(layout.root().join("unmanaged.json").exists());
    }

    #[test]
    fn cache_layout_manages_layer_cache_category() {
        let temp = tempfile::tempdir().unwrap();
        let layout = CacheLayout::from_root(temp.path().join("cache"));
        fs::create_dir_all(layout.layer_cache_dir()).unwrap();
        fs::write(layout.layer_cache_dir().join("manifest.json"), "{}").unwrap();

        let report = layout
            .clean(CacheCleanSelection::Category(CacheManagedCategory::Layers))
            .unwrap();

        assert_eq!(layout.layer_cache_dir(), layout.root().join("layers"));
        assert_eq!(CacheManagedCategory::Layers.name(), "layers");
        assert_eq!(report.removed_files, 1);
        assert!(!layout.layer_cache_dir().exists());
    }

    /// A cache directory named outside this module is a directory
    /// `CacheManagedCategory` does not know about, so `polint cache status`
    /// cannot report it, `polint cache clean` cannot remove it, and the GitHub
    /// Action cannot decide how to key it - it just quietly accumulates. That is
    /// how the extension host's target directory went unmanaged, and it also
    /// meant `POLINT_CACHE_DIR` did not move it.
    #[test]
    fn only_this_module_names_a_cache_subdirectory() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut pending = vec![src.clone()];
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(&dir).expect("read source directory") {
                let path = entry.expect("source entry").path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                if path.extension().and_then(|ext| ext.to_str()) != Some("rs")
                    || path == src.join("cache/mod.rs")
                {
                    continue;
                }
                let contents = fs::read_to_string(&path).expect("read source file");
                // Test modules may spell paths out; production code may not.
                let production = contents
                    .split_once("#[cfg(test)]")
                    .map_or(contents.as_str(), |(before, _)| before);
                if production.contains("\".polint/cache/") {
                    offenders.push(path.display().to_string());
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "cache subdirectories belong to CacheLayout, not to their callers: {offenders:?}"
        );
    }

    /// Every managed category's directory name, pinned against the cache root.
    ///
    /// `action.yml` names these directories literally, so a rename here is a
    /// silent cache loss there. `cache_layout_matches_the_github_action_contract`
    /// checks the other half of that contract.
    #[test]
    fn cache_layout_directory_names_are_stable() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        let layout = CacheLayout::from_root(&root);

        let relative = |path: PathBuf| {
            path.strip_prefix(&root)
                .expect("category stays under the cache root")
                .to_string_lossy()
                .replace('\\', "/")
        };

        assert_eq!(relative(layout.analysis_dir()), "analysis");
        assert_eq!(relative(layout.layer_cache_dir()), "layers");
        assert_eq!(relative(layout.derived_dir()), "derived");
        assert_eq!(relative(layout.semantic_store_dir()), "semantic-store");
        assert_eq!(
            relative(layout.extensions_target_dir()),
            "extensions-target"
        );
        assert_eq!(relative(layout.review_dir()), "review");
        assert_eq!(CacheManagedCategory::RulesTarget.name(), "rules-target");

        match std::env::var_os(POLINT_RULES_TARGET_DIR_ENV).filter(|value| !value.is_empty()) {
            // The action skips its build cache under this override precisely
            // because the target directory then leaves the cache root.
            Some(override_dir) => assert_eq!(
                layout.rules_target_dir(),
                absolutize_env_path(&root, PathBuf::from(override_dir))
            ),
            None => assert_eq!(relative(layout.rules_target_dir()), "rules-target"),
        }

        // Every managed category resolves under the cache root, so nothing can
        // be reported, cleaned, or pruned outside it.
        for category in CacheManagedCategory::ALL {
            let path = layout.category_path(*category);
            if *category == CacheManagedCategory::RulesTarget
                && std::env::var_os(POLINT_RULES_TARGET_DIR_ENV).is_some()
            {
                continue;
            }
            assert_eq!(
                relative(path),
                category.name(),
                "{} must live at its own name under the cache root",
                category.name()
            );
        }
    }

    /// `action.yml` is the published CI contract for this layout: the
    /// source-validated directories are restored and saved under a key scoped
    /// to the polint version and config/rule inputs, the compiler-output
    /// directories under a separate key built from build inputs, and scratch is
    /// not cached at all.
    ///
    /// This test walks the categories rather than a hardcoded list, so adding a
    /// cache directory fails here until the action says how to treat it.
    /// Getting it wrong the other way - compiler output under the analysis key -
    /// would cache a rule-host binary under a key that names no compiler.
    #[test]
    fn cache_layout_matches_the_github_action_contract() {
        let action_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../action.yml");
        let raw = fs::read_to_string(&action_path).expect("read action.yml");
        let action: serde_norway::Value =
            serde_norway::from_str(&raw).expect("action.yml parses as YAML");
        let steps = action["runs"]["steps"]
            .as_sequence()
            .expect("composite action has steps");

        let cached_paths = |step_name: &str| -> Vec<String> {
            let step = steps
                .iter()
                .find(|step| step["name"].as_str() == Some(step_name))
                .unwrap_or_else(|| panic!("action.yml has no step named {step_name:?}"));
            step["with"]["path"]
                .as_str()
                .unwrap_or_else(|| panic!("step {step_name:?} caches no path"))
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect()
        };

        let analysis_paths = cached_paths("Restore polint analysis cache");
        let build_paths = cached_paths("Restore polint rule-host build cache");
        assert_eq!(analysis_paths, cached_paths("Save polint analysis cache"));
        assert_eq!(
            build_paths,
            cached_paths("Save polint rule-host build cache")
        );

        let entry_for =
            |name: &str| format!("${{{{ inputs.working-directory }}}}/.polint/cache/{name}");
        for category in CacheManagedCategory::ALL {
            let entry = entry_for(category.name());
            let in_analysis = analysis_paths.contains(&entry);
            let in_build = build_paths.contains(&entry);
            match category.role() {
                CacheDirRole::SourceValidated => assert!(
                    in_analysis && !in_build,
                    "{} holds source-validated artifacts and belongs only in the analysis entry",
                    category.name()
                ),
                CacheDirRole::CompilerOutput => assert!(
                    in_build && !in_analysis,
                    "{} holds compiler output and belongs only in the build entry",
                    category.name()
                ),
                CacheDirRole::Scratch => assert!(
                    !in_analysis && !in_build,
                    "{} is per-run scratch and must not be cached",
                    category.name()
                ),
            }
        }

        // And nothing is cached that no category accounts for.
        for path in analysis_paths.iter().chain(build_paths.iter()) {
            assert!(
                CacheManagedCategory::ALL
                    .iter()
                    .any(|category| *path == entry_for(category.name())),
                "action.yml caches {path}, which is not a managed cache category"
            );
        }
    }

    proptest! {
        #[test]
        fn cache_key_for_file_path_participates_in_stable_id_proptest(
            left in "[a-z]{1,8}/[a-z]{1,8}\\.go",
            right in "[a-z]{1,8}/[a-z]{1,8}\\.go",
        ) {
            prop_assume!(left != right);
            let left_key = CacheKey::for_file(&left, "same-content", "config", "rule", "plan", "go-facts-v1", "tree-sitter-0.26.8+tree-sitter-go-0.25.0");
            let right_key = CacheKey::for_file(&right, "same-content", "config", "rule", "plan", "go-facts-v1", "tree-sitter-0.26.8+tree-sitter-go-0.25.0");

            prop_assert_ne!(left_key.stable_id(), right_key.stable_id());
        }
    }
}
