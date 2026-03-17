use std::collections::VecDeque;
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

#[derive(Clone, Default)]
struct ScriptedStreamingErrorState {
    attempts: Arc<AtomicUsize>,
    responses: Arc<Mutex<VecDeque<(StatusCode, Value)>>>,
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

async fn handle_streaming_chat_priced(
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
            b"data: {\"id\":\"chatcmpl-stream-priced\",\"model\":\"gpt-5.2\",\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n",
        ));
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"data: {\"id\":\"chatcmpl-stream-priced\",\"model\":\"gpt-5.2\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7}}\n\n",
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

async fn handle_streaming_tool_calls_interleaved(
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
            b"data: {\"id\":\"chatcmpl-tool\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_0\",\"type\":\"function\",\"function\":{\"name\":\"first_tool\"}}]}}]}\n\n",
        ));
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"data: {\"id\":\"chatcmpl-tool\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"second_tool\"}}]}}]}\n\n",
        ));
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"data: {\"id\":\"chatcmpl-tool\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"function\":{\"arguments\":\"{\\\"b\\\":2}\"}}]}}]}\n\n",
        ));
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"data: {\"id\":\"chatcmpl-tool\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"a\\\":1}\"}}]}}]}\n\n",
        ));
        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"data: {\"id\":\"chatcmpl-tool\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":8,\"completion_tokens\":4}}\n\n",
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

async fn handle_non_standard_json_error_body(
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

    (
        StatusCode::BAD_REQUEST,
        [("content-type", "application/json")],
        Json(json!({
            "message": "upstream rejected the request"
        })),
    )
}

async fn handle_standard_json_error_body(
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

    (
        StatusCode::BAD_REQUEST,
        [("content-type", "application/json")],
        Json(json!({
            "error": {
                "message": "upstream rejected the request",
                "type": "invalid_request_error"
            }
        })),
    )
}

async fn handle_plain_text_error_body(
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

    (
        StatusCode::TOO_MANY_REQUESTS,
        [("content-type", "text/plain; charset=utf-8")],
        "upstream rate limit",
    )
}

async fn handle_slow_json_success_body(
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
            br#"{"id":"chatcmpl-slow-fallback","object":"chat.completion","created":123,"model":"gpt-4o-mini","choices":[{"index":0,"message":{"role":"assistant","content":"late hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":11,"completion_tokens":7,"total_tokens":18}}"#,
        ));
    };

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        Body::from_stream(stream),
    )
}

