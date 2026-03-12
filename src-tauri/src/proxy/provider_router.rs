use crate::{app_config::AppType, provider::Provider};

use super::{error::ProxyError, providers::get_adapter, server::ProxyServerState};

pub struct ProviderRouter {
    app_type: AppType,
    provider: Provider,
    needs_transform: bool,
}

impl ProviderRouter {
    pub async fn resolve_current(
        state: &ProxyServerState,
        app_type: AppType,
    ) -> Result<Self, ProxyError> {
        let provider_id = state
            .db
            .get_current_provider(app_type.as_str())
            .map_err(|error| {
                ProxyError::ConfigError(format!(
                    "load current provider for {} failed: {error}",
                    app_type.as_str()
                ))
            })?
            .ok_or_else(|| {
                ProxyError::ConfigError(format!(
                    "no current provider configured for {}",
                    app_type.as_str()
                ))
            })?;

        let provider = state
            .db
            .get_provider_by_id(&provider_id, app_type.as_str())
            .map_err(|error| {
                ProxyError::ConfigError(format!(
                    "load provider {} for {} failed: {error}",
                    provider_id,
                    app_type.as_str()
                ))
            })?
            .ok_or_else(|| {
                ProxyError::ConfigError(format!(
                    "current provider {} for {} was not found",
                    provider_id,
                    app_type.as_str()
                ))
            })?;

        let needs_transform = get_adapter(&app_type).needs_transform(&provider);

        Ok(Self {
            app_type,
            provider,
            needs_transform,
        })
    }

    pub fn provider(&self) -> &Provider {
        &self.provider
    }

    pub fn upstream_endpoint(&self, endpoint: &str) -> String {
        if matches!(self.app_type, AppType::Claude)
            && self.needs_transform
            && endpoint == "/v1/messages"
        {
            "/v1/chat/completions".to_string()
        } else {
            endpoint.to_string()
        }
    }
}
