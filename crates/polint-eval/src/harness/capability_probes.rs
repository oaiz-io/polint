//! Capability-level probes over tiny standalone Go and TypeScript repositories.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::analysis_neutral::calls::facts::CallCallee;
use crate::analysis_neutral::domains::facts::{DomainLocation, DomainSlot, DomainValue};
use crate::analysis_neutral::refined_calls::facts::{RefinedCallConfidence, RefinedCallValidation};
use crate::core::{Rule, RuleOptions, run_rules};
use crate::eval::matcher::{MatchOutcome, MatcherConfig, match_case};
use crate::eval::model::{
    AssertionMode, ExpectedDiagnostic, ExpectedItem, ObservedDiagnostic, ObservedItem,
    ObservedStatus,
};
use crate::ir::{MirOperationKind, MirValue, PlaceId};
use crate::sdk::prelude::*;

const SUITE_SCHEMA: &str = "polint-capability-probes-1";
const PROBE_RULE_ID: &str = "capability/probe";

/// Levels whose claims CI enforces. A claim with no probes is a dropped claim,
/// so certification is checked against this list rather than against whichever
/// buckets the manifest happens to produce.
const CERTIFIED_LEVELS: [Level; 3] = [Level::L1, Level::L2, Level::L3];
const PROBE_LANGUAGES: [ProbeLanguage; 2] = [ProbeLanguage::Go, ProbeLanguage::Typescript];

/// Floor on cases per certified level and language, so shrinking a bucket to a
/// single easy probe cannot keep the gate green.
const MINIMUM_CERTIFIED_CASES: usize = 4;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeSuite {
    schema: String,
    probe: Vec<Probe>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
enum Level {
    L1,
    L2,
    L3,
    L4,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
enum ProbeLanguage {
    Go,
    Typescript,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    id: String,
    level: Level,
    language: ProbeLanguage,
    detector: String,
    positive: String,
    twins: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct CaseResult {
    probe_id: String,
    level: Level,
    language: ProbeLanguage,
    case: String,
    positive: bool,
    reported: bool,
    passed: bool,
}

#[derive(Default)]
struct Bucket {
    positive_passed: usize,
    positive_total: usize,
    twins_passed: usize,
    twins_total: usize,
    failures: Vec<String>,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("polint crate should live under crates/")
        .to_path_buf()
}

fn suite_root() -> PathBuf {
    repo_root().join("tests/capability-probes")
}

fn load_suite() -> ProbeSuite {
    let path = suite_root().join("probes.toml");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read capability probe suite {}: {error}", path.display()));
    let suite: ProbeSuite = toml::from_str(&raw)
        .unwrap_or_else(|error| panic!("parse capability probe suite {}: {error}", path.display()));
    assert_eq!(suite.schema, SUITE_SCHEMA, "unexpected probe suite schema");
    suite
}

fn copy_dir(source: &Path, destination: &Path) {
    fs::create_dir_all(destination)
        .unwrap_or_else(|error| panic!("create probe temp dir {}: {error}", destination.display()));
    let mut entries = fs::read_dir(source)
        .unwrap_or_else(|error| panic!("read probe case {}: {error}", source.display()))
        .collect::<Result<Vec<_>, _>>()
        .expect("read probe case entries");
    entries.sort_by_key(fs::DirEntry::path);
    for entry in entries {
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let file_type = entry.file_type().expect("inspect probe case entry");
        assert!(
            !file_type.is_symlink(),
            "probe cases must not contain symlinks"
        );
        if file_type.is_dir() {
            copy_dir(&from, &to);
        } else if file_type.is_file() {
            fs::copy(&from, &to).unwrap_or_else(|error| {
                panic!(
                    "copy probe case {} to {}: {error}",
                    from.display(),
                    to.display()
                )
            });
        }
    }
}

fn setting(ctx: &RuleCtx<'_>, name: &str) -> String {
    ctx.options()
        .settings
        .get(name)
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| panic!("probe rule requires string setting `{name}`"))
        .to_string()
}

#[polint::rule(
    id = "capability/probe",
    description = "Match a syntactic call event",
    severity = "error"
)]
fn event_probe(ctx: &mut RuleCtx<'_>, events: Events<'_>) -> RuleResult {
    let target = setting(ctx, "target");
    for violation in events.matching(EventPattern::call(target)) {
        ctx.report(violation.diagnostic(ctx.rule_id(), "capability probe matched"));
    }
    Ok(())
}

