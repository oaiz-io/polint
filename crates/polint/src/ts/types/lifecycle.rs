use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use toml::Value;

use crate::analysis_api::FactDatabase;
use crate::internal_core::Language;

/// Share of `any`/`unknown` receivers above which a file's typed answers are
/// reported as degraded rather than exact.
///
/// A quarter of a file's receivers being untyped says the file's type
/// information is partial, so the answers that remain are still used but no
/// longer claim full confidence.
pub(crate) const ANY_DENSITY_DEGRADED_PERCENT: u64 = 25;

/// Share of `any`/`unknown` receivers above which a file's inexact sites are
/// left to the field and heap tiers.
///
/// At half untyped, a typed candidate for a site the checker could not resolve
/// exactly is a guess dressed as a type answer. Exact sites in the same file
/// are still emitted: the gate is about which answers to trust, not about
/// discarding the file.
pub(crate) const ANY_DENSITY_DEFER_PERCENT: u64 = 50;

/// Resolved TypeScript type-analysis lifecycle for one scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TsTypesConfig {
    /// Whether the tier runs at all. `[languages.ts] type_sidecar = false`
    /// turns it off and leaves the heap tier alone.
    pub(crate) enabled: bool,
    /// Repo-relative tsconfig paths, sorted and deduplicated.
    pub(crate) projects: Vec<String>,
    /// Explicit TypeScript module directory from configuration.
    pub(crate) typescript_path: Option<String>,
    /// Wall-clock budget override in milliseconds.
    pub(crate) timeout_ms: Option<u64>,
    /// Repo-relative TS/JS files this scan discovered, sorted.
    pub(crate) scope_files: Vec<String>,
    /// Discovered TS/JS files with no tsconfig above them, sorted. Reported so
    /// a partially covered repository is visible rather than silently thinner.
    pub(crate) files_without_project: Vec<String>,
    /// Whether `.polint.toml` names this tier at all.
    ///
    /// A repository that never asked for type-directed analysis and has no
    /// TypeScript compiler is not misconfigured — it is a repository the tier
    /// does not apply to — so its scans stay quiet. Naming any of the tier's
    /// settings turns a setup gap into a reported failure instead.
    pub(crate) explicitly_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TsTypesLifecycleError {
    InvalidSetting(String),
}

impl TsTypesLifecycleError {
    pub(crate) fn reason(&self) -> &str {
        match self {
            Self::InvalidSetting(reason) => reason,
        }
    }
}

impl std::fmt::Display for TsTypesLifecycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.reason())
    }
}

/// Repo-relative paths of the TS/JS files this scan discovered.
pub(crate) fn ts_files(db: &dyn FactDatabase) -> Vec<String> {
    let mut files = db
        .files()
        .iter()
        .filter(|file| file.language.is_ts_family())
        .map(|file| file.relative_path.clone())
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    files
}

impl TsTypesConfig {
    pub(crate) fn from_settings(
        root: &Path,
        settings: &BTreeMap<String, Value>,
        db: &dyn FactDatabase,
    ) -> Result<Self, TsTypesLifecycleError> {
        Self::from_settings_files(root, settings, &ts_files(db))
    }

    pub(crate) fn from_settings_files(
        root: &Path,
        settings: &BTreeMap<String, Value>,
        files: &[String],
    ) -> Result<Self, TsTypesLifecycleError> {
        let enabled = match settings.get("type_sidecar") {
            None => true,
            Some(Value::Boolean(enabled)) => *enabled,
            Some(_) => {
                return Err(TsTypesLifecycleError::InvalidSetting(
                    "`[languages.ts] type_sidecar` must be a boolean.".to_string(),
                ));
            }
        };
        let configured_projects = string_list(settings, "type_projects")?;
        let typescript_path = optional_string(settings, "typescript_path")?;
        let timeout_ms = optional_integer(settings, "type_timeout_ms")?;

        let mut scope_files = files.to_vec();
        scope_files.sort();
        scope_files.dedup();

        let (projects, files_without_project) = if configured_projects.is_empty() {
            discover_projects(root, &scope_files)
        } else {
            (configured_projects, Vec::new())
        };

        Ok(Self {
            enabled,
            projects,
            typescript_path,
            timeout_ms,
            scope_files,
            files_without_project,
            explicitly_requested: ["type_sidecar", "type_projects", "typescript_path"]
                .iter()
                .any(|key| settings.contains_key(*key)),
        })
    }

