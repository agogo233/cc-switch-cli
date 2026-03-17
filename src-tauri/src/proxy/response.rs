use axum::{body::Body, http, response::Response};
use bytes::Bytes;
use futures::stream::StreamExt;
use serde_json::Value;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use super::{
    error::ProxyError,
    metrics::estimate_tokens_from_bytes,
    providers::{
        streaming::create_anthropic_sse_stream,
        streaming_responses::create_anthropic_sse_stream_from_responses,
    },
};

pub struct PreparedResponse {
    pub response: Response,
    pub stream_completion: Option<StreamCompletion>,
    pub estimated_output_tokens: u64,
    pub upstream_error_summary: Option<String>,
    pub body_bytes: Option<Bytes>,
}

impl PreparedResponse {
    fn buffered(
        response: Response,
        estimated_output_tokens: u64,
        upstream_error_summary: Option<String>,
        body_bytes: Bytes,
    ) -> Self {
        Self {
            response,
            stream_completion: None,
            estimated_output_tokens,
            upstream_error_summary,
            body_bytes: Some(body_bytes),
        }
    }

    fn streaming(response: Response, stream_completion: StreamCompletion) -> Self {
        Self {
            response,
            stream_completion: Some(stream_completion),
            estimated_output_tokens: 0,
            upstream_error_summary: None,
            body_bytes: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct StreamCompletion {
    inner: Arc<Mutex<Option<Result<(), String>>>>,
}

impl StreamCompletion {
    pub fn record_success(&self) {
        let mut outcome = self.inner.lock().expect("lock stream completion");
        if outcome.is_none() {
            *outcome = Some(Ok(()));
        }
    }

    pub fn record_error(&self, message: String) {
        let mut outcome = self.inner.lock().expect("lock stream completion");
        if outcome.is_none() {
            *outcome = Some(Err(message));
        }
    }

    pub fn outcome(&self) -> Option<Result<(), String>> {
        self.inner.lock().expect("lock stream completion").clone()
    }
}

pub fn is_sse_response(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.contains("text/event-stream"))
        .unwrap_or(false)
}

pub async fn build_passthrough_response(
    response: reqwest::Response,
    first_byte_timeout: Option<Duration>,
    idle_timeout: Option<Duration>,
) -> Result<PreparedResponse, ProxyError> {
    let status = response.status();
    let headers = response.headers().clone();
    let mut builder = Response::builder().status(status);
    copy_headers(&mut builder, &headers, false);

    if is_sse_response(&response) {
        let stream_completion = StreamCompletion::default();
        let stream = with_stream_timeouts(
            response.bytes_stream(),
            first_byte_timeout,
            idle_timeout,
            Some(stream_completion.clone()),
        );
        return builder
            .body(Body::from_stream(stream))
            .map(|response| PreparedResponse::streaming(response, stream_completion))
            .map_err(|error| {
                ProxyError::RequestFailed(format!("build streaming response failed: {error}"))
            });
    }

    let body = read_buffered_body(response, first_byte_timeout).await?;
    let upstream_error_summary = if !status.is_success() {
        summarize_upstream_body_bytes(&body)
    } else {
        None
    };
    let estimated_output_tokens = estimate_tokens_from_bytes(&body);
    let response_bytes = body.clone();
    builder
        .body(Body::from(body))
        .map(|response| {
            PreparedResponse::buffered(
                response,
                estimated_output_tokens,
                upstream_error_summary,
                response_bytes,
            )
        })
        .map_err(|error| {
            ProxyError::RequestFailed(format!("build passthrough response failed: {error}"))
        })
}

pub async fn build_json_response<F>(
    response: reqwest::Response,
    first_byte_timeout: Option<Duration>,
    transform: F,
) -> Result<PreparedResponse, ProxyError>
where
    F: FnOnce(Value) -> Result<Value, ProxyError>,
{
    let status = response.status();
    let headers = response.headers().clone();
    let body = read_buffered_body(response, first_byte_timeout).await?;
    build_buffered_json_response_inner(status, &headers, body, transform)
}

pub fn build_buffered_passthrough_response(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: Bytes,
) -> Result<PreparedResponse, ProxyError> {
    let upstream_error_summary = if !status.is_success() {
        summarize_upstream_body_bytes(&body)
    } else {
        None
    };
    let estimated_output_tokens = estimate_tokens_from_bytes(&body);
    let mut builder = Response::builder().status(status);
    copy_headers(&mut builder, headers, false);
    let response_bytes = body.clone();
    builder
        .body(Body::from(body))
        .map(|response| {
            PreparedResponse::buffered(
                response,
                estimated_output_tokens,
                upstream_error_summary,
                response_bytes,
            )
        })
        .map_err(|error| {
            ProxyError::RequestFailed(format!("build passthrough response failed: {error}"))
        })
}

pub fn build_buffered_json_response<F>(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: Bytes,
    transform: F,
) -> Result<PreparedResponse, ProxyError>
where
    F: FnOnce(Value) -> Result<Value, ProxyError>,
{
    build_buffered_json_response_inner(status, headers, body, transform)
}

pub fn build_anthropic_stream_response(
    response: reqwest::Response,
    first_byte_timeout: Option<Duration>,
    idle_timeout: Option<Duration>,
    api_format: &str,
) -> Result<PreparedResponse, ProxyError> {
    let status = response.status();
    let headers = response.headers().clone();
    let mut builder = Response::builder().status(status);
    copy_headers(&mut builder, &headers, true);

    let stream_completion = StreamCompletion::default();
    let timed_stream = with_stream_timeouts(
        response.bytes_stream(),
        first_byte_timeout,
        idle_timeout,
        None,
    );
    let stream: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<Bytes, std::io::Error>> + Send>,
    > = if api_format == "openai_responses" {
        Box::pin(create_anthropic_sse_stream_from_responses(
            timed_stream,
            stream_completion.clone(),
        ))
    } else {
        Box::pin(create_anthropic_sse_stream(
            timed_stream,
            stream_completion.clone(),
        ))
    };
    builder
        .body(Body::from_stream(stream))
        .map(|response| PreparedResponse::streaming(response, stream_completion))
        .map_err(|error| {
            ProxyError::RequestFailed(format!("build anthropic stream response failed: {error}"))
        })
}

fn with_stream_timeouts(
    stream: impl futures::Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    first_byte_timeout: Option<Duration>,
    idle_timeout: Option<Duration>,
    stream_completion: Option<StreamCompletion>,
) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        tokio::pin!(stream);
        let mut is_first_chunk = true;

        while let Some(next) = next_chunk_with_timeout(
            &mut stream,
            if is_first_chunk { first_byte_timeout } else { idle_timeout },
            if is_first_chunk {
                StreamTimeoutPhase::FirstByte
            } else {
                StreamTimeoutPhase::Idle
            },
        ).await {
            match next {
                Ok(chunk) => {
                    is_first_chunk = false;
                    yield Ok(chunk);
                }
                Err(error) => {
                    if let Some(stream_completion) = &stream_completion {
                        stream_completion.record_error(error.to_string());
                    }
                    yield Err(error);
                    return;
                }
            }
        }

        if let Some(stream_completion) = &stream_completion {
            stream_completion.record_success();
        }
    }
}

