use serde::Deserialize;
use std::collections::BTreeSet;

pub(crate) const TS_TYPES_SCHEMA: &str = "polint-ts-types-1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TsTypesProtocolError {
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

impl std::fmt::Display for TsTypesProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(f, "invalid TS types NDJSON row: {error}"),
            Self::UnsupportedSchema(schema) => write!(
                f,
                "unsupported TS types schema `{schema}`; expected `{TS_TYPES_SCHEMA}`"
            ),
            Self::UnknownKind(kind) => write!(f, "unknown TS types frame kind `{kind}`"),
            Self::RowBeforeBegin(kind) => {
                write!(f, "TS types frame `{kind}` appeared before session_begin")
            }
            Self::RowAfterEnd(kind) => {
                write!(f, "TS types frame `{kind}` appeared after session_end")
            }
            Self::DuplicateBegin => write!(f, "duplicate TS types session_begin frame"),
            Self::DuplicateEnd => write!(f, "duplicate TS types session_end frame"),
            Self::MissingBegin => write!(f, "missing TS types session_begin frame"),
            Self::MissingEnd => write!(f, "missing TS types session_end frame"),
        }
    }
}

impl std::error::Error for TsTypesProtocolError {}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct TsTypesSpan {
    pub(crate) start_byte: u32,
    pub(crate) end_byte: u32,
    pub(crate) start_line: u32,
    #[serde(rename = "start_column")]
    pub(crate) start_col: u32,
    pub(crate) end_line: u32,
    #[serde(rename = "end_column")]
    pub(crate) end_col: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TsTypesRawFrame {
    pub(crate) schema: String,
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) project: String,
    #[serde(default)]
    pub(crate) options_digest: String,
    #[serde(default)]
    pub(crate) typescript_version: String,
    #[serde(default)]
    pub(crate) node_version: String,
    #[serde(default)]
    pub(crate) file_count: u64,
    #[serde(default)]
    pub(crate) callable: String,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) callable_kind: String,
    #[serde(default)]
    pub(crate) file: String,
    #[serde(default)]
    pub(crate) span: Option<TsTypesSpan>,
    #[serde(default)]
    pub(crate) name_span: Option<TsTypesSpan>,
    #[serde(default)]
    pub(crate) callsite: String,
    #[serde(default)]
    pub(crate) enclosing: String,
    #[serde(default)]
    pub(crate) call_kind: String,
    #[serde(default)]
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) reason: String,
    #[serde(default, rename = "callsite_stable_key")]
    pub(crate) callsite_stable_key_text: String,
    #[serde(default)]
    pub(crate) external: String,
    #[serde(default)]
    pub(crate) dispatch: String,
    #[serde(default)]
    pub(crate) printed: String,
    #[serde(default)]
    pub(crate) is_any: bool,
    #[serde(default)]
    pub(crate) is_unknown: bool,
    #[serde(default)]
    pub(crate) union_size: u64,
    #[serde(default)]
    pub(crate) callsites: u64,
    #[serde(default)]
    pub(crate) any_receivers: u64,
    #[serde(default)]
    pub(crate) category: String,
    #[serde(default)]
    pub(crate) message: String,
    #[serde(default, rename = "stable_key")]
    pub(crate) stable_key_text: String,
    #[serde(default)]
    pub(crate) phase: String,
    #[serde(default)]
    pub(crate) elapsed_ms: u64,
    #[serde(default)]
    pub(crate) projects: u64,
    #[serde(default)]
    pub(crate) files: u64,
    #[serde(default)]
    pub(crate) rows_emitted: u64,
    #[serde(default)]
    pub(crate) peak_heap_bytes: u64,
}

/// One stage of the TS type sidecar, with the workload it saw.
///
/// `peak_heap_bytes` is the heap the sidecar reported at a stage boundary, not
/// a continuously sampled high-water mark.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct TsTypesPhase {
    pub(crate) phase: String,
    pub(crate) elapsed_ms: u64,
    pub(crate) projects: u64,
    pub(crate) files: u64,
    pub(crate) rows_emitted: u64,
    pub(crate) peak_heap_bytes: u64,
}

