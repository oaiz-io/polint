use std::path::{Component, Path};

use serde::Deserialize;
use thiserror::Error;

use crate::analysis_api::{FactFamily, stable_key_from_parts};
use crate::analysis_neutral::adaptation::facts::{LoadedModelFact, ModelConfidence, ModelLanguage};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModelLoadError {
    #[error("adaptation model path must be repo-relative: {0}")]
    InvalidPath(String),
    #[error("failed to parse adaptation model TOML: {0}")]
    Parse(String),
    #[error("adaptation model contains no facts")]
    Empty,
    #[error("adaptation model fact {index} is missing required field `{field}`")]
    MissingField { index: usize, field: &'static str },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelFileToml {
    #[serde(default)]
    facts: Vec<ModelFactToml>,
    /// Route models live in the same files; the Go route-model loader reads
    /// and validates them.
    #[serde(default)]
    go_route: Vec<toml::Value>,
    /// So do data-flow models; the Go flow-model loader reads and validates
    /// them.
    #[serde(default)]
    go_flow_source: Vec<toml::Value>,
    #[serde(default)]
    go_flow_sink: Vec<toml::Value>,
    #[serde(default)]
    go_flow_sanitizer: Vec<toml::Value>,
    #[serde(default)]
    go_flow_opaque: Vec<toml::Value>,
    #[serde(default)]
    go_flow_propagator: Vec<toml::Value>,
}

impl ModelFileToml {
    /// Whether the file holds tables other loaders read.
    fn has_other_models(&self) -> bool {
        !(self.go_route.is_empty()
            && self.go_flow_source.is_empty()
            && self.go_flow_sink.is_empty()
            && self.go_flow_sanitizer.is_empty()
            && self.go_flow_opaque.is_empty()
            && self.go_flow_propagator.is_empty())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelFactToml {
    source_pattern: Option<String>,
    target_pattern: Option<String>,
    confidence: Option<ModelConfidence>,
    language: Option<ModelLanguage>,
    scope: Option<String>,
    evidence: Option<Vec<String>>,
}

pub fn load_model_file(
    interner: &crate::internal_core::StableKeyInterner,
    model_path: impl AsRef<Path>,
    contents: &str,
) -> Result<Vec<LoadedModelFact>, ModelLoadError> {
    let model_path = normalize_model_path(model_path.as_ref())?;
    let parsed: ModelFileToml =
        toml::from_str(contents).map_err(|error| ModelLoadError::Parse(error.to_string()))?;

    if parsed.facts.is_empty() {
        if !parsed.has_other_models() {
            return Err(ModelLoadError::Empty);
        }
        return Ok(Vec::new());
    }

    let mut facts = Vec::with_capacity(parsed.facts.len());
    for (index, fact) in parsed.facts.into_iter().enumerate() {
        let source_pattern = require_field(index, "source_pattern", fact.source_pattern)?
            .trim()
            .to_string();
        let target_pattern = require_field(index, "target_pattern", fact.target_pattern)?
            .trim()
            .to_string();
        let confidence = fact.confidence.ok_or(ModelLoadError::MissingField {
            index,
            field: "confidence",
        })?;
        let language = fact.language.ok_or(ModelLoadError::MissingField {
            index,
            field: "language",
        })?;
        let scope = require_field(index, "scope", fact.scope)?
            .trim()
            .to_string();
        let mut evidence = fact.evidence.ok_or(ModelLoadError::MissingField {
            index,
            field: "evidence",
        })?;
        evidence
            .iter_mut()
            .for_each(|item| *item = item.trim().to_string());
        evidence.sort();
        evidence.dedup();

        let stable_key = model_stable_key(
            interner,
            &model_path,
            &source_pattern,
            &target_pattern,
            confidence,
            language,
            (&scope, &evidence),
        );
        facts.push(LoadedModelFact {
            model_path: model_path.clone(),
            source_pattern,
            target_pattern,
            confidence,
            language,
            scope,
            evidence,
            stable_key,
        });
    }

    facts.sort_by(|left, right| left.sort_key(interner).cmp(&right.sort_key(interner)));
    Ok(facts)
}

fn require_field(
    index: usize,
    field: &'static str,
    value: Option<String>,
) -> Result<String, ModelLoadError> {
    value.ok_or(ModelLoadError::MissingField { index, field })
}

fn normalize_model_path(path: &Path) -> Result<String, ModelLoadError> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => {
                return Err(ModelLoadError::InvalidPath(path.display().to_string()));
            }
        }
    }
    if parts.is_empty() {
        return Err(ModelLoadError::InvalidPath(path.display().to_string()));
    }
    Ok(parts.join("/"))
}

