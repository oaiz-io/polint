use super::host;
use super::incremental::{CacheStats, Digest, DigestKind, InputSnapshot};
use crate::analysis::summaries::provider::SccClosureProviderOutput;
use crate::analysis_api::ProviderExecution;
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::LoadedConfig;
use crate::core::{AnalysisDb, CapabilitySupportView};
use crate::diagnostics::{Diagnostic, TextRange};
use crate::frontend::{LanguageIdRegistryExt, LanguageRegistryExt};
use std::collections::BTreeMap;

pub(crate) use crate::analysis_api::{
    CachePolicy, PrecisionCeiling, Provider, ProviderCtx, ProviderKind, ProviderManifest,
    ProviderRunResult, SchemaVersion,
};

/// Facade-local view over [`ProviderCtx`] for providers that still live in this crate.
struct CtxHandle<'a> {
    db: &'a mut AnalysisDb,
    cache: Cache,
    loaded: LoadedConfig,
    input_snapshot: InputSnapshot,
    config_digest: &'a str,
    rule_digest: &'a str,
    plan: AnalysisPlan,
    #[allow(dead_code)]
    parallel: bool,
    upstream_digests: &'a BTreeMap<&'static str, Digest>,
    capability_support: CapabilitySupportView,
    scc_closure: Option<SccClosureProviderOutput>,
}

impl<'a> CtxHandle<'a> {
    fn from_ctx(ctx: &'a mut ProviderCtx<'_>) -> Self {
        let config_digest = ctx.config_digest;
        let rule_digest = ctx.rule_digest;
        let parallel = ctx.parallel;
        let upstream_digests = ctx.upstream_digests;
        let db = crate::analysis_api::FactDatabase::as_any_mut(ctx.facts)
            .downcast_mut::<AnalysisDb>()
            .expect("facade AnalysisDb host");
        let (cache, loaded, input_snapshot, plan, capability_support, scc_closure) =
            host::with_provider_host_session_mut(|session| {
                (
                    session.cache.clone(),
                    session.loaded.clone(),
                    session.input_snapshot.clone(),
                    session.plan.clone(),
                    session.capability_support.clone(),
                    session.scc_closure.take(),
                )
            });
        Self {
            db,
            cache,
            loaded,
            input_snapshot,
            config_digest,
            rule_digest,
            plan,
            parallel,
            upstream_digests,
            capability_support,
            scc_closure,
        }
    }

    fn dependency_digest(&self, provider_id: &'static str) -> Digest {
        self.upstream_digests
            .get(provider_id)
            .cloned()
            .unwrap_or_else(|| Digest::absent(DigestKind::ProviderOutput, provider_id))
    }
}

impl Drop for CtxHandle<'_> {
    fn drop(&mut self) {
        host::with_provider_host_session_mut(|session| {
            session.capability_support = self.capability_support.clone();
            session.scc_closure = self.scc_closure.take();
        });
    }
}

fn manifest_by_id(id: &'static str) -> &'static ProviderManifest {
    provider_manifests()
        .iter()
        .find(|manifest| manifest.id == id)
        .unwrap_or_else(|| panic!("missing provider manifest {id}"))
}

pub(crate) struct SourceProvider;

impl Provider for SourceProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.source")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        // Source discovery has no provider-output dependencies; the map is empty here.
        debug_assert!(ctx.upstream_digests.is_empty());
        // Source files are already loaded into the db before providers run;
        // this stage only records the discovery provider output metadata.
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: Vec::new(),
            cache_stats: CacheStats::default(),
            output_digest: None,
            execution: Default::default(),
        }
    }
}

pub(crate) struct GoSyntaxProvider;

impl Provider for GoSyntaxProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.go.syntax")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        run_registered_frontend_syntax(ctx, "go")
    }
}

pub(crate) struct TsSyntaxProvider;

impl Provider for TsSyntaxProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.ts.syntax")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        run_registered_frontend_syntax(ctx, "ts")
    }
}

fn run_registered_frontend_syntax(
    ctx: &mut ProviderCtx<'_>,
    frontend_name: &'static str,
) -> ProviderRunResult {
    let registry = crate::frontend::frontend_registry();
    let frontend = registry
        .by_name(frontend_name)
        .unwrap_or_else(|| panic!("missing language frontend {frontend_name}"));
    let frontend_id = frontend.id();
    if frontend.profile().name != frontend_name {
        panic!(
            "frontend profile name {} != requested {frontend_name}",
            frontend.profile().name
        );
    }
    let _ = (
        frontend.profile().family,
        frontend.profile().produces,
        frontend.profile().precision_ceiling,
        frontend_id.to_public_language(),
    );
    // Clone matching files so AnalysisUnit does not borrow the fact DB across `analyze`.
    let db = crate::analysis_api::FactDatabase::as_any_mut(ctx.facts)
        .downcast_mut::<AnalysisDb>()
        .expect("facade AnalysisDb host");
    let owned: Vec<crate::core::SourceFile> = db
        .files()
        .iter()
        .filter(|file| file.language.id() == frontend_id)
        .cloned()
        .collect();
    debug_assert!(
        owned.iter().all(|file| registry
            .scheduled_for(&file.path)
            .iter()
            .any(|candidate| candidate.id() == frontend_id)),
        "Language::id selection must agree with scheduled_for"
    );
    let files: Vec<&crate::core::SourceFile> = owned.iter().collect();
    let root = host::with_provider_host_session_mut(|session| session.loaded.root.clone());
    let unit = crate::frontend::AnalysisUnit {
        files: &files,
        root: &root,
    };
    frontend.analyze(ctx, &unit)
}

pub(crate) struct ModuleGraphProvider;

impl Provider for ModuleGraphProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.module_graph")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let mut ctx = CtxHandle::from_ctx(ctx);
        let derivation = crate::module_graph::derive_requested_module_graph_with_cache_stats(
            ctx.db,
            &ctx.loaded,
            &ctx.plan,
            &ctx.cache,
            &ctx.input_snapshot,
            self.manifest(),
            vec![
                ctx.dependency_digest("polint.go.syntax"),
                ctx.dependency_digest("polint.ts.syntax"),
            ],
        );
        let updated = derivation.support_view(&ctx.capability_support);
        ctx.capability_support = updated;
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: ProviderExecution::Succeeded,
        }
    }
}

pub(crate) struct SymbolGraphProvider;

impl Provider for SymbolGraphProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.symbol_graph")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let mut ctx = CtxHandle::from_ctx(ctx);
        let derivation = crate::symbol_graph::derive_requested_symbols_with_cache_stats(
            ctx.db,
            &ctx.loaded,
            &ctx.plan,
            &ctx.cache,
            &ctx.input_snapshot,
            self.manifest(),
            ctx.dependency_digest("polint.module_graph"),
            vec![
                ctx.dependency_digest("polint.go.syntax"),
                ctx.dependency_digest("polint.ts.syntax"),
            ],
        );
        let updated = derivation.support_view(&ctx.capability_support);
        ctx.capability_support = updated;
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: ProviderExecution::Succeeded,
        }
    }
}

pub(crate) struct ModuleTopologyProvider;

impl Provider for ModuleTopologyProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.module_topology")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let derivation = crate::module_graph::derive_module_topology_with_cache_stats(
            ctx.db,
            &ctx.cache,
            &ctx.input_snapshot,
            self.manifest(),
            ctx.dependency_digest("polint.module_graph"),
            ctx.dependency_digest("polint.symbol_graph"),
        );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: ProviderExecution::Succeeded,
        }
    }
}

pub(crate) struct SemanticMirProvider;

impl Provider for SemanticMirProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.semantic_mir")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let derivation = crate::analysis::provider::derive_semantic_mir_with_cache_stats(
            ctx.db,
            &ctx.input_snapshot,
            self.manifest(),
            ctx.dependency_digest("polint.module_topology"),
            ctx.dependency_digest("polint.symbol_graph"),
            vec![
                ctx.dependency_digest("polint.go.syntax"),
                ctx.dependency_digest("polint.ts.syntax"),
            ],
        );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
        }
    }
}

pub(crate) struct CfgProvider;

impl Provider for CfgProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.cfg")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let derivation = crate::analysis::cfg::provider::derive_cfg_with_cache_stats(
            ctx.db,
            &ctx.input_snapshot,
            self.manifest(),
            ctx.dependency_digest("polint.semantic_mir"),
            vec![
                ctx.dependency_digest("polint.go.syntax"),
                ctx.dependency_digest("polint.ts.syntax"),
            ],
        );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
        }
    }
}

pub(crate) struct CallsProvider;

impl Provider for CallsProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.calls")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        // Capability-closure only schedules this provider when the deep stack runs;
        // SEMANTIC/CFG/FULL triggers are identical, so the call-sites-only path is gone.
        let derivation = crate::analysis::calls::provider::derive_calls_with_cache_stats(
            ctx.db,
            &ctx.input_snapshot,
            self.manifest(),
            ctx.dependency_digest("polint.semantic_mir"),
            ctx.dependency_digest("polint.cfg"),
            ctx.dependency_digest("polint.symbol_graph"),
            ctx.dependency_digest("polint.module_topology"),
            vec![
                ctx.dependency_digest("polint.go.syntax"),
                ctx.dependency_digest("polint.ts.syntax"),
            ],
        );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
        }
    }
}

pub(crate) struct GoSemanticProvider;

