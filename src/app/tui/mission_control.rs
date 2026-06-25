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
    local_project: Option<&Path>,
) -> Result<()> {
    let mission = match mission_id {
        Some(id) => read_mission(project, id)?,
        None => latest_mission(project).unwrap_or_else(|_| empty_mission()),
    };
    let settings = load_tui_settings(project);
    let mut app = MissionControlApp {
        mission_id: mission.id.clone(),
        mission,
        focus: MissionControlPanel::Agents,
        selected_agent: 0,
        selected_diff: 0,
        workspace_parent: None,
        projects: discover_revision_projects(project, local_project),
        selected_project: 0,
        launch_revision_project: None,
        compose_input: String::new(),
        fast_profile: false,
        terminal_input: false,
        terminals: BTreeMap::new(),
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

    result?;
    if let Some(project) = app.launch_revision_project.clone() {
        launch_tui(&project).await?;
    }
    Ok(())
}

pub(crate) fn empty_mission() -> Mission {
    let now = Utc::now().to_rfc3339();
    Mission {
        id: "no-mission".to_string(),
        title: "No mission yet".to_string(),
        source_text: "Create one with `pb mission start --task project=task`.".to_string(),
        status: MissionStatus::Waiting,
        created_at: now.clone(),
        updated_at: now,
        agents: Vec::new(),
    }
}

pub(crate) fn discover_revision_projects(
    control_root: &Path,
    local_project: Option<&Path>,
) -> Vec<PathBuf> {
    let mut projects = BTreeSet::new();
    if let Some(project) = local_project {
        if project.join(STORE_DIR).join(DB_FILE).exists() {
            projects.insert(project.to_path_buf());
        }
    }

    if let Ok(registry) = load_project_registry(control_root) {
        for registered in registry.projects.values() {
            if registered.path.join(STORE_DIR).join(DB_FILE).exists() {
                projects.insert(registered.path.clone());
            }
        }
    }

    projects.into_iter().collect()
}

pub(crate) async fn run_mission_control_loop(
    project: &Path,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut MissionControlApp,
) -> Result<()> {
    loop {
        clamp_mission_control_selection(app);
        refresh_tmux_terminal_outputs(app);
        terminal.draw(|frame| render_mission_control(frame, app))?;

        if event::poll(Duration::from_millis(250))? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if app.terminal_input {
                if key.code == KeyCode::Esc {
                    app.terminal_input = false;
                    app.message = mission_control_help();
                } else if let Err(error) = send_key_to_focused_terminal(app, key.code) {
                    app.message = format!("terminal input failed: {error}");
                }
                continue;
            }
            if app.focus == MissionControlPanel::Composer {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Esc => app.focus = MissionControlPanel::Main,
                    KeyCode::Backspace => {
                        app.compose_input.pop();
                    }
                    KeyCode::Char('f') if app.compose_input.is_empty() => {
                        app.fast_profile = !app.fast_profile;
                        app.message = format!("fast profile: {}", if app.fast_profile { "on" } else { "off" });
                    }
                    KeyCode::Enter => {
                        let input = app.compose_input.trim().to_string();
                        if input.is_empty() {
                            app.message = "Compose text is empty.".to_string();
                        } else {
                            match compose_mission_command(project, Some(input), None, false, app.fast_profile).await {
                                Ok(()) => match latest_mission(project) {
                                    Ok(mission) => {
                                        app.mission_id = mission.id.clone();
                                        app.mission = mission;
                                        app.workspace_parent = None;
                                        app.selected_agent = 0;
                                        app.selected_diff = 0;
                                        app.compose_input.clear();
                                        app.focus = MissionControlPanel::Agents;
                                        start_visible_agent_terminals_from_ui(project, app);
                                        if app.terminals.is_empty() {
                                            app.message = "Mission composed, but no GSD terminals started.".to_string();
                                        }
                                    }
                                    Err(error) => app.message = format!("compose created no mission: {error}"),
                                },
                                Err(error) => app.message = format!("compose failed: {error}"),
                            }
                        }
                    }
                    KeyCode::Char(ch) => app.compose_input.push(ch),
                    KeyCode::Tab => focus_next_mission_panel(app),
                    KeyCode::BackTab => focus_previous_mission_panel(app),
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Esc => {
                        if !leave_agent_workspace(app) {
                            app.focus = MissionControlPanel::Main;
                        }
                    }
                    KeyCode::Tab => focus_next_mission_panel(app),
                    KeyCode::BackTab => focus_previous_mission_panel(app),
                    KeyCode::Char('c') => app.focus = MissionControlPanel::Composer,
                    KeyCode::Char('f') => {
                        app.fast_profile = !app.fast_profile;
                        app.message = format!("fast profile: {}", if app.fast_profile { "on" } else { "off" });
                    }
                    KeyCode::Char('d') => app.focus = MissionControlPanel::Projects,
                    KeyCode::Left | KeyCode::Up => move_mission_control_selection(app, -1),
                    KeyCode::Right | KeyCode::Down => move_mission_control_selection(app, 1),
                    KeyCode::Char('i') => enter_terminal_input_mode(app),
                    KeyCode::Char('r') => refresh_mission_control(project, app),
                    KeyCode::Char('s') => start_focused_agent_terminal_from_ui(project, app),
                    KeyCode::Char('S') => start_visible_agent_terminals_from_ui(project, app),
                    KeyCode::Char('R') => run_visible_agents_from_ui(project, app),
                    KeyCode::Enter => {
                        if app.focus == MissionControlPanel::Projects {
                            if let Some(project) = app.projects.get(app.selected_project).cloned() {
                                app.launch_revision_project = Some(project);
                                break;
                            }
                            app.message = "No initialized revision projects found.".to_string();
                        } else if app.focus == MissionControlPanel::Diffs {
                            open_selected_mission_diff(project, terminal, app).await?;
                        } else if !enter_selected_agent_workspace(app) {
                            app.message = "Selected agent has no child workspace.".to_string();
                        }
                    }
                    _ => {}
                }
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
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default().style(
            Style::default()
                .fg(app.settings.theme.foreground)
                .bg(app.settings.theme.background),
        ),
        area,
    );
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);

    render_mission_control_header(frame, app, vertical[0]);
    match app.focus {
        MissionControlPanel::Projects => render_mission_projects_panel(frame, app, vertical[1]),
        MissionControlPanel::Composer => render_mission_composer_panel(frame, app, vertical[1]),
        _ => render_agent_workspace(frame, app, vertical[1]),
    }
    frame.render_widget(
        Paragraph::new(app.message.clone()).style(
            Style::default()
                .fg(app.settings.theme.muted)
                .bg(app.settings.theme.background),
        ),
        vertical[2],
    );
}

