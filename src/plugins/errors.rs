/// Plugin-specific error types.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum PluginError {
    #[error("Plugin '{name}' failed: {message}")]
    ExecutionFailed { name: String, message: String },

    #[error("Plugin '{name}' timed out after {timeout_ms}ms")]
    Timeout { name: String, timeout_ms: u64 },

    #[error("Plugin manifest error: {0}")]
    ManifestError(String),

    #[error("Plugin WASM load failed for '{name}': {message}")]
    LoadFailed { name: String, message: String },

    #[error("Unsupported plugin API version {version} (supported: {supported})")]
    UnsupportedApiVersion { version: u32, supported: u32 },

    #[error("Plugin '{name}' lacks required capability: {capability}")]
    MissingCapability { name: String, capability: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_are_descriptive() {
        let err = PluginError::ExecutionFailed {
            name: "auth-plugin".into(),
            message: "segfault".into(),
        };
        assert_eq!(err.to_string(), "Plugin 'auth-plugin' failed: segfault");

        let err = PluginError::Timeout {
            name: "slow".into(),
            timeout_ms: 5000,
        };
        assert_eq!(err.to_string(), "Plugin 'slow' timed out after 5000ms");

        let err = PluginError::UnsupportedApiVersion {
            version: 99,
            supported: 1,
        };
        assert!(err.to_string().contains("99"));
        assert!(err.to_string().contains("1"));

        let err = PluginError::LoadFailed {
            name: "bad".into(),
            message: "file not found".into(),
        };
        assert!(err.to_string().contains("bad"));
        assert!(err.to_string().contains("file not found"));

        let err = PluginError::MissingCapability {
            name: "my-plugin".into(),
            capability: "pre_request".into(),
        };
        assert!(err.to_string().contains("pre_request"));
    }
}
