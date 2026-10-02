//! The Go semantic sidecar's call facts, as typed inputs for the calls layer.
//!
//! The sidecar's SSA program knows, per call expression, the exact static callee,
//! the candidate callees of an interface or function-value call from its call
//! graph, and which call expressions are builtins or type conversions. This
//! module names those callees in polint's terms — an in-repository callee as its
//! `FunctionId`, anything else by the sidecar's identity — and keys each answer
//! by the call expression's span, which the calls layer joins to its own sites.

use std::collections::BTreeMap;

use crate::analysis_neutral::calls::facts::{CallAlgorithm, CallEdgeKind, CallPrecision};
use crate::analysis_neutral::calls::typed::{TypedCallInputs, TypedCallSite, TypedCallTarget};
use crate::core::AnalysisDb;
use crate::go::rta::inputs::{GoCoreIndex, matching_core_function_indexed};
use crate::go::semantic::facts::{
    GoAbstractCallee, GoCallEdgeAlgorithm, GoCallMode, GoSemanticCallEdgeFact, GoSemanticCallStatus,
};
use crate::internal_core::{FunctionId, Language, StableKeyId};

/// The typed inputs for every Go call expression the sidecar described.
/// Empty when the sidecar did not run or described nothing.
pub(crate) fn go_typed_call_inputs(db: &AnalysisDb) -> TypedCallInputs {
    let mut inputs = TypedCallInputs::new(Language::Go);
    if db.go_semantic_callsites().is_empty()
        && db.go_semantic_builtin_calls().is_empty()
        && db.go_semantic_conversions().is_empty()
    {
        return inputs;
    }
    let functions = GoFunctionNames::new(db);
    let edges_by_site = db
        .go_semantic_call_edges()
        .iter()
        .map(|edges| (edges.callsite_stable_key, edges))
        .collect::<BTreeMap<StableKeyId, &GoSemanticCallEdgeFact>>();

    for callsite in db.go_semantic_callsites() {
        let (Some(file), Some(span)) = (callsite.file, callsite.span.as_ref()) else {
            continue;
        };
        // A position-only row has no call expression to join to.
        if span.start_byte >= span.end_byte {
            continue;
        }
        let started_by = match callsite.mode {
            GoCallMode::Go => Some(CallEdgeKind::Spawn),
            GoCallMode::Defer => Some(CallEdgeKind::Deferred),
            GoCallMode::Call => None,
        };
        let targets = match callsite.status {
            GoSemanticCallStatus::ResolvedStatic => {
                let Some(callee) = callsite.static_callee.as_deref() else {
                    continue;
                };
                vec![TypedCallTarget {
                    function: functions.resolve(callee, callsite.static_callee_origin.as_deref()),
                    name: format!("go:func:{callee}"),
                    algorithm: CallAlgorithm::GoStatic,
                    precision: CallPrecision::Exact,
                    edge_kind: started_by.unwrap_or(if callsite.receiver_type.is_some() {
                        CallEdgeKind::MethodDirect
                    } else {
                        CallEdgeKind::Direct
                    }),
                }]
            }
            GoSemanticCallStatus::UnresolvedDynamic => {
                let Some(edges) = edges_by_site.get(&callsite.stable_key) else {
                    continue;
                };
                // The sidecar sets a receiver type on a dynamic site only for an
                // interface call; a function-value call has none.
                let edge_kind = started_by.unwrap_or(if callsite.receiver_type.is_some() {
                    CallEdgeKind::Method
                } else {
                    CallEdgeKind::FunctionValue
                });
                dynamic_targets(edges, &functions, edge_kind)
            }
            // Builtins are described by the syntax-level builtin rows below, which
            // also cover the builtins SSA lowers to instructions other than calls.
            GoSemanticCallStatus::Unsupported => continue,
        };
        inputs.insert(
            file,
            span.start_byte,
            span.end_byte,
            TypedCallSite::Targets(targets),
        );
    }
    for builtin in db.go_semantic_builtin_calls() {
        if let (Some(file), Some(span)) = (builtin.file, builtin.span.as_ref()) {
            inputs.insert(
                file,
                span.start_byte,
                span.end_byte,
                TypedCallSite::Builtin(format!("go:builtin:{}", builtin.name)),
            );
        }
    }
    for conversion in db.go_semantic_conversions() {
        if let (Some(file), Some(span)) = (conversion.file, conversion.span.as_ref()) {
            inputs.insert(
                file,
                span.start_byte,
                span.end_byte,
                TypedCallSite::Conversion(format!("go:conversion:{}", conversion.type_name)),
            );
        }
    }
    inputs
}

