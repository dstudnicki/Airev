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
```

The adapter also records GSD agent turns automatically for initialized projects.

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
