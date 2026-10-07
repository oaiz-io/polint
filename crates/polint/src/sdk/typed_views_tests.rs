//! The call-graph and Go type views over a real kernel run of a typed Go module.

use crate::analysis_kernel::{AnalysisKernel, KernelInput, KernelOutput};
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::load_config;
use crate::sdk::facts::{
    CallEdgeAlgorithm, CallEdgePrecision, CallGraph, CallGraphCallee, CallGraphWalk, FactView,
    GoGenericTarget, GoTypes, RouteFunctionKind, Routes,
};
use crate::sdk::prelude::FunctionId;

const MODULE: &str = "module example.com/typed\n\ngo 1.22\n";

const SOURCE: &str = r#"package typed

import "fmt"

type DB struct{ dsn string }

type Store interface{ Save(id string) error }

type Repo struct {
	db    *DB
	Label string `json:"label"`
}

func NewRepo(db *DB, label string) *Repo { return &Repo{db: db, Label: label} }

func (r *Repo) Save(id string) error {
	requireAdmin(id)
	return nil
}

type Box[T any] struct{ value T }

func requireAdmin(id string) { fmt.Println(id) }

func Handle(store Store, id string) error {
	_ = Box[string]{value: id}
	return store.Save(id)
}

func Wire() error {
	return Handle(NewRepo(&DB{}, "x"), "1")
}
"#;

fn run() -> (tempfile::TempDir, KernelOutput) {
    run_with_module(Some(MODULE))
}

fn run_with_module(module: Option<&str>) -> (tempfile::TempDir, KernelOutput) {
    run_source(module, SOURCE)
}

fn run_source(module: Option<&str>, source: &str) -> (tempfile::TempDir, KernelOutput) {
    run_files(module, &[("typed.go", source)], &["call_graph", "go_types"])
}

fn run_files(
    module: Option<&str>,
    files: &[(&str, &str)],
    capabilities: &[&str],
) -> (tempfile::TempDir, KernelOutput) {
    let temp = tempfile::tempdir().expect("temp directory");
    if let Some(module) = module {
        std::fs::write(temp.path().join("go.mod"), module).expect("write go.mod");
    }
    for (name, source) in files {
        std::fs::write(temp.path().join(name), source).expect("write source");
    }
    let loaded = load_config(temp.path()).expect("default config loads");
    let plan = AnalysisPlan::from_capability_names_for_test(capabilities);
    let output = AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &Cache::new("", false),
        config_digest: "config",
        rule_digest: "rules",
        plan: &plan,
        parallel: false,
    })
    .expect("kernel should run");
    (temp, output)
}

fn function(output: &KernelOutput, name: &str) -> FunctionId {
    output
        .db
        .functions()
        .iter()
        .find(|function| function.name == name)
        .unwrap_or_else(|| panic!("function {name}"))
        .id
}

#[test]
fn the_call_graph_reaches_through_interface_calls_and_names_dependency_callees() {
    if !crate::go::semantic::process::go_toolchain_available_for_tests() {
        eprintln!("skipping: the Go toolchain is not on PATH");
        return;
    }
    let (_temp, output) = run();
    let graph = CallGraph::build(&output.db);
    let handle = function(&output, "Handle");
    let save = function(&output, "Repo.Save");
    let require_admin = function(&output, "requireAdmin");

    let interface_call = graph
        .callees(handle)
        .find(|edge| edge.callee == CallGraphCallee::Function(save))
        .expect("the interface call resolves to the only implementation");
    assert_eq!(
        interface_call.algorithm,
        CallEdgeAlgorithm::VariableTypeAnalysis
    );
    assert!(
        interface_call
            .precision
            .at_least(CallEdgePrecision::SetupAware)
    );

    let reach = graph.reachable(handle, CallGraphWalk::new(4));
    assert!(reach.contains(require_admin));
    assert!(
        reach
            .externals()
            .any(|label| label == "go:func:fmt.Println")
    );
    let path = reach.path_to(require_admin).expect("a path to the gate");
    assert_eq!(path.len(), 2);
    assert_eq!(path[0].caller, handle);
    assert_eq!(path[1].callee, CallGraphCallee::Function(require_admin));

    assert!(
        !graph
            .reachable(handle, CallGraphWalk::new(1))
            .contains(require_admin),
        "the depth bound holds"
    );
    assert!(
        graph
            .callers(require_admin)
            .any(|edge| edge.caller == save && edge.algorithm == CallEdgeAlgorithm::Static)
    );
    assert_eq!(
        graph
            .paths(handle, require_admin, CallGraphWalk::new(4), 8)
            .len(),
        1
    );
}

