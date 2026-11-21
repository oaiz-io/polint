//! The configs a `tsconfig.json` or `jsconfig.json` reaches through its
//! `extends` (and, where a caller follows them, its `references`).
//!
//! Every walk over these configs is iterative and stops after
//! [`MAX_TSCONFIG_CHAIN_CONFIGS`] configs, so a repository can make a walk
//! neither exhaust the stack nor run without end. A config that reaches more
//! configs than that is not one any resolver here loads: import resolution
//! reports its imports unknown, and the cache keys of the walk still name
//! every config the bound read.

use crate::repo_fs::{
    TOPOLOGY_MANIFEST_MAX_BYTES, normalize_repo_relative, normalize_repo_relative_input,
    read_repo_file_to_string_with_limit, repo_file_path, repo_relative_existing_path,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

/// The most configs one walk visits, the starting config included.
pub(crate) const MAX_TSCONFIG_CHAIN_CONFIGS: usize = 256;

/// The configs a walk reached, breadth first from its start.
pub(crate) struct TsconfigChain<N> {
    /// The configs in the order the walk reached them, the start first.
    pub(crate) configs: Vec<N>,
    /// The start reaches more than [`MAX_TSCONFIG_CHAIN_CONFIGS`] configs;
    /// `configs` holds the first one beyond the bound too.
    pub(crate) exceeded: bool,
}

/// Walks breadth first from `start`, asking `next` once for the configs each
/// reached config names. Each config is visited once, so a cycle ends the
/// walk, and the walk stops once it reaches one config more than the bound.
pub(crate) fn walk_tsconfig_chain<N: Ord + Clone>(
    start: N,
    mut next: impl FnMut(&N) -> Vec<N>,
) -> TsconfigChain<N> {
    let mut seen = BTreeSet::from([start.clone()]);
    let mut configs = vec![start.clone()];
    let mut queue = VecDeque::from([start]);
    while let Some(config) = queue.pop_front() {
        for named in next(&config) {
            if !seen.insert(named.clone()) {
                continue;
            }
            configs.push(named.clone());
            if configs.len() > MAX_TSCONFIG_CHAIN_CONFIGS {
                return TsconfigChain {
                    configs,
                    exceeded: true,
                };
            }
            queue.push_back(named);
        }
    }
    TsconfigChain {
        configs,
        exceeded: false,
    }
}

/// The configs that the `tsconfig.json` and `jsconfig.json` files among
/// `initial_candidates` extend, as repository-relative paths.
pub(crate) fn extended_ts_config_candidates<'a>(
    root: &Path,
    initial_candidates: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut discovered = BTreeSet::new();
    for candidate in initial_candidates {
        let file_name = Path::new(candidate)
            .file_name()
            .and_then(|name| name.to_str());
        if !matches!(file_name, Some("tsconfig.json" | "jsconfig.json")) {
            continue;
        }
        let Some(start) = repo_relative_existing_path(root, &root.join(candidate)) else {
            continue;
        };
        let chain = walk_tsconfig_chain(start, |config| {
            read_tsconfig_json::<TsconfigExtendsOnlyWire>(root, config)
                .map(|wire| extended_configs(root, config, wire.extends))
                .unwrap_or_default()
        });
        discovered.extend(chain.configs.into_iter().skip(1));
    }
    discovered.into_iter().collect()
}

/// The configs that the config at `relative_path` names in `extends`.
pub(crate) fn extended_configs(
    root: &Path,
    relative_path: &str,
    extends: Option<TsconfigExtendsWire>,
) -> Vec<String> {
    let config_dir = Path::new(relative_path).parent().unwrap_or(Path::new(""));
    extends
        .into_iter()
        .flat_map(TsconfigExtendsWire::into_specifiers)
        .filter_map(|specifier| resolve_tsconfig_extends_path(root, config_dir, &specifier))
        .collect()
}

/// The config at `relative_path` read as JSON with comments, or nothing where
/// it is missing, too large or not of the shape `T`.
pub(crate) fn read_tsconfig_json<T: DeserializeOwned>(
    root: &Path,
    relative_path: &str,
) -> Option<T> {
    let source =
        read_repo_file_to_string_with_limit(root, relative_path, TOPOLOGY_MANIFEST_MAX_BYTES)
            .ok()?;
    parse_tsconfig_json(&source)
}

