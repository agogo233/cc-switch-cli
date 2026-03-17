use std::{sync::atomic::Ordering, time::Duration};

use axum::http::{HeaderMap, HeaderValue, StatusCode};
use serde_json::json;

use super::{
    bedrock_claude_provider, claude_provider, claude_request_body, spawn_scripted_upstream,
    test_router,
};
use crate::{
    app_config::AppType,
    provider::Provider,
    proxy::{
        forwarder::{ForwardOptions, RequestForwarder},
        types::{OptimizerConfig, RectifierConfig},
    },
};

#[tokio::test]
async fn bedrock_claude_prepare_request_injects_optimizer_and_cache_breakpoints() {
    let (base_url, hits, bodies, server) =
        spawn_scripted_upstream(vec![(StatusCode::OK, json!({"ok": true}))]).await;
    let provider = bedrock_claude_provider("p1", &base_url);
    let (_db, router) = test_router().await;
    let forwarder = RequestForwarder::new(router)
        .expect("create forwarder")
        .with_optimizer_config(OptimizerConfig {
            enabled: true,
            thinking_optimizer: true,
            cache_injection: true,
            cache_ttl: "5m".to_string(),
        });

    let body = json!({
        "model": "anthropic.claude-sonnet-4-5-20250514-v1:0",
        "max_tokens": 32,
        "tools": [{"name": "tool_a"}],
        "system": [{"type": "text", "text": "sys"}],
        "messages": [{
            "role": "assistant",
            "content": [{"type": "text", "text": "hello"}]
        }]
    });

    let response = forwarder
        .forward_buffered_response(
            &AppType::Claude,
            "/v1/messages",
            body,
            &HeaderMap::new(),
            vec![provider],
            ForwardOptions {
                max_retries: 0,
                request_timeout: Some(Duration::from_secs(2)),
                bypass_circuit_breaker: true,
            },
            RectifierConfig::default(),
        )
        .await
        .expect("bedrock claude request should succeed");

    assert_eq!(response.response.status, StatusCode::OK);
    assert_eq!(hits.count.load(Ordering::SeqCst), 1);

    let sent = bodies.lock().await;
    let sent = sent.first().expect("captured upstream request body");
    assert_eq!(sent["thinking"]["type"], "enabled");
    assert_eq!(sent["thinking"]["budget_tokens"], 31);
    assert!(sent["tools"][0].get("cache_control").is_some());
    assert!(sent["system"][0].get("cache_control").is_some());
    assert!(sent["messages"][0]["content"][0]
        .get("cache_control")
        .is_some());

    server.abort();
}

#[tokio::test]
async fn non_bedrock_claude_prepare_request_skips_optimizer_and_cache_injection() {
    let (base_url, hits, bodies, server) =
        spawn_scripted_upstream(vec![(StatusCode::OK, json!({"ok": true}))]).await;
    let provider = claude_provider("p1", &base_url, None);
    let (_db, router) = test_router().await;
    let forwarder = RequestForwarder::new(router)
        .expect("create forwarder")
        .with_optimizer_config(OptimizerConfig {
            enabled: true,
            thinking_optimizer: true,
            cache_injection: true,
            cache_ttl: "5m".to_string(),
        });

    let body = json!({
        "model": "anthropic.claude-sonnet-4-5-20250514-v1:0",
        "max_tokens": 32,
        "tools": [{"name": "tool_a"}],
        "system": [{"type": "text", "text": "sys"}],
        "messages": [{
            "role": "assistant",
            "content": [{"type": "text", "text": "hello"}]
        }]
    });

    let response = forwarder
        .forward_buffered_response(
            &AppType::Claude,
            "/v1/messages",
            body,
            &HeaderMap::new(),
            vec![provider],
            ForwardOptions {
                max_retries: 0,
                request_timeout: Some(Duration::from_secs(2)),
                bypass_circuit_breaker: true,
            },
            RectifierConfig::default(),
        )
        .await
        .expect("regular claude request should succeed");

    assert_eq!(response.response.status, StatusCode::OK);
    assert_eq!(hits.count.load(Ordering::SeqCst), 1);

    let sent = bodies.lock().await;
    let sent = sent.first().expect("captured upstream request body");
    assert!(sent.get("thinking").is_none());
    assert!(sent["tools"][0].get("cache_control").is_none());
    assert!(sent["system"][0].get("cache_control").is_none());
    assert!(sent["messages"][0]["content"][0]
        .get("cache_control")
        .is_none());

    server.abort();
}

