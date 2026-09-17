//! TypeScript type sidecar: protocol, process client, and fact lowering.
//!
//! This module is an implementation detail of the TS/JS frontend. The sidecar
//! is infrastructure for the type-directed call-graph tier, not a public SDK
//! surface.

pub(crate) mod cache_key;
pub(crate) mod client;
pub(crate) mod diagnostics;
pub(crate) mod facts;
pub(crate) mod lifecycle;
pub(crate) mod lower;
pub(crate) mod process;
pub(crate) mod protocol;
pub(crate) mod provider;
pub(crate) mod store;
pub(crate) mod validate;
