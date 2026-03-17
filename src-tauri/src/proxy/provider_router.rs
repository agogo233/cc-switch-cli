use std::{collections::HashMap, str::FromStr, sync::Arc};

use tokio::sync::RwLock;

use crate::{app_config::AppType, database::Database, provider::Provider};

use super::{
    circuit_breaker::{AllowResult, CircuitBreaker, CircuitBreakerConfig, CircuitBreakerStats},
    error::ProxyError,
    providers::{get_adapter, get_claude_api_format},
};

pub struct ProviderRouter {
    db: Arc<Database>,
    circuit_breakers: Arc<RwLock<HashMap<String, Arc<CircuitBreaker>>>>,
}

impl ProviderRouter {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            circuit_breakers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn select_providers(&self, app_type: &str) -> Result<Vec<Provider>, ProxyError> {
        let mut result = Vec::new();
        let mut total_providers = 0usize;
        let mut circuit_open_count = 0usize;

        let auto_failover_enabled = self
            .db
            .get_proxy_config_for_app(app_type)
            .await
            .map(|config| config.auto_failover_enabled)
            .unwrap_or(false);

        if auto_failover_enabled {
            let all_providers = self
                .db
                .get_all_providers(app_type)
                .map_err(|error| ProxyError::DatabaseError(error.to_string()))?;
            let ordered_ids = self
                .db
                .get_failover_queue(app_type)
                .map_err(|error| ProxyError::DatabaseError(error.to_string()))?
                .into_iter()
                .map(|item| item.provider_id)
                .collect::<Vec<_>>();

            total_providers = ordered_ids.len();

            for provider_id in ordered_ids {
                let Some(provider) = all_providers.get(&provider_id).cloned() else {
                    continue;
                };

                let breaker = self
                    .get_or_create_circuit_breaker(&format!("{app_type}:{}", provider.id))
                    .await;

                if breaker.is_available().await {
                    result.push(provider);
                } else {
                    circuit_open_count += 1;
                }
            }
        } else {
            if let Some(current) = self.current_provider(app_type)? {
                total_providers = 1;
                result.push(current);
            }
        }

        if result.is_empty() {
            return if total_providers > 0 && circuit_open_count == total_providers {
                Err(ProxyError::AllProvidersCircuitOpen)
            } else {
                Err(ProxyError::NoProvidersConfigured)
            };
        }

        Ok(result)
    }

    pub async fn allow_provider_request(&self, provider_id: &str, app_type: &str) -> AllowResult {
        let breaker = self
            .get_or_create_circuit_breaker(&format!("{app_type}:{provider_id}"))
            .await;
        breaker.allow_request().await
    }

    pub async fn record_result(
        &self,
        provider_id: &str,
        app_type: &str,
        used_half_open_permit: bool,
        success: bool,
        error_msg: Option<String>,
    ) -> Result<(), ProxyError> {
        let failure_threshold = self
            .db
            .get_proxy_config_for_app(app_type)
            .await
            .map(|config| config.circuit_failure_threshold)
            .unwrap_or(5);

        let breaker = self
            .get_or_create_circuit_breaker(&format!("{app_type}:{provider_id}"))
            .await;

        if success {
            breaker.record_success(used_half_open_permit).await;
        } else {
            breaker.record_failure(used_half_open_permit).await;
        }

        self.db
            .update_provider_health_with_threshold(
                provider_id,
                app_type,
                success,
                error_msg,
                failure_threshold,
            )
            .await
            .map_err(|error| ProxyError::DatabaseError(error.to_string()))
    }

    pub async fn reset_circuit_breaker(&self, circuit_key: &str) {
        let breakers = self.circuit_breakers.read().await;
        if let Some(breaker) = breakers.get(circuit_key) {
            breaker.reset().await;
        }
    }

    pub async fn reset_provider_breaker(&self, provider_id: &str, app_type: &str) {
        self.reset_circuit_breaker(&format!("{app_type}:{provider_id}"))
            .await;
    }

    pub async fn release_permit_neutral(
        &self,
        provider_id: &str,
        app_type: &str,
        used_half_open_permit: bool,
    ) {
        if !used_half_open_permit {
            return;
        }

        let breaker = self
            .get_or_create_circuit_breaker(&format!("{app_type}:{provider_id}"))
            .await;
        breaker.release_half_open_permit();
    }

    pub async fn update_all_configs(&self, config: CircuitBreakerConfig) {
        let breakers = self.circuit_breakers.read().await;
        for breaker in breakers.values() {
            breaker.update_config(config.clone()).await;
        }
    }

