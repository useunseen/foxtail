#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${BIN:-$ROOT_DIR/target/debug/foxtail}"
SOURCE_DB="${MOCK_DATA_DB:-$ROOT_DIR/mock_data.db}"
PORT="${AWS_MOCK_VERIFY_PORT:-18080}"
LOCALSTACK_ENDPOINT="${AWS_ENDPOINT_URL:-http://127.0.0.1:4566}"
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

if [[ -z "${FOXTAIL_MUTATION_AMI_ID:-}" ]]; then
  echo "FOXTAIL_MUTATION_AMI_ID must name a valid disposable LocalStack AMI" >&2
  exit 1
fi
export AWS_ENDPOINT_URL="$LOCALSTACK_ENDPOINT"
if ! aws --output json --endpoint-url "$LOCALSTACK_ENDPOINT" ec2 describe-images \
  --image-ids "$FOXTAIL_MUTATION_AMI_ID" >/dev/null 2>&1; then
  echo "LocalStack EC2 endpoint is unavailable or FOXTAIL_MUTATION_AMI_ID is invalid: $LOCALSTACK_ENDPOINT" >&2
  exit 1
fi

cd "$ROOT_DIR"
log_step "Building debug binary"
cargo build >/dev/null
log_step "Backing up seed database to isolated temp file"
sqlite3 "$SOURCE_DB" ".backup '$TMP_DB'" >/dev/null
DATABASE_URL="sqlite:$TMP_DB" "$BIN" fixture status >/dev/null

FIXTURE_SEED_IDS=(
  "i-foxtail-fixture-0"
  "i-foxtail-fixture-1"
  "i-foxtail-fixture-2"
  "i-foxtail-fixture-3"
  "i-foxtail-fixture-4"
)
for fixture_id in "${FIXTURE_SEED_IDS[@]}"; do
  sqlite3 "$TMP_DB" "INSERT OR IGNORE INTO resources (id, resource_type, region, scenario, tags) VALUES ('$fixture_id', 'ec2', 'us-east-1', 'Baseline', '{\"Name\":\"$fixture_id\"}');"
done
sqlite3 "$TMP_DB" "UPDATE resources SET tags = '{\"Name\":' || char(34) || id || char(34) || '}' WHERE resource_type = 'ec2';"
FIXTURE_EC2_COUNT="$(sqlite3 "$TMP_DB" "SELECT COUNT(*) FROM resources WHERE resource_type = 'ec2';")"
if [[ "$FIXTURE_EC2_COUNT" -lt 5 ]]; then
  echo "fixture seed did not produce five EC2 resources" >&2
  exit 1
fi

export FOXTAIL_QUALIFICATION_ENV=isolated
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
print((now - timedelta(days=15)).isoformat().replace("+00:00", "Z"))
PY
)
CE_START_DAY="${VERIFY_DATES[0]}"
CE_END_DAY="${VERIFY_DATES[1]}"
CW_START_TIME="${VERIFY_DATES[2]}"
CW_END_TIME="${VERIFY_DATES[3]}"
FIXTURE_CW_START_TIME="${VERIFY_DATES[4]}"
FIXTURE_CW_END_TIME="${VERIFY_DATES[3]}"
FIXTURE_CE_START_DAY="${VERIFY_DATES[4]%%T*}"
FIXTURE_CE_END_DAY="${VERIFY_DATES[1]}"

aws_json() {
  aws --output json --endpoint-url "$ENDPOINT" "$@"
}

localstack_json() {
  aws --output json --endpoint-url "$LOCALSTACK_ENDPOINT" "$@"
}

python3 "$ROOT_DIR/scripts/validate_release_fixture.py" --negative

