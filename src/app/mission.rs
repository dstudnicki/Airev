use super::*;

pub(crate) fn mission_control_block<'a>(
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

pub(crate) async fn handle_mission_command(project: &Path, command: MissionCommand) -> Result<()> {
    match command {
        MissionCommand::Start {
            title,
            text,
            text_file,
            tasks,
        } => start_mission(project, title, text, text_file, tasks),
        MissionCommand::List => list_mission_command(project),
        MissionCommand::Show { mission } => show_mission_command(project, &mission),
        MissionCommand::Status { mission } => status_mission_command(project, mission.as_deref()),
        MissionCommand::UpdateAgent {
            mission,
            agent,
            status,
            summary,
            error,
            revisions,
            diffs,
        } => update_mission_agent_command(
            project, &mission, &agent, status, summary, error, revisions, diffs,
        ),
        MissionCommand::Diffs { mission, agent } => {
            mission_diffs_command(project, &mission, agent.as_deref())
        }
        MissionCommand::OpenDiff {
            mission,
            agent,
            diff_index,
            terminal,
            editor,
        } => {
            mission_open_diff_command(project, &mission, &agent, diff_index, terminal, &editor)
                .await
        }
        MissionCommand::Wm { mission } => {
            launch_mission_control_ui(project, mission.as_deref()).await
        }
    }
}

pub(crate) fn start_mission(
    project: &Path,
    title: Option<String>,
    text: Option<String>,
    text_file: Option<PathBuf>,
    explicit_tasks: Vec<String>,
) -> Result<()> {
    let source_text = read_text_arg(text, text_file)?.unwrap_or_default();
    let registry = load_project_registry(project)?;
    let task_specs = parse_mission_tasks(&registry, &source_text, &explicit_tasks)?;
    let mission_id = next_mission_id(project)?;
    let mission_title = title.unwrap_or_else(|| {
        if source_text.trim().is_empty() {
            "Mission Control run".to_string()
        } else {
            title_from_prompt(&source_text)
        }
    });
    let now = Utc::now().to_rfc3339();
    let mut agents = Vec::new();

    for (index, spec) in task_specs.iter().enumerate() {
        let registered = registry.resolve(&spec.project)?;
        let agent_id = mission_agent_id(index, &registered.name);
        agents.push(MissionAgent {
            id: agent_id,
            project: registered.name.clone(),
            project_path: registered.path.display().to_string(),
            task: spec.task.clone(),
            prompt: build_mission_agent_prompt(
                &mission_title,
                &source_text,
                registered,
                &spec.task,
            ),
            status: MissionAgentStatus::Pending,
            created_at: now.clone(),
            updated_at: now.clone(),
            summary: None,
            last_error: None,
            revision_ids: Vec::new(),
            diff_refs: Vec::new(),
        });
    }

    let mission = Mission {
        id: mission_id,
        title: mission_title,
        source_text,
        status: MissionStatus::Running,
        created_at: now.clone(),
        updated_at: now,
        agents,
    };

    write_mission(project, &mission)?;
    println!("Mission {} created: {}", mission.id, mission.title);
    for agent in &mission.agents {
        println!(
            "  {}  {:<12}  {}",
            agent.id,
            format_mission_agent_status(&agent.status),
            agent.project
        );
        println!("      {}", agent.task);
    }
    Ok(())
}

pub(crate) fn list_mission_command(project: &Path) -> Result<()> {
    let missions = list_missions(project)?;
    if missions.is_empty() {
        println!("No Airev missions recorded.");
        return Ok(());
    }
    for mission in missions {
        println!(
            "{}  {:<9}  {:>2} agent(s)  {}  {}",
            mission.id,
            format_mission_status(&mission.status),
            mission.agents.len(),
            mission.updated_at,
            mission.title
        );
    }
    Ok(())
}

pub(crate) fn show_mission_command(project: &Path, mission_id: &str) -> Result<()> {
    let mission = read_mission(project, mission_id)?;
    print_mission_detail(&mission, true);
    Ok(())
}

pub(crate) fn status_mission_command(project: &Path, mission_id: Option<&str>) -> Result<()> {
    let mission = match mission_id {
        Some(id) => read_mission(project, id)?,
        None => latest_mission(project)?,
    };
    print_mission_detail(&mission, false);
    Ok(())
}

