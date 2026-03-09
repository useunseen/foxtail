---
status: complete
priority: p3
issue_id: "052"
tags: [code-review, quality, tooling]
dependencies: []
---

# .gitignore change removed unrelated existing ignore rules

## Problem Statement

The `.gitignore` update removes prior ignore patterns (`.mock_data/`, `memory-bank/*`, `tool_catalog/`, `.worktrees`) and replaces them with Rust/SQLite patterns. This broad replacement risks accidental tracking of unrelated local artifacts.

## Findings

- Diff removes multiple pre-existing repository ignore entries while adding Rust-specific ones.
- Change is unrelated to core runtime behavior and may introduce noisy diffs in other workflows.

## Proposed Solutions

### Option 1: Merge ignore lists instead of replacing

**Approach:** Keep prior entries and append new Rust/SQLite patterns.

**Pros:**
- Preserves existing workflow expectations.
- Low-risk hygiene improvement.

**Cons:**
- Slightly longer `.gitignore`.

**Effort:** Small

**Risk:** Low

---

### Option 2: Split project-specific ignores into per-directory `.gitignore`

**Approach:** Keep root generic; place mock-service ignores under `services/aws-mock-data-service/.gitignore`.

**Pros:**
- Better scope isolation.

**Cons:**
- Requires convention updates.

**Effort:** Small

**Risk:** Low

## Recommended Action

To be filled during triage.

## Technical Details

**Affected files:**
- `.gitignore`

## Resources

- PR: `https://github.com/peterlimg/aws-optimize-agent/pull/28`

## Acceptance Criteria

- [ ] Pre-existing ignore rules are preserved unless explicitly deprecated.
- [ ] Rust mock service artifacts remain ignored.
- [ ] Repository-wide ignore behavior documented for future changes.

## Work Log

### 2026-02-19 - Code Review Discovery

**By:** Codex

**Actions:**
- Reviewed root `.gitignore` diff against `origin/main`.

**Learnings:**
- Current patch conflates service-specific additions with unrelated removals.

### 2026-02-19 - Resolution

**By:** Codex

**Actions:**
- Implemented and validated the fix.
- Ran targeted checks to verify expected behavior.
- Prepared change for merge.

**Learnings:**
- The issue is resolved on this branch.
