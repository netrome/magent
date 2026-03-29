# TODO

### Incremental response writing
- [ ] Message reconstruction from response content
- [ ] Incremental writing in the tool-use loop
- [ ] Crash recovery

## Done

### Incremental response writing
- [x] Response block status parsing
- [x] Writer support for in-progress response blocks

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

### Tool use + knowledge base search
- [x] PR 1: Tool call parser (`tool.rs`)
- [x] PR 2: Search tool (`tools/search.rs`)
- [x] PR 3: Read tool (`tools/read.rs`)
- [x] PR 4: Multi-turn LLM support (unified `complete_messages` interface)
- [x] PR 5: Wire tool use into processing loop

## Current

### Parser robustness
- [x] Skip `@magent` directives inside `<magent-response>` blocks

### Browser tool (project 007)
- [x] PR 1-3: Runtime detection, browser tool implementation, wiring + turn limit
- [x] PR 4: End-to-end testing with realistic accessibility tree fixtures

### Incremental response writing (project 008)
- [ ] PR 1: Response block status parsing
- [ ] PR 2: Writer support for in-progress response blocks
- [ ] PR 3: Message reconstruction from response content
- [ ] PR 4: Incremental writing in the tool-use loop
- [ ] PR 5: Crash recovery

## Post-MVP

- [ ] Config file (`.magent/config.toml`)
- [ ] Scheduled directives (`in:`, `at:`)
- [ ] Edit arbitrary files.
- [ ] Code execution / scripts (like python scripts).
- [ ] Per-directive model selection
- [ ] Concurrent directive processing
