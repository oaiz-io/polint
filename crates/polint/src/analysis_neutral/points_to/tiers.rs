//! The tier a solver-derived call edge claims, and what the claim rests on.
//!
//! An *exact* edge says the call reaches this function and no other. The
//! solve can say so only for a site whose callee set is closed: no value the
//! constraints do not describe can reach it. A value that enters from outside
//! the package, such as a parameter of an exported function, makes the set
//! *open*; its edges are conservative, and the site records why it is open.
//!
//! An exact claim lists the [`Assumption`]s its derivation used. A
//! package-internal assumption holds for the package on its own; a boundary
//! assumption can be broken by another package the graph is stitched with,
//! and stitching re-checks it.

use serde::{Deserialize, Serialize};

/// A condition an exact claim relies on. The id is stable text, written into
/// the claim and checked when graphs are stitched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Assumption {
    /// No reference named `eval` and no `with` statement, so every write of
    /// a binding is one the symbol graph sees.
    SealedBindings,
    /// The binding holds exactly its initialiser literal.
    SingleLiteral,
    /// Every declaration of the symbol is a function, interface or
    /// namespace, and the symbol is never written.
    DeclarationOnly,
    /// The call's identifier resolves to exactly one symbol.
    UniqueReference,
    /// The type checker resolved the call to one declaration in the tree.
    DeclaredTarget,
    /// The solve reached a fixed point with no budget event.
    WithinBudget,
    /// The import follows module links to one exported declaration.
    ExportLink,
    /// Nothing writes the exported access path after the module initialises.
    ExportNotRewritten,
    /// No code outside the package writes a prototype the call's receiver
    /// inherits through.
    PrototypeClosed,
    /// No value from outside the package reaches the callee expression.
    NoInwardFlow,
    /// No other package stores into a field the callee was read through.
    NoOutsideFieldWrite,
}

/// Whether stitching can break an assumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssumptionScope {
    /// Holds for the package on its own, whatever it is stitched with.
    PackageInternal,
    /// A stitched package can break it; stitching re-checks it.
    Boundary,
}

impl Assumption {
    pub fn id(self) -> &'static str {
        match self {
            Self::SealedBindings => "sealed_bindings",
            Self::SingleLiteral => "single_literal",
            Self::DeclarationOnly => "declaration_only",
            Self::UniqueReference => "unique_reference",
            Self::DeclaredTarget => "declared_target",
            Self::WithinBudget => "within_budget",
            Self::ExportLink => "export_link",
            Self::ExportNotRewritten => "export_not_rewritten",
            Self::PrototypeClosed => "prototype_closed",
            Self::NoInwardFlow => "no_inward_flow",
            Self::NoOutsideFieldWrite => "no_outside_field_write",
        }
    }

    pub fn scope(self) -> AssumptionScope {
        match self {
            Self::SealedBindings
            | Self::SingleLiteral
            | Self::DeclarationOnly
            | Self::UniqueReference
            | Self::DeclaredTarget
            | Self::WithinBudget => AssumptionScope::PackageInternal,
            Self::ExportLink
            | Self::ExportNotRewritten
            | Self::PrototypeClosed
            | Self::NoInwardFlow
            | Self::NoOutsideFieldWrite => AssumptionScope::Boundary,
        }
    }
}

/// How a value the constraints do not describe can reach a set, most
/// specific first. A set reached in several ways reports the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Inflow {
    /// A parameter or rest parameter of a function code outside can call.
    InwardParameter,
    /// `this` of a function code outside can call.
    InwardThis,
    /// A field of an object code outside can write.
    EscapedObject,
    /// The result of a call whose callees the solve could not resolve.
    OpenResult,
    /// A place nothing in the package defines, such as an import.
    OutsideValue,
}

impl Inflow {
    pub const ALL: [Self; 5] = [
        Self::InwardParameter,
        Self::InwardThis,
        Self::EscapedObject,
        Self::OpenResult,
        Self::OutsideValue,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::InwardParameter => "inward_parameter",
            Self::InwardThis => "inward_this",
            Self::EscapedObject => "escaped_object",
            Self::OpenResult => "open_result",
            Self::OutsideValue => "outside_value",
        }
    }

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// The ways values from outside reach one set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct InflowSet(u8);

impl InflowSet {
    pub fn of(inflow: Inflow) -> Self {
        Self(inflow.bit())
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn contains(self, inflow: Inflow) -> bool {
        self.0 & inflow.bit() != 0
    }

    /// The kinds `other` has and `self` lacks.
    pub fn missing(self, other: Self) -> Self {
        Self(other.0 & !self.0)
    }

    pub fn insert_all(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// The most specific kind in the set.
    pub fn first(self) -> Option<Inflow> {
        Inflow::ALL
            .into_iter()
            .find(|inflow| self.contains(*inflow))
    }
}

/// What a solver-derived call edge claims about its site.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeClaim {
    /// The deriver did not assign a tier; the edge's precision decides.
    #[default]
    Unassigned,
    /// The site reaches this function and no other, under the assumptions.
    Exact(Vec<Assumption>),
    /// The site's callee set is closed and holds more than one function:
    /// the site reaches some of them and nothing else.
    Complete,
    /// The site may reach this function; values from outside reach its
    /// callee set, so it may reach others too.
    Open(Inflow),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assumption_ids_match_their_serialized_form_and_are_unique() {
        let all = [
            Assumption::SealedBindings,
            Assumption::SingleLiteral,
            Assumption::DeclarationOnly,
            Assumption::UniqueReference,
            Assumption::DeclaredTarget,
            Assumption::WithinBudget,
            Assumption::ExportLink,
            Assumption::ExportNotRewritten,
            Assumption::PrototypeClosed,
            Assumption::NoInwardFlow,
            Assumption::NoOutsideFieldWrite,
        ];
        let mut ids = std::collections::BTreeSet::new();
        for assumption in all {
            let json = serde_json::to_string(&assumption).expect("serializes");
            assert_eq!(json, format!("\"{}\"", assumption.id()));
            assert!(ids.insert(assumption.id()));
        }
        assert_eq!(
            Assumption::WithinBudget.scope(),
            AssumptionScope::PackageInternal
        );
        assert_eq!(Assumption::NoInwardFlow.scope(), AssumptionScope::Boundary);
        assert_eq!(
            Assumption::PrototypeClosed.scope(),
            AssumptionScope::Boundary
        );
    }

    #[test]
    fn inflow_set_reports_the_most_specific_kind_and_what_is_new() {
        let mut set = InflowSet::of(Inflow::OutsideValue);
        assert_eq!(set.first(), Some(Inflow::OutsideValue));
        let added = InflowSet::of(Inflow::InwardThis);
        assert_eq!(set.missing(added), added);
        set.insert_all(added);
        assert_eq!(set.first(), Some(Inflow::InwardThis));
        assert!(set.missing(added).is_empty());
        assert!(InflowSet::default().first().is_none());
        for inflow in Inflow::ALL {
            let json = serde_json::to_string(&inflow).expect("serializes");
            assert_eq!(json, format!("\"{}\"", inflow.label()));
        }
    }
}
