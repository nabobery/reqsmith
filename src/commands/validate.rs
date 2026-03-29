use std::path::PathBuf;
use std::process::ExitCode;

use crate::cli::OutputMode;
use crate::core::environment;
use crate::core::models::ExitCode as HurlExitCode;
use crate::core::repository;
use crate::core::validation;
use crate::output;

pub fn execute(files: Vec<PathBuf>, env: Option<String>, output_mode: OutputMode) -> ExitCode {
    let paths = if files.is_empty() {
        resolve_paths(vec![PathBuf::from(".")])
    } else {
        resolve_paths(files)
    };
    if paths.is_empty() {
        eprintln!("No .hurl.yml files found");
        return ExitCode::from(1);
    }

    let mut any_errors = false;
    let mut any_internal_errors = false;

    for path in &paths {
        let env_set = match resolve_environment_for_path(path, env.as_deref()) {
            Ok(env_set) => env_set,
            Err(e) => {
                eprintln!("{}: environment error: {e}", path.display());
                any_internal_errors = true;
                continue;
            }
        };

        let doc = match repository::load_request(path) {
            Ok(doc) => doc,
            Err(e) => {
                eprintln!("{}: failed to load: {e}", path.display());
                any_errors = true;
                continue;
            }
        };

        let report = validation::validate_document(&doc, env_set.as_ref());
        output::print_validation_report(&report, path, &output_mode);

        if report.has_errors() {
            any_errors = true;
        }
    }

    if any_internal_errors {
        ExitCode::from(HurlExitCode::InternalError as u8)
    } else if any_errors {
        ExitCode::from(HurlExitCode::ValidationFailure as u8)
    } else {
        ExitCode::SUCCESS
    }
}

fn resolve_paths(files: Vec<PathBuf>) -> Vec<PathBuf> {
    if files.is_empty() {
        return vec![PathBuf::from(".")];
    }

    let mut paths = Vec::new();
    for file in files {
        if file.is_dir() {
            if let Ok(nodes) = repository::discover_requests(&file) {
                collect_file_paths(&nodes, &mut paths);
            }
        } else {
            paths.push(file);
        }
    }
    paths
}

fn resolve_environment_for_path(
    path: &std::path::Path,
    env_name: Option<&str>,
) -> Result<Option<crate::core::models::EnvironmentSet>, environment::EnvironmentError> {
    let base_dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    environment::resolve_environment(base_dir, env_name, &[]).map(Some)
}

fn collect_file_paths(nodes: &[crate::core::models::CollectionNode], out: &mut Vec<PathBuf>) {
    for node in nodes {
        match node.kind {
            crate::core::models::CollectionNodeKind::RequestFile => {
                out.push(node.path.clone());
            }
            crate::core::models::CollectionNodeKind::Directory => {
                collect_file_paths(&node.children, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::resolve_paths;

    #[test]
    fn resolve_paths_defaults_to_current_directory_when_empty() {
        let paths = resolve_paths(vec![]);
        assert_eq!(paths, vec![PathBuf::from(".")]);
    }
}
