//! The file system the import resolvers read through.
//!
//! The resolver loads a `tsconfig.json` by recursing over its `extends`, one
//! stack frame or more per config, so a long chain in a repository or a
//! package would exhaust the stack. Before the resolver reads a config, this
//! file system walks the configs that config extends, iteratively and the way
//! the resolver resolves them, and gives the resolver a config it cannot
//! parse where the walk reaches more than [`MAX_TSCONFIG_CHAIN_CONFIGS`]
//! configs, or where the config is larger than a manifest may be. The
//! resolver reports that as a config error, so every import under that
//! config is reported unknown. A read error would not do: the resolver skips
//! an unreadable `tsconfig.json` and goes on to the parent directory's.
//!
//! The resolver parses a `package.json` with a parser that recurses once per
//! level of nesting and sets no limit of its own, and walks an `exports` or
//! `imports` map the same way. A manifest larger than a manifest may be, or
//! nested deeper than [`MAX_JSON_DEPTH`], reads as a manifest the resolver
//! cannot parse, so every import through that package is reported unknown
//! rather than resolved as if the package had no manifest.

use crate::bounded_json::{BoundedJsonError, MAX_JSON_DEPTH, check_bounds};
use crate::repo_fs::TOPOLOGY_MANIFEST_MAX_BYTES;
use crate::ts::tsconfig_chain::{MAX_TSCONFIG_CHAIN_CONFIGS, walk_tsconfig_chain};
use oxc_resolver::{
    ExtendsField, FileMetadata, FileSystem, FileSystemOs, PathUtil, ResolveError, ResolveOptions,
    ResolverGeneric, TsConfig,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

/// What a read gives in place of a config or a manifest the resolver must not
/// load: not JSON, so loading it fails.
const UNLOADABLE: &str = "{";

#[derive(Clone)]
pub(super) struct RefusedInput {
    pub(super) path: PathBuf,
    pub(super) reason: String,
    pub(super) malformed_config: bool,
}

thread_local! {
    static REFUSAL: RefCell<Option<RefusedInput>> = const { RefCell::new(None) };
}

impl RefusedInput {
    pub(super) fn new(path: &Path, reason: impl ToString) -> Self {
        Self {
            path: path.to_path_buf(),
            reason: reason.to_string(),
            malformed_config: false,
        }
    }

    pub(super) fn record(self) {
        REFUSAL.with(|slot| {
            slot.borrow_mut().get_or_insert(self);
        });
    }

    pub(super) fn capture<T>(resolve: impl FnOnce() -> T) -> (T, Option<Self>) {
        let previous = REFUSAL.take();
        let result = resolve();
        (result, REFUSAL.replace(previous))
    }
}

type ConfigChildren = Result<Vec<PathBuf>, RefusedInput>;

pub(crate) struct BoundedFs {
    os: FileSystemOs,
    /// Whether a config is checked against the chain bound before it is
    /// read. The resolver that finds the config a package `extends` names
    /// loads no configs and checks none.
    checks_configs: bool,
    /// Finds the config a package `extends` names, as the resolver does.
    extends_resolver: OnceLock<Box<ResolverGeneric<BoundedFs>>>,
    /// The config files each config extends, by the config's path.
    extended: Mutex<HashMap<PathBuf, ConfigChildren>>,
    /// Whether the configs each config reaches are within the bound.
    within_bound: Mutex<HashMap<PathBuf, Result<(), RefusedInput>>>,
}

impl BoundedFs {
    fn with_checks(checks_configs: bool) -> Self {
        Self {
            os: FileSystemOs::new(),
            checks_configs,
            extends_resolver: OnceLock::new(),
            extended: Mutex::new(HashMap::new()),
            within_bound: Mutex::new(HashMap::new()),
        }
    }

    /// The bytes of `path`, or `None` where the file is larger than `max` or
    /// nests deeper than [`MAX_JSON_DEPTH`].
    pub(super) fn read_limited(path: &Path, max: u64) -> Result<Option<Vec<u8>>, RefusedInput> {
        let io_error = |error| RefusedInput::new(path, error);
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(error)),
        };
        if metadata.len() > max {
            return Err(RefusedInput::new(path, BoundedJsonError::TooLarge { max }));
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(io_error)?
            .take(max.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        check_bounds(&bytes, max).map_err(|error| RefusedInput::new(path, error))?;
        Ok(Some(bytes))
    }

    /// Whether the configs `config` reaches through `extends` are within the
    /// bound.
    pub(super) fn check_config(&self, config: &Path) -> Result<(), RefusedInput> {
        if let Some(known) = lock(&self.within_bound).get(config) {
            return known.clone();
        }
        let mut refusal = None;
        let chain = walk_tsconfig_chain(config.to_path_buf(), |config| {
            if refusal.is_some() {
                return Vec::new();
            }
            match self.extended_files(config) {
                Ok(files) => files,
                Err(error) => {
                    refusal = Some(error);
                    Vec::new()
                }
            }
        });
        let within = if let Some(refusal) = refusal {
            Err(refusal)
        } else if chain.exceeded {
            Err(RefusedInput::new(
                config,
                format!("extends more than {MAX_TSCONFIG_CHAIN_CONFIGS} configs"),
            ))
        } else {
            Ok(())
        };
        lock(&self.within_bound).insert(config.to_path_buf(), within.clone());
        within
    }

    /// The config files `config` extends, found as the resolver finds them.
    fn extended_files(&self, config: &Path) -> ConfigChildren {
        if let Some(known) = lock(&self.extended).get(config) {
            return known.clone();
        }
        let files = self.read_extended_files(config);
        lock(&self.extended).insert(config.to_path_buf(), files.clone());
        files
    }

    fn read_extended_files(&self, config: &Path) -> ConfigChildren {
        let Some(bytes) = Self::read_limited(config, TOPOLOGY_MANIFEST_MAX_BYTES)? else {
            return Err(RefusedInput::new(config, "config not found"));
        };
        let source = FileSystemOs::validate_string(bytes)
            .map_err(|error| RefusedInput::new(config, error))?;
        let tsconfig = TsConfig::parse(false, config, source).map_err(|error| RefusedInput {
            malformed_config: true,
            ..RefusedInput::new(config, error)
        })?;
        let specifiers = match &tsconfig.extends {
            Some(ExtendsField::Single(specifier)) => vec![specifier.as_str()],
            Some(ExtendsField::Multiple(specifiers)) => {
                specifiers.iter().map(String::as_str).collect()
            }
            None => Vec::new(),
        };
        let directory = tsconfig.directory();
        specifiers
            .into_iter()
            .map(|specifier| {
                let path = match specifier.as_bytes().first() {
                    Some(b'/') => PathBuf::from(specifier),
                    Some(b'.') => directory.normalize_with(specifier),
                    _ => {
                        let (resolution, refusal) = RefusedInput::capture(|| {
                            self.extends_resolver().resolve(directory, specifier)
                        });
                        if let Some(refusal) = refusal {
                            return Err(refusal);
                        }
                        resolution
                            .map_err(|error| RefusedInput::new(config, error))?
                            .into_path_buf()
                    }
                };
                Ok(self.config_file(path))
            })
            .collect()
    }

    /// The file the resolver reads for the config `path` names: the file
    /// itself, a directory's `tsconfig.json`, or the path with `.json` added.
    fn config_file(&self, path: PathBuf) -> PathBuf {
        match self.os.metadata(&path) {
            Ok(metadata) if metadata.is_file() => path,
            Ok(metadata) if metadata.is_dir() => path.join("tsconfig.json"),
            _ => {
                let mut with_json = path.into_os_string();
                with_json.push(".json");
                PathBuf::from(with_json)
            }
        }
    }

    fn extends_resolver(&self) -> &ResolverGeneric<BoundedFs> {
        self.extends_resolver.get_or_init(|| {
            Box::new(ResolverGeneric::new_with_file_system(
                Self::with_checks(false),
                ResolveOptions {
                    tsconfig: None,
                    condition_names: vec!["node".into(), "import".into()],
                    extensions: vec![".json".into()],
                    main_files: vec!["tsconfig".into()],
                    // The resolver keeps the path it found a config at, so
                    // the walk must too: a path through a symbolic link is a
                    // config of its own to the resolver.
                    symlinks: false,
                    ..ResolveOptions::default()
                },
            ))
        })
    }
}

impl FileSystem for BoundedFs {
    fn new() -> Self {
        Self::with_checks(true)
    }

    /// The resolver reads only manifests as bytes.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match Self::read_limited(path, TOPOLOGY_MANIFEST_MAX_BYTES) {
            Ok(Some(bytes)) => Ok(bytes),
            Ok(None) => Err(io::ErrorKind::NotFound.into()),
            Err(refusal) => {
                refusal.record();
                Ok(UNLOADABLE.as_bytes().to_vec())
            }
        }
    }

    /// The resolver reads only configs as text.
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        if self.checks_configs
            && let Err(refusal) = self.check_config(path)
        {
            refusal.record();
            return Ok(UNLOADABLE.into());
        }
        match Self::read_limited(path, TOPOLOGY_MANIFEST_MAX_BYTES) {
            Ok(Some(bytes)) => FileSystemOs::validate_string(bytes),
            Ok(None) => Err(io::ErrorKind::NotFound.into()),
            Err(refusal) => {
                refusal.record();
                Ok(UNLOADABLE.into())
            }
        }
    }

    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.os.metadata(path)
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.os.symlink_metadata(path)
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        self.os.read_link(path)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.os.canonicalize(path)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_past_the_chain_bound_reads_as_unloadable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let links = MAX_TSCONFIG_CHAIN_CONFIGS + 4;
        for link in 0..links {
            std::fs::write(
                dir.path().join(format!("c{link}.json")),
                format!(r#"{{"extends":"./c{}.json"}}"#, link + 1),
            )
            .expect("write config");
        }
        std::fs::write(dir.path().join(format!("c{links}.json")), "{}").expect("write config");
        let fs = BoundedFs::new();
        assert_eq!(
            fs.read_to_string(&dir.path().join("c0.json"))
                .expect("read"),
            UNLOADABLE
        );
        let near_end = dir.path().join(format!("c{}.json", links - 8));
        assert!(
            fs.read_to_string(&near_end)
                .expect("read")
                .contains("extends")
        );
    }

    #[test]
    fn a_manifest_past_the_depth_bound_reads_as_unloadable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let levels = MAX_JSON_DEPTH + 1;
        let deep = format!(
            r#"{{"exports":{}"./f.mjs"{}}}"#,
            r#"{"default":"#.repeat(levels - 1),
            "}".repeat(levels - 1)
        );
        let manifest = dir.path().join("package.json");
        std::fs::write(&manifest, &deep).expect("write manifest");
        let fs = BoundedFs::new();
        assert_eq!(fs.read(&manifest).expect("read"), UNLOADABLE.as_bytes());
        let shallow = r#"{"exports":{"default":"./f.mjs"}}"#;
        std::fs::write(&manifest, shallow).expect("write manifest");
        assert_eq!(fs.read(&manifest).expect("read"), shallow.as_bytes());
    }
}
