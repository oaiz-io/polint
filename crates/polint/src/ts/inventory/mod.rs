#[cfg(feature = "lang-typescript")]
pub mod extract;
pub mod facts;
pub mod store;

#[cfg(all(test, feature = "lang-typescript"))]
mod extract_function_forms {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use crate::ts::inventory::extract::extract_ts_inventory;
    use crate::ts::inventory::facts::TsFunctionInventoryKind;
    use crate::ts::local_db::LocalFactDb;

    #[test]
    fn extracts_every_required_function_form() {
        let file = fixture_file(
            r#"
function declared() {}
const expr = function namedExpr() {};
const arrow = () => {};
class Box {
  constructor() {}
  method() {}
  get value() { return 1; }
  set value(next) {}
  static {
    declared();
  }
}
"#,
        );

        let interner = crate::internal_core::StableKeyInterner::default();
        let output = extract_ts_inventory(&interner, file);
        let kinds = output
            .functions
            .iter()
            .map(|function| function.kind)
            .collect::<BTreeSet<_>>();

        for expected in [
            TsFunctionInventoryKind::Declaration,
            TsFunctionInventoryKind::FunctionExpression,
            TsFunctionInventoryKind::Arrow,
            TsFunctionInventoryKind::Method,
            TsFunctionInventoryKind::Constructor,
            TsFunctionInventoryKind::Accessor,
            TsFunctionInventoryKind::ClassStaticBlock,
        ] {
            assert!(kinds.contains(&expected), "missing {expected:?}");
        }
    }

    #[test]
    fn marks_the_functions_written_without_a_body() {
        let file = fixture_file(
            r#"
function pick(x: string): string;
function pick(x: any) { return x; }
declare function ambient(x: string): void;
abstract class Shape {
  abstract area(): number;
  size(): number;
  size() { return 1; }
}
const arrow = () => {};
"#,
        );

        let interner = crate::internal_core::StableKeyInterner::default();
        let output = extract_ts_inventory(&interner, file);
        let rows = output
            .functions
            .iter()
            .map(|function| {
                (
                    function.display_name.clone().unwrap_or_default(),
                    function.has_body,
                )
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(
            rows,
            BTreeSet::from([
                ("pick".to_string(), false),
                ("pick".to_string(), true),
                ("ambient".to_string(), false),
                ("area".to_string(), false),
                ("size".to_string(), false),
                ("size".to_string(), true),
                ("arrow".to_string(), true),
            ])
        );
    }

    #[test]
    fn normalized_function_rows_sort_by_stable_key_before_dense_ids() {
        let file = fixture_file(
            r#"
const second = () => {};
function first() {}
"#,
        );
        let interner = crate::internal_core::StableKeyInterner::default();
        let mut output = extract_ts_inventory(&interner, file);
        output.functions.reverse();

        let normalized = output.normalized(&interner);
        let stable_keys = normalized
            .functions
            .iter()
            .map(|function| interner.resolve(function.stable_key))
            .collect::<Vec<_>>();
        let mut sorted_keys = stable_keys.clone();
        sorted_keys.sort();
        let dense_ids = normalized
            .functions
            .iter()
            .map(|function| function.id.0)
            .collect::<Vec<_>>();

        assert_eq!(stable_keys, sorted_keys);
        assert_eq!(dense_ids, vec![0, 1]);
    }

    #[test]
    fn stable_keys_include_file_span_parent_kind_and_display_name() {
        let file = fixture_file(
            r#"
function outer() {
  const inner = () => {};
}
"#,
        );
        let interner = crate::internal_core::StableKeyInterner::default();
        let output = extract_ts_inventory(&interner, file);
        let inner = output
            .functions
            .iter()
            .find(|function| function.display_name.as_deref() == Some("inner"))
            .expect("inner arrow inventory row");

        let key = interner.resolve(inner.stable_key);
        assert!(key.contains("src/forms.ts"));
        assert!(key.contains("arrow"));
        assert!(key.contains("inner"));
        assert!(key.contains("outer"));
        assert!(inner.span.start_byte < inner.span.end_byte);
    }

    #[test]
    fn computed_string_concatenation_methods_keep_display_names() {
        let file = fixture_file(
            r#"
class Box {
  ["na" + "me"]() {}
  ["a" + "b" + "c"]() {}
  static [("static") + "G"]() {}
}
"#,
        );
        let interner = crate::internal_core::StableKeyInterner::default();
        let output = extract_ts_inventory(&interner, file);
        let names = output
            .functions
            .iter()
            .filter_map(|function| function.display_name.as_deref())
            .collect::<BTreeSet<_>>();

        assert!(names.contains("name"), "missing names in {names:?}");
        assert!(names.contains("abc"), "missing names in {names:?}");
        assert!(names.contains("staticG"), "missing names in {names:?}");
    }

    fn fixture_file(source: &str) -> &'static crate::analysis_api::SourceFile {
        let mut db = Box::new(LocalFactDb::new());
        let file_id = db.add_file(
            PathBuf::from("src/forms.ts"),
            "src/forms.ts".to_string(),
            source.to_string(),
        );
        let leaked = Box::leak(db);
        leaked.file(file_id).expect("fixture file")
    }
}

#[cfg(all(test, feature = "lang-typescript"))]
mod extract_callsite_forms {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use crate::ts::inventory::extract::extract_ts_inventory;
    use crate::ts::inventory::facts::{TsCallsiteInventoryKind, TsInventoryStatus};
    use crate::ts::local_db::LocalFactDb;

