//! The call-resolution cache.
//!
//! A plan whose deep capabilities stop at call resolution reads, from the whole
//! deep stack, only the call facts: call sites and their targets, the
//! entrypoints and reachability roots its queries start from, and the refined
//! call edges they walk. Every provider in that stack is a deterministic
//! function of the scanned sources, the configuration, the plan, the engine and
//! the toolchains it runs, so on a second run over unchanged inputs the stack
//! would recompute exactly what the first run stored. This module stores those
//! facts once and restores them in place of the stack.
//!
//! An entry is keyed on every input the stack reads: the input snapshot (every
//! scanned file's content digest, the configuration, the Go lifecycle, rules
//! and plan), the outputs of the providers scheduled before it, the identity of
//! the Go semantic sidecar run (frontend, toolchain, lifecycle), the adaptation
//! model files, and this engine's version. A run whose key matches restores,
//! diagnostics included; any other run recomputes and, when every provider of
//! the stack succeeded, replaces the entry. Scans that include TypeScript or JavaScript files always
//! recompute: their type-checker sidecar's identity is not part of the key.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::analysis_api::{Digest, InputSnapshot};
use crate::analysis_neutral::calls::facts::{CallSiteFact, CallTargetFact, UnresolvedCallFact};
use crate::analysis_neutral::calls::store::CallOutput;
use crate::analysis_neutral::entrypoints::facts::{
    EntrypointFact, FrameworkDispatchEdgeFact, TrustBoundaryFact, UnresolvedFrameworkFact,
};
use crate::analysis_neutral::entrypoints::store::EntrypointOutput;
use crate::analysis_neutral::ids::ReachabilityRootId;
use crate::analysis_neutral::reachability::facts::ReachabilityRootFact;
use crate::analysis_neutral::reachability::store::ReachabilityProviderOutput;
use crate::analysis_neutral::refined_calls::facts::RefinedCallEdgeFact;
use crate::analysis_neutral::refined_calls::store::RefinedCallOutput;
use crate::core::AnalysisDb;
use crate::internal_core::{Language, StableKeyId};

const CALL_CACHE_SCHEMA: &str = "polint-call-resolution-cache-1";

/// Entries kept on disk; older ones are removed when a new one is written.
const ENTRIES_KEPT: usize = 2;

/// The providers an entry stands in for, in schedule order. A run that
/// restores an entry runs none of them.
pub(crate) const CACHED_PROVIDERS: &[&str] = &[
    "polint.semantic_mir",
    "polint.go.semantic",
    "polint.cfg",
    "polint.calls",
    "polint.ts.types",
    "polint.identity",
    "polint.abstract_domains",
    "polint.direct_summaries",
    "polint.entrypoints",
    "polint.reachability",
    "polint.extensions",
    "polint.type_value_alias",
    "polint.semantic_graph",
    "polint.solver",
    "polint.refined_calls",
];

/// The providers whose outputs are part of an entry's key. The key is formed
/// as soon as the Go syntax is known, before the semantic sidecar would be
/// started, so a run that will restore never starts it; the providers that run
/// in between derive from inputs the input snapshot already pins (the scanned
/// files, the module and workspace files, the configuration).
pub(crate) const KEYED_UPSTREAM: &[&str] = &["polint.go.syntax"];

/// Whether a run may use the cache: it asks for call resolution and nothing
/// that reads past it, and it scans only Go.
pub(crate) fn eligible(
    plan: &crate::analysis_plan::AnalysisPlan,
    db: &AnalysisDb,
    enabled_providers: &std::collections::BTreeSet<&'static str>,
) -> bool {
    CACHED_PROVIDERS
        .iter()
        .all(|provider| enabled_providers.contains(provider))
        && !crate::analysis_kernel::provider::control_or_data_flow_requested(plan)
        && !plan.requests_per_point_domain_facts()
        && !db.files().is_empty()
        && db.files().iter().all(|file| file.language == Language::Go)
}

/// The key of the entry a run with these inputs would read or write.
///
/// `upstream` must hold the output digest of every [`KEYED_UPSTREAM`] provider
/// and `go_semantic_identity` the identity of the semantic sidecar run;
/// without either the run's inputs are not pinned and there is no key.
pub(crate) fn entry_key(
    input_snapshot: &InputSnapshot,
    upstream: &BTreeMap<&'static str, Digest>,
    go_semantic_identity: Option<&str>,
    root: &Path,
) -> Option<String> {
    let snapshot = serde_json::to_string(input_snapshot).ok()?;
    let mut parts = vec![
        CALL_CACHE_SCHEMA.to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
        crate::cache::stable_hash(&[snapshot.as_str()]),
        format!("go_semantic={}", go_semantic_identity?),
        format!("models={}", adaptation_models_digest(root)),
    ];
    for provider in KEYED_UPSTREAM {
        parts.push(format!("{provider}={}", upstream.get(provider)?));
    }
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Some(crate::cache::stable_hash(&refs))
}

