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
    let missions = list_missions(project).unwrap_or_default();
    let selected_mission = missions.len().saturating_sub(1);
    let mission = match mission_id {
        Some(id) => read_mission(project, id)?,
        None => missions
            .get(selected_mission)
            .cloned()
            .unwrap_or_else(empty_mission),
    };
    let settings = load_tui_settings(project);
    let theme_names = available_theme_names(project);
    let selected_theme = theme_names
        .iter()
        .position(|name| name == &settings.theme_name)
        .unwrap_or(0);
    let mut app = MissionControlApp {
        mission_id: mission.id.clone(),
        mission,
        missions,
        selected_mission,
        focus: if mission_id.is_some() {
            MissionControlPanel::Agents
        } else {
            MissionControlPanel::Main
        },
        selected_agent: 0,
        selected_diff: 0,
        workspace_parent: None,
        workspace_layout: MissionWorkspaceLayout::Auto,
        workspace_layout_anchor: None,
        projects: discover_revision_projects(project, local_project),
        selected_project: 0,
        theme_names,
        selected_theme,
        resources: Vec::new(),
        selected_resource: 0,
        last_resource_refresh: Instant::now(),
        last_auto_stop_check: Instant::now(),
        auto_stop_idle_secs: configured_auto_stop_idle_secs(),
        launch_revision_project: None,
        compose_input: String::new(),
        compose_cursor: 0,
        compose_agent_target: None,
        chat_input: String::new(),
        model_options: configured_model_options(),
        selected_model: 0,
        terminal_input: false,
        terminals: BTreeMap::new(),
        last_refresh: Instant::now(),
        message: mission_control_help(),
        settings,
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = run_mission_control_loop(project, &mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    result?;
    if let Some(project) = app.launch_revision_project.clone() {
        launch_tui(&project).await?;
    }
    Ok(())
}

pub(crate) fn configured_model_options() -> Vec<String> {
    let mut models = gsd_available_model_options()
        .or_else(|_| Ok::<_, anyhow::Error>(configured_model_options_from_env()))
        .unwrap_or_default();
    if models.is_empty() {
        models = vec!["default".to_string()];
    }
    if !models.iter().any(|model| model == "default") {
        models.insert(0, "default".to_string());
    }
    models
}

pub(crate) fn configured_model_options_from_env() -> Vec<String> {
    env::var("PATCHBAY_MODELS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>()
}

#[derive(Deserialize)]
struct GsdAvailableModel {
    provider: String,
    id: String,
}

pub(crate) fn gsd_available_model_options() -> Result<Vec<String>> {
    let root = gsd_package_root()?;
    let index = root
        .join("packages")
        .join("pi-coding-agent")
        .join("dist")
        .join("index.js");
    if !index.exists() {
        bail!(
            "GSD pi-coding-agent module not found at {}",
            index.display()
        );
    }
    let script = format!(
        r#"
import {{ AuthStorage, ModelRegistry }} from {};
const auth = AuthStorage.create();
const registry = ModelRegistry.create(auth);
registry.refresh();
const models = registry.getAvailable().map((model) => ({{ provider: model.provider, id: model.id }}));
console.log(JSON.stringify(models));
"#,
        js_string_literal(&index.display().to_string())
    );
    let output = Command::new("node")
        .args(["--input-type=module", "-e", script.as_str()])
        .output()
        .context("failed to query GSD model registry")?;
    if !output.status.success() {
        bail!(
            "GSD model registry query failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let models: Vec<GsdAvailableModel> = serde_json::from_slice(&output.stdout)
        .context("failed to parse GSD model registry output")?;
    let mut options = vec!["default".to_string()];
    for model in models {
        let option = gsd_model_option(&model.provider, &model.id);
        if !options.iter().any(|existing| existing == &option) {
            options.push(option);
        }
    }
    Ok(options)
}

pub(crate) fn gsd_model_option(provider: &str, model_id: &str) -> String {
    format!("{provider}/{model_id}")
}

pub(crate) fn gsd_package_root() -> Result<PathBuf> {
    let output = Command::new("which")
        .arg("gsd")
        .output()
        .context("failed to locate gsd binary")?;
    if !output.status.success() {
        bail!("gsd binary not found in PATH");
    }
    let binary = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let loader = fs::canonicalize(&binary)
        .with_context(|| format!("failed to resolve gsd binary `{binary}`"))?;
    let root = loader.parent().and_then(Path::parent).ok_or_else(|| {
        anyhow!(
            "could not derive GSD package root from {}",
            loader.display()
        )
    })?;
    Ok(root.to_path_buf())
}

pub(crate) fn js_string_literal(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

pub(crate) fn selected_runner_profile(app: &MissionControlApp) -> Option<String> {
    let selected = app
        .model_options
        .get(app.selected_model)
        .map(String::as_str)
        .unwrap_or("default");
    (selected != "default").then(|| selected.to_string())
}

pub(crate) fn selected_model_label(app: &MissionControlApp) -> &str {
    app.model_options
        .get(app.selected_model)
        .map(String::as_str)
        .unwrap_or("default")
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
        if app.last_resource_refresh.elapsed() >= Duration::from_secs(1) {
            refresh_mission_resources(app);
        }
        if app.last_auto_stop_check.elapsed() >= Duration::from_secs(30) {
            auto_stop_idle_missions(project, app);
        }
        if mission_agent_runtime_active(app) {
            reconcile_agent_terminals(project, app);
        }
        terminal.draw(|frame| render_mission_control(frame, app))?;

        if event::poll(Duration::from_millis(250))? {
            let key = match event::read()? {
                Event::Key(key) => key,
                Event::Paste(text) => {
                    handle_mission_control_paste(app, text);
                    terminal.clear()?;
                    continue;
                }
                Event::Resize(_, _) => {
                    terminal.clear()?;
                    continue;
                }
                _ => continue,
            };
            if key.kind == KeyEventKind::Release {
                continue;
            }
            if app.terminal_input {
                if key.code == KeyCode::Esc {
                    app.terminal_input = false;
                    app.message = mission_control_help();
                } else if let Err(error) = send_key_to_focused_terminal(app, key.code) {
                    app.message = format!("terminal input failed: {error}");
                }
                continue;
            }
            if handle_agent_chat_key(project, app, key.code, key.modifiers) {
                continue;
            }
            if handle_workspace_layout_key(app, key.code, key.modifiers) {
                continue;
            }
            if app.focus == MissionControlPanel::Composer {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Esc => {
                        app.compose_agent_target = None;
                        app.focus = MissionControlPanel::Main;
                    }
                    KeyCode::Backspace => compose_backspace(app),
                    KeyCode::Left => move_compose_cursor(app, -1),
                    KeyCode::Right => move_compose_cursor(app, 1),
                    KeyCode::Home => app.compose_cursor = 0,
                    KeyCode::End => app.compose_cursor = app.compose_input.chars().count(),
                    KeyCode::Char('m') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        cycle_model(app)
                    }
                    KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        cycle_model(app)
                    }
                    KeyCode::Enter => {
                        let input = app.compose_input.trim().to_string();
                        if input.is_empty() {
                            app.message = "Compose text is empty.".to_string();
                        } else if app.compose_agent_target.is_some() {
                            continue_focused_agent_with_prompt(project, app, input);
                        } else {
                            match compose_mission_command(
                                project,
                                Some(input),
                                None,
                                false,
                                selected_runner_profile(app),
                            )
                            .await
                            {
                                Ok(()) => match latest_mission(project) {
                                    Ok(mission) => {
                                        app.missions = list_missions(project).unwrap_or_default();
                                        app.selected_mission = app.missions.len().saturating_sub(1);
                                        app.mission_id = mission.id.clone();
                                        app.mission = mission;
                                        app.workspace_parent = None;
                                        app.selected_agent = 0;
                                        app.selected_diff = 0;
                                        app.compose_input.clear();
                                        app.compose_cursor = 0;
                                        app.compose_agent_target = None;
                                        app.focus = MissionControlPanel::Agents;
                                        let opened_child_workspace =
                                            enter_selected_agent_workspace(app);
                                        reconcile_agent_terminals(project, app);
                                        if app.terminals.is_empty() {
                                            app.message =
                                                "Mission composed, but no GSD terminals started."
                                                    .to_string();
                                        } else if opened_child_workspace {
                                            app.message = format!(
                                                "Mission composed; opened child workspace and started {} GSD terminal(s).",
                                                app.terminals.len()
                                            );
                                        }
                                    }
                                    Err(error) => {
                                        app.message = format!("compose created no mission: {error}")
                                    }
                                },
                                Err(error) => app.message = format!("compose failed: {error}"),
                            }
                        }
                    }
                    KeyCode::Char(ch) => compose_insert_char(app, ch),
                    KeyCode::Tab => focus_next_mission_panel(app),
                    KeyCode::BackTab => focus_previous_mission_panel(app),
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Esc => {
                        if app.focus == MissionControlPanel::Main {
                            app.message = mission_control_help();
                        } else if !leave_agent_workspace(app) {
                            app.focus = MissionControlPanel::Main;
                            app.workspace_parent = None;
                            app.selected_agent = 0;
                            app.selected_diff = 0;
                            app.terminals.clear();
                            app.message = mission_control_help();
                        }
                    }
                    KeyCode::Tab => focus_next_mission_panel(app),
                    KeyCode::BackTab => focus_previous_mission_panel(app),
                    KeyCode::Char('c') => {
                        app.compose_agent_target = None;
                        app.compose_input.clear();
                        app.compose_cursor = 0;
                        app.focus = MissionControlPanel::Composer;
                    }
                    KeyCode::Char('m') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        cycle_model(app)
                    }
                    KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        cycle_model(app)
                    }
                    KeyCode::Char('m') => begin_agent_followup(app),
                    KeyCode::Char('d') => app.focus = MissionControlPanel::Projects,
                    KeyCode::Char('t') => app.focus = MissionControlPanel::Themes,
                    KeyCode::Char('u') => {
                        refresh_mission_resources(app);
                        app.focus = MissionControlPanel::Resources;
                    }
                    KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                        move_mission_control_selection_for_key(app, key.code)
                    }
                    KeyCode::Char('i') => enter_terminal_input_mode(app),
                    KeyCode::Char('r') => refresh_mission_control(project, app),
                    KeyCode::Char('s') => start_focused_agent_terminal_from_ui(project, app),
                    KeyCode::Char('S') => start_visible_agent_terminals_from_ui(project, app),
                    KeyCode::Char('x') => stop_focused_agent_terminal_from_ui(project, app),
                    KeyCode::Delete | KeyCode::Char('D') => {
                        if app.focus == MissionControlPanel::Main {
                            delete_selected_mission_from_ui(project, app);
                        } else if app.focus == MissionControlPanel::Resources {
                            hard_cleanup_selected_resource_mission(project, app);
                        } else {
                            delete_focused_agent_from_ui(project, app);
                        }
                    }
                    KeyCode::Char('K') => {
                        if app.focus == MissionControlPanel::Resources {
                            hard_cleanup_selected_resource_mission(project, app);
                        } else if app.focus == MissionControlPanel::Main {
                            hard_cleanup_selected_mission(project, app);
                        }
                    }
                    KeyCode::Char('R') => run_visible_agents_from_ui(project, app),
                    KeyCode::Enter => {
                        if app.focus == MissionControlPanel::Main {
                            open_selected_mission_from_list(app);
                        } else if app.focus == MissionControlPanel::Themes {
                            select_mission_control_theme(project, app);
                        } else if app.focus == MissionControlPanel::Projects {
                            if let Some(revision_project) =
                                app.projects.get(app.selected_project).cloned()
                            {
                                suspend_tui(terminal)?;
                                let result = launch_tui(&revision_project).await;
                                resume_tui(terminal)?;
                                terminal.clear()?;
                                match result {
                                    Ok(()) => {
                                        app.message =
                                            "Returned from diffs to Mission Control.".to_string()
                                    }
                                    Err(error) => {
                                        app.message = format!("diff view failed: {error}")
                                    }
                                }
                            } else {
                                app.message = "No initialized revision projects found.".to_string();
                            }
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

        if app.last_refresh.elapsed() > Duration::from_secs(1) {
            refresh_mission_control(project, app);
            refresh_mission_resources(app);
            if mission_agent_runtime_active(app) {
                reconcile_agent_terminals(project, app);
            }
        }
    }
    Ok(())
}

pub(crate) fn normalize_pasted_prompt(text: &str) -> String {
    text.replace("\r\n", " ")
        .replace(['\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn handle_mission_control_paste(app: &mut MissionControlApp, text: String) {
    let text = normalize_pasted_prompt(&text);
    if text.is_empty() {
        return;
    }
    if app.terminal_input {
        if let Err(error) = send_text_to_focused_terminal(app, &text) {
            app.message = format!("terminal paste failed: {error}");
        }
        return;
    }
    if mission_agent_runtime_active(app)
        && matches!(
            app.focus,
            MissionControlPanel::Agents | MissionControlPanel::Detail
        )
    {
        if !app.chat_input.is_empty() {
            app.chat_input.push(' ');
        }
        app.chat_input.push_str(&text);
        app.message = "Pasted into agent chat; press Enter to send.".to_string();
        return;
    }
    if app.focus != MissionControlPanel::Composer {
        app.compose_agent_target = None;
        app.compose_input.clear();
        app.compose_cursor = 0;
        app.focus = MissionControlPanel::Composer;
    }
    if !app.compose_input.is_empty() && app.compose_cursor == app.compose_input.chars().count() {
        compose_insert_char(app, ' ');
    }
    for ch in text.chars() {
        compose_insert_char(app, ch);
    }
    app.message = "Pasted into composer; press Enter to launch.".to_string();
}

pub(crate) fn compose_byte_index(input: &str, cursor: usize) -> usize {
    input
        .char_indices()
        .nth(cursor)
        .map(|(index, _)| index)
        .unwrap_or_else(|| input.len())
}

pub(crate) fn compose_insert_char(app: &mut MissionControlApp, ch: char) {
    let index = compose_byte_index(&app.compose_input, app.compose_cursor);
    app.compose_input.insert(index, ch);
    app.compose_cursor += 1;
}

pub(crate) fn compose_backspace(app: &mut MissionControlApp) {
    if app.compose_cursor == 0 {
        return;
    }
    let remove_start = compose_byte_index(&app.compose_input, app.compose_cursor - 1);
    let remove_end = compose_byte_index(&app.compose_input, app.compose_cursor);
    app.compose_input
        .replace_range(remove_start..remove_end, "");
    app.compose_cursor -= 1;
}

pub(crate) fn move_compose_cursor(app: &mut MissionControlApp, delta: isize) {
    let len = app.compose_input.chars().count() as isize;
    app.compose_cursor = (app.compose_cursor as isize + delta).clamp(0, len) as usize;
}

pub(crate) fn cycle_model(app: &mut MissionControlApp) {
    if app.model_options.is_empty() {
        app.model_options.push("default".to_string());
    }
    app.selected_model = (app.selected_model + 1) % app.model_options.len();
    app.message = format!("model: {}", selected_model_label(app));
}

pub(crate) fn refresh_model_options_from_gsd(app: &mut MissionControlApp) {
    match gsd_available_model_options() {
        Ok(options) if !options.is_empty() => {
            let current = selected_model_label(app).to_string();
            app.model_options = options;
            if let Some(index) = app.model_options.iter().position(|model| model == &current) {
                app.selected_model = index;
            } else {
                app.selected_model = 0;
            }
        }
        Ok(_) => app.message = "GSD returned no available models".to_string(),
        Err(error) => app.message = format!("GSD model query failed: {error}"),
    }
}

pub(crate) fn select_model_option(app: &mut MissionControlApp, requested: &str) -> Result<()> {
    if requested.is_empty() {
        bail!("empty model");
    }
    let index = app
        .model_options
        .iter()
        .position(|model| model == requested || model.rsplit('/').next() == Some(requested))
        .ok_or_else(|| anyhow!("{requested}"))?;
    app.selected_model = index;
    Ok(())
}

pub(crate) fn compact_tile_text(text: &str, max_width: u16) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return String::new();
    }
    let max = max_width.saturating_sub(2).max(12) as usize;
    let mut out = collapsed.chars().take(max).collect::<String>();
    if collapsed.chars().count() > max {
        out.push('…');
    }
    out
}

pub(crate) fn compact_tile_lines<'a>(
    text: &'a str,
    max_width: u16,
    max_lines: usize,
) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| compact_tile_text(line, max_width))
        .filter(|line| !line.is_empty())
        .rev()
        .take(max_lines)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
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
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);

    render_mission_control_header(frame, app, vertical[0]);
    match app.focus {
        MissionControlPanel::Main | MissionControlPanel::Themes => {
            render_mission_home_panel(frame, app, vertical[1])
        }
        MissionControlPanel::Projects => render_mission_projects_panel(frame, app, vertical[1]),
        MissionControlPanel::Resources => render_mission_resources_panel(frame, app, vertical[1]),
        MissionControlPanel::Composer => render_mission_composer_panel(frame, app, vertical[1]),
        _ => render_agent_workspace(frame, app, vertical[1]),
    }
    render_agent_chat(frame, app, vertical[2]);
}

