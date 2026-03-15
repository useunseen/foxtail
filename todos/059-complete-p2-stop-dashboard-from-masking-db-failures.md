---
status: complete
priority: p2
issue_id: "059"
tags: [code-review, reliability, observability, rust]
dependencies: []
---

# Stop dashboard endpoints from masking database failures

## Problem Statement

The dashboard endpoints currently convert database failures into successful empty or zero-valued responses. That makes operational faults indistinguishable from a healthy-but-empty dataset, which undermines the dashboard as a diagnostic surface and makes regressions harder to detect.

## Findings

- `build_dashboard_data()` uses `unwrap_or(0)` and `unwrap_or_default()` for core count and query paths instead of surfacing errors: [`src/serve.rs:795`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L795)-[`src/serve.rs:806`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L806), [`src/serve.rs:819`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L819)-[`src/serve.rs:821`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L821), [`src/serve.rs:860`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L860)-[`src/serve.rs:862`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L862), [`src/serve.rs:881`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L881)-[`src/serve.rs:883`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L883).
- The route handlers always serialize a 200 OK response from `build_dashboard_data()` and never distinguish degraded reads from real empty state: [`src/serve.rs:1318`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1318)-[`src/serve.rs:1360`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1360).
- This creates false confidence during troubleshooting because the admin surface silently drops evidence instead of reporting a backend problem.

## Proposed Solutions

### Option 1: Return typed dashboard errors

**Approach:** Make `build_dashboard_data()` return `Result<DashboardDataResponse, DashboardError>` and map database failures to 5xx JSON responses with a stable error code.

**Pros:**
- Makes operational failures visible immediately.
- Keeps admin responses trustworthy as diagnostics.
- Clearer contract for callers and future tests.

**Cons:**
- Requires plumbing error handling through multiple handlers.
- UI/clients may need to handle non-200 responses.

**Effort:** Small

**Risk:** Low

---

### Option 2: Expose partial-data metadata explicitly

**Approach:** Preserve successful responses, but include a top-level degraded/error flag and details when one of the backing queries fails.

**Pros:**
- Less disruptive for existing consumers.
- Still surfaces backend faults.

**Cons:**
- More ambiguous than a proper failure response.
- Easier for clients to ignore.

**Effort:** Small

**Risk:** Medium

## Recommended Action

Implemented on 2026-03-11 by converting dashboard builders/handlers to explicit `Result` flows and returning a stable 500 JSON error when dashboard queries fail.

## Technical Details

**Affected files:**
- `src/serve.rs`

**Related components:**
- Dashboard data/resource/trend endpoints
- Admin observability surface

**Database changes:**
- No

## Resources

- Current review target: commit `18148ce`

## Acceptance Criteria

- [x] Database/query failures in dashboard code paths are surfaced explicitly instead of being translated into empty success payloads.
- [x] Dashboard handlers return a stable error contract for backend faults.
- [x] Tests cover at least one failing-query path and verify that it is observable to callers.

## Work Log

### 2026-03-11 - Review Discovery

**By:** Codex

**Actions:**
- Traced all dashboard query/count reads inside `build_dashboard_data()`.
- Identified every `unwrap_or*` fallback on database access.
- Checked the route handlers that serialize dashboard payloads.

**Learnings:**
- The dashboard currently optimizes for “always return something,” even when that something is operationally misleading.

### 2026-03-11 - Resolution

**By:** Codex

**Actions:**
- Made `build_dashboard_data` return `Result`.
- Added a stable dashboard 500 response body for query failures.
- Added a regression test that closes the pool and verifies `/_mock/dashboard/data` returns a 500 error instead of empty success data.

**Learnings:**
- Error visibility can be improved without changing the successful dashboard payload contract.
