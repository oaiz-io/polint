//! Go (tree-sitter) frontend: discover Go files, parse in parallel, merge facts and
//! parser diagnostics into a host [`crate::analysis_api::FactDatabase`], with optional
//! disk cache via [`crate::analysis_api::AnalysisCache`].

#[cfg(feature = "lang-go")]
mod adapter;
mod embedded_cache;
pub mod error;
mod frontend;
#[cfg(feature = "lang-go")]
mod grammar;
mod hash;
pub mod lifecycle;
mod local_db;
#[cfg(feature = "lang-go")]
mod mir;
pub mod module_graph;
mod process_runner;
pub(crate) mod rta;
#[cfg(feature = "lang-go")]
#[doc(hidden)]
pub use mir::lower_go_mir;
#[allow(dead_code)]
mod repo_fs;
pub mod semantic;
pub mod semantic_graph;
mod stable_key;
pub mod symbol_graph;
mod syntax_store;

#[cfg(all(test, feature = "lang-go"))]
mod test_cache;
#[cfg(all(test, feature = "lang-go"))]
mod tests;

/// Re-export for `polint::_bench::go`; production callers use the plan-aware entrypoint.
#[cfg(feature = "lang-go")]
#[allow(unreachable_pub, unused_imports)]
pub use adapter::analyze_with_options;
#[cfg(all(test, feature = "lang-go"))]
pub(crate) use adapter::{analyze, analyze_with_cache};
#[cfg(feature = "lang-go")]
#[allow(unused_imports)]
pub(crate) use adapter::{
    analyze_files_with_plan_options_and_cache_stats, analyze_with_plan_options,
    analyze_with_plan_options_and_cache_stats,
};
pub use frontend::{FAMILY_GO, GO_FRONTEND_PROFILE, GoFrontend};
pub use lifecycle::{
    GoAnalysisConfig, GoLifecycleError, GoWorkspaceEnv, go_files, go_work_covers_module_roots,
    workspace_env,
};
pub use syntax_store::{GO_SYNTAX_STORE_FAMILY, GoSyntaxStore};
