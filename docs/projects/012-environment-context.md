# 012: Environment Context

**Status:** Done

## Problem

The LLM currently receives the document content and tool descriptions, but lacks basic environmental context that humans take for granted:

- **No file path.** The agent doesn't know which file it's editing. It has to guess or infer the path when using the edit tool on the current document.
- **No date.** Models have a training cutoff and no sense of "now." This leads to hallucinated or outdated dates.
- **No knowledge base awareness.** The agent doesn't know what other files exist without first calling the search tool. For simple tasks ("link to the roadmap", "reference the style guide"), this wastes a tool call.

## Solution

Add a compact `=== ENVIRONMENT ===` section to the system prompt, injected before the document. It contains three pieces of information:

### 1. Current file path

The path of the file being processed, relative to the knowledge base root.

```
Current file: notes/rust.md
```

### 2. Current date

Today's date in ISO format.

```
Date: 2026-04-06
```

### 3. Knowledge base structure

Two file listings that orient the agent:

- **Top-level entries** — files and directories at the root of the knowledge base. Directories get a trailing `/`.
- **Sibling files** — files in the same directory as the current document (omitted when the current file is at the root, since it would duplicate the top-level listing).

```
Knowledge base:
  drafts/
  projects/
  notes.md
  roadmap.md

Same directory (notes/):
  rust.md
  python.md
  go.md
```

Entries are sorted alphabetically, directories first.

### Example system prompt section

```
=== ENVIRONMENT ===
Current file: notes/rust.md
Date: 2026-04-06

Knowledge base:
  drafts/
  projects/
  notes/
  roadmap.md

Same directory (notes/):
  go.md
  python.md
  rust.md
=== END ENVIRONMENT ===
```

## Design decisions

- **Filenames only, no descriptions.** The agent can `read` any file it wants. Extracting titles would add I/O and complexity for marginal benefit.
- **No recursive tree.** A deep listing would bloat the prompt for large knowledge bases. Top-level + siblings covers the common case. A future `ls` tool can handle deeper exploration.
- **Sorted, directories first.** Predictable ordering makes the listing easy to scan. Mirrors conventional `ls` behavior.
- **Omit sibling section at root.** When the current file is at the knowledge base root, the top-level listing already shows its siblings.

## Non-goals

- `ls` / `tree` tool (useful follow-up, separate project)
- File descriptions or metadata in the listing
- Recursive directory listings

## Implementation

### Changes

1. **`src/llm.rs`** — Add `EnvironmentContext` struct and a `{environment}` placeholder to `SYSTEM_PROMPT_TEMPLATE`. Update `build_system_prompt` to accept and format it.
2. **`src/context.rs`** — Add `build_environment` function that takes the root path, current file path, and date, and returns an `EnvironmentContext` with the formatted listing strings.
3. **`src/lib.rs`** — In `process_file`, gather environment info and pass it through to `build_system_prompt`.

### Task breakdown

- [x] **Add `EnvironmentContext` struct and update `build_system_prompt`** — New struct with `file_path`, `date`, `top_level`, `siblings` fields. Add `{environment}` placeholder to prompt template. Update `build_system_prompt` signature and all call sites. Add unit tests for prompt formatting.
- [x] **Implement environment gathering in `context.rs`** — `build_environment()` reads the root directory and current file's parent directory, builds sorted listings (dirs first, trailing `/`), skips sibling section when at root. Add unit tests with temp directories.
- [x] **Wire it up in `process_file`** — Call `build_environment()`, pass result to `build_system_prompt`. Update existing integration tests that call `build_system_prompt` or `process_directive`.
