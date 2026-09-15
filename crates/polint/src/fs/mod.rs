use crate::config::LoadedConfig;
use crate::core::{AnalysisDb, Language};
use crate::diagnostics::{Diagnostic, TextRange};
use crate::path_context::PathContextIndex;
use crate::repo_fs::{self, RepoFileReadError};
use anyhow::{Result, anyhow};
use globset::GlobSet;
use ignore::{WalkBuilder, WalkState};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use thiserror::Error;

/// Hard ceiling for a single source file loaded into the analysis DB.
///
/// Oversized files are skipped with a `polint/capability` diagnostic instead of
/// being read whole. Matches other bounded repo reads (cache / baseline / lockfiles).
pub(crate) const SOURCE_FILE_MAX_BYTES: u64 = 16 * 1_048_576;

#[derive(Debug, Error)]
pub(crate) enum FsError {
    #[error("failed to strip root prefix for {path}")]
    StripPrefix { path: PathBuf },
}

#[cfg(test)]
pub(crate) fn discover_files(config: &LoadedConfig) -> Result<Vec<DiscoveredFile>> {
    discover_files_scoped(config, None)
}

/// Discover files, optionally narrowed to an extra `scope` glob set.
///
/// `scope` is applied on top of the workspace include/exclude. The kernel passes
/// the union of enabled rules' file scopes when no whole-repo capability is
/// requested, so a syntactic rule set never loads (or parses) files that no rule
/// could ever match. When `scope` is `None` the behavior is identical to the
/// plain workspace discovery.
pub(crate) fn discover_files_scoped(
    config: &LoadedConfig,
    scope: Option<&GlobSet>,
) -> Result<Vec<DiscoveredFile>> {
    let include = config.include_set()?;
    let exclude = config.exclude_set()?;

    // A repo's tree is the first thing every run touches, and on a large
    // checkout the directory walk dominates the time before any analysis
    // starts. Walking it in parallel is pure wall-clock: the traversal order
    // it gives up is already discarded by the sort below, so the discovered
    // set — and everything downstream of it — is unchanged. The thread count
    // is the resolved job budget, so the walk stays inside the same CPU cap
    // as every other parallel stage.
    let walker = WalkBuilder::new(&config.root)
        .hidden(false)
        .ignore(config.respect_gitignore)
        .git_ignore(config.respect_gitignore)
        .git_exclude(config.respect_gitignore)
        .require_git(false)
        .parents(config.respect_gitignore)
        .threads(crate::jobs::resolved_job_count())
        .build_parallel();

    let files = Mutex::new(Vec::new());
    let failure = Mutex::new(None::<String>);

    walker.run(|| {
        let include = &include;
        let exclude = &exclude;
        let files = &files;
        let failure = &failure;
        let root = config.root.as_path();

        Box::new(move |entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    record_walk_failure(failure, error.to_string());
                    return WalkState::Quit;
                }
            };
            if !entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
            {
                return WalkState::Continue;
            }
            let path = entry.path();
            if Language::from_path(path) == Language::Unknown {
                return WalkState::Continue;
            }
            let relative = match relative_path(root, path) {
                Ok(relative) => relative,
                Err(error) => {
                    record_walk_failure(failure, error.to_string());
                    return WalkState::Quit;
                }
            };
            if !should_include_relative_path(include, exclude, &relative) {
                return WalkState::Continue;
            }
            if let Some(scope) = scope
                && !matches_any(scope, &relative)
            {
                return WalkState::Continue;
            }
            // Only entries that survive every filter reach the lock, so the
            // shared vector is touched once per discovered source file rather
            // than once per directory entry.
            files.lock().expect(POISONED).push(DiscoveredFile {
                path: path.to_path_buf(),
                relative_path: relative,
            });
            WalkState::Continue
        })
    });

    if let Some(message) = failure.into_inner().expect(POISONED) {
        return Err(anyhow!(message));
    }

    let mut files = files.into_inner().expect(POISONED);
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(files)
}

const POISONED: &str = "file discovery worker panicked";

/// Keeps the lexicographically smallest failure so a parallel walk reports the
/// same message on every run; the serial walk reported whichever error it
/// reached first, which parallel traversal no longer defines.
fn record_walk_failure(slot: &Mutex<Option<String>>, message: String) {
    let mut slot = slot.lock().expect(POISONED);
    match slot.as_ref() {
        Some(existing) if *existing <= message => {}
        _ => *slot = Some(message),
    }
}

