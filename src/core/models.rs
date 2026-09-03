use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Value as JsonValue;

/// HTTP methods supported by reqsmith.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
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

/// A saved HTTP request document, serializable to/from `.req.yml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_plugin: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assertions: Vec<Assertion>,
    /// Runtime-only: path this document was loaded from.
    #[serde(skip)]
    pub file_path: Option<PathBuf>,
}

/// Normalized response data from an executed HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseArtifact {
    pub status_code: u16,
    pub http_version: String,
    pub headers: Vec<(String, String)>,
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub duration_ms: u128,
    pub body_text: Option<String>,
    pub body_bytes: Option<Vec<u8>>,
    pub is_binary: bool,
    /// The body hit the read cap and is a prefix of what the server sent.
    pub truncated: bool,
}

/// A node in the collection file tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionNode {
    pub name: String,
    pub path: PathBuf,
    pub kind: CollectionNodeKind,
    pub children: Vec<CollectionNode>,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionNodeKind {
    Directory,
    RequestFile,
}

/// Tracks where a resolved variable came from (for diagnostics/debugging).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarSource {
    OsEnv,
    DotEnv,
    DotEnvNamed(String),
    ReqsmithEnvsYml(String),
    CliOverride,
    #[cfg(feature = "plugins")]
    Plugin,
}

impl fmt::Display for VarSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VarSource::OsEnv => write!(f, "OS environment"),
            VarSource::DotEnv => write!(f, ".env"),
            VarSource::DotEnvNamed(name) => write!(f, ".env.{name}"),
            VarSource::ReqsmithEnvsYml(name) => write!(f, "reqsmith_envs.yml [{name}]"),
            VarSource::CliOverride => write!(f, "--var"),
            #[cfg(feature = "plugins")]
            VarSource::Plugin => write!(f, "plugin"),
        }
    }
}

/// A resolved environment with provenance tracking for each variable.
#[derive(Debug, Clone, Default)]
pub struct EnvironmentSet {
    /// The merged variable map (final resolved values).
    pub values: HashMap<String, String>,
    /// Source provenance for each variable (for diagnostics).
    pub sources: HashMap<String, VarSource>,
}

/// Severity level for a validation diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationSeverity {
    Error,
    Warning,
}

/// A single validation finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationDiagnostic {
    pub severity: ValidationSeverity,
    pub message: String,
    /// The field path this diagnostic relates to, e.g. "url" or "headers[0].key".
    pub field: Option<String>,
}

/// Collection of validation diagnostics for a request document.
#[derive(Debug, Clone, Default)]
pub struct ValidationReport {
    pub diagnostics: Vec<ValidationDiagnostic>,
}

impl ValidationReport {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == ValidationSeverity::Error)
    }

    #[cfg(test)]
    pub fn has_warnings(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == ValidationSeverity::Warning)
    }

    pub fn error_messages(&self) -> Vec<&str> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == ValidationSeverity::Error)
            .map(|d| d.message.as_str())
            .collect()
    }
}

/// Exit code for headless CLI execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    Success = 0,
    InternalError = 1,
    ValidationFailure = 2,
    NetworkFailure = 3,
    AssertionFailure = 4,
    Interrupted = 130,
}

impl From<ExitCode> for std::process::ExitCode {
    fn from(code: ExitCode) -> Self {
        std::process::ExitCode::from(code as u8)
    }
}

/// Result of executing a request (used by both TUI and CLI).
#[derive(Debug, Clone)]
pub struct RunResult {
    pub request_name: String,
    pub request_file: PathBuf,
    pub response: Option<ResponseArtifact>,
    pub error: Option<String>,
    pub exit_code: ExitCode,
    pub cancelled: bool,
    pub assertions: AssertionReport,
}

