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

### 1. `foxtail` Command

Foxtail builds one binary at `target/debug/foxtail`.

Native commands run directly in the binary:

```bash
target/debug/foxtail gen --scenario idle-heavy --prune
target/debug/foxtail serve --port 8080
```

AWS CLI-compatible commands use the same binary:

```bash
target/debug/foxtail ce get-cost-and-usage \
  --time-period Start=2026-03-01,End=2026-03-11 \
  --granularity DAILY \
  --metrics UnblendedCost
```

Supported FinOps commands are routed to the running Foxtail service through `aws --endpoint-url http://127.0.0.1:8080`. Everything else is delegated to `awslocal`:

```bash
target/debug/foxtail s3 ls
```

Useful routing flags:

- `--debug-routing`
- `--foxtail-endpoint <url>`
- `--aws-bin <path>`
- `--awslocal-bin <path>`

Native command configuration:

- `--database-url <url>`
- `DATABASE_URL`
- `AWS_ENDPOINT_URL`
- `AWS_DEFAULT_REGION`

### 2. AWS CLI Against Foxtail

You can also call the service directly with the AWS CLI:

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
  --statistics Average Maximum \
  --period 3600 \
  --start-time 2026-03-11T00:00:00Z \
  --end-time 2026-03-11T12:00:00Z \
  --dimensions Name=InstanceId,Value=i-20652c71bedc57ced
```

### 3. Local Helper Routes

These are for local debugging and control, not AWS parity:

- `GET /_mock/status`
- `POST /_mock/scenario`
- `GET /_mock/fixture/definition`
- `POST /_mock/fixture/realize`
- `GET /_mock/fixture/status`
- `GET /_mock/fixture/manifest`
- `GET /_mock/fixture/identities`
- `GET /_mock/fixture/mutation/status`
- `POST /_mock/fixture/fault`
- `POST /_mock/fixture/reset`
- `POST /_mock/fixture/recreate`
- `POST /_mock/fixture/destroy`
- `GET /_mock/dashboard/data`
- `GET /_mock/dashboard/resources`
- `GET /_mock/dashboard/trends/cloudwatch`
- `GET /_mock/dashboard/trends/cost`

Example:

```bash
curl http://127.0.0.1:8080/_mock/status
```

### Release-Qualification Fixture v1

The release-qualification fixture is a deterministic tracer bullet for the five
positive, negative, and degraded controls declared by the v1 contract. Realize
it against a disposable LocalStack copy: realization materializes the fixture's
public metric and cost evidence for five EC2 identities, while the source estate
is never changed. The definition is available before realization; the manifest
and realized identities are published only after a successful realization.

The native CLI and local HTTP routes expose the same canonical JSON documents:

~~~~bash
target/debug/foxtail fixture definition
target/debug/foxtail fixture realize
curl http://127.0.0.1:8080/_mock/fixture/definition
curl http://127.0.0.1:8080/_mock/fixture/status
curl -X POST http://127.0.0.1:8080/_mock/fixture/realize \
  -H 'content-type: application/json' \
  -d '{"version":"release-qualification-v1"}'
~~~~

The manifest binds the definition digest, generator and LocalStack provenance,
clock anchor, AWS account/region scope, realized read-only identities, and (in
isolated mode only) four fresh generation-owned EC2 identities: `stop`,
`resize`, `stop-recovery`, and `resize-restoration`. The ordinary
AWS-compatible inventory, CloudWatch, Cost Explorer, and Compute Optimizer
routes remain the authoritative evidence surfaces. The default public account
scope is `123456789012`; an explicit fixture `account_id` must match it so
manifest ARNs and public identities cannot diverge.

Without the exact `FOXTAIL_QUALIFICATION_ENV=isolated` value, `fixture realize`
keeps those mutation controls declared-only and does not call EC2 or write a
mutation generation. In isolated mode, Foxtail uses the `AWS_ENDPOINT_URL` (or
the request's `endpoint_url`) to call EC2 `RunInstances`, `StopInstances`,
`ModifyInstanceAttribute`, `StartInstances`, `DescribeInstances`, and
`TerminateInstances`. Set `FOXTAIL_MUTATION_AMI_ID` (and, when required by the
endpoint, `FOXTAIL_MUTATION_SUBNET_ID` and `FOXTAIL_MUTATION_SECURITY_GROUP_ID`)
to values valid for that LocalStack account. A generation is not considered
complete until its returned public IDs, states, and instance types reconcile.

Mutation controls are qualification-only. Set `FOXTAIL_QUALIFICATION_ENV=isolated`
in the disposable process, and send `x-mock-admin-token` when
`AWS_MOCK_ADMIN_TOKEN` is configured. Every mutation request must repeat the
current generation, mutation-generation id, and exact manifest digest. Fault
receipts return one-use reset tokens; stale, duplicate, malformed, or
ambiguous requests fail without changing state. The `destroy` receipt proves
all generation-owned identities are absent from public Resource Groups
inventory and all active faults have been reset. Recreate and destroy receipts
encode `external_ec2_termination` as an identity-keyed object: each exact EC2
target ID appears once with a `terminated` or service-level `not-found` value.

Example authority-bound lifecycle (values come from `fixture manifest`):

```bash
export FOXTAIL_QUALIFICATION_ENV=isolated
target/debug/foxtail fixture mutation-status
target/debug/foxtail fixture fault \
  --generation 1 \
  --manifest-digest sha256:... \
  --mutation-generation 1 \
  --mutation-generation-id mg-0001 \
  --control-id ec2-mutation-stop-001 \
  --target-id i-foxtail-mutation-g0001-stop \
  --scope target --fault-kind stop