pub(crate) fn render_agent_chat(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let selected = selected_mission_agent(app);
    let active = selected.is_some()
        && matches!(
            app.focus,
            MissionControlPanel::Agents | MissionControlPanel::Detail
        );
    let title = if active {
        format!(
            "CHAT → {}",
            selected
                .map(|agent| agent.id.as_str())
                .unwrap_or("no-agent")
        )
    } else {
        "STATUS".to_string()
    };
    let body = if !app.chat_input.is_empty() {
        format!("› {}", app.chat_input)
    } else if active {
        "type to focused GSD agent; Enter sends; /model /stop /restart /delete /clear".to_string()
    } else if app.message.is_empty() {
        mission_control_help()
    } else {
        app.message.clone()
    };
    frame.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: true })
            .style(
                Style::default()
                    .fg(if active {
                        app.settings.theme.foreground
                    } else {
                        app.settings.theme.muted
                    })
                    .bg(app.settings.theme.background),
            )
            .block(panel_block(&title, &app.settings.theme)),
        area,
    );
}

pub(crate) fn render_mission_resources_panel(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let mut lines = Vec::new();
    let policy = app
        .auto_stop_idle_secs
        .map(format_duration)
        .unwrap_or_else(|| "disabled".to_string());
    lines.push(Line::from(vec![
        Span::styled(
            "auto-stop idle: ",
            Style::default().fg(app.settings.theme.muted),
        ),
        Span::styled(policy, Style::default().fg(app.settings.theme.accent)),
        Span::raw("  "),
        Span::styled(
            "K/D: hard cleanup selected mission",
            Style::default().fg(app.settings.theme.muted),
        ),
    ]));
    lines.push(Line::from(""));

    if app.resources.is_empty() {
        lines.push(Line::from(Span::styled(
            "No active patchbay tmux resources.",
            Style::default().fg(app.settings.theme.muted),
        )));
    } else {
        for (index, resource) in app.resources.iter().enumerate() {
            let selected = index == app.selected_resource;
            let idle = resource
                .idle_secs
                .map(format_duration)
                .unwrap_or_else(|| "unknown".to_string());
            let marker = if selected { "▶" } else { " " };
            let current = if resource.current_mission {
                " current"
            } else {
                ""
            };
            lines.push(Line::from(vec![
                Span::styled(
                    marker,
                    Style::default().fg(if selected {
                        app.settings.theme.accent
                    } else {
                        app.settings.theme.muted
                    }),
                ),
                Span::raw(" "),
                Span::styled(
                    &resource.mission_id,
                    Style::default()
                        .fg(app.settings.theme.foreground)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" / "),
                Span::styled(
                    &resource.agent_id,
                    Style::default().fg(app.settings.theme.accent),
                ),
                Span::styled(current, Style::default().fg(app.settings.theme.muted)),
            ]));
            lines.push(Line::from(format!(
                "   pane={} pid={} cmd={} status={} idle={}",
                resource.pane_id,
                resource
                    .pane_pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "?".to_string()),
                resource.command,
                resource.status,
                idle
            )));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .style(
                Style::default()
                    .fg(app.settings.theme.foreground)
                    .bg(app.settings.theme.background),
            )
            .block(mission_control_block(
                app,
                MissionControlPanel::Resources,
                "RESOURCES",
            )),
        area,
    );
}

pub(crate) fn render_mission_home_panel(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);
    render_mission_list_panel(frame, app, columns[0]);
    render_theme_picker_panel(frame, app, columns[1]);
}

