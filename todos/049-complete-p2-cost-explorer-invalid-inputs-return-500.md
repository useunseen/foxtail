---
status: complete
priority: p2
issue_id: "049"
tags: [code-review, rust, cost-explorer, error-handling]
dependencies: []
---

# Cost Explorer returns 500 for client-side validation failures

## Problem Statement

Invalid request fields (e.g., malformed date in `TimePeriod`) currently propagate as `InternalFailure` with HTTP 500. This misclassifies client errors as server errors and breaks contract expectations.

## Findings

- `services/aws-mock-data-service/src/serve.rs:243` and `services/aws-mock-data-service/src/serve.rs:244` parse dates with `?`.
- `services/aws-mock-data-service/src/serve.rs:233` maps all handler errors to `InternalFailure`/500.
- Live request with `Start=bad-date` returned `status 500` and body `{"Message":"premature end of input","__type":"InternalFailure"}`.

## Proposed Solutions

### Option 1: Classify parse/validation errors into 4xx

**Approach:** Use explicit error mapping for JSON parse/date parse and return `ValidationException` or `InvalidParameterValue` with 400.

**Pros:**
- Better AWS parity.
- Cleaner client troubleshooting.

**Cons:**
- Requires structured error typing.

**Effort:** Small

**Risk:** Low

---

### Option 2: Schema-level pre-validation

**Approach:** Introduce typed request model with validated date formats before business logic.

**Pros:**
- Consolidates input validation.
- Easier to expand for future parameters.

**Cons:**
- More refactor than immediate fix.

**Effort:** Medium

**Risk:** Low

## Recommended Action

To be filled during triage.

## Technical Details

**Affected files:**
- `services/aws-mock-data-service/src/serve.rs:224`
- `services/aws-mock-data-service/src/serve.rs:240`

## Resources

- PR: `https://github.com/peterlimg/aws-optimize-agent/pull/28`

## Acceptance Criteria

- [ ] Malformed dates return 400 with AWS-style validation error.
- [ ] Unknown/missing required fields return 400, not 500.
- [ ] Integration test covers invalid `TimePeriod` payloads.

## Work Log

### 2026-02-19 - Code Review Discovery

**By:** Codex

**Actions:**
- Traced cost handler error mapping.
- Executed malformed-date request.

**Learnings:**
- Validation and server failures are currently conflated.

### 2026-02-19 - Resolution

**By:** Codex

**Actions:**
- Implemented and validated the fix.
- Ran targeted checks to verify expected behavior.
- Prepared change for merge.

**Learnings:**
- The issue is resolved on this branch.
