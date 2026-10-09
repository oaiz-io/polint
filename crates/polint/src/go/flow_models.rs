//! Go data-flow models: the built-in ones (`flow_models.toml`) and a
//! repository's `[[go_flow_source]]`, `[[go_flow_sink]]`,
//! `[[go_flow_sanitizer]]`, `[[go_flow_opaque]]` and `[[go_flow_propagator]]`
//! tables of `.polint/models/*.toml`, which apply alongside them. See
//! `docs/facts/data-flow.md`.

use std::path::Path;

use crate::analysis_neutral::taint::models::{
    Models, PropagatorModel, SinkModel, SourceModel, Target,
};

/// The directory, under the repository root, that holds model files.
const MODELS_DIRECTORY: &str = ".polint/models";

const BUILT_IN: &str = include_str!("flow_models.toml");

/// The keys a table of each kind may have.
const TARGET_KEYS: &[&str] = &["function", "functions", "receivers", "methods"];
const SOURCE_KEYS: &[&str] = &["kind", "output", "argument", "parameter_type"];
const SINK_KEYS: &[&str] = &["kind", "arguments", "arguments_from"];
const PROPAGATOR_KEYS: &[&str] = &["from", "to"];

/// The built-in models and a repository's own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GoFlowModels {
    pub(crate) models: Models,
    /// One message per model file or table that could not be used, naming the
    /// file. Such a table is left out; the other models still apply.
    pub(crate) problems: Vec<String>,
    /// The digest of the built-in models and of every repository table read.
    pub(crate) digest: String,
}

/// Reads the built-in models and every flow table under `.polint/models`.
pub(crate) fn load_flow_models(root: &Path) -> GoFlowModels {
    let mut models = Models::parse(BUILT_IN).expect("the built-in Go flow models parse");
    let mut problems = Vec::new();
    let mut digest_parts = vec![BUILT_IN.to_string()];
    let directory = root.join(MODELS_DIRECTORY);
    let mut paths = std::fs::read_dir(&directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "toml"))
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        let name = format!(
            "{MODELS_DIRECTORY}/{}",
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        );
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                problems.push(format!("{name}: could not be read: {error}"));
                continue;
            }
        };
        let table = match toml::from_str::<toml::Table>(&text) {
            Ok(table) => table,
            Err(error) => {
                problems.push(format!("{name}: invalid flow model: {error}"));
                continue;
            }
        };
        let mut read = Models::default();
        for (key, value) in &table {
            let Some(kind) = key.strip_prefix("go_flow_") else {
                continue;
            };
            let Some(entries) = value.as_array() else {
                problems.push(format!("{name}: `{key}` must be an array of tables"));
                continue;
            };
            for (index, entry) in entries.iter().enumerate() {
                if let Err(problem) = read_table(&mut read, kind, entry) {
                    problems.push(format!("{name}: {key} {index}: {problem}"));
                }
            }
        }
        if read != Models::default() {
            digest_parts.push(name);
            digest_parts.push(text);
            models.extend(read);
        }
    }
    let parts = digest_parts.iter().map(String::as_str).collect::<Vec<_>>();
    GoFlowModels {
        models,
        problems,
        digest: crate::go::hash::stable_hash(&parts),
    }
}

/// Reads one `[[go_flow_<kind>]]` table into `models`.
fn read_table(models: &mut Models, kind: &str, entry: &toml::Value) -> Result<(), String> {
    let Some(table) = entry.as_table() else {
        return Err("must be a table".to_string());
    };
    let allowed: Vec<&str> = match kind {
        "source" => TARGET_KEYS.iter().chain(SOURCE_KEYS).copied().collect(),
        "sink" => TARGET_KEYS.iter().chain(SINK_KEYS).copied().collect(),
        "sanitizer" | "opaque" => TARGET_KEYS.to_vec(),
        "propagator" => TARGET_KEYS.iter().chain(PROPAGATOR_KEYS).copied().collect(),
        _ => return Err(format!("unknown table `go_flow_{kind}`")),
    };
    if let Some(unknown) = table.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("unknown field `{unknown}`"));
    }
    let parse_error = |error: toml::de::Error| error.message().to_string();
    match kind {
        "source" => {
            let model: SourceModel = entry.clone().try_into().map_err(parse_error)?;
            if model.kind.trim().is_empty() {
                return Err("`kind` must not be empty".to_string());
            }
            match model.output.as_deref() {
                None | Some("result") => {}
                Some("argument") if model.argument.is_some() => {}
                Some("argument") => {
                    return Err("`output = \"argument\"` needs an `argument`".to_string());
                }
                Some(other) => {
                    return Err(format!(
                        "unknown output `{other}` (expected `result` or `argument`)"
                    ));
                }
            }
            if model.parameter_type.is_none() {
                check_target(&model.target)?;
            }
            models.sources.push(model);
        }
        "sink" => {
            let model: SinkModel = entry.clone().try_into().map_err(parse_error)?;
            if model.kind.trim().is_empty() {
                return Err("`kind` must not be empty".to_string());
            }
            check_target(&model.target)?;
            models.sinks.push(model);
        }
        "sanitizer" | "opaque" => {
            let target: Target = entry.clone().try_into().map_err(parse_error)?;
            check_target(&target)?;
            if kind == "sanitizer" {
                models.sanitizers.push(target);
            } else {
                models.opaque.push(target);
            }
        }
        _ => {
            let model: PropagatorModel = entry.clone().try_into().map_err(parse_error)?;
            check_target(&model.target)?;
            models.propagators.push(model);
        }
    }
    Ok(())
}

