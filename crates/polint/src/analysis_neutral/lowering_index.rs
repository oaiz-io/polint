//! One pass over the whole-program fact tables, so a lowerer does not scan them
//! once per function, per body and per closure literal.
//!
//! Every MIR lowerer resolves the same four things while it walks a file: the
//! `FunctionFact` a syntactic function belongs to, the functions declared in a
//! file, the package a file is in, and its module-graph node. Each was a
//! `db.functions().iter().find(...)`-shaped scan of a program-sized table nested
//! inside a loop over another program-sized collection (research doc section 3.2,
//! the `matching_function` and `push_body` rows), which is why `polint.semantic_mir`
//! grew 5.8 times when the scope grew 1.8 times.
//!
//! The index is built once per lowering run and is lookup-only: every bucket
//! holds the rows in the fact table's own order, and every caller applies the
//! predicate it always applied to that bucket. A first-match rule over the scan
//! is therefore a first-match rule over the bucket, and no output order follows
//! hash iteration.

use std::collections::HashMap;

use crate::analysis_api::FunctionFact;
use crate::analysis_neutral::AnalysisHost;
use crate::internal_core::{FileId, Language, ModuleNodeId, PackageId};

/// Whole-program lookups a MIR lowerer needs, keyed rather than scanned.
pub(crate) struct LoweringIndex<'db> {
    functions_by_file: HashMap<(FileId, Language), FileFunctions<'db>>,
    package_by_file: HashMap<(FileId, Language), PackageId>,
    module_node_by_file: HashMap<(FileId, Language), ModuleNodeId>,
}

/// One file's function facts, whole and bucketed by name, both in table order.
#[derive(Default)]
struct FileFunctions<'db> {
    all: Vec<&'db FunctionFact>,
    by_name: HashMap<&'db str, Vec<&'db FunctionFact>>,
}

impl<'db> LoweringIndex<'db> {
    /// Build every bucket in fact-table order.
    ///
    /// `Language` is a key everywhere it was a predicate: both lowerers test it
    /// beside the file, and keeping it in the key makes the lookup equivalent to
    /// the scan even on a file that ever carries facts of two languages.
    /// `or_insert` keeps the first row per key, which is what the `find(...)`
    /// scans in `push_body` returned.
    pub(crate) fn build(db: &'db impl AnalysisHost) -> Self {
        let mut functions_by_file: HashMap<(FileId, Language), FileFunctions<'db>> = HashMap::new();
        for function in db.functions() {
            let file = functions_by_file
                .entry((function.file, function.language))
                .or_default();
            file.all.push(function);
            file.by_name
                .entry(function.name.as_str())
                .or_default()
                .push(function);
        }

        let mut package_by_file = HashMap::new();
        for package in db.packages() {
            package_by_file
                .entry((package.file, package.language))
                .or_insert(package.id);
        }

        let mut module_node_by_file = HashMap::new();
        for module in db.module_nodes() {
            let (Some(file), Some(language)) = (module.file, module.language) else {
                continue;
            };
            module_node_by_file
                .entry((file, language))
                .or_insert(module.id);
        }

        Self {
            functions_by_file,
            package_by_file,
            module_node_by_file,
        }
    }

