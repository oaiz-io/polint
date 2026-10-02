//! The call-graph and Go type views over a real kernel run of a typed Go module.

use crate::analysis_kernel::{AnalysisKernel, KernelInput, KernelOutput};
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::load_config;
use crate::sdk::facts::{
    CallEdgeAlgorithm, CallEdgePrecision, CallGraph, CallGraphCallee, CallGraphWalk, FactView,
    GoGenericTarget, GoTypes,
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
    let temp = tempfile::tempdir().expect("temp directory");
    if let Some(module) = module {
        std::fs::write(temp.path().join("go.mod"), module).expect("write go.mod");
    }
    std::fs::write(temp.path().join("typed.go"), SOURCE).expect("write Go source");
    let loaded = load_config(temp.path()).expect("default config loads");
    let plan = AnalysisPlan::from_capability_names_for_test(&["call_graph", "go_types"]);
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

#[test]
fn data_flow_through_exactly_resolved_calls_stays_within_its_precision_ceiling() {
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
