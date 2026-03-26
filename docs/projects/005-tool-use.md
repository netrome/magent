# Tool Use + Knowledge Base Search

**Status:** Accepted

## Goal

Give the agent the ability to gather information beyond the current document. The first concrete use: searching the knowledge base to find relevant files and content. This design also establishes the general tool-use framework that future tools (web fetch, messaging, etc.) will plug into.

We want:
```markdown
@magent what have I written about error handling?
```

The agent decides it needs to search, calls the search tool, gets results, and writes a response that synthesizes information from across the knowledge base — all without the user having to specify which files to look at.

## Constraints

- **Model-agnostic**: tool calling is text-based (XML tags), not API-level function calling. Must work with any model via the OpenAI-compatible chat completions endpoint.
- **Transparent**: every tool call and result is visible in the response block. No hidden state.
- **Markdown is the interface**: tool calls follow the existing `magent-*` tag vocabulary.
- **Safe by default**: local tools (search, read) auto-execute. External tools (future: web fetch) require user approval.
- **Minimal dependencies**: search is implemented with `regex` + filesystem walking. No new binary dependencies.

## Tool-use framework

### Tag vocabulary

The agent calls tools using XML tags in its response:

```markdown
<magent-tool-call tool="search">
<magent-input>error handling</magent-input>
</magent-tool-call>
```

The system provides results:

```markdown
<magent-tool-result tool="search">
3 matches across 2 files:

rust.md:45: Rust uses Result<T, E> for recoverable errors and panic! for
rust.md:46: unrecoverable errors. The ? operator propagates errors concisely.

go.md:12: Go uses multiple return values for error handling, conventionally
go.md:13: returning an error as the last value.
</magent-tool-result>
```

### Two tiers of tools

| Tier | Approval | Examples | Rationale |
|------|----------|----------|-----------|
| **Auto-execute** | None | `search`, `read`, `list` | Read-only operations within the knowledge base. No side effects, no cost. |
| **Gated** | User must accept | `web_fetch`, `send_message` | External side effects, network access, potential cost. |

Auto-execute tools are processed in a loop during a single directive processing pass. The user only sees the final response (with the tool call history embedded for transparency).

Gated tools follow the edit-acceptance pattern: the tool call is written with `status="proposed"`, the user changes it to `status="accepted"`, and magent continues processing on the next file change.

### Processing flow: auto-execute tools

```
1. Parse directive, send prompt + document to LLM
2. LLM responds with text + tool call(s)
3. Parse first tool call from response
4. Execute the tool
5. Send conversation so far (prompt, LLM response, tool result) back to LLM
6. LLM continues — may call more tools (→ repeat from 3) or produce final text
7. Write complete response (including tool call history) to file
```

All of this happens within a single `process_file` invocation. The user sees the file update once, with the final response.

### Processing flow: gated tools (future)

```
1. Parse directive, send prompt + document to LLM
2. LLM responds with a gated tool call
3. Write response with tool call as status="proposed"
   (processing pauses here — file is written, watcher will see future changes)

--- User changes status to "accepted" ---

4. Watcher triggers, process_file detects accepted tool call
5. Execute the tool
6. Reconstruct conversation from response block content
7. Send conversation + tool result to LLM
8. LLM continues (may call more tools, or produce final text)
9. Append tool result + LLM continuation to response block
```

The response block serves as the conversation log. No separate state file needed.

### One tool call per turn

The stop sequence design means the LLM emits at most one tool call per turn — generation halts at `</magent-tool-call>`, magent executes the tool, sends the result back, and the LLM continues. This is intentional for MVP: simpler loop, predictable behavior, and no ambiguity about execution order. Parallel/batch tool calls can be added later without changing the tag format.

### Stop sequences

To prevent the model from hallucinating tool results, use `</magent-tool-call>` as a stop sequence in the API request. When the model outputs a tool call, generation stops immediately. Magent executes the tool and sends the real result back.

