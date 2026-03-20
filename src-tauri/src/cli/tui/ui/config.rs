use super::*;
use serde::Serialize;

use crate::cli::tui::app::LocalProxySettingsItem;

pub(super) fn config_items_filtered(app: &App) -> Vec<ConfigItem> {
    let items = ConfigItem::ALL
        .iter()
        .filter(|item| item.visible_for_app(&app.app_type))
        .cloned()
        .collect::<Vec<_>>();

    let Some(q) = app.filter.query_lower() else {
        return items;
    };

    items
        .into_iter()
        .filter(|item| item.label().to_lowercase().contains(&q))
        .collect()
}

pub(super) fn config_item_label(item: &ConfigItem) -> &'static str {
    item.label()
}

pub(super) fn webdav_config_items_filtered(app: &App) -> Vec<WebDavConfigItem> {
    let Some(q) = app.filter.query_lower() else {
        return WebDavConfigItem::ALL.to_vec();
    };
    WebDavConfigItem::ALL
        .iter()
        .cloned()
        .filter(|item| webdav_config_item_label(item).to_lowercase().contains(&q))
        .collect()
}

pub(super) fn webdav_config_item_label(item: &WebDavConfigItem) -> &'static str {
    match item {
        WebDavConfigItem::Settings => texts::tui_config_item_webdav_settings(),
        WebDavConfigItem::CheckConnection => texts::tui_config_item_webdav_check_connection(),
        WebDavConfigItem::Upload => texts::tui_config_item_webdav_upload(),
        WebDavConfigItem::Download => texts::tui_config_item_webdav_download(),
        WebDavConfigItem::Reset => texts::tui_config_item_webdav_reset(),
        WebDavConfigItem::JianguoyunQuickSetup => {
            texts::tui_config_item_webdav_jianguoyun_quick_setup()
        }
    }
}

pub(super) fn local_proxy_settings_item_label(item: &LocalProxySettingsItem) -> &'static str {
    match item {
        LocalProxySettingsItem::ListenAddress => texts::tui_settings_proxy_listen_address_label(),
        LocalProxySettingsItem::ListenPort => texts::tui_settings_proxy_listen_port_label(),
    }
}

pub(super) fn ordered_visible_app_types(apps: &crate::settings::VisibleApps) -> Vec<AppType> {
    apps.ordered_enabled()
}

fn visible_apps_summary(apps: &crate::settings::VisibleApps) -> String {
    let labels = ordered_visible_app_types(apps)
        .into_iter()
        .map(|app_type| app_type.as_str().to_string())
        .collect::<Vec<_>>();

    if labels.is_empty() {
        texts::none().to_string()
    } else {
        labels.join(", ")
    }
}