FIXTURE_DEFINITION="$(curl -fsS "$ENDPOINT/_mock/fixture/definition?version=release-qualification-v1")"
FIXTURE_STATUS="$(curl -fsS "$ENDPOINT/_mock/fixture/status")"
CLI_FIXTURE_DEFINITION="$("$BIN" --database-url "sqlite:$TMP_DB" fixture definition)"
CLI_FIXTURE_STATUS="$("$BIN" --database-url "sqlite:$TMP_DB" fixture status)"
if [[ "$CLI_FIXTURE_DEFINITION" != "$FIXTURE_DEFINITION" ]]; then
  echo "fixture definition CLI/HTTP bytes differ" >&2
  exit 1
fi
if [[ "$CLI_FIXTURE_STATUS" != "$FIXTURE_STATUS" ]]; then
  echo "fixture status CLI/HTTP bytes differ" >&2
  exit 1
fi
FIXTURE_DEFINITION="$FIXTURE_DEFINITION" FIXTURE_STATUS="$FIXTURE_STATUS" python3 - <<'PY'
import json
import os

definition = json.loads(os.environ["FIXTURE_DEFINITION"])
status = json.loads(os.environ["FIXTURE_STATUS"])
if definition.get("schema") != "foxtail.release-fixture-definition/v1":
    raise SystemExit("fixture definition schema mismatch")
if not definition.get("digest", "").startswith("sha256:"):
    raise SystemExit("fixture definition digest missing")
if status.get("status") != "ABSENT":
    raise SystemExit("fresh fixture state should be ABSENT")
PY
log_step "Verified release fixture: definition and pre-realization status"

FIXTURE_ANCHOR="2026-08-05T00:00:00Z"
CLI_DB="$TMP_DIR/cli-fixture.db"
sqlite3 "$TMP_DB" ".backup '$CLI_DB'" >/dev/null
FIXTURE_REALIZATION="$(curl -fsS -X POST "$ENDPOINT/_mock/fixture/realize" \
  -H 'content-type: application/json' \
  -d "{\"version\":\"release-qualification-v1\",\"clock_anchor\":\"$FIXTURE_ANCHOR\"}")"
# The HTTP realization is copied into the CLI database first. A repeat realize
# without an explicit recreation request is an authority-bound read, so this
# checks exact canonical bytes without provisioning a second external generation.
CLI_REALIZATION="$("$BIN" --database-url "sqlite:$CLI_DB" fixture realize)"
if [[ "$CLI_REALIZATION" != "$FIXTURE_REALIZATION" ]]; then
  echo "fixture realization CLI/HTTP bytes differ" >&2
  exit 1
fi
FIXTURE_REALIZATION="$FIXTURE_REALIZATION" python3 - <<'PY'
import json
import os

payload = json.loads(os.environ["FIXTURE_REALIZATION"])
manifest = payload.get("manifest", {})
if manifest.get("schema") != "foxtail.release-fixture-manifest/v1":
    raise SystemExit("fixture manifest schema mismatch")
if payload.get("manifest_digest") != manifest.get("digest"):
    raise SystemExit("fixture manifest digest mismatch")
resources = manifest.get("resources", [])
if len(resources) != 5:
    raise SystemExit("fixture realization did not publish five realized controls")
expected = {
    "ec2-idle-positive-001",
    "ec2-idle-negative-001",
    "ec2-idle-degraded-001",
    "ec2-resize-positive-001",
    "ec2-resize-negative-001",
}
if {resource.get("control_id") for resource in resources} != expected:
    raise SystemExit("fixture realization published an unexpected control set")
if len({resource.get("resource_id") for resource in resources}) != 5:
    raise SystemExit("fixture realization published duplicate resource identities")
for resource in resources:
    observed = resource.get("observed", {})
    if observed.get("metric_count", 0) <= 0 or observed.get("cost_record_count", 0) != 14:
        raise SystemExit("fixture realization did not publish observed metric and cost rows")
    if resource.get("evidence", {}).get("cost_complete_days") != 14:
        raise SystemExit("fixture realization did not publish complete cost history")
