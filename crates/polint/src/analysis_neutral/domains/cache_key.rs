use super::core::{
    ConstantDomain, InitializednessDomain, NilnessDomain, ReachabilityDomain, StringDomain,
    TruthinessDomain,
};
use super::lattice::AbstractDomain;
use super::solver::SolverPolicy;
use crate::analysis_api::{Digest, DigestKind};

pub const ABSTRACT_DOMAIN_REDUCTION_GRAPH_VERSION: u32 = 1;
pub const ABSTRACT_DOMAIN_SCHEMA_LABEL: &str = "abstract-domain-facts-1";

pub fn abstract_domains_provider_parameter_digest() -> Digest {
    let policy = SolverPolicy::deterministic();
    abstract_domains_provider_parameter_digest_for_policy(
        policy.reduction_rounds,
        policy.budget.widening_fuel,
        policy.budget.max_iterations,
        policy.budget.max_carried_facts,
    )
}

fn abstract_domains_provider_parameter_digest_for_policy(
    max_reduction_rounds: u32,
    widening_fuel: u32,
    iteration_budget: u32,
    carried_fact_budget: u64,
) -> Digest {
    let parts = [
        format!("schema={ABSTRACT_DOMAIN_SCHEMA_LABEL}:1"),
        format!(
            "domain={}:{}",
            ReachabilityDomain::ID,
            ReachabilityDomain::VERSION
        ),
        format!("domain={}:{}", NilnessDomain::ID, NilnessDomain::VERSION),
        format!(
            "domain={}:{}",
            TruthinessDomain::ID,
            TruthinessDomain::VERSION
        ),
        format!("domain={}:{}", ConstantDomain::ID, ConstantDomain::VERSION),
        format!("domain={}:{}", StringDomain::ID, StringDomain::VERSION),
        format!(
            "domain={}:{}",
            InitializednessDomain::ID,
            InitializednessDomain::VERSION
        ),
        format!(
            "slot_versions=reachability:{};nilness:{};truthiness:{};constants:{};strings:{};initializedness:{}",
            ReachabilityDomain::VERSION,
            NilnessDomain::VERSION,
            TruthinessDomain::VERSION,
            ConstantDomain::VERSION,
            StringDomain::VERSION,
            InitializednessDomain::VERSION
        ),
        format!("reduction_graph_version={ABSTRACT_DOMAIN_REDUCTION_GRAPH_VERSION}"),
        format!("max_reduction_rounds={max_reduction_rounds}"),
        format!("widening_fuel={widening_fuel}"),
        format!("iteration_budget={iteration_budget}"),
        format!("carried_fact_budget={carried_fact_budget}"),
    ];
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Digest::from_parts(
        DigestKind::ProviderParameters,
        "abstract_domains_parameters",
        &refs,
    )
}

#[cfg(test)]
pub fn abstract_domains_provider_parameter_digest_for_test(
    max_reduction_rounds: u32,
    widening_fuel: u32,
    iteration_budget: u32,
    carried_fact_budget: u64,
) -> Digest {
    abstract_domains_provider_parameter_digest_for_policy(
        max_reduction_rounds,
        widening_fuel,
        iteration_budget,
        carried_fact_budget,
    )
}

#[cfg(test)]
mod abstract_domains_provider_parameters {
    use super::*;

    #[test]
    fn abstract_domains_provider_parameters_change_when_policy_inputs_change() {
        let baseline = abstract_domains_provider_parameter_digest();
        let policy = SolverPolicy::deterministic();
        let changed = abstract_domains_provider_parameter_digest_for_test(
            99,
            8,
            10_000,
            policy.budget.max_carried_facts,
        );
        let carried_changed = abstract_domains_provider_parameter_digest_for_test(
            policy.reduction_rounds,
            policy.budget.widening_fuel,
            policy.budget.max_iterations,
            policy.budget.max_carried_facts - 1,
        );

        assert_ne!(baseline, changed);
        assert_ne!(baseline, carried_changed);
        assert!(baseline.to_string().contains("provider_parameters"));
    }
}