#[test]
fn go_types_answer_fields_parameters_instantiations_and_implementations() {
    if !crate::go::semantic::process::go_toolchain_available_for_tests() {
        eprintln!("skipping: the Go toolchain is not on PATH");
        return;
    }
    let (_temp, output) = run();
    let types = GoTypes::build(&output.db);

    let fields = types
        .fields_of("example.com/typed.Repo")
        .map(|field| (field.name, field.type_name, field.tag))
        .collect::<Vec<_>>();
    assert_eq!(
        fields,
        vec![
            ("db", "*example.com/typed.DB", None),
            ("Label", "string", Some("json:\"label\"")),
        ]
    );

    let new_repo = function(&output, "NewRepo");
    let parameters = types
        .parameters(new_repo)
        .map(|parameter| (parameter.index, parameter.name, parameter.type_name))
        .collect::<Vec<_>>();
    assert_eq!(
        parameters,
        vec![(0, "db", "*example.com/typed.DB"), (1, "label", "string")]
    );
    let save = function(&output, "Repo.Save");
    assert_eq!(types.receiver(save), Some("*example.com/typed.Repo"));
    let method = types.function(save).expect("the method is typed");
    assert_eq!(method.qualified, "(*example.com/typed.Repo).Save");
    assert_eq!(method.signature, "func(id string) error");
    assert_eq!(
        types
            .parameters(save)
            .map(|parameter| (parameter.index, parameter.name, parameter.type_name))
            .collect::<Vec<_>>(),
        vec![(0, "id", "string")]
    );
    assert_eq!(
        types.method_set("example.com/typed.Repo"),
        Some(&["Save".to_string()][..])
    );
    assert!(
        types
            .implements("example.com/typed.Repo")
            .any(|implementation| implementation.interface == "example.com/typed.Store")
    );

    let boxes = types
        .instantiations_of("example.com/typed.Box")
        .collect::<Vec<_>>();
    assert_eq!(boxes.len(), 1);
    assert_eq!(boxes[0].target, GoGenericTarget::Type);
    assert_eq!(boxes[0].type_arguments, ["string".to_string()]);
    assert_eq!(boxes[0].instantiated, "example.com/typed.Box[string]");

    assert!(
        types
            .implementers("example.com/typed.Store")
            .any(
                |implementation| implementation.type_name == "example.com/typed.Repo"
                    && implementation.via_pointer
            )
    );
}

#[test]
fn optional_views_are_absent_for_an_unavailable_capability() {
    let (_temp, mut output) = run();
    assert!(CallGraph::build_optional(&output.db).is_some());
    output
        .db
        .set_unavailable_capabilities(["go_types".to_string()].into());
    assert!(GoTypes::build_optional(&output.db).is_none());
    assert!(CallGraph::build_optional(&output.db).is_some());
}

#[test]
fn go_types_are_unavailable_when_no_module_root_covers_the_go_files() {
    let (_temp, output) = run_with_module(None);

    assert!(GoTypes::build_optional(&output.db).is_none());
    assert!(
        output
            .runtime_blocked_rules
            .contains("test/requested-capabilities")
    );
    assert!(output.diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == "polint/capability"
            && diagnostic.message.contains("`go_types`")
            && diagnostic
                .evidence
                .iter()
                .any(|evidence| evidence.label == "status" && evidence.value == "setup_missing")
    }));

    let graph = CallGraph::build_optional(&output.db)
        .expect("calls still resolve from names without the typed frontend");
    let edges = graph.edges().collect::<Vec<_>>();
    assert!(!edges.is_empty());
    assert!(edges.iter().all(|edge| {
        edge.precision == CallEdgePrecision::Heuristic
            && edge.algorithm == CallEdgeAlgorithm::Syntactic
    }));
}

