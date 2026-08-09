# Issue #5: Disposable Mutation Generations

Pinned base: `b524be1d738d12e816b584ea0238113545118cf8`

Invariant: qualification mutations can affect only fresh, manifest-bound, one-use EC2 targets inside an explicitly isolated qualification environment; every transition is auditable, stale or ambiguous authority fails closed, and destruction is complete only after public inventory proves old identities absent and no injected fault remains.

## Acceptance Map

- [x] Distinct targets: fixture lifecycle/persistence owns separate stop, resize, stop-recovery, and resize-restoration targets; manifest/status and public inventory consume their identities; existing read-only controls remain compatible; prove four unique mutation identities with no reuse or read-only overlap.
- [x] Recreation identity: isolated recreation provisions a new mutation-generation identity, new public EC2 IDs, regenerated manifest digest, and retires the old public identities while preserving definition/read-only identity.
- [x] Supported controls: CLI parsing/execution and HTTP routes expose status, fault, reset, recreate, and destroy through the same fixture domain operations.
- [x] Fault receipts: fault application binds exact manifest digest, mutation control, target/scope, setup fault kind, application time, and one-use reset token; public EC2 state/type is reconciled before commit.
- [x] Reset/cleanup receipts: reset and destruction enumerate exact faults/targets acted on plus their prior and terminal state; targeted reset is guarded by the recorded terminal state.
- [x] Isolation gate: every mutating fixture operation requires an explicit isolated-qualification environment signal in addition to existing HTTP admin authentication; ordinary realization remains declared-only and read-only.
- [x] Destruction proof: cleanup removes generation-owned resources and evidence, resets every active fault, records exact EC2 termination evidence, proves old identities are absent from Foxtail's public inventory, and records the absence receipt.
- [x] Fail closed: duplicate, stale-generation, wrong-manifest, wrong-control/target, malformed, unknown-field, and ambiguous external operations are rejected without private-state repair or scope expansion.
- [x] Full proof: focused Rust unit/integration tests cover the lifecycle and deterministic mock boundary, and the complete LocalStack smoke passes with a valid mutation AMI/endpoint.

## Execution Plan

- [x] Give one Luna worker ownership of the coherent fixture lifecycle slice, schemas/goldens, migration, CLI/HTTP surfaces, documentation, tests, and commits.
- [ ] Inspect the Luna handoff and frozen committed diff against every acceptance criterion.
- [ ] Run frozen-head code review against the pinned base and return validated repair findings to the same Luna worker.
- [ ] Run broad formatting, test, clippy, schema, and interoperability verification once on the repair candidate.
- [ ] Record review evidence, residual gaps, commit IDs, and final acceptance outcome below.

## Issue #5 Review

- Implementation complete; frozen-head review remains with the parent agent.

## Issue #5 Repair Results

- Added a typed canonical mutation catalogue and a single EC2 boundary. Isolated generations use the AWS SDK against the configured endpoint and reconcile returned IDs, states, and instance types; a deterministic `mock://` backend exercises the same lifecycle in tests.
- Ordinary `fixture realize` with an unset or invalid qualification value keeps mutation controls declared-only, writes no mutation ledger, and performs no external dispatch. Mutation operations persist an intent before dispatch and finalize only after public reconciliation and affected-row guards.
- Fault, reset, recreate, and destroy now dispatch through the external boundary. Recreate records exact `terminated`/service-level not-found evidence for every prior target before retiring its ledger; destroy resets active faults, records the same external evidence, and proves public Resource Groups inventory absence.
- Verification completed with `cargo fmt --all -- --check`, `cargo test -q` (71 unit, 2 API-contract, 14 mutation integration, 3 wrapper tests), `cargo clippy --all-targets --all-features -- -D warnings`, the pinned Draft 2020-12 validator, `bash -n scripts/verify_cli_interop.sh`, and `git diff --check`. Real LocalStack/EC2 smoke remains pending a live endpoint and valid `FOXTAIL_MUTATION_AMI_ID`.

## Issue #5 Second Frozen Repair Results

