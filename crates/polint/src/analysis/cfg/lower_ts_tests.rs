//! CFG lowering tests that need the TypeScript frontend + MIR lowerer.
#![cfg(test)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::analysis::cfg::facts::{BasicBlockKind, CfgEdgeKind};
use crate::analysis_neutral::cfg::lower::lower_cfg;
use crate::core::AnalysisDb;

#[test]
fn ts_cfg_lowers_edges_from_production_mir_terminators() {
    let mut db = AnalysisDb::new();
    db.add_file(
        std::path::PathBuf::from("flow.ts"),
        "flow.ts".to_string(),
        "export function flow(x) { if (x) {} while (x) {} switch (x) { case true: break; } }"
            .to_string(),
    );
    assert!(
        crate::ts::analyze_with_options(
            &mut db,
            &crate::analysis_api::DisabledAnalysisCache,
            "",
            "",
            false
        )
        .is_empty()
    );
    let mir = crate::ts::lower_ts_mir(&db);
    db.replace_semantic_mir(mir)
        .expect("MIR output should store");

    let edge_kinds = lower_cfg(&db, |_| true)
        .edges
        .into_iter()
        .map(|edge| edge.kind)
        .collect::<BTreeSet<_>>();
    assert!(edge_kinds.contains(&CfgEdgeKind::True));
    assert!(edge_kinds.contains(&CfgEdgeKind::False));
    assert!(edge_kinds.contains(&CfgEdgeKind::SwitchCase));
    assert!(edge_kinds.contains(&CfgEdgeKind::DefaultCase));
    assert!(edge_kinds.contains(&CfgEdgeKind::Normal));
}

#[test]
fn ts_cfg_throw_prevents_impossible_fallthrough() {
    let mut db = AnalysisDb::new();
    db.add_file(
        std::path::PathBuf::from("throw.ts"),
        "throw.ts".to_string(),
        "export function fail(value) { throw new Error(value); value = 1; }".to_string(),
    );
    assert!(
        crate::ts::analyze_with_options(
            &mut db,
            &crate::analysis_api::DisabledAnalysisCache,
            "",
            "",
            false
        )
        .is_empty()
    );
    let mir = crate::ts::lower_ts_mir(&db);
    db.replace_semantic_mir(mir)
        .expect("MIR output should store");
    let output = lower_cfg(&db, |_| true);
    let unreachable_blocks = output
        .blocks
        .iter()
        .filter(|block| block.kind == BasicBlockKind::Unreachable)
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();

    assert!(
        output
            .edges
            .iter()
            .any(|edge| edge.kind == CfgEdgeKind::Throw)
    );
    assert!(
        output
            .blocks
            .iter()
            .any(|block| block.kind == BasicBlockKind::ExitExceptional && block.reachable)
    );
    assert!(
        output
            .blocks
            .iter()
            .any(|block| block.kind == BasicBlockKind::ExitNormal && !block.reachable)
    );
    assert!(output.edges.iter().all(|edge| {
        !unreachable_blocks.contains(&edge.to_block)
            || unreachable_blocks.contains(&edge.from_block)
    }));
}

#[test]
fn ts_cfg_async_cleanup_and_unsupported_rows_are_truthful() {
    let mut db = AnalysisDb::new();
    db.add_file(
        std::path::PathBuf::from("effects.ts"),
        "effects.ts".to_string(),
        r#"
export async function load(promise, value) {
  await promise;
  try { value?.run(); } finally { cleanup(); }
  return import("./module.js");
}
export function* values(value) { yield value; }
"#
        .to_string(),
    );
    assert!(
        crate::ts::analyze_with_options(
            &mut db,
            &crate::analysis_api::DisabledAnalysisCache,
            "",
            "",
            false
        )
        .is_empty()
    );
    let mir = crate::ts::lower_ts_mir(&db);
    db.replace_semantic_mir(mir)
        .expect("MIR output should store");
    let output = lower_cfg(&db, |_| true);
    let edge_kinds = output
        .edges
        .iter()
        .map(|edge| edge.kind)
        .collect::<BTreeSet<_>>();
    let unsupported = output
        .unsupported
        .iter()
        .map(|row| row.construct.as_str())
        .collect::<BTreeSet<_>>();

    assert!(edge_kinds.contains(&CfgEdgeKind::AwaitSuspend));
    assert!(edge_kinds.contains(&CfgEdgeKind::AwaitResume));
    assert!(edge_kinds.contains(&CfgEdgeKind::YieldSuspend));
    assert!(edge_kinds.contains(&CfgEdgeKind::YieldResume));
    assert!(edge_kinds.contains(&CfgEdgeKind::Finally));
    assert!(edge_kinds.contains(&CfgEdgeKind::Cleanup));
    assert!(edge_kinds.contains(&CfgEdgeKind::OptionalChain));
    assert!(edge_kinds.contains(&CfgEdgeKind::Unknown));
    assert!(unsupported.contains("dynamic import"));
}

#[test]
fn a_recoverable_syntax_error_is_recorded_as_unsupported_not_ignored() {
    let mut db = AnalysisDb::new();
    db.add_file(
        PathBuf::from("ok.ts"),
        "ok.ts".to_string(),
        "export function ok(value: number) { return value + 1; }".to_string(),
    );
    db.add_file(
        PathBuf::from("broken.tsx"),
        "broken.tsx".to_string(),
        "const x = <div></span>;".to_string(),
    );
    let diagnostics = crate::ts::analyze_with_options(
        &mut db,
        &crate::analysis_api::DisabledAnalysisCache,
        "",
        "",
        false,
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "parser/ts" && diagnostic.file == "broken.tsx"),
        "primary adapter must still emit parser/ts for the broken file: {diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.file != "ok.ts" || diagnostic.rule_id != "parser/ts"),
        "valid file must stay free of parser/ts diagnostics"
    );

    db.replace_semantic_mir(crate::ts::lower_ts_mir(&db))
        .expect("store MIR unsupported rows");

    let rows = crate::analysis::unknown_taxonomy::collect::graph_engine_unknowns(&db);
    assert!(
        rows.iter().any(|row| {
            row.family.as_deref() == Some("UnsupportedSemantic")
                && row.file == "broken.tsx"
                && row.reason.as_deref() == Some(crate::ts::PARSER_RECOVERY_CONSTRUCT)
        }),
        "inspect unknowns must surface parser-recovery unsupported rows: {rows:?}"
    );
    assert!(
        rows.iter().all(|row| row.file != "ok.ts"),
        "valid file must not gain parser-recovery unknowns: {rows:?}"
    );
    assert!(
        db.functions()
            .iter()
            .any(|function| function.name == "ok" && db.path_for(function.file) == "ok.ts"),
        "valid file analysis must remain intact"
    );
}
