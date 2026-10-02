    use super::rule::line_col;
    use super::*;
    use crate::analysis::evidence::facts::{
        EvidenceConfidence, EvidenceProvenance, EvidenceStatus, EvidenceValidation,
    };
    use crate::analysis::extensions::sinks::{ExtensionFactConfidence, ExtensionFactPrecision};
    use crate::analysis::ids::{CallSiteId, MirBodyId, MirOpId, PlaceId, UnsupportedId};
    use crate::analysis::mir::body::{MirBody, MirOutput, MirStatus};
    use crate::analysis::mir::op::{
        AssignMode, ConservativeAction, MirOperation, MirOperationKind, MirValue,
        UnsupportedDomain, UnsupportedPrecision, UnsupportedSemanticFact,
    };
    use crate::analysis::places::{PlaceFact, PlaceRoot, PlaceStatus};
    use crate::analysis_kernel::{
        FactConfidence, FactFamily, FactPrecision, FactRef, ValidationStatus,
    };
    use crate::diagnostics::{Diagnostic, Severity, TextRange as DiagnosticRange};
    use crate::rule_error::RuleResult;
    use crate::sdk::facts::{
        BranchObligations, FactView, Functions, GoTests, Imports, JsxAttributes, Packages,
        SourceFiles, StringLiterals, TsClasses, TsComponents,
    };
    use crate::symbol_graph::semantic::{
        AliasFact, ExportFact, GeneratedSymbolFact, ResolutionFact, ScopeFact, ScopeId, ScopeKind,
        SemanticImportFact, SemanticStatus, StableExportIdentity,
    };
    use anyhow::anyhow;
    use proptest::prelude::*;
    use std::thread;
    use std::time::Duration;

    #[derive(Clone, Copy)]
    enum TestRuleBehavior {
        Report,
        Error,
        Panic,
        MetaPanic,
    }

    #[derive(Clone, Copy)]
    struct TestRule {
        id: &'static str,
        capabilities: Capabilities,
        severity: Severity,
        message: &'static str,
        fingerprint: &'static str,
        delay: Duration,
        behavior: TestRuleBehavior,
    }

    #[test]
    fn analysis_db_solver_budget_status_tracks_not_run_and_replacements() {
        let mut db = AnalysisDb::new();

        assert_eq!(db.solver_budget_status(), BudgetStatus::NotRun);
        assert!(db.solver_budget_reasons().is_empty());

        db.replace_solver_facts(SolverOutput::default())
            .expect("within-budget solver facts");
        assert_eq!(db.solver_budget_status(), BudgetStatus::WithinBudget);
        assert!(db.solver_budget_reasons().is_empty());

        db.replace_solver_facts(SolverOutput {
            budget_status: BudgetStatus::BudgetExceeded,
            budget_reasons: BTreeSet::from(["solver.max_steps".to_string()]),
            ..SolverOutput::default()
        })
        .expect("budget-exceeded solver facts");
        assert_eq!(db.solver_budget_status(), BudgetStatus::BudgetExceeded);
        assert_eq!(
            db.solver_budget_reasons(),
            &BTreeSet::from(["solver.max_steps".to_string()])
        );
    }

    impl TestRule {
        fn report(id: &'static str, severity: Severity, fingerprint: &'static str) -> Self {
            Self {
                id,
                capabilities: Capabilities::new().syntax(),
                severity,
                message: "test diagnostic",
                fingerprint,
                delay: Duration::ZERO,
                behavior: TestRuleBehavior::Report,
            }
        }

        fn with_capabilities(mut self, capabilities: Capabilities) -> Self {
            self.capabilities = capabilities;
            self
        }

        fn with_message(mut self, message: &'static str) -> Self {
            self.message = message;
            self
        }

        fn with_delay(mut self, delay: Duration) -> Self {
            self.delay = delay;
            self
        }

        fn error(id: &'static str) -> Self {
            Self {
                id,
                capabilities: Capabilities::new(),
                severity: Severity::Error,
                message: "rule returned an error",
                fingerprint: "error",
                delay: Duration::ZERO,
                behavior: TestRuleBehavior::Error,
            }
        }

        fn panic(id: &'static str) -> Self {
            Self {
                id,
                capabilities: Capabilities::new(),
                severity: Severity::Error,
                message: "rule panicked",
                fingerprint: "panic",
                delay: Duration::ZERO,
                behavior: TestRuleBehavior::Panic,
            }
        }

        fn meta_panic() -> Self {
            Self {
                id: "examples/meta-panic",
                capabilities: Capabilities::new(),
                severity: Severity::Error,
                message: "metadata panicked",
                fingerprint: "meta-panic",
                delay: Duration::ZERO,
                behavior: TestRuleBehavior::MetaPanic,
            }
        }

        fn into_rule(self) -> Rule {
            let meta_rule = self;
            let capabilities_rule = self;
            let run_rule = self;
            Rule::from_parts(
                move || meta_rule.meta(),
                move || capabilities_rule.capabilities,
                move |_db, ctx| run_rule.run(ctx),
            )
        }

        fn meta(self) -> RuleMeta {
            if matches!(self.behavior, TestRuleBehavior::MetaPanic) {
                panic!("intentional metadata panic");
            }

            RuleMeta {
                id: self.id.to_string(),
                description: format!("Test rule {}", self.id),
                severity: self.severity,
                kind: RuleKind::Check,
            }
        }

        fn run(self, ctx: &mut RuleCtx<'_>) -> RuleResult {
            if !self.delay.is_zero() {
                thread::sleep(self.delay);
            }

            match self.behavior {
                TestRuleBehavior::Report => {
                    ctx.report(
                        Diagnostic::new(
                            self.id,
                            self.severity,
                            "src/main.go",
                            DiagnosticRange::point(1, 1),
                            self.message,
                        )
                        .with_fingerprint(self.fingerprint),
                    );
                    Ok(())
                }
                TestRuleBehavior::Error => Err(anyhow!("intentional rule error").into()),
                TestRuleBehavior::Panic => panic!("intentional rule panic"),
                TestRuleBehavior::MetaPanic => panic!("intentional metadata panic"),
            }
        }
    }

    fn test_span(file: FileId, line: u32) -> Span {
        Span::new(file, 0, 1, line, 1, line, 2)
    }

    fn test_scope(name: &str, file: FileId, status: SemanticStatus) -> ScopeFact {
        let scope_path = vec![name.to_string()];
        ScopeFact {
            id: ScopeId(99),
            language: Language::TypeScript,
            file: Some(file),
            package: None,
            module: None,
            parent: None,
            stable_key: ScopeFact::stable_key_for(&crate::core::AnalysisDb::new().stable_key_interner(),
Language::TypeScript,
&scope_path,
Some(format!("file:{}", file.0)),
None,
None,
(ScopeKind::Function, status)),
            scope_path,
            kind: ScopeKind::Function,
            status,
        }
    }

    fn test_mir_body(interner: &crate::core::StableKeyInterner, id: u64, file: FileId, stable_key: &str) -> MirBody {
        MirBody {
            id: MirBodyId(id),
            language: Language::TypeScript,
            file,
            function: FunctionId::from_raw(id),
            package: None,
            module: None,
            owner_stable_key: interner.intern(format!("function:{stable_key}")),
            span: test_span(file, 1),
            stable_key: interner.intern(stable_key.to_string()),
            status: MirStatus::Resolved,
        }
    }

    fn test_place(interner: &crate::core::StableKeyInterner, id: u64, file: FileId, stable_key: &str) -> PlaceFact {
        PlaceFact {
            id: PlaceId(id),
            language: Language::TypeScript,
            file: Some(file),
            function: Some(FunctionId::from_raw(0)),
            root: PlaceRoot::Local {
                function: FunctionId::from_raw(0),
                name: stable_key.to_string(),
            },
            projections: Vec::new(),
            stable_key: interner.intern(stable_key.to_string()),
            status: PlaceStatus::Resolved,
        }
    }

    fn test_mir_operation(interner: &crate::core::StableKeyInterner,
        id: u64,
        body: MirBodyId,
        place: PlaceId,
        value: PlaceId,
        stable_key: &str,
    ) -> MirOperation {
        MirOperation {
            id: MirOpId(id),
            body,
            ordinal: id as u32,
            span: test_span(FileId::from_raw(0), 1),
            kind: MirOperationKind::Assign {
                place,
                value: MirValue::Place(value),
                mode: AssignMode::Overwrite,
            },
            stable_key: interner.intern(stable_key.to_string()),
            status: MirStatus::Resolved,
        }
    }

    fn test_unsupported(interner: &crate::core::StableKeyInterner, stable_key: &str) -> UnsupportedSemanticFact {
        UnsupportedSemanticFact {
            id: UnsupportedId(
                stable_key
                    .bytes()
                    .fold(0_u64, |sum, byte| sum + u64::from(byte)),
            ),
            body: None,
            operation: None,
            language: Language::TypeScript,
            file: FileId::from_raw(0),
            span: test_span(FileId::from_raw(0), 1),
            construct: "dynamic-property".to_string(),
            source_evidence: "target[key]".to_string(),
            affected_places: Vec::new(),
            affected_domains: vec![UnsupportedDomain::Mir],
            conservative_action: ConservativeAction::HavocAffectedPlaces,
            precision: UnsupportedPrecision::Unsupported,
            status: MirStatus::Unsupported,
            stable_key: interner.intern(stable_key.to_string()),
        }
    }

    fn test_call_site(
        interner: &crate::core::StableKeyInterner,
        id: u64,
        file: FileId,
        caller: FunctionId,
        stable_key: &str,
    ) -> crate::analysis::calls::facts::CallSiteFact {
        use crate::analysis::calls::facts::{
            CallCallee, CallPrecision, CallSiteFact, CallSyntaxKind, CallTargetStatus,
        };

        CallSiteFact {
            in_throw: false,
            id: CallSiteId(id),
            language: Language::TypeScript,
            file,
            caller,
            owner_symbol: Some(SymbolId::from_raw(caller.0 + 100)),
            body: MirBodyId(id),
            operation: MirOpId(id),
            span: test_span(file, 1),
            kind: CallSyntaxKind::Function,
            callee: CallCallee::Identifier {
                reference: None,
                name: stable_key.to_string(),
            },
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Resolved,
            precision: CallPrecision::Exact,
            stable_key: interner.intern(stable_key),
        }
    }

    fn test_call_target(
        interner: &crate::core::StableKeyInterner,
        id: u64,
        site: CallSiteId,
        caller: FunctionId,
        stable_key: &str,
    ) -> crate::analysis::calls::facts::CallTargetFact {
        use crate::analysis::calls::facts::{
            CallAlgorithm, CallEdgeKind, CallPrecision, CallProvenance, CallTargetFact,
            CallTargetStatus,
        };

        CallTargetFact {
            id: crate::analysis::ids::CallTargetId(id),
            site,
            caller,
            target_function: Some(FunctionId::from_raw(id + 10)),
            target_symbol: Some(SymbolId::from_raw(id + 20)),
            synthetic_target: None,
            edge_kind: CallEdgeKind::Direct,
            algorithm: CallAlgorithm::DirectReference,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::Native,
            precision: CallPrecision::Exact,
            stable_key: interner.intern(stable_key),
        }
    }

    fn test_unresolved_call(
        interner: &crate::core::StableKeyInterner,
        site: CallSiteId,
        caller: FunctionId,
        stable_key: &str,
    ) -> crate::analysis::calls::facts::UnresolvedCallFact {
        use crate::analysis::calls::facts::{
            CallAlgorithm, CallPrecision, CallProvenance, CallTargetStatus, UnresolvedCallFact,
            UnresolvedCallReason,
        };

        UnresolvedCallFact {
            site,
            caller,
            status: CallTargetStatus::Unresolved,
            reason: UnresolvedCallReason::FunctionValue,
            algorithm: CallAlgorithm::SyntaxOnly,
            provenance: CallProvenance::MirShape,
            precision: CallPrecision::Unknown,
            stable_key: interner.intern(stable_key),
        }
    }

    mod call_fact_storage {
        use super::*;
        use crate::analysis::calls::store::CallOutput;

        #[test]
        fn replace_call_facts_removes_stale_rows_from_previous_run() {
            let mut db = AnalysisDb::new();
            let file = db.add_file(
                PathBuf::from("src/app.ts"),
                "src/app.ts".to_string(),
                "function app() { first(); second(); }\n".to_string(),
            );
            let interner = db.stable_key_interner();
            let first = CallOutput {
                sites: vec![test_call_site(
                    &interner,
                    1,
                    file,
                    FunctionId::from_raw(1),
                    "call-site:first",
                )],
                targets: vec![test_call_target(
                    &interner,
                    1,
                    CallSiteId(1),
                    FunctionId::from_raw(1),
                    "call-target:first",
                )],
                unresolved: vec![test_unresolved_call(
                    &interner,
                    CallSiteId(1),
                    FunctionId::from_raw(1),
                    "unresolved:first",
                )],
            };
            let second = CallOutput {
                sites: vec![test_call_site(
                    &interner,
                    2,
                    file,
                    FunctionId::from_raw(2),
                    "call-site:second",
                )],
                targets: Vec::new(),
                unresolved: Vec::new(),
            };

            db.replace_call_facts(first).expect("first call replace");
            assert!(db.call_store().is_some());
            assert_eq!(db.call_sites_by_caller(FunctionId::from_raw(1)).len(), 1);
            assert_eq!(db.call_targets_by_site(CallSiteId(1)).len(), 1);
            assert_eq!(db.outgoing_calls_by_function(FunctionId::from_raw(1)).len(), 1);
            assert_eq!(db.outgoing_calls_by_symbol(SymbolId::from_raw(101)).len(), 1);
            assert_eq!(db.incoming_calls_by_symbol(SymbolId::from_raw(21)).len(), 1);
            assert_eq!(db.incoming_calls_by_function(FunctionId::from_raw(11)).len(), 1);
            assert_eq!(
                db.unresolved_calls_by_reason(
                    crate::analysis::calls::facts::UnresolvedCallReason::FunctionValue,
                )
                .len(),
                1
            );
            assert_eq!(
                db.unresolved_calls_by_status(
                    crate::analysis::calls::facts::CallTargetStatus::Unresolved,
                )
                .len(),
                1
            );

            db.replace_call_facts(second).expect("second call replace");

            assert_eq!(
                db.resolve_stable_key(db.call_sites()[0].stable_key).as_ref(),
                "call-site:second"
            );
            assert!(db.call_targets().is_empty());
            assert!(db.unresolved_calls().is_empty());
        }
    }

    mod ts_object_model_storage {
        use super::*;
        use crate::ts::object_model::facts::{
            TsObjectAllocationFact, TsObjectAllocationId, TsObjectAllocationKind,
            TsObjectModelStatus, TsPropertyKey, TsPropertyKeyKind, TsPropertyReadFact,
            TsPropertyReadId, TsPropertyWriteFact, TsPropertyWriteId, TsPrototypeLinkFact,
            TsPrototypeLinkId, TsPrototypeLinkKind, TsReceiverBindingFact, TsReceiverBindingId,
            TsReceiverBindingKind,
        };
        use crate::ts::object_model::store::TsObjectModelOutput;

        #[test]
        fn replace_ts_object_model_facts_removes_stale_rows_from_previous_run() {
            let mut db = AnalysisDb::new();
            let file = db.add_file(
                PathBuf::from("src/app.ts"),
                "src/app.ts".to_string(),
                "const holder = { target() {} }; holder.target();\n".to_string(),
            );
            let interner = db.stable_key_interner();

            db.replace_ts_object_model_facts(full_output(&interner, file, "first"))
                .expect("first object-model replace");
            assert_eq!(db.ts_object_allocations().len(), 1);
            assert_eq!(db.ts_property_writes().len(), 1);
            assert_eq!(db.ts_property_reads().len(), 1);
            assert_eq!(db.ts_receiver_bindings().len(), 1);
            assert_eq!(db.ts_prototype_links().len(), 1);
            assert!(
                db.ts_object_model_store()
                    .expect("object-model store")
                    .allocation_by_stable_key(interner.intern("object:first"))
                    .is_some()
            );
            let interner = db.stable_key_interner();

            db.replace_ts_object_model_facts(allocation_only_output(&interner, file, "second"))
                .expect("second object-model replace");

            assert_eq!(db.ts_object_allocations().len(), 1);
            assert_eq!(db.ts_object_allocations()[0].id, TsObjectAllocationId(0));
            assert_eq!(db.ts_object_allocations()[0].stable_key, interner.intern("object:second"));
            assert!(db.ts_property_writes().is_empty());
            assert!(db.ts_property_reads().is_empty());
            assert!(db.ts_receiver_bindings().is_empty());
            assert!(db.ts_prototype_links().is_empty());
            let store = db.ts_object_model_store().expect("object-model store");
            assert!(store.allocation_by_stable_key(interner.intern("object:first")).is_none());
            assert!(store.allocation_by_stable_key(interner.intern("object:second")).is_some());
        }

        #[test]
        fn replace_ts_object_model_facts_rejects_duplicate_stable_keys() {
            let mut db = AnalysisDb::new();
            let file = db.add_file(
                PathBuf::from("src/app.ts"),
                "src/app.ts".to_string(),
                "const holder = {};\n".to_string(),
            );
            let interner = db.stable_key_interner();

            let error = db
                .replace_ts_object_model_facts(TsObjectModelOutput {
                    allocations: vec![
                        allocation(&interner, file, "object:dup", 1),
                        allocation(&interner, file, "object:dup", 2),
                    ],
                    property_writes: Vec::new(),
                    property_reads: Vec::new(),
                    receiver_bindings: Vec::new(),
                    prototype_links: Vec::new(),
                })
                .expect_err("duplicate stable key should be rejected");

            assert_eq!(
                error.to_string(),
                "invalid semantic fact from `polint.ts.object_model`: duplicate object allocation stable key `object:dup`"
            );
        }

        fn full_output(interner: &StableKeyInterner, file: FileId, suffix: &str) -> TsObjectModelOutput {
            TsObjectModelOutput {
                allocations: vec![allocation(interner, file, &format!("object:{suffix}"), 10)],
                property_writes: vec![property_write(interner, file, &format!("write:{suffix}"), suffix)],
                property_reads: vec![property_read(interner, file, &format!("read:{suffix}"), suffix)],
                receiver_bindings: vec![receiver_binding(interner, file, &format!("receiver:{suffix}"))],
                prototype_links: vec![prototype_link(interner, file, &format!("prototype:{suffix}"), suffix)],
            }
        }

        fn allocation_only_output(interner: &StableKeyInterner, file: FileId, suffix: &str) -> TsObjectModelOutput {
            TsObjectModelOutput {
                allocations: vec![allocation(interner, file, &format!("object:{suffix}"), 20)],
                property_writes: Vec::new(),
                property_reads: Vec::new(),
                receiver_bindings: Vec::new(),
                prototype_links: Vec::new(),
            }
        }

        fn allocation(interner: &StableKeyInterner, file: FileId, stable_key: &str, id: u64) -> TsObjectAllocationFact {
            TsObjectAllocationFact {
                id: TsObjectAllocationId(id),
                file,
                span: test_span(file, 1),
                stable_key: interner.intern(stable_key),
                lexical_parent_key: Some(interner.intern("scope:module")),
                inventory_function: None,
                inventory_function_stable_key: None,
                inventory_callsite: None,
                inventory_callsite_stable_key: None,
                kind: TsObjectAllocationKind::ObjectLiteral,
                status: TsObjectModelStatus::resolved(),
            }
        }

        fn property_write(interner: &StableKeyInterner, file: FileId, stable_key: &str, suffix: &str) -> TsPropertyWriteFact {
            TsPropertyWriteFact {
                id: TsPropertyWriteId(99),
                file,
                span: test_span(file, 2),
                stable_key: interner.intern(stable_key),
                base_object_stable_key: interner.intern(format!("object:{suffix}")),
                property_key: property_key(),
                value_function: None,
                value_function_stable_key: Some(interner.intern(format!("function:{suffix}"))),
                value_object_stable_key: None,
                status: TsObjectModelStatus::resolved(),
            }
        }

        fn property_read(interner: &StableKeyInterner, file: FileId, stable_key: &str, suffix: &str) -> TsPropertyReadFact {
            TsPropertyReadFact {
                id: TsPropertyReadId(99),
                file,
                span: test_span(file, 3),
                stable_key: interner.intern(stable_key),
                base_object_stable_key: interner.intern(format!("object:{suffix}")),
                property_key: property_key(),
                destination_stable_key: Some(interner.intern(format!("place:{suffix}"))),
                callsite: None,
                callsite_stable_key: Some(interner.intern(format!("callsite:{suffix}"))),
                status: TsObjectModelStatus::resolved(),
            }
        }

        fn receiver_binding(interner: &StableKeyInterner, file: FileId, stable_key: &str) -> TsReceiverBindingFact {
            TsReceiverBindingFact {
                id: TsReceiverBindingId(99),
                file,
                span: test_span(file, 4),
                stable_key: interner.intern(stable_key),
                kind: TsReceiverBindingKind::MethodCall,
                callsite: None,
                callsite_stable_key: Some(interner.intern("callsite:first")),
                callee_function: None,
                callee_function_stable_key: Some(interner.intern("function:first")),
                receiver_object_stable_key: Some(interner.intern("object:first")),
                receiver_place_stable_key: Some(interner.intern("place:holder")),
                lexical_parent_key: Some(interner.intern("scope:module")),
                status: TsObjectModelStatus::resolved(),
            }
        }

        fn prototype_link(interner: &StableKeyInterner, file: FileId, stable_key: &str, suffix: &str) -> TsPrototypeLinkFact {
            TsPrototypeLinkFact {
                id: TsPrototypeLinkId(99),
                file,
                span: test_span(file, 5),
                stable_key: interner.intern(stable_key),
                kind: TsPrototypeLinkKind::ClassPrototype,
                object_stable_key: interner.intern(format!("object:{suffix}")),
                prototype_stable_key: interner.intern(format!("object:{suffix}:prototype")),
                property_key: None,
                status: TsObjectModelStatus::resolved(),
            }
        }

        fn property_key() -> TsPropertyKey {
            TsPropertyKey {
                kind: TsPropertyKeyKind::Static,
                value: Some("target".to_string()),
            }
        }
    }

    mod call_fact_metadata {
        use super::*;
        use crate::analysis::calls::facts::{CallPrecision, CallTargetStatus};
        use crate::analysis::calls::store::CallOutput;

        #[test]
        fn replace_call_facts_records_metadata_provider_and_family_labels() {
            let mut db = AnalysisDb::new();
            let file = db.add_file(
                PathBuf::from("src/app.ts"),
                "src/app.ts".to_string(),
                "function app() { run(); }\n".to_string(),
            );
            let interner = db.stable_key_interner();

            db.replace_call_facts(CallOutput {
                sites: vec![test_call_site(
                    &interner,
                    0,
                    file,
                    FunctionId::from_raw(1),
                    "call-site:metadata",
                )],
                targets: vec![test_call_target(
                    &interner,
                    0,
                    CallSiteId(0),
                    FunctionId::from_raw(1),
                    "call-target:metadata",
                )],
                unresolved: vec![test_unresolved_call(
                    &interner,
                    CallSiteId(0),
                    FunctionId::from_raw(1),
                    "unresolved:metadata",
                )],
            })
            .expect("call replace");

            for family in [
                FactFamily::CallSite,
                FactFamily::CallTarget,
                FactFamily::UnresolvedCall,
            ] {
                let metadata = db
                    .metadata_for(FactRef::new(family, 0))
                    .expect("call metadata exists");
                assert_eq!(metadata.producer_id, "polint.calls");
                assert_eq!(metadata.layer_id, "polint.calls");
                assert_eq!(metadata.validation, ValidationStatus::NativeTrusted);
                assert!(matches!(
                    family.label(),
                    "CallSite" | "CallTarget" | "UnresolvedCall"
                ));
            }
        }

        #[test]
        fn call_metadata_maps_unknown_statuses_to_non_exact_precision() {
            let mut db = AnalysisDb::new();
            let file = db.add_file(
                PathBuf::from("src/app.ts"),
                "src/app.ts".to_string(),
                "function app() { target[key](); }\n".to_string(),
            );
            let interner = db.stable_key_interner();
            let mut site =
                test_call_site(&interner, 0, file, FunctionId::from_raw(1), "call-site:unsupported");
            site.status = CallTargetStatus::Unsupported;
            site.precision = CallPrecision::Unsupported;
            let mut target = test_call_target(
                &interner,
                0,
                CallSiteId(0),
                FunctionId::from_raw(1),
                "call-target:setup-missing",
            );
            target.status = CallTargetStatus::SetupMissing;
            target.precision = CallPrecision::Unknown;
            let unresolved = test_unresolved_call(
                &interner,
                CallSiteId(0),
                FunctionId::from_raw(1),
                "unresolved:unknown",
            );

            db.replace_call_facts(CallOutput {
                sites: vec![site],
                targets: vec![target],
                unresolved: vec![unresolved],
            })
            .expect("call replace");

            assert_ne!(
                db.metadata_for(FactRef::new(FactFamily::CallSite, 0))
                    .expect("call site metadata exists")
                    .precision,
                FactPrecision::Exact
            );
            assert_ne!(
                db.metadata_for(FactRef::new(FactFamily::CallTarget, 0))
                    .expect("call target metadata exists")
                    .precision,
                FactPrecision::Exact
            );
            assert_ne!(
                db.metadata_for(FactRef::new(FactFamily::UnresolvedCall, 0))
                    .expect("unresolved call metadata exists")
                    .precision,
                FactPrecision::Exact
            );
        }
    }

