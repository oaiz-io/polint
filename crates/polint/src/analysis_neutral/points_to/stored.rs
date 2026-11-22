//! A solver input written to a file: the projected constraints, calls and
//! signatures of one analysed tree, with stable keys as text so that another
//! process can intern them again and solve the same input.
//!
//! The benchmark dump writes one per package; an ignored release test solves
//! them, which times the solve apart from the providers that build its input.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::facts::{
    PointsToConstraintFact, PointsToConstraintKind, PointsToPrecision, PointsToStatus,
};
use super::solver::{PointsToCall, PointsToCallInputs, PointsToSignature, SignaturePlace};
use crate::analysis_neutral::ids::{ObjectTokenId, PointsToConstraintId, PtVarId};
use crate::internal_core::StableKeyInterner;

#[derive(Serialize, Deserialize)]
struct StoredInput {
    constraints: Vec<StoredConstraint>,
    calls: Vec<StoredCall>,
    signatures: Vec<StoredSignature>,
}

#[derive(Serialize, Deserialize)]
struct StoredConstraint {
    key: String,
    kind: PointsToConstraintKind,
    status: PointsToStatus,
    precision: PointsToPrecision,
}

#[derive(Serialize, Deserialize)]
struct StoredCall {
    key: String,
    callee: PtVarId,
    receiver: Option<PtVarId>,
    arguments: Vec<PtVarId>,
    spread_from: Option<usize>,
    result: Option<PtVarId>,
    precision: PointsToPrecision,
}

#[derive(Serialize, Deserialize)]
struct StoredPlace {
    var: PtVarId,
    key: String,
}

#[derive(Serialize, Deserialize)]
struct StoredSignature {
    object: ObjectTokenId,
    params: Vec<(usize, StoredPlace)>,
    rest: Option<StoredPlace>,
    this: Option<StoredPlace>,
    ret: Option<StoredPlace>,
}

pub(crate) fn write(
    path: &Path,
    interner: &StableKeyInterner,
    constraints: &[PointsToConstraintFact],
    calls: &PointsToCallInputs,
) -> anyhow::Result<()> {
    let place = |place: &SignaturePlace| StoredPlace {
        var: place.var,
        key: interner.resolve(place.stable_key).to_string(),
    };
    let input = StoredInput {
        constraints: constraints
            .iter()
            .map(|constraint| StoredConstraint {
                key: interner.resolve(constraint.stable_key).to_string(),
                kind: constraint.kind.clone(),
                status: constraint.status,
                precision: constraint.precision,
            })
            .collect(),
        calls: calls
            .calls
            .iter()
            .map(|call| StoredCall {
                key: interner.resolve(call.stable_key).to_string(),
                callee: call.callee,
                receiver: call.receiver,
                arguments: call.arguments.clone(),
                spread_from: call.spread_from,
                result: call.result,
                precision: call.precision,
            })
            .collect(),
        signatures: calls
            .signatures
            .iter()
            .map(|(object, signature)| StoredSignature {
                object: *object,
                params: signature
                    .params
                    .iter()
                    .map(|(position, value)| (*position, place(value)))
                    .collect(),
                rest: signature.rest.as_ref().map(place),
                this: signature.this.as_ref().map(place),
                ret: signature.ret.as_ref().map(place),
            })
            .collect(),
    };
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    serde_json::to_writer(file, &input)?;
    Ok(())
}

/// The largest stored input [`read`] loads.
const STORED_INPUT_MAX_BYTES: u64 = 1024 * 1_048_576;

