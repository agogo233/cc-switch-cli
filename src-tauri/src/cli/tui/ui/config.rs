use super::*;

pub(super) fn config_items_filtered(app: &App) -> Vec<ConfigItem> {
    let Some(q) = app.filter.query_lower() else {
        return ConfigItem::ALL.to_vec();
    };
    ConfigItem::ALL
        .iter()
        .cloned()
        .filter(|item| config_item_label(item).to_lowercase().contains(&q))
        .collect()
}

pub(super) fn config_item_label(item: &ConfigItem) -> &'static str {
    match item {
        ConfigItem::Path => texts::tui_config_item_show_path(),
        ConfigItem::ShowFull => texts::tui_config_item_show_full(),
        ConfigItem::Export => texts::tui_config_item_export(),
        ConfigItem::Import => texts::tui_config_item_import(),
        ConfigItem::Backup => texts::tui_config_item_backup(),
        ConfigItem::Restore => texts::tui_config_item_restore(),
        ConfigItem::Validate => texts::tui_config_item_validate(),
        ConfigItem::CommonSnippet => texts::tui_config_item_common_snippet(),
        ConfigItem::Proxy => texts::tui_config_item_proxy(),
        ConfigItem::WebDavSync => texts::tui_config_item_webdav_sync(),
        ConfigItem::Reset => texts::tui_config_item_reset(),
    }
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

pub(super) fn render_settings(
    frame: &mut Frame<'_>,
    app: &App,
    data: &UiData,
    area: Rect,
    theme: &super::theme::Theme,
) {
    let language = crate::cli::i18n::current_language();
    let skip_claude_onboarding = crate::settings::get_skip_claude_onboarding();
    let claude_plugin_integration = crate::settings::get_enable_claude_plugin_integration();
    let proxy_enabled = data.proxy.enabled;

    let rows_data = super::app::SettingsItem::ALL
        .iter()
        .map(|item| match item {
            super::app::SettingsItem::Language => (
                texts::tui_settings_header_language().to_string(),
                language.display_name().to_string(),
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
                if proxy_enabled {
                    texts::enabled().to_string()
                } else {
                    texts::disabled().to_string()
                },
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
