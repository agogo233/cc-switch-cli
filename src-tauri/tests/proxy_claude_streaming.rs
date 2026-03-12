use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use cc_switch_lib::{Database, Provider, ProviderMeta, ProxyService, ProxyStatus};
use serde_json::{json, Value};
use serial_test::serial;
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

async fn read_proxy_status(client: &reqwest::Client, address: &str, port: u16) -> ProxyStatus {
    client
        .get(format!("http://{}:{}/status", address, port))
        .send()
        .await
        .expect("read proxy status response")
        .json()
        .await
        .expect("parse proxy status")
}

async fn wait_for_proxy_status<F>(
    client: &reqwest::Client,
    address: &str,
    port: u16,
    predicate: F,
) -> ProxyStatus
where
    F: Fn(&ProxyStatus) -> bool,
{
    for _ in 0..20 {
        let status = read_proxy_status(client, address, port).await;
        if predicate(&status) {
            return status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    read_proxy_status(client, address, port).await
}

#[derive(Clone, Default)]
struct UpstreamState {
    request_body: Arc<Mutex<Option<Value>>>,
    authorization: Arc<Mutex<Option<String>>>,
    api_key: Arc<Mutex<Option<String>>>,
}

#[derive(Clone, Default)]
struct RetryStreamingState {
    attempts: Arc<AtomicUsize>,
}

async fn handle_streaming_chat(
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

    let stream = async_stream::stream! {
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"data: {\"id\":\"chatcmpl-stream\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n",
        ));
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"data: {\"id\":\"chatcmpl-stream\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7}}\n\n",
        ));
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"data: [DONE]\n\n",
        ));
    };

    (
        StatusCode::OK,
        [("content-type", "text/event-stream")],
        Body::from_stream(stream),
    )
}

async fn handle_streaming_responses(
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

    let stream = async_stream::stream! {
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"event: response.created\ndata: {\"response\":{\"id\":\"resp-stream\",\"model\":\"gpt-4.1-mini\",\"usage\":{\"input_tokens\":11,\"output_tokens\":0}}}\n\n",
        ));
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"event: response.content_part.added\ndata: {\"item_id\":\"msg_1\",\"content_index\":0,\"part\":{\"type\":\"output_text\"}}\n\n",
        ));
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"event: response.output_text.delta\ndata: {\"item_id\":\"msg_1\",\"content_index\":0,\"delta\":\"hello from responses\"}\n\n",
        ));
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"event: response.content_part.done\ndata: {\"item_id\":\"msg_1\",\"content_index\":0}\n\n",
        ));
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"event: response.completed\ndata: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":11,\"output_tokens\":7}}}\n\n",
        ));
    };

    (
        StatusCode::OK,
        [("content-type", "text/event-stream")],
        Body::from_stream(stream),
    )
}

async fn handle_slow_streaming_chat(
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

    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    let sse = concat!(
        "data: {\"id\":\"chatcmpl-stream\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"content\":\"late\"}}]}\n\n",
        "data: [DONE]\n\n"
    );

    (StatusCode::OK, [("content-type", "text/event-stream")], sse)
}

async fn handle_idle_streaming_chat(
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

    let stream = async_stream::stream! {
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"data: {\"id\":\"chatcmpl-stream\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n",
        ));
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"data: [DONE]\n\n",
        ));
    };

    (
        StatusCode::OK,
        [("content-type", "text/event-stream")],
        Body::from_stream(stream),
    )
}

async fn handle_delayed_first_chunk_streaming_chat(
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

    let stream = async_stream::stream! {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"data: {\"id\":\"chatcmpl-stream\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"content\":\"late\"}}]}\n\n",
        ));
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"data: [DONE]\n\n",
        ));
    };

    (
        StatusCode::OK,
        [("content-type", "text/event-stream")],
        Body::from_stream(stream),
    )
}

async fn handle_delayed_headers_and_first_chunk_streaming_chat(
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

    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    let stream = async_stream::stream! {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"data: {\"id\":\"chatcmpl-stream\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"content\":\"late\"}}]}\n\n",
        ));
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"data: [DONE]\n\n",
        ));
    };

    (
        StatusCode::OK,
        [("content-type", "text/event-stream")],
        Body::from_stream(stream),
    )
}

