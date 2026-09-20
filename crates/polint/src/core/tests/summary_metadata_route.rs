    // -----------------------------------------------------------------------
    // The SCC closure's metadata route on AnalysisDb (W3 commit 0)
    // -----------------------------------------------------------------------
    //
    // `close_summaries_by_scc` is generic over `impl AnalysisHost`. Production
    // runs on `AnalysisDb`, whose `AnalysisHost` impl overrides every `replace_*`
    // method -- and, until this commit, did not override
    // `refresh_summary_metadata_after_bulk_update`, so the closure's bulk refresh
    // fell through to the trait default and overwrote the five summary families'
    // payload column with `summary:<SummaryId>` text.
    //
    // These tests take the trait route deliberately: the whole defect was that
    // the inherent method looked right and was never reached.

    use crate::analysis_neutral::AnalysisHost as SummaryRouteHost;
    use crate::analysis_neutral::calls::facts::{
        CallAlgorithm, CallCallee, CallEdgeKind, CallPrecision as SummaryRouteCallPrecision,
        CallProvenance, CallSiteFact, CallSyntaxKind, CallTargetFact,
        CallTargetStatus as SummaryRouteCallStatus,
    };
    use crate::analysis_neutral::calls::store::CallOutput as SummaryRouteCallOutput;
    use crate::analysis_neutral::ids::{
        CallSiteId as SummaryRouteCallSiteId, CallTargetId, SummaryEventId, SummaryId,
    };
    use crate::analysis_neutral::summaries::facts::{
        SummaryDomainKind, SummaryPrecision, SummaryProvenance, SummaryStatus,
    };
    use crate::analysis_neutral::summaries::scc::compute_scc_schedule;
    use crate::analysis_neutral::summaries::store::SummaryOutput as SummaryRouteOutput;

    /// The text the `AnalysisHost` trait default writes instead of a digest.
    const ID_ONLY_PREFIXES: [&str; 2] = ["summary:", "summary-event:"];

    fn route_summary(function: u64, callable: &str, domain: SummaryDomainKind) -> SummaryFact {
        SummaryFact {
            id: SummaryId(0),
            callable_stable_key: stable_key_for_test(callable),
            function: FunctionId::from_raw(function),
            domain,
            status: SummaryStatus::Present,
            precision: SummaryPrecision::Local,
            provenance: SummaryProvenance::NativeLocal,
            payload_digest: format!("exit:Returns;async:Sync;cleanup:false;{callable}"),
            tito_flows: Vec::new(),
            stable_key: stable_key_for_test(&format!(
                "summary-route:{}:{callable}",
                domain.as_str()
            )),
        }
    }

    /// The two callee exit sets the tests contrast.
    const ROUTE_THROWS: &str = "exit:Throws;async:Sync;cleanup:false";
    const ROUTE_THROWS_AND_RETURNS: &str = "exit:Returns;exit:Throws;async:Sync;cleanup:false";

    fn route_event(function: u64, callable: &str) -> SummaryEventFact {
        SummaryEventFact {
            id: SummaryEventId(0),
            callable_stable_key: stable_key_for_test(callable),
            function: FunctionId::from_raw(function),
            domain: SummaryDomainKind::CallEffects,
            event_kind: "unresolved_callee".to_string(),
            reason: "1 unresolved call".to_string(),
            status: SummaryStatus::Unknown,
            precision: SummaryPrecision::UnknownTop,
            stable_key: stable_key_for_test(&format!("summary-route-event:{callable}")),
        }
    }

    fn route_call_site(id: u64, caller: u64) -> CallSiteFact {
        CallSiteFact {
            in_throw: false,
            id: SummaryRouteCallSiteId(id),
            language: Language::TypeScript,
            file: FileId::from_raw(1),
            caller: FunctionId::from_raw(caller),
            owner_symbol: None,
            body: MirBodyId(caller),
            operation: MirOpId(id),
            span: Span::point(FileId::from_raw(1), 1, 1),
            kind: CallSyntaxKind::Function,
            callee: CallCallee::Identifier {
                reference: None,
                name: format!("route_call_{id}"),
            },
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: SummaryRouteCallStatus::Resolved,
            precision: SummaryRouteCallPrecision::Exact,
            stable_key: stable_key_for_test(&format!("summary-route-site:{id}")),
        }
    }

    fn route_call_target(id: u64, site: u64, caller: u64, callee: u64) -> CallTargetFact {
        CallTargetFact {
            id: CallTargetId(id),
            site: SummaryRouteCallSiteId(site),
            caller: FunctionId::from_raw(caller),
            target_function: Some(FunctionId::from_raw(callee)),
            target_symbol: None,
            edge_kind: CallEdgeKind::Direct,
            algorithm: CallAlgorithm::DirectReference,
            status: SummaryRouteCallStatus::Resolved,
            reason: None,
            provenance: CallProvenance::Native,
            precision: SummaryRouteCallPrecision::Exact,
            stable_key: stable_key_for_test(&format!("summary-route-target:{id}")),
        }
    }

    /// A caller whose callee exits with `callee_exits`, plus one event, so the
    /// closure has something to update and sets its `summary_metadata_dirty`
    /// flag. The callee's control parts are a parameter so a second run can move
    /// them without reaching into the store, whose output vector is private.
    fn route_fixture_db(callee_exits: &str) -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let mut callee_control = route_summary(2, "func::callee", SummaryDomainKind::ControlEffects);
        callee_control.payload_digest = callee_exits.to_string();
        db.replace_summary_facts(SummaryRouteOutput {
            summaries: vec![
                route_summary(1, "func::caller", SummaryDomainKind::ControlEffects),
                route_summary(1, "func::caller", SummaryDomainKind::CallEffects),
                route_summary(1, "func::caller", SummaryDomainKind::MemoryEffects),
                route_summary(1, "func::caller", SummaryDomainKind::DataFlowTito),
                callee_control,
                route_summary(2, "func::callee", SummaryDomainKind::CallEffects),
                route_summary(2, "func::callee", SummaryDomainKind::MemoryEffects),
                route_summary(2, "func::callee", SummaryDomainKind::DataFlowTito),
            ],
            events: vec![route_event(2, "func::callee")],
        });
        db.replace_call_facts(SummaryRouteCallOutput {
            sites: vec![route_call_site(1, 1)],
            targets: vec![route_call_target(1, 1, 1, 2)],
            unresolved: Vec::new(),
        })
        .expect("call output should be valid");
        db
    }

    /// Drive the closure through the trait, exactly as
    /// `run_scc_closure_with_previous_digests` does. Generic on purpose: an
    /// inherent method on `AnalysisDb` is not in scope in here, so what this
    /// reaches is what production reaches.
    fn drive_closure<H: SummaryRouteHost>(db: &mut H) -> usize {
        let schedule = compute_scc_schedule(db);
        assert!(
            !schedule.sccs.is_empty(),
            "the fixture must produce an SCC schedule or the closure never runs"
        );
        let mut demand_engine =
            crate::analysis_kernel::incremental::DemandQueryEngine::default();
        let result = crate::analysis_neutral::summaries::closure::close_summaries_by_scc(
            db,
            &schedule,
            &crate::analysis_neutral::summaries::closure::SccClosureConfig::default(),
            &mut demand_engine,
            &std::collections::BTreeMap::new(),
        );
        assert!(
            result.updated_summaries > 0,
            "the fixture must make the closure update a summary, or the bulk \
             refresh this test is about is never called"
        );
        result.updated_summaries
    }

    fn summary_family_rows(db: &AnalysisDb) -> Vec<(FactFamily, u64, String)> {
        let mut rows = Vec::new();
        for family in [
            FactFamily::SummaryControl,
            FactFamily::SummaryCall,
            FactFamily::SummaryMemory,
            FactFamily::SummaryTito,
            FactFamily::SummaryEvent,
        ] {
            for (run_id, meta) in db.fact_meta().family_rows_with_run_id(family) {
                rows.push((family, run_id, meta.payload_digest.clone()));
            }
        }
        rows
    }

    #[test]
    fn scc_closure_records_summary_metadata_through_the_digest_recipe_on_analysis_db() {
        let mut db = route_fixture_db(ROUTE_THROWS);
        drive_closure(&mut db);

        let interner = db.stable_key_interner();
        let summaries: std::collections::BTreeMap<u64, SummaryFact> = db
            .summary_facts()
            .iter()
            .map(|fact| (fact.id.0, fact.clone()))
            .collect();
        let events: std::collections::BTreeMap<u64, SummaryEventFact> = db
            .summary_events()
            .iter()
            .map(|fact| (fact.id.0, fact.clone()))
            .collect();

        let rows = summary_family_rows(&db);
        assert_eq!(
            rows.len(),
            summaries.len() + events.len(),
            "every summary and event must carry one metadata row"
        );
        assert!(!rows.is_empty(), "the fixture must produce summary rows");

        for (family, run_id, digest) in &rows {
            for prefix in ID_ONLY_PREFIXES {
                assert!(
                    !digest.starts_with(prefix),
                    "{} row {run_id} still carries the trait default's id-only text: {digest}",
                    family.label()
                );
            }
            let expected = if *family == FactFamily::SummaryEvent {
                let fact = events.get(run_id).expect("an event behind the row");
                super::metadata::summary_event_payload_metadata_digest(&interner, fact)
            } else {
                let fact = summaries.get(run_id).expect("a summary behind the row");
                super::metadata::summary_fact_payload_metadata_digest(&interner, fact)
            };
            assert_eq!(
                digest,
                &expected,
                "{} row {run_id} is not the digest recipe's value",
                family.label()
            );
        }
    }

    #[test]
    fn the_closures_bulk_refresh_folds_a_parts_change_on_analysis_db() {
        // The point of the route: the payload column has to move when the parts
        // move. Under the trait default it could not, because the column was the
        // row's id.
        let mut db = route_fixture_db(ROUTE_THROWS);
        drive_closure(&mut db);
        let before = summary_family_rows(&db);

        let mut changed = route_fixture_db(ROUTE_THROWS_AND_RETURNS);
        drive_closure(&mut changed);
        let after = summary_family_rows(&changed);

        assert_eq!(
            before.len(),
            after.len(),
            "the parts change must not add or drop a row"
        );
        assert_ne!(
            before, after,
            "a parts change has to move the payload column; under the trait \
             default's id-only text it could not"
        );
    }
