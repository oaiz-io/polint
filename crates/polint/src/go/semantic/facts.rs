use crate::internal_core::{FileId, Span, StableKeyId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticPackageId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticFunctionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticCallsiteId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticMethodSetId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticPackageErrorId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticAddressTakenId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticInstantiatedTypeId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticDynamicDispatchId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticRtaEdgeId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GoSemanticFunctionKind {
    Function,
    Method,
    Init,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticCallEdgeId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticInterfaceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticImplementsId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticInstantiationId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticConversionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticDeadCallId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticFieldId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticBuiltinCallId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticParamId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticRouteId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoSemanticRouteServeId(pub u64);

/// How a route's handler or middleware was identified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GoRouteFunctionKind {
    /// A declared function or method, passed by name or as a method value.
    Function,
    /// A function literal.
    Literal,
    /// The value a call returned; named by the function that produced it.
    Factory,
    /// A value read from a struct field nothing visible stored a function in.
    Field,
    /// A value the route interpreter could not identify.
    Unknown,
}

/// One handler or middleware of a route, as the route interpreter named it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoRouteFunction {
    pub name: String,
    pub kind: GoRouteFunctionKind,
    /// The struct field the value was read from (`pkg.Type.Field`), if any.
    pub field: Option<String>,
}

/// What a route serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GoRouteTransport {
    /// An HTTP route: a method and a path.
    Http,
    /// A message subscription: a topic and a handler name.
    Message,
}

/// A route the program registers with a modelled framework: where it is
/// registered, what it serves, its handler, and the middleware a request
/// passes through first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticRouteFact {
    pub id: GoSemanticRouteId,
    pub stable_key: StableKeyId,
    pub framework: String,
    pub transport: GoRouteTransport,
    /// The HTTP method, `*` for any, `?` when it is not a constant; empty for a
    /// message route.
    pub method: String,
    /// The full path (with every group and mount prefix) or the topic; parts
    /// the interpreter could not read are `{?}`.
    pub path: String,
    pub path_complete: bool,
    /// The path the registration call itself names, without the prefixes of
    /// the groups and mounts above it.
    pub registered_path: String,
    /// A message handler's registered name.
    pub name: Option<String>,
    pub handlers: Vec<GoRouteFunction>,
    pub middleware: Vec<GoRouteFunction>,
    pub middleware_complete: bool,
    /// The routers the registration acted on (one per calling context that
    /// reached it with the same result), and the routers they derive from.
    pub routers: Vec<String>,
    pub router_roots: Vec<String>,
    /// The function the registration call is written in.
    pub function: String,
    pub relative_file: Option<String>,
    pub file: Option<FileId>,
    pub span: Option<Span>,
}

/// A call that dispatches requests to a router (an `http.Handler` serve call,
/// a test server): every route below one of `router_roots` is reachable from
/// `function`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticRouteServeFact {
    pub id: GoSemanticRouteServeId,
    pub stable_key: StableKeyId,
    pub function: String,
    pub router_roots: Vec<String>,
    pub relative_file: Option<String>,
    pub file: Option<FileId>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GoSemanticCallStatus {
    ResolvedStatic,
    UnresolvedDynamic,
    Unsupported,
}

/// How a call instruction starts its callee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum GoCallMode {
    #[default]
    Call,
    /// A `go` statement.
    Go,
    /// A `defer` statement.
    Defer,
}

/// Which call-graph algorithm produced a dynamic call's candidate callee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GoCallEdgeAlgorithm {
    /// Variable type analysis: the callee's type or function value reaches the site.
    Vta,
    /// Class hierarchy analysis: the callee's type implements the called interface,
    /// or its signature matches the called function value. Used only for a site
    /// variable type analysis gives no callee.
    Cha,
    /// The abstract interface method an interface call invokes, for a site neither
    /// analysis gives a callee: every implementation of the interface lives in a
    /// dependency loaded without bodies.
    TypeHierarchy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GoGenericKind {
    Type,
    Func,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticPackageFact {
    pub id: GoSemanticPackageId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub package_path: String,
    pub package_name: String,
    pub module_path: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticFunctionFact {
    pub id: GoSemanticFunctionId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub package_path: String,
    pub name: String,
    pub qualified: String,
    pub signature: String,
    pub kind: GoSemanticFunctionKind,
    pub receiver: Option<String>,
    pub relative_file: Option<String>,
    pub file: Option<FileId>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticCallsiteFact {
    pub id: GoSemanticCallsiteId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub package_path: String,
    pub caller: String,
    pub static_callee: Option<String>,
    /// The generic function a static callee instantiates (`pkg.Map` for
    /// `pkg.Map[int string]`), which is the function that has a declaration.
    pub static_callee_origin: Option<String>,
    /// The static type of the receiver operand of a method call: the concrete
    /// (possibly pointer) type for a static method, the interface for an
    /// interface call.
    pub receiver_type: Option<String>,
    pub mode: GoCallMode,
    pub status: GoSemanticCallStatus,
    pub reason: Option<String>,
    pub relative_file: Option<String>,
    pub file: Option<FileId>,
    pub span: Option<Span>,
}

/// The candidate callees of one dynamic (interface or function-value) call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticCallEdgeFact {
    pub id: GoSemanticCallEdgeId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub caller: String,
    pub callsite_stable_key: StableKeyId,
    pub algorithm: GoCallEdgeAlgorithm,
    /// The concrete callees a variable-type or class-hierarchy answer lists, by
    /// the sidecar's function identity, each with its generic origin when it is
    /// an instantiation. Empty for a type-hierarchy answer.
    pub callees: Vec<GoSemanticCallee>,
    /// What a type-hierarchy answer names instead of concrete callees.
    pub abstract_callee: Option<GoAbstractCallee>,
    /// How many concrete callees a class-hierarchy answer had when there were too
    /// many to list.
    pub candidates: Option<u64>,
    pub relative_file: Option<String>,
    pub file: Option<FileId>,
}

/// One concrete candidate callee of a dynamic call.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GoSemanticCallee {
    pub name: String,
    pub origin: Option<String>,
}

/// The abstract callee of a dynamic call no concrete callee is listed for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum GoAbstractCallee {
    /// An interface method, as the interface type and the method name
    /// (`io.Writer.Write`).
    InterfaceMethod(String),
    /// A function value's signature (`func(int) error`).
    Signature(String),
}