async fn handle_slow_json_error_body(
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

    let stream = async_stream::stream! {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            br#"{"error":"upstream slow body"}"#,
        ));
    };

    (
        StatusCode::BAD_REQUEST,
        [("content-type", "application/json")],
        Body::from_stream(stream),
    )
}

async fn handle_retrying_streaming_timeout(
    State(state): State<RetryStreamingState>,
) -> impl IntoResponse {
    let attempt = state.attempts.fetch_add(1, Ordering::SeqCst);
    if attempt == 0 {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    }

    (
        StatusCode::OK,
        [("content-type", "text/event-stream")],
        "data: {\"id\":\"chatcmpl-stream\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"content\":\"late\"}}]}\n\ndata: [DONE]\n\n",
    )
}

#[tokio::test]
#[serial]
async fn proxy_claude_openai_chat_streaming_transforms_sse() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/chat/completions", post(handle_streaming_chat))
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
        id: "claude-openai-chat-stream".to_string(),
        name: "Claude OpenAI Chat Stream".to_string(),
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
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "stream": true,
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert!(
        response.status().is_success(),
        "proxy should return streaming success"
    );
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );

    let in_flight = read_proxy_status(&client, &proxy.address, proxy.port).await;
    assert_eq!(in_flight.total_requests, 1);
    assert_eq!(in_flight.active_connections, 1);
    assert_eq!(in_flight.success_requests, 0);
    assert_eq!(in_flight.failed_requests, 0);
    assert!(in_flight.last_error.is_none());

    let body = response.text().await.expect("read streaming response body");
    assert!(body.contains("event: message_start"));
    assert!(body.contains("event: content_block_start"));
    assert!(body.contains("event: content_block_delta"));
    assert!(body.contains("hello"));
    assert!(body.contains("event: message_delta"));
    assert!(body.contains("\"input_tokens\":11"));
    assert!(body.contains("\"output_tokens\":7"));
    assert!(body.contains("event: message_stop"));

    let completed = wait_for_proxy_status(&client, &proxy.address, proxy.port, |status| {
        status.active_connections == 0 && status.success_requests == 1
    })
    .await;
    assert_eq!(completed.failed_requests, 0);
    assert!(completed.last_error.is_none());
    assert_eq!(completed.success_rate, 100.0);

    let upstream_body = upstream_state
        .request_body
        .lock()
        .await
        .clone()
        .expect("upstream should receive request body");
    assert_eq!(
        upstream_body.get("stream").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        upstream_state.authorization.lock().await.as_deref(),
        Some("Bearer sk-test-claude")
    );
    assert_eq!(
        upstream_state.api_key.lock().await.as_deref(),
        Some("sk-test-claude")
    );

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
#[serial]
async fn proxy_claude_openai_responses_streaming_transforms_sse() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/responses", post(handle_streaming_responses))
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
        id: "claude-openai-responses-stream".to_string(),
        name: "Claude OpenAI Responses Stream".to_string(),
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
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "stream": true,
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

    assert!(response.status().is_success());
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );

    let body = response.text().await.expect("read streaming response body");
    assert!(body.contains("event: message_start"));
    assert!(body.contains("event: content_block_start"));
    assert!(body.contains("event: content_block_delta"));
    assert!(body.contains("hello from responses"));
    assert!(body.contains("event: message_delta"));
    assert!(body.contains("\"input_tokens\":11"));
    assert!(body.contains("\"output_tokens\":7"));
    assert!(body.contains("event: message_stop"));

    let upstream_body = upstream_state
        .request_body
        .lock()
        .await
        .clone()
        .expect("upstream should receive request body");
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
        upstream_state.authorization.lock().await.as_deref(),
        Some("Bearer sk-test-claude")
    );
    assert_eq!(
        upstream_state.api_key.lock().await.as_deref(),
        Some("sk-test-claude")
    );

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
#[serial]
async fn proxy_claude_streaming_client_abort_counts_as_failure() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/chat/completions", post(handle_streaming_chat))
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
        id: "claude-openai-chat-stream-client-abort".to_string(),
        name: "Claude OpenAI Chat Stream Client Abort".to_string(),
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
    let mut response = client
        .post(format!(
            "http://{}:{}/v1/messages",
            proxy.address, proxy.port
        ))
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "stream": true,
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert!(
        response.status().is_success(),
        "proxy should establish the stream"
    );
    let first_chunk = response
        .chunk()
        .await
        .expect("read first streaming response chunk")
        .expect("first stream chunk should exist");
    assert!(String::from_utf8_lossy(&first_chunk).contains("event: message_start"));

    drop(response);

    let completed = wait_for_proxy_status(&client, &proxy.address, proxy.port, |status| {
        status.active_connections == 0 && status.failed_requests == 1
    })
    .await;
    assert_eq!(completed.success_requests, 0);
    assert_eq!(completed.total_requests, 1);
    assert!(completed
        .last_error
        .as_deref()
        .unwrap_or_default()
        .contains("before completion"));

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
#[serial]
async fn proxy_claude_streaming_first_byte_timeout_uses_app_config() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/chat/completions", post(handle_slow_streaming_chat))
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
        id: "claude-openai-chat-stream-timeout".to_string(),
        name: "Claude OpenAI Chat Stream Timeout".to_string(),
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

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.max_retries = 0;
    claude_proxy.streaming_first_byte_timeout = 1;
    db.update_proxy_config_for_app(claude_proxy)
        .await
        .expect("update claude proxy config");

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
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "stream": true,
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
        reqwest::StatusCode::BAD_GATEWAY,
        "proxy should fail fast when the first streaming byte misses the configured timeout"
    );
    let body: Value = response.json().await.expect("parse timeout error body");
    assert!(
        body.get("error")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("timed out"),
        "timeout error should surface to the client"
    );

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
#[serial]
async fn proxy_claude_streaming_idle_timeout_uses_app_config() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/chat/completions", post(handle_idle_streaming_chat))
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
        id: "claude-openai-chat-stream-idle-timeout".to_string(),
        name: "Claude OpenAI Chat Stream Idle Timeout".to_string(),
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

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.max_retries = 0;
    claude_proxy.streaming_idle_timeout = 1;
    db.update_proxy_config_for_app(claude_proxy)
        .await
        .expect("update claude proxy config");

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
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "stream": true,
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert!(
        response.status().is_success(),
        "stream setup should still succeed"
    );
    let body_result = response.text().await;
    assert!(
        body_result
            .as_deref()
            .unwrap_or_default()
            .contains("event: error"),
        "anthropic SSE transform should surface the idle-timeout as an error event"
    );
    assert!(
        body_result
            .as_deref()
            .unwrap_or_default()
            .contains("stream idle timeout"),
        "idle-timeout details should be visible in the transformed SSE stream"
    );

    let completed = wait_for_proxy_status(&client, &proxy.address, proxy.port, |status| {
        status.active_connections == 0 && status.failed_requests == 1
    })
    .await;
    assert_eq!(completed.success_requests, 0);
    assert_eq!(completed.total_requests, 1);
    assert!(completed
        .last_error
        .as_deref()
        .unwrap_or_default()
        .contains("stream idle timeout"));

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
#[serial]
async fn proxy_claude_streaming_first_chunk_timeout_after_headers_uses_app_config() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route(
            "/v1/chat/completions",
            post(handle_delayed_first_chunk_streaming_chat),
        )
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
        id: "claude-openai-chat-stream-first-chunk-timeout".to_string(),
        name: "Claude OpenAI Chat Stream First Chunk Timeout".to_string(),
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

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.max_retries = 0;
    claude_proxy.streaming_first_byte_timeout = 1;
    db.update_proxy_config_for_app(claude_proxy)
        .await
        .expect("update claude proxy config");

    let service = ProxyService::new(db);
    let mut runtime_config = service.get_config().await.expect("read proxy config");
    runtime_config.listen_port = 0;

    let proxy = service
        .start_with_runtime_config(runtime_config)
        .await
        .expect("start proxy service");
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/v1/messages",
            proxy.address, proxy.port
        ))
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "stream": true,
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert!(
        response.status().is_success(),
        "stream should be established before timeout"
    );
    let body = response.text().await.expect("read timeout event stream");
    assert!(body.contains("event: error"));
    assert!(body.contains("stream timeout after 1s"));

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
#[serial]
async fn proxy_claude_streaming_first_byte_timeout_spans_headers_and_first_chunk() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route(
            "/v1/chat/completions",
            post(handle_delayed_headers_and_first_chunk_streaming_chat),
        )
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
        id: "claude-openai-chat-stream-shared-first-byte-budget".to_string(),
        name: "Claude OpenAI Chat Stream Shared First Byte Budget".to_string(),
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

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.max_retries = 0;
    claude_proxy.streaming_first_byte_timeout = 1;
    db.update_proxy_config_for_app(claude_proxy)
        .await
        .expect("update claude proxy config");

    let service = ProxyService::new(db);
    let mut runtime_config = service.get_config().await.expect("read proxy config");
    runtime_config.listen_port = 0;

    let proxy = service
        .start_with_runtime_config(runtime_config)
        .await
        .expect("start proxy service");
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/v1/messages",
            proxy.address, proxy.port
        ))
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "stream": true,
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert!(
        response.status().is_success(),
        "stream should be established before timeout"
    );
    let body = response.text().await.expect("read timeout event stream");
    assert!(body.contains("event: error"));
    assert!(body.contains("stream timeout after 1s"));

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
#[serial]
async fn proxy_claude_streaming_non_sse_error_body_uses_timeout_budget() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/chat/completions", post(handle_slow_json_error_body))
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
        id: "claude-openai-chat-stream-json-error-timeout".to_string(),
        name: "Claude OpenAI Chat Stream Json Error Timeout".to_string(),
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

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.max_retries = 0;
    claude_proxy.streaming_first_byte_timeout = 1;
    db.update_proxy_config_for_app(claude_proxy)
        .await
        .expect("update claude proxy config");

    let service = ProxyService::new(db);
    let mut runtime_config = service.get_config().await.expect("read proxy config");
    runtime_config.listen_port = 0;

    let proxy = service
        .start_with_runtime_config(runtime_config)
        .await
        .expect("start proxy service");
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/v1/messages",
            proxy.address, proxy.port
        ))
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "stream": true,
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);
    let body: Value = response.json().await.expect("parse timeout error body");
    assert!(
        body.get("error")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("stream timeout after 1s"),
        "non-SSE fallback body reads should still honor the streaming timeout budget"
    );

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
#[serial]
async fn proxy_claude_streaming_retry_respects_total_first_byte_budget() {
    let upstream_state = RetryStreamingState::default();
    let upstream_router = Router::new()
        .route(
            "/v1/chat/completions",
            post(handle_retrying_streaming_timeout),
        )
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
        id: "claude-openai-chat-stream-retry-budget".to_string(),
        name: "Claude OpenAI Chat Stream Retry Budget".to_string(),
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

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.max_retries = 1;
    claude_proxy.streaming_first_byte_timeout = 1;
    db.update_proxy_config_for_app(claude_proxy)
        .await
        .expect("update claude proxy config");

    let service = ProxyService::new(db);
    let mut runtime_config = service.get_config().await.expect("read proxy config");
    runtime_config.listen_port = 0;

    let proxy = service
        .start_with_runtime_config(runtime_config)
        .await
        .expect("start proxy service");
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/v1/messages",
            proxy.address, proxy.port
        ))
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "stream": true,
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);
    let body: Value = response.json().await.expect("parse timeout error body");
    assert!(
        body.get("error")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("request timed out after 1s"),
        "stream retries should not restart the first-byte timeout budget from scratch"
    );
    assert_eq!(
        upstream_state.attempts.load(Ordering::SeqCst),
        1,
        "proxy should stop retrying once the shared first-byte budget is exhausted"
    );

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}