impl TsTypesPhase {
    fn from_frame(frame: &TsTypesRawFrame) -> Self {
        Self {
            phase: frame.phase.clone(),
            elapsed_ms: frame.elapsed_ms,
            projects: frame.projects,
            files: frame.files,
            rows_emitted: frame.rows_emitted,
            peak_heap_bytes: frame.peak_heap_bytes,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TsTypesOutput {
    pub(crate) typescript_version: String,
    pub(crate) node_version: String,
    pub(crate) rows: Vec<TsTypesRawFrame>,
    /// Per-stage timings in the order the sidecar closed them. Empty when the
    /// sidecar reported none.
    pub(crate) phases: Vec<TsTypesPhase>,
    /// Session totals from `session_end`.
    pub(crate) totals: TsTypesPhase,
}

enum TsTypesFrame {
    SessionBegin(TsTypesRawFrame),
    Phase(TsTypesRawFrame),
    Row(TsTypesRawFrame),
    SessionEnd(TsTypesRawFrame),
}

pub(crate) fn decode_ndjson(bytes: &[u8]) -> Result<TsTypesOutput, TsTypesProtocolError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| TsTypesProtocolError::InvalidJson(error.to_string()))?;
    decode_ndjson_str(text)
}

pub(crate) fn decode_ndjson_str(text: &str) -> Result<TsTypesOutput, TsTypesProtocolError> {
    let allowed = allowed_kinds();
    let mut saw_begin = false;
    let mut saw_end = false;
    let mut typescript_version = String::new();
    let mut node_version = String::new();
    let mut rows = Vec::new();
    let mut phases = Vec::new();
    let mut totals = TsTypesPhase::default();

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let frame: TsTypesRawFrame = serde_json::from_str(line)
            .map_err(|error| TsTypesProtocolError::InvalidJson(error.to_string()))?;
        if frame.schema != TS_TYPES_SCHEMA {
            return Err(TsTypesProtocolError::UnsupportedSchema(frame.schema));
        }
        if !allowed.contains(frame.kind.as_str()) {
            return Err(TsTypesProtocolError::UnknownKind(frame.kind));
        }
        match classify_frame(frame) {
            TsTypesFrame::SessionBegin(frame) => {
                if saw_begin {
                    return Err(TsTypesProtocolError::DuplicateBegin);
                }
                if saw_end {
                    return Err(TsTypesProtocolError::DuplicateEnd);
                }
                saw_begin = true;
                typescript_version = frame.typescript_version.clone();
                node_version = frame.node_version.clone();
            }
            TsTypesFrame::SessionEnd(frame) => {
                if !saw_begin {
                    return Err(TsTypesProtocolError::MissingBegin);
                }
                if saw_end {
                    return Err(TsTypesProtocolError::DuplicateEnd);
                }
                saw_end = true;
                totals = TsTypesPhase::from_frame(&frame);
                totals.phase = "session".to_string();
            }
            TsTypesFrame::Phase(frame) if !saw_begin => {
                return Err(TsTypesProtocolError::RowBeforeBegin(frame.kind));
            }
            TsTypesFrame::Phase(frame) if saw_end => {
                return Err(TsTypesProtocolError::RowAfterEnd(frame.kind));
            }
            TsTypesFrame::Phase(frame) => phases.push(TsTypesPhase::from_frame(&frame)),
            TsTypesFrame::Row(frame) if !saw_begin => {
                return Err(TsTypesProtocolError::RowBeforeBegin(frame.kind));
            }
            TsTypesFrame::Row(frame) if saw_end => {
                return Err(TsTypesProtocolError::RowAfterEnd(frame.kind));
            }
            TsTypesFrame::Row(frame) => rows.push(frame),
        }
    }

    if !saw_begin {
        return Err(TsTypesProtocolError::MissingBegin);
    }
    if !saw_end {
        return Err(TsTypesProtocolError::MissingEnd);
    }

    Ok(TsTypesOutput {
        typescript_version,
        node_version,
        rows,
        phases,
        totals,
    })
}

fn classify_frame(frame: TsTypesRawFrame) -> TsTypesFrame {
    match frame.kind.as_str() {
        "session_begin" => TsTypesFrame::SessionBegin(frame),
        "session_end" => TsTypesFrame::SessionEnd(frame),
        "phase" => TsTypesFrame::Phase(frame),
        _ => TsTypesFrame::Row(frame),
    }
}

