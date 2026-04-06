# Web Search Tool (Brave Search API)

**Status:** Proposed

## Goal

Give the agent the ability to search the web for information. This unlocks tasks that require current, real-world knowledge beyond what's in the knowledge base or the LLM's training data.

We want:
```markdown
@magent find the current world record for backyard ultra and summarize who holds it
```

The agent calls the web search tool, gets structured results (title, URL, snippet), and synthesizes an answer — no browser session needed.

## Why a search API (not agent-browser)

The browser tool works well for navigating specific pages, but web search through a browser is unreliable:

1. **Bot detection**: both Google and DuckDuckGo detect headless browsers and serve CAPTCHAs. This blocks search entirely — we've confirmed this on the current machine (Hetzner datacenter IP gets flagged on the first request).
2. **IP reputation degrades**: even if searches work initially, repeated automated requests from a datacenter IP erode trust until the IP is blocked.
3. **Fragile scraping**: search engine HTML changes without notice. A dedicated API returns stable, structured JSON.
4. **Overhead**: launching a browser, rendering a page, and parsing an accessibility tree just to get 10 links is wasteful when a single HTTP GET returns clean JSON.

A search API is the correct tool for programmatic search. The browser tool remains the right choice for interacting with specific web pages.

## Why Brave Search

- **Simple REST API**: single GET endpoint, JSON response, API key in a header.
- **No new dependencies**: uses `reqwest` (already a core dep) with an API key. Zero new crates.
- **Free tier**: 2,000 queries/month with $5/month free credits. Sufficient for personal/development use.
- **Independent index**: Brave maintains its own web index, not a proxy over Google/Bing.
- **Rate limit**: 1 query/second on the free plan (50/sec on paid). Simple to respect.

Alternatives considered:
- **SearXNG (self-hosted)**: free and unlimited, but requires hosting and maintaining a separate service. Adds operational complexity for a tool that should just work.
- **Google Custom Search API**: 100 free queries/day, more complex setup (requires a custom search engine ID + API key).
- **Bing Web Search API**: similar to Google — paid after free tier, Microsoft account required.

Brave is the simplest option that satisfies the requirements with minimal operational overhead.

## Constraints

