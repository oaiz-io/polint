//! Public rule-authoring API for polint.
//!
//! Repo-local and example rule authors should start with
//! `use polint::sdk::prelude::*;`. The prelude re-exports the stable rule
//! authoring types, fact views, diagnostics, severity types, scope helpers
//! ([`scope::file_in_scope`], [`scope::glob_matches`]), and [`RuleResult`](prelude::RuleResult)
//! ([`RuleError`](prelude::RuleError)) without depending on `polint::core` directly.

#![deny(missing_docs)]

pub mod facts;
pub mod policy;
pub mod scope;

use crate::core::{FileId, TestFact};
use facts::GoTests;

/// Collects [`TestFact`] references for `file` from a typed [`GoTests`] fact view.
pub fn collect_go_tests<'a>(tests: GoTests<'a>, file: FileId) -> Vec<&'a TestFact> {
    tests.for_file(file).collect()
}

/// Re-exports of the stable rule-authoring surface.
///
/// Importing `use polint::sdk::prelude::*;` is the recommended way to write a rule
/// — it pulls in the rule contract, the fact types, the diagnostic/severity types,
/// the path-scoping helpers, and [`RuleResult`](crate::sdk::prelude::RuleResult) in one star-import.
pub mod prelude {
    pub use crate::core::{
        BranchId, BranchObligation, CapabilityCompleteness, CapabilityCompletenessStatus,
        CapabilitySupport, CapabilitySupportStatus, CapabilitySupportView, ChangeStatus,
        CompletenessView, ComplexityMetricFact, CoverageFact, DefinitionFact, DefinitionId,
        DefinitionKind, FileId, FileMetricFact, FunctionFact, FunctionId, FunctionMetricFact,
        GoTypeDeclFact, GoTypeDeclKind, ImportFact, ImportId, JsxAttributeFact, Language,
        ModuleEdge, ModuleEdgeId, ModuleEdgeKind, ModuleNode, ModuleNodeId, ModuleNodeKind, NodeId,
        PackageFact, PackageId, ReferenceFact, ReferenceId, ReferenceKind, ResolutionPrecision,
        ResolutionStatus, ResolvedImportFact, ResolvedImportId, Rule, RuleConfigValue, RuleCtx,
        RuleId, RuleOptions, SourceFile, Span, StringLiteralFact, SymbolFact, SymbolId, SymbolKind,
        SymbolNamespace, SymbolPrecision, SymbolResolutionStatus, TestFact, TextRange, TsClassFact,
        TsComponentFact, UnresolvedReason,
    };
    pub use crate::diagnostics::{
        ColorChoice, Diagnostic, Evidence, Fix, JsonReportMeta, Label, OutputFormat,
        POLINT_REPORT_JSON_SCHEMA_V1_URL, PolintReport, PolintToolInfo, RenderOpts, Severity,
        StructuredEvidenceV1, Suggestion, TextRange as DiagnosticRange,
        diagnostics_from_json_report,
    };
    pub use crate::rule_error::{RuleError, RuleResult};
    pub use crate::sdk::collect_go_tests;
    pub use crate::sdk::facts::{
        BranchObligations, CallGraph, Calls, Cfg, ChangedFiles, ComplexityMetrics, ControlFlow,
        CoverageFacts, DataFlow, Events, FileMetrics, FunctionMetrics, Functions, GoTests,
        GoTypeDecls, Imports, JsxAttributes, ModuleGraphFacts, Packages, References,
        ResolvedImports, SourceFiles, StringLiterals, Symbols, TestSuiteMetrics, TsClasses,
        TsComponents,
    };
    pub use crate::sdk::policy::{
        BarrierPattern, EventPattern, FlowQuery, GuardPattern, GuardQuery, LifecycleQuery,
        PolicyConfidence, PolicyPrecision, PolicyStatus, PolicyViolation, ReachQuery, SinkPattern,
        SourcePattern,
    };
    pub use crate::sdk::scope::{file_in_scope, file_matches_globs, glob_matches};
}

/// Hidden implementation details used by generated rule code.
#[doc(hidden)]
pub mod __private {
    pub use crate::core::{AnalysisDb, Capabilities, RuleKind, RuleMeta};
    pub use crate::sdk::facts::FactView;

    use crate::core::{Rule, RuleCtx};
    use crate::rule_error::RuleResult;

    /// Hidden generated-code carrier for fact-view manifest metadata.
    #[doc(hidden)]
    pub struct FactViewRequirement {
        view_type: &'static str,
        canonical_path: &'static str,
        capability: &'static str,
        parameter_name: &'static str,
    }

    impl FactViewRequirement {
        /// Creates hidden fact-view metadata for generated `#[polint::rule]` code.
        #[doc(hidden)]
        pub fn generated(
            view_type: &'static str,
            canonical_path: &'static str,
            capability: &'static str,
            parameter_name: &'static str,
        ) -> Self {
            Self {
                view_type,
                canonical_path,
                capability,
                parameter_name,
            }
        }
    }