pub(crate) fn update_mission_agent_command(
    project: &Path,
    mission_id: &str,
    agent_id: &str,
    status: Option<MissionAgentStatusArg>,
    summary: Option<String>,
    error: Option<String>,
    revisions: Vec<i64>,
    diffs: Vec<String>,
) -> Result<()> {
    let mut mission = read_mission(project, mission_id)?;
    let now = Utc::now().to_rfc3339();
    let agent_status = {
        let agent = mission
            .agents
            .iter_mut()
            .find(|agent| agent.id == agent_id)
            .ok_or_else(|| anyhow!("Mission `{mission_id}` has no agent `{agent_id}`"))?;

        if let Some(next_status) = status {
            agent.status = next_status.into();
        }
        if let Some(summary) = summary {
            agent.summary = Some(summary);
        }
        if let Some(error) = error {
            agent.last_error = Some(error);
        }
        if !revisions.is_empty() {
            agent.revision_ids = revisions;
        }
        if !diffs.is_empty() {
            agent.diff_refs = parse_mission_diff_refs(&diffs)?;
        }
        agent.updated_at = now.clone();
        agent.status.clone()
    };
    mission.updated_at = now;
    refresh_mission_status(&mut mission);
    write_mission(project, &mission)?;
    println!(
        "Mission {} agent {} updated: {}",
        mission.id,
        agent_id,
        format_mission_agent_status(&agent_status)
    );
    Ok(())
}

pub(crate) fn mission_diffs_command(
    project: &Path,
    mission_id: &str,
    agent_id: Option<&str>,
) -> Result<()> {
    let mission = read_mission(project, mission_id)?;
    let agents = mission_agents_for_diff_command(&mission, agent_id)?;
    let mut found = false;

    for agent in agents {
        println!("{}  {}  {}", agent.id, agent.project, agent.project_path);
        if agent.diff_refs.is_empty() {
            println!("  no diff refs recorded");
            continue;
        }
        found = true;
        for (index, diff) in agent.diff_refs.iter().enumerate() {
            match diff.revision_id {
                Some(revision_id) => println!("  [{index}] revision {revision_id}: {}", diff.path),
                None => println!("  [{index}] unbound: {}", diff.path),
            }
        }
    }

    if !found {
        println!("No diff refs recorded for mission {mission_id}.");
    }
    Ok(())
}

pub(crate) async fn mission_open_diff_command(
    project: &Path,
    mission_id: &str,
    agent_id: &str,
    diff_index: usize,
    terminal: bool,
    editor: &str,
) -> Result<()> {
    let mission = read_mission(project, mission_id)?;
    let agent = mission_agent(&mission, agent_id)?;
    let diff = agent.diff_refs.get(diff_index).ok_or_else(|| {
        anyhow!("Mission `{mission_id}` agent `{agent_id}` has no diff index {diff_index}")
    })?;
    let revision_id = diff.revision_id.ok_or_else(|| {
        anyhow!(
            "Mission `{mission_id}` agent `{agent_id}` diff index {diff_index} is not bound to a revision id"
        )
    })?;
    let agent_project = PathBuf::from(&agent.project_path);
    if !agent_project.join(STORE_DIR).join(DB_FILE).exists() {
        bail!(
            "Agent project `{}` is not an initialized Airev project",
            agent.project_path
        );
    }
    let revision = revision_id.to_string();
    if terminal {
        print_terminal_diff(&agent_project, &revision, &diff.path).await
    } else {
        open_diff(&agent_project, &revision, &diff.path, editor).await
    }
}

pub(crate) fn mission_agents_for_diff_command<'a>(
    mission: &'a Mission,
    agent_id: Option<&str>,
) -> Result<Vec<&'a MissionAgent>> {
    match agent_id {
        Some(agent_id) => Ok(vec![mission_agent(mission, agent_id)?]),
        None => Ok(mission.agents.iter().collect()),
    }
}

pub(crate) fn mission_agent<'a>(mission: &'a Mission, agent_id: &str) -> Result<&'a MissionAgent> {
    mission
        .agents
        .iter()
        .find(|agent| agent.id == agent_id)
        .ok_or_else(|| anyhow!("Mission `{}` has no agent `{agent_id}`", mission.id))
}

pub(crate) fn parse_mission_tasks(
    registry: &ProjectRegistry,
    source_text: &str,
    explicit_tasks: &[String],
) -> Result<Vec<MissionTaskSpec>> {
    let mut tasks = Vec::new();
    for task in explicit_tasks {
        tasks.push(parse_explicit_mission_task(registry, task)?);
    }

    for fragment in source_text.lines().flat_map(|line| line.split(';')) {
        let fragment = fragment.trim();
        if fragment.is_empty() {
            continue;
        }
        if let Some(task) = parse_prefixed_mission_task(registry, fragment)? {
            tasks.push(task);
        }
    }

    if tasks.is_empty() {
        bail!(
            "No project tasks found. Use --task project=task or text lines like `Airev: build X`. Known projects: {}",
            registry.names().join(", ")
        );
    }
    Ok(tasks)
}

