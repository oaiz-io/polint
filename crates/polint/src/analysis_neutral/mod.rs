//! Language-neutral analysis families for polint.
//!
//! Depends only on `polint-core`, `polint-ir`, `polint-analysis-api`, and external
//! crates. Must not depend on concrete frontends (`polint-go` / `polint-ts`) or the
//! facade.

pub mod access_paths;
pub mod adaptation;
pub mod aliases;
pub mod cache_key;
pub mod calls;
pub mod cfg;
pub mod data_flow;
pub mod demand;
pub mod domains;
pub mod entrypoints;
pub mod error;
pub mod evidence;
pub mod extensions;
pub mod fact_store;
pub mod graph;
pub mod hash;
pub mod host;
pub mod identity;
pub mod ids;
pub mod ifds;
pub mod local_db;
pub mod lowering_index;
pub mod metrics;
pub mod mir_body;
pub mod mir_body_compose;
pub mod mir_op;
pub mod mir_validation;
pub mod module_graph;
pub mod places;
pub mod points_to;
pub mod reachability;
pub mod refined_calls;
pub mod semantic_graph;
pub mod slicing;
pub mod solver;
pub mod stable_key;
pub mod store;
pub mod summaries;
pub mod symbol_graph;
pub mod types;
pub mod unknown_taxonomy;
pub mod values;

pub use error::AnalysisError;
pub use host::AnalysisHost;
pub use local_db::LocalAnalysisDb;

/// Provider id for semantic MIR derivation (shared with the facade composition root).
pub const SEMANTIC_MIR_PROVIDER_ID: &str = "polint.semantic_mir";
pub const CFG_PROVIDER_ID: &str = "polint.cfg";
pub const CALLS_PROVIDER_ID: &str = "polint.calls";
pub const ENTRYPOINTS_PROVIDER_ID: &str = "polint.entrypoints";
pub const POLINT_ABSTRACT_DOMAINS_PROVIDER_ID: &str = "polint.abstract_domains";
pub const POLINT_DIRECT_SUMMARIES_PROVIDER_ID: &str = "polint.direct_summaries";
