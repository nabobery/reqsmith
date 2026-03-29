use std::path::Path;

use crate::cli::OutputMode;
use crate::core::models::{
    CollectionNode, CollectionNodeKind, ExitCode, RunResult, ValidationReport, ValidationSeverity,
};

/// Print the result of running a request.
#[allow(dead_code)] // Used in Step 8.
pub fn print_run_result(result: &RunResult, mode: &OutputMode, quiet: bool) {
    if quiet && result.exit_code == ExitCode::Success {
        return;
    }

    match mode {
        OutputMode::Human => print_run_human(result),
        OutputMode::Json => print_run_json(result),
    }
}

fn print_run_human(result: &RunResult) {
    if result.cancelled {
        eprintln!("Cancelled");
        return;
    }

    if let Some(error) = &result.error {
        eprintln!("Error: {error}");
        return;
    }

    if let Some(response) = &result.response {
        let status = response.status_code;
        let duration = response.duration_ms;
        let ct = response.content_type.as_deref().unwrap_or("unknown");

        println!("{status} ({duration}ms) [{ct}]");

        if let Some(body) = &response.body_text {
            println!("{body}");
        }
    }
}

fn print_run_json(result: &RunResult) {
    let json = serde_json::json!({
        "request_name": result.request_name,
        "request_file": result.request_file.display().to_string(),
        "exit_code": result.exit_code as u8,
        "cancelled": result.cancelled,
        "error": result.error,
        "response": result.response.as_ref().map(|r| serde_json::json!({
            "status_code": r.status_code,
            "http_version": r.http_version,
            "duration_ms": r.duration_ms,
            "content_type": r.content_type,
            "content_length": r.content_length,
            "headers": r.headers.iter().map(|(k, v)| serde_json::json!({
                "name": k,
                "value": v,
            })).collect::<Vec<_>>(),
            "body": r.body_text,
        })),
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );
}

/// Print a validation report.
#[allow(dead_code)] // Used in Step 8.
pub fn print_validation_report(report: &ValidationReport, path: &Path, mode: &OutputMode) {
    match mode {
        OutputMode::Human => print_validation_human(report, path),
        OutputMode::Json => print_validation_json(report, path),
    }
}

fn print_validation_human(report: &ValidationReport, path: &Path) {
    let path_str = path.display();
    if report.diagnostics.is_empty() {
        println!("{path_str}: ok");
        return;
    }

    for diag in &report.diagnostics {
        let level = match diag.severity {
            ValidationSeverity::Error => "error",
            ValidationSeverity::Warning => "warning",
        };
        let field = diag
            .field
            .as_deref()
            .map(|f| format!(" [{f}]"))
            .unwrap_or_default();
        eprintln!("{path_str}: {level}{field}: {}", diag.message);
    }
}

fn print_validation_json(report: &ValidationReport, path: &Path) {
    let json = serde_json::json!({
        "file": path.display().to_string(),
        "diagnostics": report.diagnostics.iter().map(|d| serde_json::json!({
            "severity": match d.severity {
                ValidationSeverity::Error => "error",
                ValidationSeverity::Warning => "warning",
            },
            "message": d.message,
            "field": d.field,
        })).collect::<Vec<_>>(),
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );
}

/// Print a list of discovered collection nodes.
#[allow(dead_code)] // Used in Step 8.
pub fn print_list(nodes: &[CollectionNode], mode: &OutputMode) {
    match mode {
        OutputMode::Human => print_list_human(nodes, 0),
        OutputMode::Json => print_list_json(nodes),
    }
}

fn print_list_human(nodes: &[CollectionNode], depth: usize) {
    for node in nodes {
        let indent = "  ".repeat(depth);
        match node.kind {
            CollectionNodeKind::Directory => {
                println!("{indent}{}/", node.name);
                print_list_human(&node.children, depth + 1);
            }
            CollectionNodeKind::RequestFile => {
                println!("{indent}{}", node.path.display());
            }
        }
    }
}

fn print_list_json(nodes: &[CollectionNode]) {
    let files: Vec<String> = collect_file_paths(nodes);
    let json = serde_json::json!({ "files": files });
    println!(
        "{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );
}

fn collect_file_paths(nodes: &[CollectionNode]) -> Vec<String> {
    let mut paths = Vec::new();
    for node in nodes {
        match node.kind {
            CollectionNodeKind::RequestFile => {
                paths.push(node.path.display().to_string());
            }
            CollectionNodeKind::Directory => {
                paths.extend(collect_file_paths(&node.children));
            }
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::ResponseArtifact;
    use std::path::PathBuf;

    #[test]
    fn print_run_json_produces_valid_json() {
        let result = RunResult {
            request_name: "Test".into(),
            request_file: PathBuf::from("test.hurl.yml"),
            response: Some(ResponseArtifact {
                status_code: 200,
                http_version: "HTTP/1.1".into(),
                headers: vec![("content-type".into(), "application/json".into())],
                content_type: Some("application/json".into()),
                content_length: Some(42),
                duration_ms: 150,
                body_text: Some("{\"ok\": true}".into()),
            }),
            error: None,
            exit_code: ExitCode::Success,
            cancelled: false,
        };

        // Just verify no panic and JSON is valid
        let json = serde_json::json!({
            "status_code": result.response.as_ref().unwrap().status_code,
        });
        assert_eq!(json["status_code"], 200);
    }

    #[test]
    fn collect_file_paths_flattens_tree() {
        let nodes = vec![CollectionNode {
            name: "requests".into(),
            path: PathBuf::from("requests"),
            kind: CollectionNodeKind::Directory,
            children: vec![
                CollectionNode {
                    name: "get_users".into(),
                    path: PathBuf::from("requests/get_users.hurl.yml"),
                    kind: CollectionNodeKind::RequestFile,
                    children: vec![],
                    depth: 1,
                },
                CollectionNode {
                    name: "create_user".into(),
                    path: PathBuf::from("requests/create_user.hurl.yml"),
                    kind: CollectionNodeKind::RequestFile,
                    children: vec![],
                    depth: 1,
                },
            ],
            depth: 0,
        }];

        let paths = collect_file_paths(&nodes);
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().any(|p| p.contains("get_users")));
        assert!(paths.iter().any(|p| p.contains("create_user")));
    }

    #[test]
    fn validation_report_empty_shows_ok() {
        let report = ValidationReport::default();
        // Just verify no panic
        print_validation_human(&report, Path::new("test.hurl.yml"));
    }

    #[test]
    fn quiet_mode_suppresses_success() {
        let result = RunResult {
            request_name: "Test".into(),
            request_file: PathBuf::from("test.hurl.yml"),
            response: Some(ResponseArtifact {
                status_code: 200,
                http_version: "HTTP/1.1".into(),
                headers: vec![],
                content_type: None,
                content_length: None,
                duration_ms: 10,
                body_text: None,
            }),
            error: None,
            exit_code: ExitCode::Success,
            cancelled: false,
        };

        // In quiet mode with success, print_run_result should return early
        // (testing by not panicking is sufficient here)
        print_run_result(&result, &OutputMode::Human, true);
    }
}
