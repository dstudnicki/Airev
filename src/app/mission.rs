use super::*;

pub(crate) async fn handle_mission_command(
    project: &Path,
    local_project: Option<&Path>,
    command: MissionCommand,
) -> Result<()> {
    match command {
        MissionCommand::Start {
            title,
            text,
            text_file,
            tasks,
        } => start_mission(project, title, text, text_file, tasks).await,
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
        MissionCommand::AddAgent {
            mission,
            parent,
            project: agent_project,
            task,
            title,
            profile,
        } => {
            add_mission_agent_command(
                project,
                &mission,
                parent.as_deref(),
                &agent_project,
                &task,
                title.as_deref(),
                profile,
            )
            .await
        }
        MissionCommand::Run {
            mission,
            agent,
            all,
            profile,
        } => run_mission_command(project, mission.as_deref(), agent.as_deref(), all, profile),
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
            launch_mission_control_ui(project, mission.as_deref(), local_project).await
        }
    }
}

pub(crate) async fn start_mission(
    project: &Path,
    title: Option<String>,
    text: Option<String>,
    text_file: Option<PathBuf>,
    explicit_tasks: Vec<String>,
) -> Result<()> {
    let source_text = read_text_arg(text, text_file)?.unwrap_or_default();
    let registry = load_project_registry(project)?;
    let task_specs = parse_mission_tasks(&registry, &source_text, &explicit_tasks)?;
    ensure_revision_stores_for_tasks(&registry, &task_specs).await?;
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
        let skill_preset = classify_mission_skill_preset(&source_text, &spec.task);
        let agent_id = mission_agent_id(index, &registered.name);
        agents.push(MissionAgent {
            id: agent_id,
            parent_id: spec.parent_id.clone(),
            project: registered.name.clone(),
            project_path: registered.path.display().to_string(),
            task: spec.task.clone(),
            prompt: build_mission_agent_prompt(
                &mission_title,
                &source_text,
                registered,
                &spec.task,
                &skill_preset,
            ),
            status: MissionAgentStatus::Pending,
            created_at: now.clone(),
            updated_at: now.clone(),
            summary: None,
            last_error: None,
            revision_ids: Vec::new(),
            diff_refs: Vec::new(),
            prompt_preset: Some(skill_preset.name.to_string()),
            recommended_skills: skill_preset
                .skills
                .iter()
                .map(|skill| skill.to_string())
                .collect(),
            runner_profile: None,
            session_id: None,
            started_at: None,
            finished_at: None,
            last_log: None,
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

pub(crate) async fn ensure_revision_stores_for_tasks(
    registry: &ProjectRegistry,
    task_specs: &[MissionTaskSpec],
) -> Result<()> {
    let mut initialized = BTreeSet::new();
    for spec in task_specs {
        if !initialized.insert(spec.project.clone()) {
            continue;
        }
        let registered = registry.resolve(&spec.project)?;
        if !registered.path.join(STORE_DIR).join(DB_FILE).exists() {
            init_project(&registered.path).await?;
        }
    }
    Ok(())
}

pub(crate) fn create_mission_from_specs(
    project: &Path,
    registry: &ProjectRegistry,
    mission_title: String,
    source_text: String,
    task_specs: &[MissionTaskSpec],
) -> Result<Mission> {
    let mission_id = next_mission_id(project)?;
    let now = Utc::now().to_rfc3339();
    let mut agents = Vec::new();
    let parent_agent_ids = task_specs
        .iter()
        .filter_map(|spec| spec.parent_id.clone())
        .collect::<BTreeSet<_>>();

    for (index, spec) in task_specs.iter().enumerate() {
        let registered = registry.resolve(&spec.project)?;
        let skill_preset = classify_mission_skill_preset(&source_text, &spec.task);
        let agent_id = mission_agent_id(index, &registered.name);
        let status = if parent_agent_ids.contains(&agent_id) {
            MissionAgentStatus::Complete
        } else {
            MissionAgentStatus::Pending
        };
        agents.push(MissionAgent {
            id: agent_id,
            parent_id: spec.parent_id.clone(),
            project: registered.name.clone(),
            project_path: registered.path.display().to_string(),
            task: spec.task.clone(),
            prompt: build_mission_agent_prompt(
                &mission_title,
                &source_text,
                registered,
                &spec.task,
                &skill_preset,
            ),
            status,
            created_at: now.clone(),
            updated_at: now.clone(),
            summary: None,
            last_error: None,
            revision_ids: Vec::new(),
            diff_refs: Vec::new(),
            prompt_preset: Some(skill_preset.name.to_string()),
            recommended_skills: skill_preset
                .skills
                .iter()
                .map(|skill| skill.to_string())
                .collect(),
            runner_profile: None,
            session_id: None,
            started_at: None,
            finished_at: None,
            last_log: None,
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
            "  {}  {:<12}  {}{}",
            agent.id,
            format_mission_agent_status(&agent.status),
            agent.project,
            agent
                .parent_id
                .as_ref()
                .map(|parent| format!("  parent:{parent}"))
                .unwrap_or_default()
        );
        println!("      {}", agent.task);
    }
    Ok(mission)
}

pub(crate) async fn compose_mission_command(
    project: &Path,
    text: Option<String>,
    text_file: Option<PathBuf>,
    run: bool,
    profile: Option<String>,
) -> Result<()> {
    let source_text = read_text_arg(text, text_file)?
        .ok_or_else(|| anyhow!("Compose requires --text or --text-file."))?;
    let registry = load_project_registry(project)?;
    let task_specs = compose_mission_tasks(&registry, &source_text)?;
    ensure_revision_stores_for_tasks(&registry, &task_specs).await?;
    let title = title_from_prompt(&source_text);
    let runner_profile = normalize_runner_profile(profile);
    let mut mission =
        create_mission_from_specs(project, &registry, title, source_text, &task_specs)?;
    if let Some(profile) = runner_profile.clone() {
        for agent in &mut mission.agents {
            agent.runner_profile = Some(profile.clone());
        }
        write_mission(project, &mission)?;
    }
    if run {
        run_mission_command(project, Some(&mission.id), None, true, runner_profile)?;
    }
    Ok(())
}

pub(crate) fn compose_mission_tasks(
    registry: &ProjectRegistry,
    source_text: &str,
) -> Result<Vec<MissionTaskSpec>> {
    let mentioned = mentioned_compose_projects(registry, source_text);
    let root_project = orchestrator_project_name(registry, &mentioned)?;
    let root_task = planner_task_from_compose_prompt(source_text);
    let root_agent_id = mission_agent_id(0, &root_project);
    let mut tasks = vec![MissionTaskSpec {
        project: root_project,
        task: root_task,
        parent_id: None,
    }];

    if mentioned.len() > 1 {
        for project in mentioned {
            tasks.push(MissionTaskSpec {
                task: child_task_from_compose_prompt(source_text, &project),
                project,
                parent_id: Some(root_agent_id.clone()),
            });
        }
    }

    Ok(tasks)
}

pub(crate) fn planner_task_from_compose_prompt(source_text: &str) -> String {
    format!("Root planner: {}", source_text.trim())
}

pub(crate) fn child_task_from_compose_prompt(source_text: &str, project: &str) -> String {
    format!(
        "Work in {project}: carry out the parts of this mission that apply to {project}. Source mission: {}",
        source_text.trim()
    )
}

pub(crate) fn mentioned_compose_projects(
    registry: &ProjectRegistry,
    source_text: &str,
) -> Vec<String> {
    let match_text = compose_project_match_text(source_text);
    registry
        .projects
        .values()
        .filter(|project| !project.name.starts_with('.'))
        .filter(|project| project_name_mentioned(&match_text, &project.name))
        .map(|project| project.name.clone())
        .collect()
}

pub(crate) fn compose_project_match_text(source_text: &str) -> String {
    source_text
        .split_whitespace()
        .filter(|token| {
            let trimmed = token.trim_end_matches(|ch: char| ch.is_ascii_punctuation());
            !(trimmed.starts_with('/') || trimmed.starts_with("~/"))
        })
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub(crate) fn project_name_mentioned(match_text: &str, project_name: &str) -> bool {
    let name = project_name.to_ascii_lowercase();
    match_text.match_indices(&name).any(|(start, _)| {
        let end = start + name.len();
        let before = match_text[..start].chars().next_back();
        let after = match_text[end..].chars().next();
        is_project_name_boundary(before) && is_project_name_boundary(after)
    })
}

pub(crate) fn is_project_name_boundary(ch: Option<char>) -> bool {
    ch.is_none_or(|ch| !(ch.is_ascii_alphanumeric() || ch == '_'))
}

pub(crate) fn orchestrator_project_name(
    registry: &ProjectRegistry,
    mentioned: &[String],
) -> Result<String> {
    for candidate in ["Patchbay", "patchbay", "Airev", "airev"] {
        if let Ok(project) = registry.resolve(candidate) {
            if !project.name.starts_with('.') {
                return Ok(project.name.clone());
            }
        }
    }
    if let Some(first) = mentioned.first() {
        return Ok(first.clone());
    }
    registry
        .names()
        .into_iter()
        .find(|name| !name.starts_with('.'))
        .ok_or_else(|| anyhow!("No non-hidden Patchbay projects are configured."))
}

pub(crate) async fn add_mission_agent_command(
    project: &Path,
    mission_id: &str,
    parent_id: Option<&str>,
    agent_project: &str,
    task: &str,
    title: Option<&str>,
    profile: Option<String>,
) -> Result<()> {
    let registry = load_project_registry(project)?;
    let registered = registry.resolve(agent_project)?;
    if !registered.path.join(STORE_DIR).join(DB_FILE).exists() {
        init_project(&registered.path).await?;
    }

    let mut mission = read_mission(project, mission_id)?;
    if let Some(parent_id) = parent_id {
        mission_agent(&mission, parent_id)?;
    }
    let now = Utc::now().to_rfc3339();
    let agent_id = next_nested_agent_id(&mission, parent_id, registered, task);
    let source_text = mission.source_text.clone();
    let mission_title = title.unwrap_or(&mission.title).to_string();
    let skill_preset = classify_mission_skill_preset(&source_text, task);
    mission.agents.push(MissionAgent {
        id: agent_id.clone(),
        parent_id: parent_id.map(str::to_string),
        project: registered.name.clone(),
        project_path: registered.path.display().to_string(),
        task: task.to_string(),
        prompt: build_mission_agent_prompt(
            &mission_title,
            &source_text,
            registered,
            task,
            &skill_preset,
        ),
        status: MissionAgentStatus::Pending,
        created_at: now.clone(),
        updated_at: now.clone(),
        summary: None,
        last_error: None,
        revision_ids: Vec::new(),
        diff_refs: Vec::new(),
        prompt_preset: Some(skill_preset.name.to_string()),
        recommended_skills: skill_preset
            .skills
            .iter()
            .map(|skill| skill.to_string())
            .collect(),
        runner_profile: normalize_runner_profile(profile),
        session_id: None,
        started_at: None,
        finished_at: None,
        last_log: None,
    });
    mission.updated_at = now;
    write_mission(project, &mission)?;
    println!("Added agent {agent_id} to mission {mission_id}");
    Ok(())
}

pub(crate) fn run_mission_command(
    project: &Path,
    mission_id: Option<&str>,
    agent_id: Option<&str>,
    all: bool,
    profile: Option<String>,
) -> Result<()> {
    let mut mission = match mission_id {
        Some(id) => read_mission(project, id)?,
        None => latest_mission(project)?,
    };
    let selected = select_agents_to_run(&mission, agent_id, all)?;
    if selected.is_empty() {
        println!("No pending agents to run.");
        return Ok(());
    }

    for id in selected {
        run_agent_loop(project, &mut mission, &id, profile.clone())?;
    }
    write_mission(project, &mission)?;
    Ok(())
}

pub(crate) fn select_agents_to_run(
    mission: &Mission,
    agent_id: Option<&str>,
    all: bool,
) -> Result<Vec<String>> {
    if let Some(agent_id) = agent_id {
        mission_agent(mission, agent_id)?;
        return Ok(vec![agent_id.to_string()]);
    }
    let ids = mission
        .agents
        .iter()
        .filter(|agent| all || agent.parent_id.is_none())
        .filter(|agent| agent.status == MissionAgentStatus::Pending)
        .map(|agent| agent.id.clone())
        .collect::<Vec<_>>();
    Ok(ids)
}

pub(crate) fn run_agent_loop(
    project: &Path,
    mission: &mut Mission,
    agent_id: &str,
    profile: Option<String>,
) -> Result<()> {
    let mission_id = mission.id.clone();
    let agent_index = mission
        .agents
        .iter()
        .position(|agent| agent.id == agent_id)
        .ok_or_else(|| anyhow!("Mission `{mission_id}` has no agent `{agent_id}`"))?;
    let profile_name = normalize_runner_profile(profile)
        .or_else(|| mission.agents[agent_index].runner_profile.clone())
        .unwrap_or_else(|| "default".to_string());
    let session_id = format!("{}-{}", mission_id, agent_id);
    let started_at = Utc::now().to_rfc3339();

    mission.agents[agent_index].status = MissionAgentStatus::Running;
    mission.agents[agent_index].updated_at = started_at.clone();
    mission.agents[agent_index].started_at = Some(started_at.clone());
    mission.agents[agent_index].finished_at = None;
    mission.agents[agent_index].session_id = Some(session_id.clone());
    mission.agents[agent_index].runner_profile = Some(profile_name.clone());
    mission.agents[agent_index].last_error = None;
    write_mission(project, mission)?;

    let agent_snapshot = mission.agents[agent_index].clone();
    let output = run_agent_process(project, &mission_id, &agent_snapshot, &profile_name);
    let finished_at = Utc::now().to_rfc3339();
    let agent = &mut mission.agents[agent_index];
    agent.finished_at = Some(finished_at.clone());
    agent.updated_at = finished_at;

    match output {
        Ok((stdout, stderr)) => {
            agent.status = MissionAgentStatus::Complete;
            agent.summary = Some(compact_log(&stdout));
            agent.last_log = Some(compact_log(&format!("{stdout}\n{stderr}")));
            if stderr.trim().is_empty() {
                agent.last_error = None;
            } else {
                agent.last_error = Some(compact_log(&stderr));
            }
            println!("Agent {agent_id} complete");
        }
        Err(error) => {
            agent.status = MissionAgentStatus::Failed;
            agent.last_error = Some(error.to_string());
            agent.last_log = Some(error.to_string());
            println!("Agent {agent_id} failed: {error}");
        }
    }
    mission.updated_at = agent.updated_at.clone();
    refresh_mission_status(mission);
    Ok(())
}

pub(crate) fn run_agent_process(
    control_root: &Path,
    mission_id: &str,
    agent: &MissionAgent,
    profile: &str,
) -> Result<(String, String)> {
    let mut command = if let Ok(runner_cmd) = env::var("PATCHBAY_RUNNER_CMD") {
        let mut command = Command::new("sh");
        command.arg("-c").arg(runner_cmd);
        command.stdin(std::process::Stdio::piped());
        command
    } else {
        let mut command = Command::new("gsd");
        command.arg("--print");
        if let Some(model) = runner_model_for_profile(profile) {
            command.arg("--model").arg(model);
        }
        command.arg(&agent.prompt);
        command
    };

    command
        .current_dir(&agent.project_path)
        .env("PATCHBAY_HOME", control_root)
        .env("PATCHBAY_MISSION_ID", mission_id)
        .env("PATCHBAY_AGENT_ID", &agent.id)
        .env("PATCHBAY_PROJECT", &agent.project)
        .env("PATCHBAY_PROFILE", profile);
    if let Ok(bin) = env::current_exe() {
        command.env("PATCHBAY_BIN", bin);
    }
    if let Some(parent_id) = &agent.parent_id {
        command.env("PATCHBAY_PARENT_AGENT_ID", parent_id);
    }
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let mut child = command
        .spawn()
        .with_context(|| format!("Failed to start runner for agent {}", agent.id))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(agent.prompt.as_bytes())?;
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("Runner crashed for agent {}", agent.id))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok((stdout, stderr))
    } else {
        bail!(
            "runner exited with {}: {}{}",
            output.status,
            compact_log(&stdout),
            compact_log(&stderr)
        )
    }
}

pub(crate) fn normalize_runner_profile(profile: Option<String>) -> Option<String> {
    profile
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value != "default")
}

pub(crate) fn runner_model_for_profile(profile: &str) -> Option<String> {
    match profile {
        "default" => env::var("PATCHBAY_MODEL").ok(),
        other => Some(other.to_string()),
    }
}

pub(crate) fn compact_log(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= 800 {
        trimmed.to_string()
    } else {
        format!("…{}", &trimmed[trimmed.len() - 800..])
    }
}

pub(crate) fn next_nested_agent_id(
    mission: &Mission,
    parent_id: Option<&str>,
    project: &RegisteredProject,
    task: &str,
) -> String {
    let base_slug = slug_text(task)
        .split('-')
        .take(4)
        .collect::<Vec<_>>()
        .join("-");
    let base = match parent_id {
        Some(parent_id) => format!("{}-{}", parent_id, base_slug),
        None => format!(
            "agent-{:02}-{}",
            mission.agents.len() + 1,
            project.name.to_ascii_lowercase()
        ),
    };
    if !mission.agents.iter().any(|agent| agent.id == base) {
        return base;
    }
    for index in 2..1000 {
        let candidate = format!("{base}-{index}");
        if !mission.agents.iter().any(|agent| agent.id == candidate) {
            return candidate;
        }
    }
    format!("{}-{}", base, Utc::now().timestamp())
}

pub(crate) fn slug_text(text: &str) -> String {
    let slug = text
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "agent".to_string()
    } else {
        slug
    }
}

pub(crate) fn list_mission_command(project: &Path) -> Result<()> {
    let missions = list_missions(project)?;
    if missions.is_empty() {
        println!("No Patchbay missions recorded.");
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
        None => match latest_mission(project) {
            Ok(mission) => mission,
            Err(_) => {
                println!("No Patchbay missions recorded.");
                println!(
                    "Create one with `pb mission start --task project=task` or open `pb` for Mission Control."
                );
                return Ok(());
            }
        },
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
            "Agent project `{}` is not an initialized Patchbay revision project",
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
            "No project tasks found. Use --task project=task or text lines like `Patchbay: build X`. Known projects: {}",
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
        parent_id: None,
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
        parent_id: None,
    }))
}

