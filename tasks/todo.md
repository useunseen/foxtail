# Review Follow-Up Plan

- [x] Re-scope review follow-up to exclude admin/dashboard auth hardening for this local-only mock service.
- [x] Refactor dashboard/resource/trend handlers so they do not all compute the full dashboard payload.
- [x] Surface dashboard database failures explicitly instead of returning empty success responses.
- [x] Improve read-path scalability with tighter query bounds and time-first indexes where current query shapes need them.
- [x] Replace hardcoded tested/coverage claims with honest scorecard values derived from what is actually verified.
- [x] Add focused Rust tests for CloudWatch pagination, dashboard/status routes, and scorecard behavior.
- [x] Run `cargo fmt`, `cargo test`, and `cargo clippy --all-targets --all-features`, then capture results here.

## Review

- User clarified on 2026-03-11 that admin and dashboard auth is intentionally out of scope because this is a local mock service.
- Remaining work is limited to performance, reliability, and verification gaps from the review.

## Results

- Added route-specific dashboard builders so `/_mock/dashboard/resources`, `/_mock/dashboard/trends/cloudwatch`, and `/_mock/dashboard/trends/cost` no longer all build the full dashboard payload.
- Dashboard data endpoints now return a 500 JSON error instead of silently converting database failures into empty success payloads.
- Added a new migration for time-first dashboard/cost indexes and bounded `GetMetricData` query fan-out plus SQL-backed pagination for `GetDimensionValues`.
- Coverage scorecard now reports untested status honestly instead of claiming perfect verification.
- Added 7 Rust tests covering status, dashboard scorecard/error handling, Cost Explorer pagination, CloudWatch JSON pagination, CloudWatch XML query handling, and scenario mutation.
- Verification commands completed successfully:
  - `cargo fmt`
  - `cargo test`
  - `cargo clippy --all-targets --all-features`

## Current Planning Focus

- [x] Normalize Cost Explorer target aliasing so CLI-emitted `AWSInsightsIndexService.*` requests reach all existing handlers.
- [x] Restore CLI-usable grouped `GetCostAndUsage` output for seeded dimensions.
- [x] Add CloudWatch `ListMetrics` so CLI-only users can discover metrics without `/_mock/*`.
- [x] Add route tests plus a reproducible AWS CLI smoke check for the public surface.
- [x] Implement the details captured in `docs/plans/2026-03-11-fix-aws-cli-api-interoperability-plan.md`.

## CLI Interoperability Results

- Cost Explorer now accepts both `AWSCostExplorer.*` and `AWSInsightsIndexService.*` targets for the implemented operations.
- `GetCostAndUsage` now returns populated groups for supported dimensions, which restores CLI-usable service breakdowns.
- CloudWatch Query/XML now supports `ListMetrics`, enabling public metric discovery before `get-metric-data` and `get-metric-statistics`.
- Added `scripts/verify_cli_interop.sh` and `make verify-cli-interoperability` for repeatable AWS CLI smoke verification.
- Verification completed successfully:
  - `cargo fmt`
  - `cargo test`
  - `cargo clippy --all-targets --all-features`
  - `bash scripts/verify_cli_interop.sh`

## Next Planning Focus

- [x] Clean up `cloudwatch get-metric-data` response quality so public CloudWatch output is query-scoped, period-aware, and deterministic.
- [x] Replace raw-row output with basic period/stat aggregation and stronger result-shape guarantees.
- [x] Extend route tests and CLI smoke checks to assert `GetMetricData` output quality, not just callability.
- [x] Implement the details captured in `docs/plans/2026-03-11-fix-cloudwatch-getmetricdata-response-quality-plan.md`.

## GetMetricData Results

- Added typed `GetMetricData` parsing plus shared aggregation logic across the JSON target and the Query/XML path used by the AWS CLI.
- `cloudwatch get-metric-data` now preserves the caller query id, buckets timestamps cleanly by period, and returns aligned timestamp/value arrays.
- Pagination now stays deterministic across multiple query series and no longer errors when a shorter series is exhausted before a longer one.
- Added route tests for JSON aggregation, Query/XML aggregation, and later-page handling for shorter result sets.
- Strengthened `scripts/verify_cli_interop.sh` so the smoke run asserts `GetMetricData` shape quality instead of only checking for a 200.
- Added `README.md` covering the supported make targets, binary subcommands, public AWS-compatible commands, and local `/_mock/*` helper routes.
- Verification completed successfully:
  - `cargo fmt`
  - `cargo test`
  - `cargo clippy --all-targets --all-features`
  - `bash scripts/verify_cli_interop.sh`
  - manual `aws cloudwatch get-metric-data` check on `127.0.0.1:8080`

