use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::core::models::{HttpMethod, KeyValueField, RequestDocument, ResponseArtifact};

// ---------------------------------------------------------------------------
// Plugin capability declaration
// ---------------------------------------------------------------------------

/// Declares what a plugin can do. Listed in `plugins.toml` per plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCapability {
    PreRequest,
    PostResponse,
    Authenticate,
    ProvideVariable,
}

// ---------------------------------------------------------------------------
// Pre-request hook types
// ---------------------------------------------------------------------------

/// Sent to a plugin's `pre_request` export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub params: Vec<(String, String)>,
    pub body: Option<String>,
    pub env_name: Option<String>,
}

/// Returned from a plugin's `pre_request` export.
/// `None` fields mean "unchanged".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookResult {
    pub method: Option<String>,
    pub url: Option<String>,
    pub headers: Option<Vec<(String, String)>>,
    pub params: Option<Vec<(String, String)>>,
    pub body: Option<String>,
    pub error: Option<String>,
}

impl HookContext {
    /// Build a `HookContext` from a `RequestDocument` and optional env name.
    pub fn from_document(doc: &RequestDocument, env_name: Option<&str>) -> Self {
        Self {
            method: doc.method.to_string(),
            url: doc.url.clone(),
            headers: doc
                .headers
                .iter()
                .filter(|h| h.enabled)
                .map(|h| (h.key.clone(), h.value.clone()))
                .collect(),
            params: doc
                .params
                .iter()
                .filter(|p| p.enabled)
                .map(|p| (p.key.clone(), p.value.clone()))
                .collect(),
            body: doc.body.clone(),
            env_name: env_name.map(String::from),
        }
    }
}

impl HookResult {
    /// Apply mutations from the hook result onto a cloned `RequestDocument`.
    pub fn apply_to_document(&self, doc: &RequestDocument) -> RequestDocument {
        let mut out = doc.clone();
        if let Some(ref method) = self.method
            && let Some(m) = parse_method(method)
        {
            out.method = m;
        }
        if let Some(ref url) = self.url {
            out.url = url.clone();
        }
        if let Some(ref headers) = self.headers {
            out.headers = headers
                .iter()
                .map(|(k, v)| KeyValueField {
                    key: k.clone(),
                    value: v.clone(),
                    enabled: true,
                })
                .collect();
        }
        if let Some(ref params) = self.params {
            out.params = params
                .iter()
                .map(|(k, v)| KeyValueField {
                    key: k.clone(),
                    value: v.clone(),
                    enabled: true,
                })
                .collect();
        }
        if let Some(ref body) = self.body {
            out.body = Some(body.clone());
        }
        out
    }
}

fn parse_method(s: &str) -> Option<HttpMethod> {
    match s.to_uppercase().as_str() {
        "GET" => Some(HttpMethod::Get),
        "POST" => Some(HttpMethod::Post),
        "PUT" => Some(HttpMethod::Put),
        "PATCH" => Some(HttpMethod::Patch),
        "DELETE" => Some(HttpMethod::Delete),
        "HEAD" => Some(HttpMethod::Head),
        "OPTIONS" => Some(HttpMethod::Options),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Post-response hook types
// ---------------------------------------------------------------------------

/// Sent to a plugin's `post_response` export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseContext {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub body_text: Option<String>,
    pub duration_ms: u128,
    pub content_type: Option<String>,
}

impl ResponseContext {
    pub fn from_artifact(artifact: &ResponseArtifact) -> Self {
        Self {
            status_code: artifact.status_code,
            headers: artifact.headers.clone(),
            body_text: artifact.body_text.clone(),
            duration_ms: artifact.duration_ms,
            content_type: artifact.content_type.clone(),
        }
    }

    /// Apply any mutations from a post-response hook back onto a
    /// `ResponseArtifact`.  Only `body_text` and `headers` are mutable.
    pub fn apply_to_artifact(&self, mut artifact: ResponseArtifact) -> ResponseArtifact {
        let body_changed = artifact.body_text != self.body_text;

        artifact.headers = self.headers.clone();
        artifact.content_type = header_value(&artifact.headers, "content-type");
        artifact.body_text = self.body_text.clone();

        if body_changed {
            artifact.body_bytes = None;
            artifact.is_binary = false;
            artifact.content_length = artifact.body_text.as_ref().map(|body| body.len() as u64);
        }

        artifact
    }
}

fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

// ---------------------------------------------------------------------------
// Auth provider types
// ---------------------------------------------------------------------------

/// Sent to a plugin's `authenticate` export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub auth_type: String,
    pub config: HashMap<String, String>,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// Returned from a plugin's `authenticate` export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResult {
    pub headers: Vec<(String, String)>,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Variable provider types
// ---------------------------------------------------------------------------

/// Sent to a plugin's `provide_variable` export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableRequest {
    pub name: String,
    pub config: HashMap<String, String>,
}

