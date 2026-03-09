---
status: pending
priority: p1
issue_id: "056"
tags: [code-review, backend, finops, cost-explorer, cloudwatch, cur, integration]
dependencies: []
---

# Wire Cost Explorer, CUR, and CloudWatch evidence into runtime decisioning

## Problem Statement

The branch adds substantial mock-service parity for Cost Explorer and some CloudWatch operations, but the main runtime still consumes only a narrow subset of that data. `CUR` is still roadmap-only, and the backend evidence layer does not yet use the newly added CE APIs for decisioning. As a result, the repo does not yet support CE/CUR/CloudWatch service APIs end-to-end beyond the existing cost-total calibration and EC2 CPU lookup paths.

## Findings

- Cost enrichment in `graph_builder.py` still calls only `ce get-cost-and-usage` for a total-cost calibration flow: `graph_builder.py:1015`-`graph_builder.py:1143`.
- The real AWS Cost Explorer path is still explicitly unimplemented: `graph_builder.py:1096`-`graph_builder.py:1097`.
- Metric enrichment still uses only `cloudwatch get-metric-statistics` for EC2 CPU collection: `graph_builder.py:1145`-`graph_builder.py:1332`.
- The roadmap explicitly says backend evidence ingestion for new APIs is still `planned`: `docs/testing/aws-api-priority-roadmap.md:82`-`docs/testing/aws-api-priority-roadmap.md:99`.
- The decision map classifies the advanced billing analytics path (`cur`, `bcm-data-exports`, `athena`, `glue`) as `planned`, not implemented: `docs/testing/aws-finops-devops-api-decision-map.md:159`-`docs/testing/aws-finops-devops-api-decision-map.md:173`.
- The mock coverage status document also frames the remaining work as deeper behavioral coverage plus additional API breadth, especially `ListMetrics` and decision-level usage tests: `docs/testing/aws-mock-api-coverage-status.md:124`-`docs/testing/aws-mock-api-coverage-status.md:179`.

## Proposed Solutions

### Option 1: Phase the runtime integration by decision value

**Approach:** First wire the already-implemented CE APIs (`GetDimensionValues`, commitment coverage/utilization, rightsizing, anomalies) into typed evidence/provenance models in `graph_builder.py` and planning tools. Keep `CUR` as a separate Phase 4 analytics path with an explicit capability flag.

**Pros:**
- Aligns with the existing roadmap and decision map.
- Delivers usable decision evidence before the larger CUR/Athena scope.
- Keeps blast radius manageable.

**Cons:**
- Still leaves the analytics path incomplete after the first phase.
- Requires model and orchestration changes, not only mock-service work.

**Effort:** 2-5 days

**Risk:** Medium

---

### Option 2: Build the CUR analytics path first and make CE/CW a fallback

**Approach:** Implement the CUR/Data Exports/Athena path as the primary billing analytics source, then use CE/CW only as compatibility or fallback evidence.

**Pros:**
- Targets the highest-fidelity cost attribution path.
- Avoids over-investing in CE-only semantics if CUR is the intended end state.

**Cons:**
- Much larger scope.
- Requires more capability modeling and test infrastructure.
- Slower path to shipping usable end-to-end improvements.

**Effort:** 1-2 weeks

**Risk:** High

## Recommended Action

To be filled during triage.

## Technical Details

**Affected files:**
- `graph_builder.py`
- `models.py` or a new decision-evidence model module
- `orchestrator_tools_analysis.py`
- `orchestrator_tools_plan.py`
- `docs/testing/aws-api-priority-roadmap.md`
- `docs/testing/aws-finops-devops-api-decision-map.md`

**Related components:**
- Rust `aws-mock-data-service`
- Runtime enrichment / graph evidence layer
- Planner outputs and evidence provenance

**Database changes:**
- Likely none for phase-one integration
- Possible new evidence metadata structures depending on implementation

## Resources

- Current runtime integration plan:
  - `docs/plans/2026-02-19-feat-integrate-aws-mock-metrics-runtime-plan.md`
- Decision roadmap:
  - `docs/testing/aws-api-priority-roadmap.md`
- Decision map:
  - `docs/testing/aws-finops-devops-api-decision-map.md`

## Acceptance Criteria

- [ ] Backend runtime uses more than `GetCostAndUsage` and `GetMetricStatistics` for evidence where those APIs are already implemented in the mock service.
- [ ] New evidence paths are exposed with provenance/freshness metadata, not silent heuristics.
- [ ] `CUR`/Athena support is either implemented behind a bounded interface or explicitly deferred behind a capability flag with documented fallback behavior.
- [ ] Integration tests cover decision-level use of the new CE/CW evidence and the chosen CUR fallback strategy.

## Work Log

### 2026-03-09 - Review Discovery

**By:** Codex

**Actions:**
- Reviewed the mock-service parity additions against the runtime evidence layer.
- Confirmed that `graph_builder.py` still consumes only the legacy CE total-cost path and CloudWatch statistics path.
- Verified that CUR remains documented as planned rather than implemented.
- Cross-checked the roadmap and decision map for remaining scope.

**Learnings:**
- The branch has strong mock-service parity progress, but end-to-end CE/CUR/CloudWatch support is still only partial.
- The missing work is primarily runtime evidence integration plus the entire CUR analytics path.
