use std::path::PathBuf;
use std::process::ExitCode;

use crate::cli::OutputMode;
use crate::core::repository;
use crate::output;

pub fn execute(path: PathBuf, output_mode: OutputMode) -> ExitCode {
    match repository::discover_requests(&path) {
        Ok(nodes) => {
            if nodes.is_empty() {
                eprintln!("No .req.yml files found in {}", path.display());
                return ExitCode::from(1);
            }
            output::print_list(&nodes, &output_mode);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Discovery error: {e}",);
            ExitCode::from(1)
        }
    }
}