pub(crate) fn parse_explicit_mission_task(
    registry: &ProjectRegistry,
    value: &str,
) -> Result<MissionTaskSpec> {
    let (project, task) = value
        .split_once('=')
        .or_else(|| value.split_once(':'))
        .ok_or_else(|| {
            anyhow!("Mission task `{value}` must use `project=task` or `project:task`")
        })?;
    let registered = registry.resolve(project.trim())?;
    let task = task.trim();
    if task.is_empty() {
        bail!(
            "Mission task for project `{}` cannot be empty",
            registered.name
        );
    }
    Ok(MissionTaskSpec {
        project: registered.name.clone(),
        task: task.to_string(),
    })
}

pub(crate) fn parse_prefixed_mission_task(
    registry: &ProjectRegistry,
    fragment: &str,
) -> Result<Option<MissionTaskSpec>> {
    let Some((project_name, task)) = fragment.split_once(':') else {
        return Ok(None);
    };
    let project_name = project_name.trim();
    if project_name.is_empty() {
        return Ok(None);
    }
    let Ok(registered) = registry.resolve(project_name) else {
        return Ok(None);
    };
    let task = task.trim();
    if task.is_empty() {
        bail!(
            "Mission task for project `{}` cannot be empty",
            registered.name
        );
    }
    Ok(Some(MissionTaskSpec {
        project: registered.name.clone(),
        task: task.to_string(),
    }))
}

pub(crate) fn build_mission_agent_prompt(
    mission_title: &str,
    source_text: &str,
    project: &RegisteredProject,
    task: &str,
) -> String {
    format!(
        "Mission: {mission_title}\nProject: {}\nProject path: {}\n\nTask:\n{task}\n\nSource mission text:\n{}\n\nWork autonomously inside this project only. Preserve context in local GSD/Airev artifacts, verify before completion, and report summary, status, revision IDs, and diff paths back to the MAIN agent.",
        project.name,
        project.path.display(),
        source_text.trim()
    )
}

pub(crate) fn next_mission_id(project: &Path) -> Result<String> {
    let base = format!("mission-{}", Utc::now().format("%Y%m%d%H%M%S"));
    for suffix in 0..1000 {
        let candidate = if suffix == 0 {
            base.clone()
        } else {
            format!("{base}-{suffix:03}")
        };
        if !mission_path(project, &candidate)?.exists() {
            return Ok(candidate);
        }
    }
    bail!("Could not allocate a unique mission id for {base}")
}

pub(crate) fn mission_agent_id(index: usize, project: &str) -> String {
    let slug = project
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    format!("agent-{:02}-{}", index + 1, slug)
}

pub(crate) fn latest_mission(project: &Path) -> Result<Mission> {
    let missions = list_missions(project)?;
    missions
        .into_iter()
        .last()
        .ok_or_else(|| anyhow!("No Airev missions recorded."))
}

