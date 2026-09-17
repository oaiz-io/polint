use std::collections::BTreeSet;
use std::path::Path;

use crate::internal_core::StableKeyInterner;
use crate::ts::error::AnalysisError;
use crate::ts::types::store::{TS_TYPES_PROVIDER_ID, TsTypesFactsOutput};

/// Final safety net over a lowered sidecar output.
///
/// The store already drops rows it cannot key or join, so this checks the
/// invariants a drop cannot repair: keys must be unique, ids must be dense and
/// ordered, and no row may name a path outside the repository.
pub(crate) fn validate_ts_types_output(
    output: &TsTypesFactsOutput,
    interner: &StableKeyInterner,
) -> Result<(), AnalysisError> {
    validate_unique(
        "project",
        output.projects.iter().map(|row| row.stable_key),
        interner,
    )?;
    validate_unique(
        "callable",
        output.callables.iter().map(|row| row.stable_key),
        interner,
    )?;
    validate_unique(
        "callsite",
        output.callsites.iter().map(|row| row.stable_key),
        interner,
    )?;
    validate_unique(
        "callee",
        output.callees.iter().map(|row| row.stable_key),
        interner,
    )?;
    validate_unique(
        "receiver",
        output.receivers.iter().map(|row| row.stable_key),
        interner,
    )?;
    validate_unique(
        "any_density",
        output.file_densities.iter().map(|row| row.stable_key),
        interner,
    )?;

    validate_dense("project", output.projects.iter().map(|row| row.id.0))?;
    validate_dense("callable", output.callables.iter().map(|row| row.id.0))?;
    validate_dense("callsite", output.callsites.iter().map(|row| row.id.0))?;
    validate_dense("callee", output.callees.iter().map(|row| row.id.0))?;
    validate_dense("receiver", output.receivers.iter().map(|row| row.id.0))?;
    validate_dense(
        "any_density",
        output.file_densities.iter().map(|row| row.id.0),
    )?;

    for row in &output.callables {
        validate_relative_path(row.relative_file.as_deref())?;
    }
    for row in &output.callsites {
        validate_relative_path(row.relative_file.as_deref())?;
    }
    for row in &output.callees {
        validate_relative_path(row.relative_file.as_deref())?;
    }
    for row in &output.file_densities {
        validate_relative_path(Some(row.relative_file.as_str()))?;
    }

    let callsite_keys = output
        .callsites
        .iter()
        .map(|row| interner.resolve(row.stable_key).to_string())
        .collect::<BTreeSet<_>>();
    for row in &output.callees {
        let site = interner.resolve(row.callsite_stable_key).to_string();
        if !callsite_keys.contains(&site) {
            return Err(invalid_fact(format!(
                "TS type callee names call site `{site}`, which this output does not contain"
            )));
        }
    }
    Ok(())
}

fn validate_unique(
    family: &str,
    keys: impl Iterator<Item = crate::internal_core::StableKeyId>,
    interner: &StableKeyInterner,
) -> Result<(), AnalysisError> {
    let mut seen = BTreeSet::new();
    for key in keys {
        let text = interner.resolve(key).to_string();
        if !seen.insert(text.clone()) {
            return Err(invalid_fact(format!(
                "duplicate TS type {family} stable key `{text}`"
            )));
        }
    }
    Ok(())
}

fn validate_dense(family: &str, ids: impl Iterator<Item = u64>) -> Result<(), AnalysisError> {
    for (index, id) in ids.enumerate() {
        if id != index as u64 {
            return Err(invalid_fact(format!(
                "TS type {family} ids must be dense and ordered; found {id} at position {index}"
            )));
        }
    }
    Ok(())
}

/// Rejects a path that leaves the repository.
///
/// Sidecar paths are repository-relative by contract. An absolute or
/// parent-escaping path is either a bug or an attempt to describe a file the
/// scan does not own, and neither should reach a fact.
pub(crate) fn validate_relative_path(path: Option<&str>) -> Result<(), AnalysisError> {
    let Some(path) = path else {
        return Ok(());
    };
    let candidate = Path::new(path);
    if candidate.is_absolute() || path == ".." || path.starts_with("../") || path.contains("/../") {
        return Err(invalid_fact(format!(
            "TS type sidecar file path `{path}` escapes repository"
        )));
    }
    Ok(())
}