#[polint::rule(
    id = "capability/probe",
    description = "Match an exact syntax string literal",
    severity = "error"
)]
fn string_probe(ctx: &mut RuleCtx<'_>, literals: StringLiterals<'_>) -> RuleResult {
    let expected = setting(ctx, "value");
    for literal in literals.iter().filter(|literal| literal.value == expected) {
        ctx.error(&literal.span, "capability probe matched");
    }
    Ok(())
}

#[polint::rule(
    id = "capability/probe",
    description = "Match an exact syntax import",
    severity = "error"
)]
fn import_probe(ctx: &mut RuleCtx<'_>, imports: Imports<'_>) -> RuleResult {
    let expected = setting(ctx, "path");
    for import in imports.iter().filter(|import| import.path == expected) {
        ctx.error(&import.span, "capability probe matched");
    }
    Ok(())
}

#[polint::rule(
    id = "capability/probe",
    description = "Match a same-function missing guard",
    severity = "error"
)]
fn guard_probe(ctx: &mut RuleCtx<'_>, control: ControlFlow<'_>) -> RuleResult {
    let event = setting(ctx, "event");
    let guard = setting(ctx, "guard");
    let mut query = GuardQuery::new(EventPattern::call(event), GuardPattern::call_any([guard]));
    query.max_paths = 64;
    for violation in control.missing_guard(query) {
        ctx.report(violation.diagnostic(ctx.rule_id(), "capability probe matched"));
    }
    Ok(())
}

#[polint::rule(
    id = "capability/probe",
    description = "Match a same-function missing cleanup",
    severity = "error"
)]
fn cleanup_probe(ctx: &mut RuleCtx<'_>, control: ControlFlow<'_>) -> RuleResult {
    let start = setting(ctx, "start");
    let cleanup = setting(ctx, "cleanup");
    let mut query = LifecycleQuery::new(EventPattern::call(start), EventPattern::call(cleanup));
    query.require_error_cleanup = true;
    query.max_paths = 64;
    for violation in control.missing_cleanup(query) {
        ctx.report(violation.diagnostic(ctx.rule_id(), "capability probe matched"));
    }
    Ok(())
}

#[polint::rule(
    id = "capability/probe",
    description = "Match a forbidden source-to-sink flow",
    severity = "error"
)]
fn flow_probe(ctx: &mut RuleCtx<'_>, flow: DataFlow<'_>) -> RuleResult {
    let source = setting(ctx, "source");
    let sink = setting(ctx, "sink");
    let barrier = setting(ctx, "barrier");
    let mut query = FlowQuery::new(
        SourcePattern::secret_like([source]),
        SinkPattern::call(sink),
    );
    query.barriers = if barrier.is_empty() {
        BarrierPattern::none()
    } else {
        BarrierPattern::call_any([barrier])
    };
    query.minimum_precision = PolicyPrecision::Heuristic;
    query.max_depth = 16;
    query.max_paths = 64;
    for violation in flow.forbidden(query) {
        ctx.report(violation.diagnostic(ctx.rule_id(), "capability probe matched"));
    }
    Ok(())
}

