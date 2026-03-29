use std::collections::HashMap;

use super::models::RequestDocument;

#[allow(dead_code)] // Used in Step 4.
/// Interpolate `{{variable}}` placeholders in a template string.
///
/// Returns the interpolated string on success, or a list of unresolved
/// variable names if any placeholders could not be resolved.
pub fn interpolate(template: &str, vars: &HashMap<String, String>) -> Result<String, Vec<String>> {
    let mut result = String::with_capacity(template.len());
    let mut unresolved = Vec::new();
    let mut rest = template;

    while let Some(start) = rest.find("{{") {
        result.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];

        if let Some(end) = after_open.find("}}") {
            let var_name = after_open[..end].trim();
            if let Some(value) = vars.get(var_name) {
                result.push_str(value);
            } else {
                unresolved.push(var_name.to_string());
                // Keep the original placeholder in output for visibility.
                result.push_str(&rest[start..start + 2 + end + 2]);
            }
            rest = &after_open[end + 2..];
        } else {
            // Unclosed `{{` — treat as literal text.
            result.push_str(&rest[start..]);
            rest = "";
        }
    }

    result.push_str(rest);

    if unresolved.is_empty() {
        Ok(result)
    } else {
        Err(unresolved)
    }
}

#[allow(dead_code)] // Used in Step 4.
/// Interpolate all template fields in a `RequestDocument`, returning a new
/// document with resolved values. The original document is not modified.
pub fn interpolate_document(
    doc: &RequestDocument,
    vars: &HashMap<String, String>,
) -> Result<RequestDocument, Vec<String>> {
    let mut all_unresolved = Vec::new();

    let url = match interpolate(&doc.url, vars) {
        Ok(v) => v,
        Err(mut missing) => {
            all_unresolved.append(&mut missing);
            doc.url.clone()
        }
    };

    let headers = doc
        .headers
        .iter()
        .map(|h| {
            let value = match interpolate(&h.value, vars) {
                Ok(v) => v,
                Err(mut missing) => {
                    all_unresolved.append(&mut missing);
                    h.value.clone()
                }
            };
            super::models::KeyValueField {
                key: h.key.clone(),
                value,
                enabled: h.enabled,
            }
        })
        .collect();

    let params = doc
        .params
        .iter()
        .map(|p| {
            let value = match interpolate(&p.value, vars) {
                Ok(v) => v,
                Err(mut missing) => {
                    all_unresolved.append(&mut missing);
                    p.value.clone()
                }
            };
            super::models::KeyValueField {
                key: p.key.clone(),
                value,
                enabled: p.enabled,
            }
        })
        .collect();

    let body = match &doc.body {
        Some(b) => match interpolate(b, vars) {
            Ok(v) => Some(v),
            Err(mut missing) => {
                all_unresolved.append(&mut missing);
                Some(b.clone())
            }
        },
        None => None,
    };

    if !all_unresolved.is_empty() {
        return Err(all_unresolved);
    }

    Ok(RequestDocument {
        name: doc.name.clone(),
        method: doc.method,
        url,
        headers,
        params,
        body,
        auth_plugin: doc.auth_plugin.clone(),
        assertions: doc.assertions.clone(),
        file_path: doc.file_path.clone(),
    })
}

/// Extract all `{{variable}}` names from a template string.
#[allow(dead_code)] // Used by runner and validation modules.
pub fn extract_variable_names(template: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = template;

    while let Some(start) = rest.find("{{") {
        let after_open = &rest[start + 2..];
        if let Some(end) = after_open.find("}}") {
            let var_name = after_open[..end].trim();
            if !var_name.is_empty() {
                names.push(var_name.to_string());
            }
            rest = &after_open[end + 2..];
        } else {
            break;
        }
    }

    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn interpolate_replaces_single_variable() {
        let result = interpolate("{{host}}/api", &vars(&[("host", "localhost")]));
        assert_eq!(result, Ok("localhost/api".into()));
    }

    #[test]
    fn interpolate_replaces_multiple_variables() {
        let result = interpolate(
            "{{scheme}}://{{host}}:{{port}}",
            &vars(&[
                ("scheme", "https"),
                ("host", "example.com"),
                ("port", "443"),
            ]),
        );
        assert_eq!(result, Ok("https://example.com:443".into()));
    }

    #[test]
    fn interpolate_returns_missing_variables() {
        let result = interpolate("{{a}} and {{b}}", &vars(&[("a", "found")]));
        assert_eq!(result, Err(vec!["b".to_string()]));
    }

    #[test]
    fn interpolate_returns_all_missing_variables() {
        let result = interpolate("{{x}} {{y}} {{z}}", &HashMap::new());
        let mut missing = result.unwrap_err();
        missing.sort();
        assert_eq!(missing, vec!["x", "y", "z"]);
    }

    #[test]
    fn interpolate_passes_through_text_without_placeholders() {
        let result = interpolate("no placeholders here", &HashMap::new());
        assert_eq!(result, Ok("no placeholders here".into()));
    }

    #[test]
    fn interpolate_handles_empty_string() {
        let result = interpolate("", &HashMap::new());
        assert_eq!(result, Ok(String::new()));
    }

    #[test]
    fn interpolate_handles_unclosed_braces() {
        let result = interpolate("prefix {{ no close", &HashMap::new());
        assert_eq!(result, Ok("prefix {{ no close".into()));
    }

    #[test]
    fn interpolate_trims_whitespace_in_variable_name() {
        let result = interpolate("{{ host }}/api", &vars(&[("host", "localhost")]));
        assert_eq!(result, Ok("localhost/api".into()));
    }

    #[test]
    fn interpolate_document_resolves_all_fields() {
        let doc = RequestDocument {
            name: "Test".into(),
            method: super::super::models::HttpMethod::Post,
            url: "{{base}}/users".into(),
            headers: vec![super::super::models::KeyValueField {
                key: "Authorization".into(),
                value: "Bearer {{token}}".into(),
                enabled: true,
            }],
            params: vec![super::super::models::KeyValueField {
                key: "page".into(),
                value: "{{page}}".into(),
                enabled: true,
            }],
            body: Some("{\"env\": \"{{env}}\"}".into()),
            auth_plugin: None,
            assertions: vec![],
            file_path: None,
        };

        let v = vars(&[
            ("base", "https://api.example.com"),
            ("token", "abc123"),
            ("page", "1"),
            ("env", "dev"),
        ]);

        let result = interpolate_document(&doc, &v).unwrap();
        assert_eq!(result.url, "https://api.example.com/users");
        assert_eq!(result.headers[0].value, "Bearer abc123");
        assert_eq!(result.params[0].value, "1");
        assert_eq!(result.body.unwrap(), "{\"env\": \"dev\"}");
    }

    #[test]
    fn interpolate_document_reports_missing_vars() {
        let doc = RequestDocument {
            name: "Test".into(),
            method: super::super::models::HttpMethod::Get,
            url: "{{base}}/users".into(),
            headers: vec![],
            params: vec![],
            body: Some("{{missing_body_var}}".into()),
            auth_plugin: None,
            assertions: vec![],
            file_path: None,
        };

        let result = interpolate_document(&doc, &HashMap::new());
        let missing = result.unwrap_err();
        assert!(missing.contains(&"base".to_string()));
        assert!(missing.contains(&"missing_body_var".to_string()));
    }
}
