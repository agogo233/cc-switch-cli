use axum::http::HeaderMap;
use bytes::Bytes;
use serde_json::Value;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{app_config::AppType, provider::Provider};

use super::{
    body_filter::filter_private_params_with_whitelist,
    error::ProxyError,
    http_client,
    model_mapper::apply_model_mapping,
    provider_router::ProviderRouter,
    providers::{get_adapter, ProviderAdapter},
    thinking_budget_rectifier::{rectify_thinking_budget, should_rectify_thinking_budget},
    thinking_rectifier::{
        normalize_thinking_type, rectify_anthropic_request, should_rectify_thinking_signature,
    },
    types::OptimizerConfig,
    types::RectifierConfig,
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
    router: Arc<ProviderRouter>,
    optimizer_config: OptimizerConfig,
}

#[derive(Debug, Clone, Copy)]
pub struct ForwardOptions {
    pub max_retries: u32,
    pub request_timeout: Option<Duration>,
    pub bypass_circuit_breaker: bool,
}

#[derive(Debug)]
pub struct BufferedResponse {
    pub status: reqwest::StatusCode,
    pub headers: reqwest::header::HeaderMap,
    pub body: Bytes,
}

#[derive(Debug)]
pub struct ForwardedResponse<T> {
    pub provider: Provider,
    pub response: T,
}

#[derive(Debug)]
pub struct ForwardFailure {
    pub provider: Option<Provider>,
    pub error: ProxyError,
}

impl ForwardFailure {
    fn new(provider: Option<Provider>, error: ProxyError) -> Self {
        Self { provider, error }
    }
}

#[derive(Debug)]
pub enum StreamingResponse {
    Live(reqwest::Response),
    Buffered(BufferedResponse),
}

