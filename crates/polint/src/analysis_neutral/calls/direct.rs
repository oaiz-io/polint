use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_api::FactFamily;
use crate::analysis_api::{
    FunctionFact, ReferenceFact, ReferenceKind, SymbolFact, SymbolKind, SymbolPrecision,
    SymbolResolutionStatus,
};
use crate::analysis_api::{ResolutionStatus, SemanticImportKind, SemanticStatus};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallCallee, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact,
    CallSyntaxKind, CallTargetFact, CallTargetStatus, UnresolvedCallReason,
};
use crate::analysis_neutral::ids::CallTargetId;
use crate::analysis_neutral::mir_op::UnsupportedDomain;
use crate::analysis_neutral::stable_key::semantic_stable_key;
use crate::analysis_neutral::symbol_graph::semantic::{ValueFunctionFact, ValueFunctionKind};
use crate::internal_core::{FileId, FunctionId, ReferenceId, Span, SymbolId};

pub fn resolve_direct_call_targets(
    db: &impl AnalysisHost,
    sites: &[CallSiteFact],
) -> Vec<CallTargetFact> {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let index = DirectIndex::new(db);
    let mut rows = Vec::new();

    for site in sites {
        if has_unsupported_call_evidence(db, site) {
            continue;
        }
        // A call lexically inside a `throw` argument sits on an error path the
        // demand-driven oracle does not exercise; resolving it produces false
        // edges (e.g. express `Router.use` -> `gettype` in a type-check throw).
        if site.in_throw {
            continue;
        }
        // A method call on a built-in global namespace (`Object.create()`,
        // `Array.from()`, `Promise.resolve()`, ...) is a native call; it must
        // never resolve to a same-named user function by property name. Both the
        // `reference_for_site` member-by-name path and the `lexical_function_for_site`
        // fallback would otherwise emit e.g. `Object.create()` -> local
        // `function create` (a false positive). Suppress every by-name fallback
        // for such sites — there is no sound user-function resolution for them.
        if matches!(
            site.kind,
            CallSyntaxKind::StaticMember | CallSyntaxKind::Member
        ) && receiver_is_builtin_global(db, site)
        {
            continue;
        }
        let Some(reference) = index.reference_for_site(site) else {
            if let Some(target_function) = index.lexical_function_for_site(site) {
                rows.push(CallTargetFact {
                    id: CallTargetId(0),
                    site: site.id,
                    caller: site.caller,
                    target_function: Some(target_function.id),
                    target_symbol: None,
                    synthetic_target: None,
                    edge_kind: edge_kind_for_site(site.kind),
                    algorithm: CallAlgorithm::SyntaxOnly,
                    status: CallTargetStatus::Resolved,
                    reason: None,
                    provenance: CallProvenance::MirShape,
                    precision: CallPrecision::Heuristic,
                    stable_key: lexical_target_stable_key(interner, site, target_function),
                });
            }
            continue;
        };
        if !is_precise_resolved_reference(reference) {
            continue;
        }
        let Some(target_symbol) = reference.target else {
            continue;
        };
        let Some(symbol) = index.symbols_by_id.get(&target_symbol).copied() else {
            continue;
        };
        // A TS/JS binding without one function it holds (one written after
        // its declaration, or in a file whose bindings `with` or `eval` can
        // reach) still names the call's callee, but not which value it holds
        // when the call runs: the edge to the binding may be taken, never
        // exactly, and the site stays open. So may the edge of an import
        // that is not followed to a declaration, unless the import loads a
        // package outside the analysed files: one the resolver could not
        // follow, or that names a different file for each way of loading
        // it, may reach any of those files' functions.
        let (target_function, open) = match index.function_for_symbol(symbol) {
            Some(function) => (Some(function), None),
            None if index.calls_bind_without_a_function(symbol) => {
                let open = if index.binds_a_declared_value(symbol) {
                    Some(UnresolvedCallReason::FunctionValue)
                } else if index.import_is_unfollowed(symbol) {
                    Some(UnresolvedCallReason::MissingImportResolution)
                } else {
                    None
                };
                (None, open)
            }
            None => continue,
        };
        let held = open.is_some();
        if symbol.kind == SymbolKind::Class
            && !matches!(site.kind, CallSyntaxKind::Constructor | CallSyntaxKind::New)
        {
            continue;
        }

        let algorithm = if index.is_import_binding(site, reference) {
            CallAlgorithm::ImportBinding
        } else if matches!(site.kind, CallSyntaxKind::Constructor | CallSyntaxKind::New) {
            CallAlgorithm::ConstructorBinding
        } else if matches!(site.kind, CallSyntaxKind::StaticMember) {
            CallAlgorithm::StaticMember
        } else if matches!(site.kind, CallSyntaxKind::Member) {
            CallAlgorithm::DirectMember
        } else {
            CallAlgorithm::DirectReference
        };
        rows.push(CallTargetFact {
            id: CallTargetId(0),
            site: site.id,
            caller: site.caller,
            target_function,
            target_symbol: Some(target_symbol),
            synthetic_target: None,
            edge_kind: edge_kind_for_site(site.kind),
            algorithm,
            status: if held {
                CallTargetStatus::Ambiguous
            } else {
                CallTargetStatus::Resolved
            },
            reason: open,
            provenance: CallProvenance::NativeDirect,
            precision: if held {
                CallPrecision::Conservative
            } else {
                CallPrecision::SetupAware
            },
            stable_key: target_stable_key(interner, site, algorithm, symbol),
        });
    }

    interner.sort_by_key_text(&mut rows, |row| row.stable_key, |row| row.site);
    rows.dedup_by(|left, right| left.stable_key == right.stable_key);
    for (index, row) in rows.iter_mut().enumerate() {
        row.id = CallTargetId(index as u64);
    }
    rows
}

struct DirectIndex<'db, H: AnalysisHost + ?Sized> {
    db: &'db H,
    references_by_id: BTreeMap<ReferenceId, &'db ReferenceFact>,
    symbols_by_id: BTreeMap<SymbolId, &'db SymbolFact>,
    functions_by_file_name: BTreeMap<(Option<FileId>, String), &'db FunctionFact>,
    functions_by_name: BTreeMap<String, Vec<&'db FunctionFact>>,
    /// `(file, name)` -> the file's functions with that full or short name,
    /// in source order.
    functions_by_file_callee_name: BTreeMap<(FileId, String), Vec<&'db FunctionFact>>,
    /// Binding symbol span -> the one function every call through the binding
    /// reaches, from the frontend's callee identity rows; `None` when the rows
    /// do not single one out.
    callees_by_binding: BTreeMap<SpanKey, Option<FunctionId>>,
    /// Import binding span -> the exported symbol it is linked to.
    import_links: BTreeMap<SpanKey, SymbolId>,
    /// `(file, local name)` of namespace imports, whose binding is the module
    /// object rather than any one export.
    namespace_imports: BTreeSet<(FileId, String)>,
    /// `(file, local name)` of import bindings each import of which loads a
    /// package outside the analysed files.
    external_imports: BTreeSet<(FileId, String)>,
    /// Call span -> property span of an `ns.name(...)` callee, `ns` being a
    /// namespace import, as the frontend recorded it from the AST.
    namespace_member_call_properties: BTreeMap<SpanKey, SpanKey>,
    /// Property span of such a callee -> the module-linked call reference on it.
    namespace_member_references: BTreeMap<SpanKey, &'db ReferenceFact>,
}

type SpanKey = (FileId, u32, u32);

fn span_key(span: &Span) -> SpanKey {
    (span.file, span.start_byte, span.end_byte)
}

/// Re-export chains longer than this are left unresolved. The bound also ends
/// import cycles.
const MAX_IMPORT_HOPS: usize = 8;

impl<'db, H: AnalysisHost + ?Sized> DirectIndex<'db, H> {
    fn new(db: &'db H) -> Self {
        let namespace_imports = db
            .semantic_imports()
            .iter()
            .filter(|import| {
                matches!(
                    import.kind,
                    SemanticImportKind::StaticNamespace | SemanticImportKind::EsNamespace
                )
            })
            .filter_map(|import| Some((import.file?, import.local_name.clone()?)))
            .collect::<BTreeSet<_>>();
        let external_imports = external_imports(db);
        let namespace_member_call_properties = db
            .namespace_member_calls()
            .iter()
            .map(|row| (span_key(&row.call), span_key(&row.property)))
            .collect::<BTreeMap<_, _>>();
        let namespace_properties = namespace_member_call_properties
            .values()
            .copied()
            .collect::<BTreeSet<_>>();
        let functions_by_span = db
            .functions()
            .iter()
            .map(|function| (span_key(&function.span), function.id))
            .collect::<BTreeMap<_, _>>();
        Self {
            db,
            references_by_id: db
                .references()
                .iter()
                .map(|reference| (reference.id, reference))
                .collect(),
            symbols_by_id: db
                .symbols()
                .iter()
                .map(|symbol| (symbol.id, symbol))
                .collect(),
            functions_by_file_name: db
                .functions()
                .iter()
                .map(|function| ((Some(function.file), function.name.clone()), function))
                .collect(),
            functions_by_name: functions_by_name(db.functions()),
            functions_by_file_callee_name: functions_by_file_callee_name(db.functions()),
            callees_by_binding: callees_by_binding(db.value_functions(), &functions_by_span),
            import_links: import_links(db.references()),
            namespace_imports,
            external_imports,
            namespace_member_references: namespace_member_references(
                db.references(),
                &namespace_properties,
            ),
            namespace_member_call_properties,
        }
    }

