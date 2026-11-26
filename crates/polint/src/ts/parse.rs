//! Shared TS/JS parse entrypoint.
//!
//! Every Oxc parse site in this crate must go through [`parse_ts_source`]. When Oxc
//! recovers from a syntax error the returned AST is partial: consumers must record
//! an unsupported fact rather than treating missing facts as absences.

use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_parser::{Parser, ParserReturn};
use oxc_span::SourceType;
use std::path::Path;

use crate::analysis_api::SourceFile;
use crate::ts::nesting::{MAX_NESTING_DEPTH, nesting_past_limit};

/// Construct name for unsupported-semantic rows recorded on recoverable Oxc errors.
pub const PARSER_RECOVERY_CONSTRUCT: &str = "parser recovery";

/// Status reason secondary extractors use when they continue on a partial AST.
pub const PARTIAL_AST_REASON: &str = "partial AST from parse errors";

/// Owned parse-error summary taken from Oxc diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub start_byte: Option<usize>,
    pub end_byte: Option<usize>,
}

/// Result of [`parse_ts_source`].
///
/// `fully_parsed == false` means the AST is partial (recoverable errors) or empty
/// (catastrophic panic). Downstream analysis may continue, but must declare the gap.
pub struct ParsedTsSource<'a> {
    pub raw: ParserReturn<'a>,
    pub errors: Vec<ParseError>,
    pub fully_parsed: bool,
}

impl<'a> ParsedTsSource<'a> {
    pub fn program(&self) -> &Program<'a> {
        &self.raw.program
    }

    /// True when Oxc aborted and left an empty program body.
    pub fn is_catastrophic(&self) -> bool {
        self.raw.panicked && self.raw.program.body.is_empty()
    }
}

/// Derive Oxc [`SourceType`] from the file path. All parse sites must use this.
pub fn source_type(path: &Path) -> SourceType {
    SourceType::from_path(path).unwrap_or_default()
}

/// The only sanctioned way to parse TS/JS in this crate.
///
/// A source that nests deeper than [`MAX_NESTING_DEPTH`] is not parsed: the
/// result is catastrophic, with one error that names the limit.
pub fn parse_ts_source<'a>(
    allocator: &'a Allocator,
    path: &Path,
    source: &'a str,
) -> ParsedTsSource<'a> {
    if let Some(offset) = nesting_past_limit(source, MAX_NESTING_DEPTH) {
        return nested_too_deep(allocator, path, source, offset);
    }
    let raw = Parser::new(allocator, source, source_type(path)).parse();
    let errors = raw
        .errors
        .iter()
        .map(|error| {
            let (start_byte, end_byte) = error
                .labels
                .as_ref()
                .and_then(|labels| labels.first())
                .map(|label| {
                    let start = label.offset();
                    (Some(start), Some(start + label.len()))
                })
                .unwrap_or((None, None));
            ParseError {
                message: error.to_string(),
                start_byte,
                end_byte,
            }
        })
        .collect::<Vec<_>>();
    let fully_parsed = errors.is_empty() && !(raw.panicked && raw.program.body.is_empty());
    ParsedTsSource {
        raw,
        errors,
        fully_parsed,
    }
}

/// The result for a source that nests past [`MAX_NESTING_DEPTH`] at `offset`:
/// an empty program over `source`, marked as an aborted parse.
fn nested_too_deep<'a>(
    allocator: &'a Allocator,
    path: &Path,
    source: &'a str,
    offset: usize,
) -> ParsedTsSource<'a> {
    let mut raw = Parser::new(allocator, "", source_type(path)).parse();
    raw.program.source_text = source;
    raw.panicked = true;
    ParsedTsSource {
        raw,
        errors: vec![ParseError {
            message: format!(
                "Source nests deeper than {MAX_NESTING_DEPTH} levels; the file is not analysed."
            ),
            start_byte: Some(offset),
            end_byte: Some(offset),
        }],
        fully_parsed: false,
    }
}

