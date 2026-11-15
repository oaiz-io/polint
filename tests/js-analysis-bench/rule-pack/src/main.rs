mod fact_census;

use std::process::ExitCode;

fn main() -> ExitCode {
    polint::runner::run_cli(vec![fact_census::fact_census()])
}
