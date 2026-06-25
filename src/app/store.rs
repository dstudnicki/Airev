use super::*;

pub(crate) async fn fetch_revisions(pool: &SqlitePool) -> Result<Vec<RevisionRow>> {
    let rows = sqlx::query_as::<_, (String, String, String, i64, String)>(
        "SELECT revision_label, title, status, changed_file_count, created_at \
         FROM revisions ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(label, title, status, changed_file_count, created_at)| RevisionRow {
                label,
                title,
                status,
                changed_file_count,
                created_at,
            },
        )
        .collect())
}

pub(crate) async fn fetch_revision_files(
    pool: &SqlitePool,
    revision: &str,
) -> Result<Vec<FileRow>> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT rf.path, rf.change_type, rf.status \
         FROM revision_files rf \
         JOIN revisions r ON r.id = rf.revision_id \
         WHERE r.revision_label = ? \
         ORDER BY rf.path",
    )
    .bind(normalize_revision_label(revision))
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(path, change_type, status)| FileRow {
            path,
            change_type,
            status,
        })
        .collect())
}

pub(crate) async fn mark_file_reviewed(
    pool: &SqlitePool,
    revision: &str,
    path: &str,
) -> Result<()> {
    let revision = normalize_revision_label(revision);
    let revision_id: i64 = sqlx::query_scalar("SELECT id FROM revisions WHERE revision_label = ?")
        .bind(&revision)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow!("Revision `{revision}` not found"))?;

    sqlx::query("UPDATE revision_files SET status = 'reviewed' WHERE revision_id = ? AND path = ?")
        .bind(revision_id)
        .bind(path)
        .execute(pool)
        .await?;

    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM revision_files WHERE revision_id = ? AND status != 'reviewed'",
    )
    .bind(revision_id)
    .fetch_one(pool)
    .await?;

    if remaining == 0 {
        sqlx::query("UPDATE revisions SET status = 'reviewed' WHERE id = ?")
            .bind(revision_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub(crate) fn find_initialized_project_root(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        if dir.join(STORE_DIR).join(DB_FILE).exists() {
            return Some(dir.to_path_buf());
        }
    }
    None
}

pub(crate) fn require_initialized_project_root(start: &Path) -> Result<PathBuf> {
    find_initialized_project_root(start).ok_or_else(|| {
        anyhow!(
            "Patchbay local revisions are not initialized for this directory tree. Run `pb init` in the project root first."
        )
    })
}

pub(crate) fn global_control_root() -> Result<PathBuf> {
    if let Ok(path) = env::var("PATCHBAY_HOME") {
        let root = PathBuf::from(path);
        fs::create_dir_all(&root)
            .with_context(|| format!("Failed to create {}", root.display()))?;
        return Ok(root);
    }

    if let Ok(home) = env::var("HOME") {
        let root = PathBuf::from(home).join(".patchbay");
        fs::create_dir_all(&root)
            .with_context(|| format!("Failed to create {}", root.display()))?;
        return Ok(root);
    }

    let root = PathBuf::from(".patchbay");
    fs::create_dir_all(&root).with_context(|| format!("Failed to create {}", root.display()))?;
    Ok(root)
}

pub(crate) fn ensure_global_control_store(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join(STORE_DIR).join(RUNTIME_DIR).join(MISSIONS_DIR))
        .with_context(|| format!("Failed to create Patchbay mission store under {}", root.display()))?;
    Ok(())
}

pub(crate) async fn init_project(project: &Path) -> Result<()> {
    ensure_store_dirs(project)?;
    ensure_gitignore_entry(project)?;
    let pool = open_db(project).await?;
    migrate(&pool).await?;
    ensure_session(&pool, project).await?;
    println!("Initialized {}", project.join(STORE_DIR).display());
    Ok(())
}

