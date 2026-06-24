use super::*;

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

pub(crate) async fn launch_mission_control_ui(
    project: &Path,
    mission_id: Option<&str>,
) -> Result<()> {
    let mission = match mission_id {
        Some(id) => read_mission(project, id)?,
        None => latest_mission(project)?,
    };
    let settings = load_tui_settings(project);
    let mut app = MissionControlApp {
        mission_id: mission.id.clone(),
        mission,
        focus: MissionControlPanel::Agents,
        selected_agent: 0,
        selected_diff: 0,
        last_refresh: Instant::now(),
        message: mission_control_help(),
        settings,
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = run_mission_control_loop(project, &mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

pub(crate) async fn run_mission_control_loop(
    project: &Path,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut MissionControlApp,
) -> Result<()> {
    loop {
        clamp_mission_control_selection(app);
        terminal.draw(|frame| render_mission_control(frame, app))?;

        if event::poll(Duration::from_millis(250))? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Tab => focus_next_mission_panel(app),
                KeyCode::BackTab => focus_previous_mission_panel(app),
                KeyCode::Char('h') => focus_left_mission_panel(app),
                KeyCode::Char('l') => focus_right_mission_panel(app),
                KeyCode::Char('j') => focus_down_mission_panel(app),
                KeyCode::Char('k') => focus_up_mission_panel(app),
                KeyCode::Char('1') => app.focus = MissionControlPanel::Main,
                KeyCode::Char('2') => app.focus = MissionControlPanel::Agents,
                KeyCode::Char('3') => app.focus = MissionControlPanel::Detail,
                KeyCode::Char('4') => app.focus = MissionControlPanel::Diffs,
                KeyCode::Up => move_mission_control_selection(app, -1),
                KeyCode::Down => move_mission_control_selection(app, 1),
                KeyCode::Char('r') => refresh_mission_control(project, app),
                KeyCode::Enter => open_selected_mission_diff(project, terminal, app).await?,
                _ => {}
            }
        }

        if app.last_refresh.elapsed() > Duration::from_secs(3) {
            refresh_mission_control(project, app);
        }
    }
    Ok(())
}

pub(crate) fn render_mission_control(frame: &mut ratatui::Frame<'_>, app: &MissionControlApp) {
    let area = frame.area();
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(36), Constraint::Percentage(64)])
        .split(vertical[1]);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(8)])
        .split(body[0]);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(body[1]);

    render_mission_control_header(frame, app, vertical[0]);
    render_mission_main_panel(frame, app, left[0]);
    render_mission_agents_panel(frame, app, left[1]);
    render_mission_detail_panel(frame, app, right[0]);
    render_mission_diffs_panel(frame, app, right[1]);
    frame.render_widget(
        Paragraph::new(app.message.clone()).style(Style::default().fg(app.settings.theme.muted)),
        vertical[2],
    );
}

pub(crate) fn render_mission_control_header(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let title = Line::from(vec![
        Span::styled(
            "Airev Mission Control",
            Style::default()
                .fg(app.settings.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            &app.mission.id,
            Style::default().fg(app.settings.theme.foreground),
        ),
        Span::raw("  "),
        Span::styled(
            format_mission_status(&app.mission.status),
            status_style(format_mission_status(&app.mission.status)),
        ),
        Span::raw("  "),
        Span::raw(&app.mission.title),
    ]);
    frame.render_widget(
        Paragraph::new(title).alignment(Alignment::Center).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        ),
        area,
    );
}

pub(crate) fn render_mission_main_panel(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let source = if app.mission.source_text.trim().is_empty() {
        "no source text".to_string()
    } else {
        truncate_pretty(app.mission.source_text.trim(), 220)
    };
    let text = vec![
        Line::from(format!(
            "status: {}",
            format_mission_status(&app.mission.status)
        )),
        Line::from(format!("agents: {}", app.mission.agents.len())),
        Line::from(format!("updated: {}", app.mission.updated_at)),
        Line::from(""),
        Line::from(source),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: true })
            .block(mission_control_block(
                app,
                MissionControlPanel::Main,
                "1 MAIN",
            )),
        area,
    );
}

