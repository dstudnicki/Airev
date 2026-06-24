use super::*;

pub(crate) fn diff_help(raw: bool) -> String {
    if raw {
        "Raw git diff · g: readable diff · n/p: files · Esc: files · q: quit".to_string()
    } else {
        "Readable diff · g: raw git diff · n/p: files · Esc: files · q: quit".to_string()
    }
}

pub(crate) fn split_diff_prefix(text: &str) -> (&str, &str) {
    match text.as_bytes().first().copied() {
        Some(b'+') => ("+", &text[1..]),
        Some(b'-') => ("−", &text[1..]),
        Some(b' ') => (" ", &text[1..]),
        _ => (" ", text),
    }
}

pub(crate) async fn open_diff(
    project: &Path,
    revision: &str,
    input_path: &str,
    editor: &str,
) -> Result<()> {
    let target = resolve_diff_target(project, revision, input_path).await?;

    let status = Command::new(editor)
        .arg("--diff")
        .arg(&target.old_path)
        .arg(&target.new_path)
        .status()
        .with_context(|| format!("Failed to launch diff editor `{editor}`"))?;

    if !status.success() {
        bail!("Diff editor `{editor}` exited with {status}");
    }
    Ok(())
}

pub(crate) async fn print_terminal_diff(
    project: &Path,
    revision: &str,
    input_path: &str,
) -> Result<()> {
    for line in build_terminal_diff(project, revision, input_path).await? {
        println!("{}", line.text);
    }
    Ok(())
}

pub(crate) async fn build_terminal_diff(
    project: &Path,
    revision: &str,
    input_path: &str,
) -> Result<Vec<DiffLine>> {
    let target = resolve_diff_target(project, revision, input_path).await?;
    let output = Command::new("git")
        .arg("diff")
        .arg("--no-index")
        .arg("--no-color")
        .arg("--")
        .arg(&target.old_path)
        .arg(&target.new_path)
        .output()
        .context("Failed to run `git diff --no-index` for terminal diff")?;

    if !output.status.success() && output.status.code() != Some(1) {
        bail!(
            "Terminal diff failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = parse_terminal_diff(&stdout, &target);

    Ok(if lines.is_empty() {
        vec![DiffLine {
            text: "No textual differences".to_string(),
            kind: DiffLineKind::Meta,
            old_line: None,
            new_line: None,
        }]
    } else {
        lines
    })
}

pub(crate) fn parse_terminal_diff(stdout: &str, target: &DiffTarget) -> Vec<DiffLine> {
    let mut old_line = 0;
    let mut new_line = 0;
    let mut in_hunk = false;
    let mut lines = Vec::new();

    for raw_line in stdout.lines() {
        let kind = classify_diff_line(raw_line);
        let text = rewrite_diff_path_for_display(raw_line, target);
        let (old_number, new_number) = match kind {
            DiffLineKind::Hunk => {
                if let Some((old_start, new_start)) = parse_hunk_starts(raw_line) {
                    old_line = old_start;
                    new_line = new_start;
                    in_hunk = true;
                }
                (None, None)
            }
            DiffLineKind::Added if in_hunk => {
                let current = Some(new_line.max(1));
                new_line = new_line.saturating_add(1);
                (None, current)
            }
            DiffLineKind::Removed if in_hunk => {
                let current = Some(old_line.max(1));
                old_line = old_line.saturating_add(1);
                (current, None)
            }
            DiffLineKind::Context if in_hunk => {
                let current_old = Some(old_line.max(1));
                let current_new = Some(new_line.max(1));
                old_line = old_line.saturating_add(1);
                new_line = new_line.saturating_add(1);
                (current_old, current_new)
            }
            _ => (None, None),
        };

        lines.push(DiffLine {
            text,
            kind,
            old_line: old_number,
            new_line: new_number,
        });
    }

    lines
}

pub(crate) async fn resolve_diff_target(
    project: &Path,
    revision: &str,
    input_path: &str,
) -> Result<DiffTarget> {
    let pool = open_initialized_db(project).await?;
    let rel = normalize_project_path(project, input_path)?;
    let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT rf.old_snapshot_path, rf.new_snapshot_path \
         FROM revision_files rf \
         JOIN revisions r ON r.id = rf.revision_id \
         WHERE r.revision_label = ? AND rf.path = ?",
    )
    .bind(normalize_revision_label(revision))
    .bind(&rel)
    .fetch_optional(&pool)
    .await?
    .ok_or_else(|| anyhow!("No file `{rel}` found in revision `{revision}`"))?;

    Ok(DiffTarget {
        old_path: materialize_diff_side(project, row.0.as_deref(), "old")?,
        new_path: materialize_diff_side(project, row.1.as_deref(), "new")?,
        rel_path: rel,
        has_old: row.0.is_some(),
        has_new: row.1.is_some(),
    })
}

pub(crate) fn classify_diff_line(line: &str) -> DiffLineKind {
    if line.starts_with("diff --git")
        || line.starts_with("index ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
    {
        DiffLineKind::Header
    } else if line.starts_with("@@") {
        DiffLineKind::Hunk
    } else if line.starts_with('+') {
        DiffLineKind::Added
    } else if line.starts_with('-') {
        DiffLineKind::Removed
    } else if line.starts_with(' ') {
        DiffLineKind::Context
    } else {
        DiffLineKind::Meta
    }
}

pub(crate) fn rewrite_diff_path_for_display(line: &str, target: &DiffTarget) -> String {
    if line.starts_with("diff --git ") {
        format!("diff --git a/{} b/{}", target.rel_path, target.rel_path)
    } else if line.starts_with("--- ") {
        if target.has_old {
            format!("--- a/{}", target.rel_path)
        } else {
            "--- /dev/null".to_string()
        }
    } else if line.starts_with("+++ ") {
        if target.has_new {
            format!("+++ b/{}", target.rel_path)
        } else {
            "+++ /dev/null".to_string()
        }
    } else {
        line.to_string()
    }
}

pub(crate) fn diff_manifest(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Vec<String> {
    let keys: BTreeSet<String> = before.keys().chain(after.keys()).cloned().collect();
    keys.into_iter()
        .filter(|path| before.get(path) != after.get(path))
        .collect()
}

pub(crate) fn materialize_diff_side(
    project: &Path,
    store_rel: Option<&str>,
    side: &str,
) -> Result<PathBuf> {
    if let Some(store_rel) = store_rel {
        return Ok(project.join(STORE_DIR).join(store_rel));
    }
    let empty = project
        .join(STORE_DIR)
        .join(RUNTIME_DIR)
        .join(format!("empty-{side}"));
    if !empty.exists() {
        if let Some(parent) = empty.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&empty, "")?;
    }
    Ok(empty)
}
