#[cfg(test)]
mod semantic_scopes {
    use oxc_semantic::{AstNodes, ReferenceId as OxcReferenceId, Scoping};
    fn reference_scope_stable_key(
        file: &SourceFile,
        scoping: &Scoping,
        nodes: &AstNodes<'_>,
        reference_id: OxcReferenceId,
    ) -> String {
        let reference = scoping.get_reference(reference_id);
        let interner = crate::internal_core::test_stable_key_interner();
        interner
            .resolve(super::scope_stable_key(
                &interner,
                file,
                scoping,
                nodes,
                reference.scope_id(),
            ))
            .to_string()
    }

    use super::derive_ts_semantic_index;
    use crate::ts::parse::parse_ts_file;
    use oxc_allocator::Allocator;
    use oxc_ast::AstKind;
    use oxc_semantic::SemanticBuilder;
    use crate::analysis_neutral::symbol_graph::semantic::ScopeKind;
    use crate::analysis_api::SourceFile;
    use crate::internal_core::{FileId, Language};
    use std::path::PathBuf;

    fn source_file(source: &str) -> SourceFile {
        SourceFile::new(FileId::from_raw(0), PathBuf::from("src/scopes.ts"), "src/scopes.ts".to_string(), Language::TypeScript, source.to_string().into(), "test-hash".to_string())
    }

    #[test]
    fn emits_module_function_block_class_catch_loop_switch_type_and_namespace_scopes() {
        let source = r#"
namespace App {
    export interface Model { value: string }
    export type Alias = Model;
    export enum Choice { One }
}

class Widget {
    render() {
        for (const item of [1]) {
            switch (item) {
                case 1: break;
            }
        }
        try {
            throw new Error();
        } catch (err) {
            const local = err;
        }
    }
}

"#;
        let file = source_file(source);
        let allocator = Allocator::default();
        let parsed = parse_ts_file(&allocator, &file);
        let semantic = SemanticBuilder::new().build(parsed.program()).semantic;

        let output = derive_ts_semantic_index(
            &crate::internal_core::test_stable_key_interner(),
            &file,
            source,
            parsed.program(),
            semantic.scoping(),
            semantic.nodes(),
            false,
        );
        let kinds = output
            .scopes
            .iter()
            .map(|scope| scope.kind)
            .collect::<Vec<_>>();

        assert!(kinds.contains(&ScopeKind::Module));
        assert!(kinds.contains(&ScopeKind::Function));
        assert!(kinds.contains(&ScopeKind::Block));
        assert!(kinds.contains(&ScopeKind::Class));
        assert!(kinds.contains(&ScopeKind::Catch));
        assert!(kinds.contains(&ScopeKind::Loop));
        assert!(kinds.contains(&ScopeKind::Switch));
        assert!(kinds.contains(&ScopeKind::Type));
        assert!(kinds.contains(&ScopeKind::Namespace));
    }

    #[test]
    fn reference_scope_stable_key_uses_the_enclosing_oxc_scope_path() {
        let source = r#"
function outer(value: number) {
    {
        const value = 1;
        return value;
    }
}
"#;
        let file = source_file(source);
        let allocator = Allocator::default();
        let parsed = parse_ts_file(&allocator, &file);
        let semantic = SemanticBuilder::new().build(parsed.program()).semantic;
        let reference = semantic
            .nodes()
            .iter()
            .find_map(|node| {
                let AstKind::IdentifierReference(identifier) = node.kind() else {
                    return None;
                };
                (identifier.name == "value")
                    .then(|| identifier.reference_id.get())
                    .flatten()
            })
            .expect("fixture has a value reference");

        let stable_key =
            reference_scope_stable_key(&file, semantic.scoping(), semantic.nodes(), reference);

        assert!(stable_key.contains("src/scopes.ts"));
        assert!(stable_key.contains("block"));
    }
}

