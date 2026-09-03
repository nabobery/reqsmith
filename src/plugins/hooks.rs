use super::models::{
    AuthRequest, AuthResult, HookContext, HookResult, PluginCapability, ResponseContext,
    VariableRequest, VariableResult,
};
use super::registry::PluginRegistry;
use std::collections::HashMap;

/// Run all pre-request hooks in order, chaining mutations.
///
/// Each plugin's output becomes the next plugin's input. If a plugin
/// returns an error or fails, it is logged and skipped.
pub async fn run_pre_request(
    registry: &PluginRegistry,
    mut ctx: HookContext,
    env_values: &HashMap<String, String>,
) -> HookContext {
    for plugin in registry.plugins_with_capability(&PluginCapability::PreRequest) {
        if !plugin.has_function("pre_request") {
            continue;
        }
        let input = match serde_json::to_string(&ctx) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(plugin = %plugin.entry.name, "Failed to serialize hook input: {e}");
                continue;
            }
        };
        match plugin.call("pre_request", input, env_values.clone()).await {
            Ok(output) => match serde_json::from_str::<HookResult>(&output) {
                Ok(result) => {
                    if let Some(ref err) = result.error {
                        tracing::warn!(
                            plugin = %plugin.entry.name,
                            "pre_request hook returned error: {err}"
                        );
                        continue;
                    }
                    ctx = apply_hook_result(ctx, &result);
                }
                Err(e) => {
                    tracing::warn!(
                        plugin = %plugin.entry.name,
                        "Failed to parse hook output: {e}"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(plugin = %plugin.entry.name, "pre_request call failed: {e}");
            }
        }
    }
    ctx
}

/// Run all post-response hooks in order, chaining mutations.
pub async fn run_post_response(
    registry: &PluginRegistry,
    mut ctx: ResponseContext,
    env_values: &HashMap<String, String>,
) -> ResponseContext {
    for plugin in registry.plugins_with_capability(&PluginCapability::PostResponse) {
        if !plugin.has_function("post_response") {
            continue;
        }
        let input = match serde_json::to_string(&ctx) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(plugin = %plugin.entry.name, "Failed to serialize response context: {e}");
                continue;
            }
        };
        match plugin
            .call("post_response", input, env_values.clone())
            .await
        {
            Ok(output) => match serde_json::from_str::<ResponseContext>(&output) {
                Ok(result) => {
                    ctx = result;
                }
                Err(e) => {
                    tracing::warn!(
                        plugin = %plugin.entry.name,
                        "Failed to parse post_response output: {e}"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(plugin = %plugin.entry.name, "post_response call failed: {e}");
            }
        }
    }
    ctx
}

/// Run authenticate hooks. Returns the first successful auth result.
pub async fn run_authenticate(
    registry: &PluginRegistry,
    req: AuthRequest,
    env_values: &HashMap<String, String>,
) -> Option<AuthResult> {
    if !req.auth_type.is_empty() {
        let plugin = registry
            .plugin_named_with_capability(&req.auth_type, &PluginCapability::Authenticate)?;
        return authenticate_with_plugin(plugin, &req, env_values, true).await;
    }

    for plugin in registry.plugins_with_capability(&PluginCapability::Authenticate) {
        if let Some(result) = authenticate_with_plugin(plugin, &req, env_values, false).await {
            return Some(result);
        }
    }

    None
}

async fn authenticate_with_plugin(
    plugin: &super::registry::LoadedPlugin,
    req: &AuthRequest,
    env_values: &HashMap<String, String>,
    required: bool,
) -> Option<AuthResult> {
    if !plugin.has_function("authenticate") {
        if required {
            tracing::warn!(
                plugin = %plugin.entry.name,
                "authenticate hook missing required export"
            );
        }
        return None;
    }

    let input = match serde_json::to_string(req) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(plugin = %plugin.entry.name, "Failed to serialize auth request: {e}");
            return None;
        }
    };

    match plugin.call("authenticate", input, env_values.clone()).await {
        Ok(output) => match serde_json::from_str::<AuthResult>(&output) {
            Ok(result) => {
                if let Some(ref err) = result.error {
                    tracing::warn!(
                        plugin = %plugin.entry.name,
                        "authenticate hook returned error: {err}"
                    );
                    return None;
                }
                Some(result)
            }
            Err(e) => {
                tracing::warn!(
                    plugin = %plugin.entry.name,
                    "Failed to parse auth output: {e}"
                );
                None
            }
        },
        Err(e) => {
            tracing::warn!(plugin = %plugin.entry.name, "authenticate call failed: {e}");
            None
        }
    }
}

