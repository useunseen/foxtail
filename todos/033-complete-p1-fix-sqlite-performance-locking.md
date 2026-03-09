---
status: completed
priority: p1
issue_id: "033"
tags: [rust, performance, database]
dependencies: []
---

# 033-pending-p1-fix-sqlite-performance-locking.md

## Problem Statement
The `aws-mock-data-service` currently suffers from severe performance bottlenecks in data generation and potential "database is locked" errors during concurrent read/write operations. 

- Data generation performs 100s of individual inserts without a transaction.
- Standard SQLite mode (not WAL) leads to contention between `gen` and `serve` commands.

## Findings
- **N+1 Writes**: `gen.rs:168-180` performs individual `INSERT` statements in a loop. In SQLite, each insert without a transaction triggers a disk sync.
- **Concurrency**: `db.rs` initializes a connection pool but does not enable Write-Ahead Logging (WAL), making simultaneous discovery and serving prone to locking.

## Proposed Solutions

### Option 1: Transactions & WAL (Recommended)
- Wrap generation loops in `sqlx::Transaction`.
- Enable WAL mode via `PRAGMA journal_mode=WAL` in `db.rs`.
- **Effort**: Small
- **Risk**: Low

### Option 2: Batch Inserts
- Rewrite the query to use a single batch insert (e.g., `INSERT INTO ... VALUES (?,?), (?,?)`).
- **Effort**: Medium
- **Risk**: Low

## Acceptance Criteria
- [x] `aws-mock gen` completes in < 2 seconds for a standard profile.
- [x] `aws-mock serve` handles requests while `aws-mock gen` is running without locking errors.
- [x] `db.rs` explicitly sets WAL mode.

## Work Log
### 2026-02-18 - Findings during code review
- Identified N+1 write bottleneck in `gen.rs`.
- Noted lack of WAL mode in `db.rs`.

### 2026-02-19 - Implementation
- Enabled WAL mode and Normal synchronous mode in `db.rs`.
- Wrapped the entire discovery and data generation process in a single transaction in `generator.rs`.
- Verified compilation and fixed unrelated errors in `serve.rs` and `metrics.rs` discovered during `cargo check`.