    /// Absolute paths of configured projects that do not exist on disk.
    pub(crate) fn missing_projects(&self, root: &Path) -> Vec<String> {
        self.projects
            .iter()
            .filter(|project| !root.join(project).is_file())
            .cloned()
            .collect()
    }
}

/// Nearest-`tsconfig.json` discovery for every discovered TS/JS file.
///
/// Topology stays where the module graph already keeps it: this walks from a
/// file to the first `tsconfig.json` at or above it, exactly as import
/// resolution does, rather than adding a second notion of where a project
/// starts.
fn discover_projects(root: &Path, files: &[String]) -> (Vec<String>, Vec<String>) {
    let mut projects = BTreeSet::new();
    let mut orphans = Vec::new();
    // One lookup per directory: a project with a thousand files must not cost
    // a thousand directory walks.
    let mut by_directory: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    for relative in files {
        let absolute = root.join(relative);
        let Some(directory) = absolute.parent().map(Path::to_path_buf) else {
            orphans.push(relative.clone());
            continue;
        };
        let resolved = by_directory
            .entry(directory)
            .or_insert_with(|| {
                crate::ts::module_graph::nearest_tsconfig_path(root, &absolute)
                    .and_then(|config| repo_relative(root, &config))
            })
            .clone();
        match resolved {
            Some(project) => {
                projects.insert(project);
            }
            None => orphans.push(relative.clone()),
        }
    }
    orphans.sort();
    orphans.dedup();
    (projects.into_iter().collect(), orphans)
}

fn repo_relative(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let text = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    (!text.is_empty()).then_some(text)
}

fn string_list(
    settings: &BTreeMap<String, Value>,
    key: &str,
) -> Result<Vec<String>, TsTypesLifecycleError> {
    let Some(value) = settings.get(key) else {
        return Ok(Vec::new());
    };
    let Value::Array(items) = value else {
        return Err(TsTypesLifecycleError::InvalidSetting(format!(
            "`[languages.ts] {key}` must be an array of strings."
        )));
    };
    let mut values = Vec::new();
    for item in items {
        let Value::String(text) = item else {
            return Err(TsTypesLifecycleError::InvalidSetting(format!(
                "`[languages.ts] {key}` must be an array of strings."
            )));
        };
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            values.push(trimmed.to_string());
        }
    }
    values.sort();
    values.dedup();
    Ok(values)
}

fn optional_string(
    settings: &BTreeMap<String, Value>,
    key: &str,
) -> Result<Option<String>, TsTypesLifecycleError> {
    match settings.get(key) {
        None => Ok(None),
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(Some(text.trim().to_string())),
        Some(Value::String(_)) => Ok(None),
        Some(_) => Err(TsTypesLifecycleError::InvalidSetting(format!(
            "`[languages.ts] {key}` must be a string."
        ))),
    }
}

fn optional_integer(
    settings: &BTreeMap<String, Value>,
    key: &str,
) -> Result<Option<u64>, TsTypesLifecycleError> {
    match settings.get(key) {
        None => Ok(None),
        Some(Value::Integer(value)) if *value > 0 => Ok(Some(*value as u64)),
        Some(_) => Err(TsTypesLifecycleError::InvalidSetting(format!(
            "`[languages.ts] {key}` must be a positive integer."
        ))),
    }
}

