use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use super::interpolation::interpolate_document;
use super::models::{HttpMethod, RequestDocument, ResponseArtifact};

/// Maximum response body size to read into memory (10 MB).
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

/// Policy knobs for [`execute_request_with_options`].
///
/// Defaults preserve the pre-hardening behavior: private/loopback addresses
/// are allowed. reqsmith is a local, user-driven tool — sending requests to
/// `localhost`, `127.0.0.1`, or a machine on the private LAN is a primary,
/// legitimate use case, so that must keep working out of the box.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecutionOptions {
    /// When `true`, reject requests whose host is a loopback, RFC1918
    /// private, link-local, or IPv6 unique-local address, or that resolves
    /// (via DNS) only to such addresses. Opt-in; default is `false`.
    pub deny_private_networks: bool,
}

/// Heuristic: treat a response as binary if >5% of the first 8 KiB are null bytes.
fn is_likely_binary(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(8192)];
    let null_count = sample.iter().filter(|&&b| b == 0).count();
    null_count > sample.len() / 20 // >5% null bytes
}

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum ExecutionError {
    #[error("Request cancelled")]
    Cancelled,
    #[error("Network error: {0}")]
    Network(String),
    #[error("Interpolation failed: missing variables {0:?}")]
    Interpolation(Vec<String>),
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
}

/// Execute an HTTP request built from a `RequestDocument` with environment
/// variable interpolation. Supports cancellation via `CancellationToken`.
///
/// Uses default [`ExecutionOptions`] (private/loopback addresses allowed).
/// Use [`execute_request_with_options`] to opt in to private-network denial.
#[allow(dead_code)] // Used in Step 5.
pub async fn execute_request(
    client: &reqwest::Client,
    doc: &RequestDocument,
    vars: &HashMap<String, String>,
    cancel: CancellationToken,
) -> Result<ResponseArtifact, ExecutionError> {
    execute_request_with_options(client, doc, vars, cancel, ExecutionOptions::default()).await
}

/// Same as [`execute_request`], with explicit network policy options.
pub async fn execute_request_with_options(
    client: &reqwest::Client,
    doc: &RequestDocument,
    vars: &HashMap<String, String>,
    cancel: CancellationToken,
    options: ExecutionOptions,
) -> Result<ResponseArtifact, ExecutionError> {
    // 1. Interpolate variables.
    let resolved = interpolate_document(doc, vars).map_err(ExecutionError::Interpolation)?;

    // 1.5. Parse and validate the URL: scheme allowlist (always enforced) and
    // optional private-network denial (opt-in via `options`).
    let parsed_url = reqwest::Url::parse(&resolved.url).map_err(|e| {
        ExecutionError::InvalidRequest(format!("Invalid URL '{}': {e}", resolved.url))
    })?;
    validate_scheme(&parsed_url)?;
    if options.deny_private_networks {
        check_private_network_policy(&parsed_url).await?;
    }

    // 2. Build the request.
    let method = to_reqwest_method(resolved.method);
    let mut builder = client.request(method, parsed_url);

    // Add query params.
    for param in &resolved.params {
        if param.enabled {
            builder = builder.query(&[(&param.key, &param.value)]);
        }
    }

    // Add headers.
    for header in &resolved.headers {
        if header.enabled {
            builder = builder.header(&header.key, &header.value);
        }
    }

    // Add body.
    if let Some(body) = &resolved.body {
        builder = builder.body(body.clone());
    }

    let request = builder
        .build()
        .map_err(|e| ExecutionError::InvalidRequest(e.to_string()))?;

    // 3. Execute with cancellation support.
    let start = Instant::now();

    let mut response = tokio::select! {
        result = client.execute(request) => {
            result.map_err(|e| ExecutionError::Network(e.to_string()))?
        }
        () = cancel.cancelled() => {
            return Err(ExecutionError::Cancelled);
        }
    };

    let duration_ms = start.elapsed().as_millis();

    // 4. Extract response data.
    let status_code = response.status().as_u16();
    let http_version = format!("{:?}", response.version());

    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
        .collect();

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let content_length = response.content_length();

    let mut body_bytes =
        Vec::with_capacity(content_length.unwrap_or(0).min(MAX_BODY_SIZE as u64) as usize);
    let mut truncated = false;

    loop {
        let next_chunk = tokio::select! {
            chunk = response.chunk() => {
                chunk.map_err(|e| ExecutionError::Network(e.to_string()))?
            }
            () = cancel.cancelled() => {
                return Err(ExecutionError::Cancelled);
            }
        };

        let Some(chunk) = next_chunk else {
            break;
        };

        let remaining = MAX_BODY_SIZE.saturating_sub(body_bytes.len());
        if remaining == 0 {
            truncated = true;
            break;
        }

        if chunk.len() > remaining {
            body_bytes.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }

        body_bytes.extend_from_slice(&chunk);
    }

    let binary = is_likely_binary(&body_bytes);

    let body_text = if binary {
        let len = body_bytes.len();
        Some(format!(
            "[Binary response: {len} bytes. Press 'w' to save to disk.]"
        ))
    } else if truncated {
        let truncated = String::from_utf8_lossy(&body_bytes);
        Some(format!(
            "{truncated}\n\n[truncated at {MAX_BODY_SIZE} bytes]"
        ))
    } else {
        let raw = String::from_utf8_lossy(&body_bytes).into_owned();
        // Pretty-print JSON if applicable.
        if content_type
            .as_deref()
            .is_some_and(|ct| ct.contains("application/json"))
        {
            match serde_json::from_str::<serde_json::Value>(&raw) {
                Ok(json) => serde_json::to_string_pretty(&json).ok().or(Some(raw)),
                Err(_) => Some(raw),
            }
        } else {
            Some(raw)
        }
    };

    Ok(ResponseArtifact {
        status_code,
        http_version,
        headers,
        content_type,
        content_length,
        duration_ms,
        body_text,
        body_bytes: binary.then_some(body_bytes),
        is_binary: binary,
    })
}

