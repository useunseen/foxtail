# AWS Mock Data Service

Local Rust service that generates and serves AWS-like Cost Explorer and CloudWatch data from SQLite.

## Command Surface

This repo exposes three different command surfaces:

1. Local developer commands via `make`
2. Binary subcommands via `target/debug/aws-mock-data-service`
3. Public AWS-compatible API calls via `aws --endpoint-url ...`

The `/_mock/*` routes are local helper endpoints. They are not public AWS APIs.

## Quick Start

Build the binary and seed baseline data:

```bash
make setup
```

Run the service:

```bash
make serve
```

The default bind address is `127.0.0.1:8080`.

For AWS CLI calls, set dummy local credentials once:

```bash
export AWS_ACCESS_KEY_ID=test
export AWS_SECRET_ACCESS_KEY=test
export AWS_DEFAULT_REGION=us-east-1
export AWS_PAGER=""
```

## Local Developer Commands

### Make Targets

| Command | Purpose |
| --- | --- |
| `make help` | Print available local commands |
| `make build` | Build the debug binary |
| `make build-release` | Build the release binary |
| `make gen` | Discover resources and regenerate `mock_data.db` |
| `make gen-baseline` | Regenerate data with the baseline scenario |
| `make gen-spike` | Regenerate data with the spike scenario |
| `make gen-idle-heavy` | Regenerate data with the idle-heavy scenario |
| `make serve` | Start the API server on `127.0.0.1:8080` |
| `make setup` | Build and seed baseline data |
| `make setup-mock` | Compatibility alias for `make setup` |
| `make verify-cli-interoperability` | Run the AWS CLI smoke suite against a temporary local server |

### Binary Commands

The binary has one global option:

```bash
target/debug/aws-mock-data-service --database-url sqlite:mock_data.db ...
```

Supported subcommands:

#### `gen`

Generate or refresh seeded data.

```bash
target/debug/aws-mock-data-service gen \
  --endpoint-url http://localhost:4566 \
  --region us-east-1 \
  --scenario baseline \
  --prune \
  --json
```

Supported flags:

- `--endpoint-url <url>`: source endpoint for discovery, default `http://localhost:4566`
- `--region <region>`: AWS region, default `us-east-1`
- `--scenario <baseline|spike|idle-heavy>`: traffic/cost scenario, default `baseline`
- `--prune`: remove discovered resources that no longer exist
- `--json`: print a JSON summary of discovered resources

#### `serve`

Start the local API server.

```bash
target/debug/aws-mock-data-service serve --address 127.0.0.1 --port 8080
```

Supported flags:

- `--address <ip-or-host>`: bind address, default `127.0.0.1`
- `--port <port>`: bind port, default `8080`

## Public AWS-Compatible Commands

All public AWS-compatible calls are served from `POST /`.

Use the local endpoint like this:

```bash
aws --endpoint-url http://127.0.0.1:8080 ...
```

### Cost Explorer

The service accepts the Cost Explorer operations below. For compatibility with different clients, both `AWSCostExplorer.*` and `AWSInsightsIndexService.*` targets are supported internally.

| AWS CLI command | Purpose | Notes |
| --- | --- | --- |
| `ce get-cost-and-usage` | Cost totals and grouped cost breakdowns | Supports `--group-by` for seeded dimensions such as `SERVICE` |
| `ce get-cost-forecast` | Forecasted spend over a time period | Requires `--granularity` |
| `ce get-dimension-values` | Discover valid dimension values | Useful for `SERVICE`, `RESOURCE_ID`, `REGION` |
| `ce get-reservation-coverage` | Mock RI coverage view | Seeded synthetic output |
| `ce get-reservation-utilization` | Mock RI utilization view | Seeded synthetic output |
| `ce get-savings-plans-coverage` | Mock Savings Plans coverage view | Seeded synthetic output |
| `ce get-savings-plans-utilization` | Mock Savings Plans utilization view | Seeded synthetic output |
| `ce get-rightsizing-recommendation` | Mock rightsizing recommendation output | Seeded synthetic output |
| `ce get-anomalies` | Mock anomaly detection output | Seeded synthetic output |
| `ce get-anomaly-monitors` | Mock anomaly monitor list | Seeded synthetic output |
| `ce get-anomaly-subscriptions` | Mock anomaly subscription list | Seeded synthetic output |

Example:

```bash
aws --endpoint-url http://127.0.0.1:8080 ce get-cost-and-usage \
  --time-period Start=2026-03-01,End=2026-03-11 \
  --granularity DAILY \
  --metrics UnblendedCost \
  --group-by Type=DIMENSION,Key=SERVICE
```

### CloudWatch

CloudWatch support is split by protocol:

- `aws cloudwatch ...` uses the Query/XML path
- direct `x-amz-target: GraniteServiceVersion20100801.GetMetricData` uses the JSON path

