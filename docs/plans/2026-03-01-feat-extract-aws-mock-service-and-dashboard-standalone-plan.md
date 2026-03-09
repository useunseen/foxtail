---
title: feat: Extract aws-mock-service and mock API metrics dashboard into a standalone product
type: feat
date: 2026-03-01
---

# feat: Extract aws-mock-service and mock API metrics dashboard into a standalone product

## Overview

Separate the Rust AWS mock service and the dashboard that visualizes mock API metrics from the main `aws-optimize-agent` product boundary.

The result should be a standalone mock-service product/repo that owns:
- AWS-compatible mock API serving
- mock dataset generation and persistence
- the dashboard used to inspect mock API coverage and metrics
- service-specific tests, docs, and developer workflows

The `aws-optimize-agent` repo should remain a client of that service through `AWS_MOCK_ENDPOINT`, not the owner of its implementation or UI.

## Enhancement Summary

**Planning depth:** A LOT  
**Primary audience:** mock-service maintainers, agent/backend maintainers, frontend maintainers  
**Primary architectural change:** extract the mock backend and mock dashboard into a standalone repo/product boundary  
**Important constraint:** this is a repo-boundary and ownership change, not just a path rename

## Brainstorm Context

Found relevant brainstorm from 2026-02-18 and used it as planning context:
- `docs/brainstorms/2026-02-18-aws-mock-data-service-rust-cli-brainstorm.md`

Key carry-forwards:
- the Rust mock service is intended to be a real product boundary, not a test helper
- AWS wire compatibility is a first-class contract
- SQLite-backed seeded data and deterministic local workflows are core behavior, not incidental implementation details
- the service should be usable by switching endpoints, not by embedding repo-specific logic into clients

## Problem Statement

The current architecture is split across the wrong boundary.

What exists today:
- the Rust mock service already behaves like a standalone service under `services/aws-mock-data-service`
- the mock API dashboard is embedded inside the main app frontend under `dashboard-ui`
- the main repo root Makefile proxies mock-service commands into the Rust service directory
- tests, docs, and scripts still assume the mock service lives inside this monorepo

Why this is a problem:
- if the mock service is moved to another repo, the dashboard that visualizes it does not move with it
- the current dashboard route is mounted inside the main application shell, which couples it to unrelated auth/session/scheduling concerns
- service-specific tests and scripts are path-coupled to this repo layout
- the current setup makes the mock service look standalone operationally, but not standalone as a product

The real problem is not that the frontend and backend use different ports in development.
The real problem is that the dashboard lives in the wrong repo and wrong app shell.

## Goals

### In Scope

- define the target standalone product boundary for the mock service and dashboard
- define what moves, what stays, and what must be rewritten
- define the migration phases to extract backend, dashboard, tests, docs, and developer commands
- preserve the existing `AWS_MOCK_ENDPOINT` integration seam used by `aws-optimize-agent`
- keep dashboard functionality focused on visualizing mock API metrics and coverage
- make the standalone repo the sole owner of mock seed/state lifecycle
- define canonical IDs and machine-readable verification artifacts for the standalone product
- define backward-compatibility and cutover behavior for existing repo-local commands and routes
- define acceptance criteria and rollout/cutover validation

### Out of Scope

- changing the `aws-optimize-agent` runtime architecture beyond consuming the standalone service by endpoint
- IAM design or auth hardening for the mock service beyond existing local-only assumptions
- redesigning the dashboard into a broader product than mock API observability
- packaging/deploying the standalone repo to production infrastructure
- implementing the extraction in this document; this plan is for the work that should follow

## Required Clarifications Resolved By This Plan

This plan makes the following decisions explicit so the extraction does not preserve split ownership.

1. **Seed/state ownership**
The standalone repo is the single owner of:
- seed generation
- SQLite schema and migrations
- scenario mutation
- dashboard/admin data contracts

`aws-optimize-agent` remains only a consumer of the running service.

