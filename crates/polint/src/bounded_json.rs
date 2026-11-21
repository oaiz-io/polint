//! JSON read from a repository or a package, bounded in size and in depth.
//!
//! A manifest, a config, a lockfile, a stored graph or a set of corrections is
//! input a repository or a package controls. Every parse of such input goes
//! through this module, which refuses a document larger than the caller's
//! size limit or nested deeper than [`MAX_JSON_DEPTH`] before a parser sees
//! it. A parser or a walker that recurses once per level therefore never
//! recurses deeper than the bound, whichever parser the input reaches. A
//! refused document is an error the caller reports as an unknown.

use serde::de::DeserializeOwned;

/// The deepest nesting of objects and arrays a document may have. It is the
/// deepest `serde_json` parses on its own, so no document `serde_json` would
/// parse is refused for its depth.
pub(crate) const MAX_JSON_DEPTH: usize = 127;

/// Why a document was not parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum BoundedJsonError {
    #[error("the document is larger than {max} bytes")]
    TooLarge { max: u64 },
    #[error("the document nests deeper than {MAX_JSON_DEPTH} levels")]
    TooDeep,
    #[error("the document is not valid JSON: {0}")]
    Invalid(String),
}

impl BoundedJsonError {
    /// The reason a topology fact gives for a document it could not read.
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Invalid(_) => "malformed json".to_string(),
            refused => refused.to_string(),
        }
    }
}

/// Whether `bytes` is at most `max_bytes` long and nests objects and arrays
/// at most [`MAX_JSON_DEPTH`] deep. Brackets inside strings and inside `//`
/// and `/* */` comments do not count, so the check holds for JSON with
/// comments too.
pub(crate) fn check_bounds(bytes: &[u8], max_bytes: u64) -> Result<(), BoundedJsonError> {
    if bytes.len() as u64 > max_bytes {
        return Err(BoundedJsonError::TooLarge { max: max_bytes });
    }
    if nesting_depth_exceeds(bytes, MAX_JSON_DEPTH) {
        return Err(BoundedJsonError::TooDeep);
    }
    Ok(())
}

/// `bytes` parsed as JSON of the shape `T`.
pub(crate) fn from_slice<T: DeserializeOwned>(
    bytes: &[u8],
    max_bytes: u64,
) -> Result<T, BoundedJsonError> {
    check_bounds(bytes, max_bytes)?;
    serde_json::from_slice(bytes).map_err(|error| BoundedJsonError::Invalid(error.to_string()))
}

/// `source` parsed as JSON of the shape `T`.
pub(crate) fn from_str<T: DeserializeOwned>(
    source: &str,
    max_bytes: u64,
) -> Result<T, BoundedJsonError> {
    from_slice(source.as_bytes(), max_bytes)
}

/// `source` parsed as JSON with comments and trailing commas, after a byte
/// order mark, of the shape `T`.
pub(crate) fn from_jsonc_str<T: DeserializeOwned>(
    source: &str,
    max_bytes: u64,
) -> Result<T, BoundedJsonError> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    check_bounds(source.as_bytes(), max_bytes)?;
    let mut source = source.to_string();
    json_strip_comments::strip(&mut source)
        .map_err(|error| BoundedJsonError::Invalid(error.to_string()))?;
    serde_json::from_str(&source).map_err(|error| BoundedJsonError::Invalid(error.to_string()))
}

/// Whether objects and arrays in `bytes` nest deeper than `max`.
fn nesting_depth_exceeds(bytes: &[u8], max: usize) -> bool {
    enum State {
        Code,
        Text { escaped: bool },
        LineComment,
        BlockComment { star: bool },
    }
    let mut depth = 0_usize;
    let mut state = State::Code;
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        state = match state {
            State::Code => match byte {
                b'"' => State::Text { escaped: false },
                b'/' if bytes.get(index + 1) == Some(&b'/') => {
                    index += 1;
                    State::LineComment
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    index += 1;
                    State::BlockComment { star: false }
                }
                b'{' | b'[' => {
                    depth += 1;
                    if depth > max {
                        return true;
                    }
                    State::Code
                }
                b'}' | b']' => {
                    depth = depth.saturating_sub(1);
                    State::Code
                }
                _ => State::Code,
            },
            State::Text { escaped: true } => State::Text { escaped: false },
            State::Text { escaped: false } => match byte {
                b'\\' => State::Text { escaped: true },
                b'"' => State::Code,
                _ => State::Text { escaped: false },
            },
            State::LineComment => match byte {
                b'\n' => State::Code,
                _ => State::LineComment,
            },
            State::BlockComment { star } => match byte {
                b'/' if star => State::Code,
                b'*' => State::BlockComment { star: true },
                _ => State::BlockComment { star: false },
            },
        };
        index += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested(levels: usize) -> String {
        format!("{}\"x\"{}", "{\"a\":".repeat(levels), "}".repeat(levels))
    }

    #[test]
    fn a_document_at_the_depth_bound_parses_and_one_deeper_is_refused() {
        let at_bound = nested(MAX_JSON_DEPTH);
        assert!(from_str::<serde_json::Value>(&at_bound, u64::MAX).is_ok());
        assert_eq!(
            from_str::<serde_json::Value>(&nested(MAX_JSON_DEPTH + 1), u64::MAX),
            Err(BoundedJsonError::TooDeep)
        );
        assert_eq!(
            check_bounds(nested(10_000).as_bytes(), u64::MAX),
            Err(BoundedJsonError::TooDeep)
        );
    }

    #[test]
    fn brackets_in_strings_and_comments_do_not_count() {
        let quoted = format!(r#"{{"a":"{}\"{}"}}"#, "[".repeat(500), "{".repeat(500));
        assert!(from_str::<serde_json::Value>(&quoted, u64::MAX).is_ok());
        let commented = format!(
            "// {}\n{{/* {} */\"a\":1}}",
            "[".repeat(500),
            "{*".repeat(500)
        );
        assert!(from_jsonc_str::<serde_json::Value>(&commented, u64::MAX).is_ok());
    }

    #[test]
    fn a_document_over_the_size_limit_is_refused() {
        assert_eq!(
            from_str::<serde_json::Value>("[1,2,3]", 6),
            Err(BoundedJsonError::TooLarge { max: 6 })
        );
        assert!(from_str::<serde_json::Value>("[1,2,3]", 7).is_ok());
    }
}
