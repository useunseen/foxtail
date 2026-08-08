#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${BIN:-$ROOT_DIR/target/debug/foxtail}"
SOURCE_DB="${MOCK_DATA_DB:-$ROOT_DIR/mock_data.db}"
PORT="${AWS_MOCK_VERIFY_PORT:-18080}"
LOCALSTACK_ENDPOINT="${AWS_ENDPOINT_URL:-http://127.0.0.1:4666}"
MANIFEST_ACCOUNT_ID="${FOXTAIL_MANIFEST_ACCOUNT_ID:-123456789012}"
MANIFEST_ACCESS_KEY_ID="${FOXTAIL_MANIFEST_ACCESS_KEY_ID:-$MANIFEST_ACCOUNT_ID}"
MANIFEST_SECRET_ACCESS_KEY="${FOXTAIL_MANIFEST_SECRET_ACCESS_KEY:-${MANIFEST_ACCOUNT_ID}-secret}"
TEST_ACCESS_KEY_ID="${FOXTAIL_TEST_ACCESS_KEY_ID:-test}"
TEST_SECRET_ACCESS_KEY="${FOXTAIL_TEST_SECRET_ACCESS_KEY:-test}"
FOXTAIL_ACCESS_KEY_ID="${FOXTAIL_OBSERVATION_ACCESS_KEY_ID:-test}"
FOXTAIL_SECRET_ACCESS_KEY="${FOXTAIL_OBSERVATION_SECRET_ACCESS_KEY:-test}"
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
if ! AWS_ACCESS_KEY_ID="$MANIFEST_ACCESS_KEY_ID" \
  AWS_SECRET_ACCESS_KEY="$MANIFEST_SECRET_ACCESS_KEY" \
  AWS_SESSION_TOKEN="" AWS_SECURITY_TOKEN="" \
  AWS_DEFAULT_REGION=us-east-1 AWS_PAGER="" \
  aws --output json --endpoint-url "$LOCALSTACK_ENDPOINT" ec2 describe-images \
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

aws_for_account() {
  local access_key_id="$1"
  local secret_access_key="$2"
  shift 2
  AWS_ACCESS_KEY_ID="$access_key_id" \
    AWS_SECRET_ACCESS_KEY="$secret_access_key" \
    AWS_SESSION_TOKEN="" \
    AWS_SECURITY_TOKEN="" \
    AWS_DEFAULT_REGION=us-east-1 \
    AWS_PAGER="" \
    aws --output json "$@"
}

aws_json() {
  # Foxtail's read-only observation surface is intentionally separate from
  # the mutation LocalStack endpoint.
  aws_for_account "$FOXTAIL_ACCESS_KEY_ID" "$FOXTAIL_SECRET_ACCESS_KEY" \
    --endpoint-url "$ENDPOINT" "$@"
}

mutation_manifest_json() {
  aws_for_account "$MANIFEST_ACCESS_KEY_ID" "$MANIFEST_SECRET_ACCESS_KEY" \
    --endpoint-url "$LOCALSTACK_ENDPOINT" "$@"
}

mutation_test_json() {
  aws_for_account "$TEST_ACCESS_KEY_ID" "$TEST_SECRET_ACCESS_KEY" \
    --endpoint-url "$LOCALSTACK_ENDPOINT" "$@"
}

validate_mutation_document() {
  local kind="$1"
  local document="$2"
  local path="$TMP_DIR/mutation-${kind}-$RANDOM.json"
  printf '%s\n' "$document" >"$path"
  if [[ "$kind" == "status" ]]; then
    python3 "$ROOT_DIR/scripts/validate_release_fixture.py" --mutation-status "$path"
  else
    python3 "$ROOT_DIR/scripts/validate_release_fixture.py" --receipt "$path"
  fi
}

python3 "$ROOT_DIR/scripts/validate_release_fixture.py" --negative