degraded = next(resource for resource in resources if resource["control_id"] == "ec2-idle-degraded-001")
if degraded["evidence"].get("cloudwatch_complete_days") != 13:
    raise SystemExit("degraded fixture control did not expose one missing CPU history day")
if len(degraded["evidence"].get("cloudwatch_missing_offsets", [])) != 1:
    raise SystemExit("degraded fixture control did not expose exactly one missing CPU offset")
PY
FIXTURE_REALIZATION_FILE="$TMP_DIR/fixture-realization.json"
printf '%s\n' "$FIXTURE_REALIZATION" >"$FIXTURE_REALIZATION_FILE"
log_step "Verified release fixture: realization manifest, digest, identities, and observed evidence"
FIXTURE_STATUS_REALIZED="$(curl -fsS "$ENDPOINT/_mock/fixture/status")"
FIXTURE_MANIFEST="$(curl -fsS "$ENDPOINT/_mock/fixture/manifest")"
FIXTURE_IDENTITIES="$(curl -fsS "$ENDPOINT/_mock/fixture/identities")"
CLI_FIXTURE_STATUS_REALIZED="$("$BIN" --database-url "sqlite:$TMP_DB" fixture status)"
CLI_FIXTURE_MANIFEST="$("$BIN" --database-url "sqlite:$TMP_DB" fixture manifest)"
CLI_FIXTURE_IDENTITIES="$("$BIN" --database-url "sqlite:$TMP_DB" fixture identities)"
if [[ "$CLI_FIXTURE_STATUS_REALIZED" != "$FIXTURE_STATUS_REALIZED" \
  || "$CLI_FIXTURE_MANIFEST" != "$FIXTURE_MANIFEST" \
  || "$CLI_FIXTURE_IDENTITIES" != "$FIXTURE_IDENTITIES" ]]; then
  echo "fixture persisted document CLI/HTTP bytes differ" >&2
  exit 1
fi
printf '%s\n' "$FIXTURE_DEFINITION" >"$TMP_DIR/fixture-definition.json"
printf '%s\n' "$FIXTURE_MANIFEST" >"$TMP_DIR/fixture-manifest.json"
printf '%s\n' "$FIXTURE_IDENTITIES" >"$TMP_DIR/fixture-identities.json"
python3 "$ROOT_DIR/scripts/validate_release_fixture.py" \
  --definition "$TMP_DIR/fixture-definition.json" \
  --manifest "$TMP_DIR/fixture-manifest.json" \
  --negative
log_step "Verified release fixture: persisted CLI/HTTP parity and executable Draft 2020-12 schema policy"

MUTATION_STATUS="$(curl -fsS "$ENDPOINT/_mock/fixture/mutation/status")"
CLI_MUTATION_STATUS="$("$BIN" --database-url "sqlite:$TMP_DB" fixture mutation-status)"
if [[ "$CLI_MUTATION_STATUS" != "$MUTATION_STATUS" ]]; then
  echo "mutation status CLI/HTTP bytes differ" >&2
  exit 1
fi
AUTHORITY="$(FIXTURE_REALIZATION="$FIXTURE_REALIZATION" python3 -c '
import json, os
m = json.loads(os.environ["FIXTURE_REALIZATION"])["manifest"]
t = next(item for item in m["mutation_resources"] if item["target_kind"] == "stop")
print(json.dumps({
  "version": "release-qualification-v1",
  "generation": m["generation"],
  "manifest_digest": m["digest"],
  "mutation_generation": m["mutation_generation"],
  "mutation_generation_id": m["mutation_generation_id"],
  "control_id": t["control_id"],
  "target_id": t["resource_id"],
  "scope": "target",
  "fault_kind": "stop",
  "application_time": "2026-08-05T00:00:00Z",
}))
')"
MUTATION_TARGET_ROWS="$(FIXTURE_REALIZATION="$FIXTURE_REALIZATION" python3 - <<'PY'
import json
import os

