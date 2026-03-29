use crate::llm;

/// A tool call parsed from an LLM response.
#[derive(Debug, PartialEq)]
pub struct ToolCall {
    pub tool: String,
    pub input: String,
    pub status: Option<ToolStatus>,
}

/// Approval status for gated tool calls.
#[derive(Debug, PartialEq)]
pub enum ToolStatus {
    Proposed,
    Accepted,
    Executed,
}

/// A tool execution result, ready for injection into the conversation.
#[derive(Debug, PartialEq)]
pub struct ToolResult {
    pub tool: String,
    pub output: String,
}

/// Parse the first tool call from an LLM response.
///
/// Returns the parsed call (if any) and the text before it. Designed for the
/// one-call-per-turn stop-sequence flow where the LLM emits at most one tool
/// call before generation halts.
///
/// Malformed tool call tags are treated as plain text (returns None).
pub fn parse_tool_call(response: &str) -> (Option<ToolCall>, String) {
    let (calls, text) = parse_tool_calls(response);
    let first = calls.into_iter().next();
    (first, text)
}

/// Parse all tool calls from an LLM response.
///
/// Returns the calls and the non-tool-call text (concatenation of text
/// segments between/around tool call blocks). Malformed blocks are skipped
/// and their content is included in the text portion.
pub fn parse_tool_calls(response: &str) -> (Vec<ToolCall>, String) {
    let blocks = find_tool_call_blocks(response);
    if blocks.is_empty() {
        return (vec![], response.to_string());
    }

    let mut calls = Vec::with_capacity(blocks.len());
    let mut text = String::new();
    let mut cursor = 0;

    for block in &blocks {
        // Text before this block
        if block.start > cursor {
            text.push_str(&response[cursor..block.start]);
        }

        if let Some(call) = parse_block(block) {
            calls.push(call);
        } else {
            // Malformed block — include raw text
            text.push_str(&response[block.start..block.end]);
        }

        cursor = block.end;
    }

    // Text after the last block
    if cursor < response.len() {
        text.push_str(&response[cursor..]);
    }

    let text = text.trim().to_string();
    (calls, text)
}

/// Format a tool result as a `<magent-tool-result>` block.
pub fn format_tool_result(result: &ToolResult) -> String {
    format!(
        "<magent-tool-result tool=\"{}\">\n{}\n</magent-tool-result>",
        result.tool, result.output,
    )
}

/// Parse a `<magent-tool-result>` block, returning the tool name and output.
///
/// Returns `None` if the string doesn't contain a valid tool result block.
pub fn parse_tool_result(text: &str) -> Option<ToolResult> {
    let open_start = text.find(RESULT_OPEN_TAG)?;
    let gt = text[open_start..].find('>')? + open_start;
    let open_tag = &text[open_start..=gt];
    let tool = extract_attribute(open_tag, "tool")?;
    if tool.is_empty() {
        return None;
    }
    let content_start = gt + 1;
    let close_start = text[content_start..].find(RESULT_CLOSE_TAG)? + content_start;
    let output = text[content_start..close_start].trim().to_string();
    Some(ToolResult { tool, output })
}

/// Reconstruct the LLM message history from response content.
///
/// Parses the response text (from a `<magent-response>` block) into the
/// alternating assistant/user message sequence the LLM originally saw.
/// The returned messages are prepended with the system and user prompt.
///
/// Each tool-result block forms a turn boundary: text before it (including
/// tool-call blocks) becomes an assistant message, the tool-result block
/// itself becomes a user message. Trailing text after the last tool-result
/// is not included (it represents the most recent LLM output).
pub fn reconstruct_messages(
    system_prompt: &str,
    user_prompt: &str,
    response_content: &str,
) -> Vec<llm::Message> {
    let mut messages = vec![
        llm::Message::system(system_prompt),
        llm::Message::user(user_prompt),
    ];

    let blocks = find_tool_result_blocks(response_content);
    if blocks.is_empty() {
        return messages;
    }

    let mut cursor = 0;
    for block in &blocks {
        let assistant_text = response_content[cursor..block.start].trim_end();
        if !assistant_text.is_empty() {
            messages.push(llm::Message::assistant(assistant_text));
        }
        let user_text = &response_content[block.start..block.end];
        messages.push(llm::Message::user(user_text));
        cursor = block.end;
        // Skip the newline separator that process_directive adds after each result
        if response_content[cursor..].starts_with('\n') {
            cursor += 1;
        }
    }

    messages
}

