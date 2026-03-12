use super::*;

pub(super) fn pane_border_style(app: &App, pane: Focus, theme: &super::theme::Theme) -> Style {
    if app.focus == pane {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.dim)
    }
}

pub(super) fn selection_style(theme: &super::theme::Theme) -> Style {
    if theme.no_color {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
            .fg(Color::Black)
            .bg(theme.accent)
            .add_modifier(Modifier::BOLD)
    }
}

pub(super) fn inactive_chip_style(theme: &super::theme::Theme) -> Style {
    if theme.no_color {
        Style::default()
    } else {
        Style::default().fg(Color::White).bg(theme.surface)
    }
}

pub(super) fn active_chip_style(theme: &super::theme::Theme) -> Style {
    if theme.no_color {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
            .fg(Color::Black)
            .bg(theme.accent)
            .add_modifier(Modifier::BOLD)
    }
}

/// Border style for overlay dialogs.
/// `attention = true` for overlays that require user action (Confirm, Update prompts).
/// `attention = false` for informational overlays (Help, TextView, pickers).
pub(super) fn overlay_border_style(theme: &super::theme::Theme, attention: bool) -> Style {
    if attention {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.dim)
    }
}

pub(super) fn transient_feedback_color(theme: &super::theme::Theme, kind: &ToastKind) -> Color {
    match kind {
        ToastKind::Info | ToastKind::Success => theme.accent,
        ToastKind::Warning => theme.warn,
        ToastKind::Error => theme.err,
    }
}

/// Left-pad a cell value with one space for visual inset inside table rows.
pub(super) fn cell_pad(s: &str) -> String {
    format!(" {s}")
}

pub(super) fn strip_trailing_colon(label: &str) -> &str {
    label.trim_end_matches([':', '：'])
}

pub(super) fn pad_to_display_width(label: &str, width: usize) -> String {
    let clean = strip_trailing_colon(label);
    let w = UnicodeWidthStr::width(clean);
    if w >= width {
        clean.to_string()
    } else {
        format!("{clean}{}", " ".repeat(width - w))
    }
}

pub(super) fn truncate_to_display_width(text: &str, width: u16) -> String {
    let width = width as usize;
    if width == 0 {
        return String::new();
    }

    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }

    if width == 1 {
        return "…".to_string();
    }

    let mut out = String::new();
    let mut used = 0usize;
    for c in text.chars() {
        let w = UnicodeWidthChar::width(c).unwrap_or(0);
        if used.saturating_add(w) > width.saturating_sub(1) {
            break;
        }
        out.push(c);
        used = used.saturating_add(w);
    }
    out.push('…');
    out
}

pub(super) fn format_sync_time_local_to_minute(ts: i64) -> Option<String> {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%Y/%m/%d %H:%M").to_string())
}

pub(super) fn app_display_name(app_type: &AppType) -> &'static str {
    match app_type {
        AppType::Claude => "Claude",
        AppType::Codex => "Codex",
        AppType::Gemini => "Gemini",
        AppType::OpenCode => "OpenCode",
    }
}

pub(super) fn format_uptime_compact(total_seconds: u64) -> String {
    let days = total_seconds / 86_400;
    let hours = (total_seconds % 86_400) / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    if seconds > 0 || parts.is_empty() {
        parts.push(format!("{seconds}s"));
    }

    parts.join(" ")
}

pub(super) fn format_proxy_rate_badge(value: Option<&str>) -> String {
    let raw = value.unwrap_or("1").trim();
    if raw.is_empty() {
        return "x1.00".to_string();
    }

    match raw.parse::<f64>() {
        Ok(parsed) => format!("x{parsed:.2}"),
        Err(_) => format!("x{raw}"),
    }
}

pub(super) fn proxy_rate_spans(
    theme: &super::theme::Theme,
    multiplier: Option<&str>,
) -> Vec<Span<'static>> {
    let badge = format_proxy_rate_badge(multiplier);
    let value_style = if theme.no_color {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    };

    vec![Span::styled(badge, value_style)]
}

pub(super) fn proxy_hero_heading(
    app_type: &AppType,
    proxy: &crate::cli::tui::data::ProxySnapshot,
) -> String {
    let app_name = app_display_name(app_type);
    if proxy.takeover_enabled_for(app_type).is_none() {
        texts::tui_proxy_dashboard_unsupported_app(app_name)
    } else if proxy.routes_current_app_through_proxy(app_type) == Some(true) {
        if let Some(target) = proxy
            .current_app_target
            .as_ref()
            .map(|target| target.provider_name.trim())
            .filter(|name| !name.is_empty())
        {
            format!(
                "{} -> {target}",
                texts::tui_proxy_dashboard_current_app_on(app_name)
            )
        } else {
            texts::tui_proxy_dashboard_current_app_on(app_name)
        }
    } else {
        texts::tui_proxy_dashboard_current_app_off(app_name)
    }
}

