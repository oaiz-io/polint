//! The route fact view: the routes a program registers with a modelled
//! framework, each with its handler and the middleware a request passes
//! through first.

use crate::core::{AnalysisDb, FileId, FunctionId, Span};
use crate::go::semantic::facts::{
    GoRouteFunction, GoRouteFunctionKind, GoRouteTransport, GoSemanticRouteFact,
};

/// Route fact view. Requesting this view maps to the `routes` capability.
///
/// For Go, the typed frontend interprets the program's route setup against
/// framework models: gin, chi, net/http (with `httptest`) and Watermill are
/// built in, and a repository adds its own in `.polint/models/*.toml`
/// (`[[go_route]]` tables). It follows routers through groups, middleware
/// `Use` calls in program order, registrar helpers, struct fields and package
/// variables, starting from every program's `main`; registrations nothing
/// reaches are listed with what is unknown about them marked. See
/// `docs/facts/routes.md`.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct Routes<'a> {
    pub(crate) db: &'a AnalysisDb,
}

/// What a route serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RouteTransport {
    /// An HTTP route: a method and a path.
    Http,
    /// A message subscription: a topic and a handler name.
    Message,
}

/// How a handler or middleware was identified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RouteFunctionKind {
    /// A declared function or method, passed by name or as a method value.
    Function,
    /// A function literal, named after the function that declares it
    /// (`example.com/app.setup$1`).
    Literal,
    /// A value a call returned, named by the function that produced it, such as
    /// a middleware constructor.
    Factory,
    /// A value read from a struct field that nothing visible stored a
    /// function in, named by the field.
    Field,
    /// A value the frontend could not identify.
    Unknown,
}

/// A handler or a middleware of a route.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RouteFunction<'a> {
    /// The identity: a function's or method's qualified name
    /// (`(*example.com/app/http.Server).list`), a literal's name, the function
    /// that produced the value, or the field it was read from. Empty when
    /// `kind` is [`RouteFunctionKind::Unknown`].
    pub name: &'a str,
    /// How the value was identified.
    pub kind: RouteFunctionKind,
    /// The struct field the value was read from (`example.com/app.Server.Auth`),
    /// when it was read from one.
    pub field: Option<&'a str>,
    /// The function from the `Functions` view `name` names, when it is a
    /// function, method or producing function in the scanned sources.
    pub function: Option<FunctionId>,
}

/// One registered route.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Route<'a> {
    /// The framework model that recognized the registration (`gin`, `chi`,
    /// `net/http`, `watermill`, or a repository model's name).
    pub framework: &'a str,
    /// What the route serves.
    pub transport: RouteTransport,
    /// The HTTP method in upper case, `*` for any method, `?` when it is not a
    /// constant; empty for a message route.
    pub method: &'a str,
    /// The full path with every group and mount prefix, or a message route's
    /// topic. Parts that are not constants read `{?}`.
    pub path: &'a str,
    /// Whether every part of the path is known: no `{?}` part, and a router
    /// whose own prefix is known.
    pub path_complete: bool,
    /// The path the registration call itself names (`"/items/:id"`), without
    /// the prefixes of the groups and mounts above it.
    pub registered_path: &'a str,
    /// A message handler's registered name.
    pub name: Option<&'a str>,
    /// Whether the middleware list is the whole chain: false when the route is
    /// registered on a router whose origin the frontend did not see, or when a
    /// middleware argument could not be read.
    pub middleware_complete: bool,
    /// The file of the registration call.
    pub file: Option<FileId>,
    /// The registration call.
    pub span: Option<&'a Span>,
    /// The function the registration is written in.
    pub registered_in: Option<FunctionId>,
    fact: &'a GoSemanticRouteFact,
    db: &'a AnalysisDb,
}

impl<'a> Route<'a> {
    /// The handlers: one for a route whose handler is known, several when the
    /// handler value may be any of them.
    pub fn handlers(&self) -> impl Iterator<Item = RouteFunction<'a>> + 'a {
        let db = self.db;
        self.fact
            .handlers
            .iter()
            .map(move |function| route_function(db, function))
    }

