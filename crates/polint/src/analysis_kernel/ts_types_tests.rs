//! End-to-end checks for the TypeScript type-directed call-graph tier.
//!
//! These run the real Node sidecar against a real TypeScript compiler, so they
//! skip when neither is present. Set `POLINT_REQUIRE_TS_TYPESCRIPT=1` to make a
//! skip a failure instead, which is how the gated CI job runs them.

use std::path::{Path, PathBuf};

use crate::analysis_kernel::{AnalysisKernel, KernelInput};
use crate::analysis_neutral::refined_calls::facts::{RefinedCallConfidence, RefinedCallTier};
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::load_config;

const REQUIRE_ENV: &str = "POLINT_REQUIRE_TS_TYPESCRIPT";

/// A TypeScript package directory this host can hand to the sidecar, or `None`
/// when the host has no compiler.
fn available_typescript() -> Option<PathBuf> {
    if let Ok(configured) = std::env::var(crate::ts::types::process::TS_TYPESCRIPT_ENV)
        && !configured.trim().is_empty()
    {
        let path = PathBuf::from(configured.trim());
        if path.join("package.json").is_file() {
            return Some(path);
        }
    }
    // Walk up from this crate: a checkout that installed TypeScript anywhere
    // above the workspace can run these tests without further setup.
    let mut current = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
    loop {
        let candidate = current.join("node_modules").join("typescript");
        if candidate.join("package.json").is_file() {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn skip_without_typescript(test: &str) -> Option<PathBuf> {
    match available_typescript() {
        Some(path) => Some(path),
        None => {
            let message = format!(
                "SKIP {test}: no TypeScript compiler on this host; \
                 install `typescript` or set {} (set {REQUIRE_ENV}=1 to fail instead of skip)",
                crate::ts::types::process::TS_TYPESCRIPT_ENV
            );
            if std::env::var_os(REQUIRE_ENV).is_some() {
                panic!("{message}");
            }
            eprintln!("{message}");
            None
        }
    }
}

/// A repository whose interface-typed call cannot be resolved by name alone.
fn write_dispatch_repo(root: &Path) {
    std::fs::write(
        root.join("tsconfig.json"),
        "{\"compilerOptions\":{\"strict\":true,\"target\":\"ES2022\",\
         \"module\":\"commonjs\"},\"include\":[\"src/**/*\"]}",
    )
    .expect("write tsconfig");
    std::fs::create_dir_all(root.join("src")).expect("create src");
    std::fs::write(
        root.join("src/shapes.ts"),
        "export interface Greeter {\n  greet(name: string): string;\n}\n\n\
         export class Loud implements Greeter {\n  greet(name: string): string {\n    \
         return `HELLO ${name}`;\n  }\n}\n\n\
         export class Quiet implements Greeter {\n  greet(name: string): string {\n    \
         return `hi ${name}`;\n  }\n}\n",
    )
    .expect("write shapes");
    std::fs::write(
        root.join("src/main.ts"),
        "import { Greeter, Loud, Quiet } from './shapes';\n\n\
         function run(greeter: Greeter, name: string): string {\n  \
         return greeter.greet(name);\n}\n\n\
         export function dispatch(loud: boolean): string {\n  \
         const greeter: Greeter = loud ? new Loud() : new Quiet();\n  \
         return run(greeter, 'world');\n}\n\n\
         export function sloppy(anything: any): unknown {\n  \
         return anything.whatever();\n}\n",
    )
    .expect("write main");
}

/// Pins the compiler through configuration rather than the environment, so the
/// tests exercise the path a repository would actually use and stay safe to run
/// in parallel.
fn write_polint_config(root: &Path, typescript: &Path) {
    std::fs::write(
        root.join(".polint.toml"),
        format!(
            "[languages.ts]\ntypescript_path = \"{}\"\n",
            typescript.display().to_string().replace('\\', "/")
        ),
    )
    .expect("write .polint.toml");
}

fn run_dispatch_repo(root: &Path, typescript: &Path) -> crate::analysis_kernel::KernelOutput {
    write_polint_config(root, typescript);
    let loaded = load_config(root).expect("config loads");
    let cache = Cache::new("", false);
    let plan = AnalysisPlan::from_capability_names_for_test(&["calls"]);
    AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &cache,
        config_digest: "config",
        rule_digest: "rules",
        plan: &plan,
        parallel: false,
    })
    .expect("kernel should run")
}

#[test]
fn interface_dispatch_resolves_to_every_instantiated_implementation() {
    let Some(typescript) =
        skip_without_typescript("interface_dispatch_resolves_to_every_instantiated_implementation")
    else {
        return;
    };
    let temp = tempfile::tempdir().expect("tempdir");
    write_dispatch_repo(temp.path());

    let output = run_dispatch_repo(temp.path(), &typescript);

    let callsites = output.db.ts_type_callsites().len();
    assert!(
        callsites > 0,
        "the sidecar produced no call sites; provider diagnostics: {:?}",
        output.diagnostics
    );

    let typed = output
        .db
        .refined_call_edges()
        .iter()
        .filter(|edge| edge.tier == RefinedCallTier::TypeDirected)
        .collect::<Vec<_>>();
    assert!(
        !typed.is_empty(),
        "no type-directed edges from {callsites} sidecar call sites"
    );

    let functions = output
        .db
        .functions()
        .iter()
        .map(|function| (function.id, function.name.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let targets = typed
        .iter()
        .filter_map(|edge| edge.target_function)
        .filter_map(|id| functions.get(&id).cloned())
        .collect::<std::collections::BTreeSet<_>>();

    // `greeter.greet(name)` is typed by the interface, so both instantiated
    // implementations are candidates and neither is reachable by name alone.
    assert!(
        targets.contains("Loud.greet"),
        "interface dispatch missed Loud.greet; resolved targets: {targets:?}"
    );
    assert!(
        targets.contains("Quiet.greet"),
        "interface dispatch missed Quiet.greet; resolved targets: {targets:?}"
    );
}

#[test]
fn an_any_receiver_never_produces_a_confident_typed_edge() {
    let Some(typescript) =
        skip_without_typescript("an_any_receiver_never_produces_a_confident_typed_edge")
    else {
        return;
    };
    let temp = tempfile::tempdir().expect("tempdir");
    write_dispatch_repo(temp.path());

    let output = run_dispatch_repo(temp.path(), &typescript);

    let any_site = output
        .db
        .ts_type_callsites()
        .iter()
        .find(|site| site.status == crate::ts::types::facts::TsTypeCallStatus::AnyReceiver)
        .expect("anything.whatever() has an any receiver");

    let edges = output
        .db
        .refined_call_edges()
        .iter()
        .filter(|edge| {
            edge.tier == RefinedCallTier::TypeDirected
                && any_site
                    .span
                    .as_ref()
                    .is_some_and(|span| edge_covers(edge, span, &output.db))
        })
        .collect::<Vec<_>>();

    assert!(
        edges
            .iter()
            .all(|edge| edge.confidence == RefinedCallConfidence::Low),
        "an `any` receiver must never yield a confident typed edge"
    );
}

fn edge_covers(
    edge: &crate::analysis_neutral::refined_calls::facts::RefinedCallEdgeFact,
    span: &crate::internal_core::Span,
    db: &crate::core::AnalysisDb,
) -> bool {
    db.call_sites()
        .iter()
        .find(|site| site.id == edge.site)
        .is_some_and(|site| site.span.start_byte == span.start_byte && site.file == span.file)
}

#[test]
fn a_repository_the_type_tier_cannot_analyze_falls_back_instead_of_failing() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_dispatch_repo(temp.path());
    // No tsconfig means no TypeScript program, which is the same setup gap a
    // missing compiler produces and does not depend on what this host has
    // installed.
    std::fs::remove_file(temp.path().join("tsconfig.json")).expect("remove tsconfig");
    std::fs::write(
        temp.path().join(".polint.toml"),
        "[languages.ts]\ntype_sidecar = true\n",
    )
    .expect("write .polint.toml");

    let loaded = load_config(temp.path()).expect("config loads");
    let cache = Cache::new("", false);
    let plan = AnalysisPlan::from_capability_names_for_test(&["calls"]);
    let output = AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &cache,
        config_digest: "config",
        rule_digest: "rules",
        plan: &plan,
        parallel: false,
    })
    .expect("an unusable type tier must not fail the run");

    assert!(output.db.ts_type_callsites().is_empty());
    assert!(
        !output.db.refined_call_edges().is_empty(),
        "the points-to and direct tiers must still answer without the type tier"
    );
    assert!(
        output
            .db
            .refined_call_edges()
            .iter()
            .all(|edge| edge.tier != RefinedCallTier::TypeDirected),
        "no typed edges may exist when the tier could not run"
    );
    // The repository named the tier, so the gap is reported rather than silent.
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "polint/ts-types"),
        "a requested tier that could not run must be reported"
    );
}