manifest = json.loads(os.environ["FIXTURE_REALIZATION"])["manifest"]
for target in manifest["mutation_resources"]:
    print("\t".join((target["resource_id"], target["target_kind"], target["initial_state"], target["initial_type"])))
PY
)"
while IFS=$'\t' read -r mutation_id target_kind expected_state expected_type; do
  [[ -z "$mutation_id" ]] && continue
  PUBLIC_INSTANCE="$(localstack_json ec2 describe-instances --instance-ids "$mutation_id")"
  PUBLIC_INSTANCE="$PUBLIC_INSTANCE" MUTATION_ID="$mutation_id" EXPECTED_STATE="$expected_state" EXPECTED_TYPE="$expected_type" python3 - <<'PY'
import json
import os

data = json.loads(os.environ["PUBLIC_INSTANCE"])
reservations = data.get("Reservations", [])
instances = [instance for reservation in reservations for instance in reservation.get("Instances", [])]
if len(instances) != 1 or instances[0].get("InstanceId") != os.environ["MUTATION_ID"]:
    raise SystemExit(f"public EC2 did not return exact mutation identity {os.environ['MUTATION_ID']}")
instance = instances[0]
state = instance.get("State", {}).get("Name")
instance_type = instance.get("InstanceType")
if state != os.environ["EXPECTED_STATE"] or instance_type != os.environ["EXPECTED_TYPE"]:
    raise SystemExit(f"unexpected initial public state/type for {os.environ['MUTATION_ID']}: {state}:{instance_type}")
PY
done <<<"$MUTATION_TARGET_ROWS"
OLD_MUTATION_ARNS="$(FIXTURE_REALIZATION="$FIXTURE_REALIZATION" python3 - <<'PY'
import json
import os

for target in json.loads(os.environ["FIXTURE_REALIZATION"])["manifest"]["mutation_resources"]:
    print(target["aws_identity"])
PY
)"
FAULT_RECEIPT="$(curl -fsS -X POST "$ENDPOINT/_mock/fixture/fault" -H 'content-type: application/json' -d "$AUTHORITY")"
STOP_TARGET_ID="$(echo "$AUTHORITY" | jq -r .target_id)"
FAULT_PUBLIC="$(localstack_json ec2 describe-instances --instance-ids "$STOP_TARGET_ID")"
FAULT_PUBLIC="$FAULT_PUBLIC" python3 - <<'PY'
import json
import os

instances = [instance for reservation in json.loads(os.environ["FAULT_PUBLIC"]).get("Reservations", []) for instance in reservation.get("Instances", [])]
if len(instances) != 1 or instances[0].get("State", {}).get("Name") != "stopped":
    raise SystemExit("public EC2 did not show the stop fault")
PY
RESET_REQUEST="$(FAULT_RECEIPT="$FAULT_RECEIPT" AUTHORITY="$AUTHORITY" python3 -c '
import json, os
a = json.loads(os.environ["AUTHORITY"])
r = json.loads(os.environ["FAULT_RECEIPT"])
out = {key: a[key] for key in ("version", "generation", "manifest_digest", "mutation_generation", "mutation_generation_id")}
out.update(receipt_id=r["receipt_id"], reset_token=r["reset_token"])
print(json.dumps(out))
')"
RESET_RECEIPT="$(curl -fsS -X POST "$ENDPOINT/_mock/fixture/reset" -H 'content-type: application/json' -d "$RESET_REQUEST")"
if [[ "$(echo "$RESET_RECEIPT" | jq -r .status)" != "RESET" ]]; then
  echo "fixture reset receipt did not report RESET" >&2
  exit 1
fi
RESET_PUBLIC="$(localstack_json ec2 describe-instances --instance-ids "$STOP_TARGET_ID")"
RESET_PUBLIC="$RESET_PUBLIC" python3 - <<'PY'
import json
import os

