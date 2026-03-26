# Tool Result Caching

**Status:** Draft — captures current thinking. Not ready for implementation.

## Problem

Tool results (search output, file content, web pages) can be large. Embedding them inline in the response block bloats the markdown file, makes it hard to read, and wastes context window on subsequent LLM turns that re-read the response block.

For the initial tool-use implementation (search + read with line ranges), output is naturally bounded and inline is fine. This becomes a real problem with:
- `read` on large files without a line range
- `web_fetch` returning full page content
- Multiple tool calls accumulating in one response

## Idea

Store tool results in a cache directory. The response block references them instead of inlining them.

### File structure

```
knowledge-base/
  .magent/
    tool-cache/
      a3f2b1.md    # search result
      f7c9e2.md    # fetched web page content
      ...
```

Cache key: hash of `(tool_name, input)`. Content is the raw tool output as plain text/markdown.

### Response block format

Instead of:
```markdown
<magent-tool-result tool="search">
(500 lines of results)
</magent-tool-result>
```

The response contains a reference:
```markdown
<magent-tool-result tool="search" ref=".magent/tool-cache/a3f2b1.md" />
```

During multi-turn processing, magent reads the cached file and injects it into the LLM conversation. The user can also inspect the file directly.

## Open questions

- **Cache invalidation**: when is a cached result stale? For search results, the KB may have changed. For web pages, the page may have updated. Options: TTL-based, content-hash-based, or just never invalidate (user deletes cache manually).
- **Cleanup policy**: cache grows over time. Prune on daemon start? Prune entries older than N days? Let the user manage it?
- **Hash scheme**: what hash function, and should collisions be handled? Probably SHA-256 truncated to 12 hex chars is fine for a local cache.
- **Inline vs cached threshold**: should small results still be inlined? A 5-line search result doesn't need a separate file. Possible heuristic: inline if under N lines, cache otherwise.
- **Cache file format**: plain text, or add a frontmatter header with metadata (tool name, input, timestamp)?

## When to build

Before or alongside the first tool that produces unbounded output (likely `web_fetch`). The initial search + read-with-line-ranges implementation ships with inline results.
