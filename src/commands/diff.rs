use std::path::PathBuf;
use std::process::ExitCode;

use crate::cli::OutputMode;
use crate::core::diffing;
use crate::core::storage;
use crate::output;

pub async fn execute(baseline: PathBuf, candidate: PathBuf, output_mode: OutputMode) -> ExitCode {
    let base_run = match storage::load_run(&baseline) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to load baseline: {e}");
            return ExitCode::from(1);
        }
    };
    let cand_run = match storage::load_run(&candidate) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to load candidate: {e}");
            return ExitCode::from(1);
        }
    };

    let base_artifact = base_run.to_response_artifact();
    let cand_artifact = cand_run.to_response_artifact();
    let baseline_label = baseline.display().to_string();
    let candidate_label = candidate.display().to_string();

    let diff = diffing::diff_responses(
        &base_artifact,
        &cand_artifact,
        &baseline_label,
        &candidate_label,
    );
    output::print_diff_result(&diff, &output_mode);

    ExitCode::SUCCESS
}