pub(crate) fn read(
    path: &Path,
    interner: &StableKeyInterner,
) -> anyhow::Result<(Vec<PointsToConstraintFact>, PointsToCallInputs)> {
    let bytes = crate::repo_fs::read_file_with_limit(path, STORED_INPUT_MAX_BYTES)?;
    let input: StoredInput = crate::bounded_json::from_slice(&bytes, STORED_INPUT_MAX_BYTES)?;
    let place = |place: StoredPlace| SignaturePlace {
        var: place.var,
        stable_key: interner.intern(place.key),
    };
    let constraints = input
        .constraints
        .into_iter()
        .map(|constraint| PointsToConstraintFact {
            id: PointsToConstraintId(0),
            kind: constraint.kind,
            status: constraint.status,
            precision: constraint.precision,
            stable_key: interner.intern(constraint.key),
        })
        .collect();
    let calls = input
        .calls
        .into_iter()
        .map(|call| PointsToCall {
            stable_key: interner.intern(call.key),
            callee: call.callee,
            receiver: call.receiver,
            arguments: call.arguments,
            spread_from: call.spread_from,
            result: call.result,
            precision: call.precision,
        })
        .collect();
    let signatures = input
        .signatures
        .into_iter()
        .map(|signature| {
            (
                signature.object,
                PointsToSignature {
                    params: signature
                        .params
                        .into_iter()
                        .map(|(position, value)| (position, place(value)))
                        .collect::<BTreeMap<_, _>>(),
                    rest: signature.rest.map(place),
                    this: signature.this.map(place),
                    ret: signature.ret.map(place),
                },
            )
        })
        .collect();
    Ok((constraints, PointsToCallInputs { calls, signatures }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::points_to::solver::{
        PointsToBudget, solve_points_to_logged, solve_points_to_with_calls,
    };

    const INPUTS_ENV: &str = "POLINT_POINTS_TO_INPUTS";
    const MAX_GROWTH_ENV: &str = "POLINT_POINTS_TO_MAX_GROWTH_MB";

    /// Solves each stored input named in `POLINT_POINTS_TO_INPUTS`
    /// (comma-separated paths the benchmark dump wrote) with the default
    /// budget and logs the solve. With `POLINT_POINTS_TO_MAX_GROWTH_MB` set,
    /// each solve must raise the process peak at most that far above the
    /// resident size it started from. Run it in release:
    ///
    /// ```sh
    /// POLINT_POINTS_TO_INPUTS=zod.solver.json,axios.solver.json \
    /// RUST_LOG=polint::points_to=info cargo test -p polint --lib --release --locked \
    ///   stored::tests::solve_stored_inputs -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "solver benchmark: run explicitly with POLINT_POINTS_TO_INPUTS set"]
    fn solve_stored_inputs() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .try_init();
        let paths = std::env::var(INPUTS_ENV)
            .unwrap_or_else(|_| panic!("{INPUTS_ENV} lists the stored inputs; it is required"));
        let max_growth_mb = std::env::var(MAX_GROWTH_ENV).ok().map(|bound| {
            bound
                .parse::<u64>()
                .unwrap_or_else(|_| panic!("{MAX_GROWTH_ENV} is a whole number of MB"))
        });
        for path in paths.split(',').filter(|path| !path.is_empty()) {
            let interner = crate::internal_core::test_stable_key_interner();
            let (constraints, calls) =
                read(Path::new(path), &interner).unwrap_or_else(|error| panic!("{path}: {error}"));
            let rss_before = crate::measure::current_rss_bytes();
            let started = std::time::Instant::now();
            let result = solve_points_to_logged(
                "stored",
                &interner,
                &constraints,
                &calls,
                PointsToBudget::default(),
                None,
            );
            eprintln!(
                "{path}: {} ms, {} sets, {:?}",
                started.elapsed().as_millis(),
                result.sets.len(),
                result.budget_status
            );
            let growth_mb =
                crate::measure::peak_rss_bytes().saturating_sub(rss_before) / (1024 * 1024);
            if let Some(bound) = max_growth_mb {
                assert!(
                    growth_mb <= bound,
                    "{path}: the solve raised the peak {growth_mb} MB, above {bound} MB"
                );
            }
        }
    }

    #[test]
    fn a_written_input_reads_back_to_the_same_solve() {
        let interner = crate::internal_core::test_stable_key_interner();
        let constraints = vec![
            PointsToConstraintFact {
                id: PointsToConstraintId(0),
                kind: PointsToConstraintKind::AddressOf {
                    dst: PtVarId(1),
                    object: ObjectTokenId(7),
                },
                status: PointsToStatus::Present,
                precision: PointsToPrecision::FlowInsensitive,
                stable_key: interner.intern("addr"),
            },
            PointsToConstraintFact {
                id: PointsToConstraintId(0),
                kind: PointsToConstraintKind::FieldStore {
                    base: PtVarId(2),
                    field: "f".to_string(),
                    src: PtVarId(1),
                },
                status: PointsToStatus::Present,
                precision: PointsToPrecision::FlowInsensitive,
                stable_key: interner.intern("store"),
            },
        ];
        let calls = PointsToCallInputs {
            calls: vec![PointsToCall {
                stable_key: interner.intern("call"),
                callee: PtVarId(1),
                receiver: None,
                arguments: vec![PtVarId(2)],
                spread_from: None,
                result: Some(PtVarId(3)),
                precision: PointsToPrecision::FlowInsensitive,
            }],
            signatures: BTreeMap::from([(
                ObjectTokenId(7),
                PointsToSignature {
                    params: BTreeMap::from([(
                        0,
                        SignaturePlace {
                            var: PtVarId(4),
                            stable_key: interner.intern("param"),
                        },
                    )]),
                    ..PointsToSignature::default()
                },
            )]),
        };
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("input.json");
        write(&path, &interner, &constraints, &calls).expect("write");

        let reread = crate::internal_core::test_stable_key_interner();
        let (read_constraints, read_calls) = read(&path, &reread).expect("read");
        let budget = PointsToBudget::default();
        let original = solve_points_to_with_calls(&interner, &constraints, &calls, budget);
        let again = solve_points_to_with_calls(&reread, &read_constraints, &read_calls, budget);
        let objects = |result: &crate::analysis_neutral::points_to::solver::PointsToSolveResult| {
            result
                .sets
                .iter()
                .map(|set| (set.variable, set.objects.clone(), set.precision))
                .collect::<Vec<_>>()
        };
        assert_eq!(objects(&original), objects(&again));
        assert_eq!(original.stats, again.stats);
    }
}
