//! Shared syntax fact row types produced by language frontends.
//!
//! These types are SourceFile-adjacent and owned here so frontends (`polint-go`,
//! `polint-ts`) can write them without depending on the facade.

use crate::internal_core::{
    BranchId, Diagnostic, FileId, FunctionId, ImportId, Language, PackageId, Span,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FunctionFact {
    pub id: FunctionId,
    pub file: FileId,
    pub name: String,
    pub span: Span,
    pub language: Language,
    pub is_test: bool,
    pub is_exported: bool,
    pub cyclomatic_complexity: u32,
    pub calls: Vec<String>,
}

/// Name used for the synthetic per-module function row inserted for TS/JS files.
pub const TS_JS_MODULE_FUNCTION_NAME: &str = "<polint:module>";

/// Structural classification of a Go type declaration or anonymous struct occurrence.
///
/// `Named` covers every underlying type expression that is neither a struct nor an
/// interface body (aliases, pointers, slices, maps, generics, qualified names, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum GoTypeDeclKind {
    /// `struct { ... }` body, either a named `type X struct` spec or an anonymous
    /// `struct { ... }` occurrence.
    Struct,
    /// `interface { ... }` body of a named `type X interface` spec.
    Interface,
    /// Any other underlying type expression on a named declaration.
    Named,
}

/// Typed Go structural fact describing one type declaration or anonymous struct occurrence.
///
/// Two row shapes share this type:
///
/// - **Named declarations** (`type_spec` / `type_alias` nodes, including grouped
///   `type ( ... )` members and function-local declarations): `name` is `Some`,
///   `span` is the declared **name token** span, and `declaration_start_byte` points
///   at the `type` keyword of the enclosing `type_declaration`.
/// - **Anonymous struct occurrences** (every `struct_type` node that is not the type
///   of a named declaration): `name` is `None`, `span` is the `struct { ... }` node
///   span, and `declaration_start_byte` equals `span.start_byte`.
///
/// `body_range` is the byte range **inside** the braces for struct and interface
/// bodies; `None` otherwise. All byte offsets are UTF-8 offsets into the file source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GoTypeDeclFact {
    pub file: FileId,
    pub span: Span,
    /// Declared identifier for named declarations; `None` for anonymous struct occurrences.
    pub name: Option<String>,
    pub kind: GoTypeDeclKind,
    /// `type A = B` alias declaration.
    pub is_alias: bool,
    /// Declared inside a grouped `type ( ... )` declaration.
    pub is_grouped: bool,
    /// Declared directly in the file body rather than inside a function or block.
    pub is_top_level: bool,
    /// Declared with type parameters (`type X[T any] ...`).
    pub has_type_parameters: bool,
    /// Final identifier of a named, non-struct/interface underlying type expression
    /// (`type X Y.Z` -> `Z`, `type X Y[T]` -> `Y`); `None` for struct and interface
    /// bodies and for expressions headed by non-identifier forms (pointers, maps, ...).
    pub direct_name: Option<String>,
    /// Byte range inside the braces of a struct or interface body; `None` otherwise.
    pub body_range: Option<(u32, u32)>,
    /// Byte offset of the `type` keyword for named declarations, or of the
    /// `struct` keyword for anonymous occurrences.
    pub declaration_start_byte: u32,
}

impl GoTypeDeclFact {
    /// Constructs a Go structural fact from its complete fields.
    #[expect(
        clippy::too_many_arguments,
        reason = "This constructor mirrors the complete non-exhaustive public fact schema."
    )]
    pub fn new(
        file: FileId,
        span: Span,
        name: Option<String>,
        kind: GoTypeDeclKind,
        is_alias: bool,
        is_grouped: bool,
        is_top_level: bool,
        has_type_parameters: bool,
        direct_name: Option<String>,
        body_range: Option<(u32, u32)>,
        declaration_start_byte: u32,
    ) -> Self {
        Self {
            file,
            span,
            name,
            kind,
            is_alias,
            is_grouped,
            is_top_level,
            has_type_parameters,
            direct_name,
            body_range,
            declaration_start_byte,
        }
    }
}

