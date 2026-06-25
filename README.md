# Patchbay

Patchbay is a local tiling control surface for coding-agent work. The short global `pb` command opens a Mission Control workspace; project-local `.ai-revisions/` storage records turns, snapshots, diffs, and review state.

Patchbay now has four connected pieces:

- **Chat composer**: describe an app and desired features; Patchbay turns that into mission agents.
- **Hierarchical agent tree**: root agents can create child agents, so the main workspace stays readable.
- **Tiled WM shell**: agents are shown as tiles with status, profile, child count, and live terminal output.
- **Loop runner**: live agents run in tmux windows controlled by Patchbay; batch mode still exists for non-interactive runs.

## Install

```bash
cargo install --path . --force
pb --help
```

## Configure projects

Patchbay reads global config from `~/.config/patchbay/config.toml` or `$XDG_CONFIG_HOME/patchbay/config.toml`.
The old `~/.config/airev/config.toml` path is still read as a compatibility fallback.

```toml
[projects.cashpilot]
path = "/path/to/CashPilot"
description = "Finance app"

[projects.patchbay]
path = "/path/to/Patchbay"
description = "Agent mission control"
```

## Open Mission Control

```bash
pb
```

Important keys:

- `c` opens the chat composer
- `f` toggles the fast profile for new GSD sessions
- `Enter` in composer creates a mission and immediately launches visible root-agent GSD terminals
- arrow keys move focus between tiles/lists
- `i` sends keyboard input to the focused GSD terminal; `Esc` returns to the WM
- `Enter` on an agent tile enters its child-agent workspace, if it has children
- `Esc` moves back up one child-agent workspace
- `r` refreshes mission state
- `s` manually starts a GSD terminal for the focused agent, useful for old pending missions
- `d` opens initialized local revision projects
- `q` quits

The workspace is hierarchical: the root view shows top-level agents; child agents are visible only after entering their parent agent workspace.

## Compose from the CLI

```bash
pb compose --text "cashpilot: onboarding, billing fixes, CSV export"
```

Run immediately with the fast profile:

```bash
pb compose --text "cashpilot: onboarding, billing fixes, CSV export" --run --fast
```

With explicit `project: feature one, feature two` syntax, this creates one root agent per listed feature and initializes `.ai-revisions/` in the target project when needed.

With natural multi-project text, Patchbay creates one root orchestrator agent and nests mentioned projects as child agents under it. Example: `Odpal projekt Airev i CashPilot...` becomes one Patchbay root tile with Airev and CashPilot child tiles inside that workspace.

## Mission commands

```bash
pb mission list
pb mission status
pb mission show <mission-id>
pb mission run <mission-id> [agent-id]
pb mission run <mission-id> --all --fast
pb mission add-agent <mission-id> --parent <agent-id> --project cashpilot --task "write tests" --fast
pb mission update-agent <mission-id> <agent-id> --status complete --summary "done"
```

Child agents are attached under their parent and appear inside that parent workspace, not as noisy root tiles.

## Runner profiles

When launched from the WM by composing a mission, or manually with `s`, Patchbay starts the focused agent in a tmux window with:

```txt
gsd [--model <model>] <agent-prompt>
```

When launched through batch commands (`pb mission run`), Patchbay still uses non-interactive print mode for now:

```txt
gsd --print [--model <model>] <agent-prompt>
```

Each mission gets a tmux session named `patchbay-<mission-id>`, and each agent gets its own tmux window. You can inspect it outside Patchbay with:

```bash
tmux attach -t patchbay-<mission-id>
```

Fast mode uses model profile `gpt-5.5-high` by default:

```bash
pb mission run <mission-id> --all --fast
```

Overrides:

```bash
PATCHBAY_FAST_MODEL="gpt-5.5-high" pb mission run <mission-id> --fast
PATCHBAY_MODEL="provider/model" pb mission run <mission-id>
```

For tests or custom integrations, replace the runner command entirely:

```bash
PATCHBAY_RUNNER_CMD='my-agent-cli --stdin' pb mission run <mission-id> --all
```

Patchbay passes these env vars into runner processes:

- `PATCHBAY_HOME`
- `PATCHBAY_MISSION_ID`
- `PATCHBAY_AGENT_ID`
- `PATCHBAY_PARENT_AGENT_ID` when nested
- `PATCHBAY_PROJECT`
- `PATCHBAY_PROFILE`

Agent prompts include instructions for creating child agents with `pb mission add-agent` and reporting completion with `pb mission update-agent`.

## Local revisions

Initialize a project manually when desired:

```bash
pb init
```

Inspect local revisions:

```bash
pb status
pb revisions
pb diff r000020 src/app.rs
pb reviewed r000020
```

Local revision storage remains project-local:

```txt
.ai-revisions/
  db.sqlite
  runtime/
  snapshots/
  config.toml
```

Global Mission Control storage defaults to:

```txt
~/.patchbay/
  .ai-revisions/runtime/missions/
```

Set `PATCHBAY_HOME` to override the global Mission Control root.

## GSD adapter

Install the adapter:

```bash
pb gsd install
```

Slash commands:

```txt
/pb-init
/pb-status
/pb-revisions
/pb-compose
/pb-mission-start
/pb-mission-list
/pb-mission-status
/pb-mission-show
/pb-mission-run
/pb-mission-add-agent
/pb-mission-update-agent
```

The adapter still captures local revision turns automatically around GSD agent sessions:

1. `pb turn begin --force --snapshot-baseline`
2. `pb touch <path>` for file tool calls
3. `pb turn end --summary ...`
