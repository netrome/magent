use std::fs;
use std::path::PathBuf;

/// Deletes files from the knowledge base.
pub struct DeleteTool {
    root: PathBuf,
}

impl DeleteTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Delete a file. Input: a relative file path.
    ///
    /// Never fails — errors are returned as the result string so the LLM
    /// can see what went wrong and adjust.
    pub fn execute(&self, input: &str) -> String {
        let path_str = input.trim();

        if path_str.is_empty() {
            return "Error: no file path provided".to_string();
        }

        let file_path = match super::path::resolve_path(&self.root, path_str) {
            Ok(p) => p,
            Err(_) => return format!("Error: file '{path_str}' not found"),
        };

        if !file_path.is_file() {
            return format!("Error: '{path_str}' is not a file");
        }

        match fs::remove_file(&file_path) {
            Ok(()) => format!("Deleted {path_str}"),
            Err(e) => format!("Error: cannot delete '{path_str}': {e}"),
        }
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn create_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn execute__should_delete_file() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "content");
        let tool = DeleteTool::new(dir.path().to_path_buf());

        let result = tool.execute("test.md");

        assert_eq!(result, "Deleted test.md");
        assert!(!dir.path().join("test.md").exists());
    }

    #[test]
    fn execute__should_delete_file_in_subdirectory() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "notes/old.md", "content");
        let tool = DeleteTool::new(dir.path().to_path_buf());

        let result = tool.execute("notes/old.md");

        assert_eq!(result, "Deleted notes/old.md");
        assert!(!dir.path().join("notes/old.md").exists());
    }

    #[test]
    fn execute__should_fail_for_nonexistent_file() {
        let dir = tempdir().unwrap();
        let tool = DeleteTool::new(dir.path().to_path_buf());

        let result = tool.execute("nonexistent.md");

        assert!(result.contains("not found"));
    }

    #[test]
    fn execute__should_reject_directories() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("subdir")).unwrap();
        let tool = DeleteTool::new(dir.path().to_path_buf());

        let result = tool.execute("subdir");

        assert!(result.contains("not a file"));
    }

    #[test]
    fn execute__should_reject_path_outside_root() {
        let dir = tempdir().unwrap();
        let tool = DeleteTool::new(dir.path().to_path_buf());

        let result = tool.execute("../../etc/passwd");

        assert!(result.contains("Error:"));
    }

    #[test]
    fn execute__should_reject_empty_input() {
        let dir = tempdir().unwrap();
        let tool = DeleteTool::new(dir.path().to_path_buf());

        assert!(tool.execute("").contains("no file path"));
        assert!(tool.execute("   ").contains("no file path"));
    }
}