/// The typed targets of one dynamic call site: its concrete candidates, or the
/// abstract callee a type-hierarchy answer names instead.
fn dynamic_targets(
    edges: &GoSemanticCallEdgeFact,
    functions: &GoFunctionNames,
    edge_kind: CallEdgeKind,
) -> Vec<TypedCallTarget> {
    let (algorithm, precision) = match edges.algorithm {
        GoCallEdgeAlgorithm::Vta => (CallAlgorithm::GoVta, CallPrecision::SetupAware),
        GoCallEdgeAlgorithm::Cha => (CallAlgorithm::GoCha, CallPrecision::Conservative),
        GoCallEdgeAlgorithm::TypeHierarchy => {
            (CallAlgorithm::TypeHierarchy, CallPrecision::Conservative)
        }
    };
    if let Some(abstract_callee) = &edges.abstract_callee {
        let name = match abstract_callee {
            GoAbstractCallee::InterfaceMethod(method) => format!("go:interface-method:{method}"),
            GoAbstractCallee::Signature(signature) => format!("go:func-value:{signature}"),
        };
        return vec![TypedCallTarget {
            function: None,
            name,
            algorithm,
            precision,
            edge_kind,
        }];
    }
    edges
        .callees
        .iter()
        .map(|callee| TypedCallTarget {
            function: functions.resolve(&callee.name, callee.origin.as_deref()),
            name: format!("go:func:{}", callee.name),
            algorithm,
            precision,
            edge_kind,
        })
        .collect()
}

/// The sidecar's function identities (`pkg.F`, `(*pkg.T).M`) mapped to the
/// polint functions they declare.
struct GoFunctionNames {
    by_identity: BTreeMap<String, FunctionId>,
}

impl GoFunctionNames {
    fn new(db: &AnalysisDb) -> Self {
        let index = GoCoreIndex::build(db);
        let by_identity = db
            .go_semantic_functions()
            .iter()
            .filter_map(|function| {
                matching_core_function_indexed(&index, function)
                    .map(|core| (function.qualified.clone(), core.id))
            })
            .collect();
        Self { by_identity }
    }

    /// The polint function a callee is, if it has a declaration polint parsed.
    ///
    /// An instantiation of a generic function resolves through its generic
    /// origin, and a bound method value (`(T).M$bound`) or a thunk to the method
    /// it calls. A function literal (`pkg.F$1`) resolves to no function: polint
    /// lowers it as part of the function that declares it, so its calls are
    /// already that function's calls, while a call *of* the literal is not a
    /// call of the declaring function. Naming the declaring function would let
    /// every caller of a callback-taking helper appear to call each function
    /// that passes it a literal, and everything those functions call.
    fn resolve(&self, callee: &str, origin: Option<&str>) -> Option<FunctionId> {
        if is_function_literal(callee) {
            return None;
        }
        [Some(callee), origin]
            .into_iter()
            .flatten()
            .find_map(|name| {
                self.by_identity
                    .get(name)
                    .or_else(|| self.by_identity.get(declaring_function(name)))
                    .copied()
            })
    }
}

/// `(T).M$bound` and `(T).M$thunk` without the synthetic suffix the SSA builder
/// appends to a bound method value or a thunk.
fn declaring_function(name: &str) -> &str {
    name.find('$').map_or(name, |index| &name[..index])
}

