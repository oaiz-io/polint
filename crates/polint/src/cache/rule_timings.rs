//! How long each rule took on the previous pass of a rule plan.
//!
//! The next pass of the same plan starts its rules longest-first from these
//! times. They order dispatch and nothing else: no cache key or digest folds
//! them, no report prints them, and no rule reads them. A missing, unreadable
//! or stale entry leaves the next pass to start its rules as registered.

use super::{Cache, CacheKey, CacheReadStatus};
use crate::core::RuleTimings;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// Version of what an entry records. Entries are keyed by it, so a change to
/// the entry's shape must change this string.
const RULE_TIMINGS_SCHEMA: &str = "rule-timings-v1";

/// Stands in for a source path in the entry's cache key; it names no file.
const RULE_TIMINGS_KEY: &str = "<rule-timings>";

#[derive(Serialize, Deserialize)]
struct RuleTimingsEntry {
    schema: String,
    /// Microseconds each rule took, keyed by rule id.
    micros: BTreeMap<String, u64>,
}

/// One entry per rule plan and scope: the config digest covers the workspace
/// include list that path arguments narrow, and the rule digest covers the
/// selected rules and their options.
fn rule_timings_key(config_digest: &str, rule_digest: &str) -> CacheKey {
    CacheKey::for_file(
        RULE_TIMINGS_KEY,
        "",
        config_digest,
        rule_digest,
        "",
        RULE_TIMINGS_SCHEMA,
        "",
    )
}

/// The times the previous pass of this rule plan recorded, if any.
pub(crate) fn read_rule_timings(
    cache: &Cache,
    config_digest: &str,
    rule_digest: &str,
) -> Option<RuleTimings> {
    let read = cache.read_json_bytes_with_status(&rule_timings_key(config_digest, rule_digest));
    if read.status != CacheReadStatus::Hit {
        return None;
    }
    let entry = serde_json::from_slice::<RuleTimingsEntry>(&read.value?).ok()?;
    (entry.schema == RULE_TIMINGS_SCHEMA).then(|| {
        entry
            .micros
            .into_iter()
            .map(|(rule_id, micros)| (rule_id, Duration::from_micros(micros)))
            .collect()
    })
}

/// Records this pass's times for the next pass of the same rule plan.
///
/// Best effort: times that cannot be written only leave the next pass to
/// start its rules as registered.
pub(crate) fn write_rule_timings(
    cache: &Cache,
    config_digest: &str,
    rule_digest: &str,
    timings: &RuleTimings,
) {
    if timings.is_empty() {
        return;
    }
    let entry = RuleTimingsEntry {
        schema: RULE_TIMINGS_SCHEMA.to_string(),
        micros: timings
            .iter()
            .map(|(rule_id, elapsed)| {
                let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
                (rule_id.clone(), micros)
            })
            .collect(),
    };
    if let Ok(bytes) = serde_json::to_vec(&entry) {
        let _ = cache
            .write_json_bytes_with_status(&rule_timings_key(config_digest, rule_digest), &bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn timings(entries: &[(&str, u64)]) -> RuleTimings {
        entries
            .iter()
            .map(|(rule_id, millis)| (rule_id.to_string(), Duration::from_millis(*millis)))
            .collect()
    }

    fn entry_path(cache: &Cache, config_digest: &str, rule_digest: &str) -> PathBuf {
        let key = rule_timings_key(config_digest, rule_digest);
        cache.root().join(format!("{}.json", key.stable_id()))
    }

    #[test]
    fn recorded_timings_read_back_for_the_same_rule_plan() {
        let dir = tempfile::tempdir().expect("cache dir");
        let cache = Cache::new(dir.path(), true);
        let recorded = timings(&[("local/slow", 2_400), ("local/fast", 12)]);

        write_rule_timings(&cache, "config", "rules", &recorded);

        assert_eq!(read_rule_timings(&cache, "config", "rules"), Some(recorded));
    }

    #[test]
    fn a_cache_without_an_entry_has_no_timings() {
        let dir = tempfile::tempdir().expect("cache dir");
        let cache = Cache::new(dir.path(), true);

        assert_eq!(read_rule_timings(&cache, "config", "rules"), None);
    }

    #[test]
    fn another_rule_plan_or_scope_does_not_read_the_timings() {
        let dir = tempfile::tempdir().expect("cache dir");
        let cache = Cache::new(dir.path(), true);
        write_rule_timings(&cache, "config", "rules", &timings(&[("local/slow", 5)]));

        let other_rules = read_rule_timings(&cache, "config", "other-rules");
        let other_scope = read_rule_timings(&cache, "other-config", "rules");

        assert_eq!((other_rules, other_scope), (None, None));
    }

    #[test]
    fn a_corrupt_entry_reads_as_no_timings_and_is_evicted() {
        let dir = tempfile::tempdir().expect("cache dir");
        let cache = Cache::new(dir.path(), true);
        let path = entry_path(&cache, "config", "rules");
        fs::write(&path, b"{\"schema\": \"rule-timings-v1\", \"micros\": {")
            .expect("corrupt entry");

        let read = read_rule_timings(&cache, "config", "rules");

        assert_eq!((read, path.exists()), (None, false));
    }

    #[test]
    fn an_entry_of_another_shape_reads_as_no_timings() {
        let dir = tempfile::tempdir().expect("cache dir");
        let cache = Cache::new(dir.path(), true);
        fs::write(
            entry_path(&cache, "config", "rules"),
            br#"{"schema": "rule-timings-v1", "micros": {"local/slow": "long"}}"#,
        )
        .expect("malformed entry");

        assert_eq!(read_rule_timings(&cache, "config", "rules"), None);
    }

    #[test]
    fn an_entry_with_another_schema_reads_as_no_timings() {
        let dir = tempfile::tempdir().expect("cache dir");
        let cache = Cache::new(dir.path(), true);
        fs::write(
            entry_path(&cache, "config", "rules"),
            br#"{"schema": "rule-timings-v0", "micros": {"local/slow": 2400000}}"#,
        )
        .expect("stale entry");

        assert_eq!(read_rule_timings(&cache, "config", "rules"), None);
    }

    #[test]
    fn a_disabled_cache_neither_writes_nor_reads_timings() {
        let dir = tempfile::tempdir().expect("cache dir");
        let cache = Cache::new(dir.path(), false);

        write_rule_timings(&cache, "config", "rules", &timings(&[("local/slow", 5)]));

        let written = fs::read_dir(dir.path()).expect("cache dir").count();
        assert_eq!(
            (written, read_rule_timings(&cache, "config", "rules")),
            (0, None)
        );
    }

    #[test]
    fn a_pass_that_ran_no_rule_writes_no_entry() {
        let dir = tempfile::tempdir().expect("cache dir");
        let cache = Cache::new(dir.path(), true);

        write_rule_timings(&cache, "config", "rules", &RuleTimings::new());

        assert!(!entry_path(&cache, "config", "rules").exists());
    }
}
