//! The taint corpus: Go cases, each a positive whose `// want-flow` lines are the
//! sinks a question must report and twins it must report nothing in, answered
//! through the analysis kernel and `DataFlow::flows`, scored for precision and
//! recall.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::sdk::facts::{FlowSink, FlowSource, FlowSpec, FlowValueKind};

const CORPUS_SCHEMA: &str = "polint-taint-corpus-1";

/// The corpus gate: the share of reported sinks that are wanted, and of wanted
/// sinks that are reported.
const MINIMUM_PRECISION: f64 = 0.9;
const MINIMUM_RECALL: f64 = 0.7;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema: String,
    case: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    category: String,
    source: Source,
    sink: Sink,
    #[serde(default)]
    sanitizers: Vec<String>,
    positive: String,
    twins: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    named: Option<Vec<String>>,
    #[serde(default)]
    call_result: Option<String>,
    #[serde(default)]
    parameter_type: Option<String>,
    #[serde(default)]
    callback_parameter: Option<Callback>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Callback {
    callee: String,
    argument: usize,
    parameter: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sink {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    returned: Option<bool>,
    #[serde(default)]
    call_argument: Option<CallArgument>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallArgument {
    callee: String,
    #[serde(default)]
    argument: Option<usize>,
}

impl Case {
    fn spec(&self) -> FlowSpec {
        let source = if let Some(kind) = &self.source.model {
            FlowSource::model(kind)
        } else if let Some(names) = &self.source.named {
            FlowSource::named(names.iter().cloned())
        } else if let Some(function) = &self.source.call_result {
            FlowSource::call_result(function)
        } else if let Some(type_name) = &self.source.parameter_type {
            FlowSource::parameter_of_type(type_name)
        } else if let Some(callback) = &self.source.callback_parameter {
            FlowSource::callback_parameter(&callback.callee, callback.argument, callback.parameter)
        } else {
            panic!("case {} has no source", self.id)
        };
        let sink = if let Some(kind) = &self.sink.model {
            FlowSink::model(kind)
        } else if self.sink.returned == Some(true) {
            FlowSink::returned()
        } else if let Some(call) = &self.sink.call_argument {
            match call.argument {
                Some(position) => FlowSink::call_argument(&call.callee, position),
                None => FlowSink::call(&call.callee),
            }
        } else {
            panic!("case {} has no sink", self.id)
        };
        let mut spec = FlowSpec::new().source(source).sink(sink);
        for sanitizer in &self.sanitizers {
            spec = spec.sanitizer(sanitizer);
        }
        // Questions about contexts and tenant scopes follow values through
        // contexts; the injection-style ones track text, which no context,
        // boolean or number carries.
        if !matches!(self.category.as_str(), "context" | "tenant-scope") {
            spec = spec
                .untracked(FlowValueKind::Context)
                .untracked(FlowValueKind::Boolean)
                .untracked(FlowValueKind::Number);
        }
        spec
    }
}

#[derive(Default)]
struct Score {
    true_positives: usize,
    false_positives: usize,
    false_negatives: usize,
}

impl Score {
    fn add(&mut self, other: &Score) {
        self.true_positives += other.true_positives;
        self.false_positives += other.false_positives;
        self.false_negatives += other.false_negatives;
    }

    fn precision(&self) -> f64 {
        self.true_positives as f64 / (self.true_positives + self.false_positives).max(1) as f64
    }

    fn recall(&self) -> f64 {
        self.true_positives as f64 / (self.true_positives + self.false_negatives).max(1) as f64
    }
}

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("polint crate should live under crates/")
        .join("tests/taint-corpus")
}

fn copy_dir(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create corpus copy directory");
    for entry in fs::read_dir(source).expect("read corpus directory") {
        let entry = entry.expect("corpus entry");
        let target = destination.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy corpus file");
        }
    }
}

