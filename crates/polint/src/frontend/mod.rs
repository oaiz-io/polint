//! Open language-frontend registry.
//!
//! Language syntax stages dispatch through [`LanguageFrontend`] so a new language
//! is one frontend module plus one `register` line in [`build_default_registry`].

mod registry;

use std::sync::OnceLock;

use crate::core::Language;

#[cfg(test)]
use crate::analysis_kernel::PrecisionCeiling;
pub(crate) use crate::frontend_api::AnalysisUnit;
#[cfg(test)]
pub(crate) use crate::frontend_api::{FrontendProfile, LanguageFrontend};
pub(crate) use registry::{
    FrontendRegistry, FrontendRegistryExt, LANGUAGE_IDS_GO, LANGUAGE_IDS_GO_AND_TS,
    LANGUAGE_IDS_NONE, LANGUAGE_IDS_TS, LanguageId, build_default_registry,
};

pub(crate) use crate::go::{FAMILY_GO, GoFrontend};
pub(crate) use crate::ts::{FAMILY_TYPESCRIPT_JAVASCRIPT, TsJsFrontend};

static DEFAULT_REGISTRY: OnceLock<FrontendRegistry> = OnceLock::new();

/// Process-wide default frontend registry (Go then TS/JS in registration order).
pub(crate) fn frontend_registry() -> &'static FrontendRegistry {
    DEFAULT_REGISTRY.get_or_init(build_default_registry)
}

pub(crate) trait LanguageRegistryExt {
    fn id(self) -> LanguageId;
}

impl LanguageRegistryExt for Language {
    fn id(self) -> LanguageId {
        frontend_registry()
            .id_for_public_language(self)
            .unwrap_or(LanguageId::UNREGISTERED)
    }
}

pub(crate) trait LanguageIdRegistryExt {
    fn to_public_language(self) -> Language;
}

impl LanguageIdRegistryExt for LanguageId {
    fn to_public_language(self) -> Language {
        frontend_registry()
            .public_language_for(self)
            .unwrap_or(Language::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{LanguageIdRegistryExt, LanguageRegistryExt};
    use crate::analysis_kernel::incremental::CacheStats;
    use crate::analysis_kernel::{AnalysisKernel, ProviderCtx, ProviderRunResult};
    use crate::analysis_plan::AnalysisPlan;
    use crate::cache::Cache;
    use crate::config::load_config;
    use crate::core::AnalysisDb;
    use crate::diagnostics::Diagnostic;
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const NOOP_PROFILE: FrontendProfile = FrontendProfile {
        name: "noop",
        family: "noop",
        produces: &[],
        precision_ceiling: PrecisionCeiling::Syntax,
    };

    struct NoopFrontend {
        id: LanguageId,
        analyze_calls: Arc<AtomicUsize>,
    }

    impl LanguageFrontend for NoopFrontend {
        fn id(&self) -> LanguageId {
            self.id
        }

        fn handles(&self, path: &Path) -> bool {
            path.extension().and_then(|ext| ext.to_str()) == Some("noop")
        }

        fn profile(&self) -> &'static FrontendProfile {
            &NOOP_PROFILE
        }

        fn analyze(
            &self,
            _ctx: &mut ProviderCtx<'_>,
            _unit: &AnalysisUnit<'_>,
        ) -> ProviderRunResult {
            self.analyze_calls.fetch_add(1, Ordering::SeqCst);
            ProviderRunResult {
                counts: Default::default(),
                diagnostics: Vec::<Diagnostic>::new(),
                cache_stats: CacheStats::default(),
                output_digest: None,
                execution: Default::default(),
            }
        }
    }

    #[test]
    fn adding_a_frontend_touches_only_its_own_module_and_the_registry() {
        // Host composition for a third frontend: one register() line (plus this module).
        let analyze_calls = Arc::new(AtomicUsize::new(0));
        let mut registry = FrontendRegistry::new();
        registry.register(|id| Box::new(GoFrontend::new(id)));
        registry.register(|id| Box::new(TsJsFrontend::new(id)));
        let noop_calls = Arc::clone(&analyze_calls);
        let noop_id = registry.register(move |id| {
            Box::new(NoopFrontend {
                id,
                analyze_calls: noop_calls,
            })
        });

        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("policy.noop"), "// noop\n").expect("write noop file");
        let loaded = load_config(temp.path()).expect("config");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::empty();
        let mut db = AnalysisDb::default();
        db.add_file(
            temp.path().join("policy.noop"),
            "policy.noop".to_string(),
            "// noop\n".to_string(),
        );

        let input_snapshot = crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            &loaded,
            &db,
            "config",
            "rules",
            plan.digest(),
            AnalysisKernel::provider_manifests(),
        );
        let capability_support = plan.support_view().clone();
        let scc_closure = None;
        let upstream_digests = BTreeMap::new();

