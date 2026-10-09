use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownCategory {
    SetupMissing,
    UnsupportedSemantic,
    MissingFact,
    OutOfScope,
    GoPackagesLoadFailed,
    GoVersionUnsupported,
    GoSidecarTimeout,
    BudgetExceeded,
    Rejected,
    ModelMissing,
    /// A provider the requested capability depends on did not produce its
    /// output: it failed, or a dependency it needs failed before it ran.
    ProviderFailed,
}

impl UnknownCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SetupMissing => "setup_missing",
            Self::UnsupportedSemantic => "unsupported_semantic",
            Self::MissingFact => "missing_fact",
            Self::OutOfScope => "out_of_scope",
            Self::GoPackagesLoadFailed => "go_packages_load_failed",
            Self::GoVersionUnsupported => "go_version_unsupported",
            Self::GoSidecarTimeout => "go_sidecar_timeout",
            Self::BudgetExceeded => "budget_exceeded",
            Self::Rejected => "rejected",
            Self::ModelMissing => "model_missing",
            Self::ProviderFailed => "provider_failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct UnknownSpan {
    pub line: u32,
    pub column: u32,
}

impl UnknownSpan {
    pub fn from_span(span: &crate::internal_core::Span) -> Self {
        let range = span.diagnostic_range();
        Self {
            line: range.start_line,
            column: range.start_col,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownRow {
    pub category: UnknownCategory,
    pub capability: Option<String>,
    pub family: Option<String>,
    pub provider: String,
    pub file: String,
    pub span: Option<UnknownSpan>,
    pub status: String,
    pub reason: Option<String>,
    pub precision: Option<String>,
    pub docs_path: Option<String>,
    pub suggested_artifact: Option<String>,
    pub source_stable_key: Option<String>,
    pub stable_sort_key: String,
}

impl UnknownRow {
    pub fn new(input: UnknownRowInput) -> Self {
        let stable_sort_key = stable_sort_key(
            input.file.as_str(),
            input.span.as_ref(),
            input.category,
            input.capability.as_deref(),
            input.reason.as_deref(),
            input.source_stable_key.as_deref(),
        );
        Self {
            category: input.category,
            capability: input.capability,
            family: input.family,
            provider: input.provider,
            file: input.file,
            span: input.span,
            status: input.status,
            reason: input.reason,
            precision: input.precision,
            docs_path: input.docs_path,
            suggested_artifact: input.suggested_artifact,
            source_stable_key: input.source_stable_key,
            stable_sort_key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownRowInput {
    pub category: UnknownCategory,
    pub capability: Option<String>,
    pub family: Option<String>,
    pub provider: String,
    pub file: String,
    pub span: Option<UnknownSpan>,
    pub status: String,
    pub reason: Option<String>,
    pub precision: Option<String>,
    pub docs_path: Option<String>,
    pub suggested_artifact: Option<String>,
    pub source_stable_key: Option<String>,
}

pub fn normalize_rows(mut rows: Vec<UnknownRow>) -> Vec<UnknownRow> {
    rows.sort_by(|left, right| left.stable_sort_key.cmp(&right.stable_sort_key));
    rows.dedup_by(|left, right| left.stable_sort_key == right.stable_sort_key);
    rows
}

fn stable_sort_key(
    file: &str,
    span: Option<&UnknownSpan>,
    category: UnknownCategory,
    capability: Option<&str>,
    reason: Option<&str>,
    source_stable_key: Option<&str>,
) -> String {
    let (line, column) = span.map_or((0, 0), |span| (span.line, span.column));
    crate::analysis_api::stable_key_text_from_parts(
        crate::analysis_api::FactFamily::UnsupportedSemantic,
        &[
            ("file", file.to_string()),
            ("line", line.to_string()),
            ("column", column.to_string()),
            ("category", category.as_str().to_string()),
            ("capability", capability.unwrap_or_default().to_string()),
            ("reason", reason.unwrap_or_default().to_string()),
            ("source", source_stable_key.unwrap_or_default().to_string()),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::LocalAnalysisDb;

    #[test]
    fn category_labels_are_stable_snake_case() {
        assert_eq!(UnknownCategory::SetupMissing.as_str(), "setup_missing");
        assert_eq!(
            UnknownCategory::GoPackagesLoadFailed.as_str(),
            "go_packages_load_failed"
        );
        assert_eq!(
            UnknownCategory::GoSidecarTimeout.as_str(),
            "go_sidecar_timeout"
        );
    }

    #[test]
    fn rows_sort_deterministically_by_stable_sort_key() {
        let mut rows = vec![row("b.ts", 3, "z"), row("a.ts", 2, "a")];

        rows = normalize_rows(rows);

        assert_eq!(rows[0].file, "a.ts");
        assert_eq!(rows[1].file, "b.ts");
    }

    fn row(file: &str, line: u32, source: &str) -> UnknownRow {
        UnknownRow::new(UnknownRowInput {
            category: UnknownCategory::SetupMissing,
            capability: Some("references".to_string()),
            family: Some("Reference".to_string()),
            provider: "polint.symbol_graph".to_string(),
            file: file.to_string(),
            span: Some(UnknownSpan { line, column: 1 }),
            status: "setup_missing".to_string(),
            reason: Some("test".to_string()),
            precision: Some("setup_missing".to_string()),
            docs_path: None,
            suggested_artifact: None,
            source_stable_key: Some(source.to_string()),
        })
    }
}
