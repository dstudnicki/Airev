# Airev

Airev is a local AI revision journal for coding-agent work. It records AI turns, tracks changed files, and gives you a terminal UI for reviewing what changed before you trust or keep it.

Airev is project-local by default. It stores runtime data in an ignored local directory and does not require a hosted service.

## Install from source

Prerequisites:

- Rust and Cargo
- GSD/pi, if you want the slash-command adapter

From the Airev checkout:

```bash
cargo install --path . --force
```

Verify the binary is available:

```bash
airev --help
```

## Install the GSD adapter

Install the Airev adapter into the global GSD/pi extension directory:

```bash
airev gsd install
```

Expected output:

```txt
Airev GSD adapter installed.
Restart or reload GSD once.
Then run /airev-init in a project.
```

Restart GSD or run its reload command once so the adapter is discovered.

Check adapter status:

```bash
airev gsd status
```

Example output:

```txt
Airev binary: ok
GSD adapter: installed
GSD adapter path: ~/.pi/agent/extensions/airev
Project initialized: yes
Active project DB: .ai-revisions/db.sqlite
```

## Initialize a project

In each project you want Airev to track, initialize storage once:

```bash
cd my-project
airev init
```

Or, after installing and reloading the GSD adapter, run this inside GSD:

```txt
/airev-init
```

Both paths initialize the same project-local Airev storage.

## Basic CLI usage

Show revision status:

```bash
airev status
```

List recorded revisions:

```bash
airev revisions
```

Open the terminal UI:

```bash
airev
```

Show a terminal diff for a recorded revision file:

```bash
airev diff REVISION PATH --terminal
```

Mark a revision as reviewed:

```bash
airev reviewed REVISION
```

## GSD slash commands

After the adapter is installed and GSD is reloaded, these commands are available inside GSD:

```txt
/airev-init
/airev-status
/airev-revisions
/airev-mission-start Airev: build X; CashPilot: fix Y
/airev-mission-list
/airev-mission-status [mission-id]
/airev-mission-show <mission-id>
/airev-mission-update-agent <mission-id> <agent-id> --status complete --summary "..."
```

The adapter also records GSD agent turns automatically for initialized projects.

## Mission Control

Airev can persist text-only multi-project missions. Voice input is intentionally out of scope; a speech-to-text layer can feed the same text commands later.

Configure named projects in `.ai-revisions/config.toml` or `~/.config/airev/config.toml`:

```toml
[projects.Airev]
path = "/home/me/Dev/Airev"
description = "Mission Control repo"

[projects.CashPilot]
path = "/home/me/Dev/CashPilot"
```

Create a mission from project-prefixed text:

```bash
airev mission start --title "Daily mission" --text "Airev: add dispatcher; CashPilot: inspect login diffs"
```

Or pass explicit project tasks:

```bash
airev mission start --task "Airev=Add dispatcher" --task "CashPilot=Inspect login diffs"
```

Inspect and update mission agents:

```bash
airev mission list
airev mission status <mission-id>
airev mission show <mission-id>
airev mission update-agent <mission-id> <agent-id> --status complete --summary "Verified" --revision 18 --diff "18:src/main.rs"
```

Mission JSON is stored under `.ai-revisions/runtime/missions/` so MAIN/LEAD agents, GSD commands, and the terminal UI can share the same state.

## Mission diff drilldown

Mission agents can point at existing Airev revision diffs. List available refs:

```bash
airev mission diffs <mission-id>
airev mission diffs <mission-id> <agent-id>
```

Open a stored diff through the existing Airev diff renderer:

```bash
airev mission open-diff <mission-id> <agent-id> 0 --terminal
airev mission open-diff <mission-id> <agent-id> 0 --editor code
```

The diff index is shown by `airev mission diffs`. If a ref is stale or not bound to a revision ID, Airev returns an explicit error instead of silently opening the wrong file.

## Mission Control window manager

Open the non-voice terminal dashboard for the latest mission or a specific mission:

```bash
airev mission wm
airev mission wm <mission-id>
```

The dashboard is a tiling-style view with MAIN, AGENTS, DETAIL, and DIFFS panels. It is intentionally terminal-native rather than a desktop window manager replacement.

Keys:

```txt
q              quit
Tab / Shift+Tab cycle focused panel
h / j / k / l  move focus like a tiling window manager
1 / 2 / 3 / 4  focus MAIN / AGENTS / DETAIL / DIFFS
Up / Down      select agent or diff in the focused list
r              reload mission JSON from disk
Enter          open the selected DIFFS item with the terminal diff renderer
```

The UI shows agent status, project path, task, summary, last error, revision IDs, and diff refs. If a selected diff is stale or unbound, the footer reports the error instead of opening the wrong file.

## Local data and git hygiene

Airev writes project-local runtime data that should stay out of git. The default ignore rules cover:

```txt
.ai-revisions/
.gsd/
.bg-shell/
.working-docs/
target/
node_modules/
```

Do not put the GSD adapter source under local runtime directories. The public adapter source ships with this repository and is installed explicitly with `airev gsd install`.

## Development checks

Useful checks before sharing changes:

```bash
cargo fmt
cargo check
cargo test
npm install --prefix adapters/gsd
npm run --prefix adapters/gsd typecheck
```