#[polint::rule(
    id = "capability/probe",
    description = "Match a reachable forbidden call",
    severity = "error"
)]
fn reach_probe(ctx: &mut RuleCtx<'_>, calls: Calls<'_>) -> RuleResult {
    let root = setting(ctx, "root");
    let target = setting(ctx, "target");
    let mut query = ReachQuery::new(EventPattern::call(target));
    query.roots = vec![EventPattern::call(root)];
    query.max_depth = 16;
    query.max_paths = 64;
    query.minimum_precision = PolicyPrecision::Heuristic;
    query.minimum_confidence = PolicyConfidence::Low;
    for violation in calls.forbidden_reachable(query) {
        ctx.report(violation.diagnostic(ctx.rule_id(), "capability probe matched"));
    }
    Ok(())
}

fn rule_detection(
    db: &crate::core::AnalysisDb,
    rule: Rule,
    settings: BTreeMap<String, toml::Value>,
    scope: &str,
) -> bool {
    let options = BTreeMap::from([(
        PROBE_RULE_ID.to_string(),
        RuleOptions {
            settings,
            ..RuleOptions::default()
        },
    )]);
    run_rules(db, &[rule], &options, None, false)
        .iter()
        .any(|diagnostic| {
            diagnostic.rule_id == PROBE_RULE_ID
                && diagnostic.file.trim_start_matches("./").starts_with(scope)
        })
}

fn parse_detector(detector: &str) -> (&str, Vec<&str>) {
    let mut parts = detector.split(':');
    let kind = parts.next().unwrap_or_default();
    (kind, parts.collect())
}

fn settings(values: &[(&str, &str)]) -> BTreeMap<String, toml::Value> {
    values
        .iter()
        .map(|(name, value)| {
            (
                (*name).to_string(),
                toml::Value::String((*value).to_string()),
            )
        })
        .collect()
}

fn file_is_in_scope(db: &crate::core::AnalysisDb, file: FileId, scope: &str) -> bool {
    db.path_for(file)
        .trim_start_matches("./")
        .starts_with(scope)
}

fn call_name(callee: &CallCallee) -> Option<&str> {
    match callee {
        CallCallee::Identifier { name, .. } => Some(name),
        CallCallee::Member { property, .. } => Some(property),
        CallCallee::Constructor { name, .. } => name.as_deref(),
        _ => None,
    }
}

fn direct_literal_argument(
    db: &crate::core::AnalysisDb,
    scope: &str,
    target: &str,
    argument_index: usize,
    expected: &str,
) -> bool {
    db.call_sites().iter().any(|site| {
        if !file_is_in_scope(db, site.file, scope) || call_name(&site.callee) != Some(target) {
            return false;
        }
        let mut literals = db
            .string_literals()
            .iter()
            .filter(|literal| {
                literal.file == site.file
                    && literal.span.start_byte >= site.span.start_byte
                    && literal.span.end_byte <= site.span.end_byte
            })
            .collect::<Vec<_>>();
        literals.sort_by_key(|literal| literal.span.start_byte);
        literals
            .get(argument_index)
            .is_some_and(|literal| literal.value == expected)
    })
}

fn symbol_reference(
    db: &crate::core::AnalysisDb,
    scope: &str,
    name: &str,
    definition_path_fragment: &str,
) -> bool {
    db.symbols().iter().any(|symbol| {
        symbol.name == name
            && symbol.file.is_some_and(|file| {
                db.path_for(file).contains(definition_path_fragment)
                    && db.references().iter().any(|reference| {
                        reference.target == Some(symbol.id)
                            && reference.file.is_some_and(|file| {
                                file_is_in_scope(db, file, scope)
                                    && !db.path_for(file).contains(definition_path_fragment)
                            })
                    })
            })
    })
}

fn unresolved_import(db: &crate::core::AnalysisDb, scope: &str, specifier: &str) -> bool {
    db.imports().iter().any(|import| {
        import.path == specifier
            && file_is_in_scope(db, import.file, scope)
            && db.resolved_imports().iter().any(|resolved| {
                resolved.import == import.id
                    && matches!(
                        resolved.status,
                        ResolutionStatus::Unresolved | ResolutionStatus::SetupMissing
                    )
            })
    })
}