/// A single assertion to evaluate against a response.
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::enum_variant_names,
    reason = "each variant names the assertion key it deserializes from"
)]
pub enum Assertion {
    ExpectStatus(u16),
    ExpectTimeUnder(u64),
    ExpectBodyPath {
        path: String,
        operator: AssertionOperator,
        expected: JsonValue,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[expect(
    clippy::enum_variant_names,
    reason = "wire variants intentionally mirror the public assertion keys"
)]
enum AssertionWire {
    ExpectStatus { expect_status: u16 },
    ExpectTimeUnder { expect_time_under: Milliseconds },
    ExpectBodyPath { expect_body_path: BodyPathAssertion },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct BodyPathAssertion {
    path: String,
    operator: AssertionOperator,
    expected: JsonValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Milliseconds(u64);

impl Serialize for Milliseconds {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{}ms", self.0))
    }
}

impl<'de> Deserialize<'de> for Milliseconds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MillisecondsVisitor;

        impl<'de> de::Visitor<'de> for MillisecondsVisitor {
            type Value = Milliseconds;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an integer millisecond value or a string ending in 'ms'")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Milliseconds(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let value =
                    u64::try_from(value).map_err(|_| E::custom("milliseconds must be >= 0"))?;
                Ok(Milliseconds(value))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let raw = value.trim();
                if let Some(stripped) = raw.strip_suffix("ms") {
                    let millis = stripped
                        .trim()
                        .parse::<u64>()
                        .map_err(|_| E::custom("invalid millisecond value"))?;
                    return Ok(Milliseconds(millis));
                }

                let millis = raw
                    .parse::<u64>()
                    .map_err(|_| E::custom("expected integer milliseconds or '<n>ms'"))?;
                Ok(Milliseconds(millis))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_any(MillisecondsVisitor)
    }
}

impl Serialize for Assertion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = match self {
            Assertion::ExpectStatus(code) => AssertionWire::ExpectStatus {
                expect_status: *code,
            },
            Assertion::ExpectTimeUnder(ms) => AssertionWire::ExpectTimeUnder {
                expect_time_under: Milliseconds(*ms),
            },
            Assertion::ExpectBodyPath {
                path,
                operator,
                expected,
            } => AssertionWire::ExpectBodyPath {
                expect_body_path: BodyPathAssertion {
                    path: path.clone(),
                    operator: operator.clone(),
                    expected: expected.clone(),
                },
            },
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Assertion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AssertionWire::deserialize(deserializer)?;
        Ok(match wire {
            AssertionWire::ExpectStatus { expect_status } => Assertion::ExpectStatus(expect_status),
            AssertionWire::ExpectTimeUnder { expect_time_under } => {
                Assertion::ExpectTimeUnder(expect_time_under.0)
            }
            AssertionWire::ExpectBodyPath { expect_body_path } => Assertion::ExpectBodyPath {
                path: expect_body_path.path,
                operator: expect_body_path.operator,
                expected: expect_body_path.expected,
            },
        })
    }
}

impl fmt::Display for Assertion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Assertion::ExpectStatus(code) => write!(f, "expect_status: {code}"),
            Assertion::ExpectTimeUnder(ms) => write!(f, "expect_time_under: {ms}ms"),
            Assertion::ExpectBodyPath {
                path,
                operator,
                expected,
            } => write!(f, "expect_body_path: {path} {operator} {expected}"),
        }
    }
}

/// Comparison operators for body-path assertions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertionOperator {
    Eq,
    Ne,
    Gt,
    Lt,
    Gte,
    Lte,
    Contains,
    Exists,
}

impl fmt::Display for AssertionOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            AssertionOperator::Eq => "==",
            AssertionOperator::Ne => "!=",
            AssertionOperator::Gt => ">",
            AssertionOperator::Lt => "<",
            AssertionOperator::Gte => ">=",
            AssertionOperator::Lte => "<=",
            AssertionOperator::Contains => "contains",
            AssertionOperator::Exists => "exists",
        };
        f.write_str(s)
    }
}

/// Result of evaluating a single assertion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssertionResult {
    pub assertion: Assertion,
    pub passed: bool,
    pub message: String,
    pub actual_value: Option<String>,
}

/// Collection of assertion results for a request run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AssertionReport {
    pub results: Vec<AssertionResult>,
}

impl AssertionReport {
    pub fn all_passed(&self) -> bool {
        self.results.iter().all(|r| r.passed)
    }

    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }

    pub fn pass_count(&self) -> usize {
        self.results.iter().filter(|r| r.passed).count()
    }

    pub fn fail_count(&self) -> usize {
        self.results.iter().filter(|r| !r.passed).count()
    }
}

/// A serializable snapshot of a completed run, for diffing and history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRun {
    pub request_name: String,
    pub request_file: PathBuf,
    pub timestamp: String,
    pub status_code: u16,
    pub duration_ms: u128,
    pub headers: Vec<(String, String)>,
    pub content_type: Option<String>,
    pub body_text: Option<String>,
    #[serde(default)]
    pub truncated: bool,
    pub assertions: Vec<AssertionResult>,
}

impl StoredRun {
    pub fn from_run_result(result: &RunResult) -> Option<Self> {
        let response = result.response.as_ref()?;
        Some(StoredRun {
            request_name: result.request_name.clone(),
            request_file: result.request_file.clone(),
            timestamp: chrono_like_timestamp(),
            status_code: response.status_code,
            duration_ms: response.duration_ms,
            headers: response.headers.clone(),
            content_type: response.content_type.clone(),
            body_text: response.body_text.clone(),
            truncated: response.truncated,
            assertions: result.assertions.results.clone(),
        })
    }

    pub fn to_response_artifact(&self) -> ResponseArtifact {
        ResponseArtifact {
            status_code: self.status_code,
            http_version: String::new(),
            headers: self.headers.clone(),
            content_type: self.content_type.clone(),
            content_length: None,
            duration_ms: self.duration_ms,
            body_text: self.body_text.clone(),
            body_bytes: None,
            is_binary: false,
            truncated: self.truncated,
        }
    }
}

fn chrono_like_timestamp() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", now.as_millis())
}

/// Representation of a diff between two responses.
#[derive(Debug, Clone)]
pub struct DiffArtifact {
    pub baseline_label: String,
    pub candidate_label: String,
    pub status_diff: Option<(u16, u16)>,
    pub header_diffs: Vec<HeaderDiff>,
    pub body_diff: Vec<DiffLine>,
}

