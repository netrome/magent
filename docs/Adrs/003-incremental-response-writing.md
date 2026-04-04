# ADR-003: Incremental Response Writing

**Status:** Proposed
**Date:** 2026-03-29

## Context

`process_directive` currently builds the full response in memory across all tool-use iterations, then writes it to the file in a single atomic operation. With browser workflows spanning 8-10 tool calls (and the potential for even longer chains), this means:

1. **No visibility** — the user sees nothing until the entire chain completes.
2. **No interruption** — no way to stop or redirect a long-running agent.
3. **Lost work on crash** — if the daemon dies at step 7 of 10, all progress is lost.

Project 007 (browser tool) identified this as a follow-up need.

## Decision

Flush the response to disk after every LLM turn and tool execution. Re-read the file before each LLM call. This makes the markdown file the live, authoritative representation of the agent's execution state.

### Core mechanism

The tool-use loop changes from "accumulate in memory, write once" to:

1. Open a response block with `status="in-progress"`.
2. Call the LLM. Append its output to the response block on disk.
3. If the LLM made a tool call: execute it, append the result to the response block on disk.
4. Before the next LLM call, re-read the file.
   - If the response block was deleted → stop.
   - If the response content differs from what we last wrote → the user intervened. Rebuild the message history from what's on disk and continue from there.
   - If unchanged → continue normally.
5. When the LLM produces a final response (no tool call) → remove the `status` attribute (response is complete).

### File as source of truth

The daemon does not own the response — the file does. The daemon's in-memory state is a working cache that is re-synced from disk on every iteration. User edits to the file are the intervention mechanism: no special commands, no side channels. This keeps the mental model simple:

> File edit → watcher detects work → daemon processes → file edit → ...

Whether the edit came from the daemon or the user is irrelevant.

### User intervention

Because the file is the source of truth, intervention is just file editing:

- **Rewind**: delete tool calls from the end of the response. The daemon re-reads, sees a shorter history, and continues from the earlier point.
- **Modify tool results**: edit a `<magent-tool-result>` in the response. The daemon picks up the modified result on the next re-read.
- **Stop**: delete the response block entirely. The directive becomes unprocessed (user can choose whether to re-trigger it).
- **Pause**: change `status="in-progress"` to `status="paused"`. The daemon sees a paused response and skips it. To resume, change back to `status="in-progress"`.

### Response block format

```markdown
@magent find the pricing page

<magent-response status="in-progress">
Let me search for that.

<magent-tool-call>
search | query: pricing
</magent-tool-call>
<magent-tool-result>
Found 2 results...
</magent-tool-result>

Now I'll read the file.
</magent-response>
```

When complete, the status attribute is removed:

```markdown
<magent-response>
...full response...
</magent-response>
```

### Watcher interaction

The daemon's own writes trigger watcher events. These events queue up during `process_directive` execution (single-threaded async — the event loop is blocked). When the directive finishes and control returns to `process_events`, the queued events fire, `process_file` re-reads the file, sees a completed response, and skips it. No special filtering or hash tracking needed.

## Consequences

### Positive

- **Progress visibility**: users see each step as it happens.
- **Crash recovery**: in-progress responses survive daemon restarts. On startup (or next file change), the daemon can detect and resume them.
- **Hackability**: users can intervene mid-execution by editing the file. This is a powerful debugging and steering tool.
- **Simplicity**: the mental model stays "everything is in the file." No new state management, no side channels.

### Negative

- **More disk I/O**: writes on every tool iteration instead of once. Negligible for markdown files.
- **Parser complexity**: the parser needs to understand `status="in-progress"` and the writer needs to support partial response blocks. Message reconstruction from response content is new logic.
- **Write conflicts**: if the user saves the file at the exact moment the daemon is writing, one write could overwrite the other. Atomic writes (write temp file + rename) reduce the window but don't eliminate it. This is an acceptable risk for a local, single-user tool.
### Scope

This changes `process_directive`, the writer, and the parser. It does not change the tool interface, the LLM client, or the watcher/`process_events` loop.
