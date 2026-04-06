# Magent

A markdown-native AI agent that runs as a background process alongside your knowledge base. You interact with it by writing directives directly in your markdown files.

## Core idea

Magent watches a directory of markdown files. When you write an `@magent` directive in any file, it picks up the instruction, interacts with an LLM, and writes the result back into the same file. The same mechanism handles one-off questions, document edits, and multi-step tasks.

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

### Multi-step tasks

Magent can use tools autonomously to answer questions and make changes. For example, a directive like `@magent fix the broken URLs in this file` will cause the agent to read the file, identify the issues, and apply edits — all within one response cycle.

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
- Use a single `magent-*` tag vocabulary for responses and future features

## Tools

Magent can use tools to answer directives more effectively:
- **Search** — find relevant files in the knowledge base by keyword or regex
- **Read** — read the contents of a specific file
- **Write** — create or overwrite a file
- **Edit** — search-and-replace within any file (supports whitespace-tolerant matching)
- **Move** — move or rename a file
- **Delete** — delete a file
- **Browser** — interact with web pages via a headless browser (when available)

When a directive requires context beyond the current document, magent searches and reads files on its own, then incorporates what it finds into its response. No special syntax needed — the agent decides when to use tools based on the question.

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

## Planned

- Config file (`.magent/config.toml`) for model defaults and watched paths
- Scheduled directives (`in:`, `at:`) for delayed and recurring tasks
- Per-directive model selection (`@magent(model:claude) ...`)
