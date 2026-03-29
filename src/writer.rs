use std::path::Path;

use crate::parser::{self, DirectiveStatus};

/// Error type for response writing operations.
#[derive(Debug)]
pub enum WriteError {
    /// The directive prompt was not found.
    DirectiveNotFound(String),
    /// An I/O error occurred reading or writing the file.
    Io(std::io::Error),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::DirectiveNotFound(prompt) => {
                write!(f, "directive not found: {prompt}")
            }
            WriteError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self {
        WriteError::Io(e)
    }
}

/// Write an LLM response into a markdown file after the directive matching `prompt`.
///
/// Re-reads the file to find the directive's current position, inserts the
/// response block, and writes the file back. This avoids stale line numbers
/// when the file was edited between the LLM call and the write.
///
/// Returns `WriteError::DirectiveNotFound` if no unprocessed directive with
/// the given prompt exists in the file.
pub fn write_response(path: &Path, prompt: &str, response: &str) -> Result<(), WriteError> {
    let content = std::fs::read_to_string(path)?;

    let updated = insert_response(&content, prompt, response)
        .ok_or_else(|| WriteError::DirectiveNotFound(prompt.to_string()))?;

    std::fs::write(path, updated)?;
    Ok(())
}

/// Insert a response block after the first unprocessed directive matching `prompt`.
///
/// Returns `None` if no matching unprocessed directive is found.
/// The response block is separated from the directive by a blank line.
pub fn insert_response(content: &str, prompt: &str, response: &str) -> Option<String> {
    let directives = parser::parse_directives(content);

    let directive = directives
        .iter()
        .find(|d| d.status == parser::DirectiveStatus::Unprocessed && d.prompt == prompt)?;

    let lines: Vec<&str> = content.lines().collect();
    let insert_after = directive.line - 1; // convert 1-based to 0-based index

    let mut result = String::with_capacity(content.len() + response.len() + 64);

    for (i, line) in lines.iter().enumerate() {
        result.push_str(line);
        result.push('\n');

        if i == insert_after {
            result.push('\n');
            result.push_str("<magent-response>\n");
            result.push_str(response);
            if !response.is_empty() && !response.ends_with('\n') {
                result.push('\n');
            }
            result.push_str("</magent-response>\n");
        }
    }

    Some(result)
}

/// Write or replace a response block for a directive in a markdown file.
///
/// If the directive has no response block, creates one. If an in-progress or
/// paused response block already exists, replaces its content. When `in_progress`
/// is true, the opening tag includes `status="in-progress"`.
///
/// Returns `WriteError::DirectiveNotFound` if no directive with the given
/// prompt exists.
pub fn write_response_block(
    path: &Path,
    prompt: &str,
    content: &str,
    in_progress: bool,
) -> Result<(), WriteError> {
    let file_content = std::fs::read_to_string(path)?;

    let updated = upsert_response_block(&file_content, prompt, content, in_progress)
        .ok_or_else(|| WriteError::DirectiveNotFound(prompt.to_string()))?;

    std::fs::write(path, updated)?;
    Ok(())
}

/// Insert or replace a response block for the first directive matching `prompt`.
///
/// - If the directive has no response block (`Unprocessed`), inserts a new one.
/// - If the directive has an `InProgress` or `Paused` response block, replaces
///   its content.
/// - If the directive has a `Complete` response block, returns `None` (won't
///   overwrite a finished response).
///
/// Returns `None` if no matching directive is found or if the response is
/// already complete.
pub fn upsert_response_block(
    file_content: &str,
    prompt: &str,
    response: &str,
    in_progress: bool,
) -> Option<String> {
    let directives = parser::parse_directives(file_content);

    let directive = directives.iter().find(|d| {
        d.prompt == prompt
            && matches!(
                d.status,
                DirectiveStatus::Unprocessed
                    | DirectiveStatus::InProgress
                    | DirectiveStatus::Paused
            )
    })?;

    let lines: Vec<&str> = file_content.lines().collect();

    match directive.status {
        DirectiveStatus::Unprocessed => {
            let insert_after = directive.line - 1;
            Some(build_with_new_block(
                &lines,
                insert_after,
                response,
                in_progress,
            ))
        }
        DirectiveStatus::InProgress | DirectiveStatus::Paused => {
            let after_directive = directive.line; // 1-based, so this is the 0-based index after
            Some(build_with_replaced_block(
                &lines,
                after_directive,
                response,
                in_progress,
            ))
        }
        DirectiveStatus::Complete => None,
    }
}

