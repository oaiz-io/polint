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

pub struct TsRecognizerOutput {
    pub entrypoints: Vec<EntrypointFact>,
    pub unresolved: Vec<UnresolvedFrameworkFact>,
}

// ---------------------------------------------------------------------------
// Framework detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TsFramework {
    Express,
    McpSdk,
    Jest,
    Vitest,
    Mocha,
    Commander,
    Yargs,
}

fn classify_import(path: &str) -> Option<(&'static str, TsFramework)> {
    match path {
        "express" => Some(("ts.express", TsFramework::Express)),
        "commander" => Some(("ts.commander", TsFramework::Commander)),
        "yargs" => Some(("ts.yargs", TsFramework::Yargs)),
        "jest" | "@jest/globals" => Some(("ts.jest", TsFramework::Jest)),
        "vitest" => Some(("ts.vitest", TsFramework::Vitest)),
        "mocha" => Some(("ts.mocha", TsFramework::Mocha)),
        _ if path.starts_with("@modelcontextprotocol/") => {
            Some(("ts.mcp_sdk", TsFramework::McpSdk))
        }
        _ => None,
    }
}

/// Import paths that look like TS/JS web/HTTP framework packages but are not
/// covered by default recognizers (Express, MCP SDK). Per D-10 these should
/// produce UnresolvedFrameworkFact rows.
fn is_unrecognized_framework_import(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let markers = [
        "fastify",
        "koa",
        "hapi",
        "nest",
        "@nestjs",
        "next",
        "nuxt",
        "remix",
        "sveltekit",
        "astro",
    ];
    // Exclude known frameworks we handle natively
    if path == "express"
        || path.starts_with("@modelcontextprotocol/")
        || path == "commander"
        || path == "yargs"
        || path == "jest"
        || path == "@jest/globals"
        || path == "vitest"
        || path == "mocha"
    {
        return false;
    }
    markers.iter().any(|marker| lower.contains(marker))
}

// ---------------------------------------------------------------------------
// Main recognizer entry point
// ---------------------------------------------------------------------------