Fallback for APIs that don't support stop sequences: parse the response up to the first `</magent-tool-call>`, discard everything after it.

### Multi-turn message construction

The LLM conversation for a tool-using directive looks like:

```
Turn 1 — system: [system prompt with document + tool definitions]
Turn 1 — user: "what have I written about error handling?"
Turn 1 — assistant: "Let me search for that.\n<magent-tool-call tool=\"search\">..."
Turn 2 — user: "<magent-tool-result tool=\"search\">...results...</magent-tool-result>"
Turn 2 — assistant: "Based on your notes, here's a summary..."
```

Tool results are injected as user messages (this is the standard convention for text-based tool use and works with any chat model). Each round-trip is a new API call.

### Conversation limits

To prevent runaway tool-call loops:
- **Max tool calls per directive**: 5 (configurable later via config file)
- If the limit is reached, the last LLM response is written as-is with a note: `(Tool call limit reached — response may be incomplete.)`

## The search tool

### Interface

```
Tool: search
Description: Search for text across markdown files in the knowledge base.

Input: A search query (plain text or regex pattern).
       Optionally prefixed with options:
       - path:subdir/ — limit search to a subdirectory
       - glob:*.md — file pattern filter (default: *.md)
       - max:N — maximum results (default: 20)

Parsing: greedy prefix parse. Consume recognized `key:value` tokens
(path:, glob:, max:) from the front of the input, left to right. Stop
at the first token that doesn't match a known key. Everything remaining
is the query. This means a query that literally starts with "path:" would
need a leading space or dummy prefix, but that's an acceptable edge case.

Examples:
  <magent-tool-call tool="search">
  <magent-input>error handling</magent-input>
  </magent-tool-call>

  <magent-tool-call tool="search">
  <magent-input>path:notes/ max:10 Result&lt;T, E&gt;</magent-input>
  </magent-tool-call>
```

### Why not expose `rg` directly?

The LLM could construct raw `rg` commands, and models are familiar with the syntax. But:

1. **Output control**: `rg` can produce enormous output. A dedicated interface truncates results and provides a consistent, compact format the model can parse reliably.
2. **No shell injection surface**: if the agent constructs arbitrary flags, we're one `--replace` away from unintended file modifications. A structured interface with a fixed parameter set eliminates this.
3. **Stable contract**: the tool interface shouldn't depend on models knowing `rg` flags. A simple query + optional filters is universal.
4. **`rg` as implementation is fine** — just not as the interface.

### Output format

```
5 matches across 3 files:

notes/rust.md:45: Rust uses Result<T, E> for recoverable errors and panic! for
notes/rust.md:46: unrecoverable errors. The ? operator propagates errors concisely.

notes/go.md:12: Go uses multiple return values for error handling, conventionally
notes/go.md:13: returning an error as the last value.

notes/python.md:30: Python uses try/except blocks for error handling.
```

Each match shows: relative path, line number, and the matching line with one line of surrounding context. Results are grouped by file. If results exceed `max`, the output ends with `(N more matches not shown)`.

When no matches are found: `No matches found for: "query"`.

### Implementation

**Option A: Shell out to `rg`**

Call `rg --json` and parse the structured output. Provides fast, correct regex search with all of rg's features.

Pros: fast, feature-complete, good regex. Cons: requires rg to be installed (not guaranteed), external process dependency.

**Option B: `regex` + filesystem walk**

Walk the directory tree with `std::fs`, filter for `.md` files, read each file, and search with the `regex` crate.

Pros: no external dependencies (regex is already a transitive dep), fully self-contained. Cons: slower for very large KBs (irrelevant at personal KB scale), no fancy rg features like smart case.

**Recommendation: Option B for MVP.** A personal knowledge base is small enough that reading all `.md` files on each search is perfectly fast. The implementation is ~50 lines of straightforward Rust. If performance becomes an issue later, we can add rg as an optional backend.

