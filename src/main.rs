use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use clap::{Parser, Subcommand};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SyntectStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use walkdir::WalkDir;

const STORE_DIR: &str = ".ai-revisions";
const DB_FILE: &str = "db.sqlite";
const RUNTIME_DIR: &str = "runtime";
const SNAPSHOTS_DIR: &str = "snapshots";
const ACTIVE_TURN_FILE: &str = "current-turn.json";
const BUILT_IN_THEME_NAMES: &[&str] = &[
    "terminal",
    "airev-dark",
    "catppuccin-mocha",
    "gruvbox-dark",
    "high-contrast",
];

#[derive(Parser)]
#[command(name = "airev")]
#[command(about = "Local AI revision journal", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize local Airev storage for this project.
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
    /// Show unreviewed revision summary.
    Status,
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
    /// List, inspect, or set Airev UI themes.
    Theme {
        #[command(subcommand)]
        command: ThemeCommand,
    },
}

#[derive(Subcommand)]
enum ThemeCommand {
    /// List built-in and custom themes visible to this project.
    List,
    /// Show the currently selected theme and config paths.
    Show,
    /// Persist a theme selection.
    Set {
        name: String,
        /// Write to ~/.config/airev/config.toml instead of .ai-revisions/config.toml.
        #[arg(long)]
        global: bool,
    },
}

#[derive(Subcommand)]
enum TurnCommand {
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

#[derive(Debug, Serialize, Deserialize)]
struct ActiveTurn {
    prompt: String,
    title: String,
    started_at: String,
    baseline: BTreeMap<String, String>,
    touched: BTreeSet<String>,
    pre_snapshots: BTreeMap<String, String>,
}

#[derive(Debug)]
struct ChangedFile {
    path: String,
    change_type: String,
    old_snapshot_path: Option<String>,
    new_snapshot_path: Option<String>,
}

struct DiffTarget {
    old_path: PathBuf,
    new_path: PathBuf,
    rel_path: String,
    has_old: bool,
    has_new: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir().context("Failed to read current directory")?;

