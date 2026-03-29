use std::fmt;
use std::future::Future;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// System prompt template used when document context is provided.
///
/// The `{document}` placeholder is replaced with the actual document content.
const SYSTEM_PROMPT_TEMPLATE: &str = "\
You are an AI assistant embedded in a markdown document. The user will ask \
questions or request changes to the document below. Additional referenced \
files may follow the main document — use them as context but only propose \
edits to the main document.

Before responding, think through your approach inside <magent-thinking> tags:

<magent-thinking>
Your reasoning here — what the user is asking, what needs to change, etc.
</magent-thinking>

Then provide your response or edit blocks after the thinking.

When making changes to the document, output your edits using this format:

<magent-edit>
<magent-search>exact text to find</magent-search>
<magent-replace>replacement text</magent-replace>
</magent-edit>

For multiline edits, put the content on its own lines:

<magent-edit>
<magent-search>
- first item
- second item
</magent-search>
<magent-replace>
- second item
- first item
</magent-replace>
</magent-edit>

Leading and trailing whitespace inside search/replace tags is ignored.

You may include multiple edit blocks. The search text must match the document \
exactly (character for character). Include enough surrounding context in the \
search text to uniquely identify the location.

You may include plain text before, after, or between edit blocks to explain \
what you changed.

When answering questions (no document edits needed), respond with plain text \
as usual — do not use edit blocks. Be concise and reference the document directly.

Respond in the same language as the document unless asked otherwise.

=== DOCUMENT ===
{document}
=== END DOCUMENT ===

=== TOOLS ===

You have access to tools for gathering information. To use a tool, output:

<magent-tool-call tool=\"tool_name\">
<magent-input>your input here</magent-input>
</magent-tool-call>

After a tool call, stop and wait. The system will provide the result in a \
<magent-tool-result> block, then you continue your response.

You may call multiple tools in sequence. Do not guess tool results — always \
call the tool and use the actual result.

Available tools:

## search
Search for text across markdown files in the knowledge base.
Input: a search query (plain text or regex). Optional prefixes: path:subdir/, max:N
Returns: matching lines with file paths and line numbers.

## read
Read the full content of a file in the knowledge base.
Input: a relative file path, optionally followed by a line range (e.g. notes/rust.md 40-60)
Returns: the file content with line numbers.

{browser_tool}\
=== END TOOLS ===";

/// A chat message with role and content.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
        }
    }
}

/// Trait for LLM completion.
pub trait LlmClient {
    fn complete_messages(
        &self,
        messages: &[Message],
        stop: &[&str],
    ) -> impl Future<Output = Result<String, LlmError>> + Send;
}

/// Error type for LLM operations.
#[derive(Debug)]
pub enum LlmError {
    /// Failed to connect to the API (e.g., connection refused).
    Connection(String),
    /// The API returned an HTTP error status.
    Api { status: u16, body: String },
    /// The response body could not be parsed.
    Parse(String),
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::Connection(msg) => write!(f, "Connection error: {msg}"),
            LlmError::Api { status, body } => write!(f, "API error ({status}): {body}"),
            LlmError::Parse(msg) => write!(f, "Parse error: {msg}"),
        }
    }
}

impl std::error::Error for LlmError {}

/// OpenAI-compatible chat completions client.
pub struct ChatClient {
    http: Client,
    api_url: String,
    model: String,
    api_key: Option<String>,
}

impl ChatClient {
    pub fn new(api_url: String, model: String, api_key: Option<String>) -> Self {
        Self {
            http: Client::new(),
            api_url,
            model,
            api_key,
        }
    }
}

