#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${BIN:-$ROOT_DIR/target/debug/aws-mock-data-service}"
SOURCE_DB="${MOCK_DATA_DB:-$ROOT_DIR/mock_data.db}"
PORT="${AWS_MOCK_VERIFY_PORT:-18080}"
TMP_DIR="$(mktemp -d)"
TMP_DB="$TMP_DIR/mock_data.db"
SERVER_LOG="$TMP_DIR/server.log"

log_step() {
  echo "[verify-cli] $*"
}

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

if ! command -v aws >/dev/null 2>&1; then
  echo "aws CLI is required for CLI interoperability verification" >&2
  exit 1
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required for CLI interoperability verification" >&2
  exit 1
fi

if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "sqlite3 is required for CLI interoperability verification" >&2
  exit 1
fi

if [[ ! -f "$SOURCE_DB" ]]; then
  echo "Seed database not found at $SOURCE_DB" >&2
  exit 1
fi

cd "$ROOT_DIR"
log_step "Building debug binary"
cargo build >/dev/null
log_step "Backing up seed database to isolated temp file"
sqlite3 "$SOURCE_DB" ".backup '$TMP_DB'" >/dev/null

log_step "Starting temporary mock server on http://127.0.0.1:$PORT"
DATABASE_URL="sqlite:$TMP_DB" "$BIN" serve --port "$PORT" >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!

for _ in {1..50}; do
  if curl -fsS "http://127.0.0.1:$PORT/_mock/status" >/dev/null 2>/dev/null; then
    break
  fi
  sleep 0.2
done

if ! curl -fsS "http://127.0.0.1:$PORT/_mock/status" >/dev/null 2>/dev/null; then
  echo "mock server failed to start; see $SERVER_LOG" >&2
  exit 1
fi

export AWS_ACCESS_KEY_ID=test
export AWS_SECRET_ACCESS_KEY=test
export AWS_DEFAULT_REGION=us-east-1
export AWS_PAGER=""

ENDPOINT="http://127.0.0.1:$PORT"
readarray -t VERIFY_DATES < <(python3 - <<'PY'
from datetime import datetime, timedelta, timezone

now = datetime.now(timezone.utc).replace(minute=0, second=0, microsecond=0)
print((now.date() - timedelta(days=10)).isoformat())
print(now.date().isoformat())
print((now - timedelta(hours=12)).isoformat().replace("+00:00", "Z"))
print(now.isoformat().replace("+00:00", "Z"))
PY
)
CE_START_DAY="${VERIFY_DATES[0]}"
CE_END_DAY="${VERIFY_DATES[1]}"
CW_START_TIME="${VERIFY_DATES[2]}"
CW_END_TIME="${VERIFY_DATES[3]}"

aws_json() {
  aws --output json --endpoint-url "$ENDPOINT" "$@"
}

SERVICE_DIMENSIONS="$(aws_json ce get-dimension-values \
  --time-period "Start=$CE_START_DAY,End=$CE_END_DAY" \
  --dimension SERVICE)"
log_step "Verified Cost Explorer: get-dimension-values"

GROUPED_COSTS="$(aws_json ce get-cost-and-usage \
  --time-period "Start=$CE_START_DAY,End=$CE_END_DAY" \
  --granularity DAILY \
  --metrics UnblendedCost \
  --group-by Type=DIMENSION,Key=SERVICE)"
log_step "Verified Cost Explorer: get-cost-and-usage with group-by SERVICE"

RESOURCE_FILTER_FILE="$TMP_DIR/resource_filter.json"
cat >"$RESOURCE_FILTER_FILE" <<'EOF'
{
  "Dimensions": {
    "Key": "SERVICE",
    "Values": ["Amazon Elastic Compute Cloud - Compute"]
  }
}
EOF

RESOURCE_COSTS="$(aws_json ce get-cost-and-usage-with-resources \
  --time-period "Start=$CE_START_DAY,End=$CE_END_DAY" \
  --granularity DAILY \
  --metrics UnblendedCost \
  --filter "file://$RESOURCE_FILTER_FILE")"
log_step "Verified Cost Explorer: get-cost-and-usage-with-resources"

RESOURCE_TAGS="$(aws_json ce get-tags \
  --time-period "Start=$CE_START_DAY,End=$CE_END_DAY" \
  --tag-key Name)"
log_step "Verified Cost Explorer: get-tags"

TAGGED_RESOURCES="$(aws_json resourcegroupstaggingapi get-resources \
  --resources-per-page 5)"
log_step "Verified Resource Groups Tagging API: get-resources"

TAG_KEYS="$(aws_json resourcegroupstaggingapi get-tag-keys)"
log_step "Verified Resource Groups Tagging API: get-tag-keys"

TAG_VALUES="$(aws_json resourcegroupstaggingapi get-tag-values \
  --key Name)"
log_step "Verified Resource Groups Tagging API: get-tag-values"

PRICING_PRODUCTS="$(aws_json pricing get-products \
  --service-code AmazonEC2 \
  --format-version aws_v1 \
  --filters Type=TERM_MATCH,Field=instanceType,Value=m6i.large)"
log_step "Verified Pricing: get-products"