    fn reference_for_site(&self, site: &CallSiteFact) -> Option<&'db ReferenceFact> {
        match &site.callee {
            CallCallee::Identifier { reference, name }
            | CallCallee::Constructor {
                reference,
                name: Some(name),
            } => reference
                .and_then(|id| self.references_by_id.get(&id).copied())
                .or_else(|| self.unique_reference_by_site_name(site, name)),
            CallCallee::Member { property, .. }
                if matches!(
                    site.kind,
                    CallSyntaxKind::StaticMember | CallSyntaxKind::Member
                ) =>
            {
                if let Some(property_span) = self
                    .namespace_member_call_properties
                    .get(&span_key(&site.span))
                {
                    return self
                        .namespace_member_references
                        .get(property_span)
                        .copied()
                        .filter(|reference| reference.name == *property);
                }
                // The TS/JS symbol graph puts references on member properties
                // only for namespace member calls, so any other same-named
                // reference near a member site belongs to a different callee.
                if site.language.is_ts_family() {
                    return None;
                }
                self.unique_reference_by_site_name(site, property)
            }
            _ => None,
        }
    }

    fn unique_reference_by_site_name(
        &self,
        site: &CallSiteFact,
        name: &str,
    ) -> Option<&'db ReferenceFact> {
        let mut matches = self
            .db
            .references_for_file(site.file)
            .into_iter()
            .filter(|reference| reference.name == name)
            .filter(|reference| {
                reference.primary_span.as_ref().is_some_and(|span| {
                    spans_overlap_or_touch(span, &site.span)
                        && !self
                            .namespace_member_references
                            .contains_key(&span_key(span))
                })
            });
        let first = matches.next()?;
        if matches.next().is_none() {
            Some(first)
        } else {
            None
        }
    }

    fn is_import_binding(&self, site: &CallSiteFact, reference: &ReferenceFact) -> bool {
        matches!(reference.precision, SymbolPrecision::ModuleLinked)
            || reference
                .target
                .and_then(|target| self.symbols_by_id.get(&target))
                .is_some_and(|symbol| symbol.kind == SymbolKind::Import)
            || self.semantic_import_matches(site, reference)
    }

    fn semantic_import_matches(&self, site: &CallSiteFact, reference: &ReferenceFact) -> bool {
        self.db.semantic_imports().iter().any(|import| {
            import.file == Some(site.file)
                && import.status == SemanticStatus::Resolved
                && (import.local_name.as_deref() == Some(reference.name.as_str())
                    || import.imported_name.as_deref() == Some(reference.name.as_str()))
        })
    }

    /// The function a call through `symbol` reaches. Import bindings follow
    /// their module link (through re-exports) to the exported declaration;
    /// an import that cannot be followed to one function has no target.
    fn function_for_symbol(&self, symbol: &'db SymbolFact) -> Option<FunctionId> {
        self.declared_symbol_for(symbol)
            .and_then(|declared| self.declared_function_for_symbol(declared))
    }

    /// The declaration `symbol` names: itself unless it is an import binding,
    /// else the declaration its module link (through re-exports) ends at.
    /// `None` when the chain passes a namespace import, leaves the linked
    /// files, cycles or runs past the hop limit.
    fn declared_symbol_for(&self, symbol: &'db SymbolFact) -> Option<&'db SymbolFact> {
        match self.import_chain_end(symbol) {
            ImportChainEnd::Declared(declared) => Some(declared),
            ImportChainEnd::Import(_) | ImportChainEnd::Lost => None,
        }
    }

    /// Where the module links from `symbol` end: at a declaration, at an
    /// import binding with no link to follow, or nowhere, where a link
    /// leaves the symbol facts, the chain cycles or it runs past the hop
    /// limit.
    fn import_chain_end(&self, symbol: &'db SymbolFact) -> ImportChainEnd<'db> {
        let mut current = symbol;
        for _ in 0..MAX_IMPORT_HOPS {
            if current.kind != SymbolKind::Import {
                return ImportChainEnd::Declared(current);
            }
            let Some(file) = current.file else {
                return ImportChainEnd::Lost;
            };
            if self
                .namespace_imports
                .contains(&(file, current.name.clone()))
            {
                return ImportChainEnd::Import(current);
            }
            let Some(span) = current.primary_span.as_ref() else {
                return ImportChainEnd::Lost;
            };
            let Some(target) = self.import_links.get(&span_key(span)) else {
                return ImportChainEnd::Import(current);
            };
            let Some(next) = self.symbols_by_id.get(target).copied() else {
                return ImportChainEnd::Lost;
            };
            current = next;
        }
        ImportChainEnd::Lost
    }

    /// Whether a call through `symbol` passes an import binding the module
    /// links do not follow to a declaration, where the import does not load
    /// a package outside the analysed files. Such an import may load files
    /// of the analysed program, so the binding does not answer for the call.
    fn import_is_unfollowed(&self, symbol: &'db SymbolFact) -> bool {
        match self.import_chain_end(symbol) {
            ImportChainEnd::Declared(_) => false,
            ImportChainEnd::Import(import) => !import
                .file
                .is_some_and(|file| self.external_imports.contains(&(file, import.name.clone()))),
            ImportChainEnd::Lost => true,
        }
    }

    /// Whether a call through `symbol` that reaches no one function still
    /// resolves to the binding. An import answers for its declaration: one
    /// linked to a variable without a single function (a `let` reassigned
    /// elsewhere) resolves no more than a call through that variable would.
    fn calls_bind_without_a_function(&self, symbol: &'db SymbolFact) -> bool {
        is_direct_callable_symbol(symbol)
            && self
                .declared_symbol_for(symbol)
                .is_none_or(is_direct_callable_symbol)
    }

    /// Whether a call through `symbol` reads a TS/JS binding declared in the
    /// analysed files, whose value the frontend's rows decide, rather than an
    /// import that leaves them.
    fn binds_a_declared_value(&self, symbol: &'db SymbolFact) -> bool {
        self.declared_symbol_for(symbol).is_some_and(|declared| {
            declared.language.is_ts_family()
                && !matches!(declared.kind, SymbolKind::Import | SymbolKind::Method)
        })
    }

    /// The function a call through a declared (non-import) symbol reaches.
    ///
    /// A TS/JS binding reaches only what the frontend's callee identity rows
    /// for its own Oxc symbol name: whatever its kind (a function, a class, a
    /// variable, an interface merged with a function), a binding without a
    /// single row target has none. Methods, and every Go symbol, resolve to
    /// the innermost same-named function around the declared name.
    fn declared_function_for_symbol(&self, symbol: &SymbolFact) -> Option<FunctionId> {
        if symbol.language.is_ts_family() && symbol.kind != SymbolKind::Method {
            return self
                .callees_by_binding
                .get(&span_key(symbol.primary_span.as_ref()?))
                .copied()
                .flatten();
        }
        match symbol.kind {
            SymbolKind::Function | SymbolKind::Method | SymbolKind::Class => self
                .function_declaring_symbol(symbol)
                .or_else(|| self.function_named_like_symbol(symbol)),
            _ => None,
        }
    }

    /// The innermost same-named function whose span holds the declared name,
    /// so a nested function never stands in for a same-named outer one.
    fn function_declaring_symbol(&self, symbol: &SymbolFact) -> Option<FunctionId> {
        let name_span = symbol.primary_span.as_ref()?;
        self.functions_by_file_callee_name
            .get(&(symbol.file?, symbol.name.clone()))?
            .iter()
            .filter(|function| {
                function.span.start_byte <= name_span.start_byte
                    && name_span.end_byte <= function.span.end_byte
            })
            .min_by_key(|function| function.span.end_byte - function.span.start_byte)
            .map(|function| function.id)
    }

    fn function_named_like_symbol(&self, symbol: &SymbolFact) -> Option<FunctionId> {
        self.functions_by_file_name
            .get(&(symbol.file, symbol.qualified_name.clone()))
            .or_else(|| {
                self.functions_by_file_name
                    .get(&(symbol.file, symbol.name.clone()))
            })
            .map(|function| function.id)
    }

    fn lexical_function_for_site(&self, site: &CallSiteFact) -> Option<&'db FunctionFact> {
        let callee_name = lexical_callee_name(site)?;
        self.unique_function_in_file(site.file, callee_name)
            .or_else(|| self.unique_function_by_name(callee_name))
    }

    fn unique_function_in_file(
        &self,
        file: FileId,
        callee_name: &str,
    ) -> Option<&'db FunctionFact> {
        match self
            .functions_by_file_callee_name
            .get(&(file, callee_name.to_owned()))?
            .as_slice()
        {
            [function] => Some(function),
            _ => None,
        }
    }

    fn unique_function_by_name(&self, callee_name: &str) -> Option<&'db FunctionFact> {
        let matches = self.functions_by_name.get(callee_name)?;
        if matches.len() == 1 {
            matches.first().copied()
        } else {
            None
        }
    }
}

/// Built-in global namespace objects whose member calls are native and must
/// never bind to a same-named user function. Receivers like `Object`/`Array`
/// are read straight from the call-site source (the member callee carries no
/// base identifier in `CallCallee::Member`).
const BUILTIN_GLOBAL_RECEIVERS: &[&str] = &[
    "Object",
    "Array",
    "Math",
    "JSON",
    "Number",
    "String",
    "Boolean",
    "Symbol",
    "Reflect",
    "Promise",
    "Date",
    "RegExp",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "Proxy",
    "BigInt",
    "Function",
    "Error",
    "Buffer",
    "Intl",
    "console",
    "globalThis",
];

/// Is the receiver of this member call a built-in global namespace? Reads the
/// leading identifier of the call-site source slice (the part before the first
/// `.`), which for a member call is the receiver expression.
fn receiver_is_builtin_global(db: &impl AnalysisHost, site: &CallSiteFact) -> bool {
    let Some(file) = db.files().iter().find(|file| file.id == site.file) else {
        return false;
    };
    let start = site.span.start_byte as usize;
    let end = site.span.end_byte as usize;
    let Some(slice) = file.source.get(start..end) else {
        return false;
    };
    let receiver: String = slice
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '$')
        .collect();
    BUILTIN_GLOBAL_RECEIVERS.contains(&receiver.as_str())
}

fn functions_by_file_callee_name(
    functions: &[FunctionFact],
) -> BTreeMap<(FileId, String), Vec<&FunctionFact>> {
    let mut by_name: BTreeMap<(FileId, String), Vec<&FunctionFact>> = BTreeMap::new();
    for function in functions {
        by_name
            .entry((function.file, function.name.clone()))
            .or_default()
            .push(function);
        if let Some(short) = short_function_name(&function.name) {
            by_name
                .entry((function.file, short.to_owned()))
                .or_default()
                .push(function);
        }
    }
    for functions in by_name.values_mut() {
        functions.sort_by_key(|function| (function.span.start_byte, function.span.end_byte));
        functions.dedup_by_key(|function| function.id);
    }
    by_name
}

