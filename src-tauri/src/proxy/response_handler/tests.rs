use std::{collections::HashMap, sync::Arc, time::Duration};

use axum::{
    body::{to_bytes, Body},
    http::StatusCode,
    response::Response,
};
use bytes::Bytes;
use tokio::sync::RwLock;

use crate::{
    database::Database,
    proxy::{provider_router::ProviderRouter, types::ProxyConfig},
};

use super::*;

fn test_state() -> ProxyServerState {
    let db = Arc::new(Database::memory().expect("memory db"));
    ProxyServerState {
        db: db.clone(),
        config: Arc::new(RwLock::new(ProxyConfig::default())),
        status: Arc::new(RwLock::new(crate::proxy::types::ProxyStatus::default())),
        start_time: Arc::new(RwLock::new(None)),
        current_providers: Arc::new(RwLock::new(HashMap::new())),
        provider_router: Arc::new(ProviderRouter::new(db)),
    }
}

async fn settle_tasks() {
    tokio::time::sleep(Duration::from_millis(10)).await;
}

#[tokio::test]
async fn buffered_failures_still_accumulate_output_tokens() {
    let state = test_state();
    state.record_request_start().await;

    let response = PreparedResponse {
        response: Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .body(Body::from("upstream failure payload"))
            .expect("response"),
        stream_completion: None,
        estimated_output_tokens: 9,
        upstream_error_summary: None,
        body_bytes: None,
    };

    let _ = ResponseHandler::finish_buffered(
        &state,
        Ok(response),
        reqwest::StatusCode::BAD_GATEWAY,
        None,
        None,
    )
    .await;

    let snapshot = state.snapshot_status().await;
    assert_eq!(snapshot.failed_requests, 1);
    assert_eq!(snapshot.estimated_output_tokens_total, 9);
}

#[tokio::test]
async fn interrupted_streams_keep_partial_output_estimate() {
    let state = test_state();
    state.record_request_start().await;

    let stream = async_stream::stream! {
        yield Ok::<Bytes, std::io::Error>(Bytes::from_static(b"partial output"));
        yield Err::<Bytes, std::io::Error>(std::io::Error::other("boom"));
    };
    let response = PreparedResponse {
        response: Response::builder()
            .status(StatusCode::OK)
            .body(Body::from_stream(stream))
            .expect("response"),
        stream_completion: None,
        estimated_output_tokens: 0,
        upstream_error_summary: None,
        body_bytes: None,
    };

    let response = ResponseHandler::finish_streaming(
        &state,
        Ok(response),
        reqwest::StatusCode::OK,
        None,
        None,
    )
    .await;
    let _ = to_bytes(response.into_body(), usize::MAX).await;
    settle_tasks().await;

    let snapshot = state.snapshot_status().await;
    assert_eq!(snapshot.failed_requests, 1);
    assert!(snapshot.estimated_output_tokens_total > 0);
}

#[tokio::test]
async fn non_success_streams_accumulate_output_tokens_after_body_drains() {
    let state = test_state();
    state.record_request_start().await;

    let response = PreparedResponse {
        response: Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from("bad request payload"))
            .expect("response"),
        stream_completion: None,
        estimated_output_tokens: 0,
        upstream_error_summary: None,
        body_bytes: None,
    };

    let response = ResponseHandler::finish_streaming(
        &state,
        Ok(response),
        reqwest::StatusCode::BAD_REQUEST,
        None,
        None,
    )
    .await;
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    settle_tasks().await;

    let snapshot = state.snapshot_status().await;
    assert_eq!(snapshot.failed_requests, 1);
    assert!(snapshot.estimated_output_tokens_total > 0);
}

#[tokio::test]
async fn buffered_success_streaming_responses_do_not_record_termination_error() {
    let state = test_state();
    state.record_request_start().await;

    let response = PreparedResponse {
        response: Response::builder()
            .status(StatusCode::OK)
            .body(Body::from("transformed buffered fallback"))
            .expect("response"),
        stream_completion: None,
        estimated_output_tokens: 0,
        upstream_error_summary: None,
        body_bytes: Some(Bytes::from_static(b"transformed buffered fallback")),
    };

    let response = ResponseHandler::finish_streaming(
        &state,
        Ok(response),
        reqwest::StatusCode::OK,
        None,
        None,
    )
    .await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    assert_eq!(body, Bytes::from_static(b"transformed buffered fallback"));
    settle_tasks().await;

    let snapshot = state.snapshot_status().await;
    assert_eq!(snapshot.success_requests, 1);
    assert_eq!(snapshot.failed_requests, 0);
    assert!(snapshot.last_error.is_none());
    assert!(snapshot.estimated_output_tokens_total > 0);
}