/// Fine-grained timings for [`load_analysis_files_with_timings`] (`polint::_bench::fs` / `polint-bench`).
#[allow(unreachable_pub)]
#[derive(Debug, Clone, Copy, Default)]
pub struct LoadSourcesTimings {
    /// `ignore` walk, language filter, glob include/exclude, stable sort.
    pub discover: Duration,
    /// Parallel bounded source reads for all discovered paths.
    pub read_parallel: Duration,
    /// Sequential `AnalysisDb::add_file` (content hash + file records).
    pub fingerprint_and_push: Duration,
}

impl LoadSourcesTimings {
    /// Sum of all sub-stage durations for `polint-bench`.
    #[cfg(feature = "bench")]
    pub fn total(&self) -> Duration {
        self.discover + self.read_parallel + self.fingerprint_and_push
    }
}

#[cfg(test)]
pub(crate) fn load_analysis_files(config: &LoadedConfig) -> Result<AnalysisDb> {
    let (db, _) = load_analysis_files_with_timings(config)?;
    Ok(db)
}

/// Load files narrowed to an extra `scope` glob set (see [`discover_files_scoped`]).
///
/// Oversized sources are omitted from the DB and reported as `polint/capability`
/// diagnostics instead of being read into memory.
pub(crate) fn load_analysis_files_scoped(
    config: &LoadedConfig,
    scope: Option<&GlobSet>,
) -> Result<(AnalysisDb, Vec<Diagnostic>)> {
    let (db, _, diagnostics) = load_analysis_files_with_timings_scoped(config, scope)?;
    Ok((db, diagnostics))
}

/// Same as in-module `load_analysis_files`, plus per-subphase timings (for profiling).
#[cfg(any(test, feature = "bench"))]
#[allow(unreachable_pub)]
pub fn load_analysis_files_with_timings(
    config: &LoadedConfig,
) -> Result<(AnalysisDb, LoadSourcesTimings)> {
    let (db, timings, _) = load_analysis_files_with_timings_scoped(config, None)?;
    Ok((db, timings))
}

fn load_analysis_files_with_timings_scoped(
    config: &LoadedConfig,
    scope: Option<&GlobSet>,
) -> Result<(AnalysisDb, LoadSourcesTimings, Vec<Diagnostic>)> {
    let mut timings = LoadSourcesTimings::default();

    let t0 = Instant::now();
    let discovered = discover_files_scoped(config, scope)?;
    timings.discover = t0.elapsed();

    let t1 = Instant::now();
    let outcomes = discovered
        .into_par_iter()
        .map(read_discovered_source)
        .collect::<Result<Vec<_>>>()?;
    timings.read_parallel = t1.elapsed();

    let t2 = Instant::now();
    let mut db = AnalysisDb::new();
    let mut diagnostics = Vec::new();
    for outcome in outcomes {
        match outcome {
            SourceReadOutcome::Loaded { file, source } => {
                db.add_file(file.path, file.relative_path, source);
            }
            SourceReadOutcome::Skipped(diagnostic) => diagnostics.push(*diagnostic),
        }
    }
    let rel_paths: Vec<String> = db.files().iter().map(|f| f.relative_path.clone()).collect();
    let path_ix = PathContextIndex::build(&config.config.path_contexts, &rel_paths);
    if !config.config.path_contexts.pairs.is_empty() {
        db.set_path_contexts(path_ix);
    }
    timings.fingerprint_and_push = t2.elapsed();

    Ok((db, timings, diagnostics))
}

enum SourceReadOutcome {
    Loaded {
        file: DiscoveredFile,
        source: String,
    },
    Skipped(Box<Diagnostic>),
}

fn read_discovered_source(file: DiscoveredFile) -> Result<SourceReadOutcome> {
    match repo_fs::read_file_to_string_with_limit(&file.path, SOURCE_FILE_MAX_BYTES) {
        Ok(source) => Ok(SourceReadOutcome::Loaded { file, source }),
        Err(RepoFileReadError::TooLarge { max_bytes }) => Ok(SourceReadOutcome::Skipped(Box::new(
            oversized_source_diagnostic(&file.relative_path, max_bytes),
        ))),
        Err(error) => Err(anyhow!("failed to read {}: {error}", file.path.display())),
    }
}

fn oversized_source_diagnostic(relative_path: &str, max_bytes: u64) -> Diagnostic {
    Diagnostic::error(
        "polint/capability",
        relative_path,
        TextRange::point(1, 1),
        format!(
            "Source file `{relative_path}` exceeds the {max_bytes}-byte read limit and was skipped."
        ),
    )
    .with_evidence("capability", "source")
    .with_evidence("status", "unsupported")
    .with_evidence("reason", "file-exceeds-source-read-size-limit")
    .with_evidence("max_bytes", max_bytes.to_string())
    .with_help(format!(
        "Exclude oversized generated files from workspace include patterns, or split the file; polint refuses to load sources larger than {max_bytes} bytes."
    ))
}

