pub(crate) mod go_x_tools_callgraph;
pub(crate) mod gosec;
pub(crate) mod jelly_callgraph;
pub(crate) mod secbench_js;

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::eval::adapter::BenchmarkAdapter;
    use crate::eval::bench::report::{
        GraphAccuracyBaseline, load_graph_accuracy_baseline, write_graph_accuracy_baseline,
    };
    use crate::eval::external::go_x_tools_callgraph::GoXToolsCallgraphAdapter;
    use crate::eval::external::jelly_callgraph::JellyCallgraphAdapter;
    use crate::eval::model::EvaluationMode;
    use crate::eval::report::EvaluationRun;
    use crate::eval::runner::run_external_suite_for_test;
    use crate::eval::suite::{LocalClonePolicy, SuiteTier};

    /// Maximum allowed F1 regression against the committed graph-accuracy baseline.
    const F1_REGRESSION_TOLERANCE: f64 = 0.005;

    #[test]
    fn committed_graph_accuracy_baseline_is_measured() {
        let baseline = load_committed_baseline();
        assert!(
            !baseline.rows.is_empty(),
            "committed persisted-graph-accuracy.json must contain suite rows"
        );
        for row in &baseline.rows {
            assert!(
                row.recall.is_some() && row.precision.is_some(),
                "suite {} must be a measured baseline (non-null recall/precision); \
                 regenerate with POLINT_WRITE_GRAPH_BENCH=1 and POLINT_GRAPH_BENCH_TIER=release",
                row.suite_id
            );
            assert!(
                row.graph_edges_expected > 0,
                "measured suite {} must record a non-zero graph_edges_expected",
                row.suite_id
            );
            let f1 = f1_from_precision_recall(row.precision, row.recall)
                .expect("measured row must yield an F1");
            assert!(
                f1.is_finite() && (0.0..=1.0).contains(&f1),
                "suite {} F1 must be in [0, 1], got {f1}",
                row.suite_id
            );
        }
    }

    /// Measurement harness for the Jelly call-graph lane on its own.
    ///
    /// The gate above needs both oracle clones and compares against the
    /// committed baseline. This one runs only Jelly and prints what it
    /// measured, so the same code can be run on two trees and their numbers
    /// compared directly.
    ///
    /// ```sh
    /// cargo test -p polint --lib --all-features --release \
    ///   eval::external::tests::measure_jelly_callgraph_lane \
    ///   -- --exact --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "measurement harness: needs the Jelly clone"]
    fn measure_jelly_callgraph_lane() {
        let root = repo_root();
        let manifest_path =
            root.join("research/evaluation-harness/suites/jelly-callgraph-micro.toml");
        let repo = root.join("research/evaluation-harness/repos/jelly");
        if !repo.exists() {
            eprintln!(
                "SKIP measure_jelly_callgraph_lane: missing {}",
                repo.display()
            );
            return;
        }
        let mut suite = JellyCallgraphAdapter
            .load_manifest(&std::fs::read_to_string(&manifest_path).unwrap())
            .unwrap();
        suite.checkout.path = repo.to_string_lossy().to_string();
        suite.checkout.local_clone_policy = LocalClonePolicy::AllowAbsolute;

        let output_dir = tempfile::tempdir().unwrap();
        let artifacts = run_external_suite_for_test(
            &JellyCallgraphAdapter,
            &suite,
            graph_bench_tier(),
            EvaluationMode::PolintBaseline,
            output_dir.path(),
        )
        .unwrap();
        let run = read_run(&artifacts.json_path);

        eprintln!("jelly lane cases            {}", run.cases.len());
        eprintln!(
            "jelly lane edges_expected   {}",
            run.metrics.graph_edges_expected
        );
        eprintln!(
            "jelly lane edges_observed   {}",
            run.metrics.graph_edges_observed
        );
        eprintln!("jelly lane unknown_count    {}", run.metrics.unknown_count);
        eprintln!("jelly lane recall           {:?}", run.metrics.recall);
        eprintln!("jelly lane precision        {:?}", run.metrics.precision);
        eprintln!(
            "jelly lane f1               {:?}",
            f1_from_precision_recall(run.metrics.precision, run.metrics.recall)
        );
    }

    #[test]
    fn external_graph_baseline_reports_can_be_generated() {
        let root = repo_root();
        let go_manifest =
            root.join("research/evaluation-harness/suites/go-x-tools-rta-callgraph.toml");
        let jelly_manifest =
            root.join("research/evaluation-harness/suites/jelly-callgraph-micro.toml");
        let go_repo = root.join("research/evaluation-harness/repos/golang-tools");
        let jelly_repo = root.join("research/evaluation-harness/repos/jelly");
        if !go_repo.exists() || !jelly_repo.exists() {
            let message = format!(
                "SKIP external_graph_baseline_reports_can_be_generated: missing benchmark \
                 clone(s); need both {} and {} (set POLINT_REQUIRE_BENCH_CORPUS=1 to fail \
                 instead of skip)",
                go_repo.display(),
                jelly_repo.display()
            );
            if std::env::var_os("POLINT_REQUIRE_BENCH_CORPUS").is_some() {
                panic!("{message}");
            }
            eprintln!("{message}");
            return;
        }

        let committed = load_committed_baseline();
        let output_dir = if std::env::var_os("POLINT_WRITE_GRAPH_BENCH").is_some() {
            root.join(".context/graph-benchmarks")
        } else {
            tempfile::tempdir().unwrap().keep()
        };

        let mut go_suite = GoXToolsCallgraphAdapter
            .load_manifest(&std::fs::read_to_string(&go_manifest).unwrap())
            .unwrap();
        go_suite.checkout.path = go_repo.to_string_lossy().to_string();
        go_suite.checkout.local_clone_policy = LocalClonePolicy::AllowAbsolute;
        let mut jelly_suite = JellyCallgraphAdapter
            .load_manifest(&std::fs::read_to_string(&jelly_manifest).unwrap())
            .unwrap();
        jelly_suite.checkout.path = jelly_repo.to_string_lossy().to_string();
        jelly_suite.checkout.local_clone_policy = LocalClonePolicy::AllowAbsolute;

        // The committed accuracy baseline is the full-suite (release) measurement.
        // Keep the gate on that tier so F1 is compared apples-to-apples.
        let tier = graph_bench_tier();
        let go_artifacts = run_external_suite_for_test(
            &GoXToolsCallgraphAdapter,
            &go_suite,
            tier,
            EvaluationMode::PolintBaseline,
            &output_dir,
        )
        .unwrap();
        let jelly_artifacts = run_external_suite_for_test(
            &JellyCallgraphAdapter,
            &jelly_suite,
            tier,
            EvaluationMode::PolintBaseline,
            &output_dir,
        )
        .unwrap();

        let go = read_run(&go_artifacts.json_path);
        let jelly = read_run(&jelly_artifacts.json_path);
        assert!(!go.cases.is_empty());
        assert!(!jelly.cases.is_empty());
        assert!(go.metrics.graph_edges_expected > 0);
        assert!(jelly.metrics.graph_edges_expected > 0);
        assert!(!go.output_hash.is_empty());
        assert!(!jelly.output_hash.is_empty());
        assert!(go.comparison_rows.len() >= 2);
        assert!(jelly.comparison_rows.len() >= 2);

        assert_f1_against_baseline(&go, &committed);
        assert_f1_against_baseline(&jelly, &committed);

        // Cost columns must be recorded on every benchmark iteration — the
        // discipline that was dropped from the Jelly iteration log. Coexists
        // with the F1 accuracy gate above (same run, same baseline JSON).
        for run in [&go, &jelly] {
            assert!(
                !run.cases.is_empty(),
                "{} must produce scored cases",
                run.suite_id
            );
            for case in &run.cases {
                assert!(
                    case.runtime.observed_runtime_ms.is_some(),
                    "{} case {} missing runtime_ms cost column",
                    run.suite_id,
                    case.case_id
                );
                assert!(
                    case.runtime.peak_rss_bytes.is_some_and(|bytes| bytes > 0),
                    "{} case {} missing peak_rss_bytes cost column",
                    run.suite_id,
                    case.case_id
                );
            }
            let performance = run
                .performance
                .as_ref()
                .expect("external suite runs must populate performance cost columns");
            assert!(performance.runtime.observed_runtime_ms.is_some());
            assert!(
                performance
                    .runtime
                    .peak_rss_bytes
                    .is_some_and(|bytes| bytes > 0)
            );
            assert!(performance.rss.peak_rss_observed_mb.is_some());
        }

        let measured = GraphAccuracyBaseline::from_runs(&[&go, &jelly]);
        for measured_row in &measured.rows {
            let baseline_row = committed
                .rows
                .iter()
                .find(|row| row.suite_id == measured_row.suite_id)
                .unwrap_or(measured_row);
            let cost_report = crate::eval::bench::gate::evaluate_cost_columns_budget(
                baseline_row,
                measured_row,
                crate::eval::bench::gate::DEFAULT_MAX_COST_COLUMN_RATIO,
            );
            assert!(
                !crate::eval::bench::gate::is_blocking(&cost_report),
                "cost-column budget breached for {}: {cost_report:#?}",
                measured_row.suite_id
            );
        }

        if std::env::var_os("POLINT_WRITE_GRAPH_BENCH").is_some() {
            // Refresh the committed pre-store persisted-graph accuracy baseline
            // with the real measured recall/precision. Rows read the metrics off
            // the produced runs; scoring is not reimplemented here.
            let baseline = GraphAccuracyBaseline::from_runs(&[&go, &jelly]);
            write_graph_accuracy_baseline(
                &root.join("research/evaluation-harness/baselines/persisted-graph-accuracy.json"),
                &baseline,
            )
            .unwrap();
            write_summary(&output_dir, &[go, jelly]).unwrap();
        }
    }

    fn assert_f1_against_baseline(run: &EvaluationRun, baseline: &GraphAccuracyBaseline) {
        let row = baseline
            .rows
            .iter()
            .find(|row| row.suite_id == run.suite_id)
            .unwrap_or_else(|| {
                panic!(
                    "committed baseline missing suite {}; regenerate with \
                     POLINT_WRITE_GRAPH_BENCH=1",
                    run.suite_id
                )
            });
        let baseline_f1 =
            f1_from_precision_recall(row.precision, row.recall).unwrap_or_else(|| {
                panic!(
                    "suite {}: committed baseline is still a null stub; regenerate with \
                 POLINT_WRITE_GRAPH_BENCH=1 and POLINT_GRAPH_BENCH_TIER=release",
                    run.suite_id
                )
            });
        let f1 = run.metrics.f1.unwrap_or_else(|| {
            panic!(
                "suite {}: evaluation run must compute F1 when the accuracy gate runs",
                run.suite_id
            )
        });
        assert!(
            f1 >= baseline_f1 - F1_REGRESSION_TOLERANCE,
            "suite {}: F1 {f1:.6} dropped below committed baseline {baseline_f1:.6} \
             (tolerance {F1_REGRESSION_TOLERANCE})",
            run.suite_id
        );
    }

    fn f1_from_precision_recall(precision: Option<f64>, recall: Option<f64>) -> Option<f64> {
        match (precision, recall) {
            (Some(precision), Some(recall)) if precision + recall > 0.0 => {
                Some(2.0 * precision * recall / (precision + recall))
            }
            _ => None,
        }
    }

    fn load_committed_baseline() -> GraphAccuracyBaseline {
        let path =
            repo_root().join("research/evaluation-harness/baselines/persisted-graph-accuracy.json");
        load_graph_accuracy_baseline(&path).unwrap_or_else(|error| {
            panic!("committed persisted-graph-accuracy.json must load: {error}")
        })
    }

    fn graph_bench_tier() -> SuiteTier {
        // Default to release: the committed F1 floor is the full-suite measurement.
        match std::env::var("POLINT_GRAPH_BENCH_TIER")
            .unwrap_or_else(|_| "release".to_string())
            .as_str()
        {
            "fast" => SuiteTier::Fast,
            "nightly" => SuiteTier::Nightly,
            "release" => SuiteTier::Release,
            other => panic!("unsupported POLINT_GRAPH_BENCH_TIER: {other}"),
        }
    }

    fn read_run(path: &Path) -> EvaluationRun {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn write_summary(output_dir: &Path, runs: &[EvaluationRun]) -> anyhow::Result<()> {
        let mut markdown = String::from(
            "# Graph Benchmark Summary\n\n| Suite | Mode | TP | FP | FN | Precision | Recall | Unknowns | Runtime ms | Peak RSS bytes | Output hash |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|\n",
        );
        for run in runs {
            let runtime_ms = run
                .cases
                .iter()
                .filter_map(|case| case.runtime.observed_runtime_ms)
                .sum::<u64>();
            let peak_rss_bytes = run
                .cases
                .iter()
                .filter_map(|case| case.runtime.peak_rss_bytes)
                .max();
            markdown.push_str(&format!(
                "| {} | {:?} | {} | {} | {} | {} | {} | {} | {} | {} | `{}` |\n",
                run.suite_id,
                run.mode,
                run.metrics.true_positives,
                run.metrics.false_positives,
                run.metrics.false_negatives,
                pct(run.metrics.precision),
                pct(run.metrics.recall),
                run.metrics.unknown_count,
                runtime_ms,
                peak_rss_bytes
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                run.output_hash
            ));
        }
        std::fs::create_dir_all(output_dir)?;
        std::fs::write(output_dir.join("summary.md"), markdown)?;
        Ok(())
    }

    fn pct(value: Option<f64>) -> String {
        value
            .map(|value| format!("{:.4}", value))
            .unwrap_or_else(|| "n/a".to_string())
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("workspace root")
            .to_path_buf()
    }
}
