use serde_json::{json, Value};

use crate::app_config::{AppType, McpServer};
use crate::cli::i18n::texts;
use crate::error::AppError;
use crate::provider::Provider;
use crate::services::{McpService, PromptService, ProviderService};
use crate::settings::{set_webdav_sync_settings, WebDavSyncSettings};

use super::super::app::{EditorSubmit, Overlay, TextViewState, ToastKind};
use super::super::data::{load_state, UiData};
use super::super::form::FormState;
use super::helpers::run_external_editor_for_current_editor;
use super::RuntimeActionContext;

pub(super) fn open_external(ctx: &mut RuntimeActionContext<'_>) -> Result<(), AppError> {
    ctx.terminal.with_terminal_restored(|| {
        run_external_editor_for_current_editor(ctx.app, crate::cli::editor::open_external_editor)
    })
}

pub(super) fn submit(
    ctx: &mut RuntimeActionContext<'_>,
    submit: EditorSubmit,
    content: String,
) -> Result<(), AppError> {
    match submit {
        EditorSubmit::PromptEdit { id } => submit_prompt_edit(ctx, id, content),
        EditorSubmit::ProviderFormApplyJson => submit_provider_form_apply_json(ctx, content),
        EditorSubmit::ProviderFormApplyCodexAuth => {
            submit_provider_form_apply_codex_auth(ctx, content)
        }
        EditorSubmit::ProviderFormApplyCodexConfigToml => {
            submit_provider_form_apply_codex_config_toml(ctx, content)
        }
        EditorSubmit::ProviderAdd => submit_provider_add(ctx, content),
        EditorSubmit::ProviderEdit { id } => submit_provider_edit(ctx, id, content),
        EditorSubmit::McpAdd => submit_mcp_add(ctx, content),
        EditorSubmit::McpEdit { id } => submit_mcp_edit(ctx, id, content),
        EditorSubmit::ConfigCommonSnippet { app_type } => {
            submit_config_common_snippet(ctx, app_type, content)
        }
        EditorSubmit::ConfigWebDavSettings => submit_webdav_settings(ctx, content),
    }
}

fn submit_prompt_edit(
    ctx: &mut RuntimeActionContext<'_>,
    id: String,
    content: String,
) -> Result<(), AppError> {
    let state = load_state()?;
    let prompts = PromptService::get_prompts(&state, ctx.app.app_type.clone())?;
    let Some(mut prompt) = prompts.get(&id).cloned() else {
        ctx.app
            .push_toast(texts::tui_toast_prompt_not_found(&id), ToastKind::Error);
        return Ok(());
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    prompt.content = content;
    prompt.updated_at = Some(timestamp);

    if let Err(err) = PromptService::upsert_prompt(&state, ctx.app.app_type.clone(), &id, prompt) {
        ctx.app.push_toast(err.to_string(), ToastKind::Error);
        return Ok(());
    }

    ctx.app.editor = None;
    ctx.app
        .push_toast(texts::tui_toast_prompt_edit_finished(), ToastKind::Success);
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

fn submit_provider_form_apply_json(
    ctx: &mut RuntimeActionContext<'_>,
    content: String,
) -> Result<(), AppError> {
    let settings_value: Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(e) => {
            ctx.app.push_toast(
                texts::tui_toast_invalid_json(&e.to_string()),
                ToastKind::Error,
            );
            return Ok(());
        }
    };

    if !settings_value.is_object() {
        ctx.app
            .push_toast(texts::tui_toast_json_must_be_object(), ToastKind::Error);
        return Ok(());
    }

    let provider_value = match ctx.app.form.as_ref() {
        Some(FormState::ProviderAdd(form)) => {
            let mut provider_value = form.to_provider_json_value();
            if let Some(obj) = provider_value.as_object_mut() {
                obj.insert("settingsConfig".to_string(), settings_value);
            }
            Some(provider_value)
        }
        _ => None,
    };

    if let Some(provider_value) = provider_value {
        let apply_result = match ctx.app.form.as_mut() {
            Some(FormState::ProviderAdd(form)) => {
                form.apply_provider_json_value_to_fields(provider_value)
            }
            _ => Ok(()),
        };

        if let Err(err) = apply_result {
            ctx.app.push_toast(err, ToastKind::Error);
            return Ok(());
        }
    }
    ctx.app.editor = None;
    Ok(())
}

fn submit_provider_form_apply_codex_auth(
    ctx: &mut RuntimeActionContext<'_>,
    content: String,
) -> Result<(), AppError> {
    let auth_value: Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(e) => {
            ctx.app.push_toast(
                texts::tui_toast_invalid_json(&e.to_string()),
                ToastKind::Error,
            );
            return Ok(());
        }
    };

    if !auth_value.is_object() {
        ctx.app
            .push_toast(texts::tui_toast_json_must_be_object(), ToastKind::Error);
        return Ok(());
    }

    let provider_value = match ctx.app.form.as_ref() {
        Some(FormState::ProviderAdd(form)) => {
            let mut provider_value = form.to_provider_json_value();
            if let Some(settings_value) = provider_value
                .as_object_mut()
                .and_then(|obj| obj.get_mut("settingsConfig"))
            {
                if !settings_value.is_object() {
                    *settings_value = json!({});
                }
                if let Some(settings_obj) = settings_value.as_object_mut() {
                    settings_obj.insert("auth".to_string(), auth_value);
                }
            }
            Some(provider_value)
        }
        _ => None,
    };

    if let Some(provider_value) = provider_value {
        let apply_result = match ctx.app.form.as_mut() {
            Some(FormState::ProviderAdd(form)) => {
                form.apply_provider_json_value_to_fields(provider_value)
            }
            _ => Ok(()),
        };

        if let Err(err) = apply_result {
            ctx.app.push_toast(err, ToastKind::Error);
            return Ok(());
        }
    }

    ctx.app.editor = None;
    Ok(())
}