/// The one function each binding reaches. A literal binding holds its
/// literal. A function symbol reaches its one implementation or, when it has
/// none, its one signature (an ambient declaration). More than one candidate,
/// a literal mixed with declarations, or a row whose function has no function
/// fact leaves the binding without a callee.
fn callees_by_binding(
    rows: &[ValueFunctionFact],
    functions_by_span: &BTreeMap<SpanKey, FunctionId>,
) -> BTreeMap<SpanKey, Option<FunctionId>> {
    let mut candidates: BTreeMap<SpanKey, Vec<(ValueFunctionKind, Option<FunctionId>)>> =
        BTreeMap::new();
    for row in rows {
        candidates.entry(span_key(&row.binding)).or_default().push((
            row.kind,
            functions_by_span.get(&span_key(&row.function)).copied(),
        ));
    }
    candidates
        .into_iter()
        .map(|(binding, candidates)| {
            let of_kind = |kind| {
                candidates
                    .iter()
                    .filter(|(candidate, _)| *candidate == kind)
                    .map(|(_, function)| *function)
                    .collect::<Vec<_>>()
            };
            let callee = match (
                of_kind(ValueFunctionKind::Literal).as_slice(),
                of_kind(ValueFunctionKind::Implementation).as_slice(),
                of_kind(ValueFunctionKind::Signature).as_slice(),
            ) {
                ([literal], [], []) => *literal,
                ([], [implementation], _) => *implementation,
                ([], [], [signature]) => *signature,
                _ => None,
            };
            (binding, callee)
        })
        .collect()
}

/// Module-linked call references on the property of a recorded namespace
/// member callee.
fn namespace_member_references<'db>(
    references: &'db [ReferenceFact],
    namespace_properties: &BTreeSet<SpanKey>,
) -> BTreeMap<SpanKey, &'db ReferenceFact> {
    references
        .iter()
        .filter(|reference| {
            reference.kind == ReferenceKind::Call
                && matches!(reference.precision, SymbolPrecision::ModuleLinked)
        })
        .filter_map(|reference| {
            let span = span_key(reference.primary_span.as_ref()?);
            namespace_properties
                .contains(&span)
                .then_some((span, reference))
        })
        .collect()
}

/// Where a chain of module links from a symbol ends.
enum ImportChainEnd<'db> {
    Declared(&'db SymbolFact),
    Import(&'db SymbolFact),
    Lost,
}

/// `(file, local name)` of the import bindings every import of which loads a
/// package outside the analysed files.
fn external_imports(db: &(impl AnalysisHost + ?Sized)) -> BTreeSet<(FileId, String)> {
    let resolutions = db
        .resolved_imports()
        .iter()
        .map(|row| (row.import, row.status))
        .collect::<BTreeMap<_, _>>();
    let mut ts_external = BTreeMap::new();
    for import in db
        .imports()
        .iter()
        .filter(|import| import.language.is_ts_family())
    {
        let confirmed = resolutions.get(&import.id) == Some(&ResolutionStatus::External);
        *ts_external
            .entry((import.file, import.path.as_str()))
            .or_insert(true) &= confirmed;
    }
    let mut external = BTreeMap::<(FileId, String), bool>::new();
    for import in db.semantic_imports() {
        let (Some(file), Some(name)) = (import.file, import.local_name.clone()) else {
            continue;
        };
        let confirmed = if import.language.is_ts_family() {
            ts_external
                .get(&(file, import.import_path.as_str()))
                .copied()
                .unwrap_or(false)
        } else {
            import.status == SemanticStatus::External
        };
        *external.entry((file, name)).or_insert(true) &= confirmed;
    }
    external
        .into_iter()
        .filter_map(|(binding, external)| external.then_some(binding))
        .collect()
}

/// Import bindings whose module link names exactly one exported symbol. A
/// binding with links to different symbols is dropped.
fn import_links(references: &[ReferenceFact]) -> BTreeMap<SpanKey, SymbolId> {
    let mut links = BTreeMap::<SpanKey, Option<SymbolId>>::new();
    for reference in references {
        if reference.kind != ReferenceKind::Import
            || reference.precision != SymbolPrecision::ModuleLinked
            || !is_precise_resolved_reference(reference)
        {
            continue;
        }
        let (Some(span), Some(target)) = (reference.primary_span.as_ref(), reference.target) else {
            continue;
        };
        links
            .entry(span_key(span))
            .and_modify(|linked| {
                if *linked != Some(target) {
                    *linked = None;
                }
            })
            .or_insert(Some(target));
    }
    links
        .into_iter()
        .filter_map(|(binding, target)| Some((binding, target?)))
        .collect()
}

fn functions_by_name(functions: &[FunctionFact]) -> BTreeMap<String, Vec<&FunctionFact>> {
    let mut by_name: BTreeMap<String, Vec<&FunctionFact>> = BTreeMap::new();
    for function in functions {
        by_name
            .entry(function.name.clone())
            .or_default()
            .push(function);
        if let Some(short) = short_function_name(&function.name) {
            by_name.entry(short.to_string()).or_default().push(function);
        }
    }
    for functions in by_name.values_mut() {
        functions.sort_by_key(|function| (function.file.0, function.span.start_byte));
        functions.dedup_by_key(|function| function.id);
    }
    by_name
}

fn lexical_callee_name(site: &CallSiteFact) -> Option<&str> {
    match &site.callee {
        CallCallee::Identifier { name, .. } => Some(name.as_str()),
        CallCallee::Constructor {
            name: Some(name), ..
        } => Some(name.as_str()),
        CallCallee::Member { property, .. } if site.kind == CallSyntaxKind::StaticMember => {
            Some(property.as_str())
        }
        _ => None,
    }
}

fn short_function_name(function_name: &str) -> Option<&str> {
    function_name
        .rsplit_once('.')
        .map(|(_, short)| short)
        .filter(|short| !short.is_empty())
}

fn has_unsupported_call_evidence(db: &impl AnalysisHost, site: &CallSiteFact) -> bool {
    db.unsupported_semantics().iter().any(|row| {
        row.file == site.file
            && row.affected_domains.contains(&UnsupportedDomain::Calls)
            && spans_overlap_or_touch(&row.span, &site.span)
            && unsupported_call_construct_blocks_direct_target(&row.construct, &row.source_evidence)
    })
}

fn unsupported_call_construct_blocks_direct_target(construct: &str, evidence: &str) -> bool {
    let construct = construct.to_ascii_lowercase();
    let evidence = evidence.to_ascii_lowercase();
    let labels = [construct.as_str(), evidence.as_str()];
    labels.iter().any(|label| {
        label.contains("reflect")
            || label.contains("go_statement")
            || label.contains("goroutine")
            || label.contains("eval")
            || label.contains("dynamic import")
            || label.contains("import(")
            || label.contains("call/apply/bind")
            || label.contains(".call")
            || label.contains(".apply")
            || label.contains(".bind")
    })
}

fn is_direct_callable_symbol(symbol: &SymbolFact) -> bool {
    matches!(
        symbol.kind,
        SymbolKind::Function | SymbolKind::Method | SymbolKind::Class | SymbolKind::Import
    )
}

fn is_precise_resolved_reference(reference: &ReferenceFact) -> bool {
    reference.status == SymbolResolutionStatus::Resolved
        && reference.target.is_some()
        && reference.target.is_none_or(|target| {
            reference
                .candidates
                .iter()
                .all(|candidate| *candidate == target)
        })
        && matches!(
            reference.precision,
            SymbolPrecision::ExactSemantic
                | SymbolPrecision::ExactLocal
                | SymbolPrecision::ModuleLinked
        )
}

fn edge_kind_for_site(kind: CallSyntaxKind) -> CallEdgeKind {
    match kind {
        CallSyntaxKind::Constructor | CallSyntaxKind::New => CallEdgeKind::Constructor,
        CallSyntaxKind::StaticMember => CallEdgeKind::StaticMember,
        CallSyntaxKind::Method | CallSyntaxKind::Member => CallEdgeKind::MethodDirect,
        _ => CallEdgeKind::Direct,
    }
}

fn spans_overlap_or_touch(left: &Span, right: &Span) -> bool {
    left.file == right.file
        && left.start_byte <= right.end_byte
        && right.start_byte <= left.end_byte
}

fn target_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    site: &CallSiteFact,
    algorithm: CallAlgorithm,
    symbol: &SymbolFact,
) -> crate::internal_core::StableKeyId {
    interner.intern(
        semantic_stable_key(
            FactFamily::CallTarget,
            &[
                ("site", interner.resolve(site.stable_key).to_string()),
                ("algorithm", format!("{algorithm:?}")),
                ("target", interner.resolve(symbol.stable_key).to_string()),
                ("provider", "polint.calls".to_string()),
                ("schema", "calls-facts-1:1".to_string()),
                ("model", "absent".to_string()),
            ],
        )
        .into_string(),
    )
}

