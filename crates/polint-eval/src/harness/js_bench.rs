//! `js_bench`: the fact dump behind the JS/TS analysis benchmark in
//! `tests/js-analysis-bench/`.
//!
//! The benchmark scores polint's JS/TS call facts against hand-labelled
//! construct fixtures and against call edges recorded while real packages ran.
//! Scoring lives in the benchmark's report script. This module only reads one
//! run's facts and writes them as one normalized JSON document per analysed
//! tree: no machine paths, no run-local ids, every list sorted. Functions, call
//! sites and symbols are named by relative path and UTF-8 byte span, which is
//! what the labels and the runtime recorder produce too.
//!
//! The read is strictly read-only and changes no analysis behaviour. Base call
//! edges are exactly the rows the public `CallGraph` view serves
//! ([`crate::core::view_index::CallGraphIndex`]). Refined call edges have no
//! public view; they are read from the crate-private store and dumped as a
//! separate list so the report can show them in their own column.
//!
//! The entry has to be a test entry: this harness is compiled into `polint`
//! only under `cfg(test)`, and the call stores are crate-private. It is run as
//!
//! ```sh
//! POLINT_JS_BENCH_MANIFEST=<file> POLINT_JS_BENCH_CAPS=syntax,call_graph \
//! cargo test -p polint --lib --all-features --locked --release \
//!   eval::js_bench::tests::dump_js_bench_facts -- --exact --ignored --nocapture
//! ```
//!
//! Each non-empty manifest line is `<tree>\t<output.json>`. Trees should live
//! outside any git checkout, so that no parent `.gitignore` hides their files,
//! and the run writes no cache.
//!
//! With `POLINT_JS_BENCH_CENSUS=1` the dump also carries a syntax census: every
//! function-like node, class, call and `new` expression Oxc finds in each
//! analysed file, with the syntactic context it sits in. The report uses it to
//! say which kind of missing extraction an unmatched observed edge falls into.
//! The census parses each file again on its own; it reads no analysis store and
//! feeds nothing back into one.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast::ast::{Expression, FunctionType, MethodDefinitionKind};
use oxc_semantic::{AstNodes, NodeId, SemanticBuilder};
use oxc_span::GetSpan;
use serde_json::{Value, json};

use crate::analysis_api::SourceFile;
use crate::analysis_neutral::ids::CallSiteId;
use crate::core::{AnalysisDb, TS_JS_MODULE_FUNCTION_NAME};
use crate::internal_core::{FileId, FunctionId, Language, Span, SymbolId};

const MANIFEST_ENV: &str = "POLINT_JS_BENCH_MANIFEST";
const CAPS_ENV: &str = "POLINT_JS_BENCH_CAPS";
const CENSUS_ENV: &str = "POLINT_JS_BENCH_CENSUS";
const SCHEMA: &str = "polint-js-bench-facts-3";

/// Analyses `tree` with the named capabilities and returns its normalized dump.
fn dump_tree(tree: &Path, capabilities: &[&str], with_census: bool) -> anyhow::Result<Value> {
    let loaded = crate::cli::load_config_for_check(tree, &[])?;
    let config_digest = crate::cache::keys::config_hash(&loaded);
    let rule_digest = crate::cache::keys::rule_hash(&[], None, &BTreeMap::new());
    let cache = crate::cache::Cache::default_for_repo(tree, false);
    let plan = crate::analysis_plan::AnalysisPlan::from_capability_names(capabilities);
    let output =
        crate::analysis_kernel::AnalysisKernel::run(crate::analysis_kernel::KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: &config_digest,
            rule_digest: &rule_digest,
            plan: &plan,
            parallel: true,
        })?;
    let mut document = Dump::new(&output.db).document(capabilities);
    // Whether the TypeScript type tier ran is part of the result: a typed run
    // whose sidecar could not start must read as that, not as a typed answer.
    document["ts_types"] = json!({
        "callsites": output.db.ts_type_callsites().len(),
        "callees": output.db.ts_type_callees().len(),
        "diagnostics": sorted(
            output
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.rule_id == "polint/ts-types")
                .map(|diagnostic| json!(diagnostic.message)),
        ),
    });
    if with_census {
        document["syntax_census"] = syntax_census(&output.db);
    }
    Ok(document)
}

/// Per analysed JS/TS file: the function-like nodes and call expressions Oxc
/// parses, each with its byte span and syntactic context.
fn syntax_census(db: &AnalysisDb) -> Value {
    let files: BTreeMap<&str, Value> = db
        .files()
        .iter()
        .filter(|file| {
            matches!(
                file.language,
                Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx
            )
        })
        .map(|file| (file.relative_path.as_str(), census_file(file)))
        .collect();
    json!(files)
}

