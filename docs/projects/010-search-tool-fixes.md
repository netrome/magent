# Search Tool Fixes

**Status:** Accepted

## Goal

Fix two bugs in the search tool that prevent the LLM from finding content in the knowledge base.

## Problem

### 1. Argument order breaks option parsing

The `parse_input` function (`src/tools/search.rs:73`) is a greedy prefix parser: it consumes `key:value` tokens from the front and stops at the first unrecognized token. When the LLM sends:

```
KZG path:. max:50
```

`KZG` is not a recognized prefix, so parsing stops immediately. The entire string becomes the regex query. No path filter is applied, no max limit is set.

The parser expects `path:. max:50 KZG`, but LLMs naturally put the search term first. Prompting the LLM to use the "correct" order is fragile — it will drift.

### 2. Search finds the agent's own conversation

The agent's response (including tool calls) is written to the task file in real-time via the writer. When the search tool runs, it reads all `.md` files — including the file containing the current conversation. The text `KZG path:. max:50` appears as a literal `<magent-input>` line and matches the query, so the agent finds its own tool calls instead of the actual content.

Each retry compounds the problem: every search adds another matching line to the conversation file.

## Fixes

### Fix 1: Position-independent option parsing

Change `parse_input` to scan all whitespace-separated tokens, extract recognized `key:value` pairs from any position, and join the remaining tokens as the query.

Before:
```
KZG path:. max:50  =>  query="KZG path:. max:50", path=None, max=20
```

After:
```
KZG path:. max:50  =>  query="KZG", path=Some("."), max=50
```

This is a drop-in change to `parse_input` only. The rest of the search pipeline is unaffected.

### Fix 2: Exclude the active task file from search

Pass the current file path to `execute_tool`, and from there to `SearchTool`. Skip that file during search.

This requires:
- Adding an `exclude` field to `SearchTool` (the path of the active task file).
- Threading the current file path from `process_directive` through to `execute_tool`.

## Non-goals

- Changing the tool input format (e.g., structured key-value). The current format works once parsing is correct.
- Excluding all `<magent-response>` content from search. The file-level exclusion is sufficient and simpler.
- Adding new search features (sorting, file-type filters, etc.).

## Task breakdown

### PR 1: Position-independent option parsing

**Changes:**
- Rewrite `parse_input` in `src/tools/search.rs` to collect tokens into options or query parts regardless of position.
- Update existing `parse_input` tests to cover mixed-order inputs.

**Acceptance criteria:**
- `KZG path:. max:50` parses as query=`KZG`, path=`.`, max=50.
- `path:notes/ KZG max:10` parses identically to `KZG path:notes/ max:10`.
- `path:notes/ something max:10` parses as query=`something`, path=`notes/`, max=10 (regression vs. current behavior where `something max:10` became the query).
- All existing search tests still pass.

### PR 2: Exclude active task file from search

**Changes:**
- Add an `exclude: Option<PathBuf>` field to `SearchTool`.
- Skip the excluded file in `search_files` (or `collect_files`).
- Thread the current file path from `process_directive` through `execute_tool` to `SearchTool::new`.

**Acceptance criteria:**
- Search does not return matches from the file containing the active `@magent` directive.
- Search still works normally for all other files.
- Unit test: create two files, one "excluded", verify only the other appears in results.
- Integration test (or extend existing): verify end-to-end that search skips the active file.
