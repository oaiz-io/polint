//! Validated platform behaviors embedded in the engine, independent of a repository.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::analysis_api::{Digest, DigestKind};

pub(crate) const SOURCE: &str = include_str!("../../../models/platform/javascript.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Platform {
    schema_version: u32,
    pub(crate) methods: Vec<Method>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Method {
    pub(crate) names: Vec<String>,
    #[serde(default)]
    pub(crate) calls: Vec<Invocation>,
    #[serde(default)]
    pub(crate) returns: Vec<String>,
    #[serde(default)]
    pub(crate) stores: Vec<Store>,
    pub(crate) allocate: Option<String>,
    pub(crate) operation: Option<Operation>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Invocation {
    pub(crate) param: usize,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    pub(crate) this: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Store {
    pub(crate) base: String,
    pub(crate) field: String,
    pub(crate) value: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    ForwardCall,
    ForwardApply,
    Bind,
    Reduce,
    Assign,
    DefineProperty,
    DefineProperties,
    Create,
    EventRegister,
    EventEmit,
    Executor,
    ArrayFrom,
    Concat,
    Values,
    Entries,
    Collection,
}

/// Invalid embedded data disables modeling instead of risking an exact claim.
pub(crate) fn embedded() -> Option<&'static Platform> {
    static MODEL: OnceLock<Option<Platform>> = OnceLock::new();
    MODEL.get_or_init(|| parse(SOURCE).ok()).as_ref()
}

pub(crate) fn digest(source: &str) -> Digest {
    Digest::from_parts(
        DigestKind::ProviderParameters,
        "javascript_platform",
        &[source],
    )
}

fn valid_value(value: &str, calls: usize) -> bool {
    let mut parts = value.split('.');
    let root = parts.next().unwrap_or_default();
    let root_valid = matches!(root, "$this" | "$result" | "$open" | "$prim")
        || root
            .strip_prefix("$param")
            .and_then(|p| p.parse::<usize>().ok())
            .is_some_and(|p| p < 256)
        || root
            .strip_prefix("$call")
            .and_then(|p| p.parse::<usize>().ok())
            .is_some_and(|p| p < calls);
    root_valid
        && parts.all(|part| {
            matches!(
                part,
                "$elements"
                    | "$fulfilled"
                    | "$rejected"
                    | "$keys"
                    | "value"
                    | "get"
                    | "set"
                    | "prototype"
            )
        })
}

fn parse(source: &str) -> Result<Platform, String> {
    let model: Platform = toml::from_str(source).map_err(|e| e.to_string())?;
    if model.schema_version != 1 || model.methods.is_empty() || model.methods.len() > 256 {
        return Err("unsupported platform schema or method count".into());
    }
    let mut names = BTreeSet::new();
    for method in &model.methods {
        if method.names.is_empty() || method.calls.len() > 8 {
            return Err("invalid platform method".into());
        }
        for name in &method.names {
            if !(name.starts_with("global:")
                || name.starts_with("member:")
                || name.starts_with("new:"))
                || !names.insert(name)
            {
                return Err(format!("invalid or duplicate platform selector {name}"));
            }
        }
        if method
            .allocate
            .as_deref()
            .is_some_and(|kind| !matches!(kind, "array" | "promise" | "object" | "collection"))
        {
            return Err("invalid allocation kind".into());
        }
        for (index, call) in method.calls.iter().enumerate() {
            if call.param >= 256
                || call
                    .args
                    .iter()
                    .chain(call.this.iter())
                    .any(|v| !valid_value(v, index))
            {
                return Err("invalid platform invocation".into());
            }
        }
        for value in &method.returns {
            if !valid_value(value, method.calls.len()) {
                return Err(format!("invalid return {value}"));
            }
        }
        for store in &method.stores {
            if !valid_value(&store.base, method.calls.len())
                || !valid_value(&store.value, method.calls.len())
                || !matches!(
                    store.field.as_str(),
                    "$elements" | "$fulfilled" | "$rejected" | "$keys" | "$proto" | "value"
                )
            {
                return Err("invalid platform store".into());
            }
        }
    }
    Ok(model)
}

#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
mod tests {
    use super::*;
    use crate::analysis_neutral::calls::facts::CallPrecision;

    #[test]
    fn embedded_schema_is_valid() {
        assert!(parse(SOURCE).is_ok());
    }

    #[test]
    fn invalid_versions_paths_and_fields_are_rejected() {
        for source in [
            SOURCE.replacen("schema_version = 1", "schema_version = 99", 1),
            SOURCE.replacen("$param0", "$param999", 1),
            format!("{SOURCE}\nunrecognized = true"),
        ] {
            assert!(parse(&source).is_err());
        }
    }

    fn edges(source: &str, needle: &str) -> Vec<(String, CallPrecision)> {
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("main.js"), source).unwrap();
        let output = crate::eval::observed::run_kernel_for_repo_for_test(repo.path()).unwrap();
        let start = source.find(needle).unwrap() as u32;
        let site = output
            .db
            .call_sites()
            .iter()
            .find(|site| {
                site.span.start_byte == start && site.span.end_byte == start + needle.len() as u32
            })
            .unwrap_or_else(|| panic!("missing site {needle}"));
        output
            .db
            .refined_call_edges()
            .iter()
            .filter(|edge| edge.site == site.id)
            .filter_map(|edge| {
                let target = output
                    .db
                    .functions()
                    .iter()
                    .find(|function| Some(function.id) == edge.target_function)?;
                Some((
                    source[target.span.start_byte as usize..target.span.end_byte as usize]
                        .to_string(),
                    edge.precision,
                ))
            })
            .collect()
    }

    #[test]
    fn builtin_callbacks_are_conservative_model_edges() {
        for (source, call, target) in [
            (
                "function cb() {} [1].forEach(cb);",
                "[1].forEach(cb)",
                "function cb() {}",
            ),
            (
                "function replacer(k, v) { return v; } JSON.stringify({}, replacer);",
                "JSON.stringify({}, replacer)",
                "function replacer(k, v) { return v; }",
            ),
            (
                "function tick() {} queueMicrotask(tick);",
                "queueMicrotask(tick)",
                "function tick() {}",
            ),
            (
                "function executor(resolve) { resolve(1); } new Promise(executor);",
                "new Promise(executor)",
                "function executor(resolve) { resolve(1); }",
            ),
        ] {
            let found = edges(source, call);
            assert!(
                found.contains(&(target.into(), CallPrecision::Conservative)),
                "{call}: {found:?}"
            );
        }
    }

    #[test]
    fn member_callbacks_and_function_forwarding_have_model_evidence() {
        for call in [
            "[1].map(cb)",
            "[1].filter(cb)",
            "[1, 2].reduce(cb, 0)",
            "[1].some(cb)",
            "[1].every(cb)",
            "[1].find(cb)",
            "[1, 2].sort(cb)",
            "'text'.replace('t', cb)",
            "Promise.resolve(1).then(cb)",
            "Promise.reject(1).catch(cb)",
            "Promise.resolve(1).finally(cb)",
            "setTimeout(cb, 0)",
            "Array.from([1], cb)",
            "cb.call(null)",
            "cb.apply(null, [])",
            "cb.bind(null)",
        ] {
            let source = format!("function cb() {{}} {call};");
            let found = edges(&source, call);
            assert!(
                found.contains(&("function cb() {}".into(), CallPrecision::Conservative)),
                "{call}: {found:?}"
            );
        }
    }

    #[test]
    fn data_properties_keep_exact_flow_through_the_model() {
        let source = "function target() {} const o = {}; Object.defineProperty(o, 'run', {value: target}); o.run();";
        assert!(
            edges(source, "o.run()")
                .iter()
                .any(|(target, _)| target == "function target() {}")
        );
    }

    #[test]
    fn reduce_cardinality_suppresses_callbacks_that_cannot_run() {
        use crate::analysis_api::SourceFile;
        use crate::internal_core::{FileId, Language};
        use std::path::PathBuf;
        for expression in ["[].reduce(cb)", "[].reduce(cb, 1)", "[1].reduce(cb)"] {
            let source = format!("function cb() {{}} {expression};");
            let file = SourceFile::new(
                FileId::from_raw(0),
                PathBuf::from("main.js"),
                "main.js".into(),
                Language::JavaScript,
                source.into(),
                "test".into(),
            );
            let flows = crate::ts::constraints::lower_ts_source_value_flows(&file);
            assert!(flows.lowered);
            assert!(
                flows.calls.iter().all(|call| call.model.is_none()),
                "{expression}"
            );
        }
    }

    #[test]
    fn event_names_keep_listener_sets_separate() {
        let source = "function one() {} function two() {} const emitter = new EventEmitter(); emitter.on('one', one); emitter.on('two', two); emitter.emit('one');";
        let found = edges(source, "emitter.emit('one')");
        assert!(
            found.contains(&("function one() {}".into(), CallPrecision::Conservative)),
            "{found:?}"
        );
        assert!(
            found
                .iter()
                .all(|(target, _)| target != "function two() {}"),
            "{found:?}"
        );
    }

    #[test]
    fn builtin_receivers_do_not_call_same_named_user_functions() {
        let source = "function stringify() {} function cb() {} JSON.stringify({}, cb);";
        assert!(
            edges(source, "JSON.stringify({}, cb)")
                .iter()
                .all(|(target, _)| target != "function stringify() {}")
        );
    }

    #[test]
    fn shadowed_globals_keep_their_own_behavior() {
        let source =
            "const JSON = { stringify(value, cb) {} }; function cb() {} JSON.stringify({}, cb);";
        assert!(
            edges(source, "JSON.stringify({}, cb)")
                .iter()
                .all(|(target, _)| target != "function cb() {}")
        );
    }

    #[test]
    fn await_loads_fulfilled_callables() {
        let source = "function target() {} async function use() { const fn = await Promise.resolve(target); fn(); } use();";
        assert!(
            edges(source, "fn()")
                .iter()
                .any(|(target, _)| target == "function target() {}")
        );
    }
}
