//! Secret redaction for response headers.
//!
//! Response headers can carry credentials the server handed back (e.g.
//! `Set-Cookie`, a refreshed `Authorization`/bearer token, a signed
//! `WWW-Authenticate` challenge). Every place that prints, diffs, displays,
//! or persists response headers must route them through [`redact_headers`]
//! first so secrets never reach stdout, a stored run snapshot, or a log
//! line.

use std::env;

/// Default set of header names considered sensitive. Matching is
/// case-insensitive (see [`is_sensitive_header`]).
const DEFAULT_SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "api-key",
    "x-auth-token",
    "x-amz-security-token",
    "www-authenticate",
    "authentication",
];

/// Name of the environment variable used to extend the default sensitive-header
/// set with additional, comma-separated header names.
pub const REDACT_HEADERS_ENV_VAR: &str = "REQSMITH_REDACT_HEADERS";

/// The literal placeholder written in place of a redacted header value.
const REDACTED_VALUE: &str = "<redacted>";

/// A resolved redaction policy: the default sensitive-header set plus any
/// extra names configured via `REQSMITH_REDACT_HEADERS`, parsed **once**.
///
/// This is the single source of truth every sink (stdout, JSON, diff, stored
/// snapshot, TUI) should consult. Build one with [`RedactionPolicy::from_env`]
/// at the start of an operation and reuse it, rather than calling the
/// convenience free functions in a per-header loop (each of which re-reads and
/// re-parses the environment variable).
#[derive(Debug, Clone, Default)]
pub struct RedactionPolicy {
    /// Extra sensitive header names from `REQSMITH_REDACT_HEADERS`, trimmed and
    /// non-empty. Matched case-insensitively, like the defaults.
    extra: Vec<String>,
}

impl RedactionPolicy {
    /// Build a policy, reading and parsing `REQSMITH_REDACT_HEADERS` exactly once.
    pub fn from_env() -> Self {
        let extra = env::var(REDACT_HEADERS_ENV_VAR)
            .ok()
            .map(|value| parse_extra_headers(&value))
            .unwrap_or_default();
        Self { extra }
    }

    /// Whether `name` is a sensitive header under this policy.
    pub fn is_sensitive(&self, name: &str) -> bool {
        DEFAULT_SENSITIVE_HEADERS
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(name))
            || self
                .extra
                .iter()
                .any(|entry| entry.eq_ignore_ascii_case(name))
    }

    /// Return a copy of `headers` with sensitive values replaced by the
    /// redaction placeholder. Names and ordering are preserved.
    pub fn redact_headers(&self, headers: &[(String, String)]) -> Vec<(String, String)> {
        headers
            .iter()
            .map(|(key, value)| (key.clone(), self.redact_value_for(key, value)))
            .collect()
    }

    /// The placeholder if `key` is sensitive, otherwise `value` unchanged.
    pub fn redact_value_for(&self, key: &str, value: &str) -> String {
        if self.is_sensitive(key) {
            REDACTED_VALUE.to_string()
        } else {
            value.to_string()
        }
    }
}

fn parse_extra_headers(env_value: &str) -> Vec<String> {
    env_value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

/// Returns the placeholder string used in place of a redacted header value.
// Part of the module's convenience API (and used in tests); most sinks go
// through `RedactionPolicy` / `redact_headers` instead.
#[allow(dead_code)]
pub fn redact_value() -> &'static str {
    REDACTED_VALUE
}

/// Returns `true` if `name` matches a default sensitive header, or one added
/// via the `REQSMITH_REDACT_HEADERS` environment variable.
///
/// Convenience wrapper over [`RedactionPolicy`]; it builds a policy (one env
/// read) per call. In a loop over many headers, build a [`RedactionPolicy`]
/// once and call [`RedactionPolicy::is_sensitive`] instead.
#[allow(dead_code)]
pub fn is_sensitive_header(name: &str) -> bool {
    RedactionPolicy::from_env().is_sensitive(name)
}

/// Pure matching logic, independent of the environment, so it can be unit
/// tested without racing other tests over a process-global env var.
#[cfg(test)]
fn is_sensitive_header_inner(name: &str, extra_env: Option<&str>) -> bool {
    RedactionPolicy {
        extra: extra_env.map(parse_extra_headers).unwrap_or_default(),
    }
    .is_sensitive(name)
}

