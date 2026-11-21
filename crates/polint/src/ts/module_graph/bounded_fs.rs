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

use crate::repo_fs::TOPOLOGY_MANIFEST_MAX_BYTES;
use crate::ts::tsconfig_chain::{MAX_TSCONFIG_CHAIN_CONFIGS, walk_tsconfig_chain};
use oxc_resolver::{
    ExtendsField, FileMetadata, FileSystem, FileSystemOs, PathUtil, ResolveError, ResolveOptions,
    ResolverGeneric, TsConfig,
};
use std::collections::HashMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

/// What a read gives in place of a config the resolver must not load: not
/// JSON, so loading it fails.
const UNLOADABLE: &str = "{";

pub(crate) struct BoundedFs {
    os: FileSystemOs,
    /// Whether a config is checked against the chain bound before it is
    /// read. The resolver that finds the config a package `extends` names
    /// loads no configs and checks none.
    checks_configs: bool,
    /// Finds the config a package `extends` names, as the resolver does.
    extends_resolver: OnceLock<Box<ResolverGeneric<BoundedFs>>>,
    /// The config files each config extends, by the config's path.
    extended: Mutex<HashMap<PathBuf, Vec<PathBuf>>>,
    /// Whether the configs each config reaches are within the bound.
    within_bound: Mutex<HashMap<PathBuf, bool>>,
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

    /// The bytes of `path`, or `None` where the file is larger than `max`.
    fn read_limited(path: &Path, max: u64) -> io::Result<Option<Vec<u8>>> {
        if std::fs::metadata(path)?.len() > max {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(max.saturating_add(1))
            .read_to_end(&mut bytes)?;
        Ok((bytes.len() as u64 <= max).then_some(bytes))
    }

    /// Whether the configs `config` reaches through `extends` are within the
    /// bound.
    fn chain_within_bound(&self, config: &Path) -> bool {
        if let Some(&known) = lock(&self.within_bound).get(config) {
            return known;
        }
        let chain = walk_tsconfig_chain(config.to_path_buf(), |config| self.extended_files(config));
        let within = !chain.exceeded;
        lock(&self.within_bound).insert(config.to_path_buf(), within);
        within
    }

    /// The config files `config` extends, found as the resolver finds them.
    fn extended_files(&self, config: &Path) -> Vec<PathBuf> {
        if let Some(known) = lock(&self.extended).get(config) {
            return known.clone();
        }
        let files = self.read_extended_files(config);
        lock(&self.extended).insert(config.to_path_buf(), files.clone());
        files
    }

    fn read_extended_files(&self, config: &Path) -> Vec<PathBuf> {
        let Ok(Some(bytes)) = Self::read_limited(config, TOPOLOGY_MANIFEST_MAX_BYTES) else {
            return Vec::new();
        };
        let Ok(source) = FileSystemOs::validate_string(bytes) else {
            return Vec::new();
        };
        let Ok(tsconfig) = TsConfig::parse(false, config, source) else {
            return Vec::new();
        };
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
            .filter_map(|specifier| match specifier.as_bytes().first()? {
                b'/' => Some(PathBuf::from(specifier)),
                b'.' => Some(directory.normalize_with(specifier)),
                _ => self
                    .extends_resolver()
                    .resolve(directory, specifier)
                    .ok()
                    .map(oxc_resolver::Resolution::into_path_buf),
            })
            .map(|path| self.config_file(path))
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

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.os.read(path)
    }

    /// The resolver reads only configs as text.
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        let Some(bytes) = Self::read_limited(path, TOPOLOGY_MANIFEST_MAX_BYTES)? else {
            return Ok(UNLOADABLE.to_string());
        };
        if self.checks_configs && !self.chain_within_bound(path) {
            return Ok(UNLOADABLE.to_string());
        }
        FileSystemOs::validate_string(bytes)
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
}
