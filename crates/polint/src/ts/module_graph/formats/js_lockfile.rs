use super::package_lock::parse_package_lock;
use crate::analysis_neutral::module_graph::topology::{TopologyPrecision, TopologyStatus};
use serde_json::Value as JsonValue;
use serde_norway::Value as YamlValue;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum JsPackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl JsPackageManager {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum JsLockfileKind {
    NpmPackageLock,
    NpmShrinkwrap,
    Pnpm,
    Yarn,
    Bun,
}

impl JsLockfileKind {
    pub(crate) fn file_name(self) -> &'static str {
        match self {
            Self::NpmPackageLock => "package-lock.json",
            Self::NpmShrinkwrap => "npm-shrinkwrap.json",
            Self::Pnpm => "pnpm-lock.yaml",
            Self::Yarn => "yarn.lock",
            Self::Bun => "bun.lock",
        }
    }

    pub(crate) fn manager(self) -> JsPackageManager {
        match self {
            Self::NpmPackageLock | Self::NpmShrinkwrap => JsPackageManager::Npm,
            Self::Pnpm => JsPackageManager::Pnpm,
            Self::Yarn => JsPackageManager::Yarn,
            Self::Bun => JsPackageManager::Bun,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsLockfileManifest {
    pub(crate) relative_path: String,
    pub(crate) source_label: String,
    pub(crate) schema_label: String,
    pub(crate) packages: Vec<JsLockfilePackage>,
    pub(crate) unsupported: Vec<JsLockfileUnsupported>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsLockfilePackage {
    pub(crate) path: String,
    pub(crate) importer_path: Option<String>,
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) source_path: String,
    pub(crate) source_label: String,
    pub(crate) precision: TopologyPrecision,
    pub(crate) status: TopologyStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsLockfileUnsupported {
    pub(crate) reason: String,
    pub(crate) source_path: String,
    pub(crate) source_label: String,
    pub(crate) precision: TopologyPrecision,
    pub(crate) status: TopologyStatus,
}

pub(crate) fn parse_js_lockfile(
    kind: JsLockfileKind,
    relative_path: &str,
    contents: &str,
) -> JsLockfileManifest {
    match kind {
        JsLockfileKind::NpmPackageLock | JsLockfileKind::NpmShrinkwrap => {
            parse_npm_lock(relative_path, contents)
        }
        JsLockfileKind::Pnpm => parse_pnpm_lock(relative_path, contents),
        JsLockfileKind::Yarn => parse_yarn_lock(relative_path, contents),
        JsLockfileKind::Bun => parse_bun_lock(relative_path, contents),
    }
}

pub(crate) fn unsupported_js_lockfile(
    kind: JsLockfileKind,
    relative_path: &str,
    reason: &str,
) -> JsLockfileManifest {
    let mut manifest = empty_manifest(
        relative_path,
        kind.file_name(),
        format!("{}-unknown", kind.manager().label()),
    );
    manifest
        .unsupported
        .push(unsupported(relative_path, kind.file_name(), reason));
    manifest
}

fn parse_npm_lock(relative_path: &str, contents: &str) -> JsLockfileManifest {
    let lock = parse_package_lock(relative_path, contents);
    let mut manifest = empty_manifest(
        relative_path,
        lock.source_label,
        lock.schema_label.to_string(),
    );
    manifest.unsupported = lock
        .unsupported
        .into_iter()
        .map(|unsupported| JsLockfileUnsupported {
            reason: unsupported.reason,
            source_path: unsupported.source_path,
            source_label: unsupported.source_label.to_string(),
            precision: unsupported.precision,
            status: unsupported.status,
        })
        .collect();
    manifest.packages = lock
        .packages
        .into_iter()
        .filter(|package| !package.path.is_empty())
        .filter_map(|package| {
            Some(JsLockfilePackage {
                path: package.path,
                importer_path: None,
                name: package.name?,
                version: package.version?,
                source_path: package.source_path,
                source_label: package.source_label.to_string(),
                precision: package.precision,
                status: package.status,
            })
        })
        .collect();
    manifest
}

fn parse_pnpm_lock(relative_path: &str, contents: &str) -> JsLockfileManifest {
    let mut manifest = empty_manifest(relative_path, "pnpm-lock.yaml", "pnpm-lock-unknown");
    let Ok(value) = serde_norway::from_str::<YamlValue>(contents) else {
        manifest.unsupported.push(unsupported(
            relative_path,
            "pnpm-lock.yaml",
            "malformed yaml",
        ));
        return manifest;
    };
    let Some(lockfile_version) = yaml_get(&value, "lockfileVersion").and_then(yaml_scalar_string)
    else {
        manifest.unsupported.push(unsupported(
            relative_path,
            "pnpm-lock.yaml",
            "missing lockfileVersion",
        ));
        return manifest;
    };
    manifest.schema_label = format!("pnpm-lock-v{}", stable_label(&lockfile_version));

    if let Some(importers) = yaml_get(&value, "importers").and_then(YamlValue::as_mapping) {
        for (importer_key, importer_value) in importers {
            let Some(importer_path) = importer_key.as_str() else {
                continue;
            };
            for section in [
                "dependencies",
                "devDependencies",
                "optionalDependencies",
                "peerDependencies",
            ] {
                append_pnpm_importer_dependencies(
                    &mut manifest,
                    importer_path,
                    section,
                    yaml_get(importer_value, section),
                );
            }
        }
    }

    let mut seen = manifest
        .packages
        .iter()
        .map(|package| (package.name.clone(), package.version.clone()))
        .collect::<BTreeSet<_>>();
    for section in ["packages", "snapshots"] {
        if let Some(packages) = yaml_get(&value, section).and_then(YamlValue::as_mapping) {
            for (package_key, _) in packages {
                let Some(key) = package_key.as_str() else {
                    continue;
                };
                let Some((name, version)) = package_name_version_from_key(key) else {
                    continue;
                };
                if seen.insert((name.clone(), version.clone())) {
                    manifest.packages.push(lockfile_package(
                        relative_path,
                        "pnpm-lock.yaml",
                        key.to_string(),
                        None,
                        name,
                        version,
                    ));
                }
            }
        }
    }

    manifest.packages.sort_by(|left, right| {
        (
            left.importer_path.as_deref().unwrap_or(""),
            left.path.as_str(),
            left.name.as_str(),
            left.version.as_str(),
        )
            .cmp(&(
                right.importer_path.as_deref().unwrap_or(""),
                right.path.as_str(),
                right.name.as_str(),
                right.version.as_str(),
            ))
    });
    manifest
}

fn append_pnpm_importer_dependencies(
    manifest: &mut JsLockfileManifest,
    importer_path: &str,
    section: &str,
    value: Option<&YamlValue>,
) {
    let Some(dependencies) = value.and_then(YamlValue::as_mapping) else {
        return;
    };
    for (name_value, dependency_value) in dependencies {
        let Some(name) = name_value.as_str() else {
            continue;
        };
        let Some(version) = pnpm_dependency_version(dependency_value) else {
            continue;
        };
        manifest.packages.push(lockfile_package(
            &manifest.relative_path,
            "pnpm-lock.yaml",
            format!("importer:{importer_path}:{section}:{name}"),
            Some(importer_path.to_string()),
            name.to_string(),
            version,
        ));
    }
}

fn pnpm_dependency_version(value: &YamlValue) -> Option<String> {
    let raw = value
        .as_str()
        .map(str::to_string)
        .or_else(|| yaml_get(value, "version").and_then(yaml_scalar_string))?;
    package_version_from_pnpm_version(&raw)
}

fn parse_yarn_lock(relative_path: &str, contents: &str) -> JsLockfileManifest {
    if contents.contains("__metadata:") {
        parse_yarn_berry_lock(relative_path, contents)
    } else {
        parse_yarn_classic_lock(relative_path, contents)
    }
}

fn parse_yarn_berry_lock(relative_path: &str, contents: &str) -> JsLockfileManifest {
    let mut manifest = empty_manifest(relative_path, "yarn.lock", "yarn-berry");
    let Ok(value) = serde_norway::from_str::<YamlValue>(contents) else {
        manifest
            .unsupported
            .push(unsupported(relative_path, "yarn.lock", "malformed yaml"));
        return manifest;
    };
    if let Some(version) = yaml_get(&value, "__metadata")
        .and_then(|metadata| yaml_get(metadata, "version"))
        .and_then(yaml_scalar_string)
    {
        manifest.schema_label = format!("yarn-berry-v{}", stable_label(&version));
    }
    let Some(entries) = value.as_mapping() else {
        manifest.unsupported.push(unsupported(
            relative_path,
            "yarn.lock",
            "yarn lock root is not a map",
        ));
        return manifest;
    };
    for (descriptor_value, entry_value) in entries {
        let Some(descriptor) = descriptor_value.as_str() else {
            continue;
        };
        if descriptor == "__metadata" {
            continue;
        }
        let Some(name) = package_name_from_descriptor(descriptor) else {
            continue;
        };
        let version = yaml_get(entry_value, "version")
            .and_then(yaml_scalar_string)
            .or_else(|| {
                yaml_get(entry_value, "resolution")
                    .and_then(YamlValue::as_str)
                    .and_then(|resolution| version_from_resolution(&name, resolution))
            });
        let Some(version) = version else {
            continue;
        };
        manifest.packages.push(lockfile_package(
            relative_path,
            "yarn.lock",
            descriptor.to_string(),
            None,
            name,
            version,
        ));
    }
    if manifest.packages.is_empty() {
        manifest.unsupported.push(unsupported(
            relative_path,
            "yarn.lock",
            "no parseable yarn berry entries",
        ));
    }
    manifest
}

fn parse_yarn_classic_lock(relative_path: &str, contents: &str) -> JsLockfileManifest {
    let mut manifest = empty_manifest(relative_path, "yarn.lock", "yarn-classic-v1");
    let mut current: Option<YarnClassicEntry> = None;

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !line.starts_with([' ', '\t']) && trimmed.ends_with(':') {
            push_yarn_classic_entry(&mut manifest, current.take());
            let descriptor = trimmed.trim_end_matches(':').to_string();
            current = package_name_from_yarn_header(&descriptor).map(|name| YarnClassicEntry {
                descriptor,
                name,
                version: None,
            });
            continue;
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        if let Some(version) = trimmed.strip_prefix("version ").map(|value| {
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string()
        }) {
            entry.version = Some(version);
        }
    }
    push_yarn_classic_entry(&mut manifest, current);

    if manifest.packages.is_empty() {
        manifest.unsupported.push(unsupported(
            relative_path,
            "yarn.lock",
            "no parseable yarn classic entries",
        ));
    }
    manifest
}

fn push_yarn_classic_entry(manifest: &mut JsLockfileManifest, entry: Option<YarnClassicEntry>) {
    let Some(entry) = entry else {
        return;
    };
    let Some(version) = entry.version else {
        return;
    };
    manifest.packages.push(lockfile_package(
        &manifest.relative_path,
        "yarn.lock",
        entry.descriptor,
        None,
        entry.name,
        version,
    ));
}

#[derive(Debug)]
struct YarnClassicEntry {
    descriptor: String,
    name: String,
    version: Option<String>,
}

fn parse_bun_lock(relative_path: &str, contents: &str) -> JsLockfileManifest {
    let mut manifest = empty_manifest(relative_path, "bun.lock", "bun-lock-unknown");
    let Ok(value) = parse_jsonc(contents) else {
        manifest
            .unsupported
            .push(unsupported(relative_path, "bun.lock", "malformed jsonc"));
        return manifest;
    };
    if let Some(version) = json_get(&value, "lockfileVersion").and_then(json_scalar_string) {
        manifest.schema_label = format!("bun-lock-v{}", stable_label(&version));
    }
    let Some(packages) = json_get(&value, "packages").and_then(JsonValue::as_object) else {
        manifest
            .unsupported
            .push(unsupported(relative_path, "bun.lock", "missing packages"));
        return manifest;
    };
    for (package_key, package_value) in packages {
        let Some(name) = package_name_from_bun_key(package_key) else {
            continue;
        };
        let version = bun_package_version(&name, package_value);
        let Some(version) = version else {
            continue;
        };
        manifest.packages.push(lockfile_package(
            relative_path,
            "bun.lock",
            package_key.clone(),
            None,
            name,
            version,
        ));
    }
    manifest
}

fn bun_package_version(name: &str, value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::Array(entries) => entries
            .first()
            .and_then(JsonValue::as_str)
            .and_then(|entry| version_from_resolution(name, entry)),
        JsonValue::Object(object) => {
            object
                .get("version")
                .and_then(json_scalar_string)
                .or_else(|| {
                    object
                        .get("resolution")
                        .and_then(JsonValue::as_str)
                        .and_then(|resolution| version_from_resolution(name, resolution))
                })
        }
        JsonValue::String(text) => version_from_resolution(name, text),
        _ => None,
    }
}

fn empty_manifest(
    relative_path: &str,
    source_label: impl Into<String>,
    schema_label: impl Into<String>,
) -> JsLockfileManifest {
    JsLockfileManifest {
        relative_path: relative_path.to_string(),
        source_label: source_label.into(),
        schema_label: schema_label.into(),
        packages: Vec::new(),
        unsupported: Vec::new(),
    }
}

fn lockfile_package(
    relative_path: &str,
    source_label: &str,
    path: String,
    importer_path: Option<String>,
    name: String,
    version: String,
) -> JsLockfilePackage {
    JsLockfilePackage {
        path,
        importer_path,
        name,
        version,
        source_path: relative_path.to_string(),
        source_label: source_label.to_string(),
        precision: TopologyPrecision::ExactLockfile,
        status: TopologyStatus::Resolved,
    }
}

fn unsupported(relative_path: &str, source_label: &str, reason: &str) -> JsLockfileUnsupported {
    JsLockfileUnsupported {
        reason: reason.to_string(),
        source_path: relative_path.to_string(),
        source_label: source_label.to_string(),
        precision: TopologyPrecision::Unsupported,
        status: TopologyStatus::Unsupported,
    }
}

fn yaml_get<'a>(value: &'a YamlValue, key: &str) -> Option<&'a YamlValue> {
    value.as_mapping()?.get(YamlValue::String(key.to_string()))
}

fn yaml_scalar_string(value: &YamlValue) -> Option<String> {
    match value {
        YamlValue::String(text) => Some(text.clone()),
        YamlValue::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn json_get<'a>(value: &'a JsonValue, key: &str) -> Option<&'a JsonValue> {
    value.as_object()?.get(key)
}

fn json_scalar_string(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn parse_jsonc(contents: &str) -> Result<JsonValue, crate::bounded_json::BoundedJsonError> {
    crate::bounded_json::from_jsonc_str(contents, crate::repo_fs::TOPOLOGY_LOCKFILE_MAX_BYTES)
}

fn package_name_version_from_key(key: &str) -> Option<(String, String)> {
    let normalized = key.trim_start_matches('/');
    let split_at = package_version_split(normalized)?;
    let name = normalized[..split_at].to_string();
    let raw_version = &normalized[split_at + 1..];
    let version = package_version_from_pnpm_version(raw_version)?;
    Some((name, version))
}

fn package_version_split(text: &str) -> Option<usize> {
    if text.starts_with('@') {
        let slash = text.find('/')?;
        text[slash + 1..].find('@').map(|offset| slash + 1 + offset)
    } else {
        text.find('@')
    }
}

fn package_version_from_pnpm_version(raw: &str) -> Option<String> {
    if raw.starts_with("link:")
        || raw.starts_with("workspace:")
        || raw.starts_with("file:")
        || raw.starts_with("path:")
    {
        return None;
    }
    let version = raw
        .split(['(', '_', '/'])
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_matches('"')
        .trim_matches('\'');
    (!version.is_empty()).then(|| version.to_string())
}

fn package_name_from_yarn_header(header: &str) -> Option<String> {
    split_yarn_descriptors(header)
        .into_iter()
        .find_map(|descriptor| package_name_from_descriptor(&descriptor))
}

fn split_yarn_descriptors(header: &str) -> Vec<String> {
    let mut descriptors = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for ch in header.chars() {
        match (ch, quote) {
            ('"' | '\'', None) => {
                quote = Some(ch);
                current.push(ch);
            }
            (value, Some(active)) if value == active => {
                quote = None;
                current.push(ch);
            }
            (',', None) => {
                let descriptor = current.trim();
                if !descriptor.is_empty() {
                    descriptors.push(descriptor.to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let descriptor = current.trim();
    if !descriptor.is_empty() {
        descriptors.push(descriptor.to_string());
    }
    descriptors
}

fn package_name_from_descriptor(descriptor: &str) -> Option<String> {
    let text = descriptor.trim().trim_matches('"').trim_matches('\'');
    let split_at = package_version_split(text)?;
    let name = &text[..split_at];
    (!name.is_empty()).then(|| name.to_string())
}

fn package_name_from_bun_key(key: &str) -> Option<String> {
    if key.is_empty() || key == "." {
        return None;
    }
    if key.starts_with('@') && key.contains('/') && !key.contains("@npm:") {
        return Some(key.to_string());
    }
    if let Some(name) = package_name_from_descriptor(key) {
        return Some(name);
    }
    Some(key.to_string())
}

fn version_from_resolution(package_name: &str, resolution: &str) -> Option<String> {
    let text = resolution.trim().trim_matches('"').trim_matches('\'');
    let text = text.strip_prefix("npm:").unwrap_or(text);
    let candidate = text
        .strip_prefix(package_name)
        .and_then(|rest| rest.strip_prefix('@'))?;
    let version = candidate
        .strip_prefix("npm:")
        .unwrap_or(candidate)
        .split(['(', '#'])
        .next()
        .unwrap_or(candidate)
        .trim();
    (!version.is_empty()
        && !version.contains(':')
        && !version.contains('/')
        && version.chars().any(|ch| ch.is_ascii_digit()))
    .then(|| version.to_string())
}

fn stable_label(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pnpm_lock_reads_importer_and_package_entries() {
        let manifest = parse_pnpm_lock(
            "pnpm-lock.yaml",
            r#"
lockfileVersion: '9.0'
importers:
  .:
    dependencies:
      react:
        specifier: ^18.0.0
        version: 18.2.0
packages:
  react@18.2.0:
    resolution:
      integrity: sha512-test
"#,
        );

        assert_eq!(manifest.schema_label, "pnpm-lock-v9.0");
        assert!(manifest.packages.iter().any(|package| {
            package.importer_path.as_deref() == Some(".")
                && package.name == "react"
                && package.version == "18.2.0"
        }));
    }

    #[test]
    fn parse_yarn_classic_lock_reads_versions() {
        let manifest = parse_yarn_lock(
            "yarn.lock",
            r#"
react@^18.0.0:
  version "18.2.0"
  resolved "https://registry.yarnpkg.com/react/-/react-18.2.0.tgz"
"#,
        );

        assert_eq!(manifest.schema_label, "yarn-classic-v1");
        assert_eq!(manifest.packages[0].name, "react");
        assert_eq!(manifest.packages[0].version, "18.2.0");
    }

    #[test]
    fn parse_yarn_berry_lock_reads_versions() {
        let manifest = parse_yarn_lock(
            "yarn.lock",
            r#"
__metadata:
  version: 8
  cacheKey: 10
"react@npm:^18.0.0":
  version: 18.2.0
  resolution: "react@npm:18.2.0"
"#,
        );

        assert_eq!(manifest.schema_label, "yarn-berry-v8");
        assert_eq!(manifest.packages[0].name, "react");
        assert_eq!(manifest.packages[0].version, "18.2.0");
    }

    #[test]
    fn parse_bun_lock_reads_text_lock_package_entries() {
        let manifest = parse_bun_lock(
            "bun.lock",
            r#"{
  "lockfileVersion": 1,
  "packages": {
    "react": ["react@18.2.0", "", {}, "sha512-test"]
  }
}"#,
        );

        assert_eq!(manifest.schema_label, "bun-lock-v1");
        assert_eq!(manifest.packages[0].name, "react");
        assert_eq!(manifest.packages[0].version, "18.2.0");
    }

    #[test]
    fn parse_bun_lock_does_not_treat_artifact_strings_as_versions() {
        let manifest = parse_bun_lock(
            "bun.lock",
            r#"{
  "lockfileVersion": 1,
  "packages": {
    "uWebSockets.js": [
      "git+https://github.com/uNetworking/uWebSockets.js.git#6609a88",
      "",
      {},
      "uNetworking-uWebSockets.js-6609a88"
    ]
  }
}"#,
        );

        assert!(manifest.packages.is_empty());
    }
}
