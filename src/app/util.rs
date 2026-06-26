use super::*;

pub(crate) fn format_line_number(line: Option<u32>, width: usize) -> String {
    line.map(|value| format!("{value:>width$}"))
        .unwrap_or_else(|| " ".repeat(width))
}

pub(crate) fn hunk_label(raw: &str) -> String {
    if let Some((old_start, new_start)) = parse_hunk_starts(raw) {
        format!("Changed around old line {old_start}, new line {new_start}")
    } else {
        raw.to_string()
    }
}

pub(crate) fn parse_hunk_starts(raw: &str) -> Option<(u32, u32)> {
    let mut parts = raw.split_whitespace();
    parts.next()?;
    let old_part = parts.next()?;
    let new_part = parts.next()?;
    Some((
        parse_hunk_start(old_part, '-')?,
        parse_hunk_start(new_part, '+')?,
    ))
}

pub(crate) fn parse_hunk_start(part: &str, prefix: char) -> Option<u32> {
    let value = part.strip_prefix(prefix)?;
    let start = value.split(',').next()?;
    start.parse::<u32>().ok()
}

pub(crate) fn sentence_case_title(title: &str) -> String {
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
        .replace(" patchbay", " Patchbay")
}

pub(crate) fn short_timestamp(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.format("%H:%M").to_string())
        .unwrap_or_else(|_| value.chars().take(16).collect())
}

pub(crate) fn truncate_pretty(value: &str, max_chars: usize) -> String {
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

pub(crate) fn plural(count: i64) -> &'static str {
    if count == 1 { "" } else { "s" }
}

impl From<MissionAgentStatusArg> for MissionAgentStatus {
    fn from(value: MissionAgentStatusArg) -> Self {
        match value {
            MissionAgentStatusArg::Pending => MissionAgentStatus::Pending,
            MissionAgentStatusArg::Running => MissionAgentStatus::Running,
            MissionAgentStatusArg::Waiting => MissionAgentStatus::Waiting,
            MissionAgentStatusArg::Complete => MissionAgentStatus::Complete,
            MissionAgentStatusArg::Failed => MissionAgentStatus::Failed,
            MissionAgentStatusArg::Blocked => MissionAgentStatus::Blocked,
        }
    }
}

pub(crate) fn display_home_relative(path: &Path) -> String {
    if let Ok(home) = env::var("HOME") {
        let home = PathBuf::from(home);
        if let Ok(relative) = path.strip_prefix(&home) {
            return format!("~/{}", path_to_forward_slashes(&relative.to_path_buf()));
        }
    }
    path.display().to_string()
}

pub(crate) fn display_project_relative(start: &Path, path: &Path) -> String {
    let base = find_initialized_project_root(start).unwrap_or_else(|| start.to_path_buf());
    path.strip_prefix(&base)
        .map(|relative| path_to_forward_slashes(&relative.to_path_buf()))
        .unwrap_or_else(|_| path.display().to_string())
}

pub(crate) fn same_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

pub(crate) fn read_text_arg(
    value: Option<String>,
    file: Option<PathBuf>,
) -> Result<Option<String>> {
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

pub(crate) fn title_from_prompt(prompt: &str) -> String {
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

pub(crate) fn title_from_revision(prompt: &str, changed_paths: &[String]) -> String {
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

pub(crate) fn title_target_from_path(path: &str) -> String {
    let file_name = Path::new(path)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or(path)
        .replace(['_', '-'], " ");
    sentence_case_title(&file_name)
}

pub(crate) fn normalize_project_path(project: &Path, input: &str) -> Result<String> {
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

pub(crate) fn lexical_normalize(path: &Path) -> PathBuf {
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

pub(crate) fn path_to_forward_slashes(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn format_revision_label(id: i64) -> String {
    format!("r{id:06}")
}

pub(crate) fn normalize_revision_label(revision: &str) -> String {
    if let Some(number) = revision.strip_prefix('r') {
        format!("r{:06}", number.parse::<i64>().unwrap_or_default())
    } else {
        format!("r{:06}", revision.parse::<i64>().unwrap_or_default())
    }
}
