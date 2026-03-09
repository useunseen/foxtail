---
status: complete
priority: p2
issue_id: "051"
tags: [code-review, security, rust, operations]
dependencies: []
---

# Mock service admin control endpoints are unauthenticated and network-exposed

## Problem Statement

The service binds to `0.0.0.0` by default and exposes `/_mock/scenario` without authentication. In shared or misconfigured environments, any network peer can alter test behavior.

## Findings

- `services/aws-mock-data-service/src/cli.rs:72` default bind address is `0.0.0.0`.
- `services/aws-mock-data-service/src/serve.rs:26` exposes `POST /_mock/scenario`.
- `services/aws-mock-data-service/src/serve.rs:189` performs updates with no auth gate.

## Proposed Solutions

### Option 1: Restrict default bind address to loopback

**Approach:** Default to `127.0.0.1` and require explicit flag for non-local bind.

**Pros:**
- Immediate risk reduction.
- Minimal code change.

**Cons:**
- Remote access requires explicit config.

**Effort:** Small

**Risk:** Low

---

### Option 2: Add admin token gate for `/_mock/*`

**Approach:** Require a shared token header when admin endpoints are enabled.

**Pros:**
- Preserves remote usage while controlling write operations.

**Cons:**
- Slightly more configuration burden.

**Effort:** Small

**Risk:** Low

## Recommended Action

To be filled during triage.

## Technical Details

**Affected files:**
- `services/aws-mock-data-service/src/cli.rs:72`
- `services/aws-mock-data-service/src/serve.rs:25`

## Resources

- PR: `https://github.com/peterlimg/aws-optimize-agent/pull/28`

## Acceptance Criteria

- [ ] Admin mutation endpoints are not reachable by default from non-local networks.
- [ ] If remote admin is enabled, requests require explicit auth token.
- [ ] Security behavior is documented in service README.

## Work Log

### 2026-02-19 - Code Review Discovery

**By:** Codex

**Actions:**
- Reviewed router exposure and CLI defaults.

**Learnings:**
- Current defaults optimize local convenience but increase accidental exposure risk.

### 2026-02-19 - Resolution

**By:** Codex

**Actions:**
- Implemented and validated the fix.
- Ran targeted checks to verify expected behavior.
- Prepared change for merge.

**Learnings:**
- The issue is resolved on this branch.
