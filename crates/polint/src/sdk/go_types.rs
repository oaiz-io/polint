//! The Go type fact view: what the typed Go frontend knows about functions,
//! struct fields, generic instantiations, method sets and interface
//! satisfaction.

use crate::core::{AnalysisDb, FileId, FunctionId, Span};
use crate::go::semantic::facts::{
    GoGenericKind, GoSemanticFieldFact, GoSemanticImplementsFact, GoSemanticInstantiationFact,
    GoSemanticParamFact,
};

/// Go type fact view. Requesting this view maps to the `go_types` capability.
///
/// Answers come from the typed Go frontend, which loads the module with the Go
/// toolchain. Types are named by the frontend's canonical type strings: a named
/// type by its package path and name (`example.com/app/store.Repo`), a pointer
/// with a leading `*` (`*gorm.io/gorm.DB`), and composite types as the Go type
/// checker spells them (`[]string`, `map[string]int`, `func(int) error`).
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct GoTypes<'a> {
    pub(crate) db: &'a AnalysisDb,
}

/// The typed frontend's view of one function or method.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct GoFunctionType<'a> {
    /// The fully qualified name: `example.com/app/store.NewRepo` for a
    /// function, `(*example.com/app/store.Repo).Save` for a method.
    pub qualified: &'a str,
    /// The signature as the type checker spells it.
    pub signature: &'a str,
    /// The receiver type of a method, `None` for a function.
    pub receiver: Option<&'a str>,
}

/// A parameter of a function or method.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct GoParameter<'a> {
    /// Zero-based position in the parameter list.
    pub index: u32,
    /// The declared name; empty for an unnamed parameter.
    pub name: &'a str,
    /// The parameter type.
    pub type_name: &'a str,
    /// Whether this is the variadic final parameter.
    pub variadic: bool,
}

/// A field of a struct type declared in the analyzed packages.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct GoField<'a> {
    /// The struct type the field belongs to.
    pub owner: &'a str,
    /// The field name; for an embedded field, the embedded type's name.
    pub name: &'a str,
    /// Zero-based position among the struct's fields.
    pub index: u32,
    /// The field type.
    pub type_name: &'a str,
    /// Whether the field is embedded.
    pub embedded: bool,
    /// The raw struct tag, when present.
    pub tag: Option<&'a str>,
    /// The file declaring the field, when it is in the scanned sources.
    pub file: Option<FileId>,
    /// The field declaration, when it is in the scanned sources.
    pub span: Option<&'a Span>,
}

/// Whether an instantiated generic is a type or a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GoGenericTarget {
    /// A generic type (`Handler[T]`).
    Type,
    /// A generic function (`Map[T, U]`).
    Function,
}

/// A generic type or function instantiation written in the scanned sources.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct GoInstantiation<'a> {
    /// The generic being instantiated (`example.com/app/decorator.CommandHandler`).
    pub generic: &'a str,
    /// Whether the generic is a type or a function.
    pub target: GoGenericTarget,
    /// The type arguments, in order.
    pub type_arguments: &'a [String],
    /// The instantiated type as the type checker spells it.
    pub instantiated: &'a str,
    /// The file of the instantiation.
    pub file: Option<FileId>,
    /// The instantiation expression.
    pub span: Option<&'a Span>,
}

/// A concrete type that implements an interface.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct GoImplementation<'a> {
    /// The concrete type.
    pub type_name: &'a str,
    /// The interface it implements.
    pub interface: &'a str,
    /// Whether only a pointer to the type implements the interface.
    pub via_pointer: bool,
}