fn module_reachable(
    db: &crate::core::AnalysisDb,
    scope: &str,
    source_fragment: &str,
    target_fragment: &str,
) -> bool {
    let starts = db.module_nodes().iter().filter(|node| {
        node.file.is_some_and(|file| {
            file_is_in_scope(db, file, scope) && db.path_for(file).contains(source_fragment)
        })
    });
    for start in starts {
        let mut seen = BTreeSet::from([start.id]);
        let mut queue = std::collections::VecDeque::from([start.id]);
        while let Some(current) = queue.pop_front() {
            for edge in db.module_edges().iter().filter(|edge| {
                edge.from == current
                    && matches!(
                        edge.status,
                        ResolutionStatus::Resolved | ResolutionStatus::External
                    )
            }) {
                if !seen.insert(edge.to) {
                    continue;
                }
                if db.module_nodes().iter().any(|node| {
                    node.id == edge.to
                        && node.file.is_some_and(|file| {
                            file_is_in_scope(db, file, scope)
                                && db.path_for(file).contains(target_fragment)
                        })
                }) {
                    return true;
                }
                queue.push_back(edge.to);
            }
        }
    }
    false
}

fn domain_slot(label: &str) -> DomainSlot {
    match label {
        "reachability" => DomainSlot::Reachability,
        "nilness" => DomainSlot::Nilness,
        "truthiness" => DomainSlot::Truthiness,
        "constants" => DomainSlot::Constants,
        "strings" => DomainSlot::Strings,
        "initializedness" => DomainSlot::Initializedness,
        _ => panic!("unknown domain slot `{label}`"),
    }
}

fn domain_value_matches(value: &DomainValue, expected: &str) -> bool {
    match value {
        DomainValue::Label(value) => value == expected,
        DomainValue::DigestParts(parts) => parts.iter().any(|part| part.contains(expected)),
        DomainValue::TopReason(reason) => reason == expected,
    }
}

fn call_domain(
    db: &crate::core::AnalysisDb,
    scope: &str,
    target: &str,
    slot: DomainSlot,
    expected: &str,
    argument_index: Option<usize>,
) -> bool {
    if db.abstract_domain_store().is_none() {
        return false;
    }
    db.call_sites().iter().any(|site| {
        if !file_is_in_scope(db, site.file, scope) || call_name(&site.callee) != Some(target) {
            return false;
        }
        let block = db
            .cfg_nodes()
            .iter()
            .find(|node| node.operation == Some(site.operation))
            .map(|node| node.block);
        let places = argument_index
            .and_then(|index| site.arguments.get(index).copied())
            .map(|argument| related_places(db, site.body, site.operation, argument));
        db.abstract_domain_observations().iter().any(|row| {
            row.body == site.body
                && (row.operation == Some(site.operation)
                    && row.location == DomainLocation::BeforeOperation
                    || row.block == block && row.location == DomainLocation::BlockEntry)
                && row.slot == slot
                && places
                    .as_ref()
                    .is_none_or(|places| row.place.is_some_and(|place| places.contains(&place)))
                && domain_value_matches(&row.value, expected)
        })
    })
}

fn related_places(
    db: &crate::core::AnalysisDb,
    body: crate::ir::MirBodyId,
    before: crate::ir::MirOpId,
    argument: PlaceId,
) -> BTreeSet<PlaceId> {
    let mut places = BTreeSet::from([argument]);
    loop {
        let mut changed = false;
        let roots = db
            .mir_places()
            .iter()
            .filter(|place| places.contains(&place.id))
            .map(|place| place.root.clone())
            .collect::<BTreeSet<_>>();
        for place in db.mir_places() {
            if roots.contains(&place.root) {
                changed |= places.insert(place.id);
            }
        }
        for operation in db
            .mir_operations()
            .iter()
            .filter(|operation| operation.body == body && operation.id != before)
        {
            let (destination, value) = match &operation.kind {
                MirOperationKind::Bind { place, value }
                | MirOperationKind::Assign { place, value, .. }
                | MirOperationKind::Write { place, value } => (*place, value),
                _ => continue,
            };
            if places.contains(&destination)
                && let MirValue::Place(source) = value
            {
                changed |= places.insert(*source);
            }
        }
        if !changed {
            return places;
        }
    }
}