pub(crate) struct MissionSkillPreset {
    pub(crate) name: &'static str,
    pub(crate) skills: &'static [&'static str],
    pub(crate) guidance: &'static str,
}

pub(crate) fn classify_mission_skill_preset(source_text: &str, task: &str) -> MissionSkillPreset {
    let text = format!("{} {}", source_text, task).to_ascii_lowercase();
    if contains_any(
        &text,
        &[
            "security",
            "secure",
            "vulnerability",
            "auth",
            "permission",
            "token",
            "secret",
        ],
    ) {
        return MissionSkillPreset {
            name: "security",
            skills: &["security-review", "best-practices", "test"],
            guidance: "Threat-model the change, protect secrets, and verify abuse/error paths before marking work complete.",
        };
    }
    if contains_any(
        &text,
        &[
            "review",
            "pull request",
            "diff",
            "audit",
            "sprawdz",
            "sprawdź",
            "code quality",
        ],
    ) {
        return MissionSkillPreset {
            name: "review",
            skills: &["review", "best-practices", "test"],
            guidance: "Review the actual diff first, file concrete findings with paths, and only fix issues that are in scope.",
        };
    }
    if contains_any(
        &text,
        &[
            "debug", "bug", "fix", "napraw", "crash", "failure", "failed", "error",
        ],
    ) {
        return MissionSkillPreset {
            name: "debug",
            skills: &["debug-like-expert", "test", "verify-before-complete"],
            guidance: "Reproduce first, form hypotheses from evidence, fix root causes, then rerun the failing proof.",
        };
    }
    if contains_any(
        &text,
        &[
            "frontend",
            "ui",
            "ux",
            "component",
            "page",
            "landing",
            "dashboard",
            "mobile",
            "ios",
            "swift",
        ],
    ) {
        return MissionSkillPreset {
            name: "frontend",
            skills: &[
                "frontend-design",
                "make-interfaces-feel-better",
                "accessibility",
                "test",
            ],
            guidance: "Build a polished user-facing slice, verify interaction behavior, and check accessibility signals.",
        };
    }
    if contains_any(
        &text,
        &["test", "tests", "tdd", "spec", "coverage", "regression"],
    ) {
        return MissionSkillPreset {
            name: "test",
            skills: &["tdd", "test", "verify-before-complete"],
            guidance: "Prefer observable contracts, add or run focused tests, and keep the red-green proof visible.",
        };
    }
    if contains_any(
        &text,
        &[
            "doc", "docs", "readme", "proposal", "rfc", "brief", "prd", "write up", "opisz",
        ],
    ) {
        return MissionSkillPreset {
            name: "docs",
            skills: &["write-docs", "write-milestone-brief"],
            guidance: "Write for a fresh reader, preserve current state only, and verify paths/commands against the repo.",
        };
    }
    if contains_any(
        &text,
        &[
            "plan",
            "decompose",
            "break",
            "slice",
            "roadmap",
            "milestone",
            "zaplanuj",
        ],
    ) {
        return MissionSkillPreset {
            name: "planning",
            skills: &[
                "decompose-into-slices",
                "write-milestone-brief",
                "design-an-interface",
            ],
            guidance: "Turn ambiguity into thin vertical slices, retire the riskiest unknowns first, and document assumptions.",
        };
    }
    MissionSkillPreset {
        name: "implementation",
        skills: &["test", "review", "verify-before-complete"],
        guidance: "Implement the smallest complete vertical slice, keep changes observable, and verify before completion.",
    }
}

