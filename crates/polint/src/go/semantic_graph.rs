//! Go semantic-graph projection.
//!
//! This adapter resolves sidecar Go identities back to source-backed host rows;
//! the graph builder and constraint vocabulary remain owned by polint-analysis.

use std::collections::{BTreeMap, HashMap};

use crate::analysis_api::FunctionFact;
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::CallSiteFact;
use crate::analysis_neutral::semantic_graph::build::{
    SemanticGraphBuilder, node_key_from_identity,
};
use crate::analysis_neutral::semantic_graph::constraints::ConstraintKind;
use crate::internal_core::{FileId, Language};

use crate::go::semantic::facts::{
    GoSemanticCallStatus, GoSemanticCallsiteFact, GoSemanticFunctionFact,
};

pub fn project_go_semantic(
    db: &impl AnalysisHost,
    builder: &mut SemanticGraphBuilder,
    semantic_functions: &[GoSemanticFunctionFact],
    semantic_callsites: &[GoSemanticCallsiteFact],
) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let index = CoreRowIndex::build(db);
    let function_node_by_qualified = semantic_functions
        .iter()
        .filter_map(|semantic_function| {
            let function = index.function(semantic_function)?;
            let node = builder.function_node(interner, db, function)?;
            Some((semantic_function.qualified.as_str(), node))
        })
        .collect::<BTreeMap<_, _>>();

    for callsite in semantic_callsites {
        let Some(core_callsite) = index.callsite(callsite) else {
            continue;
        };
        let site_key =
            node_key_from_identity("callsite", &interner.resolve(core_callsite.stable_key));
        let Some(callsite_node) = builder.node_for_key(interner, &site_key) else {
            continue;
        };
        builder.push_constraint(
            interner,
            ConstraintKind::CallConstraint {
                callsite: callsite_node,
            },
            &interner.resolve(callsite.stable_key),
        );
        if callsite.status != GoSemanticCallStatus::ResolvedStatic {
            continue;
        }
        let Some(static_callee) = callsite.static_callee.as_deref() else {
            continue;
        };
        let Some(&target_node) = function_node_by_qualified.get(static_callee) else {
            continue;
        };
        builder.push_constraint(
            interner,
            ConstraintKind::CopyEdge {
                dst: callsite_node,
                src: target_node,
            },
            &interner.resolve(callsite.stable_key),
        );
    }
}

/// The Go functions and call sites a sidecar row joins to, by file and span.
///
/// Each bucket keeps storage order, so a lookup returns the row a scan of the
/// whole family would have found first.
struct CoreRowIndex<'db> {
    functions: HashMap<(FileId, u32, u32), Vec<&'db FunctionFact>>,
    callsites: HashMap<(FileId, u32, u32), &'db CallSiteFact>,
}

impl<'db> CoreRowIndex<'db> {
    fn build(db: &'db impl AnalysisHost) -> Self {
        let mut functions = HashMap::<_, Vec<_>>::new();
        for function in db
            .functions()
            .iter()
            .filter(|function| function.language == Language::Go)
        {
            functions
                .entry((
                    function.file,
                    function.span.start_byte,
                    function.span.end_byte,
                ))
                .or_default()
                .push(function);
        }
        let mut callsites = HashMap::new();
        for callsite in db
            .call_sites()
            .iter()
            .filter(|callsite| callsite.language == Language::Go)
        {
            callsites
                .entry((
                    callsite.file,
                    callsite.span.start_byte,
                    callsite.span.end_byte,
                ))
                .or_insert(callsite);
        }
        Self {
            functions,
            callsites,
        }
    }

    fn function(&self, semantic_function: &GoSemanticFunctionFact) -> Option<&'db FunctionFact> {
        let file = semantic_function.file?;
        let span = semantic_function.span.as_ref()?;
        self.functions
            .get(&(file, span.start_byte, span.end_byte))?
            .iter()
            .copied()
            .find(|function| function.name == semantic_function.name)
    }

    fn callsite(&self, semantic_callsite: &GoSemanticCallsiteFact) -> Option<&'db CallSiteFact> {
        let file = semantic_callsite.file?;
        let span = semantic_callsite.span.as_ref()?;
        self.callsites
            .get(&(file, span.start_byte, span.end_byte))
            .copied()
    }
}