/// Reject any URL scheme other than `http`/`https`. Applies to every
/// request regardless of `ExecutionOptions` — this is not opt-in.
fn validate_scheme(url: &reqwest::Url) -> Result<(), ExecutionError> {
    match url.scheme() {
        "http" | "https" => Ok(()),
        other => Err(ExecutionError::InvalidRequest(format!(
            "Unsupported URL scheme '{other}'; only http and https are allowed"
        ))),
    }
}

/// Reject requests whose host is (or resolves to) a private/loopback address,
/// per [`ExecutionOptions::deny_private_networks`].
///
/// This guard **fails closed**: it rejects if *any* resolved address is
/// private (not only if all are — a single private A/AAAA record among public
/// ones is enough to reach an internal service), and it rejects if the host
/// cannot be resolved at all while the guard is enabled, rather than letting
/// an unchecked connection proceed.
///
/// KNOWN LIMITATION (DNS rebinding / TOCTOU): reqwest re-resolves the hostname
/// when it actually connects, so a name that resolves to a public address here
/// could resolve to a private one microseconds later at connect time. Fully
/// closing that gap requires pinning the checked IPs into the connection
/// (a custom resolver on a per-request client), which reqsmith does not yet do.
/// This check meaningfully raises the bar (static private targets, obvious
/// mixed records) but is not a complete SSRF defense on its own; see
/// `docs/security-model.md`.
async fn check_private_network_policy(url: &reqwest::Url) -> Result<(), ExecutionError> {
    let host = url
        .host_str()
        .ok_or_else(|| ExecutionError::InvalidRequest("URL is missing a host".into()))?;

    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_private_or_local(&ip) {
            Err(private_network_error(host))
        } else {
            Ok(())
        };
    }

    let port = url.port_or_known_default().unwrap_or(80);
    let ips: Vec<IpAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| {
            // Fail closed: with the guard on, an unresolvable host is rejected
            // rather than handed to reqwest to resolve (and possibly connect)
            // without a policy check.
            ExecutionError::InvalidRequest(format!(
                "Blocked '{host}' by network policy: could not resolve host to \
                 verify it is not private ({e})"
            ))
        })?
        .map(|addr| addr.ip())
        .collect();

    evaluate_resolved_ips(host, &ips)
}

/// Policy decision over a host's resolved addresses, factored out of DNS so it
/// is directly unit-testable. Rejects when resolution produced no addresses
/// (fail closed) or when ANY address is private/loopback.
fn evaluate_resolved_ips(host: &str, ips: &[IpAddr]) -> Result<(), ExecutionError> {
    if ips.is_empty() {
        return Err(ExecutionError::InvalidRequest(format!(
            "Blocked '{host}' by network policy: host resolved to no addresses"
        )));
    }
    if ips.iter().any(is_private_or_local) {
        return Err(private_network_error(host));
    }
    Ok(())
}

fn private_network_error(host: &str) -> ExecutionError {
    ExecutionError::InvalidRequest(format!(
        "Request to private/loopback host '{host}' blocked by network policy \
         (deny_private_networks is enabled)"
    ))
}

