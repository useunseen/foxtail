---
status: pending
priority: p1
issue_id: "055"
tags: [code-review, rust, cloudwatch, parity, documentation]
dependencies: []
---

# Restore CloudWatch GetMetricData NextToken parity

## Problem Statement

The branch currently claims CloudWatch `GetMetricData` pagination parity is validated, but the targeted parity suite still fails on truncated responses. That means the branch is not actually ready to claim `NextToken` support for CloudWatch JSON pagination.

## Findings

- The targeted parity run fails in `tests/integration/test_pagination_contracts.py:41` because `GraniteServiceVersion20100801.GetMetricData` returns `200` without `NextToken`.
- The implementation only emits `NextToken` when `page_end < points.len()` in `services/aws-mock-data-service/src/serve.rs:2571`-`services/aws-mock-data-service/src/serve.rs:2605`.
- The failing response body during review was `{\"Messages\": [], \"MetricDataResults\": [{\"Id\": \"m1\", \"StatusCode\": \"Complete\", \"Timestamps\": [], \"Values\": []}]}`, so the pagination path is not producing a truncation token for the contract case.
- The coverage doc still says `GetMetricData` `NextToken` output behavior is validated in parity tests: `docs/testing/aws-mock-api-coverage-status.md:150`-`docs/testing/aws-mock-api-coverage-status.md:158`.
- The roadmap also records pagination parity as validated: `docs/testing/aws-api-priority-roadmap.md:45`-`docs/testing/aws-api-priority-roadmap.md:54`.

## Proposed Solutions

### Option 1: Fix the handler and keep the current contract test authoritative

**Approach:** Update `handle_get_metric_data` so the truncation contract case actually produces paginated data and a `NextToken`, then keep `tests/integration/test_pagination_contracts.py` as the parity gate.

**Pros:**
- Restores the advertised CloudWatch parity behavior.
- Keeps the branch docs aligned with executable tests.
- Smallest change with the highest signal.

**Cons:**
- Requires understanding why the current case returns zero datapoints.
- May expose additional pagination edge cases once fixed.

**Effort:** 1-3 hours

**Risk:** Low

---

### Option 2: Downgrade the docs and scorecard claims until pagination is fully implemented

**Approach:** Treat `NextToken` as not yet shipped, revert the documentation claims, and reclassify this as unfinished parity work.

**Pros:**
- Prevents false confidence immediately.
- Smaller code change if the handler work is non-trivial.

**Cons:**
- Leaves the service short of its current priority target.
- Does not improve actual API behavior.

**Effort:** 30-60 minutes

**Risk:** Medium

## Recommended Action

To be filled during triage.

## Technical Details

**Affected files:**
- `services/aws-mock-data-service/src/serve.rs`
- `tests/integration/test_pagination_contracts.py`
- `docs/testing/aws-mock-api-coverage-status.md`
- `docs/testing/aws-api-priority-roadmap.md`

**Related components:**
- CloudWatch JSON handler
- Parity scorecard / coverage documentation

**Database changes:**
- No

## Resources

- Parity command run during review:
  - `"$PYTHON_BIN" -m pytest tests/integration/test_ce_parity_contract.py tests/integration/test_cw_parity_contract.py tests/integration/test_pagination_contracts.py -q`
- Failing test:
  - `tests/integration/test_pagination_contracts.py::test_cloudwatch_metric_data_emits_nexttoken_when_truncated`

## Acceptance Criteria

- [ ] `test_cloudwatch_metric_data_emits_nexttoken_when_truncated` passes.
- [ ] Truncated `GetMetricData` responses include `NextToken` when more datapoints remain.
- [ ] Coverage docs and roadmap status match the real test state.
- [ ] Targeted parity suite passes without xfail/skip for this behavior.

## Work Log

### 2026-03-09 - Review Discovery

**By:** Codex

**Actions:**
- Ran the targeted CE/CW parity suites and pagination tests.
- Confirmed one failing test in `tests/integration/test_pagination_contracts.py`.
- Traced the token emission path in `services/aws-mock-data-service/src/serve.rs`.
- Compared executable results to current coverage and roadmap docs.

**Learnings:**
- The branch’s CloudWatch pagination claim is ahead of reality.
- This is the only failure in the targeted parity set that was run during review.
