use super::*;

pub(super) fn render_main(
    frame: &mut Frame<'_>,
    app: &App,
    data: &UiData,
    area: Rect,
    theme: &super::theme::Theme,
) {
    let current_provider = data
        .providers
        .rows
        .iter()
        .find(|p| p.is_current)
        .map(|p| p.provider.name.as_str())
        .unwrap_or(texts::none());

    let mcp_enabled = data
        .mcp
        .rows
        .iter()
        .filter(|s| s.server.apps.is_enabled_for(&app.app_type))
        .count();
    let skills_enabled = data
        .skills
        .installed
        .iter()
        .filter(|skill| skill.apps.is_enabled_for(&app.app_type))
        .count();

    let api_url = data
        .providers
        .rows
        .iter()
        .find(|p| p.is_current)
        .and_then(|p| p.api_url.as_deref())
        .unwrap_or(texts::tui_na());

    let label_width = 14;
    let value_style = Style::default().fg(theme.cyan);
    let provider_name_style = if theme.no_color {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    };

    let proxy_running = data.proxy.running;
    let app_supports_proxy_control = data.proxy.takeover_enabled_for(&app.app_type).is_some();
    let current_app_routed = data
        .proxy
        .routes_current_app_through_proxy(&app.app_type)
        .unwrap_or(false);
    let hero_heading = proxy_hero_heading(&app.app_type, &data.proxy);
    let proxy_badge = proxy_status_badge(&app.app_type, &data.proxy);
    let proxy_badge_style = if theme.no_color {
        Style::default().add_modifier(Modifier::BOLD)
    } else if current_app_routed {
        Style::default()
            .fg(Color::Black)
            .bg(theme.ok)
            .add_modifier(Modifier::BOLD)
    } else if app_supports_proxy_control {
        Style::default()
            .fg(Color::Black)
            .bg(theme.warn)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Black)
            .bg(theme.surface)
            .add_modifier(Modifier::BOLD)
    };
    let uptime_text = if proxy_running {
        format_uptime_compact(data.proxy.uptime_seconds)
    } else {
        texts::tui_proxy_dashboard_uptime_stopped().to_string()
    };
    let request_text = match (data.proxy.total_requests, data.proxy.success_rate) {
        (0, _) => texts::tui_proxy_dashboard_requests_idle().to_string(),
        (total, Some(rate)) => texts::tui_proxy_dashboard_request_summary(total, rate),
        (total, None) => total.to_string(),
    };
    let proxy_last_error_text = data
        .proxy
        .last_error
        .clone()
        .unwrap_or_else(|| texts::none().to_string());
    let active_target = if current_app_routed {
        data.proxy
            .current_app_target
            .as_ref()
            .map(|target| target.provider_name.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| texts::tui_proxy_dashboard_target_waiting().to_string())
    } else {
        String::new()
    };

    let connection_lines = vec![
        kv_line(
            theme,
            texts::provider_label(),
            label_width,
            vec![
                Span::styled(current_provider.to_string(), provider_name_style),
                Span::raw("   "),
                Span::styled(
                    format!("{} ", texts::tui_label_mcp_short()),
                    Style::default()
                        .fg(theme.comment)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "[{}/{} {}]",
                        mcp_enabled,
                        data.mcp.rows.len(),
                        texts::tui_label_mcp_servers_active()
                    ),
                    value_style,
                ),
                Span::raw("   "),
                Span::styled(
                    format!("{} ", texts::tui_label_skills()),
                    Style::default()
                        .fg(theme.comment)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "[{}/{} {}]",
                        skills_enabled,
                        data.skills.installed.len(),
                        texts::tui_label_mcp_servers_active()
                    ),
                    if data.skills.installed.is_empty() {
                        Style::default().fg(theme.surface)
                    } else {
                        value_style
                    },
                ),
            ],
        ),
        kv_line(
            theme,
            texts::tui_label_api_url(),
            label_width,
            vec![Span::styled(api_url.to_string(), value_style)],
        ),
    ];

    let webdav = data.config.webdav_sync.as_ref();
    let is_config_value_set = |value: &str| !value.trim().is_empty();
    let webdav_enabled = webdav.map(|cfg| cfg.enabled).unwrap_or(false);
    let is_configured = webdav
        .map(|cfg| {
            is_config_value_set(&cfg.base_url)
                && is_config_value_set(&cfg.username)
                && is_config_value_set(&cfg.password)
        })
        .unwrap_or(false);
    let webdav_status = webdav.map(|cfg| &cfg.status);
    let last_error = webdav_status
        .and_then(|status| status.last_error.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let has_error = webdav_enabled && is_configured && last_error.is_some();
    let is_ok = webdav_enabled
        && is_configured
        && !has_error
        && webdav_status
            .and_then(|status| status.last_sync_at)
            .is_some();

    let webdav_status_text = if !webdav_enabled || !is_configured {
        texts::tui_webdav_status_not_configured().to_string()
    } else if has_error {
        let detail = last_error
            .map(|err| truncate_to_display_width(err, 22))
            .unwrap_or_default();
        if detail.is_empty() {
            texts::tui_webdav_status_error().to_string()
        } else {
            texts::tui_webdav_status_error_with_detail(&detail)
        }
    } else if is_ok {
        texts::tui_webdav_status_ok().to_string()
    } else {
        texts::tui_webdav_status_configured().to_string()
    };

    let webdav_status_style = if theme.no_color {
        Style::default()
    } else if has_error {
        Style::default().fg(theme.warn)
    } else if is_ok {
        Style::default().fg(theme.ok)
    } else {
        Style::default().fg(theme.surface)
    };

    let last_sync_at = webdav_status.and_then(|status| status.last_sync_at);
    let webdav_last_sync_text = last_sync_at
        .and_then(format_sync_time_local_to_minute)
        .unwrap_or_else(|| texts::tui_webdav_status_never_synced().to_string());
    let webdav_last_sync_style = if last_sync_at.is_some() {
        value_style
    } else {
        Style::default().fg(theme.surface)
    };

    let webdav_lines = vec![
        kv_line(
            theme,
            texts::tui_label_webdav_status(),
            label_width,
            vec![Span::styled(
                webdav_status_text.clone(),
                webdav_status_style,
            )],
        ),
        kv_line(
            theme,
            texts::tui_label_webdav_last_sync(),
            label_width,
            vec![Span::styled(
                webdav_last_sync_text.clone(),
                webdav_last_sync_style,
            )],
        ),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(pane_border_style(app, Focus::Content, theme))
        .title(texts::welcome_title());
    frame.render_widget(block.clone(), area);

    let inner = block.inner(area);
    let content = inset_left(inner, CONTENT_INSET_LEFT);
    let bottom_hero_height = if current_app_routed { 8 } else { 7 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(bottom_hero_height)])
        .split(content);

    let top_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(6),
            Constraint::Min(0),
        ])
        .split(chunks[0]);

    let card_border = Style::default().fg(theme.dim);
    render_connection_card(frame, top_chunks[0], theme, &connection_lines, card_border);
    render_webdav_card(frame, top_chunks[1], theme, &webdav_lines, card_border);
    render_local_env_check_card(frame, app, top_chunks[2], theme, card_border);

    let hero_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(chunks[1].height.saturating_sub(1)),
            Constraint::Length(1),
        ])
        .split(chunks[1]);

    if current_app_routed {
        render_proxy_activity_dashboard(
            frame,
            hero_chunks[0],
            theme,
            proxy_badge,
            proxy_badge_style,
            &app.proxy_activity_samples,
            &hero_heading,
            &request_text,
            &uptime_text,
            &active_target,
            &proxy_last_error_text,
            data.proxy.last_error.is_some(),
            &format!("{}:{}", data.proxy.listen_address, data.proxy.listen_port),
            data.proxy.default_cost_multiplier.as_deref(),
            data.proxy.total_requests,
        );
    } else {
        render_logo_hero(frame, hero_chunks[0], theme);
    }

    frame.render_widget(
        Paragraph::new(Line::raw(texts::tui_main_hint()))
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(theme.surface)
                    .add_modifier(Modifier::ITALIC),
            ),
        hero_chunks[1],
    );
}

