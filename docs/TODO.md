# TODO

## Done

- [x] PR 1: Project skeleton + CLI (`clap`, arg parsing, signal handling)

## Current

- [x] PR 2: File watcher (`notify`, watch directory for `.md` changes)

## Up next

- [ ] PR 3: Directive parser (find `@magent` directives, detect processed state)
- [ ] PR 4: LLM client (OpenAI-compatible chat completions via `reqwest`)
- [ ] PR 5: Response writer (insert response markers into markdown files)
- [ ] PR 6: Wire it all together (end-to-end main loop)

## Post-MVP

- [ ] Config file (`.magent/config.toml`)
- [ ] Scheduled directives (`in:`, `at:`)
- [ ] Document context in LLM prompts
- [ ] Per-directive model selection
- [ ] Concurrent directive processing