/// Measurement harness: what the typed tier adds on a real repository, and
/// what it costs.
///
/// Ignored by default because it needs a repository to point at. Run it with:
///
/// ```sh
/// POLINT_TS_TYPES_MEASURE_REPO=/path/to/repo \
///   cargo test -p polint --lib --all-features \
///   analysis_kernel::ts_types_tests::measure_type_directed_tier \
///   -- --exact --ignored --nocapture
/// ```
///
/// The two configurations alternate inside one process, so a shared host's
/// load affects both arms equally instead of being attributed to whichever ran
/// second.
#[test]
#[ignore = "measurement harness: set POLINT_TS_TYPES_MEASURE_REPO"]
fn measure_type_directed_tier() {
    let Some(repo) = std::env::var_os("POLINT_TS_TYPES_MEASURE_REPO").map(PathBuf::from) else {
        eprintln!("SKIP measure_type_directed_tier: set POLINT_TS_TYPES_MEASURE_REPO");
        return;
    };
    let samples: usize = std::env::var("POLINT_TS_TYPES_MEASURE_SAMPLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);

    let mut on_times = Vec::new();
    let mut off_times = Vec::new();
    let mut last_on = None;
    let mut last_off = None;

    for sample in 0..samples {
        for enabled in [true, false] {
            let started = std::time::Instant::now();
            let output = measure_run(&repo, enabled);
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            if enabled {
                on_times.push(elapsed);
                last_on = Some(output);
            } else {
                off_times.push(elapsed);
                last_off = Some(output);
            }
            eprintln!("sample {sample} type_sidecar={enabled} wall_ms={elapsed:.0}",);
        }
    }

    let on = last_on.expect("at least one sample");
    let off = last_off.expect("at least one sample");
    report_tier_contribution("type_sidecar=on", &on);
    report_tier_contribution("type_sidecar=off", &off);

    let on_sites = resolved_sites(&on);
    let off_sites = resolved_sites(&off);
    let gained = on_sites.difference(&off_sites).count();
    let lost = off_sites.difference(&on_sites).count();
    eprintln!(
        "call sites with a resolved target, tier on:  {}",
        on_sites.len()
    );
    eprintln!(
        "call sites with a resolved target, tier off: {}",
        off_sites.len()
    );
    eprintln!("gained by the typed tier: {gained}");
    eprintln!("lost with the typed tier: {lost}");
    eprintln!(
        "wall ms median on={:.0} off={:.0}",
        median(&mut on_times.clone()),
        median(&mut off_times.clone())
    );
    for (key, value) in provider_counts(&on) {
        eprintln!("counter {key} = {value}");
    }
}

fn measure_run(repo: &Path, enabled: bool) -> crate::analysis_kernel::KernelOutput {
    std::fs::write(
        repo.join(".polint.toml"),
        format!("[languages.ts]\ntype_sidecar = {enabled}\n"),
    )
    .expect("write .polint.toml");
    let loaded = load_config(repo).expect("config loads");
    let cache = Cache::new("", false);
    let plan = AnalysisPlan::from_capability_names_for_test(&["calls"]);
    AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &cache,
        config_digest: "measure",
        rule_digest: "measure",
        plan: &plan,
        parallel: true,
    })
    .expect("kernel should run")
}

