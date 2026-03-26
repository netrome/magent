# TODO

## Done

### MVP
- [x] PR 1: Project skeleton + CLI (`clap`, arg parsing, signal handling)
- [x] PR 2: File watcher (`notify`, watch directory for `.md` changes)
- [x] PR 3: Directive parser (find `@magent` directives, detect processed state)
- [x] PR 4: LLM client (OpenAI-compatible chat completions via `reqwest`)
- [x] PR 5: Response writer (insert response markers into markdown files)
- [x] PR 6: Wire it all together (end-to-end main loop)

### Post-MVP
- [x] Document context in LLM prompts

### Inline document editing
- [x] PR 1: Migrate response markers to custom elements (`<magent-response>`)
- [x] PR 2: Edit block parser (`edit.rs`)
- [x] PR 3: Edit application logic
- [x] PR 4: Wire edit proposals into the processing loop
- [x] PR 5: Edit acceptance processing

### Edit robustness
- [x] Trim leading/trailing whitespace in search/replace tag extraction
- [x] Add system prompt example showing multiline edit blocks with whitespace

### Chain-of-thought prompting
- [x] Add `<magent-thinking>` tags instruction to system prompt

### Explicit references
- [x] PR 1: Parse directive options into `HashMap<String, String>`
- [x] PR 2: Resolve context files, assemble extended document, update system prompt

## Current

### Tool use + knowledge base search
- [ ] PR 1: Tool call parser (`tool.rs`)
- [ ] PR 2: Search tool (`tools/search.rs`)
- [ ] PR 3: Read tool (`tools/read.rs`)
- [ ] PR 4: Multi-turn LLM support (unified `complete_messages` interface)
- [ ] PR 5: Wire tool use into processing loop

## Post-MVP

- [ ] Config file (`.magent/config.toml`)
- [ ] Scheduled directives (`in:`, `at:`)
- [ ] Per-directive model selection
- [ ] Concurrent directive processing