PRICING_STORAGE_PRODUCTS="$(aws_json pricing get-products \
  --service-code AmazonEC2 \
  --format-version aws_v1 \
  --filters Type=TERM_MATCH,Field=volumeType,Value=gp3)"
log_step "Verified Pricing: gp3 storage product lookup"

PRICING_PAGED_PRODUCTS="$(aws_json pricing get-products \
  --service-code AmazonEC2 \
  --format-version aws_v1 \
  --max-results 1)"
log_step "Verified Pricing: pagination"

EC2_RECOMMENDATIONS="$(aws_json compute-optimizer get-ec2-instance-recommendations)"
log_step "Verified Compute Optimizer: get-ec2-instance-recommendations"

EBS_RECOMMENDATIONS="$(aws_json compute-optimizer get-ebs-volume-recommendations)"
log_step "Verified Compute Optimizer: get-ebs-volume-recommendations"

USAGE_FORECAST="$(aws_json ce get-usage-forecast \
  --time-period "Start=$CE_START_DAY,End=$CE_END_DAY" \
  --metric USAGE_QUANTITY \
  --granularity DAILY)"
log_step "Verified Cost Explorer: get-usage-forecast"

log_step "Verified Cost Explorer: get-cost-forecast"
aws_json ce get-cost-forecast \
  --time-period "Start=$CE_START_DAY,End=$CE_END_DAY" \
  --metric UNBLENDED_COST \
  --granularity DAILY >/dev/null

log_step "Verified Cost Explorer: get-rightsizing-recommendation"
aws_json ce get-rightsizing-recommendation --service AmazonEC2 >/dev/null
log_step "Verified Cost Explorer: get-anomalies"
aws_json ce get-anomalies --date-interval "StartDate=$CE_START_DAY,EndDate=$CE_END_DAY" >/dev/null
log_step "Verified Cost Explorer: get-anomaly-monitors"
aws_json ce get-anomaly-monitors >/dev/null
log_step "Verified Cost Explorer: get-anomaly-subscriptions"
aws_json ce get-anomaly-subscriptions >/dev/null

CUR_REPORT_DEFINITIONS="$(aws_json cur describe-report-definitions)"
log_step "Verified CUR: describe-report-definitions"

EC2_METRICS="$(aws_json cloudwatch list-metrics \
  --namespace AWS/EC2 \
  --metric-name CPUUtilization)"
log_step "Verified CloudWatch: list-metrics"

log_step "Derived a test InstanceId from list-metrics output"
INSTANCE_ID="$(
  SERVICE_DIMENSIONS="$SERVICE_DIMENSIONS" \
  GROUPED_COSTS="$GROUPED_COSTS" \
  RESOURCE_COSTS="$RESOURCE_COSTS" \
  RESOURCE_TAGS="$RESOURCE_TAGS" \
  TAGGED_RESOURCES="$TAGGED_RESOURCES" \
  TAG_KEYS="$TAG_KEYS" \
  TAG_VALUES="$TAG_VALUES" \
  PRICING_PRODUCTS="$PRICING_PRODUCTS" \
  PRICING_STORAGE_PRODUCTS="$PRICING_STORAGE_PRODUCTS" \
  PRICING_PAGED_PRODUCTS="$PRICING_PAGED_PRODUCTS" \
  USAGE_FORECAST="$USAGE_FORECAST" \
  EC2_RECOMMENDATIONS="$EC2_RECOMMENDATIONS" \
  EBS_RECOMMENDATIONS="$EBS_RECOMMENDATIONS" \
  CUR_REPORT_DEFINITIONS="$CUR_REPORT_DEFINITIONS" \
  EC2_METRICS="$EC2_METRICS" \
  python3 - <<'PY'
import json
import os
import sys

dimensions = json.loads(os.environ["SERVICE_DIMENSIONS"])
if not dimensions.get("DimensionValues"):
    raise SystemExit("no service dimension values returned")

grouped = json.loads(os.environ["GROUPED_COSTS"])
results = grouped.get("ResultsByTime", [])
if not results or not any(bucket.get("Groups") for bucket in results):
    raise SystemExit("grouped cost results are empty")

resource_costs = json.loads(os.environ["RESOURCE_COSTS"])
resource_results = resource_costs.get("ResultsByTime", [])
if not resource_results or not any(bucket.get("Groups") for bucket in resource_results):
    raise SystemExit("resource-level cost results are empty")

resource_tags = json.loads(os.environ["RESOURCE_TAGS"])
if not resource_tags.get("Tags"):
    raise SystemExit("tag discovery returned no tag values")

tagged_resources = json.loads(os.environ["TAGGED_RESOURCES"])
if not tagged_resources.get("ResourceTagMappingList"):
    raise SystemExit("tagged resource inventory is empty")

tag_keys = json.loads(os.environ["TAG_KEYS"])
if not tag_keys.get("TagKeys"):
    raise SystemExit("tag key discovery returned no tag keys")

tag_values = json.loads(os.environ["TAG_VALUES"])
if not tag_values.get("TagValues"):
    raise SystemExit("tag value discovery returned no tag values")

