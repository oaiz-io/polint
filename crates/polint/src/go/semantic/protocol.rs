use serde::Deserialize;
use std::collections::BTreeSet;

pub const GO_SEMANTIC_SCHEMA: &str = "polint-go-semantic-3";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoSemanticProtocolError {
    InvalidJson(String),
    UnsupportedSchema(String),
    UnknownKind(String),
    RowBeforeBegin(String),
    RowAfterEnd(String),
    DuplicateBegin,
    DuplicateEnd,
    MissingBegin,
    MissingEnd,
}

impl std::fmt::Display for GoSemanticProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(f, "invalid Go semantic NDJSON row: {error}"),
            Self::UnsupportedSchema(schema) => write!(
                f,
                "unsupported Go semantic schema `{schema}`; expected `{GO_SEMANTIC_SCHEMA}`"
            ),
            Self::UnknownKind(kind) => write!(f, "unknown Go semantic frame kind `{kind}`"),
            Self::RowBeforeBegin(kind) => {
                write!(
                    f,
                    "Go semantic frame `{kind}` appeared before session_begin"
                )
            }
            Self::RowAfterEnd(kind) => {
                write!(f, "Go semantic frame `{kind}` appeared after session_end")
            }
            Self::DuplicateBegin => write!(f, "duplicate Go semantic session_begin frame"),
            Self::DuplicateEnd => write!(f, "duplicate Go semantic session_end frame"),
            Self::MissingBegin => write!(f, "missing Go semantic session_begin frame"),
            Self::MissingEnd => write!(f, "missing Go semantic session_end frame"),
        }
    }
}

impl std::error::Error for GoSemanticProtocolError {}

#[derive(Debug, Clone, Deserialize)]
pub struct GoSemanticRawFrame {
    pub schema: String,
    pub kind: String,
    #[serde(default)]
    pub package_id: String,
    #[serde(default)]
    pub package_path: String,
    #[serde(default)]
    pub package_name: String,
    #[serde(default)]
    pub module_path: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub qualified: String,
    #[serde(default)]
    pub signature: String,
    #[serde(default, rename = "stable_key")]
    pub stable_key_text: String,
    #[serde(default)]
    pub receiver: String,
    #[serde(default)]
    pub method: String,
    #[serde(default, rename = "type")]
    pub type_name: String,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub span: Option<GoSemanticSpan>,
    #[serde(default)]
    pub caller: String,
    #[serde(default)]
    pub callee: String,
    #[serde(default)]
    pub edge_kind: String,
    #[serde(default)]
    pub static_callee: String,
    #[serde(default)]
    pub function: String,
    #[serde(default)]
    pub interface_type: String,
    #[serde(default, rename = "callsite_stable_key")]
    pub callsite_stable_key_text: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub go_version: String,
    #[serde(default)]
    pub x_tools_version: String,
    #[serde(default)]
    pub phase: String,
    #[serde(default)]
    pub elapsed_ms: u64,
    #[serde(default)]
    pub packages: u64,
    #[serde(default)]
    pub compiled_go_files: u64,
    #[serde(default)]
    pub deps_with_types: u64,
    #[serde(default)]
    pub rows_emitted: u64,
    #[serde(default)]
    pub peak_heap_bytes: u64,
}

/// One stage of the Go semantic sidecar, with the workload it saw.
///
/// `peak_heap_bytes` is the largest heap allocation the sidecar observed at a
/// stage boundary, not a continuously sampled high-water mark.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GoSemanticPhase {
    pub phase: String,
    pub elapsed_ms: u64,
    pub packages: u64,
    pub compiled_go_files: u64,
    pub deps_with_types: u64,
    pub rows_emitted: u64,
    pub peak_heap_bytes: u64,
}

