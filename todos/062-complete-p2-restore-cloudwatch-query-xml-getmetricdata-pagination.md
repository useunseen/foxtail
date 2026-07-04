---
status: complete
priority: p2
issue_id: "062"
tags: ["code-review", "cloudwatch", "parity", "quality"]
dependencies: []
---

# Restore CloudWatch Query/XML GetMetricData pagination

## Problem Statement

The PR expands AWS CLI Query/XML `GetMetricData` to support multiple `MetricDataQueries.member.N` entries, but the XML path still does not honor pagination inputs or emit pagination outputs. This leaves the public AWS CLI contract inconsistent with the JSON handler and with the README, which now states that `get-metric-data` paginates deterministically.

This matters because AWS CLI callers can request large time windows and expect `NextToken`-driven traversal. On the Query/XML path, they currently receive the full response in one shot with no continuation token, even though the JSON path already implements paging semantics.

## Findings

- [`src/serve.rs:4965`](../src/serve.rs#L4965) builds the XML `GetMetricData` response directly from `series_list` and never reads `query.next_token` or any page-size/max-datapoints input.
- [`src/serve.rs:5136`](../src/serve.rs#L5136) shows the JSON handler already implementing `MaxDatapoints`, `NextToken`, and deterministic slicing logic, so the parity gap is inside the XML path rather than the shared aggregation layer.
- [`src/handlers/cloudwatch.rs:69`](../src/handlers/cloudwatch.rs#L69) defines `GetMetricDataResult` without any `NextToken` field, so the XML serializer cannot emit continuation tokens even if the handler wanted to.
- [`README.md:356`](../README.md#L356) now claims `get-metric-data` “Paginates deterministically” and supports 50 queries on both paths, which overstates current AWS CLI Query/XML behavior.
- The new test coverage validates multi-query XML parsing and non-zero network datapoints, but there is no XML pagination regression test to catch this contract gap.

## Proposed Solutions

### Option 1: Bring XML path to feature parity with JSON pagination

**Approach:** Add `MaxDatapoints` and `NextToken` parsing to the Query/XML path, reuse the same slice logic as the JSON handler, and extend the XML response structs to serialize `NextToken`.

**Pros:**
- Fixes the actual contract gap instead of documenting around it
- Keeps JSON and Query/XML behavior aligned
- Supports real AWS CLI paginated workflows

**Cons:**
- Requires light refactoring of the XML response model
- Needs additional route tests and smoke checks

**Effort:** 2-4 hours

**Risk:** Medium

---

### Option 2: Extract common pagination shaping for both handlers

**Approach:** Move result slicing and token generation into a shared helper used by both JSON and Query/XML handlers.

**Pros:**
- Reduces future parity drift
- Simplifies testing of token semantics
- Makes the behavior easier to reason about

**Cons:**
- Slightly larger refactor than the immediate bug fix
- Needs careful adaptation for JSON and XML response shapes

**Effort:** 4-6 hours

**Risk:** Medium

---

### Option 3: Narrow the docs until parity is implemented

**Approach:** Keep the code as-is for now but explicitly document that deterministic pagination currently applies only to the JSON target path.

**Pros:**
- Fastest mitigation
- Prevents users from trusting an unsupported path

**Cons:**
- Leaves AWS CLI parity incomplete
- Does not help existing paginated CLI workflows

**Effort:** 15-30 minutes

**Risk:** Low

## Recommended Action

**To be filled during triage.** Preferred direction is Option 1, or Option 2 if further CloudWatch parity work is planned soon.

## Technical Details

**Affected files:**
- [`src/serve.rs`](../src/serve.rs) - XML `GetMetricData` handler lacks token/page handling
- [`src/handlers/cloudwatch.rs`](../src/handlers/cloudwatch.rs) - XML response model cannot serialize `NextToken`
- [`README.md`](../README.md) - docs currently overclaim pagination parity

**Related components:**
- JSON `GetMetricData` path in [`src/serve.rs`](../src/serve.rs)
- CLI smoke verification in [`scripts/verify_cli_interop.sh`](../scripts/verify_cli_interop.sh)

**Database changes:**
- Migration needed? No
- New columns/tables? None

## Resources

- **PR:** https://github.com/iacai0/foxtail/pull/1
- **Plan:** [docs/plans/2026-03-16-fix-aws-cli-network-metric-queries-plan.md](../docs/plans/2026-03-16-fix-aws-cli-network-metric-queries-plan.md)
- **Reference implementation:** JSON `GetMetricData` pagination in [src/serve.rs](../src/serve.rs#L5136)

## Acceptance Criteria

- [ ] Query/XML `GetMetricData` accepts and validates pagination input (`NextToken`, and any supported page size/max datapoints field)
- [ ] Query/XML `GetMetricData` emits `NextToken` when results are truncated
- [ ] XML and JSON handlers use equivalent slice/token semantics for the same seeded series
- [ ] Route coverage includes at least one paginated Query/XML `GetMetricData` case
- [ ] README accurately describes pagination support after the change

## Work Log

### 2026-03-16 - Initial Review Finding

**By:** Codex

**Actions:**
- Reviewed PR #1 diff against `main`
- Traced the new Query/XML multi-query parser and XML response builder
- Compared XML behavior with the JSON pagination implementation
- Identified that the XML response model has no `NextToken` field and the XML handler does not read pagination inputs

**Learnings:**
- The PR correctly fixes multi-query XML parsing for `NetworkIn` and `NetworkOut`
- The remaining gap is parity of response shaping and continuation semantics, not metric aggregation itself

### 2026-03-16 - Fix Implemented

**By:** Codex

**Actions:**
- Added `MaxDatapoints` parsing to the Query/XML request model
- Added `NextToken` serialization to the XML `GetMetricData` response model
- Extracted shared metric-data pagination logic so JSON and Query/XML handlers now slice series with the same semantics
- Added a Query/XML regression test covering paginated `GetMetricData`
- Re-ran formatting, full tests, clippy, and the CLI interoperability smoke suite

**Learnings:**
- The missing parity issue was isolated to handler-level pagination and XML response serialization
- The existing shared aggregation model was already sufficient once both protocols reused the same pagination step

## Notes

- This is a parity/reliability issue, not a data corruption or security issue.
