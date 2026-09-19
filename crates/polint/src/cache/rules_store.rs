//! The machine-global store for compiled rule-host binaries: one place on a
//! machine for a build already done.
//!
//! A repository keeps its own cache under `.polint/cache`, and that cache is per
//! checkout. Every git worktree of one repository therefore recompiles the same
//! rule host from the same bytes, and a machine with a dozen worktrees pays the
//! same four-to-seven minute cold build a dozen times. The store is where that
//! build is shared.
//!
//! ## What makes sharing legitimate
//!
//! Nothing here decides a fact. Every entry is written under a key that already
//! names the complete input surface of the build that produced it — the
//! compiler and its flags, every manifest and lockfile cargo reads, and the
//! content of every rule source — and every entry records that key INSIDE
//! itself, so an entry can only be offered as the answer to the exact question
//! it answered. Same key, same inputs, same binary.
//!
//! That obligation is the caller's, and
//! [`inputs_are_the_same_from_every_checkout`] is where it is discharged: a rule
//! package whose build reads sources the fingerprint cannot name never reaches
//! the store at all. It is compiled locally instead, which is slower and never
//! wrong.
//!
//! A `path` dependency outside the rule package has no lockfile checksum, so its
//! bytes can change without changing the fingerprint. Relative escapes and
//! absolute paths are therefore both local-build-only. Paths that stay inside
//! the package are covered by the package input walk.
//!
//! ## Trust
//!
//! The store is **one user's state on one machine**, at the trust level of
//! `~/.cargo/registry`. polint creates it private to the invoking user (`0700`
//! on Unix) and never shares it between users, over a network, or with a remote.
//! Every restore re-hashes the bytes it copied before anything runs them.
//!
//! Content verification catches corruption — a truncated copy, a half-written
//! file, a bad disk. It cannot make a directory that OTHER users can write into
//! safe, because whoever can rewrite an entry can rewrite the hash beside it.
//! Point [`POLINT_CACHE_STORE_ENV`] at local storage only you can write, exactly
//! as you would a cargo registry — not at a share several machines mount. A rule
//! host is a native executable, and while the fingerprint covers the compiler
//! and its flags, it says nothing about the system libraries the machine that
//! built it linked against.
//!
//! ## Failure is a miss, never an error
//!
//! No read, write, or path resolution here can fail a run. A store that does not
//! exist, cannot be created, is full, or holds a corrupt entry is a *miss*: the
//! caller compiles the host the one way it always could. Deleting the whole
//! directory is always safe.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

/// Overrides where the store lives, or turns sharing off.
///
/// An absolute path names the store directory. `off`, `disabled`, or `none` (any
/// casing) turns sharing off, leaving each checkout with only its own
/// `.polint/cache`. Unset means the platform's user cache directory. Any other
/// value — a relative path, an empty string — is not a location polint can
/// resolve, so sharing is off for that run.
pub(crate) const POLINT_CACHE_STORE_ENV: &str = "POLINT_CACHE_STORE";

/// The schema tag every store entry and build stamp carries, and the first line
/// of every fingerprint. Bumping it retires both at once.
const SCHEMA: &str = "polint-rule-host-store-v1";

/// The directory polint keeps under the platform cache directory.
const CACHE_DIR_NAME: &str = "polint";

/// The store's directory inside polint's cache directory.
const STORE_DIR_NAME: &str = "store";

/// The values of [`POLINT_CACHE_STORE_ENV`] that turn sharing off.
const OFF_VALUES: [&str; 3] = ["off", "disabled", "none"];

/// The longest key the store will build a path from.
///
/// Every key polint writes is a sha256 hex digest, which is 64. The bound exists
/// so that a key is a file name and can never become anything else.
const MAX_KEY_LEN: usize = 128;

/// The file recording which rule host the pinned cargo target directory holds.
pub(crate) const STAMP_FILE_NAME: &str = "polint-store-stamp.json";

/// The digest recorded for a file that is not there.
const ABSENT: &str = "absent";

/// The value recorded for an environment variable that is not set, matching the
/// CI action's build-env digest.
const UNSET: &str = "<unset>";

/// The environment variables that change what the compiler produces from
/// unchanged sources, and the fingerprint line each one is recorded on.
///
/// Cargo reads all of them from the environment polint spawns it in, and none of
/// them leaves a trace in any file the fingerprint hashes.
const CARGO_FLAG_VARIABLES: [(&str, &str); 9] = [
    ("rustflags", "RUSTFLAGS"),
    ("cargo_encoded_rustflags", "CARGO_ENCODED_RUSTFLAGS"),
    ("cargo_build_rustflags", "CARGO_BUILD_RUSTFLAGS"),
    ("cargo_build_target", "CARGO_BUILD_TARGET"),
    ("cargo_incremental", "CARGO_INCREMENTAL"),
    ("rustc", "RUSTC"),
    ("rustc_bootstrap", "RUSTC_BOOTSTRAP"),
    ("rustc_wrapper", "RUSTC_WRAPPER"),
    ("rustc_workspace_wrapper", "RUSTC_WORKSPACE_WRAPPER"),
];

/// The names cargo reads a config from in a `.cargo/` directory: the current one
/// and the extensionless one it kept honoring for projects written before 1.39.
const CARGO_CONFIG_NAMES: [&str; 2] = ["config.toml", "config"];

/// Cargo-provided environment names whose values can identify a checkout.
///
/// `CARGO_MANIFEST_DIR` and `OUT_DIR` are absolute paths supplied to compiled
/// code. `CARGO` and `RUSTC` name the tool executables and can likewise carry
/// paths; their short names include a trailing quote or colon to avoid matching
/// ordinary prose. `CARGO_PKG_*` values are deliberately absent because Cargo
/// derives them from checkout-independent package metadata.
const CHECKOUT_PATH_TOKENS: [&[u8]; 6] = [
    b"CARGO_MANIFEST_DIR",
    b"OUT_DIR",
    b"CARGO\"",
    b"CARGO:",
    b"RUSTC\"",
    b"RUSTC:",
];

/// The top-level cargo config keys that make direct, cross-checkout reuse unsafe.
///
/// `patch`, `replace`, `paths`, and `source` can send a package to bytes no
/// manifest or lockfile records. `env` changes the environment Cargo supplies to
/// the build and the process it runs, while a restored host is executed directly.
/// `include` is here because this never follows one: a config whose full content
/// cannot be established is never one this claims to have proven.
const CARGO_UNSHAREABLE_KEYS: [&str; 6] = ["patch", "replace", "paths", "source", "env", "include"];

/// A machine-global content-addressed store rooted at one directory.
#[derive(Debug, Clone)]
pub(crate) struct RuleHostStore {
    root: PathBuf,
}

/// An exclusive lease on one checkout's rule-host target directory.
///
/// The lease is held through restore/build and host execution. That prevents two
/// polint processes from validating different binaries and then racing to run
/// whichever one was moved into the shared destination last. Failure to acquire
/// it is a cache miss: the caller uses Cargo's original path instead.
pub(crate) struct TargetLock {
    _file: File,
}

impl TargetLock {
    pub(crate) fn acquire(target_dir: &Path) -> Option<Self> {
        std::fs::create_dir_all(target_dir).ok()?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(target_dir.join("polint-store.lock"))
            .ok()?;
        file.lock().ok()?;
        Some(Self { _file: file })
    }
}

impl RuleHostStore {
    /// The store this environment asks for, or `None` when sharing is off or has
    /// no location.
    ///
    /// This is the one place polint reads [`POLINT_CACHE_STORE_ENV`]. Everything
    /// below it takes the resolved store as an argument, so a library call can
    /// never depend on the ambient environment and a test can never reach the
    /// developer's own store by accident.
    pub(crate) fn from_env() -> Option<Self> {
        let root = resolve_root(
            std::env::var(POLINT_CACHE_STORE_ENV).ok().as_deref(),
            std::env::var("XDG_CACHE_HOME").ok().as_deref(),
            std::env::var("LOCALAPPDATA").ok().as_deref(),
            std::env::var("HOME").ok().as_deref(),
        )?;
        Some(Self::at(root))
    }

    /// A store rooted at an explicit directory.
    pub(crate) fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The bytes recorded for `key`, or `None` when nothing is recorded.
    fn read(&self, key: &str) -> Option<Vec<u8>> {
        std::fs::read(self.entry_path(key)?).ok()
    }

    /// Record `bytes` for `key`, doing nothing at all if that is not possible.
    fn publish(&self, key: &str, bytes: &[u8]) {
        let Some(path) = self.entry_path(key) else {
            return;
        };
        let _ = write_atomically(&path, bytes);
    }