    /// Constructs an opaque rule for generated `#[polint::rule]` code.
    #[doc(hidden)]
    pub fn make_rule<M, C, R>(meta: M, capabilities: C, run: R) -> Rule
    where
        M: Fn() -> RuleMeta + Send + Sync + 'static,
        C: Fn() -> Capabilities + Send + Sync + 'static,
        R: Fn(&AnalysisDb, &mut RuleCtx<'_>) -> RuleResult + Send + Sync + 'static,
    {
        Rule::from_parts(meta, capabilities, run)
    }

    /// Constructs an opaque rule with manifest metadata for generated `#[polint::rule]` code.
    #[doc(hidden)]
    pub fn make_rule_with_manifest<M, C, R>(
        meta: M,
        capabilities: C,
        fact_views: Vec<FactViewRequirement>,
        run: R,
    ) -> Rule
    where
        M: Fn() -> RuleMeta + Send + Sync + 'static,
        C: Fn() -> Capabilities + Send + Sync + 'static,
        R: Fn(&AnalysisDb, &mut RuleCtx<'_>) -> RuleResult + Send + Sync + 'static,
    {
        let fact_views = fact_views
            .into_iter()
            .map(|view| {
                crate::rule_manifest::FactViewRequirement::generated(
                    view.view_type,
                    view.canonical_path,
                    view.capability,
                    view.parameter_name,
                )
            })
            .collect();
        Rule::from_parts_with_fact_views(meta, capabilities, fact_views, run)
    }
}

#[cfg(test)]
mod tests {
    use crate::core::AnalysisDb;
    use crate::sdk::facts::FactView;
    use crate::sdk::prelude::*;

    #[polint::rule(
        id = "examples/prelude-smoke",
        description = "Prelude smoke rule",
        severity = "warn"
    )]
    fn prelude_smoke(
        ctx: &mut RuleCtx<'_>,
        files: SourceFiles<'_>,
        imports: Imports<'_>,
        literals: StringLiterals<'_>,
        jsx: JsxAttributes<'_>,
    ) -> RuleResult {
        assert_eq!(files.iter().count(), 0);
        assert_eq!(imports.edges().count(), 0);
        assert!(literals.all().is_empty());
        assert!(jsx.all().is_empty());
        ctx.warn(&Span::point(FileId::from_raw(0), 1, 1), "prelude warning");
        Ok(())
    }

    #[polint::rule(
        id = "examples/metric-prelude-smoke",
        description = "Metric prelude smoke rule",
        severity = "warn"
    )]
    fn metric_prelude_smoke(
        _ctx: &mut RuleCtx<'_>,
        file_metrics: FileMetrics<'_>,
        function_metrics: FunctionMetrics<'_>,
        complexity_metrics: ComplexityMetrics<'_>,
    ) -> RuleResult {
        assert_eq!(file_metrics.iter().count(), 0);
        assert_eq!(function_metrics.iter().count(), 0);
        assert_eq!(complexity_metrics.iter().count(), 0);
        Ok(())
    }

    #[test]
    fn sdk_prelude_exports_rule_authoring_surface() {
        fn assert_exported<T>() {}
        assert_exported::<PackageFact>();
        assert_exported::<TsClassFact>();
        assert_exported::<FileMetricFact>();
        assert_exported::<FunctionMetricFact>();
        assert_exported::<ComplexityMetricFact>();
        assert_exported::<ResolvedImportFact>();
        assert_exported::<ResolvedImportId>();
        assert_exported::<ModuleNode>();
        assert_exported::<ModuleNodeId>();
        assert_exported::<ModuleEdge>();
        assert_exported::<ModuleEdgeId>();
        assert_exported::<ModuleNodeKind>();
        assert_exported::<ModuleEdgeKind>();
        assert_exported::<ResolutionStatus>();
        assert_exported::<ResolutionPrecision>();
        assert_exported::<UnresolvedReason>();
        assert_exported::<SymbolId>();
        assert_exported::<DefinitionId>();
        assert_exported::<ReferenceId>();
        assert_exported::<SymbolFact>();
        assert_exported::<DefinitionFact>();
        assert_exported::<ReferenceFact>();
        assert_exported::<SymbolKind>();
        assert_exported::<SymbolNamespace>();
        assert_exported::<DefinitionKind>();
        assert_exported::<ReferenceKind>();
        assert_exported::<SymbolPrecision>();
        assert_exported::<SymbolResolutionStatus>();
        assert_exported::<ResolvedImports<'static>>();
        assert_exported::<ModuleGraphFacts<'static>>();
        assert_exported::<Symbols<'static>>();
        assert_exported::<References<'static>>();

        let db = AnalysisDb::new();
        let rule = prelude_smoke();
        let capabilities = rule.capabilities();
        assert!(capabilities.syntax);
        assert!(capabilities.imports);
        assert!(capabilities.string_literals);
        assert!(capabilities.jsx_attributes);
        let manifest = rule.manifest(None);
        assert_eq!(manifest.id, "examples/prelude-smoke");
        assert_eq!(
            manifest
                .fact_views
                .iter()
                .map(|view| view.view_type.as_str())
                .collect::<Vec<_>>(),
            ["Imports", "JsxAttributes", "SourceFiles", "StringLiterals"]
        );
        let metric_capabilities = metric_prelude_smoke().capabilities();
        assert!(metric_capabilities.file_metrics);
        assert!(metric_capabilities.function_metrics);
        assert!(metric_capabilities.complexity_metrics);

        let mut ctx = RuleCtx::new(&db, rule.meta(), RuleOptions::default());
        assert_eq!(
            ctx.completeness().status_for("syntax"),
            CapabilityCompletenessStatus::Unknown
        );
        let tests = GoTests::build(&db);
        assert!(collect_go_tests(tests, FileId::from_raw(0)).is_empty());
        rule.run(&db, &mut ctx).expect("prelude rule runs");
        let diagnostics = ctx.into_diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Warn);
        assert_eq!(diagnostics[0].file, "<unknown>");
    }

    #[test]
    fn sdk_prelude_exports_capability_support_view() {
        fn assert_exported<T>() {}
        assert_exported::<CapabilitySupport>();
        assert_exported::<CapabilitySupportStatus>();
        assert_exported::<CapabilitySupportView>();
        assert_exported::<CapabilityCompleteness>();
        assert_exported::<CapabilityCompletenessStatus>();
        assert_exported::<CompletenessView>();

        let view = CapabilitySupportView::empty();
        assert!(view.entries().is_empty());
    }
}
