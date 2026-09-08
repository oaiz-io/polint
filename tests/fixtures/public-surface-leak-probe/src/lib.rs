//! Compile-time probe that locks the supported public surface.
//!
//! This crate is the proxy for an external rule crate. It reaches the polint
//! supported public surface through the SAME entry point real rule authors use:
//! a single `use ::polint::sdk::prelude::*;` glob import and nothing else.
//! (The leading `::` is required because `#![no_implicit_prelude]` disables the
//! implicit extern-prelude that would otherwise let a bare `polint::` path
//! resolve; semantically it is the identical single glob import.)
//!
//! The ONLY acceptable import in this file is `use ::polint::sdk::prelude::*;`.
//! Adding any `use polint::analysis::*`, `use polint::analysis_kernel::*`,
//! `use polint::core::*`, `use polint::cache::*`, `use polint::config::*`,
//! `use polint::cli::*`, `use polint::go::*`, `use polint::ts::*`,
//! `use polint::graph::*`, `use polint::eval::*`, or `use polint::rule_manifest::*`
//! — nor any semantic-store module, raw SQLite connection, or SQL schema type —
//! defeats the entire purpose of the probe — the leak gate test
//! (`crates/polint/tests/public_surface_leak.rs`) reads this file as text and
//! fails if any such import (or any second `use polint::` line) appears.
//!
//! `#![no_implicit_prelude]` below forces EVERY identifier (including `Result`,
//! `String`, `Vec`, `Sized`) to be explicitly imported, so the only names that
//! can be referenced in the witness module are those reachable through the
//! polint prelude glob OR through `::core` / `::std` absolute paths. This makes
//! identifier-reachability changes deterministic: if a future change drops an
//! allow-listed name from the prelude, this probe fails to compile and the gate
//! trips. Preview policy-query names are intentional; raw CFG,
//! call graph, solver, provider, parser, `AnalysisDb`, and graph internals remain
//! unnameable from the supported rule-authoring import path.

#![no_implicit_prelude]
#![allow(dead_code)]

use ::polint::sdk::prelude::*;

// One witness per allow-listed prelude identifier. Each line compiles
// ONLY because the identifier is reachable through `polint::sdk::prelude::*`.
// `PhantomData` is written as `::core::marker::PhantomData` (absolute path)
// because `#![no_implicit_prelude]` disables the std prelude. Lifetime-bearing
// fact views and `RuleCtx` / `RenderOpts` / `JsonReportMeta` are witnessed with
// `<'static>`. Free functions and the schema-URL const are witnessed with a
// value binding because `PhantomData` does not apply to them.
mod allowlist_witness {
    use super::*;

