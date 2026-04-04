# File Operations

**Status:** Proposed

## Goal

Enable magent to create, edit, move/rename, and delete files anywhere within the knowledge base — not just the document containing the `@magent` directive.

Today, magent can only modify the current document via `<magent-edit>` blocks, which require user acceptance before application. This means a directive like "create a new page summarizing these notes" or "rename this file to match its title" can only produce a suggestion in the response block — the user must act on it manually.

We want:
```markdown
@magent Create a new file `recipes/pasta.md` with a basic carbonara recipe
```

To actually create that file, without a manual accept step.

## Motivation

- **Edit other files**: The most common missing capability. "Update the index page to link to this new section" requires editing a different file.
- **Create files**: Agents should be able to scaffold new documents, split large files, or generate summaries.
- **Move/rename files**: Reorganizing a knowledge base is a natural agent task.
- **Delete files**: Cleaning up obsolete content.

## Constraints

- **Knowledge base boundary**: All operations must stay within the configured root directory. Path traversal must be prevented (same as existing search/read tools).
- **Git is the safety net**: The knowledge base is assumed to be in a git repo. Users can `git diff`, `git checkout`, or `git stash` to review and revert changes. This replaces the need for a manual accept step.
- **Markdown is the interface**: File operations and their results are visible in the response block, making them auditable.
- **No new dependencies**: These are basic filesystem operations — stdlib is sufficient.

## Key design decision: tools, not edit blocks

There are two possible approaches:

### Option A: Extend `<magent-edit>` to target other files

Add a `file="path"` attribute to `<magent-edit>` blocks. Keep the propose/accept lifecycle.

**Pros:**
- Consistent with existing edit mechanism.
- User reviews every change before it happens.

**Cons:**
- The accept step adds significant friction for multi-file operations. Imagine a directive that creates 3 files and edits 2 others — the user must accept 5+ edit blocks.
- `<magent-edit>` was designed for search/replace within a known document. Creating, moving, and deleting files don't fit the search/replace model.
- The accept step was originally motivated by safety ("no surprise rewrites"). But with git, the safety net already exists and is more powerful (full history, diff, revert).

### Option B: New tools that execute immediately

Add `write`, `move`, and `delete` as tools alongside `search` and `read`. They execute during the tool-use loop, just like search and read. Results are written to the response block in `<magent-tool-result>` tags, providing full auditability.

**Pros:**
- Follows the established tool pattern — no new mechanisms to build.
- Immediate execution enables multi-step workflows (create a file, then read it back to verify, then edit another file to link to it).
- Simpler implementation — reuses tool dispatch, tool result formatting, and the existing tool-use loop.
- Full auditability via `<magent-tool-result>` blocks in the response.
- Git provides a stronger safety net than the accept step (history, diff, selective revert).

**Cons:**
- Changes are applied immediately — no review before execution. Mitigated by git.
- A misbehaving model could write garbage files. Mitigated by: (1) git revert, (2) operations scoped to knowledge base root, (3) tool results showing exactly what happened.

### Recommendation: Option B (tools)

The tool approach is simpler, more composable, and consistent with how magent already handles search, read, and browser. The accept step made sense when edits were the only write mechanism and there was no undo story — but with git as the assumed safety net, immediate execution is the better trade-off.

## Removing `<magent-edit>` blocks

The existing `<magent-edit>` mechanism (propose/accept for the current document) should be removed entirely in favor of the `edit` tool.

**Why not keep both?**
- Having two ways to edit files — `<magent-edit>` for the current document and the `edit` tool for other files — is confusing for both the LLM and humans reading the output.
- It's the same operation with different syntax and different lifecycles. One mechanism is simpler.
- The `edit` tool is strictly more capable: it works on any file, including the current one.
- Removing `<magent-edit>` eliminates a significant chunk of code: the edit parser, `format_proposed_edits`, `process_accepted_edits`, and the entire propose/accept lifecycle.
- It reinforces a clean separation: tools perform side effects, response prose explains what happened.

The LLM can edit the current document by using the `edit` tool with the current file's path. The tool result in the response block still shows exactly what changed, preserving auditability.