pub(crate) fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

pub(crate) fn build_mission_agent_prompt(
    mission_title: &str,
    source_text: &str,
    project: &RegisteredProject,
    task: &str,
    skill_preset: &MissionSkillPreset,
) -> String {
    let skills = skill_preset.skills.join(", ");
    format!(
        "Mission: {mission_title}\nProject: {}\nProject path: {}\n\nTask:\n{task}\n\nSource mission text:\n{}\n\nPatchbay routing preset: {}\nRecommended GSD skills: {}\nPreset guidance: {}\n\nYou are a Patchbay loop agent running inside GSD. If the task begins with `Root planner:`, first decompose the source request into the smallest useful set of child agents; create them with `${{PATCHBAY_BIN:-pb}} mission add-agent $PATCHBAY_MISSION_ID --parent $PATCHBAY_AGENT_ID --project <project> --task '<child task>'`, then run them with `${{PATCHBAY_BIN:-pb}} mission run $PATCHBAY_MISSION_ID --all` when autonomous execution is appropriate. For non-root tasks, work autonomously unless parallel research, implementation, review, or project-specific work makes child agents useful. Child agents may create their own children using the same command. Use the recommended GSD skills above when applicable; load their instructions before doing matching work. Coordinate child work through Patchbay, inspect their results, continue looping until the original task is complete, and verify before completion. When done, report status with `${{PATCHBAY_BIN:-pb}} mission update-agent $PATCHBAY_MISSION_ID $PATCHBAY_AGENT_ID --status complete --summary '<summary>'`; on failure use --status failed --error '<error>'.",
        project.name,
        project.path.display(),
        source_text.trim(),
        skill_preset.name,
        skills,
        skill_preset.guidance
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
        .ok_or_else(|| anyhow!("No Patchbay missions recorded."))
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
    mission.status = if mission.agents.is_empty() {
        MissionStatus::Waiting
    } else if mission
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
        MissionStatus::Waiting => "queued",
        MissionStatus::Complete => "complete",
        MissionStatus::Failed => "failed",
        MissionStatus::Blocked => "blocked",
    }
}

