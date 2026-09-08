//! TypeScript / JavaScript (Oxc) frontend: discover TS/JS files, parse in parallel,
//! merge facts and parser diagnostics into a host [`crate::analysis_api::FactDatabase`],
//! with optional disk cache via [`crate::analysis_api::AnalysisCache`].

#[cfg(feature = "lang-typescript")]
mod adapter;
#[cfg(not(feature = "lang-typescript"))]
mod unavailable;
#[cfg(not(feature = "lang-typescript"))]
pub use unavailable::{semantic_graph, token_flow};
pub mod binding;
mod callable_flow;
pub mod error;
mod frontend;
#[cfg(test)]
mod hash;
pub mod ids;
pub mod inventory;
mod local_db;
#[cfg(feature = "lang-typescript")]
mod mir;
pub mod module_graph;
#[cfg(feature = "lang-typescript")]
#[doc(hidden)]
pub use mir::lower_ts_mir;
pub mod object_model;
#[cfg(feature = "lang-typescript")]
pub mod parse;
pub mod points_to;
#[allow(dead_code)]
mod repo_fs;
pub mod scope;
#[cfg(feature = "lang-typescript")]
pub mod semantic_graph;
pub(crate) mod semantic_graph_build;
#[cfg(feature = "lang-typescript")]
pub mod spans;
#[cfg(feature = "lang-typescript")]
pub mod symbol_graph;
pub mod syntax_store;
#[cfg(feature = "lang-typescript")]
pub mod token_flow;

use std::cell::RefCell;
use std::sync::Arc;

use crate::internal_core::{StableKeyId, StableKeyInterner};

#[cfg(all(test, feature = "lang-typescript"))]
mod test_cache;
#[cfg(all(test, feature = "lang-typescript"))]
mod tests;

thread_local! {
    static FRONTEND_STABLE_KEYS: RefCell<Option<StableKeyInterner>> = const { RefCell::new(None) };
}

struct FrontendStableKeysGuard {
    previous: Option<StableKeyInterner>,
}

impl Drop for FrontendStableKeysGuard {
    fn drop(&mut self) {
        FRONTEND_STABLE_KEYS.with(|slot| {
            slot.replace(self.previous.take());
        });
    }
}

pub fn with_frontend_stable_keys<T>(
    interner: &StableKeyInterner,
    operation: impl FnOnce() -> T,
) -> T {
    let previous = FRONTEND_STABLE_KEYS.with(|slot| slot.replace(Some(interner.clone())));
    let _guard = FrontendStableKeysGuard { previous };
    operation()
}

pub fn intern_frontend_stable_key(key: String) -> StableKeyId {
    FRONTEND_STABLE_KEYS.with(|slot| {
        slot.borrow()
            .as_ref()
            .expect("TS frontend extraction requires a stable-key interner")
            .intern(key)
    })
}

pub fn resolve_frontend_stable_key(key: StableKeyId) -> Arc<str> {
    FRONTEND_STABLE_KEYS.with(|slot| {
        slot.borrow()
            .as_ref()
            .expect("TS frontend extraction requires a stable-key interner")
            .resolve(key)
    })
}

#[cfg(feature = "lang-typescript")]
pub use parse::{PARSER_RECOVERY_CONSTRUCT, parse_ts_file};

pub use crate::analysis_api::{anonymous_callable_name, is_anonymous_callable_name};
/// Re-export for `polint::_bench::ts`; production callers use the plan-aware entrypoint.
#[cfg(feature = "lang-typescript")]
#[allow(unreachable_pub, unused_imports)]
pub use adapter::analyze_with_options;
#[cfg(feature = "lang-typescript")]
#[allow(unused_imports)]
pub use adapter::{
    DYNAMIC_IMPORT_SPECIFIER, analyze_files_with_plan_options_and_cache_stats,
    analyze_with_plan_options, analyze_with_plan_options_and_cache_stats, class_callable_name,
};
#[cfg(all(test, feature = "lang-typescript"))]
pub(crate) use adapter::{analyze, analyze_with_cache};

pub use frontend::{FAMILY_TYPESCRIPT_JAVASCRIPT, TS_FRONTEND_PROFILE, TsJsFrontend};
pub use syntax_store::{TS_SYNTAX_STORE_FAMILY, TsSyntaxStore};