/// Whether the SSA builder named a function literal: the declaring function's
/// name followed by `$` and the literal's ordinal (`pkg.F$1`, `pkg.F$1$2`).
fn is_function_literal(name: &str) -> bool {
    name.split('$')
        .nth(1)
        .is_some_and(|suffix| suffix.starts_with(|c: char| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{declaring_function, go_typed_call_inputs, is_function_literal};
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::calls::facts::{CallAlgorithm, CallEdgeKind, CallPrecision};
    use crate::analysis_neutral::calls::typed::TypedCallSite;
    use crate::core::AnalysisDb;
    use crate::go::semantic::facts::{
        GoAbstractCallee, GoCallEdgeAlgorithm, GoCallMode, GoSemanticBuiltinCallFact,
        GoSemanticBuiltinCallId, GoSemanticCallEdgeFact, GoSemanticCallEdgeId,
        GoSemanticCallStatus, GoSemanticCallsiteFact, GoSemanticCallsiteId,
        GoSemanticConversionFact, GoSemanticConversionId, GoSemanticFunctionFact,
        GoSemanticFunctionId, GoSemanticFunctionKind,
    };
    use crate::go::semantic::store::GoSemanticFactsOutput;
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    fn span(file: FileId, start: u32, end: u32) -> Span {
        Span::new(file, start, end, 1, start + 1, 1, end + 1)
    }

    fn callsite(
        db: &AnalysisDb,
        file: FileId,
        key: &str,
        (start, end): (u32, u32),
        static_callee: Option<&str>,
        receiver_type: Option<&str>,
        mode: GoCallMode,
    ) -> GoSemanticCallsiteFact {
        GoSemanticCallsiteFact {
            id: GoSemanticCallsiteId(0),
            stable_key: db.stable_key_interner().intern(key),
            package_id: "example.com/app".to_string(),
            package_path: "example.com/app".to_string(),
            caller: "example.com/app.Run".to_string(),
            static_callee: static_callee.map(str::to_string),
            static_callee_origin: None,
            receiver_type: receiver_type.map(str::to_string),
            mode,
            status: if static_callee.is_some() {
                GoSemanticCallStatus::ResolvedStatic
            } else {
                GoSemanticCallStatus::UnresolvedDynamic
            },
            reason: None,
            relative_file: Some("app/main.go".to_string()),
            file: Some(file),
            span: Some(span(file, start, end)),
        }
    }

    #[test]
    fn sidecar_rows_become_typed_answers_keyed_by_call_expression() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("app/main.go"),
            "app/main.go".to_string(),
            "package app\n".to_string(),
        );
        let helper_span = span(file, 200, 240);
        let helper = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "helper".to_string(),
            helper_span.clone(),
            Language::Go,
            false,
            false,
            1,
            Vec::new(),
        ));
        let interner = db.stable_key_interner();
        let output = GoSemanticFactsOutput {
            functions: vec![GoSemanticFunctionFact {
                id: GoSemanticFunctionId(0),
                stable_key: interner.intern("fn|helper"),
                package_id: "example.com/app".to_string(),
                package_path: "example.com/app".to_string(),
                name: "helper".to_string(),
                qualified: "example.com/app.helper".to_string(),
                signature: "func()".to_string(),
                kind: GoSemanticFunctionKind::Function,
                receiver: None,
                relative_file: Some("app/main.go".to_string()),
                file: Some(file),
                span: Some(helper_span),
            }],
            callsites: vec![
                callsite(
                    &db,
                    file,
                    "cs|static",
                    (20, 30),
                    Some("example.com/app.helper"),
                    None,
                    GoCallMode::Call,
                ),
                callsite(
                    &db,
                    file,
                    "cs|spawn",
                    (40, 50),
                    Some("fmt.Println"),
                    None,
                    GoCallMode::Go,
                ),
                callsite(
                    &db,
                    file,
                    "cs|invoke",
                    (60, 70),
                    None,
                    Some("io.Writer"),
                    GoCallMode::Call,
                ),
                callsite(
                    &db,
                    file,
                    "cs|value",
                    (72, 80),
                    None,
                    None,
                    GoCallMode::Call,
                ),
            ],
            call_edges: vec![GoSemanticCallEdgeFact {
                id: GoSemanticCallEdgeId(0),
                stable_key: interner.intern("edge|invoke"),
                package_id: "example.com/app".to_string(),
                caller: "example.com/app.Run".to_string(),
                callsite_stable_key: interner.intern("cs|invoke"),
                algorithm: GoCallEdgeAlgorithm::TypeHierarchy,
                callees: Vec::new(),
                abstract_callee: Some(GoAbstractCallee::InterfaceMethod(
                    "io.Writer.Write".to_string(),
                )),
                candidates: None,
                relative_file: Some("app/main.go".to_string()),
                file: Some(file),
            }],
            builtin_calls: vec![GoSemanticBuiltinCallFact {
                id: GoSemanticBuiltinCallId(0),
                stable_key: interner.intern("builtin|len"),
                package_id: "example.com/app".to_string(),
                name: "len".to_string(),
                relative_file: Some("app/main.go".to_string()),
                file: Some(file),
                span: Some(span(file, 82, 88)),
            }],
            conversions: vec![GoSemanticConversionFact {
                id: GoSemanticConversionId(0),
                stable_key: interner.intern("conversion|Kind"),
                package_id: "example.com/app".to_string(),
                type_name: "example.com/app.Kind".to_string(),
                relative_file: Some("app/main.go".to_string()),
                file: Some(file),
                span: Some(span(file, 90, 95)),
            }],
            ..GoSemanticFactsOutput::default()
        };
        db.replace_go_semantic_facts(output)
            .expect("go semantic facts store");

        let inputs = go_typed_call_inputs(&db);

        let Some(TypedCallSite::Targets(static_call)) = inputs.answer_at(file, 20, 30) else {
            panic!("the static call has typed targets");
        };
        assert_eq!(static_call.len(), 1);
        assert_eq!(static_call[0].function, Some(helper));
        assert_eq!(static_call[0].algorithm, CallAlgorithm::GoStatic);
        assert_eq!(static_call[0].precision, CallPrecision::Exact);
        assert_eq!(static_call[0].edge_kind, CallEdgeKind::Direct);

        let Some(TypedCallSite::Targets(spawn)) = inputs.answer_at(file, 40, 50) else {
            panic!("the go statement has typed targets");
        };
        assert_eq!(spawn[0].function, None);
        assert_eq!(spawn[0].name, "go:func:fmt.Println");
        assert_eq!(spawn[0].edge_kind, CallEdgeKind::Spawn);

        let Some(TypedCallSite::Targets(invoke)) = inputs.answer_at(file, 60, 70) else {
            panic!("the interface call has typed targets");
        };
        assert_eq!(invoke[0].name, "go:interface-method:io.Writer.Write");
        assert_eq!(invoke[0].algorithm, CallAlgorithm::TypeHierarchy);
        assert_eq!(invoke[0].precision, CallPrecision::Conservative);
        assert_eq!(invoke[0].edge_kind, CallEdgeKind::Method);

        assert_eq!(
            inputs.answer_at(file, 72, 80),
            None,
            "a dynamic call without candidates keeps the name-based resolution"
        );
        assert_eq!(
            inputs.answer_at(file, 82, 88),
            Some(&TypedCallSite::Builtin("go:builtin:len".to_string()))
        );
        assert_eq!(
            inputs.answer_at(file, 90, 95),
            Some(&TypedCallSite::Conversion(
                "go:conversion:example.com/app.Kind".to_string()
            ))
        );
    }

    #[test]
    fn method_values_resolve_to_their_method_and_function_literals_to_nothing() {
        assert_eq!(
            declaring_function("(*example.com/app.Store).Save$bound"),
            "(*example.com/app.Store).Save"
        );
        assert_eq!(declaring_function("fmt.Println"), "fmt.Println");
        assert!(is_function_literal("example.com/app.Run$1"));
        assert!(is_function_literal("(*example.com/app.Store).Save$2$1"));
        assert!(!is_function_literal("(*example.com/app.Store).Save$bound"));
        assert!(!is_function_literal("example.com/app.Run"));
    }
}
