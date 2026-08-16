use std::time::Duration;

/// Build a reusable reqwest HTTP client with sensible defaults.
///
/// # Timeouts
/// A 30s total request timeout and a separate 10s connect timeout are set so a
/// stalled TCP handshake (e.g. a firewalled or non-responsive host) fails fast
/// instead of tying up the total timeout budget.
///
/// # Redirect policy
/// Redirects are followed up to a limit of 10 hops
/// (`redirect::Policy::limited(10)`). reqwest automatically strips the
/// `Authorization`, `Cookie`, and `Proxy-Authorization` request headers
/// whenever a redirect crosses a scheme, host, or port boundary, so
/// credentials set on the original request are never forwarded to a
/// different origin. This is a property of reqwest's redirect handling, not
/// something reqsmith implements itself; see the
/// `redirect_strips_authorization_header_across_port_change` regression test
/// in `src/core/execution.rs` which locks in this guarantee against a pair of
/// local TCP listeners.
///
/// Returns an error instead of panicking if the underlying TLS backend can't
/// be initialized; callers must handle the `Result`.
pub fn build_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(format!("reqsmith/{}", env!("CARGO_PKG_VERSION")))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_client_succeeds_with_default_settings() {
        assert!(build_client().is_ok());
    }
}
