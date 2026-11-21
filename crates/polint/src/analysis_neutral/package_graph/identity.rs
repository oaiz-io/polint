//! Identity of a package graph and of the functions and call sites in it.
//!
//! These identities are the same in two scans of the same content on any
//! machine: they hold paths relative to the package root, byte spans and
//! hashes of source text, and never a run-local id.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::format::PACKAGE_GRAPH_SCHEMA;

/// What a package graph was built from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PackageIdentity {
    /// `name` of the package's `package.json`, when it has one.
    pub(crate) name: Option<String>,
    /// `version` of the package's `package.json`, when it has one.
    pub(crate) version: Option<String>,
    /// The registry integrity value of the package tarball when the caller
    /// knows it (`integrity:<value>`), and a hash of the analysed files
    /// otherwise (`files:<sha256>`).
    pub(crate) content: String,
    pub(crate) entry: EntryConfig,
    /// The engine version that built the graph.
    pub(crate) engine: String,
    pub(crate) schema: u32,
    /// The digest of the models the engine applied.
    pub(crate) models: String,
}

/// Which entry of the package was analysed: `exports` in `package.json` can
/// select different files for each condition and module format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EntryConfig {
    /// `main` of `package.json`, when it has one.
    pub(crate) main: Option<String>,
    /// The export conditions the resolver matched, in its order.
    pub(crate) conditions: Vec<String>,
    /// `module` when `package.json` sets `"type": "module"`, else `commonjs`.
    pub(crate) module_format: String,
}

impl PackageIdentity {
    /// The identity of the package rooted at `root`. `files` holds the path
    /// relative to `root` and the content hash of every analysed file.
    pub(crate) fn read<'a>(
        root: &Path,
        files: impl IntoIterator<Item = (&'a str, &'a str)>,
        integrity: Option<&str>,
        conditions: &[&str],
        models: &str,
    ) -> Self {
        let manifest = crate::repo_fs::read_file_with_limit(
            &root.join("package.json"),
            crate::repo_fs::TOPOLOGY_MANIFEST_MAX_BYTES,
        )
        .ok()
        .and_then(|bytes| {
            crate::bounded_json::from_slice::<serde_json::Value>(
                &bytes,
                crate::repo_fs::TOPOLOGY_MANIFEST_MAX_BYTES,
            )
            .ok()
        })
        .unwrap_or(serde_json::Value::Null);
        let text = |field: &str| manifest.get(field).and_then(|value| value.as_str());
        let content = match integrity {
            Some(integrity) => format!("integrity:{integrity}"),
            None => {
                let mut files = files.into_iter().collect::<Vec<_>>();
                files.sort_unstable();
                let mut hasher = Sha256::new();
                for (path, hash) in files {
                    hasher.update(path.as_bytes());
                    hasher.update([0]);
                    hasher.update(hash.as_bytes());
                    hasher.update([0]);
                }
                format!("files:{}", hex(&hasher.finalize()))
            }
        };
        Self {
            name: text("name").map(str::to_string),
            version: text("version").map(str::to_string),
            content,
            entry: EntryConfig {
                main: text("main").map(str::to_string),
                conditions: conditions.iter().map(|name| name.to_string()).collect(),
                module_format: match text("type") {
                    Some("module") => "module",
                    _ => "commonjs",
                }
                .to_string(),
            },
            engine: env!("CARGO_PKG_VERSION").to_string(),
            schema: PACKAGE_GRAPH_SCHEMA,
            models: models.to_string(),
        }
    }
}

/// A function: its path in the package, its byte span and a hash of its
/// source text, so an edit to the body gives a new identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct FunctionIdentity {
    pub(crate) path: String,
    pub(crate) start: u32,
    pub(crate) end: u32,
    /// SHA-256 of the function's source bytes, in hex.
    pub(crate) body: String,
}

impl FunctionIdentity {
    pub(crate) fn new(path: &str, start: u32, end: u32, source: &str) -> Self {
        let body = source
            .as_bytes()
            .get(start as usize..end as usize)
            .unwrap_or_default();
        Self {
            path: path.to_string(),
            start,
            end,
            body: hex(&Sha256::digest(body)),
        }
    }
}

/// A call site: its byte span relative to the start of its enclosing
/// function, whose identity anchors it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct CallsiteIdentity {
    pub(crate) start: u32,
    pub(crate) end: u32,
}

impl CallsiteIdentity {
    /// The identity of the site at `start..end` inside `function`.
    pub(crate) fn within(function: &FunctionIdentity, start: u32, end: u32) -> Self {
        Self {
            start: start.saturating_sub(function.start),
            end: end.saturating_sub(function.start),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_function_identity_follows_its_body_not_its_run() {
        let source = "function a() { return 1; }\nfunction b() { return 2; }\n";
        let first = FunctionIdentity::new("index.js", 0, 26, source);
        assert_eq!(first, FunctionIdentity::new("index.js", 0, 26, source));
        let edited = "function a() { return 3; }\nfunction b() { return 2; }\n";
        assert_ne!(
            first.body,
            FunctionIdentity::new("index.js", 0, 26, edited).body
        );
        let site = CallsiteIdentity::within(&FunctionIdentity::new("x.js", 10, 40, source), 15, 20);
        assert_eq!((site.start, site.end), (5, 10));
    }

    #[test]
    fn the_content_hash_ignores_the_order_files_are_listed_in() {
        let root = Path::new("/nonexistent");
        let one = PackageIdentity::read(root, [("a.js", "1"), ("b.js", "2")], None, &[], "");
        let two = PackageIdentity::read(root, [("b.js", "2"), ("a.js", "1")], None, &[], "");
        assert_eq!(one, two);
        assert!(one.content.starts_with("files:"));
        let pinned = PackageIdentity::read(root, [], Some("sha512-x"), &[], "");
        assert_eq!(pinned.content, "integrity:sha512-x");
    }
}