2. **Standalone UI shape**
The standalone repo ships a separate dashboard app in the same repo as the backend.
It is not hosted inside the `aws-optimize-agent` app shell.

3. **Machine-readable verification**
The standalone repo must publish canonical JSON verification artifacts for:
- supported API inventory
- parity/coverage scorecard
- contract version
- schema version
- reconciliation/health status

4. **Canonical IDs**
The standalone product must treat IDs as protocol and labels as presentation.
At minimum it must define stable:
- `resource_id`
- `series_id`
- `api_entry_id`
- `scenario_id`
- `contract_version`

5. **Cutover**
The main repo’s in-tree dashboard route and repo-local mock-service ownership commands are deprecated and then removed on a defined timeline.

## Research Consolidation

## Internal Repo Findings

### 1. The Rust mock service is already the cleanest extraction unit

The backend crate already owns its runtime, persistence, protocol routing, and data generation.

Relevant evidence:
- `services/aws-mock-data-service/src/main.rs:13`
- `services/aws-mock-data-service/src/cli.rs:21`
- `services/aws-mock-data-service/src/cli.rs:37`
- `services/aws-mock-data-service/src/serve.rs:25`
- `services/aws-mock-data-service/src/db.rs:9`
- `services/aws-mock-data-service/src/metrics.rs:23`
- `services/aws-mock-data-service/src/generator.rs:1`
- `services/aws-mock-data-service/migrations/20260218120000_initial_schema.sql:3`

What this means:
- the Rust service should move as a whole crate
- its migrations, CLI, SQLite lifecycle, and API router should stay together
- extraction should not split handlers from persistence or generation logic

### 2. The dashboard feature is portable, but the current app shell is not

The dashboard route is currently piggybacked into the main app shell.

Relevant evidence:
- `dashboard-ui/src/App.tsx:13`
- `dashboard-ui/src/App.tsx:239`
- `dashboard-ui/src/components/MockApiDashboard.tsx:246`
- `dashboard-ui/src/components/MockApiDashboard.tsx:293`
- `dashboard-ui/src/lib/api-mock-dashboard.ts:98`
- `dashboard-ui/vite.config.ts:61`

What this means:
- `MockApiDashboard` and its typed API client are good extraction candidates
- the main `dashboard-ui/src/App.tsx` should not be moved unchanged into the standalone repo
- the standalone dashboard needs its own entrypoint and router, not the agent app’s auth/session/schedule shell
- the existing Vite proxy pattern for `/_mock` can be reused directly

### 3. The agent already consumes the mock service through the correct seam

The agent repo depends on the Rust service over HTTP, not through direct code imports.

Relevant evidence:
- `graph_builder.py:74`
- `graph_builder.py:134`
- `graph_builder.py:141`
- `graph_builder.py:1003`
- `graph_builder.py:1101`
- `graph_builder.py:1160`

What this means:
- extraction does not require the agent to vendor Rust code
- the standalone service can remain an external dependency addressed via `AWS_MOCK_ENDPOINT`
- the correct direction is to reduce in-repo ownership, not create tighter code coupling

### 4. Workflow, tests, and docs are still monorepo-coupled

Relevant evidence:
- `Makefile:184`
- `Makefile:201`
- `scripts/test_aws_cli_parity.sh:5`
- `tests/integration/helpers/mock_service_runner.py:61`
- `tests/integration/test_mock_dashboard_contract.py:14`
- `README.md:56`
- `README.md:89`
- `README.md:127`
- `dashboard-ui/package.json:2`
- `dashboard-ui/README.md:2`

What this means:
- tests and scripts need path flattening when moved
- the standalone repo should promote service commands to top-level commands
- current docs already contain drift and should be cleaned during extraction
- repo extraction is not complete unless developer workflow and documentation move with it

### 5. The current system does not yet define canonical machine-readable ownership artifacts

Relevant evidence:
- `services/aws-mock-data-service/src/serve.rs`
- `docs/testing/aws-mock-api-coverage-status.md:83`
- `dashboard-ui/src/lib/api-mock-dashboard.ts:21`

