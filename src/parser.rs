/// A parsed `@magent` directive found in markdown content.
pub struct Directive {
    /// The prompt text after `@magent`.
    pub prompt: String,
    /// 1-based line number where the directive appears.
    pub line: usize,
    /// Whether this directive already has a response block.
    pub processed: bool,
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
        if let Some(prompt) = extract_prompt(line) {
            let processed = has_response_block(&lines, i + 1);
            directives.push(Directive {
                prompt,
                line: i + 1,
                processed,
            });
        }
    }

    directives
}

/// Extract the prompt from a line containing `@magent`, if present.
///
/// Handles both `@magent prompt` and `@magent(options) prompt` forms.
/// Returns `None` if no `@magent` is found or if the prompt is empty.
fn extract_prompt(line: &str) -> Option<String> {
    let marker = "@magent";
    let idx = line.find(marker)?;
    let rest = &line[idx + marker.len()..];

    // Skip optional (options) block
    let rest = if rest.starts_with('(') {
        let close = rest.find(')')?;
        &rest[close + 1..]
    } else {
        rest
    };

    let prompt = rest.trim();
    if prompt.is_empty() {
        return None;
    }

    Some(prompt.to_string())
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
        let content = "@magent(model:claude) explain this\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].prompt, "explain this");
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
        let content = "@magent(model:claude,in:1h) check the listings\n";

        // When
        let directives = parse_directives(content);

        // Then
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].prompt, "check the listings");
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
}
