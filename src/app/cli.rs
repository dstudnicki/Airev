use super::*;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "pb")]
#[command(about = "Patchbay local review and mission control", version)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Initialize local Patchbay revision storage for this project.
    Init,
    /// Record an AI turn lifecycle.
    Turn {
        #[command(subcommand)]
        command: TurnCommand,
    },
    /// Record that an AI tool is about to touch a path.
    Touch { path: String },
    /// List revisions.
    Revisions,
    /// Show unreviewed local revision summary.
    Status,
    /// Install and inspect the Patchbay GSD adapter.
    Gsd {
        #[command(subcommand)]
        command: GsdCommand,
    },
    /// Compose a mission from a natural-language request.
    Compose {
        /// User request, e.g. "Fix the login bug, review the billing flow, and add tests".
        #[arg(long)]
        text: Option<String>,
        /// Read the user request from a file.
        #[arg(long = "text-file")]
        text_file: Option<PathBuf>,
        /// Run pending root agents immediately after creating the mission.
        #[arg(long)]
        run: bool,
        /// Model/profile to use when generated agents run, e.g. gpt-5.1. Omit for GSD default.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Create and inspect multi-project Mission Control work.
    Mission {
        #[command(subcommand)]
        command: MissionCommand,
    },
    /// Show a revision/file diff.
    Diff {
        revision: String,
        path: String,
        /// Render the diff in the terminal instead of opening an editor.
        #[arg(long)]
        terminal: bool,
        #[arg(long, default_value = "code")]
        editor: String,
    },
    /// Mark a revision as reviewed.
    Reviewed { revision: String },
    /// List, inspect, or set Patchbay UI themes.
    Theme {
        #[command(subcommand)]
        command: ThemeCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum GsdCommand {
    /// Install the Patchbay adapter into the global GSD/pi extensions directory.
    Install,
    /// Show Patchbay and GSD adapter installation status.
    Status,
}

#[derive(Subcommand)]
pub(crate) enum MissionCommand {
    /// Create a text-only Mission Control run from project-prefixed tasks.
    Start {
        /// Human-readable mission title.
        #[arg(long)]
        title: Option<String>,
        /// Mission source text. Lines like `Patchbay: build X` become project tasks.
        #[arg(long)]
        text: Option<String>,
        /// Read mission source text from a file.
        #[arg(long = "text-file")]
        text_file: Option<PathBuf>,
        /// Explicit project task in `project=task` or `project:task` form. Repeatable.
        #[arg(long = "task")]
        tasks: Vec<String>,
    },
    /// Add a child agent under an existing agent.
    AddAgent {
        mission: String,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: String,
        #[arg(long)]
        title: Option<String>,
        /// Model/profile to use when this agent runs. Omit for GSD default.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Run one pending agent or all pending agents in the current workspace.
    Run {
        mission: Option<String>,
        agent: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        profile: Option<String>,
    },
    /// List stored missions.
    List,
    /// Show a stored mission with generated agent prompts.
    Show { mission: String },
    /// Show compact mission status. Defaults to the newest mission.
    Status { mission: Option<String> },
    /// Update a mission agent status, summary, error, revisions, or diff references.
    UpdateAgent {
        mission: String,
        agent: String,
        #[arg(long, value_enum)]
        status: Option<MissionAgentStatusArg>,
        #[arg(long)]
        summary: Option<String>,
        #[arg(long)]
        error: Option<String>,
        #[arg(long = "revision")]
        revisions: Vec<i64>,
        #[arg(long = "diff")]
        diffs: Vec<String>,
    },
    /// List mission diff references grouped by project and agent.
    Diffs {
        mission: String,
        agent: Option<String>,
    },
    /// Open a stored mission diff reference through Patchbay's existing diff flow.
    OpenDiff {
        mission: String,
        agent: String,
        /// Zero-based diff index as shown by `mission diffs`.
        diff_index: usize,
        /// Render the diff in the terminal instead of opening an editor.
        #[arg(long)]
        terminal: bool,
        #[arg(long, default_value = "code")]
        editor: String,
    },
    /// Open the non-voice Mission Control window-manager dashboard.
    Wm { mission: Option<String> },
}

#[derive(Clone, Debug, ValueEnum)]
pub(crate) enum MissionAgentStatusArg {
    Pending,
    Running,
    Waiting,
    Complete,
    Failed,
    Blocked,
}

#[derive(Subcommand)]
pub(crate) enum ThemeCommand {
    /// List built-in and custom themes visible to this project.
    List,
    /// Show the currently selected theme and config paths.
    Show,
    /// Persist a theme selection.
    Set {
        name: String,
        /// Write to ~/.config/patchbay/config.toml instead of .ai-revisions/config.toml.
        #[arg(long)]
        global: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum TurnCommand {
    /// Begin an AI turn and capture a baseline manifest.
    Begin {
        #[arg(long)]
        prompt: Option<String>,
        #[arg(long)]
        prompt_file: Option<PathBuf>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        force: bool,
        /// Copy baseline file contents at turn start so shell-driven edits still get exact diffs.
        #[arg(long)]
        snapshot_baseline: bool,
    },
    /// End an AI turn and persist a revision if files changed.
    End {
        #[arg(long)]
        summary: Option<String>,
        #[arg(long)]
        summary_file: Option<PathBuf>,
        #[arg(long)]
        title: Option<String>,
    },
}
