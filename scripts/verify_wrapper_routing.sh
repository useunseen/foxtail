#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${BIN:-$ROOT_DIR/target/debug/foxtail}"
TMP_DIR="$(mktemp -d)"
AWS_LOG="$TMP_DIR/aws.log"
AWSLOCAL_LOG="$TMP_DIR/awslocal.log"
AWS_BIN="$TMP_DIR/aws"
AWSLOCAL_BIN="$TMP_DIR/awslocal"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

cat >"$AWS_BIN" <<EOF
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "\$0" "\$@" > "$AWS_LOG"
EOF

cat >"$AWSLOCAL_BIN" <<EOF
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "\$0" "\$@" > "$AWSLOCAL_LOG"
EOF

chmod +x "$AWS_BIN" "$AWSLOCAL_BIN"

cd "$ROOT_DIR"
cargo build >/dev/null

echo "[verify-wrapper] checking routed FinOps command"
"$BIN" --aws-bin "$AWS_BIN" --awslocal-bin "$AWSLOCAL_BIN" ce get-cost-and-usage >/dev/null

if [[ ! -f "$AWS_LOG" ]]; then
  echo "expected routed command to invoke aws" >&2
  exit 1
fi

if ! grep -q -- "--endpoint-url" "$AWS_LOG"; then
  echo "expected routed command to inject --endpoint-url" >&2
  exit 1
fi

if ! grep -q -- "http://127.0.0.1:8080" "$AWS_LOG"; then
  echo "expected routed command to use Foxtail endpoint" >&2
  exit 1
fi

rm -f "$AWS_LOG" "$AWSLOCAL_LOG"

echo "[verify-wrapper] checking EC2 observation routing"
"$BIN" --aws-bin "$AWS_BIN" --awslocal-bin "$AWSLOCAL_BIN" ec2 describe-instances >/dev/null

if [[ ! -f "$AWS_LOG" ]] || [[ -f "$AWSLOCAL_LOG" ]]; then
  echo "expected EC2 DescribeInstances to invoke aws only" >&2
  exit 1
fi

if ! grep -q -- "ec2" "$AWS_LOG" || ! grep -q -- "describe-instances" "$AWS_LOG"; then
  echo "expected EC2 routing to preserve service and operation tokens" >&2
  exit 1
fi

rm -f "$AWS_LOG" "$AWSLOCAL_LOG"

echo "[verify-wrapper] checking passthrough command"
"$BIN" --aws-bin "$AWS_BIN" --awslocal-bin "$AWSLOCAL_BIN" s3 ls >/dev/null

if [[ -f "$AWS_LOG" ]]; then
  echo "expected passthrough command to avoid aws" >&2
  exit 1
fi

if [[ ! -f "$AWSLOCAL_LOG" ]]; then
  echo "expected passthrough command to invoke awslocal" >&2
  exit 1
fi

if ! grep -q -- "s3" "$AWSLOCAL_LOG"; then
  echo "expected awslocal invocation to preserve service token" >&2
  exit 1
fi

if ! grep -q -- "ls" "$AWSLOCAL_LOG"; then
  echo "expected awslocal invocation to preserve operation token" >&2
  exit 1
fi

echo "[verify-wrapper] wrapper routing looks correct"