pub(crate) fn print_mission_detail(mission: &Mission, include_prompts: bool) {
    println!(
        "{}  {}  {}",
        mission.id,
        format_mission_status(&mission.status),
        mission.title
    );
    println!("updated: {}", mission.updated_at);
    if !mission.source_text.trim().is_empty() {
        println!(
            "source: {}",
            truncate_pretty(mission.source_text.trim(), 120)
        );
    }
    for agent in &mission.agents {
        println!(
            "\n{}  {}  {}",
            agent.id,
            format_mission_agent_status(&agent.status),
            agent.project
        );
        println!("path: {}", agent.project_path);
        println!("task: {}", agent.task);
        if let Some(summary) = &agent.summary {
            println!("summary: {}", summary);
        }
        if let Some(error) = &agent.last_error {
            println!("last_error: {}", error);
        }
        if !agent.revision_ids.is_empty() {
            println!("revisions: {:?}", agent.revision_ids);
        }
        if !agent.diff_refs.is_empty() {
            let refs = agent
                .diff_refs
                .iter()
                .map(|diff| match diff.revision_id {
                    Some(revision_id) => format!("{}:{}", revision_id, diff.path),
                    None => diff.path.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            println!("diffs: {refs}");
        }
        if include_prompts {
            println!("prompt:\n{}", agent.prompt);
        }
    }
}

pub(crate) fn refresh_mission_status(mission: &mut Mission) {
    mission.status = if mission
        .agents
        .iter()
        .any(|agent| agent.status == MissionAgentStatus::Failed)
    {
        MissionStatus::Failed
    } else if mission
        .agents
        .iter()
        .any(|agent| agent.status == MissionAgentStatus::Blocked)
    {
        MissionStatus::Blocked
    } else if mission
        .agents
        .iter()
        .all(|agent| agent.status == MissionAgentStatus::Complete)
    {
        MissionStatus::Complete
    } else if mission
        .agents
        .iter()
        .any(|agent| agent.status == MissionAgentStatus::Waiting)
    {
        MissionStatus::Waiting
    } else {
        MissionStatus::Running
    };
}

pub(crate) fn parse_mission_diff_refs(values: &[String]) -> Result<Vec<MissionDiffRef>> {
    values
        .iter()
        .map(|value| {
            if let Some((revision, path)) = value.split_once(':') {
                if let Ok(revision_id) = revision.parse::<i64>() {
                    if path.trim().is_empty() {
                        bail!("Diff ref `{value}` has an empty path");
                    }
                    return Ok(MissionDiffRef {
                        revision_id: Some(revision_id),
                        path: path.trim().to_string(),
                    });
                }
            }
            if value.trim().is_empty() {
                bail!("Diff ref cannot be empty");
            }
            Ok(MissionDiffRef {
                revision_id: None,
                path: value.trim().to_string(),
            })
        })
        .collect()
}

pub(crate) fn format_mission_status(status: &MissionStatus) -> &'static str {
    match status {
        MissionStatus::Draft => "draft",
        MissionStatus::Running => "running",
        MissionStatus::Waiting => "waiting",
        MissionStatus::Complete => "complete",
        MissionStatus::Failed => "failed",
        MissionStatus::Blocked => "blocked",
    }
}

pub(crate) fn format_mission_agent_status(status: &MissionAgentStatus) -> &'static str {
    match status {
        MissionAgentStatus::Pending => "pending",
        MissionAgentStatus::Running => "running",
        MissionAgentStatus::Waiting => "waiting",
        MissionAgentStatus::Complete => "complete",
        MissionAgentStatus::Failed => "failed",
        MissionAgentStatus::Blocked => "blocked",
    }
}

pub(crate) fn missions_dir(project: &Path) -> PathBuf {
    project.join(STORE_DIR).join(RUNTIME_DIR).join(MISSIONS_DIR)
}

pub(crate) fn mission_path(project: &Path, mission_id: &str) -> Result<PathBuf> {
    validate_store_id(mission_id, "mission id")?;
    Ok(missions_dir(project).join(format!("{mission_id}.json")))
}

pub(crate) fn write_mission(project: &Path, mission: &Mission) -> Result<()> {
    ensure_store_dirs(project)?;
    fs::write(
        mission_path(project, &mission.id)?,
        serde_json::to_string_pretty(mission)?,
    )?;
    Ok(())
}

pub(crate) fn read_mission(project: &Path, mission_id: &str) -> Result<Mission> {
    let path = mission_path(project, mission_id)?;
    let text = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read mission `{mission_id}`"))?;
    serde_json::from_str(&text).with_context(|| format!("Failed to parse mission `{mission_id}`"))
}

pub(crate) fn list_missions(project: &Path) -> Result<Vec<Mission>> {
    let dir = missions_dir(project);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut missions = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(OsStr::to_str) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read mission file {}", path.display()))?;
        missions.push(
            serde_json::from_str(&text)
                .with_context(|| format!("Failed to parse mission file {}", path.display()))?,
        );
    }
    missions.sort_by(|left: &Mission, right: &Mission| left.id.cmp(&right.id));
    Ok(missions)
}

pub(crate) fn load_project_registry(project: &Path) -> Result<ProjectRegistry> {
    let config = load_merged_config(project);
    let mut registry = ProjectRegistry::default();

    if let Some(projects) = config.projects {
        for (name, configured) in projects {
            let resolved_path = resolve_registry_project_path(project, &configured.path)
                .with_context(|| format!("Invalid path for project `{name}`"))?;
            registry.projects.insert(
                name.clone(),
                RegisteredProject {
                    name,
                    path: resolved_path,
                    description: configured.description,
                },
            );
        }
    }

    if registry.projects.is_empty() {
        let name = project
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("current")
            .to_string();
        registry.projects.insert(
            name.clone(),
            RegisteredProject {
                name,
                path: project.to_path_buf(),
                description: Some("Current Airev project".to_string()),
            },
        );
    }

    Ok(registry)
}

pub(crate) fn validate_store_id(value: &str, label: &str) -> Result<()> {
    if value.is_empty() {
        bail!("{label} cannot be empty");
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Ok(());
    }
    bail!("{label} `{value}` may only contain letters, numbers, dashes, and underscores")
}

pub(crate) fn resolve_registry_project_path(base_project: &Path, input: &str) -> Result<PathBuf> {
    let raw = Path::new(input);
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        base_project.join(raw)
    };
    Ok(lexical_normalize(&joined))
}
