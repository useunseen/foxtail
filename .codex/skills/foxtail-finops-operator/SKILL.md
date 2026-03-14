---
name: foxtail-finops-operator
description: Operate the Foxtail AWS mock data service for FinOps workflows. Use when you need to build, seed, run, or verify the local mock service, switch between baseline/spike/idle-heavy scenarios, author a new scenario, or analyze the mock estate through public AWS CLI calls against http://127.0.0.1:8080.
---

# Foxtail FinOps Operator

## Quick Start

Use this skill when the task is to work with this repo as a local AWS-like FinOps environment.

Default bootstrap:

```bash
make setup
make serve
```

AWS CLI env:

```bash
export AWS_ACCESS_KEY_ID=test
export AWS_SECRET_ACCESS_KEY=test
export AWS_DEFAULT_REGION=us-east-1
export AWS_PAGER=""
```

Then use:

```bash
aws --endpoint-url http://127.0.0.1:8080 ...
```

## Operating Rules

1. Prefer the public AWS-compatible CLI surface for analysis work.
2. Use local `/_mock/*` helper routes only for service diagnostics or scenario mutation.
3. Existing built-in scenarios are `baseline`, `spike`, and `idle-heavy`.
4. If the user wants a new scenario, change the generator and CLI, not just the docs.
5. After behavioral changes, run `bash scripts/verify_cli_interop.sh`.

## Core Workflows

### Build, Seed, Run

- `make build`
- `make gen-baseline`
- `make gen-spike`
- `make gen-idle-heavy`
- `make serve`
- `make verify-cli-interoperability`

### Scenario Control

Use regeneration when you want a clean DB from discovery:

```bash
make gen-baseline
make gen-spike
make gen-idle-heavy
```

Use the local helper route when you want to mutate the current seeded DB in place:

```bash
curl -sS -X POST http://127.0.0.1:8080/_mock/scenario \
  -H 'content-type: application/json' \
  -d '{"scenario":"Spike"}'
```

Optional per-resource mutation:

```bash
curl -sS -X POST http://127.0.0.1:8080/_mock/scenario \
  -H 'content-type: application/json' \
  -d '{"scenario":"IdleHeavy","resource_id":"i-20652c71bedc57ced"}'
```

### FinOps Analysis

Start with:

- `ce get-dimension-values`
- `ce get-cost-and-usage`
- `ce get-cost-and-usage-with-resources`
- `ce get-tags`
- `resourcegroupstaggingapi get-resources`
- `pricing get-products`
- `compute-optimizer get-ec2-instance-recommendations`
- `compute-optimizer get-ebs-volume-recommendations`
- `cloudwatch list-metrics`
- `cloudwatch get-metric-statistics`

For exact commands and scenario-authoring steps, see [playbooks.md](playbooks.md).

## New Scenario Authoring

When adding a new scenario, update all of these together:

- [src/cli.rs](/Users/murphy/workspace/iacai0/foxtail/src/cli.rs)
- [src/generator.rs](/Users/murphy/workspace/iacai0/foxtail/src/generator.rs)
- [Makefile](/Users/murphy/workspace/iacai0/foxtail/Makefile)
- [README.md](/Users/murphy/workspace/iacai0/foxtail/README.md)
- tests and smoke checks if public behavior changes

## Success Criteria

- The service is running locally.
- The intended scenario is active.
- Public AWS CLI commands return usable data for the user’s workflow.
- Verification is rerun after any service behavior change.