/// Build file content with a new response block inserted after `insert_after`.
fn build_with_new_block(
    lines: &[&str],
    insert_after: usize,
    response: &str,
    in_progress: bool,
) -> String {
    let mut result = String::with_capacity(lines.len() * 40 + response.len() + 64);

    for (i, line) in lines.iter().enumerate() {
        result.push_str(line);
        result.push('\n');

        if i == insert_after {
            result.push('\n');
            push_response_block(&mut result, response, in_progress);
        }
    }

    result
}

/// Build file content with an existing response block replaced.
///
/// Finds the response block opening tag in `lines[from..]`, locates its
/// matching closing tag (handling nesting), and replaces everything between
/// them with the new content.
fn build_with_replaced_block(
    lines: &[&str],
    from: usize,
    response: &str,
    in_progress: bool,
) -> String {
    // Find the opening tag
    let open_idx = match lines[from..]
        .iter()
        .position(|l| parser::parse_response_open_tag(l.trim()).is_some())
    {
        Some(pos) => from + pos,
        None => return lines.join("\n") + "\n",
    };

    // Find the matching closing tag (handle nesting)
    let mut depth: usize = 1;
    let mut close_idx = None;
    for (i, line) in lines[open_idx + 1..].iter().enumerate() {
        let trimmed = line.trim();
        if parser::parse_response_open_tag(trimmed).is_some() {
            depth += 1;
        } else if trimmed == "</magent-response>" {
            depth -= 1;
            if depth == 0 {
                close_idx = Some(open_idx + 1 + i);
                break;
            }
        }
    }

    let close_idx = match close_idx {
        Some(idx) => idx,
        None => return lines.join("\n") + "\n",
    };

    // Build output: lines before open tag + new block + lines after close tag
    let mut result = String::with_capacity(lines.len() * 40 + response.len() + 64);

    for line in &lines[..open_idx] {
        result.push_str(line);
        result.push('\n');
    }

    push_response_block(&mut result, response, in_progress);

    for line in &lines[close_idx + 1..] {
        result.push_str(line);
        result.push('\n');
    }

    result
}

