//! `fact_rows_dump`: the I1b fact-row oracle (plan section 1.2, W9 commit 1).
//!
//! I1a compares provider output digests, which is enough for a workstream that
//! moves no provider in or out of a run. It is not enough for W3 and W6: there
//! the digest of every provider whose recipe names a changed provider moves by
//! construction, so the protected quantity has to be read one tier lower, at the
//! fact rows themselves.
//!
//! This writes one file per [`FactFamily`], each holding that family's rows
//! sorted, so that two runs from two checkouts can be compared with `diff -r`.
//! Columns:
//!
//! 1. the canonical stable-key text, resolved through the run's interner;
//! 2. `FactMeta::payload_digest`, whatever the live metadata path wrote;
//! 3. and 4., for the five summary families only, the plaintext parts and the
//!    attributes the FNV recipe folds beside them.
//!
//! Columns 3 and 4 exist because column 2 is a hash: it can say that a row moved
//! but not why, and the W3 allowlist has to be evaluable line by line. On the
//! tree this harness lands on, column 2 of those five families is not even a hash
//! — the SCC closure re-records them through the `AnalysisHost` trait default as
//! `summary:<SummaryId>` text (`analysis_neutral/host.rs`), id-only text that no
//! parts change moves. W3 commit 0 routes that call to the FNV recipe; until both
//! sides of a comparison carry it, column 2 of those families carries no
//! information the key does not, and only columns 1, 3 and 4 are comparable.
//!
//! The entry has to be a test entry: this harness is compiled into `polint` only
//! under `cfg(test)`, `polint-eval` declares no dependency on `polint` and cannot
//! be a binary, and `FactMetaStore::family_rows_with_run_id` and the fact stores
//! are crate-private. It is run as
//!
//! ```sh
//! POLINT_FACT_ROWS_REPO=<repo> POLINT_FACT_ROWS_CAP=calls \
//! POLINT_FACT_ROWS_OUT=<dir> \
//!   cargo test -p polint --lib --all-features --locked --release \
//!     eval::fact_rows_dump::tests::dump_fact_rows -- --exact --ignored --nocapture
//! ```
//!
//! The dump runs on a `--release` test-profile build rather than the release
//! binary the probes run. The rows are the same in both: they are a function of
//! the inputs and the code, not of the build profile, which is the same
//! assumption the determinism gate already rests on.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::analysis_kernel::FactFamily;
use crate::core::AnalysisDb;

/// Label written into `index.txt`. Bump when a column is added, removed or
/// re-ordered, so a comparison across the change fails loudly instead of
/// silently comparing two different shapes.
pub(crate) const DUMP_SCHEMA: &str = "polint-fact-rows-1";

/// Repo to analyse. Required.
const REPO_ENV: &str = "POLINT_FACT_ROWS_REPO";
/// Path-list-separated (`:`) scope inside the repo. Absent or empty == whole repo.
const PATHS_ENV: &str = "POLINT_FACT_ROWS_PATHS";
/// Comma-separated capability names, the `polint unknowns --cap` values. Default `calls`.
const CAP_ENV: &str = "POLINT_FACT_ROWS_CAP";
/// Directory the per-family files are written to. Required.
const OUT_ENV: &str = "POLINT_FACT_ROWS_OUT";

const DEFAULT_CAPABILITY: &str = "calls";

/// The five families whose rows carry a plaintext parts column and an attributes
/// column (I1b columns 3 and 4).
fn summary_family(family: FactFamily) -> bool {
    matches!(
        family,
        FactFamily::SummaryControl
            | FactFamily::SummaryCall
            | FactFamily::SummaryMemory
            | FactFamily::SummaryTito
            | FactFamily::SummaryEvent
    )
}

/// One family's dumped rows and the file they belong in.
pub(crate) struct FamilyDump {
    pub(crate) family: FactFamily,
    pub(crate) rows: Vec<String>,
}

/// What a dump wrote, for the caller's report line.
pub(crate) struct DumpSummary {
    pub(crate) families: usize,
    pub(crate) rows: usize,
}

