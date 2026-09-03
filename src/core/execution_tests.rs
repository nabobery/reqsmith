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

    let result = execute_request_with_options(
        &client,
        &doc,
        &HashMap::new(),
        token,
        ExecutionOptions::default(),
    )
    .await;
    assert!(matches!(result, Err(ExecutionError::Cancelled)));
}

#[tokio::test]
async fn execute_request_fails_on_unresolved_variables() {
    let client = reqwest::Client::new();
    let doc = simple_doc("{{base_url}}/api");
    let token = CancellationToken::new();

    let result = execute_request_with_options(
        &client,
        &doc,
        &HashMap::new(),
        token,
        ExecutionOptions::default(),
    )
    .await;
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
fn is_likely_binary_table() {
    // (bytes, expected) — the threshold is >5% nulls in the first 8 KiB.
    let one_null_in_twenty = {
        let mut v = vec![b'a'; 20];
        v[0] = 0;
        v
    };
    let two_nulls_in_twenty = {
        let mut v = vec![b'a'; 20];
        v[0] = 0;
        v[1] = 0;
        v
    };
    let cases: [(&[u8], bool); 4] = [
        (&[], false),
        (b"plain text", false),
        (&one_null_in_twenty, false), // exactly 5% is not over the threshold
        (&two_nulls_in_twenty, true),
    ];
    for (bytes, expected) in cases {
        assert_eq!(
            is_likely_binary(bytes),
            expected,
            "unexpected classification for {} bytes",
            bytes.len()
        );
    }
}

#[test]
fn is_truthy_accepts_only_one_and_true() {
    for value in ["1", "true", "TRUE", " True "] {
        assert!(is_truthy(value), "{value:?} should be truthy");
    }
    for value in ["", "0", "false", "yes", "on", "2"] {
        assert!(!is_truthy(value), "{value:?} should not be truthy");
    }
}

#[test]
fn render_body_leaves_binary_bodies_to_the_sinks() {
    assert_eq!(
        render_body(&[0, 1, 2, 3], Some("application/octet-stream"), true),
        None
    );
}

#[test]
fn render_body_never_edits_the_body_to_flag_truncation() {
    // A cut JSON body must come back verbatim: assertions read these bytes.
    let cut = br#"{"users":[{"id":1"#;
    let body = render_body(cut, Some("application/json"), false).unwrap();
    assert_eq!(body, r#"{"users":[{"id":1"#);
    assert!(!body.contains("truncated"));
}

#[test]
fn render_body_pretty_prints_json() {
    let body = render_body(br#"{"a":1}"#, Some("application/json"), false).unwrap();
    assert!(body.contains('\n'));
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
        url: "http://127.0.0.1:9".into(),
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
    let result = execute_request_with_options(
        &client,
        &doc,
        &HashMap::new(),
        token,
        ExecutionOptions::default(),
    )
    .await;
    assert!(matches!(result, Err(ExecutionError::Network(_))));
}

#[tokio::test]
async fn execute_request_reports_flattened_detail_for_a_bad_header_value() {
    // reqwest's `Display` for a builder error is just "builder error";
    // the useful detail (which header, why) is one level down in the
    // source chain, so `flatten` must surface it instead of the caller
    // seeing the generic top-level message.
    let client = reqwest::Client::new();
    let doc = RequestDocument {
        name: "Test".into(),
        method: HttpMethod::Get,
        url: "http://127.0.0.1:9".into(),
        headers: vec![KeyValueField {
            key: "X-Bad".into(),
            value: "line1\nline2".into(),
            enabled: true,
        }],
        params: vec![],
        body: None,
        auth_plugin: None,
        assertions: vec![],
        file_path: None,
    };

    let result = execute_request_with_options(
        &client,
        &doc,
        &HashMap::new(),
        CancellationToken::new(),
        ExecutionOptions::default(),
    )
    .await;

    match result {
        Err(ExecutionError::InvalidRequest(message)) => {
            assert_ne!(
                message, "builder error",
                "the source-chain detail must not be dropped"
            );
        }
        other => panic!("expected InvalidRequest, got {other:?}"),
    }
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
        execute_request_with_options(
            &client,
            &doc,
            &HashMap::new(),
            token,
            ExecutionOptions::default(),
        ),
    )
    .await;

    assert!(
        result.is_ok(),
        "request should finish once the cap is reached"
    );
    let artifact = result.unwrap().unwrap();
    assert!(
        artifact.truncated,
        "the cap was hit, so the flag must be set"
    );
    let body = artifact.body_text.unwrap();
    assert_eq!(
        body.len(),
        MAX_BODY_SIZE,
        "the body must be the real cut bytes"
    );
    assert!(
        !body.contains("[truncated at"),
        "truncation is reported by the flag, not by editing the body"
    );
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
        execute_request_with_options(
            &client,
            &doc,
            &HashMap::new(),
            token,
            ExecutionOptions::default(),
        ),
    )
    .await;

    assert!(matches!(result, Ok(Err(ExecutionError::Cancelled))));
}

#[tokio::test]
async fn network_errors_do_not_echo_the_request_url() {
    let client = reqwest::Client::new();
    let doc = simple_doc("http://127.0.0.1:9/v1?api_key=AKIA-SUPER-SECRET");
    let result = execute_request_with_options(
        &client,
        &doc,
        &HashMap::new(),
        CancellationToken::new(),
        ExecutionOptions::default(),
    )
    .await;

    let err = result.unwrap_err().to_string();
    assert!(
        !err.contains("AKIA-SUPER-SECRET"),
        "the URL (and its query-string credentials) must be stripped, got: {err}"
    );
}

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

#[test]
fn redirect_validation_enforces_scheme_downgrade_and_hop_rules() {
    let https = reqwest::Url::parse("https://example.com/start").unwrap();
    let http = reqwest::Url::parse("http://example.com/next").unwrap();
    let ftp = reqwest::Url::parse("ftp://example.com/next").unwrap();
    let next = reqwest::Url::parse("https://example.com/next").unwrap();

    assert!(validate_redirect(&https, &http, 0).is_err());
    assert!(validate_redirect(&https, &ftp, 0).is_err());
    assert!(validate_redirect(&https, &next, MAX_REDIRECTS).is_err());
    assert!(validate_redirect(&https, &next, MAX_REDIRECTS - 1).is_ok());
}

#[test]
fn redirect_rewrite_matches_post_to_get_and_cross_origin_rules() {
    let previous = reqwest::Url::parse("http://example.com:80/start#fragment").unwrap();
    let next = reqwest::Url::parse("https://example.com:443/next").unwrap();
    let mut request = reqwest::Request::new(Method::POST, previous.clone());
    *request.body_mut() = Some("payload".into());
    request
        .headers_mut()
        .insert(AUTHORIZATION, HeaderValue::from_static("secret"));
    request
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));

    rewrite_redirect_request(&mut request, StatusCode::FOUND, &previous, next.clone());

    assert_eq!(request.method(), Method::GET);
    assert!(request.body().is_none());
    assert!(!request.headers().contains_key(AUTHORIZATION));
    assert!(!request.headers().contains_key(CONTENT_TYPE));
    assert_eq!(request.url(), &next);
    assert_eq!(
        request.headers().get(REFERER).unwrap(),
        "http://example.com/start"
    );
}

