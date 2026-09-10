use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_api::FunctionFact;
use crate::analysis_api::{FactFamily, FactRef, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::{CallCallee, CallSiteFact, CallSyntaxKind};
use crate::analysis_neutral::entrypoints::facts::{
    EntrypointConfidence, EntrypointFact, EntrypointKind, EntrypointPrecision,
    EntrypointProvenance, EntrypointStatus, TriggerMetadata, UnresolvedFrameworkFact,
    UnresolvedFrameworkReason,
};
use crate::analysis_neutral::entrypoints::provider::ENTRYPOINTS_PROVIDER_ID;
use crate::analysis_neutral::ids::{EntrypointId, UnresolvedFrameworkId};
use crate::analysis_neutral::places::PlaceRoot;
use crate::internal_core::{FileId, FunctionId, Language, Span};

// ---------------------------------------------------------------------------
// Public output
// ---------------------------------------------------------------------------

pub struct GoRecognizerOutput {
    pub entrypoints: Vec<EntrypointFact>,
    pub unresolved: Vec<UnresolvedFrameworkFact>,
}

// ---------------------------------------------------------------------------
// Framework detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GoFramework {
    NetHttp,
    Chi,
    Cobra,
    Testing,
}

fn classify_import(path: &str, file_path: &str) -> Option<(&'static str, GoFramework)> {
    match path {
        "net/http" => Some(("go.net_http", GoFramework::NetHttp)),
        "github.com/go-chi/chi" | "github.com/go-chi/chi/v5" => Some(("go.chi", GoFramework::Chi)),
        "github.com/spf13/cobra" => Some(("go.cobra", GoFramework::Cobra)),
        "testing" if file_path.ends_with("_test.go") => Some(("go.testing", GoFramework::Testing)),
        _ => None,
    }
}

/// Import paths that look like Go web/HTTP framework packages but are not
/// covered by default recognizers (net/http, chi). Per D-10 these should
/// produce UnresolvedFrameworkFact rows.
fn is_unrecognized_framework_import(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let markers = [
        "router", "http", "server", "handler", "mux", "gin", "echo", "fiber", "gorilla",
    ];
    // Exclude the known frameworks we handle natively
    if path == "net/http"
        || path.starts_with("github.com/go-chi/chi")
        || path == "github.com/spf13/cobra"
        || path == "testing"
    {
        return false;
    }
    markers.iter().any(|marker| lower.contains(marker))
}

// ---------------------------------------------------------------------------
// Main recognizer entry point
// ---------------------------------------------------------------------------

pub fn recognize_go_entrypoints(db: &impl AnalysisHost) -> GoRecognizerOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let files_by_id: BTreeMap<FileId, &crate::analysis_api::SourceFile> =
        db.files().iter().map(|file| (file.id, file)).collect();
    let functions_by_id: BTreeMap<FunctionId, &FunctionFact> =
        db.functions().iter().map(|f| (f.id, f)).collect();
    let functions_by_file: BTreeMap<FileId, Vec<&FunctionFact>> = {
        let mut map: BTreeMap<FileId, Vec<&FunctionFact>> = BTreeMap::new();
        for f in db.functions() {
            if f.language == Language::Go {
                map.entry(f.file).or_default().push(f);
            }
        }
        map
    };

    // Build per-file framework imports index
    let mut file_frameworks: BTreeMap<FileId, Vec<(&str, GoFramework, Span)>> = BTreeMap::new();
    let mut unrecognized_imports: Vec<(FileId, String, Span)> = Vec::new();

    for import in db.imports() {
        if import.language != Language::Go {
            continue;
        }
        let file_path = files_by_id
            .get(&import.file)
            .map(|f| f.relative_path.as_str())
            .unwrap_or("");
        if let Some((framework_id, framework)) = classify_import(&import.path, file_path) {
            file_frameworks.entry(import.file).or_default().push((
                framework_id,
                framework,
                import.span.clone(),
            ));
        } else if is_unrecognized_framework_import(&import.path) {
            unrecognized_imports.push((import.file, import.path.clone(), import.span.clone()));
        }
    }

    let mut entrypoints = Vec::new();
    let mut unresolved = Vec::new();

    // 1. Detect net/http and chi entrypoints from call sites
    recognize_http_entrypoints(
        db,
        &files_by_id,
        &functions_by_id,
        &file_frameworks,
        &mut entrypoints,
        &mut unresolved,
    );

    // 2. Detect cobra entrypoints from call sites
    recognize_cobra_entrypoints(
        db,
        &files_by_id,
        &functions_by_id,
        &file_frameworks,
        &mut entrypoints,
        &mut unresolved,
    );

    // 3. Detect Go testing entrypoints from function naming conventions
    recognize_test_entrypoints(
        interner,
        db,
        &files_by_id,
        &file_frameworks,
        &functions_by_file,
        &mut entrypoints,
    );

    // 4. Emit UnresolvedFrameworkFact for unrecognized Go framework imports (D-10)
    for (file, import_path, span) in &unrecognized_imports {
        let file_key = file_stable_key(db, *file);
        let stable_key = stable_key_from_parts(
            interner,
            FactFamily::UnresolvedFramework,
            &[
                ("language", "Go".to_string()),
                ("file_key", file_key),
                ("import_path", import_path.clone()),
            ],
        );

        unresolved.push(UnresolvedFrameworkFact {
            id: UnresolvedFrameworkId(0),
            language: Language::Go,
            file: *file,
            span: span.clone(),
            framework_id: format!("go.unknown:{import_path}"),
            reason: UnresolvedFrameworkReason::UnsupportedFrameworkVersion,
            evidence: format!("Go import \"{import_path}\" detected"),
            scope_description: format!("unrecognized Go framework import: {import_path}"),
            precision: EntrypointPrecision::Conservative,
            provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
            stable_key,
        });
    }

    // 5. Check for chi imports without matching registration patterns (D-10)
    emit_unresolved_for_unused_frameworks(
        interner,
        db,
        &file_frameworks,
        &entrypoints,
        &mut unresolved,
    );

    // Sort output by stable key
    entrypoints.sort_by(|entrypoint, other| {
        interner.compare_canonical(entrypoint.stable_key, other.stable_key)
    });
    unresolved.sort_by(|fact, other| interner.compare_canonical(fact.stable_key, other.stable_key));

    GoRecognizerOutput {
        entrypoints,
        unresolved,
    }
}

