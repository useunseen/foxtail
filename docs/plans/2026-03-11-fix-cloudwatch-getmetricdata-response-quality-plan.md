---
title: fix: clean up CloudWatch GetMetricData response quality
type: fix
date: 2026-03-11
---

# fix: clean up CloudWatch GetMetricData response quality

## Overview
Tighten the public CloudWatch `GetMetricData` handler so AWS CLI and SDK callers get query-scoped, period-aware, deterministic results instead of the current noisy dump of raw metric rows. The goal is to make the public CloudWatch surface genuinely usable for FinOps correlation work after the Cost Explorer interoperability fixes.

## Problem Statement
`cloudwatch get-metric-data` is reachable and returns a 200, but the response quality is still poor:

- result sets can contain repeated timestamps and too many values for a single scoped query
- the returned `Id` can drift from the caller’s requested query id
- the handler currently emits raw metric rows instead of period/stat-aggregated datapoints
- pagination is applied as a simple row slice, not as stable contract-aware traversal
- the implementation ignores important request semantics such as `MetricStat.Period`, `MetricStat.Stat`, and consistent per-query shaping

This makes `GetMetricData` technically callable but not trustworthy for FinOps analysis or parity-grade testing.

## Research Consolidation

### Internal Repo Findings
- The JSON handler reads the body as untyped `serde_json::Value`, extracts `MetricDataQueries`, and then forwards the request directly to `metrics::query_metrics` with `limit: Some(10_000)`: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L3015).
- The current response is built by copying raw timestamps and values from queried rows without period bucketing or stat calculation: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L3119).
- Pagination uses one global `page_start` / `max_datapoints` slice across each query result and emits a single top-level `NextToken`, which is only partial parity: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L3055).
- Existing tests verify only that `NextToken` appears when truncated; they do not verify query identity, period aggregation, or non-duplicated timestamp behavior: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L3260).
- Prior parity planning already called out `GetMetricData` pagination/output parity as unfinished and critical to avoid false confidence: [docs/plans/2026-02-19-test-comprehensive-aws-api-parity-suite-plan.md](/Users/murphy/workspace/iacai0/foxtail/docs/plans/2026-02-19-test-comprehensive-aws-api-parity-suite-plan.md#L234).

### Institutional Learnings
- No relevant `docs/solutions/` entries exist in this checkout.
- Existing parity plans consistently treat CloudWatch contract realism as a first-class invariant, not an optional improvement.

### External Research Decision
This is an external AWS API contract fix, so external research is required.

### External Documentation Highlights
- `GetMetricData` is period/stat-driven, not a raw row export.
- `MetricDataResults[].Id` must mirror the caller’s query id.
- `Timestamps` and `Values` must align one-to-one per result.
- `NextToken` must represent stable continuation semantics rather than arbitrary output slicing.
- CloudWatch supports multiple queries per request, so output shaping must stay query-local even when paginating.

## Scope

### In Scope
- [x] Parse `GetMetricData` requests into typed structures rather than ad hoc `serde_json::Value` access.
- [x] Respect `MetricStat.Metric`, `MetricStat.Period`, and `MetricStat.Stat` for basic aggregation.
- [x] Return query-scoped `MetricDataResults` with correct ids, aligned timestamps/values, and stable ordering.
- [x] Clean up `NextToken` semantics so repeated pagination requests are deterministic and non-overlapping.
- [x] Add targeted tests and AWS CLI smoke verification for clean `GetMetricData` behavior.

### Out of Scope
- Metric math expressions.
- Full CloudWatch alarm/statistics parity beyond the `GetMetricData` contract needed here.
- New helper endpoints.
- Redesigning the underlying seeded data model.

## SpecFlow Analysis

### User Flow Overview
1. Engineer discovers metric names and dimensions via `cloudwatch list-metrics`.
2. Engineer requests one or more metric queries using `cloudwatch get-metric-data`.
3. Service resolves each query to the requested resource/metric/namespace.
4. Service buckets raw rows by period and computes the requested stat.
5. Service returns one clean `MetricDataResult` per query with matching `Id`, aligned timestamps/values, and a stable `NextToken` when needed.
6. Engineer correlates cost with utilization without needing dashboard-only helper APIs.

### Flow Permutations Matrix

| Dimension | Variants |
|---|---|
| Query count | single query, multiple queries |
| Scope | resource-scoped, namespace+metric scoped |
| Statistic | `Average`, `Sum`, `Minimum`, `Maximum` |
| Period | 300s, 3600s, larger windows |
| Pagination | no token, token continuation, invalid token |
| Output state | complete result, empty result, truncated result |

### Missing Elements and Gaps
- **Request typing gap**: loose JSON parsing makes it easy to mis-handle caller shape.
- **Aggregation gap**: output currently reflects stored rows, not CloudWatch period/stat semantics.
- **Identity gap**: response ids and query-local shaping are not strongly enforced.
- **Pagination gap**: continuation tokens are not clearly tied to stable query output.
- **Verification gap**: no tests assert against duplicate timestamps or query-id fidelity.

### Critical Questions Requiring Clarification
1. Important: Which stat set should be supported in the first pass?
Resolution: implemented `Average`, `Sum`, `Minimum`, and `Maximum` only.
2. Important: Should pagination be applied to the flattened union of all query outputs or per query result set?
Resolution: kept one top-level token offset and applied it deterministically across per-query output series, allowing shorter series to return empty pages rather than erroring.
3. Nice-to-have: Should empty query results return empty arrays with `StatusCode: Complete`, or emulate a more specific CloudWatch status?
Resolution: return empty arrays with `StatusCode: Complete`.

## Technical Approach

### 1. Replace Loose JSON Parsing with Typed Request Models
- Introduce Rust structs for:
  - `GetMetricDataRequest`
  - `MetricDataQuery`
  - `MetricStat`
  - `Metric`
  - `Dimension`
- Validate required fields explicitly and early.
- Preserve the existing 50-query ceiling and invalid token handling.

### 2. Add Query-Scoped Aggregation
- Resolve each query to its metric filter (`Namespace`, `MetricName`, dimensions/resource id).
- Pull raw metric rows from SQLite.
- Bucket rows into period windows based on `StartTime`, `EndTime`, and `MetricStat.Period`.
- Compute the requested statistic per bucket.
- Emit one timestamp/value pair per bucket, ordered deterministically.

### 3. Fix Result Identity and Shape
- Always echo the caller’s `Id`.
- Keep `Timestamps.len() == Values.len()` for every query result.
- Ensure a single-query request cannot accidentally leak rows from other resources or metrics.
- Preserve `Messages: []` for shape compatibility unless a modeled warning path is added.

### 4. Rework Pagination Semantics
- Define a canonical ordering for paginated results.
- Make `NextToken` encode a stable continuation point for the cleaned output, not the raw input rows.
- Ensure token round-trips do not overlap or skip datapoints.
- Keep invalid token responses on the existing `InvalidNextToken` path.

### 5. Expand Verification
- Add route tests for:
  - caller `Id` preserved in response
  - no duplicate timestamps for a simple single-resource query
  - period/stat aggregation over seeded rows
  - deterministic `NextToken` traversal with no overlap
  - invalid token error behavior
- Extend CLI smoke verification with one explicit `get-metric-data` assertion against expected shape, not just status code.

## Acceptance Criteria
- [x] `cloudwatch get-metric-data` returns one `MetricDataResult` per requested query.
- [x] `MetricDataResults[].Id` matches the caller-provided query id.
- [x] `Timestamps` and `Values` are aligned and free of duplicate bucket entries for a simple single-resource query.
- [x] `MetricStat.Period` and `MetricStat.Stat` affect the returned datapoints.
- [x] A resource-scoped query does not leak unrelated resources’ datapoints.
- [x] `NextToken` pagination is deterministic and non-overlapping across repeated requests.
- [x] Invalid `NextToken` still returns an AWS-style error envelope.
- [x] Existing `ListMetrics` and `GetMetricStatistics` behavior remains intact.
- [x] Route tests cover the cleaned result shape and pagination behavior.
- [x] A CLI smoke check proves the new result shape on a live local server.

## Implementation Phases

### Phase 1: Typed Contract and Core Aggregation
- [x] Add typed `GetMetricData` request structs.
- [x] Map query filters into a clean internal query model.
- [x] Implement period/stat bucketing for the first-pass statistic set.

### Phase 2: Response Shape and Pagination
- [x] Preserve query ids exactly.
- [x] Emit clean aligned timestamps/values per query.
- [x] Rework `NextToken` traversal over canonical output ordering.

### Phase 3: Verification
- [x] Add focused route tests for id fidelity, aggregation, deduplication, and token traversal.
- [x] Extend the CLI smoke script with output-shape assertions for `get-metric-data`.
- [x] Re-run manual AWS CLI checks against `127.0.0.1:8080`.

## Execution Notes
- Added typed JSON request models for CloudWatch JSON `GetMetricData` and reused one aggregation helper for both JSON and Query/XML handlers.
- Added first-pass period/stat aggregation for `Average`, `Sum`, `Minimum`, and `Maximum`.
- Fixed the public AWS CLI path by parsing `MetricDataQueries.member.1.*` fields in the CloudWatch Query/XML handler instead of defaulting to `m1` and raw unscoped rows.
- Pagination now slices the cleaned per-query series deterministically and allows shorter series to exhaust without failing later pages.

## Risks and Mitigations
- **Risk:** trying to emulate full CloudWatch semantics will bloat the fix.
  - **Mitigation:** limit first pass to typed parsing, basic stats, clean shape, and deterministic pagination.
- **Risk:** pagination changes could break the existing `NextToken` test.
  - **Mitigation:** replace the current shallow token assertion with stronger end-to-end traversal tests.
- **Risk:** seeded data granularity may not align perfectly with requested periods.
  - **Mitigation:** bucket existing rows deterministically and document the supported granularity assumptions in tests.

## Verification Plan
- `cargo fmt`
- `cargo test`
- `cargo clippy --all-targets --all-features`
- `aws --endpoint-url http://127.0.0.1:8080 cloudwatch list-metrics --namespace AWS/EC2 --metric-name CPUUtilization`
- `aws --endpoint-url http://127.0.0.1:8080 cloudwatch get-metric-data --metric-data-queries file://metric_queries.json --start-time 2026-03-11T00:00:00Z --end-time 2026-03-11T12:00:00Z`
- verify:
  - caller query id is preserved
  - timestamps are bucketed cleanly
  - no unrelated resources leak into the result
  - pagination token traversal is stable

## References
- AWS CloudWatch `GetMetricData`: https://docs.aws.amazon.com/AmazonCloudWatch/latest/APIReference/API_GetMetricData.html
- Existing parity roadmap: [2026-02-19-test-comprehensive-aws-api-parity-suite-plan.md](/Users/murphy/workspace/iacai0/foxtail/docs/plans/2026-02-19-test-comprehensive-aws-api-parity-suite-plan.md)