    /// Delete the entry for `key`, which a caller does when it proved the entry
    /// is not usable.
    fn discard(&self, key: &str) {
        if let Some(path) = self.entry_path(key) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Where the blob whose sha256 hex digest is `hash` lives.
    ///
    /// A blob is named by its own content, so two publishers of different bytes
    /// never write the same file and a published blob is never rewritten in
    /// place.
    fn blob_path(&self, hash: &str) -> Option<PathBuf> {
        Some(self.root.join("blobs").join(shard(hash)?).join(hash))
    }

    /// Copy `source` into the store under its content hash, doing nothing if
    /// that is not possible.
    ///
    /// The caller has already hashed `source` — that hash is what the entry
    /// pointing at this blob records — so the store takes it rather than reading
    /// the file a second time.
    ///
    /// A blob that is already there is left alone: it is named by its own
    /// content, so re-copying it could only produce the same bytes. If it has
    /// since been corrupted, the restore that reads it removes it, and the
    /// publish after the next build writes it again.
    fn publish_blob(&self, hash: &str, source: &Path) -> bool {
        let Some(path) = self.blob_path(hash) else {
            return false;
        };
        if path.is_file() {
            return true;
        }
        let Some(parent) = path.parent() else {
            return false;
        };
        if create_dir_all_private(parent).is_err() {
            return false;
        }
        let Ok(temporary) = tempfile::Builder::new()
            .prefix(".polint-blob-")
            .tempfile_in(parent)
        else {
            return false;
        };
        if std::fs::copy(source, temporary.path()).is_err() {
            return false;
        }
        temporary.persist_noclobber(&path).is_ok() || path.is_file()
    }

    /// The file an entry for `key` is stored in, or `None` when `key` is not a
    /// key this store writes.
    fn entry_path(&self, key: &str) -> Option<PathBuf> {
        Some(
            self.root
                .join("rule-hosts")
                .join(shard(key)?)
                .join(format!("{key}.json")),
        )
    }
}

/// What the store records about one compiled rule host: which blob is the
/// binary, where it belongs under the cargo target directory, and what it must
/// hash to.
///
/// The key is recorded INSIDE the entry as well as in its file name, and the
/// polint version beside it: an entry must be able to prove which question it
/// answers, so that a rewritten or restored store directory cannot present one
/// answer as another. A store holds entries from every polint version that ever
/// wrote to it, and a version that does not match this one is a miss.
#[derive(Debug, Serialize, Deserialize)]
struct StoreEntry {
    schema: String,
    key: String,
    polint_version: String,
    /// The binary's path relative to the rule host's cargo target directory,
    /// such as `release/polint-local-rules`.
    target_relative_path: String,
    sha256: String,
    len: u64,
}

/// The recorded identity of the rule host already in a cargo target directory.
///
/// This is what lets a warm run skip Cargo build/run: the fingerprint names the
/// build's whole input surface, so a stamp that still matches means the binary
/// beside it is the one Cargo would produce. The length and digest are re-read
/// from disk on every check, so a truncated or replaced binary is never run on
/// the strength of a stale record.
///
/// One target directory holds one stamp. A repository with several rule packages
/// pointed at the same target directory therefore keeps the stamp of whichever
/// package built last; the others find a fingerprint that is not theirs, which
/// is a miss like any other and sends them to the store or to cargo.
#[derive(Debug, Serialize, Deserialize)]
struct HostStamp {
    schema: String,
    fingerprint: String,
    target_relative_path: String,
    sha256: String,
    len: u64,
}

/// Whether `target_dir` holds a stamp at all.
///
/// Asked before a fingerprint is computed: a checkout with no stamp and no store
/// to look in has nothing to compare a key against, and resolving the compiler
/// to build one would be work spent on a question nobody asked.
pub(crate) fn is_stamped(target_dir: &Path) -> bool {
    target_dir.join(STAMP_FILE_NAME).is_file()
}

/// The rule host recorded by the stamp in `target_dir`, when it is still the one
/// `fingerprint` asks for and the bytes on disk are still the recorded bytes.
///
/// `None` is always "obtain the host some other way", never an error.
pub(crate) fn binary_recorded_by_stamp(target_dir: &Path, fingerprint: &str) -> Option<PathBuf> {
    let bytes = std::fs::read(target_dir.join(STAMP_FILE_NAME)).ok()?;
    let stamp = serde_json::from_slice::<HostStamp>(&bytes).ok()?;
    if stamp.schema != SCHEMA || stamp.fingerprint != fingerprint {
        return None;
    }
    let binary = target_dir.join(relative_target_path(&stamp.target_relative_path)?);
    let (len, sha256) = file_identity(&binary).ok()?;
    (len == stamp.len && sha256 == stamp.sha256).then_some(binary)
}

/// Record that `binary` is the rule host `fingerprint` names.
///
/// Best effort: a stamp that cannot be written only costs the next run the fast
/// path.
fn write_stamp(target_dir: &Path, fingerprint: &str, entry: &StoreEntry) {
    let stamp = HostStamp {
        schema: SCHEMA.to_string(),
        fingerprint: fingerprint.to_string(),
        target_relative_path: entry.target_relative_path.clone(),
        sha256: entry.sha256.clone(),
        len: entry.len,
    };
    if let Ok(bytes) = serde_json::to_vec(&stamp) {
        let _ = write_atomically(&target_dir.join(STAMP_FILE_NAME), &bytes);
    }
}

/// Copy the stored rule host for `fingerprint` into `target_dir`, or answer
/// `None`.
///
/// `None` is always "build it here instead", never an error: an absent entry, an
/// entry this polint cannot read, a missing blob, a copy that fails, or bytes
/// that do not hash to what the entry recorded all mean the same thing to the
/// caller. An entry that is *provably* wrong — the wrong schema, the wrong key,
/// the wrong polint version, or a blob that does not match its own record — is
/// deleted on the way out, because it can never become right.
pub(crate) fn restore(
    store: &RuleHostStore,
    target_dir: &Path,
    fingerprint: &str,
) -> Option<PathBuf> {
    let bytes = store.read(fingerprint)?;
    let entry = serde_json::from_slice::<StoreEntry>(&bytes)
        .ok()
        .filter(|entry| {
            entry.schema == SCHEMA
                && entry.key == fingerprint
                && entry.polint_version == env!("CARGO_PKG_VERSION")
        });
    let Some(entry) = entry else {
        store.discard(fingerprint);
        return None;
    };
    let Some(relative) = relative_target_path(&entry.target_relative_path) else {
        store.discard(fingerprint);
        return None;
    };
    let Some(blob) = store.blob_path(&entry.sha256) else {
        store.discard(fingerprint);
        return None;
    };

    // The binary is copied to a temporary name beside its destination and
    // verified BEFORE it is moved into place, so a corrupt entry never lands at
    // the path polint is about to execute. Renaming rather than writing over the
    // destination is also what makes replacing a host this machine is currently
    // running safe: on Unix the running process keeps its own inode, and on
    // Windows replacement is atomic when the destination is replaceable; if the
    // OS refuses to replace a running executable, persistence fails as a miss.
    let destination = target_dir.join(relative);
    let parent = destination.parent()?;
    std::fs::create_dir_all(parent).ok()?;
    let temporary = tempfile::Builder::new()
        .prefix(".polint-restore-")
        .tempfile_in(parent)
        .ok()?;
    let restored = std::fs::copy(&blob, temporary.path())
        .ok()
        .and_then(|_| file_identity(temporary.path()).ok())
        .is_some_and(|(len, sha256)| len == entry.len && sha256 == entry.sha256);
    if !restored {
        // The blob did not match the entry that named it, so neither is usable
        // by anyone.
        let _ = std::fs::remove_file(&blob);
        store.discard(fingerprint);
        return None;
    }
    if make_executable(temporary.path()).is_err() || temporary.persist(&destination).is_err() {
        return None;
    }
    write_stamp(target_dir, fingerprint, &entry);
    Some(destination)
}

/// Record a freshly built rule host: stamp it for this checkout, and publish it
/// to `store` for every other one.
///
/// Best effort in every half. The blob is written before the entry that names
/// it, so an entry never points at a blob that was never durable. Two publishers
/// of the same fingerprint can produce different bytes — cargo's output is not
/// byte-reproducible across target directories — so the blob is named by its own
/// content and the entry, not the blob, is what they race on. Either entry is
/// complete and correct.
pub(crate) fn record(
    store: Option<&RuleHostStore>,
    target_dir: &Path,
    fingerprint: &str,
    binary: &Path,
) {
    let Some(relative) = binary
        .strip_prefix(target_dir)
        .ok()
        .and_then(|relative| relative.to_str())
        .map(|relative| relative.replace('\\', "/"))
    else {
        return;
    };
    if relative_target_path(&relative).is_none() {
        return;
    }
    let Ok((len, sha256)) = file_identity(binary) else {
        return;
    };
    let entry = StoreEntry {
        schema: SCHEMA.to_string(),
        key: fingerprint.to_string(),
        polint_version: env!("CARGO_PKG_VERSION").to_string(),
        target_relative_path: relative,
        sha256,
        len,
    };
    write_stamp(target_dir, fingerprint, &entry);
    if let Some(store) = store
        && store.publish_blob(&entry.sha256, binary)
        && let Ok(bytes) = serde_json::to_vec(&entry)
    {
        store.publish(fingerprint, &bytes);
    }
}

/// `recorded` as a path under a cargo target directory, or `None` when it is not
/// one.
///
/// The value is read from a file another process wrote, so it is validated
/// rather than trusted: a relative path with no `..` and no root is the only
/// shape that stays inside the directory polint pins.
fn relative_target_path(recorded: &str) -> Option<PathBuf> {
    if recorded.is_empty() {
        return None;
    }
    let path = Path::new(recorded);
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
        .then(|| path.to_path_buf())
}

/// The subdirectory a key or hash is filed under, or `None` when it is not a hex
/// digest.
///
/// Keys are validated here rather than trusted: a key reaches the store from a
/// hash function today, and restricting it to hex is what keeps that true no
/// matter what a future caller passes. It also keeps one directory from
/// collecting every entry a machine ever wrote.
fn shard(key: &str) -> Option<String> {
    if key.len() < 2 || key.len() > MAX_KEY_LEN || !key.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    Some(key.get(..2)?.to_string())
}

/// Publish `bytes` at `path` so a reader sees either the whole file or no file.
///
/// The temporary has a randomized name, so concurrent writers never collide,
/// and persistence happens inside the destination directory so replacement is
/// atomic on every supported platform.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(std::io::Error::other(
            "a store entry has no parent directory",
        ));
    };
    create_dir_all_private(parent)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".polint-entry-")
        .tempfile_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.flush()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Create `dir` and its parents, private to the invoking user.
