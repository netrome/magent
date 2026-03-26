# Document Context for LLM Prompts

**Status:** Complete

## Goal

Enable the model to interact with the current document. Two use cases:

1. **Answer questions about the document** — "what's the main argument in this essay?"
2. **Suggest edits** — "rephrase the second paragraph", "reorganize this todo list"

Today, `process_file` sends only the directive prompt to the LLM (`client.complete(&directive.prompt)`). The model has zero visibility into the surrounding document.

## Approach

Include the full document content as context when calling the LLM. The response still goes in the `<!-- magent:start/end -->` markers as today.

This means edit directives produce the new content *inside the response block* — the user sees the suggestion and applies it manually. Actual inline editing (the model rewriting content outside the markers) is a separate, larger feature and out of scope here.

### Option A: Change `LlmClient` trait to accept context

```rust
pub trait LlmClient {
    fn complete(
        &self,
        prompt: &str,
        document: Option<&str>,
    ) -> impl Future<Output = Result<String, LlmError>> + Send;
}
```

`ChatClient` maps this to the OpenAI messages format:

- **System message**: a short preamble + the document content (when provided)
- **User message**: the directive prompt

```
system: You are an AI assistant embedded in a markdown document. The user
        will ask questions or request edits. When suggesting edits, output
        the replacement content directly — no explanations unless asked.

        === DOCUMENT ===
        {document content}
        === END DOCUMENT ===

user:   {directive prompt}
```

When `document` is `None`, no system message is sent (preserves current behavior for any future non-document use).

**Pros:** Clean separation of context vs. prompt. LLMs handle system messages well. Minimal surface area change — callers just pass one extra arg.

**Cons:** Changes the trait signature, requiring updates to all implementations (real + test fakes).

### Option B: Build composite prompt in `process_file`, keep trait as-is

```rust
// In process_file:
let full_prompt = format!(
    "The following is a markdown document:\n\n{}\n\n---\n\nUser request: {}",
    content, directive.prompt
);
client.complete(&full_prompt).await
```

**Pros:** Zero trait changes. Fakes/mocks don't need updating.

**Cons:** Stuffs everything into a single user message — loses the system/user distinction that helps models separate instructions from content. The prompt construction logic lives in the orchestration layer rather than the LLM layer where it belongs. Harder to test the prompt construction independently.

### Recommendation: Option A

The trait change is small and the system message separation is worth it. It gives us better model behavior (system messages are treated differently by most LLMs) and keeps prompt construction testable. The fake LLM implementations in tests just gain an ignored `_document` parameter.

## System prompt

Keep it minimal. The model should:
- Treat the document as the primary context
- Answer questions concisely
- When asked to edit, output the replacement content directly (not wrapped in explanations)
- Not repeat the entire document when only a small part was requested

Proposed system prompt (lives as a constant in `llm.rs`):

```
You are an AI assistant embedded in a markdown document. The user will ask
questions or request changes to the document below.

Before responding, think through your approach inside <magent-thinking> tags:

<magent-thinking>
Your reasoning here — what the user is asking, what needs to change, etc.
</magent-thinking>

Then provide your response or edit blocks after the thinking.

Rules:
- When answering questions, be concise and reference the document directly.
- When asked to edit or rewrite content, output ONLY the replacement content.
  Do not wrap it in explanations or repeat unchanged parts of the document.
- Respond in the same language as the document unless asked otherwise.

=== DOCUMENT ===
{document}
=== END DOCUMENT ===
```

## Document size

For now, include the full document. Most personal knowledge-base files are well under 100KB. If this becomes a problem, we can add truncation or windowing later — but it's not worth designing for now.

## Changes required

### `llm.rs`
- Add `document: Option<&str>` parameter to `LlmClient::complete`
- Add a system prompt constant
- In `ChatClient::complete`: when `document` is `Some`, prepend a system message with the prompt template + document content
- Update `ChatRequest.messages` construction

### `lib.rs`
- In `process_file`: pass `&content` as document context to `client.complete`
- Update fake LLM implementations in tests to accept the new parameter

### Tests
- New unit tests in `llm.rs` verifying the system message is sent when document context is provided, and omitted when it's not
- Update existing integration test in `lib.rs` to verify document content is available (e.g., a fake LLM that echoes back part of the document)

## Non-goals

- **Inline document editing** (model rewriting content outside response markers). That's a follow-up.
- **Smart context windowing** (only sending nearby paragraphs). Full document is fine for now.
- **Multi-file context** (referencing other documents). Out of scope — this is single-document only.
- **Conversation history** (multi-turn). Each directive is still a standalone request.

## Task breakdown

### PR 1: Add document context to LLM trait and client

**Changes:**
- Add `document: Option<&str>` to `LlmClient::complete`
- Add system prompt constant
- Update `ChatClient` to construct system message when document is provided
- Update all trait implementations (fakes in tests)

**Acceptance criteria:**
- When `document` is `Some`, the HTTP request includes a system message with the document content
- When `document` is `None`, behavior is identical to today (no system message)
- Existing tests pass with updated signatures
- New tests verify system message construction

### PR 2: Wire document context into the processing loop

**Changes:**
- `process_file` passes file content as document context to `client.complete`
- Update integration-style tests in `lib.rs`

**Acceptance criteria:**
- A directive like `@magent what is this document about?` in a file with content gets a context-aware response (verifiable with the fake LLM checking that document content was received)
- Existing end-to-end behavior unchanged (responses still written to markers)
