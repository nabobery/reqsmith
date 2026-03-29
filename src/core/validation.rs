use std::str::FromStr;

use reqwest::Url;
use reqwest::header::HeaderName;
use serde_json_path::JsonPath;

use super::interpolation::{extract_variable_names, interpolate};
use super::models::{
    Assertion, EnvironmentSet, HttpMethod, RequestDocument, ValidationDiagnostic, ValidationReport,
    ValidationSeverity,
};

/// Validate a request document for schema correctness and interpolation completeness.
#[allow(dead_code)] // Used in Step 8.
pub fn validate_document(doc: &RequestDocument, env: Option<&EnvironmentSet>) -> ValidationReport {
    let mut diagnostics = Vec::new();

    // Schema checks.
    if doc.name.trim().is_empty() {
        diagnostics.push(ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            message: "Request name is empty".into(),
            field: Some("name".into()),
        });
    }

    if doc.url.trim().is_empty() {
        diagnostics.push(ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            message: "URL is empty".into(),
            field: Some("url".into()),
        });
    }

    if let Some(auth_plugin) = &doc.auth_plugin {
        if auth_plugin.trim().is_empty() {
            diagnostics.push(ValidationDiagnostic {
                severity: ValidationSeverity::Error,
                message: "auth_plugin must not be empty when present".into(),
                field: Some("auth_plugin".into()),
            });
        }
    }

    // Header key checks.
    for (i, header) in doc.headers.iter().enumerate() {
        if header.key.trim().is_empty() {
            diagnostics.push(ValidationDiagnostic {
                severity: ValidationSeverity::Error,
                message: format!("Header at index {i} has an empty key"),
                field: Some(format!("headers[{i}].key")),
            });
        } else if HeaderName::from_str(&header.key).is_err() {
            diagnostics.push(ValidationDiagnostic {
                severity: ValidationSeverity::Error,
                message: format!("Header at index {i} has an invalid header name"),
                field: Some(format!("headers[{i}].key")),
            });
        }
    }

    // Body consistency: warn if body present for GET/HEAD/OPTIONS.
    if doc.body.is_some() {
        match doc.method {
            HttpMethod::Get | HttpMethod::Head | HttpMethod::Options => {
                diagnostics.push(ValidationDiagnostic {
                    severity: ValidationSeverity::Warning,
                    message: format!(
                        "Body present on {} request (unusual, may be ignored by server)",
                        doc.method
                    ),
                    field: Some("body".into()),
                });
            }
            _ => {}
        }
    }

    // Interpolation completeness check (if environment provided).
    if let Some(env) = env {
        let mut all_vars = Vec::new();
        all_vars.extend(extract_variable_names(&doc.url));
        for h in &doc.headers {
            all_vars.extend(extract_variable_names(&h.value));
        }
        for p in &doc.params {
            all_vars.extend(extract_variable_names(&p.value));
        }
        if let Some(body) = &doc.body {
            all_vars.extend(extract_variable_names(body));
        }

        all_vars.sort();
        all_vars.dedup();

        for var in &all_vars {
            if !env.values.contains_key(var) {
                // Check OS env as last fallback.
                if std::env::var(var).is_err() {
                    diagnostics.push(ValidationDiagnostic {
                        severity: ValidationSeverity::Error,
                        message: format!("Unresolved variable: {{{{{var}}}}}"),
                        field: None,
                    });
                }
            }
        }
    }

    // Assertion checks.
    diagnostics.extend(validate_assertions(&doc.assertions));

    if let Some(url_for_validation) = interpolate_url_for_validation(&doc.url, env) {
        match Url::parse(&url_for_validation) {
            Ok(_) => {}
            Err(error) => {
                if error.to_string().contains("relative URL without a base") {
                    diagnostics.push(ValidationDiagnostic {
                        severity: ValidationSeverity::Warning,
                        message: "URL does not start with http:// or https:// scheme".into(),
                        field: Some("url".into()),
                    });
                } else {
                    diagnostics.push(ValidationDiagnostic {
                        severity: ValidationSeverity::Error,
                        message: format!("URL is invalid: {error}"),
                        field: Some("url".into()),
                    });
                }
            }
        }
    }

    ValidationReport { diagnostics }
}