///
/// polint owns the directories it creates under the store root, so it creates
/// them `0700`: the store holds a binary this machine will execute, and a
/// directory another local user can write into is a directory that decides what
/// runs. A directory that already exists keeps whatever mode it has — that one
/// is the user's own to choose.
#[cfg(unix)]
fn create_dir_all_private(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_dir_all_private(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Make a restored rule host runnable.
///
/// A copy carries the blob's mode, and the blob carries the mode of the binary
/// cargo produced, so this is normally already true; it is set anyway because a
/// store whose files arrived some other way — a restored backup, a copy through
/// a tool that dropped the bit — would otherwise produce a host that cannot be
/// executed at all.
#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Where the store lives, given what the environment says — a pure function of
/// its four readings.
///
/// Pure so the whole resolution matrix is testable without touching the process
/// environment, which is also what keeps the tests hermetic and safe to run in
/// parallel.
fn resolve_root(
    override_value: Option<&str>,
    xdg_cache_home: Option<&str>,
    local_app_data: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(value) = override_value {
        let trimmed = value.trim();
        if OFF_VALUES
            .iter()
            .any(|off| trimmed.eq_ignore_ascii_case(off))
        {
            return None;
        }
        let path = Path::new(trimmed);
        // A relative store would follow the process's working directory, which
        // is not a location: the same command run from two directories would
        // mean two stores. Only an absolute path names one.
        return path.is_absolute().then(|| path.to_path_buf());
    }
    Some(
        platform_cache_dir(xdg_cache_home, local_app_data, home)?
            .join(CACHE_DIR_NAME)
            .join(STORE_DIR_NAME),
    )
}

/// The platform's user cache directory: `%LOCALAPPDATA%`, `~/Library/Caches`, or
/// `$XDG_CACHE_HOME`.
///
/// Each branch is the convention of the platform it names, taken as an idea
/// rather than through a dependency: a handful of `std::env` readings answer it,
/// and a directory layout is not a product fact worth another crate in the tree.
fn platform_cache_dir(
    xdg_cache_home: Option<&str>,
    local_app_data: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    if cfg!(windows) {
        return absolute(local_app_data);
    }
    if cfg!(target_os = "macos") {
        return Some(absolute(home)?.join("Library").join("Caches"));
    }
    // The XDG base directory specification says a relative $XDG_CACHE_HOME is
    // invalid and must be ignored, which lands on the same `~/.cache` the unset
    // case uses.
    absolute(xdg_cache_home).or_else(|| Some(absolute(home)?.join(".cache")))
}

/// `value` as a path, when it is a non-empty absolute one.
fn absolute(value: Option<&str>) -> Option<PathBuf> {
    let path = Path::new(value?.trim());
    (path.is_absolute()).then(|| path.to_path_buf())
}

/// The compiler and cargo a rule-host build would use, read once and passed in.
///
/// Resolving the compiler beats hashing `rust-toolchain.toml` alone: it also
/// covers a pinned toolchain override and a floating `stable` that moved. Both
/// readings are taken in the repository root, because that is where polint spawns
/// cargo and therefore where rustup resolves a toolchain file.
#[derive(Debug, Clone)]
pub(crate) struct ToolchainIdentity {
    /// The full `rustc -vV` output.
    pub(crate) rustc: String,
    /// The `cargo -V` output.
    pub(crate) cargo: String,
    /// The toolchain override in effect, if any.
    pub(crate) rustup_toolchain: Option<String>,
}

/// Everything about a rule-host build that is not a file: the compiler, the
/// profile, the target platform, and the flags cargo takes from its environment.
///
/// The environment is read once, when this is built, so that everything below it
/// is a function of its arguments — the same discipline [`resolve_root`] follows,
/// and what lets the fingerprint's behavior be tested without a process-wide
/// mutation that other tests would see.
#[derive(Debug, Clone)]
pub(crate) struct BuildEnvironment {
    /// The resolved cargo profile: `release`, `dev`, or a named profile.
    profile: String,
    toolchain: ToolchainIdentity,
    /// The value of each [`CARGO_FLAG_VARIABLES`] entry, in that order.
    cargo_flags: Vec<String>,
    /// Target- and profile-specific Cargo environment overrides, sorted by name.
    cargo_overrides: Vec<(String, String)>,
    /// Cargo's user-wide configuration/source root for this build.
    cargo_home: Option<PathBuf>,
}

impl BuildEnvironment {
    /// The build environment of this process, for a host compiled under
    /// `profile` by `toolchain`.
    pub(crate) fn new(profile: String, toolchain: ToolchainIdentity) -> Option<Self> {
        let mut cargo_flags = Vec::with_capacity(CARGO_FLAG_VARIABLES.len());
        for (_, variable) in CARGO_FLAG_VARIABLES {
            let value = match std::env::var_os(variable) {
                Some(value) => value.into_string().ok()?,
                None => UNSET.to_string(),
            };
            cargo_flags.push(value);
        }
        let mut cargo_overrides = Vec::new();
        for (name, value) in std::env::vars_os() {
            let Some(name) = name.to_str() else {
                continue;
            };
            if (name.starts_with("CARGO_PROFILE_") || name.starts_with("CARGO_TARGET_"))
                && name != "CARGO_TARGET_DIR"
            {
                cargo_overrides.push((name.to_string(), value.into_string().ok()?));
            }
        }
        cargo_overrides.sort();
        Some(Self {
            profile,
            toolchain,
            cargo_flags,
            cargo_overrides,
            cargo_home: cargo_home(),
        })
    }
}

/// The two digests of a rule-host build's inputs.
///
/// A build is identified by everything it reads, but a build also WRITES one of
/// those inputs: cargo creates or updates the rule package's lockfile as part of
/// compiling it. The complete digest is therefore taken again once the build has
/// finished — that is the identity the host actually has, and the one every
/// later run will compute — while the authored digest, which leaves the
/// lockfiles out, brackets the build: unchanged across it means the sources
/// compiled are the sources being recorded, and a rule edited while cargo was
/// running is never published as if it had been compiled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuildFingerprint {
    /// Every input, lockfiles included. This is the store key and what a stamp
    /// records.
    pub(crate) complete: String,
    /// Every input a build cannot rewrite for itself.
    pub(crate) authored: String,
}

/// Both digests over one input surface, folded in a single pass.
struct FingerprintDigest {
    complete: Sha256,
    authored: Sha256,
}

impl FingerprintDigest {
    fn new() -> Self {
        let mut digest = Self {
            complete: Sha256::new(),
            authored: Sha256::new(),
        };
        for hasher in [&mut digest.complete, &mut digest.authored] {
            hasher.update(SCHEMA.as_bytes());
            hasher.update(b"\n");
        }
        digest
    }

    /// One `key=value` line, terminated by a newline — the same shape the CI
    /// action's cache-input digests take.
    fn line(&mut self, key: &str, value: &str) {
        Self::write(&mut self.complete, key, value);
        Self::write(&mut self.authored, key, value);
    }

    /// A lockfile: an input to the build that the build may also write.
    fn lockfile_line(&mut self, key: &str, value: &str) {
        Self::write(&mut self.complete, key, value);
    }

    fn write(hasher: &mut Sha256, key: &str, value: &str) {
        hasher.update(key.as_bytes());
        hasher.update(b"=");
        hasher.update(value.as_bytes());
        hasher.update(b"\n");
    }

    fn finish(self) -> BuildFingerprint {
        BuildFingerprint {
            complete: hex(&self.complete.finalize()),
            authored: hex(&self.authored.finalize()),
        }
    }
}

/// The digests over every input a rule-host build reads, or `None` when the
/// surface could not be read.
///
/// Everything is hashed from bytes. A modification time says when a file was
/// written, not what is in it, and a restored checkout or a same-length rewrite
/// would pass as unchanged.
///
/// `None` means "no key for this build", which sends the caller to cargo. That
/// is the answer for every failure here, because a key computed over a surface
/// this run could not read would name a build it cannot stand for.
pub(crate) fn build_fingerprint(
    repo_root: &Path,
    rule_pkg_dir: &Path,
    environment: &BuildEnvironment,
) -> Option<BuildFingerprint> {
    let mut digest = FingerprintDigest::new();
    digest.line("polint", env!("CARGO_PKG_VERSION"));
    digest.line("profile", &environment.profile);
    digest.line("os", std::env::consts::OS);
    digest.line("arch", std::env::consts::ARCH);
    digest.line("rustc", &environment.toolchain.rustc);
    digest.line("cargo", &environment.toolchain.cargo);
    digest.line(
        "rustup_toolchain",
        environment
            .toolchain
            .rustup_toolchain
            .as_deref()
            .unwrap_or(UNSET),
    );
    for ((key, _), value) in CARGO_FLAG_VARIABLES.iter().zip(&environment.cargo_flags) {
        digest.line(key, value);
    }
    for (name, value) in &environment.cargo_overrides {
        digest.line(&format!("cargo_override_{name}"), value);
    }
    // The files cargo and rustup discover from the directory polint spawns cargo
    // in, and then the rule package's own. Both lockfiles matter: the one beside
    // the manifest cargo actually uses pins every registry and git dependency by
    // checksum, which is what lets a digest over these files stand for the whole
    // of what the build compiles.
    digest.line(
        "repo_cargo_toml",
        &optional_file_digest(&repo_root.join("Cargo.toml"))?,
    );
    digest.lockfile_line(
        "repo_cargo_lock",
        &optional_file_digest(&repo_root.join("Cargo.lock"))?,
    );
    for (key, relative) in [
        ("rust_toolchain_toml", "rust-toolchain.toml"),
        ("rust_toolchain", "rust-toolchain"),
    ] {
        digest.line(key, &optional_file_digest(&repo_root.join(relative))?);
    }
    for (index, config) in
        cargo_config_files(rule_pkg_dir, repo_root, environment.cargo_home.clone())
            .iter()
            .enumerate()
    {
        digest.line(
            &format!("cargo_config_{index:03}"),
            &optional_file_digest(config)?,
        );
    }
    for (index, manifest) in ancestor_files(rule_pkg_dir, "Cargo.toml")?
        .iter()
        .enumerate()
    {
        digest.line(
            &format!("ancestor_manifest_{index:03}"),
            &file_digest(manifest).ok()?,
        );
    }
    for (index, lockfile) in ancestor_files(rule_pkg_dir, "Cargo.lock")?
        .iter()
        .enumerate()
    {
        digest.lockfile_line(
            &format!("ancestor_lockfile_{index:03}"),
            &file_digest(lockfile).ok()?,
        );
    }
    digest.line("package", &package_label(repo_root, rule_pkg_dir));
    digest.line(
        "package_cargo_toml",
        &optional_file_digest(&rule_pkg_dir.join("Cargo.toml"))?,
    );
    digest.lockfile_line(
        "package_cargo_lock",
        &optional_file_digest(&rule_pkg_dir.join("Cargo.lock"))?,
    );
    digest.line(
        "embeds_checkout_paths",
        if sources_embed_checkout_paths(rule_pkg_dir) {
            "true"
        } else {
            "false"
        },
    );
    for (relative, source) in rule_inputs(rule_pkg_dir)? {
        digest.line(&relative, &source);
    }
    digest.line(
        "bin",
        &manifest_bin_names(&rule_pkg_dir.join("Cargo.toml"))?,
    );

    Some(digest.finish())
}

/// How the rule package is named in a fingerprint: its path relative to the
/// repository root, or the path as given when it is somewhere else entirely.
fn package_label(repo_root: &Path, rule_pkg_dir: &Path) -> String {
    rule_pkg_dir
        .strip_prefix(repo_root)
        .unwrap_or(rule_pkg_dir)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Every potential build input in the package, as `<package-relative path>` and
/// digest pairs ordered by path.
///
/// The whole package is walked rather than just `src/`: `include_bytes!`, a
/// build script, or a nested path dependency can consume any file. Files a
/// binary does not compile cost a rebuild when they change and can never cause a
/// stale host to run. Cargo lockfiles are folded separately because Cargo may
/// create or update them during the build.
///
/// `None` when a directory or a file that is there cannot be read: the set would
/// then describe fewer sources than the build compiles.
fn rule_inputs(rule_pkg_dir: &Path) -> Option<Vec<(String, String)>> {
    let files = package_files(rule_pkg_dir, &|path| {
        path.file_name().is_none_or(|name| name != "Cargo.lock")
    })?;
    let mut out = Vec::with_capacity(files.len());
    for path in files {
        let relative = path
            .strip_prefix(rule_pkg_dir)
            .ok()?
            .to_str()?
            .replace('\\', "/");
        out.push((relative, file_digest(&path).ok()?));
    }
    // Byte order over the package-relative path, which is the same order the CI
    // action's `LC_ALL=C sort` produces for the same set.
    out.sort();
    Some(out)
}

/// Whether Rust compiled for the rule package may read checkout-specific Cargo
/// environment values.
///
/// This is intentionally a broad byte-token gate rather than a Rust parser. A
/// token in a comment or inert string opts one package out of machine-global
/// sharing, which costs a local build but can never restore the wrong host. The
/// scan covers Cargo's conventional Rust target trees and a root `build.rs`.
/// An unreadable or structurally surprising source tree also fails closed.
fn sources_embed_checkout_paths(rule_pkg_dir: &Path) -> bool {
    let build_script = rule_pkg_dir.join("build.rs");
    match std::fs::symlink_metadata(&build_script) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return true,
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => return true,
        Ok(_) => {
            if source_file_embeds_checkout_paths(&build_script).unwrap_or(true) {
                return true;
            }
        }
    }

    ["src", "benches", "examples", "tests"]
        .iter()
        .map(|directory| rule_pkg_dir.join(directory))
        .any(|directory| match std::fs::symlink_metadata(&directory) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => true,
            Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => true,
            Ok(_) => rust_tree_embeds_checkout_paths(&directory).unwrap_or(true),
        })
}

