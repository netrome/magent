use std::collections::HashMap;

/// A parsed `@magent` directive found in markdown content.
pub struct Directive {
    /// The prompt text after `@magent`.
    pub prompt: String,
    /// 1-based line number where the directive appears.
    pub line: usize,
    /// Whether this directive already has a response block.
    pub processed: bool,
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

    for (i, line) in lines.iter().enumerate() {
        if let Some((prompt, options)) = extract_prompt(line) {
            let processed = has_response_block(&lines, i + 1);
            directives.push(Directive {
                prompt,
                line: i + 1,
                processed,
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

/// Check whether a `<magent-response>` block appears in `lines[from..]`
/// before the next `@magent` directive.
fn has_response_block(lines: &[&str], from: usize) -> bool {
    for line in &lines[from..] {
        let trimmed = line.trim();
        if trimmed == "<magent-response>" {
            return true;
        }
        if extract_prompt(line).is_some() {
            return false;
        }
    }
    false
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
        assert!(!directives[0].processed);
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
        assert!(directives[0].processed);
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
        assert!(directives[0].processed);
        assert_eq!(directives[1].prompt, "second question");
        assert!(!directives[1].processed);
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
        assert!(directives[0].processed);
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
        assert!(!directives[0].processed);
        assert!(directives[1].processed);
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
    fn parse_options__should_handle_whitespace_variations() {
        // Given — no spaces around colons, extra spaces around commas
        let options = parse_options("context:a.md ,  b.md ,model:claude");

        // Then
        assert_eq!(options.get("context").unwrap(), "a.md, b.md");
        assert_eq!(options.get("model").unwrap(), "claude");
    }
}
