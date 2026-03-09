---
title: "test: Verify AWS Mock Service API Parity and Data Integrity"
type: test
date: 2026-02-18
---

# test: Verify AWS Mock Service API Parity and Data Integrity

## Enhancement Summary
**Deepened on:** 2026-02-18
**Sections enhanced:** 6
**Research agents used:** explore, best-practices-researcher, architecture-strategist, security-sentinel, performance-oracle, agent-native-reviewer

### Key Improvements
1. **Protocol Depth Expansion**: Added coverage for `ListMetrics`, `PutMetricData`, and complex filtering/grouping in `GetCostAndUsage` and `GetMetricData`.
2. **Concurrency & Performance Hammer**: Introduced load testing and mixed workload verification (Read-while-Write) to ensure SQLite stability under WAL mode.
3. **Agent-Native Lifecycle Control**: Added verification for a new "Control Plane" API and standardized AWS error envelopes to ensure autonomous agent robustness.
4. **Security & Validation Gates**: Included fuzzing for manual URL/XML parsers and verification of SigV4 bypass logic.

## Overview
This plan implements a comprehensive verification suite for the `aws-mock-data-service`. It ensures that the Rust-based mock provides perfect wire-compatibility with official AWS SDKs (`boto3`) and the AWS CLI, specifically for CloudWatch metrics and Cost Explorer data.

## Problem Statement / Motivation
A mock service is only useful if it is **transparent**. Current implementation lacks deep filtering, standardized errors, and high-concurrency verification. If the agent receives a generic "Not Found" string or unfiltered metric data, it will fail to optimize correctly. We need automated proof that the service behaves like the real AWS API in all edge cases.

## Proposed Solution
Create a high-fidelity integration test suite in `tests/integration/test_aws_mock_service.py` using `boto3` and custom HTTP clients to validate wire-parity, performance, and agent-specific control flows.

## Technical Considerations
- **Protocol Toggling**: Must verify that the service handles `application/x-www-form-urlencoded` (Query API) and `application/x-amz-json-1.1` (JSON API) correctly.
- **Time Sensitivity**: Tests must verify the **Dynamic Offset Logic** (e.g., querying the same data 5 minutes apart returns shifted absolute timestamps).
- **Header Fidelity**: Verification of `x-amzn-RequestId` and `Date` headers required for SDK compatibility.

### Research Insights (Performance & Architecture)
**Best Practices:**
- Use **WAL (Write-Ahead Logging)** mode for concurrent reader/writer support in SQLite.
- Implement **AWS JSON 1.0/1.1** action dispatching via `X-Amz-Target` headers.
- Return **Strings** for cost amounts to preserve precision.

**Performance Considerations:**
- **Fat XML Payload**: Request 5,000+ datapoints to verify string allocation overhead and serialization latency.
- **Memory Pressure**: Ensure JSON serialization doesn't triple-buffer data in memory (target < 3x raw data size).
- **Concurrency**: Use `wrk` or `hey` to send 100+ concurrent requests while running `gen` to check for locking issues.

## Acceptance Criteria

### Functional Requirements (API Parity)
- [ ] **CloudWatch XML (Query)**: `GetMetricStatistics` and `ListMetrics` return valid XML schemas parsed by `boto3`.
- [ ] **CloudWatch JSON (1.1)**: `GetMetricData` correctly filters by `MetricDataQueries` ID and doesn't return leaked data.
- [ ] **Cost Explorer JSON (1.1)**: `GetCostAndUsage` correctly handles `GroupBy` (Service/Tag), `Filter`, and `Granularity`.
- [ ] **Standardized Errors**: Invalid actions return AWS-compliant error envelopes (Code/Message), not raw text.

### Non-Functional Requirements
- [ ] **Sliding Window Integrity**: Data remains "current" (not stale) across server restarts and time passages.
- [ ] **Discovery Manifest**: The `gen --json` output matches the resource counts returned by `boto3` queries against the mock.
- [ ] **Concurrency**: Zero "Database is locked" errors during simultaneous `gen` and `serve` operations.
- [ ] **Latency**: P99 response time < 100ms for queries up to 1000 datapoints.