fn model_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    model_path: &str,
    source_pattern: &str,
    target_pattern: &str,
    confidence: ModelConfidence,
    language: ModelLanguage,
    pair: (&str, &[String]),
) -> crate::internal_core::StableKeyId {
    let (scope, evidence) = pair;
    let language = language.as_str().to_string();
    let confidence = confidence.as_str().to_string();
    let mut parts = vec![
        ("model_path", model_path.to_string()),
        ("source_pattern", source_pattern.to_string()),
        ("target_pattern", target_pattern.to_string()),
        ("confidence", confidence),
        ("language", language),
        ("scope", scope.to_string()),
    ];
    parts.extend(evidence.iter().map(|item| ("evidence", item.clone())));
    stable_key_from_parts(interner, FactFamily::AdaptationModel, &parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::LocalAnalysisDb;

    const VALID: &str = r#"
[[facts]]
source_pattern = "call:src/app.ts:10:register"
target_pattern = "function:src/app.ts:1:onRegister"
confidence = "heuristic"
language = "typescript"
scope = "src/app.ts"
evidence = ["src/app.ts:10", " src/app.ts:10 "]
"#;

    #[test]
    fn loader_parses_and_normalizes_model_facts() {
        let interner = LocalAnalysisDb::new().stable_key_interner();
        let facts = load_model_file(&interner, ".polint/models/framework.toml", VALID).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].model_path, ".polint/models/framework.toml");
        assert_eq!(facts[0].confidence, ModelConfidence::Heuristic);
        assert_eq!(facts[0].language, ModelLanguage::TypeScript);
        assert_eq!(facts[0].evidence, vec!["src/app.ts:10"]);
        assert!(
            interner
                .resolve(facts[0].stable_key)
                .contains("15:AdaptationModel")
        );
    }

    #[test]
    fn loader_stable_keys_keep_evidence_boundaries() {
        let interner = LocalAnalysisDb::new().stable_key_interner();
        let first = interner.resolve(model_stable_key(
            &interner,
            ".polint/models/framework.toml",
            "call:src/app.ts:10:register",
            "function:src/app.ts:1:onRegister",
            ModelConfidence::Heuristic,
            ModelLanguage::TypeScript,
            ("src/app.ts", &["a,b".to_string(), "c".to_string()]),
        ));
        let second = interner.resolve(model_stable_key(
            &interner,
            ".polint/models/framework.toml",
            "call:src/app.ts:10:register",
            "function:src/app.ts:1:onRegister",
            ModelConfidence::Heuristic,
            ModelLanguage::TypeScript,
            ("src/app.ts", &["a".to_string(), "b,c".to_string()]),
        ));

        assert_ne!(first, second);
    }

    #[test]
    fn loader_rejects_absolute_and_parent_paths() {
        assert!(matches!(
            load_model_file(
                &LocalAnalysisDb::new().stable_key_interner(),
                "/tmp/model.toml",
                VALID
            ),
            Err(ModelLoadError::InvalidPath(_))
        ));
        assert!(matches!(
            load_model_file(
                &LocalAnalysisDb::new().stable_key_interner(),
                "../model.toml",
                VALID
            ),
            Err(ModelLoadError::InvalidPath(_))
        ));
    }

    #[test]
    fn loader_requires_all_schema_fields() {
        let err = load_model_file(
            &LocalAnalysisDb::new().stable_key_interner(),
            ".polint/models/framework.toml",
            r#"
[[facts]]
source_pattern = "call:a"
"#,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ModelLoadError::MissingField {
                index: 0,
                field: "target_pattern"
            }
        );
    }
}