What this means:
- the standalone repo needs one canonical JSON truth for contract inventory and parity status
- CI, docs, and dashboard rendering should all derive from backend-owned machine-readable artifacts
- labels such as dashboard series labels or human-readable API names must not become identity keys

## Institutional Learnings

Relevant learnings from `docs/solutions/`:
- `docs/solutions/integration-issues/high-fidelity-aws-mocking-rust-service.md`
- `docs/solutions/integration-issues/scheduled-session-unification-invariants-assistant-20260216.md`
- `docs/solutions/logic-errors/incremental-scan-ledger-state-regressions-assistant-20260213.md`
- `docs/solutions/ui-bugs/duplicate-scheduled-phase-cards-assistant-20260211.md`

Key lessons to carry forward:
- treat the mock service as a real standalone boundary with explicit protocol ownership
- enforce storage/API/UI invariants explicitly; do not let the dashboard infer them differently than the backend emits them
- use canonical IDs and backend-defined semantic keys; do not let display labels become protocol
- add regression coverage for stale responses, duplicate event delivery, and reconciliation gaps once UI and service are separated more cleanly

## External Research Decision

External research was not required for this plan.

Reason:
- this is primarily a repo-boundary and product-ownership extraction problem
- the current repo already contains the architectural evidence needed
- the critical success factors are local: file ownership, API boundaries, test movement, and developer workflow migration

## Proposed Solution

Create a standalone repo, tentatively named `aws-mock-service`, with two first-class packages:
- `server/` for the Rust AWS-compatible mock service
- `dashboard/` for the standalone metrics/coverage dashboard

The new repo should own the product end-to-end.

### Recommended Target Structure

```text
aws-mock-service/
  server/
    Cargo.toml
    src/
    migrations/
    Makefile
  dashboard/
    package.json
    vite.config.ts
    src/
  tests/
    integration/
    fixtures/
  scripts/
    test_aws_cli_parity.sh
  docs/
    testing/
    solutions/
  Makefile
  README.md
```

### Product Ownership Rule

The standalone repo owns all of the following or the extraction is incomplete:
- protocol emulation
- data generation
- DB schema and migrations
- scenario lifecycle
- dashboard/admin API contracts
- dashboard UI for metrics and coverage
- machine-readable verification outputs

The agent repo owns none of these implementations after cutover.

### Boundary Decision

Move the Rust service wholesale, but do not move the current `dashboard-ui` app wholesale.

Instead:
- move the Rust crate as the backend product unchanged except for path cleanup
- create a dedicated standalone dashboard app by extracting:
  - `dashboard-ui/src/components/MockApiDashboard.tsx`
  - `dashboard-ui/src/lib/api-mock-dashboard.ts`
  - the minimal shared UI primitives and test files required by that component
- leave behind agent-specific auth/session/schedule/chat UI concerns

This avoids carrying unrelated main-app code into the new repo.

### Development Port Strategy

Default recommendation:
- keep separate ports in development
  - mock backend on `:8080`
  - dashboard Vite dev server on `:3000` or `:5173`
- use a Vite proxy for `/_mock` to the backend during development

Reason:
- this is already close to the current working setup
- it keeps frontend HMR simple
- it avoids unnecessary reverse-proxy complexity during extraction

Optional later improvement:
- add a single-port packaged mode where the Rust server serves built dashboard assets or sits behind a simple proxy

That is a follow-up convenience improvement, not a prerequisite for extraction.

### Compatibility Policy

Define and document an explicit compatibility contract between repos:
- `aws-optimize-agent` consumes a versioned standalone service contract
- breaking response-shape changes require a contract version bump and coordinated consumer update
- the standalone repo publishes supported contract versions in machine-readable form

## What Moves vs What Stays

### Move to the Standalone Repo

Backend:
- `services/aws-mock-data-service/`

