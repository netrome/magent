use std::fs;
use std::path::PathBuf;

/// Creates or overwrites files in the knowledge base.
pub struct WriteTool {
    root: PathBuf,
}

impl WriteTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Write a file. Input format: path on first line, `---` separator, then content.
    ///
    /// Never fails — errors are returned as the result string so the LLM
    /// can see what went wrong and adjust.
    pub fn execute(&self, input: &str) -> String {
        let (path_str, content) = match parse_input(input) {
            Ok(parsed) => parsed,
            Err(msg) => return msg,
        };

        let file_path = match super::path::resolve_new_path(&self.root, path_str) {
            Ok(p) => p,
            Err(msg) => return msg,
        };

        if let Some(parent) = file_path.parent()
            && let Err(e) = fs::create_dir_all(parent)
        {
            return format!("Error: cannot create directory: {e}");
        }

        match fs::write(&file_path, content) {
            Ok(()) => format!("Created {path_str} ({} bytes)", content.len()),
            Err(e) => format!("Error: cannot write '{path_str}': {e}"),
        }
    }
}

/// Parse write tool input: first line is path, `---` separator, rest is content.
fn parse_input(input: &str) -> Result<(&str, &str), String> {
    let first_nl = input
        .find('\n')
        .ok_or("Error: expected format: path\\n---\\ncontent")?;

    let path = input[..first_nl].trim();
    if path.is_empty() {
        return Err("Error: empty file path".to_string());
    }

    let after_path = &input[first_nl + 1..];

    // Expect --- on the next line
    let sep_end = after_path.find('\n').unwrap_or(after_path.len());
    if after_path[..sep_end].trim() != "---" {
        return Err("Error: expected --- separator after path".to_string());
    }

    let content = if sep_end < after_path.len() {
        &after_path[sep_end + 1..]
    } else {
        ""
    };

    Ok((path, content))
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
    fn parse_input__should_extract_path_and_content() {
        let (path, content) = parse_input("notes/test.md\n---\n# Hello\n\nWorld\n").unwrap();
        assert_eq!(path, "notes/test.md");
        assert_eq!(content, "# Hello\n\nWorld\n");
    }

    #[test]
    fn parse_input__should_handle_empty_content() {
        let (path, content) = parse_input("test.md\n---").unwrap();
        assert_eq!(path, "test.md");
        assert_eq!(content, "");
    }

    #[test]
    fn parse_input__should_handle_empty_content_with_trailing_newline() {
        let (path, content) = parse_input("test.md\n---\n").unwrap();
        assert_eq!(path, "test.md");
        assert_eq!(content, "");
    }

    #[test]
    fn parse_input__should_reject_missing_separator() {
        let result = parse_input("test.md\ncontent without separator");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("---"));
    }

    #[test]
    fn parse_input__should_reject_missing_newline() {
        let result = parse_input("test.md");
        assert!(result.is_err());
    }

    #[test]
    fn parse_input__should_reject_empty_path() {
        let result = parse_input("\n---\ncontent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty file path"));
    }

    #[test]
    fn parse_input__should_preserve_content_with_triple_dashes() {
        // --- in content (not on the separator line) should be preserved
        let (path, content) = parse_input("test.md\n---\nfoo\n---\nbar\n").unwrap();
        assert_eq!(path, "test.md");
        assert_eq!(content, "foo\n---\nbar\n");
    }

    // --- execute ---

    #[test]
    fn execute__should_create_new_file() {
        let dir = tempdir().unwrap();
        let tool = WriteTool::new(dir.path().to_path_buf());

        let result = tool.execute("test.md\n---\n# Hello\n");

        assert_eq!(result, "Created test.md (8 bytes)");
        assert_eq!(
            fs::read_to_string(dir.path().join("test.md")).unwrap(),
            "# Hello\n"
        );
    }

    #[test]
    fn execute__should_create_intermediate_directories() {
        let dir = tempdir().unwrap();
        let tool = WriteTool::new(dir.path().to_path_buf());

        let result = tool.execute("a/b/c/deep.md\n---\ncontent");

        assert!(result.starts_with("Created a/b/c/deep.md"));
        assert_eq!(
            fs::read_to_string(dir.path().join("a/b/c/deep.md")).unwrap(),
            "content"
        );
    }

    #[test]
    fn execute__should_overwrite_existing_file() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "existing.md", "old content");
        let tool = WriteTool::new(dir.path().to_path_buf());

        let result = tool.execute("existing.md\n---\nnew content");

        assert!(result.starts_with("Created existing.md"));
        assert_eq!(
            fs::read_to_string(dir.path().join("existing.md")).unwrap(),
            "new content"
        );
    }

    #[test]
    fn execute__should_create_empty_file() {
        let dir = tempdir().unwrap();
        let tool = WriteTool::new(dir.path().to_path_buf());

        let result = tool.execute("empty.md\n---");

        assert_eq!(result, "Created empty.md (0 bytes)");
        assert_eq!(fs::read_to_string(dir.path().join("empty.md")).unwrap(), "");
    }

    #[test]
    fn execute__should_reject_path_outside_root() {
        let dir = tempdir().unwrap();
        let tool = WriteTool::new(dir.path().to_path_buf());

        let result = tool.execute("../../etc/evil.md\n---\nbad");

        assert!(result.contains("outside the knowledge base"));
        assert!(!Path::new("/etc/evil.md").exists());
    }

    #[test]
    fn execute__should_reject_missing_separator() {
        let dir = tempdir().unwrap();
        let tool = WriteTool::new(dir.path().to_path_buf());

        let result = tool.execute("test.md\ncontent");

        assert!(result.starts_with("Error:"));
    }

    #[test]
    fn execute__should_report_byte_count() {
        let dir = tempdir().unwrap();
        let tool = WriteTool::new(dir.path().to_path_buf());

        let result = tool.execute("test.md\n---\nhello");

        assert_eq!(result, "Created test.md (5 bytes)");
    }
}
