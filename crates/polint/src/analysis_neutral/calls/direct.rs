use std::collections::BTreeMap;

use crate::analysis_api::FactFamily;
use crate::analysis_api::SemanticStatus;
use crate::analysis_api::{
    FunctionFact, ReferenceFact, SymbolFact, SymbolKind, SymbolPrecision, SymbolResolutionStatus,
};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallCallee, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact,
    CallSyntaxKind, CallTargetFact, CallTargetStatus,
};
use crate::analysis_neutral::ids::CallTargetId;
use crate::analysis_neutral::mir_op::UnsupportedDomain;
use crate::analysis_neutral::stable_key::semantic_stable_key;
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
        if !is_direct_callable_symbol(symbol) {
            continue;
        }
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
            target_function: index.function_for_symbol(symbol),
            target_symbol: Some(target_symbol),
            edge_kind: edge_kind_for_site(site.kind),
            algorithm,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::NativeDirect,
            precision: CallPrecision::SetupAware,
            stable_key: target_stable_key(interner, site, algorithm, symbol),
        });
    }

    rows.sort_by(|left, right| {
        interner
            .compare_canonical(left.stable_key, right.stable_key)
            .then_with(|| left.site.cmp(&right.site))
    });
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
}

impl<'db, H: AnalysisHost + ?Sized> DirectIndex<'db, H> {
    fn new(db: &'db H) -> Self {
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
                reference
                    .primary_span
                    .as_ref()
                    .is_some_and(|span| spans_overlap_or_touch(span, &site.span))
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

    fn function_for_symbol(&self, symbol: &SymbolFact) -> Option<FunctionId> {
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
        let mut matches = self
            .db
            .functions()
            .iter()
            .filter(|function| function.file == file)
            .filter(|function| function_matches_callee(&function.name, callee_name));
        let first = matches.next()?;
        if matches.next().is_none() {
            Some(first)
        } else {
            None
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

fn function_matches_callee(function_name: &str, callee_name: &str) -> bool {
    function_name == callee_name || short_function_name(function_name) == Some(callee_name)
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
        let mut fixture = Fixture::new(Language::TypeScript, "src/caller.ts");
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

    struct Fixture {
        db: LocalAnalysisDb,
        file: FileId,
        caller: FunctionId,
        symbols: Vec<SymbolFact>,
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
                file,
                caller,
                symbols: Vec::new(),
                definitions: Vec::new(),
                references: Vec::new(),
                semantic_imports: Vec::new(),
            }
        }

        fn add_function(&mut self, name: &str, line: u32) -> FunctionId {
            self.db
                .push_function(function(self.file, Language::TypeScript, name, line))
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
                Language::TypeScript,
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
                Language::TypeScript,
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
                Language::TypeScript,
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
                language: Language::TypeScript,
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
