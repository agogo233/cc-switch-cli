mod codex_toml;

use std::{
    collections::HashMap,
    future::Future,
    process::{Command, Stdio},
    sync::{Arc, Mutex as StdMutex, OnceLock, Weak},
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::{
    app_config::AppType,
    codex_config::{get_codex_auth_path, get_codex_config_path, write_codex_live_atomic},
    config::{get_claude_settings_path, read_json_file, write_json_file, write_text_file},
    database::Database,
    gemini_config::{
        env_to_json, get_gemini_env_path, json_to_env, read_gemini_env, write_gemini_env_atomic,
    },
    provider::Provider,
    proxy::{
        types::{GlobalProxyConfig, ProxyTakeoverStatus},
        ProxyConfig, ProxyServer, ProxyServerInfo, ProxyStatus,
    },
    AppError,
};

const PROXY_TOKEN_PLACEHOLDER: &str = "PROXY_MANAGED";
const PROXY_RUNTIME_SESSION_KEY: &str = "proxy_runtime_session";
const PROXY_RUNTIME_KIND_ENV_KEY: &str = "CC_SWITCH_PROXY_RUNTIME_KIND";
const PROXY_RUNTIME_SESSION_TOKEN_ENV_KEY: &str = "CC_SWITCH_PROXY_SESSION_TOKEN";

const CLAUDE_MODEL_OVERRIDE_ENV_KEYS: [&str; 6] = [
    "ANTHROPIC_MODEL",
    "ANTHROPIC_REASONING_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
];

#[derive(Clone)]
pub struct ProxyService {
    db: Arc<Database>,
    runtime: Arc<ProxyRuntimeState>,
}

struct ProxyRuntimeState {
    server: RwLock<Option<ProxyServer>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PersistedProxyRuntimeSessionKind {
    #[serde(alias = "foreground")]
    Foreground,
    ManagedExternal,
}

impl Default for PersistedProxyRuntimeSessionKind {
    fn default() -> Self {
        Self::Foreground
    }
}

impl PersistedProxyRuntimeSessionKind {
    fn from_env() -> Self {
        match std::env::var(PROXY_RUNTIME_KIND_ENV_KEY).ok().as_deref() {
            Some("managed_external") => Self::ManagedExternal,
            _ => Self::Foreground,
        }
    }

    fn as_env_value(&self) -> &'static str {
        match self {
            Self::Foreground => "foreground",
            Self::ManagedExternal => "managed_external",
        }
    }

    fn is_managed_external(&self) -> bool {
        matches!(self, Self::ManagedExternal)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedProxyRuntimeSession {
    pid: u32,
    address: String,
    port: u16,
    started_at: String,
    #[serde(default)]
    kind: PersistedProxyRuntimeSessionKind,
    #[serde(default)]
    session_token: Option<String>,
}

fn proxy_runtime_registry() -> &'static StdMutex<HashMap<String, Weak<ProxyRuntimeState>>> {
    static REGISTRY: OnceLock<StdMutex<HashMap<String, Weak<ProxyRuntimeState>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

impl ProxyService {
    fn run_in_blocking_runtime<T, F, Fut>(&self, task: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(ProxyService) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, String>> + Send + 'static,
    {
        let service = self.clone();
        let handle = std::thread::Builder::new()
            .name("cc-switch-proxy-blocking".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("failed to create async runtime: {e}"))?;
                runtime.block_on(task(service))
            })
            .map_err(|e| format!("spawn proxy runtime helper failed: {e}"))?;

        handle
            .join()
            .map_err(|_| "proxy runtime helper panicked".to_string())?
    }

    pub fn is_running_blocking(&self) -> Result<bool, String> {
        self.run_in_blocking_runtime(|service| async move { Ok(service.is_running().await) })
    }

    pub fn is_app_takeover_active_blocking(&self, app_type: &AppType) -> Result<bool, String> {
        let app_type = app_type.clone();
        self.run_in_blocking_runtime(move |service| async move {
            service.is_app_takeover_active(&app_type).await
        })
    }

    pub fn recover_takeovers_on_startup_blocking(&self) -> Result<(), String> {
        self.run_in_blocking_runtime(|service| async move {
            service.recover_takeovers_on_startup().await
        })
    }

    pub fn new(db: Arc<Database>) -> Self {
        let runtime = Self::shared_runtime_state(db.runtime_key());
        Self { db, runtime }
    }

    fn shared_runtime_state(runtime_key: &str) -> Arc<ProxyRuntimeState> {
        let mut registry = proxy_runtime_registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(existing) = registry.get(runtime_key).and_then(Weak::upgrade) {
            return existing;
        }

        let runtime = Arc::new(ProxyRuntimeState {
            server: RwLock::new(None),
        });
        registry.insert(runtime_key.to_string(), Arc::downgrade(&runtime));
        runtime
    }

    pub async fn start(&self) -> Result<ProxyServerInfo, String> {
        let config = self.get_config().await.map_err(|e| e.to_string())?;
        self.start_with_resolved_config(config).await
    }

    pub async fn start_with_runtime_config(
        &self,
        config: ProxyConfig,
    ) -> Result<ProxyServerInfo, String> {
        self.start_with_resolved_config(config).await
    }

    pub async fn start_managed_session(&self, app_type: &str) -> Result<ProxyServerInfo, String> {
        Self::ensure_managed_sessions_supported()?;

        let app_type = Self::takeover_app_from_str(app_type)?;
        let current_status = self.get_status().await;
        if current_status.running {
            return Err(
                "proxy is already running; stop the current runtime before starting a managed session"
                    .to_string(),
            );
        }

        let executable = Self::resolve_managed_proxy_executable()?;
        let session_token = uuid::Uuid::new_v4().to_string();
        let mut child = Command::new(executable);
        child
            .arg("proxy")
            .arg("serve")
            .arg("--takeover")
            .arg(app_type.as_str())
            .env(
                PROXY_RUNTIME_KIND_ENV_KEY,
                PersistedProxyRuntimeSessionKind::ManagedExternal.as_env_value(),
            )
            .env(PROXY_RUNTIME_SESSION_TOKEN_ENV_KEY, &session_token)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        #[cfg(unix)]
        unsafe {
            child.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = child
            .spawn()
            .map_err(|error| format!("spawn managed proxy session failed: {error}"))?;
        let child_pid = child.id();

        let start_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("poll managed proxy process failed: {error}"))?
            {
                return Err(format!(
                    "managed proxy session exited before becoming ready: {}",
                    status
                ));
            }

            if let Some(session) = self
                .load_persisted_runtime_session()
                .filter(|session| session.pid == child_pid)
                .filter(|session| session.kind.is_managed_external())
                .filter(|session| session.session_token.as_deref() == Some(session_token.as_str()))
            {
                if Self::load_external_proxy_status(&session).await.is_some()
                    && self.is_app_takeover_active(&app_type).await?
                {
                    let info = ProxyServerInfo {
                        address: session.address,
                        port: session.port,
                        started_at: session.started_at,
                    };
                    Self::spawn_managed_child_reaper(child);
                    return Ok(info);
                }
            }

            if tokio::time::Instant::now() >= start_deadline {
                let _ = child.kill();
                let _ = child.wait();
                let _ = self.clear_persisted_runtime_session();
                return Err("managed proxy session did not become ready in time".to_string());
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn set_managed_session_for_app(
        &self,
        app_type: &str,
        enabled: bool,
    ) -> Result<(), String> {
        let app_type = Self::takeover_app_from_str(app_type)?;

        if enabled {
            let status = self.get_status().await;
            if !status.running {
                self.start_managed_session(app_type.as_str()).await?;
                return Ok(());
            }

            if self
                .load_persisted_runtime_session()
                .is_some_and(|session| session.kind.is_managed_external())
            {
                self.enable_takeover_for_app(&app_type).await?;
                return Ok(());
            }

            return Err(
                "proxy is already running in foreground mode; stop the current runtime before attaching another app to a managed session"
                    .to_string(),
            );
        }

        let stop_server_when_last = self
            .load_persisted_runtime_session()
            .map(|session| session.kind.is_managed_external())
            .unwrap_or(false);
        self.disable_takeover_for_app(&app_type, stop_server_when_last)
            .await
    }

    async fn start_with_resolved_config(
        &self,
        config: ProxyConfig,
    ) -> Result<ProxyServerInfo, String> {
        if let Some(server) = self.runtime.server.read().await.as_ref() {
            let status = server.get_status().await;
            if status.running {
                return Ok(ProxyServerInfo {
                    address: status.address,
                    port: status.port,
                    started_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }

        let server = ProxyServer::new(config, self.db.clone());
        let info = server.start().await?;
        if let Err(error) = self.persist_runtime_session(&info) {
            let _ = server.stop().await;
            return Err(error);
        }
        *self.runtime.server.write().await = Some(server);
        Ok(info)
    }

    pub async fn recover_takeovers_on_startup(&self) -> Result<(), String> {
        for app_type in [AppType::Claude, AppType::Codex, AppType::Gemini] {
            let app_key = app_type.as_str();
            let app_proxy = self
                .db
                .get_proxy_config_for_app(app_key)
                .await
                .map_err(|error| format!("load proxy config for {app_key} failed: {error}"))?;
            let has_backup = self
                .db
                .get_live_backup(app_key)
                .await
                .map_err(|error| format!("load live backup for {app_key} failed: {error}"))?
                .is_some();
            let live_taken_over = self.detect_takeover_in_live_config_for_app(&app_type);

            if !app_proxy.enabled && !has_backup && !live_taken_over {
                continue;
            }

            if has_backup {
                self.restore_live_config_for_app(&app_type).await?;
                self.db
                    .delete_live_backup(app_key)
                    .await
                    .map_err(|error| format!("delete live backup for {app_key} failed: {error}"))?;
            } else if live_taken_over {
                self.restore_live_from_current_provider(&app_type).await?;
            }

            if app_proxy.enabled {
                let mut cleared = app_proxy;
                cleared.enabled = false;
                self.db
                    .update_proxy_config_for_app(cleared)
                    .await
                    .map_err(|error| {
                        format!("clear takeover flag for {app_key} failed: {error}")
                    })?;
            }
        }

        Ok(())
    }

    pub async fn stop(&self) -> Result<(), String> {
        self.restore_active_takeovers_on_shutdown().await?;
        self.stop_server().await
    }

    async fn stop_server(&self) -> Result<(), String> {
        if let Some(server) = self.runtime.server.read().await.as_ref() {
            server.stop().await?;
            self.clear_persisted_runtime_session()?;
            return Ok(());
        }

        if let Some(session) = self.load_persisted_runtime_session() {
            if session.kind.is_managed_external() {
                if Self::is_process_alive(session.pid)
                    && Self::load_external_proxy_status(&session).await.is_some()
                {
                    Self::terminate_external_process(session.pid).await?;
                }
            }
        }

        self.clear_persisted_runtime_session()?;
        Ok(())
    }

    pub async fn is_running(&self) -> bool {
        self.get_status().await.running
    }

    pub async fn get_status(&self) -> ProxyStatus {
        if let Some(server) = self.runtime.server.read().await.as_ref() {
            return server.get_status().await;
        }

        if let Some(session) = self.load_persisted_runtime_session() {
            if session.kind.is_managed_external() {
                if Self::is_process_alive(session.pid) {
                    if let Some(status) = Self::load_external_proxy_status(&session).await {
                        return status;
                    }
                }

                let _ = self.clear_persisted_runtime_session();
                return ProxyStatus::default();
            }

            if Self::is_process_alive(session.pid) {
                let uptime_seconds = chrono::DateTime::parse_from_rfc3339(&session.started_at)
                    .ok()
                    .map(|started_at| {
                        let started_at = started_at.with_timezone(&chrono::Utc);
                        (chrono::Utc::now() - started_at).num_seconds().max(0) as u64
                    })
                    .unwrap_or(0);

                return ProxyStatus {
                    running: true,
                    address: session.address,
                    port: session.port,
                    uptime_seconds,
                    ..ProxyStatus::default()
                };
            }

            let _ = self.clear_persisted_runtime_session();
        }

        ProxyStatus::default()
    }

    pub async fn get_config(&self) -> Result<ProxyConfig, AppError> {
        self.db.get_proxy_config().await
    }

    pub async fn update_config(&self, config: &ProxyConfig) -> Result<(), AppError> {
        self.db.update_proxy_config(config.clone()).await
    }

    pub async fn update_circuit_breaker_configs(
        &self,
        config: crate::proxy::circuit_breaker::CircuitBreakerConfig,
    ) -> Result<(), String> {
        if let Some(server) = self.runtime.server.read().await.as_ref() {
            server.update_circuit_breaker_configs(config).await;
        }

        Ok(())
    }

    pub async fn reset_provider_circuit_breaker(
        &self,
        provider_id: &str,
        app_type: &str,
    ) -> Result<(), String> {
        if let Some(server) = self.runtime.server.read().await.as_ref() {
            server
                .reset_provider_circuit_breaker(provider_id, app_type)
                .await;
        }

        Ok(())
    }

    pub async fn get_global_config(&self) -> Result<GlobalProxyConfig, AppError> {
        self.db.get_global_proxy_config().await
    }

    pub async fn set_global_enabled(&self, enabled: bool) -> Result<GlobalProxyConfig, AppError> {
        let mut config = self.get_global_config().await?;
        config.proxy_enabled = enabled;
        self.db.update_global_proxy_config(config.clone()).await?;
        Ok(config)
    }

    pub async fn get_takeover_status(&self) -> Result<ProxyTakeoverStatus, String> {
        Ok(ProxyTakeoverStatus {
            claude: self
                .db
                .get_proxy_config_for_app("claude")
                .await
                .map_err(|error| format!("load claude proxy config failed: {error}"))?
                .enabled,
            codex: self
                .db
                .get_proxy_config_for_app("codex")
                .await
                .map_err(|error| format!("load codex proxy config failed: {error}"))?
                .enabled,
            gemini: self
                .db
                .get_proxy_config_for_app("gemini")
                .await
                .map_err(|error| format!("load gemini proxy config failed: {error}"))?
                .enabled,
        })
    }

    pub async fn set_takeover_for_app(&self, app_type: &str, enabled: bool) -> Result<(), String> {
        let app_type = Self::takeover_app_from_str(app_type)?;

        if enabled {
            self.enable_takeover_for_app(&app_type).await
        } else {
            self.disable_takeover_for_app(&app_type, true).await
        }
    }

    pub async fn is_app_takeover_active(&self, app_type: &AppType) -> Result<bool, String> {
        let app_key = app_type.as_str();
        let app_proxy = self
            .db
            .get_proxy_config_for_app(app_key)
            .await
            .map_err(|error| format!("load proxy config for {app_key} failed: {error}"))?;
        if app_proxy.enabled {
            return Ok(true);
        }

        if self
            .db
            .get_live_backup(app_key)
            .await
            .map_err(|error| format!("load live backup for {app_key} failed: {error}"))?
            .is_some()
        {
            return Ok(true);
        }

        Ok(self.detect_takeover_in_live_config_for_app(app_type))
    }

    pub fn detect_takeover_in_live_config_for_app(&self, app_type: &AppType) -> bool {
        match app_type {
            AppType::Claude => self
                .read_claude_live()
                .ok()
                .and_then(|live| {
                    let env = live.get("env")?.as_object()?;
                    let base_url = env.get("ANTHROPIC_BASE_URL")?.as_str()?;
                    let has_placeholder = [
                        "ANTHROPIC_AUTH_TOKEN",
                        "ANTHROPIC_API_KEY",
                        "OPENROUTER_API_KEY",
                        "OPENAI_API_KEY",
                    ]
                    .iter()
                    .any(|key| {
                        env.get(*key)
                            .and_then(Value::as_str)
                            .is_some_and(|value| value == PROXY_TOKEN_PLACEHOLDER)
                    });
                    Some(codex_toml::is_loopback_proxy_url(base_url) && has_placeholder)
                })
                .unwrap_or(false),
            AppType::Codex => self
                .read_codex_live()
                .ok()
                .map(|live| {
                    let has_placeholder = live
                        .get("auth")
                        .and_then(|auth| auth.get("OPENAI_API_KEY"))
                        .and_then(Value::as_str)
                        .is_some_and(|value| value == PROXY_TOKEN_PLACEHOLDER);
                    let points_to_proxy = live
                        .get("config")
                        .and_then(Value::as_str)
                        .is_some_and(codex_toml::contains_loopback_proxy_url);
                    has_placeholder && points_to_proxy
                })
                .unwrap_or(false),
            AppType::Gemini => self
                .read_gemini_live()
                .ok()
                .and_then(|live| {
                    let env = live.get("env")?.as_object()?;
                    let base_url = env.get("GOOGLE_GEMINI_BASE_URL")?.as_str()?;
                    let has_placeholder = env
                        .get("GEMINI_API_KEY")
                        .and_then(Value::as_str)
                        .is_some_and(|value| value == PROXY_TOKEN_PLACEHOLDER);
                    Some(codex_toml::is_loopback_proxy_url(base_url) && has_placeholder)
                })
                .unwrap_or(false),
            _ => false,
        }
    }

    pub async fn update_live_backup_from_provider(
        &self,
        app_type: &str,
        provider: &Provider,
    ) -> Result<(), String> {
        let app_type = Self::takeover_app_from_str(app_type)?;
        if !self.is_app_takeover_active(&app_type).await? {
            return Ok(());
        }

        let backup_snapshot = self.build_live_snapshot_from_provider(&app_type, provider)?;
        self.save_live_backup_snapshot(app_type.as_str(), &backup_snapshot)
            .await
    }

    pub async fn save_live_backup_snapshot(
        &self,
        app_type: &str,
        snapshot: &Value,
    ) -> Result<(), String> {
        let app_type = Self::takeover_app_from_str(app_type)?;
        let backup = serde_json::to_string(snapshot).map_err(|error| {
            format!(
                "serialize {} live backup failed: {error}",
                app_type.as_str()
            )
        })?;
        self.db
            .save_live_backup(app_type.as_str(), &backup)
            .await
            .map_err(|error| format!("save {} live backup failed: {error}", app_type.as_str()))
    }

    async fn restore_active_takeovers_on_shutdown(&self) -> Result<(), String> {
        for app_type in [AppType::Claude, AppType::Codex, AppType::Gemini] {
            let app_key = app_type.as_str();
            let app_proxy = self
                .db
                .get_proxy_config_for_app(app_key)
                .await
                .map_err(|error| format!("load proxy config for {app_key} failed: {error}"))?;
            if app_proxy.enabled {
                self.disable_takeover_for_app(&app_type, false).await?;
            }
        }

        Ok(())
    }

    async fn enable_takeover_for_app(&self, app_type: &AppType) -> Result<(), String> {
        if !self.is_running().await {
            self.start().await?;
        }

        let app_key = app_type.as_str();
        let app_proxy = self
            .db
            .get_proxy_config_for_app(app_key)
            .await
            .map_err(|error| format!("load proxy config for {app_key} failed: {error}"))?;
        let has_backup = self
            .db
            .get_live_backup(app_key)
            .await
            .map_err(|error| format!("load live backup for {app_key} failed: {error}"))?
            .is_some();
        let live_taken_over = self.detect_takeover_in_live_config_for_app(app_type);

        if app_proxy.enabled && has_backup && live_taken_over {
            return Ok(());
        }

        let live = self.read_or_current_provider_live(app_type).await?;
        if !has_backup {
            let backup = serde_json::to_string(&live)
                .map_err(|error| format!("serialize {app_key} live backup failed: {error}"))?;
            self.db
                .save_live_backup(app_key, &backup)
                .await
                .map_err(|error| format!("save {app_key} live backup failed: {error}"))?;
        }

        let (proxy_url, proxy_codex_base_url) = self.build_proxy_urls().await?;
        let mut taken_over = live;
        self.rewrite_live_for_proxy(app_type, &mut taken_over, &proxy_url, &proxy_codex_base_url)?;
        self.write_live_config_for_app(app_type, &taken_over)?;

        if !app_proxy.enabled {
            let mut updated = app_proxy;
            updated.enabled = true;
            self.db
                .update_proxy_config_for_app(updated)
                .await
                .map_err(|error| format!("set takeover flag for {app_key} failed: {error}"))?;
        }

        Ok(())
    }

    async fn disable_takeover_for_app(
        &self,
        app_type: &AppType,
        stop_server_when_last: bool,
    ) -> Result<(), String> {
        let app_key = app_type.as_str();
        let app_proxy = self
            .db
            .get_proxy_config_for_app(app_key)
            .await
            .map_err(|error| format!("load proxy config for {app_key} failed: {error}"))?;
        let has_backup = self
            .db
            .get_live_backup(app_key)
            .await
            .map_err(|error| format!("load live backup for {app_key} failed: {error}"))?
            .is_some();
        let live_taken_over = self.detect_takeover_in_live_config_for_app(app_type);

        if !app_proxy.enabled && !has_backup && !live_taken_over {
            return Ok(());
        }

        if has_backup {
            self.restore_live_config_for_app(app_type).await?;
            self.db
                .delete_live_backup(app_key)
                .await
                .map_err(|error| format!("delete live backup for {app_key} failed: {error}"))?;
        } else if live_taken_over {
            self.restore_live_from_current_provider(app_type).await?;
        }

        if app_proxy.enabled {
            let mut cleared = app_proxy;
            cleared.enabled = false;
            self.db
                .update_proxy_config_for_app(cleared)
                .await
                .map_err(|error| format!("clear takeover flag for {app_key} failed: {error}"))?;
        }

        self.db
            .clear_provider_health_for_app(app_key)
            .await
            .map_err(|error| format!("clear provider health for {app_key} failed: {error}"))?;

        if stop_server_when_last
            && !self
                .db
                .is_live_takeover_active()
                .await
                .map_err(|error| format!("check active takeovers failed: {error}"))?
        {
            self.stop_server().await?;
        }

        Ok(())
    }

    async fn restore_live_config_for_app(&self, app_type: &AppType) -> Result<(), String> {
        let app_key = app_type.as_str();
        let Some(backup) = self
            .db
            .get_live_backup(app_key)
            .await
            .map_err(|error| format!("load live backup for {app_key} failed: {error}"))?
        else {
            return Ok(());
        };

        let restored: Value = serde_json::from_str(&backup.original_config)
            .map_err(|error| format!("parse {app_key} live backup failed: {error}"))?;
        self.write_live_config_for_app(app_type, &restored)
    }

    async fn restore_live_from_current_provider(&self, app_type: &AppType) -> Result<(), String> {
        let Some(settings) = self.current_provider_settings(app_type).await? else {
            return self.clear_stale_takeover_from_live_config(app_type);
        };
        self.write_live_config_for_app(app_type, &settings)
    }

    async fn current_provider_settings(&self, app_type: &AppType) -> Result<Option<Value>, String> {
        let Some(current_provider) =
            self.db
                .get_current_provider(app_type.as_str())
                .map_err(|error| {
                    format!(
                        "load current provider for {} failed: {error}",
                        app_type.as_str()
                    )
                })?
        else {
            return Ok(None);
        };

        self.db
            .get_provider_by_id(&current_provider, app_type.as_str())
            .map_err(|error| {
                format!(
                    "load provider {} for {} failed: {error}",
                    current_provider,
                    app_type.as_str()
                )
            })
            .and_then(|provider| {
                provider
                    .map(|provider| self.build_live_snapshot_from_provider(app_type, &provider))
                    .transpose()
            })
    }

    async fn read_or_current_provider_live(&self, app_type: &AppType) -> Result<Value, String> {
        match self.read_live_config_for_app(app_type) {
            Ok(live) => Ok(live),
            Err(_) => self
                .current_provider_settings(app_type)
                .await?
                .ok_or_else(|| {
                    format!(
                        "missing live config and current provider for {}",
                        app_type.as_str()
                    )
                }),
        }
    }

    async fn build_proxy_urls(&self) -> Result<(String, String), String> {
        let runtime_status = self.get_status().await;
        let persisted = self.get_config().await.map_err(|e| e.to_string())?;
        let listen_address = if runtime_status.running && !runtime_status.address.is_empty() {
            runtime_status.address
        } else {
            persisted.listen_address.clone()
        };
        let listen_port = if runtime_status.running && runtime_status.port != 0 {
            runtime_status.port
        } else {
            persisted.listen_port
        };

        let connect_host = match listen_address.as_str() {
            "0.0.0.0" => "127.0.0.1".to_string(),
            "::" => "::1".to_string(),
            _ => listen_address,
        };
        let connect_host_for_url = if connect_host.contains(':') && !connect_host.starts_with('[') {
            format!("[{connect_host}]")
        } else {
            connect_host
        };

        let proxy_origin = format!("http://{}:{}", connect_host_for_url, listen_port);
        let proxy_codex_base_url = format!("{}/v1", proxy_origin.trim_end_matches('/'));
        Ok((proxy_origin, proxy_codex_base_url))
    }

    fn rewrite_live_for_proxy(
        &self,
        app_type: &AppType,
        live: &mut Value,
        proxy_url: &str,
        proxy_codex_base_url: &str,
    ) -> Result<(), String> {
        match app_type {
            AppType::Claude => {
                if !live.is_object() {
                    *live = json!({});
                }

                let root = live
                    .as_object_mut()
                    .ok_or_else(|| "claude live config root must be an object".to_string())?;
                if !root.get("env").is_some_and(Value::is_object) {
                    root.insert("env".to_string(), json!({}));
                }

                let env = root
                    .get_mut("env")
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| "claude env must be an object".to_string())?;
                env.insert("ANTHROPIC_BASE_URL".to_string(), json!(proxy_url));
                for key in CLAUDE_MODEL_OVERRIDE_ENV_KEYS {
                    env.remove(key);
                }

                let token_keys = [
                    "ANTHROPIC_AUTH_TOKEN",
                    "ANTHROPIC_API_KEY",
                    "OPENROUTER_API_KEY",
                    "OPENAI_API_KEY",
                ];
                let mut replaced_any = false;
                for key in token_keys {
                    if env.contains_key(key) {
                        env.insert(key.to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
                        replaced_any = true;
                    }
                }

                if !replaced_any {
                    env.insert(
                        "ANTHROPIC_AUTH_TOKEN".to_string(),
                        json!(PROXY_TOKEN_PLACEHOLDER),
                    );
                }
            }
            AppType::Codex => {
                if !live.is_object() {
                    *live = json!({});
                }

                let root = live
                    .as_object_mut()
                    .ok_or_else(|| "codex live config root must be an object".to_string())?;
                if !root.get("auth").is_some_and(Value::is_object) {
                    root.insert("auth".to_string(), json!({}));
                }

                let auth = root
                    .get_mut("auth")
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| "codex auth must be an object".to_string())?;
                auth.insert("OPENAI_API_KEY".to_string(), json!(PROXY_TOKEN_PLACEHOLDER));

                let config_text = root
                    .get("config")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                root.insert(
                    "config".to_string(),
                    json!(codex_toml::update_toml_base_url(
                        &config_text,
                        proxy_codex_base_url
                    )),
                );
            }
            AppType::Gemini => {
                if !live.is_object() {
                    *live = json!({});
                }

                let root = live
                    .as_object_mut()
                    .ok_or_else(|| "gemini live config root must be an object".to_string())?;
                if !root.get("env").is_some_and(Value::is_object) {
                    root.insert("env".to_string(), json!({}));
                }

                let env = root
                    .get_mut("env")
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| "gemini env must be an object".to_string())?;
                env.insert("GOOGLE_GEMINI_BASE_URL".to_string(), json!(proxy_url));
                env.insert("GEMINI_API_KEY".to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
            }
            _ => {
                return Err(format!(
                    "proxy takeover not supported for {}",
                    app_type.as_str()
                ));
            }
        }

        Ok(())
    }

    fn read_live_config_for_app(&self, app_type: &AppType) -> Result<Value, String> {
        match app_type {
            AppType::Claude => self.read_claude_live(),
            AppType::Codex => self.read_codex_live(),
            AppType::Gemini => self.read_gemini_live(),
            _ => Err(format!(
                "proxy takeover not supported for {}",
                app_type.as_str()
            )),
        }
    }

    fn write_live_config_for_app(&self, app_type: &AppType, config: &Value) -> Result<(), String> {
        match app_type {
            AppType::Claude => self.write_claude_live(config),
            AppType::Codex => self.write_codex_live(config),
            AppType::Gemini => self.write_gemini_live(config),
            _ => Err(format!(
                "proxy takeover not supported for {}",
                app_type.as_str()
            )),
        }
    }

    fn clear_stale_takeover_from_live_config(&self, app_type: &AppType) -> Result<(), String> {
        let mut live = match self.read_live_config_for_app(app_type) {
            Ok(live) => live,
            Err(_) => return Ok(()),
        };

        match app_type {
            AppType::Claude => {
                if let Some(env) = live.get_mut("env").and_then(Value::as_object_mut) {
                    if env
                        .get("ANTHROPIC_BASE_URL")
                        .and_then(Value::as_str)
                        .is_some_and(codex_toml::is_loopback_proxy_url)
                    {
                        env.remove("ANTHROPIC_BASE_URL");
                    }

                    for key in [
                        "ANTHROPIC_AUTH_TOKEN",
                        "ANTHROPIC_API_KEY",
                        "OPENROUTER_API_KEY",
                        "OPENAI_API_KEY",
                    ] {
                        if env
                            .get(key)
                            .and_then(Value::as_str)
                            .is_some_and(|value| value == PROXY_TOKEN_PLACEHOLDER)
                        {
                            env.remove(key);
                        }
                    }
                }
            }
            AppType::Codex => {
                if let Some(auth) = live.get_mut("auth").and_then(Value::as_object_mut) {
                    if auth
                        .get("OPENAI_API_KEY")
                        .and_then(Value::as_str)
                        .is_some_and(|value| value == PROXY_TOKEN_PLACEHOLDER)
                    {
                        auth.remove("OPENAI_API_KEY");
                    }
                }

                if let Some(config_text) = live.get("config").and_then(Value::as_str) {
                    live["config"] =
                        json!(codex_toml::remove_loopback_base_url_from_toml(config_text));
                }
            }
            AppType::Gemini => {
                if let Some(env) = live.get_mut("env").and_then(Value::as_object_mut) {
                    if env
                        .get("GOOGLE_GEMINI_BASE_URL")
                        .and_then(Value::as_str)
                        .is_some_and(codex_toml::is_loopback_proxy_url)
                    {
                        env.remove("GOOGLE_GEMINI_BASE_URL");
                    }
                    if env
                        .get("GEMINI_API_KEY")
                        .and_then(Value::as_str)
                        .is_some_and(|value| value == PROXY_TOKEN_PLACEHOLDER)
                    {
                        env.remove("GEMINI_API_KEY");
                    }
                }
            }
            _ => {
                return Err(format!(
                    "proxy takeover not supported for {}",
                    app_type.as_str()
                ));
            }
        }

        self.write_live_config_for_app(app_type, &live)
    }

    fn read_claude_live(&self) -> Result<Value, String> {
        let path = get_claude_settings_path();
        if !path.exists() {
            return Err("Claude settings.json does not exist".to_string());
        }

        let value: Value = read_json_file(&path)
            .map_err(|error| format!("read Claude settings.json failed: {error}"))?;
        if value.is_object() {
            Ok(value)
        } else {
            Err("Claude settings.json root must be an object".to_string())
        }
    }

    fn write_claude_live(&self, config: &Value) -> Result<(), String> {
        write_json_file(&get_claude_settings_path(), config)
            .map_err(|error| format!("write Claude settings.json failed: {error}"))
    }

    fn read_codex_live(&self) -> Result<Value, String> {
        let auth_path = get_codex_auth_path();
        let config_path = get_codex_config_path();
        if !auth_path.exists() && !config_path.exists() {
            return Err("Codex live config does not exist".to_string());
        }

        let auth = if auth_path.exists() {
            read_json_file(&auth_path)
                .map_err(|error| format!("read Codex auth.json failed: {error}"))?
        } else {
            json!({})
        };
        let config = if config_path.exists() {
            std::fs::read_to_string(&config_path)
                .map_err(|error| format!("read Codex config.toml failed: {error}"))?
        } else {
            String::new()
        };

        Ok(json!({
            "auth": auth,
            "config": config,
        }))
    }

    fn write_codex_live(&self, config: &Value) -> Result<(), String> {
        let auth = config
            .get("auth")
            .filter(|value| !value.as_object().is_some_and(|object| object.is_empty()));
        let config_text = config.get("config").and_then(Value::as_str);
        let auth_path = get_codex_auth_path();

        match (auth, config_text) {
            (Some(auth), Some(config_text)) => write_codex_live_atomic(auth, Some(config_text))
                .map_err(|error| format!("write Codex live config failed: {error}")),
            (Some(auth), None) => write_json_file(&get_codex_auth_path(), auth)
                .map_err(|error| format!("write Codex auth.json failed: {error}")),
            (None, Some(config_text)) => {
                if auth_path.exists() {
                    std::fs::remove_file(&auth_path)
                        .map_err(|error| format!("remove Codex auth.json failed: {error}"))?;
                }
                write_text_file(&get_codex_config_path(), config_text)
                    .map_err(|error| format!("write Codex config.toml failed: {error}"))
            }
            (None, None) => {
                if auth_path.exists() {
                    std::fs::remove_file(&auth_path)
                        .map_err(|error| format!("remove Codex auth.json failed: {error}"))?;
                }
                Ok(())
            }
        }
    }

    fn build_live_snapshot_from_provider(
        &self,
        app_type: &AppType,
        provider: &Provider,
    ) -> Result<Value, String> {
        let common_config_snippet =
            self.db
                .get_config_snippet(app_type.as_str())
                .map_err(|error| {
                    format!(
                        "load common config snippet for {} failed: {error}",
                        app_type.as_str()
                    )
                })?;
        let apply_common_config = provider
            .meta
            .as_ref()
            .and_then(|meta| meta.apply_common_config)
            .unwrap_or(true);

        crate::services::provider::ProviderService::build_live_backup_snapshot(
            app_type,
            provider,
            common_config_snippet.as_deref(),
            apply_common_config,
        )
        .map_err(|error| {
            format!(
                "build {} live snapshot from provider failed: {error}",
                app_type.as_str()
            )
        })
    }

    fn persist_runtime_session(&self, info: &ProxyServerInfo) -> Result<(), String> {
        let session = PersistedProxyRuntimeSession {
            pid: std::process::id(),
            address: info.address.clone(),
            port: info.port,
            started_at: info.started_at.clone(),
            kind: PersistedProxyRuntimeSessionKind::from_env(),
            session_token: std::env::var(PROXY_RUNTIME_SESSION_TOKEN_ENV_KEY)
                .ok()
                .filter(|value| !value.trim().is_empty()),
        };
        let serialized = serde_json::to_string(&session)
            .map_err(|error| format!("serialize proxy runtime session failed: {error}"))?;
        self.db
            .set_setting(PROXY_RUNTIME_SESSION_KEY, &serialized)
            .map_err(|error| format!("persist proxy runtime session failed: {error}"))
    }

    fn clear_persisted_runtime_session(&self) -> Result<(), String> {
        self.db
            .delete_setting(PROXY_RUNTIME_SESSION_KEY)
            .map_err(|error| format!("clear proxy runtime session failed: {error}"))
    }

    fn load_persisted_runtime_session(&self) -> Option<PersistedProxyRuntimeSession> {
        let raw = self
            .db
            .get_setting(PROXY_RUNTIME_SESSION_KEY)
            .ok()
            .flatten()?;
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }

        match serde_json::from_str(raw) {
            Ok(session) => Some(session),
            Err(_) => {
                let _ = self.clear_persisted_runtime_session();
                None
            }
        }
    }

    fn is_process_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }

        #[cfg(unix)]
        {
            let rc = unsafe { libc::kill(pid as i32, 0) };
            return rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
        }

        #[cfg(not(unix))]
        {
            pid == std::process::id()
        }
    }

    async fn load_external_proxy_status(
        session: &PersistedProxyRuntimeSession,
    ) -> Option<ProxyStatus> {
        let expected_session_token = session.session_token.as_deref()?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()
            .ok()?;
        let response = client
            .get(Self::build_session_status_url(session))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }

        let mut status = response.json::<ProxyStatus>().await.ok()?;
        if status.managed_session_token.as_deref() != Some(expected_session_token) {
            return None;
        }
        status.running = true;
        if status.address.trim().is_empty() {
            status.address = session.address.clone();
        }
        if status.port == 0 {
            status.port = session.port;
        }
        Some(status)
    }

    fn build_session_status_url(session: &PersistedProxyRuntimeSession) -> String {
        let connect_host = match session.address.as_str() {
            "0.0.0.0" => "127.0.0.1".to_string(),
            "::" => "::1".to_string(),
            value => value.to_string(),
        };
        let connect_host = if connect_host.contains(':') && !connect_host.starts_with('[') {
            format!("[{connect_host}]")
        } else {
            connect_host
        };

        format!("http://{}:{}/status", connect_host, session.port)
    }

    async fn terminate_external_process(pid: u32) -> Result<(), String> {
        if pid == 0 || pid == std::process::id() || !Self::is_process_alive(pid) {
            return Ok(());
        }

        #[cfg(unix)]
        {
            let rc = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            if rc != 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(format!("stop managed proxy session failed: {error}"));
                }
            }

            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while tokio::time::Instant::now() < deadline {
                if !Self::is_process_alive(pid) {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            let rc = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            if rc != 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(format!("force stop managed proxy session failed: {error}"));
                }
            }

            let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
            while tokio::time::Instant::now() < deadline {
                if !Self::is_process_alive(pid) {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }

            return Err(format!(
                "managed proxy session did not exit after termination signal: pid {}",
                pid
            ));
        }

        #[cfg(not(unix))]
        {
            let _ = pid;
            Err("managed proxy session stop is only supported on unix in this build".to_string())
        }
    }

    fn spawn_managed_child_reaper(mut child: std::process::Child) {
        tokio::task::spawn_blocking(move || {
            let _ = child.wait();
        });
    }

    fn ensure_managed_sessions_supported() -> Result<(), String> {
        #[cfg(unix)]
        {
            Ok(())
        }

        #[cfg(not(unix))]
        {
            Err("managed proxy sessions are unsupported on non-unix platforms".to_string())
        }
    }

    fn resolve_managed_proxy_executable() -> Result<std::path::PathBuf, String> {
        if let Some(path) = std::env::var_os("CARGO_BIN_EXE_cc-switch") {
            return Ok(path.into());
        }

        let current_exe = std::env::current_exe()
            .map_err(|error| format!("resolve managed proxy executable failed: {error}"))?;

        if current_exe
            .file_stem()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("cc-switch"))
        {
            return Ok(current_exe);
        }

        if let Some(debug_dir) = current_exe.parent().and_then(|parent| parent.parent()) {
            let candidate = debug_dir.join(format!("cc-switch{}", std::env::consts::EXE_SUFFIX));
            if candidate.exists() {
                return Ok(candidate);
            }
        }

        Ok(current_exe)
    }

    fn proxy_server_info_from_status(&self, status: ProxyStatus) -> ProxyServerInfo {
        ProxyServerInfo {
            address: status.address,
            port: status.port,
            started_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    fn read_gemini_live(&self) -> Result<Value, String> {
        let env_path = get_gemini_env_path();
        if !env_path.exists() {
            return Err("Gemini .env does not exist".to_string());
        }

        let env = read_gemini_env().map_err(|error| format!("read Gemini .env failed: {error}"))?;
        Ok(env_to_json(&env))
    }

    fn write_gemini_live(&self, config: &Value) -> Result<(), String> {
        let env =
            json_to_env(config).map_err(|error| format!("build Gemini .env failed: {error}"))?;
        write_gemini_env_atomic(&env).map_err(|error| format!("write Gemini .env failed: {error}"))
    }

    fn takeover_app_from_str(app_type: &str) -> Result<AppType, String> {
        match app_type {
            "claude" => Ok(AppType::Claude),
            "codex" => Ok(AppType::Codex),
            "gemini" => Ok(AppType::Gemini),
            _ => Err(format!("proxy takeover not supported for app: {app_type}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::circuit_breaker::CircuitBreakerConfig;
    use serial_test::serial;

    #[tokio::test]
    #[serial]
    async fn hot_updating_running_breaker_configs_refreshes_existing_breakers() {
        let db = Arc::new(Database::memory().expect("create database"));
        let service = ProxyService::new(db.clone());

        let provider = Provider::with_id(
            "p1".to_string(),
            "Provider One".to_string(),
            json!({}),
            None,
        );
        db.save_provider("claude", &provider)
            .expect("save provider");

        let mut app_proxy = db
            .get_proxy_config_for_app("claude")
            .await
            .expect("load app proxy config");
        app_proxy.circuit_failure_threshold = 1;
        app_proxy.circuit_timeout_seconds = 3600;
        db.update_proxy_config_for_app(app_proxy)
            .await
            .expect("persist initial breaker config");

        let mut runtime_config = service.get_config().await.expect("get proxy config");
        runtime_config.listen_port = 0;
        service
            .start_with_runtime_config(runtime_config)
            .await
            .expect("start proxy");

        let router = {
            let server_guard = service.runtime.server.read().await;
            server_guard
                .as_ref()
                .expect("running server")
                .provider_router()
        };

        router
            .record_result("p1", "claude", false, false, Some("fail".to_string()))
            .await
            .expect("open breaker");
        assert!(!router.allow_provider_request("p1", "claude").await.allowed);

        let updated = CircuitBreakerConfig {
            failure_threshold: 1,
            success_threshold: 2,
            timeout_seconds: 0,
            error_rate_threshold: 1.0,
            min_requests: u32::MAX,
        };
        db.update_circuit_breaker_config(&updated)
            .await
            .expect("persist updated breaker config");
        service
            .update_circuit_breaker_configs(updated)
            .await
            .expect("hot update running breaker config");

        let permit = router.allow_provider_request("p1", "claude").await;
        assert!(permit.allowed);
        assert!(permit.used_half_open_permit);

        service.stop().await.expect("stop proxy");
    }

    #[tokio::test]
    #[serial]
    async fn resetting_running_provider_breaker_clears_existing_breaker_state() {
        let db = Arc::new(Database::memory().expect("create database"));
        let service = ProxyService::new(db.clone());

        let provider = Provider::with_id(
            "p1".to_string(),
            "Provider One".to_string(),
            json!({}),
            None,
        );
        db.save_provider("claude", &provider)
            .expect("save provider");

        let mut app_proxy = db
            .get_proxy_config_for_app("claude")
            .await
            .expect("load app proxy config");
        app_proxy.circuit_failure_threshold = 1;
        app_proxy.circuit_timeout_seconds = 3600;
        db.update_proxy_config_for_app(app_proxy)
            .await
            .expect("persist breaker config");

        let mut runtime_config = service.get_config().await.expect("get proxy config");
        runtime_config.listen_port = 0;
        service
            .start_with_runtime_config(runtime_config)
            .await
            .expect("start proxy");

        let router = {
            let server_guard = service.runtime.server.read().await;
            server_guard
                .as_ref()
                .expect("running server")
                .provider_router()
        };

        router
            .record_result("p1", "claude", false, false, Some("fail".to_string()))
            .await
            .expect("open breaker");
        assert!(!router.allow_provider_request("p1", "claude").await.allowed);

        service
            .reset_provider_circuit_breaker("p1", "claude")
            .await
            .expect("reset running breaker");

        let permit = router.allow_provider_request("p1", "claude").await;
        assert!(permit.allowed);
        assert!(!permit.used_half_open_permit);

        service.stop().await.expect("stop proxy");
    }
}