/// Classify an address as loopback, RFC1918 private, link-local, or IPv6
/// unique-local (`fc00::/7`).
fn is_private_or_local(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_unspecified() || v4.is_loopback() || v4.is_private() || v4.is_link_local()
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_or_local(&IpAddr::V4(v4));
            }
            let o = v6.octets();
            v6.is_unspecified()
                || v6.is_loopback()
                || (o[0] & 0xfe) == 0xfc // unique-local fc00::/7
                || (o[0] == 0xfe && (o[1] & 0xc0) == 0x80) // link-local fe80::/10
        }
    }
}

fn to_reqwest_method(method: HttpMethod) -> reqwest::Method {
    match method {
        HttpMethod::Get => reqwest::Method::GET,
        HttpMethod::Post => reqwest::Method::POST,
        HttpMethod::Put => reqwest::Method::PUT,
        HttpMethod::Patch => reqwest::Method::PATCH,
        HttpMethod::Delete => reqwest::Method::DELETE,
        HttpMethod::Head => reqwest::Method::HEAD,
        HttpMethod::Options => reqwest::Method::OPTIONS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::KeyValueField;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        time::{Duration, sleep, timeout},
    };

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
            file_path: None,
        }
    }

    #[tokio::test]
    async fn execute_request_returns_cancelled_when_token_is_already_cancelled() {
        let client = reqwest::Client::new();
        let doc = simple_doc("https://httpbin.org/delay/10");
        let token = CancellationToken::new();
        token.cancel();

        let result = execute_request(&client, &doc, &HashMap::new(), token).await;
        assert!(matches!(result, Err(ExecutionError::Cancelled)));
    }

    #[tokio::test]
    async fn execute_request_fails_on_unresolved_variables() {
        let client = reqwest::Client::new();
        let doc = simple_doc("{{base_url}}/api");
        let token = CancellationToken::new();

        let result = execute_request(&client, &doc, &HashMap::new(), token).await;
        assert!(matches!(result, Err(ExecutionError::Interpolation(_))));
    }

    #[test]
    fn to_reqwest_method_maps_all_variants() {
        assert_eq!(to_reqwest_method(HttpMethod::Get), reqwest::Method::GET);
        assert_eq!(to_reqwest_method(HttpMethod::Post), reqwest::Method::POST);
        assert_eq!(to_reqwest_method(HttpMethod::Put), reqwest::Method::PUT);
        assert_eq!(to_reqwest_method(HttpMethod::Patch), reqwest::Method::PATCH);
        assert_eq!(
            to_reqwest_method(HttpMethod::Delete),
            reqwest::Method::DELETE
        );
        assert_eq!(to_reqwest_method(HttpMethod::Head), reqwest::Method::HEAD);
        assert_eq!(
            to_reqwest_method(HttpMethod::Options),
            reqwest::Method::OPTIONS
        );
    }

    #[test]
    fn json_pretty_print_works() {
        let raw = r#"{"key":"value","nested":{"a":1}}"#;
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        let pretty = serde_json::to_string_pretty(&parsed).unwrap();
        assert!(pretty.contains('\n'));
        assert!(pretty.contains("  "));
    }

    #[tokio::test]
    async fn execute_request_adds_headers_and_params() {
        let client = reqwest::Client::new();
        let doc = RequestDocument {
            name: "Test".into(),
            method: HttpMethod::Get,
            url: "https://invalid.test.example".into(),
            headers: vec![KeyValueField {
                key: "X-Custom".into(),
                value: "test-value".into(),
                enabled: true,
            }],
            params: vec![
                KeyValueField {
                    key: "q".into(),
                    value: "search".into(),
                    enabled: true,
                },
                KeyValueField {
                    key: "disabled".into(),
                    value: "skip".into(),
                    enabled: false,
                },
            ],
            body: None,
            auth_plugin: None,
            assertions: vec![],
            file_path: None,
        };
        let token = CancellationToken::new();

        // This will fail with a network error, but it tests that the request
        // is built successfully (no panics, no invalid request errors).
        let result = execute_request(&client, &doc, &HashMap::new(), token).await;
        assert!(matches!(result, Err(ExecutionError::Network(_))));
    }

    #[tokio::test]
    async fn execute_request_returns_after_hitting_body_limit_without_waiting_for_close() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request_buf = [0_u8; 1024];
            let _ = socket.read(&mut request_buf).await.unwrap();

            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
                MAX_BODY_SIZE + 100
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket
                .write_all(&vec![b'a'; MAX_BODY_SIZE + 1])
                .await
                .unwrap();
            sleep(Duration::from_secs(2)).await;
        });

        let client = reqwest::Client::new();
        let doc = simple_doc(&format!("http://{addr}"));
        let token = CancellationToken::new();

        let result = timeout(
            Duration::from_millis(500),
            execute_request(&client, &doc, &HashMap::new(), token),
        )
        .await;

        assert!(
            result.is_ok(),
            "request should finish once the cap is reached"
        );
        let artifact = result.unwrap().unwrap();
        let body = artifact.body_text.unwrap();
        assert!(body.contains("[truncated at"));
    }

    #[tokio::test]
    async fn execute_request_honors_cancellation_while_streaming_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request_buf = [0_u8; 1024];
            let _ = socket.read(&mut request_buf).await.unwrap();

            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 32\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nhello",
                )
                .await
                .unwrap();
            sleep(Duration::from_secs(2)).await;
        });

        let client = reqwest::Client::new();
        let doc = simple_doc(&format!("http://{addr}"));
        let token = CancellationToken::new();
        let cancel = token.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            cancel.cancel();
        });

        let result = timeout(
            Duration::from_millis(500),
            execute_request(&client, &doc, &HashMap::new(), token),
        )
        .await;

        assert!(matches!(result, Ok(Err(ExecutionError::Cancelled))));
    }

    // --- B2: scheme allowlist ---

    #[test]
    fn validate_scheme_allows_http_and_https() {
        let http = reqwest::Url::parse("http://example.com").unwrap();
        let https = reqwest::Url::parse("https://example.com").unwrap();
        assert!(validate_scheme(&http).is_ok());
        assert!(validate_scheme(&https).is_ok());
    }

    #[test]
    fn validate_scheme_rejects_other_schemes() {
        let file_url = reqwest::Url::parse("file:///etc/passwd").unwrap();
        let ftp_url = reqwest::Url::parse("ftp://example.com/file").unwrap();
        assert!(matches!(
            validate_scheme(&file_url),
            Err(ExecutionError::InvalidRequest(_))
        ));
        assert!(matches!(
            validate_scheme(&ftp_url),
            Err(ExecutionError::InvalidRequest(_))
        ));
    }

    #[tokio::test]
    async fn execute_request_rejects_non_http_scheme() {
        let client = reqwest::Client::new();
        let doc = simple_doc("ftp://example.com/file");
        let result =
            execute_request(&client, &doc, &HashMap::new(), CancellationToken::new()).await;
        assert!(matches!(result, Err(ExecutionError::InvalidRequest(_))));
    }

    // --- B2: private-network policy (opt-in) ---

    #[test]
    fn is_private_or_local_classifies_v4_addresses() {
        assert!(is_private_or_local(&"0.0.0.0".parse::<IpAddr>().unwrap()));
        assert!(is_private_or_local(&"127.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_or_local(&"10.1.2.3".parse::<IpAddr>().unwrap()));
        assert!(is_private_or_local(
            &"172.16.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(is_private_or_local(
            &"192.168.1.1".parse::<IpAddr>().unwrap()
        ));
        assert!(is_private_or_local(
            &"169.254.1.1".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_private_or_local(&"8.8.8.8".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn is_private_or_local_classifies_v6_addresses() {
        assert!(is_private_or_local(&"::".parse::<IpAddr>().unwrap()));
        assert!(is_private_or_local(&"::1".parse::<IpAddr>().unwrap()));
        assert!(is_private_or_local(
            &"::ffff:127.0.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(is_private_or_local(
            &"::ffff:192.168.1.1".parse::<IpAddr>().unwrap()
        ));
        assert!(is_private_or_local(&"fc00::1".parse::<IpAddr>().unwrap()));
        assert!(is_private_or_local(
            &"fd12:3456::1".parse::<IpAddr>().unwrap()
        ));
        assert!(is_private_or_local(&"fe80::1".parse::<IpAddr>().unwrap()));
        assert!(!is_private_or_local(
            &"2001:4860:4860::8888".parse::<IpAddr>().unwrap()
        ));
    }

    #[tokio::test]
    async fn execute_request_allows_loopback_by_default() {
        // Default options (deny_private_networks: false) must not block
        // localhost; this is the primary use case for a local API client.
        let client = reqwest::Client::new();
        let doc = simple_doc("http://127.0.0.1:9");
        let result =
            execute_request(&client, &doc, &HashMap::new(), CancellationToken::new()).await;
        // Not blocked by policy -> fails with a network error (closed port),
        // never InvalidRequest.
        assert!(matches!(result, Err(ExecutionError::Network(_))));
    }

    #[tokio::test]
    async fn execute_request_with_options_denies_loopback_when_opted_in() {
        let client = reqwest::Client::new();
        let doc = simple_doc("http://127.0.0.1:9");
        let options = ExecutionOptions {
            deny_private_networks: true,
        };
        let result = execute_request_with_options(
            &client,
            &doc,
            &HashMap::new(),
            CancellationToken::new(),
            options,
        )
        .await;
        assert!(matches!(result, Err(ExecutionError::InvalidRequest(_))));
    }

    #[test]
    fn evaluate_resolved_ips_rejects_any_private_among_public() {
        let mixed = [
            "8.8.8.8".parse::<IpAddr>().unwrap(),
            "10.0.0.5".parse::<IpAddr>().unwrap(),
        ];
        assert!(matches!(
            evaluate_resolved_ips("mixed.example", &mixed),
            Err(ExecutionError::InvalidRequest(_))
        ));
    }

    #[test]
    fn evaluate_resolved_ips_allows_all_public() {
        let public = [
            "8.8.8.8".parse::<IpAddr>().unwrap(),
            "1.1.1.1".parse::<IpAddr>().unwrap(),
        ];
        assert!(evaluate_resolved_ips("public.example", &public).is_ok());
    }

    #[test]
    fn evaluate_resolved_ips_fails_closed_on_empty_resolution() {
        assert!(matches!(
            evaluate_resolved_ips("unresolvable.example", &[]),
            Err(ExecutionError::InvalidRequest(_))
        ));
    }

    #[tokio::test]
    async fn check_private_network_policy_allows_public_ip_literal() {
        // Exercise the policy check directly (no real network connection
        // involved, unlike routing through `execute_request`) so this test
        // is deterministic in a sandboxed/offline CI environment.
        let url = reqwest::Url::parse("http://93.184.216.34:9").unwrap();
        assert!(check_private_network_policy(&url).await.is_ok());
    }

    // --- B2: critical regression — redirect must not leak Authorization
    // across an origin change (reqwest strips it automatically; this test
    // locks in that guarantee for a same-host, different-port redirect). ---

    #[tokio::test]
    async fn redirect_strips_authorization_header_across_port_change() {
        use tokio::sync::oneshot;

        // Second listener: the redirect target. Captures whatever raw
        // request headers it receives.
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target_listener.local_addr().unwrap();
        let (captured_tx, captured_rx) = oneshot::channel::<String>();

        tokio::spawn(async move {
            let (mut socket, _) = target_listener.accept().await.unwrap();
            let mut buf = vec![0_u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let request_text = String::from_utf8_lossy(&buf[..n]).to_string();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
                .await
                .unwrap();
            let _ = captured_tx.send(request_text);
        });

        // First listener: returns a 301 redirect to the second listener's
        // port on the SAME host — an origin change reqwest must treat as
        // cross-origin for the purposes of stripping sensitive headers.
        let redirect_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect_addr = redirect_listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut socket, _) = redirect_listener.accept().await.unwrap();
            let mut buf = [0_u8; 1024];
            let _ = socket.read(&mut buf).await.unwrap();
            let response = format!(
                "HTTP/1.1 301 Moved Permanently\r\nLocation: http://127.0.0.1:{}/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                target_addr.port()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let client = crate::infra::http_client::build_client().unwrap();
        let doc = RequestDocument {
            name: "Redirect".into(),
            method: HttpMethod::Get,
            url: format!("http://{redirect_addr}"),
            headers: vec![crate::core::models::KeyValueField {
                key: "Authorization".into(),
                value: "Bearer super-secret-token".into(),
                enabled: true,
            }],
            params: vec![],
            body: None,
            auth_plugin: None,
            assertions: vec![],
            file_path: None,
        };

        let token = CancellationToken::new();
        let result = timeout(
            Duration::from_secs(2),
            execute_request(&client, &doc, &HashMap::new(), token),
        )
        .await
        .expect("request should complete before timeout")
        .expect("redirected request should succeed");

        assert_eq!(result.status_code, 200);

        let captured = timeout(Duration::from_secs(1), captured_rx)
            .await
            .expect("target listener should receive the redirected request")
            .expect("capture channel should not be dropped");

        let lower = captured.to_lowercase();
        assert!(
            !lower.contains("authorization"),
            "reqwest must strip Authorization on a cross-port redirect, but the \
             second listener received it:\n{captured}"
        );
    }
}
