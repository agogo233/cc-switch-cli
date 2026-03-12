use super::*;

pub(super) fn render_skills_installed(
    frame: &mut Frame<'_>,
    app: &App,
    data: &UiData,
    area: Rect,
    theme: &super::theme::Theme,
) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(pane_border_style(app, Focus::Content, theme))
        .title(texts::skills_management());
    frame.render_widget(outer.clone(), area);
    let inner = outer.inner(area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(inner);

    if app.focus == Focus::Content {
        render_key_bar_center(
            frame,
            chunks[0],
            theme,
            &[
                ("Enter", texts::tui_key_details()),
                ("x", texts::tui_key_toggle()),
                ("m", texts::tui_key_apps()),
                ("f", texts::tui_key_discover()),
                ("i", texts::tui_skills_action_import_existing()),
                ("d", texts::tui_key_uninstall()),
            ],
        );
    }

    render_summary_bar(frame, chunks[1], theme, installed_summary(data));

    let visible = skills_installed_filtered(app, data);
    if visible.is_empty() {
        render_installed_empty_state(frame, chunks[2], theme);
        return;
    }

    let header = Row::new(vec![
        Cell::from(texts::header_name()),
        Cell::from(texts::tui_header_claude_short()),
        Cell::from(texts::tui_header_codex_short()),
        Cell::from(texts::tui_header_gemini_short()),
        Cell::from(texts::tui_header_opencode_short()),
    ])
    .style(Style::default().fg(theme.dim).add_modifier(Modifier::BOLD));

    let rows = visible.iter().map(|skill| {
        Row::new(vec![
            Cell::from(skill_display_name(&skill.name, &skill.directory).to_string()),
            Cell::from(skill_marker(skill.apps.claude)),
            Cell::from(skill_marker(skill.apps.codex)),
            Cell::from(skill_marker(skill.apps.gemini)),
            Cell::from(skill_marker(skill.apps.opencode)),
        ])
    });

    let table = Table::new(
        rows,
        [
            Constraint::Min(10),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::NONE))
    .row_highlight_style(selection_style(theme))
    .highlight_symbol(highlight_symbol(theme));

    let mut state = TableState::default();
    state.select(Some(app.skills_idx));
    frame.render_stateful_widget(table, inset_left(chunks[2], CONTENT_INSET_LEFT), &mut state);
}

fn installed_summary(data: &UiData) -> String {
    let enabled_claude = data
        .skills
        .installed
        .iter()
        .filter(|s| s.apps.claude)
        .count();
    let enabled_codex = data
        .skills
        .installed
        .iter()
        .filter(|s| s.apps.codex)
        .count();
    let enabled_gemini = data
        .skills
        .installed
        .iter()
        .filter(|s| s.apps.gemini)
        .count();
    let enabled_opencode = data
        .skills
        .installed
        .iter()
        .filter(|s| s.apps.opencode)
        .count();

    texts::tui_skills_installed_counts(
        enabled_claude,
        enabled_codex,
        enabled_gemini,
        enabled_opencode,
    )
}

fn render_installed_empty_state(frame: &mut Frame<'_>, area: Rect, theme: &super::theme::Theme) {
    let empty_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(7),
            Constraint::Min(0),
        ])
        .split(area);

    let icon_style = if theme.no_color {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    };

    let empty_lines = vec![
        Line::raw(""),
        Line::from(Span::styled("✦", icon_style)),
        Line::raw(""),
        Line::from(Span::styled(
            texts::tui_skills_empty_title(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            texts::tui_skills_empty_subtitle(),
            Style::default().fg(theme.dim),
        )),
    ];

    frame.render_widget(
        Paragraph::new(empty_lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        empty_chunks[1],
    );
}

fn skill_marker(enabled: bool) -> &'static str {
    if enabled {
        texts::tui_marker_active()
    } else {
        texts::tui_marker_inactive()
    }
}