impl Provider for GoSemanticProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.go.semantic")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let root = ctx.loaded.root.clone();
        let go_settings = ctx.loaded.config.languages.go.clone();
        let config_digest = ctx.config_digest;
        let go_syntax_digest = ctx.dependency_digest("polint.go.syntax");
        let sidecar_cache_dir = ctx.cache.sidecar_cache_dir();
        // The schedule may already have started this sidecar; it is handed over
        // here and to nowhere else.
        let prefetch = crate::analysis_kernel::host::with_provider_host_session_mut(|session| {
            session.go_semantic_prefetch.take()
        });
        let derivation = crate::go::semantic::provider::derive_go_semantic_with_cache_stats(
            ctx.db,
            &root,
            &go_settings,
            config_digest,
            self.manifest(),
            go_syntax_digest,
            crate::go::semantic::provider::GoSemanticSidecarAccess {
                cache_dir: sidecar_cache_dir.as_deref(),
                prefetch,
            },
        );
        ProviderRunResult {
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
            counts: derivation.counts,
        }
    }
}

pub(crate) struct TsTypesProvider;

impl Provider for TsTypesProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.ts.types")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let root = ctx.loaded.root.clone();
        let ts_settings = ctx.loaded.config.languages.ts.clone();
        let config_digest = ctx.config_digest;
        let ts_syntax_digest = ctx.dependency_digest("polint.ts.syntax");
        let sidecar_cache_dir = ctx.cache.sidecar_cache_dir();
        let derivation = crate::analysis::ts_types::provider::derive_ts_types(
            ctx.db,
            &root,
            &ts_settings,
            config_digest,
            self.manifest(),
            ts_syntax_digest,
            crate::analysis::ts_types::provider::TsTypesSidecarAccess {
                cache_dir: sidecar_cache_dir.as_deref(),
            },
        );
        ProviderRunResult {
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
            counts: derivation.counts,
        }
    }
}

pub(crate) struct IdentityProvider;

impl Provider for IdentityProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.identity")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let derivation = crate::analysis::identity::provider::derive_identity_with_cache_stats(
            ctx.db,
            &ctx.input_snapshot,
            self.manifest(),
            ctx.dependency_digest("polint.calls"),
            ctx.dependency_digest("polint.go.semantic"),
        );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
        }
    }
}

pub(crate) struct AbstractDomainsProvider;

impl Provider for AbstractDomainsProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.abstract_domains")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let compact_domain_materialization = ctx.plan.rules().iter().any(|rule| {
            rule.requested_capabilities
                .iter()
                .any(|c| c == "control_flow")
        }) && !ctx.plan.rules().iter().any(|rule| {
            rule.requested_capabilities
                .iter()
                .any(|c| c == "calls" || c == "dataflow")
        });
        let derivation = if compact_domain_materialization {
            crate::analysis::domains::provider::derive_summary_input_abstract_domains_with_cache_stats(ctx.db,
                &ctx.input_snapshot,
                self.manifest(),
                ctx.dependency_digest("polint.semantic_mir"),
                ctx.dependency_digest("polint.cfg"),
                ctx.dependency_digest("polint.calls"),
                ctx.dependency_digest("polint.symbol_graph"),
                ctx.dependency_digest("polint.module_topology"),
                vec![
                    ctx.dependency_digest("polint.go.syntax"),
                    ctx.dependency_digest("polint.ts.syntax"),
                ],
            )
        } else {
            crate::analysis::domains::provider::derive_abstract_domains_with_cache_stats(
                ctx.db,
                &ctx.input_snapshot,
                self.manifest(),
                ctx.dependency_digest("polint.semantic_mir"),
                ctx.dependency_digest("polint.cfg"),
                ctx.dependency_digest("polint.calls"),
                ctx.dependency_digest("polint.symbol_graph"),
                ctx.dependency_digest("polint.module_topology"),
                vec![
                    ctx.dependency_digest("polint.go.syntax"),
                    ctx.dependency_digest("polint.ts.syntax"),
                ],
            )
        };
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: ProviderExecution::Succeeded,
        }
    }
}

pub(crate) struct DirectSummariesProvider;

impl Provider for DirectSummariesProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.direct_summaries")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let mut ctx = CtxHandle::from_ctx(ctx);
        let derivation =
            crate::analysis::summaries::provider::derive_direct_summaries_with_cache_stats(
                ctx.db,
                &ctx.input_snapshot,
                self.manifest(),
                ctx.dependency_digest("polint.semantic_mir"),
                ctx.dependency_digest("polint.cfg"),
                ctx.dependency_digest("polint.calls"),
                ctx.dependency_digest("polint.abstract_domains"),
                ctx.dependency_digest("polint.symbol_graph"),
                ctx.dependency_digest("polint.module_topology"),
                vec![
                    ctx.dependency_digest("polint.go.syntax"),
                    ctx.dependency_digest("polint.ts.syntax"),
                ],
            );
        debug_assert!(derivation.output_digest.is_some());
        let mut diagnostics = derivation.diagnostics;
        let cache_stats = derivation.cache_stats;

        let scc_closure = crate::analysis::summaries::provider::run_scc_closure_with_cache(
            ctx.db,
            &ctx.cache,
            ctx.config_digest,
            ctx.rule_digest,
            ctx.plan.digest(),
        );
        diagnostics.extend(scc_closure.diagnostics.clone());
        ctx.scc_closure = Some(scc_closure);

        let final_direct_summaries_output = crate::analysis::summaries::store::SummaryOutput {
            summaries: ctx.db.summary_facts().to_vec(),
            events: ctx.db.summary_events().to_vec(),
        };
        let go_ts = [
            ctx.dependency_digest("polint.go.syntax"),
            ctx.dependency_digest("polint.ts.syntax"),
        ];
        let interner = ctx.db.stable_key_interner();
        let output_digest = crate::analysis::summaries::provider::direct_summaries_output_digest(
            self.manifest(),
            &ctx.input_snapshot,
            &ctx.dependency_digest("polint.semantic_mir"),
            &ctx.dependency_digest("polint.cfg"),
            &ctx.dependency_digest("polint.calls"),
            &ctx.dependency_digest("polint.abstract_domains"),
            &ctx.dependency_digest("polint.symbol_graph"),
            &ctx.dependency_digest("polint.module_topology"),
            &go_ts,
            &interner,
            &crate::analysis::summaries::provider::callable_stable_key_map(ctx.db),
            &final_direct_summaries_output,
        );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics,
            cache_stats,
            output_digest: Some(output_digest),
            execution: derivation.execution,
        }
    }
}

pub(crate) struct EntrypointsProvider;

impl Provider for EntrypointsProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.entrypoints")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let derivation =
            crate::analysis::entrypoints::provider::derive_entrypoints_with_cache_stats(
                ctx.db,
                &ctx.input_snapshot,
                self.manifest(),
                ctx.dependency_digest("polint.semantic_mir"),
                ctx.dependency_digest("polint.cfg"),
                ctx.dependency_digest("polint.calls"),
                ctx.dependency_digest("polint.symbol_graph"),
                ctx.dependency_digest("polint.module_topology"),
                vec![
                    ctx.dependency_digest("polint.go.syntax"),
                    ctx.dependency_digest("polint.ts.syntax"),
                ],
            );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
        }
    }
}

pub(crate) struct ReachabilityProvider;

impl Provider for ReachabilityProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.reachability")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let derivation =
            crate::analysis::reachability::provider::derive_reachability_with_cache_stats(
                ctx.db,
                &ctx.input_snapshot,
                self.manifest(),
                &ctx.loaded.config.reachability.roots,
                ctx.dependency_digest("polint.calls"),
                ctx.dependency_digest("polint.entrypoints"),
                ctx.dependency_digest("polint.identity"),
                ctx.dependency_digest("polint.symbol_graph"),
                ctx.dependency_digest("polint.module_topology"),
            );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
        }
    }
}

pub(crate) struct ExtensionsProvider;

impl Provider for ExtensionsProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.extensions")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let derivation = crate::analysis::extensions::provider::derive_extension_provider_outputs_with_cache_stats(
            ctx.db,
            &ctx.loaded.root,
            &ctx.input_snapshot,
            self.manifest(),
        );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
        }
    }
}

fn solver_budget_from_loaded(
    loaded: &LoadedConfig,
) -> crate::analysis::solver::budget::SolverBudget {
    crate::analysis::solver::budget::SolverBudget {
        go: loaded.config.solver.to_go_sub_budget(),
        ..crate::analysis::solver::budget::SolverBudget::default()
    }
}

pub(crate) struct TypeValueAliasProvider;

impl Provider for TypeValueAliasProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.type_value_alias")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let derivation = crate::analysis::types::provider::derive_type_value_alias_with_cache_stats(
            ctx.db,
            &ctx.input_snapshot,
            self.manifest(),
            ctx.dependency_digest("polint.semantic_mir"),
            ctx.dependency_digest("polint.cfg"),
            ctx.dependency_digest("polint.calls"),
            ctx.dependency_digest("polint.abstract_domains"),
            ctx.dependency_digest("polint.direct_summaries"),
            ctx.dependency_digest("polint.entrypoints"),
            ctx.dependency_digest("polint.extensions"),
            ctx.dependency_digest("polint.symbol_graph"),
            ctx.dependency_digest("polint.module_topology"),
            vec![
                ctx.dependency_digest("polint.go.syntax"),
                ctx.dependency_digest("polint.ts.syntax"),
            ],
        );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
        }
    }
}

pub(crate) struct SemanticGraphProvider;

impl Provider for SemanticGraphProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.semantic_graph")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let solver_budget = solver_budget_from_loaded(&ctx.loaded);
        let derivation =
            crate::analysis::semantic_graph::provider::derive_semantic_graph_with_cache_stats(
                ctx.db,
                &ctx.loaded,
                solver_budget.adaptation,
                &ctx.input_snapshot,
                self.manifest(),
                ctx.dependency_digest("polint.calls"),
                ctx.dependency_digest("polint.identity"),
                ctx.dependency_digest("polint.abstract_domains"),
                ctx.dependency_digest("polint.entrypoints"),
                ctx.dependency_digest("polint.reachability"),
                ctx.dependency_digest("polint.type_value_alias"),
                ctx.dependency_digest("polint.symbol_graph"),
                ctx.dependency_digest("polint.module_topology"),
                ctx.dependency_digest("polint.go.syntax"),
                ctx.dependency_digest("polint.ts.syntax"),
                ctx.dependency_digest("polint.semantic_mir"),
                ctx.dependency_digest("polint.go.semantic"),
            );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
        }
    }
}