/// Go data flow is answered from the typed frontend's flow programs, and a
/// scan of Go sources only builds no value-flow graph to fall back on. With no
/// package loaded a data-flow rule must be blocked with the setup diagnostic,
/// not run against no program and read "no flow" where the truth is "not
/// analyzed".
#[test]
fn data_flow_is_unavailable_on_a_go_only_scan_without_a_module_root() {
    let (_temp, output) = run_files(None, &[("typed.go", SOURCE)], &["dataflow"]);

    assert!(!output.db.capability_available("dataflow"));
    assert!(
        output
            .runtime_blocked_rules
            .contains("test/requested-capabilities")
    );
    assert!(output.diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == "polint/capability"
            && diagnostic.message.contains("`dataflow`")
            && diagnostic
                .evidence
                .iter()
                .any(|evidence| evidence.label == "status" && evidence.value == "setup_missing")
    }));
}

/// A scan with other languages keeps data flow available (the value-flow graph
/// answers `forbidden` for every file), so `flows` must say that no Go program
/// was loaded instead of answering an empty, complete answer.
#[cfg(feature = "lang-typescript")]
#[test]
fn flows_report_no_program_when_a_mixed_scan_loaded_no_go_package() {
    use crate::sdk::facts::{DataFlow, FlowSink, FlowSource, FlowSpec, FlowUnknown};

    let (_temp, output) = run_files(
        None,
        &[
            ("typed.go", SOURCE),
            (
                "app.ts",
                "export function handler(token: string): string {\n  return token;\n}\n",
            ),
        ],
        &["dataflow"],
    );

    assert!(output.db.capability_available("dataflow"));
    assert!(output.runtime_blocked_rules.is_empty());
    let spec = FlowSpec::new()
        .source(FlowSource::call_result("NewRepo"))
        .sink(FlowSink::call("requireAdmin"));
    let answer = DataFlow::build(&output.db).flows(&spec);
    assert!(answer.flows.is_empty());
    assert_eq!(answer.unknowns, vec![FlowUnknown::NoProgram]);
    assert!(!answer.is_complete());
}

#[test]
fn data_flow_through_exactly_resolved_calls_stays_within_its_precision_ceiling() {
    if !crate::go::semantic::process::go_toolchain_available_for_tests() {
        eprintln!("skipping: the Go toolchain is not on PATH");
        return;
    }
    use crate::sdk::prelude::{
        BarrierPattern, DataFlow, FlowQuery, PolicyPrecision, SinkPattern, SourcePattern,
    };

    let temp = tempfile::tempdir().expect("temp directory");
    std::fs::write(
        temp.path().join("go.mod"),
        "module example.com/flows\n\ngo 1.22\n",
    )
    .expect("write go.mod");
    std::fs::write(
        temp.path().join("flows.go"),
        "package flows\n\nimport \"log\"\n\nfunc handler(token string) { log.Println(token) }\n\nfunc Run(secret string) { handler(secret) }\n",
    )
    .expect("write Go source");
    let loaded = load_config(temp.path()).expect("default config loads");
    let plan = AnalysisPlan::from_capability_names_for_test(&["dataflow"]);
    let output = AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &Cache::new("", false),
        config_digest: "config",
        rule_digest: "rules",
        plan: &plan,
        parallel: false,
    })
    .expect("kernel should run");

    let internal = output
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule_id == "polint/internal")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();
    assert!(internal.is_empty(), "{}", internal.join("\n"));
    let mut query = FlowQuery::new(SourcePattern::secret_like(["token"]), SinkPattern::logger());
    query.barriers = BarrierPattern::call_any(["redact"]);
    query.minimum_precision = PolicyPrecision::Heuristic;
    assert_eq!(DataFlow::build(&output.db).forbidden(query).len(), 1);
}