#[derive(Debug, Clone)]
pub enum HeaderDiff {
    Added(String, String),
    Removed(String, String),
    Changed {
        key: String,
        old: String,
        new: String,
    },
}

impl HeaderDiff {
    /// The one-character marker every renderer uses for this kind of change.
    pub fn sigil(&self) -> char {
        match self {
            HeaderDiff::Added(..) => '+',
            HeaderDiff::Removed(..) => '-',
            HeaderDiff::Changed { .. } => '~',
        }
    }

    /// The header name and the value text to show after it, so the CLI and the
    /// TUI cannot drift apart on how a change is spelled.
    pub fn display_parts(&self) -> (&str, String) {
        match self {
            HeaderDiff::Added(key, value) | HeaderDiff::Removed(key, value) => (key, value.clone()),
            HeaderDiff::Changed { key, old, new } => (key, format!("{old} -> {new}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    Equal(String),
    Insert(String),
    Delete(String),
}

impl DiffLine {
    /// The one-character marker every renderer prefixes this line with.
    pub fn sigil(&self) -> char {
        match self {
            DiffLine::Equal(_) => ' ',
            DiffLine::Insert(_) => '+',
            DiffLine::Delete(_) => '-',
        }
    }

    pub fn text(&self) -> &str {
        match self {
            DiffLine::Equal(s) | DiffLine::Insert(s) | DiffLine::Delete(s) => s,
        }
    }
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
            auth_plugin: None,
            assertions: vec![],
            file_path: Some("/tmp/test.req.yml".into()),
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
            auth_plugin: None,
            assertions: vec![],
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
    fn request_document_deserializes_documented_assertion_yaml() {
        let yaml = r#"
name: "Create New User"
method: POST
url: "{{base_url}}/api/v1/users"
assertions:
  - expect_status: 201
  - expect_time_under: 500ms
  - expect_body_path:
      path: "$.user.id"
      operator: eq
      expected: 42
"#;

        let doc: RequestDocument = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(
            doc.assertions,
            vec![
                Assertion::ExpectStatus(201),
                Assertion::ExpectTimeUnder(500),
                Assertion::ExpectBodyPath {
                    path: "$.user.id".into(),
                    operator: AssertionOperator::Eq,
                    expected: serde_json::json!(42),
                },
            ]
        );
    }

    #[test]
    fn collection_node_kind_equality() {
        assert_eq!(CollectionNodeKind::Directory, CollectionNodeKind::Directory);
        assert_ne!(
            CollectionNodeKind::Directory,
            CollectionNodeKind::RequestFile
        );
    }

    #[test]
    fn validation_report_has_errors() {
        let mut report = ValidationReport::default();
        assert!(!report.has_errors());

        report.diagnostics.push(ValidationDiagnostic {
            severity: ValidationSeverity::Warning,
            message: "Body present on GET".into(),
            field: Some("body".into()),
        });
        assert!(!report.has_errors());

        report.diagnostics.push(ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            message: "URL is empty".into(),
            field: Some("url".into()),
        });
        assert!(report.has_errors());
    }

    #[test]
    fn validation_report_error_messages() {
        let report = ValidationReport {
            diagnostics: vec![
                ValidationDiagnostic {
                    severity: ValidationSeverity::Error,
                    message: "missing url".into(),
                    field: None,
                },
                ValidationDiagnostic {
                    severity: ValidationSeverity::Warning,
                    message: "body on GET".into(),
                    field: None,
                },
                ValidationDiagnostic {
                    severity: ValidationSeverity::Error,
                    message: "empty name".into(),
                    field: None,
                },
            ],
        };
        let errors = report.error_messages();
        assert_eq!(errors, vec!["missing url", "empty name"]);
    }

    #[test]
    fn exit_code_converts_to_process_exit_code() {
        let code: std::process::ExitCode = ExitCode::Success.into();
        // ExitCode doesn't expose its value, but we can verify the conversion compiles
        let _ = code;

        assert_eq!(ExitCode::Success as u8, 0);
        assert_eq!(ExitCode::InternalError as u8, 1);
        assert_eq!(ExitCode::ValidationFailure as u8, 2);
        assert_eq!(ExitCode::NetworkFailure as u8, 3);
        assert_eq!(ExitCode::AssertionFailure as u8, 4);
        assert_eq!(ExitCode::Interrupted as u8, 130);
    }

    #[test]
    fn var_source_display() {
        assert_eq!(VarSource::DotEnv.to_string(), ".env");
        assert_eq!(
            VarSource::DotEnvNamed("staging".into()).to_string(),
            ".env.staging"
        );
        assert_eq!(
            VarSource::ReqsmithEnvsYml("prod".into()).to_string(),
            "reqsmith_envs.yml [prod]"
        );
        assert_eq!(VarSource::CliOverride.to_string(), "--var");
    }

    #[test]
    fn environment_set_default_is_empty() {
        let env = EnvironmentSet::default();
        assert!(env.values.is_empty());
        assert!(env.sources.is_empty());
    }
}