        let owned: Vec<crate::core::SourceFile> = db
            .files()
            .iter()
            .filter(|file| registry.get(noop_id).unwrap().handles(&file.path))
            .cloned()
            .collect();
        assert_eq!(owned.len(), 1);

        let scheduled = registry.scheduled_for(Path::new("policy.noop"));
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].id(), noop_id);

        let files: Vec<&crate::core::SourceFile> = owned.iter().collect();
        let unit = AnalysisUnit {
            files: &files,
            root: temp.path(),
        };
        let _host_scope = crate::analysis_kernel::host::ProviderHostSessionScope::install(
            crate::analysis_kernel::host::ProviderHostSession {
                cache,
                loaded,
                input_snapshot,
                plan,
                capability_support,
                scc_closure,
            },
        );
        let mut host_services = crate::analysis_kernel::host::FacadeHostServices::default();
        let mut host_attachment = crate::analysis_kernel::host::FacadeHostAttachment::default();
        let mut ctx = ProviderCtx {
            facts: &mut db,
            host: &mut host_services,
            config_digest: "config",
            rule_digest: "rules",
            parallel: false,
            upstream_digests: &upstream_digests,
            host_attachment: &mut host_attachment,
        };
        let result = scheduled[0].analyze(&mut ctx, &unit);
        assert_eq!(analyze_calls.load(Ordering::SeqCst), 1);
        assert!(result.diagnostics.is_empty());
        assert!(db.functions().is_empty());
        assert!(db.imports().is_empty());
        assert!(db.packages().is_empty());
    }

    #[test]
    fn default_registry_exposes_go_and_ts_frontends() {
        let registry = frontend_registry();
        assert!(registry.by_name("go").is_some());
        assert!(registry.by_name("ts").is_some());
        assert_eq!(Language::Go.id(), registry.by_name("go").unwrap().id());
        assert_eq!(
            Language::TypeScript.id(),
            registry.by_name("ts").unwrap().id()
        );
        assert_eq!(
            registry.by_name("go").unwrap().id().to_public_language(),
            Language::Go
        );
        assert_eq!(
            registry.by_name("ts").unwrap().id().to_public_language(),
            Language::TypeScript
        );
    }

    #[test]
    fn language_syntax_providers_dispatch_through_frontend_registry() {
        let provider_src = include_str!("../analysis_kernel/provider.rs");
        assert!(
            provider_src.contains("run_registered_frontend_syntax"),
            "syntax providers must dispatch through the frontend registry helper"
        );
        assert!(
            !provider_src.contains("crate::go::analyze_with_plan_options_and_cache_stats"),
            "old go syntax call site must be deleted"
        );
        assert!(
            !provider_src.contains("crate::ts::analyze_with_plan_options_and_cache_stats"),
            "old ts syntax call site must be deleted"
        );
        assert!(
            provider_src.contains("frontend.analyze(ctx, &unit)"),
            "product path must call LanguageFrontend::analyze"
        );
    }
}
