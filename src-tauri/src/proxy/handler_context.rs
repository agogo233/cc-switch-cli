use std::time::Duration;

use crate::app_config::AppType;

use super::{
    error::ProxyError, provider_router::ProviderRouter, server::ProxyServerState,
    types::AppProxyConfig,
};

pub struct HandlerContext {
    pub state: ProxyServerState,
    pub app_type: AppType,
    pub provider_router: ProviderRouter,
    pub app_proxy: AppProxyConfig,
}

impl HandlerContext {
    pub async fn load(state: &ProxyServerState, app_type: AppType) -> Result<Self, ProxyError> {
        state.record_request_start().await;

        let provider_router = ProviderRouter::resolve_current(state, app_type.clone()).await?;
        state
            .record_active_target(&app_type, provider_router.provider())
            .await;

        let app_proxy = state
            .db
            .get_proxy_config_for_app(app_type.as_str())
            .await
            .map_err(|error| {
                ProxyError::ConfigError(format!(
                    "load proxy config for {} failed: {error}",
                    app_type.as_str()
                ))
            })?;

        Ok(Self {
            state: state.clone(),
            app_type,
            provider_router,
            app_proxy,
        })
    }

    pub fn streaming_first_byte_timeout(&self) -> Duration {
        Duration::from_secs(self.app_proxy.streaming_first_byte_timeout as u64)
    }

    pub fn streaming_idle_timeout(&self) -> Option<Duration> {
        match self.app_proxy.streaming_idle_timeout {
            0 => None,
            seconds => Some(Duration::from_secs(seconds as u64)),
        }
    }

    pub fn non_streaming_timeout(&self) -> Duration {
        Duration::from_secs(self.app_proxy.non_streaming_timeout as u64)
    }
}