instances = [instance for reservation in json.loads(os.environ["RESET_PUBLIC"]).get("Reservations", []) for instance in reservation.get("Instances", [])]
if len(instances) != 1 or instances[0].get("State", {}).get("Name") != "running":
    raise SystemExit("public EC2 did not show the restored stop target")
PY
RECREATE_AUTHORITY="$(echo "$AUTHORITY" | jq -c 'del(.control_id, .target_id, .scope, .fault_kind, .application_time)')"
RECREATE_RECEIPT="$(curl -fsS -X POST "$ENDPOINT/_mock/fixture/recreate" -H 'content-type: application/json' -d "$RECREATE_AUTHORITY")"
if [[ "$(echo "$RECREATE_RECEIPT" | jq -r .status)" != "RECREATED" ]]; then
  echo "fixture recreate receipt did not report RECREATED" >&2
  exit 1
fi
NEW_AUTHORITY="$(curl -fsS "$ENDPOINT/_mock/fixture/manifest" | jq -c '{version:"release-qualification-v1", generation, manifest_digest:.digest, mutation_generation, mutation_generation_id}')"
NEW_MUTATION_ARNS="$(curl -fsS "$ENDPOINT/_mock/fixture/manifest" | python3 -c '
import json, sys
for target in json.load(sys.stdin)["mutation_resources"]:
    print(target["aws_identity"])
')"
DESTROY_RECEIPT="$(curl -fsS -X POST "$ENDPOINT/_mock/fixture/destroy" -H 'content-type: application/json' -d "$NEW_AUTHORITY")"
if [[ "$(echo "$DESTROY_RECEIPT" | jq -r '.public_inventory_absence.all_absent')" != "true" ]]; then
  echo "fixture destroy did not prove public identity absence" >&2
  exit 1
fi
while IFS= read -r old_arn; do
  [[ -z "$old_arn" ]] && continue
  if [[ "$(aws_json resourcegroupstaggingapi get-resources --resource-arn-list "$old_arn" | jq '.ResourceTagMappingList | length')" != "0" ]]; then
    echo "destroyed mutation identity remains visible in Foxtail public inventory: $old_arn" >&2
    exit 1
  fi
  old_id="${old_arn##*/}"
  if localstack_json ec2 describe-instances --instance-ids "$old_id" >/dev/null 2>&1; then
    echo "destroyed mutation identity remains visible in LocalStack EC2: $old_id" >&2
    exit 1
  fi
done <<<"$OLD_MUTATION_ARNS
$NEW_MUTATION_ARNS"
log_step "Verified qualification mutation lifecycle: status, fault, reset, recreate, destroy, and public absence"

FIXTURE_IDS="$(FIXTURE_REALIZATION="$FIXTURE_REALIZATION" python3 - <<'PY'
import json
import os

payload = json.loads(os.environ["FIXTURE_REALIZATION"])
for resource in payload["manifest"]["resources"]:
    print(f'{resource["resource_id"]}\t{resource["control_id"]}')
PY
)"
FIXTURE_ARNS="$(FIXTURE_REALIZATION="$FIXTURE_REALIZATION" python3 - <<'PY'
import json
import os

payload = json.loads(os.environ["FIXTURE_REALIZATION"])
for resource in payload["manifest"]["resources"]:
    print(f'{resource["resource_id"]}\t{resource["aws_identity"]}')
PY
)"

while IFS=$'\t' read -r fixture_id fixture_arn; do
  [[ -z "$fixture_id" ]] && continue
  TAGGED_FIXTURE="$(aws_json resourcegroupstaggingapi get-resources \
    --resource-arn-list "$fixture_arn")"
  TAGGED_FIXTURE="$TAGGED_FIXTURE" FIXTURE_ID="$fixture_id" FIXTURE_ARN="$fixture_arn" python3 - <<'PY'
import json
import os

