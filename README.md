# Magent

A markdown-native AI agent that runs as a background process alongside your knowledge base. You interact with it by writing directives directly in your markdown files.

## Core idea

Magent watches a directory of markdown files. When you write an `@magent` directive in any file, it picks up the instruction, interacts with an LLM, and writes the result back into the same file. The same mechanism handles one-off questions, document edits, and recurring tasks.

The tool focuses only on agent orchestration. It does not edit, render, or manage your notes beyond responding to directives — it reads and writes markdown files that any other tool can work with.

## Directives

A directive is an `@magent` mention anywhere in a markdown file. When magent sees an unprocessed directive, it executes it and writes the response.

### Simple question

```markdown
@magent why is the sky blue?

<!-- magent:start -->
Rayleigh scattering — shorter (blue) wavelengths of sunlight are
scattered more by the atmosphere than longer wavelengths.
<!-- magent:end -->
```

### Document edit

```markdown
# Shopping list

- ~~eggs~~
- milk
- bread
- ~~butter~~

@magent clean up this list and add ingredients for tacos

<!-- magent:start -->
Removed completed items. Added taco ingredients:
<!-- magent:end -->

- milk
- bread
- ground beef
- tortillas
- salsa
- cheese
- lime
```

### Scheduled / recurring

```markdown
@magent(in:1h) check housing listings in Södermalm and update the summary below
```

Magent processes this when the time is due. For recurring tasks, the agent re-schedules itself by writing a new directive after completing the work — no separate cron system needed.

### Timing syntax

- `@magent` — process immediately on file change
- `@magent(in:1h)` — process after a delay
- `@magent(at:2026-03-15T18:00)` — process at a specific time

## Response markers

Magent wraps its output in HTML comments to track what has been processed:

```markdown
<!-- magent:start -->
Agent response here.
<!-- magent:end -->
```

This allows magent to:
- Know which directives have already been handled (skip if followed by a response block)
- Clearly delimit agent-written content from your own writing
- Stay invisible in most markdown renderers

## File structure

Magent needs minimal infrastructure in your knowledge base:

```
knowledge-base/
  .magent/
    config.toml       # model defaults, API keys, watched paths
    state.json        # tracks processed directives, pending schedules
  shopping.md         # your notes — directives live inline
  reading-list.md
  house-search.md
  ...
```

No task directory, no conversation directory. Everything lives in your existing files.

## Model support

Magent supports multiple backends:
- **Local models** via Ollama — good for simple, recurring, background tasks
- **OpenAI API** and compatible APIs

The model can be configured with a TOML file provided at startup or per-directive:

```markdown
@magent(model:claude) explain the trade-offs of this approach
```

## Runtime

Magent runs as a persistent background process:
- Watches markdown files for new or changed `@magent` directives
- Tracks scheduled directives and fires them when due
- Writes responses back into the originating file
- All activity is visible as file changes

## Principles

- **Markdown is the interface.** No GUI, no database. Files in, files out.
- **Inline by default.** You talk to the agent where you're already working.
- **Composable.** Works alongside any editor, note-taking tool, or git workflow.
- **Minimal.** The agent orchestrates — it doesn't try to be a note-taking app.
- **Transparent.** All agent activity is visible as files you can read, diff, and version.
