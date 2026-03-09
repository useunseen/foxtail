---
status: complete
priority: p1
issue_id: "046"
tags: [code-review, rust, cloudwatch, data-integrity]
dependencies: []
---

# EC2 metric regeneration drops previously generated metrics

## Problem Statement

Metric generation for a resource deletes all existing metric rows on each metric write pass. For EC2, generation is called twice (`CPUUtilization`, then `NetworkIn`), so the second call deletes the first metric series. This breaks CloudWatch parity and invalidates key API responses.

## Findings

- `services/aws-mock-data-service/src/generator.rs:75` and `services/aws-mock-data-service/src/generator.rs:76` call `generate_mock_data_tx` twice per EC2 resource.
- `services/aws-mock-data-service/src/generator.rs:200` deletes all rows from `metrics` by `resource_id` each call.
- Runtime DB check showed only `NetworkIn` for EC2 resources; `CPUUtilization` rows were absent.
- Live API check returned empty values for EC2 `CPUUtilization` query while `NetworkIn` returned data.

## Proposed Solutions

### Option 1: Delete once per resource, then insert all metric series

**Approach:** Move `DELETE FROM metrics/cost_records WHERE resource_id = ?` outside `generate_mock_data_tx`, run once per resource, then insert all metric families.

**Pros:**
- Fixes data loss deterministically.
- Preserves current schema and query logic.
- Lowest implementation risk.

**Cons:**
- Requires refactor of generation flow.
- Needs regression tests for multi-metric resources.

**Effort:** Small

**Risk:** Low

---

### Option 2: Delete per `(resource_id, namespace, metric_name)`

**Approach:** Keep current call pattern but narrow deletion scope to specific metric key.

**Pros:**
- Minimal call-site changes.
- Keeps helper function shape.

**Cons:**
- More SQL branches.
- Still does repeated cleanup work.

**Effort:** Small

**Risk:** Medium

## Recommended Action

To be filled during triage.

## Technical Details

**Affected files:**
- `services/aws-mock-data-service/src/generator.rs:75`
- `services/aws-mock-data-service/src/generator.rs:76`
- `services/aws-mock-data-service/src/generator.rs:200`

**Related components:**
- CloudWatch JSON handler
- CloudWatch Query/XML handler
- Integration parity tests

**Database changes:**
- No schema migration required.

## Resources

- PR: `https://github.com/peterlimg/aws-optimize-agent/pull/28`
- Evidence command: `python3 - <<'PY' ... select resource_id,namespace,metric_name ... from metrics ... PY`

## Acceptance Criteria

- [ ] EC2 resources retain both `CPUUtilization` and `NetworkIn` metric series after generation.
- [ ] `GetMetricStatistics` for EC2 `CPUUtilization` returns non-empty datapoints when data exists.
- [ ] `GetMetricData` for both EC2 metrics returns correct series.
- [ ] Regression test added for multi-metric generation per resource.

## Work Log

### 2026-02-19 - Code Review Discovery

**By:** Codex

**Actions:**
- Reviewed generator flow and SQL delete scope.
- Queried local SQLite mock DB contents.
- Validated API behavior for EC2 metric retrieval.

**Learnings:**
- Multi-call generation currently overwrites previous metric families for the same resource.

### 2026-02-19 - Resolution

**By:** Codex

**Actions:**
- Implemented and validated the fix.
- Ran targeted checks to verify expected behavior.
- Prepared change for merge.

**Learnings:**
- The issue is resolved on this branch.
