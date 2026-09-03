use serde_json::Value as JsonValue;
use serde_json_path::JsonPath;

use super::models::{
    Assertion, AssertionOperator, AssertionReport, AssertionResult, ResponseArtifact,
};

/// Evaluate all assertions against a response artifact.
pub fn evaluate_assertions(
    assertions: &[Assertion],
    response: &ResponseArtifact,
) -> AssertionReport {
    let results = assertions
        .iter()
        .map(|a| evaluate_one(a, response))
        .collect();
    AssertionReport { results }
}

fn evaluate_one(assertion: &Assertion, response: &ResponseArtifact) -> AssertionResult {
    match assertion {
        Assertion::ExpectStatus(expected) => {
            evaluate_status(*expected, response.status_code, assertion)
        }
        Assertion::ExpectTimeUnder(max_ms) => {
            evaluate_time_under(*max_ms, response.duration_ms, assertion)
        }
        Assertion::ExpectBodyPath {
            path,
            operator,
            expected,
        } => evaluate_body_path(
            path,
            operator,
            expected,
            response.body_text.as_deref(),
            assertion,
        ),
    }
}

fn evaluate_status(expected: u16, actual: u16, assertion: &Assertion) -> AssertionResult {
    let passed = actual == expected;
    let message = if passed {
        format!("Status {actual} matches expected {expected}")
    } else {
        format!("Expected status {expected}, got {actual}")
    };
    AssertionResult {
        assertion: assertion.clone(),
        passed,
        message,
        actual_value: Some(actual.to_string()),
    }
}

fn evaluate_time_under(max_ms: u64, actual_ms: u128, assertion: &Assertion) -> AssertionResult {
    let passed = actual_ms <= max_ms as u128;
    let message = if passed {
        format!("Response time {actual_ms}ms is under {max_ms}ms")
    } else {
        format!("Expected response under {max_ms}ms, took {actual_ms}ms")
    };
    AssertionResult {
        assertion: assertion.clone(),
        passed,
        message,
        actual_value: Some(format!("{actual_ms}ms")),
    }
}

fn evaluate_body_path(
    path: &str,
    operator: &AssertionOperator,
    expected: &JsonValue,
    body: Option<&str>,
    assertion: &Assertion,
) -> AssertionResult {
    let fail = |msg: String| AssertionResult {
        assertion: assertion.clone(),
        passed: false,
        message: msg,
        actual_value: None,
    };

    let body_str = match body {
        Some(b) => b,
        None => return fail("No response body to evaluate".into()),
    };

    let json: JsonValue = match serde_json::from_str(body_str) {
        Ok(v) => v,
        Err(e) => return fail(format!("Response body is not valid JSON: {e}")),
    };

    let jp = match JsonPath::parse(path) {
        Ok(jp) => jp,
        Err(e) => return fail(format!("Invalid JSONPath '{path}': {e}")),
    };

    let node_list = jp.query(&json);

    if matches!(operator, AssertionOperator::Exists) {
        let found = !node_list.is_empty();
        return AssertionResult {
            assertion: assertion.clone(),
            passed: found,
            message: if found {
                format!("Path '{path}' exists ({} match(es))", node_list.len())
            } else {
                format!("Path '{path}' does not exist")
            },
            actual_value: node_list.first().map(|v| v.to_string()),
        };
    }

    let actual = match node_list.first() {
        Some(v) => v,
        None => return fail(format!("JSONPath '{path}' matched no values")),
    };

    let passed = compare_values(actual, operator, expected);
    let message = if passed {
        format!("{path} {operator} {expected}")
    } else {
        format!("Expected {path} {operator} {expected}, got {actual}")
    };

    AssertionResult {
        assertion: assertion.clone(),
        passed,
        message,
        actual_value: Some(actual.to_string()),
    }
}