/// Ask variable provider plugins for a variable value.
/// Returns the first non-empty value.
pub async fn provide_variable(
    registry: &PluginRegistry,
    name: &str,
    env_values: &HashMap<String, String>,
) -> Option<String> {
    for plugin in registry.plugins_with_capability(&PluginCapability::ProvideVariable) {
        if !plugin.has_function("provide_variable") {
            continue;
        }
        let req = VariableRequest {
            name: name.to_string(),
            config: plugin.entry.flat_config(),
        };
        let input = match serde_json::to_string(&req) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(plugin = %plugin.entry.name, "Failed to serialize variable request: {e}");
                continue;
            }
        };
        match plugin
            .call("provide_variable", input, env_values.clone())
            .await
        {
            Ok(output) => match serde_json::from_str::<VariableResult>(&output) {
                Ok(result) => {
                    if let Some(ref err) = result.error {
                        tracing::warn!(
                            plugin = %plugin.entry.name,
                            "provide_variable hook returned error: {err}"
                        );
                        continue;
                    }
                    if let Some(value) = result.value
                        && !value.is_empty()
                    {
                        return Some(value);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        plugin = %plugin.entry.name,
                        "Failed to parse variable output: {e}"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(plugin = %plugin.entry.name, "provide_variable call failed: {e}");
            }
        }
    }
    None
}

/// Apply a `HookResult`'s optional fields onto a `HookContext`.
fn apply_hook_result(mut ctx: HookContext, result: &HookResult) -> HookContext {
    if let Some(ref method) = result.method {
        ctx.method = method.clone();
    }
    if let Some(ref url) = result.url {
        ctx.url = url.clone();
    }
    if let Some(ref headers) = result.headers {
        ctx.headers = headers.clone();
    }
    if let Some(ref params) = result.params {
        ctx.params = params.clone();
    }
    if let Some(ref body) = result.body {
        ctx.body = Some(body.clone());
    }
    ctx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_hook_result_partial_update() {
        let ctx = HookContext {
            method: "GET".into(),
            url: "https://example.com".into(),
            headers: vec![("Accept".into(), "text/html".into())],
            params: vec![],
            body: None,
            env_name: None,
        };
        let result = HookResult {
            url: Some("https://modified.com".into()),
            ..Default::default()
        };
        let updated = apply_hook_result(ctx, &result);
        assert_eq!(updated.url, "https://modified.com");
        assert_eq!(updated.method, "GET"); // unchanged
        assert_eq!(updated.headers.len(), 1); // unchanged
    }

    #[test]
    fn apply_hook_result_full_update() {
        let ctx = HookContext {
            method: "GET".into(),
            url: "https://example.com".into(),
            headers: vec![],
            params: vec![],
            body: None,
            env_name: None,
        };
        let result = HookResult {
            method: Some("POST".into()),
            url: Some("https://new.com".into()),
            headers: Some(vec![("X-Custom".into(), "val".into())]),
            params: Some(vec![("q".into(), "search".into())]),
            body: Some("body content".into()),
            error: None,
        };
        let updated = apply_hook_result(ctx, &result);
        assert_eq!(updated.method, "POST");
        assert_eq!(updated.url, "https://new.com");
        assert_eq!(updated.headers.len(), 1);
        assert_eq!(updated.params.len(), 1);
        assert_eq!(updated.body.as_deref(), Some("body content"));
    }

    #[tokio::test]
    async fn run_pre_request_with_empty_registry_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        // avoid discover_and_load(), which reads the real config dir/env var.
        let registry = PluginRegistry::discover_and_load_with(tmp.path(), None, false);
        let ctx = HookContext {
            method: "GET".into(),
            url: "https://example.com".into(),
            headers: vec![],
            params: vec![],
            body: None,
            env_name: None,
        };
        let result = run_pre_request(&registry, ctx.clone(), &HashMap::new()).await;
        assert_eq!(result.method, ctx.method);
        assert_eq!(result.url, ctx.url);
    }

    #[tokio::test]
    async fn run_post_response_with_empty_registry_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        // avoid discover_and_load(), which reads the real config dir/env var.
        let registry = PluginRegistry::discover_and_load_with(tmp.path(), None, false);
        let ctx = ResponseContext {
            status_code: 200,
            headers: vec![],
            body_text: Some("hello".into()),
            duration_ms: 42,
            content_type: None,
        };
        let result = run_post_response(&registry, ctx.clone(), &HashMap::new()).await;
        assert_eq!(result.status_code, 200);
        assert_eq!(result.body_text.as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn run_authenticate_with_empty_registry_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        // avoid discover_and_load(), which reads the real config dir/env var.
        let registry = PluginRegistry::discover_and_load_with(tmp.path(), None, false);
        let req = AuthRequest {
            auth_type: "bearer".into(),
            config: Default::default(),
            method: "GET".into(),
            url: "https://example.com".into(),
            headers: vec![],
            body: None,
        };
        assert!(
            run_authenticate(&registry, req, &HashMap::new())
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn provide_variable_with_empty_registry_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        // avoid discover_and_load(), which reads the real config dir/env var.
        let registry = PluginRegistry::discover_and_load_with(tmp.path(), None, false);
        assert!(
            provide_variable(&registry, "API_KEY", &HashMap::new())
                .await
                .is_none()
        );
    }
}