pub(crate) fn render_agent_workspace(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    render_agent_tiles(frame, app, area);
}

pub(crate) fn render_agent_tiles(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let agents = visible_agent_indexes(app);
    if agents.is_empty() {
        frame.render_widget(
            Paragraph::new("No agents in this workspace. Press c to compose a mission.")
                .block(mission_control_block(app, MissionControlPanel::Agents, "AGENT TILES"))
                .style(
                    Style::default()
                        .fg(app.settings.theme.muted)
                        .bg(app.settings.theme.background),
                ),
            area,
        );
        return;
    }

    let rows = if agents.len() <= 2 { 1 } else { 2 };
    let cols = ((agents.len() + rows - 1) / rows).max(1);
    let row_constraints = vec![Constraint::Ratio(1, rows as u32); rows];
    let col_constraints = vec![Constraint::Ratio(1, cols as u32); cols];
    let row_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(area);

    for (visible_index, agent_index) in agents.iter().enumerate() {
        let row = visible_index / cols;
        let col = visible_index % cols;
        let col_areas = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(col_constraints.clone())
            .split(row_areas[row]);
        let agent = &app.mission.agents[*agent_index];
        let child_count = child_agent_indexes(app, &agent.id).len();
        let selected = visible_index == app.selected_agent;
        let title = format!("{} {}", if selected { "▶" } else { " " }, agent.id);
        let mut block = panel_block(&title, &app.settings.theme);
        if selected {
            block = block.border_style(Style::default().fg(app.settings.theme.accent));
        }
        let mut lines = vec![
            Line::from(Span::styled(&agent.task, Style::default().fg(app.settings.theme.foreground).add_modifier(Modifier::BOLD))),
            Line::from(format!("status: {}", format_mission_agent_status(&agent.status))),
            Line::from(format!("project: {}", agent.project)),
            Line::from(format!("children: {child_count}")),
            Line::from(format!("profile: {}", agent.runner_profile.as_deref().unwrap_or("default"))),
        ];
        if let Some(session) = app.terminals.get(&agent.id) {
            lines.push(Line::from(Span::styled(
                format!("─ tmux {} {} ─", session.tmux_session, session.tmux_pane),
                Style::default().fg(app.settings.theme.accent),
            )));
            for line in session.output.lines().rev().take(18).collect::<Vec<_>>().into_iter().rev() {
                lines.push(Line::from(line.to_string()));
            }
        } else if let Some(last_log) = agent.last_log.as_deref() {
            lines.push(Line::from(last_log.to_string()));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: true })
                .style(
                    Style::default()
                        .fg(app.settings.theme.foreground)
                        .bg(app.settings.theme.background),
                )
                .block(block),
            col_areas[col],
        );
    }
}