MANIFEST_EMPTY_BEFORE="$(mutation_manifest_json ec2 describe-instances)"
TEST_EMPTY_BEFORE="$(mutation_test_json ec2 describe-instances)"
MANIFEST_EMPTY_BEFORE="$MANIFEST_EMPTY_BEFORE" TEST_EMPTY_BEFORE="$TEST_EMPTY_BEFORE" python3 - <<'PY'
import json
import os

for label in ("MANIFEST_EMPTY_BEFORE", "TEST_EMPTY_BEFORE"):
    payload = json.loads(os.environ[label])
    instances = [
        instance
        for reservation in payload.get("Reservations", [])
        for instance in reservation.get("Instances", [])
    ]
    if instances:
        raise SystemExit(f"fresh mutation account {label} was not empty before realization")
PY
log_step "Verified fresh mutation LocalStack is empty for manifest and default-test accounts"

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
FIXTURE_REALIZATION="$(curl -fsS -X POST "$ENDPOINT/_mock/fixture/realize" \
  -H 'content-type: application/json' \
  -d "{\"version\":\"release-qualification-v1\",\"clock_anchor\":\"$FIXTURE_ANCHOR\"}")"
# Keep one database and one isolated generation for all subsequent reads.
# Calling fixture realize again is intentionally rejected once this
# authority-bound generation exists; CLI/HTTP parity is checked through the
# supported persisted status/manifest/identity surfaces below.
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
REALIZED_MANIFEST_ACCOUNT_ID="$(FIXTURE_REALIZATION="$FIXTURE_REALIZATION" python3 -c '
import json, os
print(json.loads(os.environ["FIXTURE_REALIZATION"])["manifest"]["environment"]["account_id"])
')"
if [[ "$REALIZED_MANIFEST_ACCOUNT_ID" != "$MANIFEST_ACCOUNT_ID" ]]; then
  echo "fixture manifest account $REALIZED_MANIFEST_ACCOUNT_ID differs from configured mutation account $MANIFEST_ACCOUNT_ID" >&2
  exit 1
fi
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

FOXTAIL_EC2_OBSERVATION="$(aws_json ec2 describe-instances)"
FOXTAIL_EC2_OBSERVATION="$FOXTAIL_EC2_OBSERVATION" FIXTURE_MANIFEST="$FIXTURE_MANIFEST" python3 - <<'PY'
import json
import os

manifest = json.loads(os.environ["FIXTURE_MANIFEST"])
expected = {
    resource["resource_id"]: resource
    for resource in manifest["resources"]
}
payload = json.loads(os.environ["FOXTAIL_EC2_OBSERVATION"])
instances = [
    instance
    for reservation in payload.get("Reservations", [])
    for instance in reservation.get("Instances", [])
]
if len(instances) != 5 or {instance.get("InstanceId") for instance in instances} != set(expected):
    raise SystemExit("Foxtail EC2 observation did not return exactly the five manifest resources")
for instance in instances:
    resource = expected[instance["InstanceId"]]
    observed = resource["observed"]
    if instance.get("State", {}).get("Name") != observed["instance_state"]:
        raise SystemExit("Foxtail EC2 observation returned an unexpected instance state")
    if instance.get("InstanceType") != observed["instance_type"]:
        raise SystemExit("Foxtail EC2 observation returned an unexpected instance type")
    if instance.get("Placement", {}).get("AvailabilityZone") != observed["availability_zone"]:
        raise SystemExit("Foxtail EC2 observation returned an unexpected availability zone")
    tags = {tag.get("Key"): tag.get("Value") for tag in instance.get("Tags", [])}
    if tags != observed["tags"]:
        raise SystemExit("Foxtail EC2 observation returned tags different from the manifest")
PY
log_step "Verified Foxtail AWS-compatible EC2 observation: exactly five manifest rows and stable metadata"