    #[allow(dead_code)]
    pub async fn get_circuit_breaker_stats(
        &self,
        provider_id: &str,
        app_type: &str,
    ) -> Option<CircuitBreakerStats> {
        let circuit_key = format!("{app_type}:{provider_id}");
        let breakers = self.circuit_breakers.read().await;
        if let Some(breaker) = breakers.get(&circuit_key) {
            Some(breaker.get_stats().await)
        } else {
            None
        }
    }

    pub fn upstream_endpoint(
        &self,
        app_type: &AppType,
        provider: &Provider,
        endpoint: &str,
    ) -> String {
        if matches!(app_type, AppType::Claude)
            && get_adapter(app_type).needs_transform(provider)
            && endpoint == "/v1/messages"
        {
            match get_claude_api_format(provider) {
                "openai_responses" => "/v1/responses".to_string(),
                _ => "/v1/chat/completions".to_string(),
            }
        } else {
            endpoint.to_string()
        }
    }

    fn current_provider_id(&self, app_type: &AppType) -> Option<String> {
        self.db
            .get_current_provider(app_type.as_str())
            .ok()
            .flatten()
    }

    fn current_provider(&self, app_type: &str) -> Result<Option<Provider>, ProxyError> {
        let current_id = AppType::from_str(app_type)
            .ok()
            .and_then(|app_enum| self.current_provider_id(&app_enum))
            .or_else(|| self.db.get_current_provider(app_type).ok().flatten());

        match current_id {
            Some(current_id) => self
                .db
                .get_provider_by_id(&current_id, app_type)
                .map_err(|error| ProxyError::DatabaseError(error.to_string())),
            None => Ok(None),
        }
    }

    async fn get_or_create_circuit_breaker(&self, key: &str) -> Arc<CircuitBreaker> {
        {
            let breakers = self.circuit_breakers.read().await;
            if let Some(breaker) = breakers.get(key) {
                return breaker.clone();
            }
        }

        let mut breakers = self.circuit_breakers.write().await;
        if let Some(breaker) = breakers.get(key) {
            return breaker.clone();
        }

        let app_type = key.split(':').next().unwrap_or("claude");
        let config = self
            .db
            .get_proxy_config_for_app(app_type)
            .await
            .map(|app_config| CircuitBreakerConfig {
                failure_threshold: app_config.circuit_failure_threshold,
                success_threshold: app_config.circuit_success_threshold,
                timeout_seconds: app_config.circuit_timeout_seconds as u64,
                error_rate_threshold: app_config.circuit_error_rate_threshold,
                min_requests: app_config.circuit_min_requests,
            })
            .unwrap_or_default();

        let breaker = Arc::new(CircuitBreaker::new(config));
        breakers.insert(key.to_string(), breaker.clone());
        breaker
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{database::Database, proxy::circuit_breaker::CircuitBreakerConfig};
    use serde_json::json;
    use serial_test::serial;
    use std::{env, sync::Arc};
    use tempfile::TempDir;

    struct TempHome {
        #[allow(dead_code)]
        dir: TempDir,
        original_home: Option<String>,
        original_userprofile: Option<String>,
    }

    impl TempHome {
        fn new() -> Self {
            let dir = TempDir::new().expect("failed to create temp home");
            let original_home = env::var("HOME").ok();
            let original_userprofile = env::var("USERPROFILE").ok();

            env::set_var("HOME", dir.path());
            env::set_var("USERPROFILE", dir.path());

            Self {
                dir,
                original_home,
                original_userprofile,
            }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.original_home {
                Some(value) => env::set_var("HOME", value),
                None => env::remove_var("HOME"),
            }

            match &self.original_userprofile {
                Some(value) => env::set_var("USERPROFILE", value),
                None => env::remove_var("USERPROFILE"),
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_provider_router_creation() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());
        let router = ProviderRouter::new(db);

        let breaker = router.get_or_create_circuit_breaker("claude:test").await;
        assert!(breaker.allow_request().await.allowed);
    }

    #[tokio::test]
    #[serial]
    async fn test_failover_disabled_uses_current_provider() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        let provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        let provider_b =
            Provider::with_id("b".to_string(), "Provider B".to_string(), json!({}), None);

        db.save_provider("claude", &provider_a).unwrap();
        db.save_provider("claude", &provider_b).unwrap();
        db.set_current_provider("claude", "a").unwrap();
        db.add_to_failover_queue("claude", "b").unwrap();

        let router = ProviderRouter::new(db.clone());
        let providers = router.select_providers("claude").await.unwrap();

        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "a");
    }

