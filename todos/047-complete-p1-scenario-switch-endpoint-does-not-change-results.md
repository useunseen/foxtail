---
status: complete
priority: p1
issue_id: "047"
tags: [code-review, rust, cloudwatch, cost-explorer, feature-parity]
dependencies: []
---

# Scenario switch endpoint does not affect served metrics or costs

## Problem Statement

`/_mock/scenario` reports success but only updates `resources.scenario`. Read paths for CloudWatch and Cost Explorer ignore that field, so switching scenarios does not change returned values. This makes a headline feature non-functional.

## Findings

- `services/aws-mock-data-service/src/serve.rs:191` and `services/aws-mock-data-service/src/serve.rs:197` update only `resources.scenario`.
- Metric reads in `services/aws-mock-data-service/src/metrics.rs:29` query `metrics` table directly with no scenario filter.
- Cost reads in `services/aws-mock-data-service/src/serve.rs:250` query `cost_records` directly with no scenario filter.
- Live check on `NetworkIn` values before/after scenario patch returned identical values.

## Proposed Solutions

### Option 1: Regenerate data on scenario change

**Approach:** On `/_mock/scenario`, regenerate `metrics` and `cost_records` for affected resources using scenario-specific generation.

**Pros:**
- Behavior matches endpoint semantics.
- Keeps query path simple.

**Cons:**
- Write-heavy operation on scenario updates.
- Needs careful transaction handling.

**Effort:** Medium

**Risk:** Medium

---

### Option 2: Add scenario dimension to stored data and query filters

**Approach:** Store scenario in `metrics` and `cost_records` rows (or join via resource snapshot), and filter by active scenario.

**Pros:**
- Fast scenario switching.
- Enables side-by-side scenario comparisons.

**Cons:**
- Schema migration and broader query changes.
- Higher implementation complexity.

**Effort:** Large

**Risk:** Medium

## Recommended Action

To be filled during triage.

## Technical Details

**Affected files:**
- `services/aws-mock-data-service/src/serve.rs:189`
- `services/aws-mock-data-service/src/metrics.rs:23`
- `services/aws-mock-data-service/src/generator.rs`

**Related components:**
- Admin control surface
- Scenario-driven test workflows

**Database changes:**
- Option 1: none.
- Option 2: migration required.

## Resources

- PR: `https://github.com/peterlimg/aws-optimize-agent/pull/28`
- Evidence command: `python3 - <<'PY' ... before/after /_mock/scenario ... PY`

## Acceptance Criteria

- [ ] Changing scenario changes metric and cost responses for affected resources.
- [ ] Endpoint returns explicit scope (`all` or `resource_id`) and affected record counts.
- [ ] Integration test verifies observable response changes post-switch.

## Work Log

### 2026-02-19 - Code Review Discovery

**By:** Codex

**Actions:**
- Traced scenario update and read query paths.
- Ran live request sequence against local service.

**Learnings:**
- Scenario metadata is updated, but data plane remains unchanged.

### 2026-02-19 - Resolution

**By:** Codex

**Actions:**
- Implemented and validated the fix.
- Ran targeted checks to verify expected behavior.
- Prepared change for merge.

**Learnings:**
- The issue is resolved on this branch.