    match cli.command {
        None => {
            let project = find_initialized_project_root(&cwd).unwrap_or(cwd);
            launch_tui(&project).await
        }
        Some(command) => match command {
            Commands::Init => init_project(&cwd).await,
            Commands::Turn { command } => match command {
                TurnCommand::Begin {
                    prompt,
                    prompt_file,
                    title,
                    force,
                    snapshot_baseline,
                } => {
                    let project = require_initialized_project_root(&cwd)?;
                    begin_turn(
                        &project,
                        prompt,
                        prompt_file,
                        title,
                        force,
                        snapshot_baseline,
                    )
                    .await
                }
                TurnCommand::End {
                    summary,
                    summary_file,
                    title,
                } => {
                    let project = require_initialized_project_root(&cwd)?;
                    end_turn(&project, summary, summary_file, title).await
                }
            },
            Commands::Touch { path } => {
                let project = require_initialized_project_root(&cwd)?;
                touch_path(&project, &path)
            }
            Commands::Revisions => {
                let project = require_initialized_project_root(&cwd)?;
                list_revisions(&project).await
            }
            Commands::Status => {
                let project = find_initialized_project_root(&cwd).unwrap_or(cwd);
                status(&project).await
            }
            Commands::Diff {
                revision,
                path,
                terminal,
                editor,
            } => {
                let project = require_initialized_project_root(&cwd)?;
                if terminal {
                    print_terminal_diff(&project, &revision, &path).await
                } else {
                    open_diff(&project, &revision, &path, &editor).await
                }
            }
            Commands::Reviewed { revision } => {
                let project = require_initialized_project_root(&cwd)?;
                mark_reviewed(&project, &revision).await
            }
            Commands::Theme { command } => {
                let project = find_initialized_project_root(&cwd).unwrap_or(cwd);
                handle_theme_command(&project, command)
            }
        },
    }
}

#[derive(Clone, Debug)]
struct RevisionRow {
    label: String,
    title: String,
    status: String,
    changed_file_count: i64,
    created_at: String,
}

#[derive(Clone, Debug)]
struct FileRow {
    path: String,
    change_type: String,
    status: String,
}

#[derive(Clone, Debug)]
struct DiffLine {
    text: String,
    kind: DiffLineKind,
    old_line: Option<u32>,
    new_line: Option<u32>,
}

#[derive(Clone, Debug)]
enum DiffLineKind {
    Header,
    Hunk,
    Added,
    Removed,
    Context,
    Meta,
}

#[derive(Clone)]
enum TuiView {
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

struct TuiApp {
    view: TuiView,
    revisions: Vec<RevisionRow>,
    files: Vec<FileRow>,
    selected: usize,
    scroll: u16,
    last_refresh: Instant,
    message: String,
    settings: TuiSettings,
    theme_names: Vec<String>,
}

#[derive(Clone, Debug)]
struct TuiSettings {
    theme_name: String,
    theme: UiTheme,
    ui: UiConfig,
}

#[derive(Clone, Debug)]
struct UiConfig {
    show_raw_git_headers: bool,
    show_line_numbers: bool,
    compact_file_bar: bool,
    syntect_theme: String,
}

#[derive(Clone, Debug)]
struct UiTheme {
    background: Color,
    foreground: Color,
    muted: Color,
    border: Color,
    accent: Color,
    selected_bg: Color,
    added_fg: Color,
    added_bg: Color,
    removed_fg: Color,
    removed_bg: Color,
    hunk_fg: Color,
    metadata_fg: Color,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct AirevConfigFile {
    theme: Option<String>,
    ui: Option<UiConfigFile>,
    themes: Option<BTreeMap<String, ThemeConfigFile>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct UiConfigFile {
    show_raw_git_headers: Option<bool>,
    show_line_numbers: Option<bool>,
    compact_file_bar: Option<bool>,
    syntect_theme: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct ThemeConfigFile {
    extends: Option<String>,
    background: Option<String>,
    foreground: Option<String>,
    muted: Option<String>,
    border: Option<String>,
    accent: Option<String>,
    selected_bg: Option<String>,
    added_fg: Option<String>,
    added_bg: Option<String>,
    removed_fg: Option<String>,
    removed_bg: Option<String>,
    hunk_fg: Option<String>,
    metadata_fg: Option<String>,
}

async fn launch_tui(project: &Path) -> Result<()> {
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

fn load_tui_settings(project: &Path) -> TuiSettings {
    let config = load_merged_config(project);
    let ui = UiConfig::from_config(config.ui.as_ref());
    let requested_theme_name = config.theme.as_deref().unwrap_or("terminal");
    let empty_themes = BTreeMap::new();
    let custom_themes = config.themes.as_ref().unwrap_or(&empty_themes);
    let (theme_name, theme_config) = resolve_theme_config(requested_theme_name, custom_themes, 0)
        .map(|theme| (requested_theme_name.to_string(), theme))
        .unwrap_or_else(|| ("terminal".to_string(), default_terminal_theme_config()));

    TuiSettings {
        theme_name,
        theme: UiTheme::from_config(&theme_config),
        ui,
    }
}

fn load_merged_config(project: &Path) -> AirevConfigFile {
    let mut config = AirevConfigFile::default();
    if let Some(global_config) = read_config_file(&global_config_path()) {
        config.merge(global_config);
    }
    if let Some(project_config) = read_config_file(&project_config_path(project)) {
        config.merge(project_config);
    }
    config
}

fn project_config_path(project: &Path) -> PathBuf {
    project.join(STORE_DIR).join("config.toml")
}

fn available_theme_names(project: &Path) -> Vec<String> {
    let config = load_merged_config(project);
    let mut names = BUILT_IN_THEME_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    if let Some(themes) = config.themes {
        for name in themes.keys() {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
    }
    names
}

fn handle_theme_command(project: &Path, command: ThemeCommand) -> Result<()> {
    match command {
        ThemeCommand::List => {
            let settings = load_tui_settings(project);
            for name in available_theme_names(project) {
                let marker = if name == settings.theme_name {
                    "*"
                } else {
                    " "
                };
                println!("{marker} {name}");
            }
            Ok(())
        }
        ThemeCommand::Show => {
            let settings = load_tui_settings(project);
            println!("Current theme: {}", settings.theme_name);
            println!("Project config: {}", project_config_path(project).display());
            println!("Global config:  {}", global_config_path().display());
            Ok(())
        }
        ThemeCommand::Set { name, global } => {
            ensure_theme_exists(project, &name)?;
            if global {
                write_theme_selection(&global_config_path(), &name)?;
                println!("Set global Airev theme to {name}");
            } else {
                write_theme_selection(&project_config_path(project), &name)?;
                println!("Set project Airev theme to {name}");
            }
            Ok(())
        }
    }
}

fn ensure_theme_exists(project: &Path, name: &str) -> Result<()> {
    if available_theme_names(project)
        .iter()
        .any(|theme| theme == name)
    {
        Ok(())
    } else {
        bail!("Unknown theme `{name}`. Run `airev theme list` to see available themes.")
    }
}

fn write_theme_selection(path: &Path, name: &str) -> Result<()> {
    let mut config = read_config_file(path).unwrap_or_default();
    config.theme = Some(name.to_string());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    fs::write(path, toml::to_string_pretty(&config)?)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

fn global_config_path() -> PathBuf {
    if let Ok(xdg_config_home) = env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg_config_home)
            .join("airev")
            .join("config.toml");
    }
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("airev")
            .join("config.toml");
    }
    PathBuf::from("airev-config.toml")
}

fn read_config_file(path: &Path) -> Option<AirevConfigFile> {
    let text = fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

impl AirevConfigFile {
    fn merge(&mut self, next: AirevConfigFile) {
        if next.theme.is_some() {
            self.theme = next.theme;
        }
        if let Some(next_ui) = next.ui {
            self.ui
                .get_or_insert_with(UiConfigFile::default)
                .merge(next_ui);
        }
        if let Some(next_themes) = next.themes {
            self.themes
                .get_or_insert_with(BTreeMap::new)
                .extend(next_themes);
        }
    }
}

impl UiConfigFile {
    fn merge(&mut self, next: UiConfigFile) {
        if next.show_raw_git_headers.is_some() {
            self.show_raw_git_headers = next.show_raw_git_headers;
        }
        if next.show_line_numbers.is_some() {
            self.show_line_numbers = next.show_line_numbers;
        }
        if next.compact_file_bar.is_some() {
            self.compact_file_bar = next.compact_file_bar;
        }
        if next.syntect_theme.is_some() {
            self.syntect_theme = next.syntect_theme;
        }
    }
}

impl UiConfig {
    fn from_config(config: Option<&UiConfigFile>) -> Self {
        Self {
            show_raw_git_headers: config
                .and_then(|item| item.show_raw_git_headers)
                .unwrap_or(false),
            show_line_numbers: config
                .and_then(|item| item.show_line_numbers)
                .unwrap_or(true),
            compact_file_bar: config
                .and_then(|item| item.compact_file_bar)
                .unwrap_or(true),
            syntect_theme: config
                .and_then(|item| item.syntect_theme.clone())
                .unwrap_or_else(|| "base16-ocean.dark".to_string()),
        }
    }
}

fn resolve_theme_config(
    name: &str,
    custom_themes: &BTreeMap<String, ThemeConfigFile>,
    depth: usize,
) -> Option<ThemeConfigFile> {
    if depth > 8 {
        return None;
    }

    let theme = custom_themes
        .get(name)
        .cloned()
        .or_else(|| built_in_theme_config(name))?;
    let parent_name = theme.extends.clone().unwrap_or_else(|| {
        if name == "terminal" {
            "".to_string()
        } else {
            "terminal".to_string()
        }
    });

    if parent_name.is_empty() {
        return Some(theme);
    }

    let mut parent = resolve_theme_config(&parent_name, custom_themes, depth + 1)
        .unwrap_or_else(default_terminal_theme_config);
    parent.merge(theme);
    Some(parent)
}

impl ThemeConfigFile {
    fn merge(&mut self, next: ThemeConfigFile) {
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

impl UiTheme {
    fn from_config(config: &ThemeConfigFile) -> Self {
        Self {
            background: parse_color(config.background.as_deref()).unwrap_or(Color::Reset),
            foreground: parse_color(config.foreground.as_deref()).unwrap_or(Color::Reset),
            muted: parse_color(config.muted.as_deref()).unwrap_or(Color::DarkGray),
            border: parse_color(config.border.as_deref()).unwrap_or(Color::DarkGray),
            accent: parse_color(config.accent.as_deref()).unwrap_or(Color::Cyan),
            selected_bg: parse_color(config.selected_bg.as_deref())
                .unwrap_or(Color::Rgb(52, 64, 91)),
            added_fg: parse_color(config.added_fg.as_deref()).unwrap_or(Color::Green),
            added_bg: parse_color(config.added_bg.as_deref()).unwrap_or(Color::Rgb(28, 66, 50)),
            removed_fg: parse_color(config.removed_fg.as_deref()).unwrap_or(Color::Red),
            removed_bg: parse_color(config.removed_bg.as_deref()).unwrap_or(Color::Rgb(82, 36, 44)),
            hunk_fg: parse_color(config.hunk_fg.as_deref()).unwrap_or(Color::Magenta),
            metadata_fg: parse_color(config.metadata_fg.as_deref()).unwrap_or(Color::DarkGray),
        }
    }
}

fn built_in_theme_config(name: &str) -> Option<ThemeConfigFile> {
    let mut theme = default_terminal_theme_config();
    match name {
        "terminal" => Some(theme),
        "airev-dark" => {
            theme.background = Some("#1f2130".to_string());
            theme.foreground = Some("#f3f4f8".to_string());
            theme.muted = Some("#7d8498".to_string());
            theme.border = Some("#555b73".to_string());
            theme.accent = Some("#5eead4".to_string());
            theme.selected_bg = Some("#34405b".to_string());
            Some(theme)
        }
        "catppuccin-mocha" => {
            theme.background = Some("#1e1e2e".to_string());
            theme.foreground = Some("#cdd6f4".to_string());
            theme.muted = Some("#6c7086".to_string());
            theme.border = Some("#45475a".to_string());
            theme.accent = Some("#89b4fa".to_string());
            theme.selected_bg = Some("#313244".to_string());
            theme.added_fg = Some("#a6e3a1".to_string());
            theme.added_bg = Some("#1f3a2d".to_string());
            theme.removed_fg = Some("#f38ba8".to_string());
            theme.removed_bg = Some("#3a1f2a".to_string());
            theme.hunk_fg = Some("#cba6f7".to_string());
            Some(theme)
        }
        "gruvbox-dark" => {
            theme.background = Some("#282828".to_string());
            theme.foreground = Some("#ebdbb2".to_string());
            theme.muted = Some("#928374".to_string());
            theme.border = Some("#665c54".to_string());
            theme.accent = Some("#83a598".to_string());
            theme.selected_bg = Some("#3c3836".to_string());
            theme.added_fg = Some("#b8bb26".to_string());
            theme.added_bg = Some("#32361a".to_string());
            theme.removed_fg = Some("#fb4934".to_string());
            theme.removed_bg = Some("#3c1f1e".to_string());
            theme.hunk_fg = Some("#d3869b".to_string());
            Some(theme)
        }
        "high-contrast" => {
            theme.background = Some("#000000".to_string());
            theme.foreground = Some("#ffffff".to_string());
            theme.muted = Some("#b0b0b0".to_string());
            theme.border = Some("#ffffff".to_string());
            theme.accent = Some("#00ffff".to_string());
            theme.selected_bg = Some("#333333".to_string());
            theme.added_fg = Some("#00ff00".to_string());
            theme.added_bg = Some("#003300".to_string());
            theme.removed_fg = Some("#ff5555".to_string());
            theme.removed_bg = Some("#330000".to_string());
            theme.hunk_fg = Some("#ffff00".to_string());
            Some(theme)
        }
        _ => None,
    }
}

fn default_terminal_theme_config() -> ThemeConfigFile {
    ThemeConfigFile {
        background: Some("terminal".to_string()),
        foreground: Some("terminal".to_string()),
        muted: Some("darkgray".to_string()),
        border: Some("darkgray".to_string()),
        accent: Some("cyan".to_string()),
        selected_bg: Some("#34405b".to_string()),
        added_fg: Some("green".to_string()),
        added_bg: Some("#1c4232".to_string()),
        removed_fg: Some("red".to_string()),
        removed_bg: Some("#52242c".to_string()),
        hunk_fg: Some("magenta".to_string()),
        metadata_fg: Some("darkgray".to_string()),
        ..ThemeConfigFile::default()
    }
}

fn parse_color(value: Option<&str>) -> Option<Color> {
    let value = value?.trim().to_ascii_lowercase();
    match value.as_str() {
        "terminal" | "reset" | "inherit" | "none" => Some(Color::Reset),
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "darkgrey" => Some(Color::DarkGray),
        "white" => Some(Color::White),
        _ => parse_hex_color(&value),
    }
}

fn parse_hex_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(red, green, blue))
}

async fn run_tui_loop(
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

fn revisions_help() -> String {
    "Enter: files · t: themes · r: reviewed · q: quit".to_string()
}

fn files_help() -> String {
    "d/Enter: terminal diff · e: editor diff · r: reviewed · Esc: prompts".to_string()
}

fn diff_help(raw: bool) -> String {
    if raw {
        "Raw git diff · g: readable diff · n/p: files · Esc: files · q: quit".to_string()
    } else {
        "Readable diff · g: raw git diff · n/p: files · Esc: files · q: quit".to_string()
    }
}

fn themes_help() -> String {
    "↑/↓: choose · Enter: apply project theme · Esc: prompts · q: quit".to_string()
}

fn help_for_view(app: &TuiApp) -> String {
    match &app.view {
        TuiView::Revisions => revisions_help(),
        TuiView::Files { .. } => files_help(),
        TuiView::Diff { raw, .. } => diff_help(*raw),
        TuiView::Themes { .. } => themes_help(),
    }
}

fn open_theme_picker(app: &mut TuiApp) {
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

async fn open_selected_terminal_diff(project: &Path, app: &mut TuiApp, raw: bool) -> Result<()> {
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

async fn refresh_tui_data(pool: &SqlitePool, app: &mut TuiApp) -> Result<()> {
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

fn render_tui(frame: &mut ratatui::Frame<'_>, app: &TuiApp) {
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

fn selected_file_position(app: &TuiApp, file: &FileRow) -> (usize, usize) {
    let count = app.files.len().max(1);
    let index = app
        .files
        .iter()
        .position(|candidate| candidate.path == file.path)
        .map(|value| value + 1)
        .unwrap_or_else(|| app.selected.saturating_add(1).min(count));
    (index, count)
}

fn render_header(frame: &mut ratatui::Frame<'_>, app: &TuiApp, area: Rect) {
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

fn render_navigation_panel(frame: &mut ratatui::Frame<'_>, app: &TuiApp, area: Rect) {
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

fn render_detail_panel(frame: &mut ratatui::Frame<'_>, app: &TuiApp, area: Rect) {
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

fn file_list_item(file: &FileRow, theme: &UiTheme, selected: bool) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        selection_bar(selected, theme),
        Span::styled(
            file.path.clone(),
            selected_style(file_path_style(&file.change_type, theme), theme, selected),
        ),
    ]))
}

fn theme_list_item(name: &str, settings: &TuiSettings, selected: bool) -> ListItem<'static> {
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

fn selection_bar(selected: bool, theme: &UiTheme) -> Span<'static> {
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

fn selected_style(style: Style, theme: &UiTheme, selected: bool) -> Style {
    if selected {
        style
            .bg(theme.selected_bg)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        style
    }
}

fn revision_list_item(
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

fn selected_theme_detail(app: &TuiApp) -> Vec<Line<'static>> {
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

fn selected_revision_detail(app: &TuiApp) -> Vec<Line<'static>> {
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

fn selected_file_detail(app: &TuiApp, revision: &RevisionRow) -> Vec<Line<'static>> {
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

fn render_diff_view(
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

fn panel_block<'a>(title: &'a str, theme: &UiTheme) -> Block<'a> {
    Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
}

fn status_span(status: &str, theme: &UiTheme) -> Span<'static> {
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

fn change_span(change_type: &str, theme: &UiTheme) -> Span<'static> {
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

fn file_path_style(change_type: &str, theme: &UiTheme) -> Style {
    let color = match change_type {
        "added" => theme.added_fg,
        "deleted" => theme.removed_fg,
        "renamed" => theme.hunk_fg,
        "modified" => theme.accent,
        _ => theme.muted,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn diff_lines_to_tui_lines(
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

fn diff_line_to_highlighted_line(
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

fn line_number_width(lines: &[&DiffLine]) -> usize {
    lines
        .iter()
        .filter_map(|line| display_line_number(line))
        .max()
        .map(|line| line.to_string().len().max(3))
        .unwrap_or(3)
}

fn line_number_spans(
    line: &DiffLine,
    width: usize,
    theme: &UiTheme,
    background: Option<Color>,
) -> Vec<Span<'static>> {
    let background = background.unwrap_or(Color::Reset);
    let number_style = Style::default().fg(theme.muted).bg(background);
    let marker_style = diff_gutter_style(&line.kind, theme).bg(background);
    vec![
        Span::styled(format_line_number(display_line_number(line), width), number_style),
        Span::styled(display_line_marker(line).to_string(), marker_style),
        Span::styled(" │ ", Style::default().fg(theme.border).bg(background)),
    ]
}

fn display_line_number(line: &DiffLine) -> Option<u32> {
    match line.kind {
        DiffLineKind::Removed => line.old_line,
        DiffLineKind::Added => line.new_line,
        DiffLineKind::Context => line.new_line.or(line.old_line),
        _ => line.new_line.or(line.old_line),
    }
}

fn display_line_marker(line: &DiffLine) -> &'static str {
    match line.kind {
        DiffLineKind::Removed => "−",
        DiffLineKind::Added => "+",
        _ => " ",
    }
}

fn format_line_number(line: Option<u32>, width: usize) -> String {
    line.map(|value| format!("{value:>width$}"))
        .unwrap_or_else(|| " ".repeat(width))
}

fn hunk_label(raw: &str) -> String {
    if let Some((old_start, new_start)) = parse_hunk_starts(raw) {
        format!("Changed around old line {old_start}, new line {new_start}")
    } else {
        raw.to_string()
    }
}

fn parse_hunk_starts(raw: &str) -> Option<(u32, u32)> {
    let mut parts = raw.split_whitespace();
    parts.next()?;
    let old_part = parts.next()?;
    let new_part = parts.next()?;
    Some((
        parse_hunk_start(old_part, '-')?,
        parse_hunk_start(new_part, '+')?,
    ))
}

fn parse_hunk_start(part: &str, prefix: char) -> Option<u32> {
    let value = part.strip_prefix(prefix)?;
    let start = value.split(',').next()?;
    start.parse::<u32>().ok()
}

fn split_diff_prefix(text: &str) -> (&str, &str) {
    match text.as_bytes().first().copied() {
        Some(b'+') => ("+", &text[1..]),
        Some(b'-') => ("−", &text[1..]),
        Some(b' ') => (" ", &text[1..]),
        _ => (" ", text),
    }
}

fn diff_gutter_style(kind: &DiffLineKind, theme: &UiTheme) -> Style {
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

fn syntect_to_ratatui_style(style: SyntectStyle, background: Option<Color>) -> Style {
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

fn label_style(theme: &UiTheme) -> Style {
    Style::default()
        .fg(theme.muted)
        .add_modifier(Modifier::BOLD)
}

fn key_style(theme: &UiTheme) -> Style {
    Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD)
}

fn revision_number(label: &str) -> String {
    label
        .strip_prefix('r')
        .and_then(|value| value.parse::<i64>().ok())
        .map(|value| format!("#{value}"))
        .unwrap_or_else(|| label.to_string())
}

fn revision_display_title(title: &str) -> String {
    let normalized = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = if normalized.is_empty() {
        "AI code changes".to_string()
    } else {
        normalized
    };
    truncate_pretty(&sentence_case_title(&title), 48)
}

fn sentence_case_title(title: &str) -> String {
    let mut chars = title.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut output = first.to_uppercase().collect::<String>();
    output.push_str(chars.as_str());
    output
        .replace(" js ", " JS ")
        .replace(" tui", " TUI")
        .replace(" ui", " UI")
        .replace(" api", " API")
        .replace(" gsd", " GSD")
        .replace(" airev", " Airev")
}

fn short_timestamp(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.format("%H:%M").to_string())
        .unwrap_or_else(|_| value.chars().take(16).collect())
}

fn truncate_pretty(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut output = String::new();
    for word in value.split_whitespace() {
        let next_len =
            output.chars().count() + if output.is_empty() { 0 } else { 1 } + word.chars().count();
        if next_len > max_chars.saturating_sub(1) {
            break;
        }
        if !output.is_empty() {
            output.push(' ');
        }
        output.push_str(word);
    }
    if output.is_empty() {
        value
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>()
            + "…"
    } else {
        output + "…"
    }
}

fn plural(count: i64) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn current_len(app: &TuiApp) -> usize {
    match app.view {
        TuiView::Revisions => app.revisions.len(),
        TuiView::Files { .. } => app.files.len(),
        TuiView::Themes { .. } => app.theme_names.len(),
        TuiView::Diff { .. } => 0,
    }
}

fn move_selection(app: &mut TuiApp, delta: isize) {
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

fn suspend_tui(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn resume_tui(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    terminal.clear()?;
    Ok(())
}

async fn fetch_revisions(pool: &SqlitePool) -> Result<Vec<RevisionRow>> {
    let rows = sqlx::query_as::<_, (String, String, String, i64, String)>(
        "SELECT revision_label, title, status, changed_file_count, created_at \
         FROM revisions ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(label, title, status, changed_file_count, created_at)| RevisionRow {
                label,
                title,
                status,
                changed_file_count,
                created_at,
            },
        )
        .collect())
}

async fn fetch_revision_files(pool: &SqlitePool, revision: &str) -> Result<Vec<FileRow>> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT rf.path, rf.change_type, rf.status \
         FROM revision_files rf \
         JOIN revisions r ON r.id = rf.revision_id \
         WHERE r.revision_label = ? \
         ORDER BY rf.path",
    )
    .bind(normalize_revision_label(revision))
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(path, change_type, status)| FileRow {
            path,
            change_type,
            status,
        })
        .collect())
}

async fn mark_file_reviewed(pool: &SqlitePool, revision: &str, path: &str) -> Result<()> {
    let revision = normalize_revision_label(revision);
    let revision_id: i64 = sqlx::query_scalar("SELECT id FROM revisions WHERE revision_label = ?")
        .bind(&revision)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow!("Revision `{revision}` not found"))?;

    sqlx::query("UPDATE revision_files SET status = 'reviewed' WHERE revision_id = ? AND path = ?")
        .bind(revision_id)
        .bind(path)
        .execute(pool)
        .await?;

    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM revision_files WHERE revision_id = ? AND status != 'reviewed'",
    )
    .bind(revision_id)
    .fetch_one(pool)
    .await?;

    if remaining == 0 {
        sqlx::query("UPDATE revisions SET status = 'reviewed' WHERE id = ?")
            .bind(revision_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

fn find_initialized_project_root(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        if dir.join(STORE_DIR).join(DB_FILE).exists() {
            return Some(dir.to_path_buf());
        }
    }
    None
}

fn require_initialized_project_root(start: &Path) -> Result<PathBuf> {
    find_initialized_project_root(start).ok_or_else(|| {
        anyhow!(
            "Airev is not initialized for this directory tree. Run `airev init` in the project root first."
        )
    })
}

async fn init_project(project: &Path) -> Result<()> {
    ensure_store_dirs(project)?;
    ensure_gitignore_entry(project)?;
    let pool = open_db(project).await?;
    migrate(&pool).await?;
    ensure_session(&pool, project).await?;
    println!("Initialized {}", project.join(STORE_DIR).display());
    Ok(())
}

fn ensure_gitignore_entry(project: &Path) -> Result<()> {
    let gitignore_path = project.join(".gitignore");
    let entry = ".ai-revisions/";

    let existing = if gitignore_path.exists() {
        fs::read_to_string(&gitignore_path)
            .with_context(|| format!("Failed to read {}", gitignore_path.display()))?
    } else {
        String::new()
    };

    let already_ignored = existing.lines().map(str::trim).any(|line| {
        matches!(
            line,
            ".ai-revisions" | ".ai-revisions/" | "/.ai-revisions" | "/.ai-revisions/"
        )
    });

    if already_ignored {
        return Ok(());
    }

    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(entry);
    updated.push('\n');

    fs::write(&gitignore_path, updated)
        .with_context(|| format!("Failed to update {}", gitignore_path.display()))?;
    Ok(())
}

async fn begin_turn(
    project: &Path,
    prompt: Option<String>,
    prompt_file: Option<PathBuf>,
    title: Option<String>,
    force: bool,
    snapshot_baseline: bool,
) -> Result<()> {
    init_project(project).await?;

    let active_path = active_turn_path(project);
    if active_path.exists() && !force {
        bail!("An Airev turn is already active. Use --force to replace stale turn state.");
    }

    let prompt_text = read_text_arg(prompt, prompt_file)?.unwrap_or_default();
    let title = title.unwrap_or_else(|| title_from_prompt(&prompt_text));

    clear_runtime(project)?;
    fs::create_dir_all(runtime_pre_dir(project))?;

    let baseline = scan_manifest(project)?;
    let pre_snapshots = if snapshot_baseline {
        snapshot_baseline_files(project, baseline.keys())?
    } else {
        BTreeMap::new()
    };
    let turn = ActiveTurn {
        prompt: prompt_text,
        title,
        started_at: Utc::now().to_rfc3339(),
        baseline,
        touched: BTreeSet::new(),
        pre_snapshots,
    };

    write_active_turn(project, &turn)?;
    println!("Airev turn started: {}", turn.title);
    Ok(())
}

fn snapshot_baseline_files<'a>(
    project: &Path,
    paths: impl IntoIterator<Item = &'a String>,
) -> Result<BTreeMap<String, String>> {
    let mut snapshots = BTreeMap::new();
    for rel in paths {
        let source = project.join(rel);
        if !source.is_file() {
            continue;
        }
        let pre_rel = Path::new("pre").join(rel);
        let destination = project.join(STORE_DIR).join(RUNTIME_DIR).join(&pre_rel);
        copy_file(&source, &destination)?;
        snapshots.insert(
            rel.clone(),
            path_to_forward_slashes(&PathBuf::from(RUNTIME_DIR).join(pre_rel)),
        );
    }
    Ok(snapshots)
}

fn touch_path(project: &Path, input_path: &str) -> Result<()> {
    let mut turn = read_active_turn(project)?
        .ok_or_else(|| anyhow!("No active Airev turn. Run `airev turn begin` first."))?;
    let rel = normalize_project_path(project, input_path)?;
    turn.touched.insert(rel.clone());

    let source = project.join(&rel);
    if source.is_file() && !turn.pre_snapshots.contains_key(&rel) {
        let pre_rel = Path::new("pre").join(&rel);
        let destination = project.join(STORE_DIR).join(RUNTIME_DIR).join(&pre_rel);
        copy_file(&source, &destination)?;
        turn.pre_snapshots.insert(
            rel.clone(),
            path_to_forward_slashes(&PathBuf::from(RUNTIME_DIR).join(pre_rel)),
        );
    }

    write_active_turn(project, &turn)?;
    println!("Airev touched: {rel}");
    Ok(())
}

async fn end_turn(
    project: &Path,
    summary: Option<String>,
    summary_file: Option<PathBuf>,
    title: Option<String>,
) -> Result<()> {
    let mut turn = read_active_turn(project)?
        .ok_or_else(|| anyhow!("No active Airev turn. Run `airev turn begin` first."))?;
    let has_explicit_title = title.is_some();
    if let Some(title) = title {
        turn.title = title;
    }

    let pool = open_initialized_db(project).await?;
    let session_id = ensure_session(&pool, project).await?;
    let current = scan_manifest(project)?;
    let changed_paths = diff_manifest(&turn.baseline, &current);

    if changed_paths.is_empty() {
        remove_active_turn(project)?;
        println!("No changed files; no Airev revision recorded.");
        return Ok(());
    }

    if !has_explicit_title {
        turn.title = title_from_revision(&turn.prompt, &changed_paths);
    }

    let parent_revision_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM revisions WHERE session_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(&pool)
    .await?;

    let created_at = Utc::now().to_rfc3339();
    let assistant_summary = read_text_arg(summary, summary_file)?.unwrap_or_default();

    let mut tx = pool.begin().await?;
    let revision_result = sqlx::query(
        "INSERT INTO revisions \
         (session_id, parent_revision_id, created_at, title, prompt, assistant_summary, git_head, changed_file_count, status) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'unreviewed')",
    )
    .bind(session_id)
    .bind(parent_revision_id)
    .bind(&created_at)
    .bind(&turn.title)
    .bind(&turn.prompt)
    .bind(&assistant_summary)
    .bind(current_git_head(project))
    .bind(changed_paths.len() as i64)
    .execute(&mut *tx)
    .await?;

    let revision_id = revision_result.last_insert_rowid();
    let revision_label = format_revision_label(revision_id);
    sqlx::query("UPDATE revisions SET revision_label = ? WHERE id = ?")
        .bind(&revision_label)
        .bind(revision_id)
        .execute(&mut *tx)
        .await?;

    let changes =
        snapshot_changed_files(project, &revision_label, &changed_paths, &turn, &current)?;
    for change in &changes {
        sqlx::query(
            "INSERT INTO revision_files \
             (revision_id, path, change_type, old_snapshot_path, new_snapshot_path, additions, deletions, status) \
             VALUES (?, ?, ?, ?, ?, 0, 0, 'unreviewed')",
        )
        .bind(revision_id)
        .bind(&change.path)
        .bind(&change.change_type)
        .bind(&change.old_snapshot_path)
        .bind(&change.new_snapshot_path)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    remove_active_turn(project)?;

    println!(
        "Recorded {revision_label}: {} changed file(s) - {}",
        changes.len(),
        turn.title
    );
    Ok(())
}

async fn list_revisions(project: &Path) -> Result<()> {
    let pool = open_initialized_db(project).await?;
    let rows = sqlx::query_as::<_, (String, String, String, i64, String)>(
        "SELECT revision_label, title, status, changed_file_count, created_at \
         FROM revisions ORDER BY id DESC",
    )
    .fetch_all(&pool)
    .await?;

    if rows.is_empty() {
        println!("No Airev revisions recorded.");
        return Ok(());
    }

    for (label, title, status, count, created_at) in rows {
        println!("{label}  {status:<10}  {count:>3} file(s)  {created_at}  {title}");
    }
    Ok(())
}

async fn status(project: &Path) -> Result<()> {
    if !project.join(STORE_DIR).join(DB_FILE).exists() {
        println!("Airev is not initialized. Run `airev init`.");
        return Ok(());
    }

    let pool = open_initialized_db(project).await?;
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM revisions")
        .fetch_one(&pool)
        .await?;
    let unreviewed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM revisions WHERE status = 'unreviewed'")
            .fetch_one(&pool)
            .await?;
    let ignored: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM revisions WHERE status = 'ignored'")
            .fetch_one(&pool)
            .await?;

    println!("Airev revisions: {total} total, {unreviewed} unreviewed, {ignored} ignored");
    if unreviewed > 0 {
        println!("Run `airev revisions` to inspect them or `airev` for the TUI.");
    }
    Ok(())
}

async fn open_diff(project: &Path, revision: &str, input_path: &str, editor: &str) -> Result<()> {
    let target = resolve_diff_target(project, revision, input_path).await?;

    let status = Command::new(editor)
        .arg("--diff")
        .arg(&target.old_path)
        .arg(&target.new_path)
        .status()
        .with_context(|| format!("Failed to launch diff editor `{editor}`"))?;

    if !status.success() {
        bail!("Diff editor `{editor}` exited with {status}");
    }
    Ok(())
}

async fn print_terminal_diff(project: &Path, revision: &str, input_path: &str) -> Result<()> {
    for line in build_terminal_diff(project, revision, input_path).await? {
        println!("{}", line.text);
    }
    Ok(())
}

async fn build_terminal_diff(
    project: &Path,
    revision: &str,
    input_path: &str,
) -> Result<Vec<DiffLine>> {
    let target = resolve_diff_target(project, revision, input_path).await?;
    let output = Command::new("git")
        .arg("diff")
        .arg("--no-index")
        .arg("--no-color")
        .arg("--")
        .arg(&target.old_path)
        .arg(&target.new_path)
        .output()
        .context("Failed to run `git diff --no-index` for terminal diff")?;

    if !output.status.success() && output.status.code() != Some(1) {
        bail!(
            "Terminal diff failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = parse_terminal_diff(&stdout, &target);

    Ok(if lines.is_empty() {
        vec![DiffLine {
            text: "No textual differences".to_string(),
            kind: DiffLineKind::Meta,
            old_line: None,
            new_line: None,
        }]
    } else {
        lines
    })
}

fn parse_terminal_diff(stdout: &str, target: &DiffTarget) -> Vec<DiffLine> {
    let mut old_line = 0;
    let mut new_line = 0;
    let mut in_hunk = false;
    let mut lines = Vec::new();

    for raw_line in stdout.lines() {
        let kind = classify_diff_line(raw_line);
        let text = rewrite_diff_path_for_display(raw_line, target);
        let (old_number, new_number) = match kind {
            DiffLineKind::Hunk => {
                if let Some((old_start, new_start)) = parse_hunk_starts(raw_line) {
                    old_line = old_start;
                    new_line = new_start;
                    in_hunk = true;
                }
                (None, None)
            }
            DiffLineKind::Added if in_hunk => {
                let current = Some(new_line.max(1));
                new_line = new_line.saturating_add(1);
                (None, current)
            }
            DiffLineKind::Removed if in_hunk => {
                let current = Some(old_line.max(1));
                old_line = old_line.saturating_add(1);
                (current, None)
            }
            DiffLineKind::Context if in_hunk => {
                let current_old = Some(old_line.max(1));
                let current_new = Some(new_line.max(1));
                old_line = old_line.saturating_add(1);
                new_line = new_line.saturating_add(1);
                (current_old, current_new)
            }
            _ => (None, None),
        };

        lines.push(DiffLine {
            text,
            kind,
            old_line: old_number,
            new_line: new_number,
        });
    }

    lines
}

async fn resolve_diff_target(
    project: &Path,
    revision: &str,
    input_path: &str,
) -> Result<DiffTarget> {
    let pool = open_initialized_db(project).await?;
    let rel = normalize_project_path(project, input_path)?;
    let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT rf.old_snapshot_path, rf.new_snapshot_path \
         FROM revision_files rf \
         JOIN revisions r ON r.id = rf.revision_id \
         WHERE r.revision_label = ? AND rf.path = ?",
    )
    .bind(normalize_revision_label(revision))
    .bind(&rel)
    .fetch_optional(&pool)
    .await?
    .ok_or_else(|| anyhow!("No file `{rel}` found in revision `{revision}`"))?;

    Ok(DiffTarget {
        old_path: materialize_diff_side(project, row.0.as_deref(), "old")?,
        new_path: materialize_diff_side(project, row.1.as_deref(), "new")?,
        rel_path: rel,
        has_old: row.0.is_some(),
        has_new: row.1.is_some(),
    })
}

fn classify_diff_line(line: &str) -> DiffLineKind {
    if line.starts_with("diff --git")
        || line.starts_with("index ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
    {
        DiffLineKind::Header
    } else if line.starts_with("@@") {
        DiffLineKind::Hunk
    } else if line.starts_with('+') {
        DiffLineKind::Added
    } else if line.starts_with('-') {
        DiffLineKind::Removed
    } else if line.starts_with(' ') {
        DiffLineKind::Context
    } else {
        DiffLineKind::Meta
    }
}

fn rewrite_diff_path_for_display(line: &str, target: &DiffTarget) -> String {
    if line.starts_with("diff --git ") {
        format!("diff --git a/{} b/{}", target.rel_path, target.rel_path)
    } else if line.starts_with("--- ") {
        if target.has_old {
            format!("--- a/{}", target.rel_path)
        } else {
            "--- /dev/null".to_string()
        }
    } else if line.starts_with("+++ ") {
        if target.has_new {
            format!("+++ b/{}", target.rel_path)
        } else {
            "+++ /dev/null".to_string()
        }
    } else {
        line.to_string()
    }
}

async fn mark_reviewed(project: &Path, revision: &str) -> Result<()> {
    let pool = open_initialized_db(project).await?;
    mark_revision_reviewed(&pool, revision).await?;
    println!("Marked {} reviewed", normalize_revision_label(revision));
    Ok(())
}

async fn mark_revision_reviewed(pool: &SqlitePool, revision: &str) -> Result<()> {
    let result = sqlx::query("UPDATE revisions SET status = 'reviewed' WHERE revision_label = ?")
        .bind(normalize_revision_label(revision))
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        bail!("Revision `{revision}` not found");
    }
    Ok(())
}

fn ensure_store_dirs(project: &Path) -> Result<()> {
    fs::create_dir_all(project.join(STORE_DIR).join(SNAPSHOTS_DIR))?;
    fs::create_dir_all(project.join(STORE_DIR).join(RUNTIME_DIR))?;
    Ok(())
}

async fn open_initialized_db(project: &Path) -> Result<SqlitePool> {
    if !project.join(STORE_DIR).join(DB_FILE).exists() {
        bail!("Airev is not initialized. Run `airev init`.");
    }
    let pool = open_db(project).await?;
    migrate(&pool).await?;
    Ok(pool)
}

async fn open_db(project: &Path) -> Result<SqlitePool> {
    ensure_store_dirs(project)?;
    let db_path = project.join(STORE_DIR).join(DB_FILE);
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))?
        .create_if_missing(true);
    Ok(SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?)
}

async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_path TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            title TEXT NOT NULL,
            current_git_head TEXT
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS revisions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            revision_label TEXT UNIQUE,
            session_id INTEGER NOT NULL,
            parent_revision_id INTEGER,
            created_at TEXT NOT NULL,
            title TEXT NOT NULL,
            prompt TEXT NOT NULL,
            assistant_summary TEXT NOT NULL DEFAULT '',
            git_head TEXT,
            changed_file_count INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'unreviewed',
            FOREIGN KEY(session_id) REFERENCES sessions(id),
            FOREIGN KEY(parent_revision_id) REFERENCES revisions(id)
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS revision_files (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            revision_id INTEGER NOT NULL,
            path TEXT NOT NULL,
            change_type TEXT NOT NULL,
            old_snapshot_path TEXT,
            new_snapshot_path TEXT,
            additions INTEGER NOT NULL DEFAULT 0,
            deletions INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'unreviewed',
            FOREIGN KEY(revision_id) REFERENCES revisions(id)
        )",
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn ensure_session(pool: &SqlitePool, project: &Path) -> Result<i64> {
    let project_path = project.display().to_string();
    if let Some(id) = sqlx::query_scalar::<_, i64>("SELECT id FROM sessions WHERE project_path = ?")
        .bind(&project_path)
        .fetch_optional(pool)
        .await?
    {
        sqlx::query("UPDATE sessions SET updated_at = ?, current_git_head = ? WHERE id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(current_git_head(project))
            .bind(id)
            .execute(pool)
            .await?;
        return Ok(id);
    }

    let now = Utc::now().to_rfc3339();
    let title = project
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("project")
        .to_string();
    let result = sqlx::query(
        "INSERT INTO sessions (project_path, created_at, updated_at, title, current_git_head) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&project_path)
    .bind(&now)
    .bind(&now)
    .bind(title)
    .bind(current_git_head(project))
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

fn scan_manifest(project: &Path) -> Result<BTreeMap<String, String>> {
    let mut manifest = BTreeMap::new();
    for entry in WalkDir::new(project).into_iter().filter_entry(|entry| {
        if entry.depth() == 0 {
            return true;
        }
        let name = entry.file_name().to_string_lossy();
        !matches!(
            name.as_ref(),
            ".git" | ".gsd" | STORE_DIR | "target" | "node_modules" | ".bg-shell"
        )
    }) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(project)?.to_path_buf();
        let rel = path_to_forward_slashes(&rel);
        manifest.insert(rel, hash_file(entry.path())?);
    }
    Ok(manifest)
}

fn diff_manifest(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Vec<String> {
    let keys: BTreeSet<String> = before.keys().chain(after.keys()).cloned().collect();
    keys.into_iter()
        .filter(|path| before.get(path) != after.get(path))
        .collect()
}

fn snapshot_changed_files(
    project: &Path,
    revision_label: &str,
    changed_paths: &[String],
    turn: &ActiveTurn,
    current: &BTreeMap<String, String>,
) -> Result<Vec<ChangedFile>> {
    let mut changes = Vec::new();
    let snapshot_root = project
        .join(STORE_DIR)
        .join(SNAPSHOTS_DIR)
        .join(revision_label);

    for rel in changed_paths {
        let existed_before = turn.baseline.contains_key(rel);
        let exists_after = current.contains_key(rel);
        let change_type = match (existed_before, exists_after) {
            (false, true) => "added",
            (true, true) => "modified",
            (true, false) => "deleted",
            (false, false) => continue,
        }
        .to_string();

        let old_snapshot_path = if let Some(runtime_snapshot_rel) = turn.pre_snapshots.get(rel) {
            let source = project.join(STORE_DIR).join(runtime_snapshot_rel);
            if source.is_file() {
                let snapshot_rel = Path::new(SNAPSHOTS_DIR)
                    .join(revision_label)
                    .join("old")
                    .join(rel);
                let destination = project.join(STORE_DIR).join(&snapshot_rel);
                copy_file(&source, &destination)?;
                Some(path_to_forward_slashes(&snapshot_rel))
            } else {
                None
            }
        } else {
            None
        };

        let new_snapshot_path = if exists_after {
            let source = project.join(rel);
            let snapshot_rel = Path::new(SNAPSHOTS_DIR)
                .join(revision_label)
                .join("new")
                .join(rel);
            let destination = project.join(STORE_DIR).join(&snapshot_rel);
            copy_file(&source, &destination)?;
            Some(path_to_forward_slashes(&snapshot_rel))
        } else {
            None
        };

        fs::create_dir_all(&snapshot_root)?;
        changes.push(ChangedFile {
            path: rel.clone(),
            change_type,
            old_snapshot_path,
            new_snapshot_path,
        });
    }

    Ok(changes)
}

fn read_text_arg(value: Option<String>, file: Option<PathBuf>) -> Result<Option<String>> {
    match (value, file) {
        (Some(value), None) => Ok(Some(value)),
        (None, Some(file)) => match fs::read_to_string(&file) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("Failed to read {}", file.display())),
        },
        (None, None) => Ok(None),
        (Some(_), Some(_)) => bail!("Pass either inline text or a file, not both"),
    }
}

fn title_from_prompt(prompt: &str) -> String {
    let first_line = prompt
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("AI turn");
    let mut title = truncate_pretty(first_line.trim(), 80);
    if title.is_empty() {
        title = "AI turn".to_string();
    }
    title
}

fn title_from_revision(prompt: &str, changed_paths: &[String]) -> String {
    let prompt_lower = prompt.to_lowercase();
    let target = changed_paths
        .first()
        .map(|path| title_target_from_path(path))
        .unwrap_or_else(|| "code".to_string());

    let action = if prompt_lower.contains("usuń")
        || prompt_lower.contains("usun")
        || prompt_lower.contains("remove")
        || prompt_lower.contains("delete")
    {
        "Remove"
    } else if prompt_lower.contains("odtwórz")
        || prompt_lower.contains("odtworz")
        || prompt_lower.contains("recreate")
    {
        "Recreate"
    } else if prompt_lower.contains("zrób")
        || prompt_lower.contains("zrob")
        || prompt_lower.contains("create")
        || prompt_lower.contains("add")
    {
        "Create"
    } else if prompt_lower.contains("diff") {
        "Improve"
    } else if prompt_lower.contains("test") {
        "Test"
    } else {
        "Update"
    };

    let suffix = if changed_paths.len() > 1 {
        format!(
            " and {} more file{}",
            changed_paths.len() - 1,
            plural((changed_paths.len() - 1) as i64)
        )
    } else {
        String::new()
    };

    truncate_pretty(&format!("{action} {target}{suffix}"), 64)
}

fn title_target_from_path(path: &str) -> String {
    let file_name = Path::new(path)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or(path)
        .replace(['_', '-'], " ");
    sentence_case_title(&file_name)
}

fn active_turn_path(project: &Path) -> PathBuf {
    project
        .join(STORE_DIR)
        .join(RUNTIME_DIR)
        .join(ACTIVE_TURN_FILE)
}

fn runtime_pre_dir(project: &Path) -> PathBuf {
    project.join(STORE_DIR).join(RUNTIME_DIR).join("pre")
}

fn read_active_turn(project: &Path) -> Result<Option<ActiveTurn>> {
    let path = active_turn_path(project);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&fs::read_to_string(path)?)?))
}

fn write_active_turn(project: &Path, turn: &ActiveTurn) -> Result<()> {
    ensure_store_dirs(project)?;
    fs::write(
        active_turn_path(project),
        serde_json::to_string_pretty(turn)?,
    )?;
    Ok(())
}

fn remove_active_turn(project: &Path) -> Result<()> {
    let path = active_turn_path(project);
    if path.exists() {
        fs::remove_file(path)?;
    }
    let pre = runtime_pre_dir(project);
    if pre.exists() {
        fs::remove_dir_all(pre)?;
    }
    Ok(())
}

fn clear_runtime(project: &Path) -> Result<()> {
    let runtime = project.join(STORE_DIR).join(RUNTIME_DIR);
    if runtime.exists() {
        fs::remove_dir_all(&runtime)?;
    }
    fs::create_dir_all(runtime)?;
    Ok(())
}

fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination).with_context(|| {
        format!(
            "Failed to copy {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn normalize_project_path(project: &Path, input: &str) -> Result<String> {
    let input = input.trim_start_matches('@');
    let raw = Path::new(input);
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        project.join(raw)
    };
    let normalized = lexical_normalize(&joined);
    let project_normalized = lexical_normalize(project);
    let rel = normalized
        .strip_prefix(&project_normalized)
        .with_context(|| {
            format!(
                "Path `{}` is outside project `{}`",
                normalized.display(),
                project_normalized.display()
            )
        })?;
    Ok(path_to_forward_slashes(rel))
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn path_to_forward_slashes(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn current_git_head(project: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .current_dir(project)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn format_revision_label(id: i64) -> String {
    format!("r{id:06}")
}

fn normalize_revision_label(revision: &str) -> String {
    if let Some(number) = revision.strip_prefix('r') {
        format!("r{:06}", number.parse::<i64>().unwrap_or_default())
    } else {
        format!("r{:06}", revision.parse::<i64>().unwrap_or_default())
    }
}

fn materialize_diff_side(project: &Path, store_rel: Option<&str>, side: &str) -> Result<PathBuf> {
    if let Some(store_rel) = store_rel {
        return Ok(project.join(STORE_DIR).join(store_rel));
    }
    let empty = project
        .join(STORE_DIR)
        .join(RUNTIME_DIR)
        .join(format!("empty-{side}"));
    if !empty.exists() {
        if let Some(parent) = empty.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&empty, "")?;
    }
    Ok(empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_text_arg_returns_none_for_missing_file() {
        let missing =
            std::env::temp_dir().join(format!("airev-missing-prompt-{}", std::process::id()));

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
}