/// Scan every regular `*.rs` file below one Cargo source directory.
fn rust_tree_embeds_checkout_paths(root: &Path) -> Option<bool> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).ok()? {
            let entry = entry.ok()?;
            let kind = entry.file_type().ok()?;
            if kind.is_symlink() {
                return None;
            }
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file()
                && path.extension().is_some_and(|extension| extension == "rs")
                && source_file_embeds_checkout_paths(&path)?
            {
                return Some(true);
            }
        }
    }
    Some(false)
}

/// Read one Rust source as bytes and look for the conservative token family.
fn source_file_embeds_checkout_paths(path: &Path) -> Option<bool> {
    let bytes = std::fs::read(path).ok()?;
    Some(CHECKOUT_PATH_TOKENS.iter().any(|token| {
        bytes
            .windows(token.len())
            .any(|candidate| candidate == *token)
    }))
}

/// Every file under the rule package that `keep` accepts.
///
/// `target/` at the package root is cargo's own output rather than a source, and
/// symlinked directories are not followed: a walk that leaves the package cannot
/// answer a question about what is inside it. `None` when a directory cannot be
/// read, because a partial walk describes fewer inputs than the build has.
///
/// One walk with one set of exclusions serves both questions asked of a rule
/// package — what it compiles, and where its manifests point — so the two can
/// never disagree about what "in the package" means.
fn package_files(rule_pkg_dir: &Path, keep: &dyn Fn(&Path) -> bool) -> Option<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut pending = vec![rule_pkg_dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).ok()? {
            let entry = entry.ok()?;
            let kind = entry.file_type().ok()?;
            if kind.is_symlink() {
                // Cargo follows a symlink used as a module, build input, or path
                // dependency. This walk deliberately does not leave the package,
                // so it cannot prove a complete input set when one is present.
                return None;
            }
            let path = entry.path();
            if kind.is_dir() {
                if dir == rule_pkg_dir && path.file_name().is_some_and(|name| name == "target") {
                    continue;
                }
                pending.push(path);
            } else if kind.is_file() {
                if keep(&path) {
                    out.push(path);
                }
            } else {
                // Sockets, devices, and FIFOs cannot be safely fingerprinted as
                // finite package inputs. Cargo handles the package locally.
                return None;
            }
        }
    }
    Some(out)
}

/// Existing Cargo files from the package directory through every ancestor.
///
/// Cargo may attach a package to a workspace above the repository root, and
/// workspace manifests and lockfiles affect dependency resolution and profiles.
fn ancestor_files(start: &Path, name: &str) -> Option<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in start.ancestors().map(|ancestor| ancestor.join(name)) {
        match std::fs::metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
            Ok(metadata) if metadata.is_file() => files.push(path),
            Ok(_) => return None,
        }
    }
    Some(files)
}

/// The binary targets a manifest declares, sorted and comma-joined.
///
/// A package with no `[[bin]]` table builds `src/main.rs` under the package
/// name, so that is the answer there. `None` when the manifest cannot be read or
/// parsed, which is a build that will fail for its own reasons.
fn manifest_bin_names(manifest: &Path) -> Option<String> {
    let value = std::fs::read_to_string(manifest)
        .ok()
        .and_then(|text| toml::from_str::<toml::Value>(&text).ok())?;
    let mut names = value
        .get("bin")
        .and_then(toml::Value::as_array)
        .map(|bins| {
            bins.iter()
                .filter_map(|bin| bin.get("name")?.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if names.is_empty() {
        names = value
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
            .map(|name| vec![name.to_string()])?;
    }
    names.sort();
    Some(names.join(","))
}

/// The digest of a file that may legitimately not exist.
///
/// A file that is not there is recorded as absent, which is a value like any
/// other. A file that IS there and cannot be read answers `None`, because
/// recording it as absent would give a surface polint could not read the same
/// name as one that does not have it.
fn optional_file_digest(path: &Path) -> Option<String> {
    match file_digest(path) {
        Ok(digest) => Some(digest),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Some(ABSENT.to_string()),
        Err(_) => None,
    }
}

/// The sha256 of a file's bytes.
fn file_digest(path: &Path) -> std::io::Result<String> {
    Ok(file_identity(path)?.1)
}

/// A file's length and the sha256 of its bytes, read in one streaming pass so a
/// rule host of any size costs one buffer rather than its own size in memory.
fn file_identity(path: &Path) -> std::io::Result<(u64, String)> {
    if !std::fs::metadata(path)?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "fingerprinted input is not a regular file",
        ));
    }
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut len = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        len += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((len, hex(&hasher.finalize())))
}

