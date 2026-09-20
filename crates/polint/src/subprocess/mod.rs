//! Toolchain-neutral subprocess and sidecar-cache infrastructure.
//!
//! Every language sidecar has the same two problems: run an external program
//! under a wall-clock bound without leaking its descendants, and materialize
//! embedded sources into a per-user directory the process can trust. Both live
//! here so one language frontend never has to reach into another's module for
//! them, and so a timeout is reported with the category of the toolchain that
//! timed out.

pub(crate) mod embedded_cache;
mod runner;

pub(crate) use embedded_cache::{
    SidecarCacheFamily, materialize_embedded_sources, read_verified_private_file,
    verify_private_file, write_private_file,
};
pub(crate) use runner::{SubprocessError, SubprocessOutput, run_bounded};
