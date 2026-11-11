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

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::analysis_neutral::ids::CallSiteId;
use crate::core::{AnalysisDb, TS_JS_MODULE_FUNCTION_NAME};
use crate::internal_core::{FileId, FunctionId, Span, SymbolId};

const MANIFEST_ENV: &str = "POLINT_JS_BENCH_MANIFEST";
const CAPS_ENV: &str = "POLINT_JS_BENCH_CAPS";
const SCHEMA: &str = "polint-js-bench-facts-1";

/// Analyses `tree` with the named capabilities and returns its normalized dump.
fn dump_tree(tree: &Path, capabilities: &[&str]) -> anyhow::Result<Value> {
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
    Ok(Dump::new(&output.db).document(capabilities))
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
                "precision": debug(&edge.precision),
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

        json!({
            "schema": SCHEMA,
            "capabilities": capabilities,
            "capability_available": available,
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
        let manifest = std::env::var_os(MANIFEST_ENV)
            .unwrap_or_else(|| panic!("{MANIFEST_ENV} names the manifest; it is required"));
        let capabilities = std::env::var(CAPS_ENV)
            .unwrap_or_else(|_| panic!("{CAPS_ENV} lists the capabilities; it is required"));
        let capabilities: Vec<&str> = capabilities
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect();
        let manifest = std::fs::read_to_string(&manifest).expect("read manifest");
        for (tree, out) in manifest_entries(&manifest) {
            let document = dump_tree(&tree, &capabilities)
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
    fn sorted_rows_do_not_depend_on_input_order() {
        let forward = sorted([json!({"a": 1}), json!({"a": 2})].into_iter());
        let backward = sorted([json!({"a": 2}), json!({"a": 1})].into_iter());
        assert_eq!(forward, backward);
    }
}
