//! Automatic size budgets for the two places compiled rule hosts accumulate: a
//! checkout's rules target directory and the machine-global rule-host store.
//!
//! Both only ever grow by whole rule hosts, so both shrink by whole rule hosts,
//! least recently used first, once they are over budget. Nothing else is
//! removed: the libraries a rule host links (polint itself, its parsers) are
//! shared by every host in a target directory and are what makes the next
//! build cheap, so a target directory whose libraries alone exceed its budget
//! stays over it.
//!
//! Eviction never changes what a check reports. It runs after the rule host
//! that produced the check's results has exited, it skips every host another
//! polint process is building or running (each one holds a [`HostLease`]), and
//! a host it did remove is rebuilt, or restored, the next time it is asked for.

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Overrides the rules target directory budget, in MiB, or turns it off.
pub(crate) const RULES_TARGET_MAX_MB_ENV: &str = "POLINT_RULES_TARGET_MAX_MB";

/// Overrides the machine-global rule-host store budget, in MiB, or turns it off.
pub(crate) const CACHE_STORE_MAX_MB_ENV: &str = "POLINT_CACHE_STORE_MAX_MB";

/// Room for the shared libraries of one build profile plus a few dozen hosts.
const DEFAULT_RULES_TARGET_MAX_MB: u64 = 4096;

/// Room for a few dozen optimized hosts across checkouts and polint versions.
const DEFAULT_CACHE_STORE_MAX_MB: u64 = 2048;

/// The directory under a rules target directory that holds one lease file per
/// rule host polint has built or run there.
const LEASES_DIR_NAME: &str = "polint-hosts";

/// The lock a store publish holds shared and a store eviction holds exclusive.
pub(crate) const STORE_EVICTION_LOCK: &str = "polint-store-eviction.lock";

const OFF_VALUES: [&str; 3] = ["off", "disabled", "none"];

/// The rules target directory budget in bytes, or `None` when it is off.
pub(crate) fn rules_target_budget() -> Option<u64> {
    budget_bytes(
        std::env::var(RULES_TARGET_MAX_MB_ENV).ok().as_deref(),
        DEFAULT_RULES_TARGET_MAX_MB,
    )
}

/// The rule-host store budget in bytes, or `None` when it is off.
pub(crate) fn store_budget() -> Option<u64> {
    budget_bytes(
        std::env::var(CACHE_STORE_MAX_MB_ENV).ok().as_deref(),
        DEFAULT_CACHE_STORE_MAX_MB,
    )
}

/// A budget value in MiB, read as bytes. Unset, empty, or unreadable values
/// keep the default, so a typo can never turn the limit off.
fn budget_bytes(value: Option<&str>, default_mb: u64) -> Option<u64> {
    let value = value.map(str::trim).unwrap_or_default();
    if OFF_VALUES.iter().any(|off| value.eq_ignore_ascii_case(off)) {
        return None;
    }
    let mb = value.parse::<u64>().unwrap_or(default_mb);
    Some(mb.saturating_mul(1024 * 1024))
}

/// A claim on one rule host in a rules target directory, held for as long as
/// this process builds or runs it.
///
/// The claim is a shared lock on the host's lease file, so any number of polint
/// processes can use the same host at once, and an eviction, which needs the
/// lock exclusively, passes over the host until every one of them is done.
/// Taking the lease also marks the host as just used.
pub(crate) struct HostLease {
    _file: File,
}

impl HostLease {
    /// Lease the rule host `manifest` builds in `target_dir`, or `None` when the
    /// lease cannot be taken. `None` only means this host is not protected from
    /// eviction and does not count as recently used: building and running it
    /// proceed exactly as before.
    pub(crate) fn acquire(target_dir: &Path, manifest: &Path) -> Option<Self> {
        let package = manifest_package_name(manifest)?;
        let dir = target_dir.join(LEASES_DIR_NAME);
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join(format!("{package}.lock"));
        // An eviction deletes a lease file while it holds the lock exclusively.
        // A file opened just before that deletion is no longer the lease, so the
        // lock is retried on whatever file the path names now.
        for _ in 0..8 {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(&path)
                .ok()?;
            file.lock_shared().ok()?;
            if is_same_file(&file, &path) {
                let _ = file.set_modified(SystemTime::now());
                return Some(Self { _file: file });
            }
        }
        None
    }
}