impl GoSemanticPhase {
    fn from_frame(frame: &GoSemanticRawFrame) -> Self {
        Self {
            phase: frame.phase.clone(),
            elapsed_ms: frame.elapsed_ms,
            packages: frame.packages,
            compiled_go_files: frame.compiled_go_files,
            deps_with_types: frame.deps_with_types,
            rows_emitted: frame.rows_emitted,
            peak_heap_bytes: frame.peak_heap_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GoSemanticSpan {
    pub start_byte: u32,
    pub end_byte: u32,
    pub start_line: u32,
    #[serde(rename = "start_column")]
    pub start_col: u32,
    pub end_line: u32,
    #[serde(rename = "end_column")]
    pub end_col: u32,
}

#[derive(Debug, Clone)]
pub struct GoSemanticOutput {
    pub go_version: String,
    pub x_tools_version: String,
    pub rows: Vec<GoSemanticRawFrame>,
    /// Per-stage timings, in the order the sidecar closed them. Empty when the
    /// sidecar reported none.
    pub phases: Vec<GoSemanticPhase>,
    /// Session totals from `session_end`.
    pub totals: GoSemanticPhase,
}

#[derive(Debug, Clone)]
pub enum GoSemanticFrame {
    SessionBegin(GoSemanticRawFrame),
    Phase(GoSemanticRawFrame),
    Row(GoSemanticRawFrame),
    SessionEnd(GoSemanticRawFrame),
}

pub fn decode_ndjson(bytes: &[u8]) -> Result<GoSemanticOutput, GoSemanticProtocolError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| GoSemanticProtocolError::InvalidJson(error.to_string()))?;
    decode_ndjson_str(text)
}

pub fn decode_ndjson_str(text: &str) -> Result<GoSemanticOutput, GoSemanticProtocolError> {
    let allowed = allowed_kinds();
    let mut saw_begin = false;
    let mut saw_end = false;
    let mut go_version = String::new();
    let mut x_tools_version = String::new();
    let mut rows = Vec::new();
    let mut phases = Vec::new();
    let mut totals = GoSemanticPhase::default();

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let frame: GoSemanticRawFrame = serde_json::from_str(line)
            .map_err(|error| GoSemanticProtocolError::InvalidJson(error.to_string()))?;
        if frame.schema != GO_SEMANTIC_SCHEMA {
            return Err(GoSemanticProtocolError::UnsupportedSchema(frame.schema));
        }
        if !allowed.contains(frame.kind.as_str()) {
            return Err(GoSemanticProtocolError::UnknownKind(frame.kind));
        }
        match classify_frame(frame)? {
            GoSemanticFrame::SessionBegin(frame) => {
                if saw_begin {
                    return Err(GoSemanticProtocolError::DuplicateBegin);
                }
                if saw_end {
                    return Err(GoSemanticProtocolError::DuplicateEnd);
                }
                saw_begin = true;
                go_version = frame.go_version.clone();
                x_tools_version = frame.x_tools_version.clone();
            }
            GoSemanticFrame::SessionEnd(frame) => {
                if !saw_begin {
                    return Err(GoSemanticProtocolError::MissingBegin);
                }
                if saw_end {
                    return Err(GoSemanticProtocolError::DuplicateEnd);
                }
                saw_end = true;
                totals = GoSemanticPhase::from_frame(&frame);
                totals.phase = "session".to_string();
            }
            GoSemanticFrame::Phase(frame) if !saw_begin => {
                return Err(GoSemanticProtocolError::RowBeforeBegin(frame.kind));
            }
            GoSemanticFrame::Phase(frame) if saw_end => {
                return Err(GoSemanticProtocolError::RowAfterEnd(frame.kind));
            }
            GoSemanticFrame::Phase(frame) => phases.push(GoSemanticPhase::from_frame(&frame)),
            GoSemanticFrame::Row(frame) if !saw_begin => {
                return Err(GoSemanticProtocolError::RowBeforeBegin(frame.kind));
            }
            GoSemanticFrame::Row(frame) if saw_end => {
                return Err(GoSemanticProtocolError::RowAfterEnd(frame.kind));
            }
            GoSemanticFrame::Row(frame) => rows.push(frame),
        }
    }

    if !saw_begin {
        return Err(GoSemanticProtocolError::MissingBegin);
    }
    if !saw_end {
        return Err(GoSemanticProtocolError::MissingEnd);
    }

    Ok(GoSemanticOutput {
        go_version,
        x_tools_version,
        rows,
        phases,
        totals,
    })
}

fn classify_frame(frame: GoSemanticRawFrame) -> Result<GoSemanticFrame, GoSemanticProtocolError> {
    match frame.kind.as_str() {
        "session_begin" => Ok(GoSemanticFrame::SessionBegin(frame)),
        "session_end" => Ok(GoSemanticFrame::SessionEnd(frame)),
        "phase" => Ok(GoSemanticFrame::Phase(frame)),
        _ => Ok(GoSemanticFrame::Row(frame)),
    }
}