## Implementation Phases

### Phase 1: Parity & Schema (20+ Test Cases)
- [x] Implement `tests/integration/test_cw_parity.py` covering:
    - Namespace/MetricName filtering
    - Dimension matching (and empty dimension handling)
    - Protocol dispatch (Query vs JSON triggers)
- [x] Implement `tests/integration/test_ce_parity.py` covering:
    - GroupBy Dimension (SERVICE, REGION)
    - GroupBy Tag (Project, Environment)
    - TimePeriod boundaries (Relative offset validation)

### Phase 2: Security & Robustness
- [x] Implement `tests/integration/test_robustness_concurrency.py` covering:
    - SigV4 Bypass: Verify that malformed/expired signatures succeed (local mode).
    - Input Fuzzing: Send special characters in MetricName and Dimension Values.
    - Concurrency Hammer: 100 concurrent threads querying the same resource ID.

### Phase 3: Performance & Concurrency
- [ ] **Hammer Test**: 100 concurrent threads querying the same resource ID.
- [ ] **Read-while-Write**: Run `make gen` in a loop while querying metrics.
- [ ] **Scale Test**: Query 14 days of 1-minute data (~20,000 points) and measure RSS memory.

### Phase 4: Agent-Native Control
- [ ] **Control Plane API**: Verify `POST /_mock/scenario` correctly updates the active data profile.
- [ ] **Status Command**: Verify `aws-mock status` returns accurate resource/metric counts.

## MVP Implementation Example

### tests/integration/test_aws_mock_service.py
```python
import boto3
import pytest
from datetime import datetime, timedelta

MOCK_ENDPOINT = "http://localhost:8080"

@pytest.fixture
def cw_client():
    return boto3.client("cloudwatch", endpoint_url=MOCK_ENDPOINT, region_name="us-east-1")

def test_cw_sliding_window_integrity(cw_client):
    """Verify that data shifts absolute time but keeps relative offset."""
    q = {"Namespace": "AWS/EC2", "MetricName": "CPUUtilization", "Period": 3600, "Statistics": ["Average"]}
    
    # First query
    res1 = cw_client.get_metric_statistics(**q, StartTime=datetime.utcnow()-timedelta(h=1), EndTime=datetime.utcnow())
    t1 = res1['Datapoints'][0]['Timestamp']
    
    # Second query after 10s wait (simulated or real)
    import time; time.sleep(1) 
    res2 = cw_client.get_metric_statistics(**q, StartTime=datetime.utcnow()-timedelta(h=1), EndTime=datetime.utcnow())
    t2 = res2['Datapoints'][0]['Timestamp']
    
    assert t1 != t2 # Time has moved
    assert abs((t2 - t1).total_seconds()) >= 1

def test_ce_grouping_by_service(ce_client):
    """Verify that GroupBy SERVICE returns structured groups, not just Total."""
    res = ce_client.get_cost_and_usage(
        TimePeriod={'Start': '2026-02-01', 'End': '2026-02-18'},
        Granularity='DAILY',
        Metrics=['UnblendedCost'],
        GroupBy=[{'Type': 'DIMENSION', 'Key': 'SERVICE'}]
    )
    assert len(res['ResultsByTime'][0]['Groups']) > 0
    assert 'Keys' in res['ResultsByTime'][0]['Groups'][0]
    assert res['ResultsByTime'][0]['Groups'][0]['Keys'][0] == 'ec2' # Or exact AWS name
```

## References & Research
- Learning: `docs/solutions/integration-issues/high-fidelity-aws-mocking-rust-service.md`
- AWS API Specs: [CloudWatch Reference](https://docs.aws.amazon.com/AmazonCloudWatch/latest/APIReference/Welcome.html), [CostExplorer Reference](https://docs.aws.amazon.com/aws-cost-management/latest/APIReference/Welcome.html)
- Concurrency Patterns: [SQLite WAL Documentation](https://www.sqlite.org/wal.html)
