use std::sync::mpsc;

use crate::cli::i18n::{set_language, texts};
use crate::error::AppError;

use super::app::{Action, App, Overlay, ToastKind};
use super::data::UiData;
use super::runtime_systems::{
    LocalEnvReq, ModelFetchReq, ProxyReq, RequestTracker, SkillsReq, StreamCheckReq, UpdateReq,
    WebDavReq,
};
use super::terminal::TuiTerminal;

mod config;
mod editor;
mod helpers;
mod mcp;
mod prompts;
mod providers;
mod settings;
mod skills;
mod updates;

pub(crate) use helpers::{app_display_name, queue_managed_proxy_action};
#[cfg(test)]
pub(crate) use helpers::{
    import_mcp_for_current_app_with, open_proxy_help_overlay_with,
    run_external_editor_for_current_editor,
};

pub(super) struct RuntimeActionContext<'a> {
    terminal: &'a mut TuiTerminal,
    app: &'a mut App,
    data: &'a mut UiData,
    speedtest_req_tx: Option<&'a mpsc::Sender<String>>,
    stream_check_req_tx: Option<&'a mpsc::Sender<StreamCheckReq>>,
    skills_req_tx: Option<&'a mpsc::Sender<SkillsReq>>,
    proxy_req_tx: Option<&'a mpsc::Sender<ProxyReq>>,
    proxy_loading: &'a mut RequestTracker,
    local_env_req_tx: Option<&'a mpsc::Sender<LocalEnvReq>>,
    webdav_req_tx: Option<&'a mpsc::Sender<WebDavReq>>,
    webdav_loading: &'a mut RequestTracker,
    update_req_tx: Option<&'a mpsc::Sender<UpdateReq>>,
    update_check: &'a mut RequestTracker,
    model_fetch_req_tx: Option<&'a mpsc::Sender<ModelFetchReq>>,
}