## The read tool

A companion to search: once the agent finds relevant files via search, it may want to see more context.

```
Tool: read
Description: Read content from a file in the knowledge base.

Input: A relative file path, optionally followed by a line range.

Examples:
  <magent-tool-call tool="read">
  <magent-input>notes/rust.md</magent-input>
  </magent-tool-call>

  <magent-tool-call tool="read">
  <magent-input>notes/rust.md 40-60</magent-input>
  </magent-tool-call>
```

Without a line range, returns the full file content. With a range (e.g. `40-60`), returns only those lines (1-indexed, inclusive). The typical workflow is search → find interesting lines → read a range around them.

Returns the file content (with line numbers), or an error if the file doesn't exist or is outside the knowledge base root.

The same path validation rules from explicit references apply: no traversal outside the watched root.

This tool is what enables "agent decides to expand a link" — the agent sees a `[linked file](path.md)` in the document, and if it judges the content would be helpful, it calls `read` to pull it in.

**Note on large results:** for the initial implementation, tool results are embedded inline in the response block. This is fine for line-range reads and bounded search results. A future caching mechanism (see `docs/projects/006-tool-result-caching.md`) will store large results in `.magent/tool-cache/` and reference them from the response block instead.

## What the response looks like

### Auto-execute example

```markdown
@magent what have I written about error handling?

<magent-response>
<magent-thinking>
The user wants a summary across their knowledge base. I should search for
relevant content first.
</magent-thinking>
<magent-tool-call tool="search">
<magent-input>error handling</magent-input>
</magent-tool-call>
<magent-tool-result tool="search">
5 matches across 3 files:

notes/rust.md:45: Rust uses Result<T, E> for recoverable errors...
notes/go.md:12: Go uses multiple return values for error handling...
notes/python.md:30: Python uses try/except blocks...
</magent-tool-result>
<magent-tool-call tool="read">
<magent-input>notes/rust.md</magent-input>
</magent-tool-call>
<magent-tool-result tool="read">
(full content of rust.md)
</magent-tool-result>
You have notes on error handling in three languages:

- **Rust** (notes/rust.md): Detailed coverage of Result<T, E>, the ? operator...
- **Go** (notes/go.md): Brief note on multiple return values...
- **Python** (notes/python.md): One-liner about try/except...

Your Rust notes are the most detailed. The Go and Python notes could use expansion.
</magent-response>
```

### Gated tool example (future: web fetch)

```markdown
@magent summarize this blog post: https://example.com/post

<magent-response>
<magent-thinking>
I need to fetch the web page content to summarize it.
</magent-thinking>
<magent-tool-call tool="web_fetch" status="proposed">
<magent-input>https://example.com/post</magent-input>
</magent-tool-call>
</magent-response>
```

User changes `status="proposed"` to `status="accepted"`, magent fetches the page and continues:

```markdown
<magent-response>
<magent-thinking>
I need to fetch the web page content to summarize it.
</magent-thinking>
<magent-tool-call tool="web_fetch" status="executed">
<magent-input>https://example.com/post</magent-input>
</magent-tool-call>
<magent-tool-result tool="web_fetch">
(extracted text content of the web page)
</magent-tool-result>
The post discusses three key themes...
</magent-response>
```

## System prompt changes

The system prompt gains a tools section, appended after the document:

```
=== TOOLS ===

You have access to tools for gathering information. To use a tool, output:

<magent-tool-call tool="tool_name">
<magent-input>your input here</magent-input>
</magent-tool-call>

After a tool call, stop and wait. The system will provide the result in a
<magent-tool-result> block, then you continue your response.

You may call multiple tools in sequence. Do not guess tool results — always
call the tool and use the actual result.

Available tools:

## search
Search for text across markdown files in the knowledge base.
Input: a search query (plain text or regex). Optional prefixes: path:subdir/, max:N
Returns: matching lines with file paths and line numbers.

## read
Read the full content of a file in the knowledge base.
Input: a relative file path (e.g. notes/rust.md)
Returns: the file content.

=== END TOOLS ===
```

