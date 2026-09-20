#![allow(unreachable_patterns)]

//! polint: multi-language, repo-local static-analysis rules.
//!
//! Rule authors primarily use [`sdk`] and [`runner`]. Other modules are internal to this crate.

extern crate self as polint;

pub mod runner;
pub mod sdk;

pub use polint_macros::rule;

/// CLI entry used by the `polint` binary (`src/main.rs`).
pub fn run_main() -> anyhow::Result<u8> {
    cli::run()
}

pub(crate) mod analysis;
#[allow(dead_code, unreachable_pub, unused_imports)]
pub(crate) mod analysis_api;
pub(crate) mod analysis_kernel;
#[allow(dead_code, unreachable_patterns, unreachable_pub, unused_imports)]
pub(crate) mod analysis_neutral;
pub(crate) mod analysis_plan;
pub(crate) mod baseline;
pub(crate) mod cache;
pub(crate) mod cli;
pub(crate) mod config;
pub(crate) mod core;
pub(crate) mod diagnostics;
#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
#[path = "../../polint-eval/src/harness/mod.rs"]
pub(crate) mod eval;
pub(crate) mod frontend;
#[allow(dead_code, unreachable_pub, unused_imports)]
pub(crate) mod frontend_api;
pub(crate) mod fs;
pub(crate) mod git;
#[allow(dead_code, unreachable_pub, unused_imports)]
pub(crate) mod go;
pub(crate) mod golden_cost;
pub(crate) mod ignores;
#[allow(dead_code, unreachable_pub, unused_imports)]
pub(crate) mod internal_core;
#[allow(dead_code, unreachable_pub, unused_imports)]
pub(crate) mod ir;
pub(crate) mod jobs;
pub(crate) mod measure;
pub(crate) mod metrics;
pub(crate) mod module_graph;
pub(crate) mod path_context;
pub(crate) mod policy_queries;
pub(crate) mod repo_fs;
pub(crate) mod rule_error;
pub(crate) mod rule_manifest;
pub(crate) mod rule_test;
pub(crate) mod subprocess;
pub(crate) mod symbol_graph;
#[allow(dead_code, unreachable_pub, unused_imports)]
pub(crate) mod ts;

/// Internal surfaces for `polint-bench` (`feature = "bench"`). Not part of the supported API.
#[cfg(feature = "bench")]
pub mod _bench {
    /// Narrow surface for `polint-bench`: not a general `pub use crate::cache::*`.
    pub mod cache {
        pub use crate::cache::{Cache, stable_hash};
    }

    pub mod config {
        pub use crate::config::{LoadedConfig, load_config};
    }

    pub mod core {
        pub use crate::core::rule::run_rules;
        pub use crate::core::{AnalysisDb, Rule, RuleOptions};
    }

    pub mod fs {
        pub use crate::fs::{LoadSourcesTimings, load_analysis_files_with_timings};
    }

    pub mod go {
        use crate::cache::CacheAnalysisCache;
        use crate::core::AnalysisDb;
        use crate::diagnostics::Diagnostic;

        /// Bench entry: adapts facade [`Cache`](crate::cache::Cache) to the go frontend cache trait.
        pub fn analyze_with_options(
            db: &mut AnalysisDb,
            cache: &crate::cache::Cache,
            config_hash: &str,
            rule_hash: &str,
            parallel: bool,
        ) -> Vec<Diagnostic> {
            let cache = CacheAnalysisCache::new(cache.clone());
            crate::go::analyze_with_options(db, cache.as_ref(), config_hash, rule_hash, parallel)
        }
    }

    pub mod ts {
        use crate::cache::CacheAnalysisCache;
        use crate::core::AnalysisDb;
        use crate::diagnostics::Diagnostic;

        /// Bench entry: adapts facade [`Cache`](crate::cache::Cache) to the ts frontend cache trait.
        pub fn analyze_with_options(
            db: &mut AnalysisDb,
            cache: &crate::cache::Cache,
            config_hash: &str,
            rule_hash: &str,
            parallel: bool,
        ) -> Vec<Diagnostic> {
            let cache = CacheAnalysisCache::new(cache.clone());
            crate::ts::analyze_with_options(db, cache.as_ref(), config_hash, rule_hash, parallel)
        }
    }
    #[doc(hidden)]
    pub mod keys {
        use crate::config::LoadedConfig;
        use crate::core::{Rule, RuleOptions};
        use std::collections::BTreeMap;
        use std::collections::BTreeSet;

        #[inline]
        pub fn config_hash(config: &LoadedConfig) -> String {
            crate::cache::keys::config_hash(config)
        }

        #[inline]
        pub fn rule_hash(
            rules: &[Rule],
            enabled: Option<&BTreeSet<String>>,
            options: &BTreeMap<String, RuleOptions>,
        ) -> String {
            crate::cache::keys::rule_hash(rules, enabled, options)
        }

        #[inline]
        pub fn deterministic_rule_options(options: &RuleOptions) -> String {
            crate::cache::keys::deterministic_rule_options(options)
        }
    }
}
