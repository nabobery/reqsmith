//! Secret redaction for response headers.
//!
//! Response headers can carry credentials the server handed back (e.g.
//! `Set-Cookie`, a refreshed `Authorization`/bearer token, a signed
//! `WWW-Authenticate` challenge). Every place that prints, diffs, displays,
//! or persists response headers must route them through [`redact_headers`]
//! first so secrets never reach stdout, a stored run snapshot, or a log
//! line.

use std::borrow::Cow;
use std::env;

use sha2::{Digest, Sha256};

/// Default set of header names considered sensitive. Matching is
/// case-insensitive (see [`RedactionPolicy::is_sensitive`]).
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
    "x-csrf-token",
    "x-xsrf-token",
    "x-amz-signature",
    "private-token",
    "x-hub-signature",
    // Config-style key names, so the same policy can mask secrets in
    // key/value configuration as well as in HTTP headers.
    "secret",
    "token",
    "password",
    "client_secret",
];

/// Name of the environment variable used to extend the default sensitive-header
/// set with additional, comma-separated header names.
pub const REDACT_HEADERS_ENV_VAR: &str = "REQSMITH_REDACT_HEADERS";

/// Prefix every redacted value starts with, so callers, tests and docs can
/// recognize a placeholder without depending on the digest that follows.
pub const REDACTED_PREFIX: &str = "<redacted:";

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
            .map(|(key, value)| (key.clone(), self.redact_value_for(key, value).into_owned()))
            .collect()
    }

    /// The placeholder if `key` is sensitive, otherwise `value` borrowed unchanged.
    pub fn redact_value_for<'a>(&self, key: &str, value: &'a str) -> Cow<'a, str> {
        if !self.is_sensitive(key) {
            return Cow::Borrowed(value);
        }
        // Already a placeholder (e.g. loaded from a stored snapshot): redaction
        // must be idempotent, or re-hashing it on every diff/display would make
        // `reqsmith diff` show a fingerprint that doesn't match what's on disk.
        if value.starts_with(REDACTED_PREFIX) {
            return Cow::Borrowed(value);
        }
        Cow::Owned(placeholder_for(value))
    }
}

/// The placeholder for `value`: a truncated SHA-256 fingerprint rather than a
/// flat marker, so two different secrets get two different placeholders and a
/// rotated secret still shows up as *changed* when two stored snapshots (which
/// only ever hold the placeholder) are diffed.
///
/// The digest is unsalted and truncated to 4 bytes, so it is a fingerprint,
/// not a blindfold: a low-entropy value (e.g. `Authorization: Basic
/// dXNlcjpwYXNz`, a short PIN, a value drawn from a small known set) can be
/// confirmed offline by hashing candidates and comparing against the
/// published hex characters. See `docs/security-model.md`.
fn placeholder_for(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let fingerprint: String = digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{REDACTED_PREFIX}sha256:{fingerprint}>")
}