/// Render every fact family's rows for `db`, sorted, one [`FamilyDump`] per
/// family of [`FactFamily::ALL`] — including the families with no rows, so a
/// family that empties out shows up as a changed file rather than a missing one.
pub(crate) fn fact_row_dump(db: &AnalysisDb) -> Vec<FamilyDump> {
    // Joins for columns 3 and 4. The metadata row's run id is the producing
    // fact's own dense id (`core/db.rs`'s `refresh_summary_metadata` records
    // `fact.id.0`), which is the only exact join back from a row to its fact.
    let summaries: BTreeMap<u64, &_> = db
        .summary_facts()
        .iter()
        .map(|fact| (fact.id.0, fact))
        .collect();
    let events: BTreeMap<u64, &_> = db
        .summary_events()
        .iter()
        .map(|fact| (fact.id.0, fact))
        .collect();

    let mut dumps = Vec::with_capacity(FactFamily::ALL.len());
    for &family in FactFamily::ALL {
        let mut rows = Vec::new();
        for (run_id, meta) in db.fact_meta().family_rows_with_run_id(family) {
            let mut row = String::new();
            let key = db.resolve_stable_key(meta.stable_key);
            row.push_str(&escape(key.as_ref()));
            row.push('\t');
            row.push_str(&escape(&meta.payload_digest));
            if summary_family(family) {
                let (parts, attributes) = if family == FactFamily::SummaryEvent {
                    match events.get(&run_id) {
                        Some(fact) => (
                            format!("{};{}", fact.event_kind, fact.reason),
                            format!(
                                "{};{};{}",
                                fact.domain.as_str(),
                                fact.status.as_str(),
                                fact.precision.as_str()
                            ),
                        ),
                        None => (DANGLING.to_string(), DANGLING.to_string()),
                    }
                } else {
                    match summaries.get(&run_id) {
                        Some(fact) => (
                            fact.payload_digest.clone(),
                            format!(
                                "{};{};{}",
                                fact.status.as_str(),
                                fact.precision.as_str(),
                                fact.provenance.as_str()
                            ),
                        ),
                        None => (DANGLING.to_string(), DANGLING.to_string()),
                    }
                };
                let _ = write!(row, "\t{}\t{}", escape(&parts), escape(&attributes));
            }
            rows.push(row);
        }
        rows.sort();
        dumps.push(FamilyDump { family, rows });
    }
    dumps
}

/// Written into columns 3 and 4 when a metadata row has no fact behind it. A
/// real dump never shows it; if one does, the join is broken, not the row.
const DANGLING: &str = "<no-fact>";

/// Write [`fact_row_dump`] to `out_dir`: `<FamilyLabel>.txt` per family plus an
/// `index.txt` naming the schema and the per-family row counts.
pub(crate) fn write_fact_row_dump(
    db: &AnalysisDb,
    out_dir: &Path,
    capability: &str,
) -> anyhow::Result<DumpSummary> {
    std::fs::create_dir_all(out_dir)?;
    let dumps = fact_row_dump(db);
    let mut index = String::new();
    let _ = writeln!(index, "schema\t{DUMP_SCHEMA}");
    let _ = writeln!(index, "capability\t{capability}");
    let _ = writeln!(index, "files\t{}", db.files().len());
    let mut total = 0usize;
    for dump in &dumps {
        let mut body = String::new();
        for row in &dump.rows {
            body.push_str(row);
            body.push('\n');
        }
        std::fs::write(out_dir.join(format!("{}.txt", dump.family.label())), body)?;
        let _ = writeln!(index, "{}\t{}", dump.family.label(), dump.rows.len());
        total += dump.rows.len();
    }
    let _ = writeln!(index, "rows\t{total}");
    std::fs::write(out_dir.join("index.txt"), index)?;
    Ok(DumpSummary {
        families: dumps.len(),
        rows: total,
    })
}