/// A model names functions by `function`/`functions`, or by both `receivers`
/// and `methods`, not by both forms.
fn check_target(target: &Target) -> Result<(), String> {
    let names_functions = target
        .function
        .as_deref()
        .is_some_and(|function| !function.trim().is_empty())
        || !target.functions.is_empty();
    let names_methods = !target.receivers.is_empty() && !target.methods.is_empty();
    match (names_functions, names_methods) {
        (true, true) => Err(
            "name either a `function` (or `functions`) or `receivers` and `methods`, not both"
                .to_string(),
        ),
        (false, false) => {
            Err("needs a `function` or `functions`, or both `receivers` and `methods`".to_string())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::load_flow_models;

    fn write(root: &std::path::Path, name: &str, text: &str) {
        let directory = root.join(".polint/models");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(name), text).unwrap();
    }

    #[test]
    fn the_built_in_models_load_without_a_repository_model() {
        let root = tempfile::tempdir().unwrap();
        let models = load_flow_models(root.path());
        assert!(models.problems.is_empty(), "{}", models.problems.join("; "));
        assert!(!models.models.sources_of_kind("http_request").is_empty());
        assert!(!models.models.sinks_of_kind("sql").is_empty());
        assert!(!models.models.sanitizers.is_empty());
    }

    #[test]
    fn repository_tables_add_to_the_built_in_ones_and_change_the_digest() {
        let root = tempfile::tempdir().unwrap();
        let before = load_flow_models(root.path());
        write(
            root.path(),
            "flows.toml",
            r#"
[[go_route]]
framework = "local"
role = "passthrough"
function = "example.com/app/decorator.Apply"
argument = 0

[[go_flow_sink]]
kind = "sql"
function = "example.com/app/db.Raw"
arguments = [0]

[[go_flow_source]]
kind = "tenant"
receivers = ["example.com/app/auth.Actor"]
methods = ["SchoolID"]

[[go_flow_sanitizer]]
function = "example.com/app/text.Escape"
"#,
        );
        let after = load_flow_models(root.path());
        assert!(after.problems.is_empty(), "{}", after.problems.join("; "));
        assert_eq!(
            after.models.sinks_of_kind("sql").len(),
            before.models.sinks_of_kind("sql").len() + 1
        );
        assert_eq!(after.models.sources_of_kind("tenant").len(), 1);
        assert_eq!(
            after.models.sanitizers.len(),
            before.models.sanitizers.len() + 1
        );
        assert_ne!(after.digest, before.digest);
    }

    #[test]
    fn invalid_tables_are_reported_and_left_out() {
        let root = tempfile::tempdir().unwrap();
        write(
            root.path(),
            "bad.toml",
            r#"
[[go_flow_sink]]
kind = "sql"
function = "a.Raw"
argument = 0

[[go_flow_source]]
kind = "x"
function = "a.F"
receivers = ["a.T"]
methods = ["M"]

[[go_flow_source]]
kind = "x"
function = "a.G"
output = "argument"

[[go_flow_teleport]]
function = "a.H"

[[go_flow_opaque]]
function = "a.Compare"
"#,
        );
        let models = load_flow_models(root.path());
        assert_eq!(models.problems.len(), 4, "{}", models.problems.join("; "));
        assert!(
            models
                .problems
                .iter()
                .any(|problem| problem.contains("go_flow_sink 0: unknown field `argument`"))
        );
        assert!(
            models
                .problems
                .iter()
                .any(|problem| problem.contains("go_flow_source 0: name either"))
        );
        assert!(
            models
                .problems
                .iter()
                .any(|problem| problem.contains("go_flow_source 1: `output = \"argument\"`"))
        );
        assert!(
            models
                .problems
                .iter()
                .any(|problem| problem.contains("unknown table `go_flow_teleport`"))
        );
        assert!(
            models
                .models
                .opaque
                .iter()
                .any(|target| target.function.as_deref() == Some("a.Compare"))
        );
    }
}