impl LlmClient for ChatClient {
    async fn complete_messages(
        &self,
        messages: &[Message],
        stop: &[&str],
    ) -> Result<String, LlmError> {
        let url = format!("{}/chat/completions", self.api_url.trim_end_matches('/'));

        let api_messages: Vec<ApiMessage> = messages
            .iter()
            .map(|m| ApiMessage {
                role: &m.role,
                content: &m.content,
            })
            .collect();

        let body = ChatRequest {
            model: &self.model,
            messages: api_messages,
            stop: stop.to_vec(),
        };

        debug!(url = %url, model = %self.model, messages = messages.len(), "LLM request");

        let mut req = self.http.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let response = req
            .send()
            .await
            .map_err(|e| LlmError::Connection(format!("{e} ({url})")))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            debug!(status = status.as_u16(), "LLM request failed");
            return Err(LlmError::Api {
                status: status.as_u16(),
                body,
            });
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .map_err(|e| LlmError::Parse(e.to_string()))?;

        let result = chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| LlmError::Parse("No choices in response".to_string()));

        if let Ok(ref content) = result {
            debug!(len = content.len(), "LLM response OK");
        }

        result
    }
}

const BROWSER_TOOL_DOCS: &str = "\
## browser
Interact with web pages using a headless browser.
Input: a browser command. One command per call.

Key commands:
- open <url> — navigate to a URL (starts browser if needed)
- snapshot — get page content as an accessibility tree with element refs ([ref=e1], [ref=e2], ...)
- click @<ref> — click an element (e.g. click @e3)
- type @<ref> <text> — type text into an element
- fill @<ref> <text> — clear and fill an input field
- select @<ref> <value> — select a dropdown option
- press <key> — press a key (Enter, Tab, Escape, etc.)
- scroll <direction> — scroll the page (up, down, left, right)
- wait @<ref> — wait for an element to appear
- get text @<ref> — get text content of an element
- get title — get page title
- get url — get current URL
- screenshot <file> — save a screenshot
- back — go back
- close — close browser

Typical workflow: open URL → snapshot → read/interact → snapshot again → respond.
After open, always snapshot first to see the page content before interacting.

";

pub fn build_system_prompt(document: &str, browser_available: bool) -> String {
    let browser_section = if browser_available {
        BROWSER_TOOL_DOCS
    } else {
        ""
    };
    SYSTEM_PROMPT_TEMPLATE
        .replace("{browser_tool}", browser_section)
        .replace("{document}", document)
}

// -- Request/response types for OpenAI-compatible API --

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ApiMessage<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop: Vec<&'a str>,
}

#[derive(Serialize)]
struct ApiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use wiremock::matchers::{bearer_token, body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_client(base_url: &str, api_key: Option<&str>) -> ChatClient {
        ChatClient::new(
            base_url.to_string(),
            "test-model".to_string(),
            api_key.map(String::from),
        )
    }

