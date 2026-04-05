pub mod context;
pub mod edit;
pub mod llm;
pub mod parser;
pub mod tool;
pub mod tools;
pub mod watcher;
pub mod writer;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use llm::LlmClient;
use tools::browser::RunBrowser;

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

            let browser = if tools::browser::is_available() {
                info!("Browser tool available (agent-browser detected)");
                Some(tools::browser::AgentBrowser)
            } else {
                debug!("Browser tool not available");
                None
            };

            let (tx, rx) = mpsc::channel(100);
            let _watcher = watcher::start(&directory, tx.clone())?;

            info!(directory = %directory.display(), "Watching for directives");

            // Resume any in-progress responses left from a previous crash
            for path in scan_in_progress(&directory) {
                info!(path = %path.display(), "Resuming in-progress response");
                let _ = tx.send(path).await;
            }

            process_events(rx, &client, &directory, browser.as_ref()).await;

            if browser.is_some() {
                tools::browser::close_session();
            }

            Ok(())
        }
    }
}

async fn process_events<B: RunBrowser>(
    mut rx: mpsc::Receiver<PathBuf>,
    client: &impl LlmClient,
    root: &Path,
    browser: Option<&B>,
) {
    loop {
        let path = tokio::select! {
            Some(path) = rx.recv() => path,
            _ = tokio::signal::ctrl_c() => {
                info!("Shutting down");
                break;
            }
        };

        tokio::select! {
            _ = process_file(&path, client, root, browser) => {}
            _ = tokio::signal::ctrl_c() => {
                info!("Shutting down");
                break;
            }
        }
    }
}

/// Scan a directory recursively for `.md` files with in-progress response blocks.
///
/// Used on startup to resume work interrupted by a crash.
fn scan_in_progress(dir: &Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    scan_dir(dir, &mut results);
    results
}

fn scan_dir(dir: &Path, results: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, results);
        } else if path.extension().is_some_and(|ext| ext == "md")
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            let directives = parser::parse_directives(&content);
            if directives
                .iter()
                .any(|d| d.status == parser::DirectiveStatus::InProgress)
            {
                results.push(path);
            }
        }
    }
}

const MAX_TOOL_CALLS: usize = 10;
const TOOL_CALL_STOP: &str = "</magent-tool-call>";

async fn process_file<B: RunBrowser>(
    path: &Path,
    client: &impl LlmClient,
    root: &Path,
    browser: Option<&B>,
) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            error!(path = %path.display(), error = %e, "Failed to read file");
            return;
        }
    };

    // Handle accepted edits before processing new directives.
    // The file write triggers the watcher again for any remaining work.
    if let Some(updated) = edit::process_accepted_edits(&content) {
        info!(path = %path.display(), "Applying accepted edits");
        if let Err(e) = std::fs::write(path, updated) {
            error!(path = %path.display(), error = %e, "Failed to write edits");
        }
        return;
    }

    let directives = parser::parse_directives(&content);

    for directive in directives.iter().filter(|d| {
        matches!(
            d.status,
            parser::DirectiveStatus::Unprocessed | parser::DirectiveStatus::InProgress
        )
    }) {
        let document = match directive.status {
            parser::DirectiveStatus::Unprocessed => {
                info!(prompt = %directive.prompt, "Processing directive");

                // Resolve context file references
                let context_files = match context::resolve_context_files(
                    &directive.options,
                    root,
                    path,
                ) {
                    Ok(files) => files,
                    Err(e) => {
                        let error_msg = format!("**Error:** {e}");
                        if let Err(e) = writer::write_response(path, &directive.prompt, &error_msg)
                        {
                            error!(path = %path.display(), error = %e, "Failed to write response");
                        }
                        continue;
                    }
                };

                let filename = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let document = context::build_context_string(&content, &filename, &context_files);

                // Open in-progress response block before starting the LLM loop
                if let Err(e) = writer::write_response_block(path, &directive.prompt, "", true) {
                    error!(path = %path.display(), error = %e, "Failed to open response block");
                    continue;
                }

                document
            }
            parser::DirectiveStatus::InProgress => {
                info!(prompt = %directive.prompt, "Resuming directive");

                // Re-read file for up-to-date content
                let current_content = match std::fs::read_to_string(path) {
                    Ok(c) => c,
                    Err(e) => {
                        error!(path = %path.display(), error = %e, "Failed to read file");
                        return;
                    }
                };

                let context_files = match context::resolve_context_files(
                    &directive.options,
                    root,
                    path,
                ) {
                    Ok(files) => files,
                    Err(e) => {
                        let error_msg = format!("**Error:** {e}");
                        if let Err(e) =
                            writer::write_response_block(path, &directive.prompt, &error_msg, false)
                        {
                            error!(path = %path.display(), error = %e, "Failed to write response");
                        }
                        continue;
                    }
                };

                let filename = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                context::build_context_string(&current_content, &filename, &context_files)
            }
            _ => continue,
        };

        process_directive(client, &directive.prompt, &document, root, browser, path).await;
    }
}

