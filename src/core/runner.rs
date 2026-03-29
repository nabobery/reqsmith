use std::path::PathBuf;

use tokio_util::sync::CancellationToken;

use super::assertions;
use super::environment::{apply_os_env_fallback, resolve_environment};
use super::execution::{self, ExecutionError};
use super::interpolation::extract_variable_names;
use super::models::{AssertionReport, ExitCode, RequestDocument, RunResult};
use super::validation::validate_document;

/// Options controlling how a request is run.
#[allow(dead_code)] // Used in Step 8.
pub struct RunOptions {
    pub env_name: Option<String>,
    pub cli_vars: Vec<(String, String)>,
    pub validate_before_run: bool,
    pub cwd: PathBuf,
}

/// Run a request through the shared execution pipeline.
///
/// Used by both TUI and CLI to ensure identical behavior.
#[allow(dead_code)] // Used in Step 8 + Step 10.
pub async fn run_request(
    client: &reqwest::Client,
    doc: &RequestDocument,
    options: &RunOptions,
    cancel: CancellationToken,
) -> RunResult {
    let request_name = doc.name.clone();
    let request_file = doc.file_path.clone().unwrap_or_default();
    let resolution_root = request_resolution_root(doc, &options.cwd);

    // 1. Resolve environment.
    let mut env = match resolve_environment(
        resolution_root,
        options.env_name.as_deref(),
        &options.cli_vars,
    ) {
        Ok(env) => env,
        Err(e) => {
            return RunResult {
                request_name,
                request_file,
                response: None,
                error: Some(e.to_string()),
                exit_code: ExitCode::InternalError,
                cancelled: false,
                assertions: AssertionReport::default(),
            };
        }
    };

    // 2. Apply OS env fallback for variables referenced in the document.
    let referenced_vars = extract_variable_names_from_doc(doc);
    apply_os_env_fallback(&mut env, &referenced_vars);

    // 3. Optional pre-flight validation.
    if options.validate_before_run {
        let report = validate_document(doc, Some(&env));
        if report.has_errors() {
            let errors = report.error_messages().join("; ");
            return RunResult {
                request_name,
                request_file,
                response: None,
                error: Some(errors),
                exit_code: ExitCode::ValidationFailure,
                cancelled: false,
                assertions: AssertionReport::default(),
            };
        }
    }

    // 4. Execute.
    match execution::execute_request(client, doc, &env.values, cancel).await {
        Ok(artifact) => {
            let assertion_report = if doc.assertions.is_empty() {
                AssertionReport::default()
            } else {
                assertions::evaluate_assertions(&doc.assertions, &artifact)
            };
            let exit_code = if assertion_report.all_passed() {
                ExitCode::Success
            } else {
                ExitCode::AssertionFailure
            };
            RunResult {
                request_name,
                request_file,
                response: Some(artifact),
                error: None,
                exit_code,
                cancelled: false,
                assertions: assertion_report,
            }
        }
        Err(ExecutionError::Cancelled) => RunResult {
            request_name,
            request_file,
            response: None,
            error: None,
            exit_code: ExitCode::Interrupted,
            cancelled: true,
            assertions: AssertionReport::default(),
        },
        Err(ExecutionError::Network(msg)) => RunResult {
            request_name,
            request_file,
            response: None,
            error: Some(msg),
            exit_code: ExitCode::NetworkFailure,
            cancelled: false,
            assertions: AssertionReport::default(),
        },
        Err(ExecutionError::Interpolation(vars)) => RunResult {
            request_name,
            request_file,
            response: None,
            error: Some(format!("Unresolved variables: {}", vars.join(", "))),
            exit_code: ExitCode::ValidationFailure,
            cancelled: false,
            assertions: AssertionReport::default(),
        },
        Err(e) => RunResult {
            request_name,
            request_file,
            response: None,
            error: Some(e.to_string()),
            exit_code: ExitCode::InternalError,
            cancelled: false,
            assertions: AssertionReport::default(),
        },
    }
}

/// Extract all `{{variable}}` names referenced in a request document.
fn extract_variable_names_from_doc(doc: &RequestDocument) -> Vec<String> {
    let mut names = Vec::new();
    names.extend(extract_variable_names(&doc.url));
    for header in &doc.headers {
        names.extend(extract_variable_names(&header.value));
    }
    for param in &doc.params {
        names.extend(extract_variable_names(&param.value));
    }
    if let Some(body) = &doc.body {
        names.extend(extract_variable_names(body));
    }
    names.sort();
    names.dedup();
    names
}

