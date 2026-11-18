#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
mod tests {
    use crate::analysis_neutral::calls::facts::CallPrecision;

    fn edges(source: &str, needle: &str) -> Vec<(String, CallPrecision)> {
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("main.js"), source).unwrap();
        let output = crate::eval::observed::run_kernel_for_repo_for_test(repo.path()).unwrap();
        let start = source.find(needle).unwrap() as u32;
        let site = output
            .db
            .call_sites()
            .iter()
            .find(|site| {
                site.span.start_byte == start && site.span.end_byte == start + needle.len() as u32
            })
            .unwrap_or_else(|| panic!("missing site {needle}"));
        output
            .db
            .refined_call_edges()
            .iter()
            .filter(|edge| edge.site == site.id)
            .filter_map(|edge| {
                let target = output
                    .db
                    .functions()
                    .iter()
                    .find(|function| Some(function.id) == edge.target_function)?;
                Some((
                    source[target.span.start_byte as usize..target.span.end_byte as usize]
                        .to_string(),
                    edge.precision,
                ))
            })
            .collect()
    }

    #[test]
    fn builtin_callbacks_are_conservative_model_edges() {
        for (source, call, target) in [
            (
                "function cb() {} [1].forEach(cb);",
                "[1].forEach(cb)",
                "function cb() {}",
            ),
            (
                "function replacer(k, v) { return v; } JSON.stringify({}, replacer);",
                "JSON.stringify({}, replacer)",
                "function replacer(k, v) { return v; }",
            ),
            (
                "function tick() {} queueMicrotask(tick);",
                "queueMicrotask(tick)",
                "function tick() {}",
            ),
            (
                "function executor(resolve) { resolve(1); } new Promise(executor);",
                "new Promise(executor)",
                "function executor(resolve) { resolve(1); }",
            ),
        ] {
            let found = edges(source, call);
            assert!(
                found.contains(&(target.into(), CallPrecision::Conservative)),
                "{call}: {found:?}"
            );
        }
    }

    #[test]
    fn builtin_receivers_do_not_call_same_named_user_functions() {
        let source = "function stringify() {} function cb() {} JSON.stringify({}, cb);";
        assert!(
            edges(source, "JSON.stringify({}, cb)")
                .iter()
                .all(|(target, _)| target != "function stringify() {}")
        );
    }

    #[test]
    fn shadowed_globals_keep_their_own_behavior() {
        let source =
            "const JSON = { stringify(value, cb) {} }; function cb() {} JSON.stringify({}, cb);";
        assert!(
            edges(source, "JSON.stringify({}, cb)")
                .iter()
                .all(|(target, _)| target != "function cb() {}")
        );
    }

    #[test]
    fn await_loads_fulfilled_callables() {
        let source = "function target() {} async function use() { const fn = await Promise.resolve(target); fn(); } use();";
        assert!(
            edges(source, "fn()")
                .iter()
                .any(|(target, _)| target == "function target() {}")
        );
    }
}
