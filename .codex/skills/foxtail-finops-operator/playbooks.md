# Foxtail Playbooks

## Bootstrap The Service

```bash
make setup
make serve
```

Set local AWS CLI credentials once:

```bash
export AWS_ACCESS_KEY_ID=test
export AWS_SECRET_ACCESS_KEY=test
export AWS_DEFAULT_REGION=us-east-1
export AWS_PAGER=""
```

## Regenerate Known Scenarios

Use these when you want a clean reseed:

```bash
make gen-baseline
make gen-spike
make gen-idle-heavy
```

Equivalent binary form:

```bash
DATABASE_URL="sqlite:mock_data.db" target/debug/aws-mock-data-service gen --prune --scenario baseline
DATABASE_URL="sqlite:mock_data.db" target/debug/aws-mock-data-service gen --prune --scenario spike
DATABASE_URL="sqlite:mock_data.db" target/debug/aws-mock-data-service gen --prune --scenario idle-heavy
```

## Mutate The Active DB In Place

Use this when you want to reuse the current DB and flip resource behavior quickly:

```bash
curl -sS -X POST http://127.0.0.1:8080/_mock/scenario \
  -H 'content-type: application/json' \
  -d '{"scenario":"Spike"}'
```

Per-resource variant:

```bash
curl -sS -X POST http://127.0.0.1:8080/_mock/scenario \
  -H 'content-type: application/json' \
  -d '{"scenario":"IdleHeavy","resource_id":"i-20652c71bedc57ced"}'
```

Valid JSON `scenario` values on this route are the enum names:

- `Baseline`
- `Spike`
- `IdleHeavy`

## Public AWS CLI FinOps Workflow

All of the commands below should be run against:

```bash
--endpoint-url http://127.0.0.1:8080
```

### 1. Discover Inventory And Tags

```bash
aws --endpoint-url http://127.0.0.1:8080 ce get-dimension-values \
  --time-period Start=2026-03-01,End=2026-03-11 \
  --dimension SERVICE
```

```bash
aws --endpoint-url http://127.0.0.1:8080 ce get-dimension-values \
  --time-period Start=2026-03-01,End=2026-03-11 \
  --dimension RESOURCE_ID
```

```bash
aws --endpoint-url http://127.0.0.1:8080 resourcegroupstaggingapi get-resources \
  --resources-per-page 20
```

```bash
aws --endpoint-url http://127.0.0.1:8080 resourcegroupstaggingapi get-tag-keys
aws --endpoint-url http://127.0.0.1:8080 resourcegroupstaggingapi get-tag-values --key Name
```

### 2. Pull Cost And Usage Views

```bash
aws --endpoint-url http://127.0.0.1:8080 ce get-cost-and-usage \
  --time-period Start=2026-03-01,End=2026-03-11 \
  --granularity DAILY \
  --metrics UnblendedCost \
  --group-by Type=DIMENSION,Key=SERVICE
```

```bash
aws --endpoint-url http://127.0.0.1:8080 ce get-cost-and-usage-with-resources \
  --time-period Start=2026-03-01,End=2026-03-11 \
  --granularity DAILY \
  --metrics UnblendedCost \
  --filter '{"Dimensions":{"Key":"SERVICE","Values":["Amazon Elastic Compute Cloud - Compute"]}}'
```

```bash
aws --endpoint-url http://127.0.0.1:8080 ce get-cost-forecast \
  --time-period Start=2026-03-01,End=2026-03-11 \
  --metric UNBLENDED_COST \
  --granularity DAILY
```

```bash
aws --endpoint-url http://127.0.0.1:8080 ce get-usage-forecast \
  --time-period Start=2026-03-01,End=2026-03-11 \
  --metric USAGE_QUANTITY \
  --granularity DAILY
```

### 3. Pull Optimization Signals

```bash
aws --endpoint-url http://127.0.0.1:8080 ce get-rightsizing-recommendation \
  --service AmazonEC2
```

```bash
aws --endpoint-url http://127.0.0.1:8080 compute-optimizer get-ec2-instance-recommendations
aws --endpoint-url http://127.0.0.1:8080 compute-optimizer get-ebs-volume-recommendations
```

```bash
aws --endpoint-url http://127.0.0.1:8080 ce get-anomalies \
  --date-interval StartDate=2026-03-01,EndDate=2026-03-11
```

### 4. Pull Pricing References

```bash
aws --endpoint-url http://127.0.0.1:8080 pricing get-products \
  --service-code AmazonEC2 \
  --format-version aws_v1 \
  --filters Type=TERM_MATCH,Field=instanceType,Value=m6i.large
```

```bash
aws --endpoint-url http://127.0.0.1:8080 pricing get-products \
  --service-code AmazonEC2 \
  --format-version aws_v1 \
  --filters Type=TERM_MATCH,Field=volumeType,Value=gp3
```

### 5. Correlate With CloudWatch

Discover metrics first:

```bash
aws --endpoint-url http://127.0.0.1:8080 cloudwatch list-metrics \
  --namespace AWS/EC2 \
  --metric-name CPUUtilization
```

Then pull a time series:

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

For `get-metric-data`, generate the query payload from discovered resources rather than hardcoding old IDs:

```bash
aws --endpoint-url http://127.0.0.1:8080 cloudwatch list-metrics \
  --namespace AWS/EC2 \
  --metric-name CPUUtilization \
  --output json > /tmp/list-metrics.json

jq '[
  .Metrics[]
  | .Dimensions[]
  | select(.Name == "InstanceId")
  | {
      Id: ("m" + (.Value | gsub("[^A-Za-z0-9]"; ""))),
      MetricStat: {
        Metric: {
          Namespace: "AWS/EC2",
          MetricName: "CPUUtilization",
          Dimensions: [{Name: "InstanceId", Value: .Value}]
        },
        Period: 3600,
        Stat: "Average"
      }
    }
]' /tmp/list-metrics.json > /tmp/metric-queries.json

aws --endpoint-url http://127.0.0.1:8080 cloudwatch get-metric-data \
  --metric-data-queries file:///tmp/metric-queries.json \
  --start-time 2026-03-11T00:00:00Z \
  --end-time 2026-03-11T12:00:00Z
```

## Add A New Scenario In Code

When the built-in scenarios are not enough, update the code in this order:

1. Add the enum variant in [src/cli.rs](/Users/murphy/workspace/iacai0/foxtail/src/cli.rs).
2. Add its metric and cost behavior in [src/generator.rs](/Users/murphy/workspace/iacai0/foxtail/src/generator.rs).
3. Add a `make gen-...` target in [Makefile](/Users/murphy/workspace/iacai0/foxtail/Makefile).
4. Update [README.md](/Users/murphy/workspace/iacai0/foxtail/README.md) and this skill if the workflow changes.
5. Seed the new scenario, run the public AWS CLI commands above, and rerun:

```bash
cargo fmt
cargo test
cargo clippy --all-targets --all-features
bash scripts/verify_cli_interop.sh
```

## Troubleshooting

- If AWS CLI calls fail with `UnsupportedAction`, the running server is older than the current code. Restart it.
- If the smoke script prints connection errors before succeeding, that is just startup polling.
- If a CloudWatch query stops matching the scenario, rediscover resource IDs with `list-metrics` before rebuilding `get-metric-data` queries.
- If regenerated data looks stale, reseed with `make gen-*` instead of mutating in place.
