use axum::{body::to_bytes, http::StatusCode};
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
    assert_eq!(
        buffered_body(prepared.response).await,
        Bytes::from_static(br#"{not-json"#)
    );
}

#[tokio::test]
async fn non_success_transform_failures_fall_back_to_upstream_response() {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let prepared = build_buffered_json_response(
        reqwest::StatusCode::BAD_REQUEST,
        &headers,
        Bytes::from_static(br#"{"message":"upstream rejected the request"}"#),
        |_| {
            Err(ProxyError::TransformError(
                "missing error envelope".to_string(),
            ))
        },
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
        |_| {
            Err(ProxyError::Timeout(
                "proxy transform pipeline broke".to_string(),
            ))
        },
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
        |_| {
            Err(ProxyError::TransformError(
                "missing success envelope".to_string(),
            ))
        },
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