fn allowed_kinds() -> BTreeSet<&'static str> {
    [
        "session_begin",
        "session_end",
        "phase",
        "project",
        "callable",
        "callsite",
        "callee",
        "receiver",
        "any_density",
        "diagnostic",
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(row: &str) -> String {
        format!(
            "{{\"schema\":\"polint-ts-types-1\",\"kind\":\"session_begin\",\
             \"typescript_version\":\"5.9.3\",\"node_version\":\"v22.0.0\"}}\n{row}\n\
             {{\"schema\":\"polint-ts-types-1\",\"kind\":\"session_end\"}}\n"
        )
    }

    #[test]
    fn decode_ndjson_accepts_framed_rows() {
        let output = decode_ndjson_str(&framed(
            "{\"schema\":\"polint-ts-types-1\",\"kind\":\"project\",\"project\":\"tsconfig.json\"}",
        ))
        .expect("framed output decodes");

        assert_eq!(output.rows.len(), 1);
        assert_eq!(output.typescript_version, "5.9.3");
        assert_eq!(output.node_version, "v22.0.0");
    }

    #[test]
    fn decode_ndjson_collects_phase_rows_without_treating_them_as_facts() {
        let output = decode_ndjson_str(&framed(
            "{\"schema\":\"polint-ts-types-1\",\"kind\":\"phase\",\"phase\":\"create_program\",\
             \"elapsed_ms\":812,\"projects\":1,\"files\":84,\"rows_emitted\":0,\
             \"peak_heap_bytes\":4096}",
        ))
        .expect("phase rows decode");

        assert!(output.rows.is_empty());
        assert_eq!(output.phases.len(), 1);
        assert_eq!(output.phases[0].phase, "create_program");
        assert_eq!(output.phases[0].elapsed_ms, 812);
        assert_eq!(output.phases[0].files, 84);
    }

    #[test]
    fn decode_ndjson_reads_session_totals() {
        let output = decode_ndjson_str(
            "{\"schema\":\"polint-ts-types-1\",\"kind\":\"session_begin\"}\n\
             {\"schema\":\"polint-ts-types-1\",\"kind\":\"session_end\",\"elapsed_ms\":42,\
             \"projects\":3}\n",
        )
        .expect("totals decode");

        assert_eq!(output.totals.elapsed_ms, 42);
        assert_eq!(output.totals.projects, 3);
        assert_eq!(output.totals.phase, "session");
    }

    #[test]
    fn decode_ndjson_rejects_invalid_json() {
        let error = decode_ndjson_str("{").unwrap_err();
        assert!(error.to_string().contains("invalid TS types NDJSON"));
    }

    #[test]
    fn decode_ndjson_rejects_unsupported_schema() {
        let error =
            decode_ndjson_str("{\"schema\":\"other\",\"kind\":\"session_begin\"}").unwrap_err();
        assert!(error.to_string().contains("unsupported TS types schema"));
    }

    #[test]
    fn decode_ndjson_rejects_unknown_frame_kind() {
        let error = decode_ndjson_str(&framed(
            "{\"schema\":\"polint-ts-types-1\",\"kind\":\"mystery\"}",
        ))
        .unwrap_err();
        assert!(error.to_string().contains("unknown TS types frame kind"));
    }

    #[test]
    fn decode_ndjson_rejects_missing_terminator() {
        let error =
            decode_ndjson_str("{\"schema\":\"polint-ts-types-1\",\"kind\":\"session_begin\"}\n")
                .unwrap_err();
        assert_eq!(error, TsTypesProtocolError::MissingEnd);
    }

    #[test]
    fn decode_ndjson_rejects_rows_before_session_begin() {
        let error = decode_ndjson_str(
            "{\"schema\":\"polint-ts-types-1\",\"kind\":\"project\",\"project\":\"t\"}\n",
        )
        .unwrap_err();
        assert_eq!(
            error,
            TsTypesProtocolError::RowBeforeBegin("project".to_string())
        );
    }

    #[test]
    fn decode_ndjson_rejects_rows_after_session_end() {
        let error = decode_ndjson_str(
            "{\"schema\":\"polint-ts-types-1\",\"kind\":\"session_begin\"}\n\
             {\"schema\":\"polint-ts-types-1\",\"kind\":\"session_end\"}\n\
             {\"schema\":\"polint-ts-types-1\",\"kind\":\"project\",\"project\":\"t\"}\n",
        )
        .unwrap_err();
        assert_eq!(
            error,
            TsTypesProtocolError::RowAfterEnd("project".to_string())
        );
    }

    #[test]
    fn decode_ndjson_rejects_duplicate_session_begin() {
        let error = decode_ndjson_str(
            "{\"schema\":\"polint-ts-types-1\",\"kind\":\"session_begin\"}\n\
             {\"schema\":\"polint-ts-types-1\",\"kind\":\"session_begin\"}\n",
        )
        .unwrap_err();
        assert_eq!(error, TsTypesProtocolError::DuplicateBegin);
    }
}
