use super::super::*;

pub(crate) fn selection_bar(selected: bool, theme: &UiTheme) -> Span<'static> {
    if selected {
        Span::styled(
            "▌ ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    }
}

pub(crate) fn selected_style(style: Style, theme: &UiTheme, selected: bool) -> Style {
    if selected {
        style
            .bg(theme.selected_bg)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        style
    }
}

pub(crate) fn panel_block<'a>(title: &'a str, theme: &UiTheme) -> Block<'a> {
    Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
}

pub(crate) fn status_span(status: &str, theme: &UiTheme) -> Span<'static> {
    let (label, color) = match status {
        "reviewed" => (" reviewed ", theme.added_fg),
        "ignored" => (" ignored ", theme.muted),
        _ => (" unreviewed ", Color::Yellow),
    };
    Span::styled(
        label,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

pub(crate) fn change_span(change_type: &str, theme: &UiTheme) -> Span<'static> {
    let color = match change_type {
        "added" => theme.added_fg,
        "deleted" => theme.removed_fg,
        "modified" => Color::Yellow,
        "renamed" => theme.hunk_fg,
        _ => theme.muted,
    };
    Span::styled(
        format!(" {change_type} "),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

pub(crate) fn file_path_style(change_type: &str, theme: &UiTheme) -> Style {
    let color = match change_type {
        "added" => theme.added_fg,
        "deleted" => theme.removed_fg,
        "renamed" => theme.hunk_fg,
        "modified" => theme.accent,
        _ => theme.muted,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

pub(crate) fn label_style(theme: &UiTheme) -> Style {
    Style::default()
        .fg(theme.muted)
        .add_modifier(Modifier::BOLD)
}

pub(crate) fn key_style(theme: &UiTheme) -> Style {
    Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD)
}

pub(crate) fn suspend_tui(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

pub(crate) fn resume_tui(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    terminal.clear()?;
    Ok(())
}