pub(crate) fn render_mission_list_panel(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    if app.missions.is_empty() {
        frame.render_widget(
            Paragraph::new("No missions yet. Compose a mission to begin.")
                .style(
                    Style::default()
                        .fg(app.settings.theme.muted)
                        .bg(app.settings.theme.background),
                )
                .block(mission_control_block(
                    app,
                    MissionControlPanel::Main,
                    "MISSIONS",
                )),
            area,
        );
        return;
    }

    let items = app
        .missions
        .iter()
        .enumerate()
        .map(|(index, mission)| {
            let marker = if index == app.selected_mission {
                "▶"
            } else {
                " "
            };
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(app.settings.theme.accent)),
                Span::raw(" "),
                Span::styled(
                    &mission.id,
                    Style::default().fg(app.settings.theme.foreground),
                ),
                Span::raw("  "),
                Span::styled(
                    format_mission_status(&mission.status),
                    status_style(format_mission_status(&mission.status)),
                ),
                Span::raw(format!("  {:>2} agent(s)  ", mission.agents.len())),
                Span::styled(
                    &mission.updated_at,
                    Style::default().fg(app.settings.theme.muted),
                ),
                Span::raw("  "),
                Span::raw(&mission.title),
            ]))
            .style(
                Style::default()
                    .fg(app.settings.theme.foreground)
                    .bg(app.settings.theme.background),
            )
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        List::new(items)
            .block(mission_control_block(
                app,
                MissionControlPanel::Main,
                "MISSIONS",
            ))
            .style(Style::default().bg(app.settings.theme.background)),
        area,
    );
}

pub(crate) fn render_theme_picker_panel(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let items = app
        .theme_names
        .iter()
        .enumerate()
        .map(|(index, theme_name)| {
            let marker = if index == app.selected_theme {
                "▶"
            } else {
                " "
            };
            let current = if theme_name == &app.settings.theme_name {
                " current"
            } else {
                ""
            };
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(app.settings.theme.accent)),
                Span::raw(" "),
                Span::styled(
                    theme_name,
                    Style::default().fg(if theme_name == &app.settings.theme_name {
                        app.settings.theme.accent
                    } else {
                        app.settings.theme.foreground
                    }),
                ),
                Span::styled(current, Style::default().fg(app.settings.theme.muted)),
            ]))
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        List::new(items)
            .block(mission_control_block(
                app,
                MissionControlPanel::Themes,
                "THEMES",
            ))
            .style(Style::default().bg(app.settings.theme.background)),
        area,
    );
}

pub(crate) fn render_agent_workspace(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    render_agent_tiles(frame, app, area);
}

pub(crate) fn working_spinner() -> &'static str {
    const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
    let index = ((Utc::now().timestamp_millis() / 120) as usize) % FRAMES.len();
    FRAMES[index]
}

pub(crate) fn agent_activity_label(agent: &MissionAgent, has_terminal: bool) -> String {
    match agent.status {
        MissionAgentStatus::Running if has_terminal => format!("{} working", working_spinner()),
        MissionAgentStatus::Running => format!("{} starting", working_spinner()),
        MissionAgentStatus::Pending => "queued".to_string(),
        MissionAgentStatus::Waiting => "stopped".to_string(),
        MissionAgentStatus::Complete => "complete".to_string(),
        MissionAgentStatus::Failed => "failed".to_string(),
        MissionAgentStatus::Blocked => "blocked".to_string(),
    }
}

pub(crate) fn agent_tile_rects(app: &MissionControlApp, area: Rect) -> Vec<(usize, usize, Rect)> {
    let agents = visible_agent_indexes(app);
    if agents.is_empty() {
        return Vec::new();
    }
    if app.workspace_layout == MissionWorkspaceLayout::Auto || agents.len() == 1 {
        return agent_grid_rects(area, &agents, &agents);
    }
    docked_agent_tile_rects(app, area, &agents)
}

pub(crate) fn docked_agent_tile_rects(
    app: &MissionControlApp,
    area: Rect,
    agents: &[usize],
) -> Vec<(usize, usize, Rect)> {
    let selected_visible_index = app.selected_agent.min(agents.len().saturating_sub(1));
    let anchored_visible_index = app
        .workspace_layout_anchor
        .as_deref()
        .and_then(|anchor_id| {
            agents
                .iter()
                .position(|agent_index| app.mission.agents[*agent_index].id == anchor_id)
        })
        .unwrap_or(selected_visible_index);
    let anchored_agent_index = agents[anchored_visible_index];
    let other_agents = agents
        .iter()
        .enumerate()
        .filter_map(|(visible_index, agent_index)| {
            (visible_index != anchored_visible_index).then_some(*agent_index)
        })
        .collect::<Vec<_>>();

    let major = Constraint::Percentage(62);
    let minor = Constraint::Percentage(38);
    let mut rects = Vec::new();
    match app.workspace_layout {
        MissionWorkspaceLayout::FocusLeft => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([major, minor])
                .split(area);
            rects.push((anchored_visible_index, anchored_agent_index, split[0]));
            rects.extend(agent_grid_rects(split[1], agents, &other_agents));
        }
        MissionWorkspaceLayout::FocusRight => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([minor, major])
                .split(area);
            rects.extend(agent_grid_rects(split[0], agents, &other_agents));
            rects.push((anchored_visible_index, anchored_agent_index, split[1]));
        }
        MissionWorkspaceLayout::FocusTop => {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([major, minor])
                .split(area);
            rects.push((anchored_visible_index, anchored_agent_index, split[0]));
            rects.extend(agent_grid_rects(split[1], agents, &other_agents));
        }
        MissionWorkspaceLayout::Auto => rects.extend(agent_grid_rects(area, agents, agents)),
    }
    rects
}

pub(crate) fn agent_grid_rects(
    area: Rect,
    all_agents: &[usize],
    agents: &[usize],
) -> Vec<(usize, usize, Rect)> {
    if agents.is_empty() {
        return Vec::new();
    }
    let cols = if agents.len() <= 3 {
        agents.len().max(1)
    } else {
        (agents.len() as f64).sqrt().ceil() as usize
    };
    let rows = agents.len().div_ceil(cols);
    let row_constraints = vec![Constraint::Ratio(1, rows as u32); rows];
    let col_constraints = vec![Constraint::Ratio(1, cols as u32); cols];
    let row_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(area);
    let mut rects = Vec::new();
    for (tile_index, agent_index) in agents.iter().enumerate() {
        let row = tile_index / cols;
        let col = tile_index % cols;
        let col_areas = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(col_constraints.clone())
            .split(row_areas[row]);
        if let Some(visible_index) = all_agents.iter().position(|index| index == agent_index) {
            rects.push((visible_index, *agent_index, col_areas[col]));
        }
    }
    rects
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
                .block(mission_control_block(
                    app,
                    MissionControlPanel::Agents,
                    "AGENT TILES",
                ))
                .style(
                    Style::default()
                        .fg(app.settings.theme.muted)
                        .bg(app.settings.theme.background),
                ),
            area,
        );
        return;
    }

    if app.workspace_layout == MissionWorkspaceLayout::Auto || agents.len() == 1 {
        render_agent_tile_grid(frame, app, area, &agents);
    } else {
        render_docked_agent_tiles(frame, app, area, &agents);
    }
}

pub(crate) fn render_docked_agent_tiles(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
    agents: &[usize],
) {
    let selected_visible_index = app.selected_agent.min(agents.len().saturating_sub(1));
    let selected_agent_index = agents[selected_visible_index];
    let anchored_visible_index = app
        .workspace_layout_anchor
        .as_deref()
        .and_then(|anchor_id| {
            agents
                .iter()
                .position(|agent_index| app.mission.agents[*agent_index].id == anchor_id)
        })
        .unwrap_or(selected_visible_index);
    let anchored_agent_index = agents[anchored_visible_index];
    let other_agents = agents
        .iter()
        .enumerate()
        .filter_map(|(visible_index, agent_index)| {
            (visible_index != anchored_visible_index).then_some(*agent_index)
        })
        .collect::<Vec<_>>();
    let anchor_selected = anchored_agent_index == selected_agent_index;

    let major = Constraint::Percentage(62);
    let minor = Constraint::Percentage(38);
    match app.workspace_layout {
        MissionWorkspaceLayout::FocusLeft => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([major, minor])
                .split(area);
            render_single_agent_tile(frame, app, split[0], anchored_agent_index, anchor_selected);
            render_agent_tile_grid(frame, app, split[1], &other_agents);
        }
        MissionWorkspaceLayout::FocusRight => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([minor, major])
                .split(area);
            render_agent_tile_grid(frame, app, split[0], &other_agents);
            render_single_agent_tile(frame, app, split[1], anchored_agent_index, anchor_selected);
        }
        MissionWorkspaceLayout::FocusTop => {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([major, minor])
                .split(area);
            render_single_agent_tile(frame, app, split[0], anchored_agent_index, anchor_selected);
            render_agent_tile_grid(frame, app, split[1], &other_agents);
        }
        MissionWorkspaceLayout::Auto => render_agent_tile_grid(frame, app, area, agents),
    }
}

pub(crate) fn render_agent_tile_grid(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
    agents: &[usize],
) {
    if agents.is_empty() {
        return;
    }
    let cols = if agents.len() <= 3 {
        agents.len().max(1)
    } else {
        (agents.len() as f64).sqrt().ceil() as usize
    };
    let rows = agents.len().div_ceil(cols);
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
        let selected = visible_agent_indexes(app)
            .get(app.selected_agent)
            .is_some_and(|selected_index| selected_index == agent_index);
        render_single_agent_tile(frame, app, col_areas[col], *agent_index, selected);
    }
}

