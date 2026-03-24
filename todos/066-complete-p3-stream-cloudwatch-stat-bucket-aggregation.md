---
status: complete
priority: p3
issue_id: "066"
tags: [code-review, cloudwatch, performance, quality]
dependencies: []
---

# Stream CloudWatch statistic bucket aggregation instead of storing value vectors

## Problem Statement

The new `aggregate_metric_buckets` helper builds `Vec<f64>` collections for every bucket, then rescans each bucket to compute `SampleCount`, `Sum`, `Minimum`, `Maximum`, and `Average`. That is correct, but it carries unnecessary memory overhead and repeat work when the endpoint only needs running aggregates.

This is a low-priority optimization, but it becomes increasingly relevant as metric density or query windows grow.

## Findings

- [`src/serve.rs:2100`](../src/serve.rs#L2100) stores raw values as `BTreeMap<i64, Vec<f64>>`.
- [`src/serve.rs:2127`](../src/serve.rs#L2127) then rescans each bucket’s vector to compute the aggregate fields.
- A one-pass reducer could maintain `sample_count`, `sum`, `minimum`, and `maximum` incrementally, deriving `average` at the end without materializing every bucket member.

## Proposed Solutions

### Option 1: Replace `Vec<f64>` buckets with running aggregate structs

**Approach:** Store a per-bucket accumulator containing count, sum, min, and max; compute average when emitting `AggregatedMetricPoint`.

**Pros:**
- Lowest memory footprint for the current design
- Avoids rescanning bucket values
- Keeps the existing handler contract intact

**Cons:**
- Slightly more code in the reducer path

**Effort:** Small

**Risk:** Low

---

### Option 2: Keep vectors but short-circuit when only a subset of stats is needed

**Approach:** Preserve the current structure, but reduce some redundant work based on requested statistics.

**Pros:**
- Minimal refactor

**Cons:**
- Leaves the main memory overhead in place
- More branching for a smaller gain

**Effort:** Small

**Risk:** Low

## Recommended Action

Replace per-bucket `Vec<f64>` storage with a running accumulator that tracks count, sum, minimum, and maximum in one pass, then derive average when emitting datapoints.


## Technical Details

- Affected files:
  - [`src/serve.rs`](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs)
- Related components:
  - shared metric aggregation used by `GetMetricStatistics`
  - `GetMetricData` stat selection wrapper
- Database changes: none

## Acceptance Criteria

- [x] Bucket aggregation no longer stores every raw metric value per bucket
- [x] `SampleCount`, `Sum`, `Minimum`, `Maximum`, and `Average` are still computed correctly
- [x] Existing route tests and CLI smoke checks continue to pass
- [x] The implementation remains easy to read and reason about

## Work Log

### 2026-03-24 - Review Finding Recorded

**By:** Codex

**Actions:**
- Reviewed commit `9e5f59a` on `main`
- Analyzed the new `aggregate_metric_buckets` implementation
- Identified avoidable per-bucket vector allocation and repeated scans

**Learnings:**
- The current code is correct and acceptable at current scale
- A running-aggregate bucket model would make the shared stats path cheaper without changing behavior

### 2026-03-24 - Bucket Aggregation Streamed

**By:** Codex

**Actions:**
- Replaced per-bucket `Vec<f64>` storage in `src/serve.rs` with a `MetricBucketAccumulator` that tracks running count, sum, minimum, and maximum
- Kept the `GetMetricStatistics` response contract unchanged by deriving `Average` from the accumulator at emit time
- Added a unit test that exercises the bucket reducer across multiple values and buckets
- Updated the todo record to `complete`

**Learnings:**
- The reducer only needs running aggregates for the current CloudWatch XML path
- A small accumulator struct keeps the code readable while removing repeated scans

## Resources

- Review target: `9e5f59a`