/// The lines marked `// want-flow` in a case directory's files.
fn wanted_lines(root: &Path, directory: &str) -> BTreeSet<(String, u32)> {
    let mut lines = BTreeSet::new();
    let path = root.join(directory);
    for entry in fs::read_dir(&path).expect("read case directory") {
        let entry = entry.expect("case entry");
        let text = fs::read_to_string(entry.path()).expect("read case file");
        let relative = format!("{directory}/{}", entry.file_name().to_string_lossy());
        for (index, line) in text.lines().enumerate() {
            if line.contains("// want-flow") {
                lines.insert((relative.clone(), index as u32 + 1));
            }
        }
    }
    lines
}

/// The sinks a question reports inside one case directory.
fn reported_lines(
    db: &crate::core::AnalysisDb,
    spec: &FlowSpec,
    directory: &str,
) -> BTreeSet<(String, u32)> {
    let prefix = format!("{directory}/");
    crate::flow_queries::flows(db, spec)
        .flows
        .into_iter()
        .filter(|flow| flow.sink.path.starts_with(&prefix))
        .map(|flow| (flow.sink.path, flow.sink.line))
        .collect()
}

#[test]
fn taint_corpus_meets_its_precision_and_recall_gate() {
    let root = corpus_root();
    let raw = fs::read_to_string(root.join("corpus.toml")).expect("read the corpus manifest");
    let corpus: Corpus = toml::from_str(&raw).expect("parse the corpus manifest");
    assert_eq!(corpus.schema, CORPUS_SCHEMA);
    assert!(corpus.case.len() >= 40, "{} cases", corpus.case.len());

    let temp = tempfile::tempdir().expect("corpus temp repo");
    copy_dir(&root, temp.path());
    let output = crate::eval::observed::run_kernel_for_repo_for_test(temp.path())
        .unwrap_or_else(|error| panic!("run the taint corpus: {error:#}"));
    let db = &output.db;
    assert!(
        db.go_flow_program().is_some(),
        "the kernel loaded no Go flow program for the corpus"
    );

    let mut total = Score::default();
    let mut by_category: BTreeMap<String, Score> = BTreeMap::new();
    let mut misses = Vec::new();
    for case in &corpus.case {
        let spec = case.spec();
        let wanted = wanted_lines(&root, &case.positive);
        assert!(!wanted.is_empty(), "case {} marks no wanted flow", case.id);
        let reported = reported_lines(db, &spec, &case.positive);
        let mut score = Score {
            true_positives: wanted.intersection(&reported).count(),
            false_positives: reported.difference(&wanted).count(),
            false_negatives: wanted.difference(&reported).count(),
        };
        if score.false_positives > 0 || score.false_negatives > 0 {
            misses.push(format!(
                "{} positive: wanted {} reported {}",
                case.id,
                wanted.len(),
                reported.len()
            ));
        }
        for twin in &case.twins {
            let reported = reported_lines(db, &spec, twin);
            if !reported.is_empty() {
                misses.push(format!("{} {twin}: reported {}", case.id, reported.len()));
            }
            score.false_positives += reported.len();
        }
        by_category
            .entry(case.category.clone())
            .or_default()
            .add(&score);
        total.add(&score);
    }

    let mut report = format!(
        "taint corpus: {} cases, tp={} fp={} fn={} precision={:.3} recall={:.3}\n",
        corpus.case.len(),
        total.true_positives,
        total.false_positives,
        total.false_negatives,
        total.precision(),
        total.recall()
    );
    for (category, score) in &by_category {
        report.push_str(&format!(
            "  {category}: tp={} fp={} fn={}\n",
            score.true_positives, score.false_positives, score.false_negatives
        ));
    }
    for miss in &misses {
        report.push_str(&format!("  miss: {miss}\n"));
    }
    eprintln!("{report}");
    assert!(
        total.precision() >= MINIMUM_PRECISION && total.recall() >= MINIMUM_RECALL,
        "{report}"
    );
}
