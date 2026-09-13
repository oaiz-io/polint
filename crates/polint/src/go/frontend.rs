//! Go [`LanguageFrontend`] registered by the polint composition root.

use std::path::Path;

#[cfg(feature = "lang-go")]
use crate::analysis_api::DisabledAnalysisCache;
#[cfg(not(feature = "lang-go"))]
use crate::analysis_api::{
    CacheStats, ProviderExecution, ProviderFailureReason, ProviderFailureStage,
};
use crate::analysis_api::{PrecisionCeiling, ProviderCtx, ProviderRunResult};
#[cfg(not(feature = "lang-go"))]
use crate::diagnostics::{Diagnostic, TextRange};
use crate::frontend_api::{AnalysisUnit, FrontendProfile, LanguageFrontend};
use crate::internal_core::{Language, LanguageId};

#[cfg(feature = "lang-go")]
use crate::go::adapter::analyze_files_with_plan_options_and_cache_stats;

pub const FAMILY_GO: &str = "go";

const GO_PRODUCES: &[&str] = &[
    "packages",
    "functions",
    "imports",
    "go_tests",
    "go_types",
    "branch_obligations",
];

pub const GO_FRONTEND_PROFILE: FrontendProfile = FrontendProfile {
    name: "go",
    family: FAMILY_GO,
    produces: GO_PRODUCES,
    precision_ceiling: PrecisionCeiling::Syntax,
};

pub struct GoFrontend {
    id: LanguageId,
}

impl GoFrontend {
    pub fn new(id: LanguageId) -> Self {
        Self { id }
    }
}

impl LanguageFrontend for GoFrontend {
    fn id(&self) -> LanguageId {
        self.id
    }

    fn handles(&self, path: &Path) -> bool {
        Language::from_path(path) == Language::Go
    }

    fn profile(&self) -> &'static FrontendProfile {
        &GO_FRONTEND_PROFILE
    }

    #[cfg(feature = "lang-go")]
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
    #[cfg(not(feature = "lang-go"))]
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
            diagnostics: vec![
                Diagnostic::warning(
                    "polint/capability",
                    file,
                    TextRange::point(1, 1),
                    "Go analysis is unavailable because the `lang-go` feature is disabled.",
                )
                .with_evidence("status", "unsupported")
                .with_evidence("reason", "language-feature-disabled")
                .with_evidence("feature", "lang-go")
                .with_evidence("language", "go"),
            ],
            cache_stats: CacheStats::default(),
            output_digest: None,
            execution: ProviderExecution::Failed {
                stage: ProviderFailureStage::Setup,
                reason: ProviderFailureReason::Unsupported,
            },
        }
    }
}
