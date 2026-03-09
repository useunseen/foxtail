---
status: complete
priority: p2
issue_id: "035"
tags: [rust, performance, cloudwatch]
dependencies: []
---

# 035-pending-p2-optimize-metric-serving-limits.md

## Problem Statement
The mock service currently has hardcoded limits on metric queries and inefficient response building for large datasets.

## Findings
- **Hardcoded Limits**: `serve.rs` contains `LIMIT 100` in metric queries, which will truncate data for large account profiles.
- **Memory Inefficiency**: Metric points are collected into an in-memory `String` or `Value` before being sent, which could cause high memory pressure for 10,000+ points.
- **Missing Indexes**: Time-offset columns are not indexed, making time-range queries slow as the database grows.

## Proposed Solutions

### Option 1: Schema & Query Optimization (Recommended)
- Add composite index on `metrics(resource_id, metric_name, seconds_from_now)`.
- Replace `LIMIT 100` with proper time-range filters from the request body.
- **Effort**: Medium
- **Risk**: Low

### Option 2: Response Streaming
- Implement streaming response for large XML/JSON metric payloads.
- **Effort**: Large
- **Risk**: Medium

## Acceptance Criteria
- [ ] `GetMetricData` returns all requested points within the time range, even if > 100.
- [ ] Database schema includes indexes for metric queries.
- [ ] Query latency remains low for "large" account profiles.

## Work Log
### 2026-02-18 - Findings during code review
- Identified scalability issues in `serve.rs`.