- [x] Serialize all lifecycle operations per generation with a partial unique index and fail-closed validation; reject every subsequent isolated `realize` until authority-bound `recreate` is used.
- [x] Record every launched public target before setup, include the current and earlier IDs in cleanup/quarantine, compensate post-provision persistence failures, and add deterministic setup/cleanup/database failure-injection tests.
- [x] Quarantine pre-boundary `UNKNOWN`/default-endpoint generations during migration and require an active generation with exactly four canonical, non-retired, externally verified targets before dispatch.
- [x] Reconcile destroy/reset against externally observed state, accept only exact `terminated` or service-level not-found as irreversible EC2 cleanup, classify not-found errors through the SDK error chain, and keep public-inventory absence separate from EC2 termination evidence.
- [x] Make the CLI smoke use one database/generation, alternate CLI/HTTP fault/reset channels across all four targets, recreate and destroy both generations, verify EC2 terminal cleanup plus Tagging absence, and validate every mutation status/receipt with executable Draft 2020-12 schemas.
- Verification completed with `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test -q` (71 unit, 2 API-contract, 14 mutation integration, 3 wrapper tests), `python3 scripts/validate_release_fixture.py --negative`, `bash -n scripts/verify_cli_interop.sh`, and `git diff --check`. AWS `TerminateInstances` may keep terminated instances visible in `DescribeInstances` for approximately one hour, so EC2 termination and Foxtail public-inventory absence are recorded as separate proofs; see the [official API documentation](https://docs.aws.amazon.com/AWSEC2/latest/APIReference/API_TerminateInstances.html).

## Issue #5 Final Live Repair Results

- [x] Make the deterministic failed-realize test inject a pre-dispatch EC2 failure through the mock boundary instead of depending on an unavailable localhost port.
- [x] Poll only the EC2 state while stopping a running resize target, retain strict final state/type reconciliation, and send the documented `InstanceType` modify field without the duplicate `Attribute` field that makes LocalStack clear the type.
- Verification completed with `cargo fmt --all`, the focused pre-dispatch, four-scenario mock, and mutation lifecycle tests, and the complete LocalStack 4.14.0 smoke using `AWS_ENDPOINT_URL=http://127.0.0.1:4566`, `FOXTAIL_MUTATION_AMI_ID=ami-760aaa0f`, test credentials, and `us-east-1`. The run passed realization, all four fault/reset cycles, recreate, destroy, exact `terminated`/not-found EC2 cleanup checks, zero Foxtail Resource Groups Tagging mappings for retired ARNs, and every downstream CLI interoperability check.
- [x] Full proof: complete live smoke passed with EC2 termination evidence and separate Foxtail public-inventory absence proof.

## Issue #5 Standards Repair Results

- [x] Require prior-generation EC2 termination evidence keyed by each exact target identity, with a truthful four-identity public-inventory absence count in the receipt schema; add negative coverage for missing proof, omitted targets, contradictory duplicate IDs, and contradictory counts.
- [x] Make the LocalStack EC2 terminal check fail closed on transport, authentication, malformed JSON, wrong identity/state, and non-`InvalidInstanceID.NotFound` service errors; add backend coverage for running/stopped/error observations.
- Deterministic verification completed with `cargo test -q` (72 unit, 2 API-contract, 19 mutation integration, 3 wrapper tests), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, `python3 scripts/validate_release_fixture.py --negative`, `bash -n scripts/verify_cli_interop.sh`, and `git diff --check`. Parent-verified fresh LocalStack 4.14.0 smoke using `AWS_ENDPOINT_URL=http://127.0.0.1:4566`, `FOXTAIL_MUTATION_AMI_ID=ami-760aaa0f`, test credentials, and `us-east-1` passed the tightened EC2 terminal/not-found parser, recreate/destroy lifecycle, zero retired Resource Groups Tagging mappings, and all downstream CLI checks.

## Issue #5 Implementation Results

- Added a migration-backed disposable mutation ledger with four generation-owned EC2 targets (stop, resize, stop-recovery, and resize-restoration), manifest/status/identity exposure, complete-estate fingerprinting, and canonical persisted operation receipts.
- Added isolated-environment and existing admin-token gates for mutation status, fault, reset, recreate, and destroy; authority binds version, fixture generation, mutation generation/id, and exact manifest digest, with one-use reset tokens and fail-closed stale/duplicate/unknown-field handling.
- Added CLI and HTTP lifecycle surfaces, recreation identity replacement, destruction cleanup with public-inventory absence proof, updated schemas/goldens/docs, focused lifecycle and parser tests, and smoke-script coverage. The smoke script keeps large AWS responses in temporary files to avoid shell argument-size limits.
- Verification completed successfully with `cargo fmt --all -- --check`, `cargo test -q` (71 unit, 2 API-contract, 3 wrapper tests), `cargo clippy --all-targets --all-features -- -D warnings`, `python3 scripts/validate_release_fixture.py --definition tests/fixtures/release-qualification-v1.definition.json --manifest tests/fixtures/release-qualification-v1.manifest.json --negative`, `bash -n scripts/verify_cli_interop.sh`, `git diff --check`, and `bash scripts/verify_cli_interop.sh` (localhost access required and granted).

# Cost Explorer USAGE_TYPE Grouping Work

# Release Qualification Fixture v1

- [x] Add canonical fixture definition/manifest model, serializer, SHA-256 digest boundary, schemas, and checked-in goldens.
- [x] Add atomic persisted realization state derived from the current EC2 estate, with deterministic five-control catalogue and public evidence declarations.
- [x] Add shared CLI and HTTP definition/realize/status/manifest/identities flows with fail-closed version/input handling and byte/digest parity.
- [x] Add focused unit/integration/schema/golden tests plus AWS CLI observation smoke coverage; update README and script docs.
- [x] Run formatting, full tests, clippy, and feasible interoperability checks; record review evidence and residual environment gaps.

## Release Qualification Fixture v1 Results

- The v1 definition and manifest are canonical compact UTF-8 JSON with recursively sorted object keys and SHA-256 digests that exclude only the document's own top-level digest.
- Realization persists exact definition/manifest bytes in a singleton SQLite row, binds the manifest to the active definition digest and generation, and publishes five deterministic EC2 control identities with materialized public evidence plus two deferred mutation declarations.
- The canonical `fixture` CLI command and `/_mock/fixture/*` routes return the same bytes for definition, realization, status, manifest, and identities; unknown versions, malformed JSON, and unknown request fields fail closed.

## Release Qualification Fixture v1 Repair Results

- [x] Materialize five deterministic EC2 controls from empty rows, reload the actual metric/cost rows, validate positive/negative/degraded/resize predicates, and fingerprint the observed evidence.
- [x] Protect fixture realization with the same admin-token guard as the scenario mutation route; remove speculative CLI and HTTP aliases.
- [x] Make schema validation executable with pinned `scripts/requirements.txt` (`jsonschema==4.26.0`), Draft 2020-12 validation, recursive forbidden-policy checks, negative checks, and Rust generated-manifest golden parity.
- [x] Make the CLI smoke mandatory: seed five EC2 rows in its isolated migrated copy and assert identity-matched CloudWatch history, Cost Explorer resource groups, and Compute Optimizer findings without a skip path.
- Verification completed with `cargo fmt --all -- --check`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `bash -n scripts/verify_cli_interop.sh`, the pinned Draft 2020-12 validator, and the mandatory isolated AWS CLI smoke. A live LocalStack deployment is not available in this checkout; that remains an environment gap separate from the local proof.

## Release Qualification Fixture v1 Second Frozen Review Results

- [x] Publish implementation-owned assignment, materialization profiles, history offsets, network formulas, CPU predicates, and required resource rules through `generation_rules`; regenerate definition/manifest goldens and digest-sensitive tests.
- [x] Resolve fixture account scope from the same authoritative public account identity; reject explicit mismatches before any materialization transaction and verify default/explicit matching parity against public Compute Optimizer identities.
- [x] Extend the mandatory CLI smoke to query Resource Groups Tagging for every realized manifest ARN and assert exact ARN identity plus the deterministic `Name` tag.
- Verification completed with `cargo fmt --all -- --check`, `cargo test` (69 tests plus integration suites), `cargo clippy --all-targets --all-features -- -D warnings`, the pinned Draft 2020-12 validator, `bash -n scripts/verify_cli_interop.sh`, `git diff --check`, and the mandatory isolated AWS CLI smoke. The smoke passed exact Resource Groups Tagging ARN/Name checks for all five realized controls. A live LocalStack deployment remains an environment gap.

- [x] Diagnose why AWS CLI `ce get-cost-and-usage --group-by Type=DIMENSION,Key=USAGE_TYPE` fails validation.
- [x] Add `USAGE_TYPE` as a supported Cost Explorer grouping dimension.
- [x] Return AWS-style mock usage type keys and `UsageQuantity` metrics for grouped cost responses.
- [x] Add route tests for EC2 Compute and Elastic Load Balancing service-filtered usage type grouping.
- [x] Run `cargo fmt`, focused Cost Explorer tests, `cargo test`, and `cargo clippy --all-targets --all-features`.

## Cost Explorer USAGE_TYPE Results

- `ce get-cost-and-usage` now accepts `GroupBy` key `USAGE_TYPE`.
- Grouped Cost Explorer responses now include `UsageQuantity` when requested, using the same mock usage-rate mapping as usage forecasts.
- Service-filtered EC2 Compute requests return `USE1-BoxUsage:m6i.xlarge`; service-filtered Elastic Load Balancing requests return `USE1-LoadBalancerUsage`.
- Verification completed successfully:
  - `cargo fmt`
  - `cargo test cost_explorer_group_by_usage_type`
  - `cargo test cost_explorer_group_by_service_returns_populated_groups`
  - `cargo test`
  - `cargo clippy --all-targets --all-features`
  - `bash scripts/verify_cli_interop.sh`
  - Targeted AWS CLI `USAGE_TYPE` checks for EC2 Compute and Elastic Load Balancing

# CloudWatch JSON And ElastiCache Metrics Work

- [x] Add CloudWatch JSON target support for `GraniteServiceVersion20100801.ListMetrics`.
- [x] Add CloudWatch JSON target support for `GraniteServiceVersion20100801.GetMetricStatistics`.
- [x] Add ElastiCache metric modeling for `AWS/ElastiCache` `CPUUtilization` and `CurrConnections`.
- [x] Add ElastiCache cluster discovery during generation.
- [x] Update CLI smoke coverage and docs for current AWS CLI CloudWatch JSON query-mode behavior.
- [x] Run `cargo fmt`, targeted tests, `cargo test`, and CLI smoke verification where feasible.

## CloudWatch JSON And ElastiCache Metrics Results

- CloudWatch JSON now supports `ListMetrics` and `GetMetricStatistics`, including the epoch-second timestamp shape emitted by AWS CLI v2.34 JSON query mode.
- `GetMetricStatistics` JSON reuses the same period aggregation and standard-stat validation as the Query/XML path.
- ElastiCache clusters discovered during generation now expose `AWS/ElastiCache` `CPUUtilization` and `CurrConnections` under the `CacheClusterId` dimension after scenario regeneration.
- Verification completed successfully:
  - `cargo fmt`
  - `cargo test cloudwatch_json`
  - `cargo test scenario_endpoint_generates_elasticache_metrics`
  - `cargo clippy --all-targets --all-features`
  - `cargo test`
  - `bash scripts/verify_cli_interop.sh` with elevated localhost access

# Database Reset Make Target

- [x] Rename `make reset-db` to `make reset`.
- [x] Change reset behavior to delete `mock_data.db` without reseeding.
- [x] Update user-facing command docs to include the renamed cleanup-only target.

## Database Reset Rename Results

- Replaced `make reset-db` with `make reset`.
- `make reset` now only removes `mock_data.db`; reseeding remains explicit via `make gen` or scenario-specific generation targets.
- Updated the `Makefile` help text and the README make-target table so the cleanup-only reset flow is discoverable.
- Verification pending below.

- [x] Add a `make` target that resets the generated SQLite database.
- [x] Update user-facing command docs to include the new target.
- [x] Record the result here.

## Database Reset Results

- Historical note: the earlier `make reset-db` target removed `mock_data.db` and reseeded baseline data.
- That command has now been replaced by cleanup-only `make reset`.
- Runtime verification was not needed for this documentation-and-build-target change.

# Review Follow-Up Plan

## Current Execution Focus

- [x] Add typed Query/XML parsing for `GetMetricStatistics` standard statistics and validation rules.
- [x] Rework CloudWatch metric-statistics aggregation so one period bucket can emit `SampleCount`, `Average`, `Sum`, `Minimum`, and `Maximum`.
- [x] Update the XML response model plus route tests, CLI smoke checks, and README notes for the expanded standard-statistics support.
- [x] Run `cargo fmt`, `cargo test`, and `cargo clippy --all-targets --all-features`, then record results here.

## Current Execution Results

- `GetMetricStatistics` now parses `Statistics.member.N`, rejects mixed or unsupported stat inputs, and returns the full standard statistics set on the Query/XML path: `SampleCount`, `Average`, `Sum`, `Minimum`, and `Maximum`.
- CloudWatch period bucketing now computes shared aggregate bucket data once and reuses it for both `GetMetricStatistics` and the existing `GetMetricData` stat selection path.
- XML datapoints now emit only the requested statistic fields, which restores CLI-visible `Maximum` for `CPUUtilization` and adds explicit `SampleCount`, `Sum`, and `Minimum` support.
- Added route coverage for standard-stat aggregation and validation failures, extended the CLI smoke script with multi-stat `CPUUtilization` assertions, and updated the README CloudWatch contract notes.
- Verification completed successfully:
  - `cargo fmt`
  - `cargo test`
  - `cargo clippy --all-targets --all-features`
  - `bash scripts/verify_cli_interop.sh`

- [x] Re-scope review follow-up to exclude admin/dashboard auth hardening for this local-only mock service.
- [x] Refactor dashboard/resource/trend handlers so they do not all compute the full dashboard payload.
- [x] Surface dashboard database failures explicitly instead of returning empty success responses.
- [x] Improve read-path scalability with tighter query bounds and time-first indexes where current query shapes need them.
- [x] Replace hardcoded tested/coverage claims with honest scorecard values derived from what is actually verified.
- [x] Add focused Rust tests for CloudWatch pagination, dashboard/status routes, and scorecard behavior.
- [x] Run `cargo fmt`, `cargo test`, and `cargo clippy --all-targets --all-features`, then capture results here.

## Blog Post Draft Work

- [x] Ground the post in the current README, CLI interoperability docs, and wrapper workflow already described in the repo.
- [x] Write a narrative blog draft covering why Foxtail exists, how the LocalStack-plus-Foxtail workflow works, and when to use it.
- [x] Save the draft under `docs/` so it can be edited or published from the repo.

## Blog Post Draft Results

- Added a publishable first-pass article at `docs/posts/2026-03-23-why-i-built-foxtail-for-agentic-finops.md`.
- Framed the post around the problem with static mocked metrics and the decision to keep FinOps agents on AWS CLI-compatible workflows from the start.
- Covered the two-part architecture explicitly:
  - the AWS-compatible FinOps mock service
  - the `foxtail` routing wrapper that sends FinOps commands to the mock service and everything else to LocalStack or AWS
- Included practical examples, scenario-driven usage, and clear guidance on when Foxtail is the right tool versus when real AWS validation is still required.

## Blog Post Medium Formatting Work

- [x] Reformat the published draft for Medium-style readability without changing the article wording.
- [x] Use only structural presentation changes such as headings, spacing, and pull-quote treatment.

## Blog Post Medium Formatting Results

- Reformatted `docs/posts/2026-03-23-why-i-built-foxtail-for-agentic-finops.md` for Medium-style scannability while keeping the article text intact.
- Promoted existing transition lines into section headings and turned the key problem statement into a pull quote.

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

# Issue #14: EC2 Observation Oracle Evidence

Pinned base: `fa7675813fa565cf6aa325bcb36e2b9abfc5b759` (`origin/main`, after merged PR #13).

Invariant: Foxtail owns only deterministic fixture facts and AWS-compatible read-only observation responses. Every successful EC2 attribute or instance-type observation is bound to the exact realized fixture identity and persisted catalogue facts; malformed, duplicate, unknown, unsupported, inconsistent, or unsupported-continuation requests fail closed without synthesizing oracle outcomes. Unseen remains the authority for evidence validation, policy/oracle assessment, immutable receipts, and release qualification.

## Acceptance Map

- [x] 1. Attribute success: fixture manifest/data owns a required public termination-protection boolean; EC2 handler and Query dispatcher expose it for realized read-only fixture instances. Consumers are ordinary AWS CLI and Unseen `AwsCompatibleObservationPort`; prove handler/parser/route plus CLI JSON success and disposable smoke.
- [x] 2. Attribute exact binding/shape: standard EC2 Query/XML binds exactly requested `InstanceId` and returns `DisableApiTermination.Value` as a real boolean from fixture facts, with no default synthesis; prove exact-ID and parser/serializer assertions.
- [x] 3. Attribute failures: missing/unknown IDs, unsupported attributes, malformed members/forms, duplicate `Action`/`InstanceId`/`Attribute`, and inconsistent persisted fixture data return deterministic AWS-compatible non-2xx errors while preserving database state; prove focused negatives and fixture-integrity tests.
- [x] 4. Instance-types success: exact current and recommendation target types (`m6i.large`, `t3.medium`, `m6i.xlarge`) work across realized controls; fixture owns an explicit exact catalogue; prove each request independently.
- [x] 5. Type shape: every returned record contains `InstanceType`, `SupportedRootDeviceTypes`, `SupportedVirtualizationTypes`, `ProcessorInfo.SupportedArchitectures`, and `NetworkInfo.EnaSupport` in Query/XML decodable by ordinary AWS CLI; prove field-complete JSON/XML assertions.
- [x] 6. Type exactness/fail closed: facts are fixture-owned per exact type; unknown/unsupported types, duplicate or malformed members/requests, inconsistent catalogue data, invalid pagination inputs/tokens, and fabricated current/target reuse error deterministically. If continuation is unsupported, reject it explicitly and test; otherwise bind/test tokens fully.
- [x] 7. Dispatcher preservation: distinguish `DescribeInstances`, `DescribeInstanceAttribute`, and `DescribeInstanceTypes`, preserving existing read-only, lifecycle, and mutation behavior; prove dispatcher/route regressions.
- [x] 8. Wrapper: route both operations through Foxtail, never the LocalStack mutation endpoint; extend Rust wrapper tests and `scripts/verify_wrapper_routing.sh` as needed.
- [x] 9. Focused tests: cover handler/parser/manifest/serializer/dispatcher success, exact identity, duplicates, malformed/unsupported inputs, inconsistent data, and continuation-token behavior.
- [x] 10. Disposable CLI interop: extend `scripts/verify_cli_interop.sh` for both operations against Foxtail, asserting exact IDs/boolean/type records/all public fields, deterministic negatives, and distinct Foxtail vs LocalStack endpoints while preserving lifecycle/mutation proof.
- [x] 11. Sibling proof: using Unseen read-only commit `f4c5e7802def856fb4d4ec6996cbd616ea16bd95`, run its real `AwsCompatibleObservationPort.observe()` against repaired Foxtail disposable state; confirm no `UnsupportedAction`, missing instance-attribute/target-compatibility evidence, or `unsupported_capability`. Add a reproducible Foxtail-side harness when possible; report exact environment blocker otherwise; do not edit Unseen.
- [x] 12. Broad regression: run format, full Rust tests, clippy with `-D warnings`, pinned fixture validators (including negatives), shell syntax, wrapper proof, diff check, and full disposable CLI interop with required LocalStack settings.
- [x] 13. Contract/source report: update fixture schema/data/goldens/digests/source-owned facts where required and report exact final Foxtail commits/revision; never edit sibling Unseen pins/contracts.

## Execution Plan

- [x] Inspect EC2 fixture schema/manifest, handler, dispatcher, wrapper, scripts, tests, migration, Compute Optimizer target logic, and sibling compatibility code at the pinned revisions.
- [x] Implement one validated Foxtail observation boundary for `DescribeInstanceAttribute` and `DescribeInstanceTypes`, keeping route orchestration in `serve.rs` and fixture truth/schema/goldens in their owners.
- [x] Add focused success/failure/identity/duplicate/integrity/pagination tests and wrapper/CLI smoke assertions; update deterministic fixture artefacts and docs as needed.
- [x] Run focused verification, commit coherent implementation batches without amending, and review the complete diff against every acceptance criterion.
- [x] Run broad verification and sibling live compatibility proof; record exact results, residual gaps, and final commits below.

## Issue #14 Results / Review

The functional implementation is committed as `cbb8bf5e422b0fd1284e9474db6cf9117a034e19`.
A non-amended source-pin/golden follow-up is committed as
`b48695dd20bbefcacd9b08447782a0f90ade3d6a`, updating the checked-in manifest
and Foxtail-owned golden assertion to the functional revision. The checked-in
manifest now owns the
strict `disable_api_termination` boolean for every realized read-only control
and an exact catalogue for `m6i.large`, `t3.medium`, and `m6i.xlarge`; the
handler validates those facts before serving Query/XML observations.

- Focused proof passed: `cargo test handlers::ec2 --lib`, the EC2 oracle
  integration route (including exact IDs, boolean shape, duplicate/unknown/
  unsupported/continuation failures, and persisted digest preservation),
  `bash scripts/verify_wrapper_routing.sh`, and
  `python3 scripts/validate_release_fixture.py --negative`.
- Broad proof passed: `cargo fmt --all -- --check`, full `cargo test` (89
  library, 5 API-contract, 20 mutation integration, 4 wrapper, doc tests),
  `cargo clippy --all-targets --all-features -- -D warnings`, pinned fixture
  validator with negatives, `bash -n` for both scripts, `git diff --check`,
  and `bash scripts/verify_wrapper_routing.sh`.
- Disposable proof passed against LocalStack 4.14.0 at `127.0.0.1:4566` with
  a temporary seed database and fresh test account: the extended
  `scripts/verify_cli_interop.sh` asserted exact attribute booleans, all type
  compatibility fields, deterministic unknown/continuation failures, endpoint
  separation, and the existing mutation lifecycle.
- Sibling proof passed at Unseen commit
  `f4c5e7802def856fb4d4ec6996cbd616ea16bd95`: real
  `AwsCompatibleObservationPort.observe()` against Foxtail `:18082` and a
  distinct fresh LocalStack account on `:4566` returned five controls, five
  boolean attribute responses, and five target-compatibility responses with no
  `UnsupportedAction`, missing evidence, or `unsupported_capability`. The
  fresh LocalStack account was `222222222222` because shared account `123456789012`
  already contained unrelated resources; Foxtail's authoritative manifest
  account remains `123456789012` and was not changed. No Unseen files or
  contracts were edited.
- Final source revision is the functional Foxtail commit
  `cbb8bf5e422b0fd1284e9474db6cf9117a034e19`; final branch head is the
  non-amended source-pin commit `b48695dd20bbefcacd9b08447782a0f90ade3d6a`.

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

## Next Inventory And Pricing API Work

- [x] Add `resourcegroupstaggingapi get-resources` so tagged inventory is available from the public AWS CLI surface.
- [x] Add `pricing get-products` with a minimal mock price catalog for the seeded AWS services.
- [x] Add focused route tests and extend the CLI smoke script for the new inventory and pricing APIs.
- [x] Re-run formatting, tests, clippy, and the CLI smoke suite after the new handlers land.

## Inventory And Pricing API Results

- Added `ResourceGroupsTaggingAPI_20170126.GetResources` with tag-filter, resource-type-filter, ARN-filter, and pagination support over the seeded resources table.
- Added `AWSPriceListService.GetProducts` with a small mock catalog for `AmazonEC2`, `AmazonRDS`, `AmazonS3`, and `AWSELB`, including CLI-compatible `PriceList` string payloads.
- Added route coverage with tests for:
  - tagged resource inventory pagination
  - pricing catalog filtering and output shape
- Extended `scripts/verify_cli_interop.sh` to exercise:
  - `resourcegroupstaggingapi get-resources`
  - `pricing get-products`
- Verification completed successfully:
  - `cargo fmt`
  - `cargo test`
  - `cargo clippy --all-targets --all-features`
  - `bash scripts/verify_cli_interop.sh`

## Next FinOps Discovery API Work

- [x] Add `resourcegroupstaggingapi get-tag-keys` and `get-tag-values` so CLI callers can discover the current tag surface without helper endpoints.
- [x] Add `compute-optimizer get-ec2-instance-recommendations` and `get-ebs-volume-recommendations` backed by seeded utilization data.
- [x] Extend route coverage, dashboard capability metadata, README docs, and `scripts/verify_cli_interop.sh` for the new APIs.
- [x] Re-run formatting, tests, clippy, and the CLI smoke suite after the new handlers land.

## FinOps Discovery API Results

- Added `ResourceGroupsTaggingAPI_20170126.GetTagKeys` and `GetTagValues`, backed by distinct keys and values from `resources.tags`.
- Added `ComputeOptimizerService.GetEC2InstanceRecommendations` and `GetEBSVolumeRecommendations`, derived from seeded EC2 CPU and disk byte metrics so the outputs change with the active scenario data.
- Extended dashboard capability metadata, route coverage, README docs, and `scripts/verify_cli_interop.sh` for the new discovery and recommendation APIs.
- Verification completed successfully:
  - `cargo fmt`
  - `cargo test`
  - `cargo clippy --all-targets --all-features`
  - `bash scripts/verify_cli_interop.sh`

## Next Forecast And CUR API Work

- [x] Add `ce get-usage-forecast` so AWS CLI callers can model usage trend workflows alongside spend forecasts.
- [x] Add `cur describe-report-definitions` for mock CUR discovery from the public AWS CLI surface.
- [x] Extend route coverage, README docs, dashboard capability metadata, and `scripts/verify_cli_interop.sh` for both APIs.
- [x] Re-run formatting, tests, clippy, and the CLI smoke suite after the new handlers land.

## Forecast And CUR API Results

- Added `AWSInsightsIndexService.GetUsageForecast`, with `USAGE_QUANTITY` and `NORMALIZED_USAGE_AMOUNT` derived from seeded cost records using the mock rate catalog as a usage proxy.
- Added `AWSOrigamiServiceGatewayService.DescribeReportDefinitions`, returning a stable mock CUR definition so AWS CLI-driven CUR discovery works against the public surface.
- Extended route coverage, dashboard capability metadata, README docs, and `scripts/verify_cli_interop.sh` for both APIs.
- Verification completed successfully:
  - `cargo fmt`
  - `cargo test`
  - `cargo clippy --all-targets --all-features`
  - `bash scripts/verify_cli_interop.sh`

## Next Pricing Depth Work

- [x] Expand `pricing get-products` with enough SKU variety to support realistic EC2, EBS, RDS, S3, and ELB lookups.
- [x] Add focused tests for richer pricing filters and pagination over the expanded catalog.
- [x] Extend README docs and `scripts/verify_cli_interop.sh` so the new pricing coverage is exercised from the AWS CLI.
- [x] Re-run formatting, tests, clippy, and the CLI smoke suite after the catalog expansion lands.

## Pricing Depth Results

- Expanded the mock pricing catalog with multiple EC2 instance sizes, an EC2 gp3 storage SKU, additional RDS and S3 products, and both ALB and NLB hourly entries.
- Verified that `pricing get-products` now supports realistic filter paths such as `instanceType=m6i.large` and `volumeType=gp3`, and that AmazonEC2 pagination returns `NextToken` when truncated.
- Extended route coverage, README docs, and `scripts/verify_cli_interop.sh` for the richer pricing surface.
- Verification completed successfully:
  - `cargo fmt`
  - `cargo test`
  - `cargo clippy --all-targets --all-features`
  - `bash scripts/verify_cli_interop.sh`

## Foxtail Skill Work

- [x] Create a repo-local skill that knows how to build, seed, run, and analyze this service through both local controls and public AWS CLI commands.
- [x] Add a supporting playbook for existing scenarios, scenario switching, and authoring a new scenario in code.
- [x] Review the skill content against the current README, CLI surface, and scenario implementation so it does not drift from the service.

## Foxtail Skill Results

- Added the repo-local skill at `.codex/skills/foxtail-finops-operator/` so future agent runs can discover and use this service intentionally instead of reconstructing the workflow from scratch.
- Added a compact skill entrypoint plus a deeper playbook covering bootstrap, scenario reseeding, in-place scenario mutation, public AWS CLI FinOps workflows, and new scenario authoring.
- Reviewed the skill content against the current `Makefile`, `src/cli.rs`, `src/generator.rs`, `src/serve.rs`, and `README.md`.
- No runtime verification commands were needed because this step added operator guidance, not service behavior.


## Foxtail Wrapper Work

- [x] Add a standalone `foxtail` binary that routes supported FinOps commands to the Foxtail endpoint and delegates everything else to `awslocal`.
- [x] Add routing and subprocess tests that prove backend selection, endpoint injection, and exit-code passthrough.
- [x] Add a repeatable wrapper verification script and Make target.
- [x] Update README command-surface docs for the new wrapper.
- [x] Run `cargo fmt`, `cargo test`, `cargo clippy --all-targets --all-features`, and the wrapper verification script, then capture results here.

## Foxtail Wrapper Results

- Added a new standalone binary at `src/bin/foxtail.rs`, backed by reusable routing logic in `src/wrapper.rs`.
- The wrapper now sends the repo's supported FinOps commands to Foxtail through `aws --endpoint-url http://127.0.0.1:8080` and delegates everything else to `awslocal`.
- Added unit coverage for wrapper flag parsing, AWS global-flag skipping, explicit endpoint handling, and the routing matrix.
- Added subprocess-backed integration tests in `tests/foxtail_wrapper.rs` to verify backend selection and exit-code passthrough with fake executables.
- Added `scripts/verify_wrapper_routing.sh` and `make verify-wrapper-routing` for repeatable routing verification without requiring live AWS services.
- Updated `README.md` with the new wrapper command surface, wrapper-specific flags, and the routed command inventory.
- Verification completed successfully:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features`
  - `bash scripts/verify_wrapper_routing.sh`

## AWS CLI Network Metric Query Work

- [x] Reproduce the reported all-zero network in/out behavior against the public CloudWatch AWS CLI path.
- [x] Replace single-query `MetricDataQueries.member.1` parsing on the Query/XML path with multi-query normalization.
- [x] Add focused regression coverage for `NetworkIn` and `NetworkOut` through Query/XML `GetMetricData`.
- [x] Extend `scripts/verify_cli_interop.sh` with an explicit network ingress/egress smoke check.
- [x] Run `cargo fmt`, `cargo test`, `cargo clippy --all-targets --all-features`, and `bash scripts/verify_cli_interop.sh`, then capture results here.

## AWS CLI Network Metric Query Results

- Confirmed the public CloudWatch Query/XML `GetMetricData` path only parsed `MetricDataQueries.member.1.*`, which explained why multi-series network checks could collapse to incomplete or misleading output.
- Replaced the fixed single-query XML parsing path with indexed `MetricDataQueries.member.N` normalization and reused the existing metric-data aggregation path.
- Added route coverage for a two-query XML request that asks for EC2 `NetworkIn` and `NetworkOut` together and verifies both ids and non-zero datapoints.
- Updated `scripts/verify_cli_interop.sh` so it now:
  - asserts `cloudwatch get-metric-statistics` returns non-zero `NetworkIn` datapoints
  - asserts `cloudwatch get-metric-data` returns both `NetworkIn` and `NetworkOut` with aligned, unique, non-zero datapoints
- Updated `README.md` to document 50-query support on both the JSON and AWS CLI Query/XML `GetMetricData` paths.
- Verification completed successfully:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features`
  - `bash scripts/verify_cli_interop.sh`

## Pricing GetProducts Bug Investigation

- [x] Reproduce the reported `foxtail pricing get-products` failure against the local service using the AWS CLI path.
- [x] Isolate the request shape or handler branch that causes the failure.
- [x] Apply the minimal fix and add regression coverage if behavior is wrong in the service.
- [x] Re-run focused verification and record the result in this file.

## Pricing GetProducts Investigation Review

- Reproduced the documented pricing smoke command through both:
  - `aws --endpoint-url http://127.0.0.1:8080 pricing get-products --service-code AmazonEC2 --format-version aws_v1 --filters Type=TERM_MATCH,Field=instanceType,Value=m6i.large`
  - `foxtail pricing get-products --service-code AmazonEC2 --format-version aws_v1 --filters Type=TERM_MATCH,Field=instanceType,Value=m6i.large`
- Both commands succeeded on 2026-03-23 against the local service at `127.0.0.1:8080`.
- `GET /_mock/status` also returned `status: online`, so the currently running service is healthy.
- No repo-local server defect is confirmed yet. The remaining likely causes are:
  - a different CLI invocation than the documented smoke path
  - the Foxtail service not running when the command was issued
  - a wrapper/environment issue outside this repo checkout

## Pricing FormatVersion Follow-Up

- Updated `GetProducts` so omitted `FormatVersion` now defaults to the only supported schema, `aws_v1`.
- Kept the explicit rejection for any non-`aws_v1` format value.
- Added regression coverage for:
  - omitted `FormatVersion`
  - explicit invalid `FormatVersion`
- Verification completed:
  - `cargo fmt`
  - `cargo test pricing_get_products`
  - manual AWS CLI check against a fresh server on `127.0.0.1:18080` without `--format-version`
- Note: the long-running service on `127.0.0.1:8080` was still serving the old binary during verification, so end-to-end validation used a fresh instance on `127.0.0.1:18080`.

## Make Scenario Targets Build Prerequisite

- [x] Reproduce/inspect why `make gen-idle-heavy` can fail when `target/debug/aws-mock-data-service` is missing.
- [x] Add a binary-file prerequisite to Make targets that execute the debug service binary, so Cargo only runs when the binary is absent.
- [x] Verify the Make target plan without forcing a network-dependent rebuild.
- [x] Record verification results here.

## Make Scenario Targets Build Prerequisite Results

- Historical note: before the single-binary consolidation, `gen`, `gen-baseline`, `gen-spike`, `gen-idle-heavy`, and `serve` were changed to depend on `$(BIN)` instead of the phony `build` target.
- In the current single-binary setup, `$(BIN)` resolves to `target/debug/foxtail`.
- If `target/debug/foxtail` exists, Make skips `cargo build` and runs the requested command directly.
- If the binary is absent, Make invokes `cargo build` once to create it before running the command.
- Verification:
  - `make -n gen-idle-heavy` showed `cargo build` when the configured binary was missing.
  - `make -n BIN=target/debug/foxtail gen-idle-heavy` shows only the generation command, proving the existing-binary path skips Cargo.
  - `make -n BIN=target/debug/foxtail serve` shows only the serve command, proving the same for `serve`.
- Full `make gen-idle-heavy` was not completed because Cargo needed to download missing crates and network access was not approved.

## Single Foxtail Binary Consolidation

- [x] Resolve CLI design decisions with the user.
- [x] Rename the Cargo package/library/binary to `foxtail`.
- [x] Merge native `gen`/`serve` commands and AWS CLI-compatible routing into one `foxtail` binary.
- [x] Remove the separate `aws-mock-data-service` binary surface.
- [x] Update active Makefile, scripts, README, and tests.
- [x] Run formatting/tests where possible and record any verification blockers.

## Single Foxtail Binary Consolidation Results

- Cargo now exposes one library target named `foxtail` and one binary target named `foxtail`.
- `src/main.rs` now handles both native commands and AWS CLI-compatible routing:
  - `foxtail gen ...`
  - `foxtail serve ...`
  - `foxtail ce ...`
  - `foxtail s3 ...`
- Native `gen` and `serve` run in-process against the existing Rust service modules.
- AWS-shaped commands remain subprocess-based, preserving AWS CLI behavior and `awslocal` passthrough.
- Removed the separate `src/bin/foxtail.rs` wrapper binary source.
- Active Makefile, README, and verification scripts now target `target/debug/foxtail`.
- Added an HTTP-only AWS SDK client for `http://` discovery endpoints so LocalStack generation does not panic when native root certificates are unavailable.
- Verification completed:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features`
  - `bash scripts/verify_wrapper_routing.sh`
  - `cargo metadata --no-deps --format-version 1`
  - `make -n gen-idle-heavy`
  - `make gen-idle-heavy`
- `bash scripts/verify_cli_interop.sh` was not completed because the first run happened before `mock_data.db` existed. After generation, LocalStack discovery completed but found no resources in this environment, so the full AWS CLI smoke suite would not have useful seeded data to validate.

## PR Review Follow-Up

- [x] Move newly added CloudWatch JSON and Cost Explorer metric response formatting into `src/handlers/`.
- [x] Add integration tests under `tests/` for CloudWatch JSON/XML statistic responses and Cost Explorer `USAGE_TYPE` grouping.
- [x] Update the local repository instructions to reference `target/debug/foxtail`.
- [x] Move the remaining touched CloudWatch and Cost Explorer response envelope builders into `src/handlers/`.
- [x] Derive the dashboard CloudWatch scorecard count from the supported API inventory.
- [x] Move CloudWatch JSON `GetMetricData` and default Cost Explorer group-definition shaping into handler helpers.
- [x] Add CloudWatch JSON `ListMetrics` and `GetMetricStatistics` targets to the dashboard supported API inventory.
- [x] Move generic AWS JSON/XML error body serialization into `src/handlers/aws.rs`.
- [x] Clarify the handler ownership standard so reusable/extracted response builders live in `src/handlers/` while established endpoint orchestration can remain in `serve.rs`.
- [x] Count distinct operation names, not protocol inventory entries, in dashboard service scorecards.

# Issue #8: Foxtail fixture observation and account-bound readiness

Pinned base: `844e4cb764e8b165a8d25296642182d5de6fb1bf`

Invariant: the five deterministic read-only fixture controls are exposed only
through Foxtail's AWS-compatible EC2 observation surface; four disposable
mutation targets are created only in a fresh separate LocalStack account bound
to the manifest; STS identity and scope contradictions fail closed before any
external mutation or durable partial state.

## Acceptance Map

- [x] AC1-3: add EC2 Query/XML `DescribeInstances` parsing/response shaping
  with exactly the five manifest read-only rows and independent dispatch from
  CloudWatch Query; preserve JSON and existing CloudWatch behavior.
- [x] AC4: route wrapper `ec2 describe-instances` through `aws` with the
  Foxtail endpoint while preserving passthrough routing for unrelated commands
  and updating help text/tests.
- [x] AC5-6: bind mutation credentials to the manifest account, verify STS
  `GetCallerIdentity.Account` against that account before the first write, and
  make mock/transport/malformed/mismatch failures leave no partial mutation
  state.
- [x] AC7-8: prove account `123456789012` owns exactly four manifest mutation
  IDs, default `test` owns none, and fresh mutation LocalStack state is empty
  while the five read-only rows remain only on Foxtail's separate surface.
- [x] AC9-10: preserve Unseen checkout read-only; run reuse/readiness/recreate
  proof if feasible, otherwise record the pinned-source blocker; independently
  prove generation and all four target IDs rotate with readiness afterward.
- [x] AC11: add focused negative coverage for dispatch, account identity, and
  pre-mutation failures; preserve fail-closed behavior for wrong, missing,
  extra, stale, or contradictory observations.

## Execution Plan

- [x] Create `peter/issue-8-fixture-readiness` from the pinned base and map the
  acceptance criteria to `serve.rs`, `wrapper.rs`, `mutation.rs`, and fixture
  orchestration.
- [x] Implement the EC2 Query/XML observation route and exact manifest-driven
  response, with parser/handler/CloudWatch dispatch tests.
- [x] Implement wrapper routing/help updates and subprocess routing coverage.
- [x] Implement account-derived credentials plus pre-write STS identity proof,
  including deterministic mock success/mismatch/transport tests.
- [x] Add focused fixture/lifecycle negative coverage and update only the
  necessary contract/schema/docs artifacts.
- [x] Run focused formatting/tests and isolated live interoperability checks
  without touching shared `localstack-aws`; record environment gaps.
- [x] Inspect the complete diff and clean worktree, then commit the coherent
  implementation without pushing.

## Issue #8 Review / Results

- AC1-3 owner `src/serve.rs`: added independent EC2 Query/XML dispatch and
  manifest-driven `DescribeInstances` XML. `cargo test -q --test
  aws_api_contract ec2_query_describe_instances_returns_exact_manifest_rows`
  passed; `cargo test -q --test aws_api_contract
  cloudwatch_metric_statistics_json_and_xml_emit_requested_stats` passed;
  direct AWS CLI proof against a fresh Foxtail server on
  `http://127.0.0.1:18081` returned exactly five instances with stable IDs,
  `running`/code 16 state, `m6i.large`, `us-east-1a`, and five canonical
  fixture tags per row.
- AC4 owner `src/wrapper.rs`: `ec2 describe-instances` now routes through
  `aws --endpoint-url http://127.0.0.1:8080`; `cargo test -q --test
  foxtail_wrapper routes_ec2_describe_instances_to_aws_with_endpoint` and
  `bash scripts/verify_wrapper_routing.sh` passed. `s3 ls` remains awslocal
  passthrough.
- AC5-6 owner `src/mutation.rs` and fixture orchestration: credentials use the
  requested manifest account, and STS caller identity is checked before the
  first intent/write. Mock identity success, mismatch, transport, and
  malformed tests passed; `cargo test -q --test fixture_mutations
  wrong_sts_account_fails_before_mutation_intent_or_external_state` passed
  with zero intent, generation, or mutation resource rows on mismatch.
- AC7-8 owner mutation lifecycle: focused mock proof
  `cargo test -q --test fixture_mutations` passed all 20 tests, including four
  unique mutation targets and read-only/public absence assertions. A fresh
  dedicated LocalStack account-separation run could not be executed because
  Docker access is denied for this worker (`permission denied ...
  /Users/murphy/.orbstack/run/docker.sock`); shared `localstack-aws` was not
  touched.
- AC9-10 owner lifecycle plus the read-only Unseen consumer: deterministic
  mock lifecycle proves generation/readiness/recreation identity rotation in
  the existing suite. The pinned Unseen checkout remained clean at
  `721b6df1a179645a2bd483c9a5614a916042d36d`; its vendored contract still pins
  Foxtail source revision `844e4cb764e8b165a8d25296642182d5de6fb1bf`, so the
  consumer live reuse/readiness/recreate proof must be rerun after its pinned
  provenance and manifest golden/digest are updated to this branch's metadata
  contract, then it can point at this issue branch. Read-only targeted Unseen
  tests were not runnable because that checkout has no `pytest` module
  installed.
- AC11 owner EC2 dispatch and account preflight: unsupported/malformed EC2
  Query requests fail closed, strict manifest validation rejects unknown,
  duplicate, missing, contradictory controls/tags, and wrong-account mock
  realization leaves no durable mutation state. `cargo test -q --lib
  mutation::tests`, `cargo test -q --lib
  generated_manifest_matches_checked_in_canonical_golden`, and `git diff --check`
  passed; `python3 scripts/validate_release_fixture.py --definition
  tests/fixtures/release-qualification-v1.definition.json --manifest
  tests/fixtures/release-qualification-v1.manifest.json --negative` passed.
- Additional verification: `cargo fmt --all`, `cargo check`, and
  `cargo clippy --all-targets --all-features -- -D warnings` passed;
  `bash -n scripts/verify_cli_interop.sh scripts/verify_wrapper_routing.sh`
  passed. Broad full-suite and full CLI interoperability gates remain for the
  parent after frozen review as requested.

## Issue #8 Frozen Review Repair

- [x] Bind every mutation LocalStack query in `verify_cli_interop.sh` to
  explicit manifest-account or default-test-account credentials, prove both
  accounts are empty before realization, and verify the manifest account owns
  exactly four mutation IDs while the test account owns none afterward.
- [x] Require all four observed EC2 fields in the release manifest schema and
  cap read-only resources at exactly five; extend executable negative checks
  for each missing field and an extra sixth resource.
- [x] Move reusable EC2 Query parsing, manifest observation validation, state
  mapping, XML escaping, and response building into the deep
  `src/handlers/ec2.rs` protocol boundary while keeping route orchestration in
  `src/serve.rs`.
- [x] Add colocated EC2 handler tests for malformed/indexed Query members,
  action validation, contradictory observations/tags/scopes, invalid state,
  and XML escaping/state mapping; retain focused HTTP and CloudWatch tests.

### Frozen Repair Results

- Focused verification passed: `cargo fmt --all`, `cargo check`,
  `cargo test -q --lib handlers::ec2::tests` (7 tests), EC2 and CloudWatch
  API-contract filters (1 test each), wrapper routing filter (1 test), the
  pinned executable schema validator with `--negative`, `bash -n` for both
  verification scripts, and `git diff --check`.
- The shell repair defaults qualification proof to the dedicated LocalStack
  endpoint (`127.0.0.1:4666`) and fails closed when either account is not empty
  before realization or when post-realization IDs/scope differ. A live Docker/
  LocalStack run was not available to this worker; the parent must run the
  isolated live proof. Shared `localstack-aws` was not touched.
- The schema-only change does not alter fixture definition/manifest goldens or
  the manifest digest.

## Issue #8 Final Parent Verification

- Frozen Standards and Spec review of `3aac64f2a99e796ff922d82876351f536c1f6221`
  completed with zero findings after the repair commit.
- Broad verification passed with `cargo fmt --all -- --check`, `cargo test`
  (82 library, 3 API-contract, 20 mutation integration, 4 wrapper, and doc
  tests), and `cargo clippy --all-targets --all-features -- -D warnings`.
- After explicit user approval, the parent removed only the prior dedicated
  `foxtail-qualification-localstack` container and its anonymous data volume,
  then started a fresh ephemeral LocalStack 4.14.0 on `127.0.0.1:4666` with
  EC2 and STS. Shared `localstack-aws` on port 4566 remained untouched.
- Before realization, STS returned account `123456789012` for the manifest
  credential and `000000000000` for `test`; both EC2 inventories were empty.
- `AWS_ENDPOINT_URL=http://127.0.0.1:4666
  FOXTAIL_MUTATION_AMI_ID=ami-760aaa0f MOCK_DATA_DB=<disposable-migrated-db>
  bash scripts/verify_cli_interop.sh` passed end to end. It proved exactly five
  stable read-only EC2 rows on Foxtail, exactly four mutation IDs under the
  manifest account and none under `test`, status/fault/reset/recreate/destroy,
  mutation-generation and four-ID rotation, terminal cleanup/public absence,
  and every remaining AWS CLI compatibility check.
- The read-only Unseen #354 checkout stayed clean at
  `721b6df1a179645a2bd483c9a5614a916042d36d`. Its live `reuse` command reached
  Foxtail on port 8080 and LocalStack on 4666, then rejected fail-closed with
  primary reason `foxtail_source_revision_mismatch`, as its captured contract
  still pins the old Foxtail source.
- Additional Unseen diagnostics are consumer follow-ups, not silent Foxtail
  changes: the #354 collector compares the full Resource Groups inventory
  (five read-only rows plus four active #5 mutation mappings) with only the
  read-only IDs, and its 86,400-second CloudWatch collection reported
  `public_timestamp_outside_fixture_bucket` while comparing rebucketed public
  timestamps with raw fixture offsets. The resulting inventory, evidence, and
  fingerprint mismatches require an explicit Unseen contract/collector
  decision before `reuse` and subsequent `recreate` can be accepted.
- The exact Unseen pin refresh is Foxtail source
  `3aac64f2a99e796ff922d82876351f536c1f6221`, manifest schema file SHA-256
  `c7a50c2512a5471226355b4a97cfd048b7648fd9a5fd6ddf2ab6a2d5b33e1e66`,
  manifest golden file SHA-256
  `23e2a5f6f74c60825cfdf92185e5ffdde7f609b0fb2d96b2739fde9762db9817`,
  and manifest self-digest
  `sha256:521437a47f2dbb492525b6edd95770f0a0402b366b01e581ddcd3a20b9f9aeb1`;
  the captured schema/golden and `fixture_contract.py` source-revision guard
  must be updated together in an authorized Unseen change.

# Issue #8 Live Reuse Follow-Up

- [x] Exclude active qualification mutation targets from the Foxtail Resource Groups read-only inventory.
- [x] Prove the five-row inventory remains exact while a mutation generation is active.
- [x] Run focused tests, full Rust verification, and the live Foxtail/LocalStack/Unseen reuse check.
- [x] Record review findings and final verification below.

## Final verification

- Foxtail's frozen code head `860242fe5ee2bb8cb3db5347878ad6fe6ddee1b5`
  passed `cargo test` (82 library, 4 API-contract, 20 mutation integration,
  4 wrapper, and doc tests) and
  `cargo clippy --all-targets --all-features -- -D warnings`.
- `FOXTAIL_SOURCE_REVISION=dbe899e5df8a56c434768a71643e55b9e1315582
  FOXTAIL_MUTATION_AMI_ID=ami-03cf127a
  AWS_ENDPOINT_URL=http://127.0.0.1:4666
  AWS_MOCK_VERIFY_PORT=18080 bash scripts/verify_cli_interop.sh` passed against
  a fresh disposable LocalStack 4.14.0 estate, including exact inventory and
  the complete fault/reset/recreate/destroy lifecycle.
- Unseen's frozen head `26055541dce1dd283f3c555e6e232cc6fd12cdc7`
  passed its focused 91-test acceptance matrix and full 2,837-test unit suite.
  Its real `reuse` CLI then inspected a fresh Foxtail realization while the
  destroyed-generation LocalStack tombstones remained and returned
  `accepted=true`, `ready=true`, `status=REUSABLE`, and
  `reason=readiness_verified`.
- Frozen-object Spec/correctness review was clean across all twelve acceptance
  criteria. Standards review found no code issue and identified only the
  previously incomplete progress/results record closed by this section.

# Issue #12: Publish Explicit Oracle Policy Selectors

Pinned base: `eaf5f12c97a7997bd3a00cfff16e516c17c0477d`

## Acceptance Map

- [x] Add the exact `finding_type` map to Foxtail's canonical definition builder and prove all five read-only controls in the canonical golden/CLI/HTTP output.
- [x] Require supported non-empty `finding_type` for positive, negative, and degraded definition controls in the Draft 2020-12 schema and runtime/negative validation; mutation controls must omit it.
- [x] Document selector authority as production policy-registration metadata only and reject nested/misplaced selector or expected-outcome encodings while preserving existing forbidden policy fields.
- [x] Advance `DEFINITION_REVISION`, schema consts, canonical definition/manifest digests, and checked-in goldens deterministically; update Foxtail-owned source-revision assertions without editing Unseen.
- [x] Keep the manifest bound to the exact definition digest without duplicating selector truth; preserve canonical CLI/HTTP parity and schema validation.
- [x] Preserve mutation/readiness/AWS CLI/interoperability behavior and run focused Unseen #355 compatibility proof read-only.
- [x] Commit the functional implementation, then a separate source-pin/golden update commit if required; report exact hashes and final head.

## Execution Plan

- [x] Inspect current definition, manifest, schema, validator, CLI/HTTP parity, and sibling Unseen policy registration.
- [x] Implement selectors, schema/runtime validation, docs, focused tests, and regenerate goldens.
- [x] Run focused Rust/schema/CLI checks and the read-only Unseen compatibility command.
- [x] Commit the functional change and any separate source-pin/golden change without amending.
- [x] Review the complete diff against every acceptance criterion and record results, residual gaps, and final git status.

## Review and Results

The functional implementation is committed as `71a74458325b6f836d5b195084a7d28218bf9241`.
The frozen implementation head reviewed before this repair was
`7702c7e7046d48e0a7460b67c03158e7fccae2b1`.
The follow-up source-pin/golden commit updates the checked-in manifest's
`generator.source_revision` and the Foxtail-owned golden assertion to that
functional commit without amending it; its exact hash and final branch head
are reported in the worker handoff. The repair is likewise one focused,
non-amended commit; because a commit cannot durably contain its own hash, the
repair hash and final head are reported in that handoff.

- Canonical definition selectors are exact: idle positive/negative/degraded
  map to `idle_instance`; resize positive/negative map to `rightsizing`; all
  four mutation controls omit the selector.
- Runtime and Draft 2020-12 validation reject missing, empty, unsupported,
  whitespace-padded, mutation, nested, and manifest/control-catalogue selector
  placements while retaining forbidden expected-finding rejection.
- Definition revision is `1.0.1`; definition digest is
  `sha256:a81555c81a528dae62934786e2a51080af674daa998b94da6c4186eae6b778a6`.
  The manifest binds that digest and contains no `finding_type` key.
- Focused proof passed: Rust selector/definition tests, full `cargo test -q`,
  `cargo clippy --all-targets --all-features -- -D warnings`, canonical-format
  checks, `python3 scripts/validate_release_fixture.py` and
  `python3 scripts/validate_release_fixture.py --negative`. Selector-negative
  cases now call a schema-only Draft 2020-12 path, while recursive/runtime and
  forbidden-policy checks remain separate.
- Read-only Unseen compatibility command shape (run from this Foxtail
  worktree; it imports the sibling checkout but does not write it):

  ```sh
  python3 - <<'PY'
  import copy, json, sys
  from pathlib import Path
  sys.path.insert(0, "/Users/murphy/workspace/iacai0/unseen-agent")
  from unseen.extensions.scan.evidence_policy_registry import EVIDENCE_POLICY_REGISTRATIONS
  from unseen.extensions.scan.oracle_derivation import _manifest_read_only_controls, _registration_for

  root = Path("/Users/murphy/.codex/worktrees/6d3a/foxtail")
  definition = json.loads((root / "tests/fixtures/release-qualification-v1.definition.json").read_text())
  manifest = json.loads((root / "tests/fixtures/release-qualification-v1.manifest.json").read_text())
  controls, reasons = _manifest_read_only_controls(
      definition, manifest, manifest["environment"], EVIDENCE_POLICY_REGISTRATIONS
  )
  assert not reasons, reasons
  expected = {
      "ec2-idle-positive-001": "idle_instance",
      "ec2-idle-negative-001": "idle_instance",
      "ec2-idle-degraded-001": "idle_instance",
      "ec2-resize-positive-001": "rightsizing",
      "ec2-resize-negative-001": "rightsizing",
  }
  actual = {
      control.control_id: _registration_for(
          control.definition, EVIDENCE_POLICY_REGISTRATIONS
      ).registration.finding_type
      for control in controls
  }
  assert actual == expected, actual
  absent = copy.deepcopy(definition)
  absent["controls"][0].pop("finding_type")
  _, absent_reasons = _manifest_read_only_controls(
      absent, manifest, manifest["environment"], EVIDENCE_POLICY_REGISTRATIONS
  )
  assert "unsupported_capability:ec2-idle-positive-001" in absent_reasons
  print("unseen_policy_selector_compatibility=pass")
  print("absent_selector=unsupported_capability:ec2-idle-positive-001")
  PY
  ```

  Result: `unseen_policy_selector_compatibility=pass`; all five selectors
  selected `idle_instance`/`rightsizing` with no `unsupported_capability`, and
  removing one selector produced
  `unsupported_capability:ec2-idle-positive-001`.
- Focused positive/negative/degraded oracle proof:

  ```sh
  pytest -q tests/unit/test_runtime_oracle.py -k 'production_oracle_ignores_fixture_tags_and_freezes_receipt or oracle_blocks_degraded_control_and_rejects_positive_contradiction or pinned_unqualified_fixture_controls_block_without_registration_metadata or oracle_does_not_infer_capability_from_fixture_prose' --disable-warnings
  ```

  Result: `4 passed, 24 deselected, 2 warnings` (the known shell wrapper also
  emitted `(eval):5: parse error near \`end\`` before pytest ran).
- The Unseen checkout remained at exact head
  `f4c5e7802def856fb4d4ec6996cbd616ea16bd95` and clean. No Unseen files,
  external tracker state, or Issue #12 completion record were changed; tracker
  recording remains for an authorized future issue update.
- No implementation gap remains in this repair slice; the parent should run
  its broad verification matrix and any available LocalStack/CLI smoke before
  publication.