impl StreamingResponse {
    pub fn status(&self) -> reqwest::StatusCode {
        match self {
            Self::Live(response) => response.status(),
            Self::Buffered(response) => response.status,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptDecision {
    ProviderFailure,
    NeutralRelease,
    FatalStop,
}

enum BufferedRequestError {
    BeforeResponse(ProxyError),
    AfterResponse(ProxyError),
}

enum StreamingRequestError {
    BeforeResponse(ProxyError),
    AfterResponse(ProxyError),
}

struct BufferedAttemptOutcome {
    response: BufferedResponse,
    attempt_decision: AttemptDecision,
}

struct StreamingAttemptOutcome {
    response: StreamingResponse,
    attempt_decision: AttemptDecision,
}

impl RequestForwarder {
    pub fn new(router: Arc<ProviderRouter>) -> Result<Self, ProxyError> {
        Ok(Self {
            router,
            optimizer_config: OptimizerConfig::default(),
        })
    }

    pub fn with_optimizer_config(mut self, optimizer_config: OptimizerConfig) -> Self {
        self.optimizer_config = optimizer_config;
        self
    }

    pub async fn forward_response(
        &self,
        app_type: &AppType,
        endpoint: &str,
        body: Value,
        headers: &HeaderMap,
        providers: Vec<Provider>,
        options: ForwardOptions,
        rectifier_config: RectifierConfig,
    ) -> Result<ForwardedResponse<StreamingResponse>, ProxyError> {
        self.forward_response_detailed(
            app_type,
            endpoint,
            body,
            headers,
            providers,
            options,
            rectifier_config,
        )
        .await
        .map_err(|failure| failure.error)
    }

    pub async fn forward_response_detailed(
        &self,
        app_type: &AppType,
        endpoint: &str,
        body: Value,
        headers: &HeaderMap,
        providers: Vec<Provider>,
        options: ForwardOptions,
        rectifier_config: RectifierConfig,
    ) -> Result<ForwardedResponse<StreamingResponse>, ForwardFailure> {
        if providers.is_empty() {
            return Err(ForwardFailure::new(None, ProxyError::NoAvailableProvider));
        }

        let claude_error_path = matches!(app_type, AppType::Claude);
        let bypass_circuit_breaker = options.bypass_circuit_breaker;
        let mut last_error = None;
        let mut attempted_provider = false;
        let mut pending_upstream_response = None;

        for provider in providers {
            let permit = if bypass_circuit_breaker {
                super::circuit_breaker::AllowResult {
                    allowed: true,
                    used_half_open_permit: false,
                }
            } else {
                self.router
                    .allow_provider_request(&provider.id, app_type.as_str())
                    .await
            };

            if !permit.allowed {
                continue;
            }

            attempted_provider = true;
            pending_upstream_response = None;
            let provider_needs_transform =
                matches!(app_type, AppType::Claude) && get_adapter(app_type).needs_transform(&provider);

            match self
                .send_streaming_request(
                    app_type,
                    &provider,
                    endpoint,
                    &body,
                    headers,
                    options,
                    &rectifier_config,
                )
                .await
            {
                Ok(outcome) => {
                    let response = outcome.response;
                    if response.status().is_success() {
                        if !bypass_circuit_breaker {
                            let _ = self
                                .router
                                .record_result(
                                    &provider.id,
                                    app_type.as_str(),
                                    permit.used_half_open_permit,
                                    true,
                                    None,
                                )
                                .await;
                        }

                        return Ok(ForwardedResponse { provider, response });
                    }

                    match outcome.attempt_decision {
                        AttemptDecision::NeutralRelease => {
                            if !bypass_circuit_breaker {
                                self.router
                                    .release_permit_neutral(
                                        &provider.id,
                                        app_type.as_str(),
                                        permit.used_half_open_permit,
                                    )
                                    .await;
                            }

                            if claude_error_path && !provider_needs_transform {
                                return Err(ForwardFailure::new(
                                    Some(provider),
                                    streaming_response_to_upstream_error(response),
                                ));
                            }

                            return Ok(ForwardedResponse { provider, response });
                        }
                        AttemptDecision::ProviderFailure => {
                            if !bypass_circuit_breaker {
                                let _ = self
                                    .router
                                    .record_result(
                                        &provider.id,
                                        app_type.as_str(),
                                        permit.used_half_open_permit,
                                        false,
                                        Some(format!(
                                            "upstream returned {}",
                                            response.status().as_u16()
                                        )),
                                    )
                                    .await;
                            }

                            if claude_error_path && !provider_needs_transform {
                                last_error = Some(ForwardFailure::new(
                                    Some(provider.clone()),
                                    streaming_response_to_upstream_error(response),
                                ));
                            } else {
                                pending_upstream_response =
                                    Some(ForwardedResponse { provider, response });
                                last_error = Some(ForwardFailure::new(
                                    pending_upstream_response
                                        .as_ref()
                                        .map(|response| response.provider.clone()),
                                    ProxyError::UpstreamError {
                                        status: pending_upstream_response
                                            .as_ref()
                                            .expect("pending upstream response")
                                            .response
                                            .status()
                                            .as_u16(),
                                        body: None,
                                    },
                                ));
                            }
                            continue;
                        }
                        _ => {
                            if !bypass_circuit_breaker {
                                let _ = self
                                    .router
                                    .record_result(
                                        &provider.id,
                                        app_type.as_str(),
                                        permit.used_half_open_permit,
                                        false,
                                        Some(format!(
                                            "upstream returned {}",
                                            response.status().as_u16()
                                        )),
                                    )
                                    .await;
                            }

                            return Ok(ForwardedResponse { provider, response });
                        }
                    }
                }
                Err(StreamingRequestError::AfterResponse(error)) => {
                    if !bypass_circuit_breaker {
                        self.router
                            .release_permit_neutral(
                                &provider.id,
                                app_type.as_str(),
                                permit.used_half_open_permit,
                            )
                            .await;
                    }
                    return Err(ForwardFailure::new(Some(provider), error));
                }
                Err(StreamingRequestError::BeforeResponse(error)) => {
                    match classify_attempt_error(&error) {
                        AttemptDecision::ProviderFailure => {
                            if !bypass_circuit_breaker {
                                let _ = self
                                    .router
                                    .record_result(
                                        &provider.id,
                                        app_type.as_str(),
                                        permit.used_half_open_permit,
                                        false,
                                        Some(error.to_string()),
                                    )
                                    .await;
                            }
                            last_error = Some(ForwardFailure::new(Some(provider.clone()), error));
                        }
                        AttemptDecision::NeutralRelease | AttemptDecision::FatalStop => {
                            if !bypass_circuit_breaker {
                                self.router
                                    .release_permit_neutral(
                                        &provider.id,
                                        app_type.as_str(),
                                        permit.used_half_open_permit,
                                    )
                                    .await;
                            }
                            return Err(ForwardFailure::new(Some(provider), error));
                        }
                    }
                }
            }
        }

        if let Some(response) = pending_upstream_response {
            return Ok(response);
        }

        if attempted_provider {
            Err(last_error.unwrap_or_else(|| {
                ForwardFailure::new(None, ProxyError::NoAvailableProvider)
            }))
        } else {
            Err(ForwardFailure::new(None, ProxyError::NoAvailableProvider))
        }
    }

    pub async fn forward_buffered_response(
        &self,
        app_type: &AppType,
        endpoint: &str,
        body: Value,
        headers: &HeaderMap,
        providers: Vec<Provider>,
        options: ForwardOptions,
        rectifier_config: RectifierConfig,
    ) -> Result<ForwardedResponse<BufferedResponse>, ProxyError> {
        self.forward_buffered_response_detailed(
            app_type,
            endpoint,
            body,
            headers,
            providers,
            options,
            rectifier_config,
        )
        .await
        .map_err(|failure| failure.error)
    }

    pub async fn forward_buffered_response_detailed(
        &self,
        app_type: &AppType,
        endpoint: &str,
        body: Value,
        headers: &HeaderMap,
        providers: Vec<Provider>,
        options: ForwardOptions,
        rectifier_config: RectifierConfig,
    ) -> Result<ForwardedResponse<BufferedResponse>, ForwardFailure> {
        if providers.is_empty() {
            return Err(ForwardFailure::new(None, ProxyError::NoAvailableProvider));
        }

        let claude_error_path = matches!(app_type, AppType::Claude);
        let bypass_circuit_breaker = options.bypass_circuit_breaker;
        let mut last_error = None;
        let mut attempted_provider = false;
        let mut pending_upstream_response = None;

        for provider in providers {
            let permit = if bypass_circuit_breaker {
                super::circuit_breaker::AllowResult {
                    allowed: true,
                    used_half_open_permit: false,
                }
            } else {
                self.router
                    .allow_provider_request(&provider.id, app_type.as_str())
                    .await
            };

            if !permit.allowed {
                continue;
            }

            attempted_provider = true;
            pending_upstream_response = None;
            let provider_needs_transform =
                matches!(app_type, AppType::Claude) && get_adapter(app_type).needs_transform(&provider);

            match self
                .send_buffered_request(
                    app_type,
                    &provider,
                    endpoint,
                    &body,
                    headers,
                    options,
                    &rectifier_config,
                )
                .await
            {
                Ok(outcome) => {
                    let response = outcome.response;
                    if response.status.is_success() {
                        if !bypass_circuit_breaker {
                            let _ = self
                                .router
                                .record_result(
                                    &provider.id,
                                    app_type.as_str(),
                                    permit.used_half_open_permit,
                                    true,
                                    None,
                                )
                                .await;
                        }

                        return Ok(ForwardedResponse { provider, response });
                    }

                    match outcome.attempt_decision {
                        AttemptDecision::NeutralRelease => {
                            if !bypass_circuit_breaker {
                                self.router
                                    .release_permit_neutral(
                                        &provider.id,
                                        app_type.as_str(),
                                        permit.used_half_open_permit,
                                    )
                                    .await;
                            }

                            if claude_error_path && !provider_needs_transform {
                                return Err(ForwardFailure::new(
                                    Some(provider),
                                    buffered_response_to_upstream_error(response),
                                ));
                            }

                            return Ok(ForwardedResponse { provider, response });
                        }
                        AttemptDecision::ProviderFailure => {
                            if !bypass_circuit_breaker {
                                let _ = self
                                    .router
                                    .record_result(
                                        &provider.id,
                                        app_type.as_str(),
                                        permit.used_half_open_permit,
                                        false,
                                        Some(format!(
                                            "upstream returned {}",
                                            response.status.as_u16()
                                        )),
                                    )
                                    .await;
                            }

                            if claude_error_path && !provider_needs_transform {
                                last_error = Some(ForwardFailure::new(
                                    Some(provider.clone()),
                                    buffered_response_to_upstream_error(response),
                                ));
                            } else {
                                pending_upstream_response =
                                    Some(ForwardedResponse { provider, response });
                                last_error = Some(ForwardFailure::new(
                                    pending_upstream_response
                                        .as_ref()
                                        .map(|response| response.provider.clone()),
                                    ProxyError::UpstreamError {
                                        status: pending_upstream_response
                                            .as_ref()
                                            .expect("pending upstream response")
                                            .response
                                            .status
                                            .as_u16(),
                                        body: None,
                                    },
                                ));
                            }
                            continue;
                        }
                        _ => {
                            if !bypass_circuit_breaker {
                                let _ = self
                                    .router
                                    .record_result(
                                        &provider.id,
                                        app_type.as_str(),
                                        permit.used_half_open_permit,
                                        false,
                                        Some(format!(
                                            "upstream returned {}",
                                            response.status.as_u16()
                                        )),
                                    )
                                    .await;
                            }

                            return Ok(ForwardedResponse { provider, response });
                        }
                    }
                }
                Err(BufferedRequestError::AfterResponse(error)) => {
                    if !bypass_circuit_breaker {
                        self.router
                            .release_permit_neutral(
                                &provider.id,
                                app_type.as_str(),
                                permit.used_half_open_permit,
                            )
                            .await;
                    }
                    return Err(ForwardFailure::new(Some(provider), error));
                }
                Err(BufferedRequestError::BeforeResponse(error)) => {
                    match classify_attempt_error(&error) {
                        AttemptDecision::ProviderFailure => {
                            if !bypass_circuit_breaker {
                                let _ = self
                                    .router
                                    .record_result(
                                        &provider.id,
                                        app_type.as_str(),
                                        permit.used_half_open_permit,
                                        false,
                                        Some(error.to_string()),
                                    )
                                    .await;
                            }
                            last_error = Some(ForwardFailure::new(Some(provider.clone()), error));
                        }
                        AttemptDecision::NeutralRelease | AttemptDecision::FatalStop => {
                            if !bypass_circuit_breaker {
                                self.router
                                    .release_permit_neutral(
                                        &provider.id,
                                        app_type.as_str(),
                                        permit.used_half_open_permit,
                                    )
                                    .await;
                            }
                            return Err(ForwardFailure::new(Some(provider), error));
                        }
                    }
                }
            }
        }

        if let Some(response) = pending_upstream_response {
            return Ok(response);
        }

        if attempted_provider {
            Err(last_error.unwrap_or_else(|| {
                ForwardFailure::new(None, ProxyError::NoAvailableProvider)
            }))
        } else {
            Err(ForwardFailure::new(None, ProxyError::NoAvailableProvider))
        }
    }

    async fn send_streaming_request(
        &self,
        app_type: &AppType,
        provider: &Provider,
        endpoint: &str,
        body: &Value,
        headers: &HeaderMap,
        options: ForwardOptions,
        rectifier_config: &RectifierConfig,
    ) -> Result<StreamingAttemptOutcome, StreamingRequestError> {
        let started_at = Instant::now();
        let allow_transport_retry = uses_internal_transport_retry(app_type);
        let mut request_body = body.clone();
        let mut rectifier_retried = false;

        'request_loop: loop {
            let base_request = self
                .prepare_request(
                    app_type,
                    provider,
                    endpoint,
                    &request_body,
                    headers,
                    options,
                )
                .map_err(StreamingRequestError::BeforeResponse)?;
            let mut attempt = 0u32;

            loop {
                let attempt_started_at = if allow_transport_retry {
                    Instant::now()
                } else {
                    started_at
                };
                let remaining_timeout = match options.request_timeout {
                    Some(request_timeout) => {
                        let remaining_timeout =
                            request_timeout.saturating_sub(attempt_started_at.elapsed());
                        if remaining_timeout.is_zero() {
                            let timeout_error = request_timeout_error(request_timeout);
                            return Err(if rectifier_retried {
                                StreamingRequestError::AfterResponse(timeout_error)
                            } else {
                                StreamingRequestError::BeforeResponse(timeout_error)
                            });
                        }
                        Some(remaining_timeout)
                    }
                    None => None,
                };

                let request =
                    clone_request(&base_request).map_err(StreamingRequestError::BeforeResponse)?;

                match match remaining_timeout {
                    Some(remaining_timeout) => {
                        tokio::time::timeout(remaining_timeout, request.send())
                            .await
                            .map_err(|_| ())
                            .map(|result| result)
                    }
                    None => Ok(request.send().await),
                } {
                    Ok(Ok(response)) => {
                        if response.status().is_success() {
                            return Ok(StreamingAttemptOutcome {
                                response: StreamingResponse::Live(response),
                                attempt_decision: AttemptDecision::FatalStop,
                            });
                        }

                        if should_buffer_streaming_error_response(app_type, response.status()) {
                            let buffered_response = read_streaming_error_response(
                                response,
                                attempt_started_at,
                                options.request_timeout,
                            )
                            .await
                            .map_err(StreamingRequestError::AfterResponse)?;

                            if !rectifier_retried {
                                if let Some(rectified_body) = maybe_rectify_claude_buffered_request(
                                    app_type,
                                    &buffered_response,
                                    &request_body,
                                    rectifier_config,
                                ) {
                                    rectifier_retried = true;
                                    request_body = rectified_body;
                                    continue 'request_loop;
                                }
                            }

                            return Ok(StreamingAttemptOutcome {
                                attempt_decision: classify_upstream_response(
                                    buffered_response.status,
                                    rectifier_retried,
                                ),
                                response: StreamingResponse::Buffered(buffered_response),
                            });
                        }

                        return Ok(StreamingAttemptOutcome {
                            attempt_decision: classify_upstream_response(
                                response.status(),
                                rectifier_retried,
                            ),
                            response: StreamingResponse::Live(response),
                        });
                    }
                    Ok(Err(error)) => {
                        if allow_transport_retry
                            && attempt < options.max_retries
                            && is_retryable_transport_error(&error)
                        {
                            attempt += 1;
                            continue;
                        }

                        let mapped_error = map_request_send_error(error, options.request_timeout);
                        return Err(if rectifier_retried {
                            StreamingRequestError::AfterResponse(mapped_error)
                        } else {
                            StreamingRequestError::BeforeResponse(mapped_error)
                        });
                    }
                    Err(_) => {
                        if allow_transport_retry && attempt < options.max_retries {
                            attempt += 1;
                            continue;
                        }

                        let timeout_error = request_timeout_error(
                            options
                                .request_timeout
                                .expect("request timeout should exist when timeout future errors"),
                        );
                        return Err(if rectifier_retried {
                            StreamingRequestError::AfterResponse(timeout_error)
                        } else {
                            StreamingRequestError::BeforeResponse(timeout_error)
                        });
                    }
                }
            }
        }
    }

    async fn send_buffered_request(
        &self,
        app_type: &AppType,
        provider: &Provider,
        endpoint: &str,
        body: &Value,
        headers: &HeaderMap,
        options: ForwardOptions,
        rectifier_config: &RectifierConfig,
    ) -> Result<BufferedAttemptOutcome, BufferedRequestError> {
        let mut request_body = body.clone();
        let mut rectifier_retried = false;
        let request_started_at = Instant::now();
        let allow_transport_retry = uses_internal_transport_retry(app_type);

        'request_loop: loop {
            let base_request = self
                .prepare_request(
                    app_type,
                    provider,
                    endpoint,
                    &request_body,
                    headers,
                    options,
                )
                .map_err(BufferedRequestError::BeforeResponse)?;
            let mut attempt = 0u32;

            loop {
                let attempt_started_at = if allow_transport_retry {
                    Instant::now()
                } else {
                    request_started_at
                };
                let remaining_timeout = match options.request_timeout {
                    Some(request_timeout) => {
                        let remaining_timeout =
                            request_timeout.saturating_sub(attempt_started_at.elapsed());
                        if remaining_timeout.is_zero() {
                            let timeout_error = request_timeout_error(request_timeout);
                            return Err(if rectifier_retried {
                                BufferedRequestError::AfterResponse(timeout_error)
                            } else {
                                BufferedRequestError::BeforeResponse(timeout_error)
                            });
                        }
                        Some(remaining_timeout)
                    }
                    None => None,
                };

                let request =
                    clone_request(&base_request).map_err(BufferedRequestError::BeforeResponse)?;

                match match remaining_timeout {
                    Some(remaining_timeout) => {
                        tokio::time::timeout(remaining_timeout, request.send())
                            .await
                            .map_err(|_| ())
                            .map(|result| result)
                    }
                    None => Ok(request.send().await),
                } {
                    Ok(Ok(response)) => {
                        let status = response.status();
                        let response_headers = response.headers().clone();
                        let response_body = match options.request_timeout {
                            Some(request_timeout) => {
                                let remaining_timeout =
                                    request_timeout.saturating_sub(attempt_started_at.elapsed());
                                if remaining_timeout.is_zero() {
                                    return Err(BufferedRequestError::AfterResponse(
                                        request_timeout_error(request_timeout),
                                    ));
                                }
                                tokio::time::timeout(remaining_timeout, response.bytes())
                                    .await
                                    .map_err(|_| {
                                        BufferedRequestError::AfterResponse(request_timeout_error(
                                            request_timeout,
                                        ))
                                    })?
                                    .map_err(|error| {
                                        BufferedRequestError::AfterResponse(map_request_send_error(
                                            error,
                                            Some(request_timeout),
                                        ))
                                    })?
                            }
                            None => response.bytes().await.map_err(|error| {
                                BufferedRequestError::AfterResponse(map_request_send_error(
                                    error, None,
                                ))
                            })?,
                        };

                        let buffered_response = BufferedResponse {
                            status,
                            headers: response_headers,
                            body: response_body,
                        };

                        if !rectifier_retried {
                            if let Some(rectified_body) = maybe_rectify_claude_buffered_request(
                                app_type,
                                &buffered_response,
                                &request_body,
                                rectifier_config,
                            ) {
                                rectifier_retried = true;
                                request_body = rectified_body;
                                continue 'request_loop;
                            }
                        }

                        return Ok(BufferedAttemptOutcome {
                            attempt_decision: classify_upstream_response(
                                buffered_response.status,
                                rectifier_retried,
                            ),
                            response: buffered_response,
                        });
                    }
                    Ok(Err(error)) => {
                        if allow_transport_retry
                            && attempt < options.max_retries
                            && is_retryable_transport_error(&error)
                        {
                            attempt += 1;
                            continue;
                        }

                        let mapped_error = map_request_send_error(error, options.request_timeout);
                        return Err(if rectifier_retried {
                            BufferedRequestError::AfterResponse(mapped_error)
                        } else {
                            BufferedRequestError::BeforeResponse(mapped_error)
                        });
                    }
                    Err(_) => {
                        if allow_transport_retry && attempt < options.max_retries {
                            attempt += 1;
                            continue;
                        }

                        let timeout_error = request_timeout_error(
                            options
                                .request_timeout
                                .expect("request timeout should exist when timeout future errors"),
                        );
                        return Err(if rectifier_retried {
                            BufferedRequestError::AfterResponse(timeout_error)
                        } else {
                            BufferedRequestError::BeforeResponse(timeout_error)
                        });
                    }
                }
            }
        }
    }

    fn prepare_request(
        &self,
        app_type: &AppType,
        provider: &Provider,
        endpoint: &str,
        body: &Value,
        headers: &HeaderMap,
        options: ForwardOptions,
    ) -> Result<reqwest::RequestBuilder, ProxyError> {
        let adapter = get_adapter(app_type);
        let upstream_endpoint = self.router.upstream_endpoint(app_type, provider, endpoint);
        let base_url = adapter.extract_base_url(provider)?;
        let (mut mapped_body, _, _) = apply_model_mapping(body.clone(), provider);
        if matches!(app_type, AppType::Claude)
            && self.optimizer_config.enabled
            && is_bedrock_provider(provider)
        {
            if self.optimizer_config.thinking_optimizer {
                super::thinking_optimizer::optimize(&mut mapped_body, &self.optimizer_config);
            }
            if self.optimizer_config.cache_injection {
                super::cache_injector::inject(&mut mapped_body, &self.optimizer_config);
            }
        }
        let request_body = if adapter.needs_transform(provider) {
            adapter.transform_request(mapped_body, provider)?
        } else {
            mapped_body
        };
        let filtered_body = filter_private_params_with_whitelist(request_body, &[]);
        let client = self.client_for_provider(provider);

        Ok(self.build_request(
            &client,
            &*adapter,
            provider,
            &base_url,
            &upstream_endpoint,
            &filtered_body,
            headers,
            options,
        ))
    }

    fn build_request(
        &self,
        client: &reqwest::Client,
        adapter: &dyn ProviderAdapter,
        provider: &Provider,
        base_url: &str,
        endpoint: &str,
        request_body: &Value,
        headers: &HeaderMap,
        _options: ForwardOptions,
    ) -> reqwest::RequestBuilder {
        let mut request = client.post(adapter.build_url(base_url, endpoint));

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

    fn client_for_provider(&self, provider: &Provider) -> reqwest::Client {
        http_client::get_for_provider(
            provider
                .meta
                .as_ref()
                .and_then(|meta| meta.proxy_config.as_ref()),
        )
    }
}

fn classify_attempt_error(error: &ProxyError) -> AttemptDecision {
    match error {
        ProxyError::UpstreamError {
            status: 400 | 422, ..
        } => AttemptDecision::NeutralRelease,
        ProxyError::AlreadyRunning
        | ProxyError::NotRunning
        | ProxyError::BindFailed(_)
        | ProxyError::StopTimeout
        | ProxyError::StopFailed(_)
        | ProxyError::NoAvailableProvider
        | ProxyError::AllProvidersCircuitOpen
        | ProxyError::NoProvidersConfigured
        | ProxyError::DatabaseError(_)
        | ProxyError::InvalidRequest(_)
        | ProxyError::Internal(_) => AttemptDecision::FatalStop,
        _ => AttemptDecision::ProviderFailure,
    }
}

fn maybe_rectify_claude_buffered_request(
    app_type: &AppType,
    response: &BufferedResponse,
    request_body: &Value,
    rectifier_config: &RectifierConfig,
) -> Option<Value> {
    if *app_type != AppType::Claude {
        return None;
    }

    if !matches!(response.status.as_u16(), 400 | 422) {
        return None;
    }

    let error_message = extract_upstream_error_message(&response.body);

    if should_rectify_thinking_signature(error_message.as_deref(), rectifier_config) {
        let mut rectified_body = request_body.clone();
        let result = rectify_anthropic_request(&mut rectified_body);
        if result.applied {
            return Some(normalize_thinking_type(rectified_body));
        }
    }

    if should_rectify_thinking_budget(error_message.as_deref(), rectifier_config) {
        let mut rectified_body = request_body.clone();
        let result = rectify_thinking_budget(&mut rectified_body);
        if result.applied {
            return Some(normalize_thinking_type(rectified_body));
        }
    }

    None
}

fn should_buffer_streaming_error_response(app_type: &AppType, status: reqwest::StatusCode) -> bool {
    *app_type == AppType::Claude && !status.is_success()
}

async fn read_streaming_error_response(
    response: reqwest::Response,
    started_at: Instant,
    request_timeout: Option<Duration>,
) -> Result<BufferedResponse, ProxyError> {
    let status = response.status();
    let headers = response.headers().clone();
    let body = match request_timeout {
        Some(request_timeout) => {
            let remaining_timeout = request_timeout.saturating_sub(started_at.elapsed());
            if remaining_timeout.is_zero() {
                return Err(stream_first_byte_timeout_error(request_timeout));
            }

            tokio::time::timeout(remaining_timeout, response.bytes())
                .await
                .map_err(|_| stream_first_byte_timeout_error(request_timeout))?
                .map_err(|error| map_request_send_error(error, Some(request_timeout)))?
        }
        None => response
            .bytes()
            .await
            .map_err(|error| map_request_send_error(error, None))?,
    };

    Ok(BufferedResponse {
        status,
        headers,
        body,
    })
}

fn extract_upstream_error_message(body: &[u8]) -> Option<String> {
    if let Ok(json_body) = serde_json::from_slice::<Value>(body) {
        return [
            json_body.pointer("/error/message"),
            json_body.pointer("/message"),
            json_body.pointer("/detail"),
            json_body.pointer("/error"),
        ]
        .into_iter()
        .flatten()
        .find_map(|value| value.as_str().map(ToString::to_string));
    }

    std::str::from_utf8(body)
        .ok()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn upstream_error_body_from_bytes(body: &[u8]) -> Option<String> {
    if body.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(body).into_owned())
    }
}

fn buffered_response_to_upstream_error(response: BufferedResponse) -> ProxyError {
    ProxyError::UpstreamError {
        status: response.status.as_u16(),
        body: upstream_error_body_from_bytes(&response.body),
    }
}

fn streaming_response_to_upstream_error(response: StreamingResponse) -> ProxyError {
    match response {
        StreamingResponse::Buffered(response) => buffered_response_to_upstream_error(response),
        StreamingResponse::Live(response) => ProxyError::UpstreamError {
            status: response.status().as_u16(),
            body: None,
        },
    }
}

fn clone_request(
    base_request: &reqwest::RequestBuilder,
) -> Result<reqwest::RequestBuilder, ProxyError> {
    base_request.try_clone().ok_or_else(|| {
        ProxyError::ForwardFailed("clone proxy request failed before retry".to_string())
    })
}

fn uses_internal_transport_retry(app_type: &AppType) -> bool {
    !matches!(app_type, AppType::Claude)
}

fn is_bedrock_provider(provider: &Provider) -> bool {
    provider
        .settings_config
        .get("env")
        .and_then(|env| env.get("CLAUDE_CODE_USE_BEDROCK"))
        .and_then(|value| value.as_str())
        .map(|value| value == "1")
        .unwrap_or(false)
}

fn is_retryable_transport_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

fn map_request_send_error(error: reqwest::Error, request_timeout: Option<Duration>) -> ProxyError {
    if error.is_timeout() {
        return match request_timeout {
            Some(request_timeout) => request_timeout_error(request_timeout),
            None => ProxyError::Timeout(error.to_string()),
        };
    }

    if error.is_connect() {
        return ProxyError::ForwardFailed(format!("connection failed: {error}"));
    }

    ProxyError::ForwardFailed(error.to_string())
}

fn request_timeout_error(request_timeout: Duration) -> ProxyError {
    ProxyError::Timeout(format!(
        "request timed out after {}s",
        request_timeout.as_secs()
    ))
}

fn stream_first_byte_timeout_error(request_timeout: Duration) -> ProxyError {
    let display_seconds = request_timeout
        .as_secs()
        .max(u64::from(!request_timeout.is_zero()));
    ProxyError::Timeout(format!("stream timeout after {}s", display_seconds))
}

fn classify_upstream_response(
    status: reqwest::StatusCode,
    rectifier_retried: bool,
) -> AttemptDecision {
    match status.as_u16() {
        400 | 422 if rectifier_retried => AttemptDecision::NeutralRelease,
        400 | 422 => AttemptDecision::ProviderFailure,
        _ => AttemptDecision::ProviderFailure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    use axum::{
        body::Body,
        extract::State,
        http::{StatusCode, Uri},
        response::{IntoResponse, Response},
        routing::any,
        Json, Router,
    };
    use serde_json::json;
    use tokio::{sync::Mutex, task::JoinHandle};

    use crate::{
        database::Database,
        proxy::{
            provider_router::ProviderRouter, response::is_sse_response, types::OptimizerConfig,
        },
    };

    #[derive(Clone, Default)]
    struct UpstreamHits {
        count: Arc<AtomicUsize>,
        paths: Arc<Mutex<Vec<String>>>,
    }

    #[derive(Clone)]
    struct MockUpstream {
        status: StatusCode,
        body: Value,
        hits: UpstreamHits,
    }

    #[derive(Clone)]
    struct ScriptedUpstream {
        responses: Arc<Mutex<VecDeque<(StatusCode, Value)>>>,
        hits: UpstreamHits,
        bodies: Arc<Mutex<Vec<Value>>>,
    }

    #[derive(Clone)]
    struct DelayedScriptedUpstream {
        responses: Arc<Mutex<VecDeque<(Duration, StatusCode, Value)>>>,
        hits: UpstreamHits,
        bodies: Arc<Mutex<Vec<Value>>>,
    }

    #[derive(Clone)]
    enum ScriptedStreamingBody {
        Json(Value),
        Sse(&'static str),
    }

    #[derive(Clone)]
    struct ScriptedStreamingUpstream {
        responses: Arc<Mutex<VecDeque<(StatusCode, ScriptedStreamingBody)>>>,
        hits: UpstreamHits,
        bodies: Arc<Mutex<Vec<Value>>>,
    }

    #[derive(Clone)]
    struct DelayedScriptedStreamingUpstream {
        responses: Arc<Mutex<VecDeque<(Duration, StatusCode, ScriptedStreamingBody)>>>,
        hits: UpstreamHits,
        bodies: Arc<Mutex<Vec<Value>>>,
    }

    async fn handle_mock_upstream(State(mock): State<MockUpstream>, uri: Uri) -> impl IntoResponse {
        mock.hits.count.fetch_add(1, Ordering::SeqCst);
        mock.hits.paths.lock().await.push(uri.path().to_string());
        (mock.status, Json(mock.body))
    }

    async fn spawn_mock_upstream(
        status: StatusCode,
        body: Value,
    ) -> (String, UpstreamHits, JoinHandle<()>) {
        let hits = UpstreamHits::default();
        let mock = MockUpstream {
            status,
            body,
            hits: hits.clone(),
        };
        let app = Router::new()
            .route("/*path", any(handle_mock_upstream))
            .with_state(mock);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind upstream listener");
        let address = listener.local_addr().expect("upstream listener address");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        (format!("http://{address}"), hits, handle)
    }

    async fn handle_scripted_upstream(
        State(mock): State<ScriptedUpstream>,
        uri: Uri,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        mock.hits.count.fetch_add(1, Ordering::SeqCst);
        mock.hits.paths.lock().await.push(uri.path().to_string());
        mock.bodies.lock().await.push(body);

        let (status, body) = mock.responses.lock().await.pop_front().unwrap_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": "missing scripted response"}),
        ));
        (status, Json(body))
    }

    async fn spawn_scripted_upstream(
        responses: Vec<(StatusCode, Value)>,
    ) -> (String, UpstreamHits, Arc<Mutex<Vec<Value>>>, JoinHandle<()>) {
        let hits = UpstreamHits::default();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let mock = ScriptedUpstream {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            hits: hits.clone(),
            bodies: bodies.clone(),
        };
        let app = Router::new()
            .route("/*path", any(handle_scripted_upstream))
            .with_state(mock);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind scripted upstream listener");
        let address = listener
            .local_addr()
            .expect("scripted upstream listener address");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        (format!("http://{address}"), hits, bodies, handle)
    }

    async fn handle_delayed_scripted_upstream(
        State(mock): State<DelayedScriptedUpstream>,
        uri: Uri,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        mock.hits.count.fetch_add(1, Ordering::SeqCst);
        mock.hits.paths.lock().await.push(uri.path().to_string());
        mock.bodies.lock().await.push(body);

        let (delay, status, body) = mock.responses.lock().await.pop_front().unwrap_or((
            Duration::from_millis(0),
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": "missing scripted response"}),
        ));
        tokio::time::sleep(delay).await;
        (status, Json(body))
    }

    async fn spawn_delayed_scripted_upstream(
        responses: Vec<(Duration, StatusCode, Value)>,
    ) -> (String, UpstreamHits, Arc<Mutex<Vec<Value>>>, JoinHandle<()>) {
        let hits = UpstreamHits::default();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let mock = DelayedScriptedUpstream {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            hits: hits.clone(),
            bodies: bodies.clone(),
        };
        let app = Router::new()
            .route("/*path", any(handle_delayed_scripted_upstream))
            .with_state(mock);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind delayed scripted upstream listener");
        let address = listener
            .local_addr()
            .expect("delayed scripted upstream listener address");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        (format!("http://{address}"), hits, bodies, handle)
    }

    async fn handle_scripted_streaming_upstream(
        State(mock): State<ScriptedStreamingUpstream>,
        uri: Uri,
        Json(body): Json<Value>,
    ) -> Response {
        mock.hits.count.fetch_add(1, Ordering::SeqCst);
        mock.hits.paths.lock().await.push(uri.path().to_string());
        mock.bodies.lock().await.push(body);

        let (status, body) = mock.responses.lock().await.pop_front().unwrap_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            ScriptedStreamingBody::Json(json!({"error": {"message": "missing scripted response"}})),
        ));