/// Whether a function is the one a probe names: by its name, or, for a method
/// (named `Type.method`), by the method's own name.
fn names_function(name: &str, wanted: &str) -> bool {
    name == wanted
        || name
            .rsplit_once('.')
            .is_some_and(|(_, method)| method == wanted)
}

fn refined_must_edge(
    db: &crate::core::AnalysisDb,
    scope: &str,
    caller_name: &str,
    target_name: &str,
) -> bool {
    db.refined_call_edges().iter().any(|edge| {
        edge.validation != RefinedCallValidation::Rejected
            && edge.confidence == RefinedCallConfidence::High
            && db.functions().iter().any(|function| {
                function.id == edge.caller
                    && names_function(&function.name, caller_name)
                    && file_is_in_scope(db, function.file, scope)
            })
            && edge.target_function.is_some_and(|target| {
                db.functions().iter().any(|function| {
                    function.id == target
                        && names_function(&function.name, target_name)
                        && file_is_in_scope(db, function.file, scope)
                })
            })
    })
}

fn detect(db: &crate::core::AnalysisDb, detector: &str, scope: &str) -> bool {
    let (kind, args) = parse_detector(detector);
    match (kind, args.as_slice()) {
        ("event", [target]) => {
            rule_detection(db, event_probe(), settings(&[("target", target)]), scope)
        }
        ("string", [value]) => {
            rule_detection(db, string_probe(), settings(&[("value", value)]), scope)
        }
        ("import", [path]) => {
            rule_detection(db, import_probe(), settings(&[("path", path)]), scope)
        }
        ("call_literal", [target, index, value]) => direct_literal_argument(
            db,
            scope,
            target,
            index
                .parse()
                .expect("call_literal index must be an integer"),
            value,
        ),
        ("symbol_ref", [name, definition_path_fragment]) => {
            symbol_reference(db, scope, name, definition_path_fragment)
        }
        ("unresolved_import", [specifier]) => unresolved_import(db, scope, specifier),
        ("module_reachable", [source, target]) => module_reachable(db, scope, source, target),
        ("call_domain", [target, slot, value]) => {
            call_domain(db, scope, target, domain_slot(slot), value, None)
        }
        ("call_domain", [target, slot, value, index]) => call_domain(
            db,
            scope,
            target,
            domain_slot(slot),
            value,
            Some(index.parse().expect("call_domain index must be an integer")),
        ),
        ("guard", [event, guard]) => rule_detection(
            db,
            guard_probe(),
            settings(&[("event", event), ("guard", guard)]),
            scope,
        ),
        ("cleanup", [start, cleanup]) => rule_detection(
            db,
            cleanup_probe(),
            settings(&[("start", start), ("cleanup", cleanup)]),
            scope,
        ),
        ("flow", [source, sink, barrier]) => rule_detection(
            db,
            flow_probe(),
            settings(&[("source", source), ("sink", sink), ("barrier", barrier)]),
            scope,
        ),
        ("reach", [root, target]) => rule_detection(
            db,
            reach_probe(),
            settings(&[("root", root), ("target", target)]),
            scope,
        ),
        ("refined_must", [caller, target]) => refined_must_edge(db, scope, caller, target),
        _ => panic!("unsupported capability probe detector `{detector}`"),
    }
}

fn diagnostic_item(path: &str) -> ObservedItem {
    ObservedItem::Diagnostic(ObservedDiagnostic {
        rule_id: PROBE_RULE_ID.to_string(),
        relative_path: path.to_string(),
        line: Some(1),
        fingerprint: None,
        mode: AssertionMode::Exact,
        producer_id: Some("polint.capability_probes".to_string()),
        provenance: Some("analysis_kernel".to_string()),
        precision: Some("exact".to_string()),
        status: Some(ObservedStatus::Present),
    })
}