/// Lowercase hex, the form every key, digest, and blob name in the store takes.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Whether a fingerprint over this rule package names the same bytes read from
/// any checkout on this machine.
///
/// The fingerprint hashes the package's manifests, lockfiles, and sources, which
/// is the whole of what the build compiles as long as every dependency comes
/// from a registry or a git revision: the lockfile pins those and cargo verifies
/// them by checksum. Three constructs break that.
///
/// A path dependency does it from inside a manifest: an absolute path names
/// sources outside the fingerprint, while a relative `../helpers` names a
/// different directory in every checkout. A relative path that stays inside the
/// package is hashed by content like every other input.
///
/// A cargo config does it from outside the tree altogether: dependency source
/// redirects send a build to bytes no manifest and no lockfile records. Config
/// `[env]` and target runners also change how Cargo executes the built host, so a
/// direct restored-host execution cannot reproduce their behavior.
///
/// Rust sources and build scripts can also embed the checkout-specific paths
/// Cargo exposes as compile-time environment values. A conservative byte-token
/// scan makes any package that mentions those values local-only.
///
/// Anything unreadable or unparseable answers `false`: a surface that cannot be
/// proven is never shared. A false yes costs a local build; a false no shares the
/// wrong binary.
pub(crate) fn inputs_are_the_same_from_every_checkout(
    rule_pkg_dir: &Path,
    repo_root: &Path,
) -> bool {
    shareable_with_cargo_home(rule_pkg_dir, repo_root, cargo_home())
}

/// [`inputs_are_the_same_from_every_checkout`] with cargo's home directory given
/// rather than read, so the answer is a function of its arguments.
fn shareable_with_cargo_home(
    rule_pkg_dir: &Path,
    repo_root: &Path,
    cargo_home: Option<PathBuf>,
) -> bool {
    every_manifest_path_stays_put(rule_pkg_dir)
        && !a_cargo_config_prevents_direct_reuse(rule_pkg_dir, repo_root, cargo_home)
        && !sources_embed_checkout_paths(rule_pkg_dir)
}

/// Whether every `path` declared by every manifest under the rule package names
/// the same directory from every checkout.
///
/// Every manifest is read, not only the package's own: a `path` dependency can
/// point at a directory inside the package that has a manifest of its own, and
/// that manifest's pointers leave the package just as easily.
fn every_manifest_path_stays_put(rule_pkg_dir: &Path) -> bool {
    let manifests = package_files(rule_pkg_dir, &|path| {
        path.file_name().is_some_and(|name| name == "Cargo.toml")
    });
    manifests.is_some_and(|mut manifests| {
        let Some(workspace_manifests) = ancestor_files(rule_pkg_dir, "Cargo.toml") else {
            return false;
        };
        for workspace_manifest in workspace_manifests {
            if !manifests.contains(&workspace_manifest) {
                manifests.push(workspace_manifest);
            }
        }
        manifests
            .iter()
            .all(|manifest| manifest_paths_stay_put(rule_pkg_dir, manifest))
    })
}

/// Whether every `path` one manifest declares names the same directory from
/// every checkout.
fn manifest_paths_stay_put(rule_pkg_dir: &Path, manifest: &Path) -> bool {
    let Some(manifest_dir) = manifest.parent() else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(manifest) else {
        return false;
    };
    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        return false;
    };
    let mut declared = Vec::new();
    collect_declared_paths(&value, &mut declared);
    if let Some(workspace) = value
        .get("package")
        .and_then(|package| package.get("workspace"))
        .and_then(toml::Value::as_str)
    {
        declared.push(workspace.to_string());
    }
    declared.iter().all(|path| {
        let path = Path::new(path);
        if path.is_absolute() {
            // Cargo's lockfile carries no checksum for a path dependency. Even
            // though the path names one machine-local directory, its contents
            // can change while this package's fingerprint stays unchanged.
            return false;
        }
        let resolved = resolved_lexically(manifest_dir, path);
        let Ok(package) = std::fs::canonicalize(rule_pkg_dir) else {
            return false;
        };
        let Ok(resolved) = std::fs::canonicalize(resolved) else {
            return false;
        };
        resolved.starts_with(package)
    })
}

/// Every string filed under a `path` key anywhere in a manifest.
///
/// Taken over the whole document rather than a list of dependency tables,
/// because a manifest has many of them — `[dependencies]`, `[dev-dependencies]`,
/// `[build-dependencies]`, one pair per `[target.'cfg(…)']`, `[patch.*]`,
/// `[workspace.dependencies]` — and a table this misses is a pointer that leaves
/// the fingerprint. The keys it also collects (`[[bin]] path`, `[lib] path`) name
/// files inside the package, which is the answer they should give anyway.
fn collect_declared_paths(value: &toml::Value, out: &mut Vec<String>) {
    match value {
        toml::Value::Table(table) => {
            for (key, nested) in table {
                if key == "path"
                    && let Some(text) = nested.as_str()
                {
                    out.push(text.to_string());
                }
                collect_declared_paths(nested, out);
            }
        }
        toml::Value::Array(items) => {
            for item in items {
                collect_declared_paths(item, out);
            }
        }
        _ => {}
    }
}

