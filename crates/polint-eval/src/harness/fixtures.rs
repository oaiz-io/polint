use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail, ensure};
use serde::Deserialize;

#[cfg(test)]
use crate::eval::model::ABSTRACT_DOMAIN_FACT_FAMILIES;
use crate::eval::model::{ExpectedItem, FixtureArea, ObservedItem};

const FIXTURE_SCHEMA_VERSION: &str = "polint-eval-fixture-1";
const FIXTURE_MANIFEST_FILE: &str = "expected.polint-eval.toml";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct NativeFixtureManifest {
    pub(crate) schema_version: String,
    pub(crate) case_id: String,
    pub(crate) area: FixtureArea,
    #[serde(default)]
    pub(crate) synthetic_observed: bool,
    pub(crate) repo: FixtureRepo,
    #[serde(default)]
    pub(crate) expected: Vec<ExpectedItem>,
    #[serde(default)]
    pub(crate) observed: Vec<ObservedItem>,
    pub(crate) budget: Option<FixtureBudget>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct FixtureRepo {
    pub(crate) path: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct FixtureBudget {
    pub(crate) max_runtime_ms: u64,
    /// Optional absolute peak-RSS ceiling (bytes). When absent, only the
    /// wall-clock budget participates in `budget_passed`.
    #[serde(default)]
    pub(crate) max_peak_rss_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeFixture {
    pub(crate) fixture_dir: PathBuf,
    pub(crate) repo_dir: PathBuf,
    pub(crate) manifest: NativeFixtureManifest,
}

pub(crate) fn load_native_fixture(fixture_dir: &Path) -> anyhow::Result<NativeFixture> {
    let fixture_dir = fixture_dir
        .canonicalize()
        .with_context(|| format!("canonicalize fixture dir {}", fixture_dir.display()))?;
    let manifest_path = fixture_dir.join(FIXTURE_MANIFEST_FILE);
    let manifest_toml = fs::read_to_string(&manifest_path)
        .with_context(|| format!("read fixture manifest {}", manifest_path.display()))?;
    let mut manifest: NativeFixtureManifest = toml::from_str(&manifest_toml)
        .with_context(|| format!("parse fixture manifest {}", manifest_path.display()))?;

    ensure!(
        manifest.schema_version == FIXTURE_SCHEMA_VERSION,
        "unsupported fixture schema_version `{}`; expected `{FIXTURE_SCHEMA_VERSION}`",
        manifest.schema_version
    );
    validate_manifest_relative_path("repo.path", &manifest.repo.path)?;
    normalize_expected_item_paths(&mut manifest.expected)?;
    normalize_observed_item_paths(&mut manifest.observed)?;
    validate_synthetic_observed_rows(&manifest)?;

    let repo_dir = fixture_dir.join(Path::new(&manifest.repo.path));
    let repo_dir = repo_dir
        .canonicalize()
        .with_context(|| format!("canonicalize fixture repo {}", repo_dir.display()))?;
    ensure!(
        repo_dir.starts_with(&fixture_dir),
        "fixture repo path must stay inside fixture directory"
    );

    Ok(NativeFixture {
        fixture_dir,
        repo_dir,
        manifest,
    })
}

fn normalize_expected_item_paths(expected: &mut [ExpectedItem]) -> anyhow::Result<()> {
    for item in expected {
        if let ExpectedItem::Diagnostic(diagnostic) = item {
            diagnostic.relative_path = normalize_manifest_relative_path(
                "expected.diagnostic.relative_path",
                &diagnostic.relative_path,
            )?;
        }
    }
    Ok(())
}

fn normalize_observed_item_paths(observed: &mut [ObservedItem]) -> anyhow::Result<()> {
    for item in observed {
        if let ObservedItem::Diagnostic(diagnostic) = item {
            diagnostic.relative_path = normalize_manifest_relative_path(
                "observed.diagnostic.relative_path",
                &diagnostic.relative_path,
            )?;
        }
    }
    Ok(())
}

fn validate_synthetic_observed_rows(manifest: &NativeFixtureManifest) -> anyhow::Result<()> {
    if manifest.synthetic_observed {
        ensure!(
            matches!(
                manifest.area,
                FixtureArea::Extension | FixtureArea::RefinedCalls | FixtureArea::Promotion
            ),
            "synthetic observed rows are only allowed for extension, refined-call, or promotion fixtures"
        );
    } else {
        ensure!(
            manifest.observed.is_empty(),
            "manifest observed rows require synthetic_observed = true"
        );
    }
    Ok(())
}

fn normalize_manifest_relative_path(field: &str, value: &str) -> anyhow::Result<String> {
    validate_manifest_relative_path(field, value)?;
    Ok(value.replace('\\', "/"))
}

fn validate_manifest_relative_path(field: &str, value: &str) -> anyhow::Result<()> {
    if is_absolute_manifest_path(value) {
        bail!("{field} must be relative, not absolute");
    }
    if has_parent_dir_component(value) {
        bail!("{field} must not contain a parent directory component");
    }
    Ok(())
}

fn is_absolute_manifest_path(value: &str) -> bool {
    Path::new(value).is_absolute()
        || value.starts_with('/')
        || value.starts_with('\\')
        || value.as_bytes().get(1) == Some(&b':')
}

fn has_parent_dir_component(value: &str) -> bool {
    Path::new(value)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
        || value.split(['/', '\\']).any(|component| component == "..")
}

#[cfg(test)]
pub(crate) fn run_native_fixture_for_test(
    fixture_dir: &Path,
) -> anyhow::Result<crate::eval::report::EvaluationRun> {
    let fixture = load_native_fixture(fixture_dir)?;
    let observed = if fixture.manifest.synthetic_observed {
        fixture.manifest.observed.clone()
    } else {
        crate::eval::observed::observe_kernel_fixture(&fixture)?
    };
    Ok(evaluation_run_for_fixture(&fixture, observed))
}

/// Builds the normalized [`EvaluationRun`] for `fixture` from a CALLER-SUPPLIED
/// observed-item vector. The determinism gate (`eval::determinism_gate`) uses
/// this to feed seeded-permuted observed rows through the exact same
/// `normalize_run` + `deterministic_output_hash` path the live fixture runner
/// uses, proving the normalized observed JSON is byte-identical regardless of
/// row-insertion order (D-20/D-21).
#[cfg(test)]
pub(crate) fn evaluation_run_for_fixture_with_observed_for_test(
    fixture: &NativeFixture,
    observed: Vec<crate::eval::model::ObservedItem>,
) -> crate::eval::report::EvaluationRun {
    evaluation_run_for_fixture(fixture, observed)
}

#[cfg(test)]
pub(crate) fn run_cache_current_determinism_fixture_for_test(
    fixture_dir: &Path,
) -> anyhow::Result<crate::eval::report::EvaluationRun> {
    let fixture = load_native_fixture(fixture_dir)?;
    let temp = crate::eval::observed::copy_fixture_repo_for_test(&fixture)?;

    let cold_observed =
        crate::eval::observed::observe_kernel_fixture_repo_for_test(&fixture, temp.path(), true)?;
    let warm_observed =
        crate::eval::observed::observe_kernel_fixture_repo_for_test(&fixture, temp.path(), true)?;
    let no_cache_observed =
        crate::eval::observed::observe_kernel_fixture_repo_for_test(&fixture, temp.path(), false)?;

    let cold_run = evaluation_run_for_fixture(&fixture, cold_observed);
    let warm_run = evaluation_run_for_fixture(&fixture, warm_observed);
    let no_cache_run = evaluation_run_for_fixture(&fixture, no_cache_observed.clone());

    let cold_warm_no_cache_equal = cache_comparison_json(&cold_run)
        == cache_comparison_json(&warm_run)
        && cache_comparison_json(&cold_run) == cache_comparison_json(&no_cache_run);

    let mut observed = no_cache_observed;
    if cold_warm_no_cache_equal {
        observed.push(cache_current_determinism_observed_invariant());
    }

    Ok(evaluation_run_for_fixture(&fixture, observed))
}

#[cfg(test)]
pub(crate) fn run_layer_cache_fixture_for_test(
    fixture_dir: &Path,
) -> anyhow::Result<crate::eval::report::EvaluationRun> {
    let started = std::time::Instant::now();
    let fixture = load_native_fixture(fixture_dir)?;
    let temp = crate::eval::observed::copy_fixture_repo_for_test(&fixture)?;
    let plan = crate::analysis_plan::AnalysisPlan::from_capability_names_for_test(&[
        "resolved_imports",
        "module_graph",
        "symbols",
        "references",
        "file_metrics",
        "function_metrics",
        "complexity_metrics",
    ]);

    let cold_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    let warm_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    let layer_file_count_after_warm = managed_layer_file_count(temp.path())?;
    let disabled_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        false,
        &plan,
    )?;
    let layer_file_count_after_disabled = managed_layer_file_count(temp.path())?;

    apply_layer_cache_import_edit(temp.path())?;
    let import_edit_observed =
        crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
            &fixture,
            temp.path(),
            true,
            &plan,
        )?;

    let mut observed = Vec::new();
    observed.extend(tagged_layer_cache_invariants("cold", &cold_observed));
    observed.extend(tagged_layer_cache_invariants("warm", &warm_observed));
    observed.extend(tagged_layer_cache_invariants(
        "disabled",
        &disabled_observed,
    ));
    observed.extend(tagged_layer_cache_invariants(
        "import_edit",
        &import_edit_observed,
    ));
    observed.push(layer_cache_fixture_invariant(
        "layer_cache.disabled.no_new_files",
        bool_string(layer_file_count_after_warm == layer_file_count_after_disabled),
    ));
    if fixture.manifest.budget.is_some() {
        let elapsed = started.elapsed();
        observed.push(crate::eval::model::ObservedItem::RuntimeBudget(
            observe_fixture_runtime_budget(&fixture, elapsed),
        ));
    }

    Ok(evaluation_run_for_fixture(&fixture, observed))
}

#[cfg(test)]
pub(crate) fn run_semantic_index_core_fixture_for_test(
    fixture_dir: &Path,
) -> anyhow::Result<crate::eval::report::EvaluationRun> {
    let started = std::time::Instant::now();
    let fixture = load_native_fixture(fixture_dir)?;
    let temp = crate::eval::observed::copy_fixture_repo_for_test(&fixture)?;
    let plan = crate::analysis_plan::AnalysisPlan::from_capability_names_for_test(&[
        "resolved_imports",
        "module_graph",
        "symbols",
        "references",
    ]);

    let cold_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    let warm_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;

    let mut observed = warm_observed
        .iter()
        .filter(|item| {
            !is_cache_stats_observed_invariant(item)
                && !matches!(item, crate::eval::model::ObservedItem::RuntimeBudget(_))
        })
        .cloned()
        .collect::<Vec<_>>();
    observed.extend(tagged_layer_cache_invariants("cold", &cold_observed));
    observed.extend(tagged_layer_cache_invariants("warm", &warm_observed));

    if fixture.manifest.budget.is_some() {
        let elapsed = started.elapsed();
        observed.push(crate::eval::model::ObservedItem::RuntimeBudget(
            observe_fixture_runtime_budget(&fixture, elapsed),
        ));
    }

    Ok(evaluation_run_for_fixture(&fixture, observed))
}

#[cfg(test)]
pub(crate) fn run_module_topology_core_fixture_for_test(
    fixture_dir: &Path,
) -> anyhow::Result<crate::eval::report::EvaluationRun> {
    let started = std::time::Instant::now();
    let fixture = load_native_fixture(fixture_dir)?;
    let temp = crate::eval::observed::copy_fixture_repo_for_test(&fixture)?;
    let plan = crate::analysis_plan::AnalysisPlan::from_capability_names_for_test(&["calls"]);

    let cold_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    let warm_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;

    apply_module_topology_package_json_edit(temp.path())?;
    let package_json_observed =
        crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
            &fixture,
            temp.path(),
            true,
            &plan,
        )?;

    apply_module_topology_go_mod_edit(temp.path())?;
    let go_mod_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;

    let mut observed = warm_observed
        .iter()
        .filter(|item| !is_cache_comparison_observed_invariant(item))
        .cloned()
        .collect::<Vec<_>>();
    observed.extend(tagged_layer_cache_invariants("cold", &cold_observed));
    observed.extend(tagged_layer_cache_invariants("warm", &warm_observed));
    observed.extend(tagged_layer_cache_invariants(
        "package_json_edit",
        &package_json_observed,
    ));
    observed.extend(tagged_layer_cache_invariants(
        "go_mod_edit",
        &go_mod_observed,
    ));

    if fixture.manifest.budget.is_some() {
        let elapsed = started.elapsed();
        observed.push(crate::eval::model::ObservedItem::RuntimeBudget(
            observe_fixture_runtime_budget(&fixture, elapsed),
        ));
    }

    Ok(evaluation_run_for_fixture(&fixture, observed))
}

#[cfg(test)]
pub(crate) fn run_semantic_mir_core_fixture_for_test(
    fixture_dir: &Path,
) -> anyhow::Result<crate::eval::report::EvaluationRun> {
    let started = std::time::Instant::now();
    let fixture = load_native_fixture(fixture_dir)?;
    let temp = crate::eval::observed::copy_fixture_repo_for_test(&fixture)?;
    let plan =
        crate::analysis_plan::AnalysisPlan::from_capability_names_for_test(&["control_flow"]);

    let cold_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    let warm_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;

    let cold_run = evaluation_run_for_fixture(&fixture, cold_observed);
    let warm_run = evaluation_run_for_fixture(&fixture, warm_observed.clone());
    let cold_warm_equal = cache_comparison_json(&cold_run) == cache_comparison_json(&warm_run);

    let mut observed = warm_observed
        .iter()
        .filter(|item| !is_cache_comparison_observed_invariant(item))
        .cloned()
        .collect::<Vec<_>>();
    if cold_warm_equal {
        observed.push(semantic_mir_determinism_observed_invariant());
    }

    if fixture.manifest.budget.is_some() {
        let elapsed = started.elapsed();
        observed.push(crate::eval::model::ObservedItem::RuntimeBudget(
            observe_fixture_runtime_budget(&fixture, elapsed),
        ));
    }

    Ok(evaluation_run_for_fixture(&fixture, observed))
}

#[cfg(test)]
pub(crate) fn run_cfg_core_fixture_for_test(
    fixture_dir: &Path,
) -> anyhow::Result<crate::eval::report::EvaluationRun> {
    let started = std::time::Instant::now();
    let fixture = load_native_fixture(fixture_dir)?;
    let temp = crate::eval::observed::copy_fixture_repo_for_test(&fixture)?;
    let plan = crate::analysis_plan::AnalysisPlan::from_capability_names_for_test(&["calls"]);

    let cold_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    eprintln!("CFG_COLD_OBSERVED: {cold_observed:#?}");

    let warm_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    let no_cache_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        false,
        &plan,
    )?;

    let cold_run = evaluation_run_for_fixture(&fixture, cold_observed);
    let warm_run = evaluation_run_for_fixture(&fixture, warm_observed);
    let no_cache_run = evaluation_run_for_fixture(&fixture, no_cache_observed.clone());
    let deterministic = framework_entrypoints_cache_comparison_json(&cold_run)
        == framework_entrypoints_cache_comparison_json(&warm_run)
        && framework_entrypoints_cache_comparison_json(&cold_run)
            == framework_entrypoints_cache_comparison_json(&no_cache_run);

    let mut observed = no_cache_observed
        .iter()
        .filter(|item| !is_cache_comparison_observed_invariant(item))
        .cloned()
        .collect::<Vec<_>>();
    if deterministic {
        observed.push(cfg_determinism_observed_invariant());
    }

    if fixture.manifest.budget.is_some() {
        let elapsed = started.elapsed();
        observed.push(crate::eval::model::ObservedItem::RuntimeBudget(
            observe_fixture_runtime_budget(&fixture, elapsed),
        ));
    }

    Ok(evaluation_run_for_fixture(&fixture, observed))
}

