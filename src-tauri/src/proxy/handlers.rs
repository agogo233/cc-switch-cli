use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

use crate::app_config::AppType;

use super::{
    forwarder::{ForwardOptions, RequestForwarder},
    handler_context::HandlerContext,
    providers::{ClaudeAdapter, ProviderAdapter},
    response::{
        build_anthropic_stream_response, build_buffered_json_response,
        build_buffered_passthrough_response, build_passthrough_response, is_sse_response,
    },
    response_handler::{proxy_error_response, ResponseHandler},
    server::ProxyServerState,
};

pub async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "ok": true })))
}

pub async fn get_status(State(state): State<ProxyServerState>) -> impl IntoResponse {
    Json(state.snapshot_status().await)
}

pub async fn handle_messages(
    State(state): State<ProxyServerState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    handle_claude_request(state, headers, body).await
}

pub async fn handle_chat_completions(
    State(state): State<ProxyServerState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    handle_passthrough_request(
        state,
        headers,
        body,
        AppType::Codex,
        "/chat/completions".to_string(),
    )
    .await
}

pub async fn handle_responses(
    State(state): State<ProxyServerState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    handle_passthrough_request(
        state,
        headers,
        body,
        AppType::Codex,
        "/responses".to_string(),
    )
    .await
}

pub async fn handle_gemini(
    State(state): State<ProxyServerState>,
    uri: Uri,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let endpoint = uri
        .path_and_query()
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| uri.path().to_string());
    let endpoint = endpoint
        .strip_prefix("/gemini")
        .unwrap_or(endpoint.as_str())
        .to_string();
    handle_passthrough_request(state, headers, body, AppType::Gemini, endpoint).await
}

async fn handle_claude_request(
    state: ProxyServerState,
    headers: HeaderMap,
    body: Value,
) -> Response {
    let context = match HandlerContext::load(&state, AppType::Claude).await {
        Ok(context) => context,
        Err(error) => {
            state.record_request_error(&error).await;
            return proxy_error_response(error);
        }
    };

    let forwarder = match RequestForwarder::new() {
        Ok(forwarder) => forwarder,
        Err(error) => {
            context.state.record_request_error(&error).await;
            return proxy_error_response(error);
        }
    };

    let is_stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let adapter = ClaudeAdapter::new();

    if is_stream {
        let first_byte_timeout = context.streaming_first_byte_timeout();
        let request_started_at = Instant::now();
        let upstream_endpoint = context.provider_router.upstream_endpoint("/v1/messages");
        let options = ForwardOptions {
            max_retries: context.app_proxy.max_retries,
            request_timeout: first_byte_timeout,
        };
        let response = match forwarder
            .forward_response(
                &context.app_type,
                context.provider_router.provider(),
                &upstream_endpoint,
                body,
                &headers,
                options,
            )
            .await
        {
            Ok(response) => response,
            Err(error) => {
                context.state.record_request_error(&error).await;
                return proxy_error_response(error);
            }
        };

        let status = response.status();
        let first_byte_timeout = Some(remaining_timeout(first_byte_timeout, request_started_at));
        let idle_timeout = context.streaming_idle_timeout();
        let response_result = if adapter.needs_transform(context.provider_router.provider())
            && status.is_success()
            && is_sse_response(&response)
        {
            build_anthropic_stream_response(response, first_byte_timeout, idle_timeout)
        } else {
            build_passthrough_response(response, first_byte_timeout, idle_timeout).await
        };

        return ResponseHandler::finish_streaming(&context.state, response_result, status).await;
    }

    let provider = context.provider_router.provider();
    let upstream_endpoint = context.provider_router.upstream_endpoint("/v1/messages");
    let options = ForwardOptions {
        max_retries: context.app_proxy.max_retries,
        request_timeout: context.non_streaming_timeout(),
    };

    let response = match forwarder
        .forward_buffered_response(
            &context.app_type,
            provider,
            &upstream_endpoint,
            body,
            &headers,
            options,
        )
        .await
    {
        Ok(response) => response,
        Err(error) => {
            context.state.record_request_error(&error).await;
            return proxy_error_response(error);
        }
    };

    let status = response.status;
    let response_result = if adapter.needs_transform(&provider) && status.is_success() {
        build_buffered_json_response(status, &response.headers, response.body, |body| {
            adapter.transform_response(body)
        })
    } else {
        build_buffered_passthrough_response(status, &response.headers, response.body)
    };

    ResponseHandler::finish_buffered(&context.state, response_result, status).await
}

async fn handle_passthrough_request(
    state: ProxyServerState,
    headers: HeaderMap,
    body: Value,
    app_type: AppType,
    endpoint: String,
) -> Response {
    let context = match HandlerContext::load(&state, app_type).await {
        Ok(context) => context,
        Err(error) => {
            state.record_request_error(&error).await;
            return proxy_error_response(error);
        }
    };

    let forwarder = match RequestForwarder::new() {
        Ok(forwarder) => forwarder,
        Err(error) => {
            context.state.record_request_error(&error).await;
            return proxy_error_response(error);
        }
    };

    let is_stream = request_is_streaming(&context.app_type, &endpoint, &body);
    let options = if is_stream {
        ForwardOptions {
            max_retries: context.app_proxy.max_retries,
            request_timeout: context.streaming_first_byte_timeout(),
        }
    } else {
        ForwardOptions {
            max_retries: context.app_proxy.max_retries,
            request_timeout: context.non_streaming_timeout(),
        }
    };

    if is_stream {
        let first_byte_timeout = context.streaming_first_byte_timeout();
        let request_started_at = Instant::now();
        let upstream_endpoint = context.provider_router.upstream_endpoint(&endpoint);
        let response = match forwarder
            .forward_response(
                &context.app_type,
                context.provider_router.provider(),
                &upstream_endpoint,
                body,
                &headers,
                options,
            )
            .await
        {
            Ok(response) => response,
            Err(error) => {
                context.state.record_request_error(&error).await;
                return proxy_error_response(error);
            }
        };

        let status = response.status();
        return ResponseHandler::finish_streaming(
            &context.state,
            build_passthrough_response(
                response,
                Some(remaining_timeout(first_byte_timeout, request_started_at)),
                context.streaming_idle_timeout(),
            )
            .await,
            status,
        )
        .await;
    }

    let upstream_endpoint = context.provider_router.upstream_endpoint(&endpoint);
    let response = match forwarder
        .forward_buffered_response(
            &context.app_type,
            context.provider_router.provider(),
            &upstream_endpoint,
            body,
            &headers,
            options,
        )
        .await
    {
        Ok(response) => response,
        Err(error) => {
            context.state.record_request_error(&error).await;
            return proxy_error_response(error);
        }
    };

    ResponseHandler::finish_buffered(
        &context.state,
        build_buffered_passthrough_response(response.status, &response.headers, response.body),
        response.status,
    )
    .await
}

fn request_is_streaming(app_type: &AppType, endpoint: &str, body: &Value) -> bool {
    if body
        .get("stream")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return true;
    }

    matches!(app_type, AppType::Gemini)
        && (endpoint.contains("alt=sse") || endpoint.contains(":streamGenerateContent"))
}

fn remaining_timeout(timeout: Duration, started_at: Instant) -> Duration {
    timeout.saturating_sub(started_at.elapsed())
}