fn compare_values(actual: &JsonValue, operator: &AssertionOperator, expected: &JsonValue) -> bool {
    match operator {
        AssertionOperator::Eq => actual == expected,
        AssertionOperator::Ne => actual != expected,
        AssertionOperator::Gt
        | AssertionOperator::Lt
        | AssertionOperator::Gte
        | AssertionOperator::Lte => match (as_f64(actual), as_f64(expected)) {
            (Some(a), Some(e)) => match operator {
                AssertionOperator::Gt => a > e,
                AssertionOperator::Lt => a < e,
                AssertionOperator::Gte => a >= e,
                AssertionOperator::Lte => a <= e,
                _ => unreachable!(),
            },
            _ => false,
        },
        AssertionOperator::Contains => {
            let actual_str = value_as_string(actual);
            let expected_str = value_as_string(expected);
            actual_str.contains(&expected_str)
        }
        AssertionOperator::Exists => unreachable!("handled above"),
    }
}

fn as_f64(value: &JsonValue) -> Option<f64> {
    value.as_f64().or_else(|| value.as_i64().map(|i| i as f64))
}

fn value_as_string(value: &JsonValue) -> String {
    match value {
        JsonValue::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_response(status: u16, duration_ms: u128, body: Option<&str>) -> ResponseArtifact {
        ResponseArtifact {
            status_code: status,
            http_version: "HTTP/1.1".into(),
            headers: vec![],
            content_type: Some("application/json".into()),
            content_length: None,
            duration_ms,
            body_text: body.map(String::from),
            body_bytes: None,
            is_binary: false,
            truncated: false,
        }
    }

    #[test]
    fn status_assertion_passes() {
        let response = sample_response(200, 50, None);
        let result = evaluate_one(&Assertion::ExpectStatus(200), &response);
        assert!(result.passed);
        assert_eq!(result.actual_value.as_deref(), Some("200"));
    }

    #[test]
    fn status_assertion_fails() {
        let response = sample_response(404, 50, None);
        let result = evaluate_one(&Assertion::ExpectStatus(200), &response);
        assert!(!result.passed);
        assert!(result.message.contains("404"));
    }

    #[test]
    fn time_under_passes() {
        let response = sample_response(200, 45, None);
        let result = evaluate_one(&Assertion::ExpectTimeUnder(500), &response);
        assert!(result.passed);
    }

    #[test]
    fn time_under_fails() {
        let response = sample_response(200, 600, None);
        let result = evaluate_one(&Assertion::ExpectTimeUnder(500), &response);
        assert!(!result.passed);
        assert!(result.message.contains("600ms"));
    }

    #[test]
    fn body_path_eq_passes() {
        let body = r#"{"user": {"id": 42}}"#;
        let response = sample_response(200, 50, Some(body));
        let result = evaluate_one(
            &Assertion::ExpectBodyPath {
                path: "$.user.id".into(),
                operator: AssertionOperator::Eq,
                expected: json!(42),
            },
            &response,
        );
        assert!(result.passed);
    }

    #[test]
    fn body_path_eq_fails() {
        let body = r#"{"user": {"id": 99}}"#;
        let response = sample_response(200, 50, Some(body));
        let result = evaluate_one(
            &Assertion::ExpectBodyPath {
                path: "$.user.id".into(),
                operator: AssertionOperator::Eq,
                expected: json!(42),
            },
            &response,
        );
        assert!(!result.passed);
        assert_eq!(result.actual_value.as_deref(), Some("99"));
    }

    #[test]
    fn body_path_ne() {
        let body = r#"{"status": "active"}"#;
        let response = sample_response(200, 50, Some(body));
        let result = evaluate_one(
            &Assertion::ExpectBodyPath {
                path: "$.status".into(),
                operator: AssertionOperator::Ne,
                expected: json!("inactive"),
            },
            &response,
        );
        assert!(result.passed);
    }

    #[test]
    fn body_path_gt() {
        let body = r#"{"count": 10}"#;
        let response = sample_response(200, 50, Some(body));
        let result = evaluate_one(
            &Assertion::ExpectBodyPath {
                path: "$.count".into(),
                operator: AssertionOperator::Gt,
                expected: json!(5),
            },
            &response,
        );
        assert!(result.passed);
    }

    #[test]
    fn body_path_lt_fails() {
        let body = r#"{"count": 10}"#;
        let response = sample_response(200, 50, Some(body));
        let result = evaluate_one(
            &Assertion::ExpectBodyPath {
                path: "$.count".into(),
                operator: AssertionOperator::Lt,
                expected: json!(5),
            },
            &response,
        );
        assert!(!result.passed);
    }

    #[test]
    fn body_path_contains() {
        let body = r#"{"message": "hello world"}"#;
        let response = sample_response(200, 50, Some(body));
        let result = evaluate_one(
            &Assertion::ExpectBodyPath {
                path: "$.message".into(),
                operator: AssertionOperator::Contains,
                expected: json!("world"),
            },
            &response,
        );
        assert!(result.passed);
    }

    #[test]
    fn body_path_exists_passes() {
        let body = r#"{"user": {"name": "test"}}"#;
        let response = sample_response(200, 50, Some(body));
        let result = evaluate_one(
            &Assertion::ExpectBodyPath {
                path: "$.user.name".into(),
                operator: AssertionOperator::Exists,
                expected: json!(null),
            },
            &response,
        );
        assert!(result.passed);
    }

    #[test]
    fn body_path_exists_fails() {
        let body = r#"{"user": {}}"#;
        let response = sample_response(200, 50, Some(body));
        let result = evaluate_one(
            &Assertion::ExpectBodyPath {
                path: "$.user.name".into(),
                operator: AssertionOperator::Exists,
                expected: json!(null),
            },
            &response,
        );
        assert!(!result.passed);
    }

    #[test]
    fn body_path_invalid_json() {
        let response = sample_response(200, 50, Some("not json"));
        let result = evaluate_one(
            &Assertion::ExpectBodyPath {
                path: "$.x".into(),
                operator: AssertionOperator::Eq,
                expected: json!(1),
            },
            &response,
        );
        assert!(!result.passed);
        assert!(result.message.contains("not valid JSON"));
    }

    #[test]
    fn body_path_invalid_jsonpath() {
        let body = r#"{"x": 1}"#;
        let response = sample_response(200, 50, Some(body));
        let result = evaluate_one(
            &Assertion::ExpectBodyPath {
                path: "$[invalid".into(),
                operator: AssertionOperator::Eq,
                expected: json!(1),
            },
            &response,
        );
        assert!(!result.passed);
        assert!(result.message.contains("Invalid JSONPath"));
    }

    #[test]
    fn body_path_no_body() {
        let response = sample_response(200, 50, None);
        let result = evaluate_one(
            &Assertion::ExpectBodyPath {
                path: "$.x".into(),
                operator: AssertionOperator::Eq,
                expected: json!(1),
            },
            &response,
        );
        assert!(!result.passed);
        assert!(result.message.contains("No response body"));
    }

    #[test]
    fn body_path_no_match() {
        let body = r#"{"a": 1}"#;
        let response = sample_response(200, 50, Some(body));
        let result = evaluate_one(
            &Assertion::ExpectBodyPath {
                path: "$.nonexistent".into(),
                operator: AssertionOperator::Eq,
                expected: json!(1),
            },
            &response,
        );
        assert!(!result.passed);
        assert!(result.message.contains("matched no values"));
    }

    #[test]
    fn empty_assertions_returns_empty_report() {
        let response = sample_response(200, 50, None);
        let report = evaluate_assertions(&[], &response);
        assert!(report.is_empty());
        assert!(report.all_passed());
    }

    #[test]
    fn mixed_pass_fail_report() {
        let body = r#"{"id": 99}"#;
        let response = sample_response(200, 50, Some(body));
        let assertions = vec![
            Assertion::ExpectStatus(200),
            Assertion::ExpectBodyPath {
                path: "$.id".into(),
                operator: AssertionOperator::Eq,
                expected: json!(42),
            },
        ];
        let report = evaluate_assertions(&assertions, &response);
        assert!(!report.all_passed());
        assert_eq!(report.pass_count(), 1);
        assert_eq!(report.fail_count(), 1);
    }

    #[test]
    fn assertion_serde_round_trip() {
        let assertions = vec![
            Assertion::ExpectStatus(201),
            Assertion::ExpectTimeUnder(500),
            Assertion::ExpectBodyPath {
                path: "$.user.id".into(),
                operator: AssertionOperator::Eq,
                expected: json!(42),
            },
        ];
        let yaml = serde_yaml::to_string(&assertions).unwrap();
        let parsed: Vec<Assertion> = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(assertions, parsed);
    }
}