pub(super) fn render_config(
    frame: &mut Frame<'_>,
    app: &App,
    _data: &UiData,
    area: Rect,
    theme: &super::theme::Theme,
) {
    let items = config_items_filtered(app);
    let rows = items
        .iter()
        .map(|item| Row::new(vec![Cell::from(config_item_label(item))]));

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(pane_border_style(app, Focus::Content, theme))
        .title(texts::tui_config_title());
    frame.render_widget(outer.clone(), area);
    let inner = outer.inner(area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    if app.focus == Focus::Content {
        let mut keys = vec![("Enter", texts::tui_key_select())];
        if matches!(items.get(app.config_idx), Some(ConfigItem::CommonSnippet)) {
            keys.push(("e", texts::tui_key_edit_snippet()));
        }
        render_key_bar_center(frame, chunks[0], theme, &keys);
    }

    let table = Table::new(rows, [Constraint::Min(10)])
        .block(Block::default().borders(Borders::NONE))
        .row_highlight_style(selection_style(theme))
        .highlight_symbol(highlight_symbol(theme));

    let mut state = TableState::default();
    state.select(Some(app.config_idx));
    frame.render_stateful_widget(table, inset_left(chunks[1], CONTENT_INSET_LEFT), &mut state);
}

pub(super) fn render_config_webdav(
    frame: &mut Frame<'_>,
    app: &App,
    _data: &UiData,
    area: Rect,
    theme: &super::theme::Theme,
) {
    let items = webdav_config_items_filtered(app);
    let rows = items
        .iter()
        .map(|item| Row::new(vec![Cell::from(webdav_config_item_label(item))]));

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(pane_border_style(app, Focus::Content, theme))
        .title(texts::tui_config_webdav_title());
    frame.render_widget(outer.clone(), area);
    let inner = outer.inner(area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    if app.focus == Focus::Content {
        let mut keys = vec![("Enter", texts::tui_key_select())];
        if matches!(
            items.get(app.config_webdav_idx),
            Some(WebDavConfigItem::Settings)
        ) {
            keys.push(("e", texts::tui_key_edit()));
        }
        render_key_bar_center(frame, chunks[0], theme, &keys);
    }

    let table = Table::new(rows, [Constraint::Min(10)])
        .block(Block::default().borders(Borders::NONE))
        .row_highlight_style(selection_style(theme))
        .highlight_symbol(highlight_symbol(theme));

    let mut state = TableState::default();
    state.select(Some(app.config_webdav_idx));
    frame.render_stateful_widget(table, inset_left(chunks[1], CONTENT_INSET_LEFT), &mut state);
}

pub(super) fn render_config_openclaw_route(
    frame: &mut Frame<'_>,
    app: &App,
    data: &UiData,
    area: Rect,
    theme: &super::theme::Theme,
) {
    let Some(item) = ConfigItem::from_openclaw_route(&app.route) else {
        return;
    };

    let title = item
        .detail_title()
        .expect("OpenClaw config route should define a title");

    match item {
        ConfigItem::OpenClawEnv => render_openclaw_config_section(
            frame,
            app,
            area,
            theme,
            title,
            "env.",
            data.config.openclaw_env.as_ref(),
            data.config.openclaw_config_path.as_deref(),
            data.config.openclaw_warnings.as_deref(),
        ),
        ConfigItem::OpenClawTools => render_openclaw_config_section(
            frame,
            app,
            area,
            theme,
            title,
            "tools.",
            data.config.openclaw_tools.as_ref(),
            data.config.openclaw_config_path.as_deref(),
            data.config.openclaw_warnings.as_deref(),
        ),
        ConfigItem::OpenClawAgents => render_openclaw_config_section(
            frame,
            app,
            area,
            theme,
            title,
            "agents.defaults",
            data.config.openclaw_agents_defaults.as_ref(),
            data.config.openclaw_config_path.as_deref(),
            data.config.openclaw_warnings.as_deref(),
        ),
        _ => {}
    }
}

fn render_openclaw_config_section<T: Serialize>(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    theme: &super::theme::Theme,
    title: &'static str,
    warning_prefix: &'static str,
    section: Option<&T>,
    config_path: Option<&std::path::Path>,
    warnings: Option<&[crate::openclaw_config::OpenClawHealthWarning]>,
) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(pane_border_style(app, Focus::Content, theme))
        .title(title);
    frame.render_widget(outer.clone(), area);
    let inner = outer.inner(area);

    let config_path_display = config_path.map(|path| path.display().to_string());
    let section_warnings = warnings
        .unwrap_or_default()
        .iter()
        .filter(|warning| {
            openclaw_warning_matches_section(
                warning,
                warning_prefix,
                config_path_display.as_deref(),
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    let has_warnings = !section_warnings.is_empty();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(if has_warnings { 4 } else { 0 }),
            Constraint::Min(0),
        ])
        .split(inner);

    if app.focus == Focus::Content {
        render_key_bar_center(
            frame,
            chunks[0],
            theme,
            &[
                ("Enter", texts::tui_key_edit()),
                ("e", texts::tui_key_edit()),
                ("Esc", texts::tui_key_close()),
            ],
        );
    }

    let section_json = section
        .map(|section| {
            let section_value = serde_json::to_value(section).unwrap_or_else(|_| Value::Null);
            let redacted = redact_sensitive_json(&section_value);
            serde_json::to_string_pretty(&redacted).unwrap_or_else(|_| "{}".to_string())
        })
        .unwrap_or_else(|| "null".to_string());

    if has_warnings {
        let banner = section_warnings
            .iter()
            .map(|warning| match warning.path.as_deref() {
                Some(path) => format!("- {} ({path})", warning.message),
                None => format!("- {}", warning.message),
            })
            .collect::<Vec<_>>()
            .join("\n");
        frame.render_widget(
            Paragraph::new(format!(
                "{}\n{}",
                texts::tui_openclaw_config_warning_title(),
                banner,
            ))
            .style(Style::default().fg(theme.warn))
            .wrap(Wrap { trim: false }),
            inset_left(chunks[1], CONTENT_INSET_LEFT),
        );
    }

    let content = format!(
        "{}\n{}\n\n{}\n{}\n\n{}\n{}",
        texts::tui_openclaw_config_file_label(),
        config_path_display
            .unwrap_or_else(|| texts::tui_openclaw_config_path_not_available().to_string()),
        texts::tui_openclaw_config_section_label(),
        section_json,
        texts::tui_openclaw_config_warning_state_label(),
        if has_warnings {
            texts::tui_openclaw_config_warning_present().to_string()
        } else {
            texts::tui_openclaw_config_warning_clean().to_string()
        }
    );

    frame.render_widget(
        Paragraph::new(content).wrap(Wrap { trim: false }),
        inset_left(chunks[2], CONTENT_INSET_LEFT),
    );
}

fn openclaw_warning_matches_section(
    warning: &crate::openclaw_config::OpenClawHealthWarning,
    warning_prefix: &str,
    config_path: Option<&str>,
) -> bool {
    if warning.code == "config_parse_failed" {
        return true;
    }

    match warning.path.as_deref() {
        None => true,
        Some(path) if config_path == Some(path) => true,
        Some(path) => path.starts_with(warning_prefix),
    }
}

pub(super) fn render_settings(
    frame: &mut Frame<'_>,
    app: &App,
    data: &UiData,
    area: Rect,
    theme: &super::theme::Theme,
) {
    let language = crate::cli::i18n::current_language();
    let visible_apps = crate::settings::get_visible_apps();
    let skip_claude_onboarding = crate::settings::get_skip_claude_onboarding();
    let claude_plugin_integration = crate::settings::get_enable_claude_plugin_integration();

    let rows_data = super::app::SettingsItem::ALL
        .iter()
        .map(|item| match item {
            super::app::SettingsItem::Language => (
                texts::tui_settings_header_language().to_string(),
                language.display_name().to_string(),
            ),
            super::app::SettingsItem::VisibleApps => (
                texts::tui_settings_visible_apps_label().to_string(),
                visible_apps_summary(&visible_apps),
            ),
            super::app::SettingsItem::SkipClaudeOnboarding => (
                texts::skip_claude_onboarding_label().to_string(),
                if skip_claude_onboarding {
                    texts::enabled().to_string()
                } else {
                    texts::disabled().to_string()
                },
            ),
            super::app::SettingsItem::ClaudePluginIntegration => (
                texts::enable_claude_plugin_integration_label().to_string(),
                if claude_plugin_integration {
                    texts::enabled().to_string()
                } else {
                    texts::disabled().to_string()
                },
            ),
            super::app::SettingsItem::Proxy => (
                texts::tui_config_item_proxy().to_string(),
                format!(
                    "{}:{}",
                    data.proxy.configured_listen_address, data.proxy.configured_listen_port,
                ),
            ),
            super::app::SettingsItem::CheckForUpdates => (
                texts::tui_settings_check_for_updates().to_string(),
                format!("v{}", env!("CARGO_PKG_VERSION")),
            ),
        })
        .collect::<Vec<_>>();

    let label_col_width = field_label_column_width(
        rows_data
            .iter()
            .map(|(label, _value)| label.as_str())
            .chain(std::iter::once(texts::tui_settings_header_setting())),
        0,
    );

    let header = Row::new(vec![
        Cell::from(texts::tui_settings_header_setting()),
        Cell::from(texts::tui_settings_header_value()),
    ])
    .style(Style::default().fg(theme.dim).add_modifier(Modifier::BOLD));

    let rows = rows_data
        .iter()
        .map(|(label, value)| Row::new(vec![Cell::from(label.clone()), Cell::from(value.clone())]));

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(pane_border_style(app, Focus::Content, theme))
        .title(texts::menu_settings());
    frame.render_widget(outer.clone(), area);
    let inner = outer.inner(area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    if app.focus == Focus::Content {
        render_key_bar_center(
            frame,
            chunks[0],
            theme,
            &[("Enter", texts::tui_key_apply())],
        );
    }

    let table = Table::new(
        rows,
        [Constraint::Length(label_col_width), Constraint::Min(10)],
    )
    .header(header)
    .block(Block::default().borders(Borders::NONE))
    .row_highlight_style(selection_style(theme))
    .highlight_symbol(highlight_symbol(theme));

    let mut state = TableState::default();
    state.select(Some(app.settings_idx));
    frame.render_stateful_widget(table, inset_left(chunks[1], CONTENT_INSET_LEFT), &mut state);
}

pub(super) fn render_settings_proxy(
    frame: &mut Frame<'_>,
    app: &App,
    data: &UiData,
    area: Rect,
    theme: &super::theme::Theme,
) {
    let rows_data = LocalProxySettingsItem::ALL
        .iter()
        .map(|item| match item {
            LocalProxySettingsItem::ListenAddress => (
                local_proxy_settings_item_label(item).to_string(),
                data.proxy.configured_listen_address.clone(),
            ),
            LocalProxySettingsItem::ListenPort => (
                local_proxy_settings_item_label(item).to_string(),
                data.proxy.configured_listen_port.to_string(),
            ),
        })
        .collect::<Vec<_>>();

    let label_col_width = field_label_column_width(
        rows_data
            .iter()
            .map(|(label, _value)| label.as_str())
            .chain(std::iter::once(texts::tui_settings_header_setting())),
        0,
    );

    let header = Row::new(vec![
        Cell::from(texts::tui_settings_header_setting()),
        Cell::from(texts::tui_settings_header_value()),
    ])
    .style(Style::default().fg(theme.dim).add_modifier(Modifier::BOLD));

    let rows = rows_data
        .iter()
        .map(|(label, value)| Row::new(vec![Cell::from(label.clone()), Cell::from(value.clone())]));

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(pane_border_style(app, Focus::Content, theme))
        .title(texts::tui_settings_proxy_title());
    frame.render_widget(outer.clone(), area);
    let inner = outer.inner(area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner);

    if app.focus == Focus::Content && !data.proxy.running {
        render_key_bar_center(frame, chunks[0], theme, &[("Enter", texts::tui_key_edit())]);
    }

    let table = Table::new(
        rows,
        [Constraint::Length(label_col_width), Constraint::Min(10)],
    )
    .header(header)
    .block(Block::default().borders(Borders::NONE))
    .row_highlight_style(selection_style(theme))
    .highlight_symbol(highlight_symbol(theme));

    let mut state = TableState::default();
    state.select(Some(app.settings_proxy_idx));
    frame.render_stateful_widget(table, inset_left(chunks[1], CONTENT_INSET_LEFT), &mut state);

    frame.render_widget(
        Paragraph::new(if data.proxy.running {
            texts::tui_settings_proxy_stop_before_edit_hint()
        } else {
            texts::tui_settings_proxy_restart_hint()
        })
        .alignment(Alignment::Center)
        .style(Style::default().fg(theme.dim)),
        chunks[2],
    );
}