pub(crate) fn ensure_gitignore_entry(project: &Path) -> Result<()> {
    let gitignore_path = project.join(".gitignore");
    let entry = ".ai-revisions/";

    let existing = if gitignore_path.exists() {
        fs::read_to_string(&gitignore_path)
            .with_context(|| format!("Failed to read {}", gitignore_path.display()))?
    } else {
        String::new()
    };

    let already_ignored = existing.lines().map(str::trim).any(|line| {
        matches!(
            line,
            ".ai-revisions" | ".ai-revisions/" | "/.ai-revisions" | "/.ai-revisions/"
        )
    });

    if already_ignored {
        return Ok(());
    }

    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(entry);
    updated.push('\n');

    fs::write(&gitignore_path, updated)
        .with_context(|| format!("Failed to update {}", gitignore_path.display()))?;
    Ok(())
}

pub(crate) async fn begin_turn(
    project: &Path,
    prompt: Option<String>,
    prompt_file: Option<PathBuf>,
    title: Option<String>,
    force: bool,
    snapshot_baseline: bool,
) -> Result<()> {
    init_project(project).await?;

    let active_path = active_turn_path(project);
    if active_path.exists() && !force {
        bail!("An Airev turn is already active. Use --force to replace stale turn state.");
    }

    let prompt_text = read_text_arg(prompt, prompt_file)?.unwrap_or_default();
    let title = title.unwrap_or_else(|| title_from_prompt(&prompt_text));

    clear_runtime(project)?;
    fs::create_dir_all(runtime_pre_dir(project))?;

    let baseline = scan_manifest(project)?;
    let pre_snapshots = if snapshot_baseline {
        snapshot_baseline_files(project, baseline.keys())?
    } else {
        BTreeMap::new()
    };
    let turn = ActiveTurn {
        prompt: prompt_text,
        title,
        started_at: Utc::now().to_rfc3339(),
        baseline,
        touched: BTreeSet::new(),
        pre_snapshots,
    };

    write_active_turn(project, &turn)?;
    println!("Airev turn started: {}", turn.title);
    Ok(())
}

pub(crate) fn snapshot_baseline_files<'a>(
    project: &Path,
    paths: impl IntoIterator<Item = &'a String>,
) -> Result<BTreeMap<String, String>> {
    let mut snapshots = BTreeMap::new();
    for rel in paths {
        let source = project.join(rel);
        if !source.is_file() {
            continue;
        }
        let pre_rel = Path::new("pre").join(rel);
        let destination = project.join(STORE_DIR).join(RUNTIME_DIR).join(&pre_rel);
        copy_file(&source, &destination)?;
        snapshots.insert(
            rel.clone(),
            path_to_forward_slashes(&PathBuf::from(RUNTIME_DIR).join(pre_rel)),
        );
    }
    Ok(snapshots)
}

pub(crate) fn touch_path(project: &Path, input_path: &str) -> Result<()> {
    let mut turn = read_active_turn(project)?
        .ok_or_else(|| anyhow!("No active Airev turn. Run `airev turn begin` first."))?;
    let rel = normalize_project_path(project, input_path)?;
    turn.touched.insert(rel.clone());

    let source = project.join(&rel);
    if source.is_file() && !turn.pre_snapshots.contains_key(&rel) {
        let pre_rel = Path::new("pre").join(&rel);
        let destination = project.join(STORE_DIR).join(RUNTIME_DIR).join(&pre_rel);
        copy_file(&source, &destination)?;
        turn.pre_snapshots.insert(
            rel.clone(),
            path_to_forward_slashes(&PathBuf::from(RUNTIME_DIR).join(pre_rel)),
        );
    }

    write_active_turn(project, &turn)?;
    println!("Airev touched: {rel}");
    Ok(())
}

