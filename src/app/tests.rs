use super::*;

#[test]
fn read_text_arg_returns_none_for_missing_file() {
    let missing = std::env::temp_dir().join(format!("airev-missing-prompt-{}", std::process::id()));

    let result =
        read_text_arg(None, Some(missing)).expect("missing prompt files should be ignored");

    assert_eq!(result, None);
}

#[test]
fn read_text_arg_rejects_inline_and_file_together() {
    let result = read_text_arg(
        Some("prompt".to_string()),
        Some(PathBuf::from("prompt.txt")),
    );

    assert!(result.is_err());
}

#[test]
fn parse_terminal_diff_assigns_old_and_new_line_numbers() {
    let target = DiffTarget {
        old_path: PathBuf::from("old"),
        new_path: PathBuf::from("new"),
        rel_path: "src/example.js".to_string(),
        has_old: true,
        has_new: true,
    };
    let diff = "diff --git a/old b/new\nindex 111..222 100644\n--- a/old\n+++ b/new\n@@ -10,3 +10,4 @@\n context\n-old\n+new\n+extra\n";

    let lines = parse_terminal_diff(diff, &target);
    let content_lines = lines
        .iter()
        .filter(|line| {
            matches!(
                line.kind,
                DiffLineKind::Context | DiffLineKind::Removed | DiffLineKind::Added
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(content_lines[0].old_line, Some(10));
    assert_eq!(content_lines[0].new_line, Some(10));
    assert_eq!(content_lines[1].old_line, Some(11));
    assert_eq!(content_lines[1].new_line, None);
    assert_eq!(content_lines[2].old_line, None);
    assert_eq!(content_lines[2].new_line, Some(11));
    assert_eq!(content_lines[3].old_line, None);
    assert_eq!(content_lines[3].new_line, Some(12));
}

#[test]
fn custom_theme_can_inherit_terminal_theme() {
    let mut custom_themes = BTreeMap::new();
    custom_themes.insert(
        "custom".to_string(),
        ThemeConfigFile {
            extends: Some("terminal".to_string()),
            accent: Some("#112233".to_string()),
            ..ThemeConfigFile::default()
        },
    );

    let resolved = resolve_theme_config("custom", &custom_themes, 0).expect("theme resolves");
    let theme = UiTheme::from_config(&resolved);

    assert_eq!(theme.background, Color::Rgb(11, 15, 20));
    assert_eq!(theme.foreground, Color::Reset);
    assert_eq!(theme.accent, Color::Rgb(0x11, 0x22, 0x33));
}

#[test]
fn mission_store_round_trips_missions() {
    let project = temp_project("mission-round-trip");
    let now = "2026-06-24T00:00:00Z".to_string();
    let mission = Mission {
        id: "mission_001".to_string(),
        title: "Demo mission".to_string(),
        source_text: "Airev: build mission control".to_string(),
        status: MissionStatus::Running,
        created_at: now.clone(),
        updated_at: now.clone(),
        agents: vec![MissionAgent {
            id: "agent_001".to_string(),
            parent_id: None,
            project: "airev".to_string(),
            project_path: project.display().to_string(),
            task: "Build registry".to_string(),
            prompt: "Work on S01".to_string(),
            status: MissionAgentStatus::Pending,
            created_at: now.clone(),
            updated_at: now,
            summary: None,
            last_error: None,
            revision_ids: vec![7],
            diff_refs: vec![MissionDiffRef {
                revision_id: Some(7),
                path: "src/main.rs".to_string(),
            }],
            prompt_preset: Some("implementation".to_string()),
            recommended_skills: vec!["test".to_string()],
            runner_profile: None,
            session_id: None,
            started_at: None,
            finished_at: None,
            last_log: None,
        }],
    };

    write_mission(&project, &mission).expect("mission writes");
    let read_back = read_mission(&project, "mission_001").expect("mission reads");
    assert_eq!(read_back, mission);

    let listed = list_missions(&project).expect("missions list");
    assert_eq!(listed, vec![mission]);

    fs::remove_dir_all(project).ok();
}

#[test]
fn mission_store_rejects_path_like_ids() {
    let project = temp_project("mission-invalid-id");
    let result = mission_path(&project, "../escape");

    assert!(result.is_err());
    fs::remove_dir_all(project).ok();
}

#[test]
fn project_registry_loads_named_projects_from_config() {
    let project = temp_project("registry");
    write_project_registry_config(
        &project,
        "[projects.airev]\npath = \"../Airev\"\ndescription = \"Mission Control repo\"\n",
    );

    let registry = load_project_registry(&project).expect("registry loads");
    let airev = registry.resolve("airev").expect("project exists");

    assert_eq!(airev.name, "airev");
    assert_eq!(airev.description.as_deref(), Some("Mission Control repo"));
    assert!(airev.path.ends_with("Airev"));
    assert_eq!(registry.names(), vec!["airev".to_string()]);

    fs::remove_dir_all(project).ok();
}

#[test]
fn parse_mission_tasks_accepts_prefixed_text_and_explicit_tasks() {
    let project = temp_project("mission-parser");
    write_project_registry_config(
        &project,
        "[projects.Airev]\npath = \".\"\n\n[projects.CashPilot]\npath = \"../CashPilot\"\n",
    );
    let registry = load_project_registry(&project).expect("registry loads");

    let tasks = parse_mission_tasks(
        &registry,
        "Airev: Build dispatcher\nCashPilot: Fix login",
        &["Airev=Write tests".to_string()],
    )
    .expect("tasks parse");

    assert_eq!(tasks.len(), 3);
    assert_eq!(tasks[0].project, "Airev");
    assert_eq!(tasks[0].task, "Write tests");
    assert_eq!(tasks[1].project, "Airev");
    assert_eq!(tasks[1].task, "Build dispatcher");
    assert_eq!(tasks[2].project, "CashPilot");
    assert_eq!(tasks[2].task, "Fix login");

    fs::remove_dir_all(project).ok();
}

#[tokio::test]
async fn start_mission_creates_project_scoped_agent_records() {
    let project = temp_project("mission-start");
    write_project_registry_config(
        &project,
        "[projects.Airev]\npath = \".\"\n\n[projects.CashPilot]\npath = \"../CashPilot\"\n",
    );

    start_mission(
        &project,
        Some("Daily mission".to_string()),
        Some("Airev: Add dispatcher\nCashPilot: Inspect diffs".to_string()),
        None,
        Vec::new(),
    )
    .await
    .expect("mission starts");

    let missions = list_missions(&project).expect("missions list");
    assert_eq!(missions.len(), 1);
    let mission = &missions[0];
    assert_eq!(mission.title, "Daily mission");
    assert_eq!(mission.agents.len(), 2);
    assert_eq!(mission.agents[0].id, "agent-01-airev");
    assert_eq!(mission.agents[0].project, "Airev");
    assert!(mission.agents[0].prompt.contains("Project path:"));
    assert!(mission.agents[0].prompt.contains("Add dispatcher"));
    assert_eq!(mission.agents[1].id, "agent-02-cashpilot");

    fs::remove_dir_all(project).ok();
}

#[tokio::test]
async fn update_mission_agent_records_status_summary_and_diffs() {
    let project = temp_project("mission-update");
    write_project_registry_config(&project, "[projects.Airev]\npath = \".\"\n");
    start_mission(
        &project,
        Some("Update mission".to_string()),
        Some("Airev: Update agent".to_string()),
        None,
        Vec::new(),
    )
    .await
    .expect("mission starts");
    let mission_id = list_missions(&project).expect("missions list")[0]
        .id
        .clone();

    update_mission_agent_command(
        &project,
        &mission_id,
        "agent-01-airev",
        Some(MissionAgentStatusArg::Complete),
        Some("Finished with tests".to_string()),
        None,
        vec![42],
        vec!["42:src/main.rs".to_string(), "README.md".to_string()],
    )
    .expect("agent updates");

    let mission = read_mission(&project, &mission_id).expect("mission reads");
    assert_eq!(mission.status, MissionStatus::Complete);
    assert_eq!(mission.agents[0].status, MissionAgentStatus::Complete);
    assert_eq!(
        mission.agents[0].summary.as_deref(),
        Some("Finished with tests")
    );
    assert_eq!(mission.agents[0].revision_ids, vec![42]);
    assert_eq!(mission.agents[0].diff_refs.len(), 2);
    assert_eq!(mission.agents[0].diff_refs[0].revision_id, Some(42));
    assert_eq!(mission.agents[0].diff_refs[0].path, "src/main.rs");
    assert_eq!(mission.agents[0].diff_refs[1].revision_id, None);
    assert_eq!(mission.agents[0].diff_refs[1].path, "README.md");
    mission_diffs_command(&project, &mission_id, Some("agent-01-airev"))
        .expect("diff command lists refs");

    fs::remove_dir_all(project).ok();
}

#[tokio::test]
async fn mission_open_diff_rejects_unbound_diff_refs() {
    let project = temp_project("mission-open-diff");
    write_project_registry_config(&project, "[projects.Airev]\npath = \".\"\n");
    start_mission(
        &project,
        Some("Open diff mission".to_string()),
        Some("Airev: Update agent".to_string()),
        None,
        Vec::new(),
    )
    .await
    .expect("mission starts");
    let mission_id = list_missions(&project).expect("missions list")[0]
        .id
        .clone();
    update_mission_agent_command(
        &project,
        &mission_id,
        "agent-01-airev",
        None,
        None,
        None,
        Vec::new(),
        vec!["README.md".to_string()],
    )
    .expect("agent updates");

    let result =
        mission_open_diff_command(&project, &mission_id, "agent-01-airev", 0, true, "code").await;

    assert!(result.is_err());
    assert!(format!("{}", result.unwrap_err()).contains("not bound to a revision id"));
    fs::remove_dir_all(project).ok();
}

#[test]
fn compose_mission_tasks_creates_single_root_orchestrator() {
    let project = temp_project("compose-root");
    write_project_registry_config(&project, "[projects.CashPilot]\npath = \".\"\n");
    let registry = load_project_registry(&project).expect("registry loads");

    let prompt = "CashPilot: onboarding, billing fixes, CSV export";
    let tasks = compose_mission_tasks(&registry, prompt).expect("compose parses");

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].project, "CashPilot");
    assert_eq!(tasks[0].task, prompt);
    assert_eq!(tasks[0].parent_id, None);
    fs::remove_dir_all(project).ok();
}

#[test]
fn tmux_shell_command_quotes_prompt_and_env() {
    let agent = MissionAgent {
        id: "agent-01-demo".to_string(),
        parent_id: Some("agent-root".to_string()),
        project: "Demo".to_string(),
        project_path: "/tmp/demo project".to_string(),
        task: "Handle Bob's prompt".to_string(),
        prompt: "Say 'hello' and use spaces".to_string(),
        status: MissionAgentStatus::Pending,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        summary: None,
        last_error: None,
        revision_ids: Vec::new(),
        diff_refs: Vec::new(),
        prompt_preset: Some("implementation".to_string()),
        recommended_skills: vec!["test".to_string()],
        runner_profile: None,
        session_id: None,
        started_at: None,
        finished_at: None,
        last_log: None,
    };

    let command =
        tmux_agent_shell_command(Path::new("/tmp/patch bay"), "mission-1", &agent, "fast");

    assert!(command.contains("PATCHBAY_HOME='/tmp/patch bay'"));
    assert!(command.contains("PATCHBAY_PARENT_AGENT_ID='agent-root'"));
    assert!(command.contains("PATCHBAY_BIN="));
    assert!(command.contains("exec gsd --print"));
    assert!(command.contains("'Say '\\''hello'\\'' and use spaces'"));
}

#[test]
fn tmux_names_are_sanitized() {
    assert_eq!(
        tmux_session_name("mission:one/two"),
        "patchbay-mission-one-two"
    );
    assert_eq!(tmux_window_name("agent:01/demo"), "agent-01-demo");
}

#[test]
fn skill_router_selects_frontend_and_review_presets() {
    let frontend = classify_mission_skill_preset("", "zaplanuj mobile iOS frontend dla CashPilot");
    assert_eq!(frontend.name, "frontend");
    assert!(frontend.skills.contains(&"frontend-design"));

    let review = classify_mission_skill_preset("", "review this pull request diff for regressions");
    assert_eq!(review.name, "review");
    assert!(review.skills.contains(&"review"));

    let implementation = classify_mission_skill_preset("", "implement project lifecycle monitor");
    assert_eq!(implementation.name, "implementation");
}

#[test]
fn mission_prompt_includes_routing_preset_and_skills() {
    let project = RegisteredProject {
        name: "Patchbay".to_string(),
        path: PathBuf::from("/tmp/patchbay"),
        description: None,
    };
    let preset = classify_mission_skill_preset("", "debug failing tmux lifecycle");
    let prompt = build_mission_agent_prompt(
        "mission",
        "debug failing tmux lifecycle",
        &project,
        "debug failing tmux lifecycle",
        &preset,
    );

    assert!(prompt.contains("Patchbay routing preset: debug"));
    assert!(
        prompt.contains("Recommended GSD skills: debug-like-expert, test, verify-before-complete")
    );
    assert!(prompt.contains("${PATCHBAY_BIN:-pb} mission add-agent"));
}

#[test]
fn tmux_session_ids_parse_for_reconnect() {
    assert_eq!(
        parse_tmux_session_id("tmux:patchbay-mission-1:%3"),
        Some(("patchbay-mission-1".to_string(), "%3".to_string()))
    );
    assert_eq!(parse_tmux_session_id("pty-123"), None);
}

#[test]
fn global_registry_discovers_project_directories_from_home_dev() {
    let home = temp_project("discover-home");
    let dev = home.join("Dev");
    let airev = dev.join("Airev");
    let cashpilot = dev.join("CashPilot");
    let patchbay = home.join(".patchbay");
    fs::create_dir_all(&airev).expect("airev dir");
    fs::create_dir_all(&cashpilot).expect("cashpilot dir");
    fs::create_dir_all(&patchbay).expect("patchbay dir");
    fs::write(airev.join("Cargo.toml"), "[package]\nname=\"airev\"\n").expect("airev marker");
    fs::write(cashpilot.join("package.json"), "{}\n").expect("cashpilot marker");

    let old_home = env::var("HOME").ok();
    let old_cwd = env::current_dir().expect("cwd");
    unsafe {
        env::set_var("HOME", &home);
    }
    env::set_current_dir(&home).expect("set cwd");
    let registry = load_project_registry(&patchbay).expect("registry loads");
    env::set_current_dir(old_cwd).expect("restore cwd");
    if let Some(old_home) = old_home {
        unsafe { env::set_var("HOME", old_home) };
    } else {
        unsafe { env::remove_var("HOME") };
    }

    assert_eq!(
        registry.resolve("Airev").expect("Airev discovered").path,
        airev
    );
    assert_eq!(
        registry
            .resolve("CashPilot")
            .expect("CashPilot discovered")
            .path,
        cashpilot
    );
    fs::remove_dir_all(home).ok();
}

#[test]
fn compose_ignores_hidden_patchbay_as_root_orchestrator() {
    let project = temp_project("compose-hidden-root");
    write_project_registry_config(
        &project,
        "[projects.\".patchbay\"]\npath = \".\"\n[projects.Airev]\npath = \".\"\n[projects.CashPilot]\npath = \".\"\n",
    );
    let registry = load_project_registry(&project).expect("registry loads");

    let tasks = compose_mission_tasks(&registry, "Odpal Airev i CashPilot").expect("compose parses");

    assert_eq!(tasks.len(), 1);
    assert_ne!(tasks[0].project, ".patchbay");
    assert_eq!(tasks[0].project, "Airev");
    fs::remove_dir_all(project).ok();
}

#[test]
fn compose_natural_multi_project_prompt_creates_only_root_orchestrator() {
    let project = temp_project("compose-orchestrator");
    write_project_registry_config(
        &project,
        "[projects.Patchbay]\npath = \".\"\n[projects.Airev]\npath = \".\"\n[projects.CashPilot]\npath = \".\"\n",
    );
    let registry = load_project_registry(&project).expect("registry loads");

    let prompt = "Odpal projekt Airev i CashPilot, dowiedz sie o co chodzi w kazdym z projektów i do cashpilot zaplanuj przepisanie webowego projektu na mobilke iOS w jezyku swift";
    let tasks = compose_mission_tasks(&registry, prompt).expect("compose parses");

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].project, "Patchbay");
    assert_eq!(tasks[0].task, prompt);
    assert_eq!(tasks[0].parent_id, None);
    fs::remove_dir_all(project).ok();
}