pub fn is_synthetic_ts_js_module_function(function: &FunctionFact) -> bool {
    function.language.is_ts_family() && function.name == TS_JS_MODULE_FUNCTION_NAME
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PackageFact {
    pub id: PackageId,
    pub file: FileId,
    pub name: String,
    pub span: Span,
    pub language: Language,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ImportFact {
    pub id: ImportId,
    pub file: FileId,
    pub package: Option<String>,
    pub path: String,
    pub span: Span,
    pub language: Language,
    /// How the import is written, which decides the mode its file loads in.
    #[serde(default)]
    pub(crate) syntax: ImportSyntax,
}

/// How an import is written. A `require` loads its file in require mode.
/// Import syntax loads it in import mode where the importing file runs as an
/// ES module; elsewhere a compiler may turn it into a `require`, so it may
/// load in either mode.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub(crate) enum ImportSyntax {
    #[default]
    Import,
    Require,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BranchObligation {
    pub id: BranchId,
    pub function: Option<FunctionId>,
    pub file: FileId,
    pub decision_span: Span,
    pub condition_text: String,
    pub edge_label: String,
    pub is_error_path: bool,
    pub stable_fingerprint: String,
}

/// Facts harvested from Go test functions (`TestXxx`, benchmarks, fuzz) in `_test.go` files.
///
/// See the polint repository's `docs/facts/go-tests.md` for field semantics and harvester limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TestFact {
    pub file: FileId,
    pub function: Option<FunctionId>,
    pub name: String,
    pub span: Span,
    pub evidence_terms: Vec<String>,
    pub assertion_count: u32,
    pub subtest_count: u32,
    /// Literal string names from direct `t.Run("name", ...)` calls (first argument only).
    pub subtest_names: Vec<String>,
    pub table_rows: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CoverageFact {
    pub branch: BranchId,
    pub covered: Option<bool>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StringLiteralFact {
    pub file: FileId,
    pub value: String,
    pub span: Span,
    pub language: Language,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TsComponentFact {
    pub file: FileId,
    pub function: Option<FunctionId>,
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TsClassFact {
    pub file: FileId,
    pub name: String,
    pub span: Span,
    pub is_exported: bool,
    pub is_component_like: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct JsxAttributeFact {
    pub file: FileId,
    pub name: String,
    pub value: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedFileAnalysis {
    pub schema: String,
    pub diagnostics: Vec<Diagnostic>,
    pub facts: CachedFileFacts,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedFileFacts {
    pub packages: Vec<PackageFact>,
    pub functions: Vec<FunctionFact>,
    pub imports: Vec<ImportFact>,
    pub branches: Vec<BranchObligation>,
    pub tests: Vec<TestFact>,
    pub coverage: Vec<CoverageFact>,
    pub ts_components: Vec<TsComponentFact>,
    pub ts_classes: Vec<TsClassFact>,
    pub string_literals: Vec<StringLiteralFact>,
    pub jsx_attributes: Vec<JsxAttributeFact>,
    #[serde(default)]
    pub go_types: Vec<GoTypeDeclFact>,
}
impl FunctionFact {
    /// Constructs a syntax function fact from its complete fields.
    #[expect(
        clippy::too_many_arguments,
        reason = "This constructor mirrors the complete non-exhaustive public fact schema."
    )]
    pub fn new(
        id: FunctionId,
        file: FileId,
        name: String,
        span: Span,
        language: Language,
        is_test: bool,
        is_exported: bool,
        cyclomatic_complexity: u32,
        calls: Vec<String>,
    ) -> Self {
        Self {
            id,
            file,
            name,
            span,
            language,
            is_test,
            is_exported,
            cyclomatic_complexity,
            calls,
        }
    }
}

impl PackageFact {
    /// Constructs a syntax package fact from its complete fields.
    pub fn new(id: PackageId, file: FileId, name: String, span: Span, language: Language) -> Self {
        Self {
            id,
            file,
            name,
            span,
            language,
        }
    }
}

impl ImportFact {
    /// Constructs a syntax import fact from its complete fields.
    pub fn new(
        id: ImportId,
        file: FileId,
        package: Option<String>,
        path: String,
        span: Span,
        language: Language,
    ) -> Self {
        Self {
            id,
            file,
            package,
            path,
            span,
            language,
            syntax: ImportSyntax::Import,
        }
    }

    /// The same import, written with `syntax`.
    pub(crate) fn with_syntax(mut self, syntax: ImportSyntax) -> Self {
        self.syntax = syntax;
        self
    }
}

impl BranchObligation {
    /// Constructs a branch obligation from its complete fields.
    #[expect(
        clippy::too_many_arguments,
        reason = "This constructor mirrors the complete non-exhaustive public fact schema."
    )]
    pub fn new(
        id: BranchId,
        function: Option<FunctionId>,
        file: FileId,
        decision_span: Span,
        condition_text: String,
        edge_label: String,
        is_error_path: bool,
        stable_fingerprint: String,
    ) -> Self {
        Self {
            id,
            function,
            file,
            decision_span,
            condition_text,
            edge_label,
            is_error_path,
            stable_fingerprint,
        }
    }
}

impl TestFact {
    /// Constructs a test fact from its complete fields.
    #[expect(
        clippy::too_many_arguments,
        reason = "This constructor mirrors the complete non-exhaustive public fact schema."
    )]
    pub fn new(
        file: FileId,
        function: Option<FunctionId>,
        name: String,
        span: Span,
        evidence_terms: Vec<String>,
        assertion_count: u32,
        subtest_count: u32,
        subtest_names: Vec<String>,
        table_rows: u32,
    ) -> Self {
        Self {
            file,
            function,
            name,
            span,
            evidence_terms,
            assertion_count,
            subtest_count,
            subtest_names,
            table_rows,
        }
    }
}

impl CoverageFact {
    /// Constructs a coverage fact from its complete fields.
    pub fn new(branch: BranchId, covered: Option<bool>, source: String) -> Self {
        Self {
            branch,
            covered,
            source,
        }
    }
}

impl StringLiteralFact {
    /// Constructs a string-literal fact from its complete fields.
    pub fn new(file: FileId, value: String, span: Span, language: Language) -> Self {
        Self {
            file,
            value,
            span,
            language,
        }
    }
}

impl TsComponentFact {
    /// Constructs a TypeScript component fact from its complete fields.
    pub fn new(file: FileId, function: Option<FunctionId>, name: String, span: Span) -> Self {
        Self {
            file,
            function,
            name,
            span,
        }
    }
}

impl TsClassFact {
    /// Constructs a TypeScript class fact from its complete fields.
    pub fn new(
        file: FileId,
        name: String,
        span: Span,
        is_exported: bool,
        is_component_like: bool,
    ) -> Self {
        Self {
            file,
            name,
            span,
            is_exported,
            is_component_like,
        }
    }
}

impl JsxAttributeFact {
    /// Constructs a JSX attribute fact from its complete fields.
    pub fn new(file: FileId, name: String, value: Option<String>, span: Span) -> Self {
        Self {
            file,
            name,
            value,
            span,
        }
    }
}
