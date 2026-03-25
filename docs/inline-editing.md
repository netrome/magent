# Inline Document Editing

## Goal

Enable magent to modify document content outside the response markers. Today, all LLM output goes inside `<!-- magent:start/end -->` blocks. This means directives like "fix the links in the list above" or "sort this list" produce a *suggestion* in the response block, but the original content stays unchanged — the user must apply edits manually.

We want:
```markdown
# Links

- [Rust](htps://rust-lang.org)
- [Tokio](htps://tokio.rs)

@magent fix the broken URLs above
```

To actually rewrite the URLs in place, with an accept step so the user reviews proposed edits before they're applied.

## Constraints

- **Transparency**: all changes must be inspectable. The user should be able to see what was changed (and ideally revert it).
- **Safety**: the model should not silently corrupt unrelated content. Edits should be scoped, and the user must approve before they're applied.
- **Markdown is the interface**: no sidecar state files, no special tooling required to see what happened.
- **Model-agnostic**: must work with local models (Ollama) and hosted APIs. Cannot rely on tool-use or structured output APIs — only plain text completions.

## Options

### Option A: Full document rewrite

The LLM returns the complete modified document. Magent replaces the entire file content.

**How it works:**
1. Detect that a directive requests an edit (heuristic or explicit opt-in via syntax like `@magent(edit)`).
2. Send the document to the LLM with a system prompt asking it to return the full modified document.
3. Replace the file content with the LLM output.
4. Append a response block after the directive with a summary of what changed.

**Pros:**
- Simplest implementation — no structured output parsing.
- The model has maximum flexibility; any edit is expressible.
- Works well with weaker models that struggle with structured formats.

**Cons:**
- **Token cost**: the model must echo the entire document even for a one-line fix. Bad for large files.
- **Risk of silent corruption**: models routinely make subtle unintended changes (dropped lines, reformatted whitespace, hallucinated content). The larger the document, the worse this gets.
- **Hard to audit**: the user sees the new file but has no inline record of what specifically changed (unless they use git diff).
- **Hostile to undo**: the previous content is gone unless the user has version control.
- **No natural accept step**: edits are applied immediately, no review possible.

### Option B: Search-and-replace with accept step

The LLM returns a sequence of find-and-replace blocks as proposed edits. The user reviews and accepts them, then magent applies them to the document.

**How it works:**
1. System prompt instructs the model to output edits as XML search/replace blocks.
2. Magent parses the response into proposed edits and writes them in the response block with `status="proposed"`.
3. The user reviews the proposals in their editor (or a smart renderer shows a diff UI).
4. The user changes `status="proposed"` to `status="accepted"` (or a tool does it with a button).
5. Magent detects the file change, applies the accepted edits, and updates the status to `status="applied"`.

**Pros:**
- **Safe by default**: no edits are applied until the user explicitly accepts.
- **Token-efficient**: model only outputs the changed parts, not the whole document.
- **Auditable**: the edit blocks in the response show exactly what was proposed and what was applied.
- **Scoped risk**: if a search string isn't found, that operation fails gracefully rather than corrupting unrelated content.
- **Proven pattern**: search-and-replace is how aider and Claude Code handle edits — it works well with capable models.
- **Tooling-friendly**: smart renderers can show diffs and accept buttons, but the bare experience is still just editing a text file.

**Cons:**
- **Requires structured output discipline**: the model must follow the search/replace format precisely. Weaker/smaller models may struggle.
- **Fragile matching**: the search string must appear exactly in the document. Whitespace differences or the model paraphrasing the original will cause mismatches.
- **Two-phase processing**: the daemon now handles both new directives and edit acceptances — more states to reason about.

### Option C: Marker-based section editing

The user explicitly marks the section to be edited using markers. The model's output replaces that section.

**How it works:**
1. The user wraps the target content with markers.
2. The LLM receives the document with the target markers visible.
3. The model outputs the replacement content in its response.
4. Magent replaces the marked section with the model's response.

**Pros:**
- **Explicit scope**: no ambiguity about what the model should edit. Zero risk of unintended changes elsewhere.
- **Simple implementation**: just find markers and replace the content between them. No structured output parsing needed.
- **Works with any model**: the model just produces content, no special format required.

**Cons:**
- **Friction**: the user must manually wrap content with markers before every edit. This is tedious and breaks the natural flow of "just tell the agent what to fix."
- **Doesn't handle "fix the paragraph above"**: the whole point is that the user can reference content naturally; requiring them to pre-mark it defeats the purpose.
- **Marker clutter**: the document accumulates markers that don't serve the document's content.

## Recommendation: Option B (search-and-replace with accept step)

Option B hits the best trade-off for magent's use case:

- It handles natural directives ("fix the URLs above", "sort this list") without requiring the user to pre-mark content.
- The accept step makes edits safe by default — no surprise rewrites.
- The proposed edits are visible in the file, making them inspectable without external tools.
- It's token-efficient, which matters for both cost and latency (especially with local models).
- It composes with the existing file-watching architecture: the daemon detects acceptance via file change events, no new I/O mechanism needed.

