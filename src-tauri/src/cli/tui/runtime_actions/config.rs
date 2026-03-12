use crate::app_config::AppType;
use crate::cli::i18n::texts;
use crate::error::AppError;
use crate::services::{ConfigService, ProviderService};
use crate::settings::set_webdav_sync_settings;

use super::super::app::{LoadingKind, Overlay, TextViewState, ToastKind};
use super::super::data::{load_state, UiData};
use super::super::runtime_systems::{WebDavReq, WebDavReqKind};
use super::helpers::{
    export_target, open_proxy_help as open_proxy_help_overlay, refresh_common_snippet_overlay,
};
use super::RuntimeActionContext;

pub(super) fn export(ctx: &mut RuntimeActionContext<'_>, path: String) -> Result<(), AppError> {
    let target = export_target(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }
    ConfigService::export_config_to_path(&target)?;
    ctx.app.push_toast(
        texts::tui_toast_exported_to(&target.display().to_string()),
        ToastKind::Success,
    );
    Ok(())
}

pub(super) fn show_full(ctx: &mut RuntimeActionContext<'_>) -> Result<(), AppError> {
    let state = load_state()?;
    let config = state.config.read().map_err(AppError::from)?;
    let content = serde_json::to_string_pretty(&*config)
        .map_err(|e| AppError::Message(texts::failed_to_serialize_json(&e.to_string())))?;
    let title = texts::config_show_full()
        .trim_start_matches("👁️")
        .trim()
        .to_string();
    ctx.app.overlay = Overlay::TextView(TextViewState {
        title,
        lines: content.lines().map(|s| s.to_string()).collect(),
        scroll: 0,
        action: None,
    });
    Ok(())
}