/// Keep one row on one line and one column in one field. Key text and plaintext
/// parts are engine-produced and carry neither today, but a dump that silently
/// split a row would make a `diff` unreadable rather than failing.
fn escape(value: &str) -> String {
    if !value.contains(['\t', '\n', '\r', '\\']) {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len() + 8);
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

/// Run the kernel over `repo` scoped to `paths` with `capabilities`, exactly as
/// `polint unknowns --cap <cap> <paths>` scopes it, and return the live db.
fn run_for_dump(
    repo: &Path,
    paths: &[PathBuf],
    capabilities: &[&str],
) -> anyhow::Result<AnalysisDb> {
    let loaded = crate::cli::load_config_for_check(repo, paths)?;
    let config_digest = crate::cache::keys::config_hash(&loaded);
    let rule_digest = crate::cache::keys::rule_hash(&[], None, &BTreeMap::new());
    // A dump must not write a cache root into the scanned tree. The gate scripts
    // point every cell at a fresh `POLINT_CACHE_DIR`; without one, run uncached.
    let cache = crate::cache::Cache::default_for_repo(
        repo,
        std::env::var_os(crate::cache::POLINT_CACHE_DIR_ENV).is_some(),
    );
    let plan = crate::analysis_plan::AnalysisPlan::from_capability_names(capabilities);
    let output =
        crate::analysis_kernel::AnalysisKernel::run(crate::analysis_kernel::KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: &config_digest,
            rule_digest: &rule_digest,
            plan: &plan,
            parallel: true,
        })?;
    Ok(output.db)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The I1b oracle entry. Ignored, so a normal `cargo test` never runs it;
    /// `scripts/deep-gate/gate.sh` invokes it by its exact filter path.
    #[test]
    #[ignore = "I1b oracle: run explicitly with POLINT_FACT_ROWS_REPO and POLINT_FACT_ROWS_OUT set"]
    fn dump_fact_rows() {
        let repo = std::env::var_os(REPO_ENV)
            .unwrap_or_else(|| panic!("{REPO_ENV} names the repo to dump; it is required"));
        let out = std::env::var_os(OUT_ENV)
            .unwrap_or_else(|| panic!("{OUT_ENV} names the output directory; it is required"));
        let paths: Vec<PathBuf> = match std::env::var_os(PATHS_ENV) {
            Some(value) => std::env::split_paths(&value)
                .filter(|path| !path.as_os_str().is_empty())
                .collect(),
            None => Vec::new(),
        };
        let capability = std::env::var(CAP_ENV).unwrap_or_else(|_| DEFAULT_CAPABILITY.to_string());
        let capabilities: Vec<&str> = capability
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect();
        assert!(!capabilities.is_empty(), "{CAP_ENV} names no capability");

        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .try_init()
            .ok();

        let repo = PathBuf::from(repo);
        let out = PathBuf::from(out);
        let db = run_for_dump(&repo, &paths, &capabilities).expect("kernel run for the fact dump");
        let summary = write_fact_row_dump(&db, &out, &capability).expect("write the fact-row dump");
        println!(
            "fact_rows_dump: schema={DUMP_SCHEMA} capability={capability} families={} rows={}",
            summary.families, summary.rows
        );
    }

    /// Every family of `FactFamily::ALL` gets a file, and the summary families
    /// carry four columns while every other family carries two.
    #[test]
    fn a_dump_covers_every_family_and_widens_only_the_summary_families() {
        let repo = tempfile::tempdir().expect("scratch repo");
        write_summary_fixture(repo.path());
        let db = run_for_dump(repo.path(), &[], &["control_flow"]).expect("kernel run");

        let out = tempfile::tempdir().expect("scratch out");
        let summary = write_fact_row_dump(&db, out.path(), "control_flow").expect("write dump");
        assert_eq!(summary.families, FactFamily::ALL.len());

        for family in FactFamily::ALL {
            let path = out.path().join(format!("{}.txt", family.label()));
            assert!(path.is_file(), "no file for {}", family.label());
        }
        let index = std::fs::read_to_string(out.path().join("index.txt")).expect("index");
        assert!(index.contains(&format!("schema\t{DUMP_SCHEMA}")), "{index}");

        let control = std::fs::read_to_string(
            out.path()
                .join(format!("{}.txt", FactFamily::SummaryControl.label())),
        )
        .expect("summary control dump");
        assert!(
            !control.is_empty(),
            "the fixture must produce control-effect summaries or this test proves nothing"
        );
        for line in control.lines() {
            assert_eq!(
                line.split('\t').count(),
                4,
                "a summary row carries key, digest, parts and attributes: {line}"
            );
            assert!(!line.contains(DANGLING), "dangling summary join: {line}");
        }

        let functions = std::fs::read_to_string(
            out.path()
                .join(format!("{}.txt", FactFamily::Function.label())),
        )
        .expect("function dump");
        assert!(!functions.is_empty(), "the fixture declares functions");
        for line in functions.lines() {
            assert_eq!(
                line.split('\t').count(),
                2,
                "a non-summary row carries key and digest only: {line}"
            );
        }
    }

    /// Two dumps of the same tree are byte-identical: the oracle is a `diff`, so
    /// a dump that varied run to run would fail every cell for no reason.
    #[test]
    fn two_dumps_of_one_tree_are_identical() {
        let repo = tempfile::tempdir().expect("scratch repo");
        write_summary_fixture(repo.path());

        let first = fact_row_dump(&run_for_dump(repo.path(), &[], &["calls"]).expect("first run"));
        let second =
            fact_row_dump(&run_for_dump(repo.path(), &[], &["calls"]).expect("second run"));

        assert_eq!(first.len(), second.len());
        for (left, right) in first.iter().zip(second.iter()) {
            assert_eq!(left.family, right.family);
            assert_eq!(
                left.rows,
                right.rows,
                "family {} moved",
                left.family.label()
            );
        }
    }

    /// A Go repo whose callees do and do not return, so the run produces control
    /// effects, call effects and a summary event to dump.
    fn write_summary_fixture(root: &Path) {
        write(
            root,
            ".polint.toml",
            "[workspace]\ninclude = [\"**/*.go\"]\n",
        );
        write(root, "go.mod", "module example.com/dump\n\ngo 1.21\n");
        write(
            root,
            "main.go",
            r#"package main

import "os"

func Leaf(n int) int {
	if n > 0 {
		return n
	}
	return 0
}

func Fatal() {
	os.Exit(1)
}

func Caller(n int) int {
	total := 0
	for i := 0; i < n; i++ {
		total += Leaf(i)
	}
	if total < 0 {
		Fatal()
	}
	return total
}

func main() {
	_ = Caller(3)
}
"#,
        );
    }

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, contents).expect("write fixture file");
    }
}