pub(crate) fn render_mission_agents_panel(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let items = app
        .mission
        .agents
        .iter()
        .map(|agent| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<9}", format_mission_agent_status(&agent.status)),
                    status_style(format_mission_agent_status(&agent.status)),
                ),
                Span::raw(" "),
                Span::styled(
                    &agent.project,
                    Style::default()
                        .fg(app.settings.theme.foreground)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("  {}", truncate_pretty(&agent.task, 48))),
            ]))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(app.selected_agent.min(items.len() - 1)));
    }
    frame.render_stateful_widget(
        List::new(items)
            .highlight_style(selected_style(Style::default(), &app.settings.theme, true))
            .block(mission_control_block(
                app,
                MissionControlPanel::Agents,
                "2 AGENTS",
            )),
        area,
        &mut state,
    );
}

pub(crate) fn render_mission_detail_panel(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let Some(agent) = selected_mission_agent(app) else {
        frame.render_widget(
            Paragraph::new("No agent selected").block(mission_control_block(
                app,
                MissionControlPanel::Detail,
                "3 DETAIL",
            )),
            area,
        );
        return;
    };
    let mut lines = vec![
        Line::from(format!("id: {}", agent.id)),
        Line::from(format!("project: {}", agent.project)),
        Line::from(format!("path: {}", agent.project_path)),
        Line::from(format!(
            "status: {}",
            format_mission_agent_status(&agent.status)
        )),
        Line::from(format!("task: {}", agent.task)),
    ];
    if let Some(summary) = &agent.summary {
        lines.push(Line::from(format!("summary: {summary}")));
    }
    if let Some(error) = &agent.last_error {
        lines.push(Line::from(Span::styled(
            format!("last_error: {error}"),
            Style::default().fg(Color::Red),
        )));
    }
    if !agent.revision_ids.is_empty() {
        lines.push(Line::from(format!("revisions: {:?}", agent.revision_ids)));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(mission_control_block(
                app,
                MissionControlPanel::Detail,
                "3 DETAIL",
            )),
        area,
    );
}

pub(crate) fn render_mission_diffs_panel(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let items = selected_mission_agent(app)
        .map(|agent| {
            if agent.diff_refs.is_empty() {
                vec![ListItem::new("No diff refs recorded")]
            } else {
                agent
                    .diff_refs
                    .iter()
                    .enumerate()
                    .map(|(index, diff)| {
                        let label = match diff.revision_id {
                            Some(revision_id) => {
                                format!("[{index}] revision {revision_id}: {}", diff.path)
                            }
                            None => format!("[{index}] unbound: {}", diff.path),
                        };
                        ListItem::new(label)
                    })
                    .collect::<Vec<_>>()
            }
        })
        .unwrap_or_else(|| vec![ListItem::new("No agent selected")]);
    let mut state = ListState::default();
    if selected_mission_agent(app).is_some_and(|agent| !agent.diff_refs.is_empty()) {
        state.select(Some(app.selected_diff.min(items.len() - 1)));
    }
    frame.render_stateful_widget(
        List::new(items)
            .highlight_style(selected_style(Style::default(), &app.settings.theme, true))
            .block(mission_control_block(
                app,
                MissionControlPanel::Diffs,
                "4 DIFFS",
            )),
        area,
        &mut state,
    );
}

pub(crate) fn selected_mission_agent(app: &MissionControlApp) -> Option<&MissionAgent> {
    app.mission.agents.get(app.selected_agent)
}

pub(crate) fn clamp_mission_control_selection(app: &mut MissionControlApp) {
    if app.mission.agents.is_empty() {
        app.selected_agent = 0;
        app.selected_diff = 0;
        return;
    }
    app.selected_agent = app.selected_agent.min(app.mission.agents.len() - 1);
    let diff_len = app.mission.agents[app.selected_agent].diff_refs.len();
    if diff_len == 0 {
        app.selected_diff = 0;
    } else {
        app.selected_diff = app.selected_diff.min(diff_len - 1);
    }
}

pub(crate) fn move_mission_control_selection(app: &mut MissionControlApp, delta: isize) {
    match app.focus {
        MissionControlPanel::Agents => {
            app.selected_agent = move_index(app.selected_agent, app.mission.agents.len(), delta);
            app.selected_diff = 0;
        }
        MissionControlPanel::Diffs => {
            if let Some(agent) = selected_mission_agent(app) {
                app.selected_diff = move_index(app.selected_diff, agent.diff_refs.len(), delta);
            }
        }
        _ => {}
    }
}