Dashboard source to extract or copy into new dashboard package:
- `dashboard-ui/src/components/MockApiDashboard.tsx`
- `dashboard-ui/src/lib/api-mock-dashboard.ts`
- `dashboard-ui/src/components/__tests__/MockApiDashboard.test.tsx`
- minimal shared UI primitives used by `MockApiDashboard`
- Vite proxy setup from `dashboard-ui/vite.config.ts`

Service-focused tests and helpers:
- `tests/integration/conftest_mock_service.py`
- `tests/integration/helpers/mock_service_runner.py`
- `tests/integration/test_mock_dashboard_contract.py`
- CE/CW parity suites that are purely mock-service contract tests
- corresponding fixtures under `tests/integration/fixtures/aws_parity/`

Scripts:
- `scripts/test_aws_cli_parity.sh`

Docs:
- `docs/testing/aws-mock-api-coverage-status.md`
- `docs/testing/aws-cli-parity-command-matrix.md`
- service-specific plan/solution documents that describe the mock-service product

### Stay in `aws-optimize-agent`

- `graph_builder.py` and other agent runtime code that calls the mock service by endpoint
- agent UI, auth, session, schedule, chat, scan, and execution flows
- main product docs focused on the optimization agent itself

### Rewrite or Reshape During Extraction

- the dashboard application shell
- top-level Makefiles and startup commands
- tests/scripts that assume `services/aws-mock-data-service` lives under this monorepo root
- docs that still refer to `frontend/`, `localhost:5173`, or generic Figma-bundle naming

## Technical Approach

## Phase 0: Contract Freeze and Extraction Inventory

Goal: freeze what the standalone product owns before moving files.

Tasks:
- inventory all service-owned API endpoints in `services/aws-mock-data-service/src/serve.rs`
- inventory all dashboard dependencies imported by `MockApiDashboard.tsx`
- inventory service-owned tests vs agent-owned tests
- inventory docs/scripts that hardcode current monorepo paths
- define a single canonical list of moved artifacts
- define canonical IDs and contract-version fields for backend/dashboard/test use
- define the machine-readable verification artifacts the standalone repo must emit

Deliverables:
- extraction checklist with exact source paths
- ownership matrix: move / stay / rewrite
- canonical API contract list for `/_mock/*` and AWS-compatible endpoints
- verification artifact schema for inventory/scorecard/version/reconciliation outputs

## Phase 1: Bootstrap the Standalone Repo

Goal: create the new repo skeleton without changing behavior.

Tasks:
- create top-level `server/`, `dashboard/`, `tests/`, `scripts/`, `docs/`
- promote service build/gen/serve commands to the new repo root Makefile
- establish top-level README with local dev instructions
- define environment variables for backend and dashboard (`DATABASE_URL`, `AWS_ENDPOINT_URL`, `AWS_MOCK_PROXY_TARGET`, dashboard base URL)
- define top-level compatibility/version files for contract and schema tracking

Deliverables:
- new repo skeleton
- root dev commands such as `make build`, `make gen`, `make serve`, `make dev-dashboard`, `make test`
- explicit contract/version manifest

## Phase 2: Move the Rust Service as a Whole Unit

Goal: extract the backend without breaking its behavior.

Tasks:
- move `services/aws-mock-data-service` to `server/`
- preserve CLI, migrations, SQLite behavior, generator, and `serve.rs` routes
- flatten path assumptions in scripts/tests
- keep service-local `.gitignore`, DB paths, and build outputs scoped to the new repo
- consolidate any remaining mock seed generation logic so the standalone repo is the single source of truth

Validation:
- `cargo build`
- `cargo test` if present
- existing parity suites and dashboard contract tests against the moved server
- health checks for:
  - `/_mock/status`
  - `/_mock/dashboard/data`
  - AWS-compatible CE/CW endpoints already implemented
- verification artifact generation and schema/version reporting

## Phase 3: Build the Standalone Dashboard App

Goal: give the standalone repo its own UI product, not a borrowed route from the main app.

