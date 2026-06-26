mod diff_view;
mod mission_control;
mod revision;
mod widgets;

pub(crate) use mission_control::launch_mission_control_ui;
#[cfg(test)]
pub(crate) use mission_control::{
    handle_workspace_layout_key, move_mission_control_selection_for_key, normalize_pasted_prompt,
    parse_tmux_session_id, tmux_agent_logged_shell_command, tmux_agent_shell_command,
    tmux_session_name, tmux_window_name, visible_agent_indexes,
};
pub(crate) use revision::launch_tui;
