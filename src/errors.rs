/// Domain error types for hurl. Intentionally minimal in Phase 0;
/// Phase 1 adds request, execution, and storage errors.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)] // Error variants will be used in later phases.
pub enum HurlError {
    #[error("Terminal error: {0}")]
    Terminal(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),
}
