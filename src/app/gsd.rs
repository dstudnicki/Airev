use super::*;

pub(crate) fn install_gsd_adapter(start: &Path) -> Result<()> {
    let source = find_gsd_adapter_source_dir(start)?;
    let destination = gsd_adapter_install_dir()?;

    if same_path(&source, &destination) {
        bail!(
            "Adapter source and destination are both {}. Refusing to overwrite the source.",
            source.display()
        );
    }

    if destination.exists() {
        fs::remove_dir_all(&destination)
            .with_context(|| format!("Failed to remove existing {}", destination.display()))?;
    }
    copy_dir_recursive(&source, &destination)?;

    if adapter_needs_dependency_install(&destination)? {
        run_adapter_dependency_install(&destination)?;
    }

    println!("Patchbay GSD adapter installed.");
    println!("Restart or reload GSD once.");
    println!("Then run /pb-init in a project.");
    Ok(())
}

pub(crate) fn gsd_status(start: &Path) -> Result<()> {
    let adapter_path = gsd_adapter_install_dir()?;
    let adapter_installed = adapter_path.join("extension-manifest.json").exists()
        && (adapter_path.join("index.ts").exists() || adapter_path.join("index.js").exists());
    let project = find_initialized_project_root(start);
    let project_db = project
        .as_ref()
        .map(|root| root.join(STORE_DIR).join(DB_FILE));

    println!("Patchbay binary: ok");
    println!(
        "GSD adapter: {}",
        if adapter_installed {
            "installed"
        } else {
            "missing"
        }
    );
    println!("GSD adapter path: {}", display_home_relative(&adapter_path));
    println!(
        "Project initialized: {}",
        if project_db.as_ref().is_some_and(|path| path.exists()) {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "Active project DB: {}",
        project_db
            .as_ref()
            .map(|path| display_project_relative(start, path))
            .unwrap_or_else(|| STORE_DIR.to_string() + "/" + DB_FILE)
    );
    Ok(())
}

pub(crate) fn find_gsd_adapter_source_dir(start: &Path) -> Result<PathBuf> {
    if let Ok(value) = env::var("PATCHBAY_GSD_ADAPTER_SOURCE") {
        let candidate = PathBuf::from(value);
        validate_gsd_adapter_source(&candidate)?;
        return Ok(candidate);
    }

    for dir in start.ancestors() {
        let candidate = dir.join(GSD_ADAPTER_SOURCE_DIR);
        if validate_gsd_adapter_source(&candidate).is_ok() {
            return Ok(candidate);
        }
    }

    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
        let candidate = Path::new(manifest_dir).join(GSD_ADAPTER_SOURCE_DIR);
        if validate_gsd_adapter_source(&candidate).is_ok() {
            return Ok(candidate);
        }
    }

    bail!(
        "Could not find {GSD_ADAPTER_SOURCE_DIR}. Run `pb gsd install` from the Patchbay source checkout or set PATCHBAY_GSD_ADAPTER_SOURCE."
    )
}

pub(crate) fn validate_gsd_adapter_source(path: &Path) -> Result<()> {
    if path.join("extension-manifest.json").is_file() && path.join("index.ts").is_file() {
        Ok(())
    } else {
        bail!(
            "{} is not a Patchbay GSD adapter source directory",
            path.display()
        )
    }
}

pub(crate) fn gsd_adapter_install_dir() -> Result<PathBuf> {
    Ok(gsd_extensions_dir()?.join(GSD_ADAPTER_INSTALL_DIR_NAME))
}

pub(crate) fn gsd_extensions_dir() -> Result<PathBuf> {
    if let Ok(value) = env::var("PATCHBAY_GSD_EXTENSIONS_DIR") {
        return Ok(PathBuf::from(value));
    }

    let home = env::var("HOME").context("HOME is not set; cannot locate ~/.pi/agent/extensions")?;
    Ok(PathBuf::from(home)
        .join(".pi")
        .join("agent")
        .join("extensions"))
}

pub(crate) fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)
        .with_context(|| format!("Failed to create {}", destination.display()))?;

    for entry in WalkDir::new(source) {
        let entry = entry.with_context(|| format!("Failed to read {}", source.display()))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(source)
            .with_context(|| format!("Failed to relativize {}", path.display()))?;
        if relative.as_os_str().is_empty() || should_skip_adapter_copy_path(relative) {
            continue;
        }

        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("Failed to create {}", target.display()))?;
        } else if entry.file_type().is_file() {
            copy_file(path, &target)?;
        }
    }

    Ok(())
}

pub(crate) fn should_skip_adapter_copy_path(relative: &Path) -> bool {
    relative.components().any(|component| match component {
        Component::Normal(name) => matches!(
            name.to_str(),
            Some("node_modules" | ".git" | ".DS_Store" | "dist")
        ),
        _ => false,
    })
}

pub(crate) fn adapter_needs_dependency_install(adapter_dir: &Path) -> Result<bool> {
    let package_json_path = adapter_dir.join("package.json");
    if !package_json_path.exists() {
        return Ok(false);
    }

    let text = fs::read_to_string(&package_json_path)
        .with_context(|| format!("Failed to read {}", package_json_path.display()))?;
    let package: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("Failed to parse {}", package_json_path.display()))?;

    Ok(json_object_has_entries(package.get("dependencies")))
}

pub(crate) fn json_object_has_entries(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(|value| value.as_object())
        .is_some_and(|object| !object.is_empty())
}

pub(crate) fn run_adapter_dependency_install(adapter_dir: &Path) -> Result<()> {
    let status = Command::new("npm")
        .arg("install")
        .arg("--omit=dev")
        .current_dir(adapter_dir)
        .status()
        .context("Failed to run `npm install --omit=dev` for the GSD adapter")?;

    if !status.success() {
        bail!("GSD adapter dependency install failed with {status}");
    }
    Ok(())
}