MUTATION_STATUS="$(curl -fsS "$ENDPOINT/_mock/fixture/mutation/status")"
validate_mutation_document status "$MUTATION_STATUS"
CLI_MUTATION_STATUS="$("$BIN" --database-url "sqlite:$TMP_DB" fixture mutation-status)"
if [[ "$CLI_MUTATION_STATUS" != "$MUTATION_STATUS" ]]; then
  echo "mutation status CLI/HTTP bytes differ" >&2
  exit 1
fi
AUTHORITY="$(FIXTURE_REALIZATION="$FIXTURE_REALIZATION" python3 -c '
import json, os
m = json.loads(os.environ["FIXTURE_REALIZATION"])["manifest"]
print(json.dumps({
  "version": "release-qualification-v1",
  "generation": m["generation"],
  "manifest_digest": m["digest"],
  "mutation_generation": m["mutation_generation"],
  "mutation_generation_id": m["mutation_generation_id"],
}))
')"
AUTH_VERSION="$(echo "$AUTHORITY" | jq -r .version)"
AUTH_GENERATION="$(echo "$AUTHORITY" | jq -r .generation)"
AUTH_MANIFEST_DIGEST="$(echo "$AUTHORITY" | jq -r .manifest_digest)"
AUTH_MUTATION_GENERATION="$(echo "$AUTHORITY" | jq -r .mutation_generation)"
AUTH_MUTATION_GENERATION_ID="$(echo "$AUTHORITY" | jq -r .mutation_generation_id)"
MUTATION_TARGET_ROWS="$(FIXTURE_REALIZATION="$FIXTURE_REALIZATION" python3 - <<'PY'
import json
import os

manifest = json.loads(os.environ["FIXTURE_REALIZATION"])["manifest"]
for target in manifest["mutation_resources"]:
    print("\t".join((
        target["resource_id"],
        target["target_kind"],
        target["setup_fault_kind"],
        target["control_id"],
        target["initial_state"],
        target["initial_type"],
        target["terminal_state"],
        target["terminal_type"],
    )))
PY
)"
MANIFEST_MUTATION_AFTER="$(mutation_manifest_json ec2 describe-instances)"
TEST_MUTATION_AFTER="$(mutation_test_json ec2 describe-instances)"
MANIFEST_MUTATION_AFTER="$MANIFEST_MUTATION_AFTER" TEST_MUTATION_AFTER="$TEST_MUTATION_AFTER" FIXTURE_REALIZATION="$FIXTURE_REALIZATION" python3 - <<'PY'
import json
import os

manifest = json.loads(os.environ["FIXTURE_REALIZATION"])["manifest"]
expected = {target["resource_id"] for target in manifest["mutation_resources"]}
if len(expected) != 4:
    raise SystemExit(f"fixture manifest did not declare exactly four mutation IDs: {expected}")

manifest_payload = json.loads(os.environ["MANIFEST_MUTATION_AFTER"])
manifest_instances = [
    instance
    for reservation in manifest_payload.get("Reservations", [])
    for instance in reservation.get("Instances", [])
]
manifest_ids = {instance.get("InstanceId") for instance in manifest_instances}
if manifest_ids != expected:
    raise SystemExit(
        f"manifest account did not expose exactly the four mutation IDs: {manifest_ids} != {expected}"
    )

test_payload = json.loads(os.environ["TEST_MUTATION_AFTER"])
test_instances = [
    instance
    for reservation in test_payload.get("Reservations", [])
    for instance in reservation.get("Instances", [])
]
if test_instances:
    raise SystemExit("default test account unexpectedly exposed mutation instances")
PY
log_step "Verified manifest account owns exactly four mutation IDs and default test account owns none"

ACTIVE_TAGGED_INVENTORY="$(aws_json resourcegroupstaggingapi get-resources \
  --resource-type-filters ec2:instance)"
ACTIVE_TAGGED_INVENTORY="$ACTIVE_TAGGED_INVENTORY" FIXTURE_REALIZATION="$FIXTURE_REALIZATION" python3 - <<'PY'
import json
import os

