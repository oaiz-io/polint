//! Parser identity labels shared by the language frontends and the analysis kernel.
//!
//! Facts are only interchangeable when the parser that produced them is the same
//! parser that would produce them now. These labels are what "the same parser"
//! means to a cache key, a provider contract, and the semantic store, so they must
//! track the resolved dependency versions exactly: a stale label lets a parser or
//! grammar change reuse facts produced by the previous one, at a key that claims
//! they are current.
//!
//! `cache_toolchain_identities_track_exact_dependency_versions` enforces that
//! against the workspace manifest and lockfile.

/// Backend that parses Go sources.
///
/// Also pinned by a `CHECK` constraint in the semantic-store schema, so bumping
/// `tree-sitter` needs this constant *and* a new store migration.
pub const GO_PARSER_BACKEND: &str = "tree-sitter-0.26.8";

/// Grammar the Go backend is driven with.
///
/// This is tree-sitter-go 0.25.0 plus a compatible `special_argument_list`
/// widening for Go 1.26 `new(expr)`. Previously valid programs keep the 0.25.0
/// tree shape, so the cache identity stays on the upstream version. Pinned by
/// the same semantic-store `CHECK` constraint as [`GO_PARSER_BACKEND`].
/// The sources live in `crates/polint/vendor/tree-sitter-go`.
pub const GO_PARSER_GRAMMAR: &str = "tree-sitter-go-0.25.0";

/// Backend that parses TypeScript and JavaScript sources.
pub const TS_PARSER_BACKEND: &str = "oxc-0.129.0";

/// Resolver used to derive the TypeScript and JavaScript module graph.
pub const TS_MODULE_RESOLVER: &str = "oxc-resolver-11.19.1";

/// Every parser label this engine links, joined.
///
/// Cached artifacts derived from more than one language's facts are invalidated
/// by a change to any of those parsers, so they key on all of them at once.
#[must_use]
pub fn engine_parser_identity() -> String {
    format!("{GO_PARSER_BACKEND}+{GO_PARSER_GRAMMAR}+{TS_PARSER_BACKEND}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The labels are hand-written strings. If they drift from the resolved
    /// dependency versions, a parser or grammar bump silently keeps the same
    /// provider identity and cache keys, so facts produced by the previous parser
    /// are reused as if current.
    ///
    #[test]
    fn cache_toolchain_identities_track_exact_dependency_versions() {
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let lock = std::fs::read_to_string(workspace_root.join("Cargo.lock"))
            .expect("Cargo.lock is readable");
        let manifest = std::fs::read_to_string(workspace_root.join("Cargo.toml"))
            .expect("Cargo.toml is readable")
            .parse::<toml::Table>()
            .expect("Cargo.toml is valid TOML");
        let dependencies = manifest["workspace"]["dependencies"]
            .as_table()
            .expect("workspace dependencies are a table");
        let resolved = |crate_name: &str| {
            lock.split("[[package]]")
                .find_map(|block| {
                    let mut lines = block.lines().filter_map(|line| line.split_once(" = "));
                    let name = lines
                        .clone()
                        .find(|(key, _)| *key == "name")?
                        .1
                        .trim_matches('"');
                    (name == crate_name).then(|| {
                        lines
                            .find(|(key, _)| *key == "version")
                            .expect("locked package has a version")
                            .1
                            .trim_matches('"')
                            .to_string()
                    })
                })
                .unwrap_or_else(|| panic!("{crate_name} is not in Cargo.lock"))
        };
        let requirement = |crate_name: &str| {
            dependencies[crate_name]
                .as_str()
                .or_else(|| dependencies[crate_name].get("version")?.as_str())
                .unwrap_or_else(|| panic!("{crate_name} has a version requirement"))
        };
        for (label, crate_name, prefix, constant) in [
            (
                "Go backend",
                "tree-sitter",
                "tree-sitter",
                GO_PARSER_BACKEND,
            ),
            ("TypeScript backend", "oxc_parser", "oxc", TS_PARSER_BACKEND),
            (
                "TypeScript module resolver",
                "oxc_resolver",
                "oxc-resolver",
                TS_MODULE_RESOLVER,
            ),
        ] {
            let version = resolved(crate_name);
            assert_eq!(
                constant,
                format!("{prefix}-{version}"),
                "the {label} label drifted from the locked {crate_name} version; update the \
                 constant, and for a Go label also add a semantic-store migration for the new \
                 CHECK value"
            );
            assert_eq!(
                requirement(crate_name),
                format!("={version}"),
                "{crate_name} must be pinned exactly because its version is compiled into a \
                 persistent cache identity"
            );
        }
        assert_eq!(
            GO_PARSER_GRAMMAR, "tree-sitter-go-0.25.0",
            "the Go grammar label is the vendored tree-sitter-go 0.25.0 identity; changing it \
             requires a semantic-store migration for the CHECK constraint"
        );
        let vendor = workspace_root.join("crates/polint/vendor/tree-sitter-go");
        let upstream = std::fs::read_to_string(vendor.join("UPSTREAM"))
            .expect("vendored tree-sitter-go UPSTREAM is readable");
        assert!(
            upstream.contains("tree-sitter-go 0.25.0"),
            "vendored UPSTREAM must name tree-sitter-go 0.25.0: {upstream}"
        );
        let grammar = std::fs::read_to_string(vendor.join("grammar.js"))
            .expect("vendored grammar.js is readable");
        assert!(
            grammar.contains("choice(prec.dynamic(2, $._type), $._expression)"),
            "vendored grammar.js must keep the new(expr) widening that prefers types"
        );
        let parser_c = std::fs::read_to_string(vendor.join("src/parser.c"))
            .expect("vendored parser.c is readable");
        assert!(
            parser_c.contains("Automatically @generated by tree-sitter v0.25.8"),
            "vendored parser.c must be generated with tree-sitter CLI 0.25.8"
        );
        assert!(
            !lock.contains("name = \"tree-sitter-go\""),
            "tree-sitter-go must not remain a crates.io dependency; the grammar is vendored"
        );
        for crate_name in ["oxc_allocator", "oxc_ast", "oxc_semantic", "oxc_span"] {
            let version = resolved(crate_name);
            assert_eq!(
                requirement(crate_name),
                format!("={version}"),
                "{crate_name} must resolve with the Oxc parser identity"
            );
        }
    }
}
