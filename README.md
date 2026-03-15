# Magent

A markdown-native AI agent that runs as a background process alongside your knowledge base. Everything is governed by markdown files — tasks, conversations, and outputs.

## Core idea

Magent watches a directory of markdown files for changes. When it detects a change that requires action (a new message in a conversation, a scheduled task coming due), it runs the appropriate task against an LLM and writes the result back as markdown.

The tool focuses only on agent orchestration. It does not edit, render, or manage your notes — it reads and writes markdown files that any other tool can work with.

## File structure

Magent operates within a `.magent/` directory inside your knowledge base:

```
knowledge-base/
  .magent/
    config.toml          # global config: model defaults, API keys, etc.
    tasks/               # task definitions (recurring or triggered)
    conversations/       # conversation logs and interactive chats
  notes/
  shopping.md
  reading-list.md
  ...
```

## Tasks

A task is a markdown file with TOML frontmatter. The markdown body is the prompt/instructions.

```markdown
+++
model = "ollama/mistral"
context = ["shopping.md"]
output = "shopping.md"
schedule = "daily"
+++

Review the shopping list and remove items marked as done.
```

Key properties:
- **model** — which LLM to use (local or remote)
- **context** — files the agent should read before executing
- **output** — where to write results
- **schedule** — when to run (cron-like), if recurring

## Conversations

A conversation is a markdown file where you and the agent take turns. You write a message, the agent notices the change and responds.

```markdown
# House research

> User: What's the average price per sqm in Södermalm?

Agent: ...
```

The exact format is TBD, but the idea is that you can open any markdown editor, write a message, save the file, and the agent picks it up.

## Model support

Magent should support multiple backends:
- **Local models** via Ollama — good for simple, recurring, background tasks
- **Claude API** — for tasks that need stronger reasoning
- Potentially other OpenAI-compatible APIs

The model is configured per-task, with a global default fallback.

## Runtime

Magent runs as a persistent background process:
- Watches for file changes (new messages, modified tasks)
- Executes scheduled tasks when they come due
- Writes results back as markdown files
- Logs all conversations

## Principles

- **Markdown is the interface.** No GUI, no database. Files in, files out.
- **Composable.** Works alongside any editor, note-taking tool, or git workflow.
- **Minimal.** The agent orchestrates — it doesn't try to be a note-taking app.
- **Transparent.** All agent activity is visible as files you can read, diff, and version.
