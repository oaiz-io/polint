//! The targets a package's `exports` or `imports` map may give one specifier
//! under any set of conditions a loader may be given.
//!
//! Node selects `node`, `module-sync` and `default`, a bundler selects
//! `browser`, `module` or `worker`, and either may be given further
//! conditions such as `development`. The walk follows Node's resolution of a
//! map value under every such set at once: every condition but the mode's own
//! and `default` may or may not be selected, and the other mode's condition
//! never is. A condition map picks its first selected key whose value is not
//! `undefined`: a nested map that selects nothing lets the loader go on to
//! the next key. A list gives its first entry that neither selects nothing,
//! is `null` nor is an invalid target; any other error ends it. A map with a
//! numeric key is an invalid package configuration, and a string that is not
//! a `./` path of the package, outside an `imports` map's bare package
//! names, is an invalid target. Where the same condition appears twice the
//! walk lets each place select it or not on its own, so it may name a target
//! no single set of conditions reaches, never miss one.

use serde_json::{Map, Value};
use std::cmp::Ordering;

/// A target a specifier may resolve to through a map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Reach {
    /// A `./` path of the package that holds the map, with any `*` replaced.
    File(String),
    /// A package specifier an `imports` map names, which the loader resolves
    /// from the package that holds the map.
    Package(String),
}

/// The ways a map value may end, as a set: [`Self::UNDEFINED`] where it
/// selects nothing, [`Self::NULL`] where it is `null`, [`Self::INVALID`]
/// where it is an invalid target and [`Self::STOPS`] where it gives a target
/// or an error a list does not go on past.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Ends(u8);

impl Ends {
    const UNDEFINED: Self = Self(1);
    const NULL: Self = Self(2);
    const INVALID: Self = Self(4);
    const STOPS: Self = Self(8);
    /// The ends a list goes on past to its next entry.
    const PASSES: Self = Self(1 | 2 | 4);

    fn has(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    fn passes(self) -> Self {
        Self(self.0 & Self::PASSES.0)
    }
}

/// The targets `map` may give `key` in the mode whose condition is `own`,
/// where `other` is the condition of the other mode. `map` is the `exports`
/// value of a package when `exports` is set, with `key` `.` or a `./`
/// subpath, and its `imports` value otherwise, with `key` a `#` specifier.
/// Empty where `map` gives `key` nothing. Object order decides which key a
/// loader picks, which the resolver's JSON maps keep.
pub(super) fn reachable_targets(
    map: &Value,
    key: &str,
    exports: bool,
    own: &str,
    other: &str,
) -> Vec<Reach> {
    let found = match map {
        Value::Object(entries) if exports => match subpath_keys(entries) {
            Some(true) => entry(entries, key),
            Some(false) if key == "." => Some((map, None)),
            // A map that mixes subpaths and conditions is invalid.
            _ => None,
        },
        // An `exports` string or list is the `.` entry.
        Value::String(_) | Value::Array(_) if exports && key == "." => Some((map, None)),
        Value::Object(entries) if !exports => entry(entries, key),
        _ => None,
    };
    let mut walk = Walk {
        matched: None,
        internal: !exports,
        own,
        other,
        targets: Vec::new(),
    };
    if let Some((value, matched)) = found {
        walk.matched = matched;
        walk.value(value);
    }
    walk.targets
}

/// Whether every key of an `exports` map is a subpath, `Some(false)` where
/// none is, and `None` where they mix.
fn subpath_keys(entries: &Map<String, Value>) -> Option<bool> {
    let mut kinds = entries.keys().map(|key| key.starts_with('.'));
    let first = kinds.next().unwrap_or(false);
    kinds.all(|kind| kind == first).then_some(first)
}

/// The entry of `entries` that matches `key`, with the text a `*` of its
/// pattern stands for.
fn entry<'a>(entries: &'a Map<String, Value>, key: &str) -> Option<(&'a Value, Option<String>)> {
    if !key.contains('*')
        && !key.ends_with('/')
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
                && key.len() >= pattern.len()
                && key.ends_with(trailer);
            matches.then_some((pattern, base.len(), trailer.len(), value))
        })
        // Node prefers the pattern with the longer text before its `*`, then
        // the longer pattern, then the first.
        .reduce(
            |best, next| match (next.1, next.0.len()).cmp(&(best.1, best.0.len())) {
                Ordering::Greater => next,
                Ordering::Less | Ordering::Equal => best,
            },
        )
        .map(|(_, base, trailer, value)| (value, Some(key[base..key.len() - trailer].to_string())))
}

