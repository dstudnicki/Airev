# Patchbay keybindings

## Global Mission Control

| Key | Action |
|---|---|
| `q` | Quit Patchbay. |
| `r` | Refresh mission/resource state. |
| `Esc` | Leave the current mission/workspace and return toward the mission list; cancels non-empty agent chat input first. |
| Arrow keys | Move selection in the focused panel. |
| `Tab` / `Shift+Tab` | Cycle focus panels. |
| Paste | Pastes into raw terminal input, agent chat, or composer depending on current mode/focus. Newlines are collapsed to spaces. |

## Mission list

| Key | Action |
|---|---|
| Arrow keys | Select a mission. |
| `Enter` | Open the selected mission. |
| `c` | Compose a new mission prompt. |
| `Ctrl+M` | Cycle the model used for new GSD sessions. The list comes from GSD's configured model registry. |
| `t` | Open the theme selector. |
| `d` | Open initialized project/revision list. |
| `u` | Open the resource panel. |
| `D` / `Delete` | Delete the selected mission after hard cleanup. |
| `K` | Hard-clean the selected mission without relying only on tmux session cleanup. |
| `r` | Refresh mission/resource state. |
| `q` | Quit Patchbay. |

## Composer

| Key | Action |
|---|---|
| Type text | Edit the mission prompt or focused-agent follow-up prompt. |
| Paste | Paste text into the composer without submitting it. Newlines are collapsed to spaces. |
| `Enter` | Submit and launch the mission, or submit the focused-agent follow-up. |
| `Backspace` | Delete one character. |
| `Esc` | Cancel composer and return to mission list. |
| `Ctrl+M` | Cycle the model used for new GSD sessions. The list comes from GSD's configured model registry. |
| `Tab` / `Shift+Tab` | Cycle focus panels. |
| `q` | Quit Patchbay. |

## Agent workspace

| Key | Action |
|---|---|
| Arrow keys | Select the focused agent tile. |
| `Ctrl+←` | Dock the focused agent on the left; reflow all other agents on the right. |
| `Ctrl+→` | Dock the focused agent on the right; reflow all other agents on the left. |
| `Ctrl+↑` | Dock the focused agent at the top; reflow all other agents below. |
| `Ctrl+↓` | Dock the focused agent at the bottom; reflow all other agents above. |
| `Ctrl+0` | Reset to automatic tiling. |
| Type text | Type into the bottom chat input for the focused GSD agent. |
| Paste | Paste into the bottom chat input for the focused GSD agent. |
| `Enter` with chat text | Send bottom chat input to the focused GSD agent; `/model` is handled locally by Patchbay. |
| `Backspace` | Delete one character from bottom chat input. |
| `Esc` with chat text | Clear bottom chat input. |
| `s` | Start or restart the focused agent terminal. |
| `S` | Start all visible pending agent terminals. |
| `R` | Run all visible pending agents through the non-interactive agent loop. |
| `m` | Open composer for a follow-up prompt for the focused agent. |
| `i` | Enter raw terminal-input mode for the focused tmux pane. |
| `x` | Stop the focused agent terminal and mark it waiting. |
| `D` / `Delete` | Delete the focused agent and its descendants. |
| `Enter` with empty chat | Enter the selected agent's child workspace when it has children. |
| `Tab` / `Shift+Tab` | Cycle focus panels. |
| `Esc` | Return to parent workspace or mission list. |

### Agent chat slash commands

Type these into the bottom chat input and press `Enter`.

| Command | Action |
|---|---|
| `/stop` | Stop the focused agent terminal and mark it waiting. |
| `/restart` | Stop and start a fresh terminal for the focused agent. |
| `/delete` | Delete the focused agent subtree. |
| `/clear` | Clear chat input/status. |

## Raw terminal-input mode

Enter with `i` from an agent workspace. This sends keystrokes directly to the focused tmux pane.

| Key | Action |
|---|---|
| Type text | Send literal text to the focused tmux pane. |
| Paste | Send pasted text to the focused tmux pane. |
| `Enter` | Send Enter to the focused tmux pane. |
| `Backspace` | Send Backspace to the focused tmux pane. |
| Arrow keys | Send arrow keys to the focused tmux pane. |
| `Esc` | Exit raw terminal-input mode and return to Mission Control. |

## Agent diffs panel

| Key | Action |
|---|---|
| Arrow keys | Select a diff ref for the focused agent. |
| `Enter` | Open the selected agent diff, then return to Mission Control. |
| `Tab` / `Shift+Tab` | Cycle focus panels. |
| `Esc` | Return toward agent panels / mission list. |

## Project/revision list from Mission Control

| Key | Action |
|---|---|
| Arrow keys | Select an initialized revision project. |
| `Enter` | Open that project's revision browser; `q` returns to Mission Control. |
| `Tab` / `Shift+Tab` | Cycle focus panels. |
| `Esc` | Return to mission list. |

## Theme selector

| Key | Action |
|---|---|
| Arrow keys | Select a theme. |
| `Enter` | Apply the selected theme. |
| `Tab` / `Shift+Tab` | Cycle focus panels. |
| `Esc` | Return to mission list. |

## Resource panel

| Key | Action |
|---|---|
| Arrow keys | Select an active Patchbay tmux pane/resource. |
| `K` | Hard-clean the selected resource's whole mission: terminate pane process trees, kill tmux windows/session, and mark agents stopped. |
| `D` / `Delete` | Same as `K`: hard-clean the selected resource's whole mission. |
| `r` | Refresh mission/resource state. |
| `Tab` / `Shift+Tab` | Cycle focus panels. |
| `Esc` | Return to mission list. |

## Revision browser

Opened from Mission Control's project/revision list or by running revision commands directly.

| Key | Action |
|---|---|
| Arrow keys | Move selection in revisions/files, or scroll in a diff. |
| `PageUp` / `PageDown` | Scroll a diff by larger increments. |
| `Enter` on revisions | Open the selected revision's files. |
| `Enter` on files | Open the selected file diff. |
| `Esc` / `Backspace` | Go back from files/diff/theme picker. |
| `t` | Open theme picker from revisions view. |
| `r` on revisions | Mark selected revision reviewed. |
| `r` on files | Mark selected file reviewed. |
| `d` on files | Open selected file diff. |
| `g` in diff | Toggle readable diff / raw git diff. |
| `n` / `]` in diff | Move to next file diff. |
| `p` / `[` in diff | Move to previous file diff. |
| `q` | Quit revision browser; when opened from Mission Control, returns there. |

## Environment toggles

| Variable | Effect |
|---|---|
| `PATCHBAY_GSD_INTERACTIVE=1` | Launch agents as interactive `gsd` consoles instead of one-shot `gsd --print`. |
| `PATCHBAY_AUTO_STOP_IDLE_SECS=<seconds>` | Auto-stop non-focused Patchbay missions after this many seconds of tmux inactivity. Default: `1800`. |
| `PATCHBAY_AUTO_STOP_IDLE_SECS=0` / `off` / `false` / `disabled` | Disable auto-stop. |

## Notes

- Default agent execution is one-shot `gsd --print` so prompts execute immediately and pane output acts as a live work log.
- Resource hard cleanup terminates recorded tmux pane process trees first, then kills tmux windows/sessions.
- Non-focused Patchbay missions auto-stop after 30 minutes of tmux inactivity by default; the currently focused mission is skipped.
