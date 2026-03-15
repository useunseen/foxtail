---
title: fix: restore AWS CLI network metric queries
type: fix
status: completed
date: 2026-03-16
---

# fix: restore AWS CLI network metric queries

## Overview

Fix the public CloudWatch AWS CLI path so the common "check network in/out" workflow returns seeded EC2 `NetworkIn` and `NetworkOut` datapoints instead of zeroed or incomplete results. The current evidence points to an AWS CLI Query/XML parsing gap rather than missing metric generation.

## Problem Statement / Motivation

The reported user-visible bug is simple: when running the command used to inspect network ingress/egress, all values show as zero.

Local repo evidence suggests the underlying data is already present:

- The generator seeds both `AWS/EC2` `NetworkIn` and `NetworkOut` with non-zero scenario-dependent values: [src/generator.rs](/Users/murphy/workspace/iacai0/foxtail/src/generator.rs#L53), [src/generator.rs](/Users/murphy/workspace/iacai0/foxtail/src/generator.rs#L63), [src/generator.rs](/Users/murphy/workspace/iacai0/foxtail/src/generator.rs#L495)
- Prior scenario verification recorded non-zero EC2 `NetworkIn` values across scenarios: [tasks/todo.md](/Users/murphy/workspace/iacai0/foxtail/tasks/todo.md#L90)
- The README still documents that the AWS CLI Query/XML path for `cloudwatch get-metric-data` only supports one `MetricDataQueries.member.1` query, while the JSON path supports up to 50: [README.md](/Users/murphy/workspace/iacai0/foxtail/README.md#L290)
- The actual `CloudWatchQuery` struct is hard-coded to `MetricDataQueries.member.1.*` fields only: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L66)

That combination strongly suggests the network check command is hitting a partial implementation on the AWS CLI path, especially if it requests both `NetworkIn` and `NetworkOut` in one call.

## Proposed Solution

Extend the CloudWatch Query/XML handling so the AWS CLI path can parse and serve more than one metric-data query, then verify the concrete network workflow end to end.

The intended fix is:

- Replace the fixed `MetricDataQueries.member.1.*` request fields with indexed parsing that can read multiple query members from Query/XML form bodies.
- Reuse the existing aggregation and response-shaping logic already used by the JSON path instead of creating a second metric interpretation path.
- Add a focused regression test and CLI smoke check that explicitly request both `NetworkIn` and `NetworkOut` for the same EC2 instance and assert non-zero datapoints in a seeded time window.
- If verification shows the bug also affects single-metric requests, tighten metric filtering and bucket/stat logic on the XML path so seeded bytes metrics are not lost or normalized incorrectly.

## Research Consolidation

### Internal Repo Findings

- `NetworkIn` and `NetworkOut` are seeded for EC2 as byte-valued metrics with baseline, spike, and idle-heavy shapes: [src/generator.rs](/Users/murphy/workspace/iacai0/foxtail/src/generator.rs#L53), [src/generator.rs](/Users/murphy/workspace/iacai0/foxtail/src/generator.rs#L63)
- EC2 resource regeneration inserts both network metrics alongside CPU and disk series: [src/generator.rs](/Users/murphy/workspace/iacai0/foxtail/src/generator.rs#L492)
- CloudWatch units already map `NetworkIn` and `NetworkOut` to `Bytes`, so the serving layer is aware of these metrics: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L1691)
- The AWS CLI Query/XML contract layer only models one metric-data query member today: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L85)
- The README explicitly documents this limitation, which aligns with the likely failure mode for a dual-series network command: [README.md](/Users/murphy/workspace/iacai0/foxtail/README.md#L356)
- Existing interoperability verification does not explicitly assert `NetworkIn` plus `NetworkOut` in the same `get-metric-data` request: [scripts/verify_cli_interop.sh](/Users/murphy/workspace/iacai0/foxtail/scripts/verify_cli_interop.sh#L315)

### Institutional Learnings

- No relevant `docs/solutions/` entries exist in this checkout.
- Recent project work already established that CloudWatch contract realism matters for public AWS CLI workflows, so this should be fixed on the public surface instead of papered over with helper routes or docs.

### External Research Decision

Proceeding without external research. The likely gap is a repo-local implementation limitation that is already visible in source and README, and the fix should follow the existing local contract patterns.

## SpecFlow Analysis

### User Flow Overview

1. Engineer lists EC2 metrics with `cloudwatch list-metrics`.
2. Engineer selects an instance id and submits a `cloudwatch get-metric-data` request for `NetworkIn` and `NetworkOut`.
3. Query/XML parsing converts both `MetricDataQueries.member.N` entries into internal metric queries.
4. The service resolves the EC2 resource and fetches seeded metric rows for both byte metrics.
5. The aggregation layer buckets results by period and returns aligned timestamps and values for each query id.
6. Engineer sees non-zero ingress and egress series that match the active scenario.

### Missing Elements and Gaps

- **Request parsing gap:** only `member.1` is modeled for AWS CLI Query/XML requests.
- **Coverage gap:** the smoke script does not prove the two-series network workflow.
- **Regression visibility gap:** current tests do not isolate byte-metric behavior on the XML path.
- **Potential filtering gap:** if parsing is fixed and zeros still appear, the metric filter/resource-id extraction path needs a direct byte-metric regression test.

### Critical Questions Requiring Clarification

1. Does the broken command use one `get-metric-data` request with two queries, or two separate requests?
Resolution for planning: assume one multi-query AWS CLI request because that matches the known implementation gap.

2. Should `get-metric-statistics` for `NetworkIn` or `NetworkOut` also be covered?
Resolution for planning: yes, as a secondary regression check, but the main bug target is `get-metric-data`.

## Technical Considerations

- Keep one aggregation implementation. The XML path should feed the same internal query model already used by the JSON path.
- Avoid introducing a one-off “network command” special case. The right fix is general support for multi-query CloudWatch metric requests on the AWS CLI surface.
- Preserve existing behavior for single-query calls and current pagination/error handling.
- Confirm the response keeps per-query ids stable so downstream scripts can map `NetworkIn` and `NetworkOut` correctly.

## System-Wide Impact

- **Interaction graph**: `aws cloudwatch get-metric-data` hits the Query/XML handler in [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs), which parses request fields, builds metric queries, reads seeded SQLite rows, then serializes CloudWatch XML output through the handler structs in [src/handlers/cloudwatch.rs](/Users/murphy/workspace/iacai0/foxtail/src/handlers/cloudwatch.rs).
- **Error propagation**: malformed `MetricDataQueries.member.N` input should continue to return AWS-style validation errors instead of silently defaulting to zero-like output.
- **State lifecycle risks**: none expected; this is a read-path fix over existing seeded data.
- **API surface parity**: JSON `GetMetricData` already supports multi-query requests, so the AWS CLI Query/XML path should reach parity for this core use case.
- **Integration test scenarios**: one request with both `NetworkIn` and `NetworkOut`; one request with only `NetworkIn`; invalid second query member; scenario-changed data still non-zero on the same command.

## Acceptance Criteria

- [x] AWS CLI `cloudwatch get-metric-data` accepts at least two `MetricDataQueries.member.N` entries on the Query/XML path.
- [x] A seeded EC2 instance queried for `NetworkIn` and `NetworkOut` returns two distinct `MetricDataResults` with the caller-provided ids preserved.
- [x] The returned `NetworkIn` and `NetworkOut` series contain non-zero datapoints for a seeded time window in at least the baseline scenario.
- [x] `cloudwatch get-metric-statistics` for `NetworkIn` remains non-zero and correct after the Query/XML changes.
- [x] Existing single-query `get-metric-data` behavior remains intact.
- [x] Route tests cover multi-query Query/XML parsing and byte-metric output shape.
- [x] `scripts/verify_cli_interop.sh` includes an explicit network ingress/egress regression check.

## Success Metrics

- The user’s network check command no longer shows all-zero data for seeded EC2 resources.
- The AWS CLI public surface supports the same practical multi-series workflow already supported on the JSON path.
- Re-running the interoperability smoke suite catches any future regression in network metric output.

## Dependencies & Risks

- **Risk:** the report is caused by a command-specific time range or instance-id mismatch rather than multi-query parsing.
  Mitigation: reproduce the current network check shape exactly in tests and smoke verification before changing behavior.
- **Risk:** Query/XML parsing changes could break existing single-query requests.
  Mitigation: preserve current behavior with regression tests for `member.1` only requests.
- **Risk:** the XML response layer may flatten or mis-order multiple query results after parsing is fixed.
  Mitigation: assert result count, ids, timestamps, and non-zero values in route tests.

## Implementation Suggestions

- Introduce a small helper to collect repeated `MetricDataQueries.member.<n>` form fields into a vectorized internal structure instead of expanding the `CloudWatchQuery` struct with `member.2`, `member.3`, and so on.
- Route both JSON and Query/XML requests through the same metric-data query normalization path where practical.
- Extend the existing CLI smoke script with a concrete request file that includes:
  - `NetworkIn`
  - `NetworkOut`
  - the same `InstanceId`
  - a recent 12-hour or 24-hour seeded range

## Verification Plan

- `cargo fmt`
- `cargo test`
- `cargo clippy --all-targets --all-features`
- `bash scripts/verify_cli_interop.sh`
- Manual AWS CLI check:

```bash
aws --endpoint-url http://127.0.0.1:8080 cloudwatch get-metric-data \
  --metric-data-queries file:///tmp/network_queries.json \
  --start-time 2026-03-15T00:00:00Z \
  --end-time 2026-03-16T00:00:00Z
```

- Verify:
  - two results are returned
  - ids match the request
  - timestamps and values align
  - at least one datapoint in each series is greater than zero

## Sources & References

- Similar implementations: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L4991)
- Current Query/XML limitation: [src/serve.rs](/Users/murphy/workspace/iacai0/foxtail/src/serve.rs#L66)
- Seeded network metrics: [src/generator.rs](/Users/murphy/workspace/iacai0/foxtail/src/generator.rs#L53)
- CLI interoperability smoke suite: [scripts/verify_cli_interop.sh](/Users/murphy/workspace/iacai0/foxtail/scripts/verify_cli_interop.sh#L315)
- Current public contract note: [README.md](/Users/murphy/workspace/iacai0/foxtail/README.md#L290)