#[cfg(test)]
fn load_analysis_files_sequential(config: &LoadedConfig) -> Result<(AnalysisDb, Vec<Diagnostic>)> {
    let mut db = AnalysisDb::new();
    let mut diagnostics = Vec::new();
    for file in discover_files(config)? {
        match read_discovered_source(file)? {
            SourceReadOutcome::Loaded { file, source } => {
                db.add_file(file.path, file.relative_path, source);
            }
            SourceReadOutcome::Skipped(diagnostic) => diagnostics.push(*diagnostic),
        }
    }
    Ok((db, diagnostics))
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredFile {
    pub(crate) path: PathBuf,
    pub(crate) relative_path: String,
}

fn matches_any(globs: &GlobSet, relative_path: &str) -> bool {
    globs.is_match(relative_path) || globs.is_match(format!("./{relative_path}"))
}

pub(crate) fn should_include_relative_path(
    include: &GlobSet,
    exclude: &GlobSet,
    relative_path: &str,
) -> bool {
    matches_any(include, relative_path) && !matches_any(exclude, relative_path)
}

fn relative_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path.strip_prefix(root).map_err(|_| FsError::StripPrefix {
        path: path.to_path_buf(),
    })?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;
    use proptest::prelude::*;
    use std::fs;

    #[test]
    fn detects_language_from_path() {
        assert_eq!(Language::from_path(Path::new("a.go")), Language::Go);
        assert_eq!(Language::from_path(Path::new("a.tsx")), Language::Tsx);
    }

    #[test]
    fn discovery_is_deterministic() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("b.go"), "package main").unwrap();
        fs::write(temp.path().join("a.go"), "package main").unwrap();
        let config = load_config(temp.path()).unwrap();
        let files = discover_files(&config).unwrap();
        assert_eq!(files[0].relative_path, "a.go");
        assert_eq!(files[1].relative_path, "b.go");
    }

    #[test]
    fn discovery_order_is_root_relative_and_stable_with_nested_files() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path().join("src/z.tsx"), "export const z = 1;");
        write_file(temp.path().join("cmd/main.go"), "package main\n");
        write_file(temp.path().join("src/a.ts"), "export const a = 1;");
        write_file(temp.path().join("lib/b.js"), "export const b = 1;");

        let config = load_config(temp.path()).unwrap();
        let files = discover_files(&config).unwrap();

        assert_eq!(
            relative_paths(&files),
            ["cmd/main.go", "lib/b.js", "src/a.ts", "src/z.tsx"]
        );
    }

    #[test]
    fn discovery_filters_before_sorting() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(".gitignore"), "src/ignored.tsx\n").unwrap();
        fs::write(
            temp.path().join(".polint.toml"),
            r#"
[workspace]
include = ["src/**"]
exclude = ["src/excluded.tsx", "src/vendor/**"]
"#,
        )
        .unwrap();

        write_file(
            temp.path().join("src/nested/component.tsx"),
            "export const z = '#fff';",
        );
        write_file(
            temp.path().join("src/included.js"),
            "export const a = '#fff';",
        );
        write_file(
            temp.path().join("src/excluded.tsx"),
            "export const excluded = '#fff';",
        );
        write_file(
            temp.path().join("src/vendor/generated.ts"),
            "export const vendor = '#fff';",
        );
        write_file(
            temp.path().join("src/ignored.tsx"),
            "export const ignored = '#fff';",
        );
        write_file(temp.path().join("src/notes.txt"), "#fff\n");
        write_file(
            temp.path().join("outside.ts"),
            "export const outside = '#fff';",
        );

        let config = load_config(temp.path()).unwrap();
        let files = discover_files(&config).unwrap();

        assert_eq!(
            relative_paths(&files),
            ["src/included.js", "src/nested/component.tsx"]
        );
    }

    #[test]
    fn discovery_can_bypass_gitignore_for_explicit_internal_target_files() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(".gitignore"), "node_modules/\n").unwrap();
        write_file(
            temp.path().join("node_modules/pkg/index.js"),
            "module.exports = function pkg() {};",
        );
        let mut config = load_config(temp.path()).unwrap();
        config.config.workspace.include = vec!["node_modules/pkg/index.js".to_string()];
        config.config.workspace.exclude.clear();
        config.respect_gitignore = false;

        let files = discover_files(&config).unwrap();

        assert_eq!(relative_paths(&files), ["node_modules/pkg/index.js"]);
    }

    proptest! {
        #[test]
        fn discovery_include_exclude_decision_is_stable(
            dir in "[a-z]{1,8}",
            name in "[a-z]{1,8}",
            ext in "(go|ts|tsx|js|jsx)",
        ) {
            let relative_path = format!("{dir}/{name}.{ext}");
            let include = crate::config::build_glob_set(&[format!("{dir}/**")]).unwrap();
            let exclude = crate::config::build_glob_set(std::slice::from_ref(&relative_path)).unwrap();
            let empty_exclude = crate::config::build_glob_set(&[]).unwrap();

            prop_assert!(!should_include_relative_path(&include, &exclude, &relative_path));
            let first = should_include_relative_path(&include, &empty_exclude, &relative_path);
            let second = should_include_relative_path(&include, &empty_exclude, &relative_path);
            prop_assert_eq!(first, second);
            prop_assert!(first);
        }
    }

    #[test]
    fn load_analysis_files_preserves_discovery_order_in_file_ids() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path().join("src/z.tsx"), "export const z = 1;");
        write_file(temp.path().join("src/a.ts"), "export const a = 1;");
        write_file(temp.path().join("cmd/main.go"), "package main\n");

        let config = load_config(temp.path()).unwrap();
        let db = load_analysis_files(&config).unwrap();
        let files = db.files();

        assert_eq!(files.len(), 3);
        assert_eq!(files[0].relative_path, "cmd/main.go");
        assert_eq!(files[0].id.0, 0);
        assert_eq!(files[1].relative_path, "src/a.ts");
        assert_eq!(files[1].id.0, 1);
        assert_eq!(files[2].relative_path, "src/z.tsx");
        assert_eq!(files[2].id.0, 2);
    }

    #[test]
    fn load_analysis_files_parallel_preserves_file_ids() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path().join("src/z.tsx"), "export const z = 1;");
        write_file(temp.path().join("src/a.ts"), "export const a = 1;");
        write_file(temp.path().join("cmd/main.go"), "package main\n");

        let config = load_config(temp.path()).unwrap();
        let db = load_analysis_files(&config).unwrap();
        let ids = db
            .files()
            .iter()
            .map(|file| (file.relative_path.as_str(), file.id.0))
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec![("cmd/main.go", 0), ("src/a.ts", 1), ("src/z.tsx", 2)]
        );
    }

    #[test]
    fn load_analysis_files_parallel_matches_sequential_order() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path().join("b/view.tsx"), "export const b = 1;");
        write_file(temp.path().join("a/main.go"), "package main\n");
        write_file(temp.path().join("c/util.js"), "export const c = 1;");

        let config = load_config(temp.path()).unwrap();
        let parallel = load_analysis_files(&config).unwrap();
        let (sequential, _) = load_analysis_files_sequential(&config).unwrap();
        let parallel_paths = parallel
            .files()
            .iter()
            .map(|file| (file.relative_path.as_str(), file.id))
            .collect::<Vec<_>>();
        let sequential_paths = sequential
            .files()
            .iter()
            .map(|file| (file.relative_path.as_str(), file.id))
            .collect::<Vec<_>>();

        assert_eq!(parallel_paths, sequential_paths);
    }

    #[test]
    fn oversized_source_file_is_skipped_with_capability_diagnostic() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path().join("ok.go"), "package main\n");
        let oversized = temp.path().join("huge.go");
        {
            let file = fs::File::create(&oversized).unwrap();
            file.set_len(SOURCE_FILE_MAX_BYTES + 1).unwrap();
        }

        let config = load_config(temp.path()).unwrap();
        let (db, diagnostics) = load_analysis_files_scoped(&config, None).unwrap();

        assert_eq!(
            db.files()
                .iter()
                .map(|file| file.relative_path.as_str())
                .collect::<Vec<_>>(),
            ["ok.go"]
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule_id, "polint/capability");
        assert_eq!(diagnostics[0].file, "huge.go");
        assert!(
            diagnostics[0]
                .evidence
                .iter()
                .any(|evidence| evidence.label == "reason"
                    && evidence.value == "file-exceeds-source-read-size-limit")
        );
        assert!(
            !diagnostics[0].message.contains("topology"),
            "source-skip message must not reuse topology wording: {}",
            diagnostics[0].message
        );
    }

    fn write_file(path: impl AsRef<Path>, contents: &str) {
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn relative_paths(files: &[DiscoveredFile]) -> Vec<&str> {
        files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect()
    }
}
