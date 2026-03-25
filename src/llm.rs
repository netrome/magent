use std::fmt;
use std::future::Future;

use reqwest::Client;
use serde::{Deserialize, Serialize};

/// System prompt template used when document context is provided.
///
/// The `{document}` placeholder is replaced with the actual document content.
const SYSTEM_PROMPT_TEMPLATE: &str = "\
You are an AI assistant embedded in a markdown document. The user will ask \
questions or request changes to the document below.

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
=== END DOCUMENT ===";

/// Trait for LLM completion.
pub trait LlmClient {
    fn complete(
        &self,
        prompt: &str,
        document: Option<&str>,
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
    async fn complete(&self, prompt: &str, document: Option<&str>) -> Result<String, LlmError> {
        let url = format!("{}/chat/completions", self.api_url.trim_end_matches('/'));

        let system_prompt = document.map(build_system_prompt);
        let mut messages = Vec::new();
        if let Some(ref system) = system_prompt {
            messages.push(Message {
                role: "system",
                content: system,
            });
        }
        messages.push(Message {
            role: "user",
            content: prompt,
        });

        let body = ChatRequest {
            model: &self.model,
            messages,
        };

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
            return Err(LlmError::Api {
                status: status.as_u16(),
                body,
            });
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .map_err(|e| LlmError::Parse(e.to_string()))?;

        chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| LlmError::Parse("No choices in response".to_string()))
    }
}

fn build_system_prompt(document: &str) -> String {
    SYSTEM_PROMPT_TEMPLATE.replace("{document}", document)
}

// -- Request/response types for OpenAI-compatible API --

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
}

#[derive(Serialize)]
struct Message<'a> {
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
    async fn complete__should_return_response_text() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_response("Hello back!")))
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);

        // When
        let result = client.complete("Hello", None).await;

        // Then
        assert_eq!(result.unwrap(), "Hello back!");
    }

    #[tokio::test]
    async fn complete__should_send_model_and_prompt_in_request() {
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

        // When
        let result = client.complete("What is Rust?", None).await;

        // Then
        assert_eq!(result.unwrap(), "A language.");
    }

    #[tokio::test]
    async fn complete__should_send_bearer_token_when_api_key_is_set() {
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

        // When
        let result = client.complete("Hello", None).await;

        // Then
        assert_eq!(result.unwrap(), "Authenticated!");
    }

    #[tokio::test]
    async fn complete__should_not_send_auth_header_when_no_api_key() {
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

        // When
        let result = client.complete("Hello", None).await;

        // Then
        assert_eq!(result.unwrap(), "No auth needed.");
        let requests = server.received_requests().await.unwrap();
        assert!(
            !requests[0].headers.contains_key("Authorization"),
            "Authorization header should not be present when no API key is set"
        );
    }

    #[tokio::test]
    async fn complete__should_return_api_error_on_4xx() {
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

        // When
        let result = client.complete("Hello", None).await;

        // Then
        let err = result.unwrap_err();
        assert!(matches!(err, LlmError::Api { status: 401, .. }));
        assert!(err.to_string().contains("Unauthorized"));
    }

    #[tokio::test]
    async fn complete__should_return_api_error_on_5xx() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal server error"))
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);

        // When
        let result = client.complete("Hello", None).await;

        // Then
        let err = result.unwrap_err();
        assert!(matches!(err, LlmError::Api { status: 500, .. }));
    }

    #[tokio::test]
    async fn complete__should_return_connection_error_when_server_unreachable() {
        // Given — point to a port where nothing is listening
        let client = test_client("http://127.0.0.1:1", None);

        // When
        let result = client.complete("Hello", None).await;

        // Then
        let err = result.unwrap_err();
        assert!(matches!(err, LlmError::Connection(_)));
        assert!(err.to_string().contains("127.0.0.1:1"));
    }

    #[tokio::test]
    async fn complete__should_return_parse_error_on_malformed_json() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);

        // When
        let result = client.complete("Hello", None).await;

        // Then
        assert!(matches!(result.unwrap_err(), LlmError::Parse(_)));
    }

    #[tokio::test]
    async fn complete__should_return_parse_error_when_no_choices() {
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

        // When
        let result = client.complete("Hello", None).await;

        // Then
        let err = result.unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
        assert!(err.to_string().contains("No choices"));
    }

    #[tokio::test]
    async fn complete__should_send_system_message_when_document_provided() {
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

        // When
        let result = client
            .complete("summarize this", Some("# My Document\n\nSome content."))
            .await;

        // Then
        assert_eq!(result.unwrap(), "A summary.");
    }

    #[tokio::test]
    async fn complete__should_not_send_system_message_when_no_document() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "messages": [
                    { "role": "user", "content": "Hello" }
                ]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_response("Hi!")))
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);

        // When
        let result = client.complete("Hello", None).await;

        // Then
        assert_eq!(result.unwrap(), "Hi!");
        // Verify only one message was sent (no system message)
        let requests = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1, "should only have the user message");
        assert_eq!(messages[0]["role"], "user");
    }

    #[tokio::test]
    async fn complete__should_include_document_content_in_system_message() {
        // Given
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_response("Done.")))
            .mount(&server)
            .await;

        let client = test_client(&server.uri(), None);

        // When
        client
            .complete("edit this", Some("# Shopping List\n\n- milk\n- eggs\n"))
            .await
            .unwrap();

        // Then — verify the system message contains the document
        let requests = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        let system_content = messages[0]["content"].as_str().unwrap();
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
        let result = build_system_prompt("# Hello\n\nWorld.");

        // Then
        assert!(result.contains("# Hello\n\nWorld."));
        assert!(result.contains("=== DOCUMENT ==="));
        assert!(result.contains("=== END DOCUMENT ==="));
    }

    /// Integration test that talks to a real LLM API.
    ///
    /// Ignored by default — run it explicitly with:
    ///
    /// ```sh
    /// cargo nextest run complete__should_get_response_from_real_api --run-ignored only
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
    ///   cargo nextest run complete__should_get_response_from_real_api --run-ignored only
    /// ```
    #[tokio::test]
    #[ignore]
    async fn complete__should_get_response_from_real_api() {
        let api_url = std::env::var("MAGENT_TEST_API_URL")
            .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
        let model =
            std::env::var("MAGENT_TEST_MODEL").unwrap_or_else(|_| "smollm2:135m".to_string());
        let api_key = std::env::var("MAGENT_API_KEY").ok();

        let client = ChatClient::new(api_url, model, api_key);

        let result = client.complete("Reply with exactly: hello", None).await;

        let response = result.expect("LLM API call should succeed");
        assert!(!response.is_empty(), "Response should not be empty");
        println!("LLM response: {response}");
    }
}
