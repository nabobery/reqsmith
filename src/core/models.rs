use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// HTTP methods supported by hurl.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
#[allow(dead_code)] // Variants used progressively across Phase 1 steps.
pub enum HttpMethod {
    #[default]
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

impl HttpMethod {
    #[allow(dead_code)]
    pub const ALL: [HttpMethod; 7] = [
        HttpMethod::Get,
        HttpMethod::Post,
        HttpMethod::Put,
        HttpMethod::Patch,
        HttpMethod::Delete,
        HttpMethod::Head,
        HttpMethod::Options,
    ];
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Head => "HEAD",
            HttpMethod::Options => "OPTIONS",
        };
        f.write_str(s)
    }
}

/// A key-value field used for headers and query parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyValueField {
    pub key: String,
    pub value: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// A saved HTTP request document, serializable to/from `.hurl.yml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestDocument {
    pub name: String,
    #[serde(default)]
    pub method: HttpMethod,
    pub url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<KeyValueField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<KeyValueField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Runtime-only: path this document was loaded from.
    #[serde(skip)]
    pub file_path: Option<PathBuf>,
}

impl RequestDocument {
    #[allow(dead_code)]
    pub fn new_empty(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            method: HttpMethod::default(),
            url: String::new(),
            headers: Vec::new(),
            params: Vec::new(),
            body: None,
            file_path: None,
        }
    }
}

/// Normalized response data from an executed HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct ResponseArtifact {
    pub status_code: u16,
    pub http_version: String,
    pub headers: Vec<(String, String)>,
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub duration_ms: u128,
    pub body_text: Option<String>,
}

/// A node in the collection file tree.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct CollectionNode {
    pub name: String,
    pub path: PathBuf,
    pub kind: CollectionNodeKind,
    pub children: Vec<CollectionNode>,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum CollectionNodeKind {
    Directory,
    RequestFile,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_method_serializes_to_uppercase() {
        let json = serde_json::to_string(&HttpMethod::Post).unwrap();
        assert_eq!(json, "\"POST\"");
    }

    #[test]
    fn http_method_deserializes_from_uppercase() {
        let method: HttpMethod = serde_json::from_str("\"DELETE\"").unwrap();
        assert_eq!(method, HttpMethod::Delete);
    }

    #[test]
    fn http_method_display() {
        assert_eq!(HttpMethod::Get.to_string(), "GET");
        assert_eq!(HttpMethod::Patch.to_string(), "PATCH");
    }

    #[test]
    fn key_value_field_enabled_defaults_to_true() {
        let yaml = "key: Content-Type\nvalue: application/json\n";
        let field: KeyValueField = serde_yaml::from_str(yaml).unwrap();
        assert!(field.enabled);
    }

    #[test]
    fn request_document_yaml_round_trip() {
        let doc = RequestDocument {
            name: "Create User".into(),
            method: HttpMethod::Post,
            url: "{{base_url}}/api/users".into(),
            headers: vec![KeyValueField {
                key: "Content-Type".into(),
                value: "application/json".into(),
                enabled: true,
            }],
            params: vec![],
            body: Some("{ \"name\": \"test\" }".into()),
            file_path: Some("/tmp/test.hurl.yml".into()),
        };

        let yaml = serde_yaml::to_string(&doc).unwrap();
        let parsed: RequestDocument = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(parsed.name, doc.name);
        assert_eq!(parsed.method, doc.method);
        assert_eq!(parsed.url, doc.url);
        assert_eq!(parsed.headers, doc.headers);
        assert_eq!(parsed.body, doc.body);
        // file_path is skipped in serde
        assert_eq!(parsed.file_path, None);
    }

    #[test]
    fn request_document_skips_empty_collections() {
        let doc = RequestDocument {
            name: "Simple GET".into(),
            method: HttpMethod::Get,
            url: "https://example.com".into(),
            headers: vec![],
            params: vec![],
            body: None,
            file_path: None,
        };

        let yaml = serde_yaml::to_string(&doc).unwrap();
        assert!(!yaml.contains("headers"));
        assert!(!yaml.contains("params"));
        assert!(!yaml.contains("body"));
    }

    #[test]
    fn request_document_deserializes_minimal_yaml() {
        let yaml = "name: Ping\nurl: https://example.com\n";
        let doc: RequestDocument = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(doc.name, "Ping");
        assert_eq!(doc.method, HttpMethod::Get);
        assert!(doc.headers.is_empty());
        assert!(doc.params.is_empty());
        assert_eq!(doc.body, None);
    }

    #[test]
    fn collection_node_kind_equality() {
        assert_eq!(CollectionNodeKind::Directory, CollectionNodeKind::Directory);
        assert_ne!(
            CollectionNodeKind::Directory,
            CollectionNodeKind::RequestFile
        );
    }
}
