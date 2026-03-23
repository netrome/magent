# ADR-001: MVP Architecture

**Status:** Proposed
**Date:** 2026-03-22

## Context

We're starting the magent project from scratch. We need to decide on the initial architecture for the MVP: how the daemon is structured, how state is tracked, and what dependencies to bring in.

## Decision

### Daemon structure: single-threaded event loop

The daemon is a single async task that receives file-change events and processes directives sequentially. No thread pool, no work queue, no concurrent file writes.

**Why:** A personal knowledge base is low-throughput. Sequential processing avoids concurrent write conflicts and keeps the code simple. We can add concurrency later if needed.

### State tracking: implicit via response markers

A directive is considered "processed" if it is followed by a `<magent-response>` block in the same file. No separate state file.

**Why:** This keeps state inspectable (it's right there in the markdown), avoids sync issues between a state file and the actual files, and means the user can "retry" a directive by simply deleting the response block.

**Trade-off:** We can't track metadata like "when was this processed" or "which model was used." That's acceptable for the MVP — we can add a state file later if needed for scheduling.

### LLM integration: OpenAI-compatible HTTP API

Use raw HTTP calls to the OpenAI chat completions endpoint format (`POST /v1/chat/completions`). No SDK.

**Why:** This format is implemented by Ollama, llama.cpp, vLLM, OpenAI, and many others. One client covers all providers. Using `reqwest` directly avoids SDK version churn and keeps dependencies minimal.

### File watching: `notify` crate

Use the `notify` crate for cross-platform filesystem event watching.

**Why:** `notify` is the established Rust solution for this. It uses inotify on Linux, FSEvents on macOS, and ReadDirectoryChanges on Windows. The alternative is polling, which is wasteful for a long-running daemon. This is a justified addition beyond the core deps listed in CLAUDE.md.

### No config file for MVP

Configuration is via CLI args and one env var (`MAGENT_API_KEY`). No `.magent/config.toml` yet.

**Why:** The MVP has ~3 configurable values. CLI args are sufficient and avoid designing a config format prematurely.

## Consequences

- The daemon is simple to understand and debug.
- No persistent state to corrupt or get out of sync.
- Limited to one LLM provider format (but it's the most common one).
- No scheduling capability until we add a state file.
- Adding a config file later is straightforward and non-breaking.