## Tool designs

### `write` — Create or overwrite a file

**Input format:**
```
path/to/file.md
---
file content here
```

First line is the relative path. `---` separator. Everything after is the file content.

**Behavior:**
- Creates the file (and any intermediate directories).
- If the file already exists, overwrites it.
- Returns confirmation with the path and byte count.

**Why not separate `create` and `overwrite`?** A single `write` tool is simpler. The LLM can use `read` first to check if a file exists when it matters.

### `edit` — Search-and-replace in any file

**Input format:**
```
path/to/file.md
<<<<<<< SEARCH
exact text to find
=======
replacement text
>>>>>>> REPLACE
```

First line is the relative path. Then one or more search/replace blocks.

**Behavior:**
- Reads the target file.
- Applies each search/replace (first occurrence only, like current `<magent-edit>`).
- Writes the modified file.
- Returns per-block results (applied/failed).

**Why this format?** The conflict-marker style is well-known (git uses it), unambiguous to parse, and handles multi-line content naturally. It avoids nesting XML inside XML (which would happen if we reused `<magent-search>`/`<magent-replace>` inside `<magent-input>`).

### `move` — Move or rename a file

**Input format:**
```
old/path.md -> new/path.md
```

**Behavior:**
- Moves/renames the file (creates intermediate directories if needed).
- Returns confirmation with old and new paths.
- Fails if source doesn't exist or destination already exists.

### `delete` — Delete a file

**Input format:**
```
path/to/file.md
```

**Behavior:**
- Deletes the file.
- Returns confirmation.
- Fails if file doesn't exist.
- Only deletes files, not directories (prevents accidental recursive deletion).

## Path validation

All four tools share the same path validation as the existing search and read tools:

1. Resolve the path relative to the knowledge base root.
2. Canonicalize and verify it's within root (prevents `../` traversal).
3. Reject absolute paths.

This logic should be extracted into a shared utility since it's now used by 6 tools.

## System prompt changes

- Add documentation for the four new tools in the `=== TOOLS ===` section, following the same format as search/read/browser.
- Remove the `<magent-edit>` format instructions and examples from the system prompt.
- The system prompt should make clear that the `edit` tool is how the LLM modifies any file, including the current document.

## What this looks like in practice

### Example: Create a file and update an index

```markdown
@magent Create a new page `guides/getting-started.md` with a quick-start
guide, and add a link to it from `index.md`.
```

```markdown
<magent-response>
<magent-thinking>
I need to: 1) Create the new guide file, 2) Read index.md to find where to
add the link, 3) Edit index.md to add it.
</magent-thinking>

I'll create the guide and update the index.

<magent-tool-call tool="write">
<magent-input>guides/getting-started.md
---
# Getting Started

Welcome! This guide walks you through setting up your environment.

## Prerequisites
...
</magent-input>
</magent-tool-call>

<magent-tool-result tool="write">
Created guides/getting-started.md (187 bytes)
</magent-tool-result>

<magent-tool-call tool="read">
<magent-input>index.md</magent-input>
</magent-tool-call>

<magent-tool-result tool="read">
  1 | # Knowledge Base
  2 |
  3 | ## Guides
  4 |
  5 | - [Installation](guides/installation.md)
  6 | - [Configuration](guides/configuration.md)
</magent-tool-result>

<magent-tool-call tool="edit">
<magent-input>index.md
<<<<<<< SEARCH
- [Configuration](guides/configuration.md)
=======
- [Configuration](guides/configuration.md)
- [Getting Started](guides/getting-started.md)
>>>>>>> REPLACE
</magent-input>
</magent-tool-call>

<magent-tool-result tool="edit">
Applied 1/1 edits to index.md
</magent-tool-result>

Done! Created the getting-started guide and added it to the index.
</magent-response>
```

### Example: Rename a file

```markdown
@magent Rename `notes/draft.md` to `notes/api-design.md`
```

```markdown
<magent-response>
<magent-tool-call tool="move">
<magent-input>notes/draft.md -> notes/api-design.md</magent-input>
</magent-tool-call>

<magent-tool-result tool="move">
Moved notes/draft.md -> notes/api-design.md
</magent-tool-result>

Renamed the file.
</magent-response>
```

