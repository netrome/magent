# Agent Instructions (Magent)

This repository is a **markdown-native AI agent daemon** that watches a knowledge base and executes tasks defined as markdown files.
Optimize for: simplicity, hackability, minimal dependencies, and long-term maintainability.

## Read these first
- README.md
- docs/STYLE.md
- docs/ARCHITECTURE.md (when it exists)
- docs/Adrs/*
- docs/ROADMAP.md

## Core rules

- **NO FEATURE CREEP**
  - Implement only the explicitly requested task.
  - Keep the daemon simple — it reads markdown, calls LLMs, writes markdown.

- **MARKDOWN IS THE INTERFACE**
  - No database, no proprietary formats.
  - All configuration, tasks, conversations, and output are plain text files.
  - Agent behavior should always be inspectable and diffable.

- **DECISIONS REQUIRE ADRs**
  - If a change affects architecture, data model, file format conventions, or introduces a significant dependency,
    create/update an ADR in `docs/Adrs/`.

- **DEPENDENCY DISCIPLINE**
  - Avoid adding dependencies. If needed, justify why (and why the stdlib or existing deps aren't enough).
  - Core deps: `tokio`, `reqwest`, `serde`, `toml`, `clap`. Think carefully before adding more.

- **SECURITY IS NOT OPTIONAL**
  - API keys must never be logged or written to output files.
  - Never read/write outside the configured knowledge base root.
  - Be explicit about what file paths agents can access.

## Documentation rules

- `docs/ROADMAP.md` — potential upcoming features (simple bullet list, no task breakdowns).
- `docs/projects/` — project docs (numbered chronologically, each with a `Status:` line and task breakdown).
- `docs/Adrs/` — architectural decision records for significant choices.
- `docs/ARCHITECTURE.md` — high-level system overview (keep in sync with code).

## Work modes

### Conversation mode
Use when discussing the system more open-ended.

- Provide helpful responses.

### Design mode
Use when the task is exploratory/architectural or too large for a single PR.

- Default output is a project doc: `docs/projects/NNN-<topic>.md`
- Do not modify code unless explicitly requested (design mode is typically docs-only).
- Consider max 2-3 options, recommend one.
- End with a task breakdown of small PR-sized items, each with acceptance criteria.
- If the design changes architecture/data model/file formats or adds a significant dependency:
  - Draft/update an ADR in `docs/Adrs/`.

### Feature mode
- Smallest change that satisfies acceptance criteria.
- Avoid refactors unless required to implement the feature safely.
  - Instead, propose appropriate refactors as follow-ups.
- Add relevant tests to ensure the feature works correctly.

### Refactor mode
- Must include:
  - Clear motivation (what pain/risk it reduces).
  - A safety net (tests).
  - A bounded scope (what is NOT being refactored).

### Review mode
- Focus first on correctness:
  - Logic errors? Does the change satisfy acceptance criteria?
  - New potential bugs? Test coverage for new/changed logic?
  - Documentation in sync with changes?
- Then readability:
  - Are functions lean and focused?
  - Are names accurate and intention-revealing?
  - Is code well-organized with significant items first, helpers after?
  - Can functions be reasoned about locally?

## Development workflow (features and refactors)

1. Respond with a plan: approach, files to touch, non-goals, risks.
2. Wait for confirmation/feedback, adjust accordingly.
3. Implement exactly the plan.
4. Run:
   - `cargo fmt`
   - `cargo clippy --all-targets --all-features`
   - `cargo nextest run` (or `cargo test` if nextest not available)
5. Update docs if behavior/usage changed.
6. Update the active project doc: check off completed items, add follow-ups if needed.
7. Provide: summary of changes, tests added/updated, risks/limitations.

## What NOT to do
- No drive-by refactors.
- No new architecture without ADR.
- No adding "nice UX" unless requested.
- No "future-proofing" unless part of the task.
