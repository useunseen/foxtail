# Repository Guidelines

## Project Structure & Module Organization

This repository is a small Rust service that generates and serves AWS-like mock data. Application code lives in `src/`: `main.rs` wires the CLI, `serve.rs` hosts the Axum API, `generator.rs` seeds data, `db.rs` initializes SQLite, and `handlers/` contains service-specific response shaping. SQL migrations live in `migrations/`. Planning notes are stored in `docs/plans/`, and file-based work tracking lives in `todos/`. Treat `target/` and `mock_data.db` as generated artifacts.

## Build, Test, and Development Commands

- `make build`: build the debug binary at `target/debug/foxtail`.
- `make build-release`: build an optimized release binary.
- `make gen`: discover resources and regenerate `mock_data.db`.
- `make gen-baseline`, `make gen-spike`, `make gen-idle-heavy`: seed specific traffic scenarios.
- `make serve`: run the API locally on `127.0.0.1:8080`.
- `make setup`: build, then generate baseline data in one step.
- `cargo test`: run the current Rust test suite and integration tests; use it as the default regression gate plus compile check.
- `cargo fmt` and `cargo clippy --all-targets --all-features`: run before opening a PR.

## Coding Style & Naming Conventions

Use standard Rust formatting with 4-space indentation and `cargo fmt`. Follow existing naming: `snake_case` for modules, files, functions, and fields; `PascalCase` for structs and enums; `SCREAMING_SNAKE_CASE` for constants. Keep route dispatch and endpoint orchestration in `serve.rs`, data access in `db.rs` or query helpers, and reusable protocol serializers or extracted response builders inside `src/handlers/`.

## Testing Guidelines

Add focused unit tests next to the code they cover with `#[cfg(test)] mod tests`, and add integration tests under `tests/` if a change crosses CLI, database, or HTTP boundaries. Name tests after observable behavior, for example `returns_unauthorized_without_admin_token`. Validate both JSON and XML-facing paths when touching AWS-compatible handlers.

## Commit & Pull Request Guidelines

This checkout has no commit history yet, so no repository-specific commit style can be inferred. Use short, imperative subjects; `feat:`, `fix:`, and `refactor:` prefixes fit the existing docs and todo naming. PRs should state the user-visible change, list verification commands run, note any schema or env var changes, and include response samples or screenshots when API/dashboard behavior changes.

## Security & Configuration Tips

Local runs depend on `DATABASE_URL`, `AWS_ENDPOINT_URL`, and `AWS_DEFAULT_REGION`. Protect admin routes with `AWS_MOCK_ADMIN_TOKEN`; clients must send `x-mock-admin-token` when that variable is set.

## Workflow Orchestration Rules

### 1. Plan Mode Default

- Enter plan mode for any non-trivial task (3+ steps or architectural decisions)
- If something goes sideways, stop and re-plan immediately; do not keep pushing
- Use plan mode for verification steps, not just building
- Write detailed specs upfront to reduce ambiguity

### 2. Subagent Strategy

- Use subagents liberally to keep the main context window clean
- Offload research, exploration, and parallel analysis to subagents
- For complex problems, throw more compute at it via subagents
- One task per subagent for focused execution

### 3. Self-Improvement Loop

- After any correction from the user, update `tasks/lessons.md` with the pattern
- Write rules for yourself that prevent the same mistake
- Iterate on these lessons until the mistake rate drops
- Review lessons at session start for relevant projects

### 4. Verification Before Done

- Never mark a task complete without proving it works
- Diff behavior between main and your changes when relevant
- Ask yourself: "Would a staff engineer approve this?"
- Run tests, check logs, and demonstrate correctness

### 5. Demand Elegance (Balanced)

- For non-trivial changes, pause and ask: "Is there a more elegant way?"
- If a fix feels hacky, use: "Knowing everything I know now, implement the elegant solution"
- Skip this for simple, obvious fixes; do not over-engineer
- Challenge your own work before presenting it

### 6. Autonomous Bug Fixing

- When given a bug report, fix it without hand-holding
- Point at logs, errors, or failing tests, then resolve them
- Require zero context switching from the user
- Fix failing CI tests without being told how

## Task Management Rules

1. **Plan First**: Write a plan to `tasks/todo.md` with checkable items
2. **Verify Plans**: Check in before starting implementation
3. **Track Progress**: Mark items complete as you go
4. **Explain Changes**: Provide a high-level summary at each step
5. **Document Results**: Add a review section to `tasks/todo.md`
6. **Capture Lessons**: Update `tasks/lessons.md` after corrections

## Core Principles

- **Simplicity First**: Make every change as simple as possible. Impact minimal code.
- **No Laziness**: Find root causes. No temporary fixes. Senior developer standards.
- **Minimal Impact**: Changes should only touch what is necessary. Avoid introducing bugs.
