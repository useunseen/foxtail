---
status: complete
priority: p2
issue_id: "058"
tags: [code-review, performance, database, rust]
dependencies: []
---

# Scale dashboard and metric query paths with SQL-side limits

## Problem Statement

The service’s dashboard and metric-query paths currently scale with total table size, not with requested response size. As the generated dataset grows, routine dashboard reads and broad CloudWatch queries will degrade into full scans, large in-memory aggregations, and longer SQLite lock hold times.

## Findings

- `build_dashboard_data()` loads all matching resources, metrics, and cost rows with `fetch_all()` and aggregates them in Rust: [`src/serve.rs:780`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L780), [`src/serve.rs:835`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L835), [`src/serve.rs:864`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L864).
- Route-specific dashboard endpoints all call `build_dashboard_data()` even when they return only one slice of the payload: [`src/serve.rs:1325`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1325), [`src/serve.rs:1339`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1339), [`src/serve.rs:1351`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1351).
- Installed indexes lead on `resource_id`, but common dashboard queries are unscoped and sort/filter by `seconds_from_now`, so SQLite cannot efficiently serve the hottest shapes: [`src/serve.rs:844`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L844), [`src/serve.rs:871`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L871), [`migrations/20260218130000_performance_indexes.sql:4`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/migrations\/20260218130000_performance_indexes.sql#L4), [`migrations/20260219000000_composite_indexes.sql:2`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/migrations\/20260219000000_composite_indexes.sql#L2), [`migrations/20260219000000_composite_indexes.sql:3`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/migrations\/20260219000000_composite_indexes.sql#L3).
- `sum_cost_records_for_window()` filters on `seconds_from_now` without a supporting time-first index, turning multiple Cost Explorer operations into full-table scans: [`src/serve.rs:433`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L433), [`src/serve.rs:1636`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1636), [`migrations/20260218120000_initial_schema.sql:32`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/migrations\/20260218120000_initial_schema.sql#L32).
- `GetDimensionValues` paginates in memory after fetching and sorting the full candidate set: [`src/serve.rs:1746`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1746)-[`src/serve.rs:1801`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L1801).
- `GetMetricData` executes one SQLite query per requested metric and each query can fetch up to 10,000 points before pagination: [`src/serve.rs:2489`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L2489)-[`src/serve.rs:2575`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/serve.rs#L2575), [`src/metrics.rs:36`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/metrics.rs#L36), [`src/metrics.rs:73`](\/Users\/murphy\/workspace\/iacai0\/foxtail\/src\/metrics.rs#L73).

## Proposed Solutions

### Option 1: Push filtering, aggregation, and pagination into SQL

**Approach:** Split the dashboard handlers into narrower queries, aggregate in SQL, and paginate/limit before rows leave SQLite.

**Pros:**
- Largest latency and memory reduction.
- Removes repeated full-table reads from admin endpoints.
- Better aligns behavior with `top_n` and page-size inputs.

**Cons:**
- Requires query refactors and careful response-shape verification.
- May need dedicated structs per endpoint instead of one shared builder.

**Effort:** Medium

**Risk:** Medium

---

### Option 2: Add query-shape-specific indexes and hard request caps first

**Approach:** Add time-first indexes for dashboard and cost windows, cap query counts/page sizes, and keep current Rust-side aggregation temporarily.

**Pros:**
- Faster risk reduction.
- Smaller first patch.

**Cons:**
- Does not remove the core in-memory aggregation cost.
- May still fall over as datasets continue to grow.

**Effort:** Small to Medium

**Risk:** Medium

## Recommended Action

Implemented on 2026-03-11 by narrowing the route-specific dashboard handlers, adding time-first indexes for time-window reads, bounding `GetMetricData` query fan-out, and moving `GetDimensionValues` paging into SQL-backed reads.

## Technical Details

**Affected files:**
- `src/serve.rs`
- `src/metrics.rs`
- `migrations/20260218120000_initial_schema.sql`
- `migrations/20260218130000_performance_indexes.sql`
- `migrations/20260219000000_composite_indexes.sql`

**Related components:**
- Dashboard resources/trends endpoints
- Cost Explorer aggregation helpers
- CloudWatch `GetMetricData` path

**Database changes:**
- Likely yes; index changes are probable.

## Resources

- Prior related note: `todos/033-complete-p1-fix-sqlite-performance-locking.md`
- Current review target: commit `18148ce`

## Acceptance Criteria

- [x] Dashboard endpoints do not fetch entire matching `metrics`/`cost_records` tables when only summary slices are needed.
- [x] Query plans for hot dashboard/cost paths use indexes aligned with `seconds_from_now` filters and ordering.
- [x] `GetDimensionValues` and `GetMetricData` enforce bounded work per request.
- [x] Endpoint latency and memory use scale better with requested result size on the route-specific dashboard surfaces.

## Work Log

### 2026-03-11 - Review Discovery

**By:** Codex

**Actions:**
- Reviewed dashboard, Cost Explorer, and CloudWatch query paths.
- Cross-checked query predicates against installed SQL indexes.
- Compared endpoint response slicing with actual data-fetch scope.

**Learnings:**
- The service already added some composite indexes, but they do not match the dominant unscoped read patterns.
- The admin dashboard is currently the main source of avoidable full-table work.

### 2026-03-11 - Resolution

**By:** Codex

**Actions:**
- Added `migrations/20260311120000_dashboard_time_indexes.sql` with time-first indexes for dashboard and cost-window reads.
- Refactored the route-specific dashboard endpoints to avoid building the full dashboard payload for every slice response.
- Added SQL-backed paging for `GetDimensionValues` and bounded `GetMetricData` to 50 metric queries per request.
- Ran `cargo fmt`, `cargo test`, and `cargo clippy --all-targets --all-features`.

**Learnings:**
- Most of the avoidable cost was concentrated in the route-specific dashboard slices rather than the full all-in-one dashboard endpoint.
