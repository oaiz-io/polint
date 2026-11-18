//! Package graphs: the call graph of one package version with its boundary,
//! built once and stitched with the graphs of other packages into the graph
//! of a whole program.
//!
//! A package graph names every node by stable text, never by a run-local id,
//! so the graph of the same package content, engine and model set is the same
//! bytes on any machine and in any checkout.

mod format;
mod identity;
mod layout;

pub(crate) use format::{
    Boundary, BudgetEntry, CallsiteEntry, ConstraintEntry, FunctionEntry, ImportEntry,
    PACKAGE_GRAPH_SCHEMA, PackageGraph, SignatureEntry, SiteEntry,
};
pub(crate) use identity::{CallsiteIdentity, EntryConfig, FunctionIdentity, PackageIdentity};
pub(crate) use layout::package_of;
