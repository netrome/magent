# Incremental Response Writing

**Status:** Draft

## Goal

Write agent responses to disk incrementally during the tool-use loop, so users can watch progress, intervene mid-execution, and recover from crashes.

See ADR-003 for the architectural decision and rationale.

## Design

### Mental model

```
file edit → watcher → find work → process one step → file edit → ...
```

The file is always the source of truth. The daemon is a loop that reads the file, does some work, and writes back. Whether the previous edit was made by the daemon or the user doesn't matter.

### What counts as "work"

Today, `process_file` looks for unprocessed directives (no response block). With this change, it also looks for in-progress directives (response block with `status="in-progress"`):

| State | Detection | Action |
|---|---|---|
| Unprocessed | No response block after directive | Start processing (open in-progress response) |
| In-progress | `<magent-response status="in-progress">` | Continue processing (resume tool loop) |
| Paused | `<magent-response status="paused">` | Skip (user resumes by changing to `in-progress`) |
| Complete | `<magent-response>` (no status) | Skip |

### Directive status in the parser

`Directive.processed` (bool) becomes a status enum:

```rust
enum DirectiveStatus {
    Unprocessed,
    InProgress,
    Paused,
    Complete,
}
```

The parser detects `<magent-response status="in-progress">` vs `<magent-response>` to distinguish in-progress from complete.

### The tool-use loop

Current flow (simplified):

```
process_directive(prompt, document) → String:
    messages = [system(document), user(prompt)]
    response = ""
    loop:
        llm_output = call_llm(messages)
        response += llm_output
        if no tool call: break
        result = execute_tool(tool_call)
        response += result
        messages += [assistant(llm_output), user(result)]
    return response
```

New flow:

```
process_directive(prompt, document, path):
    open response block with status="in-progress"
    messages = [system(document), user(prompt)]
    response = ""

    loop:
        // Re-read to detect user intervention
        file_response = read_response_content(path, prompt)
        if file_response is None → break  // User deleted response
        if file_response != response:
            // User modified — adopt their version
            response = file_response
            messages = reconstruct_messages(document, prompt, response)

        llm_output = call_llm(messages)
        response += llm_output
        write_response(path, prompt, response, in_progress=true)

        if tool call:
            result = execute_tool(tool_call)
            response += result
            write_response(path, prompt, response, in_progress=true)
            messages += [assistant(llm_output), user(result)]
        else:
            formatted = format_response(response)  // Edit blocks etc.
            write_response(path, prompt, formatted, in_progress=false)
            break
```

Key differences:
- `process_directive` takes the file path and writes directly (no longer returns a String).
- Flushes to disk after each LLM output and each tool result.
- Re-reads the file before each LLM call to detect user intervention.
- Only applies `format_response` on the final write (edit proposals, etc.).

### Message reconstruction

When the daemon detects the response content on disk differs from what it wrote (user intervention), it rebuilds the LLM message history by parsing the response content.

The response content is a sequence of:
1. Free text (LLM output)
2. `<magent-tool-call>...</magent-tool-call>` blocks
3. `<magent-tool-result>...</magent-tool-result>` blocks

These map to messages:
- Text + tool call → assistant message
- Tool result → user message

The reconstruction function parses this sequence and builds the messages array, prepended with the system message and original user prompt. This is the same format the LLM originally saw, so it can continue naturally.

### Writer changes

The writer currently has one operation: `insert_response` (create a complete response block). This expands to:

- **`write_response_block(path, prompt, content, in_progress)`** — creates or replaces the response block for a directive. If `in_progress` is true, the opening tag is `<magent-response status="in-progress">`. If the block already exists, its content is replaced.

The existing `insert_response` can be refactored into this — it's a special case where the block doesn't exist yet and `in_progress` is false.

### Reading response content

New parser function: given file content and a directive prompt, extract the text content inside that directive's response block. Returns `None` if no response block exists (including if the user deleted it).

### Crash recovery

If the daemon crashes mid-execution, the file contains an in-progress response block. Recovery options:

1. **On startup**: scan all watched files for in-progress response blocks and queue them for processing.
2. **On next file change**: if the user edits and saves the file, the watcher triggers and `process_file` picks up the in-progress directive.

Option 1 is more user-friendly (automatic resume). Option 2 requires a manual touch but is simpler to implement. We should implement option 1.

### User intervention patterns

All intervention is "edit the file and save":

**Rewind** — delete tool calls from the end:
```markdown
<magent-response status="in-progress">
Let me search for that.

<magent-tool-call>
search | query: pricing
</magent-tool-call>
<magent-tool-result>
Found 2 results...
</magent-tool-result>

<!-- User deletes everything below this point -->
<!-- Agent re-reads, sees history ends at first tool result, continues from there -->
</magent-response>
```

