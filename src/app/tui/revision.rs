use super::super::*;
use super::diff_view::*;
use super::widgets::*;

pub(crate) async fn launch_tui(project: &Path) -> Result<()> {
    if !project.join(STORE_DIR).join(DB_FILE).exists() {
        println!("Airev is not initialized. Run `airev init`.");
        return Ok(());
    }

    let pool = open_initialized_db(project).await?;
    let revisions = fetch_revisions(&pool).await?;
    let settings = load_tui_settings(project);
    let mut app = TuiApp {
        view: TuiView::Revisions,
        revisions,
        files: Vec::new(),
        selected: 0,
        scroll: 0,
        last_refresh: Instant::now(),
        message: revisions_help(),
        settings,
        theme_names: available_theme_names(project),
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_tui_loop(project, &pool, &mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

pub(crate) async fn run_tui_loop(
    project: &Path,
    pool: &SqlitePool,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TuiApp,
) -> Result<()> {
    loop {
        if app.last_refresh.elapsed() >= Duration::from_secs(1) {
            refresh_tui_data(pool, app).await?;
        }

        terminal.draw(|frame| render_tui(frame, app))?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };

        match key.code {
            KeyCode::Char('q') => break,
            KeyCode::Up => match app.view {
                TuiView::Diff { .. } => app.scroll = app.scroll.saturating_sub(1),
                _ => move_selection(app, -1),
            },
            KeyCode::Down => match app.view {
                TuiView::Diff { ref lines, .. } => {
                    app.scroll = app
                        .scroll
                        .saturating_add(1)
                        .min(lines.len().saturating_sub(1) as u16)
                }
                _ => move_selection(app, 1),
            },
            KeyCode::PageUp => {
                if matches!(app.view, TuiView::Diff { .. }) {
                    app.scroll = app.scroll.saturating_sub(12);
                }
            }
            KeyCode::PageDown => {
                if let TuiView::Diff { ref lines, .. } = app.view {
                    app.scroll = app
                        .scroll
                        .saturating_add(12)
                        .min(lines.len().saturating_sub(1) as u16);
                }
            }
            KeyCode::Esc | KeyCode::Backspace => match &app.view {
                TuiView::Files { .. } => {
                    app.view = TuiView::Revisions;
                    app.files.clear();
                    app.selected = 0;
                    app.scroll = 0;
                    app.message = revisions_help();
                }
                TuiView::Diff { revision, file, .. } => {
                    app.selected = app
                        .files
                        .iter()
                        .position(|candidate| candidate.path == file.path)
                        .unwrap_or(app.selected);
                    app.view = TuiView::Files {
                        revision: revision.clone(),
                    };
                    app.scroll = 0;
                    app.message = files_help();
                }
                TuiView::Themes {
                    return_to,
                    return_selected,
                } => {
                    let selected = *return_selected;
                    app.view = (**return_to).clone();
                    app.selected = selected.min(current_len(app).saturating_sub(1));
                    app.scroll = 0;
                    app.message = help_for_view(app);
                }
                TuiView::Revisions => {}
            },
            KeyCode::Enter => match &app.view {
                TuiView::Revisions => {
                    if let Some(revision) = app.revisions.get(app.selected).cloned() {
                        app.files = fetch_revision_files(pool, &revision.label).await?;
                        app.view = TuiView::Files { revision };
                        app.selected = 0;
                        app.scroll = 0;
                        app.message = files_help();
                    }
                }
                TuiView::Files { .. } => {
                    if let Err(error) = open_selected_terminal_diff(project, app, false).await {
                        app.message = format!("Diff failed: {error}");
                    }
                }
                TuiView::Themes { .. } => {
                    if let Some(theme_name) = app.theme_names.get(app.selected).cloned() {
                        write_theme_selection(&project_config_path(project), &theme_name)?;
                        app.settings = load_tui_settings(project);
                        app.theme_names = available_theme_names(project);
                        app.selected = app
                            .theme_names
                            .iter()
                            .position(|name| name == &app.settings.theme_name)
                            .unwrap_or(0);
                        app.message = format!("Theme set to {theme_name} · {}", themes_help());
                    }
                }
                _ => {}
            },
            KeyCode::Char('t') => {
                if matches!(app.view, TuiView::Revisions) {
                    open_theme_picker(app);
                }
            }
            KeyCode::Char('r') => match &app.view {
                TuiView::Revisions => {
                    if let Some(revision) = app.revisions.get(app.selected) {
                        mark_revision_reviewed(pool, &revision.label).await?;
                        app.revisions = fetch_revisions(pool).await?;
                        app.message = format!("Revision marked reviewed · {}", revisions_help());
                    }
                }
                TuiView::Files { revision } => {
                    if let Some(file) = app.files.get(app.selected) {
                        mark_file_reviewed(pool, &revision.label, &file.path).await?;
                        app.files = fetch_revision_files(pool, &revision.label).await?;
                        app.revisions = fetch_revisions(pool).await?;
                        app.message = format!("File marked reviewed · {}", files_help());
                    }
                }
                TuiView::Diff { .. } | TuiView::Themes { .. } => {}
            },
            KeyCode::Char('d') => {
                if matches!(app.view, TuiView::Files { .. }) {
                    if let Err(error) = open_selected_terminal_diff(project, app, false).await {
                        app.message = format!("Diff failed: {error}");
                    }
                }
            }
            KeyCode::Char('g') => {
                if let TuiView::Diff { raw, .. } = &mut app.view {
                    *raw = !*raw;
                    app.message = diff_help(*raw);
                }
            }
            KeyCode::Char('n') | KeyCode::Char(']') => {
                if matches!(app.view, TuiView::Diff { .. }) && !app.files.is_empty() {
                    let raw = matches!(app.view, TuiView::Diff { raw: true, .. });
                    app.selected = (app.selected + 1).min(app.files.len().saturating_sub(1));
                    if let Err(error) = open_selected_terminal_diff(project, app, raw).await {
                        app.message = format!("Diff failed: {error}");
                    }
                }
            }
            KeyCode::Char('p') | KeyCode::Char('[') => {
                if matches!(app.view, TuiView::Diff { .. }) && !app.files.is_empty() {
                    let raw = matches!(app.view, TuiView::Diff { raw: true, .. });
                    app.selected = app.selected.saturating_sub(1);
                    if let Err(error) = open_selected_terminal_diff(project, app, raw).await {
                        app.message = format!("Diff failed: {error}");
                    }
                }
            }
            KeyCode::Char('e') => {
                let selected = match &app.view {
                    TuiView::Files { revision } => app
                        .files
                        .get(app.selected)
                        .map(|file| (revision.label.clone(), file.path.clone())),
                    TuiView::Diff { revision, file, .. } => {
                        Some((revision.label.clone(), file.path.clone()))
                    }
                    TuiView::Revisions | TuiView::Themes { .. } => None,
                };

                if let Some((revision, file_path)) = selected {
                    suspend_tui(terminal)?;
                    let diff_result = open_diff(project, &revision, &file_path, "code").await;
                    resume_tui(terminal)?;
                    match diff_result {
                        Ok(()) => {
                            app.message = format!("Editor diff opened · {}", help_for_view(app))
                        }
                        Err(error) => app.message = format!("Editor diff failed: {error}"),
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn revisions_help() -> String {
    "Enter: files · t: themes · r: reviewed · q: quit".to_string()
}

pub(crate) fn files_help() -> String {
    "d/Enter: terminal diff · e: editor diff · r: reviewed · Esc: prompts".to_string()
}

pub(crate) fn themes_help() -> String {
    "↑/↓: choose · Enter: apply project theme · Esc: prompts · q: quit".to_string()
}

pub(crate) fn help_for_view(app: &TuiApp) -> String {
    match &app.view {
        TuiView::Revisions => revisions_help(),
        TuiView::Files { .. } => files_help(),
        TuiView::Diff { raw, .. } => diff_help(*raw),
        TuiView::Themes { .. } => themes_help(),
    }
}

pub(crate) fn open_theme_picker(app: &mut TuiApp) {
    if !matches!(app.view, TuiView::Revisions) {
        return;
    }
    let return_to = Box::new(app.view.clone());
    let return_selected = app.selected;
    app.selected = app
        .theme_names
        .iter()
        .position(|name| name == &app.settings.theme_name)
        .unwrap_or(0);
    app.view = TuiView::Themes {
        return_to,
        return_selected,
    };
    app.scroll = 0;
    app.message = themes_help();
}

pub(crate) async fn open_selected_terminal_diff(
    project: &Path,
    app: &mut TuiApp,
    raw: bool,
) -> Result<()> {
    let revision = match &app.view {
        TuiView::Files { revision } | TuiView::Diff { revision, .. } => revision.clone(),
        TuiView::Revisions | TuiView::Themes { .. } => return Ok(()),
    };
    let Some(file) = app.files.get(app.selected).cloned() else {
        return Ok(());
    };
    let lines = build_terminal_diff(project, &revision.label, &file.path).await?;
    app.view = TuiView::Diff {
        revision,
        file,
        lines,
        raw,
    };
    app.scroll = 0;
    app.message = diff_help(raw);
    Ok(())
}

pub(crate) async fn refresh_tui_data(pool: &SqlitePool, app: &mut TuiApp) -> Result<()> {
    let selected_revision_label = match &app.view {
        TuiView::Revisions => app
            .revisions
            .get(app.selected)
            .map(|revision| revision.label.clone()),
        TuiView::Files { revision } | TuiView::Diff { revision, .. } => {
            Some(revision.label.clone())
        }
        TuiView::Themes { .. } => None,
    };
    let keep_top_selected = matches!(app.view, TuiView::Revisions) && app.selected == 0;

    app.revisions = fetch_revisions(pool).await?;

    match &mut app.view {
        TuiView::Revisions => {
            if app.revisions.is_empty() || keep_top_selected {
                app.selected = 0;
            } else if let Some(label) = selected_revision_label {
                app.selected = app
                    .revisions
                    .iter()
                    .position(|revision| revision.label == label)
                    .unwrap_or_else(|| app.selected.min(app.revisions.len().saturating_sub(1)));
            }
        }
        TuiView::Files { revision } => {
            let label = revision.label.clone();
            if let Some(updated_revision) = app
                .revisions
                .iter()
                .find(|item| item.label == label)
                .cloned()
            {
                *revision = updated_revision;
            }
            app.files = fetch_revision_files(pool, &label).await?;
            app.selected = app.selected.min(app.files.len().saturating_sub(1));
        }
        TuiView::Diff { .. } | TuiView::Themes { .. } => {
            // Keep the rendered diff/theme picker stable while the user is reading it.
        }
    }

    app.last_refresh = Instant::now();
    Ok(())
}

pub(crate) fn render_tui(frame: &mut ratatui::Frame<'_>, app: &TuiApp) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(frame.area());

    render_header(frame, app, root[0]);

    match &app.view {
        TuiView::Diff {
            revision,
            file,
            lines,
            raw,
        } => {
            let (file_index, file_count) = selected_file_position(app, file);
            render_diff_view(
                frame,
                revision,
                file,
                lines,
                *raw,
                file_index,
                file_count,
                app.scroll,
                root[1],
                &app.settings,
            );
        }
        _ => {
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
                .split(root[1]);
            render_navigation_panel(frame, app, body[0]);
            render_detail_panel(frame, app, body[1]);
        }
    }

    let help = Paragraph::new(app.message.clone())
        .style(Style::default().fg(app.settings.theme.muted))
        .alignment(Alignment::Center);
    frame.render_widget(help, root[2]);
}

pub(crate) fn selected_file_position(app: &TuiApp, file: &FileRow) -> (usize, usize) {
    let count = app.files.len().max(1);
    let index = app
        .files
        .iter()
        .position(|candidate| candidate.path == file.path)
        .map(|value| value + 1)
        .unwrap_or_else(|| app.selected.saturating_add(1).min(count));
    (index, count)
}

pub(crate) fn render_header(frame: &mut ratatui::Frame<'_>, app: &TuiApp, area: Rect) {
    let (title, subtitle) = match &app.view {
        TuiView::Revisions => (
            "Airev".to_string(),
            format!(
                "{} prompts · {} waiting for review",
                app.revisions.len(),
                app.revisions
                    .iter()
                    .filter(|revision| revision.status == "unreviewed")
                    .count()
            ),
        ),
        TuiView::Files { revision } => (
            format!("Prompt {}", revision_number(&revision.label)),
            format!(
                "{} · {} files",
                revision_display_title(&revision.title),
                revision.changed_file_count
            ),
        ),
        TuiView::Diff {
            revision,
            file,
            raw,
            ..
        } => {
            let (index, count) = selected_file_position(app, file);
            let mode = if *raw { "raw diff" } else { "readable diff" };
            (
                format!("Prompt {} {mode}", revision_number(&revision.label)),
                format!("file {index}/{count} · {}", file.path),
            )
        }
        TuiView::Themes { .. } => (
            "Themes".to_string(),
            format!(
                "current: {} · Enter applies to project config",
                app.settings.theme_name
            ),
        ),
    };

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("▰ ", Style::default().fg(app.settings.theme.accent)),
            Span::styled(
                title,
                Style::default()
                    .fg(app.settings.theme.foreground)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            subtitle,
            Style::default().fg(app.settings.theme.muted),
        )),
    ])
    .style(Style::default().bg(app.settings.theme.background))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(app.settings.theme.border)),
    );
    frame.render_widget(header, area);
}

pub(crate) fn render_navigation_panel(frame: &mut ratatui::Frame<'_>, app: &TuiApp, area: Rect) {
    let (title, items): (&str, Vec<ListItem<'static>>) = match &app.view {
        TuiView::Revisions => (
            "Prompts",
            app.revisions
                .iter()
                .enumerate()
                .map(|(index, revision)| {
                    revision_list_item(revision, &app.settings.theme, index == app.selected)
                })
                .collect(),
        ),
        TuiView::Files { .. } => (
            "Changed files",
            app.files
                .iter()
                .enumerate()
                .map(|(index, file)| {
                    file_list_item(file, &app.settings.theme, index == app.selected)
                })
                .collect(),
        ),
        TuiView::Themes { .. } => (
            "Themes",
            app.theme_names
                .iter()
                .enumerate()
                .map(|(index, name)| theme_list_item(name, &app.settings, index == app.selected))
                .collect(),
        ),
        TuiView::Diff { .. } => ("", Vec::new()),
    };

    let list = List::new(items)
        .block(panel_block(title, &app.settings.theme))
        .style(Style::default().bg(app.settings.theme.background))
        .highlight_symbol("  ");
    let mut state = ListState::default();
    if current_len(app) > 0 {
        state.select(Some(app.selected.min(current_len(app) - 1)));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

pub(crate) fn render_detail_panel(frame: &mut ratatui::Frame<'_>, app: &TuiApp, area: Rect) {
    let lines = match &app.view {
        TuiView::Revisions => selected_revision_detail(app),
        TuiView::Files { revision } => selected_file_detail(app, revision),
        TuiView::Themes { .. } => selected_theme_detail(app),
        TuiView::Diff { .. } => Vec::new(),
    };

    let detail = Paragraph::new(lines)
        .style(Style::default().bg(app.settings.theme.background))
        .block(panel_block("Details", &app.settings.theme))
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, area);
}

pub(crate) fn file_list_item(file: &FileRow, theme: &UiTheme, selected: bool) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        selection_bar(selected, theme),
        Span::styled(
            file.path.clone(),
            selected_style(file_path_style(&file.change_type, theme), theme, selected),
        ),
    ]))
}