fn lexical_target_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    site: &CallSiteFact,
    function: &FunctionFact,
) -> crate::internal_core::StableKeyId {
    interner.intern(
        semantic_stable_key(
            FactFamily::CallTarget,
            &[
                ("site", interner.resolve(site.stable_key).to_string()),
                ("algorithm", format!("{:?}", CallAlgorithm::SyntaxOnly)),
                (
                    "target",
                    format!(
                        "{}:{}:{}:{}:{}",
                        function.name,
                        function.file.0,
                        function.span.start_line,
                        function.span.start_col,
                        function.span.start_byte
                    ),
                ),
                ("provider", "polint.calls".to_string()),
                ("schema", "calls-facts-1:1".to_string()),
                ("model", "absent".to_string()),
            ],
        )
        .into_string(),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::resolve_direct_call_targets;
    use crate::analysis_api::{
        DefinitionFact, DefinitionKind, FunctionFact, ReferenceFact, ReferenceKind, SymbolFact,
        SymbolKind, SymbolNamespace, SymbolPrecision, SymbolResolutionStatus,
    };
    use crate::analysis_api::{
        SemanticImportFact, SemanticImportId, SemanticImportKind, SemanticStatus,
    };
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallAlgorithm, CallCallee, CallEdgeKind, CallPrecision, CallSiteFact, CallSyntaxKind,
        CallTargetStatus,
    };
    use crate::analysis_neutral::ids::{CallSiteId, MirBodyId, MirOpId};
    use crate::analysis_neutral::symbol_graph::semantic::{ValueFunctionFact, ValueFunctionKind};
    use crate::internal_core::{
        DefinitionId, FileId, FunctionId, Language, ReferenceId, Span, SymbolId,
    };

    #[test]
    fn lexical_function_call_with_resolved_reference_emits_direct_target() {
        let mut fixture = Fixture::new(Language::TypeScript, "src/caller.ts");
        let target_function = fixture.add_function("target", 10);
        let target_symbol = fixture.add_symbol("target", target_function, 10);
        let reference =
            fixture.add_reference("target", target_symbol, 3, SymbolPrecision::ExactLocal);
        fixture.store_symbols();

        let site = fixture.site(
            1,
            CallSyntaxKind::Function,
            CallCallee::Identifier {
                reference: Some(reference),
                name: "target".to_string(),
            },
            3,
        );
        let targets = resolve_direct_call_targets(&fixture.db, &[site]);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].algorithm, CallAlgorithm::DirectReference);
        assert_eq!(targets[0].status, CallTargetStatus::Resolved);
        assert_eq!(targets[0].target_symbol, Some(target_symbol));
        assert_eq!(targets[0].target_function, Some(target_function));
        assert_eq!(targets[0].edge_kind, CallEdgeKind::Direct);
    }

    #[test]
    fn lexical_function_call_without_reference_uses_unique_function_name_fallback() {
        let mut fixture = Fixture::new(Language::TypeScript, "src/caller.ts");
        let target_function = fixture.add_function("target", 10);

        let site = fixture.site(
            1,
            CallSyntaxKind::Function,
            CallCallee::Identifier {
                reference: None,
                name: "target".to_string(),
            },
            3,
        );
        let targets = resolve_direct_call_targets(&fixture.db, &[site]);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].algorithm, CallAlgorithm::SyntaxOnly);
        assert_eq!(targets[0].status, CallTargetStatus::Resolved);
        assert_eq!(targets[0].target_symbol, None);
        assert_eq!(targets[0].target_function, Some(target_function));
        assert_eq!(targets[0].precision, CallPrecision::Heuristic);
    }

    #[test]
    fn unresolved_dynamic_member_call_does_not_use_name_only_fallback() {
        let mut fixture = Fixture::new(Language::TypeScript, "src/caller.ts");
        fixture.add_function("Service.run", 10);

        let site = fixture.site(
            1,
            CallSyntaxKind::Member,
            CallCallee::Member {
                base: crate::analysis_neutral::ids::PlaceId(1),
                property: "run".to_string(),
            },
            3,
        );
        let targets = resolve_direct_call_targets(&fixture.db, &[site]);

        assert!(targets.is_empty());
    }

    #[test]
    fn lexical_static_member_call_without_reference_uses_unique_method_name_fallback() {
        let mut fixture = Fixture::new(Language::TypeScript, "src/caller.ts");
        let target_function = fixture.add_function("Service.run", 10);

        let site = fixture.site(
            1,
            CallSyntaxKind::StaticMember,
            CallCallee::Member {
                base: crate::analysis_neutral::ids::PlaceId(1),
                property: "run".to_string(),
            },
            3,
        );
        let targets = resolve_direct_call_targets(&fixture.db, &[site]);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].algorithm, CallAlgorithm::SyntaxOnly);
        assert_eq!(targets[0].edge_kind, CallEdgeKind::StaticMember);
        assert_eq!(targets[0].target_function, Some(target_function));
    }

    #[test]
    fn imported_function_call_uses_import_binding_algorithm() {
        let mut fixture = Fixture::new(Language::TypeScript, "src/caller.ts");
        let target_function = fixture.add_function("makeThing", 20);
        let target_symbol = fixture.add_symbol("makeThing", target_function, 20);
        let reference =
            fixture.add_reference("makeThing", target_symbol, 4, SymbolPrecision::ModuleLinked);
        fixture.semantic_imports.push(SemanticImportFact {
            id: SemanticImportId(0),
            language: Language::TypeScript,
            file: Some(fixture.file),
            package: None,
            module: None,
            scope: None,
            import_path: "./factory".to_string(),
            local_name: Some("makeThing".to_string()),
            imported_name: Some("makeThing".to_string()),
            namespace: SymbolNamespace::Value,
            kind: SemanticImportKind::StaticNamed,
            stable_key: fixture
                .db
                .stable_key_interner()
                .intern("semantic-import:makeThing"),
            status: SemanticStatus::Resolved,
        });
        fixture.store_symbols_and_semantic_imports();

        let site = fixture.site(
            2,
            CallSyntaxKind::Function,
            CallCallee::Identifier {
                reference: Some(reference),
                name: "makeThing".to_string(),
            },
            4,
        );
        let targets = resolve_direct_call_targets(&fixture.db, &[site]);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].algorithm, CallAlgorithm::ImportBinding);
        assert_eq!(targets[0].status, CallTargetStatus::Resolved);
        assert_eq!(targets[0].target_symbol, Some(target_symbol));
        assert_eq!(targets[0].target_function, Some(target_function));
    }

    #[test]
    fn static_member_requires_precise_semantic_reference() {
        let mut fixture = Fixture::new(Language::TypeScript, "src/caller.ts");
        let target_function = fixture.add_function("Service.run", 30);
        let target_symbol = fixture.add_symbol("Service.run", target_function, 30);
        let reference =
            fixture.add_reference("run", target_symbol, 5, SymbolPrecision::ExactSemantic);
        fixture.store_symbols();

        let resolved_site = fixture.site(
            3,
            CallSyntaxKind::StaticMember,
            CallCallee::Identifier {
                reference: Some(reference),
                name: "run".to_string(),
            },
            5,
        );
        let dynamic_member = fixture.site(
            4,
            CallSyntaxKind::Member,
            CallCallee::Member {
                base: crate::analysis_neutral::ids::PlaceId(7),
                property: "run".to_string(),
            },
            6,
        );
        let targets = resolve_direct_call_targets(&fixture.db, &[resolved_site, dynamic_member]);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].algorithm, CallAlgorithm::StaticMember);
        assert_eq!(targets[0].edge_kind, CallEdgeKind::StaticMember);
        assert_eq!(targets[0].target_symbol, Some(target_symbol));
    }

    #[test]
    fn constructor_call_uses_constructor_binding_algorithm() {
        let mut fixture = Fixture::new(Language::TypeScript, "src/caller.ts");
        let target_function = fixture.add_function("Formatter", 40);
        let target_symbol =
            fixture.add_symbol_with_kind("Formatter", target_function, 40, SymbolKind::Class);
        let reference = fixture.add_reference(
            "Formatter",
            target_symbol,
            7,
            SymbolPrecision::ExactSemantic,
        );
        fixture.store_symbols();

        let site = fixture.site(
            5,
            CallSyntaxKind::New,
            CallCallee::Constructor {
                reference: Some(reference),
                name: Some("Formatter".to_string()),
            },
            7,
        );
        let targets = resolve_direct_call_targets(&fixture.db, &[site]);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].algorithm, CallAlgorithm::ConstructorBinding);
        assert_eq!(targets[0].edge_kind, CallEdgeKind::Constructor);
        assert_eq!(targets[0].target_symbol, Some(target_symbol));
        assert_eq!(targets[0].target_function, Some(target_function));
    }

    #[test]
    fn instance_member_call_uses_direct_member_algorithm() {
        let mut fixture = Fixture::new(Language::Go, "caller.go");
        let target_function = fixture.add_function("Formatter.render", 50);
        let target_symbol = fixture.add_symbol_with_kind(
            "Formatter.render",
            target_function,
            50,
            SymbolKind::Method,
        );
        fixture.add_reference("render", target_symbol, 8, SymbolPrecision::ExactSemantic);
        fixture.store_symbols();

        let site = fixture.site(
            6,
            CallSyntaxKind::Member,
            CallCallee::Member {
                base: crate::analysis_neutral::ids::PlaceId(8),
                property: "render".to_string(),
            },
            8,
        );
        let targets = resolve_direct_call_targets(&fixture.db, &[site]);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].algorithm, CallAlgorithm::DirectMember);
        assert_eq!(targets[0].edge_kind, CallEdgeKind::MethodDirect);
        assert_eq!(targets[0].target_symbol, Some(target_symbol));
        assert_eq!(targets[0].target_function, Some(target_function));
    }

    /// The TS/JS symbol graph records no reference on a member property outside
    /// namespace member calls, so a same-named reference touching a member site
    /// belongs to another callee (an argument, the receiver) and is not used.
    #[test]
    fn ts_member_call_ignores_a_same_named_reference_near_the_site() {
        let mut fixture = Fixture::new(Language::TypeScript, "src/caller.ts");
        let target_function = fixture.add_function("render", 50);
        let target_symbol = fixture.add_symbol("render", target_function, 50);
        fixture.add_reference("render", target_symbol, 8, SymbolPrecision::ExactLocal);
        fixture.store_symbols();

        let site = fixture.site(
            6,
            CallSyntaxKind::Member,
            CallCallee::Member {
                base: crate::analysis_neutral::ids::PlaceId(8),
                property: "render".to_string(),
            },
            8,
        );

        assert!(resolve_direct_call_targets(&fixture.db, &[site]).is_empty());
    }

    struct Fixture {
        db: LocalAnalysisDb,
        language: Language,
        file: FileId,
        caller: FunctionId,
        symbols: Vec<SymbolFact>,
        value_functions: Vec<ValueFunctionFact>,
        definitions: Vec<DefinitionFact>,
        references: Vec<ReferenceFact>,
        semantic_imports: Vec<SemanticImportFact>,
    }

    impl Fixture {
        fn new(language: Language, path: &str) -> Self {
            let mut db = LocalAnalysisDb::new();
            let file = db.add_source_file(
                PathBuf::from(path),
                path.to_string(),
                language,
                "".into(),
                "content".to_string(),
            );
            let caller = db.push_function(function(file, language, "caller", 1));
            Self {
                db,
                language,
                file,
                caller,
                symbols: Vec::new(),
                value_functions: Vec::new(),
                definitions: Vec::new(),
                references: Vec::new(),
                semantic_imports: Vec::new(),
            }
        }

        fn add_function(&mut self, name: &str, line: u32) -> FunctionId {
            self.db
                .push_function(function(self.file, self.language, name, line))
        }

        fn add_symbol(&mut self, name: &str, function: FunctionId, line: u32) -> SymbolId {
            self.add_symbol_with_kind(name, function, line, SymbolKind::Function)
        }

        fn add_symbol_with_kind(
            &mut self,
            name: &str,
            function: FunctionId,
            line: u32,
            kind: SymbolKind,
        ) -> SymbolId {
            let id = SymbolId::from_raw(self.symbols.len() as u64);
            self.symbols.push(SymbolFact::new(
                id,
                self.language,
                name.to_string(),
                name.to_string(),
                kind,
                SymbolNamespace::Value,
                Some(self.file),
                None,
                None,
                None,
                Some(span(self.file, line)),
                true,
                self.db
                    .stable_key_interner()
                    .intern(format!("symbol:{name}")),
                SymbolPrecision::ExactSemantic,
            ));
            self.definitions.push(DefinitionFact::new(
                DefinitionId::from_raw(id.0),
                id,
                self.language,
                name.to_string(),
                name.to_string(),
                DefinitionKind::Definition,
                SymbolNamespace::Value,
                Some(self.file),
                None,
                None,
                None,
                Some(span(self.file, line)),
                true,
                true,
                self.db
                    .stable_key_interner()
                    .intern(format!("definition:{name}")),
                SymbolPrecision::ExactSemantic,
            ));
            assert_eq!(function.0, id.0 + 1);
            // The TS/JS frontend names a function or class declaration's
            // implementation as the callee of its binding.
            if self.language.is_ts_family()
                && matches!(kind, SymbolKind::Function | SymbolKind::Class)
            {
                self.value_functions.push(ValueFunctionFact {
                    binding: span(self.file, line),
                    function: span(self.file, line),
                    kind: ValueFunctionKind::Implementation,
                });
            }
            id
        }

        fn add_reference(
            &mut self,
            name: &str,
            target: SymbolId,
            line: u32,
            precision: SymbolPrecision,
        ) -> ReferenceId {
            let id = ReferenceId::from_raw(self.references.len() as u64);
            self.references.push(ReferenceFact::new(
                id,
                self.language,
                name.to_string(),
                name.to_string(),
                ReferenceKind::Call,
                SymbolNamespace::Value,
                Some(self.file),
                None,
                None,
                None,
                Some(span(self.file, line)),
                Some(target),
                vec![target],
                self.db
                    .stable_key_interner()
                    .intern(format!("reference:{name}:{}", id.0)),
                SymbolResolutionStatus::Resolved,
                precision,
            ));
            id
        }

        fn site(
            &self,
            id: u64,
            kind: CallSyntaxKind,
            callee: CallCallee,
            line: u32,
        ) -> CallSiteFact {
            CallSiteFact {
                in_throw: false,
                id: CallSiteId(id),
                language: self.language,
                file: self.file,
                caller: self.caller,
                owner_symbol: None,
                body: MirBodyId(0),
                operation: MirOpId(id),
                span: span(self.file, line),
                kind,
                callee,
                receiver: None,
                arguments: Vec::new(),
                result: None,
                status: CallTargetStatus::Unresolved,
                precision: CallPrecision::Conservative,
                stable_key: self
                    .db
                    .stable_key_interner()
                    .intern(format!("call-site:{id}")),
            }
        }

        fn store_symbols(&mut self) {
            self.db.replace_symbol_graph_facts(
                self.symbols.clone(),
                self.definitions.clone(),
                self.references.clone(),
            );
            self.db
                .replace_value_functions(self.value_functions.clone());
        }

        fn store_symbols_and_semantic_imports(&mut self) {
            self.store_symbols();
            self.db
                .replace_semantic_imports(self.semantic_imports.clone());
        }
    }

    fn function(file: FileId, language: Language, name: &str, line: u32) -> FunctionFact {
        FunctionFact::new(
            FunctionId::from_raw(999),
            file,
            name.to_string(),
            span(file, line),
            language,
            false,
            true,
            1,
            Vec::new(),
        )
    }

    fn span(file: FileId, line: u32) -> Span {
        Span::new(file, line * 10, line * 10 + 5, line, 1, line, 6)
    }
}