fn submit_provider_form_apply_codex_config_toml(
    ctx: &mut RuntimeActionContext<'_>,
    content: String,
) -> Result<(), AppError> {
    use toml_edit::DocumentMut;

    let config_text = if content.trim().is_empty() {
        String::new()
    } else {
        let doc: DocumentMut = match content.parse() {
            Ok(doc) => doc,
            Err(e) => {
                ctx.app.push_toast(
                    texts::common_config_snippet_invalid_toml(&e.to_string()),
                    ToastKind::Error,
                );
                return Ok(());
            }
        };
        doc.to_string()
    };

    let provider_value = match ctx.app.form.as_ref() {
        Some(FormState::ProviderAdd(form)) => {
            let mut provider_value = form.to_provider_json_value();
            if let Some(settings_value) = provider_value
                .as_object_mut()
                .and_then(|obj| obj.get_mut("settingsConfig"))
            {
                if !settings_value.is_object() {
                    *settings_value = json!({});
                }
                if let Some(settings_obj) = settings_value.as_object_mut() {
                    settings_obj.insert("config".to_string(), Value::String(config_text));
                }
            }
            Some(provider_value)
        }
        _ => None,
    };

    if let Some(provider_value) = provider_value {
        let apply_result = match ctx.app.form.as_mut() {
            Some(FormState::ProviderAdd(form)) => {
                form.apply_provider_json_value_to_fields(provider_value)
            }
            _ => Ok(()),
        };

        if let Err(err) = apply_result {
            ctx.app.push_toast(err, ToastKind::Error);
            return Ok(());
        }
    }

    ctx.app.editor = None;
    Ok(())
}

fn submit_provider_add(
    ctx: &mut RuntimeActionContext<'_>,
    content: String,
) -> Result<(), AppError> {
    let provider: Provider = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            ctx.app.push_toast(
                texts::tui_toast_invalid_json(&e.to_string()),
                ToastKind::Error,
            );
            return Ok(());
        }
    };

    if provider.id.trim().is_empty() || provider.name.trim().is_empty() {
        ctx.app.push_toast(
            texts::tui_toast_provider_add_missing_fields(),
            ToastKind::Warning,
        );
        return Ok(());
    }

    let state = load_state()?;
    match ProviderService::add(&state, ctx.app.app_type.clone(), provider) {
        Ok(true) => {
            ctx.app.editor = None;
            ctx.app.form = None;
            ctx.app
                .push_toast(texts::tui_toast_provider_add_finished(), ToastKind::Success);
            *ctx.data = UiData::load(&ctx.app.app_type)?;
        }
        Ok(false) => {
            ctx.app
                .push_toast(texts::tui_toast_provider_add_failed(), ToastKind::Error);
        }
        Err(err) => {
            ctx.app.push_toast(err.to_string(), ToastKind::Error);
        }
    }

    Ok(())
}

fn submit_provider_edit(
    ctx: &mut RuntimeActionContext<'_>,
    id: String,
    content: String,
) -> Result<(), AppError> {
    let mut provider: Provider = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            ctx.app.push_toast(
                texts::tui_toast_invalid_json(&e.to_string()),
                ToastKind::Error,
            );
            return Ok(());
        }
    };
    provider.id = id.clone();

    if provider.name.trim().is_empty() {
        ctx.app
            .push_toast(texts::tui_toast_provider_missing_name(), ToastKind::Warning);
        return Ok(());
    }

    let state = load_state()?;
    if let Err(err) = ProviderService::update(&state, ctx.app.app_type.clone(), provider) {
        ctx.app.push_toast(err.to_string(), ToastKind::Error);
        return Ok(());
    }

    ctx.app.editor = None;
    ctx.app.form = None;
    ctx.app.push_toast(
        texts::tui_toast_provider_edit_finished(),
        ToastKind::Success,
    );
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

