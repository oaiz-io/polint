# 14 — Are framework route models the right layer? An adversarial evaluation (2026-10-04)

The owner asked:

> "Are we sure that this is the right direction? Isn't this something that should be inside the
> custom rules stuff for each repo? Or what? I am OK that this is the right direction... but we
> need to be sure! Or is it that we need to improve how our engine works to not need this?"

The subject is the route-model design on `perf/deep-analysis` (`ebeee581`):

- framework knowledge is data: built-in tables for gin, chi, net/http + httptest and Watermill;
- a repository extends it with `[[go_route]]` tables in `.polint/models/*.toml`;
- the semantic sidecar interprets eight roles: `router`, `route`, `group`, `use`, `mount`,
  `subscriber`, `passthrough`, `serve`.

The implementer's claim is that the engine owns the role semantics and the data owns which
functions play each role. This note tests that design against three alternatives, using code and
measurements on the OAIZ bench and a synthetic fixture.

## Verdict: KEEP BUT MOVE

**Keep** the shape of the design:
- built-in models ship with polint;
- repositories extend them with the same kind of data;
- the engine interprets the roles.

Neither alternative survives measurement:

- **Custom-rules-only** (no built-ins) drops OAIZ's route inventory from 331 rows to **0**. It also
  silently swaps the endpoint-authority rule's 4 true findings for 81 false ones. Nothing warns.
- **Engine-smarter** (recognize registrations by shape) is about 48% precise in its loose form.
  The strict form misses all 19 Watermill subscriptions and every `Use`. No form can tell gin's
  `Use` from chi's `With`: their shapes are the same and their semantics are opposite.
- Every mature analyzer we compared ships framework models with the tool and lets the repository
  add to them. None asks repositories to declare mainstream frameworks themselves.

**Move** three things. The current layering puts them in the wrong place:

1. **Framework-specific semantics are hard-coded in the roles. Move them into model parameters.**
   - Today the `use` role always means gin's rule: middleware covers later registrations only.
     That is wrong for Watermill, echo and gorilla/mux. In those frameworks, router-level
     middleware covers every route, whenever it was added.
   - No role can attach middleware to one registration after the fact, as Watermill's
     `(*Handler).AddMiddleware` does. This hits OAIZ today: 9 of its 26 subscriptions report an
     empty, *complete* middleware chain that is really three or four middleware long.
   - A repository model cannot fix either gap (tested). The engine's role vocabulary has to grow.
     The new parameters belong in the data: when a `use` applies, middleware attached to a
     returned handle, and a method set by a chained call.
2. **The built-in table lives in the wrong place and format. Move it to polint's Rust side, in the
   `[[go_route]]` TOML format repositories write.**
   - Today it is JSON embedded in the Go sidecar (`route_models.json`, `go:embed`), and the role
     list is validated twice, once in Rust and once in Go.
   - The data-flow built-ins already live in Rust (`go/flow_models.toml`), so one pattern
     currently has two homes, two formats, and two combination rules: routes override, flows add.
3. **Failures are silent today. Add diagnostics.**
   - Warn when a repository model shadows a built-in or another repository model.
   - Allow a model to disable a built-in (CodeQL's "neutral" model).
   - Report registrations that look like an unmodeled framework (`polint/route-model`
     diagnostics). This is the one useful job for the "engine-smarter" idea: catching missing
     models, not replacing them.

Nothing here argues for moving framework knowledge into each repository's rules, or for dropping
models in favour of inference.

### The three strongest pieces of evidence

1. **Cold start without built-ins is a total, silent loss (measured).**
   - **Scans compared:** OAIZ bench copy, cold, 4 jobs, same rule host. The shipped sidecar was
     compared with one whose built-in table was emptied, and OAIZ has no model files.
   - **What disappears:**
     - HTTP routes go from 305 to 0, and message subscriptions from 26 to 0.
     - The rewritten endpoint-authority rule loses all 4 true findings.
     - It reports 81 false "declared … but registers no route" findings instead.
   - **What polint reports about it:** nothing; there is no engine diagnostic.
   - **What it takes to restore:** OAIZ has to write a 64-line, 8-table models file. That brings
     back the identical endpoint-authority findings but still drops 3 net/http routes, because
     the file's author did not think to model net/http.
2. **Shape-based recognition cannot replace models (measured on OAIZ core, 380 packages).**
   - **Loose shape** (a string parameter followed by a function-like one): 750 call sites
     flagged, 391 of them not registrations, so 48% precision.
   - **Strict shape** (a constant `"/…"` argument plus a function): 250 of 252 sites are real, but
     it finds:
     - 244 of 302 gin route registrations;
     - 0 of 19 Watermill subscriptions;
     - 6 of 34 `Group` calls;
     - no `Use` calls.
   - **Roles:** shape cannot tell `GET` from `Group` from `http.StripPrefix`. gin's `Use` (mutates
     its receiver) and chi's `With` (returns a new router) have the same shape.