fn census_file(file: &SourceFile) -> Value {
    let allocator = Allocator::default();
    let parsed = crate::ts::parse::parse_ts_file(&allocator, file);
    if parsed.is_catastrophic() {
        return json!({ "fully_parsed": false, "functions": [], "calls": [] });
    }
    let semantic = SemanticBuilder::new().build(parsed.program()).semantic;
    let nodes = semantic.nodes();
    let source = file.source.as_ref();
    let mut functions = Vec::new();
    let mut calls = Vec::new();
    for node in nodes.iter() {
        let id = node.id();
        let kind = node.kind();
        let function_kind = match kind {
            // A method's body is its `MethodDefinition` row.
            AstKind::Function(_)
                if matches!(nodes.parent_kind(id), AstKind::MethodDefinition(_)) =>
            {
                None
            }
            AstKind::Function(function) => Some(match function.r#type {
                FunctionType::FunctionDeclaration | FunctionType::TSDeclareFunction => {
                    "declaration"
                }
                FunctionType::FunctionExpression | FunctionType::TSEmptyBodyFunctionExpression => {
                    "expression"
                }
            }),
            AstKind::ArrowFunctionExpression(_) => Some("arrow"),
            AstKind::MethodDefinition(method) => Some(match method.kind {
                MethodDefinitionKind::Constructor => "constructor",
                MethodDefinitionKind::Get | MethodDefinitionKind::Set => "accessor",
                MethodDefinitionKind::Method => "method",
            }),
            AstKind::StaticBlock(_) => Some("static_block"),
            AstKind::Class(_) => Some("class"),
            _ => None,
        };
        if let Some(function_kind) = function_kind {
            functions.push(census_row(nodes, id, source, function_kind, None));
            continue;
        }
        match kind {
            AstKind::CallExpression(call) => calls.push(census_row(
                nodes,
                id,
                source,
                "call",
                Some(has_this_receiver(&call.callee)),
            )),
            AstKind::NewExpression(_) => calls.push(census_row(nodes, id, source, "new", None)),
            _ => {}
        }
    }
    json!({
        "fully_parsed": parsed.fully_parsed,
        "functions": sorted(functions.into_iter()),
        "calls": sorted(calls.into_iter()),
    })
}

fn census_row(
    nodes: &AstNodes<'_>,
    id: NodeId,
    source: &str,
    kind: &str,
    this_receiver: Option<bool>,
) -> Value {
    let span = nodes.kind(id).span();
    let (context, class) = syntactic_context(nodes, id, source);
    let mut row = json!({
        "start": span.start,
        "end": span.end,
        "kind": kind,
        "context": context,
        "class": class.map(|class| [class.start, class.end]),
    });
    if let Some(this_receiver) = this_receiver {
        row["this_receiver"] = Value::Bool(this_receiver);
    }
    row
}

/// The context tags between a node and its nearest enclosing function-like
/// node or class, innermost first, plus the span of the nearest enclosing
/// class at any depth.
fn syntactic_context(
    nodes: &AstNodes<'_>,
    id: NodeId,
    source: &str,
) -> (Vec<&'static str>, Option<oxc_span::Span>) {
    let mut tags = Vec::new();
    let mut class = None;
    let mut bounded = false;
    let mut child = nodes.kind(id).span();
    for ancestor in nodes.ancestor_ids(id) {
        let kind = nodes.kind(ancestor);
        if let AstKind::Class(found) = kind {
            class.get_or_insert(found.span);
        }
        if !bounded {
            let tag = match kind {
                AstKind::Function(_)
                | AstKind::ArrowFunctionExpression(_)
                | AstKind::StaticBlock(_)
                | AstKind::Class(_) => {
                    bounded = true;
                    None
                }
                AstKind::UnaryExpression(_) => Some("unary"),
                AstKind::TryStatement(statement) => {
                    if statement.block.span == child {
                        Some("try")
                    } else if statement
                        .finalizer
                        .as_ref()
                        .is_some_and(|block| block.span == child)
                    {
                        Some("finally")
                    } else {
                        None
                    }
                }
                AstKind::CatchClause(_) => Some("catch"),
                AstKind::IfStatement(_) => Some("if"),
                AstKind::ExportNamedDeclaration(_) | AstKind::ExportDefaultDeclaration(_) => {
                    Some("export")
                }
                AstKind::CallExpression(call) => Some(if call.callee.span() == child {
                    "callee"
                } else {
                    "argument"
                }),
                AstKind::NewExpression(call) => Some(if call.callee.span() == child {
                    "new_callee"
                } else {
                    "argument"
                }),
                AstKind::ObjectProperty(_) => Some("property"),
                AstKind::AssignmentExpression(assignment) => {
                    let left = assignment.left.span();
                    let target = source
                        .get(left.start as usize..left.end as usize)
                        .unwrap_or_default();
                    Some(if target.contains("prototype") {
                        "prototype_assignment"
                    } else {
                        "assignment"
                    })
                }
                _ => None,
            };
            tags.extend(tag);
        }
        if bounded && class.is_some() {
            break;
        }
        child = kind.span();
    }
    (tags, class)
}