This section is only included when tools are available (i.e., always for now, but the framework supports conditional tool availability).

## Changes needed

### New module: `tool.rs`

Defines the tool trait and built-in tools:

```rust
pub struct ToolCall {
    pub tool: String,
    pub input: String,
    pub status: Option<ToolStatus>,  // None for auto-execute, Some for gated
}

pub enum ToolStatus {
    Proposed,
    Accepted,
    Executed,
}

pub struct ToolResult {
    pub tool: String,
    pub output: String,
}

/// Parse tool calls from an LLM response.
pub fn parse_tool_calls(response: &str) -> (Vec<ToolCall>, String);

/// Format a tool result for injection into conversation.
pub fn format_tool_result(result: &ToolResult) -> String;
```

### New module: `tools/search.rs`

```rust
pub struct SearchTool {
    root: PathBuf,
}

impl SearchTool {
    /// Execute a search query across the knowledge base.
    pub fn execute(&self, input: &str) -> String;
}
```

Walks `.md` files under root, applies regex search, formats results.

### New module: `tools/read.rs`

```rust
pub struct ReadTool {
    root: PathBuf,
}

impl ReadTool {
    /// Read a file from the knowledge base.
    pub fn execute(&self, input: &str) -> String;
}
```

Validates path is within root, reads and returns content.

### LLM client (`llm.rs`)

- Add `stop` parameter support to `ChatRequest` (for `</magent-tool-call>` stop sequence)
- Replace `complete` with a unified `complete_messages` interface. The existing single-turn usage builds a one-turn message list at the call site. One interface, no duplication.

The `LlmClient` trait becomes:

```rust
pub trait LlmClient {
    fn complete_messages(&self, messages: &[Message], stop: &[&str])
        -> impl Future<Output = Result<String, LlmError>> + Send;
}
```

The old `complete(prompt, document)` call sites construct a `Vec<Message>` (system + user) and call `complete_messages` with an empty stop list. No convenience wrapper needed.

### Tool dispatch

Tools are dispatched via a simple match in the processing loop. No registry, no trait object indirection:

```rust
match call.tool.as_str() {
    "search" => search_tool.execute(&call.input),
    "read" => read_tool.execute(&call.input),
    _ => format!("Unknown tool: {}", call.tool),
}
```

### Error handling in tools

Tool execution must not panic or propagate errors to the caller. Tools return their errors as the tool result string, so the LLM can see what went wrong and recover (e.g., retry with a corrected path, try a different query):

- File not found → `"Error: file 'foo.md' not found"`
- Invalid regex → `"Error: invalid regex pattern: ..."`
- Path outside root → `"Error: path is outside the knowledge base"`

The `execute()` method returns `String`, not `Result`. Errors are part of the conversation, not control flow.

### Processing loop (`lib.rs`)

`process_file` gains a tool-use loop for auto-execute tools:

```rust
// Pseudocode
let mut messages = build_initial_messages(directive, document, tools_prompt);
let mut full_response = String::new();
let mut tool_call_count = 0;

loop {
    let response = client.complete_messages(&messages, &["</magent-tool-call>"]).await?;
    full_response.push_str(&response);

    let tool_call = parse_tool_call(&response);  // at most one per turn
    let Some(call) = tool_call else {
        break;  // No tool call — done
    };

    tool_call_count += 1;
    let result = execute_tool(&call);  // returns String, never fails
    full_response.push_str(&format_tool_result(&call.tool, &result));
    messages.push(assistant_message(&response));
    messages.push(user_message(&format_tool_result(&call.tool, &result)));

    if tool_call_count >= MAX_TOOL_CALLS {
        full_response.push_str("\n(Tool call limit reached.)");
        break;
    }
}

write_response(path, &directive.prompt, &full_response)?;
```

