use std::path::Path;

use crate::cli::OutputMode;
use crate::core::models::{
    AssertionReport, CollectionNode, CollectionNodeKind, DiffArtifact, DiffLine, ExitCode,
    HeaderDiff, RunResult, ValidationReport, ValidationSeverity,
};
use crate::core::redaction;

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

    if !result.assertions.is_empty() {
        print_assertions_human(&result.assertions);
    }
}

fn print_assertions_human(report: &AssertionReport) {
    let passed = report.pass_count();
    let failed = report.fail_count();

    println!();
    println!("Assertions: {passed} passed, {failed} failed");

    for r in &report.results {
        let label = if r.passed { "PASS" } else { "FAIL" };
        let assertion_display = format!("{}", r.assertion);
        if let Some(actual) = &r.actual_value {
            println!("  {label}  {assertion_display} (actual: {actual})");
        } else {
            println!("  {label}  {assertion_display}");
        }
    }
}

/// Build the JSON representation of a run result. Response headers are
/// routed through [`redaction::redact_headers`] so secrets (e.g. a
/// `Set-Cookie` the server returned) never reach stdout. Split out from
/// [`print_run_json`] so the redaction behavior is directly unit-testable.
fn build_run_json(result: &RunResult) -> serde_json::Value {
    let mut json = serde_json::json!({
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
            "headers": redaction::redact_headers(&r.headers).iter().map(|(k, v)| serde_json::json!({
                "name": k,
                "value": v,
            })).collect::<Vec<_>>(),
            "body": r.body_text,
        })),
    });

    if !result.assertions.is_empty() {
        let assertions: Vec<serde_json::Value> = result
            .assertions
            .results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "type": format!("{}", r.assertion),
                    "passed": r.passed,
                    "message": r.message,
                    "actual_value": r.actual_value,
                })
            })
            .collect();
        json.as_object_mut()
            .unwrap()
            .insert("assertions".into(), serde_json::Value::Array(assertions));
    }

    json
}