## Pending Todo Triage

- Closed todo `055` in this repo after re-running the repo-local `GetMetricData` pagination tests and AWS CLI smoke verification.
- Triaged todo `056` as external to this extracted service repo because the referenced runtime files do not exist in this checkout.
- No remaining repo-local implementation todo is queued in `tasks/todo.md`; the remaining open items are external runtime work or intentionally out of scope for this local-only service.

## Scenario Data Quality Verification

- [x] Compare generated metric and cost outputs across `baseline`, `spike`, and `idle-heavy` on an isolated temp database.
- [x] Validate that public AWS-compatible commands reflect the expected scenario-specific behavior for cost and utilization.
- [x] Record whether the observed datapoints match the generator’s intended shapes and identify any quality gaps.

## Scenario Verification Results

- Ran a three-scenario sweep (`Baseline`, `Spike`, `IdleHeavy`) against an isolated copied database behind a temporary local server on `127.0.0.1:18081`.
- Verified the expected directional behavior from the generator:
  - `Spike` materially increases EC2/RDS CPU, EC2 network throughput, ELB request/error rates, and 30-day cost totals.
  - `IdleHeavy` materially lowers utilization metrics while raising cost totals, which matches the intended “expensive but idle” FinOps scenario.
  - `Baseline` stays in the middle for utilization and spend, with modest error events and moderate throughput.
- Observed aggregate values:
  - `Baseline`: EC2 CPU `17.06`, EC2 NetworkIn `20.97M`, ELB RequestCount `535.18`, total raw cost `429.88`
  - `Spike`: EC2 CPU `76.28`, EC2 NetworkIn `72.21M`, ELB RequestCount `2000.78`, total raw cost `2032.91`
  - `IdleHeavy`: EC2 CPU `3.79`, EC2 NetworkIn `4.32M`, ELB RequestCount `54.66`, total raw cost `4376.77`
- Public AWS-compatible checks matched the same directional story:
  - `ce get-cost-and-usage`
  - `ce get-cost-and-usage --group-by Type=DIMENSION,Key=SERVICE`
  - `cloudwatch list-metrics`
  - `cloudwatch get-metric-statistics`
- The main quality caveat is semantic realism rather than correctness:
  - `IdleHeavy` uses a flat high-cost multiplier across all resource types, so the scenario is useful for FinOps “high spend, low usage” exercises but less realistic as a production-like cost shape.
  - Cost Explorer totals over a requested time window are lower than the full raw `cost_records` sum because the API query window is narrower/end-exclusive relative to the entire seeded table, which is expected.

## Next FinOps API Work

- [x] Add `ce get-cost-and-usage-with-resources` with resource-level groups for AWS CLI-driven FinOps analysis.
- [x] Add `ce get-tags` backed by `resources.tags` so cost allocation metadata is discoverable from the public CE surface.
- [x] Add focused route tests and extend `scripts/verify_cli_interop.sh` to exercise both new operations.
- [x] Re-run formatting, tests, clippy, and the CLI smoke suite after the new handlers land.

## FinOps API Results

- Added `GetCostAndUsageWithResources` on the Cost Explorer JSON path, defaulting to `RESOURCE_ID` grouping so the AWS CLI can retrieve resource-level cost slices.
- Added `GetTags` backed by distinct values from `resources.tags`, with search and pagination support.
- Added first-pass Cost Explorer filter support for `SERVICE`, `RESOURCE_ID`, `REGION`, and exact tag-key/tag-value matching, which is enough to drive the new resource-level CE flow from the AWS CLI.
- Extended route coverage with tests for:
  - resource-level `GetCostAndUsageWithResources`
  - paginated `GetTags`
- Extended `scripts/verify_cli_interop.sh` to exercise:
  - `ce get-cost-and-usage-with-resources --filter ...`
  - `ce get-tags --tag-key Name`
- Verification completed successfully:
  - `cargo fmt`
  - `cargo test`
  - `cargo clippy --all-targets --all-features`
  - `bash scripts/verify_cli_interop.sh`
