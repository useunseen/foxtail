# Foxtail

Foxtail is a local Rust service that serves AWS-like FinOps data from SQLite.

It is meant for local development, demos, testing, and agent workflows where you want AWS CLI-compatible responses without calling real AWS. It seeds synthetic cost, usage, inventory, pricing, and CloudWatch metric data, then serves that data on `http://127.0.0.1:8080`.

## What It Provides

Foxtail exposes a public AWS-compatible surface for these service areas:

- `ce` for Cost Explorer-style cost, usage, forecast, anomaly, savings, reservation, and rightsizing flows
- `cloudwatch` for metric discovery and metric queries
- `resourcegroupstaggingapi` for tagged resource inventory and tag discovery
- `pricing` for a small mock price catalog
- `compute-optimizer` for synthetic EC2 and EBS recommendations
- `cur` for mock Cost and Usage Report definition discovery

It also exposes local helper routes under `/_mock/*` for scenario control, status, and dashboard/debugging data. Those routes are not public AWS APIs.

## Quick Start

Build the binary and seed baseline data:

```bash
make setup
```

Start the service:

```bash
make serve
```

Set local AWS CLI credentials once:

```bash
export AWS_ACCESS_KEY_ID=test
export AWS_SECRET_ACCESS_KEY=test
export AWS_DEFAULT_REGION=us-east-1
export AWS_PAGER=""
```

Then call Foxtail through the AWS CLI:

```bash
aws --endpoint-url http://127.0.0.1:8080 ce get-cost-and-usage \
  --time-period Start=2026-03-01,End=2026-03-11 \
  --granularity DAILY \
  --metrics UnblendedCost
```

## Main Ways To Use It

### 1. AWS CLI Against Foxtail

This is the main service interface:

```bash
aws --endpoint-url http://127.0.0.1:8080 ...
```

Common examples:

```bash
aws --endpoint-url http://127.0.0.1:8080 ce get-cost-and-usage \
  --time-period Start=2026-03-01,End=2026-03-11 \
  --granularity DAILY \
  --metrics UnblendedCost \
  --group-by Type=DIMENSION,Key=SERVICE
```

```bash
aws --endpoint-url http://127.0.0.1:8080 cloudwatch list-metrics \
  --namespace AWS/EC2 \
  --metric-name CPUUtilization
```

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

### 2. `foxtail` Wrapper Command

Foxtail also includes a standalone wrapper at `target/debug/foxtail`.

- Supported FinOps commands are routed to Foxtail through `aws --endpoint-url http://127.0.0.1:8080`
- Everything else is delegated to `awslocal`

Examples:

```bash
target/debug/foxtail ce get-cost-and-usage \
  --time-period Start=2026-03-01,End=2026-03-11 \
  --granularity DAILY \
  --metrics UnblendedCost
```

```bash
target/debug/foxtail s3 ls
```

Useful wrapper flags:

- `--debug-routing`
- `--foxtail-endpoint <url>`
- `--aws-bin <path>`
- `--awslocal-bin <path>`

### 3. Local Helper Routes

These are for local debugging and control, not AWS parity:

- `GET /_mock/status`
- `POST /_mock/scenario`
- `GET /_mock/dashboard/data`
- `GET /_mock/dashboard/resources`
- `GET /_mock/dashboard/trends/cloudwatch`
- `GET /_mock/dashboard/trends/cost`

Example:

```bash
curl http://127.0.0.1:8080/_mock/status
```

## Supported AWS-Compatible Commands

### Cost Explorer

- `ce get-cost-and-usage`
- `ce get-cost-and-usage-with-resources`
- `ce get-cost-forecast`
- `ce get-usage-forecast`
- `ce get-dimension-values`
- `ce get-tags`
- `ce get-reservation-coverage`
- `ce get-reservation-utilization`
- `ce get-savings-plans-coverage`
- `ce get-savings-plans-utilization`
- `ce get-rightsizing-recommendation`
- `ce get-anomalies`
- `ce get-anomaly-monitors`
- `ce get-anomaly-subscriptions`

Notes:

- Cost Explorer targets accept both `AWSCostExplorer.*` and `AWSInsightsIndexService.*`
- `get-cost-and-usage-with-resources` defaults to resource grouping in this mock
- reservation, savings plan, anomaly, and rightsizing operations return synthetic mock outputs