/// Parse a [`SourceFile`] through [`parse_ts_source`].
pub fn parse_ts_file<'a>(allocator: &'a Allocator, file: &'a SourceFile) -> ParsedTsSource<'a> {
    parse_ts_source(allocator, &file.path, file.source.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use std::path::Path;

    #[test]
    fn recoverable_syntax_error_marks_ast_partial() {
        let allocator = Allocator::default();
        let parsed = parse_ts_source(
            &allocator,
            Path::new("recoverable.tsx"),
            "const x = <div></span>;",
        );
        assert!(!parsed.errors.is_empty(), "expected oxc syntax errors");
        assert!(
            !parsed.program().body.is_empty(),
            "recoverable parse should keep a non-empty body"
        );
        assert!(!parsed.is_catastrophic());
        assert!(!parsed.fully_parsed);
    }

    #[test]
    fn catastrophic_parse_marks_ast_partial_and_empty() {
        let allocator = Allocator::default();
        let parsed = parse_ts_source(
            &allocator,
            Path::new("broken.ts"),
            "import x from \"./x\";\nconst value = ;",
        );
        assert!(!parsed.fully_parsed);
        assert!(!parsed.errors.is_empty());
        // This fixture panics with an empty body; imports may still appear via module_record.
        assert!(parsed.is_catastrophic());
    }

    #[test]
    fn clean_source_is_fully_parsed() {
        let allocator = Allocator::default();
        let parsed = parse_ts_source(
            &allocator,
            Path::new("ok.ts"),
            "export function ok(value: number) { return value + 1; }",
        );
        assert!(parsed.fully_parsed);
        assert!(parsed.errors.is_empty());
        assert!(!parsed.is_catastrophic());
    }

    /// One source per construct that nests, `levels` deep, with the file name
    /// that selects its dialect. Chains that the parser reads in a loop (binary
    /// operators, member and call chains, array types) still build a syntax tree
    /// as deep as the chain is long.
    fn nested_sources(levels: usize) -> Vec<(&'static str, &'static str, String)> {
        let n = levels;
        vec![
            (
                "paren",
                "a.js",
                format!("x = {}a{};", "(".repeat(n), ")".repeat(n)),
            ),
            (
                "array",
                "a.js",
                format!("x = {}a{};", "[".repeat(n), "]".repeat(n)),
            ),
            (
                "object",
                "a.js",
                format!("x = {}1{};", "{a:".repeat(n), "}".repeat(n)),
            ),
            (
                "block",
                "a.js",
                format!("{}{}", "{".repeat(n), "}".repeat(n)),
            ),
            (
                "function",
                "a.js",
                format!("{}{}", "function f(){".repeat(n), "}".repeat(n)),
            ),
            ("arrow", "a.js", format!("x = {}1;", "a=>".repeat(n))),
            ("conditional", "a.js", format!("x = {}c;", "a?b:".repeat(n))),
            ("unary", "a.js", format!("x = {}a;", "!".repeat(n))),
            ("new", "a.js", format!("x = {}A;", "new ".repeat(n))),
            ("assignment", "a.js", format!("{}1;", "a=".repeat(n))),
            (
                "template",
                "a.js",
                format!("x = {}a{};", "`${".repeat(n), "}`".repeat(n)),
            ),
            (
                "else if",
                "a.js",
                format!("if(a){{}}{}", "else if(a){}".repeat(n)),
            ),
            ("bare if", "a.js", format!("{}a;", "if(a)".repeat(n))),
            (
                "label",
                "a.js",
                format!(
                    "{}a;",
                    (0..n).map(|i| format!("l{i}: ")).collect::<String>()
                ),
            ),
            (
                "destructuring",
                "a.js",
                format!("let {}a{} = b;", "[".repeat(n), "]".repeat(n)),
            ),
            ("binary chain", "a.js", format!("x = {}a;", "a+".repeat(n))),
            (
                "pure binary chain",
                "a.js",
                format!("x = /*#__PURE__*/ f(){};", "+a".repeat(n)),
            ),
            (
                "logical chain",
                "a.js",
                format!("x = {}a;", "a||".repeat(n)),
            ),
            ("member chain", "a.js", format!("x = a{};", ".b".repeat(n))),
            (
                "optional chain",
                "a.js",
                format!("x = a{};", "?.b".repeat(n)),
            ),
            (
                "computed member chain",
                "a.js",
                format!("x = a{};", "[0]".repeat(n)),
            ),
            ("call chain", "a.js", format!("x = a{};", "()".repeat(n))),
            (
                "tagged template chain",
                "a.js",
                format!("x = a{};", "``".repeat(n)),
            ),
            ("non-null chain", "a.ts", format!("x = a{};", "!".repeat(n))),
            (
                "jsx element",
                "a.jsx",
                format!("x = {}{};", "<a>".repeat(n), "</a>".repeat(n)),
            ),
            (
                "jsx member",
                "a.jsx",
                format!("x = <a{} />;", ".b".repeat(n)),
            ),
            (
                "type arguments",
                "a.ts",
                format!("type T = {}string{};", "Array<".repeat(n), "> ".repeat(n)),
            ),
            (
                "type literal",
                "a.ts",
                format!("type T = {}1{};", "{a:".repeat(n), "}".repeat(n)),
            ),
            (
                "type operator",
                "a.ts",
                format!("type T = {}U;", "keyof ".repeat(n)),
            ),
            (
                "conditional type",
                "a.ts",
                format!("type T = {}D;", "A extends B ? C : ".repeat(n)),
            ),
            (
                "array type",
                "a.ts",
                format!("type T = string{};", "[]".repeat(n)),
            ),
            (
                "indexed access type",
                "a.ts",
                format!("type T = U{};", "[0]".repeat(n)),
            ),
            (
                "qualified name",
                "a.ts",
                format!("let v: a{};", ".b".repeat(n)),
            ),
            (
                "type query",
                "a.ts",
                format!("let v: typeof a{};", ".b".repeat(n)),
            ),
            (
                "import type qualifier",
                "a.ts",
                format!("let v: import(\"m\").a{};", ".b".repeat(n)),
            ),
            (
                "namespace",
                "a.ts",
                format!("namespace a{} {{}}", ".b".repeat(n)),
            ),
        ]
    }

    /// Parse on a thread with the stack every analysis worker gets, so a parse
    /// that would overflow a worker overflows here too.
    fn parse_on_a_worker_stack(path: &'static str, source: String) -> (bool, bool, Vec<String>) {
        std::thread::Builder::new()
            .stack_size(crate::jobs::WORKER_STACK_BYTES)
            .spawn(move || {
                let allocator = Allocator::default();
                let parsed = parse_ts_source(&allocator, Path::new(path), &source);
                let messages = parsed
                    .errors
                    .iter()
                    .map(|error| error.message.clone())
                    .collect();
                (parsed.fully_parsed, parsed.is_catastrophic(), messages)
            })
            .expect("spawn parse thread")
            .join()
            .expect("parse thread finished")
    }

    #[test]
    fn a_file_nested_past_the_limit_is_reported_and_not_analysed() {
        for (construct, path, source) in nested_sources(100_000) {
            let (fully_parsed, catastrophic, messages) = parse_on_a_worker_stack(path, source);
            assert!(
                !fully_parsed && catastrophic,
                "{construct}: a file nested past the limit has no syntax tree to analyse"
            );
            assert!(
                messages
                    .iter()
                    .any(|message| message.contains("deeper than")),
                "{construct}: the error names the nesting limit: {messages:?}"
            );
        }
    }

    #[test]
    fn a_file_nested_well_within_the_limit_parses_fully() {
        for (construct, path, source) in nested_sources(500) {
            let (fully_parsed, _, messages) = parse_on_a_worker_stack(path, source);
            assert!(fully_parsed, "{construct}: {messages:?}");
        }
    }

    /// The most levels of one construct that stay within the nesting limit.
    fn deepest_accepted_levels(construct: &str) -> usize {
        let within = |levels: usize| {
            let (_, _, source) = nested_sources(levels)
                .into_iter()
                .find(|(name, _, _)| *name == construct)
                .expect("known construct");
            nesting_past_limit(&source, MAX_NESTING_DEPTH).is_none()
        };
        let (mut low, mut high) = (1, MAX_NESTING_DEPTH as usize + 1);
        while low + 1 < high {
            let middle = (low + high) / 2;
            if within(middle) {
                low = middle;
            } else {
                high = middle;
            }
        }
        low
    }

    /// Every construct nested as deep as the limit admits parses, and its
    /// semantic walk finishes, on the stack an analysis worker gets.
    #[test]
    fn a_file_at_the_nesting_limit_parses_and_walks_on_a_worker_stack() {
        for (construct, _, _) in nested_sources(1) {
            let levels = deepest_accepted_levels(construct);
            let (_, path, source) = nested_sources(levels)
                .into_iter()
                .find(|(name, _, _)| *name == construct)
                .expect("known construct");
            let (fully_parsed, scopes) = std::thread::Builder::new()
                .stack_size(crate::jobs::WORKER_STACK_BYTES)
                .spawn(move || {
                    let allocator = Allocator::default();
                    let parsed = parse_ts_source(&allocator, Path::new(path), &source);
                    let semantic = oxc_semantic::SemanticBuilder::new()
                        .build(parsed.program())
                        .semantic;
                    (parsed.fully_parsed, semantic.scoping().scopes_len())
                })
                .expect("spawn parse thread")
                .join()
                .expect("parse and walk finish without overflowing the stack");
            assert!(fully_parsed, "{construct} at {levels} levels");
            assert!(scopes > 0, "{construct} at {levels} levels");
        }
    }

    #[test]
    fn every_ts_parse_site_routes_through_the_shared_helper() {
        let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ts");
        let mut parser_new_relpaths = Vec::new();
        for entry in walkdir_rs_files(&src_root) {
            let source = std::fs::read_to_string(&entry).expect("read rust source");
            if !source.contains("Parser::new(") {
                continue;
            }
            let rel = entry
                .strip_prefix(&src_root)
                .expect("file under src")
                .to_path_buf();
            parser_new_relpaths.push(rel);
        }
        assert_eq!(
            parser_new_relpaths,
            vec![Path::new("parse.rs").to_path_buf()],
            "Oxc Parser::new must live only in crate::ts::parse::parse_ts_source; found {parser_new_relpaths:?}"
        );
    }

    fn walkdir_rs_files(root: &Path) -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, files);
                } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                    files.push(path);
                }
            }
        }
        walk(root, &mut files);
        files.sort();
        files
    }
}
