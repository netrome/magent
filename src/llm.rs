use std::fmt;
use std::future::Future;

use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Trait for LLM completion.
pub trait LlmClient {
    fn complete(&self, prompt: &str) -> impl Future<Output = Result<String, LlmError>> + Send;
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
    async fn complete(&self, prompt: &str) -> Result<String, LlmError> {
        let url = format!("{}/chat/completions", self.api_url.trim_end_matches('/'));

        let body = ChatRequest {
            model: &self.model,
            messages: vec![Message {
                role: "user",
                content: prompt,
            }],
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
        let result = client.complete("Hello").await;

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
        let result = client.complete("What is Rust?").await;

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
        let result = client.complete("Hello").await;

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
        let result = client.complete("Hello").await;

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
        let result = client.complete("Hello").await;

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
        let result = client.complete("Hello").await;

        // Then
        let err = result.unwrap_err();
        assert!(matches!(err, LlmError::Api { status: 500, .. }));
    }

    #[tokio::test]
    async fn complete__should_return_connection_error_when_server_unreachable() {
        // Given — point to a port where nothing is listening
        let client = test_client("http://127.0.0.1:1", None);

        // When
        let result = client.complete("Hello").await;

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
        let result = client.complete("Hello").await;

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
        let result = client.complete("Hello").await;

        // Then
        let err = result.unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
        assert!(err.to_string().contains("No choices"));
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

        let result = client.complete("Reply with exactly: hello").await;

        let response = result.expect("LLM API call should succeed");
        assert!(!response.is_empty(), "Response should not be empty");
        println!("LLM response: {response}");
    }
}
