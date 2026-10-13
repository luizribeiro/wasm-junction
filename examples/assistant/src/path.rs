use std::path::{Component, Path, PathBuf};

pub(crate) fn resolve(base: &Path, root: &str, requested: &str) -> Result<String, String> {
    let relative = lexical(requested)?;
    let mounted = base.join(root.trim_start_matches('/'));
    let mounted = mounted
        .canonicalize()
        .map_err(|error| format!("cannot resolve preopen {root}: {error}"))?;
    let resolved = resolve_existing_ancestor(&mounted.join(relative))?;
    let relative = resolved
        .strip_prefix(&mounted)
        .map_err(|_| format!("path {requested} leaves preopen {root}"))?;
    Ok(relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn lexical(path: &str) -> Result<PathBuf, String> {
    let mut clean = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir if clean.pop() => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("path {path} leaves its preopen"));
            }
        }
    }
    Ok(clean)
}

fn resolve_existing_ancestor(path: &Path) -> Result<PathBuf, String> {
    let mut ancestor = path;
    let mut missing = Vec::new();
    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .ok_or_else(|| format!("cannot resolve {}", path.display()))?;
        missing.push(name.to_owned());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| format!("cannot resolve {}", path.display()))?;
    }
    let mut resolved = ancestor
        .canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", path.display()))?;
    for part in missing.into_iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::resolve;

    fn fixture() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("wasm-junction-path-{nonce}"));
        fs::create_dir_all(base.join("notes/private")).unwrap();
        fs::write(base.join("notes/private/diary.md"), "private").unwrap();
        base
    }

    #[test]
    fn normalizes_paths_within_the_preopen() {
        let base = fixture();
        assert_eq!(
            resolve(&base, "/notes", "./notes/../private//diary.md").unwrap(),
            "private/diary.md"
        );
        assert!(resolve(&base, "/notes", "../../outside.md").is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn resolves_symlinks_and_refuses_ones_outside_the_preopen() {
        use std::os::unix::fs::symlink;

        let base = fixture();
        symlink("private/diary.md", base.join("notes/diary-link.md")).unwrap();
        symlink("/etc/passwd", base.join("notes/outside-link")).unwrap();
        assert_eq!(
            resolve(&base, "/notes", "diary-link.md").unwrap(),
            "private/diary.md"
        );
        assert!(resolve(&base, "/notes", "outside-link").is_err());
        fs::remove_dir_all(base).unwrap();
    }
}
