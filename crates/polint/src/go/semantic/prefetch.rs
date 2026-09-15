//! Starts the Go semantic sidecar before the provider that consumes it runs.
//!
//! `polint.go.semantic` is a subprocess round trip, and on a Go repository it is
//! the longest stage of a run by a wide margin. Its inputs are fixed the moment
//! `polint.go.syntax` has a digest: the Go lifecycle config is derived from the
//! same settings and the same file set, and the sidecar reads the working tree
//! rather than anything the kernel builds. Everything the schedule runs in
//! between — the module graph, the symbol sidecar, the CFG — therefore has no
//! bearing on what the semantic sidecar will answer, and waiting for those
//! stages to finish before asking the question spends their wall time twice.
//!
//! Starting the subprocess as soon as its inputs exist overlaps it with those
//! stages instead. The provider still owns the result: it hands its own
//! lifecycle config to [`GoSemanticPrefetch::take_for`], which returns the
//! prefetched run only when the config it was started with is the same
//! config, and otherwise waits for the speculative run to end and lets the
//! provider do its own. A prefetch can therefore make a run faster or waste one
//! subprocess, never change an answer.
//!
//! Memory: the overlap is with the Go *symbol* sidecar, which peaks around a
//! quarter of a gigabyte, while the semantic sidecar is the process that
//! actually needs room. A machine where the two together do not fit is a
//! machine where the semantic sidecar alone does not fit.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

use toml::Value;

use crate::analysis_api::FactDatabase;
use crate::go::lifecycle::{GoAnalysisConfig, go_files};
use crate::go::semantic::client::{GoSemanticClient, GoSemanticClientError, GoSemanticClientRun};
use crate::go::semantic::process::GoSemanticProcessError;

type ClientResult = Result<GoSemanticClientRun, GoSemanticClientError>;

/// A semantic sidecar run started ahead of its provider.
pub(crate) struct GoSemanticPrefetch {
    config: GoAnalysisConfig,
    handle: JoinHandle<ClientResult>,
}

impl GoSemanticPrefetch {
    /// Starts the sidecar for the lifecycle config these inputs describe.
    ///
    /// Returns `None` when the provider would not have run the sidecar either:
    /// no Go files, an unusable lifecycle config, files outside every module
    /// root, or a module root without a `go.mod`. Those are the cases the
    /// provider answers with a diagnostic instead of a subprocess, and
    /// speculating past them would run a sidecar whose result is never read.
    pub(crate) fn start(
        root: &Path,
        go_settings: &BTreeMap<String, Value>,
        db: &dyn FactDatabase,
        sidecar_cache_dir: Option<PathBuf>,
        upstream_digest: String,
    ) -> Option<Self> {
        let files = go_files(db);
        if files.is_empty() {
            return None;
        }
        let config = GoAnalysisConfig::from_settings_files(root, go_settings, &files).ok()?;
        if !config.files_without_module_root.is_empty()
            || !config.missing_module_roots(root).is_empty()
        {
            return None;
        }

        let root_owned = root.to_path_buf();
        let started = config.clone();
        let handle = std::thread::Builder::new()
            .name("polint-go-semantic-prefetch".to_string())
            .spawn(move || {
                let client = GoSemanticClient::new(root_owned, &started);
                match sidecar_cache_dir.as_deref() {
                    Some(dir) => client.run_cached(&started, dir, &upstream_digest),
                    None => client.run(&started),
                }
            })
            .ok()?;
        Some(Self { config, handle })
    }

    /// The prefetched run, when it answers the question `config` asks.
    ///
    /// A config that does not match is not an error and not a fallback worth
    /// racing: the speculative run is waited out first so the machine is never
    /// running two semantic sidecars over the same tree at once.
    pub(crate) fn take_for(self, config: &GoAnalysisConfig) -> Option<ClientResult> {
        if self.config != *config {
            let _ = self.handle.join();
            return None;
        }
        Some(join(self.handle))
    }

    /// Waits for a prefetch nothing asked for, so no sidecar outlives the run.
    pub(crate) fn abandon(self) {
        let _ = self.handle.join();
    }
}

fn join(handle: JoinHandle<ClientResult>) -> ClientResult {
    handle.join().unwrap_or_else(|_| {
        Err(GoSemanticClientError::Process(
            GoSemanticProcessError::CommandFailed(
                "the Go semantic sidecar thread panicked".to_string(),
            ),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::go::local_db::LocalFactDb;

    fn add_go_file(db: &mut LocalFactDb, root: &Path, relative_path: &str, source: &str) {
        let path = root.join(relative_path);
        std::fs::create_dir_all(path.parent().expect("fixture has a parent")).expect("mkdirs");
        std::fs::write(&path, source).expect("write fixture");
        db.add_file(path, relative_path.to_string(), source.to_string());
    }

    #[test]
    fn a_tree_without_go_files_starts_no_sidecar() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = LocalFactDb::new();

        let prefetch = GoSemanticPrefetch::start(
            temp.path(),
            &BTreeMap::new(),
            &db,
            None,
            "upstream".to_string(),
        );

        assert!(
            prefetch.is_none(),
            "the provider answers an empty Go file set without a subprocess, so speculating \
             would run a sidecar nothing reads"
        );
    }

    #[test]
    fn go_files_outside_every_module_root_start_no_sidecar() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut db = LocalFactDb::new();
        add_go_file(&mut db, temp.path(), "stray/thing.go", "package thing\n");

        let prefetch = GoSemanticPrefetch::start(
            temp.path(),
            &BTreeMap::new(),
            &db,
            None,
            "upstream".to_string(),
        );

        assert!(
            prefetch.is_none(),
            "a file under no go.mod is the provider's setup diagnostic, not a sidecar run"
        );
    }
}
