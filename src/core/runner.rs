use std::path::PathBuf;

use tokio_util::sync::CancellationToken;

use super::assertions;
use super::environment::{apply_os_env_fallback, resolve_environment};
use super::execution::{self, ExecutionError, ExecutionOptions};
use super::interpolation::{extract_variable_names, interpolate_document};
use super::models::{AssertionReport, ExitCode, RequestDocument, RunResult};
use super::validation::validate_document;

/// Options controlling how a request is run.
pub struct RunOptions {
    pub env_name: Option<String>,
    pub cli_vars: Vec<(String, String)>,
    pub validate_before_run: bool,
    pub cwd: PathBuf,
    /// Opt-in SSRF guard: reject requests to private/loopback hosts. Off by
    /// default so `localhost` (the primary local-API use case) keeps working.
    pub deny_private_networks: bool,
    #[cfg(feature = "plugins")]
    pub plugin_registry: Option<std::sync::Arc<crate::plugins::registry::PluginRegistry>>,
}

/// Run a request through the shared execution pipeline.
///
/// Used by both TUI and CLI to ensure identical behavior.
///
/// `client` must be built with
/// `infra::http_client::build_client(options.deny_private_networks)`: the
/// redirect-hop half of the private-network guard lives in the client's
/// redirect policy, not in this function, so a mismatched client silently
/// loses that half of the guard.
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

    // 2.5. Variable provider: resolve remaining unresolved vars via plugins.
    #[cfg(feature = "plugins")]
    if let Some(registry) = &options.plugin_registry {
        let still_missing: Vec<String> = referenced_vars
            .iter()
            .filter(|v| !env.values.contains_key(*v))
            .cloned()
            .collect();
        for var_name in still_missing {
            if let Some(value) =
                crate::plugins::hooks::provide_variable(registry, &var_name, &env.values).await
            {
                env.values.insert(var_name.clone(), value);
                env.sources
                    .insert(var_name, super::models::VarSource::Plugin);
            }
        }
    }

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

    let prepared_doc = match interpolate_document(doc, &env.values) {
        Ok(doc) => doc,
        Err(vars) => {
            return RunResult {
                request_name,
                request_file,
                response: None,
                error: Some(format!("Unresolved variables: {}", vars.join(", "))),
                exit_code: ExitCode::ValidationFailure,
                cancelled: false,
                assertions: AssertionReport::default(),
            };
        }
    };

    // 3.5. Pre-request mutation hook: plugins may modify the request.
    #[cfg(feature = "plugins")]
    let prepared_doc = {
        let mut prepared_doc = prepared_doc;

        if let Some(registry) = &options.plugin_registry {
            let ctx = crate::plugins::models::HookContext::from_document(
                &prepared_doc,
                options.env_name.as_deref(),
            );
            let mutated_ctx =
                crate::plugins::hooks::run_pre_request(registry, ctx, &env.values).await;
            let hook_result = crate::plugins::models::HookResult {
                method: Some(mutated_ctx.method),
                url: Some(mutated_ctx.url),
                headers: Some(mutated_ctx.headers),
                params: Some(mutated_ctx.params),
                body: mutated_ctx.body,
                error: None,
            };
            prepared_doc = hook_result.apply_to_document(&prepared_doc);
        }

        // 3.6. Authentication hook: plugins add auth headers.
        if let Some(registry) = &options.plugin_registry {
            let selected_auth_plugin =
                prepared_doc.auth_plugin.as_deref().and_then(|plugin_name| {
                    registry.plugin_named_with_capability(
                        plugin_name,
                        &crate::plugins::models::PluginCapability::Authenticate,
                    )
                });
            let auth_req = crate::plugins::models::AuthRequest {
                auth_type: prepared_doc.auth_plugin.clone().unwrap_or_default(),
                config: selected_auth_plugin
                    .map(|plugin| plugin.entry.flat_config())
                    .unwrap_or_default(),
                method: prepared_doc.method.to_string(),
                url: prepared_doc.url.clone(),
                headers: prepared_doc
                    .headers
                    .iter()
                    .filter(|h| h.enabled)
                    .map(|h| (h.key.clone(), h.value.clone()))
                    .collect(),
                body: prepared_doc.body.clone(),
            };
            if prepared_doc.auth_plugin.is_some() && selected_auth_plugin.is_none() {
                return RunResult {
                    request_name,
                    request_file,
                    response: None,
                    error: Some(format!(
                        "Auth plugin '{}' is not available",
                        prepared_doc.auth_plugin.as_deref().unwrap_or_default()
                    )),
                    exit_code: ExitCode::ValidationFailure,
                    cancelled: false,
                    assertions: AssertionReport::default(),
                };
            }
            if let Some(auth_result) =
                crate::plugins::hooks::run_authenticate(registry, auth_req, &env.values).await
            {
                let mut owned = prepared_doc.clone();
                for (key, value) in auth_result.headers {
                    owned.headers.push(super::models::KeyValueField {
                        key,
                        value,
                        enabled: true,
                    });
                }
                prepared_doc = owned;
            } else if let Some(auth_plugin) = &prepared_doc.auth_plugin {
                return RunResult {
                    request_name,
                    request_file,
                    response: None,
                    error: Some(format!(
                        "Auth plugin '{}' did not provide authentication headers",
                        auth_plugin
                    )),
                    exit_code: ExitCode::ValidationFailure,
                    cancelled: false,
                    assertions: AssertionReport::default(),
                };
            }
        } else if let Some(auth_plugin) = &prepared_doc.auth_plugin {
            return RunResult {
                request_name,
                request_file,
                response: None,
                error: Some(format!("Auth plugin '{}' is not available", auth_plugin)),
                exit_code: ExitCode::ValidationFailure,
                cancelled: false,
                assertions: AssertionReport::default(),
            };
        }

        prepared_doc
    };

    #[cfg(not(feature = "plugins"))]
    if let Some(auth_plugin) = &prepared_doc.auth_plugin {
        return RunResult {
            request_name,
            request_file,
            response: None,
            error: Some(format!(
                "Auth plugin '{}' requires a build with plugin support enabled",
                auth_plugin
            )),
            exit_code: ExitCode::ValidationFailure,
            cancelled: false,
            assertions: AssertionReport::default(),
        };
    }

    let exec_options = ExecutionOptions {
        deny_private_networks: options.deny_private_networks,
    };
    match execution::execute_request_with_options(
        client,
        &prepared_doc,
        &env.values,
        cancel,
        exec_options,
    )
    .await
    {
        Ok(artifact) => {
            // Post-response inspection hook.
            #[cfg(feature = "plugins")]
            let artifact = if let Some(registry) = &options.plugin_registry {
                let resp_ctx = crate::plugins::models::ResponseContext::from_artifact(&artifact);
                let mutated =
                    crate::plugins::hooks::run_post_response(registry, resp_ctx, &env.values).await;
                mutated.apply_to_artifact(artifact)
            } else {
                artifact
            };

            let assertion_report = if prepared_doc.assertions.is_empty() {
                AssertionReport::default()
            } else {
                assertions::evaluate_assertions(&prepared_doc.assertions, &artifact)
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
        // A request rejected before connecting (bad scheme, blocked by the
        // private-network policy, malformed after interpolation) is a
        // validation-class failure, not an internal error.
        Err(ExecutionError::InvalidRequest(msg)) => RunResult {
            request_name,
            request_file,
            response: None,
            error: Some(msg),
            exit_code: ExitCode::ValidationFailure,
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
            auth_plugin: None,
            assertions: vec![],
            file_path: Some(PathBuf::from("test.req.yml")),
        }
    }

    fn default_options(cwd: &std::path::Path) -> RunOptions {
        RunOptions {
            env_name: None,
            cli_vars: vec![],
            validate_before_run: false,
            cwd: cwd.to_path_buf(),
            deny_private_networks: false,
            #[cfg(feature = "plugins")]
            plugin_registry: None,
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
            auth_plugin: None,
            assertions: vec![],
            file_path: None,
        };

        let names = extract_variable_names_from_doc(&doc);
        assert_eq!(names, vec!["base_url", "body_content", "env_name", "token"]);
    }

    #[tokio::test]
    async fn run_request_env_resolution_failure() {
        let tmp = tempfile::tempdir().unwrap();
        // Create reqsmith_envs.yml without the named env to trigger error
        std::fs::write(
            tmp.path().join("reqsmith_envs.yml"),
            "environments:\n  prod:\n    X: y\n",
        )
        .unwrap();

        let doc = simple_doc("http://localhost");
        let options = RunOptions {
            env_name: Some("nonexistent".into()),
            cli_vars: vec![],
            validate_before_run: false,
            cwd: tmp.path().to_path_buf(),
            deny_private_networks: false,
            #[cfg(feature = "plugins")]
            plugin_registry: None,
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
            deny_private_networks: false,
            #[cfg(feature = "plugins")]
            plugin_registry: None,
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
            auth_plugin: None,
            assertions: vec![],
            file_path: Some(nested.join("test.req.yml")),
        };

        let options = RunOptions {
            env_name: None,
            cli_vars: vec![],
            validate_before_run: true,
            cwd: std::env::temp_dir(),
            deny_private_networks: false,
            #[cfg(feature = "plugins")]
            plugin_registry: None,
        };

        let client = reqwest::Client::new();
        let result = run_request(&client, &doc, &options, CancellationToken::new()).await;
        assert_eq!(result.exit_code, ExitCode::NetworkFailure);
    }

    #[tokio::test]
    async fn run_request_blocks_loopback_when_deny_private_networks_is_set() {
        // Proves the RunOptions flag is actually wired through to the
        // execution policy (not dead code): with it on, a loopback URL is
        // rejected as an invalid/blocked request before any connection.
        let tmp = tempfile::tempdir().unwrap();
        let doc = simple_doc("http://127.0.0.1:9");
        let options = RunOptions {
            env_name: None,
            cli_vars: vec![],
            validate_before_run: false,
            cwd: tmp.path().to_path_buf(),
            deny_private_networks: true,
            #[cfg(feature = "plugins")]
            plugin_registry: None,
        };

        // Per the contract documented on `run_request`: the client must be
        // built with the same `deny_private_networks` flag.
        let client = crate::infra::http_client::build_client(true).unwrap();
        let result = run_request(&client, &doc, &options, CancellationToken::new()).await;
        // Blocked by policy before connecting -> validation-class failure, not
        // a network error (which is what the same URL yields without the flag).
        assert_eq!(result.exit_code, ExitCode::ValidationFailure);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|m| m.contains("network policy")),
            "expected a policy-block error, got: {:?}",
            result.error
        );
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

    #[tokio::test]
    async fn run_request_requires_explicit_auth_plugin_to_be_available() {
        let tmp = tempfile::tempdir().unwrap();
        let mut doc = simple_doc("https://example.com/api");
        doc.auth_plugin = Some("aws-sigv4".into());
        let options = default_options(tmp.path());

        let client = reqwest::Client::new();
        let result = run_request(&client, &doc, &options, CancellationToken::new()).await;

        assert_eq!(result.exit_code, ExitCode::ValidationFailure);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|message| message.contains("aws-sigv4"))
        );
    }
}
