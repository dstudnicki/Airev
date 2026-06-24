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

    assert_eq!(theme.background, Color::Reset);
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

#[test]
fn start_mission_creates_project_scoped_agent_records() {
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

#[test]
fn update_mission_agent_records_status_summary_and_diffs() {
    let project = temp_project("mission-update");
    write_project_registry_config(&project, "[projects.Airev]\npath = \".\"\n");
    start_mission(
        &project,
        Some("Update mission".to_string()),
        Some("Airev: Update agent".to_string()),
        None,
        Vec::new(),
    )
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