/// Returned from a plugin's `provide_variable` export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableResult {
    pub value: Option<String>,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_doc() -> RequestDocument {
        RequestDocument {
            name: "Test".into(),
            method: HttpMethod::Post,
            url: "https://api.example.com/users".into(),
            headers: vec![
                KeyValueField {
                    key: "Content-Type".into(),
                    value: "application/json".into(),
                    enabled: true,
                },
                KeyValueField {
                    key: "X-Disabled".into(),
                    value: "ignored".into(),
                    enabled: false,
                },
            ],
            params: vec![KeyValueField {
                key: "page".into(),
                value: "1".into(),
                enabled: true,
            }],
            body: Some(r#"{"name":"test"}"#.into()),
            auth_plugin: None,
            assertions: vec![],
            file_path: None,
        }
    }

    #[test]
    fn hook_context_from_document_filters_disabled() {
        let doc = sample_doc();
        let ctx = HookContext::from_document(&doc, Some("staging"));

        assert_eq!(ctx.method, "POST");
        assert_eq!(ctx.url, "https://api.example.com/users");
        assert_eq!(ctx.headers.len(), 1);
        assert_eq!(ctx.headers[0].0, "Content-Type");
        assert_eq!(ctx.params.len(), 1);
        assert_eq!(ctx.env_name.as_deref(), Some("staging"));
        assert!(ctx.body.is_some());
    }

    #[test]
    fn hook_result_apply_partial_mutation() {
        let doc = sample_doc();
        let result = HookResult {
            url: Some("https://new-url.com".into()),
            headers: Some(vec![("Authorization".into(), "Bearer xyz".into())]),
            ..Default::default()
        };

        let mutated = result.apply_to_document(&doc);
        assert_eq!(mutated.url, "https://new-url.com");
        assert_eq!(mutated.headers.len(), 1);
        assert_eq!(mutated.headers[0].key, "Authorization");
        // Unchanged fields preserved.
        assert_eq!(mutated.method, HttpMethod::Post);
        assert_eq!(mutated.body.as_deref(), Some(r#"{"name":"test"}"#));
    }

    #[test]
    fn hook_result_apply_method_change() {
        let doc = sample_doc();
        let result = HookResult {
            method: Some("PUT".into()),
            ..Default::default()
        };
        let mutated = result.apply_to_document(&doc);
        assert_eq!(mutated.method, HttpMethod::Put);
    }

    #[test]
    fn hook_result_apply_invalid_method_ignored() {
        let doc = sample_doc();
        let result = HookResult {
            method: Some("INVALID".into()),
            ..Default::default()
        };
        let mutated = result.apply_to_document(&doc);
        assert_eq!(mutated.method, HttpMethod::Post); // unchanged
    }

    #[test]
    fn response_context_round_trip() {
        let artifact = ResponseArtifact {
            status_code: 200,
            http_version: "HTTP/1.1".into(),
            headers: vec![("content-type".into(), "application/json".into())],
            content_type: Some("application/json".into()),
            content_length: Some(42),
            duration_ms: 150,
            body_text: Some(r#"{"ok":true}"#.into()),
            body_bytes: None,
            is_binary: false,
            truncated: false,
        };

        let ctx = ResponseContext::from_artifact(&artifact);
        assert_eq!(ctx.status_code, 200);
        assert_eq!(ctx.headers.len(), 1);

        let restored = ctx.apply_to_artifact(artifact.clone());
        assert_eq!(restored.headers, artifact.headers);
        assert_eq!(restored.body_text, artifact.body_text);
        // Immutable fields preserved.
        assert_eq!(restored.status_code, 200);
        assert_eq!(restored.http_version, "HTTP/1.1");
    }

    #[test]
    fn json_serde_round_trip_hook_context() {
        let ctx = HookContext {
            method: "GET".into(),
            url: "https://example.com".into(),
            headers: vec![("Accept".into(), "text/html".into())],
            params: vec![],
            body: None,
            env_name: None,
        };

        let json = serde_json::to_string(&ctx).unwrap();
        let restored: HookContext = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.method, ctx.method);
        assert_eq!(restored.url, ctx.url);
    }

    #[test]
    fn json_serde_round_trip_hook_result() {
        let result = HookResult {
            headers: Some(vec![("X-Custom".into(), "value".into())]),
            error: None,
            ..Default::default()
        };

        let json = serde_json::to_string(&result).unwrap();
        let restored: HookResult = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.headers, result.headers);
    }

    #[test]
    fn json_serde_round_trip_auth_types() {
        let req = AuthRequest {
            auth_type: "oauth2".into(),
            config: HashMap::from([("client_id".into(), "abc".into())]),
            method: "POST".into(),
            url: "https://api.example.com".into(),
            headers: vec![],
            body: Some("{\"hello\":\"world\"}".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let restored: AuthRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.auth_type, "oauth2");
        assert_eq!(restored.body.as_deref(), Some("{\"hello\":\"world\"}"));

        let res = AuthResult {
            headers: vec![("Authorization".into(), "Bearer tok".into())],
            error: None,
        };
        let json = serde_json::to_string(&res).unwrap();
        let restored: AuthResult = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.headers.len(), 1);
    }

    #[test]
    fn json_serde_round_trip_variable_types() {
        let req = VariableRequest {
            name: "API_KEY".into(),
            config: HashMap::new(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let restored: VariableRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.name, "API_KEY");

        let res = VariableResult {
            value: Some("secret123".into()),
            error: None,
        };
        let json = serde_json::to_string(&res).unwrap();
        let restored: VariableResult = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.value.as_deref(), Some("secret123"));
    }

    #[test]
    fn plugin_capability_serde() {
        let caps = vec![
            PluginCapability::PreRequest,
            PluginCapability::PostResponse,
            PluginCapability::Authenticate,
            PluginCapability::ProvideVariable,
        ];
        let json = serde_json::to_string(&caps).unwrap();
        assert_eq!(
            json,
            r#"["pre_request","post_response","authenticate","provide_variable"]"#
        );
        let restored: Vec<PluginCapability> = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, caps);
    }

    #[test]
    fn response_context_mutation_keeps_artifact_consistent() {
        let artifact = ResponseArtifact {
            status_code: 200,
            http_version: "HTTP/1.1".into(),
            headers: vec![("content-type".into(), "application/octet-stream".into())],
            content_type: Some("application/octet-stream".into()),
            content_length: Some(4),
            duration_ms: 50,
            body_text: None,
            body_bytes: Some(vec![0, 1, 2, 3]),
            is_binary: true,
            truncated: false,
        };

        let mutated = ResponseContext {
            status_code: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body_text: Some("{\"ok\":true}".into()),
            duration_ms: 50,
            content_type: Some("application/json".into()),
        }
        .apply_to_artifact(artifact);

        assert_eq!(mutated.content_type.as_deref(), Some("application/json"));
        assert_eq!(mutated.body_text.as_deref(), Some("{\"ok\":true}"));
        assert_eq!(mutated.content_length, Some("{\"ok\":true}".len() as u64));
        assert_eq!(mutated.body_bytes, None);
        assert!(!mutated.is_binary);
    }
}