pub(crate) fn render_mission_composer_panel(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let lines = vec![
        Line::from("Describe the app and features. Example:"),
        Line::from("cashpilot: onboarding, billing fixes, CSV export"),
        Line::from(""),
        Line::from(vec![Span::styled("fast profile: ", label_style(&app.settings.theme)), Span::raw(if app.fast_profile { "on" } else { "off" })]),
        Line::from(""),
        Line::from(app.compose_input.as_str()),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .style(
                Style::default()
                    .fg(app.settings.theme.foreground)
                    .bg(app.settings.theme.background),
            )
            .block(mission_control_block(app, MissionControlPanel::Composer, "C COMPOSE MISSION")),
        area,
    );
}

pub(crate) fn render_mission_projects_panel(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let items = if app.projects.is_empty() {
        vec![ListItem::new(Line::from(vec![
            Span::styled("No initialized revision projects found", Style::default().fg(app.settings.theme.muted)),
            Span::raw(" — run `pb init` in a project or add it to config."),
        ]))]
    } else {
        app.projects
            .iter()
            .enumerate()
            .map(|(index, project)| {
                let marker = if index == app.selected_project { "› " } else { "  " };
                ListItem::new(Line::from(vec![
                    Span::styled(marker, Style::default().fg(app.settings.theme.accent)),
                    Span::raw(project.display().to_string()),
                ]))
            })
            .collect()
    };
    frame.render_widget(
        List::new(items)
            .style(
                Style::default()
                    .fg(app.settings.theme.foreground)
                    .bg(app.settings.theme.background),
            )
            .block(mission_control_block(app, MissionControlPanel::Projects, "D PROJECT DIFFS")),
        area,
    );
}