fn has_this_receiver(callee: &Expression<'_>) -> bool {
    match callee {
        Expression::StaticMemberExpression(member) => {
            matches!(member.object, Expression::ThisExpression(_))
        }
        Expression::ComputedMemberExpression(member) => {
            matches!(member.object, Expression::ThisExpression(_))
        }
        Expression::PrivateFieldExpression(member) => {
            matches!(member.object, Expression::ThisExpression(_))
        }
        _ => false,
    }
}

/// Lookup tables that turn run-local ids into path-and-span keys.
struct Dump<'a> {
    db: &'a AnalysisDb,
    functions: HashMap<FunctionId, String>,
    sites: HashMap<CallSiteId, usize>,
}

impl<'a> Dump<'a> {
    fn new(db: &'a AnalysisDb) -> Self {
        let mut dump = Self {
            db,
            functions: HashMap::new(),
            sites: HashMap::new(),
        };
        dump.functions = db
            .functions()
            .iter()
            .map(|function| {
                (
                    function.id,
                    dump.named_span_key(&function.span, &function.name),
                )
            })
            .collect();
        dump.sites = db
            .call_sites()
            .iter()
            .enumerate()
            .map(|(index, site)| (site.id, index))
            .collect();
        dump
    }

    fn path(&self, file: FileId) -> &'a str {
        self.db
            .file(file)
            .map_or("<unknown-file>", |file| file.relative_path.as_str())
    }

    fn span_key(&self, span: &Span) -> String {
        format!(
            "{}@{}..{}",
            self.path(span.file),
            span.start_byte,
            span.end_byte
        )
    }

    fn named_span_key(&self, span: &Span, name: &str) -> String {
        format!("{}#{name}", self.span_key(span))
    }

    fn function_key(&self, id: FunctionId) -> Value {
        self.functions
            .get(&id)
            .map_or(Value::Null, |key| Value::String(key.clone()))
    }

    fn site_key(&self, id: CallSiteId) -> Value {
        self.sites.get(&id).map_or(Value::Null, |&index| {
            Value::String(self.span_key(&self.db.call_sites()[index].span))
        })
    }

    fn symbol(&self, id: SymbolId) -> Value {
        let Some(symbol) = self.db.symbol_by_id(id) else {
            return Value::Null;
        };
        let key = match &symbol.primary_span {
            Some(span) => self.named_span_key(span, &symbol.name),
            None => format!("<no-span>#{}", symbol.qualified_name),
        };
        json!({
            "key": key,
            "name": symbol.name,
            "kind": debug(&symbol.kind),
        })
    }

    fn document(&self, capabilities: &[&str]) -> Value {
        let db = self.db;
        let available: BTreeMap<&str, bool> = capabilities
            .iter()
            .map(|&name| (name, db.capability_available(name)))
            .collect();

        let files = sorted(db.files().iter().map(|file| {
            json!({
                "path": file.relative_path,
                "bytes": file.source.len(),
                "language": debug(&file.language),
            })
        }));

        let functions = sorted(db.functions().iter().map(|function| {
            json!({
                "key": self.functions.get(&function.id),
                "file": self.path(function.file),
                "name": function.name,
                "start": function.span.start_byte,
                "end": function.span.end_byte,
                "line": function.span.start_line,
                "module": function.name == TS_JS_MODULE_FUNCTION_NAME,
                "exported": function.is_exported,
            })
        }));

        let call_sites = sorted(db.call_sites().iter().map(|site| {
            json!({
                "key": self.span_key(&site.span),
                "file": self.path(site.file),
                "start": site.span.start_byte,
                "end": site.span.end_byte,
                "line": site.span.start_line,
                "caller": self.function_key(site.caller),
                "kind": debug(&site.kind),
                "status": debug(&site.status),
                "precision": debug(&site.precision),
            })
        }));

        let index = db.call_graph_index();
        let base_edges = sorted((0..index.edges.len()).map(|edge| {
            let edge = index.edge(db, u32::try_from(edge).expect("edge index fits u32"));
            json!({
                "site": self.span_key(edge.span),
                "caller": self.function_key(edge.caller),
                "function": edge.function.map_or(Value::Null, |id| self.function_key(id)),
                "symbol": edge.symbol.map_or(Value::Null, |id| self.symbol(id)),
                "label": edge.label,
                "precision": debug(&edge.precision),
                "claim_tier": edge.precision.tier().as_str(),
                "algorithm": debug(&edge.algorithm),
            })
        }));

        let call_targets = sorted(db.call_targets().iter().map(|target| {
            json!({
                "site": self.site_key(target.site),
                "status": debug(&target.status),
                "reason": target.reason.as_ref().map(debug),
                "has_target": target.target_function.is_some()
                    || target.target_symbol.is_some()
                    || target.synthetic_target.is_some(),
            })
        }));

        let refined_edges = sorted(db.refined_call_edges().iter().map(|edge| {
            json!({
                "site": self.site_key(edge.site),
                "caller": self.function_key(edge.caller),
                "function": edge.target_function.map_or(Value::Null, |id| self.function_key(id)),
                "symbol": edge.target_symbol.map_or(Value::Null, |id| self.symbol(id)),
                "label": edge.synthetic_target,
                "has_base": edge.base_target.is_some(),
                "tier": debug(&edge.tier),
                "status": debug(&edge.status),
                "reason": edge.reason.as_ref().map(debug),
                "precision": debug(&edge.precision),
                "claim_tier": edge.precision.tier().as_str(),
                "dispatch": dispatch_evidence(&edge.evidence),
                "validation": debug(&edge.validation),
                "confidence": debug(&edge.confidence),
                "algorithm": debug(&edge.algorithm),
                "edge_kind": debug(&edge.edge_kind),
            })
        }));

        let unresolved_calls = sorted(db.unresolved_calls().iter().map(|call| {
            json!({
                "site": self.site_key(call.site),
                "caller": self.function_key(call.caller),
                "status": debug(&call.status),
                "reason": debug(&call.reason),
            })
        }));

        let exported_symbols = sorted(db.symbols().iter().filter(|s| s.is_exported).map(|s| {
            json!({
                "file": s.file.map(|file| self.path(file)),
                "name": s.name,
                "kind": debug(&s.kind),
            })
        }));

        let exports = sorted(db.exports().iter().map(|export| {
            json!({
                "file": export.file.map(|file| self.path(file)),
                "name": export.export_name,
                "kind": debug(&export.kind),
                "status": debug(&export.status),
                "symbol": export.symbol.map_or(Value::Null, |id| self.symbol(id)),
            })
        }));

        let mut references: BTreeMap<&str, BTreeMap<String, u64>> = BTreeMap::new();
        for reference in db.references() {
            let file = reference.file.map_or("<no-file>", |file| self.path(file));
            *references
                .entry(file)
                .or_default()
                .entry(debug(&reference.status))
                .or_default() += 1;
        }

        // A `BTreeSet`, so the reasons are already in a fixed order.
        let budget_reasons: Vec<&str> = db
            .solver_budget_reasons()
            .iter()
            .map(String::as_str)
            .collect();

        json!({
            "schema": SCHEMA,
            "capabilities": capabilities,
            "capability_available": available,
            "solver_budget": {
                "status": debug(&db.solver_budget_status()),
                "reasons": budget_reasons,
            },
            "files": files,
            "functions": functions,
            "call_sites": call_sites,
            "base_edges": base_edges,
            "call_targets": call_targets,
            "refined_edges": refined_edges,
            "unresolved_calls": unresolved_calls,
            "exported_symbols": exported_symbols,
            "exports": exports,
            "references_by_file": references,
        })
    }
}

