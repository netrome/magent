# Edit Tool Robustness

**Status:** Draft

## Goal

Make the edit tool more reliable when driven by LLMs. Three changes: switch from git conflict markers to XML-style tags, add whitespace-tolerant matching, and return actionable error messages when a search fails.

## Problems

The edit tool is the hardest tool for LLMs to use correctly. Three independent issues contribute:

1. **The input format fights LLM training.** Git conflict markers (`<<<<<<< SEARCH`, `=======`, `>>>>>>> REPLACE`) are noise in training data — models learn to *remove* them, not produce them. The `=======` divider is also ambiguous if the search or replace text contains that string. This is a regression: the previous edit mechanism used XML tags, which LLMs produce reliably.

2. **Whitespace mismatches cause silent failures.** LLMs frequently get leading/trailing whitespace wrong — extra spaces, different indentation width, trailing spaces. The search text is semantically correct but doesn't byte-match the file, so the edit fails.

3. **Error messages are opaque.** When a search block fails, the error is "search text not found" with no indication of *why*. The LLM has no signal to self-correct, so it often retries with the same mistake or gives up.

## Design

### 1. Tag-based input format

Replace conflict markers with `<search>`/`<replace>` tags:

**Current:**
```
path/to/file.md
<<<<<<< SEARCH
exact text to find
=======
replacement text
>>>>>>> REPLACE
```

**Proposed:**
```
path/to/file.md
<search>
exact text to find
</search>
<replace>
replacement text
</replace>
```

Multiple blocks by repeating the tag pairs:

```
path/to/file.md
<search>
first old
</search>
<replace>
first new
</replace>
<search>
second old
</search>
<replace>
second new
</replace>
```

**Why this works inside `<magent-input>`:** The outer parser uses simple `find()` for its open/close tags. Inner `<search>` / `<replace>` tags are opaque text to it — no conflict. The only forbidden string inside `<magent-input>` is `</magent-input>` itself. The original design doc (009-file-operations.md) raised a concern about "nesting XML inside XML", but this is a non-issue given the string-based parsing.

**Edge case:** If file content contains the literal string `</search>` or `</replace>`, the parser would close the tag prematurely. Same class of issue the conflict-marker format has with `=======`. Extremely unlikely in a markdown knowledge base — no special handling needed.

### 2. Whitespace-tolerant matching

When an exact `find()` fails, fall back to whitespace-normalized matching:

1. Strip leading and trailing whitespace from each line in both the search text and the file content.
2. Find the normalized search text within the normalized file content.
3. Map the match position back to the original file content and replace that range.

This means an LLM can write:

```
<search>
  fn main() {
    println!("hello");
  }
</search>
```

And it will match regardless of whether the actual indentation uses spaces, tabs, or a different width — as long as the non-whitespace content of each line matches.

**Boundaries:**
- Line count must still match — this does not ignore line breaks.
- Non-whitespace content must be identical — this is not fuzzy matching.
- Exact match is always attempted first. Whitespace tolerance is a fallback only.

### 3. Actionable error messages

When both exact and whitespace-tolerant matching fail, find the most similar region in the file and include it in the error:

1. Slide a window (same line count as the search text) over the file.
2. Score each window by counting lines where the trimmed content matches.
3. If the best match exceeds a threshold (e.g., >50% of lines match), include it:

```
Block 1: search text not found
  Best match (3/5 lines) near line 42:
    fn main() {
        let x = 1;
        let y = 2;
        println!("{}", x + y);
    }
```

This gives the LLM the actual file content at the closest point, so it can adjust its next attempt. No edit distance or fuzzy string algorithms — just line-by-line trimmed equality within a sliding window.

## Task breakdown

### PR 1: Tag-based input format

Switch `parse_blocks()` from conflict markers to `<search>`/`<replace>` tags, and update all callers and docs.

**Changes:**
- `src/tools/edit.rs` — rewrite `parse_blocks()` to scan for `<search>`/`</search>` + `<replace>`/`</replace>` tags instead of conflict markers.
- `src/llm.rs` — update the edit tool section in the system prompt to show the new format.
- `src/lib.rs` — update the edit tool call in the integration test.
- `src/tools/edit.rs` — update all `parse_blocks` and `execute` unit tests.

**Acceptance criteria:**
- `parse_blocks` correctly parses single, multiple, and multiline `<search>`/`<replace>` blocks.
- `parse_blocks` returns an error for malformed input (missing closing tag, missing `<replace>` after `<search>`).
- `parse_blocks` returns empty vec when no tags are present.
- Empty `<replace></replace>` works for deletion.
- All existing `execute` tests pass with the new format (same behavior, different delimiters).
- System prompt shows the tag format.
- Integration test uses the tag format.

### PR 2: Whitespace-tolerant matching and actionable error messages

Add fallback matching that tolerates whitespace differences, and improve error messages when matching fails entirely.

**Changes:**
- `src/tools/edit.rs` — add whitespace-normalized fallback in the matching logic within `execute()`: strip leading/trailing whitespace per line from both search text and file content, find the match in normalized form, map back to original byte offsets.
- `src/tools/edit.rs` — add `find_best_match()` helper: slide a window over the file lines, score by trimmed line equality, return the best region if it exceeds a threshold.
- `src/tools/edit.rs` — when a search block fails, call `find_best_match()` and include the result in the error detail.
- `src/tools/edit.rs` — unit tests for both features.

**Acceptance criteria:**
- Exact match is still attempted first and preferred.
- Whitespace-tolerant match succeeds when only leading/trailing whitespace per line differs (indentation, trailing spaces, tabs vs spaces).
- Whitespace-tolerant match fails when non-whitespace content differs or line count doesn't match.
- When both matching strategies fail and a similar region exists (>50% line match), error includes "Best match (N/M lines) near line L:" with the actual file content.
- When no similar region exists, error is the plain "search text not found" (no misleading suggestion).
- Existing tests unaffected (they use exact matches, which still take priority).

## What doesn't change

- `EditTool::execute()` flow (path resolution, sequential application, partial success reporting).
- `parse_input()` (first line is still the path, rest is passed to `parse_blocks()`).
- Tool dispatch in `lib.rs`.
- All other tools.

## Non-goals

- No changes to `write`, `move`, `delete`, `search`, or `read` tools.
- No changes to the outer `<magent-tool-call>` / `<magent-input>` parsing.
- No fuzzy/approximate string matching — whitespace tolerance is the only leniency.
