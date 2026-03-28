use std::collections::HashMap;
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use super::interpolation::interpolate_document;
use super::models::{HttpMethod, RequestDocument, ResponseArtifact};

/// Maximum response body size to read into memory (10 MB).
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

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
#[allow(dead_code)] // Used in Step 5.
pub async fn execute_request(
    client: &reqwest::Client,
    doc: &RequestDocument,
    vars: &HashMap<String, String>,
    cancel: CancellationToken,
) -> Result<ResponseArtifact, ExecutionError> {
    // 1. Interpolate variables.
    let resolved = interpolate_document(doc, vars).map_err(ExecutionError::Interpolation)?;

    // 2. Build the request.
    let method = to_reqwest_method(resolved.method);
    let mut builder = client.request(method, &resolved.url);

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

    let body_text = if truncated {
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
    })
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
}
