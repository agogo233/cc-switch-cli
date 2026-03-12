use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Instant};

use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};
use tokio::{
    sync::{oneshot, RwLock},
    task::JoinHandle,
};
use tower_http::cors::{Any, CorsLayer};

use crate::{app_config::AppType, database::Database, provider::Provider};

use super::{
    error::ProxyError,
    handlers,
    types::{ActiveTarget, ProxyConfig, ProxyServerInfo, ProxyStatus},
};

const PROXY_RUNTIME_SESSION_TOKEN_ENV_KEY: &str = "CC_SWITCH_PROXY_SESSION_TOKEN";

#[derive(Clone)]
pub struct ProxyServerState {
    pub db: Arc<Database>,
    pub config: Arc<RwLock<ProxyConfig>>,
    pub status: Arc<RwLock<ProxyStatus>>,
    pub start_time: Arc<RwLock<Option<Instant>>>,
    pub current_providers: Arc<RwLock<HashMap<String, (String, String)>>>,
}

impl ProxyServerState {
    pub async fn snapshot_status(&self) -> ProxyStatus {
        let mut status = self.status.read().await.clone();

        if let Some(start_time) = *self.start_time.read().await {
            status.uptime_seconds = start_time.elapsed().as_secs();
        }

        let mut active_targets = self
            .current_providers
            .read()
            .await
            .iter()
            .map(|(app_type, (provider_id, provider_name))| ActiveTarget {
                app_type: app_type.clone(),
                provider_id: provider_id.clone(),
                provider_name: provider_name.clone(),
            })
            .collect::<Vec<_>>();
        active_targets.sort_by(|left, right| left.app_type.cmp(&right.app_type));
        status.active_targets = active_targets;

        status
    }

    pub async fn record_request_start(&self) {
        let mut status = self.status.write().await;
        status.total_requests += 1;
        status.active_connections += 1;
        status.last_request_at = Some(chrono::Utc::now().to_rfc3339());
    }

    pub async fn record_active_target(&self, app_type: &AppType, provider: &Provider) {
        self.current_providers.write().await.insert(
            app_type.as_str().to_string(),
            (provider.id.clone(), provider.name.clone()),
        );

        let mut status = self.status.write().await;
        status.current_provider = Some(provider.name.clone());
        status.current_provider_id = Some(provider.id.clone());
    }

    pub async fn record_request_success(&self) {
        let mut status = self.status.write().await;
        status.active_connections = status.active_connections.saturating_sub(1);
        status.success_requests += 1;
        update_success_rate(&mut status);
        status.last_error = None;
    }

    pub async fn record_request_error(&self, error: &ProxyError) {
        self.record_request_error_message(error.to_string()).await;
    }

    pub async fn record_request_error_message(&self, message: String) {
        let mut status = self.status.write().await;
        status.active_connections = status.active_connections.saturating_sub(1);
        status.failed_requests += 1;
        update_success_rate(&mut status);
        status.last_error = Some(message);
    }

    pub async fn record_upstream_failure(&self, status_code: reqwest::StatusCode) {
        let mut status = self.status.write().await;
        status.active_connections = status.active_connections.saturating_sub(1);
        status.failed_requests += 1;
        update_success_rate(&mut status);
        status.last_error = Some(format!("upstream returned {}", status_code.as_u16()));
    }
}

fn update_success_rate(status: &mut ProxyStatus) {
    status.success_rate = if status.total_requests == 0 {
        0.0
    } else {
        (status.success_requests as f32 / status.total_requests as f32) * 100.0
    };
}

pub struct ProxyServer {
    state: ProxyServerState,
    shutdown_tx: Arc<RwLock<Option<oneshot::Sender<()>>>>,
    server_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
}

impl ProxyServer {
    pub fn new(config: ProxyConfig, db: Arc<Database>) -> Self {
        let managed_session_token = std::env::var(PROXY_RUNTIME_SESSION_TOKEN_ENV_KEY)
            .ok()
            .filter(|value| !value.trim().is_empty());
        let status = ProxyStatus {
            managed_session_token,
            ..ProxyStatus::default()
        };

        Self {
            state: ProxyServerState {
                db,
                config: Arc::new(RwLock::new(config)),
                status: Arc::new(RwLock::new(status)),
                start_time: Arc::new(RwLock::new(None)),
                current_providers: Arc::new(RwLock::new(HashMap::new())),
            },
            shutdown_tx: Arc::new(RwLock::new(None)),
            server_handle: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn start(&self) -> Result<ProxyServerInfo, String> {
        if self.shutdown_tx.read().await.is_some() {
            let status = self.get_status().await;
            return Ok(ProxyServerInfo {
                address: status.address,
                port: status.port,
                started_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        let bind_config = self.state.config.read().await.clone();
        let addr: SocketAddr =
            format!("{}:{}", bind_config.listen_address, bind_config.listen_port)
                .parse()
                .map_err(|e| format!("invalid bind address: {e}"))?;

        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| format!("bind proxy listener failed: {e}"))?;
        let local_addr = listener
            .local_addr()
            .map_err(|e| format!("read proxy listener address failed: {e}"))?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        *self.shutdown_tx.write().await = Some(shutdown_tx);

        {
            let mut status = self.state.status.write().await;
            status.running = true;
            status.address = bind_config.listen_address.clone();
            status.port = local_addr.port();
        }
        *self.state.start_time.write().await = Some(Instant::now());

        let app = self.build_router();
        let state = self.state.clone();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;

            state.status.write().await.running = false;
            *state.start_time.write().await = None;
        });
        *self.server_handle.write().await = Some(handle);

        Ok(ProxyServerInfo {
            address: bind_config.listen_address,
            port: local_addr.port(),
            started_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    pub async fn stop(&self) -> Result<(), String> {
        if let Some(tx) = self.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        } else {
            return Ok(());
        }

        if let Some(handle) = self.server_handle.write().await.take() {
            handle
                .await
                .map_err(|e| format!("join proxy task failed: {e}"))?;
        }
        Ok(())
    }

    pub async fn get_status(&self) -> ProxyStatus {
        self.state.snapshot_status().await
    }

    fn build_router(&self) -> Router {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);

        Router::new()
            .route("/health", get(handlers::health_check))
            .route("/status", get(handlers::get_status))
            .route("/v1/messages", post(handlers::handle_messages))
            .route("/claude/v1/messages", post(handlers::handle_messages))
            .route("/chat/completions", post(handlers::handle_chat_completions))
            .route(
                "/v1/chat/completions",
                post(handlers::handle_chat_completions),
            )
            .route(
                "/v1/v1/chat/completions",
                post(handlers::handle_chat_completions),
            )
            .route(
                "/codex/v1/chat/completions",
                post(handlers::handle_chat_completions),
            )
            .route("/responses", post(handlers::handle_responses))
            .route("/v1/responses", post(handlers::handle_responses))
            .route("/v1/v1/responses", post(handlers::handle_responses))
            .route("/codex/v1/responses", post(handlers::handle_responses))
            .route("/v1beta/*path", post(handlers::handle_gemini))
            .route("/gemini/v1beta/*path", post(handlers::handle_gemini))
            .layer(DefaultBodyLimit::max(200 * 1024 * 1024))
            .layer(cors)
            .with_state(self.state.clone())
    }
}
