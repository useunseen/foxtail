---
status: complete
priority: p2
issue_id: "063"
tags: [code-review, cloudwatch, api, quality]
dependencies: []
---

# CloudWatch GetMetricStatistics returns the wrong validation error codes

## Problem Statement

The new `GetMetricStatistics` validation path accepts the right requests and rejects the wrong ones, but it maps every validation failure to `InvalidParameterValueException`. AWS documents different error classes for these cases:

- missing required stats should surface as `MissingParameter`
- supplying both `Statistics` and `ExtendedStatistics` should surface as `InvalidParameterCombination`

Collapsing both into `InvalidParameterValueException` makes the public CloudWatch contract less accurate and can break callers that key behavior off AWS-style error codes.

## Findings

- [`src/serve.rs:5053`](../src/serve.rs#L5053) maps every `MetricStatisticsError::Validation` case to `InvalidParameterValueException`.
- [`src/serve.rs:998`](../src/serve.rs#L998) returns validation errors for the two distinct cases the AWS docs separate: missing `Statistics`/`ExtendedStatistics`, and mixing both fields.
- AWS API docs for `GetMetricStatistics` list `MissingParameter` and `InvalidParameterCombination` as distinct errors: https://docs.aws.amazon.com/AmazonCloudWatch/latest/APIReference/API_GetMetricStatistics.html

## Proposed Solutions

### Option 1: Add error variants for each CloudWatch validation class

Map missing stats to `MissingParameterException` and mixed stat inputs to `InvalidParameterCombination`.

Pros:
- Matches AWS more closely
- Keeps error handling explicit

Cons:
- Requires one more enum and a couple of match arms

Effort:
- Small

Risk:
- Low

### Option 2: Special-case the two known validation paths

Keep the existing enum and branch on the message text or caller context to select the right error code.

Pros:
- Minimal code churn

Cons:
- Fragile and harder to extend
- Couples transport error selection to string messages

Effort:
- Small

Risk:
- Medium

## Recommended Action

Implemented in `src/serve.rs` by splitting `MetricStatisticsError` into distinct validation variants and mapping them to CloudWatch's documented error codes for `GetMetricStatistics`.


## Technical Details

- Affected files: [`src/serve.rs`](../src/serve.rs)
- Related behavior: CloudWatch Query/XML `GetMetricStatistics`
- Database changes: none

## Acceptance Criteria

- [x] Missing `Statistics` and `ExtendedStatistics` returns AWS-style `MissingParameter` rather than `InvalidParameterValueException`
- [x] Supplying both `Statistics` and `ExtendedStatistics` returns `InvalidParameterCombination`
- [x] Unsupported stat names still return a value-error style response
- [x] Route tests cover the distinct error codes

## Work Log

### 2026-03-24 - Review Finding Recorded

**By:** Codex

**Actions:**
- Reviewed commit `9e5f59a` on `main`
- Compared the new `GetMetricStatistics` validation paths against AWS CloudWatch docs
- Identified that all validation failures currently collapse to `InvalidParameterValueException`

**Learnings:**
- The implementation is functionally correct, but the transport-level error mapping is too coarse for AWS parity
- The two documented error classes are easy to preserve with a small enum extension

### 2026-03-24 - Resolution

**By:** Codex

**Actions:**
- Split the CloudWatch metric statistics validation errors into AWS-style `MissingParameter` and `InvalidParameterCombination` branches.
- Kept unsupported statistics on the existing value-error path.
- Added route tests that assert the returned CloudWatch error codes.

**Learnings:**
- The response envelope did not need a structural change; the fix is purely in error classification.

## Resources

- Review target: `9e5f59a`
- AWS CloudWatch `GetMetricStatistics` API reference: https://docs.aws.amazon.com/AmazonCloudWatch/latest/APIReference/API_GetMetricStatistics.html
