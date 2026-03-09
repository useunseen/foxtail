---
status: complete
priority: p2
issue_id: "042"
tags: [rust, agent-native, observability]
dependencies: []
---

# 042-pending-p2-agent-native-status-and-control

## Problem Statement
The mock service is "controllable" only via CLI start/stop or re-generation, and lacks real-time observability for autonomous agents.

## Findings
- **Lack of Status**: There is no easy way to query "What is the server doing?" or "What data is loaded?" from a script.
- **Scenario Control**: Switching from `Baseline` to `Spike` requires a full re-generation of the database.

## Proposed Solutions
### Option 1: Admin/Status API
- Add a non-AWS endpoint (e.g., `/_mock/status`) that returns a JSON summary of the database (resource counts, scenarios).
- Add a runtime scenario switch endpoint if feasible (e.g., `POST /_mock/scenario`).

## Acceptance Criteria
- [ ] `curl localhost:8080/_mock/status` returns structured JSON.
- [ ] Agent can verify server readiness without AWS SDK overhead.

## Work Log
### 2026-02-19 - Code Review Synthesis
- Identified need for runtime observability and control.