fn render_proxy_activity_dashboard(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &super::theme::Theme,
    proxy_badge: &str,
    proxy_badge_style: Style,
    activity_samples: &[u64],
    hero_heading: &str,
    request_text: &str,
    uptime_text: &str,
    active_target: &str,
    proxy_last_error_text: &str,
    has_proxy_error: bool,
    listen_text: &str,
    multiplier: Option<&str>,
    total_requests: u64,
) -> Rect {
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(theme.accent))
        .title(format!(" {} ", texts::tui_home_section_proxy()));
    frame.render_widget(outer.clone(), area);

    let inner = outer.inner(area);
    let wave_width = inner.width.saturating_sub(2);
    let wave = proxy_activity_wave(wave_width, true, activity_samples);
    let wave_style = Style::default().fg(theme.accent);
    let hero_style = if theme.no_color {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    };
    let badge = format!("[{proxy_badge}]");
    let mut status_spans = vec![
        Span::styled(texts::tui_home_section_proxy(), hero_style),
        Span::raw("  "),
        Span::styled(badge, proxy_badge_style),
        Span::raw(" "),
        Span::styled(hero_heading.to_string(), hero_style),
    ];
    status_spans.push(Span::raw("   "));
    status_spans.extend(proxy_rate_spans(theme, multiplier));
    let status_line = Line::from(status_spans);

    let request_style = if total_requests > 0 {
        Style::default().fg(theme.cyan)
    } else {
        Style::default().fg(theme.surface)
    };
    let target_style = if active_target == texts::tui_proxy_dashboard_target_waiting() {
        Style::default().fg(theme.surface)
    } else {
        Style::default().fg(theme.cyan)
    };

    let summary_line = Line::from(vec![
        Span::styled(
            format!("{}: ", texts::tui_label_requests()),
            Style::default()
                .fg(theme.comment)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(request_text.to_string(), request_style),
        Span::raw("   "),
        Span::styled(
            format!("{}: ", texts::tui_label_listen()),
            Style::default()
                .fg(theme.comment)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(listen_text.to_string(), Style::default().fg(theme.cyan)),
    ]);

    let detail_line = Line::from(vec![
        Span::styled(
            format!("{}: ", texts::tui_label_uptime()),
            Style::default()
                .fg(theme.comment)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(uptime_text.to_string(), Style::default().fg(theme.cyan)),
        Span::raw("   "),
        Span::styled(
            format!("{}: ", texts::tui_label_active_target()),
            Style::default()
                .fg(theme.comment)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(active_target.to_string(), target_style),
    ]);

    let mut lines = vec![
        status_line,
        Line::from(vec![Span::raw(" "), Span::styled(wave, wave_style)]),
        summary_line,
        detail_line,
    ];

    if has_proxy_error {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}: ", texts::tui_label_last_proxy_error()),
                Style::default()
                    .fg(theme.comment)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                proxy_last_error_text.to_string(),
                Style::default().fg(theme.warn),
            ),
        ]));
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    inner
}