pub(crate) struct SolverProvider;

impl Provider for SolverProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.solver")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let derivation = crate::analysis::solver::provider::derive_solver_with_cache_stats(
            ctx.db,
            &ctx.input_snapshot,
            self.manifest(),
            solver_budget_from_loaded(&ctx.loaded),
            ctx.dependency_digest("polint.semantic_graph"),
            ctx.dependency_digest("polint.type_value_alias"),
            ctx.dependency_digest("polint.go.semantic"),
        );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
        }
    }
}

pub(crate) struct RefinedCallsProvider;

impl Provider for RefinedCallsProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.refined_calls")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let derivation =
            crate::analysis::refined_calls::provider::derive_refined_calls_with_cache_stats(
                ctx.db,
                &ctx.input_snapshot,
                self.manifest(),
                ctx.dependency_digest("polint.calls"),
                ctx.dependency_digest("polint.entrypoints"),
                ctx.dependency_digest("polint.direct_summaries"),
                ctx.dependency_digest("polint.type_value_alias"),
                ctx.dependency_digest("polint.extensions"),
                ctx.dependency_digest("polint.solver"),
            );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
        }
    }
}

pub(crate) struct DataFlowProvider;

impl Provider for DataFlowProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.data_flow")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let derivation = crate::analysis::data_flow::provider::derive_data_flow_with_cache_stats(
            ctx.db,
            &ctx.input_snapshot,
            self.manifest(),
            ctx.dependency_digest("polint.semantic_mir"),
            ctx.dependency_digest("polint.cfg"),
            ctx.dependency_digest("polint.calls"),
            ctx.dependency_digest("polint.refined_calls"),
            ctx.dependency_digest("polint.direct_summaries"),
            ctx.dependency_digest("polint.type_value_alias"),
            ctx.dependency_digest("polint.entrypoints"),
            ctx.dependency_digest("polint.extensions"),
        );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
        }
    }
}

pub(crate) struct EvidenceProvider;

impl Provider for EvidenceProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.evidence")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let derivation = crate::analysis::evidence::provider::derive_evidence_with_cache_stats(
            ctx.db,
            &ctx.input_snapshot,
            self.manifest(),
            ctx.dependency_digest("polint.semantic_mir"),
            ctx.dependency_digest("polint.cfg"),
            ctx.dependency_digest("polint.calls"),
            ctx.dependency_digest("polint.refined_calls"),
            ctx.dependency_digest("polint.direct_summaries"),
            ctx.dependency_digest("polint.type_value_alias"),
            ctx.dependency_digest("polint.entrypoints"),
            ctx.dependency_digest("polint.extensions"),
            ctx.dependency_digest("polint.data_flow"),
        );
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
        }
    }
}

pub(crate) struct MetricsProvider;

impl Provider for MetricsProvider {
    fn manifest(&self) -> &'static ProviderManifest {
        manifest_by_id("polint.metrics")
    }

    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
        let ctx = CtxHandle::from_ctx(ctx);
        let derivation = match crate::metrics::derive_requested_metrics_with_cache_stats(
            ctx.db,
            &ctx.plan,
            &ctx.cache,
            self.manifest(),
        ) {
            Ok(derivation) => derivation,
            Err(error) => {
                return ProviderRunResult {
                    counts: Default::default(),
                    diagnostics: vec![Diagnostic::error(
                        "internal/metrics",
                        "<workspace>",
                        TextRange::point(1, 1),
                        format!("metrics projection failed: {error}"),
                    )],
                    cache_stats: CacheStats::default(),
                    output_digest: None,
                    execution: crate::analysis_api::ProviderExecution::Failed {
                        stage: crate::analysis_api::ProviderFailureStage::Execution,
                        reason: crate::analysis_api::ProviderFailureReason::ExecutionFailed,
                    },
                };
            }
        };
        ProviderRunResult {
            counts: Default::default(),
            diagnostics: derivation.diagnostics,
            cache_stats: derivation.cache_stats,
            output_digest: derivation.output_digest,
            execution: derivation.execution,
        }
    }
}

/// Topological schedule of `PROVIDER_MANIFESTS` with ties broken by declaration index.
///
/// Edge: provider P depends on Q when any string in `P.inputs` appears in `Q.outputs`.
/// External inputs with no producer (e.g. `workspace_config`) create no edges.
///
/// Order is asserted against the declared manifest sequence; the kernel switches
/// execution onto this schedule once capability-closure gating replaces the
/// hand-rolled boolean pipeline flags.
pub(crate) fn scheduled_order() -> Vec<&'static str> {
    use std::cmp::Reverse;
    use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

    let manifests = provider_manifests();
    let n = manifests.len();

    let mut producers: BTreeMap<&'static str, Vec<usize>> = BTreeMap::new();
    for (index, manifest) in manifests.iter().enumerate() {
        for &output in manifest.outputs {
            producers.entry(output).or_default().push(index);
        }
    }

    let mut indegree = vec![0usize; n];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (index, manifest) in manifests.iter().enumerate() {
        let mut preds = BTreeSet::new();
        for &input in manifest.inputs {
            if let Some(qs) = producers.get(input) {
                for &q in qs {
                    if q != index {
                        preds.insert(q);
                    }
                }
            }
        }
        indegree[index] = preds.len();
        for &q in &preds {
            dependents[q].push(index);
        }
    }

    // Tie-break by manifest declaration index (not id/name).
    let mut ready: BinaryHeap<Reverse<usize>> = BinaryHeap::new();
    for (index, &degree) in indegree.iter().enumerate() {
        if degree == 0 {
            ready.push(Reverse(index));
        }
    }

    let mut order = Vec::with_capacity(n);
    while let Some(Reverse(index)) = ready.pop() {
        order.push(manifests[index].id);
        for &dependent in &dependents[index] {
            indegree[dependent] -= 1;
            if indegree[dependent] == 0 {
                ready.push(Reverse(dependent));
            }
        }
    }
    assert_eq!(
        order.len(),
        n,
        "provider DAG must be acyclic; scheduled {} of {n}",
        order.len()
    );
    order
}

pub(crate) fn run_named_provider(id: &str, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult {
    match id {
        "polint.source" => SourceProvider.run(ctx),
        "polint.go.syntax" => GoSyntaxProvider.run(ctx),
        "polint.ts.syntax" => TsSyntaxProvider.run(ctx),
        "polint.module_graph" => ModuleGraphProvider.run(ctx),
        "polint.symbol_graph" => SymbolGraphProvider.run(ctx),
        "polint.module_topology" => ModuleTopologyProvider.run(ctx),
        "polint.semantic_mir" => SemanticMirProvider.run(ctx),
        "polint.cfg" => CfgProvider.run(ctx),
        "polint.calls" => CallsProvider.run(ctx),
        "polint.go.semantic" => GoSemanticProvider.run(ctx),
        "polint.ts.types" => TsTypesProvider.run(ctx),
        "polint.identity" => IdentityProvider.run(ctx),
        "polint.abstract_domains" => AbstractDomainsProvider.run(ctx),
        "polint.direct_summaries" => DirectSummariesProvider.run(ctx),
        "polint.entrypoints" => EntrypointsProvider.run(ctx),
        "polint.reachability" => ReachabilityProvider.run(ctx),
        "polint.extensions" => ExtensionsProvider.run(ctx),
        "polint.type_value_alias" => TypeValueAliasProvider.run(ctx),
        "polint.semantic_graph" => SemanticGraphProvider.run(ctx),
        "polint.solver" => SolverProvider.run(ctx),
        "polint.refined_calls" => RefinedCallsProvider.run(ctx),
        "polint.data_flow" => DataFlowProvider.run(ctx),
        "polint.evidence" => EvidenceProvider.run(ctx),
        "polint.metrics" => MetricsProvider.run(ctx),
        other => panic!("unknown provider id {other}"),
    }
}

/// Providers always present in today's `run()` schedule (graphs + metrics).
const BASELINE_PROVIDER_SEEDS: &[&str] = &[
    "polint.module_graph",
    "polint.symbol_graph",
    "polint.metrics",
];

/// Seeds that pull the deep semantic/CFG/refinement stack through dependency closure.
const DEEP_PROVIDER_SEEDS: &[&str] = &[
    "polint.calls",
    "polint.direct_summaries",
    "polint.reachability",
    "polint.refined_calls",
    "polint.solver",
];

const DEEP_TRIGGER_CAPABILITIES: &[&str] = &["calls", "control_flow", "dataflow"];

/// Provider set enabled by the historical boolean pipeline gates for `requested`.
pub(crate) fn providers_enabled_by_boolean_gates(
    requested: &std::collections::BTreeSet<&str>,
) -> std::collections::BTreeSet<&'static str> {
    let mut enabled = std::collections::BTreeSet::from([
        "polint.source",
        "polint.go.syntax",
        "polint.ts.syntax",
        "polint.module_graph",
        "polint.symbol_graph",
        "polint.metrics",
    ]);
    let deep = DEEP_TRIGGER_CAPABILITIES
        .iter()
        .any(|capability| requested.contains(capability));
    if deep {
        enabled.extend([
            "polint.module_topology",
            "polint.semantic_mir",
            "polint.cfg",
            "polint.calls",
            "polint.go.semantic",
            "polint.ts.types",
            "polint.identity",
            "polint.abstract_domains",
            "polint.direct_summaries",
            "polint.entrypoints",
            "polint.reachability",
            "polint.extensions",
            "polint.type_value_alias",
            "polint.semantic_graph",
            "polint.solver",
            "polint.refined_calls",
        ]);
    }
    if requested.contains("dataflow") {
        enabled.extend(["polint.data_flow", "polint.evidence"]);
    }
    enabled
}

