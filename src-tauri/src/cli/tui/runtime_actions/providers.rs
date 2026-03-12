use crate::cli::i18n::texts;
use crate::cli::tui::form::ClaudeApiFormat;
use crate::error::AppError;
use crate::proxy::providers::get_claude_api_format;
use crate::services::ProviderService;

use super::super::app::{ConfirmAction, ConfirmOverlay, Overlay, ToastKind};
use super::super::data::{load_state, UiData};
use super::super::form::ProviderAddField;
use super::super::runtime_systems::{next_model_fetch_request_id, ModelFetchReq, StreamCheckReq};
use super::RuntimeActionContext;

pub(super) fn switch(ctx: &mut RuntimeActionContext<'_>, id: String) -> Result<(), AppError> {
    let state = load_state()?;
    let switched_provider = ctx
        .data
        .providers
        .rows
        .iter()
        .find(|row| row.id == id)
        .map(|row| row.provider.clone());
    ProviderService::switch(&state, ctx.app.app_type.clone(), &id)?;
    if let Some(provider) = switched_provider.as_ref() {
        if let Err(err) =
            crate::claude_plugin::sync_claude_plugin_on_provider_switch(&ctx.app.app_type, provider)
        {
            ctx.app.push_toast(
                texts::tui_toast_claude_plugin_sync_failed(&err.to_string()),
                ToastKind::Warning,
            );
        }
    }
    *ctx.data = UiData::load(&ctx.app.app_type)?;

    let proxy_ready = ctx
        .data
        .proxy
        .routes_current_app_through_proxy(&ctx.app.app_type)
        .unwrap_or(false);
    if let Some(api_format) = switched_provider.as_ref().and_then(|provider| {
        provider_switch_proxy_notice_api_format(&ctx.app.app_type, provider, proxy_ready)
    }) {
        ctx.app.overlay = Overlay::Confirm(ConfirmOverlay {
            title: texts::tui_claude_api_format_requires_proxy_title().to_string(),
            message: texts::tui_claude_api_format_requires_proxy_message(api_format),
            action: ConfirmAction::ProviderApiFormatProxyNotice,
        });
    }

    Ok(())
}

fn provider_requires_local_proxy(
    app_type: &crate::app_config::AppType,
    provider: &crate::provider::Provider,
) -> Option<&'static str> {
    if !matches!(app_type, crate::app_config::AppType::Claude) {
        return None;
    }

    let api_format = get_claude_api_format(provider);
    ClaudeApiFormat::from_raw(api_format)
        .requires_proxy()
        .then_some(api_format)
}

fn provider_switch_proxy_notice_api_format(
    app_type: &crate::app_config::AppType,
    provider: &crate::provider::Provider,
    proxy_ready: bool,
) -> Option<&'static str> {
    provider_requires_local_proxy(app_type, provider).filter(|_| !proxy_ready)
}

