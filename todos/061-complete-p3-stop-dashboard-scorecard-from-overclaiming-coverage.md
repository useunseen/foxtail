---
status: complete
priority: p3
issue_id: "061"
tags: [code-review, documentation, parity, dashboard]
dependencies: ["060"]
---

# Stop dashboard scorecard from overclaiming coverage

## Problem Statement

The dashboard response currently presents hardcoded perfect coverage benchmarks and equates “implemented” with “implemented and tested.” That misleads users of the admin surface and creates drift between the service’s reported confidence level and the actual verification state.

## Findings

- `implemented_tested_entries` is set to `supported_apis.len()` regardless of real test coverage: [`src/serve.rs:1276`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1276)-[`src/serve.rs:1279`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1279).
- The benchmark block is hardcoded to `1.0` for operation, input, output, and error coverage: [`src/serve.rs:1281`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1281)-[`src/serve.rs:1287`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1287).
- The repo currently has no executable tests in `cargo test`, so the scorecard materially overstates verification.

## Proposed Solutions

### Option 1: Derive scorecard data from real verification artifacts

**Approach:** Source tested/coverage fields from generated evidence or explicit test metadata rather than hardcoded literals.

**Pros:**
- Restores trust in the scorecard.
- Keeps the dashboard aligned with actual verification state.

**Cons:**
- Requires an evidence source or test-report integration.
- Slightly more plumbing than a static payload.

**Effort:** Medium

**Risk:** Low

---

### Option 2: Downgrade the scorecard to “implemented only”

**Approach:** Remove or null out tested/benchmark claims until real verification evidence exists.

**Pros:**
- Fastest honest fix.
- Avoids false precision.

**Cons:**
- Less impressive dashboard output.
- Still leaves richer evidence work for later.

**Effort:** Small

**Risk:** Low

## Recommended Action

Implemented on 2026-03-11 by setting tested/benchmark values to honest zero-state defaults until a real verification artifact source exists.

## Technical Details

**Affected files:**
- `src/serve.rs`

**Related components:**
- Dashboard scorecard payload
- Verification/reporting workflow

**Database changes:**
- No

## Resources

- Depends on stronger verification coverage from `060-pending-p2-add-regression-coverage-for-core-mock-contracts.md`
- Current review target: commit `18148ce`

## Acceptance Criteria

- [x] The dashboard no longer claims perfect tested coverage without supporting evidence.
- [x] “Implemented” and “tested” are reported as distinct concepts.
- [x] Any benchmark numbers shown in the payload are traceable to a real verification source.

## Work Log

### 2026-03-11 - Review Discovery

**By:** Codex

**Actions:**
- Inspected the dashboard scorecard payload construction in `src/serve.rs`.
- Compared reported tested coverage to the repo’s current executable test state.

**Learnings:**
- The current scorecard is acting as aspirational documentation, not measured truth.

### 2026-03-11 - Resolution

**By:** Codex

**Actions:**
- Replaced the hardcoded perfect benchmark/tested values with honest zero-state placeholders.
- Added a dashboard regression test that asserts the scorecard no longer overclaims verification.

**Learnings:**
- If a verification source does not exist yet, the payload should say so plainly instead of inferring confidence from implementation count.
