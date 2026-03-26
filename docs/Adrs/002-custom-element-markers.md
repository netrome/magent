# ADR-002: Custom Element Markers

**Status:** Accepted
**Date:** 2026-03-23

## Context

Magent currently uses HTML comments to delimit agent responses:

```markdown
<!-- magent:start -->
Response text here.
<!-- magent:end -->
```

The inline editing feature (see `docs/projects/003-inline-editing.md`) introduces new structured blocks for proposed edits with status tracking. These need to be custom elements (`<magent-edit>`, `<magent-search>`, `<magent-replace>`) because:
- They carry attributes (e.g., `status="proposed"`)
- Their content is meaningful and should be visible to the user for review
- Smart renderers (Obsidian plugins, custom CSS) should be able to style and interact with them

Having response markers as HTML comments alongside edit blocks as custom elements would mean two different markup conventions in the same file. This ADR decides whether to unify them.

## Decision

Replace HTML comment markers with custom elements throughout:

```markdown
<magent-response>
Response text here.
</magent-response>
```

All magent-generated markup uses the `magent-*` custom element namespace:

| Element | Purpose |
|---------|---------|
| `<magent-response>` | Wraps all agent output (replaces `<!-- magent:start/end -->`) |
| `<magent-edit>` | A proposed, accepted, or applied edit (carries `status` attribute) |
| `<magent-search>` | The text to find in the document (inside `<magent-edit>`) |
| `<magent-replace>` | The replacement text (inside `<magent-edit>`) |

**Why:**

- **Consistency.** One tag vocabulary is simpler to parse, document, and build tooling for than a mix of comments and elements.
- **Attributes.** Custom elements support attributes (`status="proposed"`). HTML comments don't carry structured metadata without inventing a sub-format inside the comment.
- **Tooling potential.** Custom elements with hyphenated names are valid HTML5. A markdown renderer, Obsidian plugin, or browser stylesheet can target them directly (`magent-edit[status="proposed"] { ... }`). HTML comments are invisible to CSS and the DOM.
- **Cheap to change now.** The project is early — few files exist with the old markers. The migration is mechanical (parser + writer + tests).

## Trade-offs

### Markdown rendering of content inside custom elements

In CommonMark, a custom element tag on its own line starts an HTML block. Content inside may be treated as raw HTML rather than processed as markdown. This means `**bold**` inside a `<magent-response>` might render with literal asterisks in some renderers.

This is acceptable because:
- The primary reading experience is in a text editor, where raw text is the format anyway.
- Agent responses are typically plain text or simple markdown that doesn't rely on rendering.
- If this proves problematic, we can ensure blank lines after opening tags (which ends the HTML block in CommonMark and lets subsequent content be processed as markdown).

### Breaking change

Files with `<!-- magent:start/end -->` markers will no longer be recognized as processed. Existing directives will be re-processed on the next file change.

This is acceptable because:
- The project has no production users yet.
- Re-processing a directive just produces a new response — it's a no-op in terms of data loss.
- We make a clean break rather than supporting both formats indefinitely.

## Consequences

- Parser and writer are updated in a single PR before the editing feature is built.
- All magent markup is discoverable by searching for `magent-` in any file.
- Future features (edit status tracking, metadata attributes, tooling integration) build on a consistent element-based foundation.
- ADR-001's "state tracking via response markers" decision still holds — only the marker syntax changes, not the mechanism.