#[tokio::test]
async fn claude_prepare_request_appends_claude_code_beta_to_existing_header() {
    let mut headers = HeaderMap::new();
    headers.insert("anthropic-beta", HeaderValue::from_static("existing-beta"));

    let request = build_request(
        &AppType::Claude,
        &claude_provider("p1", "https://example.com", None),
        headers,
    )
    .await;

    assert_eq!(
        request
            .headers()
            .get("anthropic-beta")
            .and_then(|value| value.to_str().ok()),
        Some("claude-code-20250219,existing-beta")
    );
}

#[tokio::test]
async fn claude_prepare_request_sets_defaults_and_filters_blocked_caller_headers() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        HeaderValue::from_static("Bearer caller-token"),
    );
    headers.insert("x-api-key", HeaderValue::from_static("caller-api-key"));
    headers.insert(
        "x-goog-api-key",
        HeaderValue::from_static("caller-goog-key"),
    );
    headers.insert("accept-encoding", HeaderValue::from_static("gzip"));
    headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.10"));
    headers.insert("x-real-ip", HeaderValue::from_static("203.0.113.11"));

    let request = build_request(
        &AppType::Claude,
        &claude_provider("p1", "https://example.com", None),
        headers,
    )
    .await;

    assert_eq!(
        header_value(&request, "anthropic-beta"),
        Some("claude-code-20250219")
    );
    assert_eq!(
        header_value(&request, "anthropic-version"),
        Some("2023-06-01")
    );
    assert_eq!(header_value(&request, "accept-encoding"), Some("identity"));
    assert_eq!(
        header_value(&request, "authorization"),
        Some("Bearer key-p1")
    );
    assert_eq!(header_value(&request, "x-api-key"), Some("key-p1"));
    assert_eq!(header_value(&request, "x-goog-api-key"), None);
    assert_eq!(
        header_value(&request, "x-forwarded-for"),
        Some("203.0.113.10")
    );
    assert_eq!(header_value(&request, "x-real-ip"), Some("203.0.113.11"));
}

#[tokio::test]
async fn non_claude_prepare_request_skips_claude_specific_headers() {
    let request = build_request(
        &AppType::Codex,
        &codex_provider("https://example.com"),
        HeaderMap::new(),
    )
    .await;

    assert_eq!(header_value(&request, "anthropic-beta"), None);
    assert_eq!(header_value(&request, "anthropic-version"), None);
    assert_eq!(
        header_value(&request, "authorization"),
        Some("Bearer codex-key")
    );
}

async fn build_request(
    app_type: &AppType,
    provider: &Provider,
    headers: HeaderMap,
) -> reqwest::Request {
    let (_db, router) = test_router().await;
    let forwarder = RequestForwarder::new(router).expect("create forwarder");

    forwarder
        .prepare_request(
            app_type,
            provider,
            "/v1/messages",
            &claude_request_body(),
            &headers,
            ForwardOptions {
                max_retries: 0,
                request_timeout: Some(Duration::from_secs(2)),
                bypass_circuit_breaker: true,
            },
        )
        .expect("prepare request")
        .build()
        .expect("build request")
}

fn codex_provider(base_url: &str) -> Provider {
    Provider::with_id(
        "codex".to_string(),
        "Codex Provider".to_string(),
        json!({
            "base_url": base_url,
            "apiKey": "codex-key",
        }),
        None,
    )
}

fn header_value<'a>(request: &'a reqwest::Request, name: &str) -> Option<&'a str> {
    request
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}
