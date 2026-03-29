use std::path::Path;

use crate::parser;

/// Error type for response writing operations.
#[derive(Debug)]
pub enum WriteError {
    /// The directive prompt was not found (or was already processed).
    DirectiveNotFound(String),
    /// An I/O error occurred reading or writing the file.
    Io(std::io::Error),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::DirectiveNotFound(prompt) => {
                write!(f, "unprocessed directive not found: {prompt}")
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
}