async fn read_buffered_body(
    response: reqwest::Response,
    timeout_duration: Option<Duration>,
) -> Result<Bytes, ProxyError> {
    match timeout_duration {
        Some(timeout) => match tokio::time::timeout(timeout, response.bytes()).await {
            Ok(result) => result.map_err(|error| {
                ProxyError::RequestFailed(format!("read response body failed: {error}"))
            }),
            Err(_) => Err(ProxyError::Timeout(
                StreamTimeoutPhase::FirstByte.error_message(timeout),
            )),
        },
        None => response.bytes().await.map_err(|error| {
            ProxyError::RequestFailed(format!("read response body failed: {error}"))
        }),
    }
}

async fn next_chunk_with_timeout<S>(
    stream: &mut S,
    timeout_duration: Option<Duration>,
    phase: StreamTimeoutPhase,
) -> Option<Result<Bytes, std::io::Error>>
where
    S: futures::Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
{
    let next = match timeout_duration {
        Some(timeout) => match tokio::time::timeout(timeout, stream.next()).await {
            Ok(next) => next,
            Err(_) => {
                return Some(Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    phase.error_message(timeout),
                )));
            }
        },
        None => stream.next().await,
    };

    next.map(|result| result.map_err(std::io::Error::other))
}

#[derive(Clone, Copy)]
enum StreamTimeoutPhase {
    FirstByte,
    Idle,
}

impl StreamTimeoutPhase {
    fn error_message(self, timeout: Duration) -> String {
        let display_seconds = timeout.as_secs().max(u64::from(!timeout.is_zero()));
        match self {
            StreamTimeoutPhase::FirstByte => {
                format!("stream timeout after {}s", display_seconds)
            }
            StreamTimeoutPhase::Idle => {
                format!("stream idle timeout after {}s", display_seconds)
            }
        }
    }
}

