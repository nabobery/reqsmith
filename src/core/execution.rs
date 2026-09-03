use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use reqwest::header::{
    AUTHORIZATION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, COOKIE, HeaderValue, LOCATION,
    PROXY_AUTHORIZATION, REFERER, TRANSFER_ENCODING, WWW_AUTHENTICATE,
};
use reqwest::{Method, StatusCode};
use tokio::time::Instant as TokioInstant;

use tokio_util::sync::CancellationToken;

use super::interpolation::interpolate_document;
use super::models::{HttpMethod, RequestDocument, ResponseArtifact};
use crate::infra::http_client::{CONNECT_TIMEOUT_SECS, MAX_REDIRECTS, TOTAL_TIMEOUT_SECS};

/// Maximum response body size to read into memory (10 MB).
pub(crate) const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

/// Human-readable form of [`MAX_BODY_SIZE`] for user-facing truncation notices.
pub(crate) fn max_body_size_label() -> String {
    format!("{} MB", MAX_BODY_SIZE / (1024 * 1024))
}

/// Upper bound on the body buffer reserved up front from `Content-Length`.
const INITIAL_BODY_CAPACITY: usize = 64 * 1024;

/// Policy knobs for [`execute_request_with_options`].
///
/// Defaults preserve the pre-hardening behavior: private/loopback addresses
/// are allowed. reqsmith is a local, user-driven tool — sending requests to
/// `localhost`, `127.0.0.1`, or a machine on the private LAN is a primary,
/// legitimate use case, so that must keep working out of the box.
///
/// The `client` passed to [`execute_request_with_options`] must be built with
/// `infra::http_client::build_client(deny_private_networks)` using this same
/// flag so proxy handling and the per-hop policy stay aligned.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecutionOptions {
    /// When `true`, reject requests whose host is — or resolves to — anything
    /// other than a public unicast address (see [`is_private_or_local`]).
    /// Opt-in; default is `false`.
    pub deny_private_networks: bool,
}

/// Heuristic: treat a response as binary if >5% of the first 8 KiB are null bytes.
fn is_likely_binary(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(8192)];
    let null_count = sample.iter().filter(|&&b| b == 0).count();
    null_count > sample.len() / 20 // >5% null bytes
}

#[derive(Debug, thiserror::Error)]
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

/// Flatten a reqwest error's source chain into one message. reqwest's own
/// `Display` is only the outer layer (e.g. plain "builder error") — the
/// useful detail ("connection refused", an invalid header value) lives in
/// the source chain.
fn flatten(e: &reqwest::Error) -> String {
    let mut message = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

/// Which timeout budget a timed-out request exceeded. reqwest's connect
/// timeout also makes `is_timeout()` true, so it must be checked first or
/// every connect timeout gets mislabeled with the (much longer) total budget.
fn timeout_label(is_connect: bool) -> String {
    if is_connect {
        format!("exceeded the {CONNECT_TIMEOUT_SECS}s connect timeout")
    } else {
        format!("exceeded the {TOTAL_TIMEOUT_SECS}s total timeout")
    }
}

/// Turn a reqwest failure into a message that never carries the request URL.
///
/// `Error::without_url` drops the URL (which can hold query-string
/// credentials).
fn network_error(e: reqwest::Error) -> ExecutionError {
    let timed_out = e.is_timeout();
    let is_connect = e.is_connect();
    let e = e.without_url();

    let message = flatten(&e);
    let message = if timed_out {
        format!("request {}: {message}", timeout_label(is_connect))
    } else {
        message
    };
    ExecutionError::Network(message)
}

/// Execute an HTTP request built from a `RequestDocument` with environment
/// variable interpolation, under the given network policy options. Supports
/// cancellation via `CancellationToken`.
pub async fn execute_request_with_options(
    client: &reqwest::Client,
    doc: &RequestDocument,
    vars: &HashMap<String, String>,
    cancel: CancellationToken,
    options: ExecutionOptions,
) -> Result<ResponseArtifact, ExecutionError> {
    let resolved = interpolate_document(doc, vars).map_err(ExecutionError::Interpolation)?;

    // Parse and validate the URL: scheme allowlist (always enforced) and
    // optional private-network denial (opt-in via `options`).
    // The raw URL is not echoed: it can carry credentials in its query string.
    let parsed_url = reqwest::Url::parse(&resolved.url)
        .map_err(|e| ExecutionError::InvalidRequest(format!("Invalid URL: {e}")))?;
    validate_scheme(&parsed_url)?;
    let request = build_request(client, &resolved, parsed_url)?;
    let start = Instant::now();
    let deadline = TokioInstant::now() + Duration::from_secs(TOTAL_TIMEOUT_SECS);

    let mut response = send_with_redirects(client, request, &cancel, options, deadline).await?;

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

    let CappedBody {
        bytes: body_bytes,
        truncated,
    } = tokio::time::timeout_at(
        deadline,
        read_body_capped(&mut response, content_length, &cancel),
    )
    .await
    .map_err(|_| total_timeout_error())??;

    // Timed here, not at the header phase: the caller cares about how long the
    // whole response took to arrive.
    let duration_ms = start.elapsed().as_millis();

    let binary = is_likely_binary(&body_bytes);
    let body_text = render_body(&body_bytes, content_type.as_deref(), binary);

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
        truncated,
    })
}

