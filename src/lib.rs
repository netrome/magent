pub mod edit;
pub mod llm;
pub mod parser;
pub mod watcher;
pub mod writer;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use tokio::sync::mpsc;

use llm::LlmClient;

#[derive(Parser)]
#[command(name = "magent", about = "A markdown-native AI agent daemon")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Watch a directory for @magent directives
    Watch {
        /// Directory to watch for markdown files
        directory: PathBuf,

        /// LLM API base URL
        #[arg(long, default_value = "http://localhost:11434/v1")]
        api_url: String,

        /// Model name
        #[arg(long, default_value = "llama3")]
        model: String,
    },
}

pub async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Watch {
            directory,
            api_url,
            model,
        } => {
            if !directory.exists() {
                return Err(format!("{} does not exist", directory.display()).into());
            }
            if !directory.is_dir() {
                return Err(format!("{} is not a directory", directory.display()).into());
            }

            let api_key = std::env::var("MAGENT_API_KEY").ok();
            let client = llm::ChatClient::new(api_url, model, api_key);

            let (tx, rx) = mpsc::channel(100);
            let _watcher = watcher::start(&directory, tx)?;

            println!("Watching {}...", directory.display());

            process_events(rx, &client).await;

            Ok(())
        }
    }
}

async fn process_events(mut rx: mpsc::Receiver<PathBuf>, client: &impl LlmClient) {
    loop {
        let path = tokio::select! {
            Some(path) = rx.recv() => path,
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down.");
                break;
            }
        };

        tokio::select! {
            _ = process_file(&path, client) => {}
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down.");
                break;
            }
        }
    }
}

async fn process_file(path: &Path, client: &impl LlmClient) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read {}: {e}", path.display());
            return;
        }
    };

    let directives = parser::parse_directives(&content);

    for directive in directives.iter().filter(|d| !d.processed) {
        println!("Processing: @magent {}", directive.prompt);

        let llm_response = match client.complete(&directive.prompt, Some(&content)).await {
            Ok(r) => r,
            Err(e) => format!("**Error:** {e}"),
        };

        let response = format_response(&llm_response);

        if let Err(e) = writer::write_response(path, &directive.prompt, &response) {
            eprintln!("Failed to write response: {e}");
        }
    }
}

