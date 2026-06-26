use super::super::*;
use super::widgets::*;

pub(crate) fn render_diff_view(
    frame: &mut ratatui::Frame<'_>,
    revision: &RevisionRow,
    file: &FileRow,
    lines: &[DiffLine],
    raw: bool,
    file_index: usize,
    file_count: usize,
    scroll: u16,
    area: Rect,
    settings: &TuiSettings,
) {
    let rendered: Vec<Line> = if lines.is_empty() {
        vec![Line::from(Span::styled(
            "No textual diff available",
            Style::default().fg(settings.theme.muted),
        ))]
    } else {
        diff_lines_to_tui_lines(lines, &file.path, raw, settings)
    };

    let mode = if raw { "raw" } else { "readable" };
    let title = if settings.ui.compact_file_bar {
        format!(
            "{} · file {}/{} · {} · {}",
            revision.label, file_index, file_count, mode, file.path
        )
    } else {
        format!(
            "revision {} · changed file {}/{} · {} mode · {}",
            revision.label, file_index, file_count, mode, file.path
        )
    };
    let diff = Paragraph::new(rendered)
        .style(Style::default().bg(settings.theme.background))
        .block(panel_block(&title, &settings.theme))
        .scroll((scroll, 0));
    frame.render_widget(diff, area);
}

pub(crate) fn diff_lines_to_tui_lines(
    lines: &[DiffLine],
    path: &str,
    raw: bool,
    settings: &TuiSettings,
) -> Vec<Line<'static>> {
    let syntax_set = SyntaxSet::load_defaults_newlines();
    let theme_set = ThemeSet::load_defaults();
    let syntax = syntax_set
        .find_syntax_for_file(path)
        .ok()
        .flatten()
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text());
    let syntect_theme = theme_set
        .themes
        .get(&settings.ui.syntect_theme)
        .or_else(|| theme_set.themes.get("base16-ocean.dark"))
        .or_else(|| theme_set.themes.values().next())
        .expect("syntect default themes are available");
    let mut highlighter = HighlightLines::new(syntax, syntect_theme);
    let visible_lines: Vec<&DiffLine> = if raw || settings.ui.show_raw_git_headers {
        lines.iter().collect()
    } else {
        lines
            .iter()
            .filter(|line| {
                !matches!(
                    line.kind,
                    DiffLineKind::Header | DiffLineKind::Hunk | DiffLineKind::Meta
                )
            })
            .collect()
    };
    let number_width = line_number_width(&visible_lines);

    visible_lines
        .iter()
        .map(|line| {
            diff_line_to_highlighted_line(
                line,
                &syntax_set,
                &mut highlighter,
                raw,
                number_width,
                settings,
            )
        })
        .collect()
}

pub(crate) fn diff_line_to_highlighted_line(
    line: &DiffLine,
    syntax_set: &SyntaxSet,
    highlighter: &mut HighlightLines<'_>,
    raw: bool,
    number_width: usize,
    settings: &TuiSettings,
) -> Line<'static> {
    let theme = &settings.theme;
    match line.kind {
        DiffLineKind::Added | DiffLineKind::Removed | DiffLineKind::Context => {
            let (prefix, code) = split_diff_prefix(&line.text);
            let background = match line.kind {
                DiffLineKind::Added => Some(theme.added_bg),
                DiffLineKind::Removed => Some(theme.removed_bg),
                _ => None,
            };
            let mut spans = Vec::new();
            if !raw && settings.ui.show_line_numbers {
                spans.extend(line_number_spans(line, number_width, theme, background));
                spans.push(Span::styled(
                    "  ",
                    Style::default().bg(background.unwrap_or(Color::Reset)),
                ));
            } else {
                spans.push(Span::styled(
                    format!("{prefix} "),
                    diff_gutter_style(&line.kind, theme).bg(background.unwrap_or(Color::Reset)),
                ));
            }

            let highlighted = highlighter
                .highlight_line(code, syntax_set)
                .unwrap_or_else(|_| vec![(SyntectStyle::default(), code)]);
            for (style, token) in highlighted {
                spans.push(Span::styled(
                    token.to_string(),
                    syntect_to_ratatui_style(style, background),
                ));
            }
            Line::from(spans)
        }
        DiffLineKind::Header => Line::from(Span::styled(
            line.text.clone(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
        DiffLineKind::Hunk => Line::from(Span::styled(
            hunk_label(&line.text),
            Style::default()
                .fg(theme.hunk_fg)
                .add_modifier(Modifier::BOLD),
        )),
        DiffLineKind::Meta => Line::from(Span::styled(
            line.text.clone(),
            Style::default().fg(theme.metadata_fg),
        )),
    }
}

pub(crate) fn line_number_width(lines: &[&DiffLine]) -> usize {
    lines
        .iter()
        .filter_map(|line| display_line_number(line))
        .max()
        .map(|line| line.to_string().len().max(3))
        .unwrap_or(3)
}

pub(crate) fn line_number_spans(
    line: &DiffLine,
    width: usize,
    theme: &UiTheme,
    background: Option<Color>,
) -> Vec<Span<'static>> {
    let background = background.unwrap_or(Color::Reset);
    let number_style = Style::default().fg(theme.muted).bg(background);
    let marker_style = diff_gutter_style(&line.kind, theme).bg(background);
    vec![
        Span::styled(
            format_line_number(display_line_number(line), width),
            number_style,
        ),
        Span::styled(display_line_marker(line).to_string(), marker_style),
        Span::styled(" │ ", Style::default().fg(theme.border).bg(background)),
    ]
}

pub(crate) fn display_line_number(line: &DiffLine) -> Option<u32> {
    match line.kind {
        DiffLineKind::Removed => line.old_line,
        DiffLineKind::Added => line.new_line,
        DiffLineKind::Context => line.new_line.or(line.old_line),
        _ => line.new_line.or(line.old_line),
    }
}

pub(crate) fn display_line_marker(line: &DiffLine) -> &'static str {
    match line.kind {
        DiffLineKind::Removed => "−",
        DiffLineKind::Added => "+",
        _ => " ",
    }
}

pub(crate) fn diff_gutter_style(kind: &DiffLineKind, theme: &UiTheme) -> Style {
    match kind {
        DiffLineKind::Added => Style::default()
            .fg(theme.added_fg)
            .add_modifier(Modifier::BOLD),
        DiffLineKind::Removed => Style::default()
            .fg(theme.removed_fg)
            .add_modifier(Modifier::BOLD),
        _ => Style::default().fg(theme.muted),
    }
}

pub(crate) fn syntect_to_ratatui_style(style: SyntectStyle, background: Option<Color>) -> Style {
    let mut tui_style = Style::default().fg(Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ));
    if let Some(background) = background {
        tui_style = tui_style.bg(background);
    }
    tui_style
}
