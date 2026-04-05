use std::fs;
use std::path::PathBuf;

/// Search-and-replace editor for files in the knowledge base.
pub struct EditTool {
    root: PathBuf,
}

impl EditTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Apply search/replace edits to a file. Input format: path on first line,
    /// then one or more conflict-marker-style search/replace blocks.
    ///
    /// Never fails — errors are returned as the result string so the LLM
    /// can see what went wrong and adjust.
    pub fn execute(&self, input: &str) -> String {
        let (path_str, blocks) = match parse_input(input) {
            Ok(parsed) => parsed,
            Err(msg) => return msg,
        };

        let file_path = match super::path::resolve_path(&self.root, path_str) {
            Ok(p) => p,
            Err(msg) => return msg,
        };

        let content = match fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(_) => return format!("Error: cannot read '{path_str}'"),
        };

        let mut modified = content;
        let mut applied = 0;
        let mut details = Vec::new();

        for (i, block) in blocks.iter().enumerate() {
            if let Some(pos) = modified.find(&block.search) {
                modified.replace_range(pos..pos + block.search.len(), &block.replace);
                applied += 1;
            } else {
                details.push(format!("Block {}: search text not found", i + 1));
            }
        }

        if applied > 0
            && let Err(e) = fs::write(&file_path, &modified)
        {
            return format!("Error: cannot write '{path_str}': {e}");
        }

        let total = blocks.len();
        let mut result = format!("Applied {applied}/{total} edits to {path_str}");
        for detail in &details {
            result.push_str(&format!("\n  {detail}"));
        }
        result
    }
}

#[derive(Debug)]
struct EditBlock {
    search: String,
    replace: String,
}

/// Parse edit tool input: first line is path, then search/replace blocks.
fn parse_input(input: &str) -> Result<(&str, Vec<EditBlock>), String> {
    let first_nl = input
        .find('\n')
        .ok_or("Error: expected file path followed by search/replace blocks")?;

    let path = input[..first_nl].trim();
    if path.is_empty() {
        return Err("Error: empty file path".to_string());
    }

    let rest = &input[first_nl + 1..];
    let blocks = parse_blocks(rest)?;

    if blocks.is_empty() {
        return Err("Error: no search/replace blocks found".to_string());
    }

    Ok((path, blocks))
}

