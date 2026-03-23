---
status: complete
priority: p2
issue_id: "060"
tags: [code-review, testing, parity, rust]
dependencies: []
---

# Add regression coverage for core mock API and generator contracts

## Problem Statement

The service has a large amount of hand-rolled request parsing, response shaping, and generator logic, but the repo currently has no executable regression tests. That leaves AWS contract drift, pagination bugs, and data-generation regressions effectively unguarded.

## Findings

- `cargo test` currently succeeds with `0 tests`, so there is no in-repo regression suite for the service.
- Core risk paths include dashboard aggregation, Cost Explorer request validation, CloudWatch JSON/XML dispatch, and generator/scenario regeneration: [`src/serve.rs:780`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L780), [`src/serve.rs:1384`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1384), [`src/serve.rs:2252`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L2252), [`src/serve.rs:2475`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L2475), [`src/generator.rs:244`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/generator.rs#L244).
- Existing repo history already records parity and generator regressions that escaped into review, including stale CloudWatch pagination parity and prior regeneration data loss: `todos/055-pending-p1-restore-cloudwatch-getmetricdata-nexttoken-parity.md`, `todos/046-complete-p1-metric-regeneration-deletes-previous-metrics.md`.
- The dashboard scorecard currently claims the supported APIs are tested even though no tests live in this repo: [`src/serve.rs:1276`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1276)-[`src/serve.rs:1287`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1287).

## Proposed Solutions

### Option 1: Add targeted integration tests around the HTTP surface

**Approach:** Spin up the Axum app against a temporary SQLite database and assert CloudWatch, Cost Explorer, dashboard, and scenario contracts end to end.

**Pros:**
- Highest confidence for contract behavior.
- Directly covers the risky request-parsing and response-shaping code.
- Pairs well with existing parity-focused todos.

**Cons:**
- More setup work than unit-only tests.
- Requires reusable test fixtures/helpers.

**Effort:** Medium

**Risk:** Low

---

### Option 2: Add a smaller unit-test layer first

**Approach:** Cover helper functions (`sum_cost_records_for_window`, pagination helpers, dashboard grouping, generation helpers) before expanding to full HTTP tests.

**Pros:**
- Faster initial coverage.
- Good fit for parsing and aggregation utilities.

**Cons:**
- Leaves routing/protocol glue largely untested.
- Easier to miss integration regressions.

**Effort:** Small to Medium

**Risk:** Medium

## Recommended Action

Implemented on 2026-03-11 with focused route-level tests that cover Cost Explorer, CloudWatch JSON, CloudWatch XML, dashboard behavior, status reporting, and scenario mutation.

## Technical Details

**Affected files:**
- `src/serve.rs`
- `src/generator.rs`
- `src/metrics.rs`
- `tests/` (new)

**Related components:**
- CloudWatch JSON/XML handlers
- Cost Explorer handlers
- Dashboard/admin endpoints
- Scenario mutation and generator flow

**Database changes:**
- No

## Resources

- Known pattern: `todos/055-pending-p1-restore-cloudwatch-getmetricdata-nexttoken-parity.md`
- Known pattern: `todos/046-complete-p1-metric-regeneration-deletes-previous-metrics.md`
- Current review target: commit `18148ce`

## Acceptance Criteria

- [x] The repo includes executable tests for at least one CloudWatch JSON path, one CloudWatch XML path, one Cost Explorer path, one dashboard path, and one generator/scenario path.
- [x] A regression test covers paginated `GetMetricData` behavior.
- [x] Test failures clearly catch response-shape or validation regressions before merge.
- [x] `cargo test` exercises real assertions rather than compile-only success.

## Work Log

### 2026-03-11 - Review Discovery

**By:** Codex

**Actions:**
- Ran `cargo test` and confirmed the crate has no test cases.
- Reviewed the highest-risk parsing, dispatch, and generation paths.
- Cross-referenced current gaps with existing parity/generator todo history.

**Learnings:**
- This service already has documented regression patterns, but no local executable safety net to keep them from returning.

### 2026-03-11 - Resolution

**By:** Codex

**Actions:**
- Added 7 route-level Rust tests in `src/serve.rs`.
- Covered status reporting, dashboard scorecard honesty, dashboard DB-failure behavior, Cost Explorer dimension pagination, CloudWatch JSON pagination, CloudWatch XML query handling, and scenario mutation.
- Ran `cargo test` successfully with all new assertions passing.

**Learnings:**
- A small route-level harness delivers much better protection here than compile-only tests, without needing a separate integration-test crate first.