Tasks:
- create a dedicated dashboard app entrypoint
- extract `MockApiDashboard` and its typed client
- copy only the minimal shared UI components and utilities needed by this dashboard
- preserve the Vite `/_mock` proxy behavior
- add a dashboard README and package metadata specific to the mock product
- remove dependencies on auth/session/agent hooks from the standalone app shell
- ensure request freshness rules are explicit so stale or out-of-order responses are discarded
- ensure the UI treats backend IDs as canonical and labels as presentation only

Validation:
- `npm install`
- `npm run test`
- `npm run dev`
- manual render of dashboard against the local mock server

## Phase 4: Move Service-Owned Tests, Fixtures, and Scripts

Goal: make the standalone repo self-verifying.

Tasks:
- move mock-service contract tests and fixtures into the new repo
- move `test_aws_cli_parity.sh`
- update helper paths to the new repo layout
- add one top-level command to run the core service test matrix
- add tests for stale responses, version skew, empty DB state, and scenario reconciliation

Recommended test split:
- standalone repo owns backend/API/dashboard contract tests
- `aws-optimize-agent` keeps only integration tests that verify it can consume a running mock service via endpoint configuration

## Phase 5: Cut `aws-optimize-agent` Over to External Consumption

Goal: make the agent repo a consumer only.

Tasks:
- remove in-repo ownership docs and commands that imply the mock service is part of this repo’s main product
- keep `AWS_MOCK_ENDPOINT` support and document it as an external service dependency
- remove or replace root Makefile proxy targets if they would mislead ownership
- update README setup guidance to point to the standalone repo for mock-service startup
- define a deprecation window for:
  - `/mock-dashboard`
  - root `make serve-mock` style commands
  - docs that assume `services/aws-mock-data-service` exists here

Validation:
- agent can still run with `AWS_MOCK_ENDPOINT=http://127.0.0.1:8080`
- agent no longer depends on internal Rust-service paths

## Phase 6: Documentation and Cutover Cleanup

Goal: make the new boundary obvious and durable.

Tasks:
- rewrite stale docs and package names
- ensure the standalone README explains local dev, seeding, serving, dashboard usage, and parity testing
- update the agent README to describe the standalone mock service as an optional companion repo
- add migration notes for contributors

## SpecFlow Analysis

SpecFlow analysis and repo evidence indicate that extraction must be defined as a product-boundary migration, not a file move.

### Core User Flows

#### Flow 1: Mock-service developer works only in the standalone repo

1. Start LocalStack or point the generator at a compatible AWS endpoint.
2. Run the standalone service generation flow.
3. Start the mock server.
4. Start the dashboard.
5. Open the dashboard and inspect coverage, metrics, resource trends, and mock API health.
6. Run parity tests and dashboard contract tests locally.

Acceptance implication:
- this flow must not require the `aws-optimize-agent` backend or frontend to be running
- this flow must also produce machine-readable verification outputs from the standalone repo alone

#### Flow 2: Agent developer consumes the standalone service from `aws-optimize-agent`

1. Start the standalone mock service separately.
2. Set `AWS_MOCK_ENDPOINT` in the agent repo.
3. Run the agent backend and, if needed, the agent UI.
4. Confirm CloudWatch/Cost Explorer enrichment still resolves through the external endpoint.

Acceptance implication:
- the agent repo must not need local Rust source paths or in-repo proxy commands to use the service
- compatibility/version expectations between repos must be explicit

#### Flow 3: Dashboard developer iterates on the dashboard alone

1. Run the standalone dashboard dev server.
2. Proxy `/_mock` to the mock backend.
3. Work only on the dashboard app.
4. Use mocked fetch in UI tests and a real local service for manual verification.

Acceptance implication:
- the dashboard app must have its own entrypoint and test setup
- it must not require agent auth/session state to render
- stale responses and out-of-order results must be discarded deterministically

#### Flow 4: CI validates the standalone product

