use crate::analysis_neutral::module_graph::topology::{
    RequirementKind, TopologyPrecision, TopologyStatus,
};
use crate::bounded_json;
use crate::repo_fs::TOPOLOGY_MANIFEST_MAX_BYTES;
use serde_json::Value;

pub(crate) const PACKAGE_JSON_SOURCE_LABEL: &str = "package.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PackageJsonManifest {
    pub(crate) relative_path: String,
    pub(crate) source_label: &'static str,
    pub(crate) name: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) package_manager: Option<String>,
    pub(crate) workspaces: Vec<String>,
    pub(crate) exports: Vec<String>,
    pub(crate) imports: Vec<String>,
    pub(crate) dependencies: Vec<PackageJsonDependency>,
    pub(crate) unsupported: Vec<PackageJsonUnsupported>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PackageJsonDependency {
    pub(crate) target_name: String,
    pub(crate) version_requirement: Option<String>,
    pub(crate) kind: RequirementKind,
    pub(crate) section: &'static str,
    pub(crate) source_path: String,
    pub(crate) source_label: &'static str,
    pub(crate) precision: TopologyPrecision,
    pub(crate) status: TopologyStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PackageJsonUnsupported {
    pub(crate) reason: String,
    pub(crate) source_path: String,
    pub(crate) source_label: &'static str,
    pub(crate) precision: TopologyPrecision,
    pub(crate) status: TopologyStatus,
}

pub(crate) fn parse_package_json(relative_path: &str, contents: &str) -> PackageJsonManifest {
    let mut manifest = empty_manifest(relative_path);
    let value = match bounded_json::from_str::<Value>(contents, TOPOLOGY_MANIFEST_MAX_BYTES) {
        Ok(value) => value,
        Err(error) => {
            manifest
                .unsupported
                .push(unsupported(relative_path, &error.reason()));
            return manifest;
        }
    };
    let Some(object) = value.as_object() else {
        manifest.unsupported.push(unsupported(
            relative_path,
            "package.json root is not an object",
        ));
        return manifest;
    };

    manifest.name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string);
    manifest.version = object
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string);
    manifest.package_manager = object
        .get("packageManager")
        .and_then(Value::as_str)
        .map(str::to_string);
    manifest.workspaces = parse_workspaces(object.get("workspaces"));
    manifest.exports = evidence_entries(object.get("exports"));
    manifest.imports = evidence_entries(object.get("imports"));

    for (section, kind) in [
        ("dependencies", RequirementKind::Direct),
        ("devDependencies", RequirementKind::Dev),
        ("peerDependencies", RequirementKind::Peer),
        ("optionalDependencies", RequirementKind::Optional),
        ("bundleDependencies", RequirementKind::Bundled),
        ("bundledDependencies", RequirementKind::Bundled),
    ] {
        append_dependency_section(
            &mut manifest,
            relative_path,
            section,
            kind,
            object.get(section),
        );
    }

    manifest
}

pub(crate) fn unsupported_package_json(relative_path: &str, reason: &str) -> PackageJsonManifest {
    let mut manifest = empty_manifest(relative_path);
    manifest
        .unsupported
        .push(unsupported(relative_path, reason));
    manifest
}

fn empty_manifest(relative_path: &str) -> PackageJsonManifest {
    PackageJsonManifest {
        relative_path: relative_path.to_string(),
        source_label: PACKAGE_JSON_SOURCE_LABEL,
        name: None,
        version: None,
        package_manager: None,
        workspaces: Vec::new(),
        exports: Vec::new(),
        imports: Vec::new(),
        dependencies: Vec::new(),
        unsupported: Vec::new(),
    }
}

fn parse_workspaces(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(entries)) => string_array(entries),
        Some(Value::Object(object)) => object
            .get("packages")
            .and_then(Value::as_array)
            .map(|entries| string_array(entries))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn evidence_entries(value: Option<&Value>) -> Vec<String> {
    let mut entries = match value {
        Some(Value::String(text)) => vec![text.clone()],
        Some(Value::Array(values)) => values.iter().filter_map(evidence_value).collect(),
        Some(Value::Object(object)) => object
            .iter()
            .filter_map(|(key, value)| {
                evidence_value(value).map(|rendered| format!("{key}:{rendered}"))
            })
            .collect(),
        _ => Vec::new(),
    };
    entries.sort();
    entries
}

fn evidence_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => serde_json::to_string(text).ok(),
        Value::Array(_) | Value::Object(_) | Value::Bool(_) | Value::Number(_) | Value::Null => {
            serde_json::to_string(value).ok()
        }
    }
}