pub(crate) fn render_mission_control_header(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let title = Line::from(vec![
        Span::styled(
            "Patchbay Mission Control",
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
        Paragraph::new(title)
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(app.settings.theme.foreground)
                    .bg(app.settings.theme.background),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .style(
                        Style::default()
                            .fg(app.settings.theme.foreground)
                            .bg(app.settings.theme.background),
                    )
                    .border_style(
                        Style::default()
                            .fg(app.settings.theme.border)
                            .bg(app.settings.theme.background),
                    ),
            ),
        area,
    );
}

pub(crate) fn selected_mission_agent(app: &MissionControlApp) -> Option<&MissionAgent> {
    visible_agent_indexes(app)
        .get(app.selected_agent)
        .and_then(|index| app.mission.agents.get(*index))
}

pub(crate) fn visible_agent_indexes(app: &MissionControlApp) -> Vec<usize> {
    app.mission
        .agents
        .iter()
        .enumerate()
        .filter(|(_, agent)| agent.parent_id.as_deref() == app.workspace_parent.as_deref())
        .map(|(index, _)| index)
        .collect()
}

pub(crate) fn child_agent_indexes(app: &MissionControlApp, parent_id: &str) -> Vec<usize> {
    app.mission
        .agents
        .iter()
        .enumerate()
        .filter(|(_, agent)| agent.parent_id.as_deref() == Some(parent_id))
        .map(|(index, _)| index)
        .collect()
}

pub(crate) fn enter_selected_agent_workspace(app: &mut MissionControlApp) -> bool {
    let Some(agent) = selected_mission_agent(app) else {
        return false;
    };
    if child_agent_indexes(app, &agent.id).is_empty() {
        return false;
    }
    app.workspace_parent = Some(agent.id.clone());
    app.selected_agent = 0;
    app.selected_diff = 0;
    true
}

pub(crate) fn leave_agent_workspace(app: &mut MissionControlApp) -> bool {
    let Some(parent_id) = app.workspace_parent.clone() else {
        return false;
    };
    let parent = app
        .mission
        .agents
        .iter()
        .find(|agent| agent.id == parent_id)
        .and_then(|agent| agent.parent_id.clone());
    app.workspace_parent = parent;
    app.selected_agent = 0;
    app.selected_diff = 0;
    true
}

pub(crate) fn clamp_mission_control_selection(app: &mut MissionControlApp) {
    if app.projects.is_empty() {
        app.selected_project = 0;
    } else {
        app.selected_project = app.selected_project.min(app.projects.len() - 1);
    }

    let visible = visible_agent_indexes(app);
    if visible.is_empty() {
        app.selected_agent = 0;
        app.selected_diff = 0;
        return;
    }
    app.selected_agent = app.selected_agent.min(visible.len() - 1);
    let diff_len = app.mission.agents[visible[app.selected_agent]].diff_refs.len();
    if diff_len == 0 {
        app.selected_diff = 0;
    } else {
        app.selected_diff = app.selected_diff.min(diff_len - 1);
    }
}

pub(crate) fn move_mission_control_selection(app: &mut MissionControlApp, delta: isize) {
    match app.focus {
        MissionControlPanel::Agents | MissionControlPanel::Main | MissionControlPanel::Detail => {
            app.selected_agent = move_index(app.selected_agent, visible_agent_indexes(app).len(), delta);
            app.selected_diff = 0;
        }
        MissionControlPanel::Diffs => {
            if let Some(agent) = selected_mission_agent(app) {
                app.selected_diff = move_index(app.selected_diff, agent.diff_refs.len(), delta);
            }
        }
        MissionControlPanel::Projects => {
            app.selected_project = move_index(app.selected_project, app.projects.len(), delta);
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
        MissionControlPanel::Diffs => MissionControlPanel::Projects,
        MissionControlPanel::Projects => MissionControlPanel::Composer,
        MissionControlPanel::Composer => MissionControlPanel::Main,
    };
}

pub(crate) fn focus_previous_mission_panel(app: &mut MissionControlApp) {
    app.focus = match app.focus {
        MissionControlPanel::Main => MissionControlPanel::Composer,
        MissionControlPanel::Agents => MissionControlPanel::Main,
        MissionControlPanel::Detail => MissionControlPanel::Agents,
        MissionControlPanel::Diffs => MissionControlPanel::Detail,
        MissionControlPanel::Projects => MissionControlPanel::Diffs,
        MissionControlPanel::Composer => MissionControlPanel::Projects,
    };
}

pub(crate) fn refresh_mission_control(project: &Path, app: &mut MissionControlApp) {
    let mission_result = if app.mission_id == "no-mission" {
        latest_mission(project)
    } else {
        read_mission(project, &app.mission_id)
    };

    match mission_result {
        Ok(mission) => {
            app.mission_id = mission.id.clone();
            app.mission = mission;
            app.last_refresh = Instant::now();
            app.message = mission_control_help();
        }
        Err(error) => {
            app.message = format!("refresh failed: {error}");
        }
    }
}

pub(crate) fn start_focused_agent_terminal_from_ui(project: &Path, app: &mut MissionControlApp) {
    let Some(agent) = selected_mission_agent(app) else {
        app.message = "No agent selected.".to_string();
        return;
    };
    let agent_id = agent.id.clone();
    if app.terminals.contains_key(&agent_id) {
        app.message = format!("agent {agent_id} already has a terminal; press i to type");
        return;
    }
    match start_agent_terminal(project, app, &agent_id) {
        Ok(()) => {
            app.terminal_input = true;
            app.message = format!("agent {agent_id} terminal started; typing goes to GSD, Esc returns to WM");
        }
        Err(error) => app.message = format!("agent {agent_id} terminal failed: {error}"),
    }
}

pub(crate) fn start_agent_terminal(
    project: &Path,
    app: &mut MissionControlApp,
    agent_id: &str,
) -> Result<()> {
    ensure_tmux_available()?;
    let agent = app
        .mission
        .agents
        .iter_mut()
        .find(|agent| agent.id == agent_id)
        .ok_or_else(|| anyhow!("Mission has no agent `{agent_id}`"))?;

    let profile = if app.fast_profile { "fast" } else { "default" };
    let session_name = tmux_session_name(&app.mission.id);
    let window_name = tmux_window_name(&agent.id);
    let shell_command = tmux_agent_shell_command(project, &app.mission.id, agent, profile);

    let pane_id = if !tmux_has_session(&session_name) {
        tmux_command([
            "new-session",
            "-d",
            "-s",
            session_name.as_str(),
            "-n",
            window_name.as_str(),
            "-c",
            agent.project_path.as_str(),
            shell_command.as_str(),
        ])?;
        tmux_command([
            "set-option",
            "-t",
            session_name.as_str(),
            "remain-on-exit",
            "on",
        ])?;
        tmux_pane_id(&format!("{}:{}", session_name, window_name))?
    } else {
        tmux_command([
            "new-window",
            "-d",
            "-t",
            session_name.as_str(),
            "-n",
            window_name.as_str(),
            "-c",
            agent.project_path.as_str(),
            shell_command.as_str(),
        ])?;
        tmux_pane_id(&format!("{}:{}", session_name, window_name))?
    };

    agent.status = MissionAgentStatus::Running;
    agent.runner_profile = Some(profile.to_string());
    agent.session_id = Some(format!("tmux:{session_name}:{pane_id}"));
    agent.started_at = Some(Utc::now().to_rfc3339());
    agent.updated_at = Utc::now().to_rfc3339();
    refresh_mission_status(&mut app.mission);
    write_mission(project, &app.mission)?;

    let output = tmux_capture_pane(&pane_id).unwrap_or_default();
    app.terminals.insert(
        agent_id.to_string(),
        AgentTerminalSession {
            tmux_session: session_name,
            tmux_pane: pane_id,
            output,
        },
    );
    Ok(())
}

pub(crate) fn enter_terminal_input_mode(app: &mut MissionControlApp) {
    let Some(agent_id) = selected_mission_agent(app).map(|agent| agent.id.clone()) else {
        app.message = "No agent selected.".to_string();
        return;
    };
    if app.terminals.contains_key(&agent_id) {
        app.terminal_input = true;
        app.message = format!("typing into {agent_id}; Esc returns to WM");
    } else {
        app.message = format!("{agent_id} has no terminal yet; press r first");
    }
}

pub(crate) fn send_key_to_focused_terminal(app: &mut MissionControlApp, key: KeyCode) -> Result<()> {
    let agent_id = selected_mission_agent(app)
        .map(|agent| agent.id.clone())
        .ok_or_else(|| anyhow!("No agent selected."))?;
    let pane_id = app
        .terminals
        .get(&agent_id)
        .map(|session| session.tmux_pane.clone())
        .ok_or_else(|| anyhow!("Agent `{agent_id}` has no terminal."))?;
    match key {
        KeyCode::Char(ch) => {
            let text = ch.to_string();
            tmux_command(["send-keys", "-t", pane_id.as_str(), "-l", text.as_str()])?;
        }
        KeyCode::Enter => {
            tmux_command(["send-keys", "-t", pane_id.as_str(), "Enter"])?;
        }
        KeyCode::Backspace => {
            tmux_command(["send-keys", "-t", pane_id.as_str(), "BSpace"])?;
        }
        KeyCode::Tab => {
            tmux_command(["send-keys", "-t", pane_id.as_str(), "Tab"])?;
        }
        KeyCode::Left => {
            tmux_command(["send-keys", "-t", pane_id.as_str(), "Left"])?;
        }
        KeyCode::Right => {
            tmux_command(["send-keys", "-t", pane_id.as_str(), "Right"])?;
        }
        KeyCode::Up => {
            tmux_command(["send-keys", "-t", pane_id.as_str(), "Up"])?;
        }
        KeyCode::Down => {
            tmux_command(["send-keys", "-t", pane_id.as_str(), "Down"])?;
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn start_visible_agent_terminals_from_ui(project: &Path, app: &mut MissionControlApp) {
    let visible_agent_ids = visible_agent_indexes(app)
        .into_iter()
        .map(|index| app.mission.agents[index].id.clone())
        .collect::<Vec<_>>();
    let mut started = 0usize;
    let mut first_error = None;
    for agent_id in visible_agent_ids {
        if app.terminals.contains_key(&agent_id) {
            continue;
        }
        match start_agent_terminal(project, app, &agent_id) {
            Ok(()) => started += 1,
            Err(error) => {
                first_error = Some(format!("{agent_id}: {error}"));
                break;
            }
        }
    }
    if let Some(error) = first_error {
        app.message = format!("started {started} GSD terminal(s), failed: {error}");
    } else {
        app.message = format!("started {started} GSD terminal(s); use arrows to focus, i to type");
    }
}

pub(crate) fn run_visible_agents_from_ui(project: &Path, app: &mut MissionControlApp) {
    let visible = visible_agent_indexes(app);
    let profile = app.fast_profile.then(|| "fast".to_string());
    let mut ran = 0usize;
    for index in visible {
        let agent_id = app.mission.agents[index].id.clone();
        if app.mission.agents[index].status != MissionAgentStatus::Pending {
            continue;
        }
        if let Err(error) = run_agent_loop(project, &mut app.mission, &agent_id, profile.clone(), app.fast_profile) {
            app.message = format!("agent {agent_id} run failed: {error}");
            let _ = write_mission(project, &app.mission);
            return;
        }
        ran += 1;
    }
    match write_mission(project, &app.mission) {
        Ok(()) => app.message = format!("ran {ran} visible pending agent(s)"),
        Err(error) => app.message = format!("saving mission failed: {error}"),
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

pub(crate) fn refresh_tmux_terminal_outputs(app: &mut MissionControlApp) {
    for session in app.terminals.values_mut() {
        match tmux_capture_pane(&session.tmux_pane) {
            Ok(output) => session.output = output,
            Err(error) => {
                session.output = format!("tmux capture failed: {error}");
            }
        }
    }
}

pub(crate) fn ensure_tmux_available() -> Result<()> {
    let output = Command::new("tmux")
        .arg("-V")
        .output()
        .context("tmux is required for Patchbay live agent terminals, but `tmux -V` could not run")?;
    if output.status.success() {
        Ok(())
    } else {
        bail!(
            "tmux is required for Patchbay live agent terminals: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
}

pub(crate) fn tmux_command<const N: usize>(args: [&str; N]) -> Result<String> {
    let output = Command::new("tmux")
        .args(args)
        .output()
        .context("failed to execute tmux")?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        bail!(
            "tmux command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
}

pub(crate) fn tmux_has_session(session: &str) -> bool {
    Command::new("tmux")
        .args(["has-session", "-t", session])
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn tmux_pane_id(target: &str) -> Result<String> {
    tmux_command(["display-message", "-p", "-t", target, "#{pane_id}"])
}

pub(crate) fn tmux_capture_pane(pane_id: &str) -> Result<String> {
    tmux_command(["capture-pane", "-t", pane_id, "-p", "-S", "-200"])
}

pub(crate) fn tmux_session_name(mission_id: &str) -> String {
    format!("patchbay-{}", sanitize_tmux_name(mission_id))
}

pub(crate) fn tmux_window_name(agent_id: &str) -> String {
    sanitize_tmux_name(agent_id)
}

pub(crate) fn sanitize_tmux_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

pub(crate) fn tmux_agent_shell_command(
    control_root: &Path,
    mission_id: &str,
    agent: &MissionAgent,
    profile: &str,
) -> String {
    let mut env_parts = vec![
        ("PATCHBAY_HOME", control_root.display().to_string()),
        ("PATCHBAY_MISSION_ID", mission_id.to_string()),
        ("PATCHBAY_AGENT_ID", agent.id.clone()),
        ("PATCHBAY_PROJECT", agent.project.clone()),
        ("PATCHBAY_PROFILE", profile.to_string()),
    ];
    if let Some(parent_id) = &agent.parent_id {
        env_parts.push(("PATCHBAY_PARENT_AGENT_ID", parent_id.clone()));
    }
    let env = env_parts
        .into_iter()
        .map(|(key, value)| format!("{key}={}", shell_quote(&value)))
        .collect::<Vec<_>>()
        .join(" ");

    let command = if let Ok(runner_cmd) = env::var("PATCHBAY_RUNNER_CMD") {
        format!("exec sh -lc {}", shell_quote(&runner_cmd))
    } else {
        let mut parts = vec!["exec".to_string(), "gsd".to_string()];
        if let Some(model) = runner_model_for_profile(profile) {
            parts.push("--model".to_string());
            parts.push(shell_quote(&model));
        }
        parts.push(shell_quote(&agent.prompt));
        parts.join(" ")
    };

    format!("{env} {command}")
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(crate) fn mission_control_help() -> String {
    "keys: q quit · arrows focus tiles · c compose+launch GSD · i type into focused GSD · esc WM mode · r refresh · s start focused · enter child workspace".to_string()
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