**Modify a tool result** — edit the `<magent-tool-result>` content:
```markdown
<magent-tool-result>
Actually, I want you to focus on this result instead: Enterprise plan is $999/mo
</magent-tool-result>
```
The agent sees the modified result and continues accordingly.

**Stop** — delete the response block entirely. This triggers the watcher, which sees an unprocessed directive and starts again. This means "retry." To truly stop, delete the `@magent` line too. Both are intuitive.

**Pause** — change `status="in-progress"` to `status="paused"`. The daemon re-reads, sees a paused response, and stops. To resume, change back to `status="in-progress"` — the watcher triggers and the daemon picks up where it left off.

## Non-goals

- **Streaming within a single LLM turn**: we flush after the LLM call completes, not token-by-token. Token streaming could be added later but is a separate concern.
- **Concurrent directives**: this project handles one directive at a time, as today.
- **Undo/version history**: the file is plain text — users can use git for this.
- **UI for intervention**: the interface is the text editor. No special commands or controls.

## Risks

- **Parser robustness**: message reconstruction from response content needs to handle edge cases (malformed tool blocks, partial writes). Good test coverage is critical here.
- **Write conflicts**: if the user saves at the exact moment the daemon writes, one could overwrite the other. Atomic writes (temp + rename) minimize the window. Acceptable risk for a local tool.
- **Edit formatting**: during in-progress, edit blocks appear raw. This is intentional but could confuse users who expect the proposed-edit format. Should be documented.

## Task breakdown

### PR 1: Response block status parsing

Extend the parser to handle `<magent-response status="in-progress">`.

**Changes:**
- `parser.rs`: change `Directive.processed: bool` to `Directive.status: DirectiveStatus` enum (`Unprocessed`, `InProgress`, `Complete`).
- Update `parse_directives` to detect the `status` attribute on the opening tag.
- Add `extract_response_content(content, prompt)` — returns the text inside a directive's response block.
- Update all call sites that check `directive.processed`.

**Acceptance criteria:**
- `<magent-response>` → `Complete`
- `<magent-response status="in-progress">` → `InProgress`
- `<magent-response status="paused">` → `Paused`
- No response block → `Unprocessed`
- `extract_response_content` returns correct content for each case.
- Existing tests updated and passing.

### PR 2: Writer support for in-progress response blocks

Replace the single-shot writer with one that can create, update, and close response blocks.

**Changes:**
- `writer.rs`: new function `write_response_block(path, prompt, content, in_progress)` that creates or replaces a response block.
- Refactor existing `write_response` / `insert_response` into the new function.

**Acceptance criteria:**
- Can create a new in-progress response block.
- Can update an existing in-progress response block (replace content).
- Can close an in-progress response block (remove status attribute).
- Can create a complete response block in one shot (backwards compatible with current behavior).

### PR 3: Message reconstruction from response content

Parse a response block's content back into the LLM message sequence.

**Changes:**
- New function (in `tool.rs` or a new module): given response text, extract the sequence of (text, tool_call, tool_result) segments.
- Build the `Vec<Message>` from this sequence (system + user prompt + alternating assistant/user messages).

**Acceptance criteria:**
- Correctly reconstructs messages from a response with multiple tool calls.
- Handles responses with no tool calls (just text).
- Handles partial responses (ends with a tool call but no result yet — though this shouldn't happen in normal flow, it could after a crash mid-tool-execution).
- Round-trips: messages → response text → reconstructed messages produces equivalent message history.

### PR 4: Incremental writing in the tool-use loop

The main change. Restructure `process_directive` to write incrementally and re-read before each LLM call.

**Changes:**
- `lib.rs`: `process_directive` takes `path` and `prompt`, writes directly to the file.
- Opens in-progress response block before the first LLM call.
- Flushes after each LLM output and tool result.
- Re-reads before each LLM call, detects user modifications, rebuilds messages if needed.
- Closes the response block (removes status) on completion.
- `process_file` no longer calls `write_response` after `process_directive` — writing is internal.
- `process_file` handles `InProgress` directives (calls `process_directive` to continue them).

**Acceptance criteria:**
- Response block updates on disk after each tool call and result.
- User can delete the response block mid-execution and the agent stops.
- User can delete tool calls from the end and the agent rewinds.
- User can modify a tool result and the agent continues with the modified version.
- Completed responses have no status attribute.
- Existing tests updated and passing, new integration tests for intervention scenarios.

### PR 5: Crash recovery

Resume in-progress responses on daemon startup.

**Changes:**
- On startup (after watcher is initialized), scan all `.md` files in the watched directory for in-progress response blocks.
- Queue matching file paths for processing (send through the watcher channel, or process directly).

**Acceptance criteria:**
- Daemon resumes an in-progress response after restart without user intervention.
- Does not reprocess already-complete responses.
- Works correctly when multiple files have in-progress responses (processes them sequentially).
