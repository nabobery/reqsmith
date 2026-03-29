use std::path::PathBuf;
use std::process::ExitCode;

use crate::core::formatter;
use crate::core::repository;

pub fn execute(files: Vec<PathBuf>, check: bool) -> ExitCode {
    let paths = if files.is_empty() {
        resolve_paths(vec![PathBuf::from(".")])
    } else {
        resolve_paths(files)
    };
    if paths.is_empty() {
        eprintln!("No .hurl.yml files found");
        return ExitCode::from(1);
    }

    let mut any_unformatted = false;
    let mut any_error = false;

    for path in &paths {
        if check {
            match formatter::is_formatted(path) {
                Ok(true) => {}
                Ok(false) => {
                    println!("{}: needs formatting", path.display());
                    any_unformatted = true;
                }
                Err(e) => {
                    eprintln!("{}: {e}", path.display());
                    any_error = true;
                }
            }
        } else {
            match formatter::format_file(path) {
                Ok(true) => {
                    println!("Formatted: {}", path.display());
                }
                Ok(false) => {
                    // Already formatted, no output needed.
                }
                Err(e) => {
                    eprintln!("{}: {e}", path.display());
                    any_error = true;
                }
            }
        }
    }

    if any_error || (check && any_unformatted) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// If a path is "." or a directory, discover all .hurl.yml files in it.
/// Otherwise, use the path as-is.
fn resolve_paths(files: Vec<PathBuf>) -> Vec<PathBuf> {
    if files.is_empty() {
        return vec![PathBuf::from(".")];
    }

    let mut paths = Vec::new();
    for file in files {
        if file.is_dir() {
            match repository::discover_requests(&file) {
                Ok(nodes) => {
                    collect_file_paths(&nodes, &mut paths);
                }
                Err(e) => {
                    eprintln!("Discovery error in {}: {e}", file.display());
                }
            }
        } else {
            paths.push(file);
        }
    }
    paths
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
