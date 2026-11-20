//! The stored form of a package graph.
//!
//! Nodes are numbered by their position in [`PackageGraph::nodes`], which is
//! sorted by node text, so the numbering is a function of the content. Every
//! other table refers to nodes by that number and is sorted by it. Field
//! order is the serialized order and is part of the format.

use serde::{Deserialize, Serialize};

use super::identity::{CallsiteIdentity, FunctionIdentity, PackageIdentity};
use crate::analysis_api::ImportSyntax;
use crate::analysis_neutral::ids::SemanticNodeId;
use crate::analysis_neutral::points_to::facts::{
    PointsToBudgetStatus, PointsToPrecision, PointsToStatus,
};
use crate::analysis_neutral::points_to::tiers::EdgeClaim;
use crate::analysis_neutral::semantic_graph::constraints::ConstraintKind;

/// The version of the stored form. It changes whenever a field changes.
pub(crate) const PACKAGE_GRAPH_SCHEMA: u32 = 2;

/// The call graph of one package version, its constraints and its boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PackageGraph {
    pub(crate) identity: PackageIdentity,
    /// The stable text of every node; a node's number is its position.
    pub(crate) nodes: Vec<String>,
    /// Every function, each a callable object of the solve.
    pub(crate) functions: Vec<FunctionEntry>,
    /// Every call site the solve answers, resolved or not.
    pub(crate) callsites: Vec<CallsiteEntry>,
    /// The places of each function a call binds.
    pub(crate) signatures: Vec<SignatureEntry>,
    /// The constraints of the package, with node numbers for node ids.
    pub(crate) constraints: Vec<ConstraintEntry>,
    /// The answer of the package's own solve at each call site: its internal
    /// edges and whether the site is complete or open.
    pub(crate) sites: Vec<SiteEntry>,
    pub(crate) boundary: Boundary,
    pub(crate) budget: BudgetEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FunctionEntry {
    pub(crate) node: u32,
    pub(crate) name: String,
    pub(crate) identity: FunctionIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CallsiteEntry {
    pub(crate) node: u32,
    /// The node of the enclosing function, whose identity anchors the
    /// site's.
    pub(crate) caller: u32,
    pub(crate) identity: CallsiteIdentity,
    pub(crate) precision: PointsToPrecision,
    /// The text of the call constraint the site's edges cite.
    pub(crate) constraint: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SignatureEntry {
    pub(crate) function: u32,
    /// Parameter places by position.
    pub(crate) params: Vec<(usize, u32)>,
    pub(crate) rest: Option<u32>,
    pub(crate) this: Option<u32>,
    pub(crate) ret: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct ConstraintEntry {
    /// The constraint, each node id holding a node number.
    pub(crate) kind: ConstraintKind,
    pub(crate) status: PointsToStatus,
    pub(crate) precision: PointsToPrecision,
    pub(crate) key: String,
}

impl ConstraintEntry {
    /// The node number a node id of [`ConstraintEntry::kind`] holds.
    pub(crate) fn node(id: SemanticNodeId) -> u32 {
        u32::try_from(id.0).expect("a node number fits in u32")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SiteEntry {
    pub(crate) callsite: u32,
    pub(crate) status: PointsToStatus,
    pub(crate) claim: EdgeClaim,
    /// The functions the site's callee set holds.
    pub(crate) targets: Vec<u32>,
}

/// What the package reads from other packages and offers them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Boundary {
    /// Each import whose module the package does not hold, sorted.
    pub(crate) imports: Vec<ImportEntry>,
    /// The places holding each file's module values, sorted by file.
    pub(crate) modules: Vec<ModuleEntry>,
    /// The parameters a call the value flow does not describe fills with an
    /// open value: as many as the most any function of the package declares.
    /// A stitch fills that many for the union of the packages.
    pub(crate) guard_arguments: u8,
}

/// Which value of a module an import receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModuleValue {
    /// `module.exports`, what `require` returns.
    Module,
    /// The ES module namespace object.
    Namespace,
}

/// One import of another package: the importing file, the specifier as
/// written, how it is written, and the place that receives the imported
/// module. Analysed alone the place receives an open value, through a copy
/// from `open`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct ImportEntry {
    pub(crate) file: String,
    pub(crate) specifier: String,
    /// `require` or import syntax: the mode the specifier resolves in.
    pub(crate) syntax: ImportSyntax,
    pub(crate) value: ModuleValue,
    pub(crate) place: u32,
    pub(crate) open: u32,
}

/// The places of one file's module values, where the graph holds them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct ModuleEntry {
    pub(crate) file: String,
    pub(crate) module: Option<u32>,
    pub(crate) namespace: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BudgetEntry {
    pub(crate) status: PointsToBudgetStatus,
    /// What exhausted the budget; empty within budget.
    pub(crate) events: Vec<String>,
}