fn copy_headers(
    builder: &mut http::response::Builder,
    headers: &reqwest::header::HeaderMap,
    force_sse_content_type: bool,
) {
    for (key, value) in headers {
        let lower = key.as_str().to_ascii_lowercase();
        if lower == "content-length" || lower == "transfer-encoding" {
            continue;
        }
        if force_sse_content_type && lower == "content-type" {
            continue;
        }
        *builder = std::mem::take(builder).header(key, value);
    }

    if force_sse_content_type {
        *builder = std::mem::take(builder).header("content-type", "text/event-stream");
    }
}

fn summarize_upstream_body_bytes(body: &[u8]) -> Option<String> {
    std::str::from_utf8(body)
        .ok()
        .map(summarize_upstream_body)
        .filter(|summary| !summary.is_empty())
}

fn build_buffered_json_response_inner<F>(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: Bytes,
    transform: F,
) -> Result<PreparedResponse, ProxyError>
where
    F: FnOnce(Value) -> Result<Value, ProxyError>,
{
    let upstream_body: Value = match serde_json::from_slice(&body) {
        Ok(body) => body,
        Err(_) if !status.is_success() => {
            return build_buffered_passthrough_response(status, headers, body);
        }
        Err(error) => {
            return Err(ProxyError::RequestFailed(format!(
                "parse upstream json failed: {error}"
            )));
        }
    };
    let upstream_error_summary = if !status.is_success() {
        summarize_upstream_json_value(&upstream_body)
    } else {
        None
    };
    let response_body = match transform(upstream_body) {
        Ok(body) => body,
        Err(error) if should_passthrough_transform_failure(status, &error) => {
            return build_buffered_passthrough_response(status, headers, body);
        }
        Err(error) => {
            if !status.is_success() {
                return Err(error);
            }
            return Err(ProxyError::RequestFailed(format!(
                "transform upstream json failed: {}",
                proxy_error_message(error)
            )));
        }
    };
    let response_body = match serde_json::to_vec(&response_body) {
        Ok(body) => body,
        Err(error) => {
            return Err(ProxyError::RequestFailed(format!(
                "serialize transformed json failed: {error}"
            )));
        }
    };
    let response_bytes = Bytes::from(response_body);
    let estimated_output_tokens = estimate_tokens_from_bytes(&response_bytes);

    let mut builder = Response::builder().status(status);
    copy_headers(&mut builder, headers, false);
    builder = builder.header("content-type", "application/json");

    builder
        .body(Body::from(response_bytes.clone()))
        .map(|response| {
            PreparedResponse::buffered(
                response,
                estimated_output_tokens,
                upstream_error_summary,
                response_bytes,
            )
        })
        .map_err(|error| {
            ProxyError::RequestFailed(format!("build transformed response failed: {error}"))
        })
}

fn should_passthrough_transform_failure(
    status: reqwest::StatusCode,
    error: &ProxyError,
) -> bool {
    !status.is_success() && matches!(error, ProxyError::TransformError(_))
}

fn proxy_error_message(error: ProxyError) -> String {
    match error {
        ProxyError::ConfigError(message)
        | ProxyError::AuthError(message)
        | ProxyError::RequestFailed(message)
        | ProxyError::TransformError(message)
        | ProxyError::ForwardFailed(message)
        | ProxyError::BindFailed(message)
        | ProxyError::StopFailed(message)
        | ProxyError::ProviderUnhealthy(message)
        | ProxyError::DatabaseError(message)
        | ProxyError::InvalidRequest(message)
        | ProxyError::Timeout(message)
        | ProxyError::Internal(message) => message,
        other => other.to_string(),
    }
}

fn summarize_upstream_body(body: &str) -> String {
    if let Ok(json_body) = serde_json::from_str::<Value>(body) {
        if let Some(summary) = summarize_upstream_json_value(&json_body) {
            return summary;
        }
    }

    summarize_text_for_log(body, 180)
}

fn summarize_upstream_json_value(body: &Value) -> Option<String> {
    if let Some(message) = extract_json_error_message(body) {
        return Some(summarize_text_for_log(&message, 180));
    }

    serde_json::to_string(body)
        .ok()
        .map(|compact_json| summarize_text_for_log(&compact_json, 180))
        .filter(|summary| !summary.is_empty())
}

fn extract_json_error_message(body: &Value) -> Option<String> {
    [
        body.pointer("/error/message"),
        body.pointer("/message"),
        body.pointer("/detail"),
        body.pointer("/error"),
    ]
    .into_iter()
    .flatten()
    .find_map(|value| value.as_str().map(ToString::to_string))
}

