    // -----------------------------------------------------------------------
    // The AnalysisHost reference and definition route on AnalysisDb (W1 commit 1)
    // -----------------------------------------------------------------------
    //
    // Both lowerers are generic over `impl AnalysisHost`, so their closure-capture
    // scans reached the trait defaults in `analysis_neutral/host.rs`, which filter
    // the whole reference and definition tables on every call. `AnalysisDb` has
    // carried `references_by_file` and `definitions_by_symbol` all along; only its
    // inherent methods reached them, and an inherent method is invisible inside a
    // generic function.
    //
    // These tests take the trait route deliberately, and each one recomputes the
    // default's own rule over the raw table beside it: the route is only correct
    // if it answers what the scan answered.

    use crate::analysis_neutral::AnalysisHost as LookupRouteHost;

    /// A bucket with more than one candidate, which is where a first-match rule
    /// and an index can disagree.
    fn lookup_route_db() -> (AnalysisDb, FileId, SymbolId, SymbolId) {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "const theme = 1;\nexport function Button() { return theme; }\n".to_string(),
        );
        let interner = db.stable_key_interner();
        let promoted = SymbolId::from_raw(0xfeed_beef);
        let plain = SymbolId::from_raw(0xabc0_1234);

        let symbol = |id: SymbolId, name: &str| {
            SymbolFact::new(
                id,
                Language::TypeScript,
                name.to_string(),
                format!("src/app.ts::{name}"),
                SymbolKind::Function,
                SymbolNamespace::Value,
                Some(file),
                None,
                None,
                None,
                Some(test_span(file, 1)),
                true,
                interner.intern(format!("ts|src/app.ts|value|function|{name}|1:1")),
                SymbolPrecision::ExactLocal,
            )
        };
        let definition = |raw: u64, symbol: SymbolId, line: u32, is_primary: bool| {
            DefinitionFact::new(
                DefinitionId::from_raw(raw),
                symbol,
                Language::TypeScript,
                "Button".to_string(),
                "src/app.ts::Button".to_string(),
                DefinitionKind::Declaration,
                SymbolNamespace::Value,
                Some(file),
                None,
                None,
                None,
                Some(test_span(file, line)),
                is_primary,
                true,
                interner.intern(format!("ts|src/app.ts|definition|Button|{line}:1")),
                SymbolPrecision::ExactLocal,
            )
        };
        let reference = |raw: u64, line: u32| {
            ReferenceFact::new(
                ReferenceId::from_raw(raw),
                Language::TypeScript,
                "theme".to_string(),
                "src/app.ts::theme".to_string(),
                ReferenceKind::Read,
                SymbolNamespace::Value,
                Some(file),
                None,
                None,
                None,
                Some(test_span(file, line)),
                Some(plain),
                Vec::new(),
                interner.intern(format!("ts|src/app.ts|reference|theme|{line}:1")),
                SymbolResolutionStatus::Resolved,
                SymbolPrecision::ExactLocal,
            )
        };

        db.replace_symbol_graph_facts(
            vec![symbol(promoted, "Button"), symbol(plain, "theme")],
            // `promoted` has three definitions and the primary one is not first,
            // so the rule has to look past the head of the bucket. `plain` has two
            // and neither is primary, so the rule falls back to the first.
            vec![
                definition(0x1010_2020, promoted, 1, false),
                definition(0x1010_2021, promoted, 2, true),
                definition(0x1010_2022, promoted, 3, false),
                definition(0x1010_2023, plain, 4, false),
                definition(0x1010_2024, plain, 5, false),
            ],
            // Table order is the push order; `references_by_file` is sorted by
            // `ReferenceId`. The ids here descend so the two orders differ and the
            // set comparison below is not vacuous.
            vec![reference(0x5050_6060, 1), reference(0x3030_4040, 2)],
        );
        (db, file, promoted, plain)
    }

    /// What `AnalysisHost`'s default would return: a filter over the whole table.
    fn lookup_route_default_definitions(db: &AnalysisDb, symbol: SymbolId) -> Vec<DefinitionId> {
        db.definitions()
            .iter()
            .filter(|definition| definition.symbol == symbol)
            .map(|definition| definition.id)
            .collect()
    }

    #[test]
    fn host_definitions_for_symbol_route_keeps_the_scan_order() {
        let (db, _file, promoted, plain) = lookup_route_db();

        for symbol in [promoted, plain] {
            assert_eq!(
                LookupRouteHost::definitions_for_symbol(&db, symbol)
                    .map(|definition| definition.id)
                    .collect::<Vec<_>>(),
                lookup_route_default_definitions(&db, symbol),
                "the indexed route must yield the table's order, not the index's"
            );
        }
    }

    #[test]
    fn host_definition_for_symbol_route_keeps_first_primary_else_first() {
        let (db, _file, promoted, plain) = lookup_route_db();

        // First-primary: the primary definition is the second of three.
        assert_eq!(
            LookupRouteHost::definition_for_symbol(&db, promoted).map(|definition| definition.id),
            Some(DefinitionId::from_raw(0x1010_2021))
        );
        // Else-first: no definition is primary, so the head of the bucket wins.
        assert_eq!(
            LookupRouteHost::definition_for_symbol(&db, plain).map(|definition| definition.id),
            Some(DefinitionId::from_raw(0x1010_2023))
        );
        assert_eq!(
            LookupRouteHost::definition_for_symbol(&db, SymbolId::from_raw(0xdead_0000))
                .map(|definition| definition.id),
            None
        );

        // And it is the same answer the default's rule computes over the table.
        for symbol in [promoted, plain] {
            let mut scan = db
                .definitions()
                .iter()
                .filter(|definition| definition.symbol == symbol);
            let first = scan.next();
            let expected = first
                .filter(|definition| definition.is_primary)
                .or_else(|| scan.find(|definition| definition.is_primary))
                .or(first);
            assert_eq!(
                LookupRouteHost::definition_for_symbol(&db, symbol).map(|row| row.id),
                expected.map(|row| row.id)
            );
        }
    }

    #[test]
    fn host_references_for_file_route_keeps_the_scan_set() {
        let (db, file, _promoted, _plain) = lookup_route_db();

        let mut routed = LookupRouteHost::references_for_file(&db, file)
            .into_iter()
            .map(|reference| reference.id)
            .collect::<Vec<_>>();
        let mut scanned = db
            .references()
            .iter()
            .filter(|reference| reference.file == Some(file))
            .map(|reference| reference.id)
            .collect::<Vec<_>>();
        assert_eq!(routed.len(), 2);
        assert_eq!(scanned.len(), 2);
        // The set is the contract. The order is not: the index sorts by
        // `ReferenceId` and the table here does not, which is exactly the case
        // every trait-generic caller has to be indifferent to.
        assert_ne!(routed, scanned, "the fixture must exercise the two orders");
        routed.sort();
        scanned.sort();
        assert_eq!(routed, scanned);

        assert!(
            LookupRouteHost::references_for_file(&db, FileId::from_raw(4242)).is_empty(),
            "an unknown file has no references on either route"
        );
    }
