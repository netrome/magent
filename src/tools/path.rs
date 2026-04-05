use std::path::{Path, PathBuf};

/// Resolve a relative path against the knowledge base root, with boundary checking.
///
/// Joins `relative` to `root` and verifies the result doesn't escape the root
/// directory (via `../` or symlinks). The path must exist on disk for canonicalization.
///
/// Returns the non-canonicalized joined path. Callers that need pre-checks
/// (e.g., exists, is_dir) should do them before calling this function with
/// their own error messages — canonicalize will also fail on non-existent paths.
pub fn resolve_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = root.join(relative);

    let canonical = path
        .canonicalize()
        .map_err(|_| format!("Error: cannot resolve path '{relative}'"))?;
    let root_canonical = root
        .canonicalize()
        .map_err(|_| "Error: cannot resolve knowledge base root".to_string())?;

    if !canonical.starts_with(&root_canonical) {
        return Err(format!(
            "Error: path '{relative}' is outside the knowledge base"
        ));
    }

    Ok(path)
}

/// Resolve a relative path that may not exist yet (e.g. for file creation).
///
/// Canonicalizes the deepest existing ancestor to detect symlink escapes,
/// then checks the remaining non-existent components for `..` traversal.
pub fn resolve_new_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.is_empty() {
        return Err("Error: empty path".to_string());
    }

    let path = root.join(relative);

    // Walk up to find the deepest existing ancestor
    let mut ancestor = path.as_path();
    while !ancestor.exists() {
        ancestor = ancestor.parent().unwrap_or(root);
    }

    let canonical = ancestor
        .canonicalize()
        .map_err(|_| format!("Error: cannot resolve path '{relative}'"))?;
    let root_canonical = root
        .canonicalize()
        .map_err(|_| "Error: cannot resolve knowledge base root".to_string())?;

    if !canonical.starts_with(&root_canonical) {
        return Err(format!(
            "Error: path '{relative}' is outside the knowledge base"
        ));
    }

    // The non-existent suffix could still contain ".." that escapes
    if let Ok(suffix) = path.strip_prefix(ancestor) {
        for comp in suffix.components() {
            if matches!(comp, std::path::Component::ParentDir) {
                return Err(format!(
                    "Error: path '{relative}' is outside the knowledge base"
                ));
            }
        }
    }

    Ok(path)
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    fn create_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn resolve_path__should_resolve_file_in_root() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "content");

        let result = resolve_path(dir.path(), "test.md").unwrap();
        assert_eq!(result, dir.path().join("test.md"));
    }

    #[test]
    fn resolve_path__should_resolve_file_in_subdirectory() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "notes/rust.md", "content");

        let result = resolve_path(dir.path(), "notes/rust.md").unwrap();
        assert_eq!(result, dir.path().join("notes/rust.md"));
    }

    #[test]
    fn resolve_path__should_resolve_directory() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("notes")).unwrap();

        let result = resolve_path(dir.path(), "notes").unwrap();
        assert_eq!(result, dir.path().join("notes"));
    }

    #[test]
    fn resolve_path__should_reject_traversal() {
        let dir = tempdir().unwrap();

        let result = resolve_path(dir.path(), "../../etc/passwd");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("cannot resolve") || err.contains("outside the knowledge base"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn resolve_path__should_reject_symlink_escape() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        create_file(outside.path(), "secret.md", "secret");
        symlink(
            outside.path().join("secret.md"),
            dir.path().join("escape.md"),
        )
        .unwrap();

        let result = resolve_path(dir.path(), "escape.md");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("outside the knowledge base"));
    }

    #[test]
    fn resolve_path__should_fail_for_nonexistent_path() {
        let dir = tempdir().unwrap();

        let result = resolve_path(dir.path(), "nonexistent.md");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot resolve"));
    }

    #[test]
    fn resolve_path__should_allow_dotdot_that_stays_within_root() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "content");

        // notes/../test.md resolves to test.md, which is within root
        fs::create_dir_all(dir.path().join("notes")).unwrap();
        let result = resolve_path(dir.path(), "notes/../test.md").unwrap();
        assert_eq!(result, dir.path().join("notes/../test.md"));
    }

    // --- resolve_new_path ---

    #[test]
    fn resolve_new_path__should_accept_nonexistent_file() {
        let dir = tempdir().unwrap();

        let result = resolve_new_path(dir.path(), "new-file.md").unwrap();
        assert_eq!(result, dir.path().join("new-file.md"));
    }

    #[test]
    fn resolve_new_path__should_accept_nonexistent_nested_path() {
        let dir = tempdir().unwrap();

        let result = resolve_new_path(dir.path(), "a/b/c/new.md").unwrap();
        assert_eq!(result, dir.path().join("a/b/c/new.md"));
    }

    #[test]
    fn resolve_new_path__should_accept_existing_file() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "existing.md", "content");

        let result = resolve_new_path(dir.path(), "existing.md").unwrap();
        assert_eq!(result, dir.path().join("existing.md"));
    }

    #[test]
    fn resolve_new_path__should_reject_traversal() {
        let dir = tempdir().unwrap();

        let result = resolve_new_path(dir.path(), "../../etc/evil.md");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("outside the knowledge base"));
    }

    #[test]
    fn resolve_new_path__should_reject_symlink_escape() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        // Create a symlink dir inside root that points outside
        symlink(outside.path(), dir.path().join("escape")).unwrap();

        let result = resolve_new_path(dir.path(), "escape/new.md");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("outside the knowledge base"));
    }

    #[test]
    fn resolve_new_path__should_reject_empty_path() {
        let dir = tempdir().unwrap();

        let result = resolve_new_path(dir.path(), "");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty path"));
    }
}