fn seed_providers_for_capability(capability: &str) -> &'static [&'static str] {
    match capability {
        "resolved_imports" | "module_graph" => &["polint.module_graph"],
        "symbols" | "references" => &["polint.symbol_graph"],
        "calls" | "control_flow" => DEEP_PROVIDER_SEEDS,
        "dataflow" => {
            const DATAFLOW_SEEDS: &[&str] = &[
                "polint.calls",
                "polint.direct_summaries",
                "polint.reachability",
                "polint.refined_calls",
                "polint.solver",
                "polint.data_flow",
                "polint.evidence",
            ];
            DATAFLOW_SEEDS
        }
        "file_metrics" | "function_metrics" | "complexity_metrics" => &["polint.metrics"],
        _ => &[],
    }
}

/// Provider set derived by seeding from requested capabilities and closing over
/// manifest dependency edges (input fact produced by provider).
pub(crate) fn providers_enabled_by_capability_closure(
    requested: &std::collections::BTreeSet<&str>,
) -> std::collections::BTreeSet<&'static str> {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};

    let manifests = provider_manifests();
    let mut producers: BTreeMap<&str, Vec<&'static str>> = BTreeMap::new();
    for manifest in manifests {
        for &output in manifest.outputs {
            producers.entry(output).or_default().push(manifest.id);
        }
    }

    let mut seeds: BTreeSet<&'static str> = BASELINE_PROVIDER_SEEDS.iter().copied().collect();
    for &capability in requested {
        seeds.extend(seed_providers_for_capability(capability).iter().copied());
    }

    let mut enabled = seeds.clone();
    let mut queue: VecDeque<&'static str> = seeds.into_iter().collect();
    while let Some(provider_id) = queue.pop_front() {
        let Some(manifest) = manifests.iter().find(|manifest| manifest.id == provider_id) else {
            continue;
        };
        for &input in manifest.inputs {
            let Some(producers_for_input) = producers.get(input) else {
                continue;
            };
            for &producer in producers_for_input {
                if enabled.insert(producer) {
                    queue.push_back(producer);
                }
            }
        }
    }
    enabled
}

/// Topological provider order restricted to the capability-closure set for `requested`.
pub(crate) fn scheduled_order_for(
    requested: &std::collections::BTreeSet<&str>,
) -> Vec<&'static str> {
    let enabled = providers_enabled_by_capability_closure(requested);
    scheduled_order()
        .into_iter()
        .filter(|id| enabled.contains(id))
        .collect()
}

pub(crate) fn provider_manifests() -> &'static [ProviderManifest] {
    PROVIDER_MANIFESTS
}

#[cfg(test)]
pub(crate) fn provider_order_for_test() -> Vec<&'static str> {
    provider_manifests()
        .iter()
        .map(|manifest| manifest.id)
        .collect()
}

#[cfg(test)]
pub(crate) fn provider_order_report_for_test() -> Vec<ProviderOrderRow> {
    provider_manifests()
        .iter()
        .map(|manifest| ProviderOrderRow {
            id: manifest.id,
            kind: provider_kind_label(manifest.kind),
            language_scope: manifest.language_scope_label(),
            inputs: manifest.inputs.to_vec(),
            outputs: manifest.outputs.to_vec(),
        })
        .collect()
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderOrderRow {
    pub(crate) id: &'static str,
    pub(crate) kind: &'static str,
    pub(crate) language_scope: &'static str,
    pub(crate) inputs: Vec<&'static str>,
    pub(crate) outputs: Vec<&'static str>,
}

#[cfg(test)]
fn provider_kind_label(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::SourceDiscovery => "source_discovery",
        ProviderKind::LanguageSyntax => "language_syntax",
        ProviderKind::WholeRepoDerived => "whole_repo_derived",
        ProviderKind::MetricsDerived => "metrics_derived",
    }
}

const SOURCE_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: "source-files-1",
    version: 1,
}];

const GO_SYNTAX_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: "go-facts-v2",
    version: 2,
}];

const TS_SYNTAX_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: "ts-facts-v5",
    version: 5,
}];

const MODULE_GRAPH_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: "module-graph-facts-2",
    version: 2,
}];

const SYMBOL_GRAPH_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: "symbol-graph-facts-2",
    version: 2,
}];

const MODULE_TOPOLOGY_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: "module-topology-facts-1",
    version: 1,
}];

const SEMANTIC_MIR_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: "semantic-mir-facts-1",
    version: 1,
}];

const CFG_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: "cfg-facts-1",
    version: 1,
}];

const CALLS_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: "calls-facts-1",
    version: 1,
}];

const GO_SEMANTIC_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: crate::go::semantic::cache_key::GO_SEMANTIC_SCHEMA_LABEL,
    version: 1,
}];

const TS_TYPES_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: crate::ts::types::cache_key::TS_TYPES_SCHEMA_LABEL,
    version: 1,
}];

const IDENTITY_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: crate::analysis::identity::cache_key::IDENTITY_SCHEMA_LABEL,
    version: 1,
}];

const ABSTRACT_DOMAINS_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: "abstract-domain-facts-1",
    version: 1,
}];

const DIRECT_SUMMARIES_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: crate::analysis::summaries::cache_key::DIRECT_SUMMARIES_SCHEMA_LABEL,
    version: 1,
}];

const ENTRYPOINTS_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: "entrypoints-facts-1",
    version: 1,
}];

const REACHABILITY_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: crate::analysis::reachability::cache_key::REACHABILITY_SCHEMA_LABEL,
    version: 1,
}];

const TYPE_VALUE_ALIAS_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: crate::analysis::types::cache_key::TYPE_VALUE_ALIAS_SCHEMA_LABEL,
    version: 1,
}];

const SEMANTIC_GRAPH_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: crate::analysis::semantic_graph::cache_key::SEMANTIC_GRAPH_SCHEMA_LABEL,
    version: 1,
}];

const SOLVER_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: crate::analysis::solver::cache_key::SOLVER_SCHEMA_LABEL,
    version: 1,
}];

const REFINED_CALLS_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: crate::analysis::refined_calls::cache_key::REFINED_CALLS_SCHEMA_LABEL,
    version: 1,
}];

const DATA_FLOW_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: crate::analysis::data_flow::cache_key::DATA_FLOW_SCHEMA_LABEL,
    version: 1,
}];

const EVIDENCE_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: crate::analysis::evidence::cache_key::EVIDENCE_SCHEMA_LABEL,
    version: 1,
}];

const EXTENSIONS_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: crate::analysis::extensions::EXTENSION_FACTS_SCHEMA_LABEL,
    version: 1,
}];

const METRICS_SCHEMA: &[SchemaVersion] = &[SchemaVersion {
    name: "metrics-facts-1",
    version: 1,
}];