fixture_id = os.environ["FIXTURE_ID"]
fixture_arn = os.environ["FIXTURE_ARN"]
mappings = json.loads(os.environ["TAGGED_FIXTURE"]).get("ResourceTagMappingList", [])
if len(mappings) != 1 or mappings[0].get("ResourceARN") != fixture_arn:
    raise SystemExit(f"tagging inventory did not return exact identity {fixture_id}")
tags = {
    tag.get("Key"): tag.get("Value")
    for tag in mappings[0].get("Tags", [])
}
if tags.get("Name") != fixture_id:
    raise SystemExit(f"tagging inventory returned the wrong Name tag for {fixture_id}")
PY
done <<<"$FIXTURE_ARNS"
log_step "Verified release fixture: identity- and tag-matched Resource Groups inventory"

while IFS=$'\t' read -r fixture_id control_id; do
  [[ -z "$fixture_id" ]] && continue
  METRICS_OUTPUT="$(aws_json cloudwatch list-metrics \
    --namespace AWS/EC2 \
    --metric-name CPUUtilization \
    --dimensions "Name=InstanceId,Value=$fixture_id")"
  METRICS_OUTPUT="$METRICS_OUTPUT" FIXTURE_ID="$fixture_id" python3 - <<'PY'
import json
import os

fixture_id = os.environ["FIXTURE_ID"]
metrics = json.loads(os.environ["METRICS_OUTPUT"]).get("Metrics", [])
if not any(
    metric.get("Namespace") == "AWS/EC2"
    and metric.get("MetricName") == "CPUUtilization"
    and any(
        dimension.get("Name") == "InstanceId"
        and dimension.get("Value") == fixture_id
        for dimension in metric.get("Dimensions", [])
    )
    for metric in metrics
):
    raise SystemExit(f"CloudWatch list-metrics did not return identity {fixture_id}")
PY

  CPU_HISTORY="$(aws_json cloudwatch get-metric-statistics \
    --namespace AWS/EC2 \
    --metric-name CPUUtilization \
    --statistics Average \
    --period 3600 \
    --start-time "$FIXTURE_CW_START_TIME" \
    --end-time "$FIXTURE_CW_END_TIME" \
    --dimensions "Name=InstanceId,Value=$fixture_id")"
  CPU_HISTORY="$CPU_HISTORY" CONTROL_ID="$control_id" python3 - <<'PY'
import json
import os

control_id = os.environ["CONTROL_ID"]
datapoints = json.loads(os.environ["CPU_HISTORY"]).get("Datapoints", [])
expected = 13 if control_id == "ec2-idle-degraded-001" else 14
if len(datapoints) != expected:
    raise SystemExit(
        f"{control_id} expected {expected} CPU history points, found {len(datapoints)}"
    )
if not all(point.get("Average", 0) > 0 for point in datapoints):
    raise SystemExit(f"{control_id} returned a non-positive CPU history point")
PY
done <<<"$FIXTURE_IDS"
log_step "Verified release fixture: identity-matched CloudWatch metrics and scoped history gaps"

FIXTURE_COSTS="$(aws_json ce get-cost-and-usage \
  --time-period "Start=$FIXTURE_CE_START_DAY,End=$FIXTURE_CE_END_DAY" \
  --granularity DAILY \
  --metrics UnblendedCost \
  --group-by Type=DIMENSION,Key=RESOURCE_ID)"
FIXTURE_COSTS_FILE="$TMP_DIR/fixture-costs.json"
printf '%s\n' "$FIXTURE_COSTS" >"$FIXTURE_COSTS_FILE"
python3 - "$FIXTURE_COSTS_FILE" "$FIXTURE_REALIZATION_FILE" <<'PY'
import json
import sys

with open(sys.argv[2], encoding="utf-8") as handle:
    manifest = json.load(handle)["manifest"]