#[tokio::test]
async fn add_mission_agent_creates_child_agent() {
    let project = temp_project("add-child");
    write_project_registry_config(&project, "[projects.Airev]\npath = \".\"\n");
    start_mission(
        &project,
        Some("Parent mission".to_string()),
        Some("Airev: Parent task".to_string()),
        None,
        Vec::new(),
    )
    .await
    .expect("mission starts");
    let mission_id = latest_mission(&project).expect("latest mission").id;

    add_mission_agent_command(
        &project,
        &mission_id,
        Some("agent-01-airev"),
        "Airev",
        "Child task",
        None,
        true,
    )
    .await
    .expect("child agent added");

    let mission = read_mission(&project, &mission_id).expect("mission reads");
    let child = mission
        .agents
        .iter()
        .find(|agent| agent.parent_id.as_deref() == Some("agent-01-airev"))
        .expect("child exists");
    assert_eq!(child.task, "Child task");
    assert_eq!(child.runner_profile.as_deref(), Some("fast"));
    fs::remove_dir_all(project).ok();
}

#[tokio::test]
async fn run_mission_uses_runner_command_and_marks_agent_complete() {
    let project = temp_project("runner");
    write_project_registry_config(&project, "[projects.Airev]\npath = \".\"\n");
    start_mission(
        &project,
        Some("Runner mission".to_string()),
        Some("Airev: Run fake command".to_string()),
        None,
        Vec::new(),
    )
    .await
    .expect("mission starts");
    let mission_id = latest_mission(&project).expect("latest mission").id;

    unsafe {
        env::set_var("PATCHBAY_RUNNER_CMD", "printf runner-ok");
    }
    run_mission_command(
        &project,
        Some(&mission_id),
        Some("agent-01-airev"),
        false,
        None,
        false,
    )
    .expect("runner succeeds");
    unsafe {
        env::remove_var("PATCHBAY_RUNNER_CMD");
    }

    let mission = read_mission(&project, &mission_id).expect("mission reads");
    let agent = mission_agent(&mission, "agent-01-airev").expect("agent exists");
    assert_eq!(agent.status, MissionAgentStatus::Complete);
    assert_eq!(agent.summary.as_deref(), Some("runner-ok"));
    fs::remove_dir_all(project).ok();
}

fn write_project_registry_config(project: &Path, contents: &str) {
    let store = project.join(STORE_DIR);
    fs::create_dir_all(&store).expect("store dir");
    fs::write(store.join("config.toml"), contents).expect("config writes");
}

fn temp_project(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "airev-test-{label}-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&path).expect("temp project dir");
    path
}