3. **The status quo is right in kind but has real holes that data cannot fill (measured on a
   fixture and on OAIZ).**
   - **gin built-ins:**
     - `Match` is reported with method `?`;
     - `Static` and `NoRoute` routes are missing.
   - **Watermill:** handler-level `AddMiddleware` is lost, and so is router-level middleware added
     after a handler. The chain is reported as `[]` with `middleware_complete = true`.
   - **echo, with repository models:**
     - a route registered before `e.Use(auth)` reports no auth;
     - inline middleware after the handler is dropped;
     - both are marked complete.
   - **gorilla/mux:** `.Methods("POST")` is lost (method `*`).
   - **What a repository model can rescue in OAIZ today:** nothing. OAIZ has zero model files, and
     its 292/292 inventory came entirely from built-ins plus the engine's interprocedural
     interpretation.

## 1. What was evaluated, exactly

**Where the built-ins live.**
- `crates/polint/src/go-sidecar/polint-go-frontend/internal/semantic/route_models.json` holds 38
  model entries: gin, chi (v1 and v5 paths), net/http + httptest, and Watermill.
- `routes.go:85` embeds it with `//go:embed`. `BuiltinRouteModels()` (`routes.go:89`)
  parses it.

**Where repository models enter.**
- `crates/polint/src/go/route_models.rs:129` reads `.polint/models/*.toml` in file-name order.
- `route_models.rs:162-167` validates each table; `ROLES` is listed at `route_models.rs:18`.
- The tables are serialized as JSON and written to a temp file
  (`go/semantic/client.rs:270`), then passed as `--route-models` (`client.rs:319-320`).
- `main.go:55-63` loads them and calls `WithBuiltinRouteModels`.
- The repository models are part of the semantic cache key (`go/semantic/cache_key.rs:71`).

**Precedence, verified in code and by a run.**
- `WithBuiltinRouteModels` (`routes.go:104-116`) puts the repository models first.
- `newRouteModelIndex` (`routes.go:144-171`) keeps the *first* model per key and skips later ones
  silently (`if _, seen := …; !seen`).
- The key is `function`, or `receiver.method` per receiver.
- Consequences:
  - A repository model beats a built-in.
  - Between two repository files, the first by file name wins.
  - The override is per receiver: a repository model naming only `gin.RouterGroup.GET` still
    leaves `gin.IRoutes.GET` (calls through the interface) on the built-in.
- **Run:** `a.toml` and `b.toml` both model `gin.RouterGroup.GET`, as frameworks `repo-a` and
  `repo-b`. The route reports `framework=repo-a`. No diagnostic is raised for the shadowed
  built-in or the shadowed `b.toml`.
- **Data flow differs:** `go/flow_models.rs:36` adds repository flow tables to the built-ins.
  Neither format can switch a built-in off.

**What the engine owns** (`routes.go:1035-1121`, `record` at `routes.go:1335`):
- `group` copies the parent's middleware at creation time.
- `use` appends middleware to the router object.
- `record` snapshots the router's middleware *at the registration call*.
- `subscriber` records the route and returns nothing.
- `passthrough` returns argument N.
- Interprocedural interpretation:
  - roots are the package initializers, then each `main`;
  - relevance follows static calls, closures, function values by signature, and interface calls
    by implementation (`routes.go:492-594`);
  - routers are followed through fields and globals.

