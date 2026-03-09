---
status: complete
priority: p3
issue_id: "053"
tags: [code-review, quality, hygiene]
dependencies: []
---

# Runtime server log file is committed in PR

## Problem Statement

`services/aws-mock-data-service/server.log` is included in PR changes. Runtime logs are environment-specific artifacts that add review noise and can leak operational details.

## Findings

- PR file list includes `services/aws-mock-data-service/server.log` with 108 added lines.
- Log files are not source-of-truth artifacts and should be excluded from source control.

## Proposed Solutions

### Option 1: Remove tracked log file and ignore it

**Approach:** Remove `server.log` from git history in this PR and add a scoped ignore pattern for it.

**Pros:**
- Clean repository history.
- Prevents recurring log noise.

**Cons:**
- Requires one-time cleanup.

**Effort:** Small

**Risk:** Low

---

### Option 2: Redirect runtime logs to stdout only

**Approach:** Keep app logs on stdout/stderr and avoid file logging by default.

**Pros:**
- Simpler local/CI behavior.
- Better container compatibility.

**Cons:**
- Less persistent local log storage unless explicitly configured.

**Effort:** Small

**Risk:** Low

## Recommended Action

To be filled during triage.

## Technical Details

**Affected files:**
- `services/aws-mock-data-service/server.log`

## Resources

- PR: `https://github.com/peterlimg/aws-optimize-agent/pull/28`

## Acceptance Criteria

- [ ] `server.log` is not tracked in repository.
- [ ] Log artifact ignore behavior is documented or enforced.

## Work Log

### 2026-02-19 - Code Review Discovery

**By:** Codex

**Actions:**
- Verified PR file manifest includes runtime log artifact.

**Learnings:**
- Artifact files should be filtered out before merge.

### 2026-02-19 - Resolution

**By:** Codex

**Actions:**
- Implemented and validated the fix.
- Ran targeted checks to verify expected behavior.
- Prepared change for merge.

**Learnings:**
- The issue is resolved on this branch.