// --- Private helpers ---

const OPEN_TAG: &str = "<magent-tool-call";
const CLOSE_TAG: &str = "</magent-tool-call>";
const INPUT_OPEN: &str = "<magent-input>";
const INPUT_CLOSE: &str = "</magent-input>";

/// Byte range of a raw tool call block in the source string.
struct RawBlock {
    start: usize,
    end: usize,
    /// The opening tag including attributes, e.g. `<magent-tool-call tool="search">`
    open_tag: String,
    /// Content between the opening tag's `>` and `</magent-tool-call>`
    inner: String,
}

fn find_tool_call_blocks(response: &str) -> Vec<RawBlock> {
    let mut blocks = Vec::new();
    let mut search_from = 0;

    while let Some(pos) = response[search_from..].find(OPEN_TAG) {
        let open_start = search_from + pos;

        // Find the end of the opening tag (the `>`)
        let Some(gt) = response[open_start..].find('>') else {
            break;
        };
        let open_end = open_start + gt + 1;

        let open_tag = response[open_start..open_end].to_string();

        // Find the closing tag
        let Some(ct) = response[open_end..].find(CLOSE_TAG) else {
            break;
        };
        let close_start = open_end + ct;

        let block_end = close_start + CLOSE_TAG.len();
        let inner = response[open_end..close_start].to_string();

        blocks.push(RawBlock {
            start: open_start,
            end: block_end,
            open_tag,
            inner,
        });

        search_from = block_end;
    }

    blocks
}

fn parse_block(block: &RawBlock) -> Option<ToolCall> {
    let tool = extract_attribute(&block.open_tag, "tool")?;
    if tool.is_empty() {
        return None;
    }

    let input = extract_input(&block.inner)?;
    let status = extract_attribute(&block.open_tag, "status").and_then(|s| parse_status(&s));

    Some(ToolCall {
        tool,
        input,
        status,
    })
}

fn extract_input(inner: &str) -> Option<String> {
    let start = inner.find(INPUT_OPEN)? + INPUT_OPEN.len();
    let end = inner[start..].find(INPUT_CLOSE)? + start;
    Some(inner[start..end].trim().to_string())
}

fn extract_attribute(tag: &str, name: &str) -> Option<String> {
    // Match: name="value" or name='value'
    let pattern = format!("{name}=\"");
    if let Some(start) = tag.find(&pattern) {
        let value_start = start + pattern.len();
        let value_end = tag[value_start..].find('"')? + value_start;
        return Some(tag[value_start..value_end].to_string());
    }

    let pattern = format!("{name}='");
    if let Some(start) = tag.find(&pattern) {
        let value_start = start + pattern.len();
        let value_end = tag[value_start..].find('\'')? + value_start;
        return Some(tag[value_start..value_end].to_string());
    }

    None
}

fn parse_status(s: &str) -> Option<ToolStatus> {
    match s {
        "proposed" => Some(ToolStatus::Proposed),
        "accepted" => Some(ToolStatus::Accepted),
        "executed" => Some(ToolStatus::Executed),
        _ => None,
    }
}

const RESULT_OPEN_TAG: &str = "<magent-tool-result";
const RESULT_CLOSE_TAG: &str = "</magent-tool-result>";

/// Byte range of a raw tool result block in the source string.
struct RawResultBlock {
    start: usize,
    end: usize,
}

