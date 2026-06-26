use super::*;

pub(crate) fn scan_manifest(project: &Path) -> Result<BTreeMap<String, String>> {
    let mut manifest = BTreeMap::new();
    for entry in WalkDir::new(project).into_iter().filter_entry(|entry| {
        if entry.depth() == 0 {
            return true;
        }
        let name = entry.file_name().to_string_lossy();
        !matches!(
            name.as_ref(),
            ".git" | ".gsd" | STORE_DIR | "target" | "node_modules" | ".bg-shell"
        )
    }) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(project)?.to_path_buf();
        let rel = path_to_forward_slashes(&rel);
        manifest.insert(rel, hash_file(entry.path())?);
    }
    Ok(manifest)
}

pub(crate) fn snapshot_changed_files(
    project: &Path,
    revision_label: &str,
    changed_paths: &[String],
    turn: &ActiveTurn,
    current: &BTreeMap<String, String>,
) -> Result<Vec<ChangedFile>> {
    let mut changes = Vec::new();
    let snapshot_root = project
        .join(STORE_DIR)
        .join(SNAPSHOTS_DIR)
        .join(revision_label);

    for rel in changed_paths {
        let existed_before = turn.baseline.contains_key(rel);
        let exists_after = current.contains_key(rel);
        let change_type = match (existed_before, exists_after) {
            (false, true) => "added",
            (true, true) => "modified",
            (true, false) => "deleted",
            (false, false) => continue,
        }
        .to_string();

        let old_snapshot_path = if let Some(runtime_snapshot_rel) = turn.pre_snapshots.get(rel) {
            let source = project.join(STORE_DIR).join(runtime_snapshot_rel);
            if source.is_file() {
                let snapshot_rel = Path::new(SNAPSHOTS_DIR)
                    .join(revision_label)
                    .join("old")
                    .join(rel);
                let destination = project.join(STORE_DIR).join(&snapshot_rel);
                copy_file(&source, &destination)?;
                Some(path_to_forward_slashes(&snapshot_rel))
            } else {
                None
            }
        } else {
            None
        };

        let new_snapshot_path = if exists_after {
            let source = project.join(rel);
            let snapshot_rel = Path::new(SNAPSHOTS_DIR)
                .join(revision_label)
                .join("new")
                .join(rel);
            let destination = project.join(STORE_DIR).join(&snapshot_rel);
            copy_file(&source, &destination)?;
            Some(path_to_forward_slashes(&snapshot_rel))
        } else {
            None
        };

        fs::create_dir_all(&snapshot_root)?;
        changes.push(ChangedFile {
            path: rel.clone(),
            change_type,
            old_snapshot_path,
            new_snapshot_path,
        });
    }

    Ok(changes)
}

pub(crate) fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination).with_context(|| {
        format!(
            "Failed to copy {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

pub(crate) fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) fn current_git_head(project: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .current_dir(project)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