## Non-goals

- **Directory operations**: No `mkdir`, `rmdir`, or recursive delete. `write` creates intermediate directories implicitly. Keeping scope minimal.
- **Binary files**: All tools assume text (UTF-8). Binary file operations are out of scope.
- **Undo mechanism**: Git handles this.
- **Concurrent file access**: Magent is single-threaded. No locking needed.
- **File permissions**: Not relevant for a markdown knowledge base.

## Task breakdown

### PR 1: Shared path validation utility

Extract the path resolution and boundary-checking logic that's duplicated in `search.rs` and `read.rs` into a shared function in `tools/mod.rs` (or a `tools/path.rs` module).

**Acceptance criteria:**
- Shared `resolve_path(root: &Path, relative: &str) -> Result<PathBuf, String>` function.
- Search and read tools refactored to use it.
- All existing tests still pass.
- Unit tests for the shared function (traversal rejection, relative resolution, etc.).

### PR 2: `write` tool

**Changes:**
- New `tools/write.rs` with `WriteTool` struct and `.execute()` method.
- Parse input format (path + separator + content).
- Create intermediate directories, write file.
- Add match arm in `execute_tool()`.
- Add tool documentation to system prompt.

**Acceptance criteria:**
- Creates new files with correct content.
- Creates intermediate directories as needed.
- Overwrites existing files.
- Rejects paths outside knowledge base root.
- Returns confirmation message with path and size.
- Unit tests for all cases.

### PR 3: `edit` tool

This is the sole edit mechanism, replacing `<magent-edit>` blocks for all files including the current document.

**Changes:**
- New `tools/edit.rs` with `EditTool` struct and `.execute()` method.
- Parse conflict-marker-style search/replace blocks.
- Read file, apply edits, write file.
- Add match arm and system prompt docs.

**Acceptance criteria:**
- Applies single and multiple search/replace blocks.
- Reports per-block success/failure.
- Fails gracefully when file doesn't exist.
- Rejects paths outside knowledge base root.
- Unit tests.

### PR 4: `move` tool

**Changes:**
- New `tools/move.rs` with `MoveTool` struct and `.execute()` method.
- Parse `old -> new` input format.
- Validate both paths, rename file, create target directories.
- Add match arm and system prompt docs.

**Acceptance criteria:**
- Moves/renames files correctly.
- Creates intermediate directories for target.
- Fails if source doesn't exist.
- Fails if destination already exists.
- Rejects paths outside knowledge base root (both source and target).
- Unit tests.

### PR 5: `delete` tool

**Changes:**
- New `tools/delete.rs` with `DeleteTool` struct and `.execute()` method.
- Validate path, delete file.
- Add match arm and system prompt docs.

**Acceptance criteria:**
- Deletes files.
- Fails if file doesn't exist.
- Only deletes files, not directories.
- Rejects paths outside knowledge base root.
- Unit tests.

### PR 6: Remove `<magent-edit>` mechanism

**Changes:**
- Remove `edit.rs` (the edit parser, `format_proposed_edits`, `process_accepted_edits`, edit status types).
- Remove `<magent-edit>` format instructions from the system prompt in `llm.rs`.
- Remove `process_accepted_edits()` call from `process_file()` in `lib.rs`.
- Remove `format_response()` helper (no longer needed — LLM responses are written as-is).
- Remove or update all tests related to `<magent-edit>` blocks.

**Acceptance criteria:**
- No `<magent-edit>`, `<magent-search>`, `<magent-replace>` references in the codebase.
- `edit.rs` module removed.
- The `edit` tool (from PR 3) is the only way to modify files.
- All remaining tests pass.

### PR 7: End-to-end integration test

**Changes:**
- Integration test that exercises a multi-tool workflow: create a file, read it back, edit it, move it, verify final state.

**Acceptance criteria:**
- Test covers the full tool chain.
- Verifies file system state after each operation.
- Verifies response block contains expected tool call/result sequence.
