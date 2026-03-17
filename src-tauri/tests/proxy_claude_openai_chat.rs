use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use cc_switch_lib::{Database, Provider, ProviderMeta, ProxyService};
use serde_json::{json, Value};
use tokio::sync::Mutex;

async fn bind_test_listener() -> tokio::net::TcpListener {
    let mut last_error = None;
    for _ in 0..20 {
        match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => return listener,
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }
    }

    panic!(
        "bind upstream listener: {:?}",
        last_error.expect("listener bind should produce an error")
    );
}

#[derive(Clone, Default)]
struct UpstreamState {
    request_body: Arc<Mutex<Option<Value>>>,
    authorization: Arc<Mutex<Option<String>>>,
    api_key: Arc<Mutex<Option<String>>>,
    anthropic_version: Arc<Mutex<Option<String>>>,
    anthropic_beta: Arc<Mutex<Option<String>>>,
    forwarded_for: Arc<Mutex<Option<String>>>,
}

async fn handle_chat_completions(
    State(state): State<UpstreamState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    *state.request_body.lock().await = Some(body);
    *state.authorization.lock().await = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    *state.api_key.lock().await = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    *state.anthropic_version.lock().await = headers
        .get("anthropic-version")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    *state.anthropic_beta.lock().await = headers
        .get("anthropic-beta")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    *state.forwarded_for.lock().await = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    (
        StatusCode::CREATED,
        [("x-upstream-trace", "claude-openai-chat")],
        Json(json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "created": 123,
            "model": "gpt-4o-mini",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "hello back"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 11,
                "completion_tokens": 7,
                "total_tokens": 18
            }
        })),
    )
}

async fn handle_chat_completions_priced(
    State(state): State<UpstreamState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    *state.request_body.lock().await = Some(body);
    *state.authorization.lock().await = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    *state.api_key.lock().await = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    *state.anthropic_version.lock().await = headers
        .get("anthropic-version")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    *state.anthropic_beta.lock().await = headers
        .get("anthropic-beta")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    *state.forwarded_for.lock().await = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    (
        StatusCode::CREATED,
        [("x-upstream-trace", "claude-openai-chat-priced")],
        Json(json!({
            "id": "chatcmpl-priced",
            "object": "chat.completion",
            "created": 123,
            "model": "gpt-5.2",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "hello back"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 11,
                "completion_tokens": 7,
                "total_tokens": 18
            }
        })),
    )
}

async fn handle_responses(
    State(state): State<UpstreamState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    *state.request_body.lock().await = Some(body);
    *state.authorization.lock().await = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    *state.api_key.lock().await = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    *state.anthropic_version.lock().await = headers
        .get("anthropic-version")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    *state.anthropic_beta.lock().await = headers
        .get("anthropic-beta")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    *state.forwarded_for.lock().await = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    (
        StatusCode::OK,
        [("x-upstream-trace", "claude-openai-responses")],
        Json(json!({
            "id": "resp_test_123",
            "object": "response",
            "status": "completed",
            "model": "gpt-4.1-mini",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": "hello from responses"
                }]
            }],
            "usage": {
                "input_tokens": 13,
                "output_tokens": 5,
                "total_tokens": 18
            }
        })),
    )
}

async fn handle_invalid_chat_completions() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        "{not-json",
    )
}

async fn handle_chat_completions_error() -> impl IntoResponse {
    (
        StatusCode::BAD_REQUEST,
        [("x-upstream-trace", "claude-openai-chat-error")],
        Json(json!({
            "error": {
                "message": "upstream rejected the request",
                "type": "invalid_request_error"
            }
        })),
    )
}

async fn handle_chat_completions_non_standard_json_error() -> impl IntoResponse {
    (
        StatusCode::BAD_REQUEST,
        [
            ("content-type", "application/json"),
            ("x-upstream-trace", "claude-openai-chat-non-standard-error"),
        ],
        Json(json!({
            "message": "upstream rejected the request"
        })),
    )
}

