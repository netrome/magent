# Code Style Guide

Conventions for the Magent codebase.

## Module organization

### Public before private

All public (`pub`/`pub(crate)`) functions are defined before any private
(`fn`) functions. The external interface of a module is more significant than
its internal helpers, and should be what the reader encounters first.

```
// ✓ public API first
pub fn parse_directives(…) { … }
pub fn write_response(…) { … }
fn extract_prompt(…) { … }        // private helper
fn find_marker_end(…) { … }       // private helper
```

### Callers before callees

Within the public (or private) section, if function A calls function B,
define A before B. The reader encounters the high-level API before the
implementation details.

Public types that appear in a function's signature are defined immediately
above that function:

```
pub struct Directive { … }
pub fn parse_directives(…) -> Vec<Directive> { … }
```

### Module files

Prefer `foo.rs` over `foo/mod.rs` for module declarations. A module `tools`
with submodules is declared via `src/tools.rs` (containing `pub mod search;`
etc.) alongside the `src/tools/` directory.

### Separation of concerns

Domain logic lives in top-level modules (`parser.rs`, `llm.rs`,
`writer.rs`, …). These modules contain pure functions or functions that
operate on data — they never depend on I/O types directly.

`main.rs` is minimal — it parses CLI args and calls into `lib.rs`.
All functionality, including the daemon loop, lives in the library crate
so it can be tested without running the binary.

### I/O at the edges

Business logic should be free of direct I/O. Ideally, functions are generic
over a trait defining the interface they need, so they can be tested with
fakes. For example, the LLM client should be behind a trait so tests can use
a mock instead of hitting a real API.

**Pragmatic exception**: for filesystem access, functions currently take a
`&Path` root directly rather than abstracting behind a trait. At this scale
the trait doesn't yet pay for itself, but new I/O boundaries (external
services, network calls) should use the trait-based approach.

## Naming

### Functions and types

- Use descriptive names that convey intent (`parse_directives`, not
  `parse` or `process`).
- Avoid redundant prefixes that repeat the module name.

## Testing

### Test naming

Use double underscores to separate the subject from the expectation:

```
fn function_name__should_describe_expected_behavior()
```

Every test module carries `#[allow(non_snake_case)]` to permit this
convention.

### Test structure

Use `// Given`, `// When`, `// Then` comment sections:

```rust
#[test]
fn parse_directives__should_find_unprocessed_directive() {
    // Given
    let content = "@magent why is the sky blue?\n";

    // When
    let directives = parse_directives(content);

    // Then
    assert_eq!(directives.len(), 1);
    assert_eq!(directives[0].prompt, "why is the sky blue?");
}
```

For trivial one-liner assertions the sections can be omitted, but prefer them
for anything with setup.

### Test isolation

Integration tests that touch the filesystem use a unique temporary directory (e.g. via
`tempfile::tempdir()`). The directory is cleaned up automatically when
dropped.