- **Optional dependency**: if no API key is configured, the tool does not appear in the system prompt. Same pattern as the browser tool's runtime detection.
- **No new crate dependencies**: `reqwest` + `serde` handle everything.
- **API key security**: the key is read from an environment variable, never logged or written to output files.
- **Read-only**: the tool only performs searches. It does not fetch or render full pages (that's what the browser tool is for).

## Design

### Configuration

The API key is read from the `BRAVE_SEARCH_API_KEY` environment variable. This follows the same pattern as `MAGENT_API_KEY` for the LLM client — environment variables for secrets, no config files.

At startup, if `BRAVE_SEARCH_API_KEY` is set and non-empty, the web search tool is enabled. If unset, the tool doesn't exist as far as the LLM is concerned.

### Tool interface

A single `web_search` tool. Input is a search query string, optionally with a `count:N` prefix to control result count (default 5, max 20).

```markdown
<magent-tool-call tool="web_search">
<magent-input>current backyard ultra world record</magent-input>
</magent-tool-call>
```

With explicit count:
```markdown
<magent-tool-call tool="web_search">
<magent-input>count:3 Rust async runtime comparison</magent-input>
</magent-tool-call>
```

### API request

```
GET https://api.search.brave.com/res/v1/web/search?q={query}&count={count}
Headers:
  Accept: application/json
  X-Subscription-Token: {api_key}
```

Parameters:
- `q`: the search query (max 400 chars, 50 words)
- `count`: number of results (1-20, default 5)

We intentionally keep parameters minimal. No language/region/freshness filtering in the initial version — the LLM can include these hints in the query itself (e.g., "site:reddit.com ...", "2024 ...").

### Response format

The API returns JSON. We extract the `web.results` array and format it as plain text for the LLM:

```
1. Title of first result
   https://example.com/page
   Snippet text describing the result...

2. Title of second result
   https://another.com/article
   Another snippet with relevant information...
```

This is compact, readable, and gives the LLM what it needs: titles to gauge relevance, URLs to potentially open with the browser tool, and snippets for quick answers.

If `web.results` is empty or missing, return: `No results found for: {query}`

### Error handling

| Condition | Behavior |
|-----------|----------|
| API key missing at call time | `Error: web search is not available` (shouldn't happen — tool hidden from prompt) |
| Network error | `Error: web search request failed: {details}` |
| 401/403 (bad key) | `Error: web search authentication failed — check BRAVE_SEARCH_API_KEY` |
| 429 (rate limited) | `Error: web search rate limited — try again shortly` |
| Other HTTP error | `Error: web search returned status {code}` |
| Malformed response | `Error: could not parse search results` |

Errors are returned as result text (same pattern as all other tools). No retries — the LLM can decide to retry or adjust its query.

### System prompt section

When the API key is configured, the tools section gains:

```
## web_search
Search the web using Brave Search.
Input: a search query. Optional prefix: count:N (1-20, default 5)
Returns: numbered results with title, URL, and description.

Use this to find current information, look up facts, or discover relevant pages.
If you need to read the full content of a result, use the browser tool to open its URL.
```

### What the response looks like

```markdown
@magent who holds the current backyard ultra world record?

<magent-response>
Let me search for that.

<magent-tool-call tool="web_search">
<magent-input>current backyard ultra world record holder</magent-input>
</magent-tool-call>
<magent-tool-result tool="web_search">
1. Backyard Ultra World Record - Wikipedia
   https://en.wikipedia.org/wiki/Backyard_ultra
   A backyard ultra is an ultramarathon format where runners complete a
   4.167-mile loop every hour. The current world record is...

2. New Backyard Ultra World Record Set at Big's Backyard 2024
   https://ultrarunning.com/big-backyard-2024
   The record was broken at Big's Backyard Ultra with a distance of...
</magent-tool-result>

Based on the search results, ...
</magent-response>
```

### Integration with browser tool

The web search tool and browser tool complement each other:
1. `web_search` finds relevant URLs quickly and cheaply.
2. `browser` opens a specific URL to read full page content or interact with it.

The LLM can chain them naturally: search → pick a result → open with browser → read details.

## Implementation

### New module: `tools/web_search.rs`

```rust
pub struct WebSearchTool {
    http: reqwest::blocking::Client,
    api_key: String,
}

impl WebSearchTool {
    pub fn new(api_key: String) -> Self { ... }
    pub fn execute(&self, input: &str) -> String { ... }
}
```

Uses `reqwest::blocking::Client` since tool execution is synchronous (same as the rest of the tool system). The client is created once and reused across calls.

Internal helpers:
- `parse_input(input) -> (query, count)` — extract optional `count:N` prefix
- `format_results(response) -> String` — extract `web.results` and format as numbered list

### Serde response types

Minimal structs to deserialize only the fields we need:

```rust
#[derive(Deserialize)]
struct BraveSearchResponse {
    web: Option<WebResults>,
}

#[derive(Deserialize)]
struct WebResults {
    results: Vec<WebResult>,
}

#[derive(Deserialize)]
struct WebResult {
    title: String,
    url: String,
    description: String,
}
```

These are private to the module. We don't need `age`, `type`, or any other fields for the initial version.

### Tool dispatch (`lib.rs`)

Add `"web_search"` arm to `execute_tool`. The `WebSearchTool` instance is created in the processing loop (like browser availability) and passed through, or created once at startup if the API key is present.

```rust
"web_search" => match web_search {
    Some(ws) => ws.execute(&call.input),
    None => "Error: web search is not available".to_string(),
},
```

### System prompt (`llm.rs`)

Add a `{web_search_tool}` placeholder in the system prompt template, conditionally replaced with the tool documentation when the API key is configured. Same pattern as `{browser_tool}`.

## Testing

### Unit tests (no API calls)

All tests use canned JSON responses. No real API calls in the test suite.

- **Input parsing**: `count:5 query` → (query, 5); `query` → (query, 5); `count:abc query` → error
- **Response formatting**: valid JSON → numbered result list
- **Empty results**: `web.results` is empty → "No results found"
- **Missing web field**: `web` is null → "No results found"
- **Error responses**: 401, 429, 500 → appropriate error messages
- **Query length**: input exceeding 400 chars → truncated or error

### Integration tests

- **Tool dispatch**: `execute_tool` routes `"web_search"` correctly when available/unavailable
- **System prompt**: web search section included when API key set, excluded when not
- **Multi-tool flow**: directive using both `web_search` and `browser` in sequence (fake LLM + canned responses)

### HTTP mocking

Use `wiremock` (already a dev dependency) to test the actual HTTP request/response cycle:
- Correct URL, headers, query parameters sent
- JSON response deserialized correctly
- Network errors handled gracefully

## Changes needed

| File | Change |
|------|--------|
| `src/tools/web_search.rs` | New module: `WebSearchTool`, input parsing, response formatting, HTTP client |
| `src/tools/mod.rs` | Add `pub mod web_search;` |
| `src/lib.rs` | Add `"web_search"` arm to `execute_tool`, create tool instance at startup |
| `src/llm.rs` | Add `{web_search_tool}` placeholder + web search tool docs + conditional inclusion |

## Non-goals

- **Full page fetching**: web search returns snippets, not full page content. Use the browser tool for that.
- **Caching results**: not needed for the initial version. Searches are fast and cheap.
- **Multiple search providers**: Brave only. If we need failover later, that's a separate project.
- **Advanced query parameters**: no language, region, freshness, or safesearch options exposed to the LLM initially. The query string itself is expressive enough.
- **Rate limiting logic in magent**: Brave's free tier allows 1 req/sec. The sequential tool execution model means we can't exceed this in practice (each search requires an LLM round-trip between calls). If this changes with concurrent execution, revisit then.

## Task breakdown

### PR 1: WebSearchTool implementation + unit tests

**Changes:**
- `src/tools/web_search.rs`: `WebSearchTool` struct, `execute()`, input parsing, response formatting
- `src/tools/mod.rs`: add module
- Unit tests with canned JSON for all paths (success, empty, errors)
- `wiremock` tests for HTTP request/response cycle

**Acceptance criteria:**
- Valid query → formatted result list
- `count:N` prefix parsed correctly, default is 5
- Empty/missing results → "No results found"
- HTTP errors (401, 429, 500) → descriptive error messages
- Network failure → error message (not panic)
- Correct URL, headers, query params sent to Brave API

### PR 2: Wire into tool dispatch + system prompt

**Changes:**
- `src/lib.rs`: create `WebSearchTool` at startup when `BRAVE_SEARCH_API_KEY` is set, add dispatch arm
- `src/llm.rs`: add conditional web search tool section to system prompt

**Acceptance criteria:**
- When `BRAVE_SEARCH_API_KEY` is set: tool appears in system prompt, `web_search` calls dispatched correctly
- When key is unset: tool absent from system prompt, `web_search` calls return error
- Integration test: multi-turn directive with fake LLM using web search tool
- API key never appears in logs or tool output