async fn handle_chat_completions_plain_error() -> impl IntoResponse {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [
            ("content-type", "text/plain; charset=utf-8"),
            ("x-upstream-trace", "claude-openai-chat-plain-error"),
        ],
        "upstream rate limit",
    )
}

fn provider_meta_from_json(value: Value) -> ProviderMeta {
    serde_json::from_value(value).expect("parse provider meta")
}

fn request_log_insert_lines(db: &Database) -> Vec<String> {
    db.export_sql_string()
        .expect("export sql string")
        .lines()
        .filter(|line| line.contains("INSERT INTO \"proxy_request_logs\""))
        .map(|line| line.to_string())
        .collect()
}

async fn wait_for_request_log_lines(db: &Database, expected: usize) -> Vec<String> {
    for _ in 0..20 {
        let lines = request_log_insert_lines(db);
        if lines.len() >= expected {
            return lines;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    request_log_insert_lines(db)
}

fn parse_insert_values(line: &str) -> Vec<String> {
    let values_start = line.find("VALUES").expect("insert values keyword");
    let start = line[values_start..]
        .find('(')
        .map(|offset| values_start + offset + 1)
        .expect("insert values start");
    let end = line.rfind(')').expect("insert values end");
    let values = &line[start..end];

    let mut parsed = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = values.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                if in_quotes && chars.peek() == Some(&'\'') {
                    current.push('\'');
                    chars.next();
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                parsed.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    parsed.push(current.trim().to_string());
    parsed
}

async fn capture_openai_chat_upstream_body(
    provider_id: &str,
    meta: ProviderMeta,
    request_body: Value,
) -> Value {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/chat/completions", post(handle_chat_completions))
        .with_state(upstream_state.clone());

    let upstream_listener = bind_test_listener().await;
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("read upstream address");
    let upstream_handle = tokio::spawn(async move {
        let _ = axum::serve(upstream_listener, upstream_router).await;
    });

    let db = Arc::new(Database::memory().expect("create memory database"));
    let provider = Provider {
        id: provider_id.to_string(),
        name: "Claude OpenAI Chat Cache Test".to_string(),
        settings_config: json!({
            "env": {
                "ANTHROPIC_BASE_URL": format!("http://{}", upstream_addr),
                "ANTHROPIC_API_KEY": "sk-test-claude"
            }
        }),
        website_url: None,
        category: Some("claude".to_string()),
        created_at: None,
        sort_index: None,
        notes: None,
        meta: Some(meta),
        icon: None,
        icon_color: None,
        in_failover_queue: false,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let service = ProxyService::new(db);
    let mut config = service.get_config().await.expect("read proxy config");
    config.listen_port = 0;
    service
        .update_config(&config)
        .await
        .expect("update proxy config");

    let proxy = service.start().await.expect("start proxy service");
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/v1/messages",
            proxy.address, proxy.port
        ))
        .header("anthropic-version", "2023-06-01")
        .json(&request_body)
        .send()
        .await
        .expect("send request to proxy");

    assert!(response.status().is_success());
    let _: Value = response.json().await.expect("parse proxy response");

    let upstream_body = upstream_state
        .request_body
        .lock()
        .await
        .clone()
        .expect("upstream should receive request body");

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();

    upstream_body
}

#[tokio::test]
async fn cache_openai_chat_uses_meta_prompt_cache_key_override() {
    let upstream_body = capture_openai_chat_upstream_body(
        "provider-fallback-id",
        provider_meta_from_json(json!({
            "apiFormat": "openai_chat",
            "promptCacheKey": "custom-cache-key"
        })),
        json!({
            "model": "claude-3-7-sonnet",
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }),
    )
    .await;

    assert_eq!(
        upstream_body
            .get("prompt_cache_key")
            .and_then(|value| value.as_str()),
        Some("custom-cache-key")
    );
}

#[tokio::test]
async fn cache_openai_chat_falls_back_to_provider_id() {
    let upstream_body = capture_openai_chat_upstream_body(
        "provider-fallback-id",
        provider_meta_from_json(json!({
            "apiFormat": "openai_chat"
        })),
        json!({
            "model": "claude-3-7-sonnet",
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }),
    )
    .await;

    assert_eq!(
        upstream_body
            .get("prompt_cache_key")
            .and_then(|value| value.as_str()),
        Some("provider-fallback-id")
    );
}

#[tokio::test]
async fn cache_openai_chat_preserves_cache_control_metadata() {
    let upstream_body = capture_openai_chat_upstream_body(
        "provider-fallback-id",
        provider_meta_from_json(json!({
            "apiFormat": "openai_chat"
        })),
        json!({
            "model": "claude-3-7-sonnet",
            "max_tokens": 64,
            "system": [{
                "type": "text",
                "text": "system prompt",
                "cache_control": { "type": "ephemeral" }
            }],
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": "hello",
                    "cache_control": { "type": "ephemeral", "ttl": "5m" }
                }]
            }],
            "tools": [{
                "name": "get_weather",
                "description": "Get weather",
                "input_schema": { "type": "object" },
                "cache_control": { "type": "ephemeral" }
            }]
        }),
    )
    .await;

    assert_eq!(
        upstream_body
            .pointer("/messages/0/cache_control/type")
            .and_then(|value| value.as_str()),
        Some("ephemeral")
    );
    assert_eq!(
        upstream_body
            .pointer("/messages/1/content/0/cache_control/type")
            .and_then(|value| value.as_str()),
        Some("ephemeral")
    );
    assert_eq!(
        upstream_body
            .pointer("/messages/1/content/0/cache_control/ttl")
            .and_then(|value| value.as_str()),
        Some("5m")
    );
    assert_eq!(
        upstream_body
            .pointer("/tools/0/cache_control/type")
            .and_then(|value| value.as_str()),
        Some("ephemeral")
    );
}