/// A digest over the adaptation model files, which can add semantic-graph
/// constraints and so refined call edges.
fn adaptation_models_digest(root: &Path) -> String {
    let directory = root.join(".polint").join("models");
    let mut files = std::fs::read_dir(&directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort();
    let mut parts = Vec::with_capacity(files.len());
    for path in files {
        let content = std::fs::read(&path).unwrap_or_default();
        parts.push(format!(
            "{}={}",
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            crate::cache::stable_hash(&[String::from_utf8_lossy(&content).as_ref()])
        ));
    }
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    crate::cache::stable_hash(&refs)
}

/// One cached provider's outcome, recorded so a restoring run reports the
/// output digest and the diagnostics the computing run did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CachedProvider {
    pub(crate) id: String,
    pub(crate) output_digest: Digest,
    pub(crate) diagnostics: Vec<crate::internal_core::Diagnostic>,
}

/// The stored facts, their stable keys replaced by indices into `keys`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Entry {
    schema: String,
    key: String,
    keys: Vec<String>,
    providers: Vec<CachedProvider>,
    call_sites: Vec<CallSiteFact>,
    call_targets: Vec<CallTargetFact>,
    unresolved_calls: Vec<UnresolvedCallFact>,
    entrypoints: Vec<EntrypointFact>,
    trust_boundaries: Vec<TrustBoundaryFact>,
    dispatch_edges: Vec<FrameworkDispatchEdgeFact>,
    unresolved_frameworks: Vec<UnresolvedFrameworkFact>,
    reachability_roots: Vec<ReachabilityRootFact>,
    /// Root ids, which the root's own serialization leaves out.
    reachability_root_ids: Vec<u64>,
    refined_edges: Vec<RefinedCallEdgeFact>,
}