fn expected_item(path: &str) -> ExpectedItem {
    ExpectedItem::Diagnostic(ExpectedDiagnostic {
        rule_id: PROBE_RULE_ID.to_string(),
        relative_path: path.to_string(),
        line: Some(1),
        fingerprint: None,
        mode: AssertionMode::Exact,
        false_positive_trap: false,
    })
}

fn case_passes(expected_report: bool, reported: bool, case_path: &str) -> bool {
    let expected = expected_report.then(|| expected_item(case_path));
    let observed = reported.then(|| diagnostic_item(case_path));
    let matches = match_case(
        expected.as_slice(),
        observed.as_slice(),
        MatcherConfig { line_tolerance: 0 },
    );
    matches.iter().all(|row| {
        matches!(
            row.outcome,
            MatchOutcome::TruePositive | MatchOutcome::TrueNegative
        )
    })
}

fn run_case(
    probe: &Probe,
    case_path: &str,
    positive: bool,
    db: &crate::core::AnalysisDb,
) -> CaseResult {
    let source = suite_root().join("repo").join(case_path);
    assert!(source.is_dir(), "missing probe case {}", source.display());
    let reported = detect(db, &probe.detector, case_path);
    CaseResult {
        probe_id: probe.id.clone(),
        level: probe.level,
        language: probe.language,
        case: case_path.to_string(),
        positive,
        reported,
        passed: case_passes(positive, reported, case_path),
    }
}

fn run_suite() -> Vec<CaseResult> {
    let suite = load_suite();
    let temp = tempfile::tempdir().expect("create capability probe suite temp repo");
    copy_dir(&suite_root().join("repo"), temp.path());
    let mut output = crate::eval::observed::run_kernel_for_repo_for_test(temp.path())
        .unwrap_or_else(|error| panic!("run capability probe suite: {error:#}"));
    let solver = crate::analysis_neutral::domains::solver::IdeDomainSolver::new(
        crate::analysis_neutral::domains::solver::SolverPolicy::deterministic(),
    );
    let solved = solver.solve(crate::analysis_neutral::domains::solver::SolverInput::from(
        &output.db,
    ));
    let place_keys = output
        .db
        .mir_places()
        .iter()
        .map(|place| {
            (
                place.id,
                output.db.resolve_stable_key(place.stable_key).to_string(),
            )
        })
        .collect();
    output.db.replace_abstract_domain_facts(
        crate::analysis_neutral::domains::store::DomainOutput::from_results_with_place_keys(
            &output.db.stable_key_interner(),
            solved.results(),
            &place_keys,
        ),
    );
    let mut results = Vec::new();
    for probe in &suite.probe {
        results.push(run_case(probe, &probe.positive, true, &output.db));
        for twin in &probe.twins {
            results.push(run_case(probe, twin, false, &output.db));
        }
    }
    results
}

fn buckets(results: &[CaseResult]) -> BTreeMap<(Level, ProbeLanguage), Bucket> {
    let mut buckets = BTreeMap::<_, Bucket>::new();
    for result in results {
        let bucket = buckets.entry((result.level, result.language)).or_default();
        if result.positive {
            bucket.positive_total += 1;
            bucket.positive_passed += usize::from(result.passed);
        } else {
            bucket.twins_total += 1;
            bucket.twins_passed += usize::from(result.passed);
        }
        if !result.passed {
            bucket.failures.push(format!(
                "{}:{} (expected_report={}, reported={})",
                result.probe_id, result.case, result.positive, result.reported
            ));
        }
    }
    buckets
}