impl<'a> GoTypes<'a> {
    /// The typed frontend's view of a function from the `Functions` view, when
    /// the frontend loaded it.
    pub fn function(self, function: FunctionId) -> Option<GoFunctionType<'a>> {
        let position = *self.db.go_types_index().function_by_id.get(&function)?;
        let fact = &self.db.go_semantic_functions()[position];
        Some(GoFunctionType {
            qualified: &fact.qualified,
            signature: &fact.signature,
            receiver: fact.receiver.as_deref(),
        })
    }

    /// The receiver type of a method from the `Functions` view.
    pub fn receiver(self, function: FunctionId) -> Option<&'a str> {
        self.function(function)?.receiver
    }

    /// The parameters of a function from the `Functions` view, in order.
    pub fn parameters(self, function: FunctionId) -> impl Iterator<Item = GoParameter<'a>> + 'a {
        let db = self.db;
        let positions = self
            .function(function)
            .and_then(|function| {
                db.go_types_index()
                    .params_by_function
                    .get(function.qualified)
            })
            .map(Vec::as_slice)
            .unwrap_or_default();
        positions
            .iter()
            .map(move |position| parameter(&db.go_semantic_params()[*position]))
    }

    /// The fields of the struct type `owner`, in declaration order.
    pub fn fields_of(self, owner: &str) -> impl Iterator<Item = GoField<'a>> + 'a {
        let db = self.db;
        db.go_types_index()
            .fields_by_owner
            .get(owner)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(move |position| field(&db.go_semantic_fields()[*position]))
    }

    /// Every struct field of the analyzed packages.
    pub fn fields(self) -> impl Iterator<Item = GoField<'a>> + 'a {
        self.db.go_semantic_fields().iter().map(field)
    }

    /// The instantiations of `generic` written in the scanned sources.
    pub fn instantiations_of(
        self,
        generic: &str,
    ) -> impl Iterator<Item = GoInstantiation<'a>> + 'a {
        let db = self.db;
        db.go_types_index()
            .instantiations_by_generic
            .get(generic)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(move |position| instantiation(&db.go_semantic_instantiations()[*position]))
    }

    /// Every generic instantiation written in the scanned sources.
    pub fn instantiations(self) -> impl Iterator<Item = GoInstantiation<'a>> + 'a {
        self.db
            .go_semantic_instantiations()
            .iter()
            .map(instantiation)
    }

    /// The method names in the method set of `type_name`, when the frontend
    /// recorded one.
    pub fn method_set(self, type_name: &str) -> Option<&'a [String]> {
        let position = *self.db.go_types_index().method_set_by_type.get(type_name)?;
        Some(&self.db.go_semantic_method_sets()[position].methods)
    }

    /// The interfaces `type_name` implements.
    pub fn implements(self, type_name: &str) -> impl Iterator<Item = GoImplementation<'a>> + 'a {
        let db = self.db;
        db.go_types_index()
            .implements_by_type
            .get(type_name)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(move |position| implementation(&db.go_semantic_implements()[*position]))
    }

    /// The concrete types that implement `interface`.
    pub fn implementers(self, interface: &str) -> impl Iterator<Item = GoImplementation<'a>> + 'a {
        let db = self.db;
        db.go_types_index()
            .implementers_by_interface
            .get(interface)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(move |position| implementation(&db.go_semantic_implements()[*position]))
    }
}

fn parameter(fact: &GoSemanticParamFact) -> GoParameter<'_> {
    GoParameter {
        index: fact.index,
        name: &fact.name,
        type_name: &fact.type_name,
        variadic: fact.variadic,
    }
}

fn field(fact: &GoSemanticFieldFact) -> GoField<'_> {
    GoField {
        owner: &fact.owner,
        name: &fact.name,
        index: fact.index,
        type_name: &fact.field_type,
        embedded: fact.embedded,
        tag: fact.tag.as_deref(),
        file: fact.file,
        span: fact.span.as_ref(),
    }
}

fn instantiation(fact: &GoSemanticInstantiationFact) -> GoInstantiation<'_> {
    GoInstantiation {
        generic: &fact.generic,
        target: match fact.generic_kind {
            GoGenericKind::Func => GoGenericTarget::Function,
            GoGenericKind::Type => GoGenericTarget::Type,
        },
        type_arguments: &fact.type_args,
        instantiated: &fact.type_name,
        file: fact.file,
        span: fact.span.as_ref(),
    }
}

fn implementation(fact: &GoSemanticImplementsFact) -> GoImplementation<'_> {
    GoImplementation {
        type_name: &fact.type_name,
        interface: &fact.interface,
        via_pointer: fact.via_pointer,
    }
}