#[cfg(all(test, feature = "lang-go"))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::analysis::calls::facts::{
        CallCallee, CallPrecision, CallSiteFact, CallSyntaxKind, CallTargetStatus,
    };
    use crate::analysis::calls::store::CallOutput;
    use crate::analysis::ids::{CallSiteId, MirBodyId, MirOpId};
    use crate::analysis_neutral::semantic_graph::constraints::ConstraintKind;
    use crate::analysis_neutral::semantic_graph::store::SemanticGraphStore;
    use crate::core::{
        AnalysisDb, FileId, FunctionFact, FunctionId, Language, PackageFact, PackageId, Span,
    };
    use crate::go::semantic::facts::{
        GoSemanticCallStatus, GoSemanticCallsiteFact, GoSemanticCallsiteId, GoSemanticFunctionFact,
        GoSemanticFunctionId, GoSemanticFunctionKind,
    };
    use crate::go::semantic::store::GoSemanticFactsOutput;

    #[test]
    fn direct_go_call_emits_call_constraint_and_static_evidence() {
        let db = go_semantic_db();
        let interner = db.stable_key_interner();
        let semantic = go_semantic_output(
            &interner,
            GoSemanticCallStatus::ResolvedStatic,
            Some("example.com/p.run"),
        );

        let output = project_output(&db, &semantic);

        assert!(output.constraints.iter().any(|constraint| {
            matches!(
                constraint.kind,
                ConstraintKind::CallConstraint { .. } | ConstraintKind::CopyEdge { .. }
            ) && db
                .resolve_stable_key(constraint.stable_key)
                .contains("go-call")
        }));
        SemanticGraphStore::from_output(output, &db.stable_key_interner())
            .expect("Go semantic graph validates");
    }

    #[test]
    fn dynamic_go_call_emits_no_static_target_evidence() {
        let db = go_semantic_db();
        let interner = db.stable_key_interner();
        let semantic = go_semantic_output(&interner, GoSemanticCallStatus::UnresolvedDynamic, None);

        let output = project_output(&db, &semantic);

        let go_copy_edges = output
            .constraints
            .iter()
            .filter(|constraint| {
                matches!(constraint.kind, ConstraintKind::CopyEdge { .. })
                    && db
                        .resolve_stable_key(constraint.stable_key)
                        .contains("go-call")
            })
            .count();
        assert_eq!(go_copy_edges, 0);
    }

    fn project_output(
        db: &AnalysisDb,
        semantic: &GoSemanticFactsOutput,
    ) -> crate::analysis_neutral::semantic_graph::store::SemanticGraphOutput {
        let mut builder = SemanticGraphBuilder::default();
        builder.project_nodes(db);
        builder.project_call_edges_and_constraints(db);
        project_go_semantic(db, &mut builder, &semantic.functions, &semantic.callsites);
        builder.into_output()
    }

    fn go_semantic_db() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("main.go"),
            "main.go".to_string(),
            "package main\nfunc main(){ run() }\nfunc run(){}\n".to_string(),
        );
        db.push_package(PackageFact::new(
            PackageId::from_raw(0),
            file,
            "main".to_string(),
            span_at(file, 0, 10),
            Language::Go,
        ));
        let caller = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "main".to_string(),
            span_at(file, 20, 24),
            Language::Go,
            false,
            false,
            1,
            vec!["run".to_string()],
        ));
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "run".to_string(),
            span_at(file, 34, 37),
            Language::Go,
            false,
            false,
            1,
            Vec::new(),
        ));
        db.replace_call_facts(CallOutput {
            sites: vec![CallSiteFact {
                in_throw: false,
                id: CallSiteId(0),
                language: Language::Go,
                file,
                caller,
                owner_symbol: None,
                body: MirBodyId(0),
                operation: MirOpId(0),
                span: span_at(file, 28, 31),
                kind: CallSyntaxKind::Function,
                callee: CallCallee::Identifier {
                    reference: None,
                    name: "run".to_string(),
                },
                receiver: None,
                arguments: Vec::new(),
                result: None,
                status: CallTargetStatus::Resolved,
                precision: CallPrecision::SetupAware,
                stable_key: db
                    .stable_key_interner()
                    .intern("go-core-callsite:run".to_string()),
            }],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call replace");
        db
    }

    fn go_semantic_output(
        interner: &crate::core::StableKeyInterner,
        status: GoSemanticCallStatus,
        static_callee: Option<&str>,
    ) -> GoSemanticFactsOutput {
        GoSemanticFactsOutput {
            functions: vec![GoSemanticFunctionFact {
                id: GoSemanticFunctionId(0),
                stable_key: interner.intern("go-fn-run"),
                package_id: "example.com/p".to_string(),
                package_path: "example.com/p".to_string(),
                name: "run".to_string(),
                qualified: "example.com/p.run".to_string(),
                signature: "()".to_string(),
                kind: GoSemanticFunctionKind::Function,
                receiver: None,
                relative_file: Some("main.go".to_string()),
                file: Some(FileId::from_raw(0)),
                span: Some(span_at(FileId::from_raw(0), 34, 37)),
            }],
            callsites: vec![GoSemanticCallsiteFact {
                id: GoSemanticCallsiteId(0),
                stable_key: interner.intern("go-call"),
                package_id: "example.com/p".to_string(),
                package_path: "example.com/p".to_string(),
                caller: "example.com/p.main".to_string(),
                static_callee: static_callee.map(str::to_string),
                static_callee_origin: None,
                receiver_type: None,
                mode: crate::go::semantic::facts::GoCallMode::Call,
                status,
                reason: None,
                relative_file: Some("main.go".to_string()),
                file: Some(FileId::from_raw(0)),
                span: Some(span_at(FileId::from_raw(0), 28, 31)),
            }],
            ..GoSemanticFactsOutput::default()
        }
    }

    fn span_at(file: FileId, start: u32, end: u32) -> Span {
        Span::new(file, start, end, 1, start, 1, end)
    }
}