/// `source` read as JSON with comments, or nothing where it is not of the
/// shape `T`.
pub(crate) fn parse_tsconfig_json<T: DeserializeOwned>(source: &str) -> Option<T> {
    let mut source = source
        .strip_prefix('\u{feff}')
        .unwrap_or(source)
        .to_string();
    json_strip_comments::strip(&mut source).ok()?;
    serde_json::from_str(&source).ok()
}

fn resolve_tsconfig_extends_path(
    root: &Path,
    config_dir: &Path,
    specifier: &str,
) -> Option<String> {
    let specifier_path = Path::new(specifier);
    if specifier_path.is_absolute() {
        return None;
    }
    if specifier.starts_with('.') {
        return resolve_tsconfig_file_candidate(root, &config_dir.join(specifier_path));
    }
    resolve_package_tsconfig_extends_path(root, config_dir, specifier)
}

fn resolve_package_tsconfig_extends_path(
    root: &Path,
    config_dir: &Path,
    specifier: &str,
) -> Option<String> {
    let mut current = normalize_repo_relative_input(config_dir)?;
    loop {
        let candidate = if current.as_os_str().is_empty() {
            PathBuf::from("node_modules").join(specifier)
        } else {
            current.join("node_modules").join(specifier)
        };
        if let Some(resolved) = resolve_tsconfig_file_candidate(root, &candidate) {
            return Some(resolved);
        }
        if current.as_os_str().is_empty() {
            return None;
        }
        current.pop();
    }
}

/// The config file `base` names: the file itself, the file with `.json`
/// added, or the `tsconfig.json` of the directory.
pub(crate) fn resolve_tsconfig_file_candidate(root: &Path, base: &Path) -> Option<String> {
    let mut candidates = vec![base.to_path_buf()];
    if base.extension().and_then(|extension| extension.to_str()) != Some("json") {
        let mut with_json = base.as_os_str().to_owned();
        with_json.push(".json");
        candidates.push(PathBuf::from(with_json));
    }
    candidates.push(base.join("tsconfig.json"));

    candidates
        .into_iter()
        .filter_map(|candidate| normalized_existing_repo_file(root, &candidate))
        .next()
}

fn normalized_existing_repo_file(root: &Path, candidate: &Path) -> Option<String> {
    repo_file_path(root, candidate).ok()?;
    let normalized = normalize_repo_relative_input(candidate)?;
    let relative = normalized.to_string_lossy();
    if relative.is_empty() {
        None
    } else {
        normalize_repo_relative(relative)
    }
}

#[derive(Debug, Deserialize)]
struct TsconfigExtendsOnlyWire {
    #[serde(default)]
    extends: Option<TsconfigExtendsWire>,
}

/// The `extends` of a config: one specifier or a list of them.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum TsconfigExtendsWire {
    Single(String),
    Multiple(Vec<String>),
}

impl TsconfigExtendsWire {
    pub(crate) fn into_specifiers(self) -> Vec<String> {
        match self {
            Self::Single(specifier) => vec![specifier],
            Self::Multiple(specifiers) => specifiers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_walk_stops_one_config_beyond_the_bound() {
        let chain = walk_tsconfig_chain(0_usize, |&config| vec![config + 1]);
        assert!(chain.exceeded);
        assert_eq!(chain.configs.len(), MAX_TSCONFIG_CHAIN_CONFIGS + 1);
        let short = walk_tsconfig_chain(0_usize, |&config| {
            if config < 10 {
                vec![config + 1]
            } else {
                Vec::new()
            }
        });
        assert!(!short.exceeded);
        assert_eq!(short.configs, (0..=10).collect::<Vec<_>>());
    }

    #[test]
    fn a_walk_visits_each_config_of_a_cycle_once() {
        let mut asked = Vec::new();
        let chain = walk_tsconfig_chain(0_usize, |&config| {
            asked.push(config);
            vec![(config + 1) % 3]
        });
        assert!(!chain.exceeded);
        assert_eq!(asked, vec![0, 1, 2]);
    }
}