#[test]
fn capability_probe_certification_rollup() {
    let results = run_suite();
    let buckets = buckets(&results);
    for ((level, language), bucket) in &buckets {
        let positive_rate = if bucket.positive_total == 0 {
            0.0
        } else {
            bucket.positive_passed as f64 / bucket.positive_total as f64
        };
        eprintln!(
            "{level:?} {language:?}: positives {}/{} ({:.1}%), twins {}/{} ({:.1}%){}",
            bucket.positive_passed,
            bucket.positive_total,
            positive_rate * 100.0,
            bucket.twins_passed,
            bucket.twins_total,
            if bucket.twins_total == 0 {
                0.0
            } else {
                bucket.twins_passed as f64 / bucket.twins_total as f64 * 100.0
            },
            if *level == Level::L4 { " [seed]" } else { "" }
        );
        for failure in &bucket.failures {
            eprintln!("  - {failure}");
        }
    }

    let mut gate_failures = Vec::new();
    for level in CERTIFIED_LEVELS {
        for language in PROBE_LANGUAGES {
            let Some(bucket) = buckets.get(&(level, language)) else {
                gate_failures.push(format!(
                    "{level:?} {language:?}: no probes ran; a certified claim must be probed"
                ));
                continue;
            };
            if bucket.positive_total < MINIMUM_CERTIFIED_CASES
                || bucket.twins_total < MINIMUM_CERTIFIED_CASES
            {
                gate_failures.push(format!(
                    "{level:?} {language:?}: {} positives and {} twins is below the \
                     {MINIMUM_CERTIFIED_CASES}-case floor for a certified claim",
                    bucket.positive_total, bucket.twins_total
                ));
            }
            let positive_rate = bucket.positive_passed as f64 / bucket.positive_total as f64;
            if positive_rate < 0.95 || bucket.twins_passed != bucket.twins_total {
                gate_failures.push(format!(
                    "{level:?} {language:?}: positives {}/{}, twins {}/{}; failures: {}",
                    bucket.positive_passed,
                    bucket.positive_total,
                    bucket.twins_passed,
                    bucket.twins_total,
                    bucket.failures.join(", ")
                ));
            }
        }
    }
    assert!(
        gate_failures.is_empty(),
        "capability certification failed:\n{}",
        gate_failures.join("\n")
    );
}

#[test]
fn capability_probe_suite_is_deterministic() {
    let first = run_suite();
    let second = run_suite();
    assert_eq!(
        serde_json::to_vec(&first).expect("serialize first probe run"),
        serde_json::to_vec(&second).expect("serialize second probe run")
    );
}

#[test]
fn capability_probe_manifest_has_unique_ids_and_cases() {
    let suite = load_suite();
    let ids = suite
        .probe
        .iter()
        .map(|probe| probe.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), suite.probe.len(), "probe ids must be unique");
    let cases = suite
        .probe
        .iter()
        .flat_map(|probe| std::iter::once(&probe.positive).chain(probe.twins.iter()))
        .collect::<Vec<_>>();
    let unique_cases = cases.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        unique_cases.len(),
        cases.len(),
        "probe case directories must be unique"
    );
    let l4_cases = suite
        .probe
        .iter()
        .filter(|probe| probe.level == Level::L4)
        .map(|probe| 1 + probe.twins.len())
        .sum::<usize>();
    assert_eq!(l4_cases, 60, "the L4 seed must contain exactly 60 cases");

    for level in CERTIFIED_LEVELS {
        for language in PROBE_LANGUAGES {
            let probes = suite
                .probe
                .iter()
                .filter(|probe| probe.level == level && probe.language == language)
                .collect::<Vec<_>>();
            assert!(
                probes.len() >= MINIMUM_CERTIFIED_CASES,
                "{level:?} {language:?} has {} probes; a certified claim needs at least \
                 {MINIMUM_CERTIFIED_CASES}",
                probes.len()
            );
            assert!(
                probes.iter().all(|probe| !probe.twins.is_empty()),
                "{level:?} {language:?} has a probe with no must-not-report twin"
            );
        }
    }
}