impl Entry {
    /// Rewrites every stable-key id the facts carry through `remap`.
    fn remap_stable_keys(&mut self, mut remap: impl FnMut(StableKeyId) -> StableKeyId) {
        for row in &mut self.call_sites {
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut self.call_targets {
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut self.unresolved_calls {
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut self.entrypoints {
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut self.trust_boundaries {
            row.entrypoint_stable_key = remap(row.entrypoint_stable_key);
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut self.dispatch_edges {
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut self.unresolved_frameworks {
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut self.reachability_roots {
            row.stable_key = remap(row.stable_key);
        }
        for row in &mut self.refined_edges {
            row.stable_key = remap(row.stable_key);
        }
    }
}

/// Where entries live: next to the sidecar outputs, when caching is enabled.
pub(crate) fn cache_dir(cache: &crate::cache::Cache) -> Option<PathBuf> {
    cache
        .sidecar_cache_dir()
        .and_then(|sidecar| sidecar.parent().map(|parent| parent.join("calls")))
}

fn entry_path(directory: &Path, key: &str) -> PathBuf {
    directory.join(format!("{key}.json"))
}

/// Whether an entry is stored under `key`; [`restore`] decides whether it is
/// usable.
pub(crate) fn has_entry(directory: &Path, key: &str) -> bool {
    entry_path(directory, key).is_file()
}

/// Restores the entry stored under `key` into `db`, returning the identities
/// of the providers it stands in for; `None` when there is no usable entry.
///
/// An entry that cannot be read, parsed or stored is deleted, so a damaged
/// entry costs one recomputation rather than one per run.
pub(crate) fn restore(
    db: &mut AnalysisDb,
    directory: &Path,
    key: &str,
) -> Option<Vec<CachedProvider>> {
    let path = entry_path(directory, key);
    let file = std::fs::File::open(&path).ok()?;
    let parsed: Result<Entry, _> = serde_json::from_reader(std::io::BufReader::new(file));
    let restored = parsed
        .ok()
        .filter(|entry| entry.schema == CALL_CACHE_SCHEMA && entry.key == key)
        .and_then(|entry| store_entry(db, entry));
    if restored.is_none() {
        let _ = std::fs::remove_file(&path);
    }
    restored
}

fn store_entry(db: &mut AnalysisDb, mut entry: Entry) -> Option<Vec<CachedProvider>> {
    let interner = db.stable_key_interner();
    let ids = entry
        .keys
        .iter()
        .map(|text| interner.intern(text.as_str()))
        .collect::<Vec<_>>();
    let mut dangling = false;
    entry.remap_stable_keys(|index| match ids.get(index.0 as usize) {
        Some(id) => *id,
        None => {
            dangling = true;
            index
        }
    });
    if dangling || entry.reachability_root_ids.len() != entry.reachability_roots.len() {
        return None;
    }
    for (root, id) in entry
        .reachability_roots
        .iter_mut()
        .zip(&entry.reachability_root_ids)
    {
        root.id = ReachabilityRootId(*id);
    }
    db.mark_call_facts_restored_without_mir();
    db.replace_call_facts(CallOutput {
        sites: entry.call_sites,
        targets: entry.call_targets,
        unresolved: entry.unresolved_calls,
    })
    .ok()?;
    db.replace_entrypoint_facts(EntrypointOutput {
        entrypoints: entry.entrypoints,
        trust_boundaries: entry.trust_boundaries,
        dispatch_edges: entry.dispatch_edges,
        unresolved: entry.unresolved_frameworks,
    })
    .ok()?;
    db.replace_reachability_facts(ReachabilityProviderOutput {
        roots: entry.reachability_roots,
    })
    .ok()?;
    db.replace_refined_call_facts(RefinedCallOutput {
        edges: entry.refined_edges,
    })
    .ok()?;
    Some(entry.providers)
}

/// Writes the facts a run just computed under `key`, replacing older entries.
pub(crate) fn persist(
    db: &AnalysisDb,
    directory: &Path,
    key: &str,
    providers: Vec<CachedProvider>,
) {
    let mut entry = Entry {
        schema: CALL_CACHE_SCHEMA.to_string(),
        key: key.to_string(),
        keys: Vec::new(),
        providers,
        call_sites: db.call_sites().to_vec(),
        call_targets: db.call_targets().to_vec(),
        unresolved_calls: db.unresolved_calls().to_vec(),
        entrypoints: db.entrypoint_facts().to_vec(),
        trust_boundaries: db.trust_boundary_facts().to_vec(),
        dispatch_edges: db.dispatch_edge_facts().to_vec(),
        unresolved_frameworks: db.unresolved_framework_facts().to_vec(),
        reachability_roots: db.reachability_roots().to_vec(),
        reachability_root_ids: db
            .reachability_roots()
            .iter()
            .map(|root| root.id.0)
            .collect(),
        refined_edges: db.refined_call_edges().to_vec(),
    };
    let interner = db.stable_key_interner();
    let mut indices = BTreeMap::<StableKeyId, StableKeyId>::new();
    let mut keys = Vec::new();
    entry.remap_stable_keys(|id| {
        *indices.entry(id).or_insert_with(|| {
            keys.push(interner.resolve(id).to_string());
            StableKeyId(u32::try_from(keys.len() - 1).expect("fewer than 2^32 cached keys"))
        })
    });
    entry.keys = keys;
    if write_entry(directory, key, &entry).is_err() {
        tracing::debug!(
            target: "polint::kernel::stage",
            "call-resolution cache entry could not be written"
        );
    }
}

fn write_entry(directory: &Path, key: &str, entry: &Entry) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(directory)?;
    let file = tempfile::NamedTempFile::new_in(directory)?;
    {
        let mut writer = std::io::BufWriter::new(file.as_file());
        serde_json::to_writer(&mut writer, entry)?;
        writer.flush()?;
    }
    file.persist(entry_path(directory, key))
        .map_err(|error| error.error)?;
    remove_older_entries(directory, key);
    Ok(())
}

/// Keeps the [`ENTRIES_KEPT`] most recently written entries, `key` among them.
fn remove_older_entries(directory: &Path, key: &str) {
    let current = entry_path(directory, key);
    let mut entries = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter(|path| *path != current)
        .filter_map(|path| {
            let modified = path.metadata().and_then(|meta| meta.modified()).ok()?;
            Some((modified, path))
        })
        .collect::<Vec<_>>();
    entries.sort();
    let keep_others = ENTRIES_KEPT.saturating_sub(1);
    let excess = entries.len().saturating_sub(keep_others);
    for (_, path) in entries.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}
