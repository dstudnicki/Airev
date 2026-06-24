use super::*;

impl UiConfig {
    pub(crate) fn from_config(config: Option<&UiConfigFile>) -> Self {
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

pub(crate) fn resolve_theme_config(
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

impl UiTheme {
    pub(crate) fn from_config(config: &ThemeConfigFile) -> Self {
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

pub(crate) fn built_in_theme_config(name: &str) -> Option<ThemeConfigFile> {
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

pub(crate) fn default_terminal_theme_config() -> ThemeConfigFile {
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

pub(crate) fn parse_color(value: Option<&str>) -> Option<Color> {
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

pub(crate) fn parse_hex_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(red, green, blue))
}