```

The target ID in a live generation is the ID returned by EC2 and copied from
the current manifest; the example ID is only the deterministic `mock://` test
backend form. After `fault` or `reset`, verify the state/type with public EC2
`DescribeInstances`; after `destroy`, verify every retired ID is either
service-level not-found or exactly `terminated` in EC2, and is absent from
Resource Groups Tagging. AWS may retain terminated instances in
`DescribeInstances` for approximately one hour; see the official
[TerminateInstances documentation](https://docs.aws.amazon.com/AWSEC2/latest/APIReference/API_TerminateInstances.html).

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
- `get-cost-and-usage` supports grouping by `SERVICE`, `REGION`, `RESOURCE_ID`, and `USAGE_TYPE`
- `get-cost-and-usage-with-resources` defaults to resource grouping in this mock
- reservation, savings plan, anomaly, and rightsizing operations return synthetic mock outputs

### CloudWatch

- `cloudwatch list-metrics`
- `cloudwatch get-metric-statistics`
- `cloudwatch get-metric-data`

Notes:

- AWS CLI `cloudwatch ...` may use CloudWatch JSON query mode or the older Query/XML path depending on CLI/botocore version
- direct `x-amz-target: GraniteServiceVersion20100801.*` uses the JSON path
- `get-metric-statistics` supports `SampleCount`, `Average`, `Sum`, `Minimum`, and `Maximum`
- `list-metrics`, `get-metric-statistics`, and `get-metric-data` are supported on both JSON and Query/XML paths
- ElastiCache clusters discovered during generation use the `AWS/ElastiCache` namespace with `CacheClusterId` dimensions
- `get-metric-data` supports up to 50 queries on both paths
- `get-metric-data` preserves query ids, aligns timestamps and values, and paginates deterministically
- `get-metric-data` supports `Average`, `Sum`, `Minimum`, and `Maximum`

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
| `make reset` | Delete `mock_data.db` |
| `make serve` | Start the local API server |
| `make setup` | Build if needed and seed baseline data |
| `make setup-mock` | Alias for `make setup` |
| `make verify-cli-interoperability` | Run the AWS CLI smoke suite |
| `make verify-wrapper-routing` | Verify wrapper routing with stub executables |

### Binary Commands

Foxtail produces one binary:

```bash
target/debug/foxtail --database-url sqlite:mock_data.db ...
```

Native subcommands:

- `gen`
- `serve`

Examples:

```bash
target/debug/foxtail gen \
  --endpoint-url http://localhost:4566 \
  --region us-east-1 \
  --scenario baseline \
  --prune \
  --json
```

```bash
target/debug/foxtail serve --address 127.0.0.1 --port 8080
```

AWS CLI-compatible examples:

```bash
target/debug/foxtail ce get-cost-and-usage \
  --time-period Start=2026-03-01,End=2026-03-11 \
  --granularity DAILY \
  --metrics UnblendedCost
```

```bash
target/debug/foxtail s3 ls
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
