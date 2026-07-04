---
status: complete
priority: p2
issue_id: "064"
tags: [code-review, cloudwatch, api, performance, quality]
dependencies: []
---

# CloudWatch GetMetricStatistics can silently truncate raw rows before aggregation

## Problem Statement

The new `GetMetricStatistics` implementation aggregates raw metric rows into period buckets, but it still fetches those raw rows through `metrics::query_metrics`, which applies a hard `LIMIT`. That means a sufficiently dense time window can return incorrect `SampleCount`, `Sum`, `Minimum`, `Maximum`, and `Average` values without any truncation signal to the caller.

This matters because the endpoint now presents itself as a standards-supporting CloudWatch surface. Returning partial aggregates with a 200 response is a correctness bug, not just a scalability concern.

## Findings

- [`src/serve.rs:5282`](../src/serve.rs#L5282) requests metric rows through `MetricQueryParams` with `limit: Some(10_000)`.
- [`src/metrics.rs:23`](../src/metrics.rs#L23) applies `LIMIT ?` directly to the raw metrics query before any bucket aggregation happens.
- The stats returned from [`src/serve.rs:5295`](../src/serve.rs#L5295) are computed from whatever subset of rows survived that limit, so large or dense windows can silently undercount or misstate results.
- No route or smoke test currently exercises a window large enough to prove that the endpoint either aggregates the full dataset or fails explicitly when it cannot.

## Proposed Solutions

### Option 1: Remove the raw-row limit for GetMetricStatistics

**Approach:** Let `GetMetricStatistics` fetch all matching rows for the requested window, then aggregate them in memory.

**Pros:**
- Preserves correctness for the existing implementation model
- Minimal change to the API contract

**Cons:**
- Increases worst-case memory and CPU usage
- Can make pathological windows expensive

**Effort:** Small

**Risk:** Medium

---

### Option 2: Keep the limit, but fail explicitly when truncation would occur

**Approach:** Detect that the raw query hit the limit and return a validation or service error instead of partial aggregates.

**Pros:**
- Avoids silently wrong results
- Keeps resource usage bounded

**Cons:**
- Degrades some requests that previously returned something
- Needs explicit contract and test updates

**Effort:** Small

**Risk:** Low

---

### Option 3: Push aggregation below the raw-row boundary

**Approach:** Aggregate at the SQL layer or with streaming reducers so the bounded unit becomes the aggregated datapoint set rather than the raw row set.

**Pros:**
- Most correct and scalable long-term shape
- Aligns the limit with what callers actually consume

**Cons:**
- Larger refactor
- More care needed to keep `GetMetricData` and `GetMetricStatistics` semantics aligned

**Effort:** Medium

**Risk:** Medium

## Recommended Action

Implemented repo-local option 2: the `GetMetricStatistics` path now fetches one extra raw row, detects truncation explicitly, and returns a validation error instead of aggregating a partial dataset.


## Technical Details

- Affected files:
  - [`src/serve.rs`](../src/serve.rs)
  - [`src/metrics.rs`](../src/metrics.rs)
- Related components:
  - CloudWatch Query/XML `GetMetricStatistics`
  - shared metric lookup path used by `GetMetricData`
- Database changes: none

## Acceptance Criteria

- [x] `GetMetricStatistics` never returns aggregates computed from a silently truncated raw metric set
- [x] If raw-row bounds remain, the endpoint returns an explicit error or continuation strategy instead of partial statistics
- [x] Regression coverage proves dense windows do not undercount `SampleCount` or distort `Sum`/`Average`
- [x] Existing `GetMetricData` behavior remains intact or is updated intentionally with matching tests

## Work Log

### 2026-03-24 - Review Finding Recorded

**By:** Codex

**Actions:**
- Reviewed commit `9e5f59a` on `main`
- Traced the new `GetMetricStatistics` handler through `MetricQueryParams` into `metrics::query_metrics`
- Identified that raw metric limiting still happens before period/stat aggregation

**Learnings:**
- The new stat support is functionally correct on small windows but can become contract-wrong on larger ones
- The important boundary is the aggregated datapoint set, not the raw metric row count

### 2026-03-24 - Resolution

**By:** Codex

**Actions:**
- Added a truncation guard in `GetMetricStatistics` so the handler fetches one sentinel raw row and returns a validation error if the window would exceed the safe raw-row cap.
- Added a regression test that seeds 10,001 raw metric rows and verifies the endpoint fails explicitly instead of returning partial aggregates.
- Kept the `GetMetricData` raw-row path unchanged.

**Learnings:**
- The correctness boundary for `GetMetricStatistics` is the raw-row cap, not the aggregated bucket count.
- An explicit validation failure is the smallest safe fix when the current implementation cannot stream or push aggregation down.

## Resources

- Review target: `9e5f59a`
- Existing related work: [`todos/058-complete-p2-scale-dashboard-and-metric-query-paths.md`](todos/058-complete-p2-scale-dashboard-and-metric-query-paths.md)