| AWS CLI command | Purpose | Protocol | Notes |
| --- | --- | --- | --- |
| `cloudwatch list-metrics` | Discover available metric definitions | Query/XML | Use this first when the scenario changes |
| `cloudwatch get-metric-statistics` | Return a single aggregated time series | Query/XML | Best simple path for one resource/metric pair |
| `cloudwatch get-metric-data` | Return one or more aggregated time series | Query/XML or JSON | AWS CLI path currently supports one `MetricDataQueries.member.1` query; JSON path supports up to 50 queries |

#### `cloudwatch list-metrics`

Use this to discover which metrics exist for the current seeded scenario. It does not return datapoints.

```bash
aws --endpoint-url http://127.0.0.1:8080 cloudwatch list-metrics \
  --namespace AWS/EC2 \
  --metric-name CPUUtilization
```

#### `cloudwatch get-metric-statistics`

Use this for one metric and one resource when you want a straightforward aggregated time series.

```bash
aws --endpoint-url http://127.0.0.1:8080 cloudwatch get-metric-statistics \
  --namespace AWS/EC2 \
  --metric-name CPUUtilization \
  --statistics Average \
  --period 3600 \
  --start-time 2026-03-11T00:00:00Z \
  --end-time 2026-03-11T12:00:00Z \
  --dimensions Name=InstanceId,Value=i-20652c71bedc57ced
```

#### `cloudwatch get-metric-data`

Use this when you want one or more query-defined metric series in one response. The CLI query JSON is a request payload, not a cached metrics dump.

Example request file:

```json
[
  {
    "Id": "cpu",
    "MetricStat": {
      "Metric": {
        "Namespace": "AWS/EC2",
        "MetricName": "CPUUtilization",
        "Dimensions": [
          {
            "Name": "InstanceId",
            "Value": "i-20652c71bedc57ced"
          }
        ]
      },
      "Period": 3600,
      "Stat": "Average"
    }
  }
]
```

Example AWS CLI call:

```bash
aws --endpoint-url http://127.0.0.1:8080 cloudwatch get-metric-data \
  --metric-data-queries file:///tmp/metric_queries.json \
  --start-time 2026-03-11T00:00:00Z \
  --end-time 2026-03-11T12:00:00Z
```

Current `get-metric-data` behavior:

- Preserves the caller query id in the response
- Buckets timestamps cleanly by `MetricStat.Period`
- Supports `Average`, `Sum`, `Minimum`, and `Maximum`
- Keeps timestamp and value arrays aligned
- Paginates deterministically
- Supports up to 50 queries on the JSON target path
- Supports one query on the current AWS CLI Query/XML path

## Local Helper Endpoints

These are local helper routes for debugging, dashboards, and scenario control. They are not part of the public AWS-compatible surface.

### `GET /_mock/status`

Returns service health and seed counts.

Example:

```bash
curl http://127.0.0.1:8080/_mock/status
```

### `POST /_mock/scenario`

Applies a scenario mutation to the current dataset.

Request body:

```json
{
  "scenario": "Baseline",
  "resource_id": "i-20652c71bedc57ced"
}
```

Fields:

- `scenario`: one of `Baseline`, `Spike`, `IdleHeavy`
- `resource_id`: optional, scope the scenario change to one resource

If `AWS_MOCK_ADMIN_TOKEN` is set in the environment, callers must also send `x-mock-admin-token`.

### Dashboard Routes

These routes share the same optional query parameters:

- `scope=aggregate|service|resource`
- `resource_type=<type>`
- `resource_id=<id>`
- `namespace=<metric-namespace>`
- `metric_name=<metric-name>`
- `top_n=<count>`
- `window_hours=<hours>`

Supported dashboard routes:

| Method and path | Purpose |
| --- | --- |
| `GET /_mock/dashboard/data` | Full dashboard payload, including supported API metadata |
| `GET /_mock/dashboard/resources` | Resource catalog, top-cost resources, and low-utilization candidates |
| `GET /_mock/dashboard/trends/cloudwatch` | Aggregated CloudWatch trend series |
| `GET /_mock/dashboard/trends/cost` | Aggregated cost trend series |

Example:

```bash
curl "http://127.0.0.1:8080/_mock/dashboard/resources?scope=resource&top_n=5"
```

## Discovery Workflow

If the scenario changes, do not rely on remembered resource ids.

Use this flow:

1. Seed or switch a scenario.
2. Discover available services or resources with `ce get-dimension-values`.
3. Discover available metrics with `cloudwatch list-metrics`.
4. Use the returned ids and metric definitions in `get-metric-statistics` or `get-metric-data`.

Example:

```bash
make gen-spike
make serve

aws --endpoint-url http://127.0.0.1:8080 ce get-dimension-values \
  --time-period Start=2026-03-01,End=2026-03-11 \
  --dimension RESOURCE_ID

aws --endpoint-url http://127.0.0.1:8080 cloudwatch list-metrics \
  --namespace AWS/EC2 \
  --metric-name CPUUtilization
```

## Verification

Core local checks:

```bash
cargo fmt
cargo test
cargo clippy --all-targets --all-features
```

Full CLI smoke verification:

```bash
make verify-cli-interoperability
```