fn submit_mcp_add(ctx: &mut RuntimeActionContext<'_>, content: String) -> Result<(), AppError> {
    let server: McpServer = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            ctx.app.push_toast(
                texts::tui_toast_invalid_json(&e.to_string()),
                ToastKind::Error,
            );
            return Ok(());
        }
    };

    if server.id.trim().is_empty() || server.name.trim().is_empty() {
        ctx.app
            .push_toast(texts::tui_toast_mcp_missing_fields(), ToastKind::Warning);
        return Ok(());
    }

    let state = load_state()?;
    if let Err(err) = McpService::upsert_server(&state, server) {
        ctx.app.push_toast(err.to_string(), ToastKind::Error);
        return Ok(());
    }

    ctx.app.editor = None;
    ctx.app.form = None;
    ctx.app
        .push_toast(texts::tui_toast_mcp_upserted(), ToastKind::Success);
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

fn submit_mcp_edit(
    ctx: &mut RuntimeActionContext<'_>,
    id: String,
    content: String,
) -> Result<(), AppError> {
    let mut server: McpServer = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            ctx.app.push_toast(
                texts::tui_toast_invalid_json(&e.to_string()),
                ToastKind::Error,
            );
            return Ok(());
        }
    };
    server.id = id.clone();

    if server.name.trim().is_empty() {
        ctx.app
            .push_toast(texts::tui_toast_mcp_missing_fields(), ToastKind::Warning);
        return Ok(());
    }

    let state = load_state()?;
    if let Err(err) = McpService::upsert_server(&state, server) {
        ctx.app.push_toast(err.to_string(), ToastKind::Error);
        return Ok(());
    }

    ctx.app.editor = None;
    ctx.app.form = None;
    ctx.app
        .push_toast(texts::tui_toast_mcp_upserted(), ToastKind::Success);
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

fn submit_config_common_snippet(
    ctx: &mut RuntimeActionContext<'_>,
    app_type: AppType,
    content: String,
) -> Result<(), AppError> {
    let edited = content.trim().to_string();
    let (next_snippet, toast) = if edited.is_empty() {
        (None, texts::common_config_snippet_cleared())
    } else if matches!(app_type, AppType::Codex) {
        let doc: toml_edit::DocumentMut = match edited.parse() {
            Ok(v) => v,
            Err(e) => {
                ctx.app.push_toast(
                    texts::common_config_snippet_invalid_toml(&e.to_string()),
                    ToastKind::Error,
                );
                return Ok(());
            }
        };
        let canonical = doc.to_string().trim().to_string();
        (Some(canonical), texts::common_config_snippet_saved())
    } else {
        let value: Value = match serde_json::from_str(&edited) {
            Ok(v) => v,
            Err(e) => {
                ctx.app.push_toast(
                    texts::common_config_snippet_invalid_json(&e.to_string()),
                    ToastKind::Error,
                );
                return Ok(());
            }
        };

        if !value.is_object() {
            ctx.app
                .push_toast(texts::common_config_snippet_not_object(), ToastKind::Error);
            return Ok(());
        }

        let pretty = match serde_json::to_string_pretty(&value) {
            Ok(v) => v,
            Err(e) => {
                ctx.app.push_toast(
                    texts::failed_to_serialize_json(&e.to_string()),
                    ToastKind::Error,
                );
                return Ok(());
            }
        };

        (Some(pretty), texts::common_config_snippet_saved())
    };

    let state = load_state()?;
    {
        let mut cfg = match state.config.write().map_err(AppError::from) {
            Ok(cfg) => cfg,
            Err(err) => {
                ctx.app.push_toast(err.to_string(), ToastKind::Error);
                return Ok(());
            }
        };
        cfg.common_config_snippets
            .set(&app_type, next_snippet.clone());
    }
    if let Err(err) = state.save() {
        ctx.app.push_toast(err.to_string(), ToastKind::Error);
        return Ok(());
    }

    ctx.app.editor = None;
    ctx.app.push_toast(toast, ToastKind::Success);
    *ctx.data = UiData::load(&ctx.app.app_type)?;

    let snippet = next_snippet.unwrap_or_else(|| {
        texts::tui_default_common_snippet_for_app(app_type.as_str()).to_string()
    });
    ctx.app.overlay = Overlay::CommonSnippetView {
        app_type: app_type.clone(),
        view: TextViewState {
            title: texts::tui_common_snippet_title(app_type.as_str()),
            lines: snippet.lines().map(|s| s.to_string()).collect(),
            scroll: 0,
            action: None,
        },
    };
    Ok(())
}

fn submit_webdav_settings(
    ctx: &mut RuntimeActionContext<'_>,
    content: String,
) -> Result<(), AppError> {
    let edited = content.trim();
    if edited.is_empty() {
        set_webdav_sync_settings(None)?;
        ctx.app.editor = None;
        ctx.app.push_toast(
            texts::tui_toast_webdav_settings_cleared(),
            ToastKind::Success,
        );
        *ctx.data = UiData::load(&ctx.app.app_type)?;
        return Ok(());
    }

    let cfg: WebDavSyncSettings = serde_json::from_str(edited)
        .map_err(|e| AppError::Message(texts::tui_toast_invalid_json(&e.to_string())))?;
    set_webdav_sync_settings(Some(cfg))?;

    ctx.app.editor = None;
    ctx.app
        .push_toast(texts::tui_toast_webdav_settings_saved(), ToastKind::Success);
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}
