//! Interprocedural taint over flow programs.
//!
//! A frontend lowers each function body into a [`ir::FlowProgram`]: numbered
//! value slots and the moves between them (copies, loads and stores through
//! access paths, calls with their candidate callees, returns, closures). The
//! solver answers source-to-sink questions over it with summaries per (body,
//! entry fact) reused across callers, k-limited access paths, models of library
//! functions as data, and per-unit step budgets and a run deadline reported as
//! unknowns. Go lowers its programs from the semantic sidecar's SSA.

pub(crate) mod index;
pub(crate) mod ir;
pub(crate) mod models;
pub(crate) mod path;
pub(crate) mod solver;

#[cfg(test)]
mod tests;