fn append_dependency_section(
    manifest: &mut PackageJsonManifest,
    relative_path: &str,
    section: &'static str,
    kind: RequirementKind,
    value: Option<&Value>,
) {
    match value {
        Some(Value::Object(object)) => {
            manifest
                .dependencies
                .extend(object.iter().filter_map(|(target_name, requirement)| {
                    requirement.as_str().map(|version| PackageJsonDependency {
                        target_name: target_name.clone(),
                        version_requirement: Some(version.to_string()),
                        kind,
                        section,
                        source_path: relative_path.to_string(),
                        source_label: PACKAGE_JSON_SOURCE_LABEL,
                        precision: TopologyPrecision::ExactStatic,
                        status: TopologyStatus::Present,
                    })
                }));
        }
        Some(Value::Array(entries))
            if matches!(section, "bundleDependencies" | "bundledDependencies") =>
        {
            manifest
                .dependencies
                .extend(entries.iter().filter_map(Value::as_str).map(|target_name| {
                    PackageJsonDependency {
                        target_name: target_name.to_string(),
                        version_requirement: None,
                        kind,
                        section,
                        source_path: relative_path.to_string(),
                        source_label: PACKAGE_JSON_SOURCE_LABEL,
                        precision: TopologyPrecision::ExactStatic,
                        status: TopologyStatus::Present,
                    }
                }));
        }
        Some(_) => manifest
            .unsupported
            .push(unsupported(relative_path, section)),
        None => {}
    }
}

fn string_array(entries: &[Value]) -> Vec<String> {
    entries
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn unsupported(relative_path: &str, reason: &str) -> PackageJsonUnsupported {
    PackageJsonUnsupported {
        reason: reason.to_string(),
        source_path: relative_path.to_string(),
        source_label: PACKAGE_JSON_SOURCE_LABEL,
        precision: TopologyPrecision::Unknown,
        status: TopologyStatus::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_package_json_reads_package_workspace_exports_imports_and_dependency_sections() {
        let manifest = parse_package_json(
            "package.json",
            r##"{
  "name": "@acme/root",
  "version": "1.2.3",
  "packageManager": "pnpm@9.0.0",
  "workspaces": { "packages": ["packages/*", "tools/*"] },
  "exports": { ".": "./src/index.ts", "./feature": "./src/feature.ts" },
  "imports": { "#internal/*": "./src/internal/*" },
  "dependencies": { "react": "^18.0.0" },
  "devDependencies": { "vitest": "^2.0.0" },
  "peerDependencies": { "typescript": "^5.0.0" },
  "optionalDependencies": { "fsevents": "^2.0.0" },
  "bundleDependencies": ["left-pad"],
  "bundledDependencies": ["right-pad"]
}"##,
        );

        assert_eq!(manifest.name.as_deref(), Some("@acme/root"));
        assert_eq!(manifest.version.as_deref(), Some("1.2.3"));
        assert_eq!(manifest.package_manager.as_deref(), Some("pnpm@9.0.0"));
        assert_eq!(manifest.workspaces, vec!["packages/*", "tools/*"]);
        assert_eq!(
            manifest.exports,
            vec!["./feature:\"./src/feature.ts\"", ".:\"./src/index.ts\""]
        );
        assert_eq!(manifest.imports, vec!["#internal/*:\"./src/internal/*\""]);
        assert_eq!(
            manifest
                .dependencies
                .iter()
                .map(|dependency| (
                    dependency.section,
                    dependency.target_name.as_str(),
                    dependency.version_requirement.as_deref(),
                    dependency.kind,
                    dependency.precision,
                    dependency.status,
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "dependencies",
                    "react",
                    Some("^18.0.0"),
                    RequirementKind::Direct,
                    TopologyPrecision::ExactStatic,
                    TopologyStatus::Present,
                ),
                (
                    "devDependencies",
                    "vitest",
                    Some("^2.0.0"),
                    RequirementKind::Dev,
                    TopologyPrecision::ExactStatic,
                    TopologyStatus::Present,
                ),
                (
                    "peerDependencies",
                    "typescript",
                    Some("^5.0.0"),
                    RequirementKind::Peer,
                    TopologyPrecision::ExactStatic,
                    TopologyStatus::Present,
                ),
                (
                    "optionalDependencies",
                    "fsevents",
                    Some("^2.0.0"),
                    RequirementKind::Optional,
                    TopologyPrecision::ExactStatic,
                    TopologyStatus::Present,
                ),
                (
                    "bundleDependencies",
                    "left-pad",
                    None,
                    RequirementKind::Bundled,
                    TopologyPrecision::ExactStatic,
                    TopologyStatus::Present,
                ),
                (
                    "bundledDependencies",
                    "right-pad",
                    None,
                    RequirementKind::Bundled,
                    TopologyPrecision::ExactStatic,
                    TopologyStatus::Present,
                ),
            ]
        );
    }

    #[test]
    fn parse_package_json_records_malformed_json_as_unsupported() {
        let manifest = parse_package_json("package.json", "{ this is not json");

        assert_eq!(manifest.unsupported.len(), 1);
        assert_eq!(manifest.unsupported[0].status, TopologyStatus::Unsupported);
        assert_eq!(
            manifest.unsupported[0].precision,
            TopologyPrecision::Unknown
        );
    }
}