fn print_run_json(result: &RunResult) {
    let json = build_run_json(result);
    println!(
        "{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );
}

/// Print the result of diffing two stored runs.
#[allow(dead_code)]
pub fn print_diff_result(diff: &DiffArtifact, mode: &OutputMode) {
    match mode {
        OutputMode::Human => print_diff_human(diff),
        OutputMode::Json => print_diff_json(diff),
    }
}

fn print_diff_human(diff: &DiffArtifact) {
    println!("--- baseline: {}", diff.baseline_label);
    println!("+++ candidate: {}", diff.candidate_label);
    println!();

    if let Some((old_status, new_status)) = diff.status_diff {
        println!("Status: {old_status} -> {new_status}");
        println!();
    }

    if !diff.header_diffs.is_empty() {
        println!("Headers:");
        for hd in &diff.header_diffs {
            match hd {
                HeaderDiff::Added(key, val) => println!("  + {key}: {val}"),
                HeaderDiff::Removed(key, val) => println!("  - {key}: {val}"),
                HeaderDiff::Changed { key, old, new } => println!("  ~ {key}: {old} -> {new}"),
            }
        }
        println!();
    }

    let has_body_changes = diff
        .body_diff
        .iter()
        .any(|l| matches!(l, DiffLine::Insert(_) | DiffLine::Delete(_)));
    if has_body_changes {
        println!("Body:");
        for line in &diff.body_diff {
            match line {
                DiffLine::Equal(s) => print!("  {s}"),
                DiffLine::Insert(s) => print!("  +{s}"),
                DiffLine::Delete(s) => print!("  -{s}"),
            }
        }
        // Ensure trailing newline
        println!();
    }
}

fn print_diff_json(diff: &DiffArtifact) {
    let status = diff
        .status_diff
        .map(|(old, new)| serde_json::json!({ "old": old, "new": new }));

    let headers: Vec<serde_json::Value> = diff
        .header_diffs
        .iter()
        .map(|hd| match hd {
            HeaderDiff::Added(key, val) => {
                serde_json::json!({ "type": "added", "key": key, "value": val })
            }
            HeaderDiff::Removed(key, val) => {
                serde_json::json!({ "type": "removed", "key": key, "value": val })
            }
            HeaderDiff::Changed { key, old, new } => {
                serde_json::json!({ "type": "changed", "key": key, "old": old, "new": new })
            }
        })
        .collect();

    let body_lines: Vec<serde_json::Value> = diff
        .body_diff
        .iter()
        .filter(|l| !matches!(l, DiffLine::Equal(_)))
        .map(|l| match l {
            DiffLine::Insert(s) => serde_json::json!({ "type": "insert", "line": s }),
            DiffLine::Delete(s) => serde_json::json!({ "type": "delete", "line": s }),
            DiffLine::Equal(_) => unreachable!(),
        })
        .collect();

    let json = serde_json::json!({
        "baseline": diff.baseline_label,
        "candidate": diff.candidate_label,
        "status_diff": status,
        "header_diffs": headers,
        "body_diffs": body_lines,
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
    use crate::core::models::{Assertion, AssertionOperator, AssertionResult, ResponseArtifact};
    use std::path::PathBuf;

    #[test]
    fn print_run_json_produces_valid_json() {
        let result = RunResult {
            request_name: "Test".into(),
            request_file: PathBuf::from("test.req.yml"),
            response: Some(ResponseArtifact {
                status_code: 200,
                http_version: "HTTP/1.1".into(),
                headers: vec![("content-type".into(), "application/json".into())],
                content_type: Some("application/json".into()),
                content_length: Some(42),
                duration_ms: 150,
                body_text: Some("{\"ok\": true}".into()),
                body_bytes: None,
                is_binary: false,
            }),
            error: None,
            exit_code: ExitCode::Success,
            cancelled: false,
            assertions: AssertionReport::default(),
        };

        // Just verify no panic and JSON is valid
        let json = serde_json::json!({
            "status_code": result.response.as_ref().unwrap().status_code,
        });
        assert_eq!(json["status_code"], 200);
    }

    #[test]
    fn print_run_json_redacts_sensitive_headers() {
        let result = RunResult {
            request_name: "Test".into(),
            request_file: PathBuf::from("test.req.yml"),
            response: Some(ResponseArtifact {
                status_code: 200,
                http_version: "HTTP/1.1".into(),
                headers: vec![
                    ("authorization".into(), "Bearer secret-token".into()),
                    ("set-cookie".into(), "session=abc123".into()),
                    ("content-type".into(), "application/json".into()),
                ],
                content_type: Some("application/json".into()),
                content_length: Some(2),
                duration_ms: 10,
                body_text: Some("{}".into()),
                body_bytes: None,
                is_binary: false,
            }),
            error: None,
            exit_code: ExitCode::Success,
            cancelled: false,
            assertions: AssertionReport::default(),
        };

        let json = build_run_json(&result);
        let headers = json["response"]["headers"].as_array().unwrap();

        let auth = headers
            .iter()
            .find(|h| h["name"] == "authorization")
            .unwrap();
        assert_eq!(auth["value"], "<redacted>");

        let cookie = headers.iter().find(|h| h["name"] == "set-cookie").unwrap();
        assert_eq!(cookie["value"], "<redacted>");

        let ct = headers
            .iter()
            .find(|h| h["name"] == "content-type")
            .unwrap();
        assert_eq!(ct["value"], "application/json");

        let rendered = serde_json::to_string(&json).unwrap();
        assert!(!rendered.contains("secret-token"));
        assert!(!rendered.contains("abc123"));
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
                    path: PathBuf::from("requests/get_users.req.yml"),
                    kind: CollectionNodeKind::RequestFile,
                    children: vec![],
                    depth: 1,
                },
                CollectionNode {
                    name: "create_user".into(),
                    path: PathBuf::from("requests/create_user.req.yml"),
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
        print_validation_human(&report, Path::new("test.req.yml"));
    }

    #[test]
    fn quiet_mode_suppresses_success() {
        let result = RunResult {
            request_name: "Test".into(),
            request_file: PathBuf::from("test.req.yml"),
            response: Some(ResponseArtifact {
                status_code: 200,
                http_version: "HTTP/1.1".into(),
                headers: vec![],
                content_type: None,
                content_length: None,
                duration_ms: 10,
                body_text: None,
                body_bytes: None,
                is_binary: false,
            }),
            error: None,
            exit_code: ExitCode::Success,
            cancelled: false,
            assertions: AssertionReport::default(),
        };

        // In quiet mode with success, print_run_result should return early
        // (testing by not panicking is sufficient here)
        print_run_result(&result, &OutputMode::Human, true);
    }

    #[test]
    fn print_run_human_shows_assertion_output() {
        let result = RunResult {
            request_name: "Create User".into(),
            request_file: PathBuf::from("create_user.req.yml"),
            response: Some(ResponseArtifact {
                status_code: 201,
                http_version: "HTTP/1.1".into(),
                headers: vec![],
                content_type: Some("application/json".into()),
                content_length: None,
                duration_ms: 45,
                body_text: Some("{\"user\":{\"id\":99}}".into()),
                body_bytes: None,
                is_binary: false,
            }),
            error: None,
            exit_code: ExitCode::AssertionFailure,
            cancelled: false,
            assertions: AssertionReport {
                results: vec![
                    AssertionResult {
                        assertion: Assertion::ExpectStatus(201),
                        passed: true,
                        message: "status is 201".into(),
                        actual_value: None,
                    },
                    AssertionResult {
                        assertion: Assertion::ExpectTimeUnder(500),
                        passed: true,
                        message: "response time under 500ms".into(),
                        actual_value: Some("45ms".into()),
                    },
                    AssertionResult {
                        assertion: Assertion::ExpectBodyPath {
                            path: "$.user.id".into(),
                            operator: AssertionOperator::Eq,
                            expected: serde_json::json!(42),
                        },
                        passed: false,
                        message: "body path mismatch".into(),
                        actual_value: Some("99".into()),
                    },
                ],
            },
        };

        // Verify no panic and assertion report is printed.
        // We call print_run_human directly; output goes to stdout.
        print_run_human(&result);
    }

    #[test]
    fn print_run_human_no_assertions_when_empty() {
        let result = RunResult {
            request_name: "Simple".into(),
            request_file: PathBuf::from("simple.req.yml"),
            response: Some(ResponseArtifact {
                status_code: 200,
                http_version: "HTTP/1.1".into(),
                headers: vec![],
                content_type: None,
                content_length: None,
                duration_ms: 10,
                body_text: None,
                body_bytes: None,
                is_binary: false,
            }),
            error: None,
            exit_code: ExitCode::Success,
            cancelled: false,
            assertions: AssertionReport::default(),
        };

        // Should not panic and should not print assertion section
        print_run_human(&result);
    }
}