async fn handle_buffered_chat_fallback(
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

    (
        StatusCode::OK,
        Json(json!({
            "id": "chatcmpl-buffered-fallback",
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

async fn handle_scripted_streaming_error(
    State(state): State<ScriptedStreamingErrorState>,
    Json(_body): Json<Value>,
) -> impl IntoResponse {
    state.attempts.fetch_add(1, Ordering::SeqCst);
    let (status, body) = state.responses.lock().await.pop_front().unwrap_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({"error": {"message": "missing scripted streaming response"}}),
    ));
    (status, Json(body))
}

fn parse_sse_events(body: &str) -> Vec<Value> {
    body.split("\n\n")
        .filter_map(|block| {
            let data = block.lines().find_map(|line| line.strip_prefix("data: "))?;
            serde_json::from_str::<Value>(data).ok()
        })
        .collect()
}

fn request_log_insert_lines(db: &Database) -> Vec<String> {
    db.export_sql_string()
        .expect("export sql string")
        .lines()
        .filter(|line| line.contains("INSERT INTO \"proxy_request_logs\""))
        .map(|line| line.to_string())
        .collect()
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

#[tokio::test]
#[serial]
async fn stream_openai_chat_transforms_sse_and_maps_model() {
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
        in_failover_queue: true,
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
        upstream_body.get("model").and_then(|v| v.as_str()),
        Some("mapped-sonnet")
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
async fn stream_openai_chat_logs_request_with_session_id_and_usage() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/chat/completions", post(handle_streaming_chat_priced))
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
        id: "claude-openai-chat-stream-log".to_string(),
        name: "Claude OpenAI Chat Stream Log".to_string(),
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
        in_failover_queue: true,
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
        .json(&json!({
            "model": "claude-3-7-sonnet",
            "stream": true,
            "max_tokens": 64,
            "metadata": {
                "session_id": "claude-stream-session"
            },
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .send()
        .await
        .expect("send request to proxy");

    assert!(response.status().is_success());
    let body = response.text().await.expect("read streaming response body");
    assert!(body.contains("event: message_delta"));

    let log_lines = wait_for_request_log_lines(&db, 1).await;
    assert_eq!(log_lines.len(), 1);
    let log_values = parse_insert_values(&log_lines[0]);
    assert_eq!(log_values[1], "claude-openai-chat-stream-log");
    assert_eq!(log_values[3], "gpt-5.2");
    assert_eq!(log_values[4], "claude-3-7-sonnet");
    assert_eq!(log_values[5], "11");
    assert_eq!(log_values[6], "7");
    assert_eq!(log_values[9], "0.00001925");
    assert_eq!(log_values[10], "0.000098");
    assert_eq!(log_values[13], "0.0002345");
    assert_eq!(log_values[17], "200");
    assert_eq!(log_values[19], "claude-stream-session");
    assert_eq!(log_values[21], "1");
    assert_eq!(log_values[22], "2");

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
#[serial]
async fn stream_openai_chat_buffered_json_fallback_marks_request_success() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route(
            "/v1/chat/completions",
            post(handle_buffered_chat_fallback),
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
        id: "claude-openai-chat-stream-buffered-fallback".to_string(),
        name: "Claude OpenAI Chat Stream Buffered Fallback".to_string(),
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
        in_failover_queue: true,
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

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );

    let body: Value = response.json().await.expect("parse buffered fallback body");
    assert_eq!(body.get("type").and_then(|value| value.as_str()), Some("message"));
    assert_eq!(body.get("role").and_then(|value| value.as_str()), Some("assistant"));
    assert_eq!(
        body.pointer("/content/0/type")
            .and_then(|value| value.as_str()),
        Some("text")
    );
    assert_eq!(
        body.pointer("/content/0/text")
            .and_then(|value| value.as_str()),
        Some("hello back")
    );
    assert_eq!(
        body.pointer("/usage/input_tokens")
            .and_then(|value| value.as_u64()),
        Some(11)
    );
    assert_eq!(
        body.pointer("/usage/output_tokens")
            .and_then(|value| value.as_u64()),
        Some(7)
    );

    let completed = wait_for_proxy_status(&client, &proxy.address, proxy.port, |status| {
        status.active_connections == 0 && status.success_requests == 1
    })
    .await;
    assert_eq!(completed.total_requests, 1);
    assert_eq!(completed.failed_requests, 0);
    assert!(completed.last_error.is_none());

    let upstream_body = upstream_state
        .request_body
        .lock()
        .await
        .clone()
        .expect("upstream should receive request body");
    assert_eq!(
        upstream_body.get("stream").and_then(|value| value.as_bool()),
        Some(true)
    );

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
#[serial]
async fn stream_openai_chat_tool_calls_interleaved_transform_to_stable_anthropic_blocks() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route(
            "/v1/chat/completions",
            post(handle_streaming_tool_calls_interleaved),
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
        id: "claude-openai-chat-stream-tools".to_string(),
        name: "Claude OpenAI Chat Stream Tools".to_string(),
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
        in_failover_queue: true,
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
                "content": "use tools"
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
    let events = parse_sse_events(&body);

    let mut tool_index_by_call: std::collections::HashMap<String, u64> =
        std::collections::HashMap::new();
    for event in &events {
        if event["type"] == "content_block_start" && event["content_block"]["type"] == "tool_use" {
            if let (Some(call_id), Some(index)) = (
                event.pointer("/content_block/id").and_then(|v| v.as_str()),
                event.get("index").and_then(|v| v.as_u64()),
            ) {
                tool_index_by_call.insert(call_id.to_string(), index);
            }
        }
    }

    assert_eq!(tool_index_by_call.len(), 2);
    assert_ne!(
        tool_index_by_call.get("call_0"),
        tool_index_by_call.get("call_1")
    );

    let deltas: Vec<(u64, String)> = events
        .iter()
        .filter(|event| {
            event["type"] == "content_block_delta" && event["delta"]["type"] == "input_json_delta"
        })
        .filter_map(|event| {
            let index = event.get("index").and_then(|v| v.as_u64())?;
            let partial_json = event
                .pointer("/delta/partial_json")
                .and_then(|v| v.as_str())?
                .to_string();
            Some((index, partial_json))
        })
        .collect();

    let second_idx = deltas
        .iter()
        .find_map(|(index, payload)| (payload == "{\"b\":2}").then_some(*index))
        .expect("second tool delta index");
    let first_idx = deltas
        .iter()
        .find_map(|(index, payload)| (payload == "{\"a\":1}").then_some(*index))
        .expect("first tool delta index");

    assert_eq!(second_idx, *tool_index_by_call.get("call_1").unwrap());
    assert_eq!(first_idx, *tool_index_by_call.get("call_0").unwrap());
    assert!(events.iter().any(|event| {
        event["type"] == "message_delta" && event["delta"]["stop_reason"] == "tool_use"
    }));

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
        in_failover_queue: true,
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
        in_failover_queue: true,
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
        in_failover_queue: true,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.auto_failover_enabled = true;
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
        reqwest::StatusCode::GATEWAY_TIMEOUT,
        "proxy should fail fast when the first streaming byte misses the configured timeout"
    );
    let body: Value = response.json().await.expect("parse timeout error body");
    assert!(
        body.pointer("/error/message")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("timed out"),
        "timeout error should surface to the client"
    );
    assert_eq!(
        body.pointer("/error/type").and_then(|v| v.as_str()),
        Some("proxy_error")
    );

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
#[serial]
async fn proxy_claude_streaming_bypasses_first_byte_timeout_when_failover_disabled() {
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
        id: "claude-openai-chat-stream-timeout-bypass".to_string(),
        name: "Claude OpenAI Chat Stream Timeout Bypass".to_string(),
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
        in_failover_queue: true,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.auto_failover_enabled = false;
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

    assert!(response.status().is_success());
    let body = response.text().await.expect("read streaming response body");
    assert!(body.contains("late"));
    assert!(body.contains("event: message_stop"));

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
        in_failover_queue: true,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.auto_failover_enabled = true;
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
async fn proxy_claude_streaming_idle_timeout_does_not_sync_failover_state() {
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
    let original_provider = Provider {
        id: "claude-stream-current".to_string(),
        name: "Claude Stream Current".to_string(),
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
        id: "claude-stream-failover".to_string(),
        name: "Claude Stream Failover".to_string(),
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
    claude_proxy.streaming_idle_timeout = 1;
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

    assert!(response.status().is_success());
    let body = response
        .text()
        .await
        .expect("read idle-timeout response body");
    assert!(body.contains("stream idle timeout"));

    let completed = wait_for_proxy_status(&client, &proxy.address, proxy.port, |status| {
        status.active_connections == 0 && status.failed_requests == 1
    })
    .await;
    assert_eq!(
        db.get_current_provider("claude")
            .expect("read current provider after idle timeout")
            .as_deref(),
        Some("claude-stream-current")
    );
    assert_eq!(completed.current_provider_id, None);
    assert_eq!(completed.current_provider, None);
    assert_eq!(completed.failover_count, 0);
    assert!(completed.active_targets.is_empty());

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
        in_failover_queue: true,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.auto_failover_enabled = true;
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
        in_failover_queue: true,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.auto_failover_enabled = true;
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
        in_failover_queue: true,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.auto_failover_enabled = true;
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

    assert_eq!(response.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
    let body: Value = response.json().await.expect("parse timeout error body");
    assert!(
        body.pointer("/error/message")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("stream timeout after 1s"),
        "non-SSE fallback body reads should still honor the streaming timeout budget"
    );
    assert_eq!(
        body.pointer("/error/type").and_then(|v| v.as_str()),
        Some("proxy_error")
    );

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
#[serial]
async fn proxy_claude_streaming_non_json_error_body_passthrough_preserves_status_and_body() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/chat/completions", post(handle_plain_text_error_body))
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
        id: "claude-openai-chat-stream-plain-error".to_string(),
        name: "Claude OpenAI Chat Stream Plain Error".to_string(),
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
        in_failover_queue: true,
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
#[serial]
async fn proxy_claude_streaming_non_standard_json_error_body_preserves_upstream_status_and_body() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/chat/completions", post(handle_non_standard_json_error_body))
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
        id: "claude-openai-chat-stream-non-standard-error".to_string(),
        name: "Claude OpenAI Chat Stream Non Standard Error".to_string(),
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
        in_failover_queue: true,
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
#[serial]
async fn proxy_claude_streaming_standard_json_error_body_transforms_to_anthropic_shape() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/chat/completions", post(handle_standard_json_error_body))
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
        id: "claude-openai-chat-stream-standard-error".to_string(),
        name: "Claude OpenAI Chat Stream Standard Error".to_string(),
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
        in_failover_queue: true,
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

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let body: Value = response.json().await.expect("parse transformed error body");
    assert_eq!(body.get("type").and_then(|value| value.as_str()), Some("error"));
    assert_eq!(
        body.pointer("/error/type").and_then(|value| value.as_str()),
        Some("invalid_request_error")
    );
    assert_eq!(
        body.pointer("/error/message")
            .and_then(|value| value.as_str()),
        Some("upstream rejected the request")
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
#[serial]
async fn proxy_claude_streaming_non_sse_success_fallback_uses_timeout_budget() {
    let upstream_state = UpstreamState::default();
    let upstream_router = Router::new()
        .route("/v1/chat/completions", post(handle_slow_json_success_body))
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
        id: "claude-openai-chat-stream-json-success-timeout".to_string(),
        name: "Claude OpenAI Chat Stream Json Success Timeout".to_string(),
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
        in_failover_queue: true,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.auto_failover_enabled = true;
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

    assert_eq!(response.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
    let body: Value = response.json().await.expect("parse timeout error body");
    assert!(
        body.pointer("/error/message")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("stream timeout after 1s"),
        "transformed non-SSE success fallback should still honor the streaming timeout budget"
    );
    assert_eq!(
        body.pointer("/error/type").and_then(|v| v.as_str()),
        Some("proxy_error")
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
        in_failover_queue: true,
    };
    db.save_provider("claude", &provider)
        .expect("save test provider");
    db.set_current_provider("claude", &provider.id)
        .expect("set current provider");

    let mut claude_proxy = db
        .get_proxy_config_for_app("claude")
        .await
        .expect("get claude proxy config");
    claude_proxy.auto_failover_enabled = true;
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

    assert_eq!(response.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
    let body: Value = response.json().await.expect("parse timeout error body");
    assert!(
        body.pointer("/error/message")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("request timed out after 1s"),
        "stream retries should not restart the first-byte timeout budget from scratch"
    );
    assert_eq!(
        body.pointer("/error/type").and_then(|v| v.as_str()),
        Some("proxy_error")
    );
    assert_eq!(
        upstream_state.attempts.load(Ordering::SeqCst),
        1,
        "proxy should stop retrying once the shared first-byte budget is exhausted"
    );

    service.stop().await.expect("stop proxy service");
    upstream_handle.abort();
}

#[tokio::test]
#[serial]
async fn proxy_claude_streaming_runtime_disabled_rectifier_does_not_retry_matching_errors() {
    let cases = [
        (
            "signature_error",
            json!({"error": {"message": "messages.1.content.0: Invalid `signature` in `thinking` block"}}),
        ),
        (
            "budget_error",
            json!({"error": {"message": "thinking.budget_tokens: Input should be greater than or equal to 1024"}}),
        ),
    ];

    for (name, error_body) in cases {
        let upstream_state = ScriptedStreamingErrorState {
            responses: Arc::new(Mutex::new(VecDeque::from(vec![
                (StatusCode::BAD_REQUEST, error_body.clone()),
                (
                    StatusCode::OK,
                    json!({"id": "msg_should_not_retry", "content": []}),
                ),
            ]))),
            ..Default::default()
        };
        let upstream_router = Router::new()
            .route("/v1/messages", post(handle_scripted_streaming_error))
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
            id: format!("claude-stream-no-rectifier-retry-{name}"),
            name: format!("Claude Stream No Rectifier Retry {name}"),
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
            meta: None,
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        };
        db.save_provider("claude", &provider)
            .expect("save test provider");
        db.set_current_provider("claude", &provider.id)
            .expect("set current provider");
        db.set_setting(
            "rectifier_config",
            r#"{"enabled":false,"requestThinkingSignature":true,"requestThinkingBudget":true}"#,
        )
        .expect("disable rectifier config for streaming test");

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
            reqwest::StatusCode::BAD_REQUEST,
            "{name} should return the upstream 400 when streaming rectifier is disabled"
        );
        assert_eq!(
            upstream_state.attempts.load(Ordering::SeqCst),
            1,
            "{name} should hit upstream exactly once when streaming rectifier is disabled"
        );

        service.stop().await.expect("stop proxy service");
        upstream_handle.abort();
    }
}