pub(crate) async fn end_turn(
    project: &Path,
    summary: Option<String>,
    summary_file: Option<PathBuf>,
    title: Option<String>,
) -> Result<()> {
    let mut turn = read_active_turn(project)?
        .ok_or_else(|| anyhow!("No active Airev turn. Run `airev turn begin` first."))?;
    let has_explicit_title = title.is_some();
    if let Some(title) = title {
        turn.title = title;
    }

    let pool = open_initialized_db(project).await?;
    let session_id = ensure_session(&pool, project).await?;
    let current = scan_manifest(project)?;
    let changed_paths = diff_manifest(&turn.baseline, &current);

    if changed_paths.is_empty() {
        remove_active_turn(project)?;
        println!("No changed files; no Airev revision recorded.");
        return Ok(());
    }

    if !has_explicit_title {
        turn.title = title_from_revision(&turn.prompt, &changed_paths);
    }

    let parent_revision_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM revisions WHERE session_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(&pool)
    .await?;

    let created_at = Utc::now().to_rfc3339();
    let assistant_summary = read_text_arg(summary, summary_file)?.unwrap_or_default();

    let mut tx = pool.begin().await?;
    let revision_result = sqlx::query(
        "INSERT INTO revisions \
         (session_id, parent_revision_id, created_at, title, prompt, assistant_summary, git_head, changed_file_count, status) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'unreviewed')",
    )
    .bind(session_id)
    .bind(parent_revision_id)
    .bind(&created_at)
    .bind(&turn.title)
    .bind(&turn.prompt)
    .bind(&assistant_summary)
    .bind(current_git_head(project))
    .bind(changed_paths.len() as i64)
    .execute(&mut *tx)
    .await?;

    let revision_id = revision_result.last_insert_rowid();
    let revision_label = format_revision_label(revision_id);
    sqlx::query("UPDATE revisions SET revision_label = ? WHERE id = ?")
        .bind(&revision_label)
        .bind(revision_id)
        .execute(&mut *tx)
        .await?;

    let changes =
        snapshot_changed_files(project, &revision_label, &changed_paths, &turn, &current)?;
    for change in &changes {
        sqlx::query(
            "INSERT INTO revision_files \
             (revision_id, path, change_type, old_snapshot_path, new_snapshot_path, additions, deletions, status) \
             VALUES (?, ?, ?, ?, ?, 0, 0, 'unreviewed')",
        )
        .bind(revision_id)
        .bind(&change.path)
        .bind(&change.change_type)
        .bind(&change.old_snapshot_path)
        .bind(&change.new_snapshot_path)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    remove_active_turn(project)?;

    println!(
        "Recorded {revision_label}: {} changed file(s) - {}",
        changes.len(),
        turn.title
    );
    Ok(())
}

pub(crate) async fn list_revisions(project: &Path) -> Result<()> {
    let pool = open_initialized_db(project).await?;
    let rows = sqlx::query_as::<_, (String, String, String, i64, String)>(
        "SELECT revision_label, title, status, changed_file_count, created_at \
         FROM revisions ORDER BY id DESC",
    )
    .fetch_all(&pool)
    .await?;

    if rows.is_empty() {
        println!("No Patchbay revisions recorded.");
        return Ok(());
    }

    for (label, title, status, count, created_at) in rows {
        println!("{label}  {status:<10}  {count:>3} file(s)  {created_at}  {title}");
    }
    Ok(())
}

pub(crate) async fn status(project: &Path) -> Result<()> {
    if !project.join(STORE_DIR).join(DB_FILE).exists() {
        println!("Patchbay local revisions are not initialized. Run `pb init`.");
        return Ok(());
    }

    let pool = open_initialized_db(project).await?;
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM revisions")
        .fetch_one(&pool)
        .await?;
    let unreviewed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM revisions WHERE status = 'unreviewed'")
            .fetch_one(&pool)
            .await?;
    let ignored: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM revisions WHERE status = 'ignored'")
            .fetch_one(&pool)
            .await?;

    println!("Patchbay revisions: {total} total, {unreviewed} unreviewed, {ignored} ignored");
    if unreviewed > 0 {
        println!("Run `pb revisions` to inspect them or press `d` in `pb` for project diffs.");
    }
    Ok(())
}

pub(crate) async fn mark_reviewed(project: &Path, revision: &str) -> Result<()> {
    let pool = open_initialized_db(project).await?;
    mark_revision_reviewed(&pool, revision).await?;
    println!("Marked {} reviewed", normalize_revision_label(revision));
    Ok(())
}

pub(crate) async fn mark_revision_reviewed(pool: &SqlitePool, revision: &str) -> Result<()> {
    let result = sqlx::query("UPDATE revisions SET status = 'reviewed' WHERE revision_label = ?")
        .bind(normalize_revision_label(revision))
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        bail!("Revision `{revision}` not found");
    }
    Ok(())
}