pub(crate) fn render_single_agent_tile(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
    agent_index: usize,
    selected: bool,
) {
    let agent = &app.mission.agents[agent_index];
    let child_count = child_agent_indexes(app, &agent.id).len();
    let title = format!("{} {}", if selected { "▶" } else { " " }, agent.id);
    let mut block = panel_block(&title, &app.settings.theme);
    if selected {
        block = block.border_style(Style::default().fg(app.settings.theme.accent));
    }
    let session = app.terminals.get(&agent.id);
    let activity = agent_activity_label(agent, session.is_some());
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                activity,
                Style::default()
                    .fg(if agent.status == MissionAgentStatus::Running {
                        app.settings.theme.accent
                    } else {
                        app.settings.theme.muted
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format_mission_agent_status(&agent.status),
                status_style(format_mission_agent_status(&agent.status)),
            ),
            Span::raw(format!("  {}", agent.project)),
        ]),
        Line::from(Span::styled(
            compact_tile_text(&agent.task, area.width),
            Style::default().fg(app.settings.theme.muted),
        )),
        Line::from(format!(
            "parent: {}  children: {child_count}  profile: {}  preset: {}",
            agent.parent_id.as_deref().unwrap_or("root"),
            agent.runner_profile.as_deref().unwrap_or("default"),
            agent.prompt_preset.as_deref().unwrap_or("implementation")
        )),
    ];
    if let Some(session) = session {
        lines.push(Line::from(Span::styled(
            "─ live GSD feed ─",
            Style::default().fg(app.settings.theme.accent),
        )));
        let max_feed_lines = area.height.saturating_sub(lines.len() as u16 + 3) as usize;
        let feed = compact_tile_lines(&session.output, area.width, max_feed_lines.max(4));
        if feed.is_empty() {
            lines.push(Line::from(Span::styled(
                "starting GSD… awaiting first screen draw",
                Style::default().fg(app.settings.theme.muted),
            )));
        } else {
            for line in feed {
                lines.push(Line::from(line));
            }
        }
    } else if let Some(last_log) = agent.last_log.as_deref() {
        for line in compact_tile_lines(last_log, area.width, 3) {
            lines.push(Line::from(line));
        }
    } else if agent.status == MissionAgentStatus::Running {
        lines.push(Line::from(Span::styled(
            "starting GSD… terminal not attached yet",
            Style::default().fg(app.settings.theme.muted),
        )));
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
        area,
    );
}

