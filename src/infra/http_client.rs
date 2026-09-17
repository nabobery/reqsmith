use std::time::Duration;

/// Maximum number of redirects followed in a single request chain.
pub(crate) const MAX_REDIRECTS: usize = 10;

/// Total request timeout applied to the shared client. Referenced from
/// `core::execution` so the number quoted in error messages can't drift from
/// what the client actually enforces.
pub(crate) const TOTAL_TIMEOUT_SECS: u64 = 30;

/// Connect timeout applied to the shared client; see [`TOTAL_TIMEOUT_SECS`].
pub(crate) const CONNECT_TIMEOUT_SECS: u64 = 10;

/// Build a reusable reqwest HTTP client with sensible defaults.
///
/// # Timeouts
/// A 30s total request timeout and a separate 10s connect timeout are set so a
/// stalled TCP handshake (e.g. a firewalled or non-responsive host) fails fast
/// instead of tying up the total timeout budget.
///
/// # Redirect policy
/// Automatic redirects are disabled. `core::execution` follows them explicitly
/// so redirect DNS checks are asynchronous, cancellable, and covered by one
/// total timeout instead of blocking inside reqwest's synchronous policy hook.
///
/// Returns an error instead of panicking if the underlying TLS backend can't
/// be initialized; callers must handle the `Result`.
pub fn build_client(deny_private_networks: bool) -> reqwest::Result<reqwest::Client> {
    let builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(TOTAL_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("reqsmith/{}", env!("CARGO_PKG_VERSION")));

    let builder = if deny_private_networks {
        // A proxy dials the target for us, so the addresses we just vetted
        // would not be the ones actually connected to.
        builder.no_proxy()
    } else {
        builder
    };

    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_client_succeeds_with_default_settings() {
        assert!(build_client(false).is_ok());
    }
}