pub(crate) fn theme_list_item(
    name: &str,
    settings: &TuiSettings,
    selected: bool,
) -> ListItem<'static> {
    let marker = if name == settings.theme_name {
        "●"
    } else {
        "○"
    };
    let style = if name == settings.theme_name {
        Style::default()
            .fg(settings.theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(settings.theme.foreground)
    };
    let style = selected_style(style, &settings.theme, selected);
    ListItem::new(Line::from(vec![
        selection_bar(selected, &settings.theme),
        Span::styled(format!("{marker} "), style),
        Span::styled(name.to_string(), style),
    ]))
}

pub(crate) fn revision_list_item(
    revision: &RevisionRow,
    theme: &UiTheme,
    selected: bool,
) -> ListItem<'static> {
    ListItem::new(vec![
        Line::from(vec![
            selection_bar(selected, theme),
            Span::styled(
                format!("Prompt {}", revision_number(&revision.label)),
                selected_style(
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                    theme,
                    selected,
                ),
            ),
            Span::raw("  "),
            Span::styled(
                revision_display_title(&revision.title),
                selected_style(
                    Style::default()
                        .fg(theme.foreground)
                        .add_modifier(Modifier::BOLD),
                    theme,
                    selected,
                ),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            status_span(&revision.status, theme),
            Span::styled(
                format!(
                    "  {} file{}",
                    revision.changed_file_count,
                    plural(revision.changed_file_count)
                ),
                Style::default().fg(theme.muted),
            ),
            Span::styled("  ·  ", Style::default().fg(theme.muted)),
            Span::styled(revision.label.clone(), Style::default().fg(theme.muted)),
            Span::styled("  ·  ", Style::default().fg(theme.muted)),
            Span::styled(
                short_timestamp(&revision.created_at),
                Style::default().fg(theme.muted),
            ),
        ]),
    ])
}

