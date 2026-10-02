use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Expr, FnArg, GenericArgument, Ident, ItemFn, Lit, Meta, Pat, PatIdent, PathArguments,
    PathSegment, ReturnType, Signature, Type, parse_macro_input, spanned::Spanned,
};

#[proc_macro_attribute]
pub fn rule(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args with syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated);
    let input = parse_macro_input!(input as ItemFn);

    match expand_rule(args.into_iter().collect(), input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[cfg_attr(test, derive(Debug))]
struct RuleArgs {
    id: String,
    description: String,
    severity: Ident,
    /// The `RuleKind` variant identifier (`Check` or `Review`); defaults to `Check`.
    kind: Ident,
}

struct ViewParam {
    ident: Ident,
    ty: Type,
    view_type: Ident,
    capability_method: Ident,
    capability_name: String,
    canonical_path: String,
    /// Requested as `Option<View<'_>>`: the rule runs without the view when
    /// the capability is unavailable.
    optional: bool,
}

fn expand_rule(args: Vec<Meta>, input: ItemFn) -> syn::Result<proc_macro2::TokenStream> {
    let rule_args = parse_rule_args(args)?;
    let vis = input.vis.clone();
    let fn_name = input.sig.ident.clone();
    let run_name = format_ident!("__polint_run_{fn_name}");
    let block = input.block;
    let attrs = input.attrs;
    let output = input.sig.output.clone();
    validate_signature_shape(&input.sig)?;
    validate_return_type(&output)?;

    let mut inputs = input.sig.inputs.iter();
    let Some(first) = inputs.next() else {
        return Err(syn::Error::new(
            input.sig.span(),
            "polint rules must take a mutable RuleCtx reference as the first parameter",
        ));
    };
    let ctx_ident = parse_ctx_param(first)?;
    let mut view_params = Vec::new();
    for input in inputs {
        view_params.push(parse_view_param(input)?);
    }
    let capability_methods = view_params
        .iter()
        .map(|param| &param.capability_method)
        .collect::<Vec<_>>();
    let view_bindings = view_params.iter().map(|param| {
        let ident = &param.ident;
        let ty = &param.ty;
        let view_type = &param.view_type;
        if param.optional {
            quote! {
                let #ident: #ty = <::polint::sdk::facts::#view_type<'_> as ::polint::sdk::__private::FactView<'_>>::build_optional(db);
            }
        } else {
            quote! {
                let #ident: #ty = <::polint::sdk::facts::#view_type<'_> as ::polint::sdk::__private::FactView<'_>>::build(db);
            }
        }
    });
    let view_idents = view_params
        .iter()
        .map(|param| &param.ident)
        .collect::<Vec<_>>();
    let fact_view_requirements = view_params.iter().map(|param| {
        let view_type = param.view_type.to_string();
        let capability = param.capability_name.as_str();
        let canonical_path = param.canonical_path.as_str();
        let parameter_name = param.ident.to_string();
        quote! {
            ::polint::sdk::__private::FactViewRequirement::generated(
                #view_type,
                #canonical_path,
                #capability,
                #parameter_name,
            )
        }
    });
    let run_inputs = std::iter::once(first.clone())
        .chain(input.sig.inputs.iter().skip(1).cloned())
        .collect::<syn::punctuated::Punctuated<FnArg, syn::Token![,]>>();
    let id = rule_args.id;
    let description = rule_args.description;
    let severity = rule_args.severity;
    let kind = rule_args.kind;

    Ok(quote! {
        #vis fn #fn_name() -> ::polint::sdk::prelude::Rule {
            ::polint::sdk::__private::make_rule_with_manifest(
                || {
                    ::polint::sdk::__private::RuleMeta {
                        id: #id.to_string(),
                        description: #description.to_string(),
                        severity: ::polint::sdk::prelude::Severity::#severity,
                        kind: ::polint::sdk::__private::RuleKind::#kind,
                    }
                },
                || {
                    ::polint::sdk::__private::Capabilities::new()#(.#capability_methods())*
                },
                vec![#(#fact_view_requirements),*],
                |db: &::polint::sdk::__private::AnalysisDb,
                 #ctx_ident: &mut ::polint::sdk::prelude::RuleCtx<'_>|
                 -> ::polint::sdk::prelude::RuleResult {
                    #(#view_bindings)*
                    #run_name(#ctx_ident, #(#view_idents),*)
                },
            )
        }

        #(#attrs)*
        fn #run_name(#run_inputs) #output #block
    })
}

fn validate_signature_shape(sig: &Signature) -> syn::Result<()> {
    if sig.constness.is_some()
        || sig.asyncness.is_some()
        || sig.unsafety.is_some()
        || sig.abi.is_some()
        || sig.variadic.is_some()
        || !sig.generics.params.is_empty()
        || sig.generics.where_clause.is_some()
    {
        return Err(syn::Error::new(
            sig.span(),
            "polint rule functions must be plain non-generic sync functions",
        ));
    }
    Ok(())
}

fn parse_rule_args(args: Vec<Meta>) -> syn::Result<RuleArgs> {
    let mut id = None;
    let mut description = None;
    let mut severity = None;
    let mut kind = None;
    for meta in args {
        let Meta::NameValue(name_value) = meta else {
            return Err(syn::Error::new(
                meta.span(),
                "expected name-value rule attribute arguments",
            ));
        };
        let Some(name) = name_value.path.get_ident().map(ToString::to_string) else {
            return Err(syn::Error::new(
                name_value.path.span(),
                "expected simple argument name",
            ));
        };
        let value = string_lit(&name_value.value)?;
        match name.as_str() {
            "id" => id = Some(value),
            "description" => description = Some(value),
            "severity" => severity = Some(parse_severity(&value, name_value.value.span())?),
            "kind" => kind = Some(parse_kind(&value, name_value.value.span())?),
            _ => {
                return Err(syn::Error::new(
                    name_value.path.span(),
                    "unknown polint rule attribute argument",
                ));
            }
        }
    }
    Ok(RuleArgs {
        id: id.ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing `id`"))?,
        description: description.ok_or_else(|| {
            syn::Error::new(proc_macro2::Span::call_site(), "missing `description`")
        })?,
        severity: severity
            .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing `severity`"))?,
        kind: kind.unwrap_or_else(|| format_ident!("Check")),
    })
}

fn string_lit(expr: &Expr) -> syn::Result<String> {
    let Expr::Lit(expr_lit) = expr else {
        return Err(syn::Error::new(expr.span(), "expected string literal"));
    };
    let Lit::Str(lit) = &expr_lit.lit else {
        return Err(syn::Error::new(expr.span(), "expected string literal"));
    };
    Ok(lit.value())
}

fn parse_severity(value: &str, span: proc_macro2::Span) -> syn::Result<Ident> {
    match value {
        "error" => Ok(format_ident!("Error")),
        "warn" | "warning" => Ok(format_ident!("Warn")),
        "info" => Ok(format_ident!("Info")),
        _ => Err(syn::Error::new(
            span,
            "severity must be one of `error`, `warn`, or `info`",
        )),
    }
}

fn parse_kind(value: &str, span: proc_macro2::Span) -> syn::Result<Ident> {
    match value {
        "check" => Ok(format_ident!("Check")),
        "review" => Ok(format_ident!("Review")),
        _ => Err(syn::Error::new(span, "kind must be `check` or `review`")),
    }
}

fn validate_return_type(output: &ReturnType) -> syn::Result<()> {
    let ReturnType::Type(_, ty) = output else {
        return Err(syn::Error::new(
            output.span(),
            "polint rule functions must return RuleResult",
        ));
    };
    let Type::Path(path) = ty.as_ref() else {
        return Err(syn::Error::new(
            ty.span(),
            "polint rule functions must return RuleResult",
        ));
    };
    let Some(segment) = path.path.segments.last() else {
        return Err(syn::Error::new(
            ty.span(),
            "polint rule functions must return RuleResult",
        ));
    };
    if segment.ident != "RuleResult"
        || !path_is_unqualified_or_under(&path.path, "RuleResult", &[&["polint", "sdk", "prelude"]])
        || !has_no_or_unit_result_argument(segment)
    {
        return Err(syn::Error::new(
            ty.span(),
            "polint rule functions must return RuleResult or RuleResult<()>",
        ));
    }
    Ok(())
}

fn parse_ctx_param(arg: &FnArg) -> syn::Result<Ident> {
    let FnArg::Typed(pat_type) = arg else {
        return Err(syn::Error::new(arg.span(), "methods are not supported"));
    };
    let Pat::Ident(PatIdent { ident, .. }) = pat_type.pat.as_ref() else {
        return Err(syn::Error::new(
            pat_type.pat.span(),
            "the RuleCtx parameter must be a simple identifier",
        ));
    };
    let Type::Reference(reference) = pat_type.ty.as_ref() else {
        return Err(syn::Error::new(
            pat_type.ty.span(),
            "the first parameter must be a mutable RuleCtx reference",
        ));
    };
    if reference.mutability.is_none() {
        return Err(syn::Error::new(
            reference.span(),
            "the first parameter must be a mutable RuleCtx reference",
        ));
    }
    let Type::Path(path) = reference.elem.as_ref() else {
        return Err(syn::Error::new(
            reference.elem.span(),
            "the first parameter must be a mutable RuleCtx reference",
        ));
    };
    let Some(segment) = path.path.segments.last() else {
        return Err(syn::Error::new(
            path.span(),
            "the first parameter must be a mutable RuleCtx reference",
        ));
    };
    if segment.ident != "RuleCtx"
        || !path_is_unqualified_or_under(&path.path, "RuleCtx", &[&["polint", "sdk", "prelude"]])
        || !has_placeholder_lifetime_argument(segment)
    {
        return Err(syn::Error::new(
            pat_type.ty.span(),
            "the first parameter must be &mut RuleCtx<'_>",
        ));
    }
    Ok(ident.clone())
}

fn parse_view_param(arg: &FnArg) -> syn::Result<ViewParam> {
    let FnArg::Typed(pat_type) = arg else {
        return Err(syn::Error::new(arg.span(), "methods are not supported"));
    };
    let Pat::Ident(PatIdent {
        ident, mutability, ..
    }) = pat_type.pat.as_ref()
    else {
        return Err(syn::Error::new(
            pat_type.pat.span(),
            "fact-view parameters must use simple identifiers",
        ));
    };
    if mutability.is_some() {
        return Err(syn::Error::new(
            pat_type.pat.span(),
            "fact-view parameters cannot be `mut`",
        ));
    }
    let (view_ty, optional) = match optional_inner_type(pat_type.ty.as_ref()) {
        Some(inner) => (inner, true),
        None => (pat_type.ty.as_ref(), false),
    };
    let (view_type, capability_method, capability_name, canonical_path) =
        capability_for_type(view_ty)?;
    let capability_method = if optional {
        match capability_name.as_str() {
            "call_graph" | "go_types" => format_ident!("{}_optional", capability_name),
            _ => {
                return Err(syn::Error::new(
                    pat_type.ty.span(),
                    "only the CallGraph and GoTypes views can be requested optionally",
                ));
            }
        }
    } else {
        capability_method
    };
    Ok(ViewParam {
        ident: ident.clone(),
        ty: (*pat_type.ty).clone(),
        view_type,
        capability_method,
        capability_name,
        canonical_path,
        optional,
    })
}

/// The `T` of an `Option<T>` parameter type, written as `Option`,
/// `std::option::Option` or `core::option::Option`.
fn optional_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segments = path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    let is_option = matches!(
        segments
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice(),
        ["Option"] | ["std", "option", "Option"] | ["core", "option", "Option"]
    );
    if !is_option || path.qself.is_some() {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &path.path.segments.last()?.arguments else {
        return None;
    };
    match arguments.args.iter().collect::<Vec<_>>().as_slice() {
        [GenericArgument::Type(inner)] => Some(inner),
        _ => None,
    }
}

fn capability_for_type(ty: &Type) -> syn::Result<(Ident, Ident, String, String)> {
    let Type::Path(path) = ty else {
        return Err(syn::Error::new(
            ty.span(),
            "unsupported polint fact view parameter",
        ));
    };
    let Some(segment) = path.path.segments.last() else {
        return Err(syn::Error::new(
            ty.span(),
            "unsupported polint fact view parameter",
        ));
    };
    if !path_is_unqualified_or_under(
        &path.path,
        &segment.ident.to_string(),
        &[&["polint", "sdk", "facts"], &["polint", "sdk", "prelude"]],
    ) {
        return Err(syn::Error::new(
            ty.span(),
            "fact-view parameters must use canonical polint SDK fact views",
        ));
    }
    if !has_placeholder_lifetime_argument(segment) {
        return Err(syn::Error::new(
            ty.span(),
            "fact-view parameters must use concrete SDK views like Imports<'_>",
        ));
    }
    let method = match segment.ident.to_string().as_str() {
        "SourceFiles" | "Packages" | "Functions" | "GoTypeDecls" => "syntax",
        "Imports" => "imports",
        "ResolvedImports" => "resolved_imports",
        "ModuleGraphFacts" => "module_graph",
        "Symbols" => "symbols",
        "References" => "references",
        "Events" => "events",
        "Calls" => "calls",
        "ControlFlow" => "control_flow",
        "Cfg" => "cfg",
        "CallGraph" => "call_graph",
        "GoTypes" => "go_types",
        "DataFlow" => "dataflow",
        "GoTests" => "go_tests",
        "BranchObligations" => "branch_obligations",
        "CoverageFacts" => "coverage_facts",
        "TestSuiteMetrics" => "test_suite_metrics",
        "FileMetrics" => "file_metrics",
        "FunctionMetrics" => "function_metrics",
        "ComplexityMetrics" => "complexity_metrics",
        "TsComponents" => "ts_components",
        "TsClasses" => "ts_classes",
        "StringLiterals" => "string_literals",
        "JsxAttributes" => "jsx_attributes",
        "ChangedFiles" => "changeset",
        _ => {
            return Err(syn::Error::new(
                segment.ident.span(),
                "unsupported polint fact view parameter",
            ));
        }
    };
    Ok((
        segment.ident.clone(),
        format_ident!("{method}"),
        method.to_string(),
        format!("polint::sdk::facts::{}<'_>", segment.ident),
    ))
}

fn path_is_unqualified_or_under(path: &syn::Path, item: &str, prefixes: &[&[&str]]) -> bool {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    if segments == [item] {
        return true;
    }
    prefixes.iter().any(|prefix| {
        segments.len() == prefix.len() + 1
            && segments.last().is_some_and(|segment| segment == item)
            && segments
                .iter()
                .take(prefix.len())
                .map(String::as_str)
                .eq(prefix.iter().copied())
    })
}

fn has_no_or_unit_result_argument(segment: &PathSegment) -> bool {
    match &segment.arguments {
        PathArguments::None => true,
        PathArguments::AngleBracketed(arguments) => {
            arguments.args.len() == 1
                && matches!(
                    arguments.args.first(),
                    Some(GenericArgument::Type(Type::Tuple(tuple))) if tuple.elems.is_empty()
                )
        }
        PathArguments::Parenthesized(_) => false,
    }
}

fn has_placeholder_lifetime_argument(segment: &PathSegment) -> bool {
    match &segment.arguments {
        PathArguments::AngleBracketed(arguments) => {
            arguments.args.len() == 1
                && matches!(
                    arguments.args.first(),
                    Some(GenericArgument::Lifetime(lifetime)) if lifetime.ident == "_"
                )
        }
        PathArguments::None | PathArguments::Parenthesized(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability(type_source: &str) -> String {
        let ty = syn::parse_str::<Type>(type_source).unwrap();
        capability_for_type(&ty).unwrap().1.to_string()
    }

    fn canonical_path(type_source: &str) -> String {
        let ty = syn::parse_str::<Type>(type_source).unwrap();
        capability_for_type(&ty).unwrap().3
    }

    fn first_arg(source: &str) -> FnArg {
        syn::parse_str::<FnArg>(source).unwrap()
    }

    fn return_type(source: &str) -> ReturnType {
        syn::parse_str::<syn::Signature>(&format!("fn rule() {source}"))
            .unwrap()
            .output
    }

    fn signature(source: &str) -> Signature {
        syn::parse_str::<syn::Signature>(source).unwrap()
    }

    fn rule_args(source: &str) -> syn::Result<RuleArgs> {
        use syn::parse::Parser as _;
        let parser = syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated;
        let metas = parser.parse_str(source).unwrap();
        parse_rule_args(metas.into_iter().collect())
    }

    #[test]
    fn parse_rule_args_kind_defaults_to_check() {
        let args = rule_args(r#"id = "local/x", description = "d", severity = "warn""#).unwrap();
        assert_eq!(args.kind.to_string(), "Check");
    }

    #[test]
    fn parse_rule_args_accepts_review_kind() {
        let args =
            rule_args(r#"id = "review/x", description = "d", severity = "warn", kind = "review""#)
                .unwrap();
        assert_eq!(args.kind.to_string(), "Review");
    }

    #[test]
    fn parse_rule_args_accepts_explicit_check_kind() {
        let args =
            rule_args(r#"id = "local/x", description = "d", severity = "warn", kind = "check""#)
                .unwrap();
        assert_eq!(args.kind.to_string(), "Check");
    }

    #[test]
    fn parse_rule_args_rejects_unknown_kind() {
        let error =
            rule_args(r#"id = "local/x", description = "d", severity = "warn", kind = "audit""#)
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("kind must be `check` or `review`")
        );
    }

    #[test]
    fn validate_signature_shape_rejects_non_plain_functions() {
        assert!(validate_signature_shape(&signature("fn rule() -> RuleResult")).is_ok());

        let async_rule =
            validate_signature_shape(&signature("async fn rule() -> RuleResult")).unwrap_err();
        assert!(
            async_rule
                .to_string()
                .contains("plain non-generic sync functions")
        );

        let generic_rule =
            validate_signature_shape(&signature("fn rule<'a>() -> RuleResult")).unwrap_err();
        assert!(
            generic_rule
                .to_string()
                .contains("plain non-generic sync functions")
        );
    }

    #[test]
    fn capability_for_type_maps_supported_fact_views() {
        assert_eq!(capability("SourceFiles<'_>"), "syntax");
        assert_eq!(capability("Packages<'_>"), "syntax");
        assert_eq!(capability("Functions<'_>"), "syntax");
        assert_eq!(capability("Imports<'_>"), "imports");
        assert_eq!(capability("GoTests<'_>"), "go_tests");
        assert_eq!(capability("BranchObligations<'_>"), "branch_obligations");
        assert_eq!(capability("CoverageFacts<'_>"), "coverage_facts");
        assert_eq!(capability("TestSuiteMetrics<'_>"), "test_suite_metrics");
        assert_eq!(capability("DataFlow<'_>"), "dataflow");
        assert_eq!(capability("Events<'_>"), "events");
        assert_eq!(capability("Calls<'_>"), "calls");
        assert_eq!(capability("ControlFlow<'_>"), "control_flow");
        assert_eq!(capability("Cfg<'_>"), "cfg");
        assert_eq!(capability("CallGraph<'_>"), "call_graph");
        assert_eq!(capability("GoTypes<'_>"), "go_types");
        assert_eq!(capability("FileMetrics<'_>"), "file_metrics");
        assert_eq!(capability("FunctionMetrics<'_>"), "function_metrics");
        assert_eq!(capability("ComplexityMetrics<'_>"), "complexity_metrics");
        assert_eq!(capability("ResolvedImports<'_>"), "resolved_imports");
        assert_eq!(capability("TsComponents<'_>"), "ts_components");
        assert_eq!(capability("TsClasses<'_>"), "ts_classes");
        // capability("polint::sdk::facts::ModuleGraphFacts<'_>") maps to module_graph.
        assert_eq!(
            capability("polint::sdk::facts::ModuleGraphFacts<'_>"),
            "module_graph"
        );
        assert_eq!(capability("Symbols<'_>"), "symbols");
        assert_eq!(capability("References<'_>"), "references");
        assert_eq!(capability("polint::sdk::facts::Symbols<'_>"), "symbols");
        assert_eq!(
            capability("polint::sdk::prelude::References<'_>"),
            "references"
        );
        assert_eq!(capability("StringLiterals<'_>"), "string_literals");
        assert_eq!(
            capability("polint::sdk::facts::JsxAttributes<'_>"),
            "jsx_attributes"
        );
        assert_eq!(capability("ChangedFiles<'_>"), "changeset");
        assert_eq!(
            canonical_path("polint::sdk::prelude::Imports<'_>"),
            "polint::sdk::facts::Imports<'_>"
        );
        assert_eq!(
            canonical_path("polint::sdk::prelude::ControlFlow<'_>"),
            "polint::sdk::facts::ControlFlow<'_>"
        );
    }

    #[test]
    fn optional_call_graph_and_go_types_views_request_their_capability_optionally() {
        for (source, method, capability) in [
            (
                "graph: Option<CallGraph<'_>>",
                "call_graph_optional",
                "call_graph",
            ),
            (
                "types: std::option::Option<GoTypes<'_>>",
                "go_types_optional",
                "go_types",
            ),
        ] {
            let param = parse_view_param(&first_arg(source)).unwrap();
            assert!(param.optional, "{source}");
            assert_eq!(param.capability_method.to_string(), method);
            assert_eq!(param.capability_name, capability);
        }
        let required = parse_view_param(&first_arg("graph: CallGraph<'_>")).unwrap();
        assert!(!required.optional);
        assert_eq!(required.capability_method.to_string(), "call_graph");
    }

    #[test]
    fn only_call_graph_and_go_types_views_can_be_optional() {
        let error = parse_view_param(&first_arg("calls: Option<Calls<'_>>"))
            .err()
            .expect("an optional Calls view is rejected");
        assert!(
            error
                .to_string()
                .contains("only the CallGraph and GoTypes views can be requested optionally")
        );
    }

    #[test]
    fn capability_for_type_rejects_non_canonical_qualified_fact_paths() {
        let ty = syn::parse_str::<Type>("local::Imports<'_>").unwrap();
        let error = capability_for_type(&ty).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("canonical polint SDK fact views")
        );
    }

    #[test]
    fn capability_for_type_rejects_unknown_fact_views() {
        let ty = syn::parse_str::<Type>("UserDefinedFacts<'_>").unwrap();
        let error = capability_for_type(&ty).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unsupported polint fact view parameter")
        );
    }

    #[test]
    fn capability_for_type_requires_explicit_lifetime_argument() {
        let ty = syn::parse_str::<Type>("Imports").unwrap();
        let error = capability_for_type(&ty).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("concrete SDK views like Imports<'_>")
        );

        let static_lifetime = syn::parse_str::<Type>("Imports<'static>").unwrap();
        let error = capability_for_type(&static_lifetime).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("concrete SDK views like Imports<'_>")
        );
    }

    #[test]
    fn parse_ctx_param_requires_mutable_rule_ctx_reference() {
        assert!(parse_ctx_param(&first_arg("ctx: &mut RuleCtx<'_>")).is_ok());
        assert!(parse_ctx_param(&first_arg("ctx: &mut polint::sdk::prelude::RuleCtx<'_>")).is_ok());

        let shared = parse_ctx_param(&first_arg("ctx: &RuleCtx<'_>")).unwrap_err();
        assert!(shared.to_string().contains("mutable RuleCtx reference"));

        let wrong_type = parse_ctx_param(&first_arg("ctx: &mut NotRuleCtx<'_>")).unwrap_err();
        assert!(wrong_type.to_string().contains("&mut RuleCtx<'_>"));

        let missing_lifetime = parse_ctx_param(&first_arg("ctx: &mut RuleCtx")).unwrap_err();
        assert!(missing_lifetime.to_string().contains("&mut RuleCtx<'_>"));

        let static_lifetime =
            parse_ctx_param(&first_arg("ctx: &mut RuleCtx<'static>")).unwrap_err();
        assert!(static_lifetime.to_string().contains("&mut RuleCtx<'_>"));
    }

    #[test]
    fn validate_return_type_requires_rule_result() {
        assert!(validate_return_type(&return_type("-> RuleResult")).is_ok());
        assert!(validate_return_type(&return_type("-> RuleResult<()>")).is_ok());
        assert!(validate_return_type(&return_type("-> polint::sdk::prelude::RuleResult")).is_ok());

        let error = validate_return_type(&return_type("-> Result<(), RuleError>")).unwrap_err();
        assert!(error.to_string().contains("must return RuleResult"));

        let value_return = validate_return_type(&return_type("-> RuleResult<usize>")).unwrap_err();
        assert!(value_return.to_string().contains("RuleResult<()>"));
    }
}