pricing_products = json.loads(os.environ["PRICING_PRODUCTS"])
if not pricing_products.get("PriceList"):
    raise SystemExit("pricing product list is empty")

pricing_storage_products = json.loads(os.environ["PRICING_STORAGE_PRODUCTS"])
if not pricing_storage_products.get("PriceList"):
    raise SystemExit("pricing storage product list is empty")

pricing_paged_products = json.loads(os.environ["PRICING_PAGED_PRODUCTS"])
if not pricing_paged_products.get("NextToken"):
    raise SystemExit("pricing pagination did not return NextToken")

usage_forecast = json.loads(os.environ["USAGE_FORECAST"])
if not usage_forecast.get("ForecastResultsByTime"):
    raise SystemExit("usage forecast results are empty")

ec2_recommendations = json.loads(os.environ["EC2_RECOMMENDATIONS"])
if not ec2_recommendations.get("instanceRecommendations"):
    raise SystemExit("compute optimizer EC2 recommendations are empty")

ebs_recommendations = json.loads(os.environ["EBS_RECOMMENDATIONS"])
if not ebs_recommendations.get("volumeRecommendations"):
    raise SystemExit("compute optimizer EBS recommendations are empty")

cur_reports = json.loads(os.environ["CUR_REPORT_DEFINITIONS"])
if not cur_reports.get("ReportDefinitions"):
    raise SystemExit("CUR report definitions are empty")

metrics = json.loads(os.environ["EC2_METRICS"]).get("Metrics", [])
for metric in metrics:
    for dimension in metric.get("Dimensions", []):
        if dimension.get("Name") == "InstanceId" and dimension.get("Value"):
            sys.stdout.write(dimension["Value"])
            raise SystemExit(0)

raise SystemExit("no EC2 InstanceId discovered via list-metrics")
PY
)"

NETWORK_STATS_OUTPUT="$(aws_json cloudwatch get-metric-statistics \
  --namespace AWS/EC2 \
  --metric-name NetworkIn \
  --statistics Average \
  --period 3600 \
  --start-time "$CW_START_TIME" \
  --end-time "$CW_END_TIME" \
  --dimensions Name=InstanceId,Value="$INSTANCE_ID")"
log_step "Verified CloudWatch: get-metric-statistics"

NETWORK_STATS_OUTPUT="$NETWORK_STATS_OUTPUT" python3 - <<'PY'
import json
import os

payload = json.loads(os.environ["NETWORK_STATS_OUTPUT"])
datapoints = payload.get("Datapoints", [])
if not datapoints:
    raise SystemExit("NetworkIn get-metric-statistics returned no datapoints")
if not any(point.get("Average", 0) > 0 for point in datapoints):
    raise SystemExit("NetworkIn get-metric-statistics returned only zero datapoints")
PY

QUERY_FILE="$TMP_DIR/metric_queries.json"
cat >"$QUERY_FILE" <<EOF
[
  {
    "Id": "netin",
    "MetricStat": {
      "Metric": {
        "Namespace": "AWS/EC2",
        "MetricName": "NetworkIn",
        "Dimensions": [
          {
            "Name": "InstanceId",
            "Value": "$INSTANCE_ID"
          }
        ]
      },
      "Period": 3600,
      "Stat": "Average"
    }
  },
  {
    "Id": "netout",
    "MetricStat": {
      "Metric": {
        "Namespace": "AWS/EC2",
        "MetricName": "NetworkOut",
        "Dimensions": [
          {
            "Name": "InstanceId",
            "Value": "$INSTANCE_ID"
          }
        ]
      },
      "Period": 3600,
      "Stat": "Average"
    }
  }
]
EOF

METRIC_DATA_OUTPUT="$(aws_json cloudwatch get-metric-data \
  --metric-data-queries "file://$QUERY_FILE" \
  --start-time "$CW_START_TIME" \
  --end-time "$CW_END_TIME")"
log_step "Verified CloudWatch: get-metric-data"

METRIC_DATA_OUTPUT="$METRIC_DATA_OUTPUT" python3 - <<'PY'
import json
import os

payload = json.loads(os.environ["METRIC_DATA_OUTPUT"])
results = payload.get("MetricDataResults", [])
if len(results) != 2:
    raise SystemExit("expected exactly two MetricDataResults")

results_by_id = {result.get("Id"): result for result in results}
for query_id in ("netin", "netout"):
    result = results_by_id.get(query_id)
    if result is None:
        raise SystemExit(f"missing MetricDataResult for {query_id}")

    timestamps = result.get("Timestamps", [])
    values = result.get("Values", [])
    if len(timestamps) != len(values):
        raise SystemExit(f"timestamps and values are not aligned for {query_id}")
    if len(timestamps) != len(set(timestamps)):
        raise SystemExit(f"duplicate timestamps found in MetricDataResult for {query_id}")
    if timestamps and not any(value > 0 for value in values):
        raise SystemExit(f"{query_id} returned only zero datapoints")
PY

log_step "Verified get-metric-data result shape: preserved query ids, aligned arrays, unique timestamps, non-zero network datapoints"
echo "CLI interoperability verification passed on $ENDPOINT"
