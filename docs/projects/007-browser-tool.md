# Browser Tool (agent-browser integration)

**Status:** Draft

## Goal

Give the agent the ability to browse the web: read pages, interact with forms, click through flows, and extract information from web applications. This unlocks a large class of tasks that require web access — from summarizing articles to filling out forms to navigating multi-step web workflows.

We want:
```markdown
@magent check the status of my PR at https://github.com/me/repo/pull/42 and summarize the review comments
```

The agent opens the URL, takes a snapshot of the accessibility tree, reads the content, navigates to comments if needed, and writes a response synthesizing what it found.

## Why agent-browser

[agent-browser](https://github.com/vercel-labs/agent-browser) is a headless browser automation CLI built in Rust, specifically designed for AI agent workflows. Key properties:

- **Rust-native CLI** — no Node.js/Python runtime needed. Magent shells out to the `agent-browser` binary, same as it would to any CLI tool.
- **Accessibility tree snapshots** — the `snapshot` command returns a compact text representation of the page with element references (`@e1`, `@e2`, ...). This is purpose-built for LLM consumption: structured, concise, no HTML/CSS noise. Solves the context bloat problem that raw HTML would cause.
- **Element references** — after a snapshot, the LLM can refer to elements as `@e1`, `@e3`, etc. in subsequent commands (click, type, fill). No CSS selectors or XPath needed.
- **Session persistence** — the browser process persists between CLI invocations. `open` starts Chrome, subsequent commands (`snapshot`, `click`, etc.) interact with the same session. `close` shuts it down. This maps naturally to magent's multi-turn tool loop.
- **Batch mode** — supports piping JSON arrays of commands for multi-step execution in a single invocation (future optimization).

### Why not a simpler web_fetch tool?

A `web_fetch` tool (HTTP GET + html-to-markdown) would be simpler to build but:

1. **Context bloat**: converted HTML is still noisy. Accessibility tree snapshots are more compact and structured.
2. **No interaction**: can't fill forms, click buttons, navigate SPAs, or handle any page that requires JavaScript.
3. **Partial replacement**: we'd build web_fetch now, then partially replace it with browser capabilities later. agent-browser's `snapshot` command already serves as a better "read this webpage" tool.
4. **Auth/cookies**: web_fetch can't handle login flows. agent-browser maintains a real browser session with cookies, local storage, etc.

agent-browser gives us both read (snapshot) and interact (click, type, fill) in one tool.

## Constraints

- **Optional dependency**: agent-browser is an external binary, not a crate dependency. If it's not installed on the host, magent works exactly as before — no browser tools appear in the system prompt.
- **Transparent**: all browser commands and their results are visible in the response block, same as search/read.
- **Markdown is the interface**: no hidden browser state. The snapshot text in tool results is the agent's view of the page.
- **Bounded scope**: we expose a small set of commands. The full agent-browser command surface (network interception, HAR recording, device emulation, etc.) is not exposed to the LLM.

## Design

### Runtime detection

At startup (or at directive processing time), magent checks whether `agent-browser` is available:

```rust
fn browser_available() -> bool {
    Command::new("agent-browser")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
```

If unavailable, the browser tool is not registered and does not appear in the system prompt. No error, no warning — it simply doesn't exist as far as the LLM is concerned.

Cache this check per daemon lifetime (don't re-check on every directive).

### Tool interface

A single `browser` tool. The input is an agent-browser CLI command:

```markdown
<magent-tool-call tool="browser">
<magent-input>open https://example.com</magent-input>
</magent-tool-call>
```

```markdown
<magent-tool-call tool="browser">
<magent-input>snapshot</magent-input>
</magent-tool-call>
```

```markdown
<magent-tool-call tool="browser">
<magent-input>click @e3</magent-input>
</magent-tool-call>
```

This maps 1:1 to the agent-browser CLI. The LLM writes the command directly — no intermediate format to design or maintain. agent-browser's command names are already self-documenting and LLM-friendly.

One command per tool call. This maintains the existing one-tool-call-per-turn model and keeps each step visible in the response block.

### Allowed commands

Only a subset of agent-browser commands are exposed. We allowlist commands rather than blocklist to prevent the LLM from accessing dangerous operations (e.g., `eval` for arbitrary JavaScript, `network route` for request interception).

**Initial allowlist:**

| Command | Purpose | Example |
|---------|---------|---------|
| `open` | Navigate to URL | `open https://example.com` |
| `snapshot` | Accessibility tree (primary read mechanism) | `snapshot` |
| `click` | Click element | `click @e3` |
| `type` | Type into focused element | `type @e5 hello world` |
| `fill` | Clear and fill input | `fill @e5 search query` |
| `select` | Select dropdown option | `select @e7 option-value` |
| `press` | Press key | `press Enter` |
| `scroll` | Scroll page | `scroll down` |
| `wait` | Wait for element/condition | `wait @e3` |
| `get text` | Get text content of element | `get text @e4` |
| `get title` | Get page title | `get title` |
| `get url` | Get current URL | `get url` |
| `screenshot` | Take screenshot (for user, not LLM) | `screenshot page.png` |
| `back` | Navigate back | `back` |
| `close` | Close browser session | `close` |

**Not exposed (intentionally):**
- `eval` — arbitrary JavaScript execution
- `network route/unroute` — request interception and mocking
- `set credentials` — credential injection
- `cookies set` — cookie manipulation
- `storage local set` / `storage session set` — storage manipulation
- `upload` — file upload (reconsider later)
- `set headers` — header manipulation

The allowlist is enforced in magent before shelling out. Any command not on the list returns an error result to the LLM.

### System prompt section

When agent-browser is available, the tools section gains:

```
## browser
Interact with web pages using a headless browser.
Input: a browser command. One command per call.

Key commands:
- open <url> — navigate to a URL (starts browser if needed)
- snapshot — get page content as an accessibility tree with element refs (@e1, @e2, ...)
- click <ref> — click an element (e.g. click @e3)
- type <ref> <text> — type text into an element
- fill <ref> <text> — clear and fill an input field
- select <ref> <value> — select a dropdown option
- press <key> — press a key (Enter, Tab, Escape, etc.)
- scroll <direction> — scroll the page (up, down, left, right)
- wait <ref> — wait for an element to appear
- get text <ref> — get text content of an element
- get title — get page title
- get url — get current URL
- back — go back
- close — close browser

Typical workflow: open URL → snapshot → read/interact → snapshot again → respond.
After open, always snapshot first to see the page content before interacting.
```

### Execution

The `BrowserTool` implementation:

```rust
pub struct BrowserTool;

impl BrowserTool {
    pub fn execute(&self, input: &str) -> String {
        let args = parse_command(input);

        if !is_allowed_command(&args) {
            return format!("Error: command '{}' is not allowed", args[0]);
        }

        match Command::new("agent-browser")
            .args(&args)
            .output()
        {
            Ok(output) => {
                if output.status.success() {
                    String::from_utf8_lossy(&output.stdout).into_owned()
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    format!("Error: {}", stderr)
                }
            }
            Err(e) => format!("Error: failed to run agent-browser: {}", e),
        }
    }
}
```

Same pattern as existing tools: returns `String`, errors are result text, never panics.

### Approval tier

The browser tool occupies an interesting middle ground:

- `snapshot`, `get text`, `get title`, `get url` are read-only — they don't change page state.
- `click`, `type`, `fill`, `select`, `press` are write operations — they can submit forms, trigger actions, change state.
- `open` initiates a network connection to an external URL.

**Recommendation: auto-execute all browser commands.**

Rationale:
- The gated tool flow (status="proposed" → user accepts → continue) is designed for single approval points, not multi-step workflows. Requiring approval for every click/type in a 10-step browser flow would make the feature unusable.
- The user already has control: they write the directive, they see every command and result in the response block, and the tool call limit (see below) bounds the total work.
- The browser operates in a headless sandbox. It can't access the user's actual browser session, cookies, or saved passwords.
- This matches how agent-browser is designed to be used: agents execute sequences of commands autonomously.

If gated approval is later needed for specific high-risk commands (e.g., submitting a payment form), we can add per-command gating without changing the tool interface.

### Tool call limit

Browser workflows are inherently multi-step: open → snapshot → click → snapshot → type → press Enter → snapshot → read result. A typical flow uses 5-8 tool calls just for one page interaction.

The current limit of 5 is too low for browser-heavy directives. Options:

1. **Raise the global limit** (e.g., to 15). Simple, but wastes budget on non-browser directives.
2. **Per-tool limits**. More complex, doesn't capture that a browser workflow is one logical operation.
3. **Per-directive limit via directive options**. The user writes `@magent turns:15 go check...` to opt into more turns. Explicit, flexible, no global change.
4. **Raise the default moderately** (e.g., to 10) and rely on the model stopping naturally.

**Recommendation: option 4 for now, option 3 as follow-up.** Raise the default to 10 — this covers most browser workflows and doesn't hurt non-browser directives (models stop when they're done). Per-directive limits can come with the config file work.

### Session lifecycle

agent-browser manages its own browser process. The lifecycle for a single directive:

1. First `open` command starts Chrome (if not already running) and navigates.
2. Subsequent commands interact with the same session.
3. Magent does **not** explicitly `close` — the LLM may or may not close the browser. If it doesn't, the session persists for the next directive that uses browser tools.
4. On daemon shutdown, we should attempt a `close` to clean up the Chrome process.

This means browser state (cookies, login sessions) can persist across directives within one daemon lifetime. This is a feature, not a bug — it enables multi-directive workflows where the agent logs in once and then performs multiple tasks.

### Snapshot output and context budget

Accessibility tree snapshots are compact but can still be large for complex pages. Considerations:

- **Tool result caching** (project 006) becomes more important with browser snapshots. A complex page snapshot might be 200-500 lines. With multiple snapshots per directive, inline embedding gets heavy.
- **Until caching is implemented**: inline is fine. The model's context window is the natural bound, and snapshots are much smaller than raw HTML would be.
- **Snapshot flags**: agent-browser's `snapshot` command may support flags for filtering (e.g., interactive elements only). We should expose these via the `-i` flag if available, and mention it in the system prompt.

### What the response looks like

```markdown
@magent what are the open review comments on https://github.com/me/repo/pull/42?

<magent-response>
I'll check the PR for you.

<magent-tool-call tool="browser">
<magent-input>open https://github.com/me/repo/pull/42</magent-input>
</magent-tool-call>
<magent-tool-result tool="browser">
Navigated to https://github.com/me/repo/pull/42
</magent-tool-result>
<magent-tool-call tool="browser">
<magent-input>snapshot</magent-input>
</magent-tool-call>
<magent-tool-result tool="browser">
document "Add retry logic to API client by user · Pull Request #42"
  banner
    navigation "Global"
      ...
  main
    heading "Add retry logic to API client #42"
    text "Open — user wants to merge 3 commits into main from retry-logic"
    navigation "Pull request tabs"
      tab "Conversation" selected
      tab "Commits 3"
      tab "Files changed 4"
    article "Review comment by reviewer1"
      @e14 link "reviewer1"
      text "2 days ago"
      paragraph "The backoff multiplier should be configurable..."
    article "Review comment by reviewer2"
      @e18 link "reviewer2"
      text "1 day ago"
      paragraph "Can we add a test for the timeout case?"
    ...
</magent-tool-result>

There are 2 open review comments on PR #42:

1. **reviewer1** (2 days ago): The backoff multiplier should be configurable rather than hardcoded.
2. **reviewer2** (1 day ago): Requesting a test for the timeout case.
</magent-response>
```

## Testing

### No Chrome in tests

Tests do not require agent-browser or Chrome. All browser tool tests use **fixtures** — canned command/response pairs that simulate agent-browser output.

### Fixture approach

```rust
struct FakeBrowser {
    responses: VecDeque<(String, String)>,  // (expected_command_prefix, response)
}

impl FakeBrowser {
    fn execute(&mut self, input: &str) -> String {
        let (expected, response) = self.responses.pop_front()
            .expect("unexpected browser call");
        assert!(input.starts_with(&expected),
            "expected command starting with '{}', got '{}'", expected, input);
        response.clone()
    }
}
```

Fixture data includes realistic accessibility tree snapshots captured from real pages, stored as string constants or test helper functions.

### Test cases

1. **Runtime detection**: mock `Command` execution to test available/unavailable paths.
2. **Command allowlist**: verify allowed commands execute, blocked commands return error.
3. **Command parsing**: edge cases in input parsing (quoted strings, special characters).
4. **Tool dispatch**: `execute_tool` routes `"browser"` to `BrowserTool` when available.
5. **System prompt**: browser tool section included/excluded based on availability.
6. **Integration**: multi-turn directive with fake LLM + fake browser, verifying the full open → snapshot → interact → respond flow.
7. **Error handling**: agent-browser returns non-zero exit code, agent-browser binary disappears mid-session.

### Startup smoke test

When agent-browser is detected at startup, run a minimal smoke test:

```
agent-browser --version
```

If this fails (binary exists but broken), log a warning and disable the tool. This catches cases like missing Chrome for Testing or broken installations without affecting daemon startup.

## Changes needed

### New module: `tools/browser.rs`

```rust
pub struct BrowserTool;

const ALLOWED_COMMANDS: &[&str] = &[
    "open", "snapshot", "click", "type", "fill", "select",
    "press", "scroll", "wait", "get", "screenshot", "back", "close",
];

impl BrowserTool {
    pub fn execute(&self, input: &str) -> String { ... }
}

pub fn is_available() -> bool { ... }
```

### Tool dispatch (`lib.rs`)

Add `"browser"` arm to `execute_tool`:

```rust
"browser" => {
    if browser_available {
        browser_tool.execute(&call.input)
    } else {
        "Error: browser tool is not available".to_string()
    }
}
```

### System prompt (`llm.rs`)

Conditionally include browser tool documentation in `build_system_prompt()` based on runtime detection.

### Tool call limit

Raise `MAX_TOOL_CALLS` from 5 to 10.

### Daemon shutdown

Add cleanup to close any open browser session on daemon exit (best-effort `agent-browser close`).

## Related work: incremental response writing

Currently, `process_directive` builds the full response in memory and writes it to the file once at the end. With browser workflows spanning 8-10 tool calls, this means:

1. **No visibility**: the user can't see what the agent is doing until it's done.
2. **No interruption**: no way to stop a runaway browser interaction mid-flow.
3. **Lost work on crash**: if the daemon crashes at step 7 of 10, all progress is lost.

**Incremental writing** — flushing each tool call/result to the file as it happens — would solve all three. This is a separate feature that touches the core processing loop and writer, not specific to the browser tool. It introduces its own design questions (in-progress markers, watcher ignoring its own writes, partial response format).

The browser tool does not depend on incremental writing, but benefits significantly from it. It should be designed and implemented as a follow-up, potentially before or alongside the browser work.

## Design decisions to revisit later

- **Per-directive tool call limits**: let users write `@magent turns:15 ...` for complex browser workflows.
- **Batch mode**: send multiple commands per tool call via agent-browser's JSON batch mode. Reduces LLM round-trips but loses per-step visibility.
- **Snapshot filtering**: expose `snapshot -i` (interactive elements only) for more compact output on complex pages.
- **Tool result caching**: large snapshots should be cached per project 006. Implement before or alongside this feature.
- **Upload support**: gated file upload could be valuable but needs careful security consideration.
- **Per-command gating**: if specific browser actions need approval (e.g., form submissions), add per-command gating without changing the tool interface.
- **Tab management**: `tab new`, `tab N`, `tab close` for multi-tab workflows.
- **Screenshot as context**: for models with vision, screenshots could supplement or replace snapshots.

## Non-goals

- **Exposing the full agent-browser command surface**: network interception, HAR recording, device emulation, credential injection, JavaScript eval, cookie/storage manipulation are out of scope.
- **Browser as a persistent service**: the browser session is tied to directive processing, not a long-lived service the user interacts with.
- **Visual testing / screenshot diffing**: the browser is for information retrieval and interaction, not visual QA.
- **Gated approval for initial implementation**: auto-execute keeps the feature usable for multi-step workflows. Gating can be added per-command later.

## Dependencies

- **Runtime**: `agent-browser` CLI binary + Chrome for Testing (managed by agent-browser).
- **Build**: none. `std::process::Command` is all we need.
- **Test**: none. Fixture-based, no browser needed.

This adds zero crate dependencies to magent.

## Task breakdown

### PR 1: Runtime detection + conditional system prompt

**Changes:**
- `browser_available()` function (cached check for `agent-browser --version`)
- `build_system_prompt()` conditionally includes browser tool section
- Wiring in `run()` to check availability at startup

**Acceptance criteria:**
- When `agent-browser` is on PATH, system prompt includes browser tool docs
- When `agent-browser` is absent, system prompt is unchanged
- Availability check is cached (not re-run per directive)
- Unit tests for both paths (mock command execution)

### PR 2: Browser tool implementation (`tools/browser.rs`)

**Changes:**
- `BrowserTool` struct with `execute()` method
- Command parsing and allowlist enforcement
- Shell out to `agent-browser` with parsed args
- Error handling (non-zero exit, missing binary, disallowed command)

**Acceptance criteria:**
- Allowed commands execute and return stdout
- Disallowed commands (eval, network, cookies set, etc.) return error
- Non-zero exit codes return stderr as error
- Missing binary returns descriptive error
- Unit tests with fixture data for each allowed command
- Tests for command allowlist (allowed + blocked)

### PR 3: Wire browser into tool dispatch + raise turn limit

**Changes:**
- Add `"browser"` arm to `execute_tool` dispatch
- Raise `MAX_TOOL_CALLS` from 5 to 10
- Add browser cleanup on daemon shutdown

**Acceptance criteria:**
- Directive triggering browser tool calls → commands executed → results fed back → response written
- Browser commands visible in response block alongside search/read calls
- Tool call limit of 10 works for browser-heavy directives
- `agent-browser close` attempted on daemon shutdown
- Integration tests with fake LLM + fake browser fixture

### PR 4: End-to-end testing with realistic fixtures

**Changes:**
- Realistic accessibility tree snapshot fixtures (captured from real pages)
- Multi-step browser workflow integration tests (open → snapshot → click → snapshot → respond)
- Mixed tool tests (search + browser in same directive)

**Acceptance criteria:**
- Full open-snapshot-interact-respond flow works with fixture data
- Mixed tool usage (knowledge base search + browser) in one directive works
- Snapshot content is correctly passed through to LLM context
- Tests document expected agent-browser output format for future reference