/// Send one request and follow redirects within one cancellable total timeout.
/// Redirects are handled here, rather than in reqwest's synchronous policy
/// callback, so DNS validation never blocks a Tokio runtime worker.
async fn send_with_redirects(
    client: &reqwest::Client,
    request: reqwest::Request,
    cancel: &CancellationToken,
    options: ExecutionOptions,
    deadline: TokioInstant,
) -> Result<reqwest::Response, ExecutionError> {
    let send = async {
        let mut request = request;
        let mut hops = 0;

        loop {
            if options.deny_private_networks {
                check_private_network_policy(request.url()).await?;
            }

            let previous_url = request.url().clone();
            let Some(mut next_request) = request.try_clone() else {
                return client.execute(request).await.map_err(network_error);
            };
            let response = client.execute(request).await.map_err(network_error)?;
            let status = response.status();
            if !is_redirect(status) {
                return Ok(response);
            }

            let Some(next_url) = redirect_target(&previous_url, response.headers()) else {
                return Ok(response);
            };
            validate_redirect(&previous_url, &next_url, hops)?;

            rewrite_redirect_request(&mut next_request, status, &previous_url, next_url);
            request = next_request;
            hops += 1;
        }
    };

    tokio::select! {
        result = tokio::time::timeout_at(deadline, send) => {
            result.unwrap_or_else(|_| Err(total_timeout_error()))
        }
        () = cancel.cancelled() => Err(ExecutionError::Cancelled),
    }
}

fn total_timeout_error() -> ExecutionError {
    ExecutionError::Network(format!(
        "request exceeded the {TOTAL_TIMEOUT_SECS}s total timeout"
    ))
}