/// Parse one or more conflict-marker-style search/replace blocks.
fn parse_blocks(input: &str) -> Result<Vec<EditBlock>, String> {
    let mut blocks = Vec::new();
    let mut remaining = input;

    while let Some(start) = remaining.find("<<<<<<< SEARCH") {
        let after_marker = &remaining[start..];

        let search_start = after_marker
            .find('\n')
            .map(|i| start + i + 1)
            .ok_or("Error: malformed search block")?;

        let divider = remaining[search_start..]
            .find("\n=======\n")
            .map(|i| search_start + i)
            .ok_or("Error: missing ======= divider")?;

        let replace_start = divider + "\n=======\n".len();

        let end = remaining[replace_start..]
            .find("\n>>>>>>> REPLACE")
            .map(|i| replace_start + i)
            .ok_or("Error: missing >>>>>>> REPLACE marker")?;

        let search = &remaining[search_start..divider];
        let replace = &remaining[replace_start..end];

        blocks.push(EditBlock {
            search: search.to_string(),
            replace: replace.to_string(),
        });

        // Advance past the REPLACE marker
        let after_replace = end + "\n>>>>>>> REPLACE".len();
        remaining = &remaining[after_replace..];
    }

    Ok(blocks)
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

    // --- parse_blocks ---

    #[test]
    fn parse_blocks__should_parse_single_block() {
        let input = "<<<<<<< SEARCH\nold text\n=======\nnew text\n>>>>>>> REPLACE";
        let blocks = parse_blocks(input).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "old text");
        assert_eq!(blocks[0].replace, "new text");
    }

    #[test]
    fn parse_blocks__should_parse_multiple_blocks() {
        let input = "\
<<<<<<< SEARCH
first old
=======
first new
>>>>>>> REPLACE
<<<<<<< SEARCH
second old
=======
second new
>>>>>>> REPLACE";
        let blocks = parse_blocks(input).unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].search, "first old");
        assert_eq!(blocks[0].replace, "first new");
        assert_eq!(blocks[1].search, "second old");
        assert_eq!(blocks[1].replace, "second new");
    }

    #[test]
    fn parse_blocks__should_handle_multiline_content() {
        let input = "\
<<<<<<< SEARCH
line one
line two
line three
=======
replaced one
replaced two
>>>>>>> REPLACE";
        let blocks = parse_blocks(input).unwrap();
        assert_eq!(blocks[0].search, "line one\nline two\nline three");
        assert_eq!(blocks[0].replace, "replaced one\nreplaced two");
    }

    #[test]
    fn parse_blocks__should_handle_empty_replace() {
        let input = "<<<<<<< SEARCH\ndelete me\n=======\n\n>>>>>>> REPLACE";
        let blocks = parse_blocks(input).unwrap();
        assert_eq!(blocks[0].search, "delete me");
        assert_eq!(blocks[0].replace, "");
    }

    #[test]
    fn parse_blocks__should_return_empty_for_no_markers() {
        let blocks = parse_blocks("just some text").unwrap();
        assert!(blocks.is_empty());
    }

    #[test]
    fn parse_blocks__should_error_on_missing_divider() {
        let input = "<<<<<<< SEARCH\nold text\n>>>>>>> REPLACE";
        let result = parse_blocks(input);
        assert!(result.is_err());
    }

    // --- parse_input ---

    #[test]
    fn parse_input__should_extract_path_and_blocks() {
        let input = "notes/test.md\n<<<<<<< SEARCH\nold\n=======\nnew\n>>>>>>> REPLACE";
        let (path, blocks) = parse_input(input).unwrap();
        assert_eq!(path, "notes/test.md");
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn parse_input__should_reject_empty_path() {
        let input = "\n<<<<<<< SEARCH\nold\n=======\nnew\n>>>>>>> REPLACE";
        let result = parse_input(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty file path"));
    }

    #[test]
    fn parse_input__should_reject_no_blocks() {
        let input = "test.md\njust some text";
        let result = parse_input(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no search/replace blocks"));
    }

    // --- execute ---

    #[test]
    fn execute__should_apply_single_edit() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "Hello world");
        let tool = EditTool::new(dir.path().to_path_buf());

        let result = tool
            .execute("test.md\n<<<<<<< SEARCH\nHello world\n=======\nHello Rust\n>>>>>>> REPLACE");

        assert_eq!(result, "Applied 1/1 edits to test.md");
        assert_eq!(
            fs::read_to_string(dir.path().join("test.md")).unwrap(),
            "Hello Rust"
        );
    }

    #[test]
    fn execute__should_apply_multiple_edits() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "AAA\nBBB\nCCC");
        let tool = EditTool::new(dir.path().to_path_buf());

        let input = "test.md\n\
<<<<<<< SEARCH
AAA
=======
111
>>>>>>> REPLACE
<<<<<<< SEARCH
CCC
=======
333
>>>>>>> REPLACE";

        let result = tool.execute(input);

        assert_eq!(result, "Applied 2/2 edits to test.md");
        assert_eq!(
            fs::read_to_string(dir.path().join("test.md")).unwrap(),
            "111\nBBB\n333"
        );
    }

    #[test]
    fn execute__should_report_failed_blocks() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "Hello world");
        let tool = EditTool::new(dir.path().to_path_buf());

        let result = tool
            .execute("test.md\n<<<<<<< SEARCH\nnonexistent\n=======\nreplacement\n>>>>>>> REPLACE");

        assert!(result.contains("Applied 0/1 edits to test.md"));
        assert!(result.contains("Block 1: search text not found"));
    }

    #[test]
    fn execute__should_handle_partial_success() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "AAA\nBBB");
        let tool = EditTool::new(dir.path().to_path_buf());

        let input = "test.md\n\
<<<<<<< SEARCH
AAA
=======
111
>>>>>>> REPLACE
<<<<<<< SEARCH
ZZZ
=======
999
>>>>>>> REPLACE";

        let result = tool.execute(input);

        assert!(result.contains("Applied 1/2 edits to test.md"));
        assert!(result.contains("Block 2: search text not found"));
        assert_eq!(
            fs::read_to_string(dir.path().join("test.md")).unwrap(),
            "111\nBBB"
        );
    }

    #[test]
    fn execute__should_replace_first_occurrence_only() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "foo bar foo bar");
        let tool = EditTool::new(dir.path().to_path_buf());

        let result = tool.execute("test.md\n<<<<<<< SEARCH\nfoo\n=======\nbaz\n>>>>>>> REPLACE");

        assert_eq!(result, "Applied 1/1 edits to test.md");
        assert_eq!(
            fs::read_to_string(dir.path().join("test.md")).unwrap(),
            "baz bar foo bar"
        );
    }

    #[test]
    fn execute__should_handle_deletion() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "keep\ndelete me\nkeep");
        let tool = EditTool::new(dir.path().to_path_buf());

        let result =
            tool.execute("test.md\n<<<<<<< SEARCH\ndelete me\n\n=======\n\n>>>>>>> REPLACE");

        assert_eq!(result, "Applied 1/1 edits to test.md");
        assert_eq!(
            fs::read_to_string(dir.path().join("test.md")).unwrap(),
            "keep\nkeep"
        );
    }

    #[test]
    fn execute__should_error_for_missing_file() {
        let dir = tempdir().unwrap();
        let tool = EditTool::new(dir.path().to_path_buf());

        let result =
            tool.execute("nonexistent.md\n<<<<<<< SEARCH\nold\n=======\nnew\n>>>>>>> REPLACE");

        assert!(result.contains("Error:"));
    }

    #[test]
    fn execute__should_reject_path_outside_root() {
        let dir = tempdir().unwrap();
        let tool = EditTool::new(dir.path().to_path_buf());

        let result =
            tool.execute("../../etc/passwd\n<<<<<<< SEARCH\nold\n=======\nnew\n>>>>>>> REPLACE");

        assert!(result.contains("Error:"));
    }

    #[test]
    fn execute__should_not_write_when_all_edits_fail() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "original content");
        let tool = EditTool::new(dir.path().to_path_buf());

        tool.execute("test.md\n<<<<<<< SEARCH\nnonexistent\n=======\nreplacement\n>>>>>>> REPLACE");

        assert_eq!(
            fs::read_to_string(dir.path().join("test.md")).unwrap(),
            "original content"
        );
    }

    #[test]
    fn execute__should_apply_edits_sequentially() {
        // Earlier edits affect later ones (search text matches against modified content)
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "AAABBB");
        let tool = EditTool::new(dir.path().to_path_buf());

        let input = "test.md\n\
<<<<<<< SEARCH
AAA
=======
CCC
>>>>>>> REPLACE
<<<<<<< SEARCH
CCCBBB
=======
DONE
>>>>>>> REPLACE";

        let result = tool.execute(input);

        assert_eq!(result, "Applied 2/2 edits to test.md");
        assert_eq!(
            fs::read_to_string(dir.path().join("test.md")).unwrap(),
            "DONE"
        );
    }

    #[test]
    fn execute__should_handle_multiline_search_and_replace() {
        let dir = tempdir().unwrap();
        create_file(
            dir.path(),
            "test.md",
            "# Title\n\nOld paragraph\nwith two lines.\n\n## Next",
        );
        let tool = EditTool::new(dir.path().to_path_buf());

        let input = "test.md\n\
<<<<<<< SEARCH
Old paragraph
with two lines.
=======
New paragraph
with three lines
of content.
>>>>>>> REPLACE";

        let result = tool.execute(input);

        assert_eq!(result, "Applied 1/1 edits to test.md");
        assert_eq!(
            fs::read_to_string(dir.path().join("test.md")).unwrap(),
            "# Title\n\nNew paragraph\nwith three lines\nof content.\n\n## Next"
        );
    }
}