        match body {
            ScriptedStreamingBody::Json(body) => (status, Json(body)).into_response(),
            ScriptedStreamingBody::Sse(body) => Response::builder()
                .status(status)
                .header("content-type", "text/event-stream")
                .body(Body::from(body))
                .expect("build scripted streaming response"),
        }
    }

    async fn spawn_scripted_streaming_upstream(
        responses: Vec<(StatusCode, ScriptedStreamingBody)>,
    ) -> (String, UpstreamHits, Arc<Mutex<Vec<Value>>>, JoinHandle<()>) {
        let hits = UpstreamHits::default();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let mock = ScriptedStreamingUpstream {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            hits: hits.clone(),
            bodies: bodies.clone(),
        };
        let app = Router::new()
            .route("/*path", any(handle_scripted_streaming_upstream))
            .with_state(mock);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind scripted streaming upstream listener");
        let address = listener
            .local_addr()
            .expect("scripted streaming upstream listener address");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        (format!("http://{address}"), hits, bodies, handle)
    }

    async fn handle_delayed_scripted_streaming_upstream(
        State(mock): State<DelayedScriptedStreamingUpstream>,
        uri: Uri,
        Json(body): Json<Value>,
    ) -> Response {
        mock.hits.count.fetch_add(1, Ordering::SeqCst);
        mock.hits.paths.lock().await.push(uri.path().to_string());
        mock.bodies.lock().await.push(body);

        let (delay, status, body) = mock.responses.lock().await.pop_front().unwrap_or((
            Duration::from_millis(0),
            StatusCode::INTERNAL_SERVER_ERROR,
            ScriptedStreamingBody::Json(json!({"error": {"message": "missing scripted response"}})),
        ));
        tokio::time::sleep(delay).await;

        match body {
            ScriptedStreamingBody::Json(body) => (status, Json(body)).into_response(),
            ScriptedStreamingBody::Sse(body) => Response::builder()
                .status(status)
                .header("content-type", "text/event-stream")
                .body(Body::from(body))
                .expect("build delayed scripted streaming response"),
        }
    }

    async fn spawn_delayed_scripted_streaming_upstream(
        responses: Vec<(Duration, StatusCode, ScriptedStreamingBody)>,
    ) -> (String, UpstreamHits, Arc<Mutex<Vec<Value>>>, JoinHandle<()>) {
        let hits = UpstreamHits::default();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let mock = DelayedScriptedStreamingUpstream {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            hits: hits.clone(),
            bodies: bodies.clone(),
        };
        let app = Router::new()
            .route("/*path", any(handle_delayed_scripted_streaming_upstream))
            .with_state(mock);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind delayed scripted streaming upstream listener");
        let address = listener
            .local_addr()
            .expect("delayed scripted streaming upstream listener address");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        (format!("http://{address}"), hits, bodies, handle)
    }

    async fn handle_delayed_body_upstream(State(hits): State<UpstreamHits>, uri: Uri) -> Response {
        hits.count.fetch_add(1, Ordering::SeqCst);
        hits.paths.lock().await.push(uri.path().to_string());

        let stream = async_stream::stream! {
            tokio::time::sleep(Duration::from_millis(150)).await;
            yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(br#"{"ok":true}"#));
        };

        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from_stream(stream))
            .expect("build delayed body response")
    }

    async fn spawn_delayed_body_upstream() -> (String, UpstreamHits, JoinHandle<()>) {
        let hits = UpstreamHits::default();
        let app = Router::new()
            .route("/*path", any(handle_delayed_body_upstream))
            .with_state(hits.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind upstream listener");
        let address = listener.local_addr().expect("upstream listener address");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        (format!("http://{address}"), hits, handle)
    }

    async fn closed_base_url() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind closed-port listener");
        let address = listener.local_addr().expect("closed-port listener address");
        drop(listener);
        format!("http://{address}")
    }

    fn claude_provider(id: &str, base_url: &str, api_format: Option<&str>) -> Provider {
        let mut settings = json!({
            "base_url": base_url,
            "apiKey": format!("key-{id}"),
        });
        if let Some(api_format) = api_format {
            settings["api_format"] = json!(api_format);
        }

        Provider::with_id(id.to_string(), format!("Provider {id}"), settings, None)
    }

    fn bedrock_claude_provider(id: &str, base_url: &str) -> Provider {
        let settings = json!({
            "env": {
                "ANTHROPIC_BASE_URL": base_url,
                "ANTHROPIC_API_KEY": format!("key-{id}"),
                "CLAUDE_CODE_USE_BEDROCK": "1"
            }
        });

        Provider::with_id(id.to_string(), format!("Bedrock {id}"), settings, None)
    }

    fn claude_request_body() -> Value {
        json!({
            "model": "claude-3-7-sonnet-20250219",
            "max_tokens": 32,
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": "hello"
                }]
            }]
        })
    }

    async fn test_router() -> (Arc<Database>, Arc<ProviderRouter>) {
        let db = Arc::new(Database::memory().expect("memory db"));
        let mut config = db
            .get_proxy_config_for_app("claude")
            .await
            .expect("load proxy config");
        config.circuit_failure_threshold = 1;
        config.circuit_timeout_seconds = 0;
        db.update_proxy_config_for_app(config)
            .await
            .expect("update proxy config");
        let router = Arc::new(ProviderRouter::new(db.clone()));
        (db, router)
    }

    #[tokio::test]
    async fn single_provider_bypasses_open_breaker() {
        let (base_url, hits, server) =
            spawn_mock_upstream(StatusCode::OK, json!({"ok": true})).await;
        let provider = claude_provider("p1", &base_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router.clone()).expect("create forwarder");

        db.save_provider("claude", &provider)
            .expect("save provider for health tracking");
        let mut config = db
            .get_proxy_config_for_app("claude")
            .await
            .expect("load proxy config");
        config.circuit_timeout_seconds = 3600;
        db.update_proxy_config_for_app(config)
            .await
            .expect("update proxy timeout");

        router
            .record_result(
                "p1",
                "claude",
                false,
                false,
                Some("open breaker".to_string()),
            )
            .await
            .expect("open breaker");
        assert!(!router.allow_provider_request("p1", "claude").await.allowed);

        let result = forwarder
            .forward_buffered_response(
                &AppType::Claude,
                "/v1/messages",
                claude_request_body(),
                &HeaderMap::new(),
                vec![provider.clone()],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_secs(2)),
                    bypass_circuit_breaker: true,
                },
                RectifierConfig::default(),
            )
            .await
            .expect("single provider request should succeed");

        assert_eq!(result.provider.id, provider.id);
        assert_eq!(hits.count.load(Ordering::SeqCst), 1);

        server.abort();
    }

    #[tokio::test]
    async fn claude_buffered_failover_uses_second_provider_and_per_provider_endpoint() {
        let (primary_url, primary_hits, primary_server) = spawn_mock_upstream(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": {"message": "primary down"}}),
        )
        .await;
        let (secondary_url, secondary_hits, secondary_server) =
            spawn_mock_upstream(StatusCode::OK, json!({"id": "resp_123", "ok": true})).await;
        let provider_one = claude_provider("p1", &primary_url, Some("openai_chat"));
        let provider_two = claude_provider("p2", &secondary_url, Some("openai_chat"));
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router.clone()).expect("create forwarder");

        db.save_provider("claude", &provider_one)
            .expect("save primary provider for health tracking");
        db.save_provider("claude", &provider_two)
            .expect("save secondary provider for health tracking");

        let result = forwarder
            .forward_buffered_response(
                &AppType::Claude,
                "/v1/messages",
                claude_request_body(),
                &HeaderMap::new(),
                vec![provider_one, provider_two.clone()],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_secs(2)),
                    bypass_circuit_breaker: false,
                },
                RectifierConfig::default(),
            )
            .await
            .expect("second provider should succeed");

        assert_eq!(result.provider.id, provider_two.id);
        assert_eq!(primary_hits.count.load(Ordering::SeqCst), 1);
        assert_eq!(secondary_hits.count.load(Ordering::SeqCst), 1);
        assert_eq!(
            primary_hits.paths.lock().await.as_slice(),
            ["/v1/chat/completions"]
        );
        assert_eq!(
            secondary_hits.paths.lock().await.as_slice(),
            ["/v1/chat/completions"]
        );

        primary_server.abort();
        secondary_server.abort();
    }

    #[tokio::test]
    async fn plain_buffered_400_fails_over_to_next_provider() {
        let (primary_url, primary_hits, primary_server) = spawn_mock_upstream(
            StatusCode::BAD_REQUEST,
            json!({"error": {"message": "bad request"}}),
        )
        .await;
        let (secondary_url, secondary_hits, secondary_server) =
            spawn_mock_upstream(StatusCode::OK, json!({"ok": true})).await;
        let provider_one = claude_provider("p1", &primary_url, None);
        let provider_two = claude_provider("p2", &secondary_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router.clone()).expect("create forwarder");

        db.save_provider("claude", &provider_one)
            .expect("save primary provider for health tracking");
        db.save_provider("claude", &provider_two)
            .expect("save secondary provider for health tracking");

        router
            .record_result(
                "p1",
                "claude",
                false,
                false,
                Some("open breaker".to_string()),
            )
            .await
            .expect("open breaker");

        let result = forwarder
            .forward_buffered_response(
                &AppType::Claude,
                "/v1/messages",
                claude_request_body(),
                &HeaderMap::new(),
                vec![provider_one, provider_two],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_secs(2)),
                    bypass_circuit_breaker: false,
                },
                RectifierConfig::default(),
            )
            .await
            .expect("plain 400 should fail over to the next provider");

        assert_eq!(result.provider.id, "p2");
        assert_eq!(result.response.status, StatusCode::OK);
        assert_eq!(primary_hits.count.load(Ordering::SeqCst), 1);
        assert_eq!(secondary_hits.count.load(Ordering::SeqCst), 1);

        let permit = router.allow_provider_request("p1", "claude").await;
        assert!(permit.allowed);
        assert!(permit.used_half_open_permit);

        primary_server.abort();
        secondary_server.abort();
    }

    #[tokio::test]
    async fn single_provider_buffered_claude_non_2xx_returns_upstream_error() {
        let (primary_url, primary_hits, primary_server) = spawn_mock_upstream(
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error": {"message": "rate limited"}}),
        )
        .await;
        let provider = claude_provider("p1", &primary_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router.clone()).expect("create forwarder");

        db.save_provider("claude", &provider)
            .expect("save provider for health tracking");

        let error = forwarder
            .forward_buffered_response(
                &AppType::Claude,
                "/v1/messages",
                claude_request_body(),
                &HeaderMap::new(),
                vec![provider],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_secs(2)),
                    bypass_circuit_breaker: false,
                },
                RectifierConfig::default(),
            )
            .await
            .expect_err("single-provider Claude 429 should surface as UpstreamError");

        match error {
            ProxyError::UpstreamError { status, body } => {
                assert_eq!(status, 429);
                assert_eq!(
                    body.as_deref(),
                    Some(r#"{"error":{"message":"rate limited"}}"#)
                );
            }
            other => panic!("expected UpstreamError, got {other:?}"),
        }
        assert_eq!(primary_hits.count.load(Ordering::SeqCst), 1);

        primary_server.abort();
    }

    #[tokio::test]
    async fn claude_buffered_rectifier_owned_400_stops_before_next_provider() {
        let (primary_url, primary_hits, primary_bodies, primary_server) = spawn_scripted_upstream(vec![
            (
                StatusCode::BAD_REQUEST,
                json!({"error": {"message": "messages.1.content.0: Invalid `signature` in `thinking` block"}}),
            ),
            (
                StatusCode::BAD_REQUEST,
                json!({"error": {"message": "messages.1.content.0: Invalid `signature` in `thinking` block"}}),
            ),
        ])
        .await;
        let (secondary_url, secondary_hits, secondary_server) =
            spawn_mock_upstream(StatusCode::OK, json!({"ok": true})).await;
        let provider_one = claude_provider("p1", &primary_url, None);
        let provider_two = claude_provider("p2", &secondary_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router.clone()).expect("create forwarder");

        db.save_provider("claude", &provider_one)
            .expect("save primary provider for health tracking");
        db.save_provider("claude", &provider_two)
            .expect("save secondary provider for health tracking");
        router
            .record_result(
                "p1",
                "claude",
                false,
                false,
                Some("open breaker".to_string()),
            )
            .await
            .expect("open primary breaker");

        let body = json!({
            "model": "claude-3-7-sonnet-20250219",
            "max_tokens": 32,
            "messages": [{
                "role": "assistant",
                "content": [
                    { "type": "thinking", "thinking": "t", "signature": "sig" },
                    { "type": "text", "text": "hello", "signature": "text-sig" }
                ]
            }]
        });

        let error = forwarder
            .forward_buffered_response(
                &AppType::Claude,
                "/v1/messages",
                body,
                &HeaderMap::new(),
                vec![provider_one, provider_two],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_secs(2)),
                    bypass_circuit_breaker: false,
                },
                RectifierConfig::default(),
            )
            .await
            .expect_err("rectifier-owned 400 should surface as UpstreamError");

        match error {
            ProxyError::UpstreamError { status, body } => {
                assert_eq!(status, 400);
                assert!(body
                    .as_deref()
                    .expect("rectifier-owned 400 should preserve body")
                    .contains("Invalid `signature`"));
            }
            other => panic!("expected UpstreamError, got {other:?}"),
        }
        assert_eq!(primary_hits.count.load(Ordering::SeqCst), 2);
        assert_eq!(secondary_hits.count.load(Ordering::SeqCst), 0);

        let sent_bodies = primary_bodies.lock().await;
        assert_eq!(sent_bodies.len(), 2);

        let permit = router.allow_provider_request("p1", "claude").await;
        assert!(permit.allowed);
        assert!(permit.used_half_open_permit);

        primary_server.abort();
        secondary_server.abort();
    }

    #[tokio::test]
    async fn plain_streaming_422_json_error_fails_over_to_next_provider() {
        let (primary_url, primary_hits, primary_bodies, primary_server) =
            spawn_scripted_streaming_upstream(vec![(
                StatusCode::UNPROCESSABLE_ENTITY,
                ScriptedStreamingBody::Json(json!({"error": {"message": "unprocessable request"}})),
            )])
            .await;
        let (secondary_url, secondary_hits, secondary_bodies, secondary_server) =
            spawn_scripted_streaming_upstream(vec![(
                StatusCode::OK,
                ScriptedStreamingBody::Sse(
                    "data: {\"id\":\"msg_123\",\"type\":\"message_start\"}\n\ndata: [DONE]\n\n",
                ),
            )])
            .await;
        let provider_one = claude_provider("p1", &primary_url, None);
        let provider_two = claude_provider("p2", &secondary_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router).expect("create forwarder");

        db.save_provider("claude", &provider_one)
            .expect("save primary provider for health tracking");
        db.save_provider("claude", &provider_two)
            .expect("save secondary provider for health tracking");

        let body = json!({
            "model": "claude-3-7-sonnet-20250219",
            "stream": true,
            "max_tokens": 32,
            "messages": [{
                "role": "user",
                "content": [{ "type": "text", "text": "hello" }]
            }]
        });

        let result = forwarder
            .forward_response(
                &AppType::Claude,
                "/v1/messages",
                body,
                &HeaderMap::new(),
                vec![provider_one, provider_two],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_secs(2)),
                    bypass_circuit_breaker: false,
                },
                RectifierConfig::default(),
            )
            .await
            .expect("plain streaming 422 should fail over to next provider");

        assert_eq!(result.provider.id, "p2");
        assert_eq!(result.response.status(), StatusCode::OK);
        assert!(matches!(
            &result.response,
            StreamingResponse::Live(response) if is_sse_response(response)
        ));
        assert_eq!(primary_hits.count.load(Ordering::SeqCst), 1);
        assert_eq!(secondary_hits.count.load(Ordering::SeqCst), 1);
        assert_eq!(primary_bodies.lock().await.len(), 1);
        assert_eq!(secondary_bodies.lock().await.len(), 1);

        primary_server.abort();
        secondary_server.abort();
    }

    #[tokio::test]
    async fn last_provider_429_returns_upstream_error() {
        let (primary_url, _primary_hits, primary_server) = spawn_mock_upstream(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": {"message": "primary down"}}),
        )
        .await;
        let (secondary_url, _secondary_hits, secondary_server) = spawn_mock_upstream(
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error": {"message": "rate limited"}}),
        )
        .await;
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router).expect("create forwarder");
        let provider_one = claude_provider("p1", &primary_url, None);
        let provider_two = claude_provider("p2", &secondary_url, None);

        db.save_provider("claude", &provider_one)
            .expect("save primary provider for health tracking");
        db.save_provider("claude", &provider_two)
            .expect("save secondary provider for health tracking");

        let error = forwarder
            .forward_buffered_response(
                &AppType::Claude,
                "/v1/messages",
                claude_request_body(),
                &HeaderMap::new(),
                vec![provider_one, provider_two],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_secs(2)),
                    bypass_circuit_breaker: false,
                },
                RectifierConfig::default(),
            )
            .await
            .expect_err("last provider 429 should surface as UpstreamError");

        match error {
            ProxyError::UpstreamError { status, body } => {
                assert_eq!(status, 429);
                let parsed: Value =
                    serde_json::from_str(body.as_deref().expect("preserve upstream body"))
                        .expect("parse body");
                assert_eq!(parsed, json!({"error": {"message": "rate limited"}}));
            }
            other => panic!("expected UpstreamError, got {other:?}"),
        }

        primary_server.abort();
        secondary_server.abort();
    }

    #[tokio::test]
    async fn last_streaming_provider_429_returns_upstream_error() {
        let (primary_url, _primary_hits, _primary_bodies, primary_server) =
            spawn_scripted_streaming_upstream(vec![(
                StatusCode::INTERNAL_SERVER_ERROR,
                ScriptedStreamingBody::Json(json!({"error": {"message": "primary down"}})),
            )])
            .await;
        let (secondary_url, _secondary_hits, _secondary_bodies, secondary_server) =
            spawn_scripted_streaming_upstream(vec![(
                StatusCode::TOO_MANY_REQUESTS,
                ScriptedStreamingBody::Json(json!({"error": {"message": "rate limited"}})),
            )])
            .await;
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router).expect("create forwarder");
        let provider_one = claude_provider("p1", &primary_url, None);
        let provider_two = claude_provider("p2", &secondary_url, None);

        db.save_provider("claude", &provider_one)
            .expect("save primary provider for health tracking");
        db.save_provider("claude", &provider_two)
            .expect("save secondary provider for health tracking");

        let error = forwarder
            .forward_response(
                &AppType::Claude,
                "/v1/messages",
                json!({
                    "model": "claude-3-7-sonnet-20250219",
                    "stream": true,
                    "max_tokens": 32,
                    "messages": [{"role": "user", "content": [{"type": "text", "text": "hello"}]}]
                }),
                &HeaderMap::new(),
                vec![provider_one, provider_two],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_secs(2)),
                    bypass_circuit_breaker: false,
                },
                RectifierConfig::default(),
            )
            .await
            .expect_err("last streaming provider 429 should surface as UpstreamError");

        match error {
            ProxyError::UpstreamError { status, body } => {
                assert_eq!(status, 429);
                let parsed: Value =
                    serde_json::from_str(body.as_deref().expect("preserve upstream body"))
                        .expect("parse body");
                assert_eq!(parsed, json!({"error": {"message": "rate limited"}}));
            }
            other => panic!("expected UpstreamError, got {other:?}"),
        }

        primary_server.abort();
        secondary_server.abort();
    }

    #[tokio::test]
    async fn single_candidate_with_failover_enabled_still_honors_open_breaker() {
        let (base_url, hits, server) =
            spawn_mock_upstream(StatusCode::OK, json!({"ok": true})).await;
        let provider = claude_provider("p1", &base_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router.clone()).expect("create forwarder");

        db.save_provider("claude", &provider)
            .expect("save provider for health tracking");
        let mut config = db
            .get_proxy_config_for_app("claude")
            .await
            .expect("load proxy config");
        config.circuit_timeout_seconds = 3600;
        db.update_proxy_config_for_app(config)
            .await
            .expect("update proxy timeout");

        router
            .record_result(
                "p1",
                "claude",
                false,
                false,
                Some("open breaker".to_string()),
            )
            .await
            .expect("open breaker");

        let error = forwarder
            .forward_buffered_response(
                &AppType::Claude,
                "/v1/messages",
                claude_request_body(),
                &HeaderMap::new(),
                vec![provider],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_secs(2)),
                    bypass_circuit_breaker: false,
                },
                RectifierConfig::default(),
            )
            .await
            .expect_err("failover-enabled single candidate should still honor breaker");

        assert!(matches!(error, ProxyError::NoAvailableProvider));
        assert_eq!(hits.count.load(Ordering::SeqCst), 0);

        server.abort();
    }

    #[tokio::test]
    async fn buffered_timeout_includes_body_read_budget_after_headers() {
        let (base_url, hits, server) = spawn_delayed_body_upstream().await;
        let provider = claude_provider("p1", &base_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router).expect("create forwarder");

        db.save_provider("claude", &provider)
            .expect("save provider for health tracking");

        let error = forwarder
            .forward_buffered_response(
                &AppType::Claude,
                "/v1/messages",
                claude_request_body(),
                &HeaderMap::new(),
                vec![provider],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_millis(50)),
                    bypass_circuit_breaker: true,
                },
                RectifierConfig::default(),
            )
            .await
            .expect_err("buffered request should time out while waiting for body");

        assert!(
            matches!(error, ProxyError::Timeout(message) if message.contains("request timed out"))
        );
        assert_eq!(hits.count.load(Ordering::SeqCst), 1);

        server.abort();
    }

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
    async fn buffered_body_timeout_after_response_does_not_failover() {
        let (slow_url, slow_hits, slow_server) = spawn_delayed_body_upstream().await;
        let (fallback_url, fallback_hits, fallback_server) =
            spawn_mock_upstream(StatusCode::OK, json!({"ok": true})).await;
        let slow_provider = claude_provider("p1", &slow_url, None);
        let fallback_provider = claude_provider("p2", &fallback_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router).expect("create forwarder");

        db.save_provider("claude", &slow_provider)
            .expect("save slow provider for health tracking");
        db.save_provider("claude", &fallback_provider)
            .expect("save fallback provider for health tracking");

        let error = forwarder
            .forward_buffered_response(
                &AppType::Claude,
                "/v1/messages",
                claude_request_body(),
                &HeaderMap::new(),
                vec![slow_provider, fallback_provider],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_millis(50)),
                    bypass_circuit_breaker: false,
                },
                RectifierConfig::default(),
            )
            .await
            .expect_err("body timeout after response should stop provider failover");

        assert!(
            matches!(error, ProxyError::Timeout(message) if message.contains("request timed out"))
        );
        assert_eq!(slow_hits.count.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_hits.count.load(Ordering::SeqCst), 0);

        slow_server.abort();
        fallback_server.abort();
    }

    #[tokio::test]
    async fn buffered_transport_retry_shares_request_timeout_budget() {
        let (base_url, hits, _bodies, server) = spawn_delayed_scripted_upstream(vec![
            (
                Duration::from_millis(100),
                StatusCode::OK,
                json!({"id": "first-attempt"}),
            ),
            (
                Duration::from_millis(0),
                StatusCode::OK,
                json!({"id": "second-attempt"}),
            ),
        ])
        .await;
        let provider = claude_provider("p1", &base_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router).expect("create forwarder");

        db.save_provider("claude", &provider)
            .expect("save provider for health tracking");

        let error = forwarder
            .forward_buffered_response(
                &AppType::Claude,
                "/v1/messages",
                claude_request_body(),
                &HeaderMap::new(),
                vec![provider],
                ForwardOptions {
                    max_retries: 1,
                    request_timeout: Some(Duration::from_millis(50)),
                    bypass_circuit_breaker: true,
                },
                RectifierConfig::default(),
            )
            .await
            .expect_err("transport retry should share a single buffered request timeout budget");

        assert!(
            matches!(error, ProxyError::Timeout(message) if message.contains("request timed out"))
        );
        assert_eq!(hits.count.load(Ordering::SeqCst), 1);

        server.abort();
    }

    #[tokio::test]
    async fn streaming_transport_timeout_fails_over_without_same_provider_retry() {
        let (primary_url, primary_hits, primary_bodies, primary_server) =
            spawn_delayed_scripted_streaming_upstream(vec![
                (
                    Duration::from_millis(100),
                    StatusCode::OK,
                    ScriptedStreamingBody::Sse(
                        "data: {\"id\":\"primary-retry\",\"type\":\"message_start\"}\n\ndata: [DONE]\n\n",
                    ),
                ),
                (
                    Duration::from_millis(0),
                    StatusCode::OK,
                    ScriptedStreamingBody::Sse(
                        "data: {\"id\":\"primary-second\",\"type\":\"message_start\"}\n\ndata: [DONE]\n\n",
                    ),
                ),
            ])
            .await;
        let (secondary_url, secondary_hits, secondary_bodies, secondary_server) =
            spawn_delayed_scripted_streaming_upstream(vec![(
                Duration::from_millis(0),
                StatusCode::OK,
                ScriptedStreamingBody::Sse(
                    "data: {\"id\":\"secondary\",\"type\":\"message_start\"}\n\ndata: [DONE]\n\n",
                ),
            )])
            .await;
        let provider_one = claude_provider("p1", &primary_url, None);
        let provider_two = claude_provider("p2", &secondary_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router).expect("create forwarder");

        db.save_provider("claude", &provider_one)
            .expect("save primary provider for health tracking");
        db.save_provider("claude", &provider_two)
            .expect("save secondary provider for health tracking");

        let result = forwarder
            .forward_response(
                &AppType::Claude,
                "/v1/messages",
                json!({
                    "model": "claude-3-7-sonnet-20250219",
                    "stream": true,
                    "max_tokens": 32,
                    "messages": [{
                        "role": "user",
                        "content": [{ "type": "text", "text": "hello" }]
                    }]
                }),
                &HeaderMap::new(),
                vec![provider_one, provider_two],
                ForwardOptions {
                    max_retries: 1,
                    request_timeout: Some(Duration::from_millis(50)),
                    bypass_circuit_breaker: true,
                },
                RectifierConfig::default(),
            )
            .await
            .expect("transport timeout should fail over to next provider");

        assert_eq!(result.provider.id, "p2");
        assert_eq!(result.response.status(), StatusCode::OK);
        assert_eq!(primary_hits.count.load(Ordering::SeqCst), 1);
        assert_eq!(secondary_hits.count.load(Ordering::SeqCst), 1);
        assert_eq!(primary_bodies.lock().await.len(), 1);
        assert_eq!(secondary_bodies.lock().await.len(), 1);

        primary_server.abort();
        secondary_server.abort();
    }

    #[tokio::test]
    async fn buffered_connect_error_maps_to_forward_failed() {
        let provider = claude_provider("p1", &closed_base_url().await, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router).expect("create forwarder");

        db.save_provider("claude", &provider)
            .expect("save provider for health tracking");

        let error = forwarder
            .forward_buffered_response(
                &AppType::Claude,
                "/v1/messages",
                claude_request_body(),
                &HeaderMap::new(),
                vec![provider],
                ForwardOptions {
                    max_retries: 2,
                    request_timeout: Some(Duration::from_secs(1)),
                    bypass_circuit_breaker: true,
                },
                RectifierConfig::default(),
            )
            .await
            .expect_err("connect failures should map to forward failed");

        assert!(matches!(error, ProxyError::ForwardFailed(_)));
    }

    #[tokio::test]
    async fn buffered_rectifier_retry_shares_request_timeout_budget() {
        let (base_url, hits, bodies, server) = spawn_delayed_scripted_upstream(vec![
            (
                Duration::from_millis(20),
                StatusCode::BAD_REQUEST,
                json!({"error": {"message": "messages.1.content.0: Invalid `signature` in `thinking` block"}}),
            ),
            (
                Duration::from_millis(40),
                StatusCode::OK,
                json!({"id": "msg_123", "content": []}),
            ),
        ])
        .await;
        let provider = claude_provider("p1", &base_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router).expect("create forwarder");

        db.save_provider("claude", &provider)
            .expect("save provider for health tracking");

        let body = json!({
            "model": "claude-3-7-sonnet-20250219",
            "max_tokens": 32,
            "messages": [{
                "role": "assistant",
                "content": [
                    { "type": "thinking", "thinking": "t", "signature": "sig" },
                    { "type": "text", "text": "hello", "signature": "text-sig" }
                ]
            }]
        });

        let error = forwarder
            .forward_buffered_response(
                &AppType::Claude,
                "/v1/messages",
                body,
                &HeaderMap::new(),
                vec![provider],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_millis(50)),
                    bypass_circuit_breaker: true,
                },
                RectifierConfig::default(),
            )
            .await
            .expect_err("rectifier retry should share a single buffered request timeout budget");

        assert!(
            matches!(error, ProxyError::Timeout(message) if message.contains("request timed out"))
        );
        assert_eq!(hits.count.load(Ordering::SeqCst), 2);
        assert_eq!(bodies.lock().await.len(), 2);

        server.abort();
    }

    #[tokio::test]
    async fn skipped_candidates_preserve_last_attempted_upstream_response() {
        let (primary_url, primary_hits, primary_server) = spawn_mock_upstream(
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error": {"message": "rate limited"}}),
        )
        .await;
        let (skipped_url, skipped_hits, skipped_server) =
            spawn_mock_upstream(StatusCode::OK, json!({"ok": true})).await;
        let primary_provider = claude_provider("p1", &primary_url, None);
        let skipped_provider = claude_provider("p2", &skipped_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router.clone()).expect("create forwarder");

        db.save_provider("claude", &primary_provider)
            .expect("save primary provider for health tracking");
        db.save_provider("claude", &skipped_provider)
            .expect("save skipped provider for health tracking");

        let mut config = db
            .get_proxy_config_for_app("claude")
            .await
            .expect("load proxy config");
        config.circuit_timeout_seconds = 3600;
        db.update_proxy_config_for_app(config)
            .await
            .expect("update proxy timeout");

        router
            .record_result(
                "p2",
                "claude",
                false,
                false,
                Some("open breaker".to_string()),
            )
            .await
            .expect("open skipped provider breaker");
        assert!(!router.allow_provider_request("p2", "claude").await.allowed);

        let result = forwarder
            .forward_buffered_response(
                &AppType::Claude,
                "/v1/messages",
                claude_request_body(),
                &HeaderMap::new(),
                vec![primary_provider, skipped_provider],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_secs(2)),
                    bypass_circuit_breaker: false,
                },
                RectifierConfig::default(),
            )
            .await
            .expect("skipped candidates should preserve last attempted upstream response");

        assert_eq!(result.provider.id, "p1");
        assert_eq!(result.response.status, StatusCode::TOO_MANY_REQUESTS);
        let parsed: Value = serde_json::from_slice(&result.response.body).expect("parse body");
        assert_eq!(parsed, json!({"error": {"message": "rate limited"}}));
        assert_eq!(primary_hits.count.load(Ordering::SeqCst), 1);
        assert_eq!(skipped_hits.count.load(Ordering::SeqCst), 0);

        primary_server.abort();
        skipped_server.abort();
    }

    #[tokio::test]
    async fn later_half_open_provider_permit_is_not_preclaimed_when_earlier_success_stops() {
        let (primary_url, primary_hits, primary_server) =
            spawn_mock_upstream(StatusCode::OK, json!({"ok": true})).await;
        let (half_open_url, half_open_hits, half_open_server) =
            spawn_mock_upstream(StatusCode::OK, json!({"ok": true})).await;
        let primary_provider = claude_provider("p1", &primary_url, None);
        let half_open_provider = claude_provider("p2", &half_open_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router.clone()).expect("create forwarder");

        db.save_provider("claude", &primary_provider)
            .expect("save primary provider for health tracking");
        db.save_provider("claude", &half_open_provider)
            .expect("save half-open provider for health tracking");

        router
            .record_result(
                "p2",
                "claude",
                false,
                false,
                Some("open breaker".to_string()),
            )
            .await
            .expect("move provider into half-open state");

        let result = forwarder
            .forward_buffered_response(
                &AppType::Claude,
                "/v1/messages",
                claude_request_body(),
                &HeaderMap::new(),
                vec![primary_provider, half_open_provider],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_secs(2)),
                    bypass_circuit_breaker: false,
                },
                RectifierConfig::default(),
            )
            .await
            .expect("earlier success should stop before later half-open provider");

        assert_eq!(result.provider.id, "p1");
        assert_eq!(primary_hits.count.load(Ordering::SeqCst), 1);
        assert_eq!(half_open_hits.count.load(Ordering::SeqCst), 0);

        let permit = router.allow_provider_request("p2", "claude").await;
        assert!(permit.allowed);
        assert!(permit.used_half_open_permit);

        primary_server.abort();
        half_open_server.abort();
    }

    #[tokio::test]
    async fn claude_buffered_rectifier_retries_same_provider_on_invalid_signature() {
        let (base_url, hits, bodies, server) = spawn_scripted_upstream(vec![
            (
                StatusCode::BAD_REQUEST,
                json!({"error": {"message": "messages.1.content.0: Invalid `signature` in `thinking` block"}}),
            ),
            (StatusCode::OK, json!({"id": "msg_123", "content": []})),
        ])
        .await;
        let provider = claude_provider("p1", &base_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router).expect("create forwarder");

        db.save_provider("claude", &provider)
            .expect("save provider for health tracking");

        let body = json!({
            "model": "claude-3-7-sonnet-20250219",
            "max_tokens": 32,
            "messages": [{
                "role": "assistant",
                "content": [
                    { "type": "thinking", "thinking": "t", "signature": "sig" },
                    { "type": "text", "text": "hello", "signature": "text-sig" }
                ]
            }]
        });

        let result = forwarder
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
            .expect("signature rectifier should retry same provider once");

        assert_eq!(result.provider.id, "p1");
        assert_eq!(result.response.status, StatusCode::OK);
        assert_eq!(hits.count.load(Ordering::SeqCst), 2);

        let sent_bodies = bodies.lock().await;
        assert_eq!(sent_bodies.len(), 2);
        assert_eq!(
            sent_bodies[0]["messages"][0]["content"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let retried_content = sent_bodies[1]["messages"][0]["content"].as_array().unwrap();
        assert_eq!(retried_content.len(), 1);
        assert_eq!(retried_content[0]["type"], "text");
        assert!(retried_content[0].get("signature").is_none());

        server.abort();
    }

    #[tokio::test]
    async fn claude_openai_chat_budget_rectifier_retries_same_provider_with_transformed_body() {
        let (base_url, hits, bodies, server) = spawn_scripted_upstream(vec![
            (
                StatusCode::BAD_REQUEST,
                json!({"error": {"message": "thinking.budget_tokens: Input should be greater than or equal to 1024"}}),
            ),
            (
                StatusCode::OK,
                json!({"id": "resp_123", "choices": [{"message": {"role": "assistant", "content": "ok"}}]}),
            ),
        ])
        .await;
        let provider = claude_provider("p1", &base_url, Some("openai_chat"));
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router).expect("create forwarder");

        db.save_provider("claude", &provider)
            .expect("save provider for health tracking");

        let body = json!({
            "model": "claude-3-7-sonnet-20250219",
            "max_tokens": 1024,
            "thinking": { "type": "enabled", "budget_tokens": 512 },
            "messages": [{
                "role": "user",
                "content": [{ "type": "text", "text": "hello" }]
            }]
        });

        let result = forwarder
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
            .expect("budget rectifier should retry same provider once");

        assert_eq!(result.provider.id, "p1");
        assert_eq!(result.response.status, StatusCode::OK);
        assert_eq!(hits.count.load(Ordering::SeqCst), 2);
        assert_eq!(
            hits.paths.lock().await.as_slice(),
            ["/v1/chat/completions", "/v1/chat/completions"]
        );

        let sent_bodies = bodies.lock().await;
        assert_eq!(sent_bodies.len(), 2);
        assert_eq!(sent_bodies[0]["max_tokens"], 1024);
        assert_eq!(sent_bodies[1]["max_tokens"], 64000);
        assert!(sent_bodies[1].get("messages").is_some());

        server.abort();
    }

    #[tokio::test]
    async fn claude_streaming_rectifier_retries_same_provider_on_invalid_signature_error() {
        let (base_url, hits, bodies, server) = spawn_scripted_streaming_upstream(vec![
            (
                StatusCode::BAD_REQUEST,
                ScriptedStreamingBody::Json(
                    json!({"error": {"message": "messages.1.content.0: Invalid `signature` in `thinking` block"}}),
                ),
            ),
            (
                StatusCode::OK,
                ScriptedStreamingBody::Sse(
                    "data: {\"id\":\"msg_123\",\"type\":\"message_start\"}\n\ndata: [DONE]\n\n",
                ),
            ),
        ])
        .await;
        let provider = claude_provider("p1", &base_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router).expect("create forwarder");

        db.save_provider("claude", &provider)
            .expect("save provider for health tracking");

        let body = json!({
            "model": "claude-3-7-sonnet-20250219",
            "stream": true,
            "max_tokens": 32,
            "messages": [{
                "role": "assistant",
                "content": [
                    { "type": "thinking", "thinking": "t", "signature": "sig" },
                    { "type": "text", "text": "hello", "signature": "text-sig" }
                ]
            }]
        });

        let result = forwarder
            .forward_response(
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
            .expect("streaming signature rectifier should retry same provider once");

        assert_eq!(result.provider.id, "p1");
        assert_eq!(result.response.status(), StatusCode::OK);
        assert_eq!(hits.count.load(Ordering::SeqCst), 2);

        let sent_bodies = bodies.lock().await;
        assert_eq!(sent_bodies.len(), 2);
        let retried_content = sent_bodies[1]["messages"][0]["content"].as_array().unwrap();
        assert_eq!(retried_content.len(), 1);
        assert_eq!(retried_content[0]["type"], "text");
        assert!(retried_content[0].get("signature").is_none());

        server.abort();
    }

    #[tokio::test]
    async fn claude_streaming_rectifier_owned_400_stops_before_next_provider() {
        let (primary_url, primary_hits, primary_bodies, primary_server) =
            spawn_delayed_scripted_streaming_upstream(vec![
                (
                    Duration::from_millis(0),
                    StatusCode::BAD_REQUEST,
                    ScriptedStreamingBody::Json(
                        json!({"error": {"message": "messages.1.content.0: Invalid `signature` in `thinking` block"}}),
                    ),
                ),
                (
                    Duration::from_millis(0),
                    StatusCode::BAD_REQUEST,
                    ScriptedStreamingBody::Json(
                        json!({"error": {"message": "messages.1.content.0: Invalid `signature` in `thinking` block"}}),
                    ),
                ),
            ])
            .await;
        let (secondary_url, secondary_hits, secondary_bodies, secondary_server) =
            spawn_delayed_scripted_streaming_upstream(vec![(
                Duration::from_millis(0),
                StatusCode::OK,
                ScriptedStreamingBody::Sse(
                    "data: {\"id\":\"msg_123\",\"type\":\"message_start\"}\n\ndata: [DONE]\n\n",
                ),
            )])
            .await;
        let provider_one = claude_provider("p1", &primary_url, None);
        let provider_two = claude_provider("p2", &secondary_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router.clone()).expect("create forwarder");

        db.save_provider("claude", &provider_one)
            .expect("save primary provider for health tracking");
        db.save_provider("claude", &provider_two)
            .expect("save secondary provider for health tracking");
        router
            .record_result(
                "p1",
                "claude",
                false,
                false,
                Some("open breaker".to_string()),
            )
            .await
            .expect("open primary breaker");

        let body = json!({
            "model": "claude-3-7-sonnet-20250219",
            "stream": true,
            "max_tokens": 32,
            "messages": [{
                "role": "assistant",
                "content": [
                    { "type": "thinking", "thinking": "t", "signature": "sig" },
                    { "type": "text", "text": "hello", "signature": "text-sig" }
                ]
            }]
        });

        let error = forwarder
            .forward_response(
                &AppType::Claude,
                "/v1/messages",
                body,
                &HeaderMap::new(),
                vec![provider_one, provider_two],
                ForwardOptions {
                    max_retries: 0,
                    request_timeout: Some(Duration::from_secs(2)),
                    bypass_circuit_breaker: false,
                },
                RectifierConfig::default(),
            )
            .await
            .expect_err("rectifier-owned streaming 400 should surface as UpstreamError");

        match error {
            ProxyError::UpstreamError { status, body } => {
                assert_eq!(status, 400);
                assert!(body
                    .as_deref()
                    .expect("rectifier-owned streaming 400 should preserve body")
                    .contains("Invalid `signature`"));
            }
            other => panic!("expected UpstreamError, got {other:?}"),
        }
        assert_eq!(primary_hits.count.load(Ordering::SeqCst), 2);
        assert_eq!(secondary_hits.count.load(Ordering::SeqCst), 0);
        assert_eq!(primary_bodies.lock().await.len(), 2);
        assert_eq!(secondary_bodies.lock().await.len(), 0);

        let permit = router.allow_provider_request("p1", "claude").await;
        assert!(permit.allowed);
        assert!(permit.used_half_open_permit);

        primary_server.abort();
        secondary_server.abort();
    }

    #[tokio::test]
    async fn claude_streaming_openai_chat_budget_rectifier_retries_same_provider() {
        let (base_url, hits, bodies, server) = spawn_scripted_streaming_upstream(vec![
            (
                StatusCode::BAD_REQUEST,
                ScriptedStreamingBody::Json(
                    json!({"error": {"message": "thinking.budget_tokens: Input should be greater than or equal to 1024"}}),
                ),
            ),
            (
                StatusCode::OK,
                ScriptedStreamingBody::Sse(
                    "data: {\"id\":\"chatcmpl_123\",\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
                ),
            ),
        ])
        .await;
        let provider = claude_provider("p1", &base_url, Some("openai_chat"));
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router).expect("create forwarder");

        db.save_provider("claude", &provider)
            .expect("save provider for health tracking");

        let body = json!({
            "model": "claude-3-7-sonnet-20250219",
            "stream": true,
            "max_tokens": 1024,
            "thinking": { "type": "enabled", "budget_tokens": 512 },
            "messages": [{
                "role": "user",
                "content": [{ "type": "text", "text": "hello" }]
            }]
        });

        let result = forwarder
            .forward_response(
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
            .expect("streaming budget rectifier should retry same provider once");

        assert_eq!(result.provider.id, "p1");
        assert_eq!(result.response.status(), StatusCode::OK);
        assert_eq!(hits.count.load(Ordering::SeqCst), 2);
        assert_eq!(
            hits.paths.lock().await.as_slice(),
            ["/v1/chat/completions", "/v1/chat/completions"]
        );

        let sent_bodies = bodies.lock().await;
        assert_eq!(sent_bodies.len(), 2);
        assert_eq!(sent_bodies[0]["max_tokens"], 1024);
        assert_eq!(sent_bodies[1]["max_tokens"], 64000);

        server.abort();
    }

    #[tokio::test]
    async fn claude_streaming_success_path_does_not_trigger_rectifier_retry() {
        let (base_url, hits, bodies, server) = spawn_scripted_streaming_upstream(vec![(
            StatusCode::OK,
            ScriptedStreamingBody::Sse(
                "data: {\"id\":\"msg_123\",\"type\":\"message_start\"}\n\ndata: [DONE]\n\n",
            ),
        )])
        .await;
        let provider = claude_provider("p1", &base_url, None);
        let (db, router) = test_router().await;
        let forwarder = RequestForwarder::new(router).expect("create forwarder");

        db.save_provider("claude", &provider)
            .expect("save provider for health tracking");

        let body = json!({
            "model": "claude-3-7-sonnet-20250219",
            "stream": true,
            "max_tokens": 32,
            "messages": [{
                "role": "assistant",
                "content": [
                    { "type": "thinking", "thinking": "t", "signature": "sig" },
                    { "type": "text", "text": "hello", "signature": "text-sig" }
                ]
            }]
        });

        let result = forwarder
            .forward_response(
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
            .expect("streaming success path should not use rectifier retry");

        assert_eq!(result.response.status(), StatusCode::OK);
        assert!(matches!(
            &result.response,
            StreamingResponse::Live(response) if is_sse_response(response)
        ));
        assert_eq!(hits.count.load(Ordering::SeqCst), 1);

        let sent_bodies = bodies.lock().await;
        assert_eq!(sent_bodies.len(), 1);
        assert_eq!(
            sent_bodies[0]["messages"][0]["content"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        server.abort();
    }
}