fn report_tier_contribution(label: &str, output: &crate::analysis_kernel::KernelOutput) {
    let mut by_tier: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for edge in output.db.refined_call_edges() {
        *by_tier.entry(format!("{:?}", edge.tier)).or_default() += 1;
    }
    eprintln!("--- {label} ---");
    eprintln!("files {}", output.db.files().len());
    eprintln!("call sites {}", output.db.call_sites().len());
    eprintln!("refined edges {}", output.db.refined_call_edges().len());
    eprintln!("ts type callsites {}", output.db.ts_type_callsites().len());
    for (tier, count) in by_tier {
        eprintln!("tier {tier} = {count}");
    }
}

fn resolved_sites(
    output: &crate::analysis_kernel::KernelOutput,
) -> std::collections::BTreeSet<(u64, u64)> {
    output
        .db
        .refined_call_edges()
        .iter()
        .filter(|edge| {
            edge.status == crate::analysis_neutral::calls::facts::CallTargetStatus::Resolved
                && edge.target_function.is_some()
        })
        .filter_map(|edge| {
            let site = output
                .db
                .call_sites()
                .iter()
                .find(|site| site.id == edge.site)?;
            Some((u64::from(site.file.0), u64::from(site.span.start_byte)))
        })
        .collect()
}

fn provider_counts(
    output: &crate::analysis_kernel::KernelOutput,
) -> std::collections::BTreeMap<String, u64> {
    output
        .run_report
        .provider_telemetry
        .iter()
        .filter(|row| row.provider_id == "polint.ts.types")
        .flat_map(|row| row.counts.clone())
        .collect()
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|left, right| left.partial_cmp(right).expect("finite timings"));
    values[values.len() / 2]
}