/// One walk of a map value: the text a pattern's `*` stands for, whether the
/// map is an `imports` map, the conditions of the mode, and the targets found.
struct Walk<'a> {
    matched: Option<String>,
    internal: bool,
    own: &'a str,
    other: &'a str,
    targets: Vec<Reach>,
}

impl Walk<'_> {
    /// Collects the targets `value` may give and says how it may end.
    fn value(&mut self, value: &Value) -> Ends {
        match value {
            Value::String(target) => self.target(target),
            Value::Array(values) => self.list(values),
            Value::Object(conditions) => self.conditions(conditions),
            Value::Null => Ends::NULL,
            _ => Ends::INVALID,
        }
    }

    fn target(&mut self, target: &str) -> Ends {
        let substitute = |target: &str| match self.matched.as_deref() {
            Some(matched) => target.replace('*', matched),
            None => target.to_string(),
        };
        let Some(path) = target.strip_prefix("./") else {
            if self.internal
                && !target.starts_with("../")
                && !target.starts_with('/')
                && !is_url(target)
            {
                let target = substitute(target);
                // Node looks a `#` name up as a package directory, which no
                // package manager writes.
                if !target.starts_with('#') {
                    self.targets.push(Reach::Package(target));
                }
                return Ends::STOPS;
            }
            return Ends::INVALID;
        };
        if has_invalid_segment(path) {
            return Ends::INVALID;
        }
        if self.matched.as_deref().is_some_and(has_invalid_segment) {
            // An invalid specifier, which no entry after it rescues.
            return Ends::STOPS;
        }
        let target = substitute(target);
        // A declaration file describes types for a compiler; no runtime
        // loads it.
        if !is_declaration(&target) {
            self.targets.push(Reach::File(target));
        }
        Ends::STOPS
    }

    fn list(&mut self, values: &[Value]) -> Ends {
        if values.is_empty() {
            return Ends::NULL;
        }
        let mut ends = Ends::default();
        // How the entries walked so far may have passed: still `undefined`,
        // or `null` or an invalid target as the last that was not.
        let mut passed = Ends::UNDEFINED;
        for value in values {
            let end = self.value(value);
            ends = ends.with(end.without(Ends::PASSES));
            let passes = end.passes();
            if passes == Ends::default() {
                return ends;
            }
            let mut next = passes.without(Ends::UNDEFINED);
            if passes.has(Ends::UNDEFINED) {
                next = next.with(passed);
            }
            passed = next;
        }
        ends.with(passed)
    }

    fn conditions(&mut self, conditions: &Map<String, Value>) -> Ends {
        if conditions.keys().any(|key| is_array_index(key)) {
            return Ends::STOPS;
        }
        let mut ends = Ends::default();
        for (condition, value) in conditions {
            if condition == self.other {
                continue;
            }
            let end = self.value(value);
            ends = ends.with(end.without(Ends::UNDEFINED));
            let always = condition == self.own || condition == "default";
            if always && !end.has(Ends::UNDEFINED) {
                return ends;
            }
        }
        ends.with(Ends::UNDEFINED)
    }
}

/// Whether `key` reads as an array index, which a condition map may not hold.
fn is_array_index(key: &str) -> bool {
    key.parse::<u32>()
        .is_ok_and(|index| index != u32::MAX && index.to_string() == key)
}

/// Whether `target` parses as a URL with a scheme, which an `imports` map
/// may not name.
fn is_url(target: &str) -> bool {
    target.split_once(':').is_some_and(|(scheme, _)| {
        let mut chars = scheme.chars();
        chars
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
            && chars.all(|char| char.is_ascii_alphanumeric() || matches!(char, '+' | '-' | '.'))
    })
}

/// Whether a path holds a `.`, `..` or `node_modules` segment, written plain
/// or percent-encoded, which Node rejects in a target or what a `*` stands
/// for. An empty segment it only warns about.
fn has_invalid_segment(path: &str) -> bool {
    path.split(['/', '\\']).any(|segment| {
        let segment = percent_decoded(segment).to_ascii_lowercase();
        matches!(segment.as_str(), "." | ".." | "node_modules")
    })
}

fn percent_decoded(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = bytes
            .get(index + 1..index + 3)
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (bytes[index], hex) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                index += 3;
            }
            (byte, _) => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// A declaration file describes types for a compiler; no runtime loads it.
