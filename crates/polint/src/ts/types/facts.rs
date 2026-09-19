use crate::internal_core::{FileId, Span, StableKeyId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct TsTypeProjectId(pub(crate) u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct TsTypeCallableId(pub(crate) u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct TsTypeCallsiteId(pub(crate) u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct TsTypeCalleeId(pub(crate) u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct TsTypeReceiverId(pub(crate) u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct TsTypeFileDensityId(pub(crate) u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct TsTypeProjectErrorId(pub(crate) u64);

/// What kind of declaration a callable row describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum TsTypeCallableKind {
    Function,
    Method,
    Constructor,
    Arrow,
    Getter,
    Setter,
    Class,
}

/// How much the type checker could say about a call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum TsTypeCallStatus {
    /// Exactly one in-scope implementation.
    Resolved,
    /// Several in-scope implementations, all of them typed candidates.
    Union,
    /// Resolved, but to a declaration the scan does not own.
    External,
    /// The receiver is `any` or `unknown`: types cannot answer this site.
    AnyReceiver,
    /// The checker answered with a declaration only, or with nothing.
    Unresolved,
}

/// Why a callee row is a candidate for its call site.
///
/// The distinction is the tier's honesty contract. A `Declared` target is what
/// the checker resolved and is exact. An `Implementation` target is a
/// rapid-type candidate: the declared target only describes the call, and this
/// is an instantiated class whose method satisfies it. `Signature` names a
/// declaration that cannot run, and exists so a site with no runnable target is
/// distinguishable from one the sidecar never saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum TsTypeDispatch {
    Declared,
    DeclaredSignature,
    Implementation,
    UnionMember,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TsTypeProjectFact {
    pub(crate) id: TsTypeProjectId,
    pub(crate) stable_key: StableKeyId,
    pub(crate) project: String,
    pub(crate) options_digest: String,
    pub(crate) typescript_version: String,
    pub(crate) file_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TsTypeCallableFact {
    pub(crate) id: TsTypeCallableId,
    pub(crate) stable_key: StableKeyId,
    pub(crate) project: String,
    pub(crate) callable: String,
    pub(crate) name: String,
    pub(crate) kind: TsTypeCallableKind,
    pub(crate) relative_file: Option<String>,
    pub(crate) file: Option<FileId>,
    pub(crate) span: Option<Span>,
    /// Span of the declaration's own name.
    ///
    /// Declaration spans differ between the TypeScript parser and the Oxc
    /// parser that produced polint's function facts — `export function f` starts
    /// at `export` for one and at `function` for the other — but both spans
    /// contain the name, so the name anchors the join when the spans disagree.
    pub(crate) name_span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TsTypeCallsiteFact {
    pub(crate) id: TsTypeCallsiteId,
    pub(crate) stable_key: StableKeyId,
    pub(crate) project: String,
    pub(crate) callsite: String,
    pub(crate) enclosing: Option<String>,
    pub(crate) call_kind: String,
    pub(crate) status: TsTypeCallStatus,
    pub(crate) reason: Option<String>,
    pub(crate) relative_file: Option<String>,
    pub(crate) file: Option<FileId>,
    pub(crate) span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TsTypeCalleeFact {
    pub(crate) id: TsTypeCalleeId,
    pub(crate) stable_key: StableKeyId,
    pub(crate) project: String,
    pub(crate) callsite_stable_key: StableKeyId,
    pub(crate) callable: Option<String>,
    /// Moniker for a declaration outside the scan (`node_modules:…`, `lib:…`).
    pub(crate) external: Option<String>,
    pub(crate) dispatch: TsTypeDispatch,
    pub(crate) relative_file: Option<String>,
    pub(crate) file: Option<FileId>,
    pub(crate) span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TsTypeReceiverFact {
    pub(crate) id: TsTypeReceiverId,
    pub(crate) stable_key: StableKeyId,
    pub(crate) project: String,
    pub(crate) callsite_stable_key: StableKeyId,
    pub(crate) printed: String,
    pub(crate) is_any: bool,
    pub(crate) is_unknown: bool,
    pub(crate) union_size: u64,
}

/// Per-file share of call sites whose receiver type is `any` or `unknown`.
///
/// The tier never treats `any` as a typed edge, and a file where most receivers
/// are `any` is not a file whose remaining typed answers should be trusted at
/// full confidence. Counting is the sidecar's job; the thresholds are the
/// caller's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TsTypeFileDensityFact {
    pub(crate) id: TsTypeFileDensityId,
    pub(crate) stable_key: StableKeyId,
    pub(crate) project: String,
    pub(crate) relative_file: String,
    pub(crate) file: Option<FileId>,
    pub(crate) callsites: u64,
    pub(crate) any_receivers: u64,
}

impl TsTypeFileDensityFact {
    /// Share of call sites in this file whose receiver the checker could not
    /// type, in percent. Zero when the file has no call sites.
    pub(crate) fn any_percent(&self) -> u64 {
        if self.callsites == 0 {
            return 0;
        }
        self.any_receivers.saturating_mul(100) / self.callsites
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TsTypeProjectErrorFact {
    pub(crate) id: TsTypeProjectErrorId,
    pub(crate) stable_key: StableKeyId,
    pub(crate) category: String,
    pub(crate) relative_file: Option<String>,
    pub(crate) message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn density(callsites: u64, any_receivers: u64) -> TsTypeFileDensityFact {
        TsTypeFileDensityFact {
            id: TsTypeFileDensityId(0),
            stable_key: crate::internal_core::stable_key_for_test("density"),
            project: "tsconfig.json".to_string(),
            relative_file: "src/app.ts".to_string(),
            file: None,
            callsites,
            any_receivers,
        }
    }

    #[test]
    fn any_percent_is_zero_for_a_file_with_no_call_sites() {
        assert_eq!(density(0, 0).any_percent(), 0);
    }

    #[test]
    fn any_percent_truncates_rather_than_rounding_up() {
        // 1 of 3 is 33.3%: a file must not be reported as more degraded than
        // it measured.
        assert_eq!(density(3, 1).any_percent(), 33);
    }

    #[test]
    fn any_percent_reports_a_fully_untyped_file() {
        assert_eq!(density(7, 7).any_percent(), 100);
    }
}