### CloudWatch

- `cloudwatch list-metrics`
- `cloudwatch get-metric-statistics`
- `cloudwatch get-metric-data`

Notes:

- AWS CLI `cloudwatch ...` uses the Query/XML path
- direct `x-amz-target: GraniteServiceVersion20100801.GetMetricData` uses the JSON path
- `get-metric-data` supports up to 50 queries on both paths
- `get-metric-data` preserves query ids, aligns timestamps and values, and paginates deterministically
- supported stats are `Average`, `Sum`, `Minimum`, and `Maximum`

### Resource Groups Tagging API

- `resourcegroupstaggingapi get-resources`
- `resourcegroupstaggingapi get-tag-keys`
- `resourcegroupstaggingapi get-tag-values`

### Pricing

- `pricing get-products`

Notes:

- the mock catalog includes EC2, RDS, S3, and ELB examples
- `TERM_MATCH` filters work for common fields such as `instanceType`, `volumeType`, `databaseEngine`, `storageClass`, `loadBalancerType`, and `location`

### Compute Optimizer

- `compute-optimizer get-ec2-instance-recommendations`
- `compute-optimizer get-ebs-volume-recommendations`

These recommendations are derived from the seeded EC2 CPU and disk activity metrics.

### Cost and Usage Reports

- `cur describe-report-definitions`

## Data Model and Scenarios

Foxtail generates synthetic data for a few practical scenarios:

- `baseline`
- `spike`
- `idle-heavy`

You can regenerate the database for a specific scenario:

```bash
make gen-baseline
make gen-spike
make gen-idle-heavy
```

Or mutate the current seeded dataset in place:

```bash
curl -sS -X POST http://127.0.0.1:8080/_mock/scenario \
  -H 'content-type: application/json' \
  -d '{"scenario":"Spike"}'
```

If `AWS_MOCK_ADMIN_TOKEN` is set, callers must also send `x-mock-admin-token`.

## Suggested Usage Flow

If you change scenarios, do not assume resource ids stayed the same.

Use this flow:

1. Seed or switch a scenario
2. Discover resources with `ce get-dimension-values` or `resourcegroupstaggingapi get-resources`
3. Discover metrics with `cloudwatch list-metrics`
4. Query the returned ids with `cloudwatch get-metric-statistics` or `cloudwatch get-metric-data`

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

## Developer Commands

### Make Targets

| Command | Purpose |
| --- | --- |
| `make help` | Show local commands |
| `make build` | Build the debug binary |
| `make build-release` | Build the release binary |
| `make gen` | Discover resources and regenerate `mock_data.db` |
| `make gen-baseline` | Seed baseline scenario |
| `make gen-spike` | Seed spike scenario |
| `make gen-idle-heavy` | Seed idle-heavy scenario |
| `make serve` | Start the local API server |
| `make setup` | Build and seed baseline data |
| `make setup-mock` | Alias for `make setup` |
| `make verify-cli-interoperability` | Run the AWS CLI smoke suite |
| `make verify-wrapper-routing` | Verify wrapper routing with stub executables |

### Binary Commands

Main service binary:

```bash
target/debug/aws-mock-data-service --database-url sqlite:mock_data.db ...
```

Subcommands:

- `gen`
- `serve`

Examples:

```bash
target/debug/aws-mock-data-service gen \
  --endpoint-url http://localhost:4566 \
  --region us-east-1 \
  --scenario baseline \
  --prune \
  --json
```

```bash
target/debug/aws-mock-data-service serve --address 127.0.0.1 --port 8080
```

## Verification

Core local checks:

```bash
cargo fmt --all
cargo test
cargo clippy --all-targets --all-features
```

Smoke checks:

```bash
make verify-cli-interoperability
make verify-wrapper-routing
```

## Configuration

Useful environment variables:

- `DATABASE_URL`
- `AWS_ENDPOINT_URL`
- `AWS_DEFAULT_REGION`
- `AWS_MOCK_ADMIN_TOKEN`
- `FOXTAIL_ENDPOINT_URL`
- `FOXTAIL_AWS_BIN`
- `FOXTAIL_AWSLOCAL_BIN`
