# 014: Conditional Pre-Execution Flush

**Status:** Done

## Problem

When the LLM calls a file-editing tool (e.g. `edit`), the daemon flushes the in-progress response block to disk *before* executing the tool. This means the file being edited now contains the response block — including the LLM's `<search>` text echoed verbatim inside the tool call.

If the LLM's search text has even a minor whitespace difference from the original content (e.g. a missing trailing space), the edit tool's exact-match `find()` fails against the original content but succeeds against the identical echo inside the response block. The edit is applied to the response block instead of the target content. The post-execution flush then overwrites the response block with `full_response`, silently discarding the edit. The original content is never modified, but the tool reports success.

**Reproduction:** Any edit where the LLM's search text is not byte-identical to the file content (common with trailing whitespace, tabs vs spaces, etc.) on the same file as the directive.

## Solution

Classify tools as *slow* (potentially user-visible latency) or *fast* (instant file operations). Only flush the response block before execution for slow tools.

- **Slow tools** (flush before + after): `browser` — takes seconds, user benefits from seeing the tool call while waiting.
- **Fast tools** (flush after only): `search`, `read`, `write`, `edit`, `move`, `delete` — instant, no user benefit from a mid-execution flush. Skipping it avoids polluting the file with the response block before the edit runs.

The post-execution flush remains unconditional — once the tool result is appended, the response block content is harmless (the search text is no longer a standalone substring).

## Tasks

- [x] Add `is_slow_tool(name: &str) -> bool` helper in `lib.rs`
- [x] Make pre-execution flush conditional on `is_slow_tool` in `process_directive`
- [x] Add test: edit tool on the active file should modify the actual content, not the response block
