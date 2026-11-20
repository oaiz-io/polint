//! The targets a package's `exports` or `imports` map may give one specifier
//! under any set of conditions a loader may be given.
//!
//! Node selects `node` and `default`, a bundler selects `browser`, `module`
//! or `worker`, and either may be given further conditions such as
//! `development`. A condition map picks its first key the loader selects, so
//! every key is reachable up to the first one that is always selected: the
//! condition of the mode the specifier loads in, or `default`.

use serde_json::{Map, Value};
use std::cmp::Ordering;

/// The targets `map` may give `key` in the mode whose condition is `own`,
/// where `other` is the condition of the other mode. `map` is the `exports`
/// value of a package when `exports` is set, with `key` `.` or a `./`
/// subpath, and its `imports` value otherwise, with `key` a `#` specifier.
/// `None` where no entry of `map` matches `key`. Object order decides which
/// key a loader picks, which the resolver's JSON maps keep.
pub(super) fn reachable_targets(
    map: &Value,
    key: &str,
    exports: bool,
    own: &str,
    other: &str,
) -> Option<Vec<String>> {
    let (value, matched) = match map {
        Value::Object(entries) if !exports || entries.keys().any(|key| key.starts_with('.')) => {
            entry(entries, key)?
        }
        // An `exports` value that is not a map of subpaths is the `.` entry.
        value if exports && key == "." => (value, None),
        _ => return None,
    };
    let mut targets = Vec::new();
    collect(value, matched.as_deref(), own, other, &mut targets);
    Some(targets)
}

/// The entry of `entries` that matches `key`, with the text a `*` of its
/// pattern stands for.
fn entry<'a>(entries: &'a Map<String, Value>, key: &str) -> Option<(&'a Value, Option<String>)> {
    if !key.contains('*')
        && let Some(value) = entries.get(key)
    {
        return Some((value, None));
    }
    entries
        .iter()
        .filter_map(|(pattern, value)| {
            let (base, trailer) = pattern.split_once('*')?;
            let matches = !trailer.contains('*')
                && key.starts_with(base)
                && key != base
                && (trailer.is_empty() || (key.ends_with(trailer) && key.len() >= pattern.len()));
            matches.then_some((pattern, base.len(), trailer.len(), value))
        })
        // Node prefers the pattern with the longer text before its `*`, then
        // the longer pattern.
        .reduce(
            |best, next| match (next.1, next.0.len()).cmp(&(best.1, best.0.len())) {
                Ordering::Greater => next,
                Ordering::Less | Ordering::Equal => best,
            },
        )
        .map(|(_, base, trailer, value)| (value, Some(key[base..key.len() - trailer].to_string())))
}

fn collect(
    value: &Value,
    matched: Option<&str>,
    own: &str,
    other: &str,
    targets: &mut Vec<String>,
) {
    match value {
        Value::String(target) if !is_declaration(target) => targets.push(match matched {
            Some(matched) => target.replace('*', matched),
            None => target.clone(),
        }),
        // A loader takes the first target of a list it can load; any may be.
        Value::Array(values) => {
            for value in values {
                collect(value, matched, own, other, targets);
            }
        }
        Value::Object(conditions) => {
            for (condition, value) in conditions {
                if condition == other {
                    continue;
                }
                collect(value, matched, own, other, targets);
                if condition == own || condition == "default" {
                    break;
                }
            }
        }
        _ => {}
    }
}

/// A declaration file describes types for a compiler; no runtime loads it.
fn is_declaration(target: &str) -> bool {
    [".d.ts", ".d.mts", ".d.cts"]
        .iter()
        .any(|extension| target.ends_with(extension))
}

#[cfg(test)]
mod tests {
    use super::reachable_targets;
    use serde_json::json;

    fn targets(map: serde_json::Value, key: &str, exports: bool) -> Option<Vec<String>> {
        reachable_targets(&map, key, exports, "import", "require")
    }

    #[test]
    fn every_condition_before_an_always_selected_one_is_reachable() {
        let map = json!({
            "types": "./index.d.ts",
            "browser": "./browser.js",
            "require": "./require.js",
            "import": "./import.js",
            "default": "./default.js"
        });
        assert_eq!(
            targets(map, ".", true),
            Some(vec!["./browser.js".into(), "./import.js".into()])
        );
    }

    #[test]
    fn a_pattern_entry_substitutes_its_match_and_the_longest_base_wins() {
        let map = json!({
            "./*": "./other/*.js",
            "./sub/*": { "node": "./n/*.js", "default": "./d/*.js" },
            "./sub/exact": "./exact.js"
        });
        assert_eq!(
            targets(map.clone(), "./sub/x", true),
            Some(vec!["./n/x.js".into(), "./d/x.js".into()])
        );
        assert_eq!(
            targets(map.clone(), "./sub/exact", true),
            Some(vec!["./exact.js".into()])
        );
        assert_eq!(targets(map, ".", true), None);
    }

    #[test]
    fn an_exports_value_without_subpaths_is_the_main_entry() {
        assert_eq!(
            targets(json!("./main.js"), ".", true),
            Some(vec!["./main.js".into()])
        );
        assert_eq!(targets(json!({ "node": "./n.js" }), "./sub", true), None);
        assert_eq!(
            targets(
                json!({ "#dep": { "worker": "./w.js", "default": "./d.js" } }),
                "#dep",
                false
            ),
            Some(vec!["./w.js".into(), "./d.js".into()])
        );
    }
}
