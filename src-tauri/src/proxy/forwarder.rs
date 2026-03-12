use axum::http::HeaderMap;
use bytes::Bytes;
use serde_json::Value;
use std::time::{Duration, Instant};

use crate::{app_config::AppType, provider::Provider};

use super::{
    error::ProxyError,
    providers::{get_adapter, ProviderAdapter},
};

const HEADER_BLACKLIST: &[&str] = &[
    "authorization",
    "x-api-key",
    "x-goog-api-key",
    "host",
    "content-length",
    "transfer-encoding",
    "accept-encoding",
    "anthropic-beta",
    "anthropic-version",
    "x-forwarded-for",
    "x-real-ip",
];

pub struct RequestForwarder {
    client: reqwest::Client,
}

#[derive(Debug, Clone, Copy)]
pub struct ForwardOptions {
    pub max_retries: u32,
    pub request_timeout: Duration,
}

pub struct BufferedResponse {
    pub status: reqwest::StatusCode,
    pub headers: reqwest::header::HeaderMap,
    pub body: Bytes,
}

impl RequestForwarder {
    pub fn new() -> Result<Self, ProxyError> {
        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| ProxyError::RequestFailed(format!("build reqwest client failed: {e}")))?;
        Ok(Self { client })
    }

    pub async fn forward_response(
        &self,
        app_type: &AppType,
        provider: &Provider,
        endpoint: &str,
        body: Value,
        headers: &HeaderMap,
        options: ForwardOptions,
    ) -> Result<reqwest::Response, ProxyError> {
        let adapter = get_adapter(app_type);
        let base_url = adapter.extract_base_url(provider)?;
        let request_body = if adapter.needs_transform(provider) {
            adapter.transform_request(body, provider)?
        } else {
            body
        };

        let base_request = self.build_request(
            &*adapter,
            provider,
            &base_url,
            endpoint,
            &request_body,
            headers,
            options,
        );

        let mut attempt = 0u32;
        let started_at = Instant::now();
        loop {
            let remaining_timeout = options.request_timeout.saturating_sub(started_at.elapsed());
            if remaining_timeout.is_zero() {
                return Err(ProxyError::RequestFailed(format!(
                    "request timed out after {}s",
                    options.request_timeout.as_secs()
                )));
            }

            let request = base_request.try_clone().ok_or_else(|| {
                ProxyError::RequestFailed("clone proxy request failed before retry".to_string())
            })?;

            match tokio::time::timeout(remaining_timeout, request.send()).await {
                Ok(Ok(response)) => return Ok(response),
                Ok(Err(error))
                    if attempt < options.max_retries
                        && (error.is_timeout() || error.is_connect()) =>
                {
                    attempt += 1;
                }
                Ok(Err(error)) => {
                    if error.is_timeout() {
                        return Err(ProxyError::RequestFailed(format!(
                            "request timed out after {}s",
                            options.request_timeout.as_secs()
                        )));
                    }
                    return Err(ProxyError::RequestFailed(error.to_string()));
                }
                Err(_) if attempt < options.max_retries => {
                    attempt += 1;
                }
                Err(_) => {
                    return Err(ProxyError::RequestFailed(format!(
                        "request timed out after {}s",
                        options.request_timeout.as_secs()
                    )));
                }
            }
        }
    }

    pub async fn forward_buffered_response(
        &self,
        app_type: &AppType,
        provider: &Provider,
        endpoint: &str,
        body: Value,
        headers: &HeaderMap,
        options: ForwardOptions,
    ) -> Result<BufferedResponse, ProxyError> {
        let adapter = get_adapter(app_type);
        let base_url = adapter.extract_base_url(provider)?;
        let request_body = if adapter.needs_transform(provider) {
            adapter.transform_request(body, provider)?
        } else {
            body
        };

        let base_request = self.build_request(
            &*adapter,
            provider,
            &base_url,
            endpoint,
            &request_body,
            headers,
            options,
        );

        let mut attempt = 0u32;
        loop {
            let request = base_request.try_clone().ok_or_else(|| {
                ProxyError::RequestFailed("clone proxy request failed before retry".to_string())
            })?;

            let result = tokio::time::timeout(options.request_timeout, async move {
                let response = request.send().await?;
                let status = response.status();
                let response_headers = response.headers().clone();
                let response_body = response.bytes().await?;
                Ok::<BufferedResponse, reqwest::Error>(BufferedResponse {
                    status,
                    headers: response_headers,
                    body: response_body,
                })
            })
            .await;

            match result {
                Ok(Ok(response)) => return Ok(response),
                Ok(Err(error))
                    if attempt < options.max_retries
                        && (error.is_timeout() || error.is_connect()) =>
                {
                    attempt += 1;
                }
                Ok(Err(error)) => {
                    if error.is_timeout() {
                        return Err(ProxyError::RequestFailed(format!(
                            "request timed out after {}s",
                            options.request_timeout.as_secs()
                        )));
                    }
                    return Err(ProxyError::RequestFailed(error.to_string()));
                }
                Err(_) if attempt < options.max_retries => {
                    attempt += 1;
                }
                Err(_) => {
                    return Err(ProxyError::RequestFailed(format!(
                        "request timed out after {}s",
                        options.request_timeout.as_secs()
                    )));
                }
            }
        }
    }

    fn build_request(
        &self,
        adapter: &dyn ProviderAdapter,
        provider: &Provider,
        base_url: &str,
        endpoint: &str,
        request_body: &Value,
        headers: &HeaderMap,
        _options: ForwardOptions,
    ) -> reqwest::RequestBuilder {
        let mut request = self.client.post(adapter.build_url(base_url, endpoint));

        for (key, value) in headers {
            if HEADER_BLACKLIST
                .iter()
                .any(|blocked| key.as_str().eq_ignore_ascii_case(blocked))
            {
                continue;
            }
            request = request.header(key, value);
        }

        if adapter.name() == "Claude" {
            const CLAUDE_CODE_BETA: &str = "claude-code-20250219";
            let beta_value = headers
                .get("anthropic-beta")
                .and_then(|value| value.to_str().ok())
                .map(|value| {
                    if value.contains(CLAUDE_CODE_BETA) {
                        value.to_string()
                    } else {
                        format!("{CLAUDE_CODE_BETA},{value}")
                    }
                })
                .unwrap_or_else(|| CLAUDE_CODE_BETA.to_string());
            request = request.header("anthropic-beta", beta_value);
        }

        if let Some(forwarded_for) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            request = request.header("x-forwarded-for", forwarded_for);
        }
        if let Some(real_ip) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
            request = request.header("x-real-ip", real_ip);
        }

        request = request.header("accept-encoding", "identity");

        if let Some(auth) = adapter.extract_auth(provider) {
            request = adapter.add_auth_headers(request, &auth);
        }

        if adapter.name() == "Claude" {
            let version = headers
                .get("anthropic-version")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("2023-06-01");
            request = request.header("anthropic-version", version);
        }

        request.json(request_body)
    }
}
