---
status: pending
priority: p2
issue_id: "057"
tags: [code-review, security, operations, rust]
dependencies: []
---

# Protect admin and dashboard surfaces consistently

## Problem Statement

The mock service still exposes sensitive admin-oriented read surfaces without authentication, and its write protection remains fail-open when the admin token is unset. That leaves internal inventory, cost, utilization, and scenario-control behavior under-protected for any non-local deployment or accidental proxy exposure.

## Findings

- `/_mock/status` and every `/_mock/dashboard/*` route are registered without an auth gate in [`src/serve.rs:28`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L28), [`src/serve.rs:29`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L29), [`src/serve.rs:31`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L31), [`src/serve.rs:35`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L35), and [`src/serve.rs:39`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L39).
- `ensure_admin_authorized` is only applied by `scenario_handler`, not by the read endpoints, even when `AWS_MOCK_ADMIN_TOKEN` is configured: [`src/serve.rs:503`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L503), [`src/serve.rs:597`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L597), [`src/serve.rs:1318`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1318), [`src/serve.rs:1363`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1363).
- If `AWS_MOCK_ADMIN_TOKEN` is unset or blank, `ensure_admin_authorized` returns `Ok(())`, so `POST /_mock/scenario` becomes unauthenticated by default: [`src/serve.rs:506`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L506)-[`src/serve.rs:528`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L528).
- Backend faults are serialized directly to clients with `e.to_string()` / `error.to_string()`, which can disclose internal SQLx or filesystem details: [`src/serve.rs:1374`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1374), [`src/serve.rs:1406`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1406), [`src/serve.rs:2458`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L2458).

## Proposed Solutions

### Option 1: Treat all `/_mock/*` endpoints as admin-only

**Approach:** Apply a shared auth guard to `/_mock/status`, dashboard routes, and scenario mutation, and return sanitized error envelopes for backend faults.

**Pros:**
- Consistent security model.
- Fixes both data exposure and write-surface exposure in one place.
- Lowest long-term ambiguity for operators.

**Cons:**
- Existing local tooling may need to send the admin header.
- Requires deciding whether any health endpoint should remain public.

**Effort:** Small

**Risk:** Low

---

### Option 2: Split public health from private admin data

**Approach:** Keep a minimal public liveness endpoint, but require auth for status details, dashboard data, and scenario mutation.

**Pros:**
- Preserves simple health checks.
- Keeps sensitive inventory and telemetry behind explicit auth.

**Cons:**
- Slightly more routing complexity.
- Requires clear documentation so clients know which endpoint to call.

**Effort:** Small

**Risk:** Low

## Recommended Action

Scoped out on 2026-03-11. This repo is intentionally treated as a local-only mock service, so admin/dashboard auth hardening is not an active work item unless the deployment model changes.

## Technical Details

**Affected files:**
- `src/serve.rs`

**Related components:**
- Admin route registration
- Scenario mutation endpoint
- Dashboard/status JSON responses

**Database changes:**
- No

## Resources

- Prior related review note: `todos/051-complete-p2-unauthenticated-admin-control-surface-on-mock-service.md`
- Current review target: commit `18148ce`

## Acceptance Criteria

- [ ] `/_mock/status` and `/_mock/dashboard/*` require the same auth policy as other admin endpoints, or are intentionally split into documented public/private variants.
- [ ] `/_mock/scenario` is not writable without explicit operator configuration.
- [ ] Error responses no longer expose raw backend exception strings to clients.
- [ ] Security behavior is documented for local and remote deployments.

## Work Log

### 2026-03-11 - Review Discovery

**By:** Codex

**Actions:**
- Reviewed route registration and admin authorization flow in `src/serve.rs`.
- Compared protected vs unprotected `/_mock/*` endpoints.
- Traced error serialization paths for admin, Cost Explorer, and CloudWatch handlers.

**Learnings:**
- The earlier auth hardening only covered part of the admin surface.
- Read-oriented admin endpoints still leak useful operational data when reachable.

### 2026-03-11 - Scope Decision

**By:** Codex

**Actions:**
- Confirmed with the user that admin/dashboard auth is intentionally out of scope for this local mock service.
- Removed this finding from the active implementation plan.

**Learnings:**
- Local mock-service assumptions should be validated before treating internal admin surfaces as required security work.
