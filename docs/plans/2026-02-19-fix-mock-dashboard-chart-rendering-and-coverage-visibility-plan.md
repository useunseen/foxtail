---
title: fix: Make mock dashboard charts visibly render and expose parity coverage scorecard
type: fix
date: 2026-02-19
---

# fix: Make mock dashboard charts visibly render and expose parity coverage scorecard

## Overview
The `/mock-dashboard` page loads summary cards and supported API rows, but chart panels can appear visually empty even when metric/cost data exists. This plan fixes chart rendering reliability and adds an explicit parity coverage scorecard so users can immediately see implemented, tested, and uncovered API surface.

## Brainstorm Context
No directly relevant brainstorm was found for this chart-visibility bug in the last 14 days. Existing dashboard and parity plans are used as context.

## Problem Statement / Motivation
Current behavior creates a trust gap:
- Dataset counts are non-zero, but charts can look blank.
- The dashboard does not clearly show parity breadth (implemented vs tested vs not covered) in the same view.
- Existing tests validate endpoint shape and component presence, but not real chart visibility constraints.

This slows debugging and parity sign-off.

## Research Consolidation

### Local Repo Findings
- Dashboard chart rendering currently relies on `ResponsiveContainer` with card-bound height in `dashboard-ui/src/components/MockApiDashboard.tsx:1`.
- App route guard needed path normalization for `/mock-dashboard/` variants in `dashboard-ui/src/App.tsx:232`.
- Current component tests mock `recharts`, which hides layout/rendering failures in real browser behavior: `dashboard-ui/src/components/__tests__/MockApiDashboard.test.tsx:5`.
- Dashboard contract tests validate response shape and supported API presence, not graph visibility semantics: `tests/integration/test_mock_dashboard_contract.py:12`.
- Canonical parity inventory and benchmark context already exist in docs: `docs/testing/aws-mock-api-coverage-status.md:1`.

### Institutional Learnings
- Responsive UI regressions must be tested at realistic widths and not only by structural assertions: `docs/solutions/ui-bugs/monitor-right-edge-clipping-assistant-20260215.md`.

### External Research Decision
Local context is sufficient for this fix. No external research is required before planning implementation.

## Proposed Solution

### High-Level Approach
1. Stabilize chart rendering behavior in real browser layout conditions.
2. Add explicit chart diagnostics/fallbacks for zero-size container or invalid series.
3. Add parity scorecard section in dashboard using documented inventory and benchmark outputs.
4. Extend tests to cover chart visibility behavior rather than only card presence.

### Technical Strategy
- Add a chart container guard that verifies measurable width/height before rendering `LineChart`; otherwise show a clear fallback message with dimensions and next actions.
- Normalize series values and reject non-finite points before passing data into charts.
- Add a scorecard panel with:
  - implemented APIs count,
  - implemented+tested count,
  - not-implemented AWS model counts,
  - benchmark snapshot values from `tests/integration/reports/parity_scorecard.json`.
- Keep dashboard read-only and use current `/_mock/dashboard/data` as primary runtime source.

## Technical Considerations
- **Rendering reliability:** `ResponsiveContainer` can silently render nothing when parent dimensions are unresolved at first paint.
- **Data quality:** Chart lines can disappear if values are `NaN`/`Infinity` or all filtered out.
- **Truth source:** Coverage scorecard should remain aligned with `docs/testing/aws-mock-api-coverage-status.md` and parity artifacts, not drift into UI-only state.

## SpecFlow Analysis

### User Flow
1. User opens `http://127.0.0.1:3000/mock-dashboard/`.
2. Page fetches dashboard payload from `/_mock/dashboard/data`.
3. Summary, supported APIs, and chart panels render.
4. If chart container cannot render, user sees explicit fallback and reason.
5. User reads parity scorecard for implemented/tested/not-covered status.

### Edge Cases
- Trailing slash route (`/mock-dashboard/`) and non-trailing slash route (`/mock-dashboard`).
- Non-empty summary counts with empty/invalid chart series.
- Mock API unavailable or delayed.
- Large time windows where chart labels are dense.

## Implementation Phases

### Phase 1: Chart Rendering Hardening
- [x] Add chart container measurement guard in `dashboard-ui/src/components/MockApiDashboard.tsx`.
- [x] Add finite-value filtering and explicit empty/invalid series messaging.
- [x] Improve chart visual defaults (axis domain, line visibility, grid contrast) for dark theme.

### Phase 2: Parity Coverage Scorecard in UI
- [x] Add scorecard data shape in `dashboard-ui/src/lib/api-mock-dashboard.ts`.
- [x] Extend backend dashboard payload or add scorecard endpoint in `services/aws-mock-data-service/src/serve.rs` and handler module.
- [x] Render new scorecard card(s) in `dashboard-ui/src/components/MockApiDashboard.tsx`.

### Phase 3: Testing + Docs Alignment
- [x] Add focused UI tests for route variant and chart fallback scenarios in `dashboard-ui/src/components/__tests__/App.schedule-dialog.test.tsx` and `dashboard-ui/src/components/__tests__/MockApiDashboard.test.tsx`.
- [x] Add integration assertion for scorecard payload fields in `tests/integration/test_mock_dashboard_contract.py`.
- [x] Update dashboard behavior notes in `docs/testing/aws-mock-api-coverage-status.md`.

## Acceptance Criteria
- [x] `/mock-dashboard` and `/mock-dashboard/` both render the dashboard view.
- [x] When series data exists, chart visuals are observable (axes + line) in browser.
- [x] When charts cannot render due layout/data issues, dashboard shows explicit diagnostics instead of blank panels.
- [x] Dashboard displays parity scorecard: implemented, tested, and not-covered counts.
- [x] Frontend tests and integration contract tests pass for new behavior.

## Success Metrics
- Zero “blank chart with non-zero dataset” incidents in local verification runs.
- Test coverage includes at least one positive chart render case and one fallback case.
- Scorecard values match the current documented parity inventory and benchmark artifact.

## Dependencies & Risks
- **Dependencies:** running mock service (`:8080`), dashboard UI (`:3000`), parity report artifacts.
- **Risks:** scorecard drift from docs/artifacts; increased payload complexity.
- **Mitigations:** source scorecard from single generated artifact path and assert shape in integration tests.

## References & Related Work
- Existing dashboard feature plan: `docs/plans/2026-02-19-feat-mock-api-metrics-dashboard-ui-plan.md`
- Coverage source document: `docs/testing/aws-mock-api-coverage-status.md`
- Dashboard contract tests: `tests/integration/test_mock_dashboard_contract.py`
- Dashboard component: `dashboard-ui/src/components/MockApiDashboard.tsx`
- Responsive regression learning: `docs/solutions/ui-bugs/monitor-right-edge-clipping-assistant-20260215.md`
