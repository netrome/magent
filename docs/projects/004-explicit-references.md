# Explicit References

**Status:** Complete

## Goal

Allow a directive to include content from other files in the knowledge base as additional context. Today, the LLM only sees the file containing the directive. For cross-document tasks ("compare these two files", "update this summary based on my rust notes"), the user has no way to point the agent at other files.

We want:
```markdown
@magent(context: rust.md, go.md) compare error handling approaches
```

The agent sees the current document plus the full content of `rust.md` and `go.md`, and can reason across all of them.

## Constraints

- **No magic**: the user explicitly names the files. No automatic link-following, no scanning the whole KB.
- **Markdown is the interface**: file references live in the directive syntax, no config files or sidecars.
- **Fail visibly**: if a referenced file doesn't exist, the error shows up in the response — not silently ignored.
- **No new dependencies**: this is plumbing between existing modules.

## How it works

1. The parser extracts the `context:` option from the directive.
2. `process_file` resolves the paths relative to the watched directory root.
3. Referenced files are read and assembled into an extended context string.
4. The extended context is passed to the LLM as the `document` parameter (which goes into the existing system prompt template).

The LLM sees the current document and referenced files in one prompt. No trait changes, no API changes — just a richer `document` string.

## Syntax

```markdown
@magent(context: path/to/file.md) your prompt here
@magent(context: file1.md, file2.md, subdir/file3.md) your prompt here
```

Paths are relative to the watched root directory. Whitespace around commas and filenames is trimmed.

Combined with other options (future):
```markdown
@magent(context: notes.md, model: claude) summarize the key themes
```

## Context assembly

The document string passed to the LLM is extended to include referenced files:

```
=== CURRENT DOCUMENT: house-search.md ===
(current file content)
=== END CURRENT DOCUMENT ===

=== REFERENCED: rust.md ===
(rust.md content)
=== END REFERENCED ===

=== REFERENCED: go.md ===
(go.md content)
=== END REFERENCED ===
```

When there are no explicit references, the document string stays as it is today (just the file content, no extra headers). This keeps the zero-reference case unchanged.

### System prompt update

The system prompt template gets a minor wording adjustment:

```
You are an AI assistant embedded in a markdown document. The user will ask
questions or request changes to the document below. Additional referenced
files may follow the main document — use them as context but only propose
edits to the main document.
```

The "only propose edits to the main document" instruction is important: the agent can read referenced files but shouldn't try to edit them (edit blocks target search strings in the current document).

## Error handling

| Situation | Behavior |
|-----------|----------|
| Referenced file doesn't exist | Write an error response: `**Error:** Referenced file not found: rust.md` |
| Referenced file is not within watched root | Write an error response: `**Error:** Referenced file is outside the knowledge base: ../secret.md` |
| Referenced file is the current file | Silently ignored (it's already in context) |
| No `context:` option | Today's behavior — current document only |

Path traversal (`../`) that escapes the watched root must be rejected. This is a security constraint from CLAUDE.md.

## Changes needed

### Parser (`parser.rs`)

The `Directive` struct gains an `options` field:

```rust
pub struct Directive {
    pub prompt: String,
    pub line: usize,
    pub processed: bool,
    pub options: HashMap<String, String>,
}
```

`extract_prompt` currently skips the `(options)` block. It needs to parse it into key-value pairs instead. The format is simple: `key:value` pairs separated by commas.

```
@magent(context: a.md, b.md, model: claude) prompt
        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
        options string: "context: a.md, b.md, model: claude"
```

Parsing challenge: commas separate both option pairs AND values within `context:`. Resolution: `context:` consumes all comma-separated values until the next `key:` token (a token containing a colon that isn't a file path). Simpler alternative: `context:` is always the last option, or uses a different separator.

**Recommended approach**: parse options as `key: value` pairs split on commas, where a new pair starts when the token contains a `:`. Everything between one `key:` and the next is that key's value.

```
"context: a.md, b.md, model: claude"
→ context = "a.md, b.md"
→ model = "claude"
```

### Processing (`lib.rs`)

In `process_file`, after parsing directives, resolve the `context:` option for each unprocessed directive:

```rust
// Pseudocode
let context_files = resolve_context_files(&directive.options, &root_dir)?;
let document = build_context_string(&content, path, &context_files);
let response = client.complete(&directive.prompt, Some(&document)).await?;
```

`resolve_context_files` returns `Vec<(String, String)>` (filename, content) after validation.

### LLM (`llm.rs`)

No changes to the trait. The system prompt template gets the minor wording update about referenced files. The `document` parameter simply carries a richer string when references are present.

## Non-goals

- **Glob patterns** (`context: notes/*.md`): useful but adds complexity. Follow-up.
- **Recursive references**: if `a.md` references `b.md` which references `c.md`, only `a.md`'s explicit references are included. No transitive expansion.
- **Editing referenced files**: the agent can read them for context, but edit blocks only target the current document.
- **Context size management**: if referenced files are huge, we pass them as-is. Token limits are the LLM's problem for now. Context windowing/summarization is a future feature.

## Task breakdown

### PR 1: Parse directive options

**Changes:**
- Add `options: HashMap<String, String>` to `Directive`
- Parse `(key: value, ...)` blocks in `extract_prompt`
- Return parsed options; unknown keys are stored but ignored

**Acceptance criteria:**
- `@magent(context: a.md, b.md) prompt` → options `{"context": "a.md, b.md"}`, prompt `"prompt"`
- `@magent(context: a.md, model: claude) prompt` → options `{"context": "a.md", "model": "claude"}`
- `@magent prompt` → empty options (existing behavior unchanged)
- Malformed options (unclosed paren) → still returns `None` as today

### PR 2: Resolve and assemble context

**Changes:**
- New function `resolve_context_files(options, root) -> Result<Vec<(String, String)>, ContextError>`
- New function `build_context_string(content, current_path, context_files) -> String`
- Path validation: reject paths outside root, skip self-references
- Wire into `process_file`

**Acceptance criteria:**
- Valid paths are resolved relative to watched root and their content is read
- Missing files produce an error response (not a panic)
- Path traversal outside root is rejected
- Self-reference is silently skipped
- When no context option, behavior is identical to today

### PR 3: System prompt update

**Changes:**
- Update system prompt wording to mention referenced files
- Update system prompt to label sections when references are present

**Acceptance criteria:**
- With references: document section has headers (`=== CURRENT DOCUMENT ===`, `=== REFERENCED ===`)
- Without references: document section is unchanged (no headers)
- Integration test: directive with context option passes all file contents to LLM
