use super::super::*;
use super::widgets::*;

fn mission_control_block<'a>(
    app: &MissionControlApp,
    panel: MissionControlPanel,
    title: &'a str,
) -> Block<'a> {
    let mut block = panel_block(title, &app.settings.theme);
    if app.focus == panel {
        block = block.border_style(Style::default().fg(app.settings.theme.accent));
    }
    block
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