    #[tokio::test]
    #[serial]
    async fn test_failover_enabled_uses_queue_order_ignoring_current() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        let mut provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        provider_a.sort_index = Some(2);
        let mut provider_b =
            Provider::with_id("b".to_string(), "Provider B".to_string(), json!({}), None);
        provider_b.sort_index = Some(1);

        db.save_provider("claude", &provider_a).unwrap();
        db.save_provider("claude", &provider_b).unwrap();
        db.set_current_provider("claude", "a").unwrap();
        db.add_to_failover_queue("claude", "b").unwrap();
        db.add_to_failover_queue("claude", "a").unwrap();

        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.auto_failover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        let router = ProviderRouter::new(db.clone());
        let providers = router.select_providers("claude").await.unwrap();

        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0].id, "b");
        assert_eq!(providers[1].id, "a");
    }

    #[tokio::test]
    #[serial]
    async fn test_failover_enabled_without_queue_returns_no_providers_configured() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        let provider = Provider::with_id(
            "codex-current".to_string(),
            "Codex Current".to_string(),
            json!({}),
            None,
        );

        db.save_provider("codex", &provider).unwrap();
        db.set_current_provider("codex", "codex-current").unwrap();

        let mut config = db.get_proxy_config_for_app("codex").await.unwrap();
        config.auto_failover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        let router = ProviderRouter::new(db.clone());
        let error = router
            .select_providers("codex")
            .await
            .expect_err("empty failover queue should no longer fall back to current provider");

        assert!(matches!(error, ProxyError::NoProvidersConfigured));
    }

    #[tokio::test]
    #[serial]
    async fn test_select_providers_does_not_consume_half_open_permit() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        db.update_circuit_breaker_config(&CircuitBreakerConfig {
            failure_threshold: 1,
            timeout_seconds: 0,
            ..Default::default()
        })
        .await
        .unwrap();

        let provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        let provider_b =
            Provider::with_id("b".to_string(), "Provider B".to_string(), json!({}), None);

        db.save_provider("claude", &provider_a).unwrap();
        db.save_provider("claude", &provider_b).unwrap();
        db.add_to_failover_queue("claude", "a").unwrap();
        db.add_to_failover_queue("claude", "b").unwrap();

        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.auto_failover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        let router = ProviderRouter::new(db.clone());

        router
            .record_result("b", "claude", false, false, Some("fail".to_string()))
            .await
            .unwrap();

        let providers = router.select_providers("claude").await.unwrap();
        assert_eq!(providers.len(), 2);

        assert!(router.allow_provider_request("b", "claude").await.allowed);
    }

    #[tokio::test]
    #[serial]
    async fn test_release_permit_neutral_frees_half_open_slot() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        db.update_circuit_breaker_config(&CircuitBreakerConfig {
            failure_threshold: 1,
            timeout_seconds: 0,
            ..Default::default()
        })
        .await
        .unwrap();

        let provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        db.save_provider("claude", &provider_a).unwrap();
        db.add_to_failover_queue("claude", "a").unwrap();

        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.auto_failover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        let router = ProviderRouter::new(db.clone());

        router
            .record_result("a", "claude", false, false, Some("fail".to_string()))
            .await
            .unwrap();

        let first = router.allow_provider_request("a", "claude").await;
        assert!(first.allowed);
        assert!(first.used_half_open_permit);

        let second = router.allow_provider_request("a", "claude").await;
        assert!(!second.allowed);

        router
            .release_permit_neutral("a", "claude", first.used_half_open_permit)
            .await;

        let third = router.allow_provider_request("a", "claude").await;
        assert!(third.allowed);
        assert!(third.used_half_open_permit);
    }

    #[tokio::test]
    #[serial]
    async fn test_record_result_uses_app_failure_threshold_for_health_updates() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        let provider =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        db.save_provider("claude", &provider).unwrap();

        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.circuit_failure_threshold = 2;
        db.update_proxy_config_for_app(config).await.unwrap();

        let router = ProviderRouter::new(db.clone());

        router
            .record_result("a", "claude", false, false, Some("fail-1".to_string()))
            .await
            .unwrap();
        let first_health = db.get_provider_health("a", "claude").await.unwrap();
        assert!(first_health.is_healthy);
        assert_eq!(first_health.consecutive_failures, 1);

        router
            .record_result("a", "claude", false, false, Some("fail-2".to_string()))
            .await
            .unwrap();
        let second_health = db.get_provider_health("a", "claude").await.unwrap();
        assert!(!second_health.is_healthy);
        assert_eq!(second_health.consecutive_failures, 2);
        assert_eq!(second_health.last_error.as_deref(), Some("fail-2"));
    }
}
