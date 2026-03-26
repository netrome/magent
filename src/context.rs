use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// Error resolving a referenced context file.
#[derive(Debug)]
pub enum ContextError {
    /// The referenced file does not exist.
    NotFound(String),
    /// The referenced path resolves outside the knowledge base root.
    OutsideRoot(String),
    /// Failed to read the referenced file.
    ReadError(String, std::io::Error),
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextError::NotFound(path) => {
                write!(f, "Referenced file not found: {path}")
            }
            ContextError::OutsideRoot(path) => {
                write!(f, "Referenced file is outside the knowledge base: {path}")
            }
            ContextError::ReadError(path, err) => {
                write!(f, "Failed to read referenced file {path}: {err}")
            }
        }
    }
}

/// Resolve context file references from directive options.
///
/// Parses the `context` option value (comma-separated paths), resolves each
/// relative to `root`, validates paths, and reads file contents. Self-references
/// (the current file) are silently skipped.
pub fn resolve_context_files(
    options: &HashMap<String, String>,
    root: &Path,
    current_file: &Path,
) -> Result<Vec<(String, String)>, ContextError> {
    let context_value = match options.get("context") {
        Some(v) => v,
        None => return Ok(Vec::new()),
    };

    let canonical_root = root
        .canonicalize()
        .map_err(|e| ContextError::ReadError(root.display().to_string(), e))?;
    let canonical_current = current_file
        .canonicalize()
        .map_err(|e| ContextError::ReadError(current_file.display().to_string(), e))?;

    let mut results = Vec::new();

    for raw_path in context_value.split(',') {
        let raw_path = raw_path.trim();
        if raw_path.is_empty() {
            continue;
        }

        let resolved = canonical_root.join(raw_path);
        let canonical = resolve_and_validate(&resolved, &canonical_root, raw_path)?;

        // Skip self-references
        if canonical == canonical_current {
            continue;
        }

        let content = std::fs::read_to_string(&canonical)
            .map_err(|e| ContextError::ReadError(raw_path.to_string(), e))?;

        results.push((raw_path.to_string(), content));
    }

    Ok(results)
}

/// Canonicalize a path and verify it stays within root.
fn resolve_and_validate(
    path: &Path,
    canonical_root: &Path,
    raw_path: &str,
) -> Result<PathBuf, ContextError> {
    let canonical = path
        .canonicalize()
        .map_err(|_| ContextError::NotFound(raw_path.to_string()))?;

    if !canonical.starts_with(canonical_root) {
        return Err(ContextError::OutsideRoot(raw_path.to_string()));
    }

    Ok(canonical)
}

