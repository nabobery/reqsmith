/// Domain error types for hurl.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum HurlError {
    #[error("Terminal error: {0}")]
    Terminal(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Environment error: {0}")]
    Environment(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Formatter error: {0}")]
    Formatter(String),

    #[error("Repository error: {0}")]
    Repository(String),
}