/// Return a copy of `headers` with sensitive values replaced by
/// [`redact_value`]. Header names and ordering are preserved; non-sensitive
/// values pass through untouched. Reads the environment once (via
/// [`RedactionPolicy::from_env`]), not once per header.
pub fn redact_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    RedactionPolicy::from_env().redact_headers(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serializes the one test below that mutates the process-wide
    // REQSMITH_REDACT_HEADERS env var, so it can't race other tests in this file.
    static ENV_GUARD: Mutex<()> = Mutex::new(());

    #[test]
    fn detects_default_sensitive_headers_case_insensitively() {
        assert!(is_sensitive_header("Authorization"));
        assert!(is_sensitive_header("AUTHORIZATION"));
        assert!(is_sensitive_header("authorization"));
        assert!(is_sensitive_header("Set-Cookie"));
        assert!(is_sensitive_header("set-cookie"));
        assert!(is_sensitive_header("Proxy-Authorization"));
        assert!(is_sensitive_header("X-Api-Key"));
        assert!(is_sensitive_header("Api-Key"));
        assert!(is_sensitive_header("X-Auth-Token"));
        assert!(is_sensitive_header("X-Amz-Security-Token"));
        assert!(is_sensitive_header("WWW-Authenticate"));
        assert!(is_sensitive_header("Authentication"));
        assert!(is_sensitive_header("Cookie"));
        assert!(!is_sensitive_header("content-type"));
        assert!(!is_sensitive_header("x-request-id"));
    }

    #[test]
    fn env_extension_logic_merges_with_defaults() {
        assert!(is_sensitive_header_inner(
            "x-custom-secret",
            Some("x-custom-secret, x-other-secret")
        ));
        assert!(is_sensitive_header_inner(
            "X-OTHER-SECRET",
            Some("x-custom-secret, x-other-secret")
        ));
        assert!(!is_sensitive_header_inner(
            "x-unrelated",
            Some("x-custom-secret")
        ));
        // Defaults still apply even when an extension list is present.
        assert!(is_sensitive_header_inner(
            "authorization",
            Some("x-custom-secret")
        ));
        // No extension configured: only defaults match.
        assert!(!is_sensitive_header_inner("x-custom-secret", None));
    }

    #[test]
    fn env_var_extends_default_set_end_to_end() {
        let _guard = ENV_GUARD.lock().unwrap();
        let previous = env::var(REDACT_HEADERS_ENV_VAR).ok();

        // SAFETY: serialized by ENV_GUARD above; no other test in this
        // process reads or writes REQSMITH_REDACT_HEADERS.
        unsafe {
            env::set_var(REDACT_HEADERS_ENV_VAR, "x-internal-token, x-tenant-secret");
        }

        assert!(is_sensitive_header("x-internal-token"));
        assert!(is_sensitive_header("X-Tenant-Secret"));
        assert!(!is_sensitive_header("x-not-listed"));

        unsafe {
            match &previous {
                Some(val) => env::set_var(REDACT_HEADERS_ENV_VAR, val),
                None => env::remove_var(REDACT_HEADERS_ENV_VAR),
            }
        }
    }

    #[test]
    fn redact_value_returns_placeholder() {
        assert_eq!(redact_value(), "<redacted>");
    }

    #[test]
    fn redact_headers_masks_sensitive_values_only() {
        let headers = vec![
            ("Authorization".to_string(), "Bearer secret123".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Set-Cookie".to_string(), "session=abc123".to_string()),
        ];
        let redacted = redact_headers(&headers);
        assert_eq!(
            redacted[0],
            ("Authorization".to_string(), "<redacted>".to_string())
        );
        assert_eq!(
            redacted[1],
            ("Content-Type".to_string(), "application/json".to_string())
        );
        assert_eq!(
            redacted[2],
            ("Set-Cookie".to_string(), "<redacted>".to_string())
        );
    }

    #[test]
    fn redaction_policy_default_matches_defaults_and_extras() {
        let default_policy = RedactionPolicy::default();
        assert!(default_policy.is_sensitive("Authorization"));
        assert!(!default_policy.is_sensitive("x-custom-secret"));

        let extended = RedactionPolicy {
            extra: parse_extra_headers("x-custom-secret, x-other"),
        };
        assert!(extended.is_sensitive("X-Custom-Secret"));
        assert!(extended.is_sensitive("x-other"));
        // Defaults still apply through a policy with extras.
        assert!(extended.is_sensitive("set-cookie"));
        assert!(!extended.is_sensitive("content-type"));

        let redacted = extended.redact_headers(&[
            ("X-Custom-Secret".into(), "shhh".into()),
            ("Content-Type".into(), "text/plain".into()),
        ]);
        assert_eq!(redacted[0].1, "<redacted>");
        assert_eq!(redacted[1].1, "text/plain");
    }

    #[test]
    fn redact_headers_preserves_order_names_and_length() {
        let headers = vec![
            ("X-Foo".to_string(), "bar".to_string()),
            ("X-Baz".to_string(), "qux".to_string()),
        ];
        let redacted = redact_headers(&headers);
        assert_eq!(redacted, headers);
        assert_eq!(redacted.len(), headers.len());
    }
}