## Design decisions to revisit later

- **Tool call limit**: hardcoded at 5 for now. Move to config file when that exists.
- **Search result size**: hardcoded max 20 results. May need tuning based on model context windows.
- **Parallel tool calls**: the stop sequence design means one call per turn. A future optimization could allow the model to emit multiple calls in one turn (requires a different parsing strategy and removing the stop sequence).
- **Context window management**: search results + file reads can blow up context. For now, this is the model's problem. Later, we may want to truncate or summarize tool results.
- **ToC / section extraction**: `read` returns the full file. A future `toc` or `read_section` tool could return just a table of contents or a specific section, saving context space.

## Non-goals

- **Native function calling**: we use text-based tool calls only. This keeps us model-agnostic.
- **Gated tools in initial implementation**: the framework supports them (and this doc describes the flow), but the first PR scope is auto-execute tools only. Gated tools come with web_fetch.
- **Tool configuration**: tools are hardcoded. A tool registry / plugin system is premature.
- **Streaming**: tool call detection works on the complete response. Streaming support is orthogonal.

## Task breakdown

### PR 1: Tool call parser (`tool.rs`)

**Changes:**
- New `tool.rs` module with `ToolCall`, `ToolResult` structs
- `parse_tool_calls()` — extract `<magent-tool-call>` blocks from LLM response
- `format_tool_result()` — format `<magent-tool-result>` blocks for injection

**Acceptance criteria:**
- Parses single and multiple tool calls from a response
- Extracts tool name and input text
- Handles responses with no tool calls (returns empty list + full text)
- Malformed tool calls are treated as plain text
- Unit tests covering all cases

### PR 2: Search tool implementation (`tools/search.rs`)

**Changes:**
- New `tools/` module directory with `search.rs`
- `SearchTool` struct with `execute()` method
- Walks `.md` files under root, searches with `regex`, formats results
- Greedy prefix parse for optional `path:`, `glob:`, `max:` options
- `execute()` returns `String` (errors are returned as result text, not `Result`)

**Acceptance criteria:**
- Finds matches across multiple files
- Returns formatted results with file paths and line numbers
- Respects `path:` filter (subdirectory)
- Respects `max:` limit with "(N more matches not shown)" note
- Returns "No matches found" for empty results
- Invalid regex returns error in result text (e.g. `"Error: invalid regex pattern: ..."`)
- Paths outside root are not searched
- Unit tests with temp directory fixtures

### PR 3: Read tool implementation (`tools/read.rs`)

**Changes:**
- `ReadTool` struct with `execute()` method
- Path validation (within root, file exists)
- `execute()` returns `String` (errors are returned as result text, not `Result`)

**Acceptance criteria:**
- Returns file content for valid paths
- Returns error message for missing files (e.g. `"Error: file 'foo.md' not found"`)
- Rejects paths outside the knowledge base root (error in result text)
- Supports optional line range (`file.md 40-60`)
- Unit tests

### PR 4: Multi-turn LLM support

**Changes:**
- Replace `complete()` with `complete_messages()` on `LlmClient` trait
- Add `stop` sequence support to `ChatRequest`
- Implement for `ChatClient`
- Update all existing call sites to build message lists

**Acceptance criteria:**
- Multi-turn conversations work (multiple messages sent, response returned)
- Stop sequences halt generation at the specified token
- All existing tests pass with the unified interface (no behavior change)
- Tests with wiremock verifying request format

### PR 5: Wire tool use into processing loop

**Changes:**
- Update system prompt to include tools section
- Add tool-use loop to `process_file`
- Tool call limit (5)
- Full response (including tool call history) written to file

**Acceptance criteria:**
- Directive that triggers a search tool call → search executed → results fed back → final response written
- Tool call history visible in response block
- Tool call limit prevents infinite loops
- Non-tool directives still work exactly as before
- Integration tests with fake LLM returning tool calls
