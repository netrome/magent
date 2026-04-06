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
    /// then one or more `<search>`/`<replace>` tag pairs.
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
            } else if let Some((start, end)) = find_whitespace_tolerant(&modified, &block.search) {
                modified.replace_range(start..end, &block.replace);
                applied += 1;
            } else {
                let mut detail = format!("Block {}: search text not found", i + 1);
                if let Some(best) = find_best_match(&modified, &block.search) {
                    detail.push_str(&format!(
                        "\n  Best match ({}/{} lines) near line {}:",
                        best.matching_lines, best.total_lines, best.line_number
                    ));
                    for line in best.content.lines() {
                        detail.push_str(&format!("\n    {line}"));
                    }
                }
                details.push(detail);
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

/// Parse one or more `<search>`/`<replace>` tag pairs.
///
/// Content between tags has one leading and one trailing newline stripped
/// (if present), so tags can sit on their own lines without affecting the
/// matched text.
fn parse_blocks(input: &str) -> Result<Vec<EditBlock>, String> {
    let mut blocks = Vec::new();
    let mut pos = 0;

    while let Some(offset) = input[pos..].find("<search>") {
        let tag_end = pos + offset + "<search>".len();

        // Strip one leading newline after <search>
        let content_start = if input[tag_end..].starts_with('\n') {
            tag_end + 1
        } else {
            tag_end
        };

        let search_close = input[content_start..]
            .find("</search>")
            .map(|i| content_start + i)
            .ok_or("Error: missing </search> tag")?;

        // Strip one trailing newline before </search>
        let content_end =
            if search_close > content_start && input.as_bytes()[search_close - 1] == b'\n' {
                search_close - 1
            } else {
                search_close
            };

        let search_text = &input[content_start..content_end];

        let after_search = search_close + "</search>".len();

        let replace_open = input[after_search..]
            .find("<replace>")
            .map(|i| after_search + i)
            .ok_or("Error: missing <replace> tag after </search>")?;

        let rep_tag_end = replace_open + "<replace>".len();

        let rep_content_start = if input[rep_tag_end..].starts_with('\n') {
            rep_tag_end + 1
        } else {
            rep_tag_end
        };

        let replace_close = input[rep_content_start..]
            .find("</replace>")
            .map(|i| rep_content_start + i)
            .ok_or("Error: missing </replace> tag")?;

        let rep_content_end =
            if replace_close > rep_content_start && input.as_bytes()[replace_close - 1] == b'\n' {
                replace_close - 1
            } else {
                replace_close
            };

        let replace_text = &input[rep_content_start..rep_content_end];

        blocks.push(EditBlock {
            search: search_text.to_string(),
            replace: replace_text.to_string(),
        });

        pos = replace_close + "</replace>".len();
    }

    Ok(blocks)
}

/// Find `search` in `content` using whitespace-tolerant line matching.
///
/// Each line is compared after stripping leading/trailing whitespace.
/// Line count must match exactly. Returns the byte range in `content`
/// that corresponds to the matched lines.
fn find_whitespace_tolerant(content: &str, search: &str) -> Option<(usize, usize)> {
    let search_lines: Vec<&str> = search.lines().collect();
    if search_lines.is_empty() {
        return None;
    }

    let search_trimmed: Vec<&str> = search_lines.iter().map(|l| l.trim()).collect();

    // Collect content lines with their byte offsets.
    let content_lines: Vec<(usize, &str)> = {
        let mut lines = Vec::new();
        let mut offset = 0;
        for line in content.lines() {
            lines.push((offset, line));
            offset += line.len() + 1; // +1 for \n
        }
        lines
    };

    if search_lines.len() > content_lines.len() {
        return None;
    }

    for i in 0..=content_lines.len() - search_lines.len() {
        let all_match =
            (0..search_lines.len()).all(|j| content_lines[i + j].1.trim() == search_trimmed[j]);

        if all_match {
            let start = content_lines[i].0;
            let last = i + search_lines.len() - 1;
            let end = content_lines[last].0 + content_lines[last].1.len();
            return Some((start, end));
        }
    }

    None
}

struct BestMatch {
    matching_lines: usize,
    total_lines: usize,
    line_number: usize, // 1-based
    content: String,
}

/// Slide a window over `content` and find the region most similar to `search`.
///
/// Scores by counting lines where trimmed content matches. Returns the best
/// region if it exceeds 50% of lines matching.
fn find_best_match(content: &str, search: &str) -> Option<BestMatch> {
    let search_lines: Vec<&str> = search.lines().collect();
    let content_lines: Vec<&str> = content.lines().collect();

    if search_lines.is_empty() || content_lines.is_empty() {
        return None;
    }

    let search_trimmed: Vec<&str> = search_lines.iter().map(|l| l.trim()).collect();
    let window = search_lines.len();

    if window > content_lines.len() {
        return None;
    }

    let mut best_score = 0;
    let mut best_idx = 0;

    for i in 0..=content_lines.len() - window {
        let score = (0..window)
            .filter(|&j| content_lines[i + j].trim() == search_trimmed[j])
            .count();

        if score > best_score {
            best_score = score;
            best_idx = i;
        }
    }

    // >50% threshold
    if best_score * 2 <= search_lines.len() {
        return None;
    }

    Some(BestMatch {
        matching_lines: best_score,
        total_lines: search_lines.len(),
        line_number: best_idx + 1,
        content: content_lines[best_idx..best_idx + window].join("\n"),
    })
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
        let input = "<search>\nold text\n</search>\n<replace>\nnew text\n</replace>";
        let blocks = parse_blocks(input).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "old text");
        assert_eq!(blocks[0].replace, "new text");
    }

    #[test]
    fn parse_blocks__should_parse_multiple_blocks() {
        let input = "\
<search>
first old
</search>
<replace>
first new
</replace>
<search>
second old
</search>
<replace>
second new
</replace>";
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
<search>
line one
line two
line three
</search>
<replace>
replaced one
replaced two
</replace>";
        let blocks = parse_blocks(input).unwrap();
        assert_eq!(blocks[0].search, "line one\nline two\nline three");
        assert_eq!(blocks[0].replace, "replaced one\nreplaced two");
    }

    #[test]
    fn parse_blocks__should_handle_empty_replace() {
        let input = "<search>\ndelete me\n</search>\n<replace></replace>";
        let blocks = parse_blocks(input).unwrap();
        assert_eq!(blocks[0].search, "delete me");
        assert_eq!(blocks[0].replace, "");
    }

    #[test]
    fn parse_blocks__should_return_empty_for_no_tags() {
        let blocks = parse_blocks("just some text").unwrap();
        assert!(blocks.is_empty());
    }

    #[test]
    fn parse_blocks__should_error_on_missing_close_search() {
        let input = "<search>\nold text";
        let result = parse_blocks(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("</search>"));
    }

    #[test]
    fn parse_blocks__should_error_on_missing_replace_after_search() {
        let input = "<search>\nold text\n</search>";
        let result = parse_blocks(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("<replace>"));
    }

    #[test]
    fn parse_blocks__should_error_on_missing_close_replace() {
        let input = "<search>\nold text\n</search>\n<replace>\nnew text";
        let result = parse_blocks(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("</replace>"));
    }

    // --- parse_input ---

    #[test]
    fn parse_input__should_extract_path_and_blocks() {
        let input = "notes/test.md\n<search>\nold\n</search>\n<replace>\nnew\n</replace>";
        let (path, blocks) = parse_input(input).unwrap();
        assert_eq!(path, "notes/test.md");
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn parse_input__should_reject_empty_path() {
        let input = "\n<search>\nold\n</search>\n<replace>\nnew\n</replace>";
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

        let result = tool.execute(
            "test.md\n<search>\nHello world\n</search>\n<replace>\nHello Rust\n</replace>",
        );

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
<search>
AAA
</search>
<replace>
111
</replace>
<search>
CCC
</search>
<replace>
333
</replace>";

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

        let result = tool.execute(
            "test.md\n<search>\nnonexistent\n</search>\n<replace>\nreplacement\n</replace>",
        );

        assert!(result.contains("Applied 0/1 edits to test.md"));
        assert!(result.contains("Block 1: search text not found"));
    }

    #[test]
    fn execute__should_handle_partial_success() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "AAA\nBBB");
        let tool = EditTool::new(dir.path().to_path_buf());

        let input = "test.md\n\
<search>
AAA
</search>
<replace>
111
</replace>
<search>
ZZZ
</search>
<replace>
999
</replace>";

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

        let result = tool.execute("test.md\n<search>\nfoo\n</search>\n<replace>\nbaz\n</replace>");

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

        let result = tool.execute("test.md\n<search>\ndelete me\n\n</search>\n<replace></replace>");

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
            tool.execute("nonexistent.md\n<search>\nold\n</search>\n<replace>\nnew\n</replace>");

        assert!(result.contains("Error:"));
    }

    #[test]
    fn execute__should_reject_path_outside_root() {
        let dir = tempdir().unwrap();
        let tool = EditTool::new(dir.path().to_path_buf());

        let result =
            tool.execute("../../etc/passwd\n<search>\nold\n</search>\n<replace>\nnew\n</replace>");

        assert!(result.contains("Error:"));
    }

    #[test]
    fn execute__should_not_write_when_all_edits_fail() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "original content");
        let tool = EditTool::new(dir.path().to_path_buf());

        tool.execute(
            "test.md\n<search>\nnonexistent\n</search>\n<replace>\nreplacement\n</replace>",
        );

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
<search>
AAA
</search>
<replace>
CCC
</replace>
<search>
CCCBBB
</search>
<replace>
DONE
</replace>";

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
<search>
Old paragraph
with two lines.
</search>
<replace>
New paragraph
with three lines
of content.
</replace>";

        let result = tool.execute(input);

        assert_eq!(result, "Applied 1/1 edits to test.md");
        assert_eq!(
            fs::read_to_string(dir.path().join("test.md")).unwrap(),
            "# Title\n\nNew paragraph\nwith three lines\nof content.\n\n## Next"
        );
    }

    // --- find_whitespace_tolerant ---

    #[test]
    fn find_whitespace_tolerant__should_match_different_indentation() {
        let content = "fn main() {\n    let x = 1;\n}\n";
        let search = "  let x = 1;";
        let result = find_whitespace_tolerant(content, search);
        assert!(result.is_some());
        let (start, end) = result.unwrap();
        assert_eq!(&content[start..end], "    let x = 1;");
    }

    #[test]
    fn find_whitespace_tolerant__should_match_tabs_vs_spaces() {
        let content = "\tlet x = 1;\n\tlet y = 2;";
        let search = "    let x = 1;\n    let y = 2;";
        let result = find_whitespace_tolerant(content, search);
        assert!(result.is_some());
        let (start, end) = result.unwrap();
        assert_eq!(&content[start..end], "\tlet x = 1;\n\tlet y = 2;");
    }

    #[test]
    fn find_whitespace_tolerant__should_match_trailing_spaces() {
        let content = "hello world";
        let search = "hello world   ";
        let result = find_whitespace_tolerant(content, search);
        assert!(result.is_some());
        let (start, end) = result.unwrap();
        assert_eq!(&content[start..end], "hello world");
    }

    #[test]
    fn find_whitespace_tolerant__should_fail_when_content_differs() {
        let content = "let x = 1;";
        let search = "let x = 2;";
        assert!(find_whitespace_tolerant(content, search).is_none());
    }

    #[test]
    fn find_whitespace_tolerant__should_fail_when_line_count_differs() {
        let content = "line one\nline two";
        let search = "line one\nline two\nline three";
        assert!(find_whitespace_tolerant(content, search).is_none());
    }

    #[test]
    fn find_whitespace_tolerant__should_return_correct_offsets_in_middle_of_file() {
        let content = "aaa\n  bbb\n  ccc\nddd";
        let search = "bbb\nccc";
        let result = find_whitespace_tolerant(content, search);
        assert!(result.is_some());
        let (start, end) = result.unwrap();
        assert_eq!(&content[start..end], "  bbb\n  ccc");
    }

    // --- find_best_match ---

    #[test]
    fn find_best_match__should_find_similar_region_above_threshold() {
        let content = "fn main() {\n    let x = 1;\n    let y = 2;\n    println!(\"hi\");\n}";
        // 3/4 lines match (fn main, let x, println) — differs on let y vs let z
        let search = "fn main() {\n    let x = 1;\n    let z = 99;\n    println!(\"hi\");";
        let result = find_best_match(content, search);
        assert!(result.is_some());
        let best = result.unwrap();
        assert_eq!(best.matching_lines, 3);
        assert_eq!(best.total_lines, 4);
        assert_eq!(best.line_number, 1);
    }

    #[test]
    fn find_best_match__should_return_none_below_threshold() {
        let content = "aaa\nbbb\nccc\nddd";
        let search = "xxx\nyyy\nzzz\nwww";
        assert!(find_best_match(content, search).is_none());
    }

    #[test]
    fn find_best_match__should_report_correct_line_number() {
        // The similar region starts at line 3 (1-based)
        let content = "header\nblank\nfn foo() {\n    let a = 1;\n    let b = 2;\n}";
        let search = "fn foo() {\n    let a = 1;\n    let c = 3;\n}";
        let result = find_best_match(content, search);
        assert!(result.is_some());
        let best = result.unwrap();
        assert_eq!(best.line_number, 3);
        assert_eq!(best.matching_lines, 3); // fn foo, let a, } match; let c differs
        assert_eq!(best.total_lines, 4);
    }

    // --- execute: whitespace-tolerant matching ---

    #[test]
    fn execute__whitespace__should_match_with_different_indentation() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "fn main() {\n    let x = 1;\n}");
        let tool = EditTool::new(dir.path().to_path_buf());

        let input = "test.md\n\
<search>
fn main() {
  let x = 1;
}
</search>
<replace>
fn main() {
    let x = 2;
}
</replace>";

        let result = tool.execute(input);

        assert_eq!(result, "Applied 1/1 edits to test.md");
        assert_eq!(
            fs::read_to_string(dir.path().join("test.md")).unwrap(),
            "fn main() {\n    let x = 2;\n}"
        );
    }

    #[test]
    fn execute__whitespace__should_prefer_exact_match() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "  hello\nhello");
        let tool = EditTool::new(dir.path().to_path_buf());

        // Exact match for "hello" (no leading spaces) should match the second line
        let result =
            tool.execute("test.md\n<search>\nhello\n</search>\n<replace>\nworld\n</replace>");

        assert_eq!(result, "Applied 1/1 edits to test.md");
        assert_eq!(
            fs::read_to_string(dir.path().join("test.md")).unwrap(),
            "  world\nhello"
        );
    }

    #[test]
    fn execute__whitespace__should_match_tabs_vs_spaces() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "\tindented line");
        let tool = EditTool::new(dir.path().to_path_buf());

        let result = tool.execute(
            "test.md\n<search>\n    indented line\n</search>\n<replace>\n\tmodified line\n</replace>",
        );

        assert_eq!(result, "Applied 1/1 edits to test.md");
        assert_eq!(
            fs::read_to_string(dir.path().join("test.md")).unwrap(),
            "\tmodified line"
        );
    }

    // --- execute: error messages ---

    #[test]
    fn execute__error__should_show_best_match_when_similar_region_exists() {
        let dir = tempdir().unwrap();
        create_file(
            dir.path(),
            "test.md",
            "fn main() {\n    let x = 1;\n    let y = 2;\n    println!(\"sum\");\n}",
        );
        let tool = EditTool::new(dir.path().to_path_buf());

        // 3/4 non-whitespace lines match, but "let z = 99" differs from "let y = 2"
        let input = "test.md\n\
<search>
    let x = 1;
    let z = 99;
    println!(\"sum\");
}
</search>
<replace>
    changed
</replace>";

        let result = tool.execute(input);

        assert!(result.contains("Applied 0/1"));
        assert!(result.contains("Best match (3/4 lines) near line 2:"));
        assert!(result.contains("let y = 2;"));
    }

    #[test]
    fn execute__error__should_not_show_best_match_when_no_similar_region() {
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "completely\ndifferent\ncontent");
        let tool = EditTool::new(dir.path().to_path_buf());

        let result =
            tool.execute("test.md\n<search>\nxxx\nyyy\nzzz\n</search>\n<replace>\naaa\n</replace>");

        assert!(result.contains("Block 1: search text not found"));
        assert!(!result.contains("Best match"));
    }
}