fn request_resolution_root<'a>(
    doc: &'a RequestDocument,
    default_root: &'a std::path::Path,
) -> &'a std::path::Path {
    doc.file_path
        .as_deref()
        .and_then(std::path::Path::parent)
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(default_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::HttpMethod;

    fn simple_doc(url: &str) -> RequestDocument {
        RequestDocument {
            name: "Test".into(),
            method: HttpMethod::Get,
            url: url.into(),
            headers: vec![],
            params: vec![],
            body: None,
            assertions: vec![],
            file_path: Some(PathBuf::from("test.hurl.yml")),
        }
    }

    fn default_options(cwd: &std::path::Path) -> RunOptions {
        RunOptions {
            env_name: None,
            cli_vars: vec![],
            validate_before_run: false,
            cwd: cwd.to_path_buf(),
        }
    }

    #[test]
    fn extract_variable_names_from_doc_finds_all() {
        use crate::core::models::KeyValueField;
        let doc = RequestDocument {
            name: "Test".into(),
            method: HttpMethod::Post,
            url: "{{base_url}}/api".into(),
            headers: vec![KeyValueField {
                key: "Authorization".into(),
                value: "Bearer {{token}}".into(),
                enabled: true,
            }],
            params: vec![KeyValueField {
                key: "env".into(),
                value: "{{env_name}}".into(),
                enabled: true,
            }],
            body: Some("{{body_content}}".into()),
            assertions: vec![],
            file_path: None,
        };

        let names = extract_variable_names_from_doc(&doc);
        assert_eq!(names, vec!["base_url", "body_content", "env_name", "token"]);
    }

    #[tokio::test]
    async fn run_request_env_resolution_failure() {
        let tmp = tempfile::tempdir().unwrap();
        // Create hurl_envs.yml without the named env to trigger error
        std::fs::write(
            tmp.path().join("hurl_envs.yml"),
            "environments:\n  prod:\n    X: y\n",
        )
        .unwrap();

        let doc = simple_doc("http://localhost");
        let options = RunOptions {
            env_name: Some("nonexistent".into()),
            cli_vars: vec![],
            validate_before_run: false,
            cwd: tmp.path().to_path_buf(),
        };

        let client = reqwest::Client::new();
        let result = run_request(&client, &doc, &options, CancellationToken::new()).await;
        assert_eq!(result.exit_code, ExitCode::InternalError);
        assert!(result.error.unwrap().contains("nonexistent"));
    }

    #[tokio::test]
    async fn run_request_network_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let doc = simple_doc("http://127.0.0.1:9"); // closed local port should fail quickly
        let options = default_options(tmp.path());
        let token = CancellationToken::new();

        let client = reqwest::Client::new();
        let result = run_request(&client, &doc, &options, token).await;
        assert_eq!(result.exit_code, ExitCode::NetworkFailure);
        assert!(result.error.is_some());
    }

    #[tokio::test]
    async fn run_request_interpolation_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let doc = simple_doc("{{missing_var}}/api");
        let options = default_options(tmp.path());

        let client = reqwest::Client::new();
        let result = run_request(&client, &doc, &options, CancellationToken::new()).await;
        assert_eq!(result.exit_code, ExitCode::ValidationFailure);
        assert!(result.error.unwrap().contains("missing_var"));
    }

    #[tokio::test]
    async fn run_request_with_cli_vars_resolves() {
        let tmp = tempfile::tempdir().unwrap();
        let doc = simple_doc("{{base_url}}/api");
        let options = RunOptions {
            env_name: None,
            cli_vars: vec![("base_url".into(), "http://127.0.0.1:9".into())],
            validate_before_run: false,
            cwd: tmp.path().to_path_buf(),
        };

        let client = reqwest::Client::new();
        let result = run_request(&client, &doc, &options, CancellationToken::new()).await;
        // Will fail with network error since the URL is unreachable, but interpolation succeeded
        assert_eq!(result.exit_code, ExitCode::NetworkFailure);
    }

    #[tokio::test]
    async fn run_request_uses_request_file_parent_for_environment_resolution() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("service");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join(".env"), "BASE_URL=http://127.0.0.1:9\n").unwrap();

        let doc = RequestDocument {
            name: "Nested".into(),
            method: HttpMethod::Get,
            url: "{{BASE_URL}}/api".into(),
            headers: vec![],
            params: vec![],
            body: None,
            assertions: vec![],
            file_path: Some(nested.join("test.hurl.yml")),
        };

        let options = RunOptions {
            env_name: None,
            cli_vars: vec![],
            validate_before_run: true,
            cwd: std::env::temp_dir(),
        };

        let client = reqwest::Client::new();
        let result = run_request(&client, &doc, &options, CancellationToken::new()).await;
        assert_eq!(result.exit_code, ExitCode::NetworkFailure);
    }

    #[tokio::test]
    async fn run_request_surfaces_cancellation_structurally() {
        let tmp = tempfile::tempdir().unwrap();
        let doc = simple_doc("http://127.0.0.1:9");
        let options = default_options(tmp.path());
        let token = CancellationToken::new();
        token.cancel();

        let client = reqwest::Client::new();
        let result = run_request(&client, &doc, &options, token).await;
        assert!(result.cancelled);
        assert_eq!(result.exit_code, ExitCode::Interrupted);
        assert!(result.response.is_none());
    }
}