/// Run the tool-use loop for a single directive, writing incrementally.
///
/// Sends the prompt to the LLM with the document as context. If the LLM
/// calls tools (search, read, browser), executes them and feeds results back
/// for up to `MAX_TOOL_CALLS` rounds. Flushes progress to disk after each
/// LLM output and tool result. Re-reads the file before each LLM call to
/// detect user intervention (edits, deletion, pause).
async fn process_directive<B: RunBrowser>(
    client: &impl LlmClient,
    prompt: &str,
    document: &str,
    root: &Path,
    browser: Option<&B>,
    path: &Path,
) {
    let system_prompt = llm::build_system_prompt(document, browser.is_some());
    let mut messages = vec![
        llm::Message::system(&system_prompt),
        llm::Message::user(prompt),
    ];
    let mut full_response = String::new();
    let mut tool_call_count = 0;

    while let Ok(file_content) = std::fs::read_to_string(path) {
        // Re-read file to detect user intervention
        match parser::extract_response_content(&file_content, prompt) {
            None => {
                debug!("Response block deleted by user, stopping");
                break;
            }
            Some(file_response) => {
                if file_response != full_response {
                    debug!("User modified response, reconstructing messages");
                    full_response = file_response;
                    messages = tool::reconstruct_messages(&system_prompt, prompt, &full_response);
                }
            }
        }

        debug!(messages = messages.len(), "Sending LLM request");
        let llm_response = match client.complete_messages(&messages, &[TOOL_CALL_STOP]).await {
            Ok(r) => {
                debug!(len = r.len(), "LLM response received");
                r
            }
            Err(e) => {
                error!(error = %e, "LLM request failed");
                full_response.push_str(&format!("**Error:** {e}"));
                if let Err(e) = writer::write_response_block(path, prompt, &full_response, false) {
                    error!(error = %e, "Failed to write error response");
                }
                return;
            }
        };

        // Stop sequences strip the closing tag — re-add it for parsing
        let completed = complete_tool_call_tag(&llm_response);
        let (tool_call, _) = tool::parse_tool_call(&completed);

        let Some(call) = tool_call else {
            debug!("No tool call, writing final response");
            full_response.push_str(&llm_response);
            let formatted = format_response(&full_response);
            if let Err(e) = writer::write_response_block(path, prompt, &formatted, false) {
                error!(error = %e, "Failed to write final response");
            }
            return;
        };

        // Append the tool call (with closing tag) to the response
        full_response.push_str(&completed);
        full_response.push('\n');

        // Flush after LLM output
        if let Err(e) = writer::write_response_block(path, prompt, &full_response, true) {
            error!(error = %e, "Failed to flush response");
        }

        // Execute the tool
        tool_call_count += 1;
        info!(tool = %call.tool, round = tool_call_count, "Executing tool");
        let output = execute_tool(&call, root, browser);
        debug!(tool = %call.tool, output_len = output.len(), "Tool execution complete");
        let result = tool::ToolResult {
            tool: call.tool.clone(),
            output,
        };
        let result_text = tool::format_tool_result(&result);
        full_response.push_str(&result_text);
        full_response.push('\n');

        // Flush after tool result
        if let Err(e) = writer::write_response_block(path, prompt, &full_response, true) {
            error!(error = %e, "Failed to flush response");
        }

        // Feed result back for the next turn
        messages.push(llm::Message::assistant(&completed));
        messages.push(llm::Message::user(&result_text));

        if tool_call_count >= MAX_TOOL_CALLS {
            warn!("Tool call limit reached ({MAX_TOOL_CALLS})");
            full_response.push_str("(Tool call limit reached.)");
            if let Err(e) = writer::write_response_block(path, prompt, &full_response, false) {
                error!(error = %e, "Failed to write response");
            }
            return;
        }
    }
}

/// Append closing tag if the response has an unclosed tool call.
/// Handles the stop-sequence case where the API strips the stop token.
fn complete_tool_call_tag(response: &str) -> String {
    if response.contains("<magent-tool-call") && !response.contains(TOOL_CALL_STOP) {
        format!("{response}\n{TOOL_CALL_STOP}")
    } else {
        response.to_string()
    }
}

