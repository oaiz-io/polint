mod guard_outcomes;

use std::process::ExitCode;

fn main() -> ExitCode {
    polint::runner::run_cli(vec![guard_outcomes::guard_outcomes()])
}