/// Validate assertions for schema correctness.
fn validate_assertions(assertions: &[Assertion]) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();

    for (i, assertion) in assertions.iter().enumerate() {
        match assertion {
            Assertion::ExpectBodyPath { path, .. } => {
                if let Err(err) = JsonPath::parse(path) {
                    diagnostics.push(ValidationDiagnostic {
                        severity: ValidationSeverity::Error,
                        message: format!("Assertion at index {i} has invalid JSONPath: {err}"),
                        field: Some(format!("assertions[{i}].path")),
                    });
                }
            }
            Assertion::ExpectTimeUnder(ms) => {
                if *ms < 1 {
                    diagnostics.push(ValidationDiagnostic {
                        severity: ValidationSeverity::Warning,
                        message: format!(
                            "Assertion at index {i}: time threshold {ms}ms is unreasonably low"
                        ),
                        field: Some(format!("assertions[{i}]")),
                    });
                }
            }
            Assertion::ExpectStatus(code) => {
                if *code < 100 || *code > 599 {
                    diagnostics.push(ValidationDiagnostic {
                        severity: ValidationSeverity::Warning,
                        message: format!(
                            "Assertion at index {i}: status code {code} is outside the valid 100-599 range"
                        ),
                        field: Some(format!("assertions[{i}]")),
                    });
                }
            }
        }
    }

    diagnostics
}

