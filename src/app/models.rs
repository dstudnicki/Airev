use super::*;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ActiveTurn {
    pub(crate) prompt: String,
    pub(crate) title: String,
    pub(crate) started_at: String,
    pub(crate) baseline: BTreeMap<String, String>,
    pub(crate) touched: BTreeSet<String>,
    pub(crate) pre_snapshots: BTreeMap<String, String>,
}

#[derive(Debug)]
pub(crate) struct ChangedFile {
    pub(crate) path: String,
    pub(crate) change_type: String,
    pub(crate) old_snapshot_path: Option<String>,
    pub(crate) new_snapshot_path: Option<String>,
}

pub(crate) struct DiffTarget {
    pub(crate) old_path: PathBuf,
    pub(crate) new_path: PathBuf,
    pub(crate) rel_path: String,
    pub(crate) has_old: bool,
    pub(crate) has_new: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct RevisionRow {
    pub(crate) label: String,
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) changed_file_count: i64,
    pub(crate) created_at: String,
}

#[derive(Clone, Debug)]
pub(crate) struct FileRow {
    pub(crate) path: String,
    pub(crate) change_type: String,
    pub(crate) status: String,
}

#[derive(Clone, Debug)]
pub(crate) struct DiffLine {
    pub(crate) text: String,
    pub(crate) kind: DiffLineKind,
    pub(crate) old_line: Option<u32>,
    pub(crate) new_line: Option<u32>,
}

#[derive(Clone, Debug)]
pub(crate) enum DiffLineKind {
    Header,
    Hunk,
    Added,
    Removed,
    Context,
    Meta,
}

#[derive(Clone)]
pub(crate) enum TuiView {
    Revisions,
    Files {
        revision: RevisionRow,
    },
    Diff {
        revision: RevisionRow,
        file: FileRow,
        lines: Vec<DiffLine>,
        raw: bool,
    },
    Themes {
        return_to: Box<TuiView>,
        return_selected: usize,
    },
}