#[tokio::test]
async fn execute_request_rejects_non_http_scheme() {
    let client = reqwest::Client::new();
    let doc = simple_doc("ftp://example.com/file");
    let result = execute_request_with_options(
        &client,
        &doc,
        &HashMap::new(),
        CancellationToken::new(),
        ExecutionOptions::default(),
    )
    .await;
    assert!(matches!(result, Err(ExecutionError::InvalidRequest(_))));
}

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
    let result = execute_request_with_options(
        &client,
        &doc,
        &HashMap::new(),
        CancellationToken::new(),
        ExecutionOptions::default(),
    )
    .await;
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

#[test]
fn is_private_or_local_classifies_extra_v4_ranges() {
    for blocked in [
        "0.0.0.1",         // "this network" 0.0.0.0/8
        "100.64.0.1",      // CGNAT 100.64.0.0/10
        "100.127.255.254", // CGNAT upper edge
        "192.0.0.1",       // IETF protocol assignments
        "198.18.0.1",      // benchmarking
        "198.19.255.254",  // benchmarking upper edge
        "224.0.0.1",       // multicast
        "240.0.0.1",       // reserved
        "255.255.255.255", // broadcast
    ] {
        assert!(
            is_private_or_local(&blocked.parse::<IpAddr>().unwrap()),
            "{blocked} should be classified as non-public"
        );
    }

    for allowed in ["100.63.255.255", "192.0.1.1", "198.17.0.1", "198.20.0.1"] {
        assert!(
            !is_private_or_local(&allowed.parse::<IpAddr>().unwrap()),
            "{allowed} should be classified as public"
        );
    }
}

