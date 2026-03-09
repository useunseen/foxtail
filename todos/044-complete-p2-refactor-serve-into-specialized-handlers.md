---
status: complete
priority: p2
issue_id: "044"
tags: [rust, architecture, cleanup]
dependencies: []
---

# 044-pending-p2-refactor-serve-into-specialized-handlers

## Problem Statement
The `src/serve.rs` file is becoming a "mega-module" with mixed routing, dispatching, and service-specific logic. This makes it harder to maintain and extend as support for more AWS services is added.

## Findings
- **Location**: `services/aws-mock-data-service/src/serve.rs` is over 350 lines.
- **Cohesion**: It contains logic for Cost Explorer, CloudWatch (Query & JSON), and Admin endpoints all in one place.
- **Maintenance**: Adding support for a new service like S3 would further bloat this file.

## Proposed Solutions
### Option 1: Module Split (Recommended)
Extract logic into specialized sub-modules:
- `handlers/admin.rs`
- `handlers/cloudwatch.rs`
- `handlers/cost_explorer.rs`

### Option 2: Trait-based Service Dispatch
Define a `MockService` trait that each AWS service implements, handling its own dispatch and response formatting.

## Acceptance Criteria
- [ ] `src/serve.rs` contains only high-level routing and protocol detection.
- [ ] Service-specific logic is moved to dedicated files/modules.

## Work Log
### 2026-02-19 - Session during code review
- Identified the need for better module structure as the service grows.