manifest = json.loads(os.environ["FIXTURE_REALIZATION"])["manifest"]
expected = {resource["resource_id"] for resource in manifest["resources"]}
observed = {
    mapping["ResourceARN"].rsplit("/", 1)[-1]
    for mapping in json.loads(os.environ["ACTIVE_TAGGED_INVENTORY"])["ResourceTagMappingList"]
}
if observed != expected:
    raise SystemExit(
        f"active Resource Groups inventory did not contain exactly the five read-only IDs: {observed} != {expected}"
    )
PY
log_step "Verified active Resource Groups inventory excludes qualification mutation targets"

while IFS=$'\t' read -r mutation_id target_kind setup_fault_kind control_id expected_state expected_type terminal_state terminal_type; do
  [[ -z "$mutation_id" ]] && continue
  PUBLIC_INSTANCE="$(mutation_manifest_json ec2 describe-instances --instance-ids "$mutation_id")"
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
SCENARIO_INDEX=0
while IFS=$'\t' read -r mutation_id target_kind setup_fault_kind control_id expected_state expected_type terminal_state terminal_type; do
  [[ -z "$mutation_id" ]] && continue
  SCENARIO_INDEX=$((SCENARIO_INDEX + 1))
  FAULT_REQUEST="$(AUTHORITY="$AUTHORITY" CONTROL_ID="$control_id" TARGET_ID="$mutation_id" FAULT_KIND="$setup_fault_kind" python3 -c '
import json, os
value = json.loads(os.environ["AUTHORITY"])
value.update(control_id=os.environ["CONTROL_ID"], target_id=os.environ["TARGET_ID"], scope="target", fault_kind=os.environ["FAULT_KIND"], application_time="2026-08-05T00:00:00Z")
print(json.dumps(value))
')"
  if [[ "$SCENARIO_INDEX" == "1" || "$SCENARIO_INDEX" == "3" ]]; then
    FAULT_RECEIPT="$("$BIN" --database-url "sqlite:$TMP_DB" fixture fault \
      --version "$AUTH_VERSION" --generation "$AUTH_GENERATION" \
      --manifest-digest "$AUTH_MANIFEST_DIGEST" \
      --mutation-generation "$AUTH_MUTATION_GENERATION" \
      --mutation-generation-id "$AUTH_MUTATION_GENERATION_ID" \
      --control-id "$control_id" --target-id "$mutation_id" --scope target \
      --fault-kind "$setup_fault_kind" --application-time "2026-08-05T00:00:00Z")"
    RESET_CHANNEL="http"
  else
    FAULT_RECEIPT="$(curl -fsS -X POST "$ENDPOINT/_mock/fixture/fault" \
      -H 'content-type: application/json' -d "$FAULT_REQUEST")"
    RESET_CHANNEL="cli"
  fi
  if [[ "$(echo "$FAULT_RECEIPT" | jq -r .status)" != "APPLIED" ]]; then
    echo "fixture fault did not report APPLIED for $target_kind" >&2
    exit 1
  fi
  validate_mutation_document receipt "$FAULT_RECEIPT"
  FAULT_PUBLIC="$(mutation_manifest_json ec2 describe-instances --instance-ids "$mutation_id")"
  FAULT_PUBLIC="$FAULT_PUBLIC" MUTATION_ID="$mutation_id" EXPECTED_STATE="$terminal_state" EXPECTED_TYPE="$terminal_type" python3 - <<'PY'
import json
import os
instances = [instance for reservation in json.loads(os.environ["FAULT_PUBLIC"]).get("Reservations", []) for instance in reservation.get("Instances", [])]
if len(instances) != 1 or instances[0].get("InstanceId") != os.environ["MUTATION_ID"]:
    raise SystemExit("public EC2 fault identity mismatch")
if instances[0].get("State", {}).get("Name") != os.environ["EXPECTED_STATE"] or instances[0].get("InstanceType") != os.environ["EXPECTED_TYPE"]:
    raise SystemExit("public EC2 did not show the expected fault state/type")