#[tokio::test]
async fn proxy_claude_openai_chat_transforms_request_and_response() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/chat/completions", post(handle_chat_completions))
        .with_state(upstream_state.clone());

    let upstream_listener = bind_test_listener().await;
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("read upstream address");
    let upstream_handle = tokio::spawn(async move {
        let _ = axum::serve(upstream_listener, upstream_router).await;
    });

    let db = Arc::new(Database::memory().expect("create memory database"));
    let provider = Provider {
        id: "claude-openai-chat".to_string(),
        name: "Claude OpenAI Chat".to_string(),
        settings_config: json!({
            "env": {
                "ANTHROPIC_BASE_URL": format!("http://{}", upstream_addr),
                "ANTHROPIC_API_KEY": "sk-test-claude",
                "ANTHROPIC_DEFAULT_SONNET_MODEL": "mapped-sonnet"
            }
        }),
        website_url: None,
        category: Some("claude".to_string()),
        created_at: None,
        sort_index: None,
        notes: None,
        meta: Some(ProviderMeta {
            api_format: Some("openai_chat".to_string()),
            ..ProviderMeta::default()
        }),
        icon: None,
        icon_color: None,
        in_failover_queue: false,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let service = ProxyService::new(db);
    let mut config = service.get_config().await.expect("read proxy config");
    config.listen_port = 0;
    service
        .update_config(&config)
        .await
        .expect("update proxy config");

    let proxy = service.start().await.expect("start proxy service");
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/v1/messages",
            proxy.address, proxy.port
        ))
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "prompt-caching-2024-07-31")
        .header("x-forwarded-for", "203.0.113.9")
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert_eq!(
        response.status(),
        reqwest::StatusCode::CREATED,
        "proxy should preserve the upstream status"
    );
    assert_eq!(
        response
            .headers()
            .get("x-upstream-trace")
            .and_then(|v| v.to_str().ok()),
        Some("claude-openai-chat"),
        "proxy should preserve the upstream headers"
    );
    let body: Value = response.json().await.expect("parse proxy response");

    let upstream_body = upstream_state
        .request_body
        .lock()
        .await
        .clone()
        .expect("upstream should receive request body");
    assert_eq!(
        upstream_body.get("model").and_then(|v| v.as_str()),
        Some("mapped-sonnet")
    );
    assert_eq!(
        upstream_body
            .pointer("/messages/0/role")
            .and_then(|v| v.as_str()),
        Some("user")
    );
    assert_eq!(
        upstream_body
            .pointer("/messages/0/content")
            .and_then(|v| v.as_str()),
        Some("hello")
    );

    assert_eq!(
        upstream_state.authorization.lock().await.as_deref(),
        Some("Bearer sk-test-claude")
    );
    assert_eq!(
        upstream_state.api_key.lock().await.as_deref(),
        Some("sk-test-claude")
    );
    assert_eq!(
        upstream_state.anthropic_version.lock().await.as_deref(),
        Some("2023-06-01")
    );
    assert_eq!(
        upstream_state.anthropic_beta.lock().await.as_deref(),
        Some("claude-code-20250219,prompt-caching-2024-07-31")
    );
    assert_eq!(
        upstream_state.forwarded_for.lock().await.as_deref(),
        Some("203.0.113.9")
    );

    assert_eq!(body.get("role").and_then(|v| v.as_str()), Some("assistant"));
    assert_eq!(
        body.pointer("/content/0/type").and_then(|v| v.as_str()),
        Some("text")
    );
    assert_eq!(
        body.pointer("/content/0/text").and_then(|v| v.as_str()),
        Some("hello back")
    );
    assert_eq!(
        body.pointer("/usage/input_tokens").and_then(|v| v.as_u64()),
        Some(11)
    );
    assert_eq!(
        body.pointer("/usage/output_tokens")
            .and_then(|v| v.as_u64()),
        Some(7)
    );

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_claude_openai_responses_transforms_request_and_response() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/responses", post(handle_responses))
        .with_state(upstream_state.clone());

    let upstream_listener = bind_test_listener().await;
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("read upstream address");
    let upstream_handle = tokio::spawn(async move {
        let _ = axum::serve(upstream_listener, upstream_router).await;
    });

    let db = Arc::new(Database::memory().expect("create memory database"));
    let provider = Provider {
        id: "claude-openai-responses".to_string(),
        name: "Claude OpenAI Responses".to_string(),
        settings_config: json!({
            "env": {
                "ANTHROPIC_BASE_URL": format!("http://{}", upstream_addr),
                "ANTHROPIC_API_KEY": "sk-test-claude",
                "ANTHROPIC_DEFAULT_SONNET_MODEL": "mapped-sonnet"
            }
        }),
        website_url: None,
        category: Some("claude".to_string()),
        created_at: None,
        sort_index: None,
        notes: None,
        meta: Some(ProviderMeta {
            api_format: Some("openai_responses".to_string()),
            ..ProviderMeta::default()
        }),
        icon: None,
        icon_color: None,
        in_failover_queue: false,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let service = ProxyService::new(db);
    let mut config = service.get_config().await.expect("read proxy config");
    config.listen_port = 0;
    service
        .update_config(&config)
        .await
        .expect("update proxy config");

    let proxy = service.start().await.expect("start proxy service");
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/v1/messages",
            proxy.address, proxy.port
        ))
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "prompt-caching-2024-07-31")
        .header("x-forwarded-for", "203.0.113.10")
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": "hello"
                }]
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-upstream-trace")
            .and_then(|v| v.to_str().ok()),
        Some("claude-openai-responses")
    );
    let body: Value = response.json().await.expect("parse proxy response");

    let upstream_body = upstream_state
        .request_body
        .lock()
        .await
        .clone()
        .expect("upstream should receive request body");
    assert_eq!(
        upstream_body.get("model").and_then(|v| v.as_str()),
        Some("mapped-sonnet")
    );
    assert_eq!(
        upstream_body
            .pointer("/input/0/role")
            .and_then(|v| v.as_str()),
        Some("user")
    );
    assert_eq!(
        upstream_body
            .pointer("/input/0/content/0/type")
            .and_then(|v| v.as_str()),
        Some("input_text")
    );
    assert_eq!(
        upstream_body
            .pointer("/input/0/content/0/text")
            .and_then(|v| v.as_str()),
        Some("hello")
    );
    assert_eq!(
        upstream_body
            .get("max_output_tokens")
            .and_then(|v| v.as_u64()),
        Some(64)
    );
    assert!(upstream_body.get("messages").is_none());

    assert_eq!(
        upstream_state.authorization.lock().await.as_deref(),
        Some("Bearer sk-test-claude")
    );
    assert_eq!(
        upstream_state.api_key.lock().await.as_deref(),
        Some("sk-test-claude")
    );
    assert_eq!(
        upstream_state.anthropic_version.lock().await.as_deref(),
        Some("2023-06-01")
    );
    assert_eq!(
        upstream_state.anthropic_beta.lock().await.as_deref(),
        Some("claude-code-20250219,prompt-caching-2024-07-31")
    );
    assert_eq!(
        upstream_state.forwarded_for.lock().await.as_deref(),
        Some("203.0.113.10")
    );

    assert_eq!(body.get("role").and_then(|v| v.as_str()), Some("assistant"));
    assert_eq!(
        body.pointer("/content/0/type").and_then(|v| v.as_str()),
        Some("text")
    );
    assert_eq!(
        body.pointer("/content/0/text").and_then(|v| v.as_str()),
        Some("hello from responses")
    );
    assert_eq!(
        body.pointer("/usage/input_tokens").and_then(|v| v.as_u64()),
        Some(13)
    );
    assert_eq!(
        body.pointer("/usage/output_tokens")
            .and_then(|v| v.as_u64()),
        Some(5)
    );

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_claude_openai_chat_non_success_error_is_transformed_to_anthropic_shape() {
    let upstream_router = Router::new().route(
        "/v1/chat/completions",
        post(handle_chat_completions_error),
    );

    let upstream_listener = bind_test_listener().await;
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("read upstream address");
    let upstream_handle = tokio::spawn(async move {
        let _ = axum::serve(upstream_listener, upstream_router).await;
    });

    let db = Arc::new(Database::memory().expect("create memory database"));
    let provider = Provider {
        id: "claude-openai-chat-error".to_string(),
        name: "Claude OpenAI Chat Error".to_string(),
        settings_config: json!({
            "env": {
                "ANTHROPIC_BASE_URL": format!("http://{}", upstream_addr),
                "ANTHROPIC_API_KEY": "sk-test-claude"
            }
        }),
        website_url: None,
        category: Some("claude".to_string()),
        created_at: None,
        sort_index: None,
        notes: None,
        meta: Some(ProviderMeta {
            api_format: Some("openai_chat".to_string()),
            ..ProviderMeta::default()
        }),
        icon: None,
        icon_color: None,
        in_failover_queue: false,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let service = ProxyService::new(db.clone());
    let mut config = service.get_config().await.expect("read proxy config");
    config.listen_port = 0;
    service
        .update_config(&config)
        .await
        .expect("update proxy config");

    let proxy = service.start().await.expect("start proxy service");
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/v1/messages",
            proxy.address, proxy.port
        ))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "metadata": {
                "session_id": "claude-session-non-success"
            },
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);

    let body: Value = response.json().await.expect("parse error response");
    assert_eq!(body.get("type").and_then(|v| v.as_str()), Some("error"));
    assert_eq!(
        body.pointer("/error/type").and_then(|v| v.as_str()),
        Some("invalid_request_error")
    );
    assert_eq!(
        body.pointer("/error/message").and_then(|v| v.as_str()),
        Some("upstream rejected the request")
    );

    let status = service.get_status().await;
    assert_eq!(
        status.last_error,
        Some("upstream returned 400: upstream rejected the request".to_string())
    );

    let log_lines = wait_for_request_log_lines(&db, 1).await;
    assert_eq!(log_lines.len(), 1);
    let log_values = parse_insert_values(&log_lines[0]);
    assert_eq!(log_values[1], "claude-openai-chat-error");
    assert_eq!(log_values[3], "claude-3-7-sonnet");
    assert_eq!(log_values[4], "claude-3-7-sonnet");
    assert_eq!(log_values[17], "400");
    assert_eq!(log_values[19], "claude-session-non-success");
    assert!(log_values[18].contains("upstream rejected the request"));

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_claude_openai_chat_non_standard_json_error_preserves_upstream_status_and_body() {
    let upstream_router =
        Router::new().route("/v1/chat/completions", post(handle_chat_completions_non_standard_json_error));

    let upstream_listener = bind_test_listener().await;
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("read upstream address");
    let upstream_handle = tokio::spawn(async move {
        let _ = axum::serve(upstream_listener, upstream_router).await;
    });

    let db = Arc::new(Database::memory().expect("create memory database"));
    let provider = Provider {
        id: "claude-openai-chat-non-standard-error".to_string(),
        name: "Claude OpenAI Chat Non Standard Error".to_string(),
        settings_config: json!({
            "env": {
                "ANTHROPIC_BASE_URL": format!("http://{}", upstream_addr),
                "ANTHROPIC_API_KEY": "sk-test-claude"
            }
        }),
        website_url: None,
        category: Some("claude".to_string()),
        created_at: None,
        sort_index: None,
        notes: None,
        meta: Some(ProviderMeta {
            api_format: Some("openai_chat".to_string()),
            ..ProviderMeta::default()
        }),
        icon: None,
        icon_color: None,
        in_failover_queue: false,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let service = ProxyService::new(db.clone());
    let mut config = service.get_config().await.expect("read proxy config");
    config.listen_port = 0;
    service
        .update_config(&config)
        .await
        .expect("update proxy config");

    let proxy = service.start().await.expect("start proxy service");
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/v1/messages",
            proxy.address, proxy.port
        ))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let body: Value = response.json().await.expect("parse passthrough error body");
    assert_eq!(
        body,
        json!({
            "message": "upstream rejected the request"
        })
    );

    let status = service.get_status().await;
    assert_eq!(
        status.last_error,
        Some("upstream returned 400: upstream rejected the request".to_string())
    );

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_claude_openai_chat_non_json_error_body_passthrough_preserves_status_and_body() {
    let upstream_router = Router::new().route(
        "/v1/chat/completions",
        post(handle_chat_completions_plain_error),
    );

    let upstream_listener = bind_test_listener().await;
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("read upstream address");
    let upstream_handle = tokio::spawn(async move {
        let _ = axum::serve(upstream_listener, upstream_router).await;
    });

    let db = Arc::new(Database::memory().expect("create memory database"));
    let provider = Provider {
        id: "claude-openai-chat-plain-error".to_string(),
        name: "Claude OpenAI Chat Plain Error".to_string(),
        settings_config: json!({
            "env": {
                "ANTHROPIC_BASE_URL": format!("http://{}", upstream_addr),
                "ANTHROPIC_API_KEY": "sk-test-claude"
            }
        }),
        website_url: None,
        category: Some("claude".to_string()),
        created_at: None,
        sort_index: None,
        notes: None,
        meta: Some(ProviderMeta {
            api_format: Some("openai_chat".to_string()),
            ..ProviderMeta::default()
        }),
        icon: None,
        icon_color: None,
        in_failover_queue: false,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let service = ProxyService::new(db);
    let mut config = service.get_config().await.expect("read proxy config");
    config.listen_port = 0;
    service
        .update_config(&config)
        .await
        .expect("update proxy config");

    let proxy = service.start().await.expect("start proxy service");
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/v1/messages",
            proxy.address, proxy.port
        ))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert_eq!(response.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(
        response.text().await.expect("read passthrough error body"),
        "upstream rate limit"
    );

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_claude_openai_chat_success_logs_request_with_session_id_and_usage() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/chat/completions", post(handle_chat_completions_priced))
        .with_state(upstream_state);

    let upstream_listener = bind_test_listener().await;
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("read upstream address");
    let upstream_handle = tokio::spawn(async move {
        let _ = axum::serve(upstream_listener, upstream_router).await;
    });

    let db = Arc::new(Database::memory().expect("create memory database"));
    let provider = Provider {
        id: "claude-openai-chat-log-success".to_string(),
        name: "Claude OpenAI Chat Log Success".to_string(),
        settings_config: json!({
            "env": {
                "ANTHROPIC_BASE_URL": format!("http://{}", upstream_addr),
                "ANTHROPIC_API_KEY": "sk-test-claude"
            }
        }),
        website_url: None,
        category: Some("claude".to_string()),
        created_at: None,
        sort_index: None,
        notes: None,
        meta: Some(ProviderMeta {
            api_format: Some("openai_chat".to_string()),
            ..ProviderMeta::default()
        }),
        icon: None,
        icon_color: None,
        in_failover_queue: false,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");
    db.set_default_cost_multiplier("claude", "2")
        .await
        .expect("set default cost multiplier");

    let service = ProxyService::new(db.clone());
    let mut config = service.get_config().await.expect("read proxy config");
    config.listen_port = 0;
    service
        .update_config(&config)
        .await
        .expect("update proxy config");

    let proxy = service.start().await.expect("start proxy service");
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/v1/messages",
            proxy.address, proxy.port
        ))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "max_tokens": 64,
            "metadata": {
                "session_id": "claude-session-success"
            },
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let _: Value = response.json().await.expect("parse proxy response");

    let log_lines = wait_for_request_log_lines(&db, 1).await;
    assert_eq!(log_lines.len(), 1);
    let log_values = parse_insert_values(&log_lines[0]);
    assert_eq!(log_values[1], "claude-openai-chat-log-success");
    assert_eq!(log_values[3], "gpt-5.2");
    assert_eq!(log_values[4], "claude-3-7-sonnet");
    assert_eq!(log_values[5], "11");
    assert_eq!(log_values[6], "7");
    assert_eq!(log_values[9], "0.00001925");
    assert_eq!(log_values[10], "0.000098");
    assert_eq!(log_values[13], "0.0002345");
    assert_eq!(log_values[17], "201");
    assert_eq!(log_values[19], "claude-session-success");
    assert_eq!(log_values[22], "2");

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_claude_buffered_transform_failure_logs_error_request_with_session_id() {
    let upstream_router = Router::new().route(
        "/v1/chat/completions",
        post(handle_invalid_chat_completions),
    );

    let upstream_listener = bind_test_listener().await;
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("read upstream address");
    let upstream_handle = tokio::spawn(async move {
        let _ = axum::serve(upstream_listener, upstream_router).await;
    });

    let db = Arc::new(Database::memory().expect("create memory database"));
    let provider = Provider {
        id: "claude-openai-chat-log-failure".to_string(),
        name: "Claude OpenAI Chat Log Failure".to_string(),
        settings_config: json!({
            "env": {
                "ANTHROPIC_BASE_URL": format!("http://{}", upstream_addr),
                "ANTHROPIC_API_KEY": "sk-test-claude"
            }
        }),
        website_url: None,
        category: Some("claude".to_string()),
        created_at: None,
        sort_index: None,
        notes: None,
        meta: Some(ProviderMeta {
            api_format: Some("openai_chat".to_string()),
            ..ProviderMeta::default()
        }),
        icon: None,
        icon_color: None,
        in_failover_queue: false,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let service = ProxyService::new(db.clone());
    let mut config = service.get_config().await.expect("read proxy config");
    config.listen_port = 0;
    service
        .update_config(&config)
        .await
        .expect("update proxy config");

    let proxy = service.start().await.expect("start proxy service");
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/v1/messages",
            proxy.address, proxy.port
        ))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "max_tokens": 64,
            "metadata": {
                "session_id": "claude-session-failure"
            },
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);

    let body: Value = response.json().await.expect("parse proxy error response");
    assert_eq!(
        body.pointer("/error/type").and_then(|value| value.as_str()),
        Some("proxy_error")
    );
    assert!(
        body.pointer("/error/message")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .contains("parse upstream json failed")
    );

    let log_lines = wait_for_request_log_lines(&db, 1).await;
    assert_eq!(log_lines.len(), 1);
    let log_values = parse_insert_values(&log_lines[0]);
    assert_eq!(log_values[1], "claude-openai-chat-log-failure");
    assert_eq!(log_values[3], "claude-3-7-sonnet");
    assert_eq!(log_values[4], "claude-3-7-sonnet");
    assert_eq!(log_values[17], "502");
    assert_eq!(log_values[19], "claude-session-failure");
    assert!(log_values[18].contains("parse upstream json failed"));

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_claude_buffered_transform_error_does_not_sync_failover_state() {
    let upstream_router = Router::new().route(
        "/v1/chat/completions",
        post(handle_invalid_chat_completions),
    );

    let upstream_listener = bind_test_listener().await;
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("read upstream address");
    let upstream_handle = tokio::spawn(async move {
        let _ = axum::serve(upstream_listener, upstream_router).await;
    });

    let db = Arc::new(Database::memory().expect("create memory database"));
    let original_provider = Provider {
        id: "claude-openai-chat-current".to_string(),
        name: "Claude OpenAI Chat Current".to_string(),
        settings_config: json!({
            "env": {
                "ANTHROPIC_BASE_URL": format!("http://{}", upstream_addr),
                "ANTHROPIC_API_KEY": "sk-test-current"
            }
        }),
        website_url: None,
        category: Some("claude".to_string()),
        created_at: None,
        sort_index: Some(1),
        notes: None,
        meta: Some(ProviderMeta {
            api_format: Some("openai_chat".to_string()),
            ..ProviderMeta::default()
        }),
        icon: None,
        icon_color: None,
        in_failover_queue: true,
    };
    let failover_provider = Provider {
        id: "claude-openai-chat-failover".to_string(),
        name: "Claude OpenAI Chat Failover".to_string(),
        settings_config: json!({
            "env": {
                "ANTHROPIC_BASE_URL": format!("http://{}", upstream_addr),
                "ANTHROPIC_API_KEY": "sk-test-failover"
            }
        }),
        website_url: None,
        category: Some("claude".to_string()),
        created_at: None,
        sort_index: Some(0),
        notes: None,
        meta: Some(ProviderMeta {
            api_format: Some("openai_chat".to_string()),
            ..ProviderMeta::default()
        }),
        icon: None,
        icon_color: None,
        in_failover_queue: true,
    };
    db.save_provider("claude", &original_provider)
        .expect("save original provider");
    db.save_provider("claude", &failover_provider)
        .expect("save failover provider");
    db.set_current_provider("claude", &original_provider.id)
        .expect("set current provider");

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.auto_failover_enabled = true;
    claude_proxy.max_retries = 0;
    db.update_proxy_config_for_app(claude_proxy)
        .await
        .expect("update claude proxy config");

    let service = ProxyService::new(db.clone());
    let mut config = service.get_config().await.expect("read proxy config");
    config.listen_port = 0;
    service
        .update_config(&config)
        .await
        .expect("update proxy config");

    let proxy = service.start().await.expect("start proxy service");
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/v1/messages",
            proxy.address, proxy.port
        ))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert!(!response.status().is_success());
    assert_eq!(
        db.get_current_provider("claude")
            .expect("read current provider after transform error")
            .as_deref(),
        Some("claude-openai-chat-current")
    );

    let status = service.get_status().await;
    assert_eq!(status.current_provider_id, None);
    assert_eq!(status.current_provider, None);
    assert_eq!(status.failover_count, 0);
    assert!(status.active_targets.is_empty());

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}