/// The dispatch kind a type-directed edge records in its evidence, such as
/// `implementation` or `union_member`; `None` for edges that record none.
fn dispatch_evidence(evidence: &[String]) -> Option<&str> {
    evidence
        .iter()
        .find_map(|entry| entry.strip_prefix("dispatch="))
}

fn debug<T: std::fmt::Debug>(value: &T) -> String {
    format!("{value:?}")
}

/// Sorts rows by their serialized text, which makes the order independent of
/// store order and worker scheduling.
fn sorted(rows: impl Iterator<Item = Value>) -> Vec<Value> {
    let mut keyed: Vec<(String, Value)> = rows.map(|row| (row.to_string(), row)).collect();
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    keyed.into_iter().map(|(_, row)| row).collect()
}

fn manifest_entries(manifest: &str) -> Vec<(PathBuf, PathBuf)> {
    manifest
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (tree, out) = line
                .split_once('\t')
                .unwrap_or_else(|| panic!("manifest line `{line}` is not `<tree>\\t<output>`"));
            (PathBuf::from(tree), PathBuf::from(out))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The benchmark's fact dump. Ignored, so a normal `cargo test` never runs
    /// it; `tests/js-analysis-bench/bench.py` invokes it by its exact filter path.
    #[test]
    #[ignore = "JS/TS benchmark dump: run explicitly with POLINT_JS_BENCH_MANIFEST set"]
    fn dump_js_bench_facts() {
        // `RUST_LOG=polint::kernel::stage=info` reports each provider stage's
        // wall time and memory on stderr; without it only errors are logged.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .try_init();
        let manifest = std::env::var_os(MANIFEST_ENV)
            .unwrap_or_else(|| panic!("{MANIFEST_ENV} names the manifest; it is required"));
        let capabilities = std::env::var(CAPS_ENV)
            .unwrap_or_else(|_| panic!("{CAPS_ENV} lists the capabilities; it is required"));
        let capabilities: Vec<&str> = capabilities
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect();
        let with_census = std::env::var_os(CENSUS_ENV).is_some_and(|value| value == "1");
        let manifest = std::fs::read_to_string(&manifest).expect("read manifest");
        for (tree, out) in manifest_entries(&manifest) {
            let document = dump_tree(&tree, &capabilities, with_census)
                .unwrap_or_else(|error| panic!("analyse {}: {error:#}", tree.display()));
            let mut text = serde_json::to_string_pretty(&document).expect("serialize dump");
            text.push('\n');
            std::fs::write(&out, text)
                .unwrap_or_else(|error| panic!("write {}: {error}", out.display()));
        }
    }

    #[test]
    fn manifest_lines_split_on_the_first_tab_and_skip_blank_lines() {
        let entries = manifest_entries("/a\t/a.json\n\n/b c\t/b.json\n");
        assert_eq!(
            entries,
            vec![
                (PathBuf::from("/a"), PathBuf::from("/a.json")),
                (PathBuf::from("/b c"), PathBuf::from("/b.json")),
            ]
        );
    }

    #[test]
    fn census_tags_the_syntactic_context_of_functions_and_calls() {
        let source = "!function () { g(); }();\n\
            try { function inTry() {} } catch (e) { h(); }\n\
            export const made = make();\n\
            A.prototype.run = function () {};\n\
            class B extends A { constructor() { super(); this.run(); } }\n";
        let file = SourceFile::new(
            FileId::from_raw(0),
            PathBuf::from("census.js"),
            "census.js".to_string(),
            Language::JavaScript,
            source.into(),
            "test-hash".to_string(),
        );
        let census = census_file(&file);
        let at = |rows: &str, text: &str| {
            let start = source.find(text).expect("text in source");
            census[rows]
                .as_array()
                .expect("rows")
                .iter()
                .find(|row| row["start"] == start)
                .unwrap_or_else(|| panic!("no {rows} row at `{text}`"))
                .clone()
        };
        let context = |row: &Value| row["context"].clone();

        assert_eq!(
            context(&at("functions", "function () { g")),
            json!(["callee", "unary"])
        );
        assert_eq!(context(&at("functions", "function inTry")), json!(["try"]));
        assert_eq!(context(&at("calls", "h()")), json!(["catch"]));
        assert_eq!(context(&at("calls", "make()")), json!(["export"]));
        assert_eq!(
            context(&at("functions", "function () {}")),
            json!(["prototype_assignment"])
        );
        let constructor = at("functions", "constructor()");
        assert_eq!(constructor["kind"], "constructor");
        let class_start = source.find("class B").expect("class");
        assert_eq!(constructor["class"][0], class_start);
        assert_eq!(at("calls", "this.run()")["this_receiver"], true);
        assert_eq!(at("calls", "g()")["this_receiver"], false);
    }

    #[test]
    fn sorted_rows_do_not_depend_on_input_order() {
        let forward = sorted([json!({"a": 1}), json!({"a": 2})].into_iter());
        let backward = sorted([json!({"a": 2}), json!({"a": 1})].into_iter());
        assert_eq!(forward, backward);
    }
}
