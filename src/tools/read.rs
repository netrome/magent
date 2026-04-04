use std::fs;
use std::path::PathBuf;

/// Reads file content from the knowledge base.
pub struct ReadTool {
    root: PathBuf,
}

impl ReadTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Read a file, optionally restricted to a line range.
    ///
    /// Never fails — errors are returned as the result string so the LLM
    /// can see what went wrong and adjust.
    pub fn execute(&self, input: &str) -> String {
        let (path_str, range) = parse_input(input);

        if path_str.is_empty() {
            return "Error: no file path provided".to_string();
        }

        let file_path = match self.resolve_path(path_str) {
            Ok(p) => p,
            Err(msg) => return msg,
        };

        let content = match fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(_) => return format!("Error: file '{path_str}' not found"),
        };

        let lines: Vec<&str> = content.lines().collect();

        match range {
            Some((start, end)) => format_range(&lines, start, end),
            None => format_all(&lines),
        }
    }

    fn resolve_path(&self, path_str: &str) -> Result<PathBuf, String> {
        let file_path = self.root.join(path_str);
        if !file_path.exists() {
            return Err(format!("Error: file '{path_str}' not found"));
        }
        super::path::resolve_path(&self.root, path_str)
    }
}

// --- Input parsing ---

/// Split input into (file_path, optional line range).
/// Format: `path/to/file.md` or `path/to/file.md 40-60`
fn parse_input(input: &str) -> (&str, Option<(usize, usize)>) {
    let trimmed = input.trim();

    // Split off the last whitespace-separated token and check if it's a range
    if let Some(i) = trimmed.rfind(char::is_whitespace) {
        let (before, after) = (&trimmed[..i], trimmed[i..].trim());
        if let Some(range) = parse_range(after) {
            return (before.trim(), Some(range));
        }
    }

    (trimmed, None)
}

/// Parse a `start-end` range (1-indexed, inclusive). Returns None if not a valid range.
fn parse_range(s: &str) -> Option<(usize, usize)> {
    let (start_str, end_str) = s.split_once('-')?;
    let start: usize = start_str.parse().ok()?;
    let end: usize = end_str.parse().ok()?;
    if start == 0 || end == 0 || start > end {
        return None;
    }
    Some((start, end))
}

// --- Formatting ---

fn format_all(lines: &[&str]) -> String {
    let width = line_number_width(lines.len());
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        out.push_str(&format!("{:>width$}: {}\n", i + 1, line));
    }
    out
}

fn format_range(lines: &[&str], start: usize, end: usize) -> String {
    let clamped_end = end.min(lines.len());
    if start > lines.len() {
        return format!(
            "Error: line range {start}-{end} is beyond end of file ({} lines)",
            lines.len()
        );
    }
    let width = line_number_width(clamped_end);
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate().take(clamped_end).skip(start - 1) {
        out.push_str(&format!("{:>width$}: {}\n", i + 1, line));
    }
    out
}