#[cfg(test)]
mod non_direct_cases {
    use crate::analysis_neutral::AnalysisHost;
    use std::path::PathBuf;

    use super::resolve_direct_call_targets;
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallCallee, CallPrecision, CallSiteFact, CallSyntaxKind, CallTargetStatus,
        UnresolvedCallReason,
    };
    use crate::analysis_neutral::ids::{CallSiteId, MirBodyId, MirOpId, PlaceId};
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    #[test]
    fn go_function_value_and_interface_shapes_do_not_emit_direct_targets() {
        let (db, file, caller) = db_with_function(Language::Go, "flow.go");
        let sites = vec![
            site(
                &db,
                Language::Go,
                file,
                caller,
                1,
                CallCallee::FunctionValue { place: PlaceId(10) },
                CallSyntaxKind::FunctionValue,
            ),
            site(
                &db,
                Language::Go,
                file,
                caller,
                2,
                CallCallee::Unknown {
                    reason: UnresolvedCallReason::InterfaceDispatch,
                },
                CallSyntaxKind::Method,
            ),
        ];

        assert!(resolve_direct_call_targets(&db, &sites).is_empty());
    }

    #[test]
    fn ts_dynamic_member_without_precise_reference_does_not_emit_direct_target() {
        let (db, file, caller) = db_with_function(Language::TypeScript, "src/app.ts");
        let sites = vec![
            site(
                &db,
                Language::TypeScript,
                file,
                caller,
                1,
                CallCallee::Index {
                    base: PlaceId(10),
                    index: None,
                },
                CallSyntaxKind::Index,
            ),
            site(
                &db,
                Language::TypeScript,
                file,
                caller,
                2,
                CallCallee::Unknown {
                    reason: UnresolvedCallReason::Eval,
                },
                CallSyntaxKind::Unknown,
            ),
            site(
                &db,
                Language::TypeScript,
                file,
                caller,
                3,
                CallCallee::Unknown {
                    reason: UnresolvedCallReason::CallApplyBind,
                },
                CallSyntaxKind::Member,
            ),
        ];

        assert!(resolve_direct_call_targets(&db, &sites).is_empty());
    }

    fn db_with_function(language: Language, path: &str) -> (LocalAnalysisDb, FileId, FunctionId) {
        let mut db = LocalAnalysisDb::new();
        let file = db.add_source_file(
            PathBuf::from(path),
            path.to_string(),
            language,
            "".into(),
            "content".to_string(),
        );
        let function = db.push_function(FunctionFact::new(
            FunctionId::from_raw(999),
            file,
            "caller".to_string(),
            span(file, 1),
            language,
            false,
            true,
            1,
            Vec::new(),
        ));
        (db, file, function)
    }

    fn site(
        db: &impl AnalysisHost,
        language: Language,
        file: FileId,
        caller: FunctionId,
        id: u64,
        callee: CallCallee,
        kind: CallSyntaxKind,
    ) -> CallSiteFact {
        CallSiteFact {
            in_throw: false,
            id: CallSiteId(id),
            language,
            file,
            caller,
            owner_symbol: None,
            body: MirBodyId(0),
            operation: MirOpId(id),
            span: span(file, id as u32 + 1),
            kind,
            callee,
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Unresolved,
            precision: CallPrecision::Conservative,
            stable_key: db.stable_key_interner().intern(format!("call-site:{id}")),
        }
    }

    fn span(file: FileId, line: u32) -> Span {
        Span::new(file, line * 10, line * 10 + 5, line, 1, line, 6)
    }
}