1. Build backend.
2. Seed or generate deterministic mock data.
3. Start backend.
4. Run service parity/contract tests.
5. Run dashboard tests.
6. Optionally run an end-to-end dashboard smoke against the local service.

Acceptance implication:
- backend, dashboard, and service-owned tests must live together in the standalone repo
- CI must fail from backend-owned verification artifacts, not from hand-maintained docs alone

### Edge Cases and Migration Risks

- dashboard still imports main-app-only hooks or context providers
- tests still assume `services/aws-mock-data-service` relative paths
- docs still instruct users to run `cd frontend` or other stale commands
- service generation still depends on hidden setup behavior from this repo
- duplicate or stale UI updates appear after extraction because request freshness and canonical IDs were not preserved
- event/status labels get repurposed as identifiers instead of keeping backend semantic IDs authoritative
- dashboard request A returns after request B and incorrectly overwrites newer data
- scenario mutation races with dashboard refresh and produces mismatched counts or labels
- empty or partially generated DB states are not represented deterministically
- dashboard and backend versions drift without a visible contract-version mismatch surface

### Mandatory Machine-Readable Artifacts

The standalone repo must publish backend-owned JSON artifacts for at least:
- supported API inventory
- parity/coverage scorecard
- contract version
- schema version
- reconciliation status / generation freshness

The dashboard and CI should consume these artifacts instead of re-encoding the same truth in separate places.

## Acceptance Criteria

### Product Boundary

- [x] A standalone repo exists that owns the Rust mock service and the mock metrics dashboard.
- [x] The standalone dashboard can be run and tested without starting the `aws-optimize-agent` app.
- [ ] The `aws-optimize-agent` repo consumes the standalone service only via endpoint configuration.
- [ ] The standalone repo is the sole owner of mock seed generation, DB state, scenario lifecycle, and dashboard/admin contracts.

### Backend Extraction

- [x] The Rust service is moved as a whole unit with CLI, migrations, SQLite storage, generator, and API router intact.
- [x] Existing service endpoints still respond with the same contract shape after extraction.
- [x] Service-specific parity tests and dashboard contract tests run from the standalone repo.
- [x] The standalone backend publishes machine-readable contract inventory, parity scorecard, schema version, and contract version outputs.
- [x] Canonical IDs are defined for resources, series, API entries, and scenarios.

### Dashboard Extraction

- [x] The standalone dashboard has its own entrypoint and does not depend on agent auth/session/schedule state.
- [x] `MockApiDashboard` functionality is preserved.
- [x] The dashboard reaches the backend through same-origin `/_mock` or a configurable base URL/proxy.
- [ ] The dashboard discards stale/out-of-order responses and reconciles correctly after scenario changes.
- [x] The dashboard treats labels as presentation and backend IDs/version fields as source of truth.

### Workflow and Docs

- [x] The standalone repo has top-level commands for build, generate, serve, dashboard dev, and test.
- [x] The standalone repo README is accurate and product-specific.
- [ ] The agent repo README no longer misrepresents the mock service/dashboard as part of its main frontend.
- [ ] Cutover docs define old vs new commands, URLs, env vars, and deprecation/removal dates.

### Integration Safety

- [ ] `aws-optimize-agent` still works against a running standalone mock service via `AWS_MOCK_ENDPOINT`.
- [ ] Contract and integration tests cover stale responses, duplicate updates, and path/config drift introduced by extraction.
- [ ] Contract/invariant tests cover empty DB state, scenario mutation reconciliation, and version skew handling.

## Recommended Deliverables

- standalone repo scaffold
- extracted Rust mock service under `server/`
- standalone dashboard app under `dashboard/`
- moved service-owned tests/fixtures/scripts/docs
- updated `aws-optimize-agent` README and Makefile guidance
- migration note documenting the new ownership boundary

## Risks and Mitigations

### Risk: Moving the whole `dashboard-ui` folder drags unrelated app code into the new repo

Mitigation:
- extract only the mock dashboard feature and its minimal dependencies
- create a new dashboard shell instead of reusing the current main app shell