fn line_number_width(max_line: usize) -> usize {
    if max_line == 0 {
        return 1;
    }
    (max_line as f64).log10().floor() as usize + 1
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
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
    fn parse_input__should_extract_path_only() {
        let (path, range) = parse_input("notes/rust.md");
        assert_eq!(path, "notes/rust.md");
        assert_eq!(range, None);
    }

    #[test]
    fn parse_input__should_extract_path_and_range() {
        let (path, range) = parse_input("notes/rust.md 40-60");
        assert_eq!(path, "notes/rust.md");
        assert_eq!(range, Some((40, 60)));
    }

    #[test]
    fn parse_input__should_handle_whitespace() {
        let (path, range) = parse_input("  notes/rust.md  40-60  ");
        assert_eq!(path, "notes/rust.md");
        assert_eq!(range, Some((40, 60)));
    }

    #[test]
    fn parse_input__should_treat_non_range_suffix_as_part_of_path() {
        // "notarange" is not a valid range, so the whole thing is the path
        let (path, range) = parse_input("notes/rust.md notarange");
        assert_eq!(path, "notes/rust.md notarange");
        assert_eq!(range, None);
    }

    #[test]
    fn parse_input__should_handle_empty_input() {
        let (path, range) = parse_input("");
        assert_eq!(path, "");
        assert_eq!(range, None);
    }

    // --- parse_range ---

    #[test]
    fn parse_range__should_parse_valid_range() {
        assert_eq!(parse_range("1-10"), Some((1, 10)));
        assert_eq!(parse_range("40-60"), Some((40, 60)));
    }

    #[test]
    fn parse_range__should_reject_invalid_formats() {
        assert_eq!(parse_range("10"), None);
        assert_eq!(parse_range("abc"), None);
        assert_eq!(parse_range("1-abc"), None);
        assert_eq!(parse_range("abc-10"), None);
        assert_eq!(parse_range("-10"), None);
    }

    #[test]
    fn parse_range__should_reject_zero_indices() {
        assert_eq!(parse_range("0-10"), None);
        assert_eq!(parse_range("1-0"), None);
    }

    #[test]
    fn parse_range__should_reject_inverted_range() {
        assert_eq!(parse_range("10-5"), None);
    }

    #[test]
    fn parse_range__should_accept_single_line_range() {
        assert_eq!(parse_range("5-5"), Some((5, 5)));
    }

    // --- execute (integration) ---

    #[test]
    fn execute__should_return_file_content_with_line_numbers() {
        // Given
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "line one\nline two\nline three");
        let tool = ReadTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("test.md");

        // Then
        assert_eq!(result, "1: line one\n2: line two\n3: line three\n");
    }

    #[test]
    fn execute__should_return_line_range() {
        // Given
        let dir = tempdir().unwrap();
        create_file(
            dir.path(),
            "test.md",
            "line 1\nline 2\nline 3\nline 4\nline 5",
        );
        let tool = ReadTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("test.md 2-4");

        // Then
        assert_eq!(result, "2: line 2\n3: line 3\n4: line 4\n");
    }

    #[test]
    fn execute__should_clamp_range_to_file_length() {
        // Given
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "line 1\nline 2\nline 3");
        let tool = ReadTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("test.md 2-100");

        // Then
        assert_eq!(result, "2: line 2\n3: line 3\n");
    }

    #[test]
    fn execute__should_error_for_range_beyond_file() {
        // Given
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "line 1\nline 2");
        let tool = ReadTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("test.md 10-20");

        // Then
        assert!(result.starts_with("Error: line range 10-20 is beyond end of file"));
    }

    #[test]
    fn execute__should_read_single_line() {
        // Given
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "line 1\nline 2\nline 3");
        let tool = ReadTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("test.md 2-2");

        // Then
        assert_eq!(result, "2: line 2\n");
    }

    #[test]
    fn execute__should_error_for_missing_file() {
        // Given
        let dir = tempdir().unwrap();
        let tool = ReadTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("nonexistent.md");

        // Then
        assert_eq!(result, "Error: file 'nonexistent.md' not found");
    }

    #[test]
    fn execute__should_error_for_empty_input() {
        // Given
        let dir = tempdir().unwrap();
        let tool = ReadTool::new(dir.path().to_path_buf());

        // When / Then
        assert_eq!(tool.execute(""), "Error: no file path provided");
        assert_eq!(tool.execute("   "), "Error: no file path provided");
    }

    #[test]
    fn execute__should_reject_path_outside_root() {
        // Given
        let dir = tempdir().unwrap();
        let tool = ReadTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("../../etc/passwd");

        // Then
        assert!(result.contains("outside the knowledge base") || result.contains("not found"));
    }

    #[test]
    fn execute__should_reject_symlink_escape() {
        // Given
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        create_file(outside.path(), "secret.md", "secret content");
        symlink(
            outside.path().join("secret.md"),
            dir.path().join("escape.md"),
        )
        .unwrap();
        let tool = ReadTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("escape.md");

        // Then
        assert!(result.contains("outside the knowledge base"));
    }

    #[test]
    fn execute__should_read_files_in_subdirectories() {
        // Given
        let dir = tempdir().unwrap();
        create_file(dir.path(), "notes/rust.md", "Rust content here");
        let tool = ReadTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("notes/rust.md");

        // Then
        assert_eq!(result, "1: Rust content here\n");
    }

    #[test]
    fn execute__should_align_line_numbers_for_large_files() {
        // Given: a file with 12 lines (so line numbers need 2-digit width)
        let dir = tempdir().unwrap();
        let content = (1..=12)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        create_file(dir.path(), "test.md", &content);
        let tool = ReadTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("test.md");

        // Then: single-digit line numbers should be right-aligned
        assert!(result.contains(" 1: line 1\n"));
        assert!(result.contains("12: line 12\n"));
    }

    #[test]
    fn execute__should_handle_empty_file() {
        // Given
        let dir = tempdir().unwrap();
        create_file(dir.path(), "empty.md", "");
        let tool = ReadTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("empty.md");

        // Then
        assert_eq!(result, "");
    }
}