    /// The middleware in the order a request passes through it, outermost first:
    /// what the router had when the route was registered (including what groups
    /// copied from their parents), then the registration's own.
    pub fn middleware(&self) -> impl Iterator<Item = RouteFunction<'a>> + 'a {
        let db = self.db;
        self.fact
            .middleware
            .iter()
            .map(move |function| route_function(db, function))
    }
}

impl<'a> Routes<'a> {
    /// Every route, in file order.
    pub fn iter(self) -> impl Iterator<Item = Route<'a>> + 'a {
        let db = self.db;
        db.go_semantic_routes()
            .iter()
            .map(move |fact| route(db, fact))
    }

    /// The HTTP routes.
    pub fn http(self) -> impl Iterator<Item = Route<'a>> + 'a {
        self.iter()
            .filter(|route| route.transport == RouteTransport::Http)
    }

    /// The message subscriptions.
    pub fn messages(self) -> impl Iterator<Item = Route<'a>> + 'a {
        self.iter()
            .filter(|route| route.transport == RouteTransport::Message)
    }

    /// The routes whose handler is `function`.
    pub fn handled_by(self, function: FunctionId) -> impl Iterator<Item = Route<'a>> + 'a {
        self.iter().filter(move |route| {
            route
                .handlers()
                .any(|handler| handler.function == Some(function))
        })
    }

    /// The routes a serve call in `function` reaches: every route of the
    /// router it serves and of the groups and mounts below it. A test that
    /// builds a router and calls `ServeHTTP` on it, or starts an `httptest`
    /// server with it, reaches each of these handlers.
    pub fn served_from(self, function: FunctionId) -> impl Iterator<Item = Route<'a>> + 'a {
        let db = self.db;
        let roots = db
            .go_types_index()
            .qualified_by_function
            .get(&function)
            .map(|qualified| {
                db.go_semantic_route_serves()
                    .iter()
                    .filter(|serve| &serve.function == qualified)
                    .flat_map(|serve| serve.router_roots.iter().map(String::as_str))
                    .collect::<std::collections::BTreeSet<_>>()
            })
            .unwrap_or_default();
        self.iter().filter(move |route| {
            route
                .fact
                .router_roots
                .iter()
                .any(|root| roots.contains(root.as_str()))
        })
    }

    /// Whether the frontend finished interpreting the program's route setup.
    /// When it stopped at its step budget, the routes listed are the ones found
    /// before it stopped.
    pub fn complete(self) -> bool {
        self.db.go_semantic_route_budget_steps().is_none()
    }
}

fn route<'a>(db: &'a AnalysisDb, fact: &'a GoSemanticRouteFact) -> Route<'a> {
    Route {
        framework: &fact.framework,
        transport: match fact.transport {
            GoRouteTransport::Http => RouteTransport::Http,
            GoRouteTransport::Message => RouteTransport::Message,
        },
        method: &fact.method,
        path: &fact.path,
        path_complete: fact.path_complete,
        registered_path: &fact.registered_path,
        name: fact.name.as_deref(),
        middleware_complete: fact.middleware_complete,
        file: fact.file,
        span: fact.span.as_ref(),
        registered_in: function_named(db, &fact.function),
        fact,
        db,
    }
}

fn route_function<'a>(db: &'a AnalysisDb, function: &'a GoRouteFunction) -> RouteFunction<'a> {
    let kind = match function.kind {
        GoRouteFunctionKind::Function => RouteFunctionKind::Function,
        GoRouteFunctionKind::Literal => RouteFunctionKind::Literal,
        GoRouteFunctionKind::Factory => RouteFunctionKind::Factory,
        GoRouteFunctionKind::Field => RouteFunctionKind::Field,
        GoRouteFunctionKind::Unknown => RouteFunctionKind::Unknown,
    };
    RouteFunction {
        name: &function.name,
        kind,
        field: function.field.as_deref(),
        function: match kind {
            RouteFunctionKind::Function | RouteFunctionKind::Factory => {
                function_named(db, &function.name)
            }
            _ => None,
        },
    }
}

fn function_named(db: &AnalysisDb, qualified: &str) -> Option<FunctionId> {
    db.go_types_index()
        .function_by_qualified
        .get(qualified)
        .copied()
}