pub(super) fn import(ctx: &mut RuntimeActionContext<'_>, path: String) -> Result<(), AppError> {
    let source = std::path::PathBuf::from(path);
    if !source.exists() {
        return Err(AppError::Message(texts::tui_error_import_file_not_found(
            &source.display().to_string(),
        )));
    }
    let state = load_state()?;
    let backup_id = ConfigService::import_config_from_path(&source, &state)?;
    if let Err(e) = crate::services::provider::ProviderService::sync_current_to_live(&state) {
        log::warn!("配置导入后同步 live 配置失败: {e}");
    }
    if backup_id.is_empty() {
        ctx.app
            .push_toast(texts::tui_toast_imported_config(), ToastKind::Success);
    } else {
        ctx.app.push_toast(
            texts::tui_toast_imported_with_backup(&backup_id),
            ToastKind::Success,
        );
    }
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

pub(super) fn backup(
    ctx: &mut RuntimeActionContext<'_>,
    name: Option<String>,
) -> Result<(), AppError> {
    let db_path = crate::config::get_app_config_dir().join("cc-switch.db");
    let id = ConfigService::create_backup(&db_path, name)?;
    if id.is_empty() {
        ctx.app
            .push_toast(texts::tui_toast_no_config_file_to_backup(), ToastKind::Info);
    } else {
        ctx.app
            .push_toast(texts::tui_toast_backup_created(&id), ToastKind::Success);
    }
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

pub(super) fn restore_backup(
    ctx: &mut RuntimeActionContext<'_>,
    id: String,
) -> Result<(), AppError> {
    let state = load_state()?;
    let pre_backup = ConfigService::restore_from_backup_id(&id, &state)?;
    if let Err(e) = crate::services::provider::ProviderService::sync_current_to_live(&state) {
        log::warn!("备份恢复后同步 live 配置失败: {e}");
    }
    if pre_backup.is_empty() {
        ctx.app
            .push_toast(texts::tui_toast_restored_from_backup(), ToastKind::Success);
    } else {
        ctx.app.push_toast(
            texts::tui_toast_restored_with_pre_backup(&pre_backup),
            ToastKind::Success,
        );
    }
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

pub(super) fn validate(ctx: &mut RuntimeActionContext<'_>) -> Result<(), AppError> {
    let config_dir = crate::config::get_app_config_dir();
    let db_path = config_dir.join("cc-switch.db");
    if !db_path.exists() {
        ctx.app.push_toast(
            texts::tui_toast_config_file_does_not_exist(),
            ToastKind::Warning,
        );
        return Ok(());
    }

    let db = crate::Database::init()?;
    let claude_count = db.get_all_providers("claude")?.len();
    let codex_count = db.get_all_providers("codex")?.len();
    let gemini_count = db.get_all_providers("gemini")?.len();
    let mcp_count = db.get_all_mcp_servers()?.len();

    let lines = vec![
        texts::tui_config_validation_ok().to_string(),
        String::new(),
        texts::tui_config_validation_provider_count(AppType::Claude.as_str(), claude_count),
        texts::tui_config_validation_provider_count(AppType::Codex.as_str(), codex_count),
        texts::tui_config_validation_provider_count(AppType::Gemini.as_str(), gemini_count),
        texts::tui_config_validation_mcp_servers(mcp_count),
    ];
    ctx.app.overlay = Overlay::TextView(TextViewState {
        title: texts::tui_config_validation_title().to_string(),
        lines,
        scroll: 0,
        action: None,
    });
    ctx.app
        .push_toast(texts::tui_toast_validation_passed(), ToastKind::Success);
    Ok(())
}

pub(super) fn open_proxy_help(ctx: &mut RuntimeActionContext<'_>) -> Result<(), AppError> {
    open_proxy_help_overlay(ctx.app, ctx.data)
}

pub(super) fn clear_common_snippet(
    ctx: &mut RuntimeActionContext<'_>,
    app_type: AppType,
) -> Result<(), AppError> {
    let state = load_state()?;
    {
        let mut cfg = state.config.write().map_err(AppError::from)?;
        cfg.common_config_snippets.set(&app_type, None);
    }
    state.save()?;

    ctx.app
        .push_toast(texts::common_config_snippet_cleared(), ToastKind::Success);
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    refresh_common_snippet_overlay(ctx.app, ctx.data);
    Ok(())
}

pub(super) fn apply_common_snippet(
    ctx: &mut RuntimeActionContext<'_>,
    app_type: AppType,
) -> Result<(), AppError> {
    let state = load_state()?;
    let current_id = ProviderService::current(&state, app_type.clone())?;
    if current_id.trim().is_empty() {
        ctx.app.push_toast(
            texts::common_config_snippet_no_current_provider(),
            ToastKind::Info,
        );
        return Ok(());
    }
    ProviderService::switch(&state, app_type.clone(), &current_id)?;
    ctx.app
        .push_toast(texts::common_config_snippet_applied(), ToastKind::Success);
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

pub(super) fn webdav_check_connection(ctx: &mut RuntimeActionContext<'_>) -> Result<(), AppError> {
    queue_webdav_request(
        ctx,
        WebDavReqKind::CheckConnection,
        texts::tui_webdav_loading_title_check_connection().to_string(),
    )
}

pub(super) fn webdav_upload(ctx: &mut RuntimeActionContext<'_>) -> Result<(), AppError> {
    queue_webdav_request(
        ctx,
        WebDavReqKind::Upload,
        texts::tui_webdav_loading_title_upload().to_string(),
    )
}

pub(super) fn webdav_download(ctx: &mut RuntimeActionContext<'_>) -> Result<(), AppError> {
    queue_webdav_request(
        ctx,
        WebDavReqKind::Download,
        texts::tui_webdav_loading_title_download().to_string(),
    )
}

pub(super) fn webdav_migrate_v1_to_v2(ctx: &mut RuntimeActionContext<'_>) -> Result<(), AppError> {
    queue_webdav_request(
        ctx,
        WebDavReqKind::MigrateV1ToV2,
        texts::tui_webdav_loading_title_v1_migration().to_string(),
    )
}

pub(super) fn webdav_reset(ctx: &mut RuntimeActionContext<'_>) -> Result<(), AppError> {
    set_webdav_sync_settings(None)?;
    ctx.app.push_toast(
        texts::tui_toast_webdav_settings_cleared(),
        ToastKind::Success,
    );
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

pub(super) fn webdav_jianguoyun_quick_setup(
    ctx: &mut RuntimeActionContext<'_>,
    username: String,
    password: String,
) -> Result<(), AppError> {
    queue_webdav_request(
        ctx,
        WebDavReqKind::JianguoyunQuickSetup { username, password },
        texts::tui_webdav_loading_title_quick_setup().to_string(),
    )
}

pub(super) fn reset(ctx: &mut RuntimeActionContext<'_>) -> Result<(), AppError> {
    let config_dir = crate::config::get_app_config_dir();
    let db_path = config_dir.join("cc-switch.db");
    let backup_id = ConfigService::create_backup(&db_path, None)?;

    if db_path.exists() {
        std::fs::remove_file(&db_path).map_err(|e| AppError::io(&db_path, e))?;
    }
    let _ = crate::Database::init()?;
    if backup_id.is_empty() {
        ctx.app.push_toast(
            texts::tui_toast_config_reset_to_defaults(),
            ToastKind::Success,
        );
    } else {
        ctx.app.push_toast(
            texts::tui_toast_config_reset_with_backup(&backup_id),
            ToastKind::Success,
        );
    }
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

fn queue_webdav_request(
    ctx: &mut RuntimeActionContext<'_>,
    kind: WebDavReqKind,
    title: String,
) -> Result<(), AppError> {
    let Some(tx) = ctx.webdav_req_tx else {
        ctx.app.push_toast(
            texts::tui_toast_webdav_worker_disabled(),
            ToastKind::Warning,
        );
        return Ok(());
    };
    let request_id = ctx.webdav_loading.start();
    ctx.app.overlay = Overlay::Loading {
        kind: LoadingKind::WebDav,
        title,
        message: texts::tui_webdav_loading_message().to_string(),
    };
    if let Err(err) = tx.send(WebDavReq { request_id, kind }) {
        ctx.webdav_loading.cancel();
        ctx.app.overlay = Overlay::None;
        ctx.app.push_toast(
            texts::tui_toast_webdav_request_failed(&err.to_string()),
            ToastKind::Error,
        );
    }
    Ok(())
}
