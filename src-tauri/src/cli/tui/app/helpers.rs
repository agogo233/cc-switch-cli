use super::*;

pub(crate) fn route_has_content_list(route: &Route) -> bool {
    matches!(
        route,
        Route::Providers
            | Route::ProviderDetail { .. }
            | Route::Mcp
            | Route::Prompts
            | Route::Config
            | Route::ConfigOpenClawEnv
            | Route::ConfigOpenClawTools
            | Route::ConfigOpenClawAgents
            | Route::ConfigWebDav
            | Route::Skills
            | Route::SkillsDiscover
            | Route::SkillsRepos
            | Route::SkillDetail { .. }
            | Route::Settings
            | Route::SettingsProxy
    )
}

pub(crate) fn route_default_focus(route: &Route) -> Focus {
    match route {
        Route::Main => Focus::Nav,
        _ => Focus::Content,
    }
}

pub(crate) fn visible_providers<'a>(
    app_type: &AppType,
    filter: &FilterState,
    data: &'a UiData,
) -> Vec<&'a super::data::ProviderRow> {
    let query = filter.query_lower();
    data.providers
        .rows
        .iter()
        .filter(|row| match &query {
            None => true,
            Some(q) => {
                super::data::provider_display_name(app_type, row)
                    .to_lowercase()
                    .contains(q)
                    || row.provider.name.to_lowercase().contains(q)
                    || row.id.to_lowercase().contains(q)
            }
        })
        .collect()
}

pub(crate) fn supports_provider_stream_check(app_type: &AppType) -> bool {
    !matches!(app_type, AppType::OpenClaw)
}

pub(crate) fn visible_mcp<'a>(
    filter: &FilterState,
    data: &'a UiData,
) -> Vec<&'a super::data::McpRow> {
    let query = filter.query_lower();
    data.mcp
        .rows
        .iter()
        .filter(|row| match &query {
            None => true,
            Some(q) => {
                row.server.name.to_lowercase().contains(q) || row.id.to_lowercase().contains(q)
            }
        })
        .collect()
}

pub(crate) fn visible_prompts<'a>(
    filter: &FilterState,
    data: &'a UiData,
) -> Vec<&'a super::data::PromptRow> {
    let query = filter.query_lower();
    data.prompts
        .rows
        .iter()
        .filter(|row| match &query {
            None => true,
            Some(q) => {
                row.prompt.name.to_lowercase().contains(q) || row.id.to_lowercase().contains(q)
            }
        })
        .collect()
}

pub(crate) fn visible_skills_installed<'a>(
    filter: &FilterState,
    data: &'a UiData,
) -> Vec<&'a crate::services::skill::InstalledSkill> {
    let query = filter.query_lower();
    data.skills
        .installed
        .iter()
        .filter(|skill| match &query {
            None => true,
            Some(q) => {
                skill.name.to_lowercase().contains(q)
                    || skill.directory.to_lowercase().contains(q)
                    || skill.id.to_lowercase().contains(q)
            }
        })
        .collect()
}

pub(crate) fn visible_skills_discover<'a>(
    filter: &FilterState,
    skills: &'a [crate::services::skill::Skill],
) -> Vec<&'a crate::services::skill::Skill> {
    let query = filter.query_lower();
    skills
        .iter()
        .filter(|skill| match &query {
            None => true,
            Some(q) => {
                skill.name.to_lowercase().contains(q)
                    || skill.directory.to_lowercase().contains(q)
                    || skill.key.to_lowercase().contains(q)
            }
        })
        .collect()
}

pub(crate) fn visible_skills_repos<'a>(
    filter: &FilterState,
    data: &'a UiData,
) -> Vec<&'a crate::services::skill::SkillRepo> {
    let query = filter.query_lower();
    data.skills
        .repos
        .iter()
        .filter(|repo| match &query {
            None => true,
            Some(q) => {
                repo.owner.to_lowercase().contains(q)
                    || repo.name.to_lowercase().contains(q)
                    || repo.branch.to_lowercase().contains(q)
            }
        })
        .collect()
}