#[cfg(test)]
mod linked_bindings {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use super::resolve_direct_call_targets;
    use crate::analysis_api::{
        DefinitionFact, FunctionFact, ReferenceFact, ReferenceKind, SemanticImportFact,
        SemanticImportId, SemanticImportKind, SemanticStatus, SymbolFact, SymbolKind,
        SymbolNamespace, SymbolPrecision, SymbolResolutionStatus,
    };
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallAlgorithm, CallCallee, CallPrecision, CallSiteFact, CallSyntaxKind, CallTargetFact,
        CallTargetStatus,
    };
    use crate::analysis_neutral::ids::{CallSiteId, MirBodyId, MirOpId, PlaceId};
    use crate::analysis_neutral::symbol_graph::semantic::{
        NamespaceMemberCallFact, ValueFunctionFact, ValueFunctionKind,
    };
    use crate::internal_core::{FileId, FunctionId, Language, ReferenceId, Span, SymbolId};

    #[test]
    fn call_through_const_bound_function_literal_targets_the_literal() {
        let mut graph = Graph::new(&["src/caller.ts"]);
        let file = graph.files[0];
        let literal = graph.function(file, "handler", 40, 60);
        let binding = graph.symbol(file, "handler", SymbolKind::Constant, 30, 37);
        graph.value_function(span(file, 30, 37), span(file, 40, 60));
        let call = graph.reference(file, "handler", ReferenceKind::Call, 80, 87, binding);

        let targets = graph.resolve(file, call, "handler", 80, 89);

        assert_eq!(targets.len(), 1, "{targets:#?}");
        assert_eq!(targets[0].target_symbol, Some(binding));
        assert_eq!(targets[0].target_function, Some(literal));
    }

    #[test]
    fn const_binding_without_a_function_literal_row_has_no_target() {
        let mut graph = Graph::new(&["src/caller.ts"]);
        let file = graph.files[0];
        graph.function(file, "handler", 40, 60);
        let binding = graph.symbol(file, "handler", SymbolKind::Constant, 30, 37);
        let call = graph.reference(file, "handler", ReferenceKind::Call, 80, 87, binding);

        assert!(graph.resolve(file, call, "handler", 80, 89).is_empty());
    }

    #[test]
    fn imported_call_follows_the_module_link_to_the_exported_function() {
        let mut graph = Graph::new(&["src/caller.ts", "src/factory.ts"]);
        let (caller_file, factory_file) = (graph.files[0], graph.files[1]);
        let exported = graph.function(factory_file, "makeThing", 0, 50);
        let export_symbol = graph.symbol(factory_file, "makeThing", SymbolKind::Function, 9, 18);
        graph.callee_row(
            factory_file,
            (9, 18),
            (0, 50),
            ValueFunctionKind::Implementation,
        );
        let import = graph.symbol(caller_file, "makeThing", SymbolKind::Import, 9, 18);
        graph.reference(
            caller_file,
            "makeThing",
            ReferenceKind::Import,
            9,
            18,
            export_symbol,
        );
        let call = graph.reference(
            caller_file,
            "makeThing",
            ReferenceKind::Call,
            60,
            69,
            import,
        );

        let targets = graph.resolve(caller_file, call, "makeThing", 60, 71);

        assert_eq!(targets.len(), 1, "{targets:#?}");
        assert_eq!(targets[0].target_symbol, Some(import));
        assert_eq!(targets[0].target_function, Some(exported));
    }

    #[test]
    fn imported_call_follows_a_re_export_to_a_const_function_literal() {
        let mut graph = Graph::new(&["src/caller.ts", "src/index.ts", "src/impl.ts"]);
        let (caller_file, index_file, impl_file) = (graph.files[0], graph.files[1], graph.files[2]);
        let literal = graph.function(impl_file, "run", 20, 40);
        let binding = graph.symbol(impl_file, "run", SymbolKind::Constant, 13, 16);
        graph.value_function(span(impl_file, 13, 16), span(impl_file, 20, 40));
        let re_export = graph.symbol(index_file, "run", SymbolKind::Import, 9, 12);
        graph.reference(index_file, "run", ReferenceKind::Import, 9, 12, binding);
        let import = graph.symbol(caller_file, "run", SymbolKind::Import, 9, 12);
        graph.reference(caller_file, "run", ReferenceKind::Import, 9, 12, re_export);
        let call = graph.reference(caller_file, "run", ReferenceKind::Call, 50, 53, import);

        let targets = graph.resolve(caller_file, call, "run", 50, 55);

        assert_eq!(targets.len(), 1, "{targets:#?}");
        assert_eq!(targets[0].target_function, Some(literal));
    }

    /// An import linked to an exported variable that the frontend gives no
    /// single function (one reassigned after its declaration) answers like the
    /// variable itself would: no resolved row, so the call stays open.
    #[test]
    fn imported_variable_without_a_function_row_resolves_nothing() {
        let mut graph = Graph::new(&["src/caller.ts", "src/index.ts", "src/state.ts"]);
        let (caller_file, index_file, state_file) =
            (graph.files[0], graph.files[1], graph.files[2]);
        graph.function(state_file, "initial", 20, 40);
        let binding = graph.symbol(state_file, "handler", SymbolKind::Variable, 11, 18);
        let re_export = graph.symbol(index_file, "handler", SymbolKind::Import, 9, 16);
        graph.reference(index_file, "handler", ReferenceKind::Import, 9, 16, binding);
        let import = graph.symbol(caller_file, "handler", SymbolKind::Import, 9, 16);
        graph.reference(
            caller_file,
            "handler",
            ReferenceKind::Import,
            9,
            16,
            re_export,
        );
        let call = graph.reference(caller_file, "handler", ReferenceKind::Call, 50, 57, import);

        let targets = graph.resolve(caller_file, call, "handler", 50, 59);

        assert!(targets.is_empty(), "{targets:#?}");
    }

    #[test]
    fn namespace_import_binding_is_never_followed_to_one_export() {
        let mut graph = Graph::new(&["src/caller.ts", "src/util.ts"]);
        let (caller_file, util_file) = (graph.files[0], graph.files[1]);
        graph.function(util_file, "util", 0, 50);
        let export_symbol = graph.symbol(util_file, "util", SymbolKind::Function, 9, 13);
        graph.callee_row(
            util_file,
            (9, 13),
            (0, 50),
            ValueFunctionKind::Implementation,
        );
        let import = graph.symbol(caller_file, "util", SymbolKind::Import, 12, 16);
        graph.reference(
            caller_file,
            "util",
            ReferenceKind::Import,
            12,
            16,
            export_symbol,
        );
        graph.namespace_import(caller_file, "util");
        let call = graph.reference(caller_file, "util", ReferenceKind::Call, 60, 64, import);

        let targets = graph.resolve(caller_file, call, "util", 60, 66);

        assert!(
            targets
                .iter()
                .all(|target| target.target_function.is_none()),
            "{targets:#?}"
        );
    }

    #[test]
    fn import_link_cycle_has_no_target_function() {
        let mut graph = Graph::new(&["src/a.ts", "src/b.ts"]);
        let (a, b) = (graph.files[0], graph.files[1]);
        graph.function(b, "loop", 100, 120);
        let in_a = graph.symbol(a, "loop", SymbolKind::Import, 9, 13);
        let in_b = graph.symbol(b, "loop", SymbolKind::Import, 9, 13);
        graph.reference(a, "loop", ReferenceKind::Import, 9, 13, in_b);
        graph.reference(b, "loop", ReferenceKind::Import, 9, 13, in_a);
        let call = graph.reference(a, "loop", ReferenceKind::Call, 60, 64, in_a);

        let targets = graph.resolve(a, call, "loop", 60, 66);

        assert!(
            targets
                .iter()
                .all(|target| target.target_function.is_none()),
            "{targets:#?}"
        );
    }

    /// An import binding whose module links name two different exports has
    /// no single declaring symbol, so a call through it gets no target.
    #[test]
    fn import_binding_linked_to_two_exports_has_no_target_function() {
        let mut graph = Graph::new(&["src/caller.ts", "src/a.ts", "src/b.ts"]);
        let (caller_file, a_file, b_file) = (graph.files[0], graph.files[1], graph.files[2]);
        let mut exports = Vec::new();
        for file in [a_file, b_file] {
            graph.function(file, "run", 0, 50);
            exports.push(graph.symbol(file, "run", SymbolKind::Function, 9, 12));
            graph.callee_row(file, (9, 12), (0, 50), ValueFunctionKind::Implementation);
        }
        let import = graph.symbol(caller_file, "run", SymbolKind::Import, 9, 12);
        for export in exports {
            graph.reference(caller_file, "run", ReferenceKind::Import, 9, 12, export);
        }
        let call = graph.reference(caller_file, "run", ReferenceKind::Call, 50, 53, import);

        let targets = graph.resolve(caller_file, call, "run", 50, 55);

        assert!(
            targets
                .iter()
                .all(|target| target.target_function.is_none()),
            "{targets:#?}"
        );
    }

    /// Links `imports` import bindings into a chain ending at an exported
    /// function and resolves a call through the first binding.
    fn resolve_through_import_chain(imports: u32) -> (Option<FunctionId>, FunctionId) {
        let mut graph = Graph::new(&["src/caller.ts", "src/impl.ts"]);
        let (caller_file, impl_file) = (graph.files[0], graph.files[1]);
        let exported = graph.function(impl_file, "run", 0, 50);
        let mut next = graph.symbol(impl_file, "run", SymbolKind::Function, 9, 12);
        graph.callee_row(
            impl_file,
            (9, 12),
            (0, 50),
            ValueFunctionKind::Implementation,
        );
        for hop in (0..imports).rev() {
            let at = hop * 10;
            let import = graph.symbol(caller_file, "run", SymbolKind::Import, at, at + 3);
            graph.reference(caller_file, "run", ReferenceKind::Import, at, at + 3, next);
            next = import;
        }
        let call = graph.reference(caller_file, "run", ReferenceKind::Call, 500, 503, next);
        let targets = graph.resolve(caller_file, call, "run", 500, 505);
        assert_eq!(targets.len(), 1, "{targets:#?}");
        (targets[0].target_function, exported)
    }

    #[test]
    fn re_export_chain_within_the_hop_limit_reaches_the_export() {
        let (target, exported) = resolve_through_import_chain(super::MAX_IMPORT_HOPS as u32 - 1);
        assert_eq!(target, Some(exported));
    }

    #[test]
    fn re_export_chain_past_the_hop_limit_has_no_target_function() {
        let (target, _) = resolve_through_import_chain(super::MAX_IMPORT_HOPS as u32);
        assert_eq!(target, None);
    }

    /// Go has no callee identity rows: a declared function is the innermost
    /// same-named function around its name, never one nested inside it.
    #[test]
    fn go_declared_function_wins_over_a_nested_same_named_function() {
        let mut graph = Graph::with_language(Language::Go, &[("caller.go", "")]);
        let file = graph.files[0];
        let outer = graph.function(file, "run", 100, 300);
        graph.function(file, "run", 150, 200);
        let declared = graph.symbol(file, "run", SymbolKind::Function, 109, 112);
        let call = graph.reference(file, "run", ReferenceKind::Call, 400, 403, declared);

        let targets = graph.resolve(file, call, "run", 400, 405);

        assert_eq!(targets.len(), 1, "{targets:#?}");
        assert_eq!(targets[0].target_function, Some(outer));
    }

    /// A TS/JS function symbol reaches only the function its own row names,
    /// even when a same-named function holds its declared name.
    #[test]
    fn ts_declared_function_reaches_only_its_callee_row() {
        let mut graph = Graph::new(&["src/caller.ts"]);
        let file = graph.files[0];
        graph.function(file, "run", 100, 300);
        let inner = graph.function(file, "run", 150, 200);
        let declared = graph.symbol(file, "run", SymbolKind::Function, 159, 162);
        graph.callee_row(
            file,
            (159, 162),
            (150, 200),
            ValueFunctionKind::Implementation,
        );
        let call = graph.reference(file, "run", ReferenceKind::Call, 400, 403, declared);

        let targets = graph.resolve(file, call, "run", 400, 405);

        assert_eq!(targets.len(), 1, "{targets:#?}");
        assert_eq!(targets[0].target_function, Some(inner));
    }

    #[test]
    fn ts_function_symbol_without_a_callee_row_has_no_target_function() {
        let mut graph = Graph::new(&["src/caller.ts"]);
        let file = graph.files[0];
        graph.function(file, "run", 100, 300);
        let declared = graph.symbol(file, "run", SymbolKind::Function, 109, 112);
        let call = graph.reference(file, "run", ReferenceKind::Call, 400, 403, declared);

        let targets = graph.resolve(file, call, "run", 400, 405);

        assert_eq!(targets.len(), 1, "{targets:#?}");
        assert_eq!(targets[0].target_function, None);
    }

    #[test]
    fn overloaded_function_targets_the_implementation_not_a_signature() {
        let mut graph = Graph::new(&["src/pick.ts"]);
        let file = graph.files[0];
        graph.function(file, "pick", 0, 30);
        graph.function(file, "pick", 40, 70);
        let implementation = graph.function(file, "pick", 80, 120);
        let declared = graph.symbol(file, "pick", SymbolKind::Function, 9, 13);
        for (kind, function) in [
            (ValueFunctionKind::Signature, (0, 30)),
            (ValueFunctionKind::Implementation, (80, 120)),
            (ValueFunctionKind::Signature, (40, 70)),
        ] {
            graph.callee_row(file, (9, 13), function, kind);
        }
        let call = graph.reference(file, "pick", ReferenceKind::Call, 1100, 1104, declared);

        let targets = graph.resolve(file, call, "pick", 1100, 1106);

        assert_eq!(targets.len(), 1, "{targets:#?}");
        assert_eq!(targets[0].target_function, Some(implementation));
    }

    #[test]
    fn interface_merged_with_a_function_targets_the_function() {
        let mut graph = Graph::new(&["src/make.ts"]);
        let file = graph.files[0];
        let function = graph.function(file, "make", 30, 60);
        let declared = graph.symbol(file, "make", SymbolKind::Interface, 10, 14);
        graph.callee_row(file, (10, 14), (30, 60), ValueFunctionKind::Implementation);
        let call = graph.reference(file, "make", ReferenceKind::Call, 1100, 1104, declared);

        let targets = graph.resolve(file, call, "make", 1100, 1106);

        assert_eq!(targets.len(), 1, "{targets:#?}");
        assert_eq!(targets[0].target_function, Some(function));
    }

    #[test]
    fn interface_without_a_callee_row_is_not_called() {
        let mut graph = Graph::new(&["src/make.ts"]);
        let file = graph.files[0];
        graph.function(file, "make", 30, 60);
        let declared = graph.symbol(file, "make", SymbolKind::Interface, 10, 14);
        let call = graph.reference(file, "make", ReferenceKind::Call, 1100, 1104, declared);

        assert!(graph.resolve(file, call, "make", 1100, 1106).is_empty());
    }

    #[test]
    fn binding_with_two_implementations_has_no_target() {
        let mut graph = Graph::new(&["src/make.ts"]);
        let file = graph.files[0];
        graph.function(file, "make", 30, 60);
        graph.function(file, "make", 70, 100);
        let declared = graph.symbol(file, "make", SymbolKind::Interface, 10, 14);
        graph.callee_row(file, (10, 14), (30, 60), ValueFunctionKind::Implementation);
        graph.callee_row(file, (10, 14), (70, 100), ValueFunctionKind::Implementation);
        let call = graph.reference(file, "make", ReferenceKind::Call, 1100, 1104, declared);

        assert!(graph.resolve(file, call, "make", 1100, 1106).is_empty());
    }

    #[test]
    fn literal_mixed_with_a_declaration_has_no_target() {
        let mut graph = Graph::new(&["src/make.ts"]);
        let file = graph.files[0];
        graph.function(file, "make", 30, 60);
        graph.function(file, "make", 70, 100);
        let declared = graph.symbol(file, "make", SymbolKind::Variable, 10, 14);
        graph.callee_row(file, (10, 14), (30, 60), ValueFunctionKind::Literal);
        graph.callee_row(file, (10, 14), (70, 100), ValueFunctionKind::Implementation);
        let call = graph.reference(file, "make", ReferenceKind::Call, 1100, 1104, declared);

        assert!(graph.resolve(file, call, "make", 1100, 1106).is_empty());
    }

    #[test]
    fn callee_row_naming_no_function_fact_has_no_target() {
        let mut graph = Graph::new(&["src/make.ts"]);
        let file = graph.files[0];
        graph.function(file, "make", 30, 60);
        let declared = graph.symbol(file, "make", SymbolKind::Constant, 10, 14);
        graph.callee_row(file, (10, 14), (31, 60), ValueFunctionKind::Literal);
        let call = graph.reference(file, "make", ReferenceKind::Call, 1100, 1104, declared);

        assert!(graph.resolve(file, call, "make", 1100, 1106).is_empty());
    }

    #[test]
    fn bodyless_declaration_without_an_implementation_keeps_its_target() {
        let mut graph = Graph::new(&["src/ambient.ts"]);
        let file = graph.files[0];
        let ambient = graph.function(file, "ambient", 0, 44);
        let declared = graph.symbol(file, "ambient", SymbolKind::Function, 17, 24);
        graph.callee_row(file, (17, 24), (0, 44), ValueFunctionKind::Signature);
        let call = graph.reference(file, "ambient", ReferenceKind::Call, 1100, 1107, declared);

        let targets = graph.resolve(file, call, "ambient", 1100, 1109);

        assert_eq!(targets.len(), 1, "{targets:#?}");
        assert_eq!(targets[0].target_function, Some(ambient));
    }

    #[test]
    fn two_signatures_without_an_implementation_have_no_target() {
        let mut graph = Graph::new(&["src/ambient.ts"]);
        let file = graph.files[0];
        graph.function(file, "ambient", 0, 44);
        graph.function(file, "ambient", 50, 94);
        let declared = graph.symbol(file, "ambient", SymbolKind::Function, 17, 24);
        graph.callee_row(file, (17, 24), (0, 44), ValueFunctionKind::Signature);
        graph.callee_row(file, (17, 24), (50, 94), ValueFunctionKind::Signature);
        let call = graph.reference(file, "ambient", ReferenceKind::Call, 1100, 1107, declared);

        let targets = graph.resolve(file, call, "ambient", 1100, 1109);

        assert_eq!(targets.len(), 1, "{targets:#?}");
        assert_eq!(targets[0].target_function, None);
    }

    /// `ns.parse(x)` and the inner `ns.parse()` of `obj.parse(ns.parse())`, with
    /// the frontend's namespace member call rows and property references.
    struct NamespaceCalls {
        graph: Graph,
        caller_file: FileId,
        parse: FunctionId,
        direct: (u32, u32),
        outer: (u32, u32),
        inner: (u32, u32),
    }

    fn namespace_calls() -> NamespaceCalls {
        let caller_source = "import * as ns from \"./util\";\n\
                             ns.parse(x);\n\
                             obj.parse(ns.parse());\n";
        let util_source = "export function parse() {}\n";
        let mut graph = Graph::with_sources(&[
            ("src/caller.ts", caller_source),
            ("src/util.ts", util_source),
        ]);
        let (caller_file, util_file) = (graph.files[0], graph.files[1]);
        let parse_range = range_of(util_source, "export function parse() {}");
        let parse = graph.function(util_file, "parse", parse_range.0, parse_range.1);
        let declared = graph.symbol(util_file, "parse", SymbolKind::Function, 16, 21);
        graph.callee_row(
            util_file,
            (16, 21),
            parse_range,
            ValueFunctionKind::Implementation,
        );
        graph.namespace_import(caller_file, "ns");
        let direct = range_of(caller_source, "ns.parse(x)");
        let outer = range_of(caller_source, "obj.parse(ns.parse())");
        let inner = range_of(caller_source, "ns.parse()");
        for site in [direct, inner] {
            let property = (site.0 + 3, site.0 + 8);
            graph.member_reference(caller_file, "parse", property, declared);
            graph.namespace_call(caller_file, site, property);
        }
        NamespaceCalls {
            graph,
            caller_file,
            parse,
            direct,
            outer,
            inner,
        }
    }

    #[test]
    fn namespace_member_call_targets_only_its_own_site() {
        let NamespaceCalls {
            mut graph,
            caller_file,
            parse,
            direct,
            outer,
            inner,
        } = namespace_calls();

        for site in [direct, inner] {
            let targets = graph.resolve_member(caller_file, "parse", site);
            assert_eq!(targets.len(), 1, "{targets:#?}");
            assert_eq!(targets[0].target_function, Some(parse));
            assert_eq!(targets[0].algorithm, CallAlgorithm::ImportBinding);
        }

        // `obj.parse` is not the namespace member, so only the name-only
        // fallback may guess at it.
        let targets = graph.resolve_member(caller_file, "parse", outer);
        assert!(
            targets
                .iter()
                .all(|target| target.algorithm == CallAlgorithm::SyntaxOnly
                    && target.precision == CallPrecision::Heuristic),
            "{targets:#?}"
        );
    }

    /// An identifier callee without its own reference must not borrow the
    /// namespace member reference of an argument: `parse(ns.parse())`.
    #[test]
    fn identifier_call_does_not_borrow_a_namespace_member_reference() {
        let NamespaceCalls {
            mut graph,
            caller_file,
            outer,
            ..
        } = namespace_calls();
        let outer_parse = (outer.0 + 4, outer.1);

        let targets = graph.resolve_site(
            caller_file,
            outer_parse,
            CallSyntaxKind::Function,
            CallCallee::Identifier {
                reference: None,
                name: "parse".to_string(),
            },
        );

        assert!(
            targets
                .iter()
                .all(|target| target.precision == CallPrecision::Heuristic),
            "{targets:#?}"
        );
    }

    /// The namespace row ties a call span to a property reference. When the
    /// call site names a different property than that reference, the two
    /// producers disagree about the callee, so the call gets no precise target.
    #[test]
    fn namespace_member_reference_with_another_name_gives_no_target() {
        let NamespaceCalls {
            mut graph,
            caller_file,
            direct,
            ..
        } = namespace_calls();

        let targets = graph.resolve_member(caller_file, "format", direct);

        assert!(
            targets
                .iter()
                .all(|target| target.precision == CallPrecision::Heuristic),
            "{targets:#?}"
        );
    }

    /// Resolution is one index probe per call, never a scan of the file's
    /// functions: a file with many distinct functions, each called once,
    /// resolves in time linear in its size. Go symbols take the declared-name
    /// lookup, which is where a per-call scan would make this quadratic.
    #[test]
    fn resolution_scales_with_one_index_probe_per_call() {
        // Wall time alone moves with machine load, so compare two sizes. One
        // index probe per call grows about 4x from 10k to 40k calls; a scan
        // of every function per call grows about 16x.
        let small = fastest_resolution(10_000);
        let large = fastest_resolution(40_000);
        let ratio = large.as_secs_f64() / small.as_secs_f64().max(1e-6);
        assert!(
            ratio < 10.0,
            "resolving 4x the calls took {ratio:.1}x as long ({small:?} -> {large:?})"
        );
    }

    /// Builds `functions` functions that each call themselves once, checks
    /// that every call reaches its own function, and returns the fastest of
    /// three resolutions.
    fn fastest_resolution(functions: u32) -> std::time::Duration {
        let mut graph = Graph::with_language(Language::Go, &[("big.go", "")]);
        let file = graph.files[0];
        let mut calls = Vec::new();
        for index in 0..functions {
            let at = 100 + index * 100;
            let name = format!("f{index}");
            let function = graph.function(file, &name, at, at + 50);
            let declared = graph.symbol(file, &name, SymbolKind::Function, at + 5, at + 10);
            let call =
                graph.reference(file, &name, ReferenceKind::Call, at + 60, at + 65, declared);
            calls.push((name, call, at + 60, function));
        }
        let sites = calls
            .iter()
            .enumerate()
            .map(|(index, (name, reference, at, _))| {
                graph.site(
                    index as u64,
                    file,
                    (*at, at + 7),
                    CallSyntaxKind::Function,
                    CallCallee::Identifier {
                        reference: Some(*reference),
                        name: name.clone(),
                    },
                )
            })
            .collect::<Vec<_>>();
        graph.store();
        let expected = sites
            .iter()
            .zip(&calls)
            .map(|(site, (.., function))| (site.id, Some(*function)))
            .collect::<BTreeSet<_>>();

        (0..3)
            .map(|_| {
                let started = std::time::Instant::now();
                let targets = resolve_direct_call_targets(&graph.db, &sites);
                let elapsed = started.elapsed();
                let reached = targets
                    .iter()
                    .map(|target| (target.site, target.target_function))
                    .collect::<BTreeSet<_>>();
                assert_eq!(reached, expected, "every call reaches its own function");
                elapsed
            })
            .min()
            .expect("three runs")
    }

    fn range_of(source: &str, needle: &str) -> (u32, u32) {
        let start = source.find(needle).expect("needle in source");
        (start as u32, (start + needle.len()) as u32)
    }

    struct Graph {
        db: LocalAnalysisDb,
        language: Language,
        files: Vec<FileId>,
        caller: FunctionId,
        symbols: Vec<SymbolFact>,
        references: Vec<ReferenceFact>,
        value_functions: Vec<ValueFunctionFact>,
        namespace_member_calls: Vec<NamespaceMemberCallFact>,
        semantic_imports: Vec<SemanticImportFact>,
    }

    impl Graph {
        fn new(paths: &[&str]) -> Self {
            let sources: Vec<(&str, &str)> = paths.iter().map(|path| (*path, "")).collect();
            Self::with_sources(&sources)
        }

        fn with_sources(files: &[(&str, &str)]) -> Self {
            Self::with_language(Language::TypeScript, files)
        }

        fn with_language(language: Language, files: &[(&str, &str)]) -> Self {
            let mut db = LocalAnalysisDb::new();
            let files: Vec<FileId> = files
                .iter()
                .map(|(path, source)| {
                    db.add_source_file(
                        PathBuf::from(path),
                        (*path).to_string(),
                        language,
                        (*source).into(),
                        "content".to_string(),
                    )
                })
                .collect();
            let caller = db.push_function(FunctionFact::new(
                FunctionId::from_raw(999),
                files[0],
                "caller".to_string(),
                span(files[0], 1000, 2000),
                language,
                false,
                true,
                1,
                Vec::new(),
            ));
            Self {
                db,
                language,
                files,
                caller,
                symbols: Vec::new(),
                references: Vec::new(),
                value_functions: Vec::new(),
                namespace_member_calls: Vec::new(),
                semantic_imports: Vec::new(),
            }
        }

        fn function(&mut self, file: FileId, name: &str, start: u32, end: u32) -> FunctionId {
            self.db.push_function(FunctionFact::new(
                FunctionId::from_raw(999),
                file,
                name.to_string(),
                span(file, start, end),
                self.language,
                false,
                true,
                1,
                Vec::new(),
            ))
        }

        fn symbol(
            &mut self,
            file: FileId,
            name: &str,
            kind: SymbolKind,
            start: u32,
            end: u32,
        ) -> SymbolId {
            let id = SymbolId::from_raw(self.symbols.len() as u64);
            self.symbols.push(SymbolFact::new(
                id,
                self.language,
                name.to_string(),
                name.to_string(),
                kind,
                SymbolNamespace::Value,
                Some(file),
                None,
                None,
                None,
                Some(span(file, start, end)),
                true,
                self.db
                    .stable_key_interner()
                    .intern(format!("symbol:{}:{name}", id.0)),
                SymbolPrecision::ExactSemantic,
            ));
            id
        }

        fn reference(
            &mut self,
            file: FileId,
            name: &str,
            kind: ReferenceKind,
            start: u32,
            end: u32,
            target: SymbolId,
        ) -> ReferenceId {
            let id = ReferenceId::from_raw(self.references.len() as u64);
            let precision = if kind == ReferenceKind::Import {
                SymbolPrecision::ModuleLinked
            } else {
                SymbolPrecision::ExactLocal
            };
            self.references.push(ReferenceFact::new(
                id,
                self.language,
                name.to_string(),
                name.to_string(),
                kind,
                SymbolNamespace::Value,
                Some(file),
                None,
                None,
                None,
                Some(span(file, start, end)),
                Some(target),
                vec![target],
                self.db
                    .stable_key_interner()
                    .intern(format!("reference:{name}:{}", id.0)),
                SymbolResolutionStatus::Resolved,
                precision,
            ));
            id
        }

        /// A namespace member call reference as the symbol graph links it.
        fn member_reference(
            &mut self,
            file: FileId,
            name: &str,
            (start, end): (u32, u32),
            target: SymbolId,
        ) -> ReferenceId {
            let id = self.reference(file, name, ReferenceKind::Call, start, end, target);
            self.references[id.0 as usize].precision = SymbolPrecision::ModuleLinked;
            id
        }

        fn value_function(&mut self, binding: Span, function: Span) {
            self.value_functions.push(ValueFunctionFact {
                binding,
                function,
                kind: ValueFunctionKind::Literal,
            });
        }

        /// A callee identity row as the TS/JS frontend records it for the
        /// binding declared at `binding`.
        fn callee_row(
            &mut self,
            file: FileId,
            (binding_start, binding_end): (u32, u32),
            (function_start, function_end): (u32, u32),
            kind: ValueFunctionKind,
        ) {
            self.value_functions.push(ValueFunctionFact {
                binding: span(file, binding_start, binding_end),
                function: span(file, function_start, function_end),
                kind,
            });
        }

        /// The frontend's record that the call at `call` has the namespace
        /// member callee whose property is at `property`.
        fn namespace_call(
            &mut self,
            file: FileId,
            (call_start, call_end): (u32, u32),
            (property_start, property_end): (u32, u32),
        ) {
            self.namespace_member_calls.push(NamespaceMemberCallFact {
                call: span(file, call_start, call_end),
                property: span(file, property_start, property_end),
            });
        }

        fn namespace_import(&mut self, file: FileId, local_name: &str) {
            self.semantic_imports.push(SemanticImportFact {
                id: SemanticImportId(self.semantic_imports.len() as u64),
                language: self.language,
                file: Some(file),
                package: None,
                module: None,
                scope: None,
                import_path: "./util".to_string(),
                local_name: Some(local_name.to_string()),
                imported_name: None,
                namespace: SymbolNamespace::Value,
                kind: SemanticImportKind::EsNamespace,
                stable_key: self
                    .db
                    .stable_key_interner()
                    .intern(format!("semantic-import:{local_name}")),
                status: SemanticStatus::Resolved,
            });
        }

        fn resolve(
            &mut self,
            file: FileId,
            reference: ReferenceId,
            name: &str,
            start: u32,
            end: u32,
        ) -> Vec<CallTargetFact> {
            self.resolve_site(
                file,
                (start, end),
                CallSyntaxKind::Function,
                CallCallee::Identifier {
                    reference: Some(reference),
                    name: name.to_string(),
                },
            )
        }

        fn resolve_member(
            &mut self,
            file: FileId,
            property: &str,
            (start, end): (u32, u32),
        ) -> Vec<CallTargetFact> {
            self.resolve_site(
                file,
                (start, end),
                CallSyntaxKind::StaticMember,
                CallCallee::Member {
                    base: PlaceId(u64::MAX),
                    property: property.to_string(),
                },
            )
        }

        fn resolve_site(
            &mut self,
            file: FileId,
            range: (u32, u32),
            kind: CallSyntaxKind,
            callee: CallCallee,
        ) -> Vec<CallTargetFact> {
            self.store();
            let site = self.site(1, file, range, kind, callee);
            resolve_direct_call_targets(&self.db, &[site])
        }

        fn store(&mut self) {
            let definitions: Vec<DefinitionFact> = Vec::new();
            self.db.replace_symbol_graph_facts(
                self.symbols.clone(),
                definitions,
                self.references.clone(),
            );
            self.db
                .replace_semantic_imports(self.semantic_imports.clone());
            self.db
                .replace_value_functions(self.value_functions.clone());
            self.db
                .replace_namespace_member_calls(self.namespace_member_calls.clone());
        }

        fn site(
            &self,
            id: u64,
            file: FileId,
            (start, end): (u32, u32),
            kind: CallSyntaxKind,
            callee: CallCallee,
        ) -> CallSiteFact {
            CallSiteFact {
                in_throw: false,
                id: CallSiteId(id),
                language: self.language,
                file,
                caller: self.caller,
                owner_symbol: None,
                body: MirBodyId(0),
                operation: MirOpId(id),
                span: span(file, start, end),
                kind,
                callee,
                receiver: None,
                arguments: Vec::new(),
                result: None,
                status: CallTargetStatus::Unresolved,
                precision: CallPrecision::Conservative,
                stable_key: self
                    .db
                    .stable_key_interner()
                    .intern(format!("call-site:{id}")),
            }
        }
    }

    fn span(file: FileId, start: u32, end: u32) -> Span {
        Span::new(file, start, end, 1, start + 1, 1, end + 1)
    }
}
