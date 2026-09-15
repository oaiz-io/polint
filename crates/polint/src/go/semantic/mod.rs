//! Go semantic sidecar protocol and process client.
//!
//! This module is an implementation detail of the Go frontend. The sidecar is
//! infrastructure for semantic graph producers, not a public SDK surface.

#![allow(dead_code)]

pub(crate) mod budget;
pub mod cache_key;
pub mod client;
pub mod diagnostics;
pub mod facts;
pub mod lower;
pub(crate) mod prefetch;
pub mod process;
pub mod protocol;
pub mod provider;
pub mod store;
pub mod validate;

#[cfg(test)]
mod tests;