const PROVIDER_MANIFESTS: &[ProviderManifest] = &[
    ProviderManifest {
        id: "polint.source",
        kind: ProviderKind::SourceDiscovery,
        inputs: &["workspace_config", "file_discovery"],
        outputs: &["source_files"],
        language_ids: crate::frontend::LANGUAGE_IDS_NONE,
        cache_policy: CachePolicy::NoCache,
        schema_versions: SOURCE_SCHEMA,
        precision_ceiling: PrecisionCeiling::Exact,
    },
    ProviderManifest {
        id: "polint.go.syntax",
        kind: ProviderKind::LanguageSyntax,
        inputs: &["source_files"],
        outputs: &[
            "packages",
            "functions",
            "imports",
            "go_tests",
            "branch_obligations",
            "string_literals",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_GO,
        cache_policy: CachePolicy::ExistingFileFactCache {
            schema: "go-facts-v2",
        },
        schema_versions: GO_SYNTAX_SCHEMA,
        precision_ceiling: PrecisionCeiling::Syntax,
    },
    ProviderManifest {
        id: "polint.ts.syntax",
        kind: ProviderKind::LanguageSyntax,
        inputs: &["source_files"],
        outputs: &[
            "functions",
            "imports",
            "ts_components",
            "ts_classes",
            "string_literals",
            "jsx_attributes",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_TS,
        cache_policy: CachePolicy::ExistingFileFactCache {
            schema: "ts-facts-v5",
        },
        schema_versions: TS_SYNTAX_SCHEMA,
        precision_ceiling: PrecisionCeiling::Syntax,
    },
    ProviderManifest {
        id: "polint.module_graph",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &["source_files", "packages", "imports"],
        outputs: &[
            "resolved_imports",
            "module_nodes",
            "module_edges",
            "workspace_roots",
            "topology_packages",
            "source_sets",
            "dependency_requirements",
            "resolved_dependency_edges",
            "repo_topology_overlays",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: MODULE_GRAPH_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.symbol_graph",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "packages",
            "imports",
            "resolved_imports",
            "module_nodes",
            "module_edges",
            "functions",
        ],
        outputs: &[
            "symbols",
            "definitions",
            "references",
            "scopes",
            "semantic_imports",
            "exports",
            "aliases",
            "resolution_facts",
            "generated_symbols",
            "stable_exports",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: SYMBOL_GRAPH_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.module_topology",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "imports",
            "resolved_imports",
            "module_nodes",
            "module_edges",
            "workspace_roots",
            "topology_packages",
            "source_sets",
            "dependency_requirements",
            "resolved_dependency_edges",
            "semantic_imports",
        ],
        outputs: &["import_to_package_edges"],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: MODULE_TOPOLOGY_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.semantic_mir",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "functions",
            "symbols",
            "references",
            "scopes",
            "semantic_imports",
            "import_to_package_edges",
        ],
        outputs: &[
            "mir_bodies",
            "mir_operations",
            "places",
            "unsupported_semantics",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: SEMANTIC_MIR_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.cfg",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "functions",
            "mir_bodies",
            "mir_operations",
            "places",
            "unsupported_semantics",
        ],
        outputs: &[
            "cfg_functions",
            "cfg_nodes",
            "basic_blocks",
            "cfg_edges",
            "cfg_reachability",
            "cfg_dominators",
            "cfg_postdominators",
            "cfg_control_dependence",
            "unsupported_control_flow",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: CFG_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.calls",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "functions",
            "symbols",
            "references",
            "semantic_imports",
            "unsupported_semantics",
            "resolved_imports",
            "import_to_package_edges",
            "mir_bodies",
            "mir_operations",
            "places",
            "cfg_functions",
            "cfg_edges",
        ],
        outputs: &["call_sites", "call_targets", "unresolved_calls"],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: CALLS_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.go.semantic",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "packages",
            "functions",
            "go.module_roots",
            "go.package_patterns",
            "go.build_tags",
            "go.include_tests",
            "go.offline",
        ],
        outputs: &[
            "go_semantic_packages",
            "go_semantic_functions",
            "go_semantic_callsites",
            "go_semantic_method_sets",
            "go_semantic_address_taken",
            "go_semantic_instantiated_types",
            "go_semantic_dynamic_dispatch",
            "go_semantic_rta_edges",
            "go_semantic_package_errors",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_GO,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: GO_SEMANTIC_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.ts.types",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "functions",
            "call_sites",
            "ts.type_sidecar",
            "ts.type_projects",
            "ts.typescript_path",
            "ts.type_timeout_ms",
        ],
        outputs: &[
            "ts_type_projects",
            "ts_type_callables",
            "ts_type_callsites",
            "ts_type_callees",
            "ts_type_receivers",
            "ts_type_file_densities",
            "ts_type_project_errors",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: TS_TYPES_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.identity",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "functions",
            "call_sites",
            "call_targets",
            "unresolved_calls",
            "go_semantic_packages",
        ],
        outputs: &["identity_records"],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: IDENTITY_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.abstract_domains",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "functions",
            "mir_bodies",
            "mir_operations",
            "places",
            "unsupported_semantics",
            "cfg_functions",
            "basic_blocks",
            "cfg_edges",
            "call_sites",
            "call_targets",
            "unresolved_calls",
        ],
        outputs: &["domain_observations", "domain_events"],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: ABSTRACT_DOMAINS_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.direct_summaries",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "functions",
            "mir_bodies",
            "mir_operations",
            "places",
            "unsupported_semantics",
            "cfg_functions",
            "basic_blocks",
            "cfg_edges",
            "call_sites",
            "call_targets",
            "unresolved_calls",
            "domain_observations",
            "domain_events",
        ],
        outputs: &[
            "summary_control",
            "summary_call",
            "summary_memory",
            "summary_tito",
            "summary_events",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: DIRECT_SUMMARIES_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.entrypoints",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "functions",
            "symbols",
            "references",
            "semantic_imports",
            "resolved_imports",
            "import_to_package_edges",
            "mir_bodies",
            "mir_operations",
            "places",
            "call_sites",
            "call_targets",
            "unresolved_calls",
        ],
        outputs: &[
            "entrypoints",
            "trust_boundaries",
            "dispatch_edges",
            "unresolved_framework",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: ENTRYPOINTS_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.reachability",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "functions",
            "symbols",
            "references",
            "call_sites",
            "call_targets",
            "unresolved_calls",
            "entrypoints",
            "identity_records",
            "exports",
        ],
        outputs: &["reachability_roots", "call_reachability"],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: REACHABILITY_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.extensions",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "functions",
            "symbols",
            "references",
            "entrypoints",
            "trust_boundaries",
            "dispatch_edges",
            "unresolved_framework",
            "extension.providers",
        ],
        outputs: &["extension_facts", "extension_rejections"],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: EXTENSIONS_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.type_value_alias",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "functions",
            "symbols",
            "references",
            "semantic_imports",
            "resolved_imports",
            "import_to_package_edges",
            "mir_bodies",
            "mir_operations",
            "places",
            "cfg_functions",
            "basic_blocks",
            "cfg_edges",
            "call_sites",
            "call_targets",
            "unresolved_calls",
            "domain_observations",
            "domain_events",
            "summary_control",
            "summary_call",
            "summary_memory",
            "summary_tito",
            "entrypoints",
            "trust_boundaries",
            "dispatch_edges",
            "unresolved_framework",
            "extension_facts",
            "extension_rejections",
        ],
        outputs: &[
            "type_facts",
            "narrowed_type_facts",
            "value_facts",
            "allocation_tokens",
            "access_paths",
            "points_to_constraints",
            "points_to_sets",
            "alias_answers",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: TYPE_VALUE_ALIAS_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.semantic_graph",
        kind: ProviderKind::WholeRepoDerived,
        // Fact families `build_semantic_graph` ACTUALLY reads today (kept in lockstep
        // with the projection so the declared read-set never overstates consumption):
        // source/import/module rows used by TS direct bindings, functions/packages
        // (syntax), scopes (symbol graph), call sites (calls), value facts
        // (type/value/alias), MIR places (semantic MIR), Go semantic rows, repo-local
        // adaptation model files, and the private TS object-model rows refreshed
        // inside the semantic-graph provider. The producer/current-row digest of
        // each is folded into the provider output digest in
        // `semantic_graph::provider::semantic_graph_output_digest` (D-17).
        //
        // SC3 inputs with NO producer yet are intentionally ABSENT until their
        // producer lands (not silently dropped): CFG / summaries. When
        // the projection begins reading a new family, add it here AND fold its
        // producer digest in the same change.
        inputs: &[
            "source_files",
            "functions",
            "packages",
            "imports",
            "resolved_imports",
            "module_nodes",
            "scopes",
            "call_sites",
            "value_facts",
            "places",
            "go_semantic_functions",
            "go_semantic_callsites",
            "ts_object_allocations",
            "ts_property_writes",
            "ts_property_reads",
            "ts_receiver_bindings",
            "ts_prototype_links",
            "adaptation_model_files",
            "adaptation_model_budget",
        ],
        outputs: &[
            "ts_object_allocations",
            "ts_property_writes",
            "ts_property_reads",
            "ts_receiver_bindings",
            "ts_prototype_links",
            "semantic_nodes",
            "semantic_edges",
            "semantic_constraints",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: SEMANTIC_GRAPH_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        // polint.solver runs AFTER polint.semantic_graph and BEFORE
        // polint.refined_calls (D-13). It consumes the unified
        // semantic-graph constraint vocabulary (`semantic_constraints`) and emits
        // derived edges with provenance. Its output digest folds the consumed
        // upstream provider digests (`polint.semantic_graph`, `polint.type_value_alias`,
        // and `polint.go.semantic`) plus the SolverBudget (D-15); those digest-only
        // dependencies are not declared here as direct fact reads. The provider
        // auto-enrolls in the determinism gate (D-14).
        id: "polint.solver",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "source_files",
            "functions",
            "call_sites",
            "imports",
            "resolved_imports",
            "module_nodes",
            "reachability_roots",
            "semantic_constraints",
            "semantic_nodes",
            "go_semantic_functions",
            "go_semantic_callsites",
            "go_semantic_method_sets",
            "go_semantic_address_taken",
            "go_semantic_instantiated_types",
            "go_semantic_dynamic_dispatch",
            "ts_object_allocations",
            "ts_property_writes",
            "ts_property_reads",
            "ts_receiver_bindings",
            "ts_prototype_links",
        ],
        outputs: &[
            "solver_derived_edges",
            "solver_budget_status",
            "solver_budget_reasons",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: SOLVER_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.refined_calls",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "call_sites",
            "call_targets",
            "unresolved_calls",
            "functions",
            "symbols",
            "entrypoints",
            "dispatch_edges",
            "summary_call",
            "summary_events",
            "type_facts",
            "value_facts",
            "allocation_tokens",
            "points_to_sets",
            "alias_answers",
            "extension_facts",
            "semantic_nodes",
            "semantic_constraints",
            "go_semantic_functions",
            "go_semantic_callsites",
            "ts_type_callsites",
            "ts_type_callees",
            "ts_type_receivers",
            "ts_type_file_densities",
            "solver_derived_edges",
        ],
        outputs: &["refined_call_edges"],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: REFINED_CALLS_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.data_flow",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "mir_bodies",
            "mir_operations",
            "places",
            "cfg_functions",
            "cfg_nodes",
            "cfg_edges",
            "call_sites",
            "call_targets",
            "refined_call_edges",
            "summary_tito",
            "summary_events",
            "type_facts",
            "value_facts",
            "access_paths",
            "points_to_sets",
            "alias_answers",
            "entrypoints",
            "trust_boundaries",
            "extension_facts",
        ],
        outputs: &[
            "data_flow_nodes",
            "data_flow_edges",
            "data_flow_models",
            "data_flow_budgets",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: DATA_FLOW_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.evidence",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &[
            "mir_bodies",
            "mir_operations",
            "places",
            "cfg_functions",
            "cfg_nodes",
            "cfg_edges",
            "cfg_control_dependence",
            "call_sites",
            "call_targets",
            "unresolved_calls",
            "refined_call_edges",
            "summary_tito",
            "summary_events",
            "type_facts",
            "value_facts",
            "access_paths",
            "points_to_sets",
            "alias_answers",
            "entrypoints",
            "trust_boundaries",
            "dispatch_edges",
            "extension_facts",
            "data_flow_nodes",
            "data_flow_edges",
            "data_flow_models",
            "data_flow_budgets",
        ],
        outputs: &[
            "evidence_nodes",
            "evidence_edges",
            "evidence_bundles",
            "evidence_paths",
            "evidence_slices",
            "evidence_unknowns",
            "evidence_omitted_regions",
            "evidence_replay_keys",
        ],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: EVIDENCE_SCHEMA,
        precision_ceiling: PrecisionCeiling::SetupAware,
    },
    ProviderManifest {
        id: "polint.metrics",
        kind: ProviderKind::MetricsDerived,
        inputs: &["source_files", "functions"],
        outputs: &["file_metrics", "function_metrics", "complexity_metrics"],
        language_ids: crate::frontend::LANGUAGE_IDS_GO_AND_TS,
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: METRICS_SCHEMA,
        precision_ceiling: PrecisionCeiling::Syntax,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    #[test]
    fn go_syntax_manifest_declares_all_owned_outputs() {
        let manifest = provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.go.syntax")
            .unwrap();
        assert_eq!(
            manifest.outputs,
            [
                "packages",
                "functions",
                "imports",
                "go_tests",
                "branch_obligations",
                "string_literals",
            ]
        );
        assert_eq!(
            manifest.language_ids,
            &[crate::internal_core::LanguageId::GO]
        );
        assert_eq!(
            manifest.cache_policy,
            CachePolicy::ExistingFileFactCache {
                schema: "go-facts-v2"
            }
        );
        assert_eq!(manifest.schema_versions, GO_SYNTAX_SCHEMA);
        assert_eq!(manifest.precision_ceiling, PrecisionCeiling::Syntax);
    }

    #[test]
    fn provider_manifests_have_required_metadata() {
        for manifest in provider_manifests() {
            assert!(!manifest.id.is_empty());
            assert!(
                !manifest.inputs.is_empty()
                    || matches!(manifest.kind, ProviderKind::SourceDiscovery)
            );
            assert!(!manifest.outputs.is_empty());
            assert!(!manifest.schema_versions.is_empty());
            let _language_ids = manifest.language_ids;
            let _cache_policy = manifest.cache_policy;
            let _precision_ceiling = manifest.precision_ceiling;
        }
    }

    #[test]
    fn topological_order_matches_declared_manifest_order() {
        let scheduled = scheduled_order();
        let declared: Vec<&str> = provider_manifests().iter().map(|m| m.id).collect();
        assert_eq!(
            scheduled, declared,
            "DAG topo (manifest-index tie-break) must reproduce PROVIDER_MANIFESTS order"
        );
    }

    #[test]
    fn capability_closure_matches_boolean_pipeline_gates() {
        let capabilities = [
            "resolved_imports",
            "module_graph",
            "symbols",
            "references",
            "calls",
            "control_flow",
            "dataflow",
        ];
        let n = capabilities.len();
        for mask in 0..(1usize << n) {
            let requested: BTreeSet<&str> = capabilities
                .iter()
                .enumerate()
                .filter(|(index, _)| mask & (1usize << index) != 0)
                .map(|(_, capability)| *capability)
                .collect();
            let via_booleans = providers_enabled_by_boolean_gates(&requested);
            let via_closure = providers_enabled_by_capability_closure(&requested);
            assert_eq!(via_closure, via_booleans, "{requested:?}");
            let scheduled = scheduled_order_for(&requested);
            let scheduled_set: BTreeSet<&str> = scheduled.iter().copied().collect();
            assert_eq!(scheduled_set, via_closure, "order filter {requested:?}");
        }
    }

    #[test]
    fn v13_cache_dependency_ledger_matches_provider_manifest_inputs() {
        for dependency in crate::analysis_neutral::cache_key::v13_cache_dependency_ledger() {
            let manifest = provider_manifests()
                .iter()
                .find(|manifest| manifest.id == dependency.provider_id)
                .unwrap_or_else(|| {
                    panic!(
                        "V13 cache dependency provider is not registered: {}",
                        dependency.provider_id
                    )
                });

            let manifest_inputs = manifest.inputs.iter().copied().collect::<BTreeSet<_>>();
            let ledger_inputs = dependency
                .manifest_inputs
                .iter()
                .copied()
                .collect::<BTreeSet<_>>();
            assert_eq!(
                ledger_inputs, manifest_inputs,
                "V13 cache dependency ledger drift for {}",
                manifest.id
            );
        }
    }

    #[test]
    fn provider_order_matches_behavior_preserving_kernel_sequence() {
        assert_eq!(
            provider_order_for_test(),
            vec![
                "polint.source",
                "polint.go.syntax",
                "polint.ts.syntax",
                "polint.module_graph",
                "polint.symbol_graph",
                "polint.module_topology",
                "polint.semantic_mir",
                "polint.cfg",
                "polint.calls",
                "polint.go.semantic",
                "polint.ts.types",
                "polint.identity",
                "polint.abstract_domains",
                "polint.direct_summaries",
                "polint.entrypoints",
                "polint.reachability",
                "polint.extensions",
                "polint.type_value_alias",
                "polint.semantic_graph",
                "polint.solver",
                "polint.refined_calls",
                "polint.data_flow",
                "polint.evidence",
                "polint.metrics",
            ]
        );
    }

    #[test]
    fn symbol_graph_manifest_declares_semantic_outputs_without_reordering_providers() {
        assert_eq!(
            provider_order_for_test(),
            vec![
                "polint.source",
                "polint.go.syntax",
                "polint.ts.syntax",
                "polint.module_graph",
                "polint.symbol_graph",
                "polint.module_topology",
                "polint.semantic_mir",
                "polint.cfg",
                "polint.calls",
                "polint.go.semantic",
                "polint.ts.types",
                "polint.identity",
                "polint.abstract_domains",
                "polint.direct_summaries",
                "polint.entrypoints",
                "polint.reachability",
                "polint.extensions",
                "polint.type_value_alias",
                "polint.semantic_graph",
                "polint.solver",
                "polint.refined_calls",
                "polint.data_flow",
                "polint.evidence",
                "polint.metrics",
            ]
        );
        let manifest = provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.symbol_graph")
            .expect("symbol graph manifest should exist");

        assert_eq!(
            manifest.outputs,
            &[
                "symbols",
                "definitions",
                "references",
                "scopes",
                "semantic_imports",
                "exports",
                "aliases",
                "resolution_facts",
                "generated_symbols",
                "stable_exports",
            ]
        );
        assert_eq!(
            manifest.schema_versions,
            &[SchemaVersion {
                name: "symbol-graph-facts-2",
                version: 2,
            }]
        );
    }

    #[test]
    fn module_graph_manifest_declares_base_topology_outputs_without_reordering_providers() {
        assert_eq!(
            provider_order_for_test(),
            vec![
                "polint.source",
                "polint.go.syntax",
                "polint.ts.syntax",
                "polint.module_graph",
                "polint.symbol_graph",
                "polint.module_topology",
                "polint.semantic_mir",
                "polint.cfg",
                "polint.calls",
                "polint.go.semantic",
                "polint.ts.types",
                "polint.identity",
                "polint.abstract_domains",
                "polint.direct_summaries",
                "polint.entrypoints",
                "polint.reachability",
                "polint.extensions",
                "polint.type_value_alias",
                "polint.semantic_graph",
                "polint.solver",
                "polint.refined_calls",
                "polint.data_flow",
                "polint.evidence",
                "polint.metrics",
            ]
        );
        let manifest = provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.module_graph")
            .expect("module graph manifest should exist");

        assert_eq!(
            manifest.outputs,
            &[
                "resolved_imports",
                "module_nodes",
                "module_edges",
                "workspace_roots",
                "topology_packages",
                "source_sets",
                "dependency_requirements",
                "resolved_dependency_edges",
                "repo_topology_overlays",
            ]
        );
        assert!(!manifest.outputs.contains(&"import_to_package_edges"));
        assert_eq!(
            manifest.schema_versions,
            &[SchemaVersion {
                name: "module-graph-facts-2",
                version: 2,
            }]
        );
    }

    #[test]
    fn topology_outputs_are_not_sdk_prelude_exports() {
        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let prelude = read_source(&crate_root.join("src/sdk/mod.rs"));

        for term in [
            ["Packages", "<'_"].concat(),
            ["Dependencies", "<'_"].concat(),
            ["SourceSets", "<'_"].concat(),
            ["RepoTopology", "<'_"].concat(),
        ] {
            assert!(
                !prelude.contains(&term),
                "unexpected topology SDK prelude export `{term}`"
            );
        }
    }

    #[test]
    fn provider_manifest_dependencies_are_deterministic_metadata() {
        let report = provider_order_report_for_test();

        assert_eq!(
            report,
            vec![
                ProviderOrderRow {
                    id: "polint.source",
                    kind: "source_discovery",
                    language_scope: "workspace",
                    inputs: vec!["workspace_config", "file_discovery"],
                    outputs: vec!["source_files"],
                },
                ProviderOrderRow {
                    id: "polint.go.syntax",
                    kind: "language_syntax",
                    language_scope: "go",
                    inputs: vec!["source_files"],
                    outputs: vec![
                        "packages",
                        "functions",
                        "imports",
                        "go_tests",
                        "branch_obligations",
                        "string_literals",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.ts.syntax",
                    kind: "language_syntax",
                    language_scope: "typescript_javascript",
                    inputs: vec!["source_files"],
                    outputs: vec![
                        "functions",
                        "imports",
                        "ts_components",
                        "ts_classes",
                        "string_literals",
                        "jsx_attributes",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.module_graph",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec!["source_files", "packages", "imports"],
                    outputs: vec![
                        "resolved_imports",
                        "module_nodes",
                        "module_edges",
                        "workspace_roots",
                        "topology_packages",
                        "source_sets",
                        "dependency_requirements",
                        "resolved_dependency_edges",
                        "repo_topology_overlays",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.symbol_graph",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "source_files",
                        "packages",
                        "imports",
                        "resolved_imports",
                        "module_nodes",
                        "module_edges",
                        "functions",
                    ],
                    outputs: vec![
                        "symbols",
                        "definitions",
                        "references",
                        "scopes",
                        "semantic_imports",
                        "exports",
                        "aliases",
                        "resolution_facts",
                        "generated_symbols",
                        "stable_exports",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.module_topology",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "source_files",
                        "imports",
                        "resolved_imports",
                        "module_nodes",
                        "module_edges",
                        "workspace_roots",
                        "topology_packages",
                        "source_sets",
                        "dependency_requirements",
                        "resolved_dependency_edges",
                        "semantic_imports",
                    ],
                    outputs: vec!["import_to_package_edges"],
                },
                ProviderOrderRow {
                    id: "polint.semantic_mir",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "source_files",
                        "functions",
                        "symbols",
                        "references",
                        "scopes",
                        "semantic_imports",
                        "import_to_package_edges",
                    ],
                    outputs: vec![
                        "mir_bodies",
                        "mir_operations",
                        "places",
                        "unsupported_semantics",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.cfg",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "source_files",
                        "functions",
                        "mir_bodies",
                        "mir_operations",
                        "places",
                        "unsupported_semantics",
                    ],
                    outputs: vec![
                        "cfg_functions",
                        "cfg_nodes",
                        "basic_blocks",
                        "cfg_edges",
                        "cfg_reachability",
                        "cfg_dominators",
                        "cfg_postdominators",
                        "cfg_control_dependence",
                        "unsupported_control_flow",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.calls",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "source_files",
                        "functions",
                        "symbols",
                        "references",
                        "semantic_imports",
                        "unsupported_semantics",
                        "resolved_imports",
                        "import_to_package_edges",
                        "mir_bodies",
                        "mir_operations",
                        "places",
                        "cfg_functions",
                        "cfg_edges",
                    ],
                    outputs: vec!["call_sites", "call_targets", "unresolved_calls"],
                },
                ProviderOrderRow {
                    id: "polint.go.semantic",
                    kind: "whole_repo_derived",
                    language_scope: "go",
                    inputs: vec![
                        "source_files",
                        "packages",
                        "functions",
                        "go.module_roots",
                        "go.package_patterns",
                        "go.build_tags",
                        "go.include_tests",
                        "go.offline",
                    ],
                    outputs: vec![
                        "go_semantic_packages",
                        "go_semantic_functions",
                        "go_semantic_callsites",
                        "go_semantic_method_sets",
                        "go_semantic_address_taken",
                        "go_semantic_instantiated_types",
                        "go_semantic_dynamic_dispatch",
                        "go_semantic_rta_edges",
                        "go_semantic_package_errors",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.ts.types",
                    kind: "whole_repo_derived",
                    language_scope: "typescript_javascript",
                    inputs: vec![
                        "source_files",
                        "functions",
                        "call_sites",
                        "ts.type_sidecar",
                        "ts.type_projects",
                        "ts.typescript_path",
                        "ts.type_timeout_ms",
                    ],
                    outputs: vec![
                        "ts_type_projects",
                        "ts_type_callables",
                        "ts_type_callsites",
                        "ts_type_callees",
                        "ts_type_receivers",
                        "ts_type_file_densities",
                        "ts_type_project_errors",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.identity",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "source_files",
                        "functions",
                        "call_sites",
                        "call_targets",
                        "unresolved_calls",
                        "go_semantic_packages",
                    ],
                    outputs: vec!["identity_records"],
                },
                ProviderOrderRow {
                    id: "polint.abstract_domains",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "source_files",
                        "functions",
                        "mir_bodies",
                        "mir_operations",
                        "places",
                        "unsupported_semantics",
                        "cfg_functions",
                        "basic_blocks",
                        "cfg_edges",
                        "call_sites",
                        "call_targets",
                        "unresolved_calls",
                    ],
                    outputs: vec!["domain_observations", "domain_events"],
                },
                ProviderOrderRow {
                    id: "polint.direct_summaries",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "source_files",
                        "functions",
                        "mir_bodies",
                        "mir_operations",
                        "places",
                        "unsupported_semantics",
                        "cfg_functions",
                        "basic_blocks",
                        "cfg_edges",
                        "call_sites",
                        "call_targets",
                        "unresolved_calls",
                        "domain_observations",
                        "domain_events",
                    ],
                    outputs: vec![
                        "summary_control",
                        "summary_call",
                        "summary_memory",
                        "summary_tito",
                        "summary_events",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.entrypoints",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "source_files",
                        "functions",
                        "symbols",
                        "references",
                        "semantic_imports",
                        "resolved_imports",
                        "import_to_package_edges",
                        "mir_bodies",
                        "mir_operations",
                        "places",
                        "call_sites",
                        "call_targets",
                        "unresolved_calls",
                    ],
                    outputs: vec![
                        "entrypoints",
                        "trust_boundaries",
                        "dispatch_edges",
                        "unresolved_framework",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.reachability",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "source_files",
                        "functions",
                        "symbols",
                        "references",
                        "call_sites",
                        "call_targets",
                        "unresolved_calls",
                        "entrypoints",
                        "identity_records",
                        "exports",
                    ],
                    outputs: vec!["reachability_roots", "call_reachability"],
                },
                ProviderOrderRow {
                    id: "polint.extensions",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "source_files",
                        "functions",
                        "symbols",
                        "references",
                        "entrypoints",
                        "trust_boundaries",
                        "dispatch_edges",
                        "unresolved_framework",
                        "extension.providers",
                    ],
                    outputs: vec!["extension_facts", "extension_rejections"],
                },
                ProviderOrderRow {
                    id: "polint.type_value_alias",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "source_files",
                        "functions",
                        "symbols",
                        "references",
                        "semantic_imports",
                        "resolved_imports",
                        "import_to_package_edges",
                        "mir_bodies",
                        "mir_operations",
                        "places",
                        "cfg_functions",
                        "basic_blocks",
                        "cfg_edges",
                        "call_sites",
                        "call_targets",
                        "unresolved_calls",
                        "domain_observations",
                        "domain_events",
                        "summary_control",
                        "summary_call",
                        "summary_memory",
                        "summary_tito",
                        "entrypoints",
                        "trust_boundaries",
                        "dispatch_edges",
                        "unresolved_framework",
                        "extension_facts",
                        "extension_rejections",
                    ],
                    outputs: vec![
                        "type_facts",
                        "narrowed_type_facts",
                        "value_facts",
                        "allocation_tokens",
                        "access_paths",
                        "points_to_constraints",
                        "points_to_sets",
                        "alias_answers",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.semantic_graph",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "source_files",
                        "functions",
                        "packages",
                        "imports",
                        "resolved_imports",
                        "module_nodes",
                        "scopes",
                        "call_sites",
                        "value_facts",
                        "places",
                        "go_semantic_functions",
                        "go_semantic_callsites",
                        "ts_object_allocations",
                        "ts_property_writes",
                        "ts_property_reads",
                        "ts_receiver_bindings",
                        "ts_prototype_links",
                        "adaptation_model_files",
                        "adaptation_model_budget",
                    ],
                    outputs: vec![
                        "ts_object_allocations",
                        "ts_property_writes",
                        "ts_property_reads",
                        "ts_receiver_bindings",
                        "ts_prototype_links",
                        "semantic_nodes",
                        "semantic_edges",
                        "semantic_constraints",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.solver",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "source_files",
                        "functions",
                        "call_sites",
                        "imports",
                        "resolved_imports",
                        "module_nodes",
                        "reachability_roots",
                        "semantic_constraints",
                        "semantic_nodes",
                        "go_semantic_functions",
                        "go_semantic_callsites",
                        "go_semantic_method_sets",
                        "go_semantic_address_taken",
                        "go_semantic_instantiated_types",
                        "go_semantic_dynamic_dispatch",
                        "ts_object_allocations",
                        "ts_property_writes",
                        "ts_property_reads",
                        "ts_receiver_bindings",
                        "ts_prototype_links",
                    ],
                    outputs: vec![
                        "solver_derived_edges",
                        "solver_budget_status",
                        "solver_budget_reasons",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.refined_calls",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "call_sites",
                        "call_targets",
                        "unresolved_calls",
                        "functions",
                        "symbols",
                        "entrypoints",
                        "dispatch_edges",
                        "summary_call",
                        "summary_events",
                        "type_facts",
                        "value_facts",
                        "allocation_tokens",
                        "points_to_sets",
                        "alias_answers",
                        "extension_facts",
                        "semantic_nodes",
                        "semantic_constraints",
                        "go_semantic_functions",
                        "go_semantic_callsites",
                        "ts_type_callsites",
                        "ts_type_callees",
                        "ts_type_receivers",
                        "ts_type_file_densities",
                        "solver_derived_edges",
                    ],
                    outputs: vec!["refined_call_edges"],
                },
                ProviderOrderRow {
                    id: "polint.data_flow",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "mir_bodies",
                        "mir_operations",
                        "places",
                        "cfg_functions",
                        "cfg_nodes",
                        "cfg_edges",
                        "call_sites",
                        "call_targets",
                        "refined_call_edges",
                        "summary_tito",
                        "summary_events",
                        "type_facts",
                        "value_facts",
                        "access_paths",
                        "points_to_sets",
                        "alias_answers",
                        "entrypoints",
                        "trust_boundaries",
                        "extension_facts",
                    ],
                    outputs: vec![
                        "data_flow_nodes",
                        "data_flow_edges",
                        "data_flow_models",
                        "data_flow_budgets",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.evidence",
                    kind: "whole_repo_derived",
                    language_scope: "multi_language",
                    inputs: vec![
                        "mir_bodies",
                        "mir_operations",
                        "places",
                        "cfg_functions",
                        "cfg_nodes",
                        "cfg_edges",
                        "cfg_control_dependence",
                        "call_sites",
                        "call_targets",
                        "unresolved_calls",
                        "refined_call_edges",
                        "summary_tito",
                        "summary_events",
                        "type_facts",
                        "value_facts",
                        "access_paths",
                        "points_to_sets",
                        "alias_answers",
                        "entrypoints",
                        "trust_boundaries",
                        "dispatch_edges",
                        "extension_facts",
                        "data_flow_nodes",
                        "data_flow_edges",
                        "data_flow_models",
                        "data_flow_budgets",
                    ],
                    outputs: vec![
                        "evidence_nodes",
                        "evidence_edges",
                        "evidence_bundles",
                        "evidence_paths",
                        "evidence_slices",
                        "evidence_unknowns",
                        "evidence_omitted_regions",
                        "evidence_replay_keys",
                    ],
                },
                ProviderOrderRow {
                    id: "polint.metrics",
                    kind: "metrics_derived",
                    language_scope: "multi_language",
                    inputs: vec!["source_files", "functions"],
                    outputs: vec!["file_metrics", "function_metrics", "complexity_metrics"],
                },
            ]
        );
    }

    #[test]
    fn provider_order_report_for_test_is_path_stable() {
        let report = provider_order_report_for_test();
        let rendered = format!("{report:?}");

        assert!(!rendered.contains(env!("CARGO_MANIFEST_DIR")));
        assert!(!rendered.contains('/'));
        assert!(!rendered.contains('\\'));
        assert!(!rendered.contains("202"));
    }

    #[test]
    fn provider_manifests_are_not_public_sdk_runner_or_cli_contract() {
        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let lib = read_source(&crate_root.join("src/lib.rs"));
        assert!(!lib.contains("pub mod analysis_kernel"));

        assert_no_manifest_contract_terms(&crate_root.join("src/runner/mod.rs"));
        assert_no_manifest_contract_terms(&crate_root.join("src/cli/mod.rs"));
        for source_path in rust_sources_under(&crate_root.join("src/sdk")) {
            assert_no_manifest_contract_terms(&source_path);
        }
    }

    fn assert_no_manifest_contract_terms(path: &Path) {
        let source = read_source(path);
        for term in ["ProviderManifest", "provider_order", "provider_manifests"] {
            assert!(
                !source.contains(term),
                "unexpected manifest contract term `{term}` in {}",
                path.display()
            );
        }
    }

    fn rust_sources_under(dir: &Path) -> Vec<PathBuf> {
        let mut sources = Vec::new();
        collect_rust_sources(dir, &mut sources);
        sources.sort();
        sources
    }

    fn collect_rust_sources(dir: &Path, sources: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read sdk source directory") {
            let entry = entry.expect("read sdk source entry");
            let path = entry.path();
            if path.is_dir() {
                collect_rust_sources(&path, sources);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path);
            }
        }
    }

    fn read_source(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|error| {
            panic!("read {}: {error}", path.display());
        })
    }

    #[test]
    fn semantic_mir_manifest_declares_private_provider_contract() {
        let manifest = provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.semantic_mir")
            .expect("semantic MIR manifest should exist");

        assert_eq!(manifest.primary_schema_label(), "semantic-mir-facts-1:1");
        assert_eq!(
            manifest.language_ids,
            crate::frontend::LANGUAGE_IDS_GO_AND_TS
        );
        assert_eq!(manifest.cache_policy, CachePolicy::InMemoryDerived);
        assert_eq!(manifest.precision_ceiling, PrecisionCeiling::SetupAware);
        assert!(manifest.inputs.contains(&"functions"));
        assert!(manifest.inputs.contains(&"symbols"));
        assert!(manifest.inputs.contains(&"references"));
        assert!(manifest.inputs.contains(&"scopes"));
        assert!(manifest.inputs.contains(&"semantic_imports"));
        assert!(manifest.inputs.contains(&"import_to_package_edges"));
        assert!(manifest.outputs.contains(&"mir_bodies"));
        assert!(manifest.outputs.contains(&"mir_operations"));
        assert!(manifest.outputs.contains(&"places"));
        assert!(manifest.outputs.contains(&"unsupported_semantics"));
    }

    #[test]
    fn calls_manifest_declares_private_provider_contract() {
        let manifest = provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.calls")
            .expect("calls manifest should exist");

        assert_eq!(manifest.primary_schema_label(), "calls-facts-1:1");
        assert_eq!(
            manifest.language_ids,
            crate::frontend::LANGUAGE_IDS_GO_AND_TS
        );
        assert_eq!(manifest.cache_policy, CachePolicy::InMemoryDerived);
        assert_eq!(manifest.precision_ceiling, PrecisionCeiling::SetupAware);
        for input in [
            "source_files",
            "functions",
            "symbols",
            "references",
            "semantic_imports",
            "unsupported_semantics",
            "resolved_imports",
            "import_to_package_edges",
            "mir_bodies",
            "mir_operations",
            "places",
            "cfg_functions",
            "cfg_edges",
        ] {
            assert!(manifest.inputs.contains(&input), "missing input {input}");
        }
        assert_eq!(
            manifest.outputs,
            &["call_sites", "call_targets", "unresolved_calls"]
        );
    }

    #[test]
    fn semantic_graph_manifest_declares_ts_direct_binding_inputs() {
        let manifest = provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.semantic_graph")
            .expect("semantic graph manifest should exist");

        for input in [
            "source_files",
            "imports",
            "resolved_imports",
            "module_nodes",
        ] {
            assert!(manifest.inputs.contains(&input), "missing input {input}");
        }
    }

    #[test]
    fn refined_calls_manifest_declares_extension_target_identity_inputs() {
        let manifest = provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.refined_calls")
            .expect("refined calls manifest should exist");

        for input in ["functions", "symbols", "extension_facts"] {
            assert!(manifest.inputs.contains(&input), "missing input {input}");
        }
        assert!(manifest.outputs.contains(&"refined_call_edges"));
    }

    #[test]
    fn demand_query_internals_are_not_public_sdk_runner_cli_or_docs_surface() {
        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));

        // Specific internal type and module names that must not appear in
        // public surfaces.
        let demand_markers = [
            "QueryContext",
            "QueryKind",
            "QueryBudget",
            "QueryTrace",
            "QuarantineSet",
            "QuarantineEntry",
            "QuarantineReason",
            "SccComponent",
            "SccGraph",
            "SccCacheEntry",
            "SccFixpointStatus",
            "DependencyRead",
            "DependencyReadKind",
            "MemoEntry",
            "demand_query_key",
            "analysis::demand",
            "DemandQuery",
            "demand::query",
            "demand::context",
            "demand::scc",
            "demand::quarantine",
        ];

        // Check SDK sources
        for source_path in rust_sources_under(&crate_root.join("src/sdk")) {
            let source = read_source(&source_path);
            for marker in &demand_markers {
                assert!(
                    !source.contains(marker),
                    "demand query internal `{marker}` leaked into SDK: {}",
                    source_path.display()
                );
            }
        }

        // Check runner
        let runner = read_source(&crate_root.join("src/runner/mod.rs"));
        for marker in &demand_markers {
            assert!(
                !runner.contains(marker),
                "demand query internal `{marker}` leaked into runner"
            );
        }

        // Check CLI
        let cli = read_source(&crate_root.join("src/cli/mod.rs"));
        for marker in &demand_markers {
            assert!(
                !cli.contains(marker),
                "demand query internal `{marker}` leaked into CLI"
            );
        }

        // Check lib.rs
        let lib = read_source(&crate_root.join("src/lib.rs"));
        assert!(
            !lib.contains("pub mod demand"),
            "demand module should not be public in lib.rs"
        );

        // Check README
        let readme_path = crate_root
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("README.md");
        if readme_path.exists() {
            let readme = read_source(&readme_path);
            for marker in &demand_markers {
                assert!(
                    !readme.contains(marker),
                    "demand query internal `{marker}` leaked into README"
                );
            }
        }

        // Check docs/facts
        let docs_facts = crate_root
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("docs/facts");
        if docs_facts.exists() {
            for source_path in rust_sources_under(&docs_facts) {
                let source = read_source(&source_path);
                for marker in &demand_markers {
                    assert!(
                        !source.contains(marker),
                        "demand query internal `{marker}` leaked into docs/facts: {}",
                        source_path.display()
                    );
                }
            }
        }
    }

    #[test]
    fn direct_summaries_provider_manifest_declares_private_outputs() {
        let manifest = provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.direct_summaries")
            .expect("direct summaries manifest should exist");

        assert_eq!(manifest.primary_schema_label(), "direct-summary-facts-1:1");
        assert_eq!(
            manifest.language_ids,
            crate::frontend::LANGUAGE_IDS_GO_AND_TS
        );
        assert_eq!(manifest.cache_policy, CachePolicy::InMemoryDerived);
        assert_eq!(manifest.precision_ceiling, PrecisionCeiling::SetupAware);
        for input in [
            "source_files",
            "functions",
            "mir_bodies",
            "mir_operations",
            "places",
            "unsupported_semantics",
            "cfg_functions",
            "basic_blocks",
            "cfg_edges",
            "call_sites",
            "call_targets",
            "unresolved_calls",
            "domain_observations",
            "domain_events",
        ] {
            assert!(manifest.inputs.contains(&input), "missing input {input}");
        }
        assert_eq!(
            manifest.outputs,
            &[
                "summary_control",
                "summary_call",
                "summary_memory",
                "summary_tito",
                "summary_events",
            ]
        );
    }
}