/// `path` joined to `base` and reduced without touching the filesystem.
///
/// Lexical reduction happens before canonicalization so `..` is resolved from
/// the manifest directory. The caller then canonicalizes the result to catch a
/// symlink that leaves the package.
fn resolved_lexically(base: &Path, path: &Path) -> PathBuf {
    let mut out = base.to_path_buf();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Whether a cargo config that applies to this build prevents direct reuse.
///
/// It never asks WHICH package is redirected. A `paths` entry names directories
/// rather than packages and could only be attributed by reading the manifests it
/// points at, a `replace` key is a package-id spec that would have to be parsed
/// to be attributed, and a patch of any crate in the host's graph moves bytes the
/// fingerprint cannot see exactly as a patch of `polint` does. One question —
/// does this build read sources the fingerprint cannot name — has one answer for
/// all of them. Runtime environment and runner settings are equally categorical:
/// only Cargo knows how to apply them, so direct execution is not equivalent.
fn a_cargo_config_prevents_direct_reuse(
    rule_pkg_dir: &Path,
    repo_root: &Path,
    cargo_home: Option<PathBuf>,
) -> bool {
    cargo_config_files(rule_pkg_dir, repo_root, cargo_home)
        .iter()
        .any(|path| cargo_config_prevents_direct_reuse(path))
}

/// Every cargo config file that can apply to a rule-host build.
///
/// polint spawns cargo with the repository root as its working directory, and
/// cargo merges the `.cargo/` config of that directory with those of every
/// ancestor and with `$CARGO_HOME`'s. The rule package's own is considered
/// conservatively as well. Both file names are taken at every location because
/// cargo still honors the extensionless one it used before 1.39. Nothing here
/// requires a file to exist; a name that is not there is read as absent.
fn cargo_config_files(
    rule_pkg_dir: &Path,
    repo_root: &Path,
    cargo_home: Option<PathBuf>,
) -> Vec<PathBuf> {
    std::iter::once(rule_pkg_dir)
        .chain(repo_root.ancestors())
        .map(|dir| dir.join(".cargo"))
        .chain(cargo_home)
        .flat_map(|dir| CARGO_CONFIG_NAMES.map(|name| dir.join(name)))
        .collect()
}

/// Where cargo keeps its user-wide config: `$CARGO_HOME`, else `~/.cargo`,
/// resolved the way cargo itself resolves it. `None` when the environment names
/// neither, which is no location to read.
fn cargo_home() -> Option<PathBuf> {
    if let Some(home) = env_path("CARGO_HOME") {
        return Some(home);
    }
    env_path("HOME")
        .or_else(|| env_path("USERPROFILE"))
        .map(|home| home.join(".cargo"))
}

/// One environment variable read as a directory, treating an empty value as the
/// absence it means.
fn env_path(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Whether one cargo config changes inputs or execution behavior that direct
/// reuse cannot reproduce.
///
/// A file that is not there redirects nothing — nor does a location where cargo
/// could not keep one, such as a `.cargo` that is a file rather than a directory.
/// A file that IS there and cannot be read or parsed is treated as if it did
/// redirect: the store may only ever make a run faster, so a config whose effect
/// this run could not establish is never one it claims to have proven.
fn cargo_config_prevents_direct_reuse(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
        Ok(metadata) if !metadata.is_file() => return true,
        Ok(_) => {}
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return true;
    };
    let Ok(config) = toml::from_str::<toml::Value>(&text) else {
        return true;
    };
    CARGO_UNSHAREABLE_KEYS
        .iter()
        .any(|key| config.get(key).is_some())
        || config
            .get("target")
            .and_then(toml::Value::as_table)
            .is_some_and(|targets| {
                targets
                    .values()
                    .any(|target| target.get("runner").is_some())
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(&format!("polint-rules-store-{label}-"))
            .tempdir()
            .expect("create a temporary directory")
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent directory");
        }
        std::fs::write(path, contents).expect("write fixture file");
    }

    fn release_environment() -> BuildEnvironment {
        BuildEnvironment {
            profile: "release".to_string(),
            toolchain: ToolchainIdentity {
                rustc: "rustc 1.95.0 (abcdef 2026-01-01)".to_string(),
                cargo: "cargo 1.95.0 (abcdef 2026-01-01)".to_string(),
                rustup_toolchain: None,
            },
            cargo_flags: CARGO_FLAG_VARIABLES
                .iter()
                .map(|_| UNSET.to_string())
                .collect(),
            cargo_overrides: Vec::new(),
            cargo_home: None,
        }
    }

    /// The same environment with one compiler flag variable set.
    fn environment_with(key: &str, value: &str) -> BuildEnvironment {
        let index = CARGO_FLAG_VARIABLES
            .iter()
            .position(|(name, _)| *name == key)
            .expect("a fingerprint line this module records");
        let mut environment = release_environment();
        environment.cargo_flags[index] = value.to_string();
        environment
    }

    /// Every shareability question in this module is asked with cargo's home
    /// given rather than read, so a config in the developer's own `~/.cargo`
    /// cannot decide a test.
    fn shareable(rule_pkg_dir: &Path, repo_root: &Path) -> bool {
        shareable_with_cargo_home(rule_pkg_dir, repo_root, None)
    }

    /// A repository with one rule package that depends only on a registry crate.
    fn rule_package(root: &Path) -> PathBuf {
        let package = root.join(".polint/rules");
        write(
            &package.join("Cargo.toml"),
            "[package]\nname = \"polint-local-rules\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
             [dependencies]\npolint = \"0.3.1\"\n\n[workspace]\n",
        );
        write(&package.join("src/main.rs"), "fn main() {}\n");
        package
    }

    #[test]
    fn an_absolute_override_names_the_store_and_off_turns_sharing_off() {
        let absolute_override = std::env::temp_dir().join("polint-store-shared");
        let override_value = absolute_override.to_string_lossy();
        assert_eq!(
            resolve_root(Some(override_value.as_ref()), None, None, Some("/home/u")),
            Some(absolute_override)
        );
        for off in ["off", "OFF", "  Disabled ", "none", "None"] {
            assert_eq!(
                resolve_root(Some(off), None, None, Some("/home/u")),
                None,
                "{off} must turn sharing off"
            );
        }
    }

    #[test]
    fn a_value_that_is_not_a_location_turns_sharing_off_rather_than_guessing() {
        for value in ["", "   ", "relative/store", "./store", "~/store"] {
            assert_eq!(
                resolve_root(Some(value), None, None, Some("/home/u")),
                None,
                "{value:?} is not an absolute path and must not resolve to a store"
            );
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn the_default_follows_the_xdg_base_directory_specification() {
        assert_eq!(
            resolve_root(None, Some("/x/cache"), None, Some("/home/u")),
            Some(PathBuf::from("/x/cache/polint/store"))
        );
        assert_eq!(
            resolve_root(None, None, None, Some("/home/u")),
            Some(PathBuf::from("/home/u/.cache/polint/store"))
        );
        // A relative XDG_CACHE_HOME is invalid per the specification and is ignored.
        assert_eq!(
            resolve_root(None, Some("relative"), None, Some("/home/u")),
            Some(PathBuf::from("/home/u/.cache/polint/store"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_default_is_the_user_caches_directory() {
        assert_eq!(
            resolve_root(None, Some("/x/cache"), None, Some("/home/u")),
            Some(PathBuf::from("/home/u/Library/Caches/polint/store"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_default_is_the_local_application_data_directory() {
        assert_eq!(
            resolve_root(None, None, Some(r"C:\Users\u\AppData\Local"), None),
            Some(PathBuf::from(r"C:\Users\u\AppData\Local\polint\store"))
        );
    }

    #[test]
    fn an_environment_with_no_home_shares_nothing_instead_of_failing() {
        assert_eq!(resolve_root(None, None, None, None), None);
    }

    #[test]
    fn a_key_that_is_not_a_hex_digest_is_not_a_key() {
        assert_eq!(shard("../../etc"), None);
        assert_eq!(shard("a/b"), None);
        assert_eq!(shard(""), None);
        assert_eq!(shard("a"), None);
        assert_eq!(shard(&"a".repeat(129)), None);
        assert_eq!(shard("deadbeef"), Some("de".to_string()));
        assert_eq!(shard("DEADBEEF"), Some("DE".to_string()));
    }

    #[test]
    fn a_recorded_binary_path_may_not_leave_the_target_directory() {
        for escape in [
            "",
            "..",
            "../outside",
            "release/../../outside",
            "/etc/passwd",
        ] {
            assert_eq!(
                relative_target_path(escape),
                None,
                "{escape:?} must not resolve under the target directory"
            );
        }
        assert_eq!(
            relative_target_path("release/polint-local-rules"),
            Some(PathBuf::from("release/polint-local-rules"))
        );
    }

    #[test]
    fn a_fingerprint_is_stable_and_changes_with_every_input_it_names() {
        let temp = temp_dir("fingerprint");
        let root = temp.path();
        let package = rule_package(root);
        let environment = release_environment();

        let baseline = build_fingerprint(root, &package, &environment).expect("a fingerprint");
        assert_eq!(
            build_fingerprint(root, &package, &environment),
            Some(baseline.clone()),
            "the same inputs must produce the same key"
        );
        assert_eq!(baseline.complete.len(), 64, "a key is a sha256 hex digest");
        assert_ne!(
            baseline.complete, baseline.authored,
            "the two digests cover different input sets"
        );

        // A lockfile is an input a build may write for itself, so it moves the
        // key a later run computes without moving the bracket that says the
        // sources compiled were the sources recorded.
        write(&package.join("Cargo.lock"), "version = 4\n");
        let locked = build_fingerprint(root, &package, &environment).expect("a fingerprint");
        assert_ne!(
            baseline.complete, locked.complete,
            "a lockfile is part of the identity a build has"
        );
        assert_eq!(
            baseline.authored, locked.authored,
            "a lockfile cargo wrote is not a change to the authored sources"
        );
        std::fs::remove_file(package.join("Cargo.lock")).expect("remove the lockfile");

        write(&package.join("src/main.rs"), "fn main() { }\n");
        let changed_source =
            build_fingerprint(root, &package, &environment).expect("a fingerprint");
        assert_ne!(
            baseline, changed_source,
            "a changed rule source is a new key"
        );

        write(
            &package.join("Cargo.toml"),
            "[package]\nname = \"polint-local-rules\"\nversion = \"0.0.1\"\nedition = \"2024\"\n\n\
             [dependencies]\npolint = \"0.3.1\"\n\n[workspace]\n",
        );
        let changed_manifest =
            build_fingerprint(root, &package, &environment).expect("a fingerprint");
        assert_ne!(
            changed_source, changed_manifest,
            "a changed manifest is a new key"
        );

        let mut dev = release_environment();
        dev.profile = "dev".to_string();
        assert_ne!(
            build_fingerprint(root, &package, &dev),
            Some(changed_manifest.clone()),
            "a different cargo profile is a new key"
        );

        let mut other_compiler = release_environment();
        other_compiler.toolchain.rustc = "rustc 1.96.0 (fedcba 2026-02-02)".to_string();
        assert_ne!(
            build_fingerprint(root, &package, &other_compiler),
            Some(changed_manifest.clone()),
            "a different compiler is a new key"
        );

        // RUSTFLAGS changes what the compiler produces from unchanged sources,
        // so it has to change the key.
        let with_flags = build_fingerprint(
            root,
            &package,
            &environment_with("rustflags", "-C target-cpu=native"),
        );
        assert_ne!(
            with_flags,
            Some(changed_manifest),
            "RUSTFLAGS is part of the key"
        );

        let with_rustc = build_fingerprint(
            root,
            &package,
            &environment_with("rustc", "/opt/custom/bin/rustc"),
        );
        assert_ne!(
            with_rustc,
            build_fingerprint(root, &package, &environment),
            "RUSTC selects the compiler Cargo actually executes"
        );

        let mut with_profile_override = environment.clone();
        with_profile_override
            .cargo_overrides
            .push(("CARGO_PROFILE_RELEASE_LTO".to_string(), "thin".to_string()));
        assert_ne!(
            build_fingerprint(root, &package, &with_profile_override),
            build_fingerprint(root, &package, &environment),
            "Cargo profile environment overrides change the built artifact"
        );
    }

    #[test]
    fn every_package_build_input_is_part_of_the_key() {
        let temp = temp_dir("sources");
        let root = temp.path();
        let package = rule_package(root);
        let environment = release_environment();

        let baseline = build_fingerprint(root, &package, &environment).expect("a fingerprint");
        write(&package.join("src/rules/extra.rs"), "// a new rule\n");
        let nested = build_fingerprint(root, &package, &environment).expect("a fingerprint");
        assert_ne!(
            baseline, nested,
            "a rule source added in a subdirectory is a new key"
        );

        // A build script runs in the compiler and decides what the crate
        // becomes; it lives beside the manifest rather than under `src/`.
        write(&package.join("build.rs"), "fn main() {}\n");
        let with_build_script =
            build_fingerprint(root, &package, &environment).expect("a fingerprint");
        assert_ne!(
            nested, with_build_script,
            "a build script is part of what the package compiles to"
        );

        // Cargo's own output under the package is not a source.
        write(&package.join("target/debug/build/stale.rs"), "// output\n");
        assert_eq!(
            build_fingerprint(root, &package, &environment),
            Some(with_build_script.clone()),
            "a package-local cargo target directory is output, not input"
        );

        write(
            &package.join("assets/policy.json"),
            "{\"mode\":\"strict\"}\n",
        );
        let with_asset = build_fingerprint(root, &package, &environment).expect("a fingerprint");
        assert_ne!(
            with_build_script, with_asset,
            "non-Rust files can be consumed by include_bytes! or build scripts"
        );

        std::fs::write(package.join("assets/raw.bin"), [0_u8, 0xff, 0x7f])
            .expect("write non-UTF-8 input bytes");
        let with_raw_bytes =
            build_fingerprint(root, &package, &environment).expect("a fingerprint");
        assert_ne!(
            with_asset, with_raw_bytes,
            "input contents are hashed as bytes rather than decoded as text"
        );
    }

    #[test]
    fn cargo_configs_are_part_of_the_key_and_cargo_home_path_is_not() {
        let temp = temp_dir("cargo-config-fingerprint");
        let root = temp.path().join("repo");
        let package = rule_package(&root);
        let mut environment = release_environment();
        environment.cargo_home = Some(temp.path().join("cargo-home"));

        let baseline = build_fingerprint(&root, &package, &environment).expect("a fingerprint");
        write(
            &temp.path().join(".cargo/config.toml"),
            "[build]\nrustflags = [\"-C\", \"target-cpu=native\"]\n",
        );
        let ancestor = build_fingerprint(&root, &package, &environment).expect("a fingerprint");
        assert_ne!(
            baseline, ancestor,
            "a config above the repository changes Cargo's build"
        );

        // A cargo home is read for its configuration, and both names cargo still
        // honors there are read, so a registry mirror configured under either one
        // moves the key rather than hiding behind a path that no longer does.
        for name in CARGO_CONFIG_NAMES {
            let before = build_fingerprint(&root, &package, &environment).expect("a fingerprint");
            write(
                &temp.path().join("cargo-home").join(name),
                "[source.crates-io]\nreplace-with = \"mirror\"\n",
            );
            assert_ne!(
                before,
                build_fingerprint(&root, &package, &environment).expect("a fingerprint"),
                "a Cargo home config named {name} is part of the build identity"
            );
        }

        // The location carries nothing the contents do not: two cargo homes that
        // hold no configuration at all are the same build from both paths.
        let mut empty_home = release_environment();
        empty_home.cargo_home = Some(temp.path().join("empty-cargo-home"));
        let from_one = build_fingerprint(&root, &package, &empty_home).expect("a fingerprint");
        empty_home.cargo_home = Some(temp.path().join("another-empty-cargo-home"));
        assert_eq!(
            build_fingerprint(&root, &package, &empty_home),
            Some(from_one),
            "an empty Cargo home is the same input wherever it sits"
        );
    }

    /// Two containers mount their cargo home at different paths and configure it
    /// identically; the compiled rule host is the same, so the key must be too.
    #[test]
    fn two_cargo_homes_with_the_same_configuration_share_one_key() {
        let temp = temp_dir("cargo-home-path");
        let root = temp.path().join("repo");
        let package = rule_package(&root);
        let configuration = "[target.x86_64-unknown-linux-gnu]\nlinker = \"clang\"\n";

        let mut environment = release_environment();
        let mut fingerprints = Vec::new();
        for home in ["home-a", "deeply/nested/home-b"] {
            let home = temp.path().join(home);
            for name in CARGO_CONFIG_NAMES {
                write(&home.join(name), configuration);
            }
            environment.cargo_home = Some(home);
            fingerprints
                .push(build_fingerprint(&root, &package, &environment).expect("a fingerprint"));
        }

        assert_eq!(
            fingerprints[0], fingerprints[1],
            "identical cargo configuration is one key from every cargo home path"
        );
    }

    #[test]
    fn a_package_that_is_not_there_has_no_key() {
        let temp = temp_dir("missing");
        assert_eq!(
            build_fingerprint(
                temp.path(),
                &temp.path().join("nowhere"),
                &release_environment()
            ),
            None
        );
    }

    #[test]
    fn a_rule_package_without_checkout_path_tokens_is_shareable() {
        let temp = temp_dir("shareable");
        let root = temp.path();
        let package = rule_package(root);
        assert!(shareable(&package, root));
    }

    #[test]
    fn a_rule_source_that_embeds_cargo_manifest_dir_is_not_shareable() {
        let temp = temp_dir("manifest-dir-token");
        let root = temp.path();
        let package = rule_package(root);
        write(
            &package.join("src/main.rs"),
            "fn main() { println!(\"{}\", env!(\"CARGO_MANIFEST_DIR\")); }\n",
        );
        assert!(!shareable(&package, root));
    }

    #[test]
    fn an_out_dir_token_in_a_doc_comment_is_conservatively_not_shareable() {
        let temp = temp_dir("doc-comment-token");
        let root = temp.path();
        let package = rule_package(root);
        write(
            &package.join("src/main.rs"),
            "/// This inert documentation mentions OUT_DIR.\nfn main() {}\n",
        );
        assert!(
            !shareable(&package, root),
            "byte-token matching accepts false positives so it never wrong-shares"
        );
    }

    #[test]
    fn a_build_script_that_embeds_out_dir_is_not_shareable() {
        let temp = temp_dir("build-script-token");
        let root = temp.path();
        let package = rule_package(root);
        write(
            &package.join("build.rs"),
            "include!(concat!(env!(\"OUT_DIR\"), \"/generated.rs\"));\nfn main() {}\n",
        );
        assert!(!shareable(&package, root));
    }

    #[test]
    fn cargo_and_rustc_environment_names_are_not_shareable_but_cargo_pkg_is() {
        for (variable, expression) in [
            ("CARGO", "env!(\"CARGO\")"),
            ("RUSTC", "std::env::var(\"RUSTC\")"),
        ] {
            let temp = temp_dir("tool-path-token");
            let root = temp.path();
            let package = rule_package(root);
            write(
                &package.join("src/main.rs"),
                &format!("fn main() {{ let _ = {expression}; }}\n"),
            );
            assert!(
                !shareable(&package, root),
                "the {variable} executable path must stay checkout-local"
            );
        }

        let temp = temp_dir("cargo-pkg-token");
        let root = temp.path();
        let package = rule_package(root);
        write(
            &package.join("src/main.rs"),
            "fn main() { let _ = env!(\"CARGO_PKG_VERSION\"); }\n",
        );
        assert!(
            shareable(&package, root),
            "Cargo package metadata is checkout-independent"
        );
    }

    #[test]
    fn a_relative_path_dependency_that_leaves_the_package_is_not_shareable() {
        let temp = temp_dir("escaping");
        let root = temp.path();
        let package = rule_package(root);
        write(
            &package.join("Cargo.toml"),
            "[package]\nname = \"polint-local-rules\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nhelpers = { path = \"../../helpers\" }\n\n[workspace]\n",
        );
        assert!(!shareable(&package, root));
    }

    #[test]
    fn a_workspace_path_dependency_outside_the_rule_package_is_not_shareable() {
        let temp = temp_dir("workspace-escape");
        let root = temp.path();
        let package = rule_package(root);
        write(
            &root.join("Cargo.toml"),
            "[workspace]\nmembers = [\".polint/rules\", \"helpers\"]\n\n\
             [workspace.dependencies]\nhelpers = { path = \"helpers\" }\n",
        );
        write(
            &root.join("helpers/Cargo.toml"),
            "[package]\nname = \"helpers\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
        );
        assert!(!shareable(&package, root));
    }

    #[cfg(unix)]
    #[test]
    fn a_path_dependency_symlinked_outside_the_package_is_not_shareable() {
        use std::os::unix::fs::symlink;

        let temp = temp_dir("symlink-escape");
        let root = temp.path();
        let package = rule_package(root);
        let external = root.join("external-helper");
        write(
            &external.join("Cargo.toml"),
            "[package]\nname = \"helpers\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
        );
        symlink(&external, package.join("helpers")).expect("create dependency symlink");
        write(
            &package.join("Cargo.toml"),
            "[package]\nname = \"polint-local-rules\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nhelpers = { path = \"helpers\" }\n\n[workspace]\n",
        );
        assert!(!shareable(&package, root));
        assert_eq!(
            build_fingerprint(root, &package, &release_environment()),
            None,
            "a symlinked source tree cannot produce a complete fingerprint"
        );
    }

    #[test]
    fn an_absolute_path_dependency_is_not_shareable_without_a_content_digest() {
        let temp = temp_dir("absolute");
        let root = temp.path();
        let package = rule_package(root);
        let elsewhere = root.join("elsewhere").to_string_lossy().replace('\\', "/");
        write(
            &package.join("Cargo.toml"),
            &format!(
                "[package]\nname = \"polint-local-rules\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
                 [dependencies]\npolint = {{ path = \"{elsewhere}\" }}\n\n[workspace]\n"
            ),
        );
        assert!(!shareable(&package, root));
    }

    #[test]
    fn a_path_dependency_inside_the_package_is_shareable() {
        let temp = temp_dir("inside");
        let root = temp.path();
        let package = rule_package(root);
        write(
            &package.join("Cargo.toml"),
            "[package]\nname = \"polint-local-rules\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nhelpers = { path = \"helpers\" }\n\n[workspace]\n",
        );
        write(
            &package.join("helpers/Cargo.toml"),
            "[package]\nname = \"helpers\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
        );
        assert!(shareable(&package, root));

        // A manifest nested inside the package is read too, so a pointer that
        // leaves from there is caught as well.
        write(
            &package.join("helpers/Cargo.toml"),
            "[package]\nname = \"helpers\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nfurther = { path = \"../../../elsewhere\" }\n",
        );
        assert!(!shareable(&package, root));
    }

    #[test]
    fn a_cargo_config_with_unreproducible_build_or_run_semantics_is_not_shareable() {
        for setting in [
            "[patch.crates-io]\npolint = { path = \"/tmp/polint\" }\n",
            "[replace]\n\"polint:0.3.1\" = { path = \"/tmp/polint\" }\n",
            "paths = [\"/tmp/polint\"]\n",
            "[source.crates-io]\nreplace-with = \"vendored\"\n",
            "[env]\nPOLINT_TEST_MODE = \"store-sensitive\"\n",
            "[target.x86_64-unknown-linux-gnu]\nrunner = \"host-wrapper\"\n",
            "include = \"other.toml\"\n",
        ] {
            let temp = temp_dir("redirect");
            let root = temp.path();
            let package = rule_package(root);
            write(&root.join(".cargo/config.toml"), setting);
            assert!(
                !shareable(&package, root),
                "a config declaring {setting:?} must not be shared"
            );
        }
    }

    #[test]
    fn a_cargo_config_or_workspace_manifest_that_is_not_a_file_is_not_shareable() {
        let config_temp = temp_dir("config-directory");
        let config_root = config_temp.path();
        let config_package = rule_package(config_root);
        std::fs::create_dir_all(config_root.join(".cargo/config.toml"))
            .expect("create non-file config");
        assert!(!shareable(&config_package, config_root));

        let manifest_temp = temp_dir("manifest-directory");
        let manifest_root = manifest_temp.path();
        let manifest_package = rule_package(manifest_root);
        std::fs::create_dir_all(manifest_root.join("Cargo.toml"))
            .expect("create non-file workspace manifest");
        assert!(!shareable(&manifest_package, manifest_root));
        assert_eq!(
            build_fingerprint(manifest_root, &manifest_package, &release_environment()),
            None
        );
    }

    #[test]
    fn a_cargo_config_that_cannot_be_parsed_is_not_shareable() {
        let temp = temp_dir("unparseable");
        let root = temp.path();
        let package = rule_package(root);
        write(&root.join(".cargo/config"), "this is not = = toml\n");
        assert!(!shareable(&package, root));
    }

    #[cfg(unix)]
    #[test]
    fn a_cargo_config_that_cannot_be_read_is_not_shareable() {
        use std::os::unix::fs::PermissionsExt;

        let temp = temp_dir("unreadable");
        let root = temp.path();
        let package = rule_package(root);
        let config = root.join(".cargo/config.toml");
        write(&config, "[build]\njobs = 1\n");
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o000))
            .expect("drop read permission");
        // A process with the privilege to read the file regardless of its mode
        // has nothing to answer here; ask the question only where it applies.
        let denied = std::fs::read_to_string(&config).is_err();
        let answer = shareable(&package, root);
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o644))
            .expect("restore read permission");
        if denied {
            assert!(!answer, "a config this run could not read is not proven");
        }
    }

    #[test]
    fn a_cargo_config_without_a_redirect_is_shareable() {
        let temp = temp_dir("plain-config");
        let root = temp.path();
        let package = rule_package(root);
        write(&root.join(".cargo/config.toml"), "[build]\njobs = 2\n");
        assert!(!a_cargo_config_prevents_direct_reuse(&package, root, None));
    }

    #[test]
    fn a_published_host_is_restored_into_a_fresh_target_directory() {
        let temp = temp_dir("roundtrip");
        let store = RuleHostStore::at(temp.path().join("store"));
        let built = temp.path().join("built");
        let binary = built.join("release/polint-local-rules");
        write(&binary, "the compiled rule host\n");

        record(Some(&store), &built, &"ab".repeat(32), &binary);
        assert!(
            binary_recorded_by_stamp(&built, &"ab".repeat(32)).is_some(),
            "the build that published is stamped for its own checkout"
        );

        let restored_into = temp.path().join("restored");
        let restored =
            restore(&store, &restored_into, &"ab".repeat(32)).expect("the stored host is restored");
        assert_eq!(restored, restored_into.join("release/polint-local-rules"));
        assert_eq!(
            std::fs::read_to_string(&restored).expect("read the restored host"),
            "the compiled rule host\n"
        );
        assert_eq!(
            binary_recorded_by_stamp(&restored_into, &"ab".repeat(32)).as_deref(),
            Some(restored.as_path()),
            "a restore stamps the checkout it restored into"
        );
    }

    #[test]
    fn restore_and_stamp_atomically_replace_an_existing_host() {
        let temp = temp_dir("replace-existing");
        let store = RuleHostStore::at(temp.path().join("store"));
        let first_target = temp.path().join("first");
        let first_binary = first_target.join("release/polint-local-rules");
        write(&first_binary, "first compiled host\n");
        record(Some(&store), &first_target, &"ab".repeat(32), &first_binary);

        let destination = temp.path().join("destination");
        restore(&store, &destination, &"ab".repeat(32)).expect("restore first host");

        let second_target = temp.path().join("second");
        let second_binary = second_target.join("release/polint-local-rules");
        write(&second_binary, "second compiled host\n");
        record(
            Some(&store),
            &second_target,
            &"cd".repeat(32),
            &second_binary,
        );
        let restored =
            restore(&store, &destination, &"cd".repeat(32)).expect("replace existing host");

        assert_eq!(
            std::fs::read_to_string(restored).expect("read replacement"),
            "second compiled host\n"
        );
        assert!(
            binary_recorded_by_stamp(&destination, &"cd".repeat(32)).is_some(),
            "the replacement stamp must supersede the old fingerprint"
        );
    }

    #[test]
    fn an_entry_for_another_question_is_never_offered_as_the_answer() {
        let temp = temp_dir("wrong-key");
        let store = RuleHostStore::at(temp.path().join("store"));
        let built = temp.path().join("built");
        let binary = built.join("release/polint-local-rules");
        write(&binary, "the compiled rule host\n");
        record(Some(&store), &built, &"ab".repeat(32), &binary);

        // The same bytes, filed under a key that is not the one recorded inside.
        let recorded = store.read(&"ab".repeat(32)).expect("the published entry");
        store.publish(&"cd".repeat(32), &recorded);
        assert_eq!(
            restore(&store, &temp.path().join("restored"), &"cd".repeat(32)),
            None
        );
        assert_eq!(
            store.read(&"cd".repeat(32)),
            None,
            "an entry that cannot prove which question it answers is deleted"
        );
    }

    #[test]
    fn a_corrupt_blob_is_discarded_rather_than_executed() {
        let temp = temp_dir("corrupt");
        let store = RuleHostStore::at(temp.path().join("store"));
        let built = temp.path().join("built");
        let binary = built.join("release/polint-local-rules");
        write(&binary, "the compiled rule host\n");
        record(Some(&store), &built, &"ab".repeat(32), &binary);

        let entry: StoreEntry =
            serde_json::from_slice(&store.read(&"ab".repeat(32)).expect("the published entry"))
                .expect("a readable entry");
        let blob = store.blob_path(&entry.sha256).expect("a blob path");
        std::fs::write(&blob, "something else entirely\n").expect("corrupt the blob");

        let restored_into = temp.path().join("restored");
        assert_eq!(restore(&store, &restored_into, &"ab".repeat(32)), None);
        assert!(!blob.exists(), "a blob that is not its own name is removed");
        assert_eq!(
            store.read(&"ab".repeat(32)),
            None,
            "the entry that named it is removed with it"
        );
        assert!(
            !restored_into.join("release/polint-local-rules").exists(),
            "no unverified bytes are ever moved into place"
        );
    }

    #[test]
    fn a_stamp_stops_matching_when_the_binary_beside_it_changes() {
        let temp = temp_dir("stamp");
        let built = temp.path().join("built");
        let binary = built.join("release/polint-local-rules");
        write(&binary, "the compiled rule host\n");
        record(None, &built, &"ab".repeat(32), &binary);

        assert!(binary_recorded_by_stamp(&built, &"ab".repeat(32)).is_some());
        assert_eq!(
            binary_recorded_by_stamp(&built, &"cd".repeat(32)),
            None,
            "a stamp answers only the fingerprint it recorded"
        );

        write(&binary, "a different rule host\n");
        assert_eq!(
            binary_recorded_by_stamp(&built, &"ab".repeat(32)),
            None,
            "a replaced binary is not the one the stamp recorded"
        );

        std::fs::remove_file(&binary).expect("remove the host");
        assert_eq!(binary_recorded_by_stamp(&built, &"ab".repeat(32)), None);
    }

    #[test]
    fn publishing_never_fails_a_run_even_when_the_store_cannot_be_written() {
        let temp = temp_dir("unwritable");
        // A file where the store root must be a directory: every path under it is
        // unwritable.
        let root = temp.path().join("root");
        std::fs::write(&root, b"not a directory").expect("write the blocking file");
        let store = RuleHostStore::at(&root);
        let built = temp.path().join("built");
        let binary = built.join("release/polint-local-rules");
        write(&binary, "the compiled rule host\n");

        record(Some(&store), &built, &"ab".repeat(32), &binary);
        assert_eq!(
            restore(&store, &temp.path().join("restored"), &"ab".repeat(32)),
            None
        );
        assert!(
            binary_recorded_by_stamp(&built, &"ab".repeat(32)).is_some(),
            "a store that cannot be written still leaves this checkout stamped"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_directories_polint_creates_are_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;

        let temp = temp_dir("private");
        let store = RuleHostStore::at(temp.path().join("store"));
        store.publish(&"ab".repeat(32), b"recorded");
        let mode = std::fs::metadata(temp.path().join("store"))
            .expect("the store root exists")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700, "the store root must be user-private");
    }
}
