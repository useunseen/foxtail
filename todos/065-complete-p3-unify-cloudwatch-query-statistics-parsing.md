---
status: complete
priority: p3
issue_id: "065"
tags: [code-review, cloudwatch, architecture, performance, quality]
dependencies: []
---

# Unify CloudWatch Query/XML GetMetricStatistics request parsing

## Problem Statement

`GetMetricStatistics` now decodes the same Query/XML body twice: first into `CloudWatchQuery` for action routing and core fields, then again into raw key/value pairs for `Statistics.member.N` and `ExtendedStatistics.member.N`. That split parsing works today, but it makes the transport boundary harder to reason about and increases the chance of drift between routing, validation, and handler-specific request semantics.

This is not a merge blocker, but it is a maintenance and hot-path overhead issue on a public CloudWatch endpoint.

## Findings

- [`src/serve.rs:64`](../src/serve.rs#L64) defines `CloudWatchQuery`, which is still the first parse target for all Query/XML CloudWatch requests.
- [`src/serve.rs:964`](../src/serve.rs#L964) reparses the raw body specifically to recover `Statistics.member.N` and `ExtendedStatistics.member.N`.
- [`src/serve.rs:5263`](../src/serve.rs#L5263) then consumes a second request model, `GetMetricStatisticsRequest`, assembled from the reparse.
- The performance review also flagged the double parse as unnecessary work on a frequently exercised API path.

## Proposed Solutions

### Option 1: Introduce one typed `GetMetricStatistics` Query/XML parser

**Approach:** Build one parser for the endpoint that handles both base fields and repeated stats, then pass that typed request through the handler chain.

**Pros:**
- Removes duplicate parsing work
- Reduces drift between validation and handler inputs
- Makes the transport boundary cleaner

**Cons:**
- Slightly more refactor than the current targeted fix

**Effort:** Small

**Risk:** Low

---

### Option 2: Extend `CloudWatchQuery` to carry statistics members

**Approach:** Add statistic fields or collections to the existing route-level struct and stop reparsing the body.

**Pros:**
- Keeps the current route shell intact
- Less disruptive than a full parser replacement

**Cons:**
- Makes `CloudWatchQuery` more ad hoc and endpoint-specific
- Harder to model repeated members cleanly

**Effort:** Small

**Risk:** Medium

## Recommended Action

Implemented on 2026-03-24 with a one-pass CloudWatch Query/XML parser that now populates the `GetMetricStatistics` request model directly.


## Technical Details

- Affected files:
  - [`src/serve.rs`](../src/serve.rs)
- Related components:
  - CloudWatch Query/XML router
  - `GetMetricStatistics` validation and request shaping
- Database changes: none

## Acceptance Criteria

- [x] The Query/XML `GetMetricStatistics` path parses the request body only once
- [x] Statistics and core request fields come from one authoritative request model
- [x] Existing route tests for missing, mixed, and unsupported statistic inputs continue to pass
- [x] The refactor does not change the public API contract

## Work Log

### 2026-03-24 - Review Finding Recorded

**By:** Codex

**Actions:**
- Reviewed commit `9e5f59a` on `main`
- Compared the route-level `CloudWatchQuery` parse with the handler-specific `GetMetricStatistics` parse
- Identified duplicate request decoding and a split transport model

**Learnings:**
- The implementation is correct today, but the doubled parsing is an avoidable source of drift
- This is a good candidate for cleanup once correctness issues are handled

### 2026-03-24 - Fix Implemented

**By:** Codex

**Actions:**
- Replaced the second `GetMetricStatistics` body parse with a single `CloudWatchQuery` decode that also carries statistics members
- Kept request validation behavior unchanged by building the internal metric-statistics request from that parsed model
- Added a parser regression test to prove the unified transport model captures standard and extended statistics members
- Ran `cargo fmt`, `cargo test`, and `cargo clippy --all-targets --all-features`

**Learnings:**
- The cleanup was safe once the route-level query model owned the statistics members directly
- No additional correctness fix was needed to complete this todo

## Resources

- Review target: `9e5f59a`