#[test]
fn a_function_literal_passed_to_a_helper_is_not_reached_by_the_helpers_other_callers() {
    if !crate::go::semantic::process::go_toolchain_available_for_tests() {
        eprintln!("skipping: the Go toolchain is not on PATH");
        return;
    }
    const CALLBACKS: &str = r#"package typed

type Tx struct{}

func (t *Tx) Run(fn func() error) error { return fn() }

func requireAdmin() {}

func Guarded(t *Tx) error {
	return t.Run(func() error {
		requireAdmin()
		return nil
	})
}

func Open(t *Tx) error { return t.Run(func() error { return nil }) }
"#;
    let (_temp, output) = run_source(Some(MODULE), CALLBACKS);
    let graph = CallGraph::build(&output.db);
    let require_admin = function(&output, "requireAdmin");
    let walk = CallGraphWalk::new(8);

    assert!(
        graph
            .reachable(function(&output, "Guarded"), walk)
            .contains(require_admin),
        "a literal's calls are calls of the function that declares it"
    );
    assert!(
        !graph
            .reachable(function(&output, "Open"), walk)
            .contains(require_admin),
        "the helper's call of its callback reaches the literals, not their declaring functions"
    );
    let mut literals = graph
        .callees(function(&output, "Tx.Run"))
        .filter_map(|edge| match edge.callee {
            CallGraphCallee::External(label) => Some(label),
            _ => None,
        })
        .collect::<Vec<_>>();
    literals.sort_unstable();
    assert_eq!(
        literals,
        [
            "go:func:example.com/typed.Guarded$1",
            "go:func:example.com/typed.Open$1"
        ]
    );
}

const GIN_STUB: &str = r#"package gin

import "net/http"

type Context struct{}

type HandlerFunc func(*Context)

type IRoutes interface {
	Use(...HandlerFunc) IRoutes
	GET(string, ...HandlerFunc) IRoutes
	POST(string, ...HandlerFunc) IRoutes
}

type RouterGroup struct{ Handlers []HandlerFunc }

func (group *RouterGroup) Use(middleware ...HandlerFunc) IRoutes { return group }

func (group *RouterGroup) Group(path string, handlers ...HandlerFunc) *RouterGroup { return group }

func (group *RouterGroup) GET(path string, handlers ...HandlerFunc) IRoutes { return group }

func (group *RouterGroup) POST(path string, handlers ...HandlerFunc) IRoutes { return group }

type Engine struct{ RouterGroup }

func New() *Engine { return &Engine{} }

func (engine *Engine) Use(middleware ...HandlerFunc) IRoutes { return engine }

func (engine *Engine) ServeHTTP(w http.ResponseWriter, r *http.Request) {}
"#;

const ROUTED_APP: &str = r#"package app

import "github.com/gin-gonic/gin"

type Server struct {
	Router *gin.Engine
	Auth   gin.HandlerFunc
}

func Authenticate() gin.HandlerFunc { return func(*gin.Context) {} }

func NewServer() *Server {
	router := gin.New()
	return &Server{Router: router, Auth: Authenticate()}
}

func (s *Server) Routes() {
	s.Router.GET("/health", health)
	api := s.Router.Group("/api")
	api.Use(s.Auth)
	api.POST("/items", decorate(s.create))
}

func decorate(handler gin.HandlerFunc) gin.HandlerFunc { return handler }

func health(c *gin.Context) {}

func (s *Server) create(c *gin.Context) {}

func Exercise() {
	s := NewServer()
	s.Routes()
	s.Router.ServeHTTP(nil, nil)
}
"#;

const ROUTED_MAIN: &str = r#"package main

import "example.com/routed/app"

func main() {
	s := app.NewServer()
	s.Routes()
}
"#;