    /// Functions declared in a file with this name, in fact-table order.
    pub(crate) fn functions_named<'index>(
        &'index self,
        file: FileId,
        language: Language,
        name: &str,
    ) -> &'index [&'db FunctionFact] {
        self.functions_by_file
            .get(&(file, language))
            .and_then(|functions| functions.by_name.get(name))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Every function declared in a file, in fact-table order.
    pub(crate) fn functions_in_file<'index>(
        &'index self,
        file: FileId,
        language: Language,
    ) -> &'index [&'db FunctionFact] {
        self.functions_by_file
            .get(&(file, language))
            .map(|functions| functions.all.as_slice())
            .unwrap_or_default()
    }

    /// The first package recorded for a file, as the scan in `push_body` read it.
    pub(crate) fn package_for_file(&self, file: FileId, language: Language) -> Option<PackageId> {
        self.package_by_file.get(&(file, language)).copied()
    }

    /// The first module-graph node recorded for a file, likewise.
    pub(crate) fn module_node_for_file(
        &self,
        file: FileId,
        language: Language,
    ) -> Option<ModuleNodeId> {
        self.module_node_by_file.get(&(file, language)).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::{FunctionFact, ModuleNode, ModuleNodeKind, PackageFact};
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::internal_core::{FunctionId, Span};
    use std::path::PathBuf;

    fn span(file: FileId, start: u32, end: u32) -> Span {
        Span::new(file, start, end, 1, 1, 1, 2)
    }

    fn function(
        file: FileId,
        name: &str,
        start: u32,
        end: u32,
        language: Language,
    ) -> FunctionFact {
        FunctionFact {
            id: FunctionId::from_raw(0),
            file,
            name: name.to_string(),
            span: span(file, start, end),
            language,
            is_test: false,
            is_exported: false,
            cyclomatic_complexity: 1,
            calls: Vec::new(),
        }
    }

    /// Two functions with the same name in one file, at different spans, plus a
    /// same-named function in another file and a same-named TypeScript row in the
    /// same file: the bucket a lookup must not widen.
    fn two_candidate_db() -> (LocalAnalysisDb, FileId, FileId) {
        let mut db = LocalAnalysisDb::new();
        let first = db.add_file(
            PathBuf::from("a.go"),
            "a.go".to_string(),
            "package a\n".to_string(),
        );
        let second = db.add_file(
            PathBuf::from("b.go"),
            "b.go".to_string(),
            "package b\n".to_string(),
        );
        db.push_function(function(first, "run", 10, 40, Language::Go));
        db.push_function(function(first, "run", 60, 90, Language::Go));
        db.push_function(function(first, "run", 60, 90, Language::TypeScript));
        db.push_function(function(second, "run", 10, 40, Language::Go));
        (db, first, second)
    }

    #[test]
    fn functions_named_keeps_the_table_order_inside_one_file_and_language() {
        let (db, first, second) = two_candidate_db();
        let index = LoweringIndex::build(&db);

        assert_eq!(
            index
                .functions_named(first, Language::Go, "run")
                .iter()
                .map(|function| (function.span.start_byte, function.span.end_byte))
                .collect::<Vec<_>>(),
            vec![(10, 40), (60, 90)],
            "both same-named candidates are in the bucket, in push order"
        );
        assert_eq!(
            index
                .functions_named(first, Language::TypeScript, "run")
                .len(),
            1,
            "the language is part of the key, so the TypeScript row is its own bucket"
        );
        assert_eq!(index.functions_named(second, Language::Go, "run").len(), 1);
        assert!(
            index
                .functions_named(first, Language::Go, "absent")
                .is_empty()
        );
        assert!(
            index
                .functions_named(FileId::from_raw(99), Language::Go, "run")
                .is_empty()
        );
    }

    #[test]
    fn functions_in_file_holds_every_language_row_separately() {
        let (db, first, _second) = two_candidate_db();
        let index = LoweringIndex::build(&db);

        assert_eq!(index.functions_in_file(first, Language::Go).len(), 2);
        assert_eq!(
            index.functions_in_file(first, Language::TypeScript).len(),
            1
        );
        assert!(
            index
                .functions_in_file(first, Language::JavaScript)
                .is_empty()
        );
    }

    /// `push_body` read the first package and the first module node the scan met.
    /// `or_insert` has to reproduce "first", not "last".
    #[test]
    fn package_and_module_lookups_keep_the_first_row_per_file() {
        let mut db = LocalAnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("a.go"),
            "a.go".to_string(),
            "package a\n".to_string(),
        );
        let first = db.push_package(PackageFact {
            id: PackageId::from_raw(0),
            file,
            name: "a".to_string(),
            span: span(file, 0, 9),
            language: Language::Go,
        });
        let second = db.push_package(PackageFact {
            id: PackageId::from_raw(0),
            file,
            name: "a_shadow".to_string(),
            span: span(file, 0, 9),
            language: Language::Go,
        });
        assert_ne!(first, second);
        db.replace_module_graph_facts(
            Vec::new(),
            vec![
                ModuleNode::new(
                    ModuleNodeId::from_raw(0),
                    ModuleNodeKind::File,
                    "a.go".to_string(),
                    Some(file),
                    None,
                    Some(Language::Go),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(0),
                    ModuleNodeKind::File,
                    "a.go#shadow".to_string(),
                    Some(file),
                    None,
                    Some(Language::Go),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(0),
                    ModuleNodeKind::File,
                    "a.go#ts".to_string(),
                    Some(file),
                    None,
                    Some(Language::TypeScript),
                ),
            ],
            Vec::new(),
        );
        let index = LoweringIndex::build(&db);

        assert_eq!(index.package_for_file(file, Language::Go), Some(first));
        assert_eq!(index.package_for_file(file, Language::TypeScript), None);
        assert_eq!(
            index.module_node_for_file(file, Language::Go),
            db.module_nodes()
                .iter()
                .find(|node| node.file == Some(file) && node.language == Some(Language::Go))
                .map(|node| node.id),
            "the first module node for the file, as the scan returned it"
        );
        assert!(
            index
                .module_node_for_file(file, Language::TypeScript)
                .is_some()
        );
        assert_eq!(
            index.module_node_for_file(FileId::from_raw(99), Language::Go),
            None
        );
    }
}
