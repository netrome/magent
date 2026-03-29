use std::collections::HashMap;

/// Processing state of a directive's response block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectiveStatus {
    /// No response block exists yet.
    Unprocessed,
    /// The daemon is actively working on this directive.
    InProgress,
    /// The user has paused execution.
    Paused,
    /// The response is finished.
    Complete,
}

/// A parsed `@magent` directive found in markdown content.
pub struct Directive {
    /// The prompt text after `@magent`.
    pub prompt: String,
    /// 1-based line number where the directive appears.
    pub line: usize,
    /// Processing state of this directive.
    pub status: DirectiveStatus,
    /// Key-value options parsed from `@magent(key: value, ...)`.
    pub options: HashMap<String, String>,
}

/// Parse all `@magent` directives from markdown content.
///
/// Scans each line for `@magent` mentions, extracts the prompt text,
/// and checks whether a `<magent-response>` block follows before the
/// next directive.
pub fn parse_directives(content: &str) -> Vec<Directive> {
    let lines: Vec<&str> = content.lines().collect();
    let mut directives = Vec::new();
    let mut response_depth: usize = 0;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if parse_response_open_tag(trimmed).is_some() {
            response_depth += 1;
            continue;
        }
        if trimmed == "</magent-response>" {
            response_depth = response_depth.saturating_sub(1);
            continue;
        }

        if response_depth > 0 {
            continue;
        }

        if let Some((prompt, options)) = extract_prompt(line) {
            let status = response_block_status(&lines, i + 1);
            directives.push(Directive {
                prompt,
                line: i + 1,
                status,
                options,
            });
        }
    }

    directives
}

/// Extract the prompt and options from a line containing `@magent`, if present.
///
/// Handles both `@magent prompt` and `@magent(key: value, ...) prompt` forms.
/// Returns `None` if no `@magent` is found or if the prompt is empty.
fn extract_prompt(line: &str) -> Option<(String, HashMap<String, String>)> {
    let marker = "@magent";
    let idx = line.find(marker)?;
    let rest = &line[idx + marker.len()..];

    let (rest, options) = if rest.starts_with('(') {
        let close = rest.find(')')?;
        let options_str = &rest[1..close];
        let options = parse_options(options_str);
        (&rest[close + 1..], options)
    } else {
        (rest, HashMap::new())
    };

    let prompt = rest.trim();
    if prompt.is_empty() {
        return None;
    }

    Some((prompt.to_string(), options))
}

/// Parse a `key: value, ...` options string into a map.
///
/// A new key starts when a comma-separated token contains a colon.
/// Values for the same key are joined with `, `. This allows multi-value
/// options like `context: a.md, b.md` to work naturally.
fn parse_options(input: &str) -> HashMap<String, String> {
    let mut options = HashMap::new();
    let mut current_key: Option<String> = None;
    let mut current_values: Vec<String> = Vec::new();

    for token in input.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }

        if let Some((key, value)) = token.split_once(':') {
            // Flush previous key
            if let Some(key) = current_key.take() {
                options.insert(key, current_values.join(", "));
                current_values.clear();
            }
            current_key = Some(key.trim().to_string());
            let value = value.trim();
            if !value.is_empty() {
                current_values.push(value.to_string());
            }
        } else if current_key.is_some() {
            // Continuation value for the current key
            current_values.push(token.to_string());
        }
        // Tokens before any key are ignored
    }

    // Flush last key
    if let Some(key) = current_key {
        options.insert(key, current_values.join(", "));
    }

    options
}

/// Try to parse a `<magent-response>` opening tag, returning its status.
///
/// Returns `None` if the line is not a response tag. Returns the appropriate
/// `DirectiveStatus` based on the `status` attribute: `Complete` for bare
/// `<magent-response>`, `InProgress` for `status="in-progress"`, `Paused`
/// for `status="paused"`.
fn parse_response_open_tag(trimmed: &str) -> Option<DirectiveStatus> {
    if trimmed == "<magent-response>" {
        return Some(DirectiveStatus::Complete);
    }
    if trimmed.starts_with("<magent-response ") && trimmed.ends_with('>') {
        let status = match extract_attribute(trimmed, "status") {
            Some("in-progress") => DirectiveStatus::InProgress,
            Some("paused") => DirectiveStatus::Paused,
            _ => DirectiveStatus::Complete,
        };
        return Some(status);
    }
    None
}