fn is_redirect(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

/// Resolve a Location header against the previous URL. Invalid or absent
/// locations stop redirecting, matching ordinary user-agent behavior.
fn redirect_target(
    previous: &reqwest::Url,
    headers: &reqwest::header::HeaderMap,
) -> Option<reqwest::Url> {
    let location = headers.get(LOCATION)?.to_str().ok()?;
    previous.join(location).ok()
}

fn validate_redirect(
    previous: &reqwest::Url,
    next: &reqwest::Url,
    hops: usize,
) -> Result<(), ExecutionError> {
    if hops >= MAX_REDIRECTS {
        return Err(ExecutionError::Network(format!(
            "too many redirects (limit {MAX_REDIRECTS} hops)"
        )));
    }
    validate_scheme(next)?;
    if previous.scheme() == "https" && next.scheme() == "http" {
        return Err(ExecutionError::InvalidRequest(
            "Refusing https -> http redirect: the downgraded hop would travel in cleartext".into(),
        ));
    }
    Ok(())
}

/// Apply the method, body, and credential rules reqwest normally applies when
/// following a redirect automatically.
fn rewrite_redirect_request(
    request: &mut reqwest::Request,
    status: StatusCode,
    previous: &reqwest::Url,
    next: reqwest::Url,
) {
    let switch_to_get = matches!(status, StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND)
        && request.method() == Method::POST
        || status == StatusCode::SEE_OTHER && request.method() != Method::HEAD;
    if switch_to_get {
        *request.method_mut() = Method::GET;
    }
    if switch_to_get || status == StatusCode::SEE_OTHER {
        *request.body_mut() = None;
        for header in [
            CONTENT_TYPE,
            CONTENT_LENGTH,
            CONTENT_ENCODING,
            TRANSFER_ENCODING,
        ] {
            request.headers_mut().remove(header);
        }
    }

    if crosses_host_or_port(previous, &next) {
        for header in [AUTHORIZATION, COOKIE, PROXY_AUTHORIZATION, WWW_AUTHENTICATE] {
            request.headers_mut().remove(header);
        }
        request.headers_mut().remove("cookie2");
    }

    if let Some(referer) = redirect_referer(previous, &next) {
        request.headers_mut().insert(REFERER, referer);
    }
    *request.url_mut() = next;
}

fn crosses_host_or_port(previous: &reqwest::Url, next: &reqwest::Url) -> bool {
    previous.host_str() != next.host_str()
        || previous.port_or_known_default() != next.port_or_known_default()
}

fn redirect_referer(previous: &reqwest::Url, next: &reqwest::Url) -> Option<HeaderValue> {
    if previous.scheme() == "https" && next.scheme() == "http" {
        return None;
    }
    let mut referer = previous.clone();
    let _ = referer.set_username("");
    let _ = referer.set_password(None);
    referer.set_fragment(None);
    HeaderValue::from_str(referer.as_str()).ok()
}

/// Build the outgoing reqwest request from a resolved (interpolated) document.
fn build_request(
    client: &reqwest::Client,
    resolved: &RequestDocument,
    url: reqwest::Url,
) -> Result<reqwest::Request, ExecutionError> {
    let method = to_reqwest_method(resolved.method);
    let mut builder = client.request(method, url);

    for param in &resolved.params {
        if param.enabled {
            builder = builder.query(&[(&param.key, &param.value)]);
        }
    }

    for header in &resolved.headers {
        if header.enabled {
            builder = builder.header(&header.key, &header.value);
        }
    }

    if let Some(body) = &resolved.body {
        builder = builder.body(body.clone());
    }

    // `flatten` here too: a bad header value otherwise reports only the
    // generic "builder error", hiding which header/value was rejected.
    builder
        .build()
        .map_err(|e| ExecutionError::InvalidRequest(flatten(&e.without_url())))
}

/// Response bytes read under [`MAX_BODY_SIZE`], and whether the cap cut them short.
struct CappedBody {
    bytes: Vec<u8>,
    truncated: bool,
}

/// Stream the response body until it ends or [`MAX_BODY_SIZE`] is reached.
async fn read_body_capped(
    response: &mut reqwest::Response,
    content_length: Option<u64>,
    cancel: &CancellationToken,
) -> Result<CappedBody, ExecutionError> {
    // Cap the pre-allocation: `Content-Length` is server-controlled, so trusting
    // it would let a response reserve up to MAX_BODY_SIZE before a byte arrives.
    let mut bytes = Vec::with_capacity(
        content_length
            .unwrap_or(0)
            .min(INITIAL_BODY_CAPACITY as u64) as usize,
    );

    loop {
        let next_chunk = tokio::select! {
            chunk = response.chunk() => {
                chunk.map_err(network_error)?
            }
            () = cancel.cancelled() => {
                return Err(ExecutionError::Cancelled);
            }
        };

        let Some(chunk) = next_chunk else {
            break;
        };

        let remaining = MAX_BODY_SIZE.saturating_sub(bytes.len());
        if remaining == 0 {
            return Ok(CappedBody {
                bytes,
                truncated: true,
            });
        }

        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            return Ok(CappedBody {
                bytes,
                truncated: true,
            });
        }

        bytes.extend_from_slice(&chunk);
    }

    Ok(CappedBody {
        bytes,
        truncated: false,
    })
}

/// Decode a body for display. Binary bodies stay `None`: the placeholder text
/// is presentation, so each sink writes its own.
fn render_body(bytes: &[u8], content_type: Option<&str>, binary: bool) -> Option<String> {
    if binary {
        return None;
    }

    let raw = String::from_utf8_lossy(bytes).into_owned();
    if content_type.is_some_and(|ct| ct.contains("application/json"))
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw)
    {
        return serde_json::to_string_pretty(&json).ok().or(Some(raw));
    }
    Some(raw)
}