fn execute_tool<B: RunBrowser>(call: &tool::ToolCall, root: &Path, browser: Option<&B>) -> String {
    match call.tool.as_str() {
        "search" => tools::search::SearchTool::new(root.to_path_buf()).execute(&call.input),
        "read" => tools::read::ReadTool::new(root.to_path_buf()).execute(&call.input),
        "write" => tools::write::WriteTool::new(root.to_path_buf()).execute(&call.input),
        "edit" => tools::edit::EditTool::new(root.to_path_buf()).execute(&call.input),
        "move" => tools::mv::MoveTool::new(root.to_path_buf()).execute(&call.input),
        "browser" => match browser {
            Some(b) => b.run_browser(&call.input),
            None => "Error: browser tool is not available".to_string(),
        },
        _ => format!("Unknown tool: {}", call.tool),
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
        async fn complete_messages(
            &self,
            _messages: &[llm::Message],
            _stop: &[&str],
        ) -> Result<String, llm::LlmError> {
            Ok(self.0.clone())
        }
    }

    struct FailingLlm(String);

    impl LlmClient for FailingLlm {
        async fn complete_messages(
            &self,
            _messages: &[llm::Message],
            _stop: &[&str],
        ) -> Result<String, llm::LlmError> {
            Err(llm::LlmError::Connection(self.0.clone()))
        }
    }

    struct SpyLlm {
        response: String,
        call_count: AtomicUsize,
    }

    impl LlmClient for SpyLlm {
        async fn complete_messages(
            &self,
            _messages: &[llm::Message],
            _stop: &[&str],
        ) -> Result<String, llm::LlmError> {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            Ok(self.response.clone())
        }
    }

    struct MessageCaptureLlm {
        captured_messages: std::sync::Mutex<Option<Vec<llm::Message>>>,
    }

    impl MessageCaptureLlm {
        fn new() -> Self {
            Self {
                captured_messages: std::sync::Mutex::new(None),
            }
        }
    }

    impl LlmClient for MessageCaptureLlm {
        async fn complete_messages(
            &self,
            messages: &[llm::Message],
            _stop: &[&str],
        ) -> Result<String, llm::LlmError> {
            *self.captured_messages.lock().unwrap() = Some(messages.to_vec());
            Ok("Response.".to_string())
        }
    }

    /// Returns a different response for each call, simulating multi-turn
    /// tool-use conversations.
    struct MultiTurnLlm {
        responses: Vec<String>,
        call_index: AtomicUsize,
    }

    impl MultiTurnLlm {
        fn new(responses: Vec<&str>) -> Self {
            Self {
                responses: responses.into_iter().map(String::from).collect(),
                call_index: AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.call_index.load(Ordering::Relaxed)
        }
    }

    impl LlmClient for MultiTurnLlm {
        async fn complete_messages(
            &self,
            _messages: &[llm::Message],
            _stop: &[&str],
        ) -> Result<String, llm::LlmError> {
            let i = self.call_index.fetch_add(1, Ordering::Relaxed);
            Ok(self.responses[i].clone())
        }
    }

    /// Stub browser executor for tests that don't use browser features.
    struct NoBrowser;
    impl RunBrowser for NoBrowser {
        fn run_browser(&self, _input: &str) -> String {
            unreachable!("browser should not be called in this test")
        }
    }

    /// Convenience: no browser available.
    const NO_BROWSER: Option<&NoBrowser> = None;

    fn create_file(dir: &std::path::Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
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
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

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
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

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
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

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
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        let directives = parser::parse_directives(&content);
        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].status, parser::DirectiveStatus::Complete);
        assert_eq!(directives[1].status, parser::DirectiveStatus::Complete);
    }

    #[tokio::test]
    async fn process_file__should_pass_document_content_to_llm() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        let file_content = "# My Essay\n\nThe sky is blue.\n\n@magent summarize this document\n";
        std::fs::write(&path, file_content).unwrap();
        let client = MessageCaptureLlm::new();

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        let captured = client.captured_messages.lock().unwrap();
        let messages = captured.as_ref().expect("messages should be passed to LLM");
        let system_content = &messages[0].content;
        assert!(
            system_content.contains("# My Essay"),
            "document context should contain the file heading"
        );
        assert!(
            system_content.contains("The sky is blue."),
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
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

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
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

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
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

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

    #[tokio::test]
    async fn process_file__should_apply_accepted_edits() {
        // Given — file with an accepted edit (simulates user accepting a proposal)
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(
            &path,
            "\
# Links

- [Rust](htps://rust-lang.org)

@magent fix the URL

<magent-response>
Fixed:
<magent-edit status=\"accepted\">
<magent-search>htps://rust-lang.org</magent-search>
<magent-replace>https://rust-lang.org</magent-replace>
</magent-edit>
</magent-response>
",
        )
        .unwrap();
        let client = SpyLlm {
            response: "Should not be called".to_string(),
            call_count: AtomicUsize::new(0),
        };

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then — edit applied, LLM not called
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("- [Rust](https://rust-lang.org)"),
            "document should have the corrected URL"
        );
        assert!(
            content.contains("status=\"applied\""),
            "status should be updated to applied"
        );
        assert!(
            !content.contains("status=\"accepted\""),
            "no accepted statuses should remain"
        );
        assert_eq!(
            client.call_count.load(Ordering::Relaxed),
            0,
            "LLM should not be called when processing accepted edits"
        );
    }

    #[tokio::test]
    async fn process_file__full_propose_accept_apply_lifecycle() {
        // Given — start with a document and directive
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

        // Step 1: Process directive — should propose edits
        process_file(&path, &client, dir.path(), NO_BROWSER).await;
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("status=\"proposed\""));
        assert!(
            content.contains("- [Rust](htps://rust-lang.org)"),
            "document unchanged after proposal"
        );

        // Step 2: Simulate user accepting the edit
        let accepted = content.replace("status=\"proposed\"", "status=\"accepted\"");
        std::fs::write(&path, &accepted).unwrap();

        // Step 3: Process acceptance — should apply edits
        process_file(&path, &client, dir.path(), NO_BROWSER).await;
        let final_content = std::fs::read_to_string(&path).unwrap();
        assert!(
            final_content.contains("- [Rust](https://rust-lang.org)"),
            "document should have the corrected URL"
        );
        assert!(
            final_content.contains("status=\"applied\""),
            "status should be applied"
        );
        assert!(
            !final_content.contains("status=\"accepted\""),
            "no accepted statuses should remain"
        );
    }

    #[tokio::test]
    async fn process_file__should_pass_referenced_files_to_llm() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("rust.md"), "# Rust\nOwnership rules.\n").unwrap();
        let path = dir.path().join("main.md");
        std::fs::write(&path, "@magent(context: rust.md) compare error handling\n").unwrap();
        let client = MessageCaptureLlm::new();

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        let captured = client.captured_messages.lock().unwrap();
        let messages = captured.as_ref().expect("messages should be passed to LLM");
        let system_content = &messages[0].content;
        assert!(
            system_content.contains("=== CURRENT DOCUMENT: main.md ==="),
            "should label the current document"
        );
        assert!(
            system_content.contains("=== REFERENCED: rust.md ==="),
            "should label the referenced file"
        );
        assert!(
            system_content.contains("Ownership rules."),
            "should include referenced file content"
        );
    }

    #[tokio::test]
    async fn process_file__should_not_add_headers_when_no_context_option() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "# Doc\n\n@magent summarize\n").unwrap();
        let client = MessageCaptureLlm::new();

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then — system message should be plain content, no headers
        let captured = client.captured_messages.lock().unwrap();
        let messages = captured.as_ref().unwrap();
        let system_content = &messages[0].content;
        assert!(
            !system_content.contains("=== CURRENT DOCUMENT"),
            "should not add headers when no context references"
        );
        assert!(system_content.contains("# Doc"));
    }

    #[tokio::test]
    async fn process_file__should_write_error_for_missing_context_file() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent(context: nonexistent.md) summarize\n").unwrap();
        let client = FakeLlm("Should not be called.".to_string());

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<magent-response>"));
        assert!(content.contains("**Error:**"));
        assert!(content.contains("nonexistent.md"));
    }

    #[tokio::test]
    async fn process_file__should_write_error_for_path_traversal() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        // Create a file outside root so the path resolves but is rejected
        let outside = dir.path().join("../outside.md");
        std::fs::write(&outside, "secret").unwrap();
        std::fs::write(&path, "@magent(context: ../outside.md) summarize\n").unwrap();
        let client = FakeLlm("Should not be called.".to_string());

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<magent-response>"));
        assert!(content.contains("**Error:**"));
        assert!(content.contains("outside the knowledge base"));

        // Cleanup
        let _ = std::fs::remove_file(&outside);
    }

    // --- Tool use integration tests ---

    #[tokio::test]
    async fn process_file__should_execute_search_tool_and_write_result() {
        // Given: a knowledge base with a searchable file
        let dir = tempfile::tempdir().unwrap();
        create_file(
            dir.path(),
            "notes/rust.md",
            "Rust uses Result for error handling.",
        );
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent what have I written about error handling?\n").unwrap();

        // LLM turn 1: calls search (without closing tag, simulating stop sequence)
        // LLM turn 2: synthesizes final response
        let client = MultiTurnLlm::new(vec![
            "<magent-tool-call tool=\"search\">\n\
             <magent-input>error handling</magent-input>\n",
            "Based on your notes, Rust uses Result for error handling.",
        ]);

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<magent-response>"));
        // Tool call history is visible
        assert!(content.contains("<magent-tool-call"));
        assert!(content.contains("<magent-tool-result"));
        // Search results are embedded
        assert!(content.contains("notes/rust.md"));
        // Final response is present
        assert!(content.contains("Rust uses Result for error handling"));
        assert!(content.contains("</magent-response>"));
        // LLM was called exactly twice
        assert_eq!(client.call_count(), 2);
    }

    #[tokio::test]
    async fn process_file__should_execute_read_tool_and_write_result() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        create_file(dir.path(), "notes/rust.md", "# Rust\nOwnership rules.\n");
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent read the rust notes\n").unwrap();

        let client = MultiTurnLlm::new(vec![
            "<magent-tool-call tool=\"read\">\n\
             <magent-input>notes/rust.md</magent-input>\n",
            "The Rust notes cover ownership rules.",
        ]);

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<magent-tool-result tool=\"read\">"));
        assert!(content.contains("# Rust"));
        assert!(content.contains("Ownership rules."));
        assert!(content.contains("The Rust notes cover ownership rules."));
    }

    #[tokio::test]
    async fn process_file__should_handle_multiple_tool_calls_in_sequence() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        create_file(
            dir.path(),
            "notes/rust.md",
            "Rust uses Result for errors.\nMore details here.",
        );
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent find and read error handling notes\n").unwrap();

        // Turn 1: search, Turn 2: read, Turn 3: final response
        let client = MultiTurnLlm::new(vec![
            "<magent-tool-call tool=\"search\">\n\
             <magent-input>error</magent-input>\n",
            "<magent-tool-call tool=\"read\">\n\
             <magent-input>notes/rust.md</magent-input>\n",
            "Your notes cover Result-based error handling in Rust.",
        ]);

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<magent-tool-result tool=\"search\">"));
        assert!(content.contains("<magent-tool-result tool=\"read\">"));
        assert!(content.contains("Result-based error handling"));
        assert_eq!(client.call_count(), 3);
    }

    #[tokio::test]
    async fn process_file__should_enforce_tool_call_limit() {
        // Given: LLM always calls a tool (would loop forever without limit)
        let dir = tempfile::tempdir().unwrap();
        create_file(dir.path(), "notes.md", "content");
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent do something\n").unwrap();

        let always_tool_call = "<magent-tool-call tool=\"search\">\n\
                                <magent-input>query</magent-input>\n";
        let client = MultiTurnLlm::new(vec![always_tool_call; MAX_TOOL_CALLS + 1]);

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("(Tool call limit reached.)"));
        // Should have called LLM exactly MAX_TOOL_CALLS times (once per tool call)
        assert_eq!(client.call_count(), MAX_TOOL_CALLS);
    }

    #[tokio::test]
    async fn process_file__should_handle_unknown_tool() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent do something\n").unwrap();

        let client = MultiTurnLlm::new(vec![
            "<magent-tool-call tool=\"unknown_tool\">\n\
             <magent-input>some input</magent-input>\n",
            "I couldn't use that tool, but here's my response.",
        ]);

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Unknown tool: unknown_tool"));
        assert!(content.contains("I couldn't use that tool"));
    }

    #[tokio::test]
    async fn process_file__should_still_work_without_tool_calls() {
        // Given: LLM responds with plain text, no tool calls
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent what is Rust?\n").unwrap();
        let client = FakeLlm("Rust is a systems programming language.".to_string());

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then: works exactly as before
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Rust is a systems programming language."));
        assert!(!content.contains("magent-tool-call"));
        assert!(!content.contains("magent-tool-result"));
    }

    #[tokio::test]
    async fn process_file__should_handle_tool_call_with_closing_tag_present() {
        // Given: API doesn't strip stop sequence (closing tag is present)
        let dir = tempfile::tempdir().unwrap();
        create_file(dir.path(), "notes.md", "some content");
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent search notes\n").unwrap();

        let client = MultiTurnLlm::new(vec![
            "<magent-tool-call tool=\"search\">\n\
             <magent-input>content</magent-input>\n\
             </magent-tool-call>",
            "Found it.",
        ]);

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<magent-tool-result tool=\"search\">"));
        assert!(content.contains("Found it."));
    }

    // --- Browser tool integration tests ---

    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct FakeBrowser {
        responses: Mutex<VecDeque<(&'static str, String)>>,
    }

    impl FakeBrowser {
        fn new(responses: Vec<(&'static str, &str)>) -> Self {
            Self {
                responses: Mutex::new(
                    responses
                        .into_iter()
                        .map(|(cmd, resp)| (cmd, resp.to_string()))
                        .collect(),
                ),
            }
        }
    }

    impl RunBrowser for FakeBrowser {
        fn run_browser(&self, input: &str) -> String {
            let (expected, response) = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("unexpected browser call: {input}"));
            assert!(
                input.starts_with(expected),
                "expected command starting with '{expected}', got '{input}'"
            );
            response
        }
    }

    #[tokio::test]
    async fn process_file__should_execute_browser_tool() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent check the page at https://example.com\n").unwrap();

        let browser = Some(FakeBrowser::new(vec![
            (
                "open https://example.com",
                "Navigated to https://example.com",
            ),
            (
                "snapshot",
                "- heading \"Example Domain\" [level=1, ref=e1]\n  - StaticText \"Example Domain\"\n- paragraph\n  - StaticText \"This domain is for use in illustrative examples.\"\n- paragraph\n  - link \"More information\" [ref=e2]",
            ),
        ]));

        let client = MultiTurnLlm::new(vec![
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>open https://example.com</magent-input>\n",
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>snapshot</magent-input>\n",
            "The page is the Example Domain placeholder page with a link to more information.",
        ]);

        // When
        process_file(&path, &client, dir.path(), browser.as_ref()).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<magent-tool-result tool=\"browser\">"));
        assert!(content.contains("Navigated to https://example.com"));
        assert!(content.contains("Example Domain"));
        assert!(content.contains("Example Domain placeholder page"));
    }

    #[tokio::test]
    async fn process_file__should_handle_browser_interaction_flow() {
        // Given: open → snapshot → click → snapshot → respond
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(
            &path,
            "@magent click the login button on https://example.com\n",
        )
        .unwrap();

        let browser = Some(FakeBrowser::new(vec![
            (
                "open https://example.com",
                "Navigated to https://example.com",
            ),
            (
                "snapshot",
                "- main\n  - button \"Login\" [ref=e3]\n    - StaticText \"Login\"",
            ),
            ("click @e3", "Clicked @e3"),
            (
                "snapshot",
                "- main\n  - textbox \"Username\" [ref=e5]\n  - textbox \"Password\" [ref=e6]",
            ),
        ]));

        let client = MultiTurnLlm::new(vec![
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>open https://example.com</magent-input>\n",
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>snapshot</magent-input>\n",
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>click @e3</magent-input>\n",
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>snapshot</magent-input>\n",
            "I clicked the Login button and the login form is now showing with Username and Password fields.",
        ]);

        // When
        process_file(&path, &client, dir.path(), browser.as_ref()).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Clicked @e3"));
        assert!(content.contains("Username"));
        assert!(content.contains("login form is now showing"));
        assert_eq!(client.call_count(), 5);
    }

    #[tokio::test]
    async fn process_file__should_mix_browser_and_search_tools() {
        // Given: search knowledge base, then browse a URL
        let dir = tempfile::tempdir().unwrap();
        create_file(dir.path(), "links.md", "Project page: https://example.com");
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent find the project URL and check it\n").unwrap();

        let browser = Some(FakeBrowser::new(vec![
            (
                "open https://example.com",
                "Navigated to https://example.com",
            ),
            (
                "snapshot",
                "- main\n  - StaticText \"Welcome to the project\"",
            ),
        ]));

        let client = MultiTurnLlm::new(vec![
            "<magent-tool-call tool=\"search\">\n\
             <magent-input>project URL</magent-input>\n",
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>open https://example.com</magent-input>\n",
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>snapshot</magent-input>\n",
            "Found the project URL in links.md and confirmed the page says \"Welcome to the project\".",
        ]);

        // When
        process_file(&path, &client, dir.path(), browser.as_ref()).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<magent-tool-result tool=\"search\">"));
        assert!(content.contains("<magent-tool-result tool=\"browser\">"));
        assert!(content.contains("Welcome to the project"));
    }

    #[tokio::test]
    async fn process_file__should_return_error_when_browser_unavailable() {
        // Given: browser is None but LLM tries to use it
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent browse something\n").unwrap();

        let client = MultiTurnLlm::new(vec![
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>open https://example.com</magent-input>\n",
            "The browser tool is not available.",
        ]);

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Error: browser tool is not available"));
    }

    #[tokio::test]
    async fn process_file__should_include_browser_docs_in_system_prompt_when_available() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent hello\n").unwrap();

        let browser = Some(FakeBrowser::new(vec![]));
        let client = MessageCaptureLlm::new();

        // When
        process_file(&path, &client, dir.path(), browser.as_ref()).await;

        // Then
        let captured = client.captured_messages.lock().unwrap();
        let messages = captured.as_ref().unwrap();
        let system_content = &messages[0].content;
        assert!(
            system_content.contains("## browser"),
            "system prompt should include browser tool docs when browser is available"
        );
    }

    #[tokio::test]
    async fn process_file__should_exclude_browser_docs_when_unavailable() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent hello\n").unwrap();
        let client = MessageCaptureLlm::new();

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        let captured = client.captured_messages.lock().unwrap();
        let messages = captured.as_ref().unwrap();
        let system_content = &messages[0].content;
        assert!(
            !system_content.contains("## browser"),
            "system prompt should not include browser tool docs when browser is unavailable"
        );
    }

    // --- Realistic browser fixture tests ---
    //
    // These tests use accessibility tree snapshots that match the actual
    // agent-browser output format. They serve as both integration tests and
    // documentation of the expected snapshot structure.

    /// Realistic snapshot of a GitHub pull request page.
    /// Format matches actual agent-browser v0.19 output.
    const GITHUB_PR_SNAPSHOT: &str = "\
- generic
  - link \"Skip to content\" [ref=e1]
    - StaticText \"Skip to content\"
  - banner
    - navigation \"Global\"
      - link \"acme/webapp\" [ref=e3]
        - StaticText \"acme/webapp\"