fn run_routed(models: Option<&str>) -> (tempfile::TempDir, KernelOutput) {
    let temp = tempfile::tempdir().expect("temp directory");
    let files = [
        (
            "go.mod",
            "module example.com/routed\n\ngo 1.22\n\nrequire github.com/gin-gonic/gin v1.0.0\n\nreplace github.com/gin-gonic/gin => ./stubs/gin\n",
        ),
        (
            "stubs/gin/go.mod",
            "module github.com/gin-gonic/gin\n\ngo 1.22\n",
        ),
        ("stubs/gin/gin.go", GIN_STUB),
        ("app/app.go", ROUTED_APP),
        ("cmd/server/main.go", ROUTED_MAIN),
    ];
    for (path, contents) in files
        .into_iter()
        .chain(models.map(|models| (".polint/models/routes.toml", models)))
    {
        let path = temp.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).expect("create directory");
        std::fs::write(path, contents).expect("write file");
    }
    let loaded = load_config(temp.path()).expect("default config loads");
    let plan = AnalysisPlan::from_capability_names_for_test(&["routes"]);
    let output = AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &Cache::new("", false),
        config_digest: "config",
        rule_digest: "rules",
        plan: &plan,
        parallel: false,
    })
    .expect("kernel should run");
    (temp, output)
}

#[test]
fn routes_name_their_handlers_middleware_and_serve_calls_by_function() {
    if !crate::go::semantic::process::go_toolchain_available_for_tests() {
        eprintln!("skipping: the Go toolchain is not on PATH");
        return;
    }
    let (_temp, output) = run_routed(Some(
        r#"
[[go_route]]
framework = "local"
role = "passthrough"
function = "example.com/routed/app.decorate"
argument = 0
"#,
    ));
    let routes = Routes::build(&output.db);
    assert!(routes.complete());
    let mut table = routes
        .http()
        .map(|route| {
            (
                route.method.to_string(),
                route.path.to_string(),
                route.path_complete && route.middleware_complete,
                route
                    .handlers()
                    .map(|handler| handler.function)
                    .collect::<Vec<_>>(),
                route
                    .middleware()
                    .map(|middleware| (middleware.kind, middleware.name.to_string()))
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    table.sort_by(|left, right| left.1.cmp(&right.1));
    assert_eq!(
        table,
        [
            (
                "POST".to_string(),
                "/api/items".to_string(),
                true,
                vec![Some(function(&output, "Server.create"))],
                vec![(
                    RouteFunctionKind::Factory,
                    "example.com/routed/app.Authenticate".to_string()
                )],
            ),
            (
                "GET".to_string(),
                "/health".to_string(),
                true,
                vec![Some(function(&output, "health"))],
                Vec::new(),
            ),
        ]
    );
    let auth = routes
        .http()
        .find(|route| route.path == "/api/items")
        .and_then(|route| route.middleware().next())
        .expect("the group's middleware");
    assert_eq!(auth.field, Some("example.com/routed/app.Server.Auth"));
    assert_eq!(
        routes
            .http()
            .find(|route| route.path == "/api/items")
            .map(|route| route.registered_path),
        Some("/items"),
        "the registration's own path, without the group's prefix"
    );
    assert_eq!(auth.function, Some(function(&output, "Authenticate")));
    assert_eq!(
        routes
            .handled_by(function(&output, "Server.create"))
            .map(|route| route.path)
            .collect::<Vec<_>>(),
        ["/api/items"]
    );
    let mut served = routes
        .served_from(function(&output, "Exercise"))
        .map(|route| route.path)
        .collect::<Vec<_>>();
    served.sort_unstable();
    assert_eq!(served, ["/api/items", "/health"]);
    assert_eq!(
        routes
            .http()
            .find(|route| route.path == "/health")
            .and_then(|route| route.registered_in),
        Some(function(&output, "Server.Routes"))
    );
}

#[test]
fn without_a_passthrough_model_a_wrapped_handler_is_named_by_its_wrapper() {
    if !crate::go::semantic::process::go_toolchain_available_for_tests() {
        eprintln!("skipping: the Go toolchain is not on PATH");
        return;
    }
    let (_temp, output) = run_routed(None);
    let routes = Routes::build(&output.db);
    let handler = routes
        .http()
        .find(|route| route.path == "/api/items")
        .and_then(|route| route.handlers().next())
        .expect("the wrapped route");
    assert_eq!(handler.kind, RouteFunctionKind::Factory);
    assert_eq!(handler.function, Some(function(&output, "decorate")));
}