fn interpolate_url_for_validation(
    url_template: &str,
    env: Option<&EnvironmentSet>,
) -> Option<String> {
    if !url_template.contains("{{") {
        return Some(url_template.to_string());
    }

    let env = env?;
    let mut vars = env.values.clone();
    for variable in extract_variable_names(url_template) {
        if let Ok(value) = std::env::var(&variable) {
            if let std::collections::hash_map::Entry::Vacant(entry) = vars.entry(variable) {
                entry.insert(value);
            }
        }
    }

    interpolate(url_template, &vars).ok()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::core::models::{AssertionOperator, KeyValueField};

    fn simple_doc() -> RequestDocument {
        RequestDocument {
            name: "Test Request".into(),
            method: HttpMethod::Get,
            url: "https://example.com/api".into(),
            headers: vec![],
            params: vec![],
            body: None,
            auth_plugin: None,
            assertions: vec![],
            file_path: None,
        }
    }

    fn env_with(vars: &[(&str, &str)]) -> EnvironmentSet {
        EnvironmentSet {
            values: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            sources: HashMap::new(),
        }
    }

    #[test]
    fn valid_doc_passes_clean() {
        let doc = simple_doc();
        let report = validate_document(&doc, None);
        assert!(!report.has_errors());
    }

    #[test]
    fn empty_name_is_error() {
        let mut doc = simple_doc();
        doc.name = "".into();
        let report = validate_document(&doc, None);
        assert!(report.has_errors());
        assert!(report.error_messages().iter().any(|m| m.contains("name")));
    }

    #[test]
    fn empty_url_is_error() {
        let mut doc = simple_doc();
        doc.url = "".into();
        let report = validate_document(&doc, None);
        assert!(report.has_errors());
        assert!(report.error_messages().iter().any(|m| m.contains("URL")));
    }

    #[test]
    fn empty_auth_plugin_is_error() {
        let mut doc = simple_doc();
        doc.auth_plugin = Some(String::new());

        let report = validate_document(&doc, None);

        assert!(report.has_errors());
        assert!(
            report
                .error_messages()
                .iter()
                .any(|m| m.contains("auth_plugin"))
        );
    }

    #[test]
    fn empty_header_key_is_error() {
        let mut doc = simple_doc();
        doc.headers = vec![KeyValueField {
            key: "".into(),
            value: "value".into(),
            enabled: true,
        }];
        let report = validate_document(&doc, None);
        assert!(report.has_errors());
        assert!(
            report
                .error_messages()
                .iter()
                .any(|m| m.contains("empty key"))
        );
    }

    #[test]
    fn body_on_get_is_warning() {
        let mut doc = simple_doc();
        doc.body = Some("some body".into());
        let report = validate_document(&doc, None);
        assert!(!report.has_errors());
        assert!(report.has_warnings());
    }

    #[test]
    fn body_on_post_is_fine() {
        let mut doc = simple_doc();
        doc.method = HttpMethod::Post;
        doc.body = Some("some body".into());
        let report = validate_document(&doc, None);
        assert!(!report.has_errors());
        assert!(!report.has_warnings());
    }

    #[test]
    fn missing_variable_is_error() {
        let mut doc = simple_doc();
        doc.url = "{{base_url}}/api".into();
        let env = env_with(&[]);
        let report = validate_document(&doc, Some(&env));
        assert!(report.has_errors());
        assert!(
            report
                .error_messages()
                .iter()
                .any(|m| m.contains("base_url"))
        );
    }

    #[test]
    fn resolved_variable_passes() {
        let mut doc = simple_doc();
        doc.url = "{{base_url}}/api".into();
        let env = env_with(&[("base_url", "https://example.com")]);
        let report = validate_document(&doc, Some(&env));
        assert!(!report.has_errors());
    }

    #[test]
    fn url_without_scheme_is_warning() {
        let mut doc = simple_doc();
        doc.url = "example.com/api".into();
        let report = validate_document(&doc, None);
        assert!(!report.has_errors());
        assert!(report.has_warnings());
    }

    #[test]
    fn url_with_variable_prefix_no_scheme_warning() {
        let mut doc = simple_doc();
        doc.url = "{{base_url}}/api".into();
        // No env provided, so no interpolation check; URL has variables so scheme check skipped
        let report = validate_document(&doc, None);
        assert!(!report.has_errors());
    }

    #[test]
    fn invalid_header_name_is_error() {
        let mut doc = simple_doc();
        doc.headers = vec![KeyValueField {
            key: "Bad Header".into(),
            value: "value".into(),
            enabled: true,
        }];

        let report = validate_document(&doc, None);
        assert!(report.has_errors());
        assert!(
            report
                .error_messages()
                .iter()
                .any(|message| message.to_ascii_lowercase().contains("header"))
        );
    }

    #[test]
    fn malformed_absolute_url_is_error() {
        let mut doc = simple_doc();
        doc.url = "http://".into();

        let report = validate_document(&doc, None);
        assert!(report.has_errors());
        assert!(
            report
                .error_messages()
                .iter()
                .any(|message| message.contains("URL"))
        );
    }

    #[test]
    fn multiple_errors_collected() {
        let doc = RequestDocument {
            name: "".into(),
            method: HttpMethod::Get,
            url: "".into(),
            headers: vec![KeyValueField {
                key: "".into(),
                value: "val".into(),
                enabled: true,
            }],
            params: vec![],
            body: None,
            auth_plugin: None,
            assertions: vec![],
            file_path: None,
        };
        let report = validate_document(&doc, None);
        assert!(report.diagnostics.len() >= 3);
    }

    #[test]
    fn invalid_jsonpath_is_error() {
        let mut doc = simple_doc();
        doc.assertions = vec![Assertion::ExpectBodyPath {
            path: "$[invalid~~path".into(),
            operator: AssertionOperator::Eq,
            expected: serde_json::json!("value"),
        }];
        let report = validate_document(&doc, None);
        assert!(report.has_errors());
        assert!(
            report
                .error_messages()
                .iter()
                .any(|m| m.contains("JSONPath"))
        );
    }

    #[test]
    fn valid_assertions_pass() {
        let mut doc = simple_doc();
        doc.assertions = vec![
            Assertion::ExpectStatus(200),
            Assertion::ExpectTimeUnder(5000),
            Assertion::ExpectBodyPath {
                path: "$.data.id".into(),
                operator: AssertionOperator::Eq,
                expected: serde_json::json!(1),
            },
        ];
        let report = validate_document(&doc, None);
        assert!(!report.has_errors());
        assert!(!report.has_warnings());
    }
}