/// Extract the value of a named attribute from an HTML-like tag.
///
/// Handles `name="value"` syntax. Returns `None` if the attribute is not found.
fn extract_attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let pattern = format!("{name}=\"");
    let start = tag.find(&pattern)? + pattern.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Determine the status of a directive by looking for a response block
/// in `lines[from..]` before the next `@magent` directive.
fn response_block_status(lines: &[&str], from: usize) -> DirectiveStatus {
    for line in &lines[from..] {
        let trimmed = line.trim();
        if let Some(status) = parse_response_open_tag(trimmed) {
            return status;
        }
        if extract_prompt(line).is_some() {
            return DirectiveStatus::Unprocessed;
        }
    }
    DirectiveStatus::Unprocessed
}

/// Extract the text content inside the response block for a given directive.
///
/// Finds the first directive matching `prompt`, then extracts everything
/// between its `<magent-response ...>` and `</magent-response>` tags.
/// Returns `None` if the directive has no response block.
pub fn extract_response_content(content: &str, prompt: &str) -> Option<String> {
    let directives = parse_directives(content);
    let directive = directives
        .iter()
        .find(|d| d.prompt == prompt && d.status != DirectiveStatus::Unprocessed)?;

    let lines: Vec<&str> = content.lines().collect();
    let after_directive = directive.line; // line is 1-based, so this is the 0-based index after

    // Find the opening tag
    let open_idx = lines[after_directive..]
        .iter()
        .position(|l| parse_response_open_tag(l.trim()).is_some())?
        + after_directive;

    // Find the matching closing tag (handle nesting)
    let mut depth: usize = 1;
    let mut close_idx = None;
    for (i, line) in lines[open_idx + 1..].iter().enumerate() {
        let trimmed = line.trim();
        if parse_response_open_tag(trimmed).is_some() {
            depth += 1;
        } else if trimmed == "</magent-response>" {
            depth -= 1;
            if depth == 0 {
                close_idx = Some(open_idx + 1 + i);
                break;
            }
        }
    }

    let close_idx = close_idx?;
    let content_lines = &lines[open_idx + 1..close_idx];
    Some(content_lines.join("\n"))
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn parse_directives__should_find_unprocessed_directive() {
        // Given
        let content = "@magent why is the sky blue?\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].prompt, "why is the sky blue?");
        assert_eq!(directives[0].line, 1);
        assert_eq!(directives[0].status, DirectiveStatus::Unprocessed);
        assert!(directives[0].options.is_empty());
    }

    #[test]
    fn parse_directives__should_detect_processed_directive() {
        // Given
        let content = "@magent why is the sky blue?\n\
                        \n\
                        <magent-response>\n\
                        Because of Rayleigh scattering.\n\
                        </magent-response>\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].status, DirectiveStatus::Complete);
    }

    #[test]
    fn parse_directives__should_return_empty_for_no_directives() {
        // Given
        let content = "# Just a heading\n\nSome regular text.\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert!(directives.is_empty());
    }

    #[test]
    fn parse_directives__should_handle_multiple_directives() {
        // Given
        let content = "@magent first question\n\
                        \n\
                        <magent-response>\n\
                        First answer.\n\
                        </magent-response>\n\
                        \n\
                        @magent second question\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].prompt, "first question");
        assert_eq!(directives[0].status, DirectiveStatus::Complete);
        assert_eq!(directives[1].prompt, "second question");
        assert_eq!(directives[1].status, DirectiveStatus::Unprocessed);
    }

    #[test]
    fn parse_directives__should_handle_options_syntax() {
        // Given
        let content = "@magent(model: claude) explain this\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].prompt, "explain this");
        assert_eq!(directives[0].options.get("model").unwrap(), "claude");
    }

    #[test]
    fn parse_directives__should_ignore_bare_magent_with_no_prompt() {
        // Given
        let content = "@magent\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert!(directives.is_empty());
    }

    #[test]
    fn parse_directives__should_find_directive_after_list_marker() {
        // Given
        let content = "- @magent summarize this section\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].prompt, "summarize this section");
    }

    #[test]
    fn parse_directives__should_detect_response_block_after_blank_lines() {
        // Given
        let content = "@magent why is the sky blue?\n\
                        \n\
                        \n\
                        <magent-response>\n\
                        Rayleigh scattering.\n\
                        </magent-response>\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].status, DirectiveStatus::Complete);
    }

    #[test]
    fn parse_directives__should_not_match_response_block_belonging_to_later_directive() {
        // Given — the response block belongs to the second directive, not the first
        let content = "@magent first question\n\
                        \n\
                        @magent second question\n\
                        \n\
                        <magent-response>\n\
                        Answer to second.\n\
                        </magent-response>\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].status, DirectiveStatus::Unprocessed);
        assert_eq!(directives[1].status, DirectiveStatus::Complete);
    }

    #[test]
    fn parse_directives__should_handle_options_with_multiple_params() {
        // Given
        let content = "@magent(model: claude, in: 1h) check the listings\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].prompt, "check the listings");
        assert_eq!(directives[0].options.get("model").unwrap(), "claude");
        assert_eq!(directives[0].options.get("in").unwrap(), "1h");
    }

    #[test]
    fn parse_directives__should_ignore_unclosed_options_paren() {
        // Given — malformed options, no closing paren
        let content = "@magent(model:claude check the listings\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert!(directives.is_empty());
    }

    #[test]
    fn parse_directives__should_parse_context_with_single_file() {
        // Given
        let content = "@magent(context: rust.md) explain error handling\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].prompt, "explain error handling");
        assert_eq!(directives[0].options.get("context").unwrap(), "rust.md");
    }

    #[test]
    fn parse_directives__should_parse_context_with_multiple_files() {
        // Given
        let content = "@magent(context: a.md, b.md) compare these\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].prompt, "compare these");
        assert_eq!(directives[0].options.get("context").unwrap(), "a.md, b.md");
    }

    #[test]
    fn parse_directives__should_parse_context_files_with_mixed_options() {
        // Given
        let content = "@magent(context: a.md, b.md, model: claude) summarize\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].prompt, "summarize");
        assert_eq!(directives[0].options.get("context").unwrap(), "a.md, b.md");
        assert_eq!(directives[0].options.get("model").unwrap(), "claude");
    }

    #[test]
    fn parse_directives__should_parse_context_with_subdirectory_paths() {
        // Given
        let content = "@magent(context: notes/rust.md, docs/go.md) compare\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(
            directives[0].options.get("context").unwrap(),
            "notes/rust.md, docs/go.md"
        );
    }

    #[test]
    fn parse_directives__should_return_empty_options_for_empty_parens() {
        // Given
        let content = "@magent() explain this\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].prompt, "explain this");
        assert!(directives[0].options.is_empty());
    }

    #[test]
    fn parse_directives__should_ignore_directives_inside_response_blocks() {
        // Given — @magent inside a response block (e.g. from search results)
        let content = "\
@magent summarize the bug

<magent-response>
Search found:
hello.md:1: @magent summarize the bug
The root cause is...
</magent-response>
";

        // When
        let directives = parse_directives(content);

        // Then — only the top-level directive, not the one inside the response
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].prompt, "summarize the bug");
        assert_eq!(directives[0].status, DirectiveStatus::Complete);
    }

    #[test]
    fn parse_directives__should_ignore_directives_inside_nested_response_blocks() {
        // Given — nested response blocks from re-executed queries
        let content = "\
@magent summarize the bug

<magent-response>
<magent-response>
@magent nested query that should be ignored
</magent-response>
The answer.
</magent-response>
";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].prompt, "summarize the bug");
        assert_eq!(directives[0].status, DirectiveStatus::Complete);
    }

    #[test]
    fn parse_directives__should_find_directive_after_response_block() {
        // Given — a new directive after a response block containing @magent text
        let content = "\
@magent first question

<magent-response>
search result: @magent first question
Answer to first.
</magent-response>

@magent second question
";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].prompt, "first question");
        assert_eq!(directives[0].status, DirectiveStatus::Complete);
        assert_eq!(directives[1].prompt, "second question");
        assert_eq!(directives[1].status, DirectiveStatus::Unprocessed);
    }

    #[test]
    fn parse_options__should_handle_whitespace_variations() {
        // Given — no spaces around colons, extra spaces around commas
        let options = parse_options("context:a.md ,  b.md ,model:claude");

        // Then
        assert_eq!(options.get("context").unwrap(), "a.md, b.md");
        assert_eq!(options.get("model").unwrap(), "claude");
    }

    #[test]
    fn parse_directives__should_detect_in_progress_response() {
        // Given
        let content = "\
@magent find the pricing page

<magent-response status=\"in-progress\">
Let me search for that.
</magent-response>
";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].status, DirectiveStatus::InProgress);
    }

    #[test]
    fn parse_directives__should_detect_paused_response() {
        // Given
        let content = "\
@magent find the pricing page

<magent-response status=\"paused\">
Let me search for that.
</magent-response>
";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].status, DirectiveStatus::Paused);
    }

    #[test]
    fn parse_directives__should_treat_unknown_status_as_complete() {
        // Given
        let content = "\
@magent hello

<magent-response status=\"unknown\">
Some text.
</magent-response>
";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].status, DirectiveStatus::Complete);
    }

    #[test]
    fn parse_directives__should_skip_directives_inside_in_progress_response() {
        // Given — directive text inside an in-progress response block
        let content = "\
@magent summarize the bug

<magent-response status=\"in-progress\">
Found: @magent summarize the bug
Working on it...
</magent-response>
";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].status, DirectiveStatus::InProgress);
    }

    #[test]
    fn extract_response_content__should_return_content_of_complete_response() {
        // Given
        let content = "\
@magent explain rust

<magent-response>
Rust is a systems language.
It emphasizes safety.
</magent-response>
";

        // When
        let result = extract_response_content(content, "explain rust");

        // Then
        assert_eq!(
            result.unwrap(),
            "Rust is a systems language.\nIt emphasizes safety."
        );
    }

    #[test]
    fn extract_response_content__should_return_content_of_in_progress_response() {
        // Given
        let content = "\
@magent find pricing

<magent-response status=\"in-progress\">
Let me search.

<magent-tool-call>
search | query: pricing
</magent-tool-call>
<magent-tool-result>
Found 2 results.
</magent-tool-result>
</magent-response>
";

        // When
        let result = extract_response_content(content, "find pricing");

        // Then
        let expected = "\
Let me search.

<magent-tool-call>
search | query: pricing
</magent-tool-call>
<magent-tool-result>
Found 2 results.
</magent-tool-result>";
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn extract_response_content__should_return_none_for_unprocessed_directive() {
        // Given
        let content = "@magent explain rust\n";

        // When
        let result = extract_response_content(content, "explain rust");

        // Then
        assert!(result.is_none());
    }

    #[test]
    fn extract_response_content__should_return_none_for_nonexistent_prompt() {
        // Given
        let content = "\
@magent explain rust

<magent-response>
Some answer.
</magent-response>
";

        // When
        let result = extract_response_content(content, "nonexistent");

        // Then
        assert!(result.is_none());
    }

    #[test]
    fn extract_response_content__should_handle_nested_response_blocks() {
        // Given — nested response block inside the content
        let content = "\
@magent summarize

<magent-response>
Found this:
<magent-response>
inner content
</magent-response>
Outer text.
</magent-response>
";

        // When
        let result = extract_response_content(content, "summarize");

        // Then
        let expected = "\
Found this:
<magent-response>
inner content
</magent-response>
Outer text.";
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn extract_response_content__should_return_empty_string_for_empty_response() {
        // Given
        let content = "\
@magent hello

<magent-response status=\"in-progress\">
</magent-response>
";

        // When
        let result = extract_response_content(content, "hello");

        // Then
        assert_eq!(result.unwrap(), "");
    }

    #[test]
    fn extract_response_content__should_find_correct_directive_among_multiple() {
        // Given
        let content = "\
@magent first

<magent-response>
First answer.
</magent-response>

@magent second

<magent-response status=\"in-progress\">
Working on second.
</magent-response>
";

        // When
        let result = extract_response_content(content, "second");

        // Then
        assert_eq!(result.unwrap(), "Working on second.");
    }
}