pub(crate) fn move_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let last = len.saturating_sub(1) as isize;
    (current as isize + delta).clamp(0, last) as usize
}

pub(crate) fn focus_next_mission_panel(app: &mut MissionControlApp) {
    app.focus = match app.focus {
        MissionControlPanel::Main => MissionControlPanel::Agents,
        MissionControlPanel::Agents => MissionControlPanel::Detail,
        MissionControlPanel::Detail => MissionControlPanel::Diffs,
        MissionControlPanel::Diffs => MissionControlPanel::Main,
    };
}

pub(crate) fn focus_previous_mission_panel(app: &mut MissionControlApp) {
    app.focus = match app.focus {
        MissionControlPanel::Main => MissionControlPanel::Diffs,
        MissionControlPanel::Agents => MissionControlPanel::Main,
        MissionControlPanel::Detail => MissionControlPanel::Agents,
        MissionControlPanel::Diffs => MissionControlPanel::Detail,
    };
}

pub(crate) fn focus_left_mission_panel(app: &mut MissionControlApp) {
    app.focus = match app.focus {
        MissionControlPanel::Detail | MissionControlPanel::Diffs => MissionControlPanel::Agents,
        _ => app.focus,
    };
}

pub(crate) fn focus_right_mission_panel(app: &mut MissionControlApp) {
    app.focus = match app.focus {
        MissionControlPanel::Main | MissionControlPanel::Agents => MissionControlPanel::Detail,
        _ => app.focus,
    };
}

pub(crate) fn focus_down_mission_panel(app: &mut MissionControlApp) {
    app.focus = match app.focus {
        MissionControlPanel::Main => MissionControlPanel::Agents,
        MissionControlPanel::Detail => MissionControlPanel::Diffs,
        _ => app.focus,
    };
}

pub(crate) fn focus_up_mission_panel(app: &mut MissionControlApp) {
    app.focus = match app.focus {
        MissionControlPanel::Agents => MissionControlPanel::Main,
        MissionControlPanel::Diffs => MissionControlPanel::Detail,
        _ => app.focus,
    };
}

pub(crate) fn refresh_mission_control(project: &Path, app: &mut MissionControlApp) {
    match read_mission(project, &app.mission_id) {
        Ok(mission) => {
            app.mission = mission;
            app.last_refresh = Instant::now();
            app.message = mission_control_help();
        }
        Err(error) => {
            app.message = format!("refresh failed: {error}");
        }
    }
}

pub(crate) async fn open_selected_mission_diff(
    project: &Path,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut MissionControlApp,
) -> Result<()> {
    if app.focus != MissionControlPanel::Diffs {
        return Ok(());
    }
    let Some(agent) = selected_mission_agent(app) else {
        app.message = "No agent selected.".to_string();
        return Ok(());
    };
    if agent.diff_refs.is_empty() {
        app.message = "Selected agent has no diff refs.".to_string();
        return Ok(());
    }
    let agent_id = agent.id.clone();
    let diff_index = app.selected_diff;
    suspend_tui(terminal)?;
    let result = mission_open_diff_command(
        project,
        &app.mission_id,
        &agent_id,
        diff_index,
        true,
        "code",
    )
    .await;
    println!("\nPress Enter to return to Mission Control...");
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);
    resume_tui(terminal)?;
    match result {
        Ok(()) => app.message = format!("Opened diff {diff_index} for {agent_id}."),
        Err(error) => app.message = format!("open diff failed: {error}"),
    }
    Ok(())
}

pub(crate) fn mission_control_help() -> String {
    "keys: q quit · tab/shift-tab cycle · h/j/k/l focus · 1-4 panels · ↑/↓ select · r refresh · enter open selected diff".to_string()
}

pub(crate) fn status_style(status: &str) -> Style {
    match status {
        "complete" | "reviewed" => Style::default().fg(Color::Green),
        "failed" | "blocked" => Style::default().fg(Color::Red),
        "waiting" | "unreviewed" => Style::default().fg(Color::Yellow),
        "running" => Style::default().fg(Color::Cyan),
        _ => Style::default(),
    }
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