pub(super) fn proxy_status_badge(
    app_type: &AppType,
    proxy: &crate::cli::tui::data::ProxySnapshot,
) -> &'static str {
    if proxy.takeover_enabled_for(app_type).is_none() {
        texts::tui_proxy_dashboard_status_unsupported()
    } else if proxy.routes_current_app_through_proxy(app_type) == Some(true) {
        texts::tui_proxy_dashboard_status_running()
    } else if proxy.takeover_enabled_for(app_type).is_some() {
        texts::tui_proxy_dashboard_status_stopped()
    } else {
        texts::tui_proxy_dashboard_status_local_only()
    }
}

pub(super) fn kv_line<'a>(
    theme: &super::theme::Theme,
    label: &'a str,
    label_width: usize,
    value_spans: Vec<Span<'a>>,
) -> Line<'a> {
    let mut spans = vec![
        Span::raw(" "), // internal padding: keep content away from │
        Span::styled(
            pad_to_display_width(label, label_width),
            Style::default()
                .fg(theme.comment)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(": "),
    ];
    spans.extend(value_spans);
    Line::from(spans)
}

pub(super) fn highlight_symbol(theme: &super::theme::Theme) -> &'static str {
    if theme.no_color {
        texts::tui_highlight_symbol()
    } else {
        ""
    }
}

pub(super) const CONTENT_INSET_LEFT: u16 = 1;

// Overlay size tiers — percentage-based (large content)
pub(super) const OVERLAY_LG: (u16, u16) = (90, 90);
pub(super) const OVERLAY_MD: (u16, u16) = (78, 62);
// Overlay size tiers — fixed character dimensions (dialogs)
pub(super) const OVERLAY_FIXED_LG: (u16, u16) = (70, 20);
pub(super) const OVERLAY_FIXED_MD: (u16, u16) = (60, 9);
pub(super) const OVERLAY_FIXED_SM: (u16, u16) = (50, 6);
pub(super) const TOAST_MIN_WIDTH: u16 = 28;
pub(super) const TOAST_MAX_WIDTH: u16 = 72;
pub(super) const TOAST_MIN_HEIGHT: u16 = 5;

pub(super) fn key_bar_line(theme: &super::theme::Theme, items: &[(&str, &str)]) -> Line<'static> {
    if theme.no_color {
        let mut parts = Vec::new();
        for (k, v) in items {
            parts.push(format!("{k}={v}"));
        }
        return Line::raw(parts.join("  "));
    }

    let base = inactive_chip_style(theme);
    let key = base.add_modifier(Modifier::BOLD);

    let mut spans: Vec<Span<'static>> = vec![Span::styled(" ", base)];
    for (idx, (k, v)) in items.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled("  ", base));
        }
        spans.push(Span::styled((*k).to_string(), key));
        spans.push(Span::styled(" ", base));
        spans.push(Span::styled((*v).to_string(), base));
    }
    spans.push(Span::styled(" ", base));
    Line::from(spans)
}

/// Render a left-aligned key bar. Used for main-screen footers where keys
/// are read left-to-right in priority order.
pub(super) fn render_key_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &super::theme::Theme,
    items: &[(&str, &str)],
) {
    frame.render_widget(
        Paragraph::new(key_bar_line(theme, items))
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// Render a center-aligned key bar. Used inside overlay dialogs where the
/// available actions are few and visually centered looks balanced.
pub(super) fn render_key_bar_center(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &super::theme::Theme,
    items: &[(&str, &str)],
) {
    frame.render_widget(
        Paragraph::new(key_bar_line(theme, items))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        area,
    );
}

pub(super) fn render_summary_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &super::theme::Theme,
    summary: String,
) {
    let summary_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(theme.dim));
    frame.render_widget(
        Paragraph::new(Line::raw(format!("  {summary}")))
            .style(Style::default().fg(theme.dim))
            .wrap(Wrap { trim: false })
            .block(summary_block),
        area,
    );
}

pub(super) fn inset_left(area: Rect, left: u16) -> Rect {
    if area.width <= left {
        return Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height,
        };
    }
    Rect {
        x: area.x + left,
        y: area.y,
        width: area.width - left,
        height: area.height,
    }
}

pub(super) fn inset_top(area: Rect, top: u16) -> Rect {
    if area.height <= top {
        return Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height,
        };
    }
    Rect {
        x: area.x,
        y: area.y + top,
        width: area.width,
        height: area.height - top,
    }
}

pub(super) fn field_label_column_width<'a, I>(labels: I, left_padding: u16) -> u16
where
    I: IntoIterator<Item = &'a str>,
{
    let max = labels
        .into_iter()
        .map(|label| UnicodeWidthStr::width(label) as u16)
        .max()
        .unwrap_or(0);
    max.saturating_add(left_padding)
}

pub(super) fn mask_api_key(key: &str) -> String {
    let mut iter = key.chars();
    let prefix: String = iter.by_ref().take(8).collect();
    if iter.next().is_some() {
        format!("{prefix}...")
    } else {
        prefix
    }
}