    #[test]
    fn extracts_every_required_callsite_form() {
        let file = fixture_file(
            r#"
function invoke(dynamicSpecifier, maybe) {
  run();
  new Widget();
  tag`hello`;
  maybe?.();
  import("./static");
  import(dynamicSpecifier);
  require("pkg");
}
"#,
        );

        let interner = crate::internal_core::StableKeyInterner::default();
        let output = extract_ts_inventory(&interner, file);
        let kinds = output
            .callsites
            .iter()
            .map(|callsite| callsite.kind)
            .collect::<BTreeSet<_>>();

        for expected in [
            TsCallsiteInventoryKind::Call,
            TsCallsiteInventoryKind::New,
            TsCallsiteInventoryKind::TaggedTemplate,
            TsCallsiteInventoryKind::OptionalCall,
            TsCallsiteInventoryKind::DynamicImport,
            TsCallsiteInventoryKind::Require,
        ] {
            assert!(kinds.contains(&expected), "missing {expected:?}");
        }
    }

    #[test]
    fn non_string_dynamic_import_is_explicitly_unresolved() {
        let file = fixture_file(
            r#"
function load(path) {
  return import(path);
}
"#,
        );

        let interner = crate::internal_core::StableKeyInterner::default();
        let output = extract_ts_inventory(&interner, file);
        let dynamic_import = output
            .callsites
            .iter()
            .find(|callsite| callsite.kind == TsCallsiteInventoryKind::DynamicImport)
            .expect("dynamic import callsite");

        assert!(matches!(
            &dynamic_import.status,
            TsInventoryStatus::Unresolved { reason } if reason == "non-string dynamic import"
        ));
    }

    #[test]
    fn callsite_spans_are_valid_byte_spans() {
        let file = fixture_file(
            r#"
function invoke() {
  run();
}
"#,
        );

        let interner = crate::internal_core::StableKeyInterner::default();
        let output = extract_ts_inventory(&interner, file);
        let call = output
            .callsites
            .iter()
            .find(|callsite| callsite.kind == TsCallsiteInventoryKind::Call)
            .expect("normal callsite");

        assert!(call.span.start_byte < call.span.end_byte);
        let key = interner.resolve(call.stable_key);
        assert!(key.contains("src/calls.ts"));
        assert!(key.contains("call"));
    }

    #[test]
    fn callsite_spans_match_jelly_parenthesized_call_shapes() {
        let source = r#"
(function(){})();
((f))();
(f());
((new f()));
g(h());
function f() {}
function g(x) {}
function h() {}
"#;
        let file = fixture_file(source);

        let interner = crate::internal_core::StableKeyInterner::default();
        let output = extract_ts_inventory(&interner, file);
        let spans = output
            .callsites
            .iter()
            .map(|callsite| {
                (
                    callsite.span.start_byte,
                    callsite.span.end_byte,
                    &source[callsite.span.start_byte as usize..callsite.span.end_byte as usize],
                )
            })
            .collect::<Vec<_>>();
        let span_texts = output
            .callsites
            .iter()
            .map(|callsite| {
                &source[callsite.span.start_byte as usize..callsite.span.end_byte as usize]
            })
            .collect::<BTreeSet<_>>();

        for expected in ["function(){})()", "(f))()", "(f())", "(new f())"] {
            assert!(
                span_texts.contains(expected),
                "missing span {expected:?} in {span_texts:?}; spans: {spans:?}"
            );
        }
        // A call passed as a call argument keeps its own span: the inner `h()`
        // must not absorb the enclosing `g(...)` argument-list parentheses.
        assert!(
            span_texts.contains("h()"),
            "inner call `h()` should keep its own span, not `(h())`; spans: {spans:?}"
        );
        assert!(
            span_texts.contains("g(h())"),
            "outer call `g(h())` span missing; spans: {spans:?}"
        );
    }

    fn fixture_file(source: &str) -> &'static crate::analysis_api::SourceFile {
        let mut db = Box::new(LocalFactDb::new());
        let file_id = db.add_file(
            PathBuf::from("src/calls.ts"),
            "src/calls.ts".to_string(),
            source.to_string(),
        );
        let leaked = Box::leak(db);
        leaked.file(file_id).expect("fixture file")
    }
}
