use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use clap::Parser;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};
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
const MISSIONS_DIR: &str = "missions";
const BUILT_IN_THEME_NAMES: &[&str] = &[
    "terminal",
    "patchbay-dark",
    "catppuccin-mocha",
    "gruvbox-dark",
    "high-contrast",
];
const GSD_ADAPTER_SOURCE_DIR: &str = "adapters/gsd";
const GSD_ADAPTER_INSTALL_DIR_NAME: &str = "patchbay";

mod cli;
mod config;
mod diff;
mod gsd;
mod mission;
mod models;
mod store;
mod theme;
mod tui;
mod util;
mod vcs;
pub(crate) use cli::*;
pub(crate) use config::*;
pub(crate) use diff::*;
pub(crate) use gsd::*;
pub(crate) use mission::*;
pub(crate) use models::*;
pub(crate) use store::*;
pub(crate) use theme::*;
pub(crate) use tui::*;
pub(crate) use util::*;
pub(crate) use vcs::*;

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir().context("Failed to read current directory")?;

    match cli.command {
        None => {
            let control_root = global_control_root()?;
            ensure_global_control_store(&control_root)?;
            let local_project = find_initialized_project_root(&cwd);
            launch_mission_control_ui(&control_root, None, local_project.as_deref()).await
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
            Commands::Gsd { command } => match command {
                GsdCommand::Install => install_gsd_adapter(&cwd),
                GsdCommand::Status => gsd_status(&cwd),
            },
            Commands::Compose {
                text,
                text_file,
                run,
                profile,
            } => {
                let control_root = global_control_root()?;
                ensure_global_control_store(&control_root)?;
                compose_mission_command(&control_root, text, text_file, run, profile).await
            }
            Commands::Mission { command } => {
                let control_root = global_control_root()?;
                ensure_global_control_store(&control_root)?;
                let local_project = find_initialized_project_root(&cwd);
                handle_mission_command(&control_root, local_project.as_deref(), command).await
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

#[cfg(test)]
mod tests;