    fn _assert_branchid() -> ::core::marker::PhantomData<BranchId> {
        ::core::marker::PhantomData
    }
    fn _assert_branchobligation() -> ::core::marker::PhantomData<BranchObligation> {
        ::core::marker::PhantomData
    }
    fn _assert_branchobligations() -> ::core::marker::PhantomData<BranchObligations<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_barrierpattern() -> ::core::marker::PhantomData<BarrierPattern> {
        ::core::marker::PhantomData
    }
    fn _assert_callgraph() -> ::core::marker::PhantomData<CallGraph<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_calls() -> ::core::marker::PhantomData<Calls<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_capabilitysupport() -> ::core::marker::PhantomData<CapabilitySupport> {
        ::core::marker::PhantomData
    }
    fn _assert_capabilitysupportstatus() -> ::core::marker::PhantomData<CapabilitySupportStatus> {
        ::core::marker::PhantomData
    }
    fn _assert_capabilitysupportview() -> ::core::marker::PhantomData<CapabilitySupportView> {
        ::core::marker::PhantomData
    }
    fn _assert_capabilitycompleteness() -> ::core::marker::PhantomData<CapabilityCompleteness> {
        ::core::marker::PhantomData
    }
    fn _assert_capabilitycompletenessstatus()
    -> ::core::marker::PhantomData<CapabilityCompletenessStatus> {
        ::core::marker::PhantomData
    }
    fn _assert_changedfiles() -> ::core::marker::PhantomData<ChangedFiles<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_changestatus() -> ::core::marker::PhantomData<ChangeStatus> {
        ::core::marker::PhantomData
    }
    fn _assert_cfg() -> ::core::marker::PhantomData<Cfg<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_colorchoice() -> ::core::marker::PhantomData<ColorChoice> {
        ::core::marker::PhantomData
    }
    fn _assert_complexitymetricfact() -> ::core::marker::PhantomData<ComplexityMetricFact> {
        ::core::marker::PhantomData
    }
    fn _assert_complexitymetrics() -> ::core::marker::PhantomData<ComplexityMetrics<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_completenessview() -> ::core::marker::PhantomData<CompletenessView> {
        ::core::marker::PhantomData
    }
    fn _assert_controlflow() -> ::core::marker::PhantomData<ControlFlow<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_coveragefact() -> ::core::marker::PhantomData<CoverageFact> {
        ::core::marker::PhantomData
    }
    fn _assert_coveragefacts() -> ::core::marker::PhantomData<CoverageFacts<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_dataflow() -> ::core::marker::PhantomData<DataFlow<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_definitionfact() -> ::core::marker::PhantomData<DefinitionFact> {
        ::core::marker::PhantomData
    }
    fn _assert_definitionid() -> ::core::marker::PhantomData<DefinitionId> {
        ::core::marker::PhantomData
    }
    fn _assert_definitionkind() -> ::core::marker::PhantomData<DefinitionKind> {
        ::core::marker::PhantomData
    }
    fn _assert_diagnostic() -> ::core::marker::PhantomData<Diagnostic> {
        ::core::marker::PhantomData
    }
    fn _assert_diagnosticrange() -> ::core::marker::PhantomData<DiagnosticRange> {
        ::core::marker::PhantomData
    }
    fn _assert_eventpattern() -> ::core::marker::PhantomData<EventPattern> {
        ::core::marker::PhantomData
    }
    fn _assert_events() -> ::core::marker::PhantomData<Events<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_evidence() -> ::core::marker::PhantomData<Evidence> {
        ::core::marker::PhantomData
    }
    fn _assert_fileid() -> ::core::marker::PhantomData<FileId> {
        ::core::marker::PhantomData
    }
    fn _assert_filemetricfact() -> ::core::marker::PhantomData<FileMetricFact> {
        ::core::marker::PhantomData
    }
    fn _assert_filemetrics() -> ::core::marker::PhantomData<FileMetrics<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_flowquery() -> ::core::marker::PhantomData<FlowQuery> {
        ::core::marker::PhantomData
    }
    fn _assert_fix() -> ::core::marker::PhantomData<Fix> {
        ::core::marker::PhantomData
    }
    fn _assert_functionfact() -> ::core::marker::PhantomData<FunctionFact> {
        ::core::marker::PhantomData
    }
    fn _assert_functionid() -> ::core::marker::PhantomData<FunctionId> {
        ::core::marker::PhantomData
    }
    fn _assert_functionmetricfact() -> ::core::marker::PhantomData<FunctionMetricFact> {
        ::core::marker::PhantomData
    }
    fn _assert_functionmetrics() -> ::core::marker::PhantomData<FunctionMetrics<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_functions() -> ::core::marker::PhantomData<Functions<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_gotests() -> ::core::marker::PhantomData<GoTests<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_guardpattern() -> ::core::marker::PhantomData<GuardPattern> {
        ::core::marker::PhantomData
    }
    fn _assert_guardquery() -> ::core::marker::PhantomData<GuardQuery> {
        ::core::marker::PhantomData
    }
    fn _assert_importfact() -> ::core::marker::PhantomData<ImportFact> {
        ::core::marker::PhantomData
    }
    fn _assert_importid() -> ::core::marker::PhantomData<ImportId> {
        ::core::marker::PhantomData
    }
    fn _assert_imports() -> ::core::marker::PhantomData<Imports<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_jsonreportmeta() -> ::core::marker::PhantomData<JsonReportMeta<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_jsxattributefact() -> ::core::marker::PhantomData<JsxAttributeFact> {
        ::core::marker::PhantomData
    }
    fn _assert_jsxattributes() -> ::core::marker::PhantomData<JsxAttributes<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_label() -> ::core::marker::PhantomData<Label> {
        ::core::marker::PhantomData
    }
    fn _assert_language() -> ::core::marker::PhantomData<Language> {
        ::core::marker::PhantomData
    }
    fn _assert_lifecyclequery() -> ::core::marker::PhantomData<LifecycleQuery> {
        ::core::marker::PhantomData
    }
    fn _assert_moduleedge() -> ::core::marker::PhantomData<ModuleEdge> {
        ::core::marker::PhantomData
    }
    fn _assert_moduleedgeid() -> ::core::marker::PhantomData<ModuleEdgeId> {
        ::core::marker::PhantomData
    }
    fn _assert_moduleedgekind() -> ::core::marker::PhantomData<ModuleEdgeKind> {
        ::core::marker::PhantomData
    }
    fn _assert_modulegraphfacts() -> ::core::marker::PhantomData<ModuleGraphFacts<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_modulenode() -> ::core::marker::PhantomData<ModuleNode> {
        ::core::marker::PhantomData
    }
    fn _assert_modulenodeid() -> ::core::marker::PhantomData<ModuleNodeId> {
        ::core::marker::PhantomData
    }
    fn _assert_modulenodekind() -> ::core::marker::PhantomData<ModuleNodeKind> {
        ::core::marker::PhantomData
    }
    fn _assert_nodeid() -> ::core::marker::PhantomData<NodeId> {
        ::core::marker::PhantomData
    }
    fn _assert_outputformat() -> ::core::marker::PhantomData<OutputFormat> {
        ::core::marker::PhantomData
    }
    fn _assert_polint_report_json_schema_v1_url() {
        let _ = POLINT_REPORT_JSON_SCHEMA_V1_URL;
    }
    fn _assert_packagefact() -> ::core::marker::PhantomData<PackageFact> {
        ::core::marker::PhantomData
    }
    fn _assert_packageid() -> ::core::marker::PhantomData<PackageId> {
        ::core::marker::PhantomData
    }
    fn _assert_packages() -> ::core::marker::PhantomData<Packages<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_polintreport() -> ::core::marker::PhantomData<PolintReport> {
        ::core::marker::PhantomData
    }
    fn _assert_polinttoolinfo() -> ::core::marker::PhantomData<PolintToolInfo> {
        ::core::marker::PhantomData
    }
    fn _assert_policyconfidence() -> ::core::marker::PhantomData<PolicyConfidence> {
        ::core::marker::PhantomData
    }
    fn _assert_policyprecision() -> ::core::marker::PhantomData<PolicyPrecision> {
        ::core::marker::PhantomData
    }
    fn _assert_policystatus() -> ::core::marker::PhantomData<PolicyStatus> {
        ::core::marker::PhantomData
    }
    fn _assert_policyviolation() -> ::core::marker::PhantomData<PolicyViolation> {
        ::core::marker::PhantomData
    }
    fn _assert_referencefact() -> ::core::marker::PhantomData<ReferenceFact> {
        ::core::marker::PhantomData
    }
    fn _assert_referenceid() -> ::core::marker::PhantomData<ReferenceId> {
        ::core::marker::PhantomData
    }
    fn _assert_referencekind() -> ::core::marker::PhantomData<ReferenceKind> {
        ::core::marker::PhantomData
    }
    fn _assert_references() -> ::core::marker::PhantomData<References<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_reachquery() -> ::core::marker::PhantomData<ReachQuery> {
        ::core::marker::PhantomData
    }
    fn _assert_renderopts() -> ::core::marker::PhantomData<RenderOpts<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_resolutionprecision() -> ::core::marker::PhantomData<ResolutionPrecision> {
        ::core::marker::PhantomData
    }
    fn _assert_resolutionstatus() -> ::core::marker::PhantomData<ResolutionStatus> {
        ::core::marker::PhantomData
    }
    fn _assert_resolvedimportfact() -> ::core::marker::PhantomData<ResolvedImportFact> {
        ::core::marker::PhantomData
    }
    fn _assert_resolvedimportid() -> ::core::marker::PhantomData<ResolvedImportId> {
        ::core::marker::PhantomData
    }
    fn _assert_resolvedimports() -> ::core::marker::PhantomData<ResolvedImports<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_rule() -> ::core::marker::PhantomData<Rule> {
        ::core::marker::PhantomData
    }
    fn _assert_ruleconfigvalue() -> ::core::marker::PhantomData<RuleConfigValue> {
        ::core::marker::PhantomData
    }
    fn _assert_rulectx() -> ::core::marker::PhantomData<RuleCtx<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_ruleerror() -> ::core::marker::PhantomData<RuleError> {
        ::core::marker::PhantomData
    }
    fn _assert_ruleid() -> ::core::marker::PhantomData<RuleId> {
        ::core::marker::PhantomData
    }
    fn _assert_ruleoptions() -> ::core::marker::PhantomData<RuleOptions> {
        ::core::marker::PhantomData
    }
    fn _assert_ruleresult() -> ::core::marker::PhantomData<RuleResult> {
        ::core::marker::PhantomData
    }
    fn _assert_severity() -> ::core::marker::PhantomData<Severity> {
        ::core::marker::PhantomData
    }
    fn _assert_structuredevidencev1() -> ::core::marker::PhantomData<StructuredEvidenceV1> {
        ::core::marker::PhantomData
    }
    fn _assert_sourcefile() -> ::core::marker::PhantomData<SourceFile> {
        ::core::marker::PhantomData
    }
    fn _assert_sinkpattern() -> ::core::marker::PhantomData<SinkPattern> {
        ::core::marker::PhantomData
    }
    fn _assert_sourcepattern() -> ::core::marker::PhantomData<SourcePattern> {
        ::core::marker::PhantomData
    }
    fn _assert_sourcefiles() -> ::core::marker::PhantomData<SourceFiles<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_span() -> ::core::marker::PhantomData<Span> {
        ::core::marker::PhantomData
    }
    fn _assert_stringliteralfact() -> ::core::marker::PhantomData<StringLiteralFact> {
        ::core::marker::PhantomData
    }
    fn _assert_stringliterals() -> ::core::marker::PhantomData<StringLiterals<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_suggestion() -> ::core::marker::PhantomData<Suggestion> {
        ::core::marker::PhantomData
    }
    fn _assert_symbolfact() -> ::core::marker::PhantomData<SymbolFact> {
        ::core::marker::PhantomData
    }
    fn _assert_symbolid() -> ::core::marker::PhantomData<SymbolId> {
        ::core::marker::PhantomData
    }
    fn _assert_symbolkind() -> ::core::marker::PhantomData<SymbolKind> {
        ::core::marker::PhantomData
    }
    fn _assert_symbolnamespace() -> ::core::marker::PhantomData<SymbolNamespace> {
        ::core::marker::PhantomData
    }
    fn _assert_symbolprecision() -> ::core::marker::PhantomData<SymbolPrecision> {
        ::core::marker::PhantomData
    }
    fn _assert_symbolresolutionstatus() -> ::core::marker::PhantomData<SymbolResolutionStatus> {
        ::core::marker::PhantomData
    }
    fn _assert_symbols() -> ::core::marker::PhantomData<Symbols<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_testfact() -> ::core::marker::PhantomData<TestFact> {
        ::core::marker::PhantomData
    }
    fn _assert_testsuitemetrics() -> ::core::marker::PhantomData<TestSuiteMetrics<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_textrange() -> ::core::marker::PhantomData<TextRange> {
        ::core::marker::PhantomData
    }
    fn _assert_tsclassfact() -> ::core::marker::PhantomData<TsClassFact> {
        ::core::marker::PhantomData
    }
    fn _assert_tsclasses() -> ::core::marker::PhantomData<TsClasses<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_tscomponentfact() -> ::core::marker::PhantomData<TsComponentFact> {
        ::core::marker::PhantomData
    }
    fn _assert_tscomponents() -> ::core::marker::PhantomData<TsComponents<'static>> {
        ::core::marker::PhantomData
    }
    fn _assert_unresolvedreason() -> ::core::marker::PhantomData<UnresolvedReason> {
        ::core::marker::PhantomData
    }
    fn _assert_collect_go_tests() {
        let _ = collect_go_tests;
    }
    fn _assert_diagnostics_from_json_report() {
        let _ = diagnostics_from_json_report;
    }
    fn _assert_file_in_scope() {
        let _ = file_in_scope;
    }
    fn _assert_file_matches_globs() {
        let _ = file_matches_globs;
    }
    fn _assert_glob_matches() {
        let _ = glob_matches;
    }
}