/// Build the document string passed to the LLM.
///
/// When `context_files` is empty, returns `content` unchanged.
/// When context files are present, wraps everything in labeled sections.
pub fn build_context_string(
    content: &str,
    current_filename: &str,
    context_files: &[(String, String)],
) -> String {
    if context_files.is_empty() {
        return content.to_string();
    }

    let mut doc = String::new();
    doc.push_str(&format!("=== CURRENT DOCUMENT: {current_filename} ===\n"));
    doc.push_str(content);
    if !content.ends_with('\n') {
        doc.push('\n');
    }
    doc.push_str("=== END CURRENT DOCUMENT ===\n");

    for (name, file_content) in context_files {
        doc.push_str(&format!("\n=== REFERENCED: {name} ===\n"));
        doc.push_str(file_content);
        if !file_content.ends_with('\n') {
            doc.push('\n');
        }
        doc.push_str("=== END REFERENCED ===\n");
    }

    doc
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    // -- build_context_string tests --

    #[test]
    fn build_context_string__should_return_content_unchanged_when_no_references() {
        // Given
        let content = "# My doc\n\nSome content.\n";

        // When
        let result = build_context_string(content, "doc.md", &[]);

        // Then
        assert_eq!(result, content);
    }

    #[test]
    fn build_context_string__should_wrap_with_headers_when_references_present() {
        // Given
        let content = "# Main doc\n";
        let refs = vec![("rust.md".to_string(), "# Rust notes\n".to_string())];

        // When
        let result = build_context_string(content, "main.md", &refs);

        // Then
        assert!(result.contains("=== CURRENT DOCUMENT: main.md ==="));
        assert!(result.contains("# Main doc"));
        assert!(result.contains("=== END CURRENT DOCUMENT ==="));
        assert!(result.contains("=== REFERENCED: rust.md ==="));
        assert!(result.contains("# Rust notes"));
        assert!(result.contains("=== END REFERENCED ==="));
    }

    #[test]
    fn build_context_string__should_include_multiple_referenced_files() {
        // Given
        let content = "main\n";
        let refs = vec![
            ("a.md".to_string(), "content a\n".to_string()),
            ("b.md".to_string(), "content b\n".to_string()),
        ];

        // When
        let result = build_context_string(content, "main.md", &refs);

        // Then
        assert!(result.contains("=== REFERENCED: a.md ==="));
        assert!(result.contains("content a"));
        assert!(result.contains("=== REFERENCED: b.md ==="));
        assert!(result.contains("content b"));
    }

    #[test]
    fn build_context_string__should_add_trailing_newline_if_missing() {
        // Given — content without trailing newline
        let content = "no newline";
        let refs = vec![("ref.md".to_string(), "also no newline".to_string())];

        // When
        let result = build_context_string(content, "doc.md", &refs);

        // Then — should still be well-formed with headers on their own lines
        assert!(result.contains("no newline\n=== END CURRENT DOCUMENT ==="));
        assert!(result.contains("also no newline\n=== END REFERENCED ==="));
    }

    // -- resolve_context_files tests --

    #[test]
    fn resolve_context_files__should_return_empty_when_no_context_option() {
        // Given
        let options = HashMap::new();
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("current.md");
        std::fs::write(&current, "content").unwrap();

        // When
        let result = resolve_context_files(&options, dir.path(), &current).unwrap();

        // Then
        assert!(result.is_empty());
    }

    #[test]
    fn resolve_context_files__should_read_single_referenced_file() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("current.md");
        std::fs::write(&current, "current content").unwrap();
        std::fs::write(dir.path().join("ref.md"), "referenced content").unwrap();

        let mut options = HashMap::new();
        options.insert("context".to_string(), "ref.md".to_string());

        // When
        let result = resolve_context_files(&options, dir.path(), &current).unwrap();

        // Then
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "ref.md");
        assert_eq!(result[0].1, "referenced content");
    }

    #[test]
    fn resolve_context_files__should_read_multiple_referenced_files() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("current.md");
        std::fs::write(&current, "").unwrap();
        std::fs::write(dir.path().join("a.md"), "content a").unwrap();
        std::fs::write(dir.path().join("b.md"), "content b").unwrap();

        let mut options = HashMap::new();
        options.insert("context".to_string(), "a.md, b.md".to_string());

        // When
        let result = resolve_context_files(&options, dir.path(), &current).unwrap();

        // Then
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "a.md");
        assert_eq!(result[1].0, "b.md");
    }

    #[test]
    fn resolve_context_files__should_skip_self_reference() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("current.md");
        std::fs::write(&current, "self").unwrap();
        std::fs::write(dir.path().join("other.md"), "other").unwrap();

        let mut options = HashMap::new();
        options.insert("context".to_string(), "current.md, other.md".to_string());

        // When
        let result = resolve_context_files(&options, dir.path(), &current).unwrap();

        // Then — self-reference skipped, only other.md included
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "other.md");
    }

    #[test]
    fn resolve_context_files__should_error_on_missing_file() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("current.md");
        std::fs::write(&current, "").unwrap();

        let mut options = HashMap::new();
        options.insert("context".to_string(), "nonexistent.md".to_string());

        // When
        let result = resolve_context_files(&options, dir.path(), &current);

        // Then
        let err = result.unwrap_err();
        assert!(matches!(err, ContextError::NotFound(_)));
        assert!(err.to_string().contains("nonexistent.md"));
    }

    #[test]
    fn resolve_context_files__should_error_on_path_traversal_outside_root() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("current.md");
        std::fs::write(&current, "").unwrap();
        // Create a file outside the root to make the path resolvable
        let outside = dir.path().join("../outside.md");
        std::fs::write(&outside, "secret").unwrap();

        let mut options = HashMap::new();
        options.insert("context".to_string(), "../outside.md".to_string());

        // When
        let result = resolve_context_files(&options, dir.path(), &current);

        // Then
        let err = result.unwrap_err();
        assert!(matches!(err, ContextError::OutsideRoot(_)));
        assert!(err.to_string().contains("../outside.md"));

        // Cleanup
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn resolve_context_files__should_resolve_subdirectory_paths() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("current.md");
        std::fs::write(&current, "").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/notes.md"), "sub content").unwrap();

        let mut options = HashMap::new();
        options.insert("context".to_string(), "sub/notes.md".to_string());

        // When
        let result = resolve_context_files(&options, dir.path(), &current).unwrap();

        // Then
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "sub/notes.md");
        assert_eq!(result[0].1, "sub content");
    }
}
