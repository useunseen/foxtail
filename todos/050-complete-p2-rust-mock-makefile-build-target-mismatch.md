---
status: complete
priority: p2
issue_id: "050"
tags: [code-review, build, devex, rust]
dependencies: []
---

# Rust mock service Makefile builds debug but executes release binary path

## Problem Statement

The service Makefile compiles with `cargo build` (debug) but `gen`/`serve` invoke `target/release/aws-mock-data-service`. Fresh environments without a prior release build can fail at runtime commands.

## Findings

- `services/aws-mock-data-service/Makefile:3` sets `BIN := target/release/aws-mock-data-service`.
- `services/aws-mock-data-service/Makefile:16` runs `cargo build` and release build is commented out.
- `services/aws-mock-data-service/Makefile:20` and `services/aws-mock-data-service/Makefile:23` run `./$(BIN)`.

## Proposed Solutions

### Option 1: Align `build` to release

**Approach:** Change `build` to `cargo build --release`.

**Pros:**
- Minimal and explicit fix.
- Keeps `BIN` as configured.

**Cons:**
- Slower build times.

**Effort:** Small

**Risk:** Low

---

### Option 2: Use debug binary path by default

**Approach:** Set `BIN := target/debug/aws-mock-data-service` and optionally add `build-release` target.

**Pros:**
- Faster local iteration.
- No surprise missing binary after default build.

**Cons:**
- Release benchmark path needs explicit command.

**Effort:** Small

**Risk:** Low

## Recommended Action

To be filled during triage.

## Technical Details

**Affected files:**
- `services/aws-mock-data-service/Makefile:3`
- `services/aws-mock-data-service/Makefile:16`

## Resources

- PR: `https://github.com/peterlimg/aws-optimize-agent/pull/28`

## Acceptance Criteria

- [ ] `make setup` succeeds from clean checkout without manual release build steps.
- [ ] `make build`, `make gen`, and `make serve` are internally consistent.
- [ ] Makefile help text matches actual behavior.

## Work Log

### 2026-02-19 - Code Review Discovery

**By:** Codex

**Actions:**
- Reviewed Makefile target dependencies and binary paths.

**Learnings:**
- Build/run mode mismatch can create environment-specific failures.

### 2026-02-19 - Resolution

**By:** Codex

**Actions:**
- Implemented and validated the fix.
- Ran targeted checks to verify expected behavior.
- Prepared change for merge.

**Learnings:**
- The issue is resolved on this branch.
