# Patchbay

Patchbay is a local tiling control surface for coding-agent work. The short global `pb` command opens a Mission Control workspace; project-local `.ai-revisions/` storage records turns, snapshots, diffs, and review state.

Patchbay now has four connected pieces:

- **Chat composer**: describe one or more apps and desired outcomes; Patchbay creates a root planner agent that decomposes the prompt and adds child agents through Patchbay when useful.
- **Hierarchical agent tree**: root agents can create additional child agents, and child agents can create their own children.
- **Tiled WM shell**: agents are shown as tmux-backed tiles with status, profile, child count, and live terminal output.
- **Loop runner**: live agents run `gsd --print` in tmux windows controlled by Patchbay; pending child agents are auto-started, visible in Resource panel, and auto-stopped when idle.
- **Skill presets**: each agent gets a prompt preset and recommended GSD skills based on the mission text, such as planning, frontend, debug, review, security, docs, or test work.

## Install

```bash
cargo install --path . --force
pb --help
```

## Configure projects

Patchbay reads global config from `~/.config/patchbay/config.toml` or `$XDG_CONFIG_HOME/patchbay/config.toml`.

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

Mission Control keeps the screen clean and does not print the full shortcut list in the footer. Learn these keys:

- `c` opens the chat composer.
- `Ctrl+M` cycles the model used for new GSD sessions; `default` leaves model choice to GSD.
- `Enter` in composer creates a mission, opens the child-agent workspace when present, and starts pending child agents.
- Agent tiles show a live GSD feed from the agent's tmux pane, including a working spinner while the pane is running.
- In an agent workspace, normal typing goes to the bottom chat input for the focused GSD agent; `Enter` sends it.
- Bottom chat slash commands: `/stop`, `/restart`, `/delete`, `/clear`.
- `Tab` / `Shift+Tab` moves focus between panels.
- Arrow keys move within the focused panel.
- `t` focuses the theme picker on the main screen; `Enter` applies the selected theme.
- `u` opens the resource panel with active Patchbay tmux panes, PIDs, commands, status, and idle time.
- `K` on a selected mission/resource hard-cleans the mission: process tree cleanup plus tmux session/window cleanup.
- Idle non-focused missions auto-stop after 30 minutes by default; set `PATCHBAY_AUTO_STOP_IDLE_SECS=0` to disable or another second value to tune it.
- `i` sends raw keyboard input to the focused tmux pane when needed; `Esc` returns to the WM.
- `Enter` on an agent tile enters its child-agent workspace, if it has children.
- `Esc` moves back up one child-agent workspace or returns to the mission list.
- `r` refreshes mission state.
- `s` manually starts/restarts a GSD terminal for the focused agent.
- `x` stops the focused agent terminal.
- `D` / `Delete` deletes the focused agent, or deletes the selected mission on the mission list.
- `d` opens initialized local revision projects.
- `q` quits.

The workspace is hierarchical: the root view shows top-level agents; child agents are visible only after entering their parent agent workspace.

## Compose from the CLI

```bash
pb compose --text "Fix the login bug, review the billing flow, and add tests"
```

Run immediately with an explicit model/profile:

```bash
pb compose --text "Fix the login bug, review the billing flow, and add tests" --run --profile gpt-5.1
```

`pb compose` creates one root planner/container agent and initializes `.ai-revisions/` in the target projects when needed. When the prompt names multiple registered projects, Patchbay immediately creates pending child agents for those projects under the root container, so execution does not depend on the planner first calling `pb mission add-agent`. Patchbay watches mission state and automatically starts pending child agents in tmux windows. By default Patchbay launches one-shot `gsd --print` agents so prompts execute immediately and pane output becomes a live work log; set `PATCHBAY_GSD_INTERACTIVE=1` only when you want an interactive GSD console instead.

Each agent prompt includes a routing preset and recommended GSD skills. For example, review tasks recommend `review`, debug tasks recommend `debug-like-expert`, planning tasks recommend `decompose-into-slices`, and frontend tasks recommend `frontend-design` plus accessibility polish.

## Mission commands

```bash
pb mission list
pb mission status
pb mission show <mission-id>
pb mission run <mission-id> [agent-id]
pb mission run <mission-id> --all --profile gpt-5.1
pb mission add-agent <mission-id> --parent <agent-id> --project my-app --task "write tests" --profile gpt-5.1
pb mission update-agent <mission-id> <agent-id> --status complete --summary "done"
```

Child agents are attached under their parent and appear inside that parent workspace, not as noisy root tiles. Any pending child agent added by a running GSD session is auto-started by Mission Control.

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

Model selection is explicit. Omit `--profile` to use GSD's default model, or pass a model/profile name through to `gsd --model`:

```bash
pb mission run <mission-id> --all --profile openai-codex/gpt-5.5
PATCHBAY_MODEL="provider/model" pb mission run <mission-id>
```

The TUI model picker queries GSD's configured model registry, matching the models shown by GSD's `/model` selector. Type `/model` in Mission Control chat to refresh/show the list, `/model provider/model-id` to select directly, or use `Ctrl+M` to cycle. `PATCHBAY_MODELS` is only a fallback if the GSD registry query fails.

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
