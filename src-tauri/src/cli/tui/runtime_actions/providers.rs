use crate::cli::i18n::texts;
use crate::error::AppError;
use crate::services::ProviderService;

use super::super::app::{Overlay, ToastKind};
use super::super::data::{load_state, UiData};
use super::super::form::ProviderAddField;
use super::super::runtime_systems::{next_model_fetch_request_id, ModelFetchReq, StreamCheckReq};
use super::RuntimeActionContext;

pub(super) fn switch(ctx: &mut RuntimeActionContext<'_>, id: String) -> Result<(), AppError> {
    let state = load_state()?;
    let provider = ctx
        .data
        .providers
        .rows
        .iter()
        .find(|row| row.id == id)
        .map(|row| row.provider.clone());
    ProviderService::switch(&state, ctx.app.app_type.clone(), &id)?;
    if let Some(provider) = provider {
        if let Err(err) = crate::claude_plugin::sync_claude_plugin_on_provider_switch(
            &ctx.app.app_type,
            &provider,
        ) {
            ctx.app.push_toast(
                texts::tui_toast_claude_plugin_sync_failed(&err.to_string()),
                ToastKind::Warning,
            );
        }
    }
    if !crate::sync_policy::should_sync_live(&ctx.app.app_type) {
        let mut message =
            texts::tui_toast_live_sync_skipped_uninitialized(ctx.app.app_type.as_str());
        message.push(' ');
        message.push_str(texts::restart_note());
        ctx.app.push_toast(message, ToastKind::Warning);
    } else {
        ctx.app
            .push_toast(texts::restart_note(), ToastKind::Success);
    }
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
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