- main
  - heading \"Add retry logic to API client #42\" [level=1, ref=e5]
    - StaticText \"Add retry logic to API client #42\"
  - StaticText \"Open — user wants to merge 3 commits into main from retry-logic\"
  - navigation \"Pull request tabs\"
    - tab \"Conversation\" [selected=true, ref=e8]
    - tab \"Commits 3\" [ref=e9]
    - tab \"Files changed 4\" [ref=e10]
  - generic
    - article
      - link \"alice\" [ref=e14]
        - StaticText \"alice\"
      - StaticText \"commented 2 days ago\"
      - paragraph
        - StaticText \"The backoff multiplier should be configurable rather than hardcoded to 2. Consider adding a parameter to RetryConfig.\"
    - article
      - link \"bob\" [ref=e18]
        - StaticText \"bob\"
      - StaticText \"commented 1 day ago\"
      - paragraph
        - StaticText \"Can we add a test for the timeout case? The current tests only cover success and immediate failure.\"
    - article
      - link \"alice\" [ref=e22]
        - StaticText \"alice\"
      - StaticText \"commented 3 hours ago\"
      - paragraph
        - StaticText \"Also, the jitter calculation on line 47 can overflow for large retry counts. Use saturating_mul instead.\"
  - generic
    - textbox \"Leave a comment\" [ref=e26]
    - button \"Comment\" [ref=e27]";

    /// Realistic snapshot of a search engine results page.
    const SEARCH_RESULTS_SNAPSHOT: &str = "\
- main
  - generic
    - article
      - link \"Error Handling in Rust - A Deep Dive\" [ref=e3]
        - StaticText \"Error Handling in Rust - A Deep Dive\"
      - StaticText \"blog.rust-lang.org\"
      - paragraph
        - StaticText \"A comprehensive guide to using Result, Option, and the ? operator for robust error handling...\"
    - article
      - link \"Rust By Example: Error handling\" [ref=e5]
        - StaticText \"Rust By Example: Error handling\"
      - StaticText \"doc.rust-lang.org\"
      - paragraph
        - StaticText \"This section covers the various ways to handle errors in Rust, including unwrap, expect, and custom error types...\"
    - article
      - link \"Best practices for error handling in Rust 2024\" [ref=e7]
        - StaticText \"Best practices for error handling in Rust 2024\"
      - StaticText \"medium.com\"
      - paragraph
        - StaticText \"Updated patterns for error handling using thiserror, anyhow, and the new features in Rust 1.75...\"
  - navigation \"Pagination\"
    - link \"Next\" [ref=e10]";

    #[tokio::test]
    async fn process_file__github_pr_review_flow() {
        // Full realistic flow: open PR → snapshot → read comments → respond with summary
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(
            &path,
            "@magent summarize the review comments on https://github.com/acme/webapp/pull/42\n",
        )
        .unwrap();

        let browser = Some(FakeBrowser::new(vec![
            (
                "open https://github.com/acme/webapp/pull/42",
                "Navigated to https://github.com/acme/webapp/pull/42",
            ),
            ("snapshot", GITHUB_PR_SNAPSHOT),
        ]));

        let client = MultiTurnLlm::new(vec![
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>open https://github.com/acme/webapp/pull/42</magent-input>\n",
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>snapshot</magent-input>\n",
            "There are 3 open review comments on PR #42:\n\n\
             1. **alice** (2 days ago): The backoff multiplier should be configurable.\n\
             2. **bob** (1 day ago): Requesting a test for the timeout case.\n\
             3. **alice** (3 hours ago): Jitter calculation can overflow — use saturating_mul.",
        ]);

        // When
        process_file(&path, &client, dir.path(), browser.as_ref()).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        // Snapshot was passed through to LLM context
        assert!(content.contains("backoff multiplier should be configurable"));
        assert!(content.contains("saturating_mul"));
        // Final response is present
        assert!(content.contains("3 open review comments"));
        assert!(content.contains("alice"));
        assert!(content.contains("bob"));
        assert_eq!(client.call_count(), 3);
    }

    #[tokio::test]
    async fn process_file__search_and_click_flow() {
        // Realistic flow: open search → snapshot → fill query → press Enter →
        // snapshot results → click link → snapshot article → respond
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(
            &path,
            "@magent search for rust error handling best practices and summarize the top result\n",
        )
        .unwrap();

        let article_snapshot = "\
- main
  - article
    - heading \"Error Handling in Rust\" [level=1, ref=e1]
      - StaticText \"Error Handling in Rust\"
    - paragraph
      - StaticText \"Rust's approach to error handling is one of its most distinctive features. \
Unlike exceptions in other languages, Rust uses the type system to encode the possibility of failure.\"
    - heading \"The Result Type\" [level=2, ref=e2]
      - StaticText \"The Result Type\"
    - paragraph
      - StaticText \"The Result<T, E> enum is the primary mechanism for recoverable errors. \
It forces callers to explicitly handle both success and failure cases.\"
    - heading \"The ? Operator\" [level=2, ref=e3]
      - StaticText \"The ? Operator\"
    - paragraph
      - StaticText \"The question mark operator provides ergonomic error propagation, \
converting and returning errors automatically.\"";

        let browser = Some(FakeBrowser::new(vec![
            (
                "open https://search.example.com",
                "Navigated to https://search.example.com",
            ),
            (
                "snapshot",
                "- main\n  - textbox \"Search\" [focused, ref=e1]\n  - button \"Search\" [ref=e2]",
            ),
            ("fill @e1 rust error handling best practices", "Filled @e1"),
            ("press Enter", "Pressed Enter"),
            ("snapshot", SEARCH_RESULTS_SNAPSHOT),
            ("click @e3", "Clicked @e3"),
            ("snapshot", article_snapshot),
        ]));

        let client = MultiTurnLlm::new(vec![
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>open https://search.example.com</magent-input>\n",
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>snapshot</magent-input>\n",
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>fill @e1 rust error handling best practices</magent-input>\n",
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>press Enter</magent-input>\n",
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>snapshot</magent-input>\n",
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>click @e3</magent-input>\n",
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>snapshot</magent-input>\n",
            "The top result is \"Error Handling in Rust - A Deep Dive\" from blog.rust-lang.org. \
Key points: Rust uses the type system (Result<T, E>) rather than exceptions, and the ? operator \
provides ergonomic error propagation.",
        ]);

        // When
        process_file(&path, &client, dir.path(), browser.as_ref()).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        // All tool calls/results are visible in the response
        assert!(content.contains("<magent-tool-result tool=\"browser\">"));
        assert!(content.contains("Filled @e1"));
        assert!(content.contains("Pressed Enter"));
        // Search results snapshot was passed through
        assert!(content.contains("Error Handling in Rust - A Deep Dive"));
        // Article content was passed through
        assert!(content.contains("Result<T, E>"));
        assert!(content.contains("question mark operator"));
        // Final synthesis is present
        assert!(content.contains("ergonomic error propagation"));
        // 7 browser calls + 1 final response = 8 LLM turns
        assert_eq!(client.call_count(), 8);
    }

    #[tokio::test]
    async fn process_file__knowledge_base_search_then_browse() {
        // Realistic mixed-tool flow: search knowledge base for a URL,
        // then browse it and synthesize information from both sources
        let dir = tempfile::tempdir().unwrap();
        create_file(
            dir.path(),
            "projects/webapp.md",
            "# Webapp\n\n\
             Status: in progress\n\
             CI dashboard: https://ci.example.com/acme/webapp\n\n\
             The webapp uses React + TypeScript.\n",
        );
        let path = dir.path().join("test.md");
        std::fs::write(
            &path,
            "@magent check the CI status for the webapp project\n",
        )
        .unwrap();

        let ci_snapshot = "\
- main
  - heading \"acme/webapp\" [level=1, ref=e1]
    - StaticText \"acme/webapp\"
  - table \"Recent builds\"
    - row
      - cell
        - StaticText \"#847\"
      - cell [ref=e3]
        - StaticText \"main\"
      - cell
        - StaticText \"passed\"
      - cell
        - StaticText \"2 min ago\"
    - row
      - cell
        - StaticText \"#846\"
      - cell [ref=e5]
        - StaticText \"retry-logic\"
      - cell
        - StaticText \"failed\"
      - cell
        - StaticText \"15 min ago\"
    - row
      - cell
        - StaticText \"#845\"
      - cell [ref=e7]
        - StaticText \"main\"
      - cell
        - StaticText \"passed\"
      - cell
        - StaticText \"1 hour ago\"";

        let browser = Some(FakeBrowser::new(vec![
            (
                "open https://ci.example.com/acme/webapp",
                "Navigated to https://ci.example.com/acme/webapp",
            ),
            ("snapshot", ci_snapshot),
        ]));

        let client = MultiTurnLlm::new(vec![
            // Turn 1: search knowledge base
            "<magent-tool-call tool=\"search\">\n\
             <magent-input>CI dashboard webapp</magent-input>\n",
            // Turn 2: read the file for the URL
            "<magent-tool-call tool=\"read\">\n\
             <magent-input>projects/webapp.md</magent-input>\n",
            // Turn 3: open CI dashboard
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>open https://ci.example.com/acme/webapp</magent-input>\n",
            // Turn 4: snapshot
            "<magent-tool-call tool=\"browser\">\n\
             <magent-input>snapshot</magent-input>\n",
            // Turn 5: synthesize
            "The CI dashboard for webapp shows:\n\n\
             - **#847** (main): passed, 2 min ago\n\
             - **#846** (retry-logic): **failed**, 15 min ago\n\
             - **#845** (main): passed, 1 hour ago\n\n\
             The `retry-logic` branch build is failing.",
        ]);

        // When
        process_file(&path, &client, dir.path(), browser.as_ref()).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        // Knowledge base tools were used
        assert!(content.contains("<magent-tool-result tool=\"search\">"));
        assert!(content.contains("<magent-tool-result tool=\"read\">"));
        // Browser tools were used
        assert!(content.contains("<magent-tool-result tool=\"browser\">"));
        // CI dashboard data was passed through
        assert!(content.contains("retry-logic"));
        assert!(content.contains("failed"));
        // Final response synthesizes everything
        assert!(content.contains("retry-logic"));
        assert!(content.contains("branch build is failing"));
        assert_eq!(client.call_count(), 5);
    }

    #[tokio::test]
    async fn process_file__snapshot_passed_to_llm_context() {
        // Verify that the full snapshot content is fed back to the LLM
        // as a user message (tool result), preserving the tree structure
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(
            &path,
            "@magent read https://github.com/acme/webapp/pull/42\n",
        )
        .unwrap();

        let browser = Some(FakeBrowser::new(vec![
            (
                "open https://github.com",
                "Navigated to https://github.com/acme/webapp/pull/42",
            ),
            ("snapshot", GITHUB_PR_SNAPSHOT),
        ]));

        // Use MessageCaptureLlm-like approach: capture what the LLM sees after the snapshot
        let captured_messages: std::sync::Arc<Mutex<Vec<Vec<llm::Message>>>> =
            std::sync::Arc::new(Mutex::new(Vec::new()));

        struct CapturingMultiTurnLlm {
            responses: Vec<String>,
            call_index: AtomicUsize,
            captured: std::sync::Arc<Mutex<Vec<Vec<llm::Message>>>>,
        }

        impl LlmClient for CapturingMultiTurnLlm {
            async fn complete_messages(
                &self,
                messages: &[llm::Message],
                _stop: &[&str],
            ) -> Result<String, llm::LlmError> {
                self.captured.lock().unwrap().push(messages.to_vec());
                let i = self.call_index.fetch_add(1, Ordering::Relaxed);
                Ok(self.responses[i].clone())
            }
        }

        let client = CapturingMultiTurnLlm {
            responses: vec![
                "<magent-tool-call tool=\"browser\">\n\
                 <magent-input>open https://github.com/acme/webapp/pull/42</magent-input>\n"
                    .to_string(),
                "<magent-tool-call tool=\"browser\">\n\
                 <magent-input>snapshot</magent-input>\n"
                    .to_string(),
                "Summary of the PR.".to_string(),
            ],
            call_index: AtomicUsize::new(0),
            captured: captured_messages.clone(),
        };

        // When
        process_file(&path, &client, dir.path(), browser.as_ref()).await;

        // Then: the third LLM call should have the snapshot in its messages
        let all_calls = captured_messages.lock().unwrap();
        assert_eq!(all_calls.len(), 3);

        // The last call's messages should include the snapshot as a tool result
        let last_call_messages = &all_calls[2];
        let snapshot_in_context = last_call_messages
            .iter()
            .any(|m| m.role == "user" && m.content.contains("backoff multiplier"));
        assert!(
            snapshot_in_context,
            "snapshot content should be passed to the LLM as a user message"
        );

        // Verify the full tree structure is preserved
        let snapshot_msg = last_call_messages
            .iter()
            .find(|m| m.content.contains("Add retry logic to API client #42"))
            .expect("should find message containing snapshot");
        assert!(snapshot_msg.content.contains("link \"alice\" [ref=e14]"));
        assert!(snapshot_msg.content.contains("link \"bob\" [ref=e18]"));
        assert!(snapshot_msg.content.contains("saturating_mul"));
    }

    // --- Incremental writing tests ---

    #[tokio::test]
    async fn process_file__should_resume_in_progress_directive() {
        // Given: a file with an in-progress response containing a completed tool call
        let dir = tempfile::tempdir().unwrap();
        create_file(
            dir.path(),
            "notes/rust.md",
            "Rust uses Result for error handling.",
        );
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent what about error handling?\n").unwrap();

        // Set up in-progress state as the writer would produce it
        let partial = "\
<magent-tool-call tool=\"search\">
<magent-input>error handling</magent-input>
</magent-tool-call>
<magent-tool-result tool=\"search\">
notes/rust.md: Rust uses Result for error handling.
</magent-tool-result>
";
        writer::write_response_block(&path, "what about error handling?", partial, true).unwrap();

        // LLM should produce the final response (no more tool calls)
        let client = FakeLlm("Based on your notes, Rust uses Result.".to_string());

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("<magent-response>"),
            "response should be complete (no status attribute)"
        );
        assert!(
            !content.contains("in-progress"),
            "in-progress status should be removed"
        );
        assert!(
            content.contains("<magent-tool-call"),
            "tool call history should be preserved"
        );
        assert!(
            content.contains("<magent-tool-result"),
            "tool result history should be preserved"
        );
        assert!(
            content.contains("Based on your notes"),
            "final response should be present"
        );
    }

    #[tokio::test]
    async fn process_file__should_skip_paused_directive() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(
            &path,
            "@magent do something\n\n\
             <magent-response status=\"paused\">\n\
             partial content\n\
             </magent-response>\n",
        )
        .unwrap();
        let client = SpyLlm {
            response: "Should not be called".to_string(),
            call_count: AtomicUsize::new(0),
        };

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then
        assert_eq!(
            client.call_count.load(Ordering::Relaxed),
            0,
            "LLM should not be called for paused directives"
        );
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("status=\"paused\""),
            "paused status should be preserved"
        );
    }

    #[tokio::test]
    async fn process_file__should_reconstruct_messages_when_resuming() {
        // Given: a file with an in-progress response containing a tool call + result
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent find errors\n").unwrap();
        let partial = "\
<magent-tool-call tool=\"search\">
<magent-input>errors</magent-input>
</magent-tool-call>
<magent-tool-result tool=\"search\">
Found 3 results.
</magent-tool-result>
";
        writer::write_response_block(&path, "find errors", partial, true).unwrap();

        let client = MessageCaptureLlm::new();

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then: LLM should receive reconstructed messages
        let captured = client.captured_messages.lock().unwrap();
        let messages = captured.as_ref().expect("LLM should have been called");
        // system + user prompt + assistant (tool call) + user (tool result) = 4 messages
        assert_eq!(
            messages.len(),
            4,
            "should have 4 messages: system, user, assistant, user"
        );
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
        assert!(messages[1].content.contains("find errors"));
        assert_eq!(messages[2].role, "assistant");
        assert!(messages[2].content.contains("magent-tool-call"));
        assert_eq!(messages[3].role, "user");
        assert!(messages[3].content.contains("Found 3 results"));
    }

    #[tokio::test]
    async fn process_file__should_write_in_progress_during_tool_calls() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        create_file(dir.path(), "notes.md", "some content");
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent search notes\n").unwrap();

        // LLM that captures file state at each call
        struct FileInspectingLlm {
            path: PathBuf,
            responses: Vec<String>,
            call_index: AtomicUsize,
            snapshots: std::sync::Mutex<Vec<String>>,
        }

        impl LlmClient for FileInspectingLlm {
            async fn complete_messages(
                &self,
                _messages: &[llm::Message],
                _stop: &[&str],
            ) -> Result<String, llm::LlmError> {
                let i = self.call_index.fetch_add(1, Ordering::Relaxed);
                let content = std::fs::read_to_string(&self.path).unwrap();
                self.snapshots.lock().unwrap().push(content);
                Ok(self.responses[i].clone())
            }
        }

        let client = FileInspectingLlm {
            path: path.clone(),
            responses: vec![
                "<magent-tool-call tool=\"search\">\n\
                 <magent-input>content</magent-input>\n"
                    .to_string(),
                "Found it.".to_string(),
            ],
            call_index: AtomicUsize::new(0),
            snapshots: std::sync::Mutex::new(Vec::new()),
        };

        // When
        process_file(&path, &client, dir.path(), NO_BROWSER).await;

        // Then: first snapshot should show in-progress status
        let snapshots = client.snapshots.lock().unwrap();
        assert_eq!(snapshots.len(), 2);
        assert!(
            snapshots[0].contains("status=\"in-progress\""),
            "file should have in-progress status before first LLM call"
        );
        assert!(
            snapshots[1].contains("status=\"in-progress\""),
            "file should have in-progress status before second LLM call"
        );
        // Second snapshot should also contain the tool call and result
        assert!(
            snapshots[1].contains("<magent-tool-call"),
            "file should contain tool call history"
        );
        assert!(
            snapshots[1].contains("<magent-tool-result"),
            "file should contain tool result"
        );

        // Final file should be complete (no in-progress)
        let final_content = std::fs::read_to_string(&path).unwrap();
        assert!(
            !final_content.contains("in-progress"),
            "final response should not have in-progress status"
        );
        assert!(final_content.contains("Found it."));
    }

    #[tokio::test]
    async fn process_directive__should_stop_when_no_response_block() {
        // Given: file with directive but no response block (simulates user deletion)
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "@magent do something\n").unwrap();

        let client = SpyLlm {
            response: "Should not be called".to_string(),
            call_count: AtomicUsize::new(0),
        };

        // When
        process_directive(
            &client,
            "do something",
            "doc",
            dir.path(),
            NO_BROWSER,
            &path,
        )
        .await;

        // Then: LLM should not be called (no response block to continue)
        assert_eq!(
            client.call_count.load(Ordering::Relaxed),
            0,
            "LLM should not be called when response block is missing"
        );
    }

    #[test]
    fn complete_tool_call_tag__should_append_tag_when_missing() {
        let response = "<magent-tool-call tool=\"search\">\n\
                        <magent-input>query</magent-input>\n";
        let completed = complete_tool_call_tag(response);
        assert!(completed.ends_with("</magent-tool-call>"));
    }

    #[test]
    fn complete_tool_call_tag__should_not_append_when_already_present() {
        let response = "<magent-tool-call tool=\"search\">\n\
                        <magent-input>query</magent-input>\n\
                        </magent-tool-call>";
        let completed = complete_tool_call_tag(response);
        assert_eq!(completed, response);
    }

    #[test]
    fn complete_tool_call_tag__should_not_append_for_plain_text() {
        let response = "Just a regular response with no tool calls.";
        let completed = complete_tool_call_tag(response);
        assert_eq!(completed, response);
    }

    // --- scan_in_progress ---

    #[test]
    fn scan_in_progress__should_find_files_with_in_progress_responses() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        create_file(
            dir.path(),
            "active.md",
            "@magent do something\n\n<magent-response status=\"in-progress\">\nWorking...\n</magent-response>\n",
        );
        create_file(
            dir.path(),
            "done.md",
            "@magent do something\n\n<magent-response>\nDone.\n</magent-response>\n",
        );

        // When
        let paths = scan_in_progress(dir.path());

        // Then
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].file_name().unwrap(), "active.md");
    }

    #[test]
    fn scan_in_progress__should_return_empty_when_no_in_progress() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        create_file(
            dir.path(),
            "done.md",
            "@magent hello\n\n<magent-response>\nHi!\n</magent-response>\n",
        );
        create_file(dir.path(), "unprocessed.md", "@magent hello\n");

        // When
        let paths = scan_in_progress(dir.path());

        // Then
        assert!(paths.is_empty());
    }

    #[test]
    fn scan_in_progress__should_find_files_in_subdirectories() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        create_file(
            dir.path(),
            "sub/nested.md",
            "@magent task\n\n<magent-response status=\"in-progress\">\nPartial.\n</magent-response>\n",
        );

        // When
        let paths = scan_in_progress(dir.path());

        // Then
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].file_name().unwrap(), "nested.md");
    }

    #[test]
    fn scan_in_progress__should_ignore_non_markdown_files() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        create_file(
            dir.path(),
            "notes.txt",
            "@magent task\n\n<magent-response status=\"in-progress\">\nWorking.\n</magent-response>\n",
        );

        // When
        let paths = scan_in_progress(dir.path());

        // Then
        assert!(paths.is_empty());
    }

    #[test]
    fn scan_in_progress__should_skip_paused_responses() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        create_file(
            dir.path(),
            "paused.md",
            "@magent task\n\n<magent-response status=\"paused\">\nPaused.\n</magent-response>\n",
        );

        // When
        let paths = scan_in_progress(dir.path());

        // Then
        assert!(paths.is_empty());
    }

    #[test]
    fn scan_in_progress__should_handle_multiple_in_progress_files() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        create_file(
            dir.path(),
            "a.md",
            "@magent task a\n\n<magent-response status=\"in-progress\">\nA.\n</magent-response>\n",
        );
        create_file(
            dir.path(),
            "b.md",
            "@magent task b\n\n<magent-response status=\"in-progress\">\nB.\n</magent-response>\n",
        );
        create_file(
            dir.path(),
            "c.md",
            "@magent task c\n\n<magent-response>\nDone.\n</magent-response>\n",
        );

        // When
        let paths = scan_in_progress(dir.path());

        // Then
        assert_eq!(paths.len(), 2);
        let names: Vec<_> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(names.contains(&"a.md"));
        assert!(names.contains(&"b.md"));
    }
}
