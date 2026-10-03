# Routes

`Routes<'_>` is a preview SDK view over the routes a Go program registers with a
web or messaging framework: each route's method and path (or topic), its
handler, and the middleware a request passes through before the handler.
Requesting it derives the `routes` capability.

The typed Go frontend builds the table. It reads the framework calls through
**route models** — data that says which call creates a router, registers a
route, derives a group, adds middleware, mounts a router, subscribes a message
handler, wraps a handler, or serves requests — and interprets the program's
route setup with them:

- It starts from every program's `main` (after the package initializers) and
  follows the functions that create routers or register routes, including
  registrar helpers that receive a router or a group, and closures passed as
  callbacks.
- A group copies its parent's middleware when it is created and extends the
  path prefix; a `Use` call adds middleware for the registrations and groups
  that come after it on that router, not before (gin's copy-at-group
  semantics).
- Routers and middleware stored in struct fields or package variables are
  followed through them: a router built in a constructor and stored in a field
  is the router a method later registers on.
- Each program is interpreted separately; one registration reached by several
  programs (or several calls) lists every router it acted on.
- A registration that no program reaches is still listed, with what is unknown
  about it marked: its path and middleware are not complete.

## Example

```rust
use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/mutating-routes-require-auth",
    description = "Mutating HTTP routes must run the authentication middleware.",
    severity = "error"
)]
fn mutating_routes_require_auth(ctx: &mut RuleCtx<'_>, routes: Routes<'_>) -> RuleResult {
    for route in routes.http().filter(|route| route.method != "GET") {
        let authenticated = route.middleware().any(|middleware| {
            middleware.name.ends_with(".Authenticate")
                || middleware.field.is_some_and(|field| field.ends_with(".Auth"))
        });
        if authenticated || !route.middleware_complete {
            continue;
        }
        let (Some(file), Some(span)) = (route.file, route.span) else {
            continue;
        };
        ctx.report(Diagnostic::error(
            ctx.rule_id(),
            ctx.file_path(file),
            span.diagnostic_range(),
            format!("{} {} does not require authentication", route.method, route.path),
        ));
    }
    Ok(())
}
```

## The route table

| Field or method | Meaning |
|-----------------|---------|
| `framework` | The model that recognized the registration: `gin`, `chi`, `net/http`, `watermill`, or a repository model's `framework`. |
| `transport` | `RouteTransport::Http` or `RouteTransport::Message`. |
| `method` | The HTTP method in upper case; `*` when the route answers any method; `?` when the method is not a constant; empty for a message route. |
| `path` | The full path with every group and mount prefix, or a message route's topic. A part that is not a constant reads `{?}`. |
| `path_complete` | Every part of the path is known. False for a `{?}` part and for a route on a router whose own prefix is unknown. |
| `registered_path` | The path the registration call itself names, without the prefixes of the groups and mounts above it. |
| `name` | A message handler's registered name. |
| `handlers()` | The handler; several when the handler value may be any of them. |
| `middleware()` | The middleware chain, outermost first: what the router had when the route was registered (including what groups copied), then the registration's own inline middleware. |
| `middleware_complete` | The chain is the whole chain. False for a route on a router whose origin the frontend did not see, and when a middleware argument could not be read. A rule should not conclude that a middleware is absent when this is false. |
| `file`, `span` | The registration call. |
| `registered_in` | The function the registration is written in, from `Functions<'_>`. |

Each handler and middleware is a `RouteFunction`:

| Field | Meaning |
|-------|---------|
| `kind` | `Function` (a declared function or method, passed by name or as a method value), `Literal` (a function literal), `Factory` (a value a call returned, named by the call's callee — most middleware is built this way), `Field` (a value read from a struct field nothing visible stored a function in), or `Unknown`. |
| `name` | The qualified name: `example.com/app.health`, `(*example.com/app.Server).create`, a literal's `example.com/app.setup$1`, a factory's callee, or the field `example.com/app.Server.Auth`. |
| `field` | The struct field the value was read from, when it was read from one, such as a middleware a constructor stored in a field. |
| `function` | The `FunctionId` from `Functions<'_>` that `name` names, for a function or factory declared in the scanned sources. |

`Routes<'_>` also answers:

| Method | Meaning |
|--------|---------|
| `iter()`, `http()`, `messages()` | All routes, the HTTP routes, the message subscriptions, in file order. |
| `handled_by(function)` | The routes whose handler is `function`. |
| `served_from(function)` | The routes a serve call in `function` reaches: `ServeHTTP` on a router, or an `httptest` server started with one, reaches every route of that router and of the groups and mounts below it. |
| `complete()` | Whether the interpretation finished. When it stopped at its step budget, the routes listed are the ones found before it stopped, and `ctx.completeness().status_for("routes")` reports `budget_exceeded`. |

## Built-in models

| Framework | Recognized |
|-----------|------------|
| gin | `gin.New`, `gin.Default` (with its logger and recovery middleware); `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, `OPTIONS`, `Any`, `Handle`, `Match` on a `RouterGroup` or an `IRoutes`; `Group` on a `RouterGroup` or an `IRouter`; `Use` on a `RouterGroup`, an `IRoutes` or an `Engine`; `Engine.ServeHTTP`; `gin.WrapF`, `gin.WrapH` as pass-through. |
| chi (`github.com/go-chi/chi` and `/v5`) | `NewRouter`, `NewMux`; `Get` … `Trace`, `Handle`, `HandleFunc`, `Method`, `MethodFunc`; `Route` and `Group` (the callback receives the child), `With`; `Use`; `Mount`; `Mux.ServeHTTP`. |
| net/http | `http.NewServeMux`; `http.HandleFunc`, `http.Handle` (the default mux) and `ServeMux.HandleFunc`, `ServeMux.Handle`, with Go 1.22 method patterns (`"GET /items/{id}"`); `ServeMux.ServeHTTP`; `httptest.NewServer`, `NewTLSServer`, `NewUnstartedServer` as serve calls; `http.StripPrefix`, `http.TimeoutHandler` as pass-through. |
| Watermill | `message.NewRouter`; `AddHandler`, `AddConsumerHandler`, `AddNoPublisherHandler` as message routes (name, topic, handler); router-level `AddMiddleware`. |

## Repository models

A repository describes its own wrappers in `.polint/models/*.toml` with
`[[go_route]]` tables. They are applied before the built-in models, so a
repository model of the same call takes precedence.

```toml
# An in-house subscriber wrapper: Subscribe(topic, handler).
[[go_route]]
framework = "pubsub"
role = "subscriber"
receivers = ["example.com/app/pubsub.Bus"]
methods = ["Subscribe"]
topic_argument = 0
handler_argument = 1

# A decorator that returns the handler it wraps.
[[go_route]]
framework = "local"
role = "passthrough"
function = "example.com/app/decorator.Apply"
argument = 0
```

| Key | Meaning |
|-----|---------|
| `framework` | The name the model's routes report. |
| `role` | `router`, `route`, `group`, `use`, `mount`, `subscriber`, `passthrough` or `serve`. |
| `function` | A package-level function, `import/path.Name`. Either this, or `receivers` and `methods`. |
| `receivers`, `methods` | Methods named in `methods` declared on any of `receivers` (`import/path.Type`; a pointer receiver matches too; interfaces match calls through them). |
| `http_method`, `method_from_name`, `method_argument`, `method_in_pattern` | Where a route's HTTP method comes from: a fixed string, the called method's name, an argument, or the method prefix of a `"GET /path"` pattern. None of them means any method (`*`). |
| `path_argument` | The argument holding the path (routes, groups, mounts). |
| `handler_argument`, `handlers_from` | The handler argument, or the first of a variadic tail whose last element is the handler and the rest inline middleware. |
| `middleware_from` | The first middleware argument of a group or use call. |
| `callback_argument` | A function argument that receives the new group as its first parameter. |
| `router_argument` | The argument holding the router, for a package-level registration or serve call. |
| `default_router` | The implicit router a package-level registration without one uses. |
| `name_argument`, `topic_argument` | A message route's handler name and topic. |
| `argument` | The argument a pass-through returns. |
| `initial_middleware` | Middleware a router constructor installs by itself (qualified function names). |
| `returns_receiver` | The call returns its own router, so a chained call registers on it. |

Argument positions count from 0 and do not include the receiver. A model file
that cannot be parsed, or a table with an unknown key or role, is reported as a
`polint/route-model` warning and left out; the other models still apply. Model
files are part of the cache key: changing one recomputes the routes.

## Coverage and limits

- Routes come from interpreting the program, not from reading registration
  text, so a registration nothing calls appears with `path_complete` and
  `middleware_complete` false rather than with guessed values.
- Struct fields and package variables keep one value per field of a named type,
  whatever value of that type holds it. Two routers stored in the same field by
  one program are both seen at every read of that field.
- Loops and branches in setup code are followed once, in block order; a
  registration repeated in a loop over a table of routes is listed once, with
  the parts read from the table unknown.
- A path or HTTP method built at run time (from configuration, `fmt.Sprintf`)
  reads `{?}` or `?`. Constants, constant concatenation, and string parameters
  bound by the caller are read.
- Middleware is named by what produced it. A middleware that is a closure over
  configuration is named by the constructor that returned it.
- Dependencies are loaded from export data: the frontend sees calls into a
  framework, not the framework's own code, which is why frameworks are modelled
  rather than analyzed.
- `_test.go` files are interpreted only when `[languages.go] include_tests` is
  `true`; serve calls in tests (`served_from`) need it.
- The interpretation is bounded (a step budget and a call depth); a budget stop
  is reported as described for `complete()`.

## Setup

The capability needs the Go toolchain and a `go.mod` module root for the
scanned Go files, like `GoTypes<'_>` (see [Go types](go-semantic-types.md) and
[the consumer setup guide](../CONSUMER-SETUP.md)). When the frontend loaded no
package, `routes` is unavailable: a rule that requests `Routes<'_>` is not run
and `polint check` reports a `polint/capability` diagnostic with status
`setup_missing`. A rule that can work without routes requests
`Option<Routes<'_>>` instead.