pub(crate) fn handle_action(
    terminal: &mut TuiTerminal,
    app: &mut App,
    data: &mut UiData,
    speedtest_req_tx: Option<&mpsc::Sender<String>>,
    stream_check_req_tx: Option<&mpsc::Sender<StreamCheckReq>>,
    skills_req_tx: Option<&mpsc::Sender<SkillsReq>>,
    proxy_req_tx: Option<&mpsc::Sender<ProxyReq>>,
    proxy_loading: &mut RequestTracker,
    local_env_req_tx: Option<&mpsc::Sender<LocalEnvReq>>,
    webdav_req_tx: Option<&mpsc::Sender<WebDavReq>>,
    webdav_loading: &mut RequestTracker,
    update_req_tx: Option<&mpsc::Sender<UpdateReq>>,
    update_check: &mut RequestTracker,
    model_fetch_req_tx: Option<&mpsc::Sender<ModelFetchReq>>,
    action: Action,
) -> Result<(), AppError> {
    let mut ctx = RuntimeActionContext {
        terminal,
        app,
        data,
        speedtest_req_tx,
        stream_check_req_tx,
        skills_req_tx,
        proxy_req_tx,
        proxy_loading,
        local_env_req_tx,
        webdav_req_tx,
        webdav_loading,
        update_req_tx,
        update_check,
        model_fetch_req_tx,
    };

    match action {
        Action::None => Ok(()),
        Action::ReloadData => {
            *ctx.data = UiData::load(&ctx.app.app_type)?;
            Ok(())
        }
        Action::SetAppType(next) => {
            let next_data = UiData::load(&next)?;
            ctx.app.app_type = next;
            *ctx.data = next_data;
            ctx.app.reset_proxy_activity(ctx.data.proxy.total_requests);
            Ok(())
        }
        Action::LocalEnvRefresh => {
            let Some(tx) = ctx.local_env_req_tx else {
                ctx.app.local_env_loading = false;
                ctx.app.push_toast(
                    texts::tui_toast_local_env_check_disabled(),
                    ToastKind::Warning,
                );
                return Ok(());
            };

            ctx.app.local_env_loading = true;
            if let Err(err) = tx.send(LocalEnvReq::Refresh) {
                ctx.app.local_env_loading = false;
                ctx.app.push_toast(
                    texts::tui_toast_local_env_check_request_failed(&err.to_string()),
                    ToastKind::Warning,
                );
            }
            Ok(())
        }
        Action::SwitchRoute(route) => {
            ctx.app.route = route;
            Ok(())
        }
        Action::Quit => {
            ctx.app.should_quit = true;
            Ok(())
        }
        Action::SkillsToggle { directory, enabled } => skills::toggle(&mut ctx, directory, enabled),
        Action::SkillsSetApps { directory, apps } => skills::set_apps(&mut ctx, directory, apps),
        Action::SkillsInstall { spec } => skills::install(&mut ctx, spec),
        Action::SkillsUninstall { directory } => skills::uninstall(&mut ctx, directory),
        Action::SkillsSync { app: scope } => skills::sync(&mut ctx, scope),
        Action::SkillsSetSyncMethod { method } => skills::set_sync_method(&mut ctx, method),
        Action::SkillsDiscover { query } => skills::discover(&mut ctx, query),
        Action::SkillsRepoAdd { spec } => skills::repo_add(&mut ctx, spec),
        Action::SkillsRepoRemove { owner, name } => skills::repo_remove(&mut ctx, owner, name),
        Action::SkillsRepoToggleEnabled {
            owner,
            name,
            enabled,
        } => skills::repo_toggle_enabled(&mut ctx, owner, name, enabled),
        Action::SkillsOpenImport => skills::open_import(&mut ctx),
        Action::SkillsScanUnmanaged => skills::scan_unmanaged(&mut ctx),
        Action::SkillsImportFromApps { directories } => {
            skills::import_from_apps(&mut ctx, directories)
        }
        Action::EditorDiscard => {
            ctx.app.editor = None;
            Ok(())
        }
        Action::EditorOpenExternal => editor::open_external(&mut ctx),
        Action::EditorSubmit { submit, content } => editor::submit(&mut ctx, submit, content),
        Action::ProviderSwitch { id } => providers::switch(&mut ctx, id),
        Action::ProviderDelete { id } => providers::delete(&mut ctx, id),
        Action::ProviderSpeedtest { url } => providers::speedtest(&mut ctx, url),
        Action::ProviderStreamCheck { id } => providers::stream_check(&mut ctx, id),
        Action::ProviderModelFetch {
            base_url,
            api_key,
            field,
            claude_idx,
        } => providers::model_fetch(&mut ctx, base_url, api_key, field, claude_idx),
        Action::McpToggle { id, enabled } => mcp::toggle(&mut ctx, id, enabled),
        Action::McpSetApps { id, apps } => mcp::set_apps(&mut ctx, id, apps),
        Action::McpDelete { id } => mcp::delete(&mut ctx, id),
        Action::McpImport => mcp::import_current_app(&mut ctx),
        Action::PromptActivate { id } => prompts::activate(&mut ctx, id),
        Action::PromptDeactivate { id } => prompts::deactivate(&mut ctx, id),
        Action::PromptDelete { id } => prompts::delete(&mut ctx, id),
        Action::ConfigExport { path } => config::export(&mut ctx, path),
        Action::ConfigShowFull => config::show_full(&mut ctx),
        Action::ConfigImport { path } => config::import(&mut ctx, path),
        Action::ConfigBackup { name } => config::backup(&mut ctx, name),
        Action::ConfigRestoreBackup { id } => config::restore_backup(&mut ctx, id),
        Action::ConfigValidate => config::validate(&mut ctx),
        Action::ConfigOpenProxyHelp => config::open_proxy_help(&mut ctx),
        Action::ConfigCommonSnippetClear { app_type } => {
            config::clear_common_snippet(&mut ctx, app_type)
        }
        Action::ConfigCommonSnippetApply { app_type } => {
            config::apply_common_snippet(&mut ctx, app_type)
        }
        Action::ConfigWebDavCheckConnection => config::webdav_check_connection(&mut ctx),
        Action::ConfigWebDavUpload => config::webdav_upload(&mut ctx),
        Action::ConfigWebDavDownload => config::webdav_download(&mut ctx),
        Action::ConfigWebDavMigrateV1ToV2 => config::webdav_migrate_v1_to_v2(&mut ctx),
        Action::ConfigWebDavReset => config::webdav_reset(&mut ctx),
        Action::ConfigWebDavJianguoyunQuickSetup { username, password } => {
            config::webdav_jianguoyun_quick_setup(&mut ctx, username, password)
        }
        Action::ConfigReset => config::reset(&mut ctx),
        Action::SetSkipClaudeOnboarding { enabled } => {
            crate::settings::set_skip_claude_onboarding(enabled)?;
            ctx.app.push_toast(
                texts::tui_toast_skip_claude_onboarding_toggled(enabled),
                ToastKind::Success,
            );
            Ok(())
        }
        Action::SetClaudePluginIntegration { enabled } => {
            crate::settings::set_enable_claude_plugin_integration(enabled)?;
            if let Err(err) = crate::claude_plugin::sync_claude_plugin_on_settings_toggle(enabled) {
                ctx.app.push_toast(
                    texts::tui_toast_claude_plugin_sync_failed(&err.to_string()),
                    ToastKind::Warning,
                );
            }
            ctx.app.push_toast(
                texts::tui_toast_claude_plugin_integration_toggled(enabled),
                ToastKind::Success,
            );
            Ok(())
        }
        Action::SetProxyEnabled { enabled } => settings::set_proxy_enabled(&mut ctx, enabled),
        Action::SetProxyTakeover { app_type, enabled } => {
            settings::set_proxy_takeover(&mut ctx, app_type, enabled)
        }
        Action::SetManagedProxyForCurrentApp { app_type, enabled } => queue_managed_proxy_action(
            ctx.app,
            ctx.proxy_req_tx,
            ctx.proxy_loading,
            app_type,
            enabled,
        ),
        Action::SetLanguage(lang) => {
            set_language(lang)?;
            ctx.app
                .push_toast(texts::language_changed(), ToastKind::Success);
            Ok(())
        }
        Action::CheckUpdate => updates::check(&mut ctx),
        Action::ConfirmUpdate => updates::confirm(&mut ctx),
        Action::CancelUpdate => {
            ctx.app.overlay = Overlay::None;
            Ok(())
        }
        Action::CancelUpdateCheck => {
            ctx.update_check.cancel();
            Ok(())
        }
    }
}