pub(crate) fn selected_theme_detail(app: &TuiApp) -> Vec<Line<'static>> {
    let theme = &app.settings.theme;
    let selected = app
        .theme_names
        .get(app.selected)
        .cloned()
        .unwrap_or_else(|| app.settings.theme_name.clone());
    vec![
        Line::from(Span::styled(
            selected.clone(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Current ", label_style(theme)),
            Span::styled(
                app.settings.theme_name.clone(),
                Style::default().fg(theme.foreground),
            ),
        ]),
        Line::from(vec![
            Span::styled("Project ", label_style(theme)),
            Span::styled(
                ".ai-revisions/config.toml",
                Style::default().fg(theme.muted),
            ),
        ]),
        Line::from(vec![
            Span::styled("Global  ", label_style(theme)),
            Span::styled(
                "~/.config/airev/config.toml",
                Style::default().fg(theme.muted),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Enter saves the selected theme for this project. Custom themes can extend terminal or any built-in theme in config.toml.",
            Style::default().fg(theme.muted),
        )),
    ]
}

pub(crate) fn selected_revision_detail(app: &TuiApp) -> Vec<Line<'static>> {
    let theme = &app.settings.theme;
    let Some(revision) = app.revisions.get(app.selected) else {
        return vec![Line::from(Span::styled(
            "No prompts recorded yet",
            Style::default().fg(theme.muted),
        ))];
    };

    vec![
        Line::from(Span::styled(
            format!("Prompt {}", revision_number(&revision.label)),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            revision_display_title(&revision.title),
            Style::default()
                .fg(theme.foreground)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Status  ", label_style(theme)),
            status_span(&revision.status, theme),
        ]),
        Line::from(vec![
            Span::styled("Files   ", label_style(theme)),
            Span::styled(
                revision.changed_file_count.to_string(),
                Style::default().fg(theme.foreground),
            ),
        ]),
        Line::from(vec![
            Span::styled("Created ", label_style(theme)),
            Span::styled(
                short_timestamp(&revision.created_at),
                Style::default().fg(theme.muted),
            ),
        ]),
        Line::from(vec![
            Span::styled("ID      ", label_style(theme)),
            Span::styled(revision.label.clone(), Style::default().fg(theme.muted)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Enter opens changed files. New prompts appear live.",
            Style::default().fg(theme.muted),
        )),
    ]
}

pub(crate) fn selected_file_detail(app: &TuiApp, revision: &RevisionRow) -> Vec<Line<'static>> {
    let theme = &app.settings.theme;
    let Some(file) = app.files.get(app.selected) else {
        return vec![Line::from(Span::styled(
            "No changed files",
            Style::default().fg(theme.muted),
        ))];
    };

    vec![
        Line::from(vec![
            Span::styled("Revision ", label_style(theme)),
            Span::styled(revision.label.clone(), Style::default().fg(theme.accent)),
        ]),
        Line::from(vec![
            Span::styled("Status   ", label_style(theme)),
            status_span(&file.status, theme),
        ]),
        Line::from(vec![
            Span::styled("Change   ", label_style(theme)),
            change_span(&file.change_type, theme),
        ]),
        Line::from(""),
        Line::from(Span::styled("Path", label_style(theme))),
        Line::from(Span::styled(
            file.path.clone(),
            Style::default().fg(theme.foreground),
        )),
        Line::from(""),
        Line::from(Span::styled("Actions", label_style(theme))),
        Line::from(vec![
            Span::styled("Enter", key_style(theme)),
            Span::raw("  show terminal diff"),
        ]),
        Line::from(vec![
            Span::styled("e", key_style(theme)),
            Span::raw("  open external editor diff"),
        ]),
        Line::from(vec![
            Span::styled("r", key_style(theme)),
            Span::raw("  mark file reviewed"),
        ]),
    ]
}

pub(crate) fn revision_number(label: &str) -> String {
    label
        .strip_prefix('r')
        .and_then(|value| value.parse::<i64>().ok())
        .map(|value| format!("#{value}"))
        .unwrap_or_else(|| label.to_string())
}

pub(crate) fn revision_display_title(title: &str) -> String {
    let normalized = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = if normalized.is_empty() {
        "AI code changes".to_string()
    } else {
        normalized
    };
    truncate_pretty(&sentence_case_title(&title), 48)
}

pub(crate) fn current_len(app: &TuiApp) -> usize {
    match app.view {
        TuiView::Revisions => app.revisions.len(),
        TuiView::Files { .. } => app.files.len(),
        TuiView::Themes { .. } => app.theme_names.len(),
        TuiView::Diff { .. } => 0,
    }
}

pub(crate) fn move_selection(app: &mut TuiApp, delta: isize) {
    let len = current_len(app);
    if len == 0 {
        app.selected = 0;
        return;
    }
    app.selected = app
        .selected
        .saturating_add_signed(delta)
        .min(len.saturating_sub(1));
}