pub(crate) struct TuiApp {
    pub(crate) view: TuiView,
    pub(crate) revisions: Vec<RevisionRow>,
    pub(crate) files: Vec<FileRow>,
    pub(crate) selected: usize,
    pub(crate) scroll: u16,
    pub(crate) last_refresh: Instant,
    pub(crate) message: String,
    pub(crate) settings: TuiSettings,
    pub(crate) theme_names: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MissionControlPanel {
    Main,
    Agents,
    Detail,
    Diffs,
    Projects,
    Composer,
}

pub(crate) struct AgentTerminalSession {
    pub(crate) tmux_session: String,
    pub(crate) tmux_pane: String,
    pub(crate) output: String,
}

pub(crate) struct MissionControlApp {
    pub(crate) mission_id: String,
    pub(crate) mission: Mission,
    pub(crate) missions: Vec<Mission>,
    pub(crate) selected_mission: usize,
    pub(crate) focus: MissionControlPanel,
    pub(crate) selected_agent: usize,
    pub(crate) selected_diff: usize,
    pub(crate) workspace_parent: Option<String>,
    pub(crate) projects: Vec<PathBuf>,
    pub(crate) selected_project: usize,
    pub(crate) launch_revision_project: Option<PathBuf>,
    pub(crate) compose_input: String,
    pub(crate) compose_agent_target: Option<String>,
    pub(crate) fast_profile: bool,
    pub(crate) terminal_input: bool,
    pub(crate) terminals: BTreeMap<String, AgentTerminalSession>,
    pub(crate) last_refresh: Instant,
    pub(crate) message: String,
    pub(crate) settings: TuiSettings,
}

#[derive(Clone, Debug)]
pub(crate) struct TuiSettings {
    pub(crate) theme_name: String,
    pub(crate) theme: UiTheme,
    pub(crate) ui: UiConfig,
}

#[derive(Clone, Debug)]
pub(crate) struct UiConfig {
    pub(crate) show_raw_git_headers: bool,
    pub(crate) show_line_numbers: bool,
    pub(crate) compact_file_bar: bool,
    pub(crate) syntect_theme: String,
}

#[derive(Clone, Debug)]
pub(crate) struct UiTheme {
    pub(crate) background: Color,
    pub(crate) foreground: Color,
    pub(crate) muted: Color,
    pub(crate) border: Color,
    pub(crate) accent: Color,
    pub(crate) selected_bg: Color,
    pub(crate) added_fg: Color,
    pub(crate) added_bg: Color,
    pub(crate) removed_fg: Color,
    pub(crate) removed_bg: Color,
    pub(crate) hunk_fg: Color,
    pub(crate) metadata_fg: Color,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct AirevConfigFile {
    pub(crate) theme: Option<String>,
    pub(crate) ui: Option<UiConfigFile>,
    pub(crate) themes: Option<BTreeMap<String, ThemeConfigFile>>,
    pub(crate) projects: Option<BTreeMap<String, ProjectConfigFile>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct ProjectConfigFile {
    pub(crate) path: String,
    pub(crate) description: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ProjectRegistry {
    pub(crate) projects: BTreeMap<String, RegisteredProject>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RegisteredProject {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) description: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum MissionStatus {
    Draft,
    Running,
    Waiting,
    Complete,
    Failed,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum MissionAgentStatus {
    Pending,
    Running,
    Waiting,
    Complete,
    Failed,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct MissionDiffRef {
    pub(crate) revision_id: Option<i64>,
    pub(crate) path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct MissionAgent {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) parent_id: Option<String>,
    pub(crate) project: String,
    pub(crate) project_path: String,
    pub(crate) task: String,
    pub(crate) prompt: String,
    pub(crate) status: MissionAgentStatus,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) summary: Option<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) revision_ids: Vec<i64>,
    pub(crate) diff_refs: Vec<MissionDiffRef>,
    #[serde(default)]
    pub(crate) prompt_preset: Option<String>,
    #[serde(default)]
    pub(crate) recommended_skills: Vec<String>,
    #[serde(default)]
    pub(crate) runner_profile: Option<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) started_at: Option<String>,
    #[serde(default)]
    pub(crate) finished_at: Option<String>,
    #[serde(default)]
    pub(crate) last_log: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MissionTaskSpec {
    pub(crate) project: String,
    pub(crate) task: String,
    pub(crate) parent_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct UiConfigFile {
    pub(crate) show_raw_git_headers: Option<bool>,
    pub(crate) show_line_numbers: Option<bool>,
    pub(crate) compact_file_bar: Option<bool>,
    pub(crate) syntect_theme: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct ThemeConfigFile {
    pub(crate) extends: Option<String>,
    pub(crate) background: Option<String>,
    pub(crate) foreground: Option<String>,
    pub(crate) muted: Option<String>,
    pub(crate) border: Option<String>,
    pub(crate) accent: Option<String>,
    pub(crate) selected_bg: Option<String>,
    pub(crate) added_fg: Option<String>,
    pub(crate) added_bg: Option<String>,
    pub(crate) removed_fg: Option<String>,
    pub(crate) removed_bg: Option<String>,
    pub(crate) hunk_fg: Option<String>,
    pub(crate) metadata_fg: Option<String>,
}

impl ThemeConfigFile {
    pub(crate) fn merge(&mut self, next: ThemeConfigFile) {
        if next.background.is_some() {
            self.background = next.background;
        }
        if next.foreground.is_some() {
            self.foreground = next.foreground;
        }
        if next.muted.is_some() {
            self.muted = next.muted;
        }
        if next.border.is_some() {
            self.border = next.border;
        }
        if next.accent.is_some() {
            self.accent = next.accent;
        }
        if next.selected_bg.is_some() {
            self.selected_bg = next.selected_bg;
        }
        if next.added_fg.is_some() {
            self.added_fg = next.added_fg;
        }
        if next.added_bg.is_some() {
            self.added_bg = next.added_bg;
        }
        if next.removed_fg.is_some() {
            self.removed_fg = next.removed_fg;
        }
        if next.removed_bg.is_some() {
            self.removed_bg = next.removed_bg;
        }
        if next.hunk_fg.is_some() {
            self.hunk_fg = next.hunk_fg;
        }
        if next.metadata_fg.is_some() {
            self.metadata_fg = next.metadata_fg;
        }
    }
}

impl ProjectRegistry {
    pub(crate) fn names(&self) -> Vec<String> {
        self.projects.keys().cloned().collect()
    }

    pub(crate) fn resolve(&self, name: &str) -> Result<&RegisteredProject> {
        if let Some(project) = self.projects.get(name) {
            return Ok(project);
        }
        let lowered = name.to_ascii_lowercase();
        self.projects
            .iter()
            .find(|(candidate, _)| candidate.to_ascii_lowercase() == lowered)
            .map(|(_, project)| project)
            .ok_or_else(|| {
                anyhow!(
                    "Unknown Patchbay project `{name}`. Known projects: {}",
                    self.names().join(", ")
                )
            })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct Mission {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) source_text: String,
    pub(crate) status: MissionStatus,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) agents: Vec<MissionAgent>,
}