fn allowed_kinds() -> BTreeSet<&'static str> {
    [
        "session_begin",
        "session_end",
        "phase",
        "package",
        "function",
        "method",
        "receiver_type",
        "init_function",
        "method_set",
        "callsite",
        "type_fact",
        "package_error",
        "unsupported",
        "address_taken",
        "instantiated_type",
        "dynamic_dispatch",
        "rta_edge",
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(row: &str) -> String {
        format!(
            "{{\"schema\":\"polint-go-semantic-3\",\"kind\":\"session_begin\",\"go_version\":\"go1.25.0\",\"x_tools_version\":\"v0.45.0\"}}\n{row}\n{{\"schema\":\"polint-go-semantic-3\",\"kind\":\"session_end\"}}\n"
        )
    }

    #[test]
    fn decode_ndjson_accepts_framed_rows() {
        let output = decode_ndjson_str(&framed(
            "{\"schema\":\"polint-go-semantic-3\",\"kind\":\"package\",\"package_id\":\"p\"}",
        ))
        .expect("framed output decodes");
        assert_eq!(output.rows.len(), 1);
    }

    #[test]
    fn decode_ndjson_collects_phase_rows_without_treating_them_as_facts() {
        let output = decode_ndjson_str(&framed(
            "{\"schema\":\"polint-go-semantic-3\",\"kind\":\"phase\",\"phase\":\"packages_load\",\
             \"elapsed_ms\":1200,\"packages\":7,\"compiled_go_files\":31,\"deps_with_types\":94,\
             \"rows_emitted\":0,\"peak_heap_bytes\":4096}",
        ))
        .expect("phase rows decode");

        assert!(output.rows.is_empty());
        assert_eq!(output.phases.len(), 1);
        assert_eq!(output.phases[0].phase, "packages_load");
        assert_eq!(output.phases[0].elapsed_ms, 1200);
        assert_eq!(output.phases[0].deps_with_types, 94);
    }

    #[test]
    fn decode_ndjson_reads_session_totals() {
        let output = decode_ndjson_str(
            "{\"schema\":\"polint-go-semantic-3\",\"kind\":\"session_begin\"}\n\
             {\"schema\":\"polint-go-semantic-3\",\"kind\":\"session_end\",\"elapsed_ms\":42,\"packages\":3}\n",
        )
        .expect("totals decode");

        assert_eq!(output.totals.elapsed_ms, 42);
        assert_eq!(output.totals.packages, 3);
    }

    #[test]
    fn decode_ndjson_accepts_a_sidecar_that_reports_no_phases() {
        let output = decode_ndjson_str(&framed(
            "{\"schema\":\"polint-go-semantic-3\",\"kind\":\"package\",\"package_id\":\"p\"}",
        ))
        .expect("framed output decodes");

        assert!(output.phases.is_empty());
        assert_eq!(output.totals.elapsed_ms, 0);
        assert_eq!(output.totals.packages, 0);
    }

    #[test]
    fn decode_ndjson_rejects_invalid_json() {
        let err = decode_ndjson_str("{").unwrap_err();
        assert!(err.to_string().contains("invalid Go semantic NDJSON"));
    }

    #[test]
    fn decode_ndjson_rejects_unsupported_schema() {
        let err =
            decode_ndjson_str("{\"schema\":\"other\",\"kind\":\"session_begin\"}").unwrap_err();
        assert!(err.to_string().contains("unsupported Go semantic schema"));
    }

    #[test]
    fn decode_ndjson_rejects_unknown_frame_kind() {
        let err = decode_ndjson_str(&framed(
            "{\"schema\":\"polint-go-semantic-3\",\"kind\":\"mystery\"}",
        ))
        .unwrap_err();
        assert!(err.to_string().contains("unknown Go semantic frame kind"));
    }

    #[test]
    fn decode_ndjson_rejects_missing_terminator() {
        let err =
            decode_ndjson_str("{\"schema\":\"polint-go-semantic-3\",\"kind\":\"session_begin\"}\n")
                .unwrap_err();
        assert_eq!(err, GoSemanticProtocolError::MissingEnd);
    }

    #[test]
    fn decode_ndjson_rejects_rows_after_session_end() {
        let err = decode_ndjson_str(
            "{\"schema\":\"polint-go-semantic-3\",\"kind\":\"session_begin\"}\n\
             {\"schema\":\"polint-go-semantic-3\",\"kind\":\"session_end\"}\n\
             {\"schema\":\"polint-go-semantic-3\",\"kind\":\"package\",\"package_id\":\"p\"}\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            GoSemanticProtocolError::RowAfterEnd("package".to_string())
        );
    }
}