/// True when the language is TypeScript or JavaScript.
pub(crate) fn is_ts_family(language: Language) -> bool {
    language.is_ts_family()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn the_tier_is_on_by_default_and_can_be_turned_off() {
        let root = Path::new("/repo");
        let on = TsTypesConfig::from_settings_files(root, &BTreeMap::new(), &[]).expect("defaults");
        assert!(on.enabled);

        let off = TsTypesConfig::from_settings_files(
            root,
            &settings(&[("type_sidecar", Value::Boolean(false))]),
            &[],
        )
        .expect("explicit off");
        assert!(!off.enabled);
    }

    #[test]
    fn a_non_boolean_enable_flag_is_a_setup_error_not_a_silent_default() {
        let error = TsTypesConfig::from_settings_files(
            Path::new("/repo"),
            &settings(&[("type_sidecar", Value::String("yes".to_string()))]),
            &[],
        )
        .expect_err("string is rejected");
        assert!(error.reason().contains("type_sidecar"));
    }

    #[test]
    fn configured_projects_are_sorted_deduplicated_and_skip_discovery() {
        let config = TsTypesConfig::from_settings_files(
            Path::new("/repo"),
            &settings(&[(
                "type_projects",
                Value::Array(vec![
                    Value::String("packages/b/tsconfig.json".to_string()),
                    Value::String("packages/a/tsconfig.json".to_string()),
                    Value::String("packages/a/tsconfig.json".to_string()),
                ]),
            )]),
            &["packages/a/src/index.ts".to_string()],
        )
        .expect("configured projects");

        assert_eq!(
            config.projects,
            vec![
                "packages/a/tsconfig.json".to_string(),
                "packages/b/tsconfig.json".to_string()
            ]
        );
        assert!(config.files_without_project.is_empty());
    }

    #[test]
    fn discovery_walks_to_the_nearest_tsconfig_and_reports_uncovered_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        std::fs::create_dir_all(root.join("packages/web/src")).expect("create web");
        std::fs::create_dir_all(root.join("loose")).expect("create loose");
        std::fs::write(root.join("packages/web/tsconfig.json"), "{}").expect("write tsconfig");
        std::fs::write(root.join("packages/web/src/app.ts"), "export {};").expect("write app");
        std::fs::write(root.join("loose/other.ts"), "export {};").expect("write other");

        let config = TsTypesConfig::from_settings_files(
            root,
            &BTreeMap::new(),
            &[
                "packages/web/src/app.ts".to_string(),
                "loose/other.ts".to_string(),
            ],
        )
        .expect("discovery");

        assert_eq!(
            config.projects,
            vec!["packages/web/tsconfig.json".to_string()]
        );
        assert_eq!(
            config.files_without_project,
            vec!["loose/other.ts".to_string()]
        );
    }

    #[test]
    fn missing_projects_are_listed_so_setup_gaps_are_explicit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config = TsTypesConfig::from_settings_files(
            temp.path(),
            &settings(&[(
                "type_projects",
                Value::Array(vec![Value::String("tsconfig.json".to_string())]),
            )]),
            &[],
        )
        .expect("configured project");

        assert_eq!(
            config.missing_projects(temp.path()),
            vec!["tsconfig.json".to_string()]
        );
    }

    #[test]
    fn a_repository_that_never_names_the_tier_is_not_treated_as_configured() {
        let quiet = TsTypesConfig::from_settings_files(Path::new("/repo"), &BTreeMap::new(), &[])
            .expect("defaults");
        assert!(!quiet.explicitly_requested);

        let asked = TsTypesConfig::from_settings_files(
            Path::new("/repo"),
            &settings(&[("type_sidecar", Value::Boolean(true))]),
            &[],
        )
        .expect("explicit request");
        assert!(asked.explicitly_requested);
    }

    #[test]
    fn a_zero_timeout_is_rejected_rather_than_meaning_no_budget() {
        let error = TsTypesConfig::from_settings_files(
            Path::new("/repo"),
            &settings(&[("type_timeout_ms", Value::Integer(0))]),
            &[],
        )
        .expect_err("zero is rejected");
        assert!(error.reason().contains("type_timeout_ms"));
    }
}
