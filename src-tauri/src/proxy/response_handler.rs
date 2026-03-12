use axum::{
    body::Body,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use futures::StreamExt;
use serde_json::json;

use super::{
    error::ProxyError,
    response::{PreparedResponse, StreamCompletion},
    server::ProxyServerState,
};

pub struct ResponseHandler;

impl ResponseHandler {
    pub async fn finish_buffered(
        state: &ProxyServerState,
        response_result: Result<PreparedResponse, ProxyError>,
        status: reqwest::StatusCode,
    ) -> Response {
        match response_result {
            Ok(response) => {
                if status.is_success() {
                    state.record_request_success().await;
                } else {
                    state.record_upstream_failure(status).await;
                }
                response.response
            }
            Err(error) => {
                state.record_request_error(&error).await;
                proxy_error_response(error)
            }
        }
    }

    pub async fn finish_streaming(
        state: &ProxyServerState,
        response_result: Result<PreparedResponse, ProxyError>,
        status: reqwest::StatusCode,
    ) -> Response {
        match response_result {
            Ok(response) => {
                if !status.is_success() {
                    state.record_upstream_failure(status).await;
                    return response.response;
                }

                track_streaming_response(state.clone(), response)
            }
            Err(error) => {
                state.record_request_error(&error).await;
                proxy_error_response(error)
            }
        }
    }
}

fn track_streaming_response(state: ProxyServerState, response: PreparedResponse) -> Response {
    let (parts, body) = response.response.into_parts();
    let mut recorder = StreamingOutcomeRecorder::new(state, response.stream_completion);
    let tracked_stream = async_stream::stream! {
        let mut stream = body.into_data_stream();

        while let Some(next) = stream.next().await {
            match next {
                Ok(chunk) => yield Ok(chunk),
                Err(error) => {
                    recorder.finish();
                    yield Err(std::io::Error::other(error));
                    return;
                }
            }
        }

        recorder.finish();
    };

    Response::from_parts(parts, Body::from_stream(tracked_stream))
}

struct StreamingOutcomeRecorder {
    state: ProxyServerState,
    stream_completion: Option<StreamCompletion>,
    finished: bool,
}

impl StreamingOutcomeRecorder {
    fn new(state: ProxyServerState, stream_completion: Option<StreamCompletion>) -> Self {
        Self {
            state,
            stream_completion,
            finished: false,
        }
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;

        let state = self.state.clone();
        match self
            .stream_completion
            .as_ref()
            .and_then(StreamCompletion::outcome)
        {
            Some(Err(message)) => {
                tokio::spawn(async move {
                    state.record_request_error_message(message).await;
                });
            }
            Some(Ok(())) => {
                tokio::spawn(async move {
                    state.record_request_success().await;
                });
            }
            None => {
                tokio::spawn(async move {
                    state
                        .record_request_error_message(
                            "stream terminated before completion".to_string(),
                        )
                        .await;
                });
            }
        }
    }
}

impl Drop for StreamingOutcomeRecorder {
    fn drop(&mut self) {
        self.finish();
    }
}

pub fn proxy_error_response(error: ProxyError) -> Response {
    match error {
        ProxyError::ConfigError(message) | ProxyError::AuthError(message) => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
        }
        ProxyError::RequestFailed(message) | ProxyError::TransformError(message) => {
            (StatusCode::BAD_GATEWAY, Json(json!({ "error": message }))).into_response()
        }
        ProxyError::UpstreamError { status, body } => {
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
            (status, Json(json!({ "error": body }))).into_response()
        }
    }
}
