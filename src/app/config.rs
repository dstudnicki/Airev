use super::*;

pub(crate) fn load_tui_settings(project: &Path) -> TuiSettings {
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

pub(crate) fn load_merged_config(project: &Path) -> AirevConfigFile {
    let mut config = AirevConfigFile::default();
    if let Some(global_config) = read_config_file(&global_config_path()) {
        config.merge(global_config);
    }
    if let Some(project_config) = read_config_file(&project_config_path(project)) {
        config.merge(project_config);
    }
    config
}

pub(crate) fn project_config_path(project: &Path) -> PathBuf {
    project.join(STORE_DIR).join("config.toml")
}

pub(crate) fn available_theme_names(project: &Path) -> Vec<String> {
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

pub(crate) fn handle_theme_command(project: &Path, command: ThemeCommand) -> Result<()> {
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

pub(crate) fn ensure_theme_exists(project: &Path, name: &str) -> Result<()> {
    if available_theme_names(project)
        .iter()
        .any(|theme| theme == name)
    {
        Ok(())
    } else {
        bail!("Unknown theme `{name}`. Run `airev theme list` to see available themes.")
    }
}

pub(crate) fn write_theme_selection(path: &Path, name: &str) -> Result<()> {
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

pub(crate) fn global_config_path() -> PathBuf {
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

pub(crate) fn read_config_file(path: &Path) -> Option<AirevConfigFile> {
    let text = fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

impl AirevConfigFile {
    pub(crate) fn merge(&mut self, next: AirevConfigFile) {
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
        if let Some(next_projects) = next.projects {
            self.projects
                .get_or_insert_with(BTreeMap::new)
                .extend(next_projects);
        }
    }
}

impl UiConfigFile {
    pub(crate) fn merge(&mut self, next: UiConfigFile) {
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
