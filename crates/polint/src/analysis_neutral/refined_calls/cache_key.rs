use crate::analysis_api::{Digest, DigestKind, InputComponent, InputSnapshot};

pub const REFINED_CALLS_SCHEMA_LABEL: &str = "refined-call-facts-1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefinedCallsProviderParameters {
    pub(crate) precision_tier: &'static str,
    pub(crate) direct_mirror: bool,
    pub(crate) solver_projection: &'static str,
    pub(crate) retired_heuristic_producers: bool,
}

impl RefinedCallsProviderParameters {
    pub fn deterministic_default() -> Self {
        Self {
            precision_tier: "setup-aware",
            direct_mirror: true,
            solver_projection: "solver_derived_edges_at_derived_site",
            retired_heuristic_producers: true,
        }
    }
}

pub fn refined_calls_provider_parameter_digest() -> Digest {
    refined_calls_provider_parameter_digest_for_settings(
        &RefinedCallsProviderParameters::deterministic_default(),
    )
}

pub fn refined_calls_provider_parameter_digest_for_snapshot(
    input_snapshot: &InputSnapshot,
    upstream_output_digests: &[Digest],
) -> Digest {
    let mut parts = parameter_parts(&RefinedCallsProviderParameters::deterministic_default());
    parts.push(format!("config={}", input_snapshot.config.digest));
    extend_component_parts(
        &mut parts,
        "go_lifecycle",
        &input_snapshot.go_lifecycle.components,
    );
    extend_component_parts(
        &mut parts,
        "ts_js_lifecycle",
        &input_snapshot.ts_js_lifecycle.components,
    );
    extend_component_parts(&mut parts, "extension", &input_snapshot.extensions);
    extend_component_parts(&mut parts, "model", &input_snapshot.models);
    extend_component_parts(&mut parts, "tool", &input_snapshot.tool_invocations);
    parts.extend(
        upstream_output_digests
            .iter()
            .map(|digest| format!("upstream={digest}")),
    );
    parts.sort();
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Digest::from_parts(
        DigestKind::ProviderParameters,
        "refined_calls_inputs",
        &refs,
    )
}

fn refined_calls_provider_parameter_digest_for_settings(
    settings: &RefinedCallsProviderParameters,
) -> Digest {
    let parts = parameter_parts(settings);
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Digest::from_parts(
        DigestKind::ProviderParameters,
        "refined_calls_parameters",
        &refs,
    )
}

fn parameter_parts(settings: &RefinedCallsProviderParameters) -> Vec<String> {
    vec![
        REFINED_CALLS_SCHEMA_LABEL.to_string(),
        format!("precision_tier={}", settings.precision_tier),
        format!("direct_mirror={}", settings.direct_mirror),
        format!("solver_projection={}", settings.solver_projection),
        format!(
            "retired_heuristic_producers={}",
            settings.retired_heuristic_producers
        ),
    ]
}

fn extend_component_parts(parts: &mut Vec<String>, prefix: &str, components: &[InputComponent]) {
    if components.is_empty() {
        parts.push(format!("{prefix}=absent"));
        return;
    }
    parts.extend(components.iter().map(|component| {
        format!(
            "{prefix}:{}:{:?}:{}",
            component.name, component.status, component.digest
        )
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refined_calls_parameters_include_schema_and_planned_refinement_knobs() {
        let digest = refined_calls_provider_parameter_digest();

        assert_eq!(
            digest,
            refined_calls_provider_parameter_digest_for_settings(
                &RefinedCallsProviderParameters::deterministic_default()
            )
        );
    }
}