pub fn recognize_ts_entrypoints(db: &impl AnalysisHost) -> TsRecognizerOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let _files_by_id: BTreeMap<FileId, &crate::analysis_api::SourceFile> =
        db.files().iter().map(|file| (file.id, file)).collect();
    let functions_by_id: BTreeMap<FunctionId, &FunctionFact> =
        db.functions().iter().map(|f| (f.id, f)).collect();

    // Build per-file framework imports index
    let mut file_frameworks: BTreeMap<FileId, Vec<(&str, TsFramework, Span)>> = BTreeMap::new();
    let mut unrecognized_imports: Vec<(FileId, String, Span)> = Vec::new();

    for import in db.imports() {
        if !matches!(import.language, Language::TypeScript | Language::JavaScript) {
            continue;
        }
        if let Some((framework_id, framework)) = classify_import(&import.path) {
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

    // 1. Detect Express entrypoints from call sites
    recognize_express_entrypoints(
        db,
        &functions_by_id,
        &file_frameworks,
        &mut entrypoints,
        &mut unresolved,
    );

    // 2. Detect MCP SDK entrypoints from call sites
    recognize_mcp_entrypoints(
        db,
        &functions_by_id,
        &file_frameworks,
        &mut entrypoints,
        &mut unresolved,
    );

    // 3. Detect test framework entrypoints (jest, vitest, mocha)
    recognize_test_entrypoints(
        interner,
        db,
        &functions_by_id,
        &file_frameworks,
        &mut entrypoints,
    );

    // 4. Detect CLI framework entrypoints (commander, yargs)
    recognize_cli_entrypoints(
        db,
        &functions_by_id,
        &file_frameworks,
        &mut entrypoints,
        &mut unresolved,
    );

    // 5. Detect top-level framework registrations that do not currently lower
    // into function-owned call-site facts.
    recognize_source_level_entrypoints(db, &functions_by_id, &file_frameworks, &mut entrypoints);

    // 6. Emit UnresolvedFrameworkFact for unrecognized TS/JS framework imports (D-10)
    for (file, import_path, span) in &unrecognized_imports {
        let file_key = file_stable_key(db, *file);
        let stable_key = stable_key_from_parts(
            interner,
            FactFamily::UnresolvedFramework,
            &[
                ("language", "TypeScript".to_string()),
                ("file_key", file_key),
                ("import_path", import_path.clone()),
            ],
        );

        unresolved.push(UnresolvedFrameworkFact {
            id: UnresolvedFrameworkId(0),
            language: Language::TypeScript,
            file: *file,
            span: span.clone(),
            framework_id: format!("ts.unknown:{import_path}"),
            reason: UnresolvedFrameworkReason::UnsupportedFrameworkVersion,
            evidence: format!("TS/JS import \"{import_path}\" detected"),
            scope_description: format!("unrecognized TS/JS framework import: {import_path}"),
            precision: EntrypointPrecision::Conservative,
            provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
            stable_key,
        });
    }

    // 7. Check for framework imports without matching patterns (D-10)
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
    let mut seen_entrypoints = BTreeSet::new();
    entrypoints.retain(|entrypoint| {
        seen_entrypoints.insert(entrypoint_semantic_identity(interner, entrypoint))
    });
    unresolved.sort_by(|fact, other| interner.compare_canonical(fact.stable_key, other.stable_key));

    TsRecognizerOutput {
        entrypoints,
        unresolved,
    }
}

// ---------------------------------------------------------------------------
// Express recognizer (D-07)
// ---------------------------------------------------------------------------

/// Express HTTP method names recognized on app/router method calls.
const EXPRESS_HTTP_METHODS: &[&str] = &["get", "post", "put", "delete", "patch", "options", "head"];

fn recognize_express_entrypoints(
    db: &impl AnalysisHost,
    functions_by_id: &BTreeMap<FunctionId, &FunctionFact>,
    file_frameworks: &BTreeMap<FileId, Vec<(&str, TsFramework, Span)>>,
    entrypoints: &mut Vec<EntrypointFact>,
    _unresolved: &mut Vec<UnresolvedFrameworkFact>,
) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    for site in db.call_sites() {
        if !matches!(site.language, Language::TypeScript | Language::JavaScript) {
            continue;
        }

        let frameworks = match file_frameworks.get(&site.file) {
            Some(fw) => fw,
            None => continue,
        };

        let has_express = frameworks
            .iter()
            .any(|(_, fw, _)| *fw == TsFramework::Express);
        if !has_express {
            continue;
        }

        match &site.callee {
            // Match app.get/post/put/delete/patch/options/head calls
            CallCallee::Member { property, .. }
                if EXPRESS_HTTP_METHODS.contains(&property.as_str())
                    && matches!(
                        site.kind,
                        CallSyntaxKind::Member | CallSyntaxKind::StaticMember
                    ) =>
            {
                let route_path = extract_http_route_path(db, site);
                let handler_function = resolve_handler_function(db, functions_by_id, site);
                let file_key = file_stable_key(db, site.file);
                let method = property.to_uppercase();

                if let Some(target_function) = handler_function {
                    let stable_key = entrypoint_stable_key(
                        interner,
                        "TypeScript",
                        &file_key,
                        "ts.express",
                        "HttpRoute",
                        &format!("{}", target_function.0),
                        &span_key(&site.span),
                    );

                    entrypoints.push(EntrypointFact {
                        id: EntrypointId(0),
                        language: site.language,
                        framework_id: "ts.express".to_string(),
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
                }
            }

            // Match app.use calls for middleware
            CallCallee::Member { property, .. }
                if property == "use"
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
                        "TypeScript",
                        &file_key,
                        "ts.express",
                        "HttpMiddleware",
                        &format!("{}", target_function.0),
                        &span_key(&site.span),
                    );

                    entrypoints.push(EntrypointFact {
                        id: EntrypointId(0),
                        language: site.language,
                        framework_id: "ts.express".to_string(),
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

            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// MCP TypeScript SDK recognizer (D-07)
// ---------------------------------------------------------------------------

/// MCP SDK registration method names.
const MCP_METHODS: &[(&str, EntrypointKind)] = &[
    ("tool", EntrypointKind::McpTool),
    ("resource", EntrypointKind::McpResource),
    ("prompt", EntrypointKind::McpPrompt),
];

fn recognize_mcp_entrypoints(
    db: &impl AnalysisHost,
    functions_by_id: &BTreeMap<FunctionId, &FunctionFact>,
    file_frameworks: &BTreeMap<FileId, Vec<(&str, TsFramework, Span)>>,
    entrypoints: &mut Vec<EntrypointFact>,
    _unresolved: &mut Vec<UnresolvedFrameworkFact>,
) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    for site in db.call_sites() {
        if !matches!(site.language, Language::TypeScript | Language::JavaScript) {
            continue;
        }

        let frameworks = match file_frameworks.get(&site.file) {
            Some(fw) => fw,
            None => continue,
        };

        let has_mcp_sdk = frameworks
            .iter()
            .any(|(_, fw, _)| *fw == TsFramework::McpSdk);
        if !has_mcp_sdk {
            continue;
        }

        if let CallCallee::Member { property, .. } = &site.callee {
            if !matches!(
                site.kind,
                CallSyntaxKind::Member | CallSyntaxKind::StaticMember
            ) {
                continue;
            }

            if let Some((_, kind)) = MCP_METHODS
                .iter()
                .find(|(method, _)| *method == property.as_str())
            {
                let tool_name = extract_first_arg_literal(db, site);
                let handler_function = resolve_handler_function(db, functions_by_id, site);
                let file_key = file_stable_key(db, site.file);
                let kind_label = format!("{kind:?}");

                if let Some(target_function) = handler_function {
                    let stable_key = entrypoint_stable_key(
                        interner,
                        "TypeScript",
                        &file_key,
                        "ts.mcp_sdk",
                        &kind_label,
                        &format!("{}", target_function.0),
                        &span_key(&site.span),
                    );

                    entrypoints.push(EntrypointFact {
                        id: EntrypointId(0),
                        language: site.language,
                        framework_id: "ts.mcp_sdk".to_string(),
                        kind: *kind,
                        target_function,
                        target_symbol: None,
                        registration_span: site.span.clone(),
                        registration_file: site.file,
                        trigger_metadata: TriggerMetadata {
                            method: None,
                            path: None,
                            tool_name,
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
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Test framework recognizer (D-08)
// ---------------------------------------------------------------------------

/// Test function identifiers that indicate test entrypoints.
const TEST_CALL_NAMES: &[&str] = &["describe", "it", "test"];

fn is_test_framework(fw: TsFramework) -> bool {
    matches!(
        fw,
        TsFramework::Jest | TsFramework::Vitest | TsFramework::Mocha
    )
}

fn framework_id_for_test(fw: TsFramework) -> &'static str {
    match fw {
        TsFramework::Jest => "ts.jest",
        TsFramework::Vitest => "ts.vitest",
        TsFramework::Mocha => "ts.mocha",
        _ => "ts.test",
    }
}

fn recognize_test_entrypoints(
    interner: &crate::internal_core::StableKeyInterner,
    db: &impl AnalysisHost,
    _functions_by_id: &BTreeMap<FunctionId, &FunctionFact>,
    file_frameworks: &BTreeMap<FileId, Vec<(&str, TsFramework, Span)>>,
    entrypoints: &mut Vec<EntrypointFact>,
) {
    for site in db.call_sites() {
        if !matches!(site.language, Language::TypeScript | Language::JavaScript) {
            continue;
        }

        let frameworks = match file_frameworks.get(&site.file) {
            Some(fw) => fw,
            None => continue,
        };

        // Find the test framework import for this file
        let test_fw = frameworks.iter().find(|(_, fw, _)| is_test_framework(*fw));
        let test_fw = match test_fw {
            Some((_, fw, _)) => *fw,
            None => continue,
        };

        // Match describe/it/test identifier calls
        let is_test_call = match &site.callee {
            CallCallee::Identifier { name, .. } => TEST_CALL_NAMES.contains(&name.as_str()),
            // Also match member calls like vitest.describe or jest.test
            CallCallee::Member { property, .. } => TEST_CALL_NAMES.contains(&property.as_str()),
            _ => false,
        };

        if !is_test_call {
            continue;
        }

        let test_name = extract_first_arg_literal(db, site);
        let file_key = file_stable_key(db, site.file);
        let fw_id = framework_id_for_test(test_fw);

        let callee_name = match &site.callee {
            CallCallee::Identifier { name, .. } => name.as_str(),
            CallCallee::Member { property, .. } => property.as_str(),
            _ => "test",
        };

        let stable_key = entrypoint_stable_key(
            interner,
            "TypeScript",
            &file_key,
            fw_id,
            "Test",
            callee_name,
            &span_key(&site.span),
        );

        entrypoints.push(EntrypointFact {
            id: EntrypointId(0),
            language: site.language,
            framework_id: fw_id.to_string(),
            kind: EntrypointKind::Test,
            target_function: site.caller,
            target_symbol: None,
            registration_span: site.span.clone(),
            registration_file: site.file,
            trigger_metadata: TriggerMetadata {
                method: None,
                path: None,
                tool_name: None,
                event_name: None,
                test_name,
            },
            trust_boundary_link: None,
            precision: EntrypointPrecision::SetupAware,
            provenance: EntrypointProvenance::NativeRecognizer,
            confidence: EntrypointConfidence::High,
            status: EntrypointStatus::Resolved,
            provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
            stable_key,
        });
    }
}

// ---------------------------------------------------------------------------
// CLI framework recognizer (D-09)
// ---------------------------------------------------------------------------

fn recognize_cli_entrypoints(
    db: &impl AnalysisHost,
    functions_by_id: &BTreeMap<FunctionId, &FunctionFact>,
    file_frameworks: &BTreeMap<FileId, Vec<(&str, TsFramework, Span)>>,
    entrypoints: &mut Vec<EntrypointFact>,
    _unresolved: &mut Vec<UnresolvedFrameworkFact>,
) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    for site in db.call_sites() {
        if !matches!(site.language, Language::TypeScript | Language::JavaScript) {
            continue;
        }

        let frameworks = match file_frameworks.get(&site.file) {
            Some(fw) => fw,
            None => continue,
        };

        let has_commander = frameworks
            .iter()
            .any(|(_, fw, _)| *fw == TsFramework::Commander);
        let has_yargs = frameworks
            .iter()
            .any(|(_, fw, _)| *fw == TsFramework::Yargs);

        if !has_commander && !has_yargs {
            continue;
        }

        match &site.callee {
            // Commander: program.command("name") or .action(handler)
            CallCallee::Member { property, .. }
                if has_commander
                    && (property == "command" || property == "action")
                    && matches!(
                        site.kind,
                        CallSyntaxKind::Member | CallSyntaxKind::StaticMember
                    ) =>
            {
                let command_name = extract_first_arg_literal(db, site);
                let handler_function = resolve_handler_function(db, functions_by_id, site);
                let file_key = file_stable_key(db, site.file);

                if let Some(target_function) = handler_function {
                    let stable_key = entrypoint_stable_key(
                        interner,
                        "TypeScript",
                        &file_key,
                        "ts.commander",
                        "CliCommand",
                        &format!("{}", target_function.0),
                        &span_key(&site.span),
                    );

                    entrypoints.push(EntrypointFact {
                        id: EntrypointId(0),
                        language: site.language,
                        framework_id: "ts.commander".to_string(),
                        kind: EntrypointKind::CliCommand,
                        target_function,
                        target_symbol: None,
                        registration_span: site.span.clone(),
                        registration_file: site.file,
                        trigger_metadata: TriggerMetadata {
                            method: None,
                            path: None,
                            tool_name: command_name,
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

            // Yargs: yargs.command("name", ...)
            CallCallee::Member { property, .. }
                if has_yargs
                    && property == "command"
                    && matches!(
                        site.kind,
                        CallSyntaxKind::Member | CallSyntaxKind::StaticMember
                    ) =>
            {
                let command_name = extract_first_arg_literal(db, site);
                let handler_function = resolve_handler_function(db, functions_by_id, site);
                let file_key = file_stable_key(db, site.file);

                if let Some(target_function) = handler_function {
                    let stable_key = entrypoint_stable_key(
                        interner,
                        "TypeScript",
                        &file_key,
                        "ts.yargs",
                        "CliCommand",
                        &format!("{}", target_function.0),
                        &span_key(&site.span),
                    );

                    entrypoints.push(EntrypointFact {
                        id: EntrypointId(0),
                        language: site.language,
                        framework_id: "ts.yargs".to_string(),
                        kind: EntrypointKind::CliCommand,
                        target_function,
                        target_symbol: None,
                        registration_span: site.span.clone(),
                        registration_file: site.file,
                        trigger_metadata: TriggerMetadata {
                            method: None,
                            path: None,
                            tool_name: command_name,
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

            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Source-level fallback recognizer
// ---------------------------------------------------------------------------

fn entrypoint_semantic_identity(
    interner: &crate::internal_core::StableKeyInterner,
    entrypoint: &EntrypointFact,
) -> crate::internal_core::StableKeyId {
    stable_key_from_parts(
        interner,
        FactFamily::Entrypoint,
        &[
            (
                "language",
                language_label_for_stable_key(entrypoint.language).to_string(),
            ),
            (
                "registration_file",
                entrypoint.registration_file.0.to_string(),
            ),
            ("framework_id", entrypoint.framework_id.clone()),
            ("kind", format!("{:?}", entrypoint.kind)),
            ("target_function", entrypoint.target_function.0.to_string()),
            (
                "target_symbol",
                entrypoint
                    .target_symbol
                    .map(|symbol| symbol.0.to_string())
                    .unwrap_or_default(),
            ),
            (
                "method",
                entrypoint
                    .trigger_metadata
                    .method
                    .clone()
                    .unwrap_or_default(),
            ),
            (
                "path",
                entrypoint.trigger_metadata.path.clone().unwrap_or_default(),
            ),
            (
                "tool_name",
                entrypoint
                    .trigger_metadata
                    .tool_name
                    .clone()
                    .unwrap_or_default(),
            ),
            (
                "event_name",
                entrypoint
                    .trigger_metadata
                    .event_name
                    .clone()
                    .unwrap_or_default(),
            ),
            (
                "test_name",
                entrypoint
                    .trigger_metadata
                    .test_name
                    .clone()
                    .unwrap_or_default(),
            ),
        ],
    )
}

fn recognize_source_level_entrypoints(
    db: &impl AnalysisHost,
    functions_by_id: &BTreeMap<FunctionId, &FunctionFact>,
    file_frameworks: &BTreeMap<FileId, Vec<(&str, TsFramework, Span)>>,
    entrypoints: &mut Vec<EntrypointFact>,
) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let functions_by_file = functions_by_id.values().copied().fold(
        BTreeMap::<FileId, Vec<&FunctionFact>>::new(),
        |mut map, function| {
            map.entry(function.file).or_default().push(function);
            map
        },
    );

    for (file_id, frameworks) in file_frameworks {
        let Some(file) = db.files().iter().find(|file| file.id == *file_id) else {
            continue;
        };
        let functions = functions_by_file
            .get(file_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);

        if frameworks
            .iter()
            .any(|(_, framework, _)| *framework == TsFramework::Express)
        {
            recognize_source_express_calls(interner, file, functions, entrypoints);
        }
        if frameworks
            .iter()
            .any(|(_, framework, _)| *framework == TsFramework::McpSdk)
        {
            recognize_source_mcp_calls(interner, file, functions, entrypoints);
        }
    }
}

fn recognize_source_express_calls(
    interner: &crate::internal_core::StableKeyInterner,
    file: &crate::analysis_api::SourceFile,
    functions: &[&FunctionFact],
    entrypoints: &mut Vec<EntrypointFact>,
) {
    let receivers = framework_receiver_names(&file.source, SourceFramework::Express);
    if receivers.is_empty() {
        return;
    }

    for method in EXPRESS_HTTP_METHODS {
        for call in source_member_calls(&file.source, method, &receivers) {
            let args = split_top_level_arguments(&call.arguments);
            let Some(handler) = resolve_source_handler(functions, &args) else {
                continue;
            };
            let method_label = method.to_uppercase();
            let path = args.first().and_then(|argument| unquote_literal(argument));
            push_source_entrypoint(
                interner,
                file,
                handler,
                call.start,
                call.end,
                "ts.express",
                EntrypointKind::HttpRoute,
                EntrypointConfidence::High,
                TriggerMetadata {
                    method: Some(method_label),
                    path,
                    tool_name: None,
                    event_name: None,
                    test_name: None,
                },
                entrypoints,
            );
        }
    }

    for call in source_express_route_chain_calls(&file.source, &receivers) {
        let args = split_top_level_arguments(&call.arguments);
        let Some(handler) = resolve_source_handler(functions, &args) else {
            continue;
        };
        push_source_entrypoint(
            interner,
            file,
            handler,
            call.start,
            call.end,
            "ts.express",
            EntrypointKind::HttpRoute,
            EntrypointConfidence::High,
            TriggerMetadata {
                method: Some(call.method),
                path: Some(call.path),
                tool_name: None,
                event_name: None,
                test_name: None,
            },
            entrypoints,
        );
    }

    for call in source_member_calls(&file.source, "use", &receivers) {
        let args = split_top_level_arguments(&call.arguments);
        let Some(handler) = resolve_source_handler(functions, &args) else {
            continue;
        };
        push_source_entrypoint(
            interner,
            file,
            handler,
            call.start,
            call.end,
            "ts.express",
            EntrypointKind::HttpMiddleware,
            EntrypointConfidence::Medium,
            TriggerMetadata::empty(),
            entrypoints,
        );
    }
}

fn recognize_source_mcp_calls(
    interner: &crate::internal_core::StableKeyInterner,
    file: &crate::analysis_api::SourceFile,
    functions: &[&FunctionFact],
    entrypoints: &mut Vec<EntrypointFact>,
) {
    let receivers = framework_receiver_names(&file.source, SourceFramework::McpSdk);
    if receivers.is_empty() {
        return;
    }

    for (method, kind) in MCP_METHODS {
        for call in source_member_calls(&file.source, method, &receivers) {
            let args = split_top_level_arguments(&call.arguments);
            let Some(handler) = resolve_source_handler(functions, &args) else {
                continue;
            };
            let tool_name = args.first().and_then(|argument| unquote_literal(argument));
            push_source_entrypoint(
                interner,
                file,
                handler,
                call.start,
                call.end,
                "ts.mcp_sdk",
                *kind,
                EntrypointConfidence::High,
                TriggerMetadata {
                    method: None,
                    path: None,
                    tool_name,
                    event_name: None,
                    test_name: None,
                },
                entrypoints,
            );
        }
    }
}

struct SourceMemberCall {
    start: u32,
    end: u32,
    arguments: String,
}

struct SourceRouteChainCall {
    start: u32,
    end: u32,
    method: String,
    path: String,
    arguments: String,
}

#[derive(Debug, Clone, Copy)]
enum SourceFramework {
    Express,
    McpSdk,
}

fn source_member_calls(
    source: &str,
    method: &str,
    receivers: &BTreeSet<String>,
) -> Vec<SourceMemberCall> {
    if receivers.is_empty() {
        return Vec::new();
    }

    let needle = format!(".{method}(");
    let mut calls = Vec::new();
    let mut offset = 0;
    let code_mask = source_code_mask(source);

    while let Some(relative_index) = source[offset..].find(&needle) {
        let member_index = offset + relative_index;
        if !code_mask.get(member_index).copied().unwrap_or(false)
            || !receiver_before_member(source, member_index).is_some_and(|receiver| {
                receivers.contains(receiver.rsplit('.').next().unwrap_or(receiver))
            })
        {
            offset = member_index + needle.len();
            continue;
        }
        let open = member_index + needle.len() - 1;
        let Some(close) = matching_close_paren(source, open) else {
            offset = open + 1;
            continue;
        };
        let start = member_expression_start(source, member_index);
        calls.push(SourceMemberCall {
            start: start as u32,
            end: (close + 1) as u32,
            arguments: source[open + 1..close].to_string(),
        });
        offset = close + 1;
    }

    calls
}

fn source_express_route_chain_calls(
    source: &str,
    receivers: &BTreeSet<String>,
) -> Vec<SourceRouteChainCall> {
    let mut chains = Vec::new();

    for route_call in source_member_calls(source, "route", receivers) {
        let route_args = split_top_level_arguments(&route_call.arguments);
        let Some(path) = route_args
            .first()
            .and_then(|argument| unquote_literal(argument))
        else {
            continue;
        };
        let mut offset = route_call.end as usize;

        loop {
            let remaining = &source[offset..];
            let trimmed = remaining.trim_start();
            offset += remaining.len() - trimmed.len();
            let Some(rest) = source.get(offset..) else {
                break;
            };
            let Some(method) = EXPRESS_HTTP_METHODS
                .iter()
                .find(|method| rest.starts_with(&format!(".{method}(")))
            else {
                break;
            };
            let member_index = offset;
            let open = member_index + method.len() + 1;
            let Some(close) = matching_close_paren(source, open) else {
                break;
            };
            chains.push(SourceRouteChainCall {
                start: route_call.start,
                end: (close + 1) as u32,
                method: method.to_uppercase(),
                path: path.clone(),
                arguments: source[open + 1..close].to_string(),
            });
            offset = close + 1;
        }
    }

    chains
}

fn framework_receiver_names(source: &str, framework: SourceFramework) -> BTreeSet<String> {
    let needles: &[&str] = match framework {
        SourceFramework::Express => &["express("],
        SourceFramework::McpSdk => &["new McpServer("],
    };
    let mut receivers = BTreeSet::new();
    let code_mask = source_code_mask(source);

    for needle in needles {
        let mut offset = 0;
        while let Some(relative_index) = source[offset..].find(needle) {
            let index = offset + relative_index;
            if code_mask.get(index).copied().unwrap_or(false)
                && let Some(receiver) = assignment_lhs_identifier(source, index)
            {
                receivers.insert(receiver.to_string());
            }
            offset = index + needle.len();
        }
    }

    receivers
}

fn assignment_lhs_identifier(source: &str, rhs_start: usize) -> Option<&str> {
    let before_rhs = source.get(..rhs_start)?;
    let equals = before_rhs.rfind('=')?;
    let statement_start = before_rhs[..equals]
        .rfind([';', '\n', '{', '}'])
        .map(|index| index + 1)
        .unwrap_or(0);
    let lhs = before_rhs[statement_start..equals].trim();
    let end = lhs
        .char_indices()
        .rev()
        .find_map(|(index, ch)| {
            if is_identifier_char(ch) {
                None
            } else {
                Some(index + ch.len_utf8())
            }
        })
        .unwrap_or(0);
    let candidate = lhs[end..].trim();
    if is_identifier(candidate) {
        Some(candidate)
    } else {
        None
    }
}

fn receiver_before_member(source: &str, member_index: usize) -> Option<&str> {
    let before_member = source.get(..member_index)?;
    let start = before_member
        .char_indices()
        .rev()
        .find_map(|(index, ch)| {
            if is_identifier_char(ch) || ch == '.' {
                None
            } else {
                Some(index + ch.len_utf8())
            }
        })
        .unwrap_or(0);
    let receiver = before_member[start..].trim();
    if receiver.is_empty() {
        None
    } else {
        Some(receiver)
    }
}

fn source_code_mask(source: &str) -> Vec<bool> {
    let mut mask = vec![true; source.len()];
    let mut chars = source.char_indices().peekable();
    let mut quote = None;
    let mut escaped = false;
    let mut block_comment = false;
    let mut line_comment = false;

    while let Some((index, ch)) = chars.next() {
        let len = ch.len_utf8();

        if line_comment {
            set_mask(&mut mask, index, len, false);
            if ch == '\n' {
                line_comment = false;
                set_mask(&mut mask, index, len, true);
            }
            continue;
        }

        if block_comment {
            set_mask(&mut mask, index, len, false);
            if ch == '*'
                && let Some(&(next_index, '/')) = chars.peek()
            {
                chars.next();
                set_mask(&mut mask, next_index, 1, false);
                block_comment = false;
            }
            continue;
        }

        if let Some(active_quote) = quote {
            set_mask(&mut mask, index, len, false);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }

        if matches!(ch, '"' | '\'' | '`') {
            quote = Some(ch);
            set_mask(&mut mask, index, len, false);
            continue;
        }

        if ch == '/'
            && let Some(&(next_index, next_ch)) = chars.peek()
        {
            if next_ch == '/' {
                line_comment = true;
                set_mask(&mut mask, index, len, false);
                chars.next();
                set_mask(&mut mask, next_index, next_ch.len_utf8(), false);
                continue;
            }
            if next_ch == '*' {
                block_comment = true;
                set_mask(&mut mask, index, len, false);
                chars.next();
                set_mask(&mut mask, next_index, next_ch.len_utf8(), false);
            }
        }
    }

    mask
}

fn set_mask(mask: &mut [bool], start: usize, len: usize, value: bool) {
    for slot in mask.iter_mut().skip(start).take(len) {
        *slot = value;
    }
}

fn matching_close_paren(source: &str, open: usize) -> Option<usize> {
    let mut depth = 0_i32;
    let mut quote = None;
    let mut escaped = false;

    for (relative, ch) in source[open..].char_indices() {
        let index = open + relative;
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
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }

    None
}

fn member_expression_start(source: &str, member_index: usize) -> usize {
    source[..member_index]
        .char_indices()
        .rev()
        .find_map(|(index, ch)| {
            if ch == '_' || ch == '$' || ch == '.' || ch.is_ascii_alphanumeric() {
                None
            } else {
                Some(index + ch.len_utf8())
            }
        })
        .unwrap_or(0)
}

fn resolve_source_handler(functions: &[&FunctionFact], args: &[&str]) -> Option<FunctionId> {
    args.iter()
        .rev()
        .filter_map(|argument| handler_name_from_argument_text(argument))
        .find_map(|name| find_function_by_name(functions, &name).map(|function| function.id))
}

#[allow(clippy::too_many_arguments)]
fn push_source_entrypoint(
    interner: &crate::internal_core::StableKeyInterner,
    file: &crate::analysis_api::SourceFile,
    handler: FunctionId,
    start_byte: u32,
    end_byte: u32,
    framework_id: &str,
    kind: EntrypointKind,
    confidence: EntrypointConfidence,
    trigger_metadata: TriggerMetadata,
    entrypoints: &mut Vec<EntrypointFact>,
) {
    let file_key = file.relative_path.replace('\\', "/");
    let span = file.span_from_byte_range(start_byte as usize, end_byte as usize);
    let kind_label = format!("{kind:?}");
    let stable_key = entrypoint_stable_key(
        interner,
        language_label_for_stable_key(file.language),
        &file_key,
        framework_id,
        &kind_label,
        &format!("{}", handler.0),
        &span_key(&span),
    );

    entrypoints.push(EntrypointFact {
        id: EntrypointId(0),
        language: file.language,
        framework_id: framework_id.to_string(),
        kind,
        target_function: handler,
        target_symbol: None,
        registration_span: span,
        registration_file: file.id,
        trigger_metadata,
        trust_boundary_link: None,
        precision: EntrypointPrecision::Heuristic,
        provenance: EntrypointProvenance::NativeRecognizer,
        confidence,
        status: EntrypointStatus::Resolved,
        provider_id: ENTRYPOINTS_PROVIDER_ID.to_string(),
        stable_key,
    });
}

// ---------------------------------------------------------------------------
// Unresolved framework detection (D-10)
// ---------------------------------------------------------------------------

/// If a framework import was detected in a file but no entrypoints were
/// recognized from that file, emit an UnresolvedFrameworkFact. This catches
/// cases where the framework is used in an unrecognized pattern.
fn emit_unresolved_for_unused_frameworks(
    interner: &crate::internal_core::StableKeyInterner,
    db: &impl AnalysisHost,
    file_frameworks: &BTreeMap<FileId, Vec<(&str, TsFramework, Span)>>,
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
            // Test framework entrypoints are based on call-site matching which we
            // do cover; skip this check for test frameworks since they may have
            // non-test imports in the same file
            if is_test_framework(*framework) {
                continue;
            }

            if !files_with_entrypoints.contains(&(*file_id, framework_id)) {
                let file_key = file_stable_key(db, *file_id);
                let stable_key = stable_key_from_parts(
                    interner,
                    FactFamily::UnresolvedFramework,
                    &[
                        ("language", "TypeScript".to_string()),
                        ("file_key", file_key),
                        ("framework_id", framework_id.to_string()),
                        ("reason", "UnrecognizedPattern".to_string()),
                        ("span", span_key(import_span)),
                    ],
                );

                let framework_label = match framework {
                    TsFramework::Express => "Express",
                    TsFramework::McpSdk => "MCP TypeScript SDK",
                    TsFramework::Commander => "commander",
                    TsFramework::Yargs => "yargs",
                    TsFramework::Jest => "jest",
                    TsFramework::Vitest => "vitest",
                    TsFramework::Mocha => "mocha",
                };

                unresolved.push(UnresolvedFrameworkFact {
                    id: UnresolvedFrameworkId(0),
                    language: Language::TypeScript,
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

/// Try to resolve the handler function from a call site. Uses the caller
/// function as a fallback target when deeper handler resolution cannot
/// resolve the specific handler function.
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
/// This is used for route paths, tool names, test names, and command names.
fn extract_http_route_path(db: &impl AnalysisHost, site: &CallSiteFact) -> Option<String> {
    extract_chained_route_path(db, site).or_else(|| extract_first_arg_literal(db, site))
}

fn extract_chained_route_path(db: &impl AnalysisHost, site: &CallSiteFact) -> Option<String> {
    let call_text = call_source_text(db, site)?;
    let route_index = call_text.find(".route(")?;
    let open = route_index + ".route".len();
    let close = matching_close_paren(call_text, open)?;
    let args = split_top_level_arguments(&call_text[open + 1..close]);
    args.first().and_then(|argument| unquote_literal(argument))
}

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
    if candidate.contains("=>")
        || candidate.starts_with("function")
        || candidate.starts_with("async ")
    {
        return None;
    }
    if let Some(inner) = single_call_argument(candidate) {
        candidate = inner;
    }
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
    (first == '_' || first == '$' || first.is_ascii_alphabetic()) && chars.all(is_identifier_char)
}

fn is_identifier_char(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphanumeric()
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
    language: &str,
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
            ("language", language.to_string()),
            ("file_key", file_key.to_string()),
            ("framework_id", framework_id.to_string()),
            ("kind", kind.to_string()),
            ("target_function_key", target_function_key.to_string()),
            ("registration_span", registration_span.to_string()),
        ],
    )
}

fn language_label_for_stable_key(language: Language) -> &'static str {
    match language {
        Language::Go => "Go",
        Language::TypeScript => "TypeScript",
        Language::Tsx => "Tsx",
        Language::JavaScript => "JavaScript",
        Language::Jsx => "Jsx",
        Language::Unknown => "Unknown",
        _ => "Unknown",
    }
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

    fn add_ts_file(db: &mut impl AnalysisHost, path: &str) -> FileId {
        db.add_source_file(
            PathBuf::from(path),
            path.to_string(),
            Language::TypeScript,
            "".into(),
            "content".to_string(),
        )
    }

    fn add_ts_file_with_source(db: &mut impl AnalysisHost, path: &str, source: &str) -> FileId {
        db.add_source_file(
            PathBuf::from(path),
            path.to_string(),
            Language::TypeScript,
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
            Language::TypeScript,
            false,
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
            Language::TypeScript,
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
            language: Language::TypeScript,
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
    fn express_app_get_produces_http_route_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/app.ts");
        let handler = add_function(&mut db, file, "getUsers", 5);
        add_import(&mut db, file, "express", 1);

        let site = make_call_site(
            1,
            file,
            handler,
            CallCallee::Member {
                base: PlaceId(0),
                property: "get".to_string(),
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

        let output = recognize_ts_entrypoints(&db);

        assert!(!output.entrypoints.is_empty(), "should produce entrypoints");
        let ep = &output.entrypoints[0];
        assert_eq!(ep.kind, EntrypointKind::HttpRoute);
        assert_eq!(ep.framework_id, "ts.express");
        assert_eq!(ep.language, Language::TypeScript);
        assert_eq!(ep.precision, EntrypointPrecision::Heuristic);
        assert_eq!(ep.provenance, EntrypointProvenance::NativeRecognizer);
        assert_eq!(ep.status, EntrypointStatus::Resolved);
        assert_eq!(ep.trigger_metadata.method, Some("GET".to_string()));
    }

    #[test]
    fn express_get_resolves_handler_argument_and_source_path() {
        let mut db = LocalAnalysisDb::new();
        let source = r#"
import express from "express";

function getUsers(req, res) {
  res.json([]);
}

function setup(app) {
  app.get("/api/users/:id", getUsers);
}
"#;
        let file = add_ts_file_with_source(&mut db, "src/app.ts", source);
        let handler = add_function(&mut db, file, "getUsers", 4);
        let setup = add_function(&mut db, file, "setup", 8);
        add_import(&mut db, file, "express", 1);

        let needle = r#"app.get("/api/users/:id", getUsers)"#;
        let mut site = make_call_site(
            1,
            file,
            setup,
            CallCallee::Member {
                base: PlaceId(0),
                property: "get".to_string(),
            },
            CallSyntaxKind::Member,
            9,
        );
        site.span = span_for_needle(file, source, needle);
        site.arguments = vec![PlaceId(404)];
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call facts should store");

        let output = recognize_ts_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        assert_eq!(output.entrypoints[0].target_function, handler);
        assert_eq!(
            output.entrypoints[0].trigger_metadata.path,
            Some("/api/users/:id".to_string())
        );
    }

    #[test]
    fn source_fallback_ignores_comments_strings_and_unrelated_receivers() {
        let mut db = LocalAnalysisDb::new();
        let source = r#"
import express from "express";

const app = express();
const cache = new Map();

function getUsers(req, res) {
  res.json([]);
}

// app.get("/commented", getUsers);
const example = 'app.get("/string", getUsers)';
cache.get("key");
app.get("/api/users/:id", getUsers);
"#;
        let file = add_ts_file_with_source(&mut db, "src/app.ts", source);
        let handler = add_function(&mut db, file, "getUsers", 6);
        add_import(&mut db, file, "express", 1);

        let output = recognize_ts_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        assert_eq!(output.entrypoints[0].target_function, handler);
        assert_eq!(
            output.entrypoints[0].trigger_metadata.path,
            Some("/api/users/:id".to_string())
        );
    }

    #[test]
    fn express_route_chain_source_fallback_preserves_path_and_handler() {
        let mut db = LocalAnalysisDb::new();
        let source = r#"
import express from "express";

const app = express();

function getUsers(req, res) {
  res.json([]);
}

app.route("/api/users/:id").get(getUsers);
"#;
        let file = add_ts_file_with_source(&mut db, "src/app.ts", source);
        let handler = add_function(&mut db, file, "getUsers", 6);
        add_import(&mut db, file, "express", 1);

        let output = recognize_ts_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        assert_eq!(output.entrypoints[0].target_function, handler);
        assert_eq!(
            output.entrypoints[0].trigger_metadata.method,
            Some("GET".to_string())
        );
        assert_eq!(
            output.entrypoints[0].trigger_metadata.path,
            Some("/api/users/:id".to_string())
        );
    }

    #[test]
    fn express_app_post_produces_http_route_with_post_method() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/app.ts");
        let handler = add_function(&mut db, file, "createUser", 5);
        add_import(&mut db, file, "express", 1);

        let site = make_call_site(
            1,
            file,
            handler,
            CallCallee::Member {
                base: PlaceId(0),
                property: "post".to_string(),
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

        let output = recognize_ts_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        assert_eq!(
            output.entrypoints[0].trigger_metadata.method,
            Some("POST".to_string())
        );
    }

    #[test]
    fn express_app_use_produces_http_middleware_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/app.ts");
        let handler = add_function(&mut db, file, "authMiddleware", 5);
        add_import(&mut db, file, "express", 1);

        let site = make_call_site(
            1,
            file,
            handler,
            CallCallee::Member {
                base: PlaceId(0),
                property: "use".to_string(),
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

        let output = recognize_ts_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        assert_eq!(output.entrypoints[0].kind, EntrypointKind::HttpMiddleware);
        assert_eq!(output.entrypoints[0].framework_id, "ts.express");
    }

    #[test]
    fn mcp_server_tool_produces_mcp_tool_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/server.ts");
        let handler = add_function(&mut db, file, "handleTool", 5);
        add_import(&mut db, file, "@modelcontextprotocol/sdk/server/mcp.js", 1);

        let site = make_call_site(
            1,
            file,
            handler,
            CallCallee::Member {
                base: PlaceId(0),
                property: "tool".to_string(),
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

        let output = recognize_ts_entrypoints(&db);

        assert!(
            !output.entrypoints.is_empty(),
            "should produce MCP tool entrypoint"
        );
        let ep = &output.entrypoints[0];
        assert_eq!(ep.kind, EntrypointKind::McpTool);
        assert_eq!(ep.framework_id, "ts.mcp_sdk");
        assert_eq!(ep.precision, EntrypointPrecision::Heuristic);
    }

    #[test]
    fn mcp_server_resource_produces_mcp_resource_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/server.ts");
        let handler = add_function(&mut db, file, "handleResource", 5);
        add_import(&mut db, file, "@modelcontextprotocol/sdk", 1);

        let site = make_call_site(
            1,
            file,
            handler,
            CallCallee::Member {
                base: PlaceId(0),
                property: "resource".to_string(),
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

        let output = recognize_ts_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        assert_eq!(output.entrypoints[0].kind, EntrypointKind::McpResource);
    }

    #[test]
    fn mcp_server_prompt_produces_mcp_prompt_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/server.ts");
        let handler = add_function(&mut db, file, "handlePrompt", 5);
        add_import(&mut db, file, "@modelcontextprotocol/sdk", 1);

        let site = make_call_site(
            1,
            file,
            handler,
            CallCallee::Member {
                base: PlaceId(0),
                property: "prompt".to_string(),
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

        let output = recognize_ts_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        assert_eq!(output.entrypoints[0].kind, EntrypointKind::McpPrompt);
    }

    #[test]
    fn jest_test_call_produces_test_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/app.test.ts");
        let test_fn = add_function(&mut db, file, "testSuite", 5);
        add_import(&mut db, file, "jest", 1);

        let site = make_call_site(
            1,
            file,
            test_fn,
            CallCallee::Identifier {
                reference: None,
                name: "test".to_string(),
            },
            CallSyntaxKind::Function,
            10,
        );
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call facts should store");

        let output = recognize_ts_entrypoints(&db);

        assert!(
            !output.entrypoints.is_empty(),
            "should produce test entrypoint"
        );
        let ep = &output.entrypoints[0];
        assert_eq!(ep.kind, EntrypointKind::Test);
        assert_eq!(ep.framework_id, "ts.jest");
        assert_eq!(ep.precision, EntrypointPrecision::SetupAware);
    }

    #[test]
    fn vitest_describe_call_produces_test_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/app.test.ts");
        let test_fn = add_function(&mut db, file, "testSuite", 5);
        add_import(&mut db, file, "vitest", 1);

        let site = make_call_site(
            1,
            file,
            test_fn,
            CallCallee::Identifier {
                reference: None,
                name: "describe".to_string(),
            },
            CallSyntaxKind::Function,
            10,
        );
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call facts should store");

        let output = recognize_ts_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        assert_eq!(output.entrypoints[0].kind, EntrypointKind::Test);
        assert_eq!(output.entrypoints[0].framework_id, "ts.vitest");
    }

    #[test]
    fn jest_globals_import_produces_test_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/app.test.ts");
        let test_fn = add_function(&mut db, file, "testSuite", 5);
        add_import(&mut db, file, "@jest/globals", 1);

        let site = make_call_site(
            1,
            file,
            test_fn,
            CallCallee::Identifier {
                reference: None,
                name: "it".to_string(),
            },
            CallSyntaxKind::Function,
            10,
        );
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call facts should store");

        let output = recognize_ts_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        assert_eq!(output.entrypoints[0].kind, EntrypointKind::Test);
        assert_eq!(output.entrypoints[0].framework_id, "ts.jest");
    }

    #[test]
    fn commander_command_produces_cli_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/cli.ts");
        let handler = add_function(&mut db, file, "main", 5);
        add_import(&mut db, file, "commander", 1);

        let site = make_call_site(
            1,
            file,
            handler,
            CallCallee::Member {
                base: PlaceId(0),
                property: "command".to_string(),
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

        let output = recognize_ts_entrypoints(&db);

        assert!(
            !output.entrypoints.is_empty(),
            "should produce CLI entrypoint"
        );
        let ep = &output.entrypoints[0];
        assert_eq!(ep.kind, EntrypointKind::CliCommand);
        assert_eq!(ep.framework_id, "ts.commander");
        assert_eq!(ep.precision, EntrypointPrecision::Conservative);
    }

    #[test]
    fn yargs_command_produces_cli_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/cli.ts");
        let handler = add_function(&mut db, file, "main", 5);
        add_import(&mut db, file, "yargs", 1);

        let site = make_call_site(
            1,
            file,
            handler,
            CallCallee::Member {
                base: PlaceId(0),
                property: "command".to_string(),
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

        let output = recognize_ts_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        assert_eq!(output.entrypoints[0].kind, EntrypointKind::CliCommand);
        assert_eq!(output.entrypoints[0].framework_id, "ts.yargs");
    }

    #[test]
    fn unrecognized_framework_import_produces_unresolved_fact() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/app.ts");
        add_import(&mut db, file, "fastify", 1);

        let output = recognize_ts_entrypoints(&db);

        assert!(
            !output.unresolved.is_empty(),
            "should produce unresolved fact"
        );
        let ur = &output.unresolved[0];
        assert_eq!(ur.language, Language::TypeScript);
        assert_eq!(
            ur.reason,
            UnresolvedFrameworkReason::UnsupportedFrameworkVersion
        );
        assert!(ur.evidence.contains("fastify"));
        assert!(ur.framework_id.contains("fastify"));
    }

    #[test]
    fn nestjs_import_produces_unresolved_fact() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/app.ts");
        add_import(&mut db, file, "@nestjs/core", 1);

        let output = recognize_ts_entrypoints(&db);

        assert!(
            !output.unresolved.is_empty(),
            "should produce unresolved fact for @nestjs"
        );
        let ur = &output.unresolved[0];
        assert!(ur.framework_id.contains("@nestjs/core"));
    }

    #[test]
    fn express_import_without_matching_calls_produces_unrecognized_pattern() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/app.ts");
        add_function(&mut db, file, "setup", 5);
        add_import(&mut db, file, "express", 1);
        // No call sites matching express patterns

        let output = recognize_ts_entrypoints(&db);

        assert!(output.entrypoints.is_empty());
        let express_unresolved: Vec<_> = output
            .unresolved
            .iter()
            .filter(|u| u.framework_id == "ts.express")
            .collect();
        assert!(
            !express_unresolved.is_empty(),
            "express import without matching calls should produce unresolved"
        );
        assert_eq!(
            express_unresolved[0].reason,
            UnresolvedFrameworkReason::UnrecognizedPattern
        );
    }

    #[test]
    fn stable_keys_use_semantic_stable_key_with_fact_family_entrypoint() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/app.ts");
        let handler = add_function(&mut db, file, "getUsers", 5);
        add_import(&mut db, file, "express", 1);

        let site = make_call_site(
            1,
            file,
            handler,
            CallCallee::Member {
                base: PlaceId(0),
                property: "get".to_string(),
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

        let output = recognize_ts_entrypoints(&db);

        assert_eq!(output.entrypoints.len(), 1);
        let key = db.resolve_stable_key(output.entrypoints[0].stable_key);
        assert!(
            key.contains("10:Entrypoint"),
            "key should contain FactFamily::Entrypoint"
        );
        assert!(
            key.contains("8:language=10:TypeScript"),
            "key should contain language=TypeScript"
        );
        assert!(
            key.contains("12:framework_id=10:ts.express"),
            "key should contain framework_id"
        );
    }

    #[test]
    fn output_is_sorted_by_stable_key() {
        let mut db = LocalAnalysisDb::new();
        let file = add_ts_file(&mut db, "src/app.ts");
        let handler1 = add_function(&mut db, file, "getUsers", 5);
        let handler2 = add_function(&mut db, file, "createUser", 10);
        let handler3 = add_function(&mut db, file, "deleteUser", 15);
        add_import(&mut db, file, "express", 1);

        let site1 = make_call_site(
            1,
            file,
            handler1,
            CallCallee::Member {
                base: PlaceId(0),
                property: "get".to_string(),
            },
            CallSyntaxKind::Member,
            20,
        );
        let site2 = make_call_site(
            2,
            file,
            handler2,
            CallCallee::Member {
                base: PlaceId(0),
                property: "post".to_string(),
            },
            CallSyntaxKind::Member,
            21,
        );
        let site3 = make_call_site(
            3,
            file,
            handler3,
            CallCallee::Member {
                base: PlaceId(0),
                property: "delete".to_string(),
            },
            CallSyntaxKind::Member,
            22,
        );
        db.replace_call_facts(CallOutput {
            sites: vec![site1, site2, site3],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call facts should store");

        let output = recognize_ts_entrypoints(&db);

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
