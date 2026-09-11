//! TypeScript/JavaScript [`LanguageFrontend`] registered by the polint composition root.

use std::path::Path;

#[cfg(feature = "lang-typescript")]
use crate::analysis_api::DisabledAnalysisCache;
#[cfg(not(feature = "lang-typescript"))]
use crate::analysis_api::{
    CacheStats, ProviderExecution, ProviderFailureReason, ProviderFailureStage,
};
use crate::analysis_api::{PrecisionCeiling, ProviderCtx, ProviderRunResult};
#[cfg(not(feature = "lang-typescript"))]
use crate::diagnostics::{Diagnostic, TextRange};
use crate::frontend_api::{AnalysisUnit, FrontendProfile, LanguageFrontend};
use crate::internal_core::{Language, LanguageId};

#[cfg(feature = "lang-typescript")]
use crate::ts::adapter::analyze_files_with_plan_options_and_cache_stats;

pub const FAMILY_TYPESCRIPT_JAVASCRIPT: &str = "typescript_javascript";

const TS_PRODUCES: &[&str] = &[
    "functions",
    "imports",
    "ts_components",
    "ts_classes",
    "string_literals",
    "jsx_attributes",
];

pub const TS_FRONTEND_PROFILE: FrontendProfile = FrontendProfile {
    name: "ts",
    family: FAMILY_TYPESCRIPT_JAVASCRIPT,
    produces: TS_PRODUCES,
    precision_ceiling: PrecisionCeiling::Syntax,
};

pub struct TsJsFrontend {
    id: LanguageId,
}

impl TsJsFrontend {
    pub fn new(id: LanguageId) -> Self {
        Self { id }
    }
}

impl LanguageFrontend for TsJsFrontend {
    fn id(&self) -> LanguageId {
        self.id
    }

    fn handles(&self, path: &Path) -> bool {
        Language::from_path(path).is_ts_family()
    }

    fn profile(&self) -> &'static FrontendProfile {
        &TS_FRONTEND_PROFILE
    }

    #[cfg(feature = "lang-typescript")]
    fn analyze(&self, ctx: &mut ProviderCtx<'_>, unit: &AnalysisUnit<'_>) -> ProviderRunResult {
        let _ = unit.root;
        let cache = ctx.host.analysis_cache().unwrap_or(&DisabledAnalysisCache);
        analyze_files_with_plan_options_and_cache_stats(
            ctx.facts,
            unit.files,
            cache,
            ctx.config_digest,
            ctx.rule_digest,
            ctx.host.plan_digest(),
            ctx.parallel,
        )
    }
    #[cfg(not(feature = "lang-typescript"))]
    fn analyze(&self, _ctx: &mut ProviderCtx<'_>, unit: &AnalysisUnit<'_>) -> ProviderRunResult {
        if unit.files.is_empty() {
            return ProviderRunResult {
                counts: Default::default(),
                diagnostics: Vec::new(),
                cache_stats: CacheStats::default(),
                output_digest: None,
                execution: ProviderExecution::Succeeded,
            };
        }
        let file = unit.files[0].relative_path.clone();
        ProviderRunResult {
                counts: Default::default(),
            diagnostics: vec![Diagnostic::warning(
                "polint/capability",
                file,
                TextRange::point(1, 1),
                "TypeScript/JavaScript analysis is unavailable because the `lang-typescript` feature is disabled.",
            )
            .with_evidence("status", "unsupported")
            .with_evidence("reason", "language-feature-disabled")
            .with_evidence("feature", "lang-typescript")
            .with_evidence("language", "typescript-javascript")],
            cache_stats: CacheStats::default(),
            output_digest: None,
            execution: ProviderExecution::Failed {
                stage: ProviderFailureStage::Setup,
                reason: ProviderFailureReason::Unsupported,
            },
        }
    }
}