fn summarize_text_for_log(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = normalized.trim();

    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let truncated: String = trimmed.chars().take(max_chars).collect();
    let truncated = truncated.trim_end();
    format!("{truncated}...")
}

#[cfg(test)]
mod tests {
    use axum::{
        body::to_bytes,
        http::StatusCode,
    };
    use bytes::Bytes;
    use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
    use serde_json::json;

    use super::*;

    async fn buffered_body(response: Response) -> Bytes {
        to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read buffered response body")
    }

    #[tokio::test]
    async fn non_success_parse_failures_fall_back_to_upstream_response() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let prepared = build_buffered_json_response(
            reqwest::StatusCode::BAD_REQUEST,
            &headers,
            Bytes::from_static(br#"{not-json"#),
            |_| Ok(json!({"type": "error"})),
        )
        .expect("fallback to raw upstream response");

        assert_eq!(prepared.response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            prepared
                .response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
        assert_eq!(buffered_body(prepared.response).await, Bytes::from_static(br#"{not-json"#));
    }

    #[tokio::test]
    async fn non_success_transform_failures_fall_back_to_upstream_response() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let prepared = build_buffered_json_response(
            reqwest::StatusCode::BAD_REQUEST,
            &headers,
            Bytes::from_static(br#"{"message":"upstream rejected the request"}"#),
            |_| Err(ProxyError::TransformError("missing error envelope".to_string())),
        )
        .expect("fallback to raw upstream response");

        assert_eq!(prepared.response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            buffered_body(prepared.response).await,
            Bytes::from_static(br#"{"message":"upstream rejected the request"}"#)
        );
    }

    #[test]
    fn non_success_non_transform_failures_preserve_original_proxy_error() {
        let headers = HeaderMap::new();
        let result = build_buffered_json_response(
            reqwest::StatusCode::BAD_REQUEST,
            &headers,
            Bytes::from_static(br#"{"message":"upstream rejected the request"}"#),
            |_| Err(ProxyError::Timeout("proxy transform pipeline broke".to_string())),
        );

        match result {
            Ok(_) => panic!("non-transform errors must not fall back to upstream passthrough"),
            Err(ProxyError::Timeout(message)) => {
                assert_eq!(message, "proxy transform pipeline broke");
            }
            Err(other) => panic!("expected original proxy error, got {other:?}"),
        }
    }

    #[test]
    fn success_parse_failures_use_proxy_request_failed_errors() {
        let headers = HeaderMap::new();
        let result = build_buffered_json_response(
            reqwest::StatusCode::OK,
            &headers,
            Bytes::from_static(br#"{not-json"#),
            |_| Ok(json!({"type": "message"})),
        );

        match result {
            Ok(_) => panic!("success responses should still fail on malformed upstream json"),
            Err(ProxyError::RequestFailed(message)) => {
                assert!(message.contains("parse upstream json failed"));
            }
            Err(other) => panic!("expected request failed error, got {other:?}"),
        }
    }

    #[test]
    fn success_transform_failures_use_proxy_request_failed_errors() {
        let headers = HeaderMap::new();
        let result = build_buffered_json_response(
            reqwest::StatusCode::OK,
            &headers,
            Bytes::from_static(br#"{"message":"upstream accepted the request"}"#),
            |_| Err(ProxyError::TransformError("missing success envelope".to_string())),
        );

        match result {
            Ok(_) => panic!("success responses must surface transform failures as proxy errors"),
            Err(ProxyError::RequestFailed(message)) => {
                assert!(message.contains("transform upstream json failed"));
                assert!(message.contains("missing success envelope"));
            }
            Err(other) => panic!("expected request failed error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_success_standard_json_errors_can_still_transform() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let prepared = build_buffered_json_response(
            reqwest::StatusCode::BAD_REQUEST,
            &headers,
            Bytes::from_static(
                br#"{"error":{"message":"upstream rejected the request","type":"invalid_request_error"}}"#,
            ),
            |body| {
                assert_eq!(
                    body,
                    json!({
                        "error": {
                            "message": "upstream rejected the request",
                            "type": "invalid_request_error"
                        }
                    })
                );
                Ok(json!({
                    "type": "error",
                    "error": {
                        "type": "invalid_request_error",
                        "message": "upstream rejected the request"
                    }
                }))
            },
        )
        .expect("standard upstream json errors should still transform");

        assert_eq!(prepared.response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            buffered_body(prepared.response).await,
            Bytes::from_static(
                br#"{"error":{"message":"upstream rejected the request","type":"invalid_request_error"},"type":"error"}"#,
            )
        );
    }
}
