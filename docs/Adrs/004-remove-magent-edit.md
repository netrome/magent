# ADR-004: Remove `<magent-edit>` in Favor of Edit Tool

**Status:** Proposed
**Date:** 2026-04-04

## Context

Magent currently has two mechanisms for modifying file content:

1. **`<magent-edit>` blocks** — the LLM proposes search/replace edits inline in its response. These are written with `status="proposed"`, the user changes the status to `status="accepted"`, and the daemon applies them. Only works on the current document.

2. **Tools** (`search`, `read`, `browser`) — the LLM calls tools during the tool-use loop. These execute immediately and results are written to the response block.

Project 009 (file operations) adds write, edit, move, and delete tools that execute immediately, with git as the safety net instead of a manual accept step. The `edit` tool uses conflict-marker-style search/replace blocks and works on any file.

This creates a problem: two ways to edit files, with different syntax and different lifecycles. The `<magent-edit>` mechanism uses XML tags with a propose/accept workflow; the `edit` tool uses conflict markers with immediate execution. Both do the same thing.

## Decision

Remove the `<magent-edit>` mechanism entirely. The `edit` tool becomes the sole way to modify files, including the current document.

### Why

- **One mechanism, not two.** Having two edit systems with different syntax and lifecycles is confusing for both the LLM and users reading the output.
- **Git is the safety net.** The propose/accept step was designed when there was no undo story. With the assumption that the knowledge base is a git repo, `git diff` and `git checkout` provide a stronger safety net than in-file acceptance.
- **The `edit` tool is strictly more capable.** It works on any file. `<magent-edit>` only works on the current document.
- **Clean separation of concerns.** Tools perform side effects; response prose explains what happened. `<magent-edit>` blurred this line by embedding actionable state transitions in the response text.
- **Less code.** Removes `edit.rs` (parser, application logic, status lifecycle), the `process_accepted_edits` flow in the main loop, and `format_response`.

### What gets removed

- `src/edit.rs` — edit block parser, `process_accepted_edits`, `format_proposed_edits`, `EditStatus` enum.
- `<magent-edit>` / `<magent-search>` / `<magent-replace>` format instructions from the system prompt.
- `process_accepted_edits()` call in `process_file()`.
- `format_response()` helper in `lib.rs`.
- All tests related to the edit block lifecycle.

## Consequences

### Positive

- Simpler codebase — one edit mechanism instead of two.
- Consistent model: all side effects go through tools.
- No more propose/accept lifecycle to reason about.
- The `edit` tool's conflict-marker format avoids the XML-in-XML nesting that `<magent-edit>` would have inside `<magent-tool-result>` blocks.

### Negative

- Editing the current document now requires a tool call round-trip instead of inline edit blocks. This adds one LLM turn of latency per edit. Acceptable trade-off for the simplicity gain.
- The LLM needs to know the current file's path (added to the system prompt's document header).
- Existing documents with `<magent-edit>` blocks will have inert markup. This is a young project with few existing files, so migration cost is negligible.
