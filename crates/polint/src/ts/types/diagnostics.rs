/// Category prefix on every `polint/ts-types` diagnostic.
///
/// The category is the first thing a reader needs: whether the tier was
/// unavailable, whether a project failed to load, or whether the sidecar ran
/// out of time. Each maps to a distinct user action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TsTypesDiagnosticCategory {
    /// The TypeScript compiler, Node, or a tsconfig was not found.
    SetupMissing,
    /// A project was found but could not be loaded or type-checked.
    ProjectError,
    /// The sidecar exceeded its wall-clock budget.
    Timeout,
    /// The resolved TypeScript major version has no supported programmatic API.
    UnsupportedVersion,
}

impl TsTypesDiagnosticCategory {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SetupMissing => "TsTypesSetupMissing",
            Self::ProjectError => "TsTypesProjectError",
            Self::Timeout => "TsTypesSidecarTimeout",
            Self::UnsupportedVersion => "TsTypesUnsupportedTypeScript",
        }
    }
}