pub(crate) fn ensure_store_dirs(project: &Path) -> Result<()> {
    fs::create_dir_all(project.join(STORE_DIR).join(SNAPSHOTS_DIR))?;
    fs::create_dir_all(project.join(STORE_DIR).join(RUNTIME_DIR))?;
    fs::create_dir_all(missions_dir(project))?;
    Ok(())
}

pub(crate) async fn open_initialized_db(project: &Path) -> Result<SqlitePool> {
    if !project.join(STORE_DIR).join(DB_FILE).exists() {
        bail!("Patchbay local revisions are not initialized. Run `pb init`.");
    }
    let pool = open_db(project).await?;
    migrate(&pool).await?;
    Ok(pool)
}

pub(crate) async fn open_db(project: &Path) -> Result<SqlitePool> {
    ensure_store_dirs(project)?;
    let db_path = project.join(STORE_DIR).join(DB_FILE);
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))?
        .create_if_missing(true);
    Ok(SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?)
}

pub(crate) async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_path TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            title TEXT NOT NULL,
            current_git_head TEXT
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS revisions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            revision_label TEXT UNIQUE,
            session_id INTEGER NOT NULL,
            parent_revision_id INTEGER,
            created_at TEXT NOT NULL,
            title TEXT NOT NULL,
            prompt TEXT NOT NULL,
            assistant_summary TEXT NOT NULL DEFAULT '',
            git_head TEXT,
            changed_file_count INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'unreviewed',
            FOREIGN KEY(session_id) REFERENCES sessions(id),
            FOREIGN KEY(parent_revision_id) REFERENCES revisions(id)
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS revision_files (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            revision_id INTEGER NOT NULL,
            path TEXT NOT NULL,
            change_type TEXT NOT NULL,
            old_snapshot_path TEXT,
            new_snapshot_path TEXT,
            additions INTEGER NOT NULL DEFAULT 0,
            deletions INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'unreviewed',
            FOREIGN KEY(revision_id) REFERENCES revisions(id)
        )",
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub(crate) async fn ensure_session(pool: &SqlitePool, project: &Path) -> Result<i64> {
    let project_path = project.display().to_string();
    if let Some(id) = sqlx::query_scalar::<_, i64>("SELECT id FROM sessions WHERE project_path = ?")
        .bind(&project_path)
        .fetch_optional(pool)
        .await?
    {
        sqlx::query("UPDATE sessions SET updated_at = ?, current_git_head = ? WHERE id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(current_git_head(project))
            .bind(id)
            .execute(pool)
            .await?;
        return Ok(id);
    }

    let now = Utc::now().to_rfc3339();
    let title = project
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("project")
        .to_string();
    let result = sqlx::query(
        "INSERT INTO sessions (project_path, created_at, updated_at, title, current_git_head) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&project_path)
    .bind(&now)
    .bind(&now)
    .bind(title)
    .bind(current_git_head(project))
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

pub(crate) fn active_turn_path(project: &Path) -> PathBuf {
    project
        .join(STORE_DIR)
        .join(RUNTIME_DIR)
        .join(ACTIVE_TURN_FILE)
}

pub(crate) fn runtime_pre_dir(project: &Path) -> PathBuf {
    project.join(STORE_DIR).join(RUNTIME_DIR).join("pre")
}

pub(crate) fn read_active_turn(project: &Path) -> Result<Option<ActiveTurn>> {
    let path = active_turn_path(project);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&fs::read_to_string(path)?)?))
}

pub(crate) fn write_active_turn(project: &Path, turn: &ActiveTurn) -> Result<()> {
    ensure_store_dirs(project)?;
    fs::write(
        active_turn_path(project),
        serde_json::to_string_pretty(turn)?,
    )?;
    Ok(())
}

pub(crate) fn remove_active_turn(project: &Path) -> Result<()> {
    let path = active_turn_path(project);
    if path.exists() {
        fs::remove_file(path)?;
    }
    let pre = runtime_pre_dir(project);
    if pre.exists() {
        fs::remove_dir_all(pre)?;
    }
    Ok(())
}

pub(crate) fn clear_runtime(project: &Path) -> Result<()> {
    let runtime = project.join(STORE_DIR).join(RUNTIME_DIR);
    if runtime.exists() {
        fs::remove_dir_all(&runtime)?;
    }
    fs::create_dir_all(runtime)?;
    Ok(())
}