fn parse_extra_headers(env_value: &str) -> Vec<String> {
    env_value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

/// Return a copy of `headers` with sensitive values replaced by a redaction
/// placeholder. Header names and ordering are preserved; non-sensitive values
/// pass through untouched. Reads the environment once (via
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

    fn policy_with(extra: &str) -> RedactionPolicy {
        RedactionPolicy {
            extra: parse_extra_headers(extra),
        }
    }

    #[test]
    fn detects_default_sensitive_headers_case_insensitively() {
        let policy = RedactionPolicy::default();
        for name in [
            "Authorization",
            "AUTHORIZATION",
            "authorization",
            "Set-Cookie",
            "set-cookie",
            "Proxy-Authorization",
            "X-Api-Key",
            "Api-Key",
            "X-Auth-Token",
            "X-Amz-Security-Token",
            "WWW-Authenticate",
            "Authentication",
            "Cookie",
            "X-Csrf-Token",
            "x-xsrf-token",
            "X-Amz-Signature",
            "private-token",
            "X-Hub-Signature",
            "secret",
            "TOKEN",
            "password",
            "client_secret",
        ] {
            assert!(policy.is_sensitive(name), "{name} should be sensitive");
        }
        assert!(!policy.is_sensitive("content-type"));
        assert!(!policy.is_sensitive("x-request-id"));
    }

    #[test]
    fn env_extension_logic_merges_with_defaults() {
        let extended = policy_with("x-custom-secret, x-other-secret");
        assert!(extended.is_sensitive("x-custom-secret"));
        assert!(extended.is_sensitive("X-OTHER-SECRET"));
        assert!(!extended.is_sensitive("x-unrelated"));
        // Defaults still apply even when an extension list is present.
        assert!(extended.is_sensitive("authorization"));
        // No extension configured: only defaults match.
        assert!(!RedactionPolicy::default().is_sensitive("x-custom-secret"));
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

        let policy = RedactionPolicy::from_env();
        assert!(policy.is_sensitive("x-internal-token"));
        assert!(policy.is_sensitive("X-Tenant-Secret"));
        assert!(!policy.is_sensitive("x-not-listed"));

        unsafe {
            match &previous {
                Some(val) => env::set_var(REDACT_HEADERS_ENV_VAR, val),
                None => env::remove_var(REDACT_HEADERS_ENV_VAR),
            }
        }
    }

    #[test]
    fn placeholder_is_a_truncated_digest_and_hides_the_value() {
        let placeholder = placeholder_for("Bearer super-secret");
        assert!(placeholder.starts_with(REDACTED_PREFIX));
        assert!(!placeholder.contains("super-secret"));
        assert_eq!(placeholder.len(), "<redacted:sha256:>".len() + 8);
        // sha256("Bearer super-secret") starts with these bytes.
        let expected = {
            let digest = Sha256::digest(b"Bearer super-secret");
            format!(
                "<redacted:sha256:{:02x}{:02x}{:02x}{:02x}>",
                digest[0], digest[1], digest[2], digest[3]
            )
        };
        assert_eq!(placeholder, expected);
    }

    #[test]
    fn distinct_values_get_distinct_placeholders_and_equal_values_match() {
        let policy = RedactionPolicy::default();
        let old = policy.redact_value_for("set-cookie", "session=old-secret");
        let new = policy.redact_value_for("set-cookie", "session=new-secret");
        let again = policy.redact_value_for("set-cookie", "session=old-secret");

        assert_ne!(old, new, "a rotated secret must change its placeholder");
        assert_eq!(old, again, "the same secret must be stable across calls");
        for rendered in [&old, &new] {
            assert!(rendered.starts_with(REDACTED_PREFIX));
            assert!(!rendered.contains("old-secret"));
            assert!(!rendered.contains("new-secret"));
        }
    }

    #[test]
    fn non_sensitive_values_are_borrowed_not_copied() {
        let policy = RedactionPolicy::default();
        assert!(matches!(
            policy.redact_value_for("content-type", "application/json"),
            Cow::Borrowed("application/json")
        ));
        assert!(matches!(
            policy.redact_value_for("authorization", "Bearer x"),
            Cow::Owned(_)
        ));
    }

    #[test]
    fn redact_headers_masks_sensitive_values_only() {
        let headers = vec![
            ("Authorization".to_string(), "Bearer secret123".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Set-Cookie".to_string(), "session=abc123".to_string()),
        ];
        let redacted = redact_headers(&headers);

        assert_eq!(redacted[0].0, "Authorization");
        assert!(redacted[0].1.starts_with(REDACTED_PREFIX));
        assert_eq!(
            redacted[1],
            ("Content-Type".to_string(), "application/json".to_string())
        );
        assert_eq!(redacted[2].0, "Set-Cookie");
        assert!(redacted[2].1.starts_with(REDACTED_PREFIX));

        let rendered = format!("{redacted:?}");
        assert!(!rendered.contains("secret123"));
        assert!(!rendered.contains("abc123"));
    }

    #[test]
    fn redaction_policy_default_matches_defaults_and_extras() {
        let default_policy = RedactionPolicy::default();
        assert!(default_policy.is_sensitive("Authorization"));
        assert!(!default_policy.is_sensitive("x-custom-secret"));

        let extended = policy_with("x-custom-secret, x-other");
        assert!(extended.is_sensitive("X-Custom-Secret"));
        assert!(extended.is_sensitive("x-other"));
        // Defaults still apply through a policy with extras.
        assert!(extended.is_sensitive("set-cookie"));
        assert!(!extended.is_sensitive("content-type"));

        let redacted = extended.redact_headers(&[
            ("X-Custom-Secret".into(), "shhh".into()),
            ("Content-Type".into(), "text/plain".into()),
        ]);
        assert!(redacted[0].1.starts_with(REDACTED_PREFIX));
        assert!(!redacted[0].1.contains("shhh"));
        assert_eq!(redacted[1].1, "text/plain");
    }

    #[test]
    fn redacting_an_already_redacted_value_is_a_no_op() {
        let policy = RedactionPolicy::default();
        let once = policy.redact_value_for("authorization", "Bearer secret");
        let twice = policy.redact_value_for("authorization", &once);
        assert_eq!(
            once, twice,
            "re-redacting a placeholder must not re-hash it"
        );
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
