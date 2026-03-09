---
title: feat: Integrate aws-mock-data-service directly into agent runtime enrichment
type: feat
date: 2026-02-19
status: superseded-in-progress
---

# feat: Integrate aws-mock-data-service directly into agent runtime enrichment

## Overview
Remove the intermediate `aws-cost-data-service` path and integrate `aws-mock-data-service` directly in runtime enrichment.

Primary outcome:
- `graph_builder.py` pulls CloudWatch and Cost Explorer data from `aws-mock-data-service` (AWS-compatible APIs on `AWS_MOCK_ENDPOINT`) instead of local file fixtures and LocalStack-only heuristics.

## Why This Changed
The previous version of this plan depended on `services/aws-cost-data-service`, which has now been removed from the repository.

## Scope
1. Keep `aws-mock-data-service` as the single mock data authority.
2. Wire runtime metrics/cost calls directly in `graph_builder.py`.
3. Retain local fallback behavior only when `AWS_MOCK_ENDPOINT` is not configured.

## Implementation Checklist
- [x] Remove `services/aws-cost-data-service` from repo.
- [x] Add direct mock-endpoint-aware command path in `graph_builder.py`.
- [x] Use mock endpoint for CloudWatch metric retrieval when configured.
- [x] Use mock endpoint Cost Explorer response to calibrate runtime cost enrichment when configured.
- [x] Keep existing fallback behavior for non-mock or endpoint-unavailable flows.
- [x] Update root README with `AWS_MOCK_ENDPOINT` runtime configuration.
- [x] Run targeted validations for graph enrichment behavior.

## Acceptance Criteria
- [ ] With `USE_LOCALSTACK=true` and `AWS_MOCK_ENDPOINT=http://127.0.0.1:8080`, runtime enrichment uses the mock service APIs.
- [ ] With `AWS_MOCK_ENDPOINT` unset, existing fallback behavior still works.
- [ ] No references to removed `aws-cost-data-service` remain in active implementation docs.

## Notes
Historical plans may still mention the removed service; treat those as archival context only.