fn is_declaration(target: &str) -> bool {
    [".d.ts", ".d.mts", ".d.cts"]
        .iter()
        .any(|extension| target.ends_with(extension))
}

#[cfg(test)]
mod tests {
    use super::{Reach, reachable_targets};
    use serde_json::json;

    fn targets(map: serde_json::Value, key: &str, exports: bool) -> Vec<Reach> {
        reachable_targets(&map, key, exports, "import", "require")
    }

    fn files(names: &[&str]) -> Vec<Reach> {
        names
            .iter()
            .map(|name| Reach::File((*name).into()))
            .collect()
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
            files(&["./browser.js", "./import.js"])
        );
    }

    #[test]
    fn a_nested_map_that_may_select_nothing_leaves_the_next_keys_reachable() {
        let map = json!({
            "import": { "feature": "./feature.js" },
            "worker": "./worker.js",
            "default": "./default.js"
        });
        assert_eq!(
            targets(map, ".", true),
            files(&["./feature.js", "./worker.js", "./default.js"])
        );
        let map = json!({
            "import": { "feature": "./feature.js", "default": "./import.js" },
            "worker": "./worker.js"
        });
        assert_eq!(
            targets(map, ".", true),
            files(&["./feature.js", "./import.js"])
        );
        let map = json!({
            "import": [{ "feature": "./feature.js" }, null],
            "worker": "./worker.js"
        });
        assert_eq!(targets(map, ".", true), files(&["./feature.js"]));
    }

    #[test]
    fn a_list_goes_on_past_null_an_invalid_target_and_an_entry_that_selects_nothing() {
        let map = json!({
            "import": [null, "lib/a.js", "./lib/../a.js", { "feature": "./f.js" }, "./a.js", "./b.js"],
            "default": "./d.js"
        });
        assert_eq!(targets(map, ".", true), files(&["./f.js", "./a.js"]));
        // A list whose entries all select nothing selects nothing itself, so
        // the map goes on to its next key.
        let map = json!({ "import": [{ "feature": "./f.js" }], "default": "./d.js" });
        assert_eq!(targets(map, ".", true), files(&["./f.js", "./d.js"]));
        // A list that ends with `null` or an invalid target ends the map.
        let map = json!({ "import": [{ "feature": "./f.js" }, "a.js"], "default": "./d.js" });
        assert_eq!(targets(map, ".", true), files(&["./f.js"]));
        let map = json!({ "import": [], "default": "./d.js" });
        assert_eq!(targets(map, ".", true), Vec::<Reach>::new());
    }

    #[test]
    fn a_map_with_a_numeric_key_or_mixed_keys_gives_nothing() {
        let map =
            json!({ "import": { "1": "./one.js", "default": "./a.js" }, "default": "./d.js" });
        assert_eq!(targets(map, ".", true), Vec::<Reach>::new());
        let map = json!({ ".": "./a.js", "browser": "./b.js" });
        assert_eq!(targets(map, ".", true), Vec::<Reach>::new());
    }

    #[test]
    fn an_imports_map_names_packages_but_not_urls_or_imports() {
        let map = json!({
            "#dep": ["node:path", "#a", "./a.js"],
            "#pkg": ["dep/sub", "./a.js"],
            "#feat/*": "dep/*.js"
        });
        assert_eq!(targets(map.clone(), "#dep", false), Vec::<Reach>::new());
        assert_eq!(
            targets(map.clone(), "#pkg", false),
            vec![Reach::Package("dep/sub".into())]
        );
        assert_eq!(
            targets(map, "#feat/x", false),
            vec![Reach::Package("dep/x.js".into())]
        );
        assert_eq!(
            targets(json!({ "./a": "dep" }), "./a", true),
            Vec::<Reach>::new()
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
            files(&["./n/x.js", "./d/x.js"])
        );
        assert_eq!(
            targets(map.clone(), "./sub/exact", true),
            files(&["./exact.js"])
        );
        assert_eq!(targets(map, ".", true), Vec::<Reach>::new());
    }

    #[test]
    fn an_exports_value_without_subpaths_is_the_main_entry() {
        assert_eq!(
            targets(json!("./main.js"), ".", true),
            files(&["./main.js"])
        );
        assert_eq!(
            targets(json!({ "node": "./n.js" }), "./sub", true),
            Vec::<Reach>::new()
        );
        assert_eq!(
            targets(
                json!({ "#dep": { "worker": "./w.js", "default": "./d.js" } }),
                "#dep",
                false
            ),
            files(&["./w.js", "./d.js"])
        );
    }
}