/// `REQSMITH_DENY_PRIVATE_NETWORKS=1|true` (case-insensitive) turns the
/// private-network guard on without passing `--deny-private-networks`.
pub fn deny_private_networks_from_env() -> bool {
    std::env::var("REQSMITH_DENY_PRIVATE_NETWORKS").is_ok_and(|value| is_truthy(&value))
}

fn is_truthy(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true")
}

/// Reject any URL scheme other than `http`/`https`. Applies to every
/// request regardless of `ExecutionOptions` — this is not opt-in.
pub(crate) fn validate_scheme(url: &reqwest::Url) -> Result<(), ExecutionError> {
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

    // `host_str` brackets IPv6 literals, so a plain `parse::<IpAddr>()` here
    // would miss `[::1]` and fall through to the DNS branch.
    if let Some(ip) = host_ip_literal(host) {
        return if is_private_or_local(&ip) {
            Err(private_network_error(host))
        } else {
            Ok(())
        };
    }

    let port = url.port_or_known_default().unwrap_or(80);
    let ips: Vec<IpAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| resolve_failure(host, &e))?
        .map(|addr| addr.ip())
        .collect();

    evaluate_resolved_ips(host, &ips)
}

/// Error for a host that could not be resolved while the guard is enabled.
/// Fail closed: rather than handing an unresolvable host to reqwest to
/// resolve (and possibly connect) without a policy check, reject it. Factored
/// out of the `tokio::net::lookup_host` call so it is unit-testable without a
/// real resolver.
fn resolve_failure(host: &str, err: &std::io::Error) -> ExecutionError {
    ExecutionError::InvalidRequest(format!(
        "Blocked '{host}' by network policy: could not resolve host to \
         verify it is not private ({err})"
    ))
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

/// Parse a URL host as an IP literal, tolerating the bracketed `[::1]` form
/// that `Url::host_str` returns for IPv6.
pub(crate) fn host_ip_literal(host: &str) -> Option<IpAddr> {
    host.strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host)
        .parse()
        .ok()
}

/// Classify an address as anything other than public unicast: loopback,
/// private, link-local, shared/CGNAT, benchmarking, multicast, or reserved.
///
/// IPv6 forms that tunnel an IPv4 address (mapped, compatible, 6to4, NAT64)
/// are classified by the address they embed, so `::ffff:10.0.0.1` and
/// `2002:a00:1::` are rejected exactly like `10.0.0.1` is.
pub(crate) fn is_private_or_local(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_unspecified()
                || v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast() // 224.0.0.0/4
                || o[0] == 0 // "this network" 0.0.0.0/8
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // shared address space 100.64.0.0/10
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF protocol assignments 192.0.0.0/24
                || (o[0] == 198 && (o[1] & 0xfe) == 18) // benchmarking 198.18.0.0/15
                || o[0] >= 240 // reserved 240.0.0.0/4, which contains 255.255.255.255
        }
        IpAddr::V6(v6) => {
            // `to_ipv4` maps `::1` to `0.0.0.1`, so these must come first.
            if v6.is_unspecified() || v6.is_loopback() {
                return true;
            }
            if let Some(v4) = v6.to_ipv4() {
                return is_private_or_local(&IpAddr::V4(v4));
            }
            let o = v6.octets();
            if o[0] == 0x20 && o[1] == 0x02 {
                // 6to4 2002::/16 embeds the v4 address in the next 32 bits.
                return is_private_or_local(&IpAddr::V4(Ipv4Addr::new(o[2], o[3], o[4], o[5])));
            }
            if o[..12] == [0x00, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0] {
                // NAT64 64:ff9b::/96 embeds the v4 address in the low 32 bits.
                return is_private_or_local(&IpAddr::V4(Ipv4Addr::new(o[12], o[13], o[14], o[15])));
            }
            v6.is_multicast() // ff00::/8
                || (o[0] & 0xfe) == 0xfc // unique-local fc00::/7
                || (o[0] == 0xfe && (o[1] & 0xc0) == 0x80) // link-local fe80::/10
                || (o[0] == 0x20 && o[1] == 0x01 && o[2] == 0 && o[3] == 0) // Teredo 2001::/32
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
#[path = "execution_tests.rs"]
mod tests;
