use std::fs;
use std::path::PathBuf;

/// Moves or renames files within the knowledge base.
pub struct MoveTool {
    root: PathBuf,
}

impl MoveTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Move or rename a file. Input format: `old/path.md -> new/path.md`
    ///
    /// Never fails — errors are returned as the result string so the LLM
    /// can see what went wrong and adjust.
    pub fn execute(&self, input: &str) -> String {
        let (src_str, dst_str) = match parse_input(input) {
            Ok(parsed) => parsed,
            Err(msg) => return msg,
        };

        // Source must exist
        let src_path = match super::path::resolve_path(&self.root, src_str) {
            Ok(p) => p,
            Err(_) => return format!("Error: source '{src_str}' not found"),
        };

        if !src_path.is_file() {
            return format!("Error: source '{src_str}' is not a file");
        }

        // Destination must be within root but may not exist yet
        let dst_path = match super::path::resolve_new_path(&self.root, dst_str) {
            Ok(p) => p,
            Err(msg) => return msg,
        };

        if dst_path.exists() {
            return format!("Error: destination '{dst_str}' already exists");
        }

        if let Some(parent) = dst_path.parent()
            && let Err(e) = fs::create_dir_all(parent)
        {
            return format!("Error: cannot create directory: {e}");
        }

        match fs::rename(&src_path, &dst_path) {
            Ok(()) => format!("Moved {src_str} -> {dst_str}"),
            Err(e) => format!("Error: cannot move '{src_str}': {e}"),
        }
    }
}

/// Parse move tool input: `old/path.md -> new/path.md`
fn parse_input(input: &str) -> Result<(&str, &str), String> {
    let (src, dst) = input
        .split_once("->")
        .ok_or("Error: expected format: old/path.md -> new/path.md")?;

    let src = src.trim();
    let dst = dst.trim();

    if src.is_empty() {
        return Err("Error: empty source path".to_string());
    }
    if dst.is_empty() {
        return Err("Error: empty destination path".to_string());
    }

    Ok((src, dst))
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

    // --- parse_input ---

    #[test]
    fn parse_input__should_extract_src_and_dst() {
        let (src, dst) = parse_input("old.md -> new.md").unwrap();
        assert_eq!(src, "old.md");
        assert_eq!(dst, "new.md");
    }

    #[test]
    fn parse_input__should_handle_paths_with_directories() {
        let (src, dst) = parse_input("notes/draft.md -> notes/final.md").unwrap();
        assert_eq!(src, "notes/draft.md");
        assert_eq!(dst, "notes/final.md");
    }

    #[test]
    fn parse_input__should_trim_whitespace() {
        let (src, dst) = parse_input("  old.md  ->  new.md  ").unwrap();
        assert_eq!(src, "old.md");
        assert_eq!(dst, "new.md");
    }

    #[test]
    fn parse_input__should_reject_missing_arrow() {
        let result = parse_input("old.md new.md");
        assert!(result.is_err());
    }

    #[test]
    fn parse_input__should_reject_empty_source() {
        let result = parse_input(" -> new.md");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty source"));
    }

    #[test]
    fn parse_input__should_reject_empty_destination() {
        let result = parse_input("old.md -> ");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty destination"));
    }

    // --- execute ---

    #[test]
    fn execute__should_rename_file() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "old.md", "content");
        let tool = MoveTool::new(dir.path().to_path_buf());

        let result = tool.execute("old.md -> new.md");

        assert_eq!(result, "Moved old.md -> new.md");
        assert!(!dir.path().join("old.md").exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("new.md")).unwrap(),
            "content"
        );
    }

    #[test]
    fn execute__should_move_to_subdirectory() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "top.md", "content");
        let tool = MoveTool::new(dir.path().to_path_buf());

        let result = tool.execute("top.md -> sub/dir/moved.md");

        assert_eq!(result, "Moved top.md -> sub/dir/moved.md");
        assert!(!dir.path().join("top.md").exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("sub/dir/moved.md")).unwrap(),
            "content"
        );
    }

    #[test]
    fn execute__should_create_intermediate_directories() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "file.md", "content");
        let tool = MoveTool::new(dir.path().to_path_buf());

        let result = tool.execute("file.md -> a/b/c/file.md");

        assert!(result.starts_with("Moved"));
        assert!(dir.path().join("a/b/c/file.md").exists());
    }

    #[test]
    fn execute__should_fail_if_source_does_not_exist() {
        let dir = tempdir().unwrap();
        let tool = MoveTool::new(dir.path().to_path_buf());

        let result = tool.execute("nonexistent.md -> new.md");

        assert!(result.contains("not found"));
    }

    #[test]
    fn execute__should_fail_if_destination_exists() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "src.md", "source");
        create_file(dir.path(), "dst.md", "destination");
        let tool = MoveTool::new(dir.path().to_path_buf());

        let result = tool.execute("src.md -> dst.md");

        assert!(result.contains("already exists"));
        // Neither file should be modified
        assert_eq!(
            fs::read_to_string(dir.path().join("src.md")).unwrap(),
            "source"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("dst.md")).unwrap(),
            "destination"
        );
    }

    #[test]
    fn execute__should_reject_source_outside_root() {
        let dir = tempdir().unwrap();
        let tool = MoveTool::new(dir.path().to_path_buf());

        let result = tool.execute("../../etc/passwd -> stolen.md");

        assert!(result.contains("Error:"));
    }

    #[test]
    fn execute__should_reject_destination_outside_root() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "file.md", "content");
        let tool = MoveTool::new(dir.path().to_path_buf());

        let result = tool.execute("file.md -> ../../etc/evil.md");

        assert!(result.contains("outside the knowledge base"));
        // Source should still exist
        assert!(dir.path().join("file.md").exists());
    }

    #[test]
    fn execute__should_reject_moving_directories() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("subdir")).unwrap();
        create_file(dir.path(), "subdir/file.md", "content");
        let tool = MoveTool::new(dir.path().to_path_buf());

        let result = tool.execute("subdir -> newdir");

        assert!(result.contains("not a file"));
    }
}