The main risk is structured output reliability with weaker models. Mitigation: start with a strict parser that rejects malformed blocks and falls back to writing the raw response in the response block (degrading gracefully to today's behavior).

## Migrate response markers to custom elements

As part of this work, migrate from HTML comment markers to custom XML elements throughout magent:

**Before (current):**
```markdown
<!-- magent:start -->
Response text here.
<!-- magent:end -->
```

**After:**
```markdown
<magent-response>
Response text here.
</magent-response>
```

**Why now:**
- The new edit tags (`<magent-edit>`, `<magent-search>`, `<magent-replace>`) are custom elements. Mixing HTML comments for responses with custom elements for edits would be inconsistent.
- One tag vocabulary (`magent-*` custom elements) is simpler to parse, style, and reason about.
- The project is young — few existing files to migrate, so the breaking change is cheap.
- Doing it as a preparatory step avoids two separate breaking changes.

**Rendering note:** in CommonMark, custom element tags on their own line start an HTML block, and content inside may not be processed as markdown. This is a change from the current behavior where content between HTML comments IS processed as markdown. In practice, the primary reading experience is in an editor (raw text), and magent controls the output format. If this proves problematic, we can ensure blank lines after opening tags to end the HTML block context.

This change affects the ADR (001-mvp-architecture) and requires an update to document the new marker format.

## What this looks like in practice

### Step 1: User writes a directive

```markdown
# Links

- [Rust](htps://rust-lang.org)
- [Tokio](htps://tokio.rs)

@magent fix the broken URLs above
```

### Step 2: Magent proposes edits

```markdown
# Links

- [Rust](htps://rust-lang.org)
- [Tokio](htps://tokio.rs)

@magent fix the broken URLs above

<magent-response>
<magent-thinking>
Both URLs use "htps" instead of "https". I need to fix each one.
</magent-thinking>
Fixed 2 broken URLs (htps → https):
<magent-edit status="proposed">
<magent-search>- [Rust](htps://rust-lang.org)</magent-search>
<magent-replace>- [Rust](https://rust-lang.org)</magent-replace>
</magent-edit>
<magent-edit status="proposed">
<magent-search>- [Tokio](htps://tokio.rs)</magent-search>
<magent-replace>- [Tokio](https://tokio.rs)</magent-replace>
</magent-edit>
</magent-response>
```

The document is unchanged. The proposed edits are visible in the response block.

### Step 3: User accepts

The user changes `status="proposed"` to `status="accepted"` on the edits they want (either manually or via a smart editor). They can also delete individual edit blocks they don't want.

### Step 4: Magent applies

Magent detects the file change, finds accepted edits, applies them to the document, and updates status:

```markdown
# Links

- [Rust](https://rust-lang.org)
- [Tokio](https://tokio.rs)

@magent fix the broken URLs above

<magent-response>
<magent-thinking>
Both URLs use "htps" instead of "https". I need to fix each one.
</magent-thinking>
Fixed 2 broken URLs (htps → https):
<magent-edit status="applied">
<magent-search>- [Rust](htps://rust-lang.org)</magent-search>
<magent-replace>- [Rust](https://rust-lang.org)</magent-replace>
</magent-edit>
<magent-edit status="applied">
<magent-search>- [Tokio](htps://tokio.rs)</magent-search>
<magent-replace>- [Tokio](https://tokio.rs)</magent-replace>
</magent-edit>
</magent-response>
```

## Design details

### Edit lifecycle

Each `<magent-edit>` block has a `status` attribute that tracks its state:

| Status | Meaning |
|--------|---------|
| `proposed` | LLM has suggested this edit. Awaiting user review. |
| `accepted` | User has approved this edit. Magent should apply it. |
| `applied` | Edit was successfully applied to the document. |
| `failed` | The search text was not found in the document. |

### Detecting edit vs. question directives

The model decides. The system prompt always includes search/replace format instructions. If the model returns `<magent-edit>` blocks, they're treated as proposed edits. If it returns plain text, it's a regular response. No user-facing syntax change needed.

### System prompt changes

Update the system prompt to instruct the model on the edit format:

```
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

You may include multiple edit blocks. The search text must match the document
exactly (character for character). Include enough surrounding context in the
search text to uniquely identify the location.

You may include plain text before, after, or between edit blocks to explain
what you changed.

When answering questions (no document edits needed), respond with plain text
as usual — do not use edit blocks.
```

### Edit parser

New module: `edit.rs`. Parses the LLM response to extract edit blocks.

```rust
pub struct Edit {
    pub search: String,
    pub replace: String,
}

/// Parse edit blocks from an LLM response.
/// Returns the edits and any non-edit text (summary, explanation).
pub fn parse_edits(response: &str) -> (Vec<Edit>, String);
```

Also needs a parser for reading `status` attributes from existing `<magent-edit>` blocks in a file (for the acceptance flow):

```rust
pub struct PendingEdit {
    pub search: String,
    pub replace: String,
    pub status: EditStatus,
}

pub enum EditStatus {
    Proposed,
    Accepted,
    Applied,
    Failed,
}

/// Parse edit blocks with status from a magent-response block.
pub fn parse_edit_blocks(response_content: &str) -> Vec<PendingEdit>;
```

### Applying edits

```rust
/// Apply a sequence of edits to document content.
/// Returns the modified content and a list of results (success/failure per edit).
pub fn apply_edits(content: &str, edits: &[Edit]) -> (String, Vec<EditResult>);
```

Each edit is a `str::replacen(..., 1)` — replaces first occurrence only. If the search string is not found, that edit is recorded as failed but others still apply.

### Processing flow changes

`process_file` needs to handle two triggers:

**Trigger 1: New directive (existing flow + edit proposal)**
1. Find unprocessed directives (as today).
2. Call `client.complete(...)`.
3. Parse response with `edit::parse_edits()`.
4. If edits found: write them as `<magent-edit status="proposed">` inside `<magent-response>`. Document is NOT modified yet.
5. If no edits found: write plain text response in `<magent-response>` (today's behavior).

**Trigger 2: Edit acceptance**
1. Find `<magent-response>` blocks containing `<magent-edit status="accepted">`.
2. For each accepted edit, apply it to the document.
3. Update the status to `applied` (or `failed` if search text not found).
4. Write the file with both the document changes and updated statuses.

### Failure modes

| Failure | Behavior |
|---------|----------|
| Search string not found on apply | Set that edit's status to `failed`. Other edits still apply. |
| Malformed edit blocks in LLM response | Treat entire response as plain text (degrade to current behavior). |
| All edits fail on apply | All statuses set to `failed`. Document unchanged. |
| LLM returns no edit blocks | Current behavior — plain text response only. |
| User deletes an edit block before accepting | That edit is simply gone — not applied. |

## Non-goals

- **Undo/revert mechanism**: users have git. We don't need to build rollback.
- **Multi-file edits**: out of scope. One directive, one file.
- **Full-document rewrite mode**: could be added later if search/replace proves insufficient for some use cases, but not in initial scope.
- **Accept-all shortcut**: user must accept edits individually (or a tool can bulk-accept). We don't build bulk-accept into the daemon.
- **Reject action**: deleting an edit block is sufficient. No explicit "rejected" status needed.

## Task breakdown

### PR 1: Migrate response markers to custom elements

**Changes:**
- Update `parser.rs` to recognize `<magent-response>` / `</magent-response>` instead of `<!-- magent:start -->` / `<!-- magent:end -->`
- Update `writer.rs` to emit the new tags
- Update system prompt in `llm.rs` (no references to old markers)
- Update ADR-001 to document the new marker format

**Acceptance criteria:**
- Parser detects directives as processed when followed by `<magent-response>` blocks
- Writer emits `<magent-response>` / `</magent-response>` tags
- All existing tests updated and passing
- Old `<!-- magent:start/end -->` markers no longer recognized (clean break)

### PR 2: Edit block parser (`edit.rs`)

**Changes:**
- New `edit.rs` module with `Edit` struct and `parse_edits()` function
- Parses `<magent-edit>` / `<magent-search>` / `<magent-replace>` blocks from LLM response text
- Returns `(Vec<Edit>, String)` — edits and remaining text (summary)
- Also: `parse_edit_blocks()` for reading status attributes from existing response blocks

**Acceptance criteria:**
- Correctly parses single and multiple edit blocks
- Preserves non-edit text as the summary
- Malformed blocks are treated as plain text (no edits extracted)
- Status attribute parsing works for all states (proposed, accepted, applied, failed)
- Comprehensive unit tests covering edge cases

### PR 3: Edit application logic

**Changes:**
- `apply_edits()` function in `edit.rs`
- Takes document content and a list of edits, returns modified content + per-edit results
- Each edit is a `str::replacen(..., 1)` — replaces first occurrence only

**Acceptance criteria:**
- Applies single and multiple edits correctly
- Reports success/failure per edit
- Failed edits don't prevent other edits from applying
- Order of application is stable (first edit in list applied first)
- Unit tests for all cases including partial failures

### PR 4: Wire edit proposals into the processing loop

**Changes:**
- Update system prompt in `llm.rs` to include edit format instructions
- Update `process_file` in `lib.rs`: when LLM response contains edit blocks, write them as `status="proposed"` inside `<magent-response>`
- When no edit blocks, behavior identical to today

**Acceptance criteria:**
- A directive that gets an edit response writes proposed edits in the response block
- Document content is NOT modified at this stage
- When the LLM returns plain text, behavior is identical to today
- Integration tests with fake LLM returning edit blocks

### PR 5: Edit acceptance processing

**Changes:**
- Extend `process_file` to detect `status="accepted"` edits in existing response blocks
- Apply accepted edits to document content
- Update status to `applied` or `failed`
- Single atomic file write

**Acceptance criteria:**
- Changing `status="proposed"` to `status="accepted"` triggers edit application
- Successfully applied edits update to `status="applied"`
- Failed edits (search text not found) update to `status="failed"`
- Partial success works (some applied, some failed)
- Integration tests covering the full propose → accept → apply lifecycle