/// Measurement harness: what one sidecar invocation costs cold and warm.
///
/// The whole-pipeline harness above runs with the cache disabled so both of its
/// arms stay comparable, which leaves the cached path unmeasured. This drives
/// the client directly against one cache directory, so the first pass is the
/// cold sidecar and the rest are the stored NDJSON being replayed.
///
/// ```sh
/// POLINT_TS_TYPES_MEASURE_REPO=/path/to/repo \
///   cargo test -p polint --lib --all-features --release \
///   analysis_kernel::ts_types_tests::measure_sidecar_cold_and_warm \
///   -- --exact --ignored --nocapture
/// ```
#[test]
#[ignore = "measurement harness: set POLINT_TS_TYPES_MEASURE_REPO"]
fn measure_sidecar_cold_and_warm() {
    let Some(repo) = std::env::var_os("POLINT_TS_TYPES_MEASURE_REPO").map(PathBuf::from) else {
        eprintln!("SKIP measure_sidecar_cold_and_warm: set POLINT_TS_TYPES_MEASURE_REPO");
        return;
    };
    let mut files = Vec::new();
    collect_ts_files(&repo, &repo, &mut files);
    files.sort();
    let config = crate::ts::types::lifecycle::TsTypesConfig::from_settings_files(
        &repo,
        &std::collections::BTreeMap::new(),
        &files,
    )
    .expect("lifecycle resolves");
    eprintln!(
        "discovered {} TS/JS files across {} project(s)",
        files.len(),
        config.projects.len()
    );

    let cache = tempfile::tempdir().expect("sidecar cache dir");
    let client = crate::ts::types::client::TsTypesClient::new(repo.clone(), &config);
    for pass in 0..3 {
        let started = std::time::Instant::now();
        let run = client
            .run_cached(&config, cache.path(), "measure-upstream")
            .expect("sidecar run");
        eprintln!(
            "pass {pass} ({}) wall_ms={:.0} rows={} sidecar_self_reported_ms={}",
            if pass == 0 { "cold" } else { "warm" },
            started.elapsed().as_secs_f64() * 1000.0,
            run.output.rows.len(),
            run.output.totals.elapsed_ms
        );
    }
}

/// Repository-relative TS/JS paths, skipping the directories a scan never
/// discovers.
fn collect_ts_files(root: &Path, directory: &Path, files: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if matches!(
                name.as_ref(),
                "node_modules" | ".git" | "lib" | "dist" | "tmp"
            ) {
                continue;
            }
            collect_ts_files(root, &path, files);
            continue;
        }
        let is_source = matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs")
        ) && !name.ends_with(".d.ts");
        if is_source && let Ok(relative) = path.strip_prefix(root) {
            files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}