fn render_logo_hero(frame: &mut Frame<'_>, area: Rect, theme: &super::theme::Theme) {
    let logo_lines = logo_hero_lines(theme);
    let logo_height = (logo_lines.len() as u16).min(area.height);
    let logo_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(logo_height),
            Constraint::Min(0),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(logo_lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        logo_chunks[1],
    );
}

fn logo_hero_lines(theme: &super::theme::Theme) -> Vec<Line<'static>> {
    let logo_style = Style::default().fg(theme.surface);
    texts::tui_home_ascii_logo()
        .lines()
        .map(|s| Line::from(Span::styled(s.to_string(), logo_style)))
        .collect::<Vec<_>>()
}

fn render_connection_card(
    frame: &mut Frame<'_>,
    area: Rect,
    _theme: &super::theme::Theme,
    connection_lines: &[Line<'_>],
    card_border: Style,
) {
    frame.render_widget(
        Paragraph::new(connection_lines.to_vec())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Plain)
                    .border_style(card_border)
                    .title(format!(" {} ", texts::tui_home_section_connection())),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_webdav_card(
    frame: &mut Frame<'_>,
    area: Rect,
    _theme: &super::theme::Theme,
    webdav_lines: &[Line<'_>],
    card_border: Style,
) {
    frame.render_widget(
        Paragraph::new(webdav_lines.to_vec())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Plain)
                    .border_style(card_border)
                    .title(format!(" {} ", texts::tui_home_section_webdav())),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_local_env_check_card(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    theme: &super::theme::Theme,
    card_border: Style,
) {
    use crate::services::local_env_check::{LocalTool, ToolCheckStatus};

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(card_border)
        .title(format!(" {} ", texts::tui_home_section_local_env_check()));
    frame.render_widget(outer.clone(), area);
    let inner = outer.inner(area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(2)])
        .split(inner);

    let cols0 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);
    let cols1 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);

    let cells = [
        (LocalTool::Claude, "Claude", cols0[0]),
        (LocalTool::Codex, "Codex", cols0[1]),
        (LocalTool::Gemini, "Gemini", cols1[0]),
        (LocalTool::OpenCode, "OpenCode", cols1[1]),
    ];

    for (tool, display_name, cell_area) in cells {
        let status = if app.local_env_loading {
            None
        } else {
            app.local_env_results
                .iter()
                .find(|r| r.tool == tool)
                .map(|r| &r.status)
        };

        let (icon, icon_style) = if app.local_env_loading {
            ("…", Style::default().fg(theme.surface))
        } else {
            match status {
                Some(ToolCheckStatus::Ok { .. }) => (
                    "✓",
                    if theme.no_color {
                        Style::default()
                    } else {
                        Style::default().fg(theme.ok)
                    },
                ),
                Some(ToolCheckStatus::NotInstalledOrNotExecutable) | None => (
                    "!",
                    if theme.no_color {
                        Style::default()
                    } else {
                        Style::default().fg(theme.warn)
                    },
                ),
                Some(ToolCheckStatus::Error { .. }) => (
                    "!",
                    if theme.no_color {
                        Style::default()
                    } else {
                        Style::default().fg(theme.warn)
                    },
                ),
            }
        };

        let name_style = if theme.no_color {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        };

        let detail_style = if theme.no_color {
            Style::default()
        } else {
            Style::default().fg(theme.surface)
        };

        let value_style = Style::default().fg(theme.cyan);
        let (detail_text, detail_line_style) = if app.local_env_loading {
            ("".to_string(), detail_style)
        } else {
            match status {
                Some(ToolCheckStatus::Ok { version }) => (version.clone(), value_style),
                Some(ToolCheckStatus::NotInstalledOrNotExecutable) | None => (
                    texts::tui_local_env_not_installed().to_string(),
                    detail_style,
                ),
                Some(ToolCheckStatus::Error { message }) => (message.clone(), detail_style),
            }
        };

        let detail_width = cell_area.width.saturating_sub(1);
        let detail_text = truncate_to_display_width(&detail_text, detail_width);

        let lines = vec![
            Line::from(vec![
                Span::raw(" "),
                Span::styled(">_ ", Style::default().fg(theme.surface)),
                Span::styled(display_name.to_string(), name_style),
                Span::raw(" "),
                Span::styled(icon.to_string(), icon_style),
            ]),
            Line::from(vec![
                Span::raw(" "),
                Span::styled(detail_text, detail_line_style),
            ]),
        ];

        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), cell_area);
    }
}

pub(super) fn proxy_activity_wave(width: u16, current_app_routed: bool, samples: &[u64]) -> String {
    const BARS: [&str; 8] = ["·", "▁", "▂", "▃", "▄", "▅", "▆", "█"];

    let width = width.max(1) as usize;
    if !current_app_routed {
        return BARS[1].repeat(width);
    }

    let recent = if samples.len() > width {
        &samples[samples.len() - width..]
    } else {
        samples
    };
    let max_delta = recent.iter().copied().max().unwrap_or(0);

    let mut out = String::with_capacity(width * 3);
    for _ in 0..width.saturating_sub(recent.len()) {
        out.push_str(BARS[1]);
    }

    for delta in recent {
        let level = if max_delta == 0 {
            1
        } else {
            let scaled = 1 + ((*delta * 6) / max_delta) as usize;
            scaled.clamp(1, 7)
        };
        out.push_str(BARS[level]);
    }

    out
}