#[cfg(unix)]
fn is_same_file(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (file.metadata(), std::fs::metadata(path)) {
        (Ok(open), Ok(named)) => open.dev() == named.dev() && open.ino() == named.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn is_same_file(_file: &File, path: &Path) -> bool {
    // Lease files are never deleted where an open file cannot be, so the open
    // file is always the one the path names.
    path.is_file()
}

/// The `[package] name` of a rule package manifest.
fn manifest_package_name(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let value = toml::from_str::<toml::Table>(&text).ok()?;
    let name = value.get("package")?.get("name")?.as_str()?;
    is_plain_name(name).then(|| name.to_string())
}

/// Whether `name` is safe to use as one file name component and as a prefix of
/// the cargo artifacts derived from it.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Bring `target_dir` within `budget` bytes by removing the least recently used
/// rule hosts, never one a polint process holds a lease on. The caller holds
/// the lease on the host it just used, which is what keeps that host.
///
/// Only rule hosts polint leased in this directory are candidates, and for each
/// only the artifacts cargo derives from that package's name: its binaries,
/// dep-info, fingerprints, build-script output, and incremental state. Best
/// effort: anything that cannot be read or removed is left in place.
pub(crate) fn evict_rules_target(target_dir: &Path, budget: u64) {
    let mut total = tree_size(target_dir);
    if total <= budget {
        return;
    }
    let leases_dir = target_dir.join(LEASES_DIR_NAME);
    for (_, package, lease_path) in oldest_first(&leases_dir, "lock") {
        if total <= budget {
            break;
        }
        if !is_plain_name(&package) {
            continue;
        }
        let Ok(lease) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lease_path)
        else {
            continue;
        };
        // A host somebody is building or running holds this shared.
        if lease.try_lock().is_err() {
            continue;
        }
        let artifacts = host_artifacts(target_dir, &package);
        total = total.saturating_sub(paths_size(&artifacts));
        for artifact in &artifacts {
            remove_path(artifact);
        }
        remove_lease_file(&lease_path);
    }
}

#[cfg(unix)]
fn remove_lease_file(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(not(unix))]
fn remove_lease_file(_path: &Path) {
    // Deleting a file another process has open fails here, and a lease that
    // outlives its host only costs one empty file.
}

/// Every artifact cargo derives from rule package `package` under any profile
/// directory of `target_dir`.
fn host_artifacts(target_dir: &Path, package: &str) -> Vec<PathBuf> {
    let crate_name = package.replace('-', "_");
    // Cargo names a unit `<name>-<hex hash>`, plus an extension for files.
    // Requiring the hash keeps a package from matching another whose name
    // merely starts with the same words.
    let unit_prefix = |name: &str, prefix: &str| {
        name.strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix('-'))
            .map(|rest| rest.split('.').next().unwrap_or_default())
            .is_some_and(|hash| !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    let mut out = Vec::new();
    for profile_dir in read_dirs(target_dir) {
        if !profile_dir.join(".fingerprint").is_dir() {
            continue;
        }
        for (subdir, prefix) in [
            (".fingerprint", package),
            ("build", package),
            ("deps", crate_name.as_str()),
            ("incremental", crate_name.as_str()),
        ] {
            for entry in read_entries(&profile_dir.join(subdir)) {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let library = name.strip_prefix("lib").unwrap_or(&name);
                if unit_prefix(&name, prefix) || (subdir == "deps" && unit_prefix(library, prefix))
                {
                    out.push(entry.path());
                }
            }
        }
        for entry in read_entries(&profile_dir) {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let stem = name.split('.').next().unwrap_or_default();
            if (stem == package || stem == crate_name) && !entry.path().is_dir() {
                out.push(entry.path());
            }
        }
    }
    out
}

/// Bring the rule-host store at `root` within `budget` bytes by removing the
/// least recently used entries and every blob no remaining entry names.
///
/// A restore marks its entry as used. Publishers hold the eviction lock shared,
/// so a blob is never removed between being written and being named by its
/// entry; an eviction that finds the lock taken leaves the store for the next
/// one, and a publisher that finds an eviction running skips publishing. A restore that loses its entry or blob to an eviction is a miss and
/// builds locally, so eviction never changes which host a check runs, only
/// where it came from.
pub(crate) fn evict_store(root: &Path, budget: u64) {
    let Some(lock) = open_lock(&root.join(STORE_EVICTION_LOCK)) else {
        return;
    };
    if lock.try_lock().is_err() {
        return;
    }
    let mut total = tree_size(root);
    if total <= budget {
        return;
    }

    let entries = store_entries(root);
    let mut references = BTreeMap::<String, usize>::new();
    for (_, _, _, blob) in &entries {
        if let Some(blob) = blob {
            *references.entry(blob.clone()).or_default() += 1;
        }
    }
    // Blobs no entry names first: no restore can reach them.
    let named = references.keys().cloned().collect::<HashSet<_>>();
    for (_, hash, path) in oldest_first_nested(&root.join("blobs"), None) {
        if total <= budget {
            return;
        }
        if !named.contains(&hash) && !hash.starts_with('.') {
            total = total.saturating_sub(tree_size(&path));
            remove_path(&path);
        }
    }
    for (_, _, path, blob) in entries {
        if total <= budget {
            return;
        }
        total = total.saturating_sub(tree_size(&path));
        remove_path(&path);
        let Some(blob) = blob else {
            continue;
        };
        let remaining = references.entry(blob.clone()).or_default();
        *remaining = remaining.saturating_sub(1);
        if *remaining == 0
            && let Some(shard) = blob.get(..2)
        {
            let path = root.join("blobs").join(shard).join(&blob);
            total = total.saturating_sub(tree_size(&path));
            remove_path(&path);
        }
    }
}

/// The shared side of the store eviction lock, held while a host is published
/// so that its blob cannot be removed before its entry names it. `None` while an
/// eviction is running: publishing is best effort, and a check never waits for
/// an eviction.
pub(crate) fn store_publish_guard(root: &Path) -> Option<File> {
    std::fs::create_dir_all(root).ok()?;
    let file = open_lock(&root.join(STORE_EVICTION_LOCK))?;
    file.try_lock_shared().ok()?;
    Some(file)
}

/// Mark the store entry at `path` as just used.
pub(crate) fn touch(path: &Path) {
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// Every store entry, oldest first, with the blob it names when it can be read.
fn store_entries(root: &Path) -> Vec<(SystemTime, String, PathBuf, Option<String>)> {
    oldest_first_nested(&root.join("rule-hosts"), Some("json"))
        .into_iter()
        .map(|(modified, name, path)| {
            let blob = std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .and_then(|entry| entry.get("sha256")?.as_str().map(str::to_string))
                .filter(|hash| hash.len() > 2 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
            (modified, name, path, blob)
        })
        .collect()
}

fn open_lock(path: &Path) -> Option<File> {
    std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .ok()
}

/// The files in `dir` with extension `extension`, oldest modification first,
/// as `(modified, stem, path)`. Ties are broken by name so the order is the
/// same on every run.
fn oldest_first(dir: &Path, extension: &str) -> Vec<(SystemTime, String, PathBuf)> {
    let mut out = read_entries(dir)
        .into_iter()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()? != extension {
                return None;
            }
            let stem = path.file_stem()?.to_str()?.to_string();
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, stem, path))
        })
        .collect::<Vec<_>>();
    out.sort();
    out
}