/// Format an LLM response for writing into the document.
///
/// If the response contains edit blocks, formats them as `status="proposed"`.
/// Otherwise returns the response as-is (plain text).
fn format_response(llm_response: &str) -> String {
    let (edits, summary) = edit::parse_edits(llm_response);
    if edits.is_empty() {
        llm_response.to_string()
    } else {
        edit::format_proposed_edits(&edits, &summary)
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeLlm(String);

    impl LlmClient for FakeLlm {
        async fn complete(
            &self,
            _prompt: &str,
            _document: Option<&str>,
        ) -> Result<String, llm::LlmError> {
            Ok(self.0.clone())
        }
    }

    struct FailingLlm(String);

    impl LlmClient for FailingLlm {
        async fn complete(
            &self,
            _prompt: &str,
            _document: Option<&str>,
        ) -> Result<String, llm::LlmError> {
            Err(llm::LlmError::Connection(self.0.clone()))
        }
    }

    struct SpyLlm {
        response: String,
        call_count: AtomicUsize,
    }

    impl LlmClient for SpyLlm {
        async fn complete(
            &self,
            _prompt: &str,
            _document: Option<&str>,
        ) -> Result<String, llm::LlmError> {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            Ok(self.response.clone())
        }
    }

    struct DocumentCaptureLlm {
        captured_document: std::sync::Mutex<Option<String>>,
    }

    impl DocumentCaptureLlm {
        fn new() -> Self {
            Self {
                captured_document: std::sync::Mutex::new(None),
            }
        }
    }

    impl LlmClient for DocumentCaptureLlm {
        async fn complete(
            &self,
            _prompt: &str,
            document: Option<&str>,
        ) -> Result<String, llm::LlmError> {
            *self.captured_document.lock().unwrap() = document.map(String::from);
            Ok("Response.".to_string())
        }
    }

    #[tokio::test]
    async fn run__should_fail_when_directory_does_not_exist() {
        // Given
        let cli = Cli {
            command: Command::Watch {
                directory: PathBuf::from("/nonexistent/path"),
                api_url: "http://localhost:11434/v1".to_string(),
                model: "llama3".to_string(),
            },
        };

        // When
        let result = run(cli).await;

        // Then
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("does not exist"),
            "error should mention directory does not exist"
        );
    }

    #[tokio::test]
    async fn run__should_fail_when_path_is_a_file() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("not_a_dir.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let cli = Cli {
            command: Command::Watch {
                directory: file_path,
                api_url: "http://localhost:11434/v1".to_string(),
                model: "llama3".to_string(),
            },
        };

        // When
        let result = run(cli).await;

        // Then
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("not a directory"),
            "error should mention path is not a directory"
        );
    }

    #[tokio::test]
    async fn process_file__should_call_llm_and_write_response() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "# Notes\n\n@magent why is the sky blue?\n").unwrap();
        let client = FakeLlm("Rayleigh scattering.".to_string());

        // When
        process_file(&path, &client).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<magent-response>"));
        assert!(content.contains("Rayleigh scattering."));
        assert!(content.contains("</magent-response>"));
    }

    #[tokio::test]
    async fn process_file__should_write_error_on_llm_failure() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent hello\n").unwrap();
        let client = FailingLlm("Connection refused (http://localhost:11434/v1)".to_string());

        // When
        process_file(&path, &client).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<magent-response>"));
        assert!(content.contains("**Error:**"));
        assert!(content.contains("Connection refused"));
        assert!(content.contains("</magent-response>"));
    }

    #[tokio::test]
    async fn process_file__should_skip_already_processed_directives() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(
            &path,
            "@magent hello\n\n<magent-response>\nHi!\n</magent-response>\n",
        )
        .unwrap();
        let client = SpyLlm {
            response: "Should not appear".to_string(),
            call_count: AtomicUsize::new(0),
        };

        // When
        process_file(&path, &client).await;

        // Then
        assert_eq!(client.call_count.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn process_file__should_handle_multiple_directives() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent first question\n\n@magent second question\n").unwrap();
        let client = FakeLlm("Answer.".to_string());

        // When
        process_file(&path, &client).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        let directives = parser::parse_directives(&content);
        assert_eq!(directives.len(), 2);
        assert!(directives[0].processed);
        assert!(directives[1].processed);
    }

    #[tokio::test]
    async fn process_file__should_pass_document_content_to_llm() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        let file_content = "# My Essay\n\nThe sky is blue.\n\n@magent summarize this document\n";
        std::fs::write(&path, file_content).unwrap();
        let client = DocumentCaptureLlm::new();

        // When
        process_file(&path, &client).await;

        // Then
        let captured = client.captured_document.lock().unwrap();
        let document = captured.as_ref().expect("document should be passed to LLM");
        assert!(
            document.contains("# My Essay"),
            "document context should contain the file heading"
        );
        assert!(
            document.contains("The sky is blue."),
            "document context should contain the file body"
        );
    }

    #[tokio::test]
    async fn process_file__should_write_proposed_edits_when_llm_returns_edit_blocks() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(
            &path,
            "# Links\n\n- [Rust](htps://rust-lang.org)\n\n@magent fix the broken URL\n",
        )
        .unwrap();

        let llm_response = "\
Fixed the URL:
<magent-edit>
<magent-search>htps://rust-lang.org</magent-search>
<magent-replace>https://rust-lang.org</magent-replace>
</magent-edit>";
        let client = FakeLlm(llm_response.to_string());

        // When
        process_file(&path, &client).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("status=\"proposed\""),
            "should contain proposed status"
        );
        assert!(content.contains("<magent-search>htps://rust-lang.org</magent-search>"));
        assert!(content.contains("<magent-replace>https://rust-lang.org</magent-replace>"));
        // Document content should NOT be modified
        assert!(
            content.contains("- [Rust](htps://rust-lang.org)"),
            "original document should be unchanged"
        );
    }

    #[tokio::test]
    async fn process_file__should_not_contain_edit_blocks_for_plain_text_response() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent what is Rust?\n").unwrap();
        let client = FakeLlm("Rust is a systems programming language.".to_string());

        // When
        process_file(&path, &client).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Rust is a systems programming language."));
        assert!(
            !content.contains("magent-edit"),
            "should not contain edit blocks"
        );
    }

    #[tokio::test]
    async fn process_file__should_write_proposed_edits_parseable_by_edit_block_parser() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "old text here\n\n@magent fix this\n").unwrap();

        let llm_response = "\
<magent-edit>
<magent-search>old text</magent-search>
<magent-replace>new text</magent-replace>
</magent-edit>";
        let client = FakeLlm(llm_response.to_string());

        // When
        process_file(&path, &client).await;

        // Then — the written response should be parseable by parse_edit_blocks
        let content = std::fs::read_to_string(&path).unwrap();
        let resp_start = content.find("<magent-response>\n").unwrap() + "<magent-response>\n".len();
        let resp_end = content.find("\n</magent-response>").unwrap();
        let response_content = &content[resp_start..resp_end];

        let blocks = edit::parse_edit_blocks(response_content);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].status, edit::EditStatus::Proposed);
        assert_eq!(blocks[0].search, "old text");
        assert_eq!(blocks[0].replace, "new text");
    }
}