#[cfg(test)]
mod semantic_imports_exports {
    use super::derive_ts_semantic_index;
    use crate::ts::parse::parse_ts_file;
    use oxc_allocator::Allocator;
    use oxc_semantic::SemanticBuilder;
    use crate::analysis_neutral::symbol_graph::semantic::{ExportKind, SemanticImportKind, SemanticStatus};
    use crate::analysis_api::SourceFile;
    use crate::internal_core::{FileId, Language};
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn derive(source: &str) -> crate::analysis_neutral::symbol_graph::semantic::SemanticIndexOutput {
        let file = SourceFile::new(FileId::from_raw(0), PathBuf::from("src/imports.ts"), "src/imports.ts".to_string(), Language::TypeScript, source.to_string().into(), "test-hash".to_string());
        let allocator = Allocator::default();
        let parsed = parse_ts_file(&allocator, &file);
        let semantic = SemanticBuilder::new().build(parsed.program()).semantic;

        derive_ts_semantic_index(
            &crate::internal_core::test_stable_key_interner(),
            &file,
            source,
            parsed.program(),
            semantic.scoping(),
            semantic.nodes(),
            false,
        )
    }

    #[test]
    fn emits_static_import_rows_for_named_default_namespace_side_effect_and_type_only_forms() {
        let output = derive(
            r#"
import defaultThing from "pkg";
import { named as local, type TypeName } from "./mod";
import * as ns from "./ns";
import "./side-effect";
"#,
        );
        let kinds = output
            .semantic_imports
            .iter()
            .map(|fact| fact.kind)
            .collect::<Vec<_>>();

        assert!(kinds.contains(&SemanticImportKind::StaticDefault));
        assert!(kinds.contains(&SemanticImportKind::StaticNamed));
        assert!(kinds.contains(&SemanticImportKind::StaticNamespace));
        assert!(kinds.contains(&SemanticImportKind::SideEffect));
        assert!(kinds.contains(&SemanticImportKind::TypeOnly));
    }