fn find_tool_result_blocks(text: &str) -> Vec<RawResultBlock> {
    let mut blocks = Vec::new();
    let mut search_from = 0;

    while let Some(pos) = text[search_from..].find(RESULT_OPEN_TAG) {
        let open_start = search_from + pos;

        let Some(gt) = text[open_start..].find('>') else {
            break;
        };

        let content_start = open_start + gt + 1;

        let Some(ct) = text[content_start..].find(RESULT_CLOSE_TAG) else {
            break;
        };
        let block_end = content_start + ct + RESULT_CLOSE_TAG.len();

        blocks.push(RawResultBlock {
            start: open_start,
            end: block_end,
        });

        search_from = block_end;
    }

    blocks
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    // --- parse_tool_call ---

    #[test]
    fn parse_tool_call__should_extract_single_tool_call() {
        // Given
        let response = concat!(
            "<magent-tool-call tool=\"search\">\n",
            "<magent-input>error handling</magent-input>\n",
            "</magent-tool-call>",
        );

        // When
        let (call, _text) = parse_tool_call(response);

        // Then
        let call = call.expect("should parse a tool call");
        assert_eq!(call.tool, "search");
        assert_eq!(call.input, "error handling");
        assert_eq!(call.status, None);
    }

    #[test]
    fn parse_tool_call__should_return_none_when_no_tool_call() {
        // Given
        let response = "The sky is blue due to Rayleigh scattering.";

        // When
        let (call, text) = parse_tool_call(response);

        // Then
        assert!(call.is_none());
        assert_eq!(text, response);
    }

    #[test]
    fn parse_tool_call__should_return_text_before_tool_call() {
        // Given
        let response = concat!(
            "Let me search for that.\n",
            "<magent-tool-call tool=\"search\">\n",
            "<magent-input>error handling</magent-input>\n",
            "</magent-tool-call>",
        );

        // When
        let (call, text) = parse_tool_call(response);

        // Then
        assert!(call.is_some());
        assert_eq!(text, "Let me search for that.");
    }

    #[test]
    fn parse_tool_call__should_extract_tool_name_from_attribute() {
        // Given
        let response = concat!(
            "<magent-tool-call tool=\"read\">\n",
            "<magent-input>notes/rust.md</magent-input>\n",
            "</magent-tool-call>",
        );

        // When
        let (call, _text) = parse_tool_call(response);

        // Then
        assert_eq!(call.unwrap().tool, "read");
    }

    #[test]
    fn parse_tool_call__should_trim_input_whitespace() {
        // Given — model puts input on its own line with surrounding whitespace
        let response = concat!(
            "<magent-tool-call tool=\"search\">\n",
            "<magent-input>\n",
            "  error handling\n",
            "</magent-input>\n",
            "</magent-tool-call>",
        );

        // When
        let (call, _text) = parse_tool_call(response);

        // Then
        assert_eq!(call.unwrap().input, "error handling");
    }

    #[test]
    fn parse_tool_call__should_treat_missing_tool_attribute_as_plain_text() {
        // Given — no tool attribute
        let response = concat!(
            "<magent-tool-call>\n",
            "<magent-input>query</magent-input>\n",
            "</magent-tool-call>",
        );

        // When
        let (call, text) = parse_tool_call(response);

        // Then
        assert!(call.is_none());
        assert_eq!(text, response);
    }

    #[test]
    fn parse_tool_call__should_treat_missing_input_tag_as_plain_text() {
        // Given — no magent-input inside
        let response = concat!(
            "<magent-tool-call tool=\"search\">\n",
            "error handling\n",
            "</magent-tool-call>",
        );

        // When
        let (call, text) = parse_tool_call(response);

        // Then
        assert!(call.is_none());
        assert_eq!(text, response);
    }

    #[test]
    fn parse_tool_call__should_treat_unclosed_tag_as_plain_text() {
        // Given — no closing tag (e.g. response truncated)
        let response = concat!(
            "Let me search.\n",
            "<magent-tool-call tool=\"search\">\n",
            "<magent-input>query</magent-input>\n",
        );

        // When
        let (call, text) = parse_tool_call(response);

        // Then
        assert!(call.is_none());
        assert_eq!(text, response);
    }

    #[test]
    fn parse_tool_call__should_parse_status_attribute() {
        // Given
        let response = concat!(
            "<magent-tool-call tool=\"web_fetch\" status=\"proposed\">\n",
            "<magent-input>https://example.com</magent-input>\n",
            "</magent-tool-call>",
        );

        // When
        let (call, _text) = parse_tool_call(response);

        // Then
        let call = call.unwrap();
        assert_eq!(call.tool, "web_fetch");
        assert_eq!(call.status, Some(ToolStatus::Proposed));
    }

    #[test]
    fn parse_tool_call__should_ignore_unknown_status() {
        // Given
        let response = concat!(
            "<magent-tool-call tool=\"search\" status=\"unknown\">\n",
            "<magent-input>query</magent-input>\n",
            "</magent-tool-call>",
        );

        // When
        let (call, _text) = parse_tool_call(response);

        // Then
        let call = call.unwrap();
        assert_eq!(call.status, None);
    }

    #[test]
    fn parse_tool_call__should_return_only_the_first_call() {
        // Given — two tool calls in one response
        let response = concat!(
            "<magent-tool-call tool=\"search\">\n",
            "<magent-input>first</magent-input>\n",
            "</magent-tool-call>\n",
            "<magent-tool-call tool=\"read\">\n",
            "<magent-input>second</magent-input>\n",
            "</magent-tool-call>",
        );

        // When
        let (call, _text) = parse_tool_call(response);

        // Then
        let call = call.unwrap();
        assert_eq!(call.tool, "search");
        assert_eq!(call.input, "first");
    }

    // --- parse_tool_calls ---

    #[test]
    fn parse_tool_calls__should_extract_multiple_tool_calls() {
        // Given
        let response = concat!(
            "<magent-tool-call tool=\"search\">\n",
            "<magent-input>error handling</magent-input>\n",
            "</magent-tool-call>\n",
            "<magent-tool-call tool=\"read\">\n",
            "<magent-input>notes/rust.md</magent-input>\n",
            "</magent-tool-call>",
        );

        // When
        let (calls, _text) = parse_tool_calls(response);

        // Then
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].tool, "search");
        assert_eq!(calls[0].input, "error handling");
        assert_eq!(calls[1].tool, "read");
        assert_eq!(calls[1].input, "notes/rust.md");
    }

    #[test]
    fn parse_tool_calls__should_return_non_tool_text() {
        // Given
        let response = concat!(
            "Let me search.\n",
            "<magent-tool-call tool=\"search\">\n",
            "<magent-input>query</magent-input>\n",
            "</magent-tool-call>\n",
            "And also read a file.\n",
            "<magent-tool-call tool=\"read\">\n",
            "<magent-input>notes/rust.md</magent-input>\n",
            "</magent-tool-call>\n",
            "Done.",
        );

        // When
        let (calls, text) = parse_tool_calls(response);

        // Then
        assert_eq!(calls.len(), 2);
        assert!(text.contains("Let me search."));
        assert!(text.contains("And also read a file."));
        assert!(text.contains("Done."));
    }

    #[test]
    fn parse_tool_calls__should_skip_malformed_and_keep_valid() {
        // Given — first block missing input, second is valid
        let response = concat!(
            "<magent-tool-call tool=\"search\">\n",
            "no input tag here\n",
            "</magent-tool-call>\n",
            "<magent-tool-call tool=\"read\">\n",
            "<magent-input>notes/rust.md</magent-input>\n",
            "</magent-tool-call>",
        );

        // When
        let (calls, text) = parse_tool_calls(response);

        // Then
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool, "read");
        // Malformed block content appears in text
        assert!(text.contains("no input tag here"));
    }

    // --- format_tool_result ---

    #[test]
    fn format_tool_result__should_produce_valid_xml_block() {
        // Given
        let result = ToolResult {
            tool: "search".to_string(),
            output: "3 matches across 2 files:\n\nrust.md:45: error handling".to_string(),
        };

        // When
        let formatted = format_tool_result(&result);

        // Then
        assert_eq!(
            formatted,
            concat!(
                "<magent-tool-result tool=\"search\">\n",
                "3 matches across 2 files:\n",
                "\n",
                "rust.md:45: error handling\n",
                "</magent-tool-result>",
            )
        );
    }

    #[test]
    fn format_tool_result__should_include_tool_name_in_attribute() {
        // Given
        let result = ToolResult {
            tool: "read".to_string(),
            output: "file content".to_string(),
        };

        // When
        let formatted = format_tool_result(&result);

        // Then
        assert!(formatted.starts_with("<magent-tool-result tool=\"read\">"));
        assert!(formatted.ends_with("</magent-tool-result>"));
    }

    // --- parse_tool_result ---

    #[test]
    fn parse_tool_result__should_extract_tool_name_and_output() {
        // Given
        let text = concat!(
            "<magent-tool-result tool=\"search\">\n",
            "3 matches found\n",
            "</magent-tool-result>",
        );

        // When
        let result = parse_tool_result(text);

        // Then
        let result = result.expect("should parse a tool result");
        assert_eq!(result.tool, "search");
        assert_eq!(result.output, "3 matches found");
    }

    #[test]
    fn parse_tool_result__should_return_none_for_plain_text() {
        // Given
        let text = "No tool result here.";

        // When
        let result = parse_tool_result(text);

        // Then
        assert!(result.is_none());
    }

    #[test]
    fn parse_tool_result__should_return_none_for_unclosed_tag() {
        // Given
        let text = concat!("<magent-tool-result tool=\"search\">\n", "partial output\n",);

        // When
        let result = parse_tool_result(text);

        // Then
        assert!(result.is_none());
    }

    #[test]
    fn parse_tool_result__should_trim_output_whitespace() {
        // Given
        let text = concat!(
            "<magent-tool-result tool=\"read\">\n",
            "\n",
            "  file content  \n",
            "\n",
            "</magent-tool-result>",
        );

        // When
        let result = parse_tool_result(text).unwrap();

        // Then
        assert_eq!(result.output, "file content");
    }

    // --- reconstruct_messages ---

    #[test]
    fn reconstruct_messages__should_return_system_and_user_for_plain_text() {
        // Given — response with no tool calls
        let response = "The answer is 42.";

        // When
        let messages = reconstruct_messages("system", "prompt", response);

        // Then
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0], llm::Message::system("system"));
        assert_eq!(messages[1], llm::Message::user("prompt"));
    }

    #[test]
    fn reconstruct_messages__should_reconstruct_single_tool_call() {
        // Given — one tool call + result, then trailing text
        let response = concat!(
            "Let me search.\n",
            "<magent-tool-call tool=\"search\">\n",
            "<magent-input>query</magent-input>\n",
            "</magent-tool-call>\n",
            "<magent-tool-result tool=\"search\">\n",
            "found it\n",
            "</magent-tool-result>\n",
            "Here is the answer.",
        );

        // When
        let messages = reconstruct_messages("sys", "prompt", response);

        // Then — system, user, assistant (text+call), user (result)
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0], llm::Message::system("sys"));
        assert_eq!(messages[1], llm::Message::user("prompt"));
        // Assistant message: text + tool call block
        assert!(messages[2].role == "assistant");
        assert!(messages[2].content.contains("Let me search."));
        assert!(messages[2].content.contains("</magent-tool-call>"));
        // User message: tool result block
        assert!(messages[3].role == "user");
        assert!(messages[3].content.contains("<magent-tool-result"));
        assert!(messages[3].content.contains("found it"));
        // Trailing text is NOT in messages
    }

    #[test]
    fn reconstruct_messages__should_reconstruct_multiple_tool_calls() {
        // Given — two tool call/result rounds
        let response = concat!(
            "Searching.\n",
            "<magent-tool-call tool=\"search\">\n",
            "<magent-input>q1</magent-input>\n",
            "</magent-tool-call>\n",
            "<magent-tool-result tool=\"search\">\n",
            "result 1\n",
            "</magent-tool-result>\n",
            "Now reading.\n",
            "<magent-tool-call tool=\"read\">\n",
            "<magent-input>file.md</magent-input>\n",
            "</magent-tool-call>\n",
            "<magent-tool-result tool=\"read\">\n",
            "result 2\n",
            "</magent-tool-result>\n",
            "Final answer.",
        );

        // When
        let messages = reconstruct_messages("sys", "prompt", response);

        // Then — system, user, (assistant, user) x2
        assert_eq!(messages.len(), 6);
        assert_eq!(messages[2].role, "assistant");
        assert!(messages[2].content.contains("Searching."));
        assert_eq!(messages[3].role, "user");
        assert!(messages[3].content.contains("result 1"));
        assert_eq!(messages[4].role, "assistant");
        assert!(messages[4].content.contains("Now reading."));
        assert_eq!(messages[5].role, "user");
        assert!(messages[5].content.contains("result 2"));
    }

    #[test]
    fn reconstruct_messages__should_ignore_trailing_text_after_last_result() {
        // Given
        let response = concat!(
            "<magent-tool-call tool=\"search\">\n",
            "<magent-input>q</magent-input>\n",
            "</magent-tool-call>\n",
            "<magent-tool-result tool=\"search\">\n",
            "found\n",
            "</magent-tool-result>\n",
            "This trailing text should not be a message.",
        );

        // When
        let messages = reconstruct_messages("sys", "prompt", response);

        // Then — no message contains the trailing text
        assert_eq!(messages.len(), 4); // sys, user, assistant, user(result)
        for msg in &messages {
            assert!(
                !msg.content.contains("trailing text"),
                "trailing text should not appear in messages"
            );
        }
    }

    #[test]
    fn reconstruct_messages__should_handle_empty_response() {
        // Given
        let response = "";

        // When
        let messages = reconstruct_messages("sys", "prompt", response);

        // Then
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn reconstruct_messages__should_handle_partial_response_ending_in_tool_call() {
        // Given — tool call with no result (e.g. crash mid-execution)
        let response = concat!(
            "Let me search.\n",
            "<magent-tool-call tool=\"search\">\n",
            "<magent-input>query</magent-input>\n",
            "</magent-tool-call>",
        );

        // When
        let messages = reconstruct_messages("sys", "prompt", response);

        // Then — no tool-result blocks, so just system + user
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn reconstruct_messages__should_roundtrip_with_process_directive_format() {
        // Given — build a response the same way process_directive does
        let llm_turn_1 = concat!(
            "Let me search for that.\n",
            "<magent-tool-call tool=\"search\">\n",
            "<magent-input>pricing</magent-input>\n",
            "</magent-tool-call>",
        );
        let result_1 = format_tool_result(&ToolResult {
            tool: "search".to_string(),
            output: "Found 2 results.".to_string(),
        });
        let llm_turn_2 = "Here is the answer based on the search.";

        // Build full_response as process_directive would
        let mut full_response = String::new();
        full_response.push_str(llm_turn_1);
        full_response.push('\n');
        full_response.push_str(&result_1);
        full_response.push('\n');
        full_response.push_str(llm_turn_2);

        // Build expected messages as process_directive would
        let expected = vec![
            llm::Message::system("sys"),
            llm::Message::user("prompt"),
            llm::Message::assistant(llm_turn_1),
            llm::Message::user(&result_1),
        ];

        // When
        let reconstructed = reconstruct_messages("sys", "prompt", &full_response);

        // Then
        assert_eq!(reconstructed, expected);
    }
}