#[test]
fn is_private_or_local_classifies_extra_v6_ranges() {
    for blocked in [
        "::127.0.0.1",    // IPv4-compatible loopback
        "::10.0.0.1",     // IPv4-compatible RFC1918
        "2002:a00:1::",   // 6to4 wrapping 10.0.0.1
        "2002:7f00:1::",  // 6to4 wrapping 127.0.0.1
        "2001::1",        // Teredo 2001::/32
        "64:ff9b::a00:1", // NAT64 wrapping 10.0.0.1
        "ff02::1",        // multicast
    ] {
        assert!(
            is_private_or_local(&blocked.parse::<IpAddr>().unwrap()),
            "{blocked} should be classified as non-public"
        );
    }

    for allowed in ["2002:808:808::", "64:ff9b::808:808", "2001:4860:4860::8888"] {
        assert!(
            !is_private_or_local(&allowed.parse::<IpAddr>().unwrap()),
            "{allowed} should be classified as public"
        );
    }
}

#[tokio::test]
async fn check_private_network_policy_blocks_ipv6_loopback_literal() {
    let url = reqwest::Url::parse("http://[::1]/").unwrap();
    let err = check_private_network_policy(&url).await.unwrap_err();
    assert!(
        err.to_string().contains("private/loopback"),
        "expected a policy rejection, got: {err}"
    );
}

#[tokio::test]
async fn check_private_network_policy_allows_public_ipv6_literal() {
    // Must be recognised as a literal rather than handed to the DNS
    // branch, which would need network access to answer.
    let url = reqwest::Url::parse("http://[2001:4860:4860::8888]/").unwrap();
    assert!(check_private_network_policy(&url).await.is_ok());
}

#[test]
fn resolve_failure_blocks_by_policy_and_names_the_host() {
    let err = std::io::Error::new(std::io::ErrorKind::NotFound, "nxdomain");
    match resolve_failure("bad.invalid", &err) {
        ExecutionError::InvalidRequest(message) => {
            assert!(message.contains("bad.invalid"));
            assert!(message.contains("could not resolve"));
        }
        other => panic!("expected InvalidRequest, got {other:?}"),
    }
}

#[tokio::test]
async fn check_private_network_policy_allows_public_ip_literal() {
    // Exercise the policy check directly (no real network connection
    // involved, unlike routing through `execute_request`) so this test
    // is deterministic in a sandboxed/offline CI environment.
    let url = reqwest::Url::parse("http://93.184.216.34:9").unwrap();
    assert!(check_private_network_policy(&url).await.is_ok());
}

// Regression: redirects must not leak Authorization across an origin
// change. This locks in that guarantee for a same-host, different-port
// redirect through reqsmith's explicit redirect loop.
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

    let client = crate::infra::http_client::build_client(false).unwrap();
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
        execute_request_with_options(
            &client,
            &doc,
            &HashMap::new(),
            token,
            ExecutionOptions::default(),
        ),
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