pub(crate) fn visible_skills_unmanaged<'a>(
    filter: &FilterState,
    skills: &'a [crate::services::skill::UnmanagedSkill],
) -> Vec<&'a crate::services::skill::UnmanagedSkill> {
    let query = filter.query_lower();
    skills
        .iter()
        .filter(|skill| match &query {
            None => true,
            Some(q) => {
                skill.name.to_lowercase().contains(q)
                    || skill.directory.to_lowercase().contains(q)
                    || skill
                        .description
                        .as_deref()
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains(q)
                    || skill.found_in.iter().any(|s| s.to_lowercase().contains(q))
            }
        })
        .collect()
}

pub(crate) fn visible_config_items(filter: &FilterState, app_type: &AppType) -> Vec<ConfigItem> {
    let all = ConfigItem::ALL
        .iter()
        .filter(|item| item.visible_for_app(app_type))
        .cloned()
        .collect::<Vec<_>>();
    let Some(q) = filter.query_lower() else {
        return all;
    };

    all.into_iter()
        .filter(|item| item.label().to_lowercase().contains(&q))
        .collect()
}

pub(crate) fn config_item_label(item: &ConfigItem) -> &'static str {
    item.label()
}

pub(crate) fn visible_webdav_config_items(filter: &FilterState) -> Vec<WebDavConfigItem> {
    let all = WebDavConfigItem::ALL.to_vec();
    let Some(q) = filter.query_lower() else {
        return all;
    };

    all.into_iter()
        .filter(|item| webdav_config_item_label(item).to_lowercase().contains(&q))
        .collect()
}

pub(crate) fn webdav_config_item_label(item: &WebDavConfigItem) -> &'static str {
    match item {
        WebDavConfigItem::Settings => crate::cli::i18n::texts::tui_config_item_webdav_settings(),
        WebDavConfigItem::CheckConnection => {
            crate::cli::i18n::texts::tui_config_item_webdav_check_connection()
        }
        WebDavConfigItem::Upload => crate::cli::i18n::texts::tui_config_item_webdav_upload(),
        WebDavConfigItem::Download => crate::cli::i18n::texts::tui_config_item_webdav_download(),
        WebDavConfigItem::Reset => crate::cli::i18n::texts::tui_config_item_webdav_reset(),
        WebDavConfigItem::JianguoyunQuickSetup => {
            crate::cli::i18n::texts::tui_config_item_webdav_jianguoyun_quick_setup()
        }
    }
}

pub(crate) fn cycle_app_type(current: &AppType, dir: i8) -> Option<AppType> {
    let visible_apps = crate::settings::get_visible_apps();
    if visible_apps.ordered_enabled().len() <= 1 {
        return None;
    }

    crate::settings::next_visible_app(&visible_apps, current, dir).filter(|next| next != current)
}

pub(crate) fn app_type_picker_index(app_type: &AppType) -> usize {
    match app_type {
        AppType::Claude => 0,
        AppType::Codex => 1,
        AppType::Gemini => 2,
        AppType::OpenCode => 3,
        AppType::OpenClaw => 4,
    }
}

pub(crate) fn four_app_picker_index(app_type: &AppType) -> usize {
    app_type_picker_index(app_type).min(3)
}

pub(crate) fn app_type_for_picker_index(index: usize) -> AppType {
    match index {
        1 => AppType::Codex,
        2 => AppType::Gemini,
        3 => AppType::OpenCode,
        4 => AppType::OpenClaw,
        _ => AppType::Claude,
    }
}

pub(crate) fn snippet_picker_index_for_app_type(app_type: &AppType) -> usize {
    app_type_picker_index(app_type)
}

pub(crate) fn snippet_picker_app_type(index: usize) -> AppType {
    app_type_for_picker_index(index)
}

pub(crate) fn sync_method_picker_index(method: SyncMethod) -> usize {
    match method {
        SyncMethod::Auto => 0,
        SyncMethod::Symlink => 1,
        SyncMethod::Copy => 2,
    }
}

pub(crate) fn sync_method_for_picker_index(index: usize) -> SyncMethod {
    match index {
        1 => SyncMethod::Symlink,
        2 => SyncMethod::Copy,
        _ => SyncMethod::Auto,
    }
}

pub(crate) fn is_save_shortcut(key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('s' | 'S') => key.modifiers.contains(KeyModifiers::CONTROL),
        KeyCode::Char('\u{13}') => true,
        _ => false,
    }
}

pub(crate) fn is_open_external_editor_shortcut(key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('o' | 'O') => key.modifiers.contains(KeyModifiers::CONTROL),
        _ => false,
    }
}
