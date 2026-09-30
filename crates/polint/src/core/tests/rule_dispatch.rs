    /// Longest-first dispatch from recorded rule times, and the
    /// registration-order dispatch a pass keeps without them.
    mod rule_dispatch {
        use super::*;
        use crate::core::rule::{RuleRunOutput, RuleStartOrder};
        use std::sync::Mutex;

        type StartLog = Arc<Mutex<Vec<&'static str>>>;

        const ORDERED_IDS: [&str; 4] = ["examples/a", "examples/b", "examples/c", "examples/d"];

        fn timings(entries: &[(&str, u64)]) -> RuleTimings {
            entries
                .iter()
                .map(|(rule_id, millis)| (rule_id.to_string(), Duration::from_millis(*millis)))
                .collect()
        }

        fn meta(id: &'static str) -> RuleMeta {
            RuleMeta {
                id: id.to_string(),
                description: format!("Test rule {id}"),
                severity: Severity::Warn,
                kind: RuleKind::Check,
            }
        }

        /// A rule that notes when it starts, counts `events` observations as a
        /// policy query would, and reports one diagnostic.
        fn logged_rule(
            id: &'static str,
            events: usize,
            delay: Duration,
            log: &StartLog,
        ) -> Rule {
            let log = StartLog::clone(log);
            Rule::from_parts(
                move || meta(id),
                Capabilities::new,
                move |_db, ctx| {
                    log.lock().expect("start log").push(id);
                    if !delay.is_zero() {
                        thread::sleep(delay);
                    }
                    crate::policy_queries::observe_events_for_test(events);
                    ctx.report(
                        Diagnostic::new(
                            id,
                            Severity::Warn,
                            "src/main.go",
                            DiagnosticRange::point(1, 1),
                            "logged rule ran",
                        )
                        .with_fingerprint(id),
                    );
                    Ok(())
                },
            )
        }

        fn rule_pass(
            rules: &[Rule],
            enabled: Option<&BTreeSet<String>>,
            parallel: bool,
            timings: Option<&RuleTimings>,
            threads: usize,
        ) -> RuleRunOutput {
            let db = AnalysisDb::new();
            let support = CapabilitySupportView::empty();
            let completeness = CompletenessView::unknown();
            let blocked = BTreeSet::new();
            let runtime = RuleRuntimeViews::new(&support, &completeness, &blocked);
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .expect("test rayon pool")
                .install(|| {
                    run_rules_observed(
                        &db,
                        rules,
                        &BTreeMap::new(),
                        enabled,
                        parallel,
                        &runtime,
                        timings,
                    )
                })
        }

        /// The order a parallel pass on one worker starts `ORDERED_IDS` in,
        /// and how it says it dispatched them.
        fn start_order(timings: Option<&RuleTimings>) -> (Vec<&'static str>, RuleStartOrder) {
            let log = StartLog::default();
            let rules: Vec<Rule> = ORDERED_IDS
                .iter()
                .map(|&id| logged_rule(id, 0, Duration::ZERO, &log))
                .collect();
            let output = rule_pass(&rules, None, true, timings, 1);
            let started = log.lock().expect("start log").clone();
            (started, output.start_order)
        }

        #[test]
        fn a_timed_pass_starts_the_longest_recorded_rule_first() {
            let recorded = timings(&[
                ("examples/a", 20),
                ("examples/b", 5),
                ("examples/c", 30),
                ("examples/d", 10),
            ]);

            assert_eq!(
                start_order(Some(&recorded)),
                (
                    vec!["examples/c", "examples/a", "examples/d", "examples/b"],
                    RuleStartOrder::LongestFirst
                )
            );
        }

        #[test]
        fn rules_without_a_recorded_time_start_before_timed_rules() {
            let recorded = timings(&[("examples/b", 5), ("examples/d", 10)]);

            assert_eq!(
                start_order(Some(&recorded)).0,
                vec!["examples/a", "examples/c", "examples/d", "examples/b"]
            );
        }

        #[test]
        fn rules_with_equal_recorded_times_start_in_registration_order() {
            let recorded = timings(&[
                ("examples/a", 7),
                ("examples/b", 7),
                ("examples/c", 7),
                ("examples/d", 7),
            ]);

            assert_eq!(start_order(Some(&recorded)).0, ORDERED_IDS.to_vec());
        }

        #[test]
        fn a_pass_without_timings_keeps_registration_order() {
            assert_eq!(
                start_order(None),
                (ORDERED_IDS.to_vec(), RuleStartOrder::Registration)
            );
        }

        #[test]
        fn timings_that_name_none_of_the_rules_keep_registration_order() {
            let recorded = timings(&[("examples/retired", 900)]);

            assert_eq!(
                start_order(Some(&recorded)),
                (ORDERED_IDS.to_vec(), RuleStartOrder::Registration)
            );
        }

        #[test]
        fn a_sequential_pass_keeps_registration_order_despite_timings() {
            let log = StartLog::default();
            let rules: Vec<Rule> = ORDERED_IDS
                .iter()
                .map(|&id| logged_rule(id, 0, Duration::ZERO, &log))
                .collect();
            let recorded = timings(&[("examples/d", 30), ("examples/c", 20)]);

            let output = rule_pass(&rules, None, false, Some(&recorded), 1);

            let started = log.lock().expect("start log").clone();
            assert_eq!(
                (started, output.start_order),
                (ORDERED_IDS.to_vec(), RuleStartOrder::Registration)
            );
        }

        #[test]
        fn a_pass_records_how_long_each_rule_that_ran_took() {
            let log = StartLog::default();
            let rules = vec![
                logged_rule("examples/slow", 0, Duration::from_millis(20), &log),
                logged_rule("examples/fast", 0, Duration::ZERO, &log),
                logged_rule("examples/disabled", 0, Duration::ZERO, &log),
                TestRule::panic("examples/panic").into_rule(),
            ];
            let enabled = BTreeSet::from([
                "examples/slow".to_string(),
                "examples/fast".to_string(),
                "examples/panic".to_string(),
            ]);

            let output = rule_pass(&rules, Some(&enabled), true, None, 2);

            let recorded: Vec<&str> = output.timings.keys().map(String::as_str).collect();
            let slow = output.timings["examples/slow"];
            assert_eq!(
                (recorded, slow >= Duration::from_millis(20)),
                (vec!["examples/fast", "examples/panic", "examples/slow"], true),
            );
        }

        /// Every row shape a pass merges: plain reports with observation
        /// counts, two rules whose diagnostics dedupe into one, an error, a
        /// panic, a rule the enabled set skips, and rules of varied length.
        fn mixed_rules(log: &StartLog) -> (Vec<Rule>, BTreeSet<String>) {
            let rules = vec![
                logged_rule("examples/alpha", 3, Duration::ZERO, log),
                logged_rule("examples/beta", 5, Duration::from_millis(2), log),
                TestRule::report("examples/duplicate-a", Severity::Warn, "shared-fingerprint")
                    .into_rule(),
                TestRule::report("examples/duplicate-b", Severity::Error, "shared-fingerprint")
                    .into_rule(),
                TestRule::error("examples/error").into_rule(),
                TestRule::panic("examples/panic").into_rule(),
                logged_rule("examples/gamma", 7, Duration::from_millis(1), log),
                logged_rule("examples/skipped", 11, Duration::ZERO, log),
                logged_rule("examples/delta", 1, Duration::from_millis(3), log),
                logged_rule("examples/epsilon", 2, Duration::ZERO, log),
                logged_rule("examples/zeta", 4, Duration::from_millis(1), log),
                logged_rule("examples/eta", 6, Duration::ZERO, log),
            ];
            let enabled = rules
                .iter()
                .map(|rule| rule.meta().id)
                .filter(|id| id != "examples/skipped")
                .collect();
            (rules, enabled)
        }

        /// A deterministic permutation of `0..len` for `seed`.
        fn shuffled_positions(len: usize, seed: u64) -> Vec<usize> {
            let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
            let mut positions: Vec<usize> = (0..len).collect();
            for index in (1..len).rev() {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let other = usize::try_from(state % (index as u64 + 1)).expect("index fits");
                positions.swap(index, other);
            }
            positions
        }

        /// Recorded times that make a timed pass start `rules` in `order`.
        fn timings_for_order(rules: &[Rule], order: &[usize]) -> RuleTimings {
            order
                .iter()
                .enumerate()
                .map(|(rank, &position)| {
                    let millis = u64::try_from(order.len() - rank).expect("rank fits");
                    (rules[position].meta().id, Duration::from_millis(millis))
                })
                .collect()
        }

        #[test]
        fn the_dispatch_order_does_not_change_what_a_pass_reports() {
            let log = StartLog::default();
            let (rules, enabled) = mixed_rules(&log);
            let expected = rule_pass(&rules, Some(&enabled), false, None, 1);

            for seed in 1..=12 {
                let order = shuffled_positions(rules.len(), seed);
                let recorded = timings_for_order(&rules, &order);
                for threads in [1, 4] {
                    let output = rule_pass(&rules, Some(&enabled), true, Some(&recorded), threads);
                    assert_eq!(
                        (
                            &output.diagnostics,
                            &output.observed_events,
                            output.start_order
                        ),
                        (
                            &expected.diagnostics,
                            &expected.observed_events,
                            RuleStartOrder::LongestFirst
                        ),
                        "seed {seed} on {threads} workers"
                    );
                }
            }
        }

        #[test]
        fn a_timed_pass_starts_rules_in_every_recorded_order() {
            let log = StartLog::default();
            let (rules, enabled) = mixed_rules(&log);
            let logged: BTreeSet<&str> = [
                "examples/alpha",
                "examples/beta",
                "examples/gamma",
                "examples/delta",
                "examples/epsilon",
                "examples/zeta",
                "examples/eta",
            ]
            .into_iter()
            .collect();

            for seed in 1..=12 {
                let order = shuffled_positions(rules.len(), seed);
                let recorded = timings_for_order(&rules, &order);
                log.lock().expect("start log").clear();

                rule_pass(&rules, Some(&enabled), true, Some(&recorded), 1);

                let expected: Vec<String> = order
                    .iter()
                    .map(|&position| rules[position].meta().id)
                    .filter(|id| logged.contains(id.as_str()))
                    .collect();
                let started: Vec<String> = log
                    .lock()
                    .expect("start log")
                    .iter()
                    .map(|id| id.to_string())
                    .collect();
                assert_eq!(started, expected, "seed {seed}");
            }
        }

        #[test]
        fn registration_order_dispatch_reports_what_a_sequential_pass_does() {
            let log = StartLog::default();
            let (rules, enabled) = mixed_rules(&log);
            let expected = rule_pass(&rules, Some(&enabled), false, None, 1);

            let output = rule_pass(&rules, Some(&enabled), true, None, 4);

            assert_eq!(
                (&output.diagnostics, &output.observed_events, output.start_order),
                (
                    &expected.diagnostics,
                    &expected.observed_events,
                    RuleStartOrder::Registration
                )
            );
        }

        #[test]
        fn a_panicking_rule_is_isolated_at_any_queue_position() {
            let rules = vec![
                TestRule::report("examples/first", Severity::Warn, "first").into_rule(),
                TestRule::panic("examples/panic").into_rule(),
                TestRule::report("examples/last", Severity::Error, "last").into_rule(),
            ];
            let expected = rule_pass(&rules, None, false, None, 1);
            let internal: Vec<&str> = expected
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.rule_id.as_str())
                .filter(|rule_id| rule_id.starts_with("internal/"))
                .collect();
            assert_eq!(
                internal,
                vec!["internal/examples/panic"],
                "the sequential pass isolates the panic"
            );

            for panic_millis in [30, 15, 5] {
                let recorded = timings(&[
                    ("examples/first", 20),
                    ("examples/panic", panic_millis),
                    ("examples/last", 10),
                ]);
                for threads in [1, 3] {
                    let output = rule_pass(&rules, None, true, Some(&recorded), threads);
                    assert_eq!(
                        output.diagnostics, expected.diagnostics,
                        "panic recorded at {panic_millis} ms on {threads} workers"
                    );
                }
            }
        }
    }