PY
  RESET_REQUEST="$(FAULT_RECEIPT="$FAULT_RECEIPT" AUTHORITY="$AUTHORITY" python3 -c '
import json, os
a = json.loads(os.environ["AUTHORITY"])
r = json.loads(os.environ["FAULT_RECEIPT"])
a.update(receipt_id=r["receipt_id"], reset_token=r["reset_token"])
print(json.dumps(a))
')"
  if [[ "$RESET_CHANNEL" == "http" ]]; then
    RESET_RECEIPT="$(curl -fsS -X POST "$ENDPOINT/_mock/fixture/reset" \
      -H 'content-type: application/json' -d "$RESET_REQUEST")"
  else
    RESET_RECEIPT="$("$BIN" --database-url "sqlite:$TMP_DB" fixture reset \
      --version "$AUTH_VERSION" --generation "$AUTH_GENERATION" \
      --manifest-digest "$AUTH_MANIFEST_DIGEST" \
      --mutation-generation "$AUTH_MUTATION_GENERATION" \
      --mutation-generation-id "$AUTH_MUTATION_GENERATION_ID" \
      --receipt-id "$(echo "$FAULT_RECEIPT" | jq -r .receipt_id)" \
      --reset-token "$(echo "$FAULT_RECEIPT" | jq -r .reset_token)")"
  fi
  if [[ "$(echo "$RESET_RECEIPT" | jq -r .status)" != "RESET" ]]; then
    echo "fixture reset receipt did not report RESET for $target_kind" >&2
    exit 1
  fi
  validate_mutation_document receipt "$RESET_RECEIPT"
  RESET_PUBLIC="$(mutation_manifest_json ec2 describe-instances --instance-ids "$mutation_id")"
  RESET_PUBLIC="$RESET_PUBLIC" MUTATION_ID="$mutation_id" EXPECTED_STATE="$expected_state" EXPECTED_TYPE="$expected_type" python3 - <<'PY'
import json
import os
instances = [instance for reservation in json.loads(os.environ["RESET_PUBLIC"]).get("Reservations", []) for instance in reservation.get("Instances", [])]
if len(instances) != 1 or instances[0].get("InstanceId") != os.environ["MUTATION_ID"]:
    raise SystemExit("public EC2 reset identity mismatch")
if instances[0].get("State", {}).get("Name") != os.environ["EXPECTED_STATE"] or instances[0].get("InstanceType") != os.environ["EXPECTED_TYPE"]:
    raise SystemExit("public EC2 did not show the expected restored state/type")
PY
done <<<"$MUTATION_TARGET_ROWS"
if "$BIN" --database-url "sqlite:$TMP_DB" fixture fault \
  --version "$AUTH_VERSION" --generation "$((AUTH_GENERATION + 1))" \
  --manifest-digest "$AUTH_MANIFEST_DIGEST" \
  --mutation-generation "$AUTH_MUTATION_GENERATION" \
  --mutation-generation-id "$AUTH_MUTATION_GENERATION_ID" \
  --control-id "ec2-mutation-stop-001" \
  --target-id "i-do-not-have-authority" --scope target --fault-kind stop >/dev/null 2>&1; then
  echo "stale CLI authority unexpectedly succeeded" >&2
  exit 1
fi
RECREATE_RECEIPT="$("$BIN" --database-url "sqlite:$TMP_DB" fixture recreate \
  --version "$AUTH_VERSION" --generation "$AUTH_GENERATION" \
  --manifest-digest "$AUTH_MANIFEST_DIGEST" \
  --mutation-generation "$AUTH_MUTATION_GENERATION" \
  --mutation-generation-id "$AUTH_MUTATION_GENERATION_ID")"
if [[ "$(echo "$RECREATE_RECEIPT" | jq -r .status)" != "RECREATED" ]]; then
  echo "fixture recreate receipt did not report RECREATED" >&2
  exit 1