/// Append a response block (opening tag + content + closing tag) to a string.
fn push_response_block(out: &mut String, response: &str, in_progress: bool) {
    if in_progress {
        out.push_str("<magent-response status=\"in-progress\">\n");
    } else {
        out.push_str("<magent-response>\n");
    }
    out.push_str(response);
    if !response.is_empty() && !response.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("</magent-response>\n");
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn insert_response__should_insert_after_directive() {
        // Given
        let content = "@magent why is the sky blue?\n";
        let response = "Rayleigh scattering.";

        // When
        let result = insert_response(content, "why is the sky blue?", response).unwrap();

        // Then
        assert_eq!(
            result,
            "@magent why is the sky blue?\n\
             \n\
             <magent-response>\n\
             Rayleigh scattering.\n\
             </magent-response>\n"
        );
    }

    #[test]
    fn insert_response__should_preserve_content_after_directive() {
        // Given
        let content = "# Notes\n\n@magent summarize this\n\nSome other content.\n";
        let response = "Summary here.";

        // When
        let result = insert_response(content, "summarize this", response).unwrap();

        // Then
        assert_eq!(
            result,
            "# Notes\n\
             \n\
             @magent summarize this\n\
             \n\
             <magent-response>\n\
             Summary here.\n\
             </magent-response>\n\
             \n\
             Some other content.\n"
        );
    }

    #[test]
    fn insert_response__should_only_insert_after_matching_directive() {
        // Given
        let content = "@magent first question\n\n@magent second question\n";
        let response = "Answer to second.";

        // When
        let result = insert_response(content, "second question", response).unwrap();

        // Then
        assert_eq!(
            result,
            "@magent first question\n\
             \n\
             @magent second question\n\
             \n\
             <magent-response>\n\
             Answer to second.\n\
             </magent-response>\n"
        );
    }

    #[test]
    fn insert_response__should_handle_directive_at_end_of_file() {
        // Given
        let content = "# Title\n\n@magent explain this\n";
        let response = "Explanation.";

        // When
        let result = insert_response(content, "explain this", response).unwrap();

        // Then
        assert_eq!(
            result,
            "# Title\n\
             \n\
             @magent explain this\n\
             \n\
             <magent-response>\n\
             Explanation.\n\
             </magent-response>\n"
        );
    }

    #[test]
    fn insert_response__should_skip_already_processed_directive() {
        // Given
        let content = "@magent hello\n\n<magent-response>\nHi!\n</magent-response>\n";

        // When
        let result = insert_response(content, "hello", "New response");

        // Then
        assert!(result.is_none());
    }

    #[test]
    fn insert_response__should_return_none_when_prompt_not_found() {
        // Given
        let content = "@magent some other prompt\n";

        // When
        let result = insert_response(content, "nonexistent prompt", "Response");

        // Then
        assert!(result.is_none());
    }

    #[test]
    fn insert_response__should_handle_multiline_response() {
        // Given
        let content = "@magent list three things\n";
        let response = "1. One\n2. Two\n3. Three\n";

        // When
        let result = insert_response(content, "list three things", response).unwrap();

        // Then
        assert_eq!(
            result,
            "@magent list three things\n\
             \n\
             <magent-response>\n\
             1. One\n\
             2. Two\n\
             3. Three\n\
             </magent-response>\n"
        );
    }

    #[test]
    fn insert_response__should_match_first_unprocessed_among_duplicates() {
        // Given — two identical prompts, first is processed
        let content = "@magent hello\n\
                        \n\
                        <magent-response>\n\
                        Hi!\n\
                        </magent-response>\n\
                        \n\
                        @magent hello\n";
        let response = "Hello again!";

        // When
        let result = insert_response(content, "hello", response).unwrap();

        // Then — response is inserted after the second (unprocessed) directive
        assert!(result.contains("@magent hello\n\n<magent-response>\nHello again!"));
    }

    #[test]
    fn write_response__should_write_to_file() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent hello world\n").unwrap();

        // When
        write_response(&path, "hello world", "Hi there!").unwrap();

        // Then
        let result = std::fs::read_to_string(&path).unwrap();
        assert!(result.contains("<magent-response>"));
        assert!(result.contains("Hi there!"));
        assert!(result.contains("</magent-response>"));
    }

    #[test]
    fn write_response__should_return_error_when_directive_not_found() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "# Just a heading\n").unwrap();

        // When
        let result = write_response(&path, "nonexistent", "Response");

        // Then
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn write_response__should_roundtrip_with_parser() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "# Notes\n\n@magent explain rust\n\nMore content.\n").unwrap();

        // When
        write_response(&path, "explain rust", "Rust is a systems language.").unwrap();

        // Then — parser should now see the directive as processed
        let content = std::fs::read_to_string(&path).unwrap();
        let directives = parser::parse_directives(&content);
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].status, parser::DirectiveStatus::Complete);
    }

    // --- upsert_response_block tests ---

    #[test]
    fn upsert__should_create_in_progress_block_for_unprocessed_directive() {
        // Given
        let content = "@magent explain this\n";

        // When
        let result = upsert_response_block(content, "explain this", "Working...", true).unwrap();

        // Then
        assert_eq!(
            result,
            "@magent explain this\n\
             \n\
             <magent-response status=\"in-progress\">\n\
             Working...\n\
             </magent-response>\n"
        );
    }

    #[test]
    fn upsert__should_create_complete_block_for_unprocessed_directive() {
        // Given
        let content = "@magent explain this\n";

        // When
        let result = upsert_response_block(content, "explain this", "Done.", false).unwrap();

        // Then
        assert_eq!(
            result,
            "@magent explain this\n\
             \n\
             <magent-response>\n\
             Done.\n\
             </magent-response>\n"
        );
    }

    #[test]
    fn upsert__should_replace_in_progress_block_content() {
        // Given
        let content = "\
@magent find pricing

<magent-response status=\"in-progress\">
Step 1 done.
</magent-response>
";

        // When
        let result =
            upsert_response_block(content, "find pricing", "Step 1 done.\nStep 2 done.", true)
                .unwrap();

        // Then
        assert_eq!(
            result,
            "@magent find pricing\n\
             \n\
             <magent-response status=\"in-progress\">\n\
             Step 1 done.\n\
             Step 2 done.\n\
             </magent-response>\n"
        );
    }

    #[test]
    fn upsert__should_close_in_progress_block() {
        // Given
        let content = "\
@magent find pricing

<magent-response status=\"in-progress\">
Partial work.
</magent-response>
";

        // When — in_progress=false closes the block
        let result =
            upsert_response_block(content, "find pricing", "Final answer.", false).unwrap();

        // Then
        assert_eq!(
            result,
            "@magent find pricing\n\
             \n\
             <magent-response>\n\
             Final answer.\n\
             </magent-response>\n"
        );
    }

    #[test]
    fn upsert__should_replace_paused_block() {
        // Given
        let content = "\
@magent research topic

<magent-response status=\"paused\">
Paused at step 3.
</magent-response>
";

        // When — resuming with in_progress=true
        let result = upsert_response_block(
            content,
            "research topic",
            "Paused at step 3.\nStep 4 done.",
            true,
        )
        .unwrap();

        // Then
        assert_eq!(
            result,
            "@magent research topic\n\
             \n\
             <magent-response status=\"in-progress\">\n\
             Paused at step 3.\n\
             Step 4 done.\n\
             </magent-response>\n"
        );
    }

    #[test]
    fn upsert__should_not_overwrite_complete_response() {
        // Given
        let content = "\
@magent hello

<magent-response>
Done!
</magent-response>
";

        // When
        let result = upsert_response_block(content, "hello", "Overwrite attempt.", true);

        // Then
        assert!(result.is_none());
    }

    #[test]
    fn upsert__should_return_none_for_nonexistent_prompt() {
        // Given
        let content = "@magent hello\n";

        // When
        let result = upsert_response_block(content, "nonexistent", "Response", true);

        // Then
        assert!(result.is_none());
    }

    #[test]
    fn upsert__should_preserve_surrounding_content() {
        // Given
        let content = "\
# Notes

@magent summarize

<magent-response status=\"in-progress\">
Old content.
</magent-response>

## Other section

More text.
";

        // When
        let result = upsert_response_block(content, "summarize", "New content.", true).unwrap();

        // Then
        assert_eq!(
            result,
            "# Notes\n\
             \n\
             @magent summarize\n\
             \n\
             <magent-response status=\"in-progress\">\n\
             New content.\n\
             </magent-response>\n\
             \n\
             ## Other section\n\
             \n\
             More text.\n"
        );
    }

    #[test]
    fn upsert__should_handle_response_with_tool_calls() {
        // Given
        let content = "\
@magent search

<magent-response status=\"in-progress\">
Let me search.
</magent-response>
";
        let new_response = "\
Let me search.

<magent-tool-call>
search | query: pricing
</magent-tool-call>
<magent-tool-result>
Found results.
</magent-tool-result>";

        // When
        let result = upsert_response_block(content, "search", new_response, true).unwrap();

        // Then
        assert!(result.contains("<magent-tool-call>"));
        assert!(result.contains("<magent-tool-result>"));
        assert!(result.contains("status=\"in-progress\""));
    }

    #[test]
    fn upsert__should_handle_empty_response_content() {
        // Given
        let content = "@magent hello\n";

        // When
        let result = upsert_response_block(content, "hello", "", true).unwrap();

        // Then
        assert_eq!(
            result,
            "@magent hello\n\
             \n\
             <magent-response status=\"in-progress\">\n\
             </magent-response>\n"
        );
    }

    #[test]
    fn write_response_block__should_create_in_progress_file() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent hello\n").unwrap();

        // When
        write_response_block(&path, "hello", "Working...", true).unwrap();

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("status=\"in-progress\""));
        assert!(content.contains("Working..."));
        let directives = parser::parse_directives(&content);
        assert_eq!(directives[0].status, parser::DirectiveStatus::InProgress);
    }

    #[test]
    fn write_response_block__should_update_in_progress_file() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(
            &path,
            "@magent hello\n\n<magent-response status=\"in-progress\">\nStep 1.\n</magent-response>\n",
        )
        .unwrap();

        // When
        write_response_block(&path, "hello", "Step 1.\nStep 2.", true).unwrap();

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Step 1.\nStep 2."));
        assert!(content.contains("status=\"in-progress\""));
    }

    #[test]
    fn write_response_block__should_close_response() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(
            &path,
            "@magent hello\n\n<magent-response status=\"in-progress\">\nPartial.\n</magent-response>\n",
        )
        .unwrap();

        // When
        write_response_block(&path, "hello", "Final.", false).unwrap();

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<magent-response>\n"));
        assert!(!content.contains("in-progress"));
        let directives = parser::parse_directives(&content);
        assert_eq!(directives[0].status, parser::DirectiveStatus::Complete);
    }
}