ids = {resource["resource_id"] for resource in manifest["resources"]}
observed = {resource_id: 0.0 for resource_id in ids}
with open(sys.argv[1], encoding="utf-8") as handle:
    costs = json.load(handle)
for bucket in costs.get("ResultsByTime", []):
    for group in bucket.get("Groups", []):
        keys = group.get("Keys", [])
        if len(keys) != 1 or keys[0] not in observed:
            continue
        amount = float(group.get("Metrics", {}).get("UnblendedCost", {}).get("Amount", 0))
        observed[keys[0]] += amount
if any(amount <= 0 for amount in observed.values()):
    raise SystemExit(f"missing positive resource cost evidence: {observed}")
PY
log_step "Verified release fixture: identity-matched Cost Explorer resource groups"

FIXTURE_RECOMMENDATIONS="$(aws_json compute-optimizer get-ec2-instance-recommendations)"
FIXTURE_RECOMMENDATIONS_FILE="$TMP_DIR/fixture-recommendations.json"
printf '%s\n' "$FIXTURE_RECOMMENDATIONS" >"$FIXTURE_RECOMMENDATIONS_FILE"
python3 - "$FIXTURE_RECOMMENDATIONS_FILE" "$FIXTURE_REALIZATION_FILE" <<'PY'
import json
import sys

with open(sys.argv[2], encoding="utf-8") as handle:
    manifest = json.load(handle)["manifest"]
expected = {
    resource["resource_id"]: (
        "OVER_PROVISIONED"
        if resource["control_id"] in {
            "ec2-idle-positive-001",
            "ec2-idle-degraded-001",
            "ec2-resize-positive-001",
        }
        else "OPTIMIZED"
        if resource["control_id"] == "ec2-resize-negative-001"
        else "UNDER_PROVISIONED"
    )
    for resource in manifest["resources"]
}
with open(sys.argv[1], encoding="utf-8") as handle:
    recommendations_payload = json.load(handle)
recommendations = {}
for recommendation in recommendations_payload.get("instanceRecommendations", []):
    arn = recommendation.get("instanceArn", "")
    resource_id = arn.rsplit("/", 1)[-1]
    if resource_id in expected:
        recommendations[resource_id] = recommendation.get("finding")
if set(recommendations) != set(expected):
    raise SystemExit(f"missing Compute Optimizer identities: {set(expected) - set(recommendations)}")
for resource_id, finding in expected.items():
    if recommendations[resource_id] != finding:
        raise SystemExit(
            f"{resource_id} expected Compute Optimizer finding {finding}, got {recommendations[resource_id]}"
        )
PY
log_step "Verified release fixture: identity-matched Compute Optimizer findings"

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

CPU_STANDARD_STATS_OUTPUT="$(aws_json cloudwatch get-metric-statistics \
  --namespace AWS/EC2 \
  --metric-name CPUUtilization \
  --statistics SampleCount Average Sum Minimum Maximum \
  --period 3600 \
  --start-time "$CW_START_TIME" \
  --end-time "$CW_END_TIME" \
  --dimensions Name=InstanceId,Value="$INSTANCE_ID")"

CPU_STANDARD_STATS_OUTPUT="$CPU_STANDARD_STATS_OUTPUT" python3 - <<'PY'
import json
import os

payload = json.loads(os.environ["CPU_STANDARD_STATS_OUTPUT"])
datapoints = payload.get("Datapoints", [])
if not datapoints:
    raise SystemExit("CPUUtilization multi-stat get-metric-statistics returned no datapoints")

required_fields = {"SampleCount", "Average", "Sum", "Minimum", "Maximum"}
if not any(required_fields.issubset(point.keys()) for point in datapoints):
    raise SystemExit("CPUUtilization multi-stat get-metric-statistics omitted one or more standard statistics")

if not any(point.get("SampleCount", 0) >= 1 for point in datapoints):
    raise SystemExit("CPUUtilization multi-stat get-metric-statistics returned invalid SampleCount values")
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