fi
validate_mutation_document receipt "$RECREATE_RECEIPT"
NEW_AUTHORITY="$(curl -fsS "$ENDPOINT/_mock/fixture/manifest" | jq -c '{version:"release-qualification-v1", generation, manifest_digest:.digest, mutation_generation, mutation_generation_id}')"
NEW_MUTATION_ARNS="$(curl -fsS "$ENDPOINT/_mock/fixture/manifest" | python3 -c '
import json, sys
for target in json.load(sys.stdin)["mutation_resources"]:
    print(target["aws_identity"])
')"
DESTROY_RECEIPT="$("$BIN" --database-url "sqlite:$TMP_DB" fixture destroy \
  --version "$(echo "$NEW_AUTHORITY" | jq -r .version)" \
  --generation "$(echo "$NEW_AUTHORITY" | jq -r .generation)" \
  --manifest-digest "$(echo "$NEW_AUTHORITY" | jq -r .manifest_digest)" \
  --mutation-generation "$(echo "$NEW_AUTHORITY" | jq -r .mutation_generation)" \
  --mutation-generation-id "$(echo "$NEW_AUTHORITY" | jq -r .mutation_generation_id)")"
if [[ "$(echo "$DESTROY_RECEIPT" | jq -r '.public_inventory_absence.all_absent')" != "true" ]]; then
  echo "fixture destroy did not prove public identity absence" >&2
  exit 1
fi
validate_mutation_document receipt "$DESTROY_RECEIPT"
while IFS= read -r old_arn; do
  [[ -z "$old_arn" ]] && continue
  if [[ "$(aws_json resourcegroupstaggingapi get-resources --resource-arn-list "$old_arn" | jq '.ResourceTagMappingList | length')" != "0" ]]; then
    echo "destroyed mutation identity remains visible in Foxtail public inventory: $old_arn" >&2
    exit 1
  fi
  old_id="${old_arn##*/}"
  old_public_file="$TMP_DIR/old-ec2-$old_id.json"
  old_error_file="$TMP_DIR/old-ec2-$old_id.err"
  set +e
  mutation_manifest_json ec2 describe-instances --instance-ids "$old_id" >"$old_public_file" 2>"$old_error_file"
  old_describe_status=$?
  set -e
  if [[ "$old_describe_status" -eq 0 ]]; then
    if ! OLD_ID="$old_id" python3 - "$old_public_file" <<'PY'
import json
import os
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    payload = json.load(handle)
instances = [
    instance
    for reservation in payload.get("Reservations", [])
    for instance in reservation.get("Instances", [])
]
if len(instances) != 1 or instances[0].get("InstanceId") != os.environ["OLD_ID"]:
    raise SystemExit("successful EC2 DescribeInstances response did not contain exactly the requested identity")
if instances[0].get("State", {}).get("Name") != "terminated":
    raise SystemExit("successful EC2 DescribeInstances response did not prove terminated state")
PY
    then
      echo "destroyed mutation identity was not terminal in LocalStack EC2: $old_id" >&2
      exit 1
    fi
  else
    if [[ -s "$old_public_file" ]] || ! python3 - "$old_error_file" <<'PY'
import re
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    error = handle.read()
if not re.search(
    r"An error occurred \(InvalidInstanceID\.NotFound\) when calling the DescribeInstances operation:",
    error,
):
    raise SystemExit("EC2 DescribeInstances failed without the explicit InvalidInstanceID.NotFound service error")
PY
    then
      echo "LocalStack EC2 absence check failed for $old_id" >&2
      cat "$old_error_file" >&2
      exit 1
    fi
  fi
done <<<"$OLD_MUTATION_ARNS
$NEW_MUTATION_ARNS"
log_step "Verified qualification mutation lifecycle: status, fault, reset, recreate, destroy, EC2 terminal cleanup, and public inventory absence"

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