    fn success_response(content: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": content
                }
            }]
        })
    }

    #[tokio::test]
    async fn complete_messages__should_return_response_text() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_response("Hello back!")))
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);
        let messages = vec![Message::user("Hello")];

        // When
        let result = client.complete_messages(&messages, &[]).await;

        // Then
        assert_eq!(result.unwrap(), "Hello back!");
    }

    #[tokio::test]
    async fn complete_messages__should_send_model_and_messages_in_request() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "model": "test-model",
                "messages": [{ "role": "user", "content": "What is Rust?" }]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_response("A language.")))
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);
        let messages = vec![Message::user("What is Rust?")];

        // When
        let result = client.complete_messages(&messages, &[]).await;

        // Then
        assert_eq!(result.unwrap(), "A language.");
    }

    #[tokio::test]
    async fn complete_messages__should_send_bearer_token_when_api_key_is_set() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(bearer_token("secret-key-123"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(success_response("Authenticated!")),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), Some("secret-key-123"));
        let messages = vec![Message::user("Hello")];

        // When
        let result = client.complete_messages(&messages, &[]).await;

        // Then
        assert_eq!(result.unwrap(), "Authenticated!");
    }

    #[tokio::test]
    async fn complete_messages__should_not_send_auth_header_when_no_api_key() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(success_response("No auth needed.")),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);
        let messages = vec![Message::user("Hello")];

        // When
        let result = client.complete_messages(&messages, &[]).await;

        // Then
        assert_eq!(result.unwrap(), "No auth needed.");
        let requests = server.received_requests().await.unwrap();
        assert!(
            !requests[0].headers.contains_key("Authorization"),
            "Authorization header should not be present when no API key is set"
        );
    }

    #[tokio::test]
    async fn complete_messages__should_return_api_error_on_4xx() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string("Unauthorized: invalid API key"),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);
        let messages = vec![Message::user("Hello")];

        // When
        let result = client.complete_messages(&messages, &[]).await;

        // Then
        let err = result.unwrap_err();
        assert!(matches!(err, LlmError::Api { status: 401, .. }));
        assert!(err.to_string().contains("Unauthorized"));
    }

    #[tokio::test]
    async fn complete_messages__should_return_api_error_on_5xx() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal server error"))
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);
        let messages = vec![Message::user("Hello")];

        // When
        let result = client.complete_messages(&messages, &[]).await;

        // Then
        let err = result.unwrap_err();
        assert!(matches!(err, LlmError::Api { status: 500, .. }));
    }

    #[tokio::test]
    async fn complete_messages__should_return_connection_error_when_server_unreachable() {
        // Given — point to a port where nothing is listening
        let client = test_client("http://127.0.0.1:1", None);
        let messages = vec![Message::user("Hello")];

        // When
        let result = client.complete_messages(&messages, &[]).await;

        // Then
        let err = result.unwrap_err();
        assert!(matches!(err, LlmError::Connection(_)));
        assert!(err.to_string().contains("127.0.0.1:1"));
    }

    #[tokio::test]
    async fn complete_messages__should_return_parse_error_on_malformed_json() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);
        let messages = vec![Message::user("Hello")];

        // When
        let result = client.complete_messages(&messages, &[]).await;

        // Then
        assert!(matches!(result.unwrap_err(), LlmError::Parse(_)));
    }

    #[tokio::test]
    async fn complete_messages__should_return_parse_error_when_no_choices() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "choices": [] })),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);
        let messages = vec![Message::user("Hello")];

        // When
        let result = client.complete_messages(&messages, &[]).await;

        // Then
        let err = result.unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
        assert!(err.to_string().contains("No choices"));
    }

    #[tokio::test]
    async fn complete_messages__should_send_system_and_user_messages() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "messages": [
                    { "role": "system" },
                    { "role": "user", "content": "summarize this" }
                ]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_response("A summary.")))
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);
        let messages = vec![
            Message::system(build_system_prompt("# My Document\n\nSome content.", false)),
            Message::user("summarize this"),
        ];

        // When
        let result = client.complete_messages(&messages, &[]).await;

        // Then
        assert_eq!(result.unwrap(), "A summary.");
    }

    #[tokio::test]
    async fn complete_messages__should_send_multi_turn_conversation() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "messages": [
                    { "role": "system", "content": "You are helpful." },
                    { "role": "user", "content": "Hello" },
                    { "role": "assistant", "content": "Hi there!" },
                    { "role": "user", "content": "Search result here" }
                ]
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(success_response("Based on results...")),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);
        let messages = vec![
            Message::system("You are helpful."),
            Message::user("Hello"),
            Message::assistant("Hi there!"),
            Message::user("Search result here"),
        ];

        // When
        let result = client.complete_messages(&messages, &[]).await;

        // Then
        assert_eq!(result.unwrap(), "Based on results...");
    }

    #[tokio::test]
    async fn complete_messages__should_send_stop_sequences() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "stop": ["</magent-tool-call>"]
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(success_response("I'll search...")),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);
        let messages = vec![Message::user("search for errors")];

        // When
        let result = client
            .complete_messages(&messages, &["</magent-tool-call>"])
            .await;

        // Then
        assert_eq!(result.unwrap(), "I'll search...");
    }

    #[tokio::test]
    async fn complete_messages__should_omit_stop_when_empty() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_response("Hi!")))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);
        let messages = vec![Message::user("Hello")];

        // When
        client.complete_messages(&messages, &[]).await.unwrap();

        // Then — verify "stop" key is not in the request body
        let requests = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(
            body.get("stop").is_none(),
            "stop field should be omitted when empty"
        );
    }

    #[tokio::test]
    async fn complete_messages__should_include_document_in_system_message() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_response("Done.")))
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);
        let messages = vec![
            Message::system(build_system_prompt(
                "# Shopping List\n\n- milk\n- eggs\n",
                false,
            )),
            Message::user("edit this"),
        ];

        // When
        client.complete_messages(&messages, &[]).await.unwrap();

        // Then — verify the system message contains the document
        let requests = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        let api_messages = body["messages"].as_array().unwrap();
        assert_eq!(api_messages.len(), 2);
        let system_content = api_messages[0]["content"].as_str().unwrap();
        assert!(
            system_content.contains("# Shopping List"),
            "system message should contain the document content"
        );
        assert!(
            system_content.contains("=== DOCUMENT ==="),
            "system message should use the document delimiter"
        );
    }

    #[test]
    fn build_system_prompt__should_insert_document_content() {
        // When
        let result = build_system_prompt("# Hello\n\nWorld.", false);

        // Then
        assert!(result.contains("# Hello\n\nWorld."));
        assert!(result.contains("=== DOCUMENT ==="));
        assert!(result.contains("=== END DOCUMENT ==="));
    }

    #[test]
    fn build_system_prompt__should_include_thinking_instruction() {
        // When
        let result = build_system_prompt("doc", false);

        // Then
        assert!(
            result.contains("<magent-thinking>"),
            "system prompt should instruct the model to use thinking tags"
        );
    }

    #[test]
    fn build_system_prompt__should_include_browser_tool_when_available() {
        // When
        let result = build_system_prompt("doc", true);

        // Then
        assert!(
            result.contains("## browser"),
            "should include browser tool section"
        );
        assert!(
            result.contains("snapshot"),
            "should include snapshot command"
        );
        assert!(result.contains("open <url>"), "should include open command");
    }

    #[test]
    fn build_system_prompt__should_exclude_browser_tool_when_unavailable() {
        // When
        let result = build_system_prompt("doc", false);

        // Then
        assert!(
            !result.contains("## browser"),
            "should not include browser tool section"
        );
    }

    /// Integration test that talks to a real LLM API.
    ///
    /// Ignored by default — run it explicitly with:
    ///
    /// ```sh
    /// cargo nextest run complete_messages__should_get_response_from_real_api --run-ignored only
    /// ```
    ///
    /// ## Setup (Ollama)
    ///
    /// 1. Install Ollama: <https://ollama.com/download>
    /// 2. Pull a small model: `ollama pull smollm2:135m`
    /// 3. Ollama serves on `http://localhost:11434` by default — no further config needed.
    ///
    /// ## Overriding defaults
    ///
    /// Set environment variables to point at a different API:
    ///
    /// ```sh
    /// MAGENT_TEST_API_URL=http://localhost:11434/v1 \
    /// MAGENT_TEST_MODEL=smollm2:135m \
    /// MAGENT_API_KEY=sk-... \
    ///   cargo nextest run complete_messages__should_get_response_from_real_api --run-ignored only
    /// ```
    #[tokio::test]
    #[ignore]
    async fn complete_messages__should_get_response_from_real_api() {
        let api_url = std::env::var("MAGENT_TEST_API_URL")
            .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
        let model =
            std::env::var("MAGENT_TEST_MODEL").unwrap_or_else(|_| "smollm2:135m".to_string());
        let api_key = std::env::var("MAGENT_API_KEY").ok();

        let client = ChatClient::new(api_url, model, api_key);
        let messages = vec![Message::user("Reply with exactly: hello")];

        let result = client.complete_messages(&messages, &[]).await;

        let response = result.expect("LLM API call should succeed");
        assert!(!response.is_empty(), "Response should not be empty");
        println!("LLM response: {response}");
    }
}
