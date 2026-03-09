---
status: complete
priority: p2
issue_id: "048"
tags: [code-review, rust, cloudwatch, protocol-parity]
dependencies: []
---

# CloudWatch GetMetricData handles only first query and hardcodes response ID

## Problem Statement

`GetMetricData` JSON handling reads only the first `MetricDataQueries` element and always responds with `Id = "m1"`. Multi-query requests lose data and violate AWS response shape expectations.

## Findings

- `services/aws-mock-data-service/src/serve.rs:389` fetches only index `0` from `MetricDataQueries`.
- `services/aws-mock-data-service/src/serve.rs:425` returns exactly one `MetricDataResults` entry with hardcoded ID.
- Live call with two queries returned `results_count = 1` and `ids = ['m1']`.

## Proposed Solutions

### Option 1: Iterate over all query members

**Approach:** Parse each query in `MetricDataQueries`, resolve parameters per query, run metric lookup, emit one result per request `Id`.

**Pros:**
- Correct AWS-compatible behavior.
- Backward-compatible with single-query clients.

**Cons:**
- More parsing and response assembly logic.

**Effort:** Medium

**Risk:** Low

---

### Option 2: Enforce single-query explicitly

**Approach:** Reject requests with `MetricDataQueries.len() > 1` using validation error.

**Pros:**
- Clear contract, minimal code.

**Cons:**
- Intentionally diverges from AWS API.
- Breaks callers expecting multi-query batching.

**Effort:** Small

**Risk:** Medium

## Recommended Action

To be filled during triage.

## Technical Details

**Affected files:**
- `services/aws-mock-data-service/src/serve.rs:383`
- `services/aws-mock-data-service/src/serve.rs:425`

## Resources

- PR: `https://github.com/peterlimg/aws-optimize-agent/pull/28`

## Acceptance Criteria

- [ ] Response contains one `MetricDataResults` member per input query.
- [ ] Returned IDs match request IDs.
- [ ] Added integration test covering 2+ query entries.

## Work Log

### 2026-02-19 - Code Review Discovery

**By:** Codex

**Actions:**
- Reviewed JSON parsing logic.
- Executed multi-query request and inspected result count.

**Learnings:**
- Current implementation silently truncates query batch.

### 2026-02-19 - Resolution

**By:** Codex

**Actions:**
- Implemented and validated the fix.
- Ran targeted checks to verify expected behavior.
- Prepared change for merge.

**Learnings:**
- The issue is resolved on this branch.