// ---------------------------------------------------------------------------
// HTTP recognizers (net/http and chi)
// ---------------------------------------------------------------------------

/// HTTP method names recognized on chi router method calls.
const CHI_METHODS: &[&str] = &[
    "Get", "Post", "Put", "Delete", "Patch", "Options", "Head", "Connect", "Trace",
];

fn recognize_http_entrypoints(
    db: &impl AnalysisHost,
    _files_by_id: &BTreeMap<FileId, &crate::analysis_api::SourceFile>,
    functions_by_id: &BTreeMap<FunctionId, &FunctionFact>,
    file_frameworks: &BTreeMap<FileId, Vec<(&str, GoFramework, Span)>>,
    entrypoints: &mut Vec<EntrypointFact>,
    unresolved: &mut Vec<UnresolvedFrameworkFact>,
) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    for site in db.call_sites() {
        if site.language != Language::Go {
            continue;
        }

        let frameworks = match file_frameworks.get(&site.file) {
            Some(fw) => fw,
            None => continue,
        };

        let has_net_http = frameworks
            .iter()
            .any(|(_, fw, _)| *fw == GoFramework::NetHttp);
        let has_chi = frameworks.iter().any(|(_, fw, _)| *fw == GoFramework::Chi);

        if !has_net_http && !has_chi {
            continue;
        }

        match &site.callee {
            // Match http.HandleFunc or http.Handle calls
            CallCallee::Member { property, .. }
                if has_net_http
                    && (property == "HandleFunc" || property == "Handle")
                    && matches!(
                        site.kind,
                        CallSyntaxKind::Member | CallSyntaxKind::StaticMember
                    ) =>
            {
                let route_path = extract_first_arg_literal(db, site);
                let handler_function = resolve_handler_function(db, functions_by_id, site);
                let file_key = file_stable_key(db, site.file);

                if let Some(target_function) = handler_function {
                    let stable_key = entrypoint_stable_key(
                        interner,
                        &file_key,
                        "go.net_http",
                        "HttpRoute",
                        &format!("{}", target_function.0),
                        &span_key(&site.span),
                    );

                    entrypoints.push(EntrypointFact {
                        id: EntrypointId(0),
                        language: Language::Go,
                        framework_id: "go.net_http".to_string(),
                        kind: EntrypointKind::HttpRoute,
                        target_function,
                        target_symbol: None,
                        registration_span: site.span.clone(),
                        registration_file: site.file,
                        trigger_metadata: TriggerMetadata {
                            method: None, // HandleFunc registers all methods
                            path: route_path,
                            tool_name: None,
                            event_name: None,
                            test_name: None,
                        },
                        trust_boundary_link: None,
                        precision: EntrypointPrecision::Heuristic,
                        provenance: EntrypointProvenance::NativeRecognizer,
                        confidence: EntrypointConfidence::High,
                        status: EntrypointStatus::Resolved,
                        provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
                        stable_key,
                    });
                } else {
                    // Cannot resolve handler
                    let stable_key = unresolved_stable_key(
                        interner,
                        &file_key,
                        "go.net_http",
                        "UnresolvedHandler",
                        &span_key(&site.span),
                    );
                    unresolved.push(UnresolvedFrameworkFact {
                        id: UnresolvedFrameworkId(0),
                        language: Language::Go,
                        file: site.file,
                        span: site.span.clone(),
                        framework_id: "go.net_http".to_string(),
                        reason: UnresolvedFrameworkReason::UnresolvedHandler,
                        evidence: format!(
                            "http.{} call but handler function could not be resolved",
                            property
                        ),
                        scope_description: format!("net/http {} registration", property),
                        precision: EntrypointPrecision::Conservative,
                        provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
                        stable_key,
                    });
                }
            }

            // Match chi router method calls: r.Get, r.Post, etc.
            CallCallee::Member { property, .. }
                if has_chi
                    && CHI_METHODS.contains(&property.as_str())
                    && matches!(
                        site.kind,
                        CallSyntaxKind::Member | CallSyntaxKind::StaticMember
                    ) =>
            {
                let route_path = extract_first_arg_literal(db, site);
                let handler_function = resolve_handler_function(db, functions_by_id, site);
                let file_key = file_stable_key(db, site.file);
                let method = property.to_uppercase();

                if let Some(target_function) = handler_function {
                    let stable_key = entrypoint_stable_key(
                        interner,
                        &file_key,
                        "go.chi",
                        "HttpRoute",
                        &format!("{}", target_function.0),
                        &span_key(&site.span),
                    );

                    entrypoints.push(EntrypointFact {
                        id: EntrypointId(0),
                        language: Language::Go,
                        framework_id: "go.chi".to_string(),
                        kind: EntrypointKind::HttpRoute,
                        target_function,
                        target_symbol: None,
                        registration_span: site.span.clone(),
                        registration_file: site.file,
                        trigger_metadata: TriggerMetadata {
                            method: Some(method),
                            path: route_path,
                            tool_name: None,
                            event_name: None,
                            test_name: None,
                        },
                        trust_boundary_link: None,
                        precision: EntrypointPrecision::Heuristic,
                        provenance: EntrypointProvenance::NativeRecognizer,
                        confidence: EntrypointConfidence::High,
                        status: EntrypointStatus::Resolved,
                        provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
                        stable_key,
                    });
                } else {
                    let stable_key = unresolved_stable_key(
                        interner,
                        &file_key,
                        "go.chi",
                        "UnresolvedHandler",
                        &span_key(&site.span),
                    );
                    unresolved.push(UnresolvedFrameworkFact {
                        id: UnresolvedFrameworkId(0),
                        language: Language::Go,
                        file: site.file,
                        span: site.span.clone(),
                        framework_id: "go.chi".to_string(),
                        reason: UnresolvedFrameworkReason::UnresolvedHandler,
                        evidence: format!(
                            "chi r.{} call but handler function could not be resolved",
                            property
                        ),
                        scope_description: format!("chi {} route registration", property),
                        precision: EntrypointPrecision::Conservative,
                        provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
                        stable_key,
                    });
                }
            }

            // Match chi r.Use for middleware
            CallCallee::Member { property, .. }
                if has_chi
                    && property == "Use"
                    && matches!(
                        site.kind,
                        CallSyntaxKind::Member | CallSyntaxKind::StaticMember
                    ) =>
            {
                let handler_function = resolve_handler_function(db, functions_by_id, site);
                let file_key = file_stable_key(db, site.file);

                if let Some(target_function) = handler_function {
                    let stable_key = entrypoint_stable_key(
                        interner,
                        &file_key,
                        "go.chi",
                        "HttpMiddleware",
                        &format!("{}", target_function.0),
                        &span_key(&site.span),
                    );

                    entrypoints.push(EntrypointFact {
                        id: EntrypointId(0),
                        language: Language::Go,
                        framework_id: "go.chi".to_string(),
                        kind: EntrypointKind::HttpMiddleware,
                        target_function,
                        target_symbol: None,
                        registration_span: site.span.clone(),
                        registration_file: site.file,
                        trigger_metadata: TriggerMetadata::empty(),
                        trust_boundary_link: None,
                        precision: EntrypointPrecision::Heuristic,
                        provenance: EntrypointProvenance::NativeRecognizer,
                        confidence: EntrypointConfidence::Medium,
                        status: EntrypointStatus::Resolved,
                        provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
                        stable_key,
                    });
                }
            }

            // Match chi r.Route for sub-router
            CallCallee::Member { property, .. }
                if has_chi
                    && property == "Route"
                    && matches!(
                        site.kind,
                        CallSyntaxKind::Member | CallSyntaxKind::StaticMember
                    ) =>
            {
                let route_path = extract_first_arg_literal(db, site);
                let handler_function = resolve_handler_function(db, functions_by_id, site);
                let file_key = file_stable_key(db, site.file);

                if let Some(target_function) = handler_function {
                    let stable_key = entrypoint_stable_key(
                        interner,
                        &file_key,
                        "go.chi",
                        "HttpRoute",
                        &format!("{}", target_function.0),
                        &span_key(&site.span),
                    );

                    entrypoints.push(EntrypointFact {
                        id: EntrypointId(0),
                        language: Language::Go,
                        framework_id: "go.chi".to_string(),
                        kind: EntrypointKind::HttpRoute,
                        target_function,
                        target_symbol: None,
                        registration_span: site.span.clone(),
                        registration_file: site.file,
                        trigger_metadata: TriggerMetadata {
                            method: None,
                            path: route_path,
                            tool_name: None,
                            event_name: None,
                            test_name: None,
                        },
                        trust_boundary_link: None,
                        precision: EntrypointPrecision::Heuristic,
                        provenance: EntrypointProvenance::NativeRecognizer,
                        confidence: EntrypointConfidence::Medium,
                        status: EntrypointStatus::Resolved,
                        provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
                        stable_key,
                    });
                }
            }

            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Cobra recognizer
// ---------------------------------------------------------------------------

fn recognize_cobra_entrypoints(
    db: &impl AnalysisHost,
    _files_by_id: &BTreeMap<FileId, &crate::analysis_api::SourceFile>,
    functions_by_id: &BTreeMap<FunctionId, &FunctionFact>,
    file_frameworks: &BTreeMap<FileId, Vec<(&str, GoFramework, Span)>>,
    entrypoints: &mut Vec<EntrypointFact>,
    _unresolved: &mut Vec<UnresolvedFrameworkFact>,
) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    for site in db.call_sites().iter() {
        if site.language != Language::Go {
            continue;
        }

        let frameworks = match file_frameworks.get(&site.file) {
            Some(fw) => fw,
            None => continue,
        };

        let has_cobra = frameworks
            .iter()
            .any(|(_, fw, _)| *fw == GoFramework::Cobra);
        if !has_cobra {
            continue;
        }

        // Detect cobra.Command constructor or AddCommand calls
        let is_cobra_pattern = match &site.callee {
            CallCallee::Member { property, .. } if property == "AddCommand" => true,
            CallCallee::Identifier { name, .. } if name.contains("Command") => true,
            CallCallee::Constructor {
                name: Some(name), ..
            } if name.contains("Command") => true,
            _ => false,
        };

        if is_cobra_pattern {
            let handler_function = resolve_handler_function(db, functions_by_id, site);
            let file_key = file_stable_key(db, site.file);

            if let Some(target_function) = handler_function {
                let stable_key = entrypoint_stable_key(
                    interner,
                    &file_key,
                    "go.cobra",
                    "CliCommand",
                    &format!("{}", target_function.0),
                    &span_key(&site.span),
                );

                entrypoints.push(EntrypointFact {
                    id: EntrypointId(0),
                    language: Language::Go,
                    framework_id: "go.cobra".to_string(),
                    kind: EntrypointKind::CliCommand,
                    target_function,
                    target_symbol: None,
                    registration_span: site.span.clone(),
                    registration_file: site.file,
                    trigger_metadata: TriggerMetadata {
                        method: None,
                        path: None,
                        tool_name: None, // Would need deeper analysis to extract Use field
                        event_name: None,
                        test_name: None,
                    },
                    trust_boundary_link: None,
                    precision: EntrypointPrecision::Conservative, // Per D-09
                    provenance: EntrypointProvenance::NativeRecognizer,
                    confidence: EntrypointConfidence::Medium,
                    status: EntrypointStatus::Resolved,
                    provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
                    stable_key,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Testing recognizer
// ---------------------------------------------------------------------------

/// Go test function naming prefixes that indicate test entrypoints.
#[cfg(test)]
const GO_TEST_PREFIXES: &[&str] = &["Test", "Benchmark", "Example", "Fuzz"];

fn recognize_test_entrypoints(
    interner: &crate::internal_core::StableKeyInterner,
    db: &impl AnalysisHost,
    files_by_id: &BTreeMap<FileId, &crate::analysis_api::SourceFile>,
    file_frameworks: &BTreeMap<FileId, Vec<(&str, GoFramework, Span)>>,
    functions_by_file: &BTreeMap<FileId, Vec<&FunctionFact>>,
    entrypoints: &mut Vec<EntrypointFact>,
) {
    // For each file that imports "testing" and ends in _test.go, scan functions
    for (file_id, frameworks) in file_frameworks {
        let has_testing = frameworks
            .iter()
            .any(|(_, fw, _)| *fw == GoFramework::Testing);
        if !has_testing {
            continue;
        }

        let is_test_file = files_by_id
            .get(file_id)
            .is_some_and(|f| f.relative_path.ends_with("_test.go"));
        if !is_test_file {
            continue;
        }

        let functions = match functions_by_file.get(file_id) {
            Some(fns) => fns,
            None => continue,
        };

        for function in functions {
            if !function.is_test {
                continue;
            }

            let file_key = file_stable_key(db, *file_id);
            let stable_key = entrypoint_stable_key(
                interner,
                &file_key,
                "go.testing",
                "Test",
                &format!("{}", function.id.0),
                &span_key(&function.span),
            );

            entrypoints.push(EntrypointFact {
                id: EntrypointId(0),
                language: Language::Go,
                framework_id: "go.testing".to_string(),
                kind: EntrypointKind::Test,
                target_function: function.id,
                target_symbol: None,
                registration_span: function.span.clone(),
                registration_file: *file_id,
                trigger_metadata: TriggerMetadata {
                    method: None,
                    path: None,
                    tool_name: None,
                    event_name: None,
                    test_name: Some(function.name.clone()),
                },
                trust_boundary_link: None,
                precision: EntrypointPrecision::ResolvedStatic,
                provenance: EntrypointProvenance::NativeRecognizer,
                confidence: EntrypointConfidence::High,
                status: EntrypointStatus::Resolved,
                provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
                stable_key,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Unresolved framework detection (D-10)
// ---------------------------------------------------------------------------

/// If a chi import was detected in a file but no entrypoints were recognized
/// from that file, emit an UnresolvedFrameworkFact. This catches cases where
/// the router is used in an unrecognized pattern.
fn emit_unresolved_for_unused_frameworks(
    interner: &crate::internal_core::StableKeyInterner,
    db: &impl AnalysisHost,
    file_frameworks: &BTreeMap<FileId, Vec<(&str, GoFramework, Span)>>,
    entrypoints: &[EntrypointFact],
    unresolved: &mut Vec<UnresolvedFrameworkFact>,
) {
    // Collect files that have entrypoints already recognized
    let files_with_entrypoints: BTreeSet<(FileId, &str)> = entrypoints
        .iter()
        .map(|ep| (ep.registration_file, ep.framework_id.as_str()))
        .collect();

    for (file_id, frameworks) in file_frameworks {
        for (framework_id, framework, import_span) in frameworks {
            // Testing entrypoints are naming-convention based, not call-site based;
            // skip this check for testing
            if *framework == GoFramework::Testing {
                continue;
            }

            if !files_with_entrypoints.contains(&(*file_id, framework_id)) {
                let file_key = file_stable_key(db, *file_id);
                let stable_key = unresolved_stable_key(
                    interner,
                    &file_key,
                    framework_id,
                    "UnrecognizedPattern",
                    &span_key(import_span),
                );

                let framework_label = match framework {
                    GoFramework::NetHttp => "net/http",
                    GoFramework::Chi => "chi",
                    GoFramework::Cobra => "cobra",
                    GoFramework::Testing => "testing",
                };

                unresolved.push(UnresolvedFrameworkFact {
                    id: UnresolvedFrameworkId(0),
                    language: Language::Go,
                    file: *file_id,
                    span: import_span.clone(),
                    framework_id: framework_id.to_string(),
                    reason: UnresolvedFrameworkReason::UnrecognizedPattern,
                    evidence: format!(
                        "{} import detected but no matching registration patterns found",
                        framework_label
                    ),
                    scope_description: format!(
                        "{} framework usage without recognized entrypoint patterns",
                        framework_label
                    ),
                    precision: EntrypointPrecision::Conservative,
                    provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
                    stable_key,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Try to resolve the handler function from a call site. For Go call sites
/// with 2+ arguments, the handler is typically the last argument (for
/// HandleFunc) or the second argument (for chi).
fn resolve_handler_function(
    db: &impl AnalysisHost,
    functions_by_id: &BTreeMap<FunctionId, &FunctionFact>,
    site: &CallSiteFact,
) -> Option<FunctionId> {
    let functions_in_file = functions_by_id
        .values()
        .copied()
        .filter(|function| function.file == site.file)
        .collect::<Vec<_>>();

    for name in handler_argument_names(db, site) {
        if let Some(function) = find_function_by_name(&functions_in_file, &name) {
            return Some(function.id);
        }
    }

    for argument in site.arguments.iter().rev() {
        let Some(place) = db.mir_places().iter().find(|place| place.id == *argument) else {
            continue;
        };
        if let PlaceRoot::Temporary { ordinal, .. } = place.root
            && let Some(function) = functions_in_file
                .iter()
                .find(|function| function.span.start_byte == ordinal)
        {
            return Some(function.id);
        }
    }

    if site.arguments.is_empty() && functions_by_id.contains_key(&site.caller) {
        return Some(site.caller);
    }
    None
}

/// Try to extract a string literal from the first argument of a call site.
/// This is used for route paths in http.HandleFunc("/path", handler) and
/// chi r.Get("/path", handler) calls.
fn extract_first_arg_literal(db: &impl AnalysisHost, site: &CallSiteFact) -> Option<String> {
    if let Some(first_arg) = site.arguments.first()
        && let Some(place) = db.mir_places().iter().find(|p| p.id == *first_arg)
        && let PlaceRoot::Unknown { evidence } = &place.root
        && let Some(value) = unquote_literal(evidence)
    {
        return Some(value);
    }

    call_argument_texts(db, site)
        .first()
        .and_then(|argument| unquote_literal(argument))
}

fn handler_argument_names(db: &impl AnalysisHost, site: &CallSiteFact) -> Vec<String> {
    let mut names = Vec::new();

    for argument in site.arguments.iter().rev() {
        let Some(place) = db.mir_places().iter().find(|place| place.id == *argument) else {
            continue;
        };
        if let Some(name) = handler_name_from_place_root(&place.root) {
            names.push(name);
        }
    }

    for argument in call_argument_texts(db, site).into_iter().rev() {
        if let Some(name) = handler_name_from_argument_text(&argument) {
            names.push(name);
        }
    }

    names
}

fn handler_name_from_place_root(root: &PlaceRoot) -> Option<String> {
    match root {
        PlaceRoot::Local { name, .. }
        | PlaceRoot::Global { name, .. }
        | PlaceRoot::Unknown { evidence: name } => handler_name_from_argument_text(name),
        _ => None,
    }
}

fn find_function_by_name<'a>(
    functions: &'a [&'a FunctionFact],
    candidate: &str,
) -> Option<&'a FunctionFact> {
    functions
        .iter()
        .copied()
        .find(|function| function.name == candidate)
        .or_else(|| {
            functions
                .iter()
                .copied()
                .find(|function| function.name.rsplit('.').next() == Some(candidate))
        })
}

fn handler_name_from_argument_text(argument: &str) -> Option<String> {
    let mut candidate = argument.trim();
    if unquote_literal(candidate).is_some() {
        return None;
    }
    if candidate.contains("=>") || candidate.starts_with("func(") || candidate.starts_with("func ")
    {
        return None;
    }
    if let Some(inner) = single_call_argument(candidate) {
        candidate = inner;
    }
    candidate = candidate.trim_start_matches('&').trim();
    let candidate = candidate
        .rsplit(['.', ':'])
        .next()
        .unwrap_or(candidate)
        .trim();
    if is_identifier(candidate) {
        Some(candidate.to_string())
    } else {
        None
    }
}

fn single_call_argument(argument: &str) -> Option<&str> {
    let open = argument.find('(')?;
    if !argument.ends_with(')') {
        return None;
    }
    let inner = &argument[open + 1..argument.len() - 1];
    let args = split_top_level_arguments(inner);
    if args.len() == 1 {
        Some(args[0].trim())
    } else {
        None
    }
}

fn call_argument_texts(db: &impl AnalysisHost, site: &CallSiteFact) -> Vec<String> {
    let Some(call_text) = call_source_text(db, site) else {
        return Vec::new();
    };
    let Some(open) = call_text.find('(') else {
        return Vec::new();
    };
    let Some(close) = call_text.rfind(')') else {
        return Vec::new();
    };
    if close <= open {
        return Vec::new();
    }
    split_top_level_arguments(&call_text[open + 1..close])
        .into_iter()
        .map(str::trim)
        .filter(|argument| !argument.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn call_source_text<'a>(db: &'a impl AnalysisHost, site: &CallSiteFact) -> Option<&'a str> {
    let file = db.files().iter().find(|file| file.id == site.file)?;
    file.source
        .get(site.span.start_byte as usize..site.span.end_byte as usize)
}

fn split_top_level_arguments(input: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut start = 0;
    let mut depth = 0_i32;
    let mut quote = None;
    let mut escaped = false;

    for (index, ch) in input.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' | '`' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                args.push(&input[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }

    args.push(&input[start..]);
    args
}

fn unquote_literal(value: &str) -> Option<String> {
    let value = value.trim();
    let mut chars = value.chars();
    let first = chars.next()?;
    let last = value.chars().last()?;
    if matches!(first, '"' | '\'' | '`') && first == last && value.len() >= 2 {
        Some(value[first.len_utf8()..value.len() - last.len_utf8()].to_string())
    } else {
        None
    }
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn file_stable_key(db: &impl AnalysisHost, file: FileId) -> String {
    db.metadata_for(FactRef::new(FactFamily::SourceFile, u64::from(file.0)))
        .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
        .or_else(|| {
            db.files()
                .iter()
                .find(|source_file| source_file.id == file)
                .map(|source_file| source_file.relative_path.replace('\\', "/"))
        })
        .unwrap_or_else(|| format!("<missing-file:{}>", file.0))
}

fn span_key(span: &Span) -> String {
    format!(
        "{}:{}..{}:{}@{}..{}",
        span.start_line,
        span.start_col,
        span.end_line,
        span.end_col,
        span.start_byte,
        span.end_byte
    )
}

fn entrypoint_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    file_key: &str,
    framework_id: &str,
    kind: &str,
    target_function_key: &str,
    registration_span: &str,
) -> crate::internal_core::StableKeyId {
    stable_key_from_parts(
        interner,
        FactFamily::Entrypoint,
        &[
            ("language", "Go".to_string()),
            ("file_key", file_key.to_string()),
            ("framework_id", framework_id.to_string()),
            ("kind", kind.to_string()),
            ("target_function_key", target_function_key.to_string()),
            ("registration_span", registration_span.to_string()),
        ],
    )
}

fn unresolved_stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    file_key: &str,
    framework_id: &str,
    reason: &str,
    span: &str,
) -> crate::internal_core::StableKeyId {
    stable_key_from_parts(
        interner,
        FactFamily::UnresolvedFramework,
        &[
            ("language", "Go".to_string()),
            ("file_key", file_key.to_string()),
            ("framework_id", framework_id.to_string()),
            ("reason", reason.to_string()),
            ("span", span.to_string()),
        ],
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::{FunctionFact, ImportFact};
    use crate::analysis_neutral::AnalysisHost;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallCallee, CallPrecision, CallSiteFact, CallSyntaxKind, CallTargetStatus,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::ids::{CallSiteId, MirBodyId, MirOpId, PlaceId};
    use crate::internal_core::{FileId, FunctionId, ImportId, Language, Span};
    use std::path::PathBuf;

    fn span(file: FileId, line: u32, start_byte: u32) -> Span {
        Span::new(file, start_byte, start_byte + 4, line, 1, line, 5)
    }

    fn span_for_needle(file: FileId, source: &str, needle: &str) -> Span {
        let start = source.find(needle).expect("needle in source") as u32;
        Span::new(
            file,
            start,
            start + needle.len() as u32,
            1,
            1,
            1,
            1 + needle.len() as u32,
        )
    }

    fn add_go_file(db: &mut impl AnalysisHost, path: &str) -> FileId {
        db.add_source_file(
            PathBuf::from(path),
            path.to_string(),
            Language::Go,
            "".into(),
            "content".to_string(),
        )
    }

    fn add_go_file_with_source(db: &mut impl AnalysisHost, path: &str, source: &str) -> FileId {
        db.add_source_file(
            PathBuf::from(path),
            path.to_string(),
            Language::Go,
            source.into(),
            "content".to_string(),
        )
    }

    fn add_function(db: &mut impl AnalysisHost, file: FileId, name: &str, line: u32) -> FunctionId {
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(999),
            file,
            name.to_string(),
            span(file, line, line * 10),
            Language::Go,
            GO_TEST_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix)),
            true,
            1,
            Vec::new(),
        ))
    }

    fn add_import(db: &mut impl AnalysisHost, file: FileId, path: &str, line: u32) -> ImportId {
        db.push_import(ImportFact::new(
            ImportId::from_raw(999),
            file,
            None,
            path.to_string(),
            span(file, line, line * 10),
            Language::Go,
        ))
    }

    fn make_call_site(
        id: u64,
        file: FileId,
        caller: FunctionId,
        callee: CallCallee,
        kind: CallSyntaxKind,
        line: u32,
    ) -> CallSiteFact {
        CallSiteFact {
            in_throw: false,
            id: CallSiteId(id),
            language: Language::Go,
            file,
            caller,
            owner_symbol: None,
            body: MirBodyId(0),
            operation: MirOpId(id),
            span: span(file, line, line * 10),
            kind,
            callee,
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Unresolved,
            precision: CallPrecision::Conservative,
            stable_key: crate::internal_core::StableKeyId(id as u32),
        }
    }

    #[test]
    fn net_http_handlefunc_produces_http_route_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_go_file(&mut db, "main.go");
        let handler = add_function(&mut db, file, "handler", 5);
        add_import(&mut db, file, "net/http", 1);

        // Simulate call site for http.HandleFunc
        let site = make_call_site(
            1,
            file,
            handler,
            CallCallee::Member {
                base: PlaceId(0),
                property: "HandleFunc".to_string(),
            },
            CallSyntaxKind::Member,
            10,
        );
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call facts should store");

        let output = recognize_go_entrypoints(&db);

        assert!(!output.entrypoints.is_empty(), "should produce entrypoints");
        let ep = &output.entrypoints[0];
        assert_eq!(ep.kind, EntrypointKind::HttpRoute);
        assert_eq!(ep.framework_id, "go.net_http");
        assert_eq!(ep.language, Language::Go);
        assert_eq!(ep.precision, EntrypointPrecision::Heuristic);
        assert_eq!(ep.provenance, EntrypointProvenance::NativeRecognizer);
        assert_eq!(ep.status, EntrypointStatus::Resolved);
        let stable_key = db.resolve_stable_key(ep.stable_key);
        assert!(stable_key.contains("10:Entrypoint"));
        assert!(stable_key.contains("8:language=2:Go"));
    }

    #[test]
    fn net_http_handlefunc_resolves_handler_argument_and_source_path() {
        let mut db = LocalAnalysisDb::new();
        let source = r#"
package main

import "net/http"

func handler(w http.ResponseWriter, r *http.Request) {}

func setup() {
	http.HandleFunc("/api/users/{id}", handler)
}
"#;
        let file = add_go_file_with_source(&mut db, "main.go", source);
        let handler = add_function(&mut db, file, "handler", 5);
        let setup = add_function(&mut db, file, "setup", 7);
        add_import(&mut db, file, "net/http", 1);

        let needle = r#"http.HandleFunc("/api/users/{id}", handler)"#;
        let mut site = make_call_site(
            1,
            file,
            setup,
            CallCallee::Member {
                base: PlaceId(0),
                property: "HandleFunc".to_string(),
            },
            CallSyntaxKind::Member,
            8,
        );
        site.span = span_for_needle(file, source, needle);
        site.arguments = vec![PlaceId(404)];
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call facts should store");

        let output = recognize_go_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        assert_eq!(output.entrypoints[0].target_function, handler);
        assert_eq!(
            output.entrypoints[0].trigger_metadata.path,
            Some("/api/users/{id}".to_string())
        );
    }

    #[test]
    fn net_http_handle_produces_http_route_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_go_file(&mut db, "main.go");
        let handler = add_function(&mut db, file, "myHandler", 5);
        add_import(&mut db, file, "net/http", 1);

        let site = make_call_site(
            1,
            file,
            handler,
            CallCallee::Member {
                base: PlaceId(0),
                property: "Handle".to_string(),
            },
            CallSyntaxKind::Member,
            10,
        );
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call facts should store");

        let output = recognize_go_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        assert_eq!(output.entrypoints[0].kind, EntrypointKind::HttpRoute);
        assert_eq!(output.entrypoints[0].framework_id, "go.net_http");
    }

    #[test]
    fn chi_router_method_produces_http_route_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_go_file(&mut db, "routes.go");
        let handler = add_function(&mut db, file, "getUsers", 5);
        add_import(&mut db, file, "github.com/go-chi/chi/v5", 1);

        let site = make_call_site(
            1,
            file,
            handler,
            CallCallee::Member {
                base: PlaceId(0),
                property: "Get".to_string(),
            },
            CallSyntaxKind::Member,
            10,
        );
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call facts should store");

        let output = recognize_go_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        let ep = &output.entrypoints[0];
        assert_eq!(ep.kind, EntrypointKind::HttpRoute);
        assert_eq!(ep.framework_id, "go.chi");
        assert_eq!(ep.trigger_metadata.method, Some("GET".to_string()));
    }

    #[test]
    fn chi_multiple_methods_produce_separate_entrypoints() {
        let mut db = LocalAnalysisDb::new();
        let file = add_go_file(&mut db, "routes.go");
        let handler_get = add_function(&mut db, file, "getUsers", 5);
        let handler_post = add_function(&mut db, file, "createUser", 10);
        add_import(&mut db, file, "github.com/go-chi/chi", 1);

        let get_site = make_call_site(
            1,
            file,
            handler_get,
            CallCallee::Member {
                base: PlaceId(0),
                property: "Get".to_string(),
            },
            CallSyntaxKind::Member,
            15,
        );
        let post_site = make_call_site(
            2,
            file,
            handler_post,
            CallCallee::Member {
                base: PlaceId(0),
                property: "Post".to_string(),
            },
            CallSyntaxKind::Member,
            16,
        );
        db.replace_call_facts(CallOutput {
            sites: vec![get_site, post_site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call facts should store");

        let output = recognize_go_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 2);
        let methods: Vec<Option<&str>> = output
            .entrypoints
            .iter()
            .map(|ep| ep.trigger_metadata.method.as_deref())
            .collect();
        assert!(methods.contains(&Some("GET")));
        assert!(methods.contains(&Some("POST")));
    }

    #[test]
    fn go_test_functions_produce_test_entrypoints() {
        let mut db = LocalAnalysisDb::new();
        let file = add_go_file(&mut db, "handler_test.go");
        let _test_func = add_function(&mut db, file, "TestHandler", 5);
        let _benchmark_func = add_function(&mut db, file, "BenchmarkHandler", 15);
        let _example_func = add_function(&mut db, file, "ExampleHandler", 25);
        let _fuzz_func = add_function(&mut db, file, "FuzzHandler", 35);
        let _helper = add_function(&mut db, file, "helperFunc", 45); // should be ignored
        add_import(&mut db, file, "testing", 1);

        let output = recognize_go_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 4);
        for ep in &output.entrypoints {
            assert_eq!(ep.kind, EntrypointKind::Test);
            assert_eq!(ep.framework_id, "go.testing");
            assert_eq!(ep.precision, EntrypointPrecision::ResolvedStatic);
            assert!(ep.trigger_metadata.test_name.is_some());
        }
        let names: Vec<&str> = output
            .entrypoints
            .iter()
            .filter_map(|ep| ep.trigger_metadata.test_name.as_deref())
            .collect();
        assert!(names.contains(&"TestHandler"));
        assert!(names.contains(&"BenchmarkHandler"));
        assert!(names.contains(&"ExampleHandler"));
        assert!(names.contains(&"FuzzHandler"));
    }

    #[test]
    fn go_test_entrypoints_use_adapter_test_classification() {
        let mut db = LocalAnalysisDb::new();
        let file = add_go_file(&mut db, "handler_test.go");
        add_function(&mut db, file, "TestReal", 5);
        let helper = db.push_function(FunctionFact::new(
            FunctionId::from_raw(999),
            file,
            "TestHelper".to_string(),
            span(file, 15, 150),
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));
        add_import(&mut db, file, "testing", 1);

        let output = recognize_go_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        assert_ne!(output.entrypoints[0].target_function, helper);
        assert_eq!(
            output.entrypoints[0].precision,
            EntrypointPrecision::ResolvedStatic
        );
    }

    #[test]
    fn testing_import_in_non_test_file_is_ignored() {
        let mut db = LocalAnalysisDb::new();
        let file = add_go_file(&mut db, "main.go"); // not _test.go
        add_function(&mut db, file, "TestFoo", 5);
        add_import(&mut db, file, "testing", 1);

        let output = recognize_go_entrypoints(&db);

        // testing import in non-test file should not produce test entrypoints
        assert!(
            output.entrypoints.is_empty(),
            "testing import in non-_test.go file should not produce entrypoints"
        );
    }

    #[test]
    fn cobra_command_produces_cli_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_go_file(&mut db, "cmd/root.go");
        let handler = add_function(&mut db, file, "rootCmd", 5);
        add_import(&mut db, file, "github.com/spf13/cobra", 1);

        let site = make_call_site(
            1,
            file,
            handler,
            CallCallee::Member {
                base: PlaceId(0),
                property: "AddCommand".to_string(),
            },
            CallSyntaxKind::Member,
            10,
        );
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call facts should store");

        let output = recognize_go_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        let ep = &output.entrypoints[0];
        assert_eq!(ep.kind, EntrypointKind::CliCommand);
        assert_eq!(ep.framework_id, "go.cobra");
        assert_eq!(ep.precision, EntrypointPrecision::Conservative);
    }

    #[test]
    fn unrecognized_framework_import_produces_unresolved_fact() {
        let mut db = LocalAnalysisDb::new();
        let file = add_go_file(&mut db, "main.go");
        add_import(&mut db, file, "github.com/gin-gonic/gin", 1);

        let output = recognize_go_entrypoints(&db);

        assert!(
            !output.unresolved.is_empty(),
            "should produce unresolved fact"
        );
        let ur = &output.unresolved[0];
        assert_eq!(ur.language, Language::Go);
        assert_eq!(
            ur.reason,
            UnresolvedFrameworkReason::UnsupportedFrameworkVersion
        );
        assert!(ur.evidence.contains("gin"));
        assert!(ur.framework_id.contains("gin"));
    }

    #[test]
    fn chi_import_without_matching_calls_produces_unrecognized_pattern() {
        let mut db = LocalAnalysisDb::new();
        let file = add_go_file(&mut db, "routes.go");
        add_function(&mut db, file, "setup", 5);
        add_import(&mut db, file, "github.com/go-chi/chi/v5", 1);
        // No call sites matching chi patterns

        let output = recognize_go_entrypoints(&db);

        assert!(output.entrypoints.is_empty());
        let chi_unresolved: Vec<_> = output
            .unresolved
            .iter()
            .filter(|u| u.framework_id == "go.chi")
            .collect();
        assert!(
            !chi_unresolved.is_empty(),
            "chi import without matching calls should produce unresolved"
        );
        assert_eq!(
            chi_unresolved[0].reason,
            UnresolvedFrameworkReason::UnrecognizedPattern
        );
    }

    #[test]
    fn stable_keys_use_semantic_stable_key_with_fact_family_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_go_file(&mut db, "handler_test.go");
        add_function(&mut db, file, "TestFoo", 5);
        add_import(&mut db, file, "testing", 1);

        let output = recognize_go_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        let key = db.resolve_stable_key(output.entrypoints[0].stable_key);
        assert!(
            key.contains("10:Entrypoint"),
            "key should contain FactFamily::Entrypoint"
        );
        assert!(
            key.contains("8:language=2:Go"),
            "key should contain language=Go"
        );
        assert!(
            key.contains("12:framework_id=10:go.testing"),
            "key should contain framework_id"
        );
    }

    #[test]
    fn output_is_sorted_by_stable_key() {
        let mut db = LocalAnalysisDb::new();
        let file = add_go_file(&mut db, "handler_test.go");
        add_function(&mut db, file, "TestZZZ", 5);
        add_function(&mut db, file, "TestAAA", 10);
        add_function(&mut db, file, "TestMMM", 15);
        add_import(&mut db, file, "testing", 1);

        let output = recognize_go_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 3);
        let keys: Vec<_> = output
            .entrypoints
            .iter()
            .map(|ep| db.resolve_stable_key(ep.stable_key))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "entrypoints should be sorted by stable key");
    }
}