**Capability wiring.**
- `GoSemanticRequest.routes` sends the repository models only when routes are requested
  (`go/semantic/provider.rs:129-130`).
- The SDK view is `Routes<'_>`; the docs are `docs/facts/routes.md`.

## 2. Measurement setup

| Item | Value |
|---|---|
| Engine | this branch, `ebeee581` |
| Rule host | the gate-2 scratch pack (`scratch/oaiz-routes`: original endpoint discovery dump, `Routes<'_>` dump, Routes-based endpoint-authority), path-patched to this worktree; built into its own target dir |
| Sidecar variants, each passed via `POLINT_GO_FRONTEND` | **stock**: built from this tree. **nobuiltin**: identical except `route_models.json` = `{"models": []}` |
| Corpus | a `cp -a` copy of `/opt/data/polint-next-direction/oaiz-bench` (OAIZ `2956a791a7`; the gate-2 corpus). The original was not touched and is `git status`-clean |
| Run | `check --format json --fail-on none core`; cold, fresh `POLINT_CACHE_DIR` per run, 4 jobs; GOROOT go1.27.0, and the module's toolchain resolved to go1.27.1 via `GOTOOLCHAIN=auto` |
| Comparison | `/opt/data/polint-deep/tools/route_inventory.py`, unchanged from gate 2 |
| Raw files | `/opt/data/polint-routemodels-eval/`: `runs/*.json`, `runs/*-inventory.md`, `run.sh`, `models-*`, `fix-echo/`, `shapeprobe.out` |

Environment notes:
- The first rule-host build died with `EAGAIN` because the shared container hit its pids limit. It
  succeeded on a re-run at `-j 3`.
- `/opt/data/polint-deep/oaiz-edit`, the path named in the brief, does not exist on this host.
  The gate-2 corpus above was used instead.

## 3. Alternative 1 — custom-rules-only (no built-ins; every repository declares its frameworks)

**Steelman.**
- polint stays framework-agnostic and ships no third-party API knowledge, so there is no semver
  exposure to gin or Watermill.
- Each repository models exactly the APIs and versions it uses, so nothing it does not use can
  misfire.
- Repository-specific wrappers and frameworks sit beside the framework models, in one place.
- The engine still owns the hard part, the role semantics, which keeps that knowledge reusable.
- Semgrep puts framework knowledge in rules, not in the engine. This is the same idea.

**Measured cold start (the real cost):**

| Run | HTTP routes | message routes | original endpoints found | endpoint-authority findings | engine warning |
|---|---|---|---|---|---|
| A: shipped (built-ins, no repository models) | 305 (302 complete) | 26 | 292 / 292 | 4 (the gate-2 set) | — |
| B: no built-ins, no repository models | **0** | **0** | **0 / 292** | **81, all false**: "`<handler>` is declared under `[rules.config.authority]` but registers no route"; the 4 true findings are gone | **none** (only the pre-existing `polint/scope` note) |
| C: no built-ins, OAIZ writes `frameworks.toml` (8 tables, 64 lines: gin `New`/routes/`Any`/`Group`/`Use`, Watermill router/subscriber/`AddMiddleware`) | 302 | 26 | 292 / 292 | 4, identical to A | — |

Run A reproduces the gate-2 inventory exactly: 292/292, 13 typed-only rows, 93 reclassified, 26
subscriptions.

What breaks:

- **The "works without config" promise.** A new repository with gin gets an empty `Routes<'_>`.
  It does not get a capability diagnostic, because the provider ran fine and simply matched
  nothing.
  - A route-security rule then either passes vacuously (a new repository with no authority
    config: zero findings) or floods with false positives (OAIZ: 81).
  - This is the worst failure mode the product has: a green check that means nothing.
  - It violates the Truthfulness constraint in spirit, because the view reports
    `complete() == true` while missing every route.
- **Quality depends on what the repository author remembers.** Run C missed OAIZ's 3 net/http
  routes in an integration-test helper. They are not product routes, but it shows the problem: a
  model file is written from what the author thinks the code uses. Built-ins cover what the code
  actually calls.