pub(crate) fn render_mission_composer_panel(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let followup = app.compose_agent_target.as_deref();
    let mode = followup
        .map(|agent_id| format!("Follow-up to {agent_id}"))
        .unwrap_or_else(|| "New root planner mission".to_string());
    let input_len = app.compose_input.chars().count();
    let cursor = app.compose_cursor.min(input_len);
    let lines = vec![
        Line::from(vec![
            Span::styled("Action: ", label_style(&app.settings.theme)),
            Span::raw(mode),
        ]),
        Line::from(
            "Type the request below. Enter creates the mission; Ctrl+M cycles model; Esc cancels.",
        ),
        Line::from("Example: Fix the login bug, review the billing flow, and add tests."),
        Line::from(vec![
            Span::styled("Model: ", label_style(&app.settings.theme)),
            Span::raw(selected_model_label(app)),
            Span::styled(
                "  (default = GSD default)",
                Style::default().fg(app.settings.theme.muted),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("> ", label_style(&app.settings.theme)),
            Span::raw(app.compose_input.as_str()),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .style(
                Style::default()
                    .fg(app.settings.theme.foreground)
                    .bg(app.settings.theme.background),
            )
            .block(mission_control_block(
                app,
                MissionControlPanel::Composer,
                "C COMPOSE",
            )),
        area,
    );
    if app.focus == MissionControlPanel::Composer {
        let cursor_x = area.x.saturating_add(3).saturating_add(cursor as u16);
        let cursor_y = area.y.saturating_add(5);
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }
}

pub(crate) fn render_mission_projects_panel(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let items = if app.projects.is_empty() {
        vec![ListItem::new(Line::from(vec![
            Span::styled(
                "No initialized revision projects found",
                Style::default().fg(app.settings.theme.muted),
            ),
            Span::raw(" — run `pb init` in a project or add it to config."),
        ]))]
    } else {
        app.projects
            .iter()
            .enumerate()
            .map(|(index, project)| {
                let marker = if index == app.selected_project {
                    "› "
                } else {
                    "  "
                };
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
            .block(mission_control_block(
                app,
                MissionControlPanel::Projects,
                "D PROJECT DIFFS",
            )),
        area,
    );
}

pub(crate) fn render_mission_control_header(
    frame: &mut ratatui::Frame<'_>,
    app: &MissionControlApp,
    area: Rect,
) {
    let title = if app.focus == MissionControlPanel::Main || app.mission.agents.is_empty() {
        Line::from(vec![
            Span::styled(
                "Patchbay",
                Style::default()
                    .fg(app.settings.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  missions:{}", app.missions.len())),
            Span::styled(
                "  press c to compose",
                Style::default().fg(app.settings.theme.muted),
            ),
        ])
    } else {
        let mut spans = vec![
            Span::styled(
                "Patchbay",
                Style::default()
                    .fg(app.settings.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  mission:"),
            Span::raw(&app.mission.id),
            Span::raw("  status:"),
            Span::styled(
                format_mission_status(&app.mission.status),
                status_style(format_mission_status(&app.mission.status)),
            ),
        ];
        if app.workspace_layout != MissionWorkspaceLayout::Auto {
            spans.push(Span::raw("  layout:"));
            spans.push(Span::styled(
                workspace_layout_label(app.workspace_layout),
                Style::default().fg(app.settings.theme.muted),
            ));
        }
        Line::from(spans)
    };
    frame.render_widget(
        Paragraph::new(title).alignment(Alignment::Left).style(
            Style::default()
                .fg(app.settings.theme.foreground)
                .bg(app.settings.theme.background),
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
    match app.workspace_parent.as_deref() {
        None => (0..app.mission.agents.len()).collect(),
        Some(parent_id) => app
            .mission
            .agents
            .iter()
            .enumerate()
            .filter(|(_, agent)| {
                agent.id == parent_id || agent.parent_id.as_deref() == Some(parent_id)
            })
            .map(|(index, _)| index)
            .collect(),
    }
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

pub(crate) fn mission_agent_runtime_active(app: &MissionControlApp) -> bool {
    !matches!(
        app.focus,
        MissionControlPanel::Main | MissionControlPanel::Themes | MissionControlPanel::Composer
    )
}

pub(crate) fn open_selected_mission_from_list(app: &mut MissionControlApp) {
    let Some(mission) = app.missions.get(app.selected_mission).cloned() else {
        app.message = "No mission selected. Press c to compose one.".to_string();
        return;
    };
    app.mission_id = mission.id.clone();
    app.mission = mission;
    app.focus = MissionControlPanel::Agents;
    app.workspace_parent = None;
    app.selected_agent = 0;
    app.selected_diff = 0;
    app.terminals.clear();
    app.message = mission_control_help();
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
    if app.missions.is_empty() {
        app.selected_mission = 0;
    } else {
        app.selected_mission = app.selected_mission.min(app.missions.len() - 1);
    }
    if app.theme_names.is_empty() {
        app.selected_theme = 0;
    } else {
        app.selected_theme = app.selected_theme.min(app.theme_names.len() - 1);
    }
    if app.resources.is_empty() {
        app.selected_resource = 0;
    } else {
        app.selected_resource = app.selected_resource.min(app.resources.len() - 1);
    }

    let visible = visible_agent_indexes(app);
    if visible.is_empty() {
        app.selected_agent = 0;
        app.selected_diff = 0;
        return;
    }
    app.selected_agent = app.selected_agent.min(visible.len() - 1);
    let diff_len = app.mission.agents[visible[app.selected_agent]]
        .diff_refs
        .len();
    if diff_len == 0 {
        app.selected_diff = 0;
    } else {
        app.selected_diff = app.selected_diff.min(diff_len - 1);
    }
}

pub(crate) fn move_mission_control_selection_for_key(app: &mut MissionControlApp, key: KeyCode) {
    let delta = match key {
        KeyCode::Left | KeyCode::Up => -1,
        KeyCode::Right | KeyCode::Down => 1,
        _ => return,
    };
    match app.focus {
        MissionControlPanel::Agents | MissionControlPanel::Detail => {
            move_agent_selection_spatial(app, key);
        }
        _ => move_mission_control_selection(app, delta),
    }
}

pub(crate) fn move_agent_selection_spatial(app: &mut MissionControlApp, key: KeyCode) {
    let rects = agent_tile_rects(app, Rect::new(0, 0, 1000, 1000));
    if rects.is_empty() {
        app.selected_agent = 0;
        app.selected_diff = 0;
        return;
    }
    let Some((_, _, current)) = rects
        .iter()
        .find(|(visible_index, _, _)| *visible_index == app.selected_agent)
        .copied()
    else {
        app.selected_agent = 0;
        app.selected_diff = 0;
        return;
    };
    let current_center = rect_center(current);
    let candidate = rects
        .iter()
        .filter_map(|(visible_index, _, rect)| {
            if *visible_index == app.selected_agent {
                return None;
            }
            let center = rect_center(*rect);
            let in_direction = match key {
                KeyCode::Left => center.0 < current_center.0,
                KeyCode::Right => center.0 > current_center.0,
                KeyCode::Up => center.1 < current_center.1,
                KeyCode::Down => center.1 > current_center.1,
                _ => false,
            };
            if !in_direction {
                return None;
            }
            let primary = match key {
                KeyCode::Left | KeyCode::Right => current_center.0.abs_diff(center.0),
                KeyCode::Up | KeyCode::Down => current_center.1.abs_diff(center.1),
                _ => 0,
            };
            let secondary = match key {
                KeyCode::Left | KeyCode::Right => current_center.1.abs_diff(center.1),
                KeyCode::Up | KeyCode::Down => current_center.0.abs_diff(center.0),
                _ => 0,
            };
            Some((*visible_index, primary, secondary))
        })
        .min_by_key(|(_, primary, secondary)| (*primary, *secondary));

    if let Some((visible_index, _, _)) = candidate {
        app.selected_agent = visible_index;
        app.selected_diff = 0;
    }
}

pub(crate) fn rect_center(rect: Rect) -> (u16, u16) {
    (
        rect.x.saturating_add(rect.width / 2),
        rect.y.saturating_add(rect.height / 2),
    )
}

pub(crate) fn move_mission_control_selection(app: &mut MissionControlApp, delta: isize) {
    match app.focus {
        MissionControlPanel::Main => {
            app.selected_mission = move_index(app.selected_mission, app.missions.len(), delta);
        }
        MissionControlPanel::Agents | MissionControlPanel::Detail => {
            app.selected_agent =
                move_index(app.selected_agent, visible_agent_indexes(app).len(), delta);
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
        MissionControlPanel::Themes => {
            app.selected_theme = move_index(app.selected_theme, app.theme_names.len(), delta);
        }
        MissionControlPanel::Resources => {
            app.selected_resource = move_index(app.selected_resource, app.resources.len(), delta);
        }
        _ => {}
    }
}

pub(crate) fn select_mission_control_theme(project: &Path, app: &mut MissionControlApp) {
    let Some(theme_name) = app.theme_names.get(app.selected_theme).cloned() else {
        app.message = "No themes available.".to_string();
        return;
    };
    match write_theme_selection(&project_config_path(project), &theme_name) {
        Ok(()) => {
            app.settings = load_tui_settings(project);
            app.theme_names = available_theme_names(project);
            app.selected_theme = app
                .theme_names
                .iter()
                .position(|name| name == &app.settings.theme_name)
                .unwrap_or(0);
            app.message = format!("theme set to {theme_name}");
        }
        Err(error) => app.message = format!("theme change failed: {error}"),
    }
}

pub(crate) fn move_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let last = len.saturating_sub(1) as isize;
    (current as isize + delta).clamp(0, last) as usize
}

pub(crate) fn handle_workspace_layout_key(
    app: &mut MissionControlApp,
    key: KeyCode,
    modifiers: KeyModifiers,
) -> bool {
    if !modifiers.contains(KeyModifiers::CONTROL)
        || app.focus == MissionControlPanel::Main
        || app.focus == MissionControlPanel::Composer
        || app.focus == MissionControlPanel::Projects
        || app.focus == MissionControlPanel::Themes
        || app.focus == MissionControlPanel::Resources
    {
        return false;
    }
    if key == KeyCode::Char('0') {
        app.workspace_layout = MissionWorkspaceLayout::Auto;
        app.workspace_layout_anchor = None;
        app.message = "workspace layout: auto".to_string();
        return true;
    }
    let Some(anchor_agent_id) = selected_mission_agent(app).map(|agent| agent.id.clone()) else {
        return false;
    };
    app.workspace_layout = match key {
        KeyCode::Left => MissionWorkspaceLayout::FocusLeft,
        KeyCode::Right => MissionWorkspaceLayout::FocusRight,
        KeyCode::Up => MissionWorkspaceLayout::FocusTop,
        _ => return false,
    };
    app.workspace_layout_anchor = Some(anchor_agent_id.clone());
    app.message = format!(
        "workspace layout: {} anchored to {anchor_agent_id}",
        workspace_layout_label(app.workspace_layout)
    );
    true
}

pub(crate) fn workspace_layout_label(layout: MissionWorkspaceLayout) -> &'static str {
    match layout {
        MissionWorkspaceLayout::Auto => "auto",
        MissionWorkspaceLayout::FocusLeft => "focus left",
        MissionWorkspaceLayout::FocusRight => "focus right",
        MissionWorkspaceLayout::FocusTop => "focus top",
    }
}

pub(crate) fn focus_next_mission_panel(app: &mut MissionControlApp) {
    app.focus = match app.focus {
        MissionControlPanel::Main => MissionControlPanel::Themes,
        MissionControlPanel::Themes => MissionControlPanel::Resources,
        MissionControlPanel::Resources => MissionControlPanel::Agents,
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
        MissionControlPanel::Themes => MissionControlPanel::Main,
        MissionControlPanel::Resources => MissionControlPanel::Themes,
        MissionControlPanel::Agents => MissionControlPanel::Resources,
        MissionControlPanel::Detail => MissionControlPanel::Agents,
        MissionControlPanel::Diffs => MissionControlPanel::Detail,
        MissionControlPanel::Projects => MissionControlPanel::Diffs,
        MissionControlPanel::Composer => MissionControlPanel::Projects,
    };
}

pub(crate) fn refresh_mission_control(project: &Path, app: &mut MissionControlApp) {
    match list_missions(project) {
        Ok(missions) => {
            app.missions = missions;
            clamp_mission_control_selection(app);
        }
        Err(error) => {
            app.message = format!("mission list refresh failed: {error}");
        }
    }

    app.theme_names = available_theme_names(project);
    if app.theme_names.is_empty() {
        app.selected_theme = 0;
    } else {
        app.selected_theme = app.selected_theme.min(app.theme_names.len() - 1);
    }

    if matches!(
        app.focus,
        MissionControlPanel::Main | MissionControlPanel::Themes
    ) {
        app.last_refresh = Instant::now();
        return;
    }

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
    if let Some(session) = app.terminals.get(&agent_id) {
        match tmux_pane_dead_status(&session.tmux_pane) {
            Ok(Some(_)) => {
                app.terminals.remove(&agent_id);
            }
            _ => {
                app.message =
                    format!("agent {agent_id} already has a live terminal; press i to type");
                return;
            }
        }
    }
    match start_agent_terminal(project, app, &agent_id) {
        Ok(()) => {
            app.terminal_input = true;
            app.message =
                format!("agent {agent_id} terminal started; typing goes to GSD, Esc returns to WM");
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
    let selected_profile = selected_model_label(app).to_string();
    let agent = app
        .mission
        .agents
        .iter_mut()
        .find(|agent| agent.id == agent_id)
        .ok_or_else(|| anyhow!("Mission has no agent `{agent_id}`"))?;
    let profile = agent
        .runner_profile
        .as_deref()
        .unwrap_or(selected_profile.as_str());
    let session_name = tmux_session_name(&app.mission.id);
    let window_name = tmux_window_name(&agent.id);
    let log_path = tmux_agent_log_path(project, &app.mission.id, &agent.id);
    let prompt_path = tmux_agent_prompt_path(project, &app.mission.id, &agent.id);
    write_agent_prompt(&prompt_path, &agent.prompt)?;
    let shell_command = tmux_agent_shell_command(project, &app.mission.id, agent, profile);
    prepare_agent_log(&log_path, &prompt_path, agent, &shell_command)?;
    let shell_command = if should_submit_prompt_to_interactive_gsd() {
        shell_command
    } else {
        tmux_agent_logged_shell_command(&shell_command, &log_path)
    };

    if tmux_has_session(&session_name) {
        kill_tmux_agent_windows(&session_name, &window_name)?;
    }
    let has_session = tmux_has_session(&session_name);
    let pane_id = if !has_session {
        tmux_command([
            "new-session",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-s",
            session_name.as_str(),
            "-n",
            window_name.as_str(),
            "-c",
            agent.project_path.as_str(),
            "sleep 86400",
        ])?
    } else {
        tmux_command([
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            session_name.as_str(),
            "-n",
            window_name.as_str(),
            "-c",
            agent.project_path.as_str(),
            "sleep 86400",
        ])?
    };
    let pane_id = pane_id.trim().to_string();
    tmux_prepare_agent_pane(&pane_id, &agent.project_path, &shell_command)?;
    if should_submit_prompt_to_interactive_gsd() {
        tmux_submit_agent_prompt(&pane_id, &agent.prompt)?;
    }

    agent.status = MissionAgentStatus::Running;
    agent.runner_profile = Some(profile.to_string());
    agent.session_id = Some(format!("tmux:{session_name}:{pane_id}"));
    agent.started_at = Some(Utc::now().to_rfc3339());
    agent.finished_at = None;
    agent.last_error = None;
    agent.updated_at = Utc::now().to_rfc3339();
    refresh_mission_status(&mut app.mission);
    write_mission(project, &app.mission)?;

    let output = read_agent_live_output(&pane_id, &log_path).unwrap_or_default();
    app.terminals.insert(
        agent_id.to_string(),
        AgentTerminalSession {
            tmux_pane: pane_id,
            log_path,
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
    if let Some(session) = app.terminals.get(&agent_id) {
        match tmux_pane_dead_status(&session.tmux_pane) {
            Ok(Some(_)) => {
                app.message =
                    format!("{agent_id} pane is dead; press m to send a follow-up or s to restart");
            }
            _ => {
                app.terminal_input = true;
                app.message = format!("typing into {agent_id}; Esc returns to WM");
            }
        }
    } else {
        app.message = format!("{agent_id} has no terminal yet; press s first");
    }
}

pub(crate) fn stop_focused_agent_terminal_from_ui(project: &Path, app: &mut MissionControlApp) {
    let Some(agent_id) = selected_mission_agent(app).map(|agent| agent.id.clone()) else {
        app.message = "No agent selected.".to_string();
        return;
    };
    if stop_agent_terminal(project, app, &agent_id) {
        app.message = format!("stopped {agent_id}; press s to restart");
    } else {
        app.message = format!("{agent_id} has no live terminal to stop");
    }
}

pub(crate) fn stop_agent_terminal(
    project: &Path,
    app: &mut MissionControlApp,
    agent_id: &str,
) -> bool {
    let mut stopped = false;
    let session_name = tmux_session_name(&app.mission.id);
    if let Some(session) = app.terminals.remove(agent_id) {
        let _ = hard_cleanup_tmux_pane(&session.tmux_pane);
        stopped = true;
    }
    if let Some(agent) = app
        .mission
        .agents
        .iter_mut()
        .find(|agent| agent.id == agent_id)
    {
        if let Some((_, pane_id)) = agent.session_id.as_deref().and_then(parse_tmux_session_id) {
            let _ = hard_cleanup_tmux_pane(&pane_id);
            stopped = true;
        }
        let _ = kill_tmux_agent_windows(&session_name, agent_id);
        if agent.session_id.is_some() || agent.status == MissionAgentStatus::Running {
            agent.session_id = None;
            agent.status = MissionAgentStatus::Waiting;
            agent.started_at = None;
            agent.finished_at = Some(Utc::now().to_rfc3339());
            agent.last_error = Some("stopped by user".to_string());
            agent.updated_at = Utc::now().to_rfc3339();
            stopped = true;
        }
    }
    if stopped {
        refresh_mission_status(&mut app.mission);
        let _ = write_mission(project, &app.mission);
    }
    stopped
}

pub(crate) fn hard_cleanup_selected_mission(project: &Path, app: &mut MissionControlApp) {
    let Some(mission) = app.missions.get(app.selected_mission).cloned() else {
        app.message = "No mission selected.".to_string();
        return;
    };
    hard_cleanup_mission_by_id(project, app, &mission.id, true);
}

pub(crate) fn hard_cleanup_selected_resource_mission(project: &Path, app: &mut MissionControlApp) {
    let Some(resource) = app.resources.get(app.selected_resource).cloned() else {
        app.message = "No resource selected.".to_string();
        return;
    };
    hard_cleanup_mission_by_id(project, app, &resource.mission_id, false);
}

pub(crate) fn hard_cleanup_mission_by_id(
    project: &Path,
    app: &mut MissionControlApp,
    mission_id: &str,
    delete_file: bool,
) {
    let session_name = tmux_session_name(mission_id);
    let _ = hard_cleanup_tmux_session(&session_name);
    app.terminals.clear();
    if delete_file {
        match mission_path(project, mission_id).and_then(|path| {
            fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))
        }) {
            Ok(()) => app.message = format!("hard-cleaned and deleted mission {mission_id}"),
            Err(error) => app.message = format!("hard cleanup ran, delete failed: {error}"),
        }
    } else {
        mark_mission_stopped(project, mission_id, "hard-cleaned by user");
        app.message = format!("hard-cleaned mission {mission_id}");
    }
    app.missions = list_missions(project).unwrap_or_default();
    if app.missions.is_empty() {
        app.mission_id = "no-mission".to_string();
        app.mission = empty_mission();
        app.selected_mission = 0;
    } else if let Some(index) = app
        .missions
        .iter()
        .position(|mission| mission.id == app.mission_id)
    {
        app.selected_mission = index;
        app.mission = app.missions[index].clone();
    } else {
        app.selected_mission = app.selected_mission.min(app.missions.len() - 1);
        app.mission_id = app.missions[app.selected_mission].id.clone();
        app.mission = app.missions[app.selected_mission].clone();
    }
    refresh_mission_resources(app);
}

pub(crate) fn delete_selected_mission_from_ui(project: &Path, app: &mut MissionControlApp) {
    let Some(mission) = app.missions.get(app.selected_mission).cloned() else {
        app.message = "No mission selected.".to_string();
        return;
    };
    let session_name = tmux_session_name(&mission.id);
    let _ = hard_cleanup_tmux_session(&session_name);
    let path = match mission_path(project, &mission.id) {
        Ok(path) => path,
        Err(error) => {
            app.message = format!("delete mission failed: {error}");
            return;
        }
    };
    if let Err(error) = fs::remove_file(&path) {
        app.message = format!("delete mission failed: {error}");
        return;
    }
    app.terminals.clear();
    app.missions = list_missions(project).unwrap_or_default();
    if app.missions.is_empty() {
        app.mission_id = "no-mission".to_string();
        app.mission = empty_mission();
        app.selected_mission = 0;
    } else {
        app.selected_mission = app.selected_mission.min(app.missions.len() - 1);
        if let Some(next) = app.missions.get(app.selected_mission).cloned() {
            app.mission_id = next.id.clone();
            app.mission = next;
        }
    }
    app.workspace_parent = None;
    app.selected_agent = 0;
    app.selected_diff = 0;
    app.message = format!("deleted mission {}", mission.id);
}

pub(crate) fn delete_focused_agent_from_ui(project: &Path, app: &mut MissionControlApp) {
    let Some(agent_id) = selected_mission_agent(app).map(|agent| agent.id.clone()) else {
        app.message = "No agent selected.".to_string();
        return;
    };
    let removed = delete_agent_subtree(project, app, &agent_id);
    if removed == 0 {
        app.message = format!("{agent_id} was not found.");
        return;
    }
    if app.workspace_parent.as_deref() == Some(agent_id.as_str())
        || app
            .workspace_parent
            .as_ref()
            .is_some_and(|parent| !app.mission.agents.iter().any(|agent| &agent.id == parent))
    {
        app.workspace_parent = None;
    }
    app.selected_agent = 0;
    app.selected_diff = 0;
    refresh_mission_status(&mut app.mission);
    if let Err(error) = write_mission(project, &app.mission) {
        app.message = format!("deleted {removed} agent(s), but save failed: {error}");
        return;
    }
    app.message = format!("deleted {removed} agent(s) from mission");
}

pub(crate) fn delete_agent_subtree(
    project: &Path,
    app: &mut MissionControlApp,
    root_id: &str,
) -> usize {
    let mut remove_ids = BTreeSet::from([root_id.to_string()]);
    loop {
        let before = remove_ids.len();
        for agent in &app.mission.agents {
            if agent
                .parent_id
                .as_ref()
                .is_some_and(|parent| remove_ids.contains(parent))
            {
                remove_ids.insert(agent.id.clone());
            }
        }
        if remove_ids.len() == before {
            break;
        }
    }
    for agent_id in &remove_ids {
        let _ = stop_agent_terminal(project, app, agent_id);
    }
    let before = app.mission.agents.len();
    app.mission
        .agents
        .retain(|agent| !remove_ids.contains(&agent.id));
    before - app.mission.agents.len()
}

pub(crate) fn begin_agent_followup(app: &mut MissionControlApp) {
    let Some(agent_id) = selected_mission_agent(app).map(|agent| agent.id.clone()) else {
        app.message = "No agent selected.".to_string();
        return;
    };
    app.compose_agent_target = Some(agent_id.clone());
    app.compose_input.clear();
    app.focus = MissionControlPanel::Composer;
    app.message = format!("follow-up for {agent_id}; Enter runs it, Esc cancels");
}

pub(crate) fn continue_focused_agent_with_prompt(
    project: &Path,
    app: &mut MissionControlApp,
    followup: String,
) {
    let Some(agent_id) = app.compose_agent_target.clone() else {
        app.message = "No follow-up target selected.".to_string();
        return;
    };
    let Some(agent) = app
        .mission
        .agents
        .iter_mut()
        .find(|agent| agent.id == agent_id)
    else {
        app.message = format!("agent {agent_id} no longer exists");
        return;
    };

    agent.task = format!("{}\n\nFollow-up:\n{}", agent.task, followup);
    agent.prompt = format!(
        "{}\n\nPatchbay follow-up from Mission Control:\n{}",
        agent.prompt, followup
    );
    agent.status = MissionAgentStatus::Pending;
    agent.summary = None;
    agent.last_error = None;
    agent.session_id = None;
    agent.finished_at = None;
    agent.updated_at = Utc::now().to_rfc3339();
    app.terminals.remove(&agent_id);
    refresh_mission_status(&mut app.mission);
    if let Err(error) = write_mission(project, &app.mission) {
        app.message = format!("failed to save follow-up: {error}");
        return;
    }

    app.compose_input.clear();
    app.compose_agent_target = None;
    app.focus = MissionControlPanel::Agents;
    match start_agent_terminal(project, app, &agent_id) {
        Ok(()) => app.message = format!("follow-up started for {agent_id}"),
        Err(error) => {
            app.message = format!("follow-up saved but start failed for {agent_id}: {error}")
        }
    }
}

pub(crate) fn handle_agent_chat_key(
    project: &Path,
    app: &mut MissionControlApp,
    key: KeyCode,
    modifiers: KeyModifiers,
) -> bool {
    if !mission_agent_runtime_active(app)
        || !matches!(
            app.focus,
            MissionControlPanel::Agents | MissionControlPanel::Detail
        )
    {
        return false;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        return false;
    }
    match key {
        KeyCode::Char(ch) => {
            app.chat_input.push(ch);
            true
        }
        KeyCode::Backspace => {
            app.chat_input.pop();
            true
        }
        KeyCode::Enter => {
            send_chat_to_focused_agent(project, app);
            true
        }
        KeyCode::Esc if !app.chat_input.is_empty() => {
            app.chat_input.clear();
            true
        }
        _ => false,
    }
}

pub(crate) fn send_chat_to_focused_agent(project: &Path, app: &mut MissionControlApp) {
    let text = app.chat_input.trim().to_string();
    if text.is_empty() {
        app.message = "chat input is empty".to_string();
        return;
    }
    let Some(agent_id) = selected_mission_agent(app).map(|agent| agent.id.clone()) else {
        app.message = "No agent selected.".to_string();
        return;
    };
    if handle_agent_chat_command(project, app, &agent_id, &text) {
        app.chat_input.clear();
        return;
    }
    if !app.terminals.contains_key(&agent_id) {
        if let Err(error) = start_agent_terminal(project, app, &agent_id) {
            app.message = format!("could not start {agent_id}: {error}");
            return;
        }
    }
    let Some(pane_id) = app
        .terminals
        .get(&agent_id)
        .map(|session| session.tmux_pane.clone())
    else {
        app.message = format!("{agent_id} has no live terminal");
        return;
    };
    match tmux_command(["send-keys", "-t", pane_id.as_str(), "-l", text.as_str()])
        .and_then(|_| tmux_command(["send-keys", "-t", pane_id.as_str(), "Enter"]))
    {
        Ok(_) => {
            app.chat_input.clear();
            app.message = format!("sent message to {agent_id}");
        }
        Err(error) => app.message = format!("send to {agent_id} failed: {error}"),
    }
}

pub(crate) fn handle_agent_chat_command(
    project: &Path,
    app: &mut MissionControlApp,
    agent_id: &str,
    text: &str,
) -> bool {
    match text.trim() {
        "/clear" => {
            app.message = "chat input cleared".to_string();
            true
        }
        "/model" => {
            refresh_model_options_from_gsd(app);
            app.message = format!("models: {}", app.model_options.join(", "));
            true
        }
        "/stop" => {
            if stop_agent_terminal(project, app, agent_id) {
                app.message = format!("stopped {agent_id}");
            } else {
                app.message = format!("{agent_id} has no live terminal to stop");
            }
            true
        }
        "/restart" => {
            let _ = stop_agent_terminal(project, app, agent_id);
            match start_agent_terminal(project, app, agent_id) {
                Ok(()) => app.message = format!("restarted {agent_id}"),
                Err(error) => app.message = format!("restart {agent_id} failed: {error}"),
            }
            true
        }
        "/delete" => {
            let removed = delete_agent_subtree(project, app, agent_id);
            refresh_mission_status(&mut app.mission);
            match write_mission(project, &app.mission) {
                Ok(()) => app.message = format!("deleted {removed} agent(s)"),
                Err(error) => {
                    app.message = format!("delete saved in memory but write failed: {error}")
                }
            }
            true
        }
        command if command.starts_with("/model ") => {
            let requested = command.trim_start_matches("/model ").trim();
            refresh_model_options_from_gsd(app);
            match select_model_option(app, requested) {
                Ok(()) => app.message = format!("model: {}", selected_model_label(app)),
                Err(error) => app.message = format!("model not found: {error}"),
            }
            true
        }
        _ => false,
    }
}

pub(crate) fn send_text_to_focused_terminal(app: &mut MissionControlApp, text: &str) -> Result<()> {
    let agent_id = selected_mission_agent(app)
        .map(|agent| agent.id.clone())
        .ok_or_else(|| anyhow!("No agent selected."))?;
    let pane_id = app
        .terminals
        .get(&agent_id)
        .map(|session| session.tmux_pane.clone())
        .ok_or_else(|| anyhow!("Agent `{agent_id}` has no terminal."))?;
    tmux_command(["send-keys", "-t", pane_id.as_str(), "-l", text])?;
    Ok(())
}

pub(crate) fn send_key_to_focused_terminal(
    app: &mut MissionControlApp,
    key: KeyCode,
) -> Result<()> {
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
        .filter(|index| app.mission.agents[*index].status == MissionAgentStatus::Pending)
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
    let profile = selected_runner_profile(app);
    let mut ran = 0usize;
    for index in visible {
        let agent_id = app.mission.agents[index].id.clone();
        if app.mission.agents[index].status != MissionAgentStatus::Pending {
            continue;
        }
        if let Err(error) = run_agent_loop(project, &mut app.mission, &agent_id, profile.clone()) {
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

pub(crate) fn reconcile_agent_terminals(project: &Path, app: &mut MissionControlApp) {
    reconnect_tmux_terminal_sessions(project, app);
    refresh_tmux_terminal_outputs(project, app);
    monitor_tmux_lifecycle(project, app);
    auto_start_pending_agent_terminals(project, app);
}

pub(crate) fn reconnect_tmux_terminal_sessions(project: &Path, app: &mut MissionControlApp) {
    let sessions = app
        .mission
        .agents
        .iter()
        .filter_map(|agent| {
            if app.terminals.contains_key(&agent.id) {
                return None;
            }
            let session_id = agent.session_id.as_deref()?;
            let (_session_name, pane_id) = parse_tmux_session_id(session_id)?;
            let log_path = tmux_agent_log_path(project, &app.mission.id, &agent.id);
            let output = read_agent_live_output(&pane_id, &log_path).ok()?;
            Some((
                agent.id.clone(),
                AgentTerminalSession {
                    tmux_pane: pane_id,
                    log_path,
                    output,
                },
            ))
        })
        .collect::<Vec<_>>();
    for (agent_id, session) in sessions {
        app.terminals.insert(agent_id, session);
    }
}

pub(crate) fn monitor_tmux_lifecycle(project: &Path, app: &mut MissionControlApp) {
    let mut changed = false;
    let agent_ids = app.terminals.keys().cloned().collect::<Vec<_>>();
    for agent_id in agent_ids {
        let Some(session) = app.terminals.get(&agent_id) else {
            continue;
        };
        let Ok(Some(status)) = tmux_pane_dead_status(&session.tmux_pane) else {
            continue;
        };
        let Some(agent) = app
            .mission
            .agents
            .iter_mut()
            .find(|agent| agent.id == agent_id)
        else {
            continue;
        };
        if matches!(
            agent.status,
            MissionAgentStatus::Complete | MissionAgentStatus::Failed | MissionAgentStatus::Blocked
        ) {
            continue;
        }
        agent.status = if status == 0 {
            MissionAgentStatus::Complete
        } else {
            MissionAgentStatus::Failed
        };
        agent.finished_at = Some(Utc::now().to_rfc3339());
        agent.updated_at = Utc::now().to_rfc3339();
        if status != 0 {
            agent.last_error = Some(format!("tmux pane exited with status {status}"));
        }
        changed = true;
    }
    if changed {
        refresh_mission_status(&mut app.mission);
        let _ = write_mission(project, &app.mission);
    }
}

pub(crate) fn auto_start_pending_agent_terminals(project: &Path, app: &mut MissionControlApp) {
    let pending = app
        .mission
        .agents
        .iter()
        .filter(|agent| agent.status == MissionAgentStatus::Pending)
        .filter(|agent| !app.terminals.contains_key(&agent.id))
        .map(|agent| agent.id.clone())
        .collect::<Vec<_>>();
    if pending.is_empty() {
        return;
    }

    let mut started = 0usize;
    for agent_id in pending {
        match start_agent_terminal(project, app, &agent_id) {
            Ok(()) => started += 1,
            Err(error) => {
                app.message = format!("auto-start failed for {agent_id}: {error}");
                break;
            }
        }
    }
    if started > 0 {
        app.message = format!("auto-started {started} pending GSD terminal(s)");
    }
}

pub(crate) fn refresh_tmux_terminal_outputs(project: &Path, app: &mut MissionControlApp) {
    let agent_ids = app.terminals.keys().cloned().collect::<Vec<_>>();
    let mut changed = false;
    for agent_id in agent_ids {
        let Some(session) = app.terminals.get_mut(&agent_id) else {
            continue;
        };
        match read_agent_live_output(&session.tmux_pane, &session.log_path) {
            Ok(output) => session.output = output,
            Err(error) => {
                app.terminals.remove(&agent_id);
                if let Some(agent) = app
                    .mission
                    .agents
                    .iter_mut()
                    .find(|agent| agent.id == agent_id)
                {
                    agent.session_id = None;
                    if agent.status == MissionAgentStatus::Running {
                        agent.status = MissionAgentStatus::Pending;
                        agent.started_at = None;
                    }
                    agent.last_error = Some(format!("tmux pane unavailable: {error}"));
                    agent.updated_at = Utc::now().to_rfc3339();
                    changed = true;
                }
                app.message = format!("removed stale tmux terminal for {agent_id}");
            }
        }
    }
    if changed {
        refresh_mission_status(&mut app.mission);
        let _ = write_mission(project, &app.mission);
    }
}

pub(crate) fn configured_auto_stop_idle_secs() -> Option<u64> {
    match env::var("PATCHBAY_AUTO_STOP_IDLE_SECS") {
        Ok(value) if matches!(value.as_str(), "0" | "off" | "false" | "disabled") => None,
        Ok(value) => value.parse::<u64>().ok(),
        Err(_) => Some(30 * 60),
    }
}

pub(crate) fn format_duration(seconds: u64) -> String {
    if seconds >= 3600 {
        format!("{}h{}m", seconds / 3600, (seconds % 3600) / 60)
    } else if seconds >= 60 {
        format!("{}m{}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

pub(crate) fn refresh_mission_resources(app: &mut MissionControlApp) {
    app.resources = list_patchbay_tmux_resources(&app.mission_id);
    if app.resources.is_empty() {
        app.selected_resource = 0;
    } else {
        app.selected_resource = app.selected_resource.min(app.resources.len() - 1);
    }
    app.last_resource_refresh = Instant::now();
}

pub(crate) fn list_patchbay_tmux_resources(current_mission_id: &str) -> Vec<MissionResourceRow> {
    let Ok(output) = tmux_command([
        "list-panes",
        "-a",
        "-F",
        "#{session_name}\t#{window_name}\t#{pane_id}\t#{pane_pid}\t#{pane_current_command}\t#{pane_dead}\t#{window_activity}",
    ]) else {
        return Vec::new();
    };
    let now = Utc::now().timestamp().max(0) as u64;
    let mut rows = output
        .lines()
        .filter_map(|line| {
            let parts = line.split('\t').collect::<Vec<_>>();
            if parts.len() != 7 || !parts[0].starts_with("patchbay-mission-") {
                return None;
            }
            let mission_id = parts[0].strip_prefix("patchbay-")?.to_string();
            let activity = parts[6].parse::<u64>().ok();
            Some(MissionResourceRow {
                current_mission: mission_id == current_mission_id,
                mission_id,
                agent_id: parts[1].to_string(),
                pane_id: parts[2].to_string(),
                pane_pid: parts[3].parse::<u32>().ok(),
                command: parts[4].to_string(),
                status: if parts[5] == "1" { "dead" } else { "live" }.to_string(),
                idle_secs: activity.map(|value| now.saturating_sub(value)),
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.mission_id
            .cmp(&right.mission_id)
            .then(left.agent_id.cmp(&right.agent_id))
            .then(left.pane_id.cmp(&right.pane_id))
    });
    rows
}

pub(crate) fn auto_stop_idle_missions(project: &Path, app: &mut MissionControlApp) {
    app.last_auto_stop_check = Instant::now();
    let Some(limit) = app.auto_stop_idle_secs else {
        return;
    };
    let Ok(output) = tmux_command([
        "list-sessions",
        "-F",
        "#{session_name}\t#{session_activity}",
    ]) else {
        return;
    };
    let current_session = tmux_session_name(&app.mission_id);
    let now = Utc::now().timestamp().max(0) as u64;
    let mut stopped = Vec::new();
    for line in output.lines() {
        let Some((session_name, activity_text)) = line.split_once('\t') else {
            continue;
        };
        if !session_name.starts_with("patchbay-mission-") || session_name == current_session {
            continue;
        }
        let Some(idle) = activity_text
            .parse::<u64>()
            .ok()
            .map(|activity| now.saturating_sub(activity))
        else {
            continue;
        };
        if idle < limit {
            continue;
        }
        let mission_id = session_name
            .strip_prefix("patchbay-")
            .unwrap_or(session_name)
            .to_string();
        if hard_cleanup_tmux_session(session_name).is_ok() {
            mark_mission_stopped(
                project,
                &mission_id,
                &format!("auto-stopped after {} idle", format_duration(idle)),
            );
            stopped.push(mission_id);
        }
    }
    if !stopped.is_empty() {
        app.missions = list_missions(project).unwrap_or_default();
        refresh_mission_resources(app);
        app.message = format!("auto-stopped idle mission(s): {}", stopped.join(", "));
    }
}

pub(crate) fn mark_mission_stopped(project: &Path, mission_id: &str, reason: &str) {
    let Ok(mut mission) = read_mission(project, mission_id) else {
        return;
    };
    let now = Utc::now().to_rfc3339();
    let mut changed = false;
    for agent in &mut mission.agents {
        if agent.session_id.is_some() || agent.status == MissionAgentStatus::Running {
            agent.session_id = None;
            agent.status = MissionAgentStatus::Waiting;
            agent.started_at = None;
            agent.finished_at = Some(now.clone());
            agent.last_error = Some(reason.to_string());
            agent.updated_at = now.clone();
            changed = true;
        }
    }
    if changed {
        refresh_mission_status(&mut mission);
        let _ = write_mission(project, &mission);
    }
}

pub(crate) fn hard_cleanup_tmux_session(session_name: &str) -> Result<()> {
    if !tmux_has_session(session_name) {
        return Ok(());
    }
    for pid in tmux_pane_pids_for_target(session_name) {
        terminate_process_tree(pid);
    }
    let _ = tmux_command(["kill-session", "-t", session_name]);
    Ok(())
}

pub(crate) fn hard_cleanup_tmux_pane(pane_id: &str) -> Result<()> {
    for pid in tmux_pane_pids_for_target(pane_id) {
        terminate_process_tree(pid);
    }
    let _ = tmux_command(["kill-pane", "-t", pane_id]);
    Ok(())
}

pub(crate) fn tmux_pane_pids_for_target(target: &str) -> Vec<u32> {
    let Ok(output) = tmux_command(["list-panes", "-t", target, "-F", "#{pane_pid}"]) else {
        return Vec::new();
    };
    output
        .lines()
        .filter_map(|line| line.parse::<u32>().ok())
        .collect()
}

pub(crate) fn terminate_process_tree(root_pid: u32) {
    let mut pids = collect_descendant_pids(root_pid);
    pids.push(root_pid);
    for pid in pids.iter().rev() {
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
    }
    std::thread::sleep(Duration::from_millis(150));
    for pid in pids.iter().rev() {
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .status();
    }
}

pub(crate) fn collect_descendant_pids(root_pid: u32) -> Vec<u32> {
    let mut descendants = Vec::new();
    collect_descendant_pids_into(root_pid, &mut descendants);
    descendants
}

pub(crate) fn collect_descendant_pids_into(parent_pid: u32, output: &mut Vec<u32>) {
    let Ok(children) = Command::new("pgrep")
        .arg("-P")
        .arg(parent_pid.to_string())
        .output()
    else {
        return;
    };
    if !children.status.success() {
        return;
    }
    for line in String::from_utf8_lossy(&children.stdout).lines() {
        let Ok(child_pid) = line.trim().parse::<u32>() else {
            continue;
        };
        collect_descendant_pids_into(child_pid, output);
        output.push(child_pid);
    }
}

pub(crate) fn ensure_tmux_available() -> Result<()> {
    let output = Command::new("tmux").arg("-V").output().context(
        "tmux is required for Patchbay live agent terminals, but `tmux -V` could not run",
    )?;
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
        .output()
        .is_ok_and(|output| output.status.success())
}

pub(crate) fn kill_tmux_agent_windows(session_name: &str, window_name: &str) -> Result<()> {
    if !tmux_has_session(session_name) {
        return Ok(());
    }
    let windows = tmux_command([
        "list-windows",
        "-t",
        session_name,
        "-F",
        "#{window_id}\t#{window_name}",
    ])?;
    for line in windows.lines() {
        let Some((window_id, name)) = line.split_once('\t') else {
            continue;
        };
        if name == window_name {
            for pid in tmux_pane_pids_for_target(window_id) {
                terminate_process_tree(pid);
            }
            let _ = tmux_command(["kill-window", "-t", window_id]);
        }
    }
    Ok(())
}

pub(crate) fn should_submit_prompt_to_interactive_gsd() -> bool {
    env::var("PATCHBAY_RUNNER_CMD").is_err()
        && env::var("PATCHBAY_GSD_INTERACTIVE").ok().as_deref() == Some("1")
}

pub(crate) fn tmux_agent_log_path(
    control_root: &Path,
    mission_id: &str,
    agent_id: &str,
) -> PathBuf {
    missions_dir(control_root).join(format!(
        "{}-{}.log",
        sanitize_tmux_name(mission_id),
        sanitize_tmux_name(agent_id)
    ))
}

pub(crate) fn tmux_agent_prompt_path(
    control_root: &Path,
    mission_id: &str,
    agent_id: &str,
) -> PathBuf {
    missions_dir(control_root).join(format!(
        "{}-{}.prompt.txt",
        sanitize_tmux_name(mission_id),
        sanitize_tmux_name(agent_id)
    ))
}

pub(crate) fn write_agent_prompt(prompt_path: &Path, prompt: &str) -> Result<()> {
    if let Some(parent) = prompt_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(prompt_path, prompt)?;
    Ok(())
}

pub(crate) fn prepare_agent_log(
    log_path: &Path,
    prompt_path: &Path,
    agent: &MissionAgent,
    shell_command: &str,
) -> Result<()> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        log_path,
        format!(
            "[patchbay] starting GSD agent {} in {}\n[patchbay] prompt: {}\n[patchbay] command: {}\n\n",
            agent.id,
            agent.project_path,
            prompt_path.display(),
            shell_command
        ),
    )?;
    Ok(())
}

pub(crate) fn tmux_agent_logged_shell_command(shell_command: &str, log_path: &Path) -> String {
    let log = shell_quote(&log_path.display().to_string());
    let inner = format!(
        "set -o pipefail; {{ {shell_command}; }} 2>&1 | tee -a {log}; status=${{PIPESTATUS[0]}}; printf '\\n[patchbay] agent exited with status %s\\n' \"$status\" | tee -a {log}; exit \"$status\""
    );
    format!("exec bash -lc {}", shell_quote(&inner))
}

pub(crate) fn read_agent_live_output(pane_id: &str, log_path: &Path) -> Result<String> {
    let log_output = fs::read_to_string(log_path).unwrap_or_default();
    let pane_output = tmux_capture_pane(pane_id).unwrap_or_default();
    let log_nonempty_lines = log_output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    let log_has_agent_output =
        log_nonempty_lines > 2 || log_output.contains("[patchbay] agent exited");
    if log_has_agent_output || pane_output.trim().is_empty() {
        return Ok(log_output);
    }
    Ok(pane_output)
}

pub(crate) fn tmux_prepare_agent_pane(
    pane_id: &str,
    project_path: &str,
    shell_command: &str,
) -> Result<()> {
    tmux_command(["set-window-option", "-t", pane_id, "remain-on-exit", "on"])?;
    tmux_command([
        "respawn-pane",
        "-k",
        "-t",
        pane_id,
        "-c",
        project_path,
        shell_command,
    ])?;
    Ok(())
}

pub(crate) fn tmux_submit_agent_prompt(pane_id: &str, prompt: &str) -> Result<()> {
    std::thread::sleep(Duration::from_millis(350));
    let prompt = prompt
        .replace(['\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    tmux_command(["send-keys", "-t", pane_id, "-l", prompt.as_str()])?;
    tmux_command(["send-keys", "-t", pane_id, "Enter"])?;
    Ok(())
}

pub(crate) fn tmux_pane_dead_status(pane_id: &str) -> Result<Option<i32>> {
    let output = tmux_command([
        "display-message",
        "-p",
        "-t",
        pane_id,
        "#{pane_dead}:#{pane_dead_status}",
    ])?;
    let Some((dead, status)) = output.split_once(':') else {
        return Ok(None);
    };
    if dead != "1" {
        return Ok(None);
    }
    Ok(Some(status.parse::<i32>().unwrap_or(1)))
}

pub(crate) fn tmux_capture_pane(pane_id: &str) -> Result<String> {
    // GSD/pi renders as a full-screen TUI on tmux's alternate screen. Capturing
    // scrollback (`-S -200`) shows stale command history, not the live tool/feed
    // surface. `-a` asks tmux for the alternate screen when present and falls
    // back to the visible pane otherwise, which makes agent tiles behave like a
    // read-only miniature of the real GSD screen. Some tmux versions return
    // `no alternate screen` as an error, so fall back on both empty output and
    // capture errors.
    match tmux_command(["capture-pane", "-a", "-p", "-t", pane_id]) {
        Ok(screen) if !screen.trim().is_empty() => Ok(screen),
        Ok(_) | Err(_) => tmux_command(["capture-pane", "-p", "-t", pane_id]),
    }
}

pub(crate) fn parse_tmux_session_id(session_id: &str) -> Option<(String, String)> {
    let rest = session_id.strip_prefix("tmux:")?;
    let (session, pane) = rest.split_once(':')?;
    Some((session.to_string(), pane.to_string()))
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
    let prompt_path = tmux_agent_prompt_path(control_root, mission_id, &agent.id);
    let mut env_parts = vec![
        ("PATCHBAY_HOME", control_root.display().to_string()),
        ("PATCHBAY_MISSION_ID", mission_id.to_string()),
        ("PATCHBAY_AGENT_ID", agent.id.clone()),
        ("PATCHBAY_PROJECT", agent.project.clone()),
        ("PATCHBAY_PROFILE", profile.to_string()),
        ("PATCHBAY_PROMPT_FILE", prompt_path.display().to_string()),
    ];
    if let Ok(bin) = env::current_exe() {
        env_parts.push(("PATCHBAY_BIN", bin.display().to_string()));
    }
    for key in ["XDG_CONFIG_HOME", "HOME"] {
        if let Ok(value) = env::var(key) {
            env_parts.push((key, value));
        }
    }
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
        let interactive_mode = env::var("PATCHBAY_GSD_INTERACTIVE").ok().as_deref() == Some("1");
        let print_mode =
            !interactive_mode || env::var("PATCHBAY_GSD_PRINT").ok().as_deref() == Some("1");
        let mut parts = vec!["exec".to_string(), "gsd".to_string()];
        if print_mode {
            parts.push("--print".to_string());
        }
        if let Some(model) = runner_model_for_profile(profile) {
            parts.push("--model".to_string());
            parts.push(shell_quote(&model));
        }
        if print_mode {
            parts.push(format!(
                "\"$(cat {})\"",
                shell_quote(&prompt_path.display().to_string())
            ));
        }
        parts.join(" ")
    };

    format!("{env} {command}")
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(crate) fn mission_control_help() -> String {
    String::new()
}

pub(crate) fn status_style(status: &str) -> Style {
    match status {
        "complete" | "reviewed" => Style::default().fg(Color::Green),
        "failed" | "blocked" => Style::default().fg(Color::Red),
        "queued" | "unreviewed" => Style::default().fg(Color::Yellow),
        "running" => Style::default().fg(Color::Cyan),
        _ => Style::default(),
    }
}
