---
title: fix: restore standard statistics in CloudWatch GetMetricStatistics
type: fix
status: completed
date: 2026-03-24
---

# fix: restore standard statistics in CloudWatch GetMetricStatistics

## Overview

Foxtail's public CloudWatch Query/XML `GetMetricStatistics` path currently emits datapoints under `Average` only, even when callers request `Maximum` for metrics like `CPUUtilization`. The reported `CPUUtilization` failure is the visible symptom, but the underlying contract gap is broader: the endpoint does not parse `Statistics.member.N`, does not validate `ExtendedStatistics`, and does not aggregate datapoints by the requested `Period` before shaping the XML response.

This matters because `aws cloudwatch get-metric-statistics --statistics Maximum ...` is a normal AWS CLI workflow, and Foxtail positions its public CloudWatch surface as AWS CLI-compatible.

This plan now explicitly treats the full standard `Statistics` set as in scope:

- `SampleCount`
- `Average`
- `Sum`
- `Minimum`
- `Maximum`

## Problem Statement

The current implementation has three connected issues:

- The Query/XML request model for CloudWatch does not capture any `Statistics.member.N` or `ExtendedStatistics.member.N` inputs, so the handler has no record of which statistics the caller requested: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L64).
- The `GetMetricStatistics` handler queries raw metric rows and serializes every datapoint as `<Average>...`, regardless of request intent or bucket size: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L5085).
- The XML response model hardcodes `Average` as the only supported datapoint field, which makes `Maximum`, `Minimum`, `Sum`, and `SampleCount` impossible to emit even if the handler computed them: [src/handlers/cloudwatch.rs](/Users/murphy/workspace/iacai0/foxtail/src/handlers/cloudwatch.rs#L47).

The result is that a command like the following can never round-trip correctly today:

```bash
aws --endpoint-url http://127.0.0.1:8080 cloudwatch get-metric-statistics \
  --namespace AWS/EC2 \
  --metric-name CPUUtilization \
  --statistics Maximum \
  --period 3600 \
  --start-time 2026-03-11T00:00:00Z \
  --end-time 2026-03-11T12:00:00Z \
  --dimensions Name=InstanceId,Value=i-20652c71bedc57ced
```

## Research Consolidation

### Internal Repo Findings

- The request model used by the CloudWatch Query/XML router includes action, metric identity, period, pagination, and two dimensions, but no statistic-selection fields: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L64).
- `handle_get_metric_statistics` builds `MetricQueryParams`, fetches rows from SQLite, and maps each point directly to a `cw::Datapoint` with `average: p.value`; the requested `Period` is only validated for presence, not used to aggregate output: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L5085).
- `GetMetricData` already has a shared aggregation path that buckets by period and computes `Average`, `Sum`, `Minimum`, and `Maximum`, which is the obvious reuse point rather than adding a second stats implementation; this fix should extend that path or wrap it so `SampleCount` is derived from the same period buckets: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L1122), [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L1942).
- Existing XML coverage for `GetMetricStatistics` only asserts that the response contains `GetMetricStatisticsResponse` and `CPUUtilization`; it does not assert requested-stat fidelity, aggregation semantics, or XML field names: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L6420).
- The CLI smoke script covers only `--statistics Average` for `NetworkIn`, so the `Maximum` regression had no runtime guardrail: [scripts/verify_cli_interop.sh](/Users/murphy/workspace/iacai0/foxtail/scripts/verify_cli_interop.sh#L282).
- The README documents `cloudwatch get-metric-statistics` as a public interface and broadly notes supported stats under the CloudWatch section, but does not reflect that the current XML implementation is still effectively `Average`-only: [README.md](/Users/murphy/workspace/iacai0/foxtail/README.md#L79), [README.md](/Users/murphy/workspace/iacai0/foxtail/README.md#L160).

### Institutional Learnings

- No relevant `docs/solutions/` entries exist in this checkout.
- Recent CloudWatch work already established a shared aggregation helper for `GetMetricData`, so the simplest durable fix is to extend and reuse that logic instead of layering another one-off XML-only stat mapper.

### External Research Decision

External research is required because this is an AWS API contract bug on a public compatibility surface.

### External Documentation Highlights

- AWS CLI documents `GetMetricStatistics` as requiring either `Statistics` or `ExtendedStatistics`, but not both, and lists valid non-percentile statistics as `SampleCount`, `Average`, `Sum`, `Minimum`, and `Maximum`: https://docs.aws.amazon.com/cli/latest/reference/cloudwatch/get-metric-statistics.html
- AWS CLI examples explicitly use `--statistics Maximum` for `CPUUtilization`, which matches the reported failure mode here: https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/US_SingleMetricPerInstance.html
- AWS documents that `GetMetricStatistics` aggregates raw data into datapoints based on the requested period, which means Foxtail should not continue returning raw stored rows as if they were already aggregated statistics: https://docs.aws.amazon.com/cli/latest/reference/cloudwatch/get-metric-statistics.html
- AWS API reference for the endpoint: https://docs.aws.amazon.com/AmazonCloudWatch/latest/APIReference/API_GetMetricStatistics.html

## Proposed Solution

### 1. Parse statistic selection explicitly on the Query/XML path

- Add a dedicated `GetMetricStatistics` request parser instead of continuing to overload the generic `CloudWatchQuery` struct.
- Parse repeated `Statistics.member.N` values, and parse `ExtendedStatistics.member.N` separately so the handler can validate AWS-style mutual exclusivity.
- Preserve the existing dimension parsing behavior, but move stat selection into a typed request model so the endpoint no longer silently defaults to `Average`.

### 2. Reuse the shared aggregation path instead of duplicating stat logic

- Route `GetMetricStatistics` through the existing period/stat aggregation helper used by `GetMetricData`: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L1942).
- Extend that helper, or add a thin wrapper around it, so `GetMetricStatistics` can compute all requested standard statistics from one bucket pass.
- Support the standard non-percentile statistics in this fix:
  - `SampleCount`
  - `Average`
  - `Sum`
  - `Minimum`
  - `Maximum`
- Compute `SampleCount` from the number of raw points in each period bucket using the same bucket boundaries as the other statistics.
- Return a clear validation error for `ExtendedStatistics` rather than silently ignoring it. Percentile support can remain a follow-up if needed.

### 3. Make the XML datapoint model stat-aware

- Replace the hardcoded `average: f64` field in `cw::Datapoint` with optional fields for each supported statistic so the serializer only emits the fields the caller asked for.
- Keep `Timestamp` and `Unit` stable.
- Preserve deterministic timestamp ordering even though AWS does not guarantee chronological order; deterministic output is already the repo's preferred testing posture.

### 4. Tighten validation and docs around the actual contract

- Reject requests that specify both `Statistics` and `ExtendedStatistics`.
- Reject requests that specify neither.
- Reject unsupported stat names with an AWS-style validation error instead of returning a misleading 200 with `Average`.
- Update README examples and notes so they describe the actual supported `GetMetricStatistics` contract after the fix lands.

### 5. Add regression coverage for the missing `Maximum` path

- Add route tests that assert XML contains `<Maximum>` when `Maximum` is requested for `CPUUtilization`.
- Add a multi-row, same-period case so `Maximum` proves real aggregation rather than simple field renaming.
- Add explicit route coverage for `SampleCount`, `Sum`, and `Minimum` so the remaining standard statistics are validated individually.
- Add a multi-stat case that verifies one datapoint can contain `SampleCount`, `Average`, `Sum`, `Minimum`, and `Maximum` together when all are requested.
- Extend CLI smoke verification to cover a `CPUUtilization --statistics Maximum` call and fail if the result only contains `Average`.
- Extend CLI smoke verification to cover a multi-stat `CPUUtilization` call and assert the additional standard statistic fields are present.

## System-Wide Impact

### Interaction Graph

`POST /` CloudWatch Query/XML request -> form parsing in [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L4821) -> `GetMetricStatistics` request validation -> SQLite metric lookup through [src/metrics.rs](/Users/murphy/workspace/iacai0/foxtail/src/metrics.rs) -> shared stat aggregation helper in [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L1942) -> XML serialization via [src/handlers/cloudwatch.rs](/Users/murphy/workspace/iacai0/foxtail/src/handlers/cloudwatch.rs) -> AWS CLI XML parsing into JSON output for the user.

### Error Propagation

- Request-shape problems should remain `MissingRequiredParameterException` or `InvalidParameterValueException` on the Query/XML surface.
- Unsupported or conflicting statistic selections should fail before database access.
- Internal database or XML serialization failures should continue to surface as `InternalFailure`.

### State Lifecycle Risks

- This is a read-only path. No schema, migration, or data mutation risk is expected.
- The main correctness risk is behavioral drift: existing `Average` callers must continue working while additional stats become available.

### API Surface Parity

- Affects the public CloudWatch Query/XML path used by `aws cloudwatch get-metric-statistics`.
- Indirectly affects the `foxtail` wrapper because it routes CloudWatch CLI calls to this same endpoint: [src/wrapper.rs](/Users/murphy/workspace/iacai0/foxtail/src/wrapper.rs#L346).
- Should not change `GetMetricData`, but the plan intentionally reuses its aggregation helper to reduce future parity drift.

### Integration Test Scenarios

- `CPUUtilization` with `--statistics Maximum` returns datapoints containing `Maximum` and no forced `Average`.
- A one-hour period covering multiple raw rows returns the correct maximum for that bucket, not a raw input row.
- `--statistics SampleCount Average Sum Minimum Maximum` returns all five fields on each datapoint when all are requested.
- A bucket with multiple raw rows returns correct values for `SampleCount`, `Sum`, `Minimum`, `Maximum`, and `Average` together.
- Requests that provide both `Statistics` and `ExtendedStatistics` fail with a validation error.
- Existing `NetworkIn --statistics Average` smoke coverage still passes after the refactor.

## SpecFlow Analysis

### User Flow Overview

1. Engineer discovers an instance id with `cloudwatch list-metrics`.
2. Engineer calls `cloudwatch get-metric-statistics` for `CPUUtilization`.
3. Engineer requests one or more standard statistics such as `SampleCount`, `Average`, `Sum`, `Minimum`, or `Maximum`.
4. Foxtail resolves the metric scope and loads raw points from SQLite.
5. Foxtail aggregates those raw points into period buckets.
6. Foxtail returns XML datapoints whose stat fields match the request.
7. AWS CLI renders the parsed XML as JSON containing the requested standard statistic fields.

### Flow Permutations Matrix

| Dimension | Variants |
|---|---|
| Metric | `CPUUtilization`, `NetworkIn`, other seeded metrics |
| Statistic input | single standard stat, all five standard stats, unsupported stat, missing stat |
| Period shape | one raw point per bucket, multiple raw points per bucket |
| Protocol consumer | direct AWS CLI, `foxtail` wrapper |
| Validation state | `Statistics` only, `ExtendedStatistics` only, both, neither |

### Gaps To Address

- No typed stat-selection parsing.
- No period/stat aggregation on `GetMetricStatistics`.
- No serializer support for requested stat fields.
- No runtime verification for `Maximum`.

## Acceptance Criteria

- [x] `aws cloudwatch get-metric-statistics --statistics Maximum` against Foxtail returns datapoints with `Maximum` for `CPUUtilization`.
- [x] `GetMetricStatistics` aggregates raw metric rows by the requested `Period` before returning datapoints.
- [x] The Query/XML path supports `Average`, `Sum`, `Minimum`, `Maximum`, and `SampleCount`.
- [x] `SampleCount` reflects the number of raw rows inside each returned period bucket.
- [x] Multi-stat requests return all requested standard stat fields on each datapoint.
- [x] Requests with unsupported stats, missing stats, or both `Statistics` and `ExtendedStatistics` fail with clear validation errors.
- [x] Existing `Average` behavior remains correct for current callers and smoke tests.
- [x] Route coverage includes a regression test for the original `Maximum` failure case.
- [x] Route coverage includes explicit assertions for `SampleCount`, `Sum`, and `Minimum`.
- [x] CLI smoke verification includes at least one `CPUUtilization --statistics Maximum` assertion.
- [x] CLI smoke verification includes at least one multi-stat `CPUUtilization` assertion covering more than `Average` and `Maximum`.
- [x] README examples and notes accurately describe supported `GetMetricStatistics` behavior after the fix.

## Success Metrics

- The reported `CPUUtilization` `Maximum` call succeeds without manual XML inspection hacks.
- The full standard `Statistics` set succeeds on the public Query/XML path, not just `Average` plus `Maximum`.
- New tests fail if the endpoint regresses back to `Average`-only serialization.
- The public CloudWatch surface stays internally consistent: `GetMetricData` and `GetMetricStatistics` use the same core stat semantics where overlap exists.

## Dependencies & Risks

### Dependencies

- No schema or migration work is needed.
- The plan depends on the existing shared aggregation helper remaining the canonical place for period/stat math.

### Risks

- Expanding the XML datapoint struct could unintentionally change the shape of existing `Average` responses.
- Supporting multiple stats per datapoint is broader than the original bug report, so scope should stay limited to the five standard statistics and not slide into percentile support.
- Reusing `GetMetricData` aggregation logic will likely require a small extension because `SampleCount` is not currently computed there.

### Mitigations

- Add focused regression tests before widening smoke coverage.
- Keep `ExtendedStatistics` explicitly unsupported for now with clear validation.
- Reuse one aggregation path to avoid stat math drift across endpoints.

## Implementation Suggestions

- Prefer a dedicated `parse_get_metric_statistics_request_from_form` helper over adding more ad hoc fields to `CloudWatchQuery`.
- Model XML datapoints with optional stat fields and `skip_serializing_if = "Option::is_none"` so only requested stats appear.
- Extend the shared bucket representation so one aggregation pass can produce `SampleCount`, `Average`, `Sum`, `Minimum`, and `Maximum` without recomputing bucket membership separately for each field.

## Verification Plan

- `cargo fmt`
- `cargo test`
- `cargo clippy --all-targets --all-features`
- `bash scripts/verify_cli_interop.sh`
- Manual smoke check:

```bash
aws --endpoint-url http://127.0.0.1:8080 cloudwatch get-metric-statistics \
  --namespace AWS/EC2 \
  --metric-name CPUUtilization \
  --statistics Maximum \
  --period 3600 \
  --start-time 2026-03-11T00:00:00Z \
  --end-time 2026-03-11T12:00:00Z \
  --dimensions Name=InstanceId,Value=<instance-id>
```

- Optional parity check:

```bash
aws --endpoint-url http://127.0.0.1:8080 cloudwatch get-metric-statistics \
  --namespace AWS/EC2 \
  --metric-name CPUUtilization \
  --statistics SampleCount Average Sum Minimum Maximum \
  --period 3600 \
  --start-time 2026-03-11T00:00:00Z \
  --end-time 2026-03-11T12:00:00Z \
  --dimensions Name=InstanceId,Value=<instance-id>
```

## Sources & References

- Request parsing gap: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L64)
- Current `GetMetricStatistics` handler: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L5085)
- Shared aggregation helper: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L1942)
- Current XML response model: [src/handlers/cloudwatch.rs](/Users/murphy/workspace/iacai0/foxtail/src/handlers/cloudwatch.rs#L23)
- Existing XML test coverage gap: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L6420)
- Existing CLI smoke coverage gap: [scripts/verify_cli_interop.sh](/Users/murphy/workspace/iacai0/foxtail/scripts/verify_cli_interop.sh#L282)
- README public contract references: [README.md](/Users/murphy/workspace/iacai0/foxtail/README.md#L79), [README.md](/Users/murphy/workspace/iacai0/foxtail/README.md#L160)
- AWS CLI `get-metric-statistics`: https://docs.aws.amazon.com/cli/latest/reference/cloudwatch/get-metric-statistics.html
- AWS CloudWatch `GetMetricStatistics` API reference: https://docs.aws.amazon.com/AmazonCloudWatch/latest/APIReference/API_GetMetricStatistics.html
- AWS CPUUtilization `Maximum` example: https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/US_SingleMetricPerInstance.html