- **Duplication.** The Go+TS monorepo uses the same stack (gin in 101 files, Watermill in 87,
  [13](13-deep-analysis.md) §2.6). It would need the same 8 tables, and every other gin user the
  same again. Built-ins are those tables written once.
- **Drift is silent.** A model keyed on a function name that disappears or moves (chi's `/v5`
  import path; Watermill adding `AddConsumerHandler` and deprecating `AddNoPublisherHandler`) does
  not fail; it just stops matching.
  - With built-ins, one polint release fixes every repository.
  - With per-repository files, each repository has to notice a loss that nothing reports.

**Maintenance delta (estimate, from gin's CHANGELOG in the module cache, v1.3.0 → v1.12.0):**

- **Major versions:** gin has had no major version bump in the CHANGELOG's range.
- **Route-table-relevant API additions:** `StaticFileFS` (v1.8.0) and `Match` (v1.9.0, "match
  method added to routergroup"). There were none in v1.10–v1.12.
- **Rate:** roughly one route-relevant change per framework every 1.5–3 years (estimate).
- **Built-in upkeep:** one polint change per event, about 0.5–1 day with a fixture test
  (estimate), shipped to all repositories at their next polint upgrade.
- **Per-repository upkeep:** the same edit times repositories × frameworks, plus an unbounded
  *detection* cost, because the failure is silent.
- **A hypothetical gin v2:** the import path changes, so every name-keyed model (built-in or
  repository) stops matching every gin route at once. That is the strongest argument for
  centralizing the table *and* for the unmatched-registration diagnostic in the verdict.

**Verdict on alternative 1:** reject. The measured cold-start loss is 331 → 0 routes, plus a
rule-quality inversion that nothing reports. The "repository owns it" benefit already exists in
the shipped design: repository models are applied first and override.

## 4. Alternative 2 — engine-smarter (no models: infer registrations)

**Steelman.**
- Models are a maintenance tax and a coverage ceiling: anything unmodeled is invisible.
- Registration APIs share a recognizable shape: a path string, then handler functions.
- The SSA program already has the values, so a flow-derived route table (where handler values
  end up, plus pointer analysis) could find routes in any framework, including in-house ones.
- Decorators could be inferred by signature (a function `H → H`).
- Pysa's model generators show inference can replace hand-written models for Django URL configs.

**Measured: shape recognition on OAIZ core.** A typed `go/packages` probe
(`/opt/data/polint-routemodels-eval/shapeprobe/main.go`) ran over 380 packages, excluding tests,
in 183 s:

| Shape | sites flagged | in modeled frameworks | elsewhere | precision |
|---|---|---|---|---|
| S1: a `string` parameter followed by a function-like parameter (func type, or an interface with `ServeHTTP`) | 750 (99 distinct callees) | 359 (gin 336, Watermill 19, net/http 4) | 391 (`firecrawlError` 80, `attioEventExample` 47, `singleflight.Group.Do`, `strings.IndexFunc`, …) | **48%** |
| S2: S1, plus the string argument is a constant starting with `/` | 252 | 250 gin, 1 `http.StripPrefix` (a pass-through, not a route) | 1 | 99%, but see recall |

Recall of S2, against the 302 gin route registrations S1 sees:
- **244 of 302** gin route registrations; the 58 it misses are base-path `""` routes and paths
  built from variables or parameters;
- **0 of 19** Watermill subscriptions, because a topic is not `"/…"`;
- **6 of 34** `Group` calls;
- **0** `Use` / `AddMiddleware` calls, which take no string.

The roles are where inference fails structurally, not just statistically:

- **Same shape, opposite semantics.**
  - gin `Use(...HandlerFunc) IRoutes` mutates its receiver and returns it.
  - chi `With(...func(http.Handler) http.Handler) Router` returns a *new* inline router and leaves
    the receiver alone.
  - A route-security rule built on the wrong guess either inherits auth that is not there or
    misses auth that is.
- **Same shape, different roles.** `GET(path, handlers...)` registers a route,
  `Group(path, handlers...)` derives a router, and `StripPrefix(prefix, h)` wraps a handler.
  Telling them apart needs the *name*, which is a model again.
- **When middleware applies is invisible in any signature.**
  - gin snapshots middleware at registration;
  - echo's `e.Use`, gorilla's `Use` and Watermill's router-level `AddMiddleware` apply at
    serve/run time to every route (verified in echo v4.13.3 `echo.go:472-473,647-661`, gorilla
    v1.8.1 `mux.go:143-144`, Watermill v1.5.3 `router.go:184-197,474-476`).
- **Flow-derived tables need framework bodies.**
  - The sidecar loads dependencies from export data; it does not analyze them. That is what made
    it affordable ([13](13-deep-analysis.md) §2.2: 7.4 GB / 21 s with dependencies from source,
    1.5 GB / 5.7 s warm with export data).
  - Interpreting gin's own `combineHandlers`/`addRoute` brings that cost back. It still needs a
    spec saying that a write into `engine.trees` *is* a route and that `group.Handlers` *is* the
    middleware chain, which is a model one level lower and more brittle (internal fields change
    more often than exported API).
- **Decorator inference by signature** (`H → H`) would mark every `ApplyCommandDecorators[C, R]`
  as a pass-through. OAIZ has 426 of those, and they are CQRS wrappers, not route wrappers. It
  would also mark any `func(HandlerFunc) HandlerFunc` middleware *factory* as transparent, which
  erases exactly the middleware a security rule looks for.
- **Where the configuration would go instead:** per-rule heuristics (each rule deciding which
  shapes it trusts) or per-repository exclusion lists for false positives. Both are worse homes
  than a model file: they are less reviewable, not shared, and not in the cache key.

**What engine-smarter *is* good for:**
- **Detection, not replacement.** Shape S2 is 99% precise. Pointed only at calls into packages
  *no model covers*, it is a cheap cold-start guard: "`example.com/ext/router.Handle("/x", h)`
  looks like a route registration in an unmodeled package; add a `[[go_route]]` model." That
  turns alternative 1's silent zero into an explicit diagnostic.
- **Assisted model generation.** The same detector can draft the model, in the spirit of Pysa's
  generators, for a human to confirm.

**Verdict on alternative 2:**
- As a replacement for models: **never**. Role semantics are not recoverable from shape, and
  analyzing framework bodies costs what export-data loading saved.
- As a guard and model suggester: **soon**. It is small, uses the same sidecar pass, and is a
  diagnostic rather than an SDK surface.

## 5. Alternative 3 — the status quo, attacked

### 5.1 Is the role list complete for real frameworks? No, and data cannot close the gaps

A synthetic fixture (`/opt/data/polint-routemodels-eval/fix-echo`, one `main` exercising gin
v1.12.0, echo v4.13.3, gorilla/mux v1.8.1, fiber v2.52.6 and Watermill v1.5.3) was scanned twice.
The first run used built-ins only. The second also used the best repository models I could write
for echo, gorilla, fiber and Watermill handler middleware (`models-fixture/frameworks.toml`).

| Case | Truth | Reported | Repository model fixes it? |
|---|---|---|---|
| gin `r.Static("/assets", dir)` | GET+HEAD `/assets/*filepath` | **missing** | yes (a `route` model with `http_method`); one row per method is not expressible |
| gin `r.NoRoute(h)` | catch-all handler | **missing** | no role for a fallback handler |
| gin `r.Match([]string{"GET","POST"}, "/match", h)` (built-in) | GET and POST | **`? /match`** | no: `method_argument` reads a string, not a list |
| Watermill `AddConsumerHandler(…).AddMiddleware(retry)`, then `router.AddMiddleware(metrics)` | [metrics, retry] | **`[]`, `middleware_complete = true`** | **no.** Tested: a `use` model on `message.Handler` has no effect, because `subscriber` returns no handle |
| echo `e.GET("/early", h)`, then `e.Use(auth)` | auth applies (echo applies root middleware in `ServeHTTP`) | **`[]`, complete** | no: `use` always means "later registrations only" |
| echo `e.POST("/inline", h, audit)` | [auth, audit] | **[auth], complete**: inline middleware after the handler dropped | no: `handlers_from` assumes the handler is *last*, and `handler_argument` drops the tail without marking it incomplete |
| echo `admin := e.Group("/admin", audit)`; `admin.DELETE(…)` | [auth, audit] | [auth, audit] ✓ | — |
| gorilla `r.HandleFunc("/orders", h).Methods("POST")` | POST | **`*`** | no role for "method set by a chained call" |
| gorilla `r.PathPrefix("/api").Subrouter()`; `api.Use(auth)` | prefix `/api`, auth | ✓ (`PathPrefix` and `Subrouter` as two `group` models) | — |
| fiber `app.Use("/api", auth)` | auth on `/api/*` only | auth on `/public` too, marked incomplete (the prefix string reads as `unknown`) | no prefix-scoped `use` |

The same gap hits **OAIZ today**, not just the fixture:
- 9 of its 26 Watermill subscriptions (`internal/ingest/ports/subscriber.go:106` and the helper at
  `:186`) attach Retry, terminal-error classification, metrics and an exhausted-delivery guard
  through `(*message.Handler).AddMiddleware`.
- `Routes<'_>` reports them with an empty chain and `middleware_complete = true`.
- A rule such as "every subscription retries" would report 9 false positives that the
  completeness flag says are trustworthy.

What this shows:
- **The implementer's claim is half right.** The engine should own role semantics, and the
  fixture shows the data cannot patch around missing semantics. But today's roles *are* gin's and
  chi's semantics, hard-coded (`record` snapshots middleware at registration, `routes.go:1343`).
- **What is framework-specific has to become model parameters:**
  - `use.applies = "later" | "all"`;
  - `route.middleware_after_handler`;
  - a route handle that later `use`-like calls can target;
  - `method_from_call`;
  - a method list.
- **The engine's job is to interpret those parameters, and to mark a chain incomplete when it
  cannot**, not to bake in one framework's rules.
- **Truthfulness:** several of the rows above are *wrong and marked complete*. The minimum fix,
  even before the new roles, is to mark a route incomplete when its registration's result is used
  by a call the engine does not model (for example, `.AddMiddleware`, `.Methods`).

The other frameworks the brief names:
- **kratos** registers through generated code, with middleware set by server options. With
  `passthrough`/`route` models of the generated `Register…HTTPServer` helpers, the path tables
  would read. Middleware set by options has no role.
- **connectRPC** passes the `(path, handler)` tuple from `NewXServiceHandler` into
  `mux.Handle`. Interceptors are options again.

Both are **unmeasured** here. I expect both to need an "options carry middleware" role.

### 5.2 Is the built-in table a semantic-versioning liability?

- **Changes are visible.** A table change changes reported routes, and so rule results. The
  embedded table is part of the sidecar sources, and those are digested into the frontend cache
  key, so a changed table recomputes routes. That is correct.
- **The cost is a release-note duty.** Treat table changes like rule-behaviour changes. The rate
  is about one relevant change per framework every 1.5–3 years (estimate, §3).
- **Versions are not modeled, and that is the real exposure.**
  - The table keys on import paths. A framework that changes semantics *without* changing its
    path (a minor release that changes when middleware applies) would be modelled wrongly for
    one of the versions in use. That has not happened in gin's v1 CHANGELOG range.
  - Nothing in the format can say "this model applies from v1.9". Today that is fine; it is a
    trigger below.

### 5.3 Does `[[go_route]]` leak engine internals into consumer repositories?

**Somewhat:**
- `handlers_from` versus `handler_argument`, `returns_receiver` and `callback_argument` are
  interpreter concepts.
- Argument positions exclude the receiver.
- `deny_unknown_fields` makes typos loud, which is good.

**This is the industry norm:** CodeQL's models-as-data rows say `Argument[0]`, `ReturnValue`,
`Argument[receiver]`; Joern's semantics use `0` for the receiver and `-1` for the return.

**The real leak is different:** the repository format is not the format the built-ins are written
in (JSON in Go, not `[[go_route]]` TOML), and the two validators can drift. Writing the built-ins
as a shipped `[[go_route]]` file makes polint the first consumer of its own format. Keep the view
and the table format **preview** until the role additions in §5.1 land, because they will add
fields.

### 5.4 What happens when two models disagree?

- The first by key wins, silently (verified, §1).
- A repository model can only *replace* a built-in per key, not disable it.
- A partial override, for example on the struct but not the interface, splits one framework
  across two models.
- Data flow uses union semantics instead.

**Fix:**
- report shadowing as a `polint/route-model` note;
- add a `neutral`/`disabled` model;
- document a single combination rule for both model families.

### 5.5 Did the repository-model mechanism already rescue something in OAIZ? No

- `.polint/models` does not exist in the OAIZ bench tree.
- The gate-2 inventory (292/292) and the 89 group-inheritance reclassifications through registrar
  helpers (`registerXRoutes(apiRoutes *gin.RouterGroup)`, 10 in `ingest/ports`) came from the
  **engine's** interprocedural interpretation plus the gin built-ins.
- OAIZ's `decorator.Apply*` wrappers are CQRS handlers, never on a route path.
- OAIZ's in-house `pubsub` package does not sit between registration and Watermill: subscribers
  call `(*message.Router).AddConsumerHandler` directly, sometimes through a local helper the
  engine follows.

So the brief's premise that repository models rescued an OAIZ registrar helper or decorator is
**not true**. That evidence actually supports the shipped split:
- the value came from engine semantics (alternative 2's legitimate core) plus shipped data
  (alternative 3);
- the repository extension point is today an unexercised safety valve;
- its one demonstrated OAIZ use case (handler-level middleware) is a case it *cannot* serve
  until the role vocabulary grows.

## 6. Who ships framework models in the industry?

| Tool | Where framework knowledge lives | How a repository extends it |
|---|---|---|
| CodeQL | Shipped in the standard library: Go `Gin`, `Echo` and others as QL classes. JS `Routing.qll` models *routing trees and middleware composition* as a framework-neutral layer, and framework adapters (`Express.qll`: `RouterDefinition.getMiddlewareStackAt`) plug into it | Models-as-data YAML extensions and model packs, for "custom frameworks or niche libraries" |
| Semgrep | Vendor-curated rules: the community registry, plus Pro rules with framework-specific analysis (Go: Gin, Gorilla, net/http, gRPC) that CE lacks | Per-repository custom rules |
| Find Security Bugs (SpotBugs) | Shipped taint configuration per framework | `findsecbugs.taint.customconfigfile` |
| Joern | `DefaultSemantics` shipped | User semantics files, same grammar |
| Pysa | Shipped `.pysa` models (for example, Django sources and sinks) | Repository `.pysa` files, plus *model generators* (for example, `RESTApiSourceGenerator` from Django URL configs) |
| golangci-lint linters (for example, gosec) | Hard-coded in each linter | Config lists |

- **Who ships the models:** the tool, every time. Repositories extend with the same format.
  Semgrep is the nearest thing to alternative 1, and even there the framework knowledge is
  vendor-curated and shared, not written per repository.
- **Where the semantics live:** CodeQL's JS split (a framework-neutral routing tree with
  middleware-ordering semantics, plus per-framework adapters) is the closest precedent for
  polint's split. It is also the direction §5.1 points: the adapters, not the core, say *when*
  middleware applies.
- **Generated models:** Pysa's generators are the precedent for the "detect and suggest" role
  given to alternative 2.

Sources:
[CodeQL: customizing library models for Go](https://codeql.github.com/docs/codeql-language-guides/customizing-library-models-for-go/),
[CodeQL supported frameworks](https://codeql.github.com/docs/codeql-overview/supported-languages-and-frameworks/),
[CodeQL JS Routing.qll](https://codeql.github.com/codeql-standard-libraries/javascript/semmle/javascript/Routing.qll/module.Routing$Routing.html),
[CodeQL Express RouterDefinition](https://codeql.github.com/codeql-standard-libraries/javascript/semmle/javascript/frameworks/Express.qll/type.Express$Express$RouterDefinition.html),
[Semgrep Go in Pro Engine](https://semgrep.dev/blog/2023/golang-in-pro-engine/),
[Semgrep Pro rules](https://docs.semgrep.dev/semgrep-code/pro-rules),
[Find Security Bugs custom signatures](https://github.com/find-sec-bugs/find-sec-bugs/wiki/Custom-signatures),
[Joern custom data-flow semantics](https://docs.joern.io/dataflow-semantics/),
[Pysa basics](https://pyre-check.org/docs/pysa-basics/), [Running Pysa](https://pyre-check.org/docs/pysa-running/).

## 7. What to do, concretely (in order; none of it is built)

1. **Truthfulness first (days).**
   - Mark a route's middleware incomplete when its registration's returned handle feeds a call no
     model covers (Watermill `.AddMiddleware`, gorilla `.Methods`, echo's middleware after the
     handler).
   - On OAIZ this flips the 9 ingest subscriptions from "complete, empty" to "incomplete".
2. **Role parameters (1–2 weeks).**
   - `use.applies = "later" | "all"`, with the built-ins set per framework: Watermill router-level
     → `all`.
   - Route handles (`subscriber`/`route` return a handle; a `use` on a handle receiver targets
     that one registration).
   - `middleware_after_handler`, `method_from_call`, method lists.
   - Then echo and gorilla built-ins, each with a fixture test.
3. **One home and format for built-ins (days).**
   - Move `route_models.json` to `crates/polint/src/go/route_models.toml` in `[[go_route]]`
     syntax, next to `flow_models.toml`, and pass the merged document to the sidecar.
   - Keep one validator, and include the digest in the cache key, as flow models already do.
4. **Model diagnostics (days).**
   - Shadowing notes.
   - A `disabled` model.
   - The unmodeled-registration guard (shape S2, restricted to unmodeled packages).
5. **Keep `Routes<'_>` and `[[go_route]]` preview** until 2–4 land.

## 8. What would change our mind (falsifiable triggers)

- **Built-ins are wrong for real users.** If **2 or more consumer repositories** ship
  `[[go_route]]` tables that *override* built-ins (rather than add in-house wrappers), revisit
  how the table is curated. Move to versioned model packs released separately from the engine,
  as CodeQL's model packs are.
- **Versions diverge.** If a modeled framework changes registration or middleware semantics
  without an import-path change, and consumers run both versions, add version predicates to the
  model format. If it happens twice in a year, split the built-ins into per-version packs.
- **The repository surface is churning.** If the `[[go_route]]` schema gains **more than 3 new
  keys in a quarter** after step 2, the TOML surface is tracking interpreter internals. Freeze it
  and expose a typed builder in the rule SDK instead.
- **Non-Go routes are needed.** If **2 or more consumers** need TS route models
  (Express/Next/TanStack, the TS leg in [13](13-deep-analysis.md) §5), the role vocabulary has to
  become language-neutral and move out of the Go sidecar into the Rust engine (the CodeQL
  `Routing.qll` shape). Revisit where roles live.
- **Inference gets good enough.** If the unmodeled-registration guard reaches **95% or better
  precision and 90% or better recall, with the role right, on 2 or more repositories** (role
  checked by hand against a sample), promote it from suggestion to generator. Its proposals
  become models, Pysa-style, and the hand-written built-ins shrink to what generation gets wrong.
- **Semantics outgrow parameters.** If a framework needs ordering or matching semantics that no
  parameter set can express (host-based routing, route precedence that changes which middleware
  applies), consider framework adapters as code inside polint. Still not repository rules.
- **Repositories adopt the mechanism, or don't.** If, 6 months after the deep providers reach a
  default profile, **no consumer has written a `[[go_route]]` table**, the repository extension
  point is unexercised. Keep it, but stop expanding its surface ahead of demand.
- **A demand trigger, now.** If OAIZ turns a "subscriptions retry" or "message handlers are
  instrumented" policy into a rule, step 2's route-handle role becomes blocking, not optional.