pub(crate) fn format_mission_agent_status(status: &MissionAgentStatus) -> &'static str {
    match status {
        MissionAgentStatus::Pending => "pending",
        MissionAgentStatus::Running => "running",
        MissionAgentStatus::Waiting => "queued",
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

pub(crate) fn current_project_display_name(project: &Path) -> String {
    let cargo_toml = project.join("Cargo.toml");
    if let Ok(text) = fs::read_to_string(cargo_toml) {
        if let Ok(value) = text.parse::<toml::Value>() {
            if let Some(name) = value
                .get("package")
                .and_then(|package| package.get("name"))
                .and_then(toml::Value::as_str)
            {
                return if name == "patchbay" {
                    "Patchbay".to_string()
                } else {
                    name.to_string()
                };
            }
        }
    }
    project
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("current")
        .to_string()
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

    let current_name = current_project_display_name(project);
    if current_name.starts_with('.') {
        discover_project_directories(&mut registry);
    }

    if (registry.projects.is_empty() || current_name.starts_with('.'))
        && !registry.projects.contains_key(&current_name)
    {
        registry.projects.insert(
            current_name.clone(),
            RegisteredProject {
                name: current_name,
                path: project.to_path_buf(),
                description: Some("Current Patchbay project".to_string()),
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

pub(crate) fn discover_project_directories(registry: &mut ProjectRegistry) {
    let mut roots = Vec::new();
    if let Ok(home) = env::var("HOME") {
        let home = PathBuf::from(home);
        roots.push(home.join("Dev"));
        roots.push(home.join("Developer"));
        roots.push(home.join("Code"));
        roots.push(home.join("Projects"));
    }
    if let Ok(cwd) = env::current_dir() {
        roots.push(cwd.clone());
        if let Some(parent) = cwd.parent() {
            roots.push(parent.to_path_buf());
        }
    }

    let mut seen_roots = BTreeSet::new();
    for root in roots {
        if !seen_roots.insert(root.clone()) || !root.is_dir() {
            continue;
        }
        maybe_register_project_dir(registry, &root);
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                maybe_register_project_dir(registry, &path);
            }
        }
    }
}

pub(crate) fn maybe_register_project_dir(registry: &mut ProjectRegistry, path: &Path) {
    if env::var("HOME").is_ok_and(|home| path == Path::new(&home)) {
        return;
    }
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return;
    };
    if name.starts_with('.') || registry.resolve(name).is_ok() {
        return;
    }
    let looks_like_project = path.join(".git").exists()
        || path.join("Cargo.toml").exists()
        || path.join("package.json").exists()
        || path.join(".ai-revisions").exists()
        || path.join(".gsd").exists();
    if !looks_like_project {
        return;
    }
    registry.projects.insert(
        name.to_string(),
        RegisteredProject {
            name: name.to_string(),
            path: path.to_path_buf(),
            description: Some("Discovered local project directory".to_string()),
        },
    );
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