### Risk: Hidden path assumptions break tests and scripts

Mitigation:
- flatten paths during Phase 0 inventory
- rewrite helpers to resolve from repo root, not `services/aws-mock-data-service`
- add one validation pass that runs tests using only the new repo layout

### Risk: LocalStack seeding remains implicitly owned by the agent repo

Mitigation:
- decide explicitly whether LocalStack resource seeding moves into the standalone repo or is documented as an external prerequisite
- do not leave this as an undocumented dependency

### Risk: Version skew between the standalone repo and `aws-optimize-agent`

Mitigation:
- define a contract version and publish it in machine-readable form
- make version mismatch visible in CI and dashboard diagnostics
- require coordinated updates for breaking contract changes

### Risk: Contract drift between service and dashboard after repo split

Mitigation:
- keep typed dashboard client with the standalone dashboard
- keep dashboard contract tests with the service repo
- preserve canonical IDs and response semantics in backend-owned contracts

### Risk: Split-brain ownership persists after the repo move

Mitigation:
- move or retire duplicate seed/generation logic
- make the standalone repo the sole owner of seed/state lifecycle
- reject partial extraction that leaves data generation responsibilities ambiguous

### Risk: Agent contributors still think the mock service is maintained here

Mitigation:
- remove misleading root commands/docs after cutover
- document the standalone repo clearly in `aws-optimize-agent`

## Decision Log

### Decision 1: Separate repo ownership is the right boundary

Reason:
- the mock service is already architected as a standalone service
- the dashboard belongs to that product, not to the optimization-agent app shell

### Decision 2: Extract the dashboard feature, not the current app shell

Reason:
- `MockApiDashboard` is reusable
- `dashboard-ui/src/App.tsx` is not a valid standalone shell for the mock product

### Decision 3: Keep two-port local development first

Reason:
- current Vite proxy setup already supports this well
- it minimizes migration complexity
- single-port packaging can be added later if desired

### Decision 4: The standalone repo must publish versioned, machine-readable verification outputs

Reason:
- docs and UI should not be the source of truth for parity/coverage status
- CI and downstream consumers need backend-owned compatibility signals

### Decision 5: The standalone repo owns seed/state lifecycle

Reason:
- split ownership would preserve the current ambiguity instead of fixing it
- product-boundary extraction only works if generation, storage, scenario control, and dashboard contracts live together

## Implementation Checklist

- [x] Freeze extraction inventory and ownership matrix
- [x] Create standalone repo skeleton
- [x] Move Rust service crate into `server/`
- [x] Create standalone dashboard app
- [x] Extract `MockApiDashboard` and typed client
- [x] Copy minimal shared UI dependencies
- [x] Define canonical IDs and contract-version fields
- [x] Add machine-readable verification artifact outputs
- [x] Move service-owned tests, fixtures, and parity scripts
- [x] Promote top-level dev/test commands in new repo
- [ ] Update `aws-optimize-agent` docs and Makefile guidance
- [ ] Define deprecation/removal dates for old routes and commands
- [x] Run backend, dashboard, contract, and consumer integration validation

## References

- `docs/brainstorms/2026-02-18-aws-mock-data-service-rust-cli-brainstorm.md`
- `docs/solutions/integration-issues/high-fidelity-aws-mocking-rust-service.md`
- `docs/testing/aws-mock-api-coverage-status.md`
- `docs/testing/aws-cli-parity-command-matrix.md`
- `services/aws-mock-data-service/src/serve.rs`
- `services/aws-mock-data-service/src/cli.rs`
- `services/aws-mock-data-service/src/generator.rs`
- `services/aws-mock-data-service/src/db.rs`
- `services/aws-mock-data-service/src/metrics.rs`
- `dashboard-ui/src/components/MockApiDashboard.tsx`
- `dashboard-ui/src/lib/api-mock-dashboard.ts`
- `dashboard-ui/vite.config.ts`
- `graph_builder.py`