pub(super) fn delete(ctx: &mut RuntimeActionContext<'_>, id: String) -> Result<(), AppError> {
    let state = load_state()?;
    ProviderService::delete(&state, ctx.app.app_type.clone(), &id)?;
    ctx.app
        .push_toast(texts::tui_toast_provider_deleted(), ToastKind::Success);
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

pub(super) fn speedtest(ctx: &mut RuntimeActionContext<'_>, url: String) -> Result<(), AppError> {
    let Some(tx) = ctx.speedtest_req_tx else {
        if matches!(&ctx.app.overlay, Overlay::SpeedtestRunning { url: running_url } if running_url == &url)
        {
            ctx.app.overlay = Overlay::None;
        }
        ctx.app
            .push_toast(texts::tui_toast_speedtest_disabled(), ToastKind::Warning);
        return Ok(());
    };

    if let Err(err) = tx.send(url.clone()) {
        if matches!(&ctx.app.overlay, Overlay::SpeedtestRunning { url: running_url } if running_url == &url)
        {
            ctx.app.overlay = Overlay::None;
        }
        ctx.app.push_toast(
            texts::tui_toast_speedtest_request_failed(&err.to_string()),
            ToastKind::Error,
        );
    }
    Ok(())
}

pub(super) fn stream_check(ctx: &mut RuntimeActionContext<'_>, id: String) -> Result<(), AppError> {
    let Some(tx) = ctx.stream_check_req_tx else {
        if matches!(&ctx.app.overlay, Overlay::StreamCheckRunning { provider_id, .. } if provider_id == &id)
        {
            ctx.app.overlay = Overlay::None;
        }
        ctx.app
            .push_toast(texts::tui_toast_stream_check_disabled(), ToastKind::Warning);
        return Ok(());
    };

    let Some(row) = ctx.data.providers.rows.iter().find(|row| row.id == id) else {
        return Ok(());
    };
    let req = StreamCheckReq {
        app_type: ctx.app.app_type.clone(),
        provider_id: row.id.clone(),
        provider_name: row.provider.name.clone(),
        provider: row.provider.clone(),
    };

    if let Err(err) = tx.send(req) {
        if matches!(&ctx.app.overlay, Overlay::StreamCheckRunning { provider_id, .. } if provider_id == &id)
        {
            ctx.app.overlay = Overlay::None;
        }
        ctx.app.push_toast(
            texts::tui_toast_stream_check_request_failed(&err.to_string()),
            ToastKind::Error,
        );
    }
    Ok(())
}

pub(super) fn model_fetch(
    ctx: &mut RuntimeActionContext<'_>,
    base_url: String,
    api_key: Option<String>,
    field: ProviderAddField,
    claude_idx: Option<usize>,
) -> Result<(), AppError> {
    let Some(tx) = ctx.model_fetch_req_tx else {
        ctx.app.push_toast(
            texts::tui_toast_model_fetch_worker_disabled(),
            ToastKind::Warning,
        );
        return Ok(());
    };
    let request_id = next_model_fetch_request_id();

    ctx.app.overlay = Overlay::ModelFetchPicker {
        request_id,
        field: field.clone(),
        claude_idx,
        input: String::new(),
        query: String::new(),
        fetching: true,
        models: Vec::new(),
        error: None,
        selected_idx: 0,
    };

    if let Err(err) = tx.send(ModelFetchReq::Fetch {
        request_id,
        base_url,
        api_key,
        field,
        claude_idx,
    }) {
        if let Overlay::ModelFetchPicker {
            fetching, error, ..
        } = &mut ctx.app.overlay
        {
            *fetching = false;
            *error = Some(texts::tui_model_fetch_error_hint(&err.to_string()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use serde_json::json;
    use serial_test::serial;
    use tempfile::TempDir;

    use super::*;
    use crate::cli::tui::app::App;
    use crate::cli::tui::app::{ConfirmAction, ConfirmOverlay};
    use crate::cli::tui::runtime_systems::RequestTracker;
    use crate::cli::tui::terminal::TuiTerminal;
    use crate::provider::Provider;
    use crate::{write_codex_live_atomic, AppType, MultiAppConfig};

    struct EnvGuard {
        old_home: Option<OsString>,
        old_userprofile: Option<OsString>,
    }

    impl EnvGuard {
        fn set_home(home: &Path) -> Self {
            let old_home = std::env::var_os("HOME");
            let old_userprofile = std::env::var_os("USERPROFILE");
            std::env::set_var("HOME", home);
            std::env::set_var("USERPROFILE", home);
            Self {
                old_home,
                old_userprofile,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.old_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match &self.old_userprofile {
                Some(value) => std::env::set_var("USERPROFILE", value),
                None => std::env::remove_var("USERPROFILE"),
            }
        }
    }

    fn codex_test_config() -> MultiAppConfig {
        let mut config = MultiAppConfig::default();
        let manager = config
            .get_manager_mut(&AppType::Codex)
            .expect("codex manager");
        manager.current = "old-provider".to_string();
        manager.providers.insert(
            "old-provider".to_string(),
            Provider::with_id(
                "old-provider".to_string(),
                "Legacy".to_string(),
                json!({
                    "auth": {"OPENAI_API_KEY": "stale"},
                    "config": "stale-config"
                }),
                None,
            ),
        );
        manager.providers.insert(
            "new-provider".to_string(),
            Provider::with_id(
                "new-provider".to_string(),
                "Latest".to_string(),
                json!({
                    "auth": {"OPENAI_API_KEY": "fresh-key"},
                    "config": "model_provider = \"latest\"\nmodel = \"gpt-5.2-codex\"\n\n[model_providers.latest]\nbase_url = \"https://api.example.com/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n"
                }),
                None,
            ),
        );
        config
    }

    fn claude_test_config(api_format: &str) -> MultiAppConfig {
        let mut config = MultiAppConfig::default();
        let manager = config
            .get_manager_mut(&AppType::Claude)
            .expect("claude manager");
        manager.current = "old-provider".to_string();
        manager.providers.insert(
            "old-provider".to_string(),
            Provider::with_id(
                "old-provider".to_string(),
                "Legacy Claude".to_string(),
                json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://api.anthropic.com",
                        "ANTHROPIC_API_KEY": "sk-old"
                    },
                    "api_format": "anthropic"
                }),
                None,
            ),
        );
        manager.providers.insert(
            "proxy-provider".to_string(),
            Provider::with_id(
                "proxy-provider".to_string(),
                "Proxy Claude".to_string(),
                json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://example.com",
                        "ANTHROPIC_API_KEY": "sk-new"
                    },
                    "api_format": api_format
                }),
                None,
            ),
        );
        config
    }

    fn claude_provider_with_api_format(api_format: &str) -> Provider {
        Provider::with_id(
            "proxy-provider".to_string(),
            "Proxy Claude".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://example.com",
                    "ANTHROPIC_API_KEY": "sk-new"
                },
                "api_format": api_format
            }),
            None,
        )
    }

    fn run_codex_switch(initialized: bool) -> Result<(Option<String>, String), AppError> {
        let temp_home = TempDir::new().expect("create temp home");
        let _env = EnvGuard::set_home(temp_home.path());

        if initialized {
            write_codex_live_atomic(
                &json!({"OPENAI_API_KEY": "legacy-key"}),
                Some("model_provider = \"legacy\"\nmodel = \"gpt-4\"\n"),
            )?;
        }

        codex_test_config().save()?;

        let mut terminal = TuiTerminal::new_for_test()?;
        let mut app = App::new(Some(AppType::Codex));
        let mut data = UiData::load(&AppType::Codex)?;
        let mut proxy_loading = RequestTracker::default();
        let mut webdav_loading = RequestTracker::default();
        let mut update_check = RequestTracker::default();
        let mut ctx = RuntimeActionContext {
            terminal: &mut terminal,
            app: &mut app,
            data: &mut data,
            speedtest_req_tx: None,
            stream_check_req_tx: None,
            skills_req_tx: None,
            proxy_req_tx: None,
            proxy_loading: &mut proxy_loading,
            local_env_req_tx: None,
            webdav_req_tx: None,
            webdav_loading: &mut webdav_loading,
            update_req_tx: None,
            update_check: &mut update_check,
            model_fetch_req_tx: None,
        };

        switch(&mut ctx, "new-provider".to_string())?;

        Ok((
            app.toast.as_ref().map(|toast| toast.message.clone()),
            data.providers.current_id,
        ))
    }

    fn run_claude_switch(api_format: &str) -> Result<(Overlay, String), AppError> {
        let temp_home = TempDir::new().expect("create temp home");
        let _env = EnvGuard::set_home(temp_home.path());

        claude_test_config(api_format).save()?;

        let mut terminal = TuiTerminal::new_for_test()?;
        let mut app = App::new(Some(AppType::Claude));
        let mut data = UiData::load(&AppType::Claude)?;
        let mut proxy_loading = RequestTracker::default();
        let mut webdav_loading = RequestTracker::default();
        let mut update_check = RequestTracker::default();
        let mut ctx = RuntimeActionContext {
            terminal: &mut terminal,
            app: &mut app,
            data: &mut data,
            speedtest_req_tx: None,
            stream_check_req_tx: None,
            skills_req_tx: None,
            proxy_req_tx: None,
            proxy_loading: &mut proxy_loading,
            local_env_req_tx: None,
            webdav_req_tx: None,
            webdav_loading: &mut webdav_loading,
            update_req_tx: None,
            update_check: &mut update_check,
            model_fetch_req_tx: None,
        };

        switch(&mut ctx, "proxy-provider".to_string())?;

        Ok((app.overlay.clone(), data.providers.current_id))
    }

    #[test]
    #[serial]
    fn provider_switch_does_not_show_restart_toast_when_live_sync_succeeds() {
        let (toast, current_id) = run_codex_switch(true).expect("switch should succeed");

        assert_eq!(current_id, "new-provider");
        assert!(
            toast.is_none(),
            "provider switch should not show restart toast"
        );
    }

    #[test]
    #[serial]
    fn provider_switch_does_not_show_restart_toast_when_live_sync_is_skipped() {
        let (toast, current_id) = run_codex_switch(false).expect("switch should succeed");

        assert_eq!(current_id, "new-provider");
        assert!(
            toast.is_none(),
            "provider switch should not show restart toast"
        );
    }

    #[test]
    #[serial]
    fn provider_switch_warns_when_claude_provider_requires_proxy_and_proxy_is_not_running() {
        let (overlay, current_id) =
            run_claude_switch("openai_chat").expect("switch should succeed");

        assert_eq!(current_id, "proxy-provider");
        assert!(matches!(
            overlay,
            Overlay::Confirm(ConfirmOverlay { title, message, action })
                if title == texts::tui_claude_api_format_requires_proxy_title()
                    && message == texts::tui_claude_api_format_requires_proxy_message("openai_chat")
                    && matches!(action, ConfirmAction::ProviderApiFormatProxyNotice)
        ));
    }

    #[test]
    #[serial]
    fn provider_switch_warns_for_openai_responses_when_proxy_is_not_running() {
        let (overlay, current_id) =
            run_claude_switch("openai_responses").expect("switch should succeed");

        assert_eq!(current_id, "proxy-provider");
        assert!(matches!(
            overlay,
            Overlay::Confirm(ConfirmOverlay { title, message, action })
                if title == texts::tui_claude_api_format_requires_proxy_title()
                    && message == texts::tui_claude_api_format_requires_proxy_message("openai_responses")
                    && matches!(action, ConfirmAction::ProviderApiFormatProxyNotice)
        ));
    }

    #[test]
    fn provider_switch_notice_is_suppressed_when_current_app_already_routes_through_proxy() {
        let provider = claude_provider_with_api_format("openai_chat");

        let notice = provider_switch_proxy_notice_api_format(&AppType::Claude, &provider, true);

        assert_eq!(notice, None);
    }

    #[test]
    fn provider_switch_notice_uses_openai_responses_api_format_when_proxy_is_not_ready() {
        let provider = claude_provider_with_api_format("openai_responses");

        let notice = provider_switch_proxy_notice_api_format(&AppType::Claude, &provider, false);

        assert_eq!(notice, Some("openai_responses"));
    }

    #[test]
    #[serial]
    fn provider_switch_does_not_warn_when_claude_provider_uses_anthropic_format() {
        let (overlay, current_id) = run_claude_switch("anthropic").expect("switch should succeed");

        assert_eq!(current_id, "proxy-provider");
        assert!(matches!(overlay, Overlay::None));
    }
}
