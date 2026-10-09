//! Repository route models: the `[[go_route]]` tables of `.polint/models/*.toml`.
//!
//! A route model names one framework call and the part it plays in building a
//! route table: a router constructor, a route registration, a group, a
//! middleware `use`, a mount, a message subscription, a pass-through wrapper, or
//! a serve call. The semantic sidecar applies a repository's models before its
//! built-in ones (gin, chi, net/http with httptest, Watermill), so a repository
//! can describe an in-house router or subscriber wrapper, or a decorator that
//! returns the handler it wraps. See `docs/facts/routes.md`.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The directory, under the repository root, that holds model files.
const MODELS_DIRECTORY: &str = ".polint/models";

const ROLES: &[&str] = &[
    "router",
    "route",
    "group",
    "use",
    "mount",
    "subscriber",
    "passthrough",
    "serve",
];

/// One `[[go_route]]` table. Field names are the sidecar's model fields.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GoRouteModel {
    framework: String,
    role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    function: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    receivers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    methods: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    http_method: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    method_from_name: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    method_argument: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    method_in_pattern: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path_argument: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    handler_argument: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    handlers_from: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    middleware_from: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    callback_argument: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    router_argument: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name_argument: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    topic_argument: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    argument: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_router: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    initial_middleware: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    returns_receiver: bool,
}

impl GoRouteModel {
    fn problem(&self) -> Option<String> {
        if self.framework.trim().is_empty() {
            return Some("`framework` must not be empty".to_string());
        }
        if !ROLES.contains(&self.role.as_str()) {
            return Some(format!(
                "unknown role `{}` (expected one of {})",
                self.role,
                ROLES.join(", ")
            ));
        }
        let names_function = self
            .function
            .as_deref()
            .is_some_and(|function| !function.trim().is_empty());
        let names_methods = !self.receivers.is_empty() && !self.methods.is_empty();
        match (names_function, names_methods) {
            (true, true) => {
                Some("name either a `function` or `receivers` and `methods`, not both".to_string())
            }
            (false, false) => {
                Some("needs a `function`, or both `receivers` and `methods`".to_string())
            }
            _ => None,
        }
    }
}

/// The `[[go_route]]` part of a model file; other tables belong to other
/// loaders and are left to them.
#[derive(Debug, Deserialize)]
struct ModelFile {
    #[serde(default)]
    go_route: Vec<GoRouteModel>,
}

#[derive(Debug, Serialize)]
struct RouteModelDocument<'a> {
    models: &'a [GoRouteModel],
}

/// A repository's route models, ready for the sidecar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RepositoryRouteModels {
    /// The valid models as the sidecar's JSON document, in file-name order;
    /// `None` when the repository defines none.
    pub(crate) json: Option<String>,
    /// One message per model file or table that could not be used, naming
    /// the file. Such a table is left out; the other models still apply.
    pub(crate) problems: Vec<String>,
}

/// Reads every `[[go_route]]` table under `.polint/models`.
pub(crate) fn load_repository_route_models(root: &Path) -> RepositoryRouteModels {
    let directory = root.join(MODELS_DIRECTORY);
    let mut paths = std::fs::read_dir(&directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "toml"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut models = Vec::new();
    let mut problems = Vec::new();
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
        let file = match toml::from_str::<ModelFile>(&text) {
            Ok(file) => file,
            Err(error) => {
                problems.push(format!("{name}: invalid route model: {error}"));
                continue;
            }
        };
        for (index, model) in file.go_route.into_iter().enumerate() {
            match model.problem() {
                Some(problem) => problems.push(format!("{name}: go_route {index}: {problem}")),
                None => models.push(model),
            }
        }
    }
    let json = (!models.is_empty()).then(|| {
        serde_json::to_string(&RouteModelDocument { models: &models })
            .expect("route models serialize")
    });
    RepositoryRouteModels { json, problems }
}

#[cfg(test)]
mod tests {
    use super::load_repository_route_models;

    fn write(root: &std::path::Path, name: &str, text: &str) {
        let directory = root.join(".polint/models");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(name), text).unwrap();
    }

    #[test]
    fn route_tables_become_the_sidecar_document_in_file_order() {
        let root = tempfile::tempdir().unwrap();
        write(
            root.path(),
            "b.toml",
            r#"
[[go_route]]
framework = "pubsub"
role = "subscriber"
receivers = ["example.com/app/pubsub.Bus"]
methods = ["Subscribe"]
topic_argument = 0
handler_argument = 1
"#,
        );
        write(
            root.path(),
            "a.toml",
            r#"
[[facts]]
source_pattern = "a"
target_pattern = "b"
confidence = "high"
language = "go"
scope = "repo"
evidence = ["x"]

[[go_route]]
framework = "local"
role = "passthrough"
function = "example.com/app/decorator.Apply"
argument = 0
"#,
        );

        let models = load_repository_route_models(root.path());

        assert!(models.problems.is_empty(), "{}", models.problems.join("; "));
        assert_eq!(
            models.json.as_deref(),
            Some(
                r#"{"models":[{"framework":"local","role":"passthrough","function":"example.com/app/decorator.Apply","argument":0},{"framework":"pubsub","role":"subscriber","receivers":["example.com/app/pubsub.Bus"],"methods":["Subscribe"],"handler_argument":1,"topic_argument":0}]}"#
            )
        );
    }

    #[test]
    fn invalid_tables_are_reported_and_left_out() {
        let root = tempfile::tempdir().unwrap();
        write(
            root.path(),
            "bad.toml",
            r#"
[[go_route]]
framework = "x"
role = "teleport"
function = "a.B"

[[go_route]]
framework = "x"
role = "route"
methods = ["GET"]

[[go_route]]
framework = "x"
role = "use"
function = "a.Use"
middleware_from = 0
"#,
        );
        write(
            root.path(),
            "typo.toml",
            "[[go_route]]\nframework = \"x\"\nrole = \"route\"\nfunction = \"a.F\"\npath_arg = 0\n",
        );

        let models = load_repository_route_models(root.path());

        assert_eq!(models.problems.len(), 3, "{}", models.problems.join("; "));
        assert!(models.problems[0].contains("bad.toml: go_route 0: unknown role `teleport`"));
        assert!(models.problems[1].contains("bad.toml: go_route 1: needs a `function`"));
        assert!(models.problems[2].starts_with(".polint/models/typo.toml: invalid route model"));
        assert_eq!(
            models.json.as_deref(),
            Some(
                r#"{"models":[{"framework":"x","role":"use","function":"a.Use","middleware_from":0}]}"#
            )
        );
    }

    #[test]
    fn a_repository_without_models_has_none() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            load_repository_route_models(root.path()),
            super::RepositoryRouteModels::default()
        );
    }
}