/// An interface type declared in a loaded package, with its method names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticInterfaceFact {
    pub id: GoSemanticInterfaceId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub type_name: String,
    pub methods: Vec<String>,
    pub relative_file: Option<String>,
    pub file: Option<FileId>,
    pub span: Option<Span>,
}

/// A concrete type of a loaded package that implements an interface, directly
/// or (`via_pointer`) only through a pointer to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticImplementsFact {
    pub id: GoSemanticImplementsId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub type_name: String,
    pub interface: String,
    pub via_pointer: bool,
}

/// A generic type or function instantiation written in source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticInstantiationFact {
    pub id: GoSemanticInstantiationId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub generic: String,
    pub generic_kind: GoGenericKind,
    pub type_args: Vec<String>,
    pub type_name: String,
    pub relative_file: Option<String>,
    pub file: Option<FileId>,
    pub span: Option<Span>,
}

/// A call expression that is a type conversion, not a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticConversionFact {
    pub id: GoSemanticConversionId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub type_name: String,
    pub relative_file: Option<String>,
    pub file: Option<FileId>,
    pub span: Option<Span>,
}

/// A call expression written in a branch a constant condition rules out
/// (`if false { ... }`, `if debug { ... }` with `const debug = false`): it
/// cannot run in the analysed build, so it calls nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticDeadCallFact {
    pub id: GoSemanticDeadCallId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    /// The function the call is written in.
    pub caller: String,
    pub relative_file: Option<String>,
    pub file: Option<FileId>,
    pub span: Option<Span>,
}

/// A call expression that calls a builtin (`len`, `make`, `panic`, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticBuiltinCallFact {
    pub id: GoSemanticBuiltinCallId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub name: String,
    pub relative_file: Option<String>,
    pub file: Option<FileId>,
    pub span: Option<Span>,
}

/// A field of a struct type declared in a loaded package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticFieldFact {
    pub id: GoSemanticFieldId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub owner: String,
    pub name: String,
    pub index: u32,
    pub field_type: String,
    pub embedded: bool,
    pub tag: Option<String>,
    pub relative_file: Option<String>,
    pub file: Option<FileId>,
    pub span: Option<Span>,
}

/// A declared parameter of a function with source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticParamFact {
    pub id: GoSemanticParamId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub function: String,
    pub index: u32,
    pub name: String,
    pub type_name: String,
    pub variadic: bool,
    pub relative_file: Option<String>,
    pub file: Option<FileId>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticMethodSetFact {
    pub id: GoSemanticMethodSetId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub package_path: String,
    pub type_name: String,
    pub methods: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticPackageErrorFact {
    pub id: GoSemanticPackageErrorId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub package_path: String,
    pub message: String,
}

/// An address-taken Go function — the RTA dispatch-candidate set for func-value
/// callsites (D-05). Harvested from `*ssa.MakeClosure` and `*ssa.Function` value
/// operands in the sidecar. `function` is the official `ssa.Function` `.String()`
/// identity; `stable_key` is length-prefixed from that identity (D-12/D-13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticAddressTakenFact {
    pub id: GoSemanticAddressTakenId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub package_path: String,
    pub function: String,
}

/// An instantiated runtime type — the RTA "rapid type" set: a concrete type converted
/// to an interface via `*ssa.MakeInterface` in the reachable SSA program (D-05). The
/// instantiated-type filter is what distinguishes RTA from coarse CHA. `type_name` is the
/// official `go/types` `.String()` identity; `stable_key` is length-prefixed from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticInstantiatedTypeFact {
    pub id: GoSemanticInstantiatedTypeId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub package_path: String,
    pub type_name: String,
}

/// Dynamic-callsite dispatch detail — the discriminant Plan 2's RTA driver needs to
/// resolve an `UnresolvedDynamic` callsite by method-set matching (D-05). For an interface
/// invoke, `interface_type` + `method` are set; for a func-value call, `signature` is set;
/// honest `None` otherwise (D-08/D-15 — no fabricated discriminant). `callsite_stable_key`
/// joins this detail back to the originating [`GoSemanticCallsiteFact`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticDynamicDispatchFact {
    pub id: GoSemanticDynamicDispatchId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub package_path: String,
    pub caller: String,
    pub callsite_stable_key: StableKeyId,
    pub interface_type: Option<String>,
    pub method: Option<String>,
    pub signature: Option<String>,
}

/// A direct x/tools RTA call-graph edge emitted by the Go sidecar.
///
/// This is intentionally an internal evaluation fact, not a public rule-author API. The
/// existing source-backed solver/refined-call pipeline cannot represent synthetic SSA
/// functions such as `init$1`, generic instantiations, bound method wrappers, or
/// reflection synthetic calls. The external x/tools benchmark therefore consumes this
/// fact directly instead of forcing those oracle identities through source-only facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticRtaEdgeFact {
    pub id: GoSemanticRtaEdgeId,
    pub stable_key: StableKeyId,
    pub package_id: String,
    pub package_path: String,
    pub caller: String,
    pub callee: String,
    pub edge_kind: String,
}
