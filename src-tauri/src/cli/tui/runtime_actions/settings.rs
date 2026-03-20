use crate::app_config::AppType;
use crate::cli::i18n::texts;
use crate::error::AppError;

use super::super::data::{load_proxy_config, load_state, UiData};
use super::helpers::open_proxy_help_overlay_with;
use super::RuntimeActionContext;

pub(super) fn set_proxy_enabled(
    ctx: &mut RuntimeActionContext<'_>,
    enabled: bool,
) -> Result<(), AppError> {
    let state = load_state()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AppError::Message(format!("failed to create async runtime: {e}")))?;
    runtime.block_on(state.proxy_service.set_global_enabled(enabled))?;
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    ctx.app.push_toast(
        if enabled {
            crate::t!("Local proxy enabled.", "本地代理已开启。")
        } else {
            crate::t!("Local proxy disabled.", "本地代理已关闭。")
        },
        super::super::app::ToastKind::Success,
    );
    Ok(())
}

pub(super) fn set_proxy_listen_address(
    ctx: &mut RuntimeActionContext<'_>,
    address: String,
) -> Result<(), AppError> {
    update_proxy_config(ctx, |config| {
        config.listen_address = address;
    })
}

pub(super) fn set_proxy_listen_port(
    ctx: &mut RuntimeActionContext<'_>,
    port: u16,
) -> Result<(), AppError> {
    update_proxy_config(ctx, |config| {
        config.listen_port = port;
    })
}

pub(super) fn set_proxy_takeover(
    ctx: &mut RuntimeActionContext<'_>,
    app_type: AppType,
    enabled: bool,
) -> Result<(), AppError> {
    let state = load_state()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AppError::Message(format!("failed to create async runtime: {e}")))?;

    let status = runtime.block_on(state.proxy_service.get_status());
    if enabled && !status.running {
        ctx.app.push_toast(
            texts::tui_toast_proxy_takeover_requires_running(),
            super::super::app::ToastKind::Warning,
        );
        return Ok(());
    }

    runtime
        .block_on(
            state
                .proxy_service
                .set_takeover_for_app(app_type.as_str(), enabled),
        )
        .map_err(AppError::Message)?;

    *ctx.data = UiData::load(&ctx.app.app_type)?;
    open_proxy_help_overlay_with(ctx.app, ctx.data, load_proxy_config)?;
    ctx.app.push_toast(
        texts::tui_toast_proxy_takeover_updated(app_type.as_str(), enabled),
        super::super::app::ToastKind::Success,
    );
    Ok(())
}

pub(super) fn set_visible_apps(
    ctx: &mut RuntimeActionContext<'_>,
    apps: crate::settings::VisibleApps,
) -> Result<(), AppError> {
    set_visible_apps_with(ctx, apps, UiData::load)
}

pub(super) fn set_visible_apps_with<F>(
    ctx: &mut RuntimeActionContext<'_>,
    apps: crate::settings::VisibleApps,
    load_data: F,
) -> Result<(), AppError>
where
    F: FnOnce(&AppType) -> Result<UiData, AppError>,
{
    if apps.ordered_enabled().is_empty() {
        ctx.app.push_toast(
            texts::tui_toast_visible_apps_zero_selection_warning(),
            super::super::app::ToastKind::Warning,
        );
        return Ok(());
    }

    if apps.is_enabled_for(&ctx.app.app_type) {
        crate::settings::set_visible_apps(apps)?;
        ctx.app.push_toast(
            texts::tui_toast_visible_apps_saved(),
            super::super::app::ToastKind::Success,
        );
        return Ok(());
    }

    let next = crate::settings::next_visible_app(&apps, &ctx.app.app_type, 1).ok_or_else(|| {
        AppError::InvalidInput("At least one app must remain visible".to_string())
    })?;
    let next_data = load_data(&next)?;

    crate::settings::set_visible_apps(apps)?;
    super::apply_preloaded_app_switch(ctx.app, ctx.data, next, next_data);
    ctx.app.push_toast(
        texts::tui_toast_visible_apps_saved(),
        super::super::app::ToastKind::Success,
    );
    Ok(())
}

fn update_proxy_config(
    ctx: &mut RuntimeActionContext<'_>,
    mutate: impl FnOnce(&mut crate::proxy::ProxyConfig),
) -> Result<(), AppError> {
    let state = load_state()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AppError::Message(format!("failed to create async runtime: {e}")))?;

    let status = runtime.block_on(state.proxy_service.get_status());
    if status.running {
        *ctx.data = UiData::load(&ctx.app.app_type)?;
        ctx.app.push_toast(
            texts::tui_toast_proxy_settings_stop_before_edit(),
            super::super::app::ToastKind::Info,
        );
        return Ok(());
    }

    let mut config = runtime.block_on(state.proxy_service.get_config())?;
    mutate(&mut config);
    runtime.block_on(state.proxy_service.update_config(&config))?;

    *ctx.data = UiData::load(&ctx.app.app_type)?;
    ctx.app.push_toast(
        texts::tui_toast_proxy_settings_saved(),
        super::super::app::ToastKind::Success,
    );
    Ok(())
}
