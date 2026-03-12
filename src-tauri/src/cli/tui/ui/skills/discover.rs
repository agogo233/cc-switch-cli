use super::*;

pub(super) fn render_skills_discover(
    frame: &mut Frame<'_>,
    app: &App,
    _data: &UiData,
    area: Rect,
    theme: &super::theme::Theme,
) {
    let title = format!(
        "{} — {}",
        texts::tui_skills_discover_title(),
        if app.skills_discover_query.trim().is_empty() {
            texts::tui_skills_discover_query_empty()
        } else {
            app.skills_discover_query.as_str()
        }
    );

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(pane_border_style(app, Focus::Content, theme))
        .title(title);
    frame.render_widget(outer.clone(), area);
    let inner = outer.inner(area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(inner);

    if app.focus == Focus::Content {
        render_key_bar_center(
            frame,
            chunks[0],
            theme,
            &[
                ("Enter", texts::tui_key_install()),
                ("f", texts::tui_key_search()),
                ("r", texts::tui_key_repos()),
            ],
        );
    }

    let query = app.filter.query_lower();
    let visible = app
        .skills_discover_results
        .iter()
        .filter(|skill| match &query {
            None => true,
            Some(q) => {
                skill.name.to_lowercase().contains(q)
                    || skill.directory.to_lowercase().contains(q)
                    || skill.key.to_lowercase().contains(q)
                    || skill.description.to_lowercase().contains(q)
            }
        })
        .collect::<Vec<_>>();

    if visible.is_empty() {
        frame.render_widget(
            Paragraph::new(texts::tui_skills_discover_hint())
                .style(Style::default().fg(theme.dim))
                .wrap(Wrap { trim: false }),
            inset_left(chunks[1], CONTENT_INSET_LEFT),
        );
        return;
    }

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from(texts::header_name()),
        Cell::from(texts::tui_header_repo()),
    ])
    .style(Style::default().fg(theme.dim).add_modifier(Modifier::BOLD));

    let rows = visible.iter().map(|skill| {
        let repo = match (&skill.repo_owner, &skill.repo_name) {
            (Some(owner), Some(name)) => format!("{owner}/{name}"),
            _ => "-".to_string(),
        };
        Row::new(vec![
            Cell::from(if skill.installed {
                texts::tui_marker_active()
            } else {
                texts::tui_marker_inactive()
            }),
            Cell::from(skill_display_name(&skill.name, &skill.directory).to_string()),
            Cell::from(repo),
        ])
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Percentage(70),
            Constraint::Percentage(30),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::NONE))
    .row_highlight_style(selection_style(theme))
    .highlight_symbol(highlight_symbol(theme));

    let mut state = TableState::default();
    state.select(Some(app.skills_discover_idx));
    frame.render_stateful_widget(table, inset_left(chunks[1], CONTENT_INSET_LEFT), &mut state);
}
