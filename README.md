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

<magent-response>
<magent-thinking>
The user is asking about sky color. This is explained by Rayleigh scattering.
</magent-thinking>
Rayleigh scattering — shorter (blue) wavelengths of sunlight are
scattered more by the atmosphere than longer wavelengths.
</magent-response>
```

### Document edit

When a directive asks for changes, magent proposes edits using search-and-replace blocks. You review them, accept the ones you want, and magent applies them to the document.

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

Change `status="proposed"` to `status="accepted"` on the edits you want, and magent applies them on the next save. Edits you don't want can simply be deleted.

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

Magent wraps its output in custom elements to track what has been processed:

```markdown
<magent-response>
Agent response here.
</magent-response>
```

This allows magent to:
- Know which directives have already been handled (skip if followed by a response block)
- Clearly delimit agent-written content from your own writing
- Use a single `magent-*` tag vocabulary for responses, edits, and future features

## Tools

Magent can use tools to answer directives more effectively:
- **Search** — find relevant files in the knowledge base by keyword
- **Read** — read the contents of a specific file

When a directive requires context beyond the current document, magent searches and reads files on its own, then incorporates what it finds into its response. No special syntax needed — the agent decides when to use tools based on the question.

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

## Quick start (Ollama)

1. Install and start [Ollama](https://ollama.com/download), then pull a model:

```sh
ollama pull llama3
```

2. Build and run magent, pointing it at a directory of markdown files:

```sh
cargo run -- watch ./notes
```

This uses Ollama's default endpoint (`http://localhost:11434/v1`) and `llama3`. To use a different model or API:

```sh
cargo run -- watch ./notes --model mistral --api-url http://localhost:11434/v1
```

For hosted providers that require an API key:

```sh
export MAGENT_API_KEY=sk-...
cargo run -- watch ./notes --api-url https://api.openai.com/v1 --model gpt-4o
```

3. Add a directive to any `.md` file in the watched directory and save:

```markdown
@magent what is the capital of France?
```

Magent picks it up and writes the response inline:

```markdown
@magent what is the capital of France?

<magent-response>
<magent-thinking>
The user is asking about the capital of France.
</magent-thinking>
Paris.
</magent-response>
```

Press `Ctrl+C` to stop the daemon.

## Principles

- **Markdown is the interface.** No GUI, no database. Files in, files out.
- **Inline by default.** You talk to the agent where you're already working.
- **Composable.** Works alongside any editor, note-taking tool, or git workflow.
- **Minimal.** The agent orchestrates — it doesn't try to be a note-taking app.
- **Transparent.** All agent activity is visible as files you can read, diff, and version.
