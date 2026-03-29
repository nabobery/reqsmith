use std::path::Path;

use super::models::{KeyValueField, RequestDocument};
use super::repository;

/// Format a `RequestDocument` into canonical YAML with deterministic key ordering.
///
/// Field order: name, method, url, headers (sorted by key), params (sorted by key), body.
/// Empty collections are omitted.
pub fn format_document(doc: &RequestDocument) -> String {
    let mut lines = Vec::new();

    lines.push(format!("name: {:?}", doc.name));
    lines.push(format!("method: {}", doc.method));
    lines.push(format!("url: {:?}", doc.url));

    let mut headers: Vec<&KeyValueField> = doc.headers.iter().collect();
    headers.sort_by(|a, b| a.key.cmp(&b.key));
    if !headers.is_empty() {
        lines.push("headers:".into());
        for h in &headers {
            lines.push(format!("- key: {:?}", h.key));
            lines.push(format!("  value: {:?}", h.value));
            if !h.enabled {
                lines.push("  enabled: false".into());
            }
        }
    }

    let mut params: Vec<&KeyValueField> = doc.params.iter().collect();
    params.sort_by(|a, b| a.key.cmp(&b.key));
    if !params.is_empty() {
        lines.push("params:".into());
        for p in &params {
            lines.push(format!("- key: {:?}", p.key));
            lines.push(format!("  value: {:?}", p.value));
            if !p.enabled {
                lines.push("  enabled: false".into());
            }
        }
    }

    if let Some(body) = &doc.body {
        if body.contains('\n') {
            lines.push("body: |".into());
            for line in body.lines() {
                lines.push(format!("  {line}"));
            }
        } else {
            lines.push(format!("body: {:?}", body));
        }
    }

    let mut result = lines.join("\n");
    result.push('\n');
    result
}

/// Check if a file is already formatted canonically.
#[allow(dead_code)] // Used in Step 8.
pub fn is_formatted(path: &Path) -> Result<bool, FormatterError> {
    let doc = repository::load_request(path).map_err(|e| FormatterError::Load(e.to_string()))?;
    let formatted = format_document(&doc);
    let current = std::fs::read_to_string(path).map_err(|e| FormatterError::Io(e.to_string()))?;
    Ok(formatted == current)
}

/// Format a file in-place. Returns `true` if the file was changed.
#[allow(dead_code)] // Used in Step 8.
pub fn format_file(path: &Path) -> Result<bool, FormatterError> {
    let doc = repository::load_request(path).map_err(|e| FormatterError::Load(e.to_string()))?;
    let formatted = format_document(&doc);
    let current = std::fs::read_to_string(path).map_err(|e| FormatterError::Io(e.to_string()))?;
    if formatted == current {
        return Ok(false);
    }
    std::fs::write(path, &formatted).map_err(|e| FormatterError::Io(e.to_string()))?;
    Ok(true)
}

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum FormatterError {
    #[error("Failed to load request: {0}")]
    Load(String),
    #[error("I/O error: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::HttpMethod;

    fn simple_doc() -> RequestDocument {
        RequestDocument {
            name: "Get Users".into(),
            method: HttpMethod::Get,
            url: "https://example.com/users".into(),
            headers: vec![],
            params: vec![],
            body: None,
            file_path: None,
        }
    }

    #[test]
    fn format_minimal_doc() {
        let doc = simple_doc();
        let output = format_document(&doc);
        assert!(output.starts_with("name:"));
        assert!(output.contains("method: GET"));
        assert!(output.contains("url:"));
        assert!(!output.contains("headers"));
        assert!(!output.contains("params"));
        assert!(!output.contains("body"));
        assert!(output.ends_with('\n'));
    }

    #[test]
    fn format_is_idempotent() {
        let doc = RequestDocument {
            name: "Test".into(),
            method: HttpMethod::Post,
            url: "https://example.com/api".into(),
            headers: vec![
                KeyValueField {
                    key: "Content-Type".into(),
                    value: "application/json".into(),
                    enabled: true,
                },
                KeyValueField {
                    key: "Authorization".into(),
                    value: "Bearer token".into(),
                    enabled: true,
                },
            ],
            params: vec![KeyValueField {
                key: "page".into(),
                value: "1".into(),
                enabled: true,
            }],
            body: Some("{\"name\": \"test\"}".into()),
            file_path: None,
        };

        let first = format_document(&doc);
        // Parse back and format again
        let reparsed: RequestDocument = serde_yaml::from_str(&first).unwrap();
        let second = format_document(&reparsed);
        assert_eq!(first, second, "Formatting should be idempotent");
    }

    #[test]
    fn format_sorts_headers_by_key() {
        let doc = RequestDocument {
            name: "Test".into(),
            method: HttpMethod::Get,
            url: "https://example.com".into(),
            headers: vec![
                KeyValueField {
                    key: "Z-Header".into(),
                    value: "z".into(),
                    enabled: true,
                },
                KeyValueField {
                    key: "A-Header".into(),
                    value: "a".into(),
                    enabled: true,
                },
            ],
            params: vec![],
            body: None,
            file_path: None,
        };

        let output = format_document(&doc);
        let a_pos = output.find("A-Header").unwrap();
        let z_pos = output.find("Z-Header").unwrap();
        assert!(a_pos < z_pos, "Headers should be sorted alphabetically");
    }

    #[test]
    fn format_multiline_body() {
        let doc = RequestDocument {
            name: "Test".into(),
            method: HttpMethod::Post,
            url: "https://example.com".into(),
            headers: vec![],
            params: vec![],
            body: Some("line1\nline2\nline3".into()),
            file_path: None,
        };

        let output = format_document(&doc);
        assert!(output.contains("body: |"));
        assert!(output.contains("  line1"));
        assert!(output.contains("  line2"));
    }

    #[test]
    fn format_disabled_field() {
        let doc = RequestDocument {
            name: "Test".into(),
            method: HttpMethod::Get,
            url: "https://example.com".into(),
            headers: vec![KeyValueField {
                key: "X-Disabled".into(),
                value: "val".into(),
                enabled: false,
            }],
            params: vec![],
            body: None,
            file_path: None,
        };

        let output = format_document(&doc);
        assert!(output.contains("enabled: false"));
    }

    #[test]
    fn format_file_writes_and_reports_change() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.hurl.yml");

        // Write unformatted YAML (different key order than canonical)
        std::fs::write(&path, "url: https://example.com\nname: Test\nmethod: GET\n").unwrap();

        let changed = format_file(&path).unwrap();
        assert!(changed, "File should be reformatted");

        // Second run should be no-op
        let changed_again = format_file(&path).unwrap();
        assert!(!changed_again, "Already formatted file should not change");
    }

    #[test]
    fn is_formatted_detects_unformatted() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.hurl.yml");
        std::fs::write(&path, "url: https://example.com\nname: Test\nmethod: GET\n").unwrap();

        assert!(!is_formatted(&path).unwrap());

        format_file(&path).unwrap();
        assert!(is_formatted(&path).unwrap());
    }
}