fn invalid_fact(reason: String) -> AnalysisError {
    AnalysisError::InvalidFact {
        provider: TS_TYPES_PROVIDER_ID,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ts::types::facts::{
        TsTypeCallStatus, TsTypeCalleeFact, TsTypeCalleeId, TsTypeCallsiteFact, TsTypeCallsiteId,
        TsTypeDispatch,
    };

    fn callsite(interner: &StableKeyInterner, key: &str, file: &str) -> TsTypeCallsiteFact {
        TsTypeCallsiteFact {
            id: TsTypeCallsiteId(0),
            stable_key: interner.intern(key),
            project: "tsconfig.json".to_string(),
            callsite: key.to_string(),
            enclosing: None,
            call_kind: "call".to_string(),
            status: TsTypeCallStatus::Resolved,
            reason: None,
            relative_file: Some(file.to_string()),
            file: None,
            span: None,
        }
    }

    #[test]
    fn a_valid_output_passes() {
        let interner = StableKeyInterner::default();
        let output = TsTypesFactsOutput {
            callsites: vec![callsite(&interner, "a", "src/app.ts")],
            ..TsTypesFactsOutput::default()
        };

        validate_ts_types_output(&output, &interner).expect("valid output");
    }

    #[test]
    fn duplicate_stable_keys_are_rejected() {
        let interner = StableKeyInterner::default();
        let mut second = callsite(&interner, "a", "src/app.ts");
        second.id = TsTypeCallsiteId(1);
        let output = TsTypesFactsOutput {
            callsites: vec![callsite(&interner, "a", "src/app.ts"), second],
            ..TsTypesFactsOutput::default()
        };

        let error = validate_ts_types_output(&output, &interner).expect_err("duplicate rejected");
        assert!(error.to_string().contains("duplicate TS type callsite"));
    }

    #[test]
    fn non_dense_ids_are_rejected() {
        let interner = StableKeyInterner::default();
        let mut row = callsite(&interner, "a", "src/app.ts");
        row.id = TsTypeCallsiteId(7);
        let output = TsTypesFactsOutput {
            callsites: vec![row],
            ..TsTypesFactsOutput::default()
        };

        let error = validate_ts_types_output(&output, &interner).expect_err("sparse id rejected");
        assert!(error.to_string().contains("dense and ordered"));
    }

    #[test]
    fn a_path_escaping_the_repository_is_rejected() {
        let interner = StableKeyInterner::default();
        let output = TsTypesFactsOutput {
            callsites: vec![callsite(&interner, "a", "../outside/app.ts")],
            ..TsTypesFactsOutput::default()
        };

        let error = validate_ts_types_output(&output, &interner).expect_err("escape rejected");
        assert!(error.to_string().contains("escapes repository"));
    }

    #[test]
    fn an_absolute_path_is_rejected() {
        assert!(validate_relative_path(Some("/etc/passwd")).is_err());
        assert!(validate_relative_path(Some("src/../../etc/passwd")).is_err());
        assert!(validate_relative_path(Some("src/app.ts")).is_ok());
        assert!(validate_relative_path(None).is_ok());
    }

    #[test]
    fn a_callee_naming_an_absent_call_site_is_rejected() {
        let interner = StableKeyInterner::default();
        let output = TsTypesFactsOutput {
            callsites: vec![callsite(&interner, "site", "src/app.ts")],
            callees: vec![TsTypeCalleeFact {
                id: TsTypeCalleeId(0),
                stable_key: interner.intern("callee"),
                project: "tsconfig.json".to_string(),
                callsite_stable_key: interner.intern("missing"),
                callable: None,
                external: Some("lib:lib.es5.d.ts#map".to_string()),
                dispatch: TsTypeDispatch::Declared,
                relative_file: None,
                file: None,
                span: None,
            }],
            ..TsTypesFactsOutput::default()
        };

        let error = validate_ts_types_output(&output, &interner).expect_err("dangling rejected");
        assert!(error.to_string().contains("does not contain"));
    }
}
