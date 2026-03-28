use std::time::Duration;

/// Build a reusable reqwest HTTP client with sensible defaults.
#[allow(dead_code)] // Used in Step 5.
pub fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(format!("hurl/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("Failed to build HTTP client")
}