#[cfg(test)]
pub(crate) fn run_direct_calls_core_fixture_for_test(
    fixture_dir: &Path,
) -> anyhow::Result<crate::eval::report::EvaluationRun> {
    let started = std::time::Instant::now();
    let fixture = load_native_fixture(fixture_dir)?;
    let temp = crate::eval::observed::copy_fixture_repo_for_test(&fixture)?;
    let plan = crate::analysis_plan::AnalysisPlan::from_capability_names_for_test(&["calls"]);

    let cold_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    let warm_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    let no_cache_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        false,
        &plan,
    )?;

    let cold_run = evaluation_run_for_fixture(&fixture, cold_observed);
    let warm_run = evaluation_run_for_fixture(&fixture, warm_observed);
    let no_cache_run = evaluation_run_for_fixture(&fixture, no_cache_observed.clone());
    let deterministic = cache_comparison_json(&cold_run) == cache_comparison_json(&warm_run)
        && cache_comparison_json(&cold_run) == cache_comparison_json(&no_cache_run);

    let mut observed = no_cache_observed
        .iter()
        .filter(|item| !is_cache_comparison_observed_invariant(item))
        .cloned()
        .collect::<Vec<_>>();
    if deterministic {
        observed.push(direct_calls_determinism_observed_invariant());
    }

    if fixture.manifest.budget.is_some() {
        let elapsed = started.elapsed();
        observed.push(crate::eval::model::ObservedItem::RuntimeBudget(
            observe_fixture_runtime_budget(&fixture, elapsed),
        ));
    }

    Ok(evaluation_run_for_fixture(&fixture, observed))
}

#[cfg(test)]
fn cached_direct_calls_core_fixture_for_test(
    fixture_dir: &Path,
) -> anyhow::Result<&'static crate::eval::report::EvaluationRun> {
    static RUN: std::sync::OnceLock<Result<crate::eval::report::EvaluationRun, String>> =
        std::sync::OnceLock::new();

    match RUN.get_or_init(|| {
        run_direct_calls_core_fixture_for_test(fixture_dir).map_err(|error| format!("{error:#}"))
    }) {
        Ok(run) => Ok(run),
        Err(error) => Err(anyhow::anyhow!("{error}")),
    }
}

#[cfg(test)]
pub(crate) fn run_abstract_domains_core_fixture_for_test(
    fixture_dir: &Path,
) -> anyhow::Result<crate::eval::report::EvaluationRun> {
    let started = std::time::Instant::now();
    let fixture = load_native_fixture(fixture_dir)?;
    let temp = crate::eval::observed::copy_fixture_repo_for_test(&fixture)?;
    let plan = crate::analysis_plan::AnalysisPlan::from_capability_names_for_test(&["calls"]);

    let cold_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    let warm_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    let no_cache_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        false,
        &plan,
    )?;

    let cold_run = evaluation_run_for_fixture(&fixture, cold_observed);
    let warm_run = evaluation_run_for_fixture(&fixture, warm_observed);
    let no_cache_run = evaluation_run_for_fixture(&fixture, no_cache_observed.clone());
    let deterministic = abstract_domain_cache_comparison_json(&cold_run)
        == abstract_domain_cache_comparison_json(&warm_run)
        && abstract_domain_cache_comparison_json(&cold_run)
            == abstract_domain_cache_comparison_json(&no_cache_run);

    let mut observed = no_cache_observed
        .iter()
        .filter(|item| !is_cache_comparison_observed_invariant(item))
        .cloned()
        .collect::<Vec<_>>();
    observed.extend(abstract_domain_observed_with_policy(
        temp.path(),
        &plan,
        crate::analysis_neutral::domains::solver::SolverPolicy::for_test(
            crate::analysis_neutral::domains::solver::SolverBudget {
                max_iterations: 10_000,
                widening_fuel: 0,
            },
        ),
    )?);
    observed.extend(abstract_domain_observed_with_policy(
        temp.path(),
        &plan,
        crate::analysis_neutral::domains::solver::SolverPolicy::for_test(
            crate::analysis_neutral::domains::solver::SolverBudget {
                max_iterations: 0,
                widening_fuel: 8,
            },
        ),
    )?);
    if deterministic {
        observed.push(abstract_domains_determinism_observed_invariant());
    }

    if fixture.manifest.budget.is_some() {
        let elapsed = started.elapsed();
        observed.push(crate::eval::model::ObservedItem::RuntimeBudget(
            observe_fixture_runtime_budget(&fixture, elapsed),
        ));
    }

    Ok(evaluation_run_for_fixture(&fixture, observed))
}

#[cfg(test)]
pub(crate) fn run_direct_summaries_core_fixture_for_test(
    fixture_dir: &Path,
) -> anyhow::Result<crate::eval::report::EvaluationRun> {
    let started = std::time::Instant::now();
    let fixture = load_native_fixture(fixture_dir)?;
    let temp = crate::eval::observed::copy_fixture_repo_for_test(&fixture)?;
    let plan = crate::analysis_plan::AnalysisPlan::from_capability_names_for_test(&["calls"]);

    let cold_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    let warm_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    let no_cache_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        false,
        &plan,
    )?;

    let cold_run = evaluation_run_for_fixture(&fixture, cold_observed);
    let warm_run = evaluation_run_for_fixture(&fixture, warm_observed);
    let no_cache_run = evaluation_run_for_fixture(&fixture, no_cache_observed.clone());
    let deterministic = cache_comparison_json(&cold_run) == cache_comparison_json(&warm_run)
        && cache_comparison_json(&cold_run) == cache_comparison_json(&no_cache_run);

    let mut observed = no_cache_observed
        .iter()
        .filter(|item| !is_cache_comparison_observed_invariant(item))
        .cloned()
        .collect::<Vec<_>>();
    if deterministic {
        observed.push(direct_summaries_determinism_observed_invariant());
    }

    if fixture.manifest.budget.is_some() {
        let elapsed = started.elapsed();
        observed.push(crate::eval::model::ObservedItem::RuntimeBudget(
            observe_fixture_runtime_budget(&fixture, elapsed),
        ));
    }

    Ok(evaluation_run_for_fixture(&fixture, observed))
}

#[cfg(test)]
pub(crate) fn run_direct_summaries_scc_closure_fixture_for_test(
    fixture_dir: &Path,
) -> anyhow::Result<crate::eval::report::EvaluationRun> {
    let started = std::time::Instant::now();
    let fixture = load_native_fixture(fixture_dir)?;
    let temp = crate::eval::observed::copy_fixture_repo_for_test(&fixture)?;
    let plan = crate::analysis_plan::AnalysisPlan::from_capability_names_for_test(&["calls"]);

    let cold_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    let warm_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        true,
        &plan,
    )?;
    let no_cache_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        false,
        &plan,
    )?;

    let cold_run = evaluation_run_for_fixture(&fixture, cold_observed);
    let warm_run = evaluation_run_for_fixture(&fixture, warm_observed);
    let no_cache_run = evaluation_run_for_fixture(&fixture, no_cache_observed.clone());
    let deterministic = cache_comparison_json(&cold_run) == cache_comparison_json(&warm_run)
        && cache_comparison_json(&cold_run) == cache_comparison_json(&no_cache_run);

    let mut observed = no_cache_observed
        .iter()
        .filter(|item| !is_cache_stats_observed_invariant(item))
        .cloned()
        .collect::<Vec<_>>();
    if deterministic {
        observed.push(direct_summaries_determinism_observed_invariant());
    }

    if fixture.manifest.budget.is_some() {
        let elapsed = started.elapsed();
        observed.push(crate::eval::model::ObservedItem::RuntimeBudget(
            observe_fixture_runtime_budget(&fixture, elapsed),
        ));
    }

    Ok(evaluation_run_for_fixture(&fixture, observed))
}

#[cfg(test)]
pub(crate) fn run_framework_entrypoints_core_fixture_for_test(
    fixture_dir: &Path,
) -> anyhow::Result<crate::eval::report::EvaluationRun> {
    let started = std::time::Instant::now();
    let fixture = load_native_fixture(fixture_dir)?;
    let temp = crate::eval::observed::copy_fixture_repo_for_test(&fixture)?;
    let plan = crate::analysis_plan::AnalysisPlan::from_capability_names_for_test(&["calls"]);

    let no_cache_observed = crate::eval::observed::observe_kernel_fixture_repo_with_plan_for_test(
        &fixture,
        temp.path(),
        false,
        &plan,
    )?;

    let mut observed = no_cache_observed
        .into_iter()
        .filter(|item| !is_cache_comparison_observed_invariant(item))
        .collect::<Vec<_>>();

    if fixture.manifest.budget.is_some() {
        let elapsed = started.elapsed();
        observed.push(crate::eval::model::ObservedItem::RuntimeBudget(
            observe_fixture_runtime_budget(&fixture, elapsed),
        ));
    }

    Ok(evaluation_run_for_fixture(&fixture, observed))
}

#[cfg(test)]
fn abstract_domain_observed_with_policy(
    repo_root: &Path,
    plan: &crate::analysis_plan::AnalysisPlan,
    policy: crate::analysis_neutral::domains::solver::SolverPolicy,
) -> anyhow::Result<Vec<crate::eval::model::ObservedItem>> {
    let loaded = crate::config::load_config(repo_root)?;
    let config_digest = crate::cache::keys::config_hash(&loaded);
    let rule_digest = crate::cache::keys::rule_hash(&[], None, &std::collections::BTreeMap::new());
    let cache = crate::cache::Cache::default_for_repo(repo_root, false);
    let output =
        crate::analysis_kernel::AnalysisKernel::run(crate::analysis_kernel::KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: &config_digest,
            rule_digest: &rule_digest,
            plan,
            parallel: true,
        })?;
    let solver = crate::analysis_neutral::domains::solver::IdeDomainSolver::new(policy);
    let result = solver.solve(crate::analysis_neutral::domains::solver::SolverInput::from(
        &output.db,
    ));
    let mut db = output.db;
    let interner = db.stable_key_interner();
    let place_stable_keys = db
        .mir_places()
        .iter()
        .map(|place| (place.id, interner.resolve(place.stable_key).to_string()))
        .collect();
    db.replace_abstract_domain_facts(
        crate::analysis_neutral::domains::store::DomainOutput::from_results_with_place_keys(
            &db.stable_key_interner(),
            result.results(),
            &place_stable_keys,
        ),
    );
    let debug = crate::analysis_kernel::AnalysisKernel::metadata_debug_json_for_test(&db);
    Ok(crate::eval::observed::abstract_domain_facts_for_test(
        &debug,
    ))
}

#[cfg(test)]
fn evaluation_run_for_fixture(
    fixture: &NativeFixture,
    observed: Vec<crate::eval::model::ObservedItem>,
) -> crate::eval::report::EvaluationRun {
    let matches = crate::eval::matcher::match_case(
        &fixture.manifest.expected,
        &observed,
        crate::eval::matcher::MatcherConfig::default(),
    );
    let mut metrics: crate::eval::report::MetricSummary =
        crate::eval::metrics::compute_metrics(&matches).into();
    metrics.sections.jelly_oracle_coverage =
        crate::eval::metrics::jelly_oracle_coverage(&fixture.manifest.expected, &observed);
    metrics.sections.categorized_failures =
        crate::eval::metrics::categorized_failures_from_observed(&observed);
    let runtime = runtime_observation(&fixture.manifest, &observed);
    let run = crate::eval::report::EvaluationRun {
        schema_version: crate::eval::report::EVALUATION_SCHEMA_VERSION.to_string(),
        suite_id: "native-fixtures".to_string(),
        mode: crate::eval::model::EvaluationMode::PolintBaseline,
        suite_manifest: None,
        cases: vec![crate::eval::report::CaseResult {
            case_id: fixture.manifest.case_id.clone(),
            area: fixture.manifest.area,
            expected: fixture.manifest.expected.clone(),
            observed,
            matches,
            runtime,
        }],
        metrics,
        performance: None,
        comparison_rows: Vec::new(),
        adaptation: None,
        adaptation_delta: None,
        limitations: Vec::new(),
        output_hash: String::new(),
    };
    let mut run = crate::eval::report::normalize_run(&run);
    run.output_hash = crate::eval::report::deterministic_output_hash(&run);

    run
}

#[cfg(test)]
fn cache_current_determinism_observed_invariant() -> crate::eval::model::ObservedItem {
    crate::eval::model::ObservedItem::Invariant(crate::eval::model::ObservedInvariant {
        name: "cache.current_determinism".to_string(),
        value: "cold_warm_no_cache_equal".to_string(),
        mode: crate::eval::model::AssertionMode::Exact,
        producer_id: Some("polint.eval".to_string()),
        provenance: Some("cache.current_behavior".to_string()),
        precision: Some("exact".to_string()),
        status: Some(crate::eval::model::ObservedStatus::Present),
    })
}

#[cfg(test)]
fn semantic_mir_determinism_observed_invariant() -> crate::eval::model::ObservedItem {
    crate::eval::model::ObservedItem::Invariant(crate::eval::model::ObservedInvariant {
        name: "semantic_mir.current_determinism".to_string(),
        value: "cold_warm_equal".to_string(),
        mode: crate::eval::model::AssertionMode::Exact,
        producer_id: Some("polint.eval".to_string()),
        provenance: Some("semantic_mir.current_behavior".to_string()),
        precision: Some("exact".to_string()),
        status: Some(crate::eval::model::ObservedStatus::Present),
    })
}

#[cfg(test)]
fn cfg_determinism_observed_invariant() -> crate::eval::model::ObservedItem {
    crate::eval::model::ObservedItem::Invariant(crate::eval::model::ObservedInvariant {
        name: "cfg.current_determinism".to_string(),
        value: "cold_warm_no_cache_equal".to_string(),
        mode: crate::eval::model::AssertionMode::Exact,
        producer_id: Some("polint.eval".to_string()),
        provenance: Some("cfg.current_behavior".to_string()),
        precision: Some("exact".to_string()),
        status: Some(crate::eval::model::ObservedStatus::Present),
    })
}

#[cfg(test)]
fn direct_calls_determinism_observed_invariant() -> crate::eval::model::ObservedItem {
    crate::eval::model::ObservedItem::Invariant(crate::eval::model::ObservedInvariant {
        name: "direct_calls.current_determinism".to_string(),
        value: "cold_warm_no_cache_equal".to_string(),
        mode: crate::eval::model::AssertionMode::Exact,
        producer_id: Some("polint.eval".to_string()),
        provenance: Some("direct_calls.current_behavior".to_string()),
        precision: Some("exact".to_string()),
        status: Some(crate::eval::model::ObservedStatus::Present),
    })
}

#[cfg(test)]
fn abstract_domains_determinism_observed_invariant() -> crate::eval::model::ObservedItem {
    crate::eval::model::ObservedItem::Invariant(crate::eval::model::ObservedInvariant {
        name: "abstract_domains.current_determinism".to_string(),
        value: "cold_warm_no_cache_equal".to_string(),
        mode: crate::eval::model::AssertionMode::Exact,
        producer_id: Some("polint.eval".to_string()),
        provenance: Some("abstract_domains.current_behavior".to_string()),
        precision: Some("exact".to_string()),
        status: Some(crate::eval::model::ObservedStatus::Present),
    })
}

#[cfg(test)]
fn direct_summaries_determinism_observed_invariant() -> crate::eval::model::ObservedItem {
    crate::eval::model::ObservedItem::Invariant(crate::eval::model::ObservedInvariant {
        name: "direct_summaries.current_determinism".to_string(),
        value: "cold_warm_no_cache_equal".to_string(),
        mode: crate::eval::model::AssertionMode::Exact,
        producer_id: Some("polint.eval".to_string()),
        provenance: Some("direct_summaries.current_behavior".to_string()),
        precision: Some("exact".to_string()),
        status: Some(crate::eval::model::ObservedStatus::Present),
    })
}

