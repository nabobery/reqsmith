use std::path::Path;

use crate::cli::OutputMode;
use crate::core::execution;
use crate::core::models::{
    AssertionReport, CollectionNode, CollectionNodeKind, DiffArtifact, DiffLine, ExitCode,
    HeaderDiff, RunResult, ValidationReport, ValidationSeverity,
};
use crate::core::redaction;

/// Print the result of running a request.
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

    print!("{}", format_run_human(result));

    if let Some(response) = &result.response
        && response.truncated
    {
        eprintln!(
            "Note: response body truncated at {}",
            execution::max_body_size_label()
        );
    }
}

/// Render the status line, body (or binary placeholder), and any assertion
/// results as one stdout-bound string. Split out from `print_run_human` so
/// tests can assert on the text directly instead of only on side effects.
fn format_run_human(result: &RunResult) -> String {
    let mut out = String::new();

    if let Some(response) = &result.response {
        let status = response.status_code;
        let duration = response.duration_ms;
        let ct = response.content_type.as_deref().unwrap_or("unknown");

        out.push_str(&format!("{status} ({duration}ms) [{ct}]\n"));

        if let Some(body) = &response.body_text {
            out.push_str(body);
            out.push('\n');
        } else if response.is_binary {
            let len = response.body_bytes.as_ref().map_or(0, Vec::len);
            out.push_str(&format!("[binary response body: {len} bytes]\n"));
        }
    }

    if !result.assertions.is_empty() {
        out.push_str(&format_assertions_human(&result.assertions));
    }

    out
}

fn format_assertions_human(report: &AssertionReport) -> String {
    let passed = report.pass_count();
    let failed = report.fail_count();

    let mut out = format!("\nAssertions: {passed} passed, {failed} failed\n");

    for r in &report.results {
        let label = if r.passed { "PASS" } else { "FAIL" };
        let assertion_display = format!("{}", r.assertion);
        if let Some(actual) = &r.actual_value {
            out.push_str(&format!(
                "  {label}  {assertion_display} (actual: {actual})\n"
            ));
        } else {
            out.push_str(&format!("  {label}  {assertion_display}\n"));
        }
    }

    out
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
            "truncated": r.truncated,
            "is_binary": r.is_binary,
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
            let (key, value) = hd.display_parts();
            println!("  {} {key}: {value}", hd.sigil());
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
            print!(" {}{}", line.sigil(), line.text());
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
        .filter_map(|l| {
            let kind = match l {
                DiffLine::Equal(_) => return None,
                DiffLine::Insert(_) => "insert",
                DiffLine::Delete(_) => "delete",
            };
            Some(serde_json::json!({ "type": kind, "line": l.text() }))
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
                truncated: false,
            }),
            error: None,
            exit_code: ExitCode::Success,
            cancelled: false,
            assertions: AssertionReport::default(),
        };

        let json = build_run_json(&result);
        assert_eq!(json["response"]["status_code"], 200);
        assert_eq!(json["exit_code"], ExitCode::Success as u8);
        assert_eq!(json["request_name"], "Test");
    }

    #[test]
    fn run_json_reports_body_truncation() {
        let mut result = RunResult {
            request_name: "Test".into(),
            request_file: PathBuf::from("test.req.yml"),
            response: Some(ResponseArtifact {
                status_code: 200,
                http_version: "HTTP/1.1".into(),
                headers: vec![],
                content_type: Some("text/plain".into()),
                content_length: None,
                duration_ms: 10,
                body_text: Some("aaa".into()),
                body_bytes: None,
                is_binary: false,
                truncated: false,
            }),
            error: None,
            exit_code: ExitCode::Success,
            cancelled: false,
            assertions: AssertionReport::default(),
        };

        assert_eq!(build_run_json(&result)["response"]["truncated"], false);

        if let Some(response) = result.response.as_mut() {
            response.truncated = true;
        }
        let json = build_run_json(&result);
        assert_eq!(json["response"]["truncated"], true);
        // The flag is the report; the body is left as the server sent it.
        assert_eq!(json["response"]["body"], "aaa");
    }

    #[test]
    fn run_json_reports_is_binary_so_consumers_can_tell_binary_from_empty() {
        let mut result = RunResult {
            request_name: "Test".into(),
            request_file: PathBuf::from("test.req.yml"),
            response: Some(ResponseArtifact {
                status_code: 200,
                http_version: "HTTP/1.1".into(),
                headers: vec![],
                content_type: Some("application/octet-stream".into()),
                content_length: None,
                duration_ms: 10,
                body_text: None,
                body_bytes: Some(vec![0, 1, 2]),
                is_binary: true,
                truncated: false,
            }),
            error: None,
            exit_code: ExitCode::Success,
            cancelled: false,
            assertions: AssertionReport::default(),
        };
        assert_eq!(build_run_json(&result)["response"]["is_binary"], true);

        // A non-binary, empty body must be distinguishable from a binary one.
        if let Some(response) = result.response.as_mut() {
            response.is_binary = false;
            response.body_bytes = None;
            response.body_text = Some(String::new());
        }
        assert_eq!(build_run_json(&result)["response"]["is_binary"], false);
    }

    #[test]
    fn diff_sigils_are_shared_by_every_renderer() {
        assert_eq!(DiffLine::Insert("x\n".into()).sigil(), '+');
        assert_eq!(DiffLine::Delete("x\n".into()).sigil(), '-');
        assert_eq!(DiffLine::Equal("x\n".into()).sigil(), ' ');
        assert_eq!(
            HeaderDiff::Changed {
                key: "ct".into(),
                old: "a".into(),
                new: "b".into(),
            }
            .display_parts(),
            ("ct", "a -> b".to_string())
        );
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
                truncated: false,
            }),
            error: None,
            exit_code: ExitCode::Success,
            cancelled: false,
            assertions: AssertionReport::default(),
        };

        let json = build_run_json(&result);
        let headers = json["response"]["headers"].as_array().unwrap();

        let placeholder = |name: &str| {
            headers
                .iter()
                .find(|h| h["name"] == name)
                .and_then(|h| h["value"].as_str())
                .unwrap()
                .to_string()
        };
        let auth = placeholder("authorization");
        let cookie = placeholder("set-cookie");
        assert!(auth.starts_with(crate::core::redaction::REDACTED_PREFIX));
        assert!(cookie.starts_with(crate::core::redaction::REDACTED_PREFIX));
        assert_ne!(
            auth, cookie,
            "distinct secrets must not share a placeholder"
        );

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
                truncated: false,
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

        assert_eq!(result.assertions.pass_count(), 2);
        assert_eq!(result.assertions.fail_count(), 1);

        let output = format_run_human(&result);
        assert!(output.contains("201 (45ms) [application/json]"));
        assert!(output.contains("{\"user\":{\"id\":99}}"));
        assert!(output.contains("Assertions: 2 passed, 1 failed"));
        assert!(output.contains("PASS  expect_status: 201"));
        assert!(output.contains("PASS  expect_time_under: 500ms (actual: 45ms)"));
        assert!(output.contains("FAIL"));
        assert!(output.contains("(actual: 99)"));
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
                truncated: false,
            }),
            error: None,
            exit_code: ExitCode::Success,
            cancelled: false,
            assertions: AssertionReport::default(),
        };

        assert!(result.assertions.is_empty());

        let output = format_run_human(&result);
        assert!(output.contains("200 (10ms) [unknown]"));
        assert!(!output.contains("Assertions:"));
        assert!(!output.contains("PASS"));
        assert!(!output.contains("FAIL"));
    }
}