    #[test]
    fn side_effect_import_stable_keys_include_import_path() {
        let output = derive(
            r#"
import "./setup-a";
import "./setup-b";
"#,
        );
        let side_effect_keys = output
            .semantic_imports
            .iter()
            .filter(|fact| fact.kind == SemanticImportKind::SideEffect)
            .map(|fact| {
                crate::internal_core::test_stable_key_interner()
                    .resolve(fact.stable_key)
                    .to_string()
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(side_effect_keys.len(), 2);
    }

    #[test]
    fn emits_export_rows_for_named_default_star_reexport_and_reexport_specifiers() {
        let output = derive(
            r#"
const local = 1;
export { local as renamed };
export default function main() {}
export * from "./star";
export { named as reexported } from "./mod";
"#,
        );
        let kinds = output
            .exports
            .iter()
            .map(|fact| fact.kind)
            .collect::<Vec<_>>();

        assert!(kinds.contains(&ExportKind::Named));
        assert!(kinds.contains(&ExportKind::Default));
        assert!(kinds.contains(&ExportKind::StarReexport));
        assert!(kinds.contains(&ExportKind::Namespace));
        assert!(
            output
                .aliases
                .iter()
                .any(|alias| alias.status == SemanticStatus::Unresolved)
        );
    }

    #[test]
    fn star_reexport_alias_stable_keys_include_reexport_target() {
        let output = derive(
            r#"
export * from "./a";
export * from "./b";
"#,
        );
        let reexport_keys = output
            .aliases
            .iter()
            .filter(|alias| {
                crate::internal_core::test_stable_key_interner()
                    .resolve(alias.source_symbol_stable_key)
                    .as_ref()
                    == "export:*"
            })
            .map(|alias| {
                crate::internal_core::test_stable_key_interner()
                    .resolve(alias.stable_key)
                    .to_string()
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(reexport_keys.len(), 2);
    }

    #[test]
    fn emits_conservative_rows_for_commonjs_and_dynamic_import_forms() {
        let output = derive(
            r#"
const req = require(moduleName);
module.exports = req;
exports.named = req;
const lazy = import(moduleName);
"#,
        );

        assert!(output.semantic_imports.iter().any(|fact| {
            fact.kind == SemanticImportKind::CommonJsRequire
                && fact.status == SemanticStatus::Dynamic
        }));
        assert!(output.semantic_imports.iter().any(|fact| {
            fact.kind == SemanticImportKind::DynamicImport && fact.status == SemanticStatus::Dynamic
        }));
        assert!(output.exports.iter().any(|fact| {
            fact.kind == ExportKind::CommonJsModuleExports
                && fact.status == SemanticStatus::Unsupported
        }));
        assert!(output.exports.iter().any(|fact| {
            fact.kind == ExportKind::CommonJsExportsProperty
                && fact.status == SemanticStatus::Unsupported
        }));
    }
}

#[cfg(test)]
mod semantic_resolution {
    use super::{derive_ts_file_symbols, derive_ts_semantic_index};
    use crate::ts::parse::parse_ts_file;
    use oxc_allocator::Allocator;
    use oxc_semantic::SemanticBuilder;
    use crate::analysis_neutral::symbol_graph::semantic::{
        ResolutionStepKind, SemanticIndexOutput, SemanticStatus,
    };
    use crate::analysis_neutral::symbol_graph::{LanguageSymbolOutput, model::SymbolGraphBuilder};
    use crate::analysis_api::{SourceFile, SymbolFact};
    use crate::internal_core::{FileId, Language};
    use std::path::PathBuf;

    fn derive(source: &str) -> crate::analysis_neutral::symbol_graph::semantic::SemanticIndexOutput {
        let file = SourceFile::new(FileId::from_raw(0), PathBuf::from("src/resolution.ts"), "src/resolution.ts".to_string(), Language::TypeScript, source.to_string().into(), "test-hash".to_string());
        let allocator = Allocator::default();
        let parsed = parse_ts_file(&allocator, &file);
        let semantic = SemanticBuilder::new().build(parsed.program()).semantic;

        derive_ts_semantic_index(
            &crate::internal_core::test_stable_key_interner(),
            &file,
            source,
            parsed.program(),
            semantic.scoping(),
            semantic.nodes(),
            true,
        )
    }

    fn derive_symbols_and_semantic(
        source: &str,
    ) -> (
        crate::internal_core::StableKeyInterner,
        Vec<SymbolFact>,
        SemanticIndexOutput,
    ) {
        let file = SourceFile::new(FileId::from_raw(0), PathBuf::from("src/resolution.ts"), "src/resolution.ts".to_string(), Language::TypeScript, source.to_string().into(), "test-hash".to_string());
        let interner = crate::internal_core::StableKeyInterner::default();
        let mut builder = SymbolGraphBuilder::new(interner.clone());
        let mut output = LanguageSymbolOutput::default();

        derive_ts_file_symbols(&interner, &mut builder, &mut output, &file, false);
        let symbol_output = builder.finish();

        (interner, symbol_output.symbols, output.semantic)
    }

    #[test]
    fn local_lexical_references_record_resolved_lookup_steps() {
        let output = derive(
            r#"
const value = 1;
export const doubled = value + value;
"#,
        );

        assert!(output.resolutions.iter().any(|resolution| {
            resolution.step == ResolutionStepKind::LexicalLookup
                && resolution.status == SemanticStatus::Resolved
                && crate::internal_core::test_stable_key_interner()
                    .resolve(resolution.source_stable_key)
                    .contains("value")
        }));
    }

    #[test]
    fn import_alias_references_record_alias_and_module_lookup_steps() {
        let output = derive(
            r#"
import { thing as localThing } from "./thing";
export const used = localThing;
"#,
        );

        assert!(
            output
                .resolutions
                .iter()
                .any(|resolution| resolution.step == ResolutionStepKind::ImportAliasLookup)
        );
        assert!(
            output
                .resolutions
                .iter()
                .any(|resolution| resolution.step == ResolutionStepKind::ModuleLookup)
        );
    }

    #[test]
    fn unresolved_references_record_unknown_fallback_steps() {
        let output = derive("export const value = missingGlobal;\n");

        assert!(output.resolutions.iter().any(|resolution| {
            resolution.step == ResolutionStepKind::UnknownFallback
                && resolution.status == SemanticStatus::Unresolved
                && crate::internal_core::test_stable_key_interner()
                    .resolve(resolution.source_stable_key)
                    .contains("missingGlobal")
        }));
    }

    #[test]
    fn stable_export_identities_use_native_discriminator() {
        let output = derive("export const value = 1;\n");

        assert!(output.stable_exports.iter().any(|identity| {
            identity.export_name == "value"
                && identity.generated_discriminator.as_deref() == Some("native")
                && crate::internal_core::test_stable_key_interner()
                    .resolve(identity.symbol_stable_key)
                    .contains("value")
        }));
    }

    #[test]
    fn stable_export_symbol_key_matches_exported_symbol_fact_key() {
        let (_interner, symbols, semantic) =
            derive_symbols_and_semantic("export const value = 1;\n");
        let exported_symbol = symbols
            .iter()
            .find(|symbol| symbol.name == "value" && symbol.is_exported)
            .expect("exported symbol exists");
        let stable_export = semantic
            .stable_exports
            .iter()
            .find(|identity| identity.export_name == "value")
            .expect("stable export exists");

        assert_eq!(stable_export.symbol_stable_key, exported_symbol.stable_key);
    }

    #[test]
    fn symbols_only_semantic_derivation_does_not_emit_reference_keyed_resolutions() {
        let (_interner, _symbols, semantic) =
            derive_symbols_and_semantic("const value = 1;\nexport const doubled = value;\n");

        assert!(semantic.resolutions.is_empty());
    }
}

#[cfg(test)]
mod value_functions {
    use super::derive_ts_semantic_index;
    use crate::analysis_api::SourceFile;
    use crate::internal_core::{FileId, Language};
    use crate::ts::parse::parse_ts_file;
    use oxc_allocator::Allocator;
    use oxc_semantic::SemanticBuilder;
    use std::path::PathBuf;

    /// Returns `(binding text, function text)` for every value-function row.
    fn links(path: &str, source: &str) -> Vec<(String, String)> {
        let file = SourceFile::new(
            FileId::from_raw(0),
            PathBuf::from(path),
            path.to_string(),
            Language::TypeScript,
            source.to_string().into(),
            "test-hash".to_string(),
        );
        let allocator = Allocator::default();
        let parsed = parse_ts_file(&allocator, &file);
        let semantic = SemanticBuilder::new().build(parsed.program()).semantic;
        let output = derive_ts_semantic_index(
            &crate::internal_core::test_stable_key_interner(),
            &file,
            source,
            parsed.program(),
            semantic.scoping(),
            semantic.nodes(),
            false,
        );
        let text = |span: &crate::internal_core::Span| {
            source[span.start_byte as usize..span.end_byte as usize].to_string()
        };
        output
            .value_functions
            .iter()
            .map(|row| (text(&row.binding), text(&row.function)))
            .collect()
    }

    #[test]
    fn links_const_bindings_to_their_function_literals_at_any_depth() {
        let found = links(
            "src/value.ts",
            "const top = () => 1;\nfunction outer() {\n  const inner = function () { return 2; };\n  return inner();\n}\n",
        );

        assert_eq!(
            found,
            vec![
                ("top".to_string(), "() => 1".to_string()),
                (
                    "inner".to_string(),
                    "function () { return 2; }".to_string()
                ),
            ]
        );
    }

    #[test]
    fn links_unwritten_function_local_let_and_var() {
        let found = links(
            "src/value.js",
            "function outer() {\n  let a = () => 1;\n  var b = () => 2;\n  return a() + b();\n}\n",
        );

        assert_eq!(
            found
                .iter()
                .map(|(binding, _)| binding.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn skips_wrapped_invoked_and_conditional_initializers() {
        let found = links(
            "src/value.ts",
            "const wrapped = wrap(() => 1);\nconst invoked = (function () { return () => 2; })();\nconst picked = flag ? () => 3 : () => 4;\nconst asserted = (() => 5) as Fn;\n",
        );

        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn skips_mutable_bindings_that_another_value_can_reach() {
        let found = links(
            "src/value.js",
            "var scriptGlobal = () => 1;\nfunction outer() {\n  let reassigned = () => 1;\n  reassigned = () => 2;\n  var redeclared = () => 3;\n  var redeclared = () => 4;\n  return reassigned() + redeclared();\n}\n",
        );

        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn module_scope_let_links_only_in_files_with_module_syntax() {
        let module = links(
            "src/value.js",
            "export let handler = () => 1;\nhandler();\n",
        );
        let with_eval = links(
            "src/value.js",
            "function outer() {\n  let local = () => 1;\n  eval(code);\n  return local();\n}\n",
        );

        assert_eq!(module.len(), 1, "{module:?}");
        assert!(with_eval.is_empty(), "{with_eval:?}");
    }
}