/// Like [`oldest_first`] over the files one shard directory below `dir`, with
/// any extension when `extension` is `None`.
fn oldest_first_nested(dir: &Path, extension: Option<&str>) -> Vec<(SystemTime, String, PathBuf)> {
    let mut out = Vec::new();
    for shard in read_dirs(dir) {
        for entry in read_entries(&shard) {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let name = match extension {
                Some(extension) if path.extension().is_some_and(|ext| ext == extension) => {
                    path.file_stem()
                }
                Some(_) => continue,
                None => path.file_name(),
            };
            let (Some(name), Ok(modified)) = (name.and_then(|n| n.to_str()), metadata.modified())
            else {
                continue;
            };
            out.push((modified, name.to_string(), path));
        }
    }
    out.sort();
    out
}

fn read_entries(dir: &Path) -> Vec<std::fs::DirEntry> {
    std::fs::read_dir(dir)
        .map(|entries| entries.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

fn read_dirs(dir: &Path) -> Vec<PathBuf> {
    read_entries(dir)
        .into_iter()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect()
}

/// The bytes the files under `path` occupy, counting each hard-linked file
/// once and never following symbolic links.
fn tree_size(path: &Path) -> u64 {
    paths_size(std::slice::from_ref(&path.to_path_buf()))
}

/// [`tree_size`] over several paths at once, so a file hard-linked under two
/// of them is counted once.
fn paths_size(paths: &[PathBuf]) -> u64 {
    let mut seen = HashSet::new();
    let mut total = 0;
    let mut pending = paths.to_vec();
    while let Some(path) = pending.pop() {
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            pending.extend(read_entries(&path).into_iter().map(|entry| entry.path()));
        } else if metadata.is_file() && seen.insert(file_identity(&metadata, &path)) {
            total += metadata.len();
        }
    }
    total
}

#[cfg(unix)]
fn file_identity(metadata: &std::fs::Metadata, _path: &Path) -> (u64, u64, PathBuf) {
    use std::os::unix::fs::MetadataExt;
    (metadata.dev(), metadata.ino(), PathBuf::new())
}

#[cfg(not(unix))]
fn file_identity(_metadata: &std::fs::Metadata, path: &Path) -> (u64, u64, PathBuf) {
    (0, 0, path.to_path_buf())
}

fn remove_path(path: &Path) {
    let _ = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(_) => Ok(()),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const MIB: u64 = 1024 * 1024;

    fn write(path: &Path, len: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0_u8; len]).unwrap();
    }

    fn age(path: &Path, seconds_ago: u64) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(seconds_ago))
            .unwrap();
    }

    fn manifest(dir: &Path, package: &str) -> PathBuf {
        let path = dir.join(package).join("Cargo.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!("[package]\nname = \"{package}\"\nversion = \"0.1.0\"\n"),
        )
        .unwrap();
        path
    }

    /// A rule host cargo built in `target`: its binary, hard link, dep-info,
    /// fingerprint, and incremental state, `len` bytes of binary.
    fn host(target: &Path, package: &str, len: usize) {
        let crate_name = package.replace('-', "_");
        let deps = target.join("debug/deps").join(format!("{crate_name}-0123"));
        write(&deps, len);
        write(&deps.with_extension("d"), 10);
        std::fs::hard_link(&deps, target.join("debug").join(package)).unwrap();
        write(
            &target
                .join("debug/.fingerprint")
                .join(format!("{package}-0123/bin-{package}")),
            10,
        );
        write(
            &target
                .join("debug/incremental")
                .join(format!("{crate_name}-0456/s-1/query")),
            10,
        );
    }

    fn lease(target: &Path, manifests: &Path, package: &str, seconds_ago: u64) -> HostLease {
        let lease = HostLease::acquire(target, &manifest(manifests, package)).unwrap();
        age(
            &target.join(LEASES_DIR_NAME).join(format!("{package}.lock")),
            seconds_ago,
        );
        lease
    }

    #[test]
    fn a_budget_is_a_size_in_mib_that_only_an_explicit_off_disables() {
        assert_eq!(budget_bytes(None, 7), Some(7 * MIB));
        assert_eq!(budget_bytes(Some(""), 7), Some(7 * MIB));
        assert_eq!(budget_bytes(Some(" 12 "), 7), Some(12 * MIB));
        assert_eq!(budget_bytes(Some("twelve"), 7), Some(7 * MIB));
        assert_eq!(budget_bytes(Some("0"), 7), Some(0));
        for off in ["off", "OFF", "disabled", "none"] {
            assert_eq!(budget_bytes(Some(off), 7), None, "{off}");
        }
    }

    #[test]
    fn the_least_recently_used_hosts_go_first_until_the_target_fits() {
        let target = tempfile::tempdir().unwrap();
        let manifests = tempfile::tempdir().unwrap();
        let target = target.path();
        write(
            &target.join("debug/deps/libpolint-9999.rlib"),
            3 * MIB as usize,
        );
        for package in ["oldest", "older", "newer", "current"] {
            host(target, package, MIB as usize);
        }
        // Another package whose name only starts with an evicted host's.
        let helper = [
            target.join("debug/.fingerprint/oldest-helper-0123/lib-oldest-helper"),
            target.join("debug/build/oldest-helper-0456/output"),
        ];
        for path in &helper {
            write(path, 10);
        }
        drop(lease(target, manifests.path(), "oldest", 300));
        drop(lease(target, manifests.path(), "older", 200));
        drop(lease(target, manifests.path(), "newer", 100));
        let _current = lease(target, manifests.path(), "current", 400);

        // 3 MiB of libraries and four 1 MiB hosts: two hosts must go.
        evict_rules_target(target, 5 * MIB + MIB / 2);

        let deps = target.join("debug/deps");
        assert!(!deps.join("oldest-0123").exists());
        assert!(!deps.join("older-0123").exists());
        assert!(!target.join("debug/oldest").exists());
        assert!(!target.join("debug/.fingerprint/oldest-0123").exists());
        assert!(!target.join("debug/incremental/oldest-0456").exists());
        // Elsewhere an evicted host's lease file is left as an empty file.
        #[cfg(unix)]
        assert!(!target.join(LEASES_DIR_NAME).join("oldest.lock").exists());
        assert!(deps.join("newer-0123").exists(), "within budget already");
        for path in &helper {
            assert!(path.exists(), "{} is not the host's", path.display());
        }
        assert!(
            deps.join("current-0123").exists(),
            "the host this check used is never evicted, however old"
        );
        assert!(
            deps.join("libpolint-9999.rlib").exists(),
            "libraries are never evicted"
        );
        assert!(tree_size(target) <= 5 * MIB + MIB / 2);
    }

    #[test]
    fn a_target_within_budget_is_left_alone() {
        let target = tempfile::tempdir().unwrap();
        let manifests = tempfile::tempdir().unwrap();
        let target = target.path();
        host(target, "idle", MIB as usize);
        drop(lease(target, manifests.path(), "idle", 300));
        let _current = lease(target, manifests.path(), "current", 0);

        evict_rules_target(target, 2 * MIB);

        assert!(target.join("debug/deps/idle-0123").exists());
    }

    #[test]
    fn a_host_another_process_is_using_is_never_evicted() {
        let target = tempfile::tempdir().unwrap();
        let manifests = tempfile::tempdir().unwrap();
        let target = target.path();
        host(target, "busy", MIB as usize);
        host(target, "idle", MIB as usize);
        // Another polint still holds the oldest host.
        let _busy = lease(target, manifests.path(), "busy", 300);
        drop(lease(target, manifests.path(), "idle", 200));
        let _current = lease(target, manifests.path(), "current", 0);

        evict_rules_target(target, 0);

        assert!(target.join("debug/deps/busy-0123").exists());
        assert!(target.join("debug/busy").exists());
        assert!(!target.join("debug/deps/idle-0123").exists());
    }

    #[test]
    fn a_lease_taken_after_its_host_was_evicted_is_a_fresh_lease() {
        let target = tempfile::tempdir().unwrap();
        let manifests = tempfile::tempdir().unwrap();
        let target = target.path();
        host(target, "evicted", MIB as usize);
        drop(lease(target, manifests.path(), "evicted", 300));
        let _current = lease(target, manifests.path(), "current", 0);
        evict_rules_target(target, 0);

        let again = lease(target, manifests.path(), "evicted", 0);
        let path = target.join(LEASES_DIR_NAME).join("evicted.lock");
        assert!(is_same_file(&again._file, &path));
        host(target, "evicted", MIB as usize);
        evict_rules_target(target, 0);
        assert!(
            target.join("debug/deps/evicted-0123").exists(),
            "the new lease protects the rebuilt host"
        );
    }

    fn store_entry(root: &Path, key: &str, blob: &str, seconds_ago: u64) {
        let path = root
            .join("rule-hosts")
            .join(&key[..2])
            .join(format!("{key}.json"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!(r#"{{"sha256":"{blob}"}}"#)).unwrap();
        age(&path, seconds_ago);
    }

    fn blob(root: &Path, hash: &str) -> PathBuf {
        root.join("blobs").join(&hash[..2]).join(hash)
    }

    #[test]
    fn the_store_drops_its_least_recently_used_entries_and_their_blobs() {
        let store = tempfile::tempdir().unwrap();
        let root = store.path();
        for hash in ["aa01", "bb02", "cc03"] {
            write(&blob(root, hash), MIB as usize);
        }
        write(&blob(root, "dd04"), MIB as usize); // named by no entry
        store_entry(root, "k1", "aa01", 400);
        store_entry(root, "k2", "bb02", 300);
        store_entry(root, "k3", "bb02", 50); // the same bytes, used recently
        store_entry(root, "k4", "cc03", 100);
        touch(&root.join("rule-hosts/k4/k4.json"));

        evict_store(root, 2 * MIB + MIB / 2);

        assert!(!blob(root, "dd04").exists(), "unnamed blobs go first");
        assert!(!blob(root, "aa01").exists(), "the oldest entry goes next");
        assert!(!root.join("rule-hosts/k1/k1.json").exists());
        assert!(
            blob(root, "bb02").exists(),
            "a blob another entry still names stays"
        );
        assert!(root.join("rule-hosts/k3/k3.json").exists());
        assert!(root.join("rule-hosts/k4/k4.json").exists());
        assert!(blob(root, "cc03").exists());
    }

    #[test]
    fn a_store_eviction_waits_for_no_publisher() {
        let store = tempfile::tempdir().unwrap();
        let root = store.path();
        write(&blob(root, "aa01"), MIB as usize);
        let publishing = store_publish_guard(root).unwrap();

        evict_store(root, 0);
        assert!(
            blob(root, "aa01").exists(),
            "a blob being published is not yet named by its entry"
        );

        drop(publishing);
        evict_store(root, 0);
        assert!(!blob(root, "aa01").exists());
    }
}