#[cfg(test)]
fn cache_comparison_json(run: &crate::eval::report::EvaluationRun) -> String {
    let mut normalized = run_without_runtime_durations(run);
    normalized.output_hash = crate::eval::report::deterministic_output_hash(&normalized);
    serde_json::to_string_pretty(&normalized).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
fn abstract_domain_cache_comparison_json(run: &crate::eval::report::EvaluationRun) -> String {
    let mut normalized = run_without_runtime_durations(run);
    for case in &mut normalized.cases {
        case.observed
            .retain(abstract_domain_comparison_observed_item);
        case.matches = crate::eval::matcher::match_case(
            &case.expected,
            &case.observed,
            crate::eval::matcher::MatcherConfig::default(),
        );
    }
    let matches = normalized
        .cases
        .iter()
        .flat_map(|case| case.matches.iter().cloned())
        .collect::<Vec<_>>();
    normalized.metrics = crate::eval::metrics::compute_metrics(&matches).into();
    normalized.output_hash = crate::eval::report::deterministic_output_hash(&normalized);
    serde_json::to_string_pretty(&normalized).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
fn framework_entrypoints_cache_comparison_json(run: &crate::eval::report::EvaluationRun) -> String {
    let mut normalized = run_without_runtime_durations(run);
    for case in &mut normalized.cases {
        case.observed
            .retain(framework_entrypoints_comparison_observed_item);
        case.matches = crate::eval::matcher::match_case(
            &case.expected,
            &case.observed,
            crate::eval::matcher::MatcherConfig::default(),
        );
    }
    let matches = normalized
        .cases
        .iter()
        .flat_map(|case| case.matches.iter().cloned())
        .collect::<Vec<_>>();
    normalized.metrics = crate::eval::metrics::compute_metrics(&matches).into();
    normalized.output_hash = crate::eval::report::deterministic_output_hash(&normalized);
    serde_json::to_string_pretty(&normalized).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
fn framework_entrypoints_comparison_observed_item(item: &ObservedItem) -> bool {
    match item {
        ObservedItem::Fact(fact) => matches!(
            fact.family.as_str(),
            "Entrypoint" | "TrustBoundary" | "DispatchEdge" | "UnresolvedFramework"
        ),
        ObservedItem::Invariant(invariant) => invariant.name.starts_with("framework_entrypoints."),
        ObservedItem::RuntimeBudget(_) => true,
        ObservedItem::Diagnostic(_) | ObservedItem::GraphEdge(_) | ObservedItem::Path(_) => false,
    }
}

#[cfg(test)]
fn abstract_domain_comparison_observed_item(item: &ObservedItem) -> bool {
    match item {
        ObservedItem::Fact(fact) => ABSTRACT_DOMAIN_FACT_FAMILIES.contains(&fact.family.as_str()),
        ObservedItem::Invariant(invariant) => {
            invariant.name.starts_with("abstract_domains.")
                || invariant
                    .name
                    .starts_with("provider_output.polint.abstract_domains.")
        }
        ObservedItem::RuntimeBudget(_) => true,
        ObservedItem::Diagnostic(_) | ObservedItem::GraphEdge(_) | ObservedItem::Path(_) => false,
    }
}

#[cfg(test)]
fn run_without_runtime_durations(
    run: &crate::eval::report::EvaluationRun,
) -> crate::eval::report::EvaluationRun {
    let mut normalized = crate::eval::report::normalize_run(run);
    normalized.output_hash.clear();
    for case in &mut normalized.cases {
        case.runtime.observed_runtime_ms = None;
        case.runtime.peak_rss_bytes = None;
        case.observed
            .retain(|item| !is_cache_comparison_observed_invariant(item));
        for item in &mut case.observed {
            if let crate::eval::model::ObservedItem::RuntimeBudget(budget) = item {
                budget.observed_runtime_ms = None;
                budget.peak_rss_bytes = None;
            }
        }
        for summary in &mut case.matches {
            summary.observed_runtime_ms = None;
        }
        case.matches = crate::eval::matcher::match_case(
            &case.expected,
            &case.observed,
            crate::eval::matcher::MatcherConfig::default(),
        );
    }
    let matches = normalized
        .cases
        .iter()
        .flat_map(|case| case.matches.iter().cloned())
        .collect::<Vec<_>>();
    normalized.metrics = crate::eval::metrics::compute_metrics(&matches).into();
    normalized
}

#[cfg(test)]
fn is_cache_stats_observed_invariant(item: &crate::eval::model::ObservedItem) -> bool {
    match item {
        crate::eval::model::ObservedItem::Invariant(invariant) => {
            (invariant.name.starts_with("provider_output.polint.")
                && invariant.name.contains(".cache_stats."))
                || invariant.name.starts_with("layer_cache.")
        }
        _ => false,
    }
}

#[cfg(test)]
fn is_cache_comparison_observed_invariant(item: &crate::eval::model::ObservedItem) -> bool {
    if is_cache_stats_observed_invariant(item) {
        return true;
    }
    match item {
        crate::eval::model::ObservedItem::Invariant(invariant) => {
            matches!(
                invariant.name.as_str(),
                "demand_queries.cache_hits.nonzero" | "demand_queries.cache_misses.nonzero"
            )
        }
        _ => false,
    }
}

#[cfg(test)]
fn tagged_layer_cache_invariants(
    pass: &str,
    observed: &[crate::eval::model::ObservedItem],
) -> Vec<crate::eval::model::ObservedItem> {
    observed
        .iter()
        .filter_map(|item| {
            let crate::eval::model::ObservedItem::Invariant(invariant) = item else {
                return None;
            };
            if !matches!(
                invariant.name.as_str(),
                name if name.starts_with("layer_cache.provider.")
                    || name.starts_with("layer_cache.aggregate.")
            ) {
                return None;
            }
            let mut tagged = invariant.clone();
            tagged.value = format!("{pass}={}", invariant.value);
            Some(crate::eval::model::ObservedItem::Invariant(tagged))
        })
        .collect()
}

#[cfg(test)]
fn apply_layer_cache_import_edit(repo_root: &Path) -> anyhow::Result<()> {
    let app_path = repo_root.join("web/src/app.ts");
    let source = fs::read_to_string(&app_path)
        .with_context(|| format!("read layer-cache fixture {}", app_path.display()))?;
    let edited = source.replace("@billing/rates", "@billing/tax");
    ensure!(
        edited != source,
        "layer-cache fixture import edit did not change {}",
        app_path.display()
    );
    fs::write(&app_path, edited)
        .with_context(|| format!("write layer-cache fixture {}", app_path.display()))
}

#[cfg(test)]
fn apply_module_topology_package_json_edit(repo_root: &Path) -> anyhow::Result<()> {
    let manifest_path = repo_root.join("web/package.json");
    let source = fs::read_to_string(&manifest_path)
        .with_context(|| format!("read module topology fixture {}", manifest_path.display()))?;
    let edited = source.replace("\"^2.3.0\"", "\"^2.4.0\"");
    ensure!(
        edited != source,
        "module topology package.json edit did not change {}",
        manifest_path.display()
    );
    fs::write(&manifest_path, edited)
        .with_context(|| format!("write module topology fixture {}", manifest_path.display()))
}

#[cfg(test)]
fn apply_module_topology_go_mod_edit(repo_root: &Path) -> anyhow::Result<()> {
    let manifest_path = repo_root.join("services/api/go.mod");
    let source = fs::read_to_string(&manifest_path)
        .with_context(|| format!("read module topology fixture {}", manifest_path.display()))?;
    let edited = source.replace("github.com/acme/lib v1.2.3", "github.com/acme/lib v1.2.4");
    ensure!(
        edited != source,
        "module topology go.mod edit did not change {}",
        manifest_path.display()
    );
    fs::write(&manifest_path, edited)
        .with_context(|| format!("write module topology fixture {}", manifest_path.display()))
}

#[cfg(test)]
fn managed_layer_file_count(repo_root: &Path) -> anyhow::Result<usize> {
    let layer_root = repo_root.join(".polint/cache/layers");
    if !layer_root.exists() {
        return Ok(0);
    }
    managed_layer_file_count_in(&layer_root)
}

#[cfg(test)]
fn managed_layer_file_count_in(path: &Path) -> anyhow::Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(path).with_context(|| format!("read {}", path.display()))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            count += managed_layer_file_count_in(&path)?;
        } else if file_type.is_file() {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
fn layer_cache_fixture_invariant(
    name: impl Into<String>,
    value: impl Into<String>,
) -> crate::eval::model::ObservedItem {
    crate::eval::model::ObservedItem::Invariant(crate::eval::model::ObservedInvariant {
        name: name.into(),
        value: value.into(),
        mode: crate::eval::model::AssertionMode::Exact,
        producer_id: Some("polint.eval".to_string()),
        provenance: Some("kernel.run_report.layer_cache.fixture".to_string()),
        precision: Some("exact".to_string()),
        status: Some(crate::eval::model::ObservedStatus::Present),
    })
}

#[cfg(test)]
fn runtime_budget_name(expected: &[crate::eval::model::ExpectedItem], case_id: &str) -> String {
    expected
        .iter()
        .find_map(|item| match item {
            crate::eval::model::ExpectedItem::RuntimeBudget(budget) => Some(budget.name.clone()),
            _ => None,
        })
        .unwrap_or_else(|| case_id.to_string())
}

#[cfg(test)]
fn bool_string(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

#[cfg(test)]
fn observe_fixture_runtime_budget(
    fixture: &NativeFixture,
    elapsed: std::time::Duration,
) -> crate::eval::model::ObservedRuntimeBudget {
    let budget = fixture
        .manifest
        .budget
        .as_ref()
        .expect("observe_fixture_runtime_budget requires a fixture budget");
    let peak_rss_bytes = crate::measure::peak_rss_bytes();
    let runtime_ms = saturating_millis(elapsed);
    let runtime_ok = runtime_ms <= budget.max_runtime_ms;
    let rss_ok = budget
        .max_peak_rss_bytes
        .is_none_or(|max| peak_rss_bytes <= max);
    crate::eval::model::ObservedRuntimeBudget {
        name: runtime_budget_name(&fixture.manifest.expected, &fixture.manifest.case_id),
        budget_passed: runtime_ok && rss_ok,
        observed_runtime_ms: Some(runtime_ms),
        peak_rss_bytes: Some(peak_rss_bytes),
    }
}

#[cfg(test)]
fn saturating_millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
fn runtime_observation(
    manifest: &NativeFixtureManifest,
    observed: &[crate::eval::model::ObservedItem],
) -> crate::eval::report::RuntimeObservation {
    let runtime_budget = observed.iter().find_map(|item| match item {
        crate::eval::model::ObservedItem::RuntimeBudget(budget) => Some(budget),
        _ => None,
    });
    crate::eval::report::RuntimeObservation {
        budget_name: runtime_budget
            .map(|budget| budget.name.clone())
            .unwrap_or_else(|| manifest.case_id.clone()),
        budget_passed: runtime_budget
            .map(|budget| budget.budget_passed)
            .unwrap_or(true),
        observed_runtime_ms: runtime_budget.and_then(|budget| budget.observed_runtime_ms),
        peak_rss_bytes: runtime_budget.and_then(|budget| budget.peak_rss_bytes),
    }
}

#[cfg(test)]
mod eval_fixture_manifest_tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::eval::model::{ExpectedItem, FixtureArea};

    fn write_fixture(root: &Path, manifest: &str) {
        fs::create_dir_all(root.join("repo/src")).unwrap();
        fs::write(root.join("repo/src/app.ts"), "export const answer = 42;\n").unwrap();
        fs::write(root.join("expected.polint-eval.toml"), manifest).unwrap();
    }

    #[test]
    fn eval_fixture_manifest_loads_native_fixture_from_expected_toml() {
        let temp = tempfile::tempdir().unwrap();
        let fixture_dir = temp
            .path()
            .join("tests/eval-fixtures/kernel/provider-order");
        write_fixture(
            &fixture_dir,
            r#"
schema_version = "polint-eval-fixture-1"
case_id = "provider-order"
area = "kernel"

[repo]
path = "repo"

[[expected]]
invariant = { name = "provider_order.0", value = "polint.source", mode = "exact" }
"#,
        );

        let fixture = load_native_fixture(&fixture_dir).unwrap();

        assert_eq!(fixture.manifest.schema_version, "polint-eval-fixture-1");
        assert_eq!(fixture.manifest.area, FixtureArea::Kernel);
        assert_eq!(fixture.manifest.case_id, "provider-order");
        assert_eq!(fixture.manifest.repo.path, "repo");
        assert_eq!(fixture.manifest.expected.len(), 1);
        assert!(matches!(
            fixture.manifest.expected.first(),
            Some(ExpectedItem::Invariant(_))
        ));
        assert_eq!(fixture.repo_dir, fixture.fixture_dir.join("repo"));
    }

    #[test]
    fn eval_fixture_manifest_rejects_absolute_repo_path() {
        let temp = tempfile::tempdir().unwrap();
        let fixture_dir = temp
            .path()
            .join("tests/eval-fixtures/kernel/provider-order");
        write_fixture(
            &fixture_dir,
            r#"
schema_version = "polint-eval-fixture-1"
case_id = "provider-order"
area = "kernel"

[repo]
path = "/tmp/polint-outside"
"#,
        );

        let err = load_native_fixture(&fixture_dir).unwrap_err();

        assert!(err.to_string().contains("absolute"));
    }

    #[test]
    fn eval_fixture_manifest_rejects_parent_repo_path() {
        let temp = tempfile::tempdir().unwrap();
        let fixture_dir = temp
            .path()
            .join("tests/eval-fixtures/kernel/provider-order");
        fs::create_dir_all(&fixture_dir).unwrap();
        fs::write(
            fixture_dir.join("expected.polint-eval.toml"),
            r#"
schema_version = "polint-eval-fixture-1"
case_id = "provider-order"
area = "kernel"

[repo]
path = "../outside"
"#,
        )
        .unwrap();

        let err = load_native_fixture(&fixture_dir).unwrap_err();

        assert!(err.to_string().contains("parent directory"));
    }

    #[test]
    fn eval_fixture_manifest_rejects_parent_expected_relative_path() {
        let temp = tempfile::tempdir().unwrap();
        let fixture_dir = temp
            .path()
            .join("tests/eval-fixtures/kernel/provider-order");
        write_fixture(
            &fixture_dir,
            r#"
schema_version = "polint-eval-fixture-1"
case_id = "provider-order"
area = "kernel"

[repo]
path = "repo"

[[expected]]
diagnostic = { rule_id = "local/rule", relative_path = "../outside.ts", line = 1, mode = "exact" }
"#,
        );

        let err = load_native_fixture(&fixture_dir).unwrap_err();

        assert!(err.to_string().contains("parent directory"));
    }

    #[test]
    fn eval_fixture_manifest_rejects_absolute_expected_relative_path() {
        let temp = tempfile::tempdir().unwrap();
        let fixture_dir = temp
            .path()
            .join("tests/eval-fixtures/kernel/provider-order");
        write_fixture(
            &fixture_dir,
            r#"
schema_version = "polint-eval-fixture-1"
case_id = "provider-order"
area = "kernel"

[repo]
path = "repo"

[[expected]]
diagnostic = { rule_id = "local/rule", relative_path = "/tmp/app.ts", line = 1, mode = "exact" }
"#,
        );

        let err = load_native_fixture(&fixture_dir).unwrap_err();

        assert!(err.to_string().contains("absolute"));
    }

    #[test]
    fn eval_fixture_manifest_normalizes_expected_relative_path_separators() {
        let temp = tempfile::tempdir().unwrap();
        let fixture_dir = temp
            .path()
            .join("tests/eval-fixtures/kernel/provider-order");
        write_fixture(
            &fixture_dir,
            r#"
schema_version = "polint-eval-fixture-1"
case_id = "provider-order"
area = "kernel"

[repo]
path = "repo"

[[expected]]
diagnostic = { rule_id = "local/rule", relative_path = "src\\app.ts", line = 1, mode = "exact" }
"#,
        );

        let fixture = load_native_fixture(&fixture_dir).unwrap();

        assert!(matches!(
            fixture.manifest.expected.first(),
            Some(ExpectedItem::Diagnostic(diagnostic)) if diagnostic.relative_path == "src/app.ts"
        ));
    }

    #[test]
    fn eval_fixture_manifest_rejects_alternate_fact_producer_field_name() {
        let temp = tempfile::tempdir().unwrap();
        let fixture_dir = temp.path().join("tests/eval-fixtures/provenance/metadata");
        write_fixture(
            &fixture_dir,
            r#"
schema_version = "polint-eval-fixture-1"
case_id = "provenance-metadata"
area = "provenance"

[repo]
path = "repo"

[[expected]]
fact = { family = "SourceFile", stable_key = "src/app.ts", mode = "partial", provider_id = "polint.source", precision = "exact", status = "present" }
"#,
        );

        let err = load_native_fixture(&fixture_dir).unwrap_err();
        let rendered = format!("{err:#}");

        assert!(
            rendered.contains("provider_id"),
            "alternate producer field should be rejected: {rendered}"
        );
    }

    #[test]
    fn eval_fixture_manifest_rejects_synthetic_observed_rows_outside_allowed_areas() {
        let temp = tempfile::tempdir().unwrap();
        let fixture_dir = temp.path().join("tests/eval-fixtures/kernel/synthetic");
        write_fixture(
            &fixture_dir,
            r#"
schema_version = "polint-eval-fixture-1"
case_id = "kernel-synthetic"
area = "kernel"
synthetic_observed = true

[repo]
path = "repo"

[[observed]]
invariant = { name = "kernel.synthetic", value = "true", mode = "exact", producer_id = "polint.eval", provenance = "synthetic_observed", precision = "exact", status = "present" }
"#,
        );

        let err = load_native_fixture(&fixture_dir).unwrap_err();
        let rendered = format!("{err:#}");

        assert!(
            rendered.contains(
                "synthetic observed rows are only allowed for extension, refined-call, or promotion fixtures"
            ),
            "synthetic observed rows outside allowed areas should be rejected: {rendered}"
        );
    }
}

#[cfg(test)]
mod eval_native_fixture_runner_tests {
    use std::path::{Path, PathBuf};

    use crate::eval::model::{
        ExpectedItem, FixtureArea, ObservedFact, ObservedItem, ObservedStatus,
    };
    use crate::eval::report::{deterministic_output_hash, to_deterministic_json_pretty};

    use super::*;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("polint crate should live under crates/")
            .to_path_buf()
    }

    fn provider_order_fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/kernel/provider-order")
    }

    fn provenance_fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/provenance/metadata")
    }

    fn cache_current_determinism_fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/cache/current-determinism")
    }

    fn cache_input_snapshots_fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/cache/input-snapshots")
    }

    fn cache_layer_cache_fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/cache/layer-cache")
    }

    fn extension_rejection_delta_fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/extension/rejection-delta")
    }

    fn evidence_fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/evidence")
    }

    #[test]
    fn eval_native_fixture_runner_evidence_fixture_passes() {
        let run = run_native_fixture_for_test(&evidence_fixture_dir()).unwrap();
        let case = run.cases.first().expect("evidence case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(case.area, FixtureArea::Evidence);
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert!(
            rendered.contains("EvidenceNode")
                && rendered.contains("EvidenceEdge")
                && rendered.contains("EvidenceBundle")
                && rendered.contains("EvidencePath")
                && rendered.contains("evidence.debug.counts.nodes")
                && rendered.contains("evidence.debug.counts.paths"),
            "evidence fixture must be backed by observed kernel evidence/debug rows: {rendered}"
        );
        assert!(!rendered.contains("/Users/"), "{rendered}");
        assert!(!rendered.contains("raw source"), "{rendered}");
    }

    #[test]
    fn eval_evidence_extension_fixture_passes() {
        let observed = synthetic_evidence_observed_rows();

        assert!(observed.iter().any(|item| matches!(
            item,
            ObservedItem::Fact(fact)
                if fact.stable_key == "evidence_extension_delta:accepted"
                    && fact.payload.as_deref() == Some("1")
        )));
        assert!(observed.iter().any(|item| matches!(
            item,
            ObservedItem::Fact(fact)
                if fact.stable_key == "evidence_extension_delta:downgraded"
                    && fact.payload.as_deref() == Some("1")
        )));
        assert!(observed.iter().any(|item| matches!(
            item,
            ObservedItem::Fact(fact)
                if fact.stable_key == "evidence_extension_delta:rejected"
                    && fact.payload.as_deref() == Some("1")
        )));
        assert!(observed.iter().any(|item| matches!(
            item,
            ObservedItem::Fact(fact)
                if fact.stable_key == "evidence_extension_delta:native_edge_count"
                    && fact.payload.as_deref() == Some("2")
        )));
    }

    #[test]
    fn eval_evidence_manifest_requires_kernel_observed_evidence_counts() {
        let fixture = load_native_fixture(&evidence_fixture_dir()).unwrap();
        assert_eq!(fixture.manifest.area, FixtureArea::Evidence);
        assert!(
            !fixture.manifest.synthetic_observed,
            "evidence fixture must use kernel observation, not synthetic self-matching rows"
        );

        assert!(fixture.manifest.expected.iter().any(|item| matches!(
            item,
            ExpectedItem::Fact(fact)
                if fact.family == "EvidenceNode"
                    && fact.stable_key == "evidence.debug.counts.nodes"
        )));
        assert!(fixture.manifest.expected.iter().any(|item| matches!(
            item,
            ExpectedItem::Fact(fact)
                if fact.family == "EvidenceEdge"
                    && fact.stable_key == "evidence.debug.counts.edges"
        )));
        assert!(fixture.manifest.expected.iter().any(|item| matches!(
            item,
            ExpectedItem::Invariant(invariant)
                if invariant.name == "evidence.debug.counts.nodes.nonzero"
        )));
        assert!(fixture.manifest.expected.iter().any(|item| matches!(
            item,
            ExpectedItem::Invariant(invariant)
                if invariant.name == "evidence.debug.counts.edges.nonzero"
        )));
    }

    fn synthetic_evidence_observed_rows() -> Vec<ObservedItem> {
        let mut rows = vec![
            ObservedItem::Path(crate::eval::model::ObservedPath {
                path_id: "evidence.path.local_dependence".to_string(),
                nodes: vec!["source".to_string(), "sink".to_string()],
                mode: crate::eval::model::AssertionMode::Partial,
                partial_truth: true,
                producer_id: Some("polint.evidence".to_string()),
                provenance: Some("native".to_string()),
                precision: Some("conservative".to_string()),
                status: Some(ObservedStatus::Partial),
            }),
            ObservedItem::Path(crate::eval::model::ObservedPath {
                path_id: "evidence.path.source_to_sink".to_string(),
                nodes: vec![
                    "source".to_string(),
                    "summary".to_string(),
                    "sink".to_string(),
                ],
                mode: crate::eval::model::AssertionMode::Partial,
                partial_truth: true,
                producer_id: Some("polint.evidence".to_string()),
                provenance: Some("summary".to_string()),
                precision: Some("setup_aware".to_string()),
                status: Some(ObservedStatus::Partial),
            }),
            ObservedItem::Fact(ObservedFact {
                family: "evidence_debug".to_string(),
                stable_key: "evidence.debug.hidden_nodes".to_string(),
                mode: crate::eval::model::AssertionMode::Exact,
                producer_id: Some("polint.evidence".to_string()),
                provenance: Some("renderer".to_string()),
                precision: Some("debug".to_string()),
                status: Some(ObservedStatus::Present),
                payload: Some("hidden_node_count=3;replay_key=replay:bundle".to_string()),
            }),
        ];
        rows.extend(
            crate::eval::observed::observed_extension_evidence_delta_rows(
                &crate::analysis::evidence::validate::ExtensionEvidenceDelta {
                    native_edge_count: 2,
                    accepted: 1,
                    downgraded: 1,
                    candidate_only: 1,
                    rejected: 1,
                    representative_reasons: vec![
                        "ExactClaimRequiresNativeAnchor".to_string(),
                        "InvalidEndpoint".to_string(),
                    ],
                },
            ),
        );
        rows
    }

    fn extension_real_sink_fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/extension/real-sink")
    }

    fn type_value_alias_extension_precision_fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/type-value-alias/extension-precision")
    }

    fn type_value_alias_go_core_fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/type-value-alias/go-core")
    }

    fn type_value_alias_ts_js_core_fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/type-value-alias/ts-js-core")
    }

    fn refined_calls_direct_vs_refined_fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/refined-calls/direct-vs-refined")
    }

    fn refined_calls_extension_model_fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/refined-calls/extension-model")
    }

    fn data_flow_core_fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/data-flow/core")
    }

    #[test]
    fn eval_native_fixture_runner_provider_order_fixture_passes() {
        let run = run_native_fixture_for_test(&provider_order_fixture_dir()).unwrap();

        assert_eq!(run.metrics.false_negatives, 0);
        assert_eq!(run.metrics.forbidden_hits, 0);
    }

    #[test]
    fn eval_native_fixture_runner_manifest_asserts_all_provider_order_invariants() {
        let fixture = load_native_fixture(&provider_order_fixture_dir()).unwrap();
        let invariants = fixture
            .manifest
            .expected
            .iter()
            .filter_map(|item| match item {
                ExpectedItem::Invariant(invariant) => {
                    Some((invariant.name.as_str(), invariant.value.as_str()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            invariants,
            vec![
                ("provider_order.0", "polint.source"),
                ("provider_order.1", "polint.go.syntax"),
                ("provider_order.2", "polint.ts.syntax"),
                ("provider_order.3", "polint.module_graph"),
                ("provider_order.4", "polint.symbol_graph"),
                ("provider_order.5", "polint.module_topology"),
                ("provider_order.6", "polint.semantic_mir"),
                ("provider_order.7", "polint.cfg"),
                ("provider_order.8", "polint.calls"),
                ("provider_order.9", "polint.go.semantic"),
                ("provider_order.10", "polint.ts.types"),
                ("provider_order.11", "polint.identity"),
                ("provider_order.12", "polint.abstract_domains"),
                ("provider_order.13", "polint.direct_summaries"),
                ("provider_order.14", "polint.entrypoints"),
                ("provider_order.15", "polint.reachability"),
                ("provider_order.16", "polint.extensions"),
                ("provider_order.17", "polint.type_value_alias"),
                ("provider_order.18", "polint.semantic_graph"),
                ("provider_order.19", "polint.solver"),
                ("provider_order.20", "polint.refined_calls"),
                ("provider_order.21", "polint.data_flow"),
                ("provider_order.22", "polint.evidence"),
                ("provider_order.23", "polint.metrics"),
                (
                    "provider_output.polint.abstract_domains.schema_version",
                    "abstract-domain-facts-1:1",
                ),
            ]
        );
    }

    #[test]
    fn eval_native_fixture_runner_repeated_runs_produce_identical_json() {
        let first = run_native_fixture_for_test(&provider_order_fixture_dir()).unwrap();
        let second = run_native_fixture_for_test(&provider_order_fixture_dir()).unwrap();

        assert_eq!(
            to_deterministic_json_pretty(&first),
            to_deterministic_json_pretty(&second)
        );
    }

    #[test]
    fn eval_native_fixture_runner_repeated_runs_produce_identical_output_hash() {
        let first = run_native_fixture_for_test(&provider_order_fixture_dir()).unwrap();
        let second = run_native_fixture_for_test(&provider_order_fixture_dir()).unwrap();

        assert_eq!(first.output_hash, second.output_hash);
    }

    #[test]
    fn eval_provenance_fixture_passes() {
        let run = run_native_fixture_for_test(&provenance_fixture_dir()).unwrap();
        let case = run.cases.first().expect("provenance case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(run.metrics.false_negatives, 0);
        assert_eq!(run.metrics.runtime_budget_failed, 0);
        assert!(case.observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.family == "SourceFile"
                    && fact.stable_key.contains("src/app.ts")
                    && fact.producer_id.as_deref() == Some("polint.source")
                    && fact.precision.as_deref() == Some("exact")
            }
            _ => false,
        }));
        assert!(case.observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.family == "Import"
                    && fact.stable_key.contains("./message")
                    && fact.producer_id.as_deref() == Some("polint.ts.syntax")
                    && fact.precision.as_deref() == Some("syntax")
            }
            _ => false,
        }));
        assert!(!rendered.contains(repo_root().to_string_lossy().as_ref()));
    }

    #[test]
    fn eval_provenance_fixture_expected_facts_use_producer_id_fields() {
        let fixture = load_native_fixture(&provenance_fixture_dir()).unwrap();
        let manifest =
            std::fs::read_to_string(provenance_fixture_dir().join("expected.polint-eval.toml"))
                .unwrap();
        let expected_facts = fixture
            .manifest
            .expected
            .iter()
            .filter_map(|item| match item {
                ExpectedItem::Fact(fact) => Some((
                    fact.family.as_str(),
                    fact.producer_id.as_deref(),
                    fact.precision.as_deref(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            expected_facts,
            vec![
                ("SourceFile", Some("polint.source"), Some("exact")),
                ("Import", Some("polint.ts.syntax"), Some("syntax")),
            ]
        );
        assert!(manifest.contains("producer_id"));
        assert!(!manifest.contains("provider_id"));
    }

    #[test]
    fn eval_provenance_fixture_runtime_budget_hash_ignores_duration() {
        let mut first = run_native_fixture_for_test(&provenance_fixture_dir()).unwrap();
        let mut second = first.clone();
        first.cases[0].runtime.observed_runtime_ms = Some(1);
        second.cases[0].runtime.observed_runtime_ms = Some(999);
        for run in [&mut first, &mut second] {
            let observed_runtime_ms = run.cases[0].runtime.observed_runtime_ms;
            for item in &mut run.cases[0].observed {
                if let ObservedItem::RuntimeBudget(budget) = item {
                    budget.observed_runtime_ms = observed_runtime_ms;
                }
            }
        }

        assert_eq!(
            deterministic_output_hash(&first),
            deterministic_output_hash(&second)
        );
    }

    #[test]
    fn eval_cache_current_determinism_fixture_passes() {
        let run = run_cache_current_determinism_fixture_for_test(
            &cache_current_determinism_fixture_dir(),
        )
        .unwrap();
        let case = run.cases.first().expect("cache determinism case");

        assert_eq!(run.metrics.false_negatives, 0);
        assert_eq!(run.metrics.runtime_budget_failed, 0);
        assert!(case.observed.iter().any(|item| match item {
            ObservedItem::Invariant(invariant) => {
                invariant.name == "cache.current_determinism"
                    && invariant.value == "cold_warm_no_cache_equal"
            }
            _ => false,
        }));
    }

    #[test]
    fn eval_cache_current_determinism_manifest_limits_cache_claims() {
        let manifest = std::fs::read_to_string(
            cache_current_determinism_fixture_dir().join("expected.polint-eval.toml"),
        )
        .unwrap();

        assert!(manifest.contains("cache.current_determinism"));
        assert!(manifest.contains("cold_warm_no_cache_equal"));
        assert!(manifest.contains("max_runtime_ms = 5000"));
        for forbidden in [
            concat!("Input", "Snapshot"),
            concat!("Layer", "Key"),
            concat!("Query", "Key"),
            concat!("Summary", "Key"),
            concat!("Diagnostic", "Key"),
        ] {
            assert!(
                !manifest.contains(forbidden),
                "cache fixture must not claim future cache term `{forbidden}`"
            );
        }
    }

    #[test]
    fn eval_cache_current_determinism_runtime_budget_hash_ignores_duration() {
        let mut first = run_cache_current_determinism_fixture_for_test(
            &cache_current_determinism_fixture_dir(),
        )
        .unwrap();
        let mut second = first.clone();
        first.cases[0].runtime.observed_runtime_ms = Some(1);
        second.cases[0].runtime.observed_runtime_ms = Some(999);
        for run in [&mut first, &mut second] {
            let observed_runtime_ms = run.cases[0].runtime.observed_runtime_ms;
            for item in &mut run.cases[0].observed {
                if let ObservedItem::RuntimeBudget(budget) = item {
                    budget.observed_runtime_ms = observed_runtime_ms;
                }
            }
        }

        assert_eq!(
            deterministic_output_hash(&first),
            deterministic_output_hash(&second)
        );
    }

    #[test]
    fn eval_input_snapshot_fixture_passes() {
        let run = run_native_fixture_for_test(&cache_input_snapshots_fixture_dir()).unwrap();
        let case = run.cases.first().expect("input snapshot case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(case.case_id, "input-snapshots");
        assert_eq!(case.area, FixtureArea::Cache);
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
        assert!(!rendered.contains(repo_root().to_string_lossy().as_ref()));
        assert!(!rendered.contains("package payment"));
        assert!(!rendered.contains("export const amount"));
        assert!(!rendered.contains("mtime_hint"));
    }

    #[test]
    fn eval_layer_cache_fixture_passes() {
        let run = run_layer_cache_fixture_for_test(&cache_layer_cache_fixture_dir()).unwrap();
        let case = run.cases.first().expect("layer cache case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(case.case_id, "layer-cache");
        assert_eq!(case.area, FixtureArea::Cache);
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
        assert!(!rendered.contains(repo_root().to_string_lossy().as_ref()));
        assert!(!rendered.contains("package payment"));
        assert!(!rendered.contains("export function charge"));
    }

    #[test]
    fn eval_extension_synthetic_rejection_delta_fixture_passes() {
        let run = run_native_fixture_for_test(&extension_rejection_delta_fixture_dir()).unwrap();
        let case = run.cases.first().expect("extension rejection-delta case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(run.metrics.false_negatives, 0);
        assert_eq!(run.metrics.runtime_budget_failed, 0);
        assert_eq!(run.metrics.false_positive_trap_hits, 1);
        assert!(rendered.contains("\"facts_accepted\": 1"));
        assert!(rendered.contains("\"facts_rejected\": 1"));
        assert!(case.observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.stable_key == "extension.synthetic_accepted_fact"
                    && fact.status == Some(ObservedStatus::Accepted)
            }
            _ => false,
        }));
        assert!(case.observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.stable_key == "extension.synthetic_rejected_fact"
                    && fact.status == Some(ObservedStatus::Rejected)
            }
            _ => false,
        }));
    }

    #[test]
    fn eval_extension_synthetic_fixture_covers_all_item_kinds() {
        let run = run_native_fixture_for_test(&extension_rejection_delta_fixture_dir()).unwrap();
        let case = run.cases.first().expect("extension rejection-delta case");

        assert_eq!(
            item_kinds(&case.expected),
            vec![
                "Diagnostic",
                "Fact",
                "GraphEdge",
                "Invariant",
                "Path",
                "RuntimeBudget",
            ]
        );
        assert_eq!(
            observed_item_kinds(&case.observed),
            vec![
                "Diagnostic",
                "Fact",
                "GraphEdge",
                "Invariant",
                "Path",
                "RuntimeBudget",
            ]
        );
    }

    #[test]
    fn eval_extension_synthetic_delta_uses_normalized_invariants() {
        let run = run_native_fixture_for_test(&extension_rejection_delta_fixture_dir()).unwrap();
        let case = run.cases.first().expect("extension rejection-delta case");

        assert!(case.observed.iter().any(|item| match item {
            ObservedItem::Invariant(invariant) => {
                invariant.name == "extension.default_vs_extension_delta"
                    && invariant.value == "changed_facts=2,rejected=1"
            }
            _ => false,
        }));
        assert!(case.observed.iter().any(|item| match item {
            ObservedItem::Invariant(invariant) => {
                invariant.name == "extension.real_sink_active" && invariant.value == "false"
            }
            _ => false,
        }));
    }

    #[test]
    #[cfg_attr(
        target_os = "windows",
        ignore = "extension fixture requires cargo build at runtime, which is unreliable on Windows CI"
    )]
    fn eval_extension_real_sink_fixture_passes() {
        let run = run_native_fixture_for_test(&extension_real_sink_fixture_dir()).unwrap();
        let case = run.cases.first().expect("extension real-sink case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(case.case_id, "extension-real-sink");
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
        assert!(rendered.contains("\"facts_accepted\": 1"), "{rendered}");
        assert!(rendered.contains("\"facts_rejected\": 1"), "{rendered}");
        assert!(case.observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.stable_key == "extension.route./ok"
                    && fact.status == Some(ObservedStatus::Accepted)
                    && fact.producer_id.as_deref() == Some("polint.extension.demo.routes")
            }
            _ => false,
        }));
        assert!(case.observed.iter().any(|item| match item {
            ObservedItem::Fact(fact) => {
                fact.stable_key == "extension.route./rejected"
                    && fact.status == Some(ObservedStatus::Rejected)
                    && fact.producer_id.as_deref() == Some("polint.extension.demo.routes")
            }
            _ => false,
        }));
        assert!(case.observed.iter().any(|item| match item {
            ObservedItem::Invariant(invariant) => {
                invariant.name == "extension.real_sink_active" && invariant.value == "true"
            }
            _ => false,
        }));
    }

    #[test]
    #[cfg_attr(
        target_os = "windows",
        ignore = "extension fixture requires cargo build at runtime, which is unreliable on Windows CI"
    )]
    fn eval_type_value_alias_extension_precision_fixture_passes() {
        let run = run_native_fixture_for_test(&type_value_alias_extension_precision_fixture_dir())
            .unwrap();
        let case = run
            .cases
            .first()
            .expect("type/value/alias extension precision case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(case.case_id, "type-value-alias-extension-precision");
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
        assert!(
            case.observed.iter().any(|item| match item {
                ObservedItem::Fact(fact) => {
                    fact.family == "AliasAnswer"
                        && fact.stable_key == "alias:extension:no_alias"
                        && fact.producer_id.as_deref() == Some("polint.type_value_alias")
                        && fact.precision.as_deref() == Some("heuristic")
                }
                _ => false,
            }),
            "{rendered}"
        );
    }

    #[test]
    fn eval_type_value_alias_go_core_fixture_passes() {
        let run = run_native_fixture_for_test(&type_value_alias_go_core_fixture_dir()).unwrap();
        let case = run.cases.first().expect("type/value/alias Go core case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(case.case_id, "type-value-alias-go-core");
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
    }

    #[test]
    fn eval_type_value_alias_ts_js_core_fixture_passes() {
        let run = run_native_fixture_for_test(&type_value_alias_ts_js_core_fixture_dir()).unwrap();
        let case = run.cases.first().expect("type/value/alias TS/JS core case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(case.case_id, "type-value-alias-ts-js-core");
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
    }

    #[test]
    fn eval_native_fixture_runner_refined_calls_fixture_passes() {
        for fixture_dir in [
            refined_calls_direct_vs_refined_fixture_dir(),
            refined_calls_extension_model_fixture_dir(),
        ] {
            if cfg!(target_os = "windows") && fixture_requires_runtime_extension(&fixture_dir) {
                continue;
            }

            let run = run_native_fixture_for_test(&fixture_dir).unwrap();
            let case = run.cases.first().expect("refined calls case");
            let rendered = to_deterministic_json_pretty(&run);

            assert_eq!(case.area, FixtureArea::RefinedCalls);
            assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
            assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
            if !windows_runtime_budget_is_uncalibrated(&run) {
                assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
            }
            assert!(!rendered.contains(repo_root().to_string_lossy().as_ref()));
        }
    }

    #[test]
    fn eval_refined_calls_manifests_cover_required_taxonomy() {
        let direct = load_native_fixture(&refined_calls_direct_vs_refined_fixture_dir()).unwrap();
        let extension = load_native_fixture(&refined_calls_extension_model_fixture_dir()).unwrap();

        assert_eq!(direct.manifest.area, FixtureArea::RefinedCalls);
        assert_eq!(extension.manifest.area, FixtureArea::RefinedCalls);

        let direct_expected = direct
            .manifest
            .expected
            .iter()
            .map(expected_identity)
            .collect::<std::collections::BTreeSet<_>>();
        let extension_expected = extension
            .manifest
            .expected
            .iter()
            .map(expected_identity)
            .collect::<std::collections::BTreeSet<_>>();

        assert!(
            direct_expected
                .iter()
                .any(|identity| identity.contains("Fact:RefinedCallEdge")),
            "direct refined-call fixture must assert refined edges"
        );
        assert!(
            direct_expected.iter().any(|identity| identity
                .contains("Invariant:refined_calls.counts.by_tier.direct_only.nonzero")),
            "direct refined-call fixture must assert direct-only tier counts"
        );
        assert!(
            direct_expected.iter().any(|identity| identity
                .contains("Invariant:refined_calls.counts.by_tier.points_to_assisted.nonzero")),
            "direct refined-call fixture must assert solver-assisted tier counts"
        );
        assert!(
            direct_expected.iter().any(|identity| identity
                .contains("Invariant:refined_calls.deltas.changed_edges.nonzero")),
            "direct refined-call fixture must assert solver projection changes refined calls"
        );
        assert!(
            direct_expected.iter().any(|identity| identity
                .contains("Invariant:refined_calls.counts.by_language.Go.nonzero")),
            "direct refined-call fixture must assert Go refined-call coverage"
        );
        assert!(
            extension_expected
                .iter()
                .any(|identity| identity.contains("Fact:refined_calls.edge:extension:refined-ok")),
            "extension refined-call fixture must assert accepted model fact"
        );
        assert!(
            extension_expected
                .iter()
                .any(|identity| identity
                    .contains("Fact:refined_calls.edge:extension:refined-rejected")),
            "extension refined-call fixture must assert rejected malformed model fact"
        );
        assert!(
            extension.manifest.expected.iter().any(|item| {
                matches!(
                    item,
                    ExpectedItem::Invariant(invariant)
                        if invariant.name == "refined_calls.deltas.extension_model_edges.nonzero"
                            && invariant.value == "true"
                )
            }),
            "extension refined-call fixture must assert extension models project to refined calls"
        );
    }

    #[test]
    fn eval_native_fixture_runner_data_flow_fixture_passes() {
        let run = run_native_fixture_for_test(&data_flow_core_fixture_dir()).unwrap();
        let case = run.cases.first().expect("data-flow case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(case.area, FixtureArea::DataFlow);
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
    }

    #[test]
    fn eval_data_flow_fixture_uses_kernel_observed_rows() {
        let fixture = load_native_fixture(&data_flow_core_fixture_dir()).unwrap();

        assert!(
            !fixture.manifest.synthetic_observed,
            "data-flow fixture must be observed from the kernel, not synthetic self-matching rows"
        );
        assert!(
            fixture.manifest.observed.is_empty(),
            "data-flow fixture should not carry manifest observed rows when kernel-observed"
        );
    }

    #[test]
    fn eval_data_flow_manifests_cover_required_taxonomy() {
        let fixture = load_native_fixture(&data_flow_core_fixture_dir()).unwrap();
        assert_eq!(fixture.manifest.area, FixtureArea::DataFlow);

        let expected = fixture
            .manifest
            .expected
            .iter()
            .map(expected_identity)
            .collect::<std::collections::BTreeSet<_>>();

        for marker in [
            "Fact:DataFlowEdge:data-flow:local",
            "Fact:DataFlowEdge:data-flow:direct-call",
            "Fact:DataFlowEdge:data-flow:summary-projected",
            "Fact:DataFlowEdge:data-flow:unknown",
        ] {
            assert!(
                expected.iter().any(|identity| identity.contains(marker)),
                "data-flow fixture must cover taxonomy marker {marker}; got {expected:#?}"
            );
        }
    }

    #[test]
    fn eval_native_fixture_suite_covers_required_categories() {
        let fixture_root = repo_root().join("tests/eval-fixtures");
        let fixture_dirs = collect_native_fixture_dirs(&fixture_root);
        let discovered_fixture_dirs = fixture_dirs
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let declared_fixture_dirs = NATIVE_FIXTURE_DIRS
            .iter()
            .map(|relative_path| fixture_root.join(relative_path))
            .collect::<std::collections::BTreeSet<_>>();

        assert!(
            !fixture_dirs.is_empty(),
            "native fixture suite should contain fixture manifests"
        );
        assert_eq!(
            discovered_fixture_dirs, declared_fixture_dirs,
            "every native fixture directory should have an independently schedulable test"
        );

        let mut fixtures_by_area = std::collections::BTreeMap::<FixtureArea, Vec<String>>::new();

        for fixture_dir in fixture_dirs {
            if cfg!(target_os = "windows") && fixture_requires_runtime_extension(&fixture_dir) {
                continue;
            }
            let fixture = load_native_fixture(&fixture_dir).unwrap_or_else(|error| {
                panic!(
                    "native fixture manifest should load: {}\n{error:#}",
                    fixture_dir.display()
                )
            });

            fixtures_by_area
                .entry(fixture.manifest.area)
                .or_default()
                .push(fixture.manifest.case_id);
        }

        for required_area in [
            FixtureArea::Kernel,
            FixtureArea::Provenance,
            FixtureArea::Cache,
            FixtureArea::Extension,
            FixtureArea::RefinedCalls,
            FixtureArea::DataFlow,
            FixtureArea::Evidence,
        ] {
            assert!(
                fixtures_by_area.contains_key(&required_area),
                "native fixture suite must include a {required_area:?} fixture; found {fixtures_by_area:#?}"
            );
        }
    }

    /// The runtime-budget thresholds in the fixture manifests are calibrated on
    /// Linux and macOS with a warm Go build cache. Hosted Windows runners start
    /// every job with a partially cold `GOCACHE`, run a debug-profile binary on
    /// two cores, and scan every freshly written executable, so the same fixture
    /// legitimately measures two to three times its Linux wall clock. The
    /// observed budget rows are still emitted (and hashed) on Windows; only the
    /// hard pass/fail gate is skipped there. Cache-warmth flakes proved this:
    /// the same fixture failed at 12.7 s and passed seconds later inside one
    /// job once the Go cache warmed.
    fn windows_runtime_budget_is_uncalibrated(run: &crate::eval::report::EvaluationRun) -> bool {
        if !cfg!(target_os = "windows") {
            return false;
        }
        run.cases.iter().any(|case| {
            case.observed
                .iter()
                .any(|item| matches!(item, crate::eval::model::ObservedItem::RuntimeBudget(_)))
        })
    }

    fn assert_native_fixture_passes(fixture_dir: &Path) {
        if cfg!(target_os = "windows") && fixture_requires_runtime_extension(fixture_dir) {
            return;
        }

        let run = run_fixture_for_suite_coverage(fixture_dir).unwrap_or_else(|error| {
            panic!(
                "native fixture should run: {}\n{error:#}",
                fixture_dir.display()
            )
        });
        let case = run
            .cases
            .first()
            .expect("native fixture run should have a case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(
            run.metrics.false_negatives, 0,
            "fixture should not miss expected rows: {}\n{rendered}",
            case.case_id
        );
        assert_eq!(
            run.metrics.forbidden_hits, 0,
            "fixture should not hit forbidden rows: {}\n{rendered}",
            case.case_id
        );
        if !windows_runtime_budget_is_uncalibrated(&run) {
            assert_eq!(
                run.metrics.runtime_budget_failed, 0,
                "fixture should stay inside runtime budget: {}\n{rendered}",
                case.case_id
            );
        }
        if case.area == FixtureArea::FrameworkEntrypoints {
            assert!(
                !rendered.contains(repo_root().to_string_lossy().as_ref()),
                "framework-entrypoint fixture output should not contain the repository root: {rendered}"
            );
        }
    }

    macro_rules! native_fixture_tests {
        ($($test_name:ident => $relative_path:literal),+ $(,)?) => {
            const NATIVE_FIXTURE_DIRS: &[&str] = &[$($relative_path),+];

            $(
                #[test]
                fn $test_name() {
                    assert_native_fixture_passes(
                        &repo_root().join("tests/eval-fixtures").join($relative_path),
                    );
                }
            )+
        };
    }

    native_fixture_tests! {
        eval_native_fixture_abstract_domains_core_passes => "abstract-domains/core",
        eval_native_fixture_cache_current_determinism_passes => "cache/current-determinism",
        eval_native_fixture_cache_input_snapshots_passes => "cache/input-snapshots",
        eval_native_fixture_cache_layer_cache_passes => "cache/layer-cache",
        eval_native_fixture_cfg_core_passes => "cfg-core",
        eval_native_fixture_data_flow_core_passes => "data-flow/core",
        eval_native_fixture_determinism_go_reachable_passes => "determinism/go_reachable",
        eval_native_fixture_determinism_go_rta_passes => "determinism/go_rta",
        eval_native_fixture_determinism_ts_object_model_passes => "determinism/ts_object_model",
        eval_native_fixture_determinism_ts_reachable_passes => "determinism/ts_reachable",
        eval_native_fixture_determinism_ts_tokens_passes => "determinism/ts_tokens",
        eval_native_fixture_direct_calls_core_passes => "direct-calls/core",
        eval_native_fixture_direct_summaries_core_passes => "direct-summaries/core",
        eval_native_fixture_direct_summaries_scc_closure_passes => "direct-summaries/scc-closure",
        eval_native_fixture_evidence_passes => "evidence",
        eval_native_fixture_extension_adaptation_delta_passes => "extension/adaptation-delta",
        eval_native_fixture_extension_real_sink_passes => "extension/real-sink",
        eval_native_fixture_extension_rejection_delta_passes => "extension/rejection-delta",
        eval_native_fixture_framework_entrypoints_mixed_go_ts_passes => "framework-entrypoints/mixed-go-ts",
        eval_native_fixture_go_rta_address_taken_passes => "go-rta/address-taken",
        eval_native_fixture_go_rta_alias_dispatch_passes => "go-rta/alias-dispatch",
        eval_native_fixture_go_rta_alias_generic_dispatch_passes => "go-rta/alias-generic-dispatch",
        eval_native_fixture_go_rta_generic_dispatch_passes => "go-rta/generic-dispatch",
        eval_native_fixture_go_rta_interface_dispatch_passes => "go-rta/interface-dispatch",
        eval_native_fixture_go_rta_iteration_cap_passes => "go-rta/iteration-cap",
        eval_native_fixture_go_rta_static_reachability_passes => "go-rta/static-reachability",
        eval_native_fixture_identity_categorized_failures_passes => "identity/categorized_failures",
        eval_native_fixture_identity_crlf_normalization_passes => "identity/crlf_normalization",
        eval_native_fixture_identity_dedup_passes => "identity/dedup",
        eval_native_fixture_identity_jelly_oracle_coverage_passes => "identity/jelly_oracle_coverage",
        eval_native_fixture_jelly_ts_inventory_spans_passes => "jelly/ts-inventory-spans",
        eval_native_fixture_kernel_provider_order_passes => "kernel/provider-order",
        eval_native_fixture_module_topology_core_passes => "module-topology/core",
        eval_native_fixture_polyglot_canary_go_ts_passes => "polyglot-canary/go-ts",
        eval_native_fixture_promotion_cfg_call_flow_evidence_passes => "promotion/cfg-call-flow-evidence",
        eval_native_fixture_provenance_cycle_detection_passes => "provenance/cycle-detection",
        eval_native_fixture_provenance_metadata_passes => "provenance/metadata",
        eval_native_fixture_refined_calls_direct_vs_refined_passes => "refined-calls/direct-vs-refined",
        eval_native_fixture_refined_calls_extension_model_passes => "refined-calls/extension-model",
        eval_native_fixture_semantic_graph_go_graph_passes => "semantic-graph/go_graph",
        eval_native_fixture_semantic_graph_go_semantic_passes => "semantic-graph/go_semantic",
        eval_native_fixture_semantic_graph_ts_direct_bindings_passes => "semantic-graph/ts_direct_bindings",
        eval_native_fixture_semantic_graph_ts_graph_passes => "semantic-graph/ts_graph",
        eval_native_fixture_semantic_index_core_passes => "semantic-index/core",
        eval_native_fixture_semantic_mir_core_passes => "semantic-mir/core",
        eval_native_fixture_ts_object_model_budget_passes => "ts-object-model/budget",
        eval_native_fixture_ts_object_model_object_literal_passes => "ts-object-model/object-literal",
        eval_native_fixture_ts_object_model_prototype_this_passes => "ts-object-model/prototype-this",
        eval_native_fixture_ts_tokens_alias_parameter_return_passes => "ts-tokens/alias-parameter-return",
        eval_native_fixture_ts_tokens_token_explosion_passes => "ts-tokens/token-explosion",
        eval_native_fixture_type_value_alias_extension_precision_passes => "type-value-alias/extension-precision",
        eval_native_fixture_type_value_alias_go_core_passes => "type-value-alias/go-core",
        eval_native_fixture_type_value_alias_ts_js_core_passes => "type-value-alias/ts-js-core",
    }

    fn expected_identity(item: &ExpectedItem) -> String {
        match item {
            ExpectedItem::Diagnostic(diagnostic) => {
                format!("Diagnostic:{}", diagnostic.rule_id)
            }
            ExpectedItem::Fact(fact) => {
                format!("Fact:{}:{}", fact.family, fact.stable_key)
            }
            ExpectedItem::GraphEdge(edge) => {
                format!("GraphEdge:{}:{}:{}", edge.graph, edge.from, edge.to)
            }
            ExpectedItem::Path(path) => format!("Path:{}", path.path_id),
            ExpectedItem::Invariant(invariant) => {
                format!("Invariant:{}", invariant.name)
            }
            ExpectedItem::RuntimeBudget(budget) => {
                format!("RuntimeBudget:{}", budget.name)
            }
        }
    }

    fn fixture_requires_runtime_extension(fixture_dir: &Path) -> bool {
        fixture_dir.ends_with("extension/real-sink")
            || fixture_dir.ends_with("type-value-alias/extension-precision")
            || fixture_dir.ends_with("refined-calls/extension-model")
    }

    fn item_kinds(items: &[ExpectedItem]) -> Vec<&'static str> {
        let mut kinds = items
            .iter()
            .map(|item| match item {
                ExpectedItem::Diagnostic(_) => "Diagnostic",
                ExpectedItem::Fact(_) => "Fact",
                ExpectedItem::GraphEdge(_) => "GraphEdge",
                ExpectedItem::Path(_) => "Path",
                ExpectedItem::Invariant(_) => "Invariant",
                ExpectedItem::RuntimeBudget(_) => "RuntimeBudget",
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        kinds.sort();
        kinds
    }

    fn observed_item_kinds(items: &[ObservedItem]) -> Vec<&'static str> {
        let mut kinds = items
            .iter()
            .map(|item| match item {
                ObservedItem::Diagnostic(_) => "Diagnostic",
                ObservedItem::Fact(_) => "Fact",
                ObservedItem::GraphEdge(_) => "GraphEdge",
                ObservedItem::Path(_) => "Path",
                ObservedItem::Invariant(_) => "Invariant",
                ObservedItem::RuntimeBudget(_) => "RuntimeBudget",
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        kinds.sort();
        kinds
    }

    fn collect_native_fixture_dirs(root: &Path) -> Vec<PathBuf> {
        let mut fixture_dirs = Vec::new();
        collect_native_fixture_dirs_in(root, &mut fixture_dirs);
        fixture_dirs.sort();
        fixture_dirs
    }

    fn collect_native_fixture_dirs_in(current: &Path, fixture_dirs: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(current).unwrap_or_else(|error| {
            panic!("read native fixture dir {}: {error}", current.display())
        }) {
            let path = entry
                .unwrap_or_else(|error| panic!("read native fixture entry: {error}"))
                .path();
            if path.is_dir() {
                collect_native_fixture_dirs_in(&path, fixture_dirs);
            } else if path
                .file_name()
                .is_some_and(|name| name == std::ffi::OsStr::new(FIXTURE_MANIFEST_FILE))
            {
                fixture_dirs.push(current.to_path_buf());
            }
        }
    }

    fn run_fixture_for_suite_coverage(
        fixture_dir: &Path,
    ) -> anyhow::Result<crate::eval::report::EvaluationRun> {
        let fixture = load_native_fixture(fixture_dir)?;
        if fixture.manifest.area == FixtureArea::Cache
            && fixture.manifest.case_id == "cache-current-determinism"
        {
            run_cache_current_determinism_fixture_for_test(fixture_dir)
        } else if fixture.manifest.area == FixtureArea::Cache
            && fixture.manifest.case_id == "layer-cache"
        {
            run_layer_cache_fixture_for_test(fixture_dir)
        } else if fixture.manifest.area == FixtureArea::SemanticIndex
            && fixture.manifest.case_id == "semantic-index-core"
        {
            run_semantic_index_core_fixture_for_test(fixture_dir)
        } else if fixture.manifest.area == FixtureArea::ModuleTopology
            && fixture.manifest.case_id == "module-topology-core"
        {
            run_module_topology_core_fixture_for_test(fixture_dir)
        } else if fixture.manifest.area == FixtureArea::SemanticMir
            && fixture.manifest.case_id == "semantic-mir-core"
        {
            run_semantic_mir_core_fixture_for_test(fixture_dir)
        } else if fixture.manifest.area == FixtureArea::Cfg
            && fixture.manifest.case_id == "cfg-core"
        {
            run_cfg_core_fixture_for_test(fixture_dir)
        } else if fixture.manifest.area == FixtureArea::DirectCalls
            && fixture.manifest.case_id == "direct-calls-core"
        {
            Ok((*cached_direct_calls_core_fixture_for_test(fixture_dir)?).clone())
        } else if fixture.manifest.area == FixtureArea::AbstractDomains
            && fixture.manifest.case_id == "abstract-domains-core"
        {
            run_abstract_domains_core_fixture_for_test(fixture_dir)
        } else if fixture.manifest.area == FixtureArea::DirectSummaries
            && fixture.manifest.case_id == "direct-summaries-core"
        {
            run_direct_summaries_core_fixture_for_test(fixture_dir)
        } else if fixture.manifest.area == FixtureArea::DirectSummaries
            && fixture.manifest.case_id == "direct-summaries-scc-closure"
        {
            run_direct_summaries_scc_closure_fixture_for_test(fixture_dir)
        } else if fixture.manifest.area == FixtureArea::FrameworkEntrypoints
            && fixture.manifest.case_id == "framework-entrypoints-core"
        {
            run_framework_entrypoints_core_fixture_for_test(fixture_dir)
        } else {
            run_native_fixture_for_test(fixture_dir)
        }
    }
}

#[cfg(test)]
mod semantic_index_core {
    use std::path::{Path, PathBuf};

    use crate::eval::model::{ExpectedItem, FixtureArea, ObservedItem, ObservedStatus};
    use crate::eval::report::to_deterministic_json_pretty;

    use super::*;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("polint crate should live under crates/")
            .to_path_buf()
    }

    fn fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/semantic-index/core")
    }

    #[test]
    fn eval_semantic_index_core_fixture_passes() {
        let run = run_semantic_index_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("semantic-index core case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(case.case_id, "semantic-index-core");
        assert_eq!(case.area, FixtureArea::SemanticIndex);
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
        assert!(!rendered.contains(repo_root().to_string_lossy().as_ref()));
    }

    #[test]
    fn eval_semantic_index_core_manifest_covers_required_taxonomy() {
        let fixture = load_native_fixture(&fixture_dir()).unwrap();
        let expected_facts = fixture
            .manifest
            .expected
            .iter()
            .filter_map(|item| match item {
                ExpectedItem::Fact(fact) => Some((
                    fact.family.as_str(),
                    fact.status,
                    fact.precision.as_deref(),
                    fact.producer_id.as_deref(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        for required in [
            ("Reference", Some(ObservedStatus::Ambiguous)),
            ("Reference", Some(ObservedStatus::Unresolved)),
            ("SemanticImport", Some(ObservedStatus::Dynamic)),
            ("Alias", Some(ObservedStatus::Resolved)),
            ("Export", Some(ObservedStatus::Resolved)),
            ("GeneratedSymbol", Some(ObservedStatus::Generated)),
            ("StableExport", Some(ObservedStatus::Resolved)),
        ] {
            assert!(
                expected_facts
                    .iter()
                    .any(|(family, status, _, _)| *family == required.0 && *status == required.1),
                "semantic fixture missing expected {required:?}: {expected_facts:#?}"
            );
        }
        assert!(
            expected_facts
                .iter()
                .any(|(_, status, precision, producer)| {
                    matches!(
                        status,
                        Some(
                            ObservedStatus::Ambiguous
                                | ObservedStatus::Unresolved
                                | ObservedStatus::Dynamic
                                | ObservedStatus::Generated
                        )
                    ) && precision.is_some()
                        && *producer == Some("polint.symbol_graph")
                })
        );
    }

    #[test]
    fn eval_semantic_index_core_observes_unknown_semantic_statuses() {
        let run = run_semantic_index_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("semantic-index core case");

        for status in [
            ObservedStatus::Ambiguous,
            ObservedStatus::Unresolved,
            ObservedStatus::Dynamic,
            ObservedStatus::Generated,
        ] {
            assert!(
                case.observed.iter().any(|item| match item {
                    ObservedItem::Fact(fact) => fact.status == Some(status),
                    _ => false,
                }),
                "semantic fixture should observe {status:?}: {:#?}",
                case.observed
            );
        }
    }
}

#[cfg(test)]
mod module_topology_core {
    use std::path::{Path, PathBuf};

    use crate::eval::model::{ExpectedItem, FixtureArea, ObservedItem, ObservedStatus};
    use crate::eval::report::to_deterministic_json_pretty;

    use super::*;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("polint crate should live under crates/")
            .to_path_buf()
    }

    fn fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/module-topology/core")
    }

    #[test]
    fn eval_module_topology_core_fixture_passes() {
        let run = run_module_topology_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("module-topology core case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(case.case_id, "module-topology-core");
        assert_eq!(case.area, FixtureArea::ModuleTopology);
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
        assert!(!rendered.contains(repo_root().to_string_lossy().as_ref()));
    }

    #[test]
    fn eval_module_topology_core_manifest_covers_required_taxonomy() {
        let fixture = load_native_fixture(&fixture_dir()).unwrap();
        let expected_facts = fixture
            .manifest
            .expected
            .iter()
            .filter_map(|item| match item {
                ExpectedItem::Fact(fact) => {
                    Some((fact.family.as_str(), fact.stable_key.as_str(), fact.status))
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        for required in [
            (
                "WorkspaceRoot",
                "services/api",
                Some(ObservedStatus::Present),
            ),
            ("TopologyPackage", "web", Some(ObservedStatus::Present)),
            (
                "SourceSet",
                "web/src/app.test.ts",
                Some(ObservedStatus::Present),
            ),
            (
                "SourceSet",
                "web/generated/client.generated.ts",
                Some(ObservedStatus::Present),
            ),
            (
                "DependencyRequirement",
                "github.com/acme/lib",
                Some(ObservedStatus::Present),
            ),
            (
                "DependencyRequirement",
                "@scope/lib",
                Some(ObservedStatus::Present),
            ),
            (
                "ResolvedDependencyEdge",
                "@scope/lib",
                Some(ObservedStatus::Resolved),
            ),
            (
                "ImportToPackage",
                "@scope/lib",
                Some(ObservedStatus::External),
            ),
            (
                "RepoTopologyOverlay",
                "generated-zone",
                Some(ObservedStatus::Present),
            ),
            (
                "RepoTopologyOverlay",
                "test-only-visibility",
                Some(ObservedStatus::Present),
            ),
        ] {
            assert!(
                expected_facts.iter().any(|(family, key, status)| {
                    *family == required.0 && key.contains(required.1) && *status == required.2
                }),
                "module topology fixture missing expected {required:?}: {expected_facts:#?}"
            );
        }
    }

    #[test]
    fn eval_module_topology_core_observes_cache_and_edit_invalidation() {
        let run = run_module_topology_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("module-topology core case");

        for required in [
            ("layer_cache.provider.polint.module_graph.hits", "warm=1"),
            ("layer_cache.provider.polint.module_topology.hits", "warm=1"),
            (
                "layer_cache.provider.polint.module_graph.misses",
                "package_json_edit=1",
            ),
            (
                "layer_cache.provider.polint.module_topology.misses",
                "go_mod_edit=1",
            ),
        ] {
            assert!(
                case.observed.iter().any(|item| match item {
                    ObservedItem::Invariant(invariant) => {
                        invariant.name == required.0 && invariant.value == required.1
                    }
                    _ => false,
                }),
                "module topology fixture should observe cache invariant {required:?}: {:#?}",
                case.observed
            );
        }
    }
}

#[cfg(test)]
mod semantic_mir_core {
    use std::path::{Path, PathBuf};

    use crate::eval::model::{ExpectedItem, FixtureArea, ObservedItem, ObservedStatus};
    use crate::eval::report::to_deterministic_json_pretty;

    use super::*;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("polint crate should live under crates/")
            .to_path_buf()
    }

    fn fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/semantic-mir/core")
    }

    #[test]
    fn eval_semantic_mir_core_fixture_passes() {
        let run = run_semantic_mir_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("semantic-MIR core case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(case.case_id, "semantic-mir-core");
        assert_eq!(case.area, FixtureArea::SemanticMir);
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
        assert!(!rendered.contains(repo_root().to_string_lossy().as_ref()));
    }

    #[test]
    fn eval_semantic_mir_core_manifest_covers_required_taxonomy() {
        let fixture = load_native_fixture(&fixture_dir()).unwrap();
        let expected_facts = fixture
            .manifest
            .expected
            .iter()
            .filter_map(|item| match item {
                ExpectedItem::Fact(fact) => Some((
                    fact.family.as_str(),
                    fact.stable_key.as_str(),
                    fact.status,
                    fact.precision.as_deref(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        for required in [
            ("MirBody", "service.go", Some(ObservedStatus::Partial)),
            ("MirBody", "app.ts", Some(ObservedStatus::Partial)),
            ("MirOperation", "call", Some(ObservedStatus::Partial)),
            ("Place", "parameter", Some(ObservedStatus::Resolved)),
            ("Place", "local", Some(ObservedStatus::Resolved)),
            ("Place", "global", Some(ObservedStatus::Partial)),
            ("Place", "temporary", Some(ObservedStatus::Partial)),
            ("Place", "call_return", Some(ObservedStatus::Partial)),
            ("Place", "unknown", Some(ObservedStatus::Unknown)),
            (
                "UnsupportedSemantic",
                "defer_statement",
                Some(ObservedStatus::Unsupported),
            ),
            (
                "UnsupportedSemantic",
                "eval",
                Some(ObservedStatus::Unsupported),
            ),
        ] {
            assert!(
                expected_facts.iter().any(|(family, key, status, _)| {
                    *family == required.0 && key.contains(required.1) && *status == required.2
                }),
                "semantic MIR fixture missing expected {required:?}: {expected_facts:#?}"
            );
        }
        assert!(expected_facts.iter().any(|(_, _, _, precision)| {
            matches!(
                *precision,
                Some("setup_aware" | "heuristic" | "unsupported")
            )
        }));
    }

    #[test]
    fn eval_semantic_mir_core_observes_unknown_and_unsupported_rows() {
        let run = run_semantic_mir_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("semantic-MIR core case");

        for status in [
            ObservedStatus::Partial,
            ObservedStatus::Unknown,
            ObservedStatus::Unsupported,
        ] {
            assert!(
                case.observed.iter().any(|item| match item {
                    ObservedItem::Fact(fact) => fact.status == Some(status),
                    _ => false,
                }),
                "semantic MIR fixture should observe {status:?}: {:#?}",
                case.observed
            );
        }
        assert!(case.observed.iter().any(|item| match item {
            ObservedItem::Invariant(invariant) => {
                invariant.name == "semantic_mir.current_determinism"
                    && invariant.value == "cold_warm_equal"
            }
            _ => false,
        }));
    }
}

#[cfg(test)]
mod cfg_core {
    use std::path::{Path, PathBuf};

    use crate::eval::model::{
        CFG_FACT_FAMILIES, ExpectedItem, FixtureArea, ObservedItem, ObservedStatus,
    };
    use crate::eval::report::to_deterministic_json_pretty;

    use super::*;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("polint crate should live under crates/")
            .to_path_buf()
    }

    fn fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/cfg-core")
    }

    #[test]
    fn eval_cfg_core_fixture_passes() {
        let run = run_cfg_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("CFG core case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(case.case_id, "cfg-core");
        assert_eq!(case.area, FixtureArea::Cfg);
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
        assert!(!rendered.contains(repo_root().to_string_lossy().as_ref()));
        assert!(!rendered.contains("package cfgcore"));
        assert!(!rendered.contains("export function route"));
    }

    #[test]
    fn eval_cfg_core_manifest_covers_required_taxonomy() {
        let fixture = load_native_fixture(&fixture_dir()).unwrap();
        let expected_facts = fixture
            .manifest
            .expected
            .iter()
            .filter_map(|item| match item {
                ExpectedItem::Fact(fact) => Some((
                    fact.family.as_str(),
                    fact.stable_key.as_str(),
                    fact.status,
                    fact.precision.as_deref(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        for family in CFG_FACT_FAMILIES {
            assert!(
                expected_facts
                    .iter()
                    .any(|(observed, _, _, _)| observed == family),
                "CFG fixture missing expected family {family}: {expected_facts:#?}"
            );
        }
        assert!(
            expected_facts
                .iter()
                .any(|(family, key, status, precision)| {
                    *family == "UnsupportedControlFlow"
                        && key.contains("throw")
                        && *status == Some(ObservedStatus::Unsupported)
                        && *precision == Some("unsupported")
                })
        );
    }

    #[test]
    fn eval_cfg_core_observes_required_families_and_determinism() {
        let run = run_cfg_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("CFG core case");

        for family in CFG_FACT_FAMILIES {
            assert!(
                case.observed.iter().any(|item| match item {
                    ObservedItem::Fact(fact) => fact.family == *family,
                    _ => false,
                }),
                "CFG fixture should observe {family}: {:#?}",
                case.observed
            );
        }
        assert!(case.observed.iter().any(|item| match item {
            ObservedItem::Invariant(invariant) => {
                invariant.name == "cfg.current_determinism"
                    && invariant.value == "cold_warm_no_cache_equal"
            }
            _ => false,
        }));
    }
}

#[cfg(test)]
mod direct_calls_core {
    use std::path::{Path, PathBuf};

    use crate::eval::model::{ExpectedItem, FixtureArea, ObservedItem, ObservedStatus};
    use crate::eval::report::to_deterministic_json_pretty;

    use super::*;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("polint crate should live under crates/")
            .to_path_buf()
    }

    fn fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/direct-calls/core")
    }

    const REQUIRED_COUNT_INVARIANTS: &[&str] = &[
        "direct_calls.counts.by_language.Go.nonzero",
        "direct_calls.counts.by_language.TypeScript.nonzero",
        "direct_calls.counts.by_call_kind.Function.nonzero",
        "direct_calls.counts.by_call_kind.Member.nonzero",
        "direct_calls.counts.by_call_kind.Constructor.nonzero",
        "direct_calls.counts.by_algorithm.DirectReference.nonzero",
        "direct_calls.counts.by_algorithm.ImportBinding.nonzero",
        "direct_calls.counts.by_algorithm.ConstructorBinding.nonzero",
        "direct_calls.counts.by_algorithm.DirectMember.nonzero",
        "direct_calls.counts.by_status.Resolved.nonzero",
        "direct_calls.counts.by_status.Unresolved.nonzero",
        "direct_calls.counts.by_status.Unsupported.nonzero",
        "direct_calls.counts.by_status.SetupMissing.nonzero",
        "direct_calls.counts.by_unresolved_reason.FunctionValue.nonzero",
        "direct_calls.counts.by_unresolved_reason.DynamicProperty.nonzero",
        "direct_calls.counts.by_unresolved_reason.Reflection.nonzero",
        "direct_calls.counts.by_unresolved_reason.GoroutineBoundary.nonzero",
        "direct_calls.counts.by_unresolved_reason.Eval.nonzero",
        "direct_calls.counts.by_unresolved_reason.DynamicImport.nonzero",
        "direct_calls.counts.by_unresolved_reason.CallApplyBind.nonzero",
        "direct_calls.counts.by_unresolved_reason.SetupMissing.nonzero",
        "direct_calls.counts.by_provider.polint.calls.nonzero",
    ];

    const REQUIRED_INDEX_INVARIANTS: &[&str] = &[
        "direct_calls.index_counts.outgoing_by_function.nonzero",
        "direct_calls.index_counts.outgoing_by_symbol.nonzero",
        "direct_calls.index_counts.incoming_by_symbol.nonzero",
        "direct_calls.index_counts.incoming_by_function.nonzero",
        "direct_calls.index_counts.unresolved_by_reason.nonzero",
        "direct_calls.index_counts.unresolved_by_status.nonzero",
    ];

    #[derive(Clone, Copy, Debug)]
    struct DirectCallFeature {
        marker: &'static str,
        family: &'static str,
        stable_key_fragment: &'static str,
        status: ObservedStatus,
    }

    const SUPPORTED_DIRECT_CALL_FEATURES: &[DirectCallFeature] = &[
        DirectCallFeature {
            marker: "direct-calls/go/direct-function",
            family: "CallTarget",
            stable_key_fragment: "DirectReference",
            status: ObservedStatus::Resolved,
        },
        DirectCallFeature {
            marker: "direct-calls/go/method-call",
            family: "CallTarget",
            stable_key_fragment: "DirectMember",
            status: ObservedStatus::Resolved,
        },
        DirectCallFeature {
            marker: "direct-calls/go/function-value",
            family: "UnresolvedCall",
            stable_key_fragment: "FunctionValue",
            status: ObservedStatus::Unresolved,
        },
        DirectCallFeature {
            marker: "direct-calls/go/goroutine-boundary",
            family: "UnresolvedCall",
            stable_key_fragment: "GoroutineBoundary",
            status: ObservedStatus::Unsupported,
        },
        DirectCallFeature {
            marker: "direct-calls/go/reflection",
            family: "UnresolvedCall",
            stable_key_fragment: "Reflection",
            status: ObservedStatus::Unsupported,
        },
        DirectCallFeature {
            marker: "direct-calls/go/setup-missing-interface",
            family: "UnresolvedCall",
            stable_key_fragment: "SetupMissing",
            status: ObservedStatus::SetupMissing,
        },
        DirectCallFeature {
            marker: "direct-calls/ts/local-function",
            family: "CallTarget",
            stable_key_fragment: "DirectReference",
            status: ObservedStatus::Resolved,
        },
        DirectCallFeature {
            marker: "direct-calls/ts/import-binding",
            family: "CallTarget",
            stable_key_fragment: "ImportBinding",
            status: ObservedStatus::Resolved,
        },
        DirectCallFeature {
            marker: "direct-calls/ts/static-member",
            family: "CallTarget",
            stable_key_fragment: "DirectMember",
            status: ObservedStatus::Resolved,
        },
        DirectCallFeature {
            marker: "direct-calls/ts/constructor-as-function",
            family: "CallTarget",
            stable_key_fragment: "Constructor",
            status: ObservedStatus::Resolved,
        },
        DirectCallFeature {
            marker: "direct-calls/ts/constructor",
            family: "CallTarget",
            stable_key_fragment: "Constructor",
            status: ObservedStatus::Resolved,
        },
        DirectCallFeature {
            marker: "direct-calls/ts/instance-member",
            family: "CallTarget",
            stable_key_fragment: "DirectMember",
            status: ObservedStatus::Resolved,
        },
        DirectCallFeature {
            marker: "direct-calls/ts/function-value",
            family: "UnresolvedCall",
            stable_key_fragment: "FunctionValue",
            status: ObservedStatus::Unresolved,
        },
        DirectCallFeature {
            marker: "direct-calls/ts/dynamic-property",
            family: "UnresolvedCall",
            stable_key_fragment: "DynamicProperty",
            status: ObservedStatus::Unresolved,
        },
        DirectCallFeature {
            marker: "direct-calls/ts/call-apply-bind",
            family: "UnresolvedCall",
            stable_key_fragment: "CallApplyBind",
            status: ObservedStatus::Unresolved,
        },
        DirectCallFeature {
            marker: "direct-calls/ts/eval",
            family: "UnresolvedCall",
            stable_key_fragment: "Eval",
            status: ObservedStatus::Unresolved,
        },
        DirectCallFeature {
            marker: "direct-calls/ts/dynamic-import",
            family: "UnresolvedCall",
            stable_key_fragment: "DynamicImport",
            status: ObservedStatus::Unresolved,
        },
    ];

    #[test]
    fn eval_direct_calls_core_fixture_passes() {
        let run = cached_direct_calls_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("direct calls core case");
        let rendered = to_deterministic_json_pretty(run);

        assert_eq!(case.case_id, "direct-calls-core");
        assert_eq!(case.area, FixtureArea::DirectCalls);
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
        assert!(!rendered.contains(repo_root().to_string_lossy().as_ref()));
        assert!(!rendered.contains("package directcalls"));
        assert!(!rendered.contains("export function handler"));
    }

    #[test]
    fn eval_direct_calls_core_manifest_covers_required_taxonomy() {
        let fixture = load_native_fixture(&fixture_dir()).unwrap();
        let expected_facts = fixture
            .manifest
            .expected
            .iter()
            .filter_map(|item| match item {
                ExpectedItem::Fact(fact) => Some((
                    fact.family.as_str(),
                    fact.stable_key.as_str(),
                    fact.status,
                    fact.precision.as_deref(),
                    fact.producer_id.as_deref(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        for required in [
            ("CallSite", "directFunction", Some(ObservedStatus::Resolved)),
            ("CallSite", "handler", Some(ObservedStatus::Resolved)),
            (
                "CallTarget",
                "DirectReference",
                Some(ObservedStatus::Resolved),
            ),
            (
                "CallTarget",
                "ImportBinding",
                Some(ObservedStatus::Resolved),
            ),
            ("CallTarget", "Constructor", Some(ObservedStatus::Resolved)),
            ("CallTarget", "DirectMember", Some(ObservedStatus::Resolved)),
            (
                "UnresolvedCall",
                "FunctionValue",
                Some(ObservedStatus::Unresolved),
            ),
            (
                "UnresolvedCall",
                "DynamicProperty",
                Some(ObservedStatus::Unresolved),
            ),
            (
                "UnresolvedCall",
                "Reflection",
                Some(ObservedStatus::Unsupported),
            ),
            (
                "UnresolvedCall",
                "GoroutineBoundary",
                Some(ObservedStatus::Unsupported),
            ),
            ("UnresolvedCall", "Eval", Some(ObservedStatus::Unresolved)),
            (
                "UnresolvedCall",
                "DynamicImport",
                Some(ObservedStatus::Unresolved),
            ),
            (
                "UnresolvedCall",
                "CallApplyBind",
                Some(ObservedStatus::Unresolved),
            ),
            (
                "UnresolvedCall",
                "SetupMissing",
                Some(ObservedStatus::SetupMissing),
            ),
        ] {
            assert!(
                expected_facts
                    .iter()
                    .any(|(family, key, status, _, producer)| {
                        *family == required.0
                            && key.contains(required.1)
                            && *status == required.2
                            && *producer == Some("polint.calls")
                    }),
                "direct-call fixture missing expected {required:?}: {expected_facts:#?}"
            );
        }
        assert!(expected_facts.iter().any(|(_, _, _, precision, _)| {
            matches!(*precision, Some("setup_aware" | "unknown" | "unsupported"))
        }));
    }

    #[test]
    fn eval_direct_calls_core_source_documents_supported_language_features() {
        let markers = fixture_feature_markers(&["service.go", "web/src/app.ts"]);
        let expected = SUPPORTED_DIRECT_CALL_FEATURES
            .iter()
            .map(|feature| feature.marker.to_string())
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(markers, expected);
    }

    #[test]
    fn eval_direct_calls_core_observes_supported_language_feature_contract() {
        let run = cached_direct_calls_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("direct calls core case");

        for feature in SUPPORTED_DIRECT_CALL_FEATURES {
            assert!(
                case.observed.iter().any(|item| match item {
                    ObservedItem::Fact(fact) => {
                        fact.family == feature.family
                            && fact.stable_key.contains(feature.stable_key_fragment)
                            && fact.status == Some(feature.status)
                    }
                    _ => false,
                }),
                "direct-call feature {feature:?} was not observed: {:#?}",
                case.observed
            );
        }
    }

    #[test]
    fn eval_direct_calls_core_manifest_covers_counts_and_indexes() {
        let fixture = load_native_fixture(&fixture_dir()).unwrap();
        let expected_invariants = fixture
            .manifest
            .expected
            .iter()
            .filter_map(|item| match item {
                ExpectedItem::Invariant(invariant) => Some(invariant.name.as_str()),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();

        for required in REQUIRED_COUNT_INVARIANTS
            .iter()
            .chain(REQUIRED_INDEX_INVARIANTS)
        {
            assert!(
                expected_invariants.contains(required),
                "direct-call fixture manifest missing invariant {required}"
            );
        }
    }

    #[test]
    fn eval_direct_calls_core_observes_required_families_and_determinism() {
        let run = cached_direct_calls_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("direct calls core case");

        for family in crate::eval::model::CALL_FACT_FAMILIES {
            assert!(
                case.observed.iter().any(|item| match item {
                    ObservedItem::Fact(fact) => fact.family == *family,
                    _ => false,
                }),
                "direct-call fixture should observe {family}: {:#?}",
                case.observed
            );
        }
        assert!(case.observed.iter().any(|item| match item {
            ObservedItem::Invariant(invariant) => {
                invariant.name == "direct_calls.current_determinism"
                    && invariant.value == "cold_warm_no_cache_equal"
            }
            _ => false,
        }));
    }

    #[test]
    fn eval_direct_calls_core_observes_counts_and_indexes() {
        let run = cached_direct_calls_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("direct calls core case");

        for required in REQUIRED_COUNT_INVARIANTS
            .iter()
            .chain(REQUIRED_INDEX_INVARIANTS)
        {
            assert!(
                case.observed.iter().any(|item| match item {
                    ObservedItem::Invariant(invariant) => {
                        invariant.name == *required && invariant.value == "true"
                    }
                    _ => false,
                }),
                "direct-call fixture should observe invariant {required}: {:#?}",
                case.observed
            );
        }
    }

    fn fixture_feature_markers(paths: &[&str]) -> std::collections::BTreeSet<String> {
        let mut markers = std::collections::BTreeSet::new();
        for path in paths {
            let source = std::fs::read_to_string(fixture_dir().join("repo").join(path))
                .expect("fixture source should be readable");
            for marker in source.lines().filter_map(feature_marker) {
                markers.insert(marker);
            }
        }
        markers
    }

    fn feature_marker(line: &str) -> Option<String> {
        Some(line.split_once("POLINT-FEATURE")?.1.trim().to_string())
    }
}

#[cfg(test)]
mod abstract_domains_core {
    use std::path::{Path, PathBuf};

    use crate::eval::model::{ExpectedItem, FixtureArea, ObservedItem, ObservedStatus};
    use crate::eval::report::to_deterministic_json_pretty;

    use super::*;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("polint crate should live under crates/")
            .to_path_buf()
    }

    fn fixture_dir() -> PathBuf {
        repo_root().join("tests/eval-fixtures/abstract-domains/core")
    }

    const REQUIRED_DOMAIN_COUNT_INVARIANTS: &[&str] = &[
        "abstract_domains.counts.by_slot.reachability.nonzero",
        "abstract_domains.counts.by_slot.nilness.nonzero",
        "abstract_domains.counts.by_slot.truthiness.nonzero",
        "abstract_domains.counts.by_slot.constants.nonzero",
        "abstract_domains.counts.by_slot.strings.nonzero",
        "abstract_domains.counts.by_slot.initializedness.nonzero",
        "abstract_domains.counts.by_status.present.nonzero",
        "abstract_domains.counts.by_status.unknown.nonzero",
        "abstract_domains.counts.by_status.unsupported.nonzero",
        "abstract_domains.counts.by_status.budget_exceeded.nonzero",
        "abstract_domains.counts.by_precision.exact_local.nonzero",
        "abstract_domains.counts.by_precision.unknown.nonzero",
        "abstract_domains.counts.by_precision.unsupported.nonzero",
        "abstract_domains.counts.by_provider.polint.abstract_domains.nonzero",
    ];

    const REQUIRED_DOMAIN_INDEX_INVARIANTS: &[&str] = &[
        "abstract_domains.index_counts.observations_by_body.nonzero",
        "abstract_domains.index_counts.observations_by_slot.nonzero",
        "abstract_domains.index_counts.events_by_status.nonzero",
    ];

    #[derive(Clone, Copy, Debug)]
    struct AbstractDomainFeature {
        marker: &'static str,
        stable_key_fragment: &'static str,
        status: ObservedStatus,
        precision: &'static str,
    }

    const SUPPORTED_ABSTRACT_DOMAIN_FEATURES: &[AbstractDomainFeature] = &[
        AbstractDomainFeature {
            marker: "abstract-domains/go/initialized-locals",
            stable_key_fragment: "initializedness",
            status: ObservedStatus::Present,
            precision: "exact_local",
        },
        AbstractDomainFeature {
            marker: "abstract-domains/go/string-constant",
            stable_key_fragment: "constants",
            status: ObservedStatus::Present,
            precision: "exact_local",
        },
        AbstractDomainFeature {
            marker: "abstract-domains/go/nil-branch",
            stable_key_fragment: "nilness",
            status: ObservedStatus::Present,
            precision: "exact_local",
        },
        AbstractDomainFeature {
            marker: "abstract-domains/go/boolean-branch",
            stable_key_fragment: "truthiness",
            status: ObservedStatus::Present,
            precision: "exact_local",
        },
        AbstractDomainFeature {
            marker: "abstract-domains/go/unknown-call-havoc",
            stable_key_fragment: "unknown",
            status: ObservedStatus::Unknown,
            precision: "unknown",
        },
        AbstractDomainFeature {
            marker: "abstract-domains/ts/maybe-uninitialized-local",
            stable_key_fragment: "initializedness",
            status: ObservedStatus::Present,
            precision: "exact_local",
        },
        AbstractDomainFeature {
            marker: "abstract-domains/ts/string-constant",
            stable_key_fragment: "constants",
            status: ObservedStatus::Present,
            precision: "exact_local",
        },
        AbstractDomainFeature {
            marker: "abstract-domains/ts/nullish-branch",
            stable_key_fragment: "nilness",
            status: ObservedStatus::Present,
            precision: "exact_local",
        },
        AbstractDomainFeature {
            marker: "abstract-domains/ts/boolean-branch",
            stable_key_fragment: "truthiness",
            status: ObservedStatus::Present,
            precision: "exact_local",
        },
        AbstractDomainFeature {
            marker: "abstract-domains/ts/dynamic-write-havoc",
            stable_key_fragment: "unsupported",
            status: ObservedStatus::Unsupported,
            precision: "unsupported",
        },
        AbstractDomainFeature {
            marker: "abstract-domains/ts/nullish-coalescing",
            stable_key_fragment: "nilness",
            status: ObservedStatus::Present,
            precision: "exact_local",
        },
    ];

    #[test]
    fn eval_abstract_domains_core_fixture_passes() {
        let run = run_abstract_domains_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("abstract-domains core case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(case.case_id, "abstract-domains-core");
        assert_eq!(case.area, FixtureArea::AbstractDomains);
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
        assert!(!rendered.contains(repo_root().to_string_lossy().as_ref()));
    }

    #[test]
    fn eval_abstract_domains_core_manifest_covers_p0_slots_and_uncertainty() {
        let fixture = load_native_fixture(&fixture_dir()).unwrap();
        let expected_facts = fixture
            .manifest
            .expected
            .iter()
            .filter_map(|item| match item {
                ExpectedItem::Fact(fact) => Some((
                    fact.family.as_str(),
                    fact.stable_key.as_str(),
                    fact.status,
                    fact.precision.as_deref(),
                    fact.producer_id.as_deref(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        for required in [
            (
                "DomainObservation",
                "reachability",
                Some(ObservedStatus::Present),
            ),
            (
                "DomainObservation",
                "nilness",
                Some(ObservedStatus::Present),
            ),
            (
                "DomainObservation",
                "truthiness",
                Some(ObservedStatus::Present),
            ),
            (
                "DomainObservation",
                "constants",
                Some(ObservedStatus::Present),
            ),
            (
                "DomainObservation",
                "strings",
                Some(ObservedStatus::Unknown),
            ),
            (
                "DomainObservation",
                "initializedness",
                Some(ObservedStatus::Present),
            ),
            (
                "DomainObservation",
                "unknown",
                Some(ObservedStatus::Unknown),
            ),
            (
                "DomainObservation",
                "unsupported",
                Some(ObservedStatus::Unsupported),
            ),
            (
                "DomainEvent",
                "budget",
                Some(ObservedStatus::BudgetExceeded),
            ),
        ] {
            assert!(
                expected_facts
                    .iter()
                    .any(|(family, key, status, _, producer)| {
                        *family == required.0
                            && key.contains(required.1)
                            && *status == required.2
                            && *producer == Some("polint.abstract_domains")
                    }),
                "abstract-domain fixture missing expected {required:?}: {expected_facts:#?}"
            );
        }
    }

    #[test]
    fn eval_abstract_domains_core_source_documents_supported_language_features() {
        let markers = fixture_feature_markers(&["domain.go", "web/src/domain.ts"]);
        let expected = SUPPORTED_ABSTRACT_DOMAIN_FEATURES
            .iter()
            .map(|feature| feature.marker.to_string())
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(markers, expected);
    }

    #[test]
    fn eval_abstract_domains_core_observes_supported_language_feature_contract() {
        let run = run_abstract_domains_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("abstract-domains core case");

        for feature in SUPPORTED_ABSTRACT_DOMAIN_FEATURES {
            assert!(
                case.observed.iter().any(|item| match item {
                    ObservedItem::Fact(fact) => {
                        fact.family == "DomainObservation"
                            && fact.stable_key.contains(feature.stable_key_fragment)
                            && fact.status == Some(feature.status)
                            && fact.precision.as_deref() == Some(feature.precision)
                    }
                    _ => false,
                }),
                "abstract-domain feature {feature:?} was not observed: {:#?}",
                case.observed
            );
        }
    }

    #[test]
    fn eval_abstract_domains_core_observes_determinism_and_indexes() {
        let run = run_abstract_domains_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("abstract-domains core case");

        for required in [(
            "abstract_domains.current_determinism",
            "cold_warm_no_cache_equal",
        )]
        .into_iter()
        .chain(
            REQUIRED_DOMAIN_COUNT_INVARIANTS
                .iter()
                .chain(REQUIRED_DOMAIN_INDEX_INVARIANTS)
                .map(|name| (*name, "true")),
        ) {
            assert!(
                case.observed.iter().any(|item| match item {
                    ObservedItem::Invariant(invariant) => {
                        invariant.name == required.0 && invariant.value == required.1
                    }
                    _ => false,
                }),
                "abstract-domain fixture should observe invariant {required:?}: {:#?}",
                case.observed
            );
        }
    }

    fn fixture_feature_markers(paths: &[&str]) -> std::collections::BTreeSet<String> {
        let mut markers = std::collections::BTreeSet::new();
        for path in paths {
            let source = std::fs::read_to_string(fixture_dir().join("repo").join(path))
                .expect("fixture source should be readable");
            for marker in source.lines().filter_map(feature_marker) {
                markers.insert(marker);
            }
        }
        markers
    }

    fn feature_marker(line: &str) -> Option<String> {
        Some(line.split_once("POLINT-FEATURE")?.1.trim().to_string())
    }
}

#[cfg(test)]
mod direct_summaries_core {
    use std::path::PathBuf;

    use crate::eval::fixtures::{load_native_fixture, run_direct_summaries_core_fixture_for_test};
    use crate::eval::model::{ExpectedItem, FixtureArea, ObservedItem, ObservedStatus};

    fn fixture_dir() -> PathBuf {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/eval-fixtures/direct-summaries/core"
        ))
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repo root should resolve")
    }

    fn to_deterministic_json_pretty(run: &crate::eval::report::EvaluationRun) -> String {
        serde_json::to_string_pretty(run).unwrap_or_else(|_| "{}".to_string())
    }

    #[test]
    fn direct_summaries_core_fixture_passes() {
        let run = run_direct_summaries_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("direct-summaries core case");
        let rendered = to_deterministic_json_pretty(&run);

        assert_eq!(case.case_id, "direct-summaries-core");
        assert_eq!(case.area, FixtureArea::DirectSummaries);
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
        assert!(!rendered.contains(repo_root().to_string_lossy().as_ref()));
    }

    #[test]
    fn direct_summaries_core_manifest_covers_four_domains_and_events() {
        let fixture = load_native_fixture(&fixture_dir()).unwrap();
        let expected_facts = fixture
            .manifest
            .expected
            .iter()
            .filter_map(|item| match item {
                ExpectedItem::Fact(fact) => Some((
                    fact.family.as_str(),
                    fact.stable_key.as_str(),
                    fact.status,
                    fact.precision.as_deref(),
                    fact.producer_id.as_deref(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        for required in [
            (
                "summary_control",
                "control_effects",
                Some(ObservedStatus::Present),
            ),
            (
                "summary_call",
                "call_effects",
                Some(ObservedStatus::Present),
            ),
            (
                "summary_memory",
                "memory_effects",
                Some(ObservedStatus::Present),
            ),
            (
                "summary_tito",
                "data_flow_tito",
                Some(ObservedStatus::Present),
            ),
            (
                "summary_event",
                "SummaryEvent",
                Some(ObservedStatus::Unknown),
            ),
        ] {
            assert!(
                expected_facts
                    .iter()
                    .any(|(family, key, status, _, producer)| {
                        *family == required.0
                            && key.contains(required.1)
                            && *status == required.2
                            && *producer == Some("polint.direct_summaries")
                    }),
                "direct-summaries fixture missing expected {required:?}: {expected_facts:#?}"
            );
        }
    }

    #[test]
    fn direct_summaries_core_observes_domain_counts_and_determinism() {
        let run = run_direct_summaries_core_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("direct-summaries core case");

        for required in [
            (
                "direct_summaries.domain_counts.control_effects.nonzero",
                "true",
            ),
            (
                "direct_summaries.domain_counts.call_effects.nonzero",
                "true",
            ),
            (
                "direct_summaries.domain_counts.memory_effects.nonzero",
                "true",
            ),
            (
                "direct_summaries.domain_counts.data_flow_tito.nonzero",
                "true",
            ),
            (
                "direct_summaries.current_determinism",
                "cold_warm_no_cache_equal",
            ),
        ] {
            assert!(
                case.observed.iter().any(|item| match item {
                    ObservedItem::Invariant(invariant) => {
                        invariant.name == required.0 && invariant.value == required.1
                    }
                    _ => false,
                }),
                "direct-summaries fixture should observe invariant {required:?}: {:#?}",
                case.observed
            );
        }
    }

    #[test]
    fn direct_summaries_core_source_documents_supported_language_features() {
        let markers = fixture_feature_markers(&["summary.go", "web/src/summary.ts"]);
        let expected = [
            "direct-summaries/go/global-read",
            "direct-summaries/go/panic-does-not-return",
            "direct-summaries/go/param-to-return-tito",
            "direct-summaries/go/pure-function",
            "direct-summaries/go/receiver-mutation",
            "direct-summaries/go/unresolved-call",
            "direct-summaries/ts/dynamic-callback",
            "direct-summaries/ts/no-effects",
            "direct-summaries/ts/param-to-return-tito",
            "direct-summaries/ts/param-write",
            "direct-summaries/ts/throw-does-not-return",
        ]
        .into_iter()
        .map(String::from)
        .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(markers, expected);
    }

    fn fixture_feature_markers(paths: &[&str]) -> std::collections::BTreeSet<String> {
        let mut markers = std::collections::BTreeSet::new();
        for path in paths {
            let source = std::fs::read_to_string(fixture_dir().join("repo").join(path))
                .expect("fixture source should be readable");
            for marker in source.lines().filter_map(feature_marker) {
                markers.insert(marker);
            }
        }
        markers
    }

    fn feature_marker(line: &str) -> Option<String> {
        Some(line.split_once("POLINT-FEATURE")?.1.trim().to_string())
    }
}

#[cfg(test)]
mod direct_summaries_scc_closure {
    use std::path::PathBuf;

    use crate::eval::fixtures::{
        load_native_fixture, run_direct_summaries_scc_closure_fixture_for_test,
    };
    use crate::eval::model::{ExpectedItem, FixtureArea, ObservedItem};

    fn fixture_dir() -> PathBuf {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/eval-fixtures/direct-summaries/scc-closure"
        ))
    }

    #[test]
    fn eval_scc_closure_fixture_loads_with_expected_identity() {
        let fixture = load_native_fixture(&fixture_dir()).unwrap();

        assert_eq!(fixture.manifest.case_id, "direct-summaries-scc-closure");
        assert_eq!(fixture.manifest.area, FixtureArea::DirectSummaries);
        assert!(fixture.repo_dir.join("main.go").exists());
        assert!(fixture.repo_dir.join("index.ts").exists());
    }

    #[test]
    fn eval_scc_closure_manifest_asserts_schedule_demand_and_determinism() {
        let fixture = load_native_fixture(&fixture_dir()).unwrap();
        let expected = fixture
            .manifest
            .expected
            .iter()
            .filter_map(|item| match item {
                ExpectedItem::Invariant(invariant) => {
                    Some((invariant.name.as_str(), invariant.value.as_str()))
                }
                _ => None,
            })
            .collect::<std::collections::BTreeMap<_, _>>();

        for required in [
            ("scc_schedule.total_sccs.nonzero", "true"),
            ("scc_schedule.recursive_scc_count.nonzero", "true"),
            ("scc_schedule.max_scc_size.at_least_2", "true"),
            ("scc_schedule.iteration_counts.nonzero", "true"),
            ("demand_queries.total_queries.nonzero", "true"),
            ("demand_queries.cache_misses.nonzero", "true"),
            (
                "direct_summaries.current_determinism",
                "cold_warm_no_cache_equal",
            ),
        ] {
            assert_eq!(expected.get(required.0).copied(), Some(required.1));
        }
    }

    #[test]
    fn eval_scc_closure_observes_schedule_demand_and_determinism() {
        let run = run_direct_summaries_scc_closure_fixture_for_test(&fixture_dir()).unwrap();
        let case = run.cases.first().expect("scc closure case");
        let rendered = serde_json::to_string_pretty(&run).unwrap_or_else(|_| "{}".to_string());
        let summary_fact_count = case
            .observed
            .iter()
            .filter(|item| match item {
                ObservedItem::Fact(fact) => fact.family.starts_with("summary_"),
                _ => false,
            })
            .count();
        let observed = case
            .observed
            .iter()
            .filter_map(|item| match item {
                ObservedItem::Invariant(invariant) => {
                    Some((invariant.name.as_str(), invariant.value.as_str()))
                }
                _ => None,
            })
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(case.case_id, "direct-summaries-scc-closure");
        assert_eq!(case.area, FixtureArea::DirectSummaries);
        assert_eq!(run.metrics.false_negatives, 0, "{rendered}");
        assert_eq!(run.metrics.forbidden_hits, 0, "{rendered}");
        assert_eq!(run.metrics.runtime_budget_failed, 0, "{rendered}");
        assert!(
            summary_fact_count > 0,
            "SCC fixture runner must observe real kernel summary facts, not only synthesize invariants: {rendered}"
        );

        for required in [
            ("scc_schedule.total_sccs.nonzero", "true"),
            ("scc_schedule.recursive_scc_count.nonzero", "true"),
            ("scc_schedule.max_scc_size.at_least_2", "true"),
            ("scc_schedule.iteration_counts.nonzero", "true"),
            ("demand_queries.total_queries.nonzero", "true"),
            ("demand_queries.cache_misses.nonzero", "true"),
            (
                "direct_summaries.current_determinism",
                "cold_warm_no_cache_equal",
            ),
        ] {
            assert_eq!(
                observed.get(required.0).copied(),
                Some(required.1),
                "missing observed invariant {required:?}"
            );
        }
    }
}
